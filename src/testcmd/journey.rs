//! Steps of the playthrough (tests/journey): each one names a goal, not a
//! number of ticks, and only does what a player can (follow the guide,
//! click on people and things, pick answers, click speech away). A step
//! that does not reach its goal in time stops the run and says where the
//! Avatar was and what was going on.

use super::*;

/// Ticks a step may take unless it says otherwise (`<goal>/<ticks>`).
const STEP_TICKS: u32 = 3000;
/// A walk across a map, with doors and teleporters on the way.
const WALK_TICKS: u32 = 20000;

/// The goal and the time allowed: "goal/ticks".
fn budget(arg: &str, default: u32) -> Result<(&str, u32), String> {
    match arg.rsplit_once('/') {
        Some((goal, t)) => Ok((goal, t.trim().parse().map_err(|_| format!("bad time in '{arg}'"))?)),
        None => Ok((arg, default)),
    }
}

/// A person by number or by name.
fn person(arg: &str) -> Result<u16, String> {
    if let Ok(id) = arg.trim().parse::<u16>() {
        return Ok(id);
    }
    crate::places::PEOPLE.iter().find(|p| p.1.eq_ignore_ascii_case(arg.trim())).map(|p| p.0).ok_or(format!("nobody called '{arg}'"))
}

/// Where things stand, for a failed step.
fn situation(game: &Game) -> String {
    let av = game.avatar();
    let mut s = format!("map {} at {},{},{}, tick {}", game.cur_map, av.x, av.y, av.z, game.tick);
    if let Some((_, choices)) = &game.ask {
        s += &format!(", answers {choices:?}");
    }
    if let Some(b) = game.barks.last() {
        s += &format!(", speech by {}: {}", b.item, b.text);
    }
    if game.states.get(&crate::game::AVATAR).is_some_and(|s| s.dead) {
        s += ", the Avatar is dead";
    } else if !game.player_in_control() {
        s += ", the Avatar is held";
        if game.stasis {
            // who holds him: the scripts at work (not the waiting ones)
            let procs: Vec<String> = game.scripts.procs.iter().filter(|p| !p.suspended && p.class != 0xFFFF).take(10)
                .map(|p| format!("{:04X}:{:04X} on {}", p.class, p.ip, p.item)).collect();
            s += &format!(" (stasis; scripts {})", procs.join(", "));
        }
        if let Some(m) = game.inputs.get(&crate::game::AVATAR) {
            s += &format!(" (moved, direction {})", m.dir);
        }
        // the scripts about him
        let procs: Vec<String> = game.scripts.procs.iter().filter(|p| p.item == crate::game::AVATAR).map(|p| format!("{:04X}:{:04X}", p.class, p.ip)).collect();
        if !procs.is_empty() {
            s += &format!(", scripts on him {}", procs.join(" "));
        }
    }
    s
}

/// One tick, clicking speech away now and then as a player does.
pub(super) fn tick(game: &mut Game, n: u32) {
    if n % 40 == 39 && !game.barks.is_empty() {
        game.skip_barks();
    }
    // the view follows as in a window (without one nothing is drawn, and
    // an open container seemed out of the picture and closed at once)
    game.place_view(game.camera_position(), game.view.size);
    game.step(None);
}

/// Is a condition met? `<a>|<b>` either, `free`, `ask`, `read` (a text open), `map=<n>`, `flag:<name>=<v>`,
/// `near=<person>,<distance>[,<x>,<y>]`, `far=...` (not near).
fn met(game: &Game, cond: &str) -> Result<bool, String> {
    if let Some((a, b)) = cond.split_once('|') {
        return Ok(met(game, a)? || met(game, b)?);
    }
    if let Some(n) = cond.strip_prefix("far=") {
        return Ok(!met(game, &format!("near={n}"))?);
    }
    Ok(if cond == "free" {
        game.player_in_control() && game.ask.is_none() && game.ui.reader.is_none()
    } else if cond == "ask" {
        game.ask.is_some()
    } else if cond == "read" {
        game.ui.reader.is_some()
    } else if let Some(m) = cond.strip_prefix("map=") {
        game.cur_map == m.trim().parse::<usize>().map_err(|_| format!("bad map in '{cond}'"))?
    } else if let Some(f) = cond.strip_prefix("flag:") {
        let (key, val) = f.split_once('=').ok_or(format!("'{cond}' expects flag:<name>=<value>"))?;
        game.flag(key) == val.trim().parse::<u32>().map_err(|_| format!("bad value in '{cond}'"))?
    } else if let Some(n) = cond.strip_prefix("near=") {
        // near=<person>,<distance>[,<x>,<y>]  (from the Avatar or from that place)
        let parts: Vec<&str> = n.split(',').collect();
        let bad = || format!("'{cond}' expects near=<person>,<distance>[,<x>,<y>]");
        let num = |s: &str| s.trim().parse::<i32>().map_err(|_| bad());
        let dist = num(parts.get(1).ok_or_else(bad)?)?;
        let (x, y) = match parts[..] {
            [_, _, x, y] => (num(x)?, num(y)?),
            [_, _] => (game.avatar().x, game.avatar().y),
            _ => return Err(bad()),
        };
        game.map().get(person(parts[0])?).is_some_and(|p| (p.x - x).abs() <= dist && (p.y - y).abs() <= dist)
    } else {
        return Err(format!("unknown condition '{cond}'"));
    })
}

/// Lets time pass until a condition is met.
fn wait_for(game: &mut Game, cond: &str, ticks: u32, cmd: &str) -> Result<u32, String> {
    for n in 0..ticks {
        if met(game, cond)? {
            return Ok(n);
        }
        if game.states.get(&crate::game::AVATAR).is_some_and(|s| s.dead) {
            return Err(format!("{cmd}: {}", situation(game)));
        }
        tick(game, n);
    }
    if met(game, cond)? {
        return Ok(ticks);
    }
    Err(format!("{cmd}: '{cond}' not reached in {ticks} ticks ({})", situation(game)))
}

/// How many creatures close to the Avatar (near enough to strike at) are fighting.
pub(super) fn enemies(game: &Game) -> usize {
    let av = game.avatar();
    game.map().items_near(av.x, av.y, 384).into_iter().map(|i| game.map().items[i])
        // (people and creatures that fight; monsters that are no persons,
        // such as changelings, only close by)
        .filter(|t| t.id != crate::game::AVATAR && !t.dead && game.states.get(&t.id).is_none_or(|s| !s.dead))
        .filter(|t| (t.x - av.x).abs() < 256 && (t.y - av.y).abs() < 256 && (t.z - av.z).abs() < 24)
        .filter(|t| if t.is_actor() {
            game.states.get(&t.id).is_some_and(|s| s.in_combat)
        } else {
            game.data.monsters.contains_key(&t.shape)
        })
        .count()
}

/// Fights as a player does (weapon drawn, F at the nearest enemy) until
/// nobody near fights any more, then puts the weapon away. Returns the ticks.
fn fight_off(game: &mut Game, ticks: u32, cmd: &str) -> Result<u32, String> {
    let mut quiet = 0;
    for n in 0..ticks {
        if game.states.get(&crate::game::AVATAR).is_some_and(|s| s.dead) {
            return Err(format!("{cmd}: the Avatar died, {}", situation(game)));
        }
        if enemies(game) == 0 {
            quiet += 1;
            if quiet > 60 {
                if game.fighting() {
                    game.toggle_combat();
                }
                return Ok(n);
            }
        } else {
            quiet = 0;
            if !game.fighting() && game.player_in_control() {
                game.toggle_combat();
            }
            if n % 10 == 0 {
                game.attack_nearest();
            }
        }
        tick(game, n);
    }
    Err(format!("{cmd}: still fighting after {ticks} ticks, {}", situation(game)))
}

/// A thing by its number, or one the Avatar carries by "shape:quality" or
/// "shape/frame" (carried things get other numbers on another map).
fn thing(game: &Game, spec: &str) -> Result<u16, String> {
    let spec = spec.trim();
    // <shape>/<frame>*  the nearest such thing in the world (one made by a
    // script, whose number changes)
    if let Some(what) = spec.strip_suffix('*') {
        let (shape, frame) = what.split_once('/').ok_or(format!("'{spec}' expects <shape>/<frame>*"))?;
        let shape: u32 = shape.parse().map_err(|_| format!("bad shape in '{spec}'"))?;
        let frame: u32 = frame.parse().map_err(|_| format!("bad frame in '{spec}'"))?;
        let av = game.avatar();
        return game.map().items.iter()
            .filter(|it| !it.gone && it.parent == 0 && it.shape == shape && it.frame == frame)
            .min_by_key(|it| (it.x - av.x).abs() + (it.y - av.y).abs() + (it.z - av.z).abs())
            .map(|it| it.id)
            .ok_or(format!("no {what} here"));
    }
    // <shape>/<frame>^<container>  a thing in a container (a body, a chest)
    if let Some((what, owner)) = spec.split_once('^') {
        let owner: u16 = owner.parse().map_err(|_| format!("bad container in '{spec}'"))?;
        let (shape, frame) = what.split_once('/').ok_or(format!("'{spec}' expects <shape>/<frame>^<container>"))?;
        let shape: u32 = shape.parse().map_err(|_| format!("bad shape in '{spec}'"))?;
        let frame: u32 = frame.parse().map_err(|_| format!("bad frame in '{spec}'"))?;
        return game.map().items.iter()
            .find(|it| !it.gone && it.shape == shape && it.frame == frame && it.parent == owner)
            .map(|it| it.id)
            .ok_or(format!("no {what} in {owner}"));
    }
    if let Some((shape, frame)) = spec.split_once('/') {
        let shape: u32 = shape.parse().map_err(|_| format!("bad shape in '{spec}'"))?;
        let frame: u32 = frame.parse().map_err(|_| format!("bad frame in '{spec}'"))?;
        return game.map().items.iter()
            .find(|it| !it.gone && it.shape == shape && it.frame == frame && game.root_container(it.id) == crate::game::AVATAR)
            .map(|it| it.id)
            .ok_or(format!("the Avatar carries no {spec}"));
    }
    if let Some((shape, q)) = spec.split_once(':') {
        let shape: u32 = shape.parse().map_err(|_| format!("bad shape in '{spec}'"))?;
        let q: u16 = q.parse().map_err(|_| format!("bad quality in '{spec}'"))?;
        return game.map().items.iter()
            .find(|it| !it.gone && it.shape == shape && it.quality == q && game.root_container(it.id) == crate::game::AVATAR)
            .map(|it| it.id)
            .ok_or(format!("the Avatar carries no {spec}"));
    }
    spec.parse().map_err(|_| format!("bad id '{spec}'"))
}

/// Follows the guide's way for a while as a player with click-walking
/// does: a click on the ground a little ahead on the way, and the game
/// finds the steps there. Returns the ticks spent.
fn follow(game: &mut Game, ahead: usize) -> Result<u32, String> {
    game.tick_guide();
    let g = &game.guide;
    let Some(&(x, y, _)) = g.route.get((g.progress + ahead).min(g.route.len().saturating_sub(1))) else {
        // no way yet (a new leg after a teleporter): the search runs in a
        // thread of its own; wait for it as the player would
        let start = std::time::Instant::now();
        while game.guide.status() == crate::guide::Status::Searching && start.elapsed().as_secs() < 60 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            game.tick_guide();
        }
        if game.guide.route.is_empty() {
            // standing right before the teleporter of this leg (a way of no
            // points): the last steps onto it with the direction keys
            let to = match game.guide.leg {
                Some(crate::guide::Leg::Teleporter(i)) => game.guide_links().get(i).map(|l| (l.x, l.y)),
                _ => None,
            };
            for _ in 0..12 {
                let a = game.avatar();
                let input = to.map(|(x, y)| crate::actor::MoveInput { dir: crate::intrinsics::world_dir(x - a.x, y - a.y) as u8, run: false });
                game.step(input);
                game.tick_guide();
            }
            return Ok(12);
        }
        return Ok(0);
    };
    let before = game.avatar();
    // PAGAN_JOURNEY=1: every click is printed
    if std::env::var_os("PAGAN_JOURNEY").is_some() {
        eprintln!("follow: map {} from {},{},{} to {x},{y} (way point {} of {})", game.cur_map, before.x, before.y, before.z, g.progress, g.route.len());
    }
    game.start_walk(x, y);
    let mut n = 0;
    let trace = std::env::var("PAGAN_JOURNEY").is_ok_and(|v| v == "2");
    while n < 300 {
        if trace {
            let a = game.avatar();
            eprintln!("  at {},{},{} walking {} control {} target {} input {:?} way {:?}", a.x, a.y, a.z, game.walking(), game.player_in_control(), game.guide.target.is_some(), game.inputs.get(&crate::game::AVATAR).map(|i| i.dir), game.paths.get(&crate::game::AVATAR).map(|w| &w.points));
        }
        game.step(None);
        game.tick_guide();
        n += 1;
        // (the way is planned in the first ticks)
        if (n > 10 && !game.walking()) || game.guide.target.is_none() || game.ask.is_some() || game.stasis {
            break;
        }
    }
    // not moved at all: a closed door in the way (the player clicks it)
    let a = game.avatar();
    if (a.x, a.y, a.z) == (before.x, before.y, before.z) {
        let map = game.map();
        let door = map.items_near(a.x, a.y, 256).into_iter().map(|i| map.items[i])
            .filter(|it| [68, 69, 340, 342].contains(&it.shape) && it.in_world())
            .min_by_key(|it| (it.x - a.x).abs() + (it.y - a.y).abs());
        let Some(door) = door else {
            // the next point too close for a click to move him (a
            // teleporter just ahead): a few steps with the direction keys
            let g = &game.guide;
            let next = g.route.get((g.progress + 1).min(g.route.len().saturating_sub(1))).copied();
            for _ in 0..12 {
                let a = game.avatar();
                let input = next.map(|(x, y, _)| crate::actor::MoveInput { dir: crate::intrinsics::world_dir(x - a.x, y - a.y) as u8, run: false });
                game.step(input);
                game.tick_guide();
            }
            return Ok(n + 12);
        };
        game.use_or_approach(door.id);
        for _ in 0..60 {
            game.step(None);
            n += 1;
        }
    }
    Ok(n)
}

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    // the guide's searches waited for: the same steps, the same run
    game.guide.sync = true;
    match name {
        "until" => {
            // until:<condition>[/<ticks>]  time passes (speech clicked away) until it holds
            let (cond, ticks) = budget(arg, STEP_TICKS)?;
            let n = wait_for(game, cond, ticks, cmd)?;
            println!("{cmd}: after {n} ticks");
        }
        "rush" => {
            // rush:...  as reach, at a run (the run switch on): away from
            // enemies too strong for him
            let was = game.run_toggle;
            game.run_toggle = true;
            let r = run(game, surf, opts, _st, "reach", arg, cmd);
            game.run_toggle = was;
            return r;
        }
        "reach" => {
            // reach:<person|place|x,y[,map]>[/<ticks>]  follow the guide there
            let (goal, ticks) = budget(arg, WALK_TICKS)?;
            wait_for(game, "free", STEP_TICKS, cmd)?;
            game.options.guide = true;
            super::run_commands(game, surf, opts, &format!("guideto:{goal};guidewait"))?;
            if game.guide.target.is_none() {
                return Err(format!("{cmd}: the guide knows no way ({})", situation(game)));
            }
            let mut spent = 0;
            let mut stuck = 0;
            while game.guide.target.is_some() {
                if spent >= ticks {
                    return Err(format!("{cmd}: not there after {ticks} ticks, guide {:?} ({})", game.guide.status(), situation(game)));
                }
                // someone stops him on the way and asks something: the
                // next steps answer, then "reach" again goes on
                if game.ask.is_some() {
                    let who = game.ask.as_ref().map_or(0, |a| a.0);
                    let name = crate::places::PEOPLE.iter().find(|p| p.0 == who).map_or(String::new(), |p| format!(" ({})", p.1));
                    println!("{cmd}: stopped by a question of {who}{name} after about {spent} ticks");
                    return Ok(Flow::Done);
                }
                // (attacked on the way he walks on: running past does better
                // than stopping to fight, and a ghost leaves him alone as
                // long as he does not fight)
                // a scene on the way: wait it out
                if !game.player_in_control() {
                    spent += wait_for(game, "free|ask", STEP_TICKS, cmd)?;
                    continue;
                }
                let before = game.avatar();
                spent += follow(game, 6)?;
                let a = game.avatar();
                let waiting = game.guide.route.is_empty();
                stuck = if (a.x, a.y, a.z) == (before.x, before.y, before.z) && !waiting { stuck + 1 } else { 0 };
                if stuck >= 16 {
                    return Err(format!("{cmd}: stuck on the way, {}", situation(game)));
                }
                if stuck >= 8 {
                    // someone in the way (a guard on a narrow wall): a player
                    // waits a moment until he has walked on
                    for n in 0..60 {
                        tick(game, n);
                    }
                    spent += 60;
                }
            }
            println!("{cmd}: there after about {spent} ticks");
        }
        "talk" => {
            // talk:<person>  click on someone (he walks up) until they speak
            let id = person(arg)?;
            wait_for(game, "free", STEP_TICKS, cmd)?;
            if game.map().get(id).is_none() {
                return Err(format!("{cmd}: {arg} is not on this map ({})", situation(game)));
            }
            game.use_or_approach(id);
            for n in 0..STEP_TICKS {
                if game.ask.is_some() || game.barks.iter().any(|b| b.item == id) {
                    println!("{cmd}: after {n} ticks");
                    return Ok(Flow::Done);
                }
                tick(game, n);
            }
            return Err(format!("{cmd}: no answer ({})", situation(game)));
        }
        "say" => {
            // say:<part of an answer>[|<another>...][/<ticks>]  waits for the question and
            // picks that answer (of several: the first one offered, for questions
            // asked in a random order)
            let (text, ticks) = budget(arg, STEP_TICKS)?;
            wait_for(game, "ask", ticks, cmd)?;
            let Some((_, choices)) = &game.ask else { unreachable!() };
            let pick = |want: &str| {
                let want = want.trim().to_lowercase();
                // (the whole answer first: "Yes." is not "Yes, I'm here to learn.")
                choices.iter().position(|c| c.trim().to_lowercase() == want)
                    .or_else(|| choices.iter().position(|c| c.to_lowercase().contains(&want)))
            };
            let Some(k) = text.split('|').find_map(pick) else {
                return Err(format!("{cmd}: no such answer among {choices:?}"));
            };
            println!("{cmd}: {}", choices[k].trim());
            game.answer(k);
        }
        "sayall" => {
            // sayall[:<answer to leave out>,...]  asks everything there is to ask, each
            // answer once (the last in the list, the farewell, at the end)
            // (buying is left out: it costs money the rest needs)
            let mut skip: Vec<String> = arg.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
            skip.extend(["buy", "pay", "kaufen", "zahle"].map(String::from));
            let mut asked: Vec<String> = Vec::new();
            for _ in 0..300 {
                // (a moment without a question: the talk is over)
                if wait_for(game, "ask|free", STEP_TICKS, cmd).is_err() || game.ask.is_none() {
                    let mut n = 0;
                    while game.ask.is_none() && n < 200 {
                        tick(game, n);
                        n += 1;
                    }
                    if game.ask.is_none() {
                        break;
                    }
                }
                let Some((_, choices)) = &game.ask else { break };
                let fresh = choices[..choices.len().saturating_sub(1)]
                    .iter()
                    .position(|c| !asked.contains(c) && !skip.iter().any(|s| c.to_lowercase().contains(s)));
                let k = fresh.unwrap_or(choices.len() - 1);
                println!("{cmd}: {}", choices[k].trim());
                asked.push(choices[k].clone());
                game.answer(k);
            }
            wait_for(game, "free", STEP_TICKS, cmd)?;
        }
        "operate" => {
            // operate:<id>  double-click a thing (he walks up to it if needed) and
            // wait until he is free again (shape:quality for a carried thing)
            let id = thing(game, arg)?;
            wait_for(game, "free", STEP_TICKS, cmd)?;
            let before = game.map().get(id).map(|it| it.frame);
            game.use_or_approach(id);
            for n in 0..STEP_TICKS {
                tick(game, n);
                if n > 10 && !game.walking() && game.approaching.is_none() && !game.mover(crate::game::AVATAR).busy() {
                    break;
                }
            }
            wait_for(game, "free|ask|read", STEP_TICKS, cmd)?;
            let after = game.map().get(id).map(|it| it.frame);
            println!("{cmd}: frame {before:?} -> {after:?}");
        }
        "fightoff" => {
            // fightoff[:/<ticks>]  weapon drawn, F (strike at the nearest enemy) until
            // nobody near fights him any more; then the weapon is put away
            let ticks = if arg.is_empty() { STEP_TICKS } else { budget(arg, STEP_TICKS)?.1 };
            let n = fight_off(game, ticks, cmd)?;
            println!("{cmd}: over after {n} ticks, hp {}", game.state(crate::game::AVATAR).hp);
        }
        "keys" => {
            // keys:<dir 0-7, 0 = north>,<ticks>  holds a direction key (walking)
            let (d, t) = arg.split_once(',').ok_or(format!("'{cmd}' expects dir,ticks"))?;
            let dir: u8 = d.trim().parse().map_err(|_| format!("bad direction in '{cmd}'"))?;
            let ticks: u32 = t.trim().parse().map_err(|_| format!("bad number in '{cmd}'"))?;
            let map = game.cur_map;
            for _ in 0..ticks {
                game.step(Some(crate::actor::MoveInput { dir: dir & 7, run: false }));
                if game.cur_map != map || game.pending_teleport.is_some() {
                    break;
                }
            }
            for n in 0..30 {
                tick(game, n);
            }
            let a = game.avatar();
            println!("{cmd}: map {} at {},{},{}", game.cur_map, a.x, a.y, a.z);
        }
        "walk" => {
            // walk:<x>,<y>[,<z>]  a click on the ground there (click-walking); waits
            // until he stands. With the height: exactly there, as after a
            // click on a small spot
            let v: Vec<i32> = arg.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let (x, y, z) = match v[..] {
                [x, y] => (x, y, None),
                [x, y, z] => (x, y, Some(z)),
                _ => return Err(format!("'{cmd}' expects x,y[,z]")),
            };
            // (attacked on the way: he fights, then walks on)
            let map = game.cur_map;
            for _ in 0..4 {
                wait_for(game, "free", STEP_TICKS, cmd)?;
                game.start_walk(x, y);
                if z.is_some() {
                    game.walk_level = z;
                    game.walk_exact = true;
                }
                let mut attacked = false;
                let hp = game.state(crate::game::AVATAR).hp;
                for n in 0..STEP_TICKS {
                    tick(game, n);
                    attacked = game.state(crate::game::AVATAR).hp < hp && enemies(game) > 0;
                    if (n > 10 && !game.walking()) || game.ask.is_some() || game.stasis || attacked {
                        break;
                    }
                }
                if !attacked {
                    break;
                }
                game.cancel_approach();
                fight_off(game, STEP_TICKS, cmd)?;
            }
            let a = game.avatar();
            // through a teleporter on the way: that is where the click led
            if game.cur_map != map {
                let a = game.avatar();
                println!("{cmd}: through to map {} at {},{},{}", game.cur_map, a.x, a.y, a.z);
                return Ok(Flow::Done);
            }
            // (a click on the ground: "there" begins some way before the spot)
            let near = if z.is_some() { 24 } else { 96 };
            if (a.x - x).abs() > near || (a.y - y).abs() > near || game.state(crate::game::AVATAR).dead {
                return Err(format!("{cmd}: not there, {}", situation(game)));
            }
            println!("{cmd}: at {},{},{}", a.x, a.y, a.z);
        }
        "hold" => {
            // hold:<x>,<y>,<z>,<ticks>  keeps to that spot for so long: whenever
            // something pushes him off it (the wind), a click there again
            let v: Vec<i32> = arg.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let [x, y, z, ticks] = v[..] else { return Err(format!("'{cmd}' expects x,y,z,ticks")) };
            let mut clicks = 0;
            for n in 0..ticks.max(0) as u32 {
                if game.states.get(&crate::game::AVATAR).is_some_and(|s| s.dead) {
                    return Err(format!("{cmd}: the Avatar died, {}", situation(game)));
                }
                let a = game.avatar();
                let off = (a.x - x).abs().max((a.y - y).abs()) > 12;
                if off && !game.walking() && game.player_in_control() && !game.mover(crate::game::AVATAR).busy() {
                    game.start_walk(x, y);
                    game.walk_level = Some(z);
                    game.walk_exact = true;
                    clicks += 1;
                }
                tick(game, n);
            }
            let a = game.avatar();
            println!("{cmd}: at {},{},{} after {clicks} clicks", a.x, a.y, a.z);
        }
        "useon" => {
            // useon:<item>,<target>  (number, or shape:quality of a carried thing)
            // double-click a thing (a key), then click the
            // target it asks for (a door, a chest)
            let (a, b) = arg.split_once(',').ok_or(format!("'{cmd}' expects item,target"))?;
            let item = thing(game, a)?;
            let target = thing(game, b)?;
            wait_for(game, "free", STEP_TICKS, cmd)?;
            let before = game.map().get(target).map(|it| (it.frame, it.quality));
            game.interact(item, true);
            for n in 0..60 {
                if game.ui.targeting.is_some() {
                    break;
                }
                tick(game, n);
            }
            if game.ui.targeting.is_none() {
                return Err(format!("{cmd}: it asks for no target, {}", situation(game)));
            }
            game.target_picked(target);
            for n in 0..120 {
                tick(game, n);
            }
            let after = game.map().get(target).map(|it| (it.frame, it.quality));
            println!("{cmd}: (frame, quality) {before:?} -> {after:?}");
        }
        "packin" => {
            // packin:<thing>,<container>[,<count>]  drags a carried thing into a
            // container; with a count only that many of a stack (the amount
            // slider)
            let parts: Vec<&str> = arg.split(',').collect();
            let (a, b) = match parts[..] {
                [a, b] | [a, b, _] => (a, b),
                _ => return Err(format!("'{cmd}' expects thing,container[,count]")),
            };
            let (mut id, bag) = (thing(game, a)?, thing(game, b)?);
            if let Some(n) = parts.get(2) {
                let n: u16 = n.trim().parse().map_err(|_| format!("bad count in '{cmd}'"))?;
                let it = game.map().get(id).copied().ok_or("gone")?;
                if it.quality > n {
                    let part = game.map_mut().add(crate::map::Item { id: 0, quality: n, parent: crate::map::ETHEREAL, fast: false, ..it });
                    if let Some(idx) = game.map().index(id) {
                        game.map_mut().items[idx].quality -= n;
                    }
                    id = part;
                }
            }
            if !game.put_in_container(id, bag, 20, 20) {
                return Err(format!("{cmd}: does not go in, {}", situation(game)));
            }
            println!("{cmd}: in {bag}");
        }
        "dropat" => {
            // dropat:<thing>:<x>,<y>  drags a thing (shape/frame for a carried
            // one) to a place in the view and lets it go there
            let (t, at) = arg.rsplit_once(':').ok_or(format!("'{cmd}' expects thing:x,y"))?;
            let id = thing(game, t)?;
            let (x, y) = point(at, cmd)?;
            game.draw(surf, opts, false);
            let ok = game.begin_drag(id, x, y) && game.drop_drag(x, y);
            game.ui.drag = None;
            if !ok {
                return Err(format!("{cmd}: could not, {}", situation(game)));
            }
            let it = game.map().get(id).copied();
            println!("{cmd}: at {:?}", it.map(|i| (i.x, i.y, i.z)));
        }
        "pickup" => {
            // pickup:<thing>  the right click that puts a thing into the backpack
            let id = thing(game, arg)?;
            wait_for(game, "free", STEP_TICKS, cmd)?;
            // out of reach on the ground: he walks up to it first
            if let Some(it) = game.map().get(id).copied().filter(|it| it.in_world() && !game.can_drag(id)) {
                game.start_walk(it.x, it.y);
                // (on its level: not down over an edge to below it)
                game.walk_level = Some(it.z);
                for n in 0..STEP_TICKS {
                    tick(game, n);
                    if game.can_drag(id) || (n > 10 && !game.walking()) {
                        break;
                    }
                }
                game.cancel_approach();
                for n in 0..20 {
                    tick(game, n);
                }
            }
            match game.take_into_backpack(id) {
                Some(true) => println!("{cmd}: in the backpack"),
                other => return Err(format!("{cmd}: not taken ({other:?}), {}", situation(game))),
            }
        }
        "puton" => {
            // puton:<thing>  drags a thing onto the Avatar: worn if it can be
            let id = thing(game, arg)?;
            if !game.drop_on_avatar(id) {
                return Err(format!("{cmd}: could not, {}", situation(game)));
            }
            let worn = game.map().get(id).is_some_and(|it| it.flags & crate::equip::FLG_EQUIPPED != 0);
            println!("{cmd}: {}", if worn { "worn" } else { "in the backpack" });
        }
        "dash" => {
            // dash:<x>,<y>  click-walking at a run, not stopping to fight (across a
            // narrow bridge, where a fight would push him off)
            let v: Vec<i32> = arg.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let [x, y] = v[..] else { return Err(format!("'{cmd}' expects x,y")) };
            wait_for(game, "free", STEP_TICKS, cmd)?;
            let was = game.run_toggle;
            game.run_toggle = true;
            game.start_walk(x, y);
            for n in 0..STEP_TICKS {
                tick(game, n);
                if (n > 10 && !game.walking()) || game.ask.is_some() || game.stasis || game.state(crate::game::AVATAR).dead {
                    break;
                }
            }
            game.run_toggle = was;
            let a = game.avatar();
            if (a.x - x).abs() > 96 || (a.y - y).abs() > 96 || game.state(crate::game::AVATAR).dead {
                return Err(format!("{cmd}: not there, {}", situation(game)));
            }
            println!("{cmd}: at {},{},{}", a.x, a.y, a.z);
        }
        "waithour" => {
            // waithour:<hour>  lets time pass (the player waits) until that hour of the day
            let hour: u32 = arg.trim().parse().map_err(|_| format!("bad number in '{cmd}'"))?;
            let mut n = 0u32;
            while game.game_hours() % 24 != hour {
                if n > 2_000_000 {
                    return Err(format!("{cmd}: the hour never came"));
                }
                if enemies(game) > 0 {
                    n += fight_off(game, STEP_TICKS, cmd)?;
                }
                tick(game, n);
                n += 1;
            }
            // (the people follow their schedules within a while)
            for k in 0..300 {
                tick(game, k);
            }
            println!("{cmd}: after {n} ticks");
        }
        "watch" => {
            // watch:<ticks>  waits there, fighting whoever comes to attack (before
            // a narrow way, a bridge, where a fight would push him off)
            let ticks: u32 = arg.trim().parse().map_err(|_| format!("bad number in '{cmd}'"))?;
            let mut n = 0;
            let mut fights = 0;
            while n < ticks {
                if enemies(game) > 0 {
                    n += fight_off(game, STEP_TICKS, cmd)?;
                    fights += 1;
                }
                tick(game, n);
                n += 1;
            }
            println!("{cmd}: {fights} fights, hp {}", game.state(crate::game::AVATAR).hp);
        }
        "rest" => {
            // rest:<hp>  waits (as a player lets time pass) until the Avatar has
            // this many hit points again; attacked meanwhile, he fights
            let want: i32 = arg.trim().parse().map_err(|_| format!("bad number in '{cmd}'"))?;
            let mut n = 0u32;
            while game.state(crate::game::AVATAR).hp < want {
                if n > 100_000 {
                    return Err(format!("{cmd}: still not healed, {}", situation(game)));
                }
                if enemies(game) > 0 {
                    n += fight_off(game, STEP_TICKS, cmd)?;
                }
                tick(game, n);
                n += 1;
            }
            println!("{cmd}: after {n} ticks");
        }
        "bypass" => {
            // bypass:<x>,<y>,<z>[,<map>][ <why>]  NOT a player's move: puts the Avatar
            // there where the playthrough does not get on yet (to be looked at)
            let (pos, why) = arg.split_once(' ').unwrap_or((arg, ""));
            let v: Vec<i32> = pos.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let (x, y, z, map) = match v[..] {
                [x, y, z] => (x, y, z, game.cur_map),
                [x, y, z, m] => (x, y, z, m as usize),
                _ => return Err(format!("'{cmd}' expects x,y,z[,map]")),
            };
            let a = game.avatar();
            println!("BYPASS from map {} {},{},{} to map {map} {x},{y},{z}: {why}", game.cur_map, a.x, a.y, a.z);
            game.teleport(crate::game::AVATAR, map, (x, y, z));
            for n in 0..10 {
                tick(game, n);
            }
        }
        "assume" => {
            // assume:<flag>=<value> <why>  NOT a player's move: sets a story flag
            // where the playthrough does not get the game to set it yet
            let (set, why) = arg.split_once(' ').unwrap_or((arg, ""));
            let (key, val) = set.split_once('=').ok_or(format!("'{cmd}' expects flag=value"))?;
            let val: u32 = val.trim().parse().map_err(|_| format!("bad value in '{cmd}'"))?;
            println!("BYPASS flag {key} {} -> {val}: {why}", game.flag(key));
            game.set_flag(key, val)?;
        }
        "climb" => {
            // climb:<x>,<y>,<z>  a click on a higher spot: he walks there and pulls himself up the ledges on the way
            let v: Vec<i32> = arg.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let [x, y, z] = v[..] else { return Err(format!("'{cmd}' expects x,y,z")) };
            wait_for(game, "free", STEP_TICKS, cmd)?;
            game.cancel_approach();
            game.start_walk(x, y);
            game.walk_level = Some(z);
            game.climb_goal = Some(crate::jump::ClimbGoal::new(x, y, z));
            for n in 0..STEP_TICKS {
                tick(game, n);
                if n > 10 && !game.walking() && game.climb_goal.is_none() && !game.mover(crate::game::AVATAR).busy() {
                    break;
                }
            }
            let a = game.avatar();
            if (a.z - z).abs() > 8 || game.state(crate::game::AVATAR).dead {
                return Err(format!("{cmd}: not up there, {}", situation(game)));
            }
            println!("{cmd}: at {},{},{}", a.x, a.y, a.z);
        }
        "hop" => {
            // hop:<x>,<y>,<z>  jumps onto that spot (the player points at it
            // and presses the jump key) and waits until he stands again;
            // in front of a ledge he pulls himself up it instead
            let v: Vec<i32> = arg.split(',').map(|s| s.trim().parse::<i32>().map_err(|_| format!("bad number in '{cmd}'"))).collect::<Result<_, _>>()?;
            let [x, y, z] = v[..] else { return Err(format!("'{cmd}' expects x,y,z")) };
            wait_for(game, "free", STEP_TICKS, cmd)?;
            game.draw(surf, opts, false);
            let p = game.to_screen(x, y, z);
            let (w, h) = game.view.size;
            if std::env::var_os("PAGAN_JOURNEY").is_some() {
                eprintln!("hop: pointer {p:?} ground {:?}", game.ground_at(p.0, p.1));
            }
            let said = game.jump(Some(p), w, h);
            // until the jump has been played and he stands (or falls no more)
            let mut jumped = false;
            for n in 0..400 {
                tick(game, n);
                let m = game.mover(crate::game::AVATAR);
                let busy = m.busy() || m.falling();
                jumped |= busy;
                if (jumped || n > 20) && !busy {
                    break;
                }
            }
            let a = game.avatar();
            // (pulling himself up a ledge ends at its edge, not at the spot)
            let near = said == "climbing" || ((a.x - x).abs() <= 48 && (a.y - y).abs() <= 48);
            if !near || (a.z - z).abs() > 8 || game.state(crate::game::AVATAR).dead {
                return Err(format!("{cmd}: missed ({said}), {}", situation(game)));
            }
            println!("{cmd}: at {},{},{}", a.x, a.y, a.z);
        }
        _ => return Ok(Flow::Unknown),
    }
    Ok(Flow::Done)
}
