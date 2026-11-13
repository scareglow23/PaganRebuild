//! Equipment of the Avatar (weapon, armour, backpack), the window that
//! shows it, and the loot of slain creatures.
//!
//! The weapon, armour and treasure tables in `data/` come from ScummVM.

use crate::combat::parse_ini;
use crate::drag::FLG_CONTAINED;
use crate::game::{Game, AVATAR};
use crate::hash::HashMap;
use crate::map::Item;
use crate::shape;
use crate::view::Surface;
use crate::vitals::{Attribute, MAX_ATTRIBUTE, PRACTICE};

pub const FLG_EQUIPPED: u16 = 0x0200;
pub const SLOT_HEAD: i32 = 3;
pub const SLOT_BODY: i32 = 4;
pub const SLOT_LEGS: i32 = 5;
pub const SLOT_WEAPON: i32 = 6;
pub const SLOT_BACKPACK: i32 = 7;
const BACKPACK_SHAPE: u32 = 529;

/// Where the worn items are drawn in the equipment window, by slot
/// (shield, arm, head, body, legs, weapon), relative to its item area.
const EQUIP_POS: [(i32, i32); 7] = [(0, 0), (24, 60), (36, 50), (40, 26), (40, 63), (40, 92), (16, 18)];
/// The backpack on the figure's shoulder.
const BACKPACK_RECT: (i32, i32, i32, i32) = (49, 25, 59, 50);

#[derive(Clone, Copy, Default)]
pub struct WeaponInfo {
    pub base_damage: i32,
    pub damage_mod: i32,
    pub damage_type: u16,
    pub attack_dex: i32,
    pub defend_dex: i32,
    pub armour: i32,
    /// Shape with the weapon as held in the hand, and which set of
    /// positions in WPNOVLAY.DAT it uses.
    pub overlay_shape: u32,
    pub overlay_type: usize,
    treasure_chance: i32,
}

#[derive(Clone, Copy, Default)]
pub struct ArmourInfo {
    pub armour: i32,
    pub defense_type: u16,
    pub kick_bonus: i32,
}

/// A kind of loot: which item, how likely, how many.
#[derive(Clone, Default)]
pub struct TreasureKind {
    shape: u32,
    frames: Vec<u32>,
    chance: f32,
    count: (i32, i32),
    special: String,
}

pub struct Tables {
    pub weapons: HashMap<u32, WeaponInfo>,
    pub armours: HashMap<(u32, u32), ArmourInfo>,
    treasures: HashMap<String, TreasureKind>,
    /// Loot specification per creature shape.
    monster_loot: HashMap<u32, String>,
}

fn num(s: Option<&String>) -> i32 {
    let Some(s) = s else { return 0 };
    match s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        Some(hex) => i32::from_str_radix(hex, 16).unwrap_or(0),
        None => s.parse().unwrap_or(0),
    }
}

/// "3" -> (3, 3), "1-8" -> (1, 8)
fn range(s: &str) -> (i32, i32) {
    match s.split_once('-') {
        Some((a, b)) => (a.parse().unwrap_or(1), b.parse().unwrap_or(1)),
        None => {
            let v = s.parse().unwrap_or(1);
            (v, v)
        }
    }
}

/// "key=value key=value" -> map
fn fields(s: &str) -> HashMap<&str, &str> {
    s.split_whitespace().filter_map(|kv| kv.split_once('=')).collect()
}

impl Tables {
    pub fn load() -> Tables {
        let weapons = parse_ini(include_str!("../data/u8weapons.ini"))
            .iter()
            .map(|e| {
                let n = |k: &str| num(e.get(k));
                (n("shape") as u32, WeaponInfo {
                    base_damage: n("base_damage"),
                    damage_mod: n("damage_mod"),
                    damage_type: n("damage_type") as u16,
                    attack_dex: n("attack_dex"),
                    defend_dex: n("defend_dex"),
                    armour: n("armour"),
                    overlay_shape: n("overlay_shape") as u32,
                    overlay_type: n("overlay") as usize,
                    treasure_chance: n("treasure_chance"),
                })
            })
            .collect();
        let armours = parse_ini(include_str!("../data/u8armour.ini"))
            .iter()
            .map(|e| {
                let n = |k: &str| num(e.get(k));
                ((n("shape") as u32, n("frame") as u32), ArmourInfo {
                    armour: n("armour"),
                    // (the table calls it "type": only the magic shield has one, fire)
                    defense_type: n("type") as u16,
                    kick_bonus: n("kick_bonus"),
                })
            })
            .collect();
        let monster_loot = parse_ini(include_str!("../data/u8monsters.ini"))
            .iter()
            .filter_map(|e| Some((num(e.get("shape")) as u32, e.get("treasure")?.clone())))
            .collect();

        // the [treasure] section of the game table: name=shape=.. frame=.. chance=.. count=..
        let mut treasures = HashMap::default();
        let mut in_section = false;
        for line in include_str!("../data/u8game.ini").lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == "[treasure]";
            } else if let (true, Some((name, spec))) = (in_section, line.split_once('=')) {
                let f = fields(spec);
                let frames = match f.get("frame") {
                    Some(fr) if fr.contains('-') => {
                        let (a, b) = range(fr);
                        (a.max(0) as u32..=b.max(0) as u32).collect()
                    }
                    Some(fr) => fr.split(',').filter_map(|x| x.parse().ok()).collect(),
                    None => vec![0],
                };
                treasures.insert(name.trim().to_string(), TreasureKind {
                    shape: f.get("shape").and_then(|s| s.parse().ok()).unwrap_or(0),
                    frames,
                    chance: f.get("chance").and_then(|s| s.parse().ok()).unwrap_or(1.0),
                    count: f.get("count").map_or((1, 1), |c| range(c)),
                    special: f.get("special").unwrap_or(&"").to_string(),
                });
            }
        }
        Tables { weapons, armours, treasures, monster_loot }
    }
}

impl Game {
    fn roll(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo { lo } else { lo + (self.rng.next() % (hi - lo + 1) as u32) as i32 }
    }

    // ---- what is worn ------------------------------------------------------

    /// The item an actor wears in a slot (1 shield, 2 arm, 3 head, 4 body,
    /// 5 legs, 6 weapon, 7 backpack).
    pub fn equipped(&self, actor: u16, slot: i32) -> Option<Item> {
        self.map().items.iter().find(|it| !it.gone && it.parent == actor && it.flags & FLG_EQUIPPED != 0 && it.z == slot).copied()
    }

    /// The slot an item belongs in, if it can be worn at all.
    fn slot_of(&self, it: &Item) -> Option<i32> {
        if it.shape == BACKPACK_SHAPE {
            return Some(SLOT_BACKPACK);
        }
        let t = self.data.infos.get(it.shape as usize)?.equip_type as i32;
        (1..=6).contains(&t).then_some(t)
    }

    /// Puts an item on. Fails if the slot is taken or the item cannot be worn.
    pub fn equip(&mut self, actor: u16, id: u16) -> bool {
        self.equip_weighed(actor, id, true)
    }

    /// `equip`; `weigh` false: also if it is too heavy (cheats).
    fn equip_weighed(&mut self, actor: u16, id: u16, weigh: bool) -> bool {
        let Some(it) = self.map().get(id).copied() else { return false };
        let Some(slot) = self.slot_of(&it) else { return false };
        if self.equipped(actor, slot).is_some() {
            return false;
        }
        if weigh && actor == AVATAR && self.can_add(AVATAR, id) == Err("heavy") {
            self.explain_refusal("heavy");
            return false;
        }
        let Some(idx) = self.map().index(id) else { return false };
        self.close_gump(id);
        let map = self.map_mut();
        map.take(idx, actor);
        let item = &mut map.items[idx];
        (item.x, item.y, item.z) = (0, 0, slot);
        item.flags |= FLG_EQUIPPED | FLG_CONTAINED;
        self.call_event(id, 10, &[]); // "equip" script
        true
    }

    /// The player lets an item go over the Avatar or his equipment
    /// window: wear it if possible, otherwise stow it in the backpack.
    pub fn drop_on_avatar(&mut self, id: u16) -> bool {
        if self.equip(AVATAR, id) {
            let volume = 1.0;
            self.audio.play_sfx(0x31, 0, false, volume);
            return true;
        }
        let Some(pack) = self.backpack() else { return false };
        if self.join_stack(id, pack) {
            return true;
        }
        let (gx, gy) = (8 + (self.rng.next() % 40) as i32, 8 + (self.rng.next() % 30) as i32);
        self.put_in_container(id, pack, gx, gy)
    }

    pub fn weapon_info(&self, actor: u16) -> Option<WeaponInfo> {
        let w = self.equipped(actor, SLOT_WEAPON)?;
        self.data.tables.weapons.get(&w.shape).copied()
    }

    /// Protection from everything the actor wears.
    pub fn armour_class(&self, actor: u16) -> i32 {
        self.map()
            .items
            .iter()
            .filter(|it| !it.gone && it.parent == actor && it.flags & FLG_EQUIPPED != 0)
            .map(|it| {
                self.data.tables.armours.get(&(it.shape, it.frame)).map_or(0, |a| a.armour) + self.data.tables.weapons.get(&it.shape).map_or(0, |w| w.armour)
            })
            .sum()
    }

    /// Kinds of damage the worn armour shrugs off.
    pub fn armour_defense(&self, actor: u16) -> u16 {
        self.map()
            .items
            .iter()
            .filter(|it| !it.gone && it.parent == actor && it.flags & FLG_EQUIPPED != 0)
            .filter_map(|it| self.data.tables.armours.get(&(it.shape, it.frame)))
            .fold(0, |acc, a| acc | a.defense_type)
    }

    pub fn kick_bonus(&self, actor: u16) -> i32 {
        self.equipped(actor, SLOT_LEGS).and_then(|l| self.data.tables.armours.get(&(l.shape, l.frame))).map_or(0, |a| a.kick_bonus)
    }

    /// The weapon as it shows in the Avatar's hand right now: (shape,
    /// frame, x and y offset from the Avatar's own position on screen).
    /// Only while the weapon is drawn, and only for animations that have
    /// overlay data.
    pub fn weapon_overlay(&mut self) -> Option<(u32, u32, i32, i32)> {
        self.weapon_overlay_for(None)
    }

    /// The weapon picture for the Avatar seen facing `dir` (None: the way
    /// he faces; another direction when the view is turned).
    pub fn weapon_overlay_for(&mut self, dir: Option<u8>) -> Option<(u32, u32, i32, i32)> {
        let s = self.fights.states.get(&AVATAR).copied()?;
        let mover = self.actors.get(&AVATAR)?;
        let (action, frame) = mover.shown();
        if s.dead || (!s.in_combat && action != crate::anim::UNREADY_WEAPON) {
            return None;
        }
        let dir = dir.unwrap_or(mover.dir) as usize & 7;
        let w = self.weapon_info(AVATAR)?;
        let length = self.data.anims.action(self.avatar().shape, action)?.size;
        // per action: for every overlay type, direction and animation
        // frame 4 bytes (x offset, y offset, frame of the overlay shape)
        let data = self.data.weapon_overlays.get(action as usize)?;
        let at = ((w.overlay_type * 8 + dir) * length + frame.min(length - 1)) * 4;
        let r = data.get(at..at + 4)?;
        let overlay_frame = u16::from_le_bytes([r[2], r[3]]) as u32;
        Some((w.overlay_shape, overlay_frame, r[0] as i8 as i32, r[1] as i8 as i32))
    }

    /// Cheat for testing: puts the strongest weapon and the best armour
    /// for every slot from the game's tables on the Avatar. Returns a
    /// description of what he got.
    pub fn cheat_equip(&mut self) -> Vec<String> {
        let mut wanted: Vec<(u32, u32, String)> = Vec::new();
        // the weapon with the highest possible damage
        if let Some((&shape, w)) = self.data.tables.weapons.iter().max_by_key(|(s, w)| (w.base_damage + w.damage_mod, **s)) {
            wanted.push((shape, 0, format!("weapon shape {shape} (damage {}-{})", w.base_damage, w.base_damage + w.damage_mod)));
        }
        // per slot the piece with the highest armour value
        for slot in 1..=5u8 {
            let best = self
                .data
                .tables
                .armours
                .iter()
                .filter(|((shape, _), _)| self.data.infos.get(*shape as usize).is_some_and(|si| si.equip_type == slot))
                .max_by_key(|((shape, frame), a)| (a.armour, *shape, *frame));
            if let Some((&(shape, frame), a)) = best {
                let place = ["", "shield", "arm", "head", "body", "legs"][slot as usize];
                wanted.push((shape, frame, format!("{place}: shape {shape} frame {frame} (armour {})", a.armour)));
            }
        }
        let cur = self.cur_map as u16;
        let mut got = Vec::new();
        for (shape, frame, text) in wanted {
            let id = self.map_mut().add(Item { shape, frame, parent: crate::map::ETHEREAL, mapnum: cur, ..Item::default() });
            if self.equip(AVATAR, id) {
                // F8 may swap it for another kind without filling the backpack
                self.cheat_worn.push(id);
                got.push(text);
            } else if let Some(idx) = self.map().index(id) {
                self.destroy_with_scripts(idx);
            }
        }
        got
    }

    /// Cheat (F8 with --cheat; with shift backwards): the Avatar's body
    /// armour changes to the next kind - none, leather, chain, plate, the
    /// two magic ones, then the 13 clothes - to see how each looks with the
    /// option "show armour".
    pub fn cheat_cycle_armour(&mut self, back: bool) {
        // (shape, frame, German name, English name)
        const ARMOUR: [(u32, u32, &str, &str); 6] = [
            (0, 0, "keine", "none"),
            (523, 4, "Leder", "leather"),
            (523, 2, "Kettenhemd", "chain mail"),
            (523, 0, "Plattenpanzer", "plate"),
            (64, 0, "magische Kette", "magic chain"),
            (841, 0, "magische Bronze", "magic bronze"),
        ];
        let mut kinds: Vec<(u32, u32, String, String)> = ARMOUR.iter().map(|k| (k.0, k.1, k.2.to_string(), k.3.to_string())).collect();
        for n in 0..13u32 {
            kinds.push((crate::outfit::CLOTHES_SHAPE, n * 2, format!("Kleidung {}", n + 1), format!("clothes {}", n + 1)));
        }
        self.cheat_cycle(SLOT_BODY, &kinds, back, ("R\u{81}stung", "Armour"));
    }

    /// Cheat (F9 with --cheat; with shift backwards): the next helmet.
    pub fn cheat_cycle_helm(&mut self, back: bool) {
        const HELMS: [(u32, u32, &str, &str); 7] = [
            (0, 0, "keiner", "none"),
            (531, 0, "Topfhelm", "great helm"),
            (531, 2, "Kettenhaube", "mail coif"),
            (531, 4, "Stahlkappe", "steel cap"),
            (531, 6, "dunkler Helm", "dark helm"),
            (531, 8, "Helm mit Kamm", "crested helm"),
            (843, 0, "magischer Helm", "magic helm"),
        ];
        let kinds: Vec<(u32, u32, String, String)> = HELMS.iter().map(|k| (k.0, k.1, k.2.to_string(), k.3.to_string())).collect();
        self.cheat_cycle(SLOT_HEAD, &kinds, back, ("Helm", "Helmet"));
    }

    /// Puts the next (or previous) of `kinds` (shape 0: nothing) on the
    /// Avatar in `slot`. What this cheat made vanishes again; what was
    /// found in the game goes to the backpack.
    fn cheat_cycle(&mut self, slot: i32, kinds: &[(u32, u32, String, String)], back: bool, what: (&str, &str)) {
        let worn = self.equipped(AVATAR, slot);
        let now = worn.and_then(|w| kinds.iter().position(|k| k.0 == w.shape && k.1 / 2 == w.frame / 2)).unwrap_or(0);
        if let Some(w) = worn {
            if self.cheat_worn.contains(&w.id) {
                self.take_off(w.id);
                if let Some(idx) = self.map().index(w.id) {
                    self.destroy_with_scripts(idx);
                }
            } else {
                let Some(pack) = self.backpack() else { return };
                if !self.put_in_container(w.id, pack, 10, 10) {
                    return; // no room: it stays on (the refusal was shown)
                }
            }
            self.cheat_worn.retain(|&id| id != w.id);
        }
        let next = if back { (now + kinds.len() - 1) % kinds.len() } else { (now + 1) % kinds.len() };
        let (shape, frame, de, en) = kinds[next].clone();
        if shape != 0 {
            let cur = self.cur_map as u16;
            let id = self.map_mut().add(Item { shape, frame, parent: crate::map::ETHEREAL, mapnum: cur, ..Item::default() });
            if self.equip_weighed(AVATAR, id, false) {
                self.cheat_worn.push(id);
            } else if let Some(idx) = self.map().index(id) {
                self.destroy_with_scripts(idx);
            }
        }
        let text = match self.language {
            'G' => format!("{}: {de}", what.0),
            _ => format!("{}: {en}", what.1),
        };
        self.notify(&text);
    }

    // ---- the equipment window -----------------------------------------------

    /// Draws the worn items and the character values into the Avatar's
    /// window whose top left corner is at (gx, gy).
    pub fn draw_paperdoll(&mut self, surf: &mut Surface, gx: i32, gy: i32) {
        // The window graphic has an opening at the bottom right where the
        // button for the small status display sits.
        const STAT_BUTTON_SHAPE: u32 = 38;
        const STAT_BUTTON_POS: (i32, i32) = (81, 84);
        if let Some(button) = self.data.gump_shapes.get(STAT_BUTTON_SHAPE) {
            if let Some(f) = button.frames.first() {
                surf.paint(f, gx + STAT_BUTTON_POS.0 + f.xoff, gy + STAT_BUTTON_POS.1 + f.yoff, false, false, &self.data.pal);
            }
        }
        // The colons of the window graphic do not line up with the text:
        // cover them with the stone just right of them.
        for y in gy + COLONS.1..gy + COLONS.3 {
            for x in gx + COLONS.0..gx + COLONS.2 {
                let from = x + COLONS.2 - COLONS.0 + 1;
                if x >= 0 && y >= 0 && from < surf.width && y < surf.height {
                    surf.pixels[(y * surf.width + x) as usize] = surf.pixels[(y * surf.width + from) as usize];
                }
            }
        }
        let area = self.data.gump_areas.first().copied().unwrap_or([0; 4]);
        for slot in (1..=6).rev() {
            let Some(it) = self.equipped(AVATAR, slot) else { continue };
            // the frame after an item's own shows it as worn
            if let Some(shape) = self.data.shapes.get(it.shape) {
                if let Some(f) = shape.frames.get(it.frame as usize + 1) {
                    let (x, y) = EQUIP_POS[slot as usize];
                    surf.paint(f, gx + area[0] + x, gy + area[1] + y, false, false, &self.data.pal);
                }
            }
        }
        let s = self.state(AVATAR);
        let labels: [&str; 7] = self.tr([
            ["ST\u{8E}", "INT", "GES", "R\u{9A}ST", "LEB", "MANA", "GEW"],
            ["FOR", "INT", "DEX", "ARMR", "VIE", "MANA", "PDS"],
            ["STR", "INT", "DEX", "ARMR", "HITS", "MANA", "WGHT"],
        ]);
        let values = [s.strength, s.intelligence, s.dex, self.armour_class(AVATAR), s.hp, self.mana, self.carried_weight() / 10];
        if let Some(font) = &self.data.font {
            for (i, (label, value)) in labels.iter().zip(values).enumerate() {
                // one row per colon of the window graphic (they stand 9 apart)
                let y = gy + STAT_TOP + STAT_ROW * i as i32;
                font.draw(surf, label, gx + 90, y, &self.data.pal);
                let v = value.to_string();
                font.draw(surf, &v, gx + 147 - font.width(&v), y, &self.data.pal);
            }
        }
        if self.options.practice {
            for (i, what) in PRACTISED.into_iter().enumerate() {
                let part = if values[i] >= MAX_ATTRIBUTE { PRACTICE } else { self.practice_of(what).clamp(0, PRACTICE) };
                draw_gauge(surf, gx + GAUGE_X, gy + STAT_TOP + STAT_ROW * i as i32 + 2, part);
            }
        }
    }

    /// Which of strength, intelligence, dexterity (0..3) the pointer is
    /// on in the Avatar's window, with the option for their progress on.
    pub fn practice_row_at(&self, pointer: Option<(i32, i32)>) -> Option<usize> {
        let (x, y) = pointer?;
        if !self.options.practice || self.grid_window(AVATAR) {
            return None;
        }
        let g = self.ui.gumps.iter().find(|g| g.owner == AVATAR)?;
        let row = (y - g.y - STAT_TOP).div_euclid(STAT_ROW);
        let inside = x >= g.x + 88 && x < g.x + 150 && (0..3).contains(&row);
        inside.then_some(row as usize)
    }

    /// Under the Avatar's window: how far the value under the pointer is
    /// from its next point and what makes it grow.
    pub fn draw_practice_hint(&mut self, surf: &mut Surface) {
        let Some(row) = self.practice_row_at(self.ui.pointer) else { return };
        let Some(g) = self.ui.gumps.iter().find(|g| g.owner == AVATAR) else { return };
        let (gx, gy, shape) = (g.x, g.y, g.shape);
        let height = self.data.gump_shapes.get(shape).and_then(|s| s.frames.first().map(|f| f.height)).unwrap_or(100);
        let s = self.state(AVATAR);
        let cur = [s.strength, s.intelligence, s.dex][row];
        let done = self.practice_of(PRACTISED[row]).clamp(0, PRACTICE);
        let german = self.language == 'G';
        let name = if german { ["St\u{84}rke", "Intelligenz", "Geschicklichkeit"][row] } else { ["Strength", "Intelligence", "Dexterity"][row] };
        let first = match (cur >= MAX_ATTRIBUTE, german) {
            (true, true) => format!("{name} {cur}: H\u{94}chstwert erreicht"),
            (true, false) => format!("{name} {cur}: highest value reached"),
            (false, true) => format!("{name} {cur}: {done} von {PRACTICE} Punkten bis {}, mit Gl\u{81}ck fr\u{81}her", cur + 1),
            (false, false) => format!("{name} {cur}: {done} of {PRACTICE} points to {}, sooner with luck", cur + 1),
        };
        let second = if german {
            ["W\u{84}chst durch Waffentreffer: je Treffer ein Viertel des Schadens", "W\u{84}chst durch Zaubern, Lesen und R\u{84}tsel", "W\u{84}chst durch Angriffe und Spr\u{81}nge auf Plattformen"][row]
        } else {
            ["Grows with weapon hits: a quarter of the damage per hit", "Grows with casting spells, reading and puzzles", "Grows with attacks and jumps on moving platforms"][row]
        };
        let Some(font) = &self.data.font else { return };
        let step = font.height_at(1.0) + 2;
        // below the window, or above it where there is no room below
        let top = if gy + height + 4 + 2 * step <= surf.height { gy + height + 4 } else { (gy - 4 - 2 * step).max(0) };
        let left_x = gx.clamp(0, (surf.width - font.width(&first).max(font.width(second))).max(0));
        font.draw(surf, &first, left_x, top, &self.data.pal);
        font.draw(surf, second, left_x, top + step, &self.data.pal);
    }

    /// The worn item (or the backpack) shown at a position of the
    /// Avatar's window, given the window's top left corner.
    pub fn pick_paperdoll(&mut self, x: i32, y: i32, gx: i32, gy: i32) -> Option<u16> {
        let area = self.data.gump_areas.first().copied().unwrap_or([0; 4]);
        let (ox, oy) = (gx + area[0], gy + area[1]);
        for slot in 1..=6 {
            let Some(it) = self.equipped(AVATAR, slot) else { continue };
            let Some(shape) = self.data.shapes.get(it.shape) else { continue };
            let Some(f) = shape.frames.get(it.frame as usize + 1) else { continue };
            let (px, py) = EQUIP_POS[slot as usize];
            let (col, row) = (x - (ox + px - f.xoff), y - (oy + py - f.yoff));
            if col >= 0 && row >= 0 && col < f.width && row < f.height && f.pixels[(row * f.width + col) as usize] != shape::TRANSPARENT {
                return Some(it.id);
            }
        }
        let (x1, y1, x2, y2) = BACKPACK_RECT;
        if x >= ox + x1 && x < ox + x2 + 8 && y >= oy + y1 && y < oy + y2 {
            return self.equipped(AVATAR, SLOT_BACKPACK).map(|b| b.id);
        }
        None
    }

    // ---- loot -----------------------------------------------------------------

    /// Fills the body of a slain creature with its treasure.
    pub fn give_treasure(&mut self, actor: u16) {
        let Some(body) = self.map().get(actor).copied() else { return };
        let Some(spec) = self.data.tables.monster_loot.get(&body.shape).cloned() else { return };
        for entry in spec.split(';') {
            let f = fields(entry);
            let Some(kind) = f.get("type").and_then(|t| self.data.tables.treasures.get(*t)).cloned() else { continue };
            // some loot only exists on one map
            if f.get("map").and_then(|m| m.parse::<usize>().ok()).is_some_and(|m| m != self.cur_map) {
                continue;
            }
            let (shape, frames) = if kind.special == "weapon" {
                // a random weapon, the common ones more often
                let mut choices: Vec<(u32, i32)> = self.data.tables.weapons.iter().map(|(s, w)| (*s, w.treasure_chance)).filter(|c| c.1 > 0).collect();
                choices.sort_unstable();
                let total: i32 = choices.iter().map(|c| c.1).sum();
                if total == 0 {
                    continue;
                }
                let mut pick = self.roll(0, total - 1);
                let shape = choices.iter().find(|c| {
                    pick -= c.1;
                    pick < 0
                }).map_or(choices[0].0, |c| c.0);
                (shape, vec![0])
            } else if !kind.special.is_empty() || kind.shape == 0 {
                continue;
            } else {
                (kind.shape, kind.frames.clone())
            };
            if (self.rng.next() % 1000) as f32 >= kind.chance * 1000.0 {
                continue;
            }
            let mult = f.get("mult").map_or((1, 1), |m| range(m));
            let count = self.roll(kind.count.0, kind.count.1) * self.roll(mult.0, mult.1);
            if count <= 0 {
                continue;
            }
            // things that stack (coins, reagents) are one item with an amount
            let stacks = self.data.infos.get(shape as usize).is_some_and(|si| si.family == 2 || si.family == 9);
            for _ in 0..if stacks { 1 } else { count.min(8) } {
                let frame = frames[(self.rng.next() as usize) % frames.len().max(1)];
                let y = (4 + (self.rng.next() % 50) as i32) | ((4 + (self.rng.next() % 30) as i32) << 8);
                let cur = self.cur_map as u16;
                self.map_mut().add(Item {
                    shape,
                    frame,
                    y,
                    quality: if stacks { count as u16 } else { 0 },
                    flags: FLG_CONTAINED,
                    parent: actor,
                    mapnum: cur,
                    ..Item::default()
                });
            }
        }
    }
}

/// The values in the Avatar's window that grow with practice, in the order shown.
const PRACTISED: [Attribute; 3] = [Attribute::Strength, Attribute::Intelligence, Attribute::Dexterity];
/// Where the progress gauges stand in the Avatar's window: between the
/// colons and the numbers.
const GAUGE_X: i32 = 125;

/// A small gauge, 10 x 5 pixels: dark rim, dark groove, filled with gold
/// as far as `part` of `PRACTICE` (bright gold when full).
fn draw_gauge(surf: &mut Surface, x0: i32, y0: i32, part: i32) {
    const W: i32 = 10;
    const H: i32 = 5;
    // any progress shows at least one stroke
    let filled = (part * (W - 2) + PRACTICE - 1) / PRACTICE;
    let full = part >= PRACTICE;
    for dy in 0..H {
        for dx in 0..W {
            let (x, y) = (x0 + dx, y0 + dy);
            if x < 0 || y < 0 || x >= surf.width || y >= surf.height {
                continue;
            }
            let rim = dy == 0 || dy == H - 1 || dx == 0 || dx == W - 1;
            let c = if rim {
                0x18_14_10
            } else if dx - 1 < filled {
                match (full, dy == 1) {
                    (true, true) => 0xFF_FF_D0,
                    (true, false) => 0xFF_E0_70,
                    (false, true) => 0xF0_C0_50,
                    (false, false) => 0xD0_90_20,
                }
            } else {
                0x40_38_30
            };
            surf.pixels[(y * surf.width + x) as usize] = c;
        }
    }
}
/// The character values in the Avatar's window: top of the first row and
/// the distance between rows, as the colons of the window graphic stand.
const STAT_TOP: i32 = 17;
const STAT_ROW: i32 = 9;
/// Where the colons are in the Avatar's window graphic: x0, y0, x1, y1 (exclusive).
const COLONS: (i32, i32, i32, i32) = (121, 17, 124, 79);
