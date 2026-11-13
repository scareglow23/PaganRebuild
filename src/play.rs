//! The game in its window: one pass of the loop reads the input, lets
//! the game run on and shows the picture.
//!
//! The state the loop keeps from one pass to the next (what the mouse
//! buttons did, what is being held or dragged ...) is in `Play`. A pass
//! goes through these steps: the size of the view, Esc, then either one of
//! the screens that take all input (title, savegame book, options menu,
//! amount slider) or the game itself: its ticks, the keys, the right and
//! the left mouse button, the keys for the view, and drawing.

use crate::*;
use crate::display::{mouse_pos, present, show, Upscaled, MOUSE_SCALE, WINDOW_FACTOR};
use crate::pacing::{Fps, FrameTimes, Pacer, PictureTimes};

/// A typed character as the game's fonts have it (they use the DOS
/// character set: umlauts and accents have codes of their own); None for
/// what they cannot show.
fn font_char(c: char) -> Option<char> {
    const DOS: [(char, u32); 16] = [
        ('\u{e4}', 0x84), ('\u{f6}', 0x94), ('\u{fc}', 0x81), ('\u{c4}', 0x8E), ('\u{d6}', 0x99), ('\u{dc}', 0x9A),
        ('\u{df}', 0xE1), ('\u{e9}', 0x82), ('\u{e8}', 0x8A), ('\u{ea}', 0x88), ('\u{e0}', 0x85), ('\u{e2}', 0x83),
        ('\u{e7}', 0x87), ('\u{ee}', 0x8C), ('\u{f4}', 0x93), ('\u{fb}', 0x96),
    ];
    if let Some(&(_, code)) = DOS.iter().find(|&&(u, _)| u == c) {
        return char::from_u32(code);
    }
    (c.is_ascii_graphic() || c == ' ').then_some(c)
}

/// Converts a direction on screen (x right, y down) to a world direction.
fn screen_dir(sx: f32, sy: f32) -> u8 {
    // octant 0 = up, clockwise; "up" on screen is north-west in the world
    let octant = (sx.atan2(-sy) / std::f32::consts::FRAC_PI_4).round() as i32;
    ((octant + 7).rem_euclid(8)) as u8
}

/// `run`: with the modern controls, whether running is switched on (the
/// classic ones run while shift is held, or when the pointer is far away).
/// `road_keys`: the keys are turned by an eighth, so that one key alone
/// walks along the roads (up = to the upper right, right = to the lower
/// right and so on; two keys together walk straight across the view).
fn read_input(window: &Window, surf: &Surface, run: Option<bool>, road_keys: bool) -> Option<MoveInput> {
    let right_walks = run.is_none();
    let down = |keys: &[Key]| keys.iter().any(|&k| window.is_key_down(k));
    let mut sx = 0.0;
    let mut sy = 0.0;
    if down(&[Key::Left, Key::A]) {
        sx -= 1.0;
    }
    if down(&[Key::Right, Key::D]) {
        sx += 1.0;
    }
    if down(&[Key::Up, Key::W]) {
        sy -= 1.0;
    }
    if down(&[Key::Down, Key::S]) {
        sy += 1.0;
    }
    if sx != 0.0 || sy != 0.0 {
        return Some(MoveInput { dir: (screen_dir(sx, sy) + road_keys as u8) % 8, run: run.unwrap_or_else(|| down(&[Key::LeftShift, Key::RightShift])) });
    }
    if right_walks && window.get_mouse_down(MouseButton::Right) {
        let (mx, my) = mouse_pos(window, MouseMode::Clamp)?;
        // the Avatar stands in the middle of the view
        let (dx, dy) = (mx - surf.width as f32 / 2.0, my - surf.height as f32 / 2.0);
        if dx.abs() + dy.abs() < 8.0 {
            return None;
        }
        // the view is squashed 2:1 vertically; far away pointer = run
        let far = (dx * dx + 4.0 * dy * dy).sqrt() > surf.height as f32 * 0.45;
        return Some(MoveInput { dir: screen_dir(dx, 2.0 * dy), run: far });
    }
    None
}

/// The pad's left stick: the Avatar walks the way it is pushed (on
/// screen), and runs when it is pushed all the way (or, with the modern
/// controls, when running is switched on).
fn stick_input(window: &Window, run: Option<bool>) -> Option<MoveInput> {
    let (sx, sy) = window.stick();
    let push = (sx * sx + sy * sy).sqrt();
    if push < 0.3 {
        return None;
    }
    Some(MoveInput { dir: screen_dir(sx, -sy), run: run.unwrap_or(false) || push > 0.9 })
}

/// How long the button has to stay down on the ground before the Avatar
/// follows the pointer instead of walking to the spot clicked.
const HOLD_TO_FOLLOW: Duration = Duration::from_millis(180);

/// Following the pointer (button held): towards it, running when it is
/// far from the Avatar, standing when it is on him.
pub fn follow_input(game: &Game, (mx, my): (i32, i32), view_h: i32) -> Option<MoveInput> {
    let av = game.avatar();
    let (sx, sy) = game.to_screen(av.x, av.y, av.z);
    // from the middle of his body
    let (dx, dy) = ((mx - sx) as f32, (my - (sy - 20)) as f32);
    // the view is squashed 2:1 vertically
    let dist = (dx * dx + 4.0 * dy * dy).sqrt();
    if dist < 20.0 {
        return None;
    }
    let far = dist > view_h as f32 * 0.35;
    Some(MoveInput { dir: game.unturn_dir(screen_dir(dx, 2.0 * dy)), run: far || game.run_toggle })
}

/// The game runs at 30 ticks per second.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / 30);
const DOUBLE_CLICK: Duration = Duration::from_millis(350);

/// How a step of a pass ends.
#[derive(PartialEq)]
enum Pass {
    /// On with the next step.
    Go,
    /// This pass is over.
    Done,
    /// The game ends.
    Quit,
}

/// The windowed game and what its loop remembers between passes.
pub struct Play {
    args: Args,
    /// The game began with the title (not straight in the world).
    with_title: bool,
    game: Game,
    window: Window,
    surf: Surface,
    /// The picture with the pointer drawn in (the view itself stays without it).
    shown: Surface,
    upscaled: Upscaled,
    opts: ViewOptions,
    /// R: the roofs stay hidden wherever the Avatar is.
    roofs_always: bool,
    /// The game's own mouse pointer replaces the system's arrow.
    mouse_pointer: cursor::Cursor,
    next_tick: Instant,
    /// Something changed: the view is drawn anew.
    dirty: bool,
    /// Number of the last F12 picture.
    shot: u32,
    left_was_down: bool,
    /// Fighting by clicking: where the button went down on the ground, and when.
    held_walk: Option<(i32, i32, u32)>,
    /// The attack key is held: keep striking.
    attack_key_held: bool,
    /// Where the left button went down and what was under it.
    press: Option<((i32, i32), Option<u16>)>,
    /// When the button went down, and whether the Avatar follows the
    /// pointer because it is held on the ground (option "hold to follow").
    press_at: Instant,
    following: bool,
    follow_pointer: Option<(i32, i32)>,
    right_was_down: bool,
    /// When the right button last went down (a double click kicks).
    right_press_at: Option<Instant>,
    /// The right button was pressed on a window: do not walk with it.
    right_on_gump: bool,
    /// The weapon is to be put away once the Avatar is free to.
    sheathe: bool,
    /// A right click in a fight (modern controls): the weapon goes away
    /// unless a second click follows (which kicks).
    sheathe_pending: Option<Instant>,
    /// A single click only counts once it is clear no second one follows.
    pending_click: Option<(Instant, u16)>,
    esc_was_down: bool,
    book_mouse_was_down: bool,
    /// The slider: button held in the last pass, and held since a press on the groove.
    slider_mouse_was_down: bool,
    slider_drag: bool,
    /// The left button was still held when the slider closed: that press
    /// belongs to the slider and does nothing in the world.
    left_swallowed: bool,
    /// The savegame book was open in the last pass.
    book_was_open: bool,
    shift_was_down: bool,
    /// A window moved with the mouse: its owner and where it is held.
    gump_held: Option<(u16, i32, i32)>,
    /// Dragging a thing over a page arrow of the backpack: which arrow, since when.
    arrow_hover: Option<(i32, Instant)>,
    started: Instant,
    /// The last pictures drawn (five seconds of movement), for F3.
    recent: std::collections::VecDeque<(Vec<u32>, String)>,
    /// The size of the view at 100 % zoom.
    base_size: (i32, i32),
    /// PAGAN_FRAMETIME: how long the parts of a picture take.
    frame_times: Option<FrameTimes>,
    pacer: Pacer,
    /// F2: pictures per second shown in the corner.
    fps: Option<Fps>,
    /// The key map last handed to the window (see bindings.rs).
    key_map: crate::hash::HashMap<Key, Option<Key>>,
    /// An arrow held on the pad in a menu: which, since when, last repeated.
    pad_repeat: Option<(Key, Instant, Instant)>,
}

impl Play {
    /// Opens the window for the game.
    pub fn new(args: Args, game: Game, surf: Surface, opts: ViewOptions, with_title: bool) -> Result<Play, Box<dyn Error>> {
        let factor = args.scale as usize;
        let (w, h) = (surf.width as usize, surf.height as usize);
        WINDOW_FACTOR.store(factor as u32, std::sync::atomic::Ordering::Relaxed);
        // the pictures are paced by the screen itself (see platform.rs)
        let mut window = Window::new("PaganRebuild", w * factor, h * factor)?;
        let mouse_pointer = cursor::Cursor::load(&game.data.static_dir);
        window.set_cursor_visibility(!mouse_pointer.available());
        let pacer = Pacer::new(window.refresh_period());
        let frame_times = FrameTimes::from_env();
        if frame_times.is_some() {
            eprintln!("screen refresh: {:.2} ms", pacer.period().as_secs_f64() * 1000.0);
        }
        Ok(Play {
            args,
            with_title,
            shown: Surface::new(surf.width, surf.height),
            upscaled: Upscaled::new(),
            base_size: (surf.width, surf.height),
            game,
            window,
            surf,
            opts,
            roofs_always: false,
            mouse_pointer,
            next_tick: Instant::now() + TICK,
            dirty: true,
            shot: 0,
            left_was_down: false,
            held_walk: None,
            attack_key_held: false,
            press: None,
            press_at: Instant::now(),
            following: false,
            follow_pointer: None,
            right_was_down: false,
            right_press_at: None,
            right_on_gump: false,
            sheathe: false,
            sheathe_pending: None,
            pending_click: None,
            esc_was_down: false,
            book_mouse_was_down: false,
            slider_mouse_was_down: true,
            slider_drag: false,
            left_swallowed: false,
            book_was_open: false,
            shift_was_down: false,
            gump_held: None,
            arrow_hover: None,
            started: Instant::now(),
            recent: std::collections::VecDeque::new(),
            frame_times,
            pacer,
            // (PAGAN_FPS=1: shown from the start)
            fps: std::env::var_os("PAGAN_FPS").map(|_| Fps::new()),
            key_map: Default::default(),
            pad_repeat: None,
        })
    }

    /// Plays until the window is closed or the game is left.
    pub fn run(mut self) -> Result<(), Box<dyn Error>> {
        while self.window.is_open() {
            if self.pass()? == Pass::Quit {
                break;
            }
        }
        Ok(())
    }

    fn pass(&mut self) -> Result<Pass, Box<dyn Error>> {
        let loop_start = Instant::now();
        if self.game.ui.slot_menu.is_none() {
            self.book_was_open = false;
        }
        self.pad_pass();
        // the presses and releases since the last pass
        let (left_edges, right_edges) = self.window.take_clicks();
        self.fit_view();
        let esc = self.window.is_key_down(Key::Escape);
        let esc_pressed = esc && !self.esc_was_down;
        match self.escape(esc) {
            Pass::Go => {}
            other => return Ok(other),
        }
        if !self.game.started && self.game.ui.title.is_none() && self.game.ui.menu.is_none() && self.game.ui.slot_menu.is_none() {
            // back from the options or the diary before the game has begun
            self.game.open_main_menu();
        }
        if self.game.ui.title.is_some() {
            self.title_pass(esc_pressed, left_edges)?;
            return Ok(Pass::Done);
        }
        self.function_keys();
        if self.game.restart {
            self.restart()?;
            return Ok(Pass::Done);
        }
        if self.game.ui.slot_menu.is_some() {
            self.book_pass(esc_pressed, left_edges)?;
            return Ok(Pass::Done);
        }
        if self.game.ui.menu.is_some() {
            self.menu_pass()?;
            return Ok(Pass::Done);
        }
        if self.game.ui.slider.is_some() {
            self.slider_pass(left_edges)?;
            return Ok(Pass::Done);
        }
        if self.game.ui.worldmap.is_some() {
            self.worldmap_pass(left_edges)?;
            return Ok(Pass::Done);
        }

        // the optional modern controls (see controls.rs)
        let modern = self.game.options.modern_controls;
        self.run_toggle_key(modern);
        self.advance(modern);
        self.game_keys(modern);
        self.right_button(modern, right_edges);
        self.left_button(modern, left_edges);
        self.view_keys(modern);
        self.draw_and_show(loop_start)?;
        Ok(Pass::Go)
    }

    /// The player's keys and the gamepad. In the world the pad's buttons
    /// press the keys (or mouse buttons) they are given (see bindings.rs);
    /// in the menus, books and conversations they do what is usual there:
    /// A confirms, B goes back, the cross and the left stick move. The
    /// right stick moves the pointer everywhere.
    fn pad_pass(&mut self) {
        let (window, game) = (&mut self.window, &self.game);
        let ui = &game.ui;
        let reading = ui.reader.is_some();
        let world = ui.title.is_none() && ui.menu.is_none() && ui.slot_menu.is_none() && ui.slider.is_none() && !reading && game.ask.is_none();
        // keys chosen by the player count in the world only (menus keep theirs)
        let map = if world { game.bindings.key_map() } else { Default::default() };
        if map != self.key_map {
            window.set_key_map(map.clone());
            self.key_map = map;
        }
        let mut keys = crate::hash::HashSet::default();
        let mut buttons = [false; 2];
        let mut nav: Option<Key> = None;
        if !game.keys_waiting(true) {
            if world {
                buttons = pad_in_world(window, &game.bindings, &mut keys);
            } else {
                let back = if reading && !game.reader_browses() { Key::Space } else { Key::Escape };
                buttons = pad_in_menus(window, reading, back, &mut keys);
                nav = [Key::Up, Key::Down, Key::Left, Key::Right].into_iter().find(|k| keys.contains(k));
            }
        }
        // an arrow held repeats, as on the keyboard
        let now = Instant::now();
        let mut repeat = Vec::new();
        self.pad_repeat = match (nav, self.pad_repeat) {
            (Some(k), Some((held, since, last))) if k == held => {
                if now - since > Duration::from_millis(400) && now - last > Duration::from_millis(110) {
                    repeat.push(k);
                    Some((k, since, now))
                } else {
                    Some((k, since, last))
                }
            }
            (Some(k), _) => Some((k, now, now)),
            (None, _) => None,
        };
        window.set_virtual(&keys, buttons, &repeat);
        // the right stick moves the pointer, faster the further it is pushed
        let (rx, ry) = window.right_stick();
        let push = (rx * rx + ry * ry).sqrt();
        if push > 0.2 {
            let (_, h) = window.pixel_size();
            let speed = (push - 0.2) / 0.8;
            let step = speed * speed * h as f32 / 50.0 + 1.0;
            window.move_pointer((rx / push * step) as f64, (-ry / push * step) as f64);
        }
    }

    /// Zoom: the view is drawn smaller or larger and stretched to the
    /// window; menus and movies keep the original size. Books, scrolls and
    /// signs are laid over the world as it is (in original pixels, so they
    /// grow and shrink with the zoom): scaling the world to the original
    /// size behind them blurred it.
    fn fit_view(&mut self) {
        let game = &self.game;
        let full_size = game.ui.title.is_some() || game.ui.menu.is_some() || game.ui.slot_menu.is_some() || game.ui.worldmap.is_some();
        // option "wide view": the picture fills the window and the view grows
        // to its proportions; menus and books over the world are centred in
        // it, only the title screen and movies keep the original format
        self.window.set_fill(game.options.wide_view && !game.title_covers_view());
        let factor = self.args.scale as i32;
        let (lw, lh) = self.window.logical_size();
        self.base_size = (lw as i32 / factor, lh as i32 / factor);
        let zoom = menu::ZOOMS[game.options.zoom.min(menu::ZOOMS.len() - 1)];
        let pct = if full_size { 100 } else { zoom };
        let want = (self.base_size.0 * pct / 100, self.base_size.1 * pct / 100);
        // the world behind a menu keeps its zoom (drawn apart and scaled)
        self.game.view.under_menu = (full_size && zoom != 100)
            .then(|| ((self.base_size.0 * zoom / 100, self.base_size.1 * zoom / 100), zoom as f32 / 100.0));
        if (self.surf.width, self.surf.height) != want {
            self.surf = Surface::new(want.0, want.1);
            self.shown = Surface::new(want.0, want.1);
            self.game.view.size = want;
            self.game.view.zoom = pct as f32 / 100.0;
            MOUSE_SCALE.store((pct as f32 / 100.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
            // the kept pictures have the old size
            self.recent.clear();
            self.dirty = true;
        }
    }

    /// Esc closes the menu if it is open, otherwise it opens the main menu.
    fn escape(&mut self, esc: bool) -> Pass {
        let game = &mut self.game;
        if esc && !self.esc_was_down {
            if game.ui.slot_menu.is_some() {
                // handled with the book
            } else if game.reader_browses() {
                game.close_reader();
                self.esc_was_down = true;
                return Pass::Done;
            } else if game.ui.menu.is_some() {
                if !game.menu_back() {
                    game.toggle_menu();
                }
            } else if game.ui.worldmap.is_some() {
                game.close_worldmap();
            } else if game.options.modern_controls && !game.ui.gumps.is_empty() {
                game.close_all_gumps();
            } else if game.ui.title.is_none() {
                game.open_main_menu();
                // this press is used up
                self.esc_was_down = true;
                return Pass::Done;
            }
        }
        self.esc_was_down = esc;
        if game.quit {
            return Pass::Quit;
        }
        Pass::Go
    }

    /// The picture of a screen that has the game standing still.
    fn show_still(&mut self) -> Result<(), Box<dyn Error>> {
        self.game.draw(&mut self.surf, &mut self.opts, self.roofs_always);
        let state = cursor::PointerState { moving_window: self.gump_held.is_some() };
        present(&mut self.window, &self.surf, &mut self.shown, &mut self.upscaled, &mut self.game, &self.mouse_pointer, state)?;
        show(&mut self.window, &self.shown, &mut self.upscaled, 1)?;
        self.next_tick = Instant::now() + TICK;
        Ok(())
    }

    /// The main menu, the name entry, a movie or the credits take all input.
    fn title_pass(&mut self, esc_pressed: bool, left_edges: platform::Edges) -> Result<(), Box<dyn Error>> {
        let window = &self.window;
        let keys = window.get_keys_pressed(KeyRepeat::Yes);
        let down = window.get_mouse_down(MouseButton::Left);
        let input = title::Input {
            up: keys.contains(&Key::Up),
            down: keys.contains(&Key::Down),
            enter: window.is_key_pressed(Key::Enter, KeyRepeat::No),
            esc: esc_pressed,
            backspace: keys.contains(&Key::Backspace),
            // what the keys typed: with the keyboard's own layout, umlauts too
            chars: window.typed().chars().filter_map(font_char).collect(),
            mouse: mouse_pos(window, MouseMode::Discard).map(|(x, y)| (x as i32, y as i32)),
            click: left_edges.pressed || (down && !self.left_was_down),
        };
        self.left_was_down = down;
        self.game.title_input(&input, self.surf.width, self.surf.height);
        self.game.title_tick();
        self.show_still()?;
        self.dirty = true;
        Ok(())
    }

    /// The menu, the savegame book, quick save and load.
    fn function_keys(&mut self) {
        let (window, game) = (&self.window, &mut self.game);
        if window.is_key_pressed(Key::V, KeyRepeat::No) || window.is_key_pressed(Key::F10, KeyRepeat::No) {
            game.toggle_menu();
        }
        if game.started && window.is_key_pressed(Key::F6, KeyRepeat::No) {
            game.open_slot_menu(slots::Mode::Save);
        }
        if window.is_key_pressed(Key::F7, KeyRepeat::No) {
            game.open_slot_menu(slots::Mode::Load);
        }
        if game.started && window.is_key_pressed(Key::F5, KeyRepeat::No) {
            game.quick_save();
        }
        if window.is_key_pressed(Key::F9, KeyRepeat::No) {
            let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
            if game.cheat && !ctrl {
                // with --cheat F9 changes the helmet (shift: backwards); ctrl+F9 loads
                game.cheat_cycle_helm(window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift));
            } else {
                game.quick_load();
            }
        }
    }

    /// A new game: everything is read afresh.
    fn restart(&mut self) -> Result<(), Box<dyn Error>> {
        self.game = new_game(&self.args)?;
        self.game.view.size = (self.surf.width, self.surf.height);
        if self.with_title {
            // a new game begins with the name
            self.game.started = false;
            self.game.enter_name();
        }
        self.next_tick = Instant::now() + TICK;
        Ok(())
    }

    /// The savegame book takes all input; the game stands still.
    fn book_pass(&mut self, esc_pressed: bool, left_edges: platform::Edges) -> Result<(), Box<dyn Error>> {
        let (window, game) = (&self.window, &mut self.game);
        for (key, dy) in [(Key::Up, -1), (Key::Down, 1)] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                game.slot_menu_key(dy);
            }
        }
        for (key, dir) in [(Key::Left, -1), (Key::Right, 1), (Key::PageUp, -1), (Key::PageDown, 1)] {
            if window.is_key_pressed(key, KeyRepeat::No) {
                game.slot_menu_turn(dir);
            }
        }
        if window.is_key_pressed(Key::Enter, KeyRepeat::No) {
            game.slot_menu_activate();
        }
        if esc_pressed {
            game.slot_menu_close();
        }
        // writing a savegame's name: the typed text, umlauts too
        let chars: Vec<char> = window.typed().chars().filter_map(font_char).collect();
        let backspace = window.is_key_pressed(Key::Backspace, KeyRepeat::Yes);
        if !chars.is_empty() || backspace {
            game.slot_menu_type(&chars, backspace);
        }
        let down = window.get_mouse_down(MouseButton::Left);
        // The click that opened the book (on "Write diary", say) is not a
        // click into it: the button may still be down when it appears,
        // over one of its entries.
        let just_opened = !self.book_was_open;
        self.book_was_open = true;
        if !just_opened && (left_edges.pressed || (down && !self.book_mouse_was_down)) {
            if let Some((x, y)) = mouse_pos(window, MouseMode::Clamp) {
                game.slot_menu_click(x as i32, y as i32, self.surf.width, self.surf.height);
            }
        }
        self.book_mouse_was_down = down || (just_opened && left_edges.pressed);
        self.show_still()
    }

    /// The options menu; the game stands still while it is open.
    fn menu_pass(&mut self) -> Result<(), Box<dyn Error>> {
        let (window, game) = (&self.window, &mut self.game);
        // the page where keys are chosen waits for one (Esc has stopped it already)
        if game.keys_waiting(false) || game.keys_waiting(true) {
            for k in window.get_keys_pressed(KeyRepeat::No) {
                game.keys_take_key(k);
            }
            if game.keys_waiting(true) {
                if let Some(&(p, _)) = window.pad_events().iter().find(|e| e.1) {
                    game.keys_take_pad(p);
                }
            }
            return self.show_still();
        }
        for (key, dx, dy) in [(Key::Left, -1, 0), (Key::Right, 1, 0), (Key::Up, 0, -1), (Key::Down, 0, 1)] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                game.menu_key(dx, dy);
            }
        }
        if window.is_key_pressed(Key::Enter, KeyRepeat::No) {
            game.menu_activate();
        }
        // page up / down: the tab before / after
        if window.is_key_pressed(Key::PageUp, KeyRepeat::No) {
            game.menu_tab(-1);
        }
        if window.is_key_pressed(Key::PageDown, KeyRepeat::No) {
            game.menu_tab(1);
        }
        if let Some((x, y)) = mouse_pos(window, MouseMode::Clamp) {
            game.menu_mouse(x as i32, y as i32, window.get_mouse_down(MouseButton::Left), self.surf.width, self.surf.height);
        }
        self.show_still()
    }

    /// A script asks for an amount: the slider takes all input.
    fn slider_pass(&mut self, left_edges: platform::Edges) -> Result<(), Box<dyn Error>> {
        let (window, game) = (&self.window, &mut self.game);
        let (w, h) = (self.surf.width, self.surf.height);
        for (key, d) in [(Key::Left, -1), (Key::Right, 1), (Key::Down, -10), (Key::Up, 10)] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                game.slider_change(d);
            }
        }
        // a click on OK confirms, on an arrow steps, on the groove moves
        // the knob, which then follows the held button
        let down = window.get_mouse_down(MouseButton::Left);
        if let Some((x, y)) = mouse_pos(window, MouseMode::Clamp) {
            if left_edges.pressed || (down && !self.slider_mouse_was_down) {
                self.slider_drag = game.slider_click(x as i32, y as i32, w, h);
            } else if down && self.slider_drag {
                game.slider_mouse(x as i32, y as i32, w, h);
            }
        }
        self.slider_mouse_was_down = down;
        if !down {
            self.slider_drag = false;
        }
        if window.is_key_pressed(Key::Enter, KeyRepeat::No) || window.is_key_pressed(Key::Space, KeyRepeat::No) {
            game.slider_confirm();
        }
        // what the button does here is the slider's: the world must not
        // take it for a new click once the slider is gone
        self.left_was_down = down;
        if game.ui.slider.is_none() && down {
            self.left_swallowed = true;
        }
        self.show_still()
    }

    /// The large map takes all input; the game stands still.
    fn worldmap_pass(&mut self, left_edges: platform::Edges) -> Result<(), Box<dyn Error>> {
        let (window, game) = (&self.window, &mut self.game);
        let (w, h) = (self.surf.width, self.surf.height);
        if window.is_key_pressed(Key::O, KeyRepeat::No) {
            game.close_worldmap();
        }
        for (key, dx, dy) in [(Key::Left, -1, 0), (Key::Right, 1, 0), (Key::Up, 0, -1), (Key::Down, 0, 1)] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                game.worldmap_key(dx, dy, 0);
            }
        }
        if window.is_key_pressed(Key::Equal, KeyRepeat::Yes) || window.is_key_pressed(Key::NumPadPlus, KeyRepeat::Yes) {
            game.worldmap_key(0, 0, -1);
        }
        if window.is_key_pressed(Key::Minus, KeyRepeat::Yes) || window.is_key_pressed(Key::NumPadMinus, KeyRepeat::Yes) {
            game.worldmap_key(0, 0, 1);
        }
        if window.is_key_pressed(Key::Delete, KeyRepeat::No) {
            game.set_guide_target(None);
        }
        let down = window.get_mouse_down(MouseButton::Left);
        if let Some((x, y)) = mouse_pos(window, MouseMode::Clamp) {
            let wheel = match window.get_scroll_wheel() {
                Some((_, v)) if v > 0.0 => -1,
                Some((_, v)) if v < 0.0 => 1,
                _ => 0,
            };
            let pressed = left_edges.pressed && !self.left_swallowed;
            let released = left_edges.released && !self.left_swallowed;
            game.worldmap_mouse(x as i32, y as i32, pressed, down, released, wheel, w, h);
        }
        if !down {
            self.left_swallowed = false;
        }
        // the search for the way goes on meanwhile
        game.tick_guide();
        self.left_was_down = down;
        if game.ui.worldmap.is_none() && down {
            self.left_swallowed = true;
        }
        self.dirty = true;
        self.show_still()
    }

    /// Modern controls: shift switches between walking and running.
    fn run_toggle_key(&mut self, modern: bool) {
        let shift = self.window.is_key_down(Key::LeftShift) || self.window.is_key_down(Key::RightShift);
        let game = &mut self.game;
        if modern && shift && !self.shift_was_down && game.ui.drag.is_none() {
            game.run_toggle = !game.run_toggle;
            let text = match (game.run_toggle, game.language) {
                (true, 'G') => "Rennen",
                (false, 'G') => "Gehen",
                (true, 'F') => "Courir",
                (false, 'F') => "Marcher",
                (true, _) => "Running",
                (false, _) => "Walking",
            };
            game.notify(text);
        }
        self.shift_was_down = shift;
    }

    /// The game's ticks, with the Avatar moved by the keys or the pointer.
    fn advance(&mut self, modern: bool) {
        let now = Instant::now();
        if now < self.next_tick {
            return;
        }
        // never try to catch up after a stall
        self.next_tick = (self.next_tick + TICK).max(now);
        let (window, game) = (&self.window, &mut self.game);
        let right_on_gump = self.right_on_gump;
        // the arrows turn the pages of the journal and the spell book
        let browsing = game.reader_browses();
        let input = read_input(window, &self.surf, modern.then_some(game.run_toggle), game.options.road_keys)
            .filter(|_| !right_on_gump || !window.get_mouse_down(MouseButton::Right))
            .or_else(|| stick_input(window, modern.then_some(game.run_toggle)))
            // answers are picked with the arrows: the Avatar stays
            .filter(|_| game.ask.is_none() && !browsing)
            // directions on screen mean other directions in a turned view
            .map(|i| MoveInput { dir: game.unturn_dir(i.dir), ..i })
            .or_else(|| follow_input(game, self.follow_pointer?, self.surf.height));
        // F4 held: the game runs six times as fast (long walks, scenes)
        let fast = window.is_key_down(Key::F4);
        if fast {
            game.audio.stop_speech();
        }
        for _ in 0..if fast { 6 } else { 1 } {
            game.step(input);
        }
        self.dirty = true;
    }

    /// Conversation answers, potions, reading, jumping, fighting, the backpack.
    fn game_keys(&mut self, modern: bool) {
        let (window, game) = (&self.window, &mut self.game);
        let (w, h) = (self.surf.width, self.surf.height);
        // conversation answers: number keys or a click on the line
        const NUMBER_KEYS: [Key; 9] =
            [Key::Key1, Key::Key2, Key::Key3, Key::Key4, Key::Key5, Key::Key6, Key::Key7, Key::Key8, Key::Key9];
        // without a conversation 1 to 5 drink a potion: yellow, red, orange,
        // purple, black (or the keys the player gave them: in the world a
        // key chosen for a potion arrives here as its number, see bindings.rs)
        for (i, &k) in NUMBER_KEYS.iter().enumerate() {
            if window.is_key_pressed(k, KeyRepeat::No) {
                if game.ask.is_some() {
                    game.answer(i);
                } else if i < crate::potions::QUICK_FRAMES.len() {
                    game.quick_potion(i);
                }
            }
        }

        // or picked with the arrows (and the pad) and taken with Enter
        if let Some((_, choices)) = &game.ask {
            let n = choices.len() as i32;
            for (key, d) in [(Key::Up, -1), (Key::Down, 1)] {
                if n > 0 && window.is_key_pressed(key, KeyRepeat::Yes) {
                    let at = game.ask_pick.map_or(if d > 0 { -1 } else { n }, |p| p as i32);
                    game.ask_pick = Some((at + d).rem_euclid(n) as usize);
                }
            }
        }
        let enter = window.is_key_pressed(Key::Enter, KeyRepeat::No);
        if let (true, Some(_), Some(p)) = (enter, &game.ask, game.ask_pick) {
            game.answer(p);
            return;
        }

        // the journal and the spell book: the arrows and the page keys
        // turn the pages both ways
        if game.reader_browses() {
            for (key, dir) in [(Key::Left, -1), (Key::Right, 1), (Key::PageUp, -1), (Key::PageDown, 1)] {
                if window.is_key_pressed(key, KeyRepeat::No) {
                    if dir < 0 {
                        game.reader_back();
                    } else {
                        game.reader_on();
                    }
                }
            }
        }
        // reading: any click or space turns the page
        let pointer = mouse_pos(window, MouseMode::Discard).map(|(x, y)| (x as i32, y as i32));
        if window.is_key_pressed(Key::Space, KeyRepeat::No) || window.is_key_pressed(Key::Enter, KeyRepeat::No) {
            if game.ui.reader.is_some() {
                game.reader_next();
            } else if !game.player_in_control() || window.is_key_pressed(Key::Enter, KeyRepeat::No) {
                // a scene is playing: on to its next line (Enter always ends
                // the speech on screen)
                game.skip_barks();
            } else if window.is_key_pressed(Key::Space, KeyRepeat::No) {
                // space jumps to where the pointer is (people talking on
                // the side do not stop that)
                game.jump(pointer, w, h);
            }
        }
        if window.is_key_pressed(Key::J, KeyRepeat::No) {
            game.jump(pointer, w, h);
        }
        if window.is_key_pressed(Key::Tab, KeyRepeat::No) {
            game.toggle_combat();
        }
        if window.is_key_pressed(Key::F, KeyRepeat::No) {
            // at the nearest enemy; held down, the Avatar keeps striking
            game.attack_nearest();
            self.attack_key_held = true;
        }
        if self.attack_key_held && !window.is_key_down(Key::F) {
            self.attack_key_held = false;
            game.combat_release();
        }
        if window.is_key_pressed(Key::K, KeyRepeat::No) {
            // the key ring, as in the original (it asks for the door or chest)
            game.use_keyring();
        }
        if window.is_key_pressed(Key::Q, KeyRepeat::No) {
            game.avatar_attack(None, true);
        }
        game.avatar_block(window.is_key_down(Key::B));
        if modern && window.is_key_pressed(Key::E, KeyRepeat::No) {
            game.use_nearest();
        }
        if window.is_key_pressed(Key::I, KeyRepeat::No) {
            game.open_inventory();
        }
        if window.is_key_pressed(Key::C, KeyRepeat::No) {
            game.close_all_gumps();
        }
    }

    /// The right button: picking no target, closing a window, looking at
    /// things, jumping, putting the weapon away.
    fn right_button(&mut self, modern: bool, right_edges: platform::Edges) {
        let (window, game) = (&self.window, &mut self.game);
        let (w, h) = (self.surf.width, self.surf.height);
        // a right click on a container window closes it
        let right = window.get_mouse_down(MouseButton::Right);
        let right_pressed = right_edges.pressed || (right && !self.right_was_down);
        if right_pressed && game.ui.reader.is_some() {
            // a book takes the click (nothing below it is looked at or
            // walked to); the journal and the spell book close
            if game.reader_browses() {
                game.close_reader();
            }
            self.right_on_gump = true;
            self.right_press_at = None;
        } else if right_pressed && game.ui.targeting.is_some() {
            game.target_picked(0);
            self.right_on_gump = true; // do not walk with this click
        } else if right_pressed {
            // a thing that can be picked up goes straight into the backpack:
            // modern controls with the right button, classic ones with shift
            // + right button (not in a fight: there the right button keeps
            // putting the weapon away and kicking)
            let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
            let mouse = mouse_pos(window, MouseMode::Discard);
            let taken = match mouse {
                Some((x, y)) if (modern || shift) && !game.fighting() => game.pick(x as i32, y as i32).and_then(|id| game.take_into_backpack(id)),
                _ => None,
            };
            if taken.is_some() {
                // (no walking or jumping with this click)
                self.right_on_gump = true;
                self.right_press_at = None;
            } else if let Some((x, y)) = mouse {
                // modern controls: on a thing in a window the right button looks at
                // it; beside the things it closes the window
                let in_window = if modern { game.pick_gump(x as i32, y as i32).and_then(|(_, hit)| hit) } else { None };
                self.right_on_gump = match in_window {
                    Some(id) => {
                        game.interact(id, false);
                        true
                    }
                    None => game.close_gump_at(x as i32, y as i32) || game.on_minimap(x as i32, y as i32),
                };
                // classic controls in a fight: a double right click kicks
                // towards the pointer, as in the original
                let now = Instant::now();
                let double = self.right_press_at.is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK);
                self.right_press_at = if double { None } else { Some(now) };
                if !self.right_on_gump && game.fighting() && double {
                    // a double right click kicks (both controls), and the
                    // weapon stays out
                    self.sheathe_pending = None;
                    let (dx, dy) = (x - w as f32 / 2.0, y - h as f32 / 2.0);
                    game.avatar_attack(Some(game.unturn_dir(screen_dir(dx, 2.0 * dy))), true);
                } else if modern && !self.right_on_gump && game.fighting() {
                    // in a fight a right click puts the weapon away (as soon
                    // as a blow under way is done), once it is sure that no
                    // second click follows
                    game.cancel_approach();
                    self.sheathe_pending = Some(now);
                } else if modern && !self.right_on_gump {
                    // the right button looks at things; on bare ground the
                    // Avatar jumps there
                    match game.pick(x as i32, y as i32) {
                        Some(id) if !game.is_ground(id) => game.interact(id, false),
                        _ => {
                            game.jump(Some((x as i32, y as i32)), w, h);
                        }
                    }
                }
            }
        }
        if !right {
            self.right_on_gump = false;
        }
        self.right_was_down = right;
        if self.sheathe_pending.is_some_and(|t| t.elapsed() > DOUBLE_CLICK) {
            self.sheathe_pending = None;
            self.sheathe = true;
        }
        if self.sheathe && !game.mover(AVATAR).busy() {
            if game.fighting() {
                game.toggle_combat();
            }
            self.sheathe = false;
        }
    }

    /// The left button: clicks, double clicks, dragging things and windows,
    /// fighting and walking by clicking, following the pointer.
    fn left_button(&mut self, modern: bool, left_edges: platform::Edges) {
        let left = self.window.get_mouse_down(MouseButton::Left);
        let mouse = mouse_pos(&self.window, MouseMode::Clamp).map(|(x, y)| (x as i32, y as i32));
        let now = Instant::now();
        if self.left_swallowed {
            // still the press that closed the slider: wait for its release
            self.left_swallowed = left;
            self.left_was_down = left;
            return;
        }
        // a quick click can be pressed and released within one pass
        let released = !left && (self.left_was_down || left_edges.released);
        if left_edges.pressed || (left && !self.left_was_down) {
            self.left_pressed(modern, mouse, now);
        }
        if !left {
            self.gump_held = None;
        }
        self.left_held(left, mouse);
        // click-walk: the button held a moment on the ground makes the
        // Avatar follow the pointer; a short click still walks to the spot
        let game = &mut self.game;
        if left && !self.following && modern && game.options.click_walk && game.options.hold_walk && game.ui.drag.is_none() && !game.fighting() {
            if let Some(((px, py), target)) = self.press {
                // things that can be used or picked up keep their click and drag;
                // an open window is never ground
                let ground = game.pick_gump(px, py).is_none() && target.is_none_or(|id| !game.usable(id) && !game.portable(id));
                if ground && self.press_at.elapsed() >= HOLD_TO_FOLLOW && game.player_in_control() {
                    self.following = true;
                    game.cancel_approach();
                }
            }
        }
        if self.following && (!left || game.fighting() || !game.player_in_control()) {
            self.following = false;
            self.press = None; // letting go only stops him
        }
        self.follow_pointer = if self.following { mouse } else { None };
        if released {
            self.left_released(modern, mouse, now);
        }
        self.left_was_down = left;
        if let Some((at, id)) = self.pending_click {
            if now.duration_since(at) > DOUBLE_CLICK {
                self.game.interact(id, false);
                self.pending_click = None;
            }
        }
    }

    /// The left button went down.
    fn left_pressed(&mut self, modern: bool, mouse: Option<(i32, i32)>, now: Instant) {
        let (window, game) = (&self.window, &mut self.game);
        let (w, h) = (self.surf.width, self.surf.height);
        let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        self.press = None;
        if game.reader_browses() {
            // the journal and the spell book: the left page turns back, the
            // right one on; beside the book the click does nothing
            match mouse.map_or(0, |(x, y)| game.reader_side_at(x, y, w, h)) {
                -1 => game.reader_back(),
                1 => game.reader_next(),
                _ => {}
            }
        } else if game.ui.reader.is_some() {
            // a book, scroll or plaque takes every click
            game.reader_next();
        } else if !modern && window.get_mouse_down(MouseButton::Right) && game.player_in_control() {
            // both buttons: jump to the pointer, as in the original
            game.jump(mouse, w, h);
        } else if game.ui.targeting.is_some() {
            // a script waits for a target: this click picks it
            let target = mouse.and_then(|(x, y)| game.pick(x, y));
            if let Some(id) = target {
                game.target_picked(id);
            }
        } else if game.ask.is_some() {
            match mouse.and_then(|(_, y)| game.answer_at(y, h)) {
                Some(n) => game.answer(n),
                None => {
                    game.skip_barks();
                }
            }
        } else if !game.player_may_order() && game.skip_barks() {
            // a scene is playing: the click ends the speech on screen and
            // does nothing else (people talking on the side, while the
            // player can move, do not take clicks away)
            self.pending_click = None;
        } else if mouse.is_some_and(|(x, y)| game.grid_button(x, y)) {
            // a page arrow of a slot window
            self.pending_click = None;
            self.dirty = true;
        } else if mouse.is_some_and(|(x, y)| game.on_minimap(x, y) && game.pick_gump(x, y).is_none()) {
            // on the minimap: nothing happens in the world below it; with
            // the guide on it opens the large map
            self.pending_click = None;
            if game.options.guide {
                game.open_worldmap();
                self.left_swallowed = true;
                self.dirty = true;
            }
        } else if let Some(held) = mouse.and_then(|(x, y)| game.grab_gump(x, y)) {
            // on a window, beside what is in it: the window is moved
            self.gump_held = Some(held);
            self.pending_click = None;
            self.dirty = true;
        } else if !ctrl && mouse.is_some_and(|(x, y)| game.combat_click(x, y)) {
            // fighting by clicking: going for the enemy or walking (see controls.rs)
            self.held_walk = mouse.map(|(x, y)| (x, y, game.tick));
        } else if let (true, Some((x, y))) = (game.fighting(), mouse) {
            // weapon drawn: a click strikes towards the pointer (with
            // fighting by clicking: ctrl+click)
            let (dx, dy) = (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0);
            game.avatar_attack(Some(game.unturn_dir(screen_dir(dx, 2.0 * dy))), false);
        } else if let Some((x, y)) = mouse {
            self.press = Some(((x, y), game.pick(x, y)));
            self.press_at = now;
        }
    }

    /// While the left button is held (or after it): the wheel, moving a
    /// window, dragging a thing, turning pages under it, walking by clicking.
    fn left_held(&mut self, left: bool, mouse: Option<(i32, i32)>) {
        let (window, game) = (&self.window, &mut self.game);
        // the mouse wheel over a slot window turns its pages
        if let (Some((_, wheel)), Some((x, y))) = (window.get_scroll_wheel(), mouse) {
            if game.ui.reader.is_some() {
                // a book is open: the wheel turns its pages (the journal and
                // the spell book), never those of a window below it
                if wheel != 0.0 && game.reader_browses() {
                    if wheel > 0.0 {
                        game.reader_back();
                    } else {
                        game.reader_on();
                    }
                    self.dirty = true;
                }
            } else if wheel != 0.0 && game.grid_wheel(x, y, if wheel > 0.0 { -1 } else { 1 }) {
                self.dirty = true;
            }
        }
        if let (Some(held), Some((x, y))) = (self.gump_held, mouse) {
            if game.move_gump(held, x, y, (self.surf.width, self.surf.height)) {
                self.dirty = true;
            }
        } else if let (true, Some((x, y))) = (left, mouse) {
            // moving the pointer with the button held picks the item up
            if let (None, Some(((px, py), Some(id)))) = (&game.ui.drag, self.press) {
                if (x - px).abs() + (y - py).abs() > 5 && game.begin_drag(id, x, y) {
                    self.pending_click = None;
                    self.press = None;
                }
            }
            game.drag_to(x, y);
        }
        // a dragged thing held over a page arrow of the backpack turns the
        // page after a moment, and on and on while it stays there
        match (game.ui.drag.is_some(), mouse.and_then(|(x, y)| game.grid_arrow_at(x, y))) {
            (true, Some((owner, dir))) => match self.arrow_hover {
                Some((d, since)) if d == dir => {
                    if since.elapsed() >= Duration::from_millis(500) {
                        game.grid_turn(owner, dir);
                        self.arrow_hover = Some((dir, Instant::now()));
                        self.dirty = true;
                    }
                }
                _ => self.arrow_hover = Some((dir, Instant::now())),
            },
            _ => self.arrow_hover = None,
        }
        // fighting by clicking: with the button held on the ground the
        // Avatar keeps following the pointer
        if !left {
            self.held_walk = None;
            if !self.attack_key_held {
                game.combat_release();
            }
        } else if let (Some((hx, hy, at)), Some((x, y))) = (self.held_walk, mouse) {
            let far = (x - hx).abs() + (y - hy).abs() > 24;
            if game.click_fight.is_none() && game.fighting() && game.tick.wrapping_sub(at) >= 10 && (far || !game.walking()) {
                game.walk_to_pointer(x, y);
                self.held_walk = Some((x, y, game.tick));
            }
        }
    }

    /// The left button was let go: a dragged thing is put down, a click
    /// uses, looks at, walks or waits for a second one.
    fn left_released(&mut self, modern: bool, mouse: Option<(i32, i32)>, now: Instant) {
        let (window, game) = (&self.window, &mut self.game);
        if game.ui.drag.is_some() {
            let (x, y) = mouse.unwrap_or((0, 0));
            let ok = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
                // shift: the whole stack at once
                game.drop_drag(x, y)
            } else {
                // a stack asks how much of it to put down
                game.drop_drag_split(x, y)
            };
            if !ok {
                self.mouse_pointer.deny();
            }
        } else if let Some(((px, py), target)) = self.press.take() {
            // using the backpack on the Avatar's back opens it straight away
            let on_pack = target == Some(AVATAR) && game.backpack_at(px, py);
            match (self.pending_click.take(), target) {
                (_, Some(AVATAR)) if modern && on_pack => game.open_inventory(),
                (Some((_, AVATAR)), Some(AVATAR)) if on_pack => game.open_inventory(),
                // a click on the ground walks there, if that is switched on
                (_, Some(id)) if modern && game.options.click_walk && !game.usable(id) && game.walk_to_pointer(px, py) => {}
                // modern controls: one click uses, without waiting for a second
                (_, Some(id)) if modern => game.use_or_approach(id),
                (Some((_, first)), Some(id)) if first == id => game.interact(id, true),
                (_, Some(id)) => self.pending_click = Some((now, id)),
                _ => {}
            }
        }
    }

    /// Keys for the view and the options that can be switched on the fly.
    fn view_keys(&mut self, modern: bool) {
        let mouse = mouse_pos(&self.window, MouseMode::Clamp).map(|(x, y)| (x as i32, y as i32));
        let (window, game) = (&self.window, &mut self.game);
        // page down / up: the next or the previous map (for looking around)
        let mut switch = 0i32;
        if window.is_key_pressed(Key::PageDown, KeyRepeat::No) {
            switch = 1;
        }
        if window.is_key_pressed(Key::PageUp, KeyRepeat::No) {
            switch = -1;
        }
        if switch != 0 {
            let mut n = game.cur_map as i32;
            loop {
                n = (n + switch).rem_euclid(256);
                if game.map_exists(n as usize) {
                    break;
                }
            }
            jump_to_map(game, n as usize, None);
            self.dirty = true;
        }
        // , and .: turn the view (optional)
        if window.is_key_pressed(Key::Comma, KeyRepeat::No) {
            game.turn_view(-1);
        }
        if window.is_key_pressed(Key::Period, KeyRepeat::No) {
            game.turn_view(1);
        }
        // + and -: zoom in and out
        let zoom_in = window.is_key_pressed(Key::Equal, KeyRepeat::No) || window.is_key_pressed(Key::NumPadPlus, KeyRepeat::No);
        let zoom_out = window.is_key_pressed(Key::Minus, KeyRepeat::No) || window.is_key_pressed(Key::NumPadMinus, KeyRepeat::No);
        if zoom_in || zoom_out {
            let z = game.options.zoom.min(menu::ZOOMS.len() - 1);
            let z = if zoom_in { z.saturating_sub(1) } else { (z + 1).min(menu::ZOOMS.len() - 1) };
            if z != game.options.zoom {
                game.options.zoom = z;
                game.save_settings();
                // as a magnification: 75 % of the pixels is 133 %
                game.notify(&format!("Zoom {} %", 10000 / menu::ZOOMS[z]));
            }
        }
        // L, G, M: lighting, smoothing, minimap on and off (remembered, like
        // the switches in the options)
        if window.is_key_pressed(Key::L, KeyRepeat::No) {
            game.options.lighting = !game.options.lighting;
            let on = game.options.lighting;
            switched(game, on, ["Beleuchtung an", "Beleuchtung aus", "Eclairage oui", "Eclairage non", "Lighting on", "Lighting off"]);
            self.dirty = true;
        }
        if window.is_key_pressed(Key::G, KeyRepeat::No) {
            game.options.pixels = (game.options.pixels + 1) % 3;
            game.save_settings();
            let text = game.tr(match game.options.pixels {
                crate::menu::PIXELS_XBR => ["Grafikfilter: xBR (gegl\u{84}ttet)", "Filtre : xBR (lissage)", "Graphics filter: xBR (smooth)"],
                crate::menu::PIXELS_CRT => ["Grafikfilter: CRT (R\u{94}hrenmonitor)", "Filtre : CRT (\u{82}cran cathodique)", "Graphics filter: CRT (tube monitor)"],
                _ => ["Grafikfilter aus", "Filtre : aucun", "Graphics filter off"],
            });
            game.notify(text);
            self.dirty = true;
        }
        if window.is_key_pressed(Key::M, KeyRepeat::No) {
            game.options.minimap = !game.options.minimap;
            let on = game.options.minimap;
            switched(game, on, ["Minikarte an", "Minikarte aus", "Mini-carte oui", "Mini-carte non", "Minimap on", "Minimap off"]);
            self.dirty = true;
        }
        // O: the large map with the places (option "map with guide")
        if game.options.guide && window.is_key_pressed(Key::O, KeyRepeat::No) {
            game.open_worldmap();
            self.dirty = true;
        }
        // T: the quest journal (with the option on); T again closes it
        if game.options.journal && window.is_key_pressed(Key::T, KeyRepeat::No) {
            if game.ui.reader.is_some() {
                game.close_reader();
            } else {
                game.open_journal();
            }
            self.dirty = true;
        }
        // N: the spell book (with the option on); N again closes it
        if game.options.spell_notes && window.is_key_pressed(Key::N, KeyRepeat::No) {
            if game.ui.reader.is_some() {
                game.close_reader();
            } else {
                game.open_spell_notes();
            }
            self.dirty = true;
        }
        if window.is_key_pressed(Key::R, KeyRepeat::No) {
            self.roofs_always = !self.roofs_always;
            self.dirty = true;
        }
        // cheat: F8 changes the body armour (to see the option "show armour")
        if game.cheat && window.is_key_pressed(Key::F8, KeyRepeat::No) {
            game.cheat_cycle_armour(window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift));
            self.dirty = true;
        }
        // (with the modern controls E uses things; the editor objects move to F11)
        if window.is_key_pressed(if modern { Key::F11 } else { Key::E }, KeyRepeat::No) {
            self.opts.show_editor_items = !self.opts.show_editor_items;
            self.dirty = true;
        }

        // H held: everything one can use or take lights up
        let show_all = modern && window.is_key_down(Key::H);
        if show_all != game.ui.show_all {
            game.ui.show_all = show_all;
            self.dirty = true;
        }
        // the details of a value's progress follow the pointer
        if game.ui.pointer != mouse {
            let before = game.practice_row_at(game.ui.pointer);
            game.ui.pointer = mouse;
            if game.practice_row_at(mouse) != before {
                self.dirty = true;
            }
        }
        let hover_changed = if modern { game.set_hover(mouse) } else { game.ui.hover.take().is_some() };
        if hover_changed {
            self.dirty = true;
        }
    }

    /// Draws the view if anything changed, F3 and F12 pictures, and hands
    /// the picture to the window in step with the screen.
    fn draw_and_show(&mut self, loop_start: Instant) -> Result<(), Box<dyn Error>> {
        let t_draw = Instant::now();
        let game = &mut self.game;
        if self.dirty || game.camera_gliding() || game.ui.notice.is_some() || game.turn.turning() {
            game.draw(&mut self.surf, &mut self.opts, self.roofs_always);
            let a = game.avatar();
            self.window.set_title(&format!("PaganRebuild - map {} - {},{},{}", game.cur_map, a.x, a.y, a.z));
            self.dirty = false;
            // the last pictures drawn (five seconds of movement, less when
            // zoomed out) are kept, to be written out with F3
            let note = format!(
                "time {} ms tick {} map {} avatar {},{},{} view {},{} roofs hidden from {:?} pointer on {:?} controls {}",
                self.started.elapsed().as_millis(), game.tick, game.cur_map, a.x, a.y, a.z, game.view.origin.0, game.view.origin.1, self.opts.z_limit, game.ui.hover,
                if game.options.modern_controls { "modern" } else { "classic" }
            );
            // (at most about 300 MB: fewer pictures when zoomed out; the
            // oldest one's memory is used again, a new 4 MB each time is slow)
            let keep = (300 * 640 * 400 / self.surf.pixels.len().max(1)).clamp(60, 300);
            let mut pixels = Vec::new();
            while self.recent.len() >= keep {
                pixels = self.recent.pop_front().map(|r| r.0).unwrap_or_default();
            }
            pixels.clone_from(&self.surf.pixels);
            self.recent.push_back((pixels, note));
        }
        if self.window.is_key_pressed(Key::F3, KeyRepeat::No) {
            self.save_recent();
        }
        if self.window.is_key_pressed(Key::F12, KeyRepeat::No) {
            self.save_shot();
        }
        if self.window.is_key_pressed(Key::F2, KeyRepeat::No) {
            self.fps = match self.fps {
                Some(_) => None,
                None => Some(Fps::new()),
            };
        }
        let t_present = Instant::now();
        let state = cursor::PointerState { moving_window: self.gump_held.is_some() };
        present(&mut self.window, &self.surf, &mut self.shown, &mut self.upscaled, &mut self.game, &self.mouse_pointer, state)?;
        if let Some(fps) = &self.fps {
            if let Some(font) = &self.game.data.font {
                let ts = self.game.text_scale();
                font.draw_at(&mut self.shown, fps.text(), 4, 4 + font.height_at(ts), &self.game.data.pal, ts);
            }
        }
        let t_vsync = Instant::now();
        let times = self.pacer.refreshes();
        show(&mut self.window, &self.shown, &mut self.upscaled, times)?;
        self.pacer.shown();
        if let Some(fps) = &mut self.fps {
            fps.count(t_vsync - loop_start);
        }
        if let Some(ft) = &mut self.frame_times {
            let times = PictureTimes { start: loop_start, draw: t_draw, present: t_present, hand_over: t_vsync, end: Instant::now() };
            let (surf, window, every) = (&self.surf, &self.window, self.pacer.every());
            ft.add(&times, self.pacer.period(), || format!("view {}x{} window {:?}, every {every} refreshes", surf.width, surf.height, window.display_size()));
        }
        Ok(())
    }

    /// F3, for hunting down errors in the picture: the last pictures as images.
    fn save_recent(&mut self) {
        let dir = Path::new("pagan_frames");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::create_dir_all(dir);
        let mut notes = String::new();
        for (i, (pixels, note)) in self.recent.iter().enumerate() {
            let mut img = Surface::new(self.surf.width, self.surf.height);
            img.pixels = pixels.clone();
            let _ = save_png(&dir.join(format!("frame_{i:03}.png")), &img);
            notes += &format!("frame_{i:03}: {note}\n");
        }
        let _ = std::fs::write(dir.join("frames.txt"), notes);
        println!("saved {} pictures to {}", self.recent.len(), dir.display());
        self.game.notify("pagan_frames");
    }

    /// F12: a picture of the view (never over one of an earlier session).
    fn save_shot(&mut self) {
        self.shot += 1;
        while Path::new(&format!("pagan_{:03}.png", self.shot)).exists() {
            self.shot += 1;
        }
        let name = format!("pagan_{:03}.png", self.shot);
        match save_png(Path::new(&name), &self.surf) {
            Ok(()) => {
                println!("saved {name}");
                self.game.notify(&name);
            }
            Err(e) => eprintln!("screenshot failed: {e}"),
        }
    }
}

/// An option switched with a key: it is remembered and the player is told
/// (`texts`: German on / off, French on / off, English on / off).
fn switched(game: &mut Game, on: bool, texts: [&str; 6]) {
    game.save_settings();
    let lang = game.tr([0, 2, 4]);
    game.notify(texts[lang + if on { 0 } else { 1 }]);
}

/// The gamepad in the world: its buttons press the keys (or mouse buttons)
/// they are given. Returns the mouse buttons held.
fn pad_in_world(window: &Window, bindings: &crate::bindings::Bindings, keys: &mut crate::hash::HashSet<Key>) -> [bool; 2] {
    use crate::bindings::Does;
    let mut buttons = [false; 2];
    for p in crate::platform::Pad::ALL.into_iter().filter(|&p| window.pad_down(p)) {
        match bindings.pad_does(p) {
            Some(Does::Key(k)) => {
                keys.insert(k);
            }
            Some(Does::Mouse(i)) => buttons[i] = true,
            None => {}
        }
    }
    buttons
}

/// The gamepad in menus, books and conversations: A confirms (in a book:
/// turns the page), `back` goes back, the cross and the left stick move.
/// The triggers stay mouse buttons (for the pointer); returns them.
fn pad_in_menus(window: &Window, reading: bool, back: Key, keys: &mut crate::hash::HashSet<Key>) -> [bool; 2] {
    use crate::platform::Pad;
    let ok = if reading { Key::Space } else { Key::Enter };
    let pairs = [
        (Pad::A, ok), (Pad::B, back), (Pad::Start, Key::Escape), (Pad::X, Key::Space),
        (Pad::LB, Key::PageUp), (Pad::RB, Key::PageDown),
        (Pad::Up, Key::Up), (Pad::Down, Key::Down), (Pad::Left, Key::Left), (Pad::Right, Key::Right),
    ];
    keys.extend(pairs.into_iter().filter(|&(p, _)| window.pad_down(p)).map(|(_, k)| k));
    // the left stick as the arrows
    let (sx, sy) = window.stick();
    let stick = [(sx < -0.5, Key::Left), (sx > 0.5, Key::Right), (sy > 0.5, Key::Up), (sy < -0.5, Key::Down)];
    keys.extend(stick.into_iter().filter(|&(on, _)| on).map(|(_, k)| k));
    [window.pad_down(Pad::RT), window.pad_down(Pad::LT)]
}
