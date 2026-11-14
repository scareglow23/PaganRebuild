//! Actor animations from ANIM.DAT: per shape up to 64 actions (walk, run,
//! stand ...), each with a frame list for the 8 directions.

use crate::flex::u32le;
use crate::hash::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

pub const WALK: u32 = 0;
pub const RUN: u32 = 1;
pub const STAND: u32 = 2;
pub const JUMP_UP: u32 = 3;
pub const STAND_UP: u32 = 4;
pub const RUNNING_JUMP: u32 = 10;
/// Coming down after a fall: knees bent, getting up.
pub const LAND: u32 = 16;
pub const SHAKE_HEAD: u32 = 11;
pub const JUMP: u32 = 17;
/// The jump that can be steered to a spot.
pub const AIRWALK_JUMP: u32 = 18;
/// Climbing onto ledges 16, 24 ... 72 units high: animations 19..=26.
pub const CLIMB_FIRST: u32 = 19;
pub const CLIMB_LAST: u32 = 26;
/// The highest ledge there is a climb for.
pub const CLIMB_MAX: i32 = 72;
pub const READY_WEAPON: u32 = 5;
pub const UNREADY_WEAPON: u32 = 6;
pub const ATTACK: u32 = 7;
pub const STUMBLE_BACKWARDS: u32 = 13;
pub const DIE: u32 = 14;
pub const COMBAT_STAND: u32 = 15;
/// Threatening gestures in a fight (not every creature has them).
pub const IDLE1: u32 = 47;
pub const IDLE2: u32 = 48;
pub const KICK: u32 = 58;
pub const START_BLOCK: u32 = 59;
pub const STOP_BLOCK: u32 = 60;

const AAF_TWOSTEP: u32 = 0x0001;
const AAF_LOOPING: u32 = 0x0004 | 0x0010;

#[derive(Clone, Copy)]
pub struct AnimFrame {
    /// Frame of the actor's shape to show.
    pub frame: u32,
    pub delta_z: i32,
    /// Distance moved in the facing direction, in units of 4.
    pub delta_dir: i32,
    /// Sound effect to play with the frame (footsteps ...), 0 = none.
    pub sfx: u8,
    flags: u32,
}

impl AnimFrame {
    /// The actor has to stand on something during this frame.
    pub fn on_ground(&self) -> bool {
        self.flags & 0x0002 != 0
    }
    /// How far in front of the actor (in tiles) this frame of an attack
    /// reaches; 0 for frames that cannot hit.
    pub fn attack_range(&self) -> i32 {
        ((self.flags >> 2) & 0x07) as i32
    }
    /// A foot touches the ground in this frame.
    pub fn footstep(&self) -> bool {
        self.flags & 0x0800 != 0
    }
    pub fn flipped(&self) -> bool {
        self.flags & 0x0020 != 0
    }
}

pub struct AnimAction {
    pub size: usize,
    /// Number of extra ticks each frame is shown.
    pub frame_repeat: u32,
    flags: u32,
    frames: [Vec<AnimFrame>; 8],
}

impl AnimAction {
    pub fn two_step(&self) -> bool {
        self.flags & AAF_TWOSTEP != 0
    }
    pub fn looping(&self) -> bool {
        self.flags & AAF_LOOPING != 0
    }
    pub fn frame(&self, dir: u8, n: usize) -> &AnimFrame {
        &self.frames[dir as usize & 7][n]
    }

    /// Frame range (start, end) to play; `end` is one past the last frame
    /// and may be smaller than `start` for looping animations.
    pub fn range(&self, action: u32, last_anim: u32, last_dir: u8, first_step: bool, dir: u8) -> (usize, usize) {
        let (mut start, mut end) = (0, self.size);
        if self.two_step() {
            // one call plays one step, i.e. half of the cycle
            if first_step {
                start = if self.looping() { self.size - 1 } else { 0 };
                end = self.size / 2;
            } else {
                start = self.size / 2;
                if self.looping() {
                    end = self.size - 1;
                }
            }
        } else if last_anim == action && last_dir == dir && self.size > 1 {
            start = 1; // skip the first frame when repeating
        }
        (start, end)
    }

    /// How far the actor moves on average per game tick in direction
    /// `dir` (in units of 4, along the facing direction), over one whole
    /// cycle of the animation. The single frames move it by quite different
    /// amounts (the Avatar's walk: 4 to 8 per frame); this is the even pace
    /// they add up to. A looping animation cycles through frames 1 to the
    /// last (frame 0 only starts it).
    pub fn average_stride(&self, dir: u8) -> f32 {
        let first = if self.looping() && self.size > 1 { 1 } else { 0 };
        let frames = self.size - first;
        if frames == 0 {
            return 0.0;
        }
        let along: i32 = (first..self.size).map(|n| self.frame(dir, n).delta_dir.abs()).sum();
        along as f32 / (frames as f32 * (self.frame_repeat + 1) as f32)
    }

    pub fn next_frame(&self, frame: usize, end: usize) -> usize {
        let mut f = frame + 1;
        if f == end {
            return end;
        }
        if f >= self.size {
            f = if self.looping() { 1 } else { 0 };
        }
        f
    }
}

pub struct AnimDat {
    data: Vec<u8>,
    cache: HashMap<(u32, u32), Option<Rc<AnimAction>>>,
}

impl AnimDat {
    pub fn load(path: &Path) -> io::Result<AnimDat> {
        Ok(AnimDat { data: fs::read(path)?, cache: HashMap::default() })
    }

    pub fn action(&mut self, shape: u32, action: u32) -> Option<Rc<AnimAction>> {
        if !self.cache.contains_key(&(shape, action)) {
            let a = self.parse(shape as usize, action as usize).map(Rc::new);
            self.cache.insert((shape, action), a);
        }
        self.cache[&(shape, action)].clone()
    }

    fn parse(&self, shape: usize, action: usize) -> Option<AnimAction> {
        let d = &self.data;
        if shape >= 2048 || action >= 64 || d.len() < 2048 * 4 {
            return None;
        }
        let off = u32le(d, shape * 4) as usize;
        if off == 0 || off + 64 * 4 > d.len() {
            return None;
        }
        let aoff = u32le(d, off + action * 4) as usize;
        if aoff == 0 || aoff + 4 > d.len() {
            return None;
        }
        let size = d[aoff] as usize;
        let flags = d[aoff + 1] as u32 | ((d[aoff + 3] as u32) << 8);
        let frame_repeat = (d[aoff + 2] & 0x0F) as u32;
        if size == 0 || aoff + 4 + 8 * size * 6 > d.len() {
            return None;
        }
        let frames = std::array::from_fn(|dir| {
            (0..size)
                .map(|j| {
                    let r = &d[aoff + 4 + (dir * size + j) * 6..];
                    AnimFrame {
                        frame: r[0] as u32 | (((r[1] & 0x07) as u32) << 8),
                        delta_z: r[2] as i8 as i32,
                        sfx: r[3],
                        delta_dir: r[4] as i8 as i32,
                        flags: r[5] as u32 | (((r[1] & 0xF8) as u32) << 8),
                    }
                })
                .collect()
        });
        Some(AnimAction { size, frame_repeat, flags, frames })
    }
}
