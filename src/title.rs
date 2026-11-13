//! What the game shows outside of the world: the main menu on the open
//! book with the pentagram, the entry of the Avatar's name, the movies
//! and the credits.

use crate::game::Game;
use crate::movie::Movie;
use crate::slots::Mode;
use crate::view::Surface;
use std::time::Instant;

const BOOK_SHAPE: u32 = 35;
const LOGO_SHAPE: u32 = 32;
/// The menu entries as graphics: for each language eight entries, each
/// plain and highlighted.
const ENTRY_SHAPE: u32 = 37;
const ENTRY_HEIGHT: i32 = 14;
const NAME_LENGTH: usize = 15;
/// The name page: where its text begins (right of the fold, from the
/// book's left edge) and how wide the name may be with its cursor, so
/// that it stays on the page.
const NAME_X: i32 = 160;
const NAME_WIDTH: i32 = 104;
/// The font of the name being written: brighter than the text.
const NAME_FONT: usize = 8;
/// The music to the credits: the theme.
const CREDITS_MUSIC: i32 = 51;
/// How fast the credits roll, in pixels per second.
const ROLL_SPEED: f32 = 24.0;

// the menu entries, in the order of the graphics
const INTRO: usize = 0;
const READ_DIARY: usize = 1;
const WRITE_DIARY: usize = 2;
const OPTIONS: usize = 3;
const CREDITS: usize = 4;
const QUIT: usize = 5;
const QUOTES: usize = 6;
const END_GAME: usize = 7;

/// Where a movie leads to.
#[derive(Clone, Copy, PartialEq)]
pub enum Next {
    Menu,
    Name,
    /// The credits roll (after the last film of the game).
    Credits,
}

pub enum Title {
    Menu { selected: usize },
    /// The player types the Avatar's name.
    Name { text: String },
    /// `music`: the piece to go back to afterwards.
    Movie { movie: Box<Movie>, next: Next, music: i32 },
    /// Rolling text: (style, line), and when it started.
    Credits { lines: Vec<(char, String)>, start: Instant },
}

/// What the player did since the last frame.
#[derive(Default)]
pub struct Input {
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub esc: bool,
    pub backspace: bool,
    /// Characters typed.
    pub chars: Vec<char>,
    pub mouse: Option<(i32, i32)>,
    /// The left button went down.
    pub click: bool,
}

/// The credits and quotes are stored lightly scrambled.
fn unscramble(data: &[u8]) -> String {
    data.iter()
        .enumerate()
        .map(|(i, &c)| {
            let x = match i {
                0 | 1 => 0,
                2 => 0xE1,
                _ => 0x20 * (i + 1) + (i >> 1) + (i % 0x40) * ((i & 0xC0) >> 6) * 0x40,
            };
            match c ^ x as u8 {
                0 => '\n',
                d => d as char,
            }
        })
        .collect()
}

impl Game {
    fn saves_exist(&self) -> bool {
        (0..=9).any(|slot| self.slot_info(slot).is_some())
    }

    /// What the game opens with: the introduction for a first start,
    /// otherwise the main menu.
    pub fn start_title(&mut self) {
        self.started = false;
        if self.saves_exist() || !self.play_movie("INTRO.SKF", Next::Name) {
            self.ui.title = Some(Title::Menu { selected: READ_DIARY });
        }
    }

    pub fn open_main_menu(&mut self) {
        self.ui.title = Some(Title::Menu { selected: if self.started { WRITE_DIARY } else { READ_DIARY } });
    }

    pub fn enter_name(&mut self) {
        self.ui.title = Some(Title::Name { text: String::new() });
    }

    /// A file of the language (GINTRO.SKF), or the English one.
    pub(crate) fn language_file(&self, name: &str) -> Option<std::path::PathBuf> {
        [format!("{}{name}", self.language), format!("E{name}"), name.to_string()].iter().map(|n| self.data.static_dir.join(n)).find(|p| p.is_file())
    }

    /// Starts a movie; false if the file is not there.
    pub fn play_movie(&mut self, name: &str, next: Next) -> bool {
        let Some(movie) = self.language_file(name).and_then(|p| Movie::open(&p)) else { return false };
        let music = self.audio.requested();
        self.audio.stop_all_sfx();
        self.ui.title = Some(Title::Movie { movie: Box::new(movie), next, music });
        true
    }

    pub fn open_credits(&mut self) {
        // the theme plays to the credits (as in ScummVM)
        self.audio.play_music(CREDITS_MUSIC);
        self.roll_text("CREDITS.DAT");
    }

    fn roll_text(&mut self, name: &str) {
        let Some(data) = self.language_file(name).and_then(|p| std::fs::read(p).ok()) else { return };
        // Each line can hold several pieces, divided by '*'; a piece starts
        // with a mark for its kind: '+' heading, '&' name, '}' what they did.
        let mut lines = Vec::new();
        for line in unscramble(&data).lines() {
            for piece in line.split('*') {
                let style = piece.chars().next().filter(|c| "+&}@".contains(*c)).unwrap_or(' ');
                let text = piece.trim_start_matches(['+', '&', '}', '@']).trim();
                let font = self.font_for(style);
                match font.filter(|_| !text.is_empty()) {
                    // the heading font only has capitals
                    Some(f) if style == '+' => lines.extend(f.wrap(&text.to_uppercase(), 300).into_iter().map(|l| (style, l))),
                    Some(f) => lines.extend(f.wrap(text, 300).into_iter().map(|l| (style, l))),
                    None => lines.push((' ', String::new())),
                }
            }
        }
        if name == "CREDITS.DAT" && self.audio.has_album() {
            lines.extend(self.album_credits());
        }
        self.ui.title = Some(Title::Credits { lines, start: Instant::now() });
    }

    /// After the game's credits, the album the music comes from, worded as
    /// its authors asked.
    fn album_credits(&self) -> Vec<(char, String)> {
        let mut lines = vec![(' ', String::new()); 3];
        for (style, text) in [
            ('+', "ULTIMA VIII: PAGAN"),
            ('+', "ORCHESTRAL RE-ARRANGEMENT"),
            (' ', ""),
            ('&', "Original music: Nenad Vugrinec"),
            ('&', "Arrangement and orchestration: Dustin Naegel"),
            (' ', ""),
            ('}', "dustinnaegel.bandcamp.com"),
        ] {
            match self.font_for(style).filter(|_| !text.is_empty()) {
                Some(f) => lines.extend(f.wrap(text, 300).into_iter().map(|l| (style, l))),
                None => lines.push((' ', String::new())),
            }
        }
        lines
    }

    /// Rolls the credits on until their last line is in the middle, for tests.
    pub fn roll_credits_to_end(&mut self, height: i32) {
        if let Some(Title::Credits { lines, start }) = &mut self.ui.title {
            let t = (height / 2 + 14 * lines.len() as i32) as f32 / ROLL_SPEED;
            *start = Instant::now() - std::time::Duration::from_secs_f32(t);
        }
    }

    fn font_for(&self, style: char) -> Option<&crate::font::Font> {
        let n = match style {
            '+' => 10,
            '&' => 8,
            _ => 0,
        };
        self.data.font_or_plain(n)
    }

    /// The entries the menu shows: the last two only once the end of the
    /// game has been seen (as in the original), or with the cheat mode.
    fn menu_entries(&self) -> Vec<usize> {
        let all = self.cheat || self.options.seen_end;
        (0..8).filter(|&i| all || (i != QUOTES && i != END_GAME)).collect()
    }

    fn book_origin(&mut self, surf_w: i32, surf_h: i32) -> (i32, i32) {
        let (w, h) = self.data.gump_shapes.get(BOOK_SHAPE).and_then(|s| s.frames.first().map(|f| (f.width, f.height))).unwrap_or((307, 158));
        ((surf_w - w) / 2, (surf_h - h) / 2)
    }

    /// Top left corner of the menu entry in row `row`.
    fn entry_origin(&mut self, row: usize, surf_w: i32, surf_h: i32) -> (i32, i32) {
        let (bx, by) = self.book_origin(surf_w, surf_h);
        (bx + 307 / 2 + 14, by + 18 + row as i32 * ENTRY_HEIGHT)
    }

    /// The row of the main menu (of `rows`) at a screen position.
    fn entry_at(&mut self, (x, y): (i32, i32), rows: usize, surf_w: i32, surf_h: i32) -> Option<usize> {
        (0..rows).rfind(|&r| {
            let (ex, ey) = self.entry_origin(r, surf_w, surf_h);
            x >= ex && x < ex + 100 && y >= ey && y < ey + ENTRY_HEIGHT
        })
    }

    fn entry_frame(&self, entry: usize, lit: bool) -> usize {
        let language = match self.language {
            'F' => 16,
            'G' => 32,
            _ => 0,
        };
        language + entry * 2 + lit as usize
    }

    fn choose(&mut self, entry: usize) {
        match entry {
            INTRO => {
                self.play_movie("INTRO.SKF", Next::Menu);
            }
            READ_DIARY => {
                self.ui.title = None;
                self.open_slot_menu(Mode::Load);
            }
            WRITE_DIARY => {
                // nothing to write down before the game has begun
                if self.started {
                    self.ui.title = None;
                    self.open_slot_menu(Mode::Save);
                }
            }
            OPTIONS => {
                self.ui.title = None;
                self.toggle_menu();
            }
            CREDITS => self.roll_text("CREDITS.DAT"),
            QUIT => self.quit = true,
            QUOTES => self.roll_text("QUOTES.DAT"),
            _ => {
                self.play_movie("ENDGAME.SKF", Next::Menu);
            }
        }
    }

    /// Leaves a movie or the credits.
    fn title_back(&mut self) {
        let Some(Title::Movie { next, music, .. }) = self.ui.title.take() else {
            self.open_main_menu();
            return;
        };
        self.audio.stop_all_sfx();
        self.audio.play_music(music);
        match next {
            Next::Menu => self.open_main_menu(),
            Next::Name => self.enter_name(),
            Next::Credits => self.open_credits(),
        }
    }

    pub fn title_input(&mut self, input: &Input, surf_w: i32, surf_h: i32) {
        let entries = self.menu_entries();
        match &mut self.ui.title {
            Some(Title::Menu { selected }) => {
                let mut row = entries.iter().position(|e| e == selected).unwrap_or(0);
                let count = entries.len();
                if input.up {
                    row = (row + count - 1) % count;
                }
                if input.down {
                    row = (row + 1) % count;
                }
                *selected = entries[row];
                let mut chosen = input.enter.then_some(entries[row]);
                // the entries are numbered
                for c in &input.chars {
                    if let Some(n) = c.to_digit(10).filter(|n| (1..=count as u32).contains(n)) {
                        chosen = Some(entries[n as usize - 1]);
                    }
                }
                // the entry under the pointer is selected, a click chooses it
                let hovered = input.mouse.and_then(|at| self.entry_at(at, entries.len(), surf_w, surf_h)).map(|r| entries[r]);
                if let Some(e) = hovered {
                    if let Some(Title::Menu { selected }) = &mut self.ui.title {
                        *selected = e;
                    }
                    if input.click {
                        chosen = Some(e);
                    }
                }
                if let Some(e) = chosen {
                    self.choose(e);
                } else if input.esc {
                    // back into the game; a new one begins with the name
                    if self.started {
                        self.ui.title = None;
                    } else {
                        self.enter_name();
                    }
                }
            }
            Some(Title::Name { text }) => {
                let font = self.data.font_or_plain(NAME_FONT);
                for &c in &input.chars {
                    let fits = font.is_none_or(|f| f.width(&format!("{text}{c}_")) <= NAME_WIDTH);
                    if fits && text.chars().count() < NAME_LENGTH && (c != ' ' || !text.is_empty()) {
                        text.push(c);
                    }
                }
                if input.backspace {
                    text.pop();
                }
                if input.enter {
                    let name = text.trim().to_string();
                    self.avatar_name = if name.is_empty() { "Avatar".to_string() } else { name };
                    self.started = true;
                    self.ui.title = None;
                } else if input.esc {
                    self.open_main_menu();
                }
            }
            Some(Title::Movie { .. } | Title::Credits { .. }) => {
                if input.esc || input.enter || input.click || input.chars.contains(&' ') {
                    self.title_back();
                }
            }
            None => {}
        }
    }

    /// Lets a movie or the credits go on.
    pub fn title_tick(&mut self) {
        let over = match &mut self.ui.title {
            Some(Title::Movie { movie, .. }) => {
                movie.run(&mut self.audio);
                movie.done
            }
            Some(Title::Credits { lines, start }) => {
                let height = lines.len() as i32 * 14;
                start.elapsed().as_secs_f32() * ROLL_SPEED > (height + self.view.size.1 + 20) as f32
            }
            _ => false,
        };
        if over {
            self.title_back();
        }
    }

    /// The movie on screen, for tests.
    pub fn movie_mut(&mut self) -> Option<(&mut Movie, &mut crate::audio::Audio)> {
        match &mut self.ui.title {
            Some(Title::Movie { movie, .. }) => Some((movie, &mut self.audio)),
            _ => None,
        }
    }

    /// Does the title cover the whole view?
    pub fn title_covers_view(&self) -> bool {
        !self.started || matches!(self.ui.title, Some(Title::Movie { .. } | Title::Credits { .. }))
    }

    fn draw_book(&mut self, surf: &mut Surface) -> (i32, i32) {
        // the world dims behind the book
        surf.dim();
        let (bx, by) = self.book_origin(surf.width, surf.height);
        if let Some(f) = self.data.gump_shapes.get(BOOK_SHAPE).as_ref().and_then(|s| s.frames.first()) {
            surf.paint(f, bx + f.xoff, by + f.yoff, false, false, &self.data.pal);
        }
        if let Some(f) = self.data.gump_shapes.get(LOGO_SHAPE).as_ref().and_then(|s| s.frames.first()) {
            surf.paint(f, bx + 42 + f.xoff, by + 10 + f.yoff, false, false, &self.data.pal);
        }
        (bx, by)
    }

    pub fn draw_title(&mut self, surf: &mut Surface) {
        match &self.ui.title {
            None => {}
            Some(Title::Menu { selected }) => {
                let selected = *selected;
                self.draw_book(surf);
                let entries = self.menu_entries();
                let shape = self.data.gump_shapes.get(ENTRY_SHAPE);
                for (row, &e) in entries.iter().enumerate() {
                    let (ex, ey) = self.entry_origin(row, surf.width, surf.height);
                    let dim = e == WRITE_DIARY && !self.started;
                    if let Some(f) = shape.as_ref().and_then(|s| s.frames.get(self.entry_frame(e, e == selected && !dim))) {
                        if dim {
                            surf.paint_faint(f, ex + f.xoff, ey + f.yoff, false, &self.data.pal);
                        } else {
                            surf.paint(f, ex + f.xoff, ey + f.yoff, false, false, &self.data.pal);
                        }
                    }
                }
            }
            Some(Title::Name { text }) => {
                let text = text.clone();
                let (bx, by) = self.draw_book(surf);
                let (label, hint) = self.tr([
                    ("Euer Name:", "Enter = weiter"),
                    ("Votre nom ?", "Entr\u{82}e : confirmer"),
                    ("What is your name?", "Enter: confirm"),
                ]);
                let x = bx + NAME_X;
                if let Some(font) = &self.data.font {
                    font.draw(surf, label, x, by + 22, &self.data.pal);
                    font.draw(surf, hint, x, by + 110, &self.data.pal);
                }
                // the name in the brighter font, with a cursor that blinks
                let cursor = if (self.view.blink_start.elapsed().as_millis() / 400).is_multiple_of(2) { "_" } else { "" };
                if let Some(font) = self.data.font_or_plain(NAME_FONT) {
                    font.draw(surf, &format!("{text}{cursor}"), x, by + 50, &self.data.pal);
                }
            }
            Some(Title::Movie { movie, .. }) => {
                let font = self.data.font_or_plain(6);
                movie.draw(surf, font, &self.data.pal);
            }
            Some(Title::Credits { lines, start }) => {
                surf.pixels.fill(0);
                let mut y = surf.height - (start.elapsed().as_secs_f32() * ROLL_SPEED) as i32;
                for (style, line) in lines {
                    if y > -20 && y < surf.height {
                        if let Some(font) = self.font_for(*style) {
                            font.draw(surf, line, (surf.width - font.width(line)) / 2, y, &self.data.pal);
                        }
                    }
                    y += 14;
                }
            }
        }
    }

    /// What is on screen, for tests.
    pub fn describe_title(&self) -> String {
        match &self.ui.title {
            None => "none".into(),
            Some(Title::Menu { selected }) => format!("main menu, entry {selected}"),
            Some(Title::Name { text }) => format!("name entry: '{text}'"),
            Some(Title::Movie { movie, .. }) => format!("movie: {}", movie.describe()),
            Some(Title::Credits { lines, .. }) => format!("credits: {} lines", lines.len()),
        }
    }
}
