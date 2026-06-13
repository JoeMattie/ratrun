//! Procedural audio: a looping chiptune track + synthesized 8-bit SFX.
//!
//! Everything is generated from scratch — no audio assets. The whole module
//! degrades to silent if no output device is available (headless, no server),
//! so the game never fails because of audio.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::source::Source;
use rodio::{OutputStream, OutputStreamHandle, Sink};

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
        }
    }
}

impl Iterator for MusicSynth {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sps = self.samples_per_step;
        let step = ((self.counter / sps) % 16) as usize;
        let pos = (self.counter % sps) as f32 / sps as f32;
        let intensity = self.intensity.load(Ordering::Relaxed);

        let mut s = 0.0f32;

        // Bass (triangle for a soft chiptune low end).
        if let Some(bf) = degree_freq(self.cfg.root * 0.5, self.cfg.scale, self.cfg.bass[step]) {
            self.bass_phase = (self.bass_phase + bf / SR as f32).fract();
            let env = (1.0 - pos * 0.7).max(0.0);
            let tri = 2.0 * (2.0 * self.bass_phase - 1.0).abs() - 1.0;
            s += tri * 0.22 * env;
        }

        // Lead (square) — only at intensity >= 1.
        if intensity >= 1 {
            if let Some(lf) = degree_freq(self.cfg.root * 2.0, self.cfg.scale, self.cfg.lead[step]) {
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
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        SR
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// ---- Engine --------------------------------------------------------------

pub struct AudioEngine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    music: Sink,
    intensity: Arc<AtomicU32>,
    muted: bool,
    music_vol: f32,
}

impl AudioEngine {
    /// Returns `None` if no output device is available (stays silent).
    pub fn new() -> Option<AudioEngine> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let music = Sink::try_new(&handle).ok()?;
        music.set_volume(0.0);
        Some(AudioEngine {
            _stream: stream,
            handle,
            music,
            intensity: Arc::new(AtomicU32::new(1)),
            muted: false,
            music_vol: 0.55,
        })
    }

    pub fn play_music(&mut self, theme: Theme, intensity: u32) {
        self.intensity.store(intensity, Ordering::Relaxed);
        self.music.clear();
        let synth = MusicSynth::new(theme, self.intensity.clone());
        self.music.append(synth);
        self.music.set_volume(if self.muted { 0.0 } else { self.music_vol });
        self.music.play();
    }

    pub fn set_intensity(&self, level: u32) {
        self.intensity.store(level, Ordering::Relaxed);
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        self.music
            .set_volume(if self.muted { 0.0 } else { self.music_vol });
        self.muted
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn play_sfx(&self, sfx: Sfx) {
        if self.muted {
            return;
        }
        let samples = sfx_samples(sfx);
        if samples.is_empty() {
            return;
        }
        let buf = SamplesBuffer::new(1, SR, samples);
        let _ = self.handle.play_raw(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SFX: [Sfx; 14] = [
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

    #[test]
    fn every_sfx_is_finite_and_bounded() {
        for sfx in ALL_SFX {
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
}
