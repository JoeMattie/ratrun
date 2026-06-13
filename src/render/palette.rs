//! Color helpers: rgb mixing, gradient ramps, and per-theme palettes.

use ratatui::style::Color;

pub type Rgb = (u8, u8, u8);

pub fn to_color(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

pub fn scale(a: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (a.0 as f32 * f).min(255.0) as u8,
        (a.1 as f32 * f).min(255.0) as u8,
        (a.2 as f32 * f).min(255.0) as u8,
    )
}

pub fn add(a: Rgb, b: Rgb) -> Rgb {
    (
        (a.0 as u16 + b.0 as u16).min(255) as u8,
        (a.1 as u16 + b.1 as u16).min(255) as u8,
        (a.2 as u16 + b.2 as u16).min(255) as u8,
    )
}

/// Sample a multi-stop gradient at `t` in [0, 1].
pub fn ramp(stops: &[(f32, Rgb)], t: f32) -> Rgb {
    if stops.is_empty() {
        return (0, 0, 0);
    }
    let t = t.clamp(0.0, 1.0);
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t >= t0 && t <= t1 {
            let local = if (t1 - t0).abs() < 1e-6 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            return mix(c0, c1, local);
        }
    }
    stops[stops.len() - 1].1
}
