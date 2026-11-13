//! Moving items with the mouse: between the world, the backpack and other
//! containers.

use crate::game::{Game, AVATAR};
use crate::map::{Item, WorldBox, IN_WORLD};
use crate::shape::{FAMILY_CONTAINER, FAMILY_GLOBEGG, FAMILY_TELEPORTEGG, FAMILY_UNKEGG};
use crate::view::Surface;

pub const FLG_CONTAINED: u16 = 0x0008;

/// How far from the Avatar things can be picked up and put down: the gap
/// between his footprint and the item's, as in the original (farther
/// away a dragged item is thrown).
const REACH: i32 = 128;
const REACH_Z: i32 = 64;

/// The item hanging on the mouse pointer.
pub struct Drag {
    pub item: u16,
    pub x: i32,
    pub y: i32,
}

/// Where a dragged item would go.
enum DropSpot {
    /// Put down there (within reach).
    Here(i32, i32, i32),
    /// Thrown towards there.
    Throw(i32, i32, i32),
}

/// What the pointer shows while an item is dragged.
#[derive(PartialEq, Clone, Copy)]
pub enum DragFeedback {
    /// Nothing is dragged.
    None,
    /// It can be put down here.
    Drop,
    /// It would be thrown.
    Throw,
    /// Too far, or in the way.
    Nothing,
}

impl Game {
    /// The container an item is ultimately in (0 if it lies in the world).
    pub(crate) fn root_container(&self, id: u16) -> u16 {
        let mut cur = id;
        for _ in 0..16 {
            match self.map().get(cur) {
                Some(it) if it.parent != IN_WORLD && it.parent != crate::map::ETHEREAL => cur = it.parent,
                _ => break,
            }
        }
        if cur == id { 0 } else { cur }
    }

    /// Where an item is in the world: for one in a container (a backpack,
    /// a chest, the Avatar's pack) the place of the outermost container,
    /// as the scripts expect (a trapped book explodes where its bag is).
    pub(crate) fn abs_pos(&self, it: &crate::map::Item) -> (i32, i32, i32) {
        let root = self.root_container(it.id);
        match self.map().get(root).filter(|_| root != 0) {
            Some(r) => (r.x, r.y, r.z),
            None => (it.x, it.y, it.z),
        }
    }

    /// Could the Avatar reach the item `id` if it lay at x, y, z?
    fn within_reach(&self, id: u16, x: i32, y: i32, z: i32) -> bool {
        let av = self.avatar();
        (self.gap_at(id, x, y, z).is_some_and(|gap| gap <= REACH) && (z - av.z).abs() <= REACH_Z)
            // (option: all over a pentacle he stands near)
            || self.pentagram_reach(x, y, z)
    }

    /// The gap between the Avatar's footprint and the item's, were it at x, y, z.
    fn gap_at(&self, id: u16, x: i32, y: i32, z: i32) -> Option<i32> {
        let it = crate::map::Item { x, y, z, ..*self.map().get(id)? };
        Some(self.box_gap(&self.avatar(), &it))
    }

    /// Is this something one can carry (wherever it is)?
    pub fn portable(&self, id: u16) -> bool {
        let Some(it) = self.map().get(id) else { return false };
        let Some(si) = self.data.infos.get(it.shape as usize) else { return false };
        // the backpack stays on the Avatar's back
        let worn_pack = it.parent == AVATAR && it.z == crate::equip::SLOT_BACKPACK && it.flags & crate::equip::FLG_EQUIPPED != 0;
        // as in the original: fixed or weightless things stay put. The "land"
        // flag is no reason - many small things (keys, reagents) carry it.
        if worn_pack || it.is_actor() || si.fixed || si.raw[6] == 0 || si.roof || si.editor
            || [FAMILY_GLOBEGG, FAMILY_UNKEGG, FAMILY_TELEPORTEGG, 7].contains(&si.family)
        {
            return false;
        }
        true
    }

    /// May the player pick this item up?
    pub fn can_drag(&self, id: u16) -> bool {
        if !self.portable(id) {
            return false;
        }
        let Some(it) = self.map().get(id) else { return false };
        if it.parent == IN_WORLD {
            self.within_reach(id, it.x, it.y, it.z)
        } else {
            // in a container: fine as long as its window is open or the Avatar carries it
            let root = self.root_container(id);
            root == AVATAR || self.ui.gumps.iter().any(|g| g.owner == it.parent)
        }
    }

    pub fn begin_drag(&mut self, id: u16, x: i32, y: i32) -> bool {
        if self.stasis || self.ask.is_some() || self.ui.reader.is_some() || !self.can_drag(id) {
            return false;
        }
        // what lies on top of it comes down
        self.grab(id);
        self.ui.drag = Some(Drag { item: id, x, y });
        true
    }

    pub fn drag_to(&mut self, x: i32, y: i32) {
        if let Some(d) = &mut self.ui.drag {
            (d.x, d.y) = (x, y);
        }
    }

    /// Would putting `id` into `container` put it inside itself?
    pub fn contains(&self, id: u16, container: u16) -> bool {
        let mut cur = container;
        for _ in 0..16 {
            if cur == id {
                return true;
            }
            match self.map().get(cur) {
                Some(it) if it.parent != IN_WORLD => cur = it.parent,
                _ => return false,
            }
        }
        false
    }

    /// Puts an item into a container at a position inside its window.
    pub fn put_in_container(&mut self, id: u16, container: u16, gx: i32, gy: i32) -> bool {
        if self.contains(id, container) || self.map().get(container).is_none() {
            return false;
        }
        if let Err(why) = self.can_add(container, id) {
            self.explain_refusal(why);
            return false;
        }
        let Some(idx) = self.map().index(id) else { return false };
        // a container that goes into another one closes its window
        self.close_gump(id);
        self.take_off(id);
        let map = self.map_mut();
        map.take(idx, container);
        let it = &mut map.items[idx];
        // contained items keep their window position in the y field
        it.x = 0;
        it.y = (gx.clamp(0, 255)) | (gy.clamp(0, 255) << 8);
        it.z = 0;
        it.flags |= FLG_CONTAINED;
        // taken off a pressure plate (or anything that noticed it lying there)
        if self.touching.contains_key(&id) {
            self.touch_items(id, 0);
        }
        true
    }

    /// If the item is being worn it comes off (its "unequip" script runs).
    pub(crate) fn take_off(&mut self, id: u16) {
        let Some(idx) = self.map().index(id) else { return };
        if self.map().items[idx].flags & crate::equip::FLG_EQUIPPED != 0 {
            self.map_mut().items[idx].flags &= !crate::equip::FLG_EQUIPPED;
            self.call_event(id, 11, &[]);
        }
    }

    /// The Avatar's backpack, if there is one.
    pub fn backpack(&self) -> Option<u16> {
        self.map()
            .items
            .iter()
            .find(|it| !it.gone && it.parent == AVATAR && self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_CONTAINER))
            .map(|it| it.id)
    }

    /// Lets go of the dragged item at a view position. If it cannot go
    /// there it stays where it was.
    pub fn drop_drag(&mut self, x: i32, y: i32) -> bool {
        let Some(drag) = self.ui.drag.take() else { return false };
        let n = self.pile_of(drag.item).len() as i32;
        self.drop_pile(drag.item, n, x, y)
    }

    /// Puts down `amount` things of the pile `id` heads (see inventory.rs),
    /// `id` first. Moved within its own slot window the others follow it
    /// by themselves. Stops at the first that cannot go there; false if
    /// not even the first could.
    fn drop_pile(&mut self, id: u16, amount: i32, x: i32, y: i32) -> bool {
        let pile = self.pile_of(id);
        let home = self.map().get(id).map(|it| it.parent);
        if !self.drop_item(id, x, y) {
            return false;
        }
        // moved within its window, or worn (or stowed) by the Avatar: only
        // the one
        let now = self.map().get(id).map(|it| it.parent);
        if now == home || now == Some(AVATAR) {
            return true;
        }
        for &other in pile.iter().skip(1).take((amount - 1).max(0) as usize) {
            if !self.drop_item(other, x, y) {
                break;
            }
        }
        true
    }

    /// Like `drop_drag`, but for a stack the player is first asked how
    /// much of it to put down (unless shift is held while letting go, or
    /// it is only moved within its own window or put on the Avatar).
    /// False if it could not be put down there.
    pub fn drop_drag_split(&mut self, x: i32, y: i32) -> bool {
        let Some(drag) = self.ui.drag.take() else { return false };
        let pile = self.pile_of(drag.item).len() as i32;
        // moved within its own window a pile goes as a whole
        let home = self.map().get(drag.item).map(|it| it.parent);
        let own_window = self.pick_gump(x, y).is_some_and(|(gi, _)| Some(self.ui.gumps[gi].owner) == home);
        // (put on the Avatar only one is worn: no question either)
        let on_avatar = match self.pick_gump(x, y) {
            Some((gi, _)) => self.ui.gumps[gi].owner == AVATAR,
            None => self.pick_world(x, y, drag.item) == Some(AVATAR),
        };
        let own_window = own_window || on_avatar;
        match self.map().get(drag.item).copied() {
            Some(it) if self.stackable(&it) && it.quality > 1 && !own_window => {
                self.start_split(it.id, it.quality as i32, x, y);
                true
            }
            Some(it) if pile > 1 && !own_window => {
                self.start_split(it.id, pile, x, y);
                true
            }
            _ => self.drop_pile(drag.item, pile, x, y),
        }
    }

    /// Puts down `amount` of a stack at a view position; the rest stays.
    pub fn drop_part(&mut self, id: u16, amount: i32, x: i32, y: i32) {
        let Some(it) = self.map().get(id).copied() else { return };
        if !self.stackable(&it) {
            // a pile of things of their own
            self.drop_pile(id, amount, x, y);
            return;
        }
        if amount >= it.quality as i32 {
            self.drop_item(id, x, y);
            return;
        }
        if amount <= 0 {
            return;
        }
        // split off a new item with the chosen amount
        let part = self.map_mut().add(Item { id: 0, quality: amount as u16, parent: crate::map::ETHEREAL, fast: false, ..it });
        self.with_item_pub(id, |i| i.quality -= amount as u16);
        if !self.drop_item(part, x, y) {
            // it could not go there: put it back on the stack
            if let Some(idx) = self.map().index(part) {
                self.destroy_with_scripts(idx);
            }
            self.with_item_pub(id, |i| i.quality += amount as u16);
        }
    }

    /// Two amounts of the same thing become one.
    /// Can two items be one stack? As in the original: things counted by
    /// amount (coins, arrows ...) whatever picture they show (coins show
    /// a bigger heap the more there are); reagents only of the same kind
    /// (the frames of the necromancy reagents come in groups).
    fn can_merge(&self, a: &crate::map::Item, b: &crate::map::Item) -> bool {
        if a.id == b.id || a.shape != b.shape || !self.stackable(a) {
            return false;
        }
        let family = self.data.infos.get(a.shape as usize).map_or(0, |si| si.family);
        if family == 2 || a.frame == b.frame {
            return true;
        }
        const NECRO_REAGENTS: u32 = 395;
        let group = |f: u32| match f {
            0..=5 => 0,
            6..=7 => 1,
            10..=12 => 2,
            14..=15 => 3,
            16..=20 => 4,
            _ => 100 + f,
        };
        a.shape == NECRO_REAGENTS && group(a.frame) == group(b.frame)
    }

    /// With the option "better inventory": a stackable thing put into the
    /// backpack joins a stack of its kind that is there already.
    pub fn join_stack(&mut self, id: u16, container: u16) -> bool {
        if !self.grid_window(container) {
            return false;
        }
        let Some(a) = self.map().get(id).copied() else { return false };
        let other = self.map().items.iter().find(|b| !b.gone && b.parent == container && self.can_merge(&a, b)).map(|b| b.id);
        other.is_some_and(|o| self.merge_stacks(id, o))
    }

    /// Joins every stack in the backpack with the others of its kind (when
    /// the slot window opens).
    pub fn join_all_stacks(&mut self, container: u16) {
        let ids: Vec<u16> = self.map().items.iter().filter(|it| !it.gone && it.parent == container && self.stackable(it)).map(|it| it.id).collect();
        for id in ids {
            // (a potion being drunk is left alone until its script is done)
            if self.num_processes(id, crate::vm::PROC_TYPE_ALL) > 0 {
                continue;
            }
            if self.map().get(id).is_some_and(|it| it.parent == container) {
                // the later one goes onto the earlier one, which keeps its slot
                let Some(a) = self.map().get(id).copied() else { continue };
                let first = self.map().items.iter().find(|b| !b.gone && b.parent == container && self.can_merge(&a, b)).map(|b| b.id);
                if let Some(f) = first.filter(|&f| self.map().index(f) < self.map().index(id) && self.num_processes(f, crate::vm::PROC_TYPE_ALL) == 0) {
                    self.merge_stacks(id, f);
                }
            }
        }
    }

    fn merge_stacks(&mut self, id: u16, onto: u16) -> bool {
        let (Some(a), Some(b)) = (self.map().get(id).copied(), self.map().get(onto).copied()) else { return false };
        if !self.can_merge(&a, &b) {
            return false;
        }
        let total = self.amount(&a) + self.amount(&b);
        if total > u16::MAX as u32 {
            return false;
        }
        // into a container only if the added amount may go there
        if b.parent != crate::map::IN_WORLD {
            if let Err(why) = self.can_add(b.parent, id) {
                self.explain_refusal(why);
                return false;
            }
        }
        self.with_item_pub(onto, |i| i.quality = total as u16);
        if let Some(idx) = self.map().index(id) {
            self.destroy_with_scripts(idx);
        }
        self.call_event(onto, 12, &[]); // "combine" script
        true
    }

    /// Puts an item down at a view position: into a container window, on
    /// the Avatar, onto a stack of the same kind, or into the world.
    /// Taking what is not the Avatar's is noticed by whoever is around.
    pub fn drop_item(&mut self, id: u16, x: i32, y: i32) -> bool {
        const FLG_OWNED: u16 = 0x0004;
        let mine_before = self.top_item(id).is_some_and(|t| t.id == AVATAR);
        let owned = self.map().get(id).is_none_or(|it| it.flags & FLG_OWNED != 0);
        if !self.place_dropped(id, x, y) {
            return false;
        }
        // as in the original, what it comes to lie on "got hit" (the plates
        // for the Blackrock stones at the end), and what it lay on before
        // is let go
        if id != AVATAR && !self.map().get(id).is_some_and(|it| it.is_actor()) {
            self.touch_items(id, 0);
        }
        if self.top_item(id).is_some_and(|t| t.id == AVATAR) {
            self.with_item_pub(id, |it| it.flags |= FLG_OWNED);
        }
        if !owned && !mine_before {
            self.avatar_stole(id);
        }
        true
    }

    /// A right click (modern controls) or shift + right click (classic) on
    /// a thing: it goes straight into the backpack, as if it were dragged
    /// there (not worn), with the same checks. None: nothing to take (the
    /// click does what it always did: things the Avatar carries already,
    /// fixed or far away things); Some(false): it is too heavy or there is
    /// no room, and the player is told so.
    pub fn take_into_backpack(&mut self, id: u16) -> Option<bool> {
        const FLG_OWNED: u16 = 0x0004;
        if self.stasis || self.ask.is_some() || self.ui.reader.is_some() || !self.player_in_control() {
            return None;
        }
        if self.root_container(id) == AVATAR || !self.can_drag(id) {
            return None;
        }
        let pack = self.backpack()?;
        // (onto a stack it needs no room, but it still weighs)
        match self.can_add(pack, id) {
            Err(why) if why == "heavy" || !self.grid_window(pack) => {
                self.explain_refusal(why);
                return Some(false);
            }
            _ => {}
        }
        let owned = self.map().get(id).is_none_or(|it| it.flags & FLG_OWNED != 0);
        // what lies on top of it comes down
        self.grab(id);
        let joined = self.join_stack(id, pack);
        if !joined {
            let (gx, gy) = (8 + (self.rng.next() % 40) as i32, 8 + (self.rng.next() % 30) as i32);
            if !self.put_in_container(id, pack, gx, gy) {
                return Some(false);
            }
            self.with_item_pub(id, |it| it.flags |= FLG_OWNED);
        }
        if !owned {
            self.avatar_stole(id);
        }
        Some(true)
    }

    /// Everybody near the Avatar sees him take `id` (their script decides
    /// whether they mind).
    fn avatar_stole(&mut self, id: u16) {
        const EVENT_STOLE: u32 = 0x13;
        let av = self.avatar();
        let near: Vec<u16> = self
            .map()
            .items
            .iter()
            .filter(|it| it.is_actor() && it.id != AVATAR && !it.dead && !it.gone && it.in_world())
            .filter(|it| (it.x - av.x).abs() <= 640 && (it.y - av.y).abs() <= 640)
            .map(|it| it.id)
            .collect();
        for actor in near {
            if !self.state(actor).dead {
                self.call_event(actor, EVENT_STOLE, &id.to_le_bytes());
            }
        }
    }

    fn place_dropped(&mut self, id: u16, x: i32, y: i32) -> bool {
        if self.map().get(id).is_none() {
            return false;
        }

        // onto an open container window, or a container shown in one
        if let Some((gi, hit)) = self.pick_gump(x, y) {
            let owner = self.ui.gumps[gi].owner;
            let is_container = |g: &Game, i: u16| {
                g.map().get(i).and_then(|t| g.data.infos.get(t.shape as usize)).is_some_and(|si| si.family == FAMILY_CONTAINER)
            };
            if self.grid_window(owner) {
                return self.drop_in_grid(id, gi, hit, x, y);
            }
            return match hit {
                Some(target) if self.merge_stacks(id, target) => true,
                Some(target) if target != id && is_container(self, target) => {
                    let (gx, gy) = (8 + (self.rng.next() % 40) as i32, 8 + (self.rng.next() % 30) as i32);
                    self.put_in_container(id, target, gx, gy)
                }
                _ if owner == AVATAR => self.drop_on_avatar(id),
                _ => {
                    let g = &self.ui.gumps[gi];
                    let area = self.data.gump_areas.get(g.shape.saturating_sub(1) as usize).copied().unwrap_or([0, 0, 64, 64]);
                    let (gx, gy) = (x - g.x - area[0], y - g.y - area[1]);
                    let (w, h) = ((area[2] - area[0]).max(1), (area[3] - area[1]).max(1));
                    self.put_in_container(id, owner, gx.clamp(0, w), gy.clamp(0, h))
                }
            };
        }

        // onto something in the world
        let target = self.pick_world(x, y, id).and_then(|t| self.map().get(t).copied());
        if let Some(t) = target {
            if t.id == AVATAR {
                // dropping on the Avatar: wear it, or stow it in the backpack
                return self.drop_on_avatar(id);
            }
            if self.merge_stacks(id, t.id) {
                return true;
            }
        }
        // (option: a reagent onto the candle of a pentacle goes in front of it)
        let onto_candle = target.and_then(|t| self.snap_onto_candle(id, &t)).filter(|&(wx, wy, z)| self.within_reach(id, wx, wy, z));
        let spot = match onto_candle {
            Some((wx, wy, z)) => Some(DropSpot::Here(wx, wy, z)),
            None => self.drop_spot(id, x, y, target.as_ref()),
        };
        match spot {
            Some(DropSpot::Here(wx, wy, z)) => {
                // (option: onto the right spot of a pentacle; reagents lying
                // on a candle's point make room, then go in front of it, and
                // so do the reagents in front of the candle where it stood)
                let mut ids = self.reagents_on_point(id, wx, wy, z);
                let along = self.reagents_of_candle(id);
                ids.extend(along.iter().filter(|r| !ids.contains(r)).collect::<Vec<_>>());
                let in_the_way: Vec<(usize, (i32, i32, i32))> = ids
                    .into_iter()
                    .filter_map(|r| self.map().index(r))
                    .map(|i| (i, (self.map().items[i].x, self.map().items[i].y, self.map().items[i].z)))
                    .collect();
                for &(i, (rx, ry, rz)) in &in_the_way {
                    self.map_mut().place(i, rx, ry, rz + 256);
                }
                let snapped = self.pentagram_snap(id, wx, wy, z, target.map(|t| t.id));
                let (wx, wy, z) = snapped.unwrap_or((wx, wy, z));
                let Some(idx) = self.map().index(id) else { return false };
                self.close_gump(id);
                self.take_off(id);
                let map = self.map_mut();
                map.place(idx, wx, wy, z);
                map.items[idx].flags &= !FLG_CONTAINED;
                map.items[idx].fast = false; // let the fast area pick it up again
                for (i, old) in in_the_way {
                    let r = self.map().items[i].id;
                    // (those of the candle only when it went onto a point)
                    let go = snapped.is_some() || !along.contains(&r);
                    let (rx, ry, rz) = go.then(|| self.pentagram_snap(r, wx, wy, z, Some(id))).flatten().unwrap_or(old);
                    self.map_mut().place(i, rx, ry, rz);
                }
                self.path_landing(id);
                true
            }
            Some(DropSpot::Throw(wx, wy, z)) => {
                self.throw_item(id, (wx, wy, z));
                true
            }
            None => false,
        }
    }

    /// Where a dragged item would go if let go at a view position over the
    /// world: put down within reach, thrown farther away (as in the
    /// original: as far as the Avatar's strength carries its weight), or
    /// nowhere. `target`: what is under the pointer.
    fn drop_spot(&self, id: u16, x: i32, y: i32, target: Option<&Item>) -> Option<DropSpot> {
        let it = self.map().get(id).copied()?;
        // (option: aimed where the invisible path over the lava would be
        // seen, it goes onto the path)
        let on_path = target.and_then(|t| self.path_tile_at(x, y, t.id)).and_then(|p| self.map().get(p).copied());
        let target = on_path.as_ref().or(target);
        // on top of what is under the pointer, or at the Avatar's feet level
        let z_top = match target {
            Some(t) => t.z + self.data.infos.get(t.shape as usize).map_or(0, |si| si.z * 8),
            None => self.avatar().z,
        };
        let si = self.data.infos.get(it.shape as usize).copied()?;
        let (xd, yd, zd) = si.footpad(false);
        let nowhere = WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        let throw_range = self.throw_range(id);
        let mut thrown = None;
        // the pointer marks the middle of the item's footprint
        for lift in [0, 8, 16, -8, 24] {
            let z = (z_top + lift).max(0);
            let (px, py) = self.to_world(x, y, z);
            let (wx, wy) = (px + xd / 2, py + yd / 2);
            let reach = self.within_reach(id, wx, wy, z);
            if !reach && !self.within_throw(id, wx, wy, z, throw_range) {
                continue;
            }
            let b = WorldBox { x: wx, y: wy, z, xd, yd, zd };
            // (lower than what is under the pointer only where nothing
            // carries at its height: a flat floor has no height, and below
            // it there is room)
            let under_floor = lift < 0 && self.map().position_info(&WorldBox { z: z_top, ..b }, &nowhere, &self.data.infos, id).supported;
            if under_floor || !self.map().position_info(&b, &nowhere, &self.data.infos, id).valid {
                continue;
            }
            if !reach {
                // (a place within reach at another height is better)
                thrown = thrown.or(Some(DropSpot::Throw(wx, wy, z)));
                continue;
            }
            // let it settle on whatever is below
            let mut z = z;
            while z > 0 {
                let here = WorldBox { z, ..b };
                if self.map().position_info(&here, &nowhere, &self.data.infos, id).supported {
                    break;
                }
                let below = WorldBox { z: z - 1, ..b };
                if !self.map().position_info(&below, &nowhere, &self.data.infos, id).valid {
                    break;
                }
                z -= 1;
            }
            // (option: aimed at the lava under the invisible path, it goes
            // onto the path, not in under it)
            let (_, _, on_path) = self.throw_target((wx, wy, z));
            if on_path != z && self.within_reach(id, wx, wy, on_path)
                && self.map().position_info(&WorldBox { z: on_path, ..b }, &nowhere, &self.data.infos, id).valid {
                return Some(DropSpot::Here(wx, wy, on_path));
            }
            return Some(DropSpot::Here(wx, wy, z));
        }
        thrown
    }

    /// How far the Avatar can throw an item (0: not at all). As in the
    /// original: the lighter the item and the stronger he is, the farther.
    fn throw_range(&self, id: u16) -> i32 {
        let speed = self.throw_speed(id);
        speed * speed / 2
    }

    fn throw_speed(&self, id: u16) -> i32 {
        let strength = self.fights.states.get(&AVATAR).map_or(15, |s| s.strength);
        (64 - self.weight(id) + strength).max(1)
    }

    fn within_throw(&self, id: u16, x: i32, y: i32, z: i32, range: i32) -> bool {
        let av = self.avatar();
        range > 0 && self.gap_at(id, x, y, z).is_some_and(|gap| gap <= range) && (z - av.z).abs() <= range.min(256)
    }

    /// The Avatar throws an item at a point: it flies from his hands, not
    /// quite where it was aimed (the less dexterous, the farther off).
    fn throw_item(&mut self, id: u16, to: (i32, i32, i32)) {
        let av = self.avatar();
        let Some(idx) = self.map().index(id) else { return };
        self.close_gump(id);
        self.take_off(id);
        let speed = self.throw_speed(id);
        {
            let map = self.map_mut();
            map.place(idx, av.x, av.y, av.z + 24);
            map.items[idx].flags &= !FLG_CONTAINED;
            map.items[idx].fast = false;
        }
        let dex = self.fights.states.get(&AVATAR).map_or(15, |s| s.dex);
        let spread = (4 * (30 - dex)).max(20);
        let mut off = || (self.rng.next() % (2 * spread as u32 + 1)) as i32 - spread;
        let (dx, dy) = (off(), off());
        // he turns towards where it goes
        let dir = crate::intrinsics::world_dir(to.0 - av.x, to.1 - av.y) as u8;
        self.mover(AVATAR).dir = dir;
        self.animate(AVATAR, crate::anim::STAND);
        let target = self.throw_target((to.0 + dx, to.1 + dy, to.2));
        self.shoot(id, target, speed, 4);
    }

    /// For the pointer: what letting go here would do with the dragged item.
    pub fn drag_feedback(&mut self, x: i32, y: i32) -> DragFeedback {
        let Some(id) = self.ui.drag.as_ref().map(|d| d.item) else { return DragFeedback::None };
        if self.pick_gump(x, y).is_some() {
            return DragFeedback::Drop;
        }
        let target = self.pick_world(x, y, id).and_then(|t| self.map().get(t).copied());
        if target.is_some_and(|t| t.id == AVATAR) {
            return DragFeedback::Drop;
        }
        if target.and_then(|t| self.snap_onto_candle(id, &t)).is_some_and(|(wx, wy, z)| self.within_reach(id, wx, wy, z)) {
            return DragFeedback::Drop;
        }
        match self.drop_spot(id, x, y, target.as_ref()) {
            Some(DropSpot::Here(..)) => DragFeedback::Drop,
            Some(DropSpot::Throw(..)) => DragFeedback::Throw,
            None => DragFeedback::Nothing,
        }
    }

    /// Dropping onto a slot window (option "better inventory"): onto a
    /// stack of the same kind, into a container in a slot, onto a slot
    /// (what is there swaps places), or onto a page arrow (the first free
    /// slot of the next / previous page).
    fn drop_in_grid(&mut self, id: u16, gi: usize, hit: Option<u16>, x: i32, y: i32) -> bool {
        use crate::inventory::GridHit;
        let owner = self.ui.gumps[gi].owner;
        let is_container = |g: &Game, i: u16| {
            g.map().get(i).and_then(|t| g.data.infos.get(t.shape as usize)).is_some_and(|si| si.family == FAMILY_CONTAINER)
        };
        match hit {
            Some(target) if self.merge_stacks(id, target) => return true,
            Some(target) if target != id && is_container(self, target) => {
                // (somewhere in its original window, as the original does it)
                let (gx, gy) = (8 + (self.rng.next() % 40) as i32, 8 + (self.rng.next() % 30) as i32);
                return self.put_in_container(id, target, gx, gy);
            }
            _ => {}
        }
        let page = self.shown_page(owner);
        let came_from = self.grid_slot_of(id, owner);
        let want = match self.grid_hit(gi, x, y) {
            Some(GridHit::Slot(s)) => Some(s),
            Some(GridHit::Next) => Some(self.grid_free_from(owner, page + 1)),
            Some(GridHit::Back) if page > 0 => Some(self.grid_free_from(owner, page - 1)),
            _ => None,
        };
        // onto a free place: coins and the like join their stack instead
        if self.grid_slot_of(id, owner).is_none() && self.join_stack(id, owner) {
            return true;
        }
        if !self.put_in_container(id, owner, 0, 0) {
            return false;
        }
        self.grid_place(id, owner, want, came_from);
        if let Some(s) = want {
            // the window shows where it went
            self.ui.gump_pages.insert(owner, s / crate::inventory::PER_PAGE);
        }
        true
    }

    pub fn draw_drag(&mut self, surf: &mut Surface) {
        let Some((id, x, y)) = self.ui.drag.as_ref().map(|d| (d.item, d.x, d.y)) else { return };
        let Some(it) = self.map().get(id).copied() else {
            self.ui.drag = None;
            return;
        };
        if let Some(shape) = self.data.shapes.get(it.shape) {
            if let Some(f) = shape.frames.get(it.frame as usize) {
                // centred on the pointer
                surf.paint(f, x - f.width / 2 + f.xoff, y - f.height / 2 + f.yoff, false, false, &self.data.pal);
            }
        }
    }
}
