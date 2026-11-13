# Notes for coding agents

PaganRebuild is an engine for Ultima VIII: Pagan written in Rust. It reads the original game data (shapes, maps, usecode scripts, sounds) and runs the game's own scripts in a virtual machine. The program is `PaganRebuild.exe`; the user-facing description is in `README.md`, the project page in `docs/index.html`.

## Layout

- `src/main.rs`: command line (`--help` lists every option and test command), finding the game folder, the user folder.
- Game data: `flex.rs` (FLX archives), `shape.rs`, `palette.rs`, `map.rs`, `gamedata.rs`, `usecode.rs`, `save.rs`, `anim.rs`, `font.rs`, `movie.rs`.
- Running the game: `game.rs` (world state), `vm.rs` (usecode virtual machine), `intrinsics.rs` (engine functions the scripts call), `effects.rs`, `eggs.rs`, `actor.rs`, `path.rs`, `combat.rs`, `magic.rs`, `jump.rs`, `vitals.rs`, `savegame.rs`.
- Picture: `view.rs`, `sorter.rs` (paint order), `turnview.rs`, `lighting.rs`, `shadows.rs`, `silhouette.rs`, `xbr.rs`, `gpuscale.rs`, `upscale.rs`, `display.rs`.
- Window, input and screens: `platform.rs`, `play.rs` (main loop), `controls.rs` (modern controls), `bindings.rs` (key and gamepad assignment), `menu.rs` (options menu), `title.rs`, `slots.rs` (savegame book), `ui.rs`, `gumps.rs`, `inventory.rs`, `drag.rs`.
- Sound: `audio.rs` (effects, speech, MIDI music with a SoundFont), `soundfix.rs`, `album.rs` (album music from Ogg/WAV files).
- Optional helpers: `minimap.rs`, `worldmap.rs`, `guide.rs`, `places.rs`, `journal.rs`, `quests.rs`, `puzzles.rs`, `hover.rs`, `sleep.rs`, `spellnotes.rs` and others; most files start with a `//!` line saying what they do.
- `src/testcmd/`: the test commands of `--do`.
- `data/*.ini`: weapon, armour and monster tables, built into the program with `include_str!`.
- `tests/regress/*.case`, `tests/regress.rs`: regression cases. `tests/journey/*.case`, `tests/journey.rs`: a playthrough of the whole game.
- `tools/bots/`: runs the test bots from many savegames (`bot`: at random; `explore`: as a player new to the game, see `src/testcmd/explore.rs`), collects their findings (`collect.py`) and shows the protocol of one run as a page (`protocol.py`).
- `docs/`: the project page (GitHub Pages, source `main` / `docs`).

## Toolchain

- Rust stable. On Windows the GNU toolchain (`stable-x86_64-pc-windows-gnu`) without MSVC build tools: do not add crates that compile C or C++ code.
- Some crates are pinned in `Cargo.toml` because newer versions do not link with this toolchain; keep the pins.
- `cargo clippy --all-targets` should stay free of warnings. `clippy.toml` forbids the standard `HashMap`/`HashSet` constructors: use `crate::hash::HashMap` with `HashMap::default()`, so that the game runs the same way every time.

## Game data

The original game files are needed to run the program and the tests; they are not in the repository and must never be added, nor music files. The program looks for a folder with `ENGLISH/STATIC/U8SHAPES.FLX` (or `GERMAN`, `FRENCH` with `--lang`) in the folders above it, or uses `--game <dir>`.

Savegames, settings (`settings.ini`, `keys.ini`) and an optional SoundFont live in the user folder: `%APPDATA%\pagan` on Windows (`APPDATA` set in the environment wins everywhere). `PAGAN_SAVE_DIR` moves the savegames. Tests point both at scratch folders; never write to a player's own folder. A crash of the windowed game is written to `crash.log` there (`src/crash.rs`: what failed, where in the code, where the game was); runs with `--screenshot` show it on the console only.

## Running without a window

Every change can be checked without opening a window:

```
PaganRebuild --screenshot out.png --do "t60;look:1;t30"
PaganRebuild --screenshot out.png --do "loadslot:5;t5;zoom:75;inv"
```

`--do` takes test commands separated by `;` (all listed in `--help`); the picture is written after them. `PAGAN_XBR=1` writes the picture with the xBR filter at twice the size.

## Tests

- `cargo test --release`: unit tests and all regression cases (about two minutes; skipped without the game data). One case: `PAGAN_CASE=<name> cargo test --release -- --nocapture`.
- Bots: `bot:<seed>,<ticks>` plays at random, `explore:<seed>,<ticks>` as a player would; both print rule breaches as `BUG ...` and how much of the game they saw (`coverage: ...`). With `PAGAN_BOT_LOG=<file>` the explorer writes a protocol of its game (one JSON event per line: way, choices, answers, what each use changed, state, findings); `python tools/bots/protocol.py <file> <page.html>` shows it.
- The playthrough is ignored by default: `cargo test --release --test journey -- --ignored --nocapture` (long).
- A case file has `# comment`, `args:`, `do:`, `expect:`, `reject:`, `needs:` lines; see the top of `tests/regress.rs`.
- Tests do what a player can do: talk and pick answers (`talk:`, `say:`), use things, walk, wait (`until:`), or start a named event with `event:<id>,<name>`. They do not call script functions by class and offset and do not check script instruction pointers; expectations check what can be seen (positions, frames, flags, messages).
- Code under test that needs files outside the repository (for example the album) is tested with file names as text, not with the files.

## Style

- Comments explain what happens in the game and why the code does it, in plain words.
- Options: a new switch of the options menu goes into `enum Switch` in `src/menu.rs` with its key in `settings.ini`, its names in German, French and English, and its tab in `TAB_SWITCHES`. Text shown with the game's fonts uses the DOS code page (for example `\u{84}` for "ä").
- The album credit must stay word for word, in `src/title.rs`, `docs/index.html` and `README.md`:
  "Original music: Nenad Vugrinec" / "Arrangement and orchestration: Dustin Naegel" and https://dustinnaegel.bandcamp.com/album/ultima-viii-pagan-orchestral-re-arrangement

## License

GPL-3.0-or-later (`LICENSE`). Parts are ported from ScummVM's Ultima 8 engine (formerly Pentagram); keep their attribution in the code.
