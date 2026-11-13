//! Renders the world around a camera position into an RGB frame buffer.

use crate::map::{Item, FLG_FLIPPED, FLG_INVISIBLE};
use crate::palette::Palette;
use crate::shape::{Frame, ShapeInfo, Shapes, TRANSPARENT};
use crate::sorter::{FrameDims, Sorter};

pub struct Surface {
    pub width: i32,
    pub height: i32,
    /// 0x00RRGGBB
    pub pixels: Vec<u32>,
}

impl Surface {
    pub fn new(width: i32, height: i32) -> Surface {
        Surface { width, height, pixels: vec![0; (width * height) as usize] }
    }

    /// Fills the rectangle from (`x0`, `y0`) up to (not including) (`x1`,
    /// `y1`); what lies outside the surface is left out.
    pub fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
        for y in y0.max(0)..y1.min(self.height) {
            for x in x0.max(0)..x1.min(self.width) {
                self.pixels[(y * self.width + x) as usize] = c;
            }
        }
    }

    /// Darkens the picture to a quarter (the world behind a menu).
    pub fn dim(&mut self) {
        for p in &mut self.pixels {
            *p = (*p >> 2) & 0x3F3F3F;
        }
    }

    /// Turns the picture upside down (swaps the rows).
    pub fn flip_rows(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        for y in 0..h / 2 {
            let (top, bottom) = self.pixels.split_at_mut((h - 1 - y) * w);
            top[y * w..(y + 1) * w].swap_with_slice(&mut bottom[..w]);
        }
    }

    /// Sets one pixel; outside the surface nothing happens.
    pub fn put(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = c;
        }
    }

    /// Paints a frame half see-through (an invisible Avatar).
    pub fn paint_faint(&mut self, f: &Frame, x: i32, y: i32, flipped: bool, pal: &Palette) {
        let y0 = y - f.yoff;
        for row in 0..f.height {
            let dy = y0 + row;
            if dy < 0 || dy >= self.height {
                continue;
            }
            for col in 0..f.width {
                let c = f.pixels[(row * f.width + col) as usize];
                let dx = if flipped { x + f.xoff - 1 - col } else { x - f.xoff + col };
                if c == TRANSPARENT || dx < 0 || dx >= self.width {
                    continue;
                }
                let p = &mut self.pixels[(dy * self.width + dx) as usize];
                // half figure, half background
                let mix = |s: u32, d: u32| (s + d) / 2;
                let s = pal.colors[c as usize];
                *p = (mix((s >> 16) & 0xFF, (*p >> 16) & 0xFF) << 16) | (mix((s >> 8) & 0xFF, (*p >> 8) & 0xFF) << 8) | mix(s & 0xFF, *p & 0xFF);
            }
        }
    }

    /// Paints a frame with its origin at (x, y).
    pub fn paint(&mut self, f: &Frame, x: i32, y: i32, flipped: bool, trans: bool, pal: &Palette) {
        let y0 = y - f.yoff;
        for row in 0..f.height {
            let dy = y0 + row;
            if dy < 0 || dy >= self.height {
                continue;
            }
            let src = &f.pixels[(row * f.width) as usize..((row + 1) * f.width) as usize];
            let line = &mut self.pixels[(dy * self.width) as usize..((dy + 1) * self.width) as usize];
            for (col, &c) in src.iter().enumerate() {
                if c == TRANSPARENT {
                    continue;
                }
                let dx = if flipped { x + f.xoff - 1 - col as i32 } else { x - f.xoff + col as i32 };
                if dx < 0 || dx >= self.width {
                    continue;
                }
                let (r, g, b, a) = pal.xform[c as usize];
                line[dx as usize] = if trans && a != 0 {
                    // pre-multiplied blend over the background
                    let d = line[dx as usize];
                    let ia = 256 - a;
                    let ch = |dv: u32, sv: u32| ((dv * ia + 256 * sv) >> 8).min(255);
                    (ch((d >> 16) & 0xFF, r) << 16) | (ch((d >> 8) & 0xFF, g) << 8) | ch(d & 0xFF, b)
                } else {
                    pal.colors[c as usize]
                };
            }
        }
    }
}

pub struct Camera {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Default)]
pub struct ViewOptions {
    pub show_editor_items: bool,
    /// Hide everything at or above this height (the roof over the Avatar).
    pub z_limit: Option<i32>,
    /// An item to leave out (for tests).
    pub hide: u16,
    /// Items drawn see-through (index into the items, strength 0..=256):
    /// creatures fading in.
    pub fading: Vec<(usize, u32)>,
    /// The figure climbing onto a ledge (index into the items) and the
    /// direction it climbs in (0 = north, clockwise): it is seen in front
    /// of the ledge (see `climbed_ledge`).
    pub climbing: Option<(usize, u8)>,
}

/// An item as it was drawn, for finding what is under the mouse pointer.
pub struct Painted {
    /// Index into the item list.
    pub index: usize,
    pub sx: i32,
    pub sy: i32,
    pub shape: u32,
    pub frame: u32,
    pub flipped: bool,
}

/// Something drawn on top of one item, in the same place of the paint
/// order: the weapon in the Avatar's hand.
pub struct Overlay {
    /// Index of the item it belongs to.
    pub index: usize,
    pub shape: u32,
    pub frame: u32,
    /// Offset from the item's position on screen.
    pub dx: i32,
    pub dy: i32,
}

/// Draws all items visible from the camera. Returns them in paint order.
/// `ids`: filled with the item drawn at every pixel (index into `items`
/// + 1; 0 where nothing is).
pub fn render(surf: &mut Surface, cam: &Camera, items: &[Item], infos: &[ShapeInfo], shapes: &mut Shapes,
              pal: &Palette, opts: &ViewOptions, overlay: Option<&Overlay>, ids: &mut Vec<u32>) -> Vec<Painted> {
    surf.pixels.fill(0);
    ids.clear();
    ids.resize((surf.width * surf.height) as usize, 0);
    let cam_sx = (cam.x - cam.y) / 4 - surf.width / 2;
    let cam_sy = (cam.x + cam.y) / 8 - cam.z - surf.height / 2;

    // Shapes reach at most a few hundred pixels from their origin.
    const MARGIN: i32 = 512;
    let mut sorter = Sorter::new(cam_sx, cam_sy, surf.width, surf.height);
    for (id, it) in items.iter().enumerate() {
        if !it.in_world() || (opts.hide != 0 && it.id == opts.hide) {
            continue;
        }
        let sx = (it.x - it.y) / 4 - cam_sx;
        let sy = (it.x + it.y) / 8 - it.z - cam_sy;
        if sx < -MARGIN || sx > surf.width + MARGIN || sy < -MARGIN || sy > surf.height + MARGIN {
            continue;
        }
        let Some(info) = infos.get(it.shape as usize) else { continue };
        if info.editor && !opts.show_editor_items {
            continue;
        }
        if opts.z_limit.is_some_and(|limit| it.z >= limit && !info.draw) {
            continue;
        }
        // invisible things are not drawn, except the Avatar: faintly
        let ghost = it.flags & FLG_INVISIBLE != 0;
        if ghost && it.id != 1 {
            continue;
        }
        let Some(shape) = shapes.get(it.shape) else { continue };
        let Some(frame) = shape.frames.get(it.frame as usize) else { continue };
        if frame.width == 0 {
            continue;
        }
        let dims = FrameDims { width: frame.width, height: frame.height, xoff: frame.xoff, yoff: frame.yoff };
        sorter.add(id, (it.x, it.y, it.z), it.shape, it.frame, it.flags & FLG_FLIPPED != 0, ghost, info, &dims);
    }

    let order = sorter.paint_order();
    // the shapes stay held while the frames are painted
    let mut held: Vec<std::rc::Rc<crate::shape::Shape>> = Vec::with_capacity(order.len() + 1);
    let mut jobs: Vec<(usize, u32, u32, i32, i32, bool, Look)> = Vec::with_capacity(order.len() + 1);
    for si in &order {
        if let Some(shape) = shapes.get(si.shape) {
            if (si.frame as usize) < shape.frames.len() {
                let look = if si.ghost {
                    Look::Faint(128)
                } else if let Some(&(_, a)) = opts.fading.iter().find(|f| f.0 == si.id) {
                    Look::Faint(a)
                } else if si.trans { Look::Blended } else { Look::Plain };
                jobs.push((held.len(), si.frame, si.id as u32 + 1, si.sx, si.sy, si.flipped, look));
                held.push(shape);
            }
        }
        if let Some(o) = overlay.filter(|o| o.index == si.id) {
            if let Some(shape) = shapes.get(o.shape) {
                if (o.frame as usize) < shape.frames.len() {
                    jobs.push((held.len(), o.frame, si.id as u32 + 1, si.sx + o.dx, si.sy + o.dy, false, Look::Plain));
                    held.push(shape);
                }
            }
        }
    }
    let jobs: Vec<PaintJob> = jobs
        .into_iter()
        .map(|(h, frame, id, x, y, flipped, look)| PaintJob { frame: &held[h].frames[frame as usize], x, y, flipped, look, id })
        .collect();
    paint_all(surf, ids, &jobs, pal);
    drop(jobs);
    if let Some((figure, dir)) = opts.climbing {
        paint_climber(surf, ids, items, infos, shapes, pal, overlay, (cam_sx, cam_sy), figure, dir);
    }
    order.iter().map(|si| Painted { index: si.id, sx: si.sx, sy: si.sy, shape: si.shape, frame: si.frame, flipped: si.flipped }).collect()
}

/// For a figure climbing onto a ledge: is `t` part of the ledge (its edge,
/// roof or floor up at about the figure's head, ahead of it and not
/// clearly in front of it)?
///
/// The ledge is what lies ahead, and the edge right over him that he
/// pulls himself over: the eaves of the roof he climbs onto overhang the
/// spot he climbs from, a strip running across his way. Where two houses
/// meet, the eaves of the other one hang over that corner too, but run
/// along his way (or lie beside it): they are not what he climbs, and he
/// stays hidden behind them.
fn climbed_ledge(t: &Item, t_info: &ShapeInfo, figure: &Item, figure_info: &ShapeInfo, dir: u8) -> bool {
    const DIRS: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
    /// How far the ledge may reach back over the figure's box.
    const REACH: i32 = 32;
    let (dx, dy) = DIRS[dir as usize % 8];
    let (txd, tyd, _) = t_info.footpad(t.flags & FLG_FLIPPED != 0);
    let (fxd, fyd, fzd) = figure_info.footpad(figure.flags & FLG_FLIPPED != 0);
    let (t_left, t_far) = (t.x - txd, t.y - tyd);
    let (f_left, f_far, f_top) = (figure.x - fxd, figure.y - fyd, figure.z + fzd);
    let ahead_x = (dx < 0 && t.x <= f_left + REACH) || (dx > 0 && t_left >= figure.x - REACH);
    let ahead_y = (dy < 0 && t.y <= f_far + REACH) || (dy > 0 && t_far >= figure.y - REACH);
    // over him, across his way (climbing along x: a strip along y)
    let over = t.x > f_left && t.y > f_far && ((dy == 0 && tyd > txd) || (dx == 0 && txd > tyd));
    let standing = t_info.creature && !t_info.lies_in(t.frame);
    (ahead_x || ahead_y || over) && !standing && t.z > figure.z && t.z <= f_top + 16 && t_far < figure.y && t_left < figure.x
}

/// The figure climbing onto a ledge is seen in front of it: its picture
/// already reaches up there with head and arms while its box is still
/// below, and the edge of a roof was painted over the Avatar climbing
/// onto it. So after everything is painted in its order, the figure is
/// painted once more wherever the picture shows the ledge. Whatever else
/// hides him (the roof of the house next door) goes on hiding him, and
/// nothing changes its place in the order for him. (Putting him behind
/// the ledge in the order instead closed circles with what stood between
/// the two: a wall beside him was painted over the roof while he climbed.)
fn paint_climber(surf: &mut Surface, ids: &mut [u32], items: &[Item], infos: &[ShapeInfo], shapes: &mut Shapes, pal: &Palette,
                 overlay: Option<&Overlay>, (cam_sx, cam_sy): (i32, i32), figure: usize, dir: u8) {
    let Some(fig) = items.get(figure) else { return };
    let Some(fig_info) = infos.get(fig.shape as usize) else { return };
    // (an invisible Avatar is only faintly there: not brought forward)
    if !fig.in_world() || fig.flags & FLG_INVISIBLE != 0 {
        return;
    }
    let ledge: Vec<bool> = items
        .iter()
        .map(|t| t.in_world() && infos.get(t.shape as usize).is_some_and(|info| climbed_ledge(t, info, fig, fig_info, dir)))
        .collect();
    let (sx, sy) = (fig.x / 4 - fig.y / 4 - cam_sx, fig.x / 8 + fig.y / 8 - fig.z - cam_sy);
    let id = figure as u32 + 1;
    let mut paint = |shape: u32, frame: u32, x: i32, y: i32, flipped: bool| {
        let Some(shape) = shapes.get(shape) else { return };
        let Some(f) = shape.frames.get(frame as usize) else { return };
        let (x0, y0) = (if flipped { x + f.xoff - f.width } else { x - f.xoff }, y - f.yoff);
        for row in (-y0).max(0)..(surf.height - y0).min(f.height) {
            for col in 0..f.width {
                let c = f.pixels[(row * f.width + if flipped { f.width - 1 - col } else { col }) as usize];
                let dx = x0 + col;
                if c == TRANSPARENT || dx < 0 || dx >= surf.width {
                    continue;
                }
                let at = ((y0 + row) * surf.width + dx) as usize;
                if ids[at] != 0 && ledge[ids[at] as usize - 1] {
                    surf.pixels[at] = pal.colors[c as usize];
                    ids[at] = id;
                }
            }
        }
    };
    paint(fig.shape, fig.frame, sx, sy, fig.flags & FLG_FLIPPED != 0);
    if let Some(o) = overlay.filter(|o| o.index == figure) {
        paint(o.shape, o.frame, sx + o.dx, sy + o.dy, false);
    }
}

/// Paint the world in one band (for the test command bandcheck).
pub static ONE_BAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How a frame is put onto the picture.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Plain,
    /// Its see-through colours blended over what is behind.
    Blended,
    /// See-through as a whole, this strong (0..=256): half for the
    /// invisible Avatar, rising for a creature fading in.
    Faint(u32),
}

/// One frame to paint, in paint order.
struct PaintJob<'a> {
    frame: &'a Frame,
    x: i32,
    y: i32,
    flipped: bool,
    look: Look,
    /// Noted in the pixel ids where it covers the picture.
    id: u32,
}

/// Paints the frames in their order, spread over the processor cores:
/// each core takes bands of rows and paints every frame clipped to its
/// band, so the picture comes out exactly as painted one after the other.
fn paint_all(surf: &mut Surface, ids: &mut [u32], jobs: &[PaintJob], pal: &Palette) {
    let (w, h) = (surf.width, surf.height);
    if w <= 0 || h <= 0 {
        return;
    }
    // more bands than cores, so that a busy one is made up for by the others
    // (one band for the test command bandcheck)
    let band = if ONE_BAND.load(std::sync::atomic::Ordering::Relaxed) { h as usize } else { (h as usize).div_ceil(crate::pool::cores() * 2).max(16) };
    let len = band * w as usize;
    let bands: Vec<_> = surf.pixels.chunks_mut(len).zip(ids.chunks_mut(len)).collect();
    crate::pool::each(bands, |bi, (pixels, ids)| {
        let top = (bi * band) as i32;
        let rows = (top, top + (pixels.len() / w as usize) as i32);
        for j in jobs {
            if let Look::Faint(a) = j.look {
                paint_faint(pixels, ids, w, rows, j.frame, j.x, j.y, j.flipped, pal, j.id, a);
            } else {
                paint_marked(pixels, ids, w, rows, j.frame, j.x, j.y, j.flipped, j.look == Look::Blended, pal, j.id);
            }
        }
    });
}

/// Paints a frame see-through (`alpha` 0..=256: how much of the figure
/// over the background) into the band `rows` (first, past the last) of a
/// picture `w` wide, and notes its pixels in `ids`.
fn paint_faint(pixels: &mut [u32], ids: &mut [u32], w: i32, rows: (i32, i32), f: &Frame, x: i32, y: i32, flipped: bool, pal: &Palette, id: u32, alpha: u32) {
    let y0 = y - f.yoff;
    for row in (rows.0 - y0).max(0)..(rows.1 - y0).min(f.height) {
        let at = (y0 + row - rows.0) * w;
        for col in 0..f.width {
            let c = f.pixels[(row * f.width + col) as usize];
            let dx = if flipped { x + f.xoff - 1 - col } else { x - f.xoff + col };
            if c == TRANSPARENT || dx < 0 || dx >= w {
                continue;
            }
            let p = &mut pixels[(at + dx) as usize];
            let mix = |s: u32, d: u32| (s * alpha + d * (256 - alpha)) >> 8;
            let s = pal.colors[c as usize];
            *p = (mix((s >> 16) & 0xFF, (*p >> 16) & 0xFF) << 16) | (mix((s >> 8) & 0xFF, (*p >> 8) & 0xFF) << 8) | mix(s & 0xFF, *p & 0xFF);
            ids[(at + dx) as usize] = id;
        }
    }
}

/// Paints a frame as `Surface::paint` does into the band `rows` (first,
/// past the last) of a picture `w` wide, and notes in `ids` which item
/// covers each pixel, in one pass and with the visible columns worked out
/// once per row (the world view's pictures are the bulk of the drawing).
fn paint_marked(pixels: &mut [u32], ids: &mut [u32], w: i32, rows: (i32, i32), f: &Frame, x: i32, y: i32, flipped: bool, trans: bool, pal: &Palette, id: u32) {
    // columns of the frame that land on the surface
    let (c0, c1) = if flipped {
        // dx = x + xoff - 1 - col
        let base = x + f.xoff - 1;
        ((base - (w - 1)).max(0), (base + 1).min(f.width))
    } else {
        // dx = x - xoff + col
        let base = x - f.xoff;
        ((-base).max(0), (w - base).min(f.width))
    };
    if c0 >= c1 {
        return;
    }
    let y0 = y - f.yoff;
    for row in (rows.0 - y0).max(0)..(rows.1 - y0).min(f.height) {
        let dy = y0 + row - rows.0;
        let src = &f.pixels[(row * f.width) as usize..((row + 1) * f.width) as usize];
        let start = (dy * w) as usize;
        let line = &mut pixels[start..start + w as usize];
        let line_ids = &mut ids[start..start + w as usize];
        for col in c0..c1 {
            let c = src[col as usize];
            if c == TRANSPARENT {
                continue;
            }
            let dx = (if flipped { x + f.xoff - 1 - col } else { x - f.xoff + col }) as usize;
            line[dx] = if trans {
                let (r, g, b, a) = pal.xform[c as usize];
                if a != 0 {
                    // pre-multiplied blend over the background
                    let d = line[dx];
                    let ia = 256 - a;
                    let ch = |dv: u32, sv: u32| ((dv * ia + 256 * sv) >> 8).min(255);
                    (ch((d >> 16) & 0xFF, r) << 16) | (ch((d >> 8) & 0xFF, g) << 8) | ch(d & 0xFF, b)
                } else {
                    pal.colors[c as usize]
                }
            } else {
                pal.colors[c as usize]
            };
            line_ids[dx] = id;
        }
    }
}
