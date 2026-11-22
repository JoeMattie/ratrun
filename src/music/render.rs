//! Offline previews of the floor music: `ratrun --music-render`.
//!
//! This is a dev tool, like `--prof-bench`: it synthesizes a floor's track
//! straight to a 16-bit mono WAV without opening an audio device, entering the
//! terminal or touching the save file. It exists so a music change can be
//! heard, and measured, without a sound card in the loop.
//!
//! ```text
//! ratrun --music-render <floor> <seconds> <out.wav> [--intensity N]
//! ```
//!
//! `<floor>` is a floor name as the captures spell it (`sewer`, `kitchen`,
//! `lab`, `laundry`, `printworks`, `flatbed`).

use std::io::Write;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use super::synth::SongSynth;
use super::{song_by_floor, theme_by_floor};
use crate::audio::SR;

/// Render `seconds` of a floor's song into a fresh buffer.
pub fn render_floor(floor: &str, seconds: f32, intensity: u32) -> Result<Vec<f32>> {
    render_floor_at(floor, seconds, intensity, SR)
}

/// Render at a chosen sample rate (the tests use a lower one for speed).
pub fn render_floor_at(
    floor: &str,
    seconds: f32,
    intensity: u32,
    sample_rate: u32,
) -> Result<Vec<f32>> {
    let Some(theme) = theme_by_floor(floor) else {
        bail!("--music-render: unknown floor {floor:?} (sewer, kitchen, lab, laundry, printworks, flatbed)");
    };
    if !(seconds.is_finite() && seconds > 0.0) {
        bail!("--music-render: seconds must be a positive number, got {seconds}");
    }
    let intensity = intensity.min(3);
    let n = (seconds * sample_rate as f32).round() as usize;
    // `RATRUN_MUSIC=classic` renders the pre-MIDI synth for a comparison (it
    // only exists at the engine rate).
    if super::classic_requested() && sample_rate == SR {
        let mut synth = crate::audio::MusicSynth::new(theme, Arc::new(AtomicU32::new(intensity)));
        return Ok((0..n).map(|_| synth.next().unwrap_or(0.0)).collect());
    }
    let mut synth = SongSynth::with_sample_rate(
        theme,
        Arc::new(AtomicU32::new(intensity)),
        sample_rate,
    );
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(synth.next().unwrap_or(0.0));
    }
    Ok(out)
}

/// Write mono `f32` samples as a 16-bit PCM WAV.
pub fn write_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("--music-render: cannot create {}", path.display()))?;
    let data_bytes = (samples.len() as u32) * 2;
    let rate = sample_rate.max(1);
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 2).to_le_bytes())?; // byte rate
    file.write_all(&2u16.to_le_bytes())?; // block align
    file.write_all(&16u16.to_le_bytes())?; // bits
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    let mut buf = Vec::with_capacity(data_bytes as usize);
    for &s in samples {
        let clamped = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        buf.extend_from_slice(&((clamped * 32767.0) as i16).to_le_bytes());
    }
    file.write_all(&buf)?;
    Ok(())
}

/// Handle `ratrun --music-render ...`. Returns `false` when the flag is absent,
/// so the caller can keep looking for another mode.
pub fn run_cli(args: &[String]) -> Result<bool> {
    let Some(at) = args.iter().position(|a| a == "--music-render") else {
        return Ok(false);
    };
    let mut positional = Vec::new();
    let mut intensity = 1u32;
    let mut i = at + 1;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--intensity" {
            let Some(v) = args.get(i + 1) else {
                bail!("--music-render: --intensity needs a value");
            };
            intensity = v
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("--music-render: bad --intensity {v:?}"))?;
            i += 2;
            continue;
        }
        if arg.starts_with("--") {
            // Belongs to another mode; leave it alone.
            i += 1;
            continue;
        }
        positional.push(arg.to_string());
        i += 1;
    }
    if positional.len() < 3 {
        bail!("usage: ratrun --music-render <floor> <seconds> <out.wav> [--intensity N]");
    }
    let floor = positional[0].clone();
    let seconds: f32 = positional[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("--music-render: bad seconds {:?}", positional[1]))?;
    let out_path = std::path::PathBuf::from(&positional[2]);
    let samples = render_floor(&floor, seconds, intensity)?;
    write_wav(&out_path, &samples, SR)?;
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    };
    let dc = if samples.is_empty() {
        0.0
    } else {
        samples.iter().sum::<f32>() / samples.len() as f32
    };
    let name = song_by_floor(&floor).map(|s| s.name).unwrap_or("?");
    println!(
        "music-render floor={floor} song=\"{name}\" intensity={} secs={seconds:.1} samples={} peak={peak:.3} rms={rms:.3} dc={dc:+.4} -> {}",
        intensity.min(3),
        samples.len(),
        out_path.display()
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_floor_to_a_readable_wav() {
        let dir = std::env::temp_dir().join("ratrun-music-render-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sewer.wav");
        let samples = render_floor_at("sewer", 0.5, 2, 22_050).unwrap();
        assert!(!samples.is_empty());
        assert!(samples.iter().all(|s| s.is_finite()));
        write_wav(&path, &samples, 22_050).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_floor_is_an_error_not_a_panic() {
        assert!(render_floor("submarine", 1.0, 1).is_err());
        assert!(render_floor("sewer", 0.0, 1).is_err());
    }
}
