//! The guide (option "map with guide"): the player picks a place on the
//! map (worldmap.rs) and glowing marks on the ground show the way there.
//!
//! The world is made of many maps joined by teleporters (eggs of family 8;
//! frame 1 marks where one arrives). A way to a place on another map goes
//! from teleporter to teleporter: the order is chosen by the distances in
//! a straight line, and the way to the next teleporter is then searched
//! for real. If it turns out there is none (the teleporter lies in another
//! part of the map, behind a wall), that teleporter is left out and the
//! next best chosen.
//!
//! The search is long (a whole town), so it runs on a copy of the map in
//! a thread of its own, without holding up the game. Doors that are not
//! locked count as open and people as not there; the way may lead down
//! over an edge (not up: climbing is left out); jumps and teleports done
//! by scripts (a ladder, a lever) are not known to it.

use crate::game::{Game, AVATAR};
use crate::hash::HashSet;
use crate::map::{Item, MapData};
use crate::shape::{ShapeInfo, FAMILY_TELEPORTEGG};
use crate::turnview::Xform;
use crate::view::Surface;
use std::sync::mpsc::{channel, Receiver};

/// Doors (two directions, two kinds): open for the search, unless locked
/// (this bit of their quality).
const DOORS: [u32; 4] = [68, 69, 340, 342];
const DOOR_LOCKED: u16 = 0x400;
/// Most points a search may look at, and how eagerly it heads for the goal.
const MOST_NODES: usize = 400_000;
const EAGER: i32 = 2;
/// Close enough to the place to be there (world units).
const ARRIVED: i32 = 96;
/// Further than this from the way, the Avatar has left it (world units),
/// and after this many ticks off it the way is searched again.
const OFF_WAY: i32 = 192;
const OFF_TICKS: u32 = 45;
/// The extra cost of going through a teleporter (world units), so that a
/// way on foot is preferred to a detour over another map.
const TELEPORT_COST: i32 = 600;
/// With only a way to near the place: how far the Avatar must have gone
/// on before it is searched again (world units). In dungeons stretches of
/// floor are laid out only when they come into view: from further on the
/// search may get further.
const SEARCH_AGAIN: i32 = 384;
/// A person followed is looked for this often (ticks); moved further than
/// this (world units), the way is searched again.
const PERSON_TICKS: u32 = 60;
const PERSON_MOVED: i32 = 256;
/// How many marks are shown ahead, and every how many points one.
const MARKS_AHEAD: usize = 36;
const MARK_EVERY: usize = 2;

/// A place to go to.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub map: usize,
    pub x: i32,
    pub y: i32,
    pub name: String,
    /// A person to go to (0: a place): the place moves with them.
    pub person: u16,
}

/// A teleporter: where it is, how far its range reaches, and where it leads.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub map: usize,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub xr: i32,
    pub yr: i32,
    pub to_map: usize,
    pub to: (i32, i32, i32),
}

/// What the way being shown leads to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Leg {
    /// The place itself (it is on this map).
    Place,
    /// The teleporter with this number (in `links`) on the way there.
    Teleporter(usize),
}

/// What the player is told about the way.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Status {
    Idle,
    Searching,
    Shown,
    /// Only a way to near the place was found.
    Near,
    /// No way known (none of the teleporters leads there).
    NoWay,
}

struct Job {
    rx: Receiver<Option<(Vec<(i32, i32, i32)>, bool)>>,
    leg: Leg,
    map: usize,
}

#[derive(Default)]
pub struct Guide {
    pub target: Option<Target>,
    /// The way on this map, point by point (32 units apart), with heights.
    pub route: Vec<(i32, i32, i32)>,
    /// How far along the way the Avatar is.
    pub progress: usize,
    pub leg: Option<Leg>,
    leg_map: usize,
    job: Option<Job>,
    /// The teleporters of all maps (made when first needed).
    links: Option<Vec<Link>>,
    /// Teleporters that turned out not to be reachable on the way.
    blocked: HashSet<usize>,
    /// The way that got nearest to the first of them: shown if no other
    /// leads there (a gate a guard opens, a wall a lever moves).
    fallback: Option<(usize, Vec<(i32, i32, i32)>)>,
    off_ticks: u32,
    status: Option<Status>,
    /// Where the Avatar was when the way shown was searched.
    searched_from: (i32, i32),
    /// Wait for each search in the same tick (the playthrough test: the
    /// same steps then always give the same run).
    pub sync: bool,
}

impl Guide {
    /// A teleporter (by its number in the list of all).
    pub fn link(&self, i: usize) -> Option<Link> {
        self.links.as_ref()?.get(i).copied()
    }

    pub fn status(&self) -> Status {
        if self.target.is_none() {
            Status::Idle
        } else if self.job.is_some() {
            Status::Searching
        } else {
            self.status.unwrap_or(Status::Searching)
        }
    }
}

/// Straight-line distance over the ground (diagonal steps a little longer).
fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    let (dx, dy) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
    dx.max(dy) + dx.min(dy) * 2 / 5
}

/// Is the Avatar standing at `p` in the range of a teleporter? (As the
/// eggs check it; `pad`: the Avatar's footprint.)
fn in_range(l: &Link, p: (i32, i32, i32), pad: (i32, i32)) -> bool {
    l.x - l.xr <= p.0 && p.0 - pad.0 < l.x + l.xr && l.y - l.yr <= p.1 && p.1 - pad.1 < l.y + l.yr && l.z - 48 < p.2 && p.2 <= l.z + 48
}

impl Game {
    /// The teleporters of all maps, from the loaded maps as they are now and
    /// from the game data for the others.
    fn make_links(&self) -> Vec<Link> {
        let teleport_egg = |it: &Item| self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_TELEPORTEGG);
        let mut eggs: Vec<(usize, Item)> = Vec::new();
        for n in 0..self.fixed.len().max(self.nonfixed.len()) {
            if let Some(map) = self.maps.get(&n) {
                eggs.extend(map.items.iter().filter(|it| it.in_world() && teleport_egg(it)).map(|it| (n, *it)));
                continue;
            }
            for d in [self.fixed.get(n), self.nonfixed.get(n)].into_iter().flatten() {
                let (items, parents) = crate::map::parse_items(d, &self.data.infos);
                eggs.extend(items.into_iter().zip(parents).filter(|(it, p)| p.is_none() && teleport_egg(it)).map(|(it, _)| (n, it)));
            }
        }
        let arrival = |map: usize, n: u16| eggs.iter().find(|(m, it)| *m == map && it.frame == 1 && it.quality & 0xFF == n).map(|(_, it)| (it.x, it.y, it.z));
        eggs.iter()
            .filter(|(_, it)| it.frame != 1)
            .filter_map(|&(map, it)| {
                let to_map = it.mapnum as usize;
                let to = arrival(to_map, it.quality & 0xFF)?;
                let (xr, yr) = (32 * ((it.npc >> 4) & 0xF) as i32, 32 * (it.npc & 0xF) as i32);
                Some(Link { map, x: it.x, y: it.y, z: it.z, xr, yr, to_map, to })
            })
            .collect()
    }

    /// Where a person is now: map and position (None: dead or nowhere).
    pub fn person_at(&self, id: u16) -> Option<(usize, i32, i32)> {
        for (&m, map) in &self.maps {
            if let Some(it) = map.get(id) {
                return (it.in_world() && !it.dead).then_some((m, it.x, it.y));
            }
        }
        self.people.npcs.iter().find(|n| n.item.id == id && !n.item.dead).map(|n| (n.map, n.item.x, n.item.y))
    }

    pub fn guide_links(&mut self) -> &[Link] {
        if self.guide.links.is_none() {
            self.guide.links = Some(self.make_links());
        }
        self.guide.links.as_deref().unwrap_or(&[])
    }

    /// Sets the place to go to (None: no more guiding).
    pub fn set_guide_target(&mut self, target: Option<Target>) {
        let g = &mut self.guide;
        g.target = target;
        g.route.clear();
        g.progress = 0;
        g.leg = None;
        g.job = None;
        g.blocked.clear();
        g.fallback = None;
        g.status = None;
        g.off_ticks = 0;
    }

    /// Which way to take first: to the place if it is on this map and
    /// nearer than any detour, otherwise to the first teleporter on the
    /// shortest way over the maps (a search over the teleporters, the
    /// distances in a straight line).
    fn choose_leg(&mut self) -> Option<Leg> {
        let target = self.guide.target.clone()?;
        let av = self.avatar();
        let cur = self.cur_map;
        self.guide_links();
        let links = self.guide.links.as_deref().unwrap_or(&[]);
        let blocked = &self.guide.blocked;
        // cost to reach each teleporter's far side, and the first teleporter used
        let n = links.len();
        let mut cost = vec![i32::MAX; n];
        let mut first = vec![usize::MAX; n];
        let mut done = vec![false; n];
        for (i, l) in links.iter().enumerate() {
            if l.map == cur && !blocked.contains(&i) {
                cost[i] = distance((av.x, av.y), (l.x, l.y)) + TELEPORT_COST;
                first[i] = i;
            }
        }
        let mut best = if target.map == cur { Some((distance((av.x, av.y), (target.x, target.y)), Leg::Place)) } else { None };
        while let Some(i) = (0..n).filter(|&i| !done[i] && cost[i] < i32::MAX).min_by_key(|&i| cost[i]) {
            if best.is_some_and(|(c, _)| c <= cost[i]) {
                break;
            }
            done[i] = true;
            let here = links[i];
            if here.to_map == target.map {
                let c = cost[i] + distance((here.to.0, here.to.1), (target.x, target.y));
                if best.is_none_or(|(b, _)| c < b) {
                    best = Some((c, Leg::Teleporter(first[i])));
                }
            }
            for (j, l) in links.iter().enumerate() {
                // (the teleporter right where one arrives would lead straight back)
                if done[j] || l.map != here.to_map || (l.to_map == here.map && distance((l.x, l.y), (here.to.0, here.to.1)) < 256) {
                    continue;
                }
                let c = cost[i] + distance((here.to.0, here.to.1), (l.x, l.y)) + TELEPORT_COST;
                if c < cost[j] {
                    cost[j] = c;
                    first[j] = first[i];
                }
            }
        }
        best.map(|(_, leg)| leg)
    }

    /// Starts the search for the way of a leg, in its own thread.
    fn start_search(&mut self, leg: Leg) {
        let av = self.avatar();
        let pad = self.data.infos.get(av.shape as usize).map_or((64, 64), |si| (si.x * 32, si.y * 32));
        let (goal, link) = match leg {
            Leg::Place => {
                let Some(t) = &self.guide.target else { return };
                ((t.x, t.y), None)
            }
            Leg::Teleporter(i) => {
                let Some(l) = self.guide.links.as_ref().and_then(|ls| ls.get(i)).copied() else { return };
                ((l.x, l.y), Some(l))
            }
        };
        // a copy of the map: doors open, people out of the way
        // and without the all-black ground below the dungeons: one can stand
        // on it, but it is the void outside the world, not a passage (the
        // way to a place on the earth plane led through it, under the floor)
        let mut map: MapData = self.map().clone();
        let mut black: crate::hash::HashMap<(u32, u32), bool> = Default::default();
        for it in &mut map.items {
            if (DOORS.contains(&it.shape) && it.quality & DOOR_LOCKED == 0) || (it.is_actor() && it.id != AVATAR) {
                it.dead = true;
                continue;
            }
            if !self.data.infos.get(it.shape as usize).is_some_and(|i| i.land) {
                continue;
            }
            let (shapes, pal) = (&mut self.data.shapes, &self.data.pal);
            if *black.entry((it.shape, it.frame)).or_insert_with(|| crate::minimap::all_black(shapes, pal, it.shape, it.frame)) {
                it.dead = true;
            }
        }
        let infos = self.data.infos.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let near = |it: &Item| match &link {
                Some(l) => in_range(l, (it.x, it.y, it.z), pad),
                None => (it.x - goal.0).abs() <= ARRIVED / 2 && (it.y - goal.1).abs() <= ARRIVED / 2,
            };
            let found = crate::path::long_way(&map, &infos, &av, goal, near, MOST_NODES, EAGER);
            let _ = tx.send(found);
        });
        self.guide.job = Some(Job { rx, leg, map: self.cur_map });
        self.guide.searched_from = (av.x, av.y);
        self.guide.leg = Some(leg);
        self.guide.leg_map = self.cur_map;
    }

    /// Takes the result of a search that has ended. Returns whether one came.
    pub fn poll_guide(&mut self) -> bool {
        let Some(job) = &self.guide.job else { return false };
        let found = if self.guide.sync { job.rx.recv().ok() } else { job.rx.try_recv().ok() };
        let Some(found) = found else { return false };
        let (leg, map) = (job.leg, job.map);
        self.guide.job = None;
        if map != self.cur_map {
            return true; // made for a map the Avatar has left
        }
        match found {
            Some((route, true)) => {
                self.guide.route = route;
                self.guide.status = Some(Status::Shown);
            }
            other if matches!(leg, Leg::Teleporter(_)) => {
                // that teleporter cannot be reached: the next best one
                if let Leg::Teleporter(i) = leg {
                    self.guide.blocked.insert(i);
                    if let (None, Some((route, _))) = (&self.guide.fallback, other) {
                        if !route.is_empty() {
                            self.guide.fallback = Some((i, route));
                        }
                    }
                }
                self.guide.route.clear();
                self.guide.leg = None;
            }
            Some((route, false)) => {
                self.guide.route = route;
                self.guide.status = Some(Status::Near);
            }
            None => {
                self.guide.route.clear();
                self.guide.status = Some(Status::NoWay);
            }
        }
        // the way starts where the Avatar was when the search began
        self.guide.progress = 0;
        self.guide.off_ticks = 0;
        true
    }

    /// Once a tick: arriving, following the way, searching it again when
    /// the Avatar has left it or changed maps.
    pub fn tick_guide(&mut self) {
        if !self.options.guide {
            return;
        }
        let Some(mut target) = self.guide.target.clone() else { return };
        // a person goes about their day: the way follows them
        if target.person != 0 && self.tick.is_multiple_of(PERSON_TICKS) {
            if let Some((map, x, y)) = self.person_at(target.person) {
                if map != target.map || (x - target.x).abs().max((y - target.y).abs()) > PERSON_MOVED {
                    target = Target { map, x, y, ..target };
                    self.set_guide_target(Some(target.clone()));
                }
            }
        }
        let av = self.avatar();
        if target.map == self.cur_map && (av.x - target.x).abs() <= ARRIVED && (av.y - target.y).abs() <= ARRIVED {
            let text = self.tr([
                format!("Ziel erreicht: {}", target.name),
                format!("Destination atteinte : {}", target.name),
                format!("Arrived: {}", target.name),
            ]);
            self.notify(&crate::journal::dos(&text));
            self.set_guide_target(None);
            return;
        }
        // another map: what was found here no longer counts (the
        // teleporters left out may be reachable from the new side)
        if self.guide.leg.is_some() && self.guide.leg_map != self.cur_map {
            let t = self.guide.target.take();
            self.set_guide_target(t);
        }
        self.poll_guide();
        if self.guide.job.is_some() {
            return;
        }
        if self.guide.leg.is_none() {
            // (with no way known, looking again once a second is enough)
            if self.guide.status == Some(Status::NoWay) && !self.tick.is_multiple_of(30) {
                return;
            }
            match self.choose_leg() {
                Some(leg) => self.start_search(leg),
                // no teleporter can be reached: as near as it got to the best one
                None if self.guide.fallback.is_some() => {
                    if let Some((i, route)) = self.guide.fallback.take() {
                        self.guide.route = route;
                        self.guide.progress = 0;
                        self.guide.leg = Some(Leg::Teleporter(i));
                        self.guide.leg_map = self.cur_map;
                        self.guide.status = Some(Status::Near);
                        self.guide.blocked.remove(&i);
                    }
                }
                None => {
                    if self.guide.status != Some(Status::NoWay) {
                        let text = self.tr(["Kein Weg dorthin bekannt", "Aucun chemin connu", "No way there known"]);
                        self.notify(text);
                    }
                    self.guide.status = Some(Status::NoWay);
                }
            }
            return;
        }
        // only near there: look again once the Avatar has gone on a stretch
        if self.guide.status == Some(Status::Near) && distance((av.x, av.y), self.guide.searched_from) > SEARCH_AGAIN {
            if let Some(leg) = self.guide.leg {
                self.start_search(leg);
                return;
            }
        }
        // following the way: the nearest point a little ahead
        let route = &self.guide.route;
        if route.is_empty() {
            return;
        }
        let from = self.guide.progress.min(route.len() - 1);
        let to = (from + 48).min(route.len());
        let (best, d) = (from..to)
            .map(|i| (i, distance((av.x, av.y), (route[i].0, route[i].1)) + (av.z - route[i].2).abs()))
            .min_by_key(|&(_, d)| d)
            .unwrap_or((from, 0));
        self.guide.progress = best;
        if d > OFF_WAY {
            self.guide.off_ticks += 1;
            if self.guide.off_ticks > OFF_TICKS {
                // off the way: look again from here
                if let Some(leg) = self.guide.leg {
                    self.guide.route.clear();
                    self.guide.off_ticks = 0;
                    self.start_search(leg);
                }
            }
        } else {
            self.guide.off_ticks = 0;
        }
    }

}

/// The marks on the ground ahead of the Avatar. Drawn after the world,
/// before the words: a mark hidden behind something taller (a wall, a
/// roof) is left out where the picture shows that (pixel ids, `ids`; not
/// kept for a turned view).
pub fn draw_marks(surf: &mut Surface, guide: &Guide, map: &MapData, infos: &[ShapeInfo], ids: &[u32], xf: &Xform, tick: u32) {
        if guide.target.is_none() || guide.route.is_empty() {
            return;
        }
        let route = &guide.route;
        let start = guide.progress + 1;
        let (w, h) = (surf.width, surf.height);
        let use_ids = xf.angle == 0.0 && ids.len() == (w * h) as usize;
        let phase = (tick / 3) as usize;
        let mut k = 0;
        for i in (start..route.len()).step_by(MARK_EVERY).take(MARKS_AHEAD) {
            let (x, y, z) = route[i];
            // the middle of the footprint: the way is that of the Avatar's corner
            let (sx, sy) = xf.to_screen(x - 32, y - 32, z);
            k += 1;
            if sx < -4 || sy < -4 || sx >= w + 4 || sy >= h + 4 {
                continue;
            }
            // a wave of light runs along the marks towards the goal; they fade far ahead
            let wave = (k + 64 - phase % 64) % 8;
            let bright = if wave == 0 { 1.0 } else if wave == 1 { 0.85 } else { 0.7 };
            let fade = 1.0 - (k as f32 / MARKS_AHEAD as f32) * 0.4;
            let a = bright * fade;
            for dy in -3..=3i32 {
                for dx in -6..=6i32 {
                    // a flat diamond, as a spot lying on the ground
                    let r = dx.abs() as f32 / 6.5 + dy.abs() as f32 / 3.5;
                    if r > 1.0 {
                        continue;
                    }
                    let (px, py) = (sx + dx, sy + dy);
                    if px < 0 || py < 0 || px >= w || py >= h {
                        continue;
                    }
                    let at = (py * w + px) as usize;
                    // something in front that reaches above the ground there
                    let in_front = |it: &Item| it.z + infos.get(it.shape as usize).map_or(0, |si| si.z * 8) > z + 8 && it.id != AVATAR;
                    if use_ids && ids[at].checked_sub(1).and_then(|i| map.items.get(i as usize)).is_some_and(in_front) {
                        continue;
                    }
                    let glow = a * (1.0 - r * r * 0.7);
                    let c = surf.pixels[at];
                    let mix = |v: u32, t: u32| ((v as f32) * (1.0 - glow) + t as f32 * glow).round().min(255.0) as u32;
                    surf.pixels[at] = mix(c >> 16 & 0xFF, 0xFF) << 16 | mix(c >> 8 & 0xFF, 0xE0) << 8 | mix(c & 0xFF, 0x60);
                }
            }
        }
    }
