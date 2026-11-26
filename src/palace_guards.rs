//! An experiment (option "palace_guards", off unless switched on): the two
//! guards beside the double door in Mordea's palace in Tenebrae (map 40),
//! behind which lie the throne room and her chambers, stop the Avatar
//! while they see him. A double click on the door does not open it, and
//! he cannot step through it from the hall when it stands open. As in the
//! scenes of the game (the guard at the town gate), the Avatar is held
//! (stasis) while the guard walks a few steps up to him, turns to him step
//! by step and says so; then the guard goes back to his post. Invisible (the black potion), he slips past,
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
/// A guard farther from the Avatar than this walks up to him first.
const STEP_UP: i32 = 48;
/// The longest the guard walks there, and back to his post (ticks).
const WALK_TICKS: u32 = 90;
const BACK_TICKS: u32 = 240;
/// Ticks per eighth of a turn (the turning of the scripts).
const TURN_TICKS: u32 = 5;
/// The Avatar is held at least this long while the guard speaks (and as
/// long as his words are shown).
const SPEAK_TICKS: u32 = 60;

/// The scene of a guard stopping the Avatar.
struct Scene {
    guard: u16,
    phase: Phase,
    /// When the phase began.
    since: u32,
    /// The walk or the speech waited for.
    pid: u16,
    /// The guard's post: where he stood and which way he looked.
    post: (i32, i32, i32),
    post_dir: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// He walks up to the Avatar.
    StepUp,
    /// He turns to him.
    Turn,
    /// He speaks (the Avatar is held until he has said it).
    Speak,
    /// He walks back to his post.
    Back,
    /// He turns as he stood.
    TurnBack,
}

/// What the guards remember.
#[derive(Default)]
pub struct PalaceGuards {
    /// The Avatar was held back at the doorway in this tick (the scene
    /// starts once the actors have moved).
    pub(crate) barred: bool,
    scene: Option<Scene>,
    /// Each guard's post, as he stood when he first stepped out (a walk
    /// back ends up to 32 short of it: he does not drift off further each
    /// time).
    posts: std::collections::HashMap<u16, ((i32, i32, i32), u8)>,
}

impl PalaceGuards {
    /// Does a guard hold the Avatar now (the stasis is the scene's own)?
    pub(crate) fn holding(&self) -> bool {
        self.scene.as_ref().is_some_and(|s| matches!(s.phase, Phase::StepUp | Phase::Turn | Phase::Speak))
    }
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
        self.guard_stops(g);
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

    /// After the actors have moved: the Avatar held back at the doorway
    /// is stopped by a guard; the scene of a guard goes on.
    pub(crate) fn tick_palace_guards(&mut self) {
        if std::mem::take(&mut self.palace.barred) {
            let av = self.avatar();
            if let Some(g) = self.guard_watching((av.x, av.y, av.z)) {
                self.guard_stops(g);
            }
        }
        self.tick_guard_scene();
    }

    /// Guard `g` stops the Avatar: the Avatar is held, turned to him, and
    /// the scene begins (anew, if he was on his way back to his post).
    fn guard_stops(&mut self, g: u16) {
        if self.palace.holding() {
            return;
        }
        let Some(it) = self.map().get(g).copied() else { return };
        self.palace.scene = None;
        let dir = self.mover(g).dir;
        let (post, post_dir) = *self.palace.posts.entry(g).or_insert(((it.x, it.y, it.z), dir));
        self.kill_processes(g, crate::effects::PROC_WALK);
        self.stasis = true;
        self.cancel_approach();
        let av = self.avatar();
        self.stop_walk(AVATAR, &av, (it.x - av.x, it.y - av.y));
        let (phase, pid) = if self.box_gap(&it, &av) > STEP_UP {
            let pid = self.add_native(crate::vm::Kind::Pathfind(crate::effects::Pathfind::new(g, AVATAR, (0, 0))), g, crate::effects::PROC_WALK);
            (Phase::StepUp, pid)
        } else {
            (Phase::Turn, 0)
        };
        self.palace.scene = Some(Scene { guard: g, phase, since: self.tick, pid, post, post_dir });
    }

    /// One tick of the scene.
    fn tick_guard_scene(&mut self) {
        let Some(scene) = self.palace.scene.as_ref() else { return };
        let (g, phase, pid, post, post_dir) = (scene.guard, scene.phase, scene.pid, scene.post, scene.post_dir);
        let ticks = self.tick.wrapping_sub(scene.since);
        let running = |game: &Game| game.scripts.procs.iter().any(|p| p.pid == pid && !p.terminated);
        // (the guard gone, fallen, or another map: the scene ends)
        let alive = self.cur_map == MAP && self.map().get(g).is_some_and(|it| it.in_world()) && !self.fights.states.get(&g).is_some_and(|s| s.dead);
        if !alive {
            if self.palace.holding() {
                self.stasis = false;
            }
            self.palace.scene = None;
            return;
        }
        let next = |game: &mut Game, phase: Phase, pid: u16| {
            let tick = game.tick;
            if let Some(s) = game.palace.scene.as_mut() {
                (s.phase, s.since, s.pid) = (phase, tick, pid);
            }
        };
        match phase {
            Phase::StepUp => {
                if !running(self) || ticks >= WALK_TICKS {
                    self.kill_processes(g, crate::effects::PROC_WALK);
                    let it = self.map().get(g).copied().unwrap_or_default();
                    self.stop_walk(g, &it, (0, 0));
                    next(self, Phase::Turn, 0);
                }
            }
            Phase::Turn => {
                let (av, it) = (self.avatar(), self.map().get(g).copied().unwrap_or_default());
                if self.turn_step(g, world_dir(av.x - it.x, av.y - it.y) as u8, ticks) {
                    let text = self.tr(["Halt! Hier ist kein Durchgang.", "Halte ! On ne passe pas.", "Halt! There is no passage here."]);
                    let pid = self.bark(g, text.to_string()) as u16;
                    next(self, Phase::Speak, pid);
                }
            }
            Phase::Speak => {
                if !running(self) && ticks >= SPEAK_TICKS {
                    self.stasis = false;
                    let it = self.map().get(g).copied().unwrap_or_default();
                    if (it.x - post.0).abs().max((it.y - post.1).abs()) > 8 {
                        let pid = self.add_native(crate::vm::Kind::Pathfind(crate::effects::Pathfind::new(g, 0, (post.0, post.1))), g, crate::effects::PROC_WALK);
                        next(self, Phase::Back, pid);
                    } else {
                        next(self, Phase::TurnBack, 0);
                    }
                }
            }
            Phase::Back => {
                if !running(self) || ticks >= BACK_TICKS {
                    self.kill_processes(g, crate::effects::PROC_WALK);
                    let it = self.map().get(g).copied().unwrap_or_default();
                    self.stop_walk(g, &it, (0, 0));
                    next(self, Phase::TurnBack, 0);
                }
            }
            Phase::TurnBack => {
                if self.turn_step(g, post_dir, ticks) {
                    self.palace.scene = None;
                }
            }
        }
    }

    /// Turns the guard an eighth towards `dir` every few ticks (as the
    /// scripts turn people), not while an animation of his still plays.
    /// True once he looks that way.
    fn turn_step(&mut self, g: u16, dir: u8, ticks: u32) -> bool {
        let Some(it) = self.map().get(g).copied() else { return true };
        self.mover(g);
        let Some(mover) = self.actors.get_mut(&g) else { return true };
        if mover.dir == dir {
            return true;
        }
        if mover.busy() || !ticks.is_multiple_of(TURN_TICKS) {
            return false;
        }
        let ahead = (dir as i32 - mover.dir as i32).rem_euclid(8);
        mover.dir = (mover.dir as i32 + if ahead <= 4 { 1 } else { -1 }).rem_euclid(8) as u8;
        mover.play(crate::anim::STAND, &it, &mut self.data.anims);
        mover.dir == dir
    }
}
