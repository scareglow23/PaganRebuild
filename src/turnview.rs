//! The turnable view (optional): the world seen from any of the four
//! sides, turning smoothly from one to the next.
//!
//! The original graphics show everything from one side only, so turned
//! views are made up in a mixed way:
//!
//! - walls, buildings and the ground become blocks of little cubes (the
//!   item's bounding box), each cube coloured with the pixel it falls on
//!   in the original picture;
//! - things whose picture is not like a box (trees, bushes, rocks,
//!   lanterns) stay flat pictures standing upright, facing the viewer;
//! - people and creatures show their picture for the turned direction.
//!
//! Seen from the original side (no turn) the game draws as always
//! (view.rs); this renderer is only used while the view is turned.

use crate::hash::HashMap;
use crate::map::{Item, FLG_FLIPPED, FLG_INVISIBLE};
use crate::palette::Palette;
use crate::shape::{Frame, Shape, ShapeInfo, Shapes, TRANSPARENT};
use crate::view::{Painted, Surface};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Size of a cube in world units (x and y; z). One unit of height is one
/// pixel row of the picture: with taller cubes every other row of a wall's
/// face would be lost, and its stones turn into streaks.
const XY: i32 = 4;
const Z: i32 = 1;
/// Things other than walls make cubes only when their box covers at
/// least this share of their picture ...
const MIN_COVER: f32 = 0.9;
/// ... and at least this share of the box outline is drawn.
const MIN_FILL: f32 = 0.85;
/// How long a quarter turn takes.
const TURN_TIME: Duration = Duration::from_millis(450);

/// A cube of an item, relative to the item's position (the far corner of
/// its box, at its bottom).
#[derive(Clone, Copy)]
struct Cube {
    dx: i16,
    dy: i16,
    dz: i16,
    colour: u8,
}

enum Model {
    /// The cubes on the outside of the item's box.
    Cubes(Vec<Cube>),
    /// A flat picture standing upright through the middle of the box;
    /// `offset`: where the picture's anchor is relative to the middle of
    /// the box's bottom, on screen.
    /// `middle`: the average column of its drawn pixels, relative to the
    /// anchor (where its bulk is, sideways).
    Upright { offset: (i32, i32), middle: f32 },
}

/// Where the view is looking from and how it is turned.
#[derive(Clone, Copy)]
pub struct Xform {
    /// The world point in the middle of the view.
    pub cam: (i32, i32, i32),
    /// Turn in degrees (0: as in the original).
    pub angle: f32,
    pub size: (i32, i32),
}

impl Xform {
    fn turned(&self) -> bool {
        self.angle != 0.0
    }

    fn sin_cos(&self) -> (f32, f32) {
        self.angle.to_radians().sin_cos()
    }

    /// World position to view position.
    pub fn to_screen(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        let (cx, cy, cz) = self.cam;
        if !self.turned() {
            // the usual isometric projection, exactly as ScummVM's renderer has it
            let (ox, oy) = ((cx - cy) / 4 - self.size.0 / 2, (cx + cy) / 8 - cz - self.size.1 / 2);
            return ((x - y) / 4 - ox, (x + y) / 8 - z - oy);
        }
        let (sx, sy) = self.projected(x as f64, y as f64, z as f64);
        let (ox, oy) = self.origin();
        ((sx - ox).floor() as i32, (sy - oy).floor() as i32)
    }

    /// The world position (x, y) at height `z` seen at a view position.
    pub fn to_world(&self, sx: i32, sy: i32, z: i32) -> (i32, i32) {
        let (cx, cy, cz) = self.cam;
        if !self.turned() {
            let (ox, oy) = ((cx - cy) / 4 - self.size.0 / 2, (cx + cy) / 8 - cz - self.size.1 / 2);
            let (ax, ay) = (sx + ox, sy + oy);
            return (2 * ax + 4 * (ay + z), 4 * (ay + z) - 2 * ax);
        }
        let (ox, oy) = self.origin();
        let (u, v) = (sx as f64 + ox, sy as f64 + oy + z as f64);
        let (rx, ry) = (2.0 * u + 4.0 * v, 4.0 * v - 2.0 * u);
        let (s, c) = self.angle.to_radians().sin_cos();
        let (s, c) = (s as f64, c as f64);
        // the inverse turn
        ((c * rx + s * ry).round() as i32, (-s * rx + c * ry).round() as i32)
    }

    /// Where a world point lands, turned, before the view is moved into
    /// place (in pixels, about the world's origin).
    fn projected(&self, x: f64, y: f64, z: f64) -> (f64, f64) {
        let (s, c) = self.angle.to_radians().sin_cos();
        let (s, c) = (s as f64, c as f64);
        let (rx, ry) = (c * x - s * y, s * x + c * y);
        ((rx - ry) / 4.0, (rx + ry) / 8.0 - z)
    }

    /// The shift that puts the camera point in the middle of the view: a
    /// whole number of pixels, so that while the view scrolls every
    /// little cube keeps its place in the pixel grid (else they would
    /// shimmer, each rounding to a pixel at another moment).
    fn origin(&self) -> (f64, f64) {
        let (cx, cy, cz) = self.cam;
        let (sx, sy) = self.projected(cx as f64, cy as f64, cz as f64);
        (sx.floor() - (self.size.0 / 2) as f64, sy.floor() - (self.size.1 / 2) as f64)
    }

    /// The nearest quarter turn (0..=3).
    pub fn quarter(&self) -> i32 {
        (self.angle / 90.0).round().rem_euclid(4.0) as i32
    }
}

/// The offsets at distance `r` around a point (r = 0: the point itself).
fn ring(r: i32) -> Vec<(i32, i32)> {
    if r == 0 {
        return vec![(0, 0)];
    }
    (-r..=r).flat_map(|a| (-r..=r).map(move |b| (a, b))).filter(|&(a, b)| a.abs().max(b.abs()) == r).collect()
}

#[derive(Default)]
pub struct TurnView {
    models: HashMap<(u32, u32, bool), Rc<Model>>,
    /// Per shape: its frames are the thing seen from eight directions (see `turns_with_view`).
    directions: HashMap<u32, bool>,
    /// Quarter turns the view is turned to (0..=3).
    pub target: i32,
    /// The turn under way: from which angle, since when.
    anim: Option<(f32, Instant)>,
    /// For tests: an angle held fixed.
    pub fixed: Option<f32>,
    /// Per pixel of the last picture: depth and the item drawn there
    /// (index into the map's items + 1), and the one behind it.
    depth: Vec<f32>,
    ids: Vec<u32>,
    behind: Vec<u32>,
    width: i32,
}

/// What the turned renderer needs to know about the scene.
pub struct Scene<'a> {
    pub items: &'a [Item],
    pub infos: &'a [ShapeInfo],
    pub pal: &'a Palette,
    pub z_limit: Option<i32>,
    pub show_editor_items: bool,
    /// An item to leave out (0: none).
    pub hide: u16,
    /// The picture (frame, flipped) of a person seen from the turned side.
    pub person_frame: &'a dyn Fn(&Item, i32) -> (u32, bool),
    /// The weapon in the Avatar's hand: (item index, shape, frame, dx, dy).
    pub overlay: Option<(usize, u32, u32, i32, i32)>,
}

impl TurnView {
    /// The angle to draw with now, in degrees.
    pub fn angle(&self) -> f32 {
        if let Some(a) = self.fixed {
            return a;
        }
        let target = 90.0 * self.target as f32;
        match self.anim {
            Some((from, start)) => {
                let t = (start.elapsed().as_secs_f32() / TURN_TIME.as_secs_f32()).min(1.0);
                // gently in and out
                let e = t * t * (3.0 - 2.0 * t);
                from + (target - from) * e
            }
            None => target,
        }
    }

    pub fn turning(&self) -> bool {
        self.anim.is_some_and(|(_, start)| start.elapsed() < TURN_TIME)
    }

    /// Ends a finished turn (the angle then stays at a quarter).
    pub fn settle(&mut self) {
        if self.anim.is_some() && !self.turning() {
            self.anim = None;
            self.target = self.target.rem_euclid(4);
        }
    }

    /// Starts turning the view by a quarter: +1 clockwise, -1 the other way.
    pub fn turn(&mut self, by: i32) {
        let now = self.angle();
        self.target += by;
        self.anim = Some((now, Instant::now()));
    }

    /// The Avatar's pictures changed (option "show armour"): what was built from them is stale.
    pub fn forget_models(&mut self) {
        self.models.retain(|k, _| k.0 != crate::outfit::AVATAR_SHAPE);
    }

    /// Back to the original side at once.
    pub fn reset(&mut self) {
        self.target = 0;
        self.anim = None;
    }

    /// The item drawn at a view position in the last turned picture
    /// (index into the map's items), the first one other than `skip`.
    pub fn pick(&self, x: i32, y: i32, skip: Option<usize>) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || self.width == 0 {
            return None;
        }
        let p = (y * self.width + x) as usize;
        let top = (*self.ids.get(p)? as usize).checked_sub(1);
        match top {
            Some(i) if Some(i) == skip => (*self.behind.get(p)? as usize).checked_sub(1),
            other => other,
        }
    }

    /// Middle of the pixels an item covers in the last turned picture.
    pub fn drawn_centre(&self, index: usize) -> Option<(i32, i32)> {
        let want = index as u32 + 1;
        let (mut sx, mut sy, mut n) = (0i64, 0i64, 0i64);
        for (p, &id) in self.ids.iter().enumerate() {
            if id == want {
                sx += p as i64 % self.width as i64;
                sy += p as i64 / self.width as i64;
                n += 1;
            }
        }
        (n > 0).then(|| ((sx / n) as i32, (sy / n) as i32))
    }

    /// Which item each pixel of the last turned picture shows (index + 1).
    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    fn model(&mut self, shapes: &mut Shapes, info: &ShapeInfo, shape: u32, frame: u32, flipped: bool) -> Option<Rc<Model>> {
        let key = (shape, frame, flipped);
        if let Some(m) = self.models.get(&key) {
            return Some(m.clone());
        }
        let s = shapes.get(shape)?;
        let f = s.frames.get(frame as usize)?;
        let (xd, yd, zd) = info.footpad(flipped);
        // The ground pictures leave their edges out (the next tile covers
        // them): there the nearest drawn pixel is used.
        let reach = if zd == 0 || info.land { 3 } else { 0 };
        let (grid, covered) = Grid::sample(f, flipped, (xd, yd, zd), reach);
        let opaque = f.pixels.iter().filter(|&&c| c != TRANSPARENT).count().max(1) as f32;
        let hit = f.pixels.iter().zip(&covered).filter(|(&c, &cov)| c != TRANSPARENT && cov).count() as f32;
        let inside = covered.iter().filter(|&&c| c).count().max(1) as f32;
        // Walls are marked as hiding what is behind them; they and the
        // ground make good cubes, anything else only if it fills its box.
        let boxy = hit >= MIN_COVER * opaque && hit >= MIN_FILL * inside;
        if std::env::var_os("PAGAN_MODELS").is_some() {
            eprintln!("model shape {shape} frame {frame} box {xd}x{yd}x{zd} cover {:.2} fill {:.2} occl {} land {} solid {}",
                      hit / opaque, hit / inside, info.occl, info.land, info.solid);
        }
        // Furniture one can put things on (tables, benches, beds) lies
        // within its box, legs and all: as cubes it keeps its shape and
        // place when turned. (Trees are much higher; a mushroom cap
        // reaches far beyond its box.)
        let furniture = info.land && zd <= 24 && hit >= MIN_COVER * opaque;
        let model = if info.occl || zd == 0 || (info.land && zd <= 8) || furniture || boxy {
            grid.cubes(xd, yd)
        } else {
            upright_model(f, flipped, xd, yd)
        };
        let m = Rc::new(model);
        self.models.insert(key, m.clone());
        Some(m)
    }

    /// Whether the frames of a shape are one thing seen from eight
    /// directions, one after the other (chairs): eight frames, where each
    /// of 1, 2, 3 is 7, 6, 5 mirrored (seen from the other side).
    pub fn turns_with_view(&mut self, shapes: &mut Shapes, shape: u32) -> bool {
        if let Some(&d) = self.directions.get(&shape) {
            return d;
        }
        let d = shapes.get(shape).is_some_and(|s| {
            s.frames.len() == 8
                && (1..4).all(|k| {
                    let (a, b) = (&s.frames[k], &s.frames[8 - k]);
                    if a.width != b.width || a.height != b.height || a.width == 0 {
                        return false;
                    }
                    // the drawn pixels of the one and of the other mirrored, and
                    // how many of them have the same colour (variants of a thing
                    // with the same outline, a floor tile, differ in colour)
                    let (mut both, mut either, mut same) = (0, 0, 0);
                    for row in 0..a.height {
                        for col in 0..a.width {
                            let ca = a.pixels[(row * a.width + col) as usize];
                            let cb = b.pixels[(row * b.width + b.width - 1 - col) as usize];
                            let (pa, pb) = (ca != TRANSPARENT, cb != TRANSPARENT);
                            both += (pa && pb) as i32;
                            either += (pa || pb) as i32;
                            same += (pa && ca == cb) as i32;
                        }
                    }
                    either > 0 && both * 10 >= either * 8 && same * 10 >= both * 6
                })
        });
        self.directions.insert(shape, d);
        d
    }

    /// The frame of a thing drawn in eight directions seen with the view
    /// turned by `quarter` (None for other things). A mirrored frame is
    /// the one for the mirrored direction.
    fn turned_frame(&mut self, shapes: &mut Shapes, shape: u32, frame: u32, flipped: bool, quarter: i32) -> Option<u32> {
        if frame >= 8 || !self.turns_with_view(shapes, shape) {
            return None;
        }
        let f = if flipped { (8 - frame as i32) % 8 } else { frame as i32 };
        Some((f + 2 * quarter).rem_euclid(8) as u32)
    }

    /// Works out the models of a few items around the view ahead of time,
    /// so that the first turn does not stall. Returns how many were made.
    pub fn prepare(&mut self, items: &[Item], infos: &[ShapeInfo], shapes: &mut Shapes, cam: (i32, i32, i32), budget: std::time::Duration) -> usize {
        let start = std::time::Instant::now();
        let mut made = 0;
        for it in items {
            if made > 0 && start.elapsed() >= budget {
                break;
            }
            if !it.in_world() || it.is_actor() || (it.x - cam.0).abs() > 2048 || (it.y - cam.1).abs() > 2048 {
                continue;
            }
            let flipped = it.flags & FLG_FLIPPED != 0;
            if self.models.contains_key(&(it.shape, it.frame, flipped)) {
                continue;
            }
            let Some(info) = infos.get(it.shape as usize) else { continue };
            self.model(shapes, info, it.shape, it.frame, flipped);
            made += 1;
        }
        made
    }

    /// Draws the world turned by `xf.angle`. Returns what was drawn.
    pub fn render(&mut self, surf: &mut Surface, xf: &Xform, scene: &Scene, shapes: &mut Shapes) -> Vec<Painted> {
        let (w, h) = (surf.width, surf.height);
        let n = (w * h) as usize;
        surf.pixels.fill(0);
        self.depth.clear();
        self.depth.resize(n, f32::MIN);
        self.ids.clear();
        self.ids.resize(n, 0);
        self.behind.clear();
        self.behind.resize(n, 0);
        self.width = w;
        let proj = Projection::new(xf);
        let quarter = xf.quarter();
        // a cube covers 2 x 3 pixels seen straight, 3 x 3 at a slant
        let slant = (xf.angle / 90.0).fract().abs() > 0.01;

        // 1. what is to be drawn, with its model
        let drawn = self.visible(&proj, scene, shapes, (w, h));

        // 2. the cubes of every model once on screen (relative to the
        // item), the pictures of the upright things; the list of jobs
        let mut list = DrawList::default();
        for (index, it, info, model) in drawn {
            match model.as_ref() {
                Some(m) if matches!(**m, Model::Cubes(..)) => list.add_cubes(m, index, &it, &info, scene.pal, &proj),
                _ => self.add_board(&mut list, shapes, scene, &proj, quarter, index, &it, &info, model.as_deref()),
            }
        }
        if std::env::var_os("PAGAN_TRACE").is_some() {
            let cubes: usize = list.jobs.iter().map(|j| match j { Job::Cubes { slot, .. } => list.cubes[*slot].len(), _ => 0 }).sum();
            eprintln!("turned picture: {} items, {cubes} cubes, {} models cached", list.painted.len(), self.models.len());
        }

        // 3. drawn in bands of rows, one per processor core
        self.paint_bands(surf, &list, scene.pal, slant);
        fill_gaps(surf, &self.ids);
        list.painted
    }

    /// The items near enough to the view to be drawn, with their models
    /// (None for people and creatures).
    fn visible(&mut self, proj: &Projection, scene: &Scene, shapes: &mut Shapes, (w, h): (i32, i32)) -> Vec<(usize, Item, ShapeInfo, Option<Rc<Model>>)> {
        let (cx, cy, _) = proj.xf.cam;
        let reach = 2 * w + 4 * h + 2048;
        let mut drawn = Vec::new();
        for (index, it) in scene.items.iter().enumerate() {
            if !it.in_world() || (scene.hide != 0 && it.id == scene.hide) || (it.x - cx).abs() > reach || (it.y - cy).abs() > reach {
                continue;
            }
            let Some(info) = scene.infos.get(it.shape as usize).copied() else { continue };
            if (info.editor && !scene.show_editor_items) || (it.flags & FLG_INVISIBLE != 0 && it.id != 1) {
                continue;
            }
            if scene.z_limit.is_some_and(|limit| it.z >= limit && !info.draw) {
                continue;
            }
            let flipped = it.flags & FLG_FLIPPED != 0;
            let model = if it.is_actor() {
                None
            } else {
                match self.model(shapes, &info, it.shape, it.frame, flipped) {
                    Some(m) => Some(m),
                    None => continue,
                }
            };
            // how far the item can reach from its position on screen
            let (ax, ay, _) = proj.point(it.x, it.y, it.z);
            let (xd, yd, zd) = info.footpad(flipped);
            let cubes = matches!(model.as_deref(), Some(Model::Cubes(..)));
            let margin = ((xd + yd) / 4 + zd + 64) as f32 + if cubes { 0.0 } else { 160.0 };
            if ax < -margin || ax > w as f32 + margin || ay < -margin || ay > h as f32 + margin {
                continue;
            }
            drawn.push((index, *it, info, model));
        }
        drawn
    }

    /// Adds an upright thing (a person, a tree, a lamp) to the draw list:
    /// people turn with the view, everything else is mirrored every other
    /// quarter turn.
    fn add_board(&mut self, list: &mut DrawList, shapes: &mut Shapes, scene: &Scene, proj: &Projection, quarter: i32, index: usize, it: &Item, info: &ShapeInfo, model: Option<&Model>) {
        let id = index as u32 + 1;
        let flipped = it.flags & FLG_FLIPPED != 0;
        let (frame, mirror) = if it.is_actor() {
            (scene.person_frame)(it, quarter)
        } else if let Some(f) = self.turned_frame(shapes, it.shape, it.frame, flipped, quarter) {
            (f, false)
        } else {
            (it.frame, flipped ^ (quarter % 2 == 1))
        };
        let (xd, yd, _) = info.footpad(flipped);
        let (offset, middle) = match model {
            Some(Model::Upright { offset, middle }) => (*offset, Some(*middle)),
            _ => (board_offset(xd, yd), None),
        };
        // Where the bulk of the picture is in the world: beside the
        // middle of the box by as much as it is beside it on screen
        // (a lamp hanging on a wall). That point turns with the world.
        let side = middle.map_or(0, |m| (offset.0 as f32 + m).round() as i32);
        let (px, py) = (it.x - xd / 2 + 2 * side, it.y - yd / 2 - 2 * side);
        let (fx, fy, _) = proj.point(px, py, it.z);
        // The board stands as near as the front of the box: the
        // ground in front of the middle must not hide the feet.
        let base = [(0, 0), (xd, 0), (0, yd), (xd, yd)]
            .iter()
            .map(|&(a, b)| proj.point(it.x - a, it.y - b, it.z).2)
            .fold(f32::MIN, f32::max);
        // The picture's bulk goes on that point; mirrored (every other
        // quarter turn) about it, so that the thing keeps its place.
        let ox = match middle {
            Some(m) if quarter % 2 == 1 => (fx + m).round() as i32 + 1,
            Some(m) => (fx - m).round() as i32,
            None => fx.round() as i32 + offset.0,
        };
        let oy = fy.round() as i32 + offset.1;
        let foot = fy.round() as i32;
        let Some(shape) = shapes.get(it.shape) else { return };
        if shape.frames.get(frame as usize).is_none() {
            return;
        }
        list.boards.push((shape, frame as usize));
        list.jobs.push(Job::Board { pic: list.boards.len() - 1, ox, oy, mirror, base, foot, bias: 0.0, id, trans: info.translucent });
        // the weapon in the Avatar's hand, just in front of him
        if let Some((_, oshape, oframe, odx, ody)) = scene.overlay.filter(|o| o.0 == index) {
            if let Some(os) = shapes.get(oshape) {
                if os.frames.get(oframe as usize).is_some() {
                    list.boards.push((os, oframe as usize));
                    list.jobs.push(Job::Board { pic: list.boards.len() - 1, ox: ox + odx, oy: oy + ody, mirror: false, base, foot, bias: 1.0, id, trans: false });
                }
            }
        }
        list.painted.push(Painted { index, sx: ox, sy: oy, shape: it.shape, frame, flipped: mirror });
    }

    /// Carries out the draw list, in bands of rows drawn side by side.
    fn paint_bands(&mut self, surf: &mut Surface, list: &DrawList, pal: &Palette, slant: bool) {
        let (w, h) = (surf.width, surf.height);
        let frames: Vec<&Frame> = list.boards.iter().map(|(s, f)| &s.frames[*f]).collect();
        let cores = crate::pool::cores() as i32;
        let band = ((h + cores - 1) / cores).max(1);
        let chunk = (band * w) as usize;
        // only plain data goes to the workers (the list also holds Rc's)
        let (frames, jobs, cubes, spans) = (&frames, &list.jobs[..], &list.cubes[..], &list.spans[..]);
        let Self { depth, ids, behind, .. } = self;
        let parts: Vec<_> = surf.pixels.chunks_mut(chunk).zip(depth.chunks_mut(chunk)).zip(ids.chunks_mut(chunk)).zip(behind.chunks_mut(chunk)).collect();
        crate::pool::each(parts, |b, (((pixels, depth), ids), behind)| {
            let y0 = b as i32 * band;
            let y1 = (y0 + band).min(h);
            let mixed = vec![0; pixels.len()];
            let mut band = Band { y0, y1, w, pixels, depth, ids, behind, mixed, colours: &pal.colors, xform: &pal.xform };
            // Translucent things (the glow of a lamp on the floor) go over
            // what is drawn, once all the solid things are there.
            for pass_trans in [false, true] {
                for job in jobs.iter().filter(|j| j.translucent() == pass_trans) {
                    match *job {
                        Job::Cubes { slot, ax, ay, ad, id, trans } => band.cubes(&cubes[slot], spans[slot], (ax, ay, ad), id, trans, slant),
                        Job::Board { pic, ox, oy, mirror, base, foot, bias, id, trans } => band.board(frames[pic], (ox, oy), mirror, base, foot, bias, id, trans),
                    }
                }
            }
        });
    }
}

/// The cubes filling an item's box, each with the colour of the pixel of
/// its picture it falls on (None: nothing drawn there).
struct Grid {
    nx: i32,
    ny: i32,
    nz: i32,
    cells: Vec<Option<u8>>,
}

impl Grid {
    /// Samples a frame for a box of size `(xd, yd, zd)`; also returns
    /// which of the frame's pixels the cubes cover. `reach`: how far
    /// around a cube's pixel to look for a drawn one.
    fn sample(f: &Frame, flipped: bool, (xd, yd, zd): (i32, i32, i32), reach: i32) -> (Grid, Vec<bool>) {
        let mut covered = vec![false; (f.width * f.height).max(0) as usize];
        let (nx, ny, nz) = ((xd / XY).max(1), (yd / XY).max(1), (zd / Z).max(1));
        let mut grid = Grid { nx, ny, nz, cells: vec![None; (nx * ny * nz) as usize] };
        for kz in 0..nz {
            for ky in 0..ny {
                for kx in 0..nx {
                    // middle of the cube, relative to the anchor
                    let (cx, cy, cz) = (kx * XY - xd + XY / 2, ky * XY - yd + XY / 2, kz * Z + Z / 2);
                    let (sx, sy) = ((cx - cy).div_euclid(4), (cx + cy).div_euclid(8) - cz);
                    let near = [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2), (0, -1), (1, -1)];
                    for (col, row) in near.into_iter().filter_map(|(ddx, ddy)| frame_pixel(f, flipped, sx + ddx, sy + ddy)) {
                        covered[(row * f.width + col) as usize] = true;
                    }
                    let i = grid.index(kx, ky, kz);
                    grid.cells[i] = (0..=reach).flat_map(ring).find_map(|(ddx, ddy)| {
                        let (col, row) = frame_pixel(f, flipped, sx + ddx, sy + ddy)?;
                        let c = f.pixels[(row * f.width + col) as usize];
                        (c != TRANSPARENT).then_some(c as u8)
                    });
                }
            }
        }
        (grid, covered)
    }

    fn index(&self, kx: i32, ky: i32, kz: i32) -> usize {
        ((kz * self.ny + ky) * self.nx + kx) as usize
    }

    fn filled(&self, kx: i32, ky: i32, kz: i32) -> bool {
        kx >= 0 && ky >= 0 && kz >= 0 && kx < self.nx && ky < self.ny && kz < self.nz && self.cells[self.index(kx, ky, kz)].is_some()
    }

    /// Was a cube itself to be seen in the original picture? Nothing of
    /// the item may lie before it, towards the viewer (one cube to +x and
    /// +y is half a cube higher on the same pixel).
    fn seen(&self, kx: i32, ky: i32, kz: i32) -> bool {
        (1..self.nx.max(self.ny) + 1).all(|t| !self.filled(kx + t, ky + t, kz + t / 2) && !self.filled(kx + t, ky + t, kz + (t + 1) / 2))
    }

    /// The colour for a cube on a back side, which was never drawn: its
    /// cubes would take the colour of the top, which lies in front of
    /// them (a counter would be black behind). It looks like the front
    /// instead, as seen in a mirror. (The cube on the front seen in the
    /// mirror is the outermost one of the same row that was drawn: the
    /// back layer of a wall has gaps where its picture ends, so a cube at
    /// a fixed depth may lie inside and take the wrong colour.)
    fn mirrored(&self, kx: i32, ky: i32, kz: i32) -> Option<u8> {
        // Open towards -x and -y (a corner, or a ragged back layer): the
        // side it lies nearer to, measured in the box's own size (a long
        // wall's back, not its end).
        let west = !self.filled(kx - 1, ky, kz);
        let north = !self.filled(kx, ky - 1, kz);
        let west = west && (!north || kx * self.ny <= ky * self.nx);
        let (mx, my) = if west {
            ((kx + 1..self.nx).rev().find(|&mx| self.filled(mx, ky, kz) && self.seen(mx, ky, kz))?, ky)
        } else if north {
            (kx, (ky + 1..self.ny).rev().find(|&my| self.filled(kx, my, kz) && self.seen(kx, my, kz))?)
        } else {
            return None;
        };
        self.cells[self.index(mx, my, kz)]
    }

    /// The model made of the cubes; only the cubes on the outside can
    /// ever be seen.
    fn cubes(&self, xd: i32, yd: i32) -> Model {
        let mut cubes = Vec::new();
        for kz in 0..self.nz {
            for ky in 0..self.ny {
                for kx in 0..self.nx {
                    let Some(colour) = self.cells[self.index(kx, ky, kz)] else { continue };
                    let outside = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
                        .iter()
                        .any(|&(a, b, c)| !self.filled(kx + a, ky + b, kz + c));
                    if !outside {
                        continue;
                    }
                    let colour = if self.seen(kx, ky, kz) { colour } else { self.mirrored(kx, ky, kz).unwrap_or(colour) };
                    cubes.push(Cube { dx: (kx * XY - xd) as i16, dy: (ky * XY - yd) as i16, dz: (kz * Z) as i16, colour });
                }
            }
        }
        Model::Cubes(cubes)
    }
}

/// The pixel (column, row) of a frame at a position relative to its
/// anchor (the item's position), if it lies within the frame.
fn frame_pixel(f: &Frame, flipped: bool, sx: i32, sy: i32) -> Option<(i32, i32)> {
    let col = if flipped { f.xoff - 1 - sx } else { sx + f.xoff };
    let row = sy + f.yoff;
    (col >= 0 && row >= 0 && col < f.width && row < f.height).then_some((col, row))
}

/// Where a picture's anchor lies relative to the middle of the bottom of
/// a box of size `xd` x `yd`, on screen.
fn board_offset(xd: i32, yd: i32) -> (i32, i32) {
    let (fx, fy) = (-xd / 2, -yd / 2);
    (-(fx - fy).div_euclid(4), -(fx + fy).div_euclid(8))
}

/// A thing that is drawn as a flat picture standing upright.
fn upright_model(f: &Frame, flipped: bool, xd: i32, yd: i32) -> Model {
    let (mut sum, mut count) = (0i64, 0i64);
    for row in 0..f.height {
        for col in 0..f.width {
            if f.pixels[(row * f.width + col) as usize] != TRANSPARENT {
                sum += (if flipped { f.xoff - 1 - col } else { col - f.xoff }) as i64;
                count += 1;
            }
        }
    }
    let middle = if count > 0 { sum as f32 / count as f32 } else { 0.0 };
    Model::Upright { offset: board_offset(xd, yd), middle }
}

/// The turned projection of a view.
struct Projection<'a> {
    xf: &'a Xform,
    sin: f32,
    cos: f32,
    /// See `Xform::origin`.
    origin: (f64, f64),
}

impl<'a> Projection<'a> {
    fn new(xf: &'a Xform) -> Self {
        let (sin, cos) = xf.sin_cos();
        Projection { xf, sin, cos, origin: xf.origin() }
    }

    /// Where an offset in the world moves the picture, and by how much it
    /// comes nearer: the projection is linear, so that is the same wherever
    /// it is.
    fn offset(&self, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        let (rx, ry) = (self.cos * x - self.sin * y, self.sin * x + self.cos * y);
        ((rx - ry) / 4.0, (rx + ry) / 8.0 - z, rx + ry + 8.0 * z)
    }

    /// A world position in the view (in the world's pixel grid, moved by
    /// whole pixels) and its depth.
    fn point(&self, x: i32, y: i32, z: i32) -> (f32, f32, f32) {
        let (cx, cy, cz) = self.xf.cam;
        let (sx, sy) = self.xf.projected(x as f64, y as f64, z as f64);
        let d = self.offset((x - cx) as f32, (y - cy) as f32, (z - cz) as f32).2;
        ((sx - self.origin.0) as f32, (sy - self.origin.1) as f32, d)
    }
}

/// One thing to draw.
enum Job {
    /// The cubes of a model (`slot` in `DrawList::cubes`) at a position
    /// in the view and its depth.
    Cubes { slot: usize, ax: f32, ay: f32, ad: f32, id: u32, trans: bool },
    /// An upright picture (`pic` in `DrawList::boards`).
    Board { pic: usize, ox: i32, oy: i32, mirror: bool, base: f32, foot: i32, bias: f32, id: u32, trans: bool },
}

impl Job {
    fn translucent(&self) -> bool {
        match *self {
            Job::Cubes { trans, .. } | Job::Board { trans, .. } => trans,
        }
    }
}

/// What a turned picture is made of.
#[derive(Default)]
struct DrawList {
    jobs: Vec<Job>,
    /// Per model drawn as cubes: its cubes in the view relative to the
    /// item (x, y, depth, colour) ...
    cubes: Vec<Vec<(f32, f32, f32, u32)>>,
    /// ... the rows they reach ...
    spans: Vec<(f32, f32)>,
    /// ... and which model is where in these lists.
    slots: HashMap<*const Model, usize>,
    /// The pictures of the upright things: shape and frame.
    boards: Vec<(Rc<Shape>, usize)>,
    painted: Vec<Painted>,
}

impl DrawList {
    /// Adds an item drawn as cubes; the cubes of a model are put on
    /// screen once for all items using it.
    fn add_cubes(&mut self, model: &Rc<Model>, index: usize, it: &Item, info: &ShapeInfo, pal: &Palette, proj: &Projection) {
        let Model::Cubes(cubes) = &**model else { return };
        let key = Rc::as_ptr(model);
        let slot = match self.slots.get(&key) {
            Some(&slot) => slot,
            None => {
                let placed: Vec<_> = cubes
                    .iter()
                    .map(|q| {
                        let (sx, sy, d) = proj.offset(q.dx as f32 + XY as f32 / 2.0, q.dy as f32 + XY as f32 / 2.0, q.dz as f32 + Z as f32 / 2.0);
                        let colour = if info.translucent { q.colour as u32 } else { pal.colors[q.colour as usize] };
                        (sx, sy, d, colour)
                    })
                    .collect();
                let lo = placed.iter().map(|p| p.1).fold(f32::MAX, f32::min);
                let hi = placed.iter().map(|p| p.1).fold(f32::MIN, f32::max);
                self.cubes.push(placed);
                self.spans.push((lo - 2.0, hi + 4.0));
                self.slots.insert(key, self.cubes.len() - 1);
                self.cubes.len() - 1
            }
        };
        let (ax, ay, ad) = proj.point(it.x, it.y, it.z);
        let flipped = it.flags & FLG_FLIPPED != 0;
        self.jobs.push(Job::Cubes { slot, ax, ay, ad, id: index as u32 + 1, trans: info.translucent });
        self.painted.push(Painted { index, sx: ax as i32, sy: ay as i32, shape: it.shape, frame: it.frame, flipped });
    }
}

/// A band of rows of the picture (rows `y0..y1`), drawn by one thread.
struct Band<'a> {
    y0: i32,
    y1: i32,
    w: i32,
    pixels: &'a mut [u32],
    depth: &'a mut [f32],
    ids: &'a mut [u32],
    behind: &'a mut [u32],
    /// Per pixel: the translucent thing last mixed into it.
    mixed: Vec<u32>,
    colours: &'a [u32; 256],
    xform: &'a [(u32, u32, u32, u32); 256],
}

impl Band<'_> {
    /// The index of a pixel of the band.
    fn at(&self, px: i32, py: i32) -> Option<usize> {
        (px >= 0 && py >= self.y0 && px < self.w && py < self.y1).then(|| ((py - self.y0) * self.w + px) as usize)
    }

    /// A solid pixel, if nothing nearer is there.
    fn plot(&mut self, px: i32, py: i32, d: f32, colour: u32, id: u32) {
        let Some(p) = self.at(px, py) else { return };
        if d > self.depth[p] {
            if self.ids[p] != id {
                self.behind[p] = self.ids[p];
            }
            self.depth[p] = d;
            self.ids[p] = id;
            self.pixels[p] = colour;
        } else if self.ids[p] != id && self.behind[p] == 0 {
            self.behind[p] = id;
        }
    }

    /// A translucent pixel mixed over what is drawn, as in the original.
    /// Translucent things lie on the ground they light, so they count as
    /// seen even a little behind it; every pixel is mixed only once per
    /// thing.
    fn mix(&mut self, px: i32, py: i32, d: f32, c: u32, id: u32) {
        let Some(p) = self.at(px, py) else { return };
        if self.mixed[p] == id || d + 48.0 < self.depth[p] {
            return;
        }
        self.mixed[p] = id;
        let (r, g, b, a) = self.xform[c as usize];
        self.pixels[p] = if a == 0 {
            self.colours[c as usize]
        } else {
            let dst = self.pixels[p];
            let ia = 256 - a;
            let ch = |dv: u32, sv: u32| ((dv * ia + 256 * sv) >> 8).min(255);
            (ch((dst >> 16) & 0xFF, r) << 16) | (ch((dst >> 8) & 0xFF, g) << 8) | ch(dst & 0xFF, b)
        };
    }

    /// The cubes of a model placed at `(ax, ay)` with depth `ad`; `span`:
    /// the rows they reach relative to it.
    fn cubes(&mut self, cubes: &[(f32, f32, f32, u32)], (lo, hi): (f32, f32), (ax, ay, ad): (f32, f32, f32), id: u32, trans: bool, slant: bool) {
        // nothing of it in this band?
        if ay + hi < self.y0 as f32 || ay + lo >= self.y1 as f32 {
            return;
        }
        let (cw, ch) = if slant { (3, 3) } else { (2, 3) };
        for &(ox, oy, od, colour) in cubes {
            let py = (ay + oy).floor() as i32 - 1;
            if py + ch <= self.y0 || py >= self.y1 {
                continue;
            }
            let px = (ax + ox).floor() as i32 - 1;
            let d = ad + od;
            for yy in py..py + ch {
                for xx in px..px + cw {
                    if !trans {
                        self.plot(xx, yy, d, colour, id);
                    } else if (xx + yy) & 1 == 0 {
                        // the original's glow is dithered: every other pixel
                        self.mix(xx, yy, d, colour, id);
                    }
                }
            }
        }
    }

    /// An upright picture with its anchor at `(ox, oy)`.
    fn board(&mut self, f: &Frame, (ox, oy): (i32, i32), mirror: bool, base: f32, foot: i32, bias: f32, id: u32, trans: bool) {
        let top = oy - f.yoff;
        if top + f.height <= self.y0 || top >= self.y1 {
            return;
        }
        for row in (self.y0 - top).max(0)..(self.y1 - top).min(f.height) {
            let py = top + row;
            // higher up on the board is higher up in the world; what
            // reaches below the box (a lantern's post) stands in front
            let d = base + 8.0 * (foot - py).abs() as f32 + bias;
            for col in 0..f.width {
                let c = f.pixels[(row * f.width + col) as usize];
                if c == TRANSPARENT {
                    continue;
                }
                let px = if mirror { ox + f.xoff - 1 - col } else { ox - f.xoff + col };
                if trans {
                    self.mix(px, py, d, c as u32, id);
                } else {
                    self.plot(px, py, d, self.colours[c as usize], id);
                }
            }
        }
    }
}

/// Where the original never drew anything (under a walkway, behind a
/// gate) the turned view has holes. A hole with something drawn above
/// and below it is filled in the shade of what is above, like the
/// shadow under an arch; open spaces (beyond the edge of the map) stay.
fn fill_gaps(surf: &mut Surface, ids: &[u32]) {
    const MOST: i32 = 64;
    let (w, h) = (surf.width, surf.height);
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if ids[(y * w + x) as usize] != 0 {
                y += 1;
                continue;
            }
            let start = y;
            while y < h && ids[(y * w + x) as usize] == 0 {
                y += 1;
            }
            // rows start..y are empty
            if start == 0 || y >= h || y - start > MOST {
                continue;
            }
            let above = surf.pixels[((start - 1) * w + x) as usize];
            let shade = |c: u32| ((c >> 16 & 0xFF) * 2 / 5) << 16 | ((c >> 8 & 0xFF) * 2 / 5) << 8 | ((c & 0xFF) * 2 / 5);
            for yy in start..y {
                surf.pixels[(yy * w + x) as usize] = shade(above);
            }
        }
    }
}

impl crate::game::Game {
    /// World position to position in the view, as last drawn.
    pub fn to_screen(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        self.view.xf.to_screen(x, y, z)
    }

    /// The world position (x, y) at height `z` under a view position.
    pub fn to_world(&self, sx: i32, sy: i32, z: i32) -> (i32, i32) {
        self.view.xf.to_world(sx, self.unflip_y(sy), z)
    }

    /// Is the view turned away from the original side?
    pub fn view_turned(&self) -> bool {
        self.view.xf.angle != 0.0
    }

    /// A direction meant on screen (0 = up, clockwise) as a direction in
    /// the world, with the view turned.
    pub fn unturn_dir(&self, dir: u8) -> u8 {
        (dir as i32 - 2 * self.view.xf.quarter()).rem_euclid(8) as u8
    }

    /// Turns the view by a quarter (+1 / -1), if that is switched on.
    pub fn turn_view(&mut self, by: i32) {
        if self.options.turn_view {
            self.turn.turn(by);
        }
    }

    /// The picture of a person seen with the view turned by `quarter`:
    /// the animation frame for the turned direction.
    pub fn person_frame(&mut self, it: &Item, quarter: i32) -> (u32, bool) {
        let flipped = it.flags & FLG_FLIPPED != 0;
        let (action, n, dir) = match self.actors.get(&it.id) {
            Some(m) => (m.shown().0, m.shown().1, m.dir),
            None => (crate::anim::STAND, 0, self.people.npc_dirs.get(&it.id).copied().unwrap_or(4)),
        };
        let dir = (dir as i32 + 2 * quarter).rem_euclid(8) as u8;
        match self.data.anims.action(it.shape, action).or_else(|| self.data.anims.action(it.shape, crate::anim::STAND)) {
            Some(a) if a.size > 0 => {
                let f = a.frame(dir, n.min(a.size - 1));
                (f.frame, f.flipped())
            }
            _ => (it.frame, flipped),
        }
    }
}
