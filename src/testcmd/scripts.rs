//! Test commands for the scripts: using things, conversations, flags, processes, state snapshots.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 27] = [
    ("use", cmd_use),
    ("look", cmd_use),
    ("click", cmd_click),
    ("dclick", cmd_click),
    ("scriptcheck", cmd_scriptcheck),
    ("answer", cmd_answer),
    ("stasis", cmd_stasis),
    ("journal", cmd_journal),
    ("fade", cmd_fade),
    ("grantpeace", cmd_grantpeace),
    ("spells", cmd_spells),
    ("flag", cmd_flag),
    ("fx", cmd_fx),
    ("hours", cmd_hours),
    ("people", cmd_people),
    ("npclist", cmd_npclist),
    ("snap", cmd_snap),
    ("diff", cmd_diff),
    ("event", cmd_event),
    ("mana", cmd_mana),
    ("vitals", cmd_vitals),
    ("practice", cmd_practice),
    ("barklog", cmd_barklog),
    ("target", cmd_target),
    ("slider", cmd_slider),
    ("pathshown", cmd_pathshown),
    ("procs", cmd_procs),
];

fn cmd_use(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.interact(num(arg)? as u16, name == "use");
    Ok(Flow::Done)
}

fn cmd_click(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    match game.pick(x, y) {
        Some(AVATAR) if name == "dclick" && game.backpack_at(x, y) => {
            println!("{cmd}: the backpack on the Avatar's back");
            game.open_inventory();
        }
        Some(id) => {
            println!("{cmd}: {}", game.map().get(id).map(|it| game.describe(it)).unwrap_or_default());
            game.interact(id, name == "dclick");
        }
        None => println!("{cmd}: nothing there"),
    }
    Ok(Flow::Done)
}

/// scriptcheck:<n>[-<m>]|all  look at and use every scripted item of the maps
fn cmd_scriptcheck(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let maps: Vec<usize> = if arg == "all" || arg.is_empty() {
        (0..256).filter(|&n| game.map_exists(n)).collect()
    } else if let Some((a, b)) = arg.split_once('-') {
        (num(a)? as usize..=num(b)? as usize).filter(|&n| game.map_exists(n)).collect()
    } else {
        vec![num(arg)? as usize]
    };
    let mut seen = crate::hash::HashMap::default();
    let mut report = check::Report::default();
    let mut jump = |g: &mut Game, m: usize| jump_to_map(g, m, None);
    for n in &maps {
        game.check_scripts(*n, &mut seen, &mut report, &mut jump);
    }
    if arg == "all" || arg.is_empty() {
        game.check_unplaced(40, &mut seen, &mut report, &mut jump);
        game.check_debug_menu(40, &mut report, &mut jump);
    }
    println!("scriptcheck: {} runs, {} crashes, {} still running at the end", report.runs, report.crashes, report.stuck);
    Ok(Flow::Done)
}

fn cmd_answer(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    match &game.ask {
        Some((_, choices)) => println!("answers: {choices:?}"),
        None => println!("answer: no conversation open"),
    }
    game.answer((num(arg)? - 1).max(0) as usize);
    Ok(Flow::Done)
}

/// stasis[:<0|1>]  hold the world as a scene does (the Avatar cannot act); without: whether it is held
fn cmd_stasis(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    if arg.is_empty() {
        println!("stasis: {}", game.stasis);
    } else {
        game.stasis = num(arg)? != 0;
    }
    Ok(Flow::Done)
}

/// journal  opens the quest journal; journal:text prints its text
fn cmd_journal(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    if arg == "text" {
        println!("{}", game.journal_text());
    } else {
        game.open_journal();
    }
    Ok(Flow::Done)
}

/// fade  tells whether a fade (to black or a colour) lies over the picture
fn cmd_fade(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    println!("fade: {}", if game.screen.fade.is_some() { "on" } else { "none" });
    println!("quake: {}", game.screen.quake);
    Ok(Flow::Done)
}

/// grantpeace:<id>  the peace spell with <id> picked as its target; prints how many it hit
fn cmd_grantpeace(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    println!("grantpeace: {}", game.grant_peace_on(num(arg)? as u16));
    Ok(Flow::Done)
}

/// spells  opens the spell book; spells:text prints its text
fn cmd_spells(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    if arg == "text" {
        println!("{}", game.spell_notes_text());
    } else {
        game.open_spell_notes();
    }
    Ok(Flow::Done)
}

/// flag:<name>=<value>  sets a global flag by its name; flag:<name> tells its value
fn cmd_flag(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    match arg.split_once('=') {
        Some((key, val)) => game.set_flag(key, val.parse().map_err(|_| "flag: value must be a number")?)?,
        None => println!("flag {arg}: {}", game.flag(arg)),
    }
    Ok(Flow::Done)
}

/// fx:pal<n> | fx:quake<n> | fx:quake? (print it) | fx:invert | fx:fireball | fx:hide  switch effects on for a look
fn cmd_fx(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let av = game.avatar();
    if arg == "quake?" {
        println!("quake: {}", game.screen.quake);
    } else if let Some(n) = arg.strip_prefix("pal") {
        game.screen.pal_transform = num(n)? as u8;
        game.data.pal.set_transform(game.screen.pal_transform);
    } else if let Some(n) = arg.strip_prefix("quake") {
        game.screen.quake = num(n)?;
    } else if arg == "invert" {
        game.screen.inverted = !game.screen.inverted;
    } else if arg == "fireball" {
        game.launch_fireball((av.x + 400, av.y - 300, av.z));
    } else if arg == "airwalk" {
        game.airwalk = !game.airwalk;
    } else if arg == "hide" {
        game.with_item_pub(AVATAR, |it| it.flags ^= 0x0010);
    }
    Ok(Flow::Done)
}

/// hours:<n>  let n game hours pass at once (the schedules follow within 256 ticks)
fn cmd_hours(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.time_offset = game.time_offset.wrapping_add(num(arg)? as u32 * 900 * 30);
    println!("hours: now hour {} of the day", game.game_hours() % 24);
    Ok(Flow::Done)
}

/// people  every permanent character: map, position, dead
fn cmd_people(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let mut ids: Vec<u16> = game.people.npc_stats.keys().copied().collect();
    ids.sort();
    for id in ids {
        if let Some((m, it)) = game.actor_anywhere(id) {
            println!("person {id}: map {m} at {},{},{}{}", it.x, it.y, it.z, if game.state(id).dead { " dead" } else { "" });
        }
    }
    Ok(Flow::Done)
}

/// the permanent characters with their fight values
fn cmd_npclist(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let mut ids: Vec<u16> = game.people.npc_stats.keys().copied().collect();
    ids.sort();
    for id in ids {
        let (st, (al, fl)) = (game.people.npc_stats[&id], game.people.npc_combat.get(&id).copied().unwrap_or((0, 0)));
        let n = game.people.npcs.iter().find(|n| n.item.id == id);
        println!("npc {id}: shape {} map {} str {} dex {} int {} hp {} align {al:02X} flags {fl:06X}",
                 n.map_or(0, |n| n.item.shape), n.map_or(0, |n| n.map), st[0], st[1], st[2], st[3]);
    }
    Ok(Flow::Done)
}

/// snap  remember the world (items, actors, mana, flags); diff tells what changed since
fn cmd_snap(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    st.snapshot = Some(Snapshot::take(game));
    Ok(Flow::Done)
}

fn cmd_diff(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    match &st.snapshot {
        Some(before) => {
            let now = Snapshot::take(game);
            let lines = before.diff(&now, &game.scripts.usecode);
            println!("diff: {} changes", lines.len());
            for l in lines.iter().take(60) {
                println!("  {l}");
            }
        }
        None => println!("diff: no snap before"),
    }
    Ok(Flow::Done)
}

/// event:<id>,<name>  start the script an item has for an event (look, use,
///   hatch, schedule ...), as the game does when that happens
fn cmd_event(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let (id, event) = arg.split_once(',').ok_or(format!("'{cmd}' expects id,event"))?;
    let id = number(id, cmd)? as u16;
    let event = crate::check::event_number(event.trim()).ok_or(format!("no event '{event}'"))?;
    let pid = game.call_event(id, event, &crate::check::event_args(event));
    println!("event: process {pid}");
    Ok(Flow::Done)
}

/// mana:<n>  the Avatar's mana (for trying spells)
fn cmd_mana(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.mana = num(arg)?;
    Ok(Flow::Done)
}

/// vitals[:<hp>]  the Avatar's health, mana, hunger, attributes (and set the health)
fn cmd_vitals(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    if !arg.is_empty() {
        game.state_mut(AVATAR).hp = num(arg)?;
    }
    println!("vitals: {}", game.describe_vitals());
    Ok(Flow::Done)
}

/// practice:<str>,<dex>,<int>  set the practice towards the next points
fn cmd_practice(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<i32> = arg.split(',').map(num).collect::<Result<_, _>>()?;
    if v.len() != 3 {
        return Err(format!("'{cmd}' expects str,dex,int"));
    }
    game.set_practice([v[0], v[1], v[2]]);
    Ok(Flow::Done)
}

/// barklog:<ticks>[,<id>...]  run the game and list every speech as it appears;
///   with ids, scripts asking for a target get them in turn (the last one again and again)
fn cmd_barklog(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let mut parts = arg.split(',');
    let ticks = num(parts.next().unwrap_or(""))?;
    let picks: Vec<u16> = parts.map(|p| num(p).map(|n| n as u16)).collect::<Result<_, _>>()?;
    let mut next_pick = 0;
    let mut seen: Vec<u16> = Vec::new();
    for _ in 0..ticks {
        if let (Some(&id), Some(_)) = (picks.get(next_pick.min(picks.len().saturating_sub(1))), game.ui.targeting) {
            println!("tick {}: target {id} picked", game.tick);
            game.target_picked(id);
            next_pick += 1;
        }
        game.step(None);
        for b in &game.barks {
            if !seen.contains(&b.pid) {
                seen.push(b.pid);
                println!("tick {} item {}: {}", game.tick, b.item, b.text);
            }
        }
    }
    Ok(Flow::Done)
}

fn cmd_target(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    println!("target: {}", if game.ui.targeting.is_some() { "picked" } else { "nobody asked" });
    game.target_picked(num(arg)? as u16);
    Ok(Flow::Done)
}

fn cmd_slider(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    match &game.ui.slider {
        Some(_) => {
            game.slider_change(num(arg)?);
            println!("slider: {}", game.ui.slider.as_ref().map_or(0, |s| s.value));
            game.slider_confirm();
        }
        None => println!("slider: nobody asked"),
    }
    Ok(Flow::Done)
}

/// pathshown  the tiles of the invisible path shown on this map
fn cmd_pathshown(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let mut shown: Vec<(i32, i32, i32)> = game.revealed_path.get(&game.cur_map).into_iter().flatten()
        .filter_map(|(&id, _)| game.map().get(id).map(|t| (t.x, t.y, t.z)))
        .collect();
    shown.sort();
    println!("pathshown: {} {:?}", shown.len(), shown);
    Ok(Flow::Done)
}

fn cmd_procs(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    for p in &game.scripts.procs {
        println!("pid {} class {:04X} ip {:04X} item {} type {:X}{}", p.pid, p.class, p.ip, p.item, p.ptype,
                 if p.suspended { " (waiting)" } else { "" });
    }
    Ok(Flow::Done)
}
