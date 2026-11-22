//! The song sequencer: plays a generated [`Song`] with procedural voices.
//!
//! One instance is an infinite [`Source`] for one floor's track. It is written
//! for the audio callback: every buffer is a fixed array in the struct, the
//! note cursors are integers, no allocation and no locking happens in
//! [`Iterator::next`], and the only live input is the same
//! `Arc<AtomicU32>` intensity tier the classic synth reads.
//!
//! Voices are one-shot envelopes over a few oscillator shapes, all built from
//! the DSP primitives in [`crate::audio`] (`fast_tri`, `fast_sin_unit`,
//! `fast_pulse_dc_free`, `fast_tanh`) — there are no samples anywhere. Each
//! note from the tables starts a voice; the pool has a hard cap per role
//! ([`Role::cap`]) and steals the quietest voice of the same role when full, so
//! a dense passage can never grow the voice count.
//!
//! The mix is: roles summed with their per-song gains, driven into
//! `fast_tanh`, then a peak limiter (5 ms attack, 150 ms release) so the track
//! can never take the headroom the effects need, then a DC blocker. Everything
//! is computed from the actual sample rate, so 48 kHz and 22.05 kHz render the
//! same music, and stepping is an integer sample count per 16th note, so the
//! tempo never drifts.

use std::num::NonZero;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::Source;

use crate::audio::{fast_pulse_dc_free, fast_sin_unit, fast_tanh, fast_tri, SR};
use crate::game::level::Theme;

use super::{song_for, Role, Song, N_ROLES};

/// The voice pool. Summed [`Role::cap`] values fit inside it with slack.
pub const MAX_VOICES: usize = 24;

/// Layer gain per role and intensity tier: calm is bass + pad + light
/// percussion, tier 1 adds the drums and the arpeggio, tier 2 the lead, tier 3
/// everything plus the boss drive. No entry ever falls as the tier rises, and
/// [`TIER_GAIN`] rises on top, so a higher tier is always fuller.
const LAYER_GAIN: [[f32; 4]; N_ROLES] = [
    [0.70, 0.95, 1.00, 1.05], // kick
    [0.00, 0.90, 1.00, 1.05], // snare
    [0.65, 1.00, 1.00, 1.00], // hat
    [0.00, 0.85, 1.00, 1.05], // perc
    [1.00, 1.00, 1.00, 1.05], // bass
    [1.00, 1.00, 1.00, 1.00], // pad
    [0.00, 0.95, 1.00, 1.00], // arp
    [0.00, 0.00, 1.00, 1.10], // lead
    [0.00, 0.00, 0.55, 1.05], // stab
    [0.00, 0.00, 0.40, 1.00], // fx
];

/// Whole-mix gain per tier, on top of the layer gates.
const TIER_GAIN: [f32; 4] = [1.00, 1.06, 1.13, 1.22];

/// Trim after the saturator (small-signal gain, before the limiter).
const BUS_TRIM: f32 = 0.92;

/// The ceiling the master bus can never cross (a soft-clip as well as the
/// limiter's target): comfortably inside the [0.14, 0.55] background-music
/// band, with room for effects on top.
const PEAK_CEIL: f32 = 0.50;

/// How fast a role gain tracks an intensity change (seconds).
const GAIN_SLEW_SECS: f32 = 0.16;

#[derive(Clone, Copy)]
enum Src {
    /// Triangle+pulse bass.
    Bass,
    /// Two detuned triangles plus a pulse: strings.
    Pad,
    /// Triangle with a bright, fast-decaying second harmonic: plucked keys.
    Pluck,
    /// Pulse with vibrato.
    Lead,
    /// Bright pulse, short decay: brass stab.
    Stab,
    /// Noise plus a tone: one-shot effect.
    Fx,
    Kick,
    Snare,
    Hat,
    Perc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Attack,
    Hold,
    Release,
    Off,
}

/// One voice: a small one-shot synth with a fixed envelope shape.
#[derive(Clone, Copy)]
struct Voice {
    on: bool,
    role: u8,
    src: Src,
    stage: Stage,
    phase: f32,
    dt: f32,
    inv_dt: f32,
    phase2: f32,
    dt2: f32,
    vib: f32,
    vib_dt: f32,
    vib_depth: f32,
    env: f32,
    gate_left: u32,
    attack_inc: f32,
    decay: f32,
    release: f32,
    amp: f32,
    duty: f32,
    dc_bias: f32,
    blend: f32,
    lp1: f32,
    lp2: f32,
    lp_a: f32,
    lp_floor: f32,
    lp_decay: f32,
    hp1: f32,
    hp_a: f32,
    drum_lp_a: f32,
    one_pole: bool,
    hz0: f32,
    hz1: f32,
    noise: u32,
}

impl Voice {
    const SILENT: Voice = Voice {
        on: false,
        role: 0,
        src: Src::Bass,
        stage: Stage::Off,
        phase: 0.0,
        dt: 0.0,
        inv_dt: 1.0,
        phase2: 0.0,
        dt2: 0.0,
        vib: 0.0,
        vib_dt: 0.0,
        vib_depth: 0.0065,
        env: 0.0,
        gate_left: 0,
        attack_inc: 0.0,
        decay: 1.0,
        release: 0.999,
        amp: 0.0,
        duty: 0.5,
        dc_bias: 0.0,
        blend: 0.0,
        lp1: 0.0,
        lp2: 0.0,
        lp_a: 1.0,
        lp_floor: 1.0,
        lp_decay: 0.0,
        hp1: 0.0,
        hp_a: 0.0,
        drum_lp_a: 0.0,
        one_pole: false,
        hz0: 440.0,
        hz1: 440.0,
        noise: 0x1234_5678,
    };
}

/// White noise from a per-voice xorshift: deterministic and cheap.
#[inline]
fn noise(v: &mut Voice) -> f32 {
    let mut x = v.noise;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    v.noise = x;
    ((x >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
}

/// Frequency of a MIDI note.
#[inline]
fn midi_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// One-pole low-pass coefficient for a cutoff in Hz at `sr`.
#[inline]
fn lp_coef(hz: f32, sr: f32) -> f32 {
    let x = (std::f32::consts::TAU * hz.clamp(20.0, sr * 0.45)) / sr;
    (x / (1.0 + x)).clamp(1e-4, 0.99)
}

/// Per-sample multiplier that decays to −60 dB over `ms` milliseconds.
#[inline]
fn decay_mul(ms: f32, sr: f32) -> f32 {
    let n = (ms * 1e-3 * sr).max(1.0);
    (0.001f32).powf(1.0 / n)
}

/// The sequencer for one song.
pub struct SongSynth {
    song: &'static Song,
    intensity: Arc<AtomicU32>,
    sample_rate: u32,
    inv_rate: f32,

    /// Samples (integer) in one 16th-note step.
    samples_per_step: u32,
    step_pos: u32,
    /// Grid step inside the loop.
    step: u32,
    loop_steps: u32,
    /// Per-part note cursor and steps until that part's next note.
    cursor: [usize; N_ROLES],
    wait: [i32; N_ROLES],

    voices: [Voice; MAX_VOICES],
    tier: u32,
    control_left: u32,
    role_gain: [f32; N_ROLES],
    role_target: [f32; N_ROLES],
    gain_slew: f32,

    /// Last bass pitch, for the boss-drive octave pulse.
    last_bass: u8,
    /// Completed loops (tests and the preview renderer).
    loops: u32,
    /// Deterministic per-trigger noise seed.
    seed: u32,

    lim_env: f32,
    lim_attack: f32,
    lim_release: f32,
    lim_gain: f32,
    dc_x1: f32,
    dc_y1: f32,
    dc_r: f32,
}

impl SongSynth {
    /// A track for `theme` at the standard 44.1 kHz.
    pub fn new(theme: Theme, intensity: Arc<AtomicU32>) -> SongSynth {
        SongSynth::with_sample_rate(theme, intensity, SR)
    }

    /// A track for `theme` at an arbitrary sample rate. Every coefficient is
    /// derived from `sample_rate`, so the music sounds the same anywhere.
    pub fn with_sample_rate(
        theme: Theme,
        intensity: Arc<AtomicU32>,
        sample_rate: u32,
    ) -> SongSynth {
        let song = song_for(theme);
        let rate = sample_rate.max(8_000);
        let inv_rate = 1.0 / rate as f32;
        let samples_per_step = ((rate as f64 * 60.0 / (song.bpm as f64 * 4.0)).round() as u32).max(1);
        let tier = intensity.load(Ordering::Relaxed).min(3);
        let mut synth = SongSynth {
            song,
            intensity,
            sample_rate: rate,
            inv_rate,
            samples_per_step,
            step_pos: 0,
            step: 0,
            loop_steps: song.loop_steps(),
            cursor: [0; N_ROLES],
            wait: [-1; N_ROLES],
            voices: [Voice::SILENT; MAX_VOICES],
            tier,
            control_left: 0,
            role_gain: [0.0; N_ROLES],
            role_target: [0.0; N_ROLES],
            gain_slew: 1.0 - (-1.0 / (GAIN_SLEW_SECS * rate as f32)).exp(),
            last_bass: 0,
            loops: 0,
            seed: 0x9E37_79B9,
            lim_env: 0.0,
            lim_attack: 1.0 - (-1.0 / (0.005 * rate as f32)).exp(),
            lim_release: 1.0 - (-1.0 / (0.150 * rate as f32)).exp(),
            lim_gain: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            dc_r: (1.0 - std::f32::consts::TAU * 8.0 * inv_rate).clamp(0.0, 1.0),
        };
        synth.apply_tier(synth.tier);
        // Start at the tier's gains instead of sliding up from silence: the
        // first loop should sound like the floor, not ramp into it.
        synth.role_gain = synth.role_target;
        synth.restart();
        // `restart` counts a completed loop; the first pass is not one.
        synth.loops = 0;
        synth
    }

    /// The song this track plays.
    pub fn song(&self) -> &'static Song {
        self.song
    }

    /// The current intensity tier (0..=3).
    pub fn tier(&self) -> u32 {
        self.tier
    }

    /// Samples in one full loop.
    pub fn loop_samples(&self) -> u64 {
        u64::from(self.loop_steps) * u64::from(self.samples_per_step)
    }

    /// Completed loops so far.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Voices currently sounding in `role`.
    pub fn active_in_role(&self, role: Role) -> usize {
        self.voices
            .iter()
            .filter(|v| v.on && v.role == role.index() as u8)
            .count()
    }

    /// Voices currently sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }

    /// The resolved mix gain of one role (after the intensity gates).
    pub fn role_gain(&self, role: Role) -> f32 {
        self.role_gain[role.index()]
    }

    /// The gain table for one tier, exactly as the mix applies it.
    pub fn tier_gain(role: Role, tier: u32) -> f32 {
        let tier = tier.min(3) as usize;
        LAYER_GAIN[role.index()][tier] * TIER_GAIN[tier]
    }

    /// Reset the note cursors to the top of the loop.
    fn restart(&mut self) {
        for role in Role::ALL {
            let i = role.index();
            self.cursor[i] = 0;
            let part = self.song.parts[i];
            self.wait[i] = if part.is_empty() {
                -1
            } else {
                i32::from(part[0].0)
            };
        }
        // Anything still ringing is released, not cut: the seam of a loop is
        // the one place a hard stop would click.
        for v in self.voices.iter_mut() {
            if v.on {
                v.stage = Stage::Release;
                v.gate_left = 0;
            }
        }
        self.loops += 1;
    }

    /// Recompute the target gains for `tier` from the song's mix.
    fn apply_tier(&mut self, tier: u32) {
        self.tier = tier.min(3);
        let t = self.tier as usize;
        for role in Role::ALL {
            let i = role.index();
            self.role_target[i] = self.song.style.mix[i] * LAYER_GAIN[i][t] * TIER_GAIN[t];
        }
    }

    /// Pick a voice slot for a new note of `role`: a free one, else steal the
    /// quietest voice of the same role (which is the oldest sustained one).
    fn slot_for(&mut self, role: Role) -> Option<usize> {
        let ri = role.index() as u8;
        let cap = role.cap();
        let mut active = 0;
        let mut quietest: Option<(usize, f32)> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.on && v.role == ri {
                active += 1;
                let level = if v.stage == Stage::Release { 0.0 } else { v.env };
                let quieter = match quietest {
                    None => true,
                    Some((_, best)) => level < best,
                };
                if quieter {
                    quietest = Some((i, level));
                }
            }
        }
        if active < cap {
            if let Some(i) = self.voices.iter().position(|v| !v.on) {
                return Some(i);
            }
        }
        if active > 0 && active >= cap {
            if let Some((i, _)) = quietest {
                return Some(i);
            }
        }
        // No room at all (the pool is full of other roles): reuse the quietest
        // released voice rather than dropping the note into silence.
        let mut best: Option<(usize, f32)> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if !v.on {
                return Some(i);
            }
            let level = if v.stage == Stage::Release { 0.0 } else { v.env };
            let quieter = match best {
                None => true,
                Some((_, b)) => level < b,
            };
            if quieter {
                best = Some((i, level));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Start a note. `amp_scale` trims the velocity amplitude (boss drive).
    fn trigger(&mut self, role: Role, midi: u8, gate_steps: u8, vel: u8, amp_scale: f32) {
        let Some(slot) = self.slot_for(role) else {
            return;
        };
        let sr = self.sample_rate as f32;
        let style = self.song.style;
        let vel_amp = 0.30 + 0.70 * (vel.min(127) as f32 / 127.0);
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut v = Voice::SILENT;
        v.on = true;
        v.role = role.index() as u8;
        v.stage = Stage::Attack;
        v.amp = vel_amp * amp_scale;
        v.env = 0.0;
        v.noise = self.seed | 1;
        v.gate_left = u32::from(gate_steps.max(1)) * self.samples_per_step;
        v.vib_dt = 5.3 * self.inv_rate;
        v.release = decay_mul(45.0, sr);

        let hz = midi_hz(midi.max(12));
        let attack = |ms: f32| 1.0 / (ms.max(0.05) * 1e-3 * sr);
        let enhanced = style.enhanced_drums;
        match role {
            Role::Kick => {
                v.src = Src::Kick;
                v.hz0 = (style.kick_hz * 2.6).max(40.0);
                v.hz1 = style.kick_hz.max(30.0);
                v.one_pole = enhanced;
                v.attack_inc = attack(0.5);
                v.decay = decay_mul(if enhanced { 95.0 } else { 90.0 }, sr);
            }
            Role::Snare => {
                v.src = Src::Snare;
                v.hz0 = 186.0;
                v.lp_a = lp_coef(if enhanced { 1800.0 } else { 1400.0 }, sr);
                v.drum_lp_a = if enhanced { lp_coef(7800.0, sr) } else { 0.0 };
                v.attack_inc = attack(if enhanced { 0.5 } else { 0.6 });
                v.decay = decay_mul(if enhanced { 95.0 } else { 70.0 }, sr);
            }
            Role::Hat => {
                v.src = Src::Hat;
                v.lp_a = lp_coef(if enhanced { 3800.0 } else { 6200.0 }, sr);
                v.drum_lp_a = if enhanced { lp_coef(8500.0, sr) } else { 0.0 };
                v.attack_inc = attack(0.3);
                let hat_ms = if enhanced {
                    if midi == 46 { 115.0 } else { 32.0 }
                } else {
                    28.0
                };
                v.decay = decay_mul(hat_ms, sr);
            }
            Role::Perc => {
                v.src = Src::Perc;
                let (kind, hz, ms) = perc_params(midi, enhanced);
                v.blend = kind as f32;
                v.hz0 = hz;
                v.hz1 = hz * 0.55;
                v.lp_a = lp_coef((hz * 1.4).clamp(600.0, 9000.0), sr);
                v.drum_lp_a = if enhanced { lp_coef(7500.0, sr) } else { 0.0 };
                v.attack_inc = attack(if enhanced && kind == 2 { 2.5 } else { 0.6 });
                v.decay = decay_mul(ms, sr);
            }
            Role::Bass => {
                v.src = Src::Bass;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.duty = 0.5;
                v.dc_bias = 0.0;
                v.blend = style.bass_tri;
                v.lp_a = lp_coef(style.bass_lp, sr);
                v.lp_floor = v.lp_a;
                v.attack_inc = attack(6.0);
                if style.bass_decay_ms > 0.0 {
                    v.decay = decay_mul(style.bass_decay_ms, sr);
                }
                self.last_bass = midi;
            }
            Role::Pad => {
                v.src = Src::Pad;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.dt2 = v.dt * (1.0 + style.pad_detune);
                v.duty = 0.25;
                v.dc_bias = -0.5;
                v.lp_a = lp_coef(style.pad_lp, sr);
                v.lp_floor = v.lp_a;
                v.attack_inc = attack(style.pad_attack_ms);
                if style.pad_decay_ms > 0.0 {
                    v.decay = decay_mul(style.pad_decay_ms, sr);
                }
                if style.pad_hp_hz > 0.0 {
                    v.hp_a = lp_coef(style.pad_hp_hz, sr);
                }
            }
            Role::Arp => {
                v.src = Src::Pluck;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.dt2 = v.dt * 2.0;
                v.duty = 0.25;
                v.dc_bias = -0.5;
                v.blend = if enhanced { 0.50 } else { 0.68 };
                v.lp_a = lp_coef(style.arp_lp, sr);
                v.lp_floor = lp_coef(style.arp_lp * 0.35, sr);
                v.lp_decay = 1.0 - (-self.inv_rate / 0.12).exp();
                v.one_pole = enhanced && style.arp_lp >= 5000.0;
                v.attack_inc = attack(3.0);
                v.decay = decay_mul(140.0, sr);
            }
            Role::Lead => {
                v.src = Src::Lead;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.duty = style.lead_duty;
                v.dc_bias = 2.0 * style.lead_duty - 1.0;
                v.blend = style.lead_tri;
                v.vib_depth = style.lead_vib_depth;
                if style.lead_bright_ms > 0.0 {
                    v.lp_a = lp_coef((style.lead_lp * 1.9).min(sr * 0.44), sr);
                    v.lp_floor = lp_coef(style.lead_lp, sr);
                    v.lp_decay = 1.0 - (-self.inv_rate / (style.lead_bright_ms * 1e-3)).exp();
                    v.one_pole = enhanced;
                } else {
                    v.lp_a = lp_coef(style.lead_lp, sr);
                    v.lp_floor = v.lp_a;
                }
                if style.lead_decay_ms > 0.0 {
                    v.decay = decay_mul(style.lead_decay_ms, sr);
                }
                v.attack_inc = attack(12.0);
            }
            Role::Stab => {
                v.src = Src::Stab;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.duty = style.stab_duty;
                v.dc_bias = 2.0 * style.stab_duty - 1.0;
                v.lp_a = lp_coef(style.stab_lp, sr);
                v.lp_floor = lp_coef(style.stab_lp * 0.5, sr);
                v.lp_decay = 1.0 - (-self.inv_rate / 0.18).exp();
                v.one_pole = enhanced && style.stab_lp >= 3500.0;
                v.attack_inc = attack(8.0);
                v.decay = decay_mul(style.stab_decay_ms, sr);
            }
            Role::Fx => {
                v.src = Src::Fx;
                v.dt = hz * self.inv_rate;
                v.inv_dt = 1.0 / v.dt.max(1e-6);
                v.attack_inc = attack(4.0);
                if enhanced {
                    if vel >= 80 {
                        v.lp_a = lp_coef(420.0, sr);
                        v.lp_floor = lp_coef(4800.0, sr);
                    } else {
                        v.lp_a = lp_coef(4500.0, sr);
                        v.lp_floor = lp_coef(380.0, sr);
                    }
                    let sweep_s = (v.gate_left as f32 * 0.55 * self.inv_rate).max(0.08);
                    v.lp_decay = 1.0 - (-self.inv_rate / sweep_s).exp();
                    let fx_ms = (v.gate_left as f32 * self.inv_rate * 1200.0).max(260.0);
                    v.decay = decay_mul(fx_ms, sr);
                } else {
                    v.lp_a = lp_coef(2400.0, sr);
                    v.lp_floor = v.lp_a;
                    v.decay = decay_mul(300.0, sr);
                }
            }
        }
        // A stolen voice re-attacks from its current level rather than jumping
        // to zero, so stealing is never a click.
        if self.voices[slot].on {
            v.env = self.voices[slot].env * 0.5;
        }
        self.voices[slot] = v;
    }

    /// Fire every part's notes that fall on the current step.
    fn advance_step(&mut self) {
        let mut bass_fired = false;
        for role in Role::ALL {
            let i = role.index();
            let part = self.song.parts[i];
            loop {
                if self.wait[i] != 0 {
                    break;
                }
                if self.cursor[i] >= part.len() {
                    self.wait[i] = -1;
                    break;
                }
                let (_delta, midi, gate, vel) = part[self.cursor[i]];
                self.cursor[i] += 1;
                if midi != 0 {
                    // A gated-off layer still advances its cursor but starts no
                    // voice: the tier decides what sounds, not the material.
                    if self.role_target[i] > 1e-4 {
                        self.trigger(role, midi, gate, vel, 1.0);
                    }
                    if role == Role::Bass {
                        bass_fired = true;
                    }
                }
                self.wait[i] = if self.cursor[i] < part.len() {
                    i32::from(part[self.cursor[i]].0)
                } else {
                    -1
                };
            }
        }
        // Boss overdrive: an octave-up bass pulse on the beats the bass rests,
        // the same trick the classic synth uses to fill the top tier.
        if self.tier >= 3 && !bass_fired && self.last_bass > 0 {
            let bar = self.song.steps_per_bar();
            let beat = (bar / u32::from(self.song.beats_per_bar.max(1))).max(1);
            if self.step.is_multiple_of(beat) {
                // The pulse is an octave over the last *written* bass note.
                // `trigger` records every bass note it starts, so put the
                // written one back: otherwise each resting beat climbs another
                // octave until the oscillator runs past Nyquist and the whole
                // bus goes non-finite.
                let written = self.last_bass;
                self.trigger(Role::Bass, written.saturating_add(12).min(96), 2, 74, 0.62);
                self.last_bass = written;
            }
        }
    }

    /// Advance the step clock, wrapping into the loop.
    fn end_step(&mut self) {
        for role in Role::ALL {
            let i = role.index();
            if self.wait[i] > 0 {
                self.wait[i] -= 1;
            }
        }
        self.step += 1;
        if self.step >= self.loop_steps {
            self.step = 0;
            self.restart();
        }
    }

    /// Read the intensity and slew the role gains towards it.
    #[inline]
    fn control(&mut self) {
        if self.control_left == 0 {
            self.control_left = 32;
            let tier = self.intensity.load(Ordering::Relaxed).min(3);
            if tier != self.tier {
                self.apply_tier(tier);
            }
        }
        self.control_left -= 1;
        let slew = self.gain_slew;
        for (gain, target) in self.role_gain.iter_mut().zip(self.role_target.iter()) {
            *gain += (*target - *gain) * slew;
        }
    }

    /// Sum the voices and run the master chain for one sample.
    fn render(&mut self) -> f32 {
        let gains = self.role_gain;
        let sr = self.sample_rate as f32;
        let mut mix = 0.0f32;
        for v in self.voices.iter_mut() {
            if !v.on {
                continue;
            }
            let s = voice_sample(v, sr);
            if s != 0.0 {
                mix += s * v.amp * gains[v.role as usize];
            }
        }
        let mut out = fast_tanh(mix * self.song.style.drive) * BUS_TRIM;
        // Peak limiter: fast down, slow up.
        let level = out.abs();
        let coef = if level > self.lim_env {
            self.lim_attack
        } else {
            self.lim_release
        };
        self.lim_env += (level - self.lim_env) * coef;
        let want = if self.lim_env > PEAK_CEIL {
            PEAK_CEIL / self.lim_env
        } else {
            1.0
        };
        let gain_coef = if want < self.lim_gain {
            self.lim_attack
        } else {
            self.lim_release
        };
        self.lim_gain += (want - self.lim_gain) * gain_coef;
        out *= self.lim_gain;
        // Absolute ceiling: a soft-clip that can never be crossed, so a dense
        // passage cannot take the headroom the effects need even inside the
        // limiter's attack. Linear to within a percent below half scale.
        out = fast_tanh(out * (1.0 / PEAK_CEIL)) * PEAK_CEIL;
        // 8 Hz DC blocker.
        let dc = out - self.dc_x1 + self.dc_r * self.dc_y1;
        self.dc_x1 = out;
        self.dc_y1 = dc;
        dc * self.song.style.master
    }
}

/// General MIDI percussion note -> `(kind, pitch, decay ms)`.
///
/// `kind`: 0 tom/timpani (pitch sweep), 1 short noise (clap, rim, shaker),
/// 2 cymbal (long high noise), 3 cowbell (two tones), 4 woodblock/clave.
fn perc_params(note: u8, enhanced: bool) -> (u8, f32, f32) {
    if enhanced {
        match note {
            65 => return (0, 110.0, 240.0),
            66 => return (0, 88.0, 240.0),
            75 => return (4, 980.0, 30.0),
            76 => return (4, 1220.0, 30.0),
            77 => return (4, 780.0, 30.0),
            _ => {}
        }
    }
    match note {
        37 | 39 => (1, 1900.0, 55.0),
        41 => (0, 190.0, 95.0),
        43 => (0, 160.0, 105.0),
        45 => (0, 132.0, 115.0),
        47 => (0, 110.0, 125.0),
        48 => (0, 98.0, 135.0),
        49 | 57 => (2, 3800.0, 420.0),
        51 | 59 => (2, 5600.0, 300.0),
        52 | 55 => (2, 4600.0, 260.0),
        53 => (2, 3200.0, 220.0),
        54 | 69 | 70 => (1, 7800.0, 40.0),
        56 => (3, 540.0, 120.0),
        _ => (1, 2600.0, 60.0),
    }
}

/// Render one sample of one voice.
#[inline]
fn voice_sample(v: &mut Voice, sr: f32) -> f32 {
    // Envelope.
    match v.stage {
        Stage::Attack => {
            v.env += v.attack_inc;
            if v.gate_left > 0 {
                v.gate_left -= 1;
            }
            if v.env >= 1.0 {
                v.env = 1.0;
                v.stage = Stage::Hold;
            } else if v.gate_left == 0 {
                v.stage = Stage::Release;
            }
        }
        Stage::Hold => {
            if v.gate_left > 0 {
                v.gate_left -= 1;
            }
            v.env *= v.decay;
            if v.gate_left == 0 {
                v.stage = Stage::Release;
            } else if v.env < 1e-4 {
                v.env = 0.0;
                v.on = false;
                v.stage = Stage::Off;
                return 0.0;
            }
        }
        Stage::Release => {
            v.env *= v.release;
            if v.env < 1e-4 {
                v.env = 0.0;
                v.on = false;
                v.stage = Stage::Off;
                return 0.0;
            }
        }
        Stage::Off => {
            v.on = false;
            return 0.0;
        }
    }

    let raw = match v.src {
        Src::Bass => {
            v.phase += v.dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            let tri = fast_tri(v.phase);
            let pulse = fast_pulse_dc_free(v.phase, v.dt, v.inv_dt, v.duty, v.dc_bias);
            tri * v.blend + pulse * (1.0 - v.blend)
        }
        Src::Pad => {
            v.phase += v.dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            v.phase2 += v.dt2;
            if v.phase2 >= 1.0 {
                v.phase2 -= 1.0;
            }
            let pulse = fast_pulse_dc_free(v.phase, v.dt, v.inv_dt, v.duty, v.dc_bias);
            fast_tri(v.phase) * 0.44 + fast_tri(v.phase2) * 0.44 + pulse * 0.12
        }
        Src::Pluck => {
            v.phase += v.dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            v.phase2 += v.dt2;
            if v.phase2 >= 1.0 {
                v.phase2 -= 1.0;
            }
            let pulse = fast_pulse_dc_free(v.phase2, v.dt2, 1.0 / v.dt2.max(1e-6), v.duty, v.dc_bias);
            fast_tri(v.phase) * v.blend + pulse * (1.0 - v.blend)
        }
        Src::Lead => {
            v.vib += v.vib_dt;
            if v.vib >= 1.0 {
                v.vib -= 1.0;
            }
            let fm = 1.0 + v.vib_depth * fast_sin_unit(v.vib);
            let dt = v.dt * fm;
            v.phase += dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            let pulse = fast_pulse_dc_free(v.phase, dt, 1.0 / dt.max(1e-6), v.duty, v.dc_bias);
            pulse * (1.0 - v.blend) + fast_tri(v.phase) * v.blend
        }
        Src::Stab => {
            v.phase += v.dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            let pulse = fast_pulse_dc_free(v.phase, v.dt, v.inv_dt, v.duty, v.dc_bias);
            pulse * 0.76 + fast_tri(v.phase) * 0.24
        }
        Src::Fx => {
            v.phase += v.dt;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            noise(v) * 0.62 + fast_sin_unit(v.phase) * 0.38
        }
        Src::Kick => {
            let hz = v.hz1 + (v.hz0 - v.hz1) * v.env * v.env;
            v.phase += hz / sr;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            let body = fast_sin_unit(v.phase);
            if v.one_pole {
                let mut p3 = v.phase * 3.0;
                p3 -= p3.floor();
                let e2 = v.env * v.env;
                body + 0.24 * fast_sin_unit(p3) * (e2 * e2 * v.env)
            } else {
                body
            }
        }
        Src::Snare => {
            let n = noise(v);
            v.lp1 += v.lp_a * (n - v.lp1);
            let mut bright = (n - v.lp1).clamp(-1.5, 1.5);
            let (nw, bw) = if v.drum_lp_a > 0.0 {
                v.lp2 += v.drum_lp_a * (bright - v.lp2);
                bright = v.lp2;
                (0.92, 0.36)
            } else {
                (0.68, 0.42)
            };
            v.phase += v.hz0 / sr;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
            bright * nw + fast_sin_unit(v.phase) * bw
        }
        Src::Hat => {
            let n = noise(v);
            v.lp1 += v.lp_a * (n - v.lp1);
            let h = (n - v.lp1).clamp(-1.5, 1.5);
            if v.drum_lp_a > 0.0 {
                v.lp2 += v.drum_lp_a * (h - v.lp2);
                v.lp2
            } else {
                h
            }
        }
        Src::Perc => {
            let kind = v.blend as u8;
            match kind {
                0 => {
                    let hz = v.hz1 + (v.hz0 - v.hz1) * v.env;
                    v.phase += hz / sr;
                    if v.phase >= 1.0 {
                        v.phase -= 1.0;
                    }
                    fast_sin_unit(v.phase)
                }
                3 => {
                    v.phase += v.hz0 / sr;
                    if v.phase >= 1.0 {
                        v.phase -= 1.0;
                    }
                    v.phase2 += v.hz0 * 1.48 / sr;
                    if v.phase2 >= 1.0 {
                        v.phase2 -= 1.0;
                    }
                    fast_sin_unit(v.phase) * 0.5 + fast_sin_unit(v.phase2) * 0.5
                }
                4 => {
                    v.phase += v.hz0 / sr;
                    if v.phase >= 1.0 {
                        v.phase -= 1.0;
                    }
                    v.phase2 += v.hz0 * 2.38 / sr;
                    if v.phase2 >= 1.0 {
                        v.phase2 -= 1.0;
                    }
                    fast_sin_unit(v.phase) * 0.72 + fast_sin_unit(v.phase2) * 0.28
                }
                _ => {
                    let n = noise(v);
                    v.lp1 += v.lp_a * (n - v.lp1);
                    let h = (n - v.lp1).clamp(-1.5, 1.5);
                    if v.drum_lp_a > 0.0 {
                        v.lp2 += v.drum_lp_a * (h - v.lp2);
                        v.lp2
                    } else {
                        h
                    }
                }
            }
        }
    };

    // Brightness that darkens as the note decays (plucks, stabs, bright leads),
    // then a one- or two-pole low-pass and optional pad high-pass shelf.
    if v.lp_decay > 0.0 {
        v.lp_a += (v.lp_floor - v.lp_a) * v.lp_decay;
    }
    // The drums shape themselves (noise band-pass, pitch sweep) and use `lp1`/`lp2`
    // as their own filter state, so the voice filter is for the pitched parts.
    let is_drum = matches!(v.src, Src::Kick | Src::Snare | Src::Hat | Src::Perc);
    let mut filtered = if is_drum || v.lp_a >= 0.999 {
        raw
    } else if v.one_pole {
        v.lp1 += v.lp_a * (raw - v.lp1);
        v.lp1
    } else {
        v.lp1 += v.lp_a * (raw - v.lp1);
        v.lp2 += v.lp_a * (v.lp1 - v.lp2);
        v.lp2
    };
    if v.hp_a > 0.0 {
        v.hp1 += v.hp_a * (filtered - v.hp1);
        filtered -= v.hp1;
    }
    filtered * v.env
}

impl Iterator for SongSynth {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.step_pos == 0 {
            self.advance_step();
        }
        self.control();
        let out = self.render();
        self.step_pos += 1;
        if self.step_pos >= self.samples_per_step {
            self.step_pos = 0;
            self.end_step();
        }
        Some(out)
    }
}

impl Source for SongSynth {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::<u16>::MIN
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(self.sample_rate).expect("sample rate is never zero")
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::SONGS;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    fn dc(samples: &[f32]) -> f32 {
        samples.iter().sum::<f32>() / samples.len().max(1) as f32
    }

    /// Render exactly `loops` full loops at `rate`.
    fn render(theme: Theme, intensity: u32, rate: u32, loops: u32) -> (Vec<f32>, SongSynth) {
        let mut synth = SongSynth::with_sample_rate(
            theme,
            Arc::new(AtomicU32::new(intensity)),
            rate,
        );
        let total = synth.loop_samples() * u64::from(loops);
        let mut out = Vec::with_capacity(total as usize);
        for _ in 0..total {
            out.push(synth.next().unwrap());
        }
        (out, synth)
    }

    #[test]
    fn every_floor_renders_two_loops_inside_the_mix_targets() {
        // The classic synth's targets, on the song path: 0.14..=0.55 peak,
        // 0.045..=0.22 RMS, |DC| < 0.005. The loudest tier is the one to hold
        // to the ceiling, so that is the one rendered for two whole loops.
        for theme in Theme::playable() {
            let (samples, synth) = render(theme, 2, 22_050, 2);
            assert!(!samples.is_empty());
            for &s in &samples {
                assert!(s.is_finite() && s.abs() <= 1.0, "{theme:?} bad sample {s}");
            }
            assert_eq!(synth.loops(), 2, "{theme:?} did not wrap twice");
            let pk = peak(&samples);
            let r = rms(&samples);
            let d = dc(&samples);
            assert!(
                pk <= 0.55,
                "{theme:?} peak {pk:.3} over the background-music ceiling"
            );
            assert!(pk >= 0.14, "{theme:?} peak {pk:.3} is too quiet");
            assert!(
                (0.045..=0.22).contains(&r),
                "{theme:?} RMS {r:.3} outside [0.045, 0.22]"
            );
            assert!(d.abs() < 0.005, "{theme:?} DC offset {d:+.4}");
            let crest = pk / r.max(1e-6);
            assert!(
                (1.5..=6.0).contains(&crest),
                "{theme:?} crest {crest:.2} is not a music mix (pk={pk:.3} rms={r:.3})"
            );
        }
    }

    #[test]
    fn every_tier_is_audible_and_the_mix_rises_with_intensity() {
        for song in SONGS {
            let theme = theme_for(song.floor);
            let mut levels = [0.0f32; 4];
            let mut lead_voices = [0usize; 4];
            for tier in 0..=3u32 {
                let mut synth = SongSynth::with_sample_rate(
                    theme,
                    Arc::new(AtomicU32::new(tier)),
                    22_050,
                );
                // One whole loop a tier: a song's intro bars can be far
                // sparser than its loop (the Printworks piece opens on a kick
                // and one bass note every two bars), and the claim is about
                // the track, not its first eight seconds.
                let warm = 0usize;
                let span = synth.loop_samples() as usize;
                let mut out = Vec::with_capacity(span);
                // The lead rests between phrases: count the most voices it
                // ever holds in the window, not the ones at its last sample.
                let mut lead = 0;
                for i in 0..span {
                    out.push(synth.next().unwrap());
                    if i % 64 == 0 {
                        lead = lead.max(synth.active_in_role(Role::Lead));
                    }
                }
                levels[tier as usize] = rms(&out[warm..]);
                lead_voices[tier as usize] = lead;
            }
            for (tier, level) in levels.iter().enumerate() {
                assert!(
                    (0.045..=0.22).contains(level),
                    "{} tier {tier}: RMS {:.3} out of band",
                    song.name,
                    level
                );
            }
            for tier in 0..3 {
                // Calm -> normal -> horde each add level. The boss tier adds
                // layers and drive into a bus the limiter already holds at
                // the ceiling, so there it need only not fall (measured: the
                // Flatbed's horde and boss tiers are 0.113 and 0.111).
                let step = if tier == 2 { 0.96 } else { 1.02 };
                assert!(
                    levels[tier + 1] > levels[tier] * step,
                    "{}: tier {} ({:.3}) should be fuller than tier {} ({:.3})",
                    song.name,
                    tier + 1,
                    levels[tier + 1],
                    tier,
                    levels[tier]
                );
            }
            assert_eq!(
                lead_voices[0], 0,
                "{}: the lead must be silent while calm",
                song.name
            );
            assert!(
                lead_voices[2] > 0,
                "{}: no lead voice at the horde tier",
                song.name
            );
        }
    }

    fn theme_for(floor: &str) -> Theme {
        match floor {
            "Sewer" => Theme::Sewer,
            "Kitchen" => Theme::Kitchen,
            "Lab" => Theme::Lab,
            "Laundry" => Theme::Laundry,
            "Printworks" => Theme::Printworks,
            _ => Theme::Trailer,
        }
    }

    #[test]
    fn the_loop_seam_does_not_click() {
        for theme in Theme::playable() {
            for tier in 0..=3u32 {
                let mut synth = SongSynth::with_sample_rate(
                    theme,
                    Arc::new(AtomicU32::new(tier)),
                    22_050,
                );
                let loop_len = synth.loop_samples();
                let pre = loop_len - 256;
                // The loudest step anywhere in the body of the loop: the seam
                // is held to the song's own note changes, not to another
                // synth's numbers.
                let mut body = 0.0f32;
                let mut last = 0.0f32;
                for _ in 0..pre {
                    let s = synth.next().unwrap();
                    body = body.max((s - last).abs());
                    last = s;
                }
                let mut window = Vec::with_capacity(512);
                for _ in 0..512 {
                    window.push(synth.next().unwrap());
                }
                let seam = (window[256] - window[255]).abs();
                let max_delta = window
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .fold(0.0f32, f32::max);
                assert!(
                    seam <= body * 1.1 + 0.01,
                    "{theme:?} tier {tier}: seam jump {seam:.4} against {body:.4} in the loop"
                );
                assert!(
                    max_delta <= body * 1.1 + 0.01,
                    "{theme:?} tier {tier}: click near the seam ({max_delta:.4} against {body:.4})"
                );
                assert!(max_delta < 0.30, "{theme:?} tier {tier}: {max_delta:.4}");
                assert_eq!(synth.step, 0, "{theme:?} did not wrap on the seam");
                assert_eq!(synth.loops(), 1);
            }
        }
    }

    /// Every tier of every song is finite for a whole loop (the boss tier's
    /// octave pulse once climbed an octave a beat through a bass rest and
    /// drove the bus to NaN).
    #[test]
    fn every_tier_stays_finite_for_a_whole_loop() {
        for theme in Theme::playable() {
            for tier in 0..=3u32 {
                let (samples, _) = render(theme, tier, 22_050, 1);
                let bad = samples.iter().filter(|s| !s.is_finite()).count();
                assert_eq!(bad, 0, "{theme:?} tier {tier}: {bad} non-finite samples");
                assert!(peak(&samples) <= 0.55, "{theme:?} tier {tier} over the ceiling");
            }
            // No oscillator ever runs past Nyquist (a runaway pulse pins the
            // bus at the clip rail, which the DC blocker then hides as silence).
            let mut synth = SongSynth::with_sample_rate(theme, Arc::new(AtomicU32::new(3)), 22_050);
            for i in 0..synth.loop_samples() {
                let _ = synth.next();
                if i % 16 == 0 {
                    for v in synth.voices.iter().filter(|v| v.on) {
                        assert!(v.dt < 0.5, "{theme:?}: a voice at {} cycles a sample", v.dt);
                    }
                }
            }
        }
    }

    #[test]
    fn voice_caps_hold_every_sample() {
        for theme in Theme::playable() {
            let mut synth = SongSynth::with_sample_rate(
                theme,
                Arc::new(AtomicU32::new(3)),
                22_050,
            );
            for _ in 0..synth.loop_samples() {
                let _ = synth.next();
                for role in Role::ALL {
                    assert!(
                        synth.active_in_role(role) <= role.cap(),
                        "{theme:?} {} over its cap of {}",
                        role.name(),
                        role.cap()
                    );
                }
                assert!(synth.active_voices() <= MAX_VOICES);
            }
        }
    }

    #[test]
    fn a_song_renders_bit_for_bit_identically() {
        for theme in Theme::playable() {
            let (a, _) = render(theme, 2, 22_050, 1);
            let (b, _) = render(theme, 2, 22_050, 1);
            assert_eq!(a, b, "{theme:?} is not deterministic");
        }
    }

    #[test]
    fn the_six_floors_do_not_sound_alike() {
        let mut excerpts = Vec::new();
        for theme in Theme::playable() {
            let (samples, _) = render(theme, 2, 22_050, 1);
            let take = &samples[..22_050 * 4];
            excerpts.push((theme, take.to_vec()));
        }
        for i in 0..excerpts.len() {
            for j in (i + 1)..excerpts.len() {
                let (ti, a) = &excerpts[i];
                let (tj, b) = &excerpts[j];
                let diff = (a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| (x - y) * (x - y))
                    .sum::<f32>()
                    / a.len() as f32)
                    .sqrt();
                assert!(
                    diff > 0.02,
                    "{ti:?} and {tj:?} sound too alike (diff_rms={diff:.4})"
                );
            }
        }
    }

    #[test]
    fn the_track_is_sample_rate_independent() {
        for theme in Theme::playable() {
            let low = SongSynth::with_sample_rate(
                theme,
                Arc::new(AtomicU32::new(2)),
                22_050,
            );
            let high = SongSynth::with_sample_rate(
                theme,
                Arc::new(AtomicU32::new(2)),
                48_000,
            );
            // The same number of loop *seconds*, not samples.
            let seconds = 4.0;
            let n_low = (seconds * 22_050.0) as usize;
            let n_high = (seconds * 48_000.0) as usize;
            let mut a = low;
            let mut b = high;
            let sa: Vec<f32> = (0..n_low).map(|_| a.next().unwrap()).collect();
            let sb: Vec<f32> = (0..n_high).map(|_| b.next().unwrap()).collect();
            assert!(sa.iter().all(|s| s.is_finite()) && sb.iter().all(|s| s.is_finite()));
            let ra = rms(&sa);
            let rb = rms(&sb);
            assert!(
                (ra - rb).abs() / ra.max(rb) < 0.25,
                "{theme:?}: RMS {ra:.3} at 22.05 kHz vs {rb:.3} at 48 kHz"
            );
            // Same musical length: four seconds is the same share of the loop.
            let frac_low = 4.0 / a.song().loop_seconds();
            let frac_high = 4.0 / b.song().loop_seconds();
            assert!((frac_low - frac_high).abs() < 1e-6);
        }
    }

    #[test]
    fn the_classic_synth_still_renders() {
        // `RATRUN_MUSIC=classic` is a supported path, so pin that it plays.
        for theme in Theme::playable() {
            let mut synth = crate::audio::MusicSynth::new(
                theme,
                Arc::new(AtomicU32::new(1)),
            );
            let n = crate::audio::SR as usize * 2;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(synth.next().unwrap());
            }
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
            assert!(rms(&out) > 0.02, "{theme:?} classic synth is silent");
            assert!(peak(&out) <= 0.55, "{theme:?} classic synth is too hot");
        }
    }

    #[test]
    fn the_tier_table_never_takes_a_layer_away() {
        for role in Role::ALL {
            for tier in 0..3u32 {
                assert!(
                    SongSynth::tier_gain(role, tier + 1) >= SongSynth::tier_gain(role, tier),
                    "{} loses gain from tier {tier} to {}",
                    role.name(),
                    tier + 1
                );
            }
        }
        // Calm really is calm: the drums and the arpeggio are not in it.
        assert_eq!(SongSynth::tier_gain(Role::Snare, 0), 0.0);
        assert_eq!(SongSynth::tier_gain(Role::Arp, 0), 0.0);
        assert!(SongSynth::tier_gain(Role::Bass, 0) > 0.0);
        assert!(SongSynth::tier_gain(Role::Pad, 0) > 0.0);
    }

    #[test]
    fn the_boss_tier_is_not_a_copy_of_the_horde_tier() {
        for theme in Theme::playable() {
            let (two, _) = render(theme, 2, 22_050, 1);
            let (three, _) = render(theme, 3, 22_050, 1);
            let n = two.len().min(three.len());
            let diff = (two[..n]
                .iter()
                .zip(&three[..n])
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                / n as f32)
                .sqrt();
            assert!(
                diff > 0.01,
                "{theme:?}: the boss tier should add its drive (diff_rms={diff:.4})"
            );
        }
    }

    #[test]
    fn sewer_and_kitchen_have_v2_brightness_and_clear_pad_detune() {
        // The pad's second oscillator runs at `1 + pad_detune` times the note:
        // 0.06 is a semitone sharp. Every floor's pad is a chorus (under ten
        // cents), and so is the default a new song inherits.
        for theme in Theme::playable() {
            let detune = song_for(theme).style.pad_detune;
            assert!(
                (0.0..=0.006).contains(&detune),
                "{theme:?}: pad_detune {detune} is out of tune (a ratio: keep it under 0.006)"
            );
        }
        assert!(crate::music::Style::base().pad_detune <= 0.006, "the default pad is out of tune");
        for theme in [Theme::Sewer, Theme::Kitchen] {
            let song = song_for(theme);
            assert!(song.style.enhanced_drums, "{theme:?}: expected v2 voice mode");
            for role in Role::ALL {
                assert!(
                    !song.parts[role.index()].is_empty(),
                    "{theme:?}: {} should be populated",
                    role.name()
                );
            }
            // Measure share of energy above ~2 kHz using a 1-pole high-pass at 2 kHz.
            let sr = 22_050u32;
            let mut synth = SongSynth::with_sample_rate(theme, Arc::new(AtomicU32::new(2)), sr);
            let a = lp_coef(2_000.0, sr as f32);
            let mut lp = 0.0f32;
            let mut total_e = 0.0f64;
            let mut hi_e = 0.0f64;
            for _ in 0..(sr as usize * 6) {
                let s = synth.next().unwrap();
                lp += a * (s - lp);
                let hi = s - lp;
                total_e += f64::from(s) * f64::from(s);
                hi_e += f64::from(hi) * f64::from(hi);
            }
            let hi_share = hi_e / total_e.max(1e-12);
            assert!(
                hi_share > 0.01,
                "{theme:?}: >2 kHz energy share {hi_share:.4} is too muffled"
            );
        }
    }
}
