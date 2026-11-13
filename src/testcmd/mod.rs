//! The test commands of `--do` (see the usage text): a list separated by
//! ";", each "name" or "name:argument". They are run without a window, for
//! checking the game from the outside. `@` in a command stands for the item
//! made last (give, pack, place, spawn).
//!
//! The commands are grouped by subject, one file each; every group says
//! whether a command is one of its own.

mod scripts;
mod world;
mod movement;
mod items;
mod combat;
mod view;
mod sound;
mod ui;
mod ticks;
mod bot;
mod explore;
mod botlog;
mod journey;

use crate::*;

/// What a group made of a command.
#[derive(PartialEq)]
pub(super) enum Flow {
    /// Not one of its commands.
    Unknown,
    Done,
    /// Done, and the commands after it are not run.
    Stop,
}

/// What the commands keep from one to the next.
#[derive(Default)]
pub(super) struct State {
    /// The item made last (`@`).
    pub last: u32,
    /// What the world looked like at "snap", for "diff".
    pub snapshot: Option<Snapshot>,
}

fn number(s: &str, cmd: &str) -> Result<i32, String> {
    s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))
}

fn point(s: &str, cmd: &str) -> Result<(i32, i32), String> {
    let (x, y) = s.split_once(',').ok_or(format!("'{cmd}' expects x,y"))?;
    Ok((number(x, cmd)?, number(y, cmd)?))
}

type Group = fn(&mut Game, &mut Surface, &mut ViewOptions, &mut State, &str, &str, &str) -> Result<Flow, String>;

/// In the order they are asked (letting time pass comes last: its names
/// are only a letter and a number).
const GROUPS: [Group; 12] = [bot::run, explore::run, journey::run, scripts::run, world::run, movement::run, items::run, combat::run, view::run, sound::run, ui::run, ticks::run];

/// Runs the test commands of `--do`.
pub fn run_commands(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, commands: &str) -> Result<(), String> {
    let mut st = State::default();
    for cmd in commands.split(';').map(str::trim).filter(|c| !c.is_empty()) {
        let with_last = cmd.replace('@', &st.last.to_string());
        let cmd = with_last.as_str();
        let (name, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        let mut flow = Flow::Unknown;
        for group in GROUPS {
            flow = group(game, surf, opts, &mut st, name, arg, cmd)?;
            if flow != Flow::Unknown {
                break;
            }
        }
        match flow {
            Flow::Unknown => return Err(format!("unknown command '{cmd}'")),
            Flow::Stop => return Ok(()),
            Flow::Done => {}
        }
    }
    Ok(())
}

/// Without a window (--screenshot): the commands are run, the view is
/// drawn and written to `path`, and what is going on is printed.
pub fn run_headless(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, path: &Path, commands: &str) -> Result<(), String> {
    // test images show the exact state, without the camera catching up
    game.options.smooth_camera = false;
    // PAGAN_XBR: the picture smoothed, as with the option on (which also
    // changes how some things are drawn)
    game.options.pixels = if std::env::var_os("PAGAN_XBR").is_some() { crate::menu::PIXELS_XBR } else { crate::menu::PIXELS_SHARP };
    run_commands(game, surf, opts, commands)?;
    if !commands.contains("gumpshot") && !commands.contains("shapeshot") {
        game.draw(surf, opts, false);
    }
    if std::env::var_os("PAGAN_XBR").is_some() {
        // the picture smoothed to twice its size, as the option shows it
        let mut big = Vec::new();
        let sharp: &[bool] = if game.view.sharp.len() == surf.pixels.len() { &game.view.sharp } else { &[] };
        println!("smoothing: {} pixels kept sharp (mask {} for {})", sharp.iter().filter(|&&b| b).count(), game.view.sharp.len(), surf.pixels.len());
        if std::env::var_os("PAGAN_SHOWSHARP").is_some() {
            let marked: Vec<usize> = sharp.iter().enumerate().filter(|(_, &b)| b).map(|(i, _)| i).collect();
            for i in marked {
                surf.pixels[i] = 0xFF00FF;
            }
        }
        xbr::scale2x_keeping(&surf.pixels, surf.width as usize, surf.height as usize, &mut big, sharp);
        let mut times = Vec::new();
        for _ in 0..30 {
            let t = Instant::now();
            xbr::scale2x_keeping(&surf.pixels, surf.width as usize, surf.height as usize, &mut big, sharp);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("smoothing took {:.1} ms (fastest {:.1}, slowest {:.1})", times[15], times[0], times[29]);
        let mut s2 = Surface::new(2 * surf.width, 2 * surf.height);
        s2.pixels = big;
        *surf = s2;
    }
    match std::env::var("PAGAN_ZOOM").ok().and_then(|z| z.parse::<i32>().ok()).filter(|z| *z > 1) {
        Some(z) => save_png(path, &zoomed(surf, z)).map_err(|e| e.to_string())?,
        None => save_png(path, surf).map_err(|e| e.to_string())?,
    }
    println!("map {} tick {}: avatar {}", game.cur_map, game.tick, game.describe(&game.avatar()));
    for b in &game.barks {
        println!("bark by {}: {}", b.item, b.text);
    }
    if let Some((_, choices)) = &game.ask {
        println!("answers: {choices:?}");
    }
    for g in &game.ui.gumps {
        println!("open container: item {} gump shape {}", g.owner, g.shape);
    }
    if let Some(r) = &game.ui.reader {
        println!("reader open ({})", match r.look {
            crate::gumps::Look::Panel => "panel",
            crate::gumps::Look::Book => "book",
            crate::gumps::Look::Scroll => "scroll",
            crate::gumps::Look::Sign { .. } => "sign",
        });
    }
    if game.ui.targeting.is_some() {
        println!("waiting for a target");
    }
    if let Some(m) = &game.ui.slot_menu {
        let (page, pages) = m.page();
        println!("savegame book open ({}) page {}/{} entry {}", match m.mode {
            slots::Mode::Save => "save",
            slots::Mode::Load => "load",
            slots::Mode::Death => "the Avatar is dead",
        }, page + 1, pages, m.selected);
    }
    if game.restart {
        println!("new game requested");
    }
    if game.ui.slider.is_some() {
        println!("waiting for an amount");
    }
    if let Some(o) = game.weapon_overlay() {
        println!("weapon in hand: shape {} frame {} at {},{}", o.0, o.1, o.2, o.3);
    }
    if !game.audio.log.is_empty() {
        println!("audio: {}", game.audio.log.join(", "));
    }
    Ok(())
}

/// The state of the world for the test commands snap and diff.
pub(super) struct Snapshot {
    items: HashMap<u16, (u32, u32, i32, i32, i32, u16, u16, u16)>,
    actors: HashMap<u16, (i32, bool, bool, bool, u16)>,
    mana: i32,
    globals: Vec<u8>,
    map: usize,
}

impl Snapshot {
    fn take(game: &mut Game) -> Snapshot {
        let items: HashMap<_, _> = game.map().items.iter().filter(|it| !it.gone)
            .map(|it| (it.id, (it.shape, it.frame, it.x, it.y, it.z, it.parent, it.quality, it.flags))).collect();
        let ids: Vec<u16> = game.map().items.iter().filter(|it| !it.gone && it.is_actor()).map(|it| it.id).collect();
        let actors = ids.into_iter().map(|id| {
            let s = game.state(id);
            (id, (s.hp, s.dead, s.immortal, s.in_combat, s.target))
        }).collect();
        Snapshot { items, actors, mana: game.mana, globals: game.scripts.usecode.globals().to_vec(), map: game.cur_map }
    }

    fn diff(&self, now: &Snapshot, usecode: &usecode::Usecode) -> Vec<String> {
        let mut out = Vec::new();
        if self.map != now.map {
            out.push(format!("map {} -> {}", self.map, now.map));
        }
        if self.mana != now.mana {
            out.push(format!("mana {} -> {}", self.mana, now.mana));
        }
        let mut ids: Vec<u16> = self.items.keys().chain(now.items.keys()).copied().collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            match (self.items.get(&id), now.items.get(&id)) {
                (Some(a), None) => out.push(format!("item {id} shape {} frame {} gone", a.0, a.1)),
                (None, Some(b)) => out.push(format!("item {id} new: shape {} frame {} at {},{},{} parent {} q {}", b.0, b.1, b.2, b.3, b.4, b.5, b.6)),
                (Some(a), Some(b)) if a != b => {
                    let mut d = Vec::new();
                    // frames alone change all the time (animations, walking)
                    if a.0 != b.0 { d.push(format!("shape/frame {}/{} -> {}/{}", a.0, a.1, b.0, b.1)); }
                    if (a.2, a.3, a.4) != (b.2, b.3, b.4) {
                        // walking actors move all the time; only big moves count
                        let far = (a.2 - b.2).abs() + (a.3 - b.3).abs() + (a.4 - b.4).abs() > 64;
                        if far || id > 255 { d.push(format!("at {},{},{} -> {},{},{}", a.2, a.3, a.4, b.2, b.3, b.4)); }
                    }
                    if a.5 != b.5 { d.push(format!("parent {} -> {}", a.5, b.5)); }
                    if a.6 != b.6 { d.push(format!("q {} -> {}", a.6, b.6)); }
                    if a.7 != b.7 { d.push(format!("flags {:X} -> {:X}", a.7, b.7)); }
                    if !d.is_empty() { out.push(format!("item {id} shape {}: {}", b.0, d.join(", "))); }
                }
                _ => {}
            }
        }
        let mut actors: Vec<_> = now.actors.iter().collect();
        actors.sort_by_key(|(id, _)| **id);
        for (id, b) in actors {
            if let Some(a) = self.actors.get(id) {
                if a != b {
                    out.push(format!("actor {id}: hp {} -> {}, dead {} -> {}, immortal {} -> {}, fighting {} -> {}, target {} -> {}", a.0, b.0, a.1, b.1, a.2, b.2, a.3, b.3, a.4, b.4));
                }
            }
        }
        out.extend(usecode.flag_changes(&self.globals, &now.globals));
        out
    }
}
