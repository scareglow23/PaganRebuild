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
//! Walls cast shadows: the light of a lamp or fire does not reach what
//! lies behind a wall from it (a candle does not shine through the wall
//! onto the street, a lamp outside on the wall not into the room, but onto
//! the ground before the house, under the eaves of its roof too). Of a wall
//! only the side seen is lit, and only by lights on that side. Whether a
//! thing stands in a room under a roof (`rooms`) only decides the general
//! light of the rooms.
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
/// Flat lights (lava, fire fields) lie in fields of dozens of pieces, and
/// their light added up stayed strong almost to the end of its reach and
/// then fell off within a few pixels: between two fields of lava the
/// floor had a dark band with hard edges, like a shadow. Each piece
/// reaches this much further, a little weaker, and falls off sooner and
/// more gently (`SOFT_FALLOFF`): near the lava it looks as before, the
/// way into the dark is long.
const SOFT_REACH: f32 = 1.8;
const SOFT_STRENGTH: f32 = 0.9;
/// The light of a flat light at a distance d (0..1 of its reach) is
/// (1 - d^2) to this power.
const SOFT_FALLOFF: i32 = 3;
/// A light falls on what reaches at least up to this much below it (a lamp
/// on an upper floor does not light the room under it).
const BELOW: i32 = 104;
/// The floor of a room under an upper storey lies this much below its
/// floor (houses: 48).
const STOREY_MIN: i32 = 24;
const STOREY_MAX: i32 = 120;
/// Flat lights (lava, a fire field) light what stands up to this high
/// above their ground: the rock beside lava is often a little higher.
const FLAT_REACH: i32 = 40;
/// Fires light what stands up to this high above their ground.
const FIRE_REACH: i32 = 48;
/// A flat light (lava, a fire field) does not end at a height but fades
/// with the height above its ground, pixel by pixel (a tall thing is lit
/// at its foot and less higher up): in full up to `SOFT_FULL`, then less
/// and less, gone after another `SOFT_FADE`. Judged by the foot of each
/// thing alone, the pieces of a wall of rock side by side were lit
/// differently all over, and their edges showed as stripes. Rocks standing in lava are
/// put together from pieces one above the other, and the floor of a way
/// rises in steps of 8: with a hard end (lit in full up to 40, above not
/// at all) a hard line ran across a rock or a way; fading over 48 still
/// left a step of a sixth of the light at each step of the floor. Over
/// 120 a step of 8 changes the light by a fifteenth. Lamps and fires keep
/// the hard end: their light is not to reach through a ceiling into the
/// room above.
const SOFT_FULL: i32 = 16;
const SOFT_FADE: i32 = 120;

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
    /// Where it is in the world (x, y of its middle).
    pub wx: i32,
    pub wy: i32,
    /// A flat thing (a lamp on a wall): thin across x (true) or y.
    pub thin: Option<bool>,
    /// A flat light (lava, a fire field): fades out above `top`.
    pub soft: bool,
    /// The height of the floor of an upper storey it stands on (`i32::MIN`:
    /// none): what lies wholly below that floor is in the room under it.
    pub floor: i32,
}

impl Light {
    /// The points of the grid within its reach (first and last in x, in y;
    /// not cut to the picture).
    fn grid_reach(&self) -> (i32, i32, i32, i32) {
        let half = self.radius / 2.0;
        let cell = CELL as f32;
        (((self.x - self.radius) / cell).floor() as i32, ((self.x + self.radius) / cell).ceil() as i32, ((self.y - half) / cell).floor() as i32, ((self.y + half) / cell).ceil() as i32)
    }
}

/// A flame bolt (and the fireball 260): a hot core, yellow-white (`core`),
/// in a deep orange glow that ends in red, not a wide even shine.
const BOLT_EDGE: [f32; 3] = [1.15, 0.38, 0.1];

/// The small hot core some fires have besides their glow (radius,
/// colour), as a second light in the same place.
fn core_of(shape: u32) -> Option<(f32, [f32; 3])> {
    match shape {
        260 | 261 => Some((26.0, [1.05, 0.6, 0.22])),
        _ => None,
    }
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
        260 | 261 => (65.0, BOLT_EDGE, true),
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
        // floor plates broken open over lava: big holes, a middling one,
        // thin cracks (it is the lava of the hole that glows)
        478 => match frame {
            0..=3 | 5..=7 => (90.0, LAVA, false),
            4 | 28 => (70.0, LAVA.map(|c| c * 0.6), false),
            _ => (50.0, LAVA.map(|c| c * 0.35), false),
        },
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
            let Some(si) = self.data.infos.get(it.shape as usize).copied() else { continue };
            let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
            // a flat light (lava, a fire field): see `SOFT_REACH`
            let soft = zd <= 8;
            let (radius, rgb) = if soft { (radius * FURTHER * SOFT_REACH, rgb.map(|c| c * SOFT_STRENGTH)) } else { (radius * FURTHER, rgb) };
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
            let (wx, wy) = (it.x - xd / 2, it.y - yd / 2);
            let thin = (xd != yd && xd.min(yd) <= 32).then_some(xd < yd);
            // (a fire's box is often taller than the room it burns in: its
            // light ends at the height of a room, not on the roof above)
            let top = it.z + if zd <= 8 { FLAT_REACH } else if flickers { zd.min(FIRE_REACH) } else { zd };
            // (a candle upstairs does not shine through the floor it stands
            // on into the room below; not so the turret on a roof (661):
            // its glow is the light of the room under it)
            let floor = if soft || post || it.shape == 661 { i32::MIN } else { self.storey_floor(&it, wx, wy) };
            if lamp_seen {
                out.push(Light { x: sx, y: sy, radius: radius * (0.95 + 0.05 * strength), rgb: rgb.map(|c| c * strength), z, top, wx, wy, thin, soft, floor });
                if let Some((core_radius, core_rgb)) = core_of(it.shape) {
                    out.push(Light { x: sx, y: sy, radius: core_radius * FURTHER, rgb: core_rgb.map(|c| c * strength), z, top, wx, wy, thin, soft, floor });
                }
            }
            if ground_seen {
                out.push(Light { x: gx as f32, y: gy as f32, radius: ground_radius, rgb: rgb.map(|c| c * 0.9), z: ground, top, wx, wy, thin, soft, floor });
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
        if self.view.covered_seen.0 != map {
            self.view.covered_seen = (map, HashMap::default());
        }
        // (only for the same thing at the same place: things made by spells
        // come and go, and a new one may get the number of an old one)
        let key = (it.shape, it.x, it.y, it.z);
        if let Some(&(was, at, of)) = self.view.covered_seen.1.get(&it.id) {
            if of == key && tick.wrapping_sub(at) < 60 + it.id as u32 % 60 {
                return was;
            }
        }
        let now = self.covered(it, si);
        self.view.covered_seen.1.insert(it.id, (now, tick, key));
        now
    }
}
impl Game {
    /// The height of the floor of an upper storey (a flat piece of ground
    /// that is also the ceiling of the room below, marked as roof) that a
    /// light standing at (`wx`, `wy`) stands on, with a room under it (floor
    /// a storey lower); `i32::MIN` if none. (Many ground floors are marked
    /// as roof too.)
    fn storey_floor(&self, it: &crate::map::Item, wx: i32, wy: i32) -> i32 {
        let items = self.map().items_near(it.x, it.y, 256);
        let under = |test: &dyn Fn(&crate::map::Item, &crate::shape::ShapeInfo, &crate::map::WorldBox) -> bool| {
            items.iter().any(|&i| {
                let o = &self.map().items[i];
                let Some(oi) = self.data.infos.get(o.shape as usize) else { return false };
                let b = crate::map::WorldBox::of(o, oi);
                o.in_world() && oi.land && wx < b.x && wx >= b.x - b.xd && wy < b.y && wy >= b.y - b.yd && test(o, oi, &b)
            })
        };
        let on_floor = under(&|o, oi, b| oi.roof && b.zd <= 8 && o.z + b.zd == it.z);
        let room_below = on_floor && under(&|o, _, b| (it.z - STOREY_MAX..=it.z - STOREY_MIN).contains(&(o.z + b.zd)));
        if room_below { it.z } else { i32::MIN }
    }

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
                // (not a roof: the hearth of a smithy lies just under it)
                && !oi.roof
                && light_of(o.shape, o.frame).is_none()
                && cx < b.x
                && cx >= b.x - b.xd
                && cy < b.y
                && cy >= b.y - b.yd
        })
    }
}

/// A pixel of lava that glows (red, orange, yellow, almost white), not one
/// of the grey stone slabs drawn into the edge of a field of lava (grey,
/// blue-grey, olive grey: as much red as green).
fn glows(px: u32) -> bool {
    let (r, g) = ((px >> 16) & 0xFF, (px >> 8) & 0xFF);
    r >= 240 || r > g + 15
}

fn is_lava(shape: u32) -> bool {
    matches!(shape, 111 | 409 | 478 | 479 | 487 | 413 | 486 | 528 | 603)
}

/// The lights are added up in a coarse grid, one for every height of light
/// (all lava on the same ground shares one): the light changes slowly over
/// the picture, so a point every CELL pixels, and the pixels in between
/// mixed from the four around them, look the same and cost a sixteenth.
/// Whether a light falls on a pixel at all (its height) is still decided
/// for every pixel.
const CELL: i32 = 4;

/// The lights of one height, added up in the grid.
struct Field {
    z: i32,
    top: i32,
    /// Fades with the height (see `SOFT_FADE`).
    soft: bool,
    /// See `Light::floor`.
    floor: i32,
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
/// `shade`: does a light (by its place in the list) reach a grid point
/// (1), or is a wall in the way (0)?
fn fields(lights: &[Light], w: i32, h: i32, shade: &(dyn Fn(usize, i32, i32) -> f32 + Sync)) -> Vec<Field> {
    let mut groups: Vec<(i32, i32, bool, i32, Vec<(usize, &Light)>)> = Vec::new();
    for (n, l) in lights.iter().enumerate() {
        match groups.iter_mut().find(|g| g.0 == l.z && g.1 == l.top && g.2 == l.soft && g.3 == l.floor) {
            Some(g) => g.4.push((n, l)),
            None => groups.push((l.z, l.top, l.soft, l.floor, vec![(n, l)])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(z, top, soft, floor, ls)| {
            // the part of the picture the group reaches, in grid points
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (_, l) in &ls {
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
            let band = crate::pool::band(gh);
            let parts: Vec<_> = values.chunks_mut((band * gw) as usize).enumerate().map(|(b, v)| (b as i32 * band, v)).collect();
            crate::pool::each(parts, |_, (r0, v)| {
                let rows = v.len() as i32 / gw;
                for &(n, l) in &ls {
                    let rr = l.radius * l.radius;
                    let (px0, px1, py0, py1) = l.grid_reach();
                    for gy in py0.max(gy0 + r0)..=py1.min(gy0 + r0 + rows - 1) {
                        let dy = 2.0 * ((gy * CELL) as f32 - l.y);
                        let row = ((gy - gy0 - r0) * gw) as usize;
                        for gx in px0.max(gx0)..=px1.min(gx0 + gw - 1) {
                            let dx = (gx * CELL) as f32 - l.x;
                            let d2 = (dx * dx + dy * dy) / rr;
                            if d2 >= 1.0 {
                                continue;
                            }
                            // soft towards the edge (flat lights longer and
                            // more gently, see `SOFT_REACH`)
                            let f = if soft { (1.0 - d2).powi(SOFT_FALLOFF) } else { (1.0 - d2) * (1.0 - 0.4 * d2) } * shade(n, gx, gy);
                            if f <= 0.0 {
                                continue;
                            }
                            let p = &mut v[row + (gx - gx0) as usize];
                            for c in 0..3 {
                                p[c] += f * l.rgb[c];
                            }
                        }
                    }
                }
            });
            Some(Field { z, top, soft, floor, gx0, gy0, gw, gh, values })
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
/// `origins` (may be empty): for the things painted (by index), where the
/// point they stand on (their corner at the front, at their foot) is in
/// the picture, to tell how high above its foot a pixel of a tall thing
/// is (see `SOFT_FADE`).
#[allow(clippy::too_many_arguments)]
pub fn apply(surf: &mut Surface, lights: &[Light], look: Look, ids: &[u32], items: &[crate::map::Item], infos: &[crate::shape::ShapeInfo], rooms: &[u8], inner: Option<Look>, origins: &[(i32, i32)], shadows: Option<Shadows>) {
    let (w, h) = (surf.width, surf.height);
    // the dark keeps dark colours a little lighter than bright ones
    // (multiplying all alike made dark floors and carpets black): the
    // factor per colour for every brightness of a pixel, worked out once
    let dims = |look: Look| -> [[f32; 256]; 3] { std::array::from_fn(|c| std::array::from_fn(|luma| dim_factor(look.ambient[c], luma))) };
    let (dim_out, lamp_out) = (dims(look), look.lamp);
    let (dim_in, lamp_in) = match inner {
        Some(inner) => (dims(inner), inner.lamp),
        None => (dim_out, lamp_out),
    };
    // the colours of pixels no light reaches, looked up (most of the
    // picture at night)
    let (unlit_out, unlit_in) = (unlit_table(look), inner.map_or_else(|| unlit_table(look), unlit_table));
    let ids = if ids.len() == (w * h) as usize { ids } else { &[] };
    // The grid moves with the world, not with the picture: a point of it
    // stays on the same spot while the view scrolls (else the edges of the
    // shadows, decided at the points, trembled when walking). Pixel x is
    // at x + dx in the grid.
    let (dx, dy) = shadows.as_ref().map_or((0, 0), |sh| ((-sh.anchor.0).rem_euclid(CELL), (-sh.anchor.1).rem_euclid(CELL)));
    let moved: Vec<Light> = lights.iter().map(|l| Light { x: l.x + dx as f32, y: l.y + dy as f32, ..*l }).collect();
    // what each point of the grid shows, in the world (for the shadows of walls)
    let targets = match &shadows {
        Some(sh) if !ids.is_empty() => targets(ids, items, infos, w, h, (dx, dy), sh.to_world, &in_reach(&moved, &sh.sights, w, h, (dx, dy))),
        _ => Vec::new(),
    };
    let (tw, th) = ((w + dx) / CELL + 2, (h + dy) / CELL + 2);
    // for each light: which grid points in its reach it lights (the rest
    // lie behind a wall from it)
    let lit: Vec<Option<Lit>> = match shadows {
        Some(sh) if !targets.is_empty() => {
            // (grid point gx lies at gx + ox on the grid fixed in the world)
            let (ox, oy) = ((-dx - sh.anchor.0).div_euclid(CELL), (-dy - sh.anchor.1).div_euclid(CELL));
            lit_points(&moved, sh.sights, &targets, (tw, th), (ox, oy), sh.cache)
        }
        _ => Vec::new(),
    };
    // Where the shadow of a wall begins, the light is mixed between four
    // grid points of which some lie on another thing (a door lit, the floor
    // behind it not): mixed across, a seam of light showed around the door
    // and along the walls. There each pixel takes only the grid points on
    // the thing it shows. `edge`: the squares of the grid with a corner in
    // the shadow of some light and not all corners on one thing (else the
    // usual mixing gives the same); `on`: the thing (id + 1) seen at each
    // grid point.
    let mut edge = shadow_squares(&lit, tw, th);
    let on = |gx: i32, gy: i32| -> u32 {
        let (sx, sy) = (gx * CELL - dx, gy * CELL - dy);
        if sx < 0 || sy < 0 || sx >= w || sy >= h { 0 } else { ids.get((sy * w + sx) as usize).copied().unwrap_or(0) }
    };
    for (k, e) in edge.iter_mut().enumerate() {
        let (gx, gy) = (k as i32 % tw, k as i32 / tw);
        if *e {
            let first = on(gx, gy);
            *e = on(gx + 1, gy) != first || on(gx, gy + 1) != first || on(gx + 1, gy + 1) != first;
        }
    }
    let shade = |n: usize, gx: i32, gy: i32| -> f32 {
        match lit.get(n) {
            Some(Some(v)) => v.at(gx, gy),
            _ => 1.0,
        }
    };
    let fields = fields(&moved, w + dx, h + dy, &shade);
    let spans: Vec<_> = fields.iter().map(Field::pixels).collect();
    // in bands of rows, one per processor core
    let band = crate::pool::band(h);
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
    let seen_of = |id: u32| -> Seen {
        let index = id.checked_sub(1).map(|i| i as usize);
        let thing = index.and_then(|i| items.get(i));
        let room = index.and_then(|i| rooms.get(i).copied()).unwrap_or(0);
        let (dim, unlit, lamp) = if room == IN_HOUSE && inner.is_some() { (&dim_in, &unlit_in[..], lamp_in) } else { (&dim_out, &unlit_out[..], lamp_out) };
        let si = thing.and_then(|it| infos.get(it.shape as usize));
        Seen {
            id,
            own_light: thing.is_some_and(|it| light_of(it.shape, it.frame).is_some()),
            lava: thing.is_some_and(|it| is_lava(it.shape)),
            dim,
            unlit,
            lamp,
            height: thing.map(|it| (it.z, it.z + si.map_or(0, |si| si.z * 8))),
            roof: si.is_some_and(|si| si.roof),
            origin: index.and_then(|i| origins.get(i)).filter(|o| o.0 != i32::MIN).copied(),
        }
    };
    crate::pool::each(parts, |_, (y0, pixels, ids)| {
        let mut seen = seen_of(0);
        // per row: the fields that reach it, with their row in the grid and
        // the share of the row below (k: index of the left point)
        let mut active: Vec<(&Field, i32, i32, usize)> = Vec::with_capacity(fields.len());
        // per active field: its light along this row at each grid column
        // (between the grid rows above and below)
        let mut rowvals: Vec<Vec<[f32; 3]>> = fields.iter().map(|f| vec![[0.0; 3]; f.gw as usize]).collect();
        for (row, line) in pixels.chunks_mut(w as usize).enumerate() {
            let y = y0 + row as i32;
            let uy = y + dy;
            active.clear();
            for (fi, (f, &(x0, fy0, x1, fy1))) in fields.iter().zip(&spans).enumerate() {
                if uy >= fy0 && uy < fy1 {
                    let gy = (uy - fy0) / CELL;
                    let fy = ((uy - fy0) % CELL) as f32 / CELL as f32;
                    let (above, below) = ((gy * f.gw) as usize, ((gy + 1) * f.gw) as usize);
                    for (gx, v) in rowvals[fi].iter_mut().enumerate() {
                        let (a, c) = (f.values[above + gx], f.values[below + gx]);
                        *v = [a[0] + (c[0] - a[0]) * fy, a[1] + (c[1] - a[1]) * fy, a[2] + (c[2] - a[2]) * fy];
                    }
                    active.push((f, x0, x1, fi));
                }
            }
            for (xi, px) in line.iter_mut().enumerate() {
                let x = xi as i32;
                let id = ids.get(row * w as usize + xi).copied().unwrap_or(0);
                if id != seen.id {
                    seen = seen_of(id);
                }
                // what gives light keeps its own colours (of lava only what
                // glows: the broken stone slabs drawn into the edge pieces of
                // a field of lava are lit as the floor beside them; kept as
                // they are, they stood cold and grey against the floor warm
                // in the glow, with hard edges)
                if seen.own_light && (!seen.lava || glows(*px)) {
                    continue;
                }
                let pixel_z = seen.z_at(x, y);
                let mut l = [0.0f32; 3];
                // (where the pixel is in the grid)
                let ux = x + dx;
                // by a shadow's edge: the share of each of the four grid
                // points around the pixel, only those on the same thing
                let (cgx, cgy) = (ux / CELL, uy / CELL);
                let corners = [(cgx, cgy), (cgx + 1, cgy), (cgx, cgy + 1), (cgx + 1, cgy + 1)];
                let near_edge = edge.get((cgy * tw + cgx) as usize) == Some(&true);
                let own: Option<[f32; 4]> = near_edge.then(|| {
                    let (fx, fy) = ((ux % CELL) as f32 / CELL as f32, (uy % CELL) as f32 / CELL as f32);
                    let mut wts = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
                    for (k, &(gx, gy)) in corners.iter().enumerate() {
                        if on(gx, gy) != id {
                            wts[k] = 0.0;
                        }
                    }
                    let sum: f32 = wts.iter().sum();
                    (sum > 0.0).then(|| wts.map(|v| v / sum))
                }).flatten();
                for &(f, x0, x1, fi) in &active {
                    if ux < x0 || ux >= x1 {
                        continue;
                    }
                    let Some(share) = seen.share(f, pixel_z) else { continue };
                    // between the four grid points around the pixel
                    let gx = ((ux - x0) / CELL) as usize;
                    if let Some(wts) = own {
                        let gyl = ((uy - f.gy0 * CELL) / CELL) as usize;
                        let base = gyl * f.gw as usize + gx;
                        let vals = [f.values[base], f.values[base + 1], f.values[base + f.gw as usize], f.values[base + f.gw as usize + 1]];
                        for n in 0..3 {
                            l[n] += (vals[0][n] * wts[0] + vals[1][n] * wts[1] + vals[2][n] * wts[2] + vals[3][n] * wts[3]) * share;
                        }
                        continue;
                    }
                    let fx = ((ux - x0) % CELL) as f32 / CELL as f32;
                    let row = &rowvals[fi];
                    let (a, b) = (row[gx], row[gx + 1]);
                    for n in 0..3 {
                        l[n] += (a[n] + (b[n] - a[n]) * fx) * share;
                    }
                }
                let (r, g, b) = ((*px >> 16) & 0xFF, (*px >> 8) & 0xFF, *px & 0xFF);
                let luma = ((77 * r + 151 * g + 28 * b) >> 8) as usize;
                if l == [0.0; 3] {
                    let t = &seen.unlit[luma * 768..];
                    *px = (u32::from(t[r as usize]) << 16) | (u32::from(t[256 + g as usize]) << 8) | u32::from(t[512 + b as usize]);
                    continue;
                }
                // where many lights meet (a field of lava) the light does not
                // add up without end: at most as strong as one strong light,
                // in its colour (else the rock by lava turned pale and pink)
                let l = l.map(|v| v * seen.lamp);
                let most = l[0].max(l[1]).max(l[2]);
                let l = if most > ADDED { l.map(|v| v * ADDED / most) } else { l };
                let dim = seen.dim;
                let ch = |v: u32, c: usize| {
                    let k = (dim[c][luma] + l[c]).min(MOST);
                    ((v as f32 * k + 255.0 * TINT * l[c]) as u32).min(255)
                };
                *px = (ch(r, 0) << 16) | (ch(g, 1) << 8) | ch(b, 2);
            }
        }
    });
}

/// How much the general light `ambient` (of one colour) keeps of a pixel
/// of brightness `luma` (0..255): dark colours a little more than bright
/// ones.
fn dim_factor(ambient: f32, luma: usize) -> f32 {
    ambient.powf(0.6 + 0.4 * luma as f32 / 255.0)
}

/// What is known of the thing seen at a pixel (in `apply`), worked out
/// once for each run of pixels showing the same thing.
struct Seen<'a> {
    /// Item number + 1 (0: nothing).
    id: u32,
    /// It gives light: it keeps its own colours.
    own_light: bool,
    /// It is lava (see `glows`).
    lava: bool,
    /// The general light on it (by colour and brightness), the colours
    /// where no light reaches (`unlit_table`), the strength of the lights.
    dim: &'a [[f32; 256]; 3],
    unlit: &'a [u8],
    lamp: f32,
    /// The height of its foot and top.
    height: Option<(i32, i32)>,
    roof: bool,
    /// Where its front corner at the foot is in the picture.
    origin: Option<(i32, i32)>,
}

impl Seen<'_> {
    /// How high the pixel (x, y) of it is in the world: a tall thing (a wall
    /// of rock) rises from its foot, and a pixel higher up in the picture
    /// than the foot below it is that much higher. Its front corner at the
    /// foot is at `origin`; the foot runs back from there half a pixel up
    /// for every pixel to the side. Flat things lie at their foot all over.
    fn z_at(&self, x: i32, y: i32) -> Option<i32> {
        self.height.map(|(foot, top)| match self.origin {
            Some((sx0, sy0)) => foot + (sy0 - (x - sx0).abs() / 2 - y).clamp(0, top - foot),
            None => foot,
        })
    }

    /// How much of the light of field `f` falls on a pixel of it at height
    /// `pixel_z`, None: none. Not on what stands above the light or lies
    /// far below it, and on a roof only from a light as high as it (the
    /// slopes of a roof come down lower than a candle in the house stands).
    fn share(&self, f: &Field, pixel_z: Option<i32>) -> Option<f32> {
        let Some((foot, top)) = self.height else { return Some(1.0) };
        // (a flat plate on the ground beside lava, marked as roof in the
        // game's data, is floor to the lava's light: unlit it showed as a
        // dark square in the floor warm in the glow)
        let floor_by_flat_light = f.soft && top == foot && foot <= f.z;
        if top < f.z - BELOW || (self.roof && foot >= f.z && !floor_by_flat_light) {
            return None;
        }
        // (in the room under the floor the light stands on)
        if foot < f.floor && top <= f.floor {
            return None;
        }
        if f.soft {
            // (a flat light fades with the height above its ground, which
            // lies `FLAT_REACH` below its top; the height of this very
            // pixel: a tall rock is lit at its foot and less higher up, and
            // the pieces of a wall of rock side by side meet without a seam)
            let above = pixel_z.unwrap_or(foot) - (f.top - FLAT_REACH) - SOFT_FULL;
            if above >= SOFT_FADE {
                return None;
            }
            return Some((1.0 - above.max(0) as f32 / SOFT_FADE as f32).min(1.0));
        }
        (foot < f.top).then_some(1.0)
    }
}

/// For pixels no light reaches: per brightness of the pixel (luma) and
/// colour, the value each original value becomes in this general light
/// (as `apply` works it out: `v * min(dim, MOST)`), 256 x 3 x 256 values.
/// The last few tables are kept: the general light only changes slowly.
fn unlit_table(look: Look) -> std::sync::Arc<Vec<u8>> {
    static KEPT: std::sync::Mutex<Vec<([u32; 3], std::sync::Arc<Vec<u8>>)>> = std::sync::Mutex::new(Vec::new());
    let key = look.ambient.map(f32::to_bits);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, t)) = kept.iter().find(|(k, _)| *k == key) {
        return t.clone();
    }
    let mut t = vec![0u8; 256 * 768];
    for luma in 0..256 {
        for c in 0..3 {
            let k = dim_factor(look.ambient[c], luma).min(MOST);
            for v in 0..256 {
                t[luma * 768 + c * 256 + v] = ((v as f32 * k) as u32).min(255) as u8;
            }
        }
    }
    let t = std::sync::Arc::new(t);
    kept.push((key, t.clone()));
    if kept.len() > 4 {
        kept.remove(0);
    }
    t
}

/// What is known of things being in a room under a roof: the map, and by
/// item id where the thing stood when looked at, and the answer.
pub type RoomsSeen = (usize, HashMap<u16, ((i32, i32, i32), bool)>);

/// In `rooms` of `apply`: a thing in a house, or outside.
pub const IN_HOUSE: u8 = 2;
pub const OUTSIDE: u8 = 1;

/// For the things painted (by index into the items): does it stand in a
/// room under a roof (`IN_HOUSE`, `OUTSIDE`)? The answers are kept
/// (`seen`) while the thing stays where it is; new ones only as far as
/// `ROOM_PROBES` looks at the map per picture, the others are not known
/// (0) until their turn comes some pictures later (no stutter after
/// loading or a change of map).
pub fn rooms(painted: &[crate::view::Painted], map: &crate::map::MapData, map_no: usize, infos: &[crate::shape::ShapeInfo], seen: &mut RoomsSeen) -> Vec<u8> {
    // The looks at the map are counted as they are made: a wall is looked
    // at along three ways of up to 40 steps (up to 120 looks), a thing
    // under a roof once. Counted as 8 for a wall and 400 per picture, the
    // first picture in a town after a change of map made thousands of
    // looks (7 ms here, about three times as long on the Steam Deck).
    // In a town each look takes about 4 microseconds (wide boxes among
    // many things); 150 take about half a millisecond, and a town is
    // known after some 40 pictures, while the picture fades in after the
    // change.
    const ROOM_PROBES: usize = 150;
    if seen.0 != map_no {
        *seen = (map_no, HashMap::default());
    }
    let mut out = vec![0u8; map.items.len()];
    let probes = std::cell::Cell::new(0);
    for p in painted {
        let Some(it) = map.items.get(p.index) else { continue };
        let at = (it.x, it.y, it.z);
        let known = seen.1.get(&it.id).filter(|(was, _)| *was == at).map(|&(_, inside)| inside);
        let inside = match known {
            Some(inside) => Some(inside),
            None if probes.get() < ROOM_PROBES => {
                let inside = inside_house(map, infos, it, &probes);
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

/// Is a thing inside a house? Of a wall or post the side seen counts,
/// anything else is inside if under a roof.
/// `probes` counts the looks at the map.
fn inside_house(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item, probes: &std::cell::Cell<usize>) -> bool {
    match wall_facing(it, infos) {
        Some(facing) => faces_room(map, infos, it, facing, probes),
        // a post or corner stone: the sides seen (not those a wall goes on from)
        None if is_post(it, infos) => {
            let open: Vec<_> = [(1, 0), (0, 1)].into_iter().filter(|&side| !walled(map, infos, it, side, probes)).collect();
            if open.is_empty() {
                under_roof(map, infos, it, probes)
            } else {
                open.into_iter().all(|side| faces_room(map, infos, it, side, probes))
            }
        }
        None => under_roof(map, infos, it, probes),
    }
}

/// For the test command `room`: whether a thing counts as inside a house
/// for the lighting, and by which rule.
pub fn explain_room(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], id: u16) -> String {
    let Some(it) = map.get(id) else { return format!("no item {id}") };
    let probes = std::cell::Cell::new(0);
    let inside = inside_house(map, infos, it, &probes);
    let none = std::cell::Cell::new(0);
    let how = if let Some(facing) = wall_facing(it, infos) {
        format!("wall, side seen {facing:?}")
    } else if is_post(it, infos) {
        "post".to_string()
    } else if under_roof(map, infos, it, &none) {
        "under a roof".to_string()
    } else {
        "under the open sky".to_string()
    };
    format!("room {id} shape {} at {},{},{}: {} ({how}, {} looks)", it.shape, it.x, it.y, it.z, if inside { "inside" } else { "outside" }, probes.get())
}

/// Does a wall or post go on right next to this side of a post (a post in
/// the middle of a wall: that side is not seen)?
fn walled(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item, (dx, _): (i32, i32), probes: &std::cell::Cell<usize>) -> bool {
    let Some(si) = infos.get(it.shape as usize) else { return false };
    let b = crate::map::WorldBox::of(it, si);
    // a thin box just beyond that side, a little shorter than it
    let probe = if dx != 0 {
        crate::map::WorldBox { x: b.x + 8, y: b.y - 4, z: it.z + 8, xd: 8, yd: (b.yd - 8).max(1), zd: 24 }
    } else {
        crate::map::WorldBox { x: b.x - 4, y: b.y + 8, z: it.z + 8, xd: (b.xd - 8).max(1), yd: 8, zd: 24 }
    };
    probes.set(probes.get() + 1);
    blocked_by_wall(map, infos, &map.position_info(&probe, &NOWHERE, infos, it.id))
}

/// A start for `position_info` that nothing overlaps.
const NOWHERE: crate::map::WorldBox = crate::map::WorldBox { x: 0, y: 0, z: -10000, xd: 0, yd: 0, zd: 0 };

/// Was a box looked at blocked by a wall or post?
fn blocked_by_wall(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], pi: &crate::map::PositionInfo) -> bool {
    !pi.valid && pi.blocker.and_then(|id| map.get(id)).is_some_and(|o| wall_facing(o, infos).is_some() || is_post(o, infos))
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

/// A post or the corner stone of walls: solid, tall, small across (not a
/// standing candlestick or lamp: that is inside if under a roof).
fn is_post(it: &crate::map::Item, infos: &[crate::shape::ShapeInfo]) -> bool {
    light_of(it.shape, it.frame).is_none() && infos.get(it.shape as usize).is_some_and(|si| {
        let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
        si.solid && !si.roof && zd >= 32 && xd <= 32 && yd <= 32
    })
}

/// Does the seen side of a wall face a room? From just in front of it,
/// going straight out and to both sides along it under the roof: a room
/// if each way meets another wall before the roof ends (a porch or the
/// eaves are open to one side: that is outside). Walls right next to it
/// do not count (the stones of the same pillar, a fireplace in front).
fn faces_room(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item, (dx, dy): (i32, i32), probes: &std::cell::Cell<usize>) -> bool {
    const STEP: i32 = 32;
    const STEPS: i32 = 40;
    let Some(si) = infos.get(it.shape as usize) else { return false };
    let b = crate::map::WorldBox::of(it, si);
    // the middle of the wall's foot, moved out to just in front of it
    let (mx, my) = (b.x - b.xd / 2 + dx * (b.xd / 2 + 64), b.y - b.yd / 2 + dy * (b.yd / 2 + 64));
    // how far to the next wall, None: out under the open sky
    let closed = |(wx, wy): (i32, i32)| -> Option<i32> {
        // open sky a few steps in a row (not just a chimney above)
        let mut open = 0;
        for k in 0..STEPS {
            let (cx, cy) = (mx + wx * k * STEP, my + wy * k * STEP);
            // wide across the way (not through a doorway)
            let (hx, hy) = if wx != 0 { (16, 48) } else { (48, 16) };
            let probe = crate::map::WorldBox { x: cx + hx, y: cy + hy, z: it.z + 8, xd: 2 * hx, yd: 2 * hy, zd: 24 };
            probes.set(probes.get() + 1);
            let pi = map.position_info(&probe, &NOWHERE, infos, it.id);
            if pi.roof_z.is_none() {
                open += 1;
                if open >= 5 {
                    return None;
                }
            } else {
                open = 0;
            }
            if k >= 2 && blocked_by_wall(map, infos, &pi) {
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
fn under_roof(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], it: &crate::map::Item, probes: &std::cell::Cell<usize>) -> bool {
    let Some(si) = infos.get(it.shape as usize) else { return false };
    probes.set(probes.get() + 1);
    let b = crate::map::WorldBox::of(it, si);
    let probe = crate::map::WorldBox { x: b.x - b.xd / 2 + 16, y: b.y - b.yd / 2 + 16, z: it.z, xd: 32, yd: 32, zd: 16 };
    map.position_info(&probe, &probe, infos, it.id).roof_z.is_some()
}

/// The shadows of walls for `apply`: for each light (by its place in the
/// list) the walls about it (None: nothing is in the way, as for lava and
/// fire fields), and where a point of the picture lies in the world.
pub struct Shadows<'a> {
    pub sights: Vec<Option<Sight>>,
    pub to_world: &'a (dyn Fn(i32, i32, i32) -> (i32, i32) + Sync),
    /// Where the world's point 0, 0, 0 is in the picture (to keep the
    /// grid in step with the world).
    pub anchor: (i32, i32),
    /// What was found out in the pictures before (see `ShadeCache`).
    pub cache: &'a mut ShadeCache,
}

/// Which grid points the lights reached in the pictures before. The grid
/// is fixed in the world, so a point keeps its answer while neither the
/// light, nor the walls about it, nor what is seen at the point change
/// (also while the view scrolls): only new points are looked at.
#[derive(Default)]
pub struct ShadeCache {
    lights: Vec<CachedLight>,
}

/// The answers for one light, on a part of the grid fixed in the world.
struct CachedLight {
    from: (i32, i32),
    z: i32,
    /// The walls about it, as they were (`Sight::key`).
    key: SightKey,
    /// The part of the grid (in grid points of the world): first point, size.
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    /// What was seen at each point, and whether the light reached it.
    points: Vec<Option<(Target, bool)>>,
}

/// The grid points one light reaches in this picture (1) or not (0).
struct Lit {
    gx0: i32,
    gy0: i32,
    w: i32,
    h: i32,
    v: Vec<u8>,
}

impl Lit {
    fn at(&self, gx: i32, gy: i32) -> f32 {
        let (x, y) = (gx - self.gx0, gy - self.gy0);
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 1.0;
        }
        self.v[(y * self.w + x) as usize] as f32
    }
}

/// The squares of the grid (of `tw` x `th` points; a square by its top
/// left point) with a corner in the shadow of some light (empty without
/// shadows). Worked out once here, not for every pixel.
fn shadow_squares(lit: &[Option<Lit>], tw: i32, th: i32) -> Vec<bool> {
    if lit.is_empty() {
        return Vec::new();
    }
    let mut squares = vec![false; (tw * th) as usize];
    for v in lit.iter().flatten() {
        for (k, &reached) in v.v.iter().enumerate() {
            if reached == 0 {
                // the four squares this point is a corner of
                let (gx, gy) = (v.gx0 + k as i32 % v.w, v.gy0 + k as i32 / v.w);
                for (x, y) in [(gx, gy), (gx - 1, gy), (gx, gy - 1), (gx - 1, gy - 1)] {
                    if x >= 0 && y >= 0 {
                        squares[(y * tw + x) as usize] = true;
                    }
                }
            }
        }
    }
    squares
}

/// For each light with walls about it: the grid points within its reach
/// that it lights. The answers of the pictures before are used again
/// where nothing changed.
fn lit_points(lights: &[Light], sights: Vec<Option<Sight>>, targets: &[Option<Target>], (tw, th): (i32, i32), (ox, oy): (i32, i32), cache: &mut ShadeCache) -> Vec<Option<Lit>> {
    let mut old = std::mem::take(&mut cache.lights);
    let jobs: Vec<_> = lights
        .iter()
        .zip(sights)
        .map(|(l, sight)| {
            let sight = sight?;
            // the grid points within reach (as `fields` goes over them)
            let (gx0, gx1, gy0, gy1) = l.grid_reach();
            let (gx0, gx1, gy0, gy1) = (gx0.max(0), gx1.min(tw - 1), gy0.max(0), gy1.min(th - 1));
            if gx1 < gx0 || gy1 < gy0 {
                return None;
            }
            // the answers kept for this light, if it and its walls are as they were
            let i = old.iter().position(|c| c.from == sight.from && c.z == l.z && c.key == sight.key);
            let kept = i.map(|i| old.swap_remove(i));
            Some((sight, (gx0, gy0, gx1, gy1), kept, l.z))
        })
        .collect();
    // the answers kept for each light (or a new part of the grid for them)
    let mut sights = Vec::new();
    let mut parts = Vec::new();
    for (n, job) in jobs.into_iter().enumerate() {
        let Some((sight, (gx0, gy0, gx1, gy1), kept, z)) = job else { continue };
        let (w, h) = (gx1 - gx0 + 1, gy1 - gy0 + 1);
        // the part of the world's grid kept: this reach and some room
        // around it (the reach of a flickering light changes a little)
        let (wx0, wy0, wx1, wy1) = (gx0 + ox, gy0 + oy, gx1 + ox, gy1 + oy);
        let c = match kept {
            Some(c) if wx0 >= c.x0 && wy0 >= c.y0 && wx1 < c.x0 + c.w && wy1 < c.y0 + c.h => c,
            _ => {
                const ROOM: i32 = 8;
                let (cw, ch) = (w + 2 * ROOM, h + 2 * ROOM);
                CachedLight { from: sight.from, z, key: sight.key.clone(), x0: wx0 - ROOM, y0: wy0 - ROOM, w: cw, h: ch, points: vec![None; (cw * ch) as usize] }
            }
        };
        sights.push(sight);
        parts.push((n, c, Lit { gx0, gy0, w, h, v: vec![1u8; (w * h) as usize] }));
    }
    // Row by row over the processor cores (by light, the one with the
    // widest reach kept one core busy while the others waited): each row
    // of a light with its row of the answers kept.
    let mut rows = Vec::new();
    for (k, (_, c, lit)) in parts.iter_mut().enumerate() {
        let first = (lit.gy0 + oy - c.y0) as usize;
        let left = (lit.gx0 + ox - c.x0) as usize;
        let kept_rows = c.points.chunks_mut(c.w as usize).skip(first);
        for (r, (row, kept)) in lit.v.chunks_mut(lit.w as usize).zip(kept_rows).enumerate() {
            rows.push((k, lit.gy0 + r as i32, lit.gx0, row, &mut kept[left..left + lit.w as usize]));
        }
    }
    let band = (rows.len() / (4 * crate::pool::cores())).max(1);
    let mut bands = Vec::new();
    while !rows.is_empty() {
        let rest = rows.split_off(band.min(rows.len()));
        bands.push(std::mem::replace(&mut rows, rest));
    }
    crate::pool::each(bands, |_, band| {
        for (k, gy, gx0, row, kept) in band {
            let sight = &sights[k];
            for (i, (lit, slot)) in row.iter_mut().zip(kept.iter_mut()).enumerate() {
                let Some(Some(t)) = targets.get((gy * tw + gx0 + i as i32) as usize).copied() else { continue };
                let reached = match *slot {
                    Some((was, r)) if was == t => r,
                    _ => {
                        let r = sight.sees(t);
                        *slot = Some((t, r));
                        r
                    }
                };
                if !reached {
                    *lit = 0;
                }
            }
        }
    });
    let mut out: Vec<Option<Lit>> = (0..lights.len()).map(|_| None).collect();
    for (n, c, lit) in parts {
        cache.lights.push(c);
        out[n] = Some(lit);
    }
    out
}

/// The squares of a `Sight` (x0, y0, n) and the boxes of the walls laid
/// into them: the same key, the same `tops` (a few dozen numbers to
/// compare instead of the thousands of squares).
#[derive(Clone, PartialEq)]
struct SightKey {
    squares: (i32, i32, i32),
    walls: Vec<(i32, i32, i32, i32, i32, i32)>,
}

/// Size of a square of `Sight`, in world units (a wall is 32 thick).
const SIGHT_CELL: i32 = 8;

/// What stands in the way of one light: the squares around it covered by
/// a wall at its height, with the height of the wall's top (a wall only
/// shades what lies lower than its top: the walk on top of a city wall is
/// lit by a lamp hanging on the wall below it).
pub struct Sight {
    /// The world position of the first square, and squares per side.
    x0: i32,
    y0: i32,
    n: i32,
    tops: Vec<i32>,
    /// What `tops` was made from (to know it again, see `ShadeCache`).
    key: SightKey,
    /// Where the light's lines start (see `sights`).
    from: (i32, i32),
    /// The world box around all walls (x0, y0, x1, y1): a line that stays
    /// outside it meets none (most lines in an open square).
    walls: (i32, i32, i32, i32),
}

impl Sight {
    /// Does a wall higher than `z` stand at this point (as far as known)?
    fn wall_at(&self, x: i32, y: i32, z: i32) -> bool {
        let (cx, cy) = ((x - self.x0).div_euclid(SIGHT_CELL), (y - self.y0).div_euclid(SIGHT_CELL));
        cx >= 0 && cy >= 0 && cx < self.n && cy < self.n && self.tops[(cy * self.n + cx) as usize] > z
    }

    /// Is a point reached by the light at `from`, with no wall between?
    fn sees(&self, t: Target) -> bool {
        let (lx, ly) = self.from;
        // (the last 12 units before a wall or thing are its own: its foot)
        let end = if matches!(t, Target::Point(..)) { 0.0 } else { 12.0 };
        let (tx, ty, tz) = match t {
            Target::Point(x, y, z) => (x, y, z),
            // of a wall only the side seen is lit, by lights on that side
            Target::Face(x, y, z, across_x, at) => {
                if (if across_x { lx } else { ly }) < at {
                    return false;
                }
                (x, y, z)
            }
            // another thing: its side towards the light
            Target::Box(x0, y0, x1, y1, z) => (lx.clamp(x0, x1), ly.clamp(y0, y1), z),
            // a post: as a wall, only its sides seen (+x, +y) are lit, by
            // lights in front of them (not from inside a house through the
            // corner of its walls)
            // (judged by the side the point is on: of a post in the middle
            // of a wall only the side towards the street is seen, and a
            // fire in the house further along it is not in front of it)
            // (a side that a wall goes on from is not seen: its pixels at the
            // edge belong to the other side)
            Target::Post(x0, y0, x1, y1, z, side) => {
                let open_x = !self.wall_at(x1 + SIGHT_CELL / 2, (y0 + y1) / 2, z);
                let open_y = !self.wall_at((x0 + x1) / 2, y1 + SIGHT_CELL / 2, z);
                let (front_x, front_y) = (open_x && lx >= x1, open_y && ly >= y1);
                let lit = match side {
                    Some(true) if open_x => front_x,
                    Some(false) if open_y => front_y,
                    _ => front_x || front_y,
                };
                if !lit {
                    return false;
                }
                (lx.clamp(x0, x1), ly.clamp(y0, y1), z)
            }
        };
        // The squares the line from the light to the point crosses, one
        // after the other (each once); the last `end` units are the
        // point's. (Skipping the first 24 units as the light's own let the
        // light of a lamp on a wall slip through the wall beside it, onto
        // the floor inside by the door.)
        let (wx0, wy0, wx1, wy1) = self.walls;
        if lx.max(tx) < wx0 || lx.min(tx) > wx1 || ly.max(ty) < wy0 || ly.min(ty) > wy1 {
            return true;
        }
        let (dx, dy) = ((tx - lx) as f32, (ty - ly) as f32);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= end + 1.0 {
            return true;
        }
        let t1 = (len - end) / len;
        let cell = SIGHT_CELL as f32;
        let (fx, fy) = ((lx - self.x0) as f32 / cell, (ly - self.y0) as f32 / cell);
        let (mut cx, mut cy) = (fx.floor() as i32, fy.floor() as i32);
        let (ux, uy) = (dx / cell, dy / cell);
        let (sx, sy) = (if ux > 0.0 { 1 } else { -1 }, if uy > 0.0 { 1 } else { -1 });
        // how far along the line (0..1) the next border in x and in y is, and between borders
        let next = |f: f32, u: f32, c: i32| if u > 0.0 { (c as f32 + 1.0 - f) / u } else if u < 0.0 { (c as f32 - f) / u } else { f32::MAX };
        let (mut nx, mut ny) = (next(fx, ux, cx), next(fy, uy, cy));
        let (stx, sty) = (if ux != 0.0 { 1.0 / ux.abs() } else { f32::MAX }, if uy != 0.0 { 1.0 / uy.abs() } else { f32::MAX });
        let mut t = 0.0f32;
        while t < t1 {
            let leave = nx.min(ny);
            if leave > 0.0 {
                if cx < 0 || cy < 0 || cx >= self.n || cy >= self.n {
                    return true;
                }
                if self.tops[(cy * self.n + cx) as usize] > tz {
                    return false;
                }
            }
            t = leave;
            if nx < ny {
                cx += sx;
                nx += stx;
            } else {
                cy += sy;
                ny += sty;
            }
        }
        true
    }
}

/// Does a thing cast a shadow for a light at height `z`: a wall or a post,
/// standing at that height (not a person, a lamp or furniture)? Its box if
/// so.
fn casts_shadow(it: &crate::map::Item, infos: &[crate::shape::ShapeInfo], z: i32) -> Option<crate::map::WorldBox> {
    if it.is_actor() || !it.in_world() || light_of(it.shape, it.frame).is_some() {
        return None;
    }
    let si = infos.get(it.shape as usize)?;
    let b = crate::map::WorldBox::of(it, si);
    let wall = wall_facing(it, infos).is_some() || is_post(it, infos);
    (wall && it.z <= z && it.z + b.zd > z).then_some(b)
}

/// Is a post part of a wall (a wall touches it): only then it casts a
/// shadow (the corner of a house), not standing alone (the pole of a
/// street lamp, a turret on the city wall, the bars of a cell).
fn in_wall(post: &crate::map::WorldBox, walls: &[crate::map::WorldBox]) -> bool {
    walls.iter().any(|w| w.x + 8 > post.x - post.xd && w.x - w.xd < post.x + 8 && w.y + 8 > post.y - post.yd && w.y - w.yd < post.y + 8)
}

/// The walls around each light (by its place in the list), for
/// `Shadows`. Lava and fire fields (flat lights) get none.
pub fn sights(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], lights: &[Light]) -> Vec<Option<Sight>> {
    // (the lights spread over the processor cores)
    crate::pool::map(lights.iter().collect(), |l| sight(map, infos, l))
}

/// The walls about one light (None: none, or a flat light).
fn sight(map: &crate::map::MapData, infos: &[crate::shape::ShapeInfo], l: &Light) -> Option<Sight> {
    if l.soft {
        return None;
    }
    // (a pixel to the side is 4 world units; rounded up to whole
    // steps: the reach of a flickering light changes a little from
    // picture to picture, the walls looked at stay the same, and so
    // do the answers kept in `ShadeCache`)
    let reach = ((4.0 * l.radius) as i32 + 64 + 63) / 64 * 64;
    let n = 2 * reach / SIGHT_CELL + 2;
    // the squares lie fixed in the world (a flickering light changes
    // its reach from picture to picture: squares laid out from it
    // moved, and the edges of its shadows with them)
    let (x0, y0) = ((l.wx - reach).div_euclid(SIGHT_CELL) * SIGHT_CELL, (l.wy - reach).div_euclid(SIGHT_CELL) * SIGHT_CELL);
    let mut tops = vec![i32::MIN; (n * n) as usize];
    let mut any = false;
    let mut around = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let near = map.items_near(l.wx, l.wy, reach + 128);
    let (mut walls, mut posts) = (Vec::new(), Vec::new());
    for &i in &near {
        let Some(it) = map.items.get(i) else { continue };
        let Some(b) = casts_shadow(it, infos, l.z) else { continue };
        if wall_facing(it, infos).is_some() { walls.push(b) } else { posts.push(b) }
    }
    posts.retain(|p| in_wall(p, &walls));
    let mut key = SightKey { squares: (x0, y0, n), walls: Vec::with_capacity(walls.len() + posts.len()) };
    for b in walls.iter().chain(&posts) {
        key.walls.push((b.x, b.y, b.z, b.xd, b.yd, b.zd));
        // the squares whose middle lies in the box (not wider than the wall)
        let first = |lo: i32| (lo - SIGHT_CELL / 2).div_euclid(SIGHT_CELL) + 1;
        let last = |hi: i32| (hi - SIGHT_CELL / 2).div_euclid(SIGHT_CELL);
        let (cx0, cx1) = (first(b.x - b.xd - x0).max(0), last(b.x - x0).min(n - 1));
        let (cy0, cy1) = (first(b.y - b.yd - y0).max(0), last(b.y - y0).min(n - 1));
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                around = (around.0.min(x0 + cx * SIGHT_CELL), around.1.min(y0 + cy * SIGHT_CELL), around.2.max(x0 + (cx + 1) * SIGHT_CELL), around.3.max(y0 + (cy + 1) * SIGHT_CELL));
                let top = &mut tops[(cy * n + cx) as usize];
                *top = (*top).max(b.z + b.zd);
                any = true;
            }
        }
    }
    // The start of its lines: a lamp hangs flat on a wall, its middle
    // close to it. The start is moved away from the wall, to the side
    // the lamp hangs on, so that lines along the wall do not graze
    // it (nor slip through it beside the lamp).
    let wall_at = |x: i32, y: i32| {
        let (cx, cy) = ((x - x0).div_euclid(SIGHT_CELL), (y - y0).div_euclid(SIGHT_CELL));
        cx >= 0 && cy >= 0 && cx < n && cy < n && tops[(cy * n + cx) as usize] > l.z
    };
    let mut from = (l.wx, l.wy);
    if let Some(across_x) = l.thin {
        for d in [-1, 1] {
            let hit = (8..=24).step_by(8).any(|k| if across_x { wall_at(l.wx + d * k, l.wy) } else { wall_at(l.wx, l.wy + d * k) });
            if hit {
                from = if across_x { (l.wx - d * 24, l.wy) } else { (l.wx, l.wy - d * 24) };
                break;
            }
        }
    }
    any.then_some(Sight { x0, y0, n, tops, key, from, walls: around })
}

/// Where a point of the picture lies in the world, for the shadows.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// On a floor or the ground (x, y, height of its top).
    Point(i32, i32, i32),
    /// On the side seen of a wall: (x, y, height of its foot), the wall
    /// across x (true) or y, and the x or y of that side.
    Face(i32, i32, i32, bool, i32),
    /// On another thing: its box (x0, y0, x1, y1) and the height of its foot.
    Box(i32, i32, i32, i32, i32),
    /// On a post (or the corner stone of walls): as `Box`, and the side
    /// this point is on: facing +y (Some(false), left of its front corner
    /// in the picture), +x (Some(true)) or not known (None, at the edge).
    Post(i32, i32, i32, i32, i32, Option<bool>),
}

/// The points of the grid (as `targets`) within reach of a light with
/// walls about it: only there it matters what is seen.
fn in_reach(lights: &[Light], sights: &[Option<Sight>], w: i32, h: i32, (dx, dy): (i32, i32)) -> Vec<bool> {
    let (tw, th) = ((w + dx) / CELL + 2, (h + dy) / CELL + 2);
    let mut wanted = vec![false; (tw * th) as usize];
    for (l, _) in lights.iter().zip(sights).filter(|(_, s)| s.is_some()) {
        let (gx0, gx1, gy0, gy1) = l.grid_reach();
        let (gx0, gx1) = (gx0.max(0), gx1.min(tw - 1));
        for gy in gy0.max(0)..=gy1.min(th - 1) {
            if gx0 <= gx1 {
                wanted[(gy * tw + gx0) as usize..=(gy * tw + gx1) as usize].fill(true);
            }
        }
    }
    wanted
}

/// For the points of the grid `wanted` (CELL pixels apart, with one more
/// row and column; pixel x is at x + dx in the grid), what the picture
/// shows there, in the world.
#[allow(clippy::too_many_arguments)]
fn targets(ids: &[u32], items: &[crate::map::Item], infos: &[crate::shape::ShapeInfo], w: i32, h: i32, (dx, dy): (i32, i32), to_world: &(dyn Fn(i32, i32, i32) -> (i32, i32) + Sync), wanted: &[bool]) -> Vec<Option<Target>> {
    let (tw, th) = ((w + dx) / CELL + 2, (h + dy) / CELL + 2);
    let mut out = vec![None; (tw * th) as usize];
    // (rows of the grid spread over the processor cores)
    let band = crate::pool::band(th);
    let parts: Vec<_> = out.chunks_mut((band * tw) as usize).enumerate().map(|(b, rows)| (b as i32 * band, rows)).collect();
    crate::pool::each(parts, |_, (gy0, rows): (i32, &mut [Option<Target>])| {
        for (r, row) in rows.chunks_mut(tw as usize).enumerate() {
            let gy = gy0 + r as i32;
            for gx in 0..tw {
                let (sx, sy) = (gx * CELL - dx, gy * CELL - dy);
                if sx < 0 || sy < 0 || sx >= w || sy >= h || !wanted[(gy * tw + gx) as usize] {
                    continue;
                }
                let Some(it) = ids.get((sy * w + sx) as usize).and_then(|&id| id.checked_sub(1)).and_then(|i| items.get(i as usize)) else { continue };
                let Some(si) = infos.get(it.shape as usize) else { continue };
                let b = crate::map::WorldBox::of(it, si);
                // (to_world at height z: x = x0 + 4z and y = y0 + 4z, with
                // (x0, y0) the point at height 0)
                row[gx as usize] = Some(match wall_facing(it, infos) {
                    // the side seen, at the height this pixel shows
                    Some((1, _)) => {
                        let (x0, _) = to_world(sx, sy, 0);
                        let (_, y) = to_world(sx, sy, (it.x - x0) / 4);
                        Target::Face(it.x, y, it.z, true, it.x)
                    }
                    Some(_) => {
                        let (_, y0) = to_world(sx, sy, 0);
                        let (x, _) = to_world(sx, sy, (it.y - y0) / 4);
                        Target::Face(x, it.y, it.z, false, it.y)
                    }
                    // a post (many count as ground too: one can stand on them)
                    // (the side: along a line of sight x - y stays the same,
                    // and is x1 - y1 at the front corner)
                    None if is_post(it, infos) => {
                        let (x, y) = to_world(sx, sy, 0);
                        let side = match (x - y).cmp(&(b.x - b.y)) {
                            std::cmp::Ordering::Less => Some(false),
                            std::cmp::Ordering::Greater => Some(true),
                            std::cmp::Ordering::Equal => None,
                        };
                        Target::Post(b.x - b.xd, b.y - b.yd, b.x, b.y, it.z, side)
                    }
                    // a floor or the ground: its top
                    None if si.land => {
                        let (x, y) = to_world(sx, sy, it.z + b.zd);
                        Target::Point(x, y, it.z + b.zd)
                    }
                    None => Target::Box(b.x - b.xd, b.y - b.yd, b.x, b.y, it.z),
                });
            }
        }
    });
    out
}
