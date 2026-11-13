//! Test commands for the title, movies, menus, books and savegames.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 36] = [
    ("title", cmd_title),
    ("titlekey", cmd_titlekey),
    ("wait", cmd_wait),
    ("movie", cmd_movie),
    ("movieholds", cmd_movieholds),
    ("readtest", cmd_readtest),
    ("menu", cmd_menu),
    ("option", cmd_option),
    ("menuok", cmd_menuok),
    ("menukey", cmd_menukey),
    ("menutab", cmd_menutab),
    ("menuhelp", cmd_menuhelp),
    ("menukeys", cmd_menukeys),
    ("saveslot", cmd_saveslot),
    ("loadslot", cmd_loadslot),
    ("book", cmd_book),
    ("booktype", cmd_booktype),
    ("bookclick", cmd_bookclick),
    ("notice", cmd_notice),
    ("save", cmd_save),
    ("load", cmd_load),
    ("skip", cmd_skip),
    ("page", cmd_page),
    ("worldmap", cmd_worldmap),
    ("png", cmd_png),
    ("underzoom", cmd_underzoom),
    ("wmclick", cmd_wmclick),
    ("wmzoom", cmd_wmzoom),
    ("wmrows", cmd_wmrows),
    ("guideto", cmd_guideto),
    ("guidelinks", cmd_guidelinks),
    ("guidewait", cmd_guidewait),
    ("guide", cmd_guide),
    ("guidecheck", cmd_guidecheck),
    ("guideroute", cmd_guideroute),
    ("guidewalk", cmd_guidewalk),
];

/// title:menu|name|intro|endgame|start|credits|creditsend   open a part of the title
/// (creditsend: the credits rolled on until their last lines are in the middle)
fn cmd_title(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match arg {
        "menu" => game.open_main_menu(),
        "name" => game.enter_name(),
        "start" => game.start_title(),
        "credits" => game.open_credits(),
        "creditsend" => {
            game.open_credits();
            game.roll_credits_to_end(surf.height);
        }
        "intro" | "endgame" => {
            let file = if arg == "intro" { "INTRO.SKF" } else { "ENDGAME.SKF" };
            println!("title: movie {}", if game.play_movie(file, title::Next::Menu) { "started" } else { "not found" });
        }
        _ => return Err(format!("unknown title part in '{cmd}'")),
    }
    Ok(Flow::Done)
}

/// titlekey:up|down|enter|esc|back|<text>   input for the title
fn cmd_titlekey(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let mut input = title::Input::default();
    match arg {
        "up" => input.up = true,
        "down" => input.down = true,
        "enter" => input.enter = true,
        "esc" => input.esc = true,
        "back" => input.backspace = true,
        text => input.chars = text.chars().collect(),
    }
    game.title_input(&input, surf.width, surf.height);
    println!("title: {}", game.describe_title());
    Ok(Flow::Done)
}

/// wait:<ms>  let real time pass (the credits roll by the clock)
fn cmd_wait(_game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    std::thread::sleep(Duration::from_millis(num(arg)? as u64));
    Ok(Flow::Done)
}

/// movie:<n>  let the movie on screen go on by n steps
fn cmd_movie(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    for _ in 0..num(arg)? {
        match game.movie_mut() {
            Some((movie, audio)) => movie.step(audio),
            None => break,
        }
    }
    game.title_tick();
    println!("title: {}", game.describe_title());
    Ok(Flow::Done)
}

/// movieholds:<file>  where a movie (STATIC\<file>) stands still for a sentence still spoken
fn cmd_movieholds(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let path = game.language_file(arg).ok_or(format!("no {arg}"))?;
    let movie = crate::movie::Movie::open(&path).ok_or(format!("cannot open {}", path.display()))?;
    for line in movie.holds() {
        println!("movieholds: {line}");
    }
    Ok(Flow::Done)
}

/// readtest:book|scroll  shows a long sample text in that reader
fn cmd_readtest(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let text = "Dies ist ein langer Probetext, um zu sehen, wie die Zeilen umbrechen und wie die Seiten sich f\u{81}llen. ".repeat(6);
    let look = if arg == "scroll" { gumps::Look::Scroll } else { gumps::Look::Book };
    game.read_text(AVATAR, &format!("Titel*{text}*Ende."), look);
    Ok(Flow::Done)
}

fn cmd_menu(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.toggle_menu();
    Ok(Flow::Done)
}

/// option:<key>,<on|off>  a switch of the options menu (key as in settings.ini), for this run
fn cmd_option(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let (key, val) = arg.split_once(',').ok_or("option:<key>,<on|off>")?;
    // (the outline has steps: option:avatar_outline,<0..4>)
    if let (true, Ok(n)) = (key == "avatar_outline", val.parse::<usize>()) {
        game.options.avatar_outline = n.min(menu::OUTLINE_SHARES.len() - 1);
        return Ok(Flow::Done);
    }
    // (the storm colours too: option:storm_colours,<0 original|1 gentle|2 none>)
    if let (true, Ok(n)) = (key == "storm_colours", val.parse::<usize>()) {
        game.options.storm = n.min(2);
        return Ok(Flow::Done);
    }
    // (no switches of the menu: the fade between maps, the passers-by)
    let on = val == "on" || val == "1";
    match key {
        "map_fade" => game.options.map_fade = on,
        "passersby" => game.options.passersby = on,
        _ if !game.options.set(key, on) => return Err(format!("no option '{key}'")),
        _ => {}
    }
    Ok(Flow::Done)
}

/// menuok  Enter in the options menu
fn cmd_menuok(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.menu_activate();
    Ok(Flow::Done)
}

fn cmd_menukey(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (dx, dy) = point(arg)?;
    game.menu_key(dx, dy);
    Ok(Flow::Done)
}

/// menutab:<n>  show tab n of the open options menu
fn cmd_menutab(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    if let Some(m) = &mut game.ui.menu {
        m.tab = num(arg)? as usize % 4;
    }
    game.draw(surf, opts, false);
    Ok(Flow::Done)
}

/// menuhelp  show the page with the keys of the open options menu
fn cmd_menuhelp(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    if let Some(m) = &mut game.ui.menu {
        m.help = true;
    }
    game.draw(surf, opts, false);
    Ok(Flow::Done)
}

/// menukeys[:<row>]  show the page where keys are chosen (open options menu), on a row
fn cmd_menukeys(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.open_keys_page();
    if let Some(k) = game.ui.menu.as_mut().and_then(|m| m.keys.as_mut()) {
        k.row = if arg.is_empty() { 0 } else { num(arg)? as usize };
    }
    game.draw(surf, opts, false);
    Ok(Flow::Done)
}

fn cmd_saveslot(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    println!("saveslot: {:?}", game.save_to(num(arg)? as usize).map(|_| "ok"));
    Ok(Flow::Done)
}

fn cmd_loadslot(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    println!("loadslot: {:?}", game.load_from(num(arg)? as usize));
    Ok(Flow::Done)
}

/// book:save | book:load | book:death  open the savegame book; book:<n> pick entry n;
/// book:next | book:prev  turn a page
fn cmd_book(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    match arg {
        "save" => game.open_slot_menu(slots::Mode::Save),
        "load" => game.open_slot_menu(slots::Mode::Load),
        "death" => game.open_slot_menu(slots::Mode::Death),
        "next" => game.slot_menu_turn(1),
        "prev" => game.slot_menu_turn(-1),
        n => {
            game.slot_menu_select_entry(num(n)? as usize);
            game.slot_menu_activate();
        }
    }
    Ok(Flow::Done)
}

/// booktype:<text>  type into the savegame name being written (a trailing < is a backspace)
fn cmd_booktype(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let (text, back) = match arg.strip_suffix('<') {
        Some(t) => (t, true),
        None => (arg, false),
    };
    game.slot_menu_type(&text.chars().collect::<Vec<_>>(), back);
    Ok(Flow::Done)
}

/// bookclick:<x>,<y>  a click into the savegame book at a view position
fn cmd_bookclick(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.slot_menu_click(x, y, surf.width, surf.height);
    println!("bookclick: book {}", if game.ui.slot_menu.is_some() { "still open" } else { "closed" });
    if let Some((text, _)) = &game.ui.notice {
        println!("notice: {text}");
    }
    Ok(Flow::Done)
}

/// notice  the short message at the top of the screen, if one is up
fn cmd_notice(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    println!("notice: {}", game.ui.notice.as_ref().map_or("none", |(t, _)| t.as_str()));
    Ok(Flow::Done)
}

fn cmd_save(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    println!("save: {:?}", game.save_game().map(|p| p.display().to_string()));
    Ok(Flow::Done)
}

fn cmd_load(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    println!("load: {:?}", game.load_game());
    Ok(Flow::Done)
}

fn cmd_skip(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    println!("skip: {}", if game.skip_barks() { "speech ended" } else { "nobody speaking" });
    Ok(Flow::Done)
}

fn cmd_page(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.reader_next();
    Ok(Flow::Done)
}

/// worldmap[:close]   open (close) the large map
fn cmd_worldmap(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    if arg == "close" {
        game.close_worldmap();
    } else {
        game.open_worldmap();
    }
    Ok(Flow::Done)
}

/// png:<file>   the view as it is now, into a picture (for a series of pictures along a scene)
fn cmd_png(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.draw(surf, opts, false);
    save_png(std::path::Path::new(arg), surf).map_err(|e| e.to_string())?;
    Ok(Flow::Done)
}

/// underzoom:<percent>   the world behind a menu or the large map drawn at this zoom and scaled (as the window does)
fn cmd_underzoom(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let pct = num(arg)?;
    let (w, h) = (surf.width * pct / 100, surf.height * pct / 100);
    game.view.under_menu = (pct != 100).then_some(((w, h), pct as f32 / 100.0));
    Ok(Flow::Done)
}

/// wmclick:<x>,<y>   a click on the large map (a row of the list or a point)
fn cmd_wmclick(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    let (w, h) = (surf.width, surf.height);
    game.worldmap_mouse(x, y, true, true, false, 0, w, h);
    game.worldmap_mouse(x, y, false, false, true, 0, w, h);
    Ok(Flow::Done)
}

/// wmzoom:<steps>   zoom the large map out (+) or in (-)
fn cmd_wmzoom(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.worldmap_key(0, 0, num(arg)?);
    Ok(Flow::Done)
}

/// wmrows   the rows of the list of places
fn cmd_wmrows(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    for (k, row) in game.worldmap_rows().iter().enumerate() {
        println!("wmrow {k}: {row:?}");
    }
    Ok(Flow::Done)
}

/// guideto:<x>,<y>[,<map>] | guideto:<part of a place's name>   where the guide leads
fn cmd_guideto(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let parts: Vec<&str> = arg.split(',').collect();
    if parts.len() >= 2 && parts[0].trim().parse::<i32>().is_ok() {
        let map = parts.get(2).map(|m| num(m)).transpose()?.map_or(game.cur_map, |m| m as usize);
        let (x, y) = (num(parts[0])?, num(parts[1])?);
        game.set_guide_target(Some(crate::guide::Target { map, x, y, name: "test".into(), person: 0 }));
    } else if let Some(&(id, who)) = crate::places::PEOPLE.iter().find(|p| p.1.eq_ignore_ascii_case(arg)) {
        let (map, x, y) = game.person_at(id).ok_or(format!("{who} is nowhere"))?;
        game.set_guide_target(Some(crate::guide::Target { map, x, y, name: who.into(), person: id }));
    } else {
        let k = crate::places::PLACES.iter().position(|p| p.names.iter().any(|n| n.to_lowercase().contains(&arg.to_lowercase())))
            .ok_or(format!("no place like '{arg}'"))?;
        let p = &crate::places::PLACES[k];
        let name = game.place_name(p);
        game.set_guide_target(Some(crate::guide::Target { map: p.map, x: p.x, y: p.y, name, person: 0 }));
    }
    Ok(Flow::Done)
}

/// guidelinks[:<map>]   the teleporters of a map (default: this one) and where they lead
fn cmd_guidelinks(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let m = if arg.is_empty() { game.cur_map } else { num(arg)? as usize };
    let links: Vec<crate::guide::Link> = game.guide_links().to_vec();
    for (i, l) in links.iter().enumerate().filter(|(_, l)| l.map == m) {
        println!("guidelink {i}: map {} at {},{},{} range {}x{} to map {} at {},{},{}", l.map, l.x, l.y, l.z, l.xr, l.yr, l.to_map, l.to.0, l.to.1, l.to.2);
    }
    Ok(Flow::Done)
}

/// guidewait   waits (up to a minute) until the guide's search has ended
fn cmd_guidewait(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let start = std::time::Instant::now();
    game.tick_guide();
    while game.options.guide && game.guide.status() == crate::guide::Status::Searching && start.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        game.tick_guide();
    }
    println!("guidewait: {:.2} s", start.elapsed().as_secs_f32());
    Ok(Flow::Done)
}

/// guide   the guide's state: place, leg, way
fn cmd_guide(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let g = &game.guide;
    let leg = g.leg.map(|l| match l {
        crate::guide::Leg::Place => "place".to_string(),
        crate::guide::Leg::Teleporter(i) => {
            let l = g.link(i).unwrap_or(crate::guide::Link { map: 0, x: 0, y: 0, z: 0, xr: 0, yr: 0, to_map: 0, to: (0, 0, 0) });
            format!("teleporter {i} at {},{},{} to map {}", l.x, l.y, l.z, l.to_map)
        }
    });
    let av = game.avatar();
    println!("guide: avatar map {} at {},{},{} target {:?} status {:?} leg {} way {} points, at {}{}", game.cur_map, av.x, av.y, av.z, g.target.as_ref().map(|t| (t.map, t.x, t.y)), g.status(),
             leg.unwrap_or("-".into()), g.route.len(), g.progress,
             g.route.last().map_or(String::new(), |e| format!(", ends at {},{},{}", e.0, e.1, e.2)));
    Ok(Flow::Done)
}

/// guidecheck   walks the Avatar's box along the guide's way (every 8 units)
///   and prints the first place where it does not fit (inside a wall)
fn cmd_guidecheck(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let av = game.avatar();
    let info = game.data.infos[av.shape as usize];
    let here = map::WorldBox::of(&av, &info);
    let route = game.guide.route.clone();
    let mut prev = (av.x, av.y, av.z);
    let mut bad = None;
    'outer: for &(x, y, z) in &route {
        let steps = ((x - prev.0).abs().max((y - prev.1).abs()) / 8).max(1);
        for s in 1..=steps {
            let (px, py) = (prev.0 + (x - prev.0) * s / steps, prev.1 + (y - prev.1) * s / steps);
            // (going up or down a step: the higher of the two heights)
            let pz = prev.2.max(z);
            let b = map::WorldBox { x: px, y: py, z: pz, ..here };
            let pi = game.map().position_info(&b, &here, &game.data.infos, crate::game::AVATAR);
            if !pi.valid {
                let what = pi.blocker.and_then(|o| game.map().get(o).copied()).map_or("?".into(), |o| format!("item {} shape {} at {},{},{}", o.id, o.shape, o.x, o.y, o.z));
                bad = Some(format!("{px},{py},{pz} between {},{},{} and {x},{y},{z}: {what}", prev.0, prev.1, prev.2));
                break 'outer;
            }
        }
        prev = (x, y, z);
    }
    println!("guidecheck: {} points, {}", route.len(), bad.unwrap_or("fits all the way".into()));
    Ok(Flow::Done)
}

/// guideroute   the next points of the guide's way
fn cmd_guideroute(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let g = &game.guide;
    // guideroute:all  the whole way, only where the height changes;
    // guideroute:every  every point
    let next: Vec<String> = if arg == "every" {
        g.route.iter().map(|p| format!("{},{},{}", p.0, p.1, p.2)).collect()
    } else if arg == "all" {
        let mut last = i32::MIN;
        g.route.iter().filter(|p| std::mem::replace(&mut last, p.2) != p.2).map(|p| format!("{},{},{}", p.0, p.1, p.2)).collect()
    } else {
        g.route.iter().skip(g.progress).take(12).map(|p| format!("{},{},{}", p.0, p.1, p.2)).collect()
    };
    println!("guideroute: at {} of {}: {}", g.progress, g.route.len(), next.join(" "));
    Ok(Flow::Done)
}

/// guidewalk:<ticks>[,careful]   the Avatar walks along the guide's way (as
/// if the player followed it); standing still in front of a door, he
/// opens it. Careful: point by point, not cutting corners (stepping
/// stones in a lake)
fn cmd_guidewalk(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (t, careful) = match arg.split_once(',') {
        Some((t, c)) => (t, c.trim() == "careful"),
        None => (arg, false),
    };
    let ticks = num(t)?;
    let mut still = 0;
    let mut last = (0, 0, 0);
    for _ in 0..ticks {
        game.tick_guide();
        let a = game.avatar();
        still = if (a.x, a.y, a.z) == last { still + 1 } else { 0 };
        last = (a.x, a.y, a.z);
        if still == 25 {
            let map = game.map();
            let door = map.items_near(a.x, a.y, 256).into_iter().map(|i| map.items[i])
                .filter(|it| [68, 69, 340, 342].contains(&it.shape) && it.in_world())
                .min_by_key(|it| (it.x - a.x).abs() + (it.y - a.y).abs());
            if let Some(d) = door {
                println!("guidewalk: opening door {} at {},{}", d.id, d.x, d.y);
                game.interact(d.id, true);
                // (he walks up to it by himself)
                for _ in 0..60 {
                    game.step(None);
                }
            }
        }
        let g = &game.guide;
        // (stuck at a corner: the very next point)
        let ahead = if still > 4 || careful { 1 } else { 3 };
        let next = g.route.get((g.progress + ahead).min(g.route.len().saturating_sub(1))).copied();
        let av = game.avatar();
        let input = next.map(|(x, y, _)| {
            crate::actor::MoveInput { dir: crate::intrinsics::world_dir(x - av.x, y - av.y) as u8, run: false }
        });
        game.step(input);
        if game.guide.target.is_none() {
            println!("guidewalk: arrived after {ticks} ticks");
            break;
        }
    }
    Ok(Flow::Done)
}
