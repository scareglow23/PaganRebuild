//! Test commands for the picture: drawing, smoothing, zoom, turning, sheets of shapes, the pointer.

use super::*;

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    match COMMANDS.iter().find(|(n, _)| *n == name) {
        Some((_, command)) => command(game, surf, opts, st, name, arg, cmd),
        None => Ok(Flow::Unknown),
    }
}

/// The commands of this group by name.
const COMMANDS: [(&str, super::Group); 46] = [
    ("blackframes", cmd_blackframes),
    ("outline", cmd_outline),
    ("pickat", cmd_pickat),
    ("grabat", cmd_grabat),
    ("gumpshot", cmd_gumpshot),
    ("usable", cmd_usable),
    ("shapeshot", cmd_shapeshot),
    ("drawtime", cmd_drawtime),
    ("bandcheck", cmd_bandcheck),
    ("gpucheck", cmd_gpucheck),
    ("gumpinfo", cmd_gumpinfo),
    ("minimap", cmd_minimap),
    ("smooth", cmd_smooth),
    ("flicker", cmd_flicker),
    ("blink", cmd_blink),
    ("sweep", cmd_sweep),
    ("roofline", cmd_roofline),
    ("ordersweep", cmd_ordersweep),
    ("order", cmd_order),
    ("anims", cmd_anims),
    ("hover", cmd_hover),
    ("mapsheet", cmd_mapsheet),
    ("names", cmd_names),
    ("namesize", cmd_namesize),
    ("showall", cmd_showall),
    ("shapesheet", cmd_shapesheet),
    ("gumpsheet", cmd_gumpsheet),
    ("fontsheet", cmd_fontsheet),
    ("colors", cmd_colors),
    ("heads", cmd_heads),
    ("sheet", cmd_sheet),
    ("mousesheet", cmd_mousesheet),
    ("pointer", cmd_pointer),
    ("turnlive", cmd_turnlive),
    ("shimmer", cmd_shimmer),
    ("zoom", cmd_zoom),
    ("textsize", cmd_textsize),
    ("angle", cmd_angle),
    ("modelsheet", cmd_modelsheet),
    ("directions", cmd_directions),
    ("turn", cmd_turn),
    ("light", cmd_light),
    ("mouse", cmd_mouse),
    ("sunlitscan", cmd_sunlitscan),
    ("sunlit", cmd_sunlit),
    ("animframes", cmd_animframes),
];

/// blackframes  shapes with frames that are black all over (they only hide what is behind)
fn cmd_blackframes(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let pal = game.data.pal.colors;
    for n in 0..game.data.infos.len() as u32 {
        let Some(shape) = game.data.shapes.get(n) else { continue };
        let black: Vec<usize> = shape
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.width > 0 && f.pixels.iter().any(|&p| p != crate::shape::TRANSPARENT) && f.pixels.iter().all(|&p| p == crate::shape::TRANSPARENT || pal[p as usize] & 0xFFFFFF == 0))
            .map(|(i, _)| i)
            .collect();
        if !black.is_empty() {
            println!("blackframes {n}: {} of {} frames: {:?}", black.len(), shape.frames.len(), black);
        }
    }
    Ok(Flow::Done)
}

/// outline  draw the view and tell how many pixels of the Avatar's outline show
fn cmd_outline(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.draw(surf, opts, false);
    let av = game.map().index(crate::game::AVATAR);
    println!("outline: {} pixels", game.draw_avatar_outline(surf, av));
    Ok(Flow::Done)
}

/// pickat:<x>,<y>  draw the view and tell which item is painted at this pixel
fn cmd_pickat(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    let id = game.view.pixel_ids.get((y * surf.width + x) as usize).copied().unwrap_or(0);
    match id.checked_sub(1).and_then(|i| game.map().items.get(i as usize)) {
        Some(it) => println!("pickat {x},{y}: {}", game.describe(it)),
        None => println!("pickat {x},{y}: nothing"),
    }
    Ok(Flow::Done)
}

/// grabat:<x>,<y>  draw the view and tell what a click there reaches
fn cmd_grabat(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    match game.pick(x, y).and_then(|id| game.map().get(id).copied()) {
        Some(it) => println!("grabat {x},{y}: {}", game.describe(&it)),
        None => println!("grabat {x},{y}: nothing"),
    }
    Ok(Flow::Done)
}

/// paint all frames of gump shapes side by side (debug aid)
fn cmd_gumpshot(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.draw(surf, opts, false);
    surf.pixels.fill(0x303030);
    let (mut x, mut y, mut row_h) = (4, 4, 0);
    for sh in arg.split(',').filter_map(|s| s.trim().parse::<u32>().ok()) {
        let Some(shape) = game.data.gump_shapes.get(sh) else { continue };
        for (i, f) in shape.frames.iter().enumerate() {
            if x + f.width + 4 > surf.width {
                x = 4;
                y += row_h + 4;
                row_h = 0;
            }
            surf.paint(f, x + f.xoff, y + f.yoff, false, false, &game.data.pal);
            println!("gump {sh} frame {i}: {}x{} off {},{} at {x},{y}", f.width, f.height, f.xoff, f.yoff);
            x += f.width + 4;
            row_h = row_h.max(f.height);
        }
    }
    Ok(Flow::Stop)
}

/// usable:<id>  can it be used (a script for it) and carried
fn cmd_usable(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let id = num(arg)? as u16;
    let Some(it) = game.map().get(id).copied() else { return Err(format!("no item {id}")) };
    let look = game.scripts.usecode.event(game.usecode_class(&it), 0).is_some();
    let si = game.data.infos.get(it.shape as usize).copied().unwrap_or_default();
    println!("usable {id}: shape {} use {} look {} carry {} family {} equip {}", it.shape, game.usable(id), look, game.portable(id), si.family, si.equip_type);
    Ok(Flow::Done)
}

/// shapeshot:<shape>,..  paint all frames of world shapes side by side on grey (debug aid)
fn cmd_shapeshot(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.draw(surf, opts, false);
    surf.pixels.fill(0x303030);
    let (mut x, mut y, mut row_h) = (4, 4, 0);
    for sh in arg.split(',').filter_map(|s| s.trim().parse::<u32>().ok()) {
        let Some(shape) = game.data.shapes.original(sh) else { continue };
        for (i, f) in shape.frames.iter().enumerate() {
            if x + f.width + 4 > surf.width {
                x = 4;
                y += row_h + 4;
                row_h = 0;
            }
            surf.paint(f, x + f.xoff, y + f.yoff, false, false, &game.data.pal);
            println!("shape {sh} frame {i}: {}x{} at {x},{y}", f.width, f.height);
            x += f.width + 4;
            row_h = row_h.max(f.height);
        }
    }
    Ok(Flow::Stop)
}

/// drawtime:<n>  draws n pictures (walking a little between them) and tells how long they took
fn cmd_drawtime(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let n = num(arg)?.max(1);
    let start = std::time::Instant::now();
    let mut worst = std::time::Duration::ZERO;
    for i in 0..n {
        if i % 4 == 0 {
            game.step(None);
        }
        let t = std::time::Instant::now();
        game.draw(surf, opts, false);
        worst = worst.max(t.elapsed());
    }
    println!("drawtime: {:.2} ms per picture, worst {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / n as f64, worst.as_secs_f64() * 1000.0);
    Ok(Flow::Done)
}

/// bandcheck[:ghost]  the world painted in bands (over the cores) and in one piece must be
/// the same (ghost: with the Avatar invisible, painted faintly)
fn cmd_bandcheck(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let ghost = arg == "ghost";
    let av = game.map().index(crate::game::AVATAR);
    if let (true, Some(i)) = (ghost, av) {
        game.map_mut().items[i].flags |= crate::map::FLG_INVISIBLE;
    }
    crate::view::ONE_BAND.store(true, std::sync::atomic::Ordering::Relaxed);
    game.draw(surf, opts, false);
    let (one, one_ids) = (surf.pixels.clone(), game.view.pixel_ids.clone());
    crate::view::ONE_BAND.store(false, std::sync::atomic::Ordering::Relaxed);
    game.draw(surf, opts, false);
    let bad = one.iter().zip(&surf.pixels).filter(|(a, b)| a != b).count();
    let bad_ids = one_ids.iter().zip(&game.view.pixel_ids).filter(|(a, b)| a != b).count();
    if let (true, Some(i)) = (ghost, av) {
        game.map_mut().items[i].flags &= !crate::map::FLG_INVISIBLE;
    }
    println!("bandcheck: {bad} pixels and {bad_ids} ids differ ({}x{})", surf.width, surf.height);
    Ok(Flow::Done)
}

/// gpucheck:<w>x<h>[s|c]  the picture stretched to w x h (s: smoothed first) on the graphics
/// card and on the processor: both must give the same pixels; c: with the tube monitor's
/// look (graphics card only), written to gpucheck_crt.png
fn cmd_gpucheck(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let (smooth, crt) = (arg.ends_with('s'), arg.ends_with('c'));
    let (w, h) = arg.trim_end_matches(['s', 'c']).split_once('x').and_then(|(w, h)| Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?))).ok_or("gpucheck:<w>x<h>[s|c]")?;
    game.draw(surf, opts, false);
    let (sw, sh) = (surf.width as usize, surf.height as usize);
    let sharp: &[bool] = if game.view.sharp.len() == sw * sh { &game.view.sharp } else { &[] };
    if crt {
        let gpu = crate::gpuscale::offscreen(&surf.pixels, sw as u32, sh as u32, crate::gpuscale::Look::Crt, (w as u32, h as u32))?;
        let mut a = Surface::new(w as i32, h as i32);
        a.pixels = gpu;
        save_png(Path::new("gpucheck_crt.png"), &a).map_err(|e| e.to_string())?;
        println!("gpucheck: {sw}x{sh} -> {w}x{h} as on a tube monitor, in gpucheck_crt.png");
        return Ok(Flow::Done);
    }
    let t = Instant::now();
    let look = if smooth { crate::gpuscale::Look::Xbr(sharp) } else { crate::gpuscale::Look::Plain };
    let gpu = crate::gpuscale::offscreen(&surf.pixels, sw as u32, sh as u32, look, (w as u32, h as u32))?;
    let gpu_ms = t.elapsed().as_secs_f64() * 1000.0;
    let (mut big, mut up) = (Vec::new(), crate::upscale::Upscale::default());
    let src = if smooth {
        crate::xbr::scale2x_keeping(&surf.pixels, sw, sh, &mut big, sharp);
        (2 * sw, 2 * sh)
    } else {
        big = surf.pixels.clone();
        (sw, sh)
    };
    up.run(&big, src, (w, h));
    let bad = gpu.iter().zip(&up.out).filter(|(a, b)| a != b).count();
    println!("gpucheck: {bad} of {} pixels differ ({sw}x{sh} -> {w}x{h}{}, graphics card {gpu_ms:.0} ms with setting up)", w * h, if smooth { " smoothed" } else { "" });
    if bad > 0 {
        let mut a = Surface::new(w as i32, h as i32);
        a.pixels = gpu;
        let _ = save_png(Path::new("gpucheck_gpu.png"), &a);
        a.pixels = up.out.clone();
        let _ = save_png(Path::new("gpucheck_cpu.png"), &a);
    }
    Ok(Flow::Done)
}

/// gumpinfo:<gump shape>  size of its first frame, its item area and a dump of the frame
fn cmd_gumpinfo(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let n = num(arg)? as u32;
    let area = game.data.gump_areas.get(n.saturating_sub(1) as usize).copied();
    let shape = game.data.gump_shapes.get(n).ok_or("no such gump")?;
    let f = shape.frames.first().ok_or("no frame")?;
    println!("gumpinfo: {}x{} off {},{} area {:?}", f.width, f.height, f.xoff, f.yoff, area);
    for y in 0..f.height {
        let row: Vec<String> = (0..f.width).map(|x| {
            let p = f.pixels[(y * f.width + x) as usize];
            if p == shape::TRANSPARENT { "..".to_string() } else { format!("{p:02x}") }
        }).collect();
        println!("gumprow {y:3}: {}", row.join(""));
    }
    Ok(Flow::Done)
}

fn cmd_minimap(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.options.minimap = arg != "off";
    Ok(Flow::Done)
}

/// walk with the smooth option on, drawing between the ticks,
/// and report where the Avatar is drawn in the view
fn cmd_smooth(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.smooth_camera = true;
    let dir = num(arg)? as u8 & 7;
    // (then stand still for a while: the view comes to rest)
    for tick in 0..64 {
        game.step((tick < 44).then_some(MoveInput { dir, run: false }));
        for sub in 0..2 {
            game.draw(surf, opts, false);
            let av = game.avatar();
            let shown = game.shown_at(AVATAR).unwrap_or((0, 0));
            let o = game.view.origin;
            println!("tick {tick}.{sub}: world {},{} drawn at {},{} view {},{}", av.x, av.y, shown.0, shown.1, o.0, o.1);
            std::thread::sleep(Duration::from_millis(16));
        }
    }
    game.options.smooth_camera = false;
    Ok(Flow::Done)
}

/// flicker:<dir>  walk with the smooth option and compare two
/// pictures drawn within the same tick: apart from the walking
/// Avatar they must show the same world, only shifted
fn cmd_flicker(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.smooth_camera = true;
    let dir = num(arg)? as u8 & 7;
    let (w, h) = (surf.width, surf.height);
    let mut total = 0;
    let mut roof = opts.z_limit;
    for tick in 0..60 {
        game.step(Some(MoveInput { dir, run: false }));
        game.draw(surf, opts, false);
        if opts.z_limit != roof {
            roof = opts.z_limit;
            let av = game.avatar();
            println!("tick {tick}: roofs hidden above {roof:?} (avatar at {},{},{})", av.x, av.y, av.z);
        }
        let (first, o1) = (surf.pixels.clone(), game.view.origin);
        std::thread::sleep(Duration::from_millis(14));
        game.draw(surf, opts, false);
        let o2 = game.view.origin;
        let (sx, sy) = (o2.0 - o1.0, o2.1 - o1.1);
        let (mut n, mut bx0, mut by0, mut bx1, mut by1) = (0, w, h, 0, 0);
        for y in 8..h - 8 {
            for x in 8..w - 8 {
                // leave out the Avatar in the middle
                if ((x - w / 2).abs() < 40 && (y - h / 2).abs() < 70) || (x > w - 40 && y < 40) {
                    continue;
                }
                if surf.pixels[(y * w + x) as usize] != first[((y + sy) * w + x + sx) as usize] {
                    n += 1;
                    (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                }
            }
        }
        if n > 0 {
            total += 1;
            println!("tick {tick}: {n} pixels differ in {bx0},{by0}..{bx1},{by1} (view moved by {sx},{sy})");
            if total == 1 {
                let mut a = Surface::new(w, h);
                a.pixels = first;
                let _ = save_png(Path::new("flicker_a.png"), &a);
                let _ = save_png(Path::new("flicker_b.png"), surf);
            }
        }
    }
    println!("flicker: {total} of 60 ticks with differences");
    game.options.smooth_camera = false;
    Ok(Flow::Done)
}

/// blink:<dir>  walk and look for pixels that change and change
/// straight back from one picture to the next but one: things
/// that flicker (what is animated goes on instead of back)
fn cmd_blink(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.smooth_camera = true;
    let dir = num(arg)? as u8 & 7;
    let (w, h) = (surf.width, surf.height);
    let mut frames: Vec<(Vec<u32>, (i32, i32))> = Vec::new();
    let (mut worst, mut ticks) = (0, 0);
    for tick in 0..90 {
        game.step(Some(MoveInput { dir, run: false }));
        std::thread::sleep(Duration::from_millis(5));
        game.draw(surf, opts, false);
        frames.push((surf.pixels.clone(), game.view.origin));
        if frames.len() < 3 {
            continue;
        }
        let (a, b, c) = (&frames[frames.len() - 3], &frames[frames.len() - 2], &frames[frames.len() - 1]);
        let (mut n, mut bx0, mut by0, mut bx1, mut by1) = (0, w, h, 0, 0);
        for y in 16..h - 16 {
            for x in 16..w - 16 {
                if ((x - w / 2).abs() < 50 && (y - h / 2).abs() < 80) || (x > w - 40 && y < 40) {
                    continue;
                }
                // the same spot of the world in the three pictures
                let at = |f: &(Vec<u32>, (i32, i32))| {
                    let (px, py) = (x + b.1 .0 - f.1 .0, y + b.1 .1 - f.1 .1);
                    (px >= 0 && py >= 0 && px < w && py < h).then(|| f.0[(py * w + px) as usize])
                };
                if let (Some(pa), Some(pb), Some(pc)) = (at(a), at(b), at(c)) {
                    // PAGAN_BLINK=any: every change from one picture to the next
                    let any = std::env::var_os("PAGAN_BLINK").is_some();
                    if (any && pb != pc) || (!any && pa == pc && pa != pb) {
                        n += 1;
                        (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                    }
                }
            }
        }
        if n > 40 {
            ticks += 1;
            let av = game.avatar();
            println!("tick {tick}: {n} pixels blink in {bx0},{by0}..{bx1},{by1} (avatar {},{},{})", av.x, av.y, av.z);
            if n > worst {
                worst = n;
                for (name, f) in [("blink_a.png", a), ("blink_b.png", b)] {
                    let mut img = Surface::new(w, h);
                    img.pixels = f.0.clone();
                    let _ = save_png(Path::new(name), &img);
                }
            }
        }
    }
    println!("blink: {ticks} of 90 ticks, worst {worst} pixels");
    game.options.smooth_camera = false;
    Ok(Flow::Done)
}

/// sweep:<x>,<y>,<z>,<x2>,<y2>  move the view along a line over the
/// standing world: whatever changes besides the shift is drawn
/// differently depending on where the view is - it would flicker
fn cmd_sweep(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let v: Vec<i32> = arg.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 5 {
        return Err(format!("'{cmd}' expects x,y,z,x2,y2"));
    }
    let (w, h) = (surf.width, surf.height);
    let steps = ((v[3] - v[0]).abs().max((v[4] - v[1]).abs()) / 12).max(1);
    let old = game.camera;
    opts.hide = AVATAR;
    let mut prev: Option<(Vec<u32>, (i32, i32))> = None;
    let mut bad = 0;
    for i in 0..=steps {
        let (cx, cy) = (v[0] + (v[3] - v[0]) * i / steps, v[1] + (v[4] - v[1]) * i / steps);
        game.camera = effects::CameraMode::Fixed(cx, cy, v[2]);
        game.draw(surf, opts, true);
        let o2 = game.view.origin;
        if let Some((first, o1)) = &prev {
            let (mut n, mut bx0, mut by0, mut bx1, mut by1) = (0, w, h, 0, 0);
            for y in 0..h {
                for x in 0..w {
                    let (x1, y1) = (x + o2.0 - o1.0, y + o2.1 - o1.1);
                    if x1 < 0 || y1 < 0 || x1 >= w || y1 >= h || (x > w - 40 && y < 40) || (x1 > w - 40 && y1 < 40) {
                        continue;
                    }
                    if surf.pixels[(y * w + x) as usize] != first[(y1 * w + x1) as usize] {
                        n += 1;
                        (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                    }
                }
            }
            if n > 0 {
                bad += 1;
                if bad <= 12 {
                    println!("view at {cx},{cy}: {n} pixels differ in {bx0},{by0}..{bx1},{by1}");
                }
                if bad == 1 {
                    let mut a = Surface::new(w, h);
                    a.pixels = first.clone();
                    let _ = save_png(Path::new("sweep_a.png"), &a);
                    let _ = save_png(Path::new("sweep_b.png"), surf);
                }
            }
        }
        prev = Some((surf.pixels.clone(), o2));
    }
    opts.hide = 0;
    game.camera = old;
    println!("sweep: {bad} of {steps} steps with differences");
    Ok(Flow::Done)
}

/// roofline:<x>,<y>,<z>,<x2>,<y2>  put the Avatar on every spot of a
/// line: where the upper floors are hidden, and how often that
/// switches with the whole footprint compared to its middle
fn cmd_roofline(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let v: Vec<i32> = arg.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 5 {
        return Err(format!("'{cmd}' expects x,y,z,x2,y2"));
    }
    let steps = ((v[3] - v[0]).abs().max((v[4] - v[1]).abs()) / 4).max(1);
    let home = game.avatar();
    let (mut whole, mut middle) = ((None, 0), (None, 0));
    let mut line = String::new();
    for i in 0..=steps {
        let (x, y) = (v[0] + (v[3] - v[0]) * i / steps, v[1] + (v[4] - v[1]) * i / steps);
        game.with_item_pub(AVATAR, |it| (it.x, it.y, it.z) = (x, y, v[2]));
        let av = game.avatar();
        let probe = crate::map::WorldBox { x, y, z: v[2], xd: 64, yd: 64, zd: 16 };
        let w = game.map().position_info(&probe, &probe, &game.data.infos, AVATAR).roof_z;
        let m = crate::game::roof_above(game.map(), &game.data.infos, &av);
        for (now, seen) in [(w, &mut whole), (m, &mut middle)] {
            if now != seen.0 {
                *seen = (now, seen.1 + 1);
            }
        }
        line.push(match (w.is_some(), m.is_some()) { (true, true) => '#', (true, false) => 'w', (false, true) => 'm', _ => '.' });
    }
    game.with_item_pub(AVATAR, |it| (it.x, it.y, it.z) = (home.x, home.y, home.z));
    println!("roofline: {line}");
    println!("roofline: whole footprint switched {} times, middle {} times", whole.1, middle.1);
    Ok(Flow::Done)
}

/// ordersweep:<x>,<y>,<z>,<x2>,<y2>  put the Avatar on every spot
/// of a line and draw with and without him: besides his figure
/// nothing may change
fn cmd_ordersweep(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let v: Vec<i32> = arg.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 5 {
        return Err(format!("'{cmd}' expects x,y,z,x2,y2"));
    }
    let (w, h) = (surf.width, surf.height);
    let steps = ((v[3] - v[0]).abs().max((v[4] - v[1]).abs()) / 7).max(1);
    let (old, home) = (game.camera, game.avatar());
    let mut bad = 0;
    let mut inside = 0u32;
    let (mut roof, mut roofs) = (None, 0);
    for i in 0..=steps {
        let (cx, cy) = (v[0] + (v[3] - v[0]) * i / steps, v[1] + (v[4] - v[1]) * i / steps);
        game.camera = effects::CameraMode::Fixed(cx, cy, v[2]);
        game.with_item_pub(AVATAR, |it| (it.x, it.y, it.z) = (cx, cy, v[2]));
        game.draw(surf, opts, false);
        if opts.z_limit != roof {
            roof = opts.z_limit;
            roofs += 1;
            if roofs <= 12 {
                println!("avatar at {cx},{cy}: everything from height {roof:?} on is hidden");
            }
        }
        game.draw(surf, opts, true);
        let with = surf.pixels.clone();
        opts.hide = AVATAR;
        game.draw(surf, opts, true);
        opts.hide = 0;
        let (mut n, mut bx0, mut by0, mut bx1, mut by1) = (0, w, h, 0, 0);
        for y in 0..h {
            for x in 0..w {
                if (x - w / 2).abs() < 36 && y - h / 2 > -80 && y - h / 2 < 16 {
                    // (counted to see that the Avatar really is in one picture only)
                    inside += (surf.pixels[(y * w + x) as usize] != with[(y * w + x) as usize]) as u32;
                    continue;
                }
                if surf.pixels[(y * w + x) as usize] != with[(y * w + x) as usize] {
                    n += 1;
                    (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                }
            }
        }
        if n > 0 {
            bad += 1;
            if bad <= 10 {
                println!("avatar at {cx},{cy}: {n} pixels differ in {bx0},{by0}..{bx1},{by1}");
            }
            if bad == 1 {
                let mut a = Surface::new(w, h);
                a.pixels = with;
                let _ = save_png(Path::new("order_a.png"), &a);
                let _ = save_png(Path::new("order_b.png"), surf);
            }
        }
    }
    game.camera = old;
    game.with_item_pub(AVATAR, |it| (it.x, it.y, it.z) = (home.x, home.y, home.z));
    println!("ordersweep: {bad} of {steps} spots with differences ({inside} pixels of the figure itself), roof hiding switched {roofs} times");
    Ok(Flow::Done)
}

/// order:<dir>  walk and draw every picture a second time without
/// the Avatar: outside of his figure nothing may be different
fn cmd_order(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.smooth_camera = true;
    let dir = num(arg)? as u8 & 7;
    let (w, h) = (surf.width, surf.height);
    let mut bad = 0;
    for tick in 0..90 {
        game.step(Some(MoveInput { dir, run: false }));
        for sub in 0..3 {
            std::thread::sleep(Duration::from_millis(9));
            game.draw(surf, opts, false);
            let (with, o1) = (surf.pixels.clone(), game.view.origin);
            let at = game.shown_at(AVATAR).unwrap_or((w / 2, h / 2));
            let at = (at.0 + o1.0, at.1 + o1.1);
            opts.hide = AVATAR;
            game.draw(surf, opts, false);
            opts.hide = 0;
            let o2 = game.view.origin;
            let (mut n, mut bx0, mut by0, mut bx1, mut by1) = (0, w, h, 0, 0);
            for y in 16..h - 16 {
                for x in 16..w - 16 {
                    // world position on screen, the same in both pictures
                    let (wx, wy) = (x + o2.0, y + o2.1);
                    let (x1, y1) = (wx - o1.0, wy - o1.1);
                    if x1 < 0 || y1 < 0 || x1 >= w || y1 >= h || (x > w - 40 && y < 40) {
                        continue;
                    }
                    // the figure itself, with the weapon in his hand
                    if (wx - at.0).abs() < 36 && wy - at.1 > -80 && wy - at.1 < 16 {
                        continue;
                    }
                    if surf.pixels[(y * w + x) as usize] != with[(y1 * w + x1) as usize] {
                        n += 1;
                        (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                    }
                }
            }
            if n > 0 {
                bad += 1;
                let av = game.avatar();
                println!("tick {tick}.{sub}: {n} pixels differ in {bx0},{by0}..{bx1},{by1} (avatar {},{},{})", av.x, av.y, av.z);
                if bad == 1 {
                    let mut a = Surface::new(w, h);
                    a.pixels = with;
                    let _ = save_png(Path::new("order_a.png"), &a);
                    let _ = save_png(Path::new("order_b.png"), surf);
                }
            }
        }
    }
    println!("order: {bad} of 270 pictures with differences");
    game.options.smooth_camera = false;
    Ok(Flow::Done)
}

/// which animations a shape has: number(frames, reach of its attack frames)
fn cmd_anims(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let shape = num(arg)? as u32;
    let list: Vec<String> = (0..64)
        .filter_map(|n| game.data.anims.action(shape, n).map(|a| {
            let reach = (0..a.size).map(|f| a.frame(0, f).attack_range()).max().unwrap_or(0);
            format!("{n}({}{})", a.size, if reach > 0 { format!(",reach {reach}") } else { String::new() })
        }))
        .collect();
    println!("anims {shape}: {}", list.join(" "));
    Ok(Flow::Done)
}

fn cmd_hover(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (x, y) = point(arg)?;
    game.draw(surf, opts, false);
    game.set_hover(Some((x, y)));
    println!("hover: {:?} {:?}", game.ui.hover, game.ui.hover_name);
    game.draw(surf, opts, false);
    Ok(Flow::Done)
}

/// mapsheet:<png>[,<first>,<last>]  every map (at its densest spot), small, side by side and numbered
fn cmd_mapsheet(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<&str> = arg.split(',').map(str::trim).collect();
    let (first, last) = (v.get(1).map_or(Ok(0), |s| num(s))? as usize, v.get(2).map_or(Ok(255), |s| num(s))? as usize);
    let maps: Vec<usize> = (first..=last).filter(|&n| game.map_exists(n)).collect();
    const COLS: i32 = 6;
    let (tw, th) = (surf.width / 2, surf.height / 2);
    let rows = (maps.len() as i32 + COLS - 1) / COLS;
    let mut sheet = Surface::new(tw * COLS, th * rows.max(1));
    for (k, &n) in maps.iter().enumerate() {
        jump_to_map(game, n, None);
        game.draw(surf, opts, false);
        let (ox, oy) = ((k as i32 % COLS) * tw, (k as i32 / COLS) * th);
        for y in 0..th {
            for x in 0..tw {
                sheet.pixels[((oy + y) * tw * COLS + ox + x) as usize] = surf.pixels[((2 * y) * surf.width + 2 * x) as usize];
            }
        }
        if let Some(font) = &game.data.font {
            font.draw_at(&mut sheet, &format!("{n}"), ox + 4, oy + 4 + font.height_at(2.0), &game.data.pal, 2.0);
        }
    }
    save_png(Path::new(v[0]), &sheet).map_err(|e| e.to_string())?;
    println!("mapsheet: {} maps: {:?}", maps.len(), maps);
    Ok(Flow::Done)
}

/// names[:all|maps]  what hovering would show for every item of the map
/// (maps: the same on every map)
fn cmd_names(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    if arg == "maps" {
        let maps: Vec<usize> = (0..256).filter(|&n| game.map_exists(n)).collect();
        for n in maps {
            jump_to_map(game, n, None);
            print!("map {n}: ");
            run_commands(game, surf, opts, "names")?;
        }
        return Ok(Flow::Done);
    }
    let ids: Vec<u16> = game.map().items.iter().filter(|it| it.id != 0).map(|it| it.id).collect();
    let (globals, procs) = (game.scripts.usecode.globals().to_vec(), game.scripts.procs.len());
    let started = Instant::now();
    let (mut named, mut tried) = (0, 0);
    for id in ids {
        if !game.noteworthy(id) {
            continue;
        }
        tried += 1;
        let n = game.look_name(id);
        named += n.is_some() as usize;
        if arg == "all" {
            println!("name {id}: {}", n.map_or("-".to_string(), |s| format!("{s:?}")));
        }
    }
    let unchanged = globals == game.scripts.usecode.globals() && procs == game.scripts.procs.len();
    println!("names: {named} of {tried} named, {:.2} ms each, state unchanged: {unchanged}", started.elapsed().as_secs_f64() * 1000.0 / tried.max(1) as f64);
    Ok(Flow::Done)
}

/// namesize:<0-3>  the option "names on hover"
fn cmd_namesize(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.hover_names = (num(arg)? as usize).min(menu::NAME_SIZES.len() - 1);
    Ok(Flow::Done)
}

/// showall:<0|1>  as if the key H were held
fn cmd_showall(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.ui.show_all = num(arg)? != 0;
    game.draw(surf, opts, false);
    Ok(Flow::Done)
}

/// shapesheet:<shape>:<png>  all frames of a world shape, numbered
fn cmd_shapesheet(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (num_s, png) = arg.split_once(':').ok_or("shapesheet:<shape>:<png>")?;
    let shape = game.data.shapes.get(num(num_s)? as u32).ok_or("no such shape")?;
    surf.pixels.fill(0x404040);
    for (i, f) in shape.frames.iter().enumerate() {
        let (cx, cy) = (40 + (i as i32 % 10) * 60, 50 + (i as i32 / 10) * 60);
        surf.paint(f, cx, cy, false, false, &game.data.pal);
        if let Some(font) = &game.data.font {
            font.draw(surf, &i.to_string(), cx - 30, cy - 40, &game.data.pal);
        }
    }
    save_png(Path::new(png), surf).map_err(|e| e.to_string())?;
    Ok(Flow::Done)
}

/// gumpsheet:<shape>:<png>  all frames of a window graphic, at their size, numbered
fn cmd_gumpsheet(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let (num_s, png) = arg.split_once(':').ok_or("gumpsheet:<shape>:<png>")?;
    let shape = game.data.gump_shapes.get(num(num_s)? as u32).ok_or("no such shape")?;
    surf.pixels.fill(0x404040);
    let (mut x, mut y, mut row_h) = (4, 14, 0);
    for (i, f) in shape.frames.iter().enumerate() {
        if x + f.width > surf.width {
            (x, y, row_h) = (4, y + row_h + 14, 0);
        }
        surf.paint(f, x + f.xoff, y + f.yoff, false, false, &game.data.pal);
        if let Some(font) = &game.data.font {
            font.draw(surf, &i.to_string(), x, y - 12, &game.data.pal);
        }
        println!("frame {i}: {}x{} offset {},{}", f.width, f.height, f.xoff, f.yoff);
        x += f.width + 8;
        row_h = row_h.max(f.height);
    }
    save_png(Path::new(png), surf).map_err(|e| e.to_string())?;
    Ok(Flow::Done)
}

/// fontsheet:<png>  a line of text in each font of the game, numbered
fn cmd_fontsheet(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    surf.pixels.fill(0xE0C890);
    let mut y = 4;
    for (i, f) in game.data.fonts.iter().enumerate() {
        if let Some(f) = f {
            f.draw(surf, &format!("{i}: The Beginning... Day 3, 23:57"), 4, y, &game.data.pal);
            y += f.height + 4;
        }
    }
    save_png(Path::new(arg), surf).map_err(|e| e.to_string())?;
    Ok(Flow::Done)
}

/// colors:<shape>[,<shape>...]  how often each palette colour occurs in the shapes
fn cmd_colors(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let mut count = [0usize; 256];
    for n in arg.split(',') {
        let Some(shape) = game.data.shapes.get(num(n)? as u32) else { continue };
        for f in &shape.frames {
            for &p in &f.pixels {
                if p != shape::TRANSPARENT {
                    count[p as usize] += 1;
                }
            }
        }
    }
    for (i, &c) in count.iter().enumerate().filter(|(_, &c)| c > 0) {
        println!("colors: {i:3} #{:06X} {c}", game.data.pal.colors[i]);
    }
    Ok(Flow::Done)
}

/// heads   for every picture of the Avatar: where the helmet goes
///   (top left and size of the head found, or "none")
fn cmd_heads(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    game.note_fallen_frames();
    let shape = game.data.shapes.get(1).ok_or("no shape 1")?;
    // the animations each picture appears in
    let mut used_in: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for action in 0..256 {
        if let Some(a) = game.data.anims.action(1, action) {
            for dir in 0..8 {
                for n in 0..a.size {
                    let v = used_in.entry(a.frame(dir, n).frame).or_default();
                    if !v.contains(&action) {
                        v.push(action);
                    }
                }
            }
        }
    }
    for (i, f) in shape.frames.iter().enumerate().filter(|(_, f)| f.width > 0) {
        print!("anims {i}: {:?}  ", used_in.get(&(i as u32)).cloned().unwrap_or_default());
        match outfit::head_pixels(f, i as u32) {
            Some(h) => {
                let w = f.width as usize;
                let (x0, y0) = (h.iter().map(|p| p % w).min().unwrap_or(0), h.iter().map(|p| p / w).min().unwrap_or(0));
                println!("heads {i}: at {x0},{y0} pixels {}", h.len());
            }
            None => println!("heads {i}: none"),
        }
    }
    Ok(Flow::Done)
}

/// sheet:<png>,<shape>[,<first frame>[,<count>]]  frames of a shape side by side, enlarged
fn cmd_sheet(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<&str> = arg.split(',').collect();
    let shape_no = num(v.get(1).ok_or("sheet:<png>,<shape>")?)? as u32;
    let first = v.get(2).map(|s| num(s)).transpose()?.unwrap_or(0) as usize;
    let count = v.get(3).map(|s| num(s)).transpose()?.unwrap_or(1000) as usize;
    game.draw(surf, opts, false); // the Avatar dressed as now
    let shape = game.data.shapes.get(shape_no).ok_or("no such shape")?;
    // PAGAN_HEADS: the Avatar's head as found for the helmet, in magenta
    let heads = std::env::var_os("PAGAN_HEADS").is_some();
    let frames: Vec<(usize, &shape::Frame)> = shape.frames.iter().enumerate().skip(first).take(count).filter(|(_, f)| f.width > 0).collect();
    let cell_w = frames.iter().map(|(_, f)| f.width).max().unwrap_or(1) + 6;
    let cell_h = frames.iter().map(|(_, f)| f.height).max().unwrap_or(1) + 14;
    let cols = (frames.len() as i32).clamp(1, 12);
    let rows = (frames.len() as i32 + cols - 1) / cols;
    let mut sheet = Surface::new(cols * cell_w, rows.max(1) * cell_h);
    sheet.pixels.fill(0x505050);
    for (k, (i, f)) in frames.iter().enumerate() {
        let (cx, cy) = ((k as i32 % cols) * cell_w + 3, (k as i32 / cols) * cell_h + 12);
        sheet.paint(f, cx + f.xoff, cy + f.yoff, false, false, &game.data.pal);
        if heads {
            for p in outfit::head_pixels(f, *i as u32).unwrap_or_default() {
                let (x, y) = (cx + (p as i32 % f.width), cy + (p as i32 / f.width));
                sheet.pixels[(y * sheet.width + x) as usize] = 0xFF00FF;
            }
        }
        if let Some(font) = &game.data.font {
            font.draw(&mut sheet, &i.to_string(), cx, cy - 11, &game.data.pal);
        }
    }
    let mut big = Surface::new(sheet.width * 3, sheet.height * 3);
    for (i, p) in big.pixels.iter_mut().enumerate() {
        let (x, y) = (i as i32 % big.width / 3, i as i32 / big.width / 3);
        *p = sheet.pixels[(y * sheet.width + x) as usize];
    }
    save_png(Path::new(v[0]), &big).map_err(|e| e.to_string())?;
    Ok(Flow::Done)
}

/// mousesheet:<png>  all pictures of the mouse pointer, numbered
fn cmd_mousesheet(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, _cmd: &str) -> Result<Flow, String> {
    let d = std::fs::read(game.data.static_dir.join("U8MOUSE.SHP")).map_err(|e| e.to_string())?;
    let shape = shape::decode_shape(&d).ok_or("cannot decode U8MOUSE.SHP")?;
    surf.pixels.fill(0x404040);
    for (i, f) in shape.frames.iter().enumerate() {
        let (cx, cy) = (40 + (i as i32 % 10) * 60, 40 + (i as i32 / 10) * 60);
        surf.paint(f, cx + f.xoff, cy + f.yoff, false, false, &game.data.pal);
        if let Some(font) = &game.data.font {
            font.draw(surf, &i.to_string(), cx - 30, cy - 30, &game.data.pal);
        }
        println!("frame {i}: {}x{} hotspot {},{}", f.width, f.height, f.xoff, f.yoff);
    }
    // mousesheet:<png>  written right away (the screenshot shows the world)
    if !arg.is_empty() {
        save_png(Path::new(arg), surf).map_err(|e| e.to_string())?;
    }
    Ok(Flow::Done)
}

/// pointer:<x>,<y>  which mouse pointer shows there (pointer:<x>,<y>,<png> also draws it)
fn cmd_pointer(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<&str> = arg.splitn(3, ',').collect();
    if v.len() < 2 {
        return Err(format!("'{cmd}' expects x,y[,png]"));
    }
    let at = (num(v[0])?, num(v[1])?);
    game.draw(surf, opts, false);
    let frame = game.cursor_frame(at, cursor::PointerState::default());
    println!("{cmd}: frame {frame:?}");
    if let (Some(png), Some(frame)) = (v.get(2), frame) {
        let c = cursor::Cursor::load(&game.data.static_dir);
        c.draw(surf, frame, at, game);
        save_png(Path::new(png), &zoomed(surf, 2)).map_err(|e| e.to_string())?;
    }
    Ok(Flow::Done)
}

/// turnlive:<+1|-1>  turn the view as the keys do, drawing as the window would;
/// reports the angle drawn every 50 ms
fn cmd_turnlive(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.turn_view = true;
    game.turn_view(num(arg)?);
    let started = std::time::Instant::now();
    let mut seen = Vec::new();
    while started.elapsed().as_millis() < 700 {
        game.draw(surf, opts, false);
        seen.push(format!("{:.0}", game.view.xf.angle));
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("{cmd}: {}", seen.join(" "));
    Ok(Flow::Done)
}

/// shimmer:<steps>  move the Avatar by 1 world unit at a time and count, per
/// picture, the pixels of the landscape that are not just shifted along (flicker)
fn cmd_shimmer(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let av = game.avatar();
    let mut worst = 0.0f32;
    let mut sum = 0.0f32;
    let steps = num(arg)?.max(1);
    game.options.smooth_camera = false;
    game.draw(surf, opts, false);
    let mut prev = surf.pixels.clone();
    for i in 1..=steps {
        game.teleport(AVATAR, game.cur_map, (av.x + i, av.y, av.z));
        game.draw(surf, opts, false);
        // the best whole-pixel shift between the two pictures, judged in the middle part
        let (w, h) = (surf.width, surf.height);
        let mut best = f32::MAX;
        for dy in -2..=2 {
            for dx in -2..=2 {
                best = best.min(mismatch(&surf.pixels, &prev, (w, h), (dx, dy)));
            }
        }
        worst = worst.max(best);
        sum += best;
        prev = surf.pixels.clone();
    }
    println!("{cmd}: changed pixels per picture {:.1} % on average, {:.1} % at most", 100.0 * sum / steps as f32, 100.0 * worst);
    Ok(Flow::Done)
}

/// zoom:<percent>  the view drawn at that size of the original (as + and - do)
fn cmd_zoom(game: &mut Game, surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let pct = num(arg)?.clamp(25, 400);
    *surf = Surface::new(640 * pct / 100, 400 * pct / 100);
    game.view.size = (surf.width, surf.height);
    game.view.zoom = pct as f32 / 100.0;
    Ok(Flow::Done)
}

/// textsize:<0-3>  option text size: original, 100 %, 150 %, 200 %
fn cmd_textsize(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.text_size = (num(arg)? as usize).min(menu::TEXT_SIZES.len() - 1);
    Ok(Flow::Done)
}

/// angle:<degrees>  the view held at any angle (as in the middle of a turn)
fn cmd_angle(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    game.options.turn_view = true;
    game.turn.fixed = Some(arg.trim().parse::<f32>().map_err(|_| format!("bad number in '{cmd}'"))?);
    Ok(Flow::Done)
}

/// modelsheet:<png>,<shape>,<frame>[,<flipped 0|1>]  one thing alone, enlarged: the original
/// picture, then the turnable view at 0, 90, 180 and 270 degrees
fn cmd_modelsheet(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    let v: Vec<&str> = arg.split(',').map(str::trim).collect();
    if v.len() < 3 {
        return Err("modelsheet:<png>,<shape>,<frame>[,<flipped>]".into());
    }
    let (shape, frame) = (num(v[1])? as u32, num(v[2])? as u32);
    let flipped = v.get(3) == Some(&"1");
    let it = crate::map::Item { x: 4096, y: 4096, z: 0, shape, frame, flags: if flipped { crate::map::FLG_FLIPPED } else { 0 }, id: 9999, ..Default::default() };
    let items = [it];
    let Game { turn, data, .. } = game;
    let info = data.infos.get(shape as usize).copied().ok_or("no such shape")?;
    let (xd, yd, zd) = info.footpad(flipped);
    let cam = (it.x - xd / 2, it.y - yd / 2, zd / 2);
    const SIZE: i32 = 240;
    let mut pics = Vec::new();
    let mut one = Surface::new(SIZE, SIZE);
    let mut ids = Vec::new();
    let vo = ViewOptions { show_editor_items: true, ..ViewOptions::default() };
    crate::view::render(&mut one, &crate::view::Camera { x: cam.0, y: cam.1, z: cam.2 }, &items, &data.infos, &mut data.shapes, &data.pal, &vo, None, &mut ids);
    pics.push(one.pixels.clone());
    let person = |it: &crate::map::Item, _q: i32| (it.frame, it.flags & crate::map::FLG_FLIPPED != 0);
    for angle in [0.01f32, 90.0, 180.0, 270.0] {
        let xf = crate::turnview::Xform { cam, angle, size: (SIZE, SIZE) };
        let scene = crate::turnview::Scene { items: &items, infos: &data.infos, pal: &data.pal, z_limit: None, show_editor_items: true, hide: 0, person_frame: &person, overlay: None };
        turn.forget_models();
        turn.render(&mut one, &xf, &scene, &mut data.shapes);
        pics.push(one.pixels.clone());
    }
    const K: i32 = 3;
    let mut sheet = Surface::new(SIZE * 5 * K, SIZE * K);
    for (n, pic) in pics.iter().enumerate() {
        for y in 0..SIZE * K {
            for x in 0..SIZE * K {
                let c = pic[((y / K) * SIZE + x / K) as usize];
                sheet.pixels[(y * SIZE * 5 * K + n as i32 * SIZE * K + x) as usize] = if c == 0 { 0x404048 } else { c };
            }
        }
    }
    save_png(Path::new(v[0]), &sheet).map_err(|e| e.to_string())?;
    println!("modelsheet: shape {shape} frame {frame} box {xd}x{yd}x{zd} saved {}", v[0]);
    Ok(Flow::Done)
}

/// directions  the shapes whose frames are one thing seen from eight directions (they turn
/// with the turnable view instead of being mirrored)
fn cmd_directions(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let n = game.data.infos.len() as u32;
    let found: Vec<String> = (0..n).filter(|&s| game.turn.turns_with_view(&mut game.data.shapes, s)).map(|s| s.to_string()).collect();
    println!("directions: {} shapes: {}", found.len(), found.join(" "));
    Ok(Flow::Done)
}

/// turn:<quarters>  the view turned to another side at once (0 = as in the original)
fn cmd_turn(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.turn_view = true;
    game.turn.reset();
    game.turn.target = num(arg)?.rem_euclid(4);
    Ok(Flow::Done)
}

/// light:<0|1>  lighting off / on (for this run only)
fn cmd_light(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    game.options.lighting = num(arg)? != 0;
    Ok(Flow::Done)
}

/// mouse:<x>,<y>  where the pointer is (for what follows it)
fn cmd_mouse(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    game.ui.pointer = Some(point(arg)?);
    Ok(Flow::Done)
}

/// sunlitscan  on every map under the open sky: how many spots with ground have how many void directions (3 and more: no sun)
fn cmd_sunlitscan(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    for n in [3, 4, 5, 6, 7, 8, 12, 29, 37, 39, 40, 41, 56, 200, 223] {
        if !game.map_exists(n) {
            continue;
        }
        jump_to_map(game, n, None);
        let map = game.map();
        let mut hist = [0usize; 9];
        let mut odd = Vec::new();
        let mut stars = 0;
        for gx in 0..256 {
            for gy in 0..256 {
                let (x, y) = (gx * 256, gy * 256);
                let ground = map.items_near(x, y, 64).iter().any(|&i| { let it = &map.items[i]; it.in_world() && it.shape != 564 && game.data.infos.get(it.shape as usize).is_some_and(|si| si.land) });
                if ground {
                    let v = crate::shadows::void_directions(map, x, y);
                    stars += crate::shadows::sees_stars(map, x, y) as usize;
                    hist[v] += 1;
                    if v >= 3 && odd.len() < 4 {
                        odd.push(format!("{x},{y}"));
                    }
                }
            }
        }
        println!("sunlitscan map {n}: {hist:?} no sun at {odd:?}, near the stars {stars}");
    }
    Ok(Flow::Done)
}

/// sunlit  whether the Avatar stands where shadows fall (and how many directions are void)
fn cmd_sunlit(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, _arg: &str, _cmd: &str) -> Result<Flow, String> {
    let a = game.map().get(1).copied().ok_or("no avatar")?;
    let (m, map) = (game.cur_map, game.map());
    println!("sunlit: {} (void directions {})", crate::shadows::sunlit(m, map, a.x, a.y), crate::shadows::void_directions(map, a.x, a.y));
    for far in [256, 512, 768, 1024, 1536, 2048] {
        let counts: Vec<String> = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)].iter().map(|&(dx, dy): &(i32, i32)| {
            let near = map.items_near(a.x + dx * far, a.y + dy * far, 128);
            let fixed = near.iter().filter(|&&i| map.items[i].in_world() && game.data.infos.get(map.items[i].shape as usize).is_some_and(|si| si.land)).count();
            let shapes: Vec<String> = near.iter().take(3).map(|&i| format!("{}:{}@{}", map.items[i].shape, map.items[i].frame, map.items[i].z)).collect();
            format!("{}/{}[{}]", near.len(), fixed, shapes.join(","))
        }).collect();
        println!("sunlit far {far}: {}", counts.join(" "));
    }
    Ok(Flow::Done)
}

/// animframes:<shape>,<action>  the first frame of an animation per direction
fn cmd_animframes(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, _name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let point = |s: &str| point(s, cmd);
    let (s, a) = point(arg)?;
    match game.data.anims.action(s as u32, a as u32) {
        Some(act) => {
            let firsts: Vec<String> = (0..8u8).map(|d| format!("{d}:{}", act.frame(d, 0).frame)).collect();
            println!("animframes {s},{a}: size {} repeat {} two_step {} {}", act.size, act.frame_repeat, act.two_step(), firsts.join(" "));
            // direction 0 in full: frame/delta along/delta z/on ground
            let all: Vec<String> = (0..act.size).map(|n| { let f = act.frame(0, n); format!("{}/{}/{}{}", f.frame, f.delta_dir, f.delta_z, if f.on_ground() { "g" } else { "" }) }).collect();
            println!("animframes {s},{a} dir 0: {}", all.join(" "));
        }
        None => println!("animframes {s},{a}: none"),
    }
    Ok(Flow::Done)
}

/// The share of pixels in the middle part of picture `a` (`w` x `h`) that
/// differ from picture `b` shifted by `dx`, `dy`.
fn mismatch(a: &[u32], b: &[u32], (w, h): (i32, i32), (dx, dy): (i32, i32)) -> f32 {
    let (mut bad, mut all) = (0, 0);
    for y in (h / 4..3 * h / 4).step_by(2) {
        for x in (w / 4..3 * w / 4).step_by(2) {
            all += 1;
            if a[(y * w + x) as usize] != b[((y + dy) * w + x + dx) as usize] {
                bad += 1;
            }
        }
    }
    bad as f32 / all as f32
}
