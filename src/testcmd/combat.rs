//! Test commands for fights: creatures, blows, health.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 16] = [
    ("spawn", cmd_spawn),
    ("fightstate", cmd_fightstate),
    ("slay", cmd_slay),
    ("worldhit", cmd_worldhit),
    ("hit", cmd_hit),
    ("combat", cmd_combat),
    ("attack", cmd_attack),
    ("strike", cmd_strike),
    ("face", cmd_face),
    ("fight", cmd_fight),
    ("fightclick", cmd_fightclick),
    ("fighthold", cmd_fighthold),
    ("hp", cmd_hp),
    ("bars", cmd_bars),
    ("hidden", cmd_hidden),
    ("attacknear", cmd_attacknear),
];

/// spawn:<shape>[,<distance>]  hatch a creature next to the Avatar (160 units east, or as far as given)
fn cmd_spawn(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let av = game.avatar();
    let (shape, dist) = match arg.split_once(',') {
        Some((s, d)) => (num(s)?, num(d)?),
        None => (num(arg)?, 160),
    };
    let egg = map::Item { x: av.x + dist, y: av.y, z: av.z, quality: shape as u16, ..map::Item::default() };
    let actor = game.hatch_monster(&egg);
    println!("spawn: actor {actor}");
    st.last = actor as u32;
    Ok(Flow::Done)
}

/// fightstate:<id>  side, enemies, target and fighting of an actor
fn cmd_fightstate(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let id = num(arg)? as u16;
    let s = game.state(id);
    println!("{cmd}: alignment {:#X} enemies {:#X} target {} in_combat {} hp {} dex {}", s.alignment, s.enemy_alignment, s.target, s.in_combat, s.hp, s.dex);
    Ok(Flow::Done)
}

fn cmd_slay(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.die(num(arg)? as u16, 0);
    Ok(Flow::Done)
}

/// worldhit:<id>,<damage>  a hit without attacker, as water and lava deal it
fn cmd_worldhit(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (id, damage) = point(arg)?;
    game.receive_hit(id as u16, 0, 0, damage, 0x80 | 0x20 | 0x08);
    Ok(Flow::Done)
}

/// hit:<id>,<damage>,<type>  a hit by the Avatar with this damage and damage type (hex)
fn cmd_hit(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<&str> = arg.split(',').map(str::trim).collect();
    if v.len() != 3 {
        return Err(format!("'{cmd}' expects id,damage,type"));
    }
    let ty = u16::from_str_radix(v[2].trim_start_matches("0x"), 16).map_err(|_| format!("bad type in '{cmd}'"))?;
    game.receive_hit(num(v[0])? as u16, 0, 0, num(v[1])?, ty);
    Ok(Flow::Done)
}

fn cmd_combat(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.toggle_combat();
    Ok(Flow::Done)
}

fn cmd_attack(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.avatar_attack(if arg.is_empty() { None } else { Some(num(arg)? as u8) }, false);
    Ok(Flow::Done)
}

/// strike:<id>  the Avatar's blow lands on an actor, wherever it stands
fn cmd_strike(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.receive_hit(num(arg)? as u16, AVATAR, 0, 0, 0);
    Ok(Flow::Done)
}

/// face:<id>  the Avatar turns and strikes towards an actor
fn cmd_face(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (av, t) = (game.avatar(), game.map().get(num(arg)? as u16).copied().ok_or("no such item")?);
    let a = ((t.y - av.y) as f32).atan2((t.x - av.x) as f32).to_degrees();
    // east is direction 2, south 4 (y grows to the south)
    let dir = (((a + 360.0 + 22.5) / 45.0) as i32 + 2) & 7;
    println!("face: avatar {},{} target {},{} dir {dir}", av.x, av.y, t.x, t.y);
    game.avatar_attack(Some(dir as u8), false);
    Ok(Flow::Done)
}

/// fight:<id>  fighting by clicking: a click on this enemy (fighthold:<id> keeps the button down)
fn cmd_fight(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.fight_target(num(arg)? as u16);
    game.combat_release();
    Ok(Flow::Done)
}

/// fightclick:<x>,<y>  a left click at a view position, as fighting by clicking takes it
fn cmd_fightclick(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    println!("{cmd}: {}", if game.combat_click(x, y) { "taken" } else { "not for fighting" });
    game.combat_release();
    Ok(Flow::Done)
}

fn cmd_fighthold(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.fight_target(num(arg)? as u16);
    Ok(Flow::Done)
}

fn cmd_hp(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let ids: Vec<u16> = game.map().items.iter().filter(|it| it.is_actor() && it.in_world() && it.fast).map(|it| it.id).collect();
    for id in ids {
        let s = game.state(id);
        println!("actor {id}: hp {} dex {} side {:X} enemies {:X}{}{}", s.hp, s.dex, s.alignment, s.enemy_alignment,
                 if s.in_combat { " fighting" } else { "" }, if s.dead { " dead" } else { "" });
    }
    Ok(Flow::Done)
}

/// bars  draw the picture and list the enemies that got a health bar
fn cmd_bars(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.draw(surf, opts, false);
    let ids: Vec<String> = game.ui.bar_areas.iter().map(|a| a.0.to_string()).collect();
    println!("bars: [{}]", ids.join(","));
    Ok(Flow::Done)
}

/// hidden:<ticks>  run on and report enemies with a health bar but no visible pixel
fn cmd_hidden(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let ticks = num(arg)?;
    let mut reported = 0;
    for _ in 0..ticks {
        game.step(None);
        game.draw(surf, opts, false);
        let foes: Vec<(usize, u16)> = game.view.painted.iter().filter_map(|p| {
            let it = game.map().items.get(p.index)?;
            let s = game.fights.states.get(&it.id)?;
            (it.id != AVATAR && it.is_actor() && !s.dead && s.in_combat && s.target == AVATAR).then_some((p.index, it.id))
        }).collect();
        for (index, id) in foes {
            let seen = game.view.pixel_ids.iter().filter(|&&v| v == index as u32 + 1).count();
            if seen == 0 && reported < 20 {
                reported += 1;
                let it = game.map().items[index];
                println!("hidden: tick {} actor {id} shape {} at {},{},{} screen {:?} - bar without a visible pixel", game.tick, it.shape, it.x, it.y, it.z, game.to_screen(it.x, it.y, it.z));
            }
        }
    }
    println!("hidden: {reported} reports");
    Ok(Flow::Done)
}

/// attacknear[:<ticks>]  the attack key, held for a while
fn cmd_attacknear(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.attack_nearest();
    println!("attacknear: target {:?}", game.orders.click_fight.map(|f| f.target));
    for _ in 0..(if arg.is_empty() { 0 } else { num(arg)? }) {
        game.step(None);
    }
    game.combat_release();
    Ok(Flow::Done)
}
