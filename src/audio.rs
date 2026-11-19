//! Procedural audio: a looping chiptune track + synthesized 8-bit SFX.
//!
//! Everything is generated from scratch — no audio assets. The whole module
//! degrades to silent if no output device is available (headless, no server),
//! so the game never fails because of audio.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::Source;
use std::num::NonZero;

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};

use crate::game::level::Theme;

pub const SR: u32 = 44_100;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
}

impl Sfx {
    /// Every effect, in cache order. `index` must agree with this ordering.
    pub const ALL: [Sfx; 14] = [
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
    ];

    /// Position in the pre-synthesized sample cache.
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Square(f32), // duty cycle
    Triangle,
    Noise,
}

/// Append a single gliding tone to `out`.
fn tone(out: &mut Vec<f32>, f0: f32, f1: f32, dur: f32, vol: f32, wave: Wave, seed: &mut u32) {
    let n = (dur * SR as f32) as usize;
    if n == 0 {
        return;
    }
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let f = f0 + (f1 - f0) * t;
        phase = (phase + f / SR as f32).fract();
        let s = match wave {
            Wave::Square(d) => {
                if phase < d {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Triangle => 2.0 * (2.0 * phase - 1.0).abs() - 1.0,
            Wave::Noise => {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((*seed >> 9) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0
            }
        };
        // Short attack to kill clicks, then linear decay.
        let attack = (i as f32 / 64.0).min(1.0);
        let env = (1.0 - t) * attack;
        out.push((s * vol * env).clamp(-1.0, 1.0));
    }
}

/// Build the sample buffer for an SFX (mono, SR).
pub fn sfx_samples(sfx: Sfx) -> Vec<f32> {
    let mut out = Vec::new();
    let mut seed = 0x1234_5678u32;
    match sfx {
        Sfx::Shoot => tone(&mut out, 880.0, 360.0, 0.06, 0.16, Wave::Square(0.5), &mut seed),
        Sfx::Hit => {
            tone(&mut out, 320.0, 160.0, 0.05, 0.12, Wave::Square(0.5), &mut seed);
            tone(&mut out, 0.0, 0.0, 0.02, 0.10, Wave::Noise, &mut seed);
        }
        Sfx::Kill => tone(&mut out, 520.0, 150.0, 0.12, 0.18, Wave::Square(0.5), &mut seed),
        Sfx::Explosion => {
            tone(&mut out, 0.0, 0.0, 0.35, 0.32, Wave::Noise, &mut seed);
            tone(&mut out, 180.0, 50.0, 0.30, 0.20, Wave::Square(0.5), &mut seed);
        }
        Sfx::Hurt => {
            tone(&mut out, 220.0, 70.0, 0.18, 0.26, Wave::Square(0.5), &mut seed);
            tone(&mut out, 0.0, 0.0, 0.08, 0.12, Wave::Noise, &mut seed);
        }
        Sfx::Gem => tone(&mut out, 1200.0, 1750.0, 0.05, 0.10, Wave::Triangle, &mut seed),
        Sfx::Pickup => {
            for f in [660.0, 880.0, 1320.0] {
                tone(&mut out, f, f * 1.02, 0.05, 0.12, Wave::Triangle, &mut seed);
            }
        }
        Sfx::LevelUp => {
            for f in [523.0, 659.0, 784.0, 1046.0] {
                tone(&mut out, f, f, 0.09, 0.18, Wave::Square(0.5), &mut seed);
            }
        }
        Sfx::Boss => {
            tone(&mut out, 130.0, 90.0, 0.6, 0.30, Wave::Square(0.5), &mut seed);
            tone(&mut out, 0.0, 0.0, 0.4, 0.18, Wave::Noise, &mut seed);
        }
        Sfx::Dash => tone(&mut out, 700.0, 1100.0, 0.10, 0.10, Wave::Noise, &mut seed),
        Sfx::UiMove => tone(&mut out, 600.0, 600.0, 0.03, 0.10, Wave::Square(0.5), &mut seed),
        Sfx::UiSelect => {
            tone(&mut out, 660.0, 660.0, 0.05, 0.14, Wave::Square(0.5), &mut seed);
            tone(&mut out, 990.0, 990.0, 0.07, 0.14, Wave::Square(0.5), &mut seed);
        }
        Sfx::Win => {
            for f in [523.0, 659.0, 784.0, 1046.0, 1318.0] {
                tone(&mut out, f, f, 0.12, 0.2, Wave::Square(0.5), &mut seed);
            }
        }
        Sfx::Lose => {
            for f in [440.0, 392.0, 330.0, 220.0] {
                tone(&mut out, f, f, 0.16, 0.22, Wave::Square(0.5), &mut seed);
            }
        }
    }
    out
}

// ---- Music ---------------------------------------------------------------

struct TrackCfg {
    root: f32,
    scale: &'static [i32],
    bpm: f32,
    bass: [i32; 16],
    lead: [i32; 16],
}

const REST: i32 = -127;

fn track_for(theme: Theme) -> TrackCfg {
    // Scale-degree patterns; REST is a rest. Lead plays an octave up.
    match theme {
        Theme::Sewer => TrackCfg {
            root: 130.81, // C3
            scale: &[0, 2, 3, 5, 7, 8, 10], // natural minor
            bpm: 96.0,
            bass: [0, REST, 0, REST, 5, REST, REST, REST, 3, REST, 3, REST, 4, REST, 4, REST],
            lead: [0, 2, 4, 2, REST, 4, 2, 0, 1, 3, 5, 3, 4, REST, 2, 0],
        },
        Theme::Kitchen => TrackCfg {
            root: 146.83, // D3
            scale: &[0, 2, 4, 5, 7, 9, 11], // major
            bpm: 124.0,
            bass: [0, REST, 4, REST, 5, REST, 4, REST, 3, REST, 4, REST, 0, REST, 4, 5],
            lead: [4, 2, 0, 2, 4, 7, 4, 2, 5, 4, 2, 4, 7, 9, 7, 4],
        },
        Theme::Lab => TrackCfg {
            root: 123.47, // B2
            scale: &[0, 1, 4, 5, 7, 8, 11], // harmonic/phrygian-ish, eerie
            bpm: 108.0,
            bass: [0, REST, REST, 0, 5, REST, REST, 5, 3, REST, REST, 3, 6, REST, 6, REST],
            lead: [7, REST, 6, 5, 7, REST, 4, 5, 3, REST, 4, 5, 7, 8, 7, REST],
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

/// An infinite chiptune source. `intensity` (0..=2) gates the lead and hat
/// layers so the music swells with the action.
pub struct MusicSynth {
    cfg: TrackCfg,
    intensity: Arc<AtomicU32>,
    counter: u64,
    samples_per_step: u64,
    bass_phase: f32,
    lead_phase: f32,
    seed: u32,
    /// Step whose note frequencies are cached below (`u64::MAX` = none yet).
    /// Frequencies only change on a step boundary, so `next` must not call the
    /// (powf-heavy) `degree_freq` for every one of the ~88k samples a second.
    cached_step: u64,
    bass_freq: Option<f32>,
    lead_freq: Option<f32>,
}

impl MusicSynth {
    pub fn new(theme: Theme, intensity: Arc<AtomicU32>) -> Self {
        let cfg = track_for(theme);
        let samples_per_step = (SR as f32 * 60.0 / (cfg.bpm * 4.0)) as u64;
        MusicSynth {
            cfg,
            intensity,
            counter: 0,
            samples_per_step: samples_per_step.max(1),
            bass_phase: 0.0,
            lead_phase: 0.0,
            seed: 0x9E37_79B9,
            cached_step: u64::MAX,
            bass_freq: None,
            lead_freq: None,
        }
    }

    /// Refresh the cached note frequencies when the pattern step advances.
    fn cache_step(&mut self, step_idx: u64) {
        let step = (step_idx % 16) as usize;
        self.cached_step = step_idx;
        self.bass_freq = degree_freq(self.cfg.root * 0.5, self.cfg.scale, self.cfg.bass[step]);
        self.lead_freq = degree_freq(self.cfg.root * 2.0, self.cfg.scale, self.cfg.lead[step]);
    }
}

impl Iterator for MusicSynth {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sps = self.samples_per_step;
        let step_idx = self.counter / sps;
        let step = (step_idx % 16) as usize;
        if self.cached_step != step_idx {
            self.cache_step(step_idx);
        }
        let pos = (self.counter % sps) as f32 / sps as f32;
        let intensity = self.intensity.load(Ordering::Relaxed);

        let mut s = 0.0f32;

        // Bass (triangle for a soft chiptune low end).
        if let Some(bf) = self.bass_freq {
            self.bass_phase = (self.bass_phase + bf / SR as f32).fract();
            let env = (1.0 - pos * 0.7).max(0.0);
            let tri = 2.0 * (2.0 * self.bass_phase - 1.0).abs() - 1.0;
            s += tri * 0.22 * env;
        }

        // Lead (square) — only at intensity >= 1.
        if intensity >= 1 {
            if let Some(lf) = self.lead_freq {
                self.lead_phase = (self.lead_phase + lf / SR as f32).fract();
                let env = (1.0 - pos).powf(1.8);
                let sq = if self.lead_phase < 0.25 { 1.0 } else { -1.0 };
                s += sq * 0.14 * env;
            }
        }

        // Noise hat on offbeats — only at intensity >= 2.
        if intensity >= 2 && step % 2 == 1 {
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = ((self.seed >> 9) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0;
            let env = (1.0 - pos).powf(4.0);
            s += n * 0.05 * env;
        }

        self.counter += 1;
        Some((s * 0.7).clamp(-1.0, 1.0))
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
/// Effects play on their own sinks so they can overlap and so mute can reach
/// the ones already sounding. Cap the live voices so a kill burst can never
/// spawn unbounded sinks/mixer tracks.
const MAX_SFX_VOICES: usize = 24;

/// Master volume for a channel: muting silences music *and* effects.
fn channel_vol(muted: bool, vol: f32) -> f32 {
    if muted {
        0.0
    } else {
        vol
    }
}

/// A cached mono sample buffer replayed as a rodio `Source`. Creating one costs
/// an `Arc` clone: no re-synthesis and no copy of the samples.
struct CachedSfx {
    data: Arc<[f32]>,
    pos: usize,
}

impl CachedSfx {
    fn new(data: Arc<[f32]>) -> Self {
        CachedSfx { data, pos: 0 }
    }
}

impl Iterator for CachedSfx {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let s = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(s)
    }
}

impl Source for CachedSfx {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.data.len().saturating_sub(self.pos))
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::<u16>::MIN
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(SR).expect("SR is non-zero")
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.data.len() as f64 / SR as f64))
    }
}

pub struct AudioEngine {
    stream: MixerDeviceSink,
    music: Player,
    /// One sink per in-flight effect, so effects overlap and muting can also
    /// silence effects that are already playing. Reaped when `play_sfx` runs.
    sfx_voices: Mutex<Vec<Player>>,
    /// Samples for every `Sfx`, synthesized once in `new` (indexed by
    /// `Sfx::index`). `play_sfx` only clones an `Arc` from here.
    cache: Vec<Arc<[f32]>>,
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
            sfx_voices: Mutex::new(Vec::new()),
            cache: Self::build_cache(),
            intensity: Arc::new(AtomicU32::new(1)),
            muted: false,
            music_vol: 0.55,
        })
    }

    /// Synthesize every effect exactly once, at startup.
    fn build_cache() -> Vec<Arc<[f32]>> {
        Sfx::ALL
            .iter()
            .map(|s| Arc::from(sfx_samples(*s).into_boxed_slice()))
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
            v.set_volume(channel_vol(self.muted, SFX_VOL));
        }
    }

    pub fn play_sfx(&self, sfx: Sfx) {
        let Some(samples) = self.cache.get(sfx.index()) else {
            return;
        };
        if samples.is_empty() {
            return;
        }
        let Ok(mut voices) = self.sfx_voices.lock() else {
            return;
        };
        // Finished voices hold no audio; drop them so the pool can be reused.
        voices.retain(|v| !v.empty());
        if voices.len() >= MAX_SFX_VOICES {
            return;
        }
        let sink = Player::connect_new(self.stream.mixer());
        sink.set_volume(channel_vol(self.muted, SFX_VOL));
        sink.append(CachedSfx::new(Arc::clone(samples)));
        sink.play();
        voices.push(sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sfx_is_finite_and_bounded() {
        for sfx in Sfx::ALL {
            let s = sfx_samples(sfx);
            assert!(!s.is_empty(), "sfx produced no samples");
            assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
        }
    }

    #[test]
    fn music_synth_produces_sound() {
        for theme in Theme::all() {
            let synth = MusicSynth::new(theme, Arc::new(AtomicU32::new(2)));
            let chunk: Vec<f32> = synth.take(SR as usize).collect();
            assert_eq!(chunk.len(), SR as usize);
            assert!(chunk.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
            // Should not be pure silence.
            assert!(chunk.iter().any(|x| x.abs() > 0.01));
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
    fn sfx_cache_is_synthesized_once_per_effect() {
        let cache = AudioEngine::build_cache();
        assert_eq!(cache.len(), Sfx::ALL.len());
        for (i, sfx) in Sfx::ALL.iter().enumerate() {
            assert_eq!(cache[i].as_ref(), sfx_samples(*sfx).as_slice());
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
    }

    #[test]
    fn mute_zeroes_both_channels() {
        assert_eq!(channel_vol(true, 0.55), 0.0);
        assert_eq!(channel_vol(true, SFX_VOL), 0.0);
        assert_eq!(channel_vol(false, 0.55), 0.55);
        assert_eq!(channel_vol(false, SFX_VOL), SFX_VOL);
    }

    /// The pre-caching `next` (frequency looked up per sample) used as a
    /// reference: caching must not change the generated samples at all.
    fn naive_music(theme: Theme, intensity: u32, n: usize) -> Vec<f32> {
        let cfg = track_for(theme);
        let sps = ((SR as f32 * 60.0 / (cfg.bpm * 4.0)) as u64).max(1);
        let mut bass_phase = 0.0f32;
        let mut lead_phase = 0.0f32;
        let mut seed = 0x9E37_79B9u32;
        let mut out = Vec::with_capacity(n);
        for counter in 0..n as u64 {
            let step = ((counter / sps) % 16) as usize;
            let pos = (counter % sps) as f32 / sps as f32;
            let mut s = 0.0f32;
            if let Some(bf) = degree_freq(cfg.root * 0.5, cfg.scale, cfg.bass[step]) {
                bass_phase = (bass_phase + bf / SR as f32).fract();
                let env = (1.0 - pos * 0.7).max(0.0);
                let tri = 2.0 * (2.0 * bass_phase - 1.0).abs() - 1.0;
                s += tri * 0.22 * env;
            }
            if intensity >= 1 {
                if let Some(lf) = degree_freq(cfg.root * 2.0, cfg.scale, cfg.lead[step]) {
                    lead_phase = (lead_phase + lf / SR as f32).fract();
                    let env = (1.0 - pos).powf(1.8);
                    let sq = if lead_phase < 0.25 { 1.0 } else { -1.0 };
                    s += sq * 0.14 * env;
                }
            }
            if intensity >= 2 && step % 2 == 1 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let nz = ((seed >> 9) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0;
                let env = (1.0 - pos).powf(4.0);
                s += nz * 0.05 * env;
            }
            out.push((s * 0.7).clamp(-1.0, 1.0));
        }
        out
    }

    #[test]
    fn cached_step_frequencies_produce_identical_samples() {
        for theme in Theme::all() {
            for intensity in 0..=2 {
                let steps = 16 * 2 + 3; // two pattern loops plus a partial one
                let probe = MusicSynth::new(theme, Arc::new(AtomicU32::new(intensity)));
                let n = (probe.samples_per_step as usize) * steps;
                let cached: Vec<f32> = MusicSynth::new(theme, Arc::new(AtomicU32::new(intensity)))
                    .take(n)
                    .collect();
                assert_eq!(cached, naive_music(theme, intensity, n));
            }
        }
    }

    #[test]
    fn music_frequencies_are_only_looked_up_at_step_boundaries() {
        let mut synth = MusicSynth::new(Theme::Sewer, Arc::new(AtomicU32::new(2)));
        let sps = synth.samples_per_step;
        let _ = synth.next();
        assert_eq!(synth.cached_step, 0);
        let (bass, lead) = (synth.bass_freq, synth.lead_freq);
        // Every sample inside the same step reuses the cached notes.
        for _ in 0..sps.min(4096) {
            let _ = synth.next();
        }
        if synth.counter < sps {
            assert_eq!(synth.cached_step, 0);
            assert_eq!((synth.bass_freq, synth.lead_freq), (bass, lead));
        }
        // Crossing the boundary refreshes them from the pattern.
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
            synth.lead_freq,
            degree_freq(synth.cfg.root * 2.0, synth.cfg.scale, synth.cfg.lead[step])
        );
    }
}
