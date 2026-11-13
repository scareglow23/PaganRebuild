//! The mouse pointer of the original game (U8MOUSE.SHP), drawn by the
//! game itself instead of the system's arrow.
//!
//! Over the world the arrow points away from the Avatar and grows longer
//! farther out, as in the original (option "pointer shows direction";
//! off: a blue arrow pointing up to the left); red with the weapon drawn
//! or over an enemy (modern controls). Special pointers: cross-hairs when a target is asked
//! for, a hand while a window is moved, a quill for writing, a red cross
//! for a moment when something cannot be done. While an item is carried:
//! the pointer where it can be put down, cross-hairs where it would be
//! thrown, the red cross where neither works.

use crate::game::Game;
use crate::shape::Shape;
use crate::view::Surface;
use std::time::{Duration, Instant};

/// Frames of the pointer shape.
const ARROW: usize = 0; // 0..=7: short blue arrows, up, then clockwise; 8.. middle, 16.. long
const RED_ARROW: usize = 25; // 25..=32
const TARGET: usize = 34;
const HAND: usize = 36;
const QUILL: usize = 38;
const MAGNIFIER: usize = 39;
const CROSS: usize = 40;
/// The plain pointer: the arrow up to the left.
const POINTER: usize = ARROW + 7;
const RED_POINTER: usize = RED_ARROW + 7;

/// How long the red cross shows.
const CROSS_TIME: Duration = Duration::from_millis(300);

pub struct Cursor {
    shape: Option<Shape>,
    /// Shown instead of the usual pointer until then.
    cross_until: Option<Instant>,
}

impl Cursor {
    pub fn load(game_static: &std::path::Path) -> Cursor {
        let shape = std::fs::read(game_static.join("U8MOUSE.SHP")).ok().and_then(|d| crate::shape::decode_shape(&d));
        if shape.is_none() {
            eprintln!("U8MOUSE.SHP not found: the system pointer is used");
        }
        Cursor { shape, cross_until: None }
    }

    /// The game's own pointer is there (else the system's arrow is shown).
    pub fn available(&self) -> bool {
        self.shape.is_some()
    }

    /// Something could not be done: a red cross for a moment.
    pub fn deny(&mut self) {
        self.cross_until = Some(Instant::now() + CROSS_TIME);
    }

    pub fn draw(&self, surf: &mut Surface, frame: usize, at: (i32, i32), game: &Game) {
        let frame = match self.cross_until {
            Some(t) if Instant::now() < t => CROSS,
            _ => frame,
        };
        let Some(f) = self.shape.as_ref().and_then(|s| s.frames.get(frame)) else { return };
        // the frame's offset is the pointer's tip
        surf.paint(f, at.0, at.1, false, false, &game.data.pal);
    }
}

/// What the pointer is doing, as far as the main loop knows.
#[derive(Clone, Copy, Default)]
pub struct PointerState {
    /// A window is held with the mouse.
    pub moving_window: bool,
}

impl Game {
    /// The pointer picture for the current situation; None hides it
    /// (a movie is playing).
    pub fn cursor_frame(&mut self, at: (i32, i32), state: PointerState) -> Option<usize> {
        use crate::title::Title;
        match &self.ui.title {
            Some(Title::Movie { .. }) => return None,
            Some(Title::Name { .. }) => return Some(QUILL),
            Some(_) => return Some(POINTER),
            None => {}
        }
        if let Some(book) = &self.ui.slot_menu {
            // writing in the diary, or looking in it (as in the original)
            return Some(if book.mode == crate::slots::Mode::Save { QUILL } else { MAGNIFIER });
        }
        if self.ui.menu.is_some() || self.ui.slider.is_some() || self.ui.reader.is_some() {
            return Some(POINTER);
        }
        if state.moving_window {
            return Some(HAND);
        }
        if self.ui.targeting.is_some() {
            return Some(TARGET);
        }
        // carrying something: a pointer to put it down, cross-hairs to throw
        // it, a red cross where neither works
        match self.drag_feedback(at.0, at.1) {
            crate::drag::DragFeedback::Drop => return Some(POINTER),
            crate::drag::DragFeedback::Throw => return Some(TARGET),
            crate::drag::DragFeedback::Nothing => return Some(CROSS),
            crate::drag::DragFeedback::None => {}
        }
        // red with the weapon drawn or over an enemy, but not over the windows
        let over_window = self.pick_gump(at.0, at.1).is_some();
        let red = !over_window
            && (self.fighting() || (self.options.modern_controls && self.foe_at(at.0, at.1, false).is_some()));
        if !over_window && !self.state(crate::game::AVATAR).dead {
            let (dir, length) = self.pointer_direction(at);
            return Some(if red { RED_ARROW + dir } else { ARROW + 8 * length + dir });
        }
        Some(if red { RED_POINTER } else { POINTER })
    }

    /// Direction (0 up, then clockwise) and length (0 short .. 2 long) of
    /// the pointer at `at`, seen from the Avatar. As the original (after
    /// ScummVM's Mouse::getMouseDirectionScreen / getMouseLength): the
    /// reference point lies a little below where the Avatar stands, up and
    /// down count double (the view is flattened), and the lengths change
    /// at 30 and 100 of 320 pixels across, 30 and 100 of 320 up and down
    /// of the original's 200 high screen; here measured by the view's
    /// height, so that a wide view does not stretch them.
    fn pointer_direction(&self, at: (i32, i32)) -> (usize, usize) {
        let av = self.avatar();
        let h = self.view.size.1.max(1) as f32;
        let (ax, ay) = self.to_screen(av.x, av.y, av.z);
        let dx = (at.0 - ax) as f32;
        let dy = (ay as f32 + h * 14.0 / 200.0) - at.1 as f32; // up is positive
        // clockwise from up, in eighths
        let angle = dx.atan2(2.0 * dy).to_degrees().rem_euclid(360.0);
        let dir = ((angle + 22.5) / 45.0) as usize % 8;
        let scale = h / 200.0;
        let (x_short, x_med) = (30.0 * scale, 100.0 * scale);
        let (y_short, y_med) = (30.0 * 200.0 / 320.0 * scale, 100.0 * 200.0 / 320.0 * scale);
        let length = if dx.abs() > x_med || dy.abs() > y_med {
            2
        } else if dx.abs() > x_short || dy.abs() > y_short {
            1
        } else {
            0
        };
        (dir, length)
    }
}
