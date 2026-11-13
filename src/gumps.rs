//! Windows on top of the world view ("gumps"): open containers showing
//! their contents, and the reader for books, scrolls and plaques.

use crate::game::{Game, AVATAR};
use crate::map::Item;
use crate::shape::{self, FAMILY_CONTAINER};
use crate::view::Surface;
use crate::vm::Kind;

pub const FLG_GUMP_OPEN: u16 = 0x0100;

/// An open container.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Gump {
    /// The container item.
    pub owner: u16,
    /// Shape in U8GUMPS.FLX.
    pub shape: u32,
    /// Top left corner in the view.
    pub x: i32,
    pub y: i32,
}

/// How a text is shown: the original's book, scroll, plaque or grave
/// stone, or (in a savegame from before) a plain panel.
#[derive(Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Look {
    #[default]
    Panel,
    Book,
    Scroll,
    /// Plaques and grave stones: their gump shape and font, text centred.
    Sign { gump: u32, font: usize },
}

/// Gump shapes and font of the readers (as in the original).
const BOOK_GUMP: u32 = 6;
const SCROLL_GUMP: u32 = 19;
const INK_FONT: usize = 9;
pub const PLAQUE_FONT: usize = 10;
pub const GRAVE_FONT: usize = 11;
/// Text areas inside the gumps: x, y, width, height.
const BOOK_PAGES: [(i32, i32, i32, i32); 2] = [(14, 10, 120, 128), (152, 10, 120, 128)];
const SCROLL_AREA: (i32, i32, i32, i32) = (22, 29, 204, 115);

/// Text the player is reading, page by page.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Reader {
    pub pid: u16,
    lines: Vec<String>,
    page: usize,
    /// Saved behind the rest of the savegame (savegame.rs), so that older
    /// savegames stay readable.
    #[serde(skip)]
    pub look: Look,
}

/// How far (in view pixels) a container may be outside the picture
/// before its window closes: its origin is at its bottom, so a tall
/// cupboard is still partly visible a little beyond the edge.
const GUMP_MARGIN: i32 = 32;

const READER_WIDTH: i32 = 300;
const READER_LINES: usize = 12;

/// How many pixels beside a thing in a container window a click still
/// takes it.
const PICK_SLACK: i32 = 4;

impl Game {
    // ---- containers ------------------------------------------------------

    pub fn open_gump(&mut self, owner: u16, shape: u32) {
        if self.ui.gumps.iter().any(|g| g.owner == owner) || self.data.gump_shapes.get(shape).is_none() {
            return;
        }
        // cascade new windows from the top left
        let n = self.ui.gumps.len() as i32;
        self.ui.gumps.push(Gump { owner, shape, x: 16 + 36 * (n % 6), y: 16 + 24 * (n % 6) });
        if let Some(idx) = self.map().index(owner) {
            self.map_mut().items[idx].flags |= FLG_GUMP_OPEN;
        }
        if self.grid_window(owner) {
            self.join_all_stacks(owner);
        }
    }

    pub fn close_gump(&mut self, owner: u16) {
        self.ui.gumps.retain(|g| g.owner != owner);
        if let Some(idx) = self.map().index(owner) {
            self.map_mut().items[idx].flags &= !FLG_GUMP_OPEN;
        }
    }

    pub fn close_all_gumps(&mut self) {
        let owners: Vec<u16> = self.ui.gumps.iter().map(|g| g.owner).collect();
        for o in owners {
            self.close_gump(o);
        }
    }

    /// Closes the windows of containers that are gone, have left the area
    /// around the Avatar (as in the original, where an item leaving the
    /// fast area closes its window) or are no longer in the picture. A
    /// container inside another one goes with the outermost; what the
    /// Avatar carries stays open.
    pub fn close_far_gumps(&mut self) {
        let far: Vec<u16> = self
            .ui
            .gumps
            .iter()
            .map(|g| g.owner)
            .filter(|&owner| {
                let mut cur = owner;
                for _ in 0..16 {
                    let Some(it) = self.map().get(cur) else { return true };
                    if it.gone {
                        return true;
                    }
                    if cur == AVATAR {
                        return false;
                    }
                    if it.in_world() {
                        // out of the picture (the fast area reaches well beyond it)
                        let (sx, sy) = self.to_screen(it.x, it.y, it.z);
                        let (w, h) = self.view.size;
                        let off = sx < -GUMP_MARGIN || sy < -GUMP_MARGIN || sx > w + GUMP_MARGIN || sy > h + GUMP_MARGIN;
                        return !it.fast || off;
                    }
                    // being carried by the mouse, or nowhere known
                    if it.parent == crate::map::ETHEREAL || it.parent == cur {
                        return false;
                    }
                    cur = it.parent;
                }
                false
            })
            .collect();
        for owner in far {
            self.close_gump(owner);
        }
    }

    /// Screen position of the corner the contents are placed relative to.
    fn gump_item_origin(&mut self, g_index: usize) -> Option<(i32, i32)> {
        let g = &self.ui.gumps[g_index];
        let area = self.data.gump_areas.get(g.shape.checked_sub(1)? as usize).copied().unwrap_or([0; 4]);
        Some((g.x + area[0], g.y + area[1]))
    }

    /// What is said about things in containers (their name, when looked
    /// at) appears over the thing in its window, or over the window.
    pub fn draw_window_barks(&mut self, surf: &mut Surface) {
        let mut shown = Vec::new();
        for bark in &self.barks {
            let Some(it) = self.map().get(bark.item).copied().filter(|it| !it.in_world()) else { continue };
            shown.push((it, bark.text.clone(), bark.pid == 0));
        }
        for (it, text, name) in shown {
            let (x, y) = self.window_bark_place(it).unwrap_or((surf.width / 2, surf.height / 3));
            // things speak in yellow (pid 0: the name under the pointer)
            let ts = if name { self.name_scale() } else { self.text_scale() };
            let Some(font) = self.data.font_or_plain(8) else { continue };
            font.draw_centered_at(surf, &text, x, y, (180.0 * ts) as i32, &self.data.pal, ts);
        }
    }

    /// Where something said by a thing in a container shows: over the
    /// window of its container, else of a container further out, else
    /// over whoever carries it or the container in the world.
    fn window_bark_place(&mut self, it: Item) -> Option<(i32, i32)> {
        let mut cur = it;
        for _ in 0..8 {
            if let Some(gi) = self.ui.gumps.iter().rposition(|g| g.owner == cur.parent) {
                return self.bark_place_in_window(gi, cur, it);
            }
            cur = self.map().get(cur.parent).copied()?;
            if cur.in_world() {
                // no window open: over whoever carries it, or the container in the world
                let height = self.data.infos.get(cur.shape as usize).map_or(0, |si| si.z * 8);
                let (sx, sy) = self.to_screen(cur.x, cur.y, cur.z);
                // upside down the feet are the top of the picture
                return Some(if self.inverted { (sx, self.unflip_y(sy) - 8) } else { (sx, sy - height - 8) });
            }
        }
        None
    }

    /// Where something said by `it` shows on window `gi`, which belongs to
    /// `cur`'s container (`cur` is `it` or a container it is in).
    fn bark_place_in_window(&mut self, gi: usize, cur: Item, it: Item) -> Option<(i32, i32)> {
        if cur.id == it.id && self.grid_window(cur.parent) {
            // over its slot, or over the window if it is on another page
            let (gx, gy) = (self.ui.gumps[gi].x, self.ui.gumps[gi].y);
            return self.grid_anchor(gi, it.id).or(Some((gx + crate::inventory::WIDTH / 2, gy - 2)));
        }
        if cur.id == it.id && cur.parent != AVATAR {
            // right over the thing
            let top = self.data.shapes.get(it.shape).and_then(|s| s.frames.get(it.frame as usize).map(|f| f.yoff)).unwrap_or(0);
            return self.gump_item_origin(gi).map(|(ox, oy)| (ox + (it.y & 0xFF), oy + ((it.y >> 8) & 0xFF) - top - 2));
        }
        let g = &self.ui.gumps[gi];
        let w = if self.grid_window(g.owner) {
            crate::inventory::WIDTH
        } else {
            self.data.gump_shapes.get(g.shape).and_then(|s| s.frames.first().map(|f| f.width)).unwrap_or(0)
        };
        Some((g.x + w / 2, g.y - 2))
    }

    /// Ids of the items directly inside a container, in drawing order.
    fn contents(&self, owner: u16) -> Vec<u16> {
        if owner == AVATAR {
            return Vec::new(); // worn items are drawn in their places instead
        }
        self.map().items.iter().filter(|it| !it.gone && it.parent == owner && it.id != 0).map(|it| it.id).collect()
    }

    /// Things a script put into a container have no place in its window
    /// yet (0,0 or 255,255): as in the original they get a random one
    /// inside the window's item area when it is shown.
    fn place_unplaced(&mut self, owner: u16, gshape: u32) {
        let area = self.data.gump_areas.get(gshape.saturating_sub(1) as usize).copied().unwrap_or([0, 0, 64, 64]);
        let (w, h) = ((area[2] - area[0]).max(1), (area[3] - area[1]).max(1));
        for id in self.contents(owner) {
            let Some(idx) = self.map().index(id) else { continue };
            let pos = self.map().items[idx].y & 0xFFFF;
            if pos != 0 && pos != 0xFFFF {
                continue;
            }
            // the picture hangs up and left of its point: keep a margin there
            let (mx, my) = (8.min(w / 2), 8.min(h / 2));
            let gx = mx + (self.rng.next() % (w - mx).max(1) as u32) as i32;
            let gy = my + (self.rng.next() % (h - my).max(1) as u32) as i32;
            self.map_mut().items[idx].y = gx.min(255) | gy.min(255) << 8;
        }
    }

    pub fn draw_gumps(&mut self, surf: &mut Surface) {
        // containers that were destroyed take their window with them
        let gone: Vec<u16> = self.ui.gumps.iter().map(|g| g.owner).filter(|&o| self.map().get(o).is_none()).collect();
        for o in gone {
            self.close_gump(o);
        }
        for gi in 0..self.ui.gumps.len() {
            let (owner, gshape, gx, gy) = { let g = &self.ui.gumps[gi]; (g.owner, g.shape, g.x, g.y) };
            if self.grid_window(owner) {
                self.draw_grid(surf, gi);
                continue;
            }
            if let Some(shape) = self.data.gump_shapes.get(gshape) {
                if let Some(f) = shape.frames.first() {
                    surf.paint(f, gx + f.xoff, gy + f.yoff, false, false, &self.data.pal);
                }
            }
            if owner == AVATAR {
                self.draw_paperdoll(surf, gx, gy);
            }
            let Some((ox, oy)) = self.gump_item_origin(gi) else { continue };
            self.place_unplaced(owner, gshape);
            for id in self.contents(owner) {
                let Some(it) = self.map().get(id).copied() else { continue };
                if self.data.infos.get(it.shape as usize).is_some_and(|si| si.editor) {
                    continue;
                }
                // inside a container the y field holds the position in the window
                let (ix, iy) = (it.y & 0xFF, (it.y >> 8) & 0xFF);
                if let Some(shape) = self.data.shapes.get(it.shape) {
                    if let Some(f) = shape.frames.get(it.frame as usize) {
                        surf.paint(f, ox + ix, oy + iy, false, false, &self.data.pal);
                    }
                }
            }
        }
        self.draw_practice_hint(surf);
    }

    /// What an open container shows at a view position: `Some(Some(id))`
    /// for an item, `Some(None)` for the window itself, `None` if no
    /// window is there.
    pub fn pick_gump(&mut self, x: i32, y: i32) -> Option<(usize, Option<u16>)> {
        for gi in (0..self.ui.gumps.len()).rev() {
            let (owner, gshape, gx, gy) = { let g = &self.ui.gumps[gi]; (g.owner, g.shape, g.x, g.y) };
            if self.grid_window(owner) {
                match self.grid_hit(gi, x, y) {
                    Some(crate::inventory::GridHit::Slot(s)) => return Some((gi, self.grid_item(owner, s))),
                    Some(_) => return Some((gi, None)),
                    None => continue,
                }
            }
            if owner == AVATAR {
                if let Some(id) = self.pick_paperdoll(x, y, gx, gy) {
                    return Some((gi, Some(id)));
                }
            }
            if let Some((ox, oy)) = self.gump_item_origin(gi) {
                // on one of its pixels; failing that, a little beside one
                // (small things are easy to miss, and a miss would take
                // hold of the whole window)
                let mut near: Option<(i32, u16)> = None;
                for id in self.contents(owner).into_iter().rev() {
                    let Some(it) = self.map().get(id).copied() else { continue };
                    let Some(shape) = self.data.shapes.get(it.shape) else { continue };
                    let Some(f) = shape.frames.get(it.frame as usize) else { continue };
                    let (col, row) = (x - (ox + (it.y & 0xFF) - f.xoff), y - (oy + ((it.y >> 8) & 0xFF) - f.yoff));
                    if col >= 0 && row >= 0 && col < f.width && row < f.height
                        && f.pixels[(row * f.width + col) as usize] != shape::TRANSPARENT
                    {
                        return Some((gi, Some(id)));
                    }
                    // how far off its picture
                    let dx = if col < 0 { -col } else if col >= f.width { col - f.width + 1 } else { 0 };
                    let dy = if row < 0 { -row } else if row >= f.height { row - f.height + 1 } else { 0 };
                    let d = dx.max(dy);
                    if d <= PICK_SLACK && near.is_none_or(|(nd, _)| d < nd) {
                        near = Some((d, id));
                    }
                }
                if let Some((_, id)) = near {
                    return Some((gi, Some(id)));
                }
            }
            if let Some(shape) = self.data.gump_shapes.get(gshape) {
                if let Some(f) = shape.frames.first() {
                    let (col, row) = (x - gx, y - gy);
                    if col >= 0 && row >= 0 && col < f.width && row < f.height
                        && f.pixels[(row * f.width + col) as usize] != shape::TRANSPARENT
                    {
                        return Some((gi, None));
                    }
                }
            }
        }
        None
    }

    /// The player takes hold of a window to move it: the window at a view
    /// position, if the pointer is on the window itself and not on
    /// something in it.  It comes to the front; returned are its owner
    /// and where in the window it is held.
    pub fn grab_gump(&mut self, x: i32, y: i32) -> Option<(u16, i32, i32)> {
        let (gi, None) = self.pick_gump(x, y)? else { return None };
        // the slot window is taken hold of by its frame, not by an empty slot
        let owner = self.ui.gumps[gi].owner;
        if self.grid_window(owner) && matches!(self.grid_hit(gi, x, y), Some(crate::inventory::GridHit::Slot(_))) {
            return None;
        }
        let g = self.ui.gumps.remove(gi);
        let held = (g.owner, x - g.x, y - g.y);
        self.ui.gumps.push(g);
        Some(held)
    }

    /// Moves a held window with the pointer. A part of it always stays in
    /// the view, so that it can be taken hold of again.
    pub fn move_gump(&mut self, held: (u16, i32, i32), x: i32, y: i32, view: (i32, i32)) -> bool {
        let (owner, dx, dy) = held;
        let Some(gi) = self.ui.gumps.iter().position(|g| g.owner == owner) else { return false };
        let (w, h) = if self.grid_window(owner) {
            (crate::inventory::WIDTH, crate::inventory::HEIGHT)
        } else {
            self.data.gump_shapes.get(self.ui.gumps[gi].shape).map_or((40, 40), |s| s.frames.first().map_or((40, 40), |f| (f.width, f.height)))
        };
        let g = &mut self.ui.gumps[gi];
        let to = ((x - dx).clamp(24 - w, view.0 - 24), (y - dy).clamp(24 - h, view.1 - 24));
        let moved = to != (g.x, g.y);
        (g.x, g.y) = to;
        moved
    }

    /// Closes the container window at a view position. Returns true if there was one.
    pub fn close_gump_at(&mut self, x: i32, y: i32) -> bool {
        match self.pick_gump(x, y) {
            Some((gi, _)) => {
                let owner = self.ui.gumps[gi].owner;
                self.close_gump(owner);
                true
            }
            None => false,
        }
    }

    /// The player asks for the inventory: use the Avatar's backpack.
    pub fn open_inventory(&mut self) {
        let pack = self.map().items.iter().find(|it| {
            !it.gone && it.parent == AVATAR && self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_CONTAINER)
        });
        if let Some(id) = pack.map(|it| it.id) {
            if self.ui.gumps.iter().any(|g| g.owner == id) {
                self.close_gump(id);
            } else {
                self.interact(id, true);
            }
        }
    }

    // ---- reading ---------------------------------------------------------

    /// Shows text for the player to read; the returned process ends when
    /// the last page is dismissed.
    pub fn read_text(&mut self, item: u16, text: &str, look: Look) -> u32 {
        if let Some(g) = Self::reader_gump(look) {
            // loaded here, so that drawing can use it
            self.data.gump_shapes.get(g);
        }
        let look = if self.reader_font(look).is_some() && self.reader_frame(look).is_some() { look } else { Look::Panel };
        let width = match look {
            Look::Panel => READER_WIDTH - 24,
            Look::Book => BOOK_PAGES[0].2,
            Look::Scroll => SCROLL_AREA.2,
            Look::Sign { .. } => self.reader_frame(look).map_or(200, |(w, _)| w - 16),
        };
        let Some(font) = self.reader_font(look) else { return 0 };
        // the texts use '*' for line breaks
        let mut lines = Vec::new();
        for para in text.split('*') {
            let wrapped = font.wrap(para, width);
            if wrapped.is_empty() && !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(wrapped);
        }
        if lines.is_empty() {
            return 0;
        }
        if let Some(old) = self.ui.reader.take() {
            self.kill_pid(old.pid);
        }
        let pid = self.add_native(Kind::Ask, item, 0);
        self.ui.reader = Some(Reader { pid, lines, page: 0, look });
        pid as u32
    }

    /// Ends all speech on screen at once, so the scripts waiting for it
    /// carry on. Returns false if nobody was speaking.
    pub fn skip_barks(&mut self) -> bool {
        self.audio.stop_speech();
        let pids: Vec<u16> = self.barks.iter().map(|b| b.pid).collect();
        for &pid in &pids {
            self.kill_pid(pid);
        }
        !pids.is_empty()
    }

    fn reader_font(&self, look: Look) -> Option<&crate::font::Font> {
        let n = match look {
            Look::Panel => return self.data.font.as_ref(),
            Look::Book | Look::Scroll => INK_FONT,
            Look::Sign { font, .. } => font,
        };
        self.data.fonts.get(n).and_then(|f| f.as_ref())
    }

    pub fn reader_gump(look: Look) -> Option<u32> {
        match look {
            Look::Panel => None,
            Look::Book => Some(BOOK_GUMP),
            Look::Scroll => Some(SCROLL_GUMP),
            Look::Sign { gump, .. } => Some(gump),
        }
    }

    /// Size of the reader's picture (None for the plain panel or if the
    /// picture is missing).
    fn reader_frame(&self, look: Look) -> Option<(i32, i32)> {
        let shape = self.data.gump_shapes.peek(Self::reader_gump(look)?)?;
        let f = shape.frames.first()?;
        Some((f.width, f.height))
    }

    /// How many lines one turn of the page shows.
    fn reader_lines_per_page(&self, look: Look) -> usize {
        let fh = self.reader_font(look).map_or(10, |f| f.height.max(1));
        match look {
            Look::Panel => READER_LINES,
            Look::Book => 2 * (BOOK_PAGES[0].3 / fh) as usize,
            Look::Scroll => (SCROLL_AREA.3 / fh) as usize,
            Look::Sign { .. } => usize::MAX / 2,
        }
        .max(1)
    }

    /// Closes the text being read at once.
    pub fn close_reader(&mut self) {
        if let Some(r) = self.ui.reader.take() {
            self.kill_pid(r.pid);
        }
    }

    /// Whether the open book is one of Pagan's own (the journal, the spell
    /// book): its pages can be turned back and forth, a right click closes
    /// it. The game's own books (opened by a script for a thing) read on
    /// as in the original.
    pub fn reader_browses(&self) -> bool {
        let Some(r) = &self.ui.reader else { return false };
        r.look == Look::Book && self.scripts.procs.iter().any(|p| p.pid == r.pid && p.item == 0)
    }

    /// The page before (nothing on the first).
    pub fn reader_back(&mut self) {
        if let Some(r) = &mut self.ui.reader {
            r.page = r.page.saturating_sub(1);
        }
    }

    /// The page after, but the last one stays open (the wheel, the arrows).
    pub fn reader_on(&mut self) {
        let Some(look) = self.ui.reader.as_ref().map(|r| r.look) else { return };
        let per_page = self.reader_lines_per_page(look);
        if let Some(r) = &mut self.ui.reader {
            if (r.page + 1) * per_page < r.lines.len() {
                r.page += 1;
            }
        }
    }

    /// Which page of the open book is at a point of the view: -1 the left
    /// one, 1 the right one, 0 beside the book.
    pub fn reader_side_at(&self, x: i32, y: i32, surf_w: i32, surf_h: i32) -> i32 {
        let Some(look) = self.ui.reader.as_ref().map(|r| r.look) else { return 0 };
        let Some((w, h)) = self.reader_frame(look) else { return 0 };
        let (x0, y0) = ((surf_w - w) / 2, (surf_h - h) / 2);
        if x < x0 || y < y0 || x >= x0 + w || y >= y0 + h {
            0
        } else if x < x0 + w / 2 {
            -1
        } else {
            1
        }
    }

    /// Next page, or close after the last one.
    pub fn reader_next(&mut self) {
        let Some(look) = self.ui.reader.as_ref().map(|r| r.look) else { return };
        let per_page = self.reader_lines_per_page(look);
        let Some(r) = &mut self.ui.reader else { return };
        r.page += 1;
        if r.page * per_page >= r.lines.len() {
            let pid = r.pid;
            self.ui.reader = None;
            self.kill_pid(pid);
        }
    }

    pub fn draw_reader(&self, surf: &mut Surface) {
        let Some(r) = &self.ui.reader else { return };
        if r.look != Look::Panel {
            self.draw_reader_gump(surf, r);
            return;
        }
        let Some(font) = &self.data.font else { return };
        let height = READER_LINES as i32 * font.height + 28;
        let (x0, y0) = ((surf.width - READER_WIDTH) / 2, (surf.height - height) / 2);
        // a darkened panel with a border
        for y in y0.max(0)..(y0 + height).min(surf.height) {
            for x in x0.max(0)..(x0 + READER_WIDTH).min(surf.width) {
                let p = &mut surf.pixels[(y * surf.width + x) as usize];
                let edge = x == x0 || y == y0 || x == x0 + READER_WIDTH - 1 || y == y0 + height - 1;
                *p = if edge { 0x80705C } else { (*p >> 2) & 0x3F3F3F };
            }
        }
        let pages = r.lines.len().div_ceil(READER_LINES);
        for (i, line) in r.lines.iter().skip(r.page * READER_LINES).take(READER_LINES).enumerate() {
            font.draw(surf, line, x0 + 12, y0 + 8 + i as i32 * font.height, &self.data.pal);
        }
        if pages > 1 {
            let hint = format!("{} - {}", r.page + 1, pages);
            font.draw(surf, &hint, x0 + READER_WIDTH - 12 - font.width(&hint), y0 + height - font.height - 6, &self.data.pal);
        }
    }

    /// The original's look: the book with two pages, the scroll, or a
    /// plaque or grave stone with the text in the middle.
    fn draw_reader_gump(&self, surf: &mut Surface, r: &Reader) {
        let (Some(font), Some(gump)) = (self.reader_font(r.look), Self::reader_gump(r.look)) else { return };
        let Some(shape) = self.data.gump_shapes.peek(gump) else { return };
        let Some(f) = shape.frames.first() else { return };
        let (x0, y0) = ((surf.width - f.width) / 2, (surf.height - f.height) / 2);
        surf.paint(f, x0 + f.xoff, y0 + f.yoff, false, false, &self.data.pal);
        let per_page = self.reader_lines_per_page(r.look);
        let shown: Vec<&String> = r.lines.iter().skip(r.page * per_page).take(per_page).collect();
        let fh = font.height;
        match r.look {
            Look::Book => {
                let per_side = per_page / 2;
                for (side, (ax, ay, _, _)) in BOOK_PAGES.iter().enumerate() {
                    for (i, line) in shown.iter().skip(side * per_side).take(per_side).enumerate() {
                        font.draw(surf, line, x0 + ax, y0 + ay + i as i32 * fh, &self.data.pal);
                    }
                }
            }
            Look::Scroll => {
                let (ax, ay, _, _) = SCROLL_AREA;
                for (i, line) in shown.iter().enumerate() {
                    font.draw(surf, line, x0 + ax, y0 + ay + i as i32 * fh, &self.data.pal);
                }
            }
            _ => {
                let top = y0 + (f.height - shown.len() as i32 * fh) / 2;
                for (i, line) in shown.iter().enumerate() {
                    font.draw(surf, line, x0 + (f.width - font.width(line)) / 2, top + i as i32 * fh, &self.data.pal);
                }
            }
        }
    }
}
