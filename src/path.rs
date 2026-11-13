//! Finding a way for an actor through the world, and the daily life of
//! the inhabitants: the hourly "schedule" scripts and idle wandering.

use crate::game::Game;
use crate::hash::HashMap;
use crate::map::{Item, MapData, PositionInfo, WorldBox};
use crate::shape::ShapeInfo;
use crate::vm::Kind;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Distance between the points a path is searched on.
const STEP: i32 = 32;
/// How many positions a search may look at before it gives up.
const MAX_NODES: usize = 1500;
/// For anyone but the Avatar (creatures closing in, people on their way):
/// their ways are short, and a search that finds none (the enemy up on a
/// wall) costs a good part of a frame at the full count.
const MAX_NODES_OTHERS: usize = 500;
/// How far a way may lead down over an edge (with drops allowed).
const MAX_DROP: i32 = 96;
/// Extra cost of a drop: a way on foot is preferred if it is not much longer.
const DROP_COST: i32 = 20;

/// Someone wandering about near where they are.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Loiter {
    pub actor: u16,
    /// Ticks to stand around before the next stroll.
    wait: i32,
}

impl Loiter {
    pub fn new(actor: u16) -> Loiter {
        Loiter { actor, wait: 0 }
    }
}

struct Node {
    pos: (i32, i32, i32),
    cost: i32,
    parent: usize,
    /// Direction of the step that led here (8: none yet).
    dir: u8,
    /// That step goes down over an edge.
    drop: bool,
}

/// Extra cost of changing direction. Among the many equally short ways
/// over a grid it picks one with few turns, so the walker does not zigzag.
const TURN: i32 = 3;

/// Where a way may end that does not get to the goal.
#[derive(Clone, Copy, PartialEq)]
enum Ends {
    /// At the nearest point it gets to.
    Nearest,
    /// Nowhere: only a way that gets there.
    There,
    /// At the nearest point with nothing overhead up to this height (the
    /// first), not higher than a step above it and not lower than the
    /// second. (Not up on the floor above a spot clicked below: from there
    /// it is no nearer.)
    OpenUpTo(i32, i32),
}

impl Game {
    /// (Option) the flames around that hurt whoever walks into them: the
    /// fire shields of a ring of fire and burning fire fields. People keep
    /// out of them (Malchir no longer burns himself on his own ring); the
    /// Avatar goes where he is sent.
    pub(crate) fn flames_to_avoid(&self, actor: &Item) -> Vec<WorldBox> {
        const FIRE_SHIELD: u32 = 569;
        const FIRE_FIELD: u32 = 400;
        if !self.options.avoid_flames || actor.id == crate::game::AVATAR {
            return Vec::new();
        }
        let map = self.map();
        map.items_near(actor.x, actor.y, 2048)
            .into_iter()
            .map(|i| &map.items[i])
            .filter(|it| it.in_world() && (it.shape == FIRE_SHIELD || (it.shape == FIRE_FIELD && it.frame <= 9)))
            .filter_map(|it| Some(WorldBox::of(it, self.data.infos.get(it.shape as usize)?)))
            .collect()
    }

    /// Searches a walkable way for `actor` to a place. `near(x, y)` says
    /// whether a position is close enough; `may_drop`: the way may lead
    /// down over edges. Returns the points to walk through, each with
    /// whether the way to it goes down over an edge; if the place cannot
    /// be reached, a way to the nearest reachable point, or None if there
    /// is no getting closer.
    pub fn find_path(&self, actor: &Item, goal: (i32, i32), may_drop: bool, near: impl Fn(&Item) -> bool) -> Option<(Vec<(i32, i32)>, Vec<bool>)> {
        self.timed_search(actor, goal, may_drop, Ends::Nearest, near)
    }

    /// As find_path, but only a way that gets there (None otherwise).
    pub fn find_path_there(&self, actor: &Item, goal: (i32, i32), may_drop: bool, near: impl Fn(&Item) -> bool) -> Option<(Vec<(i32, i32)>, Vec<bool>)> {
        self.timed_search(actor, goal, may_drop, Ends::There, near)
    }

    /// As find_path, for a goal up at height `level` that is climbed to:
    /// if it cannot be reached, a way to the nearest point with nothing
    /// overhead up to that height (not under a bridge leading there).
    /// The way does not end lower than it starts (down the stairs to the
    /// street is no way up), with `may_drop` lower only where the climb up
    /// is not too high.
    pub fn find_path_to_climb(&self, actor: &Item, goal: (i32, i32), may_drop: bool, level: i32, near: impl Fn(&Item) -> bool) -> Option<(Vec<(i32, i32)>, Vec<bool>)> {
        let lowest = if may_drop { level - crate::anim::CLIMB_MAX } else { actor.z - 8 };
        self.timed_search(actor, goal, may_drop, Ends::OpenUpTo(level, lowest), near)
    }

    /// Is nothing over `actor`'s head at `p` up to height `level`? (Under
    /// a bridge or a roof there is no climbing up to it.)
    pub fn open_above(&self, actor: &Item, p: (i32, i32, i32), level: i32) -> bool {
        open_above_on(self.map(), &self.data.infos, actor, p, level)
    }

    fn timed_search(&self, actor: &Item, goal: (i32, i32), may_drop: bool, partial: Ends, near: impl Fn(&Item) -> bool) -> Option<(Vec<(i32, i32)>, Vec<bool>)> {
        // PAGAN_PATHTIME=1: how long each search takes
        if std::env::var_os("PAGAN_PATHTIME").is_none() {
            return self.search_path(actor, goal, may_drop, partial, near);
        }
        let start = std::time::Instant::now();
        let found = self.search_path(actor, goal, may_drop, partial, near);
        eprintln!("pathtime: actor {} shape {} from {},{},{} to {},{}: {:.1} ms, {}", actor.id, actor.shape, actor.x, actor.y, actor.z, goal.0, goal.1,
                  start.elapsed().as_secs_f64() * 1000.0, found.as_ref().map_or("none".to_string(), |p| format!("{} points", p.0.len())));
        found
    }

    fn search_path(&self, actor: &Item, goal: (i32, i32), may_drop: bool, partial: Ends, near: impl Fn(&Item) -> bool) -> Option<(Vec<(i32, i32)>, Vec<bool>)> {
        let most = if actor.id == crate::game::AVATAR { MAX_NODES } else { MAX_NODES_OTHERS };
        let flames = self.flames_to_avoid(actor);
        let (steps, _) = search_on(self.map(), &self.data.infos, actor, goal, may_drop, true, partial, near, most, 1, &flames)?;
        // only the corners (and the end): walking straight on in between;
        // a drop keeps both the edge and where one lands
        let mut path = Vec::new();
        let mut drops = Vec::new();
        let mut onward = 8;
        let mut edge = false;
        for s in steps.iter().rev() {
            if s.dir != onward || s.drop || edge {
                path.push((s.pos.0, s.pos.1));
                drops.push(s.drop);
            }
            edge = s.drop;
            onward = s.dir;
        }
        path.reverse();
        drops.reverse();
        Some((path, drops))
    }
}

/// A long way for the guide (guide.rs), searched apart from the game (on
/// a copy of the map): every point of it with its height, and whether it
/// gets there (otherwise it ends as near as it gets). `weight` > 1 makes
/// the search head for the goal more eagerly: a little longer way, found
/// much sooner. The way may lead down over an edge; it keeps off the
/// bottom of the world (the void below the dungeons, under their floors).
pub fn long_way(map: &MapData, infos: &[ShapeInfo], actor: &Item, goal: (i32, i32), near: impl Fn(&Item) -> bool, most: usize, weight: i32) -> Option<(Vec<(i32, i32, i32)>, bool)> {
    let (steps, reached) = search_on(map, infos, actor, goal, true, false, Ends::Nearest, near, most, weight, &[])?;
    Some((steps.iter().map(|s| s.pos).collect(), reached))
}

/// Is nothing over `actor`'s head at `p` up to height `level`?
fn open_above_on(map: &MapData, infos: &[ShapeInfo], actor: &Item, p: (i32, i32, i32), level: i32) -> bool {
    let Some(info) = infos.get(actor.shape as usize) else { return true };
    let (xd, yd, zd) = info.footpad(false);
    let body = WorldBox { x: p.0, y: p.1, z: p.2, xd, yd, zd };
    // a roof overhead up to that height (roofs can be flat, without
    // height: in the room below a roof the head touches it)
    if map.position_info(&body, &body, infos, actor.id).roof_z.is_some_and(|r| r <= level) {
        return false;
    }
    if level <= p.2 + zd {
        return true;
    }
    let above = WorldBox { x: p.0, y: p.1, z: p.2 + zd, xd, yd, zd: level - p.2 - zd };
    map.position_info(&above, &body, infos, actor.id).valid
}

/// One point of a way found: where, the direction of the step that led
/// there, and whether that step went down over an edge.
struct Step {
    pos: (i32, i32, i32),
    dir: u8,
    drop: bool,
}

/// The search itself (A* over a grid of STEP): the points from the start
/// (not included) to the end, and whether the end is the goal.
#[allow(clippy::too_many_arguments)]
fn search_on(map: &MapData, infos: &[ShapeInfo], actor: &Item, goal: (i32, i32), may_drop: bool, bottom: bool, partial: Ends, near: impl Fn(&Item) -> bool, most: usize, weight: i32, flames: &[WorldBox]) -> Option<(Vec<Step>, bool)> {
    let info = infos.get(actor.shape as usize).copied()?;
    let ground = Terrain { map, infos, actor, pad: info.footpad(false), may_drop, bottom, flames };
    // straight and diagonal steps cost 10 and 14; the estimate to the goal likewise
    let estimate = |p: (i32, i32, i32)| {
        let (dx, dy) = ((p.0 - goal.0).abs() / STEP, (p.1 - goal.1).abs() / STEP);
        weight * (10 * dx.max(dy) + 4 * dx.min(dy))
    };
    let at = |p: (i32, i32, i32)| Item { x: p.0, y: p.1, z: p.2, ..*actor };

    let start = (actor.x, actor.y, actor.z);
    let mut nodes = vec![Node { pos: start, cost: 0, parent: usize::MAX, dir: 8, drop: false }];
    // Places already found, by grid square: a square can hold several
    // (a bridge over a passage), each at its own height.
    let mut seen: HashMap<(i32, i32), Vec<usize>> = HashMap::from_iter([((0, 0), vec![0])]);
    // the one at about a height (a step up or down)
    let at_level = |seen: &HashMap<(i32, i32), Vec<usize>>, nodes: &[Node], key: (i32, i32), z: i32| {
        seen.get(&key).and_then(|v| v.iter().copied().find(|&k| (nodes[k].pos.2 - z).abs() <= 8))
    };
    // where a way that does not get there may end
    let end_ok = |p: (i32, i32, i32)| match partial {
        Ends::OpenUpTo(level, lowest) => p.2 >= lowest && p.2 <= level + 8 && open_above_on(map, infos, actor, p, level),
        _ => true,
    };
    let mut open = BinaryHeap::from([(Reverse(estimate(start)), 0usize, 0i32)]);
    let mut best = (estimate(start), 0usize);
    let mut reached = None;
    const STEPS: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

    while let Some((_, i, queued_cost)) = open.pop() {
        let (pos, cost, came) = (nodes[i].pos, nodes[i].cost, nodes[i].dir);
        if queued_cost != cost {
            continue; // a cheaper way here was found meanwhile
        }
        // (with OpenUpTo: not under the bridge, even if near enough)
        if near(&at(pos)) && end_ok(pos) {
            reached = Some(i);
            break;
        }
        if nodes.len() >= most {
            break;
        }
        for (d, &(dx, dy)) in STEPS.iter().enumerate() {
            let key = ((pos.0 - start.0) / STEP + dx, (pos.1 - start.1) / STEP + dy);
            let step_cost = if dx != 0 && dy != 0 { 14 } else { 10 } + if came != 8 && came as usize != d { TURN } else { 0 };
            // already known on this level: walkable, only the way there changes
            let (to, key, drop, extra) = if let Some(k) = at_level(&seen, &nodes, key, pos.2) {
                if nodes[k].cost <= cost + step_cost || k == 0 {
                    continue;
                }
                ((pos.0 + dx * STEP, pos.1 + dy * STEP, nodes[k].pos.2), key, false, 0)
            } else {
                // (may still be reached at another height from elsewhere)
                let Some(s) = ground.new_step(pos, key, (dx, dy), step_cost) else { continue };
                s
            };
            let new_cost = cost + step_cost + extra;
            let n = match at_level(&seen, &nodes, key, to.2) {
                Some(k) if nodes[k].cost <= new_cost || k == 0 => continue,
                Some(k) => {
                    nodes[k] = Node { pos: to, cost: new_cost, parent: i, dir: d as u8, drop };
                    k
                }
                None => {
                    nodes.push(Node { pos: to, cost: new_cost, parent: i, dir: d as u8, drop });
                    seen.entry(key).or_default().push(nodes.len() - 1);
                    nodes.len() - 1
                }
            };
            let h = estimate(to);
            if h < best.0 && end_ok(to) {
                best = (h, n);
            }
            open.push((Reverse(new_cost + h), n, new_cost));
        }
    }

    // if the goal is out of reach, at least get closer
    let end = match reached {
        Some(i) => i,
        None if partial != Ends::There && best.1 != 0 && best.0 + 20 * weight < estimate(start) => best.1,
        None => return None,
    };
    let mut steps = Vec::new();
    let mut i = end;
    while i != usize::MAX && i != 0 {
        steps.push(Step { pos: nodes[i].pos, dir: nodes[i].dir, drop: nodes[i].drop });
        i = nodes[i].parent;
    }
    steps.reverse();
    Some((steps, reached.is_some()))
}

/// What a way search moves through: the map, and the walker with its box.
struct Terrain<'a> {
    map: &'a MapData,
    infos: &'a [ShapeInfo],
    actor: &'a Item,
    pad: (i32, i32, i32),
    may_drop: bool,
    /// Does the bottom of the world (height 0) carry where nothing lies?
    /// It does in the game; the guide keeps off it (below the dungeons it
    /// is the black void, and its way led there under the floor).
    bottom: bool,
    /// Flames the walker keeps out of (option "avoid_flames").
    flames: &'a [WorldBox],
}

impl Terrain<'_> {
    fn boxed(&self, p: (i32, i32, i32)) -> WorldBox {
        let (xd, yd, zd) = self.pad;
        WorldBox { x: p.0, y: p.1, z: p.2, xd, yd, zd }
    }

    /// The walker at `p`, coming from `from`.
    fn info(&self, p: (i32, i32, i32), from: (i32, i32, i32)) -> PositionInfo {
        self.map.position_info(&self.boxed(p), &self.boxed(from), self.infos, self.actor.id)
    }

    /// A step from `pos` in direction `(dx, dy)` onto a grid square (`key`)
    /// not known at that height yet: where it ends, the square it ends on,
    /// whether it goes down over an edge, and what that costs on top of
    /// the step. None: no way on there.
    fn new_step(&self, pos: (i32, i32, i32), key: (i32, i32), (dx, dy): (i32, i32), step_cost: i32) -> Option<((i32, i32, i32), (i32, i32), bool, i32)> {
        let (nx, ny) = (pos.0 + dx * STEP, pos.1 + dy * STEP);
        if let Some(z) = self.step_height(pos, nx, ny) {
            return Some(((nx, ny, z), key, false, 0));
        }
        // down over the edge
        if let Some(z) = self.drop_to(pos, nx, ny) {
            return Some(((nx, ny, z), key, true, DROP_COST));
        }
        if !self.may_drop {
            return None;
        }
        // One step out the Avatar still hangs over the edge and cannot
        // fall yet: down from two steps out.
        let (fx, fy) = (nx + dx * STEP, ny + dy * STEP);
        let z = self.drop_to(pos, fx, fy)?;
        self.info((nx, ny, pos.2), pos).valid.then_some(((fx, fy, z), (key.0 + dx, key.1 + dy), true, step_cost + DROP_COST))
    }

    /// The height a step from `pos` to `(nx, ny)` ends at: on the same
    /// level, or up or down a step.
    fn step_height(&self, pos: (i32, i32, i32), nx: i32, ny: i32) -> Option<i32> {
        for k in 0..=16 {
            let lift = if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) };
            let z = pos.2 + lift;
            if z < 0 {
                continue;
            }
            let c = self.info((nx, ny, z), pos);
            // (standing right on a floor that is not solid, such as the
            // trim at a roof's edge, carries as when walking)
            let carried = (c.supported && (self.bottom || z > 0 || c.support.is_some())) || c.floor_z == Some(z);
            if c.valid && carried && !c.on_water && !c.touches_sea && c.floor_z.is_none_or(|f| z >= f) {
                let body = self.boxed((nx, ny, z));
                if self.flames.iter().any(|f| f.overlaps(&body)) {
                    return None;
                }
                return Some(z);
            }
            // A wall reaching from below the lowest height tried to above
            // the highest blocks them all: the other 16 checks (each a look
            // at everything around) would find the same.
            if k == 0 && !c.valid {
                let wall = c.blocker.and_then(|b| self.map.get(b)).and_then(|b| Some(WorldBox::of(b, self.infos.get(b.shape as usize)?)));
                if wall.is_some_and(|w| pos.2 + 8 < w.z + w.zd && pos.2 - 8 + self.pad.2 > w.z) {
                    return None;
                }
            }
        }
        None
    }

    /// Where one lands when stepping off the edge at `from` towards
    /// (nx, ny): falling straight down to the first ground, as long as the
    /// way down is free and the ground is safe.
    fn drop_to(&self, from: (i32, i32, i32), nx: i32, ny: i32) -> Option<i32> {
        if !self.may_drop {
            return None;
        }
        let out = self.info((nx, ny, from.2), from);
        if !out.valid || out.supported {
            return None;
        }
        let mut z = from.2 - 1;
        while z >= (from.2 - MAX_DROP).max(0) {
            let c = self.info((nx, ny, z), from);
            if !c.valid {
                return None;
            }
            if c.supported && !self.bottom && z <= 0 && c.support.is_none() {
                return None;
            }
            if c.supported || c.floor_z.is_some_and(|f| z <= f) {
                return (from.2 - z > 8 && !c.on_water && !c.touches_sea && !c.on_fire).then_some(z);
            }
            z -= 1;
        }
        None
    }
}

impl Game {

    // ---- game time ---------------------------------------------------------

    /// Seconds of game time: one per 30 ticks.
    pub fn game_seconds(&self) -> u32 {
        self.tick.wrapping_add(self.time_offset) / 30
    }

    /// The game hour counter (an hour has 900 game seconds).
    pub fn game_hours(&self) -> u32 {
        self.game_seconds() / 900
    }

    // ---- daily life -----------------------------------------------------------

    /// Once every game hour the "schedule" script of each inhabitant runs
    /// (one per tick): it sends them to work, to the tavern, to bed. That
    /// happens on every map, not only the Avatar's: in the original the
    /// scripts move people only while the Avatar is far away or elsewhere,
    /// so whoever left this map must be brought back from where they are.
    pub fn run_scheduler(&mut self) {
        const EVENT_SCHEDULE: u32 = 8;
        if !self.sched_after_scene.is_empty() && self.flag("inDaemonScene") == 0 {
            let minutes = self.game_seconds() / 60;
            for id in std::mem::take(&mut self.sched_after_scene) {
                if !self.state(id).dead {
                    self.call_event(id, EVENT_SCHEDULE, &minutes.to_le_bytes());
                }
            }
        }
        if self.sched_next != 0 {
            let id = self.sched_next;
            self.sched_next = if id >= 255 { 0 } else { id + 1 };
            // the Avatar has one too (it switches the weather effects)
            if self.actor_anywhere(id).is_none() || !self.people.npc_stats.contains_key(&id) {
                return;
            }
            if self.state(id).dead || self.state(id).in_combat {
                return;
            }
            let minutes = self.game_seconds() / 60;
            self.call_event(id, EVENT_SCHEDULE, &minutes.to_le_bytes());
            return;
        }
        let hour = self.game_hours();
        if hour > self.sched_last_hour {
            self.sched_last_hour = hour;
            self.sched_next = 1;
        }
    }

    /// An actor starts wandering about (activity "loiter").
    pub fn start_loitering(&mut self, actor: u16) {
        let already = self.scripts.procs.iter().any(|p| !p.terminated && matches!(p.kind, Kind::Loiter(l) if l.actor == actor));
        if !already {
            self.add_native(Kind::Loiter(Loiter::new(actor)), actor, crate::effects::PROC_LOITER);
        }
    }

    /// One tick of a wandering actor. Returns true when it stops for good.
    pub fn run_loiter(&mut self, l: &mut Loiter) -> bool {
        let Some(it) = self.map().get(l.actor).copied().filter(|it| it.in_world()) else { return true };
        let s = self.state(l.actor);
        if s.dead || s.in_combat {
            return true;
        }
        // not while walking somewhere or far from the player
        if self.inputs.contains_key(&l.actor) || !it.fast {
            return false;
        }
        if l.wait > 0 {
            l.wait -= 1;
            return false;
        }
        // stroll to a random spot nearby, then stand for 4 to 7 seconds
        let jitter = |g: &mut Game| 32 * ((g.rng.next() % 21) as i32 - 10);
        let to = (it.x + jitter(self), it.y + jitter(self));
        let pf = crate::effects::Pathfind::new(l.actor, 0, to);
        self.add_native(Kind::Pathfind(pf), l.actor, crate::effects::PROC_WALK);
        l.wait = 30 * (4 + (self.rng.next() % 4) as i32);
        false
    }
}
