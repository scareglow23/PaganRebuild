//! The quest journal (option "journal", key T): what the Avatar has
//! learned and done so far, oldest first, shown in the game's own book.
//! Every entry belongs to a condition on the game's global flags (which
//! the scripts set anyway) or on what the Avatar carries; the texts are in
//! quests.rs. Only the order in which the entries were reached is kept in
//! the savegame. What the debug menu of the original (option "Avatar menu")
//! brings about is not written down.

use crate::game::{Game, AVATAR};
use crate::gumps::Look;
use crate::quests::{Quest, Stage, QUESTS};

/// German umlauts as the game's fonts have them (DOS code page 437).
pub(crate) fn dos(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ä' => '\u{84}',
            'ö' => '\u{94}',
            'ü' => '\u{81}',
            'Ä' => '\u{8E}',
            'Ö' => '\u{99}',
            'Ü' => '\u{9A}',
            'ß' => '\u{E1}',
            c => c,
        })
        .collect()
}

/// How often (in game ticks) the journal looks for new entries.
const CHECK_TICKS: u32 = 30;

/// The order in which the entries were reached (the flags alone do not
/// tell it); kept in the savegame.
#[derive(Default)]
pub struct Journal {
    /// Keys of the entries (see `key`), oldest first.
    pub order: Vec<String>,
    /// False after loading: entries found then are not announced.
    synced: bool,
    /// The debug menu was open since the last look for new entries.
    cheat_menu: bool,
    /// A new entry waits to be announced until the scene is over.
    pending: Option<&'static Quest>,
}

/// Marks an entry in `Journal::order` that was reached through the debug
/// menu: it is not shown, and once its condition is gone it is forgotten,
/// so that reaching it in play later counts.
const BY_CHEAT: char = '~';

/// Type of the process of the Avatar's look script, which holds the debug
/// menu while it is open.
const LOOK_PROCESS: u16 = 520;
/// Script class of the Avatar (people: 1024 + their number).
const AVATAR_CLASS: u16 = 1024 + AVATAR;

impl Journal {
    /// After loading: the order stored in the savegame (empty for one from
    /// before the journal).
    pub fn restore(&mut self, order: Vec<String>) {
        self.order = order;
        self.synced = false;
        self.pending = None;
        self.cheat_menu = false;
    }
}

/// A name for an entry that stays the same when entries are added.
fn key(q: &Quest, s: &Stage) -> String {
    format!("{}/{}", q.en, s.when)
}

impl Game {
    pub(crate) fn flag(&self, name: &str) -> u32 {
        match self.scripts.usecode.flag_at(name) {
            Some((pos, bits)) => self.scripts.usecode.global(pos, bits),
            None => 0,
        }
    }

    /// Sets a global flag by its name (for tests).
    pub fn set_flag(&mut self, name: &str, val: u32) -> Result<(), String> {
        let (pos, bits) = self.scripts.usecode.flag_at(name).ok_or(format!("no flag '{name}'"))?;
        self.scripts.usecode.set_global(pos, bits, val);
        Ok(())
    }

    /// Whether the Avatar carries a thing (in his backpack or anything in it).
    fn carries(&self, shape: u32, frame: Option<u32>) -> bool {
        let map = self.map();
        map.items.iter().any(|it| {
            if it.gone || it.id == 0 || it.shape != shape || frame.is_some_and(|f| it.frame != f) {
                return false;
            }
            let mut p = it.parent;
            for _ in 0..16 {
                if p == 0 || p == AVATAR {
                    break;
                }
                p = map.get(p).map_or(0, |c| c.parent);
            }
            p == AVATAR
        })
    }

    /// One term of a condition (see quests.rs).
    fn term(&self, term: &str) -> bool {
        if let Some(t) = term.strip_prefix('!') {
            return !self.term(t.trim());
        }
        if let Some(t) = term.strip_prefix("has:") {
            let mut f = t.split(':').map(|n| n.trim().parse::<u32>().ok());
            return match (f.next().flatten(), f.next().flatten()) {
                (Some(shape), frame) => self.carries(shape, frame),
                _ => false,
            };
        }
        if let Some(t) = term.strip_prefix("dead:") {
            return t.trim().parse::<u16>().is_ok_and(|npc| self.fights.states.get(&npc).is_some_and(|s| s.dead));
        }
        match term.split_once('=') {
            Some((name, val)) => val.trim().parse().is_ok_and(|v: u32| self.flag(name.trim()) == v),
            None => self.flag(term) != 0,
        }
    }

    pub(crate) fn holds(&self, cond: &str) -> bool {
        !cond.is_empty() && cond.split('|').any(|group| group.split('&').all(|t| self.term(t.trim())))
    }

    /// Whether the debug menu of the original is open (the Avatar's look
    /// script runs it, and only that long).
    fn cheat_menu_open(&self) -> bool {
        self.scripts.procs.iter().any(|p| p.item == AVATAR && p.class == AVATAR_CLASS && p.ptype == LOOK_PROCESS && !p.terminated)
    }

    fn german(&self) -> bool {
        self.language == 'G'
    }

    /// Notes newly reached entries at the end of the journal and returns
    /// the quest of the first one (none right after loading: entries of an
    /// older savegame are added quietly, in the order of the quests).
    fn sync_journal(&mut self) -> Option<&'static Quest> {
        // entries reached through the debug menu are kept apart until
        // their condition is gone again
        let gone: Vec<String> = (self.journal.order.iter())
            .filter(|k| k.starts_with(BY_CHEAT))
            .filter(|k| !QUESTS.iter().flat_map(|q| q.stages.iter().map(move |s| (q, s))).any(|(q, s)| k[1..] == key(q, s) && self.holds(s.when)))
            .cloned()
            .collect();
        self.journal.order.retain(|k| !gone.contains(k));
        if self.cheat_menu_open() {
            return None;
        }
        // (the menu may have been open only between two looks: noted every tick)
        let closed = std::mem::take(&mut self.journal.cheat_menu);
        let mut first = None;
        for q in QUESTS {
            for s in q.stages {
                if self.holds(s.when) {
                    let k = key(q, s);
                    let cheat = format!("{BY_CHEAT}{k}");
                    if self.journal.order.contains(&k) || self.journal.order.contains(&cheat) {
                        continue;
                    }
                    if closed {
                        self.journal.order.push(cheat);
                    } else {
                        self.journal.order.push(k);
                        first = first.or(Some(q));
                    }
                }
            }
        }
        let quiet = !self.journal.synced;
        self.journal.synced = true;
        if quiet { None } else { first }
    }

    /// Now and then: notes new entries and announces them (only with the
    /// option on).
    pub fn tick_journal(&mut self) {
        if self.cheat_menu_open() {
            self.journal.cheat_menu = true;
        }
        if !self.tick.is_multiple_of(CHECK_TICKS) {
            return;
        }
        // Malchir's name: in the game he stays a "mage" to the end, though
        // "Malchir the Master Sorcerer" is known for him. Known here once
        // the Avatar has met him.
        if self.flag("malchirMet") != 0 && self.flag("malchirName") == 0 {
            let _ = self.set_flag("malchirName", 1);
            let _ = self.set_flag("malchirJob", 1);
        }
        if let Some(q) = self.sync_journal() {
            self.journal.pending = self.journal.pending.or(Some(q));
        }
        // announced only once the scene is over: while a script holds the
        // Avatar or a conversation is open, the message would sit on top
        // of the speech and be gone before it is read (passers-by talking
        // in town do not hold it back)
        let busy = self.stasis || self.ask.is_some();
        if busy {
            return;
        }
        let Some(q) = self.journal.pending.take() else { return };
        if self.options.journal {
            let text = if self.german() { format!("Tagebuch: {}", q.de) } else { format!("Journal: {}", q.en) };
            self.notify(&dos(&text));
        }
    }

    /// The text of the journal: the quests in the order they began, each
    /// with its entries in the order they were reached.
    pub fn journal_text(&self) -> String {
        let de = self.german();
        let mut text = String::new();
        let shown: Vec<(&Quest, &Stage)> = self
            .journal
            .order
            .iter()
            .filter_map(|k| QUESTS.iter().flat_map(|q| q.stages.iter().map(move |s| (q, s))).find(|(q, s)| key(q, s) == *k))
            .filter(|(_, s)| !s.when.contains('!') || self.holds(s.when))
            .collect();
        // each quest once, in the order the quests began; within a quest
        // its entries in the order they were reached
        let mut quests: Vec<&Quest> = Vec::new();
        for (q, _) in &shown {
            if !quests.iter().any(|q2| q2.en == q.en) {
                quests.push(q);
            }
        }
        for q in quests {
            text += &format!("{}*", if de { q.de } else { q.en }.to_uppercase());
            for (_, s) in shown.iter().filter(|(q2, _)| q2.en == q.en) {
                text += if de { s.de } else { s.en };
                text += "**";
            }
            if q.stages.iter().any(|d| d.when == q.done && self.journal.order.contains(&key(q, d))) {
                text += if de { "- Erledigt -**" } else { "- Completed -**" };
            }
        }
        if text.is_empty() {
            text = if de { "Noch keine Einträge." } else { "No entries yet." }.to_string();
        }
        dos(text.trim_end_matches('*'))
    }

    /// Opens the journal in the game's book.
    pub fn open_journal(&mut self) {
        self.sync_journal();
        let text = self.journal_text();
        self.read_text(0, &text, Look::Book);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_is_an_entry() {
        for q in QUESTS {
            assert!(q.done.is_empty() || q.stages.iter().any(|s| s.when == q.done), "{}: done is no entry", q.en);
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<String> = QUESTS.iter().flat_map(|q| q.stages.iter().map(move |s| key(q, s))).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }

    #[test]
    fn every_flag_is_known() {
        let Some(usecode) = crate::usecode::for_tests() else { return };
        // (the conditions of the puzzle spots are read the same way)
        let spots = crate::puzzles::SPOTS.iter().flat_map(|s| [s.glows_while, s.shown_while]);
        let quests = QUESTS.iter().flat_map(|q| q.stages.iter().map(|s| s.when).chain([q.done]));
        {
            for cond in quests.chain(spots).filter(|c| !c.is_empty()) {
                for name in cond.split(['|', '&']).map(|t| t.trim().trim_start_matches('!').trim()) {
                    let name = name.split('=').next().unwrap().trim();
                    if name.starts_with("has:") || name.starts_with("dead:") {
                        continue;
                    }
                    assert!(usecode.flag_at(name).is_some(), "unknown flag {name} in {cond}");
                }
            }
        }
    }
}
