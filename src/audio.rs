//! Sound effects, speech and music.
//!
//! Effects (SOUND.FLX) and speech (E44.FLX ...) are 8 bit mono samples in
//! the "Sonarc" compression; the decoder follows ScummVM's
//! SonarcAudioSample ((C) the ScummVM Team, GPL-3.0-or-later).  Music (MUSIC.FLX) is XMIDI and
//! is played through the Windows MIDI synthesizer, or (option "better
//! music") through our own synthesizer with a SoundFont.  With the option
//! "spatial sound" effects come from the side their source is on and
//! follow its distance while they play.

use crate::flex::{u32le, Flex};
use crate::hash::HashMap;
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::path::{Path, PathBuf};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---- Sonarc decoder ------------------------------------------------------

/// Number of consecutive one bits at the low end of each byte value.
fn one_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, e) in t.iter_mut().enumerate() {
        *e = (i as u8).trailing_ones() as u8;
    }
    t
}

/// Entropy decoding of one frame into prediction errors.
fn decode_ec(mut mode: i32, count: usize, src: &[u8], dest: &mut Vec<u8>, ones: &[u8; 256]) {
    let mut zero_special = false;
    let mut data: u32 = 0;
    let mut bits: i32 = 0;
    let mut pos = 0;
    if mode >= 7 {
        mode -= 7;
        zero_special = true;
    }
    for _ in 0..count {
        while pos < src.len() && bits <= 24 {
            data |= (src[pos] as u32) << bits;
            pos += 1;
            bits += 8;
        }
        if zero_special && data & 1 == 0 {
            dest.push(0x80);
            data >>= 1;
            bits -= 1;
            continue;
        }
        if zero_special {
            data >>= 1;
            bits -= 1;
        }
        let n = ones[(data & 0xFF) as usize] as i32;
        if n == 0 {
            data >>= 1;
            // the low byte holds mode+1 bits of the sample
            let s = (((data & 0xFF) as u8) << (7 - mode)) as i8 >> (7 - mode);
            dest.push((s as u8).wrapping_add(0x80));
            data >>= mode + 1;
            bits -= mode + 2;
        } else if n < 7 - mode {
            data >>= n + 1;
            let mut s = ((((data & 0xFF) as u8) << (7 - mode - n)) & 0x7F) as i8;
            if s & 0x40 == 0 {
                s = (s as u8 | 0x80) as i8;
            }
            s >>= 7 - mode - n;
            dest.push((s as u8).wrapping_add(0x80));
            data >>= mode + n;
            bits -= mode + 2 * n + 1;
        } else {
            data >>= 7 - mode;
            let mut s = ((data & 0xFF) as u8 & 0x7F) as i8;
            if s & 0x40 == 0 {
                s = (s as u8 | 0x80) as i8;
            }
            dest.push((s as u8).wrapping_add(0x80));
            data >>= 7;
            bits -= 2 * 7 - mode;
        }
    }
}

/// Linear prediction: turns the errors of a frame into samples in place.
fn decode_lpc(order: usize, frame: &mut [u8], factors: &[u8]) {
    for i in 0..frame.len() {
        let mut accum: i32 = 0;
        for j in (0..order).rev() {
            // the `order` samples before i, oldest first; silence before the frame
            let k = i as isize - order as isize + (order - 1 - j) as isize;
            let v = if k < 0 { 0u8 } else { frame[k as usize] };
            let val1 = (v ^ 0x80) as i8 as i32;
            let val2 = i16::from_le_bytes([factors[j * 2], factors[j * 2 + 1]]) as i32;
            accum += val1 * val2;
        }
        accum += 0x800;
        frame[i] = frame[i].wrapping_sub(((accum >> 12) & 0xFF) as u8);
    }
}

/// Decodes a Sonarc sample into unsigned 8 bit PCM. Returns (rate, samples).
pub fn decode_sonarc(d: &[u8]) -> Option<(u32, Vec<u8>)> {
    if d.len() < 0x24 {
        return None;
    }
    let length = u32le(d, 0) as usize;
    let rate = u16::from_le_bytes([d[4], d[5]]) as u32;
    if rate < 2000 || length > 16 << 20 {
        return None;
    }
    let mut pos = 0x20;
    if u16::from_le_bytes([d[pos], d[pos + 1]]) == 0x20 && length > 32767 {
        pos += 0x100;
    }
    let ones = one_table();
    let mut out = Vec::with_capacity(length);
    while pos + 8 <= d.len() && out.len() < length {
        let size = u16::from_le_bytes([d[pos], d[pos + 1]]) as usize;
        let count = u16::from_le_bytes([d[pos + 2], d[pos + 3]]) as usize;
        if size < 8 || pos + size > d.len() {
            break;
        }
        let frame = &d[pos..pos + size];
        let checksum = frame.chunks_exact(2).fold(0u16, |c, w| c ^ u16::from_le_bytes([w[0], w[1]]));
        if checksum != 0xACED {
            break;
        }
        let order = frame[7] as usize;
        let mode = frame[6] as i32 - 8;
        if 8 + 2 * order > size || !(0..14).contains(&mode) {
            break;
        }
        let start = out.len();
        decode_ec(mode, count, &frame[8 + 2 * order..], &mut out, &ones);
        decode_lpc(order, &mut out[start..], &frame[8..8 + 2 * order]);
        // fix clipped samples
        for i in start + 1..out.len() {
            if out[i] == 0 && out[i - 1] > 192 {
                out[i] = 0xFF;
            }
        }
        pos += size;
    }
    (!out.is_empty()).then_some((rate, out))
}

fn to_source(rate: u32, pcm: &[u8]) -> SamplesBuffer<i16> {
    SamplesBuffer::new(1, rate, pcm.iter().map(|&b| ((b as i16) - 128) << 8).collect::<Vec<i16>>())
}

/// A sample as it is played: as it is, or made anew at 44.1 kHz (option
/// "better sound", see soundfix.rs).
fn sample_source(better: bool, rate: u32, pcm: &[u8]) -> SamplesBuffer<i16> {
    if better {
        SamplesBuffer::new(1, crate::soundfix::RATE, crate::soundfix::enhance(rate, pcm))
    } else {
        to_source(rate, pcm)
    }
}

// ---- speech ------------------------------------------------------------

/// The recorded lines of one character.
struct Speech {
    flex: Flex,
    /// Text of each sample; sample n is flex entry n + 1.
    phrases: Vec<String>,
}

impl Speech {
    fn open(path: &Path) -> Option<Speech> {
        let flex = Flex::open(path).ok()?;
        let index = flex.get(0)?;
        let phrases = index
            .split(|&b| b == 0)
            .map(|p| p.iter().map(|&b| if b == b'\t' { ' ' } else { b as char }).collect::<String>().trim().to_string())
            .collect();
        Some(Speech { flex, phrases })
    }

    /// The samples that together speak `text`, in order.
    fn samples_for(&self, text: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut rest = text.replace('\t', " ");
        loop {
            let t = rest.trim().to_string();
            if t.is_empty() {
                break;
            }
            let lower = t.to_lowercase();
            let hit = self.phrases.iter().position(|p| !p.is_empty() && lower.starts_with(&p.to_lowercase()));
            match hit {
                Some(i) => {
                    out.push(i + 1);
                    let cut = self.phrases[i].len().min(t.len());
                    rest = t.get(cut..).unwrap_or("").to_string();
                }
                None => break,
            }
        }
        out
    }
}

// ---- music ---------------------------------------------------------------

/// The MIDI synthesizer of the system: Windows and macOS have one that every
/// machine can play on; elsewhere the music always goes through the
/// SoundFont synthesizer.
#[cfg(windows)]
mod system_midi {
    use windows_sys::Win32::Media::Audio::{midiOutClose, midiOutOpen, midiOutReset, midiOutShortMsg};
    const MIDI_MAPPER: u32 = 0xFFFF_FFFF;
    pub fn open() -> Option<isize> {
        let mut handle: isize = 0;
        // SAFETY: plain call, the handle is written into the local
        (unsafe { midiOutOpen(&mut handle, MIDI_MAPPER, 0, 0, 0) } == 0).then_some(handle)
    }
    pub fn send(h: isize, msg: u32) {
        // SAFETY: h came from midiOutOpen and is not closed yet
        unsafe { midiOutShortMsg(h, msg) };
    }
    pub fn reset(h: isize) {
        // SAFETY: as above
        unsafe { midiOutReset(h) };
    }
    pub fn close(h: isize) {
        // SAFETY: as above; the handle is not used afterwards
        unsafe { midiOutClose(h) };
    }
}

/// The MIDI synthesizer built into macOS (Apple DLS Synth), played
/// through the default audio output.
#[cfg(target_os = "macos")]
mod system_midi {
    use std::ffi::c_void;

    type OSStatus = i32;
    type AudioComponent = *mut c_void;
    type AudioUnit = *mut c_void;

    #[repr(C)]
    struct AudioComponentDescription {
        component_type: u32,
        component_sub_type: u32,
        component_manufacturer: u32,
        component_flags: u32,
        component_flags_mask: u32,
    }

    #[repr(C)]
    struct AudioUnitConnection {
        source_audio_unit: AudioUnit,
        source_output_number: u32,
        dest_input_number: u32,
    }

    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioComponentFindNext(component: AudioComponent, desc: *const AudioComponentDescription) -> AudioComponent;
        fn AudioComponentInstanceNew(component: AudioComponent, out: *mut AudioUnit) -> OSStatus;
        fn AudioComponentInstanceDispose(unit: AudioUnit) -> OSStatus;
        fn AudioUnitSetProperty(unit: AudioUnit, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> OSStatus;
        fn AudioUnitInitialize(unit: AudioUnit) -> OSStatus;
        fn AudioUnitUninitialize(unit: AudioUnit) -> OSStatus;
        fn AudioOutputUnitStart(unit: AudioUnit) -> OSStatus;
        fn AudioOutputUnitStop(unit: AudioUnit) -> OSStatus;
        fn MusicDeviceMIDIEvent(unit: AudioUnit, status: u32, data1: u32, data2: u32, offset: u32) -> OSStatus;
    }

    const fn code(c: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*c)
    }
    const PROPERTY_MAKE_CONNECTION: u32 = 1;
    const SCOPE_INPUT: u32 = 1;

    /// The synthesizer and the output it plays into.
    struct Units {
        synth: AudioUnit,
        output: AudioUnit,
    }

    fn new_unit(kind: &[u8; 4], sub: &[u8; 4]) -> Option<AudioUnit> {
        let desc = AudioComponentDescription {
            component_type: code(kind),
            component_sub_type: code(sub),
            component_manufacturer: code(b"appl"),
            component_flags: 0,
            component_flags_mask: 0,
        };
        // SAFETY: desc lives for the call; the unit is written into the local
        unsafe {
            let comp = AudioComponentFindNext(std::ptr::null_mut(), &desc);
            if comp.is_null() {
                return None;
            }
            let mut unit: AudioUnit = std::ptr::null_mut();
            (AudioComponentInstanceNew(comp, &mut unit) == 0 && !unit.is_null()).then_some(unit)
        }
    }

    fn dispose(u: &Units) {
        // SAFETY: both units came from AudioComponentInstanceNew; stopping or
        // uninitializing a unit that never started is harmless
        unsafe {
            AudioOutputUnitStop(u.output);
            AudioUnitUninitialize(u.output);
            AudioUnitUninitialize(u.synth);
            AudioComponentInstanceDispose(u.output);
            AudioComponentInstanceDispose(u.synth);
        }
    }

    pub fn open() -> Option<isize> {
        let synth = new_unit(b"aumu", b"dls ")?;
        let Some(output) = new_unit(b"auou", b"def ") else {
            // SAFETY: synth came from AudioComponentInstanceNew
            unsafe { AudioComponentInstanceDispose(synth) };
            return None;
        };
        let units = Units { synth, output };
        let conn = AudioUnitConnection { source_audio_unit: synth, source_output_number: 0, dest_input_number: 0 };
        // SAFETY: valid units; conn lives for the call
        let ok = unsafe {
            AudioUnitSetProperty(
                output,
                PROPERTY_MAKE_CONNECTION,
                SCOPE_INPUT,
                0,
                (&conn as *const AudioUnitConnection).cast(),
                std::mem::size_of::<AudioUnitConnection>() as u32,
            ) == 0
                && AudioUnitInitialize(synth) == 0
                && AudioUnitInitialize(output) == 0
                && AudioOutputUnitStart(output) == 0
        };
        if !ok {
            dispose(&units);
            return None;
        }
        Some(Box::into_raw(Box::new(units)) as isize)
    }

    /// The units behind a handle from `open`.
    fn units<'a>(h: isize) -> &'a Units {
        // SAFETY: h came from Box::into_raw in open and is freed only in close
        unsafe { &*(h as *const Units) }
    }

    pub fn send(h: isize, msg: u32) {
        let (status, d1, d2) = (msg & 0xFF, (msg >> 8) & 0x7F, (msg >> 16) & 0x7F);
        // SAFETY: a valid synthesizer unit
        unsafe { MusicDeviceMIDIEvent(units(h).synth, status, d1, d2, 0) };
    }

    /// Like midiOutReset: all notes off and controllers back on every channel.
    pub fn reset(h: isize) {
        for ch in 0..16 {
            for cc in [123, 120, 121] {
                send(h, 0xB0 | ch | (cc << 8));
            }
        }
    }

    pub fn close(h: isize) {
        // SAFETY: h came from Box::into_raw in open; it is not used afterwards
        let units = unsafe { Box::from_raw(h as *mut Units) };
        dispose(&units);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod system_midi {
    pub fn open() -> Option<isize> {
        None
    }
    pub fn send(_: isize, _: u32) {}
    pub fn reset(_: isize) {}
    pub fn close(_: isize) {}
}

/// Sample rate of the SoundFont synthesizer.
const SYNTH_RATE: u32 = 44_100;
/// The sequencer runs 120 XMIDI ticks per second.
const MUSIC_TICK: Duration = Duration::from_micros(1_000_000 / 120);
/// How long the music takes to fade away at the end of the game (with the
/// last picture of the end movie).
const MUSIC_FADE_OUT: Duration = Duration::from_millis(1500);

enum MusicCommand {
    /// XMIDI event data to play in a loop; with a second piece, the
    /// first is played once and the second loops after it.
    Play(Vec<u8>, Option<Vec<u8>>),
    Stop,
    /// Grows quieter over this time, then stops (the end of the game).
    FadeOut(Duration),
    /// Output level, 0..=100.
    Volume(u8),
    /// true: our own synthesizer with a SoundFont (option "better music"),
    /// false: the MIDI synthesizer of Windows.
    SoundFont(bool),
}

/// Reads a MIDI variable length number.
fn vlq(d: &[u8], pos: &mut usize) -> u32 {
    let mut v = 0;
    while let Some(&b) = d.get(*pos) {
        *pos += 1;
        v = (v << 7) | (b & 0x7F) as u32;
        if b & 0x80 == 0 {
            break;
        }
    }
    v
}

/// The first SoundFont (.sf2) found: in `%APPDATA%\pagan\soundfonts`, or
/// in a folder `soundfonts` next to the program or in the current directory.
pub fn find_soundfont() -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = crate::user_dir() {
        dirs.push(dir.join("soundfonts"));
    }
    if let Some(exe) = crate::exe_dir() {
        dirs.push(exe.join("soundfonts"));
    }
    dirs.push(PathBuf::from("soundfonts"));
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut fonts: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sf2")))
            .collect();
        fonts.sort();
        if let Some(f) = fonts.into_iter().next() {
            return Some(f);
        }
    }
    None
}

fn load_soundfont(path: &Path) -> Option<Arc<SoundFont>> {
    let mut file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    match SoundFont::new(&mut file) {
        Ok(f) => Some(Arc::new(f)),
        Err(e) => {
            eprintln!("music: cannot read {}: {e:?}", path.display());
            None
        }
    }
}

fn new_synth(font: &Arc<SoundFont>) -> Option<Synthesizer> {
    let settings = SynthesizerSettings::new(SYNTH_RATE as i32);
    let mut synth = Synthesizer::new(font, &settings).ok()?;
    synth.set_master_volume(MASTER_VOLUME);
    Some(synth)
}

/// Loudness of the SoundFont synthesizer, so the music sits about as loud
/// against the effects as with the Windows synthesizer.
const MASTER_VOLUME: f32 = 0.9;

/// The synthesizer as a sound source for the mixer: it renders in small
/// blocks while the sequencer thread feeds it the notes.
struct SynthSource {
    synth: Arc<Mutex<Synthesizer>>,
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
}

impl SynthSource {
    fn new(synth: Arc<Mutex<Synthesizer>>) -> SynthSource {
        // 256 samples: about 6 ms, so the notes keep their timing
        SynthSource { synth, left: vec![0.0; 256], right: vec![0.0; 256], pos: usize::MAX }
    }
}

impl Iterator for SynthSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos >= 2 * self.left.len() {
            if let Ok(mut s) = self.synth.lock() {
                s.render(&mut self.left, &mut self.right);
            }
            self.pos = 0;
        }
        let i = self.pos / 2;
        let v = if self.pos.is_multiple_of(2) { self.left[i] } else { self.right[i] };
        self.pos += 1;
        Some(v)
    }
}

impl Source for SynthSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        SYNTH_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Where the notes go.
enum MidiOut {
    None,
    /// The MIDI synthesizer of Windows (a handle from midiOutOpen).
    Windows(isize),
    /// Our own synthesizer with a SoundFont; the sink plays it.
    Font(Arc<Mutex<Synthesizer>>, Option<Sink>),
}

impl MidiOut {
    fn windows() -> MidiOut {
        match system_midi::open() {
            Some(h) => MidiOut::Windows(h),
            None => {
                eprintln!("music: no MIDI output device (a SoundFont is needed, see the option \"better music\")");
                MidiOut::None
            }
        }
    }

    fn send(&self, status: u8, d1: u8, d2: u8) {
        match self {
            MidiOut::None => {}
            MidiOut::Windows(h) => system_midi::send(*h, status as u32 | ((d1 as u32) << 8) | ((d2 as u32) << 16)),
            MidiOut::Font(s, _) => {
                if let Ok(mut s) = s.lock() {
                    s.process_midi_message((status & 0x0F) as i32, (status & 0xF0) as i32, d1 as i32, d2 as i32);
                }
            }
        }
    }

    /// All notes off, controllers and instruments back to the start.
    fn reset(&self) {
        match self {
            MidiOut::None => {}
            MidiOut::Windows(h) => system_midi::reset(*h),
            MidiOut::Font(s, _) => {
                if let Ok(mut s) = s.lock() {
                    s.reset();
                }
            }
        }
    }

    fn close(&mut self) {
        self.reset();
        match std::mem::replace(self, MidiOut::None) {
            MidiOut::Windows(h) => system_midi::close(h),
            MidiOut::Font(_, Some(sink)) => sink.stop(),
            _ => {}
        }
    }
}

/// Plays XMIDI event data, one tick (1/120 s) at a time.
struct Sequencer {
    song: Option<Vec<u8>>,
    /// What to go on with when the current piece is over (instead of looping).
    after: Option<Vec<u8>>,
    pos: usize,
    /// Ticks until the next event.
    wait: u32,
    /// Sounding notes: (ticks left, channel, note).
    notes: Vec<(u32, u8, u8)>,
    /// The menu level, applied to the channel volumes (controller 7),
    /// which every synthesizer honours.
    level: u32,
    /// The channel volumes as set by the song.
    channel_volume: [u8; 16],
    ended: bool,
}

impl Sequencer {
    fn new() -> Sequencer {
        Sequencer { song: None, after: None, pos: 0, wait: 0, notes: Vec::new(), level: 100, channel_volume: [100; 16], ended: false }
    }

    fn scaled(&self, v: u8) -> u8 {
        (v as u32 * self.level / 100).min(127) as u8
    }

    fn set_level(&mut self, out: &MidiOut, level: u8) {
        self.level = level.min(100) as u32;
        for ch in 0..16 {
            out.send(0xB0 | ch as u8, 7, self.scaled(self.channel_volume[ch]));
        }
    }

    /// From the beginning of the current piece, with a fresh synthesizer.
    fn rewind(&mut self, out: &MidiOut) {
        out.reset();
        // a song starts from the default channel volumes
        self.channel_volume = [100; 16];
        for ch in 0..16u8 {
            out.send(0xB0 | ch, 7, self.scaled(100));
        }
        self.notes.clear();
        self.pos = 0;
        self.wait = 0;
    }

    fn play(&mut self, out: &MidiOut, song: Option<Vec<u8>>, after: Option<Vec<u8>>) {
        self.rewind(out);
        (self.song, self.after) = (song, after);
    }

    fn tick(&mut self, out: &MidiOut) {
        // the piece played once is over: go on with the one after it
        if self.ended {
            self.ended = false;
            if self.after.is_some() {
                self.song = self.after.take();
                self.rewind(out);
            }
        }
        let level = self.level;
        let Sequencer { song, pos, wait, notes, channel_volume, ended, after, .. } = self;
        let Some(d) = song else { return };

        // end notes whose time is up
        notes.retain_mut(|n| {
            if n.0 <= 1 {
                out.send(0x80 | n.1, n.2, 0);
                false
            } else {
                n.0 -= 1;
                true
            }
        });

        // events due now
        let mut guard = 0;
        while *wait == 0 {
            guard += 1;
            let Some(&b) = d.get(*pos) else {
                *pos = 0; // loop the song
                *ended = true;
                if guard > 1 || after.is_some() {
                    *wait = 1;
                }
                continue;
            };
            *pos += 1;
            if b < 0x80 {
                *wait += b as u32; // delay
                continue;
            }
            match b & 0xF0 {
                0x90 => {
                    let (note, vel) = (d.get(*pos).copied().unwrap_or(0), d.get(*pos + 1).copied().unwrap_or(0));
                    *pos += 2;
                    let len = vlq(d, pos);
                    out.send(b, note, vel);
                    notes.push((len.max(1), b & 0x0F, note));
                }
                0x80 | 0xA0 | 0xE0 => {
                    out.send(b, d.get(*pos).copied().unwrap_or(0), d.get(*pos + 1).copied().unwrap_or(0));
                    *pos += 2;
                }
                0xB0 => {
                    let (c, v) = (d.get(*pos).copied().unwrap_or(0), d.get(*pos + 1).copied().unwrap_or(0));
                    *pos += 2;
                    if c == 7 {
                        channel_volume[(b & 0x0F) as usize] = v;
                        out.send(b, c, (v as u32 * level / 100).min(127) as u8);
                    } else if !(110..=120).contains(&c) {
                        // 110..=120 are XMIDI's own controllers (loops, callbacks)
                        out.send(b, c, v);
                    }
                }
                0xC0 | 0xD0 => {
                    out.send(b, d.get(*pos).copied().unwrap_or(0), 0);
                    *pos += 1;
                }
                _ => {
                    if b == 0xFF {
                        let kind = d.get(*pos).copied().unwrap_or(0x2F);
                        *pos += 1;
                        let len = vlq(d, pos) as usize;
                        *pos += len;
                        if kind == 0x2F {
                            *pos = 0; // end of track: start over
                            *ended = true;
                            *wait = (*wait).max(1);
                        }
                    } else {
                        // system exclusive: skip
                        let len = vlq(d, pos) as usize;
                        *pos += len;
                    }
                }
            }
        }
        *wait -= 1;
    }
}

/// Opens the output for the music: the SoundFont synthesizer if wanted and
/// possible (the SoundFont is read only the first time), else Windows.
fn open_output(want_font: bool, font: &mut Option<Arc<SoundFont>>, font_failed: &mut bool, stream: &Option<OutputStreamHandle>) -> MidiOut {
    if want_font && !*font_failed {
        if font.is_none() {
            *font = find_soundfont().and_then(|p| load_soundfont(&p));
            *font_failed = font.is_none();
        }
        if let (Some(synth), Some(stream)) = (font.as_ref().and_then(new_synth), stream) {
            let synth = Arc::new(Mutex::new(synth));
            let sink = Sink::try_new(stream).ok();
            if let Some(sink) = &sink {
                sink.append(SynthSource::new(synth.clone()));
            }
            return MidiOut::Font(synth, sink);
        }
    }
    MidiOut::windows()
}

/// The music thread: commands from the game, the sequencer in real time.
fn music_thread(rx: Receiver<MusicCommand>, stream: Option<OutputStreamHandle>, soundfont: bool) {
    let mut font: Option<Arc<SoundFont>> = None;
    let mut font_failed = false;
    let mut out = open_output(soundfont, &mut font, &mut font_failed, &stream);
    let mut seq = Sequencer::new();
    let mut next = Instant::now();
    // the menu level, and a fade out under way: since when, how long
    let mut level = 100u8;
    let mut fading: Option<(Instant, Duration)> = None;
    loop {
        // commands; block while there is nothing to play
        let cmd = if seq.song.is_some() {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break, // the game is gone
            }
        };
        match cmd {
            Some(MusicCommand::Volume(v)) => {
                level = v;
                if fading.is_none() {
                    seq.set_level(&out, v);
                }
            }
            Some(MusicCommand::FadeOut(d)) => fading = Some((Instant::now(), d)),
            Some(MusicCommand::SoundFont(on)) => {
                // the other synthesizer, and the piece from its beginning
                // (its instruments are chosen there)
                out.close();
                out = open_output(on, &mut font, &mut font_failed, &stream);
                seq.rewind(&out);
                next = Instant::now();
            }
            Some(MusicCommand::Play(data, then)) => {
                if fading.take().is_some() {
                    seq.set_level(&out, level);
                }
                seq.play(&out, Some(data), then);
                next = Instant::now();
            }
            Some(MusicCommand::Stop) => {
                fading = None;
                seq.play(&out, None, None);
                seq.set_level(&out, level);
            }
            None => {}
        }
        if seq.song.is_none() {
            continue;
        }
        if let Some((since, d)) = fading {
            let t = since.elapsed().as_secs_f32() / d.as_secs_f32().max(0.001);
            if t >= 1.0 {
                fading = None;
                seq.play(&out, None, None);
                seq.set_level(&out, level);
                continue;
            }
            let now_level = (level as f32 * (1.0 - t)).round() as u8;
            if now_level as u32 != seq.level {
                seq.set_level(&out, now_level);
            }
        }
        seq.tick(&out);

        next += MUSIC_TICK;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(200) {
            next = now; // fell behind (e.g. after standby)
        }
    }
    out.close();
}

/// Renders a piece with the SoundFont synthesizer, for tests: stereo
/// samples at `SYNTH_RATE`, or None without a SoundFont.
pub fn render_music(events: Vec<u8>, seconds: f32) -> Option<Vec<f32>> {
    let font = load_soundfont(&find_soundfont()?)?;
    let synth = Arc::new(Mutex::new(new_synth(&font)?));
    let out = MidiOut::Font(synth.clone(), None);
    let mut seq = Sequencer::new();
    seq.play(&out, Some(events), None);
    let mut src = SynthSource::new(synth);
    let mut samples = Vec::new();
    for t in 0..(seconds * 120.0) as usize {
        seq.tick(&out);
        // the samples up to the end of this tick
        let until = (t + 1) * SYNTH_RATE as usize / 120 * 2;
        while samples.len() < until {
            samples.push(src.next().unwrap_or(0.0));
        }
    }
    Some(samples)
}

/// The event data of an XMIDI file.
fn xmidi_events(d: &[u8]) -> Option<Vec<u8>> {
    let at = d.windows(4).position(|w| w == b"EVNT")?;
    let len = u32::from_be_bytes([*d.get(at + 4)?, *d.get(at + 5)?, *d.get(at + 6)?, *d.get(at + 7)?]) as usize;
    Some(d.get(at + 8..(at + 8 + len).min(d.len()))?.to_vec())
}

// ---- spatial sound -----------------------------------------------------------

/// The level of the left and right channel of a playing effect (f32 bits),
/// changed by the game while it plays.
pub struct Gains([AtomicU32; 2]);

impl Gains {
    fn new(pan: f32) -> Arc<Gains> {
        let g = Arc::new(Gains([AtomicU32::new(0), AtomicU32::new(0)]));
        g.set_pan(pan);
        g
    }

    /// -1 = all left, 0 = middle, 1 = all right. The middle is as loud as
    /// before on both sides; towards a side the other one gets quieter.
    fn set_pan(&self, pan: f32) {
        let pan = pan.clamp(-1.0, 1.0);
        self.0[0].store((1.0 - pan).min(1.0).to_bits(), Ordering::Relaxed);
        self.0[1].store((1.0 + pan).min(1.0).to_bits(), Ordering::Relaxed);
    }

    fn get(&self, i: usize) -> f32 {
        f32::from_bits(self.0[i].load(Ordering::Relaxed))
    }
}

/// A mono effect made stereo with the levels in `Gains`. The levels glide
/// to new values over a few milliseconds, so a change does not click.
struct Panned<S> {
    inner: S,
    gains: Arc<Gains>,
    now: [f32; 2],
    right: Option<f32>,
}

impl<S: Source<Item = i16>> Panned<S> {
    fn new(inner: S, gains: Arc<Gains>) -> Panned<S> {
        let now = [gains.get(0), gains.get(1)];
        Panned { inner, gains, now, right: None }
    }
}

impl<S: Source<Item = i16>> Iterator for Panned<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let s = self.inner.next()? as f32 / 32768.0;
        for i in 0..2 {
            self.now[i] += (self.gains.get(i) - self.now[i]) * 0.002;
        }
        self.right = Some(s * self.now[1]);
        Some(s * self.now[0])
    }
}

impl<S: Source<Item = i16>> Source for Panned<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len().map(|n| n * 2)
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

// ---- the mixer -------------------------------------------------------------

pub struct Audio {
    /// Keeps the output device open.
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    sound_dir: PathBuf,
    /// 'E', 'G' or 'F': prefix of the speech files of the language.
    lang: char,
    sfx: Option<Flex>,
    music: Option<Flex>,
    speech: HashMap<u32, Option<Speech>>,
    /// Playing effects: (number, object, looping, sink, left/right levels).
    playing: Vec<(i32, u16, bool, Sink, Arc<Gains>)>,
    /// Effects come from the side of their source (option "spatial sound").
    spatial: bool,
    /// The samples are made anew at 44.1 kHz before they are played (option
    /// "better sound"), and kept once made: effect number -> samples.
    better: bool,
    better_sfx: Arc<Mutex<HashMap<i32, Arc<Vec<i16>>>>>,
    /// The effects are being made anew in a thread of their own (once).
    better_started: bool,
    /// Where each object's sounds came from last (-1 left .. 1 right).
    pans: HashMap<u16, f32>,
    /// The music is played with the SoundFont (option "better music").
    soundfont: bool,
    /// The SoundFont that would be used, if there is one.
    pub soundfont_path: Option<PathBuf>,
    voice: Option<Sink>,
    /// When the voice of a movie will have finished its sentence.
    voice_end: Option<std::time::Instant>,
    music_tx: Option<Sender<MusicCommand>>,
    /// The piece that is playing.
    track: i32,
    /// The piece the map's scripts asked for; it comes back after a fight.
    requested: i32,
    /// Fight music has taken over.
    fighting: bool,
    /// Levels 0..=100 for music, effects and speech.
    volumes: [u8; 3],
    /// The album's files for the pieces they replace (option "album music").
    album_files: HashMap<i32, PathBuf>,
    /// Plays the album, while the option is on.
    album: Option<crate::album::Player>,
    /// Option "album music".
    album_on: bool,
    /// What was requested, for tests without a sound device.
    pub log: Vec<String>,
}

impl Audio {
    /// `enabled`: open the sound devices (off for tests).
    /// `soundfont`: start the music with the SoundFont synthesizer.
    /// `album`: the music comes from the album's files where there are any.
    pub fn new(sound_dir: &Path, lang: char, enabled: bool, volumes: [u8; 3], soundfont: bool, album: bool) -> Audio {
        let (stream, handle) = match enabled.then(OutputStream::try_default) {
            Some(Ok((s, h))) => (Some(s), Some(h)),
            Some(Err(e)) => {
                eprintln!("audio: no output device ({e})");
                (None, None)
            }
            None => (None, None),
        };
        let mixer = handle.clone();
        let music_tx = enabled.then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || music_thread(rx, mixer, soundfont));
            let _ = tx.send(MusicCommand::Volume(volumes[0]));
            tx
        });
        Audio {
            _stream: stream,
            handle,
            sound_dir: sound_dir.to_path_buf(),
            lang,
            sfx: Flex::open(&sound_dir.join("SOUND.FLX")).ok(),
            music: Flex::open(&sound_dir.join("MUSIC.FLX")).ok(),
            speech: HashMap::default(),
            playing: Vec::new(),
            spatial: false,
            better: false,
            better_sfx: Arc::new(Mutex::new(HashMap::default())),
            better_started: false,
            pans: HashMap::default(),
            soundfont,
            soundfont_path: find_soundfont(),
            voice: None,
            voice_end: None,
            music_tx,
            track: 0,
            requested: 0,
            fighting: false,
            volumes,
            album_files: sound_dir.parent().and_then(Path::parent).map(crate::album::find).unwrap_or_default(),
            album: None,
            album_on: album,
            log: Vec::new(),
        }
    }

    /// The album's files were found.
    pub fn has_album(&self) -> bool {
        !self.album_files.is_empty()
    }

    /// The album's file for a piece, for tests.
    pub fn album_file(&self, track: i32) -> Option<&PathBuf> {
        self.album_files.get(&track)
    }

    /// With the album: how far into the piece for `first` the piece for
    /// `later` begins, when the album has both in one file.
    pub fn album_cue(&self, first: i32, later: i32) -> Option<std::time::Duration> {
        let (a, b) = (self.album_files.get(&first)?, self.album_files.get(&later)?);
        if !self.album_on || a != b {
            return None;
        }
        crate::album::cue(later).map(std::time::Duration::from_secs_f32)
    }

    /// Option "album music": the piece playing changes over at once.
    pub fn set_album(&mut self, on: bool) {
        if on != self.album_on {
            self.album_on = on;
            let (once, track) = (None, self.track);
            self.track = -1;
            self.start(track, once);
        }
    }


    pub fn volumes(&self) -> [u8; 3] {
        self.volumes
    }

    /// Sets the level (0..=100) of music (0), effects (1) or speech (2).
    pub fn set_volume(&mut self, kind: usize, level: u8) {
        let level = level.min(100);
        if kind >= 3 || self.volumes[kind] == level {
            return;
        }
        // rescale what is playing right now
        let ratio = |old: u8| if old == 0 { 0.0 } else { level as f32 / old as f32 };
        match kind {
            0 => {
                if let Some(a) = &self.album {
                    a.set_volume(level);
                }
                if let Some(tx) = &self.music_tx {
                    let _ = tx.send(MusicCommand::Volume(level));
                }
            }
            1 => {
                let r = ratio(self.volumes[1]);
                for p in &self.playing {
                    p.3.set_volume(if r == 0.0 { level as f32 / 100.0 } else { p.3.volume() * r });
                }
            }
            _ => {
                if let Some(v) = &self.voice {
                    v.set_volume(level as f32 / 100.0);
                }
            }
        }
        self.volumes[kind] = level;
    }

    fn forget_finished(&mut self) {
        self.playing.retain(|p| !p.3.empty());
    }

    /// Plays effect `num`; `volume` 0.0..=1.0. A looping effect of an
    /// object plays until it is stopped.
    pub fn play_sfx(&mut self, num: i32, object: u16, looping: bool, volume: f32) {
        self.play(num, object, looping, volume, 1.0);
    }

    /// Plays an effect once, faster or slower (`pitch` 1.0 = as recorded).
    pub fn play_sfx_pitched(&mut self, num: i32, object: u16, volume: f32, pitch: f32) {
        self.play(num, object, false, volume, pitch);
    }

    fn play(&mut self, num: i32, object: u16, looping: bool, volume: f32, pitch: f32) {
        self.forget_finished();
        if num <= 0 || volume <= 0.01 {
            return;
        }
        if looping && self.playing.iter().any(|p| p.0 == num && p.1 == object && p.2) {
            return;
        }
        self.log.push(format!("sfx {num}{}", if looping { " (loop)" } else { "" }));
        let (Some(handle), Some(flex)) = (&self.handle, &self.sfx) else { return };
        // do not pile up many copies of the same sound
        if self.playing.iter().filter(|p| p.0 == num).count() >= 3 {
            return;
        }
        let Some((rate, pcm)) = flex.get(num as usize).and_then(decode_sonarc) else { return };
        let Ok(sink) = Sink::try_new(handle) else { return };
        // made anew in the thread (see set_better); until it gets there, as it is
        let made = if self.better { self.better_sfx.lock().ok().and_then(|m| m.get(&num).cloned()) } else { None };
        let source = match made {
            Some(samples) => SamplesBuffer::new(1, crate::soundfix::RATE, samples.to_vec()),
            None => to_source(rate, &pcm),
        };
        sink.set_volume(volume.min(1.0) * self.volumes[1] as f32 / 100.0);
        // from where the object made a sound last; the game corrects it every tick
        let pan = if self.spatial { self.pans.get(&object).copied().unwrap_or(0.0) } else { 0.0 };
        let gains = Gains::new(pan);
        if looping {
            sink.append(Panned::new(source.repeat_infinite(), gains.clone()));
        } else {
            sink.append(Panned::new(source.speed(pitch.clamp(0.5, 2.0)), gains.clone()));
        }
        self.playing.push((num, object, looping, sink, gains));
    }

    /// Stops an effect; `object` 0 = for all objects.
    pub fn stop_sfx(&mut self, num: i32, object: u16) {
        self.playing.retain(|p| {
            let hit = p.0 == num && (object == 0 || p.1 == object);
            if hit {
                p.3.stop();
            }
            !hit
        });
    }

    /// Stops the looping effects of objects for which `keep` is false.
    pub fn stop_loops(&mut self, keep: impl Fn(u16) -> bool) {
        self.playing.retain(|p| {
            let stop = p.2 && !keep(p.1);
            if stop {
                p.3.stop();
            }
            !stop
        });
    }

    pub fn is_sfx_playing(&mut self, num: i32) -> bool {
        self.forget_finished();
        self.playing.iter().any(|p| p.0 == num)
    }

    /// Speaks `text` with the voice of the character with this shape, if
    /// it was recorded. Returns the length in milliseconds.
    pub fn speak(&mut self, shape: u32, text: &str) -> Option<u32> {
        if !self.speech.contains_key(&shape) {
            let s = Speech::open(&self.sound_dir.join(format!("{}{}.FLX", self.lang, shape)));
            self.speech.insert(shape, s);
        }
        let speech = self.speech.get(&shape)?.as_ref()?;
        let samples = speech.samples_for(text);
        if samples.is_empty() {
            return None;
        }
        let decoded: Vec<(u32, Vec<u8>)> = samples.iter().filter_map(|&i| speech.flex.get(i).and_then(decode_sonarc)).collect();
        let millis: u32 = decoded.iter().map(|(rate, pcm)| pcm.len() as u32 * 1000 / rate.max(&1)).sum();
        if millis == 0 {
            return None;
        }
        self.log.push(format!("speech {shape}: {} sample(s), {millis} ms", decoded.len()));
        self.stop_speech();
        if let Some(handle) = &self.handle {
            if let Ok(sink) = Sink::try_new(handle) {
                sink.set_volume(self.volumes[2] as f32 / 100.0);
                for (rate, pcm) in &decoded {
                    sink.append(sample_source(self.better, *rate, pcm));
                }
                self.voice = Some(sink);
            }
        }
        Some(millis)
    }

    /// The first recorded lines of a character, for testing.
    pub fn speech_sample_text(&mut self, shape: u32, lines: usize) -> String {
        self.speak(shape, "");
        match self.speech.get(&shape) {
            Some(Some(s)) => s.phrases.iter().filter(|p| !p.is_empty()).take(lines).cloned().collect::<Vec<_>>().join(" "),
            _ => String::new(),
        }
    }

    /// Plays a recording (unsigned 8 bit) as speech: the voices of the movies.
    pub fn play_voice(&mut self, rate: u32, pcm: &[u8]) {
        self.log.push(format!("voice: {} ms", pcm.len() as u32 * 1000 / rate.max(1)));
        self.stop_speech();
        if let Some(handle) = &self.handle {
            if let Ok(sink) = Sink::try_new(handle) {
                sink.set_volume(self.volumes[2] as f32 / 100.0);
                sink.append(sample_source(self.better, rate.max(1), pcm));
                self.voice = Some(sink);
                self.voice_end = Some(std::time::Instant::now() + std::time::Duration::from_millis(pcm.len() as u64 * 1000 / rate.max(1) as u64));
            }
        }
    }

    /// Is a voice still speaking?
    pub fn voice_playing(&self) -> bool {
        self.voice.as_ref().is_some_and(|v| !v.empty())
    }

    /// How long the voice of a movie still has to speak.
    pub fn voice_remaining(&self) -> Option<std::time::Duration> {
        if !self.voice_playing() {
            return None;
        }
        self.voice_end.and_then(|end| end.checked_duration_since(std::time::Instant::now()))
    }

    pub fn stop_speech(&mut self) {
        if let Some(v) = self.voice.take() {
            v.stop();
        }
    }

    /// Switches the music to a track (0 = silence).
    fn events(&self, track: i32) -> Option<Vec<u8>> {
        // the album's second death piece is the game's one in MIDI
        let track = if track == crate::album::DEATH_WORLD { 44 } else { track };
        (1..128).contains(&track).then(|| self.music.as_ref()?.get(track as usize).and_then(xmidi_events)).flatten()
    }

    /// The XMIDI events of a piece, for tests.
    pub fn music_events(&self, track: i32) -> Option<Vec<u8>> {
        self.events(track)
    }

    /// Starts a piece; with `then`, it is played once and `then` loops after it.
    fn start(&mut self, track: i32, then: Option<i32>) {
        if track == self.track && then.is_none() {
            return;
        }
        self.track = then.unwrap_or(track);
        self.log.push(match then {
            Some(t) => format!("music {track} then {t}"),
            None => format!("music {track}"),
        });
        // the piece that stays comes from the album, if it is there
        let lasting = then.unwrap_or(track);
        let file = self.album_files.get(&lasting).filter(|_| self.album_on).cloned();
        if let Some(f) = &file {
            self.log.push(format!("album {}", f.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        }
        if self.music_tx.is_none() {
            return;
        }
        match &file {
            Some(f) => {
                if self.album.is_none() {
                    self.album = self.handle.as_ref().and_then(|h| crate::album::Player::new(h, self.volumes[0]));
                }
                if let Some(a) = &mut self.album {
                    a.play(f, crate::album::loops(lasting));
                    // the short piece before it, if the album has it too,
                    // over the lasting one (the fanfare over the fight)
                    if let Some(once) = then.and_then(|_| self.album_files.get(&track)) {
                        a.overlay(once);
                    }
                }
            }
            None => {
                if let Some(a) = &mut self.album {
                    a.stop();
                }
            }
        }
        let Some(tx) = &self.music_tx else { return };
        let _ = tx.send(match (&file, then) {
            // a short piece once over the album's (the fanfare), then quiet;
            // from the album as well if it has the piece
            (Some(_), Some(_)) if self.album_files.contains_key(&track) => MusicCommand::Stop,
            (Some(_), Some(_)) => match self.events(track) {
                Some(e) => MusicCommand::Play(e, Some(Vec::new())),
                None => MusicCommand::Stop,
            },
            (Some(_), None) => MusicCommand::Stop,
            (None, _) => match self.events(track) {
                Some(e) => MusicCommand::Play(e, then.and_then(|t| self.events(t))),
                None => MusicCommand::Stop,
            },
        });
    }

    /// The scripts of a place ask for its music (0 = silence). During a
    /// fight it is only remembered.
    pub fn play_music(&mut self, track: i32) {
        self.requested = track;
        if !self.fighting {
            self.start(track, None);
        }
    }

    /// The music grows quiet and stops (the end movie fading away).
    pub fn fade_out_music(&mut self) {
        self.requested = 0;
        if self.track == 0 {
            return;
        }
        self.track = 0;
        self.log.push("music fades out".to_string());
        // (the album fades what it plays when stopped)
        if let Some(a) = &mut self.album {
            a.stop();
        }
        if let Some(tx) = &self.music_tx {
            let _ = tx.send(MusicCommand::FadeOut(MUSIC_FADE_OUT));
        }
    }

    /// Fight music takes over (the combat piece, the death piece ...).
    pub fn play_combat_music(&mut self, track: i32) {
        self.fighting = true;
        self.start(track, None);
    }

    /// A short piece once (the fanfare for a kill), then on with another.
    pub fn play_once_then(&mut self, once: i32, then: i32) {
        self.start(once, Some(then));
    }

    /// The fight is over: back to the music of the place.
    pub fn restore_music(&mut self) {
        self.fighting = false;
        self.start(self.requested, None);
    }

    /// The piece to come back to (for savegames).
    pub fn requested(&self) -> i32 {
        self.requested
    }

    /// Option "better music": switches between the SoundFont and the
    /// Windows synthesizer (the piece starts over).
    pub fn set_soundfont(&mut self, on: bool) {
        if on != self.soundfont {
            self.soundfont = on;
            if let Some(tx) = &self.music_tx {
                let _ = tx.send(MusicCommand::SoundFont(on));
            }
        }
    }

    /// Option "better sound": the effects and voices started from now on.
    pub fn set_better(&mut self, on: bool) {
        self.better = on;
        if on && !self.better_started && self.handle.is_some() {
            // all effects at once, beside the game (well under a second)
            self.better_started = true;
            let Some(flex) = self.sfx.clone() else { return };
            let cache = self.better_sfx.clone();
            std::thread::spawn(move || {
                let start = std::time::Instant::now();
                for num in 0..flex.len() {
                    let Some((rate, pcm)) = flex.get(num).and_then(decode_sonarc) else { continue };
                    let made = Arc::new(crate::soundfix::enhance(rate, &pcm));
                    if let Ok(mut m) = cache.lock() {
                        m.insert(num as i32, made);
                    }
                }
                if crate::debug::env().sound_info {
                    eprintln!("better sound: {} effects ready after {} ms", cache.lock().map_or(0, |m| m.len()), start.elapsed().as_millis());
                }
            });
        }
    }

    /// A sound effect as it would be played, for tests: (rate, samples).
    pub fn effect_samples(&self, num: i32, better: bool) -> Option<(u32, Vec<i16>)> {
        let (rate, pcm) = self.sfx.as_ref()?.get(num as usize).and_then(decode_sonarc)?;
        Some(if better {
            (crate::soundfix::RATE, crate::soundfix::enhance(rate, &pcm))
        } else {
            (rate, pcm.iter().map(|&b| ((b as i16) - 128) << 8).collect())
        })
    }

    /// Option "spatial sound". Switched off, everything sounds from the middle again.
    pub fn set_spatial(&mut self, on: bool) {
        if on != self.spatial {
            self.spatial = on;
            if !on {
                for p in &self.playing {
                    p.4.set_pan(0.0);
                }
                self.pans.clear();
            }
        }
    }

    pub fn spatial(&self) -> bool {
        self.spatial
    }

    /// The objects whose sounds are playing.
    pub fn sound_objects(&mut self) -> Vec<u16> {
        self.forget_finished();
        let mut v: Vec<u16> = self.playing.iter().map(|p| p.1).filter(|&o| o != 0).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Spatial sound: where the sounds of objects come from now (-1 left
    /// .. 1 right) and how loud they are for the player (0..=1).
    pub fn place_sounds(&mut self, spots: &[(u16, f32, f32)]) {
        let level = self.volumes[1] as f32 / 100.0;
        for &(object, pan, gain) in spots {
            self.pans.insert(object, pan);
            for p in self.playing.iter().filter(|p| p.1 == object) {
                p.4.set_pan(pan);
                p.3.set_volume(gain.clamp(0.0, 1.0) * level);
            }
        }
        if self.pans.len() > 256 {
            self.pans.clear();
        }
    }

    /// The playing effects of objects: (object, pan, level), for tests.
    pub fn spots(&self) -> Vec<(u16, f32, f32)> {
        self.playing.iter().filter(|p| p.1 != 0).map(|p| (p.1, (p.4.get(1) - p.4.get(0)), p.3.volume())).collect()
    }

    /// Silences everything that belongs to the place being left.
    pub fn stop_all_sfx(&mut self) {
        for p in self.playing.drain(..) {
            p.3.stop();
        }
        self.stop_speech();
    }
}
