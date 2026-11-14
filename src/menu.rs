//! The options menu, built from the game's own graphics: the Pagan logo
//! and the golden slider the game uses for picking amounts.  It holds the
//! volume sliders and the optional improvements that can be switched on
//! and off, sorted onto four tabs (sound, graphics, controls, display);
//! the keys are at the bottom of the controls tab.

use crate::game::Game;
use crate::view::Surface;
use std::path::PathBuf;

const LOGO_SHAPE: u32 = 32;
pub const SLIDER_SHAPE: u32 = 41;
/// The groove of the slider graphic, relative to its top left corner.
pub const TRACK_X0: i32 = 47;
pub const TRACK_X1: i32 = 141;
pub const TRACK_Y: i32 = 23;
/// Middle of the little window that shows the value.
pub const VALUE_X: i32 = 25;
/// Room between the logo and the first row, for the title.
const LOGO_GAP: i32 = 24;
/// Every row of the menu is this high and this wide.
const ROW: i32 = 13;
const PANEL_W: i32 = 260;
/// Space between the volumes, the switches and the actions.
const GROUP_GAP: i32 = 10;
/// Where the labels begin (left of them: the stud).
const LABEL_X: i32 = 20;
/// On a tab: where its label begins (left of it: the stud).
const TAB_LABEL_X: i32 = 12;
/// The volume bar: from here to this far before the right edge.
const BAR_X0: i32 = 96;
const BAR_RIGHT: i32 = 40;

pub const MUSIC: usize = 0;
pub const SOUND: usize = 1;
pub const SPEECH: usize = 2;
const SLIDERS: usize = 3;
/// Rows at the bottom of the controls tab: show the keys, choose the keys.
const ACTIONS: usize = 2;
const TABS: usize = 4;
/// The tab with the controls, which has the keys at its bottom.
const CONTROLS_TAB: usize = 2;
/// The switches on each tab; the sound tab has the volumes above them.
const TAB_SWITCHES: [&[Switch]; TABS] = [
    &[Switch::Music, Switch::BetterSound, Switch::SpatialSound, Switch::VictoryFanfare],
    &[Switch::SmoothCamera, Switch::EvenWalk, Switch::TurnView, Switch::Lighting, Switch::Shadows, Switch::Storm, Switch::Pixels, Switch::ShowArmour, Switch::ShowHelm, Switch::MagicGlow, Switch::WideView, Switch::MonsterFade, Switch::AvatarOutline],
    &[Switch::Controls, Switch::HoldWalk, Switch::RoadKeys, Switch::NoticeFar, Switch::TalkTruce, Switch::AvoidFlames, Switch::SleepUntil, Switch::PentagramSnap, Switch::PentagramReach],
    &[Switch::EnemyHealth, Switch::TextSize, Switch::HoverNames, Switch::Minimap, Switch::Guide, Switch::PuzzleSpots, Switch::BetterInventory, Switch::Journal, Switch::Practice],
];

/// The switches of the menu, in the order of the settings file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Switch {
    SmoothCamera,
    EnemyHealth,
    Controls,
    RoadKeys,
    TurnView,
    Lighting,
    Pixels,
    Music,
    SpatialSound,
    TextSize,
    ShowArmour,
    ShowHelm,
    Minimap,
    BetterInventory,
    HoverNames,
    HoldWalk,
    Journal,
    Practice,
    AlbumMusic,
    BetterSound,
    WideView,
    Shadows,
    SleepUntil,
    NoticeFar,
    TalkTruce,
    VictoryFanfare,
    AvatarOutline,
    AvatarMenu,
    Guide,
    PuzzleSpots,
    Storm,
    PentagramSnap,
    PentagramReach,
    AvoidFlames,
    MonsterFade,
    MagicGlow,
    EvenWalk,
}

impl Switch {
    const ALL: [Switch; 37] = [
        Switch::SmoothCamera, Switch::EnemyHealth, Switch::Controls, Switch::RoadKeys, Switch::TurnView, Switch::Lighting, Switch::Pixels, Switch::Music, Switch::SpatialSound, Switch::TextSize, Switch::ShowArmour, Switch::ShowHelm, Switch::Minimap, Switch::BetterInventory, Switch::HoverNames, Switch::HoldWalk, Switch::Journal, Switch::Practice, Switch::AlbumMusic, Switch::BetterSound, Switch::WideView, Switch::Shadows, Switch::SleepUntil, Switch::NoticeFar, Switch::TalkTruce, Switch::VictoryFanfare, Switch::AvatarOutline, Switch::AvatarMenu, Switch::Guide, Switch::PuzzleSpots, Switch::Storm, Switch::PentagramSnap, Switch::PentagramReach, Switch::AvoidFlames, Switch::MonsterFade, Switch::MagicGlow, Switch::EvenWalk,
    ];

    /// Key in the settings file.
    fn key(self) -> &'static str {
        match self {
            Switch::SmoothCamera => "smooth_camera",
            Switch::EnemyHealth => "enemy_health",
            Switch::Controls => "modern_controls",
            Switch::RoadKeys => "road_keys",
            Switch::TurnView => "turn_view",
            Switch::Lighting => "lighting",
            Switch::Pixels => "smooth_pixels",
            Switch::Music => "better_music",
            Switch::SpatialSound => "spatial_sound",
            Switch::TextSize => "text_size",
            Switch::ShowArmour => "show_armour",
            Switch::ShowHelm => "show_helm",
            Switch::Minimap => "minimap",
            Switch::BetterInventory => "better_inventory",
            Switch::HoverNames => "hover_names",
            Switch::HoldWalk => "hold_walk",
            Switch::Journal => "journal",
            Switch::Practice => "practice",
            Switch::AlbumMusic => "album_music",
            Switch::BetterSound => "better_sound",
            Switch::WideView => "wide_view",
            Switch::Shadows => "shadows",
            Switch::SleepUntil => "sleep_until",
            Switch::NoticeFar => "notice_far",
            Switch::TalkTruce => "talk_truce",
            Switch::VictoryFanfare => "victory_fanfare",
            Switch::AvatarOutline => "avatar_outline",
            Switch::AvatarMenu => "avatar_menu",
            Switch::Guide => "guide",
            Switch::PuzzleSpots => "puzzle_spots",
            Switch::Storm => "storm_colours",
            Switch::PentagramSnap => "pentagram_snap",
            Switch::PentagramReach => "pentagram_reach",
            Switch::AvoidFlames => "avoid_flames",
            Switch::MonsterFade => "monster_fade",
            Switch::MagicGlow => "magic_glow",
            Switch::EvenWalk => "even_walk",
        }
    }

    /// Name in the menu: German, French, English.
    fn names(self) -> [&'static str; 3] {
        match self {
            Switch::SmoothCamera => ["Weiche Bewegung", "Mouvement fluide", "Smooth movement"],
            Switch::EnemyHealth => ["Lebensbalken der Gegner", "Barre de vie des ennemis", "Enemy health bars"],
            Switch::Controls => ["Steuerung", "Commandes", "Controls"],
            Switch::RoadKeys => ["Tasten entlang der Wege", "Touches le long des chemins", "Keys follow the roads"],
            Switch::TurnView => ["Drehbare Ansicht", "Vue orientable", "Turnable view"],
            Switch::Lighting => ["Beleuchtung", "Eclairage", "Lighting"],
            Switch::Pixels => ["Grafikfilter", "Filtre graphique", "Graphics filter"],
            Switch::Music => ["Musik", "Musique", "Music"],
            Switch::SpatialSound => ["R\u{84}umlicher Klang", "Son spatial", "Spatial sound"],
            Switch::TextSize => ["Schriftgr\u{94}\u{e1}e", "Taille du texte", "Text size"],
            Switch::ShowArmour => ["R\u{81}stung sichtbar", "Armure visible", "Show armour"],
            Switch::ShowHelm => ["Helm sichtbar", "Casque visible", "Show helmet"],
            Switch::Minimap => ["Minikarte", "Mini-carte", "Minimap"],
            Switch::BetterInventory => ["Besseres Inventar", "Inventaire am\u{82}lior\u{82}", "Better inventory"],
            Switch::HoverNames => ["Namen beim Zeigen", "Noms sous le pointeur", "Names on hover"],
            Switch::HoldWalk => ["Taste halten: folgen", "Bouton tenu : suivre", "Hold button to follow"],
            Switch::Journal => ["Tagebuch (Taste T)", "Journal (touche T)", "Quest journal (key T)"],
            Switch::Practice => ["Fortschritt der Werte", "Progr\u{8A}s des attributs", "Attribute progress"],
            Switch::AlbumMusic => ["Album-Musik", "Musique de l'album", "Album music"],
            Switch::BetterSound => ["Klang verbessern", "Son am\u{82}lior\u{82}", "Better sound"],
            Switch::WideView => ["Breitbild", "Vue large", "Wide view"],
            Switch::Shadows => ["Schatten", "Ombres", "Shadows"],
            Switch::SleepUntil => ["Schlafen bis Tageszeit", "Dormir jusqu'\u{85} une heure", "Sleep until a time of day"],
            Switch::NoticeFar => ["Gegner bemerken fr\u{81}her", "Ennemis vigilants", "Enemies notice sooner"],
            Switch::TalkTruce => ["Gegner warten im Gespr\u{84}ch", "Tr\u{88}ve pendant les dialogues", "Enemies wait during talks"],
            Switch::VictoryFanfare => ["Sieges-Fanfare", "Fanfare de victoire", "Victory fanfare"],
            Switch::AvatarOutline => ["Umriss, wenn verdeckt", "Contour si cach\u{82}", "Outline when hidden"],
            Switch::AvatarMenu => ["Avatar-Men\u{81} (Cheats)", "Menu de l'Avatar (triches)", "Avatar menu (cheats)"],
            Switch::Guide => ["Karte mit Wegweiser (Taste O)", "Carte et chemin (touche O)", "Map with guide (key O)"],
            Switch::PuzzleSpots => ["R\u{84}tselstellen", "Lieux d'\u{82}nigme", "Puzzle spots"],
            Switch::Storm => ["Gewitterfarben", "Couleurs de l'orage", "Storm colours"],
            Switch::PentagramSnap => ["Pentagramm einrasten", "Pentacle aimant\u{82}", "Pentacle snaps"],
            Switch::PentagramReach => ["Reichweite am Pentagramm", "Port\u{82}e au pentacle", "Reach at the pentacle"],
            Switch::AvoidFlames => ["Figuren meiden Flammen", "On \u{82}vite les flammes", "People avoid flames"],
            Switch::MonsterFade => ["Gegner blenden ein", "Ennemis en fondu", "Enemies fade in"],
            Switch::MagicGlow => ["Magisches Leuchten", "Lueur magique", "Magic glow"],
            Switch::EvenWalk => ["Gleichm\u{84}\u{e1}iges Gehen", "Marche r\u{82}guli\u{8a}re", "Even walking"],
        }
    }

    /// Switches with more steps than on and off: stored as their step.
    fn has_steps(self) -> bool {
        matches!(self, Switch::TextSize | Switch::HoverNames | Switch::AvatarOutline | Switch::Storm | Switch::Pixels)
    }
}

/// A row of the menu.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Tabs,
    Slider(usize),
    Switch(Switch),
    Action(usize),
}

impl Row {
    /// Rows of one group stand together; groups are set apart by a gap.
    fn group(self) -> u8 {
        match self {
            Row::Tabs => 0,
            Row::Slider(_) => 1,
            Row::Switch(_) => 2,
            Row::Action(_) => 3,
        }
    }
}

/// The rows of a tab, from the top.
fn tab_rows(tab: usize) -> Vec<Row> {
    let mut rows = vec![Row::Tabs];
    if tab == 0 {
        rows.extend((0..SLIDERS).map(Row::Slider));
    }
    rows.extend(TAB_SWITCHES[tab].iter().map(|&sw| Row::Switch(sw)));
    if tab == CONTROLS_TAB {
        rows.extend((0..ACTIONS).map(Row::Action));
    }
    rows
}

/// Height of a list of rows with the gaps between the groups.
fn rows_height(rows: &[Row], gap: i32, rh: i32) -> i32 {
    let gaps = rows.windows(2).filter(|w| w[0].group() != w[1].group()).count() as i32;
    rows.len() as i32 * rh + gaps * gap
}

/// The tallest tab: the menu keeps its place when another tab is chosen.
fn tallest(gap: i32, rh: i32) -> i32 {
    (0..TABS).map(|t| rows_height(&tab_rows(t), gap, rh)).max().unwrap_or(0)
}

/// Optional changes to how the original game behaves.
#[derive(Clone, Copy)]
pub struct Options {
    /// Figures glide from step to step and the view glides along,
    /// instead of both jumping with every animation frame.
    pub smooth_camera: bool,
    /// With smooth movement: walking and running figures glide at the even
    /// pace of the whole animation instead of the pace of each frame, which
    /// changes twice a step (game.rs, `Lerp::Glide`).
    pub even_walk: bool,
    /// A small health bar above the creatures that fight the Avatar.
    pub enemy_health: bool,
    /// A click uses what it hits, a key uses what is at hand, the right
    /// button looks instead of walking.
    pub modern_controls: bool,
    /// With the modern controls: a click on the ground walks there.
    pub click_walk: bool,
    /// The direction keys are turned by an eighth: each one walks along
    /// the roads and walls, which run slanted across the view.
    pub road_keys: bool,
    /// The view can be turned to the other sides (keys , and .).
    pub turn_view: bool,
    /// Light by the time of day, dimmer underground; lamps, fires and lava
    /// light up their surroundings (see lighting.rs).
    pub lighting: bool,
    /// How the picture is shown when it is larger than drawn: sharp, smoothed
    /// (xBR) or as on a tube monitor (CRT), see `PIXELS_*`.
    pub pixels: usize,
    /// Zoom step (index into `ZOOMS`), changed with + and -.
    pub zoom: usize,
    /// Size of the text in the world (index into `TEXT_SIZES`).
    pub text_size: usize,
    /// The music is played with a SoundFont instead of the Windows synthesizer.
    pub better_music: bool,
    /// Effects come from the side of their source and follow its distance.
    pub spatial_sound: bool,
    /// The Avatar's tunic takes the colours of the armour he wears.
    pub show_armour: bool,
    /// The Avatar's helmet takes the colours of the helmet he wears.
    pub show_helm: bool,
    /// A small map of the surroundings in the top right corner (key M).
    pub minimap: bool,
    /// Containers open as a larger window with fixed slots, page by page.
    pub better_inventory: bool,
    /// The name of what is under the pointer is shown over it: 0 = not,
    /// otherwise the size (index into `NAME_SIZES`).
    pub hover_names: usize,
    /// With walking by click: the button held on the ground makes the
    /// Avatar follow the pointer (running when it is far away).
    pub hold_walk: bool,
    /// A journal of the quests (key T) that notes what the Avatar has
    /// learned and done.
    pub journal: bool,
    /// The Avatar's window shows how far strength, intelligence and
    /// dexterity are from their next point (details under the pointer).
    pub practice: bool,
    /// The music comes from the remastered album where its files are there.
    pub album_music: bool,
    /// Sound effects and voices are made anew at 44.1 kHz, without the
    /// buzz and hiss of the 8 bit samples (soundfix.rs).
    pub better_sound: bool,
    /// In the daytime a few townsfolk walk the streets of Tenebrae
    /// (passersby.rs). Not a switch of the menu for now (it caused trouble):
    /// off, only tests turn it on (`option:passersby,on`).
    pub passersby: bool,
    /// The figures cast shadows on the ground (shadows.rs).
    pub shadows: bool,
    /// Beds ask until which time of day to sleep, the Avatar asks (sleep.rs).
    pub sleep_until: bool,
    /// Hostile creatures notice the Avatar half as far again as in the
    /// original, whose small picture let them come into view late
    /// (combat.rs); not while a scene plays.
    pub notice_far: bool,
    /// Enemies do not attack the Avatar or walk up to him while a
    /// conversation goes on; the fight goes on afterwards (combat.rs). The original lets
    /// the world run on during conversations.
    pub talk_truce: bool,
    /// A spell book (key N) that notes the spells the Avatar has learned
    /// from books, scrolls and teachers, with words and reagents. Taken out
    /// of the menu for now (it shows only the earth spells): always off; the
    /// spells learned are still kept in the savegame.
    pub spell_notes: bool,
    /// Walking through a gate or into a teleporter, the view fades to
    /// black and in again on the other side (eggs.rs). Not a switch of the
    /// menu: always on, only tests turn it off (`option:map_fade,off`).
    pub map_fade: bool,
    /// A fanfare plays when an enemy falls while the weapon is out, as in
    /// the original (off: the fight music just goes on) (combat.rs). Off
    /// unless switched on.
    pub victory_fanfare: bool,
    /// Where something stands in front of the Avatar, his outline shows
    /// through it once this much of him is hidden (index into
    /// `OUTLINE_SHARES`, 0 = never) (silhouette.rs).
    pub avatar_outline: usize,
    /// The picture fills windows and screens of other proportions than
    /// the original's: more of the world is seen to the sides.
    pub wide_view: bool,
    /// Looking at the Avatar opens the debug menu the original has for
    /// cheaters (spell book, time, keys, ...), as if started with --cheat.
    /// Not in the menu: only `avatar_menu=on` in settings.ini switches it on.
    /// Off as in the original.
    pub avatar_menu: bool,
    /// A map of the surroundings (key O) with a list of places: the way to
    /// the place chosen shows on the ground (guide.rs, worldmap.rs).
    pub guide: bool,
    /// Places a puzzle needs that are easy to miss (the hollow for the
    /// Skull of Quakes) get a name under the pointer and with key H, and
    /// glow while the Avatar carries what belongs there (puzzles.rs).
    pub puzzle_spots: bool,
    /// The colours of the rain storm (after Hydros is freed): 0 as in the
    /// original (washed out to grey), 1 gentler, 2 none (only the thunder).
    pub storm: usize,
    /// Candles, reagents and the focus put down on a sorcerer's pentacle
    /// settle on its right spots (puzzles.rs).
    pub pentagram_snap: bool,
    /// Standing near a pentacle, the Avatar reaches all of it: puts things
    /// down anywhere on it, picks them up, lights the candles (puzzles.rs).
    pub pentagram_reach: bool,
    /// People keep out of flames that would hurt them (Malchir and his own
    /// ring of fire).
    pub avoid_flames: bool,
    /// A creature that comes out of a monster egg in sight fades in
    /// instead of being there all at once (the view shows more than the
    /// original, which made them appear where it could be seen).
    pub monster_fade: bool,
    /// Not a switch: the end of the game has been seen, so the main menu
    /// shows the quotes and the end film (as the original remembers it).
    pub seen_end: bool,
    /// The magic pieces the Avatar wears (helmet, body armour, arms, legs)
    /// glow blue at their outline, as their pictures as things do.
    pub magic_glow: bool,
}

/// Sizes of the names under the pointer, relative to the original text
/// at 100 % (0: no names). Rounded to whole factors like all text, but
/// each at least its own number, so that the three always differ.
pub const NAME_SIZES: [f32; 4] = [0.0, 1.0, 2.0, 3.0];

/// Text sizes: 0 = as in the original (the text shrinks and grows with
/// the zoom); otherwise the size on the screen whatever the zoom, relative
/// to the original at 100 %.
pub const TEXT_SIZES: [f32; 4] = [0.0, 1.0, 1.5, 2.0];

/// How much of the Avatar must be hidden before his outline shows, in
/// percent (0: never).
pub const OUTLINE_SHARES: [usize; 5] = [0, 25, 50, 80, 100];
/// Mostly hidden.
pub const OUTLINE_DEFAULT: usize = 3;

/// Size of the view relative to the original 640x400, in percent: below
/// 100 the world looks bigger, above 100 more of it is seen.
pub const ZOOMS: [i32; 5] = [75, 100, 125, 150, 200];
/// The original size.
pub const ZOOM_NORMAL: usize = 1;

impl Default for Options {
    fn default() -> Options {
        Options { smooth_camera: true, even_walk: true, enemy_health: true, modern_controls: true, click_walk: true, road_keys: true, turn_view: true, lighting: true, pixels: PIXELS_SHARP, zoom: ZOOM_NORMAL, text_size: 2, better_music: true, spatial_sound: true, show_armour: true, show_helm: true, minimap: true, better_inventory: true, hover_names: 0, hold_walk: true, journal: true, practice: true, album_music: true, better_sound: true, passersby: false, wide_view: true, shadows: true, sleep_until: true, notice_far: true, talk_truce: true, spell_notes: false, map_fade: true, victory_fanfare: false, avatar_outline: OUTLINE_DEFAULT, avatar_menu: false, guide: true, puzzle_spots: true, storm: 1, pentagram_snap: true, pentagram_reach: true, avoid_flames: true, monster_fade: true, seen_end: false, magic_glow: true }
    }
}

/// The steps of the option "graphics filter" (`Options::pixels`).
pub const PIXELS_SHARP: usize = 0;
pub const PIXELS_XBR: usize = 1;
pub const PIXELS_CRT: usize = 2;

impl Options {
    /// The picture is smoothed with xBR.
    pub fn xbr(&self) -> bool {
        self.pixels == PIXELS_XBR
    }

    fn get(&self, switch: Switch) -> bool {
        match switch {
            Switch::SmoothCamera => self.smooth_camera,
            Switch::EnemyHealth => self.enemy_health,
            Switch::Controls => self.modern_controls,
            Switch::RoadKeys => self.road_keys,
            Switch::TurnView => self.turn_view,
            Switch::Lighting => self.lighting,
            Switch::Pixels => self.pixels != PIXELS_SHARP,
            Switch::Music => self.better_music,
            Switch::SpatialSound => self.spatial_sound,
            Switch::TextSize => self.text_size != 0,
            Switch::ShowArmour => self.show_armour,
            Switch::ShowHelm => self.show_helm,
            Switch::Minimap => self.minimap,
            Switch::BetterInventory => self.better_inventory,
            Switch::HoverNames => self.hover_names != 0,
            Switch::HoldWalk => self.hold_walk,
            Switch::Journal => self.journal,
            Switch::Practice => self.practice,
            Switch::AlbumMusic => self.album_music,
            Switch::BetterSound => self.better_sound,
            Switch::WideView => self.wide_view,
            Switch::Shadows => self.shadows,
            Switch::SleepUntil => self.sleep_until,
            Switch::NoticeFar => self.notice_far,
            Switch::TalkTruce => self.talk_truce,
            Switch::VictoryFanfare => self.victory_fanfare,
            Switch::AvatarOutline => self.avatar_outline != 0,
            Switch::AvatarMenu => self.avatar_menu,
            Switch::Guide => self.guide,
            Switch::PuzzleSpots => self.puzzle_spots,
            Switch::Storm => self.storm != 0,
            Switch::PentagramSnap => self.pentagram_snap,
            Switch::PentagramReach => self.pentagram_reach,
            Switch::AvoidFlames => self.avoid_flames,
            Switch::MonsterFade => self.monster_fade,
            Switch::MagicGlow => self.magic_glow,
            Switch::EvenWalk => self.even_walk,
        }
    }

    fn toggle(&mut self, switch: Switch) {
        match switch {
            Switch::SmoothCamera => self.smooth_camera = !self.smooth_camera,
            Switch::EnemyHealth => self.enemy_health = !self.enemy_health,
            // two kinds of controls: classic, and modern (which walks by click)
            Switch::Controls => {
                self.modern_controls = !self.modern_controls;
                self.click_walk = self.modern_controls;
            }
            Switch::RoadKeys => self.road_keys = !self.road_keys,
            Switch::TurnView => self.turn_view = !self.turn_view,
            Switch::Lighting => self.lighting = !self.lighting,
            Switch::Pixels => self.pixels = (self.pixels + 1) % 3,
            Switch::Music => self.better_music = !self.better_music,
            Switch::SpatialSound => self.spatial_sound = !self.spatial_sound,
            Switch::TextSize => self.text_size = (self.text_size + 1) % TEXT_SIZES.len(),
            Switch::ShowArmour => self.show_armour = !self.show_armour,
            Switch::ShowHelm => self.show_helm = !self.show_helm,
            Switch::Minimap => self.minimap = !self.minimap,
            Switch::BetterInventory => self.better_inventory = !self.better_inventory,
            Switch::HoverNames => self.hover_names = (self.hover_names + 1) % NAME_SIZES.len(),
            Switch::HoldWalk => self.hold_walk = !self.hold_walk,
            Switch::Journal => self.journal = !self.journal,
            Switch::Practice => self.practice = !self.practice,
            Switch::AlbumMusic => self.album_music = !self.album_music,
            Switch::BetterSound => self.better_sound = !self.better_sound,
            Switch::WideView => self.wide_view = !self.wide_view,
            Switch::Shadows => self.shadows = !self.shadows,
            Switch::SleepUntil => self.sleep_until = !self.sleep_until,
            Switch::NoticeFar => self.notice_far = !self.notice_far,
            Switch::TalkTruce => self.talk_truce = !self.talk_truce,
            Switch::VictoryFanfare => self.victory_fanfare = !self.victory_fanfare,
            Switch::AvatarOutline => self.avatar_outline = (self.avatar_outline + 1) % OUTLINE_SHARES.len(),
            Switch::AvatarMenu => self.avatar_menu = !self.avatar_menu,
            Switch::Guide => self.guide = !self.guide,
            Switch::PuzzleSpots => self.puzzle_spots = !self.puzzle_spots,
            Switch::Storm => self.storm = (self.storm + 1) % 3,
            Switch::PentagramSnap => self.pentagram_snap = !self.pentagram_snap,
            Switch::PentagramReach => self.pentagram_reach = !self.pentagram_reach,
            Switch::AvoidFlames => self.avoid_flames = !self.avoid_flames,
            Switch::MonsterFade => self.monster_fade = !self.monster_fade,
            Switch::MagicGlow => self.magic_glow = !self.magic_glow,
            Switch::EvenWalk => self.even_walk = !self.even_walk,
        }
    }

    /// A switch is used in the menu. The music has one switch for three
    /// settings: the album, the MIDI music with the SoundFont, the MIDI
    /// music as in the original.
    fn press(&mut self, switch: Switch) {
        if switch != Switch::Music {
            return self.toggle(switch);
        }
        (self.album_music, self.better_music) = match (self.album_music, self.better_music) {
            (true, _) => (false, true),
            (false, true) => (false, false),
            (false, false) => (true, true),
        };
    }

    /// Sets a switch by its key in the settings file (for tests). False if
    /// there is no such switch.
    pub fn set(&mut self, key: &str, on: bool) -> bool {
        if key == "avatar_outline" {
            self.avatar_outline = if on { OUTLINE_DEFAULT } else { 0 };
            return true;
        }
        let Some(switch) = Switch::ALL.into_iter().find(|sw| sw.key() == key) else { return false };
        if self.get(switch) != on {
            self.toggle(switch);
        }
        true
    }

}

pub struct Menu {
    /// The tab shown.
    pub tab: usize,
    /// Row of the tab the keyboard acts on (0: the tabs themselves).
    pub selected: usize,
    /// Slider whose knob follows the mouse.
    dragging: Option<usize>,
    mouse_was_down: bool,
    /// The page with the keys is shown instead of the options.
    pub(crate) help: bool,
    /// The page where the keys are chosen is shown instead.
    pub keys: Option<crate::bindings::KeysPage>,
}

pub fn settings_path() -> Option<PathBuf> {
    Some(crate::user_dir()?.join("settings.ini"))
}

/// Volumes (0..=100) for music, sound effects and speech, and the option
/// switches, from the settings file.
pub fn load_settings() -> ([u8; 3], Options) {
    let mut volumes = [100, 50, 50];
    let mut options = Options::default();
    let Some(text) = settings_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return (volumes, options) };
    for line in text.lines() {
        let Some((key, val)) = line.split_once('=') else { continue };
        let (key, val) = (key.trim(), val.trim());
        if let Some(slot) = ["music", "sound", "speech"].iter().position(|k| *k == key) {
            if let Ok(n) = val.parse::<u8>() {
                volumes[slot] = n.min(100);
            }
        } else if key == "click_walk" {
            options.click_walk = val == "1" || val == "on";
        } else if key == "zoom" {
            if let Ok(n) = val.parse::<usize>() {
                options.zoom = n.min(ZOOMS.len() - 1);
            }
        } else if key == "text_size" {
            if let Ok(n) = val.parse::<usize>() {
                options.text_size = n.min(TEXT_SIZES.len() - 1);
            }
        } else if key == "storm_colours" {
            if let Ok(n) = val.parse::<usize>() {
                options.storm = n.min(2);
            }
        } else if key == "smooth_pixels" {
            // (on/off from when it was a switch: on is xBR)
            options.pixels = match val {
                "crt" | "2" => PIXELS_CRT,
                "on" | "xbr" | "1" => PIXELS_XBR,
                _ => PIXELS_SHARP,
            };
        } else if key == "hover_names" {
            if let Ok(n) = val.parse::<usize>() {
                options.hover_names = n.min(NAME_SIZES.len() - 1);
            }
        } else if key == "avatar_outline" {
            // (a number of the steps; "on"/"off" from when it was a switch)
            options.avatar_outline = match val.parse::<usize>() {
                Ok(n) => n.min(OUTLINE_SHARES.len() - 1),
                Err(_) if val == "on" => OUTLINE_DEFAULT,
                Err(_) => 0,
            };
        } else if key == "seen_end" {
            options.seen_end = val == "1" || val == "yes";
        } else if key == "modern_controls" {
            options.modern_controls = val == "1" || val == "on";
        } else if let Some(switch) = Switch::ALL.into_iter().find(|sw| sw.key() == key) {
            if options.get(switch) != (val == "1" || val == "on") {
                options.toggle(switch);
            }
        }
    }
    // modern controls always walk by click (the setting without that is gone)
    options.click_walk = options.modern_controls;
    (volumes, options)
}

fn save_settings(v: &[u8; 3], options: &Options) {
    let Some(path) = settings_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut text = format!("music={}\nsound={}\nspeech={}\n", v[MUSIC], v[SOUND], v[SPEECH]);
    for switch in Switch::ALL.into_iter().filter(|sw| !sw.has_steps()) {
        text += &format!("{}={}\n", switch.key(), if options.get(switch) { "on" } else { "off" });
    }
    text += &format!("click_walk={}\n", if options.click_walk { "on" } else { "off" });
    text += &format!("zoom={}\n", options.zoom);
    text += &format!("text_size={}\n", options.text_size);
    text += &format!("hover_names={}\n", options.hover_names);
    text += &format!("storm_colours={}\n", options.storm);
    text += &format!("smooth_pixels={}\n", ["off", "xbr", "crt"][options.pixels.min(2)]);
    text += &format!("avatar_outline={}\n", options.avatar_outline);
    if options.seen_end {
        text += "seen_end=yes\n";
    }
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("settings: cannot write {}: {e}", path.display());
    }
}

/// A small gold stud, as knob of a slider or mark of a switch.
pub fn draw_stud(surf: &mut Surface, cx: i32, cy: i32, bright: bool, filled: bool) {
    let (face, shine) = if bright { (0xFFE070, 0xFFFFD0) } else { (0xD09020, 0xF0C050) };
    for dy in -5..=5i32 {
        for dx in -3..=3i32 {
            // a diamond with a dark rim
            let d = dx.abs() * 5 + dy.abs() * 3;
            let (px, py) = (cx + dx, cy + dy);
            if d > 17 || px < 0 || py < 0 || px >= surf.width || py >= surf.height {
                continue;
            }
            let inside = d <= 13;
            if inside && !filled {
                // an empty socket
                surf.pixels[(py * surf.width + px) as usize] = 0x1C1008;
                continue;
            }
            surf.pixels[(py * surf.width + px) as usize] =
                if !inside { if filled { 0x301800 } else { face } } else if dx < 0 && dy < 0 { shine } else { face };
        }
    }
}

/// Lights up a row faintly.
pub fn tint(surf: &mut Surface, x: i32, y: i32, w: i32, h: i32) {
    let add = |c: u32, a: u32| (c + a).min(255);
    for py in y.max(0)..(y + h).min(surf.height) {
        for px in x.max(0)..(x + w).min(surf.width) {
            let p = &mut surf.pixels[(py * surf.width + px) as usize];
            *p = add(*p >> 16 & 0xFF, 0x30) << 16 | add(*p >> 8 & 0xFF, 0x22) << 8 | add(*p & 0xFF, 0x0C);
        }
    }
}

/// A thin volume bar: gold up to the knob, a dark groove after it.
fn draw_bar(surf: &mut Surface, x0: i32, x1: i32, knob: i32, cy: i32) {
    for py in cy - 1..=cy + 1 {
        if py < 0 || py >= surf.height {
            continue;
        }
        for px in x0.max(0)..=x1.min(surf.width - 1) {
            let edge = py != cy;
            surf.pixels[(py * surf.width + px) as usize] = match (px <= knob, edge) {
                (true, false) => 0xE0A830,
                (true, true) => 0x805010,
                (false, false) => 0x3A2A18,
                (false, true) => 0x201408,
            };
        }
    }
}

impl Game {
    /// Writes the volumes and options to the settings file.
    pub fn save_settings(&self) {
        save_settings(&self.audio.volumes(), &self.options);
    }

    pub fn toggle_menu(&mut self) {
        if self.ui.menu.take().is_some() {
            save_settings(&self.audio.volumes(), &self.options);
        } else {
            self.ui.menu = Some(Menu { tab: 0, selected: 0, dragging: None, mouse_was_down: true, help: false, keys: None });
        }
    }

    /// Labels in the language of the game data: title, sliders, "on",
    /// "off". The game fonts use the DOS character set, so umlauts are
    /// given by their code.
    fn menu_labels(&self) -> [&'static str; 6] {
        self.tr([
            ["Optionen", "Musik", "Effekte", "Sprache", "an", "aus"],
            ["Options", "Musique", "Effets", "Voix", "oui", "non"],
            ["Options", "Music", "Sound", "Speech", "on", "off"],
        ])
    }


    fn tab_labels(&self) -> [&'static str; TABS] {
        self.tr([
            ["Ton", "Grafik", "Steuerung", "Anzeige"],
            ["Son", "Graphisme", "Commandes", "Affichage"],
            ["Sound", "Graphics", "Controls", "Display"],
        ])
    }

    fn action_labels(&self) -> [&'static str; ACTIONS] {
        self.tr([
            ["Tastenbelegung ...", "Tasten umbelegen ..."],
            ["Touches ...", "Changer les touches ..."],
            ["Keys ...", "Change keys ..."],
        ])
    }

    /// Carries out an action row: 0 = the page with the keys, 1 = the page
    /// where they are chosen.
    fn menu_action(&mut self, action: usize) {
        if action == 1 {
            self.open_keys_page();
        } else if let Some(m) = &mut self.ui.menu {
            m.help = true;
        }
    }

    /// Esc: from the page with the keys back to the options. False if
    /// there was nothing to go back from (then the menu closes).
    pub fn menu_back(&mut self) -> bool {
        if self.keys_back() {
            return true;
        }
        match &mut self.ui.menu {
            Some(m) if m.help => {
                m.help = false;
                true
            }
            _ => false,
        }
    }

    /// Enter on the selected row.
    pub fn menu_activate(&mut self) {
        if self.keys_page_open() {
            self.keys_activate();
            return;
        }
        if self.menu_back() {
            return;
        }
        match self.selected_row() {
            Some(Row::Switch(sw)) => self.options.press(sw),
            Some(Row::Action(a)) => self.menu_action(a),
            Some(Row::Tabs) => self.menu_tab(1),
            _ => {}
        }
    }

    fn selected_row(&self) -> Option<Row> {
        let m = self.ui.menu.as_ref()?;
        tab_rows(m.tab).get(m.selected).copied()
    }

    /// Another tab (`d` = -1 or 1, wrapping around).
    pub fn menu_tab(&mut self, d: i32) {
        let Some(m) = &mut self.ui.menu else { return };
        if m.help || m.keys.is_some() {
            return;
        }
        m.tab = (m.tab as i32 + d).rem_euclid(TABS as i32) as usize;
        m.selected = m.selected.min(tab_rows(m.tab).len() - 1);
    }

    fn logo_height(&mut self) -> i32 {
        self.data.gump_shapes.get(LOGO_SHAPE).and_then(|s| s.frames.first().map(|f| f.height)).unwrap_or(0)
    }

    /// Top left corner of a row of the shown tab, each group set apart
    /// from the next by a gap.
    fn row_pos(&mut self, row: usize, surf_w: i32, surf_h: i32) -> (i32, i32) {
        let (logo_h, head, gap, rh) = self.menu_layout(surf_h);
        let total = logo_h + head + tallest(gap, rh);
        let top = ((surf_h - total) / 2).max(2) + logo_h + head;
        let tab = self.ui.menu.as_ref().map_or(0, |m| m.tab);
        let rows = tab_rows(tab);
        let above = rows_height(&rows[..=row.min(rows.len() - 1)], gap, rh) - rh;
        ((surf_w - PANEL_W) / 2, top + above)
    }

    /// Height of the logo (0: left out), of the room for the title below
    /// it, of the gaps between the groups and of a row. When the view is
    /// small (zoomed in), the rows and gaps shrink and if need be the logo
    /// is left out, so that all rows fit.
    fn menu_layout(&mut self, surf_h: i32) -> (i32, i32, i32, i32) {
        let logo_h = self.logo_height();
        let fits = |head: i32, gap: i32, rh: i32| head + tallest(gap, rh) + 4 <= surf_h;
        if fits(logo_h + LOGO_GAP, GROUP_GAP, ROW) {
            (logo_h, LOGO_GAP, GROUP_GAP, ROW)
        } else if fits(logo_h + LOGO_GAP, 6, ROW - 1) {
            (logo_h, LOGO_GAP, 6, ROW - 1)
        } else {
            (0, 16, 6, ROW - 1)
        }
    }

    /// Height of a row of the menu.
    fn row_height(&mut self, surf_h: i32) -> i32 {
        self.menu_layout(surf_h).3
    }

    /// Left and right end of the volume bar in a row starting at `x0`.
    fn bar(x0: i32) -> (i32, i32) {
        (x0 + BAR_X0, x0 + PANEL_W - BAR_RIGHT)
    }

    /// Where the tabs are in their row: left edge and width of each
    /// (stud and label), spread evenly over the panel.
    fn tab_spans(&self, x0: i32) -> [(i32, i32); TABS] {
        let labels = self.tab_labels();
        let width = |t: &str| self.data.font.as_ref().map_or(40, |f| f.width(t)) + TAB_LABEL_X;
        let used: i32 = labels.iter().map(|t| width(t)).sum();
        let space = ((PANEL_W - used) / TABS as i32).max(0);
        let mut spans = [(0, 0); TABS];
        let mut x = x0 + space / 2;
        for (i, t) in labels.iter().enumerate() {
            spans[i] = (x, width(t));
            x += width(t) + space;
        }
        spans
    }

    fn set_from_x(&mut self, slider: usize, row: usize, x: i32, surf_w: i32, surf_h: i32) {
        let (x0, _) = self.row_pos(row, surf_w, surf_h);
        let (b0, b1) = Self::bar(x0);
        let v = ((x - b0) * 100 + (b1 - b0) / 2) / (b1 - b0);
        self.audio.set_volume(slider, v.clamp(0, 100) as u8);
    }

    /// Keyboard: `dy` moves the selection, `dx` changes the selected row
    /// (on the row of the tabs: another tab).
    pub fn menu_key(&mut self, dx: i32, dy: i32) {
        if self.keys_page_open() {
            self.keys_move(dx, dy);
            return;
        }
        let Some(m) = &mut self.ui.menu else { return };
        if m.help {
            return;
        }
        let n = tab_rows(m.tab).len() as i32;
        m.selected = (m.selected as i32 + dy).rem_euclid(n) as usize;
        if dx == 0 {
            return;
        }
        match self.selected_row() {
            Some(Row::Tabs) => self.menu_tab(dx),
            Some(Row::Slider(s)) => {
                let v = self.audio.volumes()[s] as i32 + dx * 5;
                self.audio.set_volume(s, v.clamp(0, 100) as u8);
                if s == SOUND {
                    self.audio.play_sfx(0x90, 0, false, 1.0); // a footstep, to hear the level
                }
            }
            Some(Row::Switch(sw)) => self.options.press(sw),
            _ => {}
        }
    }

    /// Mouse: a volume bar follows the pointer while the button is held;
    /// a click on a tab shows it, a click on a row flips the switch or
    /// carries out the action. On the page with the keys a click goes back.
    pub fn menu_mouse(&mut self, x: i32, y: i32, down: bool, surf_w: i32, surf_h: i32) {
        if self.keys_page_open() {
            self.keys_mouse(x, y, down, surf_w, surf_h);
            return;
        }
        let Some(m) = &mut self.ui.menu else { return };
        let (dragging, pressed) = (m.dragging, down && !m.mouse_was_down);
        m.mouse_was_down = down;
        if m.help {
            if pressed {
                m.help = false;
            }
            return;
        }
        if !down {
            m.dragging = None;
            if dragging == Some(SOUND) {
                self.audio.play_sfx(0x90, 0, false, 1.0);
            }
            return;
        }
        let rows = tab_rows(m.tab);
        let rh = self.row_height(surf_h);
        // (slider, row) whose knob follows the pointer
        let slider = match dragging {
            Some(s) => rows.iter().position(|&r| r == Row::Slider(s)).map(|row| (s, row)),
            None if pressed => {
                let mut hit = None;
                for (row, &r) in rows.iter().enumerate() {
                    let Row::Slider(s) = r else { continue };
                    let (x0, gy) = self.row_pos(row, surf_w, surf_h);
                    let (b0, b1) = Self::bar(x0);
                    if x >= b0 - 8 && x <= b1 + 8 && y >= gy && y < gy + rh {
                        hit = Some((s, row));
                    }
                }
                hit
            }
            None => None,
        };
        if let Some((s, row)) = slider {
            self.set_from_x(s, row, x, surf_w, surf_h);
            if let Some(m) = &mut self.ui.menu {
                m.dragging = Some(s);
                m.selected = row;
            }
            return;
        }
        if !pressed {
            return;
        }
        for (row, &r) in rows.iter().enumerate() {
            let (x0, gy) = self.row_pos(row, surf_w, surf_h);
            if !(x >= x0 && x < x0 + PANEL_W && y >= gy && y < gy + rh) {
                continue;
            }
            if let Some(m) = &mut self.ui.menu {
                m.selected = row;
            }
            match r {
                Row::Tabs => {
                    let spans = self.tab_spans(x0);
                    if let Some(t) = spans.iter().position(|&(sx, w)| x >= sx - 4 && x < sx + w + 4) {
                        if let Some(m) = &mut self.ui.menu {
                            m.tab = t;
                        }
                    }
                }
                Row::Switch(sw) => self.options.press(sw),
                Row::Action(a) => self.menu_action(a),
                Row::Slider(_) => {}
            }
            return;
        }
    }

    /// The text for the state of a switch.
    fn switch_state(&self, switch: Switch) -> &'static str {
        let labels = self.menu_labels();
        match switch {
            // the controls: as in the original, or modern (walking by click)
            Switch::Controls => match (self.options.modern_controls, self.language) {
                (false, 'G') => "klassisch",
                (false, 'F') => "classiques",
                (false, _) => "classic",
                (true, _) => "modern",
            },
            // the game fonts have no "%"
            // the music: album, SoundFont or as in the original
            Switch::Music => match (self.options.album_music, self.options.better_music, self.language) {
                (true, _, _) if self.audio.has_album() => "Album",
                (true, _, 'G') => "Album: keine Dateien",
                (true, _, 'F') => "album : pas de fichiers",
                (true, _, _) => "album: no files",
                (false, true, _) if self.audio.soundfont_path.is_some() => "SoundFont",
                (false, true, 'G') => "keine SoundFont",
                (false, true, 'F') => "pas de SoundFont",
                (false, true, _) => "no SoundFont",
                (false, false, _) => "Original",
            },
            Switch::HoverNames => match (self.options.hover_names, self.language) {
                (0, _) => labels[5],
                (1, 'G') => "klein",
                (1, 'F') => "petits",
                (1, _) => "small",
                (2, 'G') => "mittel",
                (2, 'F') => "moyens",
                (2, _) => "medium",
                (_, 'G') => "gro\u{e1}",
                (_, 'F') => "grands",
                (_, _) => "large",
            },
            // the game fonts have no "%": how much of him is hidden, in words
            Switch::AvatarOutline => match (self.options.avatar_outline, self.language) {
                (0, _) => labels[5],
                (1, 'G') => "ein wenig",
                (1, 'F') => "un peu",
                (1, _) => "a little",
                (2, 'G') => "halb",
                (2, 'F') => "\u{85} moiti\u{82}",
                (2, _) => "half",
                (3, 'G') => "fast ganz",
                (3, 'F') => "presque",
                (3, _) => "mostly",
                (_, 'G') => "ganz",
                (_, 'F') => "enti\u{8a}rement",
                (_, _) => "fully",
            },
            // the graphics filter: none, xBR or tube monitor
            Switch::Pixels => match self.options.pixels {
                PIXELS_XBR => "xBR",
                PIXELS_CRT => "CRT",
                _ => labels[5],
            },
            // the rain storm: as in the original, gentler, or none
            Switch::Storm => match (self.options.storm, self.language) {
                (0, _) => "Original",
                (1, 'G') => "sanft",
                (1, 'F') => "douces",
                (1, _) => "gentle",
                (_, _) => labels[5],
            },
            Switch::TextSize => match (self.options.text_size, self.language) {
                (0, _) => "Original",
                (1, 'F') => "normale",
                (1, _) => "normal",
                (2, 'G') => "gro\u{e1}",
                (2, 'F') => "grande",
                (2, _) => "large",
                (_, 'G') => "sehr gro\u{e1}",
                (_, 'F') => "tr\u{8a}s grande",
                (_, _) => "very large",
            },
            _ => labels[if self.options.get(switch) { 4 } else { 5 }],
        }
    }

    /// Logo and title over the menu; returns where the rows begin.
    fn draw_menu_head(&mut self, surf: &mut Surface, title: &str) -> i32 {
        let (w, h) = (surf.width, surf.height);
        let (_, first_y) = self.row_pos(0, w, h);
        let (logo_h, head, _, _) = self.menu_layout(h);
        if logo_h > 0 {
            if let Some(logo) = self.data.gump_shapes.get(LOGO_SHAPE) {
                if let Some(f) = logo.frames.first() {
                    surf.paint(f, (w - f.width) / 2 + f.xoff, first_y - head - f.height + f.yoff, false, false, &self.data.pal);
                }
            }
        }
        if let Some(font) = &self.data.font {
            font.draw(surf, title, (w - font.width(title)) / 2, first_y - head + 4, &self.data.pal);
        }
        first_y
    }

    pub fn draw_menu(&mut self, surf: &mut Surface) {
        let Some((tab, selected, help)) = self.ui.menu.as_ref().map(|m| (m.tab, m.selected, m.help)) else { return };
        // sound switches are heard at once, while the game stands still
        self.audio.set_soundfont(self.options.better_music);
        self.audio.set_album(self.options.album_music);
        self.audio.set_better(self.options.better_sound);
        self.audio.set_spatial(self.options.spatial_sound);
        // the world dims behind the menu
        surf.dim();
        if help {
            self.draw_menu_help(surf);
            return;
        }
        if self.keys_page_open() {
            self.draw_keys_page(surf);
            return;
        }
        let labels = self.menu_labels();
        let volumes = self.audio.volumes();
        let (w, h) = (surf.width, surf.height);
        self.draw_menu_head(surf, labels[0]);
        let actions = self.action_labels();
        let tab_labels = self.tab_labels();

        let rh = self.row_height(h);
        for (row, r) in tab_rows(tab).into_iter().enumerate() {
            let (x0, gy) = self.row_pos(row, w, h);
            if row == selected {
                // the row the keyboard is on is lit faintly
                tint(surf, x0, gy, PANEL_W, rh);
            }
            let cy = gy + rh / 2;
            let spans = if r == Row::Tabs { self.tab_spans(x0) } else { [(0, 0); TABS] };
            let state = if let Row::Switch(sw) = r { self.switch_state(sw) } else { "" };
            let Some(font) = &self.data.font else { continue };
            let ty = gy + (rh - font.height + 1) / 2;
            match r {
                Row::Tabs => {
                    // the tab shown: a filled stud and a gold line under it
                    for (t, &(sx, tw)) in spans.iter().enumerate() {
                        font.draw(surf, tab_labels[t], sx + TAB_LABEL_X, ty, &self.data.pal);
                        draw_stud(surf, sx + 4, cy, row == selected && t == tab, t == tab);
                        if t == tab {
                            draw_bar(surf, sx, sx + tw, sx + tw, gy + rh + 2);
                        }
                    }
                }
                Row::Slider(s) => {
                    font.draw(surf, labels[s + 1], x0 + LABEL_X, ty, &self.data.pal);
                    let (b0, b1) = Self::bar(x0);
                    let k = b0 + (b1 - b0) * volumes[s] as i32 / 100;
                    draw_bar(surf, b0, b1, k, cy);
                    let value = volumes[s].to_string();
                    font.draw(surf, &value, x0 + PANEL_W - 4 - font.width(&value), ty, &self.data.pal);
                    draw_stud(surf, k, cy, row == selected, true);
                }
                Row::Switch(sw) => {
                    font.draw(surf, self.tr(sw.names()), x0 + LABEL_X, ty, &self.data.pal);
                    font.draw(surf, state, x0 + PANEL_W - 4 - font.width(state), ty, &self.data.pal);
                    draw_stud(surf, x0 + 8, cy, row == selected, self.options.get(sw));
                }
                Row::Action(a) => {
                    font.draw(surf, actions[a], x0 + LABEL_X, ty, &self.data.pal);
                    draw_stud(surf, x0 + 8, cy, row == selected, row == selected);
                }
            }
        }
    }

    /// The page with the keys of the controls in use.
    fn draw_menu_help(&mut self, surf: &mut Surface) {
        let title = self.action_labels()[0].trim_end_matches(" ...");
        let help = self.control_help();
        let line = self.data.font.as_ref().map_or(10, |f| f.height + 2);
        let (logo_h, head, _, _) = self.menu_layout(surf.height);
        let first_y = if logo_h + head + help.len() as i32 * line + 8 <= surf.height {
            self.draw_menu_head(surf, title)
        } else {
            // not enough room for the logo: only the title, at the top
            let Some(font) = &self.data.font else { return };
            font.draw(surf, title, (surf.width - font.width(title)) / 2, 4, &self.data.pal);
            4 + line + 4
        };
        let Some(font) = &self.data.font else { return };
        // closer lines, and if need be two columns (a continued line stays
        // with the one before it)
        let avail = surf.height - first_y - 2;
        let n = help.len();
        let line = if n as i32 * line <= avail { line } else { font.height + 1 };
        let mut split = n;
        if n as i32 * line > avail {
            split = n.div_ceil(2);
            while split > 1 && help[split].0.is_empty() {
                split -= 1;
            }
        }
        let columns: Vec<&[(&str, &str)]> = if split < n { vec![&help[..split], &help[split..]] } else { vec![&help[..]] };
        let widths: Vec<(i32, i32)> = columns
            .iter()
            .map(|c| {
                let key_w = c.iter().map(|(k, _)| font.width(k)).max().unwrap_or(0) + 10;
                (key_w, key_w + c.iter().map(|(_, t)| font.width(t)).max().unwrap_or(0))
            })
            .collect();
        const COLUMN_GAP: i32 = 16;
        let total: i32 = widths.iter().map(|w| w.1).sum::<i32>() + COLUMN_GAP * (widths.len() as i32 - 1);
        let mut x = ((surf.width - total) / 2).max(2);
        for (c, &(key_w, w)) in columns.iter().zip(&widths) {
            for (i, (key, what)) in c.iter().enumerate() {
                let y = first_y + i as i32 * line;
                font.draw(surf, key, x, y, &self.data.pal);
                font.draw(surf, what, x + key_w, y, &self.data.pal);
            }
            x += w + COLUMN_GAP;
        }
    }
}
