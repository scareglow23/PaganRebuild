//! Passers-by in Tenebrae (option "passers-by"; the original has none).
//!
//! In the daytime a few ordinary townsfolk walk the streets of the three
//! maps of Tenebrae. Each one appears just beyond the edge of the view, so
//! that nobody pops up in sight, walks through the town to some other
//! place out of doors, waits there a little and goes on. They are made like
//! the creatures of a monster egg: when they leave the active area around
//! the view, the engine takes them away again. They are the game's own
//! peasants, with their scripts: spoken to, they answer as peasants do.

use crate::game::Game;
use crate::map::{Item, WorldBox};
use crate::vm::Kind;

/// The maps of Tenebrae: west, centre, east.
const TOWN_MAPS: [usize; 3] = [39, 40, 41];
/// The townsfolk that can walk: three peasants to one child (the
/// other peasants of the game, 835 and 836, only sit or lean).
const SHAPES: [u32; 4] = [707, 707, 707, 708];
/// At most this many at once.
const MAX: usize = 4;
/// How often (ticks) the town is looked after.
const EVERY: u32 = 45;
/// From this hour of the day to that one they are about.
const DAY: std::ops::Range<u32> = 6..21;
/// The streets, squares and bridges of Tenebrae lie at heights 0 to 64;
/// higher up are only roofs, walls and the terraces of the castle.
const STREET_TOP: i32 = 64;

/// One passer-by: who, and from when on it may set off again.
#[derive(Clone, Copy)]
pub struct Passer {
    id: u16,
    rest_until: u32,
}

impl Game {
    /// Once per tick.
    pub fn tick_passersby(&mut self) {
        if !self.tick.is_multiple_of(EVERY) {
            return;
        }
        // the ones still about (the others have gone with the active area)
        let map = self.cur_map;
        let alive: Vec<Passer> = std::mem::take(&mut self.passersby)
            .into_iter()
            .filter(|p| self.map().get(p.id).is_some_and(|it| it.in_world() && it.shape_is_one_of(&SHAPES)) && !self.state(p.id).dead)
            .collect();
        self.passersby = alive;
        // in a cellar (a room lying in the void below the town) there are
        // none: they walked about in the black around it
        let av = self.avatar();
        let outside = crate::shadows::void_directions(self.map(), av.x, av.y) < crate::shadows::VOID_ROOM;
        if !outside && TOWN_MAPS.contains(&map) {
            // (older savegames did not keep the list: townsfolk of their
            // shapes walking on the black plates of the void are such)
            let lost: Vec<u16> = self.map().items.iter()
                .filter(|it| it.is_actor() && it.in_world() && it.shape_is_one_of(&SHAPES) && self.on_void(it))
                .map(|it| it.id)
                .collect();
            for id in lost {
                self.remove_passer(id);
            }
        }
        if !self.options.passersby || !outside {
            // switched off: they go at once
            for p in std::mem::take(&mut self.passersby) {
                self.remove_passer(p.id);
            }
            return;
        }
        if !TOWN_MAPS.contains(&map) {
            return;
        }
        let hour = self.game_hours() % 24;
        let busy = !self.player_in_control() || self.fighting() || self.ui.title.is_some();
        if DAY.contains(&hour) && !busy && self.passersby.len() < MAX && self.tick.is_multiple_of(2 * EVERY) {
            self.add_passer();
        }
        // those standing about set off again after a while
        for i in 0..self.passersby.len() {
            let p = self.passersby[i];
            if self.tick >= p.rest_until && !self.walking_somewhere(p.id) {
                let rest = 90 + self.rng.next() % 300;
                self.passersby[i].rest_until = self.tick + rest;
                self.send_somewhere(p.id);
            }
        }
    }

    /// A walk under way for this actor (from a script or from here).
    fn walking_somewhere(&self, id: u16) -> bool {
        self.scripts.procs.iter().any(|p| !p.terminated && matches!(p.kind, Kind::Pathfind(pf) if pf.actor == id)) || self.inputs.contains_key(&id)
    }

    /// A free place out of doors on firm ground at street level, near
    /// (x, y) and the height `z`, where a person of `shape` fits.
    fn outdoor_spot(&self, shape: u32, x: i32, y: i32, z: i32) -> Option<(i32, i32, i32)> {
        let si = self.data.infos.get(shape as usize)?;
        let (xd, yd, zd) = si.footpad(false);
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        for dz in [0, 8, -8, 16, -16] {
            // a flat roof has nothing above it either: keep to the streets
            if z + dz > STREET_TOP {
                continue;
            }
            let b = WorldBox { x, y, z: z + dz, xd, yd, zd };
            let info = self.map().position_info(&b, &nowhere, &self.data.infos, 0);
            // standing on something, nothing above (a roof means a house)
            if info.valid && info.supported && info.roof_z.is_none() {
                // not on water or on something burning
                let safe = info.support.and_then(|s| self.map().get(s)).is_some_and(|s| self.usecode_safe_ground(s));
                if safe {
                    return Some((x, y, z + dz));
                }
            }
        }
        None
    }

    /// Ground with no script of its own (water drowns, lava burns ...).
    fn usecode_safe_ground(&self, ground: &Item) -> bool {
        const EVENT_GOT_HIT: u32 = 6;
        self.scripts.usecode.event(ground.shape, EVENT_GOT_HIT).is_none()
    }

    /// A new passer-by just beyond the edge of the view.
    fn add_passer(&mut self) {
        let av = self.avatar();
        let shape = SHAPES[(self.rng.next() % SHAPES.len() as u32) as usize];
        let (vw, vh) = (self.view.size.0 / 2, self.view.size.1 / 2);
        for _ in 0..12 {
            // a point on the screen around the view, then in the world
            let side = self.rng.next() % 4;
            let along = |g: &mut Game, half: i32| (g.rng.next() % (2 * half as u32 + 1)) as i32 - half;
            let (sx, sy) = match side {
                0 => (-(vw + 40 + (self.rng.next() % 120) as i32), along(self, vh)),
                1 => (vw + 40 + (self.rng.next() % 120) as i32, along(self, vh)),
                2 => (along(self, vw), -(vh + 40 + (self.rng.next() % 100) as i32)),
                _ => (along(self, vw), vh + 40 + (self.rng.next() % 100) as i32),
            };
            // screen x = (dx - dy) / 4, screen y = (dx + dy) / 8
            let (dx, dy) = ((4 * sx + 8 * sy) / 2, (8 * sy - 4 * sx) / 2);
            let Some((x, y, z)) = self.outdoor_spot(shape, av.x + dx, av.y + dy, av.z) else { continue };
            // an activity of its own: none (it is sent on its way from here)
            let egg = Item { x, y, z, quality: shape as u16, mapnum: 2, ..Item::default() };
            let id = self.create_monster(&egg, true);
            if id != 0 {
                self.passersby.push(Passer { id, rest_until: 0 });
            }
            return;
        }
    }

    /// Off to some other place out of doors, a good way off.
    fn send_somewhere(&mut self, id: u16) {
        let Some(it) = self.map().get(id).copied() else { return };
        for _ in 0..12 {
            let angle = (self.rng.next() % 360) as f32 * std::f32::consts::PI / 180.0;
            let dist = 300.0 + (self.rng.next() % 900) as f32;
            let (x, y) = (it.x + (angle.cos() * dist) as i32, it.y + (angle.sin() * dist) as i32);
            if self.outdoor_spot(it.shape, x, y, it.z).is_some() {
                let pf = crate::effects::Pathfind::new(id, 0, (x, y));
                self.add_native(Kind::Pathfind(pf), id, crate::effects::PROC_WALK);
                return;
            }
        }
    }

    /// A passer-by goes away (as creatures do when they leave the active area).
    fn remove_passer(&mut self, id: u16) {
        self.kill_processes(id, crate::vm::PROC_TYPE_ALL);
        self.actors.remove(&id);
        self.states.remove(&id);
        if let Some(idx) = self.map().index(id) {
            self.map_mut().destroy(idx);
        }
    }

    /// The passers-by about now (for tests and the savegame).
    pub fn passersby_ids(&self) -> Vec<u16> {
        self.passersby.iter().map(|p| p.id).collect()
    }

    /// The passers-by of a savegame.
    pub fn set_passersby(&mut self, ids: &[u16]) {
        self.passersby = ids.iter().map(|&id| Passer { id, rest_until: 0 }).collect();
    }

    /// Does a figure stand on the black plates of the void?
    /// A creature of `shape` can stand at (x, y, z) on firm ground: not in
    /// the air and not on the black plates of the void.
    pub fn firm_ground(&self, shape: u32, x: i32, y: i32, z: i32) -> bool {
        let Some(si) = self.data.infos.get(shape as usize) else { return false };
        let (xd, yd, zd) = si.footpad(false);
        let b = WorldBox { x, y, z, xd, yd, zd };
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        let pi = self.map().position_info(&b, &nowhere, &self.data.infos, 0);
        // (any plate under its footprint: the one reported is only one of them)
        let on_void = self.map().items_near(x, y, 512).into_iter().map(|i| &self.map().items[i]).any(|g| {
            g.shape == 564 && g.frame == 2 && g.in_world() && g.z == z && {
                let gb = WorldBox::of(g, &self.data.infos[564]);
                gb.x - gb.xd < x && x - xd < gb.x && gb.y - gb.yd < y && y - yd < gb.y
            }
        });
        pi.valid && pi.supported && !on_void
    }

    fn on_void(&self, it: &Item) -> bool {
        let Some(si) = self.data.infos.get(it.shape as usize) else { return false };
        let b = WorldBox::of(it, si);
        let pi = self.map().position_info(&b, &b, &self.data.infos, it.id);
        pi.support.and_then(|s| self.map().get(s)).is_some_and(|g| g.shape == 564 && g.frame == 2)
    }
}

impl Item {
    fn shape_is_one_of(&self, shapes: &[u32]) -> bool {
        shapes.contains(&self.shape)
    }
}
