//! Pixel art for the five XP gem tiers: Shard, Chip, Gem, Crystal, and Core.
//!
//! Pure render-only drawing (zero rng, zero heap allocation). Gems follow the
//! loot shape language: **diamonds, faceted crystals, and a dark-rimmed orb**,
//! never a coin square (`art_floor::draw_gold_coin`) and never a pocket-find
//! bracket (`render::ink`).
//!
//! Each tier scales monotonically in both colour and pixel footprint so players
//! can read XP value at a glance across all six floor palettes (Sewer, Kitchen,
//! Lab, Laundry, Printworks, Flatbed) and at every zoom:
//!
//! | Tier    | Value    | Radius | Footprint & silhouette                        | Motion                          |
//! | ------- | -------- | ------ | --------------------------------------------- | ------------------------------- |
//! | Shard   | 1–4      | 1 px   | 3 px: 1-px core + 2 dim additive neighbours   | Still                           |
//! | Chip    | 5–24     | 1 px   | 5 px: 3-px cyan diamond                       | Still                           |
//! | Gem     | 25–99    | 2 px   | 13 px: 5-px violet diamond with bright facet  | Still                           |
//! | Crystal | 100–499  | 3 px   | 25 px: 7-px 4-quadrant faceted amber crystal  | Slow continuous smooth breath   |
//! | Core    | 500+     | 4 px   | 57 px: 9-px glowing crimson orb with dark rim | Slow continuous smooth breath   |
//!
//! Every pixel painted by `draw_gem` — including the single-frame bright flash
//! when `g.merged` is `true` — stays strictly inside the Chebyshev square
//! `[s.0 - r, s.0 + r] × [s.1 - r, s.1 + r]` where `r = tier.radius_px()`,
//! matching the emissive box marked by the caller.

#![allow(clippy::disallowed_methods)] // render-only: platform maths is fine

use crate::game::gem::{Gem, GemTier};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

const WHITE: Rgb = (255, 255, 255);

/// Angular frequency (rad/s) of Crystal's slow smooth brightness breath (~3.14 s period).
const CRYSTAL_BREATH_SPEED: f32 = 2.0;
/// Angular frequency (rad/s) of Core's slow smooth brightness breath (~2.62 s period).
const CORE_BREATH_SPEED: f32 = 2.4;

/// Deterministic phase offset in `[0, TAU)` derived from `seed`, so a field of
/// Crystals or Cores never breathes in lockstep.
#[inline]
fn seed_phase(seed: u32) -> f32 {
    let h = seed.wrapping_mul(0x9e37_79b9) >> 16;
    (h as f32) * (std::f32::consts::TAU / 65536.0)
}

/// Smooth continuous `[0.0, 1.0]` breath wave from `elapsed` and `seed`.
///
/// Never quantised or threshold-stepped; non-finite `elapsed` clamps cleanly to `0.0`.
#[inline]
fn smooth_breath(elapsed: f32, speed: f32, seed: u32) -> f32 {
    let t = if elapsed.is_finite() { elapsed } else { 0.0 };
    0.5 + 0.5 * (t * speed + seed_phase(seed)).sin()
}

/// Draw one XP gem centred at screen pixel `s`.
///
/// Pure render-only: reads `g.value`, `g.color`, `g.merged`, `elapsed`, and
/// `seed`, and writes only pixels inside `[-r..=r] × [-r..=r]` around `s`
/// (`r = GemTier::for_value(g.value).radius_px()`).
pub fn draw_gem(pb: &mut PixelBuffer, s: (i32, i32), g: &Gem, elapsed: f32, seed: u32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let tier = GemTier::for_value(g.value);
    let r = tier.radius_px();
    let (cx, cy) = s;
    if cx.saturating_add(r) < 0
        || cy.saturating_add(r) < 0
        || cx.saturating_sub(r) >= pb.w as i32
        || cy.saturating_sub(r) >= pb.h as i32
    {
        return;
    }

    let base = g.color;
    match tier {
        GemTier::Shard => draw_shard(pb, cx, cy, base, g.merged),
        GemTier::Chip => draw_chip(pb, cx, cy, base, g.merged),
        GemTier::Gem => draw_mid_gem(pb, cx, cy, base, g.merged),
        GemTier::Crystal => draw_crystal(pb, cx, cy, base, elapsed, seed, g.merged),
        GemTier::Core => draw_core(pb, cx, cy, base, elapsed, seed, g.merged),
    }
}

/// Tier 1 — Shard (`radius_px = 1`): unchanged legacy 3-pixel footprint
/// (1-px core + 2 dim additive neighbours) when unmerged; crisp 5-px flash
/// cross strictly within `r = 1` on the merge frame.
fn draw_shard(pb: &mut PixelBuffer, cx: i32, cy: i32, base: Rgb, merged: bool) {
    if merged {
        pb.plot(cx, cy, WHITE);
        let arm = mix(base, WHITE, 0.55);
        pb.plot_add(cx + 1, cy, arm);
        pb.plot_add(cx - 1, cy, arm);
        pb.plot_add(cx, cy + 1, arm);
        pb.plot_add(cx, cy - 1, arm);
    } else {
        pb.plot(cx, cy, base);
        let dim = scale(base, 0.5);
        pb.plot_add(cx + 1, cy, dim);
        pb.plot_add(cx, cy + 1, dim);
    }
}

/// Tier 2 — Chip (`radius_px = 1`): 3-px diamond (5 pixels) with top-left
/// specular lighting. Still image; brightens cleanly on the merge frame.
fn draw_chip(pb: &mut PixelBuffer, cx: i32, cy: i32, base: Rgb, merged: bool) {
    let (hi, lit, shd) = if merged {
        (WHITE, mix(base, WHITE, 0.80), mix(base, WHITE, 0.55))
    } else {
        (
            mix(base, WHITE, 0.62),
            mix(base, WHITE, 0.16),
            scale(base, 0.72),
        )
    };
    pb.plot(cx, cy - 1, lit);
    pb.plot(cx - 1, cy, lit);
    pb.plot(cx, cy, hi);
    pb.plot(cx + 1, cy, shd);
    pb.plot(cx, cy + 1, shd);
}

/// Tier 3 — Gem (`radius_px = 2`): 5-px cut diamond (13 pixels in `|dx|+|dy| <= 2`)
/// with a bright upper-left specular facet and deep lower-right bevel. Still image.
fn draw_mid_gem(pb: &mut PixelBuffer, cx: i32, cy: i32, base: Rgb, merged: bool) {
    let (rim_lo, mid_lo, mid, facet_hi, spec, core) = if merged {
        // Non-boxy radial starburst inside r=2 (skipping outer ±2,±2 corners).
        let glow = scale(mix(base, WHITE, 0.50), 0.36);
        for &(dx, dy) in &[
            (-2, -1),
            (2, -1),
            (-2, 1),
            (2, 1),
            (-1, -2),
            (1, -2),
            (-1, 2),
            (1, 2),
        ] {
            pb.plot_add(cx + dx, cy + dy, glow);
        }
        (
            mix(base, WHITE, 0.58),
            mix(base, WHITE, 0.72),
            mix(base, WHITE, 0.84),
            mix(base, WHITE, 0.94),
            WHITE,
            WHITE,
        )
    } else {
        (
            scale(base, 0.54),
            scale(base, 0.78),
            base,
            mix(base, WHITE, 0.46),
            mix(base, WHITE, 0.86),
            mix(base, WHITE, 0.28),
        )
    };

    // Outer diamond tips & upper-left lit edge.
    pb.plot(cx, cy - 2, mid);
    pb.plot(cx - 2, cy, mid);
    pb.plot(cx - 1, cy - 1, facet_hi);
    // Bright upper-left specular facet & core.
    pb.plot(cx, cy - 1, spec);
    pb.plot(cx - 1, cy, facet_hi);
    pb.plot(cx, cy, core);
    // Transition shoulders.
    pb.plot(cx + 1, cy - 1, mid);
    pb.plot(cx - 1, cy + 1, mid_lo);
    // Shaded lower-right cut facets.
    pb.plot(cx + 1, cy, mid_lo);
    pb.plot(cx, cy + 1, mid_lo);
    pb.plot(cx + 2, cy, rim_lo);
    pb.plot(cx + 1, cy + 1, rim_lo);
    pb.plot(cx, cy + 2, rim_lo);
}

/// Tier 4 — Crystal (`radius_px = 3`): 7-px 4-quadrant faceted bipyramid crystal
/// (25 pixels in `|dx|+|dy| <= 3`) with a slow smooth continuous brightness breath.
/// Sharp pointed silhouette + burnt-amber bevels keep it distinct from round gold
/// coins even on warm sunlit Flatbed planks.
fn draw_crystal(
    pb: &mut PixelBuffer,
    cx: i32,
    cy: i32,
    base: Rgb,
    elapsed: f32,
    seed: u32,
    merged: bool,
) {
    let b = smooth_breath(elapsed, CRYSTAL_BREATH_SPEED, seed);

    let (bevel_dark, facet_deep, facet_warm, facet_lit, facet_spec, tip_lit, center_col) = if merged
    {
        // Soft diagonal flash shimmer inside r=3 (never fills outer square corners).
        let glow = scale(mix(base, WHITE, 0.55), 0.40);
        for &(dx, dy) in &[
            (-2, -2),
            (2, -2),
            (-2, 2),
            (2, 2),
        ] {
            pb.plot_add(cx + dx, cy + dy, glow);
        }
        (
            mix(base, WHITE, 0.52),
            mix(base, WHITE, 0.68),
            mix(base, WHITE, 0.80),
            mix(base, WHITE, 0.92),
            WHITE,
            mix(base, WHITE, 0.88),
            WHITE,
        )
    } else {
        let lit = mix(base, (255, 244, 178), 0.34 + 0.28 * b);
        (
            mix((96, 36, 8), scale(base, 0.44), 0.55),
            mix((186, 92, 16), scale(base, 0.72), 0.50),
            mix(scale(base, 0.90), base, 0.50 + 0.50 * b),
            lit,
            mix(base, WHITE, 0.64 + 0.30 * b),
            mix(base, WHITE, 0.22 + 0.22 * b),
            mix(lit, WHITE, 0.45 + 0.32 * b),
        )
    };

    for dy in -3..=3i32 {
        for dx in -3..=3i32 {
            if dx.abs() + dy.abs() > 3 {
                continue;
            }
            let col = match (dx, dy) {
                // Sharp lower-right outer bevel.
                (3, 0) | (2, 1) | (1, 2) | (0, 3) => bevel_dark,
                // Sharp upper & left crystal tips.
                (0, -3) | (-3, 0) => tip_lit,
                // Upper-left lit edge & inner crown highlights.
                (-1, -2) | (-2, -1) | (0, -2) | (-2, 0) => facet_lit,
                // Upper-right facet slope & diagonal girdle transitions.
                (1, -2) | (2, -1) | (1, -1) | (-1, 1) => facet_warm,
                // Blazing upper-left specular crown facet.
                (-1, -1) | (0, -1) | (-1, 0) => facet_spec,
                // Incandescent central table ridge.
                (0, 0) => center_col,
                // Lower-left slope & lower-right inner table shadow.
                (-2, 1) | (-1, 2) | (1, 0) | (0, 1) | (2, 0) | (0, 2) => facet_deep,
                // Deep lower-right pavilion pocket (1, 1).
                _ => bevel_dark,
            };
            pb.plot(cx + dx, cy + dy, col);
        }
    }
}

/// Tier 5 — Core (`radius_px = 4`): 9-px glowing crimson orb with a dark outer
/// containment rim (57 pixels in `dx² + dy² <= 17`) and a slow smooth continuous
/// brightness breath. Fixed radial geometry guarantees zero threshold pops as
/// `elapsed` advances.
fn draw_core(
    pb: &mut PixelBuffer,
    cx: i32,
    cy: i32,
    base: Rgb,
    elapsed: f32,
    seed: u32,
    merged: bool,
) {
    let b = smooth_breath(elapsed, CORE_BREATH_SPEED, seed);

    let (rim_dark, rim_lit, outer_ruby, mid_crimson, inner_hot, heart) = if merged {
        // Strictly circular flash inside d² <= 17 — never paints outer square corners.
        (
            mix(base, WHITE, 0.42),
            mix(base, WHITE, 0.66),
            mix(base, WHITE, 0.72),
            mix(base, WHITE, 0.86),
            mix(base, WHITE, 0.95),
            WHITE,
        )
    } else {
        (
            (26, 6, 12),
            (74, 18, 28),
            scale(base, 0.48 + 0.16 * b),
            mix(scale(base, 0.84), mix(base, (255, 148, 138), 0.32), b),
            mix(base, (255, 224, 214), 0.38 + 0.38 * b),
            mix(base, WHITE, 0.72 + 0.24 * b),
        )
    };

    for dy in -4..=4i32 {
        for dx in -4..=4i32 {
            let d2 = dx * dx + dy * dy;
            if d2 > 17 {
                continue;
            }
            let col = if d2 > 10 {
                // 1-px dark outer containment rim with subtle upper-left sheen.
                if dx + dy <= -3 {
                    rim_lit
                } else {
                    rim_dark
                }
            } else {
                // Inner 7×7 glowing orb with fixed radial rings around (-0.45, -0.45).
                let sx = dx as f32 + 0.45;
                let sy = dy as f32 + 0.45;
                let sd2 = sx * sx + sy * sy;
                if sd2 <= 1.25 {
                    heart
                } else if sd2 <= 3.85 {
                    inner_hot
                } else if d2 <= 6 || dx + dy < 1 {
                    mid_crimson
                } else {
                    outer_ruby
                }
            };
            pb.plot(cx + dx, cy + dy, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;

    /// Representative XP value for each of the five tiers in ascending order.
    const SAMPLE_TIERS: [(GemTier, u32); 5] = [
        (GemTier::Shard, 1),
        (GemTier::Chip, 5),
        (GemTier::Gem, 25),
        (GemTier::Crystal, 100),
        (GemTier::Core, 500),
    ];

    const BG: Rgb = (14, 22, 20);

    fn sample_gem(value: u32, merged: bool) -> Gem {
        let mut g = Gem::new(Vec2::ZERO, value);
        g.merged = merged;
        g
    }

    fn count_painted(pb: &PixelBuffer, bg: Rgb) -> usize {
        pb.pixels().iter().filter(|&&c| c != bg).count()
    }

    fn total_luminance(pb: &PixelBuffer) -> u64 {
        pb.pixels()
            .iter()
            .map(|&(r, g, b)| u64::from(r) + u64::from(g) + u64::from(b))
            .sum()
    }

    fn px_lum(c: Rgb) -> u32 {
        u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
    }

    #[test]
    fn each_tier_paints_within_radius_px_of_centre() {
        let (cx, cy) = (16i32, 16i32);
        for &(tier, value) in &SAMPLE_TIERS {
            assert_eq!(GemTier::for_value(value), tier);
            let r = tier.radius_px();
            for merged in [false, true] {
                let g = sample_gem(value, merged);
                for elapsed in [0.0f32, 0.75, 2.35, 11.4] {
                    let mut pb = PixelBuffer::new(33, 33);
                    pb.clear(BG);
                    draw_gem(&mut pb, (cx, cy), &g, elapsed, 42);

                    let mut painted = 0usize;
                    for y in 0..pb.h {
                        for x in 0..pb.w {
                            if pb.pixel_at(x, y) != BG {
                                painted += 1;
                                let dx = (x as i32 - cx).abs();
                                let dy = (y as i32 - cy).abs();
                                assert!(
                                    dx <= r && dy <= r,
                                    "{tier:?} (merged={merged}) painted ({x},{y}) outside radius {r} from ({cx},{cy})"
                                );
                            }
                        }
                    }
                    assert!(painted > 0, "{tier:?} (merged={merged}) painted nothing");
                    assert_ne!(
                        pb.pixel_at(cx as usize, cy as usize),
                        BG,
                        "{tier:?} (merged={merged}) must paint its centre pixel"
                    );
                }
            }
        }
    }

    #[test]
    fn higher_tiers_paint_strictly_more_pixels_and_shard_keeps_legacy_footprint() {
        let (cx, cy) = (16i32, 16i32);
        let mut prev_count = 0usize;
        let mut prev_tier = None;
        for &(tier, value) in &SAMPLE_TIERS {
            let g = sample_gem(value, false);
            let mut pb = PixelBuffer::new(33, 33);
            pb.clear(BG);
            draw_gem(&mut pb, (cx, cy), &g, 0.7, 7);

            let count = count_painted(&pb, BG);
            assert!(
                count > prev_count,
                "{tier:?} painted {count} px, expected strictly more than {prev_tier:?}'s {prev_count} px"
            );
            if tier == GemTier::Shard {
                // Shard keeps today's exact 3-pixel look: (0,0), (1,0), (0,1).
                assert_eq!(count, 3, "Shard must keep its 3-pixel legacy footprint");
                assert_eq!(pb.pixel_at(cx as usize, cy as usize), g.color);
                assert_ne!(pb.pixel_at((cx + 1) as usize, cy as usize), BG);
                assert_ne!(pb.pixel_at(cx as usize, (cy + 1) as usize), BG);
            }
            prev_count = count;
            prev_tier = Some(tier);
        }
    }

    #[test]
    fn merged_gem_paints_brighter_than_unmerged() {
        let (cx, cy) = (16i32, 16i32);
        for &(tier, value) in &SAMPLE_TIERS {
            let plain = sample_gem(value, false);
            let merged = sample_gem(value, true);
            // Test at both breath trough and breath crest.
            for elapsed in [0.65f32, 0.785, 1.96, 2.356] {
                let mut pb_plain = PixelBuffer::new(33, 33);
                let mut pb_merged = PixelBuffer::new(33, 33);
                pb_plain.clear(BG);
                pb_merged.clear(BG);
                draw_gem(&mut pb_plain, (cx, cy), &plain, elapsed, 0);
                draw_gem(&mut pb_merged, (cx, cy), &merged, elapsed, 0);

                let lum_plain = total_luminance(&pb_plain);
                let lum_merged = total_luminance(&pb_merged);
                assert!(
                    lum_merged > lum_plain,
                    "{tier:?} at t={elapsed}: merged total luminance ({lum_merged}) must exceed unmerged ({lum_plain})"
                );
                let c_plain = px_lum(pb_plain.pixel_at(cx as usize, cy as usize));
                let c_merged = px_lum(pb_merged.pixel_at(cx as usize, cy as usize));
                assert!(
                    c_merged > c_plain,
                    "{tier:?} at t={elapsed}: merged centre brightness ({c_merged}) must exceed unmerged ({c_plain})"
                );
            }
        }
    }

    #[test]
    fn no_time_stepped_change_across_elapsed() {
        let (cx, cy) = (16i32, 16i32);

        // Shard, Chip, and Gem are completely still images.
        for &(tier, value) in &SAMPLE_TIERS[..3] {
            let g = sample_gem(value, false);
            let mut pb0 = PixelBuffer::new(33, 33);
            pb0.clear(BG);
            draw_gem(&mut pb0, (cx, cy), &g, 0.0, 17);
            for elapsed in [0.01f32, 0.13, 1.25, 7.89, 42.0] {
                let mut pb = PixelBuffer::new(33, 33);
                pb.clear(BG);
                draw_gem(&mut pb, (cx, cy), &g, elapsed, 17);
                assert_eq!(
                    pb.pixels(),
                    pb0.pixels(),
                    "{tier:?} must be completely still (changed at elapsed={elapsed})"
                );
            }
        }

        // Crystal and Core breathe smoothly with continuous `elapsed`: small
        // time steps never cause stepped channel jumps, while a full cycle
        // produces a visible gentle brightness swing.
        for &(tier, value) in &SAMPLE_TIERS[3..] {
            let g = sample_gem(value, false);
            let mut prev: Option<Vec<Rgb>> = None;
            let mut min_lum = u64::MAX;
            let mut max_lum = 0u64;
            for step in 0..200 {
                let elapsed = step as f32 * 0.02;
                let mut pb = PixelBuffer::new(33, 33);
                pb.clear(BG);
                draw_gem(&mut pb, (cx, cy), &g, elapsed, 42);

                let lum = total_luminance(&pb);
                min_lum = min_lum.min(lum);
                max_lum = max_lum.max(lum);

                if let Some(ref last) = prev {
                    for (&a, &b) in last.iter().zip(pb.pixels().iter()) {
                        let dr = b.0.abs_diff(a.0);
                        let dg = b.1.abs_diff(a.1);
                        let db = b.2.abs_diff(a.2);
                        let max_ch = dr.max(dg).max(db);
                        assert!(
                            max_ch <= 3,
                            "{tier:?} stepped jump of {max_ch} channels at elapsed={elapsed}"
                        );
                    }
                }
                prev = Some(pb.pixels().to_vec());
            }
            assert!(
                max_lum > min_lum,
                "{tier:?} should have a gentle continuous breath across a full period"
            );
        }
    }

    #[test]
    fn handles_non_finite_elapsed_and_out_of_bounds_safely() {
        for &(tier, value) in &SAMPLE_TIERS {
            let g = sample_gem(value, false);
            // Zero-sized buffer must not panic.
            let mut empty = PixelBuffer::new(0, 0);
            draw_gem(&mut empty, (0, 0), &g, 1.0, 3);

            // Far off-screen coordinates must leave the buffer untouched.
            let mut pb = PixelBuffer::new(16, 16);
            pb.clear(BG);
            for pos in [(-20, 8), (40, 8), (8, -20), (8, 40), (i32::MIN, 0), (i32::MAX, 0)] {
                draw_gem(&mut pb, pos, &g, 1.0, 3);
            }
            assert_eq!(count_painted(&pb, BG), 0, "{tier:?} painted from off-screen");

            // Non-finite elapsed must not panic and still paints within bounds.
            for bad_t in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut pb2 = PixelBuffer::new(16, 16);
                pb2.clear(BG);
                draw_gem(&mut pb2, (8, 8), &g, bad_t, 5);
                assert!(count_painted(&pb2, BG) > 0, "{tier:?} failed on elapsed={bad_t}");
            }
        }
    }
}
