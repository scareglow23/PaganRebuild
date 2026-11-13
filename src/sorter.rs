//! Paint-order sorting for the isometric view.
//!
//! Ported from ScummVM's Ultima 8 engine (world/item_sorter.cpp and
//! world/sort_item.cpp; (C) the ScummVM Team, GPL-3.0-or-later): every
//! item gets a list of the items that have to be painted before it, and
//! painting walks those dependencies depth first.

use crate::shape::ShapeInfo;

#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn intersects(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
    fn contains(&self, o: &Rect) -> bool {
        self.left <= o.left && o.right <= self.right && self.top <= o.top && o.bottom <= self.bottom
    }
}

/// The floating island Stratos stands on.
const STRATOS_ISLAND: u32 = 362;

/// For a figure and the island of Stratos: whether the figure is in front
/// of it (None: not that pair, or nothing special about them). In front:
/// feet below the island's floor, near its front corner.
fn in_front_of_island(figure: &SortItem, island: &SortItem) -> Option<bool> {
    if island.shape != STRATOS_ISLAND || !figure.standing || figure.z >= island.z {
        return None;
    }
    Some(figure.x + figure.y >= island.x + island.y - 192)
}

#[derive(Default)]
pub struct SortItem {
    pub shape: u32,
    pub frame: u32,
    pub flipped: bool,
    /// Index of the item in the caller's list.
    pub id: usize,
    /// Screen position of the shape's origin.
    pub sx: i32,
    pub sy: i32,
    pub trans: bool,
    /// Drawn faintly: the invisible Avatar.
    pub ghost: bool,

    sr: Rect,
    x: i32,
    x_left: i32,
    y: i32,
    y_far: i32,
    z: i32,
    z_top: i32,
    sx_left: i32,
    sx_right: i32,
    sx_top: i32,
    sy_top: i32,
    fbigsq: bool,
    flat: bool,
    occl: bool,
    solid: bool,
    draw: bool,
    roof: bool,
    anim: bool,
    land: bool,
    occluded: bool,
    /// A body lying on the ground, and a creature on its feet.
    lying: bool,
    standing: bool,
    /// A figure climbing onto a ledge.
    climbing: bool,
    depends: Vec<usize>,
}

/// For a figure climbing onto a ledge: is `t` part of the ledge (its edge,
/// roof or floor up at about the figure's head, not clearly in front of
/// it)? The climbing picture already reaches up there with head and arms
/// while the box is still below: the edge of a roof was painted over the
/// Avatar climbing onto it.
fn climbed_ledge(t: &SortItem, figure: &SortItem) -> bool {
    figure.climbing && !t.standing && t.z > figure.z && t.z <= figure.z_top + 16 && t.y_far < figure.y && t.x_left < figure.x
}

/// Two boxes clearly separated along one axis (`far`, `near`: their
/// edges on it, the same for something flat there): whether the first is
/// painted first. None if that does not decide it.
fn separated(a: (i32, i32), b: (i32, i32)) -> Option<bool> {
    let (a_far, a_near) = a;
    let (b_far, b_near) = b;
    if a_far == a_near && b_far == b_near {
        return (a_near / 32 != b_near / 32).then_some(a_near < b_near);
    }
    if a_near <= b_far {
        return Some(true);
    }
    if a_far >= b_near {
        return Some(false);
    }
    None
}

/// Along an axis where one of two boxes is flat and the other is not
/// (edges as in `separated`): by their cells, then by the middles of the
/// cells, then by the exact middles (a curtain hangs a few units in front
/// of the wall, a figure standing at the wall reaches behind it, but is
/// mostly in front). None if both are flat or neither, or it stays even.
fn half_flat_order(a: (i32, i32), b: (i32, i32)) -> Option<bool> {
    let (a_far, a_near) = a;
    let (b_far, b_near) = b;
    if (a_far == a_near) == (b_far == b_near) {
        return None;
    }
    if a_near / 32 <= b_far / 32 {
        return Some(true);
    }
    if a_far / 32 >= b_near / 32 {
        return Some(false);
    }
    let c1 = (a_far / 32 + a_near / 32) / 2;
    let c2 = (b_far / 32 + b_near / 32) / 2;
    if c1 != c2 {
        return Some(c1 < c2);
    }
    let (e1, e2) = (a_far + a_near, b_far + b_near);
    (e1 != e2).then_some(e1 < e2)
}

impl SortItem {
    fn overlap(&self, o: &SortItem) -> bool {
        if !self.sr.intersects(&o.sr) {
            return false;
        }
        // A body keeps the box of the creature on its feet, but its picture
        // reaches far beyond it (arms, legs): the pictures decide.
        if self.lying || o.lying {
            return true;
        }
        let top = (self.sx_top - o.sx, self.sy_top - o.sy);
        let bot = (self.sx - o.sx_top, self.sy - o.sy_top);
        let clear = self.sx_right <= o.sx_left
            || self.sx_left >= o.sx_right
            || top.0 + top.1 * 2 >= 0
            || -top.0 + top.1 * 2 >= 0
            || bot.0 - bot.1 * 2 >= 0
            || -bot.0 - bot.1 * 2 >= 0;
        !clear
    }

    fn occludes(&self, o: &SortItem) -> bool {
        if !self.sr.contains(&o.sr) {
            return false;
        }
        let top = (self.sx_top - o.sx_top, self.sy_top - o.sy_top);
        let bot = (self.sx - o.sx, self.sy - o.sy);
        self.sx_right >= o.sx_right
            && self.sx_left <= o.sx_left
            && top.0 + top.1 * 2 <= 0
            && -top.0 + top.1 * 2 <= 0
            && bot.0 - bot.1 * 2 <= 0
            && -bot.0 - bot.1 * 2 <= 0
    }

    /// True if `self` has to be painted before `o` (both overlap on screen).
    fn below(&self, o: &SortItem) -> bool {
        let (a, b) = (self, o);

        // The floating island of Stratos (air plane): its picture hangs
        // far below its box, and the rock where one stands before him lies
        // under its front edge. A figure there reaches with its head into
        // the box and was painted behind the hanging rocks, hidden. One
        // standing below its floor at its front is in front of it.
        if let Some(front) = in_front_of_island(a, b) {
            return !front;
        }
        if let Some(front) = in_front_of_island(b, a) {
            return front;
        }
        // a figure climbing onto a ledge is in front of it
        if climbed_ledge(b, a) {
            return false;
        }
        if climbed_ledge(a, b) {
            return true;
        }

        // clearly separated in z
        if a.flat && b.flat {
            if a.z != b.z {
                return a.z < b.z;
            }
        } else {
            if a.z_top <= b.z {
                return true;
            }
            if a.z >= b.z_top {
                return false;
            }
        }

        // clearly separated in y, in x
        let (ay, by) = ((a.y_far, a.y), (b.y_far, b.y));
        let (ax, bx) = ((a.x_left, a.x), (b.x_left, b.x));
        if let Some(r) = separated(ay, by).or_else(|| separated(ax, bx)) {
            return r;
        }

        // A body keeps the size of the creature on its feet, so whoever
        // walks over it stands inside its box: the body lies underneath.
        if a.lying && b.standing && b.z + 8 >= a.z {
            return true;
        }
        if a.standing && b.lying && a.z + 8 >= b.z {
            return false;
        }

        // z with a tolerance
        if a.z_top - 8 <= b.z && a.z < b.z_top - 8 {
            return true;
        }
        if a.z >= b.z_top - 8 && a.z_top - 8 > b.z {
            return false;
        }

        // one flat along an axis, the other not: by cells
        if let Some(r) = half_flat_order(ay, by).or_else(|| half_flat_order(ax, bx)) {
            return r;
        }

        if a.flat || b.flat {
            if a.z != b.z {
                return a.z < b.z;
            }
            if a.flat != b.flat {
                return a.flat;
            }
            if a.trans != b.trans {
                return !a.trans;
            }
            if a.anim != b.anim {
                return !a.anim;
            }
            if a.draw != b.draw {
                return a.draw;
            }
            if a.solid != b.solid {
                return a.solid;
            }
            if a.occl != b.occl {
                return a.occl;
            }
            if a.fbigsq != b.fbigsq {
                return a.fbigsq;
            }
        }

        if a.x == b.x && a.y == b.y && a.trans != b.trans {
            return !a.trans;
        }

        if a.land && b.land && a.roof != b.roof {
            return !a.roof;
        }
        // what stands under a roof (its foot lower) is below it: a tall
        // fire in a house reached into a beam of the roof, was painted
        // after it and through a cycle over the roof as well
        if a.roof != b.roof {
            let (roof, other) = if a.roof { (a, b) } else { (b, a) };
            if other.z < roof.z {
                return !a.roof;
            }
            return a.roof;
        }
        if a.z != b.z {
            return a.z < b.z;
        }

        // something flat along x or y: by the picture
        let flat_xy = |(far, near): (i32, i32)| far == near;
        if flat_xy(ax) || flat_xy(bx) || flat_xy(ay) || flat_xy(by) {
            if a.sx_left != b.sx_left {
                return a.sx_left > b.sx_left;
            }
            if a.sy != b.sy {
                return a.sy < b.sy;
            }
        }

        if a.x + a.y != b.x + b.y {
            return a.x + a.y < b.x + b.y;
        }
        if a.x_left + a.y_far != b.x_left + b.y_far {
            return a.x_left + a.y_far < b.x_left + b.y_far;
        }
        if a.y != b.y {
            return a.y < b.y;
        }
        if a.x != b.x {
            return a.x < b.x;
        }
        if a.shape != b.shape {
            return a.shape < b.shape;
        }
        a.frame < b.frame
    }
}

/// Size and origin of a shape frame, needed for the screen rectangle.
pub struct FrameDims {
    pub width: i32,
    pub height: i32,
    pub xoff: i32,
    pub yoff: i32,
}

/// Size of the cells of the screen grid that finds the items a new one
/// may overlap, in pixels.
const CELL: i32 = 64;
/// The grid reaches this far beyond the screen (items lie at most a few
/// hundred pixels outside it); farther out they share the edge cells.
const GRID_MARGIN: i32 = 1024;

pub struct Sorter {
    items: Vec<SortItem>,
    cam_sx: i32,
    cam_sy: i32,
    clip: Rect,
    /// For each cell of the screen grid, the items whose picture reaches into it.
    grid: Vec<Vec<usize>>,
    grid_w: i32,
    grid_h: i32,
    /// Per item: the last new item that looked at it (to take each once).
    seen: Vec<usize>,
    /// The figure climbing onto a ledge (index in the caller's list).
    pub climbing: Option<usize>,
}

impl Sorter {
    /// `cam_sx`, `cam_sy`: screen-space position of the surface's top left.
    pub fn new(cam_sx: i32, cam_sy: i32, width: i32, height: i32) -> Sorter {
        let grid_w = (width + 2 * GRID_MARGIN) / CELL + 1;
        let grid_h = (height + 2 * GRID_MARGIN) / CELL + 1;
        Sorter {
            items: Vec::new(),
            cam_sx,
            cam_sy,
            clip: Rect { left: 0, top: 0, right: width, bottom: height },
            grid: vec![Vec::new(); (grid_w * grid_h) as usize],
            grid_w,
            grid_h,
            seen: Vec::new(),
            climbing: None,
        }
    }

    /// The grid cells a screen rectangle covers: columns and rows.
    fn cells(&self, r: &Rect) -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
        let col = |x: i32| ((x + GRID_MARGIN).div_euclid(CELL)).clamp(0, self.grid_w - 1);
        let row = |y: i32| ((y + GRID_MARGIN).div_euclid(CELL)).clamp(0, self.grid_h - 1);
        (col(r.left)..=col(r.right - 1), row(r.top)..=row(r.bottom - 1))
    }

    /// Place in the list ScummVM's sorter keeps: by z, flat ones first among
    /// the same z, else in the order added.
    fn list_key(&self, i: usize) -> (i32, bool, usize) {
        let it = &self.items[i];
        (it.z, !it.flat, i)
    }

    pub fn add(&mut self, id: usize, pos: (i32, i32, i32), shape: u32, frame: u32, flipped: bool, invisible: bool,
               info: &ShapeInfo, dims: &FrameDims) {
        let (xd, yd, zd) = info.footpad(flipped);
        let (x, y, z) = pos;
        let (x_left, y_far, z_top) = (x - xd, y - yd, z + zd);
        let (csx, csy) = (self.cam_sx, self.cam_sy);

        let mut si = SortItem {
            shape,
            frame,
            flipped,
            id,
            x,
            y,
            z,
            x_left,
            y_far,
            z_top,
            sx_left: x_left / 4 - y / 4 - csx,
            sx_right: x / 4 - y_far / 4 - csx,
            sx_top: x_left / 4 - y_far / 4 - csx,
            sy_top: x_left / 8 + y_far / 8 - z_top - csy,
            sx: x / 4 - y / 4 - csx,
            sy: x / 8 + y / 8 - z - csy,
            fbigsq: xd == 128 && yd == 128,
            flat: zd == 0,
            draw: info.draw,
            solid: info.solid,
            occl: info.occl && !info.translucent && !invisible,
            ghost: invisible,
            roof: info.roof,
            anim: info.anim_type != 0,
            trans: info.translucent,
            land: info.land,
            lying: info.lies_in(frame),
            standing: info.creature && !info.lies_in(frame),
            climbing: self.climbing == Some(id),
            ..Default::default()
        };

        let left = if flipped { si.sx + dims.xoff - dims.width } else { si.sx - dims.xoff };
        let top = si.sy - dims.yoff;
        si.sr = Rect { left, top, right: left + dims.width, bottom: top + dims.height };
        if !self.clip.intersects(&si.sr) {
            return;
        }

        // Only items whose pictures can overlap matter: those found in the
        // grid cells the new one covers, taken in the order of the list
        // (as ScummVM's sorter walks its whole list), so the result is the same.
        let new_idx = self.items.len();
        let (cols, rows) = self.cells(&si.sr);
        let mut near: Vec<usize> = Vec::new();
        for r in rows.clone() {
            for c in cols.clone() {
                for &i in &self.grid[(r * self.grid_w + c) as usize] {
                    if self.seen[i] != new_idx + 1 {
                        self.seen[i] = new_idx + 1;
                        near.push(i);
                    }
                }
            }
        }
        near.sort_unstable_by_key(|&i| self.list_key(i));
        for &i2 in &near {
            let si2 = &mut self.items[i2];
            if si2.occluded {
                continue;
            }
            if si.overlap(si2) {
                if si.below(si2) {
                    if si2.occl && si2.occludes(&si) {
                        si.occluded = true;
                        break;
                    }
                    si2.depends.push(new_idx);
                } else if si.occl && si.occludes(si2) {
                    si2.occluded = true;
                } else {
                    si.depends.push(i2);
                }
            }
        }
        self.items.push(si);
        self.seen.push(0);
        for r in rows {
            for c in cols.clone() {
                self.grid[(r * self.grid_w + c) as usize].push(new_idx);
            }
        }
    }

    /// Whether `d` is flat ground at or below the foot of item `i`.
    fn floor_under(&self, d: usize, i: usize) -> bool {
        let (d, i) = (&self.items[d], &self.items[i]);
        d.flat && d.z <= i.z
    }

    /// A figure climbing onto a ledge is painted after the ledge. What
    /// the ledge covers and what stands in front of the figure (a wall
    /// beside him, under the eaves of the roof he climbs) then closed a
    /// circle: ledge, figure, wall, ledge - and the wall was painted over
    /// the roof for as long as he climbed. Such a thing stays under the
    /// ledge, and with it behind the figure.
    fn keep_under_climbed_ledge(&mut self) {
        let Some(f) = self.items.iter().position(|it| it.climbing && !it.occluded) else { return };
        let ledges: Vec<usize> = self.items[f].depends.iter().copied().filter(|&d| climbed_ledge(&self.items[d], &self.items[f])).collect();
        if ledges.is_empty() {
            return;
        }
        for x in 0..self.items.len() {
            if x != f && self.items[x].depends.contains(&f) && ledges.iter().any(|&l| self.items[l].depends.contains(&x)) {
                self.items[x].depends.retain(|&d| d != f);
            }
        }
    }

    /// Returns the visible items in the order they have to be painted.
    pub fn paint_order(mut self) -> Vec<SortItem> {
        self.keep_under_climbed_ledge();
        let n = self.items.len();
        let mut list: Vec<usize> = (0..n).collect();
        list.sort_unstable_by_key(|&i| self.list_key(i));

        // Dependencies are visited lowest first (ScummVM's sorter keeps the
        // lists sorted on insertion).
        let mut rank = vec![0usize; n];
        for (r, &i) in list.iter().enumerate() {
            rank[i] = r;
        }
        for it in &mut self.items {
            it.depends.sort_by_key(|&d| rank[d]);
        }

        const TODO: u8 = 0;
        const ACTIVE: u8 = 1;
        const DONE: u8 = 2;
        let mut state = vec![TODO; n];
        let mut order = Vec::with_capacity(n);
        let mut stack: Vec<(usize, usize, bool)> = Vec::new();

        for &start in &list {
            if state[start] != TODO || self.items[start].occluded {
                continue;
            }
            state[start] = ACTIVE;
            stack.push((start, 0, false));
            while let Some(top) = stack.last_mut() {
                let (i, k, cycle) = *top;
                if let Some(&d) = self.items[i].depends.get(k) {
                    top.1 += 1;
                    if state[d] == ACTIVE {
                        // cycle in the dependency graph: the remaining
                        // dependencies are left out, as in the original
                        top.2 = true;
                    } else if cycle && !self.floor_under(d, i) {
                        // but not the floor the item stands on: a figure
                        // walking into the box of the sparks at a pillar
                        // lost his head under the floor behind him
                    } else if state[d] == TODO && !self.items[d].occluded {
                        state[d] = ACTIVE;
                        stack.push((d, 0, false));
                    }
                } else {
                    state[i] = DONE;
                    order.push(i);
                    stack.pop();
                }
            }
        }

        let mut slots: Vec<Option<SortItem>> = self.items.into_iter().map(Some).collect();
        order.into_iter().filter_map(|i| slots[i].take()).collect()
    }
}
