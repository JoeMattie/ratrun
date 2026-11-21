//! Procedural audio: a looping themed chiptune track + layered synthesized SFX.
//!
//! Everything is generated from scratch — no audio assets. The whole module
//! degrades to silent if no output device is available (headless, no server),
//! so the game never fails because of audio.

use std::f32::consts::{PI, TAU};
use std::num::NonZero;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    // Traps, gates, rat-holes, and maze cache.
    TrapGlueGrab,
    TrapGlueRelease,
    TrapTrigger,
    TrapSnap,
    GateOpen,
    GateRefuse,
    GateSlam,
    HoleSqueeze,
    CacheOpen,
    // Jaguar boss (Sewer).
    CatGrowl,
    CatCrouch,
    CatPounce,
    CatLand,
    CatSwipe,
    CatRoar,
    CatHurt,
    CatDeath,
    // Human with broom boss (Kitchen).
    BroomStomp,
    BroomSweep,
    BroomJab,
    BroomDustpan,
    BroomHurt,
    BroomHop,
    BroomDefeat,
    // Robot lawnmower boss (Lab).
    MowerIdle,
    MowerScan,
    MowerLockOn,
    MowerCharge,
    MowerWallClang,
    MowerFlipped,
    MowerBladeSpin,
    MowerOverheat,
    MowerDockHum,
    MowerDeath,
    // Campaign progression fanfares.
    LevelComplete,
    CampaignVictory,
    // Active (Space-triggered) weapons.
    ShootThrash,
    SonicSqueak,
    // Wave 3 weapons A.
    ShootRain,
    Infect,
}

impl Sfx {
    /// Every effect, in cache order. `index` must agree with this ordering.
    pub const ALL: [Sfx; 63] = [
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
        // Traps, gates, rat-holes, and maze cache.
        Sfx::TrapGlueGrab,
        Sfx::TrapGlueRelease,
        Sfx::TrapTrigger,
        Sfx::TrapSnap,
        Sfx::GateOpen,
        Sfx::GateRefuse,
        Sfx::GateSlam,
        Sfx::HoleSqueeze,
        Sfx::CacheOpen,
        // Jaguar boss (Sewer).
        Sfx::CatGrowl,
        Sfx::CatCrouch,
        Sfx::CatPounce,
        Sfx::CatLand,
        Sfx::CatSwipe,
        Sfx::CatRoar,
        Sfx::CatHurt,
        Sfx::CatDeath,
        // Human with broom boss (Kitchen).
        Sfx::BroomStomp,
        Sfx::BroomSweep,
        Sfx::BroomJab,
        Sfx::BroomDustpan,
        Sfx::BroomHurt,
        Sfx::BroomHop,
        Sfx::BroomDefeat,
        // Robot lawnmower boss (Lab).
        Sfx::MowerIdle,
        Sfx::MowerScan,
        Sfx::MowerLockOn,
        Sfx::MowerCharge,
        Sfx::MowerWallClang,
        Sfx::MowerFlipped,
        Sfx::MowerBladeSpin,
        Sfx::MowerOverheat,
        Sfx::MowerDockHum,
        Sfx::MowerDeath,
        // Campaign progression fanfares.
        Sfx::LevelComplete,
        Sfx::CampaignVictory,
        // Active (Space-triggered) weapons.
        Sfx::ShootThrash,
        Sfx::SonicSqueak,
        // Wave 3 weapons A.
        Sfx::ShootRain,
        Sfx::Infect,
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
            Sfx::TrapGlueGrab => "TRAP_GLUE_GRAB",
            Sfx::TrapGlueRelease => "TRAP_GLUE_REL",
            Sfx::TrapTrigger => "TRAP_TRIGGER",
            Sfx::TrapSnap => "TRAP_SNAP",
            Sfx::GateOpen => "GATE_OPEN",
            Sfx::GateRefuse => "GATE_REFUSE",
            Sfx::GateSlam => "GATE_SLAM",
            Sfx::HoleSqueeze => "HOLE_SQUEEZE",
            Sfx::CacheOpen => "CACHE_OPEN",
            Sfx::CatGrowl => "CAT_GROWL",
            Sfx::CatCrouch => "CAT_CROUCH",
            Sfx::CatPounce => "CAT_POUNCE",
            Sfx::CatLand => "CAT_LAND",
            Sfx::CatSwipe => "CAT_SWIPE",
            Sfx::CatRoar => "CAT_ROAR",
            Sfx::CatHurt => "CAT_HURT",
            Sfx::CatDeath => "CAT_DEATH",
            Sfx::BroomStomp => "BROOM_STOMP",
            Sfx::BroomSweep => "BROOM_SWEEP",
            Sfx::BroomJab => "BROOM_JAB",
            Sfx::BroomDustpan => "BROOM_DUSTPAN",
            Sfx::BroomHurt => "BROOM_HURT",
            Sfx::BroomHop => "BROOM_HOP",
            Sfx::BroomDefeat => "BROOM_DEFEAT",
            Sfx::MowerIdle => "MOWER_IDLE",
            Sfx::MowerScan => "MOWER_SCAN",
            Sfx::MowerLockOn => "MOWER_LOCKON",
            Sfx::MowerCharge => "MOWER_CHARGE",
            Sfx::MowerWallClang => "MOWER_CLANG",
            Sfx::MowerFlipped => "MOWER_FLIPPED",
            Sfx::MowerBladeSpin => "MOWER_BLADE",
            Sfx::MowerOverheat => "MOWER_STEAM",
            Sfx::MowerDockHum => "MOWER_DOCK",
            Sfx::MowerDeath => "MOWER_DEATH",
            Sfx::LevelComplete => "LVL_COMPLETE",
            Sfx::CampaignVictory => "CAMPAIGN_WIN",
            Sfx::ShootThrash => "THRASH",
            Sfx::SonicSqueak => "SQUEAK",
            Sfx::ShootRain => "RAIN",
            Sfx::Infect => "INFECT",
        }
    }

    /// Number of pre-synthesized variants cached for this effect.
    pub fn variant_count(self) -> usize {
        match self {
            Sfx::Gem => 8, // rising pentatonic combo ladder
            Sfx::Hit | Sfx::Kill => 5,
            Sfx::Shoot
            | Sfx::Dash
            | Sfx::ShootSpray
            | Sfx::ShootWhip
            | Sfx::ShootThrash
            | Sfx::SonicSqueak
            | Sfx::BroomStomp => 4,
            Sfx::Explosion
            | Sfx::Hurt
            | Sfx::UiMove
            | Sfx::ShootNova
            | Sfx::ShootOrbit
            | Sfx::ShootAcid
            | Sfx::TrapGlueGrab
            | Sfx::TrapGlueRelease
            | Sfx::TrapTrigger
            | Sfx::TrapSnap
            | Sfx::HoleSqueeze
            | Sfx::CatGrowl
            | Sfx::CatPounce
            | Sfx::CatLand
            | Sfx::CatSwipe
            | Sfx::CatHurt
            | Sfx::BroomSweep
            | Sfx::BroomJab
            | Sfx::BroomHurt
            | Sfx::BroomHop
            | Sfx::MowerScan
            | Sfx::MowerCharge
            | Sfx::MowerWallClang
            | Sfx::MowerFlipped
            | Sfx::ShootRain => 3,
            Sfx::Pickup
            | Sfx::UiSelect
            | Sfx::Heal
            | Sfx::Magnet
            | Sfx::Nuke
            | Sfx::GateOpen
            | Sfx::GateRefuse
            | Sfx::GateSlam
            | Sfx::CatCrouch
            | Sfx::BroomDustpan
            | Sfx::MowerIdle
            | Sfx::MowerLockOn
            | Sfx::MowerBladeSpin
            | Sfx::MowerOverheat
            | Sfx::MowerDockHum
            | Sfx::Infect => 2,
            Sfx::LevelUp
            | Sfx::Boss
            | Sfx::Win
            | Sfx::Lose
            | Sfx::BossDeath
            | Sfx::CacheOpen
            | Sfx::CatRoar
            | Sfx::CatDeath
            | Sfx::BroomDefeat
            | Sfx::MowerDeath
            | Sfx::LevelComplete
            | Sfx::CampaignVictory => 1,
        }
    }

    /// Voice-stealing priority when the active voice pool is full.
    fn priority(self) -> u8 {
        match self {
            Sfx::Win
            | Sfx::Lose
            | Sfx::Boss
            | Sfx::BossDeath
            | Sfx::LevelUp
            | Sfx::CacheOpen
            | Sfx::CatRoar
            | Sfx::CatDeath
            | Sfx::BroomDefeat
            | Sfx::MowerDeath
            | Sfx::LevelComplete
            | Sfx::CampaignVictory => 3,
            Sfx::Hurt
            | Sfx::Nuke
            | Sfx::ShootThrash
            | Sfx::SonicSqueak
            | Sfx::Explosion
            | Sfx::Heal
            | Sfx::Magnet
            | Sfx::Pickup
            | Sfx::TrapSnap
            | Sfx::GateSlam
            | Sfx::CatCrouch
            | Sfx::CatPounce
            | Sfx::CatLand
            | Sfx::CatSwipe
            | Sfx::CatHurt
            | Sfx::BroomStomp
            | Sfx::BroomSweep
            | Sfx::BroomJab
            | Sfx::BroomDustpan
            | Sfx::BroomHurt
            | Sfx::MowerLockOn
            | Sfx::MowerCharge
            | Sfx::MowerWallClang
            | Sfx::MowerBladeSpin
            | Sfx::MowerOverheat
            | Sfx::Infect => 2,
            Sfx::Kill
            | Sfx::Dash
            | Sfx::ShootNova
            | Sfx::ShootWhip
            | Sfx::ShootAcid
            | Sfx::ShootOrbit
            | Sfx::UiSelect
            | Sfx::TrapGlueGrab
            | Sfx::TrapGlueRelease
            | Sfx::TrapTrigger
            | Sfx::GateOpen
            | Sfx::GateRefuse
            | Sfx::HoleSqueeze
            | Sfx::CatGrowl
            | Sfx::BroomHop
            | Sfx::MowerIdle
            | Sfx::MowerScan
            | Sfx::MowerFlipped
            | Sfx::MowerDockHum
            | Sfx::ShootRain => 1,
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
    trem_hz: f32,
    trem_depth: f32,
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
            trem_hz: 0.0,
            trem_depth: 0.0,
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
            let mut env = ad_env(t, p.dur_s, p.attack_s, p.decay_pow);
            if p.trem_depth > 0.0 && p.trem_hz > 0.0 {
                let lfo = 0.5 + 0.5 * (k as f32 / SR_F * p.trem_hz * TAU).sin();
                env *= 1.0 - p.trem_depth * (1.0 - lfo);
            }
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

        // ---- Traps, gates, rat-holes, and maze cache ---------------------

        // 24. TrapGlueGrab: sticky trap grabbing a victim — thick adhesive splat + suction gulp.
        Sfx::TrapGlueGrab => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.91];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.185);
            // Cardboard tray low contact thud
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.052,
                f0: 155.0 * p,
                f1: 62.0 * p,
                pitch_pow: 0.50,
                wave: Wave::Sine,
                gain: 0.42,
                attack_s: 0.002,
                decay_pow: 1.8,
                ..Default::default()
            });
            // Wet syrupy adhesive squelch sweep downward
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.145,
                pink: true,
                gain: 0.58,
                attack_s: 0.004,
                decay_pow: 1.65,
                filter: FilterMode::BandPass,
                fc0: 1650.0 * p,
                fc1: 360.0 * p,
                fc_pow: 0.65,
                q: 3.8,
                trem_hz: 26.0,
                trem_depth: 0.35,
                seed: base_seed,
            });
            // Viscous suction "shlup-gulp" downward then distinct syrupy upward tack
            m.add_osc(OscLayer {
                start_s: 0.006,
                dur_s: 0.072,
                f0: 320.0 * p,
                f1: 135.0 * p,
                pitch_pow: 0.55,
                wave: Wave::Triangle,
                gain: 0.52,
                attack_s: 0.004,
                decay_pow: 1.5,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.054,
                dur_s: 0.118,
                f0: 160.0 * p,
                f1: 440.0 * p,
                pitch_pow: 0.68,
                wave: Wave::Sine,
                gain: 0.62,
                attack_s: 0.007,
                decay_pow: 1.55,
                ..Default::default()
            });
            m.finish(0.44, 1.4, 2.0, 14.0)
        }

        // 25. TrapGlueRelease: stretching sticky strands culminating in a wet unstick pop!
        Sfx::TrapGlueRelease => {
            const PITCH: [f32; 3] = [1.0, 1.08, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.175);
            // Taut rubbery strand stretch leading into the break
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.095,
                pink: true,
                gain: 0.48,
                attack_s: 0.010,
                decay_pow: 0.85,
                filter: FilterMode::BandPass,
                fc0: 420.0 * p,
                fc1: 1750.0 * p,
                fc_pow: 1.05,
                q: 4.4,
                trem_hz: 22.0,
                trem_depth: 0.30,
                seed: base_seed,
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.088,
                f0: 210.0 * p,
                f1: 490.0 * p,
                pitch_pow: 0.85,
                wave: Wave::Triangle,
                gain: 0.42,
                attack_s: 0.008,
                decay_pow: 0.9,
                ..Default::default()
            });
            // Wet unstick "PLOP!" snap at 76ms
            m.add_osc(OscLayer {
                start_s: 0.076,
                dur_s: 0.086,
                f0: 270.0 * p,
                f1: 860.0 * p,
                pitch_pow: 0.58,
                wave: Wave::Sine,
                gain: 0.72,
                attack_s: 0.0025,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.076,
                dur_s: 0.042,
                pink: true,
                gain: 0.46,
                attack_s: 0.0015,
                decay_pow: 2.1,
                filter: FilterMode::BandPass,
                fc0: 1950.0 * p,
                fc1: 680.0 * p,
                q: 2.4,
                seed: base_seed ^ 0x44BB,
                ..Default::default()
            });
            m.finish(0.42, 1.35, 2.0, 14.0)
        }

        // 26. TrapTrigger: mousetrap's quiet metallic bait-plate click + tiny spring twitch.
        Sfx::TrapTrigger => {
            const PITCH: [f32; 3] = [1.0, 1.11, 0.90];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.068);
            // Delicate copper-plate trip click (starts at 1.5ms so fade-in preserves crisp peak)
            m.add_noise(NoiseLayer {
                start_s: 0.0015,
                dur_s: 0.018,
                pink: false,
                gain: 0.46,
                attack_s: 0.0008,
                decay_pow: 2.3,
                filter: FilterMode::BandPass,
                fc0: 3000.0 * p,
                fc1: 1650.0 * p,
                q: 2.8,
                seed: base_seed,
                ..Default::default()
            });
            // Crisp tiny brass pin tick
            m.add_fm(FmLayer {
                start_s: 0.0015,
                dur_s: 0.032,
                c0: 1580.0 * p,
                c1: 1220.0 * p,
                pitch_pow: 0.5,
                mod_ratio: 2.76,
                mod_index0: 1.8,
                mod_index1: 0.1,
                mod_pow: 2.4,
                gain: 0.54,
                attack_s: 0.001,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 4600.0,
                fc1: 1800.0,
                q: 1.0,
            });
            // Subtle coiled-spring tension twitch warning at 16ms
            m.add_fm(FmLayer {
                start_s: 0.016,
                dur_s: 0.048,
                c0: 540.0 * p,
                c1: 690.0 * p,
                pitch_pow: 0.7,
                mod_ratio: 1.92,
                mod_index0: 1.25,
                mod_index1: 0.2,
                mod_pow: 1.8,
                gain: 0.46,
                attack_s: 0.002,
                decay_pow: 1.65,
                filter: FilterMode::BandPass,
                fc0: 1100.0 * p,
                fc1: 1500.0 * p,
                q: 1.6,
            });
            m.finish(0.25, 1.25, 1.2, 8.0)
        }

        // 27. TrapSnap: mousetrap's loud coiled-wire whip + wooden baseboard CRACK + spring twang!
        Sfx::TrapSnap => {
            const PITCH: [f32; 3] = [1.0, 0.93, 1.08];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.210);
            // Split-second coiled wire whip-arc into the impact
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.022,
                pink: true,
                gain: 0.45,
                attack_s: 0.002,
                decay_pow: 0.8,
                filter: FilterMode::BandPass,
                fc0: 900.0 * p,
                fc1: 3100.0 * p,
                q: 2.0,
                seed: base_seed ^ 0x1234,
                ..Default::default()
            });
            // Violent hardwood baseboard CRACK at 14ms
            m.add_noise(NoiseLayer {
                start_s: 0.014,
                dur_s: 0.055,
                pink: false,
                gain: 0.88,
                attack_s: 0.0008,
                decay_pow: 2.3,
                filter: FilterMode::BandPass,
                fc0: 2500.0 * p,
                fc1: 650.0 * p,
                q: 1.2,
                seed: base_seed,
                ..Default::default()
            });
            // Punchy hardwood body knock
            m.add_osc(OscLayer {
                start_s: 0.014,
                dur_s: 0.088,
                f0: 360.0 * p,
                f1: 88.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Sine,
                gain: 0.85,
                attack_s: 0.001,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.014,
                dur_s: 0.068,
                f0: 520.0 * p,
                f1: 160.0 * p,
                pitch_pow: 0.48,
                wave: Wave::Saw,
                gain: 0.44,
                attack_s: 0.001,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 2200.0,
                fc1: 420.0,
                q: 1.3,
                ..Default::default()
            });
            // Iconic vibrating steel striker-bar spring twang
            m.add_fm(FmLayer {
                start_s: 0.016,
                dur_s: 0.185,
                c0: 620.0 * p,
                c1: 565.0 * p,
                pitch_pow: 0.7,
                mod_ratio: 2.41,
                mod_index0: 2.8,
                mod_index1: 0.15,
                mod_pow: 1.9,
                gain: 0.48,
                attack_s: 0.0015,
                decay_pow: 1.7,
                filter: FilterMode::LowPass,
                fc0: 3800.0,
                fc1: 1100.0,
                q: 1.1,
            });
            m.add_short_tail(0.14, 2400.0);
            m.finish(0.72, 1.85, 1.2, 16.0)
        }

        // 28. GateOpen: one-way door V-flap swinging open (latch click + spring hinge creak-whoosh).
        Sfx::GateOpen => {
            let p = if v == 0 { 1.0 } else { 1.08 };
            let mut m = MixBuf::new(0.185);
            // Brass latch release click (balanced so flap swing body owns the envelope)
            m.add_fm(FmLayer {
                start_s: 0.002,
                dur_s: 0.032,
                c0: 1280.0 * p,
                c1: 940.0 * p,
                pitch_pow: 0.5,
                mod_ratio: 2.31,
                mod_index0: 1.5,
                mod_index1: 0.1,
                mod_pow: 2.2,
                gain: 0.30,
                attack_s: 0.0015,
                decay_pow: 2.0,
                ..Default::default()
            });
            // Spring-loaded flap swing-open air brush
            m.add_noise(NoiseLayer {
                start_s: 0.008,
                dur_s: 0.168,
                pink: true,
                gain: 0.78,
                attack_s: 0.016,
                decay_pow: 1.30,
                filter: FilterMode::BandPass,
                fc0: 540.0 * p,
                fc1: 1850.0 * p,
                fc_pow: 0.75,
                q: 1.9,
                seed: base_seed,
                ..Default::default()
            });
            // Expressive spring-hinge turn creak
            m.add_osc(OscLayer {
                start_s: 0.012,
                dur_s: 0.160,
                f0: 310.0 * p,
                f1: 560.0 * p,
                pitch_pow: 0.8,
                wave: Wave::Saw,
                gain: 0.56,
                attack_s: 0.016,
                decay_pow: 1.30,
                filter: FilterMode::BandPass,
                fc0: 760.0 * p,
                fc1: 1560.0 * p,
                q: 2.2,
                trem_hz: 24.0,
                trem_depth: 0.34,
                ..Default::default()
            });
            m.finish(0.42, 1.55, 1.5, 15.0)
        }

        // 29. GateRefuse: sharp backwards mechanical stop-clunk + taut dissonant refusal buzz.
        Sfx::GateRefuse => {
            let p = if v == 0 { 1.0 } else { 0.93 };
            let mut m = MixBuf::new(0.185);
            // Mechanical stop-peg clunk at t=0
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.048,
                f0: 230.0 * p,
                f1: 78.0 * p,
                pitch_pow: 0.45,
                wave: Wave::Triangle,
                gain: 0.56,
                attack_s: 0.0015,
                decay_pow: 1.9,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.024,
                pink: false,
                gain: 0.34,
                attack_s: 0.001,
                decay_pow: 2.3,
                filter: FilterMode::BandPass,
                fc0: 1700.0 * p,
                fc1: 720.0 * p,
                q: 1.8,
                seed: base_seed,
                ..Default::default()
            });
            // Taut two-beat dissonant minor-2nd refusal buzz ("BZZT-BZZT!")
            for &(st, dur, mul) in &[(0.006f32, 0.078f32, 1.0f32), (0.076, 0.098, 0.93)] {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: 158.0 * p * mul,
                    f1: 144.0 * p * mul,
                    pitch_pow: 1.0,
                    wave: Wave::Pulse(0.32),
                    gain: 0.52,
                    attack_s: 0.003,
                    decay_pow: 1.3,
                    filter: FilterMode::LowPass,
                    fc0: 1250.0,
                    fc1: 560.0,
                    q: 1.4,
                    trem_hz: 44.0,
                    trem_depth: 0.35,
                    ..Default::default()
                });
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: 169.0 * p * mul,
                    f1: 154.0 * p * mul,
                    pitch_pow: 1.0,
                    wave: Wave::Saw,
                    gain: 0.44,
                    attack_s: 0.003,
                    decay_pow: 1.35,
                    filter: FilterMode::LowPass,
                    fc0: 1100.0,
                    fc1: 480.0,
                    q: 1.2,
                    ..Default::default()
                });
            }
            m.finish(0.44, 1.5, 1.5, 14.0)
        }

        // 30. GateSlam: heavy latch SLAM when the one-way gate shuts behind the rat!
        Sfx::GateSlam => {
            let p = if v == 0 { 1.0 } else { 0.94 };
            let mut m = MixBuf::new(0.340);
            // Brief rushing leaf-close whoosh into impact
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.026,
                pink: true,
                gain: 0.42,
                attack_s: 0.003,
                decay_pow: 0.8,
                filter: FilterMode::BandPass,
                fc0: 1350.0 * p,
                fc1: 480.0 * p,
                q: 1.6,
                seed: base_seed ^ 0x2211,
                ..Default::default()
            });
            // Massive iron-gate frame SLAM at 18ms
            m.add_osc(OscLayer {
                start_s: 0.018,
                dur_s: 0.210,
                f0: 158.0 * p,
                f1: 36.0 * p,
                pitch_pow: 0.40,
                wave: Wave::Sine,
                gain: 0.92,
                attack_s: 0.0015,
                decay_pow: 1.55,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.018,
                dur_s: 0.165,
                pink: true,
                gain: 0.72,
                attack_s: 0.0015,
                decay_pow: 1.8,
                filter: FilterMode::LowPass,
                fc0: 2700.0 * p,
                fc1: 240.0,
                fc_pow: 0.55,
                q: 1.3,
                seed: base_seed,
                ..Default::default()
            });
            // Metallic frame resonant ring
            m.add_fm(FmLayer {
                start_s: 0.018,
                dur_s: 0.260,
                c0: 285.0 * p,
                c1: 245.0 * p,
                pitch_pow: 0.6,
                mod_ratio: 1.63,
                mod_index0: 2.6,
                mod_index1: 0.18,
                mod_pow: 1.7,
                gain: 0.44,
                attack_s: 0.0015,
                decay_pow: 1.7,
                filter: FilterMode::LowPass,
                fc0: 2400.0,
                fc1: 680.0,
                q: 1.2,
            });
            // Authoritative steel deadbolt LATCH-CLACK at 58ms ("BOOM-CLACK!")
            m.add_fm(FmLayer {
                start_s: 0.058,
                dur_s: 0.075,
                c0: 1420.0 * p,
                c1: 920.0 * p,
                pitch_pow: 0.5,
                mod_ratio: 2.73,
                mod_index0: 2.4,
                mod_index1: 0.15,
                mod_pow: 2.3,
                gain: 0.52,
                attack_s: 0.001,
                decay_pow: 2.1,
                filter: FilterMode::LowPass,
                fc0: 4200.0,
                fc1: 1400.0,
                q: 1.0,
            });
            m.add_noise(NoiseLayer {
                start_s: 0.058,
                dur_s: 0.032,
                pink: false,
                gain: 0.52,
                attack_s: 0.0008,
                decay_pow: 2.4,
                filter: FilterMode::BandPass,
                fc0: 2200.0 * p,
                fc1: 850.0 * p,
                q: 1.6,
                seed: base_seed ^ 0x77EE,
                ..Default::default()
            });
            m.add_short_tail(0.22, 1850.0);
            m.finish(0.74, 1.85, 1.5, 22.0)
        }

        // 31. HoleSqueeze: agile 0.24s rat-hole tunnel squeeze-brush + nimble emerge pop.
        Sfx::HoleSqueeze => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.240);
            // Fur brushing tight stone cheeks with wiggle modulation
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.205,
                pink: true,
                gain: 0.62,
                attack_s: 0.016,
                decay_pow: 1.30,
                filter: FilterMode::BandPass,
                fc0: 760.0 * p,
                fc1: 1700.0 * p,
                fc_pow: 0.85,
                q: 2.2,
                trem_hz: 18.0,
                trem_depth: 0.38,
                seed: base_seed,
            });
            // Compressing downward squeeze contour overlapping smoothly into emerge
            m.add_osc(OscLayer {
                start_s: 0.004,
                dur_s: 0.130,
                f0: 390.0 * p,
                f1: 230.0 * p,
                pitch_pow: 0.8,
                wave: Wave::Triangle,
                gain: 0.48,
                attack_s: 0.012,
                decay_pow: 1.20,
                ..Default::default()
            });
            // Nimble rubbery emerge pop out the far mouth!
            m.add_osc(OscLayer {
                start_s: 0.098,
                dur_s: 0.128,
                f0: 255.0 * p,
                f1: 690.0 * p,
                pitch_pow: 0.65,
                wave: Wave::Sine,
                gain: 0.60,
                attack_s: 0.007,
                decay_pow: 1.70,
                ..Default::default()
            });
            m.finish(0.36, 1.3, 2.5, 16.0)
        }

        // 32. CacheOpen: gilded maze reliquary opening — brass clasp click + G-Lydian treasure sting
        // (crystalline FM celesta/harp bells, clearly distinct from LevelUp's C-major pulse fanfare).
        Sfx::CacheOpen => {
            let mut m = MixBuf::new(0.560);
            // Crisp gilded reliquary clasp unlatch click
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.032,
                c0: 1820.0,
                c1: 1340.0,
                pitch_pow: 0.4,
                mod_ratio: 2.41,
                mod_index0: 1.8,
                mod_index1: 0.1,
                mod_pow: 2.4,
                gain: 0.46,
                attack_s: 0.001,
                decay_pow: 2.3,
                ..Default::default()
            });
            // Shimmering G Lydian ascending treasure flourish (G5 - B5 - C#6 - D6 - G6)
            let notes: [(f32, f32, f32); 5] = [
                (0.016, 783.99, 0.18),  // G5 (blooms seamlessly out of clasp click)
                (0.076, 987.77, 0.18),  // B5
                (0.136, 1108.73, 0.19), // C#6 (Lydian #4 golden sparkle!)
                (0.196, 1174.66, 0.22), // D6
                (0.262, 1567.98, 0.29), // G6 radiant crown bell
            ];
            for (i, &(st, f, dur)) in notes.iter().enumerate() {
                let last = i == 4;
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: dur,
                    c0: f,
                    c1: f * 1.003,
                    pitch_pow: 0.4,
                    mod_ratio: 3.0,
                    mod_index0: if last { 1.65 } else { 1.35 },
                    mod_index1: 0.06,
                    mod_pow: 2.4,
                    gain: if last { 0.48 } else { 0.38 },
                    attack_s: 0.0025,
                    decay_pow: if last { 1.45 } else { 1.85 },
                    filter: FilterMode::LowPass,
                    fc0: 5800.0,
                    fc1: 2400.0,
                    q: 0.85,
                });
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur * 1.05,
                    f0: f * 0.5,
                    f1: f * 0.5,
                    wave: Wave::Sine,
                    gain: if last { 0.34 } else { 0.22 },
                    attack_s: 0.004,
                    decay_pow: 1.6,
                    vib_hz: if last { 6.0 } else { 0.0 },
                    vib_depth: if last { 0.008 } else { 0.0 },
                    ..Default::default()
                });
            }
            // Warm gilded G3 + D4 harmonic cradle underneath
            for &(bf, g) in &[(196.00f32, 0.30f32), (293.66, 0.24)] {
                m.add_osc(OscLayer {
                    start_s: 0.022,
                    dur_s: 0.49,
                    f0: bf,
                    f1: bf,
                    wave: Wave::Triangle,
                    gain: g,
                    attack_s: 0.018,
                    decay_pow: 1.4,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.24, 3800.0);
            m.finish(0.62, 1.2, 1.5, 28.0)
        }

        // ---- Jaguar boss (Sewer) -----------------------------------------

        // 33. CatGrowl: low stalking jaguar growl, loop-able as a seamless one-shot.
        Sfx::CatGrowl => {
            const PITCH: [f32; 3] = [1.0, 0.93, 1.07];
            const FLUTTER: [f32; 3] = [23.0, 20.5, 26.0];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.540);
            // Deep detuned feline vocal-cord rumble with glottal flutter
            for &(det, g) in &[(0.992f32, 0.52f32), (1.011, 0.48)] {
                m.add_osc(OscLayer {
                    start_s: 0.0,
                    dur_s: 0.540,
                    f0: 69.0 * p * det,
                    f1: 63.0 * p * det,
                    pitch_pow: 1.0,
                    wave: Wave::Saw,
                    gain: g,
                    attack_s: 0.110,
                    decay_pow: 0.65,
                    filter: FilterMode::LowPass,
                    fc0: 340.0 * p,
                    fc1: 230.0 * p,
                    q: 1.6,
                    vib_hz: 4.5,
                    vib_depth: 0.018,
                    trem_hz: FLUTTER[v],
                    trem_depth: 0.45,
                });
            }
            // Resonant feline throat formant band
            m.add_osc(OscLayer {
                start_s: 0.02,
                dur_s: 0.500,
                f0: 138.0 * p,
                f1: 124.0 * p,
                wave: Wave::Saw,
                gain: 0.34,
                attack_s: 0.100,
                decay_pow: 0.70,
                filter: FilterMode::BandPass,
                fc0: 440.0 * p,
                fc1: 310.0 * p,
                q: 3.0,
                trem_hz: FLUTTER[v],
                trem_depth: 0.50,
                ..Default::default()
            });
            // Low breathy chest air
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.540,
                pink: true,
                gain: 0.38,
                attack_s: 0.110,
                decay_pow: 0.65,
                filter: FilterMode::LowPass,
                fc0: 420.0,
                fc1: 220.0,
                q: 1.3,
                trem_hz: FLUTTER[v],
                trem_depth: 0.40,
                seed: base_seed,
                ..Default::default()
            });
            // Symmetrical long cosine fades make back-to-back triggers blend seamlessly
            m.finish(0.36, 1.5, 48.0, 72.0)
        }

        // 34. CatCrouch: taut coiling crouch tension swelling toward the pounce lock.
        Sfx::CatCrouch => {
            let p = if v == 0 { 1.0 } else { 1.07 };
            let mut m = MixBuf::new(0.480);
            // Coiling low muscular tension swelling and climbing in pitch
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.440,
                f0: 76.0 * p,
                f1: 148.0 * p,
                pitch_pow: 1.35,
                wave: Wave::Saw,
                gain: 0.58,
                attack_s: 0.145,
                decay_pow: 0.58,
                filter: FilterMode::LowPass,
                fc0: 260.0,
                fc1: 1080.0,
                q: 2.3,
                trem_hz: 28.0,
                trem_depth: 0.40,
                ..Default::default()
            });
            // Taut claws gripping stone + rising predator breath hiss
            m.add_noise(NoiseLayer {
                start_s: 0.02,
                dur_s: 0.420,
                pink: true,
                gain: 0.54,
                attack_s: 0.165,
                decay_pow: 0.62,
                filter: FilterMode::BandPass,
                fc0: 520.0 * p,
                fc1: 2150.0 * p,
                fc_pow: 1.15,
                q: 2.5,
                trem_hz: 22.0,
                trem_depth: 0.28,
                seed: base_seed,
            });
            // Sharp subtle claw-dig / target-lock glint near 75% progress (340ms)
            m.add_fm(FmLayer {
                start_s: 0.340,
                dur_s: 0.120,
                c0: 880.0 * p,
                c1: 1260.0 * p,
                pitch_pow: 0.6,
                mod_ratio: 1.5,
                mod_index0: 1.85,
                mod_index1: 0.2,
                mod_pow: 2.0,
                gain: 0.44,
                attack_s: 0.008,
                decay_pow: 1.65,
                filter: FilterMode::BandPass,
                fc0: 1400.0,
                fc1: 2150.0,
                q: 1.8,
            });
            m.finish(0.48, 1.55, 18.0, 28.0)
        }

        // 35. CatPounce: explosive jaguar launch whoosh + haunch pushoff + leap snarl.
        Sfx::CatPounce => {
            const PITCH: [f32; 3] = [1.0, 1.07, 0.93];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.270);
            // Muscular haunch push-off thump (balanced so the sweeping airborne whoosh leads)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.068,
                f0: 185.0 * p,
                f1: 56.0 * p,
                pitch_pow: 0.45,
                wave: Wave::Sine,
                gain: 0.38,
                attack_s: 0.002,
                decay_pow: 1.75,
                ..Default::default()
            });
            // Fierce mid-leap predator snarl swelling through the jump
            m.add_osc(OscLayer {
                start_s: 0.016,
                dur_s: 0.175,
                f0: 265.0 * p,
                f1: 440.0 * p,
                pitch_pow: 0.55,
                wave: Wave::Saw,
                gain: 0.60,
                attack_s: 0.022,
                decay_pow: 1.35,
                filter: FilterMode::BandPass,
                fc0: 660.0 * p,
                fc1: 1580.0 * p,
                q: 2.3,
                trem_hz: 32.0,
                trem_depth: 0.30,
                ..Default::default()
            });
            // Big sweeping airborne body whoosh (swells across mid-leap then rushes past)
            m.add_noise(NoiseLayer {
                start_s: 0.008,
                dur_s: 0.155,
                pink: true,
                gain: 0.92,
                attack_s: 0.036,
                decay_pow: 0.85,
                filter: FilterMode::BandPass,
                fc0: 360.0 * p,
                fc1: 2200.0 * p,
                fc_pow: 0.85,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.072,
                dur_s: 0.192,
                pink: true,
                gain: 0.88,
                attack_s: 0.026,
                decay_pow: 1.55,
                filter: FilterMode::BandPass,
                fc0: 2050.0 * p,
                fc1: 460.0 * p,
                fc_pow: 1.0,
                q: 1.6,
                seed: base_seed ^ 0x61A2,
                ..Default::default()
            });
            m.finish(0.58, 1.75, 2.0, 18.0)
        }

        // 36. CatLand: heavy jaguar landing thud + overshooting claw-skid on sewer stone.
        Sfx::CatLand => {
            const PITCH: [f32; 3] = [1.0, 0.93, 1.07];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.290);
            // Deep heavy predator landing sub-slam
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.175,
                f0: 148.0 * p,
                f1: 35.0 * p,
                pitch_pow: 0.40,
                wave: Wave::Sine,
                gain: 0.92,
                attack_s: 0.002,
                decay_pow: 1.6,
                ..Default::default()
            });
            // Ground impact crunch
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.145,
                pink: true,
                gain: 0.72,
                attack_s: 0.002,
                decay_pow: 1.85,
                filter: FilterMode::LowPass,
                fc0: 2400.0 * p,
                fc1: 210.0,
                fc_pow: 0.55,
                q: 1.2,
                seed: base_seed,
                ..Default::default()
            });
            // Overshooting claw-skid stone gravel scrape
            m.add_noise(NoiseLayer {
                start_s: 0.036,
                dur_s: 0.240,
                pink: true,
                gain: 0.48,
                attack_s: 0.010,
                decay_pow: 1.65,
                filter: FilterMode::BandPass,
                fc0: 1950.0 * p,
                fc1: 580.0 * p,
                fc_pow: 0.8,
                q: 2.0,
                trem_hz: 30.0,
                trem_depth: 0.35,
                seed: base_seed ^ 0x9933,
            });
            m.add_short_tail(0.16, 1500.0);
            m.finish(0.72, 1.8, 1.5, 20.0)
        }

        // 37. CatSwipe: fast triple-talon claw slash + sharp predator snap.
        Sfx::CatSwipe => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.195);
            // Sweeping razor air-rending arc
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.160,
                pink: true,
                gain: 0.80,
                attack_s: 0.012,
                decay_pow: 1.45,
                filter: FilterMode::BandPass,
                fc0: 760.0 * p,
                fc1: 3000.0 * p,
                fc_pow: 0.65,
                q: 2.0,
                seed: base_seed,
                ..Default::default()
            });
            // Triple staggered talon micro-rip transients across the slash arc
            for (i, &(st, mul)) in [(0.016f32, 1.0f32), (0.044, 1.16), (0.072, 0.91)]
                .iter()
                .enumerate()
            {
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.048,
                    pink: false,
                    gain: 0.56,
                    attack_s: 0.0012,
                    decay_pow: 2.1,
                    filter: FilterMode::BandPass,
                    fc0: 2650.0 * p * mul,
                    fc1: 1050.0 * p * mul,
                    q: 2.1,
                    seed: base_seed.wrapping_add(i as u32 * 313),
                    ..Default::default()
                });
            }
            // Fierce mid-range swipe snarl body
            m.add_osc(OscLayer {
                start_s: 0.008,
                dur_s: 0.145,
                f0: 540.0 * p,
                f1: 175.0 * p,
                pitch_pow: 0.50,
                wave: Wave::Saw,
                gain: 0.52,
                attack_s: 0.004,
                decay_pow: 1.75,
                filter: FilterMode::LowPass,
                fc0: 2400.0,
                fc1: 520.0,
                q: 1.5,
                ..Default::default()
            });
            m.finish(0.56, 1.7, 1.5, 14.0)
        }

        // 38. CatRoar: towering half-health jaguar bark-roar with throat formants & sewer reflections.
        Sfx::CatRoar => {
            let mut m = MixBuf::new(0.920);
            // Fierce rising bark-attack surge (0..0.22s)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.240,
                f0: 112.0,
                f1: 215.0,
                pitch_pow: 0.65,
                wave: Wave::Saw,
                gain: 0.56,
                attack_s: 0.025,
                decay_pow: 1.1,
                filter: FilterMode::LowPass,
                fc0: 1850.0,
                fc1: 2600.0,
                q: 1.8,
                trem_hz: 30.0,
                trem_depth: 0.35,
                ..Default::default()
            });
            // Booming sustained roar core diving into guttural chest rumble
            for &(det, g) in &[(0.991f32, 0.52f32), (1.012, 0.52)] {
                m.add_osc(OscLayer {
                    start_s: 0.09,
                    dur_s: 0.78,
                    f0: 205.0 * det,
                    f1: 76.0 * det,
                    pitch_pow: 0.78,
                    wave: Wave::Saw,
                    gain: g,
                    attack_s: 0.045,
                    decay_pow: 1.35,
                    filter: FilterMode::LowPass,
                    fc0: 2100.0,
                    fc1: 310.0,
                    q: 1.9,
                    vib_hz: 6.5,
                    vib_depth: 0.022,
                    trem_hz: 26.0,
                    trem_depth: 0.42,
                });
            }
            // Dual vocal-tract roar formants (F1 chest + F2 jaw rasp)
            m.add_osc(OscLayer {
                start_s: 0.04,
                dur_s: 0.72,
                f0: 190.0,
                f1: 88.0,
                pitch_pow: 0.8,
                wave: Wave::Pulse(0.32),
                gain: 0.44,
                attack_s: 0.035,
                decay_pow: 1.45,
                filter: FilterMode::BandPass,
                fc0: 960.0,
                fc1: 420.0,
                q: 3.0,
                trem_hz: 28.0,
                trem_depth: 0.45,
                ..Default::default()
            });
            // Harsh FM throat rasp at roar apex
            m.add_fm(FmLayer {
                start_s: 0.03,
                dur_s: 0.48,
                c0: 185.0,
                c1: 95.0,
                pitch_pow: 0.75,
                mod_ratio: 1.38,
                mod_index0: 3.4,
                mod_index1: 0.6,
                mod_pow: 1.4,
                gain: 0.42,
                attack_s: 0.03,
                decay_pow: 1.5,
                filter: FilterMode::BandPass,
                fc0: 1200.0,
                fc1: 360.0,
                q: 1.6,
            });
            // Wide roaring breath turbulence
            m.add_noise(NoiseLayer {
                start_s: 0.02,
                dur_s: 0.84,
                pink: true,
                gain: 0.62,
                attack_s: 0.035,
                decay_pow: 1.45,
                filter: FilterMode::LowPass,
                fc0: 2800.0,
                fc1: 320.0,
                fc_pow: 0.65,
                q: 1.4,
                trem_hz: 26.0,
                trem_depth: 0.35,
                seed: base_seed,
            });
            m.add_short_tail(0.24, 1650.0);
            m.finish(0.80, 1.9, 2.5, 38.0)
        }

        // 39. CatHurt: sharp startled feline hurt yowl-hiss with downward pitch bend.
        Sfx::CatHurt => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.310);
            // Sharp upward startled spike into wailing downward yowl
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.085,
                f0: 390.0 * p,
                f1: 760.0 * p,
                pitch_pow: 0.55,
                wave: Wave::Saw,
                gain: 0.52,
                attack_s: 0.005,
                decay_pow: 0.9,
                filter: FilterMode::BandPass,
                fc0: 950.0 * p,
                fc1: 1850.0 * p,
                q: 3.0,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.055,
                dur_s: 0.240,
                f0: 740.0 * p,
                f1: 270.0 * p,
                pitch_pow: 0.85,
                wave: Wave::Saw,
                gain: 0.60,
                attack_s: 0.012,
                decay_pow: 1.55,
                filter: FilterMode::BandPass,
                fc0: 1750.0 * p,
                fc1: 620.0 * p,
                q: 3.2,
                vib_hz: 14.0,
                vib_depth: 0.032,
                ..Default::default()
            });
            // Lower chest formant resonance
            m.add_osc(OscLayer {
                start_s: 0.02,
                dur_s: 0.260,
                f0: 340.0 * p,
                f1: 165.0 * p,
                pitch_pow: 0.9,
                wave: Wave::Pulse(0.35),
                gain: 0.42,
                attack_s: 0.010,
                decay_pow: 1.6,
                filter: FilterMode::LowPass,
                fc0: 1400.0,
                fc1: 420.0,
                q: 1.4,
                ..Default::default()
            });
            // Hissed pain burst
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.160,
                pink: true,
                gain: 0.42,
                attack_s: 0.003,
                decay_pow: 1.85,
                filter: FilterMode::BandPass,
                fc0: 2600.0 * p,
                fc1: 1100.0 * p,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.62, 1.65, 1.8, 20.0)
        }

        // 40. CatDeath: collapsing jaguar death yowl-roar -> heavy body fall thud -> shuddering exhale.
        Sfx::CatDeath => {
            let mut m = MixBuf::new(0.980);
            // Collapsing agonized yowl-roar rolling directly into the body fall
            for &(det, g) in &[(0.992f32, 0.50f32), (1.015, 0.46)] {
                m.add_osc(OscLayer {
                    start_s: 0.0,
                    dur_s: 0.640,
                    f0: 360.0 * det,
                    f1: 64.0 * det,
                    pitch_pow: 0.72,
                    wave: Wave::Saw,
                    gain: g,
                    attack_s: 0.018,
                    decay_pow: 1.35,
                    filter: FilterMode::LowPass,
                    fc0: 2200.0,
                    fc1: 240.0,
                    q: 2.1,
                    vib_hz: 10.5,
                    vib_depth: 0.030,
                    trem_hz: 19.0,
                    trem_depth: 0.36,
                });
            }
            // Vocal formant descent
            m.add_osc(OscLayer {
                start_s: 0.01,
                dur_s: 0.540,
                f0: 580.0,
                f1: 140.0,
                pitch_pow: 0.75,
                wave: Wave::Pulse(0.30),
                gain: 0.36,
                attack_s: 0.015,
                decay_pow: 1.5,
                filter: FilterMode::BandPass,
                fc0: 1550.0,
                fc1: 380.0,
                q: 2.8,
                ..Default::default()
            });
            // Heavy defeated body collapse THUD landing seamlessly at 0.50s
            m.add_osc(OscLayer {
                start_s: 0.50,
                dur_s: 0.28,
                f0: 132.0,
                f1: 30.0,
                pitch_pow: 0.38,
                wave: Wave::Sine,
                gain: 0.94,
                attack_s: 0.003,
                decay_pow: 1.55,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.50,
                dur_s: 0.24,
                pink: true,
                gain: 0.64,
                attack_s: 0.003,
                decay_pow: 1.8,
                filter: FilterMode::LowPass,
                fc0: 1850.0,
                fc1: 160.0,
                fc_pow: 0.55,
                q: 1.2,
                seed: base_seed,
                ..Default::default()
            });
            // Final faint exhaling breath hiss
            m.add_noise(NoiseLayer {
                start_s: 0.61,
                dur_s: 0.34,
                pink: true,
                gain: 0.34,
                attack_s: 0.035,
                decay_pow: 1.50,
                filter: FilterMode::BandPass,
                fc0: 920.0,
                fc1: 340.0,
                q: 1.5,
                seed: base_seed ^ 0x5151,
                ..Default::default()
            });
            m.add_short_tail(0.22, 1500.0);
            m.finish(0.78, 1.8, 2.5, 42.0)
        }

        // ---- Human with broom boss (Kitchen) -----------------------------

        // 41. BroomStomp: weighty giant human slipper stomp + solid kitchen floorboard thud.
        Sfx::BroomStomp => {
            const PITCH: [f32; 4] = [1.0, 0.92, 1.06, 0.96];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.215);
            // Massive clean floor-shaking sub-thump (no phase beating in decay)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.195,
                f0: 132.0 * p,
                f1: 31.0 * p,
                pitch_pow: 0.36,
                wave: Wave::Sine,
                gain: 0.96,
                attack_s: 0.002,
                decay_pow: 1.50,
                ..Default::default()
            });
            // Brief hollow wooden kitchen floorboard knock confined to onset
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.056,
                f0: 215.0 * p,
                f1: 88.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Triangle,
                gain: 0.44,
                attack_s: 0.0015,
                decay_pow: 2.1,
                ..Default::default()
            });
            // Chunky leather slipper heel slap + woody floorboard body burst
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.075,
                pink: true,
                gain: 0.72,
                attack_s: 0.0012,
                decay_pow: 2.0,
                filter: FilterMode::LowPass,
                fc0: 1650.0 * p,
                fc1: 220.0 * p,
                fc_pow: 0.55,
                q: 1.2,
                seed: base_seed,
                ..Default::default()
            });
            // Subtle kitchenware tremor rattle on heavy stomp
            m.add_fm(FmLayer {
                start_s: 0.022,
                dur_s: 0.095,
                c0: 1380.0 * p,
                c1: 1290.0 * p,
                mod_ratio: 2.63,
                mod_index0: 1.15,
                mod_index1: 0.1,
                mod_pow: 2.2,
                gain: 0.18,
                attack_s: 0.004,
                decay_pow: 2.1,
                filter: FilterMode::BandPass,
                fc0: 2100.0,
                fc1: 1200.0,
                q: 1.5,
                ..Default::default()
            });
            m.add_short_tail(0.16, 1300.0);
            m.finish(0.68, 1.80, 1.5, 18.0)
        }

        // 42. BroomSweep: wide corn-straw broom bristle swish scraping across kitchen tile.
        Sfx::BroomSweep => {
            const PITCH: [f32; 3] = [1.0, 1.08, 0.93];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.360);
            // Leading sweep acceleration scrape
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.175,
                pink: true,
                gain: 0.74,
                attack_s: 0.024,
                decay_pow: 0.85,
                filter: FilterMode::BandPass,
                fc0: 760.0 * p,
                fc1: 2450.0 * p,
                fc_pow: 0.85,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            // Follow-through wide arc deceleration
            m.add_noise(NoiseLayer {
                start_s: 0.110,
                dur_s: 0.240,
                pink: true,
                gain: 0.70,
                attack_s: 0.020,
                decay_pow: 1.65,
                filter: FilterMode::BandPass,
                fc0: 2350.0 * p,
                fc1: 620.0 * p,
                fc_pow: 0.95,
                q: 1.6,
                seed: base_seed ^ 0x3377,
                ..Default::default()
            });
            // Dry corn-straw bristle crisp texture (kept cleanly below 5 kHz)
            m.add_noise(NoiseLayer {
                start_s: 0.030,
                dur_s: 0.245,
                pink: false,
                gain: 0.36,
                attack_s: 0.022,
                decay_pow: 1.5,
                filter: FilterMode::BandPass,
                fc0: 2300.0 * p,
                fc1: 3500.0 * p,
                fc_pow: 0.5,
                q: 2.4,
                trem_hz: 34.0,
                trem_depth: 0.28,
                seed: base_seed ^ 0x9A1B,
            });
            // Wooden broom air-whoosh low body
            m.add_noise(NoiseLayer {
                start_s: 0.010,
                dur_s: 0.300,
                pink: true,
                gain: 0.48,
                attack_s: 0.030,
                decay_pow: 1.35,
                filter: FilterMode::BandPass,
                fc0: 280.0 * p,
                fc1: 600.0 * p,
                q: 1.4,
                seed: base_seed ^ 0xCC01,
                ..Default::default()
            });
            m.finish(0.54, 1.55, 3.0, 24.0)
        }

        // 43. BroomJab: quick forward broom-handle poke + crisp corn-bristle stab impact.
        Sfx::BroomJab => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.150);
            // Quick air thrust into jab
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.036,
                pink: true,
                gain: 0.44,
                attack_s: 0.004,
                decay_pow: 0.9,
                filter: FilterMode::BandPass,
                fc0: 620.0 * p,
                fc1: 1950.0 * p,
                q: 1.8,
                seed: base_seed ^ 0x1010,
                ..Default::default()
            });
            // Wooden broom-head impact poke at 20ms
            m.add_osc(OscLayer {
                start_s: 0.020,
                dur_s: 0.072,
                f0: 295.0 * p,
                f1: 105.0 * p,
                pitch_pow: 0.45,
                wave: Wave::Sine,
                gain: 0.72,
                attack_s: 0.0015,
                decay_pow: 1.75,
                ..Default::default()
            });
            // Dry stiff straw-bristle crunch burst
            m.add_noise(NoiseLayer {
                start_s: 0.020,
                dur_s: 0.115,
                pink: true,
                gain: 0.68,
                attack_s: 0.0015,
                decay_pow: 1.95,
                filter: FilterMode::BandPass,
                fc0: 2250.0 * p,
                fc1: 780.0 * p,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.50, 1.55, 1.5, 12.0)
        }

        // 44. BroomDustpan: galvanized tin dustpan crash + 3 bouncing metallic tile clatters!
        Sfx::BroomDustpan => {
            let p = if v == 0 { 1.0 } else { 1.07 };
            let mut m = MixBuf::new(0.450);
            // Primary sheet-metal crash + 3 progressively faster/quieter rim bounces
            let hits: [(f32, f32, f32); 4] = [
                (0.000, 1.00, 0.175),
                (0.112, 0.64, 0.125),
                (0.210, 0.44, 0.105),
                (0.295, 0.28, 0.095),
            ];
            for (i, &(st, amp, dur)) in hits.iter().enumerate() {
                let det = 1.0 + i as f32 * 0.04;
                // Sheet-metal inharmonic FM clatter pair
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: dur,
                    c0: 540.0 * p * det,
                    c1: 515.0 * p * det,
                    pitch_pow: 0.5,
                    mod_ratio: 1.414,
                    mod_index0: 3.0 * amp,
                    mod_index1: 0.25,
                    mod_pow: 1.6,
                    gain: 0.58 * amp,
                    attack_s: 0.0012,
                    decay_pow: 1.75,
                    filter: FilterMode::BandPass,
                    fc0: 1950.0,
                    fc1: 980.0,
                    q: 1.2,
                });
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: dur * 0.85,
                    c0: 1120.0 * p * det,
                    c1: 1060.0 * p * det,
                    pitch_pow: 0.5,
                    mod_ratio: 2.76,
                    mod_index0: 2.3 * amp,
                    mod_index1: 0.15,
                    mod_pow: 1.9,
                    gain: 0.44 * amp,
                    attack_s: 0.001,
                    decay_pow: 1.9,
                    filter: FilterMode::LowPass,
                    fc0: 4400.0,
                    fc1: 1800.0,
                    q: 1.0,
                });
                // Crisp tile slap transient
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.036,
                    pink: false,
                    gain: 0.46 * amp,
                    attack_s: 0.0008,
                    decay_pow: 2.4,
                    filter: FilterMode::BandPass,
                    fc0: 2700.0 * p,
                    fc1: 1100.0 * p,
                    q: 1.5,
                    seed: base_seed.wrapping_add(i as u32 * 701),
                    ..Default::default()
                });
            }
            // Initial tile floor impact body
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.095,
                f0: 210.0 * p,
                f1: 72.0 * p,
                pitch_pow: 0.45,
                wave: Wave::Sine,
                gain: 0.58,
                attack_s: 0.0015,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.add_short_tail(0.20, 2600.0);
            m.finish(0.68, 1.80, 1.4, 24.0)
        }

        // 45. BroomHurt: expressive comical annoyed human yelp ("YIPE! / OW!") via vocal formants.
        Sfx::BroomHurt => {
            const F_PEAK: [f32; 3] = [560.0, 620.0, 505.0];
            const F_END: [f32; 3] = [310.0, 355.0, 275.0];
            let fp = F_PEAK[v];
            let fe = F_END[v];
            let mut m = MixBuf::new(0.260);
            // Quick startled pitch jump upward ("Y-I-I...")
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.088,
                f0: fp * 0.46,
                f1: fp,
                pitch_pow: 0.55,
                wave: Wave::Saw,
                gain: 0.58,
                attack_s: 0.006,
                decay_pow: 0.85,
                filter: FilterMode::BandPass,
                fc0: 650.0,
                fc1: 1550.0,
                q: 3.4,
                ..Default::default()
            });
            // Indignant descending vocal body ("...OW!") with twin human formants (F1 + F2)
            m.add_osc(OscLayer {
                start_s: 0.058,
                dur_s: 0.190,
                f0: fp,
                f1: fe,
                pitch_pow: 0.85,
                wave: Wave::Saw,
                gain: 0.62,
                attack_s: 0.010,
                decay_pow: 1.5,
                filter: FilterMode::BandPass,
                fc0: 760.0,
                fc1: 520.0,
                q: 3.5,
                vib_hz: 12.5,
                vib_depth: 0.025,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.058,
                dur_s: 0.180,
                f0: fp,
                f1: fe,
                pitch_pow: 0.85,
                wave: Wave::Pulse(0.36),
                gain: 0.46,
                attack_s: 0.010,
                decay_pow: 1.6,
                filter: FilterMode::BandPass,
                fc0: 1620.0,
                fc1: 1120.0,
                q: 3.8,
                vib_hz: 12.5,
                vib_depth: 0.025,
                ..Default::default()
            });
            // Short breathy startled aspiration
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.075,
                pink: true,
                gain: 0.30,
                attack_s: 0.003,
                decay_pow: 1.9,
                filter: FilterMode::BandPass,
                fc0: 1950.0,
                fc1: 1200.0,
                q: 1.8,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.60, 1.5, 2.0, 18.0)
        }

        // 46. BroomHop: light bouncy one-foot hurt-hop thump + comedic slipper spring squeak.
        Sfx::BroomHop => {
            const PITCH: [f32; 3] = [1.0, 1.10, 0.91];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.170);
            // Snappy springy upward one-foot hop bounce (cleanly clears before landing)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.038,
                f0: 175.0 * p,
                f1: 310.0 * p,
                pitch_pow: 0.65,
                wave: Wave::Triangle,
                gain: 0.44,
                attack_s: 0.003,
                decay_pow: 1.5,
                ..Default::default()
            });
            // Lighter single-slipper landing pad thump (clearly lighter than BroomStomp)
            m.add_osc(OscLayer {
                start_s: 0.034,
                dur_s: 0.120,
                f0: 172.0 * p,
                f1: 60.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Sine,
                gain: 0.78,
                attack_s: 0.002,
                decay_pow: 1.75,
                ..Default::default()
            });
            // Soft slipper-sole scuff on tile
            m.add_noise(NoiseLayer {
                start_s: 0.032,
                dur_s: 0.064,
                pink: true,
                gain: 0.48,
                attack_s: 0.002,
                decay_pow: 2.0,
                filter: FilterMode::BandPass,
                fc0: 1400.0 * p,
                fc1: 520.0 * p,
                q: 1.7,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.46, 1.45, 1.8, 14.0)
        }

        // 47. BroomDefeat: indignant grumpy huff + 5 brisk retreating footsteps fading away + distant door shut!
        Sfx::BroomDefeat => {
            let mut m = MixBuf::new(1.250);
            // Opening annoyed indignant vocal "hmph!"
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.160,
                f0: 325.0,
                f1: 162.0,
                pitch_pow: 0.75,
                wave: Wave::Saw,
                gain: 0.56,
                attack_s: 0.008,
                decay_pow: 1.5,
                filter: FilterMode::BandPass,
                fc0: 880.0,
                fc1: 420.0,
                q: 3.0,
                ..Default::default()
            });
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.120,
                pink: true,
                gain: 0.34,
                attack_s: 0.006,
                decay_pow: 1.8,
                filter: FilterMode::BandPass,
                fc0: 1450.0,
                fc1: 680.0,
                q: 1.6,
                seed: base_seed,
                ..Default::default()
            });
            // Five hurrying retreating footsteps fading in level and darkening with distance
            let steps: [(f32, f32, f32, f32); 5] = [
                (0.165, 0.90, 1.00, 1450.0),
                (0.355, 0.74, 0.94, 1180.0),
                (0.535, 0.58, 1.03, 940.0),
                (0.705, 0.44, 0.95, 740.0),
                (0.865, 0.32, 1.01, 580.0),
            ];
            for (i, &(st, amp, pmul, fc)) in steps.iter().enumerate() {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.135,
                    f0: 132.0 * pmul,
                    f1: 38.0 * pmul,
                    pitch_pow: 0.42,
                    wave: Wave::Sine,
                    gain: 0.84 * amp,
                    attack_s: 0.002,
                    decay_pow: 1.65,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.050,
                    pink: true,
                    gain: 0.52 * amp,
                    attack_s: 0.0015,
                    decay_pow: 2.1,
                    filter: FilterMode::BandPass,
                    fc0: fc,
                    fc1: fc * 0.35,
                    q: 1.4,
                    seed: base_seed.wrapping_add((i as u32 + 1) * 521),
                    ..Default::default()
                });
            }
            // Distant indignant kitchen door shut clack-thud at 1.04s (softer than the footsteps)
            m.add_osc(OscLayer {
                start_s: 1.04,
                dur_s: 0.14,
                f0: 112.0,
                f1: 42.0,
                pitch_pow: 0.45,
                wave: Wave::Sine,
                gain: 0.26,
                attack_s: 0.002,
                decay_pow: 1.75,
                ..Default::default()
            });
            m.add_fm(FmLayer {
                start_s: 1.05,
                dur_s: 0.065,
                c0: 820.0,
                c1: 500.0,
                mod_ratio: 2.1,
                mod_index0: 1.4,
                mod_index1: 0.1,
                mod_pow: 2.2,
                gain: 0.18,
                attack_s: 0.0015,
                decay_pow: 2.1,
                filter: FilterMode::LowPass,
                fc0: 1300.0,
                fc1: 520.0,
                q: 1.0,
                ..Default::default()
            });
            m.add_short_tail(0.20, 1400.0);
            m.finish(0.66, 1.55, 2.0, 36.0)
        }

        // ---- Robot lawnmower boss (Lab) ----------------------------------

        // 48. MowerIdle: smooth loop-able brushless DC motor hum + gentle rotary deck whirr.
        Sfx::MowerIdle => {
            let p = if v == 0 { 1.0 } else { 1.045 };
            let mut m = MixBuf::new(0.480);
            // Smooth electric motor fundamental & 2nd harmonic
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.480,
                f0: 118.0 * p,
                f1: 120.0 * p,
                wave: Wave::Triangle,
                gain: 0.62,
                attack_s: 0.085,
                decay_pow: 0.60,
                trem_hz: 28.0,
                trem_depth: 0.18,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.480,
                f0: 236.0 * p,
                f1: 238.0 * p,
                wave: Wave::Pulse(0.30),
                gain: 0.34,
                attack_s: 0.085,
                decay_pow: 0.60,
                filter: FilterMode::LowPass,
                fc0: 680.0,
                fc1: 680.0,
                q: 1.2,
                trem_hz: 28.0,
                trem_depth: 0.28,
                ..Default::default()
            });
            // Subtle under-deck rotary blade air whirr
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.480,
                pink: true,
                gain: 0.32,
                attack_s: 0.090,
                decay_pow: 0.60,
                filter: FilterMode::BandPass,
                fc0: 540.0 * p,
                fc1: 540.0 * p,
                q: 2.5,
                trem_hz: 28.0,
                trem_depth: 0.35,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.28, 1.2, 46.0, 68.0)
        }

        // 49. MowerScan: crisp robotic lidar scan chirp + telemetry double-blip.
        Sfx::MowerScan => {
            const PITCH: [f32; 3] = [1.0, 1.08, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.185);
            // Sweeping optical lidar sonar ping
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.082,
                c0: 1120.0 * p,
                c1: 2320.0 * p,
                pitch_pow: 0.72,
                mod_ratio: 2.0,
                mod_index0: 0.9,
                mod_index1: 0.1,
                mod_pow: 2.0,
                gain: 0.56,
                attack_s: 0.003,
                decay_pow: 1.65,
                filter: FilterMode::LowPass,
                fc0: 4800.0,
                fc1: 3200.0,
                q: 0.9,
            });
            // Quick radar return telemetry blip at 94ms with optical FM shimmer
            m.add_fm(FmLayer {
                start_s: 0.094,
                dur_s: 0.082,
                c0: 1940.0 * p,
                c1: 1640.0 * p,
                pitch_pow: 0.8,
                mod_ratio: 1.5,
                mod_index0: 0.65,
                mod_index1: 0.05,
                mod_pow: 2.2,
                gain: 0.56,
                attack_s: 0.0025,
                decay_pow: 1.75,
                filter: FilterMode::LowPass,
                fc0: 4600.0,
                fc1: 2400.0,
                q: 0.9,
            });
            // Tiny gimbal servo tick underneath
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.040,
                f0: 420.0 * p,
                f1: 580.0 * p,
                wave: Wave::Triangle,
                gain: 0.24,
                attack_s: 0.002,
                decay_pow: 1.9,
                ..Default::default()
            });
            m.finish(0.40, 1.15, 1.5, 12.0)
        }

        // 50. MowerLockOn: urgent rising twin-pulse laser target lock-on tone!
        Sfx::MowerLockOn => {
            let p = if v == 0 { 1.0 } else { 1.06 };
            let mut m = MixBuf::new(0.280);
            // Pulse 1: warning acquisition tone (B5)
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.078,
                f0: 987.77 * p,
                f1: 987.77 * p,
                wave: Wave::Pulse(0.40),
                gain: 0.48,
                attack_s: 0.002,
                decay_pow: 1.35,
                filter: FilterMode::LowPass,
                fc0: 3600.0,
                fc1: 2200.0,
                q: 1.0,
                ..Default::default()
            });
            // Pulse 2: sharp locked fifth-up confirmation (F#6) with crisp lock shimmer
            m.add_osc(OscLayer {
                start_s: 0.096,
                dur_s: 0.170,
                f0: 1479.98 * p,
                f1: 1479.98 * p,
                wave: Wave::Pulse(0.35),
                gain: 0.54,
                attack_s: 0.002,
                decay_pow: 1.45,
                filter: FilterMode::LowPass,
                fc0: 4400.0,
                fc1: 2000.0,
                q: 1.1,
                trem_hz: 22.0,
                trem_depth: 0.28,
                ..Default::default()
            });
            m.add_fm(FmLayer {
                start_s: 0.096,
                dur_s: 0.140,
                c0: 1479.98 * p,
                c1: 1479.98 * p,
                mod_ratio: 1.5,
                mod_index0: 1.1,
                mod_index1: 0.08,
                mod_pow: 2.2,
                gain: 0.34,
                attack_s: 0.002,
                decay_pow: 1.8,
                ..Default::default()
            });
            // Subtle bass lock click
            m.add_osc(OscLayer {
                start_s: 0.096,
                dur_s: 0.045,
                f0: 370.0 * p,
                f1: 240.0 * p,
                wave: Wave::Triangle,
                gain: 0.34,
                attack_s: 0.002,
                decay_pow: 1.8,
                ..Default::default()
            });
            m.finish(0.52, 1.25, 1.5, 16.0)
        }

        // 51. MowerCharge: high-torque electric motor rev-up swelling into rubber tire peel-out!
        Sfx::MowerCharge => {
            const PITCH: [f32; 3] = [1.0, 1.07, 0.93];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.440);
            // Detuned high-torque drive motor accelerating upward with forward surge
            for &(det, g) in &[(0.994f32, 0.50f32), (1.008, 0.46)] {
                m.add_osc(OscLayer {
                    start_s: 0.0,
                    dur_s: 0.420,
                    f0: 138.0 * p * det,
                    f1: 560.0 * p * det,
                    pitch_pow: 0.72,
                    wave: Wave::Saw,
                    gain: g,
                    attack_s: 0.055,
                    decay_pow: 0.68,
                    filter: FilterMode::LowPass,
                    fc0: 760.0,
                    fc1: 3500.0,
                    q: 1.7,
                    trem_hz: 36.0,
                    trem_depth: 0.25,
                    ..Default::default()
                });
            }
            // Electric turbine whine harmonic swelling toward launch speed
            m.add_fm(FmLayer {
                start_s: 0.015,
                dur_s: 0.405,
                c0: 276.0 * p,
                c1: 1120.0 * p,
                pitch_pow: 0.75,
                mod_ratio: 1.5,
                mod_index0: 1.3,
                mod_index1: 2.3,
                mod_pow: 0.75,
                gain: 0.42,
                attack_s: 0.065,
                decay_pow: 0.78,
                filter: FilterMode::BandPass,
                fc0: 900.0,
                fc1: 3200.0,
                q: 1.8,
            });
            // Rubber tire peel-out squeal on lab tile
            m.add_noise(NoiseLayer {
                start_s: 0.045,
                dur_s: 0.220,
                pink: true,
                gain: 0.44,
                attack_s: 0.022,
                decay_pow: 1.35,
                filter: FilterMode::BandPass,
                fc0: 1500.0 * p,
                fc1: 2650.0 * p,
                fc_pow: 0.8,
                q: 3.3,
                seed: base_seed,
                ..Default::default()
            });
            m.finish(0.66, 1.7, 3.0, 26.0)
        }

        // 52. MowerWallClang: heavy bumper-wall smash + resonant steel chassis gong + bounce!
        Sfx::MowerWallClang => {
            const PITCH: [f32; 3] = [1.0, 0.92, 1.08];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.420);
            // Massive rubber-bumper impact sub thud
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.195,
                f0: 158.0 * p,
                f1: 36.0 * p,
                pitch_pow: 0.40,
                wave: Wave::Sine,
                gain: 0.90,
                attack_s: 0.0015,
                decay_pow: 1.55,
                ..Default::default()
            });
            // Metallic crash burst
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.130,
                pink: false,
                gain: 0.66,
                attack_s: 0.001,
                decay_pow: 2.1,
                filter: FilterMode::BandPass,
                fc0: 2600.0 * p,
                fc1: 680.0 * p,
                q: 1.3,
                seed: base_seed,
                ..Default::default()
            });
            // Three inharmonic steel-shell gong partials
            for &(cf, ratio, dur, g) in &[
                (340.0f32, 1.414f32, 0.390f32, 0.56f32),
                (615.0, 2.18, 0.330, 0.42),
                (1080.0, 1.732, 0.240, 0.30),
            ] {
                m.add_fm(FmLayer {
                    start_s: 0.002,
                    dur_s: dur,
                    c0: cf * p,
                    c1: cf * p * 0.96,
                    pitch_pow: 0.6,
                    mod_ratio: ratio,
                    mod_index0: 3.0,
                    mod_index1: 0.20,
                    mod_pow: 1.6,
                    gain: g,
                    attack_s: 0.0015,
                    decay_pow: 1.65,
                    filter: FilterMode::LowPass,
                    fc0: 4200.0,
                    fc1: 1100.0,
                    q: 1.1,
                });
            }
            // Springy rubber-bumper rebound bounce at 92ms
            m.add_osc(OscLayer {
                start_s: 0.092,
                dur_s: 0.088,
                f0: 140.0 * p,
                f1: 68.0 * p,
                pitch_pow: 0.5,
                wave: Wave::Triangle,
                gain: 0.38,
                attack_s: 0.004,
                decay_pow: 1.8,
                ..Default::default()
            });
            m.add_short_tail(0.20, 2200.0);
            m.finish(0.76, 1.85, 1.5, 28.0)
        }

        // 53. MowerFlipped: comic helpless belly-up free-spinning wheel whirr + shell wobble clunks!
        Sfx::MowerFlipped => {
            const PITCH: [f32; 3] = [1.0, 1.09, 0.92];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.460);
            // Warbling struggling free-spinning wheel servo
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.440,
                f0: 345.0 * p,
                f1: 270.0 * p,
                pitch_pow: 1.0,
                wave: Wave::Pulse(0.32),
                gain: 0.48,
                attack_s: 0.018,
                decay_pow: 1.05,
                filter: FilterMode::BandPass,
                fc0: 980.0 * p,
                fc1: 660.0 * p,
                q: 2.0,
                vib_hz: 7.5,
                vib_depth: 0.11,
                trem_hz: 19.0,
                trem_depth: 0.42,
            });
            // Sputtering secondary wheel whine
            m.add_osc(OscLayer {
                start_s: 0.01,
                dur_s: 0.420,
                f0: 510.0 * p,
                f1: 390.0 * p,
                wave: Wave::Triangle,
                gain: 0.34,
                attack_s: 0.020,
                decay_pow: 1.15,
                vib_hz: 7.5,
                vib_depth: 0.12,
                trem_hz: 15.0,
                trem_depth: 0.35,
                ..Default::default()
            });
            // Hollow rocking plastic/steel shell clunks at 135ms and 295ms
            for (i, &st) in [0.135f32, 0.295].iter().enumerate() {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.062,
                    f0: (240.0 - i as f32 * 30.0) * p,
                    f1: 92.0 * p,
                    pitch_pow: 0.45,
                    wave: Wave::Sine,
                    gain: 0.48,
                    attack_s: 0.002,
                    decay_pow: 1.9,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.028,
                    pink: true,
                    gain: 0.30,
                    attack_s: 0.001,
                    decay_pow: 2.2,
                    filter: FilterMode::BandPass,
                    fc0: 1400.0 * p,
                    fc1: 640.0 * p,
                    q: 1.9,
                    seed: base_seed.wrapping_add(i as u32 * 811),
                    ..Default::default()
                });
            }
            m.finish(0.44, 1.35, 4.0, 24.0)
        }

        // 54. MowerBladeSpin: rising rotary steel blade turbine whine + choppy air-slice!
        Sfx::MowerBladeSpin => {
            let p = if v == 0 { 1.0 } else { 1.07 };
            let mut m = MixBuf::new(0.480);
            // Rising steel blade turbine spin-up
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.460,
                c0: 185.0 * p,
                c1: 860.0 * p,
                pitch_pow: 0.78,
                mod_ratio: 2.0,
                mod_index0: 1.2,
                mod_index1: 2.4,
                mod_pow: 0.7,
                gain: 0.54,
                attack_s: 0.022,
                decay_pow: 0.90,
                filter: FilterMode::LowPass,
                fc0: 950.0,
                fc1: 4200.0,
                q: 1.5,
            });
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.460,
                f0: 92.5 * p,
                f1: 430.0 * p,
                pitch_pow: 0.78,
                wave: Wave::Saw,
                gain: 0.42,
                attack_s: 0.024,
                decay_pow: 0.95,
                filter: FilterMode::LowPass,
                fc0: 680.0,
                fc1: 2800.0,
                q: 1.6,
                trem_hz: 48.0,
                trem_depth: 0.45,
                ..Default::default()
            });
            // Signature high-speed chopping rotary blade air-slice
            m.add_noise(NoiseLayer {
                start_s: 0.020,
                dur_s: 0.440,
                pink: true,
                gain: 0.60,
                attack_s: 0.045,
                decay_pow: 0.95,
                filter: FilterMode::BandPass,
                fc0: 620.0 * p,
                fc1: 2850.0 * p,
                fc_pow: 0.85,
                q: 2.3,
                trem_hz: 48.0,
                trem_depth: 0.65,
                seed: base_seed,
            });
            m.finish(0.58, 1.55, 3.0, 26.0)
        }

        // 55. MowerOverheat: pressurized valve pop + dense billowing steam hiss + thermal sag!
        Sfx::MowerOverheat => {
            let p = if v == 0 { 1.0 } else { 0.93 };
            let mut m = MixBuf::new(0.540);
            // Pressure relief valve pop-clink at t=0
            m.add_fm(FmLayer {
                start_s: 0.002,
                dur_s: 0.034,
                c0: 1260.0 * p,
                c1: 880.0 * p,
                mod_ratio: 2.5,
                mod_index0: 1.8,
                mod_index1: 0.1,
                mod_pow: 2.3,
                gain: 0.42,
                attack_s: 0.001,
                decay_pow: 2.2,
                ..Default::default()
            });
            // Primary pressurized steam hiss (dense, ear-friendly warmth below 5 kHz)
            m.add_noise(NoiseLayer {
                start_s: 0.006,
                dur_s: 0.515,
                pink: true,
                gain: 0.82,
                attack_s: 0.016,
                decay_pow: 1.25,
                filter: FilterMode::BandPass,
                fc0: 2400.0 * p,
                fc1: 960.0 * p,
                fc_pow: 0.75,
                q: 1.2,
                trem_hz: 15.0,
                trem_depth: 0.22,
                seed: base_seed,
            });
            // Kettle-like high-Q steam whistle harmonic
            m.add_noise(NoiseLayer {
                start_s: 0.012,
                dur_s: 0.430,
                pink: false,
                gain: 0.30,
                attack_s: 0.022,
                decay_pow: 1.45,
                filter: FilterMode::BandPass,
                fc0: 3150.0 * p,
                fc1: 1850.0 * p,
                fc_pow: 0.8,
                q: 3.6,
                trem_hz: 11.0,
                trem_depth: 0.20,
                seed: base_seed ^ 0x4E4E,
            });
            // Thermal power-sag descending whine underneath
            m.add_osc(OscLayer {
                start_s: 0.008,
                dur_s: 0.440,
                f0: 460.0 * p,
                f1: 132.0 * p,
                pitch_pow: 0.85,
                wave: Wave::Triangle,
                gain: 0.48,
                attack_s: 0.018,
                decay_pow: 1.4,
                ..Default::default()
            });
            m.finish(0.56, 1.55, 2.0, 32.0)
        }

        // 56. MowerDockHum: magnetic dock coupler ping + swelling pulsing inductive recharge hum.
        Sfx::MowerDockHum => {
            let p = if v == 0 { 1.0 } else { 1.059 }; // A or Bb harmonic shift
            let mut m = MixBuf::new(0.520);
            // Clean inductive dock-latch confirmation fifth
            for &(f, st, g) in &[(440.0f32, 0.0f32, 0.34f32), (659.25, 0.036, 0.32)] {
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: 0.145,
                    c0: f * p,
                    c1: f * p,
                    mod_ratio: 2.0,
                    mod_index0: 0.9,
                    mod_index1: 0.05,
                    mod_pow: 2.2,
                    gain: g,
                    attack_s: 0.004,
                    decay_pow: 1.8,
                    ..Default::default()
                });
            }
            // Swelling pulsing inductive power-transfer chord hum
            for &(f, g) in &[(220.0f32, 0.48f32), (330.0, 0.38), (440.0, 0.26)] {
                m.add_osc(OscLayer {
                    start_s: 0.02,
                    dur_s: 0.480,
                    f0: f * p,
                    f1: f * p * 1.08,
                    pitch_pow: 1.0,
                    wave: Wave::Triangle,
                    gain: g,
                    attack_s: 0.095,
                    decay_pow: 0.72,
                    trem_hz: 7.0,
                    trem_depth: 0.34,
                    ..Default::default()
                });
            }
            // Gentle high-tech transformer shimmer
            m.add_osc(OscLayer {
                start_s: 0.04,
                dur_s: 0.440,
                f0: 880.0 * p,
                f1: 950.0 * p,
                wave: Wave::Sine,
                gain: 0.20,
                attack_s: 0.090,
                decay_pow: 0.95,
                trem_hz: 14.0,
                trem_depth: 0.40,
                ..Default::default()
            });
            m.add_short_tail(0.16, 3000.0);
            m.finish(0.44, 1.25, 6.0, 38.0)
        }

        // 57. MowerDeath: short-circuit arc crackle -> cascading chassis explosions + metallic shrapnel -> power-down!
        Sfx::MowerDeath => {
            let mut m = MixBuf::new(1.220);
            // 0.00..0.24s frantic short-circuit electrical sputter + dying motor dive
            m.add_fm(FmLayer {
                start_s: 0.0,
                dur_s: 0.240,
                c0: 860.0,
                c1: 140.0,
                pitch_pow: 0.65,
                mod_ratio: 2.71,
                mod_index0: 3.2,
                mod_index1: 1.2,
                mod_pow: 1.2,
                gain: 0.50,
                attack_s: 0.004,
                decay_pow: 1.2,
                filter: FilterMode::BandPass,
                fc0: 2400.0,
                fc1: 750.0,
                q: 1.5,
            });
            for (i, &st) in [0.00f32, 0.065, 0.135].iter().enumerate() {
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.036,
                    pink: false,
                    gain: 0.52,
                    attack_s: 0.001,
                    decay_pow: 2.3,
                    filter: FilterMode::BandPass,
                    fc0: 3200.0 - i as f32 * 400.0,
                    fc1: 1200.0,
                    q: 1.8,
                    seed: base_seed.wrapping_add(i as u32 * 997),
                    ..Default::default()
                });
            }
            // Escalating twin mechanical chassis detonations at 0.21s and 0.42s
            for (i, &(st, sub_f, scale)) in [(0.21f32, 150.0f32, 0.78f32), (0.42, 185.0, 1.00)]
                .iter()
                .enumerate()
            {
                let big = i == 1;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: if big { 0.56 } else { 0.28 },
                    f0: sub_f,
                    f1: 26.0,
                    pitch_pow: 0.38,
                    wave: Wave::Sine,
                    gain: 0.94 * scale,
                    attack_s: 0.002,
                    decay_pow: 1.45,
                    ..Default::default()
                });
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: if big { 0.72 } else { 0.26 },
                    pink: true,
                    gain: 0.86 * scale,
                    attack_s: 0.003,
                    decay_pow: 1.55,
                    filter: FilterMode::LowPass,
                    fc0: if big { 5200.0 } else { 3200.0 },
                    fc1: 125.0,
                    fc_pow: 0.50,
                    q: 1.4,
                    seed: base_seed.wrapping_add((i as u32 + 1) * 4111),
                    ..Default::default()
                });
            }
            // Flying metallic hull shrapnel clangs
            for &(st, cf) in &[(0.46f32, 680.0f32), (0.63, 940.0)] {
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: 0.28,
                    c0: cf,
                    c1: cf * 0.94,
                    mod_ratio: 1.73,
                    mod_index0: 2.6,
                    mod_index1: 0.15,
                    mod_pow: 1.8,
                    gain: 0.34,
                    attack_s: 0.002,
                    decay_pow: 1.85,
                    filter: FilterMode::LowPass,
                    fc0: 3600.0,
                    fc1: 1100.0,
                    q: 1.0,
                    ..Default::default()
                });
            }
            // Classic robotic system-shutdown descending power-off whine clearing the smoke
            m.add_osc(OscLayer {
                start_s: 0.56,
                dur_s: 0.63,
                f0: 960.0,
                f1: 70.0,
                pitch_pow: 0.82,
                wave: Wave::Triangle,
                gain: 0.50,
                attack_s: 0.022,
                decay_pow: 1.35,
                ..Default::default()
            });
            m.add_short_tail(0.24, 2000.0);
            m.finish(0.82, 1.9, 2.0, 48.0)
        }

        // ---- Campaign progression fanfares -------------------------------

        // 58. LevelComplete: joyful legato D-major stage-clear fanfare with walking bass, harmony & chime.
        Sfx::LevelComplete => {
            let mut m = MixBuf::new(1.320);
            // Bright legato D-major stage-clear melody: D5-F#5-A5 | B5-A5-F#5-A5 | D6!
            let melody: [(f32, f32, f32, f32); 8] = [
                (0.000, 587.33, 0.175, 146.83), // D5 + D3
                (0.085, 739.99, 0.175, 146.83), // F#5
                (0.170, 880.00, 0.185, 220.00), // A5 + A3
                (0.265, 987.77, 0.210, 196.00), // B5 + G3
                (0.375, 880.00, 0.195, 220.00), // A5 + A3
                (0.475, 739.99, 0.195, 185.00), // F#5 + F#3
                (0.575, 880.00, 0.210, 220.00), // A5 + A3
                (0.695, 1174.66, 0.59, 146.83), // D6 triumph hold + D3 anchor
            ];
            for (i, &(st, f, dur, bf)) in melody.iter().enumerate() {
                let last = i == 7;
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: f,
                    f1: f,
                    wave: Wave::Pulse(0.36),
                    gain: if last { 0.34 } else { 0.35 },
                    attack_s: 0.005,
                    decay_pow: if last { 1.25 } else { 1.18 },
                    filter: FilterMode::LowPass,
                    fc0: 4400.0,
                    fc1: 1900.0,
                    q: 1.0,
                    vib_hz: if last { 5.8 } else { 0.0 },
                    vib_depth: if last { 0.011 } else { 0.0 },
                    ..Default::default()
                });
                // Warm legato walking bass support
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur * 1.18,
                    f0: bf,
                    f1: bf,
                    wave: Wave::Triangle,
                    gain: if last { 0.30 } else { 0.25 },
                    attack_s: 0.008,
                    decay_pow: 1.12,
                    ..Default::default()
                });
                // Bright glockenspiel FM sparkle on strong beats
                if i == 0 || i == 3 || last {
                    m.add_fm(FmLayer {
                        start_s: st,
                        dur_s: dur * 0.9,
                        c0: f * 2.0,
                        c1: f * 2.0,
                        mod_ratio: 2.0,
                        mod_index0: 0.95,
                        mod_index1: 0.05,
                        mod_pow: 2.5,
                        gain: if last { 0.20 } else { 0.15 },
                        attack_s: 0.003,
                        decay_pow: 2.0,
                        filter: FilterMode::LowPass,
                        fc0: 5600.0,
                        fc1: 2400.0,
                        q: 0.85,
                        ..Default::default()
                    });
                }
            }
            // Celebratory harmony triad bloom on the final D6 hold (F#5 + A5 + D5)
            for &(f, g) in &[(587.33f32, 0.18f32), (739.99, 0.19), (880.00, 0.19)] {
                m.add_osc(OscLayer {
                    start_s: 0.695,
                    dur_s: 0.58,
                    f0: f,
                    f1: f,
                    wave: Wave::Triangle,
                    gain: g,
                    attack_s: 0.020,
                    decay_pow: 1.35,
                    vib_hz: 5.8,
                    vib_depth: 0.007,
                    ..Default::default()
                });
            }
            // Crisp upbeat celebratory snare-tick accents on phrase landings
            for &st in &[0.265f32, 0.695] {
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.065,
                    pink: true,
                    gain: 0.22,
                    attack_s: 0.0015,
                    decay_pow: 2.2,
                    filter: FilterMode::BandPass,
                    fc0: 2600.0,
                    fc1: 1100.0,
                    q: 1.4,
                    seed: base_seed ^ st.to_bits(),
                    ..Default::default()
                });
            }
            m.add_short_tail(0.24, 3400.0);
            m.finish(0.68, 1.35, 2.0, 48.0)
        }

        // 59. CampaignVictory: grand, jubilant 2.18s multi-phrase legato victory anthem —
        // longer, fuller, and happier than Sfx::Win (1.06s)!
        Sfx::CampaignVictory => {
            let mut m = MixBuf::new(2.180);
            // 3-phrase joyful C-major -> F-major -> G7 -> C-major9 anthem (overlapping legato)
            let melody: [(f32, f32, f32, f32, f32); 12] = [
                // Phrase 1: upbeat herald fanfare
                (0.000, 392.00, 0.165, 196.00, 523.25), // G4 pickup
                (0.095, 523.25, 0.185, 130.81, 659.25), // C5 + C3
                (0.190, 659.25, 0.185, 164.81, 783.99), // E5 + E3
                (0.285, 783.99, 0.230, 196.00, 1046.50), // G5 + G3
                (0.415, 659.25, 0.175, 164.81, 783.99), // E5
                (0.515, 783.99, 0.200, 196.00, 1046.50), // G5
                // Phrase 2: radiant F-major lift -> G dominant climb
                (0.635, 880.00, 0.260, 174.61, 1046.50), // A5 + F3 (warm IV lift!)
                (0.795, 783.99, 0.205, 174.61, 987.77),  // G5
                (0.910, 698.46, 0.205, 174.61, 880.00),  // F5
                (1.025, 783.99, 0.205, 196.00, 987.77),  // G5 + G3
                (1.140, 987.77, 0.220, 196.00, 1174.66), // B5 + G3 (triumphant dominant!)
                // Phrase 3: soaring grand finale C6 hold
                (1.275, 1046.50, 0.87, 130.81, 1318.51), // C6 + C3 + E6 harmony!
            ];
            for (i, &(st, f, dur, bf, hf)) in melody.iter().enumerate() {
                let last = i == 11;
                // Hero brassy chiptune lead
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur,
                    f0: f,
                    f1: f,
                    wave: Wave::Pulse(0.38),
                    gain: if last { 0.34 } else { 0.33 },
                    attack_s: 0.005,
                    decay_pow: if last { 1.20 } else { 1.15 },
                    filter: FilterMode::LowPass,
                    fc0: 4600.0,
                    fc1: 2100.0,
                    q: 1.0,
                    vib_hz: if last { 5.6 } else { 0.0 },
                    vib_depth: if last { 0.011 } else { 0.0 },
                    ..Default::default()
                });
                // Joyful upper-third/sixth harmony voice on key notes
                if i == 3 || i == 6 || i >= 9 {
                    m.add_osc(OscLayer {
                        start_s: st,
                        dur_s: dur * 0.98,
                        f0: hf,
                        f1: hf,
                        wave: Wave::Triangle,
                        gain: if last { 0.25 } else { 0.18 },
                        attack_s: 0.010,
                        decay_pow: 1.25,
                        vib_hz: if last { 5.6 } else { 0.0 },
                        vib_depth: if last { 0.008 } else { 0.0 },
                        ..Default::default()
                    });
                }
                // Warm legato bouncy bass foundation
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: dur * 1.20,
                    f0: bf,
                    f1: bf,
                    wave: Wave::Triangle,
                    gain: if last { 0.30 } else { 0.24 },
                    attack_s: 0.008,
                    decay_pow: 1.10,
                    ..Default::default()
                });
                // Sparkling celebratory bell accent on phrase peaks
                if i == 1 || i == 3 || i == 6 || i == 10 || last {
                    m.add_fm(FmLayer {
                        start_s: st,
                        dur_s: dur * 0.88,
                        c0: f * 2.0,
                        c1: f * 2.0,
                        mod_ratio: 2.0,
                        mod_index0: 0.90,
                        mod_index1: 0.05,
                        mod_pow: 2.5,
                        gain: if last { 0.20 } else { 0.14 },
                        attack_s: 0.003,
                        decay_pow: 1.95,
                        filter: FilterMode::LowPass,
                        fc0: 5600.0,
                        fc1: 2400.0,
                        q: 0.85,
                        ..Default::default()
                    });
                }
            }
            // Lush grand finale C-major9 chord bloom (C4, E4, G4, D5, E5, G5)
            for &(f, g) in &[
                (261.63f32, 0.18f32),
                (329.63, 0.16),
                (392.00, 0.16),
                (587.33, 0.14), // radiant major-9th D5 sparkle!
                (659.25, 0.17),
                (783.99, 0.17),
            ] {
                m.add_osc(OscLayer {
                    start_s: 1.275,
                    dur_s: 0.86,
                    f0: f,
                    f1: f,
                    wave: Wave::Triangle,
                    gain: g,
                    attack_s: 0.026,
                    decay_pow: 1.30,
                    vib_hz: 5.4,
                    vib_depth: 0.007,
                    ..Default::default()
                });
            }
            // Cascading crystalline victory harp arpeggio over the finale chord
            for (i, &af) in [1046.50f32, 1318.51, 1567.98, 2093.00, 2349.32, 2637.02]
                .iter()
                .enumerate()
            {
                let st = 1.33 + i as f32 * 0.068;
                m.add_fm(FmLayer {
                    start_s: st,
                    dur_s: 0.36,
                    c0: af,
                    c1: af,
                    mod_ratio: 2.0,
                    mod_index0: 0.90,
                    mod_index1: 0.04,
                    mod_pow: 2.5,
                    gain: 0.16,
                    attack_s: 0.004,
                    decay_pow: 1.85,
                    filter: FilterMode::LowPass,
                    fc0: 5800.0,
                    fc1: 2400.0,
                    q: 0.8,
                    ..Default::default()
                });
            }
            // Celebratory timpani downbeats + upbeat snare flourish into finale
            for &st in &[0.095f32, 0.635, 1.275] {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.14,
                    f0: 138.0,
                    f1: 65.4, // C2 timpani
                    pitch_pow: 0.45,
                    wave: Wave::Sine,
                    gain: 0.36,
                    attack_s: 0.003,
                    decay_pow: 1.7,
                    ..Default::default()
                });
            }
            for &(st, g) in &[(1.195f32, 0.14f32), (1.235, 0.18), (1.275, 0.24)] {
                m.add_noise(NoiseLayer {
                    start_s: st,
                    dur_s: 0.055,
                    pink: true,
                    gain: g,
                    attack_s: 0.0015,
                    decay_pow: 2.2,
                    filter: FilterMode::BandPass,
                    fc0: 2700.0,
                    fc1: 1200.0,
                    q: 1.4,
                    seed: base_seed ^ st.to_bits(),
                    ..Default::default()
                });
            }
            m.add_short_tail(0.26, 3600.0);
            m.finish(0.72, 1.35, 2.5, 65.0)
        }

        // 60. ShootThrash: the panic button. A rising wind-up whoosh into a heavy
        // bone-deep thump, then a long leathery tail-swish as the storm runs on.
        Sfx::ShootThrash => {
            const PITCH: [f32; 4] = [1.0, 0.94, 1.06, 0.98];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.52);
            // 90 ms wind-up: the tail coiling (bandpass sweeping upward).
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.090,
                pink: true,
                gain: 0.58,
                attack_s: 0.020,
                decay_pow: 0.8,
                filter: FilterMode::BandPass,
                fc0: 220.0 * p,
                fc1: 1500.0 * p,
                fc_pow: 1.6,
                q: 1.8,
                seed: base_seed,
                ..Default::default()
            });
            // The release: a sub-heavy thump with body (the tail hitting air).
            m.add_osc(OscLayer {
                start_s: 0.082,
                dur_s: 0.22,
                f0: 150.0 * p,
                f1: 44.0 * p,
                pitch_pow: 0.42,
                wave: Wave::Sine,
                gain: 0.92,
                attack_s: 0.002,
                decay_pow: 1.35,
                ..Default::default()
            });
            // The crack itself: broadband snap right on the release.
            m.add_noise(NoiseLayer {
                start_s: 0.082,
                dur_s: 0.060,
                pink: false,
                gain: 0.70,
                attack_s: 0.0006,
                decay_pow: 2.4,
                filter: FilterMode::BandPass,
                fc0: 2100.0 * p,
                fc1: 700.0 * p,
                q: 1.1,
                seed: base_seed ^ 0x22AA,
                ..Default::default()
            });
            // Sustained swish under the burst (the repeated revolutions).
            m.add_noise(NoiseLayer {
                start_s: 0.140,
                dur_s: 0.34,
                pink: true,
                gain: 0.34,
                attack_s: 0.030,
                decay_pow: 1.2,
                filter: FilterMode::BandPass,
                fc0: 900.0 * p,
                fc1: 380.0 * p,
                fc_pow: 1.3,
                q: 1.6,
                seed: base_seed ^ 0x5151,
                ..Default::default()
            });
            m.add_short_tail(0.18, 2400.0);
            m.finish(0.55, 1.4, 2.2, 38.0)
        }

        // 61. SonicSqueak: a high, piercing rat squeak (two fast slides) over a
        // thin glassy shimmer as the ring leaves the body.
        Sfx::SonicSqueak => {
            const PITCH: [f32; 4] = [1.0, 1.12, 0.90, 1.06];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.34);
            // The squeak: a near-sine chirp sliding up then down, thin and high.
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.150,
                f0: 2100.0 * p,
                f1: 2950.0 * p,
                pitch_pow: 0.75,
                wave: Wave::Sine,
                gain: 0.52,
                attack_s: 0.004,
                decay_pow: 1.5,
                vib_hz: 46.0,
                vib_depth: 0.030,
                ..Default::default()
            });
            m.add_osc(OscLayer {
                start_s: 0.014,
                dur_s: 0.170,
                f0: 3100.0 * p,
                f1: 1750.0 * p,
                pitch_pow: 1.1,
                wave: Wave::Triangle,
                gain: 0.24,
                attack_s: 0.006,
                decay_pow: 1.8,
                ..Default::default()
            });
            // Air/ring: a whisper of high-passed noise as the wavefront expands.
            m.add_noise(NoiseLayer {
                start_s: 0.010,
                dur_s: 0.20,
                pink: false,
                gain: 0.22,
                attack_s: 0.010,
                decay_pow: 1.6,
                filter: FilterMode::HighPass,
                fc0: 3600.0 * p,
                fc1: 2200.0 * p,
                q: 0.9,
                seed: base_seed,
                ..Default::default()
            });
            // A low thump so the ring has weight on the floor.
            m.add_osc(OscLayer {
                start_s: 0.006,
                dur_s: 0.16,
                f0: 210.0 * p,
                f1: 96.0 * p,
                pitch_pow: 0.6,
                wave: Wave::Sine,
                gain: 0.30,
                attack_s: 0.003,
                decay_pow: 1.5,
                ..Default::default()
            });
            m.add_short_tail(0.12, 5200.0);
            m.finish(0.50, 1.2, 2.4, 26.0)
        }

        // 62. ShootRain: the ceiling lets go. A soft, airy high hiss with three
        // lazy water blips falling through it — drizzle, not a downpour.
        Sfx::ShootRain => {
            const PITCH: [f32; 3] = [1.0, 0.94, 1.07];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.30);
            // The spatter bed: high-passed noise, quiet and wide.
            m.add_noise(NoiseLayer {
                start_s: 0.0,
                dur_s: 0.26,
                pink: true,
                gain: 0.30,
                attack_s: 0.018,
                decay_pow: 1.2,
                filter: FilterMode::HighPass,
                fc0: 2600.0 * p,
                fc1: 4200.0 * p,
                q: 0.7,
                seed: base_seed,
                ..Default::default()
            });
            // Three droplets landing, each a short falling sine blip.
            for &(st, f0, f1, g) in &[
                (0.040f32, 1180.0f32, 520.0f32, 0.40f32),
                (0.105, 940.0, 430.0, 0.32),
                (0.165, 1320.0, 610.0, 0.24),
            ] {
                m.add_osc(OscLayer {
                    start_s: st,
                    dur_s: 0.070,
                    f0: f0 * p,
                    f1: f1 * p,
                    pitch_pow: 1.4,
                    wave: Wave::Sine,
                    gain: g,
                    attack_s: 0.002,
                    decay_pow: 2.1,
                    ..Default::default()
                });
            }
            m.add_short_tail(0.16, 3800.0);
            m.finish(0.30, 1.3, 1.8, 16.0)
        }

        // 63. Infect: a wet, low *tick* with a queasy upward buzz behind it —
        // the sound of a note being passed along a packed carriage.
        Sfx::Infect => {
            const PITCH: [f32; 2] = [1.0, 0.91];
            let p = PITCH[v];
            let mut m = MixBuf::new(0.36);
            // The tag: a short damp thump.
            m.add_osc(OscLayer {
                start_s: 0.0,
                dur_s: 0.10,
                f0: 260.0 * p,
                f1: 120.0 * p,
                pitch_pow: 0.9,
                wave: Wave::Triangle,
                gain: 0.62,
                attack_s: 0.002,
                decay_pow: 1.9,
                ..Default::default()
            });
            // The queasy buzz: narrow band-passed noise sliding up a little.
            m.add_noise(NoiseLayer {
                start_s: 0.012,
                dur_s: 0.15,
                pink: true,
                gain: 0.42,
                attack_s: 0.010,
                decay_pow: 1.3,
                filter: FilterMode::BandPass,
                fc0: 420.0 * p,
                fc1: 780.0 * p,
                fc_pow: 0.8,
                q: 3.4,
                trem_hz: 22.0,
                trem_depth: 0.5,
                seed: base_seed ^ 0x00A5,
            });
            m.add_short_tail(0.12, 2600.0);
            m.finish(0.42, 1.25, 1.9, 14.0)
        }
    }
}

// ---- Music ---------------------------------------------------------------

const STEPS_PER_BAR: usize = 16;
const SECTION_BARS: usize = 4;
const SECTION_STEPS: usize = SECTION_BARS * STEPS_PER_BAR; // 64 steps per 4-bar section
const NUM_SECTIONS: usize = 8; // 8 × 4 bars = 32 bars (512 16th-note steps)
const LOOP_STEPS: usize = NUM_SECTIONS * SECTION_STEPS; // 512 steps
const REST: i32 = -127;

/// Compact step note markers (semitone offsets from the voice's reference root).
const R: i8 = i8::MIN; // rest (silence)
const H: i8 = i8::MIN + 1; // hold/sustain previous note smoothly

// Drum bitmask: normal groove in low nibble, Boss overdrive accents in high nibble.
const KICK: u8 = 1;
const SNARE: u8 = 2;
const HAT: u8 = 4;
const OHAT: u8 = 8;
const BKICK: u8 = 16;
const BSNARE: u8 = 32;
const BHAT: u8 = 64;

/// A three-voice chord voicing expressed in semitone offsets above `TrackCfg::root`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ChordSlot {
    name: &'static str,
    tones: [i8; 3],
}

/// One 4-bar (64-step) musical section with explicit half-bar chord harmony and
/// four independent compositional tracks (bass, lead, counter-melody, drums).
#[derive(Clone, Copy)]
struct Section4 {
    name: &'static str,
    /// Two half-bar chord slots per bar (8 steps each).
    chords: [ChordSlot; 8],
    /// Bass semitones relative to `root * 0.5` (`R` = rest, `H` = hold).
    bass: [i8; SECTION_STEPS],
    /// Lead melody semitones relative to `root * 2.0`.
    lead: [i8; SECTION_STEPS],
    /// Counter-melody / harmony line semitones relative to `root * 2.0`.
    counter: [i8; SECTION_STEPS],
    /// Percussion hits (normal + boss overdrive bits).
    drums: [u8; SECTION_STEPS],
    /// Whether Boss mode adds driving octave-bass pulse on non-triggered offbeats.
    boss_drive: bool,
}

struct TrackCfg {
    key_name: &'static str,
    root: f32,
    bpm: f32,
    lead_duty: f32,
    lead_lp: f32,
    counter_duty: f32,
    counter_lp: f32,
    chord_pulse_duty: f32,
    chord_pulse_mix: f32,
    chord_lp: f32,
    sections: &'static [Section4; NUM_SECTIONS],
}

#[inline]
const fn chord(name: &'static str, a: i8, b: i8, c: i8) -> ChordSlot {
    ChordSlot {
        name,
        tones: [a, b, c],
    }
}

// ---- Theme 1: SEWER — C Minor (104 BPM, ~73.8s loop) ---------------------
// Mood: Subterranean stealth & Castlevania/NES dark grit.
// Semitones from C: C=0, D=2, Eb=3, F=5, G=7, Ab=8, Bb=10, B=11, C'=12.

const SW_CM: ChordSlot = chord("Cm", 3, 7, 12);
const SW_CM_HI: ChordSlot = chord("Cm", 7, 12, 15);
const SW_AB: ChordSlot = chord("Ab", 3, 8, 12);
const SW_FM: ChordSlot = chord("Fm", 5, 8, 12);
const SW_G: ChordSlot = chord("G", 2, 7, 11);
const SW_G7: ChordSlot = chord("G7", 5, 7, 11);
const SW_EB: ChordSlot = chord("Eb", 7, 10, 15);
const SW_BB: ChordSlot = chord("Bb", 5, 10, 14);

const SEWER_SECTIONS: [Section4; NUM_SECTIONS] = [
    // 0: INTRO (Bars 0..4) — i - i - bVI - V subterranean pulse & hook teaser
    Section4 {
        name: "INTRO",
        chords: [SW_CM, SW_CM, SW_CM, SW_CM, SW_AB, SW_AB, SW_G, SW_G],
        bass: [
            0, H, R, 0, R, 0, 7, R, 0, H, R, 0, 3, R, 7, R,
            0, H, R, 0, R, 0, 7, R, 0, H, 3, R, 5, R, 7, R,
            8, H, R, 8, R, 8, 3, R, 8, H, R, 8, 7, R, 5, R,
            7, H, R, 7, R, 2, 7, R, 7, H, 5, R, 2, R, -1, R,
        ],
        lead: [
            7, H, H, R, 12, H, 10, 7, 8, H, 7, R, R, R, 5, R,
            7, H, H, R, 12, H, 14, 15, 14, H, H, R, R, R, 12, 10,
            8, H, H, R, 12, H, 8, 7, 8, H, 10, 12, 8, H, R, R,
            11, H, H, R, 14, H, 11, 7, 11, H, 14, H, 11, R, 7, R,
        ],
        counter: [
            3, H, 5, 7, 3, H, R, R, 3, H, 5, 7, 10, H, 7, 3,
            3, H, 5, 7, 8, H, R, R, 7, H, 5, 3, 7, H, 5, R,
            3, H, 5, 8, 3, H, R, R, 3, H, 7, 8, 12, H, 8, 3,
            2, H, 5, 7, 7, H, R, R, 5, H, 7, 11, 7, R, 2, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BKICK, KICK, 0, HAT, 0, SNARE, BSNARE, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BKICK, SNARE, 0, HAT, KICK, SNARE, 0, KICK, BSNARE, SNARE, 0, SNARE | OHAT, BSNARE,
        ],
        boss_drive: true,
    },
    // 1: A1 (Bars 4..8) — i - bVI - iv - V iconic Sewer hook (call-and-response counterpoint)
    Section4 {
        name: "A1",
        chords: [SW_CM, SW_CM, SW_AB, SW_AB, SW_FM, SW_FM, SW_G, SW_G7],
        bass: [
            0, H, 0, R, 0, R, 7, R, 0, H, 3, R, 5, R, 7, R,
            8, H, 8, R, 8, R, 3, R, 8, H, 7, R, 5, R, 3, R,
            5, H, 5, R, 5, R, 0, R, 5, H, 7, R, 8, R, 5, R,
            7, H, 7, R, 11, R, 7, R, 7, H, 5, R, 2, R, -1, R,
        ],
        lead: [
            7, H, H, 10, 12, H, 10, 8, 7, H, H, R, 5, H, 3, 5,
            8, H, H, 12, 15, H, 12, 10, 8, H, H, R, R, R, 7, 8,
            12, H, H, 10, 8, H, 5, 8, 12, H, 14, 15, 12, H, 8, 7,
            11, H, H, 14, 15, H, 14, 11, 14, H, H, R, 11, R, 7, R,
        ],
        counter: [
            3, H, 5, 7, 3, H, H, R, 3, H, 5, 7, 10, H, 7, R,
            3, H, 7, 8, 8, H, H, R, 3, H, 5, 8, 12, H, 8, 3,
            5, H, 8, 7, 5, H, H, R, 5, H, 8, 12, 8, H, 5, 3,
            7, H, 11, 7, 7, H, H, R, 5, H, 7, 11, 7, R, 5, 2,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, KICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, 0, HAT, BKICK, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, KICK, SNARE, BSNARE, HAT, KICK, SNARE, 0, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 2: A2 (Bars 8..12) — i - bIII - iv - V->i answering phrase with home resolution
    Section4 {
        name: "A2",
        chords: [SW_CM, SW_CM, SW_EB, SW_EB, SW_FM, SW_FM, SW_G, SW_CM],
        bass: [
            0, H, 0, R, 3, R, 7, R, 0, H, 3, R, 5, R, 7, R,
            3, H, 3, R, 7, R, 10, R, 3, H, 2, R, 0, R, 3, R,
            5, H, 5, R, 8, R, 12, R, 5, H, 8, R, 7, R, 5, R,
            7, H, 11, R, 7, R, 2, R, 0, H, R, 0, 7, R, 3, R,
        ],
        lead: [
            7, H, 10, 12, 15, H, 14, 12, 10, H, H, R, 7, H, 10, R,
            15, H, H, 14, 12, H, 10, 7, 10, H, H, R, 7, H, 10, 12,
            12, H, 15, 14, 12, H, 8, 12, 17, H, 15, 14, 12, H, 8, 7,
            14, H, 15, 14, 11, H, 14, 11, 12, H, H, H, H, H, R, R,
        ],
        counter: [
            3, H, H, R, 7, H, 5, 3, 3, H, 5, 7, 10, H, 7, 5,
            7, H, 10, 12, 7, H, H, R, 3, H, 7, 10, 12, H, 7, R,
            8, H, H, R, 5, H, 8, 5, 8, H, 12, 8, 5, H, 3, 5,
            7, H, 11, 7, 5, H, 7, 5, 3, H, 7, 10, 7, H, 5, 3,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, KICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, 0, HAT, KICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, BKICK, KICK, 0, HAT, 0, SNARE, BSNARE, OHAT, KICK,
        ],
        boss_drive: true,
    },
    // 3: B1 (Bars 12..16) — bIII - bVII - bVI - V heroic subterranean chorus lift
    Section4 {
        name: "B1",
        chords: [SW_EB, SW_EB, SW_BB, SW_BB, SW_AB, SW_AB, SW_G, SW_G],
        bass: [
            3, H, 3, 7, 10, R, 7, R, 3, H, 3, 7, 10, R, 7, 3,
            10, H, 10, 5, 2, R, 5, R, 10, H, 10, 5, 7, R, 10, R,
            8, H, 8, 3, 0, R, 3, R, 8, H, 8, 3, 5, R, 8, R,
            7, H, 7, 2, -1, R, 2, R, 7, H, 11, R, 14, R, 7, R,
        ],
        lead: [
            15, H, H, 14, 12, H, 10, 12, 15, H, H, H, 12, H, 10, R,
            14, H, H, 12, 10, H, 5, 10, 14, H, H, H, 10, H, 7, R,
            12, H, H, 10, 8, H, 3, 8, 12, H, 14, 15, 12, H, 8, R,
            14, H, H, 11, 14, H, 15, 14, 11, H, H, H, 7, R, 11, R,
        ],
        counter: [
            10, H, 7, 10, 7, H, H, R, 7, 10, 12, 10, 7, H, 3, 7,
            10, H, 5, 10, 5, H, H, R, 5, 10, 12, 10, 5, H, 2, 5,
            8, H, 3, 8, 3, H, H, R, 3, 8, 10, 12, 8, H, 3, R,
            7, H, 2, 7, 7, H, H, R, 2, 5, 7, 11, 7, R, 2, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, SNARE, 0, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 4: B2 (Bars 16..20) — bVI - bIII - iv - V7 dramatic chorus climax
    Section4 {
        name: "B2",
        chords: [SW_AB, SW_AB, SW_EB, SW_EB, SW_FM, SW_FM, SW_G, SW_G7],
        bass: [
            8, H, 8, 12, 8, R, 3, R, 8, H, 8, 12, 10, R, 8, R,
            3, H, 3, 7, 3, R, 10, R, 3, H, 3, 7, 5, R, 3, R,
            5, H, 5, 8, 5, R, 0, R, 5, H, 5, 8, 12, R, 8, R,
            7, H, 7, 11, 7, R, 2, R, 7, H, 5, R, 2, R, -1, R,
        ],
        lead: [
            15, H, H, 12, 15, H, 17, 15, 12, H, H, R, 12, H, 15, R,
            19, H, H, 15, 15, H, 14, 12, 10, H, H, R, 12, H, 15, R,
            17, H, H, 15, 17, H, 15, 12, 15, H, H, 17, 15, H, 12, R,
            14, H, H, 15, 11, H, H, 14, 15, H, 14, 11, 7, H, R, R,
        ],
        counter: [
            8, H, 12, 8, 12, H, H, R, 8, 3, 8, 10, 8, H, 12, R,
            10, H, 12, 10, 12, H, H, R, 7, 3, 7, 10, 7, H, 10, R,
            12, H, 8, 12, 8, H, H, R, 8, 5, 8, 12, 8, H, 5, R,
            7, H, 11, 7, 7, H, 5, 7, 11, H, 7, 5, 2, H, 5, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, KICK, BHAT, SNARE, 0, KICK, SNARE, SNARE, BSNARE, KICK, SNARE, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 5: BREAKDOWN (Bars 20..24) — iv - i - bVI - V7 spacious atmospheric drop + riser
    Section4 {
        name: "BREAK",
        chords: [SW_FM, SW_FM, SW_CM_HI, SW_CM_HI, SW_AB, SW_AB, SW_G, SW_G7],
        bass: [
            5, H, H, H, H, H, H, H, 5, H, H, H, 3, H, R, R,
            0, H, H, H, H, H, H, H, 0, H, H, H, 7, H, R, R,
            8, H, H, H, H, H, H, H, 8, H, H, H, 5, H, R, R,
            7, H, H, R, 7, H, H, R, 7, R, 7, R, 7, 7, 7, 11,
        ],
        lead: [
            R, R, R, R, R, R, R, R, 8, H, H, R, 7, H, 5, R,
            R, R, R, R, R, R, R, R, 3, H, H, R, 7, H, 3, R,
            8, H, 7, 5, 3, H, 5, 7, 8, H, 12, H, 15, H, 12, R,
            14, H, 11, H, 14, H, 15, H, 17, H, 15, 14, 11, R, 14, R,
        ],
        counter: [
            0, H, 5, R, 8, H, 5, R, 0, H, 3, R, 5, H, R, R,
            3, H, 7, R, 12, H, 7, R, 3, H, 2, R, 0, H, R, R,
            3, H, 8, R, 12, H, 8, R, 3, H, 7, R, 8, H, 5, R,
            7, H, 2, R, 7, H, 11, R, 7, R, 11, R, 7, R, 11, 2,
        ],
        drums: [
            KICK, 0, 0, 0, 0, 0, HAT, 0, 0, 0, HAT, 0, 0, 0, OHAT, 0,
            KICK, 0, 0, 0, 0, 0, HAT, 0, 0, 0, HAT, 0, 0, 0, HAT, 0,
            KICK, 0, HAT, 0, 0, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, OHAT, 0,
            KICK, 0, SNARE, 0, KICK, 0, SNARE, 0, KICK, SNARE, KICK, SNARE, SNARE, SNARE | BSNARE, SNARE | OHAT, SNARE,
        ],
        boss_drive: false,
    },
    // 6: A_RET (Bars 24..28) — i - bVI - bVII - V triumphant embellished return
    Section4 {
        name: "A_RET",
        chords: [SW_CM_HI, SW_CM_HI, SW_AB, SW_AB, SW_BB, SW_BB, SW_G, SW_G7],
        bass: [
            0, H, 0, 7, 0, R, 7, 3, 0, H, 3, R, 5, R, 7, R,
            8, H, 8, 12, 8, R, 3, 5, 8, H, 7, R, 5, R, 3, R,
            10, H, 10, 5, 10, R, 5, 7, 10, H, 7, R, 5, R, 2, R,
            7, H, 7, 11, 7, R, 2, 5, 7, H, 11, R, 14, R, 7, R,
        ],
        lead: [
            12, H, 15, 14, 12, H, 10, 7, 12, H, R, 15, 17, H, 15, 14,
            12, H, 8, 10, 12, H, 15, 12, 8, H, H, R, 12, R, 10, 8,
            10, H, 14, 12, 10, H, 7, 5, 10, H, 12, 14, 17, H, 14, 12,
            14, H, 15, 14, 11, H, 14, 15, 19, H, 15, 14, 11, H, 14, R,
        ],
        counter: [
            7, H, 12, 10, 7, H, 5, 3, 7, H, 3, 7, 12, H, 10, 7,
            8, H, 3, 5, 8, H, 12, 8, 3, H, 5, 7, 8, R, 7, 3,
            5, H, 10, 7, 5, H, 2, 0, 5, H, 7, 10, 14, H, 10, 5,
            7, H, 11, 7, 2, H, 7, 11, 14, H, 11, 7, 2, H, 7, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, KICK, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, HAT, KICK, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 7: TURN (Bars 28..32) — i - iv - bVI->V - V7 crisp turnaround into Bar 0 Cm
    Section4 {
        name: "TURN",
        chords: [SW_CM, SW_CM, SW_FM, SW_FM, SW_AB, SW_G, SW_G7, SW_G],
        bass: [
            0, H, 0, R, 7, R, 3, R, 0, H, 3, R, 5, R, 3, R,
            5, H, 5, R, 8, R, 5, R, 5, H, 3, R, 0, R, 5, R,
            8, H, 8, R, 3, R, 8, R, 7, H, 7, R, 2, R, 7, R,
            7, H, 5, R, 2, R, -1, R, 7, H, 5, R, 2, R, -1, R,
        ],
        lead: [
            12, H, 10, 7, 3, H, 7, 10, 12, H, H, R, 7, R, 5, 3,
            5, H, 8, 12, 15, H, 12, 8, 5, H, 8, 7, 5, H, 3, R,
            8, H, 12, 8, 15, H, 12, 8, 14, H, 11, 7, 14, H, 11, 7,
            11, H, 7, 5, 2, H, 5, 7, 11, H, 7, 5, 2, R, -1, R,
        ],
        counter: [
            7, H, 5, 3, 0, H, 3, 5, 7, H, 3, 5, 3, R, 2, 0,
            0, H, 5, 8, 8, H, 5, 0, 0, H, 5, 3, 0, H, -4, R,
            3, H, 8, 3, 8, H, 3, 0, 7, H, 2, -1, 7, H, 2, -1,
            2, H, 5, 2, -1, H, 2, 5, 7, H, 5, 2, -1, R, -5, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, OHAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, SNARE, 0, KICK, BKICK, SNARE, 0, KICK, BSNARE, SNARE, 0, KICK, SNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
];

// ---- Theme 2: KITCHEN — D Mixolydian / Major (126 BPM, ~61.0s loop) ------
// Mood: Frantic, playful, bouncy NES/SNES cartoon kitchen chase.
// Semitones from D: D=0, E=2, F#=4, G=5, A=7, B=9, C=10, C#=11, D'=12.

const KT_D: ChordSlot = chord("D", 4, 7, 12);
const KT_G: ChordSlot = chord("G", 5, 9, 12);
const KT_C: ChordSlot = chord("C", 5, 10, 14);
const KT_A: ChordSlot = chord("A", 2, 7, 11);
const KT_A_HI: ChordSlot = chord("A", 7, 11, 14);
const KT_A7: ChordSlot = chord("A7", 5, 7, 11);
const KT_BM: ChordSlot = chord("Bm", 4, 9, 12);
const KT_EM: ChordSlot = chord("Em", 5, 9, 14);

const KITCHEN_SECTIONS: [Section4; NUM_SECTIONS] = [
    // 0: INTRO (Bars 0..4) — I - IV - bVII - V upbeat bouncy kitchen start
    Section4 {
        name: "INTRO",
        chords: [KT_D, KT_D, KT_G, KT_G, KT_C, KT_C, KT_A, KT_A],
        bass: [
            0, R, 7, R, 0, R, 7, 9, 0, R, 7, R, 4, R, 7, R,
            5, R, 9, R, 5, R, 9, 10, 5, R, 9, R, 7, R, 5, R,
            10, R, 5, R, 10, R, 5, 7, 10, R, 5, R, 2, R, 5, R,
            7, R, 2, R, 7, R, 11, R, 7, R, 4, R, 2, R, 7, 9,
        ],
        lead: [
            7, H, R, 9, 12, H, H, R, 9, H, 7, 4, 7, H, R, R,
            9, H, R, 12, 14, H, H, R, 12, H, 9, 5, 9, H, R, R,
            10, H, R, 12, 14, H, 12, 10, 7, H, H, R, 10, H, 12, R,
            11, H, R, 14, 16, H, 14, 11, 14, H, H, R, 11, R, 7, R,
        ],
        counter: [
            4, H, 7, 4, 7, H, 4, 7, 4, H, R, R, 4, 7, 9, 7,
            5, H, 9, 5, 9, H, 5, 9, 5, H, R, R, 5, 9, 12, 9,
            5, H, 7, 5, 10, H, R, R, 5, 7, 10, 7, 5, H, 7, R,
            7, H, 11, 7, 11, H, R, R, 7, 9, 11, 14, 7, R, 4, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BKICK, KICK, HAT, KICK, 0, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, KICK, SNARE, BSNARE, KICK, 0, SNARE, HAT, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 1: A1 (Bars 4..8) — I - IV - bVII->IV - V cheeky main chase theme (call & response)
    Section4 {
        name: "A1",
        chords: [KT_D, KT_D, KT_G, KT_G, KT_C, KT_G, KT_A, KT_A],
        bass: [
            0, R, 7, R, 0, R, 4, 7, 0, R, 7, R, 9, R, 7, R,
            5, R, 9, R, 5, R, 7, 9, 5, R, 12, R, 9, R, 5, R,
            10, R, 5, R, 10, R, 7, R, 5, R, 9, R, 5, R, 2, R,
            7, R, 7, 9, 11, R, 7, R, 7, R, 4, R, 2, R, 7, 9,
        ],
        lead: [
            7, H, 4, 7, 12, H, H, 9, 7, 9, 12, 9, 7, H, H, R,
            9, H, 5, 9, 12, H, H, 10, 9, 7, 5, 7, 9, H, H, R,
            10, H, 7, 10, 14, H, 12, 10, 12, H, 9, 12, 14, H, 12, 9,
            11, H, 12, 14, 16, H, 14, 11, 14, H, H, R, 11, R, 14, R,
        ],
        counter: [
            4, H, H, R, 7, 9, 7, 4, 4, H, H, R, 4, 7, 9, 7,
            5, H, H, R, 9, 10, 9, 5, 5, H, H, R, 5, 9, 12, 9,
            5, H, H, R, 10, 7, 10, 7, 5, H, H, R, 9, 12, 9, 5,
            7, H, H, R, 11, 7, 11, 7, 7, 9, 11, 7, 7, R, 9, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, BKICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, HAT, KICK, 0, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, BKICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, KICK, SNARE, BSNARE, KICK, 0, SNARE, HAT, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 2: A2 (Bars 8..12) — I - vi - IV - V->I warm major resolution
    Section4 {
        name: "A2",
        chords: [KT_D, KT_D, KT_BM, KT_BM, KT_G, KT_G, KT_A, KT_D],
        bass: [
            0, R, 7, R, 0, R, 4, 7, 0, R, 4, R, 7, R, 11, R,
            9, R, 4, R, 9, R, 12, R, 9, R, 7, R, 4, R, 9, R,
            5, R, 9, R, 5, R, 12, R, 5, R, 9, R, 7, R, 5, R,
            7, R, 11, R, 7, R, 2, R, 0, H, R, 0, 7, R, 4, R,
        ],
        lead: [
            7, 4, 0, 4, 7, H, 12, 14, 16, H, 14, 12, 9, H, 7, R,
            12, 11, 9, 7, 9, H, 12, 14, 16, H, 14, 12, 9, H, 4, R,
            12, H, 9, 12, 14, H, 12, 9, 12, H, 14, 16, 17, H, 14, 12,
            14, H, 16, 14, 11, H, 9, 11, 12, H, H, H, H, R, R, R,
        ],
        counter: [
            4, R, 0, R, 4, H, 7, 9, 12, H, 9, 7, 4, H, 0, R,
            9, 7, 4, 4, 4, H, 9, 11, 12, H, 11, 9, 4, H, 0, R,
            9, H, 5, 9, 9, H, 9, 5, 9, H, 9, 12, 12, H, 9, 5,
            7, H, 11, 7, 7, H, 4, 7, 4, H, 7, 4, 0, H, 4, 7,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, BKICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, HAT, KICK, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, BKICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, BKICK, KICK, 0, HAT, 0, SNARE, BSNARE, OHAT, KICK,
        ],
        boss_drive: true,
    },
    // 3: B1 (Bars 12..16) — IV - I - bVII - IV bright kitchen scramble chorus
    Section4 {
        name: "B1",
        chords: [KT_G, KT_G, KT_D, KT_D, KT_C, KT_C, KT_G, KT_G],
        bass: [
            5, R, 9, 12, 5, R, 9, R, 5, R, 12, R, 9, R, 5, 7,
            0, R, 4, 7, 0, R, 7, R, 0, R, 9, R, 7, R, 4, 2,
            10, R, 5, 7, 10, R, 5, R, 10, R, 7, R, 5, R, 2, R,
            5, R, 9, 12, 5, R, 9, R, 5, R, 7, R, 9, R, 12, R,
        ],
        lead: [
            17, H, H, 14, 12, H, 9, 12, 17, H, H, H, 14, H, 12, R,
            16, H, H, 14, 12, H, 7, 12, 16, H, H, H, 12, H, 9, R,
            14, H, H, 12, 10, H, 7, 10, 14, H, H, 12, 10, H, 7, R,
            12, H, H, 14, 19, H, 17, 14, 12, H, H, H, 14, H, 17, R,
        ],
        counter: [
            12, H, 9, 12, 9, H, H, R, 9, 12, 14, 12, 9, H, 5, 9,
            12, H, 9, 7, 7, H, H, R, 7, 9, 12, 9, 7, H, 4, 7,
            10, H, 7, 5, 5, H, H, R, 5, 7, 10, 7, 5, H, 2, 5,
            9, H, 5, 9, 12, H, H, R, 5, 9, 12, 9, 9, H, 12, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, HAT, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, HAT, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, HAT, KICK, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 4: B2 (Bars 16..20) — ii - V - vi - V7 comedy chase escalation
    Section4 {
        name: "B2",
        chords: [KT_EM, KT_EM, KT_A_HI, KT_A_HI, KT_BM, KT_BM, KT_A, KT_A7],
        bass: [
            2, R, 5, 9, 2, R, 9, R, 2, R, 5, R, 9, R, 5, R,
            7, R, 11, 14, 7, R, 11, R, 7, R, 2, R, 4, R, 7, R,
            9, R, 12, 16, 9, R, 12, R, 9, R, 7, R, 4, R, 9, R,
            7, R, 11, R, 7, R, 5, R, 7, R, 2, R, 7, 9, 11, R,
        ],
        lead: [
            14, H, H, 12, 14, H, 17, 14, 12, H, H, R, 9, H, 12, R,
            16, H, H, 14, 16, H, 19, 16, 14, H, H, R, 11, H, 14, R,
            16, H, H, 14, 16, H, 19, 16, 14, H, H, R, 12, H, 16, R,
            19, H, H, 16, 14, H, H, 16, 19, H, 17, 14, 11, H, R, R,
        ],
        counter: [
            9, H, 5, 9, 9, H, H, R, 5, 9, 12, 9, 5, H, 9, R,
            11, H, 7, 11, 11, H, H, R, 7, 11, 14, 11, 7, H, 11, R,
            12, H, 9, 12, 12, H, H, R, 9, 4, 9, 12, 9, H, 12, R,
            11, H, 7, 11, 7, H, 11, 7, 11, H, 7, 5, 7, H, 11, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, HAT, KICK, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, KICK, BHAT, SNARE, 0, KICK, SNARE, SNARE, BSNARE, KICK, SNARE, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 5: BREAKDOWN (Bars 20..24) — vi - IV - ii - V tiptoe sneak + buildup
    Section4 {
        name: "BREAK",
        chords: [KT_BM, KT_BM, KT_G, KT_G, KT_EM, KT_EM, KT_A, KT_A7],
        bass: [
            9, H, H, R, 4, H, H, R, 9, H, H, R, 7, H, R, R,
            5, H, H, R, 0, H, H, R, 5, H, H, R, 4, H, R, R,
            2, H, H, R, 9, H, H, R, 2, H, H, R, 5, H, R, R,
            7, H, R, 7, 7, H, R, 7, 7, R, 7, R, 7, 9, 11, 7,
        ],
        lead: [
            R, R, R, R, R, R, R, R, 9, R, 12, R, 14, H, 12, R,
            R, R, R, R, R, R, R, R, 9, R, 12, R, 17, H, 14, R,
            14, R, 12, R, 9, R, 5, R, 9, R, 12, R, 14, H, 12, R,
            11, R, 14, R, 16, R, 14, R, 16, R, 17, R, 19, R, 16, R,
        ],
        counter: [
            4, R, 9, R, 12, R, 9, R, 4, H, R, R, 0, H, R, R,
            5, R, 9, R, 12, R, 9, R, 5, H, R, R, 0, H, R, R,
            5, R, 9, R, 12, R, 9, R, 5, R, 9, R, 12, H, 9, R,
            7, R, 11, R, 7, R, 11, R, 7, R, 11, R, 14, R, 11, R,
        ],
        drums: [
            KICK, 0, 0, 0, HAT, 0, 0, 0, KICK, 0, 0, 0, HAT, 0, OHAT, 0,
            KICK, 0, 0, 0, HAT, 0, 0, 0, KICK, 0, 0, 0, HAT, 0, HAT, 0,
            KICK, 0, HAT, 0, SNARE, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, OHAT, 0,
            KICK, 0, SNARE, 0, KICK, 0, SNARE, 0, KICK, SNARE, KICK, SNARE, SNARE, SNARE | BSNARE, SNARE | OHAT, SNARE,
        ],
        boss_drive: false,
    },
    // 6: A_RET (Bars 24..28) — I - IV - bVII->vi - V high-energy hustle return
    Section4 {
        name: "A_RET",
        chords: [KT_D, KT_D, KT_G, KT_G, KT_C, KT_BM, KT_A_HI, KT_A7],
        bass: [
            0, R, 7, 12, 0, R, 4, 7, 0, R, 7, R, 9, R, 7, 4,
            5, R, 9, 12, 5, R, 7, 9, 5, R, 12, R, 9, R, 5, 7,
            10, R, 5, 10, 10, R, 7, R, 9, R, 4, 9, 9, R, 7, R,
            7, R, 11, 14, 7, R, 11, R, 7, R, 5, R, 4, R, 2, R,
        ],
        lead: [
            12, 9, 7, 9, 12, H, 16, 12, 9, 12, 14, 12, 9, H, 7, R,
            14, 12, 9, 12, 14, H, 17, 14, 12, 9, 7, 9, 12, H, 9, R,
            14, H, 10, 14, 17, H, 14, 10, 16, H, 12, 14, 16, H, 12, 9,
            14, 16, 17, 19, 16, H, 14, 11, 14, H, 11, 7, 11, H, 14, R,
        ],
        counter: [
            7, 4, 0, 4, 7, H, 12, 7, 4, 7, 9, 7, 4, H, 0, R,
            9, 5, 5, 5, 9, H, 12, 9, 5, 5, 0, 5, 9, H, 5, R,
            10, H, 5, 10, 12, H, 10, 5, 9, H, 4, 9, 12, H, 9, 4,
            7, 11, 11, 14, 11, H, 7, 7, 7, H, 7, 2, 7, H, 11, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, HAT, KICK, BKICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, BKICK, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, HAT, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, HAT, KICK, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 7: TURN (Bars 28..32) — IV - ii - V - V7 bright turnaround into Bar 0 D
    Section4 {
        name: "TURN",
        chords: [KT_G, KT_G, KT_EM, KT_EM, KT_A, KT_A_HI, KT_A7, KT_A],
        bass: [
            5, R, 9, R, 5, R, 12, R, 5, R, 9, R, 7, R, 5, R,
            2, R, 5, R, 2, R, 9, R, 2, R, 5, R, 4, R, 2, R,
            7, R, 11, R, 7, R, 2, R, 7, R, 11, R, 14, R, 11, R,
            7, R, 5, R, 4, R, 2, R, 7, R, 9, R, 11, R, 7, R,
        ],
        lead: [
            12, H, 9, 5, 12, H, 14, 12, 9, H, 5, 7, 9, H, 12, R,
            14, H, 12, 9, 14, H, 12, 9, 5, H, 9, 12, 14, H, 12, R,
            11, H, 7, 11, 14, H, 16, 14, 11, H, 7, 11, 16, H, 14, 11,
            14, H, 12, 11, 9, H, 7, 9, 11, H, 14, 11, 7, R, 11, R,
        ],
        counter: [
            9, H, 5, 0, 9, H, 9, 5, 5, H, 0, 2, 5, H, 9, R,
            9, H, 5, 2, 9, H, 5, 2, 2, H, 5, 9, 9, H, 5, R,
            7, H, 2, 7, 11, H, 11, 7, 7, H, 2, 7, 11, H, 7, 2,
            7, H, 7, 7, 5, H, 2, 4, 7, H, 11, 7, 2, R, 7, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, HAT, KICK, 0, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, OHAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, SNARE, 0, KICK, BKICK, SNARE, 0, KICK, BSNARE, SNARE, 0, KICK, SNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
];

// ---- Theme 3: LAB — B Phrygian Dominant (114 BPM, ~67.4s loop) -----------
// Mood: Eerie neon synthwave-chiptune laboratory (Mega Man / Metroid robotics).
// Semitones from B: B=0, C=1, D=3, D#=4, E=5, F#=7, G=8, A=10, B'=12.

const LB_B: ChordSlot = chord("B", 4, 7, 12);
const LB_B_HI: ChordSlot = chord("B", 7, 12, 16);
const LB_C: ChordSlot = chord("C", 5, 8, 13);
const LB_EM: ChordSlot = chord("Em", 5, 8, 12);
const LB_G: ChordSlot = chord("G", 3, 8, 12);
const LB_AM: ChordSlot = chord("Am", 5, 10, 13);
const LB_FSDIM: ChordSlot = chord("F#d", 7, 10, 13);

const LAB_SECTIONS: [Section4; NUM_SECTIONS] = [
    // 0: INTRO (Bars 0..4) — I - bII - I - bII hypnotic neon lab boot-up
    Section4 {
        name: "INTRO",
        chords: [LB_B, LB_B, LB_C, LB_C, LB_B, LB_B, LB_C, LB_B],
        bass: [
            0, R, 0, 12, 0, R, 0, R, 0, R, 0, 12, 1, R, 0, R,
            1, R, 1, 13, 1, R, 1, R, 1, R, 1, 13, 0, R, -2, R,
            0, R, 0, 12, 0, R, 7, R, 0, R, 0, 12, 4, R, 7, R,
            1, R, 1, 8, 1, R, 5, R, 0, R, 0, 7, 1, R, 0, R,
        ],
        lead: [
            7, H, H, R, 12, H, 10, 8, 7, H, 4, R, R, R, 5, 7,
            8, H, H, R, 13, H, 12, 8, 5, H, H, R, R, R, 4, 5,
            7, H, H, 10, 8, H, 7, 4, 12, H, H, R, 8, H, 7, R,
            8, H, 12, 8, 5, H, 8, 5, 7, H, H, R, 4, R, 7, R,
        ],
        counter: [
            4, H, 7, 4, 7, H, R, R, 4, H, 1, 4, 7, H, 4, R,
            5, H, 8, 5, 8, H, R, R, 1, H, 5, 8, 5, H, 1, R,
            4, H, 7, 4, 4, H, R, R, 7, H, 4, 7, 4, H, 1, R,
            5, H, 8, 5, 1, H, R, R, 4, H, 7, 4, 1, R, 4, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, 0, SNARE, 0, OHAT, KICK,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BKICK, KICK, 0, KICK, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, 0, SNARE, 0, OHAT, KICK,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, BKICK, KICK, HAT, SNARE, BSNARE, KICK, 0, SNARE | OHAT, HAT,
        ],
        boss_drive: true,
    },
    // 1: A1 (Bars 4..8) — I - bII - iv - bII->I eerie exotic main theme (call & response)
    Section4 {
        name: "A1",
        chords: [LB_B, LB_B, LB_C, LB_C, LB_EM, LB_EM, LB_C, LB_B],
        bass: [
            0, R, 0, 12, 0, R, 7, R, 0, R, 0, 12, 1, R, 0, R,
            1, R, 1, 13, 1, R, 8, R, 1, R, 0, R, 10, R, 7, R,
            5, R, 5, 12, 5, R, 8, R, 5, R, 5, 12, 8, R, 7, R,
            1, R, 5, R, 8, R, 5, R, 0, R, 0, 7, 1, R, 0, R,
        ],
        lead: [
            12, H, H, 10, 8, H, 7, 4, 7, H, H, R, 4, H, 7, 8,
            13, H, H, 12, 8, H, 5, 8, 12, H, H, R, 8, H, 5, R,
            12, H, 13, 12, 8, H, H, 7, 12, H, 16, 13, 12, H, 8, 7,
            13, H, 12, 8, 5, H, 8, 13, 12, H, H, R, 7, R, 4, R,
        ],
        counter: [
            7, H, 8, 7, 4, H, H, R, 4, H, 7, 8, 7, H, 4, R,
            8, H, 5, 8, 5, H, H, R, 5, H, 8, 13, 5, H, 1, R,
            8, H, H, R, 5, H, 8, 12, 8, H, H, R, 8, H, 5, 8,
            8, H, H, R, 1, H, 5, 8, 4, H, 7, 4, 4, R, 0, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, 0, KICK, BKICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, 0, SNARE, 0, OHAT, KICK,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, BKICK, KICK, HAT, SNARE, BSNARE, KICK, 0, SNARE | OHAT, HAT,
        ],
        boss_drive: true,
    },
    // 2: A2 (Bars 8..12) — I - bII - bVI - bII->I answering sci-fi hook
    Section4 {
        name: "A2",
        chords: [LB_B, LB_B, LB_C, LB_C, LB_G, LB_G, LB_C, LB_B],
        bass: [
            0, R, 0, 12, 0, R, 4, 7, 0, R, 0, 12, 1, R, 0, R,
            1, R, 1, 13, 1, R, 5, 8, 1, R, 1, 8, 5, R, 1, R,
            8, R, 8, 15, 8, R, 12, R, 8, R, 8, 15, 12, R, 8, R,
            1, R, 8, R, 5, R, 1, R, 0, H, R, 0, 7, R, 4, R,
        ],
        lead: [
            12, H, 13, 12, 7, H, 12, 13, 16, H, 13, 12, 7, H, 4, R,
            13, H, 16, 17, 13, H, 12, 8, 13, H, 12, 8, 5, H, 8, R,
            15, H, 12, 8, 15, H, 19, 15, 12, H, 8, 12, 15, H, 12, 8,
            13, H, 12, 8, 5, H, 8, 13, 12, H, H, H, H, R, R, R,
        ],
        counter: [
            7, H, 8, 7, 4, H, 7, 8, 12, H, 8, 7, 4, H, 0, R,
            8, H, 12, 13, 8, H, 5, 1, 8, H, 5, 1, 1, H, 5, R,
            8, H, 8, 3, 8, H, 12, 8, 8, H, 3, 8, 12, H, 8, 3,
            8, H, 5, 1, 1, H, 5, 8, 4, H, 7, 4, 0, H, 4, 7,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, HAT, BHAT, SNARE, 0, OHAT, 0, KICK, 0, KICK, BKICK, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, HAT, 0, SNARE, 0, OHAT, KICK,
            KICK, 0, HAT, KICK, SNARE, 0, HAT, BKICK, KICK, 0, HAT, 0, SNARE, BSNARE, OHAT, KICK,
        ],
        boss_drive: true,
    },
    // 3: B1 (Bars 12..16) — iv - bII - bvii - I pulsing lab alarm chorus
    Section4 {
        name: "B1",
        chords: [LB_EM, LB_EM, LB_C, LB_C, LB_AM, LB_AM, LB_B, LB_B],
        bass: [
            5, R, 5, 12, 8, R, 5, R, 5, R, 5, 12, 8, R, 5, 7,
            1, R, 1, 8, 5, R, 1, R, 1, R, 1, 8, 5, R, 1, 0,
            10, R, 10, 5, 1, R, 5, R, 10, R, 10, 5, 8, R, 10, R,
            0, R, 0, 7, 4, R, 7, R, 0, R, 4, R, 7, R, 12, R,
        ],
        lead: [
            17, H, H, 16, 17, H, 19, 17, 12, H, H, H, 17, H, 12, R,
            13, H, H, 12, 13, H, 17, 13, 12, H, H, H, 13, H, 8, R,
            17, H, H, 13, 17, H, 16, 13, 10, H, H, 13, 17, H, 13, R,
            16, H, H, 12, 16, H, 19, 16, 12, H, H, H, 12, H, 16, R,
        ],
        counter: [
            12, H, 8, 12, 12, H, H, R, 8, 5, 8, 12, 8, H, 5, R,
            8, H, 5, 8, 8, H, H, R, 5, 1, 5, 8, 5, H, 1, R,
            10, H, 5, 10, 10, H, H, R, 5, 10, 13, 10, 10, H, 5, R,
            7, H, 4, 7, 7, H, H, R, 4, 7, 12, 7, 7, H, 12, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, KICK, BKICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, SNARE, 0, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 4: B2 (Bars 16..20) — bVI - iv - bII - I dramatic synthwave peak
    Section4 {
        name: "B2",
        chords: [LB_G, LB_G, LB_EM, LB_EM, LB_C, LB_C, LB_B_HI, LB_B],
        bass: [
            8, R, 8, 15, 8, R, 12, R, 8, R, 8, 15, 12, R, 8, R,
            5, R, 5, 12, 5, R, 8, R, 5, R, 5, 12, 8, R, 5, R,
            1, R, 1, 8, 1, R, 5, R, 1, R, 1, 8, 5, R, 1, R,
            0, R, 0, 7, 0, R, 4, R, 0, R, 1, R, 4, R, 7, R,
        ],
        lead: [
            19, H, H, 15, 19, H, 20, 19, 15, H, H, R, 15, H, 19, R,
            17, H, H, 12, 17, H, 19, 17, 12, H, H, R, 12, H, 17, R,
            13, H, H, 17, 13, H, 12, 8, 17, H, H, 16, 13, H, 8, R,
            16, H, H, 12, 19, H, H, 16, 12, H, 7, 4, 7, H, R, R,
        ],
        counter: [
            12, H, 8, 12, 12, H, H, R, 8, 3, 8, 12, 8, H, 12, R,
            12, H, 8, 12, 12, H, H, R, 8, 5, 8, 12, 8, H, 12, R,
            8, H, 13, 8, 8, H, H, R, 8, 5, 8, 13, 8, H, 5, R,
            7, H, 4, 7, 12, H, 7, 4, 4, H, 0, 4, 4, H, 7, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, KICK, BKICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, KICK, BHAT, SNARE, 0, KICK, SNARE, SNARE, BSNARE, KICK, SNARE, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 5: BREAKDOWN (Bars 20..24) — iv - I - bII - F#dim sonar lab breakdown
    Section4 {
        name: "BREAK",
        chords: [LB_EM, LB_EM, LB_B, LB_B, LB_C, LB_C, LB_FSDIM, LB_C],
        bass: [
            5, H, H, H, H, H, H, H, 5, H, H, H, 8, H, R, R,
            0, H, H, H, H, H, H, H, 0, H, H, H, 4, H, R, R,
            1, H, H, H, H, H, H, H, 1, H, H, H, 5, H, R, R,
            7, H, R, 7, 10, H, R, 10, 1, R, 1, R, 1, 5, 8, 1,
        ],
        lead: [
            R, R, R, R, R, R, R, R, 12, H, H, R, 8, H, 5, R,
            R, R, R, R, R, R, R, R, 7, H, H, R, 4, H, 0, R,
            8, H, 5, 1, 8, H, 12, 8, 13, H, 12, 8, 13, H, 17, R,
            13, H, 10, H, 13, H, 16, H, 17, H, 13, 12, 8, R, 13, R,
        ],
        counter: [
            8, H, 5, R, 12, H, 8, R, 5, H, R, R, 0, H, R, R,
            4, H, 7, R, 12, H, 7, R, 4, H, R, R, 0, H, R, R,
            5, H, 8, R, 13, H, 8, R, 5, H, 1, R, 8, H, 5, R,
            7, H, 10, R, 13, H, 10, R, 8, R, 5, R, 8, R, 5, 1,
        ],
        drums: [
            KICK, 0, 0, 0, 0, 0, HAT, 0, 0, 0, HAT, 0, 0, 0, OHAT, 0,
            KICK, 0, 0, 0, 0, 0, HAT, 0, 0, 0, HAT, 0, 0, 0, HAT, 0,
            KICK, 0, HAT, 0, 0, 0, HAT, 0, KICK, 0, HAT, 0, SNARE, 0, OHAT, 0,
            KICK, 0, SNARE, 0, KICK, 0, SNARE, 0, KICK, SNARE, KICK, SNARE, SNARE, SNARE | BSNARE, SNARE | OHAT, SNARE,
        ],
        boss_drive: false,
    },
    // 6: A_RET (Bars 24..28) — I - bII - iv - bVI high-voltage return
    Section4 {
        name: "A_RET",
        chords: [LB_B_HI, LB_B_HI, LB_C, LB_C, LB_EM, LB_EM, LB_G, LB_G],
        bass: [
            0, R, 0, 12, 0, R, 4, 7, 0, R, 0, 12, 1, R, 0, 7,
            1, R, 1, 13, 1, R, 5, 8, 1, R, 1, 13, 8, R, 5, 1,
            5, R, 5, 12, 5, R, 8, 12, 5, R, 5, 12, 8, R, 5, 7,
            8, R, 8, 15, 8, R, 12, R, 8, R, 12, R, 15, R, 8, R,
        ],
        lead: [
            16, H, 13, 12, 16, H, 19, 16, 12, H, 7, 12, 16, H, 13, 12,
            17, H, 13, 12, 17, H, 20, 17, 13, H, 8, 12, 13, H, 17, R,
            17, H, 19, 17, 12, H, 17, 19, 20, H, 19, 17, 12, H, 8, 12,
            19, H, 15, 12, 19, H, 20, 19, 15, H, 12, 15, 19, H, 15, R,
        ],
        counter: [
            12, H, 8, 7, 12, H, 12, 7, 7, H, 4, 7, 12, H, 8, 7,
            13, H, 8, 5, 13, H, 13, 8, 8, H, 5, 8, 8, H, 13, R,
            12, H, 12, 8, 8, H, 12, 12, 12, H, 12, 8, 8, H, 5, 8,
            12, H, 8, 3, 12, H, 15, 12, 8, H, 3, 8, 12, H, 8, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, OHAT, KICK,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, BKICK, KICK, 0, SNARE, 0, HAT, HAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, OHAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, HAT, KICK, SNARE, 0, KICK, SNARE, KICK, BSNARE, HAT, KICK, SNARE, BSNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
    // 7: TURN (Bars 28..32) — bvii - bVI - bII - bII->I exotic Phrygian turnaround into Bar 0 B
    Section4 {
        name: "TURN",
        chords: [LB_AM, LB_AM, LB_G, LB_G, LB_C, LB_C, LB_FSDIM, LB_C],
        bass: [
            10, R, 10, 5, 10, R, 1, R, 10, R, 8, R, 5, R, 10, R,
            8, R, 8, 12, 8, R, 3, R, 8, R, 7, R, 3, R, 8, R,
            1, R, 1, 8, 1, R, 5, R, 1, R, 8, R, 5, R, 1, R,
            7, R, 10, R, 1, R, 7, R, 1, R, 5, R, 1, R, 1, R,
        ],
        lead: [
            17, H, 13, 10, 13, H, 17, 13, 10, H, 5, 10, 13, H, 10, R,
            15, H, 12, 8, 12, H, 15, 12, 8, H, 3, 8, 12, H, 8, R,
            13, H, 12, 8, 5, H, 8, 13, 17, H, 13, 12, 8, H, 5, R,
            13, H, 10, 7, 10, H, 13, 10, 8, H, 5, 1, 4, R, 1, R,
        ],
        counter: [
            10, H, 5, 1, 5, H, 10, 5, 5, H, 1, 5, 10, H, 5, R,
            8, H, 3, 8, 8, H, 12, 8, 3, H, 0, 3, 8, H, 3, R,
            8, H, 5, 1, 1, H, 5, 8, 13, H, 8, 5, 5, H, 1, R,
            7, H, 1, 7, 7, H, 10, 7, 5, H, 1, 5, 1, R, -2, R,
        ],
        drums: [
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, BHAT, KICK, 0, HAT, KICK, SNARE, 0, OHAT, 0,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, BKICK, HAT, 0, SNARE, 0, HAT, BHAT,
            KICK, 0, HAT, BHAT, SNARE, 0, HAT, KICK, KICK, 0, OHAT, 0, SNARE, 0, HAT, KICK,
            KICK, 0, SNARE, 0, KICK, BKICK, SNARE, 0, KICK, BSNARE, SNARE, 0, KICK, SNARE, SNARE | OHAT, 0,
        ],
        boss_drive: true,
    },
];

fn track_for(theme: Theme) -> TrackCfg {
    match theme {
        Theme::Sewer => TrackCfg {
            key_name: "C Minor",
            root: 130.81, // C3
            bpm: 104.0,
            lead_duty: 0.25,
            lead_lp: 2300.0,
            counter_duty: 0.50,
            counter_lp: 1750.0,
            chord_pulse_duty: 0.25,
            chord_pulse_mix: 0.36,
            chord_lp: 1450.0,
            sections: &SEWER_SECTIONS,
        },
        Theme::Kitchen => TrackCfg {
            key_name: "D Mixolydian",
            root: 146.83, // D3
            bpm: 126.0,
            lead_duty: 0.36,
            lead_lp: 2850.0,
            counter_duty: 0.25,
            counter_lp: 2150.0,
            chord_pulse_duty: 0.34,
            chord_pulse_mix: 0.38,
            chord_lp: 1700.0,
            sections: &KITCHEN_SECTIONS,
        },
        Theme::Lab => TrackCfg {
            key_name: "B Phrygian Dominant",
            root: 123.47, // B2
            bpm: 114.0,
            lead_duty: 0.18,
            lead_lp: 2600.0,
            counter_duty: 0.38,
            counter_lp: 1900.0,
            chord_pulse_duty: 0.20,
            chord_pulse_mix: 0.44,
            chord_lp: 1650.0,
            sections: &LAB_SECTIONS,
        },
    }
}

/// Legacy scale-degree frequency helper kept for compatibility and tests.
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

#[inline]
fn semi_freq(root: f32, semi: i8) -> f32 {
    root * 2f32.powf(semi as f32 / 12.0)
}

#[inline(always)]
fn wrap_phase(p: f32) -> f32 {
    if p >= 1.0 {
        p - 1.0
    } else {
        p
    }
}

#[inline(always)]
fn fast_tri(p: f32) -> f32 {
    2.0 * (2.0 * p - 1.0).abs() - 1.0
}

/// Fast, branch-light sine approximation on phase `p in [0, 1)` (max error < 0.0011,
/// exact 0 at 0 and 0.5, exact ±1 at 0.25 and 0.75, zero libm calls).
#[inline(always)]
fn fast_sin_unit(p: f32) -> f32 {
    let u = p * 2.0;
    let y = if u < 1.0 {
        4.0 * u * (1.0 - u)
    } else {
        let t = u - 1.0;
        -4.0 * t * (1.0 - t)
    };
    0.225 * (y * y.abs() - y) + y
}

/// Division-free PolyBLEP residual using precomputed `inv_dt = 1.0 / dt`.
#[inline(always)]
fn poly_blep_fast(t: f32, dt: f32, inv_dt: f32) -> f32 {
    if t < dt {
        let u = t * inv_dt;
        u + u - u * u - 1.0
    } else if t > 1.0 - dt {
        let u = (t - 1.0) * inv_dt;
        u * u + u + u + 1.0
    } else {
        0.0
    }
}

/// DC-centered, division-free PolyBLEP pulse oscillator (zero DC bias at every duty cycle).
#[inline(always)]
fn fast_pulse_dc_free(p: f32, dt: f32, inv_dt: f32, duty: f32, dc_bias: f32) -> f32 {
    let raw = if p < duty { 1.0 } else { -1.0 };
    let mut p2 = p - duty;
    if p2 < 0.0 {
        p2 += 1.0;
    }
    let blep = poly_blep_fast(p, dt, inv_dt) - poly_blep_fast(p2, dt, inv_dt);
    (raw + blep) - dc_bias
}

/// Fast rational Padé soft clipper approximating `tanh(x)` without transcendentals.
#[inline(always)]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct MusicVoiceFrame {
    bass: f32,
    chord: [f32; 3],
    lead: f32,
    counter: f32,
    drums: f32,
    mix: f32,
}

/// An infinite polyphonic themed chiptune source (32-bar / 512-step arrangement).
///
/// `intensity` smoothly crossfades layers without clicks:
/// - `0` (Calm): warm legato Bass + 3-voice Chord Pad (4 simultaneous pitched voices)
/// - `1` (Normal): + Lead Melody + light kick/hat groove (5 pitched voices)
/// - `2` (Horde): + Counter-Melody harmony line + full driving drums (6 pitched voices)
/// - `3` (Boss Overdrive): + driving octave bass pulse, gated chord energy, bright vibrato lead & double-kick/fill drums
pub struct MusicSynth {
    cfg: TrackCfg,
    intensity: Arc<AtomicU32>,
    cached_intensity: u32,
    counter: u64,
    samples_per_step: u64,
    inv_sps: f32,
    step_idx: u64,
    step_pos: u32,
    cached_step: u64,

    // Oscillator phases (continuous across all steps & loop seam -> zero phase clicks)
    bass_phase: f32,
    chord_phase: [f32; 3],
    lead_phase: f32,
    counter_phase: f32,
    vib_phase: f32,
    kick_phase: f32,
    snare_body_phase: f32,

    // Precomputed per-step phase increments (`freq / SR_F`), reciprocals & frequencies
    bass_freq: Option<f32>,
    bass_dt: f32,
    bass_active_semi: i8,
    chord_freqs: [f32; 3],
    chord_dt: [f32; 3],
    chord2_inv_dt: f32,
    prev_chord_tones: [i8; 3],
    lead_freq: Option<f32>,
    lead_dt: f32,
    lead_inv_dt: f32,
    counter_freq: Option<f32>,
    counter_dt: f32,
    counter_inv_dt: f32,

    // Precomputed pulse DC bias offsets (`2 * duty - 1`)
    lead_dc_bias: f32,
    counter_dc_bias: f32,
    chord_dc_bias: f32,

    // Envelopes & note durations
    bass_env: f32,
    bass_attack_rem: u16,
    bass_boss_env: f32,
    bass_boss_att: u16,
    chord_env: f32,
    lead_active: bool,
    lead_note_pos: u32,
    lead_note_len: u32,
    lead_inv_len: f32,
    counter_active: bool,
    counter_note_pos: u32,
    counter_note_len: u32,
    counter_inv_len: f32,

    // Drum states (with micro-attack ramps to guarantee click-free transients)
    kick_freq: f32,
    kick_env: f32,
    kick_att: u16,
    kick_boss_only: bool,
    snare_env: f32,
    snare_att: u16,
    snare_body_freq: f32,
    snare_boss_only: bool,
    hat_env: f32,
    hat_att: u16,
    hat_decay: f32,
    hat_boss_only: bool,

    // 1-pole warmth filters
    lead_lp_state: f32,
    lead_lp_alpha: f32,
    counter_lp_state: f32,
    counter_lp_alpha: f32,
    chord_lp_state: f32,
    chord_lp_alpha: f32,
    noise_hp_prev_x: f32,
    noise_hp_prev_y: f32,

    // Smooth intensity crossfades & cached control-rate gains
    lead_mix: f32,
    counter_mix: f32,
    drum_mix: f32,
    boss_mix: f32,
    ctrl_chord_gain: f32,
    ctrl_chord_pmix: f32,
    ctrl_lead_lp: f32,
    ctrl_lead_gain: f32,
    ctrl_ctr_gain: f32,

    // Deterministic drum PRNG
    seed: u32,
}

impl MusicSynth {
    pub fn new(theme: Theme, intensity: Arc<AtomicU32>) -> Self {
        let cfg = track_for(theme);
        let samples_per_step = ((SR_F * 60.0 / (cfg.bpm * 4.0)) as u64).max(1);
        let lead_lp_alpha = (TAU * cfg.lead_lp / SR_F).clamp(0.05, 0.85);
        let counter_lp_alpha = (TAU * cfg.counter_lp / SR_F).clamp(0.05, 0.85);
        let chord_lp_alpha = (TAU * cfg.chord_lp / SR_F).clamp(0.05, 0.85);
        let init_int = intensity.load(Ordering::Relaxed);
        let init_bass_hz = cfg.root * 0.5;
        let init_chord = cfg.sections[0].chords[0].tones;
        let init_chord_freqs = [
            semi_freq(cfg.root, init_chord[0]),
            semi_freq(cfg.root, init_chord[1]),
            semi_freq(cfg.root, init_chord[2]),
        ];
        let inv_sr = 1.0 / SR_F;
        let c2_dt = (init_chord_freqs[2] * inv_sr).clamp(1e-5, 0.45);
        let lead_mix = if init_int >= 1 { 1.0 } else { 0.0 };
        let counter_mix = if init_int >= 2 { 1.0 } else { 0.0 };
        let drum_mix = if init_int >= 2 { 1.0 } else { 0.0 };
        let boss_mix = if init_int >= 3 { 1.0 } else { 0.0 };
        let lead_dc_bias = 2.0 * cfg.lead_duty - 1.0;
        let counter_dc_bias = 2.0 * cfg.counter_duty - 1.0;
        let chord_dc_bias = 2.0 * cfg.chord_pulse_duty - 1.0;
        let chord_pmix = (cfg.chord_pulse_mix + 0.18 * boss_mix).min(0.68);
        MusicSynth {
            cfg,
            intensity,
            cached_intensity: init_int,
            counter: 0,
            samples_per_step,
            inv_sps: 1.0 / samples_per_step as f32,
            step_idx: 0,
            step_pos: 0,
            cached_step: u64::MAX,
            bass_phase: 0.0,
            chord_phase: [0.0; 3],
            lead_phase: 0.0,
            counter_phase: 0.0,
            vib_phase: 0.0,
            kick_phase: 0.0,
            snare_body_phase: 0.0,
            bass_freq: None,
            bass_dt: init_bass_hz * inv_sr,
            bass_active_semi: 0,
            chord_freqs: init_chord_freqs,
            chord_dt: [
                init_chord_freqs[0] * inv_sr,
                init_chord_freqs[1] * inv_sr,
                c2_dt,
            ],
            chord2_inv_dt: 1.0 / c2_dt,
            prev_chord_tones: [i8::MIN; 3],
            lead_freq: None,
            lead_dt: 0.0,
            lead_inv_dt: 1.0,
            counter_freq: None,
            counter_dt: 0.0,
            counter_inv_dt: 1.0,
            lead_dc_bias,
            counter_dc_bias,
            chord_dc_bias,
            bass_env: 0.0,
            bass_attack_rem: 0,
            bass_boss_env: 0.0,
            bass_boss_att: 0,
            chord_env: 0.0,
            lead_active: false,
            lead_note_pos: 0,
            lead_note_len: samples_per_step as u32,
            lead_inv_len: 1.0 / samples_per_step as f32,
            counter_active: false,
            counter_note_pos: 0,
            counter_note_len: samples_per_step as u32,
            counter_inv_len: 1.0 / samples_per_step as f32,
            kick_freq: 46.0,
            kick_env: 0.0,
            kick_att: 0,
            kick_boss_only: false,
            snare_env: 0.0,
            snare_att: 0,
            snare_body_freq: 170.0,
            snare_boss_only: false,
            hat_env: 0.0,
            hat_att: 0,
            hat_decay: 0.9966,
            hat_boss_only: false,
            lead_lp_state: 0.0,
            lead_lp_alpha,
            counter_lp_state: 0.0,
            counter_lp_alpha,
            chord_lp_state: 0.0,
            chord_lp_alpha,
            noise_hp_prev_x: 0.0,
            noise_hp_prev_y: 0.0,
            lead_mix,
            counter_mix,
            drum_mix,
            boss_mix,
            ctrl_chord_gain: 0.056,
            ctrl_chord_pmix: chord_pmix,
            ctrl_lead_lp: (lead_lp_alpha * (1.0 + 0.30 * boss_mix)).min(0.88),
            ctrl_lead_gain: (0.112 + 0.022 * boss_mix) * lead_mix,
            ctrl_ctr_gain: (0.082 + 0.016 * boss_mix) * counter_mix,
            seed: 0x9E37_79B9,
        }
    }

    #[inline]
    fn step_note_at(&self, loop_step: usize, track: u8) -> i8 {
        let sec = &self.cfg.sections[(loop_step / SECTION_STEPS) % NUM_SECTIONS];
        let local = loop_step % SECTION_STEPS;
        match track {
            0 => sec.bass[local],
            1 => sec.lead[local],
            _ => sec.counter[local],
        }
    }

    /// Count how many 16th-note steps a newly triggered note holds (1 + consecutive `H` steps).
    fn count_hold_steps(&self, loop_step: usize, track: u8) -> u32 {
        let mut count = 1u32;
        for ahead in 1..16usize {
            let next_step = (loop_step + ahead) % LOOP_STEPS;
            if self.step_note_at(next_step, track) == H {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    #[inline]
    fn chord_at_step(&self, loop_step: usize) -> ChordSlot {
        let sec = &self.cfg.sections[(loop_step / SECTION_STEPS) % NUM_SECTIONS];
        let half_bar = (loop_step % SECTION_STEPS) / 8;
        sec.chords[half_bar]
    }

    /// Refresh cached per-step frequencies, reciprocals, held note lengths, and drum triggers.
    fn cache_step(&mut self, step_idx: u64) {
        let loop_step = (step_idx % LOOP_STEPS as u64) as usize;
        let sec_idx = loop_step / SECTION_STEPS;
        let local = loop_step % SECTION_STEPS;
        let sec = &self.cfg.sections[sec_idx];
        let inv_sr = 1.0 / SR_F;
        self.cached_step = step_idx;

        // 1. Chord pad (3 simultaneous pitched voices from stated half-bar chord)
        let ch = sec.chords[local / 8];
        if ch.tones != self.prev_chord_tones {
            for i in 0..3 {
                let hz = semi_freq(self.cfg.root, ch.tones[i]);
                self.chord_freqs[i] = hz;
                self.chord_dt[i] = (hz * inv_sr).clamp(1e-5, 0.45);
            }
            self.chord2_inv_dt = 1.0 / self.chord_dt[2];
            self.prev_chord_tones = ch.tones;
            // Gentle breathing swell on chord changes (never drops to 0 mid-song, never clicks)
            if self.counter > 0 {
                self.chord_env = self.chord_env.min(0.62);
            }
        }

        // 2. Bass (legato across H/R; Boss mode adds driving octave pulse on offbeats)
        let b_note = sec.bass[local];
        if b_note == R {
            self.bass_freq = None;
            if sec.boss_drive && (local & 1) == 1 {
                self.bass_boss_att = 56;
                self.bass_boss_env = self.bass_boss_env.max(0.30);
            }
        } else if b_note != H {
            let hz = semi_freq(self.cfg.root * 0.5, b_note);
            self.bass_freq = Some(hz);
            self.bass_dt = (hz * inv_sr).clamp(1e-5, 0.45);
            self.bass_active_semi = b_note;
            self.bass_attack_rem = 88;
        } else if sec.boss_drive && (local & 1) == 1 {
            self.bass_boss_att = 56;
            self.bass_boss_env = self.bass_boss_env.max(0.26);
        }

        // 3. Lead melody (with multi-step hold sustain)
        let l_note = sec.lead[local];
        if l_note == R {
            self.lead_freq = None;
            self.lead_active = false;
        } else if l_note != H {
            let hz = semi_freq(self.cfg.root * 2.0, l_note);
            let dt = (hz * inv_sr).clamp(1e-5, 0.45);
            self.lead_freq = Some(hz);
            self.lead_dt = dt;
            self.lead_inv_dt = 1.0 / dt;
            let hold = self.count_hold_steps(loop_step, 1);
            self.lead_note_len = hold * self.samples_per_step as u32;
            self.lead_inv_len = 1.0 / self.lead_note_len as f32;
            self.lead_note_pos = 0;
            self.lead_active = true;
        }

        // 4. Counter-melody / harmony line (answers lead, independent phrasing)
        let c_note = sec.counter[local];
        if c_note == R {
            self.counter_freq = None;
            self.counter_active = false;
        } else if c_note != H {
            let hz = semi_freq(self.cfg.root * 2.0, c_note);
            let dt = (hz * inv_sr).clamp(1e-5, 0.45);
            self.counter_freq = Some(hz);
            self.counter_dt = dt;
            self.counter_inv_dt = 1.0 / dt;
            let hold = self.count_hold_steps(loop_step, 2);
            self.counter_note_len = hold * self.samples_per_step as u32;
            self.counter_inv_len = 1.0 / self.counter_note_len as f32;
            self.counter_note_pos = 0;
            self.counter_active = true;
        }

        // 5. Percussion triggers (normal + Boss overdrive bits)
        let d = sec.drums[local];
        let boss_on = self.cached_intensity >= 3 || self.boss_mix > 0.02;
        if d & KICK != 0 {
            self.kick_phase = 0.0;
            self.kick_freq = 150.0;
            self.kick_env = 1.0;
            self.kick_att = 20;
            self.kick_boss_only = false;
        } else if boss_on && (d & BKICK != 0) {
            self.kick_phase = 0.0;
            self.kick_freq = 165.0;
            self.kick_env = 0.90;
            self.kick_att = 20;
            self.kick_boss_only = true;
        }
        if d & SNARE != 0 {
            self.snare_env = 1.0;
            self.snare_att = 28;
            self.snare_body_phase = 0.0;
            self.snare_body_freq = 198.0;
            self.snare_boss_only = false;
        } else if boss_on && (d & BSNARE != 0) {
            self.snare_env = 0.80;
            self.snare_att = 28;
            self.snare_body_phase = 0.0;
            self.snare_body_freq = 220.0;
            self.snare_boss_only = true;
        }
        if d & (HAT | OHAT) != 0 {
            self.hat_env = if d & OHAT != 0 { 1.0 } else { 0.84 };
            self.hat_att = 24;
            self.hat_decay = if d & OHAT != 0 { 0.9988 } else { 0.9964 };
            self.hat_boss_only = false;
        } else if boss_on && (d & BHAT != 0) {
            self.hat_env = 0.72;
            self.hat_att = 24;
            self.hat_decay = 0.9958;
            self.hat_boss_only = true;
        }
    }

    #[inline(always)]
    fn update_control(&mut self) {
        if (self.step_pos & 63) == 0 {
            self.cached_intensity = self.intensity.load(Ordering::Relaxed);
        }
        let intensity = self.cached_intensity;
        let target_lead = if intensity >= 1 { 1.0 } else { 0.0 };
        let target_ctr = if intensity >= 2 { 1.0 } else { 0.0 };
        let target_drum = if intensity >= 2 { 1.0 } else { 0.0 };
        let target_boss = if intensity >= 3 { 1.0 } else { 0.0 };
        // Control rate = SR/16 (~2756 Hz): 0.0032 gives the exact same ~180ms smooth crossfade.
        self.lead_mix += (target_lead - self.lead_mix) * 0.0032;
        self.counter_mix += (target_ctr - self.counter_mix) * 0.0032;
        self.drum_mix += (target_drum - self.drum_mix) * 0.0032;
        self.boss_mix += (target_boss - self.boss_mix) * 0.0032;

        self.chord_env += (1.0 - self.chord_env) * 0.055;
        let step_frac = self.step_pos as f32 * self.inv_sps;
        let beat_pulse = if (self.step_idx & 1) == 0 {
            1.0 - 0.22 * step_frac
        } else {
            0.80 - 0.24 * step_frac
        };
        let comp_depth = 0.12 * self.lead_mix + 0.24 * self.drum_mix + 0.42 * self.boss_mix;
        let base_c = 0.055 + 0.009 * self.boss_mix;
        self.ctrl_chord_gain = self.chord_env * (1.0 - comp_depth + comp_depth * beat_pulse) * base_c;
        self.ctrl_chord_pmix = (self.cfg.chord_pulse_mix + 0.20 * self.boss_mix).min(0.68);
        self.ctrl_lead_lp = (self.lead_lp_alpha * (1.0 + 0.30 * self.boss_mix)).min(0.88);
        self.ctrl_lead_gain = (0.112 + 0.024 * self.boss_mix) * self.lead_mix;
        self.ctrl_ctr_gain = (0.082 + 0.018 * self.boss_mix) * self.counter_mix;
    }

    #[inline(always)]
    fn note_env(pos: u32, len: u32, inv_len: f32) -> f32 {
        let att = if pos < 128 {
            pos as f32 * (1.0 / 128.0)
        } else {
            1.0
        };
        let sustain = 1.0 - 0.50 * (pos as f32 * inv_len);
        let rem = len.saturating_sub(pos);
        let tail = if rem < 110 {
            rem as f32 * (1.0 / 110.0)
        } else {
            1.0
        };
        att * sustain.max(0.0) * tail
    }

    #[inline(always)]
    fn synth_core<const CAPTURE: bool>(&mut self) -> (f32, [f32; 3], f32, f32, f32, f32) {
        if self.cached_step != self.step_idx {
            self.cache_step(self.step_idx);
        }
        if (self.step_pos & 15) == 0 {
            self.update_control();
        }

        let master_fade = if self.counter < 256 {
            self.counter as f32 * (1.0 / 256.0)
        } else {
            1.0
        };

        // 1. Warm legato Bass (soft-saturated triangle — rich harmonic body, 0.0 DC, zero branches)
        if self.bass_attack_rem > 0 {
            self.bass_env += (1.0 - self.bass_env) * 0.06;
            self.bass_attack_rem -= 1;
        } else {
            self.bass_env *= 0.999_86;
        }
        let mut bass_out = 0.0f32;
        if self.bass_env > 0.0008 {
            self.bass_phase = wrap_phase(self.bass_phase + self.bass_dt);
            let tri = fast_tri(self.bass_phase);
            let warm = tri * (1.24 - 0.24 * tri * tri);
            bass_out = warm * 0.165 * self.bass_env;
        }
        if self.boss_mix > 0.001 {
            if self.bass_boss_att > 0 {
                self.bass_boss_env += (1.0 - self.bass_boss_env) * 0.08;
                self.bass_boss_att -= 1;
            } else {
                self.bass_boss_env *= 0.9993;
            }
            if self.bass_boss_env > 0.001 {
                let p2 = wrap_phase(self.bass_phase + self.bass_phase);
                let t2 = fast_tri(p2);
                bass_out += t2 * 0.105 * self.bass_boss_env * self.boss_mix;
            }
        }

        // 2. 3-Voice Polyphonic Chord Pad / Chiptune Comping (always 3 simultaneous tones!)
        let cg = self.ctrl_chord_gain;
        let p0 = wrap_phase(self.chord_phase[0] + self.chord_dt[0]);
        let p1 = wrap_phase(self.chord_phase[1] + self.chord_dt[1]);
        let p2 = wrap_phase(self.chord_phase[2] + self.chord_dt[2]);
        self.chord_phase = [p0, p1, p2];

        let c0 = fast_tri(p0) * cg;
        let c1 = fast_tri(p1) * (cg * 0.94);
        let c2_pulse = fast_pulse_dc_free(
            p2,
            self.chord_dt[2],
            self.chord2_inv_dt,
            self.cfg.chord_pulse_duty,
            self.chord_dc_bias,
        );
        self.chord_lp_state += self.chord_lp_alpha * (c2_pulse - self.chord_lp_state);
        let pmix = self.ctrl_chord_pmix;
        let c2 = (fast_tri(p2) * (1.0 - pmix) + self.chord_lp_state * pmix) * (cg * 0.96);

        // 3. Band-limited, DC-free Lead Melody (enters at int >= 1; vibrato + brighter in Boss)
        let mut lead_out = 0.0f32;
        if self.lead_active && self.ctrl_lead_gain > 0.0005 {
            let mut dt = self.lead_dt;
            if self.lead_note_pos > 4400 {
                self.vib_phase = wrap_phase(self.vib_phase + (5.6 / SR_F));
                let vib_depth = 0.0038 + 0.0035 * self.boss_mix;
                dt *= 1.0 + fast_sin_unit(self.vib_phase) * vib_depth;
            }
            self.lead_phase = wrap_phase(self.lead_phase + dt);
            let env = Self::note_env(self.lead_note_pos, self.lead_note_len, self.lead_inv_len);
            let raw = fast_pulse_dc_free(
                self.lead_phase,
                dt,
                self.lead_inv_dt,
                self.cfg.lead_duty,
                self.lead_dc_bias,
            );
            self.lead_lp_state += self.ctrl_lead_lp * (raw - self.lead_lp_state);
            lead_out = self.lead_lp_state * self.ctrl_lead_gain * env;
            self.lead_note_pos += 1;
        } else {
            self.lead_lp_state *= 0.985;
        }

        // 4. Counter-Melody / Harmony Line (answers lead; enters at int >= 2)
        let mut counter_out = 0.0f32;
        if self.counter_active && self.ctrl_ctr_gain > 0.0005 {
            self.counter_phase = wrap_phase(self.counter_phase + self.counter_dt);
            let env = Self::note_env(
                self.counter_note_pos,
                self.counter_note_len,
                self.counter_inv_len,
            );
            let raw = fast_pulse_dc_free(
                self.counter_phase,
                self.counter_dt,
                self.counter_inv_dt,
                self.cfg.counter_duty,
                self.counter_dc_bias,
            );
            self.counter_lp_state += self.counter_lp_alpha * (raw - self.counter_lp_state);
            counter_out = self.counter_lp_state * self.ctrl_ctr_gain * env;
            self.counter_note_pos += 1;
        } else {
            self.counter_lp_state *= 0.985;
        }

        // 5. Chiptune Groove Percussion (Kick, Snare w/ body, Closed/Open Hat, Boss Overdrive)
        let mut drums_out = 0.0f32;
        if self.kick_env > 0.0005 {
            let kick_scale = if self.kick_boss_only {
                0.175 * self.boss_mix
            } else {
                0.065 * self.lead_mix + 0.115 * self.drum_mix + 0.035 * self.boss_mix
            };
            if kick_scale > 0.001 {
                self.kick_phase = wrap_phase(self.kick_phase + self.kick_freq * (1.0 / SR_F));
                let att = if self.kick_att > 0 {
                    self.kick_att -= 1;
                    (20 - self.kick_att) as f32 * (1.0 / 20.0)
                } else {
                    1.0
                };
                drums_out += fast_sin_unit(self.kick_phase) * self.kick_env * att * kick_scale;
                self.kick_freq = 44.0 + (self.kick_freq - 44.0) * 0.9964;
                self.kick_env *= 0.9990;
            }
        }

        let need_snare = self.snare_env > 0.0005
            && (if self.snare_boss_only {
                self.boss_mix
            } else {
                self.drum_mix
            }) > 0.001;
        let need_hat = self.hat_env > 0.0005
            && (if self.hat_boss_only {
                self.boss_mix
            } else {
                self.lead_mix
            }) > 0.001;

        if need_snare || need_hat {
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = ((self.seed >> 9) as f32 * (1.0 / (1u32 << 23) as f32)) * 2.0 - 1.0;
            // Band-shaped crisp chiptune noise: 1-pole HP + gentle 2-tap Nyquist smoothing
            // keeps snare/hat bright & punchy while eliminating harsh single-sample Nyquist spikes.
            let prev_hp = self.noise_hp_prev_y;
            let hp = 0.72 * (prev_hp + white - self.noise_hp_prev_x);
            self.noise_hp_prev_x = white;
            self.noise_hp_prev_y = hp;
            let crisp = 0.58 * hp + 0.42 * prev_hp;

            if need_snare {
                let att = if self.snare_att > 0 {
                    self.snare_att -= 1;
                    (28 - self.snare_att) as f32 * (1.0 / 28.0)
                } else {
                    1.0
                };
                let smix = if self.snare_boss_only {
                    self.boss_mix
                } else {
                    self.drum_mix
                };
                self.snare_body_phase =
                    wrap_phase(self.snare_body_phase + self.snare_body_freq * (1.0 / SR_F));
                self.snare_body_freq = 112.0 + (self.snare_body_freq - 112.0) * 0.997;
                let body = fast_sin_unit(self.snare_body_phase) * 0.34;
                let noise = crisp * 0.52 + white * 0.20;
                let sgain = 0.102 + 0.018 * self.boss_mix;
                drums_out += (noise + body) * self.snare_env * att * sgain * smix;
                self.snare_env *= 0.9985;
            }
            if need_hat {
                let att = if self.hat_att > 0 {
                    self.hat_att -= 1;
                    (24 - self.hat_att) as f32 * (1.0 / 24.0)
                } else {
                    1.0
                };
                let hgain = if self.hat_boss_only {
                    0.058 * self.boss_mix
                } else {
                    0.022 * self.lead_mix + 0.042 * self.drum_mix + 0.010 * self.boss_mix
                };
                drums_out += crisp * self.hat_env * att * hgain;
                self.hat_env *= self.hat_decay;
            }
        }

        // Advance step & sample counters without 64-bit integer division
        self.counter += 1;
        self.step_pos += 1;
        if self.step_pos as u64 >= self.samples_per_step {
            self.step_pos = 0;
            self.step_idx += 1;
        }

        let sum = (bass_out + c0 + c1 + c2 + lead_out + counter_out + drums_out) * master_fade;
        // Soft bus ceiling keeps total music well inside [-0.52, 0.52] with generous SFX headroom.
        let mix = (fast_tanh(sum * 1.05) * 0.76).clamp(-1.0, 1.0);
        if CAPTURE {
            (
                bass_out * master_fade,
                [c0 * master_fade, c1 * master_fade, c2 * master_fade],
                lead_out * master_fade,
                counter_out * master_fade,
                drums_out * master_fade,
                mix,
            )
        } else {
            (0.0, [0.0; 3], 0.0, 0.0, 0.0, mix)
        }
    }

    #[cfg(test)]
    fn next_voices(&mut self) -> MusicVoiceFrame {
        let (bass, chord, lead, counter, drums, mix) = self.synth_core::<true>();
        MusicVoiceFrame {
            bass,
            chord,
            lead,
            counter,
            drums,
            mix,
        }
    }
}

impl Iterator for MusicSynth {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        Some(self.synth_core::<false>().5)
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

// ---- Volume --------------------------------------------------------------

/// Music gain at the 100% setting — today's fixed music level.
pub const MUSIC_MAX_GAIN: f32 = 0.50;
/// Effects gain at the 100% setting — today's fixed effect level.
pub const SFX_MAX_GAIN: f32 = 0.85;
/// Slider granularity, in percent.
pub const VOLUME_STEP: u8 = 5;

/// Round a volume setting into `0..=100` on the [`VOLUME_STEP`] grid.
pub const fn snap_percent(percent: u8) -> u8 {
    let clamped = if percent > 100 { 100 } else { percent };
    ((clamped + VOLUME_STEP / 2) / VOLUME_STEP) * VOLUME_STEP
}

/// Map a `0..=100` setting to a `0..=1` amplitude factor.
///
/// **Squared curve**: `gain = (percent/100)²`. Perceived loudness tracks the
/// square root of amplitude, so a linear amplitude slider feels like it does
/// nothing over its top half and falls off a cliff at the bottom; squaring
/// makes each slider step feel roughly equal (6 dB per 30%, 12 dB per halving)
/// and hits exactly `0.0` at 0 instead of the −40 dB floor a dB curve leaves.
pub fn volume_curve(percent: u8) -> f32 {
    let x = f32::from(snap_percent(percent)) / 100.0;
    x * x
}

/// Resolved music amplitude for a `0..=100` setting (`0.0` at 0).
pub fn music_gain(percent: u8) -> f32 {
    MUSIC_MAX_GAIN * volume_curve(percent)
}

/// Resolved effects amplitude for a `0..=100` setting (`0.0` at 0).
pub fn effects_gain(percent: u8) -> f32 {
    SFX_MAX_GAIN * volume_curve(percent)
}

/// A sink whose amplitude the mixer can push to. `Player` is the real one;
/// tests substitute a recorder, so the gain path is exercised with no device.
pub trait GainSink {
    fn set_gain(&self, gain: f32);
}

impl GainSink for Player {
    fn set_gain(&self, gain: f32) {
        self.set_volume(gain);
    }
}

/// The player's two independent channel levels plus the master mute on top.
///
/// Device-free on purpose: `AudioEngine::new()` returns `None` without an
/// output device (every test and headless run), so all the level/mute logic
/// and the gain resolution live here where it can be tested directly, and the
/// engine only owns the sinks that receive [`VolumeMixer::apply`].
pub struct VolumeMixer {
    music: u8,
    effects: u8,
    muted: bool,
}

impl Default for VolumeMixer {
    /// 100% / 100% unmuted: exactly the pre-slider balance.
    fn default() -> Self {
        VolumeMixer {
            music: 100,
            effects: 100,
            muted: false,
        }
    }
}

impl VolumeMixer {
    pub fn new(music: u8, effects: u8) -> Self {
        VolumeMixer {
            music: snap_percent(music),
            effects: snap_percent(effects),
            muted: false,
        }
    }

    pub fn music(&self) -> u8 {
        self.music
    }

    pub fn effects(&self) -> u8 {
        self.effects
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// Set the music level; returns `true` when it actually moved.
    pub fn set_music(&mut self, percent: u8) -> bool {
        let next = snap_percent(percent);
        let changed = next != self.music;
        self.music = next;
        changed
    }

    /// Set the effects level; returns `true` when it actually moved.
    pub fn set_effects(&mut self, percent: u8) -> bool {
        let next = snap_percent(percent);
        let changed = next != self.effects;
        self.effects = next;
        changed
    }

    /// Flip the master mute, keeping both levels underneath.
    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Music amplitude with mute applied (`0.0` while muted).
    pub fn music_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            music_gain(self.music)
        }
    }

    /// Effects amplitude with mute applied (`0.0` while muted).
    pub fn effects_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            effects_gain(self.effects)
        }
    }

    /// Push the resolved gains to the music sink and every live effect voice.
    /// The same function runs in the engine (real sinks) and in tests
    /// (recording sinks), so both channels and *already playing* effects are
    /// covered by one code path.
    pub fn apply<'a>(&self, music: &dyn GainSink, voices: impl Iterator<Item = &'a dyn GainSink>) {
        music.set_gain(self.music_gain());
        let gain = self.effects_gain();
        for voice in voices {
            voice.set_gain(gain);
        }
    }
}

// ---- Engine --------------------------------------------------------------

/// Effects play on pooled sinks so they can overlap and so mute can reach
/// the ones already sounding. Cap the live voices so a kill burst can never
/// spawn unbounded sinks/mixer tracks.
const MAX_SFX_VOICES: usize = 24;
/// Window in milliseconds during which consecutive gem pickups climb the scale.
const GEM_COMBO_WINDOW_MS: u64 = 480;

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

/// The next variant to play for `sfx`, or `None` while the background cache is
/// still being synthesized. Never blocks and never panics: a cache that is not
/// ready simply drops the sound.
fn cached_sample(
    cache: &OnceLock<SfxBank>,
    picker: &VariantPicker,
    sfx: Sfx,
    now_ms: u64,
) -> Option<Arc<[f32]>> {
    let bank = cache.get()?;
    let variants = bank.get(sfx.index())?;
    if variants.is_empty() {
        return None;
    }
    let v_idx = picker.pick(sfx, now_ms) % variants.len();
    let samples = &variants[v_idx];
    if samples.is_empty() {
        return None;
    }
    Some(Arc::clone(samples))
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

/// One bank of pre-synthesized variants per `Sfx`, indexed by `Sfx::index`.
type SfxBank = Vec<Vec<Arc<[f32]>>>;

pub struct AudioEngine {
    stream: MixerDeviceSink,
    music: Player,
    /// Pooled sinks for in-flight effects: idle slots are reused without
    /// allocating new mixer tracks, and high-priority alerts can steal from
    /// low-priority chatter when saturated.
    sfx_voices: Mutex<Vec<VoiceSlot>>,
    /// Pre-synthesized variant banks for every `Sfx` (indexed by `Sfx::index`).
    ///
    /// A background thread fills this while the first frame draws: until it
    /// publishes, `play_sfx` drops the request instead of blocking. Music needs
    /// no cache and starts immediately.
    cache: Arc<OnceLock<SfxBank>>,
    picker: VariantPicker,
    started_at: Instant,
    intensity: Arc<AtomicU32>,
    /// The player's music/effects levels and the master mute on top. Owns the
    /// resolved gains; every change is pushed to the live sinks through
    /// [`VolumeMixer::apply`].
    levels: VolumeMixer,
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
            cache: Self::spawn_cache(),
            picker: VariantPicker::new(),
            started_at: Instant::now(),
            intensity: Arc::new(AtomicU32::new(1)),
            levels: VolumeMixer::default(),
        })
    }

    /// Synthesize every effect and its variants exactly once.
    fn build_cache() -> SfxBank {
        Sfx::ALL
            .iter()
            .map(|&sfx| {
                (0..sfx.variant_count())
                    .map(|v| Arc::from(sfx_variant_samples(sfx, v).into_boxed_slice()))
                    .collect()
            })
            .collect()
    }

    /// Synthesize the cache on a background thread and return the slot it will
    /// publish into. The first frame no longer waits for ~150 buffers; effects
    /// requested before the thread finishes are silently skipped (see
    /// [`Self::play_sfx_pitched`]). If the thread cannot be spawned (rare),
    /// fall back to building in place so audio still works.
    fn spawn_cache() -> Arc<OnceLock<SfxBank>> {
        let cache: Arc<OnceLock<SfxBank>> = Arc::new(OnceLock::new());
        let slot = Arc::clone(&cache);
        let spawned = std::thread::Builder::new()
            .name("ratrun-audio-cache".to_string())
            .spawn(move || {
                let _ = slot.set(Self::build_cache());
            });
        if spawned.is_err() {
            let _ = cache.set(Self::build_cache());
        }
        cache
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
        let muted = self.levels.toggle_mute();
        // Reach the sinks that are already playing, not just future sounds.
        self.apply_volumes();
        muted
    }

    pub fn is_muted(&self) -> bool {
        self.levels.muted()
    }

    /// Set the master mute explicitly (the app restores a saved state through
    /// this instead of toggling blind). Reaches the live sinks at once.
    pub fn set_muted(&mut self, muted: bool) {
        if self.levels.muted() != muted {
            self.levels.set_muted(muted);
            self.apply_volumes();
        }
    }

    /// Music level, 0..=100 in steps of 5.
    pub fn music_volume(&self) -> u8 {
        self.levels.music()
    }

    /// Effects level, 0..=100 in steps of 5.
    pub fn effects_volume(&self) -> u8 {
        self.levels.effects()
    }

    /// Set the music level and restart no audio: the playing track's sink takes
    /// the new gain immediately.
    pub fn set_music_volume(&mut self, percent: u8) {
        if self.levels.set_music(percent) {
            self.apply_volumes();
        }
    }

    /// Set the effects level and push it to the voices already sounding (and to
    /// every voice spawned from now on, which is initialised from the same
    /// [`Self::effect_voice_gain`]).
    pub fn set_effects_volume(&mut self, percent: u8) {
        if self.levels.set_effects(percent) {
            self.apply_volumes();
        }
    }

    /// The gain a freshly created effect voice starts at.
    fn effect_voice_gain(&self) -> f32 {
        self.levels.effects_gain()
    }

    /// Push the resolved music/effects gains to every live sink.
    fn apply_volumes(&self) {
        let Ok(voices) = self.sfx_voices.lock() else {
            return;
        };
        self.levels.apply(
            &self.music,
            voices.iter().map(|v| &v.player as &dyn GainSink),
        );
    }

    pub fn play_sfx(&self, sfx: Sfx) {
        self.play_sfx_pitched(sfx, 1.0);
    }

    /// Play `sfx` with an optional playback pitch multiplier in `[0.5, 2.0]`.
    ///
    /// A no-op while the background cache is still synthesizing: dropping a
    /// sound is better than stalling the frame that produced it.
    pub fn play_sfx_pitched(&self, sfx: Sfx, pitch_ratio: f32) {
        let now_ms = self.started_at.elapsed().as_millis() as u64 + 1;
        let Some(samples) = cached_sample(&self.cache, &self.picker, sfx, now_ms) else {
            return;
        };

        let Ok(mut voices) = self.sfx_voices.lock() else {
            return;
        };
        let vol = self.effect_voice_gain();
        let src = if (pitch_ratio - 1.0).abs() < 1e-4 {
            CachedSfx::new(samples)
        } else {
            CachedSfx::with_pitch(samples, pitch_ratio)
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
                (25.0..=2500.0).contains(&dur_ms),
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

        // New content mix hierarchy: quiet cues stay under heavy slams, roars, and deaths.
        assert!(
            peak(&sfx_samples(Sfx::TrapTrigger)) * 2.0 < peak(&sfx_samples(Sfx::TrapSnap)),
            "TrapTrigger should be noticeably quieter than TrapSnap"
        );
        assert!(peak(&sfx_samples(Sfx::GateOpen)) < peak(&sfx_samples(Sfx::GateSlam)));
        assert!(peak(&sfx_samples(Sfx::GateRefuse)) < peak(&sfx_samples(Sfx::GateSlam)));
        assert!(peak(&sfx_samples(Sfx::CatGrowl)) < peak(&sfx_samples(Sfx::CatRoar)));
        assert!(peak(&sfx_samples(Sfx::CatGrowl)) < peak(&sfx_samples(Sfx::CatLand)));
        assert!(peak(&sfx_samples(Sfx::BroomHop)) < peak(&sfx_samples(Sfx::BroomStomp)));
        assert!(peak(&sfx_samples(Sfx::MowerIdle)) < peak(&sfx_samples(Sfx::MowerCharge)));
        assert!(peak(&sfx_samples(Sfx::MowerCharge)) < peak(&sfx_samples(Sfx::MowerWallClang)));
        assert!(peak(&sfx_samples(Sfx::MowerWallClang)) <= peak(&sfx_samples(Sfx::MowerDeath)));

        // Campaign progression fanfare durations: CampaignVictory > LevelComplete > Win.
        let win_len = sfx_samples(Sfx::Win).len();
        let lvl_len = sfx_samples(Sfx::LevelComplete).len();
        let camp_len = sfx_samples(Sfx::CampaignVictory).len();
        assert!(
            lvl_len > win_len,
            "LevelComplete ({lvl_len}) should be longer than Win ({win_len})"
        );
        assert!(
            camp_len > lvl_len && camp_len as f32 > win_len as f32 * 1.8,
            "CampaignVictory ({camp_len}) should be substantially longer than Win ({win_len}) and LevelComplete ({lvl_len})"
        );

        // CacheOpen must be sonically distinct from LevelUp.
        let cache_s = sfx_samples(Sfx::CacheOpen);
        let lvlup_s = sfx_samples(Sfx::LevelUp);
        let common = cache_s.len().min(lvlup_s.len());
        let diff = (cache_s[..common]
            .iter()
            .zip(&lvlup_s[..common])
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            / common as f32)
            .sqrt();
        assert!(
            diff > 0.12,
            "CacheOpen should be clearly distinct from LevelUp (diff_rms={diff:.4})"
        );
    }

    #[test]
    fn loopable_one_shots_blend_without_seam_clicks() {
        for sfx in [Sfx::CatGrowl, Sfx::MowerIdle] {
            for v in 0..sfx.variant_count() {
                let s = sfx_variant_samples(sfx, v);
                let pk = peak(&s);
                // First 15ms and last 25ms stay well below peak so repeated triggers crossfade smoothly.
                let head = peak(&s[..(SR as usize * 15 / 1000).min(s.len())]);
                let tail = peak(&s[s.len().saturating_sub(SR as usize * 25 / 1000)..]);
                assert!(
                    head < pk * 0.45,
                    "{sfx:?} v{v} loop head too abrupt: head={head:.3} pk={pk:.3}"
                );
                assert!(
                    tail < pk * 0.45,
                    "{sfx:?} v{v} loop tail too abrupt: tail={tail:.3} pk={pk:.3}"
                );
                // Concatenating two copies back-to-back produces zero step jump at the seam.
                let seam_delta = (s[0] - s[s.len() - 1]).abs();
                assert!(
                    seam_delta < 1e-4,
                    "{sfx:?} v{v} seam jump={seam_delta:.5}"
                );
            }
        }
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
        // New content spectral role separation: bright scan/steam >> low growl/stomp.
        let z_scan = zero_crossing_rate_hz(&sfx_samples(Sfx::MowerScan));
        let z_steam = zero_crossing_rate_hz(&sfx_samples(Sfx::MowerOverheat));
        let z_growl = zero_crossing_rate_hz(&sfx_samples(Sfx::CatGrowl));
        let z_stomp = zero_crossing_rate_hz(&sfx_samples(Sfx::BroomStomp));
        assert!(
            z_scan > z_growl * 2.0 && z_scan > z_stomp * 2.0,
            "MowerScan ({z_scan:.0}Hz) should be much brighter than CatGrowl ({z_growl:.0}Hz) and BroomStomp ({z_stomp:.0}Hz)"
        );
        assert!(
            z_steam > z_growl * 2.0 && z_steam > z_stomp * 2.0,
            "MowerOverheat ({z_steam:.0}Hz) should be much brighter than CatGrowl ({z_growl:.0}Hz) and BroomStomp ({z_stomp:.0}Hz)"
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
    fn background_cache_matches_the_synchronous_build() {
        let cache = AudioEngine::spawn_cache();
        let deadline = Instant::now() + Duration::from_secs(30);
        while cache.get().is_none() {
            assert!(
                Instant::now() < deadline,
                "the background cache never finished"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let background = cache.get().expect("the background thread published");
        let synchronous = AudioEngine::build_cache();
        assert_eq!(background.len(), synchronous.len());
        for (b, s) in background.iter().zip(synchronous.iter()) {
            assert_eq!(b.len(), s.len(), "variant count differs");
            for (bv, sv) in b.iter().zip(s.iter()) {
                assert_eq!(bv.as_ref(), sv.as_ref(), "a background buffer differs");
            }
        }
    }

    #[test]
    fn an_unready_cache_plays_nothing_and_never_panics() {
        let cache: OnceLock<SfxBank> = OnceLock::new();
        let picker = VariantPicker::new();
        for &sfx in Sfx::ALL.iter() {
            assert!(
                cached_sample(&cache, &picker, sfx, 1).is_none(),
                "an unready cache must not return a buffer"
            );
        }
        // Once published, the same lookup serves a real buffer for every effect.
        let _ = cache.set(AudioEngine::build_cache());
        let ready = cache.get().expect("the bank was just set");
        for &sfx in Sfx::ALL.iter() {
            let got = cached_sample(&cache, &picker, sfx, 1).expect("the bank is ready");
            assert!(
                ready[sfx.index()]
                    .iter()
                    .any(|v| v.as_ref() == got.as_ref()),
                "the lookup must return one of the bank's own buffers"
            );
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

    // ---- Volume curve, mixer and sink application ----------------------

    /// A gain sink that records what it was handed, so the apply path can be
    /// tested with no output device (the engine needs one to exist at all).
    struct RecordingSink(std::cell::Cell<f32>);

    impl RecordingSink {
        fn new() -> Self {
            RecordingSink(std::cell::Cell::new(-1.0))
        }
        fn gain(&self) -> f32 {
            self.0.get()
        }
    }

    impl GainSink for RecordingSink {
        fn set_gain(&self, gain: f32) {
            self.0.set(gain);
        }
    }

    #[test]
    fn volume_curve_is_squared_zero_at_zero_and_reaches_the_old_levels_at_100() {
        // 0 is truly silent on both channels (not a -40 dB floor).
        assert_eq!(volume_curve(0), 0.0);
        assert_eq!(music_gain(0), 0.0);
        assert_eq!(effects_gain(0), 0.0);
        // 100 reproduces exactly the pre-slider constants.
        assert_eq!(music_gain(100), MUSIC_MAX_GAIN);
        assert_eq!(effects_gain(100), SFX_MAX_GAIN);
        assert_eq!(music_gain(100), 0.50);
        assert_eq!(effects_gain(100), 0.85);
        // Squared curve: half the setting is a quarter of the amplitude.
        assert!((volume_curve(50) - 0.25).abs() < 1e-6);
        assert!((music_gain(50) - 0.125).abs() < 1e-6);
        // Monotonic non-decreasing across the whole slider, on both channels.
        let mut prev_music = -1.0;
        let mut prev_fx = -1.0;
        for pct in 0..=100u8 {
            let m = music_gain(pct);
            let f = effects_gain(pct);
            assert!(m >= prev_music, "music gain fell at {pct}%");
            assert!(f >= prev_fx, "effects gain fell at {pct}%");
            assert!(m <= MUSIC_MAX_GAIN && f <= SFX_MAX_GAIN);
            prev_music = m;
            prev_fx = f;
        }
    }

    #[test]
    fn settings_snap_to_five_percent_steps_and_clamp() {
        assert_eq!(snap_percent(0), 0);
        assert_eq!(snap_percent(2), 0);
        assert_eq!(snap_percent(3), 5);
        assert_eq!(snap_percent(67), 65);
        assert_eq!(snap_percent(98), 100);
        assert_eq!(snap_percent(100), 100);
        assert_eq!(snap_percent(200), 100);

        let mut m = VolumeMixer::default();
        assert_eq!((m.music(), m.effects(), m.muted()), (100, 100, false));
        assert!(m.set_music(63));
        assert_eq!(m.music(), 65);
        assert!(!m.set_music(64), "same snapped level is not a change");
        assert!(m.set_effects(7));
        assert_eq!(m.effects(), 5);
        assert!(m.set_effects(255));
        assert_eq!(m.effects(), 100);
    }

    #[test]
    fn both_channels_reach_their_sinks_and_every_live_voice() {
        let mut m = VolumeMixer::new(60, 80);
        let music = RecordingSink::new();
        let voices: Vec<RecordingSink> = (0..3).map(|_| RecordingSink::new()).collect();
        let sinks = || voices.iter().map(|v| v as &dyn GainSink);

        m.apply(&music, sinks());
        assert_eq!(music.gain(), music_gain(60));
        for v in &voices {
            assert_eq!(v.gain(), effects_gain(80));
        }

        // A change while voices are sounding reaches the live sinks...
        assert!(m.set_effects(30));
        m.apply(&music, sinks());
        for v in &voices {
            assert_eq!(v.gain(), effects_gain(30));
        }
        // ...and a change to the music leaves the effect voices alone.
        assert!(m.set_music(100));
        m.apply(&music, sinks());
        assert_eq!(music.gain(), MUSIC_MAX_GAIN);
        assert_eq!(voices[0].gain(), effects_gain(30));

        // A voice created *after* the change starts at the current level.
        let new_voice = RecordingSink::new();
        new_voice.set_gain(m.effects_gain());
        assert_eq!(new_voice.gain(), effects_gain(30));

        // Silence at 0 on both channels.
        m.set_effects(0);
        m.set_music(0);
        m.apply(&music, sinks());
        assert_eq!(music.gain(), 0.0);
        assert_eq!(voices[0].gain(), 0.0);
    }

    #[test]
    fn mute_layers_on_top_and_remembers_both_levels() {
        let mut m = VolumeMixer::new(55, 35);
        let music = RecordingSink::new();
        let voice = RecordingSink::new();
        let sinks = || [&voice as &dyn GainSink].into_iter();

        m.apply(&music, sinks());
        assert_eq!(music.gain(), music_gain(55));

        assert!(m.toggle_mute());
        assert!(m.muted());
        m.apply(&music, sinks());
        assert_eq!(music.gain(), 0.0, "mute silences the music sink");
        assert_eq!(voice.gain(), 0.0, "mute silences live effect voices");
        assert_eq!(m.music_gain(), 0.0);
        assert_eq!(m.effects_gain(), 0.0);
        // The levels underneath are remembered, not zeroed.
        assert_eq!((m.music(), m.effects()), (55, 35));

        // Changing a level while muted stays silent but is kept.
        assert!(m.set_music(90));
        m.apply(&music, sinks());
        assert_eq!(music.gain(), 0.0);
        assert_eq!(m.music(), 90);

        // Unmute restores the levels underneath.
        assert!(!m.toggle_mute());
        m.apply(&music, sinks());
        assert_eq!(music.gain(), music_gain(90));
        assert_eq!(voice.gain(), effects_gain(35));
    }

    #[test]
    fn mute_and_levels_are_independent_across_resets() {
        let mut m = VolumeMixer::new(0, 100);
        assert_eq!(m.music_gain(), 0.0);
        m.set_muted(true);
        m.set_muted(false);
        assert_eq!(m.music_gain(), 0.0, "a 0 level stays silent after unmuting");
        assert_eq!(m.effects_gain(), SFX_MAX_GAIN);

        // Re-creating a mixer for a new engine keeps the defaults.
        let fresh = VolumeMixer::default();
        assert_eq!(fresh.music_gain(), MUSIC_MAX_GAIN);
        assert_eq!(fresh.effects_gain(), SFX_MAX_GAIN);
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
        // Heavy new impacts and growls carry strong low-frequency weight relative to MowerScan.
        let stomp_low = band_power(&sfx_samples(Sfx::BroomStomp), &low_f);
        let slam_low = band_power(&sfx_samples(Sfx::GateSlam), &low_f);
        let growl_low = band_power(&sfx_samples(Sfx::CatGrowl), &low_f);
        let scan_low = band_power(&sfx_samples(Sfx::MowerScan), &low_f);
        assert!(
            stomp_low > scan_low * 3.0 && slam_low > scan_low * 3.0 && growl_low > scan_low * 2.0,
            "Expected weighty BroomStomp ({stomp_low:.1}), GateSlam ({slam_low:.1}), CatGrowl ({growl_low:.1}) >> MowerScan ({scan_low:.1})"
        );
    }

    #[test]
    fn voice_priorities_protect_critical_alerts() {
        for alert in [
            Sfx::Win,
            Sfx::Lose,
            Sfx::Boss,
            Sfx::BossDeath,
            Sfx::LevelUp,
            Sfx::CacheOpen,
            Sfx::CatRoar,
            Sfx::CatDeath,
            Sfx::BroomDefeat,
            Sfx::MowerDeath,
            Sfx::LevelComplete,
            Sfx::CampaignVictory,
        ] {
            for chatter in [Sfx::Shoot, Sfx::Hit, Sfx::Gem, Sfx::UiMove] {
                assert!(alert.priority() > chatter.priority());
            }
        }
        assert!(Sfx::Hurt.priority() > Sfx::Hit.priority());
        assert!(Sfx::Nuke.priority() > Sfx::Shoot.priority());
        assert!(Sfx::TrapSnap.priority() > Sfx::TrapTrigger.priority());
        assert!(Sfx::GateSlam.priority() > Sfx::GateOpen.priority());
        assert!(Sfx::BroomStomp.priority() > Sfx::BroomHop.priority());
        assert!(Sfx::MowerWallClang.priority() > Sfx::MowerIdle.priority());
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
        assert!(
            total_variants >= 140,
            "expected rich variant bank across all 59 SFX, got {total_variants}"
        );
        assert!(
            bytes < 16 * 1024 * 1024,
            "audio cache unexpectedly large: {bytes} bytes"
        );
    }

    #[test]
    fn music_frequencies_are_only_looked_up_at_step_boundaries() {
        let mut synth = MusicSynth::new(Theme::Sewer, Arc::new(AtomicU32::new(2)));
        let sps = synth.samples_per_step;
        let _ = synth.next();
        assert_eq!(synth.cached_step, 0);
        let snap = (
            synth.bass_freq,
            synth.chord_freqs,
            synth.lead_freq,
            synth.counter_freq,
        );
        for _ in 0..sps.min(4096) {
            let _ = synth.next();
        }
        if synth.counter < sps {
            assert_eq!(synth.cached_step, 0);
            assert_eq!(
                (
                    synth.bass_freq,
                    synth.chord_freqs,
                    synth.lead_freq,
                    synth.counter_freq
                ),
                snap
            );
        }
        while synth.counter < sps {
            let _ = synth.next();
        }
        let _ = synth.next(); // first sample of step 1
        assert_eq!(synth.counter, sps + 1);
        assert_eq!(synth.cached_step, 1);
        // Step 1 holds bass & lead in Sewer INTRO, so active chord tones remain cached from bar 0
        let ch0 = synth.chord_at_step(1);
        for i in 0..3 {
            assert!(
                (synth.chord_freqs[i] - semi_freq(synth.cfg.root, ch0.tones[i])).abs() < 1e-3
            );
        }
    }

    /// YIN cumulative-mean-normalized difference fundamental estimator (Hz) on a
    /// monophonic voice waveform. Robust against strong pulse-wave harmonics.
    fn estimate_f0_hz(samples: &[f32], min_hz: f32, max_hz: f32) -> Option<f32> {
        let max_tau = ((SR_F / min_hz) as usize).min(samples.len() / 2);
        let min_tau = ((SR_F / max_hz) as usize).max(16);
        let win = samples.len().saturating_sub(max_tau).min(1536);
        if win < 256 || max_tau <= min_tau + 2 {
            return None;
        }
        let mut diff = vec![0.0f32; max_tau + 1];
        for tau in 1..=max_tau {
            let mut sum = 0.0f32;
            for i in 0..win {
                let d = samples[i] - samples[i + tau];
                sum += d * d;
            }
            diff[tau] = sum;
        }
        let mut cmnd = vec![1.0f32; max_tau + 1];
        let mut running = 0.0f32;
        for tau in 1..=max_tau {
            running += diff[tau];
            cmnd[tau] = if running > 1e-12 {
                diff[tau] * tau as f32 / running
            } else {
                1.0
            };
        }
        // First local minimum below threshold 0.15; fallback to global min in range.
        let mut best_tau = None;
        for tau in min_tau..max_tau {
            if cmnd[tau] < 0.15 && cmnd[tau] <= cmnd[tau - 1] && cmnd[tau] <= cmnd[tau + 1] {
                best_tau = Some(tau);
                break;
            }
        }
        let tau = best_tau.unwrap_or_else(|| {
            (min_tau..=max_tau)
                .min_by(|&a, &b| cmnd[a].total_cmp(&cmnd[b]))
                .unwrap_or(min_tau)
        });
        if cmnd[tau] > 0.35 {
            return None;
        }
        // Parabolic refinement around tau
        let tau_f = if tau > min_tau && tau < max_tau {
            let s0 = cmnd[tau - 1];
            let s1 = cmnd[tau];
            let s2 = cmnd[tau + 1];
            let denom = 2.0 * (s0 - 2.0 * s1 + s2);
            if denom.abs() > 1e-9 {
                tau as f32 + (s0 - s2) / denom
            } else {
                tau as f32
            }
        } else {
            tau as f32
        };
        Some(SR_F / tau_f)
    }

    #[test]
    fn music_polyphony_simultaneous_distinct_voices_at_every_intensity() {
        for theme in Theme::all() {
            // Check Section A1 bar 4 step 0 (loop_step = 64) where bass + 3 chords + lead + counter all play!
            let target_step = SECTION_STEPS; // step 64 = start of A1
            let expected_min = [(0u32, 4usize), (1, 5), (2, 6), (3, 6)];
            let mut int2_buf = Vec::new();
            let mut int3_buf = Vec::new();

            for (lvl, min_voices) in expected_min {
                let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                let sps = synth.samples_per_step as usize;
                // Advance cleanly to start of A1 (step 64)
                for _ in 0..(target_step * sps) {
                    let _ = synth.next();
                }
                // Capture 2 steps (an 8th note, ~10500-12700 samples) of per-voice signals
                let cap_len = (sps * 2).min(12000);
                let mut v_bass = Vec::with_capacity(cap_len);
                let mut v_c0 = Vec::with_capacity(cap_len);
                let mut v_c1 = Vec::with_capacity(cap_len);
                let mut v_c2 = Vec::with_capacity(cap_len);
                let mut v_lead = Vec::with_capacity(cap_len);
                let mut v_ctr = Vec::with_capacity(cap_len);
                let mut v_mix = Vec::with_capacity(cap_len);
                for _ in 0..cap_len {
                    let f = synth.next_voices();
                    v_bass.push(f.bass);
                    v_c0.push(f.chord[0]);
                    v_c1.push(f.chord[1]);
                    v_c2.push(f.chord[2]);
                    v_lead.push(f.lead);
                    v_ctr.push(f.counter);
                    v_mix.push(f.mix);
                }
                if lvl == 2 {
                    int2_buf = v_mix;
                } else if lvl == 3 {
                    int3_buf = v_mix;
                }

                // Skip the first 256 samples (attack ramp) and analyze steady tone window
                let slice_end = (sps - 256).min(cap_len).max(2400);
                let start = 256usize;
                let voices: [(&str, &[f32], f32, f32); 6] = [
                    ("bass", &v_bass[start..slice_end], 50.0, 260.0),
                    ("chord0", &v_c0[start..slice_end], 100.0, 600.0),
                    ("chord1", &v_c1[start..slice_end], 100.0, 700.0),
                    ("chord2", &v_c2[start..slice_end], 100.0, 900.0),
                    ("lead", &v_lead[start..slice_end], 160.0, 1400.0),
                    ("counter", &v_ctr[start..slice_end], 140.0, 1200.0),
                ];

                let mut active_f0s: Vec<(&str, f32)> = Vec::new();
                for (name, sig, lo, hi) in voices {
                    if rms(sig) > 0.010 {
                        let f0 = estimate_f0_hz(sig, lo, hi).unwrap_or_else(|| {
                            panic!("{theme:?} int={lvl} voice {name} had energy but no clear f0")
                        });
                        active_f0s.push((name, f0));
                    }
                }

                // Count distinct pitches (separated by > 2.5% in frequency, ~43 cents)
                let mut distinct: Vec<f32> = Vec::new();
                for &(_, f0) in &active_f0s {
                    if !distinct.iter().any(|&g| (f0 - g).abs() / g.max(1.0) < 0.025) {
                        distinct.push(f0);
                    }
                }
                assert!(
                    distinct.len() >= min_voices,
                    "{theme:?} int={lvl}: expected >= {min_voices} distinct simultaneous pitched voices, got {} ({active_f0s:?})",
                    distinct.len()
                );
            }

            // Boss overdrive (int=3) must meaningfully differ from Horde (int=2).
            let diff = (int2_buf
                .iter()
                .zip(&int3_buf)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                / int2_buf.len() as f32)
                .sqrt();
            assert!(
                diff > 0.015,
                "{theme:?} Boss (int=3) should differ from Horde (int=2): diff_rms={diff:.4}"
            );
        }
    }

    #[test]
    fn music_chord_tones_belong_to_stated_chords() {
        for theme in Theme::all() {
            let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(0)));
            let sps = synth.samples_per_step as usize;
            let half_bar_samples = sps * 8;
            // Walk every single half-bar chord slot across all 32 bars (64 chord slots per theme)
            for slot in 0..(LOOP_STEPS / 8) {
                let expected = synth.chord_at_step(slot * 8);
                // Verify stated chord has 3 distinct tones
                assert!(
                    expected.tones[0] != expected.tones[1]
                        && expected.tones[1] != expected.tones[2]
                        && expected.tones[0] != expected.tones[2],
                    "{theme:?} slot {slot} ({}) has duplicate chord tones: {:?}",
                    expected.name,
                    expected.tones
                );
                let mut c_sigs = [
                    Vec::with_capacity(2800),
                    Vec::with_capacity(2800),
                    Vec::with_capacity(2800),
                ];
                for i in 0..half_bar_samples {
                    let f = synth.next_voices();
                    // Sample a steady 2800-sample window starting 1200 samples into the half-bar
                    if (1200..4000).contains(&i) {
                        c_sigs[0].push(f.chord[0]);
                        c_sigs[1].push(f.chord[1]);
                        c_sigs[2].push(f.chord[2]);
                    }
                }
                for voice_i in 0..3 {
                    let expected_hz = semi_freq(synth.cfg.root, expected.tones[voice_i]);
                    let est_hz = estimate_f0_hz(&c_sigs[voice_i], 85.0, 900.0).unwrap_or_else(|| {
                        panic!(
                            "{theme:?} slot {slot} ({}) chord voice {voice_i} had no clear f0",
                            expected.name
                        )
                    });
                    let rel_err = (est_hz - expected_hz).abs() / expected_hz;
                    assert!(
                        rel_err < 0.025,
                        "{theme:?} slot {slot} ({}) chord[{voice_i}]: expected {expected_hz:.1}Hz, estimated {est_hz:.1}Hz (err={:.2}%)",
                        expected.name,
                        rel_err * 100.0
                    );
                }
            }
        }
    }

    #[test]
    fn music_loop_point_is_continuous_without_click_or_hiccup() {
        for theme in Theme::all() {
            for lvl in 0..=3u32 {
                let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                let sps = synth.samples_per_step as usize;
                let loop_len = LOOP_STEPS * sps;
                // Advance to 128 samples before the loop boundary
                let pre = loop_len - 128;
                for _ in 0..pre {
                    let _ = synth.next();
                }
                let mut seam_win = Vec::with_capacity(256);
                for _ in 0..256 {
                    seam_win.push(synth.next().unwrap());
                }
                // At index 127 -> 128 we crossed the exact loop point (step 511 -> step 512 == step 0).
                let seam_jump = (seam_win[128] - seam_win[127]).abs();
                let max_win_delta = seam_win
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .fold(0.0f32, f32::max);
                assert!(
                    seam_jump < 0.08,
                    "{theme:?} int={lvl} loop seam discontinuity: jump={seam_jump:.4}"
                );
                assert!(
                    max_win_delta < 0.14,
                    "{theme:?} int={lvl} click near loop seam: max_delta={max_win_delta:.4}"
                );
                // Verify exact step counter alignment (zero tempo hiccup)
                assert_eq!(synth.step_idx, LOOP_STEPS as u64);
                assert_eq!(synth.cached_step, LOOP_STEPS as u64);
            }
        }
    }

    #[test]
    fn music_full_loop_mix_targets_headroom_and_zero_dc() {
        for theme in Theme::all() {
            let mut rms_by_int = [0.0f32; 4];
            for lvl in 0..=3u32 {
                let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                let loop_len = LOOP_STEPS * synth.samples_per_step as usize;
                let mut sum_sq = 0.0f64;
                let mut sum = 0.0f64;
                let mut pk = 0.0f32;
                for _ in 0..loop_len {
                    let x = synth.next().unwrap();
                    assert!(
                        x.is_finite() && x.abs() <= 1.0,
                        "{theme:?} int={lvl} non-finite or clipped sample: {x}"
                    );
                    pk = pk.max(x.abs());
                    let xd = x as f64;
                    sum += xd;
                    sum_sq += xd * xd;
                }
                let r = (sum_sq / loop_len as f64).sqrt() as f32;
                let dc = (sum / loop_len as f64).abs() as f32;
                assert!(
                    (0.14..=0.55).contains(&pk),
                    "{theme:?} int={lvl} peak out of background-music target [0.14, 0.55]: pk={pk:.4}"
                );
                assert!(
                    (0.045..=0.22).contains(&r),
                    "{theme:?} int={lvl} RMS out of mix target [0.045, 0.22]: rms={r:.4}"
                );
                assert!(
                    dc < 0.005,
                    "{theme:?} int={lvl} excessive DC offset over full loop: dc={dc:.5}"
                );
                rms_by_int[lvl as usize] = r;
            }
            assert!(
                rms_by_int[1] > rms_by_int[0] * 1.05,
                "{theme:?} full-loop RMS should rise int 0->1: {rms_by_int:?}"
            );
            assert!(
                rms_by_int[2] > rms_by_int[1] * 1.05,
                "{theme:?} full-loop RMS should rise int 1->2: {rms_by_int:?}"
            );
            assert!(
                rms_by_int[3] > rms_by_int[2] * 1.03,
                "{theme:?} full-loop RMS should rise int 2->3 (Boss): {rms_by_int:?}"
            );
        }
    }

    #[test]
    fn music_themes_are_musically_distinct_and_deterministic() {
        let mut excerpts: Vec<(Theme, Vec<f32>, f32, f32)> = Vec::new();
        for theme in Theme::all() {
            let mut a = MusicSynth::new(theme, Arc::new(AtomicU32::new(2)));
            let mut b = MusicSynth::new(theme, Arc::new(AtomicU32::new(2)));
            let n = SR as usize * 4;
            let sa: Vec<f32> = a.by_ref().take(n).collect();
            let sb: Vec<f32> = b.by_ref().take(n).collect();
            assert_eq!(sa, sb, "{theme:?} MusicSynth must be bit-for-bit deterministic");
            let bpm = a.cfg.bpm;
            let root = a.cfg.root;
            excerpts.push((theme, sa, bpm, root));
        }
        for i in 0..excerpts.len() {
            for j in (i + 1)..excerpts.len() {
                let (ti, ref si, bi, ri) = excerpts[i];
                let (tj, ref sj, bj, rj) = excerpts[j];
                assert!((bi - bj).abs() > 4.0, "{ti:?} and {tj:?} tempos too close: {bi} vs {bj}");
                assert!((ri - rj).abs() > 4.0, "{ti:?} and {tj:?} keys too close: {ri} vs {rj}");
                let diff_rms = (si
                    .iter()
                    .zip(sj)
                    .map(|(x, y)| (x - y) * (x - y))
                    .sum::<f32>()
                    / si.len() as f32)
                    .sqrt();
                assert!(
                    diff_rms > 0.08,
                    "{ti:?} and {tj:?} audio too similar: diff_rms={diff_rms:.4}"
                );
            }
        }
    }

    #[test]
    fn music_intensity_transitions_across_all_levels_click_free() {
        for theme in Theme::all() {
            let intensity = Arc::new(AtomicU32::new(0));
            let mut synth = MusicSynth::new(theme, Arc::clone(&intensity));
            let mut prev = 0.0f32;
            let mut max_delta = 0.0f32;
            let seg = SR as usize;
            let seq = [0u32, 1, 2, 3, 2, 0];
            for &lvl in &seq {
                intensity.store(lvl, Ordering::Relaxed);
                for _ in 0..seg {
                    let s = synth.next().unwrap();
                    max_delta = max_delta.max((s - prev).abs());
                    prev = s;
                }
            }
            assert!(
                max_delta < 0.18,
                "{theme:?} had harsh discontinuity across 0->1->2->3->2->0: max_delta={max_delta:.4}"
            );
        }
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

        // Dump representative variant sets (classic + new wave1 effects)
        let mut var_json = Vec::new();
        for &(sfx, max_v, col) in &[
            (Sfx::Gem, 4usize, "[130,235,255]"),
            (Sfx::TrapSnap, 3, "[255,175,105]"),
            (Sfx::CatGrowl, 3, "[235,175,95]"),
            (Sfx::BroomStomp, 4, "[205,155,255]"),
            (Sfx::MowerWallClang, 3, "[120,220,255]"),
            (Sfx::MowerCharge, 3, "[255,140,160]"),
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
            let mut out = Vec::with_capacity(seg * 4);
            for lvl in 0..=3u32 {
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

    #[test]
    fn dump_music_analysis() {
        let Ok(dir_str) = std::env::var("RATRUN_MUSIC_DUMP") else {
            return;
        };
        let tag = std::env::var("RATRUN_MUSIC_TAG").unwrap_or_else(|_| "V1".into());
        let dir = std::path::PathBuf::from(dir_str);
        std::fs::create_dir_all(&dir).unwrap();

        let mut themes_json = Vec::new();
        for theme in Theme::all() {
            let slug = theme.name().to_lowercase();
            let cfg = track_for(theme);
            let sps = ((SR_F * 60.0 / (cfg.bpm * 4.0)) as u64).max(1) as usize;
            let loop_len = LOOP_STEPS * sps;

            let mut int_entries = Vec::new();
            for lvl in 0..=3u32 {
                let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                let mut buf = Vec::with_capacity(loop_len + 1);
                for _ in 0..=loop_len {
                    buf.push(synth.next().unwrap_or(0.0));
                }
                let seam_delta = (buf[loop_len] - buf[loop_len - 1]).abs();
                let fname = format!("{slug}_int{lvl}.wav");
                write_wav(&dir.join(&fname), &buf[..loop_len]);
                if lvl == 3 {
                    // Also save a clean listening copy at full Boss intensity as requested.
                    write_wav(&dir.join(format!("{slug}_full.wav")), &buf[..loop_len]);
                }
                int_entries.push(format!(
                    "{{\"intensity\":{lvl},\"file\":\"{fname}\",\"seam_delta\":{seam_delta:.6}}}"
                ));
            }

            let mut sec_entries = Vec::new();
            for (si, sec) in cfg.sections.iter().enumerate() {
                sec_entries.push(format!(
                    "{{\"name\":\"{}\",\"start_bar\":{},\"bars\":{}}}",
                    sec.name,
                    si * SECTION_BARS,
                    SECTION_BARS
                ));
            }

            let mut chord_entries = Vec::new();
            for sec in cfg.sections.iter() {
                for ch in sec.chords.iter() {
                    chord_entries.push(format!("{{\"name\":\"{}\"}}", ch.name));
                }
            }

            // Step-by-step active pitch trace for piano-roll plotting
            let mut step_entries = Vec::new();
            let mut cur_bass: Option<f32> = None;
            let mut cur_lead: Option<f32> = None;
            let mut cur_ctr: Option<f32> = None;
            for step in 0..LOOP_STEPS {
                let sec = &cfg.sections[step / SECTION_STEPS];
                let local = step % SECTION_STEPS;
                let ch = sec.chords[local / 8];
                let c0 = semi_freq(cfg.root, ch.tones[0]);
                let c1 = semi_freq(cfg.root, ch.tones[1]);
                let c2 = semi_freq(cfg.root, ch.tones[2]);

                let bn = sec.bass[local];
                if bn == R {
                    cur_bass = None;
                } else if bn != H {
                    cur_bass = Some(semi_freq(cfg.root * 0.5, bn));
                }
                let ln = sec.lead[local];
                if ln == R {
                    cur_lead = None;
                } else if ln != H {
                    cur_lead = Some(semi_freq(cfg.root * 2.0, ln));
                }
                let cn = sec.counter[local];
                if cn == R {
                    cur_ctr = None;
                } else if cn != H {
                    cur_ctr = Some(semi_freq(cfg.root * 2.0, cn));
                }
                let fmt_opt = |o: Option<f32>| match o {
                    Some(v) => format!("{v:.2}"),
                    None => "null".to_string(),
                };
                step_entries.push(format!(
                    "{{\"bass\":{},\"chord\":[{c0:.2},{c1:.2},{c2:.2}],\"lead\":{},\"counter\":{},\"drums\":{}}}",
                    fmt_opt(cur_bass),
                    fmt_opt(cur_lead),
                    fmt_opt(cur_ctr),
                    sec.drums[local]
                ));
            }

            themes_json.push(format!(
                "{{\"name\":\"{}\",\"key\":\"{}\",\"bpm\":{:.1},\"samples_per_step\":{},\"loop_steps\":{},\"sections\":[{}],\"chords\":[{}],\"intensities\":[{}],\"steps\":[{}]}}",
                theme.name(),
                cfg.key_name,
                cfg.bpm,
                sps,
                LOOP_STEPS,
                sec_entries.join(","),
                chord_entries.join(","),
                int_entries.join(","),
                step_entries.join(","),
            ));
        }

        let meta = format!(
            "{{\"tag\":\"{tag}\",\"themes\":[{}]}}",
            themes_json.join(",")
        );
        std::fs::write(dir.join("music_meta.json"), meta).unwrap();
    }

    #[test]
    fn bench_music_per_sample() {
        if std::env::var("RATRUN_MUSIC_BENCH").is_err() {
            return;
        }
        let samples_per_run = SR as usize * 30; // 30s = 1,323,000 samples
        for theme in Theme::all() {
            for lvl in [0u32, 1, 2, 3] {
                let mut best_ns = f64::INFINITY;
                let mut acc = 0.0f64;
                for _trial in 0..5 {
                    let mut synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(lvl)));
                    for _ in 0..44_100 {
                        acc += synth.next().unwrap_or(0.0) as f64;
                    }
                    let t0 = Instant::now();
                    for _ in 0..samples_per_run {
                        acc += std::hint::black_box(synth.next().unwrap_or(0.0)) as f64;
                    }
                    let ns = t0.elapsed().as_nanos() as f64 / samples_per_run as f64;
                    if ns < best_ns {
                        best_ns = ns;
                    }
                }
                let cpu_pct = (best_ns * SR as f64 / 1e9) * 100.0;
                println!(
                    "BENCH {:?} int={}: {:.2} ns/sample ({:.3}% of 1 core @44.1kHz) [acc={:.1}]",
                    theme, lvl, best_ns, cpu_pct, acc
                );
            }
        }
    }
}

