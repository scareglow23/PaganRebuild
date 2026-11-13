//! The invisible path revealed: over the lava of the earth plane runs a
//! path no one can see. A thrown thing that comes to rest on it shows
//! where it is; here the stretch of path around it
//! becomes faintly visible as well and stays so. The shading keeps to the
//! pixels of the lava below the path (buffer of which item each pixel
//! shows), so nothing in front of the path is painted over.

use crate::game::Game;
use crate::view::Surface;

/// The invisible floor tiles of the path.
const PATH_TILE: u32 = 797;
/// Size of a tile in world units.
const TILE: i32 = 128;
/// Tiles this far from the one hit (in tiles, also diagonally) show too.
const REACH: i32 = 1;
/// Ticks a tile takes to fade in.
const FADE_TICKS: u32 = 20;
/// Inside of a shown tile: dark stone over the glow of the lava.
const STONE: (f32, f32, f32) = (70.0, 58.0, 50.0);
const STONE_PART: f32 = 0.55;
/// Its edge: a pale line.
const EDGE: (f32, f32, f32) = (235.0, 205.0, 150.0);
const EDGE_PART: f32 = 0.6;

impl Game {
    /// A thrown item came to rest on `support`: if that is a tile of the
    /// invisible path, it and its neighbours on the path are shown.
    fn path_hit(&mut self, tile: u16) {
        let Some(tile) = self.map().get(tile).copied().filter(|t| t.shape == PATH_TILE && t.in_world()) else { return };
        let tiles: Vec<crate::map::Item> = self.map().items.iter()
            .filter(|t| t.shape == PATH_TILE && t.in_world() && (t.z - tile.z).abs() <= 8
                && (t.x - tile.x).abs() <= 2 * TILE + TILE / 2 && (t.y - tile.y).abs() <= 2 * TILE + TILE / 2)
            .copied()
            .collect();
        // a tile at about this offset from the one hit, in tiles
        let at = |dx: i32, dy: i32| tiles.iter().find(|t| (t.x - tile.x - dx * TILE).abs() <= TILE / 2 && (t.y - tile.y - dy * TILE).abs() <= TILE / 2).map(|t| t.id);
        let mut show = vec![tile.id];
        for dx in -REACH..=REACH {
            for dy in -REACH..=REACH {
                if (dx, dy) == (0, 0) {
                    continue;
                }
                // the next tile that way; across a gap (the path has gaps
                // to jump over) the one beyond it
                match at(dx, dy) {
                    Some(id) => show.push(id),
                    None => show.extend(at(2 * dx, 2 * dy)),
                }
            }
        }
        let (map, now) = (self.cur_map, self.tick);
        let shown = self.revealed_path.entry(map).or_default();
        for id in show {
            shown.entry(id).or_insert(now);
        }
    }

    /// A thing thrown or put down has come to rest: if it lies under a
    /// tile of the path (it came in from the side, the path being so thin)
    /// it goes up onto it (on top of another thing lying there, if need
    /// be); lying on the path, it shows that stretch. Returns the tile it
    /// lies on: that is what it hit, not the lava below, which would
    /// swallow it.
    pub fn path_landing(&mut self, id: u16) -> Option<u16> {
        let it = self.map().get(id).copied().filter(|it| it.in_world())?;
        let si = self.data.infos.get(it.shape as usize).copied()?;
        let (xd, yd, zd) = si.footpad(false);
        // (any part of it over the tile: lying on its edge it rests on it)
        let tile = self.map().items.iter()
            .filter(|t| t.shape == PATH_TILE && t.in_world() && t.z >= it.z && t.z - it.z <= 64
                && it.x > t.x - TILE && it.x - xd < t.x && it.y > t.y - TILE && it.y - yd < t.y)
            .min_by_key(|t| t.z)
            .copied()?;
        if it.z < tile.z {
            let here = crate::map::WorldBox { x: it.x, y: it.y, z: it.z, xd, yd, zd };
            let z = [0, 8, 16, 24].into_iter().map(|up| tile.z + up).find(|&z| {
                let pi = self.map().position_info(&crate::map::WorldBox { z, ..here }, &here, &self.data.infos, id);
                pi.valid && pi.supported
            })?;
            if let Some(idx) = self.map().index(id) {
                self.map_mut().place(idx, it.x, it.y, z);
            }
        }
        self.path_hit(tile.id);
        Some(tile.id)
    }

    /// A revealed tile of the path shown at a view position over `hit`
    /// (what is drawn there: the lava below it). For the pointer: one can
    /// walk and jump onto the tiles that show, and put things on them.
    pub fn revealed_tile_at(&self, x: i32, y: i32, hit: u16) -> Option<u16> {
        let shown = self.revealed_path.get(&self.cur_map)?;
        self.path_tile_at(x, y, hit).filter(|id| shown.contains_key(id))
    }

    /// A tile of the path (shown or not) at a view position over `hit`,
    /// what is drawn there: the lava below the path, nothing in front of
    /// it. A thing aimed there goes onto the path, where it would be seen
    /// lying at that point.
    pub fn path_tile_at(&self, x: i32, y: i32, hit: u16) -> Option<u16> {
        if self.view_turned() {
            return None;
        }
        let below = self.map().get(hit).copied()?;
        if below.shape == PATH_TILE {
            return Some(below.id);
        }
        let below_top = below.z + self.data.infos.get(below.shape as usize)?.footpad(false).2;
        let (cx, cy, _) = self.camera_position();
        let reach = 4 * (self.view.size.0 + self.view.size.1);
        self.map().items.iter()
            .filter(|t| t.shape == PATH_TILE && t.in_world() && t.z > below_top && (t.x - cx).abs() < reach && (t.y - cy).abs() < reach)
            .find(|t| {
                let (fx, fy) = self.to_screen(t.x, t.y, t.z);
                let (dx, dy) = ((x - fx).abs() as f32, (y - (fy - TILE / 8)).abs() as f32);
                dx / (TILE / 4) as f32 + dy / (TILE / 8) as f32 <= 1.0
            })
            .map(|t| t.id)
    }

    /// Where a thing thrown at (x, y, z) comes down: on the path, if that
    /// point lies under it. The player aims at the lava, as the path
    /// cannot be seen; flying down at a slant to the lava the thing would
    /// slip in under the path instead of landing on it.
    pub fn throw_target(&self, (x, y, z): (i32, i32, i32)) -> (i32, i32, i32) {
        let above = self.map().items.iter()
            .filter(|t| t.shape == PATH_TILE && t.in_world() && t.z > z && x > t.x - TILE && x <= t.x && y > t.y - TILE && y <= t.y)
            .map(|t| t.z)
            .min();
        (x, y, above.unwrap_or(z))
    }

    /// Shades the shown tiles of the path.
    pub fn draw_revealed_path(&mut self, surf: &mut Surface) {
        if self.view_turned() || surf.width != self.view.size.0 || surf.height != self.view.size.1 {
            return;
        }
        let Some(shown) = self.revealed_path.get(&self.cur_map) else { return };
        let ids = self.drawn_ids();
        if ids.len() != surf.pixels.len() {
            return;
        }
        let (w, h) = (surf.width, surf.height);
        for (&id, &since) in shown {
            let Some(tile) = self.map().get(id).copied().filter(|t| t.in_world()) else { continue };
            let fade = (self.tick.wrapping_sub(since).min(FADE_TICKS) as f32 + 1.0) / (FADE_TICKS as f32 + 1.0);
            // the tile's diamond: its corner (x, y) is the front, the
            // middle half a tile behind it
            let (fx, fy) = self.to_screen(tile.x, tile.y, tile.z);
            let (cx, cy) = (fx, fy - TILE / 8);
            let (hw, hh) = (TILE / 4, TILE / 8);
            for dy in -hh..=hh {
                for dx in -hw..=hw {
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    // 0 in the middle, 1 on the edge
                    let d = dx.abs() as f32 / hw as f32 + dy.abs() as f32 / hh as f32;
                    if d > 1.0 {
                        continue;
                    }
                    let at = (y * w + x) as usize;
                    if !self.below_path(ids[at], &tile) {
                        continue;
                    }
                    let (col, part) = if d > 1.0 - 2.5 / hh as f32 { (EDGE, EDGE_PART) } else { (STONE, STONE_PART) };
                    let a = part * fade;
                    let c = surf.pixels[at];
                    let mix = |v: u32, t: f32| ((v as f32) * (1.0 - a) + t * a).round().min(255.0) as u32;
                    surf.pixels[at] = (c & 0xFF00_0000) | mix(c >> 16 & 0xFF, col.0) << 16 | mix(c >> 8 & 0xFF, col.1) << 8 | mix(c & 0xFF, col.2);
                }
            }
        }
    }

    /// Whether the pixel shows something the tile covers: wholly lower
    /// than it and under or behind it (the lava and its edges), not
    /// something standing on it or lying in front of it (a stone in the
    /// lava just before the path).
    fn below_path(&self, pixel_id: u32, tile: &crate::map::Item) -> bool {
        if pixel_id == 0 {
            return false;
        }
        let Some(it) = self.map().items.get(pixel_id as usize - 1) else { return false };
        let Some(si) = self.data.infos.get(it.shape as usize) else { return false };
        let (xd, yd, zd) = si.footpad(it.flags & crate::map::FLG_FLIPPED != 0);
        let in_front = it.x - xd >= tile.x || it.y - yd >= tile.y;
        it.z + zd < tile.z && !in_front
    }
}
