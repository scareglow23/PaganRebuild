//! Engine functions callable from usecode ("intrinsics").
//!
//! Numbers and argument layouts follow the original game as documented by
//! Pentagram / ScummVM.  Functions that are not implemented yet are
//! reported once and return 0 (or a process that ends at once, for those
//! the scripts wait on).

use crate::anim;
use crate::game::{Bark, Game, AVATAR, GUARDIAN};
use crate::intrinsic_names::NAMES;
use crate::map::{Item, WorldBox, ETHEREAL, FLG_FLIPPED, IN_WORLD};
use crate::vm::Kind;

/// The process type of an egg's hatch script.
const PTYPE_HATCH: u16 = 0x210;

/// Reads the arguments of an intrinsic call (top of the stack first).
struct Args<'a> {
    d: &'a [u8],
    p: usize,
}

impl Args<'_> {
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.d.get(self.p).copied().unwrap_or(0), self.d.get(self.p + 1).copied().unwrap_or(0)]);
        self.p += 2;
        v
    }
    fn i16(&mut self) -> i32 {
        self.u16() as i16 as i32
    }
    fn u32(&mut self) -> u32 {
        let lo = self.u16() as u32;
        lo | ((self.u16() as u32) << 16)
    }
}

/// Direction (0 = north, clockwise) of an offset in world coordinates.
pub fn world_dir(dx: i32, dy: i32) -> u32 {
    if dx == 0 && dy == 0 {
        return 0;
    }
    let octant = ((dx as f32).atan2(-dy as f32) / std::f32::consts::FRAC_PI_4).round() as i32;
    octant.rem_euclid(8) as u32
}

impl Game {
    fn item(&self, id: u16) -> Option<Item> {
        self.map().get(id).copied().or_else(|| self.just_destroyed.get(&id).copied())
    }

    pub fn with_item_pub(&mut self, id: u16, f: impl FnOnce(&mut Item)) {
        self.with_item(id, f);
    }

    fn with_item(&mut self, id: u16, f: impl FnOnce(&mut Item)) {
        if let Some(idx) = self.map().index(id) {
            f(&mut self.map_mut().items[idx]);
        }
    }

    /// An actor's item wherever it is: on this map, another loaded one, or
    /// one not loaded yet. Scripts set the status of people who are still
    /// elsewhere (Beren is made invisible before he is fetched).
    fn with_actor_anywhere<R>(&mut self, id: u16, f: impl FnOnce(&mut Item) -> R) -> Option<R> {
        if let Some(idx) = self.map().index(id) {
            return Some(f(&mut self.map_mut().items[idx]));
        }
        if id == 0 || id >= 256 {
            return None;
        }
        if let Some(m) = self.maps.values_mut().find(|m| m.index(id).is_some()) {
            let idx = m.index(id)?;
            return Some(f(&mut m.items[idx]));
        }
        self.people.npcs.iter_mut().find(|n| n.item.id == id).map(|n| f(&mut n.item))
    }

    /// May an item of this shape be put at the position?
    pub fn can_exist(&self, shape: u32, x: i32, y: i32, z: i32, skip: u16) -> bool {
        let Some(si) = self.data.infos.get(shape as usize) else { return false };
        if !si.solid {
            return true;
        }
        let (xd, yd, zd) = si.footpad(false);
        let b = WorldBox { x, y, z, xd, yd, zd };
        // an empty start box: everything that overlaps counts
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        self.map().position_info(&b, &nowhere, &self.data.infos, skip).valid
    }

    /// Like can_exist, but the spot must also stand on something and not
    /// lie hidden under a floor: the ring of fire asks for this.
    pub fn can_stand(&self, shape: u32, x: i32, y: i32, z: i32) -> bool {
        let Some(si) = self.data.infos.get(shape as usize) else { return false };
        let (xd, yd, zd) = si.footpad(false);
        let b = WorldBox { x, y, z, xd, yd, zd };
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        let p = self.map().position_info(&b, &nowhere, &self.data.infos, 0);
        // (nothing right on top, like a flat floor or the foot of a wall:
        // there it would stand hidden below the ground)
        let above = self.map().position_info(&WorldBox { z: z + zd, ..b }, &nowhere, &self.data.infos, 0);
        p.valid && p.supported && above.valid && !above.supported
    }

    pub fn create_item(&mut self, shape: u32, frame: u32, parent: u16) -> u16 {
        let cur = self.cur_map as u16;
        self.map_mut().add(Item { shape, frame, parent, mapnum: cur, ..Item::default() })
    }

    fn to_ethereal(&mut self, id: u16) {
        if let Some(idx) = self.map().index(id) {
            let prev = self.map().items[idx].parent;
            if prev != ETHEREAL {
                self.map_mut().take(idx, ETHEREAL);
                self.ethereal.push((id, prev));
            }
        }
    }

    /// Takes the item off the ethereal list; returns where it was before.
    /// A person put somewhere by a script stands on the ground below: the
    /// places scripts use (markers, eggs) are often a little above it, and
    /// in the original people fall down at once. Up in the air, every
    /// animation would stop short for want of ground.
    fn settle_actor(&mut self, id: u16) {
        let Some(it) = self.item(id).filter(|it| it.is_actor() && it.in_world()) else { return };
        let Some(info) = self.data.infos.get(it.shape as usize).copied() else { return };
        let here = crate::map::WorldBox::of(&it, &info);
        for drop in 0..=48 {
            let b = crate::map::WorldBox { z: it.z - drop, ..here };
            if b.z < 0 {
                return;
            }
            let pi = self.map().position_info(&b, &here, &self.data.infos, id);
            if !pi.valid {
                return;
            }
            if pi.supported || pi.floor_z.is_some_and(|f| b.z <= f) {
                if drop > 0 {
                    if let Some(idx) = self.map().index(id) {
                        self.map_mut().place(idx, it.x, it.y, b.z);
                    }
                }
                return;
            }
        }
    }

    /// The process calling an earthquake intrinsic, if it may change the
    /// quake: not while a newer script's quake goes on. (Lithos beaten
    /// quickly: the summoning of his golems, begun before, let its own
    /// quake die away and stopped the one of his last words with it.)
    fn quake_caller(&self, caller: usize) -> Option<u16> {
        let pid = self.scripts.procs.get(caller).map_or(0, |p| p.pid);
        let owner = self.screen.quake_by;
        let owner_alive = owner != 0 && self.scripts.procs.iter().any(|p| p.pid == owner && !p.terminated);
        // started before the owner (the numbers count up, wrapping around)
        let older = (owner.wrapping_sub(pid) as i16) > 0;
        (!(owner_alive && pid != owner && older)).then_some(pid)
    }

    fn from_ethereal(&mut self, id: u16) -> Option<u16> {
        let pos = self.ethereal.iter().rposition(|e| e.0 == id)?;
        Some(self.ethereal.remove(pos).1)
    }

    /// A process that ends at once, for effects that are not there yet.
    fn dummy_process(&mut self) -> u32 {
        self.add_native(Kind::Delay(1), 0, 0) as u32
    }

    pub(crate) fn bark(&mut self, item: u16, text: String) -> u32 {
        let text = text.trim().to_string();
        if text.is_empty() {
            return 0;
        }
        self.note_spells(&text);
        // as long as the recorded voice takes, or long enough to read
        // the Guardian (not in the world) has his voice under his own number
        let shape = if item == GUARDIAN { GUARDIAN as u32 } else { self.item(item).map_or(0, |it| it.shape) };
        let ticks = match self.audio.speak(shape, &text) {
            Some(millis) => (millis as i32 * 30 / 1000 + 10).max(20),
            None => 50 + 2 * text.len() as i32,
        };
        self.barks.retain(|b| b.item != item);
        let pid = self.add_native(Kind::Delay(ticks), item, 0);
        self.barks.push(Bark { pid, item, text });
        pid as u32
    }

    /// cSetActivity: 0 = wander about, 1 = fight, 2 = stand. Returns the
    /// process to wait for.
    fn set_activity(&mut self, id: u16, activity: u16) -> u32 {
        // a creature a script made and sent into a fight behaves
        // like one from a monster egg; one sent wandering about
        // keeps to itself (the theurgists on the Plane of Air, who
        // are there to talk to, do not see the Avatar as a foe)
        if activity == 1 {
            self.fights.script_made.remove(&id);
        }
        // a new activity other than fighting ends a fight
        if activity != 1 && self.state(id).in_combat {
            self.set_in_combat(id, false);
        }
        match activity {
            0 => {
                self.start_loitering(id);
                self.dummy_process()
            }
            1 => {
                self.set_in_combat(id, true);
                0
            }
            _ => {
                // standing: it does not notice the Avatar on sight
                self.fights.calmed.insert(id);
                self.animate(id, anim::STAND);
                0
            }
        }
    }

    /// A new actor outside the world (as a script makes one), or None if
    /// all actor numbers are taken.
    fn create_ethereal_actor(&mut self, shape: u32, frame: u32) -> Option<u16> {
        let id = self.free_actor_id()?;
        // a fresh creature, even if the number was used before
        self.fights.states.remove(&id);
        self.fights.full_hp.remove(&id);
        self.actors.remove(&id);
        let cur = self.cur_map as u16;
        self.map_mut().add(Item {
            shape,
            frame,
            id,
            npc: id,
            parent: ETHEREAL,
            mapnum: cur,
            flags: crate::eggs::FLG_FAST_ONLY | crate::eggs::FLG_DISPOSABLE | crate::eggs::FLG_IN_NPC_LIST,
            ..Item::default()
        });
        self.ethereal.push((id, ETHEREAL));
        self.fights.script_made.insert(id);
        Some(id)
    }

    /// Whether a flame of a ring of fire may stand at a height (None: the
    /// usual test decides). The scripts try height after height (wrapping
    /// round all 256 below the caster's feet) until one fits. A column
    /// without ground anywhere (inside a wall): the caster's own height ends
    /// the search, or it would go on forever; the flame then appears at
    /// his feet, or not at all where there is no room.
    fn fire_ring_fits(&mut self, caller: usize, shape: u32, x: i32, y: i32, z: i32) -> Option<bool> {
        if self.can_stand(shape, x, y, z) {
            return Some(true);
        }
        if (0..32).any(|k| self.can_stand(shape, x, y, z % 8 + 8 * k)) {
            return Some(false);
        }
        let caster = self.scripts.procs.get(caller).map(|p| p.item).and_then(|id| self.top_item(id))?;
        Some(z == caster.z & 0xFF)
    }

    /// A conversation: the player picks one of the answers in string list
    /// `list`. Returns the process that waits for the choice (0: no answers).
    fn ask_answers(&mut self, id: u16, list: u16) -> u32 {
        let choices: Vec<String> = match self.scripts.lists.get(&list) {
            Some(l) if l.elem == 2 => (0..l.len()).map(|i| self.string(u16::from_le_bytes([l.get(i)[0], l.get(i)[1]])).to_string()).collect(),
            _ => Vec::new(),
        };
        if choices.is_empty() {
            return 0;
        }
        // (sleeping until a time of day: times instead of numbers)
        let choices = match self.sleep_choices(id, &choices) {
            Some(times) => times,
            None => {
                self.sleep_menu = None;
                choices
            }
        };
        let pid = self.add_native(Kind::Ask, id, 0);
        self.ask = Some((pid, choices));
        pid as u32
    }

    /// A script destroys an item (never the Avatar).
    fn destroy_by_script(&mut self, id: u16) {
        // A person who is not on this map is not destroyed here: in
        // the original people are kept apart from the maps. (Torwin
        // drowns in two spots of water at once: the first sends him
        // away dead, the second no longer finds him a person here
        // and destroyed him, which brought him back to life.)
        if self.people.npc_stats.contains_key(&id) && self.map().index(id).is_none() {
            return;
        }
        self.from_ethereal(id);
        if id == AVATAR {
            return;
        }
        if let Some(idx) = self.map().index(id) {
            let it = self.map().items[idx];
            self.just_destroyed.insert(id, it);
            self.map_mut().destroy(idx);
        }
        self.actors.remove(&id);
        self.fights.states.remove(&id);
    }

    /// A new item in the world at a position.
    fn create_item_at(&mut self, shape: u32, frame: u32, (x, y, z): (i32, i32, i32)) -> u16 {
        let id = self.create_item(shape, frame, ETHEREAL);
        if let Some(idx) = self.map().index(id) {
            self.map_mut().place(idx, x, y, z);
        }
        id
    }

    pub fn intrinsic(&mut self, caller: usize, func: u16, args: &[u8]) -> u32 {
        let mut a = Args { d: args, p: 0 };
        // most functions start with a pointer to the item they act on
        macro_rules! this {
            () => {{
                let ptr = a.u32();
                self.ptr_to_object(ptr)
            }};
        }
        macro_rules! this_item {
            () => {{
                let id = this!();
                match self.item(id) {
                    Some(it) => it,
                    None => return 0,
                }
            }};
        }

        // a look script running on the side may only ask (hover.rs)
        if self.name_probe.is_some() {
            let said = |g: &Game| {
                let mut a = Args { d: args, p: 4 };
                let ptr = a.u32();
                g.string(g.ptr_to_object(ptr)).to_string()
            };
            if let Some(r) = self.probe_intrinsic(caller, func, said) {
                return r;
            }
        }
        match func {
            // ---- what the player is asked for
            0x00 => self.start_targeting(),
            0x4B => {
                // getSliderInput(min, max, step): the script sleeps until confirmed
                let _ = a.u32();
                let (min, max, step) = (a.i16(), a.i16(), a.i16());
                self.start_slider(caller, min, max, step);
                0
            }

            // ---- item properties
            0x02 => 0, // touch
            // (people on other maps too: their schedule asks where they are)
            0x03..=0x05 => {
                let id = this!();
                let Some((_, it)) = self.actor_anywhere(id) else { return 0 };
                let (x, y, z) = self.abs_pos(&it);
                [x, y, z][func as usize - 3] as u32
            }
            0x06..=0x08 => {
                // centre of the bounding box
                let it = this_item!();
                let (xd, yd, zd) = self.data.infos.get(it.shape as usize).map_or((0, 0, 0), |si| si.footpad(it.flags & FLG_FLIPPED != 0));
                let (x, y, z) = self.abs_pos(&it);
                (match func {
                    0x06 => x - xd / 2,
                    0x07 => y - yd / 2,
                    _ => z + zd / 2,
                }) as u32
            }
            0x0C => {
                let it = this_item!();
                let ptr = a.u32();
                let mut buf = [0u8; 5];
                let (x, y, z) = self.abs_pos(&it);
                buf[0..2].copy_from_slice(&(x as u16).to_le_bytes());
                buf[2..4].copy_from_slice(&(y as u16).to_le_bytes());
                buf[4] = z as u8;
                self.assign(ptr, &buf);
                0
            }
            0x0D => this_item!().shape,
            0x0E => {
                let id = this!();
                let v = a.u16() as u32;
                self.with_item(id, |it| it.shape = v);
                0
            }
            0x0F => this_item!().frame,
            0x10 => {
                let id = this!();
                let v = a.u16() as u32;
                self.with_item(id, |it| it.frame = v);
                0
            }
            0x11 | 0x12 | 0x13 | 0x17 | 0x19 => this_item!().quality as u32,
            0x18 | 0x1A | 0x1B | 0x1C | 0x1D => {
                let id = this!();
                let v = a.u16();
                self.with_item(id, |it| it.quality = v);
                0
            }
            0x15 => {
                let p = this_item!().parent;
                if p == ETHEREAL { 0 } else { p as u32 }
            }
            0x16 => {
                // outermost container
                let mut it = this_item!();
                let mut root = 0;
                for _ in 0..16 {
                    if it.parent == IN_WORLD || it.parent == ETHEREAL {
                        break;
                    }
                    root = it.parent;
                    match self.item(root) {
                        Some(p) => it = p,
                        None => break,
                    }
                }
                root as u32
            }
            0x1E => {
                let it = this_item!();
                self.data.infos.get(it.shape as usize).map_or(0, |si| si.family as u32)
            }
            0x1F => {
                let it = this_item!();
                let bit = a.u16() as usize;
                self.data.infos.get(it.shape as usize).map_or(0, |si| ((si.raw.get(bit / 8).copied().unwrap_or(0) >> (bit % 8)) & 1) as u32)
            }
            0x20 => {
                let id = this!();
                self.with_actor_anywhere(id, |it| it.flags).unwrap_or(0) as u32
            }
            0x21 => {
                let id = this!();
                let m = a.u16();
                self.with_actor_anywhere(id, |it| it.flags |= m);
                0
            }
            0x22 => {
                let id = this!();
                let m = a.u16();
                self.with_actor_anywhere(id, |it| it.flags &= m);
                0
            }
            0x23 => {
                // footpad size in tiles, written through three pointers
                let it = this_item!();
                let si = self.data.infos.get(it.shape as usize).copied().unwrap_or_default();
                for v in [si.x, si.y, si.z] {
                    let ptr = a.u32();
                    self.assign(ptr, &(v as u16).to_le_bytes());
                }
                0
            }
            0x24 | 0x25 | 0x27 => {
                // touches / overlaps / is on another item
                let it = this_item!();
                let Some(other) = self.item(a.u16()) else { return 0 };
                let (Some(si), Some(so)) = (self.data.infos.get(it.shape as usize), self.data.infos.get(other.shape as usize)) else { return 0 };
                let (b1, b2) = (WorldBox::of(&it, si), WorldBox::of(&other, so));
                // (not through a floor or roof: see `floor_between`)
                let apart = |low: &WorldBox, high: &WorldBox, low_si: &crate::shape::ShapeInfo, skip: u16| !low_si.solid && self.floor_between(low, high, skip);
                (match func {
                    0x25 => b1.overlaps(&b2) && !apart(&b1, &b2, si, other.id) && !apart(&b2, &b1, so, it.id),
                    0x27 => b1.overlaps_xy(&b2) && b1.z == b2.z + b2.zd,
                    _ => b1.overlaps_xy(&b2),
                }) as u32
            }
            0x42 => this_item!().is_actor() as u32,
            0x50 => this_item!().mapnum as u32,
            0x52 => {
                let id = this!();
                let v = a.u16();
                self.with_item(id, |it| it.mapnum = v);
                0
            }
            0x55 => {
                // canReach(other, range): near enough (line of sight is not checked)
                let id = this!();
                let other = a.u16();
                let range = a.i16();
                self.can_reach(id, other, range) as u32
            }
            0x56 => {
                // getRange: the space between the two footprints in world
                // units, along the axis where it is largest (0 if they touch)
                let it = this_item!();
                let Some(o) = self.item(a.u16()) else { return 0 };
                let pad = |i: &Item| self.data.infos.get(i.shape as usize).map_or((0, 0), |si| {
                    let b = WorldBox::of(i, si);
                    (b.xd, b.yd)
                });
                let ((ixd, iyd), (oxd, oyd)) = (pad(&it), pad(&o));
                let gap = [it.x - ixd - o.x, o.x - oxd - it.x, it.y - iyd - o.y, o.y - oyd - it.y];
                gap.into_iter().max().unwrap_or(0).max(0) as u32
            }
            0x58 | 0x59 => {
                let it = this_item!();
                let (x, y) = (a.u16() as i32, a.u16() as i32);
                let d = world_dir(x - it.x, y - it.y);
                if func == 0x58 { d } else { (d + 4) & 7 }
            }
            0x5A | 0x5B => {
                let it = this_item!();
                let Some(o) = self.item(a.u16()) else { return 0 };
                let d = world_dir(o.x - it.x, o.y - it.y);
                if func == 0x5A { d } else { (d + 4) & 7 }
            }

            // ---- creating, moving and destroying items
            0x31 | 0x33 => {
                // create an item at a position if there is room
                let out = a.u32();
                let shape = a.u16() as u32;
                let frame = a.u16() as u32;
                let (x, y, z) = if func == 0x31 {
                    (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF)
                } else {
                    let ptr = a.u32();
                    let p = self.deref(ptr, 5).unwrap_or_else(|| vec![0; 5]);
                    (u16::from_le_bytes([p[0], p[1]]) as i32, u16::from_le_bytes([p[2], p[3]]) as i32, p[4] as i32)
                };
                if !self.can_exist(shape, x, y, z, 0) {
                    return 0;
                }
                let id = self.create_item_at(shape, frame, (x, y, z));
                self.assign(out, &id.to_le_bytes());
                1
            }
            0x32 => {
                // create an item outside the world
                let out = a.u32();
                let shape = a.u16() as u32;
                let frame = a.u16() as u32;
                let id = self.create_item(shape, frame, ETHEREAL);
                self.ethereal.push((id, ETHEREAL));
                self.assign(out, &id.to_le_bytes());
                id as u32
            }
            0x35 => {
                let id = this!();
                self.to_ethereal(id);
                0
            }
            // The pops take the item on top of the stack of things outside
            // the world, whatever item they are called on (as in the
            // original): `egg.pop(npc)` puts the thing just created into
            // the npc, not the egg.
            0x36 => {
                // back into the world at a position
                let _ = this!();
                let (x, y, z) = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF);
                let Some(id) = self.ethereal.last().map(|e| e.0) else { return 0 };
                self.from_ethereal(id);
                if let Some(idx) = self.map().index(id) {
                    self.map_mut().place(idx, x, y, z);
                }
                self.settle_actor(id);
                id as u32
            }
            0x37 | 0x39 => {
                // into a container
                let _ = this!();
                let container = a.u16();
                let Some(id) = self.ethereal.last().map(|e| e.0) else { return 0 };
                self.from_ethereal(id);
                if let (Some(idx), true) = (self.map().index(id), self.item(container).is_some()) {
                    let map = self.map_mut();
                    map.take(idx, container);
                    // no place in the container's window yet: it gets one when shown
                    let it = &mut map.items[idx];
                    (it.x, it.y, it.z) = (0, 0xFFFF, 0);
                    it.flags |= crate::drag::FLG_CONTAINED;
                }
                id as u32
            }
            0x38 => {
                // back to where it was
                let _ = this!();
                let Some(id) = self.ethereal.last().map(|e| e.0) else { return 0 };
                if let (Some(prev), Some(idx)) = (self.from_ethereal(id), self.map().index(id)) {
                    let it = self.map().items[idx];
                    if prev == IN_WORLD {
                        self.map_mut().place(idx, it.x, it.y, it.z);
                    } else {
                        self.map_mut().take(idx, prev);
                    }
                }
                id as u32
            }
            0x3A => {
                let id = this!();
                self.destroy_by_script(id);
                0
            }
            0x3E => {
                let id = this!();
                let (x, y, z) = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF);
                if let Some(idx) = self.map().index(id) {
                    self.map_mut().place(idx, x, y, z);
                }
                // an actor put there stays there, also in the air
                if let Some(m) = self.actors.get_mut(&id) {
                    m.placed = true;
                }
                0
            }
            0x40 => {
                // move if there is room
                let it = this_item!();
                let ptr = a.u32();
                let p = self.deref(ptr, 5).unwrap_or_else(|| vec![0; 5]);
                let (x, y, z) = (u16::from_le_bytes([p[0], p[1]]) as i32, u16::from_le_bytes([p[2], p[3]]) as i32, p[4] as i32);
                if !self.can_exist(it.shape, x, y, z, it.id) {
                    return 0;
                }
                if let Some(idx) = self.map().index(it.id) {
                    self.map_mut().place(idx, x, y, z);
                }
                // what it comes to is touched, as by a walking actor
                if it.in_world() {
                    self.touch_items(it.id, 0);
                }
                1
            }
            0x44 => {
                // throw: speeds per tick and gravity
                let id = this!();
                let speed = (a.i16(), a.i16(), a.i16());
                let gravity = a.i16();
                if self.item(id).is_some_and(|it| it.in_world()) {
                    self.add_native(Kind::Hurl(crate::effects::Hurl { item: id, speed, gravity }), id, crate::effects::PROC_HURL) as u32
                } else {
                    0
                }
            }
            0x46 => {
                let id = this!();
                self.fall(id);
                0
            }
            0x47 => {
                let id = this!();
                self.grab(id);
                0
            }
            0x45 => {
                // shoot(point, speed, gravity): fly to a place in an arc
                let id = this!();
                let ptr = a.u32();
                let p = self.deref(ptr, 5).unwrap_or_else(|| vec![0; 5]);
                let target = (u16::from_le_bytes([p[0], p[1]]) as i32, u16::from_le_bytes([p[2], p[3]]) as i32, p[4] as i32);
                let (speed, gravity) = (a.u16() as i32, a.u16() as i32);
                self.shoot(id, target, speed, gravity);
                0
            }
            0x2B => {
                let id = this!();
                let delta = a.i16();
                self.ascend(id, delta) as u32
            }
            0x41 => {
                // legal_move(&container, 0): into a container, if it fits
                let id = this!();
                let ptr = a.u32();
                let container = self.deref(ptr, 2).map_or(0, |c| u16::from_le_bytes([c[0], c[1]]));
                self.move_into(id, container) as u32
            }
            0x2C | 0x2D => {
                let id = this!();
                (self.weight(id) / 10) as u32
            }
            0x2E => {
                let id = this!();
                (self.surface_weight(id) / 10) as u32
            }
            0x34 => {
                // create an item inside a container
                let out = a.u32();
                let shape = a.u16() as u32;
                let frame = a.u16() as u32;
                let container = a.u16();
                if self.item(container).is_none() {
                    return 0;
                }
                let id = self.create_item(shape, frame, container);
                self.with_item(id, |it| {
                    it.flags |= crate::drag::FLG_CONTAINED;
                    it.y = 0xFFFF; // a place in the window when it is shown
                });
                self.assign(out, &id.to_le_bytes());
                1
            }
            0x3C => {
                let id = this!();
                let inside: Vec<usize> =
                    self.map().items.iter().enumerate().filter(|(_, it)| !it.gone && it.parent == id && id != 0).map(|(i, _)| i).collect();
                for idx in inside {
                    let it = self.map().items[idx];
                    self.just_destroyed.insert(it.id, it);
                    self.map_mut().destroy(idx);
                }
                0
            }
            0x3D => {
                // isExplosive: a flag of the item's type
                let it = this_item!();
                self.data.infos.get(it.shape as usize).map_or(0, |si| (si.raw[5] >> 5) as u32 & 1)
            }
            0x54 => {
                let id = this!();
                self.explode(id);
                0
            }
            0x61 => {
                // gotHit(other, damage): run the item's own reaction
                let id = this!();
                self.call_event(id, 6, &args[4.min(args.len())..]) as u32
            }
            0x6D => {
                // guardianBark(number): the Guardian speaks
                let id = this!();
                self.call_event(id, 0x15, &args[4.min(args.len())..]) as u32
            }
            0xD5 | 0xD6 => {
                // createSprite(shape, first, last, [unknown, repeats,] delay, x, y, z)
                let (shape, first, last) = (a.i16().max(0) as u32, a.i16().max(0) as u32, a.i16().max(0) as u32);
                let repeats = if func == 0xD5 {
                    let _ = a.u16();
                    a.i16()
                } else {
                    1
                };
                let delay = a.i16().max(1) as u32;
                let pos = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF);
                self.create_sprite(shape, first, last, repeats, delay, pos)
            }
            0xB8 => self.grant_peace(),
            0x81 => {
                // areEnemiesNear: anybody alive within 2048 who is fighting,
                // friend or foe, as in the original (the bedroll refuses
                // then: "This is no time to sleep")
                let id = this!();
                let Some(it) = self.item(id) else { return 0 };
                let near: Vec<u16> = self.map().items_near(it.x, it.y, 2048).into_iter().map(|i| self.map().items[i]).filter(|t| t.is_actor() && t.id != id && !t.dead).map(|t| t.id).collect();
                near.into_iter().any(|t| { let s = self.state(t); s.in_combat && !s.dead }) as u32
            }
            0xB1 => {
                // schedule(time): run the actor's schedule script now
                let id = this!();
                let time = a.u32();
                // Pyros' script sends Malchir home by his schedule before it
                // ends the scene, and during the scene the schedule does
                // nothing: he stood outside the world until the next full
                // hour (and the fortress's greeting looked for him there)
                if self.flag("inDaemonScene") != 0 && !self.sched_after_scene.contains(&id) {
                    self.sched_after_scene.push(id);
                }
                if self.state(id).dead { 0 } else { self.call_event(id, 8, &time.to_le_bytes()) as u32 }
            }
            0xB0 => self.airwalk as u32,
            0xBD => {
                // igniteChaos(x, y, z): every oil flask around the point goes off
                let (x, y) = (a.u16() as i32, a.u16() as i32);
                let flasks: Vec<u16> = self.map().items_near(x, y, 160).into_iter().map(|i| self.map().items[i]).filter(|it| it.shape == 592 && it.id != 0).map(|it| it.id).collect();
                for id in flasks {
                    self.call_event(id, 1, &[]);
                }
                0
            }
            0xFB => {
                // the sorcerer's homing fireball: (?, ?, x, y, z)
                let _ = (a.u16(), a.u16());
                let pos = (a.i16(), a.i16(), a.u16() as i32 & 0xFF);
                self.launch_fireball(pos);
                0
            }
            0xE4 => {
                // the end: the last film, then the credits; from now on the
                // main menu also offers the quotes and the end film
                if !self.options.seen_end {
                    self.options.seen_end = true;
                    self.save_settings();
                }
                if !self.play_movie("ENDGAME.SKF", crate::title::Next::Credits) {
                    self.open_credits();
                }
                0
            }
            0xFD => 0, // MakeAvatarACheater: only marks the savegame
            0xDB => {
                // SetTimeInGameHours: an hour has 27000 ticks
                let hour = a.u16() as u32;
                // (after sleeping until a time of day: that hour)
                let hour = self.sleep_hour(hour);
                self.time_offset = (hour * 27000).wrapping_sub(self.tick);
                self.sched_last_hour = self.sched_last_hour.min(hour.saturating_sub(1));
                0
            }
            0xD1 => self.ethereal.last().map_or(0, |e| e.0 as u32),
            0xD4 => {
                let shape = a.u16() as u32;
                let (x, y, z) = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF);
                // the ring of fire of the fire spells (the place for
                // each flame is tried with the shape 398) only on the ground
                const FIRE_SPELLS: u16 = 0x0583;
                let fire_ring = shape == 398 && self.scripts.procs.get(caller).is_some_and(|p| p.class == FIRE_SPELLS);
                if fire_ring {
                    if let Some(fits) = self.fire_ring_fits(caller, shape, x, y, z) {
                        return fits as u32;
                    }
                }
                self.can_exist(shape, x, y, z, 0) as u32
            }
            0xD7 => {
                let shape = a.u16() as usize;
                self.data.infos.get(shape).map_or(0, |si| si.family as u32)
            }

            // ---- events and speech
            0x49 => {
                let id = this!();
                let sptr = a.u32();
                let text = self.string(self.ptr_to_object(sptr)).to_string();
                // (sleeping until a time of day: the Avatar asks instead of the bed)
                let (id, text) = self.sleep_question(id, &text).unwrap_or((id, text));
                // (the recall teleporter's question goes where its answers are)
                let id = self.question_speaker(id, &text).unwrap_or(id);
                self.bark(id, text)
            }
            0x4A => {
                // conversation: the player picks one of the answers
                let id = this!();
                let list = a.u16();
                self.ask_answers(id, list)
            }
            0x5C | 0x5D | 0x67 => {
                let id = this!();
                let event = match func {
                    0x5C => 0,
                    0x5D => 1,
                    _ => 15,
                };
                self.call_event(id, event, &[]) as u32
            }
            0x6E | 0x6F => {
                // books and scrolls
                let id = this!();
                let sptr = a.u32();
                let text = self.string(self.ptr_to_object(sptr)).to_string();
                let look = if func == 0x6E { crate::gumps::Look::Book } else { crate::gumps::Look::Scroll };
                self.note_spells(&text);
                self.read_text(id, &text, look)
            }
            0x70 | 0x71 => {
                // gravestones and plaques: (shape of the gump, text)
                let id = this!();
                let gump = a.u16() as u32;
                let sptr = a.u32();
                let text = self.string(self.ptr_to_object(sptr)).to_string();
                let font = if func == 0x70 { crate::gumps::GRAVE_FONT } else { crate::gumps::PLAQUE_FONT };
                self.note_spells(&text);
                self.read_text(id, &text, crate::gumps::Look::Sign { gump, font })
            }
            0x4C => {
                let id = this!();
                let shape = a.u16() as u32;
                self.open_gump(id, shape);
                0
            }
            0x4D => {
                let id = this!();
                self.close_gump(id);
                0
            }
            0x4E => {
                let id = this!();
                self.ui.gumps.iter().any(|g| g.owner == id) as u32
            }

            // ---- eggs
            0x72 => ((this_item!().npc >> 4) & 0xF) as u32,
            0x73 => (this_item!().npc & 0xF) as u32,
            0x74 | 0x75 => {
                // egg range in tiles, x in the high and y in the low nibble
                let id = this!();
                let r = a.u16() & 0xF;
                self.with_item(id, |it| it.npc = if func == 0x74 { (it.npc & 0x0F) | (r << 4) } else { (it.npc & 0xF0) | r });
                0
            }
            0x76 => this_item!().mapnum as u32,
            0x78 => {
                let id = this!();
                self.call_event(id, 7, &[]) as u32
            }
            0x79 => {
                let egg = this_item!();
                self.hatch_monster(&egg) as u32
            }
            0x7A => (this_item!().mapnum >> 3) as u32,
            0xCC => {
                // teleportToEgg(map, egg number, put in stasis)
                let (map, n) = (a.u16() as usize, a.u16());
                let target = (map, crate::eggs::Destination::Egg(n));
                // an egg's script that is a teleporter (the way from the
                // caves to Carthax Lake) fades like the other teleporters;
                // other scripts fade themselves where they want it
                if self.scripts.procs.get(caller).is_some_and(|p| p.ptype == PTYPE_HATCH) && self.gate_teleport.is_none() {
                    self.gate_teleport_to(target);
                } else {
                    self.pending_teleport = Some(target);
                }
                self.dummy_process()
            }
            0x77 => {
                let id = this!();
                let v = a.u16();
                self.with_item(id, |it| it.mapnum = v);
                0
            }

            // ---- actors
            0x80 => {
                let id = this!();
                self.actors.get(&id).map_or(0, |m| m.busy() as u32)
            }
            0x82 => {
                let id = this!();
                self.state(id).in_combat as u32
            }
            0x83 | 0x84 => {
                let id = this!();
                if func == 0x84 && self.state(id).in_combat {
                    self.fights.calmed.insert(id);
                }
                self.set_in_combat(id, func == 0x83);
                0
            }
            0x85 => {
                // setTarget: as in ScummVM, an actor given someone to fight
                // goes into combat (the ghouls rising from graves, EGHOUL)
                let id = this!();
                let target = a.u16();
                if target != 0 && id != AVATAR && !self.state(id).in_combat {
                    self.set_in_combat(id, true);
                }
                self.state_mut(id).target = target;
                0
            }
            0x86 => {
                let id = this!();
                self.state(id).target as u32
            }
            0x87 | 0x89 => {
                let id = this!();
                let v = a.u16() as u8 & 0x0F;
                let s = self.state_mut(id);
                if func == 0x87 { s.alignment = v } else { s.enemy_alignment = v }
                0
            }
            0x88 => {
                let id = this!();
                self.state(id).alignment as u32
            }
            0x8A => {
                let id = this!();
                self.state(id).enemy_alignment as u32
            }
            0x8B => {
                let id = this!();
                let other = a.u16();
                (self.state(id).enemy_alignment & self.state(other).alignment != 0) as u32
            }
            0x8C => {
                let id = this!();
                self.state(id).dead as u32
            }
            0x8D | 0x8E => {
                let id = this!();
                self.state_mut(id).dead = func == 0x8D;
                if let Some(idx) = self.map().index(id) {
                    self.map_mut().items[idx].dead = func == 0x8D;
                }
                0
            }
            0x8F => {
                let id = this!();
                self.state(id).immortal as u32
            }
            0x90 | 0x91 => {
                let id = this!();
                self.state_mut(id).immortal = func == 0x90;
                0
            }
            0x92 => {
                let id = this!();
                self.vitals.withstand.contains(&id) as u32
            }
            0x93 => {
                let id = this!();
                self.vitals.withstand.insert(id);
                0
            }
            0x94 => {
                let id = this!();
                self.vitals.withstand.remove(&id);
                0
            }
            0x95 => {
                let id = this!();
                self.feigning_death(id) as u32
            }
            0x96 => {
                let id = this!();
                self.feign_death(id);
                0
            }
            0x53 => {
                // receiveHit(other, direction, damage, damage type)
                let id = this!();
                let other = a.u16();
                let dir = a.u16() as u8 & 7;
                let damage = a.i16();
                let dtype = a.u16();
                if self.item(id).is_some_and(|it| it.is_actor()) {
                    self.receive_script_hit(id, other, dir, damage, dtype);
                }
                0
            }
            0xAE => {
                // cSetActivity: 0 = wander about, 1 = fight, 2 = stand
                let id = this!();
                let activity = a.u16();
                self.set_activity(id, activity)
            }
            0x9C => {
                let id = this!();
                self.actors.get(&id).map(|m| m.dir).or(self.people.npc_dirs.get(&id).copied()).unwrap_or(0) as u32
            }
            0x9D => {
                // getMap: where the person is, which need not be here
                let id = this!();
                self.actor_anywhere(id).map_or(self.cur_map, |(n, _)| n) as u32
            }
            0x9E => {
                let id = this!();
                let (x, y, z, map) = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF, a.u16() as usize & 0xFF);
                self.teleport(id, map, (x, y, z));
                if id != AVATAR && map == self.cur_map {
                    self.enter_fast_area_now(id);
                }
                0
            }
            0x9F => {
                // play an animation; the script waits for the process
                let id = this!();
                let action = a.u16() as u32;
                let dir = a.u16();
                let Some(it) = self.item(id) else { return 0 };
                let mut mover = self.actors.remove(&id).unwrap_or_else(|| {
                    crate::actor::Mover::new(self.people.npc_dirs.get(&id).copied().unwrap_or(4))
                });
                if dir < 8 {
                    mover.dir = dir as u8;
                }
                let ok = mover.play(action, &it, &mut self.data.anims);
                self.actors.insert(id, mover);
                if ok { self.add_native(Kind::AnimWait(id), id, 0x00F0) as u32 } else { 0 }
            }
            0xA1 | 0xA2 => {
                // walk to a point / to an item
                let id = this!();
                if self.item(id).is_none() {
                    return 0;
                }
                let mut pf = if func == 0xA1 {
                    crate::effects::Pathfind::new(id, 0, (a.u16() as i32, a.u16() as i32))
                } else {
                    let target = a.u16();
                    if self.item(target).is_none() {
                        return 0;
                    }
                    crate::effects::Pathfind::new(id, target, (0, 0))
                };
                pf.scripted = true;
                self.add_native(Kind::Pathfind(pf), id, crate::effects::PROC_WALK) as u32
            }
            0xAD => {
                // create an actor outside the world
                let out = a.u32();
                let shape = a.u16() as u32;
                let frame = a.u16() as u32;
                let Some(id) = self.create_ethereal_actor(shape, frame) else { return 0 };
                self.assign(out, &id.to_le_bytes());
                id as u32
            }
            0xB2 => {
                // getEquip(slot): slots are counted from 0 here
                let id = this!();
                let slot = a.u16() as i32 + 1;
                self.equipped(id, slot).map_or(0, |it| it.id as u32)
            }
            0xB3 => {
                let id = this!();
                let _slot = a.u16();
                let item = a.u16();
                self.equip(id, item) as u32
            }
            0xA0 => {
                let id = this!();
                self.actors.get(&id).map_or(anim::STAND, |m| m.last_anim())
            }
            0xA3..=0xA6 => {
                // strength, intelligence, dexterity, hit points
                let id = this!();
                let st = self.state(id);
                [st.strength, st.intelligence, st.dex, st.hp][(func - 0xA3) as usize].clamp(0, 255) as u32
            }
            0xA7 => {
                let _ = this!();
                self.mana.max(0) as u32
            }
            0xA8..=0xAB => {
                let id = this!();
                let v = a.u16() as i32 & 0xFF;
                let st = self.state_mut(id);
                match func {
                    0xA8 => st.strength = v,
                    0xA9 => st.intelligence = v,
                    0xAA => st.dex = v,
                    _ => st.hp = v,
                }
                0
            }
            0xAC => {
                let _ = this!();
                self.mana = a.i16();
                0
            }
            0xAF => {
                let _ = this!();
                self.airwalk = a.u16() != 0;
                0
            }
            0xCF => self.stasis as u32,
            0xD0 => {
                let on = a.u16() != 0;
                if on && !self.stasis {
                    // caught walking, running, jumping or climbing: that
                    // movement ends as usual, then he stands
                    if let Some(m) = self.actors.get_mut(&AVATAR) {
                        let a = m.last_anim();
                        m.settle = matches!(a, anim::WALK | anim::RUN | anim::JUMP_UP | anim::RUNNING_JUMP | anim::JUMP | anim::AIRWALK_JUMP | anim::LAND)
                            || (anim::CLIMB_FIRST..=anim::CLIMB_LAST).contains(&a);
                    }
                }
                self.stasis = on;
                0
            }
            0xE5 => {
                let food = a.u16();
                self.feed_avatar(food);
                0
            }
            0xE6..=0xE8 => {
                use crate::vitals::Attribute;
                let n = a.i16();
                let what = [Attribute::Strength, Attribute::Intelligence, Attribute::Dexterity][(func - 0xE6) as usize];
                self.practise(what, n);
                0
            }
            0xE9 | 0xEA => {
                self.set_in_combat(AVATAR, func == 0xEA);
                0
            }
            0xEB => self.state(AVATAR).in_combat as u32,

            // ---- processes
            0xCA => {
                let (item, ptype) = (a.u16(), a.u16());
                self.num_processes(item, ptype)
            }
            0xCB | 0xCD => {
                let (item, ptype) = (a.u16(), a.u16());
                self.kill_processes(item, ptype);
                0
            }
            0xCE => 0,

            // ---- camera and screen effects
            0xB5 | 0xBF => {
                let (x, y, z) = (a.u16() as i32, a.u16() as i32, a.u16() as i32 & 0xFF);
                if func == 0xB5 {
                    // glide there within 25 ticks; the script waits
                    let from = self.camera_position();
                    self.camera = crate::effects::CameraMode::Scroll(from, (x, y, z), 0, 25);
                    self.add_native(Kind::Delay(25), 0, 0) as u32
                } else {
                    self.camera = crate::effects::CameraMode::Fixed(x, y, z);
                    0
                }
            }
            0xBE => {
                let id = a.u16();
                self.camera = if id == 0 {
                    // follow nobody: the view stays where it is
                    let (x, y, z) = self.camera_position();
                    crate::effects::CameraMode::Fixed(x, y, z)
                } else {
                    crate::effects::CameraMode::Follow(id)
                };
                0
            }
            0xC6 => {
                let strength = a.u16() as i32;
                if let Some(pid) = self.quake_caller(caller) {
                    self.screen.quake = strength;
                    self.screen.quake_by = pid;
                }
                0
            }
            0xC7 => {
                if self.quake_caller(caller).is_some() {
                    self.screen.quake = 0;
                    self.screen.quake_by = 0;
                }
                0
            }
            0xC8 => {
                self.screen.inverted = a.u16() != 0;
                0
            }
            0xDE => self.start_fade((0, 0, 0), 0, 256, 30),
            0xDF => self.start_fade((0, 0, 0), 256, 0, 30),
            0xE2 => self.start_fade((255, 255, 255), 0, 256, 30),
            0xE3 => self.start_fade((255, 255, 255), 256, 0, 30),
            0xE1 => self.start_fade((207, 207, 207), 200, 0, 10), // lightning
            0xE0 => {
                // FadeToPalette(transform, priority): storm colours and the like
                let kind = a.u16() as u8;
                // blended over 45 ticks; the scripts wait for it (the
                // mushroom trip changes colours each time the fade is done)
                self.screen.pal_transform = kind;
                let shown = crate::palette::shown_transform(kind, self.options.storm);
                self.screen.storm_shown = self.options.storm;
                self.data.pal.fade_to_transform(shown, 45);
                self.add_native(Kind::Delay(45), 0, 0) as u32
            }

            // ---- misc
            0xB4 => {
                self.close_all_gumps();
                0
            }
            0xB6 => {
                let n = a.i16();
                if n <= 1 { 0 } else { self.rng.next() % n as u32 }
            }
            0xB7 => {
                let (lo, hi) = (a.i16(), a.i16());
                if hi <= lo { lo as u32 } else { (lo + (self.rng.next() % (hi - lo + 1) as u32) as i32) as u32 }
            }
            0xB9 => {
                let n = a.u16();
                self.new_string(n.to_string()) as u32
            }
            0xBC => self.new_string(self.avatar_name.clone()) as u32,
            // 60 Hz timer; the game runs at 30 ticks per second
            0xD2 => self.tick.wrapping_mul(2),
            0xD8 => self.game_hours(),
            0xD9 => self.game_seconds() / 60, // game minutes
            0xDA => self.game_seconds(),
            // ---- sound and music
            0xEC..=0xF1 => {
                // playSFX / playAmbientSFX (number [, priority [, object]])
                let num = a.i16();
                let _priority = if args.len() >= 4 { a.i16() } else { 0 };
                let object = if args.len() >= 6 { a.u16() } else { 0 };
                let volume = self.sfx_volume(object);
                // ambient sounds of an object loop while it is near
                self.audio.play_sfx(num, object, func >= 0xEF && object != 0, volume);
                0
            }
            0xF2 => {
                let num = a.i16();
                self.audio.is_sfx_playing(num) as u32
            }
            0xF3 => 0, // setVolumeSFX
            0xF4 | 0xF5 => {
                let num = a.i16();
                let object = if args.len() >= 4 { a.u16() } else { 0 };
                self.audio.stop_sfx(num, object);
                0
            }
            0xBB => {
                let track = a.u16() as i32 & 0xFF;
                self.audio.play_music(track);
                0
            }
            0xF8 | 0xF9 => {
                self.audio.play_music(0);
                0
            }
            0xFC => (self.cheat || self.options.avatar_menu) as u32,                     // AvatarCanCheat
            0xFE => 1,                                     // isGameRunning

            _ => {
                let name = NAMES.get(func as usize).copied().unwrap_or("?");
                if self.scripts.warned.insert(func) {
                    eprintln!("usecode: intrinsic {func:04X} not implemented: {name}");
                }
                if name.contains("process ") { self.dummy_process() } else { 0 }
            }
        }
    }
}
