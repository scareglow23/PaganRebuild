//! The test bot: `bot:<seed>,<ticks>` plays on its own for a number of
//! game ticks (walking, running, jumping, using things, answering, fighting)
//! and checks rules the game must keep. It only does what a player can:
//! keys, and clicks on what is on screen. Every breach is printed as a line
//! "BUG <kind> | map .. at x,y,z | tick .. | <detail>" and saved as a
//! savegame (slot 900 on, in PAGAN_SAVE_DIR) to look at later. The same
//! seed plays the same way.

use super::*;
use crate::game::AVATAR;
use crate::map::WorldBox;
use crate::hash::HashSet;

/// After this many ticks a breach counts (shorter ones are part of normal
/// play: a jump, a step down).
const FLOAT_TICKS: u32 = 60;
/// A scene that holds the Avatar this long without a question, a book or
/// speech on screen is stuck.
const STUCK_TICKS: u32 = 3600;
/// Each kind of breach is reported once per run and place.
const SAME_PLACE: i32 = 1024;
/// At most this many reports of one kind per run.
const PER_KIND: usize = 5;
/// The savegame slot the bot goes back to after a death.
const CHECKPOINT: usize = 899;

pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: i32) -> Rng {
        Rng(seed as u64 * 0x9E37_79B9_7F4A_7C15 + 1)
    }
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

/// What the bot is doing.
enum Plan {
    Walk { dir: u8, run: bool, left: u32 },
    Wait(u32),
}

/// The rules the game must keep, checked after every tick, and how much of
/// the game a run got to see (used by both bots: `bot` and `explore`).
pub(super) struct Watch {
    /// Ticks played in this run (to replay up to a finding).
    pub step: u32,
    pub found: Vec<(String, (i32, i32, i32))>,
    /// The last breach reported, as printed.
    pub last: String,
    floating: u32,
    inside: u32,
    held: u32,
    saves: usize,
    /// Places stood on (map, x / CELL, y / CELL).
    cells: HashSet<(usize, i32, i32)>,
    /// Maps entered.
    maps: HashSet<usize>,
    /// Script classes that ran (what of the game's scripts got to work).
    classes: HashSet<u16>,
}

/// The size of a place for the coverage (a room, a stretch of street).
const CELL: i32 = 256;

impl Watch {
    pub fn new() -> Watch {
        Watch { step: 0, found: Vec::new(), last: String::new(), floating: 0, inside: 0, held: 0, saves: 0, cells: HashSet::default(), maps: HashSet::default(), classes: HashSet::default() }
    }

    /// Back at the checkpoint after a death: what was counted up starts again.
    pub fn restart(&mut self) {
        (self.floating, self.inside, self.held) = (0, 0, 0);
    }

    /// Standing fine: a moment to save a checkpoint.
    pub fn calm(&self) -> bool {
        self.floating == 0 && self.inside == 0
    }

    /// Has he stood on this place yet?
    pub fn seen(&self, map: usize, x: i32, y: i32) -> bool {
        self.cells.contains(&(map, x.div_euclid(CELL), y.div_euclid(CELL)))
    }

    /// The coverage, as one line: "maps .. places .. scripts ..".
    pub fn coverage(&self) -> String {
        let mut maps: Vec<usize> = self.maps.iter().copied().collect();
        maps.sort_unstable();
        format!("maps {} {maps:?}, places {}, script classes {}", maps.len(), self.cells.len(), self.classes.len())
    }
}

struct Bot {
    rng: Rng,
    plan: Plan,
    watch: Watch,
    /// How many things it used (clicks that hit something usable).
    uses: u32,
}

impl Watch {
    /// Reports a breach (once per kind and place).
    pub fn report(&mut self, game: &mut Game, kind: &str, detail: String) {
        let av = game.avatar();
        let here = (av.x, av.y, av.z);
        let same_kind = self.found.iter().filter(|(k, _)| k == kind).count();
        if same_kind >= PER_KIND || self.found.iter().any(|(k, p)| k == kind && (p.0 - here.0).abs() < SAME_PLACE && (p.1 - here.1).abs() < SAME_PLACE) {
            return;
        }
        self.found.push((kind.to_string(), here));
        let slot = 900 + self.saves;
        self.saves += 1;
        let saved = game.save_to(slot).map(|_| format!("slot {slot}")).unwrap_or_else(|e| format!("not saved: {e}"));
        self.last = format!("BUG {kind} | map {} at {},{},{} | tick {} | {detail} | {saved} | bot step {}", game.cur_map, av.x, av.y, av.z, game.tick, self.step);
        println!("{}", self.last);
    }

    /// The rules, checked after every tick.
    pub fn check(&mut self, game: &mut Game) {
        // what of the game he got to
        let a = game.avatar();
        self.maps.insert(game.cur_map);
        self.cells.insert((game.cur_map, a.x.div_euclid(CELL), a.y.div_euclid(CELL)));
        self.classes.extend(game.scripts.procs.iter().filter(|p| !p.terminated && p.class != 0xFFFF).map(|p| p.class));
        let av = game.avatar();
        let Some(info) = game.data.infos.get(av.shape as usize).copied() else { return };
        let (xd, yd, zd) = info.footpad(av.flags & crate::map::FLG_FLIPPED != 0);
        let here = WorldBox { x: av.x, y: av.y, z: av.z, xd, yd, zd };
        let pi = game.map().position_info(&here, &here, &game.data.infos, AVATAR);
        let falling = game.mover(AVATAR).falling();
        let free = game.player_in_control() && !game.state(AVATAR).dead;

        // standing in the air without falling
        // (as the game itself decides: only if there is room one unit lower;
        // in a niche at the back of a wall he stands fine)
        let room_below = game.map().position_info(&WorldBox { z: av.z - 1, ..here }, &here, &game.data.infos, AVATAR).valid;
        // (pulling himself up a ledge takes longer than the limit; a
        // stagger over a stair step plays to its end before he steps down)
        let anim = game.mover(AVATAR).shown().0;
        let climbing = (crate::anim::CLIMB_FIRST..=crate::anim::CLIMB_LAST).contains(&anim);
        let stair = anim == crate::anim::STUMBLE_BACKWARDS && pi.floor_z.is_some_and(|f| av.z - f <= 16);
        let in_air = av.z > 0 && !pi.supported && pi.floor_z.is_none_or(|f| av.z > f) && room_below && !falling && !climbing && !stair;
        self.floating = if in_air && free { self.floating + 1 } else { 0 };
        if self.floating == FLOAT_TICKS {
            self.report(game, "avatar-floats", format!("no ground under him for {FLOAT_TICKS} ticks, anim {anim} frame {}", av.frame));
        }

        // inside something solid
        self.inside = if !pi.valid && free { self.inside + 1 } else { 0 };
        if self.inside == FLOAT_TICKS {
            let what = pi.blocker.and_then(|b| game.map().get(b).copied()).map_or("?".to_string(), |b| format!("item {} shape {}", b.id, b.shape));
            self.report(game, "avatar-inside-solid", format!("stuck in {what}"));
        }

        // a scene that never lets go
        let waiting_for_player = game.ask.is_some() || game.ui.reader.is_some() || game.ui.slider.is_some() || game.ui.targeting.is_some();
        self.held = if !game.player_in_control() && !waiting_for_player && !game.state(AVATAR).dead { self.held + 1 } else { 0 };
        if self.held == STUCK_TICKS {
            let procs: Vec<String> = game.scripts.procs.iter().filter(|p| !p.terminated).take(8).map(|p| format!("{:04X}:{:04X} item {}", p.class, p.ip, p.item)).collect();
            self.report(game, "scene-stuck", format!("no control for {STUCK_TICKS} ticks, stasis {}, processes {}", game.stasis, procs.join(", ")));
        }

        // values out of range
        let hp = game.state(AVATAR).hp;
        let max = game.max_hp(AVATAR);
        if hp < 0 || hp > max.max(1) {
            self.report(game, "avatar-hp", format!("hp {hp} of {max}"));
        }
        if av.z < 0 || av.z > 255 {
            self.report(game, "avatar-height", format!("z {}", av.z));
        }
    }

}

impl Bot {
    /// Picks what to do next, or answers what the game asks.
    fn act(&mut self, game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions) -> Option<MoveInput> {
        if let Some((_, choices)) = &game.ask {
            let n = choices.len() as u32;
            if self.rng.below(20) == 0 {
                let pick = self.rng.below(n) as usize;
                game.answer(pick);
            }
            return None;
        }
        if game.ui.reader.is_some() {
            if self.rng.below(30) == 0 {
                game.close_reader();
            }
            return None;
        }
        if game.ui.slider.is_some() {
            game.slider_confirm();
            return None;
        }
        if game.ui.targeting.is_some() {
            game.target_picked(0);
            return None;
        }
        if !game.player_in_control() {
            if self.rng.below(60) == 0 {
                game.skip_barks();
            }
            return None;
        }
        match &mut self.plan {
            Plan::Walk { dir, run, left } if *left > 0 => {
                *left -= 1;
                return Some(MoveInput { dir: *dir, run: *run });
            }
            Plan::Wait(left) if *left > 0 => {
                *left -= 1;
                return None;
            }
            _ => {}
        }
        // something new
        match self.rng.below(100) {
            0..=54 => {
                let dir = self.rng.below(8) as u8;
                let run = self.rng.below(3) == 0;
                self.plan = Plan::Walk { dir, run, left: 10 + self.rng.below(80) };
            }
            55..=64 => {
                let (w, h) = game.view_size;
                let p = (self.rng.below(w.max(1) as u32) as i32, self.rng.below(h.max(1) as u32) as i32);
                game.jump(Some(p), w, h);
                self.plan = Plan::Wait(20);
            }
            65..=84 => {
                // use something on screen, as a player does with the mouse:
                // only what is drawn on top at the point clicked
                game.draw(surf, opts, false);
                let (w, h) = game.view_size;
                for _ in 0..80 {
                    let (x, y) = (self.rng.below(w.max(1) as u32) as i32, self.rng.below(h.max(1) as u32) as i32);
                    let Some(id) = game.pick(x, y).filter(|&id| id != AVATAR && game.usable(id)) else { continue };
                    if game.options.modern_controls {
                        game.use_or_approach(id);
                    } else {
                        game.interact(id, true);
                    }
                    self.uses += 1;
                    self.plan = Plan::Wait(60);
                    break;
                }
            }
            85..=92 => {
                if game.fighting() || self.rng.below(2) == 0 {
                    game.attack_nearest();
                }
                self.plan = Plan::Wait(20);
            }
            93..=95 => {
                game.toggle_combat();
                self.plan = Plan::Wait(10);
            }
            _ => self.plan = Plan::Wait(self.rng.below(60)),
        }
        None
    }
}

/// Going back after a death, as a player loads his last savegame: the bot
/// saves now and then where he stands fine and loads it when he died.
pub(super) struct Checkpoint {
    saved: bool,
    pub deaths: u32,
}

impl Checkpoint {
    pub fn new(game: &Game) -> Checkpoint {
        Checkpoint { saved: game.save_to(CHECKPOINT).is_ok(), deaths: 0 }
    }

    /// After a tick: saves now and then; after a death says who was
    /// fighting him and goes back. False when it is over (no savegame,
    /// or too many deaths).
    pub fn after_tick(&mut self, game: &mut Game, watch: &mut Watch, n: u32) -> bool {
        if game.state(AVATAR).dead {
            let av = game.avatar();
            let close: Vec<crate::map::Item> = game.map().items_near(av.x, av.y, 1024).into_iter().map(|i| game.map().items[i]).filter(|t| t.is_actor() && t.id != AVATAR && !t.dead).collect();
            let fighting: Vec<String> = close.iter().filter(|t| game.state(t.id).in_combat).map(|t| format!("{} (shape {})", t.id, t.shape)).collect();
            println!("EVENT avatar-died | map {} at {},{},{} | tick {} | fighting near: {}", game.cur_map, av.x, av.y, av.z, game.tick, if fighting.is_empty() { "nobody".to_string() } else { fighting.join(", ") });
            self.deaths += 1;
            if !self.saved || self.deaths > 20 || game.load_from(CHECKPOINT).is_err() {
                return false;
            }
            watch.restart();
            return true;
        }
        if n % 3000 == 2999 && game.player_in_control() && watch.calm() {
            self.saved = game.save_to(CHECKPOINT).is_ok();
        }
        true
    }
}

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    if name != "bot" {
        return Ok(Flow::Unknown);
    }
    let (seed, ticks) = point(arg, cmd)?;
    let mut bot = Bot { rng: Rng::new(seed), plan: Plan::Wait(0), watch: Watch::new(), uses: 0 };
    let mut checkpoint = Checkpoint::new(game);
    for n in 0..ticks as u32 {
        bot.watch.step = n;
        let input = bot.act(game, surf, opts);
        game.step(input);
        bot.watch.check(game);
        let dead = game.state(AVATAR).dead;
        if !checkpoint.after_tick(game, &mut bot.watch, n) {
            break;
        }
        if dead {
            // play differently from there
            bot.rng.next();
            bot.plan = Plan::Wait(0);
        }
    }
    println!("bot: seed {seed}, {} findings, {} deaths, {} things used", bot.watch.found.len(), checkpoint.deaths, bot.uses);
    println!("coverage: {}", bot.watch.coverage());
    Ok(Flow::Done)
}
