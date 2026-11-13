//! The exploring bot: `explore:<seed>,<ticks>` plays as someone new to the
//! game does, not at random. He looks at what is on the screen and goes to
//! what he has not tried yet (people, doors, chests, levers, bodies), talks
//! to people and asks them everything once, takes what lies in containers,
//! reads what he opens, fights who attacks him and runs away when he is
//! nearly dead, drinks a healing potion when he is hurt, leaves alone what
//! hurt him before, and when nothing on the screen is new he walks to
//! ground he has not stood on. Like the test bot (`bot`) it only does what a
//! player can (clicks on what is drawn on the screen, keys) and checks the
//! same rules after every tick (see `bot::Watch`); a breach is printed as
//! "BUG ...". At the end it says how much of the game it got to
//! ("coverage: ..."), to compare the two bots by. The same seed plays the
//! same way. With PAGAN_BOT_LOG it writes a protocol of the game it played
//! (see botlog.rs).

use super::bot::{Checkpoint, Rng, Watch};
use super::botlog::{BotLog, V};
use super::journey::enemies;
use super::*;
use crate::actor::MoveInput;
use crate::game::AVATAR;
use crate::hash::HashSet;

/// How far apart the clicks are when he looks over the screen for things.
const LOOK_STEP: i32 = 10;
/// Ticks a walk to something may take before he gives up on it.
const GOAL_TICKS: u32 = 1500;
/// Ticks without moving on the way before he gives up on it.
const STILL_TICKS: u32 = 240;
/// Tries to get to a thing before he leaves it alone (one he used he leaves
/// alone at once).
const TRIES: u8 = 2;
/// A fight that is not over after this long gets nowhere: he goes away
/// and leaves the enemies near alone for as long.
const FIGHT_TICKS: u32 = 900;
/// Ticks he takes to read a text, a question's answers.
const READ_TICKS: u32 = 40;
/// Below this share of his hit points (in percent) he runs from a fight...
const FLEE_HP: i32 = 25;
/// ...and below this one, when nobody is near, he drinks a healing potion
/// (red when badly hurt, else yellow); without one he pauses a while, but
/// not again soon: hit points come back slowly (one a minute), and a
/// player does not wait for that, he goes on carefully.
const HEAL_HP: i32 = 50;
const PAUSE_TICKS: u32 = 900;
const PAUSE_AGAIN: u32 = 6000;
/// Answers he leaves out: buying costs money the rest of the game needs.
const SKIP: [&str; 4] = ["buy", "pay", "kaufen", "zahle"];
/// The size of a dead end (a place no click-walk got to).
const DEAD_END: i32 = 128;
/// After this many clicks on the ground in a row that found no way he
/// clicks close by; after twice as many he walks with the keys.
const WALK_FAILS: u32 = 3;
/// How often (ticks) the protocol says how he is doing.
const STATE_TICKS: u32 = 1000;
/// At most this many changes of the world per use in the protocol.
const MAX_CHANGES: usize = 25;

/// What he is doing.
enum Goal {
    /// Nothing: look for something.
    None,
    /// Clicked a thing: he walks up to it and uses it (hit points before,
    /// and when he got to it: what hurts him on the way, a fall, is not
    /// the thing's doing).
    Use { id: u16, shape: u32, since: u32, hp: i32, at_hand: Option<i32> },
    /// Clicked on the ground: he walks there (from where, to where).
    Walk { since: u32, from: (i32, i32, i32), to: (i32, i32) },
    /// Fights (since when).
    Fight { since: u32 },
    /// Runs away from a fight (direction, ticks left).
    Flee { dir: u8, left: u32 },
    /// Lets time pass (until this tick).
    Rest { until: u32 },
    /// Reads what is open, or thinks over the answers (ticks left).
    Read { left: u32 },
}

impl Goal {
    fn name(&self) -> String {
        match self {
            Goal::None => "look".to_string(),
            Goal::Use { id, .. } => format!("use {id}"),
            Goal::Walk { .. } => "walk".to_string(),
            Goal::Flee { dir, .. } => format!("keys {dir}"),
            Goal::Rest { .. } => "rest".to_string(),
            Goal::Fight { .. } => "fight".to_string(),
            Goal::Read { .. } => "read".to_string(),
        }
    }
}

struct Explorer {
    rng: Rng,
    goal: Goal,
    watch: Watch,
    log: BotLog,
    /// The world before the thing in use was clicked (only for the protocol).
    before: Option<Snapshot>,
    /// Things tried, and how often (map, item).
    tried: HashMap<(usize, u16), u8>,
    /// Kinds of things that hurt him when he used them (a poisonous
    /// mushroom): he leaves them alone after that, as a player learns.
    harmful: HashSet<u32>,
    /// Answers given (person, answer).
    answered: HashSet<(u16, String)>,
    /// The person he clicked on last, whom he talks to (a question names
    /// only the script that waits for the answer); 0 for nobody.
    partner: u16,
    /// Places where a click on the ground found no way (map, cell).
    dead_ends: HashSet<(usize, i32, i32)>,
    /// Clicks on the ground in a row that found no way.
    walk_fails: u32,
    /// Where he was when the walk last moved on, and when.
    last_pos: ((i32, i32, i32), u32),
    quiet: u32,
    // what he did, for the end
    uses: u32,
    talks: u32,
    answers: u32,
    taken: u32,
    unreachable: u32,
    fights: u32,
    flights: u32,
    rests: u32,
    potions: u32,
    /// Until this tick he leaves the enemies near alone.
    ignore_until: u32,
    /// Not before this tick does he see to his wounds again.
    next_heal: u32,
}

/// One tick: the view follows him as in a window (what is clicked is what
/// is drawn there), then the game goes on with the keys held.
fn step(game: &mut Game, input: Option<MoveInput>, n: u32) {
    if input.is_none() {
        super::journey::tick(game, n);
        return;
    }
    game.place_view(game.camera_position(), game.view.size);
    game.step(input);
}

/// The fighter nearest to him among those who fight.
fn nearest_enemy(game: &mut Game) -> Option<(i32, i32)> {
    let av = game.avatar();
    let near: Vec<crate::map::Item> = game.map().items_near(av.x, av.y, 384).into_iter().map(|i| game.map().items[i])
        .filter(|t| t.id != AVATAR && t.is_actor() && !t.dead).collect();
    near.into_iter().filter(|t| game.state(t.id).in_combat && !game.state(t.id).dead)
        .min_by_key(|t| (t.x - av.x).abs() + (t.y - av.y).abs()).map(|t| (t.x, t.y))
}

/// A person's name, if the game has one for them.
fn name_of(id: u16) -> &'static str {
    crate::places::PEOPLE.iter().find(|p| p.0 == id).map_or("", |p| p.1)
}

/// The changes of the world worth reading: not the people walking about
/// (nor the Avatar himself).
fn worth_reading(change: &str) -> bool {
    let Some(rest) = change.strip_prefix("item ") else { return true };
    let id: u32 = rest.split(|c: char| !c.is_ascii_digit()).next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let only_moved = rest.split_once(": ").is_some_and(|(_, what)| what.starts_with("at ") && !what.contains(", "));
    !(id < 256 && only_moved)
}

impl Explorer {
    fn percent_hp(game: &mut Game) -> i32 {
        let hp = game.state(AVATAR).hp;
        hp * 100 / game.max_hp(AVATAR).max(1)
    }

    /// Something worth looking at later: printed as "EVENT ..." and written
    /// to the protocol.
    fn event(&mut self, game: &mut Game, kind: &str, detail: String) {
        let av = game.avatar();
        println!("EVENT {kind} | map {} at {},{},{} | tick {} | {detail}", game.cur_map, av.x, av.y, av.z, game.tick);
        self.log.write(game, kind, &[("detail", V::S(&detail))]);
    }

    /// What can be clicked on the screen: the things drawn on top, looked
    /// over in a grid as the eye goes over the picture.
    fn on_screen(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions) -> Vec<u16> {
        game.draw(surf, opts, false);
        let (w, h) = game.view.size;
        let mut seen: Vec<u16> = Vec::new();
        for y in (LOOK_STEP / 2..h).step_by(LOOK_STEP as usize) {
            for x in (LOOK_STEP / 2..w).step_by(LOOK_STEP as usize) {
                if let Some(id) = game.pick(x, y) {
                    if id != AVATAR && !seen.contains(&id) {
                        seen.push(id);
                    }
                }
            }
        }
        seen
    }

    /// Picks an answer: one not given to this person yet (not buying), else
    /// the last one, which ends the talk.
    fn answer(&mut self, game: &mut Game) {
        let Some((_, choices)) = game.ask.clone() else { return };
        let who = self.partner;
        if choices.is_empty() {
            return;
        }
        let fresh = choices[..choices.len() - 1].iter().position(|c| {
            !self.answered.contains(&(who, c.clone())) && !SKIP.iter().any(|s| c.to_lowercase().contains(s))
        });
        let k = fresh.unwrap_or(choices.len() - 1);
        let new = self.answered.insert((who, choices[k].clone()));
        if new {
            self.answers += 1;
        }
        let why = if fresh.is_some() { "not asked yet" } else { "all asked: the last answer" };
        let options: Vec<String> = choices.iter().map(|c| c.trim().to_string()).collect();
        self.log.write(game, "answer", &[("who", V::N(who as i64)), ("name", V::S(name_of(who))), ("options", V::L(options)), ("picked", V::S(choices[k].trim())), ("why", V::S(why))]);
        game.answer(k);
    }

    /// Takes what lies in the open containers into the backpack, then closes them.
    fn loot(&mut self, game: &mut Game) {
        let owners: Vec<u16> = game.ui.gumps.iter().map(|g| g.owner).filter(|&o| o != AVATAR).collect();
        for owner in owners {
            // (not the backpack, nor what lies in it)
            let mine = game.map().get(owner).is_some_and(|b| !b.in_world());
            let mut got = Vec::new();
            let mut left = Vec::new();
            if !mine {
                let inside: Vec<(u16, u32, u32)> = game.map().items.iter().filter(|it| !it.gone && it.parent == owner).map(|it| (it.id, it.shape, it.frame)).collect();
                for (id, shape, frame) in inside {
                    if game.take_into_backpack(id) == Some(true) {
                        self.taken += 1;
                        got.push(format!("{shape}/{frame}"));
                    } else {
                        left.push(format!("{shape}/{frame}"));
                    }
                }
                let shape = game.map().get(owner).map_or(0, |it| it.shape);
                self.log.write(game, "loot", &[("container", V::N(owner as i64)), ("shape", V::N(shape as i64)), ("taken", V::L(got)), ("left", V::L(left))]);
            }
            game.close_gump(owner);
        }
    }

    /// Something new to do, from what is on the screen; else a walk to
    /// ground he has not stood on.
    fn choose(&mut self, game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, n: u32) {
        let av = game.avatar();
        let map = game.cur_map;
        let mut best: Option<(i32, u16)> = None;
        let mut candidates = 0;
        for id in Self::on_screen(game, surf, opts) {
            if self.tried.get(&(map, id)).copied().unwrap_or(0) >= TRIES || !game.usable(id) {
                continue;
            }
            let Some(it) = game.map().get(id).copied() else { continue };
            if !it.in_world() || self.harmful.contains(&it.shape) {
                continue;
            }
            candidates += 1;
            // the nearest first, people before things, with a little chance
            let mut score = (it.x - av.x).abs() + (it.y - av.y).abs() + self.rng.below(128) as i32;
            if it.is_actor() && !it.dead {
                score -= 256;
            }
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, id));
            }
        }
        if let Some((_, id)) = best {
            let it = game.map().get(id).copied().unwrap_or_default();
            let tries = self.tried.entry((map, id)).or_insert(0);
            *tries += 1;
            let tries = *tries;
            let person = it.is_actor() && !it.dead;
            if person {
                self.talks += 1;
            }
            self.partner = if person { id } else { 0 };
            self.uses += 1;
            if self.log.on() {
                self.before = Some(Snapshot::take(game));
                let why = if person { "a person not talked to" } else if tries > 1 { "tried again" } else { "new on the screen" };
                self.log.write(game, "goal", &[("do", V::S("use")), ("id", V::N(id as i64)), ("shape", V::N(it.shape as i64)), ("frame", V::N(it.frame as i64)),
                    ("name", V::S(name_of(id))), ("to", V::L(vec![it.x.to_string(), it.y.to_string(), it.z.to_string()])), ("why", V::S(why)), ("choices", V::N(candidates))]);
            }
            game.use_or_approach(id);
            self.goal = Goal::Use { id, shape: it.shape, since: n, hp: game.state(AVATAR).hp, at_hand: None };
            self.last_pos = ((av.x, av.y, av.z), n);
            return;
        }
        // nothing new on the screen: a click on ground not stood on yet
        // (the farther, the better: he wants to see what is beyond)
        let (w, h) = game.view.size;
        // (after clicks that found no way: close by, as a player tries)
        if self.walk_fails >= 2 * WALK_FAILS {
            let dir = self.rng.below(8) as u8;
            self.walk_fails = 0;
            self.event(game, "walk-stuck", format!("{} clicks on the ground found no way; he walks with the keys, direction {dir}", 2 * WALK_FAILS));
            self.goal = Goal::Flee { dir, left: 45 };
            return;
        }
        let close = self.walk_fails >= WALK_FAILS;
        let mut spot: Option<(i32, (i32, i32), (i32, i32, i32), bool)> = None;
        for _ in 0..40 {
            let p = (self.rng.below(w.max(1) as u32) as i32, self.rng.below(h.max(1) as u32) as i32);
            let Some(((x, y, z), _)) = game.ground_spot(p.0, p.1) else { continue };
            if self.dead_ends.contains(&(map, x.div_euclid(DEAD_END), y.div_euclid(DEAD_END))) {
                continue;
            }
            let far = (x - av.x).abs() + (y - av.y).abs();
            let seen = self.watch.seen(map, x, y);
            let score = if close { -far } else if seen { far / 4 } else { far + 1024 };
            if spot.is_none_or(|s| score > s.0) {
                spot = Some((score, p, (x, y, z), seen));
            }
        }
        if let Some((_, p, to, seen)) = spot {
            if game.walk_to_pointer(p.0, p.1) {
                let why = if close { "the last clicks found no way: close by" } else if seen { "nothing new near: somewhere far" } else { "ground not stood on" };
                self.log.write(game, "goal", &[("do", V::S("walk")), ("to", V::L(vec![to.0.to_string(), to.1.to_string(), to.2.to_string()])), ("why", V::S(why))]);
                self.goal = Goal::Walk { since: n, from: (av.x, av.y, av.z), to: (to.0, to.1) };
                self.last_pos = ((av.x, av.y, av.z), n);
                return;
            }
        }
        // no ground to click on (a narrow ledge): a few steps with the keys
        let dir = self.rng.below(8) as u8;
        self.log.write(game, "goal", &[("do", V::S("keys")), ("dir", V::N(dir as i64)), ("why", V::S("no ground to click on"))]);
        self.goal = Goal::Flee { dir, left: 30 };
    }

    /// The use clicked last is over: what it did.
    fn used(&mut self, game: &mut Game, n: u32, reached: bool) {
        let Goal::Use { id, shape, since, hp, at_hand } = self.goal else { return };
        if reached {
            // done with it: a second try is for what he did not get to
            self.tried.insert((game.cur_map, id), TRIES);
        }
        let now = game.state(AVATAR).hp;
        // it hurt him, and nobody struck him: not that again
        // (what he used may be gone: eaten)
        let there = at_hand.unwrap_or(hp);
        if reached && now < there && self.quiet > n - since && self.harmful.insert(shape) {
            self.event(game, "harmful", format!("item {id} shape {shape}: hp {there} -> {now}, left alone from now on"));
        }
        if let Some(before) = self.before.take() {
            let after = Snapshot::take(game);
            let mut changes: Vec<String> = before.diff(&after, &game.scripts.usecode).into_iter().filter(|c| worth_reading(c)).collect();
            let more = changes.len().saturating_sub(MAX_CHANGES);
            changes.truncate(MAX_CHANGES);
            if more > 0 {
                changes.push(format!("... and {more} more"));
            }
            self.log.write(game, "done", &[("id", V::N(id as i64)), ("shape", V::N(shape as i64)), ("reached", V::N(reached as i64)), ("ticks", V::N((n - since) as i64)), ("hp_before", V::N(hp as i64)), ("changes", V::L(changes))]);
        }
    }

    /// The keys held this tick, or what was clicked.
    fn act(&mut self, game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, n: u32) -> Option<MoveInput> {
        // what the game asks first: he reads, then answers
        if game.ask.is_some() || game.ui.reader.is_some() {
            match &mut self.goal {
                Goal::Read { left } if *left > 0 => {
                    *left -= 1;
                    return None;
                }
                Goal::Read { .. } => {
                    self.goal = Goal::None;
                    if game.ask.is_some() {
                        self.answer(game);
                    } else {
                        let look = game.ui.reader.as_ref().map_or("", |r| match r.look {
                            crate::gumps::Look::Panel => "panel",
                            crate::gumps::Look::Book => "book",
                            crate::gumps::Look::Scroll => "scroll",
                            crate::gumps::Look::Sign { .. } => "sign",
                        });
                        self.log.write(game, "read", &[("what", V::S(look))]);
                        game.close_reader();
                    }
                    return None;
                }
                _ => {
                    // (a use that opened it is over)
                    self.used(game, n, true);
                    self.goal = Goal::Read { left: READ_TICKS };
                    return None;
                }
            }
        }
        if game.ui.slider.is_some() {
            game.slider_confirm();
            return None;
        }
        if game.ui.targeting.is_some() {
            game.target_picked(0);
            return None;
        }
        // (a walk he was sent on is his own: he may give it up)
        if !game.player_may_order() {
            return None;
        }
        if game.ui.gumps.iter().any(|g| g.owner != AVATAR) {
            self.loot(game);
        }

        // a fight: he runs when nearly dead, else he fights back; a fight
        // that gets nowhere (an enemy he cannot reach) he leaves for a while
        let foes = enemies(game);
        if foes > 0 && n >= self.ignore_until {
            self.quiet = 0;
            if let Goal::Fight { since } = self.goal {
                if n - since > FIGHT_TICKS {
                    let av = game.avatar();
                    let at = nearest_enemy(game);
                    self.event(game, "fight-stuck", format!("no end after {FIGHT_TICKS} ticks, nearest fighter at {at:?}; he goes away"));
                    if game.fighting() {
                        game.toggle_combat();
                    }
                    self.ignore_until = n + FIGHT_TICKS;
                    let dir = at.map_or(self.rng.below(8) as u8, |(ex, ey)| crate::intrinsics::world_dir(av.x - ex, av.y - ey) as u8);
                    self.goal = Goal::Flee { dir, left: 90 };
                    return None;
                }
            }
            if let Goal::Flee { dir, left } = &mut self.goal {
                if *left > 0 {
                    *left -= 1;
                    return Some(MoveInput { dir: *dir, run: true });
                }
            }
            let hp = Self::percent_hp(game);
            if hp < FLEE_HP {
                if let Some((ex, ey)) = nearest_enemy(game) {
                    let av = game.avatar();
                    let dir = crate::intrinsics::world_dir(av.x - ex, av.y - ey) as u8;
                    if game.fighting() {
                        game.toggle_combat();
                    }
                    self.flights += 1;
                    self.log.write(game, "flee", &[("dir", V::N(dir as i64)), ("hp_percent", V::N(hp as i64)), ("enemies", V::N(foes as i64))]);
                    self.goal = Goal::Flee { dir, left: 60 };
                    return Some(MoveInput { dir, run: true });
                }
            }
            if !game.fighting() {
                game.cancel_approach();
                game.toggle_combat();
            }
            if !matches!(self.goal, Goal::Fight { .. }) {
                // (what he was on his way to is given up)
                self.used(game, n, false);
                self.fights += 1;
                self.log.write(game, "fight", &[("enemies", V::N(foes as i64)), ("hp_percent", V::N(hp as i64))]);
                self.goal = Goal::Fight { since: n };
            }
            if n.is_multiple_of(10) {
                game.attack_nearest();
            }
            return None;
        }
        self.quiet += 1;
        if let Goal::Fight { since } = self.goal {
            if self.quiet > 60 {
                if game.fighting() {
                    game.toggle_combat();
                }
                let hp = Self::percent_hp(game);
                self.log.write(game, "fight-over", &[("ticks", V::N((n - since) as i64)), ("hp_percent", V::N(hp as i64))]);
                self.goal = Goal::None;
            }
            return None;
        }

        let av = game.avatar();
        match &mut self.goal {
            Goal::Flee { dir, left } if *left > 0 => {
                *left -= 1;
                return Some(MoveInput { dir: *dir, run: true });
            }
            Goal::Rest { until } => {
                if n >= *until {
                    self.goal = Goal::None;
                }
                return None;
            }
            Goal::Use { since, .. } | Goal::Walk { since, .. } => {
                let since = *since;
                let id = if let Goal::Use { id, .. } = self.goal { Some(id) } else { None };
                let busy = game.walking() || game.orders.approaching.is_some() || game.mover(AVATAR).busy();
                // (his hit points on the way, the last ones before he uses it)
                if let Goal::Use { at_hand, .. } = &mut self.goal {
                    if game.walking() || game.orders.approaching.is_some() {
                        *at_hand = Some(game.state(AVATAR).hp);
                    }
                }
                if n - since > 10 && !busy {
                    self.used(game, n, true);
                    if let Goal::Walk { from, to, .. } = self.goal {
                        let a = game.avatar();
                        if (a.x - from.0).abs() + (a.y - from.1).abs() < 32 {
                            // the click found no way there
                            self.walk_fails += 1;
                            self.dead_ends.insert((game.cur_map, to.0.div_euclid(DEAD_END), to.1.div_euclid(DEAD_END)));
                            self.log.write(game, "walk-failed", &[("to", V::L(vec![to.0.to_string(), to.1.to_string()])), ("in_a_row", V::N(self.walk_fails as i64))]);
                        } else {
                            self.walk_fails = 0;
                        }
                    }
                    self.goal = Goal::None;
                } else {
                    let here = (av.x, av.y, av.z);
                    if here != self.last_pos.0 {
                        self.last_pos = (here, n);
                    }
                    if n - since > GOAL_TICKS || n - self.last_pos.1 > STILL_TICKS {
                        // seen, but he does not get there
                        let why = if n - since > GOAL_TICKS { format!("not there after {GOAL_TICKS} ticks") } else { format!("not moved for {STILL_TICKS} ticks") };
                        match id {
                            Some(id) => {
                                self.unreachable += 1;
                                let shape = game.map().get(id).map_or(0, |it| it.shape);
                                self.event(game, "not-reached", format!("item {id} shape {shape}: {why}"));
                                self.used(game, n, false);
                            }
                            None => self.log.write(game, "walk-given-up", &[("why", V::S(&why))]),
                        }
                        game.cancel_approach();
                        self.goal = Goal::None;
                    }
                    return None;
                }
            }
            _ => {}
        }
        // hurt, nobody near: a healing potion (quick key), else a pause
        let hp = Self::percent_hp(game);
        if hp < HEAL_HP && n >= self.next_heal {
            // (quick keys: 0 yellow, a little health; 1 red, all of it)
            let order = if hp < FLEE_HP { [1, 0] } else { [0, 1] };
            let drunk = order.into_iter().any(|k| game.quick_potion(k));
            if drunk {
                self.potions += 1;
                self.next_heal = n + 120;
                self.goal = Goal::Rest { until: n + 60 };
            } else {
                self.rests += 1;
                self.next_heal = n + PAUSE_AGAIN;
                self.goal = Goal::Rest { until: n + PAUSE_TICKS };
            }
            self.log.write(game, "heal", &[("how", V::S(if drunk { "potion" } else { "pause, no potion" })), ("hp_percent", V::N(hp as i64))]);
            return None;
        }
        self.choose(game, surf, opts, n);
        None
    }

    /// How he is doing, now and then in the protocol.
    fn state(&mut self, game: &mut Game, deaths: u32) {
        let max = game.max_hp(AVATAR);
        let s = game.state(AVATAR);
        let goal = self.goal.name();
        let (control, stasis, fighting) = (game.player_in_control(), game.stasis, game.fighting());
        let procs: Vec<String> = if control {
            Vec::new()
        } else {
            // who holds him: the scripts at work
            game.scripts.procs.iter().filter(|p| !p.terminated && !p.suspended && p.class != 0xFFFF).take(6).map(|p| format!("{:04X}:{:04X} on {}", p.class, p.ip, p.item)).collect()
        };
        let coverage = self.watch.coverage();
        self.log.write(game, "state", &[("max_hp", V::N(max as i64)), ("mana", V::N(game.mana as i64)), ("goal", V::S(&goal)), ("control", V::N(control as i64)),
            ("stasis", V::N(stasis as i64)), ("fighting", V::N(fighting as i64)), ("dead", V::N(s.dead as i64)), ("scripts", V::L(procs)),
            ("uses", V::N(self.uses as i64)), ("answers", V::N(self.answers as i64)), ("deaths", V::N(deaths as i64)), ("coverage", V::S(&coverage))]);
    }
}

pub(super) fn run(game: &mut Game, surf: &mut Surface, opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    if name != "explore" {
        return Ok(Flow::Unknown);
    }
    let (seed, ticks) = point(arg, cmd)?;
    let mut ex = Explorer {
        rng: Rng::new(seed), goal: Goal::None, watch: Watch::new(), log: BotLog::open(), before: None, tried: HashMap::default(), harmful: HashSet::default(), answered: HashSet::default(), dead_ends: HashSet::default(), walk_fails: 0, partner: 0,
        last_pos: ((0, 0, 0), 0), quiet: 0, uses: 0, talks: 0, answers: 0, taken: 0, unreachable: 0, fights: 0, flights: 0, rests: 0, potions: 0, next_heal: 0, ignore_until: 0,
    };
    ex.log.write(game, "start", &[("seed", V::N(seed as i64)), ("ticks", V::N(ticks as i64))]);
    let mut checkpoint = Checkpoint::new(game);
    // PAGAN_EXPLORE=1: every new goal is printed
    let trace = crate::debug::env().explore;
    let mut last = String::new();
    for n in 0..ticks as u32 {
        ex.watch.step = n;
        ex.log.tick = n;
        let input = ex.act(game, surf, opts, n);
        if trace {
            let now = ex.goal.name();
            if now != last {
                let (a, hp) = (game.avatar(), Explorer::percent_hp(game));
                println!("explore {n}: {now} (at {},{},{}, hp {hp}%)", a.x, a.y, a.z);
                last = now;
            }
        }
        step(game, input, n);
        let found = ex.watch.found.len();
        ex.watch.check(game);
        if ex.watch.found.len() > found {
            let line = ex.watch.last.clone();
            ex.log.write(game, "bug", &[("report", V::S(&line))]);
        }
        ex.log.way(game);
        if n.is_multiple_of(STATE_TICKS) {
            ex.state(game, checkpoint.deaths);
        }
        let dead = game.state(AVATAR).dead;
        if dead {
            ex.log.write(game, "death", &[("goal", V::S(&ex.goal.name()))]);
        }
        if !checkpoint.after_tick(game, &mut ex.watch, n) {
            ex.log.write(game, "end", &[("why", V::S("dead, no savegame to go back to"))]);
            break;
        }
        if dead {
            // from the savegame he plays on differently
            ex.log.write(game, "reload", &[]);
            ex.rng.next();
            ex.before = None;
            ex.goal = Goal::None;
        }
    }
    let summary = format!("explore: seed {seed}, {} findings, {} deaths, {} things used ({} people), {} answers, {} things taken, {} not reached, {} fights, {} flights, {} rests, {} potions",
        ex.watch.found.len(), checkpoint.deaths, ex.uses, ex.talks, ex.answers, ex.taken, ex.unreachable, ex.fights, ex.flights, ex.rests, ex.potions);
    let coverage = ex.watch.coverage();
    ex.log.write(game, "summary", &[("text", V::S(&summary)), ("coverage", V::S(&coverage))]);
    ex.log.flush();
    println!("{summary}");
    println!("coverage: {coverage}");
    Ok(Flow::Done)
}
