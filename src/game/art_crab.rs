//! Art for the Outfall **Shore Crab** — a red-orange bottom-walking crab with a
//! wide low carapace, two big claws angled along `facing`, eight legs that
//! scuttle sideways along `facing.perp()`, and protruding eye stalks.
//!
//! # Render Passes
//!
//! 1. [`draw_crab`] — full-detail body pass: tight contact shadow directly under
//!    the carapace (walks on the seabed bottom, stamped with pen `0`), eight
//!    articulated sideways-scuttling legs, wide low domed carapace, eye stalks,
//!    two large pincers driven smoothly by [`CrabState`] + `progress`, warm hurt
//!    flash, and gilded elite mark matching `art_enemy::draw_brute` /
//!    `art_roster_outfall::draw_grouper`.
//! 2. [`draw_crab_lod`] — fast crowd LOD: compact wide carapace + two claw
//!    blobs, strictly smaller than full detail while preserving silhouette,
//!    facing, hurt flash, and elite stripes.
//! 3. [`draw_crab_tell`] — hostile layer, drawn **after** lighting: the oriented
//!    claw pinch box (`0..CLAW_REACH` along `facing`, `±CLAW_HALF` across
//!    `facing.perp()`).
//!
//! Pure render-only drawing: zero heap allocations, zero RNG, smooth continuous
//! animation from `view.anim` and `view.progress`.

use std::f32::consts::TAU;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Direction *toward* the light in screen space (up-left), matching `art_enemy`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Body collision radius in world pixels.
pub const CRAB_RADIUS: f32 = 6.0;

/// Pinch hit reach from `pos` along `+facing`, in world pixels.
pub const CLAW_REACH: f32 = 14.0;

/// Pinch hit half-width across `facing.perp()`, in world pixels.
pub const CLAW_HALF: f32 = 7.0;

/// Combat / movement state of the Outfall Shore Crab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrabState {
    /// Sideways scuttling with claws held ready along `facing`.
    Scuttle,
    /// Claws raised and splayed wide open across `progress` (`0..1`).
    Tell,
    /// Claws snapping shut forward into the `0..CLAW_REACH` pinch box.
    Pinch,
    /// Claws lowered and settling back to scuttle stance across `progress`.
    Recover,
}

/// Pure render input for one Outfall Shore Crab.
#[derive(Clone, Copy, Debug)]
pub struct CrabView {
    /// World centre of the crab.
    pub pos: Vec2,
    /// Unit direction the claws point (toward the rat); crabs move sideways
    /// along `facing.perp()`.
    pub facing: Vec2,
    /// Current attack / locomotion state.
    pub state: CrabState,
    /// Normalized progress `0.0..=1.0` within the current state.
    pub progress: f32,
    /// Continuous leg cycle clock in seconds.
    pub anim: f32,
    /// Hit flash intensity `0.0..=1.0`.
    pub hurt: f32,
    /// Elite flag (gilded carapace chevrons, crown boss, gold tips & perimeter arcs).
    pub elite: bool,
}

// ---- Lighting material pens (`crate::render::lighting::material`) ----------
// Layout: bits 0..=3 = MaterialKind, bit 4 = CREATURE, bits 5..=7 = height (0..=7).

/// Floor contact shadow: untagged flat default (`MaterialKind::Default = 0`).
const PEN_SHADOW: u8 = 0;
/// Fallback creature pen if caller left pen at 0:
/// `MaterialKind::Shell = 12`, `CREATURE = 0x10`, height class `3` (~12 px).
const PEN_CRAB_SHELL: u8 = (3 << 5) | 0x10 | 12;

// ---- Palette (red-orange shore crab against dark Outfall seabed) -----------

const CRAB_OUTLINE: Rgb = (14, 12, 16);
const CRAB_SHADOW_BASE: Rgb = (106, 28, 18);
const CRAB_MID_BASE: Rgb = (216, 78, 38);
const CRAB_HI_BASE: Rgb = (254, 152, 86);
const CRAB_SPEC: Rgb = (255, 224, 168);

const CRAB_CLAW_DARK: Rgb = (128, 32, 20);
const CRAB_CLAW_MID: Rgb = (236, 94, 44);
const CRAB_CLAW_HI: Rgb = (255, 174, 102);
const CRAB_CLAW_TIP: Rgb = (248, 228, 188);

const CRAB_LEG_LO: Rgb = (118, 36, 22);
const CRAB_LEG_MID: Rgb = (196, 76, 38);
const CRAB_LEG_HI: Rgb = (242, 134, 76);

const CRAB_EYE_STALK: Rgb = (164, 58, 32);
const CRAB_EYE_DARK: Rgb = (14, 12, 18);
const CRAB_EYE_GLINT: Rgb = (255, 242, 164);

const ELITE_GOLD_SH: Rgb = (138, 92, 24);
const ELITE_GOLD: Rgb = (242, 192, 68);
const ELITE_GOLD_HI: Rgb = (255, 246, 178);

// ---- Pose & pure pixel/SDF helpers -----------------------------------------

#[derive(Clone, Copy)]
struct CrabPose {
    body_u: f32,
    body_v: f32,
    claw_u: f32,
    claw_v: f32,
    claw_open: f32,
    claw_lift_y: f32,
    leg_amp: f32,
    snap_flash: f32,
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn safe_dir(v: Vec2) -> Vec2 {
    let len_sq = v.x * v.x + v.y * v.y;
    if len_sq > 1e-4 {
        let inv = 1.0 / len_sq.sqrt();
        Vec2::new(v.x * inv, v.y * inv)
    } else {
        Vec2::new(1.0, 0.0)
    }
}

#[inline]
fn shade3(sh: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(sh, mid, l * (1.0 / 0.62))
    } else {
        mix(mid, hi, (l - 0.62) * (1.0 / 0.53))
    }
}

#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(x as usize, y as usize);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amt: f32) {
    if amt <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(bg, (1.0 - amt).clamp(0.0, 1.0)));
}

fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    for _ in 0..256 {
        blend(pb, x, y, c, alpha);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[inline]
fn sd_disc(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> f32 {
    (u - cu).hypot(v - cv) - r
}

#[inline]
fn sd_ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let nu = (u - cu) / ru;
    let nv = (v - cv) / rv;
    (nu.hypot(nv) - 1.0) * ru.min(rv)
}

#[inline]
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let (pax, pay) = (u - a.0, v - a.1);
    let (bax, bay) = (b.0 - a.0, b.1 - a.1);
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    (pax - bax * h).hypot(pay - bay * h) - (r0 + (r1 - r0) * h)
}

#[inline]
fn sd_box(u: f32, v: f32, hu: f32, hv: f32) -> f32 {
    let dx = u.abs() - hu;
    let dy = v.abs() - hv;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
}

#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k {
        a.min(b)
    } else {
        let h = (k - diff) / k;
        a.min(b) - h * h * k * 0.25
    }
}

#[inline]
fn hit_flash_params(hurt: f32) -> (f32, Rgb, f32) {
    let hit = hurt.clamp(0.0, 1.0);
    if hit > 0.0 {
        let fc = if hit > 0.6 {
            (255, 244, 218)
        } else {
            (255, 218, 152)
        };
        (hit, fc, (0.30 + hit * 0.56).min(0.86))
    } else {
        (0.0, (0, 0, 0), 0.0)
    }
}

fn crab_pose(view: &CrabView) -> CrabPose {
    let prog = view.progress.clamp(0.0, 1.0);
    match view.state {
        CrabState::Scuttle => CrabPose {
            body_u: 0.0,
            body_v: (view.anim * 7.5).sin() * 0.35,
            claw_u: 7.0 + (view.anim * 3.5).sin() * 0.35,
            claw_v: 4.6,
            claw_open: 0.30 + 0.08 * (view.anim * 4.2).sin(),
            claw_lift_y: 0.0,
            leg_amp: 1.0,
            snap_flash: 0.0,
        },
        CrabState::Tell => {
            let e = smoothstep(prog);
            CrabPose {
                body_u: -e,
                body_v: (view.anim * 34.0).sin() * (0.22 * e),
                claw_u: lerp(6.6, 5.8, e),
                claw_v: lerp(4.7, 7.6, e),
                claw_open: lerp(0.32, 1.35, e),
                claw_lift_y: -2.0 * e,
                leg_amp: 0.25 * (1.0 - e),
                snap_flash: 0.0,
            }
        }
        CrabState::Pinch => {
            let (body_u, claw_u, claw_v, claw_open, claw_lift_y) = if prog <= 0.32 {
                let s = smoothstep(prog / 0.32);
                (
                    lerp(-1.0, 1.1, s),
                    lerp(5.8, 10.6, s),
                    lerp(7.6, 2.4, s),
                    lerp(1.35, 0.04, s),
                    lerp(-2.0, 0.0, s),
                )
            } else {
                let s = smoothstep((prog - 0.32) / 0.68);
                (
                    lerp(1.1, 0.3, s),
                    lerp(10.6, 8.0, s),
                    lerp(2.4, 3.8, s),
                    lerp(0.04, 0.14, s),
                    0.0,
                )
            };
            let snap_flash = (1.0 - (prog - 0.34).abs() / 0.18).max(0.0);
            CrabPose {
                body_u,
                body_v: 0.0,
                claw_u,
                claw_v,
                claw_open,
                claw_lift_y,
                leg_amp: 0.35,
                snap_flash,
            }
        }
        CrabState::Recover => {
            let r = smoothstep(prog);
            CrabPose {
                body_u: lerp(0.3, 0.0, r),
                body_v: (view.anim * 7.5).sin() * (0.35 * r),
                claw_u: lerp(5.8, 6.6, r),
                claw_v: lerp(4.0, 4.7, r),
                claw_open: lerp(0.08, 0.28, r),
                claw_lift_y: lerp(1.4, 0.0, r),
                leg_amp: 0.25 + 0.75 * r,
                snap_flash: 0.0,
            }
        }
    }
}

// ---- Public draw entry points ----------------------------------------------

/// Draw a full-detail Outfall Shore Crab into `pb` at `view.pos - cam`.
pub fn draw_crab(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2, tint: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let sx = view.pos.x - cam.x;
    let sy = view.pos.y - cam.y;
    let reach = CLAW_REACH + 8.0;
    if sx + reach < -1.0
        || sx - reach > pb.w as f32 + 1.0
        || sy + reach < -1.0
        || sy - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let prev_pen = pb.pen();
    let body_pen = if prev_pen != 0 { prev_pen } else { PEN_CRAB_SHELL };

    let f = safe_dir(view.facing);
    let p = f.perp();
    let fl = f.x * LIGHT.x + f.y * LIGHT.y;
    let pl = p.x * LIGHT.x + p.y * LIGHT.y;
    let pose = crab_pose(view);

    let (hit, flash_col, flash_str) = hit_flash_params(view.hurt);
    let flash = |c: Rgb| -> Rgb {
        if flash_str <= 0.0 {
            c
        } else {
            mix(c, flash_col, flash_str)
        }
    };

    // Harmonise subtly with `tint` while keeping unmistakable red-orange crab hues.
    let sh_c = mix(CRAB_SHADOW_BASE, tint, 0.12);
    let mid_c = mix(CRAB_MID_BASE, tint, 0.14);
    let hi_c = mix(CRAB_HI_BASE, tint, 0.08);
    let cl_mid = mix(CRAB_CLAW_MID, tint, 0.12);
    let cl_hi = mix(CRAB_CLAW_HI, tint, 0.08);
    let rim_col = if hit > 0.4 {
        (176, 118, 92)
    } else {
        CRAB_OUTLINE
    };

    let bu = pose.body_u;
    let bv = pose.body_v;

    let to_px = |u: f32, vv: f32| -> (i32, i32) {
        (
            (sx + f.x * u + p.x * vv).round() as i32,
            (sy + f.y * u + p.y * vv).round() as i32,
        )
    };

    // 0) Elite rotating royal-gold segmented perimeter arcs (matches Brute elite grammar).
    if view.elite {
        pb.set_pen(PEN_SHADOW);
        let pulse = 0.55 + 0.45 * (view.anim * 2.2).sin();
        let aura_r = CRAB_RADIUS + 4.8;
        let gold = scale((255, 206, 72), 0.52 + 0.35 * pulse);
        for i in 0..8 {
            let a0 = i as f32 * (TAU / 8.0) + view.anim * 0.35;
            let mut last = (i32::MIN, i32::MIN);
            for step in 0..4 {
                let (sa, ca) = (a0 + step as f32 * 0.08).sin_cos();
                let tx = (sx + ca * aura_r).round() as i32;
                let ty = (sy + sa * aura_r).round() as i32;
                if (tx, ty) != last {
                    blend(pb, tx, ty, gold, 0.72);
                    last = (tx, ty);
                }
            }
        }
    }

    // 1) Contact shadow directly under the crab (bottom-walker, pen 0).
    let pad = 15;
    let x0 = (sx.floor() as i32 - pad).max(0);
    let x1 = (sx.ceil() as i32 + pad).min(pb.w as i32 - 1);
    let y0 = (sy.floor() as i32 - pad).max(0);
    let y1 = (sy.ceil() as i32 + pad).min(pb.h as i32 - 1);

    pb.set_pen(PEN_SHADOW);
    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            let dx0 = x0 as f32 - sx;
            let dy = y as f32 - sy;
            let mut u = dx0 * f.x + dy * f.y;
            let mut vv = dx0 * p.x + dy * p.y;
            for x in x0..=x1 {
                let d_sh = sd_ellipse(u, vv, bu - 0.1, bv, 4.9, 7.6);
                if d_sh < 1.2 {
                    let amt = 0.44 * (1.0 - (d_sh + 0.6) / 2.2).clamp(0.0, 1.0);
                    darken(pb, x, y, amt);
                }
                u += f.x;
                vv += p.x;
            }
        }
    }

    // 2) Eight jointed walking legs (4 per side) scuttling sideways along `facing.perp()`.
    let leg_lo = flash(CRAB_LEG_LO);
    let leg_mid = flash(CRAB_LEG_MID);
    let leg_hi = flash(CRAB_LEG_HI);
    const LEGS: [(f32, f32, f32); 4] = [
        (1.9, 1.1, 10.6),
        (0.3, -0.2, 11.8),
        (-1.4, -1.5, 11.4),
        (-3.1, -2.9, 10.0),
    ];
    for (i, &(uh, tu_off, reach_v)) in LEGS.iter().enumerate() {
        let ph = view.anim * 8.0 + i as f32 * 1.55;
        let step_v = ph.sin() * 1.55 * pose.leg_amp;
        let flex_u = ph.cos() * 0.45 * pose.leg_amp;
        for side in [-1.0f32, 1.0] {
            let sv = step_v * side;
            let hip = to_px(bu + uh, bv + side * 5.6);
            let knee_v = bv + side * (reach_v * 0.77) + sv * 0.55;
            let knee_u = bu + uh + 0.40 + (tu_off - uh) * 0.25 + flex_u * 0.5;
            let tip_v = bv + side * reach_v + sv;
            let tip_u = bu + uh + (tu_off - uh) * 0.85 + flex_u;
            let knee = to_px(knee_u, knee_v);
            let tip = to_px(tip_u, tip_v);

            pb.set_pen(PEN_SHADOW);
            darken(pb, tip.0, tip.1 + 1, 0.35);

            pb.set_pen(body_pen);
            line(pb, hip, knee, leg_mid, 0.96);
            line(pb, knee, tip, leg_lo, 0.96);
            pb.plot(knee.0, knee.1, leg_hi);
        }
    }

    // 3) Wide low trapezoidal shore-crab carapace.
    let ru = 4.2f32;
    let rv = 6.7f32;
    pb.set_pen(body_pen);
    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            let dx0 = x0 as f32 - sx;
            let dy = y as f32 - sy;
            let mut u = dx0 * f.x + dy * f.y;
            let mut vv = dx0 * p.x + dy * p.y;
            for x in x0..=x1 {
                let nu_raw = ((u - bu) / ru).clamp(-1.0, 1.0);
                let width_mod = (1.0 + 0.14 * nu_raw).max(0.72);
                let d = sd_ellipse(u, vv, bu, bv, ru, rv * width_mod);
                if d < 1.1 {
                    let av = (vv - bv).abs();
                    if d <= 0.0 {
                        let nu = nu_raw;
                        let nv = ((vv - bv) / rv).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.52 + nv * nv * 0.58)).clamp(0.0, 1.0);
                        let lit = nu * fl + nv * pl;
                        let mut lum = 0.38 + 0.38 * dome + 0.56 * lit;
                        if d > -0.9 && lit > 0.22 {
                            lum += 0.24;
                        }
                        let mut col = shade3(sh_c, mid_c, hi_c, lum);
                        // Sculpted H-groove across carapace centre.
                        if (u - (bu + 0.2)).abs() < 0.45 && av < 3.6 {
                            col = mix(col, sh_c, 0.42);
                        } else if (av - 2.3).abs() < 0.42 && (u - bu).abs() < 1.8 {
                            col = mix(col, sh_c, 0.34);
                        }
                        if sd_disc(u, av, bu - 1.5, 3.6, 0.85) < 0.0 {
                            col = mix(col, CRAB_SPEC, 0.38);
                        }
                        if view.elite {
                            let s1 = ((u - (bu + 1.0)) + av * 0.40).abs();
                            let s2 = ((u - (bu - 1.2)) + av * 0.40).abs();
                            if (s1 < 0.65 || s2 < 0.60) && (1.1..5.6).contains(&av) {
                                col = shade3(
                                    ELITE_GOLD_SH,
                                    ELITE_GOLD,
                                    ELITE_GOLD_HI,
                                    0.55 + 0.50 * lit,
                                );
                            } else if sd_disc(u, vv, bu + 2.4, bv, 1.0) < 0.0 {
                                col = ELITE_GOLD_HI;
                            }
                        }
                        pb.plot(x, y, flash(col));
                    } else {
                        blend(pb, x, y, rim_col, 0.88);
                    }
                }
                u += f.x;
                vv += p.x;
            }
        }
    }

    // 4) Two big articulated claws angled along `+facing`.
    let cl_lift = pose.claw_lift_y;
    let cl_u = pose.claw_u;
    let cl_v = pose.claw_v;
    let cl_open = pose.claw_open;

    let to_px_cl = |u: f32, vv: f32| -> (i32, i32) {
        (
            (sx + f.x * u + p.x * vv).round() as i32,
            (sy + f.y * u + p.y * vv + cl_lift).round() as i32,
        )
    };

    for side in [-1.0f32, 1.0] {
        let sh_u = bu + 2.6;
        let sh_v = bv + side * 4.8;
        let wr_u = bu + (cl_u - 2.2);
        let wr_v = bv + side * (cl_v + 1.2);
        let palm_u = bu + cl_u;
        let palm_v = bv + side * cl_v;

        if cl_lift < -0.25 {
            pb.set_pen(PEN_SHADOW);
            let sh_p = to_px(palm_u, palm_v);
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    darken(pb, sh_p.0 + dx, sh_p.1 + dy + 1, 0.25);
                }
            }
        }

        pb.set_pen(body_pen);
        line(pb, to_px(sh_u, sh_v), to_px_cl(wr_u, wr_v), flash(CRAB_CLAW_DARK), 0.96);
        line(pb, to_px_cl(wr_u, wr_v), to_px_cl(palm_u, palm_v), flash(cl_mid), 0.98);

        let outer_tip = (palm_u + 2.9, palm_v + side * (0.45 + 1.15 * cl_open));
        let inner_tip = (palm_u + 2.6, palm_v - side * (0.40 + 1.25 * cl_open));

        let (pcx, pcy) = to_px_cl(palm_u + 1.1, palm_v);
        let cx0 = (pcx - 6).max(0);
        let cx1 = (pcx + 6).min(pb.w as i32 - 1);
        let cy0 = (pcy - 6).max(0);
        let cy1 = (pcy + 6).min(pb.h as i32 - 1);
        if cx0 <= cx1 && cy0 <= cy1 {
            for y in cy0..=cy1 {
                let dx0 = cx0 as f32 - sx;
                let dy = (y as f32 - cl_lift) - sy;
                let mut u = dx0 * f.x + dy * f.y;
                let mut vv = dx0 * p.x + dy * p.y;
                for x in cx0..=cx1 {
                    let d_palm = sd_ellipse(u, vv, palm_u, palm_v, 1.95, 1.45);
                    let d_out = sd_taper(
                        u,
                        vv,
                        (palm_u + 0.5, palm_v + side * 0.55),
                        outer_tip,
                        0.92,
                        0.42,
                    );
                    let d_in = sd_taper(
                        u,
                        vv,
                        (palm_u + 0.4, palm_v - side * 0.45),
                        inner_tip,
                        0.78,
                        0.36,
                    );
                    let dc = smin(d_palm, d_out, 0.35).min(smin(d_palm, d_in, 0.30));
                    if dc <= 0.0 {
                        let lit = (u - palm_u) * 0.45 * fl + (vv - palm_v) * 0.45 * pl;
                        let col = if u > palm_u + 1.85 {
                            if view.elite {
                                ELITE_GOLD_HI
                            } else {
                                CRAB_CLAW_TIP
                            }
                        } else {
                            shade3(CRAB_CLAW_DARK, cl_mid, cl_hi, 0.58 + 0.52 * lit)
                        };
                        pb.plot(x, y, flash(col));
                    } else if dc <= 0.85 {
                        blend(pb, x, y, rim_col, 0.88);
                    }
                    u += f.x;
                    vv += p.x;
                }
            }
        }
    }

    // Pinch snap glint between closing tips.
    if pose.snap_flash > 0.05 {
        let sf = pose.snap_flash;
        let sp = to_px(bu + cl_u + 2.4, bv);
        pb.plot_add(sp.0, sp.1, scale((255, 240, 178), sf));
        for &(ox, oy) in &[(-1, 0), (1, 0), (0, -1), (0, 1)] {
            pb.plot_add(sp.0 + ox, sp.1 + oy, scale((255, 176, 92), sf * 0.55));
        }
    }

    // 5) Eye stalks on the front carapace rim.
    let stalk_c = flash(CRAB_EYE_STALK);
    let glint_c = flash(CRAB_EYE_GLINT);
    for side in [-1.0f32, 1.0] {
        let st = to_px(bu + 3.6, bv + side * 2.0);
        let ep = to_px(bu + 4.7, bv + side * 2.2);
        let gl = to_px(bu + 4.3, bv + side * 1.9);
        pb.plot(st.0, st.1, stalk_c);
        pb.plot(ep.0, ep.1, CRAB_EYE_DARK);
        pb.plot(gl.0, gl.1, glint_c);
    }

    pb.set_pen(prev_pen);
}

/// Crowd LOD: wide low carapace + two claw blobs, strictly smaller than
/// [`draw_crab`].
pub fn draw_crab_lod(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2, tint: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let sx = view.pos.x - cam.x;
    let sy = view.pos.y - cam.y;
    let reach = 12.0;
    if sx + reach < -1.0
        || sx - reach > pb.w as f32 + 1.0
        || sy + reach < -1.0
        || sy - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let prev_pen = pb.pen();
    let body_pen = if prev_pen != 0 { prev_pen } else { PEN_CRAB_SHELL };
    pb.set_pen(body_pen);

    let f = safe_dir(view.facing);
    let p = f.perp();
    let fl = f.x * LIGHT.x + f.y * LIGHT.y;
    let pl = p.x * LIGHT.x + p.y * LIGHT.y;
    let pose = crab_pose(view);

    let (_, flash_col, flash_str) = hit_flash_params(view.hurt);
    let flash = |c: Rgb| -> Rgb {
        if flash_str <= 0.0 {
            c
        } else {
            mix(c, flash_col, flash_str)
        }
    };

    let sh_c = mix(CRAB_SHADOW_BASE, tint, 0.12);
    let mid_c = mix(CRAB_MID_BASE, tint, 0.14);
    let hi_c = mix(CRAB_HI_BASE, tint, 0.08);
    let cl_hi = mix(CRAB_CLAW_HI, tint, 0.08);

    let cu_off = (pose.claw_u * 0.72).clamp(4.2, 7.6);
    let cv_off = (pose.claw_v * 0.80).clamp(2.4, 5.2);

    let pad = 9;
    let x0 = (sx.floor() as i32 - pad).max(0);
    let x1 = (sx.ceil() as i32 + pad).min(pb.w as i32 - 1);
    let y0 = (sy.floor() as i32 - pad).max(0);
    let y1 = (sy.ceil() as i32 + pad).min(pb.h as i32 - 1);
    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            let dx0 = x0 as f32 - sx;
            let dy = y as f32 - sy;
            let mut u = dx0 * f.x + dy * f.y;
            let mut vv = dx0 * p.x + dy * p.y;
            for x in x0..=x1 {
                let d_body = sd_ellipse(u, vv, 0.0, 0.0, 3.0, 5.8);
                let d_claw = sd_disc(u, vv.abs(), cu_off, cv_off, 1.55);
                let d = d_body.min(d_claw);
                if d <= 0.0 {
                    let lit = (u / 4.5) * fl + (vv / 5.5) * pl;
                    let col = if d_claw < d_body {
                        if view.elite && u > cu_off + 0.5 {
                            ELITE_GOLD_HI
                        } else {
                            shade3(CRAB_CLAW_DARK, mid_c, cl_hi, 0.62 + 0.45 * lit)
                        }
                    } else {
                        let mut c = shade3(sh_c, mid_c, hi_c, 0.48 + 0.52 * lit);
                        if view.elite && (u + vv.abs() * 0.35).abs() < 0.75 {
                            c = ELITE_GOLD;
                        }
                        c
                    };
                    pb.plot(x, y, flash(col));
                } else if d <= 0.75 {
                    blend(pb, x, y, CRAB_OUTLINE, 0.82);
                }
                u += f.x;
                vv += p.x;
            }
        }
    }

    pb.set_pen(prev_pen);
}

/// Hostile-layer tell, drawn **after** lighting: the oriented claw pinch box
/// (`0..CLAW_REACH` along `facing`, `±CLAW_HALF` across `facing.perp()`).
pub fn draw_crab_tell(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let prog = view.progress.clamp(0.0, 1.0);
    match view.state {
        CrabState::Scuttle | CrabState::Recover => return,
        CrabState::Pinch if prog >= 0.48 => return,
        CrabState::Tell | CrabState::Pinch => {}
    }

    let f = safe_dir(view.facing);
    let p = f.perp();
    let sx = view.pos.x - cam.x;
    let sy = view.pos.y - cam.y;

    let u_mid = CLAW_REACH * 0.5;
    let u_half = CLAW_REACH * 0.5;
    let v_half = CLAW_HALF;

    let cx = sx + f.x * u_mid;
    let cy = sy + f.y * u_mid;
    let hx = (u_half + 2.5) * f.x.abs() + (v_half + 2.5) * p.x.abs();
    let hy = (u_half + 2.5) * f.y.abs() + (v_half + 2.5) * p.y.abs();
    let x0 = ((cx - hx).floor() as i32).max(0);
    let x1 = ((cx + hx).ceil() as i32).min(pb.w as i32 - 1);
    let y0 = ((cy - hy).floor() as i32).max(0);
    let y1 = ((cy + hy).ceil() as i32).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    if view.state == CrabState::Pinch {
        let fade = 1.0 - prog / 0.48;
        let flash_c = mix((255, 118, 72), (255, 244, 210), fade);
        for y in y0..=y1 {
            let dx0 = x0 as f32 - sx;
            let dy = y as f32 - sy;
            let mut u = dx0 * f.x + dy * f.y;
            let mut vv = dx0 * p.x + dy * p.y;
            for x in x0..=x1 {
                let d = sd_box(u - u_mid, vv, u_half, v_half);
                if (-0.70..=0.70).contains(&d) {
                    blend(pb, x, y, flash_c, 0.92 * fade);
                } else if d < -0.70 {
                    blend(pb, x, y, (255, 152, 92), 0.28 * fade);
                }
                u += f.x;
                vv += p.x;
            }
        }
        return;
    }

    // Tell state: crisp warm crimson-amber pinch box + inward-closing jaw lines.
    let pulse = 0.68 + 0.32 * (view.anim * 14.0 + prog * 10.0).sin();
    let rim_c = mix((255, 196, 68), (255, 86, 54), prog);
    let hot_c = mix(rim_c, (255, 244, 188), 0.52 * pulse);
    let jaw_v = lerp(v_half - 0.6, 1.2, smoothstep(prog));
    let inv_jaw_u = 1.0 / (CLAW_REACH - 2.5);

    for y in y0..=y1 {
        let dx0 = x0 as f32 - sx;
        let dy = y as f32 - sy;
        let mut u = dx0 * f.x + dy * f.y;
        let mut vv = dx0 * p.x + dy * p.y;
        for x in x0..=x1 {
            let d = sd_box(u - u_mid, vv, u_half, v_half);
            if (-0.70..=0.70).contains(&d) {
                blend(pb, x, y, if u > CLAW_REACH - 2.0 { hot_c } else { rim_c }, 0.95);
            } else if (-1.50..=1.50).contains(&d) {
                blend(pb, x, y, (12, 16, 22), 0.78);
            } else if d < -0.70 {
                let target_v = jaw_v * (1.0 - 0.35 * ((u - 2.5) * inv_jaw_u));
                if u > 2.5 && (vv.abs() - target_v).abs() < 0.55 {
                    blend(pb, x, y, hot_c, 0.62 * pulse);
                } else {
                    blend(pb, x, y, rim_c, (0.11 + 0.12 * prog) * pulse);
                }
            }
            u += f.x;
            vv += p.x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgb = (18, 30, 38);
    const TINT: Rgb = (224, 92, 52);

    fn sample_view(state: CrabState, facing: Vec2) -> CrabView {
        CrabView {
            pos: Vec2::new(32.0, 32.0),
            facing,
            state,
            progress: 0.5,
            anim: 0.35,
            hurt: 0.0,
            elite: false,
        }
    }

    fn painted_coords(pb: &PixelBuffer) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) != BG {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn every_state_and_facing_draws_non_empty() {
        let states = [
            CrabState::Scuttle,
            CrabState::Tell,
            CrabState::Pinch,
            CrabState::Recover,
        ];
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.707, 0.707),
        ];
        for state in states {
            for facing in facings {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear(BG);
                draw_crab(&mut pb, &sample_view(state, facing), Vec2::ZERO, TINT);
                let n = painted_coords(&pb).len();
                assert!(
                    n > 120,
                    "expected full shore crab for state={state:?} facing={facing:?}, got {n} px"
                );
            }
        }
    }

    #[test]
    fn silhouette_wider_than_tall_for_facing_down_and_claws_on_facing_side() {
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(BG);
        let v = sample_view(CrabState::Scuttle, Vec2::new(0.0, 1.0));
        draw_crab(&mut pb, &v, Vec2::ZERO, TINT);
        let pts = painted_coords(&pb);
        let min_x = pts.iter().map(|&(x, _)| x).min().unwrap();
        let max_x = pts.iter().map(|&(x, _)| x).max().unwrap();
        let min_y = pts.iter().map(|&(_, y)| y).min().unwrap();
        let max_y = pts.iter().map(|&(_, y)| y).max().unwrap();
        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        assert!(
            width > height,
            "shore crab facing (0,1) must be wider than tall: width={width}, height={height}"
        );

        // Claws sit on the +facing (+y) side, reaching notably farther forward than the rear shell.
        let forward_reach = max_y as i32 - 32;
        let rear_reach = 32 - min_y as i32;
        assert!(
            forward_reach > rear_reach + 2,
            "claws must extend on +facing (+y) side: forward={forward_reach}, rear={rear_reach}"
        );
    }

    #[test]
    fn lod_is_non_empty_and_smaller_than_full() {
        for facing in [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)] {
            let v = sample_view(CrabState::Scuttle, facing);
            let mut pb_full = PixelBuffer::new(64, 64);
            let mut pb_lod = PixelBuffer::new(64, 64);
            pb_full.clear(BG);
            pb_lod.clear(BG);
            draw_crab(&mut pb_full, &v, Vec2::ZERO, TINT);
            draw_crab_lod(&mut pb_lod, &v, Vec2::ZERO, TINT);
            let full_n = painted_coords(&pb_full).len();
            let lod_n = painted_coords(&pb_lod).len();
            assert!(lod_n >= 40, "LOD must stay readable ({lod_n} px)");
            assert!(
                lod_n < full_n,
                "LOD must be smaller than full: lod={lod_n}, full={full_n}"
            );
        }
    }

    #[test]
    fn states_anim_hurt_and_elite_are_distinct() {
        let mut pb_a = PixelBuffer::new(64, 64);
        let mut pb_b = PixelBuffer::new(64, 64);
        pb_a.clear(BG);
        pb_b.clear(BG);

        // Sideways leg scuttle animates continuously.
        let mut a = sample_view(CrabState::Scuttle, Vec2::new(0.0, 1.0));
        let mut b = a;
        b.anim = 0.75;
        draw_crab(&mut pb_a, &a, Vec2::ZERO, TINT);
        draw_crab(&mut pb_b, &b, Vec2::ZERO, TINT);
        assert_ne!(pb_a.pixels(), pb_b.pixels(), "legs must animate with anim");

        // Hurt flash brightens the sprite.
        pb_b.clear(BG);
        a.hurt = 0.85;
        draw_crab(&mut pb_b, &a, Vec2::ZERO, TINT);
        assert_ne!(pb_a.pixels(), pb_b.pixels(), "hurt flash must alter pixels");

        // Elite mark is visible in both full and LOD.
        let plain = sample_view(CrabState::Scuttle, Vec2::new(0.0, 1.0));
        let mut elite = plain;
        elite.elite = true;
        pb_a.clear(BG);
        pb_b.clear(BG);
        draw_crab(&mut pb_a, &plain, Vec2::ZERO, TINT);
        draw_crab(&mut pb_b, &elite, Vec2::ZERO, TINT);
        assert_ne!(pb_a.pixels(), pb_b.pixels(), "elite full crab must differ");

        pb_a.clear(BG);
        pb_b.clear(BG);
        draw_crab_lod(&mut pb_a, &plain, Vec2::ZERO, TINT);
        draw_crab_lod(&mut pb_b, &elite, Vec2::ZERO, TINT);
        assert_ne!(pb_a.pixels(), pb_b.pixels(), "elite LOD crab must differ");
    }

    #[test]
    fn tell_marks_claw_box_in_tell_and_is_quiet_in_scuttle() {
        let mut pb_quiet = PixelBuffer::new(64, 64);
        pb_quiet.clear(BG);
        draw_crab_tell(
            &mut pb_quiet,
            &sample_view(CrabState::Scuttle, Vec2::new(1.0, 0.0)),
            Vec2::ZERO,
        );
        assert_eq!(painted_coords(&pb_quiet).len(), 0);

        let mut pb_tell = PixelBuffer::new(64, 64);
        pb_tell.clear(BG);
        draw_crab_tell(
            &mut pb_tell,
            &sample_view(CrabState::Tell, Vec2::new(1.0, 0.0)),
            Vec2::ZERO,
        );
        let pts = painted_coords(&pb_tell);
        assert!(pts.len() > 60, "Tell must paint the pinch box ({} px)", pts.len());
        let max_x = pts.iter().map(|&(x, _)| x).max().unwrap();
        let min_x = pts.iter().map(|&(x, _)| x).min().unwrap();
        assert!(
            max_x >= 32 + CLAW_REACH as usize - 1,
            "tell box must reach CLAW_REACH along facing (max_x={max_x})"
        );
        assert!(min_x >= 30, "tell box must stay on the +facing side (min_x={min_x})");
    }

    #[test]
    fn pens_restore_and_stamp_shell_body_with_zero_shadow() {
        let mut pb = PixelBuffer::new(64, 64);
        pb.set_materials(true);
        pb.clear(BG);
        pb.set_pen(0);
        draw_crab(&mut pb, &sample_view(CrabState::Scuttle, Vec2::new(0.0, 1.0)), Vec2::ZERO, TINT);
        assert_eq!(pb.pen(), 0, "draw_crab must restore caller pen");
        assert!(pb.mat().contains(&PEN_CRAB_SHELL), "body must stamp creature shell pen");

        pb.set_pen(37);
        draw_crab_lod(&mut pb, &sample_view(CrabState::Scuttle, Vec2::new(0.0, 1.0)), Vec2::ZERO, TINT);
        assert_eq!(pb.pen(), 37, "draw_crab_lod must restore caller pen");
    }
}
