//! Lighting (optional): the general light follows the time of day under
//! the open sky (as in the original by day, warm at dawn and dusk, dark and
//! bluish at night) and is dimmer in caves and cellars. Around lamps,
//! fires, candles, lava and fiery spells it is lit up in their colour, the
//! more, the darker it is; lava glows slowly up and down.
//!
//! Inside houses (while the Avatar is in one, its roof hidden) the rooms
//! have a light of their own: a little dimmer
//! than outside by day, and at night warmer and lighter than the dark
//! outside, as if a fire or a lamp burnt somewhere (many houses have none).
//! What is seen of the outside keeps the light of outside.
//!
//! Lights inside a house (under a roof) light only what is inside, and
//! lights outside only what is outside: a candle does not shine through
//! the wall onto the street, a lamp on the wall not into the room. Walls
//! themselves are lit from both sides (they belong to both).
//!
//! A light only falls on what is about as high as it, not on a floor or
//! roof above it (the lamps in a house would light its roof). This is done
//! on the finished picture of the world, so it works with the turned view
//! as well (there without the heights).

use crate::game::Game;
use crate::hash::HashMap;
use crate::view::Surface;

/// The general light under the open sky through the day, per colour
/// (1.0 = as in the original): (hour, red, green, blue), from midnight on.
const SKY: [(f32, [f32; 3]); 8] = [
    (0.0, NIGHT),
    (5.0, NIGHT),
    (6.5, [0.88, 0.78, 0.72]),
    (8.0, [1.0; 3]),
    (17.0, [1.0; 3]),
    (19.0, [0.92, 0.74, 0.62]),
    (21.0, NIGHT),
    (24.0, NIGHT),
];
const NIGHT: [f32; 3] = [0.36, 0.44, 0.68];
/// The general light in caves, cellars and dungeons (no sun, no night).
const UNDERGROUND: [f32; 3] = [0.76, 0.75, 0.8];
/// How much a light also adds of its colour, on top of brightening what it
/// falls on: dark stone near lava turns orange, not just a lighter grey.
const TINT: f32 = 0.12;

/// The general light in rooms under a roof, by day and at night.
const INDOOR_DAY: [f32; 3] = [0.84, 0.8, 0.74];
const INDOOR_NIGHT: [f32; 3] = [0.6, 0.53, 0.46];

/// The general light in a room under a roof, when it is `outside` outside:
/// between the room's light by day and at night, as far as the day is.
pub fn indoor(outside: [f32; 3]) -> [f32; 3] {
    let luma = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let day = ((luma(outside) - luma(NIGHT)) / (1.0 - luma(NIGHT))).clamp(0.0, 1.0);
    [0, 1, 2].map(|c| INDOOR_NIGHT[c] + (INDOOR_DAY[c] - INDOOR_NIGHT[c]) * day)
}

/// The general light where no lamp shines: under the open sky (`sky`) by
/// the time of day (`hour`, 0.0 .. 24.0), otherwise as underground.
pub fn ambient(hour: f32, sky: bool) -> [f32; 3] {
    if !sky {
        return UNDERGROUND;
    }
    let h = hour.rem_euclid(24.0);
    let i = SKY.iter().rposition(|&(at, _)| at <= h).unwrap_or(0).min(SKY.len() - 2);
    let ((h0, a), (h1, b)) = (SKY[i], SKY[i + 1]);
    let t = ((h - h0) / (h1 - h0)).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t)
}
/// How a picture is lit (see the top of this file).
#[derive(Clone, Copy)]
pub struct Look {
    /// The general light per colour (1.0 = as in the original).
    pub ambient: [f32; 3],
    /// How strongly the lights brighten.
    lamp: f32,
}

impl Look {
    /// With the general light `ambient`.
    pub fn by_day(ambient: [f32; 3]) -> Look {
        // lamps show the more, the darker it is around them (by day hardly)
        let dark = (1.0 - (0.3 * ambient[0] + 0.59 * ambient[1] + 0.11 * ambient[2])).max(0.0);
        Look { ambient, lamp: 0.25 + 1.4 * dark }
    }
}

/// Lights reach this much further than their radius in `light_of`.
const FURTHER: f32 = 1.35;
/// A light falls on what reaches at least up to this much below it (a lamp
/// on an upper floor does not light the room under it).
const BELOW: i32 = 104;
/// Flat lights (lava, a fire field) light what stands up to this high
/// above their ground: the rock beside lava is often a little higher.
const FLAT_REACH: i32 = 40;
/// Fires light what stands up to this high above their ground.
const FIRE_REACH: i32 = 48;

/// Height of a lamp post below its lamp.
const POST: i32 = 80;
/// How much brighter than the original a lit spot may get.
const MOST: f32 = 1.3;
/// The most light that lights add together (in the strongest colour).
const ADDED: f32 = 1.0;

/// A light on screen.
#[derive(Clone, Copy)]
pub struct Light {
    pub x: f32,
    pub y: f32,
    /// Reach in pixels (sideways; half as far up and down, as the ground
    /// is seen at a slant).
    pub radius: f32,
    /// Colour and strength.
    pub rgb: [f32; 3],
    /// Height in the world: of its middle, and of its top (what stands on
    /// that height or higher is above the light: a ceiling, a roof).
    pub z: i32,
    pub top: i32,
    /// The thing giving the light (index into the items).
    pub item: usize,
    /// Is it inside a house (under a roof)? None: not known.
    pub inside: Option<bool>,
}

/// What kind of light a thing gives: (radius, colour, flickers).
fn light_of(shape: u32, frame: u32) -> Option<(f32, [f32; 3], bool)> {
    const LAMP: [f32; 3] = [1.5, 1.25, 0.8];
    const FIRE: [f32; 3] = [1.1, 0.6, 0.25];
    const CANDLE: [f32; 3] = [1.0, 0.8, 0.45];
    // (lava lies in fields of dozens of pieces: each one gives little)
    const LAVA: [f32; 3] = [0.5, 0.18, 0.05];
    Some(match shape {
        // lamp posts (the lamp at the top), wall lamps that burn, the lamp at the end
        690 | 691 => (90.0, LAMP, false),
        // a little tower on a roof with a lit window
        661 => (70.0, CANDLE, false),
        314 | 317 | 640 => (95.0, LAMP, false),
        216 => (60.0, LAMP, false),
        // camp fire, fireplaces, fire pit, fire field
        267 => (140.0, FIRE, true),
        276 | 277 => (120.0, FIRE, true),
        318 => (150.0, FIRE, true),
        400 => (110.0, FIRE, true),
        // a bonfire that burns down over its frames (0 high .. 10 embers)
        381 => (140.0 * (1.0 - frame.min(10) as f32 / 12.0), FIRE, true),
        // a burning torch, jets of flame, small flames, embers
        840 => (80.0, FIRE, true),
        166 | 265 => (80.0, FIRE, true),
        230 | 569 => (55.0, CANDLE, true),
        786 | 804 => (45.0, FIRE, true),
        // spells: explosions, a fire storm, balls and bolts of fire
        // (260 the fireball that chases the Avatar, 261 a flame bolt)
        // (480 and 483 also burst out of lava all the time: not too far)
        480 | 483 | 660 => (100.0, FIRE, true),
        110 => (180.0, FIRE, true),
        260 | 261 => (90.0, FIRE, true),
        // explosions: a big one (0-19), a small one (20-34), the burst of a
        // flame bolt (40-65): flaring up and dying down over their frames
        578 => {
            let (first, last, peak, radius) = match frame {
                0..=19 => (0, 19, 2, 200.0),
                20..=34 => (20, 34, 23, 140.0),
                40..=65 => (40, 58, 46, 150.0),
                _ => return None,
            };
            if frame > last {
                return None;
            }
            let k = if frame <= peak { (frame - first + 1) as f32 / (peak - first + 1) as f32 } else { (last - frame + 1) as f32 / (last - peak + 1) as f32 };
            (radius * (0.4 + 0.6 * k), FIRE.map(|c| c * k), true)
        }
        59 | 372 | 697 => (60.0, FIRE, true),
        // candles and candlesticks, when lit
        454 if frame & 3 != 0 => (55.0, CANDLE, true),
        452 if frame & 3 != 0 => (75.0, CANDLE, true),
        // tall candlesticks, when lit (the first frame of each four is out):
        // a candle, one with a small blue flame, a black one
        453 if frame & 3 != 0 && frame < 4 => (75.0, CANDLE, true),
        453 if frame & 3 != 0 && frame < 8 => (45.0, [0.45, 0.6, 1.0], true),
        453 if frame & 3 != 0 => (45.0, CANDLE, true),
        // lava, on the ground and running down walls
        111 | 409 | 479 | 487 => (90.0, LAVA, false),
        413 | 486 | 528 => (90.0, LAVA, false),
        603 => (60.0, LAVA, true),
        _ => return None,
    })
}

impl Game {
    /// The lights in and around the view, as it is about to be drawn.
    pub fn lights(&mut self, xf: &crate::turnview::Xform) -> Vec<Light> {
        let (cx, cy, _) = xf.cam;
        let (w, h) = (xf.size.0 as f32, xf.size.1 as f32);
        let reach = 2 * xf.size.0 + 4 * xf.size.1 + 1024;
        let tick = self.tick as f32;
        let on_screen = |x: f32, y: f32, r: f32| x >= -r && x <= w + r && y >= -r && y <= h + r;
        let mut out = Vec::new();
        for i in self.map().items_near(cx, cy, reach) {
            let it = self.map().items[i];
            if !it.in_world() {
                continue;
            }
            let Some((radius, rgb, flickers)) = light_of(it.shape, it.frame) else { continue };
            let radius = radius * FURTHER;
            let Some(si) = self.data.infos.get(it.shape as usize).copied() else { continue };
            let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
            // from the middle of the thing
            // (a tall fire lights the ground around it most: from low in it)
            let mid = if flickers { (zd / 2).min(16) } else { zd / 2 };
            let (sx, sy) = xf.to_screen(it.x - xd / 2, it.y - yd / 2, it.z + mid);
            let (sx, sy) = (sx as f32, sy as f32);
            // a lamp on a post also lights the ground below it, which may be
            // in the picture while the lamp itself is not
            let post = matches!(it.shape, 690 | 691);
            let ground = (it.z - POST).max(0);
            let (gx, gy) = xf.to_screen(it.x - xd / 2, it.y - yd / 2, ground);
            let ground_radius = 150.0 * FURTHER;
            let lamp_seen = on_screen(sx, sy, radius);
            let ground_seen = post && on_screen(gx as f32, gy as f32, ground_radius);
            if !lamp_seen && !ground_seen {
                continue;
            }
            if self.covered_cached(&it, &si) {
                continue;
            }
            // fire flickers, every one in its own rhythm
            let strength = if flickers {
                let p = it.id as f32 * 1.7;
                0.85 + 0.1 * (tick * 0.31 + p).sin() + 0.05 * (tick * 0.83 + 2.0 * p).sin()
            } else if is_lava(it.shape) {
                // lava glows up and down slowly, in waves rolling over it
                0.88 + 0.12 * (tick * 0.05 + (it.x + it.y) as f32 / 300.0).sin()
            } else {
                1.0
            };
            let z = it.z + mid;
            // (a fire's box is often taller than the room it burns in: its
            // light ends at the height of a room, not on the roof above)
            let top = it.z + if zd <= 8 { FLAT_REACH } else if flickers { zd.min(FIRE_REACH) } else { zd };
            if lamp_seen {
                out.push(Light { x: sx, y: sy, radius: radius * (0.95 + 0.05 * strength), rgb: rgb.map(|c| c * strength), z, top, item: i, inside: None });
            }
            if ground_seen {
                out.push(Light { x: gx as f32, y: gy as f32, radius: ground_radius, rgb: rgb.map(|c| c * 0.9), z: ground, top, item: i, inside: None });
            }
        }
        out
    }

    /// `covered`, remembered for a while: there are hundreds of pieces of
    /// lava in a picture, and looking for floor above each of them every
    /// picture took longer than all the rest of the lighting. Each answer
    /// is looked at again after two to four seconds (not all at once), so
    /// that a floor that falls in shows the lava under it.
    fn covered_cached(&mut self, it: &crate::map::Item, si: &crate::shape::ShapeInfo) -> bool {
        // only what lies flat on the ground can be covered
        if si.z != 0 {
            return false;
        }
        let (map, tick) = (self.cur_map, self.tick);
        if self.covered_seen.0 != map {
            self.covered_seen = (map, HashMap::default());
        }
        // (only for the same thing at the same place: things made by spells
        // come and go, and a new one may get the number of an old one)
        let key = (it.shape, it.x, it.y, it.z);
        if let Some(&(was, at, of)) = self.covered_seen.1.get(&it.id) {
            if of == key && tick.wrapping_sub(at) < 60 + it.id as u32 % 60 {
                return was;
            }
        }
        let now = self.covered(it, si);
        self.covered_seen.1.insert(it.id, (now, tick, key));
        now
    }
}
impl Game {
    /// Is a light covered by floor right above it (lava under a corridor
    /// that has not fallen in)? Then its glow does not show.
    fn covered(&self, it: &crate::map::Item, si: &crate::shape::ShapeInfo) -> bool {
        // only what lies flat on the ground can be covered so
        if si.z != 0 {
            return false;
        }
        let me = crate::map::WorldBox::of(it, si);
        let (cx, cy) = (it.x - me.xd / 2, it.y - me.yd / 2);
        self.map().items_near(it.x, it.y, 512).into_iter().any(|i| {
            let o = &self.map().items[i];
            let Some(oi) = self.data.infos.get(o.shape as usize) else { return false };
            let b = crate::map::WorldBox::of(o, oi);
            o.in_world()
                && o.z > it.z
                && o.z <= it.z + 48
                && oi.land
                && light_of(o.shape, o.frame).is_none()
                && cx < b.x
                && cx >= b.x - b.xd
                && cy < b.y
                && cy >= b.y - b.yd
        })
    }
}

fn is_lava(shape: u32) -> bool {
    matches!(shape, 111 | 409 | 479 | 487 | 413 | 486 | 528 | 603)
}

/// The lights are added up in a coarse grid, one for every height of light
/// (all lava on the same ground shares one): the light changes slowly over
/// the picture, so a point every CELL pixels, and the pixels in between
/// mixed from the four around them, look the same and cost a sixteenth.
/// Whether a light falls on a pixel at all (its height) is still decided
/// for every pixel.
const CELL: i32 = 4;

/// The lights of one height (inside or outside), added up in the grid.
struct Field {
    z: i32,
    top: i32,
    inside: Option<bool>,
    /// The first point of the grid (in grid steps) and its size in points.
    gx0: i32,
    gy0: i32,
    gw: i32,
    gh: i32,
    values: Vec<[f32; 3]>,
}

impl Field {
    /// The pixels covered: from (x0, y0) up to (not with) (x1, y1).
    fn pixels(&self) -> (i32, i32, i32, i32) {
        (self.gx0 * CELL, self.gy0 * CELL, (self.gx0 + self.gw - 1) * CELL, (self.gy0 + self.gh - 1) * CELL)
    }
}

/// The lights grouped by height, each group added up in its grid.
fn fields(lights: &[Light], lamp: f32, w: i32, h: i32) -> Vec<Field> {
    let mut groups: Vec<(i32, i32, Option<bool>, Vec<&Light>)> = Vec::new();
    for l in lights {
        match groups.iter_mut().find(|g| g.0 == l.z && g.1 == l.top && g.2 == l.inside) {
            Some(g) => g.3.push(l),
            None => groups.push((l.z, l.top, l.inside, vec![l])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(z, top, inside, ls)| {
            // the part of the picture the group reaches, in grid points
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for l in &ls {
                (x0, x1) = (x0.min(l.x - l.radius), x1.max(l.x + l.radius));
                (y0, y1) = (y0.min(l.y - l.radius / 2.0), y1.max(l.y + l.radius / 2.0));
            }
            let gx0 = ((x0.max(0.0) as i32) / CELL).max(0);
            let gy0 = ((y0.max(0.0) as i32) / CELL).max(0);
            let gx1 = (x1.min(w as f32) as i32 + CELL) / CELL + 1;
            let gy1 = (y1.min(h as f32) as i32 + CELL) / CELL + 1;
            let (gw, gh) = (gx1 - gx0 + 1, gy1 - gy0 + 1);
            if gw <= 1 || gh <= 1 {
                return None;
            }
            let mut values = vec![[0.0f32; 3]; (gw * gh) as usize];
            // rows of the grid spread over the processor cores
            let cores = crate::pool::cores() as i32;
            let band = ((gh + cores - 1) / cores).max(1);
            let parts: Vec<_> = values.chunks_mut((band * gw) as usize).enumerate().map(|(b, v)| (b as i32 * band, v)).collect();
            crate::pool::each(parts, |_, (r0, v)| {
                let rows = v.len() as i32 / gw;
                for l in &ls {
                    let half = l.radius / 2.0;
                    let rr = l.radius * l.radius;
                    // the grid points within reach of the light
                    let (px0, px1) = (((l.x - l.radius) / CELL as f32).floor() as i32, ((l.x + l.radius) / CELL as f32).ceil() as i32);
                    let (py0, py1) = (((l.y - half) / CELL as f32).floor() as i32, ((l.y + half) / CELL as f32).ceil() as i32);
                    for gy in py0.max(gy0 + r0)..=py1.min(gy0 + r0 + rows - 1) {
                        let dy = 2.0 * ((gy * CELL) as f32 - l.y);
                        let row = ((gy - gy0 - r0) * gw) as usize;
                        for gx in px0.max(gx0)..=px1.min(gx0 + gw - 1) {
                            let dx = (gx * CELL) as f32 - l.x;
                            let d2 = (dx * dx + dy * dy) / rr;
                            if d2 >= 1.0 {
                                continue;
                            }
                            // soft towards the edge
                            let f = (1.0 - d2) * (1.0 - 0.4 * d2) * lamp;
                            let p = &mut v[row + (gx - gx0) as usize];
                            for c in 0..3 {
                                p[c] += f * l.rgb[c];
                            }
                        }
                    }
                }
            });
            Some(Field { z, top, inside, gx0, gy0, gw, gh, values })
        })
        .collect()
}

/// Darkens the picture to the general light and lights it up around the
/// lights, as `look` has it. `ids` (may be empty): the buffer of item
/// numbers + 1 of the picture, to know how high the thing at each pixel
/// stands, and which pixels belong to something that gives light: those
/// keep their own colours (lit by themselves and their neighbours, a
/// stretch of lava turned a flat yellow).
///
/// `rooms` (may be empty): for the things (by index), are they inside a
/// house (`IN_HOUSE`, `OUTSIDE`, 0: not known); `inner`: the look of the
/// rooms of the house the Avatar is in (see the top of this file).
#[allow(clippy::too_many_arguments)]
pub fn apply(surf: &mut Surface, lights: &[Light], look: Look, ids: &[u32], items: &[crate::map::Item], infos: &[crate::shape::ShapeInfo], rooms: &[u8], inner: Option<Look>) {
    let (w, h) = (surf.width, surf.height);
    // the dark keeps dark colours a little lighter than bright ones
    // (multiplying all alike made dark floors and carpets black): the
    // factor per colour for every brightness of a pixel, worked out once
    let dims = |look: Look| -> [[f32; 256]; 3] {
        std::array::from_fn(|c| {
            std::array::from_fn(|y| look.ambient[c].powf(0.6 + 0.4 * y as f32 / 255.0))
        })
    };
    let (dim_out, lamp_out) = (dims(look), look.lamp);
    let (dim_in, lamp_in) = match inner {
        Some(inner) => (dims(inner), inner.lamp),
        None => (dim_out, lamp_out),
    };
    let fields = fields(lights, 1.0, w, h);
    let spans: Vec<_> = fields.iter().map(Field::pixels).collect();
    let ids = if ids.len() == (w * h) as usize { ids } else { &[] };
    // in bands of rows, one per processor core
    let cores = crate::pool::cores() as i32;
    let band = ((h + cores - 1) / cores).max(1);
    let parts: Vec<_> = surf
        .pixels
        .chunks_mut((band * w) as usize)
        .enumerate()
        .map(|(b, pixels)| {
            let start = b * (band * w) as usize;
            let ids = ids.get(start..start + pixels.len()).unwrap_or(&[]);
            (b as i32 * band, pixels, ids)
        })
        .collect();
    crate::pool::each(parts, |_, (y0, pixels, ids)| {
        for (i, px) in pixels.iter_mut().enumerate() {
            let (x, y) = (i as i32 % w, y0 + i as i32 / w);
            // the thing seen here: how high it stands, and does it give light?
            let index = ids.get(i).and_then(|&id| id.checked_sub(1)).map(|i| i as usize);
            let thing = index.and_then(|i| items.get(i));
            if thing.is_some_and(|it| light_of(it.shape, it.frame).is_some()) {
                continue;
            }
            let room = index.and_then(|i| rooms.get(i).copied()).unwrap_or(0);
            let (dim, lamp) = if room == IN_HOUSE && inner.is_some() { (&dim_in, lamp_in) } else { (&dim_out, lamp_out) };
            // inside or outside (of a wall: the side seen)
            let side = match room {
                IN_HOUSE => Some(true),
                OUTSIDE => Some(false),
                _ => None,
            };
            let si = thing.and_then(|it| infos.get(it.shape as usize));
            let height = thing.map(|it| (it.z, it.z + si.map_or(0, |si| si.z * 8)));
            let roof = si.is_some_and(|si| si.roof);
            let mut l = [0.0f32; 3];
            for (f, &(x0, fy0, x1, fy1)) in fields.iter().zip(&spans) {
                if x < x0 || y < fy0 || x >= x1 || y >= fy1 {
                    continue;
                }
                // not from inside a house out, or from outside in
                if let (Some(a), Some(b)) = (f.inside, side) {
                    if a != b {
                        continue;
                    }
                }
                // not on what stands above the light or lies far below it
                // and a roof only from a light as high as it (the slopes of
                // a roof come down lower than a candle in the house stands)
                if let Some((foot, top)) = height {
                    if foot >= f.top || top < f.z - BELOW || (roof && foot > f.z) {
                        continue;
                    }
                }
                // between the four grid points around the pixel
                let (gx, gy) = ((x - x0) / CELL, (y - fy0) / CELL);
                let (fx, fy) = (((x - x0) % CELL) as f32 / CELL as f32, ((y - fy0) % CELL) as f32 / CELL as f32);
                let k = (gy * f.gw + gx) as usize;
                let (a, b, c, d) = (f.values[k], f.values[k + 1], f.values[k + f.gw as usize], f.values[k + f.gw as usize + 1]);
                for n in 0..3 {
                    let top = a[n] + (b[n] - a[n]) * fx;
                    let bottom = c[n] + (d[n] - c[n]) * fx;
                    l[n] += top + (bottom - top) * fy;
                }
            }
            let (r, g, b) = ((*px >> 16) & 0xFF, (*px >> 8) & 0xFF, *px & 0xFF);
            let luma = ((77 * r + 151 * g + 28 * b) >> 8) as usize;
            // where many lights meet (a field of lava) the light does not
            // add up without end: at most as strong as one strong light,
            // in its colour (else the rock by lava turned pale and pink)
            let l = l.map(|v| v * lamp);
            let most = l[0].max(l[1]).max(l[2]);
            let l = if most > ADDED { l.map(|v| v * ADDED / most) } else { l };
            let ch = |v: u32, c: usize| {
                let k = (dim[c][luma] + l[c]).min(MOST);
                ((v as f32 * k + 255.0 * TINT * l[c]) as u32).min(255)
            };
            *px = (ch(r, 0) << 16) | (ch(g, 1) << 8) | ch(b, 2);
        }
    });
}

/// What is known of things being in a room under a roof: the map, and by
/// item id where the thing stood when looked at, and the answer.
pub type RoomsSeen = (usize, HashMap<u16, ((i32, i32, i32), bool)>);

/// In `rooms` of `apply`: a thing in a house, or outside.
pub const IN_HOUSE: u8 = 2;
pub const OUTSIDE: u8 = 1;

/// For the things painted (by index into the items): does it stand in a
/// room under a roof (`IN_HOUSE`, `OUTSIDE`)? The answers are kept
/// (`seen`) while the thing stays where it is; at most `ROOM_CHECKS` new
/// ones per picture, the others are not known (0) until their turn comes
/// (a moment later).
pub fn rooms(painted: &[crate::view::Painted], map: &crate::map::MapData, map_no: usize, infos: &[crate::shape::ShapeInfo], seen: &mut RoomsSeen) -> Vec<u8> {
    const ROOM_CHECKS: usize = 600;
    if seen.0 != map_no {
        *seen = (map_no, HashMap::default());
    }
    let mut out = vec![0u8; map.items.len()];
    let mut checks = 0;
    for p in painted {
        let Some(it) = map.items.get(p.index) else { continue };
        let at = (it.x, it.y, it.z);
        let known = seen.1.get(&it.id).filter(|(was, _)| *was == at).map(|&(_, inside)| inside);
        let inside = match known {
            Some(inside) => Some(inside),
            None if checks < ROOM_CHECKS => {
                checks += 1;
                let inside = match wall_facing(it, infos) {
                    Some(facing) => faces_room(map, infos, it, facing),
                    // a post or corner stone: both seen sides
                    None if is_post(it, infos) => faces_room(map, infos, it, (1, 0)) && faces_room(map, infos, it, (0, 1)),
                    None => under_roof(map, infos, it),
                };
                seen.1.insert(it.id, (at, inside));
                Some(inside)
            }
            None => None,
        };
        out[p.index] = match inside {
            Some(true) => IN_HOUSE,
            Some(false) => OUTSIDE,
            None => 0,
        };
    }
    out
}

/// Is a light inside a house (as `rooms`, but always found out: there
/// are only a few lights)?
pub fn light_inside(map: &crate::map::MapData, map_no: usize, infos: &[crate::shape::ShapeInfo], item: usize, seen: &mut RoomsSeen) -> Option<bool> {
    if seen.0 != map_no {
        *seen = (map_no, HashMap::default());
    }
    let it = map.items.get(item)?;
    let at = (it.x, it.y, it.z);
    if let Some(&(was, inside)) = seen.1.get(&it.id) {
        if was == at {
            return Some(inside);
        }
    }
    // hanging on a wall: on the side of the wall it is on
    let inside = match on_wall(map, infos, it) {
        Some((wall, facing)) => faces_room(map, infos, wall, facing),
        None => around_under_roof(map, infos, it),
    };
    seen.1.insert(it.id, (at, inside));
    Some(inside)
}

/// The wall a thing hangs on (right behind it, its seen side towards the
/// thing), and the way that side faces.
fn on_wall<'a>(map: &'a crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item) -> Option<(&'a crate::map::Item, (i32, i32))> {
    let si = infos.get(it.shape as usize)?;
    let b = crate::map::WorldBox::of(it, si);
    let (mx, my) = (b.x - b.xd / 2, b.y - b.yd / 2);
    let nowhere = crate::map::WorldBox { x: 0, y: 0, z: -10000, xd: 0, yd: 0, zd: 0 };
    for (dx, dy) in [(-1, 0), (0, -1)] {
        let (cx, cy) = (mx + dx * (b.xd / 2 + 8), my + dy * (b.yd / 2 + 8));
        let probe = crate::map::WorldBox { x: cx + 8, y: cy + 8, z: it.z, xd: 16, yd: 16, zd: 16 };
        let pi = map.position_info(&probe, &nowhere, infos, it.id);
        if pi.valid {
            continue;
        }
        if let Some(wall) = pi.blocker.and_then(|id| map.get(id)) {
            if wall_facing(wall, infos) == Some((-dx, -dy)) {
                return Some((wall, (-dx, -dy)));
            }
        }
    }
    None
}

/// As `under_roof`, and around it as well, most of five points (above a
/// fireplace the chimney goes through the roof; a lamp or wall under the
/// eaves or a porch is still outside).
fn around_under_roof(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item) -> bool {
    [(0, 0), (48, 0), (-48, 0), (0, 48), (0, -48)].iter().filter(|&&(dx, dy)| under_roof(map, infos, &crate::map::Item { x: it.x + dx, y: it.y + dy, ..*it })).count() >= 3
}

/// A wall (solid, tall, thin in x or y and long in the other; not a post):
/// of it only the side towards the viewer is seen, the one facing +x or
/// +y. Returns that direction, or None for anything else.
fn wall_facing(it: &crate::map::Item, infos: &[crate::shape::ShapeInfo]) -> Option<(i32, i32)> {
    let si = infos.get(it.shape as usize)?;
    let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
    if !si.solid || si.roof || zd < 32 || xd.min(yd) > 32 || xd.max(yd) <= 32 {
        return None;
    }
    Some(if xd <= 32 { (1, 0) } else { (0, 1) })
}

/// A post or the corner stone of walls: solid, tall, small across.
fn is_post(it: &crate::map::Item, infos: &[crate::shape::ShapeInfo]) -> bool {
    infos.get(it.shape as usize).is_some_and(|si| {
        let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
        si.solid && !si.roof && zd >= 32 && xd <= 32 && yd <= 32
    })
}

/// Does the seen side of a wall face a room? From just in front of it,
/// going straight out and to both sides along it under the roof: a room
/// if each way meets another wall before the roof ends (a porch or the
/// eaves are open to one side: that is outside). Walls right next to it
/// do not count (the stones of the same pillar, a fireplace in front).
fn faces_room(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item, (dx, dy): (i32, i32)) -> bool {
    const STEP: i32 = 32;
    const STEPS: i32 = 40;
    let Some(si) = infos.get(it.shape as usize) else { return false };
    let b = crate::map::WorldBox::of(it, si);
    // the middle of the wall's foot, moved out to just in front of it
    let (mx, my) = (b.x - b.xd / 2 + dx * (b.xd / 2 + 64), b.y - b.yd / 2 + dy * (b.yd / 2 + 64));
    // a start nothing overlaps
    let nowhere = crate::map::WorldBox { x: 0, y: 0, z: -10000, xd: 0, yd: 0, zd: 0 };
    // how far to the next wall, None: out under the open sky
    let closed = |(wx, wy): (i32, i32)| -> Option<i32> {
        // open sky a few steps in a row (not just a chimney above)
        let mut open = 0;
        for k in 0..STEPS {
            let (cx, cy) = (mx + wx * k * STEP, my + wy * k * STEP);
            // wide across the way (not through a doorway)
            let (hx, hy) = if wx != 0 { (16, 48) } else { (48, 16) };
            let probe = crate::map::WorldBox { x: cx + hx, y: cy + hy, z: it.z + 8, xd: 2 * hx, yd: 2 * hy, zd: 24 };
            let pi = map.position_info(&probe, &nowhere, infos, it.id);
            if pi.roof_z.is_none() {
                open += 1;
                if open >= 5 {
                    return None;
                }
            } else {
                open = 0;
            }
            if k >= 2 && !pi.valid && pi.blocker.and_then(|id| map.get(id)).is_some_and(|o| wall_facing(o, infos).is_some() || is_post(o, infos)) {
                return Some(k * STEP);
            }
        }
        Some(STEPS * STEP)
    };
    closed((dx, dy)).is_some() && closed((dy, dx)).is_some() && closed((-dy, -dx)).is_some()
}

/// Is there a roof above the middle of a thing (as the test that hides
/// the roofs while the Avatar is below one)? (Also for a thing marked as a
/// roof: many floors of houses are, as the ceiling of a room below.)
fn under_roof(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item) -> bool {
    let Some(si) = infos.get(it.shape as usize) else { return false };
    let b = crate::map::WorldBox::of(it, si);
    let probe = crate::map::WorldBox { x: b.x - b.xd / 2 + 16, y: b.y - b.yd / 2 + 16, z: it.z, xd: 32, yd: 32, zd: 16 };
    map.position_info(&probe, &probe, infos, it.id).roof_z.is_some()
}
