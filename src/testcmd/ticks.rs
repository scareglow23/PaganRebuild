//! Letting time pass: t<n> runs n ticks, w<dir>[r]:<n> walks (r: runs) n ticks in a direction.

use super::*;

pub(super) fn run(game: &mut Game, _surf: &mut Surface, _opts: &mut ViewOptions, _st: &mut State, name: &str, arg: &str, cmd: &str) -> Result<Flow, String> {
    let num = |s: &str| number(s, cmd);
    if let Some(n) = name.strip_prefix('t') {
        // PAGAN_TICKTIME=1: report every tick that takes longer than 5 ms
        let timed = crate::debug::env().tick_time;
        for _ in 0..num(n)? {
            let start = std::time::Instant::now();
            game.step(None);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if timed && ms > 5.0 {
                println!("ticktime: tick {} took {ms:.1} ms", game.tick);
            }
        }
    } else if let Some(d) = name.strip_prefix('w') {
        let run = d.ends_with('r');
        let dir = num(d.trim_end_matches('r'))? as u8 & 7;
        for _ in 0..num(arg)? {
            game.step(Some(MoveInput { dir, run }));
        }
    } else {
        return Ok(Flow::Unknown);
    }
    Ok(Flow::Done)
}
