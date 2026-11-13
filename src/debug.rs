//! The switches of the environment (`PAGAN_...`): what they are and what
//! each one does, read once when the program starts.
//!
//! They are for finding faults and for the tests, not for players: none
//! of them is needed to play. A switch is on when its variable is set,
//! whatever it is set to; the few that take a value say so.
//!
//! Not listed here, because the program itself does not read them:
//! `PAGAN_CASE` (tests/regress.rs: run one regression case) and
//! `PAGAN_ALBUM_WAV` / `PAGAN_ALBUM_OGG` (a test in album.rs that compares
//! the album's files in two folders).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct Env {
    // ---- what is printed
    /// `PAGAN_DEBUG`: fights (blows, hits, why an attack is refused, what
    /// each creature is after), creatures coming out of eggs, practice.
    pub debug: bool,
    /// `PAGAN_TRACE`: walking by click and climbing on the way, tick by
    /// tick; what the turned view draws.
    pub trace: bool,
    /// `PAGAN_CALLS`: every call and engine function of the scripts.
    pub calls: bool,
    /// `PAGAN_STEP=<id>`: every animation step of this actor.
    pub step: Option<u16>,
    /// `PAGAN_KEYS`: every key and typed character the window gets.
    pub keys: bool,
    /// `PAGAN_SOUNDINFO`: when the improved sound effects are ready.
    pub sound_info: bool,
    /// `PAGAN_MODELS`: how the turned view judges each picture it turns.
    pub models: bool,

    // ---- what is timed
    /// `PAGAN_PATHTIME`: how long each way search takes.
    pub path_time: bool,
    /// `PAGAN_TICKTIME`: every tick of the test command `t<n>` that takes
    /// longer than 5 ms.
    pub tick_time: bool,
    /// `PAGAN_FRAMETIME`: how long the parts of a picture take.
    pub frame_time: bool,
    /// `PAGAN_FPS`: pictures per second in the corner from the start (F2).
    pub fps: bool,

    // ---- window and sound
    /// `PAGAN_SOUND`: open the sound devices also without a window
    /// (`--screenshot`), for the sound test commands.
    pub sound: bool,
    /// `PAGAN_FULLSCREEN`: the window fills the screen, without a frame.
    pub fullscreen: bool,
    /// `PAGAN_NO_DWM`: do not pace the pictures by the desktop (Windows).
    pub no_dwm: bool,
    /// `PAGAN_LATENCY=<n>`: how many pictures the graphics card may hold
    /// back (1 if not set).
    pub latency: Option<u32>,

    // ---- where files go
    /// `PAGAN_SAVE_DIR=<folder>`: the savegames lie there instead of in
    /// the player's folder (the tests use a scratch folder).
    pub save_dir: Option<PathBuf>,
    /// `PAGAN_BOT_LOG=<file>`: the exploring bot writes the protocol of
    /// its game there.
    pub bot_log: Option<OsString>,

    // ---- the picture of `--screenshot`
    /// `PAGAN_XBR`: smoothed (xBR) at twice the size, as the option shows it.
    pub xbr: bool,
    /// `PAGAN_SHOWSHARP`: with `PAGAN_XBR`, the pixels the smoothing
    /// leaves sharp in magenta.
    pub show_sharp: bool,
    /// `PAGAN_ZOOM=<n>`: the middle of the picture, enlarged n times.
    pub zoom: Option<i32>,
    /// `PAGAN_HEADS`: in sheets of the Avatar's pictures, the head as it
    /// is found for the helmet, in magenta.
    pub heads: bool,

    // ---- test commands
    /// `PAGAN_BLINK`: the test command `blink` counts every change from
    /// one picture to the next, not only those that change back.
    pub blink: bool,
    /// `PAGAN_JOURNEY`: the playthrough prints every click (1 when set;
    /// 2 with `PAGAN_JOURNEY=2`: every tick of a walk as well).
    pub journey: u8,
    /// `PAGAN_EXPLORE`: the exploring bot prints every new goal.
    pub explore: bool,
}

/// The switches as they were set when the program started.
pub fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let on = |name: &str| std::env::var_os(name).is_some();
        let text = |name: &str| std::env::var(name).ok();
        Env {
            debug: on("PAGAN_DEBUG"),
            trace: on("PAGAN_TRACE"),
            calls: on("PAGAN_CALLS"),
            step: text("PAGAN_STEP").and_then(|v| v.parse().ok()),
            keys: on("PAGAN_KEYS"),
            sound_info: on("PAGAN_SOUNDINFO"),
            models: on("PAGAN_MODELS"),
            path_time: on("PAGAN_PATHTIME"),
            tick_time: on("PAGAN_TICKTIME"),
            frame_time: on("PAGAN_FRAMETIME"),
            fps: on("PAGAN_FPS"),
            sound: on("PAGAN_SOUND"),
            fullscreen: on("PAGAN_FULLSCREEN"),
            no_dwm: on("PAGAN_NO_DWM"),
            latency: text("PAGAN_LATENCY").and_then(|v| v.parse().ok()),
            save_dir: std::env::var_os("PAGAN_SAVE_DIR").map(PathBuf::from),
            bot_log: std::env::var_os("PAGAN_BOT_LOG"),
            xbr: on("PAGAN_XBR"),
            show_sharp: on("PAGAN_SHOWSHARP"),
            zoom: text("PAGAN_ZOOM").and_then(|v| v.parse().ok()),
            heads: on("PAGAN_HEADS"),
            blink: on("PAGAN_BLINK"),
            journey: match text("PAGAN_JOURNEY").as_deref() {
                Some("2") => 2,
                _ if on("PAGAN_JOURNEY") => 1,
                _ => 0,
            },
            explore: on("PAGAN_EXPLORE"),
        }
    })
}
