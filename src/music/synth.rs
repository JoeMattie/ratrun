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
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rodio::source::Source;

use crate::audio::{fast_pulse_dc_free, fast_sin_unit, fast_tanh, fast_tri, SR};
use crate::game::level::Theme;

use super::{chip_requested, song_for, DrumKit, Patch, Role, Song, N_ROLES};

/// The voice pool. Summed [`Role::cap`] values fit inside it with slack.
pub const MAX_VOICES: usize = 24;

/// Which instrument palette [`SongSynth`] plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SynthMode {
    /// 16-bit console palette (FM, wavetable, Karplus-Strong, sampled-style kit, SNES echo).
    Bit16,
    /// Legacy chiptune oscillator palette (`RATRUN_MUSIC=chip`).
    Chip,
}

const WT_LEN: usize = 256;
const WT_MASK: usize = WT_LEN - 1;
const N_WT_TABLES: usize = 16;
const KS_MAX_LEN: usize = 1024;
const KS_MASK: usize = KS_MAX_LEN - 1;

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

#[derive(Clone, Copy, PartialEq, Eq)]
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
    /// 2-op / 4-op FM synthesis (Mega Drive / DX7 character).
    Fm16,
    /// Dual-wavetable crossfade + unison + delayed vibrato (SNES character).
    Wt16,
    /// Karplus-Strong fractional-delay plucked string / upright piano.
    Ks16,
    /// 16-bit filtered air/noise sweep.
    Fx16,
    /// 16-bit pitched-sweep + beater-click kick.
    Kick16,
    /// 16-bit dual-mode shell + band-passed wire snare.
    Snare16,
    /// 16-bit metallic partial + band-passed air hi-hat.
    Hat16,
    /// 16-bit sampled-style percussion (clap, rim, toms, timpani, cymbals, cowbell, woodblock, shaker).
    Perc16,
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
    vib_delay_left: u32,
    vib_ramp: f32,
    vib_ramp_inc: f32,
    env: f32,
    sustain: f32,
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
    // 16-bit FM operator 1 & 2 state
    c1_ratio: f32,
    m1_phase: f32,
    m1_ratio: f32,
    m1_idx: f32,
    m1_fb: f32,
    m1_prev: f32,
    m1_env: f32,
    m1_att_inc: f32,
    m1_dec: f32,
    m1_sus: f32,
    m1_in_att: bool,
    c2_mix: f32,
    c2_ratio: f32,
    m2_phase: f32,
    m2_ratio: f32,
    m2_idx: f32,
    m2_env: f32,
    m2_att_inc: f32,
    m2_dec: f32,
    m2_in_att: bool,
    c2_env: f32,
    c2_dec: f32,
    fm_drive: f32,
    fm_inv_tanh: f32,
    // 16-bit wavetable & chiff state
    wt_bright: u8,
    wt_warm: u8,
    wt_xfade: f32,
    wt_xfade_floor: f32,
    wt_xfade_dec: f32,
    wt_detune: f32,
    chiff_env: f32,
    chiff_dec: f32,
    chiff_mix: f32,
    chiff_hp_a: f32,
    chiff_lp_a: f32,
    chiff_hp1: f32,
    chiff_lp1: f32,
    // 16-bit Karplus-Strong state
    ks_pos: usize,
    ks_d_int: usize,
    ks_d_frac: f32,
    ks_init_left: usize,
    ks_rho: f32,
    ks_hammer_env: f32,
    ks_hammer_dec: f32,
    ks_hammer_mix: f32,
    // 16-bit drum extras
    drum_p3: f32,
    drum_dt3: f32,
    drum_wire: f32,
    drum_body: f32,
    drum_t: u32,
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
        vib_delay_left: 0,
        vib_ramp: 1.0,
        vib_ramp_inc: 0.0,
        env: 0.0,
        sustain: 0.0,
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
        c1_ratio: 1.0,
        m1_phase: 0.0,
        m1_ratio: 1.0,
        m1_idx: 0.0,
        m1_fb: 0.0,
        m1_prev: 0.0,
        m1_env: 0.0,
        m1_att_inc: 1.0,
        m1_dec: 1.0,
        m1_sus: 0.0,
        m1_in_att: false,
        c2_mix: 0.0,
        c2_ratio: 1.0,
        m2_phase: 0.0,
        m2_ratio: 1.0,
        m2_idx: 0.0,
        m2_env: 0.0,
        m2_att_inc: 1.0,
        m2_dec: 1.0,
        m2_in_att: false,
        c2_env: 0.0,
        c2_dec: 1.0,
        fm_drive: 0.0,
        fm_inv_tanh: 1.0,
        wt_bright: 0,
        wt_warm: 1,
        wt_xfade: 1.0,
        wt_xfade_floor: 0.5,
        wt_xfade_dec: 1.0,
        wt_detune: 0.0,
        chiff_env: 0.0,
        chiff_dec: 1.0,
        chiff_mix: 0.0,
        chiff_hp_a: 0.0,
        chiff_lp_a: 1.0,
        chiff_hp1: 0.0,
        chiff_lp1: 0.0,
        ks_pos: 0,
        ks_d_int: 16,
        ks_d_frac: 0.0,
        ks_init_left: 0,
        ks_rho: 0.99,
        ks_hammer_env: 0.0,
        ks_hammer_dec: 1.0,
        ks_hammer_mix: 0.0,
        drum_p3: 0.0,
        drum_dt3: 0.0,
        drum_wire: 0.0,
        drum_body: 0.0,
        drum_t: 0,
    };
}

/// Wrap any finite phase into `[0.0, 1.0)`.
#[inline]
fn wrap01(x: f32) -> f32 {
    let p = x - x.floor();
    if p >= 1.0 { 0.0 } else { p }
}

/// Precomputed single-cycle wavetables built once from additive harmonics.
struct WavetableBank {
    tables: [[f32; WT_LEN]; N_WT_TABLES],
}

static WT_BANK: OnceLock<WavetableBank> = OnceLock::new();

fn build_wt(harmonics: &[(u32, f64, f64)]) -> [f32; WT_LEN] {
    let mut tmp = [0.0f64; WT_LEN];
    let mut sum = 0.0f64;
    for (i, slot) in tmp.iter_mut().enumerate() {
        let t = i as f64 / WT_LEN as f64;
        let mut s = 0.0f64;
        for &(k, amp, ph) in harmonics {
            s += amp * (std::f64::consts::TAU * f64::from(k) * t + ph).sin();
        }
        *slot = s;
        sum += s;
    }
    let mean = sum / WT_LEN as f64;
    let mut pk = 0.0f64;
    for slot in tmp.iter_mut() {
        *slot -= mean;
        pk = pk.max(slot.abs());
    }
    let inv = if pk > 1e-6 { 1.0 / pk } else { 1.0 };
    let mut out = [0.0f32; WT_LEN];
    for (dst, &src) in out.iter_mut().zip(tmp.iter()) {
        *dst = (src * inv) as f32;
    }
    out
}

impl WavetableBank {
    fn build() -> WavetableBank {
        WavetableBank {
            tables: [
                // 0: strings_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.78, 0.3), (3, 0.62, 0.8), (4, 0.45, 1.4),
                    (5, 0.42, 0.2), (6, 0.30, 1.9), (7, 0.28, 0.6), (8, 0.20, 2.3),
                    (9, 0.17, 1.1), (10, 0.13, 2.7), (11, 0.11, 0.4), (12, 0.08, 1.6),
                    (13, 0.06, 2.1), (14, 0.05, 0.9),
                ]),
                // 1: strings_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.62, 0.3), (3, 0.38, 0.8), (4, 0.22, 1.4),
                    (5, 0.15, 0.2), (6, 0.09, 1.9), (7, 0.05, 0.6),
                ]),
                // 2: soft_strings_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.68, 0.2), (3, 0.48, 0.7), (4, 0.30, 1.1),
                    (5, 0.24, 0.4), (6, 0.16, 1.8), (7, 0.12, 0.9), (8, 0.08, 2.2),
                    (9, 0.05, 1.3),
                ]),
                // 3: soft_strings_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.48, 0.2), (3, 0.26, 0.7), (4, 0.12, 1.1),
                    (5, 0.07, 0.4),
                ]),
                // 4: choir_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.28, 0.5), (3, 0.58, 1.1), (4, 0.18, 2.0),
                    (5, 0.44, 0.4), (6, 0.36, 1.5), (7, 0.14, 2.4), (8, 0.24, 0.8),
                    (9, 0.16, 1.9), (10, 0.14, 0.3), (11, 0.08, 2.6),
                ]),
                // 5: choir_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.22, 0.5), (3, 0.40, 1.1), (4, 0.12, 2.0),
                    (5, 0.22, 0.4), (6, 0.14, 1.5),
                ]),
                // 6: flute_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.24, 0.2), (3, 0.62, 0.6), (4, 0.16, 1.3),
                    (5, 0.54, 0.4), (6, 0.14, 1.8), (7, 0.42, 0.9), (8, 0.12, 2.2),
                    (9, 0.30, 1.5), (11, 0.18, 0.3),
                ]),
                // 7: flute_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.16, 0.2), (3, 0.40, 0.6), (5, 0.24, 0.4),
                    (7, 0.12, 0.9),
                ]),
                // 8: whistle_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.36, 0.1), (3, 0.28, 0.4), (4, 0.16, 0.9),
                    (5, 0.10, 1.5), (6, 0.05, 0.3),
                ]),
                // 9: whistle_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.18, 0.1), (3, 0.10, 0.4), (4, 0.04, 0.9),
                ]),
                // 10: reed_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.36, 0.3), (3, 0.74, 0.7), (4, 0.26, 1.2),
                    (5, 0.56, 0.4), (6, 0.18, 1.9), (7, 0.38, 0.9), (8, 0.12, 2.3),
                    (9, 0.24, 0.2), (11, 0.14, 1.5),
                ]),
                // 11: reed_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.26, 0.3), (3, 0.52, 0.7), (4, 0.14, 1.2),
                    (5, 0.28, 0.4), (7, 0.14, 0.9),
                ]),
                // 12: thin_sine_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.26, 0.3), (3, 0.46, 0.8), (4, 0.16, 1.4),
                    (5, 0.40, 0.5), (7, 0.28, 1.7), (9, 0.18, 0.2), (11, 0.10, 1.1),
                ]),
                // 13: thin_sine_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.12, 0.3), (3, 0.18, 0.8), (5, 0.10, 0.5),
                ]),
                // 14: organ_bright
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.75, 0.1), (3, 0.68, 0.5), (4, 0.48, 0.9),
                    (5, 0.22, 1.4), (6, 0.36, 0.3), (8, 0.26, 1.8), (10, 0.14, 0.7),
                    (12, 0.10, 2.1),
                ]),
                // 15: organ_warm
                build_wt(&[
                    (1, 1.00, 0.0), (2, 0.62, 0.1), (3, 0.52, 0.5), (4, 0.30, 0.9),
                    (6, 0.18, 0.3), (8, 0.10, 1.8),
                ]),
            ],
        }
    }
}

#[inline]
fn wavetables() -> &'static WavetableBank {
    WT_BANK.get_or_init(WavetableBank::build)
}

#[inline]
fn wt_sample(table: &[f32; WT_LEN], phase: f32) -> f32 {
    let p = wrap01(phase);
    let idx_f = p * WT_LEN as f32;
    let i0 = (idx_f as usize) & WT_MASK;
    let i1 = (i0 + 1) & WT_MASK;
    let frac = idx_f - (idx_f as u32 as f32);
    table[i0] * (1.0 - frac) + table[i1] * frac
}

#[derive(Clone, Copy)]
struct FmPreset {
    att_ms: f32,
    dec_ms: f32,
    sus: f32,
    rel_ms: f32,
    c1_ratio: f32,
    m1_ratio: f32,
    index1: f32,
    m1_att_ms: f32,
    m1_dec_ms: f32,
    m1_sus: f32,
    m1_fb: f32,
    vel_to_idx: f32,
    c2_mix: f32,
    c2_ratio: f32,
    m2_ratio: f32,
    index2: f32,
    m2_dec_ms: f32,
    c2_dec_ms: f32,
    lp_hz: f32,
    hp_hz: f32,
    drive: f32,
    vib_hz: f32,
    vib_depth: f32,
    vib_delay_ms: f32,
}

fn fm_preset(patch: Patch) -> Option<FmPreset> {
    Some(match patch {
        Patch::FmDarkBass => FmPreset {
            att_ms: 4.0, dec_ms: 640.0, sus: 0.20, rel_ms: 55.0,
            c1_ratio: 1.0, m1_ratio: 1.0, index1: 0.24, m1_att_ms: 2.0, m1_dec_ms: 220.0, m1_sus: 0.14,
            m1_fb: 0.20, vel_to_idx: 0.55,
            c2_mix: 0.24, c2_ratio: 2.0, m2_ratio: 2.0, index2: 0.14, m2_dec_ms: 110.0, c2_dec_ms: 170.0,
            lp_hz: 1050.0, hp_hz: 0.0, drive: 0.0, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmSlapBass => FmPreset {
            att_ms: 2.5, dec_ms: 260.0, sus: 0.12, rel_ms: 45.0,
            c1_ratio: 1.0, m1_ratio: 2.0, index1: 0.68, m1_att_ms: 1.0, m1_dec_ms: 80.0, m1_sus: 0.08,
            m1_fb: 0.30, vel_to_idx: 0.70,
            c2_mix: 0.42, c2_ratio: 1.0, m2_ratio: 5.0, index2: 0.52, m2_dec_ms: 48.0, c2_dec_ms: 95.0,
            lp_hz: 5200.0, hp_hz: 0.0, drive: 0.0, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmUprightBass => FmPreset {
            att_ms: 5.0, dec_ms: 720.0, sus: 0.14, rel_ms: 65.0,
            c1_ratio: 1.0, m1_ratio: 1.0, index1: 0.22, m1_att_ms: 2.0, m1_dec_ms: 120.0, m1_sus: 0.06,
            m1_fb: 0.12, vel_to_idx: 0.50,
            c2_mix: 0.22, c2_ratio: 2.0, m2_ratio: 3.0, index2: 0.20, m2_dec_ms: 45.0, c2_dec_ms: 85.0,
            lp_hz: 950.0, hp_hz: 0.0, drive: 0.0, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmPickedBass => FmPreset {
            att_ms: 2.5, dec_ms: 290.0, sus: 0.24, rel_ms: 45.0,
            c1_ratio: 1.0, m1_ratio: 2.0, index1: 0.46, m1_att_ms: 1.0, m1_dec_ms: 95.0, m1_sus: 0.18,
            m1_fb: 0.28, vel_to_idx: 0.60,
            c2_mix: 0.32, c2_ratio: 1.0, m2_ratio: 4.0, index2: 0.36, m2_dec_ms: 45.0, c2_dec_ms: 115.0,
            lp_hz: 3600.0, hp_hz: 0.0, drive: 1.40, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmEpiano => FmPreset {
            att_ms: 3.0, dec_ms: 760.0, sus: 0.14, rel_ms: 85.0,
            c1_ratio: 1.0, m1_ratio: 1.0, index1: 0.24, m1_att_ms: 1.5, m1_dec_ms: 360.0, m1_sus: 0.08,
            m1_fb: 0.10, vel_to_idx: 0.65,
            c2_mix: 0.46, c2_ratio: 1.003, m2_ratio: 7.0, index2: 0.42, m2_dec_ms: 120.0, c2_dec_ms: 340.0,
            lp_hz: 8200.0, hp_hz: 340.0, drive: 0.0, vib_hz: 5.2, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmMarimba => FmPreset {
            att_ms: 2.2, dec_ms: 210.0, sus: 0.0, rel_ms: 55.0,
            c1_ratio: 1.0, m1_ratio: 4.0, index1: 0.62, m1_att_ms: 1.0, m1_dec_ms: 72.0, m1_sus: 0.04,
            m1_fb: 0.08, vel_to_idx: 0.60,
            c2_mix: 0.45, c2_ratio: 4.0, m2_ratio: 1.0, index2: 0.28, m2_dec_ms: 48.0, c2_dec_ms: 75.0,
            lp_hz: 8800.0, hp_hz: 300.0, drive: 0.0, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmClav => FmPreset {
            att_ms: 4.5, dec_ms: 240.0, sus: 0.08, rel_ms: 45.0,
            c1_ratio: 1.0, m1_ratio: 2.0, index1: 0.62, m1_att_ms: 3.5, m1_dec_ms: 145.0, m1_sus: 0.18,
            m1_fb: 0.34, vel_to_idx: 0.65,
            c2_mix: 0.40, c2_ratio: 2.0, m2_ratio: 3.0, index2: 0.34, m2_dec_ms: 90.0, c2_dec_ms: 135.0,
            lp_hz: 8600.0, hp_hz: 340.0, drive: 0.0, vib_hz: 5.4, vib_depth: 0.0015, vib_delay_ms: 120.0,
        },
        Patch::FmHardBell => FmPreset {
            att_ms: 3.5, dec_ms: 460.0, sus: 0.0, rel_ms: 105.0,
            c1_ratio: 1.0, m1_ratio: 3.5, index1: 0.66, m1_att_ms: 2.0, m1_dec_ms: 220.0, m1_sus: 0.10,
            m1_fb: 0.14, vel_to_idx: 0.55,
            c2_mix: 0.48, c2_ratio: 2.0, m2_ratio: 1.414, index2: 0.46, m2_dec_ms: 145.0, c2_dec_ms: 280.0,
            lp_hz: 8800.0, hp_hz: 360.0, drive: 0.0, vib_hz: 5.0, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmWarmBrass => FmPreset {
            att_ms: 28.0, dec_ms: 880.0, sus: 0.52, rel_ms: 95.0,
            c1_ratio: 1.0, m1_ratio: 1.0, index1: 0.54, m1_att_ms: 38.0, m1_dec_ms: 460.0, m1_sus: 0.48,
            m1_fb: 0.42, vel_to_idx: 0.55,
            c2_mix: 0.72, c2_ratio: 1.0045, m2_ratio: 2.0, index2: 0.42, m2_dec_ms: 380.0, c2_dec_ms: 820.0,
            lp_hz: 7200.0, hp_hz: 260.0, drive: 0.0, vib_hz: 5.2, vib_depth: 0.0025, vib_delay_ms: 220.0,
        },
        Patch::FmPunchBrass => FmPreset {
            att_ms: 8.0, dec_ms: 390.0, sus: 0.34, rel_ms: 65.0,
            c1_ratio: 1.0, m1_ratio: 1.0, index1: 0.68, m1_att_ms: 12.0, m1_dec_ms: 240.0, m1_sus: 0.38,
            m1_fb: 0.44, vel_to_idx: 0.65,
            c2_mix: 0.70, c2_ratio: 1.005, m2_ratio: 2.0, index2: 0.46, m2_dec_ms: 180.0, c2_dec_ms: 360.0,
            lp_hz: 8200.0, hp_hz: 280.0, drive: 0.0, vib_hz: 5.4, vib_depth: 0.0, vib_delay_ms: 0.0,
        },
        Patch::FmOverdriveGuitar => FmPreset {
            att_ms: 4.5, dec_ms: 880.0, sus: 0.48, rel_ms: 70.0,
            c1_ratio: 1.0, m1_ratio: 2.0, index1: 0.45, m1_att_ms: 2.0, m1_dec_ms: 260.0, m1_sus: 0.31,
            m1_fb: 0.42, vel_to_idx: 0.55,
            c2_mix: 0.42, c2_ratio: 1.0035, m2_ratio: 3.0, index2: 0.34, m2_dec_ms: 140.0, c2_dec_ms: 650.0,
            lp_hz: 5600.0, hp_hz: 260.0, drive: 2.35, vib_hz: 5.8, vib_depth: 0.0082, vib_delay_ms: 140.0,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct WtPreset {
    bright: u8,
    warm: u8,
    att_ms: f32,
    dec_ms: f32,
    sus: f32,
    rel_ms: f32,
    xfade_ms: f32,
    xfade_floor: f32,
    detune: f32,
    lp_hz: f32,
    hp_hz: f32,
    chiff: f32,
    vib_hz: f32,
    vib_depth: f32,
    vib_delay_ms: f32,
    vib_ramp_ms: f32,
}

fn wt_preset(patch: Patch) -> Option<WtPreset> {
    Some(match patch {
        Patch::WtStrings => WtPreset {
            bright: 0, warm: 1,
            att_ms: 62.0, dec_ms: 1350.0, sus: 0.65, rel_ms: 125.0,
            xfade_ms: 650.0, xfade_floor: 0.48, detune: 0.0042,
            lp_hz: 6800.0, hp_hz: 320.0, chiff: 0.0,
            vib_hz: 5.2, vib_depth: 0.0038, vib_delay_ms: 220.0, vib_ramp_ms: 200.0,
        },
        Patch::WtSoftStrings => WtPreset {
            bright: 2, warm: 3,
            att_ms: 24.0, dec_ms: 680.0, sus: 0.50, rel_ms: 95.0,
            xfade_ms: 380.0, xfade_floor: 0.42, detune: 0.0038,
            lp_hz: 6600.0, hp_hz: 340.0, chiff: 0.0,
            vib_hz: 5.0, vib_depth: 0.0032, vib_delay_ms: 180.0, vib_ramp_ms: 160.0,
        },
        Patch::WtColdChoir => WtPreset {
            bright: 4, warm: 5,
            att_ms: 72.0, dec_ms: 2400.0, sus: 0.70, rel_ms: 145.0,
            xfade_ms: 800.0, xfade_floor: 0.52, detune: 0.0040,
            lp_hz: 6800.0, hp_hz: 340.0, chiff: 0.0,
            vib_hz: 4.8, vib_depth: 0.0035, vib_delay_ms: 260.0, vib_ramp_ms: 240.0,
        },
        Patch::WtHollowFlute => WtPreset {
            bright: 6, warm: 7,
            att_ms: 12.0, dec_ms: 520.0, sus: 0.56, rel_ms: 75.0,
            xfade_ms: 360.0, xfade_floor: 0.58, detune: 0.0024,
            lp_hz: 8400.0, hp_hz: 260.0, chiff: 0.34,
            vib_hz: 5.4, vib_depth: 0.0062, vib_delay_ms: 130.0, vib_ramp_ms: 150.0,
        },
        Patch::WtWhistle => WtPreset {
            bright: 8, warm: 9,
            att_ms: 9.0, dec_ms: 420.0, sus: 0.60, rel_ms: 60.0,
            xfade_ms: 280.0, xfade_floor: 0.60, detune: 0.0020,
            lp_hz: 8800.0, hp_hz: 340.0, chiff: 0.32,
            vib_hz: 6.1, vib_depth: 0.0092, vib_delay_ms: 95.0, vib_ramp_ms: 120.0,
        },
        Patch::WtClarinetAccordion => WtPreset {
            bright: 10, warm: 11,
            att_ms: 11.0, dec_ms: 640.0, sus: 0.62, rel_ms: 70.0,
            xfade_ms: 340.0, xfade_floor: 0.62, detune: 0.0034,
            lp_hz: 8200.0, hp_hz: 280.0, chiff: 0.08,
            vib_hz: 5.5, vib_depth: 0.0048, vib_delay_ms: 150.0, vib_ramp_ms: 160.0,
        },
        Patch::WtThinSine => WtPreset {
            bright: 12, warm: 13,
            att_ms: 6.0, dec_ms: 340.0, sus: 0.42, rel_ms: 70.0,
            xfade_ms: 260.0, xfade_floor: 0.48, detune: 0.0018,
            lp_hz: 8800.0, hp_hz: 280.0, chiff: 0.08,
            vib_hz: 5.6, vib_depth: 0.0070, vib_delay_ms: 110.0, vib_ramp_ms: 140.0,
        },
        Patch::WtRockOrgan => WtPreset {
            bright: 14, warm: 15,
            att_ms: 8.0, dec_ms: 1100.0, sus: 0.68, rel_ms: 65.0,
            xfade_ms: 320.0, xfade_floor: 0.58, detune: 0.0036,
            lp_hz: 6200.0, hp_hz: 340.0, chiff: 0.10,
            vib_hz: 5.8, vib_depth: 0.0030, vib_delay_ms: 200.0, vib_ramp_ms: 160.0,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct KsPreset {
    excite_lp_hz: f32,
    tone_hz: f32,
    decay_s: f32,
    pick_pos: f32,
    body_mix: f32,
    hammer_mix: f32,
    hp_hz: f32,
    rel_ms: f32,
}

fn ks_preset(patch: Patch) -> Option<KsPreset> {
    Some(match patch {
        Patch::KsHarp => KsPreset {
            excite_lp_hz: 8400.0, tone_hz: 7600.0, decay_s: 0.78, pick_pos: 0.16,
            body_mix: 0.16, hammer_mix: 0.0, hp_hz: 300.0, rel_ms: 70.0,
        },
        Patch::KsPizzicato => KsPreset {
            excite_lp_hz: 7600.0, tone_hz: 6600.0, decay_s: 0.34, pick_pos: 0.22,
            body_mix: 0.20, hammer_mix: 0.12, hp_hz: 320.0, rel_ms: 55.0,
        },
        Patch::KsUprightPiano => KsPreset {
            excite_lp_hz: 7400.0, tone_hz: 6400.0, decay_s: 0.68, pick_pos: 0.13,
            body_mix: 0.22, hammer_mix: 0.20, hp_hz: 320.0, rel_ms: 65.0,
        },
        Patch::KsClavinet => KsPreset {
            excite_lp_hz: 8800.0, tone_hz: 8000.0, decay_s: 0.40, pick_pos: 0.11,
            body_mix: 0.14, hammer_mix: 0.08, hp_hz: 340.0, rel_ms: 45.0,
        },
        Patch::KsRockGuitar => KsPreset {
            excite_lp_hz: 8200.0, tone_hz: 7200.0, decay_s: 0.55, pick_pos: 0.16,
            body_mix: 0.16, hammer_mix: 0.06, hp_hz: 320.0, rel_ms: 55.0,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct KitParams {
    kick_hz: f32,
    kick_sweep: f32,
    kick_sweep_ms: f32,
    kick_decay_ms: f32,
    kick_click: f32,
    kick_sat: f32,
    snare_hz: f32,
    snare_decay_ms: f32,
    snare_hp: f32,
    snare_lp: f32,
    snare_wire: f32,
    snare_body: f32,
    snare_att_ms: f32,
    hat_hp: f32,
    hat_lp: f32,
    hat_closed_ms: f32,
    hat_open_ms: f32,
    hat_metal: f32,
    hat_base_hz: f32,
    tom_decay_scale: f32,
    clap_hp: f32,
    clap_lp: f32,
}

fn kit_params(kit: DrumKit) -> KitParams {
    match kit {
        DrumKit::Deep | DrumKit::Chip => KitParams {
            kick_hz: 46.0, kick_sweep: 3.4, kick_sweep_ms: 24.0, kick_decay_ms: 132.0, kick_click: 0.26, kick_sat: 1.32,
            snare_hz: 172.0, snare_decay_ms: 112.0, snare_hp: 1800.0, snare_lp: 6800.0, snare_wire: 0.98, snare_body: 0.42, snare_att_ms: 0.8,
            hat_hp: 3200.0, hat_lp: 7800.0, hat_closed_ms: 36.0, hat_open_ms: 165.0, hat_metal: 0.34, hat_base_hz: 3150.0,
            tom_decay_scale: 1.12, clap_hp: 1300.0, clap_lp: 5200.0,
        },
        DrumKit::TightBright => KitParams {
            kick_hz: 58.0, kick_sweep: 3.6, kick_sweep_ms: 18.0, kick_decay_ms: 100.0, kick_click: 0.34, kick_sat: 1.28,
            snare_hz: 206.0, snare_decay_ms: 92.0, snare_hp: 2000.0, snare_lp: 7800.0, snare_wire: 1.05, snare_body: 0.40, snare_att_ms: 0.7,
            hat_hp: 3400.0, hat_lp: 8400.0, hat_closed_ms: 32.0, hat_open_ms: 150.0, hat_metal: 0.38, hat_base_hz: 3450.0,
            tom_decay_scale: 1.02, clap_hp: 1500.0, clap_lp: 5600.0,
        },
        DrumKit::Brushed => KitParams {
            kick_hz: 49.0, kick_sweep: 3.1, kick_sweep_ms: 22.0, kick_decay_ms: 125.0, kick_click: 0.20, kick_sat: 1.20,
            snare_hz: 184.0, snare_decay_ms: 128.0, snare_hp: 1850.0, snare_lp: 7400.0, snare_wire: 0.98, snare_body: 0.32, snare_att_ms: 3.2,
            hat_hp: 3400.0, hat_lp: 8000.0, hat_closed_ms: 44.0, hat_open_ms: 175.0, hat_metal: 0.28, hat_base_hz: 3250.0,
            tom_decay_scale: 1.00, clap_hp: 1400.0, clap_lp: 5200.0,
        },
        DrumKit::Electronic => KitParams {
            kick_hz: 44.0, kick_sweep: 4.0, kick_sweep_ms: 20.0, kick_decay_ms: 125.0, kick_click: 0.36, kick_sat: 1.38,
            snare_hz: 215.0, snare_decay_ms: 88.0, snare_hp: 2150.0, snare_lp: 8400.0, snare_wire: 1.02, snare_body: 0.40, snare_att_ms: 0.8,
            hat_hp: 3600.0, hat_lp: 8600.0, hat_closed_ms: 30.0, hat_open_ms: 155.0, hat_metal: 0.44, hat_base_hz: 3550.0,
            tom_decay_scale: 1.00, clap_hp: 1600.0, clap_lp: 5800.0,
        },
        DrumKit::Mechanical => KitParams {
            kick_hz: 53.0, kick_sweep: 3.7, kick_sweep_ms: 17.0, kick_decay_ms: 102.0, kick_click: 0.34, kick_sat: 1.35,
            snare_hz: 198.0, snare_decay_ms: 96.0, snare_hp: 2050.0, snare_lp: 8200.0, snare_wire: 1.08, snare_body: 0.40, snare_att_ms: 0.8,
            hat_hp: 3500.0, hat_lp: 8400.0, hat_closed_ms: 32.0, hat_open_ms: 160.0, hat_metal: 0.40, hat_base_hz: 3450.0,
            tom_decay_scale: 0.95, clap_hp: 1500.0, clap_lp: 5600.0,
        },
        DrumKit::BigRock => KitParams {
            kick_hz: 54.0, kick_sweep: 3.6, kick_sweep_ms: 22.0, kick_decay_ms: 125.0, kick_click: 0.32, kick_sat: 1.40,
            snare_hz: 178.0, snare_decay_ms: 138.0, snare_hp: 1800.0, snare_lp: 7800.0, snare_wire: 1.04, snare_body: 0.46, snare_att_ms: 0.8,
            hat_hp: 3400.0, hat_lp: 8400.0, hat_closed_ms: 36.0, hat_open_ms: 190.0, hat_metal: 0.36, hat_base_hz: 3350.0,
            tom_decay_scale: 1.22, clap_hp: 1350.0, clap_lp: 5400.0,
        },
    }
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
    mode: SynthMode,
    use_16bit: bool,
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
    /// Remaining soft-intro steps where only bass + pad trigger (menu entry).
    soft_intro_left: u32,
    /// Optional master fade-in ramp (samples) for gentle menu entry/switching.
    fade_in_pos: u32,
    fade_in_len: u32,

    voices: [Voice; MAX_VOICES],
    /// Fixed Karplus-Strong delay lines allocated once at construction (`MAX_VOICES * KS_MAX_LEN`).
    ks_bufs: Box<[f32]>,
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

    /// Global SNES-style echo ring buffer allocated once at construction.
    echo_buf: Box<[f32]>,
    echo_len: usize,
    echo_tap2: usize,
    echo_pos: usize,
    echo_fb: f32,
    echo_lp_a: f32,
    echo_hp_a: f32,
    echo_lp_y: f32,
    echo_hp_y: f32,
    echo_send: [f32; N_ROLES],
    /// Soft Gaussian-style 2-pole master low-pass (16-bit palette).
    master_lp_a: f32,
    master_lp1: f32,
    master_lp2: f32,

    lim_env: f32,
    lim_attack: f32,
    lim_release: f32,
    lim_gain: f32,
    dc_x1: f32,
    dc_y1: f32,
    dc_r: f32,
}

impl SongSynth {
    /// A track for `theme` at the standard 44.1 kHz (16-bit by default;
    /// `RATRUN_MUSIC=chip` selects the legacy chip oscillator palette).
    pub fn new(theme: Theme, intensity: Arc<AtomicU32>) -> SongSynth {
        SongSynth::with_sample_rate(theme, intensity, SR)
    }

    /// A menu track for `theme` at 44.1 kHz: enters softly with a 300 ms
    /// fade-in and two bars of bass + pad before the calm percussion joins.
    pub fn new_menu(theme: Theme, intensity: Arc<AtomicU32>) -> SongSynth {
        let mode = if chip_requested() {
            SynthMode::Chip
        } else {
            SynthMode::Bit16
        };
        SongSynth::with_sample_rate_and_mode_menu(theme, intensity, SR, mode, true)
    }

    /// A track for `theme` at an arbitrary sample rate. Every coefficient is
    /// derived from `sample_rate`, so the music sounds the same anywhere.
    pub fn with_sample_rate(
        theme: Theme,
        intensity: Arc<AtomicU32>,
        sample_rate: u32,
    ) -> SongSynth {
        let mode = if chip_requested() {
            SynthMode::Chip
        } else {
            SynthMode::Bit16
        };
        SongSynth::with_sample_rate_and_mode(theme, intensity, sample_rate, mode)
    }

    /// A track for `theme` at an arbitrary sample rate with an explicit palette
    /// mode (useful for deterministic tests and A/B comparisons).
    pub fn with_sample_rate_and_mode(
        theme: Theme,
        intensity: Arc<AtomicU32>,
        sample_rate: u32,
        mode: SynthMode,
    ) -> SongSynth {
        SongSynth::with_sample_rate_and_mode_menu(theme, intensity, sample_rate, mode, false)
    }

    /// Internal constructor supporting soft menu entry (`menu_soft_entry`).
    pub fn with_sample_rate_and_mode_menu(
        theme: Theme,
        intensity: Arc<AtomicU32>,
        sample_rate: u32,
        mode: SynthMode,
        menu_soft_entry: bool,
    ) -> SongSynth {
        let song = song_for(theme);
        let style = song.style;
        let rate = sample_rate.max(8_000);
        let inv_rate = 1.0 / rate as f32;
        let samples_per_step = ((rate as f64 * 60.0 / (song.bpm as f64 * 4.0)).round() as u32).max(1);
        let tier = intensity.load(Ordering::Relaxed).min(3);

        let has_16bit_palette = style.drum_kit != DrumKit::Chip
            || style.patches.iter().any(|&p| p != Patch::Chip);
        let use_16bit = mode == SynthMode::Bit16 && has_16bit_palette;
        if use_16bit {
            // Ensure wavetables are initialized outside the audio callback.
            let _ = wavetables();
        }

        let ks_bufs = if use_16bit {
            vec![0.0f32; MAX_VOICES * KS_MAX_LEN].into_boxed_slice()
        } else {
            Vec::new().into_boxed_slice()
        };

        let (echo_buf, echo_len, echo_tap2, echo_fb, echo_lp_a, echo_hp_a, echo_send) =
            if use_16bit && style.echo_ms > 0.0 && style.echo_feedback > 0.0 {
                let d = ((style.echo_ms * 1e-3 * rate as f32).round() as usize).max(1);
                let d2 = ((d * 62) / 100).clamp(1, d);
                (
                    vec![0.0f32; d].into_boxed_slice(),
                    d,
                    d2,
                    style.echo_feedback.clamp(0.0, 0.48),
                    lp_coef(style.echo_lp_hz, rate as f32),
                    lp_coef(220.0, rate as f32),
                    style.echo_send,
                )
            } else {
                (
                    Vec::new().into_boxed_slice(),
                    0,
                    0,
                    0.0,
                    1.0,
                    0.0,
                    [0.0; N_ROLES],
                )
            };

        let master_lp_a = if use_16bit && style.master_lp_hz > 0.0 {
            lp_coef(style.master_lp_hz, rate as f32)
        } else {
            0.0
        };

        let (soft_intro_left, fade_in_len) = if menu_soft_entry {
            (
                song.steps_per_bar().saturating_mul(2),
                (0.30 * rate as f32).round() as u32,
            )
        } else {
            (0, 0)
        };

        let mut synth = SongSynth {
            song,
            intensity,
            mode,
            use_16bit,
            sample_rate: rate,
            inv_rate,
            samples_per_step,
            step_pos: 0,
            step: 0,
            loop_steps: song.loop_steps(),
            cursor: [0; N_ROLES],
            wait: [-1; N_ROLES],
            soft_intro_left,
            fade_in_pos: 0,
            fade_in_len,
            voices: [Voice::SILENT; MAX_VOICES],
            ks_bufs,
            tier,
            control_left: 0,
            role_gain: [0.0; N_ROLES],
            role_target: [0.0; N_ROLES],
            gain_slew: 1.0 - (-1.0 / (GAIN_SLEW_SECS * rate as f32)).exp(),
            last_bass: 0,
            loops: 0,
            seed: 0x9E37_79B9,
            echo_buf,
            echo_len,
            echo_tap2,
            echo_pos: 0,
            echo_fb,
            echo_lp_a,
            echo_hp_a,
            echo_lp_y: 0.0,
            echo_hp_y: 0.0,
            echo_send,
            master_lp_a,
            master_lp1: 0.0,
            master_lp2: 0.0,
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

    /// Which palette mode this track is using.
    pub fn mode(&self) -> SynthMode {
        self.mode
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
        let use_mix_16 = self.use_16bit && self.song.style.mix_16.iter().any(|&g| g > 0.0);
        let mix = if use_mix_16 {
            &self.song.style.mix_16
        } else {
            &self.song.style.mix
        };
        for role in Role::ALL {
            let i = role.index();
            self.role_target[i] = mix[i] * LAYER_GAIN[i][t] * TIER_GAIN[t];
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
        let vel_norm = vel.min(127) as f32 / 127.0;
        let attack = |ms: f32| 1.0 / (ms.max(0.05) * 1e-3 * sr);

        let mut handled_16bit = false;
        if self.use_16bit {
            if role.is_drum() && style.drum_kit != DrumKit::Chip {
                init_drum16_voice(&mut v, role, midi, sr, kit_params(style.drum_kit));
                handled_16bit = true;
            } else {
                let patch = style.patches[role.index()];
                if let Some(fp) = fm_preset(patch) {
                    init_fm16_voice(&mut v, hz, vel_norm, sr, fp);
                    if role == Role::Bass {
                        self.last_bass = midi;
                    }
                    handled_16bit = true;
                } else if let Some(wp) = wt_preset(patch) {
                    init_wt16_voice(&mut v, hz, vel_norm, sr, wp);
                    if role == Role::Bass {
                        self.last_bass = midi;
                    }
                    handled_16bit = true;
                } else if let Some(kp) = ks_preset(patch) {
                    let start = slot * KS_MAX_LEN;
                    let buf = &mut self.ks_bufs[start..start + KS_MAX_LEN];
                    init_ks16_voice(&mut v, buf, hz, vel_norm, sr, kp);
                    if role == Role::Bass {
                        self.last_bass = midi;
                    }
                    handled_16bit = true;
                } else if patch == Patch::Fx16 {
                    init_fx16_voice(&mut v, hz, vel, sr);
                    handled_16bit = true;
                }
            }
        }

        if !handled_16bit {
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
        }
        // A stolen voice re-attacks from its current level rather than jumping
        // to zero, so stealing is never a click (KS and 16-bit drums use their
        // own smooth onset ramps over fresh excitation buffers).
        if self.voices[slot].on
            && !matches!(
                v.src,
                Src::Ks16 | Src::Kick16 | Src::Snare16 | Src::Hat16 | Src::Perc16
            )
        {
            v.env = self.voices[slot].env * 0.5;
            if v.src == Src::Fm16 {
                v.c2_env = v.env;
            }
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
                    // During a soft menu intro, only bass and pad trigger.
                    let soft_ok = self.soft_intro_left == 0
                        || matches!(role, Role::Bass | Role::Pad);
                    if self.role_target[i] > 1e-4 && soft_ok {
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
        if self.soft_intro_left > 0 {
            self.soft_intro_left -= 1;
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
        let mut dry_mix = 0.0f32;
        let mut echo_in = 0.0f32;
        let do_echo = self.echo_len > 0;
        let echo_send = self.echo_send;

        for (slot, v) in self.voices.iter_mut().enumerate() {
            if !v.on {
                continue;
            }
            let s = if v.src == Src::Ks16 {
                let start = slot * KS_MAX_LEN;
                voice_sample(v, sr, &mut self.ks_bufs[start..start + KS_MAX_LEN])
            } else {
                voice_sample(v, sr, &mut [])
            };
            if s != 0.0 {
                let r = v.role as usize;
                let contrib = s * v.amp * gains[r];
                dry_mix += contrib;
                if do_echo {
                    echo_in += contrib * echo_send[r];
                }
            }
        }

        let mut mix = dry_mix;
        if do_echo {
            let pos = self.echo_pos;
            let len = self.echo_len;
            let tap1 = self.echo_buf[pos];
            let idx2 = (pos + len - self.echo_tap2) % len;
            let tap2 = self.echo_buf[idx2];
            let wet = tap1 * 0.72 + tap2 * 0.28;

            // Warm 1-pole LP + 220 Hz HP inside the feedback loop so repeats
            // stay warm and bass mud never accumulates across loops.
            self.echo_lp_y += self.echo_lp_a * (tap1 - self.echo_lp_y);
            self.echo_hp_y += self.echo_hp_a * (self.echo_lp_y - self.echo_hp_y);
            let fb_sig = self.echo_lp_y - self.echo_hp_y;
            self.echo_buf[pos] = (echo_in + fb_sig * self.echo_fb).clamp(-2.0, 2.0);
            self.echo_pos = if pos + 1 >= len { 0 } else { pos + 1 };
            mix += wet;
        }

        // Soft Gaussian-style 2-pole master low-pass (16-bit palette).
        if self.master_lp_a > 0.0 {
            self.master_lp1 += self.master_lp_a * (mix - self.master_lp1);
            self.master_lp2 += self.master_lp_a * (self.master_lp1 - self.master_lp2);
            mix = self.master_lp2;
        }

        let drive = if self.use_16bit && self.song.style.drive_16 > 0.0 {
            self.song.style.drive_16
        } else {
            self.song.style.drive
        };
        let mut out = fast_tanh(mix * drive) * BUS_TRIM;
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
        let mut final_out = dc * self.song.style.master;
        if self.fade_in_pos < self.fade_in_len {
            let fade = self.fade_in_pos as f32 / self.fade_in_len as f32;
            self.fade_in_pos += 1;
            final_out *= fade;
        }
        final_out
    }
}

/// Initialise a 2-op / 4-op FM voice (`Src::Fm16`).
fn init_fm16_voice(v: &mut Voice, hz: f32, vel_norm: f32, sr: f32, p: FmPreset) {
    let inv_rate = 1.0 / sr;
    let attack = |ms: f32| 1.0 / (ms.max(0.05) * 1e-3 * sr);
    v.src = Src::Fm16;
    v.dt = hz * inv_rate;
    v.inv_dt = 1.0 / v.dt.max(1e-6);
    v.attack_inc = attack(p.att_ms);
    v.decay = decay_mul(p.dec_ms, sr);
    v.sustain = p.sus;
    v.release = decay_mul(p.rel_ms, sr);

    let vel_idx = (1.0 - p.vel_to_idx) + p.vel_to_idx * vel_norm;
    let nyq1 = ((sr * 0.36 - hz * p.m1_ratio) / (sr * 0.30)).clamp(0.18, 1.0);
    let nyq2 = ((sr * 0.36 - hz * p.m2_ratio) / (sr * 0.30)).clamp(0.18, 1.0);

    v.c1_ratio = p.c1_ratio;
    v.m1_ratio = p.m1_ratio;
    v.m1_idx = p.index1 * vel_idx * nyq1;
    v.m1_fb = p.m1_fb * nyq1;
    v.m1_prev = 0.0;
    v.m1_env = 0.0;
    v.m1_att_inc = attack(p.m1_att_ms);
    v.m1_dec = decay_mul(p.m1_dec_ms, sr);
    v.m1_sus = p.m1_sus;
    v.m1_in_att = true;

    v.c2_mix = p.c2_mix;
    v.c2_ratio = p.c2_ratio;
    v.m2_ratio = p.m2_ratio;
    v.m2_idx = p.index2 * vel_idx * nyq2;
    v.m2_env = 0.0;
    v.m2_att_inc = attack(1.5);
    v.m2_dec = decay_mul(p.m2_dec_ms, sr);
    v.m2_in_att = true;
    v.c2_env = 0.0;
    v.c2_dec = decay_mul(p.c2_dec_ms, sr);

    v.fm_drive = p.drive;
    v.fm_inv_tanh = if p.drive > 0.0 {
        1.0 / fast_tanh(p.drive).max(1e-4)
    } else {
        1.0
    };

    v.lp_a = if p.lp_hz > 0.0 { lp_coef(p.lp_hz, sr) } else { 1.0 };
    v.lp_floor = v.lp_a;
    v.one_pole = true;
    v.hp_a = if p.hp_hz > 0.0 { lp_coef(p.hp_hz, sr) } else { 0.0 };

    v.vib_dt = p.vib_hz * inv_rate;
    v.vib_depth = p.vib_depth;
    v.vib_delay_left = (p.vib_delay_ms * 1e-3 * sr).round() as u32;
    v.vib_ramp = if v.vib_delay_left == 0 { 1.0 } else { 0.0 };
    v.vib_ramp_inc = 1.0 / (0.160 * sr).max(1.0);
}

/// Initialise a dual-wavetable crossfade + unison voice (`Src::Wt16`).
fn init_wt16_voice(v: &mut Voice, hz: f32, vel_norm: f32, sr: f32, p: WtPreset) {
    let inv_rate = 1.0 / sr;
    let attack = |ms: f32| 1.0 / (ms.max(0.05) * 1e-3 * sr);
    v.src = Src::Wt16;
    v.dt = hz * inv_rate;
    v.inv_dt = 1.0 / v.dt.max(1e-6);
    v.phase2 = 0.37;
    v.attack_inc = attack(p.att_ms);
    v.decay = decay_mul(p.dec_ms, sr);
    v.sustain = p.sus;
    v.release = decay_mul(p.rel_ms, sr);

    v.wt_bright = p.bright;
    v.wt_warm = p.warm;
    v.wt_xfade = (0.55 + 0.45 * vel_norm).min(1.0);
    v.wt_xfade_floor = p.xfade_floor;
    v.wt_xfade_dec = decay_mul(p.xfade_ms, sr);
    v.wt_detune = p.detune * 0.5;

    v.chiff_mix = p.chiff;
    if p.chiff > 0.0 {
        v.chiff_env = 1.0;
        v.chiff_dec = decay_mul(22.0, sr);
        v.chiff_hp_a = lp_coef((hz * 2.0).clamp(1200.0, 3200.0), sr);
        v.chiff_lp_a = lp_coef((hz * 4.0).clamp(3000.0, 7000.0), sr);
    }

    v.lp_a = if p.lp_hz > 0.0 { lp_coef(p.lp_hz, sr) } else { 1.0 };
    v.lp_floor = v.lp_a;
    v.one_pole = true;
    v.hp_a = if p.hp_hz > 0.0 { lp_coef(p.hp_hz, sr) } else { 0.0 };

    v.vib_dt = p.vib_hz * inv_rate;
    v.vib_depth = p.vib_depth;
    v.vib_delay_left = (p.vib_delay_ms * 1e-3 * sr).round() as u32;
    v.vib_ramp = if v.vib_delay_left == 0 { 1.0 } else { 0.0 };
    v.vib_ramp_inc = 1.0 / (p.vib_ramp_ms.max(1.0) * 1e-3 * sr).max(1.0);
}

/// Initialise a Karplus-Strong plucked-string voice (`Src::Ks16`).
fn init_ks16_voice(
    v: &mut Voice,
    buf: &mut [f32],
    hz: f32,
    vel_norm: f32,
    sr: f32,
    p: KsPreset,
) {
    let inv_rate = 1.0 / sr;
    v.src = Src::Ks16;
    v.dt = hz * inv_rate;
    v.inv_dt = 1.0 / v.dt.max(1e-6);

    let max_hold = (((p.decay_s * 1.4).max(0.25) * sr).round() as u32).max(1);
    v.gate_left = v.gate_left.clamp(1, max_hold);
    v.attack_inc = 1.0 / (0.0024 * sr).max(1.0);
    v.decay = 1.0;
    v.sustain = 0.0;
    v.release = decay_mul(p.rel_ms, sr);

    let tone_hz = (hz * 3.8).max(p.tone_hz.min(sr * 0.44)).min(sr * 0.44);
    let tone_a = lp_coef(tone_hz, sr);
    let gd = (1.0 - tone_a) / tone_a;
    let delay = (sr / hz - gd).clamp(4.0, (KS_MAX_LEN - 4) as f32);
    let d_int = delay.floor() as usize;
    let d_frac = delay - d_int as f32;
    let n_init = (d_int + 2).min(KS_MAX_LEN - 2);

    // Fill excitation burst directly in the preallocated ring buffer:
    // xorshift noise -> pre-LP -> pick-position comb -> sine body seed -> DC remove & normalize.
    let ex_a = lp_coef(p.excite_lp_hz * (0.65 + 0.45 * vel_norm), sr);
    let mut lp_ex = 0.0f32;
    for slot in buf[..n_init].iter_mut() {
        let n = noise(v);
        lp_ex += ex_a * (n - lp_ex);
        *slot = lp_ex;
    }
    let pick_off = ((delay * p.pick_pos).round() as usize).max(1);
    if pick_off < n_init {
        for i in (pick_off..n_init).rev() {
            buf[i] -= 0.72 * buf[i - pick_off];
        }
    }
    let mut sum = 0.0f32;
    for (i, slot) in buf[..n_init].iter_mut().enumerate() {
        let body = fast_sin_unit(wrap01(i as f32 / delay));
        *slot = *slot * (1.0 - p.body_mix) + body * p.body_mix;
        sum += *slot;
    }
    let mean = sum / n_init.max(1) as f32;
    let mut pk = 0.0f32;
    for slot in buf[..n_init].iter_mut() {
        *slot -= mean;
        pk = pk.max(slot.abs());
    }
    if pk > 1e-6 {
        let inv = 1.0 / pk;
        for slot in buf[..n_init].iter_mut() {
            *slot *= inv;
        }
    }

    v.lp_a = tone_a;
    v.lp_floor = tone_a;
    v.ks_pos = 0;
    v.ks_d_int = d_int;
    v.ks_d_frac = d_frac;
    v.ks_init_left = n_init;
    v.ks_rho = 0.001f32
        .powf(1.0 / (p.decay_s * hz).max(1.0))
        .clamp(0.84, 0.9992);

    v.ks_hammer_mix = p.hammer_mix;
    if p.hammer_mix > 0.0 {
        v.ks_hammer_env = 1.0;
        v.ks_hammer_dec = decay_mul(18.0, sr);
    }
    v.hp_a = if p.hp_hz > 0.0 { lp_coef(p.hp_hz, sr) } else { 0.0 };
}

/// Initialise a 16-bit filtered air/noise FX sweep (`Src::Fx16`).
fn init_fx16_voice(v: &mut Voice, hz: f32, vel: u8, sr: f32) {
    let inv_rate = 1.0 / sr;
    v.src = Src::Fx16;
    v.dt = hz * inv_rate;
    v.dt2 = (hz * 1.5 * inv_rate).min(0.45);
    v.attack_inc = 1.0 / (8.0 * 1e-3 * sr).max(1.0);
    v.release = decay_mul(65.0, sr);
    let fx_ms = (v.gate_left as f32 * inv_rate * 1200.0).max(260.0);
    v.decay = decay_mul(fx_ms, sr);
    if vel >= 80 {
        v.lp_a = lp_coef(380.0, sr);
        v.lp_floor = lp_coef(5600.0, sr);
    } else {
        v.lp_a = lp_coef(5200.0, sr);
        v.lp_floor = lp_coef(360.0, sr);
    }
    let sweep_s = (v.gate_left as f32 * 0.55 * inv_rate).max(0.08);
    v.lp_decay = 1.0 - (-inv_rate / sweep_s).exp();
    v.one_pole = false;
}

/// Initialise a 16-bit sampled-style drum voice (`Kick16`, `Snare16`, `Hat16`, `Perc16`).
fn init_drum16_voice(v: &mut Voice, role: Role, midi: u8, sr: f32, k: KitParams) {
    let inv_rate = 1.0 / sr;
    let attack = |ms: f32| 1.0 / (ms.max(0.05) * 1e-3 * sr);
    match role {
        Role::Kick => {
            v.src = Src::Kick16;
            let dec_ms = k.kick_decay_ms;
            v.gate_left = ((dec_ms * 1.15 * 1e-3 * sr).round() as u32).max(1);
            v.attack_inc = attack(0.8);
            v.decay = decay_mul(dec_ms, sr);
            v.release = decay_mul(25.0, sr);
            v.hz1 = k.kick_hz;
            v.hz0 = k.kick_hz * k.kick_sweep;
            v.m1_env = 1.0;
            v.m1_dec = decay_mul(k.kick_sweep_ms, sr);
            v.chiff_env = 1.0;
            v.chiff_dec = decay_mul(7.0, sr);
            v.chiff_hp_a = lp_coef(1400.0, sr);
            v.chiff_lp_a = lp_coef(5200.0, sr);
            v.chiff_mix = k.kick_click;
            v.fm_drive = k.kick_sat;
            v.fm_inv_tanh = 1.0 / fast_tanh(k.kick_sat).max(1e-4);
        }
        Role::Snare => {
            v.src = Src::Snare16;
            let dec_ms = k.snare_decay_ms;
            v.gate_left = ((dec_ms * 1.15 * 1e-3 * sr).round() as u32).max(1);
            v.attack_inc = attack(k.snare_att_ms);
            v.decay = decay_mul(dec_ms, sr);
            v.release = decay_mul(25.0, sr);
            v.hz0 = k.snare_hz;
            v.c2_env = 1.0;
            v.c2_dec = decay_mul((dec_ms * 0.65).min(72.0), sr);
            v.m1_env = 1.0;
            v.m1_dec = decay_mul(22.0, sr);
            v.m2_env = 1.0;
            v.m2_dec = decay_mul(16.0, sr);
            v.lp_a = lp_coef(k.snare_hp, sr);
            v.drum_lp_a = lp_coef(k.snare_lp, sr);
            v.drum_wire = k.snare_wire;
            v.drum_body = k.snare_body;
        }
        Role::Hat => {
            v.src = Src::Hat16;
            let dec_ms = if midi == 46 { k.hat_open_ms } else { k.hat_closed_ms };
            v.gate_left = ((dec_ms * 1.15 * 1e-3 * sr).round() as u32).max(1);
            v.attack_inc = attack(0.8);
            v.decay = decay_mul(dec_ms, sr);
            v.release = decay_mul(20.0, sr);
            v.lp_a = lp_coef(k.hat_hp, sr);
            v.drum_lp_a = lp_coef(k.hat_lp, sr);
            v.drum_wire = k.hat_metal;
            let f0 = k.hat_base_hz.min(sr * 0.24);
            let f1 = (f0 * 1.447).min(sr * 0.36);
            let f2 = (f0 * 1.791).min(sr * 0.44);
            v.dt = f0 * inv_rate;
            v.dt2 = f1 * inv_rate;
            v.drum_dt3 = f2 * inv_rate;
        }
        Role::Perc => {
            v.src = Src::Perc16;
            match midi {
                39 => {
                    // 16-bit 3-burst handclap
                    v.blend = 5.0;
                    v.gate_left = ((0.155 * sr).round() as u32).max(1);
                    v.attack_inc = attack(0.8);
                    v.decay = 1.0;
                    v.release = decay_mul(15.0, sr);
                    v.lp_a = lp_coef(k.clap_hp, sr);
                    v.drum_lp_a = lp_coef(k.clap_lp, sr);
                    v.drum_t = 0;
                }
                37 => {
                    // Rimshot
                    v.blend = 6.0;
                    v.gate_left = ((0.032 * sr).round() as u32).max(1);
                    v.attack_inc = attack(0.5);
                    v.decay = decay_mul(24.0, sr);
                    v.release = decay_mul(12.0, sr);
                    v.dt = 840.0 * inv_rate;
                    v.dt2 = 1720.0 * inv_rate;
                    v.m1_env = 1.0;
                    v.m1_dec = decay_mul(8.0, sr);
                    v.lp_a = lp_coef(2400.0, sr);
                    v.drum_lp_a = lp_coef(6800.0, sr);
                }
                75..=77 => {
                    // Woodblock / clave
                    let hz = match midi {
                        75 => 980.0,
                        76 => 1220.0,
                        _ => 780.0,
                    };
                    v.blend = 4.0;
                    v.gate_left = ((0.042 * sr).round() as u32).max(1);
                    v.attack_inc = attack(0.6);
                    v.decay = decay_mul(32.0, sr);
                    v.release = decay_mul(15.0, sr);
                    v.dt = hz * inv_rate;
                    v.dt2 = (hz * 2.38 * inv_rate).min(0.45);
                }
                56 => {
                    // Cowbell
                    v.blend = 3.0;
                    v.gate_left = ((0.150 * sr).round() as u32).max(1);
                    v.attack_inc = attack(0.6);
                    v.decay = decay_mul(130.0, sr);
                    v.release = decay_mul(25.0, sr);
                    v.dt = 540.0 * inv_rate;
                    v.dt2 = 805.0 * inv_rate;
                    v.lp_a = lp_coef(4200.0, sr);
                }
                41 | 43 | 45 | 47 | 48 | 65 | 66 => {
                    // Toms & timpani
                    let (hz0, base_ms) = match midi {
                        41 => (190.0, 130.0),
                        43 => (160.0, 145.0),
                        45 => (132.0, 160.0),
                        47 => (110.0, 175.0),
                        48 => (96.0, 190.0),
                        65 => (112.0, 270.0),
                        _ => (88.0, 290.0),
                    };
                    let dec_ms = base_ms * k.tom_decay_scale;
                    v.blend = 0.0;
                    v.gate_left = ((dec_ms * 1.15 * 1e-3 * sr).round() as u32).max(1);
                    v.attack_inc = attack(1.0);
                    v.decay = decay_mul(dec_ms, sr);
                    v.release = decay_mul(30.0, sr);
                    v.hz0 = hz0;
                    v.hz1 = hz0 * 0.58;
                    v.m1_env = 1.0;
                    v.m1_dec = decay_mul(48.0, sr);
                }
                49 | 51 | 52 | 53 | 55 | 57 | 59 => {
                    // Crash & ride cymbals
                    let (f_base, dec_ms): (f32, f32) = match midi {
                        49 => (3200.0, 460.0),
                        57 => (3400.0, 440.0),
                        51 => (4600.0, 320.0),
                        59 => (4800.0, 310.0),
                        52 => (4000.0, 280.0),
                        55 => (4200.0, 270.0),
                        _ => (3000.0, 240.0),
                    };
                    v.blend = 2.0;
                    v.gate_left = ((dec_ms * 1.15 * 1e-3 * sr).round() as u32).max(1);
                    v.attack_inc = attack(3.5);
                    v.decay = decay_mul(dec_ms, sr);
                    v.release = decay_mul(40.0, sr);
                    v.lp_a = lp_coef(2800.0, sr);
                    v.drum_lp_a = lp_coef(8800.0, sr);
                    let f0 = (f_base * 0.72).min(sr * 0.24);
                    let f1 = (f0 * 1.447).min(sr * 0.35);
                    let f2 = (f0 * 1.813).min(sr * 0.43);
                    v.dt = f0 * inv_rate;
                    v.dt2 = f1 * inv_rate;
                    v.drum_dt3 = f2 * inv_rate;
                }
                _ => {
                    // Shaker / tambourine / default short perc
                    let is_shaker = matches!(midi, 54 | 69 | 70);
                    let dec_ms = if is_shaker { 44.0 } else { 60.0 };
                    let att_ms = if is_shaker { 4.0 } else { 1.0 };
                    let hp_hz = if is_shaker { 4600.0 } else { 2400.0 };
                    v.blend = 1.0;
                    v.gate_left = ((dec_ms * 1.2 * 1e-3 * sr).round() as u32).max(1);
                    v.attack_inc = attack(att_ms);
                    v.decay = decay_mul(dec_ms, sr);
                    v.release = decay_mul(18.0, sr);
                    v.lp_a = lp_coef(hp_hz, sr);
                    v.drum_lp_a = lp_coef(9200.0, sr);
                }
            }
        }
        _ => {}
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
fn voice_sample(v: &mut Voice, sr: f32, ks_buf: &mut [f32]) -> f32 {
    // Envelope (ADSR: sustains toward `v.sustain`, which is 0.0 for legacy voices).
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
            if v.sustain > 0.0 {
                v.env = v.sustain + (v.env - v.sustain) * v.decay;
            } else {
                v.env *= v.decay;
            }
            if v.gate_left == 0 {
                v.stage = Stage::Release;
            } else if v.sustain <= 1e-4 && v.env < 1e-4 {
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
        Src::Fm16 => {
            let vib_fm = if v.vib_depth > 0.0 {
                v.vib = wrap01(v.vib + v.vib_dt);
                let amt = if v.vib_delay_left > 0 {
                    v.vib_delay_left -= 1;
                    0.0
                } else {
                    v.vib_ramp = (v.vib_ramp + v.vib_ramp_inc).min(1.0);
                    v.vib_ramp
                };
                1.0 + v.vib_depth * amt * fast_sin_unit(v.vib)
            } else {
                1.0
            };
            if v.m1_in_att {
                v.m1_env += v.m1_att_inc;
                if v.m1_env >= 1.0 {
                    v.m1_env = 1.0;
                    v.m1_in_att = false;
                }
            } else {
                v.m1_env = v.m1_sus + (v.m1_env - v.m1_sus) * v.m1_dec;
            }
            let dt_v = v.dt * vib_fm;
            let m1_dt = (dt_v * v.m1_ratio).min(0.45);
            v.m1_phase = wrap01(v.m1_phase + m1_dt);
            let m1 = fast_sin_unit(wrap01(v.m1_phase + v.m1_fb * v.m1_prev));
            v.m1_prev = m1;
            v.phase = wrap01(v.phase + dt_v * v.c1_ratio);
            let c1 = fast_sin_unit(wrap01(v.phase + m1 * (v.m1_idx * v.m1_env)));
            let mut s = c1 * v.env;

            if v.c2_mix > 0.0 {
                if v.m2_in_att {
                    v.m2_env += v.m2_att_inc;
                    if v.m2_env >= 1.0 {
                        v.m2_env = 1.0;
                        v.m2_in_att = false;
                    }
                } else {
                    v.m2_env = 0.04 + (v.m2_env - 0.04) * v.m2_dec;
                }
                match v.stage {
                    Stage::Attack => v.c2_env = v.env,
                    Stage::Hold => {
                        let s2 = v.sustain * 0.5;
                        v.c2_env = s2 + (v.c2_env - s2) * v.c2_dec;
                    }
                    Stage::Release => v.c2_env *= v.release,
                    Stage::Off => v.c2_env = 0.0,
                }
                let m2_dt = (dt_v * v.m2_ratio).min(0.45);
                let c2_dt = (dt_v * v.c2_ratio).min(0.45);
                v.m2_phase = wrap01(v.m2_phase + m2_dt);
                v.phase2 = wrap01(v.phase2 + c2_dt);
                let m2 = fast_sin_unit(v.m2_phase);
                let c2 = fast_sin_unit(wrap01(v.phase2 + m2 * (v.m2_idx * v.m2_env)));
                s = (s + v.c2_mix * c2 * v.c2_env) / (1.0 + v.c2_mix * 0.55);
            }
            if v.fm_drive > 0.0 {
                s = fast_tanh(s * v.fm_drive) * v.fm_inv_tanh;
            }
            s
        }
        Src::Wt16 => {
            let vib_fm = if v.vib_depth > 0.0 {
                v.vib = wrap01(v.vib + v.vib_dt);
                let amt = if v.vib_delay_left > 0 {
                    v.vib_delay_left -= 1;
                    0.0
                } else {
                    v.vib_ramp = (v.vib_ramp + v.vib_ramp_inc).min(1.0);
                    v.vib_ramp
                };
                1.0 + v.vib_depth * amt * fast_sin_unit(v.vib)
            } else {
                1.0
            };
            let dt_v = v.dt * vib_fm;
            v.phase = wrap01(v.phase + dt_v * (1.0 - v.wt_detune));
            v.phase2 = wrap01(v.phase2 + dt_v * (1.0 + v.wt_detune));
            v.wt_xfade = v.wt_xfade_floor + (v.wt_xfade - v.wt_xfade_floor) * v.wt_xfade_dec;
            let xf = v.wt_xfade;
            let bank = wavetables();
            let tb = &bank.tables[v.wt_bright as usize];
            let tw = &bank.tables[v.wt_warm as usize];
            let s1 = wt_sample(tb, v.phase) * xf + wt_sample(tw, v.phase) * (1.0 - xf);
            let s2 = wt_sample(tb, v.phase2) * xf + wt_sample(tw, v.phase2) * (1.0 - xf);
            let mut s = 0.54 * s1 + 0.46 * s2;
            if v.chiff_env > 1e-4 && v.chiff_mix > 0.0 {
                let n = noise(v);
                v.chiff_hp1 += v.chiff_hp_a * (n - v.chiff_hp1);
                let hp = n - v.chiff_hp1;
                v.chiff_lp1 += v.chiff_lp_a * (hp - v.chiff_lp1);
                s += v.chiff_lp1 * v.chiff_env * v.chiff_mix;
                v.chiff_env *= v.chiff_dec;
            }
            s
        }
        Src::Ks16 => {
            let pos = v.ks_pos;
            let mut s = if v.ks_init_left > 0 {
                v.ks_init_left -= 1;
                ks_buf[pos]
            } else {
                let r0 = (pos + KS_MAX_LEN - v.ks_d_int) & KS_MASK;
                let r1 = (r0 + KS_MAX_LEN - 1) & KS_MASK;
                let x_del = ks_buf[r0] * (1.0 - v.ks_d_frac) + ks_buf[r1] * v.ks_d_frac;
                v.lp1 += v.lp_a * (x_del - v.lp1);
                let out_s = v.lp1 * v.ks_rho;
                ks_buf[pos] = out_s;
                out_s
            };
            v.ks_pos = (pos + 1) & KS_MASK;
            if v.ks_hammer_env > 1e-4 && v.ks_hammer_mix > 0.0 {
                v.phase = wrap01(v.phase + v.dt);
                s += fast_sin_unit(v.phase) * v.ks_hammer_env * v.ks_hammer_mix;
                v.ks_hammer_env *= v.ks_hammer_dec;
            }
            s
        }
        Src::Fx16 => {
            v.phase = wrap01(v.phase + v.dt);
            v.phase2 = wrap01(v.phase2 + v.dt2);
            noise(v) * 0.62
                + fast_sin_unit(v.phase) * 0.30
                + fast_sin_unit(v.phase2) * 0.14
        }
        Src::Kick16 => {
            let hz = v.hz1 * (0.94 + 0.06 * v.env) + (v.hz0 - v.hz1) * v.m1_env;
            v.m1_env *= v.m1_dec;
            v.phase = wrap01(v.phase + hz / sr);
            let body = fast_sin_unit(v.phase)
                + 0.14 * fast_sin_unit(wrap01(v.phase * 2.0)) * v.env;
            let mut s = body * v.env;
            if v.chiff_env > 1e-4 {
                let n = noise(v);
                v.chiff_hp1 += v.chiff_hp_a * (n - v.chiff_hp1);
                let hp = n - v.chiff_hp1;
                v.chiff_lp1 += v.chiff_lp_a * (hp - v.chiff_lp1);
                let click_att = if v.stage == Stage::Attack { v.env } else { 1.0 };
                s += v.chiff_lp1 * v.chiff_env * click_att * v.chiff_mix;
                v.chiff_env *= v.chiff_dec;
            }
            fast_tanh(s * v.fm_drive) * v.fm_inv_tanh
        }
        Src::Snare16 => {
            let sh_hz = v.hz0 * (1.0 + 0.22 * v.m1_env);
            v.m1_env *= v.m1_dec;
            v.phase = wrap01(v.phase + sh_hz / sr);
            v.phase2 = wrap01(v.phase2 + (sh_hz * 1.72) / sr);
            let body = (fast_sin_unit(v.phase) * 0.72 + fast_sin_unit(v.phase2) * 0.28) * v.c2_env;
            v.c2_env *= v.c2_dec;

            let n = noise(v);
            v.lp1 += v.lp_a * (n - v.lp1);
            let hp = (n - v.lp1).clamp(-1.5, 1.5);
            v.lp2 += v.drum_lp_a * (hp - v.lp2);
            let crack = 1.0 + 0.45 * v.m2_env;
            v.m2_env *= v.m2_dec;
            (body * v.drum_body + v.lp2 * v.drum_wire * crack) * v.env
        }
        Src::Hat16 => {
            let n = noise(v);
            v.lp1 += v.lp_a * (n - v.lp1);
            let hp = (n - v.lp1).clamp(-1.5, 1.5);
            v.lp2 += v.drum_lp_a * (hp - v.lp2);
            let air = v.lp2;
            v.phase = wrap01(v.phase + v.dt);
            v.phase2 = wrap01(v.phase2 + v.dt2);
            v.drum_p3 = wrap01(v.drum_p3 + v.drum_dt3);
            let metal = (fast_sin_unit(v.phase)
                + fast_sin_unit(v.phase2)
                + fast_sin_unit(v.drum_p3))
                * 0.38
                * (0.72 + 0.28 * air);
            (air * (1.0 - v.drum_wire) + metal * v.drum_wire) * v.env
        }
        Src::Perc16 => {
            let kind = v.blend as u8;
            match kind {
                0 => {
                    let hz = v.hz1 + (v.hz0 - v.hz1) * v.m1_env;
                    v.m1_env *= v.m1_dec;
                    v.phase = wrap01(v.phase + hz / sr);
                    (fast_sin_unit(v.phase) * 0.86
                        + fast_sin_unit(wrap01(v.phase * 2.0)) * 0.16 * v.env)
                        * v.env
                }
                2 => {
                    let n = noise(v);
                    v.lp1 += v.lp_a * (n - v.lp1);
                    let hp = (n - v.lp1).clamp(-1.5, 1.5);
                    v.lp2 += v.drum_lp_a * (hp - v.lp2);
                    v.phase = wrap01(v.phase + v.dt);
                    v.phase2 = wrap01(v.phase2 + v.dt2);
                    v.drum_p3 = wrap01(v.drum_p3 + v.drum_dt3);
                    let metal = (fast_sin_unit(v.phase)
                        + fast_sin_unit(v.phase2)
                        + fast_sin_unit(v.drum_p3))
                        * 0.34;
                    (v.lp2 * 0.72 + metal * 0.28) * v.env
                }
                3 => {
                    v.phase = wrap01(v.phase + v.dt);
                    v.phase2 = wrap01(v.phase2 + v.dt2);
                    let c = fast_tanh(
                        (fast_sin_unit(v.phase) * 0.55 + fast_sin_unit(v.phase2) * 0.45) * 1.9,
                    ) * 0.85;
                    v.lp1 += v.lp_a * (c - v.lp1);
                    v.lp1 * v.env
                }
                4 => {
                    v.phase = wrap01(v.phase + v.dt);
                    v.phase2 = wrap01(v.phase2 + v.dt2);
                    (fast_sin_unit(v.phase) * 0.72 + fast_sin_unit(v.phase2) * 0.28) * v.env
                }
                5 => {
                    // 3-burst micro-flam handclap
                    let n = noise(v);
                    v.lp1 += v.lp_a * (n - v.lp1);
                    let hp = (n - v.lp1).clamp(-1.5, 1.5);
                    v.lp2 += v.drum_lp_a * (hp - v.lp2);
                    let t_ms = (v.drum_t + 1) as f32 * (1000.0 / sr);
                    v.drum_t += 1;
                    let mut eb = (-t_ms / 5.5).exp() * 0.85;
                    if t_ms >= 9.5 {
                        eb += (-(t_ms - 9.5) / 5.5).exp() * 0.85;
                    }
                    if t_ms >= 19.0 {
                        eb += (-(t_ms - 19.0) / 36.0).exp();
                    }
                    v.lp2 * eb * 1.05 * v.env
                }
                6 => {
                    // Rimshot
                    v.phase = wrap01(v.phase + v.dt);
                    v.phase2 = wrap01(v.phase2 + v.dt2);
                    let tone = fast_sin_unit(v.phase) * 0.62 + fast_sin_unit(v.phase2) * 0.38;
                    let n = noise(v);
                    v.lp1 += v.lp_a * (n - v.lp1);
                    let hp = n - v.lp1;
                    v.lp2 += v.drum_lp_a * (hp - v.lp2);
                    let s = (tone * 0.82 + v.lp2 * v.m1_env * 0.42) * v.env;
                    v.m1_env *= v.m1_dec;
                    s
                }
                _ => {
                    let n = noise(v);
                    v.lp1 += v.lp_a * (n - v.lp1);
                    let hp = (n - v.lp1).clamp(-1.5, 1.5);
                    v.lp2 += v.drum_lp_a * (hp - v.lp2);
                    v.lp2 * v.env
                }
            }
        }
    };

    // Brightness that darkens as the note decays (plucks, stabs, bright leads),
    // then a one- or two-pole low-pass and optional pad high-pass shelf.
    if v.lp_decay > 0.0 {
        v.lp_a += (v.lp_floor - v.lp_a) * v.lp_decay;
    }
    // Drums and Karplus-Strong run their own internal filters using `lp1`/`lp2`.
    let skip_lp = matches!(
        v.src,
        Src::Kick
            | Src::Snare
            | Src::Hat
            | Src::Perc
            | Src::Kick16
            | Src::Snare16
            | Src::Hat16
            | Src::Perc16
            | Src::Ks16
    );
    let mut filtered = if skip_lp || v.lp_a >= 0.999 {
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
    let env_applied = matches!(
        v.src,
        Src::Fm16 | Src::Kick16 | Src::Snare16 | Src::Hat16 | Src::Perc16
    );
    if env_applied {
        filtered
    } else {
        filtered * v.env
    }
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
            "Outfall" => Theme::Outfall,
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

    fn render_mode(
        theme: Theme,
        intensity: u32,
        rate: u32,
        loops: u32,
        mode: SynthMode,
    ) -> (Vec<f32>, SongSynth) {
        let mut synth = SongSynth::with_sample_rate_and_mode(
            theme,
            Arc::new(AtomicU32::new(intensity)),
            rate,
            mode,
        );
        let total = synth.loop_samples() * u64::from(loops);
        let mut out = Vec::with_capacity(total as usize);
        for _ in 0..total {
            out.push(synth.next().unwrap());
        }
        (out, synth)
    }

    #[test]
    fn both_16bit_and_chip_palettes_meet_mix_and_seam_targets() {
        for mode in [SynthMode::Bit16, SynthMode::Chip] {
            for theme in Theme::playable() {
                let (samples, synth) = render_mode(theme, 2, 22_050, 2, mode);
                assert_eq!(synth.mode(), mode);
                assert_eq!(synth.loops(), 2);
                assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
                let pk = peak(&samples);
                let r = rms(&samples);
                let d = dc(&samples);
                let crest = pk / r.max(1e-6);
                assert!(
                    (0.14..=0.55).contains(&pk),
                    "{mode:?} {theme:?} peak {pk:.3} out of [0.14, 0.55]"
                );
                assert!(
                    (0.045..=0.22).contains(&r),
                    "{mode:?} {theme:?} RMS {r:.3} out of [0.045, 0.22]"
                );
                assert!(d.abs() < 0.005, "{mode:?} {theme:?} DC {d:+.4}");
                assert!(
                    (1.5..=6.0).contains(&crest),
                    "{mode:?} {theme:?} crest {crest:.2} out of [1.5, 6.0]"
                );

                // Seam continuity across loop boundary.
                let lp = synth.loop_samples() as usize;
                let pre = lp - 256;
                let body = samples[..pre]
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .fold(0.0f32, f32::max);
                let win = &samples[pre..pre + 512];
                let max_delta = win
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .fold(0.0f32, f32::max);
                assert!(
                    max_delta <= body * 1.1 + 0.01 && max_delta < 0.30,
                    "{mode:?} {theme:?} seam max_delta={max_delta:.4} body={body:.4}"
                );
            }
        }
    }

    #[test]
    fn bit16_palette_is_distinct_from_chip_and_has_bright_presence() {
        for theme in Theme::playable() {
            let (b16, _) = render_mode(theme, 2, 22_050, 1, SynthMode::Bit16);
            let (chip, _) = render_mode(theme, 2, 22_050, 1, SynthMode::Chip);
            let n = b16.len().min(chip.len()).min(22_050 * 6);
            let diff = (b16[..n]
                .iter()
                .zip(&chip[..n])
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                / n as f32)
                .sqrt();
            assert!(
                diff > 0.03,
                "{theme:?}: 16-bit and chip sound too alike (diff_rms={diff:.4})"
            );
            // Measure 1-pole >2 kHz energy share on the 16-bit mix.
            let a = lp_coef(2_000.0, 22_050.0);
            let mut lp = 0.0f32;
            let mut total_e = 0.0f64;
            let mut hi_e = 0.0f64;
            for &s in &b16[..n] {
                lp += a * (s - lp);
                let hi = s - lp;
                total_e += f64::from(s) * f64::from(s);
                hi_e += f64::from(hi) * f64::from(hi);
            }
            let hi_share = hi_e / total_e.max(1e-12);
            assert!(
                hi_share > 0.04,
                "{theme:?}: 16-bit >2 kHz energy share {hi_share:.4} is too dull"
            );
        }
    }

    #[test]
    fn snes_echo_tail_stays_bounded_across_multiple_loops() {
        for theme in Theme::playable() {
            let (samples, synth) = render_mode(theme, 3, 22_050, 4, SynthMode::Bit16);
            assert_eq!(synth.loops(), 4);
            let q = samples.len() / 4;
            let r1 = rms(&samples[..q]);
            let r4 = rms(&samples[3 * q..]);
            let pk4 = peak(&samples[3 * q..]);
            assert!(
                (r4 - r1).abs() / r1.max(1e-6) < 0.06,
                "{theme:?}: echo energy drifted from loop 1 ({r1:.4}) to loop 4 ({r4:.4})"
            );
            assert!(pk4 <= 0.55, "{theme:?}: loop 4 peak {pk4:.3} over ceiling");
        }
    }

    #[test]
    fn wavetable_bank_is_normalized_and_dc_free() {
        let bank = wavetables();
        for (idx, table) in bank.tables.iter().enumerate() {
            let pk = peak(table);
            let mean = dc(table);
            assert!(table.iter().all(|s| s.is_finite()), "table {idx} non-finite");
            assert!((0.99..=1.01).contains(&pk), "table {idx} peak {pk:.4}");
            assert!(mean.abs() < 1e-4, "table {idx} DC {mean:+.6}");
        }
    }

    #[test]
    fn menu_track_enters_softly_in_16bit_calm_mode() {
        for theme in Theme::playable() {
            let mut synth = SongSynth::with_sample_rate_and_mode_menu(
                theme,
                Arc::new(AtomicU32::new(0)),
                22_050,
                SynthMode::Bit16,
                true,
            );
            assert_eq!(synth.mode(), SynthMode::Bit16);
            let bar_samples = (synth.song().steps_per_bar() * synth.samples_per_step) as usize;
            let mut first_bar = Vec::with_capacity(bar_samples);
            let mut drums_in_bar0 = 0usize;
            for i in 0..bar_samples {
                first_bar.push(synth.next().unwrap());
                if i % 64 == 0 {
                    drums_in_bar0 += synth.active_in_role(Role::Kick)
                        + synth.active_in_role(Role::Snare)
                        + synth.active_in_role(Role::Hat)
                        + synth.active_in_role(Role::Perc);
                }
            }
            assert_eq!(
                drums_in_bar0, 0,
                "{theme:?}: menu soft intro should keep drums out for the first two bars"
            );
            assert!(first_bar.iter().all(|s| s.is_finite() && s.abs() <= 0.55));
            // Fade-in starts quietly over the first 50 ms compared to the rest of bar 0.
            let onset = rms(&first_bar[..1_100]);
            let steady = rms(&first_bar[1_100..]);
            assert!(
                onset < steady,
                "{theme:?}: menu fade-in onset ({onset:.4}) should be softer than steady ({steady:.4})"
            );
        }
    }

    #[test]
    fn outfall_return_current_meets_mix_transition_and_palette_targets() {
        let song = song_for(Theme::Outfall);
        assert_eq!(song.name, "Return Current");
        assert_eq!(song.floor, "Outfall");
        assert_eq!(song.bpm, 84);
        assert_eq!(song.beats_per_bar, 4);
        assert_eq!(song.bars, 24);
        assert!(song.style.enhanced_drums);
        assert!(song.style.pad_detune <= 0.006);
        for role in Role::ALL {
            assert!(
                !song.parts[role.index()].is_empty(),
                "Outfall {} part should be populated",
                role.name()
            );
        }
        // Smooth intensity transitions across 0 -> 1 -> 2 -> 3 -> 2 -> 0 in both palettes:
        // switching intensity slews role gains over GAIN_SLEW_SECS so the transition
        // window never exceeds normal steady-state transient deltas.
        for mode in [SynthMode::Bit16, SynthMode::Chip] {
            let intensity = Arc::new(AtomicU32::new(0));
            let mut synth = SongSynth::with_sample_rate_and_mode(
                Theme::Outfall,
                Arc::clone(&intensity),
                22_050,
                mode,
            );
            let mut prev = 0.0f32;
            let mut body_delta = 0.0f32;
            for _ in 0..22_050 {
                let s = synth.next().unwrap();
                assert!(s.is_finite() && s.abs() <= 0.55);
                body_delta = body_delta.max((s - prev).abs());
                prev = s;
            }
            for &lvl in &[1u32, 2, 3, 2, 0] {
                intensity.store(lvl, Ordering::Relaxed);
                let mut trans_delta = 0.0f32;
                for i in 0..22_050 {
                    let s = synth.next().unwrap();
                    assert!(s.is_finite() && s.abs() <= 0.55);
                    let d = (s - prev).abs();
                    if i < 256 {
                        trans_delta = trans_delta.max(d);
                    } else {
                        body_delta = body_delta.max(d);
                    }
                    prev = s;
                }
                assert!(
                    trans_delta <= body_delta * 1.15 + 0.05 && trans_delta < 0.45,
                    "Outfall {mode:?} -> tier {lvl}: harsh transition step {trans_delta:.4} (body={body_delta:.4})"
                );
            }
        }
    }
}
