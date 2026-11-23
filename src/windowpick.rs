//! Option "cycle through things" in open containers (modern controls, made
//! for the pad): while a container's window is open, two buttons go
//! through the things in the open windows one by one, the backpack's
//! pages turning along; the thing picked is framed and says its name.
//! "Use" uses it, and one button moves it between the backpack and the
//! other container open (out of the backpack, or into it). The Avatar's
//! own window with what he wears is left out: it shows no container.

use crate::game::{Game, AVATAR};
use crate::view::Surface;

/// The row of a slot of the backpack's slot window on its page.
fn grid_row(slot: usize) -> usize {
    (slot % crate::inventory::PER_PAGE) / crate::inventory::COLS
}

/// The column of a slot counted over all pages laid side by side.
fn grid_column(slot: usize) -> usize {
    (slot / crate::inventory::PER_PAGE) * crate::inventory::COLS + slot % crate::inventory::COLS
}

/// The frame around the thing picked: the gold of the menu's studs; red
/// while the game asks for a target (something used on something else).
const FRAME: u32 = 0xFFE070;
pub const TARGET_FRAME: u32 = 0xFF5030;

impl Game {
    /// Is a container's window open (not just the Avatar's own window)?
    pub fn item_windows_open(&self) -> bool {
        self.ui.gumps.iter().any(|g| g.owner != AVATAR)
    }

    /// The things in the open windows (a pile once), window by window.
    fn window_things(&mut self) -> Vec<u16> {
        let owners: Vec<u16> = self.pick_windows().into_iter().map(|w| w.1).collect();
        owners.into_iter().flat_map(|o| self.window_cells(o).into_iter().map(|c| c.2).collect::<Vec<_>>()).collect()
    }

    /// Picks the next thing in the open windows (`step` 1, to the right)
    /// or the one before (-1). It stays in its window (the window buttons
    /// go over to another). In the backpack's slots the pick stays in its
    /// row, the pages turning along; past the row's start it goes round to
    /// the row's end (the last page), past its end into the next row from
    /// its start (the first page). Elsewhere it goes through the things
    /// row by row, round at the end. The first press picks the first (or
    /// last) thing of the window on the left (right), in the backpack on
    /// the page shown. None if the windows are empty.
    pub fn pick_in_window(&mut self, step: i32) -> Option<u16> {
        if !self.options.pick_things {
            self.ui.window_picked = None;
            return None;
        }
        // the pick in the world gives way: the last one picked counts
        self.ui.picked = None;
        let id = match self.window_picked() {
            Some(id) => self.window_step(id, step),
            None => self.window_start(step),
        };
        let Some(id) = id else {
            self.ui.window_picked = None;
            return None;
        };
        self.set_window_pick(id);
        Some(id)
    }

    /// Over into the next window to the right (`step` 1) or left (-1), round
    /// at the end, with one press: onto the thing picked there last if it
    /// is still there, else the first one shown. Nothing picked yet: as
    /// the first press of `pick_in_window`. None if the windows are empty.
    pub fn pick_other_window(&mut self, step: i32) -> Option<u16> {
        if !self.options.pick_things {
            self.ui.window_picked = None;
            return None;
        }
        self.ui.picked = None;
        let Some(owner) = self.window_picked().and_then(|id| self.map().get(id)).map(|it| it.parent) else {
            return self.pick_in_window(step);
        };
        let windows: Vec<u16> = self.pick_windows().into_iter().map(|w| w.1).collect();
        let at = windows.iter().position(|&o| o == owner)? as i32;
        let n = windows.len() as i32;
        for k in 1..n {
            let other = windows[(at + k * step).rem_euclid(n) as usize];
            let things = self.window_cells(other);
            let mark = self.ui.window_marks.get(&other).copied().filter(|m| things.iter().any(|c| c.2 == *m));
            if let Some(id) = mark.or_else(|| self.cells_shown(other).first().map(|c| c.2)).or_else(|| things.first().map(|c| c.2)) {
                self.set_window_pick(id);
                return Some(id);
            }
        }
        self.ui.window_picked
    }

    /// Picks thing `id` in its window: its name, the backpack on its
    /// page, the window to the front (as when it is clicked), kept as the
    /// window's last pick.
    fn set_window_pick(&mut self, id: u16) {
        if self.ui.window_picked != Some(id) {
            self.ui.window_picked_name = self.look_name(id).or_else(|| self.unnamed_thing(id));
        }
        self.ui.window_picked = Some(id);
        self.show_page_of(id);
        if let Some(parent) = self.map().get(id).map(|it| it.parent) {
            self.ui.window_marks.insert(parent, id);
            if let Some(gi) = self.ui.gumps.iter().position(|g| g.owner == parent) {
                let g = self.ui.gumps.remove(gi);
                self.ui.gumps.push(g);
            }
        }
    }

    /// The first press: the first thing (`step` 1) of the leftmost window
    /// with something in it, or the last (-1) of the rightmost one; in the
    /// backpack's slots on the page shown.
    fn window_start(&mut self, step: i32) -> Option<u16> {
        let mut windows = self.pick_windows();
        if step < 0 {
            windows.reverse();
        }
        for (_, owner) in windows {
            let cells = self.cells_shown(owner);
            let first = if step < 0 { cells.last() } else { cells.first() };
            if let Some(&(_, _, id)) = first {
                return Some(id);
            }
        }
        None
    }

    /// One step from thing `id` (see `pick_in_window`).
    fn window_step(&mut self, id: u16, step: i32) -> Option<u16> {
        let owner = self.map().get(id)?.parent;
        let cells = self.window_cells(owner);
        let &(row, col, _) = cells.iter().find(|c| c.2 == id)?;
        let grid = self.grid_window(owner);
        // the backpack turned to another page since (its page buttons): on
        // in the same row there, else from the start of that page
        if grid && col as usize / crate::inventory::COLS != self.grid_page(owner) {
            let shown = self.cells_shown(owner);
            let in_row: Vec<u16> = shown.iter().filter(|c| c.0 == row).map(|c| c.2).collect();
            let all: Vec<u16> = shown.iter().map(|c| c.2).collect();
            let from = if in_row.is_empty() { all } else { in_row };
            if let Some(&next) = if step < 0 { from.last() } else { from.first() } {
                return Some(next);
            }
        }
        // in the backpack along the row, elsewhere through all the things
        let line: Vec<u16> = cells.iter().filter(|c| !grid || c.0 == row).map(|c| c.2).collect();
        let i = line.iter().position(|&t| t == id)? as i32 + step;
        if i >= 0 && i < line.len() as i32 {
            return Some(line[i as usize]);
        }
        // past the backpack's row end: the next row with something in it
        // (after the last the first), from its start
        if grid && step > 0 {
            let next = cells.iter().find(|c| c.0 > row).or(cells.first());
            return next.map(|c| c.2);
        }
        Some(line[if step < 0 { line.len() - 1 } else { 0 }])
    }

    /// The open windows that hold things (not the Avatar's own), with the
    /// middle of each across the screen, from the left.
    fn pick_windows(&self) -> Vec<(i32, u16)> {
        let mut windows: Vec<(i32, u16)> = self
            .ui
            .gumps
            .iter()
            .filter(|g| g.owner != AVATAR)
            .map(|g| {
                let area = self.data.gump_areas.get(g.shape.saturating_sub(1) as usize).copied().unwrap_or([0; 4]);
                (g.x + (area[0] + area[2]) / 2, g.owner)
            })
            .collect();
        windows.sort();
        windows
    }

    /// Every thing of window `owner` with its row and column, row by row
    /// from the top left: in the backpack's slots the pages side by side
    /// (a row runs on over all pages), elsewhere their place in the window,
    /// one a few pixels higher or lower than the one before it still in
    /// the same row.
    fn window_cells(&mut self, owner: u16) -> Vec<(i32, i32, u16)> {
        if self.grid_window(owner) {
            let mut cells: Vec<(i32, i32, u16)> = self.grid_layout(owner).into_iter().map(|(s, t)| (grid_row(s) as i32, grid_column(s) as i32, t)).collect();
            cells.sort();
            return cells;
        }
        const SAME_ROW: i32 = 2;
        let mut things: Vec<(i32, i32, u16)> = self
            .map()
            .items
            .iter()
            .filter(|it| !it.gone && it.parent == owner && it.id != 0)
            .filter(|it| !self.data.infos.get(it.shape as usize).is_some_and(|si| si.editor))
            // (inside a container the y field holds the place in the window)
            .map(|it| ((it.y >> 8) & 0xFF, it.y & 0xFF, it.id))
            .collect();
        things.sort();
        let (mut row, mut last) = (0, None);
        let mut cells: Vec<(i32, i32, u16)> = things
            .into_iter()
            .map(|(y, x, t)| {
                if last.is_some_and(|l| y - l > SAME_ROW) {
                    row += 1;
                }
                last = Some(y);
                (row, x, t)
            })
            .collect();
        cells.sort();
        cells
    }

    /// `window_cells`, in the backpack's slots only those on the page shown.
    fn cells_shown(&mut self, owner: u16) -> Vec<(i32, i32, u16)> {
        let cells = self.window_cells(owner);
        if !self.grid_window(owner) {
            return cells;
        }
        let page = self.grid_page(owner);
        cells.into_iter().filter(|c| c.1 as usize / crate::inventory::COLS == page).collect()
    }

    /// Moves the pick in its window a row up (`step` -1) or down (1): in the
    /// backpack's slots the next thing above or below in the same column
    /// (empty slots passed over, never into another column; alone in its
    /// column it stays). Elsewhere the nearest thing in the next row with
    /// something in it, as close to the same column as can be. At the top
    /// or bottom it goes round to the other end. False if nothing is
    /// picked in a window.
    pub fn pick_in_window_row(&mut self, step: i32) -> bool {
        let Some(id) = self.window_picked() else { return false };
        let Some(owner) = self.map().get(id).map(|it| it.parent) else { return false };
        // (in the backpack the pages side by side: up and down stay on the page)
        let mut cells = self.window_cells(owner);
        let Some(&(row, col, _)) = cells.iter().find(|c| c.2 == id) else { return false };
        if self.grid_window(owner) {
            cells.retain(|c| c.1 == col);
        }
        // the nearest row in that direction with something in it, else
        // round to the farthest one the other way
        let ahead = |r: i32| if step < 0 { r < row } else { r > row };
        let target_row = cells
            .iter()
            .map(|c| c.0)
            .filter(|&r| ahead(r))
            .min_by_key(|&r| (r - row).abs())
            .or_else(|| cells.iter().map(|c| c.0).filter(|&r| r != row).max_by_key(|&r| (r - row).abs()));
        let Some(target_row) = target_row else { return true };
        let Some(&(_, _, next)) = cells.iter().filter(|c| c.0 == target_row).min_by_key(|c| ((c.1 - col).abs(), c.1)) else { return false };
        self.set_window_pick(next);
        true
    }

    /// The thing picked in a window, while it is still in an open one.
    pub fn window_picked(&mut self) -> Option<u16> {
        let id = self.ui.window_picked?;
        if !self.options.pick_things || !self.window_things().contains(&id) {
            self.ui.window_picked = None;
        }
        self.ui.window_picked
    }

    /// The backpack's slot window turns to the page the thing is on.
    fn show_page_of(&mut self, id: u16) {
        let Some(owner) = self.map().get(id).map(|it| it.parent) else { return };
        if !self.grid_window(owner) {
            return;
        }
        if let Some((slot, _)) = self.grid_layout(owner).into_iter().find(|&(_, t)| t == id) {
            self.ui.gump_pages.insert(owner, slot / crate::inventory::PER_PAGE);
        }
    }

    /// Moves the thing picked between the backpack and the other container
    /// open: out of the backpack into the container opened last, from
    /// anywhere else into the backpack, with the checks of dragging it there
    /// (weight, room; taking what belongs to others counts as stealing).
    /// The pick stays on it. False if nothing moved (the player is told why
    /// where there is a reason).
    pub fn carry_picked(&mut self) -> bool {
        let Some(id) = self.window_picked() else { return false };
        let Some(pack) = self.backpack() else { return false };
        let parent = self.map().get(id).map_or(0, |it| it.parent);
        if parent != pack {
            return self.take_into_backpack(id) == Some(true);
        }
        // the container opened last that the Avatar does not carry himself
        let Some(target) = self.ui.gumps.iter().rev().map(|g| g.owner).find(|&o| o != AVATAR && o != pack && self.root_container(o) != AVATAR) else {
            return false;
        };
        let g = self.ui.gumps.iter().find(|g| g.owner == target).map(|g| g.shape).unwrap_or(0);
        let area = self.data.gump_areas.get(g.saturating_sub(1) as usize).copied().unwrap_or([0, 0, 64, 64]);
        let (w, h) = ((area[2] - area[0]).max(1), (area[3] - area[1]).max(1));
        // somewhere in its window, as when the original puts something in
        let (gx, gy) = (8.min(w / 2) + (self.rng.next() % (w - 8).max(1) as u32) as i32, 8.min(h / 2) + (self.rng.next() % (h - 8).max(1) as u32) as i32);
        self.put_in_container(id, target, gx, gy)
    }

    /// Where thing `id` is in window `gi` on the screen (left, top, right,
    /// bottom: the frame of the pick), if it is shown there.
    fn pick_rect(&mut self, gi: usize, id: u16) -> Option<(i32, i32, i32, i32)> {
        let it = self.map().get(id).copied()?;
        let owner = self.ui.gumps[gi].owner;
        if it.parent != owner {
            return None;
        }
        if self.grid_window(owner) {
            return self.grid_slot_rect(gi, id);
        }
        let (ox, oy) = self.gump_item_origin(gi)?;
        let f = self.data.shapes.get(it.shape).and_then(|s| s.frames.get(it.frame as usize).map(|f| (f.xoff, f.yoff, f.width, f.height)))?;
        let (x, y) = (ox + (it.y & 0xFF) - f.0, oy + ((it.y >> 8) & 0xFF) - f.1);
        Some((x - 2, y - 2, x + f.2 + 1, y + f.3 + 1))
    }

    /// A frame around the thing picked, if it is in window `gi` (on the
    /// page shown).
    pub fn draw_window_pick(&mut self, surf: &mut Surface, gi: usize) {
        let Some(id) = self.ui.window_picked else { return };
        let Some((x0, y0, x1, y1)) = self.pick_rect(gi, id) else { return };
        let c = if self.ui.targeting.is_some() { TARGET_FRAME } else { FRAME };
        for x in x0..=x1 {
            surf.put(x, y0, c);
            surf.put(x, y1, c);
        }
        for y in y0..=y1 {
            surf.put(x0, y, c);
            surf.put(x1, y, c);
        }
    }
}
