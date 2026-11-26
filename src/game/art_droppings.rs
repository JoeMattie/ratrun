//! Pixel art for Hantavirus rat droppings on the floor.
//!
//! Pure render-only drawing (zero rng, zero heap allocation). Each dropping is
//! a small, deadpan cluster of 2–4 dark brown rice-grain pellets (2×1, 3×1,
//! 1×2, or angled 2–3 px grains) within a ~5×4 px footprint — clearly smaller
//! and looser than a floor gold coin — wrapped in a radioactive sickly-green
//! glow whose intensity scales with `d.level` (rank 1 faint, rank 5 vivid):
//! a 1-px green rim/halo around the cluster, 1–2 bright green core pixels
//! embedded on the pellets, and 1–2 still green wisps floating above.
//!
//! All glow pixels are marked emissive via [`mark_dropping_emissive`] so they
//! stay bright under the lighting pass across all six floors (Sewer, Kitchen,
//! Lab, Laundry, Printworks, Flatbed). Drawing is completely **still** (no time
//! stepping on `elapsed`); visual changes happen only across the three age
//! states matching the weapon and light contracts:
//!
//! 1. **Fresh / unarmed (`age < 0.25`)**: slightly brighter, glossier wet
//!    pellets with half-strength green glow and no floating wisps above.
//! 2. **Live / armed (`0.25 <= age <= 12.0`)**: rich dark brown pellets with
//!    full rank-scaled green core(s), 1-px green rim/halo, and 1–2 still green
//!    wisps 2–3 px above the cluster.
//! 3. **Drying (`age > 12.0`, final 2 s of `DROPPING_LIFE`)**: pellets lighten
//!    toward chalky grey-brown, the green glow fades smoothly to zero, the
//!    floating wisps stop, and emissive marking ceases.

use crate::game::ecs::components::Dropping;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::LightMap;
use crate::render::palette::{self, Rgb};

/// Arming delay in seconds (contract: `weapon::DROPPING_ARM = 0.25`).
const ARM_AGE: f32 = 0.25;
/// Age in seconds at which a dropping begins drying (last 2 s of 14.0 s life).
const DRYING_AGE: f32 = 12.0;
/// Duration of the drying window in seconds (`14.0 - 12.0`).
const DRYING_SPAN: f32 = 2.0;

/// Sickly radioactive green palette matching the dropping point light
/// (`(110, 255, 120)`), ramped from eerie dim green at rank 1 to vivid lime-white
/// core and crisp green rim at rank 5.
const GLOW_RIM_LOW: Rgb = (52, 178, 66);
const GLOW_RIM_HIGH: Rgb = (88, 242, 98);
const GLOW_CORE_LOW: Rgb = (118, 226, 114);
const GLOW_CORE_HIGH: Rgb = (218, 255, 188);
const GLOW_CORE2: Rgb = (128, 255, 126);
const GLOW_WISP: Rgb = (104, 248, 116);
/// Deep neutral-warm dark keyline blended along the lower-right of each pellet.
const KEYLINE_DARK: Rgb = (8, 5, 5);

/// Per-rank (L1 → L5) intensity curves keeping the brightness hierarchy intact:
/// hot green core > warm dark brown pellets > translucent 1-px green rim.
const RIM_ALPHA: [f32; 5] = [0.16, 0.24, 0.34, 0.44, 0.54];
const CORE1_MIX: [f32; 5] = [0.58, 0.72, 0.84, 0.94, 1.00];
const CORE2_MIX: [f32; 5] = [0.00, 0.28, 0.48, 0.70, 0.88];
const WISP_ALPHA: [f32; 5] = [0.22, 0.32, 0.42, 0.52, 0.62];

/// Armed pellet palette: `(body, shaded_end)` pairs of warm dark browns.
/// Warm enough to still read unmistakably brown next to the green glow and
/// under the dropping's green point light, yet darker than Flatbed oak planks.
const PAL_ARMED: [(Rgb, Rgb); 3] = [
    ((108, 66, 38), (74, 44, 25)),
    ((94, 58, 34), (64, 38, 22)),
    ((116, 74, 44), (80, 50, 29)),
];
const HI_ARMED: Rgb = (164, 122, 84);

/// Fresh / unarmed (`age < 0.25`): slightly brighter, wet-sheen browns.
const PAL_WET: [(Rgb, Rgb); 3] = [
    ((124, 80, 48), (88, 54, 32)),
    ((108, 70, 42), (76, 46, 28)),
    ((132, 88, 54), (92, 58, 36)),
];
const HI_WET: Rgb = (206, 170, 134);

/// Drying (`age > 12.0`): lighter, desaturated chalky grey-browns.
const PAL_DRY: [(Rgb, Rgb); 3] = [
    ((124, 112, 100), (88, 79, 70)),
    ((112, 102, 92), (78, 72, 65)),
    ((132, 120, 108), (94, 85, 76)),
];
const HI_DRY: Rgb = (168, 158, 146);

/// Eight hand-crafted rice-grain cluster layouts (2–4 pellets each).
///
/// Every coordinate sits inside `dx in -2..=2, dy in -1..=2` (at most 5×4 px),
/// with clean 1-px floor gaps between pellets so clusters never collapse into a
/// solid coin-sized disc. Combined with horizontal mirroring and brown-tone
/// permutation from `d.seed`, these yield varied deadpan scatters while staying
/// strictly inside a 9×9 box (`-4..=4` in both axes) including keyline, 1-px
/// green rim and still green wisps.
const LAYOUTS: &[&[&[(i8, i8)]]] = &[
    // 0: 3 pellets — loose trio (3×1 upper-left, 2×1 mid-right, 2-px slant lower-left)
    &[
        &[(-2, -1), (-1, -1), (0, -1)],
        &[(1, 1), (2, 1)],
        &[(-2, 1), (-1, 2)],
    ],
    // 1: 2 pellets — sparse angled pair (3-px slant + 2×1)
    &[
        &[(-2, 0), (-1, -1), (0, -1)],
        &[(0, 1), (1, 1)],
    ],
    // 2: 4 pellets — airy 4-grain scatter
    &[
        &[(-2, -1), (-1, -1)],
        &[(1, -1), (2, -1)],
        &[(-2, 1), (-1, 1)],
        &[(1, 2), (2, 1)],
    ],
    // 3: 3 pellets — angled fan (2-px slant, 3×1 mid, 2×1 lower)
    &[
        &[(-2, 0), (-1, -1)],
        &[(0, 0), (1, 0), (2, 0)],
        &[(-1, 2), (0, 2)],
    ],
    // 4: 3 pellets — 1×2 vertical + 2×1 upper-right + 3-px gentle bend lower
    &[
        &[(-2, -1), (-2, 0)],
        &[(0, -1), (1, -1)],
        &[(-1, 2), (0, 1), (1, 1)],
    ],
    // 5: 2 pellets — compact asymmetric pair (3×1 + 2-px slant)
    &[
        &[(-1, -1), (0, -1), (1, -1)],
        &[(-2, 1), (-1, 2)],
    ],
    // 6: 4 pellets — staggered 4-grain cluster (4-disconnected 2×1 grains)
    &[
        &[(0, -1), (1, -1)],
        &[(-2, 0), (-1, 0)],
        &[(1, 1), (2, 1)],
        &[(-1, 2), (0, 2)],
    ],
    // 7: 3 pellets — oblique trio (2×1 top, 1×2 right, 3-px curved bottom-left)
    &[
        &[(-1, -1), (0, -1)],
        &[(2, 0), (2, 1)],
        &[(-2, 1), (-1, 1), (0, 2)],
    ],
];

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::mix(bg, c, a));
}

/// Blend a green glow pixel over a half-grounded background sample so emissive
/// restoration preserves a pure green hue on cool/warm floors (e.g. Laundry
/// indigo tiles) without freezing bright unlit floor blue into the halo.
#[inline]
fn blend_glow(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = palette::scale(pb.pixel_at(x as usize, y as usize), 0.50);
    pb.plot(x, y, palette::mix(bg, c, a));
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

/// Shared stack-only layout and glow geometry computed once for both
/// [`draw_dropping`] and [`mark_dropping_emissive`] so painted glow pixels and
/// emissive marks can never drift apart.
struct DroppingPlan {
    layout: &'static [&'static [(i8, i8)]],
    h: u32,
    flip_x: bool,
    tier: usize,
    wet: bool,
    drying: bool,
    glow_scale: f32,
    occupied: u64,
    rim_mask: u64,
    rim_inner: u64,
    hi_pellet: usize,
    second_pellet: usize,
    core1_pos: (i32, i32),
    core2_pos: Option<(i32, i32)>,
    wisp1_pos: Option<(i32, i32)>,
    wisp2_pos: Option<(i32, i32)>,
}

impl DroppingPlan {
    fn build(d: &Dropping) -> Self {
        let h = mix_seed(d.seed);
        let layout_idx = (d.seed.wrapping_add(d.seed >> 4) as usize) % LAYOUTS.len();
        let flip_x = (((d.seed >> 3) ^ (h >> 5)) & 1) != 0;
        let layout = LAYOUTS[layout_idx];
        let tier = (d.level.clamp(1, 5) - 1) as usize;

        let wet = d.age < ARM_AGE;
        let drying = d.age > DRYING_AGE;
        let glow_scale = if wet {
            0.50
        } else if drying {
            let dry_t = ((d.age - DRYING_AGE) / DRYING_SPAN).clamp(0.0, 1.0);
            let rem = 1.0 - dry_t;
            rem * rem * 0.50
        } else {
            1.0
        };

        let mut occupied = 0u64;
        let mut top_y = 2i32;
        for pellet in layout {
            for &(rx, ry) in *pellet {
                let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
                let dy = i32::from(ry);
                if dy < top_y {
                    top_y = dy;
                }
                occupied |= mask_bit(dx, dy);
            }
        }

        // 1-px 4-connected rim around the cluster; `rim_inner` tracks tight
        // interior gaps touching >= 2 pellet pixels so grains stay separated.
        let mut rim_mask = 0u64;
        let mut rim_inner = 0u64;
        if RIM_ALPHA[tier] * glow_scale > 0.03 {
            for pellet in layout {
                for &(rx, ry) in *pellet {
                    let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
                    let dy = i32::from(ry);
                    for &(ox, oy) in &[(0, -1), (0, 1), (-1, 0), (1, 0)] {
                        let (nx, ny) = (dx + ox, dy + oy);
                        let b = mask_bit(nx, ny);
                        if occupied & b == 0 {
                            if rim_mask & b != 0 {
                                rim_inner |= b;
                            } else {
                                rim_mask |= b;
                            }
                        }
                    }
                }
            }
        }

        let hi_pellet = ((h >> 8) as usize) % layout.len();
        let second_pellet = (hi_pellet + 1) % layout.len();
        let pellet_tl = |pellet: &[(i8, i8)]| -> (i32, i32) {
            let mut tl = (i32::MAX, i32::MAX);
            for &(rx, ry) in pellet {
                let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
                let dy = i32::from(ry);
                if (dy, dx) < (tl.1, tl.0) {
                    tl = (dx, dy);
                }
            }
            tl
        };

        let core1_pos = pellet_tl(layout[hi_pellet]);
        let core2_pos = if second_pellet != hi_pellet && CORE2_MIX[tier] * glow_scale > 0.12 {
            Some(pellet_tl(layout[second_pellet]))
        } else {
            None
        };

        // Still all-green wisps above armed droppings (seeded from `d.seed`
        // only — never `elapsed`). Prefer a detached row when the rim occupies
        // the cell directly beneath `y_low`.
        let (wisp1_pos, wisp2_pos) = if !wet && !drying {
            let base_x = ((h >> 16) % 3) as i32 - 1;
            let alt_x = if base_x >= 0 { -1 } else { 1 };
            let y_low = (top_y - 2).clamp(-4, 4);
            let y_hi = (top_y - 3).clamp(-4, 4);
            let pattern = ((h >> 12) & 0x1f) % 4;
            let (wx1, wy1, wx2, wy2) = if pattern == 0 || pattern == 2 {
                let wy1 = if has_bit(rim_mask, base_x, top_y - 1) {
                    y_hi
                } else {
                    y_low
                };
                let wy2 = if wy1 == y_hi { y_low } else { y_hi };
                (base_x, wy1, alt_x, wy2)
            } else {
                let wy1 = if has_bit(rim_mask, alt_x, top_y - 1) {
                    y_hi
                } else {
                    y_low
                };
                let wy2 = if wy1 == y_hi { y_low } else { y_hi };
                (alt_x, wy1, base_x, wy2)
            };
            let second = if tier >= 2 || (tier == 1 && (pattern & 1) == 1) {
                Some((wx2, wy2))
            } else {
                None
            };
            (Some((wx1, wy1)), second)
        } else {
            (None, None)
        };

        Self {
            layout,
            h,
            flip_x,
            tier,
            wet,
            drying,
            glow_scale,
            occupied,
            rim_mask,
            rim_inner,
            hi_pellet,
            second_pellet,
            core1_pos,
            core2_pos,
            wisp1_pos,
            wisp2_pos,
        }
    }
}

/// Draw one Hantavirus dropping on the floor at `d.pos`.
///
/// Every painted pixel (keyline, 1-px green rim, pellets with green core(s),
/// and still green wisps) stays strictly within the 9×9 screen box centered on
/// `w2s(d.pos, cam)`. Still with respect to `_elapsed`: only `d.age` and
/// `d.level` change the appearance.
pub fn draw_dropping(pb: &mut PixelBuffer, d: &Dropping, cam: Vec2, _elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let (sx, sy) = w2s(d.pos, cam);
    if sx.saturating_add(4) < 0
        || sy.saturating_add(4) < 0
        || sx.saturating_sub(4) >= pb.w as i32
        || sy.saturating_sub(4) >= pb.h as i32
    {
        return;
    }

    let plan = DroppingPlan::build(d);
    let rank_t = plan.tier as f32 / 4.0;

    let (pal, hi_col, under_alpha, right_alpha) = if plan.wet {
        (PAL_WET, HI_WET, 0.72f32, 0.44f32)
    } else if plan.drying {
        let dry_t = ((d.age - DRYING_AGE) / DRYING_SPAN).clamp(0.0, 1.0);
        let k = 0.58 + 0.42 * dry_t;
        let p = [
            (
                palette::mix(PAL_ARMED[0].0, PAL_DRY[0].0, k),
                palette::mix(PAL_ARMED[0].1, PAL_DRY[0].1, k),
            ),
            (
                palette::mix(PAL_ARMED[1].0, PAL_DRY[1].0, k),
                palette::mix(PAL_ARMED[1].1, PAL_DRY[1].1, k),
            ),
            (
                palette::mix(PAL_ARMED[2].0, PAL_DRY[2].0, k),
                palette::mix(PAL_ARMED[2].1, PAL_DRY[2].1, k),
            ),
        ];
        (p, palette::mix(HI_ARMED, HI_DRY, k), 0.58, 0.34)
    } else {
        (PAL_ARMED, HI_ARMED, 0.74, 0.45)
    };

    // 1. Clean 1-px lower-right dark keyline directly under each pellet pixel
    //    and off its rightmost tip, softened slightly as the green rim grows
    //    vivid at higher ranks.
    let key_scale = 1.0 - 0.45 * RIM_ALPHA[plan.tier] * plan.glow_scale;
    let mut key_done = 0u64;
    for pellet in plan.layout {
        let mut max_x = i32::MIN;
        for &(rx, _) in *pellet {
            let dx = if plan.flip_x {
                -i32::from(rx)
            } else {
                i32::from(rx)
            };
            if dx > max_x {
                max_x = dx;
            }
        }
        for &(rx, ry) in *pellet {
            let dx = if plan.flip_x {
                -i32::from(rx)
            } else {
                i32::from(rx)
            };
            let dy = i32::from(ry);

            let ub = mask_bit(dx, dy + 1);
            if (plan.occupied | key_done) & ub == 0 {
                key_done |= ub;
                blend_px(
                    pb,
                    sx + dx,
                    sy + dy + 1,
                    KEYLINE_DARK,
                    under_alpha * key_scale,
                );
            }

            if dx == max_x {
                let rb = mask_bit(dx + 1, dy);
                if (plan.occupied | key_done) & rb == 0 {
                    key_done |= rb;
                    blend_px(
                        pb,
                        sx + dx + 1,
                        sy + dy,
                        KEYLINE_DARK,
                        right_alpha * key_scale,
                    );
                }
            }
        }
    }

    // 2. 1-px translucent green rim/halo hugging the cluster (rank 1 faint,
    //    rank 5 vivid), gentler inside tight gaps and on the shaded underside
    //    so individual brown grains stay clearly separated.
    let rim_a = RIM_ALPHA[plan.tier] * plan.glow_scale;
    if plan.rim_mask != 0 && rim_a > 0.03 {
        let rim_col = palette::mix(GLOW_RIM_LOW, GLOW_RIM_HIGH, rank_t);
        for dy in -2..=3i32 {
            for dx in -3..=3i32 {
                let b = mask_bit(dx, dy);
                if plan.rim_mask & b != 0 {
                    let mut local_a = if plan.rim_inner & b != 0 {
                        rim_a * 0.68
                    } else {
                        rim_a
                    };
                    if key_done & b != 0 {
                        local_a *= 0.78;
                    }
                    blend_glow(pb, sx + dx, sy + dy, rim_col, local_a);
                }
            }
        }
    }

    // 3. Pellet bodies: warm dark brown grains shaded toward the lower-right
    //    tip, with 1–2 bright green core pixels scaled by `d.level` and age.
    let core1_col = palette::mix(GLOW_CORE_LOW, GLOW_CORE_HIGH, rank_t);
    let core2_col = palette::mix(GLOW_RIM_LOW, GLOW_CORE2, rank_t);
    let c1_t = CORE1_MIX[plan.tier] * plan.glow_scale;
    let c2_t = CORE2_MIX[plan.tier] * plan.glow_scale;

    for (pi, pellet) in plan.layout.iter().enumerate() {
        let tone = pal[(pi + ((plan.h >> 2) as usize)) % pal.len()];
        let mut tl = (i32::MAX, i32::MAX);
        let mut br = (i32::MIN, i32::MIN);
        let mut br_key = i32::MIN;

        for &(rx, ry) in *pellet {
            let dx = if plan.flip_x {
                -i32::from(rx)
            } else {
                i32::from(rx)
            };
            let dy = i32::from(ry);
            if (dy, dx) < (tl.1, tl.0) {
                tl = (dx, dy);
            }
            let k = dx + dy;
            if (k, dx) > (br_key, br.0) {
                br_key = k;
                br = (dx, dy);
            }
        }

        for &(rx, ry) in *pellet {
            let dx = if plan.flip_x {
                -i32::from(rx)
            } else {
                i32::from(rx)
            };
            let dy = i32::from(ry);
            let col = if (dx, dy) == tl && pi == plan.hi_pellet {
                if c1_t > 0.05 {
                    palette::mix(hi_col, core1_col, c1_t)
                } else {
                    hi_col
                }
            } else if (dx, dy) == tl
                && pi == plan.second_pellet
                && plan.second_pellet != plan.hi_pellet
            {
                if plan.wet {
                    let base_w = palette::mix(tone.0, hi_col, 0.58);
                    if c2_t > 0.05 {
                        palette::mix(base_w, core2_col, c2_t)
                    } else {
                        base_w
                    }
                } else if c2_t > 0.05 {
                    palette::mix(tone.0, core2_col, c2_t)
                } else {
                    tone.0
                }
            } else if (dx, dy) == br {
                tone.1
            } else {
                tone.0
            };
            pb.plot(sx + dx, sy + dy, col);
        }
    }

    // 4. Infectious tell (armed only: 0.25 <= age <= 12.0):
    //    1–2 STILL all-green wisp pixels floating 2–3 px above the cluster.
    let wisp_a = WISP_ALPHA[plan.tier];
    if let Some((wx, wy)) = plan.wisp1_pos {
        blend_glow(pb, sx + wx, sy + wy, GLOW_WISP, wisp_a);
    }
    if let Some((wx, wy)) = plan.wisp2_pos {
        blend_glow(pb, sx + wx, sy + wy, GLOW_WISP, wisp_a * 0.60);
    }
}

/// Mark the green glow pixels of one dropping as emissive on `lm` so they
/// stay bright under the lighting pass at every rank (1..=5), regardless of
/// the highlight luminance threshold.
///
/// Call right after [`draw_dropping`] with the same `pb`, `d` and `cam`.
/// Drying droppings (`age > 12.0`) are skipped so they dim naturally alongside
/// their fading point light.
pub fn mark_dropping_emissive(lm: &mut LightMap, pb: &PixelBuffer, d: &Dropping, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 || d.age > DRYING_AGE {
        return;
    }
    let (sx, sy) = w2s(d.pos, cam);
    if sx.saturating_add(4) < 0
        || sy.saturating_add(4) < 0
        || sx.saturating_sub(4) >= pb.w as i32
        || sy.saturating_sub(4) >= pb.h as i32
    {
        return;
    }

    let plan = DroppingPlan::build(d);

    if plan.rim_mask != 0 {
        for dy in -2..=3i32 {
            for dx in -3..=3i32 {
                if plan.rim_mask & mask_bit(dx, dy) != 0 {
                    lm.mark_emissive_rect(pb, sx + dx, sy + dy, 1, 1);
                }
            }
        }
    }

    lm.mark_emissive_rect(pb, sx + plan.core1_pos.0, sy + plan.core1_pos.1, 1, 1);
    if let Some((cx, cy)) = plan.core2_pos {
        lm.mark_emissive_rect(pb, sx + cx, sy + cy, 1, 1);
    }

    if let Some((wx, wy)) = plan.wisp1_pos {
        lm.mark_emissive_rect(pb, sx + wx, sy + wy, 1, 1);
    }
    if let Some((wx, wy)) = plan.wisp2_pos {
        lm.mark_emissive_rect(pb, sx + wx, sy + wy, 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Sum of green dominance `g.saturating_sub(r.max(b))` over painted pixels
    /// inside the 9×9 box centered at `(sx, sy)`.
    fn green_excess_in_box(pb: &PixelBuffer, sx: usize, sy: usize, bg: Rgb) -> u32 {
        let mut total = 0u32;
        for y in (sy - 4)..=(sy + 4) {
            for x in (sx - 4)..=(sx + 4) {
                let c = pb.pixel_at(x, y);
                if c != bg {
                    total += u32::from(c.1.saturating_sub(c.0.max(c.2)));
                }
            }
        }
        total
    }

    /// Peak green channel value inside the 9×9 box centered at `(sx, sy)`.
    fn peak_green_in_box(pb: &PixelBuffer, sx: usize, sy: usize) -> u8 {
        let mut peak = 0u8;
        for y in (sy - 4)..=(sy + 4) {
            for x in (sx - 4)..=(sx + 4) {
                peak = peak.max(pb.pixel_at(x, y).1);
            }
        }
        peak
    }

    #[test]
    fn draws_inside_nine_by_nine_box_and_nothing_outside() {
        const BG: Rgb = (28, 34, 32);
        let cam = Vec2::new(10.0, 5.0);
        let pos = Vec2::new(26.0, 21.0); // screen (16, 16)
        let (sx, sy) = (16i32, 16i32);

        for seed in 0..24u32 {
            for level in 1..=5u8 {
                for &age in &[0.05f32, 0.24, 0.25, 3.5, 12.0, 12.1, 13.8] {
                    let mut pb = PixelBuffer::new(33, 33);
                    pb.clear(BG);
                    let d = sample_dropping(pos, age, seed, level);
                    draw_dropping(&mut pb, &d, cam, 0.0);

                    let mut inside = 0usize;
                    for y in 0..pb.h {
                        for x in 0..pb.w {
                            let changed = pb.pixel_at(x, y) != BG;
                            let in_box = (x as i32 - sx).abs() <= 4 && (y as i32 - sy).abs() <= 4;
                            if in_box {
                                if changed {
                                    inside += 1;
                                }
                            } else {
                                assert!(
                                    !changed,
                                    "seed={seed} level={level} age={age} painted outside 9x9 at ({x},{y})"
                                );
                            }
                        }
                    }
                    assert!(
                        inside >= 5,
                        "seed={seed} level={level} age={age} drew too few pixels ({inside})"
                    );
                }
            }
        }
    }

    #[test]
    fn different_seeds_differ() {
        const BG: Rgb = (24, 28, 26);
        let pos = Vec2::new(12.0, 12.0);
        for level in [1u8, 5] {
            let render = |seed: u32| -> Vec<Rgb> {
                let mut pb = PixelBuffer::new(25, 25);
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
    fn age_past_twelve_fades_green_glow_and_stops_wisps() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        let mut live_pb = PixelBuffer::new(25, 25);
        live_pb.clear(BG);
        draw_dropping(&mut live_pb, &sample_dropping(pos, 4.0, 0, 3), Vec2::ZERO, 0.25);

        let mut dry_pb = PixelBuffer::new(25, 25);
        dry_pb.clear(BG);
        draw_dropping(&mut dry_pb, &sample_dropping(pos, 13.5, 0, 3), Vec2::ZERO, 0.25);

        assert_ne!(
            live_pb.pixels(),
            dry_pb.pixels(),
            "drying dropping (age > 12) must differ from armed dropping"
        );

        assert!(
            green_excess_in_box(&live_pb, 12, 12, BG)
                > green_excess_in_box(&dry_pb, 12, 12, BG) * 4,
            "drying dropping should fade its green glow sharply toward chalky grey-brown"
        );

        // Floating wisp zone (rows 8..=9, 2–3 px above the cluster): present when
        // armed, completely stopped once drying (age > 12).
        let wisp_pixels = |pb: &PixelBuffer| -> usize {
            (8..=9usize)
                .flat_map(|y| (8..=16usize).map(move |x| (x, y)))
                .filter(|&(x, y)| pb.pixel_at(x, y) != BG)
                .count()
        };
        assert!(
            wisp_pixels(&live_pb) >= 1,
            "live armed dropping should show 1–2 still green wisps above it"
        );
        assert_eq!(
            wisp_pixels(&dry_pb),
            0,
            "drying dropping (age > 12) must stop its floating wisps"
        );
    }

    #[test]
    fn fresh_unarmed_dropping_has_softer_glow_and_no_wisps() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        let mut wet_pb = PixelBuffer::new(25, 25);
        wet_pb.clear(BG);
        draw_dropping(&mut wet_pb, &sample_dropping(pos, 0.1, 3, 3), Vec2::ZERO, 0.5);

        let mut live_pb = PixelBuffer::new(25, 25);
        live_pb.clear(BG);
        draw_dropping(&mut live_pb, &sample_dropping(pos, 2.0, 3, 3), Vec2::ZERO, 0.5);

        let wisp_wet = (8..=9usize)
            .flat_map(|y| (8..=16usize).map(move |x| (x, y)))
            .filter(|&(x, y)| wet_pb.pixel_at(x, y) != BG)
            .count();
        assert_eq!(wisp_wet, 0, "fresh dropping (age < 0.25) has no floating wisps");

        assert!(
            green_excess_in_box(&live_pb, 12, 12, BG) > green_excess_in_box(&wet_pb, 12, 12, BG),
            "armed dropping should glow stronger green than a fresh unarmed dropping"
        );
        assert_ne!(
            wet_pb.pixels(),
            live_pb.pixels(),
            "fresh wet dropping must read distinct from armed dropping"
        );
    }

    #[test]
    fn rank_five_paints_brighter_green_than_rank_one() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        for seed in 0..8u32 {
            let mut prev_green = 0u32;
            let mut rank1_peak = 0u8;
            let mut rank5_peak = 0u8;

            for level in 1..=5u8 {
                let mut pb = PixelBuffer::new(25, 25);
                pb.clear(BG);
                draw_dropping(&mut pb, &sample_dropping(pos, 3.0, seed, level), Vec2::ZERO, 0.0);

                let g = green_excess_in_box(&pb, 12, 12, BG);
                let peak = peak_green_in_box(&pb, 12, 12);
                assert!(
                    g > prev_green,
                    "seed={seed}: rank {level} green glow ({g}) should exceed rank {} ({prev_green})",
                    level - 1
                );
                prev_green = g;
                if level == 1 {
                    rank1_peak = peak;
                } else if level == 5 {
                    rank5_peak = peak;
                }
            }

            assert!(
                rank5_peak > rank1_peak,
                "seed={seed}: rank 5 peak green ({rank5_peak}) must exceed rank 1 ({rank1_peak})"
            );
        }
    }

    #[test]
    fn different_elapsed_values_with_same_age_state_paint_identically() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        for &(age, level) in &[(0.1f32, 2u8), (4.0, 1), (4.0, 5), (13.0, 4)] {
            let d = sample_dropping(pos, age, 5, level);
            let render_at = |elapsed: f32| -> Vec<Rgb> {
                let mut pb = PixelBuffer::new(25, 25);
                pb.clear(BG);
                draw_dropping(&mut pb, &d, Vec2::ZERO, elapsed);
                pb.pixels().to_vec()
            };

            let base = render_at(0.0);
            for &elapsed in &[0.12f32, 0.25, 0.50, 1.25, 7.77, 42.0] {
                assert_eq!(
                    base,
                    render_at(elapsed),
                    "age={age} level={level}: elapsed={elapsed} must paint identically (no time stepping)"
                );
            }
        }
    }

    #[test]
    fn mark_dropping_emissive_preserves_rank_one_glow_and_skips_drying() {
        const FLOOR: Rgb = (14, 22, 20);
        let mut w = World::new(Theme::Sewer, 80);
        w.level = Level::empty(Theme::Sewer);
        // Place the rat far away so only dark Sewer ambient touches (20, 20).
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
        };

        // Rank 1 armed dropping: peak green is well under 232, yet explicit
        // emissive marking preserves its glow core through the lighting pass.
        let d_r1 = sample_dropping(Vec2::new(20.0, 20.0), 3.0, 0, 1);
        let plan = DroppingPlan::build(&d_r1);
        let (cx, cy) = ((20 + plan.core1_pos.0) as usize, (20 + plan.core1_pos.1) as usize);

        let mut pb_marked = PixelBuffer::new(40, 40);
        pb_marked.clear(FLOOR);
        draw_dropping(&mut pb_marked, &d_r1, Vec2::ZERO, 0.0);
        let core_unlit = pb_marked.pixel_at(cx, cy);
        assert!(
            core_unlit.1 < 232,
            "rank 1 core ({core_unlit:?}) should sit below the 232 highlight threshold"
        );

        let mut lm_marked = LightMap::new();
        lm_marked.begin_frame();
        mark_dropping_emissive(&mut lm_marked, &pb_marked, &d_r1, Vec2::ZERO);
        lm_marked.apply_scene(&mut pb_marked, &scene);

        let mut pb_unmarked = PixelBuffer::new(40, 40);
        pb_unmarked.clear(FLOOR);
        draw_dropping(&mut pb_unmarked, &d_r1, Vec2::ZERO, 0.0);
        let mut lm_unmarked = LightMap::new();
        lm_unmarked.begin_frame();
        lm_unmarked.apply_scene(&mut pb_unmarked, &scene);

        assert!(
            pb_marked.pixel_at(cx, cy).1 >= core_unlit.1,
            "emissive mark must keep rank 1 green core bright after lighting"
        );
        assert!(
            pb_marked.pixel_at(cx, cy).1 > pb_unmarked.pixel_at(cx, cy).1 + 40,
            "emissive rank 1 core must stay visibly brighter than an unmarked one"
        );

        // Drying dropping: `mark_dropping_emissive` is a no-op so it dims with the floor.
        let d_dry = sample_dropping(Vec2::new(20.0, 20.0), 13.5, 0, 5);
        let mut pb_dry_marked = PixelBuffer::new(40, 40);
        pb_dry_marked.clear(FLOOR);
        draw_dropping(&mut pb_dry_marked, &d_dry, Vec2::ZERO, 0.0);
        let mut lm_dry_marked = LightMap::new();
        lm_dry_marked.begin_frame();
        mark_dropping_emissive(&mut lm_dry_marked, &pb_dry_marked, &d_dry, Vec2::ZERO);
        lm_dry_marked.apply_scene(&mut pb_dry_marked, &scene);

        let mut pb_dry_unmarked = PixelBuffer::new(40, 40);
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
