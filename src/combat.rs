//! Fighting: hit points, attacks and their effect, death, and the
//! behaviour of hostile creatures.
//!
//! Rules and numbers follow ScummVM's Ultima 8 engine (Actor::receiveHit,
//! calculateAttackDamage, CombatProcess; (C) the ScummVM Team,
//! GPL-3.0-or-later).  The creature and weapon tables in `data/` come
//! from the same project.

use crate::actor::Mover;
use crate::anim;
use crate::effects::PROC_WALK;
use crate::game::{Game, AVATAR};
use crate::hash::HashMap;
use crate::intrinsics::world_dir;
use crate::map::WorldBox;
use crate::view::Surface;

/// Pieces of music (numbers in MUSIC.FLX).
const MUSIC_COMBAT: i32 = 98;
const MUSIC_FANFARE: i32 = 109;
const MUSIC_DEATH: i32 = 44;

const DMG_BLADE: u16 = 0x0002;
pub const DMG_FIRE: u16 = 0x0008;
const DMG_UNDEAD: u16 = 0x0010;
pub const DMG_MAGIC: u16 = 0x0020;
const DMG_SLAYER: u16 = 0x0040;
pub const DMG_PIERCE: u16 = 0x0080;

/// Values of a kind of creature (data/u8monsters.ini).
#[derive(Clone, Copy, Default)]
pub struct MonsterInfo {
    hp: (i32, i32),
    dex: (i32, i32),
    damage: (i32, i32),
    armour: i32,
    /// Low nibble: own side, high nibble: the sides it attacks.
    alignment: u8,
    damage_type: u16,
    defense_type: u16,
    ranged: bool,
    /// Gets up again a while after being killed (skeletons), unless fire
    /// killed it.
    resurrection: bool,
}

/// Parses an ini table with one `[section]` per entry into key/value maps.
pub fn parse_ini(text: &str) -> Vec<HashMap<String, String>> {
    let mut out: Vec<HashMap<String, String>> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            out.push(HashMap::default());
        } else if let (Some((k, v)), Some(cur)) = (line.split_once('='), out.last_mut()) {
            cur.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn number(s: Option<&String>) -> i32 {
    let Some(s) = s else { return 0 };
    match s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        Some(hex) => i32::from_str_radix(hex, 16).unwrap_or(0),
        None => s.parse().unwrap_or(0),
    }
}

pub fn load_monsters() -> HashMap<u32, MonsterInfo> {
    parse_ini(include_str!("../data/u8monsters.ini"))
        .iter()
        .map(|e| {
            let n = |k: &str| number(e.get(k));
            (n("shape") as u32, MonsterInfo {
                hp: (n("hp_min"), n("hp_max")),
                dex: (n("dex_min"), n("dex_max")),
                damage: (n("damage_min"), n("damage_max")),
                armour: n("armour"),
                alignment: n("alignment") as u8,
                damage_type: n("damage_type") as u16,
                defense_type: n("defense_type") as u16,
                ranged: n("ranged") != 0,
                resurrection: n("resurrection") != 0,
            })
        })
        .collect()
}

/// The side of the ordinary people (alignment bit).
pub const TOWNSFOLK: u8 = 0x01;

/// Who fights and how the creatures stand to it: the tables of the
/// fights, kept by the number of the creature.
#[derive(Default)]
pub struct Fights {
    /// The fight states of the actors of the map the Avatar is on.
    pub(crate) states: HashMap<u16, ActorState>,
    /// The fight states of the creatures of the maps the Avatar is not on
    /// (their numbers from 256 on belong to one map: on another the same
    /// number is something else, a floor tile, say).
    pub(crate) map_states: HashMap<usize, HashMap<u16, ActorState>>,
    /// Creatures a script took out of a fight (Confusion Blast ...): they
    /// do not start another one on sight, only when attacked again.
    pub(crate) calmed: crate::hash::HashSet<u16>,
    /// Creatures a script made (createActor) for a scene: they do not start
    /// fights on sight, only when their script puts them in one (or they
    /// are attacked). The ghouls at Lothian's ceremony are such.
    pub(crate) script_made: crate::hash::HashSet<u16>,
    /// Changelings in the middle of taking the Avatar's shape:
    /// the step they are at and when it ends.
    pub(crate) changelings: HashMap<u16, Shift>,
    /// Killed creatures that get up again, and the tick when.
    pub(crate) resurrections: HashMap<u16, u32>,
    /// Creatures whose walks to their enemy brought them no closer: the
    /// gap after the last walk and how many walks in a row failed.
    pub(crate) approach_fails: HashMap<u16, (i32, u32)>,
    /// Creatures on their way to where they last saw the vanished Avatar.
    pub(crate) searching: crate::hash::HashSet<u16>,
    /// Health the creatures started with, for the bars above them.
    pub(crate) full_hp: HashMap<u16, i32>,
}

/// What changes about an actor in a fight.
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ActorState {
    pub hp: i32,
    pub strength: i32,
    pub dex: i32,
    pub intelligence: i32,
    pub alignment: u8,
    pub enemy_alignment: u8,
    pub dead: bool,
    pub immortal: bool,
    pub in_combat: bool,
    pub target: u16,
    /// Ticks until the creature decides what to do next.
    wait: i32,
}

/// How far (in world units, each way) a hostile creature notices the
/// Avatar: about what the original's small picture showed around him.
const NOTICE_REACH: i32 = 640;

/// The changeling, and the Avatar's shape it takes for a while.
pub const CHANGELING: u32 = 357;
const AVATAR_SHAPE: u32 = 1;
/// Its animations: turning into the Avatar's likeness and back.
const TO_AVATAR: u32 = 22;
/// One in this many fight decisions a changeling takes the Avatar's shape.
const SHIFT_CHANCE: u32 = 3;
/// The changelings at the pentagram of the end keep their shape (as in
/// the original and in ScummVM).
const ENDGAME_MAP: usize = 43;
/// ... and one in this many seconds on its way to him.
const WALK_SHIFT_CHANCE: u32 = 4;
const FROM_AVATAR: u32 = 23;

/// A changeling taking the Avatar's shape: instead of attacking, now and
/// then (see `SHIFT_CHANCE`) it turns into his likeness, fights so for 5
/// to 25 seconds and turns back. Saved behind the rest of the savegame.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Shift {
    /// Animation 22 plays; then it takes the Avatar's shape.
    Turning,
    /// In the Avatar's shape until this tick.
    Disguised(u32),
    /// Animation 23 plays; then it fights again.
    Returning,
}

/// Width of the health bars over the enemies, and their red from top to
/// bottom (that of the Avatar's own health bar).
const HEALTH_BAR_WIDTH: i32 = 24;
const HEALTH_BAR_RED: [u32; 3] = [0xD4_30_30, 0xBC_0C_0C, 0x98_04_04];

/// A pixel of a health bar, `dx`/`dy` from its top left inside corner:
/// dark outline, golden rim (lit from the top left), red up to `filled`,
/// then a dark groove.
fn health_bar_colour(dx: i32, dy: i32, filled: i32) -> u32 {
    let rim = dy == -1 || dy == 3 || dx == -1 || dx == HEALTH_BAR_WIDTH;
    let outline = dy == -2 || dy == 4 || dx == -2 || dx == HEALTH_BAR_WIDTH + 1;
    if outline {
        0x1C1008
    } else if rim {
        if dy == -1 || dx == -1 { 0xF0C050 } else { 0x906018 }
    } else if dx < filled {
        HEALTH_BAR_RED[dy as usize]
    } else {
        0x301800
    }
}

impl Game {
    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo { lo } else { lo + (self.rng.next() % (hi - lo + 1) as u32) as i32 }
    }

    /// The fight-related state of an actor, set up on first use: from the
    /// savegame for the permanent characters, from the creature table for
    /// hatched monsters.
    pub fn state(&mut self, id: u16) -> ActorState {
        if let Some(s) = self.fights.states.get(&id) {
            return *s;
        }
        let shape = self.map().get(id).map_or(0, |it| it.shape);
        let s = if let Some(stats) = self.people.npc_stats.get(&id).copied() {
            let (align, flags) = self.people.npc_combat.get(&id).copied().unwrap_or((0, 0));
            ActorState {
                strength: stats[0] as i32,
                dex: stats[1] as i32,
                intelligence: stats[2] as i32,
                hp: stats[3] as i32,
                alignment: align & 0x0F,
                enemy_alignment: align >> 4,
                dead: flags & 0x1000 != 0,
                immortal: flags & 0x04_0000 != 0,
                ..ActorState::default()
            }
        } else if let Some(mi) = self.data.monsters.get(&shape).copied() {
            ActorState {
                hp: self.random(mi.hp.0, mi.hp.1),
                dex: self.random(mi.dex.0, mi.dex.1),
                strength: 10,
                alignment: mi.alignment & 0x0F,
                enemy_alignment: mi.alignment >> 4,
                ..ActorState::default()
            }
        } else {
            ActorState { hp: 10, strength: 10, dex: 10, ..ActorState::default() }
        };
        self.fights.states.insert(id, s);
        self.fights.full_hp.insert(id, s.hp);
        s
    }

    pub fn state_mut(&mut self, id: u16) -> &mut ActorState {
        self.state(id);
        self.fights.states.get_mut(&id).unwrap()
    }

    pub fn max_hp(&mut self, id: u16) -> i32 {
        if id == AVATAR { 2 * self.state(id).strength } else { self.state(id).hp }
    }

    pub fn mover(&mut self, id: u16) -> &mut Mover {
        if !self.actors.contains_key(&id) {
            let dir = self.people.npc_dirs.get(&id).copied().unwrap_or(4);
            self.actors.insert(id, Mover::new(dir));
        }
        self.actors.get_mut(&id).unwrap()
    }

    /// Plays an animation for an actor; false if its shape has none.
    pub fn animate(&mut self, id: u16, action: u32) -> bool {
        let Some(it) = self.map().get(id).copied() else { return false };
        let mut mover = self.actors.remove(&id).unwrap_or_else(|| Mover::new(self.people.npc_dirs.get(&id).copied().unwrap_or(4)));
        let ok = mover.play(action, &it, &mut self.data.anims);
        self.actors.insert(id, mover);
        ok
    }

    /// A creature lying on the ground (a skeleton that rises from its
    /// bones) gets up, facing the way it lay, instead of standing there
    /// at once.
    fn get_up(&mut self, id: u16) {
        let Some(it) = self.map().get(id).copied() else { return };
        if !self.data.infos.get(it.shape as usize).is_some_and(|i| i.lies_in(it.frame)) {
            return;
        }
        let Some(up) = self.data.anims.action(it.shape, anim::STAND_UP).filter(|a| a.size > 0) else { return };
        let Some(dir) = (0..8u8).find(|&d| up.frame(d, 0).frame == it.frame) else { return };
        self.mover(id).dir = dir;
        self.animate(id, anim::STAND_UP);
    }

    // ---- entering and leaving a fight ---------------------------------

    pub fn set_in_combat(&mut self, id: u16, on: bool) {
        let s = self.state(id);
        if s.in_combat == on || s.dead {
            return;
        }
        self.state_mut(id).in_combat = on;
        if on {
            self.fights.calmed.remove(&id);
        }
        if on && id != AVATAR {
            // a stroll it was on ends here, or it would walk on and never
            // get round to fighting
            self.kill_processes(id, PROC_WALK);
            self.inputs.remove(&id);
            // the creature's own script for going into a fight (calling
            // for help, fleeing, a word before the fight ...)
            self.call_event(id, 0x11, &0u16.to_le_bytes());
        }
        // weapon out: stand in the fighting pose from now on
        let shape = self.map().get(id).map_or(0, |it| it.shape);
        let has_pose = self.data.anims.action(shape, anim::COMBAT_STAND).is_some();
        self.mover(id).stand_anim = if on && has_pose { anim::COMBAT_STAND } else { anim::STAND };
        if on && id != AVATAR {
            self.get_up(id);
        }
        if id == AVATAR {
            self.animate(id, if on { anim::READY_WEAPON } else { anim::UNREADY_WEAPON });
            // the fight music plays for as long as the weapon is out
            if on {
                self.audio.play_combat_music(MUSIC_COMBAT);
            } else {
                self.audio.restore_music();
            }
        }
    }

    /// Is the Avatar's weapon drawn?
    pub fn fighting(&self) -> bool {
        self.fights.states.get(&AVATAR).is_some_and(|s| s.in_combat && !s.dead)
    }

    /// The player draws or puts away the weapon.
    pub fn toggle_combat(&mut self) {
        if self.stasis || self.state(AVATAR).dead || self.mover(AVATAR).busy() {
            return;
        }
        let on = !self.state(AVATAR).in_combat;
        self.set_in_combat(AVATAR, on);
    }

    /// The Avatar strikes (or kicks) in a direction, if he is ready to.
    pub fn avatar_attack(&mut self, dir: Option<u8>, kick: bool) {
        let s = self.state(AVATAR);
        if !s.in_combat || s.dead || self.stasis || self.mover(AVATAR).busy() {
            if crate::debug::env().debug {
                let (stasis, busy, last) = (self.stasis, self.mover(AVATAR).busy(), self.mover(AVATAR).last_anim());
                eprintln!("attack refused: combat {} dead {} stasis {stasis} busy {busy} last {last}", s.in_combat, s.dead);
            }
            return;
        }
        if let Some(d) = dir {
            self.mover(AVATAR).dir = d & 7;
        }
        self.animate(AVATAR, if kick { anim::KICK } else { anim::ATTACK });
    }

    /// The Avatar raises or lowers his guard.
    pub fn avatar_block(&mut self, on: bool) {
        let s = self.state(AVATAR);
        if !s.in_combat || s.dead || self.stasis {
            return;
        }
        let blocking = self.mover(AVATAR).last_anim() == anim::START_BLOCK;
        if on && !blocking && !self.mover(AVATAR).busy() {
            self.animate(AVATAR, anim::START_BLOCK);
            self.mover(AVATAR).hold = true;
        } else if !on && blocking {
            self.mover(AVATAR).hold = false;
            self.animate(AVATAR, anim::STOP_BLOCK);
        }
    }

    // ---- hits ------------------------------------------------------------

    /// An attack animation has reached a frame that can hit: look for a
    /// victim in front of the attacker. Returns true if someone was hit.
    pub fn weapon_hit(&mut self, attacker: u16, range: i32) -> bool {
        let Some(it) = self.map().get(attacker).copied() else { return false };
        let Some(si) = self.data.infos.get(it.shape as usize).copied() else { return false };
        let dir = self.mover(attacker).dir as usize;
        if attacker == AVATAR && crate::debug::env().debug {
            eprintln!("avatar swing: dir {dir} range {range} at {},{},{}", it.x, it.y, it.z);
        }
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        // flat creatures still strike and get struck at ankle height
        let tall = |mut b: WorldBox| {
            b.zd = b.zd.max(16);
            b
        };
        let mut b = tall(WorldBox::of(&it, &si));
        b.x += XF[dir] * 32 * range;
        b.y += YF[dir] * 32 * range;
        let in_reach: Vec<u16> = self.map().items_near(it.x, it.y, 320).into_iter().filter_map(|i| {
            let t = &self.map().items[i];
            let ti = self.data.infos.get(t.shape as usize)?;
            (self.can_fight(t) && t.id != attacker && !t.dead && b.overlaps(&tall(WorldBox::of(t, ti)))).then_some(t.id)
        }).collect();
        // the one it aims at if it is within reach (a daemon fighting the
        // Avatar does not strike the sorcerer standing next to him)
        let aim = if attacker == AVATAR { 0 } else { self.state(attacker).target };
        let victim = in_reach.iter().copied().find(|&v| v == aim).or(in_reach.first().copied());
        match victim {
            Some(v) => {
                self.receive_hit(v, attacker, dir as u8, 0, 0);
                if attacker == AVATAR {
                    self.magic_weapon_hit(v);
                }
                true
            }
            None => false,
        }
    }

    /// What the Avatar's magic weapons do when they strike: the Hammer of
    /// Strength rings, Slayer calls lightning and thunder when it kills,
    /// Flame Sting sets the ground in front of the Avatar on fire.
    fn magic_weapon_hit(&mut self, victim: u16) {
        const HAMMER: u32 = 0x32F;
        const SLAYER: u32 = 0x330;
        const FLAME_STING: u32 = 0x331;
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        let Some(weapon) = self.equipped(AVATAR, crate::equip::SLOT_WEAPON) else { return };
        let volume = self.sfx_volume(AVATAR);
        match weapon.shape {
            HAMMER => self.audio.play_sfx(23, 0, false, volume),
            SLAYER if self.map().get(victim).is_some_and(|it| it.is_actor()) && self.state(victim).dead => {
                self.start_fade((207, 207, 207), 200, 0, 10);
                let sfx = [91, 94, 96][(self.rng.next() % 3) as usize];
                self.audio.play_sfx(sfx, 0, false, volume);
            }
            FLAME_STING => {
                let av = self.avatar();
                let dir = self.mover(AVATAR).dir as usize & 7;
                let at = (av.x + 96 * XF[dir], av.y + 96 * YF[dir], av.z);
                // a burst of flame, then the fire field (its own script
                // burns whoever stands in it)
                self.create_sprite(480, 0, 9, 1, 2, at);
                let sfx = if self.rng.next() & 1 == 0 { 33 } else { 101 };
                self.audio.play_sfx(sfx, 0, false, volume);
                let fire = crate::effects::FlameSting { at, field: 0, age: 0 };
                self.add_native(crate::vm::Kind::FlameSting(fire), 0, crate::effects::PROC_FLAME_STING);
            }
            _ => {}
        }
    }

    fn damage_amount(&mut self, attacker: u16) -> i32 {
        let shape = self.map().get(attacker).map_or(0, |it| it.shape);
        if attacker == AVATAR {
            let str = self.state(AVATAR).strength;
            if self.mover(AVATAR).last_anim() == anim::KICK {
                // a kick: strength, helped by sturdy leg armour
                self.random(0, str / 2) + self.kick_bonus(AVATAR)
            } else if let Some(w) = self.weapon_info(AVATAR) {
                self.random(0, w.damage_mod) + w.base_damage + str / 5
            } else {
                self.random(0, str / 2) + 1 // bare hands
            }
        } else if let Some(mi) = self.data.monsters.get(&shape).copied() {
            self.random(mi.damage.0, mi.damage.1)
        } else {
            1
        }
    }

    /// What is left of an attack after resistances, armour, blocking and
    /// the roll whether it lands at all.
    fn attack_damage(&mut self, victim: u16, attacker: u16, mut damage: i32, mut damage_type: u16, by_actor: bool) -> i32 {
        // The table has an entry for the Avatar's shape (for his
        // doubles); the player's own protection comes from what he wears.
        let shape = self.map().get(victim).map_or(0, |it| it.shape);
        let mi = if victim == AVATAR { None } else { self.data.monsters.get(&shape).copied() };
        let defense = if victim == AVATAR { self.armour_defense(AVATAR) } else { mi.map_or(0, |m| m.defense_type) };
        damage_type &= !(defense & !(DMG_MAGIC | DMG_UNDEAD | DMG_PIERCE));
        if defense & DMG_MAGIC != 0 && damage_type & DMG_MAGIC == 0 {
            damage = 0;
        }
        let mut slayer = false;
        if damage != 0 && damage_type != 0 {
            if damage_type & DMG_SLAYER != 0 && self.random(0, 9) == 0 {
                slayer = true;
                damage = 255;
            }
            if damage_type & DMG_UNDEAD != 0 && defense & DMG_UNDEAD != 0 {
                damage *= 2;
            }
            if defense & DMG_PIERCE != 0 && damage_type & (DMG_BLADE | DMG_FIRE | DMG_PIERCE) == 0 {
                damage /= 2;
            }
        } else {
            damage = 0;
        }
        if damage != 0 && damage_type & DMG_PIERCE == 0 && !slayer {
            let last = self.mover(victim).last_anim();
            if last == anim::START_BLOCK || last == anim::STOP_BLOCK {
                damage -= self.state(victim).strength / 5;
            }
            let mut ac = 3 * if victim == AVATAR { self.armour_class(AVATAR) } else { mi.map_or(0, |m| m.armour) };
            if damage_type & DMG_FIRE != 0 {
                ac /= 2;
            }
            // what is left; a remainder counts as one more point with
            // the matching chance (2.67 is 3 two times in three)
            let kept = (100 - ac.min(100)) * damage;
            damage = kept / 100;
            if kept % 100 != 0 && self.random(0, 99) < kept % 100 {
                damage += 1;
            }
            damage = damage.max(0);
        }
        if damage != 0 && damage_type & DMG_PIERCE == 0 && by_actor {
            // skill against skill (only with an attacker)
            // the weapon in hand makes its wielder quicker or slower
            let attack = (self.state(attacker).dex + self.weapon_info(attacker).map_or(0, |w| w.attack_dex)).max(1);
            let defend = (self.state(victim).dex + self.weapon_info(victim).map_or(0, |w| w.defend_dex)).max(1);
            // a hit if a roll on the defence stays below a roll on the
            // attack plus three
            if self.random(0, defend - 1) >= self.random(0, attack - 1) + 3 {
                damage = 0;
            } else if attacker == AVATAR {
                // a hit makes the Avatar quicker, the more the quicker the
                // foe is than him (2 * (defence - attack), at least 2)
                self.practise(crate::vitals::Attribute::Dexterity, if defend > attack { 2 * (defend - attack) } else { 2 });
            }
        }
        damage
    }

    /// An actor is struck. `damage` 0: as much as the attacker deals.
    pub fn receive_hit(&mut self, victim: u16, attacker: u16, dir: u8, damage: i32, damage_type: u16) {
        let attacker_is_actor = self.map().get(attacker).is_some_and(|it| self.can_fight(it));
        self.receive_hit_by(victim, attacker, dir, damage, damage_type, attacker_is_actor);
    }

    /// Like `receive_hit`; `by_enemy` tells whether an enemy dealt the hit
    /// (also through a spell of its own) or the world did (water, lava,
    /// traps). It picks the music when the Avatar dies of it.
    pub fn receive_hit_by(&mut self, victim: u16, attacker: u16, dir: u8, damage: i32, damage_type: u16, by_enemy: bool) {
        self.receive_hit_with(victim, attacker, dir, damage, damage_type, by_enemy, true);
    }

    /// A hit a script deals with a fixed amount (receiveHit): no roll of
    /// skill against skill, that belongs to a blow in a fight (those come
    /// with the amount 0). An amount of exactly the victim's health (the
    /// script asked for it first) is a deliberate kill and goes through
    /// whole (Vividos' quake, the end of a trapped book): the scripts wait
    /// for the death that follows, and a miss left the Avatar frozen for
    /// good (found by the test bot).
    pub fn receive_script_hit(&mut self, victim: u16, attacker: u16, dir: u8, damage: i32, damage_type: u16) {
        let deadly = damage > 0 && damage == self.state(victim).hp;
        let damage_type = if deadly { damage_type | DMG_PIERCE } else { damage_type };
        let by_enemy = self.map().get(attacker).is_some_and(|it| self.can_fight(it));
        self.receive_hit_with(victim, attacker, dir, damage, damage_type, by_enemy, damage == 0);
    }

    fn receive_hit_with(&mut self, victim: u16, attacker: u16, dir: u8, mut damage: i32, mut damage_type: u16, by_enemy: bool, roll: bool) {
        if self.state(victim).dead || self.map().get(victim).is_none() {
            return;
        }
        let attacker_is_actor = self.map().get(attacker).is_some_and(|it| self.can_fight(it));
        if damage == 0 && attacker_is_actor {
            damage = self.damage_amount(attacker);
        }
        if damage_type == 0 {
            let shape = self.map().get(attacker).map_or(0, |it| it.shape);
            damage_type = if attacker == AVATAR {
                self.weapon_info(AVATAR).map_or(1, |w| w.damage_type.max(1))
            } else {
                self.data.monsters.get(&shape).map_or(1, |m| m.damage_type.max(1))
            };
        }
        let raw = damage;
        // armour and protection count against every hit, also one without
        // an attacker (the fire bombs of a trapped book), as in the original
        let damage = self.attack_damage(victim, attacker, damage, damage_type, attacker_is_actor && roll);
        if crate::debug::env().debug {
            eprintln!("hit: {attacker} -> {victim}: raw {raw} type {damage_type:X} -> {damage}");
        }
        let volume = self.sfx_volume(victim);

        // being attacked makes creatures fight back; but a stray blow from
        // someone not of its enemies does not turn it from the foe it is
        // fighting (Malchir and his own daemon)
        if victim != AVATAR && attacker_is_actor {
            let s = self.state(victim);
            let was_fighting = s.in_combat;
            let busy = was_fighting && s.target != 0 && s.target != attacker && self.fights.states.get(&s.target).is_some_and(|t| !t.dead);
            let foe = attacker == AVATAR || self.state(attacker).alignment & s.enemy_alignment != 0;
            if !(busy && !foe) {
                self.set_in_combat(victim, true);
                self.state_mut(victim).target = attacker;
                if !was_fighting {
                    self.call_for_help(victim, attacker);
                }
            }
        }
        if damage <= 0 {
            // blocked, parried or missed
            let sfx = self.random(20, 22);
            self.audio.play_sfx(sfx, victim, false, volume);
            return;
        }
        let sfx = self.random(50, 51);
        self.audio.play_sfx(sfx, victim, false, volume);

        // the Avatar grows stronger by striking (not by kicking)
        if attacker == AVATAR && self.mover(AVATAR).last_anim() != anim::KICK {
            self.practise(crate::vitals::Attribute::Strength, damage / 4);
        }
        let s = self.state(victim);
        if !s.immortal {
            if damage >= s.hp {
                if !self.withstand_death(victim) {
                    self.die_by(victim, dir, by_enemy);
                    self.schedule_resurrection(victim, damage_type);
                }
                return;
            }
            self.state_mut(victim).hp -= damage;
        }
        // staggering back interrupts whatever the victim was doing
        let last = self.mover(victim).last_anim();
        if last != anim::START_BLOCK && last != anim::DIE {
            // The victim turns to the blow and staggers back from it. Not
            // the Avatar while the player steers him: a blow from the side
            // snapped him round in the middle of his own fight, away from
            // the enemy he was striking at. He keeps facing his way and
            // staggers back only from a blow in front of him (staggering
            // back from one beside him would carry him off sideways, blow
            // after blow). In a scene he turns, as everyone does.
            let toward_blow = (dir + 4) & 7;
            let steered = victim == AVATAR && self.player_in_control();
            let from_front = matches!((toward_blow + 8 - self.mover(victim).dir) & 7, 0 | 1 | 7);
            if !steered {
                self.mover(victim).dir = toward_blow;
            }
            if !steered || from_front {
                self.animate(victim, anim::STUMBLE_BACKWARDS);
                self.state_mut(victim).wait = 10;
            }
        }
    }

    pub fn die(&mut self, id: u16, dir: u8) {
        self.die_by(id, dir, true);
    }

    /// `by_enemy`: killed by an enemy, not by the world (see `receive_hit_by`).
    pub fn die_by(&mut self, id: u16, _dir: u8, by_enemy: bool) {
        // a changeling in the Avatar's shape dies in its own
        if self.fights.changelings.remove(&id).is_some() {
            self.set_shape(id, CHANGELING);
        }
        {
            let s = self.state_mut(id);
            s.dead = true;
            s.hp = 0;
            s.in_combat = false;
        }
        self.kill_processes(id, crate::vm::PROC_TYPE_ALL);
        self.inputs.remove(&id);
        self.mover(id).stand_anim = anim::DIE; // stay down
        self.mover(id).hold = true;
        self.animate(id, anim::DIE);
        if let Some(idx) = self.map().index(id) {
            // a body on the ground is not in anybody's way
            self.map_mut().items[idx].dead = true;
        }
        if id == AVATAR {
            self.avatar_died = Some(120);
            self.stasis = true;
            // killed by an enemy: "Game Over" as in the original; drowned,
            // burnt or caught in a trap: the album's "Death", or its "Game
            // Over" when it has none
            self.audio.play_combat_music(if by_enemy { MUSIC_DEATH } else { crate::album::DEATH_WORLD });
        } else {
            // an enemy falls while the weapon is out: a fanfare, then on
            // with the fight (option "victory fanfare")
            let hostile = self.state(id).enemy_alignment & self.state(AVATAR).alignment != 0;
            if hostile && self.state(AVATAR).in_combat && self.options.victory_fanfare {
                self.audio.play_once_then(MUSIC_FANFARE, MUSIC_COMBAT);
            }
            self.give_treasure(id);
        }
    }

    /// Is this creature undead (its defence marks it so: skeletons, ghouls,
    /// ghosts, fire skulls)?
    pub fn undead(&self, id: u16) -> bool {
        let shape = self.map().get(id).map_or(0, |it| it.shape);
        self.data.monsters.get(&shape).is_some_and(|m| m.defense_type & DMG_UNDEAD != 0)
    }

    /// A creature that gets up again after being killed (skeletons, as
    /// in the original): in 5 to 30 seconds, unless fire killed it.
    fn schedule_resurrection(&mut self, id: u16, damage_type: u16) {
        let shape = self.map().get(id).map_or(0, |it| it.shape);
        let rises = self.data.monsters.get(&shape).is_some_and(|m| m.resurrection);
        if id == AVATAR || !rises || damage_type & DMG_FIRE != 0 {
            return;
        }
        let wait = self.random(5, 29) as u32 * 30;
        self.fights.resurrections.insert(id, self.tick.wrapping_add(wait));
    }

    /// Someone attacked who was not fighting yet: the others of its side
    /// close by (1024 each way) join in against the attacker. Guards only
    /// help guards; a few kinds never come to help.
    fn call_for_help(&mut self, victim: u16, attacker: u16) {
        const HELP_REACH: i32 = 1024;
        const NEVER_HELP: [u32; 4] = [707, 708, 835, 836];
        const GUARD: u32 = 574;
        let Some(v) = self.map().get(victim).copied() else { return };
        let side = self.state(victim).alignment & 0x0F;
        let helpers: Vec<u16> = self
            .map()
            .items_near(v.x, v.y, HELP_REACH)
            .into_iter()
            .map(|i| self.map().items[i])
            // (not those a script put there peacefully: the theurgists on
            // the Plane of Air stay out of the skellots' fights)
            .filter(|it| it.in_world() && it.fast && it.id != victim && it.id != AVATAR && it.id != attacker && !it.dead && !self.fights.script_made.contains(&it.id))
            .filter(|it| (it.x - v.x).abs() <= HELP_REACH && (it.y - v.y).abs() <= HELP_REACH && self.can_fight(it))
            .filter(|it| if v.shape == GUARD { it.shape == GUARD } else { !NEVER_HELP.contains(&it.shape) })
            .map(|it| it.id)
            .collect();
        for id in helpers {
            let s = self.state(id);
            if s.dead || s.alignment & 0x0F != side {
                continue;
            }
            self.set_in_combat(id, true);
            self.state_mut(id).target = attacker;
        }
    }

    /// The killed creatures whose time has come get up and fight on.
    fn tick_resurrections(&mut self) {
        let now = self.tick;
        let due: Vec<u16> = self.fights.resurrections.iter().filter(|&(_, &at)| now >= at).map(|(&id, _)| id).collect();
        for id in due {
            self.fights.resurrections.remove(&id);
            // gone meanwhile (left behind, burnt, taken apart by a script)
            let Some(idx) = self.map().index(id) else { continue };
            if !self.map().items[idx].in_world() || !self.state(id).dead {
                continue;
            }
            self.map_mut().items[idx].dead = false;
            let full = self.fights.full_hp.get(&id).copied().unwrap_or(1).max(1);
            {
                let s = self.state_mut(id);
                s.dead = false;
                s.hp = full;
            }
            let m = self.mover(id);
            m.hold = false;
            m.stand_anim = anim::STAND;
            // (getting up from where it lay: set_in_combat plays it)
            self.set_in_combat(id, true);
            self.state_mut(id).target = AVATAR;
        }
    }

    /// After the Avatar's death: the view fades out, then the savegame
    /// book opens with the games that can be loaded and "new game".
    fn tick_avatar_death(&mut self) {
        let Some(left) = self.avatar_died else { return };
        if left == 60 {
            self.start_fade((0, 0, 0), 0, 256, 30);
        }
        if left > 0 {
            self.avatar_died = Some(left - 1);
            return;
        }
        if self.ui.slot_menu.is_none() {
            self.close_all_gumps();
            self.open_slot_menu(crate::slots::Mode::Death);
        }
    }

    // ---- behaviour of fighting creatures -----------------------------------


    /// A feigned death (Mask of Death) fools the undead and the daemons
    /// (shape 96): they take the actor for dead (as in the original).
    fn fooled_by_feign(&self, attacker: u16, target: u16) -> bool {
        if !self.feigning_death(target) {
            return false;
        }
        let shape = self.map().get(attacker).map_or(0, |it| it.shape);
        shape == 96 || self.data.monsters.get(&shape).is_some_and(|m| m.defense_type & DMG_UNDEAD != 0)
    }

    /// Someone who fights: an actor, or a creature placed on the map (the
    /// monster table knows its shape) - not whatever else has the number
    /// of a creature of another map.
    pub fn can_fight(&self, it: &crate::map::Item) -> bool {
        it.is_actor() || self.data.monsters.contains_key(&it.shape)
    }

    /// Gives an actor another shape, standing.
    fn set_shape(&mut self, id: u16, shape: u32) {
        if let Some(idx) = self.map().index(id) {
            self.map_mut().items[idx].shape = shape;
            self.map_mut().items[idx].frame = 0;
        }
        self.animate(id, anim::STAND);
    }

    /// The next step of a changeling taking the Avatar's shape, once the
    /// last one's animation is over. True if that step is all it does now.
    fn changeling_step(&mut self, id: u16) -> bool {
        let Some(&shift) = self.fights.changelings.get(&id) else { return false };
        match shift {
            Shift::Turning => {
                self.set_shape(id, AVATAR_SHAPE);
                let seconds = 5 + self.rng.next() % 21;
                self.fights.changelings.insert(id, Shift::Disguised(self.tick + seconds * 30));
                true
            }
            // in the Avatar's likeness it fights on (with the values the
            // monster table gives his shape)
            Shift::Disguised(until) if self.tick < until => false,
            Shift::Disguised(_) => {
                self.set_shape(id, CHANGELING);
                self.animate(id, FROM_AVATAR);
                self.fights.changelings.insert(id, Shift::Returning);
                true
            }
            Shift::Returning => {
                self.fights.changelings.remove(&id);
                false
            }
        }
    }

    /// A conversation is going on: the answers wait for the player, or the
    /// script of someone spoken to still runs (the scripts mark the "use"
    /// of a thing or person with process type 519; a person used is
    /// talked to). Covers the greeting before the answers come up.
    pub fn in_conversation(&self) -> bool {
        const USE_PROCESS: u16 = 519;
        self.ask.is_some()
            || self.scripts.procs.iter().any(|p| {
                !p.terminated && p.ptype == USE_PROCESS && p.item != AVATAR && self.map().get(p.item).is_some_and(|it| it.is_actor())
            })
    }

    fn find_target(&mut self, id: u16) -> u16 {
        let Some(it) = self.map().get(id).copied() else { return 0 };
        let enemies = self.state(id).enemy_alignment;
        // a creature a script made (the golem of the spell) does not turn
        // on the Avatar by itself once its foe has fallen, whatever its
        // kind; only if he strikes it, or a script tells it to
        let spare_avatar = self.fights.script_made.contains(&id);
        let near: Vec<u16> = self
            .map()
            .items_near(it.x, it.y, 768)
            .into_iter()
            .map(|i| self.map().items[i])
            .filter(|t| self.can_fight(t) && t.id != id && !t.dead && !(spare_avatar && t.id == AVATAR))
            .map(|t| t.id)
            .collect();
        near.into_iter().find(|&t| {
            let ts = self.state(t);
            !ts.dead && ts.alignment & enemies != 0 && !self.fooled_by_feign(id, t) && !self.hidden_from(id, t)
        }).unwrap_or(0)
    }

    /// A creature loses the vanished Avatar: it walks on to where he was
    /// (where it saw him last) and looks about there.
    fn lose_avatar(&mut self, id: u16) {
        let av = self.avatar();
        self.kill_processes(id, PROC_WALK);
        self.inputs.remove(&id);
        self.paths.remove(&id);
        let pf = crate::effects::Pathfind::new(id, 0, (av.x, av.y));
        self.add_native(crate::vm::Kind::Pathfind(pf), id, PROC_WALK);
        self.fights.searching.insert(id);
        self.state_mut(id).target = 0;
        self.state_mut(id).wait = 5;
    }

    /// The black potion: an invisible Avatar is no target for a creature,
    /// unless he stands right next to it (it runs into him). It does not
    /// seek him out or shoot at him. The original works so; ScummVM notes
    /// it as still to do.
    fn hidden_from(&self, id: u16, target: u16) -> bool {
        if target != AVATAR {
            return false;
        }
        let (Some(it), Some(av)) = (self.map().get(id), self.map().get(AVATAR)) else { return false };
        av.flags & crate::map::FLG_INVISIBLE != 0 && !(self.box_gap(it, av) <= 20 && (av.z - it.z).abs() < 48)
    }

    /// One decision step for every creature that is in a fight.
    pub fn run_combat(&mut self) {
        self.tick_avatar_death();
        self.tick_resurrections();
        if self.tick.is_multiple_of(15) {
            self.notice_avatar();
        }
        let fighters: Vec<u16> = self.fights.states.iter().filter(|(&id, s)| id != AVATAR && s.in_combat && !s.dead).map(|(&id, _)| id).collect();
        let talk_truce = self.options.talk_truce && self.in_conversation();
        for id in fighters {
            self.combat_step(id, talk_truce);
        }
    }

    /// Hostile creatures notice the Avatar when he comes close (with the
    /// option "enemies notice sooner" half as far again, but not while a
    /// scene holds him: the scripts set up their own fights).
    fn notice_avatar(&mut self) {
        let (reach, scene_safe) = if self.options.notice_far { (NOTICE_REACH * 3 / 2, !self.stasis) } else { (NOTICE_REACH, true) };
        if self.state(AVATAR).dead || !scene_safe {
            return;
        }
        let av = self.avatar();
        let side = self.state(AVATAR).alignment;
        let idle: Vec<u16> = self
            .fights
            .states
            .iter()
            .filter(|(&id, s)| id != AVATAR && !s.in_combat && !s.dead && s.enemy_alignment & side != 0)
            // The townsfolk (side 1: peasants, children, guards) count the
            // Avatar among their enemies only for when it comes to a fight:
            // they never start one on sight, only when attacked or when
            // their script says so (a guard seeing a theft).
            .filter(|(_, s)| s.alignment & TOWNSFOLK == 0)
            .filter(|(id, _)| !self.fights.calmed.contains(id) && !self.fights.script_made.contains(id))
            .map(|(&id, _)| id)
            // The permanent characters (with a number of their own) do not
            // start a fight on sight, hostile or not: their script decides,
            // or a blow. The skeletons lined up in the Lower Catacombs
            // stand there swinging their axes. Only creatures attack.
            .filter(|&id| !self.people.npc_stats.contains_key(&id))
            .collect();
        for id in idle {
            let near = self.map().get(id).is_some_and(|it| {
                it.in_world() && it.fast && self.can_fight(it) && !it.dead && (it.x - av.x).abs().max((it.y - av.y).abs()) <= reach && (it.z - av.z).abs() < 64
            });
            if near && !self.fooled_by_feign(id, AVATAR) && !self.hidden_from(id, AVATAR) && self.in_sight(id, AVATAR) {
                self.set_in_combat(id, true);
                self.state_mut(id).target = AVATAR;
            }
        }
    }

    /// Can `id` see `other`: nothing solid between their heads (a wall, a
    /// closed door)? People in the way do not count. The skeletons behind
    /// the gate of the dungeon below the Fortress do not notice the Avatar
    /// standing outside.
    fn in_sight(&self, id: u16, other: u16) -> bool {
        let (Some(a), Some(b)) = (self.map().get(id).copied(), self.map().get(other).copied()) else { return false };
        let head = |it: &crate::map::Item| {
            let si = self.data.infos.get(it.shape as usize);
            let (xd, yd, zd) = si.map_or((0, 0, 0), |si| si.footpad(it.flags & crate::map::FLG_FLIPPED != 0));
            (it.x - xd / 2, it.y - yd / 2, it.z + (zd * 3 / 4).max(8))
        };
        let (from, to) = (head(&a), head(&b));
        let (dx, dy, dz) = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
        let steps = (dx.abs().max(dy.abs()) / 16).max(1);
        for k in 1..steps {
            let (x, y, z) = (from.0 + dx * k / steps, from.1 + dy * k / steps, from.2 + dz * k / steps);
            let probe = WorldBox { x: x + 4, y: y + 4, z: z - 4, xd: 8, yd: 8, zd: 8 };
            // (a start far away: things the probe overlaps all count)
            let nowhere = WorldBox { x: -100_000, y: -100_000, z: -100_000, xd: 0, yd: 0, zd: 0 };
            let pi = self.map().position_info(&probe, &nowhere, &self.data.infos, id);
            if !pi.valid && !pi.blocker.and_then(|b| self.map().get(b)).is_some_and(|t| t.is_actor()) {
                return false;
            }
        }
        true
    }

    /// One decision of a creature in a fight.
    fn combat_step(&mut self, id: u16, talk_truce: bool) {
        // (only those who can fight: a number from another map's state
        // may name a floor tile here)
        let Some(it) = self.map().get(id).copied().filter(|it| it.in_world() && it.fast && self.can_fight(it)) else { return };
        if self.state(id).wait > 0 {
            self.state_mut(id).wait -= 1;
            return;
        }
        if self.walk_interrupted(id, &it, talk_truce) {
            return;
        }
        if self.mover(id).busy() || self.inputs.contains_key(&id) {
            if crate::debug::env().debug && self.tick.is_multiple_of(20) {
                eprintln!("combat {id}: busy={} last={} input={}", self.mover(id).busy(), self.mover(id).last_anim(), self.inputs.contains_key(&id));
            }
            return; // in the middle of a move
        }
        if self.changeling_step(id) {
            return;
        }
        // someone to fight?
        let mut target = self.state(id).target;
        let valid = |g: &mut Game, t: u16| {
            t != 0 && g.map().get(t).is_some_and(|x| x.in_world() && x.fast) && !g.state(t).dead && !g.fooled_by_feign(id, t) && !g.hidden_from(id, t)
        };
        if !valid(self, target) {
            target = self.find_target(id);
            self.state_mut(id).target = target;
            if target == 0 {
                // a creature a script made (the golem) is done fighting:
                // a click gives it orders again
                if self.fights.script_made.contains(&id) {
                    self.set_in_combat(id, false);
                    return;
                }
                self.state_mut(id).wait = 30;
                return;
            }
        }
        let Some(t) = self.map().get(target).copied() else { return };
        self.fight(id, &it, &t, talk_truce);
    }

    /// Things that stop a creature on its way somewhere. Returns true if
    /// one did (the step is over).
    fn walk_interrupted(&mut self, id: u16, it: &crate::map::Item, talk_truce: bool) -> bool {
        // a changeling on its way to the Avatar may take his shape too
        // (a chance once a second)
        if it.shape == CHANGELING && self.cur_map != ENDGAME_MAP && self.inputs.contains_key(&id) && !self.fights.changelings.contains_key(&id)
            && self.tick.is_multiple_of(30) && self.rng.next().is_multiple_of(WALK_SHIFT_CHANCE)
        {
            self.kill_processes(id, PROC_WALK);
            self.inputs.remove(&id);
            self.paths.remove(&id);
            if self.animate(id, TO_AVATAR) {
                self.fights.changelings.insert(id, Shift::Turning);
            }
            return true;
        }
        // option "enemies wait during talks": an enemy on its way to the
        // Avatar stops where it is while a conversation goes on
        if talk_truce && self.state(id).target == AVATAR && self.inputs.contains_key(&id) {
            self.kill_processes(id, PROC_WALK);
            let av = self.avatar();
            self.stop_walk(id, it, (av.x - it.x, av.y - it.y));
            self.state_mut(id).wait = 10;
            return true;
        }
        // the Avatar has vanished (black potion): the creature loses him
        // and goes on to where it saw him last
        if self.state(id).target == AVATAR && self.hidden_from(id, AVATAR) {
            self.lose_avatar(id);
            return true;
        }
        // seen again while still on its way there: after him
        if !self.inputs.contains_key(&id) {
            self.fights.searching.remove(&id);
        } else if self.state(id).target == 0 && self.fights.searching.contains(&id) {
            let av = self.avatar();
            let seen = !self.hidden_from(id, AVATAR) && self.state(id).enemy_alignment & self.state(AVATAR).alignment != 0;
            if seen && (av.x - it.x).abs().max((av.y - it.y).abs()) <= 768 {
                self.kill_processes(id, PROC_WALK);
                self.stop_walk(id, it, (av.x - it.x, av.y - it.y));
                self.fights.searching.remove(&id);
                self.state_mut(id).target = AVATAR;
                return true;
            }
        }
        false
    }

    /// A creature `it` against its target `t`: strikes, shoots or closes in.
    fn fight(&mut self, id: u16, it: &crate::map::Item, t: &crate::map::Item, talk_truce: bool) {
        let target = t.id;
        let (dx, dy) = (t.x - it.x, t.y - it.y);
        let dir = world_dir(dx, dy) as u8;
        // option "enemies wait during talks": during a conversation (see
        // in_conversation), his enemies only face him
        if target == AVATAR && talk_truce {
            self.mover(id).dir = dir;
            self.state_mut(id).wait = 10;
            return;
        }
        let gap = self.box_gap(it, t);
        let shape_ranged = self.data.monsters.get(&it.shape).is_some_and(|m| m.ranged);
        if crate::debug::env().debug {
            eprintln!("combat {id}: target {target} gap {gap} dz {} ranged {shape_ranged}", t.z - it.z);
        }
        // a changeling takes the Avatar's shape now and then instead
        if it.shape == CHANGELING && self.cur_map != ENDGAME_MAP && self.rng.next().is_multiple_of(SHIFT_CHANCE) {
            self.mover(id).dir = dir;
            if self.animate(id, TO_AVATAR) {
                self.fights.changelings.insert(id, Shift::Turning);
                return;
            }
        }
        // one tile is the shortest reach of an attack
        if gap <= 20 && (t.z - it.z).abs() < 48 && !shape_ranged {
            self.mover(id).dir = dir;
            if self.animate(id, anim::ATTACK) {
                // slower creatures need longer to recover
                let dex = self.state(id).dex;
                self.state_mut(id).wait = 12 + 3 * (25 - dex).max(0);
            } else {
                self.state_mut(id).wait = 30;
            }
        } else if dx.abs().max(dy.abs()) > 1500 {
            self.state_mut(id).target = 0;
        } else if shape_ranged {
            // attacks from where it stands; the attack animation
            // itself lets the missile loose. Now and then (one time in
            // five, as in ScummVM) it makes a threatening gesture instead:
            // the ghost's gesture calls a ghoul out of the ground.
            self.mover(id).dir = dir;
            let dex = self.state(id).dex;
            let gesture = self.rng.next().is_multiple_of(5) && {
                let idle = if self.rng.next() & 1 == 0 { anim::IDLE1 } else { anim::IDLE2 };
                self.animate(id, idle) || self.animate(id, anim::IDLE1 + anim::IDLE2 - idle)
            };
            self.state_mut(id).wait = if gesture {
                10
            } else if self.animate(id, anim::ATTACK) {
                40 + 3 * (25 - dex).max(0)
            } else {
                30
            };
        } else {
            // A walk that brought it no closer (the enemy up on a wall it
            // cannot climb): wait longer each time before the next one.
            let fails = {
                let e = self.fights.approach_fails.entry(id).or_insert((i32::MAX, 0));
                e.1 = if gap >= e.0 - 8 { e.1 + 1 } else { 0 };
                e.0 = gap;
                e.1
            };
            if fails >= 2 && fails % 2 == 0 {
                self.mover(id).dir = dir;
                self.state_mut(id).wait = 30 * fails.min(12) as i32 / 2;
                return;
            }
            // close in; the walk ends next to the target or gives up
            let mut pf = crate::effects::Pathfind::new(id, target, (0, 0));
            pf.touch = true;
            self.add_native(crate::vm::Kind::Pathfind(pf), id, PROC_WALK);
            self.state_mut(id).wait = 5;
        }
    }

    // ---- the health display --------------------------------------------------

    /// Optional: a small bar above every creature that fights the Avatar,
    /// in the red of the Avatar's own health bar with a rim of old gold.
    pub fn draw_health_bars(&mut self, surf: &mut Surface) {
        const WIDTH: i32 = HEALTH_BAR_WIDTH;
        if !self.options.enemy_health {
            return;
        }
        // A creature behind a wall or a roof shows no bar: it needs a few
        // pixels of its own in the picture (the per-pixel id buffer).
        const MIN_SEEN: u32 = 6;
        let fighters: Vec<usize> = self
            .view
            .painted
            .iter()
            .map(|p| p.index)
            .filter(|&i| {
                self.map().items.get(i).is_some_and(|it| {
                    it.id != AVATAR
                        && self.can_fight(it)
                        && self.fights.states.get(&it.id).is_some_and(|s| !s.dead && s.in_combat && s.target == AVATAR)
                })
            })
            .collect();
        let ids = self.drawn_ids();
        let check = !fighters.is_empty() && ids.len() == surf.pixels.len();
        // only the area around each creature is looked at (its picture
        // reaches up from its foot point, a little to the sides)
        let seen: Vec<u32> = fighters
            .iter()
            .map(|&index| {
                if !check {
                    return 0;
                }
                let it = self.map().items[index];
                let (sx, sy) = self.to_screen(it.x, it.y, it.z);
                let want = index as u32 + 1;
                let (x0, x1) = ((sx - 96).clamp(0, surf.width), (sx + 96).clamp(0, surf.width));
                let (y0, y1) = ((sy - 192).clamp(0, surf.height), (sy + 48).clamp(0, surf.height));
                let mut n = 0;
                for y in y0..y1 {
                    let row = (y * surf.width) as usize;
                    n += ids[row + x0 as usize..row + x1.max(x0) as usize].iter().filter(|&&v| v == want).count() as u32;
                }
                n
            })
            .collect();
        let mut bars = Vec::new();
        self.ui.bar_areas.clear();
        for (k, &index) in fighters.iter().enumerate() {
            if check && seen[k] < MIN_SEEN {
                continue;
            }
            let Some(it) = self.map().items.get(index) else { continue };
            let Some(s) = self.fights.states.get(&it.id) else { continue };
            let height = self.data.infos.get(it.shape as usize).map_or(0, |si| si.z * 8);
            let (sx, sy) = self.to_screen(it.x, it.y, it.z);
            bars.push((it.id, it.shape, s.hp, sx - WIDTH / 2, sy - height - 12));
            self.ui.bar_areas.push((it.id, sx - WIDTH / 2 - 6, sy - height - 18, sx + WIDTH / 2 + 6, sy + 6));
        }
        let bars: Vec<(i32, i32, i32)> = bars
            .into_iter()
            .map(|(id, shape, hp, x, y)| {
                // A creature from a savegame: what it started with is not
                // known, so take what it has now (or the least its kind
                // starts with) and count down from there.
                let least = self.data.monsters.get(&shape).map_or(0, |m| m.hp.0);
                let full = self.fights.full_hp.entry(id).or_insert(hp.max(least));
                *full = (*full).max(hp).max(1);
                (x, y, (hp.max(0) * WIDTH + *full - 1) / *full)
            })
            .collect();
        for (x0, y0, filled) in bars {
            for dy in -2..5 {
                for dx in -2..WIDTH + 2 {
                    surf.put(x0 + dx, y0 + dy, health_bar_colour(dx, dy, filled));
                }
            }
        }
    }

    /// The small status window: red bar for health, blue for mana.
    pub fn draw_status(&mut self, surf: &mut Surface) {
        const SHAPE: u32 = 33;
        const BAR_BOTTOM: i32 = 19;
        const BAR_HEIGHT: i32 = 14;
        let Some(shape) = self.data.gump_shapes.get(SHAPE) else { return };
        let Some(f) = shape.frames.first() else { return };
        let (x0, y0) = (surf.width - f.width - 6, 6);
        surf.paint(f, x0 + f.xoff, y0 + f.yoff, false, false, &self.data.pal);
        let s = self.state(AVATAR);
        let max_hp = self.max_hp(AVATAR).max(1);
        let max_mana = (2 * s.intelligence).max(1);
        let bars = [
            (6, s.hp.clamp(0, max_hp) * BAR_HEIGHT / max_hp, [0x98_04_04u32, 0xBC_0C_0C, 0xD4_30_30]),
            (13, self.mana.clamp(0, max_mana) * BAR_HEIGHT / max_mana, [0x40_50_FC, 0x1C_28_FC, 0x0C_0C_CC]),
        ];
        for (bx, height, colours) in bars {
            for (i, c) in colours.iter().enumerate() {
                for dy in 0..height {
                    let (px, py) = (x0 + bx + i as i32, y0 + BAR_BOTTOM - dy);
                    if px >= 0 && py >= 0 && px < surf.width && py < surf.height {
                        surf.pixels[(py * surf.width + px) as usize] = *c;
                    }
                }
            }
        }
    }
}
