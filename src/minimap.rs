//! The minimap (optional): a small map of the surroundings in the top
//! right corner, seen from above but turned like the view, so that up on
//! it is up on the screen. Every spot shows the colour of the highest
//! thing there; walls cast a little shadow. Under a roof the roofs are left
//! out, as in the view. The Avatar is the yellow dot in the middle, enemies
//! fighting him are red, other people light.

use crate::game::{Game, AVATAR};
use crate::hash::HashMap;
use crate::map::{MapData, FLG_INVISIBLE};
use crate::palette::Palette;
use crate::shape::{ShapeInfo, Shapes, TRANSPARENT};
use crate::view::Surface;

/// Size of the map on the screen.
pub const WIDTH: i32 = 112;
pub const HEIGHT: i32 = 80;
/// How much smaller than the view: one pixel across is 4 * ZOOM world units.
const ZOOM: i32 = 12;
/// A spot of the map: 32 world units square (a tile's footpad unit).
const CELL: i32 = 32;
/// The map is made in chunks of 16 x 16 spots.
const CHUNK: i32 = 512;
const SPOTS: usize = (CHUNK / CELL) as usize;
/// Chunks older than this (in pictures) are made again, a few per picture,
/// so that opened doors and moved things show.
const STALE: u32 = 90;
const REFRESH_PER_PICTURE: usize = 3;
const EMPTY: i32 = i32::MIN;

struct Chunk {
    /// Height of the top of the highest thing on each spot (EMPTY: nothing).
    top: [i32; SPOTS * SPOTS],
    colour: [u32; SPOTS * SPOTS],
    made: u32,
}

#[derive(Default)]
pub struct Minimap {
    chunks: HashMap<(i32, i32), Chunk>,
    /// Average colour of a picture (shape, frame); None: nothing to see.
    colours: HashMap<(u32, u32), Option<u32>>,
    /// The map and roof height the chunks were made for.
    made_for: Option<(usize, Option<i32>)>,
    clock: u32,
}

/// The average colour of a picture, without its black outline.
fn frame_colour(shapes: &mut Shapes, pal: &Palette, shape: u32, frame: u32) -> Option<u32> {
    let s = shapes.get(shape)?;
    let f = s.frames.get(frame as usize)?;
    let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
    for &p in &f.pixels {
        if p == TRANSPARENT || p == 0 {
            continue;
        }
        let c = pal.base_color(p as usize);
        r += c >> 16 & 0xFF;
        g += c >> 8 & 0xFF;
        b += c & 0xFF;
        n += 1;
    }
    (n > 0).then(|| (r / n) << 16 | (g / n) << 8 | (b / n))
}

/// Is the picture all black (only black and see-through pixels)? Such
/// ground is the void below the dungeons (guide.rs).
pub fn all_black(shapes: &mut Shapes, pal: &Palette, shape: u32, frame: u32) -> bool {
    shapes.get(shape).is_some_and(|s| s.frames.get(frame as usize).is_some()) && frame_colour(shapes, pal, shape, frame).is_none()
}

/// Does the thing belong on the map? The ground, buildings and other fixed
/// or solid things; not what lies about, not people.
fn on_map(info: &ShapeInfo) -> bool {
    !info.editor && (info.fixed || info.land || info.solid || info.roof)
}

impl Minimap {
    fn make_chunk(
        &mut self,
        (cx, cy): (i32, i32),
        map: &MapData,
        infos: &[ShapeInfo],
        shapes: &mut Shapes,
        pal: &Palette,
        z_limit: Option<i32>,
    ) -> Chunk {
        let mut c = Chunk { top: [EMPTY; SPOTS * SPOTS], colour: [0; SPOTS * SPOTS], made: self.clock };
        let (x0, y0) = (cx * CHUNK, cy * CHUNK);
        // things reach from their origin up to 16 tiles towards smaller x, y
        for i in map.items_near(x0 + CHUNK, y0 + CHUNK, CHUNK) {
            let it = &map.items[i];
            if it.is_actor() || it.flags & FLG_INVISIBLE != 0 {
                continue;
            }
            let Some(info) = infos.get(it.shape as usize) else { continue };
            if !on_map(info) || z_limit.is_some_and(|limit| it.z >= limit && !info.draw) {
                continue;
            }
            let (xd, yd, zd) = info.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
            let top = it.z + zd;
            let span = |from: i32, to: i32, base: i32| {
                let a = ((from - base).div_euclid(CELL)).max(0);
                let b = ((to - 1 - base).div_euclid(CELL)).min(SPOTS as i32 - 1);
                a..=b
            };
            let (xs, ys) = (span(it.x - xd.max(1), it.x, x0), span(it.y - yd.max(1), it.y, y0));
            if xs.is_empty() || ys.is_empty() {
                continue;
            }
            let colour = match self.colours.get(&(it.shape, it.frame)) {
                Some(c) => *c,
                None => {
                    let c = frame_colour(shapes, pal, it.shape, it.frame);
                    self.colours.insert((it.shape, it.frame), c);
                    c
                }
            };
            let Some(colour) = colour else { continue };
            for gy in ys {
                for gx in xs.clone() {
                    let k = gy as usize * SPOTS + gx as usize;
                    if top >= c.top[k] {
                        c.top[k] = top;
                        c.colour[k] = colour;
                    }
                }
            }
        }
        c
    }

    /// For the large map (worldmap.rs): makes the missing parts between
    /// two world points (the nearest to `middle` first), for at most
    /// `budget`. Returns whether all of them are there. Roofs are shown.
    pub fn make_area(&mut self, map_no: usize, map: &MapData, infos: &[ShapeInfo], shapes: &mut Shapes, pal: &Palette,
                     lo: (i32, i32), hi: (i32, i32), middle: (i32, i32), budget: std::time::Duration) -> bool {
        if self.made_for != Some((map_no, None)) {
            self.chunks.clear();
            self.made_for = Some((map_no, None));
        }
        let start = std::time::Instant::now();
        // the maps reach from 0 to 32767
        let range = |a: i32, b: i32| (a.div_euclid(CHUNK) - 1).max(0)..=(b.div_euclid(CHUNK) + 1).min(32768 / CHUNK - 1);
        let mut missing: Vec<(i32, i32)> = Vec::new();
        for cy in range(lo.1, hi.1) {
            for cx in range(lo.0, hi.0) {
                if !self.chunks.contains_key(&(cx, cy)) {
                    missing.push((cx, cy));
                }
            }
        }
        let (mx, my) = (middle.0.div_euclid(CHUNK), middle.1.div_euclid(CHUNK));
        missing.sort_unstable_by_key(|&(cx, cy)| (cx - mx).abs().max((cy - my).abs()));
        for (k, key) in missing.iter().enumerate() {
            if start.elapsed() > budget {
                return k == missing.len();
            }
            let chunk = self.make_chunk(*key, map, infos, shapes, pal, None);
            self.chunks.insert(*key, chunk);
        }
        true
    }

    /// For the large map: the top of the highest thing at a world point
    /// and its colour (None: nothing there, or not made yet).
    pub fn top_at(&self, x: i32, y: i32) -> Option<(i32, u32)> {
        let (top, colour) = self.spot(x, y);
        (top != EMPTY).then_some((top, colour))
    }

    /// The highest thing at a world point: (height of its top, colour).
    fn spot(&self, x: i32, y: i32) -> (i32, u32) {
        let Some(c) = self.chunks.get(&(x.div_euclid(CHUNK), y.div_euclid(CHUNK))) else { return (EMPTY, 0) };
        let (gx, gy) = (x.rem_euclid(CHUNK) / CELL, y.rem_euclid(CHUNK) / CELL);
        let k = (gy * SPOTS as i32 + gx) as usize;
        (c.top[k], c.colour[k])
    }
}

/// Scales a colour by `f`.
pub fn shade(c: u32, f: f32) -> u32 {
    let ch = |v: u32| ((v as f32 * f).round() as i32).clamp(0, 255) as u32;
    ch(c >> 16 & 0xFF) << 16 | ch(c >> 8 & 0xFF) << 8 | ch(c & 0xFF)
}

impl Game {
    /// Is a view position on the minimap? (Clicks there do not reach the world.)
    pub fn on_minimap(&self, x: i32, y: i32) -> bool {
        self.ui.minimap_area.is_some_and(|(x0, y0, x1, y1)| x >= x0 && y >= y0 && x < x1 && y < y1)
    }

    /// Draws the minimap in the top right corner, below the health and
    /// mana. `angle`: how far the view is turned; `z_limit`: the roof above
    /// the Avatar.
    pub fn draw_minimap(&mut self, surf: &mut Surface, angle: f32, z_limit: Option<i32>) {
        let below = self.data.gump_shapes.get(33).and_then(|s| s.frames.first().map(|f| f.height)).unwrap_or(26);
        let (x0, y0) = (surf.width - WIDTH - 6, 6 + below + 4);
        if x0 < 0 || y0 + HEIGHT > surf.height {
            return;
        }
        self.ui.minimap_area = Some((x0 - 3, y0 - 3, x0 + WIDTH + 3, y0 + HEIGHT + 3));
        let av = self.avatar();
        let (s, c) = angle.to_radians().sin_cos();
        // map point (relative to the middle) to world point and back
        let to_world = |du: i32, dv: i32| -> (i32, i32) {
            let (a, b) = ((du * 4 * ZOOM) as f32, (dv * 8 * ZOOM) as f32);
            let (rx, ry) = ((a + b) / 2.0, (b - a) / 2.0);
            (av.x + (c * rx + s * ry).round() as i32, av.y + (-s * rx + c * ry).round() as i32)
        };
        let to_map = |x: i32, y: i32| -> (i32, i32) {
            let (dx, dy) = ((x - av.x) as f32, (y - av.y) as f32);
            let (rx, ry) = (c * dx - s * dy, s * dx + c * dy);
            (((rx - ry) / (4 * ZOOM) as f32).round() as i32, ((rx + ry) / (8 * ZOOM) as f32).round() as i32)
        };
        let (hw, hh) = (WIDTH / 2, HEIGHT / 2);

        // the chunks under the map: made if missing, the oldest made again
        let Game { data: crate::gamedata::GameData { infos, shapes, pal, .. }, minimap: mm, maps, cur_map, .. } = self;
        let map = &maps[cur_map];
        if mm.made_for != Some((*cur_map, z_limit)) {
            mm.chunks.clear();
            mm.made_for = Some((*cur_map, z_limit));
        }
        mm.clock += 1;
        let corners = [to_world(-hw, -hh), to_world(hw, -hh), to_world(-hw, hh), to_world(hw, hh)];
        let lo = |f: fn(&(i32, i32)) -> i32| corners.iter().map(f).min().unwrap_or(0).div_euclid(CHUNK) - 1;
        let hi = |f: fn(&(i32, i32)) -> i32| corners.iter().map(f).max().unwrap_or(0).div_euclid(CHUNK) + 1;
        let (cx0, cx1, cy0, cy1) = (lo(|p| p.0), hi(|p| p.0), lo(|p| p.1), hi(|p| p.1));
        let mut stale = Vec::new();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                match mm.chunks.get(&(cx, cy)) {
                    None => {
                        let chunk = mm.make_chunk((cx, cy), map, infos, shapes, pal, z_limit);
                        mm.chunks.insert((cx, cy), chunk);
                    }
                    Some(ch) if mm.clock - ch.made > STALE => stale.push((ch.made, (cx, cy))),
                    _ => {}
                }
            }
        }
        stale.sort_unstable();
        for (_, key) in stale.into_iter().take(REFRESH_PER_PICTURE) {
            let chunk = mm.make_chunk(key, map, infos, shapes, pal, z_limit);
            mm.chunks.insert(key, chunk);
        }
        // forget what is far away
        mm.chunks.retain(|&(cx, cy), _| cx >= cx0 - 4 && cx <= cx1 + 4 && cy >= cy0 - 4 && cy <= cy1 + 4);

        // the frame, in the gold of the menu
        for y in -3..HEIGHT + 3 {
            for x in -3..WIDTH + 3 {
                let ring = (-x - 1).max(x - WIDTH).max(-y - 1).max(y - HEIGHT);
                let c = match ring {
                    2 => 0x301800,
                    1 => 0xE0A830,
                    0 => 0x805010,
                    _ => continue,
                };
                surf.put(x0 + x, y0 + y, c);
            }
        }
        // the land
        for py in 0..HEIGHT {
            for px in 0..WIDTH {
                let (wx, wy) = to_world(px - hw, py - hh);
                let (top, colour) = mm.spot(wx, wy);
                let c = if top == EMPTY {
                    0x0C0804
                } else {
                    // higher lighter, lower darker; in the shadow of a wall darker
                    let mut f = 1.0 + ((top - av.z) as f32 / 200.0).clamp(-0.45, 0.3);
                    let (behind, _) = mm.spot(wx - CELL, wy - CELL);
                    if behind != EMPTY && behind > top + 16 {
                        f *= 0.6;
                    }
                    shade(colour, f)
                };
                surf.put(x0 + px, y0 + py, c);
            }
        }

        // people
        let near: Vec<(i32, i32, u32)> = map
            .items_near(av.x, av.y, 6000)
            .into_iter()
            .filter_map(|i| {
                let it = &map.items[i];
                if !it.is_actor() || it.id == AVATAR || !it.in_world() || it.flags & FLG_INVISIBLE != 0 {
                    return None;
                }
                if z_limit.is_some_and(|limit| it.z >= limit) {
                    return None; // upstairs, under the roof hidden from view
                }
                let state = self.fights.states.get(&it.id);
                if state.is_some_and(|s| s.dead) || it.dead {
                    return None;
                }
                let foe = state.is_some_and(|s| s.in_combat && s.target == AVATAR);
                let (u, v) = to_map(it.x, it.y);
                Some((u, v, if foe { 0xF03020 } else { 0xD8D8C8 }))
            })
            .collect();
        for (u, v, colour) in near {
            if u.abs() >= hw - 1 || v.abs() >= hh - 1 {
                continue;
            }
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                surf.put(x0 + hw + u + dx, y0 + hh + v + dy, colour);
            }
        }
        // the Avatar: a yellow dot with a dark rim
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let c = if dx.abs() == 2 || dy.abs() == 2 { 0x201000 } else { 0xFFE040 };
                if dx.abs() == 2 && dy.abs() == 2 {
                    continue;
                }
                surf.put(x0 + hw + dx, y0 + hh + dy, c);
            }
        }
    }
}
