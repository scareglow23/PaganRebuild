//! An experiment (option "palace_guards", off unless switched on): the two
//! guards beside the double door in Mordea's palace in Tenebrae (map 40),
//! behind which lie the throne room and her chambers, stop the Avatar
//! while they see him. A double click on the door does not open it, and
//! he cannot step through it from the hall when it stands open. As in the
//! scenes of the game (the guard at the town gate), the Avatar is held
//! (stasis) while the guard walks a few steps up to him, turns to him step
//! by step, holds his halberd out and says so; then the guard puts it up
//! and goes back to his post. Invisible (the black potion), the Avatar
//! slips past, unless he brushes right past a guard (as creatures in a
//! fight see him, combat.rs); once he is through, a guard comes, shuts the
//! door he left open, and shakes his head over it (so too when the door
//! is opened from inside). Coming out again where a guard sees him, the
//! Avatar is held a moment and warned, and the guard shuts the door
//! behind him. A guard comes to shut the door only a while after the
//! Avatar has left it.
//! Scenes that lead the Avatar are not stopped.
//!
//! This makes the quest for the ceremonial dagger harder than in the
//! original: the way to the key under the pillow and into her bedroom
//! needs the Avatar invisible (the player's wish).

use crate::anim::{COMBAT_STAND, READY_WEAPON, SHAKE_HEAD, STAND, UNREADY_WEAPON};
use crate::game::{Game, AVATAR};
use crate::intrinsics::world_dir;
use crate::map::{Item, FLG_INVISIBLE};

/// Tenebrae.
const MAP: usize = 40;
/// A wing of the double door while it is shut, and open: only opening is
/// stopped, not shutting. The wings are known by their place, not their
/// numbers (the starting data of the German, French and English game
/// differ).
const SHUT_DOOR: u32 = 69;
const OPEN_DOOR: u32 = 68;
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
/// long as his words are shown); as long the guard wonders.
const SPEAK_TICKS: u32 = 60;
/// After the Avatar slipped through: the guard comes once he is this far
/// from the doorway, not before this many ticks, and not after this many.
const CLEAR: i32 = 96;
const SNEAK_WAIT: u32 = 20;
/// The guard notices the open door only after the Avatar has been clear
/// of it this long (not at once: as if he had to notice it first).
const REACT_TICKS: u32 = 90;
const SNEAK_FORGET: u32 = 1800;
/// How long the door takes to shut before the guard wonders.
const CLOSE_TICKS: u32 = 20;

/// What a guard does.
#[derive(Clone, Copy, PartialEq)]
enum Task {
    /// He stops the Avatar.
    Stop,
    /// He warns the Avatar who came out (not holding him).
    Warn,
    /// He shuts the door (this wing) the Avatar left open; and wonders who
    /// did, unless he saw him.
    Shut(u16, bool),
}

/// The scene of a guard.
struct Scene {
    guard: u16,
    task: Task,
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
    /// He walks up to the Avatar (to the door).
    StepUp,
    /// He turns to him (to the door).
    Turn,
    /// He holds his halberd out.
    Ready,
    /// He speaks (the Avatar is held until he has said it).
    Speak,
    /// He puts the halberd up again.
    Unready,
    /// He shuts the door.
    Shut,
    /// He shakes his head and wonders who left it open.
    Wonder,
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
    /// The Avatar left the door open: which wing, since when, and whether
    /// the guard is to wonder who did (he did not see him).
    sneaked: Option<(u16, u32, bool)>,
    /// Where the Avatar was in the last tick (to see him slip through).
    last: Option<(i32, i32, i32)>,
    /// Since when the Avatar has been clear of the open door.
    clear_since: Option<u32>,
}

impl PalaceGuards {
    /// Does a guard hold the Avatar now (the stasis is the scene's own)?
    pub(crate) fn holding(&self) -> bool {
        self.scene.as_ref().is_some_and(|s| matches!(s.task, Task::Stop | Task::Warn) && matches!(s.phase, Phase::StepUp | Phase::Turn | Phase::Ready | Phase::Speak))
    }
}

/// Is a point in front of the doorway, on the side of the hall (by the
/// Avatar's footprint, which reaches from y - `yd` to y)?
fn in_hall(p: (i32, i32, i32), yd: i32) -> bool {
    p.1 - yd >= DOOR_FRONT
}

/// Is a point (the Avatar's place, footprint `xd` by `yd`) within the
/// width of the doorway, at its height?
fn at_doorway(p: (i32, i32, i32), xd: i32) -> bool {
    p.0 > DOOR_X0 && p.0 - xd < DOOR_X1 && (p.2 - DOOR_Z).abs() < 24
}

impl Game {
    /// Does the rule apply now: switched on, in Tenebrae, no scene leading
    /// the Avatar?
    fn palace_rule(&self) -> bool {
        self.options.palace_guards && self.cur_map == MAP && !self.stasis
    }

    /// The Avatar's footprint (x, y).
    fn avatar_pad(&self) -> (i32, i32) {
        self.map().get(AVATAR).and_then(|av| self.data.infos.get(av.shape as usize)).map_or((64, 64), |si| (si.x * 32, si.y * 32))
    }

    /// Is the Avatar where the wings swing (in the doorway or just inside
    /// it)? They swing in, towards the throne room: in the hall he is not
    /// in their way.
    fn avatar_in_door(&self) -> bool {
        let av = self.avatar();
        let (xd, yd) = self.avatar_pad();
        let at = (av.x, av.y, av.z);
        at_doorway(at, xd) && !in_hall(at, yd) && av.y >= DOOR_FRONT - 32 - CLEAR
    }

    /// The guards at their posts by the doorway, alive.
    fn door_guards(&self) -> Vec<u16> {
        let (mx, my) = ((DOOR_X0 + DOOR_X1) / 2, DOOR_FRONT);
        let near_door = self.map().items_near(mx, my, POST_REACH + 64);
        near_door
            .into_iter()
            .map(|i| self.map().items[i])
            .filter(|it| it.is_actor() && it.in_world() && it.shape == GUARD_SHAPE && (it.x - mx).abs().max((it.y - my).abs()) <= POST_REACH)
            .filter(|it| !self.fights.states.get(&it.id).is_some_and(|s| s.dead))
            .map(|it| it.id)
            .collect()
    }

    /// The guard that sees the Avatar (at `at`), if one does.
    fn guard_watching(&self, at: (i32, i32, i32)) -> Option<u16> {
        let mut av: Item = *self.map().get(AVATAR)?;
        (av.x, av.y, av.z) = at;
        self.door_guards().into_iter().find(|&g| {
            let Some(it) = self.map().get(g).copied() else { return false };
            let gap = self.box_gap(&it, &av);
            let close = gap <= 20 && (av.z - it.z).abs() < 48;
            gap <= SIGHT && (av.z - it.z).abs() < 48 && (av.flags & FLG_INVISIBLE == 0 || close) && self.clear_between(&it, &av)
        })
    }

    /// Is `id` a wing of the double door (`shape`: shut or open)?
    fn palace_door(&self, id: u16, shape: u32) -> bool {
        self.map().get(id).is_some_and(|d| d.in_world() && d.shape == shape && d.y == DOOR_FRONT && (DOOR_X0..=DOOR_X1).contains(&d.x) && d.z == DOOR_Z)
    }

    /// A double click on the door (before its script runs): true if a
    /// guard stops it. Opened unseen (invisible), a guard will come and
    /// shut it once the Avatar is through.
    pub(crate) fn guards_keep_door(&mut self, id: u16) -> bool {
        if !self.palace_door(id, SHUT_DOOR) || !self.palace_rule() {
            return false;
        }
        let av = self.avatar();
        let (_, yd) = self.avatar_pad();
        // (from the hall, seen: stopped)
        if in_hall((av.x, av.y, av.z), yd) {
            if let Some(g) = self.guard_watching((av.x, av.y, av.z)) {
                self.guard_stops(g);
                return true;
            }
        }
        // (unseen, or from inside: a guard will shut it again)
        if !self.door_guards().is_empty() {
            self.palace.sneaked = Some((id, self.tick, true));
        }
        false
    }

    /// Would the Avatar, going from `from` to `to`, step into the doorway
    /// from the hall while a guard sees him?
    pub(crate) fn guards_bar_step(&self, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        if !self.palace_rule() {
            return false;
        }
        let (xd, yd) = self.avatar_pad();
        at_doorway(to, xd) && in_hall(from, yd) && !in_hall(to, yd) && self.guard_watching(from).is_some()
    }

    /// After the actors have moved: the Avatar held back at the doorway
    /// is stopped by a guard; one who slipped through unseen leaves the
    /// door to be shut; the scene of a guard goes on.
    pub(crate) fn tick_palace_guards(&mut self) {
        let av = self.avatar();
        let now = (av.x, av.y, av.z);
        if std::mem::take(&mut self.palace.barred) {
            if let Some(g) = self.guard_watching(now) {
                self.guard_stops(g);
            }
        }
        // (through a door that stood open, unseen: the wing nearest him is
        // to be shut; coming out where a guard sees him: he is warned, and
        // the door shut behind him)
        let (xd, yd) = self.avatar_pad();
        if let Some(was) = self.palace.last.replace(now) {
            let through = at_doorway(now, xd) && in_hall(was, yd) && !in_hall(now, yd);
            let out = at_doorway(now, xd) && !in_hall(was, yd) && in_hall(now, yd);
            if (through || out) && self.palace_rule() {
                let wing = self.map().items_near(now.0, DOOR_FRONT, 256).into_iter().map(|i| self.map().items[i]).filter(|d| self.palace_door(d.id, OPEN_DOOR)).min_by_key(|d| (d.x - now.0).abs()).map(|d| d.id);
                let seen = if out { self.guard_watching(now) } else { None };
                if let Some(g) = seen {
                    if let Some(wing) = wing.or(self.palace.sneaked.map(|s| s.0)) {
                        self.palace.sneaked = Some((wing, self.tick, false));
                    }
                    self.guard_holds(g, Task::Warn);
                } else if through && av.flags & FLG_INVISIBLE != 0 && self.palace.sneaked.is_none() {
                    if let Some(wing) = wing {
                        self.palace.sneaked = Some((wing, self.tick, true));
                    }
                }
            }
        }
        self.guard_comes_to_shut();
        self.tick_guard_scene();
    }

    /// The Avatar slipped through unseen and is clear of the doorway: the
    /// guard nearest the open wing comes to shut it.
    fn guard_comes_to_shut(&mut self) {
        let Some((wing, since, wonder)) = self.palace.sneaked else { return };
        let waited = self.tick.wrapping_sub(since);
        // (shut again meanwhile - not just clicked, still shut until it
        // swings open -, or long ago, or switched off: nothing to wonder at)
        let shut = waited >= SNEAK_WAIT && self.palace_door(wing, SHUT_DOOR);
        if waited > SNEAK_FORGET || shut || !self.options.palace_guards || self.cur_map != MAP {
            self.palace.sneaked = None;
            return;
        }
        // (still swinging open, or he is not yet through)
        if !self.palace_door(wing, OPEN_DOOR) || waited < SNEAK_WAIT || self.palace.scene.is_some() || self.stasis {
            return;
        }
        if self.avatar_in_door() {
            self.palace.clear_since = None;
            return;
        }
        let tick = self.tick;
        if tick.wrapping_sub(*self.palace.clear_since.get_or_insert(tick)) < REACT_TICKS {
            return;
        }
        self.palace.clear_since = None;
        let Some(door) = self.map().get(wing).copied() else { return };
        let Some(g) = self.door_guards().into_iter().min_by_key(|&g| self.map().get(g).map_or(i32::MAX, |it| (it.x - door.x).abs() + (it.y - door.y).abs())) else { return };
        self.palace.sneaked = None;
        self.start_scene(g, Task::Shut(wing, wonder));
    }

    /// Guard `g` stops the Avatar: the Avatar is held, turned to him, and
    /// the scene begins (anew, if he was on his way back to his post).
    fn guard_stops(&mut self, g: u16) {
        self.guard_holds(g, Task::Stop);
    }

    /// Guard `g` holds the Avatar for `task` (stopping or warning him).
    fn guard_holds(&mut self, g: u16, task: Task) {
        if self.palace.holding() {
            return;
        }
        let Some(it) = self.map().get(g).copied() else { return };
        self.stasis = true;
        self.cancel_approach();
        let av = self.avatar();
        self.stop_walk(AVATAR, &av, (it.x - av.x, it.y - av.y));
        self.start_scene(g, task);
    }

    /// Guard `g` sets out on `task` (an earlier scene of the guards ends).
    fn start_scene(&mut self, g: u16, task: Task) {
        let Some(it) = self.map().get(g).copied() else { return };
        if let Some(old) = self.palace.scene.take() {
            if let Some(m) = self.actors.get_mut(&old.guard) {
                m.stand_anim = STAND;
            }
        }
        let dir = self.mover(g).dir;
        self.mover(g).stand_anim = STAND;
        let (post, post_dir) = *self.palace.posts.entry(g).or_insert(((it.x, it.y, it.z), dir));
        self.kill_processes(g, crate::effects::PROC_WALK);
        let target = match task {
            Task::Stop | Task::Warn => AVATAR,
            Task::Shut(wing, _) => wing,
        };
        // (a warning he calls from where he stands)
        let far = task != Task::Warn && self.map().get(target).copied().is_none_or(|t| self.box_gap(&it, &t) > STEP_UP);
        let (phase, pid) = if far {
            let pid = self.add_native(crate::vm::Kind::Pathfind(crate::effects::Pathfind::new(g, target, (0, 0))), g, crate::effects::PROC_WALK);
            (Phase::StepUp, pid)
        } else {
            (Phase::Turn, 0)
        };
        self.palace.scene = Some(Scene { guard: g, task, phase, since: self.tick, pid, post, post_dir });
    }

    /// One tick of the scene.
    fn tick_guard_scene(&mut self) {
        let Some(scene) = self.palace.scene.as_ref() else { return };
        let (g, task, phase, pid, post, post_dir) = (scene.guard, scene.task, scene.phase, scene.pid, scene.post, scene.post_dir);
        let ticks = self.tick.wrapping_sub(scene.since);
        let running = |game: &Game| game.scripts.procs.iter().any(|p| p.pid == pid && !p.terminated);
        // (the guard gone, fallen, or another map: the scene ends)
        let alive = self.cur_map == MAP && self.map().get(g).is_some_and(|it| it.in_world()) && !self.fights.states.get(&g).is_some_and(|s| s.dead);
        if !alive {
            if self.palace.holding() {
                self.stasis = false;
            }
            if let Some(m) = self.actors.get_mut(&g) {
                m.stand_anim = STAND;
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
        let it = self.map().get(g).copied().unwrap_or_default();
        let busy = self.actors.get(&g).is_some_and(|m| m.busy());
        match phase {
            Phase::StepUp => {
                if !running(self) || ticks >= WALK_TICKS {
                    self.kill_processes(g, crate::effects::PROC_WALK);
                    self.stop_walk(g, &it, (0, 0));
                    next(self, Phase::Turn, 0);
                }
            }
            Phase::Turn => {
                let to = match task {
                    Task::Stop | Task::Warn => {
                        let av = self.avatar();
                        (av.x, av.y)
                    }
                    Task::Shut(wing, _) => {
                        let d = self.map().get(wing).copied().unwrap_or_default();
                        let (xd, yd) = self.data.infos.get(d.shape as usize).map_or((0, 0), |si| (si.x * 32, si.y * 32));
                        (d.x - xd / 2, d.y - yd / 2)
                    }
                };
                if self.turn_step(g, world_dir(to.0 - it.x, to.1 - it.y) as u8, ticks) {
                    match task {
                        Task::Stop | Task::Warn => {
                            self.guard_plays(g, READY_WEAPON);
                            next(self, Phase::Ready, 0);
                        }
                        Task::Shut(..) => next(self, Phase::Shut, 0),
                    }
                }
            }
            Phase::Ready => {
                // (the halberd held out while he speaks)
                if !busy {
                    if let Some(m) = self.actors.get_mut(&g) {
                        m.stand_anim = COMBAT_STAND;
                    }
                    self.guard_plays(g, COMBAT_STAND);
                    let text = if task == Task::Warn {
                        self.tr(["Lass dich da drin nicht noch einmal blicken!", "Que je ne t'y reprenne plus !", "Let me not catch thee in there again!"])
                    } else {
                        self.tr(["Halt! Hier ist kein Durchgang.", "Halte ! On ne passe pas.", "Halt! There is no passage here."])
                    };
                    let pid = self.bark(g, text.to_string()) as u16;
                    next(self, Phase::Speak, pid);
                }
            }
            Phase::Speak => {
                if !running(self) && ticks >= SPEAK_TICKS {
                    // (only the stasis of his own stopping or warning)
                    if matches!(task, Task::Stop | Task::Warn) {
                        self.stasis = false;
                    }
                    if let Some(m) = self.actors.get_mut(&g) {
                        m.stand_anim = STAND;
                    }
                    self.guard_plays(g, UNREADY_WEAPON);
                    next(self, Phase::Unready, 0);
                }
            }
            Phase::Unready | Phase::Wonder => {
                let said = phase == Phase::Unready || (!running(self) && ticks >= SPEAK_TICKS);
                if !busy && said {
                    if (it.x - post.0).abs().max((it.y - post.1).abs()) > 8 {
                        // (right into his niche, not near it)
                        let mut pf = crate::effects::Pathfind::new(g, 0, (post.0, post.1));
                        pf.exact = true;
                        let pid = self.add_native(crate::vm::Kind::Pathfind(pf), g, crate::effects::PROC_WALK);
                        next(self, Phase::Back, pid);
                    } else {
                        next(self, Phase::TurnBack, 0);
                    }
                }
            }
            Phase::Shut => {
                let Task::Shut(wing, wonder) = task else { return };
                // (pid: 0 until he has shut it; the time counts from then)
                if pid == 0 {
                    // (not onto the Avatar, who came back into the doorway)
                    if self.avatar_in_door() {
                        return;
                    }
                    if self.palace_door(wing, OPEN_DOOR) {
                        self.call_event(wing, 1, &[]);
                    }
                    next(self, Phase::Shut, 1);
                } else if ticks >= CLOSE_TICKS && !wonder {
                    // (he saw who it was: nothing to wonder at)
                    next(self, Phase::Unready, 0);
                } else if ticks >= CLOSE_TICKS {
                    self.guard_plays(g, SHAKE_HEAD);
                    let text = self.tr(["Nanu? Wer hat die T\u{81}r offen gelassen?", "Tiens ? Qui a laiss\u{82} cette porte ouverte ?", "Hm? Who left this door open?"]);
                    let pid = self.bark(g, text.to_string()) as u16;
                    next(self, Phase::Wonder, pid);
                }
            }
            Phase::Back => {
                if !running(self) || ticks >= BACK_TICKS {
                    self.kill_processes(g, crate::effects::PROC_WALK);
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

    /// The guard plays an animation (his own, as he looks now).
    fn guard_plays(&mut self, g: u16, anim: u32) {
        let Some(it) = self.map().get(g).copied() else { return };
        self.mover(g);
        if let Some(m) = self.actors.get_mut(&g) {
            m.play(anim, &it, &mut self.data.anims);
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
        mover.play(STAND, &it, &mut self.data.anims);
        mover.dir == dir
    }
}
