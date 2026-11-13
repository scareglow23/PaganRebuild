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

/// Ticks the Avatar must stay hidden before the outline shows: walking
/// past a pillar or a tree, it would come and go (about 0.3 s).
const DELAY: u32 = 9;
/// Ticks the outline takes to come in and to go (about 0.2 s).
const FADE: f32 = 6.0;

/// How strongly the outline is shown: it comes in only after the Avatar
/// has been hidden for a moment, and comes and goes softly.
#[derive(Default)]
pub struct OutlineFade {
    /// From 0 (not shown) to 1, and the tick it was last worked out at.
    strength: f32,
    tick: u32,
    /// Since which tick enough of him has been hidden (None: not now).
    hidden_since: Option<u32>,
}

fn blend(px: u32, k: u32) -> u32 {
    let mix = |c: u32, t: u32| (c * (256 - k) + t * k) >> 8;
    (mix((px >> 16) & 0xFF, EDGE.0) << 16) | (mix((px >> 8) & 0xFF, EDGE.1) << 8) | mix(px & 0xFF, EDGE.2)
}

/// The Avatar's figure on screen: his frame, where it lies, and which of
/// its pixels the picture shows something else at.
struct Hidden {
    shape: std::rc::Rc<crate::shape::Shape>,
    frame: usize,
    flipped: bool,
    left: i32,
    top: i32,
    /// The hidden pixels of the figure (column, row in its frame), and
    /// how many it has on screen in all.
    pixels: Vec<(i32, i32)>,
    body_pixels: usize,
}

impl Hidden {
    /// Is (column, row) of the frame part of the figure?
    fn body(&self, col: i32, row: i32) -> bool {
        let f = &self.shape.frames[self.frame];
        if col < 0 || row < 0 || col >= f.width || row >= f.height {
            return false;
        }
        let src = if self.flipped { f.width - 1 - col } else { col };
        f.pixels[(row * f.width + src) as usize] != TRANSPARENT
    }
}

impl Game {
    /// Draws the Avatar's outline where he is hidden. `av_index`: his
    /// place in the map's item list. Returns how many pixels it drew.
    pub fn draw_avatar_outline(&mut self, surf: &mut Surface, av_index: Option<usize>) -> usize {
        // (not while the view turns: the figures are drawn otherwise then)
        // how much of him must be hidden (menu): while he can still be made
        // out well enough, the picture stays as it is
        let share = crate::menu::OUTLINE_SHARES[self.options.avatar_outline.min(crate::menu::OUTLINE_SHARES.len() - 1)];
        let found = if share == 0 || self.view_turned() { None } else { self.hidden_part(av_index, surf.width, surf.height) };
        let Some(hidden) = found else {
            self.view.outline = OutlineFade::default();
            return 0;
        };

        let now = self.tick;
        let fade = &mut self.view.outline;
        // (ticks since the last picture; pictures within a tick change nothing)
        let steps = now.wrapping_sub(fade.tick).min(30) as f32;
        fade.tick = now;
        // Once shown it stays while half as much of him is hidden: at
        // about the share set it would come and go.
        let shown = fade.strength > 0.0;
        let need = if shown { share / 2 } else { share };
        let enough = !hidden.pixels.is_empty() && hidden.pixels.len() * 100 >= hidden.body_pixels * need;
        fade.hidden_since = if enough { Some(fade.hidden_since.unwrap_or(now)) } else { None };
        // (hidden again while it fades: back at once)
        let on = fade.hidden_since.is_some_and(|since| shown || now.wrapping_sub(since) >= DELAY);
        fade.strength = if on { (fade.strength + steps / FADE).min(1.0) } else { (fade.strength - steps / FADE).max(0.0) };
        let k = fade.strength;
        if k <= 0.0 {
            return 0;
        }
        let w = surf.width;
        for &(col, row) in &hidden.pixels {
            let b = |c, r| hidden.body(c, r);
            let edge = !(b(col - 1, row) && b(col + 1, row) && b(col, row - 1) && b(col, row + 1));
            let i = ((hidden.top + row) * w + hidden.left + col) as usize;
            surf.pixels[i] = blend(surf.pixels[i], ((if edge { LINE } else { FILL }) as f32 * k) as u32);
        }
        hidden.pixels.len()
    }

    /// Which pixels of the Avatar's figure are hidden in the last picture
    /// (a view `w` x `h`). None if he is not to be seen in it at all.
    fn hidden_part(&mut self, av_index: Option<usize>, w: i32, h: i32) -> Option<Hidden> {
        let av = av_index?;
        let it = self.map().items.get(av)?;
        if it.id != AVATAR || !it.in_world() || it.flags & crate::map::FLG_INVISIBLE != 0 {
            return None;
        }
        let p = self.view.painted.iter().find(|p| p.index == av).map(|p| crate::view::Painted { ..*p })?;
        let shape = self.data.shapes.original(p.shape)?;
        let frame = p.frame as usize;
        let f = shape.frames.get(frame)?;
        let ids = self.drawn_ids();
        if ids.len() != (w * h) as usize {
            return None;
        }
        // the figure on screen: inside its frame, which pixels are his
        let left = if p.flipped { p.sx + f.xoff - f.width } else { p.sx - f.xoff };
        let top = p.sy - f.yoff;
        let mut hidden = Hidden { shape: shape.clone(), frame, flipped: p.flipped, left, top, pixels: Vec::new(), body_pixels: 0 };
        let me = av as u32 + 1;
        for row in 0..f.height {
            let y = top + row;
            if y < 0 || y >= h {
                continue;
            }
            for col in 0..f.width {
                let x = left + col;
                if x < 0 || x >= w || !hidden.body(col, row) {
                    continue;
                }
                hidden.body_pixels += 1;
                if ids[(y * w + x) as usize] != me {
                    hidden.pixels.push((col, row));
                }
            }
        }
        Some(hidden)
    }
}
