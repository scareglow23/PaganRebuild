//! The protocol of a bot run (`explore`), for looking at a game it played
//! afterwards: which way it went, what it chose and why, what that did to
//! the world, how the Avatar was doing, and where something went wrong.
//! Written when PAGAN_BOT_LOG names a file, one event per line as JSON
//! (JSON Lines). Every line has the bot's tick `t`, the game's `tick`, the
//! kind of event `k`, the map, where the Avatar stands (`at`) and his hit
//! points; then what the event says. tools/bots/collect.py reads them.

use super::*;
use crate::game::AVATAR;
use std::io::Write;

/// A value in a protocol line.
pub(super) enum V<'a> {
    N(i64),
    S(&'a str),
    L(Vec<String>),
}

pub(super) struct BotLog {
    out: Option<std::io::BufWriter<std::fs::File>>,
    /// The bot's tick, for every line.
    pub tick: u32,
    /// Where he was at the last line of the way (map, x, y, z).
    last: (usize, i32, i32, i32),
}

/// How often (ticks) the way is written, when he has moved.
const WAY_TICKS: u32 = 30;

impl BotLog {
    /// The file named by PAGAN_BOT_LOG, or no protocol.
    pub fn open() -> BotLog {
        let out = std::env::var_os("PAGAN_BOT_LOG").and_then(|p| match std::fs::File::create(&p) {
            Ok(f) => Some(std::io::BufWriter::new(f)),
            Err(e) => {
                eprintln!("bot protocol {}: {e}", Path::new(&p).display());
                None
            }
        });
        BotLog { out, tick: 0, last: (usize::MAX, 0, 0, 0) }
    }

    pub fn on(&self) -> bool {
        self.out.is_some()
    }

    /// One event.
    pub fn write(&mut self, game: &mut Game, kind: &str, fields: &[(&str, V)]) {
        if self.out.is_none() {
            return;
        }
        let a = game.avatar();
        let hp = game.state(AVATAR).hp;
        let mut line = format!("{{\"t\":{},\"tick\":{},\"k\":{},\"map\":{},\"at\":[{},{},{}],\"hp\":{hp}", self.tick, game.tick, text(kind), game.cur_map, a.x, a.y, a.z);
        for (key, value) in fields {
            line += &format!(",{}:", text(key));
            match value {
                V::N(x) => line += &x.to_string(),
                V::S(s) => line += &text(s),
                V::L(l) => line += &format!("[{}]", l.iter().map(|s| text(s)).collect::<Vec<_>>().join(",")),
            }
        }
        line += "}\n";
        if let Some(out) = &mut self.out {
            let _ = out.write_all(line.as_bytes());
        }
    }

    /// The way he goes: where he is now and then (when he moved), and
    /// every change of map.
    pub fn way(&mut self, game: &mut Game) {
        if self.out.is_none() {
            return;
        }
        let a = game.avatar();
        let here = (game.cur_map, a.x, a.y, a.z);
        if here.0 != self.last.0 {
            let from = self.last.0;
            self.write(game, "map", &[("from", V::N(if from == usize::MAX { -1 } else { from as i64 }))]);
            self.last = here;
        } else if self.tick.is_multiple_of(WAY_TICKS) && here != self.last {
            self.write(game, "pos", &[]);
            self.last = here;
        }
    }

    pub fn flush(&mut self) {
        if let Some(out) = &mut self.out {
            let _ = out.flush();
        }
    }
}

/// A string as JSON.
fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
