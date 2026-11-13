//! Help for finding things (modern controls): the name of what is under
//! the pointer, and on a held key everything one can use or take lights up.
//!
//! The original has no list of names: looking at a thing runs its "look"
//! script, which says the name (a coin pile counts its coins, people say
//! their name only once the Avatar knows it). To show the same name without
//! the script doing anything, it runs on the side: only questions about
//! items are allowed, the first thing said is caught instead of spoken, and
//! everything the script touched is put back afterwards.

use crate::game::{Bark, Game, AVATAR};
use crate::view::Surface;
use crate::vm::Kind;

/// Engine functions a look script may call while it runs on the side:
/// they only ask about items, people, the time or the like.
const HARMLESS: &[u16] = &[
    0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0C, 0x0D, 0x0F, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x19,
    0x1E, 0x1F, 0x20, 0x23, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x42, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x72, 0x73, 0x76,
    0x7A, 0x7B, 0x7C, 0x86, 0x88, 0x8A, 0x8C, 0x9A, 0x9C, 0x9D, 0xA0, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xB0, 0xB2, 0xB6,
    0xB9, 0xBC, 0xC3, 0xC4, 0xC5, 0xCA, 0xCF, 0xD1, 0xD2, 0xD7, 0xD8, 0xEB, 0xFC,
];
/// Engine functions that would change something; on the side they do
/// nothing and the script goes on (a grenade resets its fuse when looked at).
const SKIPPED: &[u16] = &[0xCB, 0xCD];
const BARK: u16 = 0x49;

/// A look script running on the side.
#[derive(Default)]
pub struct NameProbe {
    pub text: Option<String>,
    /// It wanted to do something other than ask: no name then.
    pub stopped: bool,
}

/// Brighter by a part of the way to white.
fn lift(v: u32, part: u32) -> u32 {
    let l = |c: u32| c + (255 - c) / part;
    (l((v >> 16) & 0xFF) << 16) | (l((v >> 8) & 0xFF) << 8) | l(v & 0xFF)
}

impl Game {
    /// Called by the script machine for every engine function while a
    /// look script runs on the side. Some(result) takes the call over.
    pub fn probe_intrinsic(&mut self, caller: usize, func: u16, text: impl FnOnce(&Game) -> String) -> Option<u32> {
        self.name_probe.as_ref()?;
        if func == BARK {
            let t = text(self).trim().to_string();
            let probe = self.name_probe.as_mut()?;
            if !t.is_empty() {
                probe.text = Some(t);
                self.scripts.procs[caller].term_deferred = true;
            }
            return Some(0);
        }
        if HARMLESS.contains(&func) {
            return None;
        }
        if SKIPPED.contains(&func) {
            return Some(0);
        }
        self.name_probe.as_mut()?.stopped = true;
        self.scripts.procs[caller].term_deferred = true;
        Some(0)
    }

    /// The look script has said something, or cannot go on.
    fn probe_done(&self) -> bool {
        self.name_probe.as_ref().is_none_or(|p| p.text.is_some() || p.stopped)
    }

    /// One pass over the scripts the look started (from `start` on).
    /// False if none of them ran.
    fn probe_pass(&mut self, start: usize) -> bool {
        let mut ran = false;
        for i in start..self.scripts.procs.len() {
            if self.probe_done() {
                break;
            }
            let p = &self.scripts.procs[i];
            if p.term_deferred && !p.terminated {
                self.terminate(i);
                ran = true;
                continue;
            }
            if p.terminated || p.suspended {
                continue;
            }
            if !matches!(p.kind, Kind::Usecode) {
                // waits for something to happen in the world: give up
                if let Some(p) = self.name_probe.as_mut() {
                    p.stopped = true;
                }
                break;
            }
            self.exec(i);
            ran = true;
        }
        ran
    }

    /// What looking at an item would say, without saying it.
    pub fn look_name(&mut self, id: u16) -> Option<String> {
        let globals = self.scripts.usecode.globals().to_vec();
        let saved = (self.rng.0, self.scripts.next_pid, self.scripts.strings.clone(), self.scripts.next_string, self.scripts.lists.clone(), self.scripts.next_list);
        let start = self.scripts.procs.len();
        self.name_probe = Some(NameProbe::default());
        if self.call_event(id, 0, &[]) != 0 {
            // the look script and whatever it waits for
            for _ in 0..16 {
                if !self.probe_pass(start) || self.probe_done() {
                    break;
                }
            }
        }
        let probe = self.name_probe.take();
        // put everything back as it was
        self.scripts.procs.truncate(start);
        self.scripts.usecode.set_globals(&globals);
        (self.rng.0, self.scripts.next_pid, self.scripts.strings, self.scripts.next_string, self.scripts.lists, self.scripts.next_list) = saved;
        probe.and_then(|p| p.text)
    }

    /// A name for a thing the game gives none (it has no look script):
    /// the spheres for the fire puzzle in the lava caves.
    pub fn unnamed_thing(&self, id: u16) -> Option<String> {
        const SPHERE: u32 = 784;
        let it = self.map().get(id)?;
        (it.shape == SPHERE).then(|| self.tr(["Kugel", "sph\u{8a}re", "sphere"]).to_string())
    }

    /// Worth pointing out: what can be used or taken, people and enemies.
    pub fn noteworthy(&mut self, id: u16) -> bool {
        if id == AVATAR || id == 0 {
            return false;
        }
        let Some(it) = self.map().get(id).copied() else { return false };
        (self.can_fight(&it) && it.in_world()) || self.portable(id) || self.usable(id) || self.hostile(id) || self.puzzle_spot(id).is_some()
    }

    /// The name of the item under the pointer, shown like speech over it
    /// (unless it is just speaking itself). Removed again by `end_hover_label`.
    pub fn begin_hover_label(&mut self) {
        if !self.options.modern_controls {
            return;
        }
        let (Some(id), Some(text)) = (self.ui.hover, self.ui.hover_name.clone()) else { return };
        // (puzzle spots are named even with the names switched off)
        if self.options.hover_names == 0 && self.puzzle_spot(id).is_none() {
            return;
        }
        if self.barks.iter().any(|b| b.item == id) {
            return;
        }
        // With the weapon drawn no names over people and creatures: the
        // pointer and the health bars show the enemies, and a name would
        // flicker on and off as they move under the pointer.
        if self.state(AVATAR).in_combat && self.map().get(id).is_some_and(|it| self.can_fight(it)) {
            return;
        }
        self.barks.push(Bark { pid: 0, item: id, text });
    }

    pub fn end_hover_label(&mut self) {
        self.barks.retain(|b| b.pid != 0);
    }

    /// While the key is held: everything worth pointing out in the view
    /// lights up, clearly brighter than the thing under the pointer.
    pub fn draw_show_all(&mut self, surf: &mut Surface) {
        if !self.ui.show_all {
            return;
        }
        let n = self.map().items.len();
        // what can be seen at all, then which of it is worth a light
        let mut seen = vec![false; n];
        for &v in self.drawn_ids() {
            if v != 0 && (v as usize) <= n {
                seen[v as usize - 1] = true;
            }
        }
        let mut wanted = vec![false; n];
        for index in (0..n).filter(|&i| seen[i]) {
            let it = self.map().items[index];
            wanted[index] = it.in_world() && self.noteworthy(it.id);
        }
        self.brighten(surf, &|i| wanted.get(i).copied().unwrap_or(false), 3);
    }

    /// Brightens the item under the pointer where it can be seen.
    pub fn draw_hover(&mut self, surf: &mut Surface) {
        let Some(index) = self.ui.hover.and_then(|id| self.map().index(id)) else { return };
        self.brighten(surf, &|i| i == index, 5);
    }

    /// Which item each pixel of the last picture shows (index + 1, 0 = none).
    pub(crate) fn drawn_ids(&self) -> &[u32] {
        if self.view_turned() { self.turn.ids() } else { &self.view.pixel_ids }
    }

    /// Lifts the pixels of the chosen items a part of the way to white.
    /// Only where an item itself shows, not through what stands in front.
    fn brighten(&self, surf: &mut Surface, want: &dyn Fn(usize) -> bool, part: u32) {
        let ids = self.drawn_ids();
        if ids.len() != surf.pixels.len() {
            return;
        }
        for (px, &v) in surf.pixels.iter_mut().zip(ids) {
            if v != 0 && want(v as usize - 1) {
                *px = lift(*px, part);
            }
        }
    }
}
