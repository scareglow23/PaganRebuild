//! Test commands about the world: maps, finding and describing items, positions, teleporters.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 27] = [
    ("passersby", cmd_passersby),
    ("map", cmd_map),
    ("near", cmd_near),
    ("chunkcheck", cmd_chunkcheck),
    ("swapcheck", cmd_swapcheck),
    ("floorgaps", cmd_floorgaps),
    ("find", cmd_find),
    ("item", cmd_item),
    ("findf", cmd_findf),
    ("goto", cmd_goto),
    ("sendto", cmd_sendto),
    ("info", cmd_info),
    ("moved", cmd_moved),
    ("mapids", cmd_mapids),
    ("eggsof", cmd_eggsof),
    ("where", cmd_where),
    ("at", cmd_at),
    ("placed", cmd_placed),
    ("screenpos", cmd_screenpos),
    ("count", cmd_count),
    ("shapeinfo", cmd_shapeinfo),
    ("canexist", cmd_canexist),
    ("telecheck", cmd_telecheck),
    ("holes", cmd_holes),
    ("posinfo", cmd_posinfo),
    ("dirs", cmd_dirs),
    ("inside", cmd_inside),
];

/// passersby  the townsfolk walking about (option "passers-by"): where, and how far from the Avatar
fn cmd_passersby(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let av = game.avatar();
    let ids = game.passersby_ids();
    println!("passersby: {}", ids.len());
    for id in ids {
        if let Some(it) = game.map().get(id).copied() {
            println!("  {id} shape {} at {},{},{} ({} away)", it.shape, it.x, it.y, it.z, (it.x - av.x).abs().max((it.y - av.y).abs()));
        }
    }
    Ok(Flow::Done)
}

/// map:<n>  jump to another map, as PgUp/PgDn do
fn cmd_map(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let n = num(arg)? as usize;
    if !game.map_exists(n) {
        return Err(format!("there is no map {n}"));
    }
    jump_to_map(game, n, None);
    // the pointer is still over the view: ask what lies under it before the next frame
    println!("{cmd}: under the pointer {:?}", game.pick(320, 200));
    Ok(Flow::Done)
}

fn cmd_near(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let range = if arg.is_empty() { 256 } else { num(arg)? };
    let av = game.avatar();
    for i in game.map().items_near(av.x, av.y, range) {
        println!("{}", game.describe(&game.map().items[i]));
    }
    Ok(Flow::Done)
}

/// chunkcheck[:<quality>,...]  from every trigger egg of these kinds on this map (default:
/// the ones that lay out or swap a chunk's things), walk the view 2048 away in each of
/// four directions and back, and list the things that live only in the fast area and
/// are missing (or new) afterwards: holes in the floor of a dungeon
fn cmd_chunkcheck(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let kinds: Vec<u16> = if arg.is_empty() { vec![17, 42, 49] } else { arg.split(',').map(|s| num(s).map(|n| n as u16)).collect::<Result<_, _>>()? };
    let family = |g: &Game, it: &map::Item| g.data.infos.get(it.shape as usize).map_or(0, |si| si.family);
    let origins: Vec<(i32, i32, i32)> = game
        .map()
        .items
        .iter()
        .filter(|it| it.in_world() && family(game, it) == 4 && kinds.contains(&it.quality))
        .map(|it| (it.x, it.y, it.z))
        .collect();
    // things near the egg (without their frame: scripts change some)
    let things = |g: &Game, (ox, oy): (i32, i32)| -> std::collections::BTreeSet<(u32, i32, i32, i32)> {
        g.map()
            .items
            .iter()
            .filter(|it| it.in_world() && it.fast && it.flags & crate::eggs::FLG_FAST_ONLY != 0)
            .filter(|it| (it.x - ox).abs() < 1024 && (it.y - oy).abs() < 1024)
            .filter(|it| !g.data.infos.get(it.shape as usize).is_some_and(|si| si.creature))
            .map(|it| (it.shape, it.x, it.y, it.z))
            .collect()
    };
    let cur = game.cur_map;
    let mut bad = 0;
    for (n, &(x, y, z)) in origins.iter().enumerate() {
        game.state_mut(AVATAR).hp = 255;
        game.teleport(AVATAR, cur, (x, y, z));
        for _ in 0..6 {
            game.step(None);
        }
        let before = things(game, (x, y));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for k in (1..=8).chain((0..8).rev()) {
                game.state_mut(AVATAR).hp = 255;
                game.teleport(AVATAR, cur, (x + dx * 256 * k, y + dy * 256 * k, z));
                for _ in 0..2 {
                    game.step(None);
                }
            }
        }
        for _ in 0..6 {
            game.step(None);
        }
        let after = things(game, (x, y));
        let missing: Vec<_> = before.difference(&after).collect();
        let new: Vec<_> = after.difference(&before).collect();
        if !missing.is_empty() || !new.is_empty() {
            bad += 1;
            println!("chunkcheck: egg {n} at {x},{y},{z}: {} missing, {} new; missing e.g. {:?}", missing.len(), new.len(), missing.iter().take(4).collect::<Vec<_>>());
            println!("chunkcheck:   new e.g. {:?}", new.iter().take(4).collect::<Vec<_>>());
        }
    }
    println!("chunkcheck: map {cur}: {} eggs, {bad} with changes", origins.len());
    Ok(Flow::Done)
}

/// swapcheck  for every chunk-swapping egg (q 49): the heights in its chunk on the map
/// and in its pattern chunk (egg q 47 with the same id), to see how they line up
fn cmd_swapcheck(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let chunk = |x: i32, y: i32| (x & 0x7E00, y & 0x7E00);
    let eggs: Vec<map::Item> = game.map().items.iter().filter(|it| it.in_world() && it.shape == 73 && it.quality == 49).copied().collect();
    for e in eggs {
        let Some(t) = game.map().items.iter().find(|it| it.in_world() && it.shape == 73 && it.quality == 47 && it.mapnum == e.mapnum).copied() else { continue };
        let list = |c: (i32, i32)| -> Vec<String> {
            let mut v: Vec<(i32, u32, i32, i32)> = game
                .map()
                .items
                .iter()
                .filter(|it| it.in_world() && it.shape != 73 && it.shape != 1 && chunk(it.x, it.y) == c)
                .map(|it| (it.z, it.shape, it.x & 511, it.y & 511))
                .collect();
            v.sort();
            let mut zs: Vec<String> = Vec::new();
            for (z, s, ..) in &v {
                let k = format!("{s}@{z}");
                if !zs.contains(&k) {
                    zs.push(k);
                }
            }
            zs
        };
        println!("swapcheck: egg at {},{},{} id {} | map: {} | pattern (egg z {}): {}", e.x, e.y, e.z, e.mapnum, list(chunk(e.x, e.y)).join(" "), t.z, list(chunk(t.x, t.y)).join(" "));
    }
    Ok(Flow::Done)
}

/// floorgaps  places in the 128 grid of the floor where nothing lies at the height of
/// the floor around it, but floor on at least three of the four sides (holes, not edges)
fn cmd_floorgaps(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    use crate::hash::HashMap;
    let infos = &game.data.infos;
    // what covers each cell (by its middle) at each height: floor (flat) or anything
    let mut flat: HashMap<(i32, i32, i32), bool> = HashMap::default();
    let mut body: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::default();
    for it in game.map().items.iter().filter(|it| it.in_world()) {
        let Some(si) = infos.get(it.shape as usize) else { continue };
        let (xd, yd, zd) = si.footpad(it.flags & map::FLG_FLIPPED != 0);
        if xd == 0 || yd == 0 {
            continue;
        }
        // cell c holds the points c*128 ..= c*128+127 (a floor tile ends at x = c*128+127);
        // the cells whose middle the thing covers
        let cells = |a: i32, d: i32| ((a - d).div_euclid(128)..=a.div_euclid(128)).filter(move |c| {
            let mid = c * 128 + 64;
            mid > a - d && mid <= a
        });
        for cx in cells(it.x, xd) {
            for cy in cells(it.y, yd) {
                if zd == 0 {
                    flat.insert((cx, cy, it.z), true);
                }
                body.entry((cx, cy)).or_default().push((it.z, it.z + zd.max(8)));
            }
        }
    }
    let mut found = 0;
    let floors: Vec<(i32, i32, i32)> = flat.keys().copied().collect();
    let mut seen = crate::hash::HashSet::default();
    for (cx, cy, z) in floors {
        for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
            if flat.contains_key(&(nx, ny, z)) || !seen.insert((nx, ny, z)) {
                continue;
            }
            let sides = [(nx + 1, ny), (nx - 1, ny), (nx, ny + 1), (nx, ny - 1)].iter().filter(|&&(a, b)| flat.contains_key(&(a, b, z))).count();
            // something else stands there at that height (rock, wall, raised floor)
            let filled = body.get(&(nx, ny)).is_some_and(|v| v.iter().any(|&(lo, hi)| lo <= z + 8 && hi >= z));
            if sides >= 3 && !filled {
                println!("floorgaps: hole at {},{},{} ({sides} sides floor)", nx * 128 + 127, ny * 128 + 127, z);
                found += 1;
            }
        }
    }
    println!("floorgaps: {found} in all");
    Ok(Flow::Done)
}

fn cmd_find(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let shape = num(arg)? as u32;
    for it in game.map().items.iter().filter(|it| !it.gone && it.shape == shape).take(12) {
        println!("{}", game.describe(it));
    }
    Ok(Flow::Done)
}

/// item:<id>  describe one item and the containers it is in
fn cmd_item(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let mut id = num(arg)? as u16;
    while let Some(it) = game.map().get(id) {
        println!("{}", game.describe(it));
        if it.parent == 0 || it.parent == map::ETHEREAL || it.parent == id {
            break;
        }
        id = it.parent;
    }
    Ok(Flow::Done)
}

/// findf:<family>[,<quality>]
fn cmd_findf(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (f, q) = arg.split_once(',').map_or((arg, None), |(f, q)| (f, Some(q)));
    let family = num(f)? as u8;
    let quality = q.map(num).transpose()?;
    let fam = |it: &map::Item| game.data.infos.get(it.shape as usize).map_or(0, |si| si.family);
    for it in game.map().items.iter().filter(|it| it.in_world() && fam(it) == family && quality.is_none_or(|q| it.quality as i32 == q)).take(20) {
        println!("{}", game.describe(it));
    }
    Ok(Flow::Done)
}

fn cmd_goto(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let v: Vec<i32> = arg.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 3 {
        return Err(format!("'{cmd}' expects x,y,z"));
    }
    let map = game.cur_map;
    game.teleport(AVATAR, map, (v[0], v[1], v[2]));
    Ok(Flow::Done)
}

/// sendto:<id>,<map>,<x>,<y>,<z>  send a person elsewhere, as a script does
fn cmd_sendto(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let v: Vec<i32> = arg.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 5 {
        return Err(format!("'{cmd}' expects id,map,x,y,z"));
    }
    game.teleport(v[0] as u16, v[1] as usize, (v[2], v[3], v[4]));
    Ok(Flow::Done)
}

/// info:<id>  describe an item with its weight and volume
fn cmd_info(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let id = num(arg)? as u16;
    match game.map().get(id) {
        Some(it) => println!("info: {} weight {} volume {} parent {}", game.describe(it), game.weight(id), game.volume(id), it.parent),
        None => println!("info: item {id} is gone"),
    }
    Ok(Flow::Done)
}

/// moved  the fixed things of this map that are not where the map data put them
fn cmd_moved(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let fresh = game.original_map(game.cur_map);
    let moved: Vec<(map::Item, map::Item)> = game
        .map()
        .items
        .iter()
        .filter(|it| !it.gone && it.id != 0 && game.data.infos.get(it.shape as usize).is_some_and(|si| si.fixed))
        .filter_map(|it| fresh.get(it.id).filter(|o| o.shape == it.shape && (o.x, o.y, o.z) != (it.x, it.y, it.z)).map(|o| (*it, *o)))
        .collect();
    for (it, o) in &moved {
        println!("moved: {} shape {} at {},{},{} (map data: {},{},{})", it.id, it.shape, it.x, it.y, it.z, o.x, o.y, o.z);
    }
    println!("moved: {} in all", moved.len());
    Ok(Flow::Done)
}

/// mapids  how many items the map keeps, how many of them are gone, the next new id
fn cmd_mapids(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let (n, gone, next) = game.map().id_stats();
    println!("mapids: items {n} gone {gone} next id {next}");
    Ok(Flow::Done)
}

/// eggsof:<shape>  the monster eggs on this map that make creatures of a shape
fn cmd_eggsof(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let shape = num(arg)? as u16;
    let found: Vec<map::Item> = game.map().items.iter().filter(|it| !it.gone && it.quality & 0x7FF == shape && it.shape != shape as u32).copied().collect();
    for it in found {
        println!("eggsof: id {} shape {} at {},{},{} activity {} range {:#X}", it.id, it.shape, it.x, it.y, it.z, it.mapnum & 7, it.npc);
    }
    Ok(Flow::Done)
}

fn cmd_where(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    for id in arg.split(',').filter_map(|s| s.trim().parse::<u16>().ok()) {
        match game.map().get(id) {
            Some(it) => {
                let facing = game.actors.get(&id).map_or(String::new(), |m| format!(" dir {}", m.dir));
                println!("where {id}: {},{},{} shape {} frame {}{facing}", it.x, it.y, it.z, it.shape, it.frame)
            }
            None => println!("where {id}: not on this map"),
        }
    }
    Ok(Flow::Done)
}

/// at:<x>,<y>  the items drawn at a view position (top one first)
fn cmd_at(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    game.draw(surf, opts, false);
    let (x, y) = point(arg)?;
    let top = game.pick_world(x, y, 0);
    if let Some(id) = top {
        let it = game.map().get(id).copied().unwrap_or_default();
        println!("{cmd}: {}", game.describe(&it));
        if let Some(next) = game.pick_world(x, y, id) {
            let it = game.map().get(next).copied().unwrap_or_default();
            println!("{cmd}: behind it {}", game.describe(&it));
        }
    } else {
        println!("{cmd}: nothing");
    }
    Ok(Flow::Done)
}

/// placed:<id>  turned view: where an item is drawn compared with the middle
/// of its box (should not change with the turn)
fn cmd_placed(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.draw(surf, opts, false);
    let it = game.map().get(num(arg)? as u16).copied().ok_or("no such item")?;
    let si = game.data.infos.get(it.shape as usize).copied().unwrap_or_default();
    let (xd, yd, _) = si.footpad(it.flags & map::FLG_FLIPPED != 0);
    let (bx, by) = game.to_screen(it.x - xd / 2, it.y - yd / 2, it.z);
    let index = game.map().index(it.id).ok_or("gone")?;
    match game.turn.drawn_centre(index) {
        Some((dx, dy)) => println!("{cmd}: box middle {bx},{by}, picture middle {dx},{dy}, apart {}", dx - bx),
        None => println!("{cmd}: not drawn"),
    }
    Ok(Flow::Done)
}

/// screenpos:<id>  where an item is in the view (as last drawn)
fn cmd_screenpos(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.draw(surf, opts, false);
    let it = game.map().get(num(arg)? as u16).copied().ok_or("no such item")?;
    let (sx, sy) = game.to_screen(it.x, it.y, it.z);
    println!("{cmd}: {sx},{sy}");
    Ok(Flow::Done)
}

/// count:<shape>,<shape>...  how many items of these shapes the map has
fn cmd_count(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let shapes: Vec<u32> = arg.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let n = game.map().items.iter().filter(|it| it.in_world() && shapes.contains(&it.shape)).count();
    println!("{cmd}: {n}");
    Ok(Flow::Done)
}

/// shapeinfo:<shape>  frames, sizes and drawn pixels of a shape, and its type flags
fn cmd_shapeinfo(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let n = num(arg)? as u32;
    let si = game.data.infos.get(n as usize).copied();
    if let Some(si) = si {
        println!("shapeinfo {n}: family {} solid {} translucent {} draw {} pad {:?} raw {:02X?} monster {}", si.family, si.solid, si.translucent, si.draw, si.footpad(false), si.raw, game.data.monsters.contains_key(&n));
    }
    if let Some(shape) = game.data.shapes.get(n) {
        for (i, f) in shape.frames.iter().enumerate().take(40) {
            let drawn = f.pixels.iter().filter(|&&p| p != shape::TRANSPARENT).count();
            println!("  frame {i}: {}x{} drawn {drawn}", f.width, f.height);
        }
    }
    for action in 0..64u32 {
        if let Some(a) = game.data.anims.action(n, action) {
            let frames: Vec<u32> = (0..a.size).map(|k| a.frame(0, k).frame).collect();
            println!("  action {action}: frames {frames:?}");
        }
    }
    Ok(Flow::Done)
}

/// canexist:<shape>,<x>,<y>,<z>  could the shape stand there (what is in the way)
fn cmd_canexist(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<i32> = arg.split(',').map(num).collect::<Result<_, _>>()?;
    if v.len() != 4 {
        return Err(format!("'{cmd}' expects shape,x,y,z"));
    }
    let si = game.data.infos.get(v[0] as usize).copied().ok_or("no such shape")?;
    let (xd, yd, zd) = si.footpad(false);
    let b = map::WorldBox { x: v[1], y: v[2], z: v[3], xd, yd, zd };
    let nowhere = map::WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
    let info = game.map().position_info(&b, &nowhere, &game.data.infos, 0);
    let what = info.blocker.and_then(|id| game.map().get(id).map(|it| game.describe(it))).unwrap_or_default();
    let support = info.support.and_then(|id| game.map().get(id).map(|it| it.shape));
    println!("canexist {}: {} {what} (supported {} by shape {:?}, roof {:?}, floor {:?})", arg, if game.can_exist(v[0] as u32, v[1], v[2], v[3], 0) { "yes" } else { "no" }, info.supported, support, info.roof_z, info.floor_z);
    Ok(Flow::Done)
}

/// telecheck  every teleporter of every map: is there a destination, and can the Avatar stand there?
fn cmd_telecheck(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let maps: Vec<usize> = (0..256).filter(|&n| game.map_exists(n)).collect();
    for &m in &maps {
        game.load_map(m);
    }
    let is_tele = |g: &Game, it: &map::Item| g.data.infos.get(it.shape as usize).is_some_and(|si| si.family == shape::FAMILY_TELEPORTEGG);
    let (mut ok, mut bad) = (0, 0);
    let av = game.data.infos[1];
    let (xd, yd, zd) = av.footpad(false);
    for &m in &maps {
        let sources: Vec<map::Item> = game.maps[&m].items.iter().filter(|it| it.in_world() && it.frame != 1 && is_tele(game, it)).copied().collect();
        for src in sources {
            let (to, n) = (src.mapnum as usize, src.quality & 0xFF);
            let Some(dest) = game.maps.get(&to).and_then(|md| md.items.iter().find(|it| it.in_world() && it.frame == 1 && it.quality & 0xFF == n && is_tele(game, it))).copied() else {
                println!("telecheck: map {m} at {},{},{} -> map {to} egg {n}: NO DESTINATION", src.x, src.y, src.z);
                bad += 1;
                continue;
            };
            // somewhere at or a little above the egg he has to fit, with ground below
            let md = &game.maps[&to];
            let nowhere = map::WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
            let fits = (0..8).any(|k| {
                let b = map::WorldBox { x: dest.x, y: dest.y, z: dest.z + 8 * k, xd, yd, zd };
                md.position_info(&b, &nowhere, &game.data.infos, 0).valid
            });
            if fits {
                ok += 1;
            } else {
                bad += 1;
                println!("telecheck: map {m} at {},{},{} -> map {to} egg {n} at {},{},{}: BLOCKED", src.x, src.y, src.z, dest.x, dest.y, dest.z);
            }
        }
    }
    println!("telecheck: {ok} fine, {bad} broken");
    Ok(Flow::Done)
}

/// holes:<x0>,<y0>,<x1>,<y1>,<z>  where the Avatar standing at height z (every 16) would have nothing under him
fn cmd_holes(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<i32> = arg.split(',').map(num).collect::<Result<_, _>>()?;
    if v.len() != 5 {
        return Err(format!("'{cmd}' expects x0,y0,x1,y1,z"));
    }
    let av = game.avatar();
    let info = game.data.infos[av.shape as usize];
    let mut n = 0;
    for y in (v[1]..=v[3]).step_by(16) {
        for x in (v[0]..=v[2]).step_by(16) {
            let b = map::WorldBox::of(&map::Item { x, y, z: v[4], ..av }, &info);
            let pi = game.map().position_info(&b, &b, &game.data.infos, av.id);
            if pi.valid && !pi.supported {
                println!("holes: {x},{y} floor {:?}", pi.floor_z);
                n += 1;
            }
        }
    }
    println!("holes: {n} in all");
    Ok(Flow::Done)
}

/// posinfo:<id>  what the world says about where an item stands
fn cmd_posinfo(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let id = num(arg)? as u16;
    let it = *game.map().get(id).ok_or("no such item")?;
    let info = game.data.infos[it.shape as usize];
    let b = map::WorldBox::of(&it, &info);
    let pi = game.map().position_info(&b, &b, &game.data.infos, id);
    println!("posinfo {id}: valid {} supported {} floor {:?} roof {:?} box {}x{}x{}", pi.valid, pi.supported, pi.floor_z, pi.roof_z, b.xd, b.yd, b.zd);
    Ok(Flow::Done)
}

/// dirs:<id>,<id>...  the direction and animation of actors
fn cmd_dirs(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    for id in arg.split(',').filter_map(|s| s.trim().parse::<u16>().ok()) {
        let (dir, anim, frame) = match game.actors.get(&id) {
            Some(m) => (m.dir as i32, m.shown().0 as i32, game.map().get(id).map_or(-1, |it| it.frame as i32)),
            None => (-1, -1, -1),
        };
        println!("dirs: {id} dir {dir} anim {anim} frame {frame} npc_dir {:?}", game.people.npc_dirs.get(&id));
    }
    Ok(Flow::Done)
}

fn cmd_inside(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let owner = num(arg)? as u16;
    for it in game.map().items.iter().filter(|it| !it.gone && it.parent == owner) {
        println!("{}", game.describe(it));
    }
    Ok(Flow::Done)
}
