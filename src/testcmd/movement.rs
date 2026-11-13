//! Test commands for moving the Avatar and others: walking, climbing, following, jumping.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 10] = [
    ("walkturns", cmd_walkturns),
    ("ledges", cmd_ledges),
    ("climbwalk", cmd_climbwalk),
    ("walkto", cmd_walkto),
    ("useat", cmd_useat),
    ("walkclick", cmd_walkclick),
    ("run", cmd_run),
    ("usenear", cmd_usenear),
    ("follow", cmd_follow),
    ("jump", cmd_jump),
];

/// walkturns:<x>,<y>  walk there as after a click on the ground; reports the turns
fn cmd_walkturns(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.start_walk(x, y);
    let (mut dirs, mut turns, mut last) = (Vec::new(), 0, None);
    for _ in 0..1500 {
        if !game.walking() {
            break;
        }
        game.step(None);
        if std::env::var_os("PAGAN_TRACE").is_some() && game.tick.is_multiple_of(5) {
            let av = game.avatar();
            println!("  tick {} at {},{},{} dir {:?} way {:?}", game.tick, av.x, av.y, av.z, game.inputs.get(&AVATAR).map(|i| i.dir), game.paths.get(&AVATAR).map(|w| &w.points));
        }
        if let Some(d) = game.inputs.get(&AVATAR).map(|i| i.dir) {
            if last.is_some() && last != Some(d) {
                turns += 1;
            }
            if last != Some(d) {
                dirs.push(d);
            }
            last = Some(d);
        }
    }
    let av = game.avatar();
    let stand = game.mover(AVATAR).dir;
    println!("{cmd}: at {},{} after {} ticks, {turns} turns {dirs:?}, standing dir {}", av.x, av.y, game.tick, stand);
    Ok(Flow::Done)
}

/// ledges[:<range>[,<lowest z>]]  places around the Avatar where he can climb up (x,y,z dir)
fn cmd_ledges(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (r, low) = arg.split_once(',').unwrap_or((arg, "0"));
    let range = if r.is_empty() { 1024 } else { num(r)? };
    let low = num(low)?;
    let av = game.avatar();
    let info = game.data.infos[av.shape as usize];
    let mut found = 0;
    'scan: for gy in (-range..=range).step_by(32) {
        for gx in (-range..=range).step_by(32) {
            for z in [0, 8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96].into_iter().filter(|&z| z >= low) {
                let it = map::Item { x: av.x + gx, y: av.y + gy, z, ..av };
                let b = map::WorldBox::of(&it, &info);
                let pi = game.map().position_info(&b, &b, &game.data.infos, AVATAR);
                if !pi.valid || !pi.supported {
                    continue;
                }
                for dir in climb_dirs(game, &it) {
                    println!("ledge: {},{},{} dir {dir}", it.x, it.y, it.z);
                    found += 1;
                    if found >= 8 {
                        break 'scan;
                    }
                }
            }
        }
    }
    Ok(Flow::Done)
}

/// climbwalk:<x>,<y>,<z>  walk there as after a click on higher ground (climbing automatically)
fn cmd_climbwalk(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<i32> = arg.split(',').map(&num).collect::<Result<_, _>>()?;
    let [x, y, z] = v[..] else { return Err(format!("'{cmd}' expects x,y,z")) };
    game.cancel_approach();
    game.start_walk(x, y);
    game.orders.climb_goal = Some(jump::ClimbGoal::new(x, y, z));
    let mut climbs = 0;
    for _ in 0..1500 {
        let was_busy = game.mover(AVATAR).busy();
        game.step(None);
        let a = game.mover(AVATAR).last_anim();
        if !was_busy && game.mover(AVATAR).busy() && (anim::CLIMB_FIRST..=anim::CLIMB_LAST).contains(&a) {
            climbs += 1;
        }
        if game.orders.climb_goal.is_none() && !game.walking() && !game.mover(AVATAR).busy() {
            break;
        }
    }
    let av = game.avatar();
    println!("{cmd}: at {},{},{} after {} ticks, {climbs} climbs", av.x, av.y, av.z, game.tick);
    Ok(Flow::Done)
}

/// walkto:<actor>:<x>,<y>  send an actor somewhere through the path finder
fn cmd_walkto(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let point = |s: &str| point(s, cmd);
    let (who, to) = arg.split_once(':').ok_or(format!("'{cmd}' expects actor:x,y"))?;
    let (x, y) = point(to)?;
    let pf = effects::Pathfind::new(num(who)? as u16, 0, (x, y));
    game.add_native(vm::Kind::Pathfind(pf), num(who)? as u16, crate::effects::PROC_WALK);
    Ok(Flow::Done)
}

/// useat:<id>  modern controls: a click on an item (walk there, use it)
fn cmd_useat(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.use_or_approach(num(arg)? as u16);
    println!("useat: {}", match game.orders.approaching {
        Some(id) => format!("walking to {id}"),
        None => "used at once".to_string(),
    });
    Ok(Flow::Done)
}

/// walkclick:<x>,<y>  modern controls: a click on the ground
fn cmd_walkclick(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    println!("walkclick: {}", if game.walk_to_pointer(x, y) { "walking" } else { "no ground there" });
    Ok(Flow::Done)
}

fn cmd_run(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.orders.run_toggle = !game.orders.run_toggle;
    Ok(Flow::Done)
}

fn cmd_usenear(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let used = game.use_nearest();
    println!("usenear: {}", used.and_then(|id| game.map().get(id).map(|it| game.describe(it))).unwrap_or("nothing".into()));
    Ok(Flow::Done)
}

/// follow:<x>,<y>,<ticks>  as if the button were held there (hold to follow)
fn cmd_follow(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let parts: Vec<i32> = arg.split(',').map(|v| v.trim().parse().map_err(|_| format!("bad number in {arg}"))).collect::<Result<_, _>>()?;
    let [x, y, ticks] = parts[..] else { return Err("follow:<x>,<y>,<ticks>".into()) };
    game.draw(surf, opts, false);
    let start = game.avatar();
    let mut seen = Vec::new();
    for _ in 0..ticks {
        let input = crate::play::follow_input(game, (x, y), surf.height);
        if seen.last() != Some(&input.map(|i| (i.dir, i.run))) {
            seen.push(input.map(|i| (i.dir, i.run)));
        }
        game.step(input);
        game.draw(surf, opts, false);
    }
    let av = game.avatar();
    println!("{cmd}: from {},{} to {},{}, inputs (dir, run) {:?}", start.x, start.y, av.x, av.y, seen);
    Ok(Flow::Done)
}

/// jump[:<x>,<y>]  jump to a view position (or straight ahead)
fn cmd_jump(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let at = if arg.is_empty() { None } else { Some(point(arg)?) };
    game.draw(surf, opts, false);
    println!("jump: {}", game.jump(at, surf.width, surf.height));
    Ok(Flow::Done)
}

/// The directions in which the Avatar, standing as `it`, could climb up.
fn climb_dirs(game: &mut Game, it: &map::Item) -> Vec<u8> {
    (0..8u8)
        .filter(|&dir| {
            let mut m = actor::Mover::new(dir);
            m.dir = dir;
            (anim::CLIMB_FIRST..=anim::CLIMB_LAST).any(|a| {
                let Game { data: crate::gamedata::GameData { infos, anims, .. }, maps, cur_map, .. } = &mut *game;
                m.would_succeed(a, it, &maps[cur_map], infos, anims)
            })
        })
        .collect()
}
