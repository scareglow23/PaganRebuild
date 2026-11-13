//! The large map (option "map with guide", key O, or a click on the
//! minimap): the map the Avatar is on, seen from above like the minimap,
//! with a list of places beside it. A click on the map or on a place sets
//! where to go; the guide (guide.rs) then shows the way on the ground.
//! The game stands still while the map is open.

use crate::game::Game;
use crate::guide::{Leg, Status, Target};
use crate::map::Item;
use crate::minimap::shade;
use crate::places::{Place, MAP_NAMES, PEOPLE, PLACES};
use crate::view::Surface;

/// World units per map pixel across (as the minimap's ZOOM; 1 = the view).
const ZOOMS: [i32; 7] = [2, 4, 8, 16, 32, 64, 128];
const ZOOM_START: usize = 3;
/// Width of the list of places (at least; a third of a wide screen), and the margins.
const LIST_W: i32 = 150;
const MARGIN: i32 = 8;
/// How long the parts of the map may take to make, per picture.
const MAKE_BUDGET: std::time::Duration = std::time::Duration::from_millis(12);

// colours of the frame (the gold of the menu) and the marks
const GOLD: u32 = 0xE0A830;
const GOLD_DARK: u32 = 0x805010;
const RIM: u32 = 0x301800;
const UNKNOWN: u32 = 0x0C0804;
const ROW_LIT: u32 = 0x4A3010;
const WAY: u32 = 0xFFE060;
const TARGET: u32 = 0xF03020;
const PLACE: u32 = 0xE8E8D8;
const TELEPORTER: u32 = 0x60D0F0;

pub struct WorldMap {
    /// The world point in the middle of the map.
    pub centre: (i32, i32),
    pub zoom: usize,
    /// The first row of the list shown.
    pub scroll: usize,
    /// Where the left button went down, and the centre then (for moving the map).
    press: Option<((i32, i32), (i32, i32))>,
    moved: bool,
    pub pointer: Option<(i32, i32)>,
}

/// A row of the list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    /// A heading: here / elsewhere.
    Heading(bool),
    /// The heading of the people.
    People,
    Place(usize),
    /// A person (number in PEOPLE).
    Person(usize),
    /// Stop guiding.
    Clear,
}

/// Where the parts of the screen are: (x0, y0, x1, y1).
struct Layout {
    list: (i32, i32, i32, i32),
    list_w: i32,
    map: (i32, i32, i32, i32),
    row_h: i32,
}

fn layout(w: i32, h: i32, font_h: i32) -> Layout {
    let bottom = h - MARGIN - font_h - 4;
    let list_w = (w / 3).clamp(LIST_W, 260);
    Layout {
        list: (MARGIN, MARGIN, MARGIN + list_w, bottom),
        list_w,
        map: (MARGIN * 2 + list_w, MARGIN, w - MARGIN, bottom),
        row_h: font_h + 3,
    }
}

fn inside(r: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    x >= r.0 && y >= r.1 && x < r.2 && y < r.3
}

/// A frame in the gold of the menu around a box, the inside dark.
fn frame(surf: &mut Surface, r: (i32, i32, i32, i32), fill: Option<u32>) {
    for y in r.1 - 3..r.3 + 3 {
        for x in r.0 - 3..r.2 + 3 {
            let ring = (r.0 - 1 - x).max(x - r.2).max(r.1 - 1 - y).max(y - r.3);
            let c = match ring {
                2 => RIM,
                1 => GOLD,
                0 => GOLD_DARK,
                _ => match fill {
                    Some(c) if ring < 0 => c,
                    _ => continue,
                },
            };
            surf.put(x, y, c);
        }
    }
}

impl WorldMap {
    /// Map pixel (relative to the middle of the map) to world point, and
    /// back; turned like the view (`angle`), as the minimap.
    fn to_world(&self, du: i32, dv: i32, angle: f32) -> (i32, i32) {
        let z = ZOOMS[self.zoom];
        let (s, c) = angle.to_radians().sin_cos();
        let (a, b) = ((du * z) as f32, (dv * 2 * z) as f32);
        let (rx, ry) = ((a + b) / 2.0, (b - a) / 2.0);
        (self.centre.0 + (c * rx + s * ry).round() as i32, self.centre.1 + (-s * rx + c * ry).round() as i32)
    }

    fn to_map(&self, x: i32, y: i32, angle: f32) -> (i32, i32) {
        let z = ZOOMS[self.zoom] as f32;
        let (s, c) = angle.to_radians().sin_cos();
        let (dx, dy) = ((x - self.centre.0) as f32, (y - self.centre.1) as f32);
        let (rx, ry) = (c * dx - s * dy, s * dx + c * dy);
        (((rx - ry) / z).round() as i32, ((rx + ry) / (2.0 * z)).round() as i32)
    }
}

impl Game {
    fn map_angle(&self) -> f32 {
        if self.options.turn_view { self.turn.angle() } else { 0.0 }
    }

    pub fn open_worldmap(&mut self) {
        let av = self.avatar();
        self.close_reader();
        self.ui.worldmap = Some(WorldMap {
            centre: (av.x, av.y),
            zoom: ZOOM_START,
            scroll: 0,
            press: None,
            moved: false,
            pointer: None,
        });
    }

    pub fn close_worldmap(&mut self) {
        self.ui.worldmap = None;
    }

    /// The name of a place in the game's language (in the game's font).
    pub fn place_name(&self, p: &Place) -> String {
        let name = self.tr([p.names[0], p.names[1], p.names[2]]);
        crate::journal::dos(name)
    }

    /// The name of the map the Avatar is on (in the game's font), or its
    /// number if it has none.
    pub fn map_name(&self) -> String {
        let Some((_, names)) = MAP_NAMES.iter().find(|(m, _)| *m == self.cur_map) else {
            return self.tr([format!("Karte {}", self.cur_map), format!("Carte {}", self.cur_map), format!("Map {}", self.cur_map)]);
        };
        let name = self.tr([names[0], names[1], names[2]]);
        crate::journal::dos(name)
    }

    /// The rows of the list: stop guiding (if guiding), the places on this
    /// map, then those elsewhere.
    pub fn worldmap_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.guide.target.is_some() {
            rows.push(Row::Clear);
        }
        for here in [true, false] {
            let mut list: Vec<usize> = (0..PLACES.len()).filter(|&i| (PLACES[i].map == self.cur_map) == here).collect();
            if list.is_empty() {
                continue;
            }
            // (elsewhere: by map, the number shown in front)
            list.sort_by_key(|&i| (if here { 0 } else { PLACES[i].map }, self.place_name(&PLACES[i])));
            rows.push(Row::Heading(here));
            rows.extend(list.into_iter().map(Row::Place));
        }
        let people: Vec<usize> = (0..PEOPLE.len()).filter(|&k| self.person_at(PEOPLE[k].0).is_some()).collect();
        if !people.is_empty() {
            rows.push(Row::People);
            rows.extend(people.into_iter().map(Row::Person));
        }
        rows
    }

    fn row_text(&self, row: Row) -> String {
        match (row, self.language) {
            (Row::Heading(true), 'G') => "In dieser Gegend".into(),
            (Row::Heading(true), 'F') => "Dans cette r\u{82}gion".into(),
            (Row::Heading(true), _) => "In this area".into(),
            (Row::Heading(false), 'G') => "Anderswo".into(),
            (Row::Heading(false), 'F') => "Ailleurs".into(),
            (Row::Heading(false), _) => "Elsewhere".into(),
            (Row::Clear, 'G') => "Wegweiser ausschalten".into(),
            (Row::Clear, 'F') => "Arr\u{88}ter le guidage".into(),
            (Row::Clear, _) => "Stop guiding".into(),
            (Row::People, 'G') => "Personen".into(),
            (Row::People, 'F') => "Personnes".into(),
            (Row::People, _) => "People".into(),
            (Row::Place(i), _) if PLACES[i].map == self.cur_map => self.place_name(&PLACES[i]),
            (Row::Place(i), _) => format!("{}: {}", PLACES[i].map, self.place_name(&PLACES[i])),
            (Row::Person(k), _) => PEOPLE[k].1.into(),
        }
    }

    /// Goes to a place of the list (or stops guiding).
    pub fn choose_row(&mut self, row: Row) {
        match row {
            Row::Heading(_) | Row::People => {}
            Row::Clear => self.set_guide_target(None),
            Row::Person(k) => {
                let (id, name) = PEOPLE[k];
                let Some((map, x, y)) = self.person_at(id) else { return };
                self.set_guide_target(Some(Target { map, x, y, name: name.into(), person: id }));
                if map == self.cur_map {
                    if let Some(wm) = &mut self.ui.worldmap {
                        wm.centre = (x, y);
                    }
                }
                self.tick_guide();
            }
            Row::Place(i) => {
                let p = &PLACES[i];
                let name = self.place_name(p);
                self.set_guide_target(Some(Target { map: p.map, x: p.x, y: p.y, name, person: 0 }));
                if p.map == self.cur_map {
                    if let Some(wm) = &mut self.ui.worldmap {
                        wm.centre = (p.x, p.y);
                    }
                }
                self.tick_guide();
            }
        }
    }

    /// Goes to a point of the map (a click on it).
    pub fn choose_point(&mut self, x: i32, y: i32) {
        let name = self.tr(["Markierter Punkt", "Point choisi", "Point on the map"]);
        self.set_guide_target(Some(Target { map: self.cur_map, x, y, name: name.into(), person: 0 }));
        self.tick_guide();
    }

    /// The row of the list at a view position.
    fn row_at(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Row> {
        let wm = self.ui.worldmap.as_ref()?;
        let font_h = self.data.font.as_ref().map_or(8, |f| f.height);
        let l = layout(w, h, font_h);
        if !inside(l.list, x, y) {
            return None;
        }
        let k = ((y - l.list.1 - 2) / l.row_h).max(0) as usize + wm.scroll;
        self.worldmap_rows().get(k).copied()
    }

    /// Mouse on the map screen: `pressed` / `released` edges of the left
    /// button, `down` whether it is held, `wheel` the turn of the wheel.
    pub fn worldmap_mouse(&mut self, x: i32, y: i32, pressed: bool, down: bool, released: bool, wheel: i32, w: i32, h: i32) {
        let font_h = self.data.font.as_ref().map_or(8, |f| f.height);
        let l = layout(w, h, font_h);
        let angle = self.map_angle();
        let hover = self.row_at(x, y, w, h);
        let rows = self.worldmap_rows().len();
        let visible = ((l.list.3 - l.list.1 - 4) / l.row_h).max(1) as usize;
        let Some(wm) = self.ui.worldmap.as_mut() else { return };
        wm.pointer = Some((x, y));
        if wheel != 0 {
            if inside(l.list, x, y) {
                wm.scroll = (wm.scroll as i32 + wheel * 3).clamp(0, rows.saturating_sub(visible) as i32) as usize;
            } else if inside(l.map, x, y) {
                // zoom about the point under the pointer
                let (mcx, mcy) = ((l.map.0 + l.map.2) / 2, (l.map.1 + l.map.3) / 2);
                let before = wm.to_world(x - mcx, y - mcy, angle);
                wm.zoom = (wm.zoom as i32 + wheel).clamp(0, ZOOMS.len() as i32 - 1) as usize;
                let after = wm.to_world(x - mcx, y - mcy, angle);
                wm.centre = (wm.centre.0 + before.0 - after.0, wm.centre.1 + before.1 - after.1);
            }
        }
        if pressed {
            wm.press = Some(((x, y), wm.centre));
            wm.moved = false;
        }
        if down {
            if let Some(((px, py), centre)) = wm.press {
                if inside(l.map, px, py) && ((x - px).abs() > 3 || (y - py).abs() > 3 || wm.moved) {
                    // dragging moves the map
                    wm.moved = true;
                    wm.centre = centre;
                    let (ox, oy) = wm.to_world(0, 0, angle);
                    let (dx, dy) = wm.to_world(px - x, py - y, angle);
                    wm.centre = (centre.0 + dx - ox, centre.1 + dy - oy);
                }
            }
        }
        if released {
            let Some(((px, py), _)) = wm.press.take() else { return };
            if wm.moved {
                return;
            }
            if inside(l.map, px, py) {
                let (mcx, mcy) = ((l.map.0 + l.map.2) / 2, (l.map.1 + l.map.3) / 2);
                let (wx, wy) = wm.to_world(x - mcx, y - mcy, angle);
                self.choose_point(wx, wy);
            } else if let Some(row) = hover {
                self.choose_row(row);
            }
        }
    }

    /// Keys on the map screen: arrows move it, +/- zoom.
    pub fn worldmap_key(&mut self, dx: i32, dy: i32, zoom: i32) {
        let angle = self.map_angle();
        let Some(wm) = self.ui.worldmap.as_mut() else { return };
        wm.zoom = (wm.zoom as i32 + zoom).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        if dx != 0 || dy != 0 {
            let (ox, oy) = wm.to_world(0, 0, angle);
            let (wx, wy) = wm.to_world(dx * 40, dy * 40, angle);
            wm.centre = (wm.centre.0 + wx - ox, wm.centre.1 + wy - oy);
        }
    }

    pub fn draw_worldmap(&mut self, surf: &mut Surface) {
        if self.ui.worldmap.is_none() {
            return;
        }
        // the world dims behind the map
        surf.dim();
        let (w, h) = (surf.width, surf.height);
        // the smoothing (option "smooth graphics") would blur the map's
        // fine dots and the writing: all of it stays sharp
        if self.options.xbr() {
            self.view.sharp.clear();
            self.view.sharp.resize((w * h) as usize, true);
        }
        let font_h = self.data.font.as_ref().map_or(8, |f| f.height);
        let l = layout(w, h, font_h);
        if l.map.2 - l.map.0 < 40 || l.map.3 - l.map.1 < 40 {
            return;
        }
        let angle = self.map_angle();
        let av = self.avatar();
        let (mcx, mcy) = ((l.map.0 + l.map.2) / 2, (l.map.1 + l.map.3) / 2);
        let (hw, hh) = ((l.map.2 - l.map.0) / 2, (l.map.3 - l.map.1) / 2);
        let Some(wm) = self.ui.worldmap.as_ref() else { return };
        let corners = [wm.to_world(-hw, -hh, angle), wm.to_world(hw, -hh, angle), wm.to_world(-hw, hh, angle), wm.to_world(hw, hh, angle)];
        let lo = (corners.iter().map(|c| c.0).min().unwrap_or(0), corners.iter().map(|c| c.1).min().unwrap_or(0));
        let hi = (corners.iter().map(|c| c.0).max().unwrap_or(0), corners.iter().map(|c| c.1).max().unwrap_or(0));
        let centre = wm.centre;

        // the parts of the map in view, as many as time allows
        let Game { data: crate::gamedata::GameData { infos, shapes, pal, .. }, big_map, maps, cur_map, .. } = self;
        big_map.make_area(*cur_map, &maps[cur_map], infos, shapes, pal, lo, hi, centre, MAKE_BUDGET);
        let Some(wm) = self.ui.worldmap.as_ref() else { return };

        frame(surf, l.map, None);
        let step = ZOOMS[wm.zoom];
        for py in l.map.1..l.map.3 {
            for px in l.map.0..l.map.2 {
                let (wx, wy) = wm.to_world(px - mcx, py - mcy, angle);
                let c = match self.big_map.top_at(wx, wy) {
                    None => UNKNOWN,
                    Some((top, colour)) => {
                        // higher lighter, lower darker; in the shadow of a wall darker
                        let mut f = 1.0 + ((top - av.z) as f32 / 200.0).clamp(-0.45, 0.3);
                        let d = step.max(32);
                        if self.big_map.top_at(wx - d, wy - d).is_some_and(|(behind, _)| behind > top + 16) {
                            f *= 0.6;
                        }
                        shade(colour, f)
                    }
                };
                surf.pixels[(py * w + px) as usize] = c;
            }
        }

        self.draw_worldmap_marks(surf, &l, angle, av);
        // the name of the map, at the top of the map on a dark plate
        if let Some(font) = &self.data.font {
            let mut name = self.map_name();
            while font.width(&name) > l.map.2 - l.map.0 - 12 && name.pop().is_some() {}
            let tw = font.width(&name);
            let (x, y) = ((l.map.0 + l.map.2 - tw) / 2, l.map.1 + 4);
            frame(surf, (x - 4, y - 2, x + tw + 4, y + font_h + 1), Some(0x140C06));
            font.draw(surf, &name, x, y, &self.data.pal);
        }

        self.draw_worldmap_list(surf, &l, font_h);
    }

    /// The marks on the map: the places of this map, the way and the
    /// teleporter it leads to, the place to go to and the Avatar.
    fn draw_worldmap_marks(&self, surf: &mut Surface, l: &Layout, angle: f32, av: Item) {
        let Some(wm) = self.ui.worldmap.as_ref() else { return };
        let (mcx, mcy) = ((l.map.0 + l.map.2) / 2, (l.map.1 + l.map.3) / 2);
        let on_map = |(u, v): (i32, i32)| {
            let (x, y) = (mcx + u, mcy + v);
            (x > l.map.0 + 2 && y > l.map.1 + 2 && x < l.map.2 - 3 && y < l.map.3 - 3).then_some((x, y))
        };
        // the places of this map
        for p in PLACES.iter().filter(|p| p.map == self.cur_map) {
            if let Some((x, y)) = on_map(wm.to_map(p.x, p.y, angle)) {
                for (dx, dy) in [(0, -2), (-2, 0), (0, 0), (2, 0), (0, 2), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    surf.put(x + dx, y + dy, PLACE);
                }
            }
        }
        // the way, and the teleporter it leads to
        for &(x, y, _) in &self.guide.route {
            if let Some((sx, sy)) = on_map(wm.to_map(x - 32, y - 32, angle)) {
                surf.put(sx, sy, WAY);
                surf.put(sx + 1, sy, WAY);
            }
        }
        let teleporter = match self.guide.leg {
            Some(Leg::Teleporter(i)) => self.guide_link(i).and_then(|link| on_map(wm.to_map(link.x, link.y, angle))),
            _ => None,
        };
        if let Some((x, y)) = teleporter {
            for k in -4..=4 {
                for (dx, dy) in [(k, -4), (k, 4), (-4, k), (4, k)] {
                    surf.put(x + dx, y + dy, TELEPORTER);
                }
            }
        }
        // the place to go to: a red cross
        if let Some(t) = self.guide.target.as_ref().filter(|t| t.map == self.cur_map) {
            if let Some((x, y)) = on_map(wm.to_map(t.x, t.y, angle)) {
                for k in -4..=4 {
                    for (dx, dy) in [(k, k), (k, -k), (k + 1, k), (k + 1, -k)] {
                        surf.put(x + dx, y + dy, TARGET);
                    }
                }
            }
        }
        // the Avatar: a yellow dot with a dark rim
        if let Some((x, y)) = on_map(wm.to_map(av.x - 32, av.y - 32, angle)) {
            for dy in -3..=3i32 {
                for dx in -3..=3i32 {
                    if dx.abs() == 3 && dy.abs() == 3 {
                        continue;
                    }
                    let c = if dx.abs() == 3 || dy.abs() == 3 { 0x201000 } else { 0xFFE040 };
                    surf.put(x + dx, y + dy, c);
                }
            }
        }

    }

    /// The list of places and people beside the map, and under the map
    /// what is going on.
    fn draw_worldmap_list(&self, surf: &mut Surface, l: &Layout, font_h: i32) {
        let Some(wm) = self.ui.worldmap.as_ref() else { return };
        let (w, h) = (surf.width, surf.height);
        // the list of places
        frame(surf, l.list, Some(0x140C06));
        let rows = self.worldmap_rows();
        let pointer_row = wm.pointer.and_then(|(x, y)| self.row_at(x, y, w, h));
        let visible = ((l.list.3 - l.list.1 - 4) / l.row_h).max(1) as usize;
        let scroll = wm.scroll.min(rows.len().saturating_sub(visible));
        let target_place = self.guide.target.as_ref().and_then(|t| PLACES.iter().position(|p| p.map == t.map && p.x == t.x && p.y == t.y));
        if let Some(font) = &self.data.font {
            for (k, &row) in rows.iter().enumerate().skip(scroll).take(visible) {
                let y = l.list.1 + 2 + (k - scroll) as i32 * l.row_h;
                let lit = pointer_row == Some(row) && !matches!(row, Row::Heading(_) | Row::People);
                let chosen = matches!(row, Row::Place(i) if Some(i) == target_place)
                    || matches!(row, Row::Person(k) if self.guide.target.as_ref().is_some_and(|t| t.person == PEOPLE[k].0));
                if lit || chosen {
                    surf.fill_rect(l.list.0, y - 1, l.list.2, y + l.row_h - 1, if lit { ROW_LIT } else { 0x2A1A08 });
                }
                let text = self.row_text(row);
                let indent = if matches!(row, Row::Place(_) | Row::Person(_)) { 8 } else { 2 };
                let mut shown = text.clone();
                while font.width(&shown) > l.list_w - indent - 4 && shown.pop().is_some() {}
                font.draw(surf, &shown, l.list.0 + indent, y, &self.data.pal);
                if matches!(row, Row::Heading(_) | Row::People) {
                    // a line under the heading
                    for xx in l.list.0 + 2..l.list.2 - 2 {
                        surf.put(xx, y + font_h + 1, GOLD_DARK);
                    }
                }
            }
            // what is going on, under the map
            let dos = crate::journal::dos;
            let status: String = match (self.guide.status(), self.language) {
                (Status::Searching, 'G') => "Suche den Weg ...".into(),
                (Status::Searching, 'F') => "Recherche du chemin ...".into(),
                (Status::Searching, _) => "Searching for the way ...".into(),
                (Status::Near, 'G') => dos("Weg nur bis in die Nähe gefunden"),
                (Status::Near, 'F') => "Chemin seulement jusqu'aux environs".into(),
                (Status::Near, _) => "Way found only to near there".into(),
                (Status::NoWay, 'G') => "Kein Weg dorthin bekannt".into(),
                (Status::NoWay, 'F') => "Aucun chemin connu".into(),
                (Status::NoWay, _) => "No way there known".into(),
                (Status::Shown, 'G') => dos("Weg gefunden - Esc schließt die Karte"),
                (Status::Shown, 'F') => "Chemin trouv\u{82} - Echap ferme la carte".into(),
                (Status::Shown, _) => "Way found - Esc closes the map".into(),
                (Status::Idle, 'G') => dos("Klick auf Karte oder Ort wählt das Ziel, Ziehen verschiebt, Rad zoomt"),
                (Status::Idle, 'F') => "Clic sur la carte ou un lieu : destination".into(),
                (Status::Idle, _) => "Click the map or a place to go there; drag moves, wheel zooms".into(),
            };
            let mut shown = status;
            while font.width(&shown) > w - 2 * MARGIN && shown.pop().is_some() {}
            font.draw(surf, &shown, MARGIN, h - MARGIN - font_h, &self.data.pal);
        }
    }

    fn guide_link(&self, i: usize) -> Option<crate::guide::Link> {
        self.guide.link(i)
    }
}
