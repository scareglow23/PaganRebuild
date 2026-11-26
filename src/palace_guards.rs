//! An experiment (option "palace_guards", off unless switched on): the two
//! guards beside the double door in Mordea's palace in Tenebrae (map 40),
//! behind which lie the throne room and her chambers, stop the Avatar
//! while they see him. A double click on the door does not open it, and
//! he cannot step through it from the hall when it stands open; the guard
//! turns to him and says so. Invisible (the black potion), he slips past,
//! unless he brushes right past a guard (as creatures in a fight see him,
//! combat.rs). Scenes that lead him are not stopped.
//!
//! This makes the quest for the ceremonial dagger harder than in the
//! original: the way to the key under the pillow and into her bedroom
//! needs the Avatar invisible (the player's wish).

use crate::game::{Game, AVATAR};
use crate::intrinsics::world_dir;
use crate::map::{Item, FLG_INVISIBLE};

/// Tenebrae.
const MAP: usize = 40;
/// A wing of the double door while it is shut (open it is 68): only
/// opening is stopped, not shutting. The wings are known by their place,
/// not their numbers (the starting data of the German, French and English
/// game differ).
const SHUT_DOOR: u32 = 69;
/// A guard of the palace; those at the doorway (each about 130 from it,
/// npc 50 and 51 in the English game) watch it.
const GUARD_SHAPE: u32 = 574;
const POST_REACH: i32 = 320;
/// The doorway: from x0 to x1 (the two wings), the front of the doors (on
/// the side of the hall, y; the throne room lies at smaller y), its height.
const DOOR_X0: i32 = 12927;
const DOOR_X1: i32 = 13183;
const DOOR_FRONT: i32 = 9215;
const DOOR_Z: i32 = 48;
/// How far a guard sees the Avatar (the doorway lies between the two,
/// about 130 from each).
const SIGHT: i32 = 400;
/// Ticks between two warnings while the Avatar keeps trying.
const WARN_AGAIN: u32 = 60;

/// What the guards remember.
#[derive(Default)]
pub struct PalaceGuards {
    /// The Avatar was held back at the doorway in this tick (the warning
    /// follows once the actors have moved).
    pub(crate) barred: bool,
    /// When a guard last warned him.
    warned_at: Option<u32>,
}

impl Game {
    /// Does the rule apply now: switched on, in Tenebrae, no scene leading
    /// the Avatar?
    fn palace_rule(&self) -> bool {
        self.options.palace_guards && self.cur_map == MAP && !self.stasis
    }

    /// The guard that sees the Avatar (at `at`), if one does.
    fn guard_watching(&self, at: (i32, i32, i32)) -> Option<u16> {
        let mut av: Item = *self.map().get(AVATAR)?;
        (av.x, av.y, av.z) = at;
        let (mx, my) = ((DOOR_X0 + DOOR_X1) / 2, DOOR_FRONT);
        let near_door = self.map().items_near(mx, my, POST_REACH + 64);
        near_door.into_iter().map(|i| self.map().items[i]).filter(|it| it.is_actor() && it.shape == GUARD_SHAPE && (it.x - mx).abs().max((it.y - my).abs()) <= POST_REACH).map(|it| it.id).find(|&g| {
            let Some(it) = self.map().get(g).copied() else { return false };
            let dead = self.fights.states.get(&g).is_some_and(|s| s.dead);
            let gap = self.box_gap(&it, &av);
            let close = gap <= 20 && (av.z - it.z).abs() < 48;
            it.in_world() && !dead && gap <= SIGHT && (av.z - it.z).abs() < 48 && (av.flags & FLG_INVISIBLE == 0 || close) && self.clear_between(&it, &av)
        })
    }

    /// A double click on the door (before its script runs): true if a
    /// guard stops it.
    pub(crate) fn guards_keep_door(&mut self, id: u16) -> bool {
        let door = self.map().get(id).is_some_and(|d| d.in_world() && d.shape == SHUT_DOOR && d.y == DOOR_FRONT && (DOOR_X0..=DOOR_X1).contains(&d.x) && d.z == DOOR_Z);
        if !door || !self.palace_rule() {
            return false;
        }
        let av = self.avatar();
        let Some(g) = self.guard_watching((av.x, av.y, av.z)) else { return false };
        self.guard_warns(g, true);
        true
    }

    /// Would the Avatar, going from `from` to `to`, step into the doorway
    /// from the hall while a guard sees him?
    pub(crate) fn guards_bar_step(&self, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        if !self.palace_rule() {
            return false;
        }
        let Some(si) = self.map().get(AVATAR).and_then(|av| self.data.infos.get(av.shape as usize)) else { return false };
        let (xd, yd) = (si.x * 32, si.y * 32);
        // (his footprint reaches from x - xd to x and from y - yd to y)
        let in_front = |p: (i32, i32, i32)| p.1 - yd >= DOOR_FRONT;
        let across = to.0 > DOOR_X0 && to.0 - xd < DOOR_X1 && (to.2 - DOOR_Z).abs() < 24;
        across && in_front(from) && !in_front(to) && self.guard_watching(from).is_some()
    }

    /// After the actors have moved: a guard warns the Avatar he held back
    /// at the doorway, who stops walking.
    pub(crate) fn tick_palace_guards(&mut self) {
        if !std::mem::take(&mut self.palace.barred) {
            return;
        }
        self.cancel_approach();
        let av = self.avatar();
        self.stop_walk(AVATAR, &av, (0, 0));
        if let Some(g) = self.guard_watching((av.x, av.y, av.z)) {
            self.guard_warns(g, false);
        }
    }

    /// The guard turns to the Avatar and tells him he may not pass (not
    /// again and again while he keeps pushing, unless he asked by using
    /// the door).
    fn guard_warns(&mut self, g: u16, asked: bool) {
        let tick = self.tick;
        if !asked && self.palace.warned_at.is_some_and(|t| tick.wrapping_sub(t) < WARN_AGAIN) {
            return;
        }
        self.palace.warned_at = Some(tick);
        let av = self.avatar();
        if let Some(it) = self.map().get(g).copied() {
            self.mover(g);
            if let Some(mover) = self.actors.get_mut(&g).filter(|m| !m.busy()) {
                mover.dir = world_dir(av.x - it.x, av.y - it.y) as u8;
                mover.play(crate::anim::STAND, &it, &mut self.data.anims);
            }
        }
        let text = self.tr(["Halt! Hier ist kein Durchgang.", "Halte ! On ne passe pas.", "Halt! There is no passage here."]);
        self.bark(g, text.to_string());
    }
}
