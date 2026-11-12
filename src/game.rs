//! The running game: world state, actors, script processes.

use crate::actor::{MoveInput, Mover};
use crate::anim::AnimDat;
use crate::flex::Flex;
use crate::font::Font;
use crate::hash::{HashMap, HashSet};
use crate::map::{self, Item, MapData, Npc, WorldBox};
use crate::palette::Palette;
use crate::save;
use crate::shape::{self, Shapes};
use crate::usecode::Usecode;
use crate::view::{self, Camera, Painted, Surface, ViewOptions};

use std::error::Error;
use std::path::Path;

pub const AVATAR: u16 = 1;
/// The number the scripts let the Guardian speak under. No item has it:
/// he is only heard (his voice is SOUND/?666.FLX).
pub const GUARDIAN: u16 = 666;
/// The font his words are shown in: red, like the Avatar's, but always at
/// the top edge (the other readable colours belong to the people).
const GUARDIAN_FONT: usize = 6;

/// A move of an actor that is spread over the time until its next step.
pub(crate) struct Lerp {
    from: (f32, f32, f32),
    to: (i32, i32, i32),
    start: std::time::Instant,
    seconds: f32,
}

impl Lerp {
    /// Where the actor is drawn right now, and whether the move is over.
    fn position(&self, now: std::time::Instant) -> ((i32, i32, i32), bool) {
        let t = (now.duration_since(self.start).as_secs_f32() / self.seconds).min(1.0);
        let f = |a: f32, b: i32| (a + (b as f32 - a) * t).round() as i32;
        ((f(self.from.0, self.to.0), f(self.from.1, self.to.1), f(self.from.2, self.to.2)), t >= 1.0)
    }
}

/// Text shown above an item while its process lives.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Bark {
    pub pid: u16,
    pub item: u16,
    pub text: String,
}

pub struct Game {
    pub(crate) fixed: Flex,
    pub(crate) nonfixed: Flex,
    globs: Flex,
    /// Permanent NPCs that are not on the current map.
    pub npcs: Vec<Npc>,
    pub maps: HashMap<usize, MapData>,
    pub cur_map: usize,

    /// Animation state of the actors that have moved so far.
    pub actors: HashMap<u16, Mover>,
    pub npc_dirs: HashMap<u16, u8>,
    pub npc_stats: HashMap<u16, [u8; 4]>,
    /// Alignment byte and actor flags of the permanent NPCs.
    pub npc_combat: HashMap<u16, (u8, u32)>,
    pub states: HashMap<u16, crate::combat::ActorState>,
    /// The fight states of the creatures of the maps the Avatar is not on
    /// (their numbers from 256 on belong to one map: on another the same
    /// number is something else, a floor tile, say).
    pub map_states: HashMap<usize, HashMap<u16, crate::combat::ActorState>>,
    /// Ticks until the fallen Avatar gets up again.
    pub avatar_died: Option<u32>,
    /// Things destroyed by a script this tick: a script may still read
    /// them (a candle put out is destroyed, then its frame read to make
    /// the unlit one), as in the original, where they live on to the end
    /// of the tick.
    pub just_destroyed: crate::hash::HashMap<u16, crate::map::Item>,
    /// Creatures a script took out of a fight (Confusion Blast ...): they
    /// do not start another one on sight, only when attacked again.
    pub calmed: crate::hash::HashSet<u16>,
    /// Creatures a script made (createActor) for a scene: they do not start
    /// fights on sight, only when their script puts them in one (or they
    /// are attacked). The ghouls at Lothian's ceremony are such.
    pub script_made: crate::hash::HashSet<u16>,
    /// Changelings in the middle of taking the Avatar's shape (combat.rs):
    /// the step they are at and when it ends.
    pub changelings: crate::hash::HashMap<u16, crate::combat::Shift>,
    /// Killed creatures that get up again, and the tick when (combat.rs).
    /// Not saved: in a loaded game they stay down.
    pub resurrections: crate::hash::HashMap<u16, u32>,
    /// Tiles of the invisible path a thrown thing has shown, per map, with
    /// the tick they were shown (pathreveal.rs).
    pub revealed_path: crate::hash::HashMap<usize, crate::hash::HashMap<u16, u32>>,
    /// Townsfolk walking about in Tenebrae (option "passers-by").
    pub passersby: Vec<crate::passersby::Passer>,
    /// The height of the spot the player sent the Avatar to with a click:
    /// he has only arrived on that level (a click below an edge is not
    /// reached by standing above it).
    pub walk_level: Option<i32>,
    /// That spot is on something small (a pillar): only there exactly is he there.
    pub walk_exact: bool,
    /// For every moving actor the items it touched after its last step,
    /// so that their "got hit" scripts run once per contact.
    pub touching: HashMap<u16, Vec<u16>>,
    pub mana: i32,
    /// Healing over time, hunger, practice, magic states (see vitals.rs).
    pub vitals: crate::vitals::Vitals,
    pub tick: u32,
    pub rng: map::Rng,

    /// The script machine: the scripts, their processes and values (vm.rs).
    pub scripts: crate::vm::Scripts,
    pub barks: Vec<Bark>,
    /// Where the answers of the open conversation were drawn (top line)
    /// and how high each line is.
    ask_top: i32,
    ask_line: i32,
    /// Open conversation: the waiting process and the answers to pick from.
    pub ask: Option<(u16, Vec<String>)>,
    /// The answer picked with the keys or the gamepad (Enter takes it).
    pub ask_pick: Option<usize>,
    /// The player's own keys and gamepad buttons.
    pub bindings: crate::bindings::Bindings,
    /// Items taken out of the world by scripts, with where they were.
    pub ethereal: Vec<(u16, u16)>,
    /// Creatures that came out of a monster egg in sight, with the tick
    /// they appeared: they fade in (option "monster_fade").
    pub fade_ins: HashMap<u16, u32>,
    /// The Avatar ignores player input (cutscenes, conversations).
    pub stasis: bool,

    /// What the last frame showed, for mouse picking.
    pub(crate) painted: Vec<Painted>,

    /// Size of the view in pixels; decides how far the world is active.
    pub view_size: (i32, i32),
    /// How the Avatar is drawn now (option "show armour").
    pub outfit: (crate::outfit::Look, crate::outfit::Helm, crate::outfit::Glow),
    /// The minimap's picture of the surroundings (option "minimap").
    pub minimap: crate::minimap::Minimap,
    /// Pixels of the view the smoothing leaves sharp: the visible patterned
    /// armour of the Avatar (empty: none). Made with every picture.
    pub sharp: Vec<bool>,
    /// View pixels per pixel of the original size (the zoom): above 1 the
    /// view is drawn larger and shown smaller.
    pub view_zoom: f32,
    /// While a menu or book covers the world (drawn at the original size),
    /// the world's own size and zoom: it is drawn as in play and scaled
    /// under the menu, so the zoom does not jump.
    pub world_under_menu: Option<((i32, i32), f32)>,
    /// Indices of the items in the active area.
    pub fast_items: Vec<usize>,
    pub pending_teleport: Option<(usize, crate::eggs::Destination)>,
    /// A gate to another map the Avatar walked into: the view fades to
    /// black first, then the teleport is carried out (eggs.rs).
    pub gate_teleport: Option<(usize, crate::eggs::Destination)>,
    /// Set after a teleport until the Avatar has left the teleporter.
    pub just_teleported: bool,

    /// Directions scripts make actors walk in this tick.
    pub inputs: HashMap<u16, MoveInput>,
    pub camera: crate::effects::CameraMode,
    pub fade: Option<crate::effects::Fade>,

    /// Started with --cheat: the game's own debug functions are unlocked.
    pub cheat: bool,
    /// What the cheat keys F8 / F9 put on (it vanishes on the next press).
    pub cheat_worn: Vec<u16>,
    /// Strength of the earthquake shaking the view (0 = none), and where
    /// it has pushed the view in this tick (screen pixels).
    pub quake: i32,
    pub quake_offset: (i32, i32),
    /// The script process that set the earthquake (0: none): one started
    /// before it may not weaken or end it (scripts.rs, `quake_blocked`).
    pub quake_by: u16,
    /// The view is shown upside down (a spell).
    pub inverted: bool,
    /// Colour transform of the palette (storms).
    pub pal_transform: u8,
    /// The "storm colours" setting the colours on screen were made with.
    pub storm_shown: usize,
    /// The Avatar walks on air (a spell).
    pub airwalk: bool,
    /// Game time at tick 0, in ticks.
    pub time_offset: u32,
    /// The way each walking actor follows (not saved; searched again).
    pub paths: HashMap<u16, crate::effects::Way>,
    /// The last way search of each walker: from where to where (on a
    /// 32 grid). The same search again finds nothing new - and one that
    /// finds no way can take a good part of a frame.
    pub path_tried: HashMap<u16, (i32, i32, i32, i32)>,
    /// Creatures whose walks to their enemy brought them no closer: the
    /// gap after the last walk and how many walks in a row failed.
    pub approach_fails: HashMap<u16, (i32, u32)>,
    /// Creatures on their way to where they last saw the vanished Avatar.
    pub searching: HashSet<u16>,
    /// Game hour of the last round of schedule scripts, and whose turn it
    /// is in the current round (0: none running).
    pub sched_last_hour: u32,
    /// People whose schedule a script ran during the ceremony at the Great
    /// Pentacle, when it does nothing: run again once the scene is over.
    pub sched_after_scene: Vec<u16>,
    /// Sleeping until a time of day (sleep.rs): the hours the open choices
    /// lead to, and the hour chosen for the bed's script to set.
    pub sleep_menu: Option<Vec<Option<u32>>>,
    pub sleep_until: Option<u32>,
    pub sched_next: u16,
    /// The quest journal (option "journal", key T).
    pub journal: crate::journal::Journal,
    /// The spells learned so far (option "spell_notes", key N).
    pub spell_notes: crate::spellnotes::SpellNotes,
    /// The way to a place chosen on the map (option "guide", guide.rs).
    pub guide: crate::guide::Guide,
    /// The pictures of the map's parts for the large map (worldmap.rs).
    pub big_map: crate::minimap::Minimap,
    pub audio: crate::audio::Audio,
    /// The player asked for a new game; the main loop starts one.
    pub restart: bool,
    pub options: crate::menu::Options,
    /// Actors on their way from one animation step to the next: drawn at
    /// positions in between while the smooth option is on.
    pub(crate) lerps: HashMap<u16, Lerp>,
    /// Where the gliding camera is right now, and when it was last moved.
    pub(crate) cam_smooth: Option<((f32, f32, f32), std::time::Instant)>,
    /// The general light of the lighting as last drawn, and at which tick:
    /// it goes over smoothly to a new one (into a cave, through the day).
    ambient_now: Option<([f32; 3], u32)>,
    /// Lights found covered by floor (see lighting.rs), on which map, and
    /// when (by item id).
    pub(crate) covered_seen: (usize, crate::hash::HashMap<u16, (bool, u32, (u32, i32, i32, i32))>),
    /// Things found in a room under a roof (see lighting.rs).
    rooms_seen: crate::lighting::RoomsSeen,
    /// How far the light of rooms has come in (0 outside, 1 in a house),
    /// and at which tick: it goes over smoothly when the Avatar goes in or out.
    room_light: (f32, u32),
    /// Health the creatures started with, for the bars above them.
    pub(crate) full_hp: HashMap<u16, i32>,
    /// How fast the gliding camera is moving (world units per second).
    pub(crate) cam_vel: (f32, f32, f32),
    /// The camera's target as of the last picture, and since when it has
    /// been there.
    cam_target: Option<((f32, f32, f32), std::time::Instant)>,
    /// 'E', 'G' or 'F': language of the game data.
    pub language: char,
    /// Modern controls: the item the Avatar walks to in order to use it,
    /// and the item under the pointer.
    pub(crate) approaching: Option<u16>,
    /// Walking by click to a higher place (see jump.rs).
    pub(crate) climb_goal: Option<crate::jump::ClimbGoal>,
    /// A look script running on the side to find that name.
    pub(crate) name_probe: Option<crate::hover::NameProbe>,
    /// Fighting by clicking: the enemy the Avatar goes for.
    pub(crate) click_fight: Option<crate::controls::ClickFight>,
    /// Modern controls: shift switches between walking and running.
    pub run_toggle: bool,
    /// The game itself has begun (a name was given or a game loaded);
    /// before that there is no world behind the menus.
    pub started: bool,
    /// The player chose to leave.
    pub quit: bool,
    pub avatar_name: String,
    pub(crate) blink_start: std::time::Instant,
    /// Screen-space position of the view's top left corner in the last frame.
    pub view_origin: (i32, i32),
    /// How the last picture was drawn: centre and turn (for converting
    /// between view and world positions).
    pub view_xf: crate::turnview::Xform,
    /// The turnable view (optional).
    pub turn: crate::turnview::TurnView,
    /// The item drawn at every pixel of the last picture (original view;
    /// index + 1, 0 where nothing is).
    pub pixel_ids: Vec<u32>,
    /// The windows, menus, books and other things on the screen for the player (ui.rs).
    pub ui: crate::ui::Ui,
    /// The game's own data from its files (gamedata.rs).
    pub data: crate::gamedata::GameData,
}

impl Game {
    /// `lang_dir`: the language folder of the game (ENGLISH, GERMAN ...).
    /// `sound`: open the sound devices (off for tests without a window).
    pub fn load(lang_dir: &Path, sound: bool) -> Result<Game, Box<dyn Error>> {
        let stat = lang_dir.join("STATIC");
        let open = |name: &str| Flex::open(&stat.join(name)).map_err(|e| format!("{name}: {e}"));

        // U8SAVE.000 is the state of the world at the start of a new game
        let save_path = lang_dir.join("SAVEGAME").join("U8SAVE.000");
        let mut save = save::read_savegame(&save_path).map_err(|e| format!("{}: {e}", save_path.display()))?;
        let mut member = |name: &str| -> Result<Flex, Box<dyn Error>> {
            let data = save.remove(name).ok_or(format!("{name} missing in {}", save_path.display()))?;
            Ok(Flex::from_bytes(data).map_err(|e| format!("{name}: {e}"))?)
        };
        let nonfixed = member("NONFIXED.DAT")?;
        let npcs = map::load_npcs(&member("ITEMCACH.DAT")?, &member("NPCDATA.DAT")?);
        let avatar = npcs.iter().find(|n| n.item.id == AVATAR).ok_or("the savegame has no Avatar")?;
        let cur_map = avatar.map;
        let start_mana = avatar.mana as i32;

        // the usecode file is named after the language: EUSECODE, GUSECODE ...
        let uc_dir = lang_dir.join("USECODE");
        let initial = lang_dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.chars().next()).unwrap_or('E');
        let uc_path = [format!("{initial}USECODE.FLX"), "EUSECODE.FLX".to_string()]
            .iter()
            .map(|n| uc_dir.join(n))
            .find(|p| p.is_file())
            .ok_or(format!("no usecode file in {}", uc_dir.display()))?;

        // font 0 is the usual one; the others are picked by number where needed
        let (font, fonts) = match Shapes::open(&stat.join("U8FONTS.FLX")) {
            Ok(mut f) => (f.get(0).map(Font::new), (0..16).map(|n| f.get(n).map(Font::new)).collect()),
            Err(_) => (None, Vec::new()),
        };

        let settings = crate::menu::load_settings();
        let mut game = Game {
            data: crate::gamedata::GameData {
                shapes: Shapes::open(&stat.join("U8SHAPES.FLX"))?,
                infos: shape::load_type_flags(&stat.join("TYPEFLAG.DAT"))?,
                pal: Palette::load(&stat.join("U8PAL.PAL"))?,
                anims: AnimDat::load(&stat.join("ANIM.DAT")).map_err(|e| format!("ANIM.DAT: {e}"))?,
                font,
                fonts,
                monsters: crate::combat::load_monsters(),
                weapon_overlays: open("WPNOVLAY.DAT")?,
                tables: crate::equip::Tables::load(),
                gump_shapes: Shapes::open(&stat.join("U8GUMPS.FLX"))?,
                gump_areas: std::fs::read(stat.join("GUMPAGE.DAT"))
                    .unwrap_or_default()
                    .chunks_exact(8)
                    .map(|r| std::array::from_fn(|i| i16::from_le_bytes([r[i * 2], r[i * 2 + 1]]) as i32))
                    .collect(),
                static_dir: stat.clone(),
            },
            ui: Default::default(),
            fixed: open("FIXED.DAT")?,
            nonfixed,
            globs: open("GLOB.FLX")?,
            npc_dirs: npcs.iter().map(|n| (n.item.id, n.dir)).collect(),
            npc_stats: npcs.iter().map(|n| (n.item.id, n.stats)).collect(),
            npc_combat: npcs.iter().map(|n| (n.item.id, (n.align, n.flags))).collect(),
            states: HashMap::default(),
            avatar_died: None,
            just_destroyed: Default::default(),
            calmed: Default::default(),
            script_made: Default::default(),
            changelings: Default::default(),
            resurrections: Default::default(),
            revealed_path: Default::default(),
            map_states: Default::default(),
            passersby: Vec::new(),
            walk_level: None,
            walk_exact: false,
            touching: HashMap::default(),
            mana: start_mana,
            vitals: Default::default(),
            npcs,
            maps: HashMap::default(),
            cur_map,
            actors: HashMap::default(),
            tick: 0,
            rng: map::Rng(0x2F6E_2B1D),
            scripts: crate::vm::Scripts {
                usecode: Usecode::load(&uc_path).map_err(|e| format!("{}: {e}", uc_path.display()))?,
                procs: Vec::new(),
                next_pid: 16,
                strings: HashMap::default(),
                next_string: 0,
                lists: HashMap::default(),
                next_list: 0,
                warned: HashSet::default(),
            },
            barks: Vec::new(),
            ask_top: 0,
            ask_line: 8,
            ask: None,
            ask_pick: None,
            bindings: crate::bindings::Bindings::load(),
            ethereal: Vec::new(),
            fade_ins: HashMap::default(),
            stasis: false,
            painted: Vec::new(),
            view_size: (640, 400),
            view_zoom: 1.0,
            world_under_menu: None,
            outfit: (crate::outfit::Look::Plain, crate::outfit::Helm::Plain, crate::outfit::Glow { head: false, body: false, arms: false, legs: false }),
            minimap: Default::default(),
            sharp: Vec::new(),
            fast_items: Vec::new(),
            pending_teleport: None,
            gate_teleport: None,
            just_teleported: false,
            inputs: HashMap::default(),
            camera: crate::effects::CameraMode::Follow(AVATAR),
            fade: None,
            cheat: false,
            cheat_worn: Vec::new(),
            quake: 0,
            quake_by: 0,
            quake_offset: (0, 0),
            inverted: false,
            pal_transform: 0,
            storm_shown: 1,
            airwalk: false,
            // a new game starts at eight in the morning
            time_offset: 8 * 27000,
            paths: HashMap::default(),
            path_tried: HashMap::default(),
            approach_fails: HashMap::default(),
            searching: HashSet::default(),
            sched_last_hour: 0,
            sched_after_scene: Vec::new(),
            sleep_menu: None,
            sleep_until: None,
            sched_next: 0,
            journal: Default::default(),
            spell_notes: Default::default(),
            guide: Default::default(),
            big_map: Default::default(),
            audio: crate::audio::Audio::new(&lang_dir.join("SOUND"), initial.to_ascii_uppercase(), sound, settings.0, settings.1.better_music, settings.1.album_music),
            restart: false,
            options: settings.1,
            cam_smooth: None,
            ambient_now: None,
            covered_seen: (usize::MAX, crate::hash::HashMap::default()),
            rooms_seen: (usize::MAX, HashMap::default()),
            room_light: (0.0, 0),
            full_hp: HashMap::default(),
            cam_vel: (0.0, 0.0, 0.0),
            cam_target: None,
            lerps: HashMap::default(),
            language: initial.to_ascii_uppercase(),
            approaching: None,
            climb_goal: None,
            click_fight: None,
            name_probe: None,
            run_toggle: false,
            started: true,
            quit: false,
            avatar_name: "Avatar".to_string(),
            blink_start: std::time::Instant::now(),
            view_origin: (0, 0),
            view_xf: crate::turnview::Xform { cam: (0, 0, 0), angle: 0.0, size: (640, 400) },
            turn: crate::turnview::TurnView::default(),
            pixel_ids: Vec::new(),
        };
        game.data.mark_lying();
        game.load_map(cur_map);
        // the Avatar starts with an empty backpack
        game.map_mut().add(Item {
            shape: 529,
            parent: AVATAR,
            z: crate::equip::SLOT_BACKPACK,
            flags: crate::equip::FLG_EQUIPPED | crate::drag::FLG_CONTAINED,
            mapnum: cur_map as u16,
            ..Item::default()
        });
        let mut mover = Mover::new(game.npc_dirs.get(&AVATAR).copied().unwrap_or(4));
        mover.auto_stand = true;
        game.actors.insert(AVATAR, mover);
        game.cache_in();
        Ok(game)
    }

    pub fn map_exists(&self, n: usize) -> bool {
        self.fixed.get(n).is_some() || self.nonfixed.get(n).is_some()
    }

    /// Makes sure the map is loaded; NPCs standing on it move in.
    pub fn load_map(&mut self, n: usize) {
        if self.maps.contains_key(&n) {
            return;
        }
        let sources: Vec<&[u8]> = [&self.fixed, &self.nonfixed].iter().filter_map(|f| f.get(n)).collect();
        let here: Vec<Item> = self.npcs.iter().filter(|npc| npc.map == n).map(|npc| npc.item).collect();
        let mut map = MapData::build(&sources, &here, &self.globs, &self.data.infos);
        crate::mapfix::fix_map(n, &mut map, || MapData::build(&sources, &[], &self.globs, &self.data.infos));
        self.npcs.retain(|npc| npc.map != n);
        self.maps.insert(n, map);
    }

    /// A map as the game data make it, without its people (for where its
    /// fixed things belong).
    pub fn original_map(&self, n: usize) -> MapData {
        let sources: Vec<&[u8]> = [&self.fixed, &self.nonfixed].iter().filter_map(|f| f.get(n)).collect();
        MapData::build(&sources, &[], &self.globs, &self.data.infos)
    }

    /// The map the Avatar is on.
    /// Of German, French and English: the one for the game's language
    /// (English for any other).
    pub fn tr<T>(&self, [german, french, english]: [T; 3]) -> T {
        match self.language {
            'G' => german,
            'F' => french,
            _ => english,
        }
    }

    pub fn map(&self) -> &MapData {
        &self.maps[&self.cur_map]
    }

    pub fn map_mut(&mut self) -> &mut MapData {
        self.maps.get_mut(&self.cur_map).unwrap()
    }

    pub fn avatar(&self) -> Item {
        self.map().get(AVATAR).copied().unwrap_or_default()
    }

    /// Moves an actor to another position. The Avatar changes maps at the
    /// end of the tick; anyone else moves at once, from whatever map they
    /// are on (Beren, called by the townspeople, comes from his own map).
    pub fn teleport(&mut self, id: u16, map: usize, pos: (i32, i32, i32)) {
        if id == AVATAR {
            if map != self.cur_map {
                self.pending_teleport = Some((map, crate::eggs::Destination::Position(pos.0, pos.1, pos.2)));
            } else if let Some(idx) = self.map().index(id) {
                self.map_mut().place(idx, pos.0, pos.1, pos.2);
                // the view jumps along at once: what it leaves and enters
                // notices it right away, even if he is moved back before
                // the next tick (the Skull of Quakes)
                if matches!(self.camera, crate::effects::CameraMode::Follow(AVATAR)) {
                    self.update_fast_area();
                }
            }
            return;
        }
        let Some(from) = self.map_of(id) else { return };
        if from == map {
            if let Some(idx) = self.maps[&map].index(id) {
                self.maps.get_mut(&map).unwrap().place(idx, pos.0, pos.1, pos.2);
            }
            return;
        }
        if !self.map_exists(map) {
            return;
        }
        self.load_map(map);
        if from == self.cur_map {
            // gone from here: no more steps or scripts of its own here
            self.actors.remove(&id);
            self.lerps.remove(&id);
        }
        self.move_between_maps(id, from, map, pos);
    }

    /// A person (id below 256) wherever they are: on this map, on another
    /// loaded one, or still waiting on a map not loaded yet. With the map.
    pub fn actor_anywhere(&self, id: u16) -> Option<(usize, Item)> {
        if let Some(it) = self.map().get(id) {
            return Some((self.cur_map, *it));
        }
        if id == 0 || id >= 256 {
            return None;
        }
        if let Some((&n, it)) = self.maps.iter().find_map(|(n, m)| m.get(id).map(|it| (n, it))) {
            return Some((n, *it));
        }
        self.npcs.iter().find(|npc| npc.item.id == id).map(|npc| (npc.map, npc.item))
    }

    /// The map an actor is on (loading it if it is not loaded yet).
    fn map_of(&mut self, id: u16) -> Option<usize> {
        if self.map().index(id).is_some() {
            return Some(self.cur_map);
        }
        if let Some((&n, _)) = self.maps.iter().find(|(_, m)| m.index(id).is_some()) {
            return Some(n);
        }
        let n = self.npcs.iter().find(|npc| npc.item.id == id)?.map;
        self.load_map(n);
        self.maps[&n].index(id).is_some().then_some(n)
    }

    /// Moves an item and everything it carries from one loaded map to
    /// another, putting it into the world there. The contents get new ids
    /// on the new map; the item itself keeps its own.
    fn move_between_maps(&mut self, id: u16, from: usize, to: usize, pos: (i32, i32, i32)) {
        let src = &self.maps[&from];
        let Some(root) = src.index(id) else { return };
        // the item first, then the contents level by level (parents before children)
        let mut order = vec![root];
        let mut k = 0;
        while k < order.len() {
            let (this, parent) = (order[k], src.items[order[k]].id);
            if parent != 0 {
                order.extend(src.items.iter().enumerate().filter(|(j, it)| !it.gone && it.parent == parent && *j != this).map(|(j, _)| j));
            }
            k += 1;
        }
        let moving: Vec<Item> = order.iter().map(|&j| src.items[j]).collect();
        self.maps.get_mut(&from).unwrap().destroy(root);
        let dst = self.maps.get_mut(&to).unwrap();
        let mut new_ids: HashMap<u16, u16> = HashMap::default();
        for (k, mut it) in moving.into_iter().enumerate() {
            let old = it.id;
            it.fast = false;
            it.hatched = false;
            if k == 0 {
                (it.x, it.y, it.z, it.parent) = (pos.0, pos.1, pos.2, map::IN_WORLD);
            } else {
                it.parent = new_ids.get(&it.parent).copied().unwrap_or(it.parent);
                if !it.is_actor() {
                    it.id = 0;
                }
            }
            if it.is_actor() {
                // (for other items the field means something else)
                it.mapnum = to as u16;
            }
            let new = dst.add(it);
            new_ids.insert(old, new);
        }
    }

    fn tick_actor(&mut self, id: u16, input: Option<MoveInput>) {
        let Some(mut mover) = self.actors.remove(&id) else { return };
        if let Some(idx) = self.map().index(id) {
            let mut it = self.map().items[idx];
            if it.in_world() {
                let map = &self.maps[&self.cur_map];
                let before = (it.x, it.y, it.z);
                mover.in_scene = id == AVATAR && self.stasis;
                if mover.tick(input, &mut it, map, &self.data.infos, &mut self.data.anims) {
                    let after = (it.x, it.y, it.z);
                    let jump = (after.0 - before.0).abs().max((after.1 - before.1).abs()).max((after.2 - before.2).abs());
                    if self.options.smooth_camera && after != before && jump < 200 {
                        // glide there until the next step is due, starting
                        // from wherever the actor is drawn at this moment
                        let now = std::time::Instant::now();
                        let shown = self.lerps.get(&id).filter(|l| l.to == before).map_or(before, |l| l.position(now).0);
                        self.lerps.insert(id, Lerp {
                            from: (shown.0 as f32, shown.1 as f32, shown.2 as f32),
                            to: after,
                            start: now,
                            seconds: mover.step_ticks() as f32 / 30.0,
                        });
                    } else if after != before {
                        self.lerps.remove(&id);
                    }
                    let map = self.map_mut();
                    map.place(idx, it.x, it.y, it.z);
                    map.items[idx].frame = it.frame;
                    map.items[idx].flags = it.flags;
                    if after != before {
                        let force = crate::effects::hit_force(after.0 - before.0, after.1 - before.1, after.2 - before.2);
                        self.touch_items(id, force);
                    }
                }
                if let Some(running) = mover.footstep.take() {
                    // a frame marked "special": a footfall, or what the
                    // creature's kind does at this point of the animation
                    let action = mover.last_anim();
                    if !self.special_frame(&it, action) {
                        self.play_footstep(&it, running);
                    }
                }
                if mover.sfx != 0 {
                    let volume = self.sfx_volume(id);
                    self.audio.play_sfx(mover.sfx as i32, id, false, volume);
                    mover.sfx = 0;
                }
            }
        }
        self.actors.insert(id, mover);
        // an attack frame: does it reach someone?
        if let Some(range) = self.actors.get_mut(&id).and_then(|m| m.attack.take()) {
            if self.weapon_hit(id, range) {
                if let Some(m) = self.actors.get_mut(&id) {
                    m.hit_done = true;
                }
            }
        }
    }

    /// Runs the world for one game tick.
    pub fn step(&mut self, input: Option<MoveInput>) {
        self.tick = self.tick.wrapping_add(1);
        self.just_destroyed.clear();
        if input.is_some() {
            // steering takes over from walking to what was clicked
            self.cancel_approach();
        }
        // the setting "storm colours" changed during a storm: at once
        if self.storm_shown != self.options.storm {
            self.storm_shown = self.options.storm;
            if self.pal_transform == 3 {
                self.data.pal.set_transform(crate::palette::shown_transform(3, self.options.storm));
            }
        }
        self.tick_vitals();
        self.tidy_potions();
        self.tick_passersby();
        self.tick_journal();
        self.tick_guide();
        self.tick_approach();
        self.tick_auto_climb();
        self.tick_click_fight();
        let ids: Vec<u16> = self.actors.keys().copied().collect();
        for id in ids {
            // scripts steer first, then the player
            let scripted = self.inputs.get(&id).copied();
            let input = scripted.or(if id == AVATAR && self.player_in_control() { input } else { None });
            if let Some(m) = self.actors.get_mut(&id) {
                m.careful = scripted.is_some();
                m.player_walk = id == AVATAR && scripted.is_none() && input.is_some();
            }
            self.tick_actor(id, input);
        }
        if self.tick % 4 == 1 {
            self.update_fast_area();
            // (the clearing egg's scripts have run: its floor gap is filled)
            let n = self.cur_map;
            crate::mapfix::fix_wiped(n, self.map_mut());
            self.close_far_gumps();
        }
        self.hatch_eggs();
        self.run_scheduler();
        self.run_combat();
        self.run_processes();
        self.gate_fade_done();
        self.apply_teleport();
        self.tick_effects();
        self.place_sounds();
        if self.tick.is_multiple_of(30) {
            // looping sounds end when their source is out of range
            let Game { audio, maps, cur_map, .. } = self;
            let map = &maps[cur_map];
            audio.stop_loops(|obj| obj == 0 || map.get(obj).is_some_and(|it| it.fast));
        }
        let tick = self.tick;
        let mut scripted = Vec::new();
        let Game { data: crate::gamedata::GameData { shapes, infos, .. }, maps, rng, cur_map, .. } = self;
        map::animate(&mut maps.get_mut(cur_map).unwrap().items, infos, shapes, tick, rng, &mut scripted);
        // candles, drips ...: their script picks the frame
        for id in scripted {
            self.call_event(id, 2, &[]);
        }
    }

    /// With a menu or book open and a zoom other than 100 %: draws the world
    /// at its zoom with the covering windows left out, scales it to `surf`
    /// and draws those windows on top. False if there is nothing to do.
    fn draw_world_under_menu(&mut self, surf: &mut Surface, opts: &mut ViewOptions, roofs_always: bool) -> bool {
        let Some(((w, h), zoom)) = self.world_under_menu else { return false };
        if (w, h) == (surf.width, surf.height) || w <= 0 || h <= 0 {
            return false;
        }
        let mut world = Surface::new(w, h);
        let size = (self.view_size, self.view_zoom);
        let menu = self.ui.menu.take();
        let slots = self.ui.slot_menu.take();
        let reader = self.ui.reader.take();
        let title = self.ui.title.take();
        // (the large map too: drawn with the world and scaled down with it,
        // its writing lost pixels and the smoothing blurred it)
        let worldmap = self.ui.worldmap.take();
        self.world_under_menu = None;
        (self.view_size, self.view_zoom) = ((w, h), zoom);
        self.draw(&mut world, opts, roofs_always);
        (self.view_size, self.view_zoom) = size;
        self.world_under_menu = Some(((w, h), zoom));
        self.ui.menu = menu;
        self.ui.slot_menu = slots;
        self.ui.reader = reader;
        self.ui.title = title;
        self.ui.worldmap = worldmap;
        // nearest pixel, the menu dims it anyway
        for y in 0..surf.height {
            let sy = (y * h / surf.height) as usize;
            for x in 0..surf.width {
                let sx = (x * w / surf.width) as usize;
                surf.pixels[(y * surf.width + x) as usize] = world.pixels[sy * w as usize + sx];
            }
        }
        self.draw_reader(surf);
        self.draw_worldmap(surf);
        self.draw_menu(surf);
        self.draw_slot_menu(surf);
        self.draw_title(surf);
        self.draw_notice(surf);
        true
    }

    /// What the Avatar says and the answers he can give, above his head.
    /// Drawn over open windows (a backpack beside him would hide the
    /// start of a line), as words are on top in the original.
    fn draw_avatar_words(&mut self, surf: &mut Surface) {
        let ts = self.text_scale();
        let Some(avatar) = self.map().get(AVATAR).copied() else { return };
        let height = self.data.infos.get(avatar.shape as usize).map_or(0, |si| si.z * 8);
        let (ax, ay) = self.view_xf.to_screen(avatar.x, avatar.y, avatar.z);
        let head = ay - height - 8;
        let Some(font) = self.data.font_or_plain(6) else { return };
        let pal = &self.data.pal;
        for bark in self.barks.iter().filter(|b| b.item == AVATAR && b.pid != 0 && avatar.in_world()) {
            font.draw_centered_at(surf, &bark.text, ax, head, (180.0 * ts) as i32, pal, ts);
        }
        // answers of an open conversation, numbered, above the Avatar
        // like what the others say above themselves
        if let Some((_, choices)) = &self.ask {
            let lh = font.height_at(ts);
            let total = choices.len() as i32 * lh;
            let top = (head - total).clamp(2, (surf.height - total - 4).max(2));
            self.ask_top = top;
            self.ask_line = lh;
            for (i, c) in choices.iter().enumerate() {
                // the one picked with the keys or the pad stands out
                let picked = self.ask_pick == Some(i);
                let line = if picked { format!("> {}. {} <", i + 1, c.trim()) } else { format!("{}. {}", i + 1, c.trim()) };
                let w = font.width_at(&line, ts);
                let x = (ax - w / 2).clamp(2, (surf.width - w - 2).max(2));
                font.draw_at(surf, &line, x, top + i as i32 * lh, pal, ts);
            }
        }
    }

    /// The pictures (frame, flipped) of the people near `centre` as seen
    /// from the turned side, by item index.
    fn turned_people(&mut self, (cx, cy): (i32, i32), quarter: i32) -> HashMap<usize, (u32, bool)> {
        let near: Vec<(usize, Item)> = self.map().items.iter().enumerate()
            .filter(|(_, it)| it.is_actor() && it.in_world() && (it.x - cx).abs() < 4096 && (it.y - cy).abs() < 4096)
            .map(|(i, it)| (i, *it))
            .collect();
        let mut people = HashMap::default();
        for (i, it) in near {
            people.insert(i, self.person_frame(&it, quarter));
        }
        people
    }

    /// Creatures fading in (item index, opacity).
    fn fading_items(&self) -> Vec<(usize, u32)> {
        self.fade_ins.keys().filter_map(|&id| Some((self.map().index(id)?, self.fade_in_alpha(id)?))).collect()
    }

    /// Draws the view centred on the Avatar.
    pub fn draw(&mut self, surf: &mut Surface, opts: &mut ViewOptions, roofs_always: bool) {
        self.dress_avatar();
        if self.title_covers_view() {
            // no world behind a movie, or before the game has begun
            surf.pixels.fill(0);
            self.draw_menu(surf);
            self.draw_slot_menu(surf);
            self.draw_title(surf);
            self.draw_notice(surf);
            return;
        }
        if self.draw_world_under_menu(surf, opts, roofs_always) {
            return;
        }
        // actors between two steps are shown on their way
        let moved = self.place_between_steps();

        let avatar = self.avatar();
        let (cx, cy, cz) = self.view_centre();
        // the view turned to another side (optional), smoothly from one to the next
        self.turn.settle();
        let angle = if self.options.turn_view { self.turn.angle() } else { 0.0 };
        let xf = crate::turnview::Xform { cam: (cx, cy, cz), angle, size: (surf.width, surf.height) };
        let quarter = xf.quarter();
        // the weapon in the Avatar's hand, for the direction he is seen in
        let av_dir = self.actors.get(&AVATAR).map(|m| m.dir);
        let overlay = self.weapon_overlay_for(av_dir.map(|d| (d as i32 + 2 * quarter).rem_euclid(8) as u8));
        let av_index = self.map().index(AVATAR);
        // people are seen from the turned side
        let people = if angle != 0.0 { self.turned_people((cx, cy), quarter) } else { HashMap::default() };
        // lamps and fires (optional lighting)
        let lighting = self.options.lighting;
        let lights = if lighting { Some(self.lights(&xf)) } else { None };
        // under the open sky (for shadows and the general light), and the time of day
        let sky = (self.options.shadows || lighting) && crate::shadows::sunlit(self.cur_map, self.map(), avatar.x, avatar.y);
        let hour = (self.tick.wrapping_add(self.time_offset) / 30 % (24 * 900)) as f32 / 900.0;
        let outside = self.ambient_light(crate::lighting::ambient(hour, sky));
        let look = crate::lighting::Look::by_day(outside);
        // in a house under the open sky: the rooms' own light, eased in and out
        let in_house = lighting && sky && !roofs_always && roof_above(self.map(), &self.data.infos, &avatar).is_some();
        let room_k = self.room_light(in_house);
        let inner = crate::lighting::indoor(outside);
        let room_look = (room_k > 0.01).then(|| crate::lighting::Look::by_day([0, 1, 2].map(|c| outside[c] + (inner[c] - outside[c]) * room_k)));
        let ts = self.text_scale();
        let name_ts = self.name_scale();
        self.begin_hover_label();
        // creatures from monster eggs in sight fade in
        opts.fading = self.fading_items();
        // the Avatar climbing onto a ledge is painted in front of it
        let climbing = self.actors.get(&AVATAR).is_some_and(|m| m.busy() && (crate::anim::CLIMB_FIRST..=crate::anim::CLIMB_LAST).contains(&m.last_anim()));
        opts.climbing = if climbing { self.map().index(AVATAR) } else { None };
        let Game { data: crate::gamedata::GameData { shapes, infos, pal, .. }, maps, cur_map, turn, pixel_ids, rooms_seen, .. } = self;
        let map = &maps[cur_map];

        // hide roofs (and everything above them) while the Avatar is below one
        opts.z_limit = if roofs_always { None } else { roof_above(map, infos, &avatar) };

        // An invisible Avatar is shown faintly while the player moves him
        // (potion, scroll), but not at all in a scene: there the scripts
        // hide him on purpose (burnt to pieces, carried off by a spell).
        let saved_hide = opts.hide;
        if self.stasis && avatar.flags & crate::map::FLG_INVISIBLE != 0 && opts.hide == 0 {
            opts.hide = AVATAR;
        }
        let cam = Camera { x: cx, y: cy, z: cz };
        self.painted = if angle == 0.0 {
            if self.options.turn_view {
                // get the turned models of the surroundings ready, a few per picture
                // (by time: after a map change every model is new, and 30
                // of them at once made the first pictures there stumble)
                turn.prepare(&map.items, infos, shapes, (cx, cy, cz), std::time::Duration::from_millis(2));
            }
            let overlay = overlay.and_then(|(shape, frame, dx, dy)| Some(view::Overlay { index: av_index?, shape, frame, dx, dy }));
            view::render(surf, &cam, &map.items, infos, shapes, pal, opts, overlay.as_ref(), pixel_ids)
        } else {
            let lookup = |it: &Item, _q: i32| -> (u32, bool) {
                map.index(it.id).and_then(|i| people.get(&i).copied()).unwrap_or((it.frame, it.flags & crate::map::FLG_FLIPPED != 0))
            };
            let scene = crate::turnview::Scene {
                items: &map.items,
                infos,
                pal,
                z_limit: opts.z_limit,
                show_editor_items: opts.show_editor_items,
                hide: opts.hide,
                person_frame: &lookup,
                overlay: overlay.and_then(|(shape, frame, dx, dy)| Some((av_index?, shape, frame, dx, dy))),
            };
            turn.render(surf, &xf, &scene, shapes)
        };
        opts.hide = saved_hide;
        self.view_origin = ((cam.x - cam.y) / 4 - surf.width / 2, (cam.x + cam.y) / 8 - cam.z - surf.height / 2);
        self.view_xf = xf;
        // shadows (optional), under the open sky (not in a cellar or cave), in the view as in the original (not turned)
        if self.options.shadows && angle == 0.0 && sky {
            // the light as the time of day has it
            crate::shadows::apply(surf, pixel_ids, &self.painted, map, infos, shapes, crate::shadows::Sun::at(hour));
        }
        if let Some(lights) = &lights {
            // what gives the light keeps its own colours (lava would turn a flat yellow)
            // and how high things stand (only known for the view as in the original)
            // (what is at each pixel is only known for the view as in the original)
            let ids: &[u32] = if angle == 0.0 { pixel_ids } else { &[] };
            // the things in the rooms of the house the Avatar is in
            // what is inside a house and what outside (lights do not
            // shine from one to the other), only under the open sky
            let (rooms, lights) = if angle == 0.0 && sky {
                let rooms = crate::lighting::rooms(&self.painted, map, *cur_map, infos, rooms_seen);
                let lights: Vec<crate::lighting::Light> = lights
                    .iter()
                    .map(|l| crate::lighting::Light { inside: crate::lighting::light_inside(map, *cur_map, infos, l.item, rooms_seen), ..*l })
                    .collect();
                (rooms, std::borrow::Cow::Owned(lights))
            } else {
                (Vec::new(), std::borrow::Cow::Borrowed(&lights[..]))
            };
            crate::lighting::apply(surf, &lights[..], look, ids, &map.items, infos, &rooms, room_look.filter(|_| !rooms.is_empty()));
        }

        // the way to the place chosen (option "map with guide")
        if self.options.guide {
            crate::guide::draw_marks(surf, &self.guide, map, infos, pixel_ids, &xf, self.tick);
        }

        // speech above the speakers (and the name of what is under the pointer)
        self.draw_barks(surf, &xf, ts, name_ts);
        // upside down the words turn with the world, as before
        if self.inverted {
            self.draw_avatar_words(surf);
        }
        self.mark_sharp(surf.width, surf.height, av_index);
        self.draw_avatar_outline(surf, av_index);
        self.draw_hidden_things(surf);
        self.draw_puzzle_spots(surf);
        self.draw_revealed_path(surf);
        if self.options.modern_controls {
            self.draw_show_all(surf);
            self.draw_hover(surf);
        }
        self.draw_health_bars(surf);
        // the real positions back
        for (idx, pos) in moved {
            let it = &mut self.map_mut().items[idx];
            (it.x, it.y, it.z) = pos;
        }
        if self.inverted {
            surf.flip_rows();
        }
        self.ui.minimap_area = None;
        if self.options.minimap {
            self.draw_minimap(surf, angle, opts.z_limit);
        }
        self.draw_status(surf);
        self.draw_gumps(surf);
        self.draw_window_barks(surf);
        if !self.inverted {
            self.draw_avatar_words(surf);
        }
        self.end_hover_label();
        self.draw_drag(surf);
        self.draw_reader(surf);
        self.draw_slider(surf);
        self.draw_fade(surf);
        self.draw_target_prompt(surf);
        self.draw_worldmap(surf);
        self.draw_menu(surf);
        self.draw_slot_menu(surf);
        self.draw_title(surf);
        self.draw_notice(surf);
    }

    /// With the smooth camera: actors between two steps are put where they
    /// are on their way. Returns the real positions, to be put back after
    /// drawing.
    fn place_between_steps(&mut self) -> Vec<(usize, (i32, i32, i32))> {
        let mut moved = Vec::new();
        if !self.options.smooth_camera || self.ui.menu.is_some() {
            self.lerps.clear();
            return moved;
        }
        let now = std::time::Instant::now();
        let lerps = std::mem::take(&mut self.lerps);
        for (id, l) in lerps {
            let (pos, done) = l.position(now);
            let Some(idx) = self.map().index(id) else { continue };
            let it = &mut self.map_mut().items[idx];
            if (it.x, it.y, it.z) != l.to {
                continue; // moved by something else in the meantime
            }
            if !done {
                moved.push((idx, l.to));
                (it.x, it.y, it.z) = pos;
                self.lerps.insert(id, l);
            }
        }
        moved
    }

    /// Speech above the speakers, and the name of what is under the
    /// pointer (`ts`, `name_ts`: their text sizes).
    fn draw_barks(&self, surf: &mut Surface, xf: &crate::turnview::Xform, ts: f32, name_ts: f32) {
        let Some(font) = &self.data.font else { return };
        let (fonts, pal, infos, map) = (&self.data.fonts, &self.data.pal, &self.data.infos, self.map());
        for bark in &self.barks {
            if bark.item == GUARDIAN {
                // the Guardian is nowhere to be seen: his words at the top edge
                let font = fonts.get(GUARDIAN_FONT).and_then(|f| f.as_ref()).unwrap_or(font);
                let width = (surf.width * 3 / 4).max(120);
                let lines = font.wrap_at(&bark.text, width, ts).len().max(1) as i32;
                font.draw_centered_at(surf, &bark.text, surf.width / 2, 6 + lines * font.height_at(ts), width, pal, ts);
                continue;
            }
            if bark.item == AVATAR && bark.pid != 0 {
                continue; // over the windows (draw_avatar_words)
            }
            let Some(it) = map.get(bark.item) else { continue };
            if !it.in_world() {
                continue; // in a container: shown over its window (draw_window_barks)
            }
            let height = infos.get(it.shape as usize).map_or(0, |si| si.z * 8);
            let (sx, sy) = xf.to_screen(it.x, it.y, it.z);
            // someone far outside the view: not at the edge
            if bark.pid != 0 && bark.item != AVATAR
                && (sx < 0 || sx >= surf.width || sy < 0 || sy - height >= surf.height)
            {
                continue;
            }
            let sy = sy - height - 8;
            // Who speaks decides the colour, as in the original: the
            // Avatar red, things yellow, people one of three colours.
            let colour = match bark.item {
                AVATAR => 6,
                id if id > 256 => 8,
                id => [0, 5, 7][id as usize % 3],
            };
            let font = fonts.get(colour).and_then(|f| f.as_ref()).unwrap_or(font);
            // (pid 0: the name of what is under the pointer, in its own size)
            let ts = if bark.pid == 0 { name_ts } else { ts };
            font.draw_centered_at(surf, &bark.text, sx, sy, (180.0 * ts) as i32, pal, ts);
        }
    }

    /// While a script waits for the player to pick a target.
    fn draw_target_prompt(&self, surf: &mut Surface) {
        if self.ui.targeting.is_none() {
            return;
        }
        let text = self.tr([
            "Ziel w\u{84}hlen (Rechtsklick bricht ab)",
            "Choisir une cible (clic droit pour annuler)",
            "Select a target (right click cancels)",
        ]);
        if let Some(font) = &self.data.font {
            font.draw_centered_at(surf, text, surf.width / 2, surf.height - 8, surf.width - 40, &self.data.pal, self.text_scale());
        }
    }

    /// With the smoothing on: which pixels of the picture just drawn show
    /// the Avatar's patterned armour (chain mail), so that the smoothing
    /// leaves them as they are - it would blur the rings.
    fn mark_sharp(&mut self, w: i32, h: i32, av_index: Option<usize>) {
        self.sharp.clear();
        let (body, head) = (crate::outfit::patterned(self.outfit.0), crate::outfit::helm_patterned(self.outfit.1));
        if !self.options.xbr() || !(body || head) {
            return;
        }
        let Some(av) = av_index else { return };
        let Some(p) = self.painted.iter().find(|p| p.index == av).map(|p| crate::view::Painted { ..*p }) else { return };
        let Some(shape) = self.data.shapes.original(p.shape) else { return };
        let Some(f) = shape.frames.get(p.frame as usize) else { return };
        let turned = self.view_turned();
        self.sharp.resize((w * h) as usize, false);
        let helmet: crate::hash::HashSet<usize> =
            if head { crate::outfit::head_pixels(f, p.frame).unwrap_or_default().into_iter().collect() } else { Default::default() };
        for row in 0..f.height {
            let y = p.sy - f.yoff + row;
            if y < 0 || y >= h {
                continue;
            }
            for col in 0..f.width {
                let i = (row * f.width + col) as usize;
                if !((body && crate::outfit::is_tunic(f.pixels[i])) || helmet.contains(&i)) {
                    continue;
                }
                let x = if p.flipped { p.sx + f.xoff - 1 - col } else { p.sx - f.xoff + col };
                if x < 0 || x >= w {
                    continue;
                }
                // only where he is seen, not where something stands in front
                let shown = if turned {
                    self.turn.pick(x, y, None) == Some(av)
                } else {
                    self.pixel_ids.get((y * w + x) as usize) == Some(&(av as u32 + 1))
                };
                if shown {
                    self.sharp[(y * w + x) as usize] = true;
                }
            }
        }
    }

    /// Tells the helmet (outfit.rs) the Avatar's pictures of falling, lying
    /// and getting up: those of the animations "die" and "stand up".
    pub fn note_fallen_frames(&mut self) {
        let anims = &mut self.data.anims;
        crate::outfit::note_fallen_frames(|| {
            let mut frames = std::collections::BTreeSet::new();
            for action in [crate::anim::DIE, crate::anim::STAND_UP] {
                let Some(a) = anims.action(crate::outfit::AVATAR_SHAPE, action) else { continue };
                for dir in 0..8 {
                    for n in 0..a.size {
                        frames.insert(a.frame(dir, n).frame);
                    }
                }
            }
            frames
        });
    }

    /// Option "show armour": the Avatar's pictures in the colours of the
    /// body armour he wears; made anew only when that changes.
    fn dress_avatar(&mut self) {
        use crate::outfit::{self, Look, AVATAR_SHAPE};
        self.note_fallen_frames();
        let look = match self.equipped(AVATAR, 4) {
            Some(it) if self.options.show_armour => outfit::look_of(it.shape, it.frame),
            _ => Look::Plain,
        };
        let helm = match self.equipped(AVATAR, 3) {
            Some(it) if self.options.show_helm => outfit::helm_of(it.shape, it.frame),
            _ => outfit::Helm::Plain,
        };
        // the magic pieces he wears glow (option "magic glow")
        let mut glow = outfit::Glow::default();
        if self.options.magic_glow {
            for slot in [2, 3, 4, 5] {
                if let Some(it) = self.equipped(AVATAR, slot) {
                    outfit::add_glow(&mut glow, it.shape);
                }
            }
        }
        if (look, helm, glow) == self.outfit {
            return;
        }
        self.outfit = (look, helm, glow);
        let cloth = match look {
            Look::Cloth(n) => self.data.shapes.original(outfit::CLOTHES_SHAPE).map_or(Vec::new(), |s| outfit::cloth_colours(&s, n)),
            _ => Vec::new(),
        };
        let dressed = match (look, helm, self.data.shapes.original(AVATAR_SHAPE)) {
            (Look::Plain, outfit::Helm::Plain, _) if !glow.any() => None,
            (_, _, None) => None,
            (_, _, Some(orig)) => Some(std::rc::Rc::new(outfit::dress(&orig, look, helm, glow, &self.data.pal, &cloth))),
        };
        self.data.shapes.replace(AVATAR_SHAPE, dressed);
        // the turned view keeps models of the pictures it has drawn
        self.turn.forget_models();
    }

    /// Size factor for the text in the world (option "text size"): as in
    /// the original it is drawn in view pixels and so shrinks with the
    /// zoom; otherwise it keeps about its size on the screen. Always a
    /// whole factor (at least 1): in-between factors look washed out.
    /// Size of the name of what is under the pointer (option "names on hover").
    pub fn name_scale(&self) -> f32 {
        let n = self.options.hover_names.min(crate::menu::NAME_SIZES.len() - 1);
        (crate::menu::NAME_SIZES[n] * self.view_zoom).round().max(n as f32).max(1.0)
    }

    pub fn text_scale(&self) -> f32 {
        let f = crate::menu::TEXT_SIZES[self.options.text_size.min(crate::menu::TEXT_SIZES.len() - 1)];
        if f == 0.0 {
            return 1.0;
        }
        (f * self.view_zoom).round().max(1.0)
    }

    /// The answer shown at a view position, if a conversation is open.
    pub fn answer_at(&self, y: i32, view_height: i32) -> Option<usize> {
        let (_, choices) = self.ask.as_ref()?;
        let _ = view_height;
        let line = (y - self.ask_top).div_euclid(self.ask_line.max(1));
        (line >= 0 && (line as usize) < choices.len()).then_some(line as usize)
    }

    /// How far the light of rooms is in now (0..1): towards 1 while the
    /// Avatar is in a house, towards 0 outside, most of the way within half
    /// a second of game time.
    fn room_light(&mut self, in_house: bool) -> f32 {
        let (was, at) = self.room_light;
        let dt = self.tick.wrapping_sub(at) as f32;
        let target = if in_house { 1.0 } else { 0.0 };
        let k = if dt > 90.0 { 1.0 } else { 1.0 - (-dt / 6.0).exp() };
        let now = was + (target - was) * k;
        self.room_light = (now, self.tick);
        now
    }

    /// The general light to draw with now: on the way from the one drawn
    /// last to `target`, most of the way within a second of game time (a
    /// jump of more than a few seconds, as after loading or sleeping, goes
    /// there at once).
    fn ambient_light(&mut self, target: [f32; 3]) -> [f32; 3] {
        let now = self.tick;
        let rgb = match self.ambient_now {
            Some((was, at)) => {
                // (30 ticks are a second)
                let dt = now.wrapping_sub(at) as f32;
                let k = if dt > 90.0 { 1.0 } else { 1.0 - (-dt / 12.0).exp() };
                [0, 1, 2].map(|c| was[c] + (target[c] - was[c]) * k)
            }
            None => target,
        };
        self.ambient_now = Some((rgb, now));
        rgb
    }

    /// The point the view is centred on in this frame.  With the smooth
    /// camera option it glides after the camera target instead of
    /// jumping with every animation step.
    fn view_centre(&mut self) -> (i32, i32, i32) {
        let target = self.camera_position();
        // an earthquake shakes the view directly; it is not smoothed away
        let (qx, qy) = self.quake_offset;
        let quake = (2 * qx + 4 * qy, -2 * qx + 4 * qy);
        let t = ((target.0 - quake.0) as f32, (target.1 - quake.1) as f32, target.2 as f32);
        let now = std::time::Instant::now();
        let far = |c: (f32, f32, f32)| (c.0 - t.0).abs().max((c.1 - t.1).abs()) > 1500.0;
        match self.cam_smooth {
            // The camera is pulled after its target like on a damped spring.
            // A walking figure covers a different distance with every step
            // of its animation; a camera fixed to it would scroll the whole
            // landscape at that uneven pace, which makes fences and paving
            // shimmer.  This way the ground moves evenly and the figure
            // sways a little instead, as its feet do.
            // In a scene that moves the Avatar (in stasis) the view stays on
            // him: scripts walk him a step and let him wait for the others,
            // and a camera still closing up during the wait slid him across
            // the screen in the pose of a step (the march to the execution).
            Some((c, last)) if self.options.smooth_camera && self.ui.menu.is_none() && !far(c) && !self.scene_follows_avatar() => {
                // Once the target stands still the camera closes up quickly:
                // a view creeping on pixel by pixel shimmers as well.
                let since = match self.cam_target {
                    Some((p, since)) if p == t => since,
                    _ => now,
                };
                self.cam_target = Some((t, since));
                let resting = now.duration_since(since).as_secs_f32() > 0.05;
                let smooth_time = if resting { 0.06 } else { 0.2 };
                let dt = now.duration_since(last).as_secs_f32().min(0.1);
                let omega = 2.0 / smooth_time;
                let x = omega * dt;
                let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
                let glide = |c: f32, t: f32, v: &mut f32| {
                    let change = c - t;
                    let temp = (*v + omega * change) * dt;
                    *v = (*v - omega * temp) * decay;
                    t + (change + temp) * decay
                };
                let mut v = self.cam_vel;
                let mut c = (glide(c.0, t.0, &mut v.0), glide(c.1, t.1, &mut v.1), glide(c.2, t.2, &mut v.2));
                let gap = (c.0 - t.0).abs().max((c.1 - t.1).abs()).max((c.2 - t.2).abs());
                // (four world units are one pixel of the view)
                if gap < 1.0 || (resting && gap < 6.0) {
                    // arrived
                    (c, v) = (t, (0.0, 0.0, 0.0));
                }
                self.cam_vel = v;
                self.cam_smooth = Some((c, now));
                (c.0.round() as i32 + quake.0, c.1.round() as i32 + quake.1, c.2.round() as i32)
            }
            _ => {
                self.cam_vel = (0.0, 0.0, 0.0);
                self.cam_smooth = Some((t, now));
                target
            }
        }
    }

    /// The view follows the Avatar while a scene moves him (stasis).
    fn scene_follows_avatar(&self) -> bool {
        self.stasis && matches!(self.camera, crate::effects::CameraMode::Follow(AVATAR))
    }

    /// Is the gliding camera still on its way? Then frames have to be
    /// drawn even though the game state did not change.
    pub fn camera_gliding(&self) -> bool {
        let t = self.camera_position();
        self.options.smooth_camera
            && (!self.lerps.is_empty()
                || self.cam_smooth.is_some_and(|(c, _)| (c.0 - t.0 as f32).abs() + (c.1 - t.1 as f32).abs() + (c.2 - t.2 as f32).abs() >= 1.0)
                || self.cam_vel != (0.0, 0.0, 0.0))
    }

    /// Where an item was drawn in the last frame (view coordinates).
    pub fn shown_at(&self, id: u16) -> Option<(i32, i32)> {
        let idx = self.map().index(id)?;
        self.painted.iter().find(|p| p.index == idx).map(|p| (p.sx, p.sy))
    }

    /// Is the backpack on the Avatar's back drawn at a view position? It is
    /// part of his pictures, so it is told by its brown colours, looked up
    /// in the original pictures (with "show armour" a leather tunic is
    /// brown as well). A few brown pixels of the belt do not count: around
    /// the point there has to be more of the pack.
    pub fn backpack_at(&mut self, x: i32, y: i32) -> bool {
        const PACK: std::ops::RangeInclusive<u16> = 213..=223;
        if self.view_turned() || self.pick_world(x, y, 0) != Some(AVATAR) {
            return false;
        }
        let Some(av) = self.map().index(AVATAR) else { return false };
        let Some(p) = self.painted.iter().rev().find(|p| p.index == av) else { return false };
        let Some(shape) = self.data.shapes.original(crate::outfit::AVATAR_SHAPE) else { return false };
        let Some(f) = shape.frames.get(p.frame as usize) else { return false };
        let col = if p.flipped { p.sx + f.xoff - 1 - x } else { x - (p.sx - f.xoff) };
        let row = self.unflip_y(y) - (p.sy - f.yoff);
        let brown = |c: i32, r: i32| {
            c >= 0 && r >= 0 && c < f.width && r < f.height && PACK.contains(&{ f.pixels[(r * f.width + c) as usize] })
        };
        if !brown(col, row) {
            return false;
        }
        let around = (-2..=2).flat_map(|dr| (-2..=2).map(move |dc| (dc, dr))).filter(|&(dc, dr)| brown(col + dc, row + dr)).count();
        around >= 10
    }

    /// A row of the view as the world was drawn: with the view upside
    /// down (a lever in the game turns it), what is shown at row `y` was
    /// drawn at the mirrored row. Clicks on the world go through this, so
    /// that they hit what is seen there; walking by keys or towards the
    /// pointer stays turned round, as in the original.
    pub fn unflip_y(&self, y: i32) -> i32 {
        if self.inverted { self.view_size.1 - 1 - y } else { y }
    }

    /// The object id of the topmost item drawn at a screen position.
    pub fn pick(&mut self, x: i32, y: i32) -> Option<u16> {
        // open containers lie on top of the world
        if let Some((_, hit)) = self.pick_gump(x, y) {
            return hit;
        }
        let top = self.pick_world(x, y, 0);
        // (a key hidden behind it right there, option "hidden keys")
        self.pick_hidden(x, y, top)
    }

    /// The topmost item of the world view at a position, ignoring `skip`.
    pub fn pick_world(&mut self, x: i32, y: i32, skip: u16) -> Option<u16> {
        let y = self.unflip_y(y);
        if self.view_turned() {
            // the turned picture keeps what is drawn at every pixel
            let skip_index = if skip != 0 { self.map().index(skip) } else { None };
            let index = self.turn.pick(x, y, skip_index)?;
            let id = self.map().items.get(index)?.id;
            return (id != 0 && id != skip).then_some(id);
        }
        for p in self.painted.iter().rev() {
            let Some(shape) = self.data.shapes.get(p.shape) else { continue };
            let Some(f) = shape.frames.get(p.frame as usize) else { continue };
            let col = if p.flipped { p.sx + f.xoff - 1 - x } else { x - (p.sx - f.xoff) };
            let row = y - (p.sy - f.yoff);
            if col < 0 || row < 0 || col >= f.width || row >= f.height {
                continue;
            }
            if f.pixels[(row * f.width + col) as usize] != shape::TRANSPARENT {
                let Some(id) = self.map().items.get(p.index).map(|it| it.id) else { continue };
                if id != 0 && id != skip {
                    // (option: a revealed tile of the invisible path over the lava)
                    return Some(self.revealed_tile_at(x, y, id).unwrap_or(id));
                }
            }
        }
        None
    }

    /// Is there a floor or roof between something not solid (`low`) and
    /// what stands higher in it (`high`, item `skip`)? A tall fire in a
    /// house reaches through the roof, but one on the roof does not burn.
    /// (Only a floor more than a step above the foot of `low` counts.)
    pub fn floor_between(&self, low: &WorldBox, high: &WorldBox, skip: u16) -> bool {
        if high.z <= low.z + 24 {
            return false;
        }
        self.map().items_near(high.x, high.y, 640).into_iter().any(|i| {
            let other = &self.map().items[i];
            let Some(oi) = self.data.infos.get(other.shape as usize) else { return false };
            if other.id == skip || !(oi.solid || oi.roof) {
                return false;
            }
            let ob = WorldBox::of(other, oi);
            ob.z > low.z + 24 && ob.z <= high.z && high.overlaps_xy(&ob)
        })
    }

    /// An actor has moved: everything it newly stands on or runs into is
    /// told so ("got hit" script). This is how water drowns, lava burns
    /// and pressure plates notice someone. `force`: how hard the step was
    /// (see `effects::hit_force`), 0 for something put down.
    pub fn touch_items(&mut self, actor: u16, force: u16) {
        const EVENT_GOT_HIT: u32 = 6;
        const EVENT_HIT: u32 = 5;
        const EVENT_RELEASE: u32 = 9;
        const EVENT_JUST_MOVED: u32 = 0x12;
        let Some(it) = self.map().get(actor).copied().filter(|it| it.in_world()) else {
            // taken out of the world (picked up): what it lay on is let go
            for id in self.touching.remove(&actor).unwrap_or_default() {
                self.call_event(id, EVENT_RELEASE, &[]);
            }
            return;
        };
        let Some(info) = self.data.infos.get(it.shape as usize).copied() else { return };
        let b = WorldBox::of(&it, &info);
        // (a mover that cares what it hits is told of everything, also of
        // things without a script of their own: markers it turns at)
        let mover_hits = !it.is_actor() && self.scripts.usecode.event(it.shape, EVENT_HIT).is_some();
        let mut now = Vec::new();
        for i in self.map().items_near(it.x, it.y, 640) {
            let other = &self.map().items[i];
            if other.id == actor || other.id == 0 {
                continue;
            }
            if !mover_hits && self.scripts.usecode.event(other.shape, EVENT_GOT_HIT).is_none() && self.scripts.usecode.event(other.shape, EVENT_RELEASE).is_none() {
                continue;
            }
            let Some(oi) = self.data.infos.get(other.shape as usize) else { continue };
            let ob = WorldBox::of(other, oi);
            // standing on it, or inside it (one stands only on what is solid
            // or flat: the Avatar on a roof touched the top of a tall fire
            // in the house below and burnt)
            let on_top = ob.z + ob.zd == b.z && (oi.solid || ob.zd == 0);
            if !oi.solid && !on_top && self.floor_between(&ob, &b, actor) {
                continue;
            }
            if b.overlaps_xy(&ob) && (on_top || b.overlaps(&ob)) {
                now.push((other.id, on_top));
            } else if mover_hits && b.x >= ob.x - ob.xd && b.x - b.xd <= ob.x && b.y >= ob.y - ob.yd && b.y - b.yd <= ob.y
                && b.z < ob.z + ob.zd && b.z + b.zd > ob.z
            {
                // (for such a mover, brushing its side counts too)
                now.push((other.id, false));
            }
        }
        let ids: Vec<u16> = now.iter().map(|n| n.0).collect();
        let before = self.touching.insert(actor, ids.clone()).unwrap_or_default();
        // stepped off: pressure plates spring back, sinking floors stop
        let mut released = false;
        for id in before.iter().filter(|id| !ids.contains(id)) {
            released = true;
            self.call_event(*id, EVENT_RELEASE, &[]);
        }
        if released {
            self.call_event(actor, EVENT_RELEASE, &[]);
        }
        if actor == AVATAR {
            // (the Avatar's own script, for spells that follow him)
            self.call_event(actor, EVENT_JUST_MOVED, &[]);
        }
        for (id, floor) in now {
            // The ground underfoot is told with every step (its script
            // keeps itself from running twice); other things only when
            // the contact is new.
            if floor || !before.contains(&id) {
                // arguments: who touched, and with how much force
                let mut args = actor.to_le_bytes().to_vec();
                args.extend_from_slice(&force.to_le_bytes());
                self.call_event(id, EVENT_GOT_HIT, &args);
                if !floor {
                    self.call_event(actor, EVENT_HIT, &[id.to_le_bytes(), force.to_le_bytes()].concat());
                }
            }
        }
    }

    /// The player looks at (single click) or uses (double click) an item.
    pub fn interact(&mut self, id: u16, use_it: bool) {
        if self.stasis || self.ask.is_some() || self.ui.reader.is_some() || self.ui.slider.is_some() {
            return;
        }
        // searching a body: its belongings show in the window for the dead
        if use_it && id != AVATAR && self.states.get(&id).is_some_and(|s| s.dead) {
            const BODY_GUMP: u32 = 12;
            if self.ui.gumps.iter().any(|g| g.owner == id) {
                self.close_gump(id);
            } else {
                self.open_gump(id, BODY_GUMP);
            }
            return;
        }
        // drinking from a stack of potions: one comes off it and is drunk
        let id = if use_it { self.one_potion_off(id) } else { id };
        // a bed (or the bedroll) is already asking or sleeping: a second one
        // started meanwhile (the other half of the same bed) would leave
        // the Avatar frozen for good
        if use_it && self.bed_busy(id) {
            return;
        }
        if !use_it && self.look_at_puzzle_spot(id) {
            return;
        }
        self.call_event(id, if use_it { 1 } else { 0 }, &[]);
    }

    /// Runs the "cachein" scripts of the current map's items. The game
    /// does this when a map is entered; it starts the map's music.
    pub fn cache_in(&mut self) {
        const EVENT_CACHEIN: u32 = 4;
        let ids: Vec<u16> = self
            .map()
            .items
            .iter()
            .filter(|it| it.in_world() && it.id >= 256 && self.scripts.usecode.event(it.shape, EVENT_CACHEIN).is_some())
            .map(|it| it.id)
            .collect();
        for id in ids {
            self.call_event(id, EVENT_CACHEIN, &[]);
        }
    }

    /// The sound of a foot coming down depends on the ground: stone,
    /// water and wooden planks have their own, everything else the default.
    fn play_footstep(&mut self, actor: &Item, running: bool) {
        let Some(si) = self.data.infos.get(actor.shape as usize) else { return };
        let b = WorldBox::of(actor, si);
        let info = self.map().position_info(&b, &b, &self.data.infos, actor.id);
        let (true, Some(floor)) = (info.supported, info.land_shape) else { return };
        let sfx = match floor {
            0x03 | 0x04 | 0x09 | 0x0B | 0x5C | 0x5E => 0x2B,
            0x7E | 0x80 => 0xCD,
            0xA1..=0xA4 => if running { 0x99 } else { 0x91 },
            _ => if running { 0x97 } else { 0x90 },
        };
        // every step sounds a little different
        let pitch = 1.0 + ((self.rng.next() % 0x2000) as f32 - 4096.0) / 65536.0;
        let volume = self.sfx_volume(actor.id);
        self.audio.play_sfx_pitched(sfx, actor.id, volume, pitch);
    }

    /// The sound options take effect, and with spatial sound the effects
    /// of objects follow where they are: from the left or right of the
    /// view, quieter the farther they are from the Avatar.
    pub fn place_sounds(&mut self) {
        self.audio.set_soundfont(self.options.better_music);
        self.audio.set_album(self.options.album_music);
        self.audio.set_better(self.options.better_sound);
        self.audio.set_spatial(self.options.spatial_sound);
        if !self.audio.spatial() {
            return;
        }
        let half = (self.view_size.0 / 2).max(1) as f32;
        let spots: Vec<(u16, f32, f32)> = self
            .audio
            .sound_objects()
            .into_iter()
            .filter_map(|o| {
                if o == AVATAR {
                    // the view is centred on him; also while the camera catches up
                    return Some((o, 0.0, 1.0));
                }
                let it = self.map().get(o).copied().filter(|it| it.in_world())?;
                let (sx, _) = self.to_screen(it.x, it.y, it.z);
                // never quite all on one side
                let pan = ((sx as f32 - half) / half).clamp(-1.0, 1.0) * 0.75;
                Some((o, pan, self.sfx_volume(o)))
            })
            .collect();
        self.audio.place_sounds(&spots);
    }

    /// How loud a sound made by an object is for the player (0.0..=1.0).
    pub fn sfx_volume(&self, object: u16) -> f32 {
        let Some(it) = self.map().get(object).filter(|it| it.in_world() && object != 0) else { return 1.0 };
        let av = self.avatar();
        let dist = (it.x - av.x).abs().max((it.y - av.y).abs()) as f32;
        (1.0 - dist / 1600.0).clamp(0.0, 1.0)
    }

    /// Shows a short message at the top of the view for a few seconds.
    pub fn notify(&mut self, text: &str) {
        self.ui.notice = Some((text.to_string(), std::time::Instant::now()));
    }

    fn draw_notice(&mut self, surf: &mut Surface) {
        let Some((text, since)) = &self.ui.notice else { return };
        if since.elapsed().as_secs_f32() > 3.0 {
            self.ui.notice = None;
            return;
        }
        let ts = self.text_scale();
        if let Some(font) = &self.data.font {
            font.draw_centered_at(surf, text, surf.width / 2, 12 + font.height_at(ts), surf.width - 40, &self.data.pal, ts);
        }
    }

    /// Saves the game and tells the player how it went.
    pub fn quick_save(&mut self) {
        let msg = match (self.save_game(), self.language) {
            (Ok(_), 'G') => "Spiel gespeichert.".to_string(),
            (Ok(_), 'F') => "Partie sauvegard\u{82}e.".to_string(),
            (Ok(_), _) => "Game saved.".to_string(),
            (Err(e), _) => {
                eprintln!("save: {e}");
                e
            }
        };
        self.notify(&msg);
    }

    /// Loads the saved game and tells the player how it went.
    pub fn quick_load(&mut self) {
        let msg = match (self.load_game(), self.language) {
            (Ok(()), 'G') => "Spiel geladen.".to_string(),
            (Ok(()), 'F') => "Partie charg\u{82}e.".to_string(),
            (Ok(()), _) => "Game loaded.".to_string(),
            (Err(e), _) => {
                eprintln!("load: {e}");
                e
            }
        };
        self.notify(&msg);
    }

    /// One-line description of an item for debugging.
    pub fn describe(&self, it: &Item) -> String {
        let family = self.data.infos.get(it.shape as usize).map_or(0, |si| si.family);
let si = self.data.infos.get(it.shape as usize).copied().unwrap_or_default();
        let kind: String = [(si.solid, 's'), (si.land, 'l'), (si.sea, 'w'), (si.damaging, 'd'), (si.roof, 'r'), (si.fixed, 'f')]
            .iter()
            .filter(|k| k.0)
            .map(|k| k.1)
            .collect();
        format!("id {:5} shape {:4} frame {:3} at {},{},{} flags {:04X} q {} npc {} map {} parent {} family {} [{}] pad {}x{}x{}",
                it.id, it.shape, it.frame, it.x, it.y, it.z, it.flags, it.quality, it.npc, it.mapnum, it.parent, family, kind, si.x, si.y, si.z)
    }
}

/// Height from which on everything is hidden because the Avatar stands
/// below a roof. Only the middle half of his footprint counts, as in the
/// original: with the whole of it, an eave he merely brushed while walking
/// past a house hid the upper floor for a moment.
pub fn roof_above(map: &MapData, infos: &[crate::shape::ShapeInfo], avatar: &Item) -> Option<i32> {
    let (xd, yd) = infos.get(avatar.shape as usize).map_or((64, 64), |si| { let b = WorldBox::of(avatar, si); (b.xd, b.yd) });
    let probe = WorldBox { x: avatar.x - xd / 4, y: avatar.y - yd / 4, z: avatar.z, xd: xd / 2, yd: yd / 2, zd: 16 };
    map.position_info(&probe, &probe, infos, AVATAR).roof_z
}
