// Style lints that do not fit this code base (wide drawing and script
// functions, index loops over parallel tables, chunks_exact for older Rust).
#![allow(unknown_lints)]
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_range_loop,
    clippy::wrong_self_convention,
    clippy::chunks_exact_to_as_chunks,
    clippy::collapsible_match
)]

mod actor;
mod album;
mod anim;
mod audio;
mod barks;
mod bindings;
mod check;
mod combat;
mod cursor;
mod display;
mod controls;
mod crash;
mod drag;
mod effects;
mod eggs;
mod equip;
mod flex;
mod font;
mod game;
mod gamedata;
mod gpuscale;
mod gumps;
mod hidden;
mod puzzles;
mod pathreveal;
mod hover;
mod intrinsic_names;
mod intrinsics;
mod journal;
mod spellnotes;
mod quests;
mod jump;
mod lighting;
mod magic;
mod map;
mod mapfix;
mod menu;
mod movie;
mod palette;
mod passersby;
mod path;
mod play;
mod pacing;
mod pool;
mod potions;
mod save;
mod savegame;
mod shape;
mod sleep;
mod slots;
mod soundfix;
mod title;
mod shadows;
mod silhouette;
mod sorter;
mod testcmd;
mod platform;
mod ui;
mod usecode;
mod view;
mod vitals;
mod turnview;
mod inventory;
mod minimap;
mod guide;
mod hash;
mod worldmap;
mod places;
mod outfit;
mod upscale;
mod vm;
mod xbr;

use actor::MoveInput;
use game::{Game, AVATAR};
use hash::HashMap;
use platform::{Key, KeyRepeat, MouseButton, MouseMode, Window};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use view::{Surface, ViewOptions};

const USAGE: &str = "\
PaganRebuild - Ultima VIII engine (work in progress)

usage: PaganRebuild [options]
  --game <dir>        game folder (contains ENGLISH/, GERMAN/ ...); default: search upwards
  --lang <name>       language folder to use (default: ENGLISH)
  --map <n>           put the Avatar on this map
  --pos <x,y,z>       put the Avatar at this world position
  --size <WxH>        view size in pixels (default: 640x400)
  --scale <1|2|3|4>   window scale factor (default: 2)
  --no-sound          run without sound effects, speech and music
  --no-menu           go straight into a new game: no introduction, menu or name entry
  --screenshot <png>  render one frame to a PNG file and exit
  --do <commands>     with --screenshot: run test commands first, separated by ';'
                        t<n>           run n game ticks
                        w<dir>[r]:<n>  walk (run) in direction 0..7 for n ticks
                        use:<id>  look:<id>   use / look at the item with this id
                        click:<x>,<y>  dclick:<x>,<y>   mouse click at a view position
                        near[:<range>] list the items around the Avatar
                        find:<shape>   list items of a shape on the map (findf:<family>[,<quality>] by family)
                        item:<id>      describe an item and the containers holding it
                        chunkcheck[:<q>,..] walk away from each chunk-laying egg and back, list fast-area things lost
                        swapcheck      heights of every chunk-swapping egg's chunk and its pattern chunk
                        pickat:<x>,<y> draw the view and name the item painted at this pixel
                        grabat:<x>,<y> draw the view and name what a click at this pixel reaches
                        usable:<id>    can it be used, looked at, carried; its family and equip type
                        shapeshot:<shape>,..  the picture is all frames of these world shapes, on grey
                        outline        draw the view and count the pixels of the Avatar's outline
                        floorgaps      holes in the floor grid with floor on three or four sides
                        blackframes    shapes with frames that are black all over
                        hit:<id>,<dmg>,<type> a hit with this damage and damage type (hex, 8 = fire)
                        map:<n>        jump to another map (like PgUp/PgDn)
                        walkturns:<x>,<y>  walk there as after a click on the ground, count the turns
                        fight:<id>  fighthold:<id>  fighting by clicking: click / hold on an enemy
                        pointer:<x>,<y>[,<png>]  the mouse pointer there (and a picture with it, enlarged)
                        light:<0|1>    lighting off / on, for this run
                        musicwav:<track>,<seconds>,<file.wav>  a piece rendered with the SoundFont
                        albumfiles     the album's file for each piece it can replace
                        zoom:<percent> the view drawn at that size (200 = zoomed out); textsize:<0-3> text size option
                        turn:<n>       the view turned by n quarters (0 = original side); angle:<degrees>
                        screenpos:<id> where an item is in the view; turnlive:<1|-1> turn as the keys do
                        goto:<x>,<y>,<z>  move the Avatar
                        sendto:<id>,<map>,<x>,<y>,<z>  send a person to another place or map
                        reach:<person|place>[/<ticks>]  follow the guide there (as a player clicks along it); rush: the same at a run
                        talk:<person>  click on someone; say:<part of an answer>[|<another>..]  pick it; sayall  ask everything
                        until:<free|ask|read|map=<n>|flag:<name>=<v>|near=<person>,<dist>[,<x>,<y>]|far=..>[/<ticks>]  wait for it
                        hop:<x>,<y>,<z>  jump onto that spot (or pull himself up the ledge in front)
                        hold:<x>,<y>,<z>,<ticks>  keep to that spot (walk back whenever pushed off it)
                        climb:<x>,<y>,<z>  walk to a higher spot, climbing on the way; walk:<x>,<y>[,<z>]  a click on the ground
                        operate:<id>  double-click a thing; fightoff  strike with F until nobody near fights
                        bypass:<x>,<y>,<z>[,<map>] <why>  NOT a player's move: puts the Avatar on where the playthrough is stuck
                        assume:<flag>=<value> <why>  NOT a player's move: sets a story flag the playthrough does not get set yet
                        pickup:<thing>  into the backpack; puton:<thing>  onto the Avatar (thing: id, shape/frame carried, shape/frame^container, shape/frame* nearest); dash:<x>,<y>  click-walking at a run; packin:<thing>,<bag>[,<n>]  into a bag
                        rest:<hp>  wait until healed; watch:<ticks>  wait, fighting whoever attacks
                        useon:<thing>,<target>  a key on a door (shape:quality for a carried thing); waithour:<h>  wait until that hour
                        dropat:<thing>:<x>,<y>  drag a thing to a view position (shape/frame for a carried thing)
                        answer:<n>     pick answer n in a conversation
                        drag:<id>:<x>,<y>  drag an item to a view position
                        combat  attack[:<dir>]  hp   draw the weapon, strike, list the health of actors
                        skip           end the speech on screen
                        page  inv  inside:<id>   next reader page, open inventory, list contents
                        procs          list the running script processes
                        event:<id>,<name>  start an item's script for an event (look, use, hatch, schedule ...)
                        pathshown      the tiles of the invisible path shown on this map
                        bot:<seed>,<ticks>  the test bot plays on its own and prints rule breaches (lines BUG ...)
                        explore:<seed>,<ticks>  a bot that plays as a player new to the game: goes to what is new on the screen,
                                       asks people everything, loots, fights or flees, rests; checks the same rules

mouse: left click = look at, double click = use, drag = move an item (shift: part of a stack; onto the Avatar:
       into the backpack), right button = walk towards the pointer;
       in conversations click an answer or press its number;
       a click or space ends the speech on screen
keys:  arrows / WASD = walk (shift = run), I = backpack, C = close containers,
       space, J or both mouse buttons = jump to the pointer (standing at a ledge: climb),
       Tab = draw / put away the weapon; click or F = strike at the nearest enemy (held: keep striking; a click on a health bar attacks that creature), Q = kick, B = block, K = key ring,
       (fighting by clicking, with the modern controls: a click on an enemy walks up and
       strikes, holding the button keeps striking, ctrl+click strikes on the spot),
       V or F10 = options menu: volume, optional improvements, save and load
       (arrows or mouse, Enter; Esc closes), F5 = quick save, F9 = quick load,
       F6 / F7 = savegame book to save / load (it grows: there is always a free entry),
       space = next page when reading, PgUp/PgDn = jump to another map,
       + / - = zoom in / out (the view is drawn smaller or larger; remembered),
       , / . = turn the view by a quarter (if switched on in the options menu),
       R = roofs always on/off, L = lighting on/off, G = graphics filter off/xBR/CRT, E = editor objects on/off, F12 = screenshot,
       F4 held = the game runs six times as fast (to get through scenes and long walks),
       F3 = write the last 300 pictures drawn to the folder pagan_frames (to report display errors),
       modern controls (a switch in the options menu, which also lists the keys):
       left click = walk there (climbing ledges on the way) or use (walking there first), right click = take into the backpack (on open ground: jump), pointing = highlight, E = use the
       nearest thing, right click in combat = put the weapon away (double: kick), F11 = editor objects,
       Esc = main menu (introduction, read / write diary, options, credits, quit);
       a movie or the credits end with Esc, Enter, space or a click";

struct Args {
    game: Option<PathBuf>,
    lang: String,
    map: Option<usize>,
    pos: Option<(i32, i32, i32)>,
    size: (i32, i32),
    scale: u32,
    screenshot: Option<PathBuf>,
    commands: String,
    no_sound: bool,
    cheat: bool,
    no_menu: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        game: None,
        lang: "ENGLISH".into(),
        map: None,
        pos: None,
        size: (640, 400),
        scale: 2,
        screenshot: None,
        commands: String::new(),
        no_sound: false,
        cheat: false,
        no_menu: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or(format!("missing value for {arg}"));
        match arg.as_str() {
            "--game" => a.game = Some(val()?.into()),
            "--lang" => a.lang = val()?.to_uppercase(),
            "--map" => a.map = Some(val()?.parse().map_err(|_| "invalid map number")?),
            "--pos" => {
                let v: Vec<i32> = val()?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if v.len() != 3 {
                    return Err("--pos expects x,y,z".into());
                }
                a.pos = Some((v[0], v[1], v[2]));
            }
            "--size" => {
                let v: Vec<i32> = val()?.split('x').filter_map(|s| s.parse().ok()).collect();
                if v.len() != 2 || v[0] < 64 || v[1] < 64 {
                    return Err("--size expects WxH".into());
                }
                a.size = (v[0], v[1]);
            }
            "--scale" => a.scale = val()?.parse().ok().filter(|s| (1..=4).contains(s)).ok_or("invalid scale (1, 2, 3 or 4)")?,
            "--screenshot" => a.screenshot = Some(val()?.into()),
            "--do" => a.commands = val()?,
            "--no-sound" => a.no_sound = true,
            "--cheat" => a.cheat = true,
            "--no-menu" => a.no_menu = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

/// Looks for the game folder in the current directory and its parents,
/// then in the program's folder and its parents.
fn find_game(lang: &str) -> Option<PathBuf> {
    let starts = std::env::current_dir().ok().into_iter().chain(exe_dir());
    for mut dir in starts {
        loop {
            if dir.join(lang).join("STATIC").join("U8SHAPES.FLX").is_file() {
                return Some(dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

/// The player's own folder for savegames, settings, SoundFonts and
/// the album: %APPDATA%pagan on Windows, ~/Library/Application Support/pagan
/// on a Mac, $XDG_DATA_HOME/pagan (~/.local/share/pagan) elsewhere. APPDATA,
/// if set, wins everywhere (tests point it at a scratch folder).
pub fn user_dir() -> Option<PathBuf> {
    if let Some(app) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(app).join("pagan"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return Some(home?.join("Library").join("Application Support").join("pagan"));
    }
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("pagan")),
        _ => Some(home?.join(".local").join("share").join("pagan")),
    }
}

/// The folder the program lies in.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// Centre of the most densely built-up area of a map.
fn densest_spot(items: &[map::Item]) -> (i32, i32, i32) {
    let mut cells: HashMap<(i32, i32), (u32, i64)> = HashMap::default();
    for it in items.iter().filter(|it| it.in_world()) {
        let e = cells.entry((it.x >> 11, it.y >> 11)).or_default();
        e.0 += 1;
        e.1 += it.z as i64;
    }
    match cells.iter().max_by_key(|(k, v)| (v.0, **k)) {
        Some((&(cx, cy), &(n, zsum))) => ((cx << 11) + 1024, (cy << 11) + 1024, (zsum / n as i64) as i32),
        None => (16384, 16384, 0),
    }
}

fn jump_to_map(game: &mut Game, map: usize, pos: Option<(i32, i32, i32)>) {
    game.load_map(map);
    let pos = pos.unwrap_or_else(|| densest_spot(&game.maps[&map].items));
    game.pending_teleport = Some((map, eggs::Destination::Position(pos.0, pos.1, pos.2)));
    game.apply_teleport();
}

fn save_png(path: &Path, surf: &Surface) -> Result<(), Box<dyn Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, surf.width as u32, surf.height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    let rgb: Vec<u8> = surf.pixels.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect();
    w.write_image_data(&rgb)?;
    Ok(())
}

/// The middle of the view, enlarged: for checking details in test images.
fn zoomed(surf: &Surface, factor: i32) -> Surface {
    let mut out = Surface::new(surf.width, surf.height);
    let (w, h) = (surf.width / factor, surf.height / factor);
    let (x0, y0) = ((surf.width - w) / 2, (surf.height - h) / 2);
    for y in 0..surf.height {
        for x in 0..surf.width {
            let (sx, sy) = (x0 + x / factor, y0 + y / factor);
            out.pixels[(y * surf.width + x) as usize] = surf.pixels[(sy * surf.width + sx) as usize];
        }
    }
    out
}

/// Starts a game from the beginning, as the command line asks for it.
fn new_game(args: &Args) -> Result<Game, Box<dyn Error>> {
    let game_dir = match args.game.clone().or_else(|| find_game(&args.lang)) {
        Some(d) => d,
        None => return Err("game folder not found - pass it with --game <dir>".into()),
    };
    let lang_dir = game_dir.join(&args.lang);
    let mut game = Game::load(&lang_dir, (args.screenshot.is_none() || std::env::var_os("PAGAN_SOUND").is_some()) && !args.no_sound).map_err(|e| format!("{}: {e}", lang_dir.display()))?;

    if args.map.is_some() || args.pos.is_some() {
        let map = args.map.unwrap_or(game.cur_map);
        if !game.map_exists(map) {
            return Err(format!("map {map} has no data").into());
        }
        jump_to_map(&mut game, map, args.pos);
    } else {
        // a new game starts with the Avatar lying on the beach
        let avatar = game.avatar();
        let Game { data: crate::gamedata::GameData { anims, .. }, actors, .. } = &mut game;
        if let Some(mover) = actors.get_mut(&AVATAR) {
            mover.play(actor::intro_anim(), &avatar, anims);
        }
    }

    game.cheat = args.cheat;
    if args.cheat {
        println!("cheat mode: the Avatar starts equipped with");
        for line in game.cheat_equip() {
            println!("  {line}");
        }
    }

    Ok(game)
}

/// Runs the game: in its window, or without one (--screenshot) for the
/// test commands.
fn run(args: Args) -> Result<(), Box<dyn Error>> {
    let mut game = new_game(&args)?;
    // straight into the world for tests and for looking at a certain place
    let with_title = args.screenshot.is_none() && !args.no_menu && args.map.is_none() && args.pos.is_none();
    if with_title {
        game.start_title();
    }
    let mut opts = ViewOptions::default();
    let mut surf = Surface::new(args.size.0, args.size.1);
    game.view.size = args.size;
    if let Some(path) = &args.screenshot {
        return Ok(testcmd::run_headless(&mut game, &mut surf, &mut opts, path, &args.commands)?);
    }
    // in the window a crash leaves a log behind (test runs show it on the
    // console, and must not write to the player's folder)
    if let Some(dir) = user_dir() {
        crash::install(dir);
    }
    play::Play::new(args, game, surf, opts, with_title)?.run()
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}\n");
            }
            eprintln!("{USAGE}");
            std::process::exit(if msg.is_empty() { 0 } else { 2 });
        }
    };
    let result = run(args);
    // an autosave still being written must not be cut off
    savegame::wait_for_saves();
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
