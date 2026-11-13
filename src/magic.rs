//! What the spell scripts need from the engine: picking a target,
//! entering an amount, effect sprites, explosions, weights and a few
//! special effects.  The spells themselves are usecode.

use crate::game::{Game, AVATAR};
use crate::intrinsics::world_dir;
use crate::map::{Item, WorldBox};
use crate::menu::{draw_stud, SLIDER_SHAPE, TRACK_X0, TRACK_X1, TRACK_Y, VALUE_X};
use crate::view::Surface;
use crate::vm::Kind;

/// The golem the earth spell makes.
pub const GOLEM_SHAPE: u32 = 214;

/// An effect animation (flames, sparkles ...) playing in the world.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Sprite {
    pub item: u16,
    frame: u32,
    first: u32,
    last: u32,
    repeats: i32,
    delay: u32,
    counter: u32,
}

/// The amount the player is asked to enter with the slider.
pub struct SliderInput {
    purpose: Purpose,
    min: i32,
    max: i32,
    step: i32,
    pub value: i32,
}

/// Who wants the amount.
enum Purpose {
    /// A script that sleeps until it is entered (its process).
    Script(u16),
    /// The player drops part of a stack at a view position.
    Split { item: u16, x: i32, y: i32 },
}

/// The OK button of the slider window (graphic and place), and the arrows
/// painted into the window (x, y, width, height relative to its corner).
const SLIDER_OK_SHAPE: u32 = 42;
const SLIDER_OK: (i32, i32, i32, i32) = (158, 17, 21, 11);
const SLIDER_LEFT: (i32, i32, i32, i32) = (36, 17, 19, 12);
const SLIDER_RIGHT: (i32, i32, i32, i32) = (139, 17, 18, 12);

const DMG_FIRE: u16 = 0x0008;
const DMG_BLUNT: u16 = 0x0004;

impl Game {
    // ---- picking a target ------------------------------------------------

    /// A script asks the player to click on something. The returned
    /// process ends with the object id as its result (0 = nothing).
    pub fn start_targeting(&mut self) -> u32 {
        if let Some(old) = self.ui.targeting.take() {
            self.kill_pid(old);
        }
        let pid = self.add_native(Kind::Ask, 0, 0);
        self.ui.targeting = Some(pid);
        pid as u32
    }

    /// The player clicked on `id` (0: cancelled).
    pub fn target_picked(&mut self, id: u16) {
        let Some(pid) = self.ui.targeting.take() else { return };
        let id = if self.wrong_piece_for_titan(id) { 0 } else { id };
        self.end_process_with(pid, id as u32);
        if self.ui.peace_target.take() == Some(pid) && id != 0 {
            self.grant_peace_on(id);
        }
    }

    /// A Blackrock piece used on Hydros must be the Tear of the Seas, as
    /// the other Titans each take only their own piece. Hydros alone took
    /// any of them, and the wrong one, charged at him, left the Tear
    /// uncharged for good, so that the game could no longer be won. Such
    /// a pick counts as no target: nothing happens, as with the other
    /// Titans.
    fn wrong_piece_for_titan(&self, target: u16) -> bool {
        const HYDROS: u32 = 80;
        const BLACKROCK: u32 = 442;
        const TEAR_OF_THE_SEAS: u32 = 2;
        if self.map().get(target).is_none_or(|t| t.shape != HYDROS) {
            return false;
        }
        self.scripts.procs.iter().filter(|p| !p.terminated && p.class as u32 == BLACKROCK).any(|p| {
            self.map().get(p.item).is_some_and(|it| it.shape == BLACKROCK && it.frame != TEAR_OF_THE_SEAS)
        })
    }

    // ---- entering an amount ------------------------------------------------

    /// A script asks for a number; it sleeps until the player confirms.
    pub fn start_slider(&mut self, caller: usize, min: i32, max: i32, step: i32) {
        let (min, max) = (min.min(max), max.max(min));
        self.scripts.procs[caller].suspended = true;
        self.ui.slider = Some(SliderInput { purpose: Purpose::Script(self.scripts.procs[caller].pid), min, max, step: step.max(1), value: min });
    }

    /// Asks how much of a stack to put down at a view position.
    pub fn start_split(&mut self, item: u16, amount: i32, x: i32, y: i32) {
        self.ui.slider = Some(SliderInput { purpose: Purpose::Split { item, x, y }, min: 1, max: amount, step: 1, value: amount });
    }

    pub fn slider_change(&mut self, steps: i32) {
        if let Some(s) = &mut self.ui.slider {
            s.value = (s.value + steps * s.step).clamp(s.min, s.max);
        }
    }

    fn slider_pos(&mut self, surf_w: i32, surf_h: i32) -> (i32, i32) {
        let (w, h) = self.data.gump_shapes.get(SLIDER_SHAPE).and_then(|s| s.frames.first().map(|f| (f.width, f.height))).unwrap_or((194, 47));
        ((surf_w - w) / 2, (surf_h - h) / 2)
    }

    /// Moves the knob to a mouse position if the pointer is on the groove.
    pub fn slider_mouse(&mut self, x: i32, y: i32, surf_w: i32, surf_h: i32) {
        let (gx, gy) = self.slider_pos(surf_w, surf_h);
        let Some(s) = &mut self.ui.slider else { return };
        if y < gy + TRACK_Y - 12 || y > gy + TRACK_Y + 12 || x < gx + TRACK_X0 - 12 || x > gx + TRACK_X1 + 12 {
            return;
        }
        let share = ((x - gx - TRACK_X0) as f32 / (TRACK_X1 - TRACK_X0) as f32).clamp(0.0, 1.0);
        let raw = s.min as f32 + share * (s.max - s.min) as f32;
        s.value = (s.min + ((raw - s.min as f32) / s.step as f32).round() as i32 * s.step).clamp(s.min, s.max);
    }

    /// A click on the slider window: OK confirms, the arrows step by one,
    /// the groove moves the knob. True if it was the groove (the knob then
    /// follows the held button).
    pub fn slider_click(&mut self, x: i32, y: i32, surf_w: i32, surf_h: i32) -> bool {
        let (gx, gy) = self.slider_pos(surf_w, surf_h);
        let on = |(bx, by, bw, bh): (i32, i32, i32, i32)| x >= gx + bx && x < gx + bx + bw && y >= gy + by && y < gy + by + bh;
        if on(SLIDER_OK) {
            self.slider_confirm();
        } else if on(SLIDER_LEFT) {
            self.slider_change(-1);
        } else if on(SLIDER_RIGHT) {
            self.slider_change(1);
        } else {
            self.slider_mouse(x, y, surf_w, surf_h);
            return y >= gy + TRACK_Y - 12 && y <= gy + TRACK_Y + 12 && x >= gx + TRACK_X0 - 12 && x <= gx + TRACK_X1 + 12;
        }
        false
    }

    /// The player confirms the amount; the waiting script gets it.
    pub fn slider_confirm(&mut self) {
        let Some(s) = self.ui.slider.take() else { return };
        match s.purpose {
            Purpose::Script(pid) => self.set_return_and_wake(pid, s.value as u32),
            Purpose::Split { item, x, y } => self.drop_part(item, s.value, x, y),
        }
    }

    pub fn draw_slider(&mut self, surf: &mut Surface) {
        let Some((min, max, value)) = self.ui.slider.as_ref().map(|s| (s.min, s.max, s.value)) else { return };
        let (gx, gy) = self.slider_pos(surf.width, surf.height);
        if let Some(shape) = self.data.gump_shapes.get(SLIDER_SHAPE) {
            if let Some(f) = shape.frames.first() {
                surf.paint(f, gx + f.xoff, gy + f.yoff, false, false, &self.data.pal);
            }
        }
        if let Some(shape) = self.data.gump_shapes.get(SLIDER_OK_SHAPE) {
            if let Some(f) = shape.frames.first() {
                surf.paint(f, gx + SLIDER_OK.0 + f.xoff, gy + SLIDER_OK.1 + f.yoff, false, false, &self.data.pal);
            }
        }
        let span = (max - min).max(1);
        draw_stud(surf, gx + TRACK_X0 + (TRACK_X1 - TRACK_X0) * (value - min) / span, gy + TRACK_Y, true, true);
        if let Some(font) = &self.data.font {
            let v = value.to_string();
            font.draw(surf, &v, gx + VALUE_X - font.width(&v) / 2, gy + TRACK_Y - font.height / 2, &self.data.pal);
        }
    }

    // ---- effect sprites and explosions ---------------------------------------

    /// Starts an effect animation at a place: the frames `first..=last`
    /// of a shape, `repeats` times, each frame shown for `delay` ticks.
    pub fn create_sprite(&mut self, shape: u32, first: u32, last: u32, repeats: i32, delay: u32, pos: (i32, i32, i32)) -> u32 {
        let cur = self.cur_map as u16;
        let id = self.map_mut().add(Item {
            x: pos.0,
            y: pos.1,
            z: pos.2,
            shape,
            frame: first,
            mapnum: cur,
            flags: crate::eggs::FLG_FAST_ONLY | crate::eggs::FLG_DISPOSABLE,
            dead: true, // an effect is never in the way
            fast: true,
            ..Item::default()
        });
        if let Some(idx) = self.map().index(id) {
            self.fast_items.push(idx);
        }
        let sprite = Sprite { item: id, frame: first, first, last, repeats: repeats.max(1), delay: delay.max(1), counter: 0 };
        self.add_native(Kind::Sprite(sprite), 0, 0) as u32
    }

    /// One tick of an effect. Returns true when it is over.
    pub fn run_sprite(&mut self, s: &mut Sprite) -> bool {
        let Some(idx) = self.map().index(s.item) else { return true };
        if s.frame > s.last && s.repeats <= 1 {
            self.destroy_with_scripts(idx);
            return true;
        }
        if s.counter == 0 {
            if s.frame > s.last {
                s.frame = s.first;
                s.repeats -= 1;
            }
            self.map_mut().items[idx].frame = s.frame;
            s.frame += 1;
        }
        s.counter = (s.counter + 1) % s.delay;
        false
    }

    /// An item blows up: flash, bang, and damage to whoever stands close.
    pub fn explode(&mut self, id: u16) {
        let Some(it) = self.map().get(id).copied().filter(|it| it.in_world()) else { return };
        self.create_sprite(578, 20, 34, 1, 1, (it.x, it.y, it.z));
        let sfx = if self.rng.next() & 1 == 0 { 31 } else { 158 };
        let volume = self.sfx_volume(id);
        self.audio.play_sfx(sfx, 0, false, volume);
        if let Some(idx) = self.map().index(id) {
            self.destroy_with_scripts(idx);
        }
        let victims: Vec<Item> = self.map().items_near(it.x, it.y, 160).into_iter().map(|i| self.map().items[i]).filter(|v| v.is_actor() && !v.dead).collect();
        for v in victims {
            let dir = (world_dir(v.x - it.x, v.y - it.y) as u8 + 4) & 7;
            let damage = 6 + (self.rng.next() % 6) as i32;
            self.receive_hit(v.id, 0, dir, damage, DMG_FIRE | DMG_BLUNT);
        }
    }

    // ---- gravity ------------------------------------------------------------------

    /// An item drops if nothing holds it. Things fixed in place or hung
    /// on a wall stay.
    pub fn fall(&mut self, id: u16) {
        const FLG_HANGING: u16 = 0x1000;
        let Some(it) = self.map().get(id).copied().filter(|it| it.in_world()) else { return };
        let fixed = self.data.infos.get(it.shape as usize).is_none_or(|si| si.fixed);
        if fixed || it.flags & FLG_HANGING != 0 || it.is_actor() {
            return;
        }
        let already = self.scripts.procs.iter().any(|p| !p.terminated && matches!(p.kind, Kind::Hurl(h) if h.item == id));
        if !already {
            self.add_native(Kind::Hurl(crate::effects::Hurl { item: id, speed: (0, 0, 0), gravity: 4 }), id, crate::effects::PROC_HURL);
        }
    }

    /// An item is taken from its place: what lay on top of it comes down.
    pub fn grab(&mut self, id: u16) {
        let Some(base) = self.map().get(id).copied().filter(|it| it.in_world()) else { return };
        let Some(bi) = self.data.infos.get(base.shape as usize) else { return };
        let bb = WorldBox::of(&base, bi);
        let on_top: Vec<u16> = self
            .map()
            .items_near(base.x, base.y, 512)
            .into_iter()
            .map(|i| &self.map().items[i])
            .filter(|it| it.id != id && it.id != 0)
            .filter(|it| self.data.infos.get(it.shape as usize).is_some_and(|si| {
                let ib = WorldBox::of(it, si);
                ib.z == bb.z + bb.zd && ib.overlaps_xy(&bb)
            }))
            .map(|it| it.id)
            .collect();
        for other in on_top {
            self.fall(other);
        }
    }

    /// The item that holds `id` in the world (itself, if it lies there).
    pub fn top_item(&self, mut id: u16) -> Option<Item> {
        for _ in 0..16 {
            let it = self.map().get(id).copied()?;
            if it.in_world() {
                return Some(it);
            }
            id = it.parent;
        }
        None
    }

    /// canReach: are the two items within `range` of each other?
    pub fn can_reach(&self, id: u16, other: u16, range: i32) -> bool {
        let (Some(a), Some(b)) = (self.top_item(id), self.top_item(other)) else { return false };
        let (Some(ai), Some(bi)) = (self.data.infos.get(a.shape as usize), self.data.infos.get(b.shape as usize)) else { return false };
        let (ab, bb) = (WorldBox::of(&a, ai), WorldBox::of(&b, bi));
        !(ab.x - ab.xd - bb.x > range || bb.x - bb.xd - ab.x > range || ab.y - ab.yd - bb.y > range || bb.y - bb.yd - ab.y > range)
    }

    /// ascend: a platform moves up or down, with what stands on it.
    /// True if it got all the way.
    pub fn ascend(&mut self, id: u16, delta: i32) -> bool {
        if delta == 0 {
            return true;
        }
        let Some(base) = self.map().get(id).copied().filter(|it| it.in_world()) else { return false };
        let Some(bi) = self.data.infos.get(base.shape as usize) else { return false };
        let bb = WorldBox::of(&base, bi);
        // what rests on it (and on that, and so on)
        let mut riders: Vec<u16> = Vec::new();
        let mut tops = vec![bb];
        let near = self.map().items_near(base.x, base.y, 512);
        while let Some(t) = tops.pop() {
            for &i in &near {
                let it = &self.map().items[i];
                if it.id == id || it.id == 0 || it.gone || riders.contains(&it.id) {
                    continue;
                }
                let Some(si) = self.data.infos.get(it.shape as usize) else { continue };
                let ib = WorldBox::of(it, si);
                if !si.fixed && ib.z == t.z + t.zd && ib.overlaps_xy(&t) {
                    riders.push(it.id);
                    tops.push(ib);
                }
            }
        }
        for rid in std::iter::once(id).chain(riders) {
            if let Some(idx) = self.map().index(rid) {
                let it = self.map().items[idx];
                self.map_mut().place(idx, it.x, it.y, it.z + delta);
            }
        }
        true
    }

    /// legal_move into a container: true if it fits.
    pub fn move_into(&mut self, id: u16, container: u16) -> bool {
        if self.map().get(container).is_none() || self.contains(id, container) || self.can_add(container, id).is_err() {
            return false;
        }
        self.put_in_container(id, container, 20, 20)
    }

    // ---- weights ----------------------------------------------------------------

    /// Does the item come in amounts (coins, arrows, reagents)? With the
    /// option "better inventory" potions do as well (not in the original;
    /// see potions.rs).
    pub fn stackable(&self, it: &Item) -> bool {
        (it.shape == crate::potions::POTION && self.options.better_inventory)
            || self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == 2 || si.family == 9)
    }

    /// How many things an item stands for (a single potion has 0 as its amount).
    pub fn amount(&self, it: &Item) -> u32 {
        if it.shape == crate::potions::POTION { (it.quality as u32).max(1) } else { it.quality as u32 }
    }

    /// Room an item takes up in a container.
    pub fn volume(&self, id: u16) -> i32 {
        let Some(it) = self.map().get(id) else { return 0 };
        let Some(si) = self.data.infos.get(it.shape as usize) else { return 0 };
        let v = si.raw[7] as i32;
        match si.family {
            2 => (it.quality as i32 * v + 99) / 100,
            9 => (it.quality as i32 * v + 9) / 10,
            6 => if v == 0 { 32 } else { v }, // a container takes up what it can hold
            _ if it.shape == crate::potions::POTION => v * (it.quality as i32).max(1),
            _ => v,
        }
    }

    /// May the item go into the container? Err says why not.
    pub fn can_add(&self, container: u16, id: u16) -> Result<(), &'static str> {
        let Some(c) = self.map().get(container) else { return Err("gone") };
        // what the Avatar carries is limited by his strength (told first:
        // too heavy says more than too large)
        let carried_by = |g: &Game, mut cur: u16| {
            for _ in 0..16 {
                match g.map().get(cur) {
                    Some(i) if i.parent != crate::map::IN_WORLD && i.parent != crate::map::ETHEREAL => cur = i.parent,
                    _ => break,
                }
            }
            cur
        };
        if carried_by(self, container) == AVATAR && carried_by(self, id) != AVATAR {
            let strength = self.fights.states.get(&AVATAR).map_or(15, |s| s.strength);
            if self.carried_weight() + self.weight(id) > 40 * strength {
                return Err("heavy");
            }
        }
        // bodies and the Avatar himself hold whatever is put there
        if !c.is_actor() {
            let capacity = self.data.infos.get(c.shape as usize).map_or(32, |si| if si.raw[7] == 0 { 32 } else { si.raw[7] as i32 });
            let used: i32 = self.map().items.iter().filter(|i| !i.gone && i.parent == container && i.id != id && i.id != 0).map(|i| self.volume(i.id)).sum();
            if used + self.volume(id) > capacity {
                return Err("full");
            }
        }
        Ok(())
    }

    /// Weight of everything the Avatar wears and carries.
    pub fn carried_weight(&self) -> i32 {
        self.map().items.iter().filter(|c| !c.gone && c.parent == AVATAR && c.id != 0).map(|c| self.weight(c.id)).sum()
    }

    /// Tells the player why something does not fit.
    pub fn explain_refusal(&mut self, why: &str) {
        let text = match (why, self.language) {
            ("full", 'G') => "Dort ist kein Platz mehr.",
            ("full", 'F') => "Il n'y a plus de place.",
            ("full", _) => "There is no room left.",
            ("heavy", 'G') => "Das ist zu schwer.",
            ("heavy", 'F') => "C'est trop lourd.",
            ("heavy", _) => "That is too heavy.",
            _ => return,
        };
        self.notify(text);
    }

    /// Weight of an item with everything in it.
    pub fn weight(&self, id: u16) -> i32 {
        let Some(it) = self.map().get(id) else { return 0 };
        let Some(si) = self.data.infos.get(it.shape as usize) else { return 0 };
        let w = si.raw[6] as i32;
        let own = match si.family {
            2 => (it.quality as i32 * w + 9) / 10, // amounts: coins, arrows
            9 => it.quality as i32 * w,             // reagents
            _ if it.shape == crate::potions::POTION => w * 10 * (it.quality as i32).max(1),
            _ => w * 10,
        };
        own + self.map().items.iter().filter(|c| !c.gone && c.parent == id && c.id != 0).map(|c| self.weight(c.id)).sum::<i32>()
    }

    /// Weight of everything resting on top of an item (pressure plates).
    pub fn surface_weight(&self, id: u16) -> i32 {
        let Some(base) = self.map().get(id).filter(|it| it.in_world()) else { return 0 };
        let Some(bi) = self.data.infos.get(base.shape as usize) else { return 0 };
        let bb = WorldBox::of(base, bi);
        self.map()
            .items_near(base.x, base.y, 512)
            .into_iter()
            .map(|i| &self.map().items[i])
            .filter(|it| it.id != id && it.id != 0)
            .filter(|it| {
                self.data.infos.get(it.shape as usize).is_some_and(|si| {
                    let ib = WorldBox::of(it, si);
                    ib.z == bb.z + bb.zd && ib.overlaps_xy(&bb)
                })
            })
            .map(|it| if it.is_actor() { 300 } else { self.weight(it.id) })
            .sum()
    }

    /// The peace spell asks the player to click on a creature; the spell
    /// takes effect once it is picked (see `grant_peace_on`).
    pub fn grant_peace(&mut self) -> u32 {
        let pid = self.start_targeting();
        self.ui.peace_target = Some(pid as u16);
        0
    }

    /// The peace spell on the picked creature, as in ScummVM. Undead picked:
    /// every undead creature within 768 of the Avatar is laid to rest for
    /// good (a deadly hit of magic and fire, so they do not get up again).
    /// Anyone else picked: one chance in ten that it dies. Returns how many
    /// were hit.
    pub fn grant_peace_on(&mut self, target: u16) -> u32 {
        if target == AVATAR || !self.map().get(target).is_some_and(|it| it.is_actor() && it.in_world()) {
            return 0;
        }
        let deadly = crate::combat::DMG_MAGIC | crate::combat::DMG_PIERCE | crate::combat::DMG_FIRE;
        let mut hits = 0;
        if self.undead(target) {
            let av = self.avatar();
            // Khumash-Gor's stone throne (shape 289) within 2048: when his
            // ghost (shape 411) is laid to rest, it sparkles away and the
            // throne is empty from then on (frame 1)
            let throne = self.map().items_near(av.x, av.y, 2048).into_iter().map(|i| self.map().items[i]).find(|it| it.shape == 289).map(|it| it.id);
            let near: Vec<u16> = self.map().items_near(av.x, av.y, 768).into_iter().map(|i| self.map().items[i]).filter(|it| it.is_actor() && it.in_world() && it.id != AVATAR).map(|it| it.id).collect();
            for id in near {
                if self.state(id).dead || !self.undead(id) {
                    continue;
                }
                let hp = self.state(id).hp.max(1);
                self.receive_hit(id, 0, 0, hp, deadly);
                hits += 1;
                if let (Some(throne), Some(ghost)) = (throne, self.map().get(id).filter(|it| it.shape == 411).map(|it| (it.x, it.y, it.z))) {
                    self.create_sprite(480, 0, 9, 1, 1, ghost);
                    self.with_item_pub(throne, |it| it.frame = 1);
                }
            }
        } else {
            let s = self.state(target);
            if !s.dead && !s.immortal && self.rng.next().is_multiple_of(10) {
                let hp = s.hp.max(1);
                self.receive_hit(target, 0, 0, hp, deadly);
                hits += 1;
            }
        }
        if hits > 0 {
            // lightning and thunder
            self.start_fade((207, 207, 207), 200, 0, 10);
            let sfx = [91, 94, 96][(self.rng.next() % 3) as usize];
            let volume = self.sfx_volume(AVATAR);
            self.audio.play_sfx(sfx, 0, false, volume);
        }
        hits
    }
}
