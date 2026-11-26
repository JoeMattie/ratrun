//! Underwater colour grade and analytic caustic net for The Outfall.
//!
//! Render-only: no rng, no particle pool, no per-frame allocation and no wall
//! clock. Every effect is gated strictly to `Theme::Outfall` so the six earlier
//! floors stay bit-for-bit identical under both `fx = true` and `fx = false`.
//!
//! * [`grade_grid`] — theme-wide blue-green underwater colour grade over the
//!   lit playfield (floor, walls, props, bodies, the rat) on every lighting
//!   tier (`Full`, `Hard`, `Coarse`, `Compact`) and with `LIGHT EFFECTS` off.
//!   Warm colours desaturate slightly with depth/distance from light while warm
//!   lamps stay warm near their source; near-black abyssal tidal pits stay dark
//!   drops. Emissive pixels and post-lighting layers (hostile layer, HUD, ink
//!   marks, damage digits) stay untinted.
//! * [`apply_flat_grade`] — cheap flat underwater grade used when lighting is
//!   turned off (`LightingMode::Off` in captures/tests).
//! * [`apply_fx_caustics`] / [`apply_cheap_caustics`] — world-anchored analytic
//!   warped two-wave caustic web driven continuously by `(level.seed, elapsed)`.
//!   Strongest under lamps and in open halls, weaker in dark corridors/corners,
//!   faintly catching creatures, absent in `Blackout` zones and on walls, and
//!   quantised into a few flat brightness levels so neighbouring cells share
//!   colours and keep terminal byte churn low.

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{Canvas, FxFrame};
use super::material::{self, MaterialKind};
use super::motes::{caustics_here, sin_q8};
use crate::game::level::LightZoneKind;
#[cfg(test)]
use crate::game::level::Theme;
#[cfg(test)]
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;

/// Primary and secondary caustic wave drift speeds, in 256ths of a cycle per
/// second (~0.14 and ~0.09 cycles/s). Incommensurate speeds make the caustic
/// net shimmer organically while gliding slowly enough to keep cell churn low.
const DRIFT_A_PER_SEC: f32 = 36.0;
const DRIFT_B_PER_SEC: f32 = 23.0;

/// Full-tier Outfall caustic peak lift before `fx::strength::CAUSTICS` (0.5),
/// yielding up to 68 raw levels at full weight before flat quantisation.
const OUTFALL_CAUSTIC_GAIN: i32 = 108;

/// Cheap-path (`fx = false` / `Coarse` / `Compact`) caustic peak raw lift.
const CHEAP_CAUSTIC_GAIN: i32 = 50;

/// Cool sunlit aqua-cyan tint of refracted ocean caustics.
const OUTFALL_CAUSTIC_RGB: Rgb = (76, 224, 255);

/// Near-black pixel peak threshold below which caustics stay off (protects
/// bottomless abyssal tidal drop pits and out-of-bounds void).
const CAUSTIC_DARK_FLOOR: u8 = 16;

/// Flat proximity value used by [`apply_flat_grade`] when lighting is off:
/// cools the whole playfield into marine blue-teal while keeping bright warm
/// accents slightly legible.
#[cfg(test)]
const FLAT_GRADE_PROX: i32 = 64;

/// Grade one lit RGB pixel for The Outfall's water column.
///
/// `prox` is `0..=256`: how strongly this pixel sits inside a nearby light
/// source (`0` = ambient / deep water, `256` = hot core right at a lamp).
#[inline]
pub(super) fn grade_pixel((r, g, b): Rgb, prox: i32) -> Rgb {
    let r0 = i32::from(r);
    let g0 = i32::from(g);
    let b0 = i32::from(b);
    let peak = r0.max(g0).max(b0);
    if peak == 0 {
        return (0, 0, 0);
    }
    let prox = prox.clamp(0, 256);
    let depth = 256 - prox;

    // 1. Slight desaturation of warm colours with depth/distance from light.
    let luma = (r0 * 77 + g0 * 150 + b0 * 29) >> 8;
    let r1 = if r0 > luma && depth > 0 {
        let desat = (depth * 72) >> 8;
        r0 + (((luma - r0) * desat) >> 8)
    } else {
        r0
    };

    // 2. Water spectral absorption (red attenuates strongly, green stays near
    //    unity, blue-cyan rises), relaxing near bright warm light cores so
    //    warm lamps still read warm near their source.
    let warm = (((r0 - b0).max(0) * prox) >> 8).min(128);
    let kr = (116 + ((44 * prox) >> 8) + ((warm * 3) >> 2)).min(252);
    let kg = 262 + ((10 * prox) >> 8);
    let kb = (372 - ((24 * prox) >> 8) - ((warm * 3) >> 2)).max(236);

    let r2 = (r1 * kr) >> 8;
    let g2 = (g0 * kg) >> 8;
    let b2 = (b0 * kb) >> 8;

    // 3. Cool blue-green water-column in-scattering, tapered smoothly on
    //    near-black pixels so abyssal tidal pits stay dark drops, and softened
    //    at hot warm lamp cores.
    let dark_scale = peak.min(8);
    let veil_scale = 160 + ((96 * depth) >> 8);
    let warm_keep = 256 - warm;
    let vg = (((((16 * veil_scale) >> 8) * warm_keep) >> 8) * dark_scale) >> 3;
    let vb = (((((38 * veil_scale) >> 8) * warm_keep) >> 8) * dark_scale) >> 3;

    (
        r2.clamp(0, 255) as u8,
        (g2 + vg).clamp(0, 255) as u8,
        (b2 + vb).clamp(0, 255) as u8,
    )
}

/// Coarse 5-step light proximity (`0, 64, 128, 192, 256`) from a sample's peak
/// multiplier so neighbouring cells share identical grading parameters.
#[inline]
fn sample_prox(mul: [u16; 3]) -> i32 {
    let peak = i32::from(mul[0].max(mul[1]).max(mul[2]));
    let raw = ((peak - 104).clamp(0, 224) * 256) / 224;
    (((raw + 32) / 64) * 64).clamp(0, 256)
}

/// Apply The Outfall's underwater colour grade across the lit world canvas.
/// Runs on every lighting tier (`Full`, `Hard`, `Coarse`, `Compact`) and with
/// `LIGHT EFFECTS` both on and off.
pub(super) fn grade_grid(grid: &Grid, canvas: &mut Canvas<'_>) {
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.stride == 0 || grid.w == 0 || grid.h == 0 {
        return;
    }
    let stride = grid.stride;
    let (ox, oy) = grid.offset;
    let first = (stride - ox % stride).min(w);
    let first_sample = ox / stride;
    let has_mat = canvas.mat.len() >= w * h;

    for y in 0..h {
        let by = y + oy;
        let row_base = (by / stride).min(grid.h - 1) * grid.w;
        let row_px = y * w;
        let mut x = 0;
        let mut end = first;
        let mut sx = first_sample;
        while x < w {
            let sample_idx = row_base + sx.min(grid.w - 1);
            let prox = sample_prox(grid.samples[sample_idx].mul);
            if has_mat {
                for px in x..end {
                    let i = row_px + px;
                    if material::kind(canvas.mat[i]) == MaterialKind::Emissive {
                        continue;
                    }
                    canvas.px[i] = grade_pixel(canvas.px[i], prox);
                }
            } else {
                for px in x..end {
                    let i = row_px + px;
                    canvas.px[i] = grade_pixel(canvas.px[i], prox);
                }
            }
            x = end;
            end = (end + stride).min(w);
            sx += 1;
        }
    }
}

/// Cheap flat underwater blue-green grade for `LightingMode::Off`.
#[cfg(test)]
pub(super) fn apply_flat_grade(pb: &mut PixelBuffer) {
    for pixel in pb.pixels_mut() {
        *pixel = grade_pixel(*pixel, FLAT_GRADE_PROX);
    }
}

/// Apply the cheap flat underwater grade when `LightingMode::Off` is active on
/// The Outfall; a no-op on every other theme.
#[cfg(test)]
pub(crate) fn apply_off_grade(pb: &mut PixelBuffer, theme: Theme) {
    if theme == Theme::Outfall {
        apply_flat_grade(pb);
    }
}

/// Deterministic spatial offset and initial phase from `level.seed` (no rng).
#[inline]
fn seed_params(seed: u64) -> (i32, i32, i64, i64) {
    let lo = seed as u32;
    let hi = (seed >> 32) as u32;
    let mix = lo.wrapping_mul(0x9e37_79b9) ^ hi.wrapping_mul(0x7feb_352d);
    let ox = (mix & 0x3ff) as i32;
    let oy = ((mix >> 10) & 0x3ff) as i32;
    let sp1 = i64::from((mix >> 20) & 0xff);
    let sp2 = i64::from((mix >> 24) & 0xff);
    (ox, oy, sp1, sp2)
}

#[inline]
fn phases(seed: u64, elapsed: f32) -> (i32, i32, i32, i32) {
    let (ox, oy, sp1, sp2) = seed_params(seed);
    let t = elapsed.max(0.0);
    let p1 = ((t * DRIFT_A_PER_SEC).floor() as i64 + sp1).rem_euclid(256) as i32;
    let p2 = ((t * DRIFT_B_PER_SEC).floor() as i64 + sp2).rem_euclid(256) as i32;
    (ox, oy, p1, p2)
}

/// Warped two-front caustic web intensity at world pixel `(wx, wy)`, Q8.8
/// (`0..=256`). Forms a connected curved honeycomb of bright refracted ribbons
/// with brighter intersection nodes and broad quiet troughs.
#[inline]
pub(super) fn caustic_net_q8(wx: i32, wy: i32, ox: i32, oy: i32, p1: i32, p2: i32) -> i32 {
    let x = wx.wrapping_add(ox);
    let y = wy.wrapping_add(oy);
    let w1 = sin_q8(((x.wrapping_mul(4) - y.wrapping_mul(7) + p2) & 255) as u8) >> 3;
    let w2 = sin_q8(((x.wrapping_mul(6) + y.wrapping_mul(5) - p1) & 255) as u8) >> 3;
    let i1 = (x.wrapping_mul(10) + y.wrapping_mul(6) + p1 + w1) & 255;
    let i2 = (y.wrapping_mul(11) - x.wrapping_mul(7) - p2 + w2) & 255;
    let s1 = 256 - sin_q8(i1 as u8).abs();
    let s2 = 256 - sin_q8(i2 as u8).abs();
    let r1 = (((s1 * s1) >> 8) * s1) >> 8;
    let r2 = (((s2 * s2) >> 8) * s2) >> 8;
    let web = r1.max(r2);
    let node = (r1 * r2) >> 8;
    ((web * 176 + node * 112) >> 8).min(256)
}

/// Faster unwarped caustic web evaluated on 2×2 world-pixel blocks for
/// `LIGHT EFFECTS` off and `Coarse`/`Compact` tiers.
#[inline]
fn cheap_caustic_q8(wx: i32, wy: i32, ox: i32, oy: i32, p1: i32, p2: i32) -> i32 {
    let x = (wx & !1).wrapping_add(ox);
    let y = (wy & !1).wrapping_add(oy);
    let i1 = (x.wrapping_mul(10) + y.wrapping_mul(6) + p1) & 255;
    let i2 = (y.wrapping_mul(11) - x.wrapping_mul(7) - p2) & 255;
    let s1 = 256 - sin_q8(i1 as u8).abs();
    let s2 = 256 - sin_q8(i2 as u8).abs();
    let r1 = (((s1 * s1) >> 8) * s1) >> 8;
    let r2 = (((s2 * s2) >> 8) * s2) >> 8;
    let web = r1.max(r2);
    let node = (r1 * r2) >> 8;
    ((web * 176 + node * 112) >> 8).min(256)
}

/// Ambient refracted-sunlight caustic base (`0..=256`) by room zone on The
/// Outfall: open halls carry gentle ocean light, corridors are dimmer, and
/// dark maze corners, blackouts and unzoned tiles wait for a local lamp.
#[inline]
pub(super) fn zone_caustic_base(zone: Option<LightZoneKind>) -> i32 {
    match zone {
        Some(LightZoneKind::Start | LightZoneKind::Room | LightZoneKind::Arena) => 112,
        Some(LightZoneKind::Corridor) => 56,
        Some(LightZoneKind::Maze | LightZoneKind::Blackout) | None => 0,
    }
}

/// Quantise a raw caustic lift (`0..=68`) into a few flat levels so adjacent
/// cells along the same filament share colours and cut SGR byte churn.
#[inline]
fn quantise_fx_level(raw: i32, low_tier: bool) -> i32 {
    if raw < 14 {
        0
    } else if low_tier {
        if raw < 32 {
            13
        } else {
            30
        }
    } else if raw < 28 {
        12
    } else if raw < 46 {
        25
    } else {
        43
    }
}

#[inline]
fn quantise_cheap_level(raw: i32) -> i32 {
    if raw < 14 {
        0
    } else if raw < 32 {
        18
    } else {
        36
    }
}

/// Rich Outfall caustic net (`fx = true`, `Full` / `Hard`): bright animated
/// aqua filaments over lit floor and open halls, faint ripples over bodies,
/// skipping walls, emissives, abyssal pits and `Blackout` zones.
pub(super) fn apply_fx_caustics(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    elapsed: f32,
    gain_q8: i32,
) {
    if gain_q8 <= 0 || canvas.w == 0 || canvas.h == 0 {
        return;
    }
    let (ox, oy, p1, p2) = phases(frame.level.seed, elapsed);
    let cam_x = frame.cam.x.round() as i32;
    let cam_y = frame.cam.y.round() as i32;
    let (w, h) = (canvas.w, canvas.h);
    let low_tier = frame.mode == LightingMode::Hard || frame.depth != ColorDepth::Truecolor;

    for py in 0..h {
        let wy = cam_y + py as i32;
        let row_px = py * w;
        for px in 0..w {
            let i = grid.sample_index(px, py);
            let Some(surface) = grid.surfaces.get(i) else {
                continue;
            };
            if surface.layer & LayerMask::FLOOR.0 == 0 {
                continue;
            }
            if !caustics_here(frame.theme, surface.zone) {
                continue;
            }
            let Some(info) = grid.info.get(i) else {
                continue;
            };
            let base = zone_caustic_base(surface.zone);
            let lit = (i32::from(info.dom).min(240) * 256) / 240;
            let weight = if surface.zone == Some(LightZoneKind::Maze) {
                lit.min(96)
            } else {
                (base + lit).min(256)
            };
            if weight <= 0 {
                continue;
            }

            let index = row_px + px;
            let byte = canvas.mat_at(index);
            if material::kind(byte) == MaterialKind::Emissive {
                continue;
            }
            let pixel = canvas.px[index];
            if pixel.0.max(pixel.1).max(pixel.2) < CAUSTIC_DARK_FLOOR {
                continue;
            }

            let wx = cam_x + px as i32;
            let net = caustic_net_q8(wx, wy, ox, oy, p1, p2);
            let mut raw = (((((net * OUTFALL_CAUSTIC_GAIN) >> 8) * gain_q8) >> 8) * weight) >> 8;
            if material::is_creature(byte) {
                raw = (raw * 3) >> 3;
            }
            let level = quantise_fx_level(raw, low_tier);
            if level <= 0 {
                continue;
            }

            let tint = if info.dom > 0 {
                (
                    ((u16::from(info.dom_rgb.0) + u16::from(OUTFALL_CAUSTIC_RGB.0) * 2) / 3) as u8,
                    ((u16::from(info.dom_rgb.1) + u16::from(OUTFALL_CAUSTIC_RGB.1) * 2) / 3) as u8,
                    ((u16::from(info.dom_rgb.2) + u16::from(OUTFALL_CAUSTIC_RGB.2) * 2) / 3) as u8,
                )
            } else {
                OUTFALL_CAUSTIC_RGB
            };
            canvas.px[index] = (
                pixel.0.saturating_add(((level * i32::from(tint.0)) >> 8) as u8),
                pixel.1.saturating_add(((level * i32::from(tint.1)) >> 8) as u8),
                pixel.2.saturating_add(((level * i32::from(tint.2)) >> 8) as u8),
            );
        }
    }
}

/// Cheaper 2×2-blocked Outfall caustic net for `LIGHT EFFECTS` off and the
/// `Coarse`/`Compact` lighting tiers: evaluates once per 2×2 block using the
/// light grid's sample multipliers instead of `LightInfo`.
pub(super) fn apply_cheap_caustics(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    elapsed: f32,
) {
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 || grid.stride == 0 || grid.w == 0 || grid.h == 0 {
        return;
    }
    let (ox, oy, p1, p2) = phases(frame.level.seed, elapsed);
    let cam_x = frame.cam.x.round() as i32;
    let cam_y = frame.cam.y.round() as i32;
    let has_mat = canvas.mat.len() >= w * h;

    let mut py = 0;
    while py < h {
        let wy = cam_y + py as i32;
        let y_end = (py + 2).min(h);
        let mut px = 0;
        while px < w {
            let x_end = (px + 2).min(w);
            let i = grid.sample_index(px, py);
            let Some(surface) = grid.surfaces.get(i) else {
                px = x_end;
                continue;
            };
            if surface.layer & LayerMask::FLOOR.0 == 0
                || surface.zone == Some(LightZoneKind::Blackout)
            {
                px = x_end;
                continue;
            }
            let Some(sample) = grid.samples.get(i) else {
                px = x_end;
                continue;
            };
            let base = zone_caustic_base(surface.zone);
            let mul_peak = i32::from(sample.mul[0].max(sample.mul[1]).max(sample.mul[2]));
            let lit = ((mul_peak - 104).clamp(0, 200) * 256) / 200;
            let weight = if surface.zone == Some(LightZoneKind::Maze) {
                lit.min(96)
            } else {
                (base + lit).min(256)
            };
            if weight <= 0 {
                px = x_end;
                continue;
            }

            let wx = cam_x + px as i32;
            let net = cheap_caustic_q8(wx, wy, ox, oy, p1, p2);
            let raw = (((net * CHEAP_CAUSTIC_GAIN) >> 8) * weight) >> 8;
            let level = quantise_cheap_level(raw);
            if level > 0 {
                let dr = ((level * i32::from(OUTFALL_CAUSTIC_RGB.0)) >> 8) as u8;
                let dg = ((level * i32::from(OUTFALL_CAUSTIC_RGB.1)) >> 8) as u8;
                let db = ((level * i32::from(OUTFALL_CAUSTIC_RGB.2)) >> 8) as u8;
                for yy in py..y_end {
                    let row_px = yy * w;
                    for xx in px..x_end {
                        let si = grid.sample_index(xx, yy);
                        if let Some(surf) = grid.surfaces.get(si) {
                            if surf.layer & LayerMask::FLOOR.0 == 0
                                || surf.zone == Some(LightZoneKind::Blackout)
                            {
                                continue;
                            }
                        }
                        let idx = row_px + xx;
                        if has_mat && material::kind(canvas.mat[idx]) == MaterialKind::Emissive {
                            continue;
                        }
                        let p = canvas.px[idx];
                        if p.0.max(p.1).max(p.2) < CAUSTIC_DARK_FLOOR {
                            continue;
                        }
                        canvas.px[idx] = (
                            p.0.saturating_add(dr),
                            p.1.saturating_add(dg),
                            p.2.saturating_add(db),
                        );
                    }
                }
            }
            px = x_end;
        }
        py = y_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::map_rgb_with;
    use ratatui::style::Color;

    #[test]
    fn grade_shifts_neutral_seabed_toward_blue_cyan_and_survives_256_folding() {
        // Outfall sand under mid lighting before grade looks olive-brown.
        let sand = (42, 46, 44);
        let graded = grade_pixel(sand, 64);
        assert!(
            graded.2 > graded.0 + 40 && graded.1 > graded.0 + 25,
            "graded sand must read blue-green: {graded:?}"
        );
        // Must land in the xterm-256 RGB colour cube (16..232), not greyscale (232..=255).
        match map_rgb_with(ColorDepth::Ansi256, graded) {
            Color::Indexed(idx) => {
                assert!(
                    (16..232).contains(&idx),
                    "graded Outfall sand folded to greyscale index {idx} ({graded:?})"
                );
            }
            other => panic!("expected Indexed colour, got {other:?}"),
        }
    }

    #[test]
    fn grade_keeps_warm_lamps_warm_near_source_and_cools_them_with_depth() {
        let warm_near = grade_pixel((230, 175, 100), 256);
        assert!(
            warm_near.0 > warm_near.2 + 40,
            "warm lamp core must remain warm near its source: {warm_near:?}"
        );
        let warm_far = grade_pixel((115, 87, 50), 0);
        assert!(
            warm_far.2 > warm_far.0 + 30,
            "warm tones far from light must cool under water: {warm_far:?}"
        );
    }

    #[test]
    fn grade_preserves_black_void_and_dark_abyssal_pits() {
        assert_eq!(grade_pixel((0, 0, 0), 0), (0, 0, 0));
        let pit = grade_pixel((0, 0, 2), 0);
        assert!(
            pit.0.max(pit.1).max(pit.2) < CAUSTIC_DARK_FLOOR,
            "abyssal pit must stay dark: {pit:?}"
        );
    }

    #[test]
    fn caustic_net_varies_across_space_and_continuous_time_and_holds_when_frozen() {
        let (ox, oy, p1a, p2a) = phases(42, 3.000);
        let (_, _, p1b, p2b) = phases(42, 3.040);
        let (_, _, p1f, p2f) = phases(42, 3.000);
        assert_eq!((p1a, p2a), (p1f, p2f), "frozen time produces identical phases");
        assert_ne!((p1a, p2a), (p1b, p2b), "continuous drift moves inside 8 Hz window");

        let mut min_v = i32::MAX;
        let mut max_v = i32::MIN;
        let mut moved = 0usize;
        for wy in 0..48 {
            for wx in 0..64 {
                let a = caustic_net_q8(wx, wy, ox, oy, p1a, p2a);
                let b = caustic_net_q8(wx, wy, ox, oy, p1b, p2b);
                min_v = min_v.min(a);
                max_v = max_v.max(a);
                if a != b {
                    moved += 1;
                }
            }
        }
        assert!(min_v <= 10 && max_v >= 200, "net needs dark troughs and bright crests: {min_v}..{max_v}");
        assert!(moved > 100, "caustic net must animate smoothly over time");
    }
}
