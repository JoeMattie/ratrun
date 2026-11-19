//! Procedural audio: a looping themed chiptune track + layered synthesized SFX.
//!
//! Everything is generated from scratch — no audio assets. The whole module
//! degrades to silent if no output device is available (headless, no server),
//! so the game never fails because of audio.

use std::f32::consts::{PI, TAU};
use std::num::NonZero;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::source::Source;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};

use crate::game::level::Theme;

pub const SR: u32 = 44_100;
const SR_F: f32 = SR as f32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sfx {
    Shoot,
    Hit,
    Kill,
    Explosion,
    Hurt,
    Gem,
    LevelUp,
    Pickup,
    Boss,
    Dash,
    UiMove,
    UiSelect,
    Win,
    Lose,
    // Specialized weapon, pickup, and boss effects.
    ShootSpray,
    ShootNova,
    ShootOrbit,
    ShootWhip,
    ShootAcid,
    Heal,
    Magnet,
    Nuke,
    BossDeath,
}

impl Sfx {
    /// Every effect, in cache order. `index` must agree with this ordering.
    pub const ALL: [Sfx; 23] = [
        Sfx::Shoot,
        Sfx::Hit,
        Sfx::Kill,
        Sfx::Explosion,
        Sfx::Hurt,
        Sfx::Gem,
        Sfx::LevelUp,
        Sfx::Pickup,
        Sfx::Boss,
        Sfx::Dash,
        Sfx::UiMove,
        Sfx::UiSelect,
        Sfx::Win,
        Sfx::Lose,
        Sfx::ShootSpray,
        Sfx::ShootNova,
        Sfx::ShootOrbit,
        Sfx::ShootWhip,
        Sfx::ShootAcid,
        Sfx::Heal,
        Sfx::Magnet,
        Sfx::Nuke,
        Sfx::BossDeath,
    ];

    /// Short uppercase identifier for diagnostics and logging.
    pub fn name(self) -> &'static str {
        match self {
            Sfx::Shoot => "SHOOT",
            Sfx::Hit => "HIT",
            Sfx::Kill => "KILL",
            Sfx::Explosion => "EXPLOSION",
            Sfx::Hurt => "HURT",
            Sfx::Gem => "GEM",
            Sfx::LevelUp => "LEVELUP",
            Sfx::Pickup => "PICKUP",
            Sfx::Boss => "BOSS",
            Sfx::Dash => "DASH",
            Sfx::UiMove => "UIMOVE",
            Sfx::UiSelect => "UISELECT",
            Sfx::Win => "WIN",
            Sfx::Lose => "LOSE",
            Sfx::ShootSpray => "SPRAY",
            Sfx::ShootNova => "NOVA",
            Sfx::ShootOrbit => "ORBIT",
            Sfx::ShootWhip => "WHIP",
            Sfx::ShootAcid => "ACID",
            Sfx::Heal => "HEAL",
            Sfx::Magnet => "MAGNET",
            Sfx::Nuke => "NUKE",
            Sfx::BossDeath => "BOSSDEATH",
        }
    }

    /// Number of pre-synthesized variants cached for this effect.
    pub fn variant_count(self) -> usize {
        match self {
            Sfx::Gem => 8, // rising pentatonic combo ladder
            Sfx::Hit | Sfx::Kill => 5,
            Sfx::Shoot | Sfx::Dash | Sfx::ShootSpray | Sfx::ShootWhip => 4,
            Sfx::Explosion
            | Sfx::Hurt
            | Sfx::UiMove
            | Sfx::ShootNova
            | Sfx::ShootOrbit
            | Sfx::ShootAcid => 3,
            Sfx::Pickup | Sfx::UiSelect | Sfx::Heal | Sfx::Magnet | Sfx::Nuke => 2,
            Sfx::LevelUp | Sfx::Boss | Sfx::Win | Sfx::Lose | Sfx::BossDeath => 1,
        }
    }

    /// Voice-stealing priority when the active voice pool is full.
    fn priority(self) -> u8 {
        match self {
            Sfx::Win | Sfx::Lose | Sfx::Boss | Sfx::BossDeath | Sfx::LevelUp => 3,
            Sfx::Hurt | Sfx::Nuke | Sfx::Explosion | Sfx::Heal | Sfx::Magnet | Sfx::Pickup => 2,
            Sfx::Kill
            | Sfx::Dash
            | Sfx::ShootNova
            | Sfx::ShootWhip
            | Sfx::ShootAcid
            | Sfx::ShootOrbit
            | Sfx::UiSelect => 1,
            Sfx::Shoot | Sfx::ShootSpray | Sfx::Hit | Sfx::Gem | Sfx::UiMove => 0,
        }
    }

    /// Position in the pre-synthesized sample cache.
    fn index(self) -> usize {
        self as usize
    }
}

// ---- DSP primitives ------------------------------------------------------

#[derive(Clone, Copy)]
struct Rng32(u32);

impl Rng32 {
    fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x6D2B_79F5 } else { seed })
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform white noise in [-1, 1].
    #[inline]
    fn white(&mut self) -> f32 {
        ((self.next_u32() >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }
}

/// Paul Kellet 3-pole approximation of -3 dB/octave pink noise.
#[derive(Clone, Copy, Default)]
struct PinkNoise {
    b0: f32,
    b1: f32,
    b2: f32,
}

impl PinkNoise {
    #[inline]
    fn next(&mut self, rng: &mut Rng32) -> f32 {
        let w = rng.white();
        self.b0 = 0.997_65 * self.b0 + w * 0.099_046_0;
        self.b1 = 0.963_00 * self.b1 + w * 0.296_516_4;
        self.b2 = 0.570_00 * self.b2 + w * 1.052_691_3;
        ((self.b0 + self.b1 + self.b2 + w * 0.184_8) * 0.24).clamp(-1.0, 1.0)
    }
}

/// PolyBLEP residual for band-limited pulse and sawtooth oscillators.
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let u = t / dt;
        u + u - u * u - 1.0
    } else if t > 1.0 - dt {
        let u = (t - 1.0) / dt;
        u * u + u + u + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Triangle,
    Pulse(f32), // duty cycle in (0, 1)
    Saw,
}

impl Wave {
    #[inline]
    fn sample(self, phase: f32, dt: f32) -> f32 {
        match self {
            Wave::Sine => (phase * TAU).sin(),
            Wave::Triangle => 2.0 * (2.0 * phase - 1.0).abs() - 1.0,
            Wave::Pulse(duty) => {
                let raw = if phase < duty { 1.0 } else { -1.0 };
                let p2 = (phase - duty).rem_euclid(1.0);
                (raw + poly_blep(phase, dt) - poly_blep(p2, dt)).clamp(-1.2, 1.2)
            }
            Wave::Saw => (2.0 * phase - 1.0 - poly_blep(phase, dt)).clamp(-1.2, 1.2),
        }
    }
}

/// Andrew Simper TPT 2-pole State Variable Filter (unconditionally stable).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1eq: f32,
    ic2eq: f32,
}

#[derive(Clone, Copy)]
enum FilterMode {
    None,
    LowPass,
    BandPass,
    HighPass,
}

impl Svf {
    #[inline]
    fn process(&mut self, v0: f32, cutoff_hz: f32, q: f32, mode: FilterMode) -> f32 {
        if matches!(mode, FilterMode::None) {
            return v0;
        }
        let fc = cutoff_hz.clamp(25.0, SR_F * 0.44);
        let g = (PI * fc / SR_F).tan();
        let k = 1.0 / q.clamp(0.5, 12.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = v0 - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        match mode {
            FilterMode::None => v0,
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => v0 - k * v1 - v2,
        }
    }
}

/// Smooth attack/decay envelope: raised-cosine attack then power decay.
#[inline]
fn ad_env(t: f32, dur_s: f32, attack_s: f32, decay_pow: f32) -> f32 {
    let elapsed = t * dur_s;
    let att = if attack_s <= 1e-6 || elapsed >= attack_s {
        1.0
    } else {
        0.5 * (1.0 - (PI * elapsed / attack_s).cos())
    };
    let rem = (1.0 - t).max(0.0);
    att * rem.powf(decay_pow)
}

/// Interpolate frequency exponentially (musically linear in pitch).
#[inline]
fn exp_glide(f0: f32, f1: f32, u: f32) -> f32 {
    if f0 <= 1.0 || f1 <= 1.0 {
        f0 + (f1 - f0) * u
    } else {
        f0 * (f1 / f0).powf(u)
    }
}

/// Parameters for a layered oscillator voice in `MixBuf`.
struct OscLayer {
    start_s: f32,
    dur_s: f32,
    f0: f32,
    f1: f32,
    pitch_pow: f32,
    wave: Wave,
    gain: f32,
    attack_s: f32,
    decay_pow: f32,
    filter: FilterMode,
    fc0: f32,
    fc1: f32,
    q: f32,
    vib_hz: f32,
    vib_depth: f32,
}

impl Default for OscLayer {
    fn default() -> Self {
        Self {
            start_s: 0.0,
            dur_s: 0.1,
            f0: 440.0,
            f1: 440.0,
            pitch_pow: 1.0,
            wave: Wave::Sine,
            gain: 0.3,
            attack_s: 0.002,
            decay_pow: 1.5,
            filter: FilterMode::None,
            fc0: 8000.0,
            fc1: 8000.0,
            q: 0.8,
            vib_hz: 0.0,
            vib_depth: 0.0,
        }
    }
}

/// Parameters for a 2-operator FM voice (bells, chimes, metallic/gritty bites).
struct FmLayer {
    start_s: f32,
    dur_s: f32,
    c0: f32,
    c1: f32,
    pitch_pow: f32,
    mod_ratio: f32,
    mod_index0: f32,
    mod_index1: f32,
    mod_pow: f32,
    gain: f32,
    attack_s: f32,
    decay_pow: f32,
    filter: FilterMode,
    fc0: f32,
    fc1: f32,
    q: f32,
}

impl Default for FmLayer {
    fn default() -> Self {
        Self {
            start_s: 0.0,
            dur_s: 0.1,
            c0: 880.0,
            c1: 880.0,
            pitch_pow: 1.0,
            mod_ratio: 2.0,
            mod_index0: 2.0,
            mod_index1: 0.1,
            mod_pow: 2.0,
            gain: 0.3,
            attack_s: 0.0015,
            decay_pow: 1.8,
            filter: FilterMode::None,
            fc0: 8000.0,
            fc1: 8000.0,
            q: 0.8,
        }
    }
}

/// Parameters for a sculpted noise layer (white or pink + swept SVF).
struct NoiseLayer {
    start_s: f32,
    dur_s: f32,
    pink: bool,
    gain: f32,
    attack_s: f32,
    decay_pow: f32,
    filter: FilterMode,
    fc0: f32,
    fc1: f32,
    fc_pow: f32,
    q: f32,
    trem_hz: f32,
    trem_depth: f32,
    seed: u32,
}

impl Default for NoiseLayer {
    fn default() -> Self {
        Self {
            start_s: 0.0,
            dur_s: 0.1,
            pink: false,
            gain: 0.25,
            attack_s: 0.002,
            decay_pow: 1.8,
            filter: FilterMode::LowPass,
            fc0: 4000.0,
            fc1: 600.0,
            fc_pow: 1.0,
            q: 0.9,
            trem_hz: 0.0,
            trem_depth: 0.0,
            seed: 0xA5A5_1234,
        }
    }
}

/// Accumulator buffer for multi-layered SFX synthesis.
struct MixBuf {
    samples: Vec<f32>,
}

impl MixBuf {
    fn new(dur_s: f32) -> Self {
        let n = ((dur_s * SR_F).round() as usize).max(64);
        Self {
            samples: vec![0.0; n],
        }
    }

    fn add_osc(&mut self, p: OscLayer) {
        let start = (p.start_s * SR_F).round() as usize;
        let n = ((p.dur_s * SR_F).round() as usize).max(1);
        let end = (start + n).min(self.samples.len());
        if start >= end {
            return;
        }
        let mut phase = 0.0f32;
        let mut svf = Svf::default();
        let inv_n = 1.0 / n as f32;
        for (k, slot) in self.samples[start..end].iter_mut().enumerate() {
            let t = k as f32 * inv_n;
            let u = t.powf(p.pitch_pow);
            let mut f = exp_glide(p.f0, p.f1, u);
            if p.vib_depth > 0.0 && p.vib_hz > 0.0 {
                let vib = (k as f32 / SR_F * p.vib_hz * TAU).sin();
                f *= 1.0 + p.vib_depth * vib * t.min(1.0);
            }
            let dt = (f / SR_F).clamp(0.0, 0.45);
            phase = (phase + dt).fract();
            let raw = p.wave.sample(phase, dt);
            let fc = exp_glide(p.fc0, p.fc1, t);
            let filtered = svf.process(raw, fc, p.q, p.filter);
            let env = ad_env(t, p.dur_s, p.attack_s, p.decay_pow);
            *slot += filtered * env * p.gain;
        }
    }

    fn add_fm(&mut self, p: FmLayer) {
        let start = (p.start_s * SR_F).round() as usize;
        let n = ((p.dur_s * SR_F).round() as usize).max(1);
        let end = (start + n).min(self.samples.len());
        if start >= end {
            return;
        }
        let mut c_phase = 0.0f32;
        let mut m_phase = 0.0f32;
        let mut svf = Svf::default();
        let inv_n = 1.0 / n as f32;
        for (k, slot) in self.samples[start..end].iter_mut().enumerate() {
            let t = k as f32 * inv_n;
            let f = exp_glide(p.c0, p.c1, t.powf(p.pitch_pow));
            let dt_c = (f / SR_F).clamp(0.0, 0.45);
            let dt_m = (f * p.mod_ratio / SR_F).clamp(0.0, 0.45);
            c_phase = (c_phase + dt_c).fract();
            m_phase = (m_phase + dt_m).fract();
            let idx = p.mod_index1 + (p.mod_index0 - p.mod_index1) * (1.0 - t).powf(p.mod_pow);
            let raw = (c_phase * TAU + idx * (m_phase * TAU).sin()).sin();
            let fc = exp_glide(p.fc0, p.fc1, t);
            let filtered = svf.process(raw, fc, p.q, p.filter);
            let env = ad_env(t, p.dur_s, p.attack_s, p.decay_pow);
            *slot += filtered * env * p.gain;
        }
    }

    fn add_noise(&mut self, p: NoiseLayer) {
        let start = (p.start_s * SR_F).round() as usize;
        let n = ((p.dur_s * SR_F).round() as usize).max(1);
        let end = (start + n).min(self.samples.len());
        if start >= end {
            return;
        }
        let mut rng = Rng32::new(p.seed);
        let mut pink = PinkNoise::default();
        let mut svf = Svf::default();
        let inv_n = 1.0 / n as f32;
        for (k, slot) in self.samples[start..end].iter_mut().enumerate() {
            let t = k as f32 * inv_n;
            let raw = if p.pink {
                pink.next(&mut rng)
            } else {
                rng.white()
            };
            let fc = exp_glide(p.fc0, p.fc1, t.powf(p.fc_pow));
            let filtered = svf.process(raw, fc, p.q, p.filter);
            let mut env = ad_env(t, p.dur_s, p.attack_s, p.decay_pow);
            if p.trem_depth > 0.0 && p.trem_hz > 0.0 {
                let lfo = 0.5 + 0.5 * (k as f32 / SR_F * p.trem_hz * TAU).sin();
                env *= 1.0 - p.trem_depth * (1.0 - lfo);
            }
            *slot += filtered * env * p.gain;
        }
    }

    /// Add subtle multi-tap damped early reflections for celebratory or big sounds.
    fn add_short_tail(&mut self, wet: f32, damp_hz: f32) {
        let taps: [(usize, f32); 3] = [
            ((0.017 * SR_F) as usize, 0.52),
            ((0.031 * SR_F) as usize, 0.36),
            ((0.053 * SR_F) as usize, 0.22),
        ];
        let dry = self.samples.clone();
        let alpha = (TAU * damp_hz / SR_F).clamp(0.02, 0.85);
        let mut lp = 0.0f32;
        for i in 0..self.samples.len() {
            let mut sum = 0.0f32;
            for &(d, g) in &taps {
                if i >= d {
                    sum += dry[i - d] * g;
                }
            }
            lp += alpha * (sum - lp);
            self.samples[i] += lp * wet;
        }
    }

    /// DC-block, soft-saturate, normalize to `target_peak`, and apply click-free
    /// cosine fades at both ends.
    fn finish(mut self, target_peak: f32, drive: f32, fade_in_ms: f32, fade_out_ms: f32) -> Vec<f32> {
        // 1. 1-pole DC blocker (~18 Hz) to eliminate sub-audible offset.
        let r = 0.9974f32;
        let mut prev_x = 0.0f32;
        let mut prev_y = 0.0f32;
        for s in &mut self.samples {
            let x = *s;
            let y = x - prev_x + r * prev_y;
            prev_x = x;
            prev_y = y;
            *s = y;
        }

        // 2. Soft saturation for analog punch without hard clipping.
        if drive > 1.001 {
            let norm = 1.0 / drive.tanh();
            for s in &mut self.samples {
                *s = (*s * drive).tanh() * norm;
            }
        }

        // 3. Peak-normalize to the deliberate mix level.
        let peak = self
            .samples
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        if peak > 1e-6 {
            let scale = target_peak.clamp(0.0, 0.98) / peak;
            for s in &mut self.samples {
                *s *= scale;
            }
        }

        // 4. Click-free raised-cosine fade-in & fade-out.
        let n = self.samples.len();
        let fi = ((fade_in_ms * 0.001 * SR_F).round() as usize).min(n / 4);
        for (i, s) in self.samples.iter_mut().take(fi).enumerate() {
            let u = i as f32 / fi.max(1) as f32;
            *s *= 0.5 * (1.0 - (PI * u).cos());
        }
        let fo = ((fade_out_ms * 0.001 * SR_F).round() as usize).min(n / 2);
        for i in 0..fo {
            let u = (i + 1) as f32 / fo.max(1) as f32;
            let w = 0.5 * (1.0 + (PI * u).cos());
            self.samples[n - fo + i] *= w;
        }
        if let Some(first) = self.samples.first_mut() {
            *first = 0.0;
        }
        if let Some(last) = self.samples.last_mut() {
            *last = 0.0;
        }

        for s in &mut self.samples {
            *s = s.clamp(-1.0, 1.0);
        }
        self.samples
    }
}

/// Build the primary sample buffer for an SFX (mono, SR).
pub fn sfx_samples(sfx: Sfx) -> Vec<f32> {
    sfx_variant_samples(sfx, 0)
}

/// Number of cached variants for an SFX.
pub fn sfx_variant_count(sfx: Sfx) -> usize {
    sfx.variant_count()
}

/// Build a specific cached variant buffer for an SFX (mono, SR).
pub fn sfx_variant_samples(sfx: Sfx, variant: usize) -> Vec<f32> {
    let v = variant % sfx.variant_count();
    let base_seed = 0x51A7_1001u32
        .wrapping_add((sfx.index() as u32).wrapping_mul(0x9E37_79B9))
        .wrapping_add((v as u32).wrapping_mul(0x7F4A_7C15));

    match sfx {
        // 1. Gnaw dart: crisp tooth-snap click + tight resonant pulse dart + warm body.
        Sfx::Shoot => {
            const PITCH: [f32; 4] = [1.0, 1.10, 0.90, 1.04];
            const DUTY: [f32; 4] = [0.22, 0.36, 0.28, 0.42];
            const LP0: [f32; 4] = [3800.0, 2900.0, 2400.0, 3400.0];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.060);
            // Crisp transient click
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.012,
                pink: false,
                gain: 0.32,
                attack_s: 0.0008,
                decay_pow: 2.6,
                filter: FilterMode::BandPass,
                fc0: 3400.0 * p,
                fc1: 1600.0 * p,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            // Band-limited dart sweep
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.054,
                f0: 1180.0 * p,
                f1: 270.0 * p,
                pitch_pow: 0.52,
                wave: Wave::Pulse(DUTY[v]),
                gain: 0.54,
                attack_s: 0.0012,
                decay_pow: 1.85,
                filter: FilterMode::LowPass,
                fc0: LP0[v],
                fc1: 900.0,
                q: 1.45,
                ..Default::default()
            });
            // Tactile low-mid body
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.042,
                f0: 540.0 * p,
                f1: 165.0 * p,
                pitch_pow: 0.60,
                wave: Wave::Sine,
                gain: 0.42,
                attack_s: 0.0012,
                decay_pow: 1.7,
                ..Default::default()
            });
            m.finish(0.32, 1.4, 1.1, 5.5)
        }

        // 2. Enemy hit: crisp percussive smack + tight clean thud + short attack bite.
        Sfx::Hit => {
            const PITCH: [f32; 5] = [1.0, 0.90, 1.11, 0.95, 1.05];
            const SNAP_F: [f32; 5] = [2150.0, 1580.0, 2550.0, 1800.0, 2320.0];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.050);
            // Immediate crisp smack transient
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.016,
                pink: false,
                gain: 0.72,
                attack_s: 0.0008,
                decay_pow: 2.5,
                filter: FilterMode::BandPass,
                fc0: SNAP_F[v],
                fc1: SNAP_F[v] * 0.50,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            // Clean tight impact body thud (no mid-decay beating)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.044,
                f0: 320.0 * p,
                f1: 88.0 * p,
                pitch_pow: 0.40,
                wave: Wave::Sine,
                gain: 0.68,
                attack_s: 0.001,
                decay_pow: 1.85,
                ..Default::default()
            });
            // Brief crisp attack bite confined to onset
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.018,
                c0: 520.0 * p,
                c1: 190.0 * p,
                pitch_pow: 0.55,
                mod_ratio: 2.35,
                mod_index0: 2.6,
                mod_index1: 0.15,
                mod_pow: 2.6,
                gain: 0.36,
                attack_s: 0.0008,
                decay_pow: 2.3,
                filter: FilterMode::LowPass,
                fc0: 2900.0,
                fc1: 900.0,
                q: 0.95,
            });
            m.finish(0.31, 1.55, 1.0, 5.5)
        }

        // 3. Kill: juicy arcade pop-crunch — impact punch + pronounced upward pop + crunch.
        Sfx::Kill => {
            const PITCH: [f32; 5] = [1.0, 1.10, 0.92, 1.18, 1.04];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.118);
            // Impact sub-punch
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.068,
                f0: 280.0 * p,
                f1: 72.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Sine,
                gain: 0.66,
                attack_s: 0.0012,
                decay_pow: 1.75,
                ..Default::default()
            });
            // Crisp crunch burst
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.088,
                pink: true,
                gain: 0.48,
                attack_s: 0.0012,
                decay_pow: 1.95,
                filter: FilterMode::BandPass,
                fc0: 2400.0 * p,
                fc1: 520.0 * p,
                q: 1.35,
                seed: base_seed,
                ..Default::default()
            });
            // Pronounced juicy upward arcade pop
            m.add_osc(OscLayer {
                start_s: 0.005,
                dur_s: 0.056,
                f0: 290.0 * p,
                f1: 780.0 * p,
                pitch_pow: 0.68,
                wave: Wave::Triangle,
                gain: 0.58,
                attack_s: 0.0025,
                decay_pow: 1.55,
                ..Default::default()
            });
            // Short bright confirm sparkle
            m.add_fm(FmLayer {
                start_s: 0.020,
                dur_s: 0.086,
                c0: 740.0 * p,
                c1: 480.0 * p,
                pitch_pow: 1.0,
                mod_ratio: 1.5,
                mod_index0: 1.9,
                mod_index1: 0.1,
                mod_pow: 2.1,
                gain: 0.34,
                attack_s: 0.0025,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 3400.0,
                fc1: 1150.0,
                q: 1.1,
            });
            m.finish(0.45, 1.6, 1.1, 9.0)
        }

        // 4. Explosion: simultaneous sub-drop + sharp crack + lowpass-swept pink rumble.
        Sfx::Explosion => {
            const PITCH: [f32; 3] = [1.0, 0.92, 1.07];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.46);
            // Chest-thump sub drop
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.24,
                f0: 150.0 * p,
                f1: 34.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Sine,
                gain: 0.85,
                attack_s: 0.002,
                decay_pow: 1.6,
                ..Default::default()
            });
            // Mid-bass growl body
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.20,
                f0: 110.0 * p,
                f1: 38.0 * p,
                pitch_pow: 0.55,
                wave: Wave::Saw,
                gain: 0.35,
                attack_s: 0.002,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 950.0,
                fc1: 140.0,
                q: 1.3,
                ..Default::default()
            });
            // Sharp blast crack at t=0
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.042,
                pink: false,
                gain: 0.55,
                attack_s: 0.001,
                decay_pow: 2.6,
                filter: FilterMode::BandPass,
                fc0: 2600.0,
                fc1: 900.0,
                q: 1.1,
                seed: base_seed ^ 0x1111,
                ..Default::default()
            });
            // Rolling lowpass-swept blast roar
            m.add_noise(NoiseLayer {
                start_s: 0.004,
                dur_s: 0.43,
                pink: true,
                gain: 0.78,
                attack_s: 0.005,
                decay_pow: 1.65,
                filter: FilterMode::LowPass,
                fc0: 4200.0 * p,
                fc1: 160.0,
                fc_pow: 0.55,
                q: 1.4,
                seed: base_seed,
                ..Default::default()
            });
            m.add_short_tail(0.16, 1800.0);
            m.finish(0.74, 1.8, 1.5, 22.0)
        }

        // 5. Hurt: urgent dissonant growl + sharp smack + tiny pained rat-squeak glint.
        Sfx::Hurt => {
            const PITCH: [f32; 3] = [1.0, 0.94, 1.06];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.24);
            // Impact slap
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.048,
                pink: true,
                gain: 0.52,
                attack_s: 0.001,
                decay_pow: 2.2,
                filter: FilterMode::BandPass,
                fc0: 1950.0,
                fc1: 680.0,
                q: 1.5,
                seed: base_seed,
                ..Default::default()
            });
            // Dissonant minor-2nd twin sawtooth crunch
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.21,
                f0: 295.0 * p,
                f1: 86.0 * p,
                pitch_pow: 0.6,
                wave: Wave::Saw,
                gain: 0.52,
                attack_s: 0.002,
                decay_pow: 1.5,
                filter: FilterMode::LowPass,
                fc0: 1900.0,
                fc1: 420.0,
                q: 1.6,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.20,
                f0: 314.0 * p,
                f1: 93.0 * p,
                pitch_pow: 0.62,
                wave: Wave::Pulse(0.35),
                gain: 0.42,
                attack_s: 0.002,
                decay_pow: 1.6,
                filter: FilterMode::LowPass,
                fc0: 1700.0,
                fc1: 380.0,
                q: 1.4,
                ..Default::default()
            });
            // Brief thematic rat squeak glint on top
            m.add_fm(FmLayer {
                start_s: 0.004,
                dur_s: 0.072,
                c0: 1620.0 * p,
                c1: 1120.0 * p,
                pitch_pow: 0.85,
                mod_ratio: 1.45,
                mod_index0: 1.2,
                mod_index1: 0.1,
                mod_pow: 2.0,
                gain: 0.22,
                attack_s: 0.003,
                decay_pow: 1.9,
                filter: FilterMode::BandPass,
                fc0: 1800.0,
                fc1: 1100.0,
                q: 1.2,
            });
            m.finish(0.68, 1.85, 1.5, 16.0)
        }

        // 6. Gem: 8-step rising pentatonic glassy bell chime!
        Sfx::Gem => {
            const SCALE_HZ: [f32; 8] = [
                1046.50, // C6
                1174.66, // D6
                1318.51, // E6
                1567.98, // G6
                1760.00, // A6
                2093.00, // C7
                2349.32, // D7
                2637.02, // E7
            ];
            let f = SCALE_HZ[v];
            let mut m = MixBuf::new(0.088);
            // Pure crystalline bell fundamental with micro upward bloom
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.082,
                f0: f * 0.97,
                f1: f,
                pitch_pow: 0.25,
                wave: Wave::Sine,
                gain: 0.68,
                attack_s: 0.0018,
                decay_pow: 1.9,
                ..Default::default()
            });
            // Glassy FM overtone ping (decays quickly so it sparkles, not rings)
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.068,
                c0: f,
                c1: f * 1.005,
                pitch_pow: 0.4,
                mod_ratio: 2.0,
                mod_index0: 1.35,
                mod_index1: 0.05,
                mod_pow: 2.8,
                gain: 0.36,
                attack_s: 0.0015,
                decay_pow: 2.4,
                filter: FilterMode::LowPass,
                fc0: 6800.0,
                fc1: 2800.0,
                q: 0.8,
            });
            // Subtle warm octave-below triangle body
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.058,
                f0: f * 0.5,
                f1: f * 0.5,
                wave: Wave::Triangle,
                gain: 0.22,
                attack_s: 0.002,
                decay_pow: 2.0,
                ..Default::default()
            });
            m.finish(0.30, 1.15, 1.4, 9.0)
        }

        // 7. Generic Pickup: bright overlapping major-9th arpeggio chime.
        Sfx::Pickup => {
            let root = if v == 0 { 659.25 } else { 739.99 }; // E5 or F#5
            let notes = [root, root * 1.5, root * 2.25];
            let mut m = MixBuf::new(0.28);
            for (i, &f) in notes.iter().enumerate() {
                let st = i as f32 * 0.048;
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: 0.16,
                    c0: f * 0.985,
                    c1: f,
                    pitch_pow: 0.3,
                    mod_ratio: 2.0,
                    mod_index0: 1.4,
                    mod_index1: 0.1,
                    mod_pow: 2.5,
                    gain: 0.45,
                    attack_s: 0.003,
                    decay_pow: 2.0,
                    filter: FilterMode::LowPass,
                    fc0: 5500.0,
                    fc1: 2200.0,
                    q: 0.9,
                });
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.15,
                    f0: f * 0.5,
                    f1: f * 0.5,
                    wave: Wave::Triangle,
                    gain: 0.26,
                    attack_s: 0.004,
                    decay_pow: 1.8,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.14, 3400.0);
            m.finish(0.46, 1.2, 1.5, 18.0)
        }

        // 8. LevelUp: lush overlapping chiptune-synth heroic fanfare with harmony + tail.
        Sfx::LevelUp => {
            let mut m = MixBuf::new(0.64);
            let melody: [(f32, f32, f32); 4] = [
                (0.000, 523.25, 0.20), // C5
                (0.068, 659.25, 0.20), // E5
                (0.136, 783.99, 0.22), // G5
                (0.204, 1046.50, 0.40), // C6 hero hold
            ];
            for (i, &(st, f, dur)) in melody.iter().enumerate() {
                let is_last = i == 3;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: f,
                    f1: f,
                    wave: Wave::Pulse(0.35),
                    gain: if is_last { 0.46 } else { 0.36 },
                    attack_s: 0.004,
                    decay_pow: if is_last { 1.4 } else { 1.8 },
                    filter: FilterMode::LowPass,
                    fc0: 4200.0,
                    fc1: 1800.0,
                    q: 1.1,
                    vib_hz: if is_last { 5.8 } else { 0.0 },
                    vib_depth: if is_last { 0.010 } else { 0.0 },
                    ..Default::default()
                });
                // Octave bell shimmer
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: dur * 0.85,
                    c0: f * 2.0,
                    c1: f * 2.0,
                    mod_ratio: 2.0,
                    mod_index0: 1.1,
                    mod_index1: 0.05,
                    mod_pow: 2.8,
                    gain: 0.20,
                    attack_s: 0.003,
                    decay_pow: 2.2,
                    filter: FilterMode::LowPass,
                    fc0: 6200.0,
                    fc1: 2600.0,
                    q: 0.8,
                    ..Default::default()
                });
            }
            // Harmony E6 + warm bass foundation on climax
            m.add_osc(OscLayer {
                start_s: 0.204,
                dur_s: 0.38,
                f0: 1318.51,
                f1: 1318.51,
                wave: Wave::Triangle,
                gain: 0.28,
                attack_s: 0.012,
                decay_pow: 1.5,
                vib_hz: 5.8,
                vib_depth: 0.008,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.52,
                f0: 261.63,
                f1: 261.63,
                wave: Wave::Triangle,
                gain: 0.34,
                attack_s: 0.012,
                decay_pow: 1.4,
                ..Default::default()
            });
            m.add_short_tail(0.20, 3200.0);
            m.finish(0.62, 1.3, 2.0, 28.0)
        }

        // 9. Boss: menacing brassy war-horn + trembling sub-rumble + twin heavy slams.
        Sfx::Boss => {
            let mut m = MixBuf::new(0.96);
            // Twin heavy industrial slams at 0ms and 260ms
            for &st in &[0.0f32, 0.26] {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.18,
                    f0: 135.0,
                    f1: 38.0,
                    pitch_pow: 0.45,
                    wave: Wave::Sine,
                    gain: 0.72,
                    attack_s: 0.002,
                    decay_pow: 1.8,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.08,
                    pink: true,
                    gain: 0.38,
                    attack_s: 0.002,
                    decay_pow: 2.2,
                    filter: FilterMode::BandPass,
                    fc0: 1400.0,
                    fc1: 420.0,
                    q: 2.2,
                    seed: base_seed ^ (st.to_bits()),
                    ..Default::default()
                });
            }
            // Detuned brassy war-horn dropping a brooding semitone (F#2 -> F2)
            for &(detune, gain) in &[(0.996f32, 0.48f32), (1.005, 0.48), (2.0, 0.26)] {
                m.add_osc(OscLayer {
                    start_s: 0.04,
                    dur_s: 0.88,
                    f0: 92.50 * detune,
                    f1: 87.31 * detune,
                    pitch_pow: 1.2,
                    wave: Wave::Saw,
                    gain,
                    attack_s: 0.08,
                    decay_pow: 1.25,
                    filter: FilterMode::LowPass,
                    fc0: 1450.0,
                    fc1: 240.0,
                    q: 2.1,
                    ..Default::default()
                });
            }
            // Trembling dark sub-rumble
            m.add_noise(NoiseLayer {
                start_s: 0.02,
                dur_s: 0.90,
                pink: true,
                gain: 0.48,
                attack_s: 0.04,
                decay_pow: 1.4,
                filter: FilterMode::LowPass,
                fc0: 360.0,
                fc1: 110.0,
                q: 1.3,
                trem_hz: 9.5,
                trem_depth: 0.45,
                seed: base_seed,
                ..Default::default()
            });
            m.add_short_tail(0.20, 1400.0);
            m.finish(0.76, 1.75, 2.0, 35.0)
        }

        // 10. Dash: silky aerodynamic bandpass swoosh + subtle paw pushoff + air-zip.
        Sfx::Dash => {
            const PITCH: [f32; 4] = [1.0, 1.09, 0.92, 1.04];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.140);
            // Subtle paw push-off thump (balanced so swoosh owns the envelope)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.030,
                f0: 175.0 * p,
                f1: 72.0 * p,
                pitch_pow: 0.55,
                wave: Wave::Sine,
                gain: 0.26,
                attack_s: 0.002,
                decay_pow: 1.7,
                ..Default::default()
            });
            // Bandpass-swept air swoosh (low -> bright -> soft)
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.064,
                pink: true,
                gain: 0.84,
                attack_s: 0.008,
                decay_pow: 1.0,
                filter: FilterMode::BandPass,
                fc0: 500.0 * p,
                fc1: 2650.0 * p,
                fc_pow: 0.8,
                q: 2.2,
                seed: base_seed,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.032,
                dur_s: 0.105,
                pink: true,
                gain: 0.74,
                attack_s: 0.012,
                decay_pow: 1.85,
                filter: FilterMode::BandPass,
                fc0: 2500.0 * p,
                fc1: 640.0 * p,
                fc_pow: 1.1,
                q: 1.9,
                seed: base_seed ^ 0x55AA,
                ..Default::default()
            });
            // Subtle kinetic upward glide
            m.add_osc(OscLayer {
                start_s: 0.004,
                dur_s: 0.085,
                f0: 340.0 * p,
                f1: 920.0 * p,
                pitch_pow: 0.7,
                wave: Wave::Sine,
                gain: 0.28,
                attack_s: 0.006,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.finish(0.38, 1.45, 2.0, 12.0)
        }

        // 11. UiMove: velvety tactile click-pip (soft, non-fatiguing menu tick).
        Sfx::UiMove => {
            const PITCH: [f32; 3] = [1.0, 1.04, 0.96];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.034);
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.030,
                f0: 760.0 * p,
                f1: 960.0 * p,
                pitch_pow: 0.5,
                wave: Wave::Triangle,
                gain: 0.65,
                attack_s: 0.0015,
                decay_pow: 2.2,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.008,
                pink: false,
                gain: 0.22,
                attack_s: 0.0008,
                decay_pow: 2.8,
                filter: FilterMode::BandPass,
                fc0: 2400.0,
                fc1: 1800.0,
                q: 2.0,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.24, 1.1, 1.2, 5.0)
        }

        // 12. UiSelect: crisp affirmative two-note harmonic chime.
        Sfx::UiSelect => {
            let f0 = if v == 0 { 659.25 } else { 698.46 };
            let f1 = f0 * 1.5;
            let mut m = MixBuf::new(0.16);
            // Note 1
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.085,
                f0,
                f1: f0,
                wave: Wave::Triangle,
                gain: 0.55,
                attack_s: 0.002,
                decay_pow: 1.9,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.075,
                f0,
                f1: f0,
                wave: Wave::Pulse(0.25),
                gain: 0.22,
                attack_s: 0.002,
                decay_pow: 2.1,
                filter: FilterMode::LowPass,
                fc0: 3200.0,
                fc1: 1400.0,
                q: 0.9,
                ..Default::default()
            });
            // Note 2 (overlapping fifth up)
            m.add_osc(OscLayer {
                start_s: 0.044,
                dur_s: 0.11,
                f0: f1,
                f1,
                wave: Wave::Triangle,
                gain: 0.62,
                attack_s: 0.002,
                decay_pow: 1.8,
                ..Default::default()
            });
            m.add_fm(FmLayer {
                start_s: 0.044,
                dur_s: 0.10,
                c0: f1,
                c1: f1,
                mod_ratio: 2.0,
                mod_index0: 0.9,
                mod_index1: 0.05,
                mod_pow: 2.4,
                gain: 0.26,
                attack_s: 0.002,
                decay_pow: 2.1,
                ..Default::default()
            });
            m.finish(0.38, 1.15, 1.4, 12.0)
        }

        // 13. Win: triumphant harmonized victory fanfare with legato bass + triad bloom.
        Sfx::Win => {
            let mut m = MixBuf::new(1.06);
            let melody: [(f32, f32, f32, f32); 5] = [
                (0.000, 523.25, 0.21, 261.63), // C5 + C4 bass
                (0.100, 659.25, 0.21, 329.63), // E5 + E4
                (0.200, 783.99, 0.21, 392.00), // G5 + G4
                (0.300, 880.00, 0.22, 440.00), // A5 + A4
                (0.405, 1046.50, 0.62, 261.63), // C6 triumph hold + C4 anchor
            ];
            for (i, &(st, f, dur, bf)) in melody.iter().enumerate() {
                let last = i == 4;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: f,
                    f1: f,
                    wave: Wave::Pulse(0.38),
                    gain: if last { 0.34 } else { 0.36 },
                    attack_s: 0.005,
                    decay_pow: if last { 1.3 } else { 1.25 },
                    filter: FilterMode::LowPass,
                    fc0: 4200.0,
                    fc1: 1800.0,
                    q: 1.0,
                    vib_hz: if last { 5.5 } else { 0.0 },
                    vib_depth: if last { 0.011 } else { 0.0 },
                    ..Default::default()
                });
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur * 1.15,
                    f0: bf,
                    f1: bf,
                    wave: Wave::Triangle,
                    gain: if last { 0.28 } else { 0.24 },
                    attack_s: 0.008,
                    decay_pow: 1.15,
                    ..Default::default()
                });
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: dur * 0.85,
                    c0: f * 2.0,
                    c1: f * 2.0,
                    mod_ratio: 2.0,
                    mod_index0: 0.85,
                    mod_index1: 0.05,
                    mod_pow: 2.5,
                    gain: 0.14,
                    attack_s: 0.004,
                    decay_pow: 2.0,
                    filter: FilterMode::LowPass,
                    fc0: 5600.0,
                    fc1: 2200.0,
                    q: 0.8,
                    ..Default::default()
                });
            }
            // Balanced final major triad bloom (gentle attack so level stays cohesive)
            for &(f, g) in &[(523.25f32, 0.16f32), (659.25, 0.17), (783.99, 0.17)] {
                m.add_osc(OscLayer {
                    start_s: 0.405,
                    dur_s: 0.60,
                    f0: f,
                    f1: f,
                    wave: Wave::Triangle,
                    gain: g,
                    attack_s: 0.022,
                    decay_pow: 1.4,
                    vib_hz: 5.5,
                    vib_depth: 0.007,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.22, 3000.0);
            m.finish(0.66, 1.35, 2.0, 42.0)
        }

        // 14. Lose: moody legato descending minor cadence with sighing pitch bend.
        Sfx::Lose => {
            let mut m = MixBuf::new(0.94);
            // Dark sustained minor pad underneath so notes flow smoothly together
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.90,
                f0: 110.0, // A2
                f1: 108.5,
                pitch_pow: 2.0,
                wave: Wave::Triangle,
                gain: 0.34,
                attack_s: 0.025,
                decay_pow: 1.25,
                ..Default::default()
            });
            let notes: [(f32, f32, f32, f32); 4] = [
                (0.00, 440.00, 432.0, 0.24), // A4
                (0.16, 392.00, 384.0, 0.24), // G4
                (0.32, 349.23, 340.0, 0.25), // F4
                (0.48, 329.63, 246.94, 0.44), // E4 -> deep sigh down to B3
            ];
            for &(st, f0, f1, dur) in &notes {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0,
                    f1,
                    pitch_pow: 1.7,
                    wave: Wave::Pulse(0.42),
                    gain: 0.38,
                    attack_s: 0.014,
                    decay_pow: 1.35,
                    filter: FilterMode::LowPass,
                    fc0: 2000.0,
                    fc1: 440.0,
                    q: 1.0,
                    ..Default::default()
                });
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: f0 * 0.5,
                    f1: f1 * 0.5,
                    pitch_pow: 1.7,
                    wave: Wave::Triangle,
                    gain: 0.28,
                    attack_s: 0.015,
                    decay_pow: 1.35,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.20, 1400.0);
            m.finish(0.60, 1.35, 2.5, 38.0)
        }

        // 15. CheeseSpray: chunky 3-pellet staggered scatter burst!
        Sfx::ShootSpray => {
            const PITCH: [f32; 4] = [1.0, 1.08, 0.92, 1.04];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.092);
            for (i, &(st, mul)) in [(0.0f32, 1.0f32), (0.015, 1.15), (0.030, 0.90)]
                .iter()
                .enumerate()
            {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.044,
                    f0: 780.0 * p * mul,
                    f1: 200.0 * p * mul,
                    pitch_pow: 0.5,
                    wave: Wave::Pulse(0.40),
                    gain: 0.38,
                    attack_s: 0.0012,
                    decay_pow: 2.0,
                    filter: FilterMode::LowPass,
                    fc0: 2600.0,
                    fc1: 750.0,
                    q: 1.3,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.022,
                    pink: true,
                    gain: 0.34,
                    attack_s: 0.001,
                    decay_pow: 2.4,
                    filter: FilterMode::BandPass,
                    fc0: 1900.0 * mul,
                    fc1: 700.0 * mul,
                    q: 1.5,
                    seed: base_seed.wrapping_add(i as u32 * 97),
                    ..Default::default()
                });
            }
            m.finish(0.35, 1.45, 1.2, 8.0)
        }

        // 16. SqueakNova: ear-friendly ultrasonic rat-chirp + warm radial shockwave!
        Sfx::ShootNova => {
            const PITCH: [f32; 3] = [1.0, 1.07, 0.94];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.175);
            // Thrilling rat squeak arc (bright yet ear-friendly, centroid ~2.3 kHz)
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.11,
                c0: 1480.0 * p,
                c1: 2380.0 * p,
                pitch_pow: 0.42,
                mod_ratio: 1.5,
                mod_index0: 1.35,
                mod_index1: 0.15,
                mod_pow: 2.2,
                gain: 0.40,
                attack_s: 0.002,
                decay_pow: 2.0,
                filter: FilterMode::BandPass,
                fc0: 2000.0,
                fc1: 2500.0,
                q: 1.3,
            });
            // Warm radial shockwave pulse underneath
            m.add_osc(OscLayer {
                start_s: 0.002,
                dur_s: 0.155,
                f0: 440.0 * p,
                f1: 112.0 * p,
                pitch_pow: 0.48,
                wave: Wave::Triangle,
                gain: 0.72,
                attack_s: 0.0025,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.115,
                pink: true,
                gain: 0.24,
                attack_s: 0.003,
                decay_pow: 2.1,
                filter: FilterMode::BandPass,
                fc0: 2800.0,
                fc1: 1100.0,
                q: 1.2,
                seed: base_seed,
                ..Default::default()
            });
            m.add_short_tail(0.15, 2800.0);
            m.finish(0.42, 1.35, 1.5, 14.0)
        }

        // 17. SporeOrbit: ethereal bubbly fungal ring deployment hum.
        Sfx::ShootOrbit => {
            const PITCH: [f32; 3] = [1.0, 1.07, 0.94];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.165);
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.15,
                c0: 460.0 * p,
                c1: 820.0 * p,
                pitch_pow: 0.7,
                mod_ratio: 2.5,
                mod_index0: 1.8,
                mod_index1: 0.15,
                mod_pow: 2.0,
                gain: 0.48,
                attack_s: 0.008,
                decay_pow: 1.7,
                filter: FilterMode::LowPass,
                fc0: 3400.0,
                fc1: 1500.0,
                q: 1.4,
            });
            m.add_osc(OscLayer {
                start_s: 0.012,
                dur_s: 0.14,
                f0: 690.0 * p,
                f1: 1230.0 * p,
                pitch_pow: 0.75,
                wave: Wave::Sine,
                gain: 0.34,
                attack_s: 0.010,
                decay_pow: 1.8,
                vib_hz: 18.0,
                vib_depth: 0.025,
                ..Default::default()
            });
            m.finish(0.34, 1.15, 2.0, 14.0)
        }

        // 18. TailWhip: meaty aerodynamic swoosh culminating in a punchy whip-crack!
        Sfx::ShootWhip => {
            const PITCH: [f32; 4] = [1.0, 1.08, 0.93, 1.04];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.125);
            // Leading swoosh (fattened so you hear the wind-up before the snap)
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.058,
                pink: true,
                gain: 0.62,
                attack_s: 0.010,
                decay_pow: 0.9,
                filter: FilterMode::BandPass,
                fc0: 480.0 * p,
                fc1: 2400.0 * p,
                fc_pow: 1.15,
                q: 2.0,
                seed: base_seed,
                ..Default::default()
            });
            // Sharp whip-crack snap at 34ms (balanced mid-high crack, not shrill)
            m.add_noise(NoiseLayer {
                start_s: 0.034,
                dur_s: 0.044,
                pink: false,
                gain: 0.68,
                attack_s: 0.0008,
                decay_pow: 2.6,
                filter: FilterMode::BandPass,
                fc0: 2600.0 * p,
                fc1: 850.0 * p,
                q: 1.4,
                seed: base_seed ^ 0x33CC,
                ..Default::default()
            });
            // Meaty whip-snap body drop
            m.add_osc(OscLayer {
                start_s: 0.034,
                dur_s: 0.078,
                f0: 680.0 * p,
                f1: 155.0 * p,
                pitch_pow: 0.40,
                wave: Wave::Saw,
                gain: 0.62,
                attack_s: 0.001,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 2800.0,
                fc1: 580.0,
                q: 1.4,
                ..Default::default()
            });
            m.finish(0.43, 1.65, 1.5, 10.0)
        }

        // 19. AcidSpit: resonant wet bubbling squelch with distinct pop-bubbles!
        Sfx::ShootAcid => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.135);
            // High-Q resonant wet squelch noise
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.115,
                pink: true,
                gain: 0.44,
                attack_s: 0.004,
                decay_pow: 1.8,
                filter: FilterMode::BandPass,
                fc0: 520.0 * p,
                fc1: 2200.0 * p,
                fc_pow: 0.65,
                q: 4.8,
                trem_hz: 36.0,
                trem_depth: 0.45,
                seed: base_seed,
            });
            // Three staggered rising liquid bubble pops
            for &(st, f0, f1, g) in &[
                (0.000f32, 240.0f32, 660.0f32, 0.52f32),
                (0.030, 320.0, 840.0, 0.44),
                (0.062, 410.0, 990.0, 0.34),
            ] {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.044,
                    f0: f0 * p,
                    f1: f1 * p,
                    pitch_pow: 0.65,
                    wave: Wave::Sine,
                    gain: g,
                    attack_s: 0.003,
                    decay_pow: 1.8,
                    ..Default::default()
                });
            }
            m.finish(0.37, 1.35, 1.5, 12.0)
        }

        // 20. Heal: warm restorative major chord bloom with crystalline shimmer top.
        Sfx::Heal => {
            let root = if v == 0 { 587.33 } else { 659.25 }; // D5 or E5
            let ratios = [1.0f32, 1.25, 1.5, 2.0];
            let mut m = MixBuf::new(0.34);
            for (i, &r) in ratios.iter().enumerate() {
                let st = i as f32 * 0.036;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.24 - i as f32 * 0.015,
                    f0: root * r,
                    f1: root * r * 1.006,
                    wave: Wave::Sine,
                    gain: 0.36,
                    attack_s: 0.012,
                    decay_pow: 1.7,
                    vib_hz: 5.0,
                    vib_depth: 0.006,
                    ..Default::default()
                });
                if i >= 2 {
                    m.add_fm(FmLayer {
                        start_s: st,
                        dur_s: 0.16,
                        c0: root * r * 2.0,
                        c1: root * r * 2.0,
                        mod_ratio: 2.0,
                        mod_index0: 0.9,
                        mod_index1: 0.05,
                        mod_pow: 2.6,
                        gain: 0.16,
                        attack_s: 0.006,
                        decay_pow: 2.2,
                        filter: FilterMode::LowPass,
                        fc0: 5400.0,
                        fc1: 2200.0,
                        q: 0.8,
                        ..Default::default()
                    });
                }
            }
            m.add_short_tail(0.18, 3600.0);
            m.finish(0.48, 1.15, 2.0, 22.0)
        }

        // 21. Magnet: rising resonant magnetic vacuum sweep + lock-in chime!
        Sfx::Magnet => {
            let p = if v == 0 { 1.0 } else { 1.06 };
            let mut m = MixBuf::new(0.35);
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.31,
                c0: 210.0 * p,
                c1: 1580.0 * p,
                pitch_pow: 1.2,
                mod_ratio: 1.414,
                mod_index0: 2.2,
                mod_index1: 0.5,
                mod_pow: 1.4,
                gain: 0.54,
                attack_s: 0.014,
                decay_pow: 1.25,
                filter: FilterMode::BandPass,
                fc0: 380.0,
                fc1: 3200.0,
                q: 2.3,
            });
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.29,
                pink: true,
                gain: 0.36,
                attack_s: 0.018,
                decay_pow: 1.4,
                filter: FilterMode::BandPass,
                fc0: 440.0 * p,
                fc1: 3400.0 * p,
                fc_pow: 1.15,
                q: 2.8,
                seed: base_seed,
                ..Default::default()
            });
            // Crisp magnetic lock-in ping near apex
            m.add_osc(OscLayer {
                start_s: 0.21,
                dur_s: 0.12,
                f0: 1580.0 * p,
                f1: 1620.0 * p,
                wave: Wave::Sine,
                gain: 0.34,
                attack_s: 0.004,
                decay_pow: 2.1,
                ..Default::default()
            });
            m.add_short_tail(0.16, 3200.0);
            m.finish(0.48, 1.35, 2.0, 20.0)
        }

        // 22. Nuke: bright flash-inhale -> colossal sub-drop -> roaring shockwave!
        Sfx::Nuke => {
            let p = if v == 0 { 1.0 } else { 0.94 };
            let mut m = MixBuf::new(0.78);
            // Distinct 38ms flash-ping lead-in ("shing—BOOM!")
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.048,
                c0: 980.0 * p,
                c1: 2400.0 * p,
                pitch_pow: 0.7,
                mod_ratio: 2.0,
                mod_index0: 2.2,
                mod_index1: 0.4,
                mod_pow: 1.8,
                gain: 0.62,
                attack_s: 0.002,
                decay_pow: 1.6,
                ..Default::default()
            });
            // Massive seismic sub-drop hitting right after flash
            m.add_osc(OscLayer {
                start_s: 0.032,
                dur_s: 0.54,
                f0: 170.0 * p,
                f1: 26.0 * p,
                pitch_pow: 0.36,
                wave: Wave::Sine,
                gain: 0.98,
                attack_s: 0.003,
                decay_pow: 1.45,
                ..Default::default()
            });
            // Wide roaring shockwave
            m.add_noise(NoiseLayer {
                start_s: 0.032,
                dur_s: 0.72,
                pink: true,
                gain: 0.90,
                attack_s: 0.004,
                decay_pow: 1.55,
                filter: FilterMode::LowPass,
                fc0: 6200.0,
                fc1: 115.0,
                fc_pow: 0.48,
                q: 1.5,
                seed: base_seed,
                ..Default::default()
            });
            m.add_short_tail(0.24, 1600.0);
            m.finish(0.82, 2.0, 1.5, 36.0)
        }

        // 23. BossDeath: escalating 3-stage chain blast + collapsing roar + release chord.
        Sfx::BossDeath => {
            let mut m = MixBuf::new(1.12);
            for (i, &(st, sub_f, scale)) in [
                (0.00f32, 125.0f32, 0.56f32),
                (0.16, 155.0, 0.76),
                (0.34, 190.0, 1.00),
            ]
            .iter()
            .enumerate()
            {
                let big = i == 2;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: if big { 0.54 } else { 0.24 },
                    f0: sub_f,
                    f1: 28.0,
                    pitch_pow: 0.40,
                    wave: Wave::Sine,
                    gain: 0.90 * scale,
                    attack_s: 0.002,
                    decay_pow: 1.45,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: if big { 0.70 } else { 0.24 },
                    pink: true,
                    gain: 0.82 * scale,
                    attack_s: 0.003,
                    decay_pow: 1.55,
                    filter: FilterMode::LowPass,
                    fc0: if big { 5400.0 } else { 3000.0 + i as f32 * 800.0 },
                    fc1: 130.0,
                    fc_pow: 0.52,
                    q: 1.4,
                    seed: base_seed.wrapping_add(i as u32 * 4099),
                    ..Default::default()
                });
            }
            // Collapsing monster synth roar bridging all three blasts
            m.add_osc(OscLayer {
                start_s: 0.02,
                dur_s: 0.68,
                f0: 230.0,
                f1: 42.0,
                pitch_pow: 0.80,
                wave: Wave::Saw,
                gain: 0.44,
                attack_s: 0.02,
                decay_pow: 1.4,
                filter: FilterMode::LowPass,
                fc0: 1600.0,
                fc1: 180.0,
                q: 2.0,
                ..Default::default()
            });
            // Triumphant shimmering release chord blooming clearly as the smoke clears
            for &(f, g) in &[
                (523.25f32, 0.34f32),
                (659.25, 0.28),
                (783.99, 0.32),
                (1046.50, 0.36),
            ] {
                m.add_fm(FmLayer {
                    start_s: 0.50,
                    dur_s: 0.60,
                    c0: f,
                    c1: f,
                    mod_ratio: 2.0,
                    mod_index0: 1.25,
                    mod_index1: 0.05,
                    mod_pow: 2.2,
                    gain: g,
                    attack_s: 0.022,
                    decay_pow: 1.4,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.24, 2600.0);
            m.finish(0.82, 1.85, 2.0, 48.0)
        }
    }
}

// ---- Music ---------------------------------------------------------------

const PATTERN_LEN: usize = 64; // 4 bars of 16th notes
const REST: i32 = -127;
const KICK: u8 = 1;
const SNARE: u8 = 2;
const HAT: u8 = 4;

struct TrackCfg {
    root: f32,
    scale: &'static [i32],
    bpm: f32,
    lead_duty: f32,
    lead_lp: f32,
    bass: [i32; PATTERN_LEN],
    arp: [i32; PATTERN_LEN],
    lead: [i32; PATTERN_LEN],
    drums: [u8; PATTERN_LEN],
}

fn track_for(theme: Theme) -> TrackCfg {
    // 64-step (4-bar) scale-degree patterns; REST is a rest.
    match theme {
        Theme::Sewer => TrackCfg {
            root: 130.81, // C3 natural minor — subterranean stealth & grit
            scale: &[0, 2, 3, 5, 7, 8, 10],
            bpm: 102.0,
            lead_duty: 0.28,
            lead_lp: 2200.0,
            bass: [
                // Bar 1 (i)
                0, REST, 0, REST, 0, REST, 4, REST, 0, REST, 2, REST, 3, REST, 4, REST,
                // Bar 2 (VI)
                5, REST, 5, REST, 5, REST, 2, REST, 5, REST, 4, REST, 3, REST, 2, REST,
                // Bar 3 (iv)
                3, REST, 3, REST, 3, REST, 0, REST, 3, REST, 4, REST, 5, REST, 3, REST,
                // Bar 4 (V -> i turnaround)
                4, REST, 4, REST, 6, REST, 4, REST, 4, REST, 2, REST, 1, REST, 4, REST,
            ],
            arp: [
                0, 4, 7, 4, 0, 4, 7, 4, 0, 4, 7, 4, 2, 4, 7, 4,
                5, 7, 9, 7, 5, 7, 9, 7, 5, 7, 9, 7, 4, 7, 9, 7,
                3, 5, 7, 5, 3, 5, 7, 5, 3, 5, 7, 5, 3, 5, 8, 5,
                4, 6, 8, 6, 4, 6, 8, 6, 4, 7, 8, 7, 4, 6, 4, 2,
            ],
            lead: [
                0, REST, 2, 3, 4, REST, 3, 2, 0, REST, REST, 2, 3, REST, 2, 0,
                5, REST, 4, 3, 2, REST, 3, 4, 5, REST, REST, 4, 3, REST, 2, REST,
                3, REST, 5, 4, 3, REST, 2, 0, 3, REST, 4, 5, 7, REST, 5, 4,
                4, REST, 6, 7, 8, REST, 7, 6, 4, REST, 3, 2, 1, REST, 2, REST,
            ],
            drums: [
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, KICK, SNARE, 0, HAT, 0,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, HAT, HAT,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, KICK, SNARE, 0, HAT, 0,
                KICK, 0, HAT, 0, SNARE, 0, HAT, KICK, SNARE, 0, HAT, KICK, SNARE, 0, SNARE | HAT, 0,
            ],
        },
        Theme::Kitchen => TrackCfg {
            root: 146.83, // D3 Mixolydian/major — frantic playful kitchen chase
            scale: &[0, 2, 4, 5, 7, 9, 10],
            bpm: 126.0,
            lead_duty: 0.34,
            lead_lp: 2800.0,
            bass: [
                // Bar 1 (I)
                0, REST, 4, REST, 0, REST, 4, 5, 0, REST, 4, REST, 2, REST, 4, REST,
                // Bar 2 (IV)
                3, REST, 5, REST, 3, REST, 5, 6, 3, REST, 5, REST, 4, REST, 3, REST,
                // Bar 3 (bVII -> IV)
                6, REST, 4, REST, 6, REST, 4, REST, 3, REST, 5, REST, 3, REST, 2, REST,
                // Bar 4 (V -> I hustle)
                4, REST, 4, 5, 6, REST, 4, REST, 4, REST, 2, REST, 0, REST, 4, 5,
            ],
            arp: [
                0, 2, 4, 7, 0, 2, 4, 7, 0, 2, 4, 7, 0, 4, 7, 4,
                3, 5, 7, 9, 3, 5, 7, 9, 3, 5, 7, 9, 3, 5, 7, 5,
                6, 8, 10, 8, 6, 8, 10, 8, 3, 5, 7, 5, 3, 5, 7, 5,
                4, 6, 8, 6, 4, 6, 8, 11, 4, 6, 8, 6, 4, 2, 4, 6,
            ],
            lead: [
                4, 2, 0, 2, 4, REST, 7, 4, 2, 4, 5, 4, 2, REST, 0, REST,
                5, 4, 3, 4, 5, REST, 7, 5, 4, 3, 2, 3, 4, REST, 2, REST,
                6, REST, 4, 6, 7, REST, 6, 4, 5, REST, 3, 5, 7, REST, 5, 4,
                4, 5, 6, 7, 9, REST, 7, 6, 7, REST, 4, 2, 4, REST, 7, REST,
            ],
            drums: [
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, HAT, KICK, 0, SNARE, 0, HAT, 0,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, HAT, KICK, 0, SNARE, 0, HAT, HAT,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, HAT, KICK, 0, SNARE, 0, HAT, 0,
                KICK, 0, HAT, KICK, SNARE, 0, HAT, KICK, SNARE, 0, KICK, 0, SNARE, HAT, SNARE, 0,
            ],
        },
        Theme::Lab => TrackCfg {
            root: 123.47, // B2 Phrygian Dominant — eerie synthwave lab
            scale: &[0, 1, 4, 5, 7, 8, 10],
            bpm: 114.0,
            lead_duty: 0.22,
            lead_lp: 2500.0,
            bass: [
                // Bar 1 (i driving 16ths/octaves)
                0, REST, 0, 7, 0, REST, 0, REST, 0, REST, 0, 7, 1, REST, 0, REST,
                // Bar 2 (bII eerie shift)
                1, REST, 1, 8, 1, REST, 1, REST, 1, REST, 0, REST, 6, REST, 4, REST,
                // Bar 3 (iv -> bVI)
                3, REST, 3, 7, 3, REST, 3, REST, 5, REST, 5, 8, 5, REST, 4, REST,
                // Bar 4 (bII -> i tension)
                1, REST, 4, REST, 5, REST, 4, REST, 1, REST, 1, 0, 6, REST, 1, REST,
            ],
            arp: [
                0, 4, 7, 4, 0, 4, 7, 8, 0, 4, 7, 4, 1, 4, 7, 4,
                1, 5, 8, 5, 1, 5, 8, 7, 1, 5, 8, 5, 0, 4, 7, 4,
                3, 5, 7, 10, 3, 5, 7, 5, 5, 7, 8, 10, 5, 7, 8, 7,
                1, 4, 8, 4, 1, 4, 8, 7, 1, 4, 7, 4, 0, 1, 4, 7,
            ],
            lead: [
                7, REST, 6, 5, 4, REST, 5, 4, 1, REST, 0, REST, 4, REST, 5, REST,
                8, REST, 7, 5, 4, REST, 1, 4, 5, REST, 4, 1, 0, REST, REST, REST,
                7, REST, 8, 7, 5, REST, 4, 5, 8, REST, 10, 8, 7, REST, 5, 4,
                5, REST, 4, 1, 4, REST, 5, 7, 8, REST, 7, 5, 4, REST, 1, REST,
            ],
            drums: [
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, HAT, KICK,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, KICK, 0, SNARE, 0, HAT, 0,
                KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, HAT, KICK,
                KICK, 0, HAT, KICK, SNARE, 0, HAT, 0, KICK, HAT, SNARE, 0, KICK, 0, SNARE, HAT,
            ],
        },
    }
}

fn degree_freq(root: f32, scale: &[i32], degree: i32) -> Option<f32> {
    if degree == REST {
        return None;
    }
    let len = scale.len() as i32;
    let oct = degree.div_euclid(len);
    let idx = degree.rem_euclid(len) as usize;
    let semis = scale[idx] + 12 * oct;
    Some(root * 2f32.powf(semis as f32 / 12.0))
}

/// An infinite themed chiptune source. `intensity` (0..=2) smoothly crossfades
/// the harmony/lead/drum layers so the music swells with the action without pops.
pub struct MusicSynth {
    cfg: TrackCfg,
    intensity: Arc<AtomicU32>,
    counter: u64,
    samples_per_step: u64,
    bass_phase: f32,
    bass_env: f32,
    bass_attack_rem: u16,
    pad_phase: f32,
    arp_phase: f32,
    lead_phase: f32,
    kick_phase: f32,
    kick_freq: f32,
    kick_env: f32,
    snare_env: f32,
    hat_env: f32,
    lead_lp_state: f32,
    lead_lp_alpha: f32,
    noise_hp_prev_x: f32,
    noise_hp_prev_y: f32,
    lead_mix: f32,
    drum_mix: f32,
    seed: u32,
    /// Step whose note frequencies are cached below (`u64::MAX` = none yet).
    cached_step: u64,
    bass_freq: Option<f32>,
    bass_active_freq: f32,
    arp_freq: Option<f32>,
    lead_freq: Option<f32>,
}

impl MusicSynth {
    pub fn new(theme: Theme, intensity: Arc<AtomicU32>) -> Self {
        let cfg = track_for(theme);
        let samples_per_step = (SR_F * 60.0 / (cfg.bpm * 4.0)) as u64;
        let lead_lp_alpha = (TAU * cfg.lead_lp / SR_F).clamp(0.05, 0.85);
        let init_int = intensity.load(Ordering::Relaxed);
        let root_half = cfg.root * 0.5;
        MusicSynth {
            cfg,
            intensity,
            counter: 0,
            samples_per_step: samples_per_step.max(1),
            bass_phase: 0.0,
            bass_env: 0.0,
            bass_attack_rem: 0,
            pad_phase: 0.0,
            arp_phase: 0.0,
            lead_phase: 0.0,
            kick_phase: 0.0,
            kick_freq: 46.0,
            kick_env: 0.0,
            snare_env: 0.0,
            hat_env: 0.0,
            lead_lp_state: 0.0,
            lead_lp_alpha,
            noise_hp_prev_x: 0.0,
            noise_hp_prev_y: 0.0,
            lead_mix: if init_int >= 1 { 1.0 } else { 0.0 },
            drum_mix: if init_int >= 2 { 1.0 } else { 0.0 },
            seed: 0x9E37_79B9,
            cached_step: u64::MAX,
            bass_freq: None,
            bass_active_freq: root_half,
            arp_freq: None,
            lead_freq: None,
        }
    }

    /// Refresh cached note frequencies and trigger step percussion at step boundaries.
    fn cache_step(&mut self, step_idx: u64) {
        let step = (step_idx % PATTERN_LEN as u64) as usize;
        self.cached_step = step_idx;
        self.bass_freq = degree_freq(self.cfg.root * 0.5, self.cfg.scale, self.cfg.bass[step]);
        if let Some(bf) = self.bass_freq {
            self.bass_active_freq = bf;
            self.bass_attack_rem = 96;
        }
        self.arp_freq = degree_freq(self.cfg.root, self.cfg.scale, self.cfg.arp[step]);
        self.lead_freq = degree_freq(self.cfg.root * 2.0, self.cfg.scale, self.cfg.lead[step]);

        let d = self.cfg.drums[step];
        if d & KICK != 0 {
            self.kick_phase = 0.0;
            self.kick_freq = 152.0;
            self.kick_env = 1.0;
        }
        if d & SNARE != 0 {
            self.snare_env = 1.0;
        }
        if d & HAT != 0 {
            self.hat_env = 1.0;
        }
    }

    /// Click-free step envelope: short linear attack (~2.5ms) and smooth squared decay to 0.
    #[inline]
    fn step_env(pos_samples: u64, sps: u64, release_curve: u8) -> f32 {
        let attack = (pos_samples as f32 / 110.0).min(1.0);
        let rem = (1.0 - pos_samples as f32 / sps as f32).max(0.0);
        let decay = match release_curve {
            1 => rem,
            2 => rem * rem,
            _ => rem * rem * rem,
        };
        // Smooth tiny fade-out on the last ~2ms of the step so boundaries never click.
        let tail = ((sps.saturating_sub(pos_samples)) as f32 / 96.0).min(1.0);
        attack * decay * tail
    }
}

impl Iterator for MusicSynth {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sps = self.samples_per_step;
        let step_idx = self.counter / sps;
        if self.cached_step != step_idx {
            self.cache_step(step_idx);
        }
        let pos_samples = self.counter % sps;

        // Smoothly crossfade layers when intensity changes (zero volume pops).
        let intensity = self.intensity.load(Ordering::Relaxed);
        let target_lead = if intensity >= 1 { 1.0 } else { 0.0 };
        let target_drum = if intensity >= 2 { 1.0 } else { 0.0 };
        self.lead_mix += (target_lead - self.lead_mix) * 0.000_18;
        self.drum_mix += (target_drum - self.drum_mix) * 0.000_18;

        let mut s = 0.0f32;

        // 0. Subtle atmospheric root drone pad (glues calm intensity-0 moments).
        self.pad_phase = (self.pad_phase + self.cfg.root / SR_F).fract();
        let pad_fade = (self.counter as f32 / 440.0).min(1.0);
        s += (self.pad_phase * TAU).sin() * 0.028 * pad_fade;

        // 1. Warm legato Bass (rings naturally across REST steps, click-free re-trigger).
        if self.bass_attack_rem > 0 {
            self.bass_env += (1.0 - self.bass_env) * 0.06;
            self.bass_attack_rem -= 1;
        } else {
            self.bass_env *= 0.999_84;
        }
        if self.bass_env > 0.0008 {
            let dt = self.bass_active_freq / SR_F;
            self.bass_phase = (self.bass_phase + dt).fract();
            let tri = 2.0 * (2.0 * self.bass_phase - 1.0).abs() - 1.0;
            let sub_sq = Wave::Pulse(0.5).sample(self.bass_phase, dt);
            s += (tri * 0.23 + sub_sq * 0.04) * self.bass_env;
        }

        // 2. Gentle background arpeggio glue (active at intensity >= 1).
        if self.lead_mix > 0.001 {
            if let Some(af) = self.arp_freq {
                let dt = af / SR_F;
                self.arp_phase = (self.arp_phase + dt).fract();
                let env = Self::step_env(pos_samples, sps, 2);
                let tri = 2.0 * (2.0 * self.arp_phase - 1.0).abs() - 1.0;
                s += tri * 0.062 * env * self.lead_mix;
            }
        }

        // 3. Band-limited, lowpass-warmed Lead (active at intensity >= 1).
        if self.lead_mix > 0.001 {
            if let Some(lf) = self.lead_freq {
                let dt = (lf / SR_F).clamp(0.0, 0.45);
                self.lead_phase = (self.lead_phase + dt).fract();
                let env = Self::step_env(pos_samples, sps, 2);
                let raw = Wave::Pulse(self.cfg.lead_duty).sample(self.lead_phase, dt);
                self.lead_lp_state += self.lead_lp_alpha * (raw - self.lead_lp_state);
                s += self.lead_lp_state * 0.145 * env * self.lead_mix;
            }
        }

        // 4. Punchy chiptune drums (kick enters at intensity >= 1, full driving kit at >= 2).
        let kick_gain = 0.10 * self.lead_mix + 0.16 * self.drum_mix;
        if kick_gain > 0.001 && self.kick_env > 0.0005 {
            self.kick_phase = (self.kick_phase + self.kick_freq / SR_F).fract();
            let k_sample = (self.kick_phase * TAU).sin();
            s += k_sample * self.kick_env * kick_gain;
            self.kick_freq = 44.0 + (self.kick_freq - 44.0) * 0.9964;
            self.kick_env *= 0.9990;
        }

        if self.drum_mix > 0.001 && (self.snare_env > 0.0005 || self.hat_env > 0.0005) {
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = ((self.seed >> 9) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0;
            // Crisp 1-pole highpass keeps snare/hat air sparkling above the bass/lead.
            let hp = 0.72 * (self.noise_hp_prev_y + white - self.noise_hp_prev_x);
            self.noise_hp_prev_x = white;
            self.noise_hp_prev_y = hp;

            if self.snare_env > 0.0005 {
                let sn = (hp * 0.65 + white * 0.35) * self.snare_env * 0.125 * self.drum_mix;
                s += sn;
                self.snare_env *= 0.9986;
            }
            if self.hat_env > 0.0005 {
                s += hp * self.hat_env * 0.082 * self.drum_mix;
                self.hat_env *= 0.9968;
            }
        }

        self.counter += 1;
        // Soft bus ceiling so SFX always have crisp headroom over the music.
        Some((s * 0.85).tanh().clamp(-1.0, 1.0))
    }
}

impl Source for MusicSynth {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::<u16>::MIN
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(SR).expect("SR is non-zero")
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// ---- Engine --------------------------------------------------------------

/// Level for effects; music has its own `music_vol`.
const SFX_VOL: f32 = 0.85;
/// Effects play on pooled sinks so they can overlap and so mute can reach
/// the ones already sounding. Cap the live voices so a kill burst can never
/// spawn unbounded sinks/mixer tracks.
const MAX_SFX_VOICES: usize = 24;
/// Window in milliseconds during which consecutive gem pickups climb the scale.
const GEM_COMBO_WINDOW_MS: u64 = 480;

/// Master volume for a channel: muting silences music *and* effects.
fn channel_vol(muted: bool, vol: f32) -> f32 {
    if muted {
        0.0
    } else {
        vol
    }
}

/// Lock-free variant rotation & gem combo pitch-ladder selector.
struct VariantPicker {
    counters: Vec<AtomicUsize>,
    last_gem_ms: AtomicU64,
    gem_combo: AtomicUsize,
}

impl VariantPicker {
    fn new() -> Self {
        let counters = (0..Sfx::ALL.len()).map(|_| AtomicUsize::new(0)).collect();
        Self {
            counters,
            last_gem_ms: AtomicU64::new(0),
            gem_combo: AtomicUsize::new(0),
        }
    }

    /// Pick the cached variant index for `sfx` at `now_ms` (monotonic ms >= 1).
    fn pick(&self, sfx: Sfx, now_ms: u64) -> usize {
        let count = sfx.variant_count();
        if count <= 1 {
            return 0;
        }
        if sfx == Sfx::Gem {
            let prev = self.last_gem_ms.swap(now_ms, Ordering::Relaxed);
            if prev > 0 && now_ms.saturating_sub(prev) <= GEM_COMBO_WINDOW_MS {
                let step = self.gem_combo.fetch_add(1, Ordering::Relaxed) + 1;
                if step < count {
                    return step;
                }
                // Sustain near the shimmering top of the pentatonic ladder.
                return count - 3 + (step % 3);
            } else {
                self.gem_combo.store(0, Ordering::Relaxed);
                return 0;
            }
        }
        let n = self.counters[sfx.index()].fetch_add(1, Ordering::Relaxed);
        n % count
    }
}

/// A cached mono sample buffer replayed as a rodio `Source`. Creating one costs
/// an `Arc` clone: no re-synthesis and no copy of the samples.
struct CachedSfx {
    data: Arc<[f32]>,
    pos: f32,
    step: f32,
}

impl CachedSfx {
    fn new(data: Arc<[f32]>) -> Self {
        CachedSfx {
            data,
            pos: 0.0,
            step: 1.0,
        }
    }

    fn with_pitch(data: Arc<[f32]>, pitch_ratio: f32) -> Self {
        CachedSfx {
            data,
            pos: 0.0,
            step: pitch_ratio.clamp(0.5, 2.0),
        }
    }
}

impl Iterator for CachedSfx {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let idx = self.pos as usize;
        if idx >= self.data.len() {
            return None;
        }
        let sample = if (self.step - 1.0).abs() < 1e-6 {
            self.data[idx]
        } else {
            let frac = self.pos - idx as f32;
            let s0 = self.data[idx];
            let s1 = self.data.get(idx + 1).copied().unwrap_or(0.0);
            s0 + (s1 - s0) * frac
        };
        self.pos += self.step;
        Some(sample)
    }
}

impl Source for CachedSfx {
    fn current_span_len(&self) -> Option<usize> {
        let rem = self.data.len() as f32 - self.pos;
        if rem <= 0.0 {
            Some(0)
        } else {
            Some((rem / self.step).ceil() as usize)
        }
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::<u16>::MIN
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(SR).expect("SR is non-zero")
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.data.len() as f64 / (SR as f64 * self.step as f64),
        ))
    }
}

struct VoiceSlot {
    player: Player,
    priority: u8,
}

pub struct AudioEngine {
    stream: MixerDeviceSink,
    music: Player,
    /// Pooled sinks for in-flight effects: idle slots are reused without
    /// allocating new mixer tracks, and high-priority alerts can steal from
    /// low-priority chatter when saturated.
    sfx_voices: Mutex<Vec<VoiceSlot>>,
    /// Pre-synthesized variant banks for every `Sfx` (indexed by `Sfx::index`).
    cache: Vec<Vec<Arc<[f32]>>>,
    picker: VariantPicker,
    started_at: Instant,
    intensity: Arc<AtomicU32>,
    muted: bool,
    music_vol: f32,
}

impl AudioEngine {
    /// Returns `None` if no output device is available (stays silent).
    pub fn new() -> Option<AudioEngine> {
        let mut stream = DeviceSinkBuilder::open_default_sink().ok()?;
        // rodio prints to stderr when the device sink drops, which would land
        // on the restored terminal at exit.
        stream.log_on_drop(false);
        let music = Player::connect_new(stream.mixer());
        music.set_volume(0.0);
        Some(AudioEngine {
            stream,
            music,
            sfx_voices: Mutex::new(Vec::with_capacity(MAX_SFX_VOICES)),
            cache: Self::build_cache(),
            picker: VariantPicker::new(),
            started_at: Instant::now(),
            intensity: Arc::new(AtomicU32::new(1)),
            muted: false,
            music_vol: 0.50,
        })
    }

    /// Synthesize every effect and its variants exactly once, at startup.
    fn build_cache() -> Vec<Vec<Arc<[f32]>>> {
        Sfx::ALL
            .iter()
            .map(|&sfx| {
                (0..sfx.variant_count())
                    .map(|v| Arc::from(sfx_variant_samples(sfx, v).into_boxed_slice()))
                    .collect()
            })
            .collect()
    }

    pub fn play_music(&mut self, theme: Theme, intensity: u32) {
        self.intensity.store(intensity, Ordering::Relaxed);
        self.music.clear();
        let synth = MusicSynth::new(theme, self.intensity.clone());
        self.music.append(synth);
        self.apply_volumes();
        self.music.play();
    }

    pub fn set_intensity(&self, level: u32) {
        self.intensity.store(level, Ordering::Relaxed);
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        // Reach the sinks that are already playing, not just future sounds.
        self.apply_volumes();
        self.muted
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Push the current mute state to every live sink (music and effects).
    fn apply_volumes(&self) {
        self.music
            .set_volume(channel_vol(self.muted, self.music_vol));
        let Ok(voices) = self.sfx_voices.lock() else {
            return;
        };
        for v in voices.iter() {
            v.player.set_volume(channel_vol(self.muted, SFX_VOL));
        }
    }

    pub fn play_sfx(&self, sfx: Sfx) {
        self.play_sfx_pitched(sfx, 1.0);
    }

    /// Play `sfx` with an optional playback pitch multiplier in `[0.5, 2.0]`.
    pub fn play_sfx_pitched(&self, sfx: Sfx, pitch_ratio: f32) {
        let Some(bank) = self.cache.get(sfx.index()) else {
            return;
        };
        if bank.is_empty() {
            return;
        }
        let now_ms = self.started_at.elapsed().as_millis() as u64 + 1;
        let v_idx = self.picker.pick(sfx, now_ms) % bank.len();
        let samples = &bank[v_idx];
        if samples.is_empty() {
            return;
        }

        let Ok(mut voices) = self.sfx_voices.lock() else {
            return;
        };
        let vol = channel_vol(self.muted, SFX_VOL);
        let src = if (pitch_ratio - 1.0).abs() < 1e-4 {
            CachedSfx::new(Arc::clone(samples))
        } else {
            CachedSfx::with_pitch(Arc::clone(samples), pitch_ratio)
        };
        let prio = sfx.priority();

        // 1. Reuse an idle voice slot (zero new Player allocation in steady state).
        if let Some(slot) = voices.iter_mut().find(|v| v.player.empty()) {
            slot.priority = prio;
            slot.player.set_volume(vol);
            slot.player.append(src);
            slot.player.play();
            return;
        }

        // 2. Grow pool up to MAX_SFX_VOICES.
        if voices.len() < MAX_SFX_VOICES {
            let player = Player::connect_new(self.stream.mixer());
            player.set_volume(vol);
            player.append(src);
            player.play();
            voices.push(VoiceSlot {
                player,
                priority: prio,
            });
            return;
        }

        // 3. Pool saturated: steal the lowest-priority voice only if strictly lower.
        if let Some((idx, lowest)) = voices
            .iter()
            .enumerate()
            .min_by_key(|(_, slot)| slot.priority)
        {
            if prio > lowest.priority {
                let slot = &mut voices[idx];
                slot.player.clear();
                slot.priority = prio;
                slot.player.set_volume(vol);
                slot.player.append(src);
                slot.player.play();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f32 = samples.iter().map(|&x| x * x).sum();
        (sum / samples.len() as f32).sqrt()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    fn zero_crossing_rate_hz(samples: &[f32]) -> f32 {
        if samples.len() < 2 {
            return 0.0;
        }
        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f32 * SR_F / (2.0 * samples.len() as f32)
    }

    #[test]
    fn every_sfx_and_variant_is_finite_and_bounded() {
        for sfx in Sfx::ALL {
            for v in 0..sfx.variant_count() {
                let s = sfx_variant_samples(sfx, v);
                assert!(!s.is_empty(), "{sfx:?} v{v} produced no samples");
                assert!(
                    s.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
                    "{sfx:?} v{v} out of [-1, 1] or non-finite"
                );
            }
        }
    }

    #[test]
    fn every_sfx_has_click_free_start_and_end_and_no_dc_bias() {
        for sfx in Sfx::ALL {
            for v in 0..sfx.variant_count() {
                let s = sfx_variant_samples(sfx, v);
                let pk = peak(&s);
                assert!(pk > 0.15, "{sfx:?} v{v} too quiet: peak={pk}");
                // Exact zero endpoints prevent boundary step clicks.
                assert_eq!(s[0], 0.0, "{sfx:?} v{v} non-zero first sample");
                assert_eq!(s[s.len() - 1], 0.0, "{sfx:?} v{v} non-zero last sample");
                // First 0.5ms and last 2ms stay gently ramped.
                let start_max = peak(&s[..16.min(s.len())]);
                let end_max = peak(&s[s.len().saturating_sub(64)..]);
                assert!(
                    start_max < pk * 0.45,
                    "{sfx:?} v{v} abrupt onset: start_max={start_max} pk={pk}"
                );
                assert!(
                    end_max < pk * 0.25,
                    "{sfx:?} v{v} abrupt tail cutoff: end_max={end_max} pk={pk}"
                );
                // Negligible DC offset.
                let dc = (s.iter().sum::<f32>() / s.len() as f32).abs();
                assert!(dc < 0.015, "{sfx:?} v{v} DC offset too large: {dc}");
            }
        }
    }

    #[test]
    fn sfx_durations_rms_and_mix_hierarchy_are_sane() {
        for sfx in Sfx::ALL {
            let s = sfx_samples(sfx);
            let dur_ms = s.len() as f32 * 1000.0 / SR_F;
            let pk = peak(&s);
            let r = rms(&s);
            assert!(
                (25.0..=1200.0).contains(&dur_ms),
                "{sfx:?} duration out of sane bounds: {dur_ms:.1}ms"
            );
            assert!(
                r >= 0.025 && r <= pk,
                "{sfx:?} RMS out of sane bounds: rms={r:.4} peak={pk:.4}"
            );
        }
        // Combat chatter stays below big impacts in peak level.
        assert!(peak(&sfx_samples(Sfx::Shoot)) < peak(&sfx_samples(Sfx::Explosion)));
        assert!(peak(&sfx_samples(Sfx::Hit)) < peak(&sfx_samples(Sfx::Hurt)));
        assert!(peak(&sfx_samples(Sfx::Gem)) < peak(&sfx_samples(Sfx::LevelUp)));
        assert!(peak(&sfx_samples(Sfx::UiMove)) < peak(&sfx_samples(Sfx::UiSelect)));
        assert!(peak(&sfx_samples(Sfx::Explosion)) <= peak(&sfx_samples(Sfx::Nuke)));
    }

    #[test]
    fn spectral_zero_crossing_sanity_across_roles() {
        // High crystalline gem should have a higher ZCR than sub-heavy explosion/boss.
        let z_gem = zero_crossing_rate_hz(&sfx_samples(Sfx::Gem));
        let z_exp = zero_crossing_rate_hz(&sfx_samples(Sfx::Explosion));
        let z_boss = zero_crossing_rate_hz(&sfx_samples(Sfx::Boss));
        let z_hit = zero_crossing_rate_hz(&sfx_samples(Sfx::Hit));
        let z_nova = zero_crossing_rate_hz(&sfx_samples(Sfx::ShootNova));
        assert!(
            z_gem > z_exp * 1.5,
            "Gem ({z_gem:.0}Hz) should be brighter than Explosion ({z_exp:.0}Hz)"
        );
        assert!(
            z_gem > z_boss * 1.5,
            "Gem ({z_gem:.0}Hz) should be brighter than Boss ({z_boss:.0}Hz)"
        );
        assert!(
            z_nova > z_hit,
            "SqueakNova ({z_nova:.0}Hz) should be brighter than Hit ({z_hit:.0}Hz)"
        );
        // Gem combo ladder rises in pitch from step 0 to step 7.
        let z_gem0 = zero_crossing_rate_hz(&sfx_variant_samples(Sfx::Gem, 0));
        let z_gem7 = zero_crossing_rate_hz(&sfx_variant_samples(Sfx::Gem, 7));
        assert!(
            z_gem7 > z_gem0 * 2.0,
            "Gem ladder should climb >1 octave: v0={z_gem0:.0}Hz v7={z_gem7:.0}Hz"
        );
    }

    #[test]
    fn cached_variants_meaningfully_differ() {
        for sfx in Sfx::ALL {
            let count = sfx.variant_count();
            if count < 2 {
                continue;
            }
            let vars: Vec<Vec<f32>> = (0..count).map(|v| sfx_variant_samples(sfx, v)).collect();
            for i in 0..count {
                for j in (i + 1)..count {
                    let len = vars[i].len().min(vars[j].len());
                    let diff_rms = (vars[i][..len]
                        .iter()
                        .zip(&vars[j][..len])
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f32>()
                        / len as f32)
                        .sqrt();
                    assert!(
                        diff_rms > 0.015,
                        "{sfx:?} variants {i} and {j} are too similar (diff_rms={diff_rms:.4})"
                    );
                }
            }
        }
    }

    #[test]
    fn variant_picker_rotates_and_climbs_gem_combo() {
        let picker = VariantPicker::new();
        // Round-robin rotation for Shoot (4 variants)
        let seq: Vec<usize> = (0..6).map(|t| picker.pick(Sfx::Shoot, 100 + t * 50)).collect();
        assert_eq!(seq, vec![0, 1, 2, 3, 0, 1]);

        // Gem combo climbs within window, resets after gap > GEM_COMBO_WINDOW_MS.
        let g0 = picker.pick(Sfx::Gem, 1000);
        let g1 = picker.pick(Sfx::Gem, 1120);
        let g2 = picker.pick(Sfx::Gem, 1240);
        let g3 = picker.pick(Sfx::Gem, 1360);
        assert_eq!((g0, g1, g2, g3), (0, 1, 2, 3));
        let g_reset = picker.pick(Sfx::Gem, 2500);
        assert_eq!(g_reset, 0);
    }

    #[test]
    fn music_synth_produces_sound_and_scales_with_intensity() {
        for theme in Theme::all() {
            let mut rms_by_int = [0.0f32; 3];
            for lvl in 0..=2u32 {
                let synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                let chunk: Vec<f32> = synth.take((SR as usize) * 2).collect();
                assert_eq!(chunk.len(), (SR as usize) * 2);
                assert!(chunk.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
                let r = rms(&chunk);
                assert!(r > 0.02, "{theme:?} intensity {lvl} too silent: rms={r}");
                // Music leaves plenty of headroom for SFX.
                assert!(
                    peak(&chunk) <= 0.55,
                    "{theme:?} intensity {lvl} too hot for background music: peak={}",
                    peak(&chunk)
                );
                rms_by_int[lvl as usize] = r;
            }
            assert!(
                rms_by_int[1] > rms_by_int[0],
                "{theme:?} intensity 1 should be fuller than 0: {rms_by_int:?}"
            );
            assert!(
                rms_by_int[2] > rms_by_int[1] * 0.98,
                "{theme:?} intensity 2 should add energy over 1: {rms_by_int:?}"
            );
        }
    }

    #[test]
    fn music_intensity_transitions_have_no_large_step_clicks() {
        for theme in Theme::all() {
            let intensity = Arc::new(AtomicU32::new(0));
            let mut synth = MusicSynth::new(theme, Arc::clone(&intensity));
            let mut prev = 0.0f32;
            let mut max_delta = 0.0f32;
            for i in 0..(SR as usize * 3) {
                if i == SR as usize {
                    intensity.store(1, Ordering::Relaxed);
                } else if i == SR as usize * 2 {
                    intensity.store(2, Ordering::Relaxed);
                }
                let s = synth.next().unwrap();
                max_delta = max_delta.max((s - prev).abs());
                prev = s;
            }
            assert!(
                max_delta < 0.25,
                "{theme:?} had harsh discontinuity/click: max_delta={max_delta:.4}"
            );
        }
    }

    #[test]
    fn degree_rest_is_silent() {
        assert!(degree_freq(220.0, &[0, 2, 4], REST).is_none());
        assert!(degree_freq(220.0, &[0, 2, 4], 0).is_some());
    }

    #[test]
    fn sfx_indices_are_unique_and_cover_the_cache() {
        let mut seen = std::collections::HashSet::new();
        for sfx in Sfx::ALL {
            let i = sfx.index();
            assert!(i < Sfx::ALL.len(), "index out of cache range");
            assert!(seen.insert(i), "duplicate cache index: {i}");
        }
        assert_eq!(seen.len(), Sfx::ALL.len());
    }

    #[test]
    fn sfx_cache_is_synthesized_once_per_effect_and_variant() {
        let cache = AudioEngine::build_cache();
        assert_eq!(cache.len(), Sfx::ALL.len());
        for (i, &sfx) in Sfx::ALL.iter().enumerate() {
            assert_eq!(cache[i].len(), sfx.variant_count());
            assert_eq!(cache[i][0].as_ref(), sfx_samples(sfx).as_slice());
            for (v, cached) in cache[i].iter().enumerate() {
                assert_eq!(cached.as_ref(), sfx_variant_samples(sfx, v).as_slice());
            }
        }
    }

    #[test]
    fn cached_sfx_replays_the_buffer_without_copying() {
        let data: Arc<[f32]> = Arc::from(sfx_samples(Sfx::Shoot).into_boxed_slice());
        let mut src = CachedSfx::new(Arc::clone(&data));
        assert_eq!(src.current_span_len(), Some(data.len()));
        let played: Vec<f32> = src.by_ref().collect();
        assert_eq!(played, data.to_vec());
        assert!(src.next().is_none(), "source should end with its buffer");
        assert_eq!(src.current_span_len(), Some(0));

        // Pitched replay stays bounded and scales duration cleanly.
        let pitched: Vec<f32> = CachedSfx::with_pitch(Arc::clone(&data), 1.25).collect();
        assert!(!pitched.is_empty());
        assert!(pitched.len() < data.len());
        assert!(pitched.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }

    #[test]
    fn mute_zeroes_both_channels() {
        assert_eq!(channel_vol(true, 0.50), 0.0);
        assert_eq!(channel_vol(true, SFX_VOL), 0.0);
        assert_eq!(channel_vol(false, 0.50), 0.50);
        assert_eq!(channel_vol(false, SFX_VOL), SFX_VOL);
    }

    #[test]
    fn sfx_crest_factor_and_dynamic_range_are_balanced() {
        for sfx in Sfx::ALL {
            for v in 0..sfx.variant_count() {
                let s = sfx_variant_samples(sfx, v);
                let pk = peak(&s);
                let r = rms(&s);
                let crest = pk / r.max(1e-6);
                assert!(
                    (1.8..=7.5).contains(&crest),
                    "{sfx:?} v{v} crest factor out of healthy range: {crest:.2} (pk={pk:.3}, rms={r:.3})"
                );
            }
        }
    }

    #[test]
    fn spectral_band_energy_avoids_harsh_aliasing_and_separates_roles() {
        // Compute Goertzel power at frequency `hz` around peak window (512 samples).
        fn band_power(samples: &[f32], freqs: &[f32]) -> f32 {
            let n = 512usize.min(samples.len());
            let pk_i = samples
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let s0 = pk_i.saturating_sub(n / 4).min(samples.len().saturating_sub(n));
            let win = &samples[s0..s0 + n];
            let mut total = 0.0f32;
            for &hz in freqs {
                let w = TAU * hz / SR_F;
                let coeff = 2.0 * w.cos();
                let mut q1 = 0.0f32;
                let mut q2 = 0.0f32;
                for (k, &x) in win.iter().enumerate() {
                    let hann = 0.5 * (1.0 - (TAU * k as f32 / (n - 1).max(1) as f32).cos());
                    let q0 = coeff * q1 - q2 + x * hann;
                    q2 = q1;
                    q1 = q0;
                }
                let pwr = q1 * q1 + q2 * q2 - q1 * q2 * coeff;
                total += pwr.max(0.0);
            }
            total / freqs.len().max(1) as f32
        }

        let low_f = [90.0, 150.0, 240.0, 380.0];
        let mid_f = [700.0, 1100.0, 1700.0, 2400.0];
        let harsh_f = [9500.0, 12500.0, 15500.0];

        for sfx in Sfx::ALL {
            let s = sfx_samples(sfx);
            let audible = band_power(&s, &low_f) + band_power(&s, &mid_f);
            let harsh = band_power(&s, &harsh_f);
            assert!(
                harsh < audible * 0.35 + 1e-3,
                "{sfx:?} has excessive >9kHz harsh energy: harsh={harsh:.3} audible={audible:.3}"
            );
        }

        // Boss and Explosion should carry strong low-frequency body relative to Gem.
        let exp_low = band_power(&sfx_samples(Sfx::Explosion), &low_f);
        let boss_low = band_power(&sfx_samples(Sfx::Boss), &low_f);
        let gem_low = band_power(&sfx_samples(Sfx::Gem), &low_f);
        assert!(
            exp_low > gem_low * 3.0 && boss_low > gem_low * 3.0,
            "Expected sub-heavy Explosion ({exp_low:.1}) and Boss ({boss_low:.1}) >> Gem ({gem_low:.1})"
        );
    }

    #[test]
    fn voice_priorities_protect_critical_alerts() {
        for alert in [Sfx::Win, Sfx::Lose, Sfx::Boss, Sfx::BossDeath, Sfx::LevelUp] {
            for chatter in [Sfx::Shoot, Sfx::Hit, Sfx::Gem, Sfx::UiMove] {
                assert!(alert.priority() > chatter.priority());
            }
        }
        assert!(Sfx::Hurt.priority() > Sfx::Hit.priority());
        assert!(Sfx::Nuke.priority() > Sfx::Shoot.priority());
    }

    #[test]
    fn cache_total_variants_and_memory_footprint_are_bounded() {
        let cache = AudioEngine::build_cache();
        let total_variants: usize = cache.iter().map(Vec::len).sum();
        let total_samples: usize = cache
            .iter()
            .flat_map(|b| b.iter())
            .map(|buf| buf.len())
            .sum();
        let bytes = total_samples * std::mem::size_of::<f32>();
        assert!(total_variants >= 60, "expected rich variant bank, got {total_variants}");
        assert!(
            bytes < 10 * 1024 * 1024,
            "audio cache unexpectedly large: {bytes} bytes"
        );
    }

    #[test]
    fn music_frequencies_are_only_looked_up_at_step_boundaries() {
        let mut synth = MusicSynth::new(Theme::Sewer, Arc::new(AtomicU32::new(2)));
        let sps = synth.samples_per_step;
        let _ = synth.next();
        assert_eq!(synth.cached_step, 0);
        let (bass, arp, lead) = (synth.bass_freq, synth.arp_freq, synth.lead_freq);
        for _ in 0..sps.min(4096) {
            let _ = synth.next();
        }
        if synth.counter < sps {
            assert_eq!(synth.cached_step, 0);
            assert_eq!((synth.bass_freq, synth.arp_freq, synth.lead_freq), (bass, arp, lead));
        }
        while synth.counter < sps {
            let _ = synth.next();
        }
        let _ = synth.next(); // first sample of step 1
        assert_eq!(synth.counter, sps + 1);
        let step = 1;
        assert_eq!(synth.cached_step, 1);
        assert_eq!(
            synth.bass_freq,
            degree_freq(synth.cfg.root * 0.5, synth.cfg.scale, synth.cfg.bass[step])
        );
        assert_eq!(
            synth.arp_freq,
            degree_freq(synth.cfg.root, synth.cfg.scale, synth.cfg.arp[step])
        );
        assert_eq!(
            synth.lead_freq,
            degree_freq(synth.cfg.root * 2.0, synth.cfg.scale, synth.cfg.lead[step])
        );
    }

    fn write_wav(path: &std::path::Path, samples: &[f32]) {
        use std::io::Write;
        let mut f = std::fs::File::create(path).expect("create wav");
        let n = samples.len() as u32;
        let data_bytes = n * 2;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&SR.to_le_bytes()).unwrap();
        f.write_all(&(SR * 2).to_le_bytes()).unwrap();
        f.write_all(&2u16.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_bytes.to_le_bytes()).unwrap();
        let mut buf = Vec::with_capacity(data_bytes as usize);
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            buf.extend_from_slice(&v.to_le_bytes());
        }
        f.write_all(&buf).unwrap();
    }

    #[test]
    fn dump_audio_analysis() {
        let Ok(dir_str) = std::env::var("RATRUN_AUDIO_DUMP") else {
            return;
        };
        let tag = std::env::var("RATRUN_AUDIO_TAG").unwrap_or_else(|_| "SNAP".into());
        let dir = std::path::PathBuf::from(dir_str);
        std::fs::create_dir_all(&dir).unwrap();

        let mut sfx_json = Vec::new();
        for &sfx in &Sfx::ALL {
            let fname = format!("sfx_{}.wav", sfx.name().to_lowercase());
            let samples = sfx_samples(sfx);
            write_wav(&dir.join(&fname), &samples);
            let label = format!("{} (X{})", sfx.name(), sfx.variant_count());
            let col = match sfx.priority() {
                3 => "[255,140,185]",
                2 => "[250,185,95]",
                1 => "[120,210,255]",
                _ => "[110,230,190]",
            };
            sfx_json.push(format!(
                "{{\"label\":\"{}\",\"file\":\"{}\",\"color\":{}}}",
                label, fname, col
            ));
        }

        // Dump representative variant sets (Gem combo ladder, Shoot, Kill, Dash)
        let mut var_json = Vec::new();
        for &(sfx, max_v, col) in &[
            (Sfx::Gem, 8usize, "[130,235,255]"),
            (Sfx::Shoot, 4, "[255,220,120]"),
            (Sfx::Kill, 4, "[255,145,120]"),
        ] {
            for v in 0..max_v.min(sfx.variant_count()) {
                let fname = format!("var_{}_v{}.wav", sfx.name().to_lowercase(), v);
                write_wav(&dir.join(&fname), &sfx_variant_samples(sfx, v));
                let label = format!("{} V{}", sfx.name(), v);
                var_json.push(format!(
                    "{{\"label\":\"{}\",\"file\":\"{}\",\"color\":{}}}",
                    label, fname, col
                ));
            }
        }

        let mut music_json = Vec::new();
        for theme in Theme::all() {
            let intensity = Arc::new(AtomicU32::new(0));
            let mut synth = MusicSynth::new(theme, Arc::clone(&intensity));
            let seg = (SR as usize) * 2;
            let mut out = Vec::with_capacity(seg * 3);
            for lvl in 0..=2u32 {
                intensity.store(lvl, Ordering::Relaxed);
                for _ in 0..seg {
                    out.push(synth.next().unwrap_or(0.0));
                }
            }
            let fname = format!("music_{}.wav", theme.name().to_lowercase());
            write_wav(&dir.join(&fname), &out);
            music_json.push(format!(
                "{{\"label\":\"{}\",\"file\":\"{}\",\"color\":[150,185,255]}}",
                theme.name(),
                fname
            ));
        }

        let manifest = format!(
            "{{\"tag\":\"{}\",\"sfx\":[{}],\"variants\":[{}],\"music\":[{}]}}",
            tag,
            sfx_json.join(","),
            var_json.join(","),
            music_json.join(",")
        );
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
    }
}
