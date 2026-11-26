//! Pixel art for Hantavirus rat droppings on the floor.
//!
//! Pure render-only drawing (zero rng, zero heap allocation). Each dropping is
//! a single distinct, solid rounded/oblong rat turd (a fat grain-of-rice /
//! capsule, 3–4 px long with rounded ends) painted dark brown to near-black
//! with a tiny dull upper-left highlight and a 1-px darker underside contact
//! shadow so it reads cleanly on any floor (Sewer brick, Outfall sand, glazed
//! tiles, wet puddles, dark corners) and survives 2× zoom-out.
//!
//! The pellet itself is **never** tinted green, **never** marked emissive, and
//! is stamped with the matte non-creature pen ([`crate::render::lighting::dropping_pen`])
//! so the lighting effects (`surface`, `specular`, `softshadow`, `water`) never
//! glaze or emboss it. Around and underneath the pellet sits a soft green floor
//! halo/ground pool that grows in radius and brightness with `d.level` (ranks
//! 1..=5), blending directly into the floor pixels without overwriting the
//! underlying floor/puddle material byte. Only the strong inner/mid floor halo
//! pixels and the narrow green rim along the top edge are marked emissive via
//! [`mark_dropping_emissive`].
//!
//! Drawing is completely **still** (no time stepping on `elapsed`); visual
//! changes happen only across the three age states matching the weapon and
//! ground-light contracts:
//!
//! 1. **Fresh / arming (`age < 0.25`)**: slightly glossier wet dark-brown
//!    pellet with a dimmer (`0.45×`) green floor halo.
//! 2. **Live / armed (`0.25 <= age <= 12.0`)**: dark brown/near-black pellet
//!    framed by the full rank-scaled green floor halo.
//! 3. **Drying (`age > 12.0`, final 2 s of `DROPPING_LIFE`)**: pellet dries
//!    toward muted dark umber-charcoal, the green floor halo fades smoothly to
//!    zero, and emissive marking ceases.

use crate::game::ecs::components::Dropping;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::{self, LightMap};
use crate::render::palette::{self, Rgb};

/// Arming delay in seconds (contract: `weapon::DROPPING_ARM = 0.25`).
const ARM_AGE: f32 = 0.25;
/// Age in seconds at which a dropping begins drying (last 2 s of 14.0 s life).
const DRYING_AGE: f32 = 12.0;
/// Duration of the drying window in seconds (`14.0 - 12.0`).
const DRYING_SPAN: f32 = 2.0;

/// Bounds of the floor bloom, inside the scene's 11 px dropping cull margin.
const MAX_EXTENT: i32 = 10;
const HALO_OUTER: Rgb = (12, 172, 42);
const HALO_INNER: Rgb = (114, 249, 33);
const RIM_COL: Rgb = (164, 255, 80);

/// Per-rank floor bloom radius (world px from the centre) and peak opacity.
const HALO_REACH: [f32; 5] = [6.0, 7.0, 8.0, 9.0, 10.0];
const HALO_ALPHA: [f32; 5] = [0.82, 0.86, 0.90, 0.94, 0.98];
/// Keep the bright inner glow from dimming with the floor.
const HALO_EMISSIVE_MIN: f32 = 0.40;

/// Deep dark brown-black underside contact shadow colour.
const SHADOW_COL: Rgb = (6, 4, 3);
const SHADOW_UNDER_ALPHA: f32 = 0.82;
const SHADOW_SIDE_ALPHA: f32 = 0.42;

/// Dark umber albedo. The green channel stays low so the pellet reads as
/// brown or near-black even under overlapping ground lights.
const PAL_ARMED: [(Rgb, Rgb, Rgb); 3] = [
    ((58, 7, 8), (36, 4, 4), (20, 2, 2)),
    ((52, 6, 7), (32, 4, 4), (18, 2, 2)),
    ((64, 8, 9), (40, 5, 5), (22, 3, 3)),
];
const HI_ARMED: Rgb = (96, 12, 13);

/// Fresh, wet dark browns.
const PAL_WET: [(Rgb, Rgb, Rgb); 3] = [
    ((66, 8, 9), (42, 5, 6), (24, 3, 3)),
    ((60, 7, 8), (38, 5, 5), (22, 3, 3)),
    ((72, 9, 10), (46, 6, 6), (26, 3, 3)),
];
const HI_WET: Rgb = (112, 14, 16);

/// Drying, muted umber-charcoal.
const PAL_DRY: [(Rgb, Rgb, Rgb); 3] = [
    ((62, 10, 11), (42, 7, 8), (26, 4, 5)),
    ((56, 9, 10), (38, 6, 7), (24, 4, 4)),
    ((68, 11, 12), (46, 7, 8), (28, 4, 5)),
];
const HI_DRY: Rgb = (88, 14, 17);

/// Eight hand-crafted rounded/oblong rat-turd silhouettes (3–4 px long, fat
/// rounded capsule, 7–9 solid pixels each).
///
/// Every coordinate sits inside `dx in -2..=2, dy in -1..=1` and every shape
/// is asymmetric around `dx = 0`, so horizontal flipping (`flip_x`) yields 16
/// distinct solid rat-dropping orientations without ever breaking into loose
/// specks.
const SHAPES: &[&[(i8, i8)]] = &[
    // 0: horizontal plump rat turd (4×3, 8 px)
    &[
        (-1, -1),
        (0, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
    // 1: gently curved oblong capsule (4×3, 8 px)
    &[
        (-1, -1),
        (0, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (0, 1),
        (1, 1),
    ],
    // 2: plump rounded oblong pellet (4×3, 9 px)
    &[
        (-1, -1),
        (0, -1),
        (1, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
    // 3: slightly angled blunt capsule (4×3, 8 px)
    &[
        (-2, -1),
        (-1, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
    // 4: compact 4-px fat grain-of-rice (4×3, 7 px)
    &[
        (-1, -1),
        (0, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
    ],
    // 5: short plump rounded nugget (3×3, 7 px)
    &[
        (-1, -1),
        (0, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
    // 6: slightly tilted oval turd (4×3, 8 px)
    &[
        (0, -1),
        (1, -1),
        (-2, 0),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
    // 7: plump 3×3 angled pellet (3×3, 7 px)
    &[
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
    ],
];

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn mix_seed(seed: u32) -> u32 {
    let mut n = seed ^ 0x9e37_79b9;
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}

/// Bitmask position for local offsets `(dx, dy)` in `-4..=3 × -4..=3`.
#[inline]
fn mask_bit(dx: i32, dy: i32) -> u64 {
    debug_assert!((-4..=3).contains(&dx) && (-4..=3).contains(&dy));
    1u64 << (((dy + 4) * 8 + (dx + 4)) as u32)
}

#[inline]
fn has_bit(mask: u64, dx: i32, dy: i32) -> bool {
    (-4..=3).contains(&dx) && (-4..=3).contains(&dy) && (mask & mask_bit(dx, dy)) != 0
}

/// Blend a colour directly into `pb.pixels_mut()` without altering the pixel's
/// material byte, so floor halo and contact shadow preserve underlying floor
/// and puddle (`MaterialKind::Water`) tags right up to the pellet edge.
#[inline]
fn blend_floor_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let idx = y as usize * pb.w + x as usize;
    let bg = pb.pixels()[idx];
    pb.pixels_mut()[idx] = palette::mix(bg, c, a);
}

/// Shared stack-only pellet and floor-halo geometry for both
/// [`draw_dropping`] and [`mark_dropping_emissive`] so painted floor-halo
/// pixels and emissive marks can never drift apart, and pellet pixels are
/// guaranteed to stay strictly non-emissive.
struct DroppingPlan {
    shape: &'static [(i8, i8)],
    h: u32,
    flip_x: bool,
    tier: usize,
    wet: bool,
    drying: bool,
    occupied: u64,
    shadow_under: u64,
    shadow_side: u64,
    tl: (i32, i32),
    br: (i32, i32),
    halo_extent: i32,
    halo_inv_radius_sq: f32,
    halo_peak: f32,
}

impl DroppingPlan {
    fn build(d: &Dropping) -> Self {
        let h = mix_seed(d.seed);
        let shape_idx = (d.seed as usize) % SHAPES.len();
        let flip_x = ((d.seed >> 3) & 1) != 0;
        let shape = SHAPES[shape_idx];
        let tier = (d.level.clamp(1, 5) - 1) as usize;

        let wet = d.age < ARM_AGE;
        let drying = d.age > DRYING_AGE;
        let glow_scale = if wet {
            0.45
        } else if drying {
            let dry_t = ((d.age - DRYING_AGE) / DRYING_SPAN).clamp(0.0, 1.0);
            let rem = 1.0 - dry_t;
            rem * rem * 0.50
        } else {
            1.0
        };

        let mut occupied = 0u64;
        let mut tl = (i32::MAX, i32::MAX);
        let mut br = (i32::MIN, i32::MIN);
        let mut br_key = i32::MIN;
        for &(rx, ry) in shape {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);
            occupied |= mask_bit(dx, dy);
            if (dy, dx) < (tl.1, tl.0) {
                tl = (dx, dy);
            }
            let k = dx + dy;
            if (k, dx) > (br_key, br.0) {
                br_key = k;
                br = (dx, dy);
            }
        }

        // 1-px darker underside / contact shadow immediately beneath the
        // pellet's bottom contour and off its lower-right flank.
        let mut shadow_under = 0u64;
        let mut shadow_side = 0u64;
        for &(rx, ry) in shape {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);
            let ub = mask_bit(dx, dy + 1);
            if occupied & ub == 0 {
                shadow_under |= ub;
            }
            if dy >= 0 {
                let sb = mask_bit(dx + 1, dy);
                if occupied & sb == 0 {
                    shadow_side |= sb;
                }
            }
        }
        shadow_side &= !shadow_under;

        let radius = HALO_REACH[tier];

        Self {
            shape,
            h,
            flip_x,
            tier,
            wet,
            drying,
            occupied,
            shadow_under,
            shadow_side,
            tl,
            br,
            halo_extent: radius as i32,
            halo_inv_radius_sq: 1.0 / (radius * radius),
            halo_peak: HALO_ALPHA[tier] * glow_scale,
        }
    }

    /// A squared falloff around the pellet, flattened vertically onto the floor.
    #[inline]
    fn halo_sample(&self, dx: i32, dy: i32) -> Option<(f32, f32)> {
        let d2 = (dx * dx) as f32 + (dy * dy) as f32 * 1.32;
        let t = 1.0 - d2 * self.halo_inv_radius_sq;
        if t <= 0.0 || has_bit(self.occupied, dx, dy) {
            return None;
        }
        let fall = t * t;
        let a = self.halo_peak * fall;
        (a > 0.02).then_some((fall, a))
    }

    fn upper_rim(&self) -> impl Iterator<Item = (i32, i32)> {
        let y = self.tl.1;
        let occupied = self.occupied;
        (-2..=2)
            .filter(move |&x| has_bit(occupied, x, y))
            .map(move |x| (x, y - 1))
    }
}

/// Draw one Hantavirus dropping on the floor at `d.pos`.
///
/// Layer order:
/// 1. Soft rank-scaled green floor halo/ground pool around the pellet
///    (preserves underlying floor/puddle material bytes).
/// 2. 1-px darker underside / contact shadow right beneath the pellet.
/// 3. Solid dark-brown/near-black rounded rat turd with a tiny dull highlight
///    on its upper-left crown, stamped with the matte non-creature pen
///    ([`lighting::dropping_pen`]).
///
/// The floor bloom and rim stay within `MAX_EXTENT` of the dropping's centre.
/// Only the dropping's age and rank change the glow.
pub fn draw_dropping(pb: &mut PixelBuffer, d: &Dropping, cam: Vec2, _elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let (sx, sy) = w2s(d.pos, cam);
    if sx.saturating_add(MAX_EXTENT) < 0
        || sy.saturating_add(MAX_EXTENT) < 0
        || sx.saturating_sub(MAX_EXTENT) >= pb.w as i32
        || sy.saturating_sub(MAX_EXTENT) >= pb.h as i32
    {
        return;
    }

    let plan = DroppingPlan::build(d);
    let rank_t = plan.tier as f32 / 4.0;

    // 1. Soft green floor halo / ground pool radiating from under/around the
    //    pellet onto the floor. No outer darkening ring: grounding scales with
    //    `a`, so the outer edge fades cleanly into the floor.
    if plan.halo_peak > 0.03 {
        let ir = plan.halo_extent;
        for dy in -ir..=ir {
            for dx in -ir..=ir {
                let Some((fall, a)) = plan.halo_sample(dx, dy) else {
                    continue;
                };
                let px = sx + dx;
                let py = sy + dy;
                if px < 0 || py < 0 || px as usize >= pb.w || py as usize >= pb.h {
                    continue;
                }
                let idx = py as usize * pb.w + px as usize;
                let bg = pb.pixels()[idx];
                let grounded = palette::scale(bg, 1.0 - 0.35 * a);
                let g_col = palette::mix(
                    HALO_OUTER,
                    HALO_INNER,
                    (fall * 0.75 + rank_t * 0.35).clamp(0.0, 1.0),
                );
                pb.pixels_mut()[idx] = palette::mix(grounded, g_col, a);
            }
        }
    }

    // Trace the top contour onto the floor side of the silhouette. The
    // bright rim leaves every brown pellet pixel intact.
    for (dx, dy) in plan.upper_rim() {
        blend_floor_px(pb, sx + dx, sy + dy, RIM_COL, plan.halo_peak);
    }

    // 2. 1-px darker underside / contact shadow right beneath the pellet so it
    //    reads as a solid object sitting on top of the floor glow.
    for dy in -1..=2i32 {
        for dx in -2..=3i32 {
            let b = mask_bit(dx, dy);
            if plan.shadow_under & b != 0 {
                blend_floor_px(pb, sx + dx, sy + dy, SHADOW_COL, SHADOW_UNDER_ALPHA);
            } else if plan.shadow_side & b != 0 {
                blend_floor_px(pb, sx + dx, sy + dy, SHADOW_COL, SHADOW_SIDE_ALPHA);
            }
        }
    }

    // 3. Pellet body: solid dark-brown/near-black rounded rat turd stamped with
    //    the matte non-creature pen so lighting effects never glaze it green.
    let tone_idx = ((plan.h >> 2) as usize) % PAL_ARMED.len();
    let (mid, shade, deep, hi) = if plan.wet {
        let (m, s, dp) = PAL_WET[tone_idx];
        (m, s, dp, HI_WET)
    } else if plan.drying {
        let dry_t = ((d.age - DRYING_AGE) / DRYING_SPAN).clamp(0.0, 1.0);
        let k = 0.55 + 0.45 * dry_t;
        let (am, as_, ad) = PAL_ARMED[tone_idx];
        let (dm, ds_, dd) = PAL_DRY[tone_idx];
        (
            palette::mix(am, dm, k),
            palette::mix(as_, ds_, k),
            palette::mix(ad, dd, k),
            palette::mix(HI_ARMED, HI_DRY, k),
        )
    } else {
        let (m, s, dp) = PAL_ARMED[tone_idx];
        (m, s, dp, HI_ARMED)
    };

    let prev_pen = pb.set_pen(lighting::dropping_pen());
    for &(rx, ry) in plan.shape {
        let dx = if plan.flip_x {
            -i32::from(rx)
        } else {
            i32::from(rx)
        };
        let dy = i32::from(ry);
        let col = if (dx, dy) == plan.tl {
            hi
        } else if (dx, dy) == plan.br || (dy > 0 && dx >= 0) {
            deep
        } else if dy > 0 || dx > 0 {
            shade
        } else {
            mid
        };
        pb.plot(sx + dx, sy + dy, col);
    }
    pb.set_pen(prev_pen);
}

/// Mark the strong floor-halo pixels surrounding one dropping as emissive on
/// `lm` so the green under-glow stays bright under the lighting pass at every
/// rank (1..=5).
///
/// The pellet itself and its underside contact shadow are **never** marked
/// emissive so the rat turd stays dark brown/black rather than washing green.
/// Drying droppings (`age > 12.0`) are skipped so they dim naturally alongside
/// their fading ground light.
pub fn mark_dropping_emissive(lm: &mut LightMap, pb: &PixelBuffer, d: &Dropping, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 || d.age > DRYING_AGE {
        return;
    }
    let (sx, sy) = w2s(d.pos, cam);
    if sx.saturating_add(MAX_EXTENT) < 0
        || sy.saturating_add(MAX_EXTENT) < 0
        || sx.saturating_sub(MAX_EXTENT) >= pb.w as i32
        || sy.saturating_sub(MAX_EXTENT) >= pb.h as i32
    {
        return;
    }

    let plan = DroppingPlan::build(d);
    for (dx, dy) in plan.upper_rim() {
        lm.mark_emissive_rect(pb, sx + dx, sy + dy, 1, 1);
    }
    if plan.halo_peak < HALO_EMISSIVE_MIN {
        return;
    }

    let ir = plan.halo_extent;
    for dy in -ir..=ir {
        for dx in -ir..=ir {
            if has_bit(plan.shadow_under | plan.shadow_side, dx, dy) {
                continue;
            }
            if let Some((_, a)) = plan.halo_sample(dx, dy) {
                if a >= HALO_EMISSIVE_MIN {
                    lm.mark_emissive_rect(pb, sx + dx, sy + dy, 1, 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ecs::components::SpawnId;
    use crate::game::level::{Level, Theme};
    use crate::game::world::World;
    use crate::render::lighting::{LightScene, LightingMode};
    use crate::term::ColorDepth;

    fn sample_dropping(pos: Vec2, age: f32, seed: u32, level: u8) -> Dropping {
        Dropping {
            pos,
            age,
            damage: 3.0,
            jumps: 1,
            reach: 14.0,
            radius: 7.0,
            seed,
            level,
        }
    }

    #[inline]
    fn luma((r, g, b): Rgb) -> u8 {
        ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8) as u8
    }

    /// Sum of green dominance `g.saturating_sub(r.max(b))` over painted floor
    /// halo pixels inside the dropping's bounding box centered at `(sx, sy)`.
    fn green_excess_in_box(pb: &PixelBuffer, sx: usize, sy: usize, bg: Rgb) -> u32 {
        let mut total = 0u32;
        let extent = MAX_EXTENT as usize;
        for y in (sy - extent)..=(sy + extent) {
            for x in (sx - extent)..=(sx + extent) {
                let c = pb.pixel_at(x, y);
                if c != bg {
                    total += u32::from(c.1.saturating_sub(c.0.max(c.2)));
                }
            }
        }
        total
    }

    /// Maximum Chebyshev radius from `(sx, sy)` of any green-dominant floor
    /// halo pixel (`g > r.max(b)`).
    fn green_halo_radius(pb: &PixelBuffer, sx: usize, sy: usize, bg: Rgb) -> usize {
        let mut max_r = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                if c.1 > bg.1 && c.1 > c.0.max(c.2) {
                    let r = x.abs_diff(sx).max(y.abs_diff(sy));
                    max_r = max_r.max(r);
                }
            }
        }
        max_r
    }

    #[test]
    fn draws_inside_halo_box_and_nothing_outside() {
        const BG: Rgb = (28, 34, 32);
        let cam = Vec2::new(10.0, 5.0);
        let pos = Vec2::new(34.0, 29.0); // screen (24, 24)
        let (sx, sy) = (24i32, 24i32);

        for seed in 0..16u32 {
            for level in 1..=5u8 {
                for &age in &[0.05f32, 0.24, 0.25, 3.5, 12.0, 12.1, 13.8] {
                    let mut pb = PixelBuffer::new(49, 49);
                    pb.clear(BG);
                    let d = sample_dropping(pos, age, seed, level);
                    draw_dropping(&mut pb, &d, cam, 0.0);

                    let mut inside = 0usize;
                    for y in 0..pb.h {
                        for x in 0..pb.w {
                            let changed = pb.pixel_at(x, y) != BG;
                            let in_box = (x as i32 - sx).abs() <= MAX_EXTENT
                                && (y as i32 - sy).abs() <= MAX_EXTENT;
                            if in_box {
                                if changed {
                                    inside += 1;
                                }
                            } else {
                                assert!(
                                    !changed,
                                    "seed={seed} level={level} age={age} painted outside halo box at ({x},{y})"
                                );
                            }
                        }
                    }
                    assert!(
                        inside >= 7,
                        "seed={seed} level={level} age={age} drew too few pixels ({inside})"
                    );
                }
            }
        }
    }

    #[test]
    fn pellet_pixels_are_dark_and_not_green_at_every_rank_and_state() {
        const BG: Rgb = (28, 36, 34);
        let pos = Vec2::new(24.0, 24.0);

        // 1. Raw albedo: every pellet pixel across every shape, rank, and age
        //    state must be dark (luma < 90) and warm brown/black (r >= g).
        for seed in 0..16u32 {
            for level in 1..=5u8 {
                for &age in &[0.05f32, 0.24, 0.25, 3.5, 12.0, 12.5, 13.8] {
                    let d = sample_dropping(pos, age, seed, level);
                    let plan = DroppingPlan::build(&d);
                    let mut pb = PixelBuffer::new(49, 49);
                    pb.clear(BG);
                    draw_dropping(&mut pb, &d, Vec2::ZERO, 0.0);

                    for &(rx, ry) in plan.shape {
                        let dx = if plan.flip_x {
                            -i32::from(rx)
                        } else {
                            i32::from(rx)
                        };
                        let dy = i32::from(ry);
                        let c = pb.pixel_at((24 + dx) as usize, (24 + dy) as usize);
                        assert!(
                            luma(c) < 90,
                            "seed={seed} level={level} age={age} pellet px ({dx},{dy}) too bright: {c:?}"
                        );
                        assert!(
                            c.0 >= c.1,
                            "seed={seed} level={level} age={age} pellet px ({dx},{dy}) tinted green: {c:?}"
                        );
                    }
                }
            }
        }

        // 2. Lit scene (Full lighting + fx on, within FOG_NEAR so distance fog
        //    is zero and the dropping's own green ground light is active):
        //    every pellet pixel stays dark and not green.
        for level in [1u8, 3, 5] {
            for &age in &[0.1f32, 3.0, 13.5] {
                let mut w = World::new(Theme::Sewer, 80);
                w.level = Level::empty(Theme::Sewer);
                w.player.pos = Vec2::new(68.0, 32.0);
                let d = sample_dropping(Vec2::new(32.0, 32.0), age, 0, level);
                w.ecs.spawn((d, SpawnId(1)));
                let plan = DroppingPlan::build(&d);

                let mut pb = PixelBuffer::new(64, 64);
                pb.set_materials(true);
                pb.clear((18, 28, 26));
                draw_dropping(&mut pb, &d, Vec2::ZERO, 0.0);
                let mut lm = LightMap::new();
                lm.begin_frame();
                mark_dropping_emissive(&mut lm, &pb, &d, Vec2::ZERO);
                let scene = LightScene {
                    ecs: &w.ecs,
                    level: &w.level,
                    player: &w.player,
                    particles: &w.particles,
                    thrash: None,
                    squeak: None,
                    cam: Vec2::ZERO,
                    elapsed: 0.0,
                    mode: LightingMode::Full,
                    depth: ColorDepth::Truecolor,
                    fx: true,
                    gi: crate::render::lighting::GlobalLight::Off,
                    blackout: 0.0,
                };
                lm.apply_scene(&mut pb, &scene);

                let mut sum_luma = 0u32;
                let mut count = 0u32;
                for &(rx, ry) in plan.shape {
                    let dx = if plan.flip_x {
                        -i32::from(rx)
                    } else {
                        i32::from(rx)
                    };
                    let dy = i32::from(ry);
                    let c = pb.pixel_at((32 + dx) as usize, (32 + dy) as usize);
                    let lum = luma(c);
                    sum_luma += u32::from(lum);
                    count += 1;
                    assert!(
                        lum < 110,
                        "lit level={level} age={age} pellet px ({dx},{dy}) too bright: {c:?}"
                    );
                    if age <= DRYING_AGE {
                        assert!(
                            c.0 >= c.1,
                            "lit level={level} age={age} pellet px ({dx},{dy}) washed green: {c:?}"
                        );
                    }
                }
                assert!(
                    sum_luma / count < 55,
                    "lit level={level} age={age} pellet mean luma too bright ({})",
                    sum_luma / count
                );
            }
        }
    }

    #[test]
    fn pellet_uses_matte_pen_and_preserves_floor_material_under_halo() {
        let water_pen = lighting::pool_pen();
        let matte_pen = lighting::dropping_pen();
        assert_ne!(water_pen, matte_pen);

        let d = sample_dropping(Vec2::new(24.0, 24.0), 3.0, 0, 5);
        let plan = DroppingPlan::build(&d);

        let mut pb = PixelBuffer::new(49, 49);
        pb.set_materials(true);
        pb.set_pen(water_pen);
        pb.clear((28, 72, 82));
        draw_dropping(&mut pb, &d, Vec2::ZERO, 0.0);

        // Every pellet pixel is stamped with the matte pen.
        for &(rx, ry) in plan.shape {
            let dx = if plan.flip_x {
                -i32::from(rx)
            } else {
                i32::from(rx)
            };
            let dy = i32::from(ry);
            let idx = (24 + dy) as usize * pb.w + (24 + dx) as usize;
            assert_eq!(
                pb.mat()[idx],
                matte_pen,
                "pellet pixel ({dx},{dy}) must wear the matte pen"
            );
        }

        // Surrounding halo pixel keeps the puddle's Water material byte.
        let halo_idx = (24 - 3) * pb.w + 24;
        assert_eq!(
            pb.mat()[halo_idx],
            water_pen,
            "floor halo must preserve underlying puddle material byte"
        );
        // Caller's pen is restored after draw_dropping returns.
        assert_eq!(pb.pen(), water_pen);
    }

    #[test]
    fn different_seeds_differ() {
        const BG: Rgb = (24, 28, 26);
        let pos = Vec2::new(20.0, 20.0);
        for level in [1u8, 5] {
            let render = |seed: u32| -> Vec<Rgb> {
                let mut pb = PixelBuffer::new(41, 41);
                pb.clear(BG);
                let d = sample_dropping(pos, 3.0, seed, level);
                draw_dropping(&mut pb, &d, Vec2::ZERO, 0.0);
                pb.pixels().to_vec()
            };

            let frames: Vec<Vec<Rgb>> = (0..16u32).map(render).collect();
            for i in 0..frames.len() {
                for j in (i + 1)..frames.len() {
                    assert_ne!(
                        frames[i], frames[j],
                        "level={level} seeds {i} and {j} produced identical output"
                    );
                }
            }
        }
    }

    #[test]
    fn halo_grows_with_rank_and_exceeds_old_rim() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(20.0, 20.0);

        for seed in 0..8u32 {
            let mut prev_green = 0u32;
            let mut r1_radius = 0usize;
            let mut r5_radius = 0usize;

            for level in 1..=5u8 {
                let mut pb = PixelBuffer::new(41, 41);
                pb.clear(BG);
                draw_dropping(&mut pb, &sample_dropping(pos, 3.0, seed, level), Vec2::ZERO, 0.0);

                let g = green_excess_in_box(&pb, 20, 20, BG);
                let rad = green_halo_radius(&pb, 20, 20, BG);
                assert!(
                    g > prev_green,
                    "seed={seed}: rank {level} green halo ({g}) should exceed rank {} ({prev_green})",
                    level - 1
                );
                prev_green = g;
                if level == 1 {
                    r1_radius = rad;
                } else if level == 5 {
                    r5_radius = rad;
                }
            }

            assert!(
                r1_radius >= 3,
                "seed={seed}: rank 1 floor halo radius ({r1_radius}) should reach at least 3 px"
            );
            assert!(
                r5_radius > r1_radius && r5_radius >= 6,
                "seed={seed}: rank 5 floor halo radius ({r5_radius}) must exceed rank 1 ({r1_radius}) and reach >= 6 px"
            );
        }
    }

    #[test]
    fn arming_is_dimmer_than_armed_and_drying_fades() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(20.0, 20.0);

        let mut arming_pb = PixelBuffer::new(41, 41);
        arming_pb.clear(BG);
        draw_dropping(&mut arming_pb, &sample_dropping(pos, 0.1, 3, 3), Vec2::ZERO, 0.5);

        let mut armed_pb = PixelBuffer::new(41, 41);
        armed_pb.clear(BG);
        draw_dropping(&mut armed_pb, &sample_dropping(pos, 3.0, 3, 3), Vec2::ZERO, 0.5);

        let mut dry_pb = PixelBuffer::new(41, 41);
        dry_pb.clear(BG);
        draw_dropping(&mut dry_pb, &sample_dropping(pos, 13.5, 3, 3), Vec2::ZERO, 0.5);

        let g_arming = green_excess_in_box(&arming_pb, 20, 20, BG);
        let g_armed = green_excess_in_box(&armed_pb, 20, 20, BG);
        let g_dry = green_excess_in_box(&dry_pb, 20, 20, BG);

        assert!(
            g_armed > g_arming && g_arming > 0,
            "arming ({g_arming}) must be dimmer than armed ({g_armed})"
        );
        assert!(
            g_armed > g_dry * 4,
            "drying ({g_dry}) must fade its green halo sharply from armed ({g_armed})"
        );
    }

    #[test]
    fn frozen_frame_is_identical_across_elapsed() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(20.0, 20.0);

        for &(age, level) in &[(0.1f32, 2u8), (4.0, 1), (4.0, 5), (13.0, 4)] {
            let d = sample_dropping(pos, age, 5, level);
            let render_at = |elapsed: f32| -> Vec<Rgb> {
                let mut pb = PixelBuffer::new(41, 41);
                pb.clear(BG);
                draw_dropping(&mut pb, &d, Vec2::ZERO, elapsed);
                pb.pixels().to_vec()
            };

            let base = render_at(0.0);
            for &elapsed in &[0.12f32, 0.25, 0.50, 1.25, 7.77, 42.0] {
                assert_eq!(
                    base,
                    render_at(elapsed),
                    "age={age} level={level}: elapsed={elapsed} must paint identically"
                );
            }
        }
    }

    #[test]
    fn mark_dropping_emissive_preserves_floor_halo_not_pellet_and_skips_drying() {
        const FLOOR: Rgb = (14, 22, 20);
        let mut w = World::new(Theme::Sewer, 80);
        w.level = Level::empty(Theme::Sewer);
        w.player.pos = Vec2::new(700.0, 700.0);
        let scene = LightScene {
            ecs: &w.ecs,
            level: &w.level,
            player: &w.player,
            particles: &w.particles,
            thrash: None,
            squeak: None,
            cam: Vec2::ZERO,
            elapsed: 0.0,
            mode: LightingMode::Full,
            depth: ColorDepth::Truecolor,
            fx: false,
            gi: crate::render::lighting::GlobalLight::Off,
            blackout: 0.0,
        };

        let d_r1 = sample_dropping(Vec2::new(24.0, 24.0), 3.0, 0, 1);
        let plan = DroppingPlan::build(&d_r1);
        // Floor halo pixel just above the pellet crown (dx = 0, dy = -2).
        let (hx, hy) = (24usize, 22usize);
        // Pellet crown pixel (must NOT be marked emissive).
        let (px, py) = ((24 + plan.tl.0) as usize, (24 + plan.tl.1) as usize);

        let mut pb_marked = PixelBuffer::new(48, 48);
        pb_marked.clear(FLOOR);
        draw_dropping(&mut pb_marked, &d_r1, Vec2::ZERO, 0.0);
        let halo_unlit = pb_marked.pixel_at(hx, hy);

        let mut lm_marked = LightMap::new();
        lm_marked.begin_frame();
        mark_dropping_emissive(&mut lm_marked, &pb_marked, &d_r1, Vec2::ZERO);
        lm_marked.apply_scene(&mut pb_marked, &scene);

        let mut pb_unmarked = PixelBuffer::new(48, 48);
        pb_unmarked.clear(FLOOR);
        draw_dropping(&mut pb_unmarked, &d_r1, Vec2::ZERO, 0.0);
        let mut lm_unmarked = LightMap::new();
        lm_unmarked.begin_frame();
        lm_unmarked.apply_scene(&mut pb_unmarked, &scene);

        assert!(
            pb_marked.pixel_at(hx, hy).1 >= halo_unlit.1,
            "emissive mark must keep floor halo green bright after lighting"
        );
        assert!(
            pb_marked.pixel_at(hx, hy).1 > pb_unmarked.pixel_at(hx, hy).1 + 25,
            "emissive floor halo must stay visibly brighter than an unmarked one"
        );
        assert_eq!(
            pb_marked.pixel_at(px, py),
            pb_unmarked.pixel_at(px, py),
            "pellet pixel itself must never be marked emissive"
        );

        // Drying dropping: `mark_dropping_emissive` is a no-op so it dims with the floor.
        let d_dry = sample_dropping(Vec2::new(24.0, 24.0), 13.5, 0, 5);
        let mut pb_dry_marked = PixelBuffer::new(48, 48);
        pb_dry_marked.clear(FLOOR);
        draw_dropping(&mut pb_dry_marked, &d_dry, Vec2::ZERO, 0.0);
        let mut lm_dry_marked = LightMap::new();
        lm_dry_marked.begin_frame();
        mark_dropping_emissive(&mut lm_dry_marked, &pb_dry_marked, &d_dry, Vec2::ZERO);
        lm_dry_marked.apply_scene(&mut pb_dry_marked, &scene);

        let mut pb_dry_unmarked = PixelBuffer::new(48, 48);
        pb_dry_unmarked.clear(FLOOR);
        draw_dropping(&mut pb_dry_unmarked, &d_dry, Vec2::ZERO, 0.0);
        let mut lm_dry_unmarked = LightMap::new();
        lm_dry_unmarked.begin_frame();
        lm_dry_unmarked.apply_scene(&mut pb_dry_unmarked, &scene);

        assert_eq!(
            pb_dry_marked.pixels(),
            pb_dry_unmarked.pixels(),
            "drying dropping must not be marked emissive"
        );
    }

    #[test]
    fn no_panic_at_buffer_edges_or_empty_buffer() {
        let mut pb = PixelBuffer::new(8, 8);
        let mut lm = LightMap::new();
        for seed in 0..8u32 {
            for level in [0u8, 1, 5, 99] {
                for &pos in &[
                    Vec2::new(0.0, 0.0),
                    Vec2::new(7.0, 7.0),
                    Vec2::new(0.0, 7.0),
                    Vec2::new(7.0, 0.0),
                    Vec2::new(-1.0, 4.0),
                    Vec2::new(4.0, -1.0),
                    Vec2::new(8.0, 4.0),
                    Vec2::new(4.0, 8.0),
                    Vec2::new(-40.0, -40.0),
                    Vec2::new(200.0, 200.0),
                ] {
                    let d = sample_dropping(pos, 3.0, seed, level);
                    lm.begin_frame();
                    draw_dropping(&mut pb, &d, Vec2::ZERO, 0.5);
                    mark_dropping_emissive(&mut lm, &pb, &d, Vec2::ZERO);
                    draw_dropping(&mut pb, &d, Vec2::ZERO, f32::NAN);
                }
            }
        }

        let mut zero = PixelBuffer::new(0, 0);
        let d = sample_dropping(Vec2::ZERO, 3.0, 0, 1);
        draw_dropping(&mut zero, &d, Vec2::ZERO, 0.0);
        mark_dropping_emissive(&mut lm, &zero, &d, Vec2::ZERO);
    }
}
