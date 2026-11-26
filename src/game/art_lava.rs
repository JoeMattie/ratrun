//! Pixel art for the **Lava Breath** weapon: airborne ballistic molten globs,
//! cooling volcanic pools, and the short fiery muzzle gout.
//!
//! Pure render-only drawing (zero rng, zero heap allocation, integer-ish maths
//! in inner loops, no platform trig or `powf`/`hypot`).
//!
//! Height readability is the core visual cue for airborne globs: the game is
//! top-down, and globs launch *up out of the screen* toward the player before
//! plunging back down in front of the rat. Each glob therefore draws:
//!
//! 1. A soft dark ground shadow at `w2s(g.ground, cam)` stamped with material
//!    pen `0` (wider and fainter near apex, tighter and darker as it lands).
//! 2. The incandescent molten blob at `w2s(g.ground, cam) - (0, height)` scaled
//!    by `1 + height / 40` (3×3 at height 0 growing to ~6×6 at apex), plus 1–2
//!    detached trailing ember pixels along the screen-space parabolic velocity.
//!
//! Molten pixels (glob body, embers, glowing pool magma, and breath gout) are
//! stamped with [`EMISSIVE_PEN`] (`MaterialKind::Emissive = 14` in
//! `render::lighting::material`) so the batch-13 lighting stages leave them
//! self-lit; shadows and cooled dark basalt crust use pen `0`.

use crate::game::ecs::components::{LavaGlob, LavaPool};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Core, body, and crust colours of a Lava Breath glob:
/// - `[0]` white-yellow incandescent core
/// - `[1]` fiery molten orange body
/// - `[2]` deep glowing red crust rim
pub const GLOB_RGB: [Rgb; 3] = [
    (255, 250, 198),
    (255, 128, 24),
    (168, 28, 16),
];

/// Material pen byte for `MaterialKind::Emissive` (`14`, non-creature, flat)
/// in `crate::render::lighting::material`.
const EMISSIVE_PEN: u8 = 14;

/// Duration of the short muzzle gout in seconds (contract: `< 0.25`).
const BREATH_SECS: f32 = 0.25;

/// Held step rate for airborne glob ember/surface wobble (6 Hz).
const CHURN_HZ: f32 = 6.0;

// Supporting lava and crust tones.
const HOT_YELLOW: Rgb = (255, 214, 72);
const EMBER_RED: Rgb = (118, 22, 14);
const CRUST_WARM: Rgb = (68, 26, 20);
const CRUST_DARK: Rgb = (36, 24, 26);
const CRUST_ASH: Rgb = (58, 44, 44);
const SHADOW_INK: Rgb = (4, 6, 8);

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

#[inline]
fn hash_u32(x: u32) -> u32 {
    let mut n = x ^ 0x9e37_79b9;
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}

#[inline]
fn hash3(x: i32, y: i32, z: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ z.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n
}

/// Map a non-zero integer offset `(dx, dy)` to `(sector 0..15, frac 0..255)`
/// around the circle using pure integer arithmetic (no `atan2`).
#[inline]
fn octant_angle16(dx: i32, dy: i32) -> (usize, i32) {
    let ax = dx.abs();
    let ay = dy.abs();
    if ax == 0 && ay == 0 {
        return (0, 0);
    }
    let pos = if ax >= ay {
        let r = (ay * 512) / ax;
        if dx >= 0 && dy >= 0 {
            r
        } else if dx < 0 && dy >= 0 {
            2048 - r
        } else if dx < 0 && dy < 0 {
            2048 + r
        } else {
            4096 - r
        }
    } else {
        let r = (ax * 512) / ay;
        if dx >= 0 && dy >= 0 {
            1024 - r
        } else if dx < 0 && dy >= 0 {
            1024 + r
        } else if dx < 0 && dy < 0 {
            3072 - r
        } else {
            3072 + r
        }
    } & 4095;
    ((pos >> 8) as usize, pos & 255)
}

/// Build a 16-sector smoothed organic splat profile in Q8 (`256 = 1.0 * r`).
///
/// Produces 3–4 irregular molten lobes in `186..=286` (~0.73–1.12× radius) so
/// every lava pool reads as an organic volcanic splat rather than a disc.
fn make_splat_lobes(seed: u32) -> [i32; 16] {
    let mut raw = [240i32; 16];
    let phase = (seed & 15) as usize;
    for (i, slot) in raw.iter_mut().enumerate() {
        let h = hash_u32(seed ^ (i as u32).wrapping_mul(0x9e37_79b9));
        let wave = if ((i + phase) & 3) < 2 { 22 } else { -18 };
        let jitter = (h % 65) as i32 - 32;
        *slot = 240 + wave + jitter;
    }
    let mut out = [256i32; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let v = (raw[(i + 15) & 15] + 2 * raw[i] + raw[(i + 1) & 15] + 2) >> 2;
        *slot = v.clamp(186, 286);
    }
    out
}

/// Draw one airborne **Lava Breath** glob and its ground shadow.
///
/// - Ground shadow at `w2s(g.ground, cam)` (pen `0`), wider and fainter near
///   apex, tighter and darker as it lands.
/// - Molten body at `w2s(g.ground, cam) - (0, height)` (emissive pen), scaled
///   by `1 + height / 40` (3×3 on the floor growing to ~6×6 at apex), with
///   1–2 detached trailing ember pixels behind its screen-space motion.
pub fn draw_glob(pb: &mut PixelBuffer, g: &LavaGlob, cam: Vec2, elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let prev_pen = pb.pen();

    let (gx, gy) = w2s(g.ground, cam);
    let height = g.height().max(0.0);
    let scale = (1.0 + height / 40.0).clamp(1.0, 4.0);
    let lift = height.round() as i32;
    let (bx, by) = (gx, gy.saturating_sub(lift));
    let u = if g.flight > 1e-4 {
        (g.t / g.flight).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // 1. Soft dark elliptical ground shadow at `w2s(g.ground, cam)` (pen 0).
    let h_frac = (height / 24.0).clamp(0.0, 1.0);
    let sh_rx = 2.0 + h_frac * 2.2;
    let sh_ry = 1.25 + h_frac * 1.25;
    let sh_peak = 0.72 - h_frac * 0.32;
    let irx = sh_rx.ceil() as i32;
    let iry = sh_ry.ceil() as i32;
    let sh_rx_q8 = (sh_rx * 256.0).round() as i32;
    let sh_ry_q8 = (sh_ry * 256.0).round() as i32;

    pb.set_pen(0);
    for dy in -iry..=iry {
        let ey = (dy << 16) / sh_ry_q8;
        let ny2_q8 = (ey * ey) >> 8;
        if ny2_q8 > 256 {
            continue;
        }
        for dx in -irx..=irx {
            let ex = (dx << 16) / sh_rx_q8;
            let e2_q8 = ((ex * ex) >> 8) + ny2_q8;
            if e2_q8 <= 256 {
                let falloff = 1.0 - (e2_q8 as f32 / 256.0) * 0.62;
                blend_px(pb, gx + dx, gy + dy, SHADOW_INK, sh_peak * falloff);
            }
        }
    }

    // 2. Molten glob + trailing ember sparks (emissive).
    pb.set_pen(EMISSIVE_PEN);

    // 1.90 * scale gives an exact 3×3 droplet at height 0 (scale 1.0) and a
    // plump ~6×6 incandescent glob at apex (height 22 -> scale 1.55).
    let body_r = 1.90 * scale;

    // 1–2 detached trailing embers behind screen-space parabolic motion:
    // trail downward toward the mouth while rising, upward while plunging.
    if height > 1.5 && g.flight > 1e-4 {
        let inv_f = 1.0 / g.flight;
        let svx = (g.to.x - g.from.x) * inv_f;
        let dh_dt = 4.0 * g.apex * (1.0 - 2.0 * u) * inv_f;
        let svy = (g.to.y - g.from.y) * inv_f - dh_dt;
        let slen_sq = svx * svx + svy * svy;
        if slen_sq > 1.0 {
            let inv_len = 1.0 / slen_sq.sqrt();
            let dx_dir = svx * inv_len;
            let dy_dir = svy * inv_len;
            let px_dir = -dy_dir;
            let py_dir = dx_dir;

            let step = (elapsed.max(0.0) * CHURN_HZ).floor() as u32;
            let eh = hash_u32(g.seed ^ step.wrapping_mul(0x9e37_79b9));
            let wob1 = ((eh & 3) as f32 - 1.5) * 0.45;
            let wob2 = (((eh >> 4) & 3) as f32 - 1.5) * 0.85;

            let d1 = body_r + 1.35;
            let e1x = bx - (dx_dir * d1 + px_dir * wob1).round() as i32;
            let e1y = by - (dy_dir * d1 + py_dir * wob1).round() as i32;
            pb.plot(e1x, e1y, GLOB_RGB[1]);

            if height > 5.0 {
                let d2 = body_r + 3.0;
                let e2x = bx - (dx_dir * d2 + px_dir * wob2).round() as i32;
                let e2y = by - (dy_dir * d2 + py_dir * wob2).round() as i32;
                if (e2x - e1x).abs() + (e2y - e1y).abs() >= 2 {
                    pb.plot(e2x, e2y, palette::mix(GLOB_RGB[2], GLOB_RGB[1], 0.30));
                }
            }
        }
    }

    // 3. Molten body at `(bx, by)`.
    if scale <= 1.06 {
        // Compact 3×3 molten droplet near launch and landing.
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                let ad = dx.abs() + dy.abs();
                let col = if ad == 0 {
                    GLOB_RGB[0]
                } else if ad == 1 {
                    if dx < 0 || dy < 0 {
                        HOT_YELLOW
                    } else {
                        GLOB_RGB[1]
                    }
                } else {
                    GLOB_RGB[2]
                };
                pb.plot(bx + dx, by + dy, col);
            }
        }
    } else {
        let ir = (body_r + 0.35).ceil() as i32;
        let step = (elapsed.max(0.0) * CHURN_HZ).floor() as u32;
        let w_h = hash_u32(g.seed ^ step.wrapping_mul(0x85eb_ca6b));
        // Slight up-left incandescent core shift for a glowing 3D dome read.
        let core_ox = -0.22 * body_r;
        let core_oy = -0.22 * body_r;
        let inv_body_r = 1.0 / body_r.max(0.5);

        for dy in -ir..=ir {
            for dx in -ir..=ir {
                let d2 = (dx * dx + dy * dy) as f32;
                let q = u32::from(dx >= 0) | (u32::from(dy >= 0) << 1);
                let q_bit = (w_h >> (q * 3)) & 3;
                let lump = (q_bit as f32 - 1.5) * 0.05 * (scale - 1.0);
                let r_eff = body_r * (1.0 + lump);
                if d2 > r_eff * r_eff {
                    continue;
                }
                let d = d2.sqrt();
                let edge_n = d / r_eff.max(0.5);
                let cdx = dx as f32 - core_ox;
                let cdy = dy as f32 - core_oy;
                let core_n = (cdx * cdx + cdy * cdy).sqrt() * inv_body_r;

                let col = if edge_n > 0.77 {
                    if dx + dy > 1 {
                        EMBER_RED
                    } else {
                        GLOB_RGB[2]
                    }
                } else if core_n < 0.42 {
                    GLOB_RGB[0]
                } else if core_n < 0.62 {
                    HOT_YELLOW
                } else {
                    GLOB_RGB[1]
                };
                pb.plot(bx + dx, by + dy, col);
            }
        }
    }

    pb.set_pen(prev_pen);
}

/// Draw one molten **Lava Breath** ground pool at `w2s(p.pos, cam)`.
///
/// Renders an irregular volcanic splat of radius `p.radius` with a fixed, still
/// molten magma pattern seeded from `p.seed` (independent of `elapsed`, so it
/// never steps choppily). Only the continuous `p.age / p.life` cooling changes
/// the pool over time: dark basalt crust creeps smoothly inward from the rim
/// until only a dull crust and a faint smoldering center remain. Molten pixels
/// stamp [`EMISSIVE_PEN`]; cooled crust stamps pen `0`.
pub fn draw_pool(pb: &mut PixelBuffer, p: &LavaPool, cam: Vec2, _elapsed: f32) {
    if pb.w == 0 || pb.h == 0 || p.radius <= 0.0 || p.life <= 0.0 {
        return;
    }

    let (cx, cy) = w2s(p.pos, cam);
    let r = p.radius.clamp(1.0, 64.0);
    let max_r = (r * 1.12).ceil() as i32;
    let (bw, bh) = (pb.w as i32, pb.h as i32);
    if cx.saturating_add(max_r) < 0
        || cy.saturating_add(max_r) < 0
        || cx.saturating_sub(max_r) >= bw
        || cy.saturating_sub(max_r) >= bh
    {
        return;
    }

    let prev_pen = pb.pen();
    let cool = (p.age / p.life).clamp(0.0, 1.0);
    let lobes = make_splat_lobes(p.seed);

    // Two fixed molten hot-spots seeded from `p.seed` (still pattern; only
    // continuous `p.age` cooling animates the pool).
    let sh_seed = hash_u32(p.seed);
    let c_span = (r * 0.32).round().max(1.0) as i32;
    let hot1_x = (((sh_seed & 7) as i32) * 2 * c_span) / 7 - c_span;
    let hot1_y = ((((sh_seed >> 3) & 7) as i32) * 2 * c_span) / 7 - c_span;
    let hot2_x = ((((sh_seed >> 6) & 7) as i32) * 2 * c_span) / 7 - c_span;
    let hot2_y = ((((sh_seed >> 9) & 7) as i32) * 2 * c_span) / 7 - c_span;

    let cool_q8 = (cool * 256.0).round() as i32;
    let fade_alpha = if cool <= 0.85 {
        1.0
    } else {
        (1.0 - (cool - 0.85) / 0.15 * 0.42).clamp(0.58, 1.0)
    };
    let r_q8 = ((r * 256.0).round() as i32).max(256);
    let max_r2 = (max_r + 1) * (max_r + 1);
    let cell_r2 = ((r * r * 0.22).round() as i32).max(4);
    let churn_atten = 256 - ((cool_q8 * 192) >> 8);
    let rim_cap = 114 - ((cool_q8 * 96) >> 8);
    let global_cool = (cool_q8 * 182) >> 8;

    let seed_ox = (p.seed & 15) as i32 - 8;
    let seed_oy = ((p.seed >> 4) & 15) as i32 - 8;

    for dy in -max_r..=max_r {
        let sy = cy + dy;
        if sy < 0 || sy >= bh {
            continue;
        }
        let dy2 = dy * dy;
        let ry = dy + seed_oy;
        let plate_y = ry.div_euclid(3);
        let d1_y = (dy - hot1_y) * (dy - hot1_y);
        let d2_y = (dy - hot2_y) * (dy - hot2_y);

        for dx in -max_r..=max_r {
            let sx = cx + dx;
            if sx < 0 || sx >= bw {
                continue;
            }
            let d2 = dx * dx + dy2;
            if d2 > max_r2 {
                continue;
            }

            let norm_q8 = if dx == 0 && dy == 0 {
                0
            } else {
                let (sec, frac) = octant_angle16(dx, dy);
                let l0 = lobes[sec];
                let l1 = lobes[(sec + 1) & 15];
                let lobe_q8 = l0 + (((l1 - l0) * frac) >> 8);
                let edge_r_q8 = (r_q8 * lobe_q8) >> 8;
                let d_q8 = ((d2 as u32) << 16).isqrt() as i32;
                if d_q8 > edge_r_q8 {
                    continue;
                }
                ((d_q8 * 256) / edge_r_q8.max(1)).min(256)
            };

            // Cohesive volcanic basalt plates separated by crisp 1 px glowing fissures.
            let rx = dx + seed_ox;
            let ridge_a = ((rx * 3 + ry * 2).rem_euclid(10) - 5).abs();
            let ridge_b = ((rx * 2 - ry * 3).rem_euclid(12) - 6).abs();
            let plate_h = (hash3(rx.div_euclid(3), plate_y, p.seed) & 63) as i32;
            let crack_boost = if ridge_a == 0 || ridge_b == 0 {
                44
            } else if ridge_a == 1 && ridge_b <= 2 {
                22
            } else {
                (plate_h >> 1) - 24
            };

            // Fixed molten hot-spots inside the pool core (attenuated smoothly by cooling).
            let d1_sq = (dx - hot1_x) * (dx - hot1_x) + d1_y;
            let d2_sq = (dx - hot2_x) * (dx - hot2_x) + d2_y;
            let mut hot_bump = 0;
            if d1_sq < cell_r2 {
                hot_bump += ((cell_r2 - d1_sq) * 44) / cell_r2;
            }
            if d2_sq < cell_r2 {
                hot_bump += ((cell_r2 - d2_sq) * 32) / cell_r2;
            }
            let churn = (hot_bump * churn_atten) >> 8;

            // Radial warmth minus rim-first crust cooling.
            let base_heat = 248 - ((norm_q8 * 162) >> 8);
            let rim_cool = (cool_q8 * norm_q8 * 116) >> 16;
            let mut heat = base_heat + crack_boost + churn - global_cool - rim_cool;

            if norm_q8 > 224 {
                heat = heat.min(rim_cap);
            }
            if d2 <= 1
                && cool < 0.96
                && ((sh_seed ^ (dx.wrapping_add(dy * 3) as u32)) & 3) == 0
            {
                heat = heat.max(64);
            }

            if heat >= 204 {
                pb.set_pen(EMISSIVE_PEN);
                pb.plot(sx, sy, GLOB_RGB[0]);
            } else if heat >= 166 {
                pb.set_pen(EMISSIVE_PEN);
                pb.plot(sx, sy, HOT_YELLOW);
            } else if heat >= 120 {
                pb.set_pen(EMISSIVE_PEN);
                pb.plot(sx, sy, GLOB_RGB[1]);
            } else if heat >= 80 {
                pb.set_pen(EMISSIVE_PEN);
                pb.plot(sx, sy, GLOB_RGB[2]);
            } else if heat >= 48 {
                pb.set_pen(EMISSIVE_PEN);
                pb.plot(sx, sy, EMBER_RED);
            } else if heat >= 22 {
                pb.set_pen(0);
                if fade_alpha >= 0.99 {
                    pb.plot(sx, sy, CRUST_WARM);
                } else {
                    blend_px(pb, sx, sy, CRUST_WARM, fade_alpha);
                }
            } else {
                pb.set_pen(0);
                let is_ridge = (plate_h & 3) == 0 && (dx + dy < 0) && norm_q8 < 218;
                let col = if is_ridge { CRUST_ASH } else { CRUST_DARK };
                if fade_alpha >= 0.99 {
                    pb.plot(sx, sy, col);
                } else {
                    blend_px(pb, sx, sy, col, fade_alpha);
                }
            }
        }
    }

    pb.set_pen(prev_pen);
}

/// Draw the short fiery **Lava Breath** muzzle gout from `mouth` along `dir`.
///
/// `t` is seconds since the volley started; draws an 8–12 px flame cone with
/// flying sparks for `0.0 <= t < 0.25` and is a no-op outside that window.
/// All painted pixels are stamped with [`EMISSIVE_PEN`].
pub fn draw_breath(pb: &mut PixelBuffer, mouth: Vec2, dir: Vec2, cam: Vec2, t: f32) {
    if pb.w == 0 || pb.h == 0 || !(0.0..BREATH_SECS).contains(&t) {
        return;
    }

    let unit = if dir.len_sq() > 1e-4 {
        dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = unit.perp();

    let u = (t / BREATH_SECS).clamp(0.0, 1.0);
    // Burst out quickly to 12 px, then burn down to 8 px as the gout dissipates.
    let cone_len = if u < 0.24 {
        8.5 + 3.5 * (u / 0.24)
    } else {
        12.0 - 4.0 * ((u - 0.24) / 0.76)
    };
    let max_hw = 0.8 + cone_len * 0.42;
    let step = (u * 6.0) as u32;

    let mx = mouth.x - cam.x;
    let my = mouth.y - cam.y;
    let tip_x = mx + unit.x * cone_len;
    let tip_y = my + unit.y * cone_len;
    let pad = max_hw.ceil() as i32 + 1;
    let (bw, bh) = (pb.w as i32, pb.h as i32);

    let x0 = (mx.min(tip_x).floor() as i32 - pad).max(0);
    let x1 = (mx.max(tip_x).ceil() as i32 + pad).min(bw - 1);
    let y0 = (my.min(tip_y).floor() as i32 - pad).max(0);
    let y1 = (my.max(tip_y).ceil() as i32 + pad).min(bh - 1);

    let prev_pen = pb.set_pen(EMISSIVE_PEN);

    if x0 <= x1 && y0 <= y1 {
        let width_scale = 1.0 - u * 0.22;
        let fringe_scale = 0.85 * (1.0 - u * 0.35);

        for sy in y0..=y1 {
            let ry = sy as f32 - my;
            for sx in x0..=x1 {
                let rx = sx as f32 - mx;
                let along = rx * unit.x + ry * unit.y;
                if along < -0.4 || along > cone_len + 0.5 {
                    continue;
                }
                let across = rx * perp.x + ry * perp.y;
                let hw = (0.75 + along.max(0.0) * 0.38) * width_scale;
                if across.abs() > hw {
                    continue;
                }

                // Ragged fiery tongue profile along the tip.
                let lane = (across * 1.5).round() as i32;
                let h = hash3(lane, step as i32, 0x4c41_5641);
                let tongue_reach = cone_len * (0.74 + 0.26 * ((h & 15) as f32 / 15.0));
                if along > tongue_reach {
                    continue;
                }

                let d_frac = (along / tongue_reach.max(1.0)).clamp(0.0, 1.0);
                let w_frac = (across.abs() / hw.max(0.5)).clamp(0.0, 1.0);
                let heat = (1.0 - d_frac * 0.56 - w_frac * 0.46) - u * 0.52;
                if heat <= 0.04 {
                    continue;
                }
                if u > 0.55 && heat < 0.24 && (hash3(sx, sy, step) & 7) < 4 {
                    continue;
                }

                let col = if heat > 0.62 {
                    GLOB_RGB[0]
                } else if heat > 0.42 {
                    HOT_YELLOW
                } else if heat > 0.22 {
                    GLOB_RGB[1]
                } else {
                    GLOB_RGB[2]
                };

                if heat > 0.26 {
                    pb.plot(sx, sy, col);
                } else {
                    pb.plot_add(sx, sy, palette::scale(col, fringe_scale));
                }
            }
        }
    }

    // Four flying sparks racing out along the cone perimeter: `(lateral_angle, speed)`.
    const SPARKS: [(f32, f32); 4] = [
        (-0.52, 1.05),
        (-0.18, 0.88),
        (0.22, 1.12),
        (0.50, 0.94),
    ];
    for (k, &(angle, speed)) in SPARKS.iter().enumerate() {
        let s_dist = 2.8 + u * 9.2 * speed;
        if s_dist > 12.2 {
            continue;
        }
        let s_lat = angle * s_dist * 0.82;
        let sx = (mx + unit.x * s_dist + perp.x * s_lat).round() as i32;
        let sy = (my + unit.y * s_dist + perp.y * s_lat).round() as i32;
        let scol = if u < 0.45 {
            if (k & 1) == 0 {
                GLOB_RGB[0]
            } else {
                GLOB_RGB[1]
            }
        } else if u < 0.78 {
            GLOB_RGB[1]
        } else {
            GLOB_RGB[2]
        };
        pb.plot(sx, sy, scol);
    }

    pb.set_pen(prev_pen);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_glob(t: f32) -> LavaGlob {
        let from = Vec2::new(20.0, 64.0);
        let to = Vec2::new(76.0, 64.0);
        let flight = 0.55;
        let u = (t / flight).clamp(0.0, 1.0);
        let ground = from + (to - from) * u;
        LavaGlob {
            ground,
            from,
            to,
            t,
            flight,
            apex: 22.0,
            damage: 14.0,
            seed: 7,
        }
    }

    fn luma_sum(c: Rgb) -> u32 {
        u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
    }

    #[test]
    fn glob_paints_inside_expected_box_and_separates_shadow_from_emissive_body() {
        let bg = (24, 34, 32);
        let mut pb = PixelBuffer::new(96, 96);
        pb.set_materials(true);
        let saved_pen = 5u8;
        pb.set_pen(saved_pen);
        pb.clear(bg);

        let g = sample_glob(0.275); // apex: ground (48, 64), height 22 -> body (48, 42)
        draw_glob(&mut pb, &g, Vec2::ZERO, 1.0);
        assert_eq!(pb.pen(), saved_pen, "draw_glob must restore the caller's pen");

        let mut shadow_px = 0usize;
        let mut body_emissive_px = 0usize;
        let mut outside_px = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) == bg {
                    continue;
                }
                let in_shadow = (43..=53).contains(&x) && (61..=67).contains(&y);
                let in_body = (39..=57).contains(&x) && (33..=51).contains(&y);
                let mat = pb.mat()[y * pb.w + x];
                if in_shadow {
                    shadow_px += 1;
                    assert_eq!(mat, 0, "ground shadow at ({x},{y}) must not be emissive");
                } else if in_body {
                    if mat == EMISSIVE_PEN {
                        body_emissive_px += 1;
                    }
                } else {
                    outside_px += 1;
                }
            }
        }
        assert_eq!(outside_px, 0, "glob painted outside shadow and body boxes");
        assert!(shadow_px >= 10, "expected visible ground shadow, got {shadow_px} px");
        assert!(
            body_emissive_px >= 18,
            "expected plump emissive apex glob, got {body_emissive_px} px"
        );
    }

    #[test]
    fn higher_glob_draws_higher_on_screen_and_larger() {
        let bg = (24, 34, 32);
        let measure = |t: f32| -> (usize, f32, usize) {
            let mut pb = PixelBuffer::new(96, 96);
            pb.set_materials(true);
            pb.clear(bg);
            // Pin ground at (48, 64) for both so only height differs.
            let mut g = sample_glob(t);
            g.ground = Vec2::new(48.0, 64.0);
            draw_glob(&mut pb, &g, Vec2::ZERO, 0.5);
            let mut min_y = pb.h;
            let mut sum_y = 0usize;
            let mut count = 0usize;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if pb.mat()[y * pb.w + x] == EMISSIVE_PEN {
                        min_y = min_y.min(y);
                        sum_y += y;
                        count += 1;
                    }
                }
            }
            let mean_y = if count > 0 {
                sum_y as f32 / count as f32
            } else {
                pb.h as f32
            };
            (min_y, mean_y, count)
        };

        let (low_min_y, low_mean_y, low_px) = measure(0.0);
        let (mid_min_y, mid_mean_y, mid_px) = measure(0.06);
        let (apex_min_y, apex_mean_y, apex_px) = measure(0.275);

        assert!(
            apex_min_y < mid_min_y && mid_min_y < low_min_y,
            "higher globs must sit higher on screen: apex={apex_min_y}, mid={mid_min_y}, low={low_min_y}"
        );
        assert!(
            apex_mean_y < mid_mean_y && mid_mean_y < low_mean_y && apex_mean_y < low_mean_y - 15.0,
            "higher globs must lift centroid: apex={apex_mean_y}, mid={mid_mean_y}, low={low_mean_y}"
        );
        assert!(
            apex_px > mid_px && mid_px > low_px,
            "higher globs must draw larger: apex={apex_px}, mid={mid_px}, low={low_px}"
        );
    }

    #[test]
    fn pool_paints_inside_expected_box_and_is_still_across_elapsed() {
        let bg = (20, 28, 26);
        let pool = LavaPool {
            pos: Vec2::new(32.0, 32.0),
            radius: 6.0,
            age: 0.0,
            life: 2.5,
            dps: 10.0,
            seed: 42,
        };

        let mut pb_a = PixelBuffer::new(64, 64);
        pb_a.set_materials(true);
        pb_a.clear(bg);
        draw_pool(&mut pb_a, &pool, Vec2::ZERO, 0.02);

        let mut inside = 0usize;
        let mut outside = 0usize;
        let mut emissive = 0usize;
        for y in 0..pb_a.h {
            for x in 0..pb_a.w {
                if pb_a.pixel_at(x, y) == bg {
                    continue;
                }
                if (25..=39).contains(&x) && (25..=39).contains(&y) {
                    inside += 1;
                    if pb_a.mat()[y * pb_a.w + x] == EMISSIVE_PEN {
                        emissive += 1;
                    }
                } else {
                    outside += 1;
                }
            }
        }
        assert_eq!(outside, 0, "pool painted outside its expected radius box");
        assert!(inside >= 60, "expected substantial molten pool, got {inside} px");
        assert!(emissive >= 50, "fresh pool should mark molten pixels emissive, got {emissive}");

        // Pattern is still (fixed from seed): elapsed alone never changes pixels.
        let mut pb_b = PixelBuffer::new(64, 64);
        pb_b.set_materials(true);
        for t in [0.12f32, 0.50, 3.75, 12.34] {
            pb_b.clear(bg);
            draw_pool(&mut pb_b, &pool, Vec2::ZERO, t);
            assert_eq!(
                pb_a.pixels(),
                pb_b.pixels(),
                "pool pattern must stay still across elapsed={t}"
            );
        }
    }

    #[test]
    fn pool_near_end_of_life_is_darker_than_fresh_pool() {
        let bg = (14, 22, 20);
        let fresh = LavaPool {
            pos: Vec2::new(24.0, 24.0),
            radius: 6.0,
            age: 0.0,
            life: 2.5,
            dps: 10.0,
            seed: 99,
        };
        let dying = LavaPool {
            age: 2.4,
            ..fresh
        };

        let render_stats = |p: &LavaPool| -> (u32, u32, usize) {
            let mut pb = PixelBuffer::new(48, 48);
            pb.clear(bg);
            draw_pool(&mut pb, p, Vec2::ZERO, p.age);
            let mut total_luma = 0u32;
            let mut peak_luma = 0u32;
            let mut changed = 0usize;
            for y in 16..=32 {
                for x in 16..=32 {
                    let c = pb.pixel_at(x, y);
                    if c != bg {
                        let l = luma_sum(c);
                        total_luma += l;
                        peak_luma = peak_luma.max(l);
                        changed += 1;
                    }
                }
            }
            (total_luma, peak_luma, changed)
        };

        let (fresh_total, fresh_peak, fresh_n) = render_stats(&fresh);
        let (dying_total, dying_peak, dying_n) = render_stats(&dying);
        assert!(fresh_n >= 60 && dying_n >= 40, "both pools should paint visible splats");
        assert!(
            dying_total < fresh_total / 2,
            "cooling pool total luma ({dying_total}) must be much darker than fresh ({fresh_total})"
        );
        assert!(
            dying_peak < fresh_peak,
            "cooling pool peak ({dying_peak}) must be dimmer than fresh core ({fresh_peak})"
        );
    }

    #[test]
    fn breath_paints_inside_forward_cone_and_stops_after_duration() {
        let bg = (16, 22, 20);
        let mouth = Vec2::new(20.0, 32.0);
        let dir = Vec2::new(1.0, 0.0);

        let mut pb = PixelBuffer::new(64, 64);
        pb.set_materials(true);
        pb.clear(bg);
        draw_breath(&mut pb, mouth, dir, Vec2::ZERO, 0.06);

        let mut inside = 0usize;
        let mut outside = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) == bg {
                    continue;
                }
                if (19..=34).contains(&x) && (25..=39).contains(&y) {
                    inside += 1;
                    assert_eq!(
                        pb.mat()[y * pb.w + x],
                        EMISSIVE_PEN,
                        "breath gout pixel ({x},{y}) must be emissive"
                    );
                } else {
                    outside += 1;
                }
            }
        }
        assert_eq!(outside, 0, "breath gout painted outside forward cone box");
        assert!(inside >= 20, "breath gout should paint a crisp fiery cone, got {inside} px");

        // Outside [0.0, 0.25): draws nothing.
        for bad_t in [-0.05f32, 0.25, 0.60, f32::NAN] {
            pb.clear(bg);
            draw_breath(&mut pb, mouth, dir, Vec2::ZERO, bad_t);
            assert!(
                pb.pixels().iter().all(|&c| c == bg),
                "draw_breath must be a no-op at t={bad_t}"
            );
        }
    }

    #[test]
    fn no_panic_at_buffer_edges_and_degenerate_inputs() {
        let mut zero = PixelBuffer::new(0, 0);
        let g = sample_glob(0.25);
        let p = LavaPool {
            pos: Vec2::ZERO,
            radius: 6.0,
            age: 0.5,
            life: 2.5,
            dps: 10.0,
            seed: 1,
        };
        draw_glob(&mut zero, &g, Vec2::ZERO, 0.0);
        draw_pool(&mut zero, &p, Vec2::ZERO, 0.0);
        draw_breath(&mut zero, Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::ZERO, 0.05);

        let mut edge = PixelBuffer::new(16, 16);
        edge.set_materials(true);
        edge.set_pen(3);
        let positions = [
            Vec2::new(0.0, 0.0),
            Vec2::new(15.0, 15.0),
            Vec2::new(-4.0, 8.0),
            Vec2::new(19.0, 8.0),
            Vec2::new(8.0, -3.0),
            Vec2::new(8.0, 18.0),
            Vec2::new(-500.0, 500.0),
        ];
        for pos in positions {
            let gg = LavaGlob {
                ground: pos,
                from: pos - Vec2::new(10.0, 0.0),
                to: pos + Vec2::new(10.0, 0.0),
                t: 0.275,
                flight: 0.55,
                apex: 22.0,
                damage: 14.0,
                seed: 3,
            };
            draw_glob(&mut edge, &gg, Vec2::ZERO, 1.0);
            let pp = LavaPool { pos, ..p };
            draw_pool(&mut edge, &pp, Vec2::ZERO, 1.0);
            draw_breath(&mut edge, pos, Vec2::ZERO, Vec2::ZERO, 0.08);
            assert_eq!(edge.pen(), 3, "pen must be restored even at buffer edges");
        }

        // Degenerate zero flight / zero radius / zero life inputs.
        let deg_g = LavaGlob {
            flight: 0.0,
            apex: 0.0,
            ..g
        };
        let deg_p = LavaPool {
            radius: 0.0,
            life: 0.0,
            ..p
        };
        draw_glob(&mut edge, &deg_g, Vec2::ZERO, 0.0);
        draw_pool(&mut edge, &deg_p, Vec2::ZERO, 0.0);
        assert_eq!(edge.pen(), 3);
    }
}
