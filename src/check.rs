//! Test run over the game scripts (`--do scriptcheck:<maps>`): on each
//! map, every item with a script of its own is looked at and used (books
//! and the like once per text, everything else once per script class).
//! Conversations and menus are explored: every path through the answers
//! up to a depth is tried. Items of scripts that lie on no map are made
//! next to the Avatar, and the original debug menu (with its spell book)
//! is walked through. Questions for a target pick the nearest creature,
//! amounts are confirmed, books paged through.
//!
//! Script errors, endless loops and crashes are written to stderr after a
//! line that names the item and the answers given, so the log shows the
//! culprit.

use crate::game::{Game, AVATAR};
use crate::hash::HashMap;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Ticks a script gets to finish (30 per second).
const TICK_LIMIT: u32 = 6000;
/// Answers given before the conversation is left alone.
const ANSWER_LIMIT: usize = 40;

#[derive(Default)]
pub struct Report {
    pub runs: usize,
    pub crashes: usize,
    pub stuck: usize,
}

/// How a script run ended.
enum Outcome {
    Done,
    Stuck,
    Crash,
}

/// How deep and how often to explore the answers of one script.
#[derive(Clone, Copy)]
struct Explore {
    runs: usize,
    depth: usize,
}

/// The events tried besides look (0) and use (1): anim, hit, got hit,
/// hatch, schedule, release, enter / leave fast area, cast, stolen.
const OTHER_EVENTS: [u32; 10] = [2, 5, 6, 7, 8, 9, 0x0F, 0x10, 0x11, 0x13];

/// Names for the log.
pub(crate) fn event_name(event: u32) -> &'static str {
    match event {
        0 => "look",
        1 => "use",
        2 => "anim",
        5 => "hit",
        6 => "gotHit",
        7 => "hatch",
        8 => "schedule",
        9 => "release",
        0x0F => "enterFastArea",
        0x10 => "leaveFastArea",
        0x11 => "cast",
        0x13 => "AvatarStoleSomething",
        _ => "?",
    }
}

/// The number of a named event (as `event_name` names it).
pub(crate) fn event_number(name: &str) -> Option<u32> {
    (0..32).find(|&e| event_name(e) == name)
}

/// Arguments as the engine passes them: who hit / what was taken is the
/// Avatar or an item next to him, the time is noon.
pub(crate) fn event_args(event: u32) -> Vec<u8> {
    match event {
        5 | 6 => [AVATAR.to_le_bytes(), 4u16.to_le_bytes()].concat(),
        8 => (12u16 * 60).to_le_bytes().to_vec(),
        0x11 => 0u16.to_le_bytes().to_vec(),
        0x13 => AVATAR.to_le_bytes().to_vec(),
        _ => Vec::new(),
    }
}

const ONCE: Explore = Explore { runs: 1, depth: 0 };
const TALK: Explore = Explore { runs: 16, depth: 2 };
const DEBUG_MENU: Explore = Explore { runs: 400, depth: 5 };

impl Game {
    /// Looks at and uses every scripted item of the map; `seen` counts
    /// what was tried already (also on other maps).
    pub fn check_scripts(&mut self, map: usize, seen: &mut HashMap<(u32, u32, u16), usize>, report: &mut Report, jump: &mut dyn FnMut(&mut Game, usize)) {
        jump(self, map);
        let candidates: Vec<(u16, u32, u16, bool)> = self
            .map()
            .items
            .iter()
            .filter(|it| !it.gone && it.id != 0 && it.id != AVATAR)
            .map(|it| (it.id, self.usecode_class(it), it.quality, it.is_actor()))
            .collect();
        for (id, class, quality, actor) in candidates {
            for event in [0u32, 1].into_iter().chain(OTHER_EVENTS) {
                if self.scripts.usecode.event(class, event).is_none() {
                    continue;
                }
                // the same script once, but books (and other things whose
                // quality picks the text) once per quality
                let per_class = seen.entry((class, event, u16::MAX)).or_insert(0);
                if *per_class >= 60 {
                    continue;
                }
                let key = (class, event, if actor { 0 } else { quality });
                if seen.contains_key(&key) {
                    continue;
                }
                *seen.get_mut(&(class, event, u16::MAX)).unwrap() += 1;
                seen.insert(key, 1);
                let how = if actor && event == 1 { TALK } else { ONCE };
                self.explore(map, id, event, how, report, jump);
            }
        }
    }

    /// Items of the script classes that lie on no map (body parts, things
    /// made by spells ...): made next to the Avatar and tried there.
    pub fn check_unplaced(&mut self, map: usize, seen: &mut HashMap<(u32, u32, u16), usize>, report: &mut Report, jump: &mut dyn FnMut(&mut Game, usize)) {
        for class in 2..0x400u32 {
            for event in [0u32, 1].into_iter().chain(OTHER_EVENTS) {
                if self.scripts.usecode.event(class, event).is_none() || seen.contains_key(&(class, event, u16::MAX)) {
                    continue;
                }
                seen.insert((class, event, u16::MAX), 1);
                if self.cur_map != map {
                    jump(self, map);
                }
                let av = self.avatar();
                let id = self.create_item(class, 0, crate::map::IN_WORLD);
                if let Some(idx) = self.map().index(id) {
                    self.map_mut().place(idx, av.x - 96, av.y - 96, av.z);
                }
                eprintln!("scriptcheck: made item {id} shape {class}");
                self.explore(map, id, event, ONCE, report, jump);
            }
        }
    }

    /// The debug menu of the original (looking at the Avatar with --cheat):
    /// every spell of the spell book and the other entries.
    pub fn check_debug_menu(&mut self, map: usize, report: &mut Report, jump: &mut dyn FnMut(&mut Game, usize)) {
        jump(self, map);
        let cheat = self.cheat;
        self.cheat = true;
        self.explore(map, AVATAR, 0, DEBUG_MENU, report, jump);
        self.cheat = cheat;
    }

    /// Runs an event again and again, each time with other answers: first
    /// none chosen deliberately, then every choice of every question met,
    /// breadth first, up to the depth.
    fn explore(&mut self, map: usize, id: u16, event: u32, how: Explore, report: &mut Report, jump: &mut dyn FnMut(&mut Game, usize)) {
        let mut queue: VecDeque<Vec<usize>> = VecDeque::from([Vec::new()]);
        let mut runs = 0;
        while let Some(path) = queue.pop_front() {
            if runs >= how.runs {
                break;
            }
            runs += 1;
            if self.cur_map != map {
                jump(self, map);
            }
            let Some(it) = self.map().get(id).copied() else { return };
            eprintln!(
                "scriptcheck: map {map} item {id} shape {} class {:04X} {} answers {path:?}",
                it.shape,
                self.usecode_class(&it),
                event_name(event)
            );
            report.runs += 1;
            let mut found: Option<usize> = None;
            let outcome = match catch_unwind(AssertUnwindSafe(|| self.run_with_answers(id, event, &path, &mut found))) {
                Ok(true) => Outcome::Done,
                Ok(false) => Outcome::Stuck,
                Err(e) => {
                    let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    eprintln!("scriptcheck: CRASH - {msg}");
                    Outcome::Crash
                }
            };
            match outcome {
                Outcome::Done => {}
                Outcome::Stuck => {
                    eprintln!("scriptcheck: STUCK - still running after {TICK_LIMIT} ticks; {}", self.describe_busy());
                    report.stuck += 1;
                }
                Outcome::Crash => {
                    report.crashes += 1;
                    jump(self, map);
                }
            }
            self.calm_down();
            if let Some(n) = found {
                if path.len() < how.depth {
                    for i in 0..n {
                        let mut p = path.clone();
                        p.push(i);
                        queue.push_back(p);
                    }
                }
            }
        }
    }

    /// Runs one event, answering the questions with `path` (by position),
    /// later ones with the last choice (mostly "bye" or "done"). `found`:
    /// how many choices the first question beyond the path had. False if
    /// the script is still busy at the end.
    fn run_with_answers(&mut self, id: u16, event: u32, path: &[usize], found: &mut Option<usize>) -> bool {
        // stand next to it, as a player would
        if id != AVATAR {
            if let Some(it) = self.map().get(id).copied().filter(|it| it.in_world()) {
                let map = self.cur_map;
                self.teleport(AVATAR, map, (it.x + 64, it.y + 64, it.z));
            }
        }
        let pid = self.call_event(id, event, &event_args(event));
        if pid == 0 {
            return true;
        }
        let mut asked = 0;
        for _ in 0..TICK_LIMIT {
            if let Some((_, choices)) = self.ask.clone() {
                let n = match path.get(asked) {
                    Some(&i) => i.min(choices.len().saturating_sub(1)),
                    None => {
                        if asked == path.len() {
                            *found = Some(choices.len());
                        }
                        choices.len().saturating_sub(1)
                    }
                };
                asked += 1;
                if asked > ANSWER_LIMIT {
                    return false;
                }
                self.answer(n);
            }
            if self.ui.targeting.is_some() {
                let target = self.nearest_creature();
                self.target_picked(target);
            }
            if self.ui.slider.is_some() {
                self.slider_confirm();
            }
            if self.ui.reader.is_some() {
                self.reader_next();
            }
            self.step(None);
            if self.proc_index(pid).is_none() && self.ask.is_none() && self.ui.reader.is_none() {
                return true;
            }
        }
        false
    }

    /// For questions for a target: the nearest living creature but the Avatar.
    fn nearest_creature(&self) -> u16 {
        let av = self.avatar();
        self.map()
            .items
            .iter()
            .filter(|it| it.is_actor() && it.id != AVATAR && !it.dead && !it.gone && it.in_world())
            .min_by_key(|it| (it.x - av.x).abs() + (it.y - av.y).abs())
            .map_or(0, |it| it.id)
    }

    /// What the scripts are busy with (for a report).
    fn describe_busy(&self) -> String {
        let busy: Vec<String> = self
            .scripts
            .procs
            .iter()
            .filter(|p| !p.terminated && p.class != 0)
            .take(6)
            .map(|p| format!("{:04X}:{:04X} item {}{}", p.class, p.ip, p.item, if p.suspended { " (waits)" } else { "" }))
            .collect();
        format!("ask {:?}, reader {}, stasis {}, scripts: {}", self.ask.as_ref().map(|a| &a.1), self.ui.reader.is_some(), self.stasis, busy.join(", "))
    }

    /// Back to a plain state for the next item: no open windows, the
    /// Avatar alive and free to move.
    fn calm_down(&mut self) {
        if let Some((pid, _)) = self.ask.take() {
            self.kill_pid(pid);
        }
        if let Some(r) = self.ui.reader.take() {
            self.kill_pid(r.pid);
        }
        self.target_picked(0);
        self.ui.slider = None;
        self.ui.gumps.clear();
        self.stasis = false;
        // a script stopped half-way may have left the view somewhere else
        // (the active area goes with the view)
        self.camera = crate::effects::CameraMode::Follow(AVATAR);
        self.avatar_died = None;
        self.screen.fade = None;
        self.ui.title = None;
        self.ui.slot_menu = None;
        let s = self.state_mut(AVATAR);
        s.dead = false;
        s.hp = s.hp.max(1);
    }
}
