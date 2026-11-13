//! Saving and loading the state of a running game.
//!
//! This is the engine's own format (not the original U8SAVE files): the
//! whole changeable state, written with serde.  A savegame only fits the
//! language it was made with, because running scripts carry their texts.

use crate::actor::Mover;
use crate::combat::ActorState;
use crate::effects::CameraMode;
use crate::eggs::Destination;
use crate::game::{Bark, Game, AVATAR};
use crate::gumps::{Gump, Reader};
use crate::hash::{HashMap, HashSet};
use crate::map::{MapData, Npc, Rng};
use crate::vm::{Process, UcList};
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"PAGANSAV";
/// Raised whenever the layout of the saved data changes.
const FORMAT: u32 = 7;
/// The oldest format still read: its additions stand behind the body
/// without names or lengths (see `Appended::read_format_6`).
const OLDEST_FORMAT: u32 = 6;

/// Slot numbers: 0 is the quick save (F5 / F9), 9 is written whenever the
/// Avatar enters a new map, all others are the entries of the savegame
/// book (entry n is slot n up to 8, slot n + 1 from 9 on).
pub const QUICK_SLOT: usize = 0;
pub const AUTO_SLOT: usize = 9;

/// What the savegame book shows about a slot. Stored in front of the
/// game data so it can be read without loading the game.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SlotInfo {
    pub map: u32,
    pub game_seconds: u32,
}

/// Size of the encoded SlotInfo.
const INFO_LEN: usize = 8;

/// The body of a savegame. One list of fields makes both the form it is
/// written from (borrowed from the game) and the form it is read into:
/// the values stand in the file one after the other without names, so
/// the two must agree in every field and in their order.
macro_rules! body {
    ($($name:ident: $ty:ty,)*) => {
        #[derive(serde::Serialize)]
        struct SaveRef<'a> {
            $($name: &'a $ty,)*
        }

        #[derive(serde::Deserialize)]
        struct SaveData {
            $($name: $ty,)*
        }
    };
}

body! {
    language: char,
    cur_map: usize,
    tick: u32,
    rng: Rng,
    maps: HashMap<usize, MapData>,
    npcs: Vec<Npc>,
    actors: HashMap<u16, Mover>,
    npc_dirs: HashMap<u16, u8>,
    states: HashMap<u16, ActorState>,
    mana: i32,
    globals: Vec<u8>,
    procs: Vec<Process>,
    next_pid: u16,
    strings: HashMap<u16, String>,
    next_string: u16,
    lists: HashMap<u16, UcList>,
    next_list: u16,
    barks: Vec<Bark>,
    ask: Option<(u16, Vec<String>)>,
    reader: Option<Reader>,
    ethereal: Vec<(u16, u16)>,
    stasis: bool,
    pending_teleport: Option<(usize, Destination)>,
    just_teleported: bool,
    camera: CameraMode,
    gumps: Vec<Gump>,
    music: i32,
    time_offset: u32,
    sched_last_hour: u32,
    sched_next: u16,
    pal_transform: u8,
    inverted: bool,
    airwalk: bool,
}

/// The save last started in the background (see save_to_in_background).
static WRITING: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(None);

/// Waits until the saves written in the background are on disk.
pub fn wait_for_saves() {
    let last = WRITING.lock().ok().and_then(|mut w| w.take());
    if let Some(h) = last {
        let _ = h.join();
    }
}

/// Writes a savegame file: next to the old one first, so a failure cannot
/// destroy it. A save written in the background is waited for.
fn write_save(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    wait_for_saves();
    write_file(path, bytes)
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Appends a value to a savegame being written.
fn put<T: serde::Serialize + ?Sized>(bytes: &mut Vec<u8>, value: &T) -> Result<(), String> {
    bincode::serialize_into(bytes, value).map_err(|e| format!("cannot encode the game: {e}"))
}

/// Appends a named block to a savegame being written: its name, the
/// length of its data, the data.
fn put_block<T: serde::Serialize + ?Sized>(bytes: &mut Vec<u8>, name: &[u8; 4], value: &T) -> Result<(), String> {
    let mut data = Vec::new();
    put(&mut data, value)?;
    let len = u32::try_from(data.len()).map_err(|_| "cannot encode the game: a part is too large".to_string())?;
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&data);
    Ok(())
}

/// The named blocks behind the body of a savegame.
struct Blocks<'a>(Vec<([u8; 4], &'a [u8])>);

impl<'a> Blocks<'a> {
    fn read(mut rest: &'a [u8]) -> Result<Blocks<'a>, String> {
        let mut blocks = Vec::new();
        while !rest.is_empty() {
            let damaged = || "the savegame is damaged: it ends in the middle of a part".to_string();
            let (head, data) = rest.split_at_checked(8).ok_or_else(damaged)?;
            let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
            let (data, after) = data.split_at_checked(len).ok_or_else(damaged)?;
            blocks.push(([head[0], head[1], head[2], head[3]], data));
            rest = after;
        }
        Ok(Blocks(blocks))
    }

    /// The value of a block; None if the savegame has no such block (it
    /// was made before the block existed). A block that is there and
    /// cannot be read is an error: nothing is made up in its place.
    fn get<T: serde::de::DeserializeOwned>(&self, name: &[u8; 4]) -> Result<Option<T>, String> {
        let Some((_, data)) = self.0.iter().find(|(n, _)| n == name) else { return Ok(None) };
        bincode::deserialize(data).map(Some).map_err(|e| format!("the savegame is damaged (part \"{}\"): {e}", String::from_utf8_lossy(name)))
    }
}

/// What stands behind the body of a savegame: added by later versions,
/// so an older savegame lacks some of it.
struct Appended {
    /// The Avatar's name and the full health of the creatures.
    extra: Option<(String, HashMap<u16, i32>)>,
    vitals: crate::vitals::Vitals,
    journal: Vec<String>,
    look: Option<crate::gumps::Look>,
    changelings: HashMap<u16, crate::combat::Shift>,
    /// None: a savegame made before the fight states stayed with their map.
    map_states: Option<HashMap<usize, HashMap<u16, crate::combat::ActorState>>>,
    spells: Vec<String>,
    /// Creatures made by scripts, and the calmed ones.
    made: Option<(HashSet<u16>, HashSet<u16>)>,
    passersby: Vec<u16>,
    resurrections: HashMap<u16, u32>,
    searching: HashSet<u16>,
    revealed_path: HashMap<usize, HashMap<u16, u32>>,
    /// None: a savegame from before the guide was saved (the current
    /// target stays).
    guide_target: Option<Option<crate::guide::Target>>,
}

impl Appended {
    /// Appends everything to a savegame being written, as named blocks.
    fn write(game: &Game, bytes: &mut Vec<u8>) -> Result<(), String> {
        // the Avatar's name and what the creatures' health started at
        put_block(bytes, b"name", &(&game.avatar_name, &game.fights.full_hp))?;
        // healing, hunger, practice (vitals.rs)
        put_block(bytes, b"vitl", &game.vitals)?;
        // the order of the journal's entries (journal.rs)
        put_block(bytes, b"jrnl", &game.journal.order)?;
        // how an open book, scroll or the journal looks
        put_block(bytes, b"look", &game.ui.reader.as_ref().map(|r| r.look))?;
        // changelings in the middle of taking the Avatar's shape
        put_block(bytes, b"chng", &game.fights.changelings)?;
        // the fight states of the creatures of the other maps
        put_block(bytes, b"mpst", &game.fights.map_states)?;
        // the spells learned (spellnotes.rs)
        put_block(bytes, b"spel", &game.spell_notes.learned)?;
        // the creatures scripts made and those a script calmed
        put_block(bytes, b"made", &(&game.fights.script_made, &game.fights.calmed))?;
        // the passers-by (passersby.rs)
        put_block(bytes, b"pass", &game.passersby_ids())?;
        // killed creatures that will get up again (when), and those on
        // their way to where they last saw the Avatar
        put_block(bytes, b"rise", &(&game.fights.resurrections, &game.fights.searching))?;
        // the tiles of the invisible path shown so far (pathreveal.rs)
        put_block(bytes, b"path", &game.revealed_path)?;
        // the place the guide leads to (guide.rs)
        put_block(bytes, b"guid", &game.guide.target)?;
        Ok(())
    }

    fn read(rest: &[u8]) -> Result<Appended, String> {
        let b = Blocks::read(rest)?;
        let (resurrections, searching) = b.get(b"rise")?.unwrap_or_default();
        Ok(Appended {
            extra: b.get(b"name")?,
            vitals: b.get(b"vitl")?.unwrap_or_default(),
            journal: b.get(b"jrnl")?.unwrap_or_default(),
            look: b.get(b"look")?.flatten(),
            changelings: b.get(b"chng")?.unwrap_or_default(),
            map_states: b.get(b"mpst")?,
            spells: b.get(b"spel")?.unwrap_or_default(),
            made: b.get(b"made")?,
            passersby: b.get(b"pass")?.unwrap_or_default(),
            resurrections,
            searching,
            revealed_path: b.get(b"path")?.unwrap_or_default(),
            guide_target: b.get(b"guid")?,
        })
    }

    /// The same from a savegame of format 6, where these values stand one
    /// after the other without names: the ones at the end are missing in
    /// a savegame made before they were added.
    fn read_format_6(rest: &mut &[u8]) -> Appended {
        let extra = bincode::deserialize_from(&mut *rest).ok();
        let vitals = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let journal = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let look = bincode::deserialize_from(&mut *rest).ok().flatten();
        let changelings = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let map_states = bincode::deserialize_from(&mut *rest).ok();
        let spells = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let made = bincode::deserialize_from(&mut *rest).ok();
        let passersby = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let (resurrections, searching) = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let revealed_path = bincode::deserialize_from(&mut *rest).unwrap_or_default();
        let guide_target = bincode::deserialize_from(&mut *rest).ok();
        Appended { extra, vitals, journal, look, changelings, map_states, spells, made, passersby, resurrections, searching, revealed_path, guide_target }
    }
}

impl Game {
    fn state_is_dead(&self) -> bool {
        self.fights.states.get(&AVATAR).is_some_and(|s| s.dead)
    }

    /// The file of a savegame slot of this language.
    pub fn save_path(&self, slot: usize) -> Option<PathBuf> {
        // PAGAN_SAVE_DIR redirects savegames, so tests do not touch the player's
        let dir = match &crate::debug::env().save_dir {
            Some(d) => d.clone(),
            None => crate::user_dir()?,
        };
        Some(dir.join(match slot {
            QUICK_SLOT => format!("quicksave_{}.sav", self.language),
            AUTO_SLOT => format!("autosave_{}.sav", self.language),
            n => format!("save_{}_{n}.sav", self.language),
        }))
    }

    /// The slots of the savegame book that hold a game of this language
    /// (a file is there; it may be from another version).
    pub fn used_slots(&self) -> Vec<usize> {
        let Some(dir) = self.save_path(1).and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return Vec::new() };
        let prefix = format!("save_{}_", self.language);
        let mut slots: Vec<usize> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_prefix(&prefix)?.strip_suffix(".sav")?.parse().ok())
            .filter(|&n| n != QUICK_SLOT && n != AUTO_SLOT)
            .collect();
        slots.sort_unstable();
        slots
    }

    /// When a slot was last written (None: empty).
    pub fn slot_time(&self, slot: usize) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.save_path(slot)?).ok()?.modified().ok()
    }

    /// What is in a slot; None if it is empty or from another version.
    pub fn slot_info(&self, slot: usize) -> Option<SlotInfo> {
        use std::io::Read;
        let mut head = [0u8; 12 + INFO_LEN];
        std::fs::File::open(self.save_path(slot)?).ok()?.read_exact(&mut head).ok()?;
        if &head[..8] != MAGIC || !(OLDEST_FORMAT..=FORMAT).contains(&u32::from_le_bytes([head[8], head[9], head[10], head[11]])) {
            return None;
        }
        bincode::deserialize(&head[12..]).ok()
    }

    /// The name the player gave a slot (kept beside the savegame, so that
    /// the savegame itself stays as it is).
    pub fn slot_name(&self, slot: usize) -> Option<String> {
        let text = std::fs::read_to_string(self.save_path(slot)?.with_extension("name")).ok()?;
        let text = text.trim_end_matches(['\r', '\n']).to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Gives a slot a name (empty: none).
    pub fn set_slot_name(&self, slot: usize, name: &str) {
        let Some(path) = self.save_path(slot).map(|p| p.with_extension("name")) else { return };
        if name.trim().is_empty() {
            let _ = std::fs::remove_file(path);
        } else {
            let _ = std::fs::write(path, name);
        }
    }

    /// The quick save.
    pub fn save_game(&self) -> Result<PathBuf, String> {
        self.save_to(QUICK_SLOT)
    }

    pub fn load_game(&mut self) -> Result<(), String> {
        self.load_from(QUICK_SLOT)
    }

    pub fn save_to(&self, slot: usize) -> Result<PathBuf, String> {
        let path = self.save_path(slot).ok_or("no folder for savegames (APPDATA is not set)")?;
        let bytes = self.save_bytes()?;
        write_save(&path, &bytes)?;
        Ok(path)
    }

    /// As save_to, but the file is written in the background: the game
    /// does not wait for it (the autosave at a map change would make the
    /// picture stumble). Saves written this way go one after the other.
    pub fn save_to_in_background(&self, slot: usize) -> Result<(), String> {
        let path = self.save_path(slot).ok_or("no folder for savegames (APPDATA is not set)")?;
        let bytes = self.save_bytes()?;
        let mut last = WRITING.lock().map_err(|_| "save: lock".to_string())?;
        // one after the other: the new one waits for the one before
        let before = last.take();
        *last = Some(std::thread::spawn(move || {
            if let Some(h) = before {
                let _ = h.join();
            }
            if let Err(err) = write_file(&path, &bytes) {
                eprintln!("save: {err}");
            }
        }));
        Ok(())
    }

    /// The game encoded as a savegame.
    fn save_bytes(&self) -> Result<Vec<u8>, String> {
        if self.avatar_died.is_some() {
            return Err("cannot save while the Avatar is dying".into());
        }
        if self.ui.targeting.is_some() || self.ui.slider.is_some() {
            return Err("cannot save while a spell waits for input".into());
        }
        if self.state_is_dead() {
            return Err("cannot save: the Avatar is dead".into());
        }
        let globals = self.scripts.usecode.globals().to_vec();
        let music = self.audio.requested();
        let data = SaveRef {
            language: &self.language,
            cur_map: &self.cur_map,
            tick: &self.tick,
            rng: &self.rng,
            maps: &self.maps,
            npcs: &self.people.npcs,
            actors: &self.actors,
            npc_dirs: &self.people.npc_dirs,
            states: &self.fights.states,
            mana: &self.mana,
            globals: &globals,
            procs: &self.scripts.procs,
            next_pid: &self.scripts.next_pid,
            strings: &self.scripts.strings,
            next_string: &self.scripts.next_string,
            lists: &self.scripts.lists,
            next_list: &self.scripts.next_list,
            barks: &self.barks,
            ask: &self.ask,
            reader: &self.ui.reader,
            ethereal: &self.ethereal,
            stasis: &self.stasis,
            pending_teleport: &self.pending_teleport,
            just_teleported: &self.just_teleported,
            camera: &self.camera,
            gumps: &self.ui.gumps,
            music: &music,
            time_offset: &self.time_offset,
            sched_last_hour: &self.sched_last_hour,
            sched_next: &self.sched_next,
            pal_transform: &self.screen.pal_transform,
            inverted: &self.screen.inverted,
            airwalk: &self.airwalk,
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&FORMAT.to_le_bytes());
        let info = SlotInfo { map: self.cur_map as u32, game_seconds: self.game_seconds() };
        put(&mut bytes, &info)?;
        debug_assert_eq!(bytes.len(), 12 + INFO_LEN);
        put(&mut bytes, &data)?;
        // behind the body: what later versions added, as named blocks
        Appended::write(self, &mut bytes)?;
        Ok(bytes)
    }

    /// What is not in a savegame and belongs to the game left behind
    /// starts fresh after loading.
    fn reset_after_load(&mut self) {
        self.screen.quake = 0;
        self.screen.quake_by = 0;
        // (the walks are searched again; what was tried before belongs to
        // the game left behind)
        self.paths.clear();
        self.path_tried.clear();
        self.fights.approach_fails.clear();
        self.ui.targeting = None;
        self.ui.peace_target = None;
        self.ui.slider = None;

        // everything that only describes the moment of drawing starts fresh
        self.fast_items = self.map().items.iter().enumerate().filter(|(_, it)| it.fast && !it.gone).map(|(i, _)| i).collect();
        self.inputs.clear();
        self.view.lerps.clear();
        self.view.outline = Default::default();
        self.view.cam_smooth = None;
        self.view.painted.clear();
        self.ui.drag = None;
        self.screen.fade = None;
        self.avatar_died = None;
        self.just_destroyed.clear();
        self.end_orphaned_processes();
        self.ui.slot_menu = None;
        self.touching.clear();
        self.started = true;
        self.ui.title = None;
        // what the player had the Avatar do (walk, climb, fight) ends
        self.orders.approaching = None;
        self.orders.climb_goal = None;
        self.orders.click_fight = None;
        self.orders.walk_level = None;
        self.ui.hover = None;
        self.ui.hover_name = None;
    }

    pub fn load_from(&mut self, slot: usize) -> Result<(), String> {
        let path = self.save_path(slot).ok_or("no folder for savegames (APPDATA is not set)")?;
        // (a save still being written in the background is finished first)
        wait_for_saves();
        let bytes = std::fs::read(&path).map_err(|_| "there is no saved game yet".to_string())?;
        if bytes.len() < 12 + INFO_LEN || &bytes[..8] != MAGIC {
            return Err(format!("{} is not a savegame", path.display()));
        }
        let format = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if !(OLDEST_FORMAT..=FORMAT).contains(&format) {
            return Err(format!("the savegame is from another version of the engine (format {format}, expected {OLDEST_FORMAT} to {FORMAT})"));
        }
        let mut rest = &bytes[12 + INFO_LEN..];
        let d: SaveData = bincode::deserialize_from(&mut rest).map_err(|e| format!("the savegame is damaged: {e}"))?;
        let Appended { extra, vitals, journal, look, changelings, map_states, spells, made, passersby, resurrections, searching, revealed_path, guide_target } =
            if format == OLDEST_FORMAT { Appended::read_format_6(&mut rest) } else { Appended::read(rest)? };
        // (a savegame made before the fight states stayed with their map)
        let mixed_states = map_states.is_none();
        let map_states = map_states.unwrap_or_default();
        if d.language != self.language {
            return Err("the savegame belongs to another language version of the game".into());
        }
        if !d.maps.contains_key(&d.cur_map) || d.maps[&d.cur_map].items.iter().all(|it| it.gone || it.id != AVATAR) {
            return Err("the savegame is damaged: no Avatar".into());
        }

        self.audio.stop_all_sfx();
        self.cur_map = d.cur_map;
        self.tick = d.tick;
        self.rng = d.rng;
        let mut maps = d.maps;
        for (&n, map) in &mut maps {
            map.reindex();
            // (savegames carry their maps: corrections to the map data apply here too)
            crate::mapfix::fix_map(n, map, || self.original_map(n));
        }
        self.maps = maps;
        self.people.npcs = d.npcs;
        self.actors = d.actors;
        self.people.npc_dirs = d.npc_dirs;
        self.fights.states = d.states;
        self.mana = d.mana;
        self.vitals = vitals;
        self.scripts.usecode.set_globals(&d.globals);
        self.journal.restore(journal);
        self.scripts.procs = d.procs;
        self.scripts.next_pid = d.next_pid;
        self.scripts.strings = d.strings;
        self.scripts.next_string = d.next_string;
        self.scripts.lists = d.lists;
        self.scripts.next_list = d.next_list;
        self.barks = d.barks;
        self.ask = d.ask;
        self.ui.reader = d.reader;
        self.fights.changelings = changelings;
        self.fights.map_states = map_states;
        self.spell_notes.learned = spells;
        self.set_passersby(&passersby);
        self.fights.resurrections = resurrections;
        self.revealed_path = revealed_path;
        if let Some(target) = guide_target {
            self.set_guide_target(target);
        }
        self.fights.searching = searching;
        match made {
            Some((script_made, calmed)) => (self.fights.script_made, self.fights.calmed) = (script_made, calmed),
            None => {
                // older savegames: a golem that does not fight is the one of
                // the spell (golems from monster eggs start fighting at once)
                self.fights.calmed.clear();
                self.fights.script_made = self.map().items.iter()
                    .filter(|it| !it.gone && it.shape == crate::magic::GOLEM_SHAPE)
                    .map(|it| it.id)
                    .filter(|id| self.fights.states.get(id).is_some_and(|s| !s.in_combat && !s.dead))
                    .collect();
            }
        }
        self.spells_from_story();
        if let (Some(r), Some(look)) = (&mut self.ui.reader, look) {
            r.look = look;
            // its picture is loaded when it is needed for drawing
            if let Some(g) = Self::reader_gump(look) {
                self.data.gump_shapes.get(g);
            }
        }
        self.ethereal = d.ethereal;
        self.fade_ins.clear();
        self.stasis = d.stasis;
        self.pending_teleport = d.pending_teleport;
        self.gate_teleport = None;
        self.just_teleported = d.just_teleported;
        self.camera = d.camera;
        self.ui.gumps = d.gumps;
        self.time_offset = d.time_offset;
        self.sched_last_hour = d.sched_last_hour;
        self.sched_after_scene.clear();
        self.sched_next = d.sched_next;
        self.screen.pal_transform = d.pal_transform;
        self.data.pal.set_transform(crate::palette::shown_transform(d.pal_transform, self.options.storm));
        self.screen.storm_shown = self.options.storm;
        self.screen.inverted = d.inverted;
        self.airwalk = d.airwalk;
        self.reset_after_load();
        (self.avatar_name, self.fights.full_hp) = extra.unwrap_or_else(|| ("Avatar".to_string(), HashMap::default()));
        // back to the music of the place, or of the fight that was going on
        self.audio.restore_music();
        self.audio.play_music(d.music);
        // only there: newer ones keep the states of things on this map
        // that cannot fight (a floating platform asked for its hit points),
        // and their scripts must go on
        if mixed_states {
            self.repair_foreign_states();
        }
        if self.fights.states.get(&AVATAR).is_some_and(|s| s.in_combat) {
            self.audio.play_combat_music(98);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written() -> Vec<u8> {
        let mut bytes = Vec::new();
        put_block(&mut bytes, b"name", &("Avatar", 7u32)).unwrap();
        put_block(&mut bytes, b"new!", &[1u8, 2, 3]).unwrap();
        put_block(&mut bytes, b"spel", &vec!["Light".to_string(), "Flash".to_string()]).unwrap();
        bytes
    }

    /// Every block is found by its name, whatever stands around it; one
    /// the savegame lacks is simply not there.
    #[test]
    fn blocks_by_name() {
        let bytes = written();
        let b = Blocks::read(&bytes).unwrap();
        assert_eq!(b.get::<Vec<String>>(b"spel").unwrap(), Some(vec!["Light".to_string(), "Flash".to_string()]));
        assert_eq!(b.get::<(String, u32)>(b"name").unwrap(), Some(("Avatar".to_string(), 7)));
        assert_eq!(b.get::<u32>(b"vitl").unwrap(), None);
    }

    /// A block that cannot be read is reported, and does not spoil the others.
    #[test]
    fn damaged_block() {
        let bytes = written();
        let b = Blocks::read(&bytes).unwrap();
        assert!(b.get::<Vec<String>>(b"name").is_err());
        assert!(b.get::<Vec<String>>(b"spel").is_ok());
        // a savegame cut off in the middle of a block
        assert!(Blocks::read(&bytes[..bytes.len() - 3]).is_err());
        assert!(Blocks::read(&bytes[..5]).is_err());
    }
}
