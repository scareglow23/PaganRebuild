//! The crash log: when the program fails (a panic), what failed and where
//! the game was at that moment is written to `crash.log` in the player's
//! folder. Without it a crash in the window leaves nothing behind to find
//! its cause with.
//!
//! No savegame is written: in the middle of a tick the world is half
//! done, and such a savegame would carry the damage on.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where the game is, noted at the start of every tick.
#[derive(Clone, Copy)]
pub struct Moment {
    pub map: usize,
    pub tick: u32,
    pub avatar: (i32, i32, i32),
    pub language: char,
}

static MOMENT: Mutex<Option<Moment>> = Mutex::new(None);

/// Notes where the game is (for the log of a crash later in this tick).
pub fn note(moment: Moment) {
    // (a crash while it was held leaves the lock poisoned: the value in
    // it is still good)
    *MOMENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(moment);
}

/// A log grown larger than this starts anew.
const MAX_LOG: u64 = 256 * 1024;

/// From now on a crash is written to `crash.log` in `dir` (and shown on
/// the console as before).
pub fn install(dir: PathBuf) {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        before(info);
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned());
        let at = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        // (never wait here: the failed thread may hold the lock itself)
        let moment = match MOMENT.try_lock() {
            Ok(m) => *m,
            Err(std::sync::TryLockError::Poisoned(e)) => **e.get_ref(),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let thread = std::thread::current();
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        let text = entry(seconds, thread.name(), what.as_deref(), at.as_deref(), moment, &trace);
        match append(&dir, &text) {
            Ok(path) => eprintln!("the crash was written to {}", path.display()),
            Err(err) => eprintln!("the crash log could not be written: {err}"),
        }
    }));
}

/// One entry of the log.
fn entry(seconds: u64, thread: Option<&str>, what: Option<&str>, at: Option<&str>, moment: Option<Moment>, trace: &str) -> String {
    let mut text = format!("---- PaganRebuild {} crashed, {} UTC\n", env!("CARGO_PKG_VERSION"), date_time(seconds));
    text += &format!("what:   {}\n", what.unwrap_or("(no message)"));
    text += &format!("where:  {}\n", at.unwrap_or("(unknown)"));
    text += &format!("thread: {}\n", thread.unwrap_or("(unnamed)"));
    text += &match moment {
        Some(m) => format!("game:   map {}, tick {}, Avatar at {},{},{}, language {}\n", m.map, m.tick, m.avatar.0, m.avatar.1, m.avatar.2, m.language),
        None => "game:   not started yet\n".to_string(),
    };
    text += "the way there:\n";
    for line in trace.lines() {
        text += &format!("  {}\n", line.trim_end());
    }
    text.push('\n');
    text
}

/// Adds an entry to the log in `dir`; returns the log's file.
fn append(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("crash.log");
    let anew = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG);
    let mut file = std::fs::OpenOptions::new().create(true).write(true).append(!anew).truncate(anew).open(&path)?;
    file.write_all(text.as_bytes())?;
    Ok(path)
}

/// Seconds since 1970 as date and time of day.
fn date_time(seconds: u64) -> String {
    let (days, rest) = ((seconds / 86400) as i64, seconds % 86400);
    // the calendar counted in spans of 400 years that begin on a 1st of March
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest / 60 % 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date_time(0), "1970-01-01 00:00:00");
        assert_eq!(date_time(951_782_399), "2000-02-28 23:59:59");
        assert_eq!(date_time(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(date_time(1_794_583_845), "2026-11-13 15:30:45");
    }

    #[test]
    fn entry_names_the_place() {
        let m = Moment { map: 47, tick: 1234, avatar: (100, 200, 8), language: 'G' };
        let text = entry(0, Some("main"), Some("index out of bounds"), Some("src/game.rs:504:9"), Some(m), "0: first\n1: second");
        assert!(text.contains("what:   index out of bounds"));
        assert!(text.contains("where:  src/game.rs:504:9"));
        assert!(text.contains("map 47, tick 1234, Avatar at 100,200,8, language G"));
        assert!(text.contains("  1: second"));
        assert!(entry(0, None, None, None, None, "").contains("not started yet"));
    }

    /// A failure anywhere in the program ends up in the log, with the
    /// place in the code and where the game was.
    #[test]
    fn a_crash_is_logged() {
        let dir = std::env::temp_dir().join(format!("pagan_crash_hook_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        note(Moment { map: 6, tick: 99, avatar: (1, 2, 3), language: 'E' });
        install(dir.clone());
        let failed = std::panic::catch_unwind(|| panic!("the test's own failure {}", 42));
        assert!(failed.is_err());
        let text = std::fs::read_to_string(dir.join("crash.log")).unwrap();
        assert!(text.contains("what:   the test's own failure 42"), "{text}");
        assert!(text.contains("where:  src"), "{text}");
        assert!(text.contains("map 6, tick 99, Avatar at 1,2,3"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Entries follow one another; a log grown too large starts anew.
    #[test]
    fn log_grows_and_starts_anew() {
        let dir = std::env::temp_dir().join(format!("pagan_crash_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = append(&dir, "one\n").unwrap();
        append(&dir, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        std::fs::write(&path, vec![b'x'; MAX_LOG as usize + 1]).unwrap();
        append(&dir, "three\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
