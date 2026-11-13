//! The usecode virtual machine: cooperative processes running the game's
//! script bytecode on a small stack machine.
//!
//! Opcode semantics follow ScummVM's Ultima 8 engine (usecode/uc_machine.cpp,
//! (C) the ScummVM Team, GPL-3.0-or-later).

use crate::game::Game;
use crate::map::FLG_FLIPPED;
use crate::usecode::code_byte;

pub const PROC_TYPE_ALL: u16 = 6;
const STACK_SIZE: usize = 0x1000;

const SEG_STRING: u32 = 0x8000;
const SEG_OBJ: u32 = 0x8002;
const SEG_GLOBAL: u32 = 0x8003;

/// The state of the script machine: the scripts themselves and what
/// they work with (processes, strings, lists).
pub struct Scripts {
    pub usecode: crate::usecode::Usecode,
    pub procs: Vec<Process>,
    pub next_pid: u16,
    pub strings: crate::hash::HashMap<u16, String>,
    pub next_string: u16,
    pub lists: crate::hash::HashMap<u16, UcList>,
    pub next_list: u16,
    /// Intrinsics already reported as missing.
    pub warned: crate::hash::HashSet<u16>,
}

impl Scripts {
    /// Things that went to another map with the Avatar got new numbers:
    /// their scripts follow (the item they belong to, and the copy of it
    /// at the top of their stack that `this` reads).
    pub fn renumber_items(&mut self, new_ids: &crate::hash::HashMap<u16, u16>) {
        for p in self.procs.iter_mut().filter(|p| !p.terminated) {
            let Some(&new) = new_ids.get(&p.item) else { continue };
            let old = p.item;
            p.item = new;
            if p.stack.len() == STACK_SIZE && p.stack[STACK_SIZE - 2..] == old.to_le_bytes() {
                p.stack[STACK_SIZE - 2..].copy_from_slice(&new.to_le_bytes());
            }
        }
    }
}

/// PAGAN_CALLS=1: every call and engine function of the scripts is
/// written out (for finding where a script goes wrong).
fn trace_calls() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PAGAN_CALLS").is_some())
}

pub fn obj_ptr(id: u16) -> u32 {
    (SEG_OBJ << 16) | id as u32
}
fn str_ptr(id: u16) -> u32 {
    (SEG_STRING << 16) | id as u32
}
fn stack_ptr(pid: u16, offset: usize) -> u32 {
    ((pid as u32) << 16) | (offset as u32 & 0xFFFF)
}

/// A list of fixed-size elements (object ids, string ids, numbers).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct UcList {
    pub elem: usize,
    pub data: Vec<u8>,
}

impl UcList {
    pub fn len(&self) -> usize {
        self.data.len().checked_div(self.elem).unwrap_or(0)
    }
    pub fn get(&self, i: usize) -> &[u8] {
        &self.data[i * self.elem..(i + 1) * self.elem]
    }
    fn word(&self, i: usize) -> u16 {
        let e = self.get(i);
        u16::from_le_bytes([e[0], e.get(1).copied().unwrap_or(0)])
    }
    /// Sets element `index` (the list grows up to it), if `value` has the
    /// size of the elements and the index is sensible.
    fn set(&mut self, index: usize, value: &[u8]) {
        let size = self.elem;
        if value.len() != size || index >= 4096 {
            return;
        }
        if self.data.len() < (index + 1) * size {
            self.data.resize((index + 1) * size, 0);
        }
        self.data[index * size..(index + 1) * size].copy_from_slice(value);
    }
}

/// What a process does when it is its turn.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Usecode,
    /// Ends after this many ticks.
    Delay(i32),
    /// Ends when the actor has finished its current animation.
    AnimWait(u16),
    /// Ends when the player has picked an answer in a conversation.
    Ask,
    /// An actor walking somewhere.
    Pathfind(crate::effects::Pathfind),
    /// An item flying through the air.
    Hurl(crate::effects::Hurl),
    /// An effect animation playing in the world.
    Sprite(crate::magic::Sprite),
    /// An actor wandering about.
    Loiter(crate::path::Loiter),
    /// A fireball chasing someone.
    Fireball(crate::effects::Fireball),
    /// The fire left by a blow of Flame Sting.
    FlameSting(crate::effects::FlameSting),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Process {
    pub pid: u16,
    pub kind: Kind,
    pub class: u16,
    pub ip: u16,
    bp: usize,
    sp: usize,
    stack: Vec<u8>,
    temp32: u32,
    pub result: u32,
    pub suspended: bool,
    pub terminated: bool,
    pub term_deferred: bool,
    /// Item and type the process belongs to (for "kill all processes of").
    pub item: u16,
    pub ptype: u16,
    /// Processes waiting for this one to end.
    waiting: Vec<u16>,
    /// Strings and lists to free when the process ends: (id, type).
    free_on_term: Vec<(u16, u8)>,
}

impl Process {
    pub fn native(pid: u16, kind: Kind, item: u16, ptype: u16) -> Process {
        Process {
            pid,
            kind,
            class: 0xFFFF,
            ip: 0xFFFF,
            bp: 0,
            sp: 0,
            stack: Vec::new(),
            temp32: 0,
            result: 0,
            suspended: false,
            terminated: false,
            term_deferred: false,
            item,
            ptype,
            waiting: Vec::new(),
            free_on_term: Vec::new(),
        }
    }

    fn at(&self, off: usize, n: usize) -> &[u8] {
        static ZERO: [u8; 64] = [0; 64];
        self.stack.get(off..off + n).unwrap_or(&ZERO[..n.min(64)])
    }
    fn put(&mut self, off: usize, d: &[u8]) {
        if let Some(s) = self.stack.get_mut(off..off + d.len()) {
            s.copy_from_slice(d);
        }
    }
    fn get2(&self, off: usize) -> u16 {
        let b = self.at(off, 2);
        u16::from_le_bytes([b[0], b[1]])
    }
    fn get4(&self, off: usize) -> u32 {
        let b = self.at(off, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    fn push(&mut self, d: &[u8]) {
        self.sp = self.sp.saturating_sub(d.len());
        let sp = self.sp;
        self.put(sp, d);
    }
    fn push2(&mut self, v: u16) {
        self.push(&v.to_le_bytes());
    }
    fn push4(&mut self, v: u32) {
        self.push(&v.to_le_bytes());
    }
    fn push0(&mut self, n: usize) {
        self.sp = self.sp.saturating_sub(n);
        let sp = self.sp;
        if let Some(s) = self.stack.get_mut(sp..sp + n) {
            s.fill(0);
        }
    }
    fn pop2(&mut self) -> u16 {
        let v = self.get2(self.sp);
        self.sp += 2;
        v
    }
    fn pop4(&mut self) -> u32 {
        let v = self.get4(self.sp);
        self.sp += 4;
        v
    }
    /// Address of a local variable or argument.
    fn local(&self, off: i8) -> usize {
        (self.bp as i32 + off as i32).max(0) as usize
    }
    fn call(&mut self, class: u16, offset: u16) {
        self.push2(self.class);
        self.push2(self.ip);
        self.push2(self.bp as u16);
        self.class = class;
        self.ip = offset;
        self.bp = self.sp;
    }
}

impl Game {
    // ---- strings and lists -------------------------------------------

    pub fn new_string(&mut self, s: String) -> u16 {
        loop {
            self.scripts.next_string = if self.scripts.next_string >= 0xFFF0 { 1 } else { self.scripts.next_string + 1 };
            if !self.scripts.strings.contains_key(&self.scripts.next_string) {
                break;
            }
        }
        self.scripts.strings.insert(self.scripts.next_string, s);
        self.scripts.next_string
    }

    pub fn string(&self, id: u16) -> &str {
        self.scripts.strings.get(&id).map_or("", |s| s.as_str())
    }

    fn dup_string(&mut self, id: u16) -> u16 {
        let s = self.string(id).to_string();
        self.new_string(s)
    }

    pub fn new_list(&mut self, l: UcList) -> u16 {
        loop {
            self.scripts.next_list = if self.scripts.next_list >= 0xFFF0 { 1 } else { self.scripts.next_list + 1 };
            if !self.scripts.lists.contains_key(&self.scripts.next_list) {
                break;
            }
        }
        self.scripts.lists.insert(self.scripts.next_list, l);
        self.scripts.next_list
    }

    fn free_string_list(&mut self, id: u16) {
        if let Some(l) = self.scripts.lists.remove(&id) {
            for i in 0..l.len() {
                self.scripts.strings.remove(&l.word(i));
            }
        }
    }

    fn dup_string_list(&mut self, id: u16) -> u16 {
        let mut out = UcList { elem: 2, data: Vec::new() };
        if let Some(l) = self.scripts.lists.get(&id).cloned() {
            for i in 0..l.len() {
                let s = self.dup_string(l.word(i));
                out.data.extend_from_slice(&s.to_le_bytes());
            }
        }
        self.new_list(out)
    }

    fn string_in_list(&self, list: &UcList, s: &str) -> bool {
        (0..list.len()).any(|i| self.string(list.word(i)) == s)
    }

    /// Adds the strings of `src` that `dst` lacks to it; `src` is used up
    /// (its strings either move to `dst` or are freed).
    fn string_list_union(&mut self, src: &UcList, dst: &mut UcList) {
        for i in 0..src.len() {
            let sid = src.word(i);
            if self.string_in_list(dst, self.string(sid)) {
                self.scripts.strings.remove(&sid);
            } else {
                dst.data.extend_from_slice(&sid.to_le_bytes());
            }
        }
    }

    /// Takes the strings of `src` out of `dst`; `src` is used up and the
    /// strings taken out are freed.
    fn string_list_difference(&mut self, src: &UcList, dst: &mut UcList) {
        let mut keep = Vec::new();
        for i in 0..dst.len() {
            let did = dst.word(i);
            if self.string_in_list(src, self.string(did)) {
                self.scripts.strings.remove(&did);
            } else {
                keep.extend_from_slice(&did.to_le_bytes());
            }
        }
        dst.data = keep;
        for i in 0..src.len() {
            self.scripts.strings.remove(&src.word(i));
        }
    }

    // ---- pointers ------------------------------------------------------

    pub fn proc_index(&self, pid: u16) -> Option<usize> {
        self.scripts.procs.iter().position(|p| p.pid == pid && !p.terminated)
    }

    /// Reads `size` bytes through a script pointer.
    pub fn deref(&self, ptr: u32, size: usize) -> Option<Vec<u8>> {
        let (seg, off) = (ptr >> 16, (ptr & 0xFFFF) as usize);
        match seg {
            1..=0x7FFE => {
                let p = &self.scripts.procs[self.proc_index(seg as u16)?];
                Some(p.at(off, size).to_vec())
            }
            SEG_OBJ if size == 2 => Some((off as u16).to_le_bytes().to_vec()),
            SEG_GLOBAL if size <= 2 => {
                let v = self.scripts.usecode.global(off, size) as u16;
                Some(v.to_le_bytes()[..size].to_vec())
            }
            _ => None,
        }
    }

    /// Writes through a script pointer.
    pub fn assign(&mut self, ptr: u32, data: &[u8]) -> bool {
        let (seg, off) = (ptr >> 16, (ptr & 0xFFFF) as usize);
        match seg {
            1..=0x7FFE => match self.proc_index(seg as u16) {
                Some(i) => {
                    self.scripts.procs[i].put(off, data);
                    true
                }
                None => false,
            },
            SEG_GLOBAL if data.len() <= 2 => {
                let v = data.iter().rev().fold(0u32, |a, &b| (a << 8) | b as u32);
                self.scripts.usecode.set_global(off, data.len(), v);
                true
            }
            _ => false,
        }
    }

    /// The 16 bit value a pointer refers to: an object id or string id.
    pub fn ptr_to_object(&self, ptr: u32) -> u16 {
        let (seg, off) = (ptr >> 16, (ptr & 0xFFFF) as u16);
        match seg {
            1..=0x7FFE => self.deref(ptr, 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])),
            SEG_OBJ | SEG_STRING => off,
            SEG_GLOBAL => self.scripts.usecode.global(off as usize, 2) as u16,
            _ => 0,
        }
    }

    // ---- processes -----------------------------------------------------

    pub fn new_pid(&mut self) -> u16 {
        loop {
            self.scripts.next_pid = if self.scripts.next_pid >= 0x7F00 { 16 } else { self.scripts.next_pid + 1 };
            if !self.scripts.procs.iter().any(|p| p.pid == self.scripts.next_pid) {
                return self.scripts.next_pid;
            }
        }
    }

    pub fn add_native(&mut self, kind: Kind, item: u16, ptype: u16) -> u16 {
        let pid = self.new_pid();
        self.scripts.procs.push(Process::native(pid, kind, item, ptype));
        pid
    }

    /// Creates a usecode process. `this_ptr` is dereferenced into the new
    /// process' own stack, as the caller's copy may be gone by the time
    /// the process runs.
    pub fn spawn(&mut self, class: u16, offset: u16, this_ptr: u32, this_size: usize, args: &[u8]) -> u16 {
        let pid = self.new_pid();
        let mut p = Process::native(pid, Kind::Usecode, 0, 0);
        p.stack = vec![0; STACK_SIZE];
        p.sp = STACK_SIZE;
        let mut this_sp = 0;
        if this_ptr != 0 && this_size > 0 {
            let this = self.deref(this_ptr, this_size).unwrap_or_else(|| vec![0; this_size]);
            p.push(&this);
            this_sp = p.sp;
        }
        p.push(args);
        if this_sp != 0 {
            p.push4(stack_ptr(pid, this_sp));
        }
        p.call(class, offset);
        self.scripts.procs.push(p);
        pid
    }

    /// Starts the handler of an event (0 = look, 1 = use ...) for an item.
    /// Returns the pid, or 0 if the item has no such handler.
    /// The script class of an item: the main characters have their own
    /// (1024 + their number); people the game creates on the way (a guard
    /// in a scene, a monster) use the one of their shape, even when their
    /// number is below 256; eggs choose theirs by their quality.
    pub fn usecode_class(&self, it: &crate::map::Item) -> u32 {
        let family = self.data.infos.get(it.shape as usize).map_or(0, |si| si.family);
        if it.npc != 0 && it.id < 256 && self.scripts.usecode.code(it.id + 1024).is_some() {
            it.id as u32 + 1024
        } else if family == crate::shape::FAMILY_UNKEGG {
            it.quality as u32 + 0x47F
        } else {
            it.shape
        }
    }

    pub fn call_event(&mut self, item_id: u16, event: u32, args: &[u8]) -> u16 {
        // people run their scripts on other maps too (their daily schedule)
        let Some((_, it)) = self.actor_anywhere(item_id) else { return 0 };
        let class = self.usecode_class(&it);
        match self.scripts.usecode.event(class, event) {
            Some(offset) => self.spawn(class as u16, offset, obj_ptr(item_id), 2, args),
            None => 0,
        }
    }

    pub fn terminate(&mut self, idx: usize) {
        let p = &mut self.scripts.procs[idx];
        if p.terminated {
            return;
        }
        p.terminated = true;
        let (pid, result) = (p.pid, p.result);
        let waiting = std::mem::take(&mut p.waiting);
        let free = std::mem::take(&mut p.free_on_term);
        for (id, typ) in free {
            match typ {
                1 => {
                    self.scripts.strings.remove(&id);
                }
                2 => self.free_string_list(id),
                _ => {
                    self.scripts.lists.remove(&id);
                }
            }
        }
        for w in waiting {
            if let Some(i) = self.proc_index(w) {
                self.scripts.procs[i].result = result;
                self.scripts.procs[i].suspended = false;
            }
        }
        self.barks.retain(|b| b.pid != pid);
        if let Kind::Pathfind(pf) = self.scripts.procs[idx].kind {
            self.inputs.remove(&pf.actor);
            self.paths.remove(&pf.actor);
        }
        if self.ask.as_ref().is_some_and(|a| a.0 == pid) {
            self.ask = None;
        }
        if self.ui.reader.as_ref().is_some_and(|r| r.pid == pid) {
            self.ui.reader = None;
        }
        if self.ui.targeting == Some(pid) {
            self.ui.targeting = None;
        }
    }

    /// Ends a process with a result for whoever waits for it.
    pub fn end_process_with(&mut self, pid: u16, result: u32) {
        if let Some(idx) = self.proc_index(pid) {
            self.scripts.procs[idx].result = result;
            self.terminate(idx);
        }
    }

    /// Hands a sleeping script the return value of the engine function
    /// it called, and lets it run on.
    pub fn set_return_and_wake(&mut self, pid: u16, value: u32) {
        if let Some(idx) = self.proc_index(pid) {
            self.scripts.procs[idx].temp32 = value;
            self.scripts.procs[idx].suspended = false;
        }
    }

    pub fn kill_pid(&mut self, pid: u16) {
        if let Some(idx) = self.proc_index(pid) {
            self.terminate(idx);
        }
    }

    /// The player picks answer `n` (0-based) of the open conversation.
    pub fn answer(&mut self, n: usize) {
        let Some((pid, choices)) = self.ask.clone() else { return };
        let Some(choice) = choices.get(n) else { return };
        // (sleeping until a time of day: the bed's script gets a number)
        let choice = self.sleep_answer(n).unwrap_or_else(|| choice.clone());
        self.golem_order(pid, choices.len(), n);
        if let Some(idx) = self.proc_index(pid) {
            // the waiting script gets the chosen text as a string
            let id = self.new_string(choice);
            self.scripts.procs[idx].result = id as u32;
            self.terminate(idx);
        }
        self.ask = None;
        self.ask_pick = None;
    }

    /// The golem of the earth spell takes orders from a list of eight
    /// (follow, stop, move, open, attack, leave, grunt, nothing). Following
    /// goes on until "stop", so a later "move" fought with it and the golem
    /// went nowhere; here an order to move, open or attack ends the
    /// following first (an improvement over the original).
    fn golem_order(&mut self, ask_pid: u16, choices: usize, n: usize) {
        const FOLLOWING: u16 = 0x20A;
        let Some(golem) = self.proc_index(ask_pid).map(|i| self.scripts.procs[i].item) else { return };
        let is_golem = self.map().get(golem).is_some_and(|it| it.shape == crate::magic::GOLEM_SHAPE);
        if is_golem && choices == 8 && (2..=4).contains(&n) {
            self.kill_processes(golem, FOLLOWING);
        }
    }

    pub fn num_processes(&self, item: u16, ptype: u16) -> u32 {
        self.scripts.procs
            .iter()
            .filter(|p| !p.terminated && (item == 0 || p.item == item) && (ptype == PROC_TYPE_ALL || p.ptype == ptype))
            .count() as u32
    }

    pub fn kill_processes(&mut self, item: u16, ptype: u16) {
        for i in 0..self.scripts.procs.len() {
            let p = &self.scripts.procs[i];
            if p.item != 0 && (item == 0 || p.item == item) && (ptype == PROC_TYPE_ALL || p.ptype == ptype)
                && !p.terminated && !p.term_deferred
            {
                self.terminate(i);
            }
        }
    }

    /// Gives every process its turn for this tick.
    pub fn run_processes(&mut self) {
        let mut i = 0;
        while i < self.scripts.procs.len() {
            if self.scripts.procs[i].term_deferred && !self.scripts.procs[i].terminated {
                self.terminate(i);
            }
            if !self.scripts.procs[i].terminated && !self.scripts.procs[i].suspended {
                self.run_process(i);
            }
            i += 1;
        }
        // The scripts of things a script destroyed end with them (as in the
        // original, after the processes of this round): otherwise they went
        // on without them, and their sounds came from nowhere at full
        // volume (the crackling on the earth plane).
        let destroyed: Vec<u16> = self.just_destroyed.keys().copied().filter(|&id| id != 0 && self.map().index(id).is_none()).collect();
        for id in destroyed {
            self.kill_processes(id, PROC_TYPE_ALL);
        }
        self.scripts.procs.retain(|p| !p.terminated);
    }

    /// Takes a thing out of the world for good (not by a script: an effect
    /// that is over, a stack merged into another), and its scripts end with
    /// it, as in the original. The fire left by the flame sword went on
    /// crackling without it, everywhere at full volume.
    pub fn destroy_with_scripts(&mut self, idx: usize) {
        let id = self.map().items[idx].id;
        if id != 0 {
            self.kill_processes(id, PROC_TYPE_ALL);
        }
        self.map_mut().destroy(idx);
    }

    /// After loading: ends the scripts of things that are no longer there
    /// (left over in games saved before they ended with their thing).
    pub fn end_orphaned_processes(&mut self) {
        let orphans: Vec<u16> = self.scripts.procs.iter().map(|p| p.item).filter(|&id| id >= 256 && self.map().index(id).is_none()).collect();
        for id in orphans {
            self.kill_processes(id, PROC_TYPE_ALL);
        }
        self.scripts.procs.retain(|p| !p.terminated);
    }

    fn run_process(&mut self, idx: usize) {
        let mut kind = self.scripts.procs[idx].kind;
        let done = match &mut kind {
            Kind::Usecode => {
                self.exec(idx);
                return;
            }
            Kind::Delay(n) => {
                *n -= 1;
                *n <= 0
            }
            Kind::AnimWait(actor) => self.actors.get(actor).is_none_or(|m| !m.busy()),
            Kind::Ask => false,
            Kind::Pathfind(pf) => self.run_pathfind(pf),
            Kind::Hurl(h) => self.run_hurl(h),
            Kind::Sprite(s) => self.run_sprite(s),
            Kind::Loiter(l) => self.run_loiter(l),
            Kind::Fireball(f) => self.run_fireball(f),
            Kind::FlameSting(f) => self.run_flame_sting(f),
        };
        self.scripts.procs[idx].kind = kind;
        if done {
            self.terminate(idx);
        }
    }

    // ---- item searches for the "loop" opcodes --------------------------

    fn loop_script_matches(&self, script: &[u8], it: &crate::map::Item) -> bool {
        let mut st: Vec<u16> = vec![1];
        let family = self.data.infos.get(it.shape as usize).map_or(0, |si| si.family) as u16;
        let mut i = 0;
        let word = |i: usize| script.get(i + 1).copied().unwrap_or(0) as u16 | ((script.get(i + 2).copied().unwrap_or(0) as u16) << 8);
        while i < script.len() {
            let t = script[i];
            let bin = |f: fn(u16, u16) -> bool, st: &mut Vec<u16>| {
                let a = st.pop().unwrap_or(0);
                let b = st.pop().unwrap_or(0);
                st.push(f(b, a) as u16);
            };
            match t {
                0 => st.push(0),
                1 => st.push(1),
                b'$' => return st.pop().unwrap_or(0) != 0,
                b'%' => {
                    st.push(word(i));
                    i += 2;
                }
                b'&' => bin(|b, a| a != 0 && b != 0, &mut st),
                b'+' => bin(|b, a| a != 0 || b != 0, &mut st),
                b'!' => {
                    let a = st.pop().unwrap_or(0);
                    st.push((a == 0) as u16);
                }
                b'?' => st.push(it.flags),
                b'*' => st.push(it.quality),
                b'#' => st.push(it.npc),
                b'=' => bin(|b, a| a == b, &mut st),
                b'>' => bin(|b, a| b > a, &mut st),
                b'<' => bin(|b, a| b < a, &mut st),
                b']' => bin(|b, a| b >= a, &mut st),
                b'[' => bin(|b, a| b <= a, &mut st),
                b':' => st.push(family),
                b'@' => st.push(it.shape as u16),
                b'`' => st.push(it.frame as u16),
                // shape / frame is one of n values
                b'A'..=b'Z' | b'a'..=b'z' => {
                    let (count, val) = if t <= b'Z' { (t - b'@', it.shape as u16) } else { (t - b'`', it.frame as u16) };
                    let mut found = false;
                    for _ in 0..count {
                        found |= word(i) == val;
                        i += 2;
                    }
                    st.push(found as u16);
                }
                _ => return false,
            }
            i += 1;
        }
        st.pop().unwrap_or(0) != 0
    }

    /// Ids of the items selected by a "loop" opcode.
    fn search_items(&self, kind: u8, a: u16, b: u16, script: &[u8]) -> Vec<u16> {
        match kind {
            // area around an item; 3 also searches everything inside the
            // things there (containers, and what people carry), as ScummVM
            2 | 3 => self.search_area(a, b as i32, kind == 3, script),
            // contents of a container (5: and of the containers in it)
            4 | 5 => self.search_contents(b, kind == 5, script),
            // items resting on / under another item
            _ => self.search_touching(a, b, script),
        }
    }

    /// The items whose footpad touches the square `range` around item `a`
    /// (`inside`: and everything inside them).
    fn search_area(&self, a: u16, range: i32, inside: bool, script: &[u8]) -> Vec<u16> {
        let map = self.map();
        let Some(centre) = map.get(a).copied() else { return Vec::new() };
        // a thing in a container searches around the outermost
        // container (a trapped book in a bag in a chest); one taken
        // out of the world for a while, around where it was (the
        // blackrock looks for the egg of Pyros' scene around the
        // Avatar, from whose pack it came)
        let base = if centre.parent == crate::map::ETHEREAL {
            self.ethereal.iter().rev().find(|e| e.0 == centre.id).and_then(|&(_, prev)| map.get(prev).copied()).unwrap_or(centre)
        } else {
            centre
        };
        let (cx, cy, _) = self.abs_pos(&base);
        let centre = crate::map::Item { x: cx, y: cy, ..centre };
        // around a person the square reaches `range` beyond its
        // footpad (Bane finds the candle in front of his feet);
        // around a thing from its point (the clearing egg in the
        // catacombs leaves the big floor tiles at its edge)
        let (cxd, cyd, _) = if centre.is_actor() {
            self.data.infos.get(centre.shape as usize).map_or((0, 0, 0), |si| si.footpad(centre.flags & FLG_FLIPPED != 0))
        } else {
            (0, 0, 0)
        };
        let mut out = Vec::new();
        let mut children: crate::hash::HashMap<u16, Vec<usize>> = crate::hash::HashMap::default();
        if inside {
            for (i, it) in map.items.iter().enumerate() {
                if !it.gone && it.parent != 0 && it.id != 0 {
                    children.entry(it.parent).or_default().push(i);
                }
            }
        }
        for i in map.items_near(centre.x, centre.y, range + 512) {
            let it = &map.items[i];
            let Some(si) = self.data.infos.get(it.shape as usize) else { continue };
            let (xd, yd, _) = si.footpad(it.flags & FLG_FLIPPED != 0);
            // the item's footpad has to touch the search square
            if it.x - xd >= centre.x + range || it.x <= centre.x - cxd - range
                || it.y - yd >= centre.y + range || it.y <= centre.y - cyd - range
            {
                continue;
            }
            if it.id != 0 && self.loop_script_matches(script, it) {
                out.push(it.id);
            }
            if inside && it.id != 0 {
                self.search_inside(it.id, &children, script, &mut out);
            }
        }
        out
    }

    /// What is inside `root` at any depth (`children`: the items in each
    /// container), as far as `script` selects it.
    fn search_inside(&self, root: u16, children: &crate::hash::HashMap<u16, Vec<usize>>, script: &[u8], out: &mut Vec<u16>) {
        let map = self.map();
        let mut open = vec![root];
        while let Some(c) = open.pop() {
            for &k in children.get(&c).map_or(&[][..], |v| &v[..]) {
                let inner = &map.items[k];
                if self.loop_script_matches(script, inner) {
                    out.push(inner.id);
                }
                open.push(inner.id);
            }
        }
    }

    /// The contents of container `b` (`deep`: also of the containers in it).
    fn search_contents(&self, b: u16, deep: bool, script: &[u8]) -> Vec<u16> {
        let map = self.map();
        let mut out = Vec::new();
        let mut open = vec![b];
        while let Some(c) = open.pop() {
            for it in map.items.iter().filter(|it| !it.gone && it.parent == c && c != 0) {
                if self.loop_script_matches(script, it) {
                    out.push(it.id);
                }
                if deep {
                    open.push(it.id);
                }
            }
        }
        out
    }

    /// The items resting on item `a` (or under item `b`).
    fn search_touching(&self, a: u16, b: u16, script: &[u8]) -> Vec<u16> {
        let map = self.map();
        let above = a != 0xFFFF;
        let Some(base) = map.get(if b != 0xFFFF { b } else { a }) else { return Vec::new() };
        let Some(bi) = self.data.infos.get(base.shape as usize) else { return Vec::new() };
        let bb = crate::map::WorldBox::of(base, bi);
        let mut out = Vec::new();
        for i in map.items_near(base.x, base.y, 1024) {
            let it = &map.items[i];
            let Some(si) = self.data.infos.get(it.shape as usize) else { continue };
            let ib = crate::map::WorldBox::of(it, si);
            let touching = if above { ib.z == bb.z + bb.zd } else { ib.z + ib.zd == bb.z };
            if it.id != base.id && it.id != 0 && touching && ib.overlaps_xy(&bb) && self.loop_script_matches(script, it) {
                out.push(it.id);
            }
        }
        out
    }

    // ---- the interpreter -----------------------------------------------

    /// Opcode 0x70: the items selected by search `kind` (with the values
    /// `a`, `b` from the stack and the selecting script after them) go into
    /// a list, and the loop's frame is laid on the stack: padding, script,
    /// its size, the loop variable, the index and the list.
    fn start_item_loop(&mut self, idx: usize, var: i8, script_size: usize, kind: u8) {
        let p = &mut self.scripts.procs[idx];
        let a = p.pop2();
        let b = p.pop2();
        let sp = p.sp;
        let script = p.at(sp, script_size.min(32)).to_vec();
        p.sp += script_size;
        let ids = self.search_items(kind, a, b, &script);
        let list = self.new_list(UcList { elem: 2, data: ids.iter().flat_map(|i| i.to_le_bytes()).collect() });
        // the scripts free a fixed amount of stack after the loop
        let frame: usize = match kind {
            2 | 3 => 0x34,
            4 => 0x28,
            5 => 0x2A,
            _ => 0x3D,
        };
        let p = &mut self.scripts.procs[idx];
        p.push0(frame.saturating_sub(script_size + 8));
        p.push(&script);
        p.push2(script_size as u16);
        p.push2(var as i16 as u16);
        p.push2(0);
        p.push2(list);
    }

    /// The next item of an item loop that still exists goes into the loop
    /// variable. False at the end (the list is freed then).
    fn next_loop_item(&mut self, idx: usize) -> bool {
        let p = &self.scripts.procs[idx];
        let sp = p.sp;
        let list = p.get2(sp);
        let mut index = p.get2(sp + 2) as usize;
        let var = p.get2(sp + 4) as i16 as i8;
        let ids = self.scripts.lists.get(&list).cloned().unwrap_or(UcList { elem: 2, data: Vec::new() });
        while index < ids.len() {
            let id = ids.word(index);
            index += 1;
            if self.map().get(id).is_some() {
                let p = &mut self.scripts.procs[idx];
                let a = p.local(var);
                p.put(a, &id.to_le_bytes());
                p.put(sp + 2, &(index as u16).to_le_bytes());
                return true;
            }
        }
        self.scripts.lists.remove(&list);
        false
    }

    /// An inline body works on the local variables of the function it
    /// stands in (it starts without any of its own): process `child` gets a
    /// copy of those of `parent`, at the same place below its frame.
    /// (Pyros' daemons are told to attack the Avatar this way; without the
    /// copy the order went to nobody.)
    fn copy_locals(&mut self, parent: usize, child: usize) {
        let p = &self.scripts.procs[parent];
        if p.bp <= p.sp {
            return;
        }
        let frame = p.stack[p.sp..p.bp].to_vec();
        let c = &mut self.scripts.procs[child];
        if c.bp >= frame.len() {
            let start = c.bp - frame.len();
            c.stack[start..c.bp].copy_from_slice(&frame);
            c.sp = start;
        }
    }

    /// Runs a usecode process until it suspends, waits or ends.
    pub fn exec(&mut self, idx: usize) {
        let Some(mut code) = self.scripts.usecode.code(self.scripts.procs[idx].class) else {
            self.terminate(idx);
            return;
        };
        let mut ip = self.scripts.procs[idx].ip as usize;
        let mut cede = false;
        let mut go_until_cede = false;
        let mut error: Option<String> = None;

        macro_rules! byte {
            () => {{
                let b = code_byte(&code, ip);
                ip += 1;
                match b {
                    Some(b) => b,
                    None => {
                        error = Some("ran past the end of the class".into());
                        break;
                    }
                }
            }};
        }
        macro_rules! word {
            () => {{
                let lo = byte!() as u16;
                lo | ((byte!() as u16) << 8)
            }};
        }
        macro_rules! p {
            () => {
                self.scripts.procs[idx]
            };
        }
        macro_rules! bin16 {
            ($f:expr) => {{
                let a = p!().pop2() as i16;
                let b = p!().pop2() as i16;
                let f: fn(i16, i16) -> u16 = $f;
                p!().push2(f(b, a));
            }};
        }
        macro_rules! bin32 {
            ($f:expr) => {{
                let a = p!().pop4() as i32;
                let b = p!().pop4() as i32;
                let f: fn(i32, i32) -> u32 = $f;
                p!().push4(f(b, a));
            }};
        }
        macro_rules! cmp32 {
            ($f:expr) => {{
                let a = p!().pop4() as i32;
                let b = p!().pop4() as i32;
                let f: fn(i32, i32) -> bool = $f;
                p!().push2(f(b, a) as u16);
            }};
        }

        let mut steps = 0u32;
        while !cede && !p!().terminated && !p!().term_deferred {
            steps += 1;
            if steps > 2_000_000 {
                error = Some("endless loop".into());
                break;
            }
            let op = byte!();
            match op {
                // ---- pop into variables
                0x00 => {
                    let off = byte!() as i8;
                    let v = p!().pop2();
                    let a = p!().local(off);
                    p!().put(a, &[v as u8]);
                }
                0x01 => {
                    let off = byte!() as i8;
                    let v = p!().pop2();
                    let a = p!().local(off);
                    p!().put(a, &v.to_le_bytes());
                }
                0x02 => {
                    let off = byte!() as i8;
                    let v = p!().pop4();
                    let a = p!().local(off);
                    p!().put(a, &v.to_le_bytes());
                }
                0x03 => {
                    let off = byte!() as i8;
                    let size = byte!() as usize;
                    let sp = p!().sp;
                    let d = p!().at(sp, size).to_vec();
                    p!().sp += size;
                    let a = p!().local(off);
                    p!().put(a, &d);
                }
                0x08 => {
                    let v = p!().pop4();
                    p!().result = v;
                }
                0x09 => {
                    // assign to list element
                    let off = byte!() as i8;
                    let size = byte!() as usize;
                    let slist = byte!() != 0;
                    let size = if slist { 2 } else { size };
                    let index = p!().pop2().wrapping_sub(1) as usize;
                    let a = p!().local(off);
                    let id = p!().get2(a);
                    let sp = p!().sp;
                    let d = p!().at(sp, size).to_vec();
                    p!().sp += size;
                    if let Some(l) = self.scripts.lists.get_mut(&id) {
                        l.set(index, &d);
                    }
                }

                // ---- push constants
                0x0A => {
                    let v = byte!() as i8 as i16 as u16;
                    p!().push2(v);
                }
                0x0B => {
                    let v = word!();
                    p!().push2(v);
                }
                0x0C => {
                    let lo = word!() as u32;
                    let hi = word!() as u32;
                    p!().push4(lo | (hi << 16));
                }
                0x0D => {
                    let len = word!() as usize;
                    let mut s = String::with_capacity(len);
                    for _ in 0..len {
                        s.push(byte!() as char);
                    }
                    let _nul = byte!();
                    let id = self.new_string(s);
                    p!().push2(id);
                }
                0x0E => {
                    // build a list from the top `count` stack entries
                    let elem = byte!() as usize;
                    let count = byte!() as usize;
                    let sp = p!().sp;
                    let mut data = Vec::with_capacity(elem * count);
                    for i in 0..count {
                        data.extend_from_slice(p!().at(sp + elem * (count - 1 - i), elem));
                    }
                    p!().sp += elem * count;
                    let id = self.new_list(UcList { elem, data });
                    p!().push2(id);
                }

                // ---- calls
                0x0F => {
                    let nargs = byte!() as usize;
                    let func = word!();
                    let sp = p!().sp;
                    let args = p!().at(sp, nargs).to_vec();
                    p!().ip = ip as u16;
                    let teleport_before = self.pending_teleport.is_some();
                    let r = self.intrinsic(idx, func, &args);
                    p!().temp32 = r;
                    // the Avatar was sent to another map: in the original the
                    // map changes at once, and the script goes on over there
                    // (Malchir looks for the Great Pentacle next to him and
                    // starts the ceremony, which fades in again). Here the map
                    // changes at the end of the tick, so the script waits for it.
                    if !teleport_before && self.pending_teleport.is_some_and(|(map, _)| map != self.cur_map) {
                        go_until_cede = false;
                        cede = true;
                    }
                    if trace_calls() {
                        let name = crate::intrinsic_names::NAMES.get(func as usize).copied().unwrap_or("?");
                        eprintln!("trace {:04X}:{:04X} intrinsic {func:02X} {name} args {args:02X?} -> {r} [pid {} item {}]", p!().class, ip.saturating_sub(4), p!().pid, p!().item);
                    }
                }
                0x11 => {
                    let class = word!();
                    let offset = word!();
                    if trace_calls() {
                        eprintln!("trace {:04X}:{:04X} calls {class:04X}:{offset:04X}", p!().class, ip.saturating_sub(5));
                    }
                    p!().ip = ip as u16;
                    p!().call(class, offset);
                    match self.scripts.usecode.code(class) {
                        Some(c) => code = c,
                        None => {
                            error = Some(format!("call into missing class {class:04X}"));
                            break;
                        }
                    }
                    ip = offset as usize;
                }
                0x12 => {
                    let v = p!().pop2();
                    p!().temp32 = v as u32;
                }
                0x13 => {
                    let v = p!().pop4();
                    p!().temp32 = v;
                }

                // ---- arithmetic
                0x14 => bin16!(|b, a| b.wrapping_add(a) as u16),
                0x15 => bin32!(|b, a| b.wrapping_add(a) as u32),
                0x16 => {
                    let a = p!().pop2();
                    let b = p!().pop2();
                    let tail = self.scripts.strings.remove(&a).unwrap_or_default();
                    self.scripts.strings.entry(b).or_default().push_str(&tail);
                    p!().push2(b);
                }
                0x17 => {
                    let a = p!().pop2();
                    let b = p!().pop2();
                    let r = match (self.scripts.lists.remove(&a), self.scripts.lists.get_mut(&b)) {
                        (Some(la), Some(lb)) => {
                            if la.elem == lb.elem {
                                lb.data.extend_from_slice(&la.data);
                            }
                            b
                        }
                        (Some(la), None) => {
                            self.scripts.lists.insert(a, la);
                            a
                        }
                        (None, Some(_)) => b,
                        (None, None) => 0,
                    };
                    p!().push2(r);
                }
                0x19 | 0x1A => {
                    // union / difference of string lists
                    let _elem = byte!();
                    let a = p!().pop2();
                    let b = p!().pop2();
                    if let (Some(src), Some(mut dst)) = (self.scripts.lists.remove(&a), self.scripts.lists.remove(&b)) {
                        if op == 0x19 {
                            self.string_list_union(&src, &mut dst);
                        } else {
                            self.string_list_difference(&src, &mut dst);
                        }
                        self.scripts.lists.insert(b, dst);
                    }
                    p!().push2(b);
                }
                0x1B => {
                    let _elem = byte!();
                    let a = p!().pop2();
                    let b = p!().pop2();
                    if let (Some(src), Some(dst)) = (self.scripts.lists.remove(&a), self.scripts.lists.get_mut(&b)) {
                        if src.elem == dst.elem && dst.elem > 0 {
                            let keep: Vec<u8> = (0..dst.len())
                                .filter(|&i| !(0..src.len()).any(|j| src.get(j) == dst.get(i)))
                                .flat_map(|i| dst.get(i).to_vec())
                                .collect();
                            dst.data = keep;
                        }
                    }
                    p!().push2(b);
                }
                0x1C => bin16!(|b, a| b.wrapping_sub(a) as u16),
                0x1D => bin32!(|b, a| b.wrapping_sub(a) as u32),
                0x1E => bin16!(|b, a| b.wrapping_mul(a) as u16),
                0x1F => bin32!(|b, a| b.wrapping_mul(a) as u32),
                0x20 => bin16!(|b, a| if a != 0 { b.wrapping_div(a) as u16 } else { 0 }),
                0x21 => bin32!(|b, a| if a != 0 { b.wrapping_div(a) as u32 } else { 0 }),
                0x22 => bin16!(|b, a| if a != 0 { b.wrapping_rem(a) as u16 } else { 0 }),
                0x23 => bin32!(|b, a| if a != 0 { b.wrapping_rem(a) as u32 } else { 0 }),
                0x24 => bin16!(|b, a| (a == b) as u16),
                0x25 => cmp32!(|b, a| a == b),
                0x26 => {
                    let a = p!().pop2();
                    let b = p!().pop2();
                    let eq = self.string(a) == self.string(b);
                    self.scripts.strings.remove(&a);
                    self.scripts.strings.remove(&b);
                    p!().push2(eq as u16);
                }
                0x28 => bin16!(|b, a| (b < a) as u16),
                0x29 => cmp32!(|b, a| b < a),
                0x2A => bin16!(|b, a| (b <= a) as u16),
                0x2B => cmp32!(|b, a| b <= a),
                0x2C => bin16!(|b, a| (b > a) as u16),
                0x2D => cmp32!(|b, a| b > a),
                0x2E => bin16!(|b, a| (b >= a) as u16),
                0x2F => cmp32!(|b, a| b >= a),
                0x30 => {
                    let v = p!().pop2();
                    p!().push2((v == 0) as u16);
                }
                0x31 => {
                    let v = p!().pop4();
                    p!().push2((v == 0) as u16);
                }
                0x32 => bin16!(|b, a| (a != 0 && b != 0) as u16),
                0x33 => cmp32!(|b, a| a != 0 && b != 0),
                0x34 => bin16!(|b, a| (a != 0 || b != 0) as u16),
                0x35 => cmp32!(|b, a| a != 0 || b != 0),
                0x36 => bin16!(|b, a| (a != b) as u16),
                0x37 => cmp32!(|b, a| a != b),
                0x38 => {
                    // is the value in the list?
                    let elem = byte!() as usize;
                    let slist = byte!() != 0;
                    let id = p!().pop2();
                    let list = self.scripts.lists.get(&id).cloned();
                    let found = if slist {
                        let s = p!().pop2();
                        let found = list.as_ref().is_some_and(|l| self.string_in_list(l, self.string(s)));
                        self.scripts.strings.remove(&s);
                        self.free_string_list(id);
                        found
                    } else {
                        let sp = p!().sp;
                        let v = p!().at(sp, elem).to_vec();
                        p!().sp += elem;
                        self.scripts.lists.remove(&id);
                        list.is_some_and(|l| l.elem == elem && (0..l.len()).any(|i| l.get(i) == &v[..]))
                    };
                    p!().push2(found as u16);
                }
                0x39 => bin16!(|b, a| (a & b) as u16),
                0x3A => bin16!(|b, a| (a | b) as u16),
                0x3B => {
                    let v = p!().pop2();
                    p!().push2(!v);
                }
                // shifts: the value is on top, the count below it
                0x3C => bin16!(|count, v| v.wrapping_shl(count as u32) as u16),
                0x3D => bin16!(|count, v| v.wrapping_shr(count as u32) as u16),

                // ---- push variables
                0x3E => {
                    let a = p!().local(byte!() as i8);
                    let v = p!().at(a, 1)[0] as i8 as i16 as u16;
                    p!().push2(v);
                }
                0x3F => {
                    let a = p!().local(byte!() as i8);
                    let v = p!().get2(a);
                    p!().push2(v);
                }
                0x40 => {
                    let a = p!().local(byte!() as i8);
                    let v = p!().get4(a);
                    p!().push4(v);
                }
                0x41 => {
                    let a = p!().local(byte!() as i8);
                    let id = p!().get2(a);
                    let copy = self.dup_string(id);
                    p!().push2(copy);
                }
                0x42 => {
                    let a = p!().local(byte!() as i8);
                    let elem = byte!() as usize;
                    let id = p!().get2(a);
                    let copy = self.scripts.lists.get(&id).cloned().unwrap_or(UcList { elem, data: Vec::new() });
                    let copy = self.new_list(copy);
                    p!().push2(copy);
                }
                0x43 => {
                    let a = p!().local(byte!() as i8);
                    let id = p!().get2(a);
                    let copy = self.dup_string_list(id);
                    p!().push2(copy);
                }
                0x44 => {
                    // list element (1-based index)
                    let elem = byte!() as usize;
                    let slist = byte!() != 0;
                    let index = p!().pop2().wrapping_sub(1) as usize;
                    let id = p!().pop2();
                    match self.scripts.lists.get(&id).cloned() {
                        Some(l) if index < l.len() => {
                            if slist {
                                let s = self.dup_string(l.word(index));
                                p!().push2(s);
                            } else {
                                let e = l.get(index).to_vec();
                                p!().push(&e);
                            }
                        }
                        _ => p!().push0(if slist { 2 } else { elem }),
                    }
                }
                0x45 => {
                    let a = p!().local(byte!() as i8);
                    let size = byte!() as usize;
                    let d = p!().at(a, size).to_vec();
                    p!().push(&d);
                }
                0x4B => {
                    let a = p!().local(byte!() as i8);
                    let pid = p!().pid;
                    p!().push4(stack_ptr(pid, a));
                }
                0x4C => {
                    let size = byte!() as usize;
                    let ptr = p!().pop4();
                    let d = self.deref(ptr, size).unwrap_or_else(|| vec![0; size]);
                    p!().push(&d);
                }
                0x4D => {
                    let size = byte!() as usize;
                    let ptr = p!().pop4();
                    let sp = p!().sp;
                    let d = p!().at(sp, size).to_vec();
                    p!().sp += size;
                    self.assign(ptr, &d);
                }
                0x4E => {
                    let pos = word!() as usize;
                    let bits = byte!() as usize;
                    let v = self.scripts.usecode.global(pos, bits);
                    p!().push2(v as u16);
                }
                0x4F => {
                    let pos = word!() as usize;
                    let bits = byte!() as usize;
                    let v = p!().pop2();
                    self.scripts.usecode.set_global(pos, bits, v as u32);
                }

                // ---- control flow
                0x50 => {
                    let pr = &mut p!();
                    pr.sp = pr.bp;
                    pr.bp = pr.pop2() as usize;
                    pr.ip = pr.pop2();
                    pr.class = pr.pop2();
                    if pr.ip == 0xFFFF && pr.class == 0xFFFF {
                        pr.term_deferred = true;
                        break;
                    }
                    ip = pr.ip as usize;
                    let class = pr.class;
                    match self.scripts.usecode.code(class) {
                        Some(c) => code = c,
                        None => {
                            error = Some("return into missing class".into());
                            break;
                        }
                    }
                }
                0x51 => {
                    let rel = word!() as i16;
                    if p!().pop2() == 0 {
                        ip = (ip as i32 + rel as i32) as usize & 0xFFFF;
                    }
                }
                0x52 => {
                    let rel = word!() as i16;
                    ip = (ip as i32 + rel as i32) as usize & 0xFFFF;
                }
                0x53 => {
                    go_until_cede = false;
                    cede = true;
                }
                0x54 => {
                    // "implies": the process below waits for the one on top
                    let _ = word!();
                    let a = p!().pop2();
                    let b = p!().pop2();
                    p!().push2(a);
                    if let (Some(ia), Some(ib)) = (self.proc_index(a), self.proc_index(b)) {
                        if ia != ib {
                            self.scripts.procs[ia].waiting.push(b);
                            self.scripts.procs[ib].suspended = true;
                        }
                        go_until_cede = true;
                    }
                }
                0x57 => {
                    let nargs = byte!() as usize;
                    let this_size = byte!() as usize;
                    let class = word!();
                    let offset = word!();
                    let this_ptr = p!().pop4();
                    let sp = p!().sp;
                    let args = p!().at(sp, nargs).to_vec();
                    p!().ip = ip as u16;
                    let pid = self.spawn(class, offset, this_ptr, this_size, &args);
                    if trace_calls() {
                        eprintln!("trace {:04X}:{:04X} spawns {class:04X}:{offset:04X} pid {pid} [pid {}]", p!().class, ip, p!().pid);
                    }
                    // the new process gets to run right away
                    let child = self.scripts.procs.len() - 1;
                    self.exec(child);
                    p!().temp32 = pid as u32;
                }
                0x58 => {
                    let class = word!();
                    let offset = word!();
                    let delta = word!();
                    let this_size = byte!() as usize;
                    let _unknown = byte!();
                    let this_ptr = if this_size > 0 {
                        let a = p!().local(6);
                        p!().get4(a)
                    } else {
                        0
                    };
                    p!().ip = ip as u16;
                    let pid = self.spawn(class, offset.wrapping_add(delta), this_ptr, this_size, &[]);
                    if trace_calls() {
                        eprintln!("trace {:04X}:{:04X} spawns inline {class:04X}:{:04X} pid {pid} [pid {}]", p!().class, ip, offset.wrapping_add(delta), p!().pid);
                    }
                    let child = self.scripts.procs.len() - 1;
                    self.copy_locals(idx, child);
                    self.exec(child);
                    p!().push2(pid);
                }
                0x59 => {
                    let pid = p!().pid;
                    p!().push2(pid);
                }
                0x5A => {
                    let n = byte!() as usize;
                    p!().push0((n + 1) & !1);
                }
                0x5B => {
                    let _ = word!();
                }
                0x5C => {
                    for _ in 0..11 {
                        let _ = byte!();
                    }
                }
                0x5D => {
                    let v = p!().temp32 as u8 as u16;
                    p!().push2(v);
                }
                0x5E => {
                    let v = p!().temp32 as u16;
                    p!().push2(v);
                }
                0x5F => {
                    let v = p!().temp32;
                    p!().push4(v);
                }
                0x60 => {
                    let v = p!().pop2() as i16 as i32 as u32;
                    p!().push4(v);
                }
                0x61 => {
                    let v = p!().pop4() as u16;
                    p!().push2(v);
                }

                // ---- freeing strings and lists
                0x62..=0x64 => {
                    let a = p!().local(byte!() as i8);
                    let id = p!().get2(a);
                    self.free_value(op - 0x62, id);
                }
                0x65..=0x67 => {
                    let a = (p!().sp as i32 + byte!() as i8 as i32).max(0) as usize;
                    let id = p!().get2(a);
                    // 65 string, 66 list, 67 string list
                    self.free_value([0, 2, 1][(op - 0x65) as usize], id);
                }
                0x69 => {
                    let a = p!().local(byte!() as i8);
                    let id = p!().get2(a);
                    p!().push4(str_ptr(id));
                }
                0x6B => {
                    let id = p!().pop2();
                    p!().push4(str_ptr(id));
                }
                0x6C => {
                    // a string / list argument was created by another
                    // process: take a private copy
                    let a = p!().local(byte!() as i8);
                    let typ = byte!();
                    let id = p!().get2(a);
                    let copy = match typ {
                        1 => self.dup_string(id),
                        2 => self.dup_string_list(id),
                        _ => {
                            let l = self.scripts.lists.get(&id).cloned().unwrap_or(UcList { elem: 2, data: Vec::new() });
                            self.new_list(l)
                        }
                    };
                    p!().put(a, &copy.to_le_bytes());
                    p!().free_on_term.push((copy, typ));
                }
                0x6D => {
                    let v = p!().result;
                    p!().push4(v);
                }
                0x6E => {
                    let n = byte!() as i8 as i32;
                    p!().sp = (p!().sp as i32 - n).clamp(0, STACK_SIZE as i32) as usize;
                }
                0x6F => {
                    let off = byte!() as i8 as i32;
                    let a = (p!().sp as i32 - off).max(0) as usize;
                    let pid = p!().pid;
                    p!().push4(stack_ptr(pid, a));
                }

                // ---- loops over items
                0x70 | 0x73 => {
                    // 0x70 starts a loop over the items a search selects,
                    // both go on to the next one
                    if op == 0x70 {
                        let var = byte!() as i8;
                        let script_size = byte!() as usize;
                        let kind = byte!();
                        self.start_item_loop(idx, var, script_size, kind);
                    }
                    let found = self.next_loop_item(idx);
                    p!().push2(found as u16);
                }
                0x74 => {
                    let b = byte!();
                    p!().push(&[b]);
                }
                0x75 | 0x76 => {
                    // for each element of the list below the loop index
                    let var = byte!() as i8;
                    let elem = byte!() as usize;
                    let rel = word!() as i16;
                    let sp = p!().sp;
                    let index = p!().get2(sp);
                    let id = p!().get2(sp + 2);
                    let index = if index == 0xFFFF { 0 } else { index as usize + 1 };
                    let element = self.scripts.lists.get(&id).filter(|l| index < l.len() && l.elem == elem).map(|l| l.get(index).to_vec());
                    if let Some(e) = element {
                        p!().put(sp, &(index as u16).to_le_bytes());
                        let a = p!().local(var);
                        p!().put(a, &e);
                    } else {
                        // the end: the list goes, and so does the loop
                        if op == 0x75 {
                            self.scripts.lists.remove(&id);
                        } else {
                            self.free_string_list(id);
                        }
                        p!().sp += 4;
                        ip = (ip as i32 + rel as i32) as usize & 0xFFFF;
                    }
                }
                0x77 => {
                    let item = p!().pop2();
                    let ptype = p!().pop2();
                    p!().item = item;
                    p!().ptype = ptype;
                }
                0x78 => {
                    // only one process per (item, type)
                    let (item, ptype) = (p!().item, p!().ptype);
                    if trace_calls() {
                        let others: Vec<(u16, u16, u16, bool)> = self.scripts.procs.iter().map(|q| (q.pid, q.item, q.ptype, q.terminated)).collect();
                        eprintln!("trace exclude item {item} type {ptype}: {} processes {others:?}", self.num_processes(item, ptype));
                    }
                    if self.num_processes(item, ptype) > 1 {
                        p!().term_deferred = true;
                    }
                }
                _ => {
                    error = Some(format!("bad opcode {op:02X}"));
                    break;
                }
            }
            p!().ip = ip as u16;
            if p!().suspended && !go_until_cede {
                cede = true;
            }
        }

        if let Some(msg) = error {
            let p = &self.scripts.procs[idx];
            eprintln!("usecode: process {} killed at {:04X}:{:04X} (item {}): {msg}", p.pid, p.class, ip, p.item);
            self.scripts.procs[idx].term_deferred = true;
        }
    }

    /// kind: 0 = string, 1 = string list, 2 = list
    fn free_value(&mut self, kind: u8, id: u16) {
        match kind {
            0 => {
                self.scripts.strings.remove(&id);
            }
            1 => self.free_string_list(id),
            _ => {
                self.scripts.lists.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process() -> Process {
        let mut p = Process::native(20, Kind::Usecode, 0, 0);
        p.stack = vec![0; STACK_SIZE];
        p.sp = STACK_SIZE;
        p
    }

    /// What is pushed comes back in reverse order, low byte first.
    #[test]
    fn stack_push_and_pop() {
        let mut p = process();
        p.push2(0x1234);
        p.push4(0xDEAD_BEEF);
        p.push(&[1, 2, 3]);
        assert_eq!(p.sp, STACK_SIZE - 9);
        assert_eq!(p.at(p.sp, 3), &[1, 2, 3]);
        p.sp += 3;
        assert_eq!(p.pop4(), 0xDEAD_BEEF);
        assert_eq!(p.pop2(), 0x1234);
        assert_eq!(p.sp, STACK_SIZE);
        // low byte first, as the scripts read their values
        p.push2(0x0102);
        assert_eq!(p.at(p.sp, 2), &[0x02, 0x01]);
    }

    /// A script reading or writing outside its stack gets nothing and
    /// changes nothing: the machine does not fail on it.
    #[test]
    fn stack_out_of_range() {
        let mut p = process();
        // popping from the empty stack
        assert_eq!(p.pop2(), 0);
        assert_eq!(p.pop4(), 0);
        assert_eq!(p.get4(STACK_SIZE - 2), 0);
        p.put(STACK_SIZE - 1, &[9, 9]);
        assert_eq!(p.stack[STACK_SIZE - 1], 0);
        // pushing more than the stack holds
        p.sp = 1;
        p.push4(0xFFFF_FFFF);
        assert_eq!(p.sp, 0);
        p.push0(8);
        assert_eq!(p.sp, 0);
        // a process without a stack of its own (a walk, a delay)
        let mut native = Process::native(21, Kind::Delay(3), 0, 0);
        native.push2(7);
        assert_eq!(native.pop2(), 0);
    }

    /// A call notes where to go on afterwards and opens a new frame.
    #[test]
    fn call_keeps_the_way_back() {
        let mut p = process();
        (p.class, p.ip, p.bp) = (5, 0x40, STACK_SIZE);
        p.push2(77); // an argument
        p.call(9, 0x100);
        assert_eq!((p.class, p.ip), (9, 0x100));
        assert_eq!(p.bp, p.sp);
        // the frame: old frame pointer, where to go on, class, then the argument
        assert_eq!(p.get2(p.bp), STACK_SIZE as u16);
        assert_eq!(p.get2(p.bp + 2), 0x40);
        assert_eq!(p.get2(p.bp + 4), 5);
        assert_eq!(p.get2(p.local(6)), 77);
        // locals lie below the frame pointer; never before the stack's start
        assert_eq!(p.local(-2), p.bp - 2);
        p.bp = 1;
        assert_eq!(p.local(-8), 0);
    }

    #[test]
    fn pointers_carry_their_kind() {
        assert_eq!(obj_ptr(1) >> 16, SEG_OBJ);
        assert_eq!(obj_ptr(0x1234) & 0xFFFF, 0x1234);
        assert_eq!(str_ptr(9) >> 16, SEG_STRING);
        assert_eq!(stack_ptr(0x20, 0xFFE), 0x0020_0FFE);
    }

    #[test]
    fn list_elements() {
        let mut l = UcList { elem: 2, data: Vec::new() };
        assert_eq!(l.len(), 0);
        // setting beyond the end makes the list grow, filled with zero
        l.set(2, &[0x34, 0x12]);
        assert_eq!(l.len(), 3);
        assert_eq!((l.word(0), l.word(2)), (0, 0x1234));
        assert_eq!(l.get(2), &[0x34, 0x12]);
        // a value of the wrong size or a senseless index is left out
        l.set(0, &[1]);
        l.set(4096, &[1, 2]);
        assert_eq!(l.len(), 3);
        assert_eq!(l.word(0), 0);
        // elements of one byte read as a number too
        let bytes = UcList { elem: 1, data: vec![7, 8] };
        assert_eq!((bytes.len(), bytes.word(1)), (2, 8));
        // a list without an element size has no elements
        assert_eq!(UcList { elem: 0, data: vec![1, 2] }.len(), 0);
    }
}
