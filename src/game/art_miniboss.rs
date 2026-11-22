//! Art for the six named minibosses and their ground telegraphs (wave 3, §8).
//!
//! Each miniboss is drawn with a bold 28–38 px silhouette (extending limbs,
//! blades, cords, handles and shadows beyond its 12–15 px collision body),
//! crisp dark contours that survive 2× nearest-pixel zoom-out, top-left
//! directional lighting, expressive faces/eyes that react to every state, an
//! arrival flourish (`Entrance`), animated locomotion (`Idle`, `Approach`),
//! three-beat anticipation (`Tell`), smear-and-debris impact (`Strike`), an
//! unmistakable open punish pose (`Recover`), attitude-filled regrouping
//! (`Retreat`), physical hurt recoil, and a chest-pointing defeat (`Death`).
//!
//! Ground telegraphs are driven strictly from the simulation's [`MinibossShape`]
//! geometry (never hard-coded sizes). Zero heap allocations; early-culled to
//! [`PixelBuffer`] bounds.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use crate::game::ecs::systems::miniboss::{
    Miniboss, MinibossKind, MinibossShape, MinibossState, SignatureShape, RECOVER_TIME,
    RING_GAP_TURN, TELL_TIME,
};
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Top-left directional light shared across `art_rat`, `art_enemy`, `art_props`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// Deep ink outline shared across miniboss silhouettes (high contrast at 2× zoom-out).
const INK: Rgb = (16, 13, 22);
/// Warm ivory hurt-flash tint.
const FLASH_COL: Rgb = (255, 248, 226);
/// Duration (seconds) of the initial arrival flourish on first spawn.
const ENTRANCE_TIME: f32 = 1.15;

/// Animation pose derived strictly from `Enemy` + `Miniboss` state (render-only).
#[derive(Clone, Copy, Debug)]
enum Pose {
    Entrance(f32),
    Idle,
    Approach,
    Tell(f32),
    Strike(f32),
    Recover(f32),
    Retreat(f32),
    Death(f32),
}

impl Pose {
    #[inline]
    fn from_state(e: &Enemy, mb: &Miniboss) -> Self {
        if !e.alive() {
            return Pose::Death(mb.timer.clamp(0.0, 1.0));
        }
        match mb.state {
            MinibossState::Approach => {
                if !mb.tell_cued && mb.origin == Vec2::ZERO && mb.timer < ENTRANCE_TIME {
                    Pose::Entrance((mb.timer / ENTRANCE_TIME).clamp(0.0, 1.0))
                } else if e.vel.len_sq() < 4.0 {
                    Pose::Idle
                } else {
                    Pose::Approach
                }
            }
            MinibossState::Tell => Pose::Tell((mb.timer / TELL_TIME.max(0.05)).clamp(0.0, 1.0)),
            MinibossState::Active => {
                Pose::Strike((mb.timer / mb.kind.active_duration().max(0.05)).clamp(0.0, 1.0))
            }
            MinibossState::Recover => {
                Pose::Recover((mb.timer / RECOVER_TIME.max(0.05)).clamp(0.0, 1.0))
            }
            MinibossState::Cooldown => Pose::Retreat((mb.timer / 3.0).clamp(0.0, 1.0)),
        }
    }
}

/// Eye expression driven by pose + hurt flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EyeExpr {
    Normal,
    Blink,
    Angry,
    Wide,
    Dizzy,
    Wince,
    Dead,
}

#[inline]
fn safe_dir(v: Vec2, fallback: Vec2) -> Vec2 {
    if v.x.is_finite() && v.y.is_finite() && v.len_sq() > 1e-4 {
        v.normalized()
    } else if fallback.x.is_finite() && fallback.y.is_finite() && fallback.len_sq() > 1e-4 {
        fallback.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// Three-beat anticipation ramp: 0..0.60 ease-out pull-back, 0.60..0.90 held
/// tension beat, 0.90..1.00 micro-coil trigger right before Strike.
#[inline]
fn tell_coil(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.60 {
        let u = t / 0.60;
        1.0 - (1.0 - u) * (1.0 - u)
    } else if t < 0.90 {
        1.0
    } else {
        1.0 + 0.16 * ((t - 0.90) / 0.10)
    }
}

/// Taut vibration during the held anticipation beat (`t >= 0.56`).
#[inline]
fn tell_quiver(t: f32) -> f32 {
    if t > 0.56 {
        let ramp = ((t - 0.56) / 0.44).clamp(0.0, 1.0);
        (t * 58.0).sin() * ramp
    } else {
        0.0
    }
}

/// Physical hurt recoil offset (pushes backward & shakes laterally when hit).
#[inline]
fn hurt_jolt(f: Vec2, flash: f32, anim: f32) -> Vec2 {
    if flash <= 0.01 {
        Vec2::ZERO
    } else {
        let shake = (anim * 44.0 + flash * 31.0).sin() * flash * 1.4;
        -f * (flash * 2.6) + f.perp() * shake
    }
}

#[inline]
fn eye_expr_for(pose: Pose, anim: f32, flash: f32) -> EyeExpr {
    if matches!(pose, Pose::Death(_)) {
        return EyeExpr::Dead;
    }
    if flash > 0.15 {
        return EyeExpr::Wince;
    }
    match pose {
        Pose::Entrance(t) => {
            if t < 0.45 {
                EyeExpr::Wide
            } else {
                EyeExpr::Angry
            }
        }
        Pose::Idle => {
            if (anim * 0.55).rem_euclid(1.0) > 0.91 {
                EyeExpr::Blink
            } else {
                EyeExpr::Normal
            }
        }
        Pose::Approach => EyeExpr::Angry,
        Pose::Tell(t) => {
            if t > 0.60 {
                EyeExpr::Wide
            } else {
                EyeExpr::Angry
            }
        }
        Pose::Strike(_) => EyeExpr::Wide,
        Pose::Recover(_) => EyeExpr::Dizzy,
        Pose::Retreat(_) => EyeExpr::Angry,
        Pose::Death(_) => EyeExpr::Dead,
    }
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, mix(cur, c, alpha.clamp(0.0, 1.0)));
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, factor: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(cur, factor.clamp(0.0, 1.0)));
}

#[inline]
fn plot_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb) {
    if x >= 0 && y >= 0 && (x as usize) < pb.w && (y as usize) < pb.h {
        pb.plot(x, y, c);
    }
}

#[inline]
fn apply_flash(c: Rgb, flash: f32) -> Rgb {
    if flash > 0.01 {
        mix(c, FLASH_COL, (flash * 0.72).clamp(0.0, 0.82))
    } else {
        c
    }
}

/// Fast soft elliptical ground shadow cast down-right (no per-pixel `sqrt`).
fn draw_shadow(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
    if rx <= 0.5 || ry <= 0.5 || strength <= 0.01 {
        return;
    }
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(bw - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(bh - 1);
    let inv_rx = 1.0 / rx;
    let inv_ry = 1.0 / ry;
    for y in y0..=y1 {
        let ny = (y as f32 + 0.5 - cy) * inv_ry;
        let ny2 = ny * ny;
        for x in x0..=x1 {
            let nx = (x as f32 + 0.5 - cx) * inv_rx;
            let d2 = nx * nx + ny2;
            if d2 < 1.0 {
                darken_px(pb, x, y, 1.0 - strength * (1.0 - d2));
            }
        }
    }
}

/// Draw a shaded disc with crisp dark outline & top-left directional highlight.
#[allow(clippy::too_many_arguments)]
fn draw_shaded_disc(
    pb: &mut PixelBuffer,
    cx: f32,
    cy: f32,
    r: f32,
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let outer = r + 0.95;
    let outer2 = outer * outer;
    let rim = (r - 0.25).max(0.0);
    let rim2 = rim * rim;
    let inv_r = 1.0 / r.max(0.5);
    let ir = outer.ceil() as i32;
    let x0 = (cx.floor() as i32 - ir).max(0);
    let x1 = (cx.ceil() as i32 + ir).min(bw - 1);
    let y0 = (cy.floor() as i32 - ir).max(0);
    let y1 = (cy.ceil() as i32 + ir).min(bh - 1);
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - cy;
        let dy2 = dy * dy;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let d2 = dx * dx + dy2;
            if d2 > outer2 {
                continue;
            }
            let c = if d2 > rim2 {
                ink_c
            } else {
                let lit = (0.52 + 0.54 * ((dx * LIGHT.x + dy * LIGHT.y) * inv_r)).clamp(0.0, 1.0);
                let base = if lit > 0.58 {
                    mix(mid, hi, (lit - 0.58) / 0.42)
                } else {
                    mix(lo, mid, lit / 0.58)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Draw a shaded axis-aligned ellipse with crisp dark contour & top-left lighting.
#[allow(clippy::too_many_arguments)]
fn draw_shaded_ellipse(
    pb: &mut PixelBuffer,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let ox = rx + 1.0;
    let oy = ry + 1.0;
    let x0 = ((cx - ox).floor() as i32).max(0);
    let x1 = ((cx + ox).ceil() as i32).min(bw - 1);
    let y0 = ((cy - oy).floor() as i32).max(0);
    let y1 = ((cy + oy).ceil() as i32).min(bh - 1);
    let inv_rx = 1.0 / rx.max(0.5);
    let inv_ry = 1.0 / ry.max(0.5);
    let inv_ox = 1.0 / ox;
    let inv_oy = 1.0 / oy;
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - cy;
        let noy2 = (dy * inv_oy) * (dy * inv_oy);
        let ny = dy * inv_ry;
        let ny2 = ny * ny;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let nox = dx * inv_ox;
            if nox * nox + noy2 > 1.0 {
                continue;
            }
            let nx = dx * inv_rx;
            let c = if nx * nx + ny2 > 0.94 {
                ink_c
            } else {
                let lit = (0.52 + 0.54 * (nx * LIGHT.x + ny * LIGHT.y)).clamp(0.0, 1.0);
                let base = if lit > 0.58 {
                    mix(mid, hi, (lit - 0.58) / 0.42)
                } else {
                    mix(lo, mid, lit / 0.58)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Draw a thick oriented tapered capsule from `a` (radius `ra`) to `b` (radius `rb`).
#[allow(clippy::too_many_arguments)]
fn draw_tapered_capsule(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    ra: f32,
    rb: f32,
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let rmax = ra.max(rb);
    let pad = rmax + 1.4;
    let x0 = (a.x.min(b.x) - pad).floor().max(0.0) as i32;
    let x1 = (a.x.max(b.x) + pad).ceil().min((bw - 1) as f32) as i32;
    let y0 = (a.y.min(b.y) - pad).floor().max(0.0) as i32;
    let y1 = (a.y.max(b.y) + pad).ceil().min((bh - 1) as f32) as i32;
    let ab = b - a;
    let len2 = ab.len_sq();
    let inv_len2 = if len2 > 1e-6 { 1.0 / len2 } else { 0.0 };
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let py = y as f32 + 0.5;
        for x in x0..=x1 {
            let px = x as f32 + 0.5;
            let pax = px - a.x;
            let pay = py - a.y;
            let t = ((pax * ab.x + pay * ab.y) * inv_len2).clamp(0.0, 1.0);
            let vx = pax - ab.x * t;
            let vy = pay - ab.y * t;
            let r = ra + (rb - ra) * t;
            let outer = r + 0.90;
            let d2 = vx * vx + vy * vy;
            if d2 > outer * outer {
                continue;
            }
            let rim = (r - 0.20).max(0.0);
            let c = if d2 > rim * rim {
                ink_c
            } else {
                let inv_r = 1.0 / r.max(0.5);
                let lit = (0.54 + 0.52 * ((vx * LIGHT.x + vy * LIGHT.y) * inv_r)).clamp(0.0, 1.0);
                let base = if lit > 0.56 {
                    mix(mid, hi, (lit - 0.56) / 0.44)
                } else {
                    mix(lo, mid, lit / 0.56)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Uniform-radius capsule convenience wrapper.
#[allow(clippy::too_many_arguments)]
fn draw_capsule(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    r: f32,
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
    flash: f32,
) {
    draw_tapered_capsule(pb, a, b, r, r, hi, mid, lo, flash);
}

/// Oriented beveled rectangle (for tilted stamps, cartons, snapped blades).
#[allow(clippy::too_many_arguments)]
fn draw_rotated_box(
    pb: &mut PixelBuffer,
    center: Vec2,
    axis: Vec2,
    half_len: f32,
    half_w: f32,
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
    flash: f32,
) {
    let u = safe_dir(axis, Vec2::new(1.0, 0.0));
    let v = u.perp();
    let rad = half_len.hypot(half_w) + 1.5;
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((center.x - rad).floor() as i32).max(0);
    let x1 = ((center.x + rad).ceil() as i32).min(bw - 1);
    let y0 = ((center.y - rad).floor() as i32).max(0);
    let y1 = ((center.y + rad).ceil() as i32).min(bh - 1);
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - center.y;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - center.x;
            let au = dx * u.x + dy * u.y;
            let av = dx * v.x + dy * v.y;
            if au.abs() > half_len + 0.9 || av.abs() > half_w + 0.9 {
                continue;
            }
            let c = if au.abs() > half_len - 0.25 || av.abs() > half_w - 0.25 {
                ink_c
            } else {
                let lit = (0.52 + 0.45 * ((dx * LIGHT.x + dy * LIGHT.y) / rad.max(1.0)))
                    .clamp(0.0, 1.0);
                let base = if lit > 0.62 {
                    hi
                } else if lit < 0.36 {
                    lo
                } else {
                    mid
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Expressive character eye with dark contour, sclera/iris, pupil & pose reaction.
#[allow(clippy::too_many_arguments)]
fn draw_eye(
    pb: &mut PixelBuffer,
    pos: Vec2,
    r: f32,
    sclera: Rgb,
    pupil: Rgb,
    look: Vec2,
    expr: EyeExpr,
    flash: f32,
) {
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    match expr {
        EyeExpr::Blink => {
            for dx in -1..=1 {
                plot_px(pb, cx + dx, cy, INK);
            }
        }
        EyeExpr::Wince => {
            // Squeezed `>` / `<` hurt wince!
            plot_px(pb, cx - 1, cy - 1, INK);
            plot_px(pb, cx, cy, INK);
            plot_px(pb, cx - 1, cy + 1, INK);
            plot_px(pb, cx + 1, cy, FLASH_COL);
        }
        EyeExpr::Dead => {
            // Bold dark X eye on defeat.
            for k in -1..=1 {
                plot_px(pb, cx + k, cy + k, INK);
                plot_px(pb, cx + k, cy - k, INK);
            }
        }
        EyeExpr::Dizzy => {
            // Spiral / stunned pale eye with offset dark dot!
            draw_shaded_disc(pb, pos.x, pos.y, r, (242, 255, 248), sclera, (128, 146, 152), flash);
            plot_px(pb, cx - 1, cy - 1, INK);
            plot_px(pb, cx + 1, cy + 1, INK);
            plot_px(pb, cx, cy, (255, 224, 108));
        }
        EyeExpr::Normal | EyeExpr::Angry | EyeExpr::Wide => {
            let rr = if expr == EyeExpr::Wide { r + 0.45 } else { r };
            draw_shaded_disc(pb, pos.x, pos.y, rr, (255, 255, 255), sclera, scale(sclera, 0.72), flash);
            let p = pos + look * (rr * 0.35);
            plot_px(pb, p.x.round() as i32, p.y.round() as i32, pupil);
            // Tiny crisp specular catchlight on top-left so eyes pop at 2× zoom-out.
            plot_px(pb, cx - 1, cy - 1, (255, 255, 255));
            if expr == EyeExpr::Angry {
                // Furrowed slanted brow across top edge.
                let b0 = pos + Vec2::new(-rr, -rr * 0.85);
                let b1 = pos + Vec2::new(rr, -rr * 0.35);
                pb.line(b0.x.round() as i32, b0.y.round() as i32, b1.x.round() as i32, b1.y.round() as i32, INK);
            }
        }
    }
}

/// Three crisp orbiting dizzy stars above a stunned/exposed miniboss in Recover.
fn draw_dizzy_stars(pb: &mut PixelBuffer, center: Vec2, rx: f32, t: f32) {
    let ry = (rx * 0.42).max(2.2);
    for i in 0..3 {
        let ang = t * TAU * 1.6 + i as f32 * (TAU / 3.0);
        let q = center + Vec2::new(ang.cos() * rx, ang.sin() * ry);
        let qx = q.x.round() as i32;
        let qy = q.y.round() as i32;
        let col = if i % 2 == 0 { (255, 236, 118) } else { (168, 255, 214) };
        plot_px(pb, qx, qy, (255, 255, 255));
        blend_px(pb, qx - 1, qy, col, 0.85);
        blend_px(pb, qx + 1, qy, col, 0.85);
        blend_px(pb, qx, qy - 1, col, 0.85);
        blend_px(pb, qx, qy + 1, col, 0.85);
    }
}

/// Unmistakable mint-gold punish-window beacon during `Pose::Recover`.
fn draw_recover_beacon(pb: &mut PixelBuffer, pos: Vec2, t: f32) {
    let pulse = 0.55 + 0.45 * (t * TAU * 3.5).sin();
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    let mint: Rgb = (138, 255, 198);
    let gold: Rgb = (255, 232, 124);
    let core = mix(gold, (255, 255, 255), pulse * 0.7);
    // Crisp pulsing star cross right on the weak spot.
    plot_px(pb, cx, cy, (255, 255, 255));
    for d in 1..=3 {
        let a = (1.0 - d as f32 * 0.24) * (0.65 + 0.35 * pulse);
        let col = if d == 1 { core } else { mint };
        blend_px(pb, cx - d, cy, col, a);
        blend_px(pb, cx + d, cy, col, a);
        blend_px(pb, cx, cy - d, col, a);
        blend_px(pb, cx, cy + d, col, a);
    }
    // Subtle rotating diamond pips around the exposed spot.
    let rr = 4.2 + pulse * 1.4;
    for k in 0..4 {
        let a = t * TAU + k as f32 * FRAC_PI_2;
        let q = pos + Vec2::from_angle(a) * rr;
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, mint, 0.78);
    }
}

/// Crisp radial impact sparks & flying chips on a hurt flash.
fn draw_hurt_burst(pb: &mut PixelBuffer, center: Vec2, f: Vec2, flash: f32, spark_col: Rgb) {
    if flash <= 0.12 {
        return;
    }
    let spread = (1.0 - flash) * 5.0 + 7.5;
    for k in 0..6 {
        let ang = k as f32 * (TAU / 6.0) + 0.35;
        let dir = f.rotate(ang);
        let p0 = center + dir * (spread * 0.55);
        let p1 = center + dir * spread;
        pb.line(
            p0.x.round() as i32,
            p0.y.round() as i32,
            p1.x.round() as i32,
            p1.y.round() as i32,
            if k % 2 == 0 { FLASH_COL } else { spark_col },
        );
    }
}

/// Expanding arrival dust / energy shock-ring during `Pose::Entrance(t)`.
fn draw_entrance_ring(pb: &mut PixelBuffer, center: Vec2, t: f32, col: Rgb) {
    let fade = (1.0 - t).clamp(0.0, 1.0);
    if fade <= 0.04 {
        return;
    }
    let rx = 7.0 + t * 14.0;
    let ry = 4.2 + t * 8.2;
    let steps = 28;
    for i in 0..steps {
        let a = (i as f32 / steps as f32) * TAU + t * 1.5;
        let q = center + Vec2::new(a.cos() * rx, a.sin() * ry + 3.0);
        let c = if i % 3 == 0 { (255, 250, 224) } else { col };
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, c, fade * 0.85);
    }
}

/// Gleaming golden chest-drop beacon & bouncing pointer chevron on `Pose::Death(t)`.
fn draw_chest_pointer(pb: &mut PixelBuffer, center: Vec2, t: f32) {
    let reveal = ((t - 0.28) / 0.35).clamp(0.0, 1.0);
    if reveal <= 0.01 {
        return;
    }
    let bob = (t * TAU * 2.5).sin() * 1.8;
    let gold_hi: Rgb = (255, 244, 158);
    let gold_mid: Rgb = (244, 192, 68);
    // Warm shimmering golden spot right where the chest spawns.
    for dy in -2..=2 {
        for dx in -3..=3 {
            let d2 = (dx * dx) as f32 / 9.0 + (dy * dy) as f32 / 4.0;
            if d2 <= 1.0 {
                blend_px(
                    pb,
                    center.x.round() as i32 + dx,
                    center.y.round() as i32 + dy + 1,
                    mix(gold_mid, gold_hi, 1.0 - d2),
                    (1.0 - d2 * 0.65) * 0.72 * reveal,
                );
            }
        }
    }
    // Bouncing downward golden arrow chevron pointing cleanly at the chest center!
    let ax = center.x.round() as i32;
    let ay = (center.y - 8.5 + bob).round() as i32;
    for k in 0..=3 {
        plot_px(pb, ax - k, ay - k, INK);
        plot_px(pb, ax + k, ay - k, INK);
        plot_px(pb, ax - k, ay - k - 1, gold_hi);
        plot_px(pb, ax + k, ay - k - 1, gold_hi);
    }
    plot_px(pb, ax, ay, gold_hi);
    plot_px(pb, ax, ay + 1, (255, 255, 255));
    // Rising golden loot sparkles.
    for i in 0..4 {
        let phase = (t * 1.8 + i as f32 * 0.25).fract();
        let sx = center.x + ((i as f32 * 2.3 + t * 5.0).sin()) * 6.5;
        let sy = center.y + 1.0 - phase * 12.0;
        blend_px(pb, sx.round() as i32, sy.round() as i32, gold_hi, (1.0 - phase) * reveal);
    }
}

/// Fast curved smear arc (for scissor snaps, bobbin swings, drill blurs).
#[allow(clippy::too_many_arguments)]
fn draw_smear_arc(
    pb: &mut PixelBuffer,
    origin: Vec2,
    a0: f32,
    a1: f32,
    r_inner: f32,
    r_outer: f32,
    col_inner: Rgb,
    col_outer: Rgb,
    alpha: f32,
) {
    if alpha <= 0.02 || r_outer <= r_inner {
        return;
    }
    let span = (a1 - a0).abs();
    let steps = ((span * r_outer).ceil() as i32).clamp(6, 36);
    let r_steps = ((r_outer - r_inner).ceil() as i32).clamp(2, 10);
    for i in 0..=steps {
        let u = i as f32 / steps as f32;
        let ang = a0 + (a1 - a0) * u;
        let dir = Vec2::from_angle(ang);
        let taper = (u * PI).sin().max(0.25);
        for j in 0..=r_steps {
            let v = j as f32 / r_steps as f32;
            let r = r_inner + (r_outer - r_inner) * v;
            let q = origin + dir * r;
            let col = mix(col_inner, col_outer, v);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha * taper);
        }
    }
}

// ============================================================================
// Public Miniboss Body Entry Point
// ============================================================================

/// Draw one miniboss body at `e.pos` with full pose animation, entrance flourish,
/// hurt reaction, death collapse, and outer radial HP ring.
pub fn draw(pb: &mut PixelBuffer, e: &Enemy, mb: &Miniboss, cam: Vec2, accent: Rgb) {
    let p = e.pos - cam;
    if !p.x.is_finite() || !p.y.is_finite() {
        return;
    }
    if p.x < -44.0 || p.y < -44.0 || p.x > pb.w as f32 + 44.0 || p.y > pb.h as f32 + 44.0 {
        return;
    }

    let pose = Pose::from_state(e, mb);
    let flash = (e.flash / 0.12).clamp(0.0, 1.0);
    let facing = match pose {
        Pose::Tell(_) | Pose::Strike(_) | Pose::Recover(_) => safe_dir(mb.aim, e.facing),
        _ => safe_dir(e.facing, mb.aim),
    };
    let anim = if e.anim.is_finite() {
        e.anim + if matches!(pose, Pose::Idle) { mb.timer * 2.2 } else { 0.0 }
    } else {
        mb.timer * 2.2
    };

    // Exact Bell Diver chip direction comes straight from `mb.telegraph()`'s
    // `RingGap { gap_dir }` (falling back to `facing.rotate(RING_GAP_TURN)`), so
    // the missing-note gap in the ring always emanates right from the chipped rim.
    let bell_gap_dir = match mb.telegraph() {
        Some(MinibossShape::RingGap { gap_dir, .. }) => {
            safe_dir(gap_dir, facing.rotate(RING_GAP_TURN))
        }
        _ => facing.rotate(RING_GAP_TURN),
    };

    match mb.kind {
        MinibossKind::Corkscrew => draw_corkscrew(pb, p, facing, pose, anim, flash),
        MinibossKind::SisterSnip => draw_sister_snip(pb, p, facing, pose, anim, flash),
        MinibossKind::Bobbin => draw_bobbin(pb, p, facing, pose, anim, flash),
        MinibossKind::BellDiver => draw_bell_diver(pb, p, facing, bell_gap_dir, pose, anim, flash),
        MinibossKind::Foreman => draw_foreman(pb, p, facing, pose, anim, flash),
        MinibossKind::MilkCarton => draw_milk_carton(pb, p, facing, pose, anim, flash),
        MinibossKind::ValveWidow => draw_valve_widow(pb, p, facing, pose, anim, flash),
        MinibossKind::Mudlark => draw_mudlark(pb, p, facing, pose, anim, flash),
        MinibossKind::PastryWheel => draw_pastry_wheel(pb, p, facing, pose, anim, flash),
        MinibossKind::SaltHen => draw_salt_hen(pb, p, facing, pose, anim, flash),
        MinibossKind::PotBrush => draw_pot_brush(pb, p, facing, pose, anim, flash),
        MinibossKind::DosePump => draw_dose_pump(pb, p, facing, pose, anim, flash),
        MinibossKind::PetriMother => draw_petri_mother(pb, p, facing, pose, anim, flash),
        MinibossKind::CageCounter => draw_cage_counter(pb, p, facing, pose, anim, flash),
        MinibossKind::RotorTechnician => draw_rotor_technician(pb, p, facing, pose, anim, flash),
        MinibossKind::SudsBailiff => draw_suds_bailiff(pb, p, facing, pose, anim, flash),
        MinibossKind::WringerBoy => draw_wringer_boy(pb, p, facing, pose, anim, flash),
        MinibossKind::QuoinKey => draw_quoin_key(pb, p, facing, pose, anim, flash),
        MinibossKind::CarbonTwin => draw_carbon_twin(pb, p, facing, pose, anim, flash),
        MinibossKind::LatePlate => draw_late_plate(pb, p, facing, pose, anim, flash),
        MinibossKind::TarpaulinPup => draw_tarpaulin_pup(pb, p, facing, pose, anim, flash),
        MinibossKind::AxleNut => draw_axle_nut(pb, p, facing, pose, anim, flash),
        MinibossKind::ChainDog => draw_chain_dog(pb, p, facing, pose, anim, flash),
        MinibossKind::RoadAtlas => draw_road_atlas(pb, p, facing, pose, anim, flash),
    }

    // Outer radial HP ring framed cleanly outside the ~32 px character silhouette
    // (hidden on Death so the collapsed wreckage & chest pointer read cleanly).
    if e.alive() && e.max_hp > 0.0 {
        let frac = (e.hp / e.max_hp).clamp(0.0, 1.0);
        let r = 16.5;
        let segs = 24;
        let lit = (frac * segs as f32).ceil() as i32;
        let ring_col = match pose {
            Pose::Entrance(_) => (255, 232, 134),
            Pose::Tell(_) => (255, 212, 86),
            Pose::Strike(_) => (255, 92, 74),
            Pose::Recover(_) => (136, 255, 198),
            _ => accent,
        };
        for i in 0..segs {
            let a = -FRAC_PI_2 + (i as f32 / segs as f32) * TAU;
            let px = (p.x + a.cos() * r).round() as i32;
            let py = (p.y + a.sin() * r).round() as i32;
            if i < lit {
                blend_px(pb, px, py, ring_col, 0.86);
            } else {
                blend_px(pb, px, py, (22, 24, 32), 0.52);
            }
        }
    }
}

// ============================================================================
// 1. CORKSCREW — "The drain has grown a tooth."
//    Massive helical brass auger (~34 px) ridden by a muscular pink sewer worm
// ============================================================================

fn draw_corkscrew(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let brass_spec: Rgb = (255, 248, 192);
    let brass_hi: Rgb = (255, 214, 102);
    let brass_mid: Rgb = (204, 148, 48);
    let brass_lo: Rgb = (114, 72, 20);
    let worm_hi: Rgb = (255, 186, 206);
    let worm_mid: Rgb = (224, 114, 148);
    let worm_lo: Rgb = (138, 52, 82);
    let chip_col: Rgb = (146, 152, 162);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.0, 15.0, 7.5, 0.50);

        // Cracked floor scar + flying brass shards early in death.
        draw_shaded_ellipse(pb, center.x, center.y + 2.0, 7.5, 4.0, (44, 46, 54), (26, 28, 34), INK, 0.0);

        // Snapped brass auger halves flying apart left & right!
        let rear_off = -f * (5.0 + burst * 6.5) + p * (burst * 5.0) - Vec2::new(0.0, bounce);
        let front_off = f * (4.0 + burst * 8.0) - p * (burst * 4.5) - Vec2::new(0.0, bounce * 0.7);
        draw_tapered_capsule(
            pb,
            center + rear_off - f * 6.0,
            center + rear_off + f * 3.0,
            4.2,
            3.2,
            brass_hi,
            brass_mid,
            brass_lo,
            flash,
        );
        draw_tapered_capsule(
            pb,
            center + front_off - f * 2.0,
            center + front_off + f * 7.5,
            2.9,
            0.9,
            brass_spec,
            brass_mid,
            brass_lo,
            flash,
        );
        // Jagged snap sparks early in collapse.
        if t < 0.45 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.45, brass_spec);
        }

        // Pink worm uncoiling into a dramatic C-curve framing the golden chest spot!
        for seg in 0..7 {
            let s = seg as f32 / 6.0;
            let ang = -0.7 + s * (2.6 + burst * 0.8);
            let rad = 7.5 + burst * 3.0;
            let pos = center + f.rotate(ang) * rad + Vec2::new(0.0, -1.0 + (1.0 - t) * -2.0);
            let r = if seg == 6 { 3.6 } else { 2.6 + (s * PI).sin() * 1.1 };
            draw_shaded_disc(
                pb,
                pos.x,
                pos.y,
                r,
                mix(worm_hi, (216, 178, 190), burst * 0.5),
                mix(worm_mid, (164, 108, 128), burst * 0.5),
                worm_lo,
                flash,
            );
            if seg == 6 {
                draw_eye(pb, pos + f * 0.8 + p * 1.3, 1.4, (240, 232, 232), INK, f, EyeExpr::Dead, flash);
                draw_eye(pb, pos + f * 0.8 - p * 1.3, 1.4, (240, 232, 232), INK, f, EyeExpr::Dead, flash);
            }
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    // Entrance burst out of a jagged floor breach!
    let (ent_rise, ent_breach) = if let Pose::Entrance(t) = pose {
        let ease = 1.0 - (1.0 - t).powi(3);
        ((1.0 - ease) * 14.0, (1.0 - t).clamp(0.0, 1.0))
    } else {
        (0.0, 0.0)
    };

    let (spin, recoil, lift, wiggle, pitch_down) = match pose {
        Pose::Entrance(t) => (t * 30.0, -2.0 * (1.0 - t), ent_rise, (t * 16.0).sin() * 1.4, 0.0),
        Pose::Idle => (anim * 1.8, 0.0, (anim * 2.2).sin() * 0.6, (anim * 2.5).sin() * 1.0, 0.0),
        Pose::Approach => (anim * 9.5, (anim * 9.5).sin() * 0.9, (anim * 9.5).cos() * 0.6, (anim * 7.5).sin() * 2.1, 0.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (t * 36.0, -5.2 * c + q * 0.7, -2.4 * c, q * 1.5, -0.15 * c)
        }
        Pose::Strike(t) => {
            let lunge = (t * PI).sin();
            (t * 62.0, 5.4 * lunge, 0.6, (t * 32.0).sin() * 0.6, 0.12)
        }
        Pose::Recover(t) => (0.0, 3.2 * (1.0 - t * 0.35), 2.2, (t * 9.0).sin() * 1.1, 0.45),
        Pose::Retreat(t) => (-t * 18.0, -3.2, -0.4, (anim * 5.0).sin() * 1.2, -0.08),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(0.0, lift);

    // Floor shadow (tight & dark under the heavy brass body).
    draw_shadow(pb, base.x + 1.5, center.y + 6.0, 15.5, 6.8, 0.56);

    // Entrance jagged floor breach + flying masonry chunks!
    if let Pose::Entrance(t) = pose {
        draw_shaded_ellipse(
            pb,
            center.x,
            center.y + 4.0,
            11.0 * ent_breach + 3.0,
            5.5 * ent_breach + 1.5,
            (54, 58, 66),
            (28, 30, 38),
            INK,
            0.0,
        );
        draw_entrance_ring(pb, center, t, brass_hi);
        for k in 0..6 {
            let a = k as f32 * (TAU / 6.0) + 0.4;
            let dist = 5.0 + t * 13.0;
            let arc_y = -t * (1.0 - t) * 18.0;
            let q = center + Vec2::new(a.cos() * dist, a.sin() * dist * 0.6 + arc_y);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, chip_col);
            plot_px(pb, q.x.round() as i32 + 1, q.y.round() as i32, brass_hi);
        }
    }

    // Jammed floor crater under auger tip during Recover!
    if matches!(pose, Pose::Recover(_)) {
        let hole = base + f * 13.5 + Vec2::new(0.0, 4.2);
        draw_shaded_ellipse(pb, hole.x, hole.y, 6.2, 3.2, (62, 66, 74), (24, 24, 30), INK, 0.0);
    }

    // 1. Heavy industrial brass rear drive collar + helical tapered auger bit (~30 px!).
    let tail = base - f * 12.5 + Vec2::new(0.0, -pitch_down * 4.5);
    let tip = base + f * 16.5 + Vec2::new(0.0, pitch_down * 5.5);

    // Riveted brass collar ring at the tail.
    draw_shaded_disc(pb, tail.x, tail.y + 1.2, 5.4, brass_hi, brass_mid, brass_lo, flash);
    draw_shaded_disc(pb, (tail + f * 2.2).x, (tail + f * 2.2).y + 1.0, 4.6, brass_spec, brass_mid, brass_lo, flash);

    // Core tapered steel-brass drill cone.
    draw_tapered_capsule(
        pb,
        tail + f * 2.0 + Vec2::new(0.0, 1.2),
        tip + Vec2::new(0.0, 1.2),
        4.4,
        1.1,
        brass_hi,
        brass_mid,
        brass_lo,
        flash,
    );

    // 8 animated helical spiral flute ridges wrapping & scrolling along the drill shaft!
    let flutes = 8;
    for i in 0..flutes {
        let u = (i as f32 + (spin / TAU).fract()) / flutes as f32;
        if u > 0.98 {
            continue;
        }
        let pos = tail + f * 2.0 + (tip - (tail + f * 2.0)) * u + Vec2::new(0.0, 1.2);
        let rad = (4.8 * (1.0 - u * 0.72)).max(1.3);
        let a = pos - p * rad - f * 1.1;
        let b = pos + p * rad + f * 1.1;
        let gleam = (u * 7.0 - spin).sin() * 0.5 + 0.5;
        let hi_c = mix(brass_hi, brass_spec, gleam);
        draw_tapered_capsule(pb, a, b, 1.15, 0.95, hi_c, brass_mid, brass_lo, flash);
    }
    // Blinding razor-sharp auger drill tip.
    let tip_p = tip + Vec2::new(0.0, 1.0);
    draw_shaded_disc(pb, tip_p.x, tip_p.y, 1.6, (255, 255, 240), brass_spec, brass_mid, flash);

    // Approach: corkscrewing rubble chips & dust kicking back from the nose!
    if matches!(pose, Pose::Approach) {
        for k in 0..4 {
            let cyc = (anim * 1.3 + k as f32 * 0.25).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let q = tip_p - f * (cyc * 14.0) + p * (side * (3.0 + cyc * 6.5))
                + Vec2::new(0.0, -cyc * (1.0 - cyc) * 10.0 + 1.5);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 2 == 0 { chip_col } else { brass_hi });
            if cyc < 0.6 {
                blend_px(pb, q.x.round() as i32 - 1, q.y.round() as i32, (98, 102, 112), 0.75);
            }
        }
    }

    // Tell: high-RPM revving spark shower off the brass flutes!
    if let Pose::Tell(t) = pose {
        for k in 0..5 {
            let phase = (t * 4.5 + k as f32 * 0.2).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let sp = tip_p - f * (phase * 7.0) + p * (side * (2.5 + phase * 7.0))
                + Vec2::new(0.0, -phase * 4.0);
            blend_px(pb, sp.x.round() as i32, sp.y.round() as i32, brass_spec, 1.0 - phase * 0.5);
        }
    }

    // Strike: golden helical smear ribbons + nose impact flash + rubble spray!
    if let Pose::Strike(t) = pose {
        let ang = f.y.atan2(f.x);
        draw_smear_arc(pb, base + f * 5.0, ang - 0.55, ang + 0.55, 7.0, 14.0, brass_hi, (255, 252, 224), 0.65 * (1.0 - t * 0.4));
        // Nose impact cross flash!
        let fx = tip_p.x.round() as i32;
        let fy = tip_p.y.round() as i32;
        for d in -4..=4 {
            blend_px(pb, fx + d, fy, (255, 252, 214), 0.90);
            blend_px(pb, fx, fy + d, (255, 230, 134), 0.85);
        }
        for k in 0..6 {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let fly = ((t * 3.0 + k as f32 * 0.18).fract()) * 14.0;
            let q = tip_p - f * (fly * 0.6) + p * (side * (3.5 + fly * 0.75)) + Vec2::new(0.0, -fly * 0.35);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 3 == 0 { brass_spec } else { chip_col });
        }
    }

    // 2. Muscular, plump segmented fleshy pink sewer worm riding the auger spine!
    let head_ext = match pose {
        Pose::Recover(_) => 6.0, // flops way forward & exposed!
        Pose::Tell(t) => -3.2 * tell_coil(t), // coils back to strike!
        Pose::Strike(t) => 3.5 * (t * PI).sin(),
        _ => 0.0,
    };
    let arch_boost = match pose {
        Pose::Tell(t) => 3.2 * tell_coil(t),
        Pose::Recover(_) => -1.5,
        _ => 0.0,
    };
    let segs = 7;
    let mut head_pos = base;
    for seg in 0..segs {
        let s = seg as f32 / (segs - 1) as f32;
        let u = -9.0 + s * (16.5 + head_ext);
        let sway = ((s * 4.5 - anim * 3.6).sin() * wiggle) * (1.0 - s * 0.3);
        let arch = -3.4 - (s * PI).sin() * (2.6 + arch_boost) + pitch_down * s * 4.5;
        let pos = base + f * u + p * sway + Vec2::new(0.0, arch);
        let r = if seg == segs - 1 {
            4.6 // big expressive worm head!
        } else {
            2.9 + (s * PI).sin() * 1.5
        };
        // Alternating subtle fleshy rib rings so segments read cleanly at 2× zoom-out.
        let hi_c = if seg % 2 == 0 { worm_hi } else { mix(worm_hi, worm_mid, 0.25) };
        draw_shaded_disc(pb, pos.x, pos.y, r, hi_c, worm_mid, worm_lo, flash);
        if seg == segs - 1 {
            head_pos = pos;
        }
    }

    // Expressive worm face: menacing twin eyes + serrated ivory-pink mandibles!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_l = head_pos + f * 1.6 + p * 2.1 + Vec2::new(0.0, -0.8);
    let eye_r = head_pos + f * 1.6 - p * 2.1 + Vec2::new(0.0, -0.8);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 96),
        _ => (255, 248, 236),
    };
    draw_eye(pb, eye_l, 1.6, eye_col, INK, f, expr, flash);
    draw_eye(pb, eye_r, 1.6, eye_col, INK, f, expr, flash);

    // Chattering ivory mandibles under the snout!
    let jaw_open = match pose {
        Pose::Tell(t) => 1.2 + tell_coil(t) * 1.5,
        Pose::Strike(_) => 2.6,
        Pose::Idle => 0.8 + (anim * 4.0).sin().abs() * 0.7,
        _ => 1.0,
    };
    for &side in &[-1.0_f32, 1.0] {
        let m0 = head_pos + f * 3.4 + p * (side * 1.6) + Vec2::new(0.0, 1.2);
        let m1 = head_pos + f * (5.6 + jaw_open * 0.4) + p * (side * (0.7 + jaw_open * 0.45)) + Vec2::new(0.0, 1.6);
        draw_tapered_capsule(pb, m0, m1, 1.1, 0.5, (255, 246, 222), (224, 206, 174), worm_lo, flash);
    }

    // Retreat: annoyed pink steam puffs drifting off the back!
    if let Pose::Retreat(t) = pose {
        for k in 0..2 {
            let ph = (t * 2.8 + k as f32 * 0.5).fract();
            let puff = head_pos - f * (2.0 + ph * 4.0) + Vec2::new(0.0, -4.5 - ph * 6.0);
            blend_px(pb, puff.x.round() as i32, puff.y.round() as i32, (242, 224, 232), (1.0 - ph) * 0.8);
            blend_px(pb, puff.x.round() as i32 + 1, puff.y.round() as i32, (242, 224, 232), (1.0 - ph) * 0.6);
        }
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, head_pos + Vec2::new(0.0, -6.5), 6.2, t);
        draw_recover_beacon(pb, head_pos, t);
    }
    draw_hurt_burst(pb, base, f, flash, brass_spec);
}

// ============================================================================
// 2. SISTER SNIP — "She trims loose ends."
//    Tall Victorian tailor's shears (~36 px) stalking on razor blade tips
// ============================================================================

fn draw_sister_snip(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let steel_spec: Rgb = (255, 255, 255);
    let steel_hi: Rgb = (232, 242, 255);
    let steel_mid: Rgb = (156, 174, 198);
    let steel_lo: Rgb = (78, 92, 114);
    let grip_hi: Rgb = (198, 152, 228);
    let grip_mid: Rgb = (126, 86, 162);
    let grip_lo: Rgb = (58, 38, 84);
    let brass_hi: Rgb = (255, 234, 140);
    let brass_mid: Rgb = (214, 166, 62);
    let brass_lo: Rgb = (116, 78, 24);
    let ribbon_col: Rgb = (178, 64, 118);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 5.0;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 16.0, 7.5, 0.50);

        // Two unhinged steel blades clattering flat in a crossed X framing the chest spot!
        for &side in &[-1.0_f32, 1.0] {
            let drift = p * (side * (4.5 + burst * 5.5)) - Vec2::new(0.0, bounce * (0.6 + 0.3 * side));
            let bdir = f.rotate(side * (0.28 + burst * 0.45));
            let a = center + drift - bdir * 9.0;
            let b = center + drift + bdir * 12.0;
            draw_tapered_capsule(pb, a, b, 2.4, 0.8, steel_hi, steel_mid, steel_lo, flash);
            // Enamel handle ring attached to rear of each unhinged half.
            let bow = a - bdir * 3.0;
            draw_shaded_disc(pb, bow.x, bow.y, 3.8, grip_hi, grip_mid, grip_lo, flash);
            draw_shaded_disc(pb, bow.x, bow.y, 1.6, (28, 24, 38), (18, 16, 26), INK, 0.0);
        }
        // Popped brass pivot screw rolling away!
        let screw = center + f * (4.0 + burst * 7.0) + p * (2.0 + burst * 5.0) - Vec2::new(0.0, bounce * 1.4);
        draw_shaded_disc(pb, screw.x, screw.y, 2.4, brass_hi, brass_mid, brass_lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, steel_spec);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    // Entrance: descends sharply from above, stabs floor, springs blades wide open!
    let (ent_drop, ent_spread) = if let Pose::Entrance(t) = pose {
        if t < 0.45 {
            let u = t / 0.45;
            ((1.0 - u * u) * -22.0, 0.06)
        } else {
            let u = (t - 0.45) / 0.55;
            let snap = 1.0 - (1.0 - u).powi(3);
            (0.0, 0.06 + snap * 0.55)
        }
    } else {
        (0.0, 0.0)
    };

    // Blade spread, alternating stilt-stride, lift, and blade asymmetry (for stalking gait).
    let (spread, stride, lift, asym) = match pose {
        Pose::Entrance(_) => (ent_spread, 0.0, ent_drop, 0.0),
        Pose::Idle => (0.28 + (anim * 2.2).sin() * 0.09, 0.0, (anim * 2.2).cos() * 0.5, 0.0),
        Pose::Approach => {
            // Alternating scissor-stilt walk: blades open/close + rock left/right!
            let s = (anim * 6.5).sin();
            (0.24 + s.abs() * 0.34, s * 2.4, -s.abs() * 2.0, s * 0.22)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Opens dramatically past 45° wedge with tense quiver at peak!
            (0.26 + 0.62 * c + q * 0.05, -2.5 * c, -3.6 * c, 0.0)
        }
        Pose::Strike(t) => {
            // Razor-fast snap across the first 40% of Strike, then clamped shut with recoil!
            let close = (t / 0.40).clamp(0.0, 1.0);
            let ease = close * close;
            (0.88 * (1.0 - ease) + 0.03, 4.2 * ease, -3.6 * (1.0 - ease) + 0.8 * ease, 0.0)
        }
        Pose::Recover(t) => {
            // Wedged crossed/stuck into the floor wobbling!
            let wob = (t * 16.0).sin() * (1.0 - t * 0.5) * 0.06;
            (-0.06 + wob, 2.6 * (1.0 - t * 0.4), 2.0, wob)
        }
        Pose::Retreat(t) => {
            // Haughty two-snip backward stride!
            let snip = ((t * 6.0).sin().abs()) * 0.36;
            (0.10 + snip, -2.8, -0.6, 0.0)
        }
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let pivot = center + f * stride + jolt + Vec2::new(0.0, lift);

    // Grounded shadow (shrinks slightly when hovering high in Entrance/Tell).
    let sh_scale = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.1);
    draw_shadow(pb, center.x + 1.5, center.y + 6.5, 13.5 * sh_scale, 6.5 * sh_scale, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), steel_hi);
        }
    }

    // Strike: curved white-crimson blade-closure smear arcs sweeping inward!
    if let Pose::Strike(t) = pose {
        let base_ang = f.y.atan2(f.x);
        if t < 0.62 {
            let smear_a = (1.0 - t / 0.62).clamp(0.0, 1.0) * 0.78;
            draw_smear_arc(pb, pivot, base_ang - 0.84, base_ang - spread, 5.0, 17.5, (255, 118, 146), steel_spec, smear_a);
            draw_smear_arc(pb, pivot, base_ang + spread, base_ang + 0.84, 5.0, 17.5, (255, 118, 146), steel_spec, smear_a);
        }
    }

    // Fluttering dark-rose tailor's ribbon tied to the upper finger-bow!
    let rib_anchor = pivot - f * 10.5 - p * 5.2;
    let mut prev_r = rib_anchor;
    for k in 1..=6 {
        let u = k as f32 / 6.0;
        let flutter = if matches!(pose, Pose::Recover(_)) {
            u * 3.5 // drooping limp!
        } else {
            (anim * 6.0 - k as f32 * 0.9).sin() * (1.2 + u * 2.4)
        };
        let next_r = rib_anchor - f * (u * 8.5) + p * flutter + Vec2::new(0.0, if matches!(pose, Pose::Recover(_)) { u * 3.0 } else { 0.0 });
        pb.line(prev_r.x.round() as i32, prev_r.y.round() as i32, next_r.x.round() as i32, next_r.y.round() as i32, apply_flash(ribbon_col, flash));
        prev_r = next_r;
    }

    // 1. Twin ornate plum-enamel finger-bow handle rings behind pivot (`-f`)
    //    which double as Sister Snip's glaring glowing SPECTACLE EYES!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_glow = match pose {
        Pose::Tell(_) => (255, 224, 118),
        Pose::Strike(_) => (255, 124, 142),
        Pose::Recover(_) => (148, 255, 206),
        _ => (232, 246, 255),
    };
    for &side in &[-1.0_f32, 1.0] {
        let s_ang = side * (spread * 0.68 + 0.12) + asym * 0.4;
        let dir = f.rotate(s_ang);
        let bow_c = pivot - dir * 9.5 + p * (side * 3.4);
        // Sculpted enamel shank from pivot to finger ring.
        draw_tapered_capsule(pb, pivot, bow_c, 2.2, 1.6, grip_hi, grip_mid, grip_lo, flash);
        // Chunky enamel finger bow rim.
        draw_shaded_disc(pb, bow_c.x, bow_c.y, 4.6, grip_hi, grip_mid, grip_lo, flash);
        // Hollow interior acting as a creepy staring spectacle eye!
        draw_shaded_disc(pb, bow_c.x, bow_c.y, 2.3, (42, 34, 58), (22, 18, 34), INK, 0.0);
        draw_eye(pb, bow_c, 1.7, eye_glow, INK, f, expr, flash);
    }

    // 2. Twin long, gleaming mirror-steel shear blades projecting forward (`+f`, ~18 px)!
    for &side in &[-1.0_f32, 1.0] {
        let hurt_splay = if flash > 0.1 { side * flash * 0.25 } else { 0.0 };
        let bdir = f.rotate(side * spread + asym + hurt_splay);
        let mid_pt = pivot + bdir * 9.0 + bdir.perp() * (side * 1.4);
        let tip = pivot + bdir * 18.0;
        // Back bevel & razor taper.
        draw_tapered_capsule(pb, pivot, mid_pt, 2.8, 2.2, steel_hi, steel_mid, steel_lo, flash);
        draw_tapered_capsule(pb, mid_pt, tip, 2.1, 0.7, steel_spec, steel_hi, steel_lo, flash);
        // Honed inner cutting-edge catchlight racing toward tip during Tell!
        let glint_u = match pose {
            Pose::Tell(t) => (t * 2.4).fract(),
            _ => (anim * 0.7).fract(),
        };
        let gp = pivot + bdir * (4.0 + glint_u * 12.5);
        plot_px(pb, gp.x.round() as i32, gp.y.round() as i32, steel_spec);
        let gp2 = pivot + bdir * (5.2 + glint_u * 12.5);
        plot_px(pb, gp2.x.round() as i32, gp2.y.round() as i32, steel_spec);

        // Approach: tiny metallic tap-sparks under the planted blade tip!
        if matches!(pose, Pose::Approach) {
            let planted = (side * (anim * 6.5).sin()) > 0.55;
            if planted {
                let tx = tip.x.round() as i32;
                let ty = tip.y.round() as i32;
                plot_px(pb, tx, ty + 1, (255, 244, 174));
                blend_px(pb, tx - 1, ty + 1, steel_hi, 0.8);
                blend_px(pb, tx + 1, ty + 1, steel_hi, 0.8);
            }
        }
    }

    // 3. Heavy ornate brass pivot medallion right at the blade crossing.
    let piv_vib = if matches!(pose, Pose::Recover(_)) {
        p * ((anim * 42.0).sin() * 0.9)
    } else {
        Vec2::ZERO
    };
    let pm = pivot + piv_vib;
    draw_shaded_disc(pb, pm.x, pm.y, 3.4, brass_hi, brass_mid, brass_lo, flash);
    // Crisp screwdriver slot across the brass pivot bolt!
    let s0 = pm - f * 1.8;
    let s1 = pm + f * 1.8;
    pb.line(s0.x.round() as i32, s0.y.round() as i32, s1.x.round() as i32, s1.y.round() as i32, INK);

    // Strike: blinding metallic snap cross-flash + flying snipped thread scraps!
    if let Pose::Strike(t) = pose {
        if (0.28..0.82).contains(&t) {
            let glint = pivot + f * 12.5;
            let gx = glint.x.round() as i32;
            let gy = glint.y.round() as i32;
            let k = 1.0 - ((t - 0.28) / 0.54);
            for d in -5..=5 {
                blend_px(pb, gx + d, gy, steel_spec, k * 0.95);
                blend_px(pb, gx, gy + d, (192, 238, 255), k * 0.90);
            }
            for i in 0..4 {
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let u = (t - 0.28) / 0.54;
                let scr = glint + f * (u * 6.0) + p * (side * (3.0 + u * 8.0));
                plot_px(pb, scr.x.round() as i32, scr.y.round() as i32, ribbon_col);
            }
        }
    }

    if let Pose::Recover(t) = pose {
        // Punish cue sits BEHIND the pivot ("Walk behind the pivot")!
        let back_spot = pivot - f * 5.0;
        draw_dizzy_stars(pb, pivot + Vec2::new(0.0, -7.5), 6.5, t);
        draw_recover_beacon(pb, back_spot, t);
    }
    draw_hurt_burst(pb, pivot, f, flash, steel_spec);
}

// ============================================================================
// 3. BOBBIN — "It learned to sew in circles."
//    Manic spinning wooden textile spindle rat (~30 px tall) + whipping cord
// ============================================================================

fn draw_bobbin(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let wood_hi: Rgb = (238, 188, 122);
    let wood_mid: Rgb = (176, 118, 62);
    let wood_lo: Rgb = (96, 58, 28);
    let thread_spec: Rgb = (255, 198, 218);
    let thread_hi: Rgb = (248, 122, 162);
    let thread_mid: Rgb = (196, 56, 102);
    let thread_lo: Rgb = (112, 24, 54);
    let brass_hi: Rgb = (255, 238, 152);
    let brass_mid: Rgb = (218, 170, 68);
    let brass_lo: Rgb = (118, 82, 26);
    let fur_hi: Rgb = (252, 246, 244);
    let fur_mid: Rgb = (204, 192, 196);
    let fur_lo: Rgb = (116, 106, 118);
    let ear_hi: Rgb = (255, 194, 210);
    let silver: Rgb = (234, 244, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let roll = burst * 7.5;
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 15.0, 7.2, 0.50);

        // Unspooling crimson thread ribbon spiraling cleanly around the golden chest center!
        let loops = 26;
        let mut prev_q = center + Vec2::new(6.0, 2.0);
        for k in 1..=loops {
            let u = k as f32 / loops as f32;
            if u > 0.25 + burst * 0.75 {
                break;
            }
            let ang = u * TAU * 1.45;
            let rad = 5.5 + u * (5.5 + burst * 4.0);
            let q = center + Vec2::new(ang.cos() * rad, ang.sin() * rad * 0.68 + 1.5);
            pb.line(prev_q.x.round() as i32, prev_q.y.round() as i32, q.x.round() as i32, q.y.round() as i32, apply_flash(thread_hi, flash));
            prev_q = q;
        }
        // Gleaming silver needle pointing right back toward the chest center!
        let n_tail = center + Vec2::new(11.0, 4.0);
        let n_tip = center + Vec2::new(4.5, 1.5);
        draw_tapered_capsule(pb, n_tail, n_tip, 1.0, 0.4, (255, 255, 255), silver, (112, 126, 146), flash);

        // Toppled split wooden spool rolling away to the upper-left!
        let sp = center - f * (3.0 + roll * 0.7) + Vec2::new(-roll * 0.4, -2.5 - bounce);
        draw_shaded_disc(pb, sp.x - 4.0, sp.y + 1.0, 5.2, wood_hi, wood_mid, wood_lo, flash);
        draw_shaded_disc(pb, sp.x + 1.5, sp.y + 1.5, 4.6, thread_hi, thread_mid, thread_lo, flash);
        draw_shaded_disc(pb, sp.x + 5.5, sp.y + 2.0, 4.5, wood_hi, wood_mid, wood_lo, flash);
        // Popped brass spindle pin.
        let pin = center + Vec2::new(6.5 + burst * 4.0, -5.0 - bounce * 0.6);
        draw_shaded_disc(pb, pin.x, pin.y, 2.2, brass_hi, brass_mid, brass_lo, flash);
        if t < 0.40 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.40, thread_spec);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    // Entrance: rappels down a vertical crimson thread from above, landing spinning!
    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.48 {
            let u = t / 0.48;
            (1.0 - u * u) * -24.0
        } else {
            let u = (t - 0.48) / 0.52;
            -(u * PI).sin() * 2.5 * (1.0 - u)
        }
    } else {
        0.0
    };

    let (spin, lean_x, bob, tilt_ang, cord_whip) = match pose {
        Pose::Entrance(t) => (t * 42.0, (t * 24.0).sin() * 1.2, ent_drop, 0.0, 1.5),
        Pose::Idle => (anim * 2.2, (anim * 2.4).sin() * 0.9, (anim * 2.6).cos() * 0.7, (anim * 2.4).sin() * 0.06, 0.5),
        Pose::Approach => {
            let w = (anim * 10.5).sin();
            (anim * 15.0, w * 1.8, (anim * 10.5).cos() * 1.1 - 0.6, w * 0.14, 1.6)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Leans back away from aim winding the crimson drum to a blur!
            (t * 44.0, -f.x * 3.8 * c + q * 0.9, -2.0 * c, -f.x * 0.18 * c, 0.9)
        }
        Pose::Strike(t) => {
            // Sweeps wildly through the 90° swing with swirling ribbons!
            let sw = (-FRAC_PI_4 + t * FRAC_PI_2).sin();
            (t * 68.0, f.x * 3.2 + sw * 2.4, -0.5, sw * 0.24, 2.2)
        }
        Pose::Recover(t) => {
            // Tipped sideways like a dying spinning top, tangled in crimson loops!
            let wob = (t * 10.0).sin() * (1.0 - t * 0.35);
            (t * 3.0, 3.6 + wob * 1.5, 1.8, 0.42 + wob * 0.10, 0.2)
        }
        Pose::Retreat(t) => (-t * 26.0, -f.x * 2.6, -(t * 14.0).sin().abs() * 1.2, -f.x * 0.12, 1.1),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + jolt + Vec2::new(lean_x, bob);

    // Grounded shadow beneath the top.
    let sh_s = (1.0 - (-bob).max(0.0) * 0.025).clamp(0.42, 1.1);
    draw_shadow(pb, center.x + 1.5, center.y + 8.5, 11.5 * sh_s, 5.5 * sh_s, 0.54);

    // Entrance vertical rappel thread + landing thread-ring burst!
    if let Pose::Entrance(t) = pose {
        if t < 0.65 {
            let top_y = (base.y - 28.0).max(0.0);
            pb.line(base.x.round() as i32, top_y.round() as i32, base.x.round() as i32, (base.y - 10.0).round() as i32, thread_spec);
        }
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), thread_hi);
        }
    }

    // 1. Long dynamic whipping crimson cord + gleaming silver needle trailing behind `-f`!
    let cord_len = 16.0;
    let segs = 8;
    let mut prev_c = base - f * 6.0 + Vec2::new(0.0, 2.0);
    for k in 1..=segs {
        let u = k as f32 / segs as f32;
        let wave = (anim * 7.5 - u * 5.2).sin() * (2.6 + u * 4.2) * cord_whip;
        let q = base - f * (6.0 + u * cord_len) + p * wave + Vec2::new(0.0, 2.0 - (u * PI).sin() * 1.8);
        let col = if k % 2 == 0 { thread_hi } else { thread_spec };
        pb.line(prev_c.x.round() as i32, prev_c.y.round() as i32, q.x.round() as i32, q.y.round() as i32, apply_flash(col, flash));
        // Second pixel thickness near spool so the cord survives 2× zoom-out!
        if k <= 5 {
            pb.line(prev_c.x.round() as i32, prev_c.y.round() as i32 + 1, q.x.round() as i32, q.y.round() as i32 + 1, apply_flash(thread_mid, flash));
        }
        prev_c = q;
    }
    // Gleaming silver sewing needle tied to the whipping tail end!
    let ndir = safe_dir(prev_c - base, -f);
    let n_tip = prev_c + ndir * 4.5;
    pb.line(prev_c.x.round() as i32, prev_c.y.round() as i32, n_tip.x.round() as i32, n_tip.y.round() as i32, silver);
    plot_px(pb, n_tip.x.round() as i32, n_tip.y.round() as i32, (255, 255, 255));

    // Tiny blurring pink rat feet pattering under the wooden base!
    let foot_phase = anim * 12.0;
    for &side in &[-1.0_f32, 1.0] {
        let fx = base.x + side * 4.2 + (foot_phase + side).sin() * 2.0;
        let fy = base.y + 8.5 + (foot_phase + side).cos() * 1.2;
        draw_shaded_disc(pb, fx, fy, 1.9, ear_hi, (214, 136, 158), (112, 58, 76), flash);
    }

    // 2. Bottom flared turned-wood bobbin flange (big chunky ellipse!).
    let up = Vec2::new(tilt_ang.sin(), -tilt_ang.cos());
    let bot_c = base - up * 6.8;
    let top_c = base + up * 7.4;
    draw_shaded_ellipse(pb, bot_c.x, bot_c.y, 9.2, 4.4, wood_hi, wood_mid, wood_lo, flash);

    // 3. Fast, lush, shaded crimson-rose wound thread spindle drum (single tight pixel loop!)
    let drum_h = 11.5_f32;
    let drum_r = 7.4_f32;
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let dy_min = (base.y - 8.5).floor().max(0.0) as i32;
    let dy_max = (base.y + 7.5).ceil().min((bh - 1) as f32) as i32;
    let dx_min = (base.x - 10.5).floor().max(0.0) as i32;
    let dx_max = (base.x + 10.5).ceil().min((bw - 1) as f32) as i32;
    let ink_c = apply_flash(INK, flash * 0.35);
    let side_v = Vec2::new(-up.y, up.x);
    for y in dy_min..=dy_max {
        let vy = y as f32 + 0.5 - base.y;
        for x in dx_min..=dx_max {
            let vx = x as f32 + 0.5 - base.x;
            let au = vx * up.x + vy * up.y;
            let av = vx * side_v.x + vy * side_v.y;
            if au.abs() > drum_h * 0.55 {
                continue;
            }
            // Slight plump barrel curve to the wound thread mass!
            let barrel = drum_r + (1.0 - (au / (drum_h * 0.55)).powi(2)) * 1.4;
            if av.abs() > barrel + 1.0 {
                continue;
            }
            let c = if av.abs() > barrel - 0.15 {
                ink_c
            } else {
                // Horizontal wound thread grooves + fast diagonal spinning gleam!
                let groove = ((au * 3.0).sin() * 0.5 + 0.5) * 0.25;
                let spin_streak = ((av * 0.65 - spin + au * 0.35).sin() * 0.5 + 0.5).powi(3);
                let lit = (0.52 - (av / barrel) * 0.42 + groove + spin_streak * 0.38).clamp(0.0, 1.0);
                let base_c = if lit > 0.78 {
                    mix(thread_hi, thread_spec, (lit - 0.78) / 0.22)
                } else if lit > 0.46 {
                    mix(thread_mid, thread_hi, (lit - 0.46) / 0.32)
                } else {
                    mix(thread_lo, thread_mid, lit / 0.46)
                };
                apply_flash(base_c, flash)
            };
            pb.plot(x, y, c);
        }
    }

    // Strike: swirling crimson-white motion-smear ribbons orbiting the spinning drum!
    if let Pose::Strike(t) = pose {
        let sw = t * TAU * 2.5;
        draw_smear_arc(pb, base, sw, sw + 1.6, 8.2, 11.8, thread_hi, thread_spec, 0.78);
        draw_smear_arc(pb, base, sw + PI, sw + PI + 1.4, 8.2, 11.5, thread_mid, thread_hi, 0.68);
    }

    // 4. Expressive spindle-rat face peeking aggressively out along `+f`!
    let face_c = base + f * 6.5 + up * 1.2;
    let snout = face_c + f * 3.4;
    // Fluffy round cheeks + pointed snout + bright pink twitching nose!
    draw_shaded_disc(pb, face_c.x, face_c.y, 4.4, fur_hi, fur_mid, fur_lo, flash);
    draw_tapered_capsule(pb, face_c, snout, 3.0, 1.4, fur_hi, fur_mid, fur_lo, flash);
    let nose = snout + f * 1.4 + Vec2::new(0.0, (anim * 14.0).sin() * 0.3);
    draw_shaded_disc(pb, nose.x, nose.y, 1.2, (255, 198, 218), (252, 120, 158), thread_lo, flash);
    // Crisp twitching whiskers!
    for &side in &[-1.0_f32, 1.0] {
        let w0 = snout + p * (side * 1.2);
        let w1 = snout + f * 1.5 + p * (side * 4.5);
        pb.line(w0.x.round() as i32, w0.y.round() as i32, w1.x.round() as i32, w1.y.round() as i32, (242, 236, 240));
    }
    // Big round rose-lined rat ears (pin back during Tell/Hurt, droop during Recover)!
    let ear_pin = match pose {
        Pose::Tell(t) => -2.2 * tell_coil(t),
        Pose::Recover(_) => -1.5,
        _ => 0.0,
    };
    let ear_droop = if matches!(pose, Pose::Recover(_)) { 2.2 } else { 0.0 };
    for &side in &[-1.0_f32, 1.0] {
        let ear = base + f * (3.6 + ear_pin) + p * (side * 6.2) + up * (4.4 - ear_droop);
        draw_shaded_disc(pb, ear.x, ear.y, 3.1, fur_hi, fur_mid, fur_lo, flash);
        draw_shaded_disc(pb, ear.x, ear.y, 1.6, ear_hi, (226, 128, 158), thread_lo, flash);
    }
    // Manic glossy rat eyes!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 108),
        _ => (255, 250, 244),
    };
    draw_eye(pb, face_c + f * 1.2 + p * 2.1 + Vec2::new(0.0, -1.0), 1.5, eye_col, INK, f, expr, flash);
    draw_eye(pb, face_c + f * 1.2 - p * 2.1 + Vec2::new(0.0, -1.0), 1.5, eye_col, INK, f, expr, flash);

    // 5. Turned-wood top flange cap & gleaming brass spindle finial on top!
    draw_shaded_ellipse(pb, top_c.x, top_c.y, 7.6, 3.6, wood_hi, wood_mid, wood_lo, flash);
    draw_shaded_disc(pb, top_c.x, top_c.y - 1.5, 2.7, brass_hi, brass_mid, brass_lo, flash);
    plot_px(pb, top_c.x.round() as i32 - 1, top_c.y.round() as i32 - 2, (255, 255, 232));

    // Recover: messy tangled crimson thread loops wrapped around the tipped bobbin!
    if let Pose::Recover(t) = pose {
        for k in 0..3 {
            let off = (k as f32 - 1.0) * 3.5;
            let a = base + Vec2::new(-8.5, off);
            let b = base + Vec2::new(8.5, -off * 0.7);
            pb.line(a.x.round() as i32, a.y.round() as i32, b.x.round() as i32, b.y.round() as i32, thread_spec);
        }
        draw_dizzy_stars(pb, top_c + Vec2::new(0.0, -6.5), 6.5, t);
        draw_recover_beacon(pb, top_c, t);
    }
    draw_hurt_burst(pb, base, f, flash, thread_spec);
}

// ============================================================================
// 4. BELL DIVER — "Listen for the missing note."
//    Heavy riveted verdigris diving-bell beetle (~32 px) with chipped brass rim
//    STRICTLY aligned to `gap_dir` so the ring's missing note comes from the chip!
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_bell_diver(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    chip_dir: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let bell_spec: Rgb = (212, 255, 244);
    let bell_hi: Rgb = (146, 228, 210);
    let bell_mid: Rgb = (74, 162, 146);
    let bell_lo: Rgb = (34, 84, 82);
    let brass_spec: Rgb = (255, 248, 196);
    let brass_hi: Rgb = (248, 212, 114);
    let brass_mid: Rgb = (192, 144, 56);
    let brass_lo: Rgb = (104, 72, 24);
    let leg_hi: Rgb = (96, 108, 120);
    let leg_lo: Rgb = (30, 36, 44);
    let teal_glow: Rgb = (142, 252, 236);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 16.0, 8.0, 0.52);

        // Cracked verdigris dome split wide open down the chipped fissure!
        let sep = chip_dir.perp() * (3.5 + burst * 5.8);
        let h1 = center - sep - Vec2::new(0.0, bounce * 0.8);
        let h2 = center + sep + Vec2::new(0.0, 1.5 - bounce * 0.5);
        draw_shaded_disc(pb, h1.x, h1.y, 7.6, bell_hi, bell_mid, bell_lo, flash);
        draw_shaded_disc(pb, h2.x, h2.y, 6.4, bell_mid, bell_lo, (22, 44, 46), flash);
        // Broken brass rim arcs on the cracked halves.
        draw_capsule(pb, h1 - chip_dir * 4.0, h1 + chip_dir * 3.0, 1.8, brass_hi, brass_mid, brass_lo, flash);
        // Heavy brass clapper rolling out pointing at the golden chest center!
        let clap = center + chip_dir * (5.5 + burst * 4.5) - Vec2::new(0.0, bounce * 1.1);
        draw_shaded_disc(pb, clap.x, clap.y, 3.1, brass_spec, brass_hi, brass_lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, teal_glow);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    // Entrance: heavy diving-bell drop with booming teal-gold acoustic shockwave!
    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.45 {
            let u = t / 0.45;
            (1.0 - u * u) * -24.0
        } else {
            let u = (t - 0.45) / 0.55;
            ((u * PI * 2.0).sin() * (1.0 - u)) * 2.5
        }
    } else {
        0.0
    };

    // Hop gait on Approach: rises off floor, slams down emitting little sound rings!
    let hop_cycle = (anim * 1.15).fract();
    let hop_lift = if matches!(pose, Pose::Approach) {
        if hop_cycle < 0.65 {
            -(hop_cycle / 0.65 * PI).sin() * 5.8
        } else {
            0.8
        }
    } else {
        0.0
    };

    let (lift, rattle, tilt, squash) = match pose {
        Pose::Entrance(t) => (ent_drop, if t >= 0.45 { ((1.0 - t) * 40.0).sin() * 1.6 } else { 0.0 }, 0.0, if (0.45..0.60).contains(&t) { 1.4 } else { 0.0 }),
        Pose::Idle => ((anim * 2.4).sin() * 0.7, 0.0, 0.0, 0.0),
        Pose::Approach => (hop_lift, if hop_cycle >= 0.65 { ((hop_cycle - 0.65) * 32.0).sin() * 1.0 } else { 0.0 }, (anim * 3.5).sin() * 1.2, if hop_cycle >= 0.65 { 0.9 } else { 0.0 }),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-7.2 * c, q * 1.4, 0.0, -0.8 * c)
        }
        Pose::Strike(t) => {
            // Slams violently flat onto the floor and rings with double-edge vibration!
            let ring = (1.0 - t) * (t * 58.0).sin() * 2.2;
            (1.8, ring, 0.0, 1.8 * (1.0 - t * 0.5))
        }
        Pose::Recover(t) => {
            // Tipped awkwardly sideways, muted & rattling!
            let wob = (t * 9.0).sin() * 0.8;
            (1.2, wob * 0.5, 4.2 * (1.0 - t * 0.25), 0.0)
        }
        Pose::Retreat(t) => (-((t * 9.0).sin().abs()) * 3.6, 0.0, -1.6, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + jolt + Vec2::new(rattle + tilt * 0.65, lift);

    // Shadow shrinks & softens when bell hoists high in Tell / mid-hop.
    let sh_s = (1.0 - (-lift).max(0.0) * 0.032).clamp(0.45, 1.15);
    draw_shadow(pb, center.x + 1.5, center.y + 7.5, 13.5 * sh_s, 6.8 * sh_s, 0.54);

    // Entrance & Approach landing sound rings!
    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            let u = ((t - 0.44) / 0.56).clamp(0.0, 1.0);
            draw_entrance_ring(pb, center, u, teal_glow);
        }
    }
    if matches!(pose, Pose::Approach) && hop_cycle >= 0.62 {
        let u = ((hop_cycle - 0.62) / 0.38).clamp(0.0, 1.0);
        let rx = 10.0 + u * 11.0;
        let ry = 5.5 + u * 6.0;
        for k in 0..24 {
            let ang = k as f32 * (TAU / 24.0);
            let d = Vec2::from_angle(ang);
            // Even the approach ring skips the chipped rim notch!
            if d.dot(chip_dir) > 0.84 {
                continue;
            }
            let q = center + Vec2::new(d.x * rx, d.y * ry + 5.0);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, teal_glow, (1.0 - u) * 0.88);
        }
    }

    // 1. Six articulated iron-and-brass beetle legs (tuck cleanly with the bell
    //    when airborne in Entrance/Tell, wave helplessly on one side in Recover!).
    let airborne_tuck = ((-lift - 1.5).max(0.0) / 16.0).clamp(0.0, 1.0);
    for i in 0..3 {
        let side_ang = -0.85 + i as f32 * 0.85;
        let step = if matches!(pose, Pose::Approach) {
            (anim * 7.2 + i as f32 * 2.0).sin() * 2.2
        } else if matches!(pose, Pose::Recover(_)) {
            (anim * 16.0 + i as f32 * 1.7).sin() * 2.5
        } else {
            0.0
        };
        for &s in &[-1.0_f32, 1.0] {
            let ldir = f.rotate(side_ang * s + s * FRAC_PI_2 * 0.42);
            let hip = body + ldir * 7.5 + Vec2::new(0.0, 3.6);
            let foot = if matches!(pose, Pose::Recover(_)) && s < 0.0 {
                // Waving in the air on the lifted side!
                body + ldir * 13.5 + Vec2::new(0.0, -1.5 + step)
            } else {
                let reach = 12.6 * (1.0 - airborne_tuck * 0.25) + step * 0.45;
                let foot_y = 6.2 - airborne_tuck * 1.4;
                body + ldir * reach + Vec2::new(0.0, foot_y)
            };
            let knee = (hip + foot) * 0.5 + Vec2::new(0.0, -2.5);
            pb.line(hip.x.round() as i32, hip.y.round() as i32, knee.x.round() as i32, knee.y.round() as i32, apply_flash(leg_hi, flash));
            pb.line(knee.x.round() as i32, knee.y.round() as i32, foot.x.round() as i32, foot.y.round() as i32, apply_flash(leg_lo, flash));
            draw_shaded_disc(pb, foot.x, foot.y, 1.4, brass_hi, brass_mid, INK, flash);
        }
    }

    // 2. Heavy swinging brass clapper visible underneath the lifted bell (BLAZING during Tell!).
    let clapper_vis = matches!(pose, Pose::Tell(_) | Pose::Idle | Pose::Recover(_)) || lift < -1.5;
    if clapper_vis {
        let swing = match pose {
            Pose::Tell(t) => tell_quiver(t) * 2.2,
            Pose::Recover(_) => 3.0,
            _ => (anim * 3.5).sin() * 2.2,
        };
        let clap = Vec2::new(body.x + swing, body.y + 7.2);
        pb.line(body.x.round() as i32, body.y.round() as i32, clap.x.round() as i32, clap.y.round() as i32, brass_lo);
        let clap_hi = if let Pose::Tell(t) = pose {
            mix(brass_hi, (255, 255, 232), tell_coil(t))
        } else {
            brass_spec
        };
        draw_shaded_disc(pb, clap.x, clap.y, 3.0, clap_hi, brass_hi, brass_mid, flash);
    }

    // 3. Flared heavy brass bell skirt rim with UNMISTAKABLE 60° JAGGED CHIPPED NOTCH
    //    aligned strictly along `chip_dir` (the ring's safe missing-note gap)!
    let rim_y = body.y + 4.0;
    let rx = 12.4 + squash;
    let ry = 6.4 + squash * 0.35;
    let inv_rx = 1.0 / rx;
    let inv_ry = 1.0 / ry;
    let bx0 = ((body.x - rx - 1.5).floor() as i32).max(0);
    let bx1 = ((body.x + rx + 1.5).ceil() as i32).min(pb.w as i32 - 1);
    let by0 = ((rim_y - ry - 1.5).floor() as i32).max(0);
    let by1 = ((rim_y + ry + 1.5).ceil() as i32).min(pb.h as i32 - 1);
    let cos_chip = 0.84_f32; // ~33° half-angle => unmistakable 60° broken notch!
    let chip_glow = matches!(pose, Pose::Tell(_) | Pose::Strike(_));
    let ink_c = apply_flash(INK, flash * 0.35);

    for y in by0..=by1 {
        let dy = (y as f32 + 0.5 - rim_y) * inv_ry;
        let dy2 = dy * dy;
        for x in bx0..=bx1 {
            let dx = (x as f32 + 0.5 - body.x) * inv_rx;
            let d2 = dx * dx + dy2;
            if d2 > 1.12 {
                continue;
            }
            let dot = dx * chip_dir.x + dy * chip_dir.y;
            let in_chip = d2 > 0.14 && dot > 0.0 && (dot * dot) > d2 * (cos_chip * cos_chip);
            if in_chip {
                if d2 > 0.32 && d2 < 0.72 {
                    let frac_c = if chip_glow { (255, 250, 178) } else { brass_spec };
                    pb.plot(x, y, apply_flash(frac_c, flash));
                }
                continue;
            }
            let near_chip_lip = d2 > 0.36 && dot > 0.0 && (dot * dot) > d2 * (0.74 * 0.74);
            let c = if d2 > 0.90 {
                ink_c
            } else if near_chip_lip {
                if chip_glow { (255, 252, 186) } else { brass_spec }
            } else if dx + dy < -0.25 {
                brass_hi
            } else if dx + dy < 0.32 {
                brass_mid
            } else {
                brass_lo
            };
            pb.plot(x, y, apply_flash(c, flash));
        }
    }

    // 4. Heavy riveted verdigris-bronze domed bell carapace WITH A MATCHING
    //    CARVED JAGGED CHIP NOTCH along `chip_dir` so even when `chip_dir`
    //    points straight up (0, -1) the missing chunk in the bell is 100%
    //    unmistakable and visibly launches the 60° missing-note gap!
    let dome_rx = 9.6 + squash * 0.6;
    let dome_ry = 9.2 - squash * 0.5;
    let dome_c = Vec2::new(body.x, body.y - 2.2);
    // Double-edge ringing ghost vibration during Strike!
    if let Pose::Strike(t) = pose {
        let g_off = (1.0 - t) * 2.8;
        for &s in &[-1.0_f32, 1.0] {
            let gx = dome_c.x + s * g_off;
            for k in 0..16 {
                let a = k as f32 * (TAU / 16.0);
                let d_vec = Vec2::from_angle(a);
                if d_vec.dot(chip_dir) > 0.84 {
                    continue;
                }
                let q = Vec2::new(gx + d_vec.x * (dome_rx + 1.4), dome_c.y + d_vec.y * (dome_ry + 1.0));
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, teal_glow, 0.58 * (1.0 - t));
            }
        }
    }
    // Custom dome draw that carves the broken 60° chip right out of the outer rim along `chip_dir`!
    {
        let ox = dome_rx + 1.0;
        let oy = dome_ry + 1.0;
        let dx0 = ((dome_c.x - ox).floor() as i32).max(0);
        let dx1 = ((dome_c.x + ox).ceil() as i32).min(pb.w as i32 - 1);
        let dy0 = ((dome_c.y - oy).floor() as i32).max(0);
        let dy1 = ((dome_c.y + oy).ceil() as i32).min(pb.h as i32 - 1);
        let inv_drx = 1.0 / dome_rx;
        let inv_dry = 1.0 / dome_ry;
        let inv_ox = 1.0 / ox;
        let inv_oy = 1.0 / oy;
        for y in dy0..=dy1 {
            let vy = y as f32 + 0.5 - dome_c.y;
            let noy2 = (vy * inv_oy) * (vy * inv_oy);
            let ny = vy * inv_dry;
            let ny2 = ny * ny;
            for x in dx0..=dx1 {
                let vx = x as f32 + 0.5 - dome_c.x;
                let nox = vx * inv_ox;
                if nox * nox + noy2 > 1.0 {
                    continue;
                }
                let nx = vx * inv_drx;
                let nd2 = nx * nx + ny2;
                let cdot = nx * chip_dir.x + ny * chip_dir.y;
                // Carve jagged missing wedge from the outer 45% of the dome along `chip_dir`!
                if nd2 > 0.24 && cdot > 0.0 && (cdot * cdot) > nd2 * (0.86 * 0.86) {
                    if nd2 < 0.42 {
                        // Bright gleaming fractured-brass interior edge of the chip!
                        let fc = if chip_glow { (255, 250, 178) } else { brass_spec };
                        pb.plot(x, y, apply_flash(fc, flash));
                    }
                    continue;
                }
                let near_fissure = nd2 > 0.26 && cdot > 0.0 && (cdot * cdot) > nd2 * (0.76 * 0.76);
                let c = if nd2 > 0.94 {
                    ink_c
                } else if near_fissure {
                    // Fractured bronze-gold border along both sides of the chip!
                    apply_flash(if chip_glow { (255, 244, 152) } else { brass_hi }, flash)
                } else {
                    let lit = (0.52 + 0.54 * (nx * LIGHT.x + ny * LIGHT.y)).clamp(0.0, 1.0);
                    let base = if lit > 0.58 {
                        mix(bell_mid, bell_hi, (lit - 0.58) / 0.42)
                    } else {
                        mix(bell_lo, bell_mid, lit / 0.58)
                    };
                    apply_flash(base, flash)
                };
                pb.plot(x, y, c);
            }
        }
    }
    // Crisp specular verdigris patina highlight bar on top-left (away from chip).
    if chip_dir.x > -0.3 || chip_dir.y > -0.3 {
        draw_capsule(pb, dome_c + Vec2::new(-5.2, -3.2), dome_c + Vec2::new(-3.2, -5.0), 1.0, bell_spec, bell_hi, bell_mid, flash);
    }
    // Subtle golden "missing-note" notch rays radiating right out of the broken chip!
    if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        let t_ray = match pose {
            Pose::Tell(t) => t,
            Pose::Strike(t) => 1.0 - t * 0.5,
            _ => 0.0,
        };
        for &sign in &[-1.0_f32, 1.0] {
            let edge = chip_dir.rotate(sign * 0.50);
            let p0 = dome_c + Vec2::new(edge.x * dome_rx * 0.78, edge.y * dome_ry * 0.78);
            let p1 = p0 + edge * (4.0 + t_ray * 4.5);
            pb.line(p0.x.round() as i32, p0.y.round() as i32, p1.x.round() as i32, p1.y.round() as i32, (255, 236, 128));
        }
    }
    // Brass vertical reinforcing rivets across the lower dome skirt.
    for &rx_off in &[-5.5_f32, 0.0, 5.5] {
        plot_px(pb, (dome_c.x + rx_off).round() as i32, (dome_c.y + 4.8).round() as i32, apply_flash(brass_hi, flash));
    }

    // 5. Heavy brass crown suspension lug ring offset slightly away from an upward chip
    //    so the missing-note notch at top rim stays 100% open and dramatic!
    let crown_shift = if chip_dir.y < -0.6 { Vec2::new(-f.x * 5.2, 1.2) } else { Vec2::ZERO };
    let crown = Vec2::new(body.x, body.y - 11.4 + squash * 0.4) + crown_shift;
    draw_shaded_disc(pb, crown.x, crown.y, 2.9, brass_spec, brass_mid, brass_lo, flash);
    draw_shaded_disc(pb, crown.x, crown.y, 1.1, (24, 30, 36), INK, INK, 0.0);

    // 6. Big riveted AMBER GLASS DIVING-HELMET PORTHOLE EYE facing `f`!
    let eye_c = dome_c + f * 4.4 + Vec2::new(0.0, 0.4);
    let expr = eye_expr_for(pose, anim, flash);
    draw_shaded_disc(pb, eye_c.x, eye_c.y, 4.1, brass_hi, brass_mid, brass_lo, flash);
    let visor_col = match pose {
        Pose::Strike(_) => (210, 255, 250),
        Pose::Tell(_) => (255, 234, 118),
        Pose::Recover(_) => (124, 168, 152),
        _ => (255, 198, 72),
    };
    draw_eye(pb, eye_c, 2.7, visor_col, (56, 26, 12), f, expr, flash);
    // Diving-helmet brass cross-bar grille glint on outer rim.
    plot_px(pb, eye_c.x.round() as i32 - 3, eye_c.y.round() as i32, brass_spec);
    plot_px(pb, eye_c.x.round() as i32 + 3, eye_c.y.round() as i32, brass_lo);

    // Retreat: indignant steam/bubble puffs from the brass crown valve!
    if let Pose::Retreat(t) = pose {
        for k in 0..2 {
            let ph = (t * 2.6 + k as f32 * 0.5).fract();
            let q = crown + Vec2::new(((k as f32) - 0.5) * 3.0, -2.5 - ph * 6.5);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, bell_spec, (1.0 - ph) * 0.85);
        }
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, crown + Vec2::new(0.0, -4.5), 7.2, t);
        draw_recover_beacon(pb, crown, t);
    }
    draw_hurt_burst(pb, body, f, flash, brass_spec);
}

// ============================================================================
// 5. THE FOREMAN — "Denied in triplicate."
//    Towering pompous mahogany-and-brass walking rubber stamp (~34 px tall)
//    with stern spectacles, crimson ink pad & REVERSED raised die text!
// ============================================================================

fn draw_foreman(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let wood_spec: Rgb = (252, 206, 146);
    let wood_hi: Rgb = (224, 148, 86);
    let wood_mid: Rgb = (166, 92, 48);
    let wood_lo: Rgb = (94, 46, 22);
    let rubber_hi: Rgb = (224, 62, 86);
    let rubber_mid: Rgb = (158, 34, 56);
    let rubber_lo: Rgb = (78, 16, 30);
    let ink_bright: Rgb = (255, 86, 102);
    let brass_spec: Rgb = (255, 246, 174);
    let brass_hi: Rgb = (238, 194, 84);
    let brass_lo: Rgb = (128, 88, 28);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 16.5, 8.2, 0.50);

        // Expanding glossy crimson stamp-ink puddle!
        let pr = 8.0 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x, center.y + 3.0, pr, pr * 0.56, rubber_hi, rubber_mid, rubber_lo, flash);

        // Cracked mahogany mount halves splayed apart!
        let left_h = center + Vec2::new(-5.5 - burst * 4.5, -1.0 - bounce * 0.5);
        let right_h = center + Vec2::new(5.5 + burst * 4.0, 1.5 - bounce * 0.4);
        draw_rotated_box(pb, left_h, Vec2::new(0.92, -0.38), 5.2, 4.2, wood_hi, wood_mid, wood_lo, flash);
        draw_rotated_box(pb, right_h, Vec2::new(0.92, 0.38), 4.8, 3.8, wood_mid, wood_lo, (56, 26, 12), flash);

        // Snapped turned-wood fist-knob handle bouncing & rolling away!
        let knob = center + Vec2::new(7.0 + burst * 6.0, -6.5 - bounce * 1.4);
        draw_shaded_disc(pb, knob.x, knob.y, 4.4, wood_spec, wood_hi, wood_lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, ink_bright);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    // Entrance: drops down with a heavy crimson ink-burst slam!
    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -22.0
        } else {
            let u = (t - 0.44) / 0.56;
            -((u * PI).sin()) * 2.5 * (1.0 - u)
        }
    } else {
        0.0
    };

    // Three explosive slams matching exact 0.3 s intervals across the 0.9 s Strike window!
    let (lift, lean_x, squash_w, squash_h, slam_flash, show_die) = match pose {
        Pose::Entrance(t) => (ent_drop, 0.0, if (0.44..0.60).contains(&t) { 2 } else { 0 }, if (0.44..0.60).contains(&t) { -1 } else { 0 }, (0.44..0.58).contains(&t), t < 0.40),
        Pose::Idle => ((anim * 2.2).sin() * 0.4, 0.0, 0, 0, false, false),
        Pose::Approach => {
            let g = (anim * 6.2).sin();
            (-g.abs() * 2.8, g * 2.2, 0, 0, false, g.abs() > 0.65)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Hoists dramatically 9 px high showing reversed rubber die text!
            (-9.0 * c, f.x * 2.4 * c + q * 1.2, 0, 0, false, c > 0.25)
        }
        Pose::Strike(t) => {
            let beat = (t * 3.0).fract();
            if beat < 0.28 {
                // Explosive SLAM impact!
                (2.4, f.x * 3.2, 3, -2, true, false)
            } else {
                // Snaps back aloft for the next stamp beat!
                let up = ((beat - 0.28) / 0.72).sin();
                (-7.0 * up, f.x * 1.6, 0, 0, false, up > 0.45)
            }
        }
        Pose::Recover(t) => {
            // Stuck forward in tacky ink, handle drooping bent!
            let wob = (t * 10.0).sin() * 0.9;
            (1.6, f.x * 3.0 + wob, 1, -1, false, false)
        }
        Pose::Retreat(t) => (-(t * 10.0).sin().abs() * 1.8, -f.x * 2.4, 0, 0, false, false),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let bx = (center.x + lean_x + jolt.x).round() as i32;
    let by = (center.y + lift + jolt.y).round() as i32;
    let hw = 10 + squash_w;
    let hh = (7 + squash_h).max(5);

    // Grounded shadow (expands & softens when hoisted 9 px high in Tell!).
    let hoist = (-lift).max(0.0);
    draw_shadow(pb, center.x + 1.5, center.y + 8.5, 13.0 + hoist * 0.35, 5.8 + hoist * 0.15, (0.56 - hoist * 0.02).max(0.30));

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), ink_bright);
        }
    }

    // Strike: vertical motion-smear streaks + radial crimson ink-splatter ring on each slam!
    if slam_flash {
        for &sx in &[-hw + 2, 0, hw - 2] {
            for dy in 1..=9 {
                blend_px(pb, bx + sx, by - hh - dy, (255, 224, 178), (1.0 - dy as f32 / 10.0) * 0.75);
            }
        }
        for k in 0..10 {
            let a = k as f32 * (TAU / 10.0);
            let q = Vec2::new(bx as f32 + a.cos() * (hw as f32 + 4.5), (by + hh + 2) as f32 + a.sin() * 4.5);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 2 == 0 { ink_bright } else { (255, 246, 200) });
        }
    }

    // 1. Two chunky ink-stained rubber marching boots anchored cleanly beneath the stamp!
    let stride = match pose {
        Pose::Approach => (anim * 6.2).sin() * 4.2,
        Pose::Idle => {
            // Impatient right-foot tap!
            if (anim * 1.4).sin() > 0.7 { 2.2 } else { 0.0 }
        }
        Pose::Retreat(t) => (t * 10.0).sin() * 3.0,
        _ => 0.0,
    };
    for &sx in &[-6i32, 6] {
        let lift_foot = if sx < 0 {
            (-stride).max(0.0)
        } else {
            stride.max(0.0)
        };
        let fx = bx + sx;
        let fy = by + hh + 4 - lift_foot.round() as i32;
        pb.rect_fill(fx - 3, fy - 2, 6, 4, apply_flash(INK, flash * 0.35));
        pb.rect_fill(fx - 2, fy - 1, 4, 2, apply_flash(rubber_mid, flash));
        pb.rect_fill(fx - 2, fy - 1, 3, 1, apply_flash(rubber_hi, flash));
    }

    // 2. Crimson ink-soaked rubber die pad underneath the mahogany block!
    let pad_y = by + hh - 1;
    let pad_h = if show_die { 6 } else { 4 };
    pb.rect_fill(bx - hw, pad_y, hw * 2 + 1, pad_h, apply_flash(INK, flash * 0.35));
    pb.rect_fill(bx - hw + 1, pad_y + 1, hw * 2 - 1, pad_h - 2, apply_flash(rubber_lo, flash));
    pb.rect_fill(bx - hw + 1, pad_y + 1, hw * 2 - 2, 1, apply_flash(rubber_hi, flash));

    // REVERSED raised crimson rubber stamp lettering ("И O" / "D E" reversed!) visible when lifted!
    if show_die {
        // Raised reversed letter ridges gleaming with wet crimson ink!
        let ly = pad_y + 2;
        for &lx in &[-7i32, -6, -4, -2, -1, 1, 3, 5, 6] {
            plot_px(pb, bx + lx, ly, apply_flash(ink_bright, flash));
        }
        for &lx in &[-7i32, -5, -2, 0, 2, 3, 6] {
            plot_px(pb, bx + lx, ly + 1, apply_flash(rubber_hi, flash));
        }
    }

    // Dripping fat crimson ink drops during Idle, Approach & Tell!
    if matches!(pose, Pose::Idle | Pose::Approach | Pose::Tell(_)) {
        let drop_t = (anim * 1.45).fract();
        let dy_drop = (drop_t * (6.0 + hoist)).round() as i32;
        let dx_drop = bx + if f.x >= 0.0 { 4 } else { -4 };
        plot_px(pb, dx_drop, pad_y + pad_h + dy_drop, ink_bright);
        if drop_t < 0.7 {
            plot_px(pb, dx_drop, pad_y + pad_h + dy_drop - 1, rubber_hi);
        }
    }

    // 3. Heavy beveled mahogany wooden stamp block!
    pb.rect_fill(bx - hw, by - hh, hw * 2 + 1, hh * 2, apply_flash(INK, flash * 0.35));
    for dy in (-hh + 1)..hh {
        for dx in (-hw + 1)..hw {
            let col = if dy == -hh + 1 || dx == -hw + 1 {
                wood_hi
            } else if dy >= hh - 2 || dx >= hw - 2 {
                wood_lo
            } else if dy == -hh + 2 {
                mix(wood_mid, wood_hi, 0.55)
            } else {
                wood_mid
            };
            pb.plot(bx + dx, by + dy, apply_flash(col, flash));
        }
    }
    // Gleaming brass front index plate (doubles as a stern bureaucratic mustache!).
    pb.rect_fill(bx - 6, by + 2, 13, 3, apply_flash(INK, flash * 0.35));
    pb.rect_fill(bx - 5, by + 3, 11, 1, apply_flash(brass_lo, flash));
    pb.rect_fill(bx - 5, by + 2, 11, 1, apply_flash(brass_hi, flash));
    pb.rect_fill(bx - 4, by + 2, 8, 1, apply_flash(brass_spec, flash));

    // Stern bureaucratic face: twin brass spectacles + furrowed angry brows!
    let expr = eye_expr_for(pose, anim, flash);
    let spec_tilt = if matches!(pose, Pose::Recover(_)) || flash > 0.15 { 1.4 } else { 0.0 };
    let eye_l = Vec2::new((bx - 4) as f32, (by - 2) as f32 - spec_tilt * 0.5);
    let eye_r = Vec2::new((bx + 4) as f32, (by - 2) as f32 + spec_tilt * 0.5);
    // Brass spectacle bridge.
    pb.line(eye_l.x.round() as i32, eye_l.y.round() as i32, eye_r.x.round() as i32, eye_r.y.round() as i32, brass_hi);
    let lens_c = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 108),
        _ => (246, 250, 255),
    };
    draw_eye(pb, eye_l, 2.2, lens_c, INK, f, expr, flash);
    draw_eye(pb, eye_r, 2.2, lens_c, INK, f, expr, flash);
    // Heavy angry V-brows over spectacles in Approach/Tell/Strike!
    if matches!(pose, Pose::Approach | Pose::Tell(_) | Pose::Strike(_) | Pose::Retreat(_)) && flash <= 0.15 {
        pb.line(bx - 7, by - 5, bx - 2, by - 4, INK);
        pb.line(bx + 7, by - 5, bx + 2, by - 4, INK);
    }

    // 4. Brass ferrule collar + tapered turned-wood fist-knob handle rising high on top!
    let handle_bend = match pose {
        Pose::Recover(t) => f.x * 3.2 + (t * 12.0).sin() * 1.4,
        _ if flash > 0.1 => (anim * 46.0).sin() * 2.5,
        Pose::Approach => (anim * 6.2).sin() * 1.5,
        _ => 0.0,
    };
    let collar_y = by - hh - 2;
    pb.rect_fill(bx - 4, collar_y, 9, 3, apply_flash(INK, flash * 0.35));
    pb.rect_fill(bx - 3, collar_y + 1, 7, 1, apply_flash(brass_hi, flash));
    let neck_top = Vec2::new(bx as f32 + handle_bend, (collar_y - 5) as f32);
    draw_tapered_capsule(
        pb,
        Vec2::new(bx as f32, collar_y as f32),
        neck_top,
        2.6,
        1.8,
        wood_hi,
        wood_mid,
        wood_lo,
        flash,
    );
    // Big glossy bulbous fist-knob on top!
    let knob_c = neck_top + Vec2::new(handle_bend * 0.3, -3.5);
    draw_shaded_disc(pb, knob_c.x, knob_c.y, 5.0, wood_spec, wood_hi, wood_lo, flash);
    // Crisp gleaming mahogany highlight dot on knob.
    plot_px(pb, knob_c.x.round() as i32 - 2, knob_c.y.round() as i32 - 2, (255, 244, 212));

    // Recover: exhausted steam puffs rising from brass collar + dizzy stars + beacon!
    if let Pose::Recover(t) = pose {
        for k in 0..2 {
            let ph = (t * 3.0 + k as f32 * 0.5).fract();
            let sx = bx as f32 + (if k == 0 { -6.0 } else { 6.0 }) * (1.0 + ph * 0.5);
            let sy = collar_y as f32 - ph * 7.0;
            blend_px(pb, sx.round() as i32, sy.round() as i32, (242, 234, 236), (1.0 - ph) * 0.85);
        }
        draw_dizzy_stars(pb, knob_c + Vec2::new(0.0, -6.5), 7.0, t);
        draw_recover_beacon(pb, knob_c, t);
    }
    draw_hurt_burst(pb, Vec2::new(bx as f32, by as f32), f, flash, brass_spec);
}

// ============================================================================
// 6. MILK CARTON — "Three weeks past instructions."
//    Bulky waxed cardboard half-gallon carton (~34 px tall) on FOUR square feet
//    with dairy-blue band, red barn, expired use-by date, gable peak & spout!
// ============================================================================

fn draw_milk_carton(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let wax_spec: Rgb = (255, 255, 246);
    let wax_hi: Rgb = (246, 244, 232);
    let wax_mid: Rgb = (212, 208, 194);
    let wax_lo: Rgb = (146, 144, 134);
    let blue_hi: Rgb = (112, 174, 236);
    let blue_mid: Rgb = (54, 108, 176);
    let blue_lo: Rgb = (28, 60, 108);
    let barn_red: Rgb = (218, 64, 70);
    let sour_hi: Rgb = (252, 254, 224);
    let sour_mid: Rgb = (224, 230, 170);
    let sour_curd: Rgb = (182, 198, 104);
    let stink_green: Rgb = (176, 224, 128);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 16.5, 8.0, 0.48);

        // Spreading chalky sour-milk puddle with lumpy yellow-green curds!
        let pr = 8.5 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x + 2.0, center.y + 3.0, pr, pr * 0.56, sour_hi, sour_mid, sour_curd, flash);
        for k in 0..5 {
            let a = k as f32 * (TAU / 5.0) + 0.4;
            let q = center + Vec2::new(2.0 + a.cos() * pr * 0.65, 3.0 + a.sin() * pr * 0.35);
            draw_shaded_disc(pb, q.x, q.y, 1.5, sour_hi, sour_curd, (132, 148, 72), 0.0);
        }

        // Crumpled flat waxed carton collapsed on its side!
        let cx = (center.x - 4.0 - burst * 3.0).round() as i32;
        let cy = (center.y - 2.0 - bounce * 0.6).round() as i32;
        pb.rect_fill(cx - 9, cy - 4, 18, 9, apply_flash(INK, flash * 0.35));
        pb.rect_fill(cx - 8, cy - 3, 16, 7, apply_flash(wax_mid, flash));
        pb.rect_fill(cx - 8, cy - 3, 16, 2, apply_flash(wax_hi, flash));
        pb.rect_fill(cx - 8, cy, 16, 3, apply_flash(blue_mid, flash));
        // Crushed gable fin sticking out one end!
        pb.rect_fill(cx + 8, cy - 2, 4, 5, apply_flash(INK, flash * 0.35));
        pb.rect_fill(cx + 8, cy - 1, 3, 3, apply_flash(blue_hi, flash));
        if t < 0.40 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.40, sour_hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let face_x: f32 = if f.x >= 0.0 { 1.0 } else { -1.0 };

    // Entrance: heavy carton thud with accordion squash & milk burst out the top!
    let (ent_drop, ent_sq) = if let Pose::Entrance(t) = pose {
        if t < 0.42 {
            let u = t / 0.42;
            ((1.0 - u * u) * -20.0, 0.0)
        } else {
            let u = (t - 0.42) / 0.58;
            (0.0, ((u * PI).sin() * (1.0 - u)) * 2.5)
        }
    } else {
        (0.0, 0.0)
    };

    // Shear-tilt (`lean` shifts top horizontally relative to bottom), lift, squash, spout state.
    let (lean, lift, squash, spout_open, rear_kick) = match pose {
        Pose::Entrance(_) => (0.0, ent_drop, ent_sq, false, 0.0),
        Pose::Idle => ((anim * 2.0).sin() * 1.4, (anim * 2.0).cos() * 0.4, 0.0, false, 0.0),
        Pose::Approach => {
            // Heavy boxy 4-footed waddle & slosh!
            let s = (anim * 6.0).sin();
            (s * 2.8, -s.abs() * 1.8, (anim * 12.0).sin() * 0.5, false, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Rear feet kick high off the floor as carton tilts dramatically forward!
            (face_x * (6.2 * c) + q * 1.0, -2.5 * c, -0.5, true, c)
        }
        Pose::Strike(t) => {
            let glug = (t * 24.0).sin() * 0.9;
            (face_x * 8.2 + glug, 0.6, 0.6, true, 1.0)
        }
        Pose::Recover(t) => {
            // Rocks heavily backward onto its heels exposing the folded rear seam!
            let slosh = (t * 8.0).sin() * 1.4;
            (-face_x * 5.2 + slosh, 0.4, 0.4, false, 0.0)
        }
        Pose::Retreat(t) => (-face_x * 3.2 + (t * 10.0).sin() * 1.8, -(t * 10.0).sin().abs() * 1.2, 0.0, false, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.45, 1.1);
    draw_shadow(pb, center.x + 1.5 + lean * 0.3, center.y + 9.0, 12.5 * sh_s, 5.5 * sh_s, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), sour_hi);
        }
    }

    // 1. FOUR distinct chunky square cardboard corner feet!
    let waddle = if matches!(pose, Pose::Approach) {
        (anim * 6.0).sin() * 2.2
    } else if let Pose::Retreat(t) = pose {
        (t * 10.0).sin() * 1.8
    } else {
        0.0
    };
    let feet = [
        (-7.5_f32, 7.5_f32, waddle, true),   // back-left
        (-3.0_f32, 8.8_f32, -waddle, true),  // inner-left
        (3.0_f32, 8.8_f32, waddle, false),   // inner-right
        (7.5_f32, 7.5_f32, -waddle, false),  // front-right
    ];
    for &(fx, fy, step, is_left) in &feet {
        let is_rear = (face_x > 0.0 && is_left) || (face_x < 0.0 && !is_left);
        let kick = if is_rear { -rear_kick * 5.2 } else { 0.0 };
        let px = (base.x + fx + if is_rear { -face_x * rear_kick * 1.5 } else { 0.0 }).round() as i32;
        let py = (base.y + fy + step.min(0.0) + kick).round() as i32;
        pb.rect_fill(px - 2, py - 2, 5, 4, apply_flash(INK, flash * 0.35));
        pb.rect_fill(px - 1, py - 1, 3, 2, apply_flash(wax_mid, flash));
        plot_px(pb, px - 1, py - 1, apply_flash(wax_hi, flash));
    }

    // 2. Waxed cardboard rectangular carton body (sheared horizontally by `lean` so
    //    it physically tilts forward to pour and rocks backward in Recover!).
    let hw = (8.5 + squash * 0.7).round() as i32;
    let bot_y = (base.y + 7.0).round() as i32;
    let top_y = (base.y - 8.0 + squash).round() as i32;
    let h_span = (bot_y - top_y).max(6) as f32;
    let ink_c = apply_flash(INK, flash * 0.35);

    // Helper: x-center at screen row `py` with shear tilt (`bot` stays anchored, `top` leans!).
    let row_cx = |py: i32| -> i32 {
        let u = ((bot_y - py) as f32 / h_span).clamp(0.0, 1.4);
        (base.x + lean * u).round() as i32
    };

    for py in top_y..=bot_y {
        let rcx = row_cx(py);
        let rel_y = py - top_y;
        let on_stripe = (5..=10).contains(&rel_y);
        let is_edge_y = py == top_y || py == bot_y;
        // Hurt buckle dents the middle wall inward!
        let buckle = if flash > 0.15 && (4..=11).contains(&rel_y) { 1 } else { 0 };
        let rhw = (hw - buckle).max(4);
        for px in (rcx - rhw)..=(rcx + rhw) {
            let is_edge_x = px == rcx - rhw || px == rcx + rhw;
            let c = if is_edge_x || is_edge_y {
                ink_c
            } else {
                // Distinct front panel vs darker side panel (3D boxy carton read!).
                let on_side = (face_x > 0.0 && px > rcx + 3) || (face_x < 0.0 && px < rcx - 3);
                if on_stripe {
                    if px == rcx - rhw + 1 {
                        blue_hi
                    } else if on_side {
                        blue_lo
                    } else {
                        blue_mid
                    }
                } else if px == rcx - rhw + 1 || rel_y == 1 {
                    wax_spec
                } else if on_side {
                    wax_lo
                } else {
                    wax_mid
                }
            };
            plot_px(pb, px, py, apply_flash(c, flash));
        }
        // Prominent folded rear seam crease on the back flank (`-face_x`) — player's punish side!
        if !is_edge_y {
            let seam_x = rcx - (face_x as i32) * (rhw - 3);
            let seam_col = if matches!(pose, Pose::Recover(_)) {
                (148, 255, 206)
            } else {
                wax_lo
            };
            plot_px(pb, seam_x, py, apply_flash(seam_col, flash));
        }
    }

    // Printed red barn silhouette emblem on the lower blue dairy band!
    let barn_y = top_y + 8;
    let barn_cx = row_cx(barn_y) + (face_x as i32);
    pb.rect_fill(barn_cx - 2, barn_y, 5, 3, apply_flash(barn_red, flash));
    plot_px(pb, barn_cx, barn_y - 1, apply_flash(barn_red, flash));
    plot_px(pb, barn_cx, barn_y + 1, apply_flash(wax_spec, flash)); // tiny white barn door!

    // Expressive printed dairy-mascot carton face right above the blue band!
    let eye_y = top_y + 3;
    let eye_cx = row_cx(eye_y) + (face_x as i32);
    let expr = eye_expr_for(pose, anim, flash);
    let eye_bg = match pose {
        Pose::Recover(_) => stink_green,
        Pose::Tell(_) | Pose::Strike(_) => (255, 246, 176),
        _ => wax_spec,
    };
    draw_eye(pb, Vec2::new((eye_cx - 3) as f32, eye_y as f32), 1.8, eye_bg, blue_lo, f, expr, flash);
    draw_eye(pb, Vec2::new((eye_cx + 3) as f32, eye_y as f32), 1.8, eye_bg, blue_lo, f, expr, flash);

    // 3. Folded gable roof peak + crimped blue ridge + EXPIRED "USE BY" date ticks!
    for dy in 1..=6 {
        let py = top_y - dy;
        let rcx = row_cx(py);
        let rhw = (hw - dy).max(2);
        for px in (rcx - rhw - 1)..=(rcx + rhw + 1) {
            let c = if px == rcx - rhw - 1 || px == rcx + rhw + 1 || dy == 6 {
                ink_c
            } else if dy == 5 {
                blue_hi // crimped dairy-blue sealed ridge fin!
            } else if dy == 2 && (px - rcx).abs() <= 3 && ((px - rcx) & 1) == 0 {
                // Crisp printed expired "USE BY" stamp ticks on the sloped gable!
                (76, 86, 52)
            } else if px < rcx {
                wax_spec
            } else {
                wax_mid
            };
            plot_px(pb, px, py, apply_flash(c, flash));
        }
    }

    // 4. Articulated open triangular pouring spout beak facing `+face_x`!
    let spout_base = Vec2::new((row_cx(top_y - 2) + (face_x as i32) * (hw - 2)) as f32, (top_y - 2) as f32);
    let spout_tip = spout_base
        + Vec2::new(face_x * if spout_open { 6.2 } else { 3.2 }, if spout_open { 1.4 } else { -0.5 });
    draw_tapered_capsule(pb, spout_base, spout_tip, 2.4, 1.1, wax_spec, blue_hi, blue_lo, flash);
    // Dark inner open spout mouth when pouring!
    if spout_open {
        plot_px(pb, spout_tip.x.round() as i32, spout_tip.y.round() as i32, INK);
    }

    // Idle & Recover: curling greenish expired sour-milk stink wisps / bubbles!
    if matches!(pose, Pose::Idle | Pose::Tell(_) | Pose::Recover(_)) {
        for k in 0..2 {
            let ph = (anim * 1.4 + k as f32 * 0.5).fract();
            let sx = spout_tip.x + (ph * TAU).sin() * 1.8;
            let sy = spout_tip.y - 1.5 - ph * 6.5;
            blend_px(pb, sx.round() as i32, sy.round() as i32, stink_green, (1.0 - ph) * 0.85);
        }
    }

    // Approach: sloshing pale milk drops spilling out of the spout on peak waddle!
    if matches!(pose, Pose::Approach) {
        let ph = (anim * 2.0).fract();
        let drop = spout_tip + Vec2::new(face_x * (1.5 + ph * 3.5), ph * ph * 7.0);
        plot_px(pb, drop.x.round() as i32, drop.y.round() as i32, sour_hi);
    }

    // Tell: swelling wobbling pale-yellow curdled milk glob bulging at the spout lip!
    if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        let glob = spout_tip + Vec2::new(face_x * (1.8 + c * 2.2), 1.2 + c * 2.0);
        draw_shaded_disc(pb, glob.x, glob.y, 1.2 + c * 1.5, (255, 255, 246), sour_mid, sour_curd, 0.0);
    }

    // Strike: thick arcing frothy sour-milk waterfall stream + bouncing foam & curds!
    if let Pose::Strike(t) = pose {
        let mid_arc = spout_tip + f * 5.0 + Vec2::new(0.0, 3.8);
        let splash = spout_tip + f * 9.5 + Vec2::new(0.0, 9.5);
        draw_tapered_capsule(pb, spout_tip, mid_arc, 2.0, 2.5, (255, 255, 250), sour_hi, sour_mid, 0.0);
        draw_tapered_capsule(pb, mid_arc, splash, 2.4, 3.2, (255, 255, 250), sour_mid, sour_curd, 0.0);
        draw_shaded_ellipse(pb, splash.x, splash.y, 4.6, 2.6, (255, 255, 250), sour_hi, sour_curd, 0.0);
        // Splashing foam beads!
        for k in 0..4 {
            let ph = (t * 4.0 + k as f32 * 0.25).fract();
            let dx = (k as f32 - 1.5) * 3.0 * (0.5 + ph);
            let dy = -ph * (1.0 - ph) * 11.0;
            plot_px(pb, (splash.x + dx).round() as i32, (splash.y + dy).round() as i32, (255, 255, 250));
        }
    }

    // Recover: highlight the exposed folded rear seam ("Circle to the folded seam") + dizzy stars!
    if let Pose::Recover(t) = pose {
        let seam_spot = Vec2::new((row_cx(top_y + 6) - (face_x as i32) * (hw - 2)) as f32, (top_y + 6) as f32);
        draw_dizzy_stars(pb, Vec2::new(row_cx(top_y - 6) as f32, (top_y - 9) as f32), 6.8, t);
        draw_recover_beacon(pb, seam_spot, t);
    }
    draw_hurt_burst(pb, base, f, flash, sour_hi);
}


// ============================================================================
// 7–24. THE WAVE-4 NAMED BODIES
//
// Eighteen further silhouettes (WAVE4_ROSTERS.md "Named appointments and
// elites"), one per row, each drawn from three or four primitives along its
// own axis so the twenty-four are told apart at a glance: a light edge, a dark
// edge, one accessory that IS the signature (the wrench foreleg, the claws,
// the rollers, the ring's torn notch) and a tell pose that winds it up.
//
// A dedicated art pass follows this batch. These are deliberately simple: the
// budget went on correct shapes and visible tells, not polish.
// ============================================================================

/// A three-stop palette: light edge, body, dark edge.
#[derive(Clone, Copy)]
struct Pal {
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
}

#[inline]
const fn pal(hi: Rgb, mid: Rgb, lo: Rgb) -> Pal {
    Pal { hi, mid, lo }
}

/// One wave-4 body's pose, resolved once into the offsets the drawing uses:
/// where the body is, which way it faces, how far it has coiled and how much
/// hurt jolt to add. The six wave-3 bodies resolve their own poses; the
/// eighteen shared the same recipe so their tells read alike.
struct Frame {
    /// The body centre after the hurt jolt and the pose's own lift.
    c: Vec2,
    /// Facing (normalised).
    f: Vec2,
    /// `f.perp()`.
    s: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
    /// 0 → 1 (and a little past, at the trigger) through the tell.
    coil: f32,
}

impl Frame {
    fn new(center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) -> Self {
        let (lean, lift, coil) = match pose {
            Pose::Entrance(t) => (0.0, -13.0 * (1.0 - t) * (1.0 - t), 0.0),
            Pose::Idle => ((anim * 2.0).sin() * 0.7, (anim * 2.0).cos() * 0.4, 0.0),
            Pose::Approach => {
                let s = (anim * 6.0).sin();
                (s * 1.6, -s.abs() * 1.5, 0.0)
            }
            // Wind back against the facing (with the held beat's quiver), then
            // lunge along it on the strike.
            Pose::Tell(t) => {
                let c = tell_coil(t);
                (-c * 2.6 + tell_quiver(t) * 0.5, -c * 1.1, c)
            }
            Pose::Strike(_) => (4.4, 0.0, 1.0),
            Pose::Recover(_) => (-2.6, 0.0, 0.0),
            Pose::Retreat(t) => (-((t * 9.0).sin().abs()) * 2.0, 0.0, 0.0),
            Pose::Death(_) => (0.0, 0.0, 0.0),
        };
        // `lean` runs along the facing (it used to be a screen-x offset, so a
        // body facing left lunged backwards on its strike); `lift` is screen-up.
        let fdir = safe_dir(f, Vec2::new(1.0, 0.0));
        Frame {
            c: center + hurt_jolt(f, flash, anim) + fdir * lean + Vec2::new(0.0, lift),
            f: fdir,
            s: fdir.perp(),
            pose,
            anim,
            flash,
            coil,
        }
    }

    /// A point `along` the facing and `across` it, from the body centre.
    #[inline]
    fn at(&self, along: f32, across: f32) -> Vec2 {
        self.c + self.f * along + self.s * across
    }

    fn shadow(&self, pb: &mut PixelBuffer, rx: f32, ry: f32) {
        draw_shadow(pb, self.c.x, self.c.y + ry + 1.0, rx, ry, 0.50);
    }

    fn disc(&self, pb: &mut PixelBuffer, at: Vec2, r: f32, p: Pal) {
        draw_shaded_disc(pb, at.x, at.y, r, p.hi, p.mid, p.lo, self.flash);
    }

    fn ellipse(&self, pb: &mut PixelBuffer, at: Vec2, rx: f32, ry: f32, p: Pal) {
        draw_shaded_ellipse(pb, at.x, at.y, rx, ry, p.hi, p.mid, p.lo, self.flash);
    }

    fn capsule(&self, pb: &mut PixelBuffer, a: Vec2, b: Vec2, ra: f32, rb: f32, p: Pal) {
        draw_tapered_capsule(pb, a, b, ra, rb, p.hi, p.mid, p.lo, self.flash);
    }

    fn slab(&self, pb: &mut PixelBuffer, at: Vec2, axis: Vec2, hl: f32, hw: f32, p: Pal) {
        draw_rotated_box(pb, at, axis, hl, hw, p.hi, p.mid, p.lo, self.flash);
    }

    /// One expressive eye on the body at (along, across).
    fn eye(&self, pb: &mut PixelBuffer, along: f32, across: f32, r: f32, sclera: Rgb, pupil: Rgb) {
        let at = self.at(along, across);
        let expr = eye_expr_for(self.pose, self.anim, self.flash);
        draw_eye(pb, at, r, sclera, pupil, self.f, expr, self.flash);
    }

    /// The three cues every named body owes the player: an arrival ring, the
    /// exposed-body beacon with its dizzy stars, and the hurt sparks.
    fn cues(&self, pb: &mut PixelBuffer, accent: Rgb, spark: Rgb) {
        if let Pose::Entrance(t) = self.pose {
            draw_entrance_ring(pb, self.c, t, accent);
        }
        if let Pose::Recover(t) = self.pose {
            draw_recover_beacon(pb, self.c, t);
            draw_dizzy_stars(pb, self.c - Vec2::new(0.0, 11.0), 6.4, t);
        }
        // The shared wind-up mark: a warning "!" over the body, flickering
        // while it coils and steady through the held beat, so every one of the
        // eighteen has a tell that reads without knowing its accessory.
        if let Pose::Tell(t) = self.pose {
            if t >= 0.6 || (t * 10.0).fract() < 0.6 {
                let x = self.c.x.round() as i32;
                let y = (self.c.y - 15.0).round() as i32;
                let col: Rgb = (255, 226, 96);
                for dy in 0..4 {
                    plot_px(pb, x, y + dy, col);
                    plot_px(pb, x + 1, y + dy, col);
                }
                plot_px(pb, x, y + 5, col);
                plot_px(pb, x + 1, y + 5, col);
            }
        }
        draw_hurt_burst(pb, self.c, self.f, self.flash, spark);
    }
}

/// The shared defeat pose: the body flattened, two of its own parts flung out,
/// and the golden pointer at the chest it leaves behind.
fn wave4_death(pb: &mut PixelBuffer, fr: &Frame, t: f32, p: Pal) {
    let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 3.0;
    let c = fr.c + Vec2::new(0.0, -bounce);
    draw_shadow(pb, fr.c.x, fr.c.y + 5.0, 10.5, 4.6, 0.48);
    draw_shaded_ellipse(pb, c.x, c.y + 2.0, 8.4, 3.6, p.lo, p.lo, p.lo, 0.0);
    let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
    let spread = 6.0 + burst * 6.0;
    fr_capsule_plain(pb, c - fr.f * spread, c - fr.f * (spread - 4.5), 2.2, 1.2, p);
    fr_capsule_plain(pb, c + fr.f * (spread - 4.5), c + fr.f * spread, 2.2, 1.2, p);
    if t < 0.40 {
        draw_hurt_burst(pb, fr.c, fr.f, 1.0 - t / 0.40, p.hi);
    }
    draw_chest_pointer(pb, fr.c, t);
}

/// A capsule with no hurt flash (the wreckage is already dead).
fn fr_capsule_plain(pb: &mut PixelBuffer, a: Vec2, b: Vec2, ra: f32, rb: f32, p: Pal) {
    draw_tapered_capsule(pb, a, b, ra, rb, p.hi, p.mid, p.lo, 0.0);
}

/// A ring of short spokes: wheels, fans, rollers and tally faces all use it.
#[allow(clippy::too_many_arguments)]
fn spokes(pb: &mut PixelBuffer, fr: &Frame, at: Vec2, r: f32, n: i32, spin: f32, w: f32, p: Pal) {
    for i in 0..n {
        let a = spin + i as f32 * TAU / n as f32;
        let dir = Vec2::from_angle(a);
        draw_tapered_capsule(pb, at + dir * (r * 0.30), at + dir * r, w, w * 0.6, p.hi, p.mid, p.lo, fr.flash);
    }
}

// ---- 7. VALVE WIDOW · "Someone left the water running." --------------------
// A 26 px wheel spider: a brass hand-wheel with a rust-red hub and a long
// wrench foreleg that winds back through the tell and strikes forward.
fn draw_valve_widow(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let brass = pal((255, 236, 172), (206, 152, 54), (104, 68, 22));
    let rust = pal((255, 148, 108), (188, 74, 42), (96, 32, 18));
    let steel = pal((236, 244, 255), (158, 176, 196), (72, 84, 100));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, brass);
        return;
    }
    fr.shadow(pb, 11.5, 5.0);
    // Eight spokes of the hand-wheel, spinning as it walks.
    spokes(pb, &fr, fr.c, 8.0, 8, anim * 2.4, 1.1, brass);
    fr.disc(pb, fr.c, 3.4, rust);
    // The long wrench foreleg: wound back through the tell, thrown forward on
    // the strike. This is the row's tell.
    // Long (the silhouette's one asymmetry): out past the wheel at rest, cocked
    // back beside the hub through the tell, at full stretch on the strike.
    let reach = if matches!(pose, Pose::Strike(_)) {
        15.0
    } else {
        11.0 - fr.coil * 7.0
    };
    let jaw = fr.at(reach, -5.4 - fr.coil * 2.2);
    fr.capsule(pb, fr.at(1.5, -3.4), jaw, 1.9, 1.3, steel);
    // Open-ended wrench jaw.
    fr.capsule(pb, jaw, jaw + fr.f.rotate(0.35) * 4.4, 1.2, 0.8, steel);
    fr.capsule(pb, jaw, jaw + fr.f.rotate(-0.35) * 4.4, 1.2, 0.8, steel);
    // Six thin legs.
    for i in 0..3 {
        let t = i as f32 - 1.0;
        let step = (anim * 5.0 + i as f32).sin() * 1.2;
        fr.capsule(pb, fr.at(-1.0, t * 3.6), fr.at(-5.0 + step, t * 5.4), 0.9, 0.6, brass);
        fr.capsule(pb, fr.at(1.0, t * 3.6), fr.at(5.0 - step, t * 5.4), 0.9, 0.6, brass);
    }
    fr.eye(pb, 0.6, -0.4, 1.5, (255, 246, 214), (96, 32, 18));
    fr.cues(pb, (255, 202, 96), steel.hi);
}

// ---- 8. THE MUDLARK · "Both traps are occupied." ---------------------------
// A 32 px mud crab under two broken mousetraps; the claws astride the aim open
// wide for the tell and shut on the strike.
fn draw_mudlark(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let mud = pal((186, 160, 112), (124, 100, 64), (62, 48, 28));
    let trap = pal((232, 226, 206), (168, 154, 128), (74, 66, 52));
    let steel = pal((230, 240, 250), (152, 168, 184), (66, 76, 90));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, mud);
        return;
    }
    let striking = matches!(pose, Pose::Strike(_));
    fr.shadow(pb, 15.0, 5.6);
    // Eight jointed legs splayed wide under the shell: the crab is far wider
    // than it is long (about 30 px toe to toe).
    for i in 0..4 {
        let t = i as f32 - 1.5;
        let step = (anim * 6.0 + i as f32 * 1.3).sin() * 1.2;
        for side in [-1.0f32, 1.0] {
            let knee = fr.at(t * 2.6, side * 9.4);
            fr.capsule(pb, fr.at(t * 1.8, side * 4.0), knee, 1.1, 0.9, mud);
            fr.capsule(pb, knee, fr.at(t * 3.4 + step * side, side * 13.4), 0.9, 0.5, mud);
        }
    }
    // The carapace: a stadium lying across the facing.
    fr.capsule(pb, fr.at(0.0, -5.2), fr.at(0.0, 5.2), 5.4, 5.4, mud);
    // The two broken mousetraps it wears on its back: a plank each, with the
    // sprung bail bent up across it.
    for side in [-1.0f32, 1.0] {
        let at = fr.at(-1.2, side * 4.6);
        fr.slab(pb, at, fr.f, 4.2, 2.6, trap);
        fr.capsule(
            pb,
            at - fr.f * 3.0 - fr.s * (side * 1.8),
            at + fr.f * 1.6 + fr.s * (side * 2.6),
            0.6,
            0.6,
            steel,
        );
        fr.disc(pb, at, 0.9, steel);
    }
    // The claws astride the aim: arms thrown wide for the tell (they start 28
    // apart), slammed to the centre line on the strike. Each ends in a
    // two-finger pincer that gapes through the tell and is shut on the strike.
    let sep = if matches!(pose, Pose::Tell(_)) {
        13.0 + fr.coil * 1.5
    } else if striking {
        2.0
    } else {
        8.0
    };
    let gape = if striking { 0.08 } else { 0.35 + fr.coil * 0.35 };
    for side in [-1.0f32, 1.0] {
        let shoulder = fr.at(3.0, side * 6.0);
        let wrist = fr.at(9.0, side * sep);
        fr.capsule(pb, shoulder, wrist, 2.2, 2.6, mud);
        fr.capsule(pb, wrist, wrist + fr.f.rotate(side * gape) * 6.0, 2.0, 0.8, steel);
        fr.capsule(pb, wrist, wrist + fr.f.rotate(-side * gape) * 5.0, 1.6, 0.7, steel);
    }
    // Two eyes on stalks.
    fr.disc(pb, fr.at(5.4, -2.2), 1.2, mud);
    fr.disc(pb, fr.at(5.4, 2.2), 1.2, mud);
    fr.eye(pb, 5.6, -2.2, 1.3, (255, 250, 220), (52, 40, 22));
    fr.eye(pb, 5.6, 2.2, 1.3, (255, 250, 220), (52, 40, 22));
    fr.cues(pb, (196, 168, 116), steel.hi);
}

// ---- 9. PASTRY WHEEL · "Makes six equal portions." -------------------------
// A 22 px fluted cutter wheel on a fork handle; the wheel spins up through the
// tell and throws the cut forward.
fn draw_pastry_wheel(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let steel = pal((246, 250, 255), (172, 186, 202), (78, 88, 104));
    let wood = pal((236, 190, 132), (172, 118, 66), (86, 54, 26));
    let dough = pal((255, 248, 224), (226, 208, 168), (150, 132, 96));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, steel);
        return;
    }
    let striking = matches!(pose, Pose::Strike(_));
    fr.shadow(pb, 10.5, 4.6);
    let spin = anim * (3.0 + fr.coil * 9.0);
    // The wheel rears back along its handle through the tell and is thrown
    // forward down the cut on the strike.
    let wheel = fr.at(if striking { 7.0 } else { 5.0 - fr.coil * 2.4 }, 0.0);
    // The handle: a long wooden grip ending in a two-tine fork that carries
    // the axle either side of the wheel.
    fr.capsule(pb, fr.at(-11.0, 0.0), fr.at(-2.5, 0.0), 2.0, 1.5, wood);
    for side in [-1.0f32, 1.0] {
        fr.capsule(pb, fr.at(-2.5, side * 1.0), wheel + fr.s * (side * 2.8), 0.9, 0.8, steel);
    }
    // The fluted coin: ten scallops round a solid disc, so the edge is wavy
    // rather than spoked.
    for i in 0..10 {
        let dir = fr.f.rotate(spin + i as f32 * TAU / 10.0);
        fr.disc(pb, wheel + dir * 5.4, 1.5, steel);
    }
    fr.disc(pb, wheel, 5.2, steel);
    fr.disc(pb, wheel, 2.0, wood);
    // One dark flute so the spin (and the spin-up of the tell) is visible.
    fr.capsule(pb, wheel + fr.f.rotate(spin) * 2.0, wheel + fr.f.rotate(spin) * 4.6, 0.6, 0.6, wood);
    // A curl of cut pastry at the wheel's leading edge, growing with the tell.
    if fr.coil > 0.15 {
        fr.disc(pb, wheel + fr.f * 5.0 + fr.s * 3.4, 1.0 + fr.coil * 1.4, dough);
    }
    fr.eye(pb, -6.0, 0.0, 1.4, (255, 252, 240), (86, 54, 26));
    fr.cues(pb, (232, 236, 244), steel.hi);
}

// ---- 10. SALT HEN · "Season to taste. Hers." -------------------------------
// A 24 px china hen with three holes in its high tail; the tail lifts through
// the tell and the three piles fall.
fn draw_salt_hen(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let china = pal((255, 252, 244), (226, 216, 200), (150, 140, 126));
    let blue = pal((176, 214, 244), (92, 138, 190), (44, 70, 106));
    let hole = pal((44, 70, 106), (26, 42, 66), (14, 22, 36));
    let salt = pal((255, 255, 250), (238, 240, 236), (188, 194, 190));
    let beak = pal((255, 206, 120), (214, 158, 62), (110, 74, 24));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, china);
        return;
    }
    let striking = matches!(pose, Pose::Strike(_));
    fr.shadow(pb, 10.5, 4.4);
    // Two china feet under the body.
    for side in [-1.0f32, 1.0] {
        fr.capsule(pb, fr.at(0.4, side * 2.4), fr.at(0.4, side * 3.4) + Vec2::new(0.0, 4.0), 0.9, 0.5, blue);
    }
    // Plump china body with a painted wing.
    fr.ellipse(pb, fr.c, 7.0, 6.0, china);
    fr.capsule(pb, fr.at(-2.0, 0.0) + Vec2::new(0.0, 1.0), fr.at(2.5, 0.0) + Vec2::new(0.0, 2.2), 1.3, 0.8, blue);
    // The high tail is the shaker's cap: a tall fan with three dark holes. It
    // stands upright at rest and tips over toward the rat to pour.
    let tip = if striking { 6.0 } else { fr.coil * 5.0 };
    let root = fr.at(-4.5, 0.0) + Vec2::new(0.0, -3.0);
    let top = fr.at(-6.5 + tip, 0.0) + Vec2::new(0.0, -10.5 + tip * 0.3);
    fr.capsule(pb, root, top, 3.0, 3.6, china);
    for k in 0..3 {
        let at = top + fr.s * ((k as f32 - 1.0) * 2.2) + Vec2::new(0.0, -0.4);
        fr.disc(pb, at, 1.0, hole);
    }
    // Head, comb and beak.
    let head = fr.at(6.0, 0.0) + Vec2::new(0.0, -3.5);
    fr.disc(pb, head, 2.6, china);
    fr.slab(pb, head + Vec2::new(0.0, -2.6), fr.f, 1.6, 0.8, blue);
    fr.capsule(pb, head + fr.f * 1.8, head + fr.f * 3.8 + Vec2::new(0.0, 0.4), 1.0, 0.4, beak);
    // Salt running out of the tipped cap: three grains, one per hole.
    if fr.coil > 0.2 {
        for k in 0..3 {
            let fall = ((anim * 2.4 + k as f32 * 0.33).fract()) * 6.0;
            let at = top + fr.f * (2.0 + fr.coil * 2.0) + fr.s * ((k as f32 - 1.0) * 2.2) + Vec2::new(0.0, fall);
            fr.disc(pb, at, 0.9, salt);
        }
    }
    fr.eye(pb, 6.6, 0.0, 1.2, (255, 255, 255), (44, 70, 106));
    fr.cues(pb, (236, 242, 252), china.hi);
}

// ---- 11. POT BRUSH · "Scrub until the stain runs away." --------------------
// A 34 px bristle fan at the end of a jointed handle; the fan splay is the
// tell and the whole arm swings on the strike.
fn draw_pot_brush(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let bristle = pal((244, 226, 168), (198, 168, 96), (104, 82, 40));
    let wood = pal((238, 200, 146), (176, 122, 68), (88, 56, 28));
    let soap = pal((240, 252, 250), (176, 212, 208), (92, 128, 128));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, bristle);
        return;
    }
    fr.shadow(pb, 12.5, 5.2);
    // Jointed handle from the back to the elbow.
    let elbow = fr.at(-1.0 + fr.coil * 2.0, -4.0);
    // Two sections with a visible knuckle, so the whole reads as a long
    // jointed arm rather than a stub behind a fan.
    let knuckle = fr.at(-8.0, 1.6);
    fr.capsule(pb, fr.at(-15.0, -1.0), knuckle, 1.4, 1.4, wood);
    fr.capsule(pb, knuckle, elbow, 1.5, 1.3, wood);
    fr.disc(pb, knuckle, 1.7, bristle);
    fr.disc(pb, elbow, 1.6, wood);
    // The bristle fan: eleven short strokes spreading from the elbow.
    let spread = 0.55 + fr.coil * 0.45;
    for i in 0..11 {
        let u = i as f32 / 10.0 - 0.5;
        let dir = fr.f.rotate(u * spread * 2.0);
        let root = elbow + dir * 1.0;
        let tip = root + dir * (5.0 + (1.0 - u.abs() * 1.4) * 2.4);
        draw_tapered_capsule(pb, root, tip, 1.1, 0.7, bristle.hi, bristle.mid, bristle.lo, fr.flash);
    }
    // A soap bubble or two.
    if matches!(pose, Pose::Idle | Pose::Strike(_)) {
        let ph = (anim * 1.6).fract();
        fr.disc(pb, fr.at(7.0 - ph * 3.0, -7.0 - ph * 4.0), 1.2, soap);
    }
    fr.eye(pb, -3.0, 1.6, 1.4, (255, 250, 232), (88, 56, 28));
    fr.cues(pb, (240, 226, 176), bristle.hi);
}

// ---- 12. DOSE PUMP · "Read the label twice." ------------------------------
// A 19 px glass barrel on a plunger foot; the plunger draws back through the
// tell and drives through the strike.
fn draw_dose_pump(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let glass = pal((244, 254, 255), (186, 218, 226), (96, 128, 140));
    let fluid = pal((196, 226, 130), (130, 170, 70), (60, 88, 34));
    let steel = pal((236, 244, 252), (156, 172, 190), (70, 80, 96));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, glass);
        return;
    }
    let striking = matches!(pose, Pose::Strike(_));
    fr.shadow(pb, 8.0, 4.2);
    // The barrel stands on its nose at rest and is levelled at the rat through
    // the tell: `nose` swings from straight down to the locked aim, which is
    // the line the injection trench will run along.
    let down = Vec2::new(0.0, 1.0);
    let level = if striking || matches!(pose, Pose::Recover(_)) {
        1.0
    } else {
        fr.coil.min(1.0)
    };
    let nose = safe_dir(down * (1.0 - level) + fr.f * level, down);
    let side = nose.perp();
    // Glass barrel.
    fr.slab(pb, fr.c, nose, 6.4, 3.4, glass);
    // The dose: drawn in as the plunger pulls back, gone once it is driven.
    let dose = if striking { 0.8 } else { 1.6 + fr.coil * 2.4 };
    fr.slab(pb, fr.c + nose * (5.6 - dose), nose, dose, 2.8, fluid);
    // Two printed graduations.
    for k in 0..2 {
        fr.slab(pb, fr.c + nose * (-3.4 + k as f32 * 2.4), side, 1.8, 0.4, fluid);
    }
    // The plunger foot: a wide flange on the barrel's back end, the rod
    // through it, and the thumb plate — pulled far back through the tell.
    let pull = if striking { 7.6 } else { 10.0 + fr.coil * 2.6 };
    fr.capsule(pb, fr.c - nose * 6.0, fr.c - nose * pull, 1.3, 1.0, steel);
    fr.slab(pb, fr.c - nose * pull, side, 3.0, 1.0, steel);
    fr.slab(pb, fr.c - nose * 6.8, side, 4.4, 1.0, steel);
    // The needle, shot out on the strike.
    let needle = if striking { 13.0 } else { 9.6 };
    fr.capsule(pb, fr.c + nose * 6.4, fr.c + nose * needle, 0.8, 0.4, steel);
    fr.eye(pb, 0.0, 0.0, 1.3, (250, 255, 255), (60, 88, 34));
    fr.cues(pb, (200, 232, 214), glass.hi);
}

// ---- 13. PETRI MOTHER · "The blank sample had children." -------------------
// A 24 px shallow culture dish on cilia with a missing rim; the colony swells
// inside it through the tell.
fn draw_petri_mother(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let glass = pal((240, 250, 252), (182, 208, 214), (94, 120, 128));
    let colony = pal((255, 196, 226), (206, 122, 172), (108, 56, 92));
    let agar = pal((232, 226, 186), (184, 176, 132), (104, 98, 68));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, glass);
        return;
    }
    fr.shadow(pb, 12.0, 4.4);
    // Shallow dish: a wide flat ellipse with a rim and one missing arc.
    fr.ellipse(pb, fr.c, 10.4, 4.0, glass);
    fr.ellipse(pb, fr.c + Vec2::new(0.0, -0.8), 7.6, 2.4, agar);
    // The rim, broken where the sterile gap is: a quarter turn off the aim.
    let gap = fr.f.rotate(RING_GAP_TURN);
    for i in 0..18 {
        let a = i as f32 * TAU / 18.0;
        let dir = Vec2::from_angle(a);
        // The break is as wide as the ring's own 75° sterile gap.
        if dir.dot(gap) > 0.79 {
            continue;
        }
        let at = fr.c + Vec2::new(dir.x * 11.0, dir.y * 4.6);
        fr.disc(pb, at, 1.3, glass);
    }
    // The culture front: a ring of spots creeping out across the agar toward
    // the rim through the tell — the ring that is about to leave the dish.
    if fr.coil > 0.1 {
        let front = 3.0 + fr.coil.min(1.0) * 6.0;
        for i in 0..10 {
            let dir = Vec2::from_angle(i as f32 * TAU / 10.0 + anim);
            if dir.dot(gap) > 0.79 {
                continue;
            }
            let at = fr.c + Vec2::new(dir.x * front, dir.y * front * 0.42 - 0.8);
            fr.disc(pb, at, 0.8, colony);
        }
    }
    // The colony, swelling with the tell.
    let r = 1.8 + fr.coil * 2.6 + (anim * 3.0).sin() * 0.3;
    fr.disc(pb, fr.c + Vec2::new(0.0, -1.0), r, colony);
    for k in 0..3 {
        let a = anim * 1.6 + k as f32 * TAU / 3.0;
        fr.disc(pb, fr.c + Vec2::from_angle(a) * 3.4 + Vec2::new(0.0, -1.0), 0.9, colony);
    }
    // Cilia under the dish.
    for i in 0..5 {
        let x = (i as f32 - 2.0) * 3.4;
        let wob = (anim * 5.0 + i as f32).sin() * 0.9;
        fr.capsule(
            pb,
            fr.c + Vec2::new(x, 3.4),
            fr.c + Vec2::new(x + wob, 7.0),
            0.7,
            0.4,
            glass,
        );
    }
    fr.eye(pb, 3.4, -1.2, 1.2, (255, 255, 255), (108, 56, 92));
    fr.cues(pb, (232, 240, 246), colony.hi);
}

// ---- 14. CAGE COUNTER · "One rat short again." -----------------------------
// A 26 px tally wheel on a tripod; the wheel clicks round through the tell.
fn draw_cage_counter(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let brass = pal((255, 236, 176), (204, 154, 62), (104, 70, 24));
    let ink = pal((196, 208, 226), (120, 136, 158), (54, 62, 74));
    let pad = pal((255, 172, 170), (200, 84, 92), (98, 34, 44));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, brass);
        return;
    }
    fr.shadow(pb, 10.0, 4.6);
    // Tripod.
    for k in 0..3 {
        let a = k as f32 * TAU / 3.0 + FRAC_PI_2;
        let foot = fr.c + Vec2::from_angle(a) * 7.4 + Vec2::new(0.0, 4.4);
        fr.capsule(pb, fr.c + Vec2::new(0.0, 1.4), foot, 1.0, 0.7, ink);
    }
    // Tally wheel with ten notches, clicking round with the coil.
    let spin = anim * 1.4 + fr.coil * 2.6;
    spokes(pb, &fr, fr.c - Vec2::new(0.0, 2.0), 6.2, 10, spin, 1.0, brass);
    fr.disc(pb, fr.c - Vec2::new(0.0, 2.0), 3.0, ink);
    // The pointer arm at the top, and the tally mark it lands on.
    fr.capsule(pb, fr.c - Vec2::new(0.0, 8.4), fr.c - Vec2::new(0.0, 5.4), 1.0, 0.7, pad);
    fr.disc(pb, fr.c - Vec2::new(0.0, 9.0), 1.2, pad);
    // Three reader lamps under the wheel: dark at rest, lit one by one as the
    // tell counts up (left, right, centre — the order its pads pulse in).
    let lit = if matches!(pose, Pose::Strike(_)) {
        3
    } else {
        (fr.coil * 3.0).floor() as i32
    };
    for (n, x) in [(0, -4.0f32), (1, 4.0), (2, 0.0)] {
        let at = fr.c + Vec2::new(x, 5.6);
        fr.disc(pb, at, 1.3, if n < lit { pad } else { ink });
    }
    fr.eye(pb, 1.8, 0.0, 1.4, (255, 252, 236), (54, 62, 74));
    fr.cues(pb, (236, 224, 188), brass.hi);
}

// ---- 15. ROTOR TECHNICIAN · "Switch off before removing tail." -------------
// A 32 px triangular rotor head over a snaking cable; the head turns through
// the tell and the cable whips on the strike.
fn draw_rotor_technician(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let shell = pal((176, 214, 236), (98, 140, 172), (46, 68, 92));
    let cable = pal((236, 196, 90), (172, 122, 44), (82, 56, 20));
    let spark = pal((255, 250, 214), (250, 208, 96), (176, 116, 32));
    let steel = pal((244, 250, 255), (176, 190, 206), (80, 90, 108));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, shell);
        return;
    }
    fr.shadow(pb, 12.0, 5.0);
    // The snaking cable behind the head: seven beads on a travelling sine,
    // long enough that head and cable together run about 32 px.
    for i in 0..7 {
        let u = i as f32;
        let ph = anim * 4.0 + u * 1.1 + fr.coil * 2.0;
        let at = fr.at(-7.0 - u * 2.4, ph.sin() * (1.0 + u * 0.45));
        fr.disc(pb, at, 1.5 - u * 0.1, cable);
    }
    // The triangular rotor head: three corners joined by straight sides, idly
    // turning, squared up to the rat as the tell locks.
    let turn = anim * 1.2 * (1.0 - fr.coil.min(1.0));
    let corners: [Vec2; 3] =
        std::array::from_fn(|k| fr.c + fr.f.rotate(turn + k as f32 * TAU / 3.0) * 8.6);
    for k in 0..3 {
        fr.capsule(pb, corners[k], corners[(k + 1) % 3], 1.6, 1.6, shell);
    }
    fr.disc(pb, fr.c, 4.6, shell);
    fr.disc(pb, fr.c, 2.2, spark);
    // The blade. Folded back over the cable at rest; through the tell it
    // swings out to the edge its 150° turn starts from (never across the aim),
    // then follows the strike round to the far edge and hangs there, spent.
    let half_sweep = 75.0_f32.to_radians();
    let (ang, reach) = match pose {
        Pose::Tell(_) => {
            let k = fr.coil.min(1.0);
            (-PI + (PI - half_sweep) * k, 9.0 + 5.0 * k)
        }
        Pose::Strike(t) => (-half_sweep + 2.0 * half_sweep * t, 15.0),
        Pose::Recover(_) => (half_sweep, 12.0),
        _ => (-PI, 9.0),
    };
    let blade = fr.f.rotate(ang);
    fr.capsule(pb, fr.c, fr.c + blade * reach, 1.8, 0.9, steel);
    // A live spark on the blade's tip through the tell.
    if fr.coil > 0.3 {
        fr.disc(pb, fr.c + blade * (reach + 1.0), 1.1, spark);
    }
    fr.eye(pb, 2.0, 0.0, 1.3, (255, 255, 255), (46, 68, 92));
    fr.cues(pb, (180, 220, 240), spark.hi);
}

// ---- 16. SUDS BAILIFF · "All delicates report to rinse." ------------------
// A 27 px soap dome with one flat broken edge, foaming through the tell.
fn draw_suds_bailiff(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let soap = pal((236, 250, 252), (176, 214, 222), (88, 124, 138));
    let cut = pal((88, 124, 138), (58, 86, 98), (34, 52, 62));
    let foam = pal((255, 255, 255), (222, 240, 244), (150, 180, 190));
    let brass = pal((255, 230, 158), (196, 148, 58), (96, 66, 22));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, soap);
        return;
    }
    fr.shadow(pb, 13.0, 5.0);
    // The dome: a big round bubble sitting in a flat skirt of foam.
    let dome = fr.c + Vec2::new(0.0, -1.0);
    fr.disc(pb, dome, 9.0, soap);
    fr.slab(pb, fr.c + Vec2::new(0.0, 6.8), Vec2::new(1.0, 0.0), 9.8, 1.6, foam);
    // The one flat broken edge: a chord sliced off the dome on the side its
    // rim's gap opens (a quarter turn off the facing), dark with a bright lip.
    let gap = fr.f.rotate(RING_GAP_TURN);
    let chord = dome + gap * 7.6;
    fr.slab(pb, chord, gap.perp(), 4.6, 1.5, cut);
    fr.slab(pb, chord - gap * 1.6, gap.perp(), 4.2, 0.5, foam);
    // A peaked bailiff's cap on top.
    fr.slab(pb, dome + Vec2::new(0.0, -9.6), Vec2::new(1.0, 0.0), 2.6, 1.2, brass);
    fr.slab(pb, dome + Vec2::new(0.0, -8.2), Vec2::new(1.0, 0.0), 4.4, 0.8, brass);
    // Foam beads boiling off the dome: more of them, further out, through the
    // tell — the rim that is about to roll outward.
    let n = 4 + (fr.coil * 6.0) as i32;
    for k in 0..n {
        let a = anim * 1.2 + k as f32 * TAU / n as f32;
        let dir = Vec2::from_angle(a);
        if dir.dot(gap) > 0.84 {
            continue;
        }
        let r = 10.0 + (k as f32 * 1.7 + anim * 3.0).sin() * 1.2 + fr.coil * 3.0;
        fr.disc(pb, dome + dir * r, 1.3, foam);
    }
    fr.eye(pb, 3.0, -1.8, 1.5, (255, 255, 255), (88, 124, 138));
    fr.cues(pb, (236, 250, 250), foam.hi);
}

// ---- 17. THE WRINGER BOY · "Please remove hands before pressing." ----------
// A 36 px pair of rollers with a limp glove between them; the rollers spin up
// and close on the strike.
fn draw_wringer_boy(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let steel = pal((240, 248, 255), (162, 182, 204), (70, 86, 108));
    let rubber = pal((206, 196, 224), (136, 122, 158), (62, 54, 76));
    let glove = pal((255, 226, 202), (206, 160, 132), (108, 76, 56));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, steel);
        return;
    }
    fr.shadow(pb, 15.5, 5.4);
    // The frame the pair runs in: a low bed behind the rollers.
    fr.slab(pb, fr.at(-3.4, 0.0), fr.s, 12.0, 1.6, rubber);
    // Two rollers end to end across the aim, exactly as the strike's bars lie:
    // a cylinder either side of the central seam the glove hangs through.
    // They part for the tell and slam back to the seam on the strike.
    let inner = if matches!(pose, Pose::Tell(_)) {
        4.4 + fr.coil * 1.6
    } else if matches!(pose, Pose::Strike(_)) {
        2.6
    } else {
        3.4
    };
    let roll = (anim * (3.4 + fr.coil * 6.0)).rem_euclid(TAU);
    for side in [-1.0f32, 1.0] {
        let mid = fr.at(0.6, side * (inner + 6.0));
        fr.slab(pb, mid, fr.s, 6.0, 2.8, steel);
        // End caps and a rolling seam line, so the cylinder visibly turns.
        fr.disc(pb, fr.at(0.6, side * inner), 2.8, rubber);
        fr.disc(pb, fr.at(0.6, side * (inner + 12.0)), 2.8, rubber);
        fr.slab(pb, mid + fr.f * (roll.cos() * 2.0), fr.s, 5.4, 0.5, rubber);
    }
    // The limp glove hanging between them, dragged further in by the coil.
    let hand = fr.at(5.6 - fr.coil * 1.4, 0.0);
    fr.disc(pb, hand, 1.8, glove);
    for k in 0..3 {
        let a = -0.5 + k as f32 * 0.5;
        fr.capsule(pb, hand, hand + fr.f.rotate(a) * (3.0 + fr.coil), 0.8, 0.5, glove);
    }
    fr.eye(pb, -4.0, 0.0, 1.4, (255, 255, 255), (62, 54, 76));
    fr.cues(pb, (196, 204, 226), steel.hi);
}

// ---- 18. QUOIN KEY · "Locks the story in place." ---------------------------
// A 23 px steel fork driven by a walking wedge; the fork closes over the tell.
fn draw_quoin_key(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let steel = pal((244, 250, 255), (168, 184, 202), (74, 86, 104));
    let wood = pal((244, 202, 148), (182, 132, 78), (90, 60, 30));
    let ink = pal((120, 130, 148), (72, 80, 96), (34, 38, 48));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, steel);
        return;
    }
    fr.shadow(pb, 9.5, 4.6);
    // The walking wedge behind: a tapered block on two feet.
    fr.capsule(pb, fr.at(-4.0, 0.0), fr.at(-10.4, 0.0), 3.4, 1.4, wood);
    for side in [-1.0f32, 1.0] {
        let step = (anim * 5.0 + side).sin() * 1.0;
        fr.capsule(pb, fr.at(-6.0, side * 2.4), fr.at(-6.0 + step, side * 3.6) + Vec2::new(0.0, 2.4), 0.9, 0.5, ink);
    }
    // The steel fork: the prongs spread wide through the tell (the wedge the
    // chase frame is about to close over) and snap shut on the strike.
    let close = if matches!(pose, Pose::Strike(_)) {
        2.6
    } else {
        -fr.coil * 2.6
    };
    for side in [-1.0f32, 1.0] {
        let root = fr.at(-1.0, side * (3.6 - close * 0.5));
        let tip = fr.at(7.4, side * (3.0 - close));
        fr.capsule(pb, root, tip, 1.5, 0.9, steel);
    }
    fr.slab(pb, fr.at(-1.6, 0.0), fr.s, 1.6, 4.2, steel);
    fr.eye(pb, 2.0, 0.0, 1.3, (240, 246, 255), (34, 38, 48));
    fr.cues(pb, (232, 240, 250), steel.hi);
}

// ---- 19. CARBON TWIN · "Your duplicate stayed behind." ---------------------
// A 26 px pair of offset sheets peeling apart; they separate further through
// the tell and slap down on the strike.
fn draw_carbon_twin(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let sheet = pal((252, 252, 250), (222, 222, 216), (150, 152, 148));
    let carbon = pal((128, 138, 158), (74, 82, 100), (34, 38, 50));
    let inkline = pal((96, 104, 124), (60, 66, 82), (28, 32, 42));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, sheet);
        return;
    }
    fr.shadow(pb, 12.0, 4.8);
    // Two real sheets, offset: dark carbon under, white paper over. They peel
    // further apart through the tell and slap flat together on the strike.
    let apart = if matches!(pose, Pose::Strike(_)) {
        0.6
    } else {
        2.0 + fr.coil * 3.0 + (anim * 3.0).sin() * 0.3
    };
    fr.slab(pb, fr.at(-apart * 0.6, -apart), fr.f, 10.0, 5.0, carbon);
    let top = fr.at(apart * 0.6, apart);
    fr.slab(pb, top, fr.f, 10.0, 5.0, sheet);
    // Ruled lines on the paper.
    for k in 0..3 {
        fr.slab(pb, top + fr.s * ((k as f32 - 1.0) * 2.6), fr.f, 7.0, 0.4, inkline);
    }
    // The folded edge the transfer is laid from: a dark spine joining the two
    // at the front.
    fr.slab(pb, fr.at(10.2, 0.0), fr.s, apart + 5.0, 0.8, inkline);
    // The paper's back corner curling up as it peels.
    if fr.coil > 0.1 {
        let corner = top - fr.f * 9.0 + fr.s * 4.2 + Vec2::new(0.0, -fr.coil * 2.0);
        fr.disc(pb, corner, 1.4 + fr.coil, sheet);
    }
    fr.eye(pb, 3.0 + apart * 0.6, apart, 1.4, (255, 255, 255), (34, 38, 50));
    fr.cues(pb, (212, 218, 228), sheet.hi);
}

// ---- 20. LATE PLATE · "That's all the room we have tonight." ---------------
// A 32 px curved printing plate with a torn registration notch; the plate
// curls tighter through the tell.
fn draw_late_plate(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let steel = pal((236, 240, 248), (158, 168, 186), (68, 74, 92));
    let ink = pal((150, 128, 210), (94, 76, 146), (44, 34, 72));
    let paper = pal((255, 252, 240), (228, 222, 202), (152, 146, 126));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, steel);
        return;
    }
    fr.shadow(pb, 11.5, 4.8);
    // The curved plate: nine tangent segments on a wide arc bowed toward the
    // rat (about 30 px tip to tip), curling tighter through the tell the way
    // its ring closes. One segment is missing: the torn registration notch,
    // a real break in the silhouette like the break in its ring.
    let curl = 1.0 + fr.coil * 0.45;
    let radius = 11.0 - fr.coil * 2.4;
    let hub = fr.c - fr.f * 4.0;
    for i in 0..9 {
        if i == 6 {
            continue;
        }
        let u = i as f32 - 4.0;
        let dir = fr.f.rotate(u * 0.32 * curl);
        let at = hub + dir * radius;
        fr.slab(pb, at, dir.perp(), 2.4, 1.7, steel);
        // The inked printing face on the inside of the curve.
        fr.slab(pb, at - dir * 1.4, dir.perp(), 2.0, 0.5, ink);
    }
    // Torn paper edges either side of the notch.
    for u in [1.45f32, 2.55] {
        let dir = fr.f.rotate(u * 0.32 * curl);
        fr.disc(pb, hub + dir * radius, 1.0, paper);
    }
    // A registration cross on the plate.
    fr.slab(pb, fr.c, fr.f, 2.2, 0.4, ink);
    fr.slab(pb, fr.c, fr.s, 2.2, 0.4, ink);
    fr.eye(pb, 0.6, -2.6, 1.3, (255, 255, 255), (44, 34, 72));
    fr.cues(pb, (206, 214, 230), steel.hi);
}

// ---- 21. TARPAULIN PUP · "Contents may have shifted." ---------------------
// A 20 px folded tarp on four grommet feet; it unfurls wider through the tell.
fn draw_tarpaulin_pup(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let tarp = pal((176, 190, 128), (118, 132, 78), (58, 68, 38));
    let weave = pal((226, 232, 196), (176, 184, 144), (96, 104, 72));
    let grommet = pal((236, 226, 178), (176, 160, 98), (84, 74, 42));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, tarp);
        return;
    }
    fr.shadow(pb, 10.0, 4.2);
    // A wagging tail of tarp at the back.
    fr.capsule(pb, fr.at(-6.0, 0.0), fr.at(-9.4, (anim * 7.0).sin() * 1.8), 1.3, 0.6, tarp);
    // Four grommet feet under the fold.
    for (a, s) in [(-4.4f32, -3.0f32), (-4.4, 3.0), (4.4, -3.0), (4.4, 3.0)] {
        let at = fr.at(a, s);
        fr.capsule(pb, at, at + Vec2::new(0.0, 3.0), 0.9, 0.6, grommet);
        fr.disc(pb, at, 1.0, grommet);
    }
    // The folded tarp: a base panel with two seams...
    fr.slab(pb, fr.at(-1.2, 0.0), fr.f, 5.6, 3.6, tarp);
    for k in 0..2 {
        fr.slab(pb, fr.at(-4.0 + k as f32 * 2.6, 0.0), fr.s, 3.4, 0.4, grommet);
    }
    // ...and the top fold, which slides forward and lengthens as it unfurls:
    // a little through the tell, all the way out on the strike.
    let unfurl = if matches!(pose, Pose::Strike(_)) {
        7.0
    } else {
        fr.coil * 4.0
    };
    fr.slab(pb, fr.at(1.4 + unfurl * 0.5, 0.0), fr.f, 4.6 + unfurl * 0.5, 3.2, weave);
    // The hem on its leading edge.
    fr.slab(pb, fr.at(6.0 + unfurl, 0.0), fr.s, 3.2, 0.5, grommet);
    // Two floppy corner ears.
    for side in [-1.0f32, 1.0] {
        fr.capsule(pb, fr.at(3.4, side * 2.6), fr.at(1.8, side * 5.2), 1.3, 0.6, weave);
    }
    fr.eye(pb, 3.2, -0.8, 1.3, (255, 255, 250), (58, 68, 38));
    fr.cues(pb, (186, 196, 148), weave.hi);
}

// ---- 22. AXLE NUT · "Last seen under the truck." --------------------------
// A 24 px walking hex nut with a black hole in its face; it rolls up to speed
// through the tell.
fn draw_axle_nut(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let steel = pal((238, 244, 252), (160, 176, 196), (70, 80, 96));
    let hole = pal((52, 54, 66), (28, 30, 38), (14, 16, 22));
    let grease = pal((120, 116, 92), (78, 76, 58), (38, 36, 26));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, steel);
        return;
    }
    fr.shadow(pb, 9.6, 4.4);
    // Six flat facets: a ring of short slabs, rolling with the coil.
    let spin = anim * 1.8 + fr.coil * 2.2;
    // Each flat is a slab lying along the tangent, so the outline has six
    // straight sides and six corners instead of reading as a toothed gear.
    for k in 0..6 {
        let dir = fr.f.rotate(spin + k as f32 * TAU / 6.0);
        let at = fr.c + dir * 8.4;
        fr.slab(pb, at, dir.perp(), 5.6, 1.6, steel);
    }
    fr.disc(pb, fr.c, 8.2, steel);
    // The black hole in its face: big enough to be the first thing read.
    fr.disc(pb, fr.c, 4.0, hole);
    fr.disc(pb, fr.c + Vec2::new(-0.8, -0.8), 1.8, grease);
    // Two stubby walking legs, out of phase with the roll.
    for side in [-1.0f32, 1.0] {
        let step = (anim * 6.0 + side * 1.6).sin() * 1.2;
        fr.capsule(pb, fr.at(-1.0, side * 4.0), fr.at(-2.6 + step, side * 5.4) + Vec2::new(0.0, 2.6), 0.9, 0.5, steel);
    }
    fr.eye(pb, 2.8, -1.8, 1.2, (255, 255, 255), (14, 16, 22));
    fr.cues(pb, (212, 222, 236), steel.hi);
}

// ---- 23. CHAIN DOG · "Stays where it's tied. Mostly." ----------------------
// A 28 px chain loop dragging a triangular binder; the chain lifts through the
// tell and swings on the strike.
fn draw_chain_dog(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let chain = pal((236, 240, 250), (156, 166, 184), (64, 70, 86));
    let link = pal((156, 166, 184), (98, 106, 124), (44, 48, 60));
    let binder = pal((255, 196, 108), (196, 124, 48), (96, 54, 20));
    let hide = pal((206, 186, 150), (146, 124, 92), (70, 58, 40));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, chain);
        return;
    }
    fr.shadow(pb, 12.5, 4.8);
    // The triangular load binder on its drag chain. Dragged behind at rest;
    // through the tell it is hauled round to the edge the swing starts from
    // (never across the aim), then follows the strike to the far edge.
    let half_sweep = 57.5_f32.to_radians();
    let (ang, reach) = match pose {
        Pose::Tell(_) => {
            let k = fr.coil.min(1.0);
            (-PI + (PI - half_sweep) * k, 11.0 + 2.0 * k)
        }
        Pose::Strike(t) => (-half_sweep + 2.0 * half_sweep * t, 14.0),
        Pose::Recover(_) => (half_sweep, 12.0),
        _ => (-PI + (anim * 3.0).sin() * 0.25, 11.0),
    };
    let drag = fr.f.rotate(ang);
    let anchor = fr.at(-1.0, 0.0);
    let at_binder = anchor + drag * reach;
    for k in 1..4 {
        fr.disc(pb, anchor + drag * (reach * k as f32 / 4.0), 1.2, link);
    }
    let corners: [Vec2; 3] =
        std::array::from_fn(|k| at_binder + drag.rotate(k as f32 * TAU / 3.0) * 3.8);
    for k in 0..3 {
        fr.capsule(pb, corners[k], corners[(k + 1) % 3], 1.2, 1.2, binder);
    }
    // The body: a closed loop of chain, twelve links on an oval, crawling.
    for i in 0..12 {
        let a = i as f32 * TAU / 12.0 + anim * 0.8;
        let at = fr.at(a.cos() * 6.5 - 1.0, a.sin() * 4.6);
        fr.disc(pb, at, 1.5, if i % 2 == 0 { chain } else { link });
    }
    // Head, snout and two ears at the front of the loop.
    let head = fr.at(6.5, 0.0);
    for side in [-1.0f32, 1.0] {
        fr.capsule(pb, head + fr.s * (side * 1.8), fr.at(4.6, side * 4.4), 1.1, 0.6, hide);
    }
    fr.disc(pb, head, 3.0, hide);
    fr.capsule(pb, head, fr.at(10.0, 0.0), 1.8, 1.0, hide);
    fr.eye(pb, 7.0, -0.8, 1.3, (255, 250, 232), (70, 58, 40));
    fr.cues(pb, (226, 214, 190), binder.hi);
}

// ---- 24. THE ROAD ATLAS · "Every exit has a bird on it." -------------------
// A 34 px gull with a road map folded over its wings; the wings sweep back
// through the tell and hammer down on the strike.
fn draw_road_atlas(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let gull = pal((252, 252, 250), (216, 216, 210), (140, 142, 140));
    let slate = pal((168, 186, 204), (108, 126, 146), (52, 62, 74));
    let map = pal((255, 246, 214), (224, 208, 160), (146, 132, 92));
    let route = pal((240, 96, 96), (192, 52, 60), (104, 24, 30));
    let fr = Frame::new(center, f, pose, anim, flash);
    if let Pose::Death(t) = pose {
        wave4_death(pb, &fr, t, gull);
        return;
    }
    fr.shadow(pb, 13.0, 4.8);
    // Two swept wings, drawn back further as the tell coils.
    // A gull's span: held out to the sides at rest (the 34 px silhouette),
    // swept back through the tell, thrown forward and down on the strike.
    let sweep = if matches!(pose, Pose::Strike(_)) {
        0.75
    } else {
        1.35 + fr.coil * 0.75
    };
    for side in [-1.0f32, 1.0] {
        let dir = fr.f.rotate(side * sweep);
        let wrist = fr.c + dir * 8.0;
        fr.capsule(pb, fr.at(-1.0, side * 1.6), wrist, 2.2, 1.6, slate);
        // The outer wing kinks back at the wrist, as a gull's does.
        fr.capsule(pb, wrist, wrist + fr.f.rotate(side * (sweep + 0.45)) * 7.6, 1.6, 0.7, gull);
    }
    // Body and tail.
    fr.ellipse(pb, fr.c, 5.6, 3.0, gull);
    fr.capsule(pb, fr.at(-4.0, 0.0), fr.at(-8.4, 0.0), 1.4, 0.8, slate);
    // Beak.
    fr.capsule(pb, fr.at(5.0, 0.0), fr.at(8.4, 0.0), 0.9, 0.3, route);
    // The road map folded over the near wing: a cream panel with a red route.
    let panel = fr.at(-2.0, 4.6);
    fr.slab(pb, panel, fr.f.rotate(0.25), 4.2, 2.6, map);
    for k in 0..2 {
        fr.slab(pb, panel + fr.f * (k as f32 * 1.6 - 0.8), fr.f.rotate(0.25), 1.0, 0.4, route);
    }
    fr.eye(pb, 3.0, -1.4, 1.3, (255, 255, 255), (52, 62, 74));
    fr.cues(pb, (236, 240, 246), map.hi);
}

// ============================================================================
// Public Ground Telegraph Renderer (Driven Strictly by MinibossShape Data!)
// ============================================================================

/// Draw the current telegraph (Tell warning + Active live impact) on the ground
/// layer strictly from `mb.telegraph()` and `mb.active_shape()` geometry.
pub fn draw_telegraph(pb: &mut PixelBuffer, mb: &Miniboss, cam: Vec2, _accent: Rgb) {
    let Some(full_shape) = mb.telegraph() else {
        return;
    };
    let active_shape = mb.active_shape();
    let is_active = mb.state == MinibossState::Active;
    let progress = if is_active {
        (mb.timer / mb.kind.active_duration().max(0.05)).clamp(0.0, 1.0)
    } else {
        (mb.timer / TELL_TIME.max(0.05)).clamp(0.0, 1.0)
    };

    // A swinging cord's tell shape is the wedge it sweeps (a `Cone`), but it is
    // drawn as a cord: the stitched arc of the row's own sweep, both edges of
    // the sector, and the cord itself waiting on the edge the swing starts from.
    if let SignatureShape::Cord { len, half, sweep } = mb.kind.signature().shape {
        let aim = safe_dir(mb.aim, Vec2::new(1.0, 0.0));
        draw_cord_telegraph(
            pb,
            mb.origin,
            mb.origin + aim * len,
            half,
            sweep,
            active_shape,
            cam,
            is_active,
            progress,
        );
        return;
    }

    match full_shape {
        MinibossShape::Trench { a, b, half } => {
            draw_trench_telegraph(pb, a, b, half, active_shape, cam, is_active, progress);
        }
        MinibossShape::Cone {
            origin,
            aim,
            half_angle,
            len,
        } => {
            draw_cone_telegraph(pb, origin, aim, half_angle, len, cam, is_active, progress);
        }
        MinibossShape::Cord { spool, tip, half } => {
            // Not reached through a row today (cords tell as their sector,
            // above); a bare cord shape draws as the wave-3 quarter turn.
            draw_cord_telegraph(pb, spool, tip, half, FRAC_PI_2, active_shape, cam, is_active, progress);
        }
        MinibossShape::RingGap {
            origin,
            radius,
            half,
            gap_dir,
            gap_half,
        } => {
            draw_ring_telegraph(
                pb,
                origin,
                radius,
                half,
                gap_dir,
                gap_half,
                active_shape,
                cam,
                is_active,
                progress,
            );
            // A *closing* ring (the Late Plate) passes over everything inside
            // its rim, so the tell also washes the interior — all of it but
            // the wedge of the break, which is the one way out.
            if let SignatureShape::Ring { from, to, .. } = mb.kind.signature().shape {
                if from > to && !is_active {
                    let o = origin - cam;
                    let g = safe_dir(gap_dir, Vec2::new(1.0, 0.0));
                    let cos_gap = gap_half.cos();
                    let mut r = to.max(3.0);
                    while r < radius - half {
                        let n = ((r * TAU / 4.0).ceil() as i32).max(6);
                        for i in 0..n {
                            let dir = Vec2::from_angle(i as f32 * TAU / n as f32 + r * 0.37);
                            if dir.dot(g) > cos_gap {
                                continue;
                            }
                            let q = o + dir * r;
                            blend_px(
                                pb,
                                q.x.round() as i32,
                                q.y.round() as i32,
                                (72, 198, 184),
                                0.16 + 0.18 * progress,
                            );
                        }
                        r += 4.0;
                    }
                }
            }
        }
        MinibossShape::Stamps { centers, half } => {
            draw_stamps_telegraph(pb, centers, half, active_shape, cam, is_active, progress);
        }
        MinibossShape::Spill {
            center,
            aim,
            half_len,
            half_w,
        } => {
            draw_spill_telegraph(pb, center, aim, half_len, half_w, cam, is_active, progress);
        }
        MinibossShape::Pincer {
            origin,
            aim,
            len,
            half,
            gap_lo,
            gap_hi,
        } => {
            draw_pincer_telegraph(
                pb,
                origin,
                aim,
                len,
                half,
                gap_lo,
                gap_hi,
                active_shape,
                cam,
                is_active,
                progress,
            );
        }
        MinibossShape::Lane {
            origin,
            aim,
            len,
            seam_half,
            half,
            ..
        } => {
            draw_lane_telegraph(
                pb,
                origin,
                aim,
                len,
                seam_half,
                half,
                active_shape,
                cam,
                is_active,
                progress,
            );
        }
    }
}

/// One oriented band of a signature, mirrored on both sides of the lane: the
/// stretch `from..to` along `aim` from `origin`, `inner..outer` across it.
///
/// The pincer's claw travel and the rollers' lane are both two bands like this,
/// so both tell renderers read from one primitive and the drawing cannot
/// disagree with [`MinibossShape::contains`].
#[allow(clippy::too_many_arguments)]
fn band_wash(
    pb: &mut PixelBuffer,
    origin: Vec2,
    aim: Vec2,
    from: f32,
    to: f32,
    inner: f32,
    outer: f32,
    cam: Vec2,
    fill: Rgb,
    edge: Rgb,
    alpha: f32,
    dashed: bool,
    progress: f32,
) {
    if to <= from || outer <= inner {
        return;
    }
    let o = origin - cam;
    let dir = safe_dir(aim, Vec2::new(1.0, 0.0));
    let perp = dir.perp();
    let pad = to.max(outer).max(-from) + 1.5;
    let x0 = (o.x - pad).floor().max(0.0) as i32;
    let x1 = (o.x + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (o.y - pad).floor().max(0.0) as i32;
    let y1 = (o.y + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;
    for y in y0..=y1 {
        let qy = y as f32 + 0.5 - o.y;
        for x in x0..=x1 {
            let qx = x as f32 + 0.5 - o.x;
            let along = qx * dir.x + qy * dir.y;
            if along < from || along > to {
                continue;
            }
            let across = (qx * perp.x + qy * perp.y).abs();
            if across < inner || across > outer {
                continue;
            }
            let on_edge = (across - inner).abs() < 0.9 || (outer - across).abs() < 0.9;
            let c = if on_edge { edge } else { fill };
            let a = if dashed && !on_edge {
                // Chevrons travelling down the lane.
                if ((along - across - progress * 20.0).rem_euclid(9.0)) < 1.8 {
                    alpha
                } else {
                    alpha * 0.42
                }
            } else {
                alpha
            };
            blend_px(pb, x, y, c, a);
        }
    }
}

/// The Mudlark: the two claws' whole travel marked at the tell, and the pair's
/// live position lit on the strike.
///
/// "Two 30×5 straight claws start 28 apart and close to 4 apart in 0.65 s":
/// the tell has to mark where the claws *will* be, not only where they start,
/// or the closing mouth between them would be an unmarked trap.
#[allow(clippy::too_many_arguments)]
fn draw_pincer_telegraph(
    pb: &mut PixelBuffer,
    origin: Vec2,
    aim: Vec2,
    len: f32,
    half: f32,
    gap_lo: f32,
    gap_hi: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let warn_fill: Rgb = (150, 118, 84);
    let warn_edge: Rgb = (246, 214, 150);
    let hot_fill: Rgb = (255, 132, 72);
    let hot_core: Rgb = (255, 250, 196);
    // The swept travel, in warning colours.
    band_wash(
        pb,
        origin,
        aim,
        0.0,
        len,
        gap_lo * 0.5 - half,
        gap_hi * 0.5 + half,
        cam,
        warn_fill,
        warn_edge,
        if is_active { 0.34 } else { 0.72 },
        true,
        progress,
    );
    // The claws where they are right now.
    if is_active {
        if let Some(MinibossShape::Pincer {
            gap_lo: lo,
            gap_hi: hi,
            ..
        }) = active_shape
        {
            band_wash(
                pb,
                origin,
                aim,
                0.0,
                len,
                lo * 0.5 - half,
                hi * 0.5 + half,
                cam,
                hot_fill,
                hot_core,
                0.86,
                false,
                progress,
            );
        }
    }
}

/// The Wringer Boy: the whole lane marked at the tell (seam free), and the bar
/// pair's live stretch lit as it rolls.
///
/// The live stretch comes from `active_shape`, never from the tell's own
/// `covered` (the tell marks the whole lane, so reading it would paint the
/// entire lane as if it were being pressed).
#[allow(clippy::too_many_arguments)]
fn draw_lane_telegraph(
    pb: &mut PixelBuffer,
    origin: Vec2,
    aim: Vec2,
    len: f32,
    seam_half: f32,
    half: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let warn_fill: Rgb = (120, 140, 168);
    let warn_edge: Rgb = (218, 236, 255);
    let hot_fill: Rgb = (255, 124, 168);
    let hot_core: Rgb = (255, 248, 210);
    // The lane it will press, seam and all.
    band_wash(
        pb,
        origin,
        aim,
        0.0,
        len,
        seam_half,
        seam_half + half * 2.0,
        cam,
        warn_fill,
        warn_edge,
        if is_active { 0.30 } else { 0.74 },
        true,
        progress,
    );
    if !is_active {
        return;
    }
    // The stretch the bars are pressing right now.
    if let Some(MinibossShape::Lane {
        bar_len, covered, ..
    }) = active_shape
    {
        band_wash(
            pb,
            origin,
            aim,
            (covered - bar_len).max(0.0),
            covered.min(len),
            seam_half,
            seam_half + half * 2.0,
            cam,
            hot_fill,
            hot_core,
            0.86,
            false,
            progress,
        );
    }
}

/// Corkscrew: dotted brass trench filling from tail to tip with churning head!
#[allow(clippy::too_many_arguments)]
fn draw_trench_telegraph(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    half: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let sa = a - cam;
    let sb = b - cam;
    let delta = sb - sa;
    let len = delta.len();
    if len < 0.5 {
        return;
    }
    let dir = delta * (1.0 / len);
    let perp = dir.perp();
    let warn_fill: Rgb = (212, 148, 48);
    let warn_edge: Rgb = (255, 224, 118);
    let hot_fill: Rgb = (255, 128, 56);
    let hot_core: Rgb = (255, 250, 196);

    let pad = half + 1.5;
    let x0 = (sa.x.min(sb.x) - pad).floor().max(0.0) as i32;
    let x1 = (sa.x.max(sb.x) + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (sa.y.min(sb.y) - pad).floor().max(0.0) as i32;
    let y1 = (sa.y.max(sb.y) + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;

    let active_len = match active_shape {
        Some(MinibossShape::Trench { a: aa, b: ab, .. }) => (ab - aa).len(),
        _ => 0.0,
    };
    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();

    for y in y0..=y1 {
        let qy = y as f32 + 0.5 - sa.y;
        for x in x0..=x1 {
            let qx = x as f32 + 0.5 - sa.x;
            let along = qx * dir.x + qy * dir.y;
            if along < -0.8 || along > len + 0.8 {
                continue;
            }
            let across = (qx * perp.x + qy * perp.y).abs();
            if across > half + 0.8 {
                continue;
            }
            // Gently soften right over Corkscrew's body origin so the sprite reads cleanly.
            let body_fade = if along < 6.0 { 0.45 } else { 1.0 };
            let in_active = is_active && along >= 0.0 && along <= active_len && across <= half;
            if in_active {
                let head_dist = (active_len - along).abs();
                let churn = ((along - progress * 28.0) * 0.9 + across).sin() * 0.5 + 0.5;
                let col = if head_dist < 4.5 {
                    hot_core
                } else if across > half - 1.2 {
                    warn_edge
                } else {
                    mix(hot_fill, warn_edge, churn * 0.55)
                };
                blend_px(pb, x, y, col, 0.84 * body_fade);
            } else if across >= half - 0.85 || along <= 0.7 || along >= len - 0.7 {
                let dash = ((along - progress * 18.0).rem_euclid(5.0)) < 3.0;
                if dash || is_active {
                    blend_px(pb, x, y, warn_edge, (if is_active { 0.48 } else { 0.92 * pulse }) * body_fade);
                }
            } else {
                let chev = ((along - across - progress * 22.0).rem_euclid(9.0)) < 1.8;
                let a_val = if chev { 0.38 * pulse } else { 0.16 + 0.14 * progress };
                blend_px(pb, x, y, if chev { warn_edge } else { warn_fill }, a_val * body_fade);
            }
        }
    }
}

/// Sister Snip: 45° shear wedge with converging steel blade-closure sweep!
/// Uses pure dot/perp math (zero per-pixel `acos` or `sqrt` outside the cone).
#[allow(clippy::too_many_arguments)]
fn draw_cone_telegraph(
    pb: &mut PixelBuffer,
    origin: Vec2,
    aim: Vec2,
    half_angle: f32,
    len: f32,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let o = origin - cam;
    let dir = safe_dir(aim, Vec2::new(1.0, 0.0));
    let ray_l = dir.rotate(-half_angle);
    let ray_r = dir.rotate(half_angle);
    let perp_l = ray_l.perp();
    let perp_r = ray_r.perp();

    let pad = len + 1.5;
    let x0 = (o.x - pad).floor().max(0.0) as i32;
    let x1 = (o.x + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (o.y - pad).floor().max(0.0) as i32;
    let y1 = (o.y + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;

    let warn_fill: Rgb = (138, 172, 224);
    let warn_edge: Rgb = (232, 244, 255);
    let hot_fill: Rgb = (255, 108, 128);
    let blade_white: Rgb = (255, 255, 255);

    let blade_ang = if is_active {
        half_angle * (1.0 - (progress / 0.45).clamp(0.0, 1.0))
    } else {
        half_angle
    };
    let b_ray_l = dir.rotate(-blade_ang);
    let b_ray_r = dir.rotate(blade_ang);
    let b_perp_l = b_ray_l.perp();
    let b_perp_r = b_ray_r.perp();

    let cos_outer = (half_angle + 0.04).cos();
    let cos_outer2 = cos_outer * cos_outer;
    let cos_blade = blade_ang.cos();
    let cos_blade2 = cos_blade * cos_blade;
    let r_min2 = 25.0_f32; // start cleanly outside Sister Snip's brass pivot medallion
    let r_max2 = (len + 1.0) * (len + 1.0);
    let r_arc_in2 = (len - 1.1).max(0.0) * (len - 1.1).max(0.0);
    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();

    for y in y0..=y1 {
        let vy = y as f32 + 0.5 - o.y;
        let vy2 = vy * vy;
        for x in x0..=x1 {
            let vx = x as f32 + 0.5 - o.x;
            let along = vx * dir.x + vy * dir.y;
            if along <= 0.0 {
                continue;
            }
            let d2 = vx * vx + vy2;
            if d2 < r_min2 || d2 > r_max2 {
                continue;
            }
            let along2 = along * along;
            if along2 < d2 * cos_outer2 {
                continue;
            }
            let arc_edge = d2 >= r_arc_in2;
            let side_edge =
                (vx * perp_l.x + vy * perp_l.y).abs() <= 1.1 || (vx * perp_r.x + vy * perp_r.y).abs() <= 1.1;

            if is_active {
                let on_blade = (vx * b_perp_l.x + vy * b_perp_l.y).abs() <= 1.25
                    || (vx * b_perp_r.x + vy * b_perp_r.y).abs() <= 1.25;
                let swept = along2 <= d2 * cos_blade2;
                if on_blade {
                    blend_px(pb, x, y, blade_white, 0.96);
                } else if swept {
                    blend_px(
                        pb,
                        x,
                        y,
                        mix(hot_fill, blade_white, 0.35 * (1.0 - progress)),
                        0.60 * (1.0 - progress * 0.42),
                    );
                } else if arc_edge || side_edge {
                    blend_px(pb, x, y, warn_edge, 0.78);
                } else {
                    blend_px(pb, x, y, warn_fill, 0.24);
                }
            } else if side_edge {
                blend_px(pb, x, y, warn_edge, 0.94 * pulse);
            } else if arc_edge {
                // Crisp tailor-chalk stitched outer arc.
                let dash = ((x + y) & 3) < 2;
                if dash {
                    blend_px(pb, x, y, warn_edge, 0.88 * pulse);
                }
            } else {
                blend_px(pb, x, y, warn_fill, 0.16 + 0.16 * progress);
            }
        }
    }
}

/// Bobbin: anchored spool + taut rose cord + 90° stitched sweep arc + flying silver needle!
#[allow(clippy::too_many_arguments)]
fn draw_cord_telegraph(
    pb: &mut PixelBuffer,
    spool: Vec2,
    tip: Vec2,
    half: f32,
    sweep: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let s = spool - cam;
    let base_v = tip - spool;
    let len = base_v.len();
    if len < 0.5 {
        return;
    }
    let base_dir = base_v * (1.0 / len);
    let rose_edge: Rgb = (255, 174, 212);
    let rose_fill: Rgb = (218, 72, 132);
    let hot_core: Rgb = (255, 246, 252);
    let silver: Rgb = (238, 246, 255);

    // 0. The swept sector itself, through the tell: a sparse wash over every
    //    place the cord will pass and a dotted line down both of its edges, so
    //    the warning is the footprint and not just its rim.
    let half_sweep = sweep * 0.5;
    if !is_active {
        let rings = (len / 3.0) as i32;
        for r in 2..=rings {
            let rr = r as f32 * 3.0;
            let n = ((rr * sweep / 3.0).ceil() as i32).max(2);
            for i in 0..=n {
                let ang = -half_sweep + sweep * (i as f32 / n as f32);
                let pt = s + base_dir.rotate(ang) * rr;
                blend_px(pb, pt.x.round() as i32, pt.y.round() as i32, rose_fill, 0.16 + 0.16 * progress);
            }
        }
        for &side in &[-1.0_f32, 1.0] {
            let edge = base_dir.rotate(side * half_sweep);
            let mut d = 5.0;
            while d <= len + half {
                let pt = s + edge * d;
                blend_px(pb, pt.x.round() as i32, pt.y.round() as i32, rose_edge, 0.66);
                d += 2.0;
            }
        }
    }

    // 1. Stitched sweep-arc preview: the row's own sweep (`±sweep/2` around
    //    `base_dir`) at the cord's far edge.
    let arc_steps = ((len * sweep * 0.9).ceil() as i32).clamp(18, 120);
    for i in 0..=arc_steps {
        let u = i as f32 / arc_steps as f32;
        let ang = -half_sweep + u * sweep;
        let pt = s + base_dir.rotate(ang) * (len + half);
        if i % 2 == 0 || is_active {
            blend_px(
                pb,
                pt.x.round() as i32,
                pt.y.round() as i32,
                rose_edge,
                if is_active { 0.45 } else { 0.72 },
            );
        }
    }
    // Small directional sweep chevron at the far end of the arc so swing direction is obvious!
    let end_dir = base_dir.rotate(half_sweep);
    let end_pt = s + end_dir * (len + half);
    let tang = end_dir.perp();
    for &side in &[-1.0_f32, 1.0] {
        let tip_c = end_pt - tang * 3.5 + end_dir * (side * 2.5);
        pb.line(end_pt.x.round() as i32, end_pt.y.round() as i32, tip_c.x.round() as i32, tip_c.y.round() as i32, rose_edge);
    }

    // 2. Active sweeping cord (from `active_shape`) or Tell initial cord.
    let (live_s, live_t, live_half) = match active_shape {
        Some(MinibossShape::Cord {
            spool: aspool,
            tip: atip,
            half: ahalf,
        }) => (aspool - cam, atip - cam, ahalf),
        // Through the tell the cord waits on the edge its swing starts from.
        _ => (s, s + base_dir.rotate(-half_sweep) * len, half),
    };

    // During Active, paint a lush multi-ribbon crimson smear wake behind the sweeping cord!
    if is_active {
        let cur_dir = safe_dir(live_t - live_s, base_dir);
        for trail in 1..=4 {
            let lag_dir = cur_dir.rotate(-(trail as f32) * 0.085);
            let lag_tip = live_s + lag_dir * len;
            let a_val = 0.36 * (1.0 - trail as f32 * 0.19);
            let steps = (len as i32 / 2).max(12);
            for k in 2..=steps {
                let q = live_s + (lag_tip - live_s) * (k as f32 / steps as f32);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, rose_fill, a_val);
            }
        }
    }

    // Exact cord band (half-thickness `live_half`).
    let ldelta = live_t - live_s;
    let llen = ldelta.len().max(0.5);
    let ldir = ldelta * (1.0 / llen);
    let lperp = ldir.perp();
    let pad = live_half + 1.2;
    let x0 = (live_s.x.min(live_t.x) - pad).floor().max(0.0) as i32;
    let x1 = (live_s.x.max(live_t.x) + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (live_s.y.min(live_t.y) - pad).floor().max(0.0) as i32;
    let y1 = (live_s.y.max(live_t.y) + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;
    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();

    for y in y0..=y1 {
        let qy = y as f32 + 0.5 - live_s.y;
        for x in x0..=x1 {
            let qx = x as f32 + 0.5 - live_s.x;
            let along = qx * ldir.x + qy * ldir.y;
            if along < 4.5 || along > llen {
                continue;
            }
            let across = (qx * lperp.x + qy * lperp.y).abs();
            if across > live_half + 0.5 {
                continue;
            }
            if is_active {
                let col = if across < 0.9 {
                    hot_core
                } else if across < live_half - 0.5 {
                    rose_edge
                } else {
                    rose_fill
                };
                blend_px(pb, x, y, col, 0.92);
            } else if across < 0.8 {
                let dash = ((along - progress * 18.0).rem_euclid(5.0)) < 3.0;
                if dash {
                    blend_px(pb, x, y, rose_edge, 0.94 * pulse);
                }
            } else if across >= live_half - 0.65 {
                blend_px(pb, x, y, rose_fill, 0.56 * pulse);
            } else {
                blend_px(pb, x, y, rose_fill, 0.20 + 0.12 * progress);
            }
        }
    }

    // Gleaming flying silver sewing needle + brass ferrule right at `live_t`!
    let n_back = live_t - ldir * 4.5;
    let n_tip = live_t + ldir * 3.5;
    draw_tapered_capsule(pb, n_back, n_tip, 1.5, 0.4, (255, 255, 255), silver, (96, 110, 132), 0.0);
}

/// Visit only the exact horizontal scanline chords of ring `[r_in, r_out]`
/// outside the 60° missing-note gap (skips empty interior & corners -> ~6× faster).
#[allow(clippy::too_many_arguments)]
#[inline]
fn for_ring_pixels(
    pb: &mut PixelBuffer,
    o: Vec2,
    r_in: f32,
    r_out: f32,
    gdir: Vec2,
    cos_gap2: f32,
    mut f_px: impl FnMut(&mut PixelBuffer, i32, i32, f32),
) {
    let r_in = r_in.max(2.0);
    if r_out <= r_in {
        return;
    }
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let r_in2 = r_in * r_in;
    let r_out2 = r_out * r_out;
    let y0 = (o.y - r_out).floor().max(0.0) as i32;
    let y1 = (o.y + r_out).ceil().min((bh - 1) as f32) as i32;
    for y in y0..=y1 {
        let vy = y as f32 + 0.5 - o.y;
        let vy2 = vy * vy;
        if vy2 > r_out2 {
            continue;
        }
        let hx_out = (r_out2 - vy2).sqrt();
        let spans: [(i32, i32); 2] = if vy2 < r_in2 {
            let hx_in = (r_in2 - vy2).sqrt();
            [
                ((o.x - hx_out).floor() as i32, (o.x - hx_in).ceil() as i32),
                ((o.x + hx_in).floor() as i32, (o.x + hx_out).ceil() as i32),
            ]
        } else {
            [
                ((o.x - hx_out).floor() as i32, (o.x + hx_out).ceil() as i32),
                (1, 0),
            ]
        };
        for &(sx0, sx1) in &spans {
            let x0 = sx0.max(0);
            let x1 = sx1.min(bw - 1);
            for x in x0..=x1 {
                let vx = x as f32 + 0.5 - o.x;
                let d2 = vx * vx + vy2;
                if d2 < r_in2 || d2 > r_out2 {
                    continue;
                }
                let gdot = vx * gdir.x + vy * gdir.y;
                if gdot > 0.0 && gdot * gdot >= d2 * cos_gap2 {
                    continue;
                }
                f_px(pb, x, y, d2);
            }
        }
    }
}

/// Bell Diver: fast scanline-chord expanding acoustic ring with golden capped
/// 60° missing-note gap aligned strictly to Bell Diver's chipped rim!
#[allow(clippy::too_many_arguments)]
fn draw_ring_telegraph(
    pb: &mut PixelBuffer,
    origin: Vec2,
    radius: f32,
    half: f32,
    gap_dir: Vec2,
    gap_half: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let o = origin - cam;
    let gdir = safe_dir(gap_dir, Vec2::new(1.0, 0.0));
    let cos_gap = gap_half.cos();
    let cos_gap2 = cos_gap * cos_gap;
    let teal_fill: Rgb = (72, 198, 184);
    let teal_edge: Rgb = (164, 252, 238);
    let hot_core: Rgb = (244, 255, 252);
    let gap_cap: Rgb = (255, 228, 118);

    let (live_r, live_half) = match active_shape {
        Some(MinibossShape::RingGap {
            radius: ar,
            half: ah,
            ..
        }) => (ar, ah),
        _ => (radius, half),
    };

    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();

    // 1. Target outer warning ring (bold in Tell, soft rim guide in Active).
    let t_in = (radius - half).max(2.0);
    let t_out = radius + half;
    let t_rim_lo2 = (t_in + 1.1) * (t_in + 1.1);
    let t_rim_hi2 = (t_out - 1.1).max(0.0) * (t_out - 1.1).max(0.0);
    for_ring_pixels(pb, o, t_in, t_out, gdir, cos_gap2, |pb, x, y, d2| {
        let on_rim = d2 <= t_rim_lo2 || d2 >= t_rim_hi2;
        if is_active {
            if on_rim {
                blend_px(pb, x, y, teal_edge, 0.30);
            }
        } else if on_rim {
            blend_px(pb, x, y, teal_edge, 0.90 * pulse);
        } else {
            blend_px(pb, x, y, teal_fill, 0.16 + 0.16 * progress);
        }
    });

    // 2. Live expanding sonic shockwave ring during Active!
    if is_active {
        let l_in = (live_r - live_half).max(2.0);
        let l_out = live_r + live_half;
        let inv_lh = 1.0 / live_half.max(0.5);
        for_ring_pixels(pb, o, l_in, l_out, gdir, cos_gap2, |pb, x, y, d2| {
            let core = 1.0 - (d2.sqrt() - live_r).abs() * inv_lh;
            let col = if core > 0.58 {
                hot_core
            } else if core > 0.22 {
                teal_edge
            } else {
                teal_fill
            };
            blend_px(pb, x, y, col, 0.50 + 0.44 * core);
        });
    }

    // Crisp gold gap end-caps + subtle dashed safe-corridor guide lines radiating
    // straight from Bell Diver's chipped rim through the 60° missing-note gap!
    for &sign in &[-1.0_f32, 1.0] {
        let edge_dir = gdir.rotate(sign * gap_half);
        let r_cap = if is_active { live_r } else { radius };
        let p_in = o + edge_dir * (r_cap - half).max(3.0);
        let p_out = o + edge_dir * (r_cap + half);
        pb.line(
            p_in.x.round() as i32,
            p_in.y.round() as i32,
            p_out.x.round() as i32,
            p_out.y.round() as i32,
            gap_cap,
        );
    }
    let steps = 7;
    for k in 2..=steps {
        let r_pt = 13.0 + (k as f32 / steps as f32) * (radius - 13.0);
        let q = o + gdir * r_pt;
        blend_px(
            pb,
            q.x.round() as i32,
            q.y.round() as i32,
            gap_cap,
            if is_active { 0.45 } else { 0.65 * pulse },
        );
    }
}

/// Draw crisp tiny 3×5 numeral (`1`, `2`, or `3`) inside a waiting Foreman stamp box.
fn draw_stamp_digit(pb: &mut PixelBuffer, cx: i32, cy: i32, n: usize, col: Rgb) {
    let rows: [u8; 5] = match n {
        1 => [0b010, 0b110, 0b010, 0b010, 0b111],
        2 => [0b111, 0b001, 0b111, 0b100, 0b111],
        _ => [0b111, 0b001, 0b111, 0b001, 0b111],
    };
    for (r, &bits) in rows.iter().enumerate() {
        for c in 0..3 {
            if (bits >> (2 - c)) & 1 != 0 {
                plot_px(pb, cx - 1 + c, cy - 2 + r as i32, col);
            }
        }
    }
}

/// The Foreman: three sequential 12×12 bureaucratic stamp squares with 1-2-3 digits!
#[allow(clippy::too_many_arguments)]
fn draw_stamps_telegraph(
    pb: &mut PixelBuffer,
    centers: [Vec2; 3],
    half: f32,
    active_shape: Option<MinibossShape>,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let warn_edge: Rgb = (255, 184, 102);
    let warn_fill: Rgb = (196, 82, 58);
    let hot_fill: Rgb = (255, 78, 74);
    let hot_core: Rgb = (255, 246, 196);
    let spent_col: Rgb = (128, 38, 52);

    // Dotted bureaucratic sequence line linking stamp 0 -> 1 -> 2.
    let s0 = centers[0] - cam;
    let s2 = centers[2] - cam;
    let seg = s2 - s0;
    let slen = seg.len();
    if slen > 1.0 {
        let steps = (slen / 3.0).ceil() as i32;
        for i in 0..=steps {
            if i % 2 == 0 {
                let q = s0 + seg * (i as f32 / steps as f32);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, warn_edge, 0.48);
            }
        }
    }

    let active_center = match active_shape {
        Some(MinibossShape::Stamps { centers: ac, .. }) => Some(ac[0]),
        _ => None,
    };
    let active_idx = if is_active {
        active_center
            .and_then(|ac| centers.iter().position(|c| c.dist_sq(ac) < 1.0))
            .unwrap_or(((progress * 3.0).floor() as usize).min(2))
    } else {
        usize::MAX
    };

    let s = half.round().max(2.0) as i32;
    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();
    let bw = pb.w as i32;
    let bh = pb.h as i32;

    for (idx, &c) in centers.iter().enumerate() {
        let p = c - cam;
        let cx = p.x.round() as i32;
        let cy = p.y.round() as i32;
        if cx + s < 0 || cy + s < 0 || cx - s >= bw || cy - s >= bh {
            continue;
        }
        let is_now = is_active && idx == active_idx;
        let is_done = is_active && idx < active_idx;

        let x0 = (cx - s).max(0);
        let x1 = (cx + s - 1).min(bw - 1);
        let y0 = (cy - s).max(0);
        let y1 = (cy + s - 1).min(bh - 1);
        for y in y0..=y1 {
            let dy = y - cy;
            let on_by = dy == -s || dy == s - 1;
            for x in x0..=x1 {
                let dx = x - cx;
                let on_border = on_by || dx == -s || dx == s - 1;
                if is_now {
                    let col = if on_border { hot_core } else { hot_fill };
                    blend_px(pb, x, y, col, if on_border { 0.96 } else { 0.66 });
                } else if is_done {
                    let on_x = dx == dy || dx == -dy;
                    if on_border || on_x {
                        blend_px(pb, x, y, spent_col, 0.70);
                    } else {
                        blend_px(pb, x, y, spent_col, 0.20);
                    }
                } else if on_border {
                    let corner = dx.abs() >= s - 3 && dy.abs() >= s - 3;
                    let dash = ((dx + dy).rem_euclid(3)) != 0;
                    if corner || dash {
                        blend_px(pb, x, y, warn_edge, 0.92 * pulse);
                    }
                } else {
                    blend_px(pb, x, y, warn_fill, 0.16 + 0.15 * progress);
                }
            }
        }

        if is_now {
            // Bold white-gold stamped DENIED bars inside the live active impact square!
            for dx in -(s - 2)..(s - 2) {
                blend_px(pb, cx + dx, cy - 2, hot_core, 0.94);
                blend_px(pb, cx + dx, cy + 1, hot_core, 0.94);
            }
        } else if !is_done {
            // Clean crisp pixel numeral `1`, `2`, `3`!
            draw_stamp_digit(pb, cx, cy, idx + 1, hot_core);
        }
    }
}

/// Milk Carton: exact oriented 20×32 spill rectangle with animated pour chevrons
/// in Tell and surging frothy sour-milk wave + curds in Active!
#[allow(clippy::too_many_arguments)]
fn draw_spill_telegraph(
    pb: &mut PixelBuffer,
    center: Vec2,
    aim: Vec2,
    half_len: f32,
    half_w: f32,
    cam: Vec2,
    is_active: bool,
    progress: f32,
) {
    let c = center - cam;
    let a = safe_dir(aim, Vec2::new(1.0, 0.0));
    let p = a.perp();

    let corners = [
        c + a * half_len + p * half_w,
        c + a * half_len - p * half_w,
        c - a * half_len - p * half_w,
        c - a * half_len + p * half_w,
    ];
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for q in &corners {
        min_x = min_x.min(q.x);
        max_x = max_x.max(q.x);
        min_y = min_y.min(q.y);
        max_y = max_y.max(q.y);
    }
    let x0 = (min_x - 1.0).floor().max(0.0) as i32;
    let x1 = (max_x + 1.0).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (min_y - 1.0).floor().max(0.0) as i32;
    let y1 = (max_y + 1.0).ceil().min(((pb.h as i32) - 1) as f32) as i32;

    let warn_fill: Rgb = (224, 216, 174);
    let warn_edge: Rgb = (255, 248, 206);
    let milk_body: Rgb = (242, 244, 224);
    let milk_foam: Rgb = (255, 255, 250);
    let curd_yellow: Rgb = (196, 212, 118);
    let hot_rim: Rgb = (255, 216, 126);
    let pulse = 0.74 + 0.26 * (progress * TAU * 3.5).sin();

    for y in y0..=y1 {
        let vy = y as f32 + 0.5 - c.y;
        for x in x0..=x1 {
            let vx = x as f32 + 0.5 - c.x;
            let along = vx * a.x + vy * a.y;
            if along.abs() > half_len + 0.6 {
                continue;
            }
            let across = vx * p.x + vy * p.y;
            if across.abs() > half_w + 0.6 {
                continue;
            }
            let on_border = (along.abs() >= half_len - 0.9) || (across.abs() >= half_w - 0.9);
            // Softly feather the spill where it starts under Milk Carton's body
            // so the tilting carton and red barn stay crisp in front of the pour.
            let body_fade = if along < -half_len + 6.5 { 0.42 } else { 1.0 };
            if is_active {
                if on_border {
                    blend_px(pb, x, y, hot_rim, 0.94 * body_fade);
                } else {
                    let wave = (along * 0.55 - progress * 20.0 + (across * 0.35).sin() * 2.0).sin() * 0.5 + 0.5;
                    let curd = ((along as i32 * 3 + across as i32 * 5).rem_euclid(13)) == 0;
                    let col = if curd {
                        curd_yellow
                    } else if wave > 0.72 {
                        milk_foam
                    } else {
                        milk_body
                    };
                    blend_px(pb, x, y, col, 0.80 * body_fade);
                }
            } else if on_border {
                let dash = ((along + across - progress * 16.0).rem_euclid(5.0)) < 3.0;
                if dash {
                    blend_px(pb, x, y, warn_edge, 0.92 * pulse * body_fade);
                }
            } else {
                let chev = ((along - across.abs() * 0.8 - progress * 18.0).rem_euclid(8.0)) < 1.6;
                blend_px(
                    pb,
                    x,
                    y,
                    if chev { warn_edge } else { warn_fill },
                    (if chev { 0.38 * pulse } else { 0.16 + 0.14 * progress }) * body_fade,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::hazards::SteamPhase;
    use crate::game::level::{Prop, PropKind, Rectf, Theme};
    use crate::game::world::World;
    use crate::render::lighting::LightingMode;
    use std::path::Path;
    use std::time::Instant;

    #[test]
    fn all_six_minibosses_and_poses_render_cleanly() {
        let mut pb = PixelBuffer::new(96, 72);
        let cam = Vec2::new(100.0, 100.0);
        let origin = Vec2::new(160.0, 136.0);
        let aim = Vec2::new(-1.0, 0.0);
        let kinds = [
            MinibossKind::Corkscrew,
            MinibossKind::SisterSnip,
            MinibossKind::Bobbin,
            MinibossKind::BellDiver,
            MinibossKind::Foreman,
            MinibossKind::MilkCarton,
        ];
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(9);
        for kind in kinds {
            for state in [
                MinibossState::Approach,
                MinibossState::Tell,
                MinibossState::Active,
                MinibossState::Recover,
                MinibossState::Cooldown,
            ] {
                for alive in [true, false] {
                    pb.clear((16, 18, 22));
                    let mut e = Enemy::spawn(crate::game::enemy::EnemyKind::Brute, origin, 1.0, &mut rng);
                    e.hp = if alive { 120.0 } else { 0.0 };
                    e.max_hp = 160.0;
                    e.radius = kind.body_radius();
                    e.facing = aim;
                    e.vel = if state == MinibossState::Approach { aim * 30.0 } else { Vec2::ZERO };
                    e.flash = if state == MinibossState::Recover { 0.1 } else { 0.0 };
                    let mut mb = Miniboss::new(kind, 0);
                    mb.origin = origin;
                    mb.aim = aim;
                    mb.state = state;
                    mb.timer = 0.4;
                    mb.stamp_centers = [origin + aim * 16.0, origin + aim * 32.0, origin + aim * 48.0];
                    draw_telegraph(&mut pb, &mb, cam, (90, 220, 200));
                    draw(&mut pb, &e, &mb, cam, (90, 220, 200));
                    let changed = pb.pixels().iter().filter(|&&c| c != (16, 18, 22)).count();
                    assert!(changed >= 40, "{kind:?}/{state:?}/alive={alive}: too few pixels ({changed})");
                }
            }
        }
    }

    fn write_ppm(path: &Path, pb: &PixelBuffer) {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for &(r, g, b) in pb.pixels() {
            out.push(r);
            out.push(g);
            out.push(b);
        }
        std::fs::write(path, out).unwrap();
    }

    fn place_view_at(world: &mut World, center: Vec2, w: usize, h: usize, mode: LightingMode) -> PixelBuffer {
        world.player.pos = center;
        world.lighting = mode;
        world.cam = center - Vec2::new(w as f32 * 0.5, h as f32 * 0.5);
        let mut pb = PixelBuffer::new(w, h);
        world.draw(&mut pb);
        pb
    }

    /// Gated behind `RATRUN_ART4_CAPTURE=<dir>`: captures the full art review
    /// gallery (1x & 2x zoom, Full & Off lighting, hazards in all states,
    /// shop holes on both floors, props showcase, and all 6 minibosses in
    /// every pose + telegraph) and measures 120x40 (120x80 px) draw cost.
    #[test]
    fn capture_art4_levels_and_minibosses() {
        let Ok(dir_str) = std::env::var("RATRUN_ART4_CAPTURE") else {
            return;
        };
        let dir = Path::new(&dir_str);
        std::fs::create_dir_all(dir).unwrap();

        // 1. Floor captures (1x = 120x80 px, 2x zoom-out = 240x160 px; Full & Off)
        //    plus 120x40 (120x80 px) draw-cost measurement on each floor.
        for (name, theme) in [("laundry", Theme::Laundry), ("printworks", Theme::Printworks)] {
            let mut world = World::new(theme, 7);
            world.clear_enemies();
            // Pick a hall with furniture + props so we see walls, floor, props, landmark.
            let focus = world
                .level
                .rooms_of(crate::game::level::RoomKind::Hall)
                .map(|r| r.rect)
                .find(|r| r.area() > 600)
                .map(|r| r.center_world() + Vec2::new(24.0, 12.0))
                .unwrap_or(world.player.pos);
            world.elapsed = 4.2;

            for (mname, mode) in [("full", LightingMode::Full), ("off", LightingMode::Off)] {
                let pb1 = place_view_at(&mut world, focus, 120, 80, mode);
                write_ppm(&dir.join(format!("{name}-1x-{mname}.ppm")), &pb1);
                let pb2 = place_view_at(&mut world, focus, 240, 160, mode);
                write_ppm(&dir.join(format!("{name}-2x-{mname}.ppm")), &pb2);
            }

            // Benchmark 120x40 view (120x80 half-block pixels) with Full and Off lighting.
            for (mname, mode) in [("full", LightingMode::Full), ("off", LightingMode::Off)] {
                world.player.pos = focus;
                world.lighting = mode;
                world.cam = focus - Vec2::new(60.0, 40.0);
                let mut pb = PixelBuffer::new(120, 80);
                // Warmup
                for _ in 0..5 {
                    world.draw(&mut pb);
                }
                let iters = 40u32;
                let t0 = Instant::now();
                for _ in 0..iters {
                    world.draw(&mut pb);
                }
                let us = t0.elapsed().as_secs_f64() * 1e6 / iters as f64;
                println!("[art4 draw bench] {name} 120x40 ({mname}): {us:.1} us/frame");
            }

            // Shop hole capture (first shop hole on this floor, Full & Off).
            if let Some(hole) = world.level.objects.shop_holes.first().copied() {
                let spot = hole.pos + hole.facing * 18.0;
                world.elapsed = 2.0;
                let pb_s = place_view_at(&mut world, spot, 120, 80, LightingMode::Full);
                write_ppm(&dir.join(format!("{name}-shop-full.ppm")), &pb_s);
                let pb_so = place_view_at(&mut world, spot, 120, 80, LightingMode::Off);
                write_ppm(&dir.join(format!("{name}-shop-off.ppm")), &pb_so);
            }

            // Boss arena capture (2x zoom-out so lanes, drain/page, perimeter & doors show).
            if let Some(arena) = world.level.boss_room().map(|r| r.center()) {
                let pb_a = place_view_at(&mut world, arena, 240, 160, LightingMode::Full);
                write_ppm(&dir.join(format!("{name}-arena-2x-full.ppm")), &pb_a);
            }
        }

        // 2. Steam grate states (Resting, Warning x2, Plume) & Ink pad (Full & Off)
        {
            let mut world = World::new(Theme::Laundry, 7);
            world.clear_enemies();
            if let Some(g0) = world.level.objects.steam_grates.first().copied() {
                let spot = g0.pos + Vec2::new(0.0, 14.0);
                for (sname, phase, timer) in [
                    ("resting", SteamPhase::Resting, 1.0),
                    ("warn-early", SteamPhase::Warning, 0.25),
                    ("warn-late", SteamPhase::Warning, 0.78),
                    ("plume", SteamPhase::Plume, 0.30),
                ] {
                    world.level.objects.steam_grates[0].phase = phase;
                    world.level.objects.steam_grates[0].timer = timer;
                    world.elapsed = 5.0;
                    let pb = place_view_at(&mut world, spot, 120, 80, LightingMode::Full);
                    write_ppm(&dir.join(format!("hazard-steam-{sname}.ppm")), &pb);
                }
            }

            let mut pw = World::new(Theme::Printworks, 7);
            pw.clear_enemies();
            if let Some(pad) = pw.level.objects.ink_pads.first().copied() {
                let spot = pad.pos + Vec2::new(0.0, 14.0);
                pw.elapsed = 5.0;
                let pb_f = place_view_at(&mut pw, spot, 120, 80, LightingMode::Full);
                write_ppm(&dir.join("hazard-ink-full.ppm"), &pb_f);
                let pb_o = place_view_at(&mut pw, spot, 120, 80, LightingMode::Off);
                write_ppm(&dir.join("hazard-ink-off.ppm"), &pb_o);
            }
        }

        // 3. Props showcase on Laundry and Printworks (all 3 props + wall + rat, Full & Off)
        for (name, theme, kinds) in [
            ("laundry", Theme::Laundry, [PropKind::Washer, PropKind::TowelRoll, PropKind::Clothespin]),
            ("printworks", Theme::Printworks, [PropKind::PressIsland, PropKind::PaperReel, PropKind::TypeBlock]),
        ] {
            let mut world = World::new(theme, 7);
            world.clear_enemies();
            let center = world.level.boss_room().map(|r| r.center()).unwrap_or(world.player.pos);
            world.level.props.clear();
            let mut x = center.x - 76.0;
            for kind in kinds {
                let (pw, ph) = kind.size();
                world.level.props.push(Prop {
                    kind,
                    rect: Rectf::new(x, center.y - ph * 0.5 - 6.0, pw, ph),
                });
                x += pw + 10.0;
            }
            world.elapsed = 4.0;
            let pb_f = place_view_at(&mut world, center + Vec2::new(0.0, 12.0), 180, 100, LightingMode::Full);
            write_ppm(&dir.join(format!("props-{name}-full.ppm")), &pb_f);
            let pb_o = place_view_at(&mut world, center + Vec2::new(0.0, 12.0), 180, 100, LightingMode::Off);
            write_ppm(&dir.join(format!("props-{name}-off.ppm")), &pb_o);
        }

        // 4. All 6 minibosses across every pose + telegraph contact sheet!
        //    Columns: Idle, Approach, Tell, Strike, Recover, Retreat, Hurt, Death
        let all_kinds = [
            ("corkscrew", MinibossKind::Corkscrew),
            ("snip", MinibossKind::SisterSnip),
            ("bobbin", MinibossKind::Bobbin),
            ("bell", MinibossKind::BellDiver),
            ("foreman", MinibossKind::Foreman),
            ("milk", MinibossKind::MilkCarton),
        ];
        const CELL_W: usize = 96;
        const CELL_H: usize = 72;
        const COLS: usize = 8;
        let mut sheet = PixelBuffer::new(CELL_W * COLS, CELL_H * all_kinds.len());
        let sewer_level = crate::game::level::Level::empty(Theme::Sewer);
        let accent = sewer_level.palette.accent;
        for (row, &(_kname, kind)) in all_kinds.iter().enumerate() {
            for col in 0..COLS {
                let mut cell = PixelBuffer::new(CELL_W, CELL_H);
                crate::game::art_floor::draw_floor(
                    &mut cell,
                    &sewer_level,
                    Vec2::new(160.0, 160.0),
                    2.0,
                );
                let origin = Vec2::new(222.0, 196.0);
                let aim = Vec2::new(-1.0, 0.0);
                let mut rng = rand::rngs::StdRng::seed_from_u64(42);
                use rand::SeedableRng;
                let mut e = Enemy::spawn(crate::game::enemy::EnemyKind::Brute, origin, 1.0, &mut rng);
                e.hp = 160.0;
                e.max_hp = 160.0;
                e.radius = kind.body_radius();
                e.named = true;
                e.facing = aim;
                e.anim = 1.4;

                let mut mb = Miniboss::new(kind, 0);
                mb.origin = origin;
                mb.aim = aim;
                mb.stamp_centers = [
                    origin + aim * 16.0,
                    origin + aim * 32.0,
                    origin + aim * 48.0,
                ];

                match col {
                    0 => {
                        // Idle
                        mb.state = MinibossState::Approach;
                        e.vel = Vec2::ZERO;
                        mb.timer = 0.4;
                    }
                    1 => {
                        // Approach
                        mb.state = MinibossState::Approach;
                        e.vel = aim * 38.0;
                        mb.timer = 0.8;
                    }
                    2 => {
                        // Tell + telegraph
                        mb.state = MinibossState::Tell;
                        mb.timer = 0.65;
                    }
                    3 => {
                        // Strike (Active) + telegraph
                        mb.state = MinibossState::Active;
                        mb.timer = kind.active_duration() * 0.55;
                    }
                    4 => {
                        // Recover (exposed)
                        mb.state = MinibossState::Recover;
                        mb.timer = 0.45;
                    }
                    5 => {
                        // Retreat / Cooldown
                        mb.state = MinibossState::Cooldown;
                        e.vel = -aim * 28.0;
                        mb.timer = 0.7;
                    }
                    6 => {
                        // Hurt flash
                        mb.state = MinibossState::Approach;
                        e.flash = 0.11;
                        e.hp = 96.0;
                    }
                    _ => {
                        // Death
                        mb.state = MinibossState::Recover;
                        e.hp = 0.0;
                        mb.timer = 0.6;
                    }
                }

                let cam = Vec2::new(160.0, 160.0);
                // Place a rat silhouette on the left for scale in Tell & Strike cells.
                if col == 2 || col == 3 {
                    let mut p = crate::game::player::Player::new(
                        origin + Vec2::new(-38.0, 0.0),
                        crate::game::weapon::WeaponKind::Gnaw,
                    );
                    p.face = Vec2::new(1.0, 0.0);
                    crate::game::art_rat::draw_rat(&mut cell, &p, 0.5, cam);
                }
                if e.alive() {
                    draw_telegraph(&mut cell, &mb, cam, accent);
                }
                draw(&mut cell, &e, &mb, cam, accent);

                // Blit cell into sheet with a subtle divider border.
                let ox = col * CELL_W;
                let oy = row * CELL_H;
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        let c = if x == 0 || y == 0 {
                            (40, 44, 52)
                        } else {
                            cell.pixel_at(x, y)
                        };
                        sheet.plot((ox + x) as i32, (oy + y) as i32, c);
                    }
                }
            }
        }
        write_ppm(&dir.join("minibosses-sheet.ppm"), &sheet);
    }

    /// Gated behind `RATRUN_ART5_CAPTURE=<dir>`: captures every miniboss across
    /// every state through the REAL `World::draw` (Full lighting) at 1x & 2x
    /// zoom-out on a dark theme (`Sewer`) and a bright theme (`Kitchen`), plus
    /// 8–12 frame filmstrips of Entrance, Approach, full Attack (Tell → Strike →
    /// Recover), and Death, and prints per-miniboss draw microseconds.
    #[test]
    fn capture_art5_minibosses() {
        let Ok(dir_str) = std::env::var("RATRUN_ART5_CAPTURE") else {
            return;
        };
        let dir = Path::new(&dir_str);
        std::fs::create_dir_all(dir).unwrap();

        let all_kinds = [
            ("corkscrew", MinibossKind::Corkscrew),
            ("snip", MinibossKind::SisterSnip),
            ("bobbin", MinibossKind::Bobbin),
            ("bell", MinibossKind::BellDiver),
            ("foreman", MinibossKind::Foreman),
            ("milk", MinibossKind::MilkCarton),
        ];

        // Helper: build a clean World on an open boss-arena floor with lighting Full.
        fn make_stage(theme: Theme, zoom: f32) -> (World, Vec2) {
            let mut w = World::new(theme, 11);
            w.clear_enemies();
            w.level.props.clear();
            w.lighting = LightingMode::Full;
            w.set_zoom_instant(zoom);
            w.elapsed = 190.0;
            w.ecs
                .resource_mut::<crate::game::ecs::systems::miniboss::MinibossSchedule>()
                .next = 4;
            let focus = w
                .level
                .boss_room()
                .map(|r| r.center())
                .unwrap_or(w.player.pos);
            (w, focus)
        }

        #[allow(clippy::too_many_arguments)]
        fn render_frame(
            w: &mut World,
            focus: Vec2,
            kind: MinibossKind,
            state: MinibossState,
            timer: f32,
            anim: f32,
            vel: Vec2,
            flash: f32,
            alive: bool,
            is_entrance: bool,
            mb_offset: Vec2,
            cell_w: usize,
            cell_h: usize,
        ) -> PixelBuffer {
            w.clear_enemies();
            // Position rat slightly left of center and miniboss slightly right so
            // both rat and miniboss + full 48px telegraph fit inside the frame.
            let rat_pos = focus + Vec2::new(-24.0, 2.0);
            let mb_pos = focus + Vec2::new(22.0, 0.0) + mb_offset;
            w.player.pos = rat_pos;
            w.player.face = Vec2::new(1.0, 0.0);
            let aim = Vec2::new(-1.0, 0.0);

            let ent = w.spawn_miniboss(kind, 0, mb_pos);
            {
                let mut e = w.ecs.get_mut::<Enemy>(ent).unwrap();
                e.hp = if alive {
                    if flash > 0.0 { 96.0 } else { 160.0 }
                } else {
                    0.0
                };
                e.max_hp = 160.0;
                e.facing = aim;
                e.vel = vel;
                e.anim = anim;
                e.flash = flash;
            }
            {
                let mut mb = w.ecs.get_mut::<Miniboss>(ent).unwrap();
                mb.state = state;
                mb.timer = timer;
                mb.aim = aim;
                mb.announced = true;
                if is_entrance {
                    mb.origin = Vec2::ZERO;
                    mb.tell_cued = false;
                } else {
                    mb.origin = mb_pos;
                    mb.tell_cued = true;
                }
                mb.stamp_centers = [
                    mb_pos + aim * 16.0,
                    mb_pos + aim * 32.0,
                    mb_pos + aim * 48.0,
                ];
            }
            // Center camera nicely between rat and miniboss by placing player at
            // the midpoint during draw? No: keep real rat visible! Place rat at
            // `focus + (-26, 2)` and miniboss at `focus + (18, 0)`, so relative
            // to rat (screen center), miniboss sits at `(+44, -2)` and its left
            // telegraph sweeps right across the rat at `(0, 0)`!
            // Actually, center the frame on `focus` by putting rat at `focus + (-20, 4)`
            // and miniboss at `focus + (18, 0)` (38 px apart, right in Tell range!).
            w.player.pos = focus + Vec2::new(-16.0, 2.0);
            {
                let mut e = w.ecs.get_mut::<Enemy>(ent).unwrap();
                e.pos = focus + Vec2::new(20.0, 0.0) + mb_offset;
            }
            {
                let mut mb = w.ecs.get_mut::<Miniboss>(ent).unwrap();
                if !is_entrance {
                    let p = focus + Vec2::new(20.0, 0.0) + mb_offset;
                    mb.origin = p;
                    mb.stamp_centers = [p + aim * 16.0, p + aim * 32.0, p + aim * 48.0];
                }
            }
            w.elapsed = 190.0 + anim * 0.15 + timer;
            let mut pb = PixelBuffer::new(cell_w, cell_h);
            w.draw(&mut pb);
            pb
        }

        fn blit_cell(
            sheet: &mut PixelBuffer,
            cell: &PixelBuffer,
            ox: usize,
            oy: usize,
            border: Rgb,
        ) {
            for y in 0..cell.h {
                for x in 0..cell.w {
                    let c = if x == 0 || y == 0 {
                        border
                    } else {
                        cell.pixel_at(x, y)
                    };
                    sheet.plot((ox + x) as i32, (oy + y) as i32, c);
                }
            }
        }

        // 1. Master state sheets (6 rows × 9 cols: Entrance, Idle, Approach,
        //    Tell, Strike, Recover, Retreat, Hurt, Death) across
        //    {dark=Sewer, bright=Kitchen} × {1x, 2x zoom-out}.
        for (tname, theme) in [("dark", Theme::Sewer), ("bright", Theme::Kitchen)] {
            for (zname, zoom, cw, ch) in [("1x", 1.0_f32, 112usize, 84usize), ("2x", 2.0, 96, 72)] {
                let (mut w, focus) = make_stage(theme, zoom);
                let cols = 9usize;
                let mut sheet = PixelBuffer::new(cw * cols, ch * all_kinds.len());
                for (row, &(_kname, kind)) in all_kinds.iter().enumerate() {
                    let aim = Vec2::new(-1.0, 0.0);
                    let specs: [(MinibossState, f32, f32, Vec2, f32, bool, bool); 9] = [
                        // 0: Entrance
                        (MinibossState::Approach, 0.34, 0.9, aim * 36.0, 0.0, true, true),
                        // 1: Idle
                        (MinibossState::Approach, 2.0, 1.4, Vec2::ZERO, 0.0, true, false),
                        // 2: Approach
                        (MinibossState::Approach, 2.2, 2.6, aim * 38.0, 0.0, true, false),
                        // 3: Tell (held beat)
                        (MinibossState::Tell, 0.72, 1.8, Vec2::ZERO, 0.0, true, false),
                        // 4: Strike (Active)
                        (MinibossState::Active, kind.active_duration() * 0.48, 1.8, Vec2::ZERO, 0.0, true, false),
                        // 5: Recover (exposed)
                        (MinibossState::Recover, 0.42, 2.1, Vec2::ZERO, 0.0, true, false),
                        // 6: Retreat (Cooldown)
                        (MinibossState::Cooldown, 0.9, 3.0, -aim * 28.0, 0.0, true, false),
                        // 7: Hurt
                        (MinibossState::Approach, 2.0, 1.6, aim * 20.0, 0.11, true, false),
                        // 8: Death
                        (MinibossState::Recover, 0.65, 2.0, Vec2::ZERO, 0.0, false, false),
                    ];
                    for (col, &(st, tm, an, vel, fl, alive, ent)) in specs.iter().enumerate() {
                        let cell = render_frame(
                            &mut w,
                            focus,
                            kind,
                            st,
                            tm,
                            an,
                            vel,
                            fl,
                            alive,
                            ent,
                            Vec2::ZERO,
                            cw,
                            ch,
                        );
                        blit_cell(&mut sheet, &cell, col * cw, row * ch, (44, 48, 58));
                    }
                }
                write_ppm(&dir.join(format!("sheet-{tname}-{zname}.ppm")), &sheet);
            }
        }

        // 2. Per-miniboss animated filmstrips (through real World::draw, Full lighting):
        //    - approach (10 frames, dark 1x)
        //    - attack (12 frames: 4 Tell + 4 Strike + 4 Recover, dark 1x)
        //    - attack-bright-2x (12 frames, bright Kitchen 2x zoom-out)
        //    - death (8 frames, dark 1x)
        //    - entrance (8 frames, dark 1x)
        let (mut w_dark_1x, focus_dark) = make_stage(Theme::Sewer, 1.0);
        let (mut w_bright_2x, focus_bright) = make_stage(Theme::Kitchen, 2.0);
        let aim = Vec2::new(-1.0, 0.0);

        for &(kname, kind) in &all_kinds {
            // Approach: 10 consecutive frames
            let cw = 96usize;
            let ch = 76usize;
            let mut strip = PixelBuffer::new(cw * 10, ch);
            for i in 0..10 {
                let u = i as f32 / 9.0;
                let anim = u * 9.0;
                let off = Vec2::new((1.0 - u) * 10.0 - 8.0, 0.0);
                let cell = render_frame(
                    &mut w_dark_1x,
                    focus_dark,
                    kind,
                    MinibossState::Approach,
                    2.0 + u * 0.9,
                    anim,
                    aim * 38.0,
                    0.0,
                    true,
                    false,
                    off,
                    cw,
                    ch,
                );
                blit_cell(&mut strip, &cell, i * cw, 0, (44, 48, 58));
            }
            write_ppm(&dir.join(format!("strip-{kname}-approach.ppm")), &strip);

            // Full attack (4 Tell + 4 Strike + 4 Recover = 12 frames) on dark 1x & bright 2x
            for (tag, w, foc, fw, fh) in [
                ("attack", &mut w_dark_1x, focus_dark, 112usize, 84usize),
                ("attack-bright-2x", &mut w_bright_2x, focus_bright, 96usize, 72usize),
            ] {
                let mut astrip = PixelBuffer::new(fw * 12, fh);
                let ad = kind.active_duration();
                let frames: [(MinibossState, f32); 12] = [
                    (MinibossState::Tell, 0.12),
                    (MinibossState::Tell, 0.42),
                    (MinibossState::Tell, 0.72),
                    (MinibossState::Tell, 0.94),
                    (MinibossState::Active, ad * 0.10),
                    (MinibossState::Active, ad * 0.36),
                    (MinibossState::Active, ad * 0.66),
                    (MinibossState::Active, ad * 0.92),
                    (MinibossState::Recover, 0.10),
                    (MinibossState::Recover, 0.38),
                    (MinibossState::Recover, 0.68),
                    (MinibossState::Recover, 0.94),
                ];
                for (i, &(st, tm)) in frames.iter().enumerate() {
                    let cell = render_frame(
                        w,
                        foc,
                        kind,
                        st,
                        tm,
                        1.5 + i as f32 * 0.25,
                        Vec2::ZERO,
                        0.0,
                        true,
                        false,
                        Vec2::ZERO,
                        fw,
                        fh,
                    );
                    blit_cell(&mut astrip, &cell, i * fw, 0, (44, 48, 58));
                }
                write_ppm(&dir.join(format!("strip-{kname}-{tag}.ppm")), &astrip);
            }

            // Death: 8 consecutive frames (0.0 -> 1.0)
            let mut dstrip = PixelBuffer::new(cw * 8, ch);
            for i in 0..8 {
                let u = i as f32 / 7.0;
                let cell = render_frame(
                    &mut w_dark_1x,
                    focus_dark,
                    kind,
                    MinibossState::Recover,
                    u,
                    2.0 + u * 2.0,
                    Vec2::ZERO,
                    if i == 0 { 0.12 } else { 0.0 },
                    false,
                    false,
                    Vec2::ZERO,
                    cw,
                    ch,
                );
                blit_cell(&mut dstrip, &cell, i * cw, 0, (44, 48, 58));
            }
            write_ppm(&dir.join(format!("strip-{kname}-death.ppm")), &dstrip);

            // Entrance: 8 consecutive frames (0.0 -> 1.05s)
            let mut estrip = PixelBuffer::new(cw * 8, ch);
            for i in 0..8 {
                let u = i as f32 / 7.0;
                let cell = render_frame(
                    &mut w_dark_1x,
                    focus_dark,
                    kind,
                    MinibossState::Approach,
                    u * 1.05,
                    u * 5.0,
                    aim * 36.0,
                    0.0,
                    true,
                    true,
                    Vec2::ZERO,
                    cw,
                    ch,
                );
                blit_cell(&mut estrip, &cell, i * cw, 0, (44, 48, 58));
            }
            write_ppm(&dir.join(format!("strip-{kname}-entrance.ppm")), &estrip);
        }

        // 3. Draw-cost microbenchmark for every miniboss (body + telegraph),
        //    taking the median of 5 batches so OS scheduling spikes don't skew.
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut pb = PixelBuffer::new(160, 120);
        let cam = Vec2::new(100.0, 100.0);
        let origin = Vec2::new(180.0, 160.0);
        let accent = (120, 220, 190);
        let iters = 150u32;
        for &(kname, kind) in &all_kinds {
            let mut e = Enemy::spawn(crate::game::enemy::EnemyKind::Brute, origin, 1.0, &mut rng);
            e.hp = 160.0;
            e.max_hp = 160.0;
            e.radius = kind.body_radius();
            e.facing = aim;
            e.anim = 2.4;
            let mut mb = Miniboss::new(kind, 0);
            mb.origin = origin;
            mb.aim = aim;
            mb.state = MinibossState::Active;
            mb.timer = kind.active_duration() * 0.5;
            mb.stamp_centers = [origin + aim * 16.0, origin + aim * 32.0, origin + aim * 48.0];
            for _ in 0..10 {
                draw_telegraph(&mut pb, &mb, cam, accent);
                draw(&mut pb, &e, &mb, cam, accent);
            }
            let mut body_samples = [0.0f64; 5];
            let mut tele_samples = [0.0f64; 5];
            for s in 0..5 {
                let t0 = Instant::now();
                for _ in 0..iters {
                    draw(&mut pb, &e, &mb, cam, accent);
                }
                body_samples[s] = t0.elapsed().as_secs_f64() * 1e6 / iters as f64;
                let t1 = Instant::now();
                for _ in 0..iters {
                    draw_telegraph(&mut pb, &mb, cam, accent);
                }
                tele_samples[s] = t1.elapsed().as_secs_f64() * 1e6 / iters as f64;
            }
            body_samples.sort_by(|a, b| a.total_cmp(b));
            tele_samples.sort_by(|a, b| a.total_cmp(b));
            let body_us = body_samples[2];
            let tele_us = tele_samples[2];
            println!(
                "[art5 bench] {kname:<10} body={body_us:5.1}us  tele={tele_us:5.1}us  total={:5.1}us",
                body_us + tele_us
            );
        }
    }

    #[test]
    fn entrance_hurt_and_death_poses_animate_distinctly_for_every_miniboss() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let cam = Vec2::new(100.0, 100.0);
        let origin = Vec2::new(160.0, 136.0);
        let aim = Vec2::new(-1.0, 0.0);
        for kind in [
            MinibossKind::Corkscrew,
            MinibossKind::SisterSnip,
            MinibossKind::Bobbin,
            MinibossKind::BellDiver,
            MinibossKind::Foreman,
            MinibossKind::MilkCarton,
        ] {
            let mut pb_a = PixelBuffer::new(96, 84);
            let mut pb_b = PixelBuffer::new(96, 84);
            // Entrance early vs settled must produce distinct frames.
            let mut e = Enemy::spawn(crate::game::enemy::EnemyKind::Brute, origin, 1.0, &mut rng);
            e.hp = 160.0;
            e.max_hp = 160.0;
            e.radius = kind.body_radius();
            e.facing = aim;
            e.vel = aim * 36.0;
            let mut mb = Miniboss::new(kind, 0);
            mb.timer = 0.15;
            pb_a.clear((16, 18, 22));
            draw(&mut pb_a, &e, &mb, cam, (90, 220, 200));
            mb.timer = 0.95;
            pb_b.clear((16, 18, 22));
            draw(&mut pb_b, &e, &mb, cam, (90, 220, 200));
            assert_ne!(pb_a.pixels(), pb_b.pixels(), "{kind:?}: entrance should animate");

            // Death early vs settled must produce distinct frames.
            e.hp = 0.0;
            mb.state = MinibossState::Recover;
            mb.timer = 0.05;
            pb_a.clear((16, 18, 22));
            draw(&mut pb_a, &e, &mb, cam, (90, 220, 200));
            mb.timer = 0.85;
            pb_b.clear((16, 18, 22));
            draw(&mut pb_b, &e, &mb, cam, (90, 220, 200));
            assert_ne!(pb_a.pixels(), pb_b.pixels(), "{kind:?}: death should animate");
        }
    }
}
