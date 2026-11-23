//! Art for the **Drake** — a big white domestic duck that pecks (Sewer guest).
//!
//! Pure top-down procedural drawing in the body's local `(u forward, v sideways)`
//! frame, matching `art_enemy.rs` and `art_roster_sewer.rs` (`LIGHT` up-left,
//! crisp 1-px dark silhouette outline, down-right drop shadow stamped with pen
//! `0`, and warm hit flash).
//!
//! [`bill_tip`] and [`draw_duck`] both read [`geometry`], so gameplay hit tests,
//! hostile-layer tell outlines, and rendered pixels agree on the bill tip for
//! every pose and facing.

use std::f32::consts::{PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// How far the bill tip travels from the body centre at full extension
/// (`DuckPose::Peck(0.55)`).
pub const PECK_REACH: f32 = 22.0;

/// Animation phase of the Drake; the `f32` payload is progress `0.0..=1.0`
/// through that phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DuckPose {
    Waddle,
    Tell(f32),
    Peck(f32),
    Recover(f32),
}

/// Pure render input for one Drake body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DuckView {
    /// World pixels (body centre).
    pub pos: Vec2,
    /// Unit vector toward its target.
    pub facing: Vec2,
    pub pose: DuckPose,
    /// Seconds of walking, for the ~3 Hz waddle cycle.
    pub walk: f32,
    /// Paddling in a puddle: webbed feet hidden and lower body edged by a ripple arc.
    pub on_water: bool,
    /// `0.0..=1.0` hit flash.
    pub hurt: f32,
    pub seed: u32,
}

/// Direction *toward* the light in screen space (up-left), matching `art_enemy`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// ---- Palette (domestic white Pekin duck on dark wet Sewer masonry) --------

const FEATHER_SHADOW: Rgb = (132, 142, 154);
const FEATHER_MID: Rgb = (228, 228, 220);
const FEATHER_HI: Rgb = (254, 252, 244);
const WING_SHADOW: Rgb = (92, 102, 116);
const WING_MID: Rgb = (180, 186, 192);
const OUTLINE_DARK: Rgb = (12, 16, 18);
const RIM_HIT_TINT: Rgb = (176, 150, 112);

const BILL_SHADOW: Rgb = (164, 72, 16);
const BILL_MID: Rgb = (238, 126, 26);
const BILL_HI: Rgb = (255, 178, 66);
const BILL_NAIL: Rgb = (194, 86, 18);

const FOOT_HI: Rgb = (246, 140, 36);
const FOOT_MID: Rgb = (216, 104, 20);
const FOOT_DARK: Rgb = (132, 54, 14);

const EYE_COL: Rgb = (14, 12, 18);
const RIPPLE_HI: Rgb = (164, 226, 218);
const RIPPLE_LO: Rgb = (86, 148, 152);

// ---- Shared pose geometry (single source of truth for draw + bill_tip) ----

#[derive(Clone, Copy)]
struct DuckGeom {
    f: Vec2,
    p: Vec2,
    body_u: f32,
    body_v: f32,
    body_ru: f32,
    body_rv: f32,
    tail_tip_u: f32,
    tail_tip_v: f32,
    neck_a: (f32, f32),
    neck_b: (f32, f32),
    neck_r0: f32,
    neck_r1: f32,
    head_u: f32,
    head_v: f32,
    head_r: f32,
    bill_base_u: f32,
    bill_tip_u: f32,
    bill_v: f32,
    bill_hw: f32,
    wing_lift: f32,
    squat: f32,
    head_elev: f32,
    foot_stride: f32,
    foot_stance: f32,
    ruffle: f32,
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn geometry(v: &DuckView) -> DuckGeom {
    let len_sq = v.facing.x * v.facing.x + v.facing.y * v.facing.y;
    let f = if len_sq > 1e-4 {
        let inv = 1.0 / len_sq.sqrt();
        Vec2::new(v.facing.x * inv, v.facing.y * inv)
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = Vec2::new(-f.y, f.x);

    let rest_rv = if v.on_water { 5.0 } else { 5.6 };
    let mut body_u = -2.6f32;
    let mut body_v = 0.0f32;
    let mut body_ru = 5.1f32;
    let mut body_rv = rest_rv;
    let mut tail_tip_u = -10.1f32;
    let mut tail_tip_v = 0.0f32;

    let mut head_r = 2.45f32;
    let mut head_u = 4.2f32;
    let mut head_v = 0.0f32;
    let mut neck_r0 = 1.90f32;
    let mut neck_r1 = 1.38f32;

    let mut bill_len = 4.0f32;
    let mut bill_base_u = head_u + 1.4;
    let mut bill_tip_u = bill_base_u + bill_len; // 9.6 at rest (< 10 px)
    let mut bill_v = 0.0f32;
    let bill_hw = 1.40f32;

    let mut wing_lift = 0.0f32;
    let mut squat = 0.0f32;
    let mut head_elev = 0.0f32;
    let mut foot_stride = 0.0f32;
    let mut foot_stance = 0.0f32;
    let mut ruffle = 0.0f32;

    match v.pose {
        DuckPose::Waddle => {
            let waddle = (v.walk * TAU * 3.0).sin();
            let rock = waddle * 1.0;
            body_v = rock;
            tail_tip_v = rock * 1.15;
            head_v = rock * 0.45;
            bill_v = rock * 0.45;
            foot_stride = waddle;
        }
        DuckPose::Tell(prog) => {
            let prog = prog.clamp(0.0, 1.0);
            let e = smoothstep((prog * 1.2).min(1.0));
            squat = e;
            body_u = -2.6 - 1.0 * e;
            body_ru = 5.1 - 0.45 * e;
            body_rv = rest_rv + 0.85 * e;
            tail_tip_u = -10.1 - 0.5 * e;
            // Neck coils back and up over the shoulders.
            head_u = 4.2 - 3.6 * e;
            head_r = 2.45 + 0.25 * e;
            head_elev = e;
            bill_len = 4.0 - 0.4 * e;
            bill_base_u = head_u + 1.35;
            bill_tip_u = bill_base_u + bill_len;
            wing_lift = 1.25 * e;
            foot_stance = 0.55 * e;
        }
        DuckPose::Peck(prog) => {
            let prog = prog.clamp(0.0, 1.0);
            let ext = if prog <= 0.55 {
                smoothstep((prog / 0.55).clamp(0.0, 1.0))
            } else {
                1.0 - smoothstep(((prog - 0.55) / 0.45).clamp(0.0, 1.0))
            };
            // Starts compact and hits exact PECK_REACH along `facing` at p = 0.55.
            let start_tip = 7.4f32;
            bill_tip_u = start_tip + (PECK_REACH - start_tip) * ext;
            bill_len = 4.0 + 0.2 * ext;
            bill_base_u = bill_tip_u - bill_len;
            head_u = bill_base_u - 1.45;
            // Head keeps its size while the neck tapers as it shoots out.
            head_r = 2.45;
            body_u = -2.6 + 1.6 * ext;
            body_ru = 5.1 + 0.35 * ext;
            body_rv = rest_rv - 0.35 * ext;
            tail_tip_u = -10.1 + 1.2 * ext;
            neck_r0 = 1.95 - 0.15 * ext;
            neck_r1 = 1.38 - 0.36 * ext;
            wing_lift = 0.25 * ext;
            foot_stance = 0.45;
        }
        DuckPose::Recover(prog) => {
            let prog = prog.clamp(0.0, 1.0);
            let shake = (prog * TAU * 4.5).sin() * (1.0 - prog * 0.60) * 1.9;
            body_u = -2.6 - 0.3 * (1.0 - prog);
            head_u = 3.4 + 0.5 * prog;
            head_v = shake;
            head_r = 2.40;
            bill_len = 3.5 + 0.35 * prog;
            bill_base_u = head_u + 1.35;
            bill_tip_u = bill_base_u + bill_len;
            bill_v = shake * 1.15;
            head_elev = -0.35 * (1.0 - prog * 0.5);
            wing_lift = 0.45 * (1.0 - prog * 0.5);
            ruffle = 1.0 - prog * 0.35;
        }
    }

    DuckGeom {
        f,
        p,
        body_u,
        body_v,
        body_ru,
        body_rv,
        tail_tip_u,
        tail_tip_v,
        neck_a: (body_u + body_ru - 1.5, body_v),
        neck_b: (head_u, head_v),
        neck_r0,
        neck_r1,
        head_u,
        head_v,
        head_r,
        bill_base_u,
        bill_tip_u,
        bill_v,
        bill_hw,
        wing_lift,
        squat,
        head_elev,
        foot_stride,
        foot_stance,
        ruffle,
    }
}

/// Head/bill tip in world px for this pose: the hit test and the tell outline
/// use THIS, so art and gameplay cannot disagree.
pub fn bill_tip(v: &DuckView) -> Vec2 {
    let g = geometry(v);
    v.pos + g.f * g.bill_tip_u + g.p * g.bill_v
}

// ---- Pixel & SDF helpers --------------------------------------------------

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
fn scale_rgb(a: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (a.0 as f32 * f).min(255.0) as u8,
        (a.1 as f32 * f).min(255.0) as u8,
        (a.2 as f32 * f).min(255.0) as u8,
    )
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(shadow, mid, l * (1.0 / 0.62))
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
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale_rgb(bg, (1.0 - amount).clamp(0.0, 1.0)));
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
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let diff = (a - b).abs();
    if diff >= k {
        a.min(b)
    } else {
        let h = (k - diff) / k;
        a.min(b) - h * h * k * 0.25
    }
}

// ---- Main draw entry point ------------------------------------------------

/// Draw the Drake into `pb` at `v.pos - cam`.
pub fn draw_duck(pb: &mut PixelBuffer, v: &DuckView, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }

    let g = geometry(v);
    let spos = v.pos - cam;

    // Conservative off-screen early out (covers full PECK_REACH + outline + shadow).
    let reach = PECK_REACH + 8.0;
    if spos.x + reach < -1.0
        || spos.x - reach > pb.w as f32 + 1.0
        || spos.y + reach < -1.0
        || spos.y - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let fl = g.f.x * LIGHT.x + g.f.y * LIGHT.y;
    let pl = g.p.x * LIGHT.x + g.p.y * LIGHT.y;
    let lit = |nu: f32, nv: f32| -> f32 { nu * fl + nv * pl };

    let hit = v.hurt.clamp(0.0, 1.0);
    let (flash_col, flash_str) = if hit > 0.0 {
        let fc = if hit > 0.6 {
            (255, 244, 218)
        } else {
            (255, 218, 152)
        };
        (fc, (0.30 + hit * 0.56).min(0.86))
    } else {
        ((0, 0, 0), 0.0)
    };
    let apply_flash = |c: Rgb| -> Rgb {
        if flash_str <= 0.0 {
            c
        } else {
            mix(c, flash_col, flash_str)
        }
    };

    let to_screen = |u: f32, vv: f32| -> (i32, i32) {
        (
            (spos.x + g.f.x * u + g.p.x * vv).round() as i32,
            (spos.y + g.f.y * u + g.p.y * vv).round() as i32,
        )
    };

    let rim_col = if hit > 0.4 {
        RIM_HIT_TINT
    } else {
        OUTLINE_DARK
    };
    let body_pen = pb.pen();

    // 1) Two orange webbed feet (dry floor only; hidden on water).
    if !v.on_water {
        for side in [-1.0f32, 1.0] {
            let stride = side * g.foot_stride * 2.5;
            let fu = g.body_u + 0.5 + stride;
            let fv = g.body_v * 0.4 + side * (5.3 + g.foot_stance);
            let ankle = (fu - 1.5, fv - side * 1.1);
            let toe_out = (fu + 1.6, fv + side * 1.35);
            let toe_mid = (fu + 2.1, fv + side * 0.25);
            let toe_in = (fu + 1.4, fv - side * 0.55);

            let fcx = spos.x + g.f.x * fu + g.p.x * fv;
            let fcy = spos.y + g.f.y * fu + g.p.y * fv;
            let fx0 = ((fcx - 5.0).floor() as i32).max(0);
            let fx1 = ((fcx + 5.0).ceil() as i32).min(pb.w as i32 - 1);
            let fy0 = ((fcy - 5.0).floor() as i32).max(0);
            let fy1 = ((fcy + 5.0).ceil() as i32).min(pb.h as i32 - 1);

            for y in fy0..=fy1 {
                let dx0 = fx0 as f32 - spos.x;
                let dy = y as f32 - spos.y;
                let mut u = dx0 * g.f.x + dy * g.f.y;
                let mut vv = dx0 * g.p.x + dy * g.p.y;
                for x in fx0..=fx1 {
                    let d1 = sd_taper(u, vv, ankle, toe_out, 0.55, 0.55);
                    let d2 = sd_taper(u, vv, ankle, toe_mid, 0.55, 0.55);
                    let d3 = sd_taper(u, vv, ankle, toe_in, 0.55, 0.50);
                    let d_web = sd_taper(u, vv, toe_in, toe_out, 0.45, 0.45);
                    let df = d1.min(d2).min(d3).min(d_web);
                    if df < 0.0 {
                        let fc = if u > fu + 0.5 && lit(0.5, side * 0.5) > 0.0 {
                            FOOT_HI
                        } else if u > fu - 0.3 {
                            FOOT_MID
                        } else {
                            FOOT_DARK
                        };
                        pb.plot(x, y, apply_flash(fc));
                    } else if df < 0.90 {
                        blend(pb, x, y, rim_col, 0.80);
                    }
                    u += g.f.x;
                    vv += g.p.x;
                }
            }
        }
    }

    // 2) Unified SDF for body, rounded lifted wings, tail, tapering neck, head, and flat bill.
    let sd_body_part = |u: f32, vv: f32| -> f32 {
        let d_oval = sd_ellipse(u, vv, g.body_u, g.body_v, g.body_ru, g.body_rv);
        let d_tail = sd_taper(
            u,
            vv,
            (g.body_u - g.body_ru + 1.8, g.body_v),
            (g.tail_tip_u, g.tail_tip_v),
            2.2,
            0.42,
        );
        let mut d_torso = smin(d_oval, d_tail, 1.2);
        if g.wing_lift > 0.05 {
            let dv = (vv - g.body_v).abs();
            let d_wing = sd_ellipse(
                u,
                dv,
                g.body_u - 0.4,
                g.body_rv - 1.6 + g.wing_lift,
                3.7 + 0.3 * g.wing_lift,
                1.9 + 0.25 * g.wing_lift,
            );
            d_torso = smin(d_torso, d_wing, 0.85);
        }
        d_torso
    };

    let sd_neck_head = |u: f32, vv: f32| -> f32 {
        let d_neck = sd_taper(u, vv, g.neck_a, g.neck_b, g.neck_r0, g.neck_r1);
        let d_head = sd_disc(u, vv, g.head_u, g.head_v, g.head_r);
        smin(d_neck, d_head, 0.42)
    };

    let sd_bill = |u: f32, vv: f32| -> f32 {
        let tip_r = g.bill_hw * 0.92;
        let base_r = g.bill_hw * 0.80;
        let tip_center_u = g.bill_tip_u - tip_r;
        sd_taper(
            u,
            vv,
            (g.bill_base_u, g.bill_v),
            (tip_center_u, g.bill_v),
            base_r,
            tip_r,
        )
    };

    let u_min = g.tail_tip_u - 2.4;
    let u_max = g.bill_tip_u + 2.4;
    let v_abs_max = (g.body_rv + g.wing_lift + 3.0)
        .max(g.body_v.abs() + g.body_rv + 3.0)
        .max(g.bill_v.abs() + g.head_r + 3.0);
    let u_mid = 0.5 * (u_min + u_max);
    let hbu = 0.5 * (u_max - u_min);
    let hbv = v_abs_max;
    let cx = spos.x + g.f.x * u_mid;
    let cy = spos.y + g.f.y * u_mid;
    let hx = hbu * g.f.x.abs() + hbv * g.p.x.abs();
    let hy = hbu * g.f.y.abs() + hbv * g.p.y.abs();

    let sh_off: (i32, i32) = if v.on_water {
        (0, 0)
    } else if g.squat > 0.5 {
        (1, 1)
    } else {
        (1, 2)
    };
    let x0 = ((cx - hx).floor() as i32 - 1).max(0);
    let x1 = ((cx + hx).ceil() as i32 + sh_off.0 + 1).min(pb.w as i32 - 1);
    let y0 = ((cy - hy).floor() as i32 - 1).max(0);
    let y1 = ((cy + hy).ceil() as i32 + sh_off.1 + 1).min(pb.h as i32 - 1);

    let du_sh = -(sh_off.0 as f32 * g.f.x + sh_off.1 as f32 * g.f.y);
    let dv_sh = -(sh_off.0 as f32 * g.p.x + sh_off.1 as f32 * g.p.y);
    let rim_lim = 0.96f32;

    let neck_dx = g.neck_b.0 - g.neck_a.0;
    let neck_dy = g.neck_b.1 - g.neck_a.1;
    let neck_len = neck_dx.hypot(neck_dy).max(1e-5);
    let neck_ux = neck_dx / neck_len;
    let neck_uy = neck_dy / neck_len;
    let bill_mid_u = 0.5 * (g.bill_base_u + g.bill_tip_u);
    let inv_bill_len = 1.0 / (g.bill_tip_u - g.bill_base_u + 0.1);

    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            let dx0 = x0 as f32 - spos.x;
            let dy = y as f32 - spos.y;
            let mut u = dx0 * g.f.x + dy * g.f.y;
            let mut vv = dx0 * g.p.x + dy * g.p.y;
            for x in x0..=x1 {
                let d_torso = sd_body_part(u, vv);
                let d_nh = sd_neck_head(u, vv);
                let d_b = sd_bill(u, vv);
                let d_white = smin(d_torso, d_nh, 0.38);
                let d = d_white.min(d_b);

                let cover = (0.5 - d * 1.25).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let d_head_only = sd_disc(u, vv, g.head_u, g.head_v, g.head_r);
                    let col = if d_b <= d_white + 0.12 && d_head_only > -0.15 {
                        // Flat orange duck bill.
                        let nu = ((u - bill_mid_u) * inv_bill_len).clamp(-1.0, 1.0);
                        let nv = ((vv - g.bill_v) / (g.bill_hw + 0.1)).clamp(-1.0, 1.0);
                        let dome = (1.0 - 0.55 * nv * nv - 0.25 * nu * nu).clamp(0.0, 1.0);
                        let lum = 0.48 + 0.34 * dome + 0.42 * lit(nu * 0.5, nv);
                        let mut c = shade3(BILL_SHADOW, BILL_MID, BILL_HI, lum);
                        if u > g.bill_tip_u - 0.85 && (vv - g.bill_v).abs() < 0.55 {
                            c = mix(c, BILL_NAIL, 0.45);
                        } else if (u - (g.bill_base_u + 0.9)).abs() < 0.42
                            && (0.32..0.85).contains(&(vv - g.bill_v).abs())
                        {
                            c = mix(c, BILL_SHADOW, 0.42);
                        }
                        c
                    } else if d_head_only <= 0.20 {
                        // Round white duck head.
                        let nu = ((u - g.head_u) / g.head_r).clamp(-1.0, 1.0);
                        let nv = ((vv - g.head_v) / g.head_r).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu + nv * nv) * 0.70).clamp(0.0, 1.0);
                        let lum =
                            0.46 + 0.38 * dome + 0.48 * lit(nu, nv) + 0.12 * g.head_elev;
                        let mut c = shade3(FEATHER_SHADOW, FEATHER_MID, FEATHER_HI, lum);
                        if g.head_elev > 0.25 && d_head_only > -0.65 && u < g.head_u - 0.2 {
                            c = mix(c, WING_SHADOW, 0.65 * g.head_elev);
                        }
                        c
                    } else if d_nh <= d_torso + 0.12 {
                        // Tapering white neck segment.
                        let nx_v = -(u - g.neck_a.0) * neck_uy + (vv - g.neck_a.1) * neck_ux;
                        let nv = (nx_v / g.neck_r1.max(0.8)).clamp(-1.0, 1.0);
                        let dome = (1.0 - 0.65 * nv * nv).clamp(0.0, 1.0);
                        let lum = 0.42 + 0.34 * dome + 0.46 * lit(0.0, nv);
                        shade3(FEATHER_SHADOW, FEATHER_MID, FEATHER_HI, lum)
                    } else {
                        // Plump oval body + curved folded-wing line + short pointed tail.
                        let nu = ((u - g.body_u) / g.body_ru).clamp(-1.0, 1.0);
                        let nv = ((vv - g.body_v) / g.body_rv).clamp(-1.0, 1.0);
                        let dome = (1.0 - (nu * nu * 0.42 + nv * nv * 0.62)).clamp(0.0, 1.0);
                        let lum = 0.44 + 0.38 * dome + 0.48 * lit(nu * 0.72, nv);
                        let mut c = shade3(FEATHER_SHADOW, FEATHER_MID, FEATHER_HI, lum);

                        let av = (vv - g.body_v).abs();
                        let wt = ((g.body_u + 2.8 - u) / 7.6).clamp(0.0, 1.0);
                        let wing_curve = (g.body_rv * 0.50 + g.wing_lift * 0.45)
                            * (wt * PI * 0.90 + 0.12).sin()
                            + 1.15 * (1.0 - wt * 0.35);
                        let dist_wing = (av - wing_curve).abs();
                        if (g.body_u - 4.8..=g.body_u + 2.8).contains(&u) {
                            if dist_wing < 0.58 {
                                let str_w = if g.wing_lift > 0.4 { 0.90 } else { 0.68 };
                                c = mix(c, WING_SHADOW, str_w);
                            } else if av > wing_curve {
                                c = mix(c, WING_MID, 0.28 + 0.14 * g.wing_lift.min(1.0));
                            }
                        }
                        if g.head_elev > 0.2
                            && sd_disc(u, vv, g.head_u - 0.8, g.head_v, g.head_r + 1.05) < 0.0
                        {
                            c = mix(c, WING_SHADOW, 0.45 * g.head_elev);
                        }
                        c
                    };

                    let col = apply_flash(col);
                    if cover >= 0.99 {
                        pb.plot(x, y, col);
                    } else {
                        let bg = pb.pixel_at(x as usize, y as usize);
                        let out = mix(bg, rim_col, 0.88);
                        pb.plot(x, y, mix(out, col, cover));
                    }
                } else if d < rim_lim {
                    blend(pb, x, y, rim_col, 0.88);
                } else if !v.on_water {
                    let su = u + du_sh;
                    let sv = vv + dv_sh;
                    let ds = smin(sd_body_part(su, sv), sd_neck_head(su, sv), 0.38)
                        .min(sd_bill(su, sv));
                    if ds < 0.35 {
                        // Cast drop shadow stamped with pen 0 so floor shadows stay unlit.
                        pb.set_pen(0);
                        darken(pb, x, y, 0.38);
                        pb.set_pen(body_pen);
                    }
                }

                u += g.f.x;
                vv += g.p.x;
            }
        }
    }

    // 3) On water: 1-px lighter waterline ripple arc wrapping the floating hull (pen 0).
    if v.on_water {
        let wru = g.body_ru + 1.0;
        let wrv = g.body_rv + 0.9 + g.wing_lift * 0.45;
        let pulse = 0.5 + 0.5 * (v.walk * 5.0).sin();
        let cx_w = spos.x + g.f.x * g.body_u + g.p.x * g.body_v;
        let cy_w = spos.y + g.f.y * g.body_u + g.p.y * g.body_v;
        let rad_w = wru.max(wrv) + 4.5;
        let wx0 = ((cx_w - rad_w).floor() as i32).max(0);
        let wx1 = ((cx_w + rad_w).ceil() as i32).min(pb.w as i32 - 1);
        let wy0 = ((cy_w - rad_w).floor() as i32).max(0);
        let wy1 = ((cy_w + rad_w).ceil() as i32).min(pb.h as i32 - 1);

        pb.set_pen(0);
        for y in wy0..=wy1 {
            let dx0 = wx0 as f32 - spos.x;
            let dy = y as f32 - spos.y;
            let mut u = dx0 * g.f.x + dy * g.f.y;
            let mut vv = dx0 * g.p.x + dy * g.p.y;
            for x in wx0..=wx1 {
                let clear_neck =
                    u > g.body_u + g.body_ru - 0.5 && (vv - g.body_v).abs() < g.neck_r0 + 1.4;
                if !clear_neck {
                    let d_w = sd_ellipse(u, vv, g.body_u - 0.2, g.body_v, wru, wrv);
                    if (-0.45..=0.60).contains(&d_w) {
                        let l_side = lit((u - g.body_u) / wru, (vv - g.body_v) / wrv);
                        let rc = mix(RIPPLE_LO, RIPPLE_HI, (0.60 + 0.35 * l_side).clamp(0.0, 1.0));
                        blend(pb, x, y, rc, 0.86 * (1.0 - d_w.abs() * 0.65));
                    } else if (1.8 + pulse * 0.55..=2.65 + pulse * 0.55).contains(&d_w)
                        && u < g.body_u + 1.5
                    {
                        blend(pb, x, y, RIPPLE_LO, 0.38 * (1.0 - pulse * 0.3));
                    }
                }
                u += g.f.x;
                vv += g.p.x;
            }
        }
        pb.set_pen(body_pen);
    }

    // 4) Two crisp dark eyes on either side of the head (deadpan).
    let eye_col = if hit > 0.7 { (56, 44, 34) } else { EYE_COL };
    for side in [-1.0f32, 1.0] {
        let (ex, ey) = to_screen(g.head_u + 0.65, g.head_v + side * 1.65);
        pb.plot(ex, ey, eye_col);
    }

    // 5) Loose ruffled feather pixels during Recover (3 deterministic pixels).
    if g.ruffle > 0.05 {
        let rf = g.ruffle;
        let h = v.seed.wrapping_mul(0x9E37_79B9);
        let j0 = ((h & 3) as f32 - 1.5) * 0.4;
        let j1 = (((h >> 2) & 3) as f32 - 1.5) * 0.4;
        let offsets = [
            (g.body_u + 0.4 + j0, g.body_v - g.body_rv - 1.4 - (1.0 - rf) * 1.2),
            (g.body_u - 2.3 + j1, g.body_v + g.body_rv + 1.5 + (1.0 - rf) * 1.1),
            (g.body_u - 5.0, g.body_v - 2.6 - (1.0 - rf) * 1.3),
        ];
        for (idx, (ru_p, rv_p)) in offsets.iter().enumerate() {
            let (fx_p, fy_p) = to_screen(*ru_p, *rv_p);
            pb.set_pen(0);
            darken(pb, fx_p + 1, fy_p + 1, 0.26 * rf);
            pb.set_pen(body_pen);
            let fc = if idx == 1 { FEATHER_MID } else { FEATHER_HI };
            blend(pb, fx_p, fy_p, apply_flash(fc), 0.92 * rf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view(facing: Vec2, pose: DuckPose) -> DuckView {
        DuckView {
            pos: Vec2::new(40.0, 40.0),
            facing,
            pose,
            walk: 0.12,
            on_water: false,
            hurt: 0.0,
            seed: 42,
        }
    }

    #[test]
    fn every_pose_and_facing_paints_pixels() {
        let bg: Rgb = (14, 22, 20);
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.707, 0.707),
            Vec2::new(-0.707, 0.707),
            Vec2::new(-0.707, -0.707),
            Vec2::new(0.707, -0.707),
        ];
        let poses = [
            DuckPose::Waddle,
            DuckPose::Tell(0.0),
            DuckPose::Tell(0.5),
            DuckPose::Tell(1.0),
            DuckPose::Peck(0.0),
            DuckPose::Peck(0.55),
            DuckPose::Peck(1.0),
            DuckPose::Recover(0.0),
            DuckPose::Recover(0.4),
            DuckPose::Recover(1.0),
        ];

        for facing in facings {
            for pose in poses {
                let mut pb = PixelBuffer::new(96, 96);
                pb.clear(bg);
                let v = sample_view(facing, pose);
                draw_duck(&mut pb, &v, Vec2::ZERO);
                let painted = pb.pixels().iter().filter(|&&c| c != bg).count();
                assert!(
                    painted > 80,
                    "expected a full duck silhouette for facing={facing:?} pose={pose:?}, painted={painted}"
                );
            }
        }
    }

    #[test]
    fn bill_tip_matches_peck_reach_and_stays_inside_rest_radius() {
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(3.0, 4.0),
            Vec2::new(-2.0, 1.0),
        ];

        for raw_facing in facings {
            let dir = raw_facing.normalized();

            // At Peck(0.55), bill_tip must sit at PECK_REACH along facing (within 1 px).
            let peck = sample_view(raw_facing, DuckPose::Peck(0.55));
            let tip = bill_tip(&peck);
            let expected = peck.pos + dir * PECK_REACH;
            let err = tip.dist(expected);
            assert!(
                err < 1.0,
                "Peck(0.55) bill_tip {tip:?} drifted {err:.3} px from {expected:?} (facing={raw_facing:?})"
            );

            // At Waddle, bill_tip must stay strictly within 10 px of body centre across the cycle.
            for step in 0..8 {
                let mut waddle = sample_view(raw_facing, DuckPose::Waddle);
                waddle.walk = step as f32 * 0.04;
                let wtip = bill_tip(&waddle);
                let dist = wtip.dist(waddle.pos);
                assert!(
                    dist < 10.0,
                    "Waddle bill_tip distance {dist:.3} >= 10.0 px (facing={raw_facing:?}, walk={})",
                    waddle.walk
                );
            }

            // At Tell(1.0), the head coils back so the bill tip sits closer than at Waddle.
            let tell = sample_view(raw_facing, DuckPose::Tell(1.0));
            let waddle0 = sample_view(raw_facing, DuckPose::Waddle);
            assert!(
                bill_tip(&tell).dist(tell.pos) < bill_tip(&waddle0).dist(waddle0.pos),
                "Tell(1.0) should retract the bill tip behind Waddle"
            );
        }
    }

    #[test]
    fn on_water_and_hurt_flash_change_rendered_output() {
        let bg: Rgb = (28, 72, 82);
        let mut dry = PixelBuffer::new(80, 80);
        let mut wet = PixelBuffer::new(80, 80);
        let mut hurt = PixelBuffer::new(80, 80);
        dry.clear(bg);
        wet.clear(bg);
        hurt.clear(bg);

        let v_dry = sample_view(Vec2::new(1.0, 0.0), DuckPose::Waddle);
        let mut v_wet = v_dry;
        v_wet.on_water = true;
        let mut v_hurt = v_dry;
        v_hurt.hurt = 0.85;

        draw_duck(&mut dry, &v_dry, Vec2::ZERO);
        draw_duck(&mut wet, &v_wet, Vec2::ZERO);
        draw_duck(&mut hurt, &v_hurt, Vec2::ZERO);

        assert_ne!(dry.pixels(), wet.pixels(), "on_water must alter feet/ripple rendering");
        assert_ne!(dry.pixels(), hurt.pixels(), "hurt flash must brighten the sprite");
        assert!(
            wet.pixels().iter().filter(|&&c| c != bg).count() > 80,
            "on_water duck should still paint a full silhouette"
        );
    }

    #[test]
    fn handles_empty_buffer_and_zero_facing_without_panic() {
        let mut empty = PixelBuffer::new(0, 0);
        let v = sample_view(Vec2::ZERO, DuckPose::Peck(0.55));
        draw_duck(&mut empty, &v, Vec2::ZERO);
        assert!(bill_tip(&v).dist(v.pos + Vec2::new(PECK_REACH, 0.0)) < 1e-3);
    }
}
