//! The pace of the pictures in the window, and how long they take: the
//! pacing itself (`Pacer`), the counter in the corner (F2, `Fps`) and the
//! times written out with PAGAN_FRAMETIME (`FrameTimes`).

use std::time::{Duration, Instant};

/// Keeps the pictures coming at an even pace. A picture that takes longer
/// than a screen refresh is shown twice, its neighbours once: that unevenness
/// shows as judder. When pictures keep coming close to the refresh time,
/// every picture is shown for two refreshes instead (half as many pictures,
/// but all for the same time); when the recent pictures fit well again, it
/// goes back.
pub struct Pacer {
    /// The screen's refresh time.
    period: Duration,
    /// When the last picture went to the screen.
    last: Instant,
    /// The work on the recent pictures (from one to the next, without waiting).
    recent: std::collections::VecDeque<Duration>,
    /// Refreshes per picture: 1 or 2.
    every: u32,
    /// Pictures since the switch to two refreshes per picture.
    calm: u32,
    /// Pictures shown so far (the first ones, while things are loaded, do not count).
    count: u32,
}

impl Pacer {
    /// `period`: the screen's refresh time, as far as the system tells it.
    pub fn new(period: Option<Duration>) -> Pacer {
        let period = period.unwrap_or(Duration::from_micros(16_667)).clamp(Duration::from_millis(4), Duration::from_millis(40));
        Pacer { period, last: Instant::now(), recent: Default::default(), every: 1, calm: 0, count: 0 }
    }

    /// The screen's refresh time.
    pub fn period(&self) -> Duration {
        self.period
    }

    /// Refreshes per picture at the moment: 1 or 2.
    pub fn every(&self) -> u32 {
        self.every
    }

    /// For how many screen refreshes the next picture is to be shown.
    /// (Handing it over waits for the screen, see platform.rs.)
    pub fn refreshes(&mut self) -> u32 {
        let work = self.last.elapsed();
        self.count = self.count.saturating_add(1);
        if self.count < 60 {
            return 1;
        }
        self.recent.push_back(work);
        if self.recent.len() > 60 {
            self.recent.pop_front();
        }
        let tight = self.recent.iter().filter(|w| **w > self.period.mul_f32(0.9)).count();
        if self.every == 1 && tight >= 3 {
            self.every = 2;
            self.calm = 0;
        } else if self.every == 2 {
            // back when the recent pictures fit well (at most one close one
            // among the last 60): a single slow picture must not start the
            // count again, or work near 70 % of the time stays at half rate
            self.calm += 1;
            let close = self.recent.iter().filter(|w| **w > self.period.mul_f32(0.8)).count();
            if self.calm > 120 && close <= 1 {
                self.every = 1;
                self.recent.clear();
            }
        }
        // a picture that took longer than one refresh already gets only one more
        if self.every == 2 && work < self.period { 2 } else { 1 }
    }

    /// The picture has gone to the screen.
    pub fn shown(&mut self) {
        self.last = Instant::now();
    }
}

/// F2: pictures per second and the work on one picture (drawing and
/// getting it ready, without the waiting), counted for a second at a time.
pub struct Fps {
    count: u32,
    work: Duration,
    since: Instant,
    text: String,
}

impl Fps {
    pub fn new() -> Fps {
        Fps { count: 0, work: Duration::ZERO, since: Instant::now(), text: String::from("...") }
    }

    /// What is shown in the corner.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// One more picture, which took `work`.
    pub fn count(&mut self, work: Duration) {
        self.count += 1;
        self.work += work;
        let secs = self.since.elapsed().as_secs_f64();
        if secs >= 1.0 {
            self.text = format!("{:.0} FPS  {:.1} ms", self.count as f64 / secs, self.work.as_secs_f64() * 1000.0 / self.count as f64);
            (self.count, self.work, self.since) = (0, Duration::ZERO, Instant::now());
        }
    }
}

/// When the parts of one picture began and ended.
pub struct PictureTimes {
    /// The pass of the game loop began.
    pub start: Instant,
    /// Drawing began.
    pub draw: Instant,
    /// Getting the picture ready for the window began (mouse pointer ...).
    pub present: Instant,
    /// Handing it over (and waiting for the screen) began.
    pub hand_over: Instant,
    /// It was handed over.
    pub end: Instant,
}

/// PAGAN_FRAMETIME: how long the parts of a picture take, written out every
/// 120 pictures.
pub struct FrameTimes {
    /// Per picture: drawing, getting ready, waiting, all, work (all without
    /// the waiting), in ms.
    times: Vec<[f64; 5]>,
}

impl FrameTimes {
    /// Only with PAGAN_FRAMETIME set.
    pub fn from_env() -> Option<FrameTimes> {
        crate::debug::env().frame_time.then(|| FrameTimes { times: Vec::new() })
    }

    /// One more picture; every 120 the averages and longest times go to
    /// stderr, with `more` as a second line. `period`: the screen's refresh time.
    pub fn add(&mut self, t: &PictureTimes, period: Duration, more: impl FnOnce() -> String) {
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
        self.times.push([ms(t.draw, t.present), ms(t.present, t.hand_over), ms(t.hand_over, t.end), ms(t.start, t.end), ms(t.start, t.hand_over)]);
        if self.times.len() < 120 {
            return;
        }
        let ft = &self.times;
        let avg = |i: usize| ft.iter().map(|f| f[i]).sum::<f64>() / ft.len() as f64;
        let max = |i: usize| ft.iter().map(|f| f[i]).fold(0.0, f64::max);
        let late = ft.iter().filter(|f| f[3] > period.as_secs_f64() * 1500.0).count();
        eprintln!(
            "frames: draw {:.1}/{:.1} present {:.1}/{:.1} work {:.1}/{:.1} wait {:.1} total {:.1}/{:.1} ms (avg/max), {late} late",
            avg(0), max(0), avg(1), max(1), avg(4), max(4), avg(2), avg(3), max(3)
        );
        eprintln!("  {}", more());
        self.times.clear();
    }
}
