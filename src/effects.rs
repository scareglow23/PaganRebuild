//! Engine-side processes scripts can start and wait for: walking an actor
//! to a place, throwing items, camera moves and screen fades.

use crate::actor::{MoveInput, Mover};
use crate::anim;
use crate::game::{Game, AVATAR};
use crate::intrinsics::world_dir;
use crate::map::{Item, WorldBox};
use crate::view::Surface;

/// State of an actor walking to a target.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Pathfind {
    pub actor: u16,
    /// Walk to this item (0: to `pos`).
    pub target: u16,
    pub pos: (i32, i32),
    /// Walk right up to the target (to attack it) instead of stopping at
    /// talking distance.
    pub touch: bool,
    /// A way has been searched for the current goal.
    planned: bool,
    /// Where the target was when the way was searched.
    goal: (i32, i32),
    /// Next point of the way to walk to.
    next: u16,
    /// How often a new way was searched.
    replans: u8,
    /// Position at the last progress check.
    last: (i32, i32),
    /// Direction offset while walking around an obstacle, and for how long.
    detour: (i8, u32),
    stuck: u32,
    ticks: u32,
    /// Sent by a script (not kept in savegames): as in the original it
    /// gives up at once when there is no way (the golem told to open a
    /// door opens it from where he stands).
    #[serde(skip)]
    pub scripted: bool,
}

impl Pathfind {
    pub fn new(actor: u16, target: u16, pos: (i32, i32)) -> Pathfind {
        Pathfind { actor, target, pos, touch: false, planned: false, goal: (0, 0), next: 0, replans: 0, last: (i32::MIN, i32::MIN), detour: (0, 0), stuck: 0, ticks: 0, scripted: false }
    }

    /// No progress for a while: look for another way, or sidestep.
    fn check_progress(&mut self, it: &Item) {
        if !self.ticks.is_multiple_of(10) {
            return;
        }
        if (it.x, it.y) == self.last {
            self.stuck += 1;
            if self.replans < 8 {
                self.planned = false;
                self.replans += 1;
            }
            self.detour = (DETOURS[self.stuck as usize % DETOURS.len()], 25);
        } else if self.detour.1 == 0 {
            self.stuck = 0;
        }
        self.last = (it.x, it.y);
    }
}

/// How hard a move by (dx, dy, dz) strikes what it meets: the force the
/// scripts get with "got hit" and "hit" (a small spider takes half of it
/// as damage when stepped on).
pub fn hit_force(dx: i32, dy: i32, dz: i32) -> u16 {
    let (dx, dy, dz) = (dx.abs() / 4, dy.abs() / 4, dz.abs());
    ((dx + dy + dz + dy.max(dz)) / 2) as u16
}

/// An item flying through the air.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Hurl {
    pub item: u16,
    pub speed: (i32, i32, i32),
    pub gravity: i32,
}

/// A ball of fire chasing someone, with a tail of flames behind it.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Fireball {
    pub item: u16,
    target: u16,
    speed: (i32, i32),
    age: u32,
    tail: [u16; 3],
}

/// The fire Flame Sting leaves where it strikes: after a moment a fire
/// field burns there for three seconds, then a flame dies down.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FlameSting {
    pub at: (i32, i32, i32),
    pub field: u16,
    pub age: u32,
}

/// What the view is centred on.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum CameraMode {
    Follow(u16),
    Fixed(i32, i32, i32),
    /// Gliding from one point to another: (from, to, elapsed, duration).
    Scroll((i32, i32, i32), (i32, i32, i32), u32, u32),
}

/// A colour laid over the whole view, fading between two strengths (0..=256).
#[derive(Clone, Copy)]
pub struct Fade {
    pub rgb: (u32, u32, u32),
    /// The colour it starts from: another than `rgb` when it takes over
    /// from a fade to another colour (white, then black at the end of the
    /// game), so that the picture does not show again in between.
    pub from_rgb: (u32, u32, u32),
    pub from: i32,
    pub to: i32,
    pub elapsed: i32,
    pub steps: i32,
}

impl Fade {
    pub fn new(rgb: (u32, u32, u32), from: i32, to: i32, steps: i32) -> Fade {
        Fade { rgb, from_rgb: rgb, from, to, elapsed: 0, steps: steps.max(1) }
    }

    fn level(&self) -> i32 {
        self.from + (self.to - self.from) * self.elapsed.min(self.steps) / self.steps.max(1)
    }

    fn color(&self) -> (u32, u32, u32) {
        let (t, n) = (self.elapsed.min(self.steps) as u32, self.steps.max(1) as u32);
        let mix = |a: u32, b: u32| (a * (n - t) + b * t) / n;
        (mix(self.from_rgb.0, self.rgb.0), mix(self.from_rgb.1, self.rgb.1), mix(self.from_rgb.2, self.rgb.2))
    }
}

/// Process types of the engine's own processes (scripts can wait for
/// them or end them by type).
pub const PROC_WALK: u16 = 0x0204;
pub const PROC_LOITER: u16 = 0x0205;
pub const PROC_HURL: u16 = 0x00F1;
pub const PROC_FIREBALL: u16 = 0x0218;
pub const PROC_FLAME_STING: u16 = 0x0219;

const DETOURS: [i8; 6] = [1, -1, 2, -2, 3, -3];

/// Walking to a thing or a person ends this close to it (space between
/// the two).
const NEAR_GAP: i32 = 80;

/// The way a walking actor follows.
#[derive(Clone, Default, Debug)]
pub struct Way {
    /// Its corners, from where the actor set off.
    pub points: Vec<(i32, i32)>,
    /// Per point: the way to it goes down over an edge.
    pub drops: Vec<bool>,
}

impl From<(Vec<(i32, i32)>, Vec<bool>)> for Way {
    fn from((points, drops): (Vec<(i32, i32)>, Vec<bool>)) -> Way {
        Way { points, drops }
    }
}

impl Way {
    /// Whether the way to point `i` goes down over an edge.
    fn drop_at(&self, i: u16) -> bool {
        self.drops.get(i as usize).copied().unwrap_or(false)
    }
}

/// Is height `z` on the level a walk was sent to (if any)?
fn on_level(level: Option<i32>, z: i32) -> bool {
    level.is_none_or(|l| (z - l).abs() <= 8)
}

impl Game {
    /// One tick of a walking actor. Returns true when the walk is over.
    pub fn run_pathfind(&mut self, pf: &mut Pathfind) -> bool {
        let Some(it) = self.map().get(pf.actor).copied().filter(|it| it.in_world()) else { return true };
        // The player's click on a thing or a person: the Avatar walks up to
        // it until the space between them is small, from whatever side (by
        // the corners a big golem was out of reach from one side). Scripts
        // keep the old measure: they lead him into gates and onto marks.
        let by_gap = pf.actor == AVATAR && pf.target != 0 && self.approaching == Some(pf.target);
        let Some((tx, ty, reach)) = self.walk_goal(pf, &it, by_gap) else { return true };
        pf.ticks += 1;
        let (dx, dy) = (tx - it.x, ty - it.y);
        // sent by a click to a spot on another level: only there is he there
        let level = if pf.actor == AVATAR && pf.target == 0 { self.walk_level } else { None };
        let exact = level.is_some() && self.walk_exact;
        let reach = if exact { reach.min(12) } else { reach };
        let arrived = dx.abs().max(dy.abs()) <= reach && on_level(level, it.z);
        // Sent by the player to a spot on the ground, the Avatar keeps the
        // direction he walked in: "arrived" begins some way before the
        // spot, which can lie to the side or already behind him, and
        // turning to it would be a sudden jerk at the end. Towards a thing
        // or a person he turns to face it.
        let to_spot = pf.actor == AVATAR && pf.target == 0;
        if arrived || pf.ticks > 1500 || pf.stuck > 12 {
            // stop in the standing pose, facing the target
            self.stop_walk(pf.actor, &it, if arrived && !to_spot { (dx, dy) } else { (0, 0) });
            return true;
        }

        // Search a way around the obstacles; again if the target has moved
        // away from where it was, or the actor got stuck.
        let moved = (tx - pf.goal.0).abs().max((ty - pf.goal.1).abs()) > 96;
        if (!pf.planned || (moved && pf.replans < 8 && pf.ticks.is_multiple_of(15)))
            && !self.plan_way(pf, &it, (tx, ty), reach, level, by_gap)
            && (pf.actor == AVATAR || pf.scripted)
        {
            // the player's Avatar does not march blindly at what he cannot
            // reach (into the water, against a wall)
            self.stop_walk(pf.actor, &it, (0, 0));
            return true;
        }
        pf.check_progress(&it);

        // Right below (or above) the spot clicked, but not on its level,
        // and no way leads on: walking on would only pace to and fro there
        // until the walk gives up. (Climbing up, where it works, follows.)
        let below_goal = to_spot && level.is_some() && dx.abs().max(dy.abs()) <= reach;

        // head for the next point of the way, or straight for the target
        let heading = self.next_corner(pf, &it);
        let following = heading.is_some();
        let ((dx, dy), keep_dir) = heading.unwrap_or(((dx, dy), None));
        if following {
            pf.detour = (0, 0);
        } else if below_goal {
            self.stop_walk(pf.actor, &it, (0, 0));
            return true;
        } else if pf.actor == AVATAR && self.paths.get(&pf.actor).is_some_and(|w| !w.points.is_empty()) && !(exact && dx.abs().max(dy.abs()) <= 64) {
            // At the end of the way: it leads as close as one can get, so
            // stop rather than walk straight on (into the water, say); onto
            // something small the last bit is straight on.
            self.stop_walk(pf.actor, &it, if to_spot { (0, 0) } else { (tx - it.x, ty - it.y) });
            return true;
        }
        let mut dir = keep_dir.unwrap_or(world_dir(dx, dy) as i32);
        if pf.detour.1 > 0 {
            pf.detour.1 -= 1;
            dir += pf.detour.0 as i32;
        }
        if !self.actors.contains_key(&pf.actor) {
            let facing = self.npc_dirs.get(&pf.actor).copied().unwrap_or(4);
            self.actors.insert(pf.actor, Mover::new(facing));
        }
        // heading for a point below an edge: stepping off it is meant
        let drop = following && self.paths.get(&pf.actor).is_some_and(|w| w.drop_at(pf.next));
        if let Some(m) = self.actors.get_mut(&pf.actor) {
            m.may_drop = drop;
        }
        // (option) a step into flames that would hurt: rather stand (where
        // the way search found none around them, the walk goes straight on)
        if pf.actor != AVATAR && self.step_into_flames(&it, dir.rem_euclid(8) as usize) {
            self.stop_walk(pf.actor, &it, (0, 0));
            return true;
        }
        // the player's Avatar walks or runs as he has chosen
        let run = pf.actor == AVATAR && self.approaching.is_some() && self.run_toggle;
        self.inputs.insert(pf.actor, MoveInput { dir: dir.rem_euclid(8) as u8, run });
        false
    }

    /// Would a step of `it` in direction `dir` take it into flames it is
    /// not standing in already (option "avoid_flames")?
    fn step_into_flames(&self, it: &Item, dir: usize) -> bool {
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        let flames = self.flames_to_avoid(it);
        if flames.is_empty() {
            return false;
        }
        let Some(si) = self.data.infos.get(it.shape as usize) else { return false };
        let here = crate::map::WorldBox::of(it, si);
        let there = crate::map::WorldBox { x: here.x + 32 * XF[dir], y: here.y + 32 * YF[dir], ..here };
        flames.iter().any(|f| f.overlaps(&there) && !f.overlaps(&here))
    }

    /// Where a walk is heading and how close counts as there; None when
    /// the target is gone.
    fn walk_goal(&self, pf: &Pathfind, it: &Item, by_gap: bool) -> Option<(i32, i32, i32)> {
        if pf.target == 0 {
            return Some((pf.pos.0, pf.pos.1, 48));
        }
        let t = self.map().get(pf.target).copied().filter(|t| t.in_world())?;
        let pad = self.data.infos.get(t.shape as usize).map_or(0, |si| 32 * si.x.max(si.y));
        // A mark a script sends someone to (a small thing one walks over,
        // not into): onto it, the middle of the figure on the mark. (The
        // measure below counted the Avatar as there a hundred units off:
        // at the end of the game he stayed at the plates instead of
        // stepping back to watch the obelisk rise.)
        let mark = pf.scripted && !pf.touch && self.data.infos.get(t.shape as usize).is_some_and(|si| !si.solid && si.x <= 1 && si.y <= 1);
        if mark {
            let (axd, ayd) = self.data.infos.get(it.shape as usize).map_or((64, 64), |si| (si.x * 32, si.y * 32));
            return Some((t.x + axd / 2, t.y + ayd / 2, 16));
        }
        Some(if (pf.touch && self.box_gap(it, &t) <= 12) || (by_gap && self.box_gap(it, &t) <= NEAR_GAP) {
            (it.x, it.y, 1) // arrived
        } else {
            (t.x, t.y, if pf.touch { 0 } else { 80 + pad })
        })
    }

    /// Searches (again) the way of a walk to `(tx, ty)` and keeps it in
    /// `paths`. Returns whether there is one.
    fn plan_way(&mut self, pf: &mut Pathfind, it: &Item, (tx, ty): (i32, i32), reach: i32, level: Option<i32>, by_gap: bool) -> bool {
        if pf.planned {
            pf.replans += 1;
        }
        pf.planned = true;
        pf.goal = (tx, ty);
        pf.next = 0;
        // from the same place to the same goal as last time: nothing new
        // to find, keep the way found then (and do not pay for it again)
        let tried = (it.x >> 5, it.y >> 5, tx >> 5, ty >> 5);
        // (not for the player's Avatar: a second click must walk him)
        let again = pf.actor != AVATAR && self.path_tried.get(&pf.actor) == Some(&tried);
        self.path_tried.insert(pf.actor, tried);
        let way = if again {
            // (a kept way already starts where it was searched from)
            self.paths.get(&pf.actor).cloned().map(|mut w| {
                if !w.points.is_empty() {
                    w.points.remove(0);
                    w.drops.remove(0);
                }
                w
            })
        } else {
            self.search_way(pf, it, (tx, ty), reach, level, by_gap)
        };
        match way {
            Some(mut w) => {
                // starting from here: the first stretch has a direction too
                if !w.points.is_empty() {
                    w.points.insert(0, (it.x, it.y));
                    w.drops.insert(0, false);
                }
                // The player's click on a spot he cannot get to (in the
                // lava): the way ends as near as it goes, right at the
                // shore, and the last stride carried him on into the lava,
                // where he stood and burnt. At a shore it ends a step short.
                let n = w.points.len();
                if pf.actor == AVATAR && pf.target == 0 && !pf.scripted && n >= 2 && !w.drops[n - 1] && self.near_sea(it, w.points[n - 1]) {
                    let (ex, ey) = w.points[n - 1];
                    let (px, py) = w.points[n - 2];
                    let (sx, sy) = (ex - px, ey - py);
                    let len = ((sx * sx + sy * sy) as f32).sqrt();
                    if (tx - ex).abs().max((ty - ey).abs()) > reach.max(40) && len > 64.0 {
                        let k = 32.0 / len;
                        w.points[n - 1] = (ex - (sx as f32 * k) as i32, ey - (sy as f32 * k) as i32);
                    }
                }
                self.paths.insert(pf.actor, w);
                true
            }
            None => {
                self.paths.remove(&pf.actor);
                false
            }
        }
    }

    /// The way search of `plan_way`.
    fn search_way(&self, pf: &Pathfind, it: &Item, (tx, ty): (i32, i32), reach: i32, level: Option<i32>, by_gap: bool) -> Option<Way> {
        let goal_item = self.map().get(pf.target).copied();
        let may_drop = pf.actor == AVATAR;
        let near = |at: &Item, need_level: bool| match (&goal_item, pf.touch) {
            (Some(t), true) => self.box_gap(at, t) <= 12,
            (Some(t), false) if by_gap => self.box_gap(at, t) <= NEAR_GAP,
            _ => (tx - at.x).abs().max((ty - at.y).abs()) <= reach.max(40) && (!need_level || on_level(level, at.z)),
        };
        // a way down to (or up the stairs to) the clicked level first;
        // if there is none, as near as it goes (ledges are climbed then)
        let found = match level {
            // A goal higher up (or as high) without a way there (it is
            // climbed): as near as it goes without going down, for the
            // nearest point with drops can be the ground below the goal,
            // from where it is out of reach. A way down over edges is
            // taken instead if it ends clearly nearer (a
            // step down to the wall walk and up the pillar across it);
            // the drops there end only where the climb up is not too high.
            Some(l) if l >= it.z - 8 => self.find_path_there(it, (tx, ty), may_drop, |at| near(at, true)).or_else(|| {
                let level_way = self.find_path_to_climb(it, (tx, ty), false, l, |at| near(at, false));
                let down_way = if may_drop { self.find_path_to_climb(it, (tx, ty), true, l, |at| near(at, false)) } else { None };
                let gap = |w: &Option<(Vec<(i32, i32)>, Vec<bool>)>| {
                    let end = w.as_ref().and_then(|(p, _)| p.last().copied()).unwrap_or((it.x, it.y));
                    (tx - end.0).abs().max((ty - end.1).abs())
                };
                if down_way.is_some() && gap(&down_way) + 48 < gap(&level_way) { down_way } else { level_way.or(down_way) }
            }),
            Some(_) => self.find_path(it, (tx, ty), may_drop, |at| near(at, true)),
            None => self.find_path(it, (tx, ty), may_drop, |at| near(at, false)),
        };
        found.map(Way::from)
    }

    /// The next corner of the way to walk to, as an offset from the actor,
    /// and the direction to keep while close to the planned line; None
    /// past the end of the way (or without one).
    fn next_corner(&self, pf: &mut Pathfind, it: &Item) -> Option<((i32, i32), Option<i32>)> {
        let way = self.paths.get(&pf.actor)?;
        let path = &way.points;
        while let Some(&(wx, wy)) = path.get(pf.next as usize) {
            // a corner counts as reached when close, or when already
            // passed (a stride can be longer than the margin)
            // (not where one lands below an edge: passing it up on the
            // edge, he would walk on up there instead of stepping off)
            let passed = pf.next > 0 && !way.drop_at(pf.next) && {
                let (px, py) = path[pf.next as usize - 1];
                (wx - px) * (wx - it.x) + (wy - py) * (wy - it.y) <= 0
            };
            if passed || (wx - it.x).abs().max((wy - it.y).abs()) <= 28 {
                pf.next += 1;
                continue;
            }
            let (dx, dy) = (wx - it.x, wy - it.y);
            // Close to the planned line: keep its direction. Aiming at the
            // corner from a little to the side would switch between two
            // directions every few steps.
            let mut keep_dir = None;
            if pf.next > 0 {
                let (px, py) = path[pf.next as usize - 1];
                let (sx, sy) = (wx - px, wy - py);
                let len = ((sx * sx + sy * sy) as f32).sqrt().max(1.0);
                let off = ((sx * dy - sy * dx) as f32).abs() / len;
                // (not close to the end of the player's way at a shore:
                // there the line's direction carried him past the end to
                // the side into the lava, where he stood and burnt)
                let at_end = pf.actor == AVATAR && !pf.scripted && pf.next as usize + 1 == path.len() && dx.abs().max(dy.abs()) <= 64 && self.near_sea(it, (wx, wy));
                if off <= 40.0 && !at_end {
                    keep_dir = Some(world_dir(sx, sy) as i32);
                }
            }
            return Some(((dx, dy), keep_dir));
        }
        None
    }

    /// Would `it`, standing at `(x, y)`, be within a stride of water or
    /// lava (the shore where a walk must not carry on)?
    fn near_sea(&self, it: &Item, (x, y): (i32, i32)) -> bool {
        const STRIDE: i32 = 24;
        let Some(si) = self.data.infos.get(it.shape as usize) else { return false };
        let b = crate::map::WorldBox::of(it, si);
        let there = crate::map::WorldBox { x: x + STRIDE, y: y + STRIDE, xd: b.xd + 2 * STRIDE, yd: b.yd + 2 * STRIDE, ..b };
        self.map().position_info(&there, &there, &self.data.infos, it.id).touches_sea
    }

    /// Ends a walk in the standing pose, turned towards `face` if given.
    pub(crate) fn stop_walk(&mut self, actor: u16, it: &Item, face: (i32, i32)) {
        self.inputs.remove(&actor);
        self.paths.remove(&actor);
        if let Some(mover) = self.actors.get_mut(&actor) {
            mover.may_drop = false;
            if face != (0, 0) {
                mover.dir = world_dir(face.0, face.1) as u8;
            }
            mover.play(anim::STAND, it, &mut self.data.anims);
        }
    }

    /// Free space between the footprints of two items (0 if they touch).
    pub fn box_gap(&self, a: &Item, b: &Item) -> i32 {
        let pad = |i: &Item| self.data.infos.get(i.shape as usize).map_or((64, 64), |si| (si.x * 32, si.y * 32));
        let ((axd, ayd), (bxd, byd)) = (pad(a), pad(b));
        let gx = ((a.x - axd) - b.x).max((b.x - bxd) - a.x).max(0);
        let gy = ((a.y - ayd) - b.y).max((b.y - byd) - a.y).max(0);
        gx.max(gy)
    }

    /// One tick of a flying item. Returns true when it has come to rest.
    pub fn run_hurl(&mut self, h: &mut Hurl) -> bool {
        let Some(idx) = self.map().index(h.item) else { return true };
        let it = self.map().items[idx];
        let Some(si) = self.data.infos.get(it.shape as usize).copied() else { return true };
        if !it.in_world() {
            return true;
        }
        h.speed.2 -= h.gravity;
        let (xd, yd, zd) = si.footpad(false);
        let here = WorldBox { x: it.x, y: it.y, z: it.z, xd, yd, zd };
        // move in small steps so nothing is skipped
        let steps = h.speed.0.abs().max(h.speed.1.abs()).max(h.speed.2.abs()).max(1);
        let (mut x, mut y, mut z) = (it.x, it.y, it.z);
        let mut landed = false;
        let mut struck = None;
        let speed = h.speed;
        for s in 1..=steps {
            let (nx, ny, nz) = (it.x + speed.0 * s / steps, it.y + speed.1 * s / steps, (it.z + speed.2 * s / steps).max(0));
            let at = |x, y, z| self.map().position_info(&WorldBox { x, y, z, xd, yd, zd }, &here, &self.data.infos, it.id);
            let info = at(nx, ny, nz);
            if !info.valid {
                struck = info.blocker;
                // which ways are blocked (a corner counts as both sides)
                let (mut bx, mut by) = (nx != x && !at(nx, y, z).valid, ny != y && !at(x, ny, z).valid);
                let bz = nz != z && !at(x, y, nz).valid;
                if !bx && !by && !bz {
                    (bx, by) = (nx != x, ny != y);
                }
                if bz && !bx && !by && speed.2 < 0 {
                    // came down on something: rests there, or slips off its edge
                    let below = at(x, y, z);
                    if below.supported || z == 0 {
                        landed = true;
                    } else {
                        h.speed.2 = 0;
                        self.slip_off_edge(h, struck, (x - xd / 2, y - yd / 2));
                    }
                } else {
                    // struck from the side (or from below): bounces back at half the speed
                    if bx {
                        h.speed.0 = -h.speed.0 / 2;
                    }
                    if by {
                        h.speed.1 = -h.speed.1 / 2;
                    }
                    if bz {
                        h.speed.2 = -h.speed.2 / 2;
                    }
                }
                // wedged in so that it can neither move nor fall: it stays
                if (x, y, z) == (it.x, it.y, it.z) && h.speed.2 <= 0 && (z == 0 || !at(x, y, z - 1).valid) {
                    landed = true;
                }
                break;
            }
            (x, y, z) = (nx, ny, nz);
            if speed.2 <= 0 && (info.supported || nz == 0) {
                landed = true;
                struck = info.support;
                break;
            }
        }
        self.map_mut().place(idx, x, y, z);
        if landed {
            if let Some(tile) = self.path_landing(h.item) {
                struck = Some(tile);
            }
        }
        if let Some(other) = struck {
            // as in the original: what was struck "got hit", the flying
            // thing "hit" it (grenades burst, darts stick, water swallows)
            let force = hit_force(speed.0, speed.1, speed.2);
            self.call_event(other, 6, &[h.item.to_le_bytes(), force.to_le_bytes()].concat());
            self.call_event(h.item, 5, &[other.to_le_bytes(), force.to_le_bytes()].concat());
        }
        landed
    }

    /// A flying thing came down on the edge of `below` without being
    /// carried: it slips off that edge (its centre at `cx`, `cy`), at least
    /// 16 a tick sideways.
    fn slip_off_edge(&self, h: &mut Hurl, below: Option<u16>, (cx, cy): (i32, i32)) {
        let Some(b) = below.and_then(|o| self.map().get(o).copied()) else { return };
        let (bxd, byd) = self.data.infos.get(b.shape as usize).map_or((64, 64), |si| (si.x * 32, si.y * 32));
        if h.speed.0.abs() < 16 {
            h.speed.0 = if cx + 16 > b.x { 16 } else if cx - 16 < b.x - bxd { -16 } else { h.speed.0 };
        }
        if h.speed.1.abs() < 16 {
            h.speed.1 = if cy + 16 > b.y { 16 } else if cy - 16 < b.y - byd { -16 } else { h.speed.1 };
        }
    }

    /// The world position the view is centred on.
    pub fn camera_position(&self) -> (i32, i32, i32) {
        let follow = |id: u16| {
            let it = self.map().get(id).copied().filter(|it| it.in_world()).unwrap_or_else(|| self.avatar());
            (it.x, it.y, it.z + 20)
        };
        let (x, y, z) = match self.camera {
            CameraMode::Follow(id) => follow(id),
            CameraMode::Fixed(x, y, z) => (x, y, z),
            CameraMode::Scroll(a, b, t, total) => {
                let f = |a: i32, b: i32| a + (b - a) * t.min(total) as i32 / total.max(1) as i32;
                (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
            }
        };
        // an earthquake pushes the view around (the offset is in screen pixels)
        let (qx, qy) = self.quake_offset;
        (x + 2 * qx + 4 * qy, y - 2 * qx + 4 * qy, z)
    }

    /// An item is thrown so that it comes down at a point: `speed` is how
    /// far it travels per tick, gravity pulls it down a little more each tick.
    pub fn shoot(&mut self, id: u16, target: (i32, i32, i32), speed: i32, gravity: i32) {
        let Some(it) = self.map().get(id).copied().filter(|it| it.in_world()) else { return };
        let speed = speed.max(1);
        let range = (it.x - target.0).abs() + (it.y - target.1).abs();
        let mut frames = (range + speed / 2) / speed;
        let rise = target.2 - it.z;
        let vz_for = |frames: i32| (gravity * frames * (frames - 1) / 2 + rise) / frames;
        let velocity = if frames > 0 {
            // not steeper than a quarter of the speed: take longer instead
            if vz_for(frames) > speed / 4 {
                if gravity == 0 || speed / (4 * gravity) <= frames {
                    if speed >= 4 && rise / (speed / 4) > frames {
                        frames = rise / (speed / 4);
                    }
                } else {
                    frames = speed / (4 * gravity);
                }
            }
            let frames = frames.max(1);
            ((target.0 - it.x + frames / 2) / frames, (target.1 - it.y + frames / 2) / frames, vz_for(frames))
        } else {
            (0, 0, if rise > 0 { speed / 4 } else { -speed / 4 })
        };
        self.add_native(crate::vm::Kind::Hurl(Hurl { item: id, speed: velocity, gravity }), id, PROC_HURL);
    }

    /// A homing fireball starts at a point and chases the Avatar.
    pub fn launch_fireball(&mut self, pos: (i32, i32, i32)) {
        let cur = self.cur_map as u16;
        let id = self.map_mut().add(Item {
            x: pos.0,
            y: pos.1,
            z: pos.2,
            shape: 260,
            frame: 4,
            mapnum: cur,
            flags: crate::eggs::FLG_FAST_ONLY,
            dead: true,
            fast: true,
            ..Item::default()
        });
        if let Some(idx) = self.map().index(id) {
            self.fast_items.push(idx);
        }
        let ball = Fireball { item: id, target: crate::game::AVATAR, speed: (0, 0), age: 0, tail: [0; 3] };
        self.add_native(crate::vm::Kind::Fireball(ball), id, PROC_FIREBALL);
    }

    /// One tick of a fireball. Returns true when it is spent.
    pub fn run_fireball(&mut self, f: &mut Fireball) -> bool {
        f.age += 1;
        let (Some(it), Some(t)) = (self.map().get(f.item).copied(), self.map().get(f.target).copied()) else {
            self.end_fireball(f);
            return true;
        };
        if f.age > 300 && self.rng.next().is_multiple_of(20) {
            self.end_fireball(f);
            return true;
        }
        // it is pulled towards the target, up to a top speed
        let (dx, dy) = (t.x - it.x, t.y - it.y);
        let dir = world_dir(dx, dy) as usize;
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        if f.speed == (0, 0) && dx / 64 == 0 && dy / 64 == 0 {
            f.speed = (2 * XF[dir], 2 * YF[dir]);
        } else {
            f.speed = (f.speed.0 + dx / 64, f.speed.1 + dy / 64);
        }
        let v = ((f.speed.0 * f.speed.0 + f.speed.1 * f.speed.1) as f32).sqrt() as i32;
        if v > 32 {
            f.speed = (f.speed.0 * 32 / v, f.speed.1 * 32 / v);
        }
        let (nx, ny) = (it.x + f.speed.0, it.y + f.speed.1);

        // does it run into someone or something?
        let ball = WorldBox { x: nx, y: ny, z: it.z, xd: 32, yd: 32, zd: 16 };
        let victim = self.map().items_near(nx, ny, 256).into_iter().find_map(|i| {
            let o = &self.map().items[i];
            let oi = self.data.infos.get(o.shape as usize)?;
            (self.can_fight(o) && !o.dead && ball.overlaps(&WorldBox::of(o, oi))).then_some(o.id)
        });
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        let blocked = victim.is_none() && !self.map().position_info(&ball, &nowhere, &self.data.infos, f.item).valid;
        if !blocked {
            if let Some(idx) = self.map().index(f.item) {
                self.map_mut().place(idx, nx, ny, it.z);
            }
        }

        // the oldest piece of the tail moves to where the ball just was
        if f.tail[2] == 0 {
            let cur = self.cur_map as u16;
            f.tail[2] = self.map_mut().add(Item {
                shape: 261,
                mapnum: cur,
                flags: crate::eggs::FLG_FAST_ONLY | crate::eggs::FLG_DISPOSABLE,
                dead: true,
                parent: crate::map::ETHEREAL,
                ..Item::default()
            });
        }
        let piece = f.tail[2];
        if let Some(idx) = self.map().index(piece) {
            self.map_mut().items[idx].frame = world_dir(f.speed.0, f.speed.1);
            self.map_mut().place(idx, it.x, it.y, it.z);
        }
        f.tail = [piece, f.tail[0], f.tail[1]];

        if let Some(v) = victim {
            let damage = 5 + (self.rng.next() % 5) as i32;
            // the fireballs are the enemy casters' (they always fly at the Avatar)
            self.receive_hit_by(v, 0, ((dir + 4) & 7) as u8, damage, 0x0008, true);
            self.end_fireball(f);
            return true;
        }
        if blocked {
            f.speed = (-f.speed.0, -f.speed.1); // bounce off
        }
        false
    }

    /// One tick of the fire left by Flame Sting. Returns true when it is out.
    pub fn run_flame_sting(&mut self, f: &mut FlameSting) -> bool {
        const FIRE_FIELD: u32 = 400;
        const DYING_FLAME: u32 = 381;
        f.age += 1;
        if f.age == 3 {
            let cur = self.cur_map as u16;
            let (x, y, z) = f.at;
            f.field = self.map_mut().add(crate::map::Item {
                x,
                y,
                z,
                shape: FIRE_FIELD,
                mapnum: cur,
                flags: crate::eggs::FLG_FAST_ONLY | crate::eggs::FLG_DISPOSABLE,
                fast: true,
                ..crate::map::Item::default()
            });
            if let Some(idx) = self.map().index(f.field) {
                self.fast_items.push(idx);
            }
        }
        if f.age < 3 + 90 {
            return false;
        }
        if let Some(idx) = self.map().index(f.field) {
            self.destroy_with_scripts(idx);
        }
        self.create_sprite(DYING_FLAME, 0, 9, 1, 2, f.at);
        true
    }

    fn end_fireball(&mut self, f: &Fireball) {
        for id in [f.item, f.tail[0], f.tail[1], f.tail[2]] {
            if id != 0 {
                if let Some(idx) = self.map().index(id) {
                    self.destroy_with_scripts(idx);
                }
            }
        }
    }

    /// A frame of an animation is marked for something special. Returns
    /// false if it is just a footfall.
    pub fn special_frame(&mut self, actor: &Item, action: u32) -> bool {
        const GHOST: u32 = 0x19B;
        const SKULL: u32 = 0x19D;
        const GHOUL: u32 = 0x8E;
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        let id = actor.id;
        if id == crate::game::AVATAR && (action == anim::READY_WEAPON || action == anim::UNREADY_WEAPON) {
            // the weapon leaves or enters its sheath
            if self.weapon_info(id).is_some() {
                let sfx = if self.rng.next() & 1 == 0 { 0x51 } else { 0x52 };
                self.audio.play_sfx(sfx, id, false, 1.0);
            }
            return true;
        }
        let count_near = |g: &Game, shape: u32, range: i32| {
            g.map().items_near(actor.x, actor.y, range).into_iter().filter(|&i| g.map().items[i].shape == shape).count()
        };
        if actor.shape == GHOST {
            let target = self.state(id).target;
            let helper = if action == anim::ATTACK {
                // the ghost throws a flaming skull that hunts its enemy
                if count_near(self, SKULL, 6 * 256) > 5 {
                    return true;
                }
                let dir = self.mover(id).dir as usize;
                let at = Item { x: actor.x + 32 * XF[dir], y: actor.y + 32 * YF[dir], z: actor.z, quality: SKULL as u16, ..Item::default() };
                // it starts out inside the ghost, so room is not asked for
                self.create_monster(&at, false)
            } else if self.cur_map != 54 {
                // otherwise it calls a ghoul out of the ground
                if count_near(self, GHOUL, 8 * 256) > 2 {
                    return true;
                }
                // (on firm ground, not on the black plates of the void
                // between the caves: there a ghoul could not be reached,
                // and half of it showed through the dark)
                let jitter = |g: &mut Game| (g.rng.next() % (6 * 256 + 1)) as i32 - 3 * 256;
                let mut ghoul = 0;
                for _ in 0..6 {
                    let at = Item { x: actor.x + jitter(self), y: actor.y + jitter(self), z: actor.z, quality: GHOUL as u16, ..Item::default() };
                    if self.firm_ground(GHOUL, at.x, at.y, at.z) {
                        ghoul = self.hatch_monster(&at);
                        break;
                    }
                }
                if ghoul != 0 {
                    self.animate(ghoul, anim::STAND_UP);
                }
                ghoul
            } else {
                0
            };
            if helper != 0 {
                self.set_in_combat(helper, true);
                self.state_mut(helper).target = target;
            }
            return true;
        }
        if actor.shape == SKULL {
            // the skull bursts when it has reached the Avatar
            let av = self.avatar();
            if (av.x - actor.x).abs().max((av.y - actor.y).abs()) < 96 + 64 {
                self.state_mut(id).dead = true;
                self.actors.remove(&id);
                self.explode(id);
            }
            return true;
        }
        false
    }

    /// Advances camera glides and fades by one tick.
    pub fn tick_effects(&mut self) {
        self.data.pal.tick();
        self.quake_offset = if self.quake > 0 {
            let q = self.quake.min(8);
            let mut r = || (self.rng.next() % (2 * q as u32 + 1)) as i32 - q;
            (r(), r())
        } else {
            (0, 0)
        };
        if let CameraMode::Scroll(a, b, t, total) = self.camera {
            self.camera = if t + 1 >= total { CameraMode::Fixed(b.0, b.1, b.2) } else { CameraMode::Scroll(a, b, t + 1, total) };
        }
        if let Some(f) = &mut self.fade {
            f.elapsed += 1;
            // a finished fade to "nothing" is over, a fade to black stays
            if f.elapsed >= f.steps && f.to == 0 {
                self.fade = None;
            }
        }
    }

    pub fn start_fade(&mut self, rgb: (u32, u32, u32), from: i32, to: i32, steps: i32) -> u32 {
        // continue from whatever is on screen now; fading out to another
        // colour than the one on screen, from that colour
        let mut fade = Fade::new(rgb, from, to, steps);
        match self.fade {
            Some(f) if f.rgb == rgb => fade.from = f.level(),
            Some(f) if to > from && f.level() > 0 => {
                fade.from = f.level();
                fade.from_rgb = f.color();
            }
            _ => {}
        }
        self.fade = Some(fade);
        self.add_native(crate::vm::Kind::Delay(steps.max(1)), 0, 0) as u32
    }

    /// Lays the current fade colour over the finished frame.
    pub fn draw_fade(&self, surf: &mut Surface) {
        let Some(f) = self.fade else { return };
        let a = f.level().clamp(0, 256) as u32;
        if a == 0 {
            return;
        }
        let ia = 256 - a;
        let rgb = f.color();
        for p in &mut surf.pixels {
            let ch = |shift: u32, c: u32| ((((*p >> shift) & 0xFF) * ia + c * a) >> 8) << shift;
            *p = ch(16, rgb.0) | ch(8, rgb.1) | ch(0, rgb.2);
        }
    }

    /// The Avatar may be steered by the player.
    pub fn player_in_control(&self) -> bool {
        !self.stasis && self.ask.is_none() && !self.inputs.contains_key(&AVATAR)
    }

    /// The player may give the Avatar a new order: as `player_in_control`,
    /// but a walk the player sent him on (a click, going for an enemy,
    /// climbing on the way) does not count - a new order replaces it.
    pub fn player_may_order(&self) -> bool {
        !self.stasis && self.ask.is_none() && (!self.inputs.contains_key(&AVATAR) || self.approaching.is_some() || self.climb_goal.is_some())
    }
}
