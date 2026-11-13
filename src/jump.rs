//! Jumping, climbing, and ground that kills (water, lava).
//!
//! Jumps go exactly to the spot under the mouse pointer.  The original
//! game got this with a patch; ScummVM calls it "targeted jump".  The
//! jump animation is steered: the difference between where it would land
//! by itself and the chosen spot is spread over its frames in the air.

use crate::anim;
use crate::game::{Game, AVATAR};
use crate::intrinsics::world_dir;
use crate::map::WorldBox;

/// Walking by click to a place higher up: whenever the walk ends below
/// it, the Avatar climbs the ledge towards it and walks on.
#[derive(Clone, Copy)]
pub struct ClimbGoal {
    x: i32,
    y: i32,
    z: i32,
    /// Ledges climbed so far.
    climbs: u8,
    /// Single steps taken towards the ledge to stand right at it.
    steps: u8,
    /// A step is under way (the input is taken away again next tick).
    pub(crate) nudging: bool,
    /// Climbed since the last walk: walk on towards the goal.
    walk_on: bool,
}

impl ClimbGoal {
    pub fn new(x: i32, y: i32, z: i32) -> ClimbGoal {
        ClimbGoal { x, y, z, climbs: 0, steps: 0, nudging: false, walk_on: false }
    }
}

impl Game {
    /// Climbs the highest ledge that works, trying the directions in
    /// order. Returns true if he climbs; otherwise his direction is kept.
    pub(crate) fn try_climb(&mut self, dirs: &[u8], close_in: bool) -> bool {
        // one step of a direction in the world
        const UNIT: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
        let av = self.avatar();
        let Some(info) = self.data.infos.get(av.shape as usize).copied() else { return false };
        let mut mover = self.actors.remove(&AVATAR).unwrap();
        let before = mover.dir;
        // (action, position to climb from)
        let mut climb = None;
        // from where he stands; with `close_in` also from a little nearer
        // to the ledge (a stride can stop short of it)
        let reach = if close_in { 10 } else { 0 };
        'dirs: for &dir in dirs {
            mover.dir = dir & 7;
            let (ux, uy) = UNIT[mover.dir as usize];
            for k in 0..=reach {
                let at = crate::map::Item { x: av.x + 4 * k * ux, y: av.y + 4 * k * uy, ..av };
                if k > 0 {
                    let b = WorldBox::of(&at, &info);
                    let pi = self.map().position_info(&b, &b, &self.data.infos, AVATAR);
                    if !pi.valid || !pi.supported {
                        break;
                    }
                }
                for action in (anim::CLIMB_FIRST..=anim::CLIMB_LAST).rev() {
                    let Game { data: crate::gamedata::GameData { infos, anims, .. }, maps, cur_map, .. } = self;
                    if mover.would_succeed(action, &at, &maps[cur_map], infos, anims) {
                        climb = Some((action, at));
                        break 'dirs;
                    }
                }
            }
        }
        if climb.is_none() {
            mover.dir = before;
        }
        self.actors.insert(AVATAR, mover);
        let Some((action, at)) = climb else { return false };
        if (at.x, at.y) != (av.x, av.y) {
            // the last few units up to the ledge
            let map = self.cur_map;
            self.teleport(AVATAR, map, (at.x, at.y, at.z));
        }
        // as tried above: from the first frame on
        self.mover(AVATAR).forget_last();
        self.animate(AVATAR, action)
    }

    /// One tick of walking by click to a higher place: the ledges on the
    /// way are climbed.
    pub fn tick_auto_climb(&mut self) {
        let Some(mut g) = self.orders.climb_goal else { return };
        if g.nudging {
            // one step at a time
            self.inputs.remove(&AVATAR);
            g.nudging = false;
        }
        if self.state(AVATAR).dead || self.stasis || self.ask.is_some() {
            self.orders.climb_goal = None;
            return;
        }
        // still on the way, or in the middle of a step or a climb
        if self.walking() || self.mover(AVATAR).busy() {
            self.orders.climb_goal = Some(g);
            return;
        }
        let av = self.avatar();
        let (dx, dy) = (g.x - av.x, g.y - av.y);
        let close = dx.abs().max(dy.abs()) <= 48;
        if crate::debug::env().trace {
            eprintln!("auto climb: tick {} at {},{},{} climbs {} steps {} walk_on {}", self.tick, av.x, av.y, av.z, g.climbs, g.steps, g.walk_on);
        }
        if av.z + 8 >= g.z {
            // up there: walk the rest of the way, once
            if g.walk_on && !close {
                self.climb_walk_on(g);
            } else {
                self.orders.climb_goal = None;
            }
            return;
        }
        let d = world_dir(dx, dy) as u8;
        const STEP: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
        if g.climbs < 8 && self.try_climb(&[d, d + 1, d + 7, d + 2, d + 6], true) {
            g.climbs += 1;
            g.walk_on = true;
            self.orders.climb_goal = Some(g);
        } else if g.walk_on {
            // climbed, but not yet high enough: on to the next ledge
            self.climb_walk_on(g);
        } else if g.steps < 4 && !close && self.open_above(&av, (av.x + 32 * STEP[d as usize].0, av.y + 32 * STEP[d as usize].1, av.z), g.z) && self.ledge_ahead(&av, d) {
            // perhaps not quite at the ledge yet: a step towards it (not
            // under a bridge or a roof: from there nothing leads up; and
            // only with a ledge right ahead - steps over open floor towards
            // a bed made the Avatar zigzag before shaking his head)
            g.steps += 1;
            g.nudging = true;
            self.inputs.insert(AVATAR, crate::actor::MoveInput { dir: d, run: false });
            self.orders.climb_goal = Some(g);
        } else {
            // no way up from here
            self.orders.climb_goal = None;
            self.animate(AVATAR, anim::SHAKE_HEAD);
        }
    }

    /// Will the Avatar, standing at height `z`, climb on towards the spot
    /// clicked when his walk ends (it lies higher up)?
    pub(crate) fn climb_follows(&self, z: i32) -> bool {
        self.orders.climb_goal.is_some_and(|g| z + 8 < g.z)
    }

    /// Is something in the way within two steps in direction `d`, at the
    /// height the Avatar stands (the ledge he should climb)?
    fn ledge_ahead(&self, av: &crate::map::Item, d: u8) -> bool {
        const STEP: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
        let Some(info) = self.data.infos.get(av.shape as usize) else { return false };
        let (sx, sy) = STEP[d as usize & 7];
        (1..=2).any(|k| {
            let at = crate::map::Item { x: av.x + 32 * k * sx, y: av.y + 32 * k * sy, ..*av };
            let b = WorldBox::of(&at, info);
            !self.map().position_info(&b, &b, &self.data.infos, AVATAR).valid
        })
    }

    /// After a climb: walk on towards the goal.
    fn climb_walk_on(&mut self, mut g: ClimbGoal) {
        g.walk_on = false;
        g.steps = 0;
        self.start_walk(g.x, g.y);
        // (still for the height clicked: not into the room below a roof)
        self.orders.walk_level = Some(g.z);
        self.orders.climb_goal = Some(g);
    }

    /// The spot on the ground a view position points at: the top of the
    /// land item drawn there, as the position the Avatar would stand at.
    pub(crate) fn ground_at(&mut self, x: i32, y: i32) -> Option<(i32, i32, i32)> {
        self.ground_spot(x, y).map(|(spot, _)| spot)
    }

    /// As ground_at, and whether the top there is small (a pillar, a
    /// crate): the Avatar is placed wholly on it, and must get there exactly.
    pub(crate) fn ground_spot(&mut self, x: i32, y: i32) -> Option<((i32, i32, i32), bool)> {
        // an open window (backpack, chest) covers the world behind it
        if self.pick_gump(x, y).is_some() {
            return None;
        }
        let id = self.pick_world(x, y, AVATAR)?;
        let it = self.map().get(id).copied()?;
        let si = self.data.infos.get(it.shape as usize).copied()?;
        if si.sea || si.damaging {
            return None;
        }
        let av = self.data.infos.get(self.avatar().shape as usize).copied()?;
        if !si.land {
            // A bush or a plant one can walk through: the ground it stands on.
            if si.solid || !it.in_world() {
                return None;
            }
            let (px, py) = self.to_world(x, y, it.z);
            return Some(((px + av.x * 16, py + av.y * 16, it.z), false));
        }
        let z = it.z + si.z * 8;
        // the pointer marks the middle of the Avatar's footprint; a click on
        // the side of a block he can stand on counts as one on its top (the
        // point at the top's height under the pointer would lie off to the
        // side; a thin wall means the walk behind it)
        let (px, py) = self.to_world(x, y, z);
        let b = WorldBox::of(&it, &si);
        let room = si.x >= av.x && si.y >= av.y;
        let (px, py) = (px + av.x * 16, py + av.y * 16);
        if !room {
            return Some(((px, py, z), false));
        }
        // all of him on it
        let (px, py) = (px.clamp(b.x - b.xd + av.x * 32, b.x), py.clamp(b.y - b.yd + av.y * 32, b.y));
        let small = si.x <= av.x + 1 && si.y <= av.y + 1;
        Some(((px, py, z), small))
    }

    /// The player wants to jump; `pointer` is the mouse position in the
    /// view. Returns what happens, for the log.
    pub fn jump(&mut self, pointer: Option<(i32, i32)>, view_w: i32, view_h: i32) -> &'static str {
        let s = self.state(AVATAR);
        // walking or running is no hindrance: the jump takes over from it
        let moving = matches!(self.mover(AVATAR).last_anim(), anim::WALK | anim::RUN) && !self.mover(AVATAR).falling();
        if !self.player_in_control() || s.dead || (self.mover(AVATAR).busy() && !moving) {
            return "not now";
        }
        let av = self.avatar();
        let Some(info) = self.data.infos.get(av.shape as usize).copied() else { return "not now" };

        // face the pointer. A raised ledge is drawn higher up in the view
        // than where it lies, so the direction on screen can be off by an
        // eighth; where the pointer is on ground, its place in the world
        // gives the right direction.
        let was_facing = self.mover(AVATAR).dir;
        if let Some((x, y)) = pointer {
            let (dx, dy) = ((x - view_w / 2) as f32, (y - view_h / 2) as f32);
            let on_ground = self.ground_at(x, y).map(|g| (g.0 - av.x, g.1 - av.y)).filter(|&(gx, gy)| gx.abs().max(gy.abs()) > 16);
            if let Some((gx, gy)) = on_ground {
                self.mover(AVATAR).dir = world_dir(gx, gy) as u8;
            } else if dx.abs() + dy.abs() > 12.0 {
                let octant = (dx.atan2(-2.0 * dy) / std::f32::consts::FRAC_PI_4).round() as i32;
                self.mover(AVATAR).dir = self.unturn_dir((octant + 7).rem_euclid(8) as u8);
            }
        }

        // The air walk spell: a jump carries far through the air, straight
        // ahead (this is how the leap of faith is crossed).
        let running = self.mover(AVATAR).last_anim() == anim::RUN;
        if self.airwalk && !running {
            return if self.animate(AVATAR, anim::AIRWALK_JUMP) { "air walk" } else { "not possible" };
        }

        // out of a run: the long running jump ahead, as in the original
        if running {
            return if self.animate(AVATAR, anim::RUNNING_JUMP) { "running jump" } else { "not possible" };
        }

        // 1. a spot on the ground within reach: land exactly there
        if let Some(target) = pointer.and_then(|(x, y)| self.ground_at(x, y)) {
            let reach = self.state(AVATAR).strength * 32;
            let (dx, dy, dz) = (target.0 - av.x, target.1 - av.y, target.2 - av.z);
            let (xd, yd, zd) = info.footpad(false);
            let landing = WorldBox { x: target.0, y: target.1, z: target.2, xd, yd, zd };
            let here = WorldBox::of(&av, &info);
            let free = self.map().position_info(&landing, &here, &self.data.infos, AVATAR).valid;
            if dx.abs() < reach && dy.abs() < reach && (-256..=8).contains(&dz) && dx.abs().max(dy.abs()) > 48 && free {
                self.mover(AVATAR).dir = world_dir(dx, dy) as u8;
                let mut mover = self.actors.remove(&AVATAR).unwrap();
                // the "jump up" animation is steered to the target (as in the
                // game's official update)
                let ok = mover.play_steered(anim::JUMP_UP, &av, &mut self.data.anims, target);
                self.actors.insert(AVATAR, mover);
                if ok {
                    return "to the pointer";
                }
            }
        }

        // 2. standing in front of a ledge: climb the highest one that works.
        // Tried towards the pointer first, then the way he faced before
        // (usually towards the ledge he walked up to), then the eighths
        // beside the pointer's direction.
        let aimed = self.mover(AVATAR).dir;
        if self.try_climb(&[aimed, was_facing, (aimed + 1) % 8, (aimed + 7) % 8], false) {
            return "climbing";
        }

        // 4. the pointer is on nothing he could land on: he shakes his head
        if pointer.is_some() {
            self.animate(AVATAR, anim::SHAKE_HEAD);
            return "no (shakes his head)";
        }
        if self.animate(AVATAR, anim::JUMP) { "ahead" } else { "not possible" }
    }
}
