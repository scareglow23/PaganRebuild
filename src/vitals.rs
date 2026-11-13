//! What keeps the Avatar going between fights: hit points and mana come
//! back over time, food heals as much as he has gone hungry, and use
//! makes him stronger, quicker and wiser. Also two magic states of
//! actors: "withstand death" (one deadly blow is survived) and "feign
//! death" (lying down for a while, so enemies leave him alone).
//!
//! As in the original game and ScummVM's HealProcess / MainActor.

use crate::anim;
use crate::game::{Game, AVATAR};
use crate::hash::{HashMap, HashSet};

/// Ticks (30 per second) until one point of mana comes back ...
const MANA_TICKS: u16 = 900;
/// ... and one hit point (which also makes the Avatar a little hungrier).
const HP_TICKS: u16 = 1800;
/// Hunger does not grow beyond this.
const MAX_HUNGER: u16 = 200;
/// Practice needed for one more point of strength, dexterity or intelligence.
pub const PRACTICE: i32 = 650;
/// No attribute grows beyond this.
pub const MAX_ATTRIBUTE: i32 = 25;
/// Sound of an attribute going up.
const SFX_GAIN: i32 = 0x36;
/// Sound of a deadly blow being withstood.
const SFX_WITHSTOOD: i32 = 59;
/// How long a feigned death lasts, after getting up.
const FEIGN_TICKS: u32 = 900;
/// Time for falling down before getting up again.
const FALL_TICKS: u32 = 45;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Vitals {
    heal_counter: u16,
    hunger: u16,
    /// Practice towards strength, dexterity, intelligence.
    practice: [i32; 3],
    /// Actors that survive the next deadly blow.
    pub withstand: HashSet<u16>,
    /// Actors feigning death: (tick to get up, tick it ends).
    pub feign: HashMap<u16, (u32, u32)>,
}

/// Which attribute is practised.
#[derive(Clone, Copy)]
pub enum Attribute {
    Strength = 0,
    Dexterity = 1,
    Intelligence = 2,
}

impl Game {
    /// Once per tick.
    pub fn tick_vitals(&mut self) {
        let s = self.state(AVATAR);
        if !s.dead {
            self.vitals.heal_counter += 1;
            if self.vitals.heal_counter == MANA_TICKS && self.mana < 2 * s.intelligence {
                self.mana += 1;
            }
            if self.vitals.heal_counter >= HP_TICKS {
                if s.hp < 2 * s.strength {
                    self.state_mut(AVATAR).hp += 1;
                }
                self.vitals.heal_counter = 0;
                self.vitals.hunger = (self.vitals.hunger + 1).min(MAX_HUNGER);
            }
        }
        if !self.vitals.feign.is_empty() {
            let now = self.tick;
            let due: Vec<(u16, bool)> = self
                .vitals
                .feign
                .iter()
                .filter_map(|(&id, &(up, end))| if now == up { Some((id, false)) } else if now >= end { Some((id, true)) } else { None })
                .collect();
            for (id, ended) in due {
                if ended {
                    self.vitals.feign.remove(&id);
                } else if !self.state(id).dead {
                    self.mover(id).stand_anim = anim::STAND;
                    self.mover(id).hold = false;
                    self.animate(id, anim::STAND_UP);
                }
            }
        }
    }

    /// FeedAvatar: food heals as much as the Avatar has gone hungry.
    pub fn feed_avatar(&mut self, food: u16) {
        let s = self.state(AVATAR);
        if s.dead {
            return;
        }
        let food = food.min(self.vitals.hunger);
        if food == 0 {
            return;
        }
        let old = self.vitals.hunger;
        self.vitals.hunger -= food;
        let healed = (old / 4 - self.vitals.hunger / 4) as i32;
        if s.hp < 2 * s.strength {
            self.state_mut(AVATAR).hp = (s.hp + healed).min(2 * s.strength);
        }
    }

    /// Practice gathered towards the next point of an attribute (of `PRACTICE`).
    pub fn practice_of(&self, what: Attribute) -> i32 {
        self.vitals.practice[what as usize]
    }

    /// Sets the practice of strength, dexterity, intelligence (for tests).
    pub fn set_practice(&mut self, practice: [i32; 3]) {
        self.vitals.practice = practice;
    }

    /// Use makes the Avatar better: after enough practice (sooner with luck)
    /// the attribute goes up by one.
    pub fn practise(&mut self, what: Attribute, amount: i32) {
        let s = self.state(AVATAR);
        let cur = match what {
            Attribute::Strength => s.strength,
            Attribute::Dexterity => s.dex,
            Attribute::Intelligence => s.intelligence,
        };
        if cur >= MAX_ATTRIBUTE {
            return;
        }
        let p = &mut self.vitals.practice[what as usize];
        *p += amount;
        let left = PRACTICE - *p;
        let lucky = left > 0 && self.rng.next().is_multiple_of(left as u32 + 1);
        if left <= 0 || lucky {
            self.vitals.practice[what as usize] = 0;
            let st = self.state_mut(AVATAR);
            match what {
                Attribute::Strength => st.strength += 1,
                Attribute::Dexterity => st.dex += 1,
                Attribute::Intelligence => st.intelligence += 1,
            }
            let volume = self.sfx_volume(AVATAR);
            self.audio.play_sfx(SFX_GAIN, AVATAR, false, volume);
            if crate::debug::env().debug {
                eprintln!("practice: {} up to {}", ["strength", "dexterity", "intelligence"][what as usize], cur + 1);
            }
        }
    }

    /// A blow that would kill `id`: true if a "withstand death" spell
    /// takes it instead (the actor is healed fully).
    pub fn withstand_death(&mut self, id: u16) -> bool {
        if !self.vitals.withstand.remove(&id) {
            return false;
        }
        let full = self.max_hp(id);
        self.state_mut(id).hp = full;
        let volume = self.sfx_volume(id);
        self.audio.play_sfx(SFX_WITHSTOOD, id, false, volume);
        true
    }

    /// The actor falls down as if dead, gets up again and is left alone
    /// by enemies for a while.
    pub fn feign_death(&mut self, id: u16) {
        if self.vitals.feign.contains_key(&id) {
            return;
        }
        let up = self.tick.wrapping_add(FALL_TICKS);
        self.vitals.feign.insert(id, (up, up.wrapping_add(FEIGN_TICKS)));
        self.mover(id).stand_anim = anim::DIE;
        self.mover(id).hold = true;
        self.animate(id, anim::DIE);
    }

    /// For test runs.
    pub fn describe_vitals(&mut self) -> String {
        let s = self.state(AVATAR);
        format!(
            "hp {}/{} mana {}/{} hunger {} str {} dex {} int {} practice {:?}",
            s.hp,
            2 * s.strength,
            self.mana,
            2 * s.intelligence,
            self.vitals.hunger,
            s.strength,
            s.dex,
            s.intelligence,
            self.vitals.practice
        )
    }

    pub fn feigning_death(&self, id: u16) -> bool {
        self.vitals.feign.contains_key(&id)
    }
}
