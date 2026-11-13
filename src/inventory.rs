//! The better inventory (optional): the Avatar's backpack opens as a
//! larger backpack with fixed slots, page by page, instead of the
//! original's small one with the things lying about in it. It is the
//! original picture, stretched: the opening grows, flap, straps and pocket
//! keep their look. Other containers and the paperdoll stay as they are.
//!
//! Every thing keeps its slot: it is kept in the item's x field, which the
//! original window does not use for things in containers (it holds their
//! position in its y field). Things without a slot (put there by a script,
//! or before the option was switched on) get the first free one.
//!
//! Things that are alike in every way (same kind, picture, quality and
//! flags) share one slot, which shows how many there are: a pile. They
//! stay things of their own (scripts and savegames see them as in the
//! original); only the window puts them together. Things counted by amount
//! (coins, reagents, potions) are one item with an amount instead.

use crate::game::Game;
use crate::shape::{Frame, TRANSPARENT};
use crate::view::Surface;

/// Obsidian coins, and the slot of the backpack window kept for them.
const COINS: u32 = 143;
const MONEY_SLOT: usize = 0;

/// Marks a slot number in an item's x field (items loaded from the game
/// files have their nesting depth there, a small number).
const SLOT_MARK: i32 = 0x10000;

const COLS: usize = 6;
const ROWS: usize = 4;
pub const PER_PAGE: usize = COLS * ROWS;

/// The bottom row of the first page is kept for the potions with quick
/// keys (yellow, red, orange, purple, black; see potions.rs), in the
/// order of their keys. The last slot of the row stays free.
const POTION_SLOTS: usize = PER_PAGE - COLS;
const POTION_COUNT: usize = crate::potions::QUICK_FRAMES.len();

/// Is `s` a slot kept for one kind of thing?
fn reserved(s: usize) -> bool {
    s == MONEY_SLOT || potion_slot(s)
}

fn potion_slot(s: usize) -> bool {
    (POTION_SLOTS..POTION_SLOTS + POTION_COUNT).contains(&s)
}

/// The slot kept for a thing of this kind (shape, frame), if there is one.
fn kept_slot(shape: u32, frame: u32) -> Option<usize> {
    if shape == COINS {
        return Some(MONEY_SLOT);
    }
    if shape == crate::potions::POTION {
        return crate::potions::QUICK_FRAMES.iter().position(|&f| f == frame).map(|i| POTION_SLOTS + i);
    }
    None
}

/// The kept slots with the shape and frame that shows what they are for.
fn kept_slots() -> impl Iterator<Item = (usize, u32, u32)> {
    let potions = crate::potions::QUICK_FRAMES.iter().enumerate().map(|(i, &f)| (POTION_SLOTS + i, crate::potions::POTION, f));
    std::iter::once((MONEY_SLOT, COINS, 5)).chain(potions)
}
/// Size of a slot.
const SLOT: i32 = 30;

/// The original backpack picture (gump 2) is 128 x 102. To make it larger,
/// two strips of columns beside the buckle and a strip of rows of the
/// opening are repeated; the rest (edges, buckle, straps, pocket) is kept.
const X_STRIPS: [(i32, i32); 2] = [(42, 58), (72, 88)];
// (rows 57..77 are all alike; row 56 still has the lower edge of the strap)
const Y_STRIP: (i32, i32) = (57, 77);
/// How much longer each column strip / the row strip gets.
const MORE_X: i32 = 52;
const MORE_Y: i32 = 95;
pub const WIDTH: i32 = 128 + 2 * MORE_X;
pub const HEIGHT: i32 = 102 + MORE_Y;
/// Top left of the slots, in the opening of the stretched backpack.
const GRID_X: i32 = 26;
const GRID_Y: i32 = 53;
/// The pocket below the opening: page arrows and page number.
const POCKET_Y: i32 = 79 + MORE_Y;
const ARROW_Y: i32 = POCKET_Y + 11;
const ARROW_DX: i32 = 46;

// arrows in the gold of the menu; the slots sunk into the dark opening
const GOLD: u32 = 0xE0A830;
const GOLD_DARK: u32 = 0x6A4818;
const SLOT_FACE: u32 = 0x1A120A;
const SLOT_SHADE: u32 = 0x070402;
const SLOT_LIGHT: u32 = 0x4A3622;

/// Which point of the original picture a point of the stretched one shows.
fn stretched(x: i32, y: i32) -> (i32, i32) {
    fn along(mut d: i32, parts: &[(i32, i32, i32)]) -> i32 {
        for &(a, b, more) in parts {
            if d < b - a + more {
                return a + d % (b - a);
            }
            d -= b - a + more;
        }
        parts.last().map_or(0, |p| p.1 - 1)
    }
    let [(a0, b0), (a1, b1)] = X_STRIPS;
    let xs = [(0, a0, 0), (a0, b0, MORE_X), (b0, a1, 0), (a1, b1, MORE_X), (b1, 128, 0)];
    let ys = [(0, Y_STRIP.0, 0), (Y_STRIP.0, Y_STRIP.1, MORE_Y), (Y_STRIP.1, 102, 0)];
    (along(x, &xs), along(y, &ys))
}

/// What is at a point of a slot window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GridHit {
    /// A slot of the page shown (its number counted over all pages).
    Slot(usize),
    /// The arrow to the page before / after.
    Back,
    Next,
    /// The window itself, beside the slots.
    Frame,
}

fn slot_of(x: i32) -> Option<usize> {
    (SLOT_MARK..SLOT_MARK + 0x10000).contains(&x).then(|| (x - SLOT_MARK) as usize)
}

impl Game {
    /// Is this container shown with slots? (Option "better inventory":
    /// only the Avatar's backpack.)
    pub fn grid_window(&self, owner: u16) -> bool {
        self.options.better_inventory && self.backpack() == Some(owner)
    }

    /// The colour index of the stretched backpack at a point of the window
    /// (TRANSPARENT beside it).
    fn backpack_pixel(&mut self, gshape: u32, x: i32, y: i32) -> u16 {
        let Some(shape) = self.data.gump_shapes.get(gshape) else { return TRANSPARENT };
        let Some(f) = shape.frames.first() else { return TRANSPARENT };
        let (sx, sy) = stretched(x, y);
        if sx < 0 || sy < 0 || sx >= f.width || sy >= f.height {
            return TRANSPARENT;
        }
        f.pixels[(sy * f.width + sx) as usize]
    }

    /// Can a thing lie on a pile with others like it? Not the things with
    /// an amount (they are one item), containers (they have contents) or
    /// creatures.
    fn pileable(&self, it: &crate::map::Item) -> bool {
        it.npc == 0 && !it.is_actor() && !self.stackable(it)
            && self.data.infos.get(it.shape as usize).is_some_and(|si| si.family != crate::shape::FAMILY_CONTAINER)
    }

    /// Are two things alike enough for one pile? The special weapons are
    /// kinds (shapes) of their own, so they never lie with the plain ones.
    pub fn alike(&self, a: &crate::map::Item, b: &crate::map::Item) -> bool {
        // Only flags that make a thing different count; the others are
        // left over from the world (owned, bouncing, hanging, in the fast
        // area, ...) and do not matter in the backpack.
        const KIND_FLAGS: u16 = 0x0002 | 0x0010 | 0x0040 | 0x0080; // disposable, invisible, in npc list, fast only
        a.id != b.id && a.shape == b.shape && self.kind_frame(a) == self.kind_frame(b) && a.quality == b.quality
            && a.flags & KIND_FLAGS == b.flags & KIND_FLAGS && self.pileable(a) && self.pileable(b)
    }

    /// The part of a thing's frame that says what it is. Things animated in
    /// the world (death disks, scrolls) stop on whatever picture they had
    /// when picked up; that picture does not make them different.
    fn kind_frame(&self, it: &crate::map::Item) -> u32 {
        let Some(si) = self.data.infos.get(it.shape as usize) else { return it.frame };
        let data = si.anim_data as u32;
        match si.anim_type {
            // run through blocks of `data` frames: the block is the kind
            1 | 3 | 6 if data > 1 => it.frame / data,
            // like 1, but frame 0 means "off"
            6 => (it.frame != 0) as u32,
            // all frames, or random ones: just a picture
            1..=4 => 0,
            // 0: still; 5: moved by its script, whose frame may mean something
            _ => it.frame,
        }
    }

    /// The things in a container with their slots (one per slot: the first
    /// of a pile). Things without one, or on a slot already taken, get the
    /// first free slot, which they then keep.
    pub fn grid_layout(&mut self, owner: u16) -> Vec<(usize, u16)> {
        self.grid_piles(owner).into_iter().map(|(s, pile)| (s, pile[0])).collect()
    }

    /// The pile `id` lies on in the slot window of its container (`id`
    /// first), or just `id`.
    pub fn pile_of(&mut self, id: u16) -> Vec<u16> {
        let Some(owner) = self.map().get(id).map(|it| it.parent) else { return vec![id] };
        if !self.grid_window(owner) {
            return vec![id];
        }
        match self.grid_piles(owner).into_iter().find(|(_, p)| p.contains(&id)) {
            Some((_, p)) => std::iter::once(id).chain(p.into_iter().filter(|&o| o != id)).collect(),
            None => vec![id],
        }
    }

    /// The things in a container by slot, alike things together on one.
    fn grid_piles(&mut self, owner: u16) -> Vec<(usize, Vec<u16>)> {
        let ids: Vec<usize> = self
            .map()
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| !it.gone && it.parent == owner && it.id != 0)
            .filter(|(_, it)| !self.data.infos.get(it.shape as usize).is_some_and(|si| si.editor))
            .map(|(i, _)| i)
            .collect();
        // the kept slots (money, quick potions) are for their kind only and
        // stay empty while there is none; one thing of each kind gets its
        // slot (the one that has it already; a second stack, not yet joined
        // to the first, lies on a free slot meanwhile)
        let mut taken: crate::hash::HashSet<usize> = kept_slots().map(|(s, ..)| s).collect();
        let mut out: Vec<(usize, Vec<u16>)> = Vec::new();
        let mut homeless = Vec::new();
        let mut kept: Vec<usize> = Vec::new();
        for on_it in [true, false] {
            for &i in &ids {
                let it = &self.map().items[i];
                let Some(s) = kept_slot(it.shape, it.frame) else { continue };
                if kept.contains(&i) || out.iter().any(|&(o, _)| o == s) || (on_it && slot_of(it.x) != Some(s)) {
                    continue;
                }
                let it = &mut self.map_mut().items[i];
                it.x = SLOT_MARK + s as i32;
                out.push((s, vec![it.id]));
                kept.push(i);
            }
        }
        // (piles: the slot of the first thing; the others lie on it, so
        // that moving the first one moves the whole pile)
        for &i in &ids {
            if kept.contains(&i) {
                continue;
            }
            if self.join_pile(&mut out, i) {
                continue;
            }
            let it = &self.map().items[i];
            match slot_of(it.x) {
                Some(s) if taken.insert(s) => out.push((s, vec![it.id])),
                _ => homeless.push(i),
            }
        }
        let mut free = 0;
        for i in homeless {
            if self.join_pile(&mut out, i) {
                continue;
            }
            while taken.contains(&free) {
                free += 1;
            }
            taken.insert(free);
            let it = &mut self.map_mut().items[i];
            it.x = SLOT_MARK + free as i32;
            out.push((free, vec![it.id]));
        }
        out.sort_unstable();
        out
    }

    /// Puts the thing at index `i` on a pile of things like it in `piles`
    /// (it takes that slot). False if there is none.
    fn join_pile(&mut self, piles: &mut [(usize, Vec<u16>)], i: usize) -> bool {
        let it = self.map().items[i];
        let Some(pile) = piles.iter_mut().find(|(_, p)| self.map().get(p[0]).is_some_and(|first| self.alike(first, &it))) else { return false };
        pile.1.push(it.id);
        let s = pile.0;
        self.map_mut().items[i].x = SLOT_MARK + s as i32;
        true
    }

    /// How many pages the window of a container has: up to the last thing,
    /// and one more if every slot so far is taken, or while something is
    /// being dragged (so that it can be put on a new page).
    fn grid_pages(&mut self, owner: u16) -> usize {
        let layout = self.grid_layout(owner);
        let last = layout.last().map_or(0, |&(s, _)| s / PER_PAGE + 1);
        let full = self.grid_free_from(owner, 0) >= last * PER_PAGE;
        (last + (full || self.ui.drag.is_some()) as usize).max(1)
    }

    /// The page arrow of a slot window at a view position: the window's
    /// container and the direction (-1 back, 1 on).
    pub fn grid_arrow_at(&mut self, x: i32, y: i32) -> Option<(u16, i32)> {
        let (gi, _) = self.pick_gump(x, y)?;
        let owner = self.ui.gumps[gi].owner;
        if !self.grid_window(owner) {
            return None;
        }
        match self.grid_hit(gi, x, y)? {
            GridHit::Back => Some((owner, -1)),
            GridHit::Next => Some((owner, 1)),
            _ => None,
        }
    }

    /// The page a container's window shows.
    pub fn grid_page(&mut self, owner: u16) -> usize {
        let pages = self.grid_pages(owner);
        let p = self.ui.gump_pages.get(&owner).copied().unwrap_or(0).min(pages - 1);
        self.ui.gump_pages.insert(owner, p);
        p
    }

    /// The page a window shows, as it was last drawn (not checked against
    /// the pages there are: dropping ends the drag, which may take away the
    /// empty page the thing is dropped on).
    pub fn shown_page(&self, owner: u16) -> usize {
        self.ui.gump_pages.get(&owner).copied().unwrap_or(0)
    }

    /// Turns the page of a container's window (`by` pages, not past the ends).
    pub fn grid_turn(&mut self, owner: u16, by: i32) -> bool {
        let pages = self.grid_pages(owner) as i32;
        let now = self.grid_page(owner) as i32;
        let to = (now + by).clamp(0, pages - 1);
        self.ui.gump_pages.insert(owner, to as usize);
        to != now
    }

    /// Top left corner of slot `s` (of the page shown) in the window.
    fn slot_corner(s: usize) -> (i32, i32) {
        let s = s % PER_PAGE;
        (GRID_X + (s % COLS) as i32 * SLOT, GRID_Y + (s / COLS) as i32 * SLOT)
    }

    /// What is at a view position in the slot window `gi`.
    pub fn grid_hit(&mut self, gi: usize, x: i32, y: i32) -> Option<GridHit> {
        let (owner, gshape, gx, gy) = { let g = &self.ui.gumps[gi]; (g.owner, g.shape, g.x, g.y) };
        let (lx, ly) = (x - gx, y - gy);
        if lx < 0 || ly < 0 || lx >= WIDTH || ly >= HEIGHT || self.backpack_pixel(gshape, lx, ly) == TRANSPARENT {
            return None;
        }
        let page = self.shown_page(owner);
        if ly >= POCKET_Y {
            let mid = WIDTH / 2;
            return Some(if lx < mid - 20 { GridHit::Back } else if lx > mid + 20 { GridHit::Next } else { GridHit::Frame });
        }
        let (cx, cy) = (lx - GRID_X, ly - GRID_Y);
        if cx < 0 || cy < 0 || cx >= COLS as i32 * SLOT || cy >= ROWS as i32 * SLOT {
            return Some(GridHit::Frame);
        }
        Some(GridHit::Slot(page * PER_PAGE + (cy / SLOT) as usize * COLS + (cx / SLOT) as usize))
    }

    /// The thing in a slot of a container, if any.
    pub fn grid_item(&mut self, owner: u16, slot: usize) -> Option<u16> {
        self.grid_layout(owner).into_iter().find(|&(s, _)| s == slot).map(|(_, id)| id)
    }

    /// A click on a page arrow of a slot window turns the page. True if
    /// the click was on one.
    pub fn grid_button(&mut self, x: i32, y: i32) -> bool {
        let Some((gi, _)) = self.pick_gump(x, y) else { return false };
        let owner = self.ui.gumps[gi].owner;
        if !self.grid_window(owner) {
            return false;
        }
        match self.grid_hit(gi, x, y) {
            Some(GridHit::Back) => self.grid_turn(owner, -1) || true,
            Some(GridHit::Next) => self.grid_turn(owner, 1) || true,
            _ => false,
        }
    }

    /// The mouse wheel over a slot window turns its pages.
    pub fn grid_wheel(&mut self, x: i32, y: i32, by: i32) -> bool {
        let Some((gi, _)) = self.pick_gump(x, y) else { return false };
        let owner = self.ui.gumps[gi].owner;
        self.grid_window(owner) && self.grid_turn(owner, by)
    }

    /// Puts `id` (already in container `owner`) on a slot: the one under the
    /// pointer if given (what is there takes the place `id` came from), else
    /// it keeps its slot, or gets the first free one.
    /// `came_from`: its slot before, if it was in this container already.
    pub fn grid_place(&mut self, id: u16, owner: u16, want: Option<usize>, came_from: Option<usize>) {
        // the money and the quick potions have their own slots, and nothing
        // else goes there
        if self.map().get(id).is_some_and(|it| kept_slot(it.shape, it.frame).is_some()) {
            self.grid_layout(owner);
            return;
        }
        let want = want.filter(|&s| !reserved(s));
        let came_from = came_from.filter(|&s| !reserved(s));
        // a thing on a pile moves with the whole pile (one just put in may
        // have joined a pile of its kind)
        let piles = self.grid_piles(owner);
        let Some((mine, members)) = piles.iter().find(|(_, p)| p.contains(&id)).cloned() else { return };
        let taken: crate::hash::HashSet<usize> = piles.iter().map(|&(s, _)| s).filter(|&s| s != mine).collect();
        let free = |s: &usize| !taken.contains(s);
        let Some(want) = want else {
            // beside the slots: moved within the window it keeps its slot
            let s = came_from.filter(free).unwrap_or(mine);
            for m in members {
                self.set_slot(m, s);
            }
            return;
        };
        if let Some((_, other)) = piles.iter().find(|&&(s, _)| s == want && s != mine).cloned() {
            // swap: the other pile goes where this one was (or to a free slot)
            let to = came_from.filter(free).filter(|&s| s != want).unwrap_or(mine);
            for o in other {
                self.set_slot(o, to);
            }
        }
        for m in members {
            self.set_slot(m, want);
        }
    }

    /// Puts a thing on slot `s`. Its place in the original window (the y
    /// field) follows the slot, so that with the option switched off the
    /// things do not lie in a heap.
    fn set_slot(&mut self, id: u16, s: usize) {
        let (col, row) = ((s % COLS) as i32, ((s / COLS) % ROWS) as i32);
        self.with_item_pub(id, |it| {
            it.x = SLOT_MARK + s as i32;
            it.y = (4 + col * 12) | (4 + row * 12) << 8;
        });
    }

    /// The first free slot of a container from page `page` on.
    pub fn grid_free_from(&mut self, owner: u16, page: usize) -> usize {
        let taken: crate::hash::HashSet<usize> = self.grid_layout(owner).into_iter().map(|(s, _)| s).collect();
        (page * PER_PAGE..).find(|s| !reserved(*s) && !taken.contains(s)).unwrap_or(1)
    }

    /// Slot of `id` if it lies in container `owner` (before it moves).
    pub fn grid_slot_of(&self, id: u16, owner: u16) -> Option<usize> {
        let it = self.map().get(id)?;
        if it.parent != owner {
            return None;
        }
        slot_of(it.x)
    }

    /// Where over a thing in a slot window its name is shown (above the
    /// middle of its slot), if it is on the page shown.
    pub fn grid_anchor(&mut self, gi: usize, id: u16) -> Option<(i32, i32)> {
        let (owner, gx, gy) = { let g = &self.ui.gumps[gi]; (g.owner, g.x, g.y) };
        let page = self.grid_page(owner);
        let (s, _) = self.grid_layout(owner).into_iter().find(|&(_, i)| i == id)?;
        if s / PER_PAGE != page {
            return None;
        }
        let (sx, sy) = Self::slot_corner(s);
        Some((gx + sx + SLOT / 2, gy + sy - 2))
    }

    /// Draws the slot window `gi`.
    pub fn draw_grid(&mut self, surf: &mut Surface, gi: usize) {
        let (owner, gshape, gx, gy) = { let g = &self.ui.gumps[gi]; (g.owner, g.shape, g.x, g.y) };
        // the original backpack, stretched
        if let Some(f) = self.data.gump_shapes.get(gshape).and_then(|s| s.frames.first().map(|f| (f.width, f.height, f.pixels.clone()))) {
            let (fw, fh, pixels) = f;
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let (sx, sy) = stretched(x, y);
                    if sx >= fw || sy >= fh {
                        continue;
                    }
                    let p = pixels[(sy * fw + sx) as usize];
                    if p != TRANSPARENT {
                        surf.put(gx + x, gy + y, self.data.pal.colors[p as usize]);
                    }
                }
            }
        }
        let page = self.grid_page(owner);
        let pages = self.grid_pages(owner);
        let piles = self.grid_piles(owner);
        let layout: Vec<(usize, u16)> = piles.iter().map(|(s, p)| (*s, p[0])).collect();
        // the slots, sunk in
        for s in 0..PER_PAGE {
            let (sx, sy) = Self::slot_corner(s);
            for y in 1..SLOT - 1 {
                for x in 1..SLOT - 1 {
                    let c = if x == 1 || y == 1 {
                        SLOT_SHADE
                    } else if x == SLOT - 2 || y == SLOT - 2 {
                        SLOT_LIGHT
                    } else {
                        SLOT_FACE
                    };
                    surf.put(gx + sx + x, gy + sy + y, c);
                }
            }
        }
        // the kept slots (money, quick potions) while they are empty: a faint
        // coin or bottle shows what they are for
        for (kept, shape, frame) in kept_slots() {
            if page != kept / PER_PAGE || layout.iter().any(|&(s, _)| s == kept) {
                continue;
            }
            let (sx, sy) = Self::slot_corner(kept);
            self.draw_kept_slot_hint(surf, (gx + sx, gy + sy), shape, frame);
        }
        // the things on this page
        for &(s, id) in &layout {
            if s / PER_PAGE != page {
                continue;
            }
            let Some(it) = self.map().get(id).copied() else { continue };
            let (sx, sy) = Self::slot_corner(s);
            let (cx, cy) = (gx + sx + SLOT / 2, gy + sy + SLOT / 2);
            if let Some(shape) = self.data.shapes.get(it.shape) {
                if let Some(f) = shape.frames.get(it.frame as usize) {
                    paint_fitted(surf, f, cx, cy, SLOT - 6, &self.data.pal);
                }
            }
            // (the quick potions' slots show their count below, 0 and 1 too)
            let pile = piles.iter().find(|(ps, _)| *ps == s).map_or(1, |(_, p)| p.len());
            let count = if self.stackable(&it) { it.quality as usize } else { pile };
            if count > 1 && !potion_slot(s) {
                if let Some(font) = &self.data.font {
                    let n = count.to_string();
                    font.draw(surf, &n, gx + sx + SLOT - 3 - font.width(&n), gy + sy + SLOT - 3 - font.height, &self.data.pal);
                }
            }
        }
        // the quick potions' slots: how many there are in the bottom right
        // corner (also 0, so that an empty slot shows it is empty), the key
        // in the top left one
        if page == POTION_SLOTS / PER_PAGE {
            let counts: Vec<u32> = (0..POTION_COUNT)
                .map(|n| {
                    let id = layout.iter().find(|&&(s, _)| s == POTION_SLOTS + n).map(|&(_, id)| id);
                    id.and_then(|id| self.map().get(id)).map_or(0, |it| (it.quality as u32).max(1))
                })
                .collect();
            if let Some(font) = &self.data.font {
                for (n, count) in counts.into_iter().enumerate() {
                    let (sx, sy) = Self::slot_corner(POTION_SLOTS + n);
                    let c = count.to_string();
                    font.draw(surf, &c, gx + sx + SLOT - 3 - font.width(&c), gy + sy + SLOT - 3 - font.height, &self.data.pal);
                }
                for n in 0..POTION_COUNT {
                    let Some(key) = self.bindings.potion_key(n) else { continue };
                    let name = crate::bindings::key_name(key);
                    if font.width(&name) > SLOT - 6 {
                        continue;
                    }
                    let (sx, sy) = Self::slot_corner(POTION_SLOTS + n);
                    font.draw(surf, &name, gx + sx + 3, gy + sy + 3, &self.data.pal);
                }
            }
        }
        if self.is_avatar_pack(owner) {
            self.draw_pack_weight(surf, gx + GRID_X + COLS as i32 * SLOT - 2, gy + GRID_Y - 3);
        }
        // the page arrows and the page number, on the pocket
        let mid = gy + ARROW_Y;
        let arrow = |surf: &mut Surface, x: i32, dir: i32, lit: bool| {
            let c = if lit { GOLD } else { GOLD_DARK };
            for i in 0..6 {
                for dy in -i..=i {
                    surf.put(x + dir * (6 - i), mid + dy, c);
                }
            }
        };
        arrow(surf, gx + WIDTH / 2 - ARROW_DX, -1, page > 0);
        arrow(surf, gx + WIDTH / 2 + ARROW_DX, 1, page + 1 < pages);
        if let Some(font) = &self.data.font {
            let text = format!("{} - {}", page + 1, pages); // (the game fonts have no "/")
            font.draw(surf, &text, gx + (WIDTH - font.width(&text)) / 2, mid - font.height / 2, &self.data.pal);
        }
    }
}

/// Paints a frame with its middle at (cx, cy), made smaller if it is
/// larger than `room` (every n-th pixel).
fn paint_fitted(surf: &mut Surface, f: &Frame, cx: i32, cy: i32, room: i32, pal: &crate::palette::Palette) {
    if f.width <= 0 || f.height <= 0 {
        return;
    }
    // only what is drawn counts: the bounds of the opaque pixels
    let (mut x0, mut y0, mut x1, mut y1) = (f.width, f.height, -1, -1);
    for y in 0..f.height {
        for x in 0..f.width {
            if f.pixels[(y * f.width + x) as usize] != TRANSPARENT {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if x1 < 0 {
        return;
    }
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    // everything at its own size, so that all things have the same pixel
    // size as the world (doubling only the small ones made them look like
    // another style); only things too large for the slot are made smaller
    let scale = (room as f32 / w.max(h) as f32).min(1.0);
    let (dw, dh) = (((w as f32 * scale).round() as i32).max(1), ((h as f32 * scale).round() as i32).max(1));
    let (left, top) = (cx - dw / 2, cy - dh / 2);
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = x0 + (dx as f32 / scale) as i32;
            let sy = y0 + (dy as f32 / scale) as i32;
            if sx > x1 || sy > y1 {
                continue;
            }
            let c = f.pixels[(sy * f.width + sx) as usize];
            if c == TRANSPARENT {
                continue;
            }
            let (px, py) = (left + dx, top + dy);
            if px >= 0 && py >= 0 && px < surf.width && py < surf.height {
                surf.pixels[(py * surf.width + px) as usize] = pal.colors[c as usize];
            }
        }
    }
}

impl Game {
    /// An empty kept slot (top left corner at `x`, `y`): a thin rim of
    /// dark gold and a faint picture of what it is for.
    fn draw_kept_slot_hint(&mut self, surf: &mut Surface, (x0, y0): (i32, i32), shape: u32, frame: u32) {
        const MARK: u32 = 0xFF00FF;
        let Some(thing) = self.data.shapes.get(shape) else { return };
        let Some(f) = thing.frames.get(frame as usize) else { return };
        let mut tmp = Surface::new(SLOT, SLOT);
        tmp.pixels.fill(MARK);
        paint_fitted(&mut tmp, f, SLOT / 2, SLOT / 2, SLOT - 6, &self.data.pal);
        let blend = |a: u32, b: u32| {
            let ch = |sh: u32| (((a >> sh) & 0xFF) * 6 + ((b >> sh) & 0xFF) * 4) / 10;
            ch(16) << 16 | ch(8) << 8 | ch(0)
        };
        for i in 2..SLOT - 2 {
            for (x, y) in [(i, 2), (i, SLOT - 3), (2, i), (SLOT - 3, i)] {
                surf.put(x0 + x, y0 + y, GOLD_DARK);
            }
        }
        for y in 3..SLOT - 3 {
            for x in 3..SLOT - 3 {
                let c = tmp.pixels[(y * SLOT + x) as usize];
                if c != MARK {
                    surf.put(x0 + x, y0 + y, blend(c, SLOT_FACE));
                }
            }
        }
    }

    /// Is this window's owner the backpack the Avatar wears?
    pub fn is_avatar_pack(&self, owner: u16) -> bool {
        self.map().get(owner).is_some_and(|it| {
            !it.gone && it.parent == crate::game::AVATAR && self.data.infos.get(it.shape as usize).is_some_and(|si| si.family == crate::shape::FAMILY_CONTAINER)
        })
    }

    /// The weight in the backpack (better inventory): what the Avatar carries and what
    /// his strength allows ("12 - 60", the game fonts have no "/"), right
    /// aligned at `right`, just above the slots ending at `bottom`. The
    /// numbers are those of the paperdoll.
    pub fn draw_pack_weight(&self, surf: &mut Surface, right: i32, bottom: i32) {
        let Some(font) = &self.data.font else { return };
        let strength = self.fights.states.get(&crate::game::AVATAR).map_or(15, |s| s.strength);
        let text = format!("{} - {}", self.carried_weight() / 10, 4 * strength);
        font.draw(surf, &text, right - font.width(&text), bottom - font.height, &self.data.pal);
    }
}
