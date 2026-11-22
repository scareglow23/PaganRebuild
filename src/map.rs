//! Map data: FIXED.DAT (static scenery per map), NONFIXED.DAT (movable
//! objects), the NPC tables and GLOB.FLX (prefabs that "glob eggs" expand
//! into).

use crate::flex::{u16le, Flex};
use crate::hash::HashMap;
use crate::shape::{ShapeInfo, Shapes, FAMILY_CONTAINER, FAMILY_GLOBEGG};

pub const FLG_INVISIBLE: u16 = 0x0010;
pub const FLG_FLIPPED: u16 = 0x0020;

/// `Item::parent` of an item that is in the world.
pub const IN_WORLD: u16 = 0;
/// `Item::parent` of an item that was taken out of the world by a script.
pub const ETHEREAL: u16 = 0xFFFF;

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub shape: u32,
    pub frame: u32,
    pub flags: u16,
    pub quality: u16,
    /// NPC number (1 = the Avatar), 0 for ordinary items.
    pub npc: u16,
    pub mapnum: u16,
    /// Object id scripts use to refer to the item (NPCs: their number).
    pub id: u16,
    /// `IN_WORLD`, `ETHEREAL` or the id of the container holding the item.
    pub parent: u16,
    /// Destroyed; the slot is kept so that indices stay valid.
    pub gone: bool,
    /// In the active area around the view.
    pub fast: bool,
    /// Egg that has already triggered.
    pub hatched: bool,
    /// A dead body: it is drawn but does not block the way.
    pub dead: bool,
    /// Laid out by a glob egg of the map data (most floors and walls). The
    /// original lays these out only near the Avatar, marked "fast only";
    /// here they stay, but scripts see them so marked (`script_flags`).
    /// Not saved: found again from the map data on loading.
    #[serde(skip)]
    pub glob: bool,
}

impl Item {
    pub fn in_world(&self) -> bool {
        !self.gone && self.parent == IN_WORLD
    }

    /// The flags as the scripts see them: what a glob egg laid out is
    /// "fast only", as in the original. The eggs that lay out a piece of
    /// the map anew (the crack after the earthquake in the Upper
    /// Catacombs) clear everything not so marked first: the lava under the
    /// crack, from a glob, would go with it.
    pub fn script_flags(&self) -> u16 {
        if self.glob { self.flags | crate::eggs::FLG_FAST_ONLY } else { self.flags }
    }
    /// The Avatar or another NPC (they have the ids 1..=255).
    pub fn is_actor(&self) -> bool {
        self.id != 0 && self.id < 256
    }
}

/// Reads the 16 byte item records used by FIXED.DAT and NONFIXED.DAT.
///
/// The contents of a container follow it in the file, with their nesting
/// depth stored in the x field.  Returns the items and, for contained
/// items, the index of their container.
pub fn parse_items(d: &[u8], infos: &[ShapeInfo]) -> (Vec<Item>, Vec<Option<usize>>) {
    let mut out = Vec::with_capacity(d.len() / 16);
    let mut parents = Vec::with_capacity(d.len() / 16);
    let mut stack: Vec<usize> = Vec::new();
    for r in d.chunks_exact(16) {
        let it = Item {
            x: u16le(r, 0) as i32,
            y: u16le(r, 2) as i32,
            z: r[4] as i32,
            shape: u16le(r, 5) as u32,
            frame: r[7] as u32,
            flags: u16le(r, 8),
            quality: u16le(r, 10),
            npc: r[12] as u16,
            mapnum: r[13] as u16,
            ..Item::default()
        };
        // x is the nesting depth for contained items; anything else ends
        // the open containers
        while stack.len() as i32 != it.x && !stack.is_empty() {
            stack.pop();
        }
        parents.push(stack.last().copied());
        if infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_CONTAINER) {
            stack.push(out.len());
        }
        out.push(it);
    }
    (out, parents)
}

/// The items a glob egg stands for.
fn glob_items(egg: &Item, globs: &Flex) -> Vec<Item> {
    let Some(g) = globs.get(egg.quality as usize) else { return Vec::new() };
    if g.len() < 2 {
        return Vec::new();
    }
    let count = u16le(g, 0) as usize;
    g[2..]
        .chunks_exact(6)
        .take(count)
        .map(|r| Item {
            x: (egg.x & !0x1FF) + ((r[0] as i32) << 1) + 1,
            y: (egg.y & !0x1FF) + ((r[1] as i32) << 1) + 1,
            z: egg.z + r[2] as i32,
            shape: u16le(r, 3) as u32,
            frame: r[5] as u32,
            glob: true,
            ..Item::default()
        })
        .collect()
}

/// A permanent NPC as stored in the savegame.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Npc {
    pub map: usize,
    pub item: Item,
    pub dir: u8,
    pub stats: [u8; 4],
    /// Alignment byte: own side in the low, enemies in the high nibble.
    pub align: u8,
    /// Actor flags (invincible, dead, immortal ...).
    pub flags: u32,
    pub mana: i16,
}

/// The permanent NPCs (numbers 1..=255).
///
/// ITEMCACH.DAT stores the item fields as parallel arrays; NPCDATA.DAT has
/// a 0x31 byte record per NPC (strength, dexterity, intelligence, hit
/// points, direction ... and the high byte of the frame number).
pub fn load_npcs(itemcach: &Flex, npcdata: &Flex) -> Vec<Npc> {
    let (Some(ic), Some(nd)) = (itemcach.get(0), npcdata.get(0)) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 1..256usize {
        if 0x1D400 + i >= ic.len() || (i + 1) * 0x31 > nd.len() {
            break;
        }
        let shape = u16le(ic, 0x0B400 + i * 2) as u32;
        if shape == 0 {
            continue; // unused slot
        }
        let rec = &nd[i * 0x31..];
        let item = Item {
            x: u16le(ic, i * 2) as i32,
            y: u16le(ic, 0x04800 + i * 2) as i32,
            z: ic[0x09000 + i] as i32,
            shape,
            frame: ic[0x0FC00 + i] as u32 + ((rec[7] as u32) << 8),
            flags: u16le(ic, 0x12000 + i * 2),
            quality: u16le(ic, 0x16800 + i * 2),
            npc: i as u16,
            mapnum: ic[0x1D400 + i] as u16,
            id: i as u16,
            ..Item::default()
        };
        out.push(Npc { map: ic[0x1D400 + i] as usize, item, dir: rec[4] & 7, stats: [rec[0], rec[1], rec[2], rec[3]], align: rec[0x0B], flags: rec[0x1B] as u32 | ((rec[0x2F] as u32) << 8) | ((rec[0x30] as u32) << 16), mana: i16::from_le_bytes([rec[0x2D], rec[0x2E]]) });
    }
    out
}

/// Small deterministic random generator.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn bit(&mut self) -> bool {
        self.next() & 0x100 != 0
    }
}

/// Advances the frame of self-animating shapes (water, fire, torches ...)
/// by one game tick.  Returns true if any frame changed. Items whose
/// script animates them (type 5) and whose turn it is go into `scripted`.
pub fn animate(items: &mut [Item], infos: &[ShapeInfo], shapes: &mut Shapes, tick: u32, rng: &mut Rng, scripted: &mut Vec<u16>) -> bool {
    let mut changed = false;
    for it in items.iter_mut() {
        let Some(info) = infos.get(it.shape as usize) else { continue };
        if info.anim_type == 0 || it.is_actor() || !it.in_world() || !tick.is_multiple_of(info.anim_speed.max(1) as u32) {
            continue;
        }
        if info.anim_type == 5 {
            if it.fast {
                scripted.push(it.id);
            }
            continue;
        }
        let count = shapes.get(it.shape).map_or(0, |s| s.frames.len() as u32);
        if count == 0 {
            continue;
        }
        let data = info.anim_data as u32;
        let old = it.frame;
        let mut f = it.frame;
        match info.anim_type {
            // randomly change frame
            2 => {
                if rng.bit() {
                    f = rng.next() % count;
                }
            }
            // data 0: always advance, 1: 50% chance, 2+: loop in blocks of `data` frames
            1 | 3 => {
                if data == 0 || (data == 1 && rng.bit()) {
                    f += 1;
                    if f >= count {
                        f = 0;
                    }
                } else if data > 1 {
                    f += 1;
                    let num = (f - 1) / data;
                    if f == (num + 1) * data {
                        f = num * data;
                    }
                }
            }
            // randomly start, then run through all frames once
            4 => {
                if f != 0 || rng.next().is_multiple_of(data + 2) {
                    f += 1;
                    if f >= count {
                        f = 0;
                    }
                }
            }
            // like 1, but frame 0 (of each block) means "off"
            6 => {
                if data == 0 || (data == 1 && rng.bit()) {
                    if f != 0 {
                        f += 1;
                        if f >= count {
                            f = 1;
                        }
                    }
                } else if data > 1 && f % data != 0 {
                    f += 1;
                    let num = (f - 1) / data;
                    if f == (num + 1) * data {
                        f = num * data + 1;
                    }
                }
            }
            // 5: driven by usecode
            _ => {}
        }
        if f != old {
            it.frame = f;
            changed = true;
        }
    }
    changed
}

/// Axis-aligned box in world space. (x, y) is the near corner: the box
/// covers x-xd..x, y-yd..y and z..z+zd.
#[derive(Clone, Copy)]
pub struct WorldBox {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub xd: i32,
    pub yd: i32,
    pub zd: i32,
}

impl WorldBox {
    pub fn of(it: &Item, info: &ShapeInfo) -> WorldBox {
        let (xd, yd, zd) = info.footpad(it.flags & FLG_FLIPPED != 0);
        WorldBox { x: it.x, y: it.y, z: it.z, xd, yd, zd }
    }
    pub fn overlaps_xy(&self, o: &WorldBox) -> bool {
        self.x > o.x - o.xd && self.x - self.xd < o.x && self.y > o.y - o.yd && self.y - self.yd < o.y
    }
    pub fn overlaps(&self, o: &WorldBox) -> bool {
        self.overlaps_xy(o) && self.z + self.zd > o.z && self.z < o.z + o.zd
    }
}

/// Lava floors (the burning and the sinking kind, and their edges): in the
/// shape data they are ground one can stand on, their scripts burn.
const LAVA: [u32; 2] = [409, 479];

#[derive(Default)]
pub struct PositionInfo {
    /// Not inside anything solid.
    pub valid: bool,
    /// What is in the way, if it is not valid (one of them).
    pub blocker: Option<u16>,
    /// What it stands on (the highest thing below, if supported).
    pub support: Option<u16>,
    /// Standing on something.
    pub supported: bool,
    /// Bottom of the lowest roof above the position.
    pub roof_z: Option<i32>,
    /// Shape of the ground below the middle of the box.
    pub land_shape: Option<u32>,
    /// Top of the ground at the middle of the box, even if the box has
    /// sunk a little into it. Nothing should stand lower than this.
    pub floor_z: Option<i32>,
    /// What is directly below the middle of the box is water / something
    /// that burns, not ordinary ground.
    pub on_water: bool,
    pub on_fire: bool,
    /// Some part of the box (or close to it) stands on or in water. Its
    /// script drowns whoever touches it, even at the edge of the feet.
    pub touches_sea: bool,
}

/// What `MapData::position_info` gathers from the things around a box.
struct PositionScan<'a> {
    target: &'a WorldBox,
    start: &'a WorldBox,
    /// The box with a margin: water that close counts as touched.
    wide: WorldBox,
    /// The middle of the box.
    mid: (i32, i32),
    blocked: bool,
    blocker: Option<u16>,
    support: Option<u16>,
    support_z: i32,
    land_z: i32,
    land_shape: Option<u32>,
    floor_z: Option<i32>,
    /// Highest water and highest harmful thing below the middle.
    sea_z: i32,
    fire_z: i32,
    touches_sea: bool,
    roof_z: Option<i32>,
}

impl<'a> PositionScan<'a> {
    fn new(target: &'a WorldBox, start: &'a WorldBox) -> PositionScan<'a> {
        const SEA_MARGIN: i32 = 8;
        let wide = WorldBox { x: target.x + SEA_MARGIN, y: target.y + SEA_MARGIN, xd: target.xd + 2 * SEA_MARGIN, yd: target.yd + 2 * SEA_MARGIN, ..*target };
        PositionScan {
            target,
            start,
            wide,
            mid: (target.x - target.xd / 2, target.y - target.yd / 2),
            blocked: false,
            blocker: None,
            support: None,
            support_z: i32::MIN,
            land_z: i32::MIN,
            land_shape: None,
            floor_z: None,
            sea_z: i32::MIN,
            fire_z: i32::MIN,
            touches_sea: false,
            roof_z: None,
        }
    }

    /// Does the footprint `ib` lie under the middle of the box?
    fn under_middle(&self, ib: &WorldBox) -> bool {
        let (midx, midy) = self.mid;
        midx > ib.x - ib.xd && midx <= ib.x && midy > ib.y - ib.yd && midy <= ib.y
    }

    /// One thing that is solid, ground or a roof.
    fn add(&mut self, it: &Item, si: &ShapeInfo) {
        let target = self.target;
        let ib = WorldBox::of(it, si);
        if si.solid && target.overlaps(&ib) && !self.start.overlaps(&ib) {
            self.blocked = true;
            self.blocker = self.blocker.or(Some(it.id));
        }
        let top = ib.z + ib.zd;
        // lava is ground in the shape data (it burns by its
        // script); for finding ways it counts as water
        let sea = si.sea || LAVA.contains(&it.shape);
        // (with a margin: a step does not end exactly where planned)
        if sea && self.wide.overlaps_xy(&ib) && (top == target.z || self.wide.overlaps(&ib)) {
            self.touches_sea = true;
        }
        if target.overlaps_xy(&ib) {
            // only solid things carry: a roof that is not solid
            // (the see-through ceiling over a stairwell) does not
            if si.solid && top > self.support_z && top <= target.z {
                self.support_z = top;
                self.support = Some(it.id);
            }
            if si.roof && ib.z >= target.z + target.zd && self.roof_z.is_none_or(|r| ib.z < r) {
                self.roof_z = Some(ib.z);
            }
        }
        let under_middle = self.under_middle(&ib);
        // (a floating platform faded out keeps its "land" mark but
        // carries no one: a thing with height is a floor only if
        // it is solid; flat ground marks without it still are)
        if si.land && (si.solid || ib.zd < 16) && under_middle && top <= target.z + 16 && self.floor_z.is_none_or(|f| top > f) {
            self.floor_z = Some(top);
        }
        if (sea || si.damaging) && under_middle && top <= target.z {
            if sea {
                self.sea_z = self.sea_z.max(top);
            } else {
                self.fire_z = self.fire_z.max(top);
            }
        }
        // what is below the centre of the box?
        if (si.land || si.roof) && under_middle && top <= target.z && top > self.land_z {
            self.land_z = top;
            self.land_shape = Some(it.shape);
        }
    }

    fn finish(self) -> PositionInfo {
        let target = self.target;
        let supported = (self.support_z == target.z && self.land_z.saturating_add(8) >= target.z) || target.z <= 0;
        PositionInfo {
            valid: !self.blocked,
            blocker: self.blocker,
            support: if supported && self.support_z == target.z { self.support } else { None },
            // partial support only counts if there is ground close below
            supported,
            roof_z: self.roof_z,
            land_shape: self.land_shape,
            floor_z: self.floor_z,
            // deadly only if no solid ground lies on top of it
            on_water: self.sea_z > i32::MIN && self.sea_z >= self.land_z && self.sea_z >= target.z - 8,
            on_fire: self.fire_z > i32::MIN && self.fire_z >= self.land_z && self.fire_z >= target.z - 8,
            touches_sea: self.touches_sea,
        }
    }
}

/// A copy of the things of all maps, encoded exactly as the maps
/// themselves (`HashMap<usize, MapData>`, in the same order): the autosave
/// encodes it in the background while the game goes on.
pub struct MapsCopy(Vec<(usize, MapCopy)>);

/// What of a map goes into a savegame (the fields `MapData` saves, in
/// their order).
#[derive(serde::Serialize)]
struct MapCopy {
    items: Vec<Item>,
    next_id: u32,
}

impl MapsCopy {
    pub fn of(maps: &HashMap<usize, MapData>) -> MapsCopy {
        MapsCopy(maps.iter().map(|(&n, m)| (n, MapCopy { items: m.items.clone(), next_id: m.next_id })).collect())
    }
}

impl serde::Serialize for MapsCopy {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(n, m)| (n, m)))
    }
}

const CHUNK_SHIFT: i32 = 9;
const FIRST_ITEM_ID: u32 = 256;

/// The items of one map with a grid for spatial queries.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct MapData {
    pub items: Vec<Item>,
    #[serde(skip)]
    grid: HashMap<(i32, i32), Vec<u32>>,
    #[serde(skip)]
    by_id: HashMap<u16, u32>,
    next_id: u32,
}

impl MapData {
    /// Builds a map from the raw item lists of FIXED.DAT and NONFIXED.DAT
    /// and the NPCs standing on it.
    pub fn build(sources: &[&[u8]], npcs: &[Item], globs: &Flex, infos: &[ShapeInfo]) -> MapData {
        // ids below 256 are reserved for actors
        let mut map = MapData { items: Vec::new(), grid: HashMap::default(), by_id: HashMap::default(), next_id: FIRST_ITEM_ID };
        for d in sources {
            let (items, parents) = parse_items(d, infos);
            let mut ids = vec![0u16; items.len()];
            for (i, mut it) in items.into_iter().enumerate() {
                if let Some(p) = parents[i] {
                    it.parent = ids[p];
                    if it.parent == IN_WORLD {
                        continue; // container was not added
                    }
                } else if infos.get(it.shape as usize).is_some_and(|si| si.family == FAMILY_GLOBEGG) {
                    for g in glob_items(&it, globs) {
                        map.add(g);
                    }
                    continue;
                }
                ids[i] = map.add(it);
            }
        }
        for npc in npcs {
            map.add(*npc);
        }
        map
    }

    /// After loading a savegame, which does not keep `Item::glob`: marks
    /// what lies where a glob egg of the map data put it (`original`: the
    /// map as the data make it), and lays out again what of it a script
    /// destroyed, as the original does whenever the Avatar comes near.
    /// Only what cannot be carried off (floors, walls, lava), and only in
    /// the place it had. Returns how many things were laid out again.
    pub fn find_globs(&mut self, original: &MapData, infos: &[ShapeInfo]) -> usize {
        let spot = |it: &Item| (it.shape, it.x, it.y, it.z);
        let spots: HashMap<(u32, i32, i32, i32), Item> = original.items.iter().filter(|it| it.glob && it.in_world()).map(|it| (spot(it), *it)).collect();
        let mut here: crate::hash::HashSet<(u32, i32, i32, i32)> = crate::hash::HashSet::default();
        for it in self.items.iter_mut().filter(|it| it.in_world()) {
            let s = spot(it);
            it.glob = spots.contains_key(&s);
            here.insert(s);
        }
        let fixed = |it: &Item| infos.get(it.shape as usize).is_some_and(|si| si.fixed || si.raw[6] == 0 || si.roof || si.editor);
        let lost: Vec<Item> = self
            .items
            .iter()
            .filter(|it| it.gone && it.parent == IN_WORLD)
            .filter_map(|it| spots.get(&spot(it)).copied())
            .filter(|it| fixed(it))
            .collect();
        let mut laid = 0;
        for it in lost {
            if here.insert(spot(&it)) {
                self.add(Item { id: 0, fast: false, hatched: false, ..it });
                laid += 1;
            }
        }
        laid
    }

    /// Rebuilds the lookup tables after loading a savegame.
    /// Items kept (gone ones too), gone ones, and the next new id (tests).
    pub fn id_stats(&self) -> (usize, usize, u32) {
        (self.items.len(), self.items.iter().filter(|it| it.gone).count(), self.next_id)
    }

    pub fn reindex(&mut self) {
        self.grid.clear();
        self.by_id.clear();
        for (i, it) in self.items.iter().enumerate() {
            if it.gone {
                continue;
            }
            if it.id != 0 {
                self.by_id.insert(it.id, i as u32);
            }
            if it.in_world() {
                self.grid.entry(Self::cell(it)).or_default().push(i as u32);
            }
        }
    }

    fn cell(it: &Item) -> (i32, i32) {
        (it.x >> CHUNK_SHIFT, it.y >> CHUNK_SHIFT)
    }

    /// Adds an item and returns its object id (0 if no id is left).
    /// NPCs keep the id they come with.
    pub fn add(&mut self, mut it: Item) -> u16 {
        if it.id == 0 {
            while self.next_id < 0xFFF0 && self.by_id.contains_key(&(self.next_id as u16)) {
                self.next_id += 1;
            }
            if self.next_id < 0xFFF0 {
                it.id = self.next_id as u16;
                self.next_id += 1;
            }
        }
        let idx = self.items.len() as u32;
        if it.id != 0 {
            self.by_id.insert(it.id, idx);
        }
        if it.in_world() {
            self.grid.entry(Self::cell(&it)).or_default().push(idx);
        }
        self.items.push(it);
        it.id
    }

    pub fn index(&self, id: u16) -> Option<usize> {
        let i = *self.by_id.get(&id)? as usize;
        (!self.items[i].gone).then_some(i)
    }

    pub fn get(&self, id: u16) -> Option<&Item> {
        self.index(id).map(|i| &self.items[i])
    }

    fn unlink(&mut self, idx: usize) {
        let it = &self.items[idx];
        if it.in_world() {
            if let Some(cell) = self.grid.get_mut(&Self::cell(it)) {
                cell.retain(|&i| i as usize != idx);
            }
        }
    }

    /// Puts an item at a position in the world.
    pub fn place(&mut self, idx: usize, x: i32, y: i32, z: i32) {
        self.unlink(idx);
        let it = &mut self.items[idx];
        (it.x, it.y, it.z, it.parent) = (x, y, z, IN_WORLD);
        let cell = Self::cell(it);
        self.grid.entry(cell).or_default().push(idx as u32);
    }

    /// Takes an item out of the world: into a container or the ethereal void.
    pub fn take(&mut self, idx: usize, parent: u16) {
        self.unlink(idx);
        self.items[idx].parent = parent;
    }

    pub fn destroy(&mut self, idx: usize) {
        self.unlink(idx);
        let id = self.items[idx].id;
        self.items[idx].gone = true;
        self.by_id.remove(&id);
        // contents go with the container
        if id != 0 {
            let inside: Vec<usize> =
                self.items.iter().enumerate().filter(|(_, it)| !it.gone && it.parent == id).map(|(i, _)| i).collect();
            for i in inside {
                self.destroy(i);
            }
        }
    }

    /// Indices of the items in the world whose origin lies within `range`
    /// (in x and y) of a point.
    pub fn items_near(&self, x: i32, y: i32, range: i32) -> Vec<usize> {
        let mut out = Vec::new();
        for cx in ((x - range) >> CHUNK_SHIFT)..=((x + range) >> CHUNK_SHIFT) {
            for cy in ((y - range) >> CHUNK_SHIFT)..=((y + range) >> CHUNK_SHIFT) {
                let Some(cell) = self.grid.get(&(cx, cy)) else { continue };
                for &i in cell {
                    let it = &self.items[i as usize];
                    if (it.x - x).abs() <= range && (it.y - y).abs() <= range {
                        out.push(i as usize);
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Checks whether a box may be at `target`. Items already overlapping
    /// the `start` box do not block (so nothing gets stuck inside things),
    /// and neither does the item with id `skip` (the one being moved).
    pub fn position_info(&self, target: &WorldBox, start: &WorldBox, infos: &[ShapeInfo], skip: u16) -> PositionInfo {
        let mut scan = PositionScan::new(target, start);
        for cx in ((target.x - target.xd) >> CHUNK_SHIFT) - 1..=(target.x >> CHUNK_SHIFT) + 1 {
            for cy in ((target.y - target.yd) >> CHUNK_SHIFT) - 1..=(target.y >> CHUNK_SHIFT) + 1 {
                let Some(cell) = self.grid.get(&(cx, cy)) else { continue };
                for &i in cell {
                    let it = &self.items[i as usize];
                    if (skip != 0 && it.id == skip) || it.dead {
                        continue;
                    }
                    let Some(si) = infos.get(it.shape as usize) else { continue };
                    if si.solid || si.land || si.roof {
                        scan.add(it, si);
                    }
                }
            }
        }
        scan.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copy of the maps for the autosave is encoded byte for byte as
    /// the maps themselves.
    #[test]
    fn maps_copy_encodes_as_the_maps() {
        let mut maps: HashMap<usize, MapData> = HashMap::default();
        for n in [5, 0, 41, 12] {
            let mut m = MapData { items: Vec::new(), grid: HashMap::default(), by_id: HashMap::default(), next_id: FIRST_ITEM_ID };
            for i in 0..n {
                m.add(Item { x: i * 32, y: 7 - i, z: n, shape: 100 + i as u32, frame: 2, quality: 9, flags: 0x40, hatched: i % 2 == 0, ..Item::default() });
            }
            maps.insert(n as usize, m);
        }
        assert_eq!(bincode::serialize(&maps).unwrap(), bincode::serialize(&MapsCopy::of(&maps)).unwrap());
    }
}
