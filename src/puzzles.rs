//! Option "Puzzle spots": places a puzzle needs that are easy to miss get
//! a name under the pointer, when looked at and with key H, and glow softly
//! while the Avatar carries what belongs there. The glow keeps to the
//! pixels of the place itself (buffer of which item each pixel shows), so
//! nothing shines through walls.

use crate::game::Game;
use crate::view::Surface;

/// A place that belongs to a puzzle.
pub struct Spot {
    /// Map and the item that is the place: shape and position.
    pub map: usize,
    pub shape: u32,
    pub pos: (i32, i32, i32),
    /// Its name: German, French, English (game font, DOS characters).
    pub name: [&'static str; 3],
    /// Where on it the glow sits, in world units from the item's position.
    pub glow: (i32, i32, i32),
    /// While this holds (journal condition: flags, `has:<shape>`) it glows.
    pub glows_while: &'static str,
    /// While this holds it is pointed out at all.
    pub shown_while: &'static str,
}

pub const SPOTS: &[Spot] = &[
    // the hollow in the tomb wall of the necropolis for the Skull of Quakes
    Spot {
        map: 50,
        shape: 805,
        pos: (15519, 6143, 48),
        name: ["Sch\u{84}delmulde", "Creux de cr\u{83}ne", "Skull-shaped hollow"],
        glow: (-16, -24, 20),
        glows_while: "has:814 & !postEarthQuake",
        shown_while: "!postEarthQuake",
    },
];

/// The small pentacle of a sorcerer (Bane's, Vardion's ...).
const PENTACLE: (u32, u32) = (124, 4);
const CANDLE: u32 = 454;
const REAGENT: u32 = 398;
const FOCUS: u32 = 397;
/// Its five points, from the pentacle's position: where the candles go.
const POINTS: [(i32, i32); 5] = [(-70, -70), (-41, -201), (-167, -266), (-266, -167), (-201, -41)];
/// Its middle (the pentacle is 288 wide): where the focus goes.
const MIDDLE: (i32, i32) = (-144, -144);
/// A reagent counts for a candle up to this far from it.
const REAGENT_REACH: i32 = 48;
/// Standing this near a pentacle, the Avatar reaches all of it.
const PENT_NEAR: i32 = 128;
/// A candle counts for a point up to this far from it.
const CANDLE_SLACK: i32 = 24;

/// Radius of the glow in pixels.
const RADIUS: i32 = 9;
/// Colour of the glow (a warm gold).
const GLOW: (f32, f32, f32) = (255.0, 215.0, 120.0);

impl Game {
    /// The puzzle spot an item is, if the option is on and it is pointed out now.
    pub fn puzzle_spot(&self, id: u16) -> Option<&'static Spot> {
        if !self.options.puzzle_spots || id == 0 {
            return None;
        }
        let it = self.map().get(id)?;
        if !it.in_world() {
            return None;
        }
        SPOTS
            .iter()
            .find(|s| s.map == self.cur_map && s.shape == it.shape && s.pos == (it.x, it.y, it.z))
            .filter(|s| self.holds(s.shown_while))
    }

    pub fn puzzle_spot_name(&self, id: u16) -> Option<String> {
        self.puzzle_spot(id).map(|s| self.tr(s.name).to_string())
    }

    /// Looking at a puzzle spot says its name. True if it was one.
    pub fn look_at_puzzle_spot(&mut self, id: u16) -> bool {
        let Some(name) = self.puzzle_spot_name(id) else { return false };
        self.bark(id, name);
        true
    }

    /// Option "Reach at the pentacle": does a thing at (x, y, z) lie on a
    /// pentacle the Avatar stands near? Then he reaches it from there.
    /// (Any pentacle, also the large one of four parts; nothing else.)
    pub fn pentagram_reach(&self, x: i32, y: i32, z: i32) -> bool {
        if !self.options.pentagram_reach {
            return false;
        }
        let av = self.avatar();
        let map = self.map();
        let pentacles: Vec<crate::map::Item> = map.items_near(x, y, 640).into_iter().map(|i| map.items[i]).filter(|p| p.in_world() && p.shape == PENTACLE.0).collect();
        let Some(si) = self.data.infos.get(PENTACLE.0 as usize) else { return false };
        let (xd, yd) = (si.x * 32, si.y * 32);
        // (a little beyond its edge too: a candle on a point sticks out)
        let on = |p: &crate::map::Item| x > p.x - xd - 32 && x <= p.x + 32 && y > p.y - yd - 32 && y <= p.y + 32 && (z - p.z).abs() <= 24;
        pentacles.iter().filter(|p| on(p)).any(|p| {
            // near it, or near a part of the same pentacle
            (av.z - p.z).abs() <= 24 && pentacles.iter().any(|q| q.z == p.z && self.box_gap(p, q) == 0 && self.box_gap(&av, q) <= PENT_NEAR)
        })
    }

    /// Option "Pentacle snaps": where an item put down at (x, y, z) settles
    /// instead. A candle near a point of a pentacle goes onto the point (or
    /// the nearest free place that still counts as it), a reagent put onto
    /// the candle of a point right in front of it (towards the middle), the
    /// focus into the middle. `onto`: what it was put onto. None: it stays
    /// where it was put.
    pub fn pentagram_snap(&self, id: u16, x: i32, y: i32, z: i32, onto: Option<u16>) -> Option<(i32, i32, i32)> {
        if !self.options.pentagram_snap {
            return None;
        }
        let it = *self.map().get(id)?;
        if ![CANDLE, REAGENT, FOCUS].contains(&it.shape) {
            return None;
        }
        let pent = *self.map().items.iter().find(|p| {
            p.in_world() && (p.shape, p.frame) == PENTACLE && (-330..=40).contains(&(x - p.x)) && (-330..=40).contains(&(y - p.y)) && (z - p.z).abs() <= 16
        })?;
        let dist = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let point = |k: usize| (pent.x + POINTS[k].0, pent.y + POINTS[k].1);
        let at = point((0..5).min_by_key(|&k| dist(point(k), (x, y)))?);
        let others = |shape: u32| self.map().items.iter().filter(move |o| o.in_world() && o.shape == shape && o.id != id && (o.z - pent.z).abs() <= 8);
        let candle = others(CANDLE).map(|c| (c.x, c.y)).find(|&c| dist(c, at) <= CANDLE_SLACK);
        let tries: Vec<(i32, i32)> = match it.shape {
            CANDLE => {
                // (a point that has a candle already keeps it)
                if dist(at, (x, y)) > 48 || candle.is_some() {
                    return None;
                }
                // the point itself, else the nearest place that still counts
                // (something thin may stand in the way), keeping the
                // reagents laid there within reach
                let reagents: Vec<(i32, i32)> = others(REAGENT).map(|r| (r.x, r.y)).filter(|&r| dist(r, at) <= REAGENT_REACH).collect();
                let mut near: Vec<(i32, i32)> = (-3..=3).flat_map(|i| (-3..=3).map(move |j| (8 * i, 8 * j))).collect();
                near.sort_by_key(|&(dx, dy)| (dx.abs().max(dy.abs()), dx.abs() + dy.abs()));
                near.into_iter().map(|(dx, dy)| (at.0 + dx, at.1 + dy)).filter(|&c| reagents.iter().all(|&r| dist(r, c) <= REAGENT_REACH)).collect()
            }
            REAGENT => {
                // only onto the candle of a point: put onto it, or come to
                // lie on it (the pointer on the pentacle just beside it)
                let on_point = |c: &crate::map::Item| c.shape == CANDLE && c.in_world() && (c.z - pent.z).abs() <= 8 && (0..5).any(|k| dist(point(k), (c.x, c.y)) <= CANDLE_SLACK);
                let c = match onto.and_then(|o| self.map().get(o).copied()).filter(|c| on_point(c)) {
                    Some(c) => c,
                    None => *others(CANDLE).find(|c| on_point(c) && (c.x - x).abs() < 32 && (c.y - y).abs() < 32)?,
                };
                // right in front of it: straight towards the middle, as close
                // as it goes without touching it, else a little to the side
                let middle = (pent.x + MIDDLE.0, pent.y + MIDDLE.1);
                let toward = ((middle.1 - c.y) as f64).atan2((middle.0 - c.x) as f64);
                [0.0f64, 22.5, -22.5, 45.0, -45.0, 67.5, -67.5, 90.0, -90.0]
                    .iter()
                    .map(|deg| {
                        let a = toward + deg.to_radians();
                        let (dx, dy) = (a.cos(), a.sin());
                        let scale = 32.0 / dx.abs().max(dy.abs());
                        (c.x + (dx * scale).round() as i32, c.y + (dy * scale).round() as i32)
                    })
                    .collect()
            }
            _ => {
                // (the focus only in the middle, from anywhere on the pentacle)
                let (xd, yd, _) = self.data.infos.get(it.shape as usize)?.footpad(false);
                vec![(pent.x + MIDDLE.0 + xd / 2, pent.y + MIDDLE.1 + yd / 2)]
            }
        };
        let si = self.data.infos.get(it.shape as usize)?;
        let (xd, yd, zd) = si.footpad(false);
        let nowhere = crate::map::WorldBox { x: i32::MIN / 2, y: i32::MIN / 2, z: 0, xd: 0, yd: 0, zd: 0 };
        tries.into_iter().map(|(px, py)| (px, py, pent.z)).find(|&(px, py, pz)| {
            let b = crate::map::WorldBox { x: px, y: py, z: pz, xd, yd, zd };
            let pi = self.map().position_info(&b, &nowhere, &self.data.infos, id);
            pi.valid && pi.supported
        })
    }

    /// Option "Pentacle snaps": where a reagent put onto the item `onto`
    /// goes, if that is the candle of a point (in front of it).
    pub fn snap_onto_candle(&self, id: u16, onto: &crate::map::Item) -> Option<(i32, i32, i32)> {
        if onto.shape != CANDLE || self.map().get(id)?.shape != REAGENT {
            return None;
        }
        self.pentagram_snap(id, onto.x, onto.y, onto.z, Some(onto.id))
    }

    /// Option "Pentacle snaps": the reagents in front of a candle standing
    /// in the world (they go along when it is put onto a point).
    pub fn reagents_of_candle(&self, id: u16) -> Vec<u16> {
        let Some(c) = self.map().get(id).copied() else { return Vec::new() };
        if !self.options.pentagram_snap || c.shape != CANDLE || !c.in_world() {
            return Vec::new();
        }
        let near = |r: &&crate::map::Item| (r.x - c.x).abs().max((r.y - c.y).abs()) <= REAGENT_REACH && (r.z - c.z).abs() <= 8;
        self.map().items.iter().filter(|r| r.in_world() && r.shape == REAGENT).filter(near).map(|r| r.id).collect()
    }

    /// Option "Pentacle snaps": the reagents lying on the point a candle put
    /// down at (x, y, z) goes onto (they make room for it, then go in front
    /// of it).
    pub fn reagents_on_point(&self, id: u16, x: i32, y: i32, z: i32) -> Vec<u16> {
        let Some(it) = self.map().get(id).copied() else { return Vec::new() };
        if !self.options.pentagram_snap || it.shape != CANDLE {
            return Vec::new();
        }
        let Some(pent) = self.map().items.iter().find(|p| p.in_world() && (p.shape, p.frame) == PENTACLE && (-330..=40).contains(&(x - p.x)) && (-330..=40).contains(&(y - p.y)) && (z - p.z).abs() <= 16).copied() else { return Vec::new() };
        let dist = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let Some(at) = POINTS.iter().map(|o| (pent.x + o.0, pent.y + o.1)).min_by_key(|&p| dist(p, (x, y))).filter(|&p| dist(p, (x, y)) <= 48) else { return Vec::new() };
        self.map().items.iter().filter(|r| r.in_world() && r.shape == REAGENT && (r.z - pent.z).abs() <= 8 && (r.x - at.0).abs() < 32 && (r.y - at.1).abs() < 32).map(|r| r.id).collect()
    }

    /// The glow on the spots whose puzzle the Avatar carries the piece for.
    pub fn draw_puzzle_spots(&mut self, surf: &mut Surface) {
        if !self.options.puzzle_spots || self.view_turned() || surf.width != self.view.size.0 || surf.height != self.view.size.1 {
            return;
        }
        let ids = self.drawn_ids();
        if ids.len() != surf.pixels.len() {
            return;
        }
        for s in SPOTS.iter().filter(|s| s.map == self.cur_map) {
            let Some(index) = self.map().items.iter().position(|it| it.in_world() && it.shape == s.shape && (it.x, it.y, it.z) == s.pos) else { continue };
            if !self.holds(s.glows_while) {
                continue;
            }
            let (cx, cy) = self.to_screen(s.pos.0 + s.glow.0, s.pos.1 + s.glow.1, s.pos.2 + s.glow.2);
            // breathes slowly: about two seconds from dim to bright and back
            let phase = (self.tick % 60) as f32 / 60.0 * std::f32::consts::TAU;
            let strength = 0.45 + 0.35 * phase.sin();
            let me = index as u32 + 1;
            let (w, h) = (surf.width, surf.height);
            for dy in -RADIUS..=RADIUS {
                for dx in -RADIUS..=RADIUS {
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    let r = ((dx * dx + dy * dy) as f32).sqrt() / RADIUS as f32;
                    let at = (y * w + x) as usize;
                    if r > 1.0 || ids[at] != me {
                        continue;
                    }
                    let a = strength * (1.0 - r * r);
                    let c = surf.pixels[at];
                    let mix = |v: u32, t: f32| ((v as f32) * (1.0 - a) + t * a).round().min(255.0) as u32;
                    surf.pixels[at] = (c & 0xFF00_0000) | mix(c >> 16 & 0xFF, GLOW.0) << 16 | mix(c >> 8 & 0xFF, GLOW.1) << 8 | mix(c & 0xFF, GLOW.2);
                }
            }
        }
    }
}
