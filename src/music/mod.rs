//! Per-floor song music, built from the owner's MIDI stems.
//!
//! `tools/midi2music.py` reads the six supplied stem sets, quantises them onto
//! a 16th grid, classifies every stem into a role, finds a clean whole-bar
//! loop, and writes the tables in this module's children (`sewer.rs`,
//! `kitchen.rs`, ...). Those files are **generated**: edit the converter, not
//! them. [`synth::SongSynth`] plays the tables with procedural voices built
//! from the same DSP primitives as the rest of [`crate::audio`] — no samples,
//! no asset files, and nothing allocated in the audio callback.
//!
//! # The song table
//!
//! [`song_for`] is the single mapping from a run's [`Theme`] to a song. The
//! owner may swap two songs around, so the mapping lives in one `match` here
//! and nowhere else (the converter's `SONGS` table names the same pairing).
//!
//! # What the data looks like
//!
//! A part is a flat `&[(u8, u8, u8, u8)]` of `(delta_steps, midi, gate_steps,
//! velocity)`: the wait in 16th steps since that part's previous note (0 for a
//! simultaneous note such as the rest of a chord), the MIDI note or General
//! MIDI percussion note, the gate in steps, and the MIDI velocity. `midi == 0`
//! is a rest (advance time, sound nothing). Everything is `u8`, so a part is
//! four bytes per note and a whole song is a few kilobytes of source.
//!
//! # Determinism
//!
//! The sequencer is driven by integer sample counters and the tables are
//! fixed, so a song renders bit-for-bit identically on every run (pinned by
//! `a_song_renders_bit_for_bit_identically`). Nothing here reads the simulation: the only
//! live input is the intensity tier, which is what the app already drives.
//!
//! # Why a second synth
//!
//! [`crate::audio::MusicSynth`] stays exactly as it was and remains reachable
//! with `RATRUN_MUSIC=classic`; the engine picks the song synth by default (see
//! `AudioEngine::play_music`). That keeps the classic synth's tests — the ones
//! that reach into its fields — meaningful without editing them.

pub mod render;
#[allow(clippy::disallowed_methods)] // audio-only: platform maths is fine
pub mod synth;

mod flatbed;
mod kitchen;
mod lab;
mod laundry;
mod outfall;
mod printworks;
mod sewer;

use crate::game::level::Theme;

pub use synth::SongSynth;

/// One generated note: `(delta_steps, midi, gate_steps, velocity)`.
pub type Note = (u8, u8, u8, u8);

/// 16-bit procedural instrument patch assigned per role in [`Style::patches`].
///
/// [`Patch::Chip`] reproduces the legacy chiptune oscillator voice for that role
/// (the default in [`Style::base`]). Every other variant selects a 16-bit
/// FM, wavetable, Karplus-Strong plucked-string, or filtered FX engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Patch {
    /// Legacy oscillator voice (square/saw/triangle + simple filter).
    Chip,
    /// Deep warm 2/4-op FM sub/finger bass (Sewer, Lab).
    FmDarkBass,
    /// Bouncy Mega Drive FM slap bass with velocity-sensitive pop (Kitchen, Printworks).
    FmSlapBass,
    /// Warm acoustic upright / boom-bap FM bass (Laundry).
    FmUprightBass,
    /// Aggressive picked rock FM bass with subtle grit (Flatbed).
    FmPickedBass,
    /// Glassy DX7-style electric piano tine chords (Lab, Printworks).
    FmEpiano,
    /// Bright wooden FM marimba pluck (Kitchen).
    FmMarimba,
    /// Snappy metallic-woody FM clavinet lead (Printworks).
    FmClav,
    /// Hard inharmonic glassy FM bell for alarm chimes (Lab).
    FmHardBell,
    /// Lush 4-op FM brass swells (Sewer, Flatbed).
    FmWarmBrass,
    /// Snappy Genesis-style FM brass stabs (Kitchen, Printworks).
    FmPunchBrass,
    /// Overdriven FM electric guitar lead with soft-clip drive & delayed vibrato (Flatbed).
    FmOverdriveGuitar,
    /// Slow lush SNES wavetable string ensemble (Sewer).
    WtStrings,
    /// Soft chamber wavetable strings (Laundry).
    WtSoftStrings,
    /// Cold formant-shaped SNES choir pad (Lab).
    WtColdChoir,
    /// Hollow ocarina / flute wavetable lead with breath chiff & delayed vibrato (Sewer).
    WtHollowFlute,
    /// Bright airy whistle wavetable lead with delayed vibrato (Kitchen).
    WtWhistle,
    /// Clarinet / accordion reed wavetable lead with musette unison (Laundry).
    WtClarinetAccordion,
    /// Thin glassy sine wavetable lead with delayed vibrato (Lab).
    WtThinSine,
    /// Hammond / power-chord rock organ wavetable pad (Flatbed).
    WtRockOrgan,
    /// Karplus-Strong plucked harp / muted noir guitar (Sewer).
    KsHarp,
    /// Karplus-Strong crisp orchestral pizzicato strings (Kitchen).
    KsPizzicato,
    /// Karplus-Strong upright-piano chords with hammer thump (Laundry).
    KsUprightPiano,
    /// Karplus-Strong snappy clavinet / funk guitar pluck (Printworks).
    KsClavinet,
    /// Karplus-Strong picked electric guitar arpeggio (Flatbed).
    KsRockGuitar,
    /// 16-bit filtered air/noise transition sweep.
    Fx16,
}

/// 16-bit sampled-style drum kit assigned per floor in [`Style::drum_kit`].
///
/// [`DrumKit::Chip`] reproduces the legacy drum voices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum DrumKit {
    /// Legacy chip drums.
    Chip,
    /// Deep sparse trip-hop kit (Sewer).
    Deep,
    /// Tight bright polka/chase kit with woodblocks & timpani (Kitchen).
    TightBright,
    /// Soft brushed waltz kit (Laundry).
    Brushed,
    /// Clicky electronic kit with metallic hats & sonar cowbell (Lab).
    Electronic,
    /// Mechanical tight funk kit with prominent clap & rim (Printworks).
    Mechanical,
    /// Big arena rock kit with resonant toms (Flatbed).
    BigRock,
}

/// The ten parts a song is sequenced in. The order is fixed: the generated
/// tables and [`Song::parts`] both index by [`Role::index`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Role {
    Kick = 0,
    Snare,
    Hat,
    Perc,
    Bass,
    Pad,
    Arp,
    Lead,
    Stab,
    Fx,
}

/// Number of parts in a song (one per [`Role`]).
pub const N_ROLES: usize = 10;

impl Role {
    /// Every role, in table order.
    pub const ALL: [Role; N_ROLES] = [
        Role::Kick,
        Role::Snare,
        Role::Hat,
        Role::Perc,
        Role::Bass,
        Role::Pad,
        Role::Arp,
        Role::Lead,
        Role::Stab,
        Role::Fx,
    ];

    /// Index into [`Song::parts`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// A short name for reports and tests.
    pub const fn name(self) -> &'static str {
        match self {
            Role::Kick => "kick",
            Role::Snare => "snare",
            Role::Hat => "hat",
            Role::Perc => "perc",
            Role::Bass => "bass",
            Role::Pad => "pad",
            Role::Arp => "arp",
            Role::Lead => "lead",
            Role::Stab => "stab",
            Role::Fx => "fx",
        }
    }

    /// The most notes of this role allowed to sound at once. The bass is one
    /// line, the lead at most two, chords and arpeggios four; the percussion
    /// parts are one voice each (two for the miscellaneous percussion).
    pub const fn cap(self) -> usize {
        match self {
            Role::Kick | Role::Snare | Role::Hat => 1,
            Role::Perc => 2,
            Role::Bass => 1,
            Role::Pad | Role::Arp => 4,
            Role::Lead => 2,
            Role::Stab => 3,
            Role::Fx => 2,
        }
    }

    /// Drums are one-shot: their length is the instrument, not the gate.
    pub const fn is_drum(self) -> bool {
        matches!(self, Role::Kick | Role::Snare | Role::Hat | Role::Perc)
    }
}

/// Per-song instrument assignment and mix.
///
/// Everything here is a number the converter writes per song, so the six
/// floors differ in timbre as well as in material: filter cutoffs, pulse
/// widths, pad detune, the bus drive. `mix` is the per-role gain, already
/// level-matched to the background-music target by the converter.
pub struct Style {
    /// Per-role gain, indexed by [`Role::index`].
    pub mix: [f32; N_ROLES],
    /// Bus drive into the saturator.
    pub drive: f32,
    /// Trim after the saturator.
    pub master: f32,
    /// Triangle share of the bass voice (rest is pulse).
    pub bass_tri: f32,
    /// Bass low-pass cutoff, Hz.
    pub bass_lp: f32,
    /// Lead pulse duty cycle.
    pub lead_duty: f32,
    /// Lead low-pass cutoff, Hz.
    pub lead_lp: f32,
    /// Pad detune, as a fraction of the note's frequency.
    pub pad_detune: f32,
    /// Pad low-pass cutoff, Hz.
    pub pad_lp: f32,
    /// Plucked-key low-pass cutoff, Hz.
    pub arp_lp: f32,
    /// Brass pulse duty cycle.
    pub stab_duty: f32,
    /// Brass low-pass cutoff, Hz.
    pub stab_lp: f32,
    /// Kick drum body pitch, Hz.
    pub kick_hz: f32,
    /// Pad attack time, ms.
    pub pad_attack_ms: f32,
    /// Pad hold decay time to -60 dB, ms (`0.0` = flat sustain).
    pub pad_decay_ms: f32,
    /// Optional one-pole high-pass cutoff on the pad, Hz (`0.0` = bypassed).
    pub pad_hp_hz: f32,
    /// Bass hold decay time to -60 dB, ms (`0.0` = flat sustain).
    pub bass_decay_ms: f32,
    /// Triangle share of the lead voice (rest is pulse).
    pub lead_tri: f32,
    /// Lead hold decay time to -60 dB, ms (`0.0` = flat sustain).
    pub lead_decay_ms: f32,
    /// Lead brightness envelope sweep time, ms (`0.0` = static cutoff).
    pub lead_bright_ms: f32,
    /// Lead vibrato depth as a frequency fraction.
    pub lead_vib_depth: f32,
    /// Brass stab decay time to -60 dB, ms.
    pub stab_decay_ms: f32,
    /// Enables v2 voice/drum shaping (band-passed noise drums, open hat,
    /// woodblock/timpani, 1-pole bright LP, kick transient, FX sweep).
    pub enhanced_drums: bool,
    /// 16-bit patch per role (`Patch::Chip` reproduces the legacy chip voice).
    pub patches: [Patch; N_ROLES],
    /// 16-bit drum kit (`DrumKit::Chip` reproduces the legacy chip drums).
    pub drum_kit: DrumKit,
    /// Optional per-role mix gains for the 16-bit palette (all zero => use `mix`).
    pub mix_16: [f32; N_ROLES],
    /// Optional bus drive for the 16-bit palette (`0.0` => use `drive`).
    pub drive_16: f32,
    /// Global SNES-style echo delay time, ms (`0.0` => disabled).
    pub echo_ms: f32,
    /// Global echo feedback ratio (`0.0..=0.45`).
    pub echo_feedback: f32,
    /// One-pole low-pass cutoff inside the echo feedback loop, Hz.
    pub echo_lp_hz: f32,
    /// Per-role send level into the global echo bus.
    pub echo_send: [f32; N_ROLES],
    /// Soft Gaussian-style master low-pass cutoff, Hz (`0.0` => bypassed).
    pub master_lp_hz: f32,
}

impl Style {
    /// The neutral style: silent mix, mid-range filters. A generated song
    /// overrides everything it cares about with `..Style::base()`.
    pub const fn base() -> Style {
        Style {
            // Silence until a generated song supplies its own balance: a
            // `Style` that leaks a default gain would be a surprise.
            mix: [0.0; N_ROLES],
            drive: 1.15,
            master: 1.0,
            bass_tri: 0.55,
            bass_lp: 700.0,
            lead_duty: 0.32,
            lead_lp: 2800.0,
            pad_detune: 0.004,
            pad_lp: 1600.0,
            arp_lp: 3000.0,
            stab_duty: 0.40,
            stab_lp: 2000.0,
            kick_hz: 52.0,
            pad_attack_ms: 90.0,
            pad_decay_ms: 0.0,
            pad_hp_hz: 0.0,
            bass_decay_ms: 0.0,
            lead_tri: 0.14,
            lead_decay_ms: 0.0,
            lead_bright_ms: 0.0,
            lead_vib_depth: 0.0065,
            stab_decay_ms: 240.0,
            enhanced_drums: false,
            patches: [Patch::Chip; N_ROLES],
            drum_kit: DrumKit::Chip,
            mix_16: [0.0; N_ROLES],
            drive_16: 0.0,
            echo_ms: 0.0,
            echo_feedback: 0.0,
            echo_lp_hz: 2800.0,
            echo_send: [0.0; N_ROLES],
            master_lp_hz: 0.0,
        }
    }
}

/// One song: its generated parts plus the numbers the sequencer needs.
pub struct Song {
    /// Human title, from the stem set.
    pub name: &'static str,
    /// The floor this song was supplied for.
    pub floor: &'static str,
    /// Tempo, resolved to one constant from the stems' wobbling tempo map.
    pub bpm: u16,
    /// Beats per bar: 4, except the waltz (3).
    pub beats_per_bar: u8,
    /// Loop length in bars.
    pub bars: u16,
    /// Section boundaries inside the loop, as `(start bar, bars)`. The whole
    /// loop is one cycle that already contains them.
    pub sections: &'static [(u16, u16)],
    /// The ten parts, indexed by [`Role::index`].
    pub parts: [&'static [Note]; N_ROLES],
    /// Instrument and mix numbers.
    pub style: &'static Style,
}

impl Song {
    /// Grid steps in one bar (a step is a 16th note).
    pub const fn steps_per_bar(&self) -> u32 {
        4 * self.beats_per_bar as u32
    }

    /// Grid steps in the whole loop.
    pub const fn loop_steps(&self) -> u32 {
        self.steps_per_bar() * self.bars as u32
    }

    /// Loop length in seconds.
    pub fn loop_seconds(&self) -> f32 {
        self.loop_steps() as f32 * 60.0 / (self.bpm as f32 * 4.0)
    }

    /// Notes in the loop across every part.
    pub fn note_count(&self) -> usize {
        self.parts.iter().map(|p| p.len()).sum()
    }

    /// Bar the given grid step falls in.
    pub const fn bar_of(&self, step: u32) -> u16 {
        (step / self.steps_per_bar()) as u16
    }
}

/// Every song, indexed by the converter's song number (1-based), followed by
/// original arrangements.
pub static SONGS: [&Song; 7] = [
    &sewer::SONG,
    &kitchen::SONG,
    &laundry::SONG,
    &lab::SONG,
    &printworks::SONG,
    &flatbed::SONG,
    &outfall::SONG,
];

/// The one and only theme -> song table.
///
/// The owner's pairing: Silent Return -> Sewer, The Great Dash -> Kitchen,
/// A Minor Waltz -> Laundry, B-C-D# Warning -> Lab, Platen Pace -> Printworks,
/// Rat on the Flatbed -> Flatbed, Return Current -> Outfall. Each floor's file
/// holds its own song, so swapping two songs is a two-line change here (and
/// nothing else).
pub fn song_for(theme: Theme) -> &'static Song {
    match theme {
        Theme::Sewer => &sewer::SONG,
        Theme::Kitchen => &kitchen::SONG,
        Theme::Laundry => &laundry::SONG,
        Theme::Lab => &lab::SONG,
        Theme::Printworks => &printworks::SONG,
        Theme::Trailer => &flatbed::SONG,
        Theme::Outfall => &outfall::SONG,
    }
}

/// The theme for a floor name, as `--music-render` and the capture scripts
/// spell it (`sewer`, `kitchen`, `laundry`, `outfall`, `lab`, `printworks`,
/// `flatbed`, plus `trailer`).
pub fn theme_by_floor(floor: &str) -> Option<Theme> {
    Some(match floor {
        "sewer" => Theme::Sewer,
        "kitchen" => Theme::Kitchen,
        "lab" => Theme::Lab,
        "laundry" => Theme::Laundry,
        "printworks" => Theme::Printworks,
        "flatbed" | "trailer" => Theme::Trailer,
        "outfall" => Theme::Outfall,
        _ => return None,
    })
}

/// The song for a floor name (see [`theme_by_floor`]).
pub fn song_by_floor(floor: &str) -> Option<&'static Song> {
    theme_by_floor(floor).map(song_for)
}

/// Whether the main engine should use the classic pre-MIDI synth.
/// `RATRUN_MUSIC=classic` keeps the `MusicSynth` path for comparison and for
/// tests that reach into its internals.
pub fn classic_requested() -> bool {
    matches!(std::env::var("RATRUN_MUSIC").as_deref(), Ok("classic"))
}

/// Whether the song synth should use the legacy chip oscillator palette.
/// The 16-bit instrument palette is the default; `RATRUN_MUSIC=chip` plays the
/// chip palette.
pub fn chip_requested() -> bool {
    matches!(std::env::var("RATRUN_MUSIC").as_deref(), Ok("chip"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_playable_theme_gets_its_own_song() {
        let mut seen: Vec<&'static Song> = Vec::new();
        for theme in Theme::playable() {
            let song = song_for(theme);
            assert!(!song.name.is_empty(), "{theme:?} has no song name");
            for other in &seen {
                assert!(
                    !std::ptr::eq(song, *other),
                    "{theme:?} shares its song with another floor"
                );
            }
            seen.push(song);
        }
        assert_eq!(seen.len(), Theme::playable().len());
        assert_eq!(SONGS.len(), 7);
        // No two songs share a name, a tempo or a loop length.
        for (i, a) in SONGS.iter().enumerate() {
            for b in &SONGS[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.bpm, b.bpm, "{} and {}", a.name, b.name);
                assert_ne!(
                    (a.bpm, a.bars),
                    (b.bpm, b.bars),
                    "{} and {}",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn song_tables_are_sorted_in_range_and_fill_the_loop() {
        for song in SONGS {
            assert!(song.bpm >= 60 && song.bpm <= 220, "{} bpm {}", song.name, song.bpm);
            assert!(matches!(song.beats_per_bar, 3 | 4), "{} meter", song.name);
            assert!(song.bars >= 4 && song.bars <= 64, "{} bars {}", song.name, song.bars);
            let steps = song.loop_steps();
            for role in Role::ALL {
                let part = song.parts[role.index()];
                let mut cursor = 0u32;
                let mut last_step = 0u32;
                for &(delta, midi, gate, vel) in part {
                    cursor += delta as u32;
                    assert!(
                        cursor < steps,
                        "{} {}: note at step {cursor} is past the loop ({steps})",
                        song.name,
                        role.name()
                    );
                    if midi != 0 {
                        assert!(midi >= 12, "{} {}: midi {midi}", song.name, role.name());
                        assert!(vel >= 1, "{} {}: velocity {vel}", song.name, role.name());
                        assert!(gate >= 1, "{} {}: zero gate", song.name, role.name());
                        assert!(
                            cursor + gate as u32 <= steps,
                            "{} {}: note at {cursor} overruns the loop",
                            song.name,
                            role.name()
                        );
                        if role.is_drum() {
                            assert!(
                                (28..=87).contains(&midi),
                                "{} {}: percussion note {midi} is not a GM drum",
                                song.name,
                                role.name()
                            );
                        }
                    } else {
                        assert_eq!(gate, 0, "{} {}: rest carries a gate", song.name, role.name());
                    }
                    last_step = cursor;
                }
                assert!(last_step < steps, "{} {} runs past the loop", song.name, role.name());
            }
            // Sections tile the loop exactly, in order.
            let mut bar = 0u16;
            for &(start, len) in song.sections {
                assert_eq!(start, bar, "{}: section gap at bar {bar}", song.name);
                assert!(len > 0, "{}: empty section", song.name);
                bar += len;
            }
            assert_eq!(bar, song.bars, "{}: sections do not cover the loop", song.name);
        }
    }

    #[test]
    fn every_song_uses_a_band_of_roles_and_a_live_mix() {
        for song in SONGS {
            assert!(song.note_count() > 100, "{}: only {} notes", song.name, song.note_count());
            assert!(!song.parts[Role::Bass.index()].is_empty() || !song.parts[Role::Pad.index()].is_empty());
            let used = Role::ALL
                .iter()
                .filter(|r| !song.parts[r.index()].is_empty())
                .count();
            assert!(used >= 4, "{}: only {used} roles carry notes", song.name);
            // Every role the song uses is audible in its mix.
            for role in Role::ALL {
                if !song.parts[role.index()].is_empty() {
                    assert!(
                        song.style.mix[role.index()] > 0.0,
                        "{} {} has notes but no mix gain",
                        song.name,
                        role.name()
                    );
                }
            }
            for role in Role::ALL {
                assert!(
                    (0.0..=1.5).contains(&song.style.mix[role.index()]),
                    "{} {}: unrealistic mix gain {}",
                    song.name,
                    role.name(),
                    song.style.mix[role.index()]
                );
            }
            assert!(song.loop_seconds() > 4.0, "{}: {:.1}s loop is too short", song.name, song.loop_seconds());
            assert!(song.loop_seconds() < 120.0, "{}: {:.1}s loop is too long", song.name, song.loop_seconds());
        }
    }

    #[test]
    fn role_caps_are_a_small_polyphony_budget() {
        let total: usize = Role::ALL.iter().map(|r| r.cap()).sum();
        assert!(total <= synth::MAX_VOICES, "{total} caps over the voice budget");
        assert_eq!(Role::Bass.cap(), 1);
        assert_eq!(Role::Lead.cap(), 2);
        assert!(Role::Pad.cap() <= 4);
    }

    #[test]
    fn floor_names_resolve_to_songs() {
        assert_eq!(song_by_floor("sewer").map(|s| s.floor), Some("Sewer"));
        assert_eq!(song_by_floor("flatbed").map(|s| s.floor), Some("Flatbed"));
        assert_eq!(song_by_floor("trailer").map(|s| s.floor), Some("Flatbed"));
        assert_eq!(song_by_floor("outfall").map(|s| s.floor), Some("Outfall"));
        assert!(song_by_floor("kitchen").is_some());
        assert!(song_by_floor("submarine").is_none());
    }

    #[test]
    fn every_song_has_a_valid_16bit_palette() {
        // Style::base() defaults to the legacy chip palette.
        let base = Style::base();
        assert_eq!(base.drum_kit, DrumKit::Chip);
        assert!(base.patches.iter().all(|&p| p == Patch::Chip));
        assert_eq!(base.echo_ms, 0.0);

        // Every floor song defines its own 16-bit kit, pitched patches, echo, and mix.
        for song in SONGS {
            let st = song.style;
            assert_ne!(st.drum_kit, DrumKit::Chip, "{} missing 16-bit drum kit", song.name);
            for role in [Role::Bass, Role::Pad, Role::Arp, Role::Lead, Role::Stab, Role::Fx] {
                assert_ne!(
                    st.patches[role.index()],
                    Patch::Chip,
                    "{} {} still on Chip patch",
                    song.name,
                    role.name()
                );
            }
            for role in Role::ALL {
                if !song.parts[role.index()].is_empty() {
                    let g = st.mix_16[role.index()];
                    assert!(
                        g > 0.0 && g <= 1.5,
                        "{} {}: unrealistic 16-bit mix gain {g}",
                        song.name,
                        role.name()
                    );
                }
                assert!(
                    (0.0..=1.0).contains(&st.echo_send[role.index()]),
                    "{} {}: echo send out of range",
                    song.name,
                    role.name()
                );
            }
            assert!(
                (160.0..=260.0).contains(&st.echo_ms),
                "{}: echo_ms {} out of [160, 260]",
                song.name,
                st.echo_ms
            );
            assert!(
                (0.15..=0.45).contains(&st.echo_feedback),
                "{}: echo_feedback {} out of [0.15, 0.45]",
                song.name,
                st.echo_feedback
            );
            assert!(st.master_lp_hz > 6_000.0, "{}: master_lp_hz too low", song.name);
        }
    }
}
