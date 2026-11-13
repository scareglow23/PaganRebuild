//! Option "Avatar outline": where something stands in front of the Avatar
//! (a wall, a tree, a roof edge), his outline shows through it, as in many
//! newer games. Done on the finished picture of the world with the buffer
//! of which item each pixel shows.

use crate::game::{Game, AVATAR};
use crate::shape::TRANSPARENT;
use crate::view::Surface;

/// Colour of the outline (a pale blue that stands out from stone and wood).
const EDGE: (u32, u32, u32) = (200, 225, 255);
/// How far the hidden inside of the figure is tinted towards it (0..=256).
const FILL: u32 = 64;
/// How far its edge is (0..=256).
const LINE: u32 = 210;

fn blend(px: u32, k: u32) -> u32 {
    let mix = |c: u32, t: u32| (c * (256 - k) + t * k) >> 8;
    (mix((px >> 16) & 0xFF, EDGE.0) << 16) | (mix((px >> 8) & 0xFF, EDGE.1) << 8) | mix(px & 0xFF, EDGE.2)
}

impl Game {
    /// Draws the Avatar's outline where he is hidden. `av_index`: his
    /// place in the map's item list. Returns how many pixels it drew.
    pub fn draw_avatar_outline(&mut self, surf: &mut Surface, av_index: Option<usize>) -> usize {
        // (not while the view turns: the figures are drawn otherwise then)
        // how much of him must be hidden (menu): while he can still be made
        // out well enough, the picture stays as it is
        let share = crate::menu::OUTLINE_SHARES[self.options.avatar_outline.min(crate::menu::OUTLINE_SHARES.len() - 1)];
        if share == 0 || self.view_turned() {
            return 0;
        }
        let Some(av) = av_index else { return 0 };
        let Some(it) = self.map().items.get(av) else { return 0 };
        if it.id != AVATAR || !it.in_world() || it.flags & crate::map::FLG_INVISIBLE != 0 {
            return 0;
        }
        let Some(p) = self.view.painted.iter().find(|p| p.index == av).map(|p| crate::view::Painted { ..*p }) else { return 0 };
        let Some(shape) = self.data.shapes.original(p.shape) else { return 0 };
        let Some(f) = shape.frames.get(p.frame as usize) else { return 0 };
        let ids = self.drawn_ids();
        let (w, h) = (surf.width, surf.height);
        if ids.len() != (w * h) as usize {
            return 0;
        }

        // the figure on screen: inside its frame, which pixels are his
        let (fw, fh) = (f.width, f.height);
        let left = if p.flipped { p.sx + f.xoff - fw } else { p.sx - f.xoff };
        let top = p.sy - f.yoff;
        let body = |col: i32, row: i32| -> bool {
            if col < 0 || row < 0 || col >= fw || row >= fh {
                return false;
            }
            let src = if p.flipped { fw - 1 - col } else { col };
            f.pixels[(row * fw + src) as usize] != TRANSPARENT
        };
        let me = av as u32 + 1;
        let mut hidden = Vec::new();
        let mut body_pixels = 0;
        for row in 0..fh {
            let y = top + row;
            if y < 0 || y >= h {
                continue;
            }
            for col in 0..fw {
                let x = left + col;
                if x < 0 || x >= w || !body(col, row) {
                    continue;
                }
                body_pixels += 1;
                if ids[(y * w + x) as usize] != me {
                    hidden.push((col, row));
                }
            }
        }
        if hidden.is_empty() || hidden.len() * 100 < body_pixels * share {
            return 0;
        }
        let drawn = hidden.len();
        for (col, row) in hidden {
            let edge = !(body(col - 1, row) && body(col + 1, row) && body(col, row - 1) && body(col, row + 1));
            let i = ((top + row) * w + left + col) as usize;
            surf.pixels[i] = blend(surf.pixels[i], if edge { LINE } else { FILL });
        }
        drawn
    }
}
