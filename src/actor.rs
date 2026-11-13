//! Moves an actor through the world by playing animations.
//!
//! The step logic follows ScummVM's AnimationTracker: every animation
//! frame carries the distance to move, and a frame is only taken if the
//! resulting position is free and (for ground frames) supported.

use crate::anim::{AnimAction, AnimDat, JUMP_UP, LAND, RUN, STAND, STAND_UP, WALK};
use crate::map::{Item, MapData, PositionInfo, WorldBox, FLG_FLIPPED};
use crate::shape::ShapeInfo;
use std::rc::Rc;

/// Movement per direction (0 = north, clockwise in 45 degree steps).
const X_FACTOR: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
const Y_FACTOR: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];

const GRAVITY: i32 = 4;
/// A fall at least this deep ends with the landing animation (three steps).
const LAND_HEIGHT: i32 = 24;
/// From this deep on a falling figure shows the falling pose.
const POSE_HEIGHT: i32 = 12;
/// "Oof": landing after a fall (as ScummVM's GravityProcess).
const SFX_LANDED: u8 = 0x8F;

#[derive(Clone, Copy)]
pub struct MoveInput {
    pub dir: u8,
    pub run: bool,
}

struct Playing {
    action: Rc<AnimAction>,
    id: u32,
    frame: usize,
    end: usize,
    first: bool,
    /// Position at the start of the animation.
    start: (i32, i32, i32),
}

enum StepResult {
    Moved,
    Done,
    Blocked,
    Unsupported,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Mover {
    pub dir: u8,
    last_anim: u32,
    first_step: bool,
    /// Not saved: after loading every actor starts from its idle pose.
    #[serde(skip)]
    playing: Option<Playing>,
    repeat: u32,
    /// Vertical speed while falling, None when on the ground.
    falling: Option<i32>,
    /// Stand around when there is nothing else to do (the Avatar).
    pub auto_stand: bool,
    /// Sound effect of the frame just taken, for the game to play.
    pub sfx: u8,
    /// A foot came down in the frame just taken; true if running.
    pub footstep: Option<bool>,
    /// Animation to fall back to when idle: standing, or the fighting pose.
    pub stand_anim: u32,
    /// Stay in the last frame instead of going back to the idle pose
    /// (holding a block, lying dead).
    pub hold: bool,
    /// Reach of the attack frame just taken, for the game to look for a victim.
    pub attack: Option<i32>,
    /// The current attack has already hit someone.
    pub hit_done: bool,
    /// Animation and frame number last shown.
    shown: (u32, usize),
    /// A steered jump: what is still missing to land on the chosen spot
    /// (x, y, z), spread over the remaining frames in the air.
    #[serde(skip)]
    steer: Option<((i32, i32, i32), i32)>,
    /// Trying an animation out: running into something makes it fail
    /// instead of playing on the spot.
    #[serde(skip)]
    strict: bool,
    /// Walking on a planned way (scripts, a click): never step off an
    /// edge, stop instead.
    #[serde(skip)]
    pub careful: bool,
    /// Careful, but the way leads down over the edge ahead: stepping off
    /// it is allowed.
    #[serde(skip)]
    pub may_drop: bool,
    /// Height the current fall began at.
    #[serde(skip)]
    fall_from: Option<i32>,
    /// In a scene (the Avatar held by a script): after an animation the
    /// actor keeps its last frame (kneeling ...) instead of going back to
    /// its idle pose, as in the original. Set by the game.
    #[serde(skip)]
    pub in_scene: bool,
    /// Walked by the player with the keys or the pad: slantwise into a wall
    /// he slides along it, over an edge he steps clear of it.
    #[serde(skip)]
    pub player_walk: bool,
    /// The player was walking, running, jumping or climbing when a scene
    /// took hold of the Avatar: that movement ends as usual and he comes
    /// to stand, not frozen in the middle of a stride (the Titans: on
    /// reaching the spot where they wake). Cleared when a script plays an
    /// animation of its own. Set by the game.
    #[serde(skip)]
    pub settle: bool,
    /// Put where it is by a script (Item::move): it stays there, also in
    /// the air, until it moves by itself again (Stratos lifts the Avatar
    /// two units at a time up to her island). Set by the game.
    #[serde(skip)]
    pub placed: bool,
    /// An animation to be kept (dying) that a fall cut short: it goes on
    /// once the actor is down (a changeling a little above the floor of
    /// the ether sinks into the ground after all).
    #[serde(skip)]
    resume: Option<u32>,
}

impl Mover {
    pub fn new(dir: u8) -> Mover {
        Mover { dir: dir & 7, last_anim: STAND, first_step: true, playing: None, repeat: 0, falling: None, auto_stand: false, sfx: 0, footstep: None, stand_anim: STAND, hold: false, attack: None, hit_done: false, shown: (STAND, 0), steer: None, strict: false, careful: false, may_drop: false, fall_from: None, in_scene: false, player_walk: false, settle: false, placed: false, resume: None }
    }

    /// Plays an animation once. Returns false if the shape has no such animation.
    pub fn play(&mut self, id: u32, item: &Item, anims: &mut AnimDat) -> bool {
        self.settle = false;
        self.start(id, item, anims)
    }

    /// Starts the next animation from its first frame, even if it is the
    /// same as the last one (a climb right after a climb).
    pub fn forget_last(&mut self) {
        self.last_anim = STAND;
        self.first_step = true;
    }

    /// The animation and the number of its frame the actor shows.
    pub fn shown(&self) -> (u32, usize) {
        self.shown
    }

    pub fn falling(&self) -> bool {
        self.falling.is_some()
    }

    /// Game ticks one frame of the current animation is shown for, i.e.
    /// the time until the actor moves again.
    pub fn step_ticks(&self) -> u32 {
        self.playing.as_ref().map_or(1, |p| p.action.frame_repeat + 1)
    }

    /// In the middle of an animation (other than standing around) or falling.
    pub fn busy(&self) -> bool {
        self.falling.is_some() || self.playing.as_ref().is_some_and(|p| p.id != STAND && p.id != self.stand_anim)
    }

    pub fn last_anim(&self) -> u32 {
        self.playing.as_ref().map_or(self.last_anim, |p| p.id)
    }

    fn start(&mut self, id: u32, item: &Item, anims: &mut AnimDat) -> bool {
        self.placed = false;
        let Some(action) = anims.action(item.shape, id) else { return false };
        let (frame, end) = action.range(id, self.last_anim, self.dir, self.first_step, self.dir);
        self.playing = Some(Playing { action, id, frame, end, first: true, start: (item.x, item.y, item.z) });
        self.repeat = 0;
        self.hit_done = false;
        self.steer = None;
        true
    }

    /// Plays a jump so that it ends at `target`: the difference between
    /// where the animation would land by itself and the target is spread
    /// over the frames in which the actor is in the air.
    pub fn play_steered(&mut self, id: u32, item: &Item, anims: &mut AnimDat, target: (i32, i32, i32)) -> bool {
        if !self.start(id, item, anims) {
            return false;
        }
        let p = self.playing.as_ref().unwrap();
        let dir = self.dir as usize;
        let (mut along, mut up, mut airborne) = (0, 0, 0);
        let mut f = p.frame;
        for _ in 0..p.action.size + 1 {
            if f == p.end || f >= p.action.size {
                break;
            }
            let fr = p.action.frame(self.dir, f);
            along += fr.delta_dir;
            up += fr.delta_z;
            if !fr.on_ground() {
                airborne += 1;
            }
            f = p.action.next_frame(f, p.end);
        }
        if airborne == 0 {
            return true; // nothing to steer with; plays as it is
        }
        let natural = (item.x + 4 * X_FACTOR[dir] * along, item.y + 4 * Y_FACTOR[dir] * along, item.z + up);
        self.steer = Some(((target.0 - natural.0, target.1 - natural.1, target.2 - natural.2), airborne));
        true
    }

    /// Would this animation play through from here without running into
    /// anything, and end on solid ground somewhere else? Used to find out
    /// which ledge can be climbed.
    pub fn would_succeed(&self, id: u32, item: &Item, map: &MapData, infos: &[ShapeInfo], anims: &mut AnimDat) -> bool {
        let Some(info) = infos.get(item.shape as usize).copied() else { return false };
        let mut probe = Mover::new(self.dir);
        probe.strict = true;
        let mut it = *item;
        if !probe.start(id, &it, anims) {
            return false;
        }
        for _ in 0..200 {
            match probe.step(&mut it, map, infos, &info) {
                StepResult::Moved => {}
                StepResult::Done => {
                    let b = WorldBox::of(&it, &info);
                    let pi = map.position_info(&b, &b, infos, item.id);
                    // played for real he is lifted onto the ground at the end
                    // (a ledge's top): there he must fit too, not with his
                    // head in the roof above a low wall
                    if let Some(fz) = pi.floor_z.filter(|&f| f > it.z) {
                        let from = WorldBox::of(item, &info);
                        let top = WorldBox { z: fz, ..b };
                        if !map.position_info(&top, &from, infos, item.id).valid {
                            return false;
                        }
                    }
                    return (it.x, it.y, it.z) != (item.x, item.y, item.z) && pi.supported;
                }
                StepResult::Blocked | StepResult::Unsupported => return false,
            }
        }
        false
    }

    /// Advances the actor by one game tick. Returns true if it changed.
    pub fn tick(&mut self, input: Option<MoveInput>, item: &mut Item, map: &MapData, infos: &[ShapeInfo],
                anims: &mut AnimDat) -> bool {
        let Some(info) = infos.get(item.shape as usize).copied() else { return false };
        if self.falling.is_some() {
            let from = *self.fall_from.get_or_insert(item.z);
            let moved = self.fall(item, map, infos, &info);
            if self.falling.is_none() {
                self.fall_from = None;
                if let Some(id) = self.resume.take() {
                    self.start(id, item, anims);
                    return moved;
                }
                // down: after a deep fall he lands (goes down on his knees
                // and gets up), with an "oof", as in the original
                if from - item.z >= LAND_HEIGHT && !self.hold && self.start(LAND, item, anims) {
                    self.sfx = SFX_LANDED;
                }
            } else if from - item.z >= POSE_HEIGHT {
                self.show_fall_pose(item, anims);
            }
            return moved;
        }

        // standing around can be interrupted at any time
        let idle = self.stand_anim;
        if input.is_some() && !self.hold && self.playing.as_ref().is_some_and(|p| p.id == STAND || p.id == idle) {
            self.playing = None;
        }

        // standing in the air (a jump stopped by a branch, a savegame made
        // then, a floating platform gone from under him): come down
        let standing = self.playing.as_ref().is_none_or(|p| p.id == STAND || p.id == idle);
        if standing && !self.placed && self.in_air(item, map, infos, &info) {
            // a script just turned him (stand facing a way): he comes down
            // facing that way (the acolytes put next to the Great Pentacle)
            if let Some(p) = self.playing.take() {
                // (lying down dead: the dying goes on once it is down)
                if self.hold {
                    self.resume = Some(p.id);
                }
                let f = *p.action.frame(self.dir, p.frame);
                item.frame = f.frame;
                if f.flipped() {
                    item.flags |= FLG_FLIPPED;
                } else {
                    item.flags &= !FLG_FLIPPED;
                }
                self.shown = (p.id, p.frame);
            }
            self.falling = Some(0);
            return true;
        }

        let mut changed = false;
        // at most two rounds: finishing one animation and starting the next
        for _ in 0..2 {
            if self.playing.is_none() {
                self.choose(input, item, anims);
            }
            let Some(p) = &self.playing else { return changed };
            if !p.first {
                self.repeat += 1;
                if self.repeat <= p.action.frame_repeat {
                    return changed;
                }
            }
            self.repeat = 0;
            match self.step(item, map, infos, &info) {
                StepResult::Moved => return true,
                StepResult::Done => {
                    let p = self.playing.take().unwrap();
                    if p.action.two_step() {
                        self.first_step = !self.first_step;
                    }
                    self.last_anim = p.id;
                    changed = true;
                    // a jump that was cut short ends in mid-air: come down
                    if p.id != WALK && p.id != RUN && item.z > 0 {
                        let (xd, yd, zd) = info.footpad(item.flags & FLG_FLIPPED != 0);
                        let here = WorldBox { x: item.x, y: item.y, z: item.z, xd, yd, zd };
                        let pi = map.position_info(&here, &here, infos, item.id);
                        if !pi.supported && pi.floor_z.is_none_or(|f| item.z > f) {
                            self.falling = Some(0);
                            return true;
                        }
                    }
                }
                StepResult::Blocked => {
                    self.playing = None;
                    self.last_anim = STAND;
                    self.first_step = true;
                    // a jump that runs into something in mid-air falls down
                    if self.in_air(item, map, infos, &info) {
                        self.falling = Some(0);
                        return true;
                    }
                    return changed;
                }
                StepResult::Unsupported => {
                    if self.hold {
                        self.resume = self.playing.as_ref().map(|p| p.id);
                    }
                    self.playing = None;
                    self.last_anim = STAND;
                    self.falling = Some(0);
                    return true;
                }
            }
        }
        changed
    }

    /// Nothing carries the actor where it is, and there is room below.
    fn in_air(&self, item: &Item, map: &MapData, infos: &[ShapeInfo], info: &ShapeInfo) -> bool {
        if item.z <= 0 {
            return false;
        }
        let (xd, yd, zd) = info.footpad(item.flags & FLG_FLIPPED != 0);
        let here = WorldBox { x: item.x, y: item.y, z: item.z, xd, yd, zd };
        let pi = map.position_info(&here, &here, infos, item.id);
        if pi.supported || pi.floor_z.is_some_and(|f| item.z <= f) {
            return false;
        }
        map.position_info(&WorldBox { z: item.z - 1, ..here }, &here, infos, item.id).valid
    }

    fn choose(&mut self, input: Option<MoveInput>, item: &Item, anims: &mut AnimDat) {
        if self.hold {
            return;
        }
        match input {
            Some(inp) if inp.dir & 7 != self.dir => {
                // turn one step towards the wanted direction
                let diff = (inp.dir + 8 - self.dir) & 7;
                self.dir = (self.dir + if diff <= 4 { 1 } else { 7 }) & 7;
                self.last_anim = STAND;
                self.first_step = true;
                self.start(self.stand_anim, item, anims);
            }
            Some(inp) => {
                let id = if inp.run { RUN } else { WALK };
                if self.last_anim != id {
                    self.first_step = true;
                }
                self.start(id, item, anims);
            }
            None => {
                // standing still, the next step starts with the first foot
                // again; not between the steps a script walks one by one
                // (each began with the same foot: a stiff march)
                if (!self.in_scene || self.settle) && (self.auto_stand || self.stand_anim != STAND) {
                    self.settle = false;
                    self.first_step = true;
                    self.start(self.stand_anim, item, anims);
                }
            }
        }
    }

    fn step(&mut self, item: &mut Item, map: &MapData, infos: &[ShapeInfo], info: &ShapeInfo) -> StepResult {
        let p = self.playing.as_mut().unwrap();
        if p.first {
            p.first = false;
        } else {
            p.frame = p.action.next_frame(p.frame, p.end);
        }
        if p.frame == p.end || p.frame >= p.action.size {
            return StepResult::Done;
        }
        let f = *p.action.frame(self.dir, p.frame);
        let (anim, frame, start) = (p.id, p.frame, p.start);
        let walking = anim == WALK || anim == RUN;
        let dir = self.dir as usize;
        let (mut dx, mut dy, mut dz) = (4 * X_FACTOR[dir] * f.delta_dir, 4 * Y_FACTOR[dir] * f.delta_dir, f.delta_z);
        if let (Some((rest, left)), false) = (&mut self.steer, f.on_ground()) {
            // a steered jump: take this frame's share of the correction
            let n = (*left).max(1);
            let share = (rest.0 / n, rest.1 / n, rest.2 / n);
            dx += share.0;
            dy += share.1;
            dz += share.2;
            *rest = (rest.0 - share.0, rest.1 - share.1, rest.2 - share.2);
            *left -= 1;
        }

        let (xd, yd, zd) = info.footpad(f.flipped());
        let ground = Footing { map, infos, id: item.id, pad: (xd, yd, zd), start: WorldBox { x: start.0, y: start.1, z: start.2, xd, yd, zd } };
        let (mut tx, mut ty, mut tz) = (item.x + dx, item.y + dy, item.z + dz);

        // (standing right on a floor that is not solid, the trim at a roof's
        // edge, carries below as it does where the actor stands: in_air)
        let pi = ground.at(tx, ty, tz);
        // PAGAN_STEP=<id>: every step of this actor is printed (for tests)
        if crate::debug::env().step == Some(item.id) {
            eprintln!("step {} anim {} frame {} dir {} delta {},{},{} at {},{},{} valid {} supported {} floor {:?} on_ground {}", item.id, anim, frame, dir, dx, dy, dz, item.x, item.y, item.z, pi.valid, pi.supported, pi.floor_z, f.on_ground());
        }
        if self.strict && !pi.valid {
            return StepResult::Blocked;
        } else if !walking && !pi.valid {
            // A lunge or stagger that runs into something (usually the
            // opponent) plays on the spot instead of being cut short.
            (tx, ty, tz) = (item.x, item.y, item.z);
        } else if !walking && pi.valid && !self.careful && f.on_ground() && !(pi.supported || pi.floor_z == Some(tz)) {
            // Pushed over an edge, falling back off it: the animation
            // plays on as in the original, and only at its end the actor
            // falls if nothing carries it (cut short at the first frame
            // half over the edge, he hung in the air: the man Mordea
            // pushes off the execution pier). Frames that only hold the
            // pose are not played in the air over a drop: he falls at once
            // (Torwin's leap into the water hung there for 12 ticks); a
            // step down a stair (ground within 16) plays on
            let step = ground.at(tx, ty, tz - 16);
            if dx == 0 && dy == 0 && dz == 0 && step.valid && !step.supported {
                return StepResult::Unsupported;
            }
        } else if !pi.valid || (f.on_ground() && !(pi.supported || pi.floor_z == Some(tz))) || (self.careful && pi.on_water) {
            if !f.on_ground() || self.strict {
                return StepResult::Blocked;
            }
            match self.foothold(&ground, item, (tx, ty, tz), (dx, dy), !pi.valid && walking, dir) {
                Some(t) => (tx, ty, tz) = t,
                // walking off an edge: fall instead of stopping
                None if pi.valid && (dx != 0 || dy != 0) && (!self.careful || self.may_drop) => {
                    self.walk_off_edge(&ground, item, (tx, ty, tz), dir);
                    if self.player_walk {
                        item.frame = f.frame;
                    }
                    return StepResult::Unsupported;
                }
                None => return StepResult::Blocked,
            }
        }

        // never end up inside the ground (a jump coming down a little too far)
        if !self.strict {
            if let Some(fz) = ground.at(tx, ty, tz).floor_z.filter(|fz| tz < *fz) {
                tz = fz;
            }
        }
        // nor below the bottom of the world (a running jump on ground at
        // height 0 ended at -18, and he walked on sunk into it; found by
        // the test bot)
        tz = tz.max(0);
        (item.x, item.y, item.z) = (tx, ty, tz);
        item.frame = f.frame;
        self.shown = (anim, frame);
        self.sfx = f.sfx;
        if f.attack_range() > 0 && !self.hit_done {
            self.attack = Some(f.attack_range());
        }
        if f.footstep() {
            self.footstep = Some(anim == RUN);
        }
        if f.flipped() {
            item.flags |= FLG_FLIPPED;
        } else {
            item.flags &= !FLG_FLIPPED;
        }
        StepResult::Moved
    }

    /// A supported spot close to where a step on the ground was to end
    /// (`(tx, ty, tz)`, `(dx, dy)` the stride): up or down a step, or a
    /// little to the side to get around corners. `blocked_walk`: a walk
    /// or run into something solid.
    fn foothold(&self, ground: &Footing, item: &Item, (tx, ty, tz): (i32, i32, i32), (dx, dy): (i32, i32), blocked_walk: bool, dir: usize) -> Option<(i32, i32, i32)> {
        let (px, py) = (-Y_FACTOR[dir], X_FACTOR[dir]);
        // Places to try: straight on, a little to either side; walking
        // slantwise into a wall also straight along it (else he stood
        // still mid-stride, as if stuck)
        let mut bases: Vec<(i32, i32)> = Vec::new();
        for side in [0, 8, -8, 16, -16] {
            if side != 0 && dx == 0 && dy == 0 {
                break;
            }
            bases.push((tx + px * side, ty + py * side));
        }
        if blocked_walk && dx != 0 && dy != 0 && self.player_walk {
            bases.push((tx, item.y));
            bases.push((item.x, ty));
        }
        // a step up or down: 8 high, more for a longer stride (running
        // up or down a stair covers two of its steps at once; with only
        // 8 he stumbled down and slid up), at most 16
        let stride = dx.abs().max(dy.abs());
        let reach = ((stride + 31) / 32 * 8).clamp(8, 16);
        // more than 8 at once only over a step half way (a stair), not
        // up or down a single higher ledge (a bed, a wall)
        let step_between = |cx: i32, cy: i32, cz: i32| {
            let (mx, my) = ((item.x + cx) / 2, (item.y + cy) / 2);
            let mid = (item.z + cz) / 2;
            (-4..=4).any(|d| {
                let m = ground.at(mx, my, mid + d);
                m.valid && (m.supported || m.floor_z == Some(mid + d))
            })
        };
        for &(bx, by) in &bases {
            for k in 0..=2 * reach {
                let lift = if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) };
                let (cx, cy, cz) = (bx, by, tz + lift);
                if cz < 0 {
                    continue;
                }
                let c = ground.at(cx, cy, cz);
                // never step down into the ground
                if c.valid && (c.supported || c.floor_z == Some(cz)) && c.floor_z.is_none_or(|f| cz >= f) && !(self.careful && c.on_water)
                    && (lift.abs() <= 8 || step_between(cx, cy, cz))
                {
                    return Some((cx, cy, cz));
                }
            }
        }
        None
    }

    /// Walking off an edge to `(tx, ty, tz)`. Still hanging half over the
    /// edge he could not fall (and went on in the air step by step): the
    /// player's Avatar goes on until he is clear of it.
    fn walk_off_edge(&self, ground: &Footing, item: &mut Item, (mut tx, mut ty, tz): (i32, i32, i32), dir: usize) {
        let (ux, uy) = (X_FACTOR[dir], Y_FACTOR[dir]);
        for _ in 0..if self.player_walk { 8 } else { 0 } {
            if ground.at(tx, ty, tz - 1).valid || !ground.at(tx + 8 * ux, ty + 8 * uy, tz).valid {
                break;
            }
            (tx, ty) = (tx + 8 * ux, ty + 8 * uy);
        }
        (item.x, item.y, item.z) = (tx, ty, tz);
    }
    /// The pose of a standing jump coming down (its first frame in the air
    /// going down), for a fall.
    fn show_fall_pose(&mut self, item: &mut Item, anims: &mut AnimDat) {
        let Some(a) = anims.action(item.shape, JUMP_UP) else { return };
        let Some(n) = (0..a.size).find(|&n| { let f = a.frame(self.dir, n); !f.on_ground() && f.delta_z < 0 }) else { return };
        let f = *a.frame(self.dir, n);
        item.frame = f.frame;
        if f.flipped() {
            item.flags |= FLG_FLIPPED;
        } else {
            item.flags &= !FLG_FLIPPED;
        }
        self.shown = (JUMP_UP, n);
    }

    fn fall(&mut self, item: &mut Item, map: &MapData, infos: &[ShapeInfo], info: &ShapeInfo) -> bool {
        let speed = self.falling.unwrap_or(0) + GRAVITY;
        self.falling = Some(speed);
        let (xd, yd, zd) = info.footpad(item.flags & FLG_FLIPPED != 0);
        for _ in 0..speed {
            let here = WorldBox { x: item.x, y: item.y, z: item.z, xd, yd, zd };
            let info = map.position_info(&here, &here, infos, item.id);
            if item.z <= 0 || info.supported || info.floor_z.is_some_and(|f| item.z <= f) {
                self.falling = None;
                break;
            }
            let below = WorldBox { z: item.z - 1, ..here };
            if !map.position_info(&below, &here, infos, item.id).valid {
                self.falling = None;
                break;
            }
            item.z -= 1;
        }
        true
    }
}

/// The animation the Avatar starts a new game with: getting up.
pub fn intro_anim() -> u32 {
    STAND_UP
}

/// The space a step of an actor moves in: its box (`pad`) tried at
/// positions on the map, measured from where the animation began.
struct Footing<'a> {
    map: &'a MapData,
    infos: &'a [ShapeInfo],
    /// The actor (it does not get in its own way).
    id: u16,
    pad: (i32, i32, i32),
    start: WorldBox,
}

impl Footing<'_> {
    fn at(&self, x: i32, y: i32, z: i32) -> PositionInfo {
        let (xd, yd, zd) = self.pad;
        self.map.position_info(&WorldBox { x, y, z, xd, yd, zd }, &self.start, self.infos, self.id)
    }
}
