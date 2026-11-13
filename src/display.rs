//! From the drawn picture to the window: the mouse pointer drawn in, then
//! handed over (the graphics card applies the option "graphics filter"
//! and stretches it to the size the window shows, see gpuscale.rs).

use crate::*;

/// The view is drawn smaller or larger than the window (zoom): how many
/// view pixels one pixel of the original size is, as f32 bits.
pub static MOUSE_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000); // 1.0

/// How many screen pixels the window has per pixel of the original size
/// (the mouse is reported in screen pixels).
pub static WINDOW_FACTOR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// The mouse position in view pixels. (The window reports it in pixels of
/// the size it was opened with.)
pub fn mouse_pos(window: &Window, mode: MouseMode) -> Option<(f32, f32)> {
    let k = f32::from_bits(MOUSE_SCALE.load(std::sync::atomic::Ordering::Relaxed))
        / WINDOW_FACTOR.load(std::sync::atomic::Ordering::Relaxed) as f32;
    window.get_mouse_pos(mode).map(|(x, y)| (x * k, y * k))
}

/// What `present` made ready for `show`.
pub struct Upscaled {
    /// The step of the option "graphics filter" (`menu::PIXELS_*`), and the
    /// pixels the smoothing leaves sharp.
    filter: usize,
    sharp: Vec<bool>,
    /// A picture is ready to be handed over.
    ready: bool,
}

impl Upscaled {
    pub fn new() -> Upscaled {
        Upscaled { filter: crate::menu::PIXELS_SHARP, sharp: Vec::new(), ready: false }
    }
}

/// Makes the picture ready for the window, with the mouse pointer drawn
/// over it; `show` hands it over after the wait for the screen refresh.
/// (Smoothing and stretching to the window are done by the graphics card,
/// see gpuscale.rs.)
pub fn present(window: &mut Window, surf: &Surface, shown: &mut Surface, up: &mut Upscaled, game: &mut Game, pointer: &cursor::Cursor,
           state: cursor::PointerState) -> Result<(), Box<dyn Error>> {
    shown.pixels.clone_from(&surf.pixels);
    if let Some((x, y)) = mouse_pos(window, MouseMode::Discard) {
        let at = (x as i32, y as i32);
        if let Some(frame) = game.cursor_frame(at, state) {
            pointer.draw(shown, frame, at, game);
        }
    }
    // filtered only when shown larger than drawn (the mask only fits a
    // picture of the same size)
    let src = (surf.width as usize, surf.height as usize);
    up.filter = if window.display_size().0 > src.0 { game.options.pixels } else { crate::menu::PIXELS_SHARP };
    up.sharp.clear();
    if up.filter == crate::menu::PIXELS_XBR && game.view.sharp.len() == src.0 * src.1 {
        up.sharp.extend_from_slice(&game.view.sharp);
    }
    up.ready = true;
    Ok(())
}

/// Hands the picture made ready to the window. Done right after the wait
/// for the screen refresh, so that the copy is long finished when the
/// screen is put together the next time: a copy under way at that moment
/// shows as a seam across the picture (the old picture above, the new one
/// below).
pub fn show(window: &mut Window, shown: &Surface, up: &mut Upscaled, times: u32) -> Result<(), Box<dyn Error>> {
    if std::mem::take(&mut up.ready) {
        use crate::gpuscale::Look;
        let look = match up.filter {
            crate::menu::PIXELS_XBR => Look::Xbr(&up.sharp),
            crate::menu::PIXELS_CRT => Look::Crt,
            _ => Look::Plain,
        };
        window.update_with_buffer(&shown.pixels, shown.width as usize, shown.height as usize, look, times)?;
    } else {
        window.update();
    }
    Ok(())
}
