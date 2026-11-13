//! The "fast area" around the Avatar and the eggs in it.
//!
//! Only items near the view are active: when they come into range their
//! `enterFastArea` script runs, when they drop out `leaveFastArea`.  Eggs
//! are invisible triggers: usecode eggs start a script when the Avatar
//! steps into their range, teleport eggs move him to another place or map,
//! monster eggs create creatures (through their own script).

use crate::actor::Mover;
use crate::anim;
use crate::game::{Game, AVATAR};
use crate::map::{Item, IN_WORLD};
use crate::shape::{FAMILY_TELEPORTEGG, FAMILY_UNKEGG};
use crate::effects::Fade;

/// Ticks (30 a second) of the fade to black at a gate to another map, and
/// of the fade in on the other side.
const GATE_FADE_OUT: i32 = 9;
const GATE_FADE_IN: i32 = 12;
/// Ticks a creature from a monster egg in sight takes to fade in.
const FADE_IN_TICKS: u32 = 24;

pub const FLG_DISPOSABLE: u16 = 0x0002;
pub const FLG_IN_NPC_LIST: u16 = 0x0040;
pub const FLG_FAST_ONLY: u16 = 0x0080;
/// Processes of this type go on when the Avatar changes maps.
const KEEP_ON_MAP_CHANGE: u16 = 1;
/// Numbers below are actors that keep theirs on every map.
const FIRST_MAP_ID: u16 = 256;

const EVENT_HATCH: u32 = 7;
const EVENT_ENTER_FAST_AREA: u32 = 15;
const EVENT_LEAVE_FAST_AREA: u32 = 16;

/// Where a teleport ends.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Destination {
    Position(i32, i32, i32),
    /// The destination teleport egg with this number.
    Egg(u16),
}

impl Game {
    /// Is a world position close enough to the view to be active?
    /// `centre`: where the view is. As in the original, the map's chunks
    /// (512 units square) come and go as a whole, decided by their middle:
    /// an egg that lays out a dungeon's furnishings (things that live only
    /// in the fast area) and what it laid out stay or go together, so that
    /// no floor goes missing while its egg is still active (holes).
    fn in_fast_area(&self, centre: (i32, i32), x: i32, y: i32) -> bool {
        const CHUNK: i32 = 512;
        let (x, y) = (x.div_euclid(CHUNK) * CHUNK + CHUNK / 2, y.div_euclid(CHUNK) * CHUNK + CHUNK / 2);
        let (mut dx, mut dy) = (x - centre.0, y - centre.1);
        // with the view turned, what is in view is turned as well
        for _ in 0..self.view.xf.quarter() {
            (dx, dy) = (-dy, dx);
        }
        ((dx - dy) / 4).abs() <= self.view.size.0 / 2 + 256 && ((dx + dy) / 8).abs() <= self.view.size.1 / 2 + 256
    }

    /// Finds the items that came into or dropped out of the fast area and
    /// runs their scripts.
    /// As in the original, the area goes with the view, not with the
    /// Avatar: scripts that put him out of the way for a moment (Flash
    /// parks him at 0,0,0) do not empty the surroundings.
    pub fn update_fast_area(&mut self) {
        let (cx, cy, _) = self.camera_position();
        let av = (cx, cy);
        let mut keep = Vec::new();
        let mut left = Vec::new();
        for idx in std::mem::take(&mut self.fast_items) {
            let it = self.map().items[idx];
            if it.gone {
                continue;
            }
            if it.parent != IN_WORLD || self.in_fast_area(av, it.x, it.y) {
                keep.push(idx);
            } else {
                left.push(idx);
            }
        }

        // (and what is in a chunk whose middle is that near)
        let reach = 2 * (self.view.size.0 / 2 + 256) + 4 * (self.view.size.1 / 2 + 256) + 512;
        let mut entered = Vec::new();
        for idx in self.map().items_near(av.0, av.1, reach) {
            let it = &self.map().items[idx];
            if !it.fast && self.in_fast_area(av, it.x, it.y) {
                entered.push(idx);
            }
        }
        for &idx in &entered {
            self.map_mut().items[idx].fast = true;
        }
        keep.extend_from_slice(&entered);
        self.fast_items = keep;

        for idx in left {
            self.leave_fast_area(idx);
        }
        for idx in entered {
            let id = self.map().items[idx].id;
            if id != 0 && id != AVATAR {
                self.call_event(id, EVENT_ENTER_FAST_AREA, &[]);
            }
        }
    }

    /// Something a script put into the view (a person teleported here):
    /// it enters the area at once, its script runs before the next steps
    /// of the one that brought it (as in the original, where moving an
    /// item there tells it right away; Malchir turning to his default
    /// direction after the scene had turned him to Bane).
    pub fn enter_fast_area_now(&mut self, id: u16) {
        let Some(idx) = self.map().index(id) else { return };
        let it = self.map().items[idx];
        let (cx, cy, _) = self.camera_position();
        if it.fast || it.parent != IN_WORLD || !self.in_fast_area((cx, cy), it.x, it.y) {
            return;
        }
        self.map_mut().items[idx].fast = true;
        self.fast_items.push(idx);
        if id != 0 && id != AVATAR {
            self.call_event(id, EVENT_ENTER_FAST_AREA, &[]);
        }
    }

    fn leave_fast_area(&mut self, idx: usize) {
        let it = self.map().items[idx];
        {
            let item = &mut self.map_mut().items[idx];
            item.fast = false;
            item.hatched = false;
        }
        if it.id == AVATAR {
            return;
        }
        if it.flags & FLG_FAST_ONLY != 0 {
            // temporary things (hatched monsters ...) do not persist
            if it.parent == IN_WORLD {
                self.kill_processes(it.id, crate::vm::PROC_TYPE_ALL);
                self.actors.remove(&it.id);
                self.states.remove(&it.id);
                self.map_mut().destroy(idx);
            }
        } else if it.id != 0 {
            self.call_event(it.id, EVENT_LEAVE_FAST_AREA, &[]);
        }
    }

    /// Hatches the eggs whose range the Avatar is in.
    pub fn hatch_eggs(&mut self) {
        let av = self.avatar();
        let (axd, ayd) = self.data.infos.get(av.shape as usize).map_or((64, 64), |si| (si.x * 32, si.y * 32));
        let mut near_teleporter = false;
        for i in 0..self.fast_items.len() {
            let idx = self.fast_items[i];
            let egg = self.map().items[idx];
            let family = self.data.infos.get(egg.shape as usize).map_or(0, |si| si.family);
            if !egg.in_world() || (family != FAMILY_UNKEGG && family != FAMILY_TELEPORTEGG) {
                continue;
            }
            // the range is stored in the NPC number field, in tiles
            let xr = 32 * ((egg.npc >> 4) & 0xF) as i32;
            let yr = 32 * (egg.npc & 0xF) as i32;
            let inside = egg.x - xr <= av.x && av.x - axd < egg.x + xr
                && egg.y - yr <= av.y && av.y - ayd < egg.y + yr
                && egg.z - 48 < av.z && av.z <= egg.z + 48;
            if !inside {
                // out of its range it is ready again: walking back through
                // the row of skeletons sets them swinging their axes again
                if egg.hatched {
                    self.map_mut().items[idx].hatched = false;
                }
                continue;
            }
            if family == FAMILY_TELEPORTEGG {
                // frame 1 marks a destination, everything else teleports
                if egg.frame != 1 {
                    near_teleporter = true;
                    if !self.just_teleported && self.pending_teleport.is_none() && self.gate_teleport.is_none() {
                        self.gate_teleport_to((egg.mapnum as usize, Destination::Egg(egg.quality & 0xFF)));
                    }
                }
            } else if !egg.hatched {
                self.map_mut().items[idx].hatched = true;
                self.call_event(egg.id, EVENT_HATCH, &[]);
            }
        }
        // arriving next to a teleporter must not send the Avatar straight back
        if !near_teleporter {
            self.just_teleported = false;
        }
    }

    /// A teleporter sends the Avatar on: with the option "Fade when
    /// teleporting" the view fades out first (gate_fade_done), to another
    /// map or to another place on this one, as the original does with its
    /// teleporters.
    pub fn gate_teleport_to(&mut self, target: (usize, Destination)) {
        if self.options.map_fade && self.fade.is_none() {
            self.gate_teleport = Some(target);
            self.fade = Some(Fade::new((0, 0, 0), 0, 256, GATE_FADE_OUT));
        } else {
            self.pending_teleport = Some(target);
        }
    }

    /// A gate's fade to black is over: the teleport goes ahead, and the
    /// view fades in again on the other side (option "Fade when teleporting").
    pub fn gate_fade_done(&mut self) {
        let Some(target) = self.gate_teleport else { return };
        if self.fade.is_some_and(|f| f.elapsed < f.steps) {
            return;
        }
        self.gate_teleport = None;
        self.pending_teleport = Some(target);
        self.fade = Some(Fade::new((0, 0, 0), 256, 0, GATE_FADE_IN));
    }

    /// Carries out a teleport requested by a script or an egg.  Done
    /// between ticks, because a map change ends all running scripts.
    pub fn apply_teleport(&mut self) {
        let Some((map, dest)) = self.pending_teleport.take() else { return };
        if !self.map_exists(map) {
            eprintln!("teleport: map {map} does not exist");
            return;
        }
        self.load_map(map);
        let pos = match dest {
            Destination::Position(x, y, z) => (x, y, z),
            Destination::Egg(n) => {
                let target = self.maps[&map].items.iter().find(|it| {
                    it.in_world() && it.frame == 1 && it.quality & 0xFF == n
                        && self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_TELEPORTEGG)
                });
                match target {
                    Some(it) => (it.x, it.y, it.z),
                    None => {
                        eprintln!("teleport: no destination egg {n} on map {map}");
                        return;
                    }
                }
            }
        };

        let changed_map = map != self.cur_map;
        if changed_map {
            // everything on the old map goes to sleep
            for idx in std::mem::take(&mut self.fast_items) {
                let it = self.map().items[idx];
                let item = &mut self.map_mut().items[idx];
                item.fast = false;
                item.hatched = false;
                if it.flags & FLG_FAST_ONLY != 0 && it.id != AVATAR && !it.gone {
                    self.map_mut().destroy(idx);
                }
            }
            // all processes end, except those that belong to no item and
            // those of type 1 that belong to no item of the old map: a
            // script that sends the Avatar elsewhere goes on over there
            // (fade in again, end the Avatar's stasis; the arrest in the
            // palace dungeon); things of the old map are gone
            // (also those of type 1 of something the Avatar carries: it goes
            // along, and so does its script - the blackrock fades in again
            // over at the ether after Pyros is banished)
            let carried_ids = self.carried_by_avatar();
            for i in 0..self.scripts.procs.len() {
                let p = &self.scripts.procs[i];
                let goes_along = p.ptype == KEEP_ON_MAP_CHANGE && carried_ids.contains(&p.item);
                if p.item != 0 && !goes_along && (p.ptype != KEEP_ON_MAP_CHANGE || p.item >= FIRST_MAP_ID) {
                    self.terminate(i);
                }
            }
            self.scripts.procs.retain(|p| !p.terminated);
            // an earthquake ends with the map: the script that would have
            // stopped it may just have ended above (a trap that shakes the
            // ground and lets the Avatar fall into a teleporter below)
            self.quake = 0;
            self.quake_by = 0;
            self.quake_offset = (0, 0);
            self.barks.clear();
            self.audio.stop_all_sfx();
            self.ask = None;
            self.stasis = false;
            self.actors.retain(|&id, _| id == AVATAR);
            self.fade_ins.clear();

            self.close_all_gumps();
            self.stash_map_states(map);
            let old = self.avatar();
            // everything the Avatar carries comes along; the items get
            // new ids on the new map, so collect them parents first
            let mut carried: Vec<Item> = Vec::new();
            let mut open = vec![AVATAR];
            while let Some(parent) = open.pop() {
                for it in self.map().items.iter().filter(|it| !it.gone && it.parent == parent) {
                    carried.push(*it);
                    if it.id != 0 {
                        open.push(it.id);
                    }
                }
            }
            if let Some(idx) = self.map().index(AVATAR) {
                self.map_mut().destroy(idx);
            }
            self.cur_map = map;
            // the last frame showed the old map: its item indices mean nothing here
            self.view.painted.clear();
            self.map_mut().add(Item { parent: IN_WORLD, mapnum: map as u16, fast: false, ..old });
            let mut new_ids = crate::hash::HashMap::from_iter([(AVATAR, AVATAR)]);
            for it in carried {
                let parent = new_ids.get(&it.parent).copied().unwrap_or(AVATAR);
                let id = self.map_mut().add(Item { id: 0, parent, mapnum: map as u16, fast: false, ..it });
                new_ids.insert(it.id, id);
            }
            // the scripts that went along belong to the new numbers
            self.scripts.renumber_items(&new_ids);
        }
        if let Some(idx) = self.map().index(AVATAR) {
            self.map_mut().place(idx, pos.0, pos.1, pos.2);
        }
        self.just_teleported = true;
        if changed_map {
            // on another map the view goes with the Avatar again, whatever
            // a script had it look at before
            self.camera = crate::effects::CameraMode::Follow(AVATAR);
            // and is there at once: the gliding camera would otherwise slide
            // over from where it was on the old map (if that is near), and
            // people would glide from their last steps over there
            self.view.cam_smooth = None;
            self.view.cam_vel = (0.0, 0.0, 0.0);
            self.view.lerps.clear();
            self.cache_in();
            if let Err(err) = self.save_to_in_background(crate::savegame::AUTO_SLOT) {
                eprintln!("autosave: {err}");
            }
        }
        self.update_fast_area();
    }

    /// Everything the Avatar carries (in his pack and bags too).
    fn carried_by_avatar(&self) -> crate::hash::HashSet<u16> {
        let mut out = crate::hash::HashSet::default();
        let mut open = vec![AVATAR];
        while let Some(parent) = open.pop() {
            for it in self.map().items.iter().filter(|it| !it.gone && it.parent == parent && it.id != 0) {
                if out.insert(it.id) {
                    open.push(it.id);
                }
            }
        }
        out
    }

    /// Leaving the map for map `next`: the fight states of its own
    /// creatures (numbers from 256 on) stay with it, those of `next` come
    /// back. A changeling in the Avatar's shape takes its own again.
    pub fn stash_map_states(&mut self, next: usize) {
        for (id, _) in std::mem::take(&mut self.changelings) {
            if let Some(idx) = self.map().index(id) {
                if self.map().items[idx].shape != crate::combat::CHANGELING {
                    self.map_mut().items[idx].shape = crate::combat::CHANGELING;
                    self.map_mut().items[idx].frame = 0;
                }
            }
        }
        let local: Vec<u16> = self.states.keys().copied().filter(|&id| id >= FIRST_MAP_ID).collect();
        let stash: crate::hash::HashMap<u16, crate::combat::ActorState> = local.into_iter().filter_map(|id| Some((id, self.states.remove(&id)?))).collect();
        self.script_made.retain(|&id| id < FIRST_MAP_ID);
        self.calmed.retain(|&id| id < FIRST_MAP_ID);
        self.map_states.insert(self.cur_map, stash);
        if let Some(back) = self.map_states.remove(&next) {
            self.states.extend(back);
        }
    }

    /// A savegame from before the fight states stayed with their map: the
    /// states of creatures from another map that now name something else
    /// here are dropped, and what was moved for them (floor tiles walking
    /// off as fighters, sinking into the ground) goes back to its place.
    pub fn repair_foreign_states(&mut self) {
        let mut foreign: Vec<u16> = self
            .states
            .keys()
            .chain(self.actors.keys())
            .copied()
            .filter(|&id| id >= FIRST_MAP_ID && self.map().get(id).is_none_or(|it| !self.can_fight(it)))
            .collect();
        foreign.sort_unstable();
        foreign.dedup();
        if foreign.is_empty() {
            return;
        }
        // the map as it was made, for where its fixed things belong
        let fresh = self.original_map(self.cur_map);
        for id in foreign {
            self.states.remove(&id);
            // (its walking and falling too)
            self.actors.remove(&id);
            self.kill_processes(id, crate::vm::PROC_TYPE_ALL);
            let (Some(idx), Some(orig)) = (self.map().index(id), fresh.get(id).copied()) else { continue };
            let it = self.map().items[idx];
            let fixed = self.data.infos.get(it.shape as usize).is_some_and(|si| si.fixed);
            if fixed && it.shape == orig.shape && (it.x, it.y, it.z) != (orig.x, orig.y, orig.z) {
                eprintln!("repair: item {id} (shape {}) back from {},{},{} to {},{},{}", it.shape, it.x, it.y, it.z, orig.x, orig.y, orig.z);
                self.map_mut().place(idx, orig.x, orig.y, orig.z);
            }
        }
    }

    /// A number for a new creature. Actors need one of the ids below 256.
    /// The numbers of the permanent characters are theirs wherever they
    /// are (on another map too): a creature with such a number would get
    /// that character's strength, side and immortality. A person a script
    /// just sent to another map keeps their number too, and with it the
    /// scripts still running for them (a theurgist who vanishes when
    /// attacked, in the middle of the scene that holds the Avatar still).
    pub fn free_actor_id(&self) -> Option<u16> {
        (2..256u16).find(|id| {
            self.actor_anywhere(*id).is_none() && !self.npc_stats.contains_key(id)
        })
    }

    /// Whether a creature standing here would be seen in the view (with
    /// some room around it: its picture reaches beyond its foot point).
    fn spot_in_view(&self, x: i32, y: i32, z: i32) -> bool {
        let (cx, cy, cz) = self.camera_position();
        let sx = (x - y) / 4 - (cx - cy) / 4;
        let sy = (x + y) / 8 - z - ((cx + cy) / 8 - cz);
        let (w, h) = self.view.size;
        sx.abs() <= w / 2 + 64 && sy >= -h / 2 - 16 && sy <= h / 2 + 128
    }

    /// How strongly a creature fading in is drawn (0..=256), if it is.
    pub fn fade_in_alpha(&self, id: u16) -> Option<u32> {
        let age = self.tick.wrapping_sub(*self.fade_ins.get(&id)?);
        (age < FADE_IN_TICKS).then(|| (age + 1) * 256 / (FADE_IN_TICKS + 1))
    }

    /// A monster egg creates its creature. Returns the new actor's id.
    pub fn hatch_monster(&mut self, egg: &Item) -> u16 {
        let id = self.create_monster(egg, true);
        if id != 0 && self.options.monster_fade && self.spot_in_view(egg.x, egg.y, egg.z) {
            let now = self.tick;
            self.fade_ins.retain(|_, start| now.wrapping_sub(*start) < FADE_IN_TICKS);
            self.fade_ins.insert(id, now);
        }
        if std::env::var_os("PAGAN_DEBUG").is_some() {
            eprintln!("hatch: egg {} at {},{},{} shape {} activity {} -> actor {id}", egg.id, egg.x, egg.y, egg.z, egg.quality & 0x7FF, egg.mapnum & 7);
        }
        id
    }

    /// Creates a creature of the kind given in the "egg" at its place.
    /// `need_room`: only if nothing is in the way there.
    pub fn create_monster(&mut self, egg: &Item, need_room: bool) -> u16 {
        let shape = (egg.quality & 0x7FF) as u32;
        if shape == 0 || (need_room && !self.can_exist(shape, egg.x, egg.y, egg.z, 0)) {
            return 0;
        }
        let Some(id) = self.free_actor_id() else { return 0 };
        let item = Item {
            x: egg.x,
            y: egg.y,
            z: egg.z,
            shape,
            flags: FLG_FAST_ONLY | FLG_DISPOSABLE | FLG_IN_NPC_LIST,
            npc: id,
            mapnum: self.cur_map as u16,
            id,
            fast: true,
            ..Item::default()
        };
        self.map_mut().add(item);
        if let Some(idx) = self.map().index(id) {
            self.fast_items.push(idx);
        }
        // start in the standing pose
        let dir = (self.rng.next() % 8) as u8;
        let mut mover = Mover::new(dir);
        mover.play(anim::STAND, &item, &mut self.data.anims);
        self.actors.insert(id, mover);
        // a fresh creature, even if the id was used before
        self.states.remove(&id);
        self.script_made.remove(&id);
        self.calmed.remove(&id);
        self.changelings.remove(&id);
        self.state(id);
        // what the egg says the creature does: 0 wander about, 1 look for a
        // fight, 2 stand where it is (the toraxen in their pens on map 7):
        // such a one does not notice the Avatar, it only fights back.
        // Townsfolk (the child in the caves below the Obsidian Fortress)
        // never look for a fight: to their script a foe means they were
        // attacked, and a sorcerer comes to sentence the Avatar.
        let townsfolk = self.state(id).alignment & crate::combat::TOWNSFOLK != 0;
        match egg.mapnum & 7 {
            0 => self.start_loitering(id),
            1 if townsfolk => self.start_loitering(id),
            1 => self.set_in_combat(id, true),
            2 => {
                self.calmed.insert(id);
            }
            _ => {}
        }
        id
    }
}
