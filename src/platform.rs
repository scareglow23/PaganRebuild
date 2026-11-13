//! The window, keyboard, mouse and picture output, for Windows, Linux and
//! macOS alike: winit opens the window and reports the input, wgpu hands
//! the finished pictures to the screen (Vulkan on Windows and Linux, Metal
//! on the Mac, OpenGL where neither is there).
//!
//! The game asks for its input the way it did with minifb: "is this key
//! down", "was it pressed since the last picture", "where is the mouse".
//! Here the window's events are collected and answered from that. The
//! window is not driven by an event loop that calls the game; the game's
//! own loop asks for the events now and then (`pump_app_events`).
//!
//! The window can be resized and maximized, and Alt+Enter switches to the
//! full screen and back. The picture keeps its proportions: it is fitted
//! into the window, with black bars where the window is wider or taller.
//! The game makes its pictures in the fitted size (`display_size`).
//!
//! Every picture goes to the screen whole, in step with its refresh
//! ("Fifo"), so there is no tearing line across it; handing over a picture
//! waits for the next refresh when the previous one is still waiting.

use std::sync::Arc;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::WindowId;

/// The keys the game knows. Letters, digits and signs follow the
/// keyboard layout (Z is the key labelled Z, also on a German keyboard),
/// the others are taken by their place.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Up, Down, Left, Right, PageUp, PageDown, Home, End, Insert, Delete,
    Escape, Enter, Space, Tab, Backspace,
    LeftShift, RightShift, LeftCtrl, RightCtrl, LeftAlt, RightAlt,
    Comma, Period, Minus, Equal, Apostrophe,
    NumPad0, NumPad1, NumPad2, NumPad3, NumPad4, NumPad5, NumPad6, NumPad7, NumPad8, NumPad9,
    NumPadMinus, NumPadPlus, NumPadEnter,
}

/// The buttons of a gamepad, named as on the usual pads (A at the bottom,
/// B right, X left, Y at the top).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Pad {
    A, B, X, Y,
    /// Shoulder buttons and triggers.
    LB, RB, LT, RT,
    Back, Start,
    /// The sticks pressed in.
    LStick, RStick,
    Up, Down, Left, Right,
}

impl Pad {
    pub const ALL: [Pad; 16] = [
        Pad::A, Pad::B, Pad::X, Pad::Y, Pad::LB, Pad::RB, Pad::LT, Pad::RT, Pad::Back, Pad::Start, Pad::LStick, Pad::RStick,
        Pad::Up, Pad::Down, Pad::Left, Pad::Right,
    ];

    fn of(b: gilrs::Button) -> Option<Pad> {
        use gilrs::Button as G;
        Some(match b {
            G::South => Pad::A,
            G::East => Pad::B,
            G::West => Pad::X,
            G::North => Pad::Y,
            G::LeftTrigger => Pad::LB,
            G::RightTrigger => Pad::RB,
            G::LeftTrigger2 => Pad::LT,
            G::RightTrigger2 => Pad::RT,
            G::Select => Pad::Back,
            G::Start => Pad::Start,
            G::LeftThumb => Pad::LStick,
            G::RightThumb => Pad::RStick,
            G::DPadUp => Pad::Up,
            G::DPadDown => Pad::Down,
            G::DPadLeft => Pad::Left,
            G::DPadRight => Pad::Right,
            _ => return None,
        })
    }
}

/// Whether held keys repeating count as new presses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyRepeat {
    Yes,
    No,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What `get_mouse_pos` does when the pointer is outside the window:
/// keep it at the edge (`Clamp`) or report nothing (`Discard`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Clamp,
    Discard,
}

/// What a mouse button did since the last look.
#[derive(Default, Clone, Copy)]
pub struct Edges {
    pub pressed: bool,
    pub released: bool,
}

/// The key a key event is about.
fn key_of(event: &winit::event::KeyEvent) -> Option<Key> {
    use Key::*;
    let PhysicalKey::Code(code) = event.physical_key else { return None };
    // the number pad without NumLock: its arrows are arrows
    if let winit::keyboard::Key::Named(named) = &event.logical_key {
        use winit::keyboard::NamedKey as N;
        let arrow = match named {
            N::ArrowUp => Some(Up),
            N::ArrowDown => Some(Down),
            N::ArrowLeft => Some(Left),
            N::ArrowRight => Some(Right),
            N::PageUp => Some(PageUp),
            N::PageDown => Some(PageDown),
            _ => None,
        };
        if arrow.is_some() {
            return arrow;
        }
    }
    // keys taken by their place (the number pad gives digits as text too)
    let by_place = match code {
        KeyCode::F1 => Some(F1), KeyCode::F2 => Some(F2), KeyCode::F3 => Some(F3), KeyCode::F4 => Some(F4),
        KeyCode::F5 => Some(F5), KeyCode::F6 => Some(F6), KeyCode::F7 => Some(F7), KeyCode::F8 => Some(F8),
        KeyCode::F9 => Some(F9), KeyCode::F10 => Some(F10), KeyCode::F11 => Some(F11), KeyCode::F12 => Some(F12),
        KeyCode::ArrowUp => Some(Up), KeyCode::ArrowDown => Some(Down),
        KeyCode::ArrowLeft => Some(Left), KeyCode::ArrowRight => Some(Right),
        KeyCode::PageUp => Some(PageUp), KeyCode::PageDown => Some(PageDown),
        KeyCode::Home => Some(Home), KeyCode::End => Some(End),
        KeyCode::Insert => Some(Insert), KeyCode::Delete => Some(Delete),
        KeyCode::Escape => Some(Escape), KeyCode::Enter => Some(Enter), KeyCode::Space => Some(Space),
        KeyCode::Tab => Some(Tab), KeyCode::Backspace => Some(Backspace),
        KeyCode::ShiftLeft => Some(LeftShift), KeyCode::ShiftRight => Some(RightShift),
        KeyCode::ControlLeft => Some(LeftCtrl), KeyCode::ControlRight => Some(RightCtrl),
        KeyCode::AltLeft => Some(LeftAlt), KeyCode::AltRight => Some(RightAlt),
        KeyCode::Numpad0 => Some(NumPad0), KeyCode::Numpad1 => Some(NumPad1), KeyCode::Numpad2 => Some(NumPad2),
        KeyCode::Numpad3 => Some(NumPad3), KeyCode::Numpad4 => Some(NumPad4), KeyCode::Numpad5 => Some(NumPad5),
        KeyCode::Numpad6 => Some(NumPad6), KeyCode::Numpad7 => Some(NumPad7), KeyCode::Numpad8 => Some(NumPad8),
        KeyCode::Numpad9 => Some(NumPad9),
        KeyCode::NumpadSubtract => Some(NumPadMinus), KeyCode::NumpadAdd => Some(NumPadPlus),
        KeyCode::NumpadEnter => Some(NumPadEnter),
        _ => None,
    };
    if by_place.is_some() {
        return by_place;
    }
    // letters, digits, signs: what the key says without shift
    let winit::keyboard::Key::Character(s) = event.key_without_modifiers() else { return None };
    let c = s.chars().next()?.to_ascii_lowercase();
    const LETTERS: [Key; 26] = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    const DIGITS: [Key; 10] = [Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9];
    match c {
        'a'..='z' => Some(LETTERS[(c as u8 - b'a') as usize]),
        '0'..='9' => Some(DIGITS[(c as u8 - b'0') as usize]),
        ',' => Some(Comma),
        '.' => Some(Period),
        '-' => Some(Minus),
        // the key right of 0/ß: "=" on English keyboards, "+" on German ones
        '=' | '+' => Some(Equal),
        '\'' => Some(Apostrophe),
        _ => None,
    }
}

/// The graphics card's side: the window's surface, and the picture as a
/// texture drawn into the window.
struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// The picture, smoothed and stretched to the window (gpuscale.rs).
    scaler: crate::gpuscale::Scaler,
}

impl Gpu {
    fn new(window: Arc<winit::window::Window>) -> Result<Gpu, String> {
        // WGPU_BACKEND=vulkan|gl|metal picks one (for tests)
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let surface = instance.create_surface(window.clone()).map_err(|e| format!("no surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or("no graphics adapter")?;
        let info = adapter.get_info();
        eprintln!("graphics: {} through {:?}", info.name, info.backend);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("pagan"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("no device: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        // the pixels are sRGB already: a format that would convert them again is avoided
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: crate::debug::env().latency.unwrap_or(1),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let scaler = crate::gpuscale::Scaler::new(&device, format);
        Ok(Gpu { surface, device, queue, config, scaler })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 && (size.width, size.height) != (self.config.width, self.config.height) {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Takes the picture (0x00RRGGBB per pixel, as the game draws), to be
    /// shown with `look`.
    fn upload(&mut self, pixels: &[u32], w: u32, h: u32, look: crate::gpuscale::Look) {
        self.scaler.set_picture(&self.device, &self.queue, pixels, w, h, look);
    }

    /// Shows the picture in the part `rect` (x, y, width, height) of the
    /// window, black around it; waits if the screen has not taken the one
    /// before yet. False if nothing could be shown.
    fn draw(&mut self, rect: (u32, u32, u32, u32)) -> bool {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            Err(_) => return false,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("picture") });
        let drawn = {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let (x, y, w, h) = rect;
            let (x, y) = (x.min(self.config.width - 1), y.min(self.config.height - 1));
            let (w, h) = (w.clamp(1, self.config.width - x), h.clamp(1, self.config.height - y));
            self.scaler.draw(&self.device, &self.queue, &mut pass, (x, y, w, h))
        };
        if !drawn {
            return false;
        }
        self.queue.submit([enc.finish()]);
        frame.present();
        true
    }
}

/// What the window's events left behind since the last look.
#[derive(Default)]
struct Input {
    down: crate::hash::HashSet<Key>,
    /// Pressed since the last `update`, in order; true: by repeating.
    pressed: Vec<(Key, bool)>,
    /// Position in window pixels, and whether the pointer is over the window.
    mouse: (f64, f64),
    inside: bool,
    buttons: [bool; 3],
    /// Presses and releases of left and right (see `take_clicks`).
    edges: [Edges; 2],
    wheel: (f32, f32),
    /// What the keys typed since the last look (with the keyboard's layout,
    /// shift, dead keys ...).
    typed: String,
    closed: bool,
    /// An Alt key is held (as the system reports it).
    alt: bool,
    /// The keys held and what they were taken for (see `App::key_map`),
    /// so that letting go lets go of the same key.
    held_as: crate::hash::HashMap<Key, Key>,
    /// When the window last got the focus back: a click right after it is
    /// the one that brought the window to the front, not meant for the game.
    focused_at: Option<std::time::Instant>,
    /// Buttons whose press was such a click (their release is let go too).
    swallowed: [bool; 3],
}

/// The part winit calls with the events.
struct App {
    title: String,
    size: PhysicalSize<u32>,
    window: Option<Arc<winit::window::Window>>,
    gpu: Option<Gpu>,
    error: Option<String>,
    input: Input,
    /// Keys that do what another key does (the player's own key bindings);
    /// None: the key does nothing.
    key_map: crate::hash::HashMap<Key, Option<Key>>,
}

impl App {
    /// A key pressed or let go: typed text, the key as the game sees it
    /// (after the key map), Alt+Enter for the full screen.
    fn keyboard_input(&mut self, event: &winit::event::KeyEvent) {
        let input = &mut self.input;
        if event.state == ElementState::Pressed {
            if let Some(t) = &event.text {
                input.typed.extend(t.chars().filter(|c| !c.is_control()));
                if crate::debug::env().keys {
                    eprintln!("typed {t:?}");
                }
            }
        }
        let Some(raw) = key_of(event) else { return };
        let k = match event.state {
            ElementState::Pressed => match self.key_map.get(&raw).copied().unwrap_or(Some(raw)) {
                Some(k) => {
                    input.held_as.insert(raw, k);
                    k
                }
                None => return,
            },
            ElementState::Released => match input.held_as.remove(&raw) {
                Some(k) => k,
                None => return,
            },
        };
        if crate::debug::env().keys {
            eprintln!("key {k:?} {:?} repeat {} alt {}", event.state, event.repeat, input.alt);
        }
        // Alt+Enter: full screen or back (the game does not see this Enter)
        if (k == Key::Enter || k == Key::NumPadEnter) && input.alt {
            if event.state == ElementState::Pressed && !event.repeat {
                if let Some(w) = &self.window {
                    let full = w.fullscreen().is_some();
                    w.set_fullscreen((!full).then_some(winit::window::Fullscreen::Borderless(None)));
                }
            }
            return;
        }
        match event.state {
            ElementState::Pressed => {
                input.down.insert(k);
                input.pressed.push((k, event.repeat));
            }
            ElementState::Released => {
                input.down.remove(&k);
            }
        }
    }

    /// A mouse button pressed or let go. A click that brings the window
    /// back to the front is not passed on.
    fn mouse_button(&mut self, state: ElementState, button: winit::event::MouseButton) {
        let input = &mut self.input;
        let i = match button {
            winit::event::MouseButton::Left => 0,
            winit::event::MouseButton::Right => 1,
            winit::event::MouseButton::Middle => 2,
            _ => return,
        };
        let pressed = state == ElementState::Pressed;
        if pressed {
            let fresh = input.focused_at.take().is_some_and(|t| t.elapsed() < Duration::from_millis(300));
            input.swallowed[i] = fresh;
            if fresh {
                return;
            }
        } else if std::mem::take(&mut input.swallowed[i]) {
            return;
        }
        input.buttons[i] = pressed;
        if i < 2 {
            if pressed {
                input.edges[i].pressed = true;
            } else {
                input.edges[i].released = true;
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = winit::window::Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(self.size)
            .with_min_inner_size(PhysicalSize::new(320, 200))
            .with_resizable(true)
            // PAGAN_FULLSCREEN=1: in the full screen from the start (for tests)
            .with_fullscreen(crate::debug::env().fullscreen.then_some(winit::window::Fullscreen::Borderless(None)));
        match el.create_window(attrs) {
            Ok(w) => {
                let w = Arc::new(w);
                match Gpu::new(w.clone()) {
                    Ok(g) => self.gpu = Some(g),
                    Err(e) => self.error = Some(e),
                }
                self.window = Some(w);
            }
            Err(e) => self.error = Some(format!("no window: {e}")),
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let input = &mut self.input;
        match event {
            WindowEvent::CloseRequested => input.closed = true,
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(size);
                }
            }
            WindowEvent::Focused(false) => {
                // keys let go while another window had the keyboard never come back up
                input.down.clear();
                input.buttons = [false; 3];
                input.focused_at = None;
            }
            WindowEvent::Focused(true) => input.focused_at = Some(std::time::Instant::now()),
            WindowEvent::KeyboardInput { event, .. } => self.keyboard_input(&event),
            WindowEvent::ModifiersChanged(m) => input.alt = m.state().alt_key(),
            WindowEvent::CursorMoved { position, .. } => {
                input.mouse = (position.x, position.y);
                input.inside = true;
            }
            WindowEvent::CursorEntered { .. } => input.inside = true,
            WindowEvent::CursorLeft { .. } => input.inside = false,
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(state, button),
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 40.0, p.y as f32 / 40.0),
                };
                input.wheel.0 += x;
                input.wheel.1 += y;
            }
            _ => {}
        }
    }
}

/// Waits for the desktop's next refresh (Windows). Vulkan under Windows
/// gives the buffers back at uneven moments (the desktop copies the picture
/// rather than flipping to it), so waiting there alone makes one pass take
/// 5 ms and the next 15; pictures handed over right after the desktop's
/// refresh go out evenly. False if there is nothing to wait for.
#[cfg(windows)]
fn desktop_refresh() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    // how often in a row the wait came back at once
    static INSTANT: AtomicU32 = AtomicU32::new(0);
    let start = std::time::Instant::now();
    // SAFETY: a plain call without arguments
    let ok = unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() } == 0;
    // a wait that keeps coming back at once means there is no composition
    // (or the window is hidden): then the swap chain paces alone
    if ok && start.elapsed() < Duration::from_micros(300) {
        return INSTANT.fetch_add(1, Ordering::Relaxed) < 5;
    }
    INSTANT.store(0, Ordering::Relaxed);
    ok
}

#[cfg(not(windows))]
fn desktop_refresh() -> bool {
    false
}

/// The game's window.
pub struct Window {
    events: EventLoop<()>,
    app: App,
    /// The size the game opened the window with; the mouse is reported in
    /// these units (as minifb did), whatever size the window has on screen.
    logical: (usize, usize),
    /// Keys pressed by the last `update` (`pressed` collects the next).
    pressed: Vec<(Key, bool)>,
    /// The text typed by then.
    typed: String,
    wheel: Option<(f32, f32)>,
    /// Wait for the desktop's refresh before handing over (see `desktop_refresh`).
    desktop_pace: bool,
    /// The picture fills the whole window, whatever its proportions (the
    /// game then shows more of the world to the sides or above and below).
    fill: bool,
    /// Gamepads (None if they cannot be used on this system).
    pads: Option<gilrs::Gilrs>,
    /// The pads' buttons pressed (true) and let go (false) since the last
    /// picture, and the ones collected for the next.
    pad_events: Vec<(Pad, bool)>,
    pad_next: Vec<(Pad, bool)>,
    /// Where the sticks are pushed (left, right): x right, y up, -1..1.
    stick: (f32, f32),
    right_stick: (f32, f32),
    /// The pad's buttons held.
    pad_held: crate::hash::HashSet<Pad>,
    /// Keys and mouse buttons held down for the pad (see `set_virtual`).
    virtual_down: crate::hash::HashSet<Key>,
    virtual_buttons: [bool; 2],
}

impl Window {
    /// Opens a window of `w` x `h` screen pixels.
    pub fn new(title: &str, w: usize, h: usize) -> Result<Window, String> {
        let events = EventLoop::new().map_err(|e| format!("no event loop: {e}"))?;
        let app = App {
            title: title.to_string(),
            size: PhysicalSize::new(w as u32, h as u32),
            window: None,
            gpu: None,
            error: None,
            input: Input::default(),
            key_map: Default::default(),
        };
        let desktop_pace = cfg!(windows) && !crate::debug::env().no_dwm;
        let mut win = Window { events, app, logical: (w, h), pressed: Vec::new(), typed: String::new(), wheel: None, desktop_pace, fill: false,
            pads: gilrs::Gilrs::new().ok(), pad_events: Vec::new(), pad_next: Vec::new(), stick: (0.0, 0.0), right_stick: (0.0, 0.0), pad_held: Default::default(), virtual_down: Default::default(), virtual_buttons: [false; 2] };
        // the window is made when the event loop starts
        for _ in 0..100 {
            win.pump();
            if win.app.window.is_some() || win.app.error.is_some() {
                break;
            }
        }
        if let Some(e) = win.app.error.take() {
            return Err(e);
        }
        if win.app.window.is_none() {
            return Err("the window did not open".into());
        }
        Ok(win)
    }

    fn pump(&mut self) {
        if let PumpStatus::Exit(_) = self.events.pump_app_events(Some(Duration::ZERO), &mut self.app) {
            self.app.input.closed = true;
        }
        if let Some(pads) = &mut self.pads {
            while let Some(ev) = pads.next_event() {
                match ev.event {
                    gilrs::EventType::ButtonPressed(b, _) => {
                        if let Some(p) = Pad::of(b) {
                            self.pad_next.push((p, true));
                            self.pad_held.insert(p);
                        }
                    }
                    gilrs::EventType::ButtonReleased(b, _) => {
                        if let Some(p) = Pad::of(b) {
                            self.pad_next.push((p, false));
                            self.pad_held.remove(&p);
                        }
                    }
                    gilrs::EventType::AxisChanged(gilrs::Axis::LeftStickX, v, _) => self.stick.0 = v,
                    gilrs::EventType::AxisChanged(gilrs::Axis::LeftStickY, v, _) => self.stick.1 = v,
                    gilrs::EventType::AxisChanged(gilrs::Axis::RightStickX, v, _) => self.right_stick.0 = v,
                    gilrs::EventType::AxisChanged(gilrs::Axis::RightStickY, v, _) => self.right_stick.1 = v,
                    gilrs::EventType::Disconnected => {
                        self.stick = (0.0, 0.0);
                        self.right_stick = (0.0, 0.0);
                        self.pad_held.clear();
                    }
                    _ => {}
                }
            }
        }
    }

    /// The gamepad buttons pressed (true) and let go (false) since the
    /// picture before.
    pub fn pad_events(&self) -> &[(Pad, bool)] {
        &self.pad_events
    }

    /// The left stick: x right, y up, each -1..1.
    pub fn stick(&self) -> (f32, f32) {
        self.stick
    }

    /// The right stick, the same way.
    pub fn right_stick(&self) -> (f32, f32) {
        self.right_stick
    }

    /// The pad's buttons held, and the ones pressed since the picture
    /// before (a quick press may be over already).
    pub fn pad_down(&self, p: Pad) -> bool {
        self.pad_held.contains(&p) || self.pad_events.contains(&(p, true))
    }

    /// Moves the pointer by window pixels (the pad's right stick).
    pub fn move_pointer(&mut self, dx: f64, dy: f64) {
        let Some(win) = &self.app.window else { return };
        let size = win.inner_size();
        let m = &mut self.app.input.mouse;
        m.0 = (m.0 + dx).clamp(0.0, size.width.saturating_sub(1) as f64);
        m.1 = (m.1 + dy).clamp(0.0, size.height.saturating_sub(1) as f64);
        self.app.input.inside = true;
        let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(m.0, m.1));
    }

    /// The window's size in its pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        self.app.window.as_ref().map_or((1, 1), |w| (w.inner_size().width.max(1), w.inner_size().height.max(1)))
    }

    /// Is this key held by a button of the pad (not by the keyboard)?
    pub fn pad_holds(&self, k: Key) -> bool {
        self.virtual_down.contains(&k)
    }

    /// The keys and mouse buttons (left, right) the pad holds down now.
    /// Keys that come anew count as pressed in this picture, the ones
    /// missing are let go. `repeat`: held keys pressed again (by repeating).
    pub fn set_virtual(&mut self, keys: &crate::hash::HashSet<Key>, buttons: [bool; 2], repeat: &[Key]) {
        let input = &mut self.app.input;
        for &k in keys {
            if self.virtual_down.insert(k) {
                input.down.insert(k);
                self.pressed.push((k, false));
            }
        }
        let gone: Vec<Key> = self.virtual_down.difference(keys).copied().collect();
        for k in gone {
            self.virtual_down.remove(&k);
            // unless the keyboard holds it as well
            if !input.held_as.values().any(|&h| h == k) {
                input.down.remove(&k);
            }
        }
        for &k in repeat {
            if self.virtual_down.contains(&k) {
                self.pressed.push((k, true));
            }
        }
        for i in 0..2 {
            if buttons[i] != self.virtual_buttons[i] {
                self.virtual_buttons[i] = buttons[i];
                input.buttons[i] = buttons[i];
                if buttons[i] {
                    input.edges[i].pressed = true;
                } else {
                    input.edges[i].released = true;
                }
            }
        }
    }

    /// Keys that do what other keys do (the player's own bindings).
    pub fn set_key_map(&mut self, map: crate::hash::HashMap<Key, Option<Key>>) {
        self.app.key_map = map;
    }


    pub fn is_open(&self) -> bool {
        !self.app.input.closed
    }

    /// The screen's refresh time, if the system tells it.
    pub fn refresh_period(&self) -> Option<Duration> {
        let mhz = self.app.window.as_ref()?.current_monitor()?.refresh_rate_millihertz()?;
        (mhz > 0).then(|| Duration::from_secs_f64(1000.0 / mhz as f64))
    }

    pub fn set_title(&mut self, title: &str) {
        if let Some(w) = &self.app.window {
            w.set_title(title);
        }
    }

    /// Where the picture goes in the window: x, y, width, height in screen
    /// pixels, as large as fits with the proportions of the size the window
    /// was opened with.
    fn fit(&self) -> (u32, u32, u32, u32) {
        let Some(win) = &self.app.window else { return (0, 0, self.logical.0 as u32, self.logical.1 as u32) };
        let size = win.inner_size();
        let (sw, sh) = (size.width.max(1) as u64, size.height.max(1) as u64);
        if self.fill {
            return (0, 0, sw as u32, sh as u32);
        }
        let (lw, lh) = (self.logical.0.max(1) as u64, self.logical.1.max(1) as u64);
        // the full width, unless the height is too small for it
        let (w, h) = if sw * lh <= sh * lw { (sw, (sw * lh / lw).max(1)) } else { ((sh * lw / lh).max(1), sh) };
        (((sw - w) / 2) as u32, ((sh - h) / 2) as u32, w as u32, h as u32)
    }

    /// The size the pictures are shown in, in screen pixels (it changes
    /// with the window).
    /// The picture fills the whole window (true) or keeps the proportions
    /// the window was opened with, with black bars beside it (false).
    pub fn set_fill(&mut self, on: bool) {
        self.fill = on;
    }

    /// The size the game draws for, in the units the window was opened
    /// with: that size, or, filling the window, as high as that and as
    /// wide as the window's proportions make it.
    pub fn logical_size(&self) -> (usize, usize) {
        match (&self.app.window, self.fill) {
            (Some(win), true) => {
                let size = win.inner_size();
                let (sw, sh) = (size.width.max(1) as usize, size.height.max(1) as usize);
                ((self.logical.1 * sw / sh).max(1), self.logical.1)
            }
            _ => self.logical,
        }
    }

    pub fn display_size(&self) -> (usize, usize) {
        let (_, _, w, h) = self.fit();
        (w as usize, h as usize)
    }

    pub fn set_cursor_visibility(&mut self, visible: bool) {
        if let Some(w) = &self.app.window {
            w.set_cursor_visible(visible);
        }
    }

    pub fn is_key_down(&self, key: Key) -> bool {
        self.app.input.down.contains(&key)
    }

    /// Pressed since the picture before (with `Yes` also by holding it).
    pub fn is_key_pressed(&self, key: Key, repeat: KeyRepeat) -> bool {
        self.pressed.iter().any(|&(k, rep)| k == key && (repeat == KeyRepeat::Yes || !rep))
    }

    /// The text typed since the picture before.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    pub fn get_keys_pressed(&self, repeat: KeyRepeat) -> Vec<Key> {
        self.pressed.iter().filter(|&&(_, rep)| repeat == KeyRepeat::Yes || !rep).map(|&(k, _)| k).collect()
    }

    pub fn get_mouse_down(&self, button: MouseButton) -> bool {
        self.app.input.buttons[match button {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        }]
    }

    /// The mouse position in the units the window was opened with.
    pub fn get_mouse_pos(&self, mode: MouseMode) -> Option<(f32, f32)> {
        self.app.window.as_ref()?;
        let (fx, fy, fw, fh) = self.fit();
        let (lw, lh) = self.logical_size();
        let (lw, lh) = (lw as f64, lh as f64);
        let x = (self.app.input.mouse.0 - fx as f64) * lw / fw.max(1) as f64;
        let y = (self.app.input.mouse.1 - fy as f64) * lh / fh.max(1) as f64;
        match mode {
            MouseMode::Clamp => Some((x.clamp(0.0, lw - 1.0) as f32, y.clamp(0.0, lh - 1.0) as f32)),
            MouseMode::Discard => {
                (self.app.input.inside && x >= 0.0 && y >= 0.0 && x < lw && y < lh).then_some((x as f32, y as f32))
            }
        }
    }

    /// The wheel's turn since the picture before (up is positive).
    pub fn get_scroll_wheel(&self) -> Option<(f32, f32)> {
        self.wheel
    }

    /// Presses and releases of the left and the right button since the
    /// last call. A short click (a touchpad tap) can go down and up between
    /// two pictures; it is still seen here.
    pub fn take_clicks(&mut self) -> (Edges, Edges) {
        let e = std::mem::take(&mut self.app.input.edges);
        (e[0], e[1])
    }

    /// Takes the events that came in, and shows the last picture again
    /// (so that the pace stays that of the screen).
    pub fn update(&mut self) {
        let rect = self.fit();
        if let Some(g) = &mut self.app.gpu {
            if self.desktop_pace {
                desktop_refresh();
            }
            if !g.draw(rect) {
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        self.events_in();
    }

    /// Shows a picture of `w` x `h` pixels, stretched over the whole
    /// window, with `look` (sharp, smoothed or as on a tube monitor); `times`: for how many screen refreshes (1 or more).
    pub fn update_with_buffer(&mut self, pixels: &[u32], w: usize, h: usize, look: crate::gpuscale::Look, times: u32) -> Result<(), String> {
        if pixels.len() < w * h {
            return Err(format!("picture of {w}x{h} with only {} pixels", pixels.len()));
        }
        let minimized = self.app.window.as_ref().is_none_or(|w| w.is_minimized().unwrap_or(false));
        let rect = self.fit();
        match &mut self.app.gpu {
            Some(g) if !minimized => {
                for i in 0..times.max(1) {
                    if self.desktop_pace {
                        desktop_refresh();
                    }
                    if i == 0 {
                        g.upload(pixels, w as u32, h as u32, look);
                    }
                    if !g.draw(rect) {
                        std::thread::sleep(Duration::from_millis(8));
                    }
                }
            }
            // nothing to show it in: keep the pace anyway
            _ => std::thread::sleep(Duration::from_millis(16)),
        }
        self.events_in();
        Ok(())
    }

    fn events_in(&mut self) {
        self.pump();
        self.pressed = std::mem::take(&mut self.app.input.pressed);
        self.typed = std::mem::take(&mut self.app.input.typed);
        self.pad_events = std::mem::take(&mut self.pad_next);
        let wheel = std::mem::take(&mut self.app.input.wheel);
        self.wheel = (wheel != (0.0, 0.0)).then_some(wheel);
    }
}
