//! Hidden keys: a key that lies almost hidden behind something
//! bigger (under a skeleton) is easy to miss and hard to grab, as only a
//! few of its pixels show. Within reach of the Avatar its outline shows through
//! what covers them, and a click on the covering thing where they lie
//! behind it (or just beside them) reaches them. Only within reach, so
//! that nothing shows through roofs and walls. Done with the buffer of
//! which item each pixel shows.

use crate::game::Game;
use crate::shape::TRANSPARENT;
use crate::view::Surface;

/// Colour of the outline (a pale gold, apart from the Avatar's blue): only
/// a faint hint at its edge, it is meant to be found, not pointed out.
const EDGE: (u32, u32, u32) = (255, 225, 150);
/// How far the hidden inside is tinted towards it (0..=256).
const FILL: u32 = 0;
/// How far its edge is (0..=256).
const LINE: u32 = 130;
/// Only keys: other things (bones behind a shield on an altar) shone
/// through what covers them like a fault.
const KEY: u32 = 82;
/// Hidden when at most this share (percent) of it can be seen.
const VISIBLE: usize = 50;

fn blend(px: u32, k: u32) -> u32 {
    let mix = |c: u32, t: u32| (c * (256 - k) + t * k) >> 8;
    (mix((px >> 16) & 0xFF, EDGE.0) << 16) | (mix((px >> 8) & 0xFF, EDGE.1) << 8) | mix(px & 0xFF, EDGE.2)
}

/// Where a painted item lies on screen and which of its pixels are its own.
struct Figure {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    /// Opaque pixels of the frame, row by row (already mirrored).
    body: Vec<bool>,
}

impl Figure {
    fn at(&self, col: i32, row: i32) -> bool {
        col >= 0 && row >= 0 && col < self.width && row < self.height && self.body[(row * self.width + col) as usize]
    }

    /// Does it cover this screen pixel?
    fn covers(&self, x: i32, y: i32) -> bool {
        self.at(x - self.left, y - self.top)
    }

    /// Does it cover a pixel this close to this one?
    fn near(&self, x: i32, y: i32, r: i32) -> bool {
        (-r..=r).any(|dy| (-r..=r).any(|dx| self.covers(x + dx, y + dy)))
    }
}

/// A click this many pixels beside a hidden key still reaches it.
const MARGIN: i32 = 2;

impl Game {
    /// The figure of the painted entry `n` (place in `self.view.painted`).
    fn figure(&mut self, n: usize) -> Option<Figure> {
        self.figure_near(n, None)
    }

    /// The same, but only if its frame reaches within `r` of the point
    /// (`near`: x, y, r), so that most entries are passed over quickly.
    fn figure_near(&mut self, n: usize, near: Option<(i32, i32, i32)>) -> Option<Figure> {
        let p = self.view.painted.get(n)?;
        let (shape, frame, flipped, sx, sy) = (p.shape, p.frame, p.flipped, p.sx, p.sy);
        let shape = self.data.shapes.original(shape)?;
        let f = shape.frames.get(frame as usize)?;
        let (w, h) = (f.width, f.height);
        let left = if flipped { sx + f.xoff - w } else { sx - f.xoff };
        if let Some((x, y, r)) = near {
            let top = sy - f.yoff;
            if x + r < left || x - r >= left + w || y + r < top || y - r >= top + h {
                return None;
            }
        }
        let mut body = vec![false; (w * h).max(0) as usize];
        for row in 0..h {
            for col in 0..w {
                let src = if flipped { w - 1 - col } else { col };
                body[(row * w + col) as usize] = f.pixels[(row * w + src) as usize] != TRANSPARENT;
            }
        }
        Some(Figure { left, top: sy - f.yoff, width: w, height: h, body })
    }

    /// Is the painted entry `n` a key that is mostly
    /// hidden? Gives its figure and the screen pixels of it that are hidden.
    fn hidden_thing(&mut self, n: usize) -> Option<(Figure, Vec<(i32, i32)>)> {
        let index = self.view.painted.get(n)?.index;
        let it = self.map().items.get(index).copied()?;
        if it.id == 0 || !it.in_world() || it.is_actor() || it.flags & crate::map::FLG_INVISIBLE != 0 || it.shape != KEY || !self.portable(it.id) {
            return None;
        }
        let fig = self.figure(n)?;
        let ids = self.drawn_ids();
        let (w, h) = (self.view.size.0, self.view.size.1);
        if ids.len() != (w * h) as usize {
            return None;
        }
        let me = index as u32 + 1;
        let (mut total, mut hidden) = (0, Vec::new());
        for row in 0..fig.height {
            for col in 0..fig.width {
                let (x, y) = (fig.left + col, fig.top + row);
                if !fig.at(col, row) || x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                total += 1;
                if ids[(y * w + x) as usize] != me {
                    hidden.push((x, y));
                }
            }
        }
        (total > 0 && (total - hidden.len()) * 100 <= total * VISIBLE).then_some((fig, hidden))
    }

    /// A click at this pixel: if what is drawn on top is not a person and
    /// a key lies hidden behind it right there, that
    /// thing (see `pick`).
    pub(crate) fn pick_hidden(&mut self, x: i32, y: i32, top: Option<u16>) -> Option<u16> {
        if self.view_turned() {
            return top;
        }
        let top_id = top?;
        if self.map().get(top_id).is_some_and(|it| it.is_actor()) {
            return top;
        }
        let y = self.unflip_y(y);
        // the entries drawn at this pixel, the topmost first
        let mut first = None;
        for n in (0..self.view.painted.len()).rev() {
            let Some(fig) = self.figure_near(n, Some((x, y, MARGIN))) else { continue };
            let id =self.map().items.get(self.view.painted[n].index).map_or(0, |it| it.id);
            let Some(top_size) = first else {
                if id == top_id && fig.covers(x, y) {
                    first = Some(fig.body.iter().filter(|&&b| b).count());
                }
                continue;
            };
            if !fig.near(x, y, MARGIN) {
                continue;
            }
            if id != 0 && id != top_id && self.can_drag(id) && self.hidden_thing(n).is_some() && fig.body.iter().filter(|&&b| b).count() < top_size {
                return Some(id);
            }
        }
        top
    }

    /// Draws the outlines of the hidden keys where they are hidden.
    pub fn draw_hidden_things(&mut self, surf: &mut Surface) {
        if self.view_turned() || surf.width != self.view.size.0 || surf.height != self.view.size.1 {
            return;
        }
        for n in 0..self.view.painted.len() {
            let index = self.view.painted[n].index;
            let Some(it) = self.map().items.get(index).copied() else { continue };
            // only within reach: no peeking into houses through roofs and walls
            let wanted = self.can_drag(it.id);
            if !wanted {
                continue;
            }
            let Some((fig, hidden)) = self.hidden_thing(n) else { continue };
            for (x, y) in hidden {
                let (col, row) = (x - fig.left, y - fig.top);
                let edge = !(fig.at(col - 1, row) && fig.at(col + 1, row) && fig.at(col, row - 1) && fig.at(col, row + 1));
                let i = (y * surf.width + x) as usize;
                surf.pixels[i] = blend(surf.pixels[i], if edge { LINE } else { FILL });
            }
        }
    }
}
