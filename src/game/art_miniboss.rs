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
    // A cast shadow is floor, not body: stamp it with the floor pen so the
    // lighting's rim and sheen never treat it as a creature (batch 13).
    let prev_pen = pb.set_pen(0);
    draw_shadow_body(pb, cx, cy, rx, ry, strength);
    pb.set_pen(prev_pen);
}

fn draw_shadow_body(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
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

// ============================================================================
// Private helpers for minibosses 7–15 (Sewer, Kitchen, Lab).
// Prefixed `mb_a_` to stay isolated from sibling miniboss art passes.
// ============================================================================

/// Arrival drop with landing squash: returns `(y_offset, land_squash)`.
#[inline]
fn mb_a_ent_drop(t: f32, height: f32) -> (f32, f32) {
    if t < 0.46 {
        let u = t / 0.46;
        ((1.0 - u * u) * -height, 0.0)
    } else {
        let u = (t - 0.46) / 0.54;
        let bounce = -((u * PI).sin()) * 2.4 * (1.0 - u);
        let sq = if u < 0.28 { 1.0 - u / 0.28 } else { 0.0 };
        (bounce, sq)
    }
}

/// Outlined, top-left lit annular ring (handwheels, tally rims, hubs).
#[allow(clippy::too_many_arguments)]
fn mb_a_ring(
    pb: &mut PixelBuffer,
    cx: f32,
    cy: f32,
    r_in: f32,
    r_out: f32,
    p: Pal,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let outer = r_out + 0.9;
    let inner = (r_in - 0.85).max(0.0);
    let o2 = outer * outer;
    let i2 = inner * inner;
    let ro_rim = (r_out - 0.25).max(0.0);
    let ri_rim = r_in + 0.25;
    let ro2 = ro_rim * ro_rim;
    let ri2 = ri_rim * ri_rim;
    let inv_r = 1.0 / r_out.max(0.5);
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
            if d2 > o2 || d2 < i2 {
                continue;
            }
            let c = if d2 > ro2 || d2 < ri2 {
                ink_c
            } else {
                let lit = (0.52 + 0.54 * ((dx * LIGHT.x + dy * LIGHT.y) * inv_r)).clamp(0.0, 1.0);
                let base = if lit > 0.58 {
                    mix(p.mid, p.hi, (lit - 0.58) / 0.42)
                } else {
                    mix(p.lo, p.mid, lit / 0.58)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Crisp glowing `+` glint star for tell charge-ups, blade tips, and impacts.
fn mb_a_spark_cross(pb: &mut PixelBuffer, pos: Vec2, arm: i32, core: Rgb, glow: Rgb, alpha: f32) {
    if alpha <= 0.03 {
        return;
    }
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    blend_px(pb, cx, cy, core, alpha.clamp(0.0, 1.0));
    for d in 1..=arm {
        let a = alpha * (1.0 - d as f32 / (arm as f32 + 1.0));
        let col = if d == 1 { core } else { glow };
        blend_px(pb, cx - d, cy, col, a);
        blend_px(pb, cx + d, cy, col, a);
        blend_px(pb, cx, cy - d, col, a);
        blend_px(pb, cx, cy + d, col, a);
    }
}

/// Soft drifting steam / vapor / overload-smoke puff.
fn mb_a_puff(pb: &mut PixelBuffer, pos: Vec2, r: i32, col: Rgb, alpha: f32) {
    if alpha <= 0.03 || r <= 0 {
        return;
    }
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    let rf = r as f32 + 0.5;
    let inv = 1.0 / (rf * rf);
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = (dx * dx + dy * dy) as f32 * inv;
            if d2 < 1.0 {
                blend_px(pb, cx + dx, cy + dy, col, alpha * (1.0 - d2 * 0.7));
            }
        }
    }
}

/// Crisp 2-pixel specular rim glint that survives low ambient and 2× zoom-out.
#[inline]
fn mb_a_glint(pb: &mut PixelBuffer, pos: Vec2, col: Rgb) {
    let x = pos.x.round() as i32;
    let y = pos.y.round() as i32;
    plot_px(pb, x, y, (255, 255, 255));
    blend_px(pb, x + 1, y, col, 0.8);
    blend_px(pb, x, y + 1, col, 0.75);
}

/// Outlined 2-segment articulated limb (spider, crab, or tripod leg).
#[allow(clippy::too_many_arguments)]
fn mb_a_jointed_leg(
    pb: &mut PixelBuffer,
    hip: Vec2,
    knee: Vec2,
    foot: Vec2,
    r_hip: f32,
    r_foot: f32,
    p: Pal,
    foot_col: Rgb,
    flash: f32,
) {
    let r_mid = (r_hip + r_foot) * 0.55;
    draw_tapered_capsule(pb, hip, knee, r_hip, r_mid, p.hi, p.mid, p.lo, flash);
    draw_tapered_capsule(pb, knee, foot, r_mid, r_foot, p.mid, p.lo, INK, flash);
    plot_px(pb, foot.x.round() as i32, foot.y.round() as i32, apply_flash(foot_col, flash));
}

// ---- 7. VALVE WIDOW · "Someone left the water running." --------------------
// 26–34 px crimson cast-iron valve-wheel spider with brass hub, leaking water
// gland, 3-eye arachnid cluster, and a long chrome pipe-wrench foreleg that
// cocks back to -50° through Tell and whips 100° across on Strike.
fn draw_valve_widow(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let valve = pal((255, 168, 140), (226, 74, 68), (118, 28, 32));
    let brass = pal((255, 244, 174), (216, 162, 54), (108, 70, 20));
    let iron = pal((192, 206, 224), (116, 128, 146), (48, 56, 70));
    let steel = pal((252, 255, 255), (186, 206, 228), (78, 94, 114));
    let water_hi: Rgb = (218, 252, 255);
    let water_mid: Rgb = (104, 220, 242);
    let water_lo: Rgb = (38, 114, 144);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 3.8;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 15.5, 7.2, 0.50);
        // Spreading blue-cyan puddle from the burst valve!
        let pr = 7.5 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x, center.y + 2.5, pr, pr * 0.52, water_hi, water_mid, water_lo, flash);
        // Split crimson valve wheel halves flung apart.
        let h1 = center - s * (4.0 + burst * 5.0) - Vec2::new(0.0, bounce * 0.8);
        let h2 = center + s * (4.0 + burst * 5.0) + Vec2::new(0.0, 1.0 - bounce * 0.5);
        draw_tapered_capsule(pb, h1 - f * 5.0, h1 + f * 4.5, 2.3, 2.0, valve.hi, valve.mid, valve.lo, flash);
        draw_tapered_capsule(pb, h2 - f * 4.5, h2 + f * 5.0, 2.2, 1.9, valve.mid, valve.lo, INK, flash);
        // Snapped brass stem hub.
        draw_shaded_disc(pb, center.x - f.x * 3.0, center.y - 1.5 - bounce, 3.2, brass.hi, brass.mid, brass.lo, flash);
        // Long chrome pipe wrench flung forward pointing back toward the chest!
        let w_base = center + f * (5.5 + burst * 6.5) + s * 4.0 - Vec2::new(0.0, bounce * 0.6);
        let w_jaw = w_base + f * 6.5 - s * 3.5;
        draw_tapered_capsule(pb, w_base, w_jaw, 1.8, 1.3, steel.hi, steel.mid, steel.lo, flash);
        draw_tapered_capsule(pb, w_jaw, w_jaw + f.rotate(0.5) * 3.6, 1.1, 0.7, steel.hi, steel.mid, steel.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, water_hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let half_sweep = 50.0_f32.to_radians(); // exact half of 100° Cord sweep
    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 22.0)
    } else {
        (0.0, 0.0)
    };

    let (recoil, lift, wheel_spin, leg_freq, tilt_x) = match pose {
        Pose::Entrance(t) => (0.0, ent_y, t * 18.0, 10.0, 0.0),
        Pose::Idle => (0.0, (anim * 2.6).sin() * 0.7, (anim * 1.4).sin() * 0.55, 2.4, (anim * 1.4).cos() * 0.4),
        Pose::Approach => ((anim * 8.0).sin() * 1.1, -(anim * 8.0).sin().abs() * 1.4, anim * 4.8, 9.5, 0.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-4.2 * c + q * 0.8, -2.2 * c, anim * 2.0 + t * 22.0, 3.0, q * 0.9)
        }
        Pose::Strike(t) => {
            let lunge = (t * PI).sin();
            (4.6 * lunge, -0.6 + 1.2 * lunge, anim * 2.0 + t * 38.0, 4.0, 0.0)
        }
        Pose::Recover(t) => {
            let wob = (t * 11.0).sin() * 0.9;
            (-2.0, 1.8, 0.0, 12.0, 2.8 + wob)
        }
        Pose::Retreat(t) => (-2.6, -(t * 12.0).sin().abs() * 1.2, -t * 12.0, 8.5, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(tilt_x, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.48, 1.15);
    draw_shadow(pb, center.x + 1.5, center.y + 7.2, 13.0 * sh_s + ent_sq * 2.0, 5.8 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), water_mid);
        }
    }

    // 1. Six articulated spider legs splayed around the valve body (3 per side).
    let leg_spread = match pose {
        Pose::Tell(t) => 1.0 + tell_coil(t) * 0.16,
        Pose::Recover(_) => 1.12,
        _ => 1.0,
    };
    for i in 0..3 {
        let idx = i as f32 - 1.0;
        let phase = anim * leg_freq + i as f32 * 2.1;
        for side in [-1.0_f32, 1.0] {
            let step = (phase + if side > 0.0 { PI } else { 0.0 }).sin();
            let stride = if matches!(pose, Pose::Approach | Pose::Retreat(_)) {
                step * 2.8
            } else if matches!(pose, Pose::Idle) {
                step * 0.7
            } else if matches!(pose, Pose::Recover(_)) {
                step * 1.8
            } else {
                0.0
            };
            let hip = base + f * (idx * 3.4) + s * (side * 6.0) + Vec2::new(0.0, 1.6);
            let knee = base + f * (idx * 5.2 + stride * 0.5) + s * (side * 10.2 * leg_spread) + Vec2::new(0.0, -2.4);
            let foot = base + f * (idx * 6.8 + stride) + s * (side * 13.0 * leg_spread) + Vec2::new(0.0, 4.6 - stride.max(0.0) * 0.6);
            mb_a_jointed_leg(pb, hip, knee, foot, 1.25, 0.6, iron, brass.hi, flash);
        }
    }

    // 2. Heavy crimson cast-iron gate-valve handwheel (5 spokes + thick outer rim).
    let wheel_c = base + Vec2::new(0.0, -1.0 + ent_sq * 1.0);
    for k in 0..5 {
        let a = wheel_spin + k as f32 * (TAU / 5.0);
        let d = Vec2::from_angle(a);
        draw_tapered_capsule(
            pb,
            wheel_c + d * 2.4,
            wheel_c + d * 8.6,
            1.45,
            1.2,
            valve.hi,
            valve.mid,
            valve.lo,
            flash,
        );
    }
    mb_a_ring(pb, wheel_c.x, wheel_c.y, 7.0, 9.6, valve, flash);
    // Crisp specular light rim on the crimson handwheel so it survives 60% ambient Sewer light.
    mb_a_glint(pb, wheel_c + Vec2::new(-5.8, -5.6), (255, 224, 206));
    mb_a_glint(pb, wheel_c + Vec2::new(4.8, -6.2), valve.hi);

    // 3. Central riveted brass valve gland & hex stem nut!
    draw_shaded_disc(pb, wheel_c.x, wheel_c.y, 4.4, brass.hi, brass.mid, brass.lo, flash);
    draw_shaded_disc(pb, wheel_c.x, wheel_c.y, 2.0, iron.hi, iron.mid, iron.lo, flash);
    mb_a_glint(pb, wheel_c + Vec2::new(-1.5, -2.0), brass.hi);

    // Leaking sewer water drops in Idle / pressurized mist in Tell / geyser in Recover!
    if matches!(pose, Pose::Idle | Pose::Approach) {
        let ph = (anim * 1.8).fract();
        let drop_p = wheel_c + Vec2::new(0.5, -2.5 - ph * (1.0 - ph) * 8.0 + ph * 6.0);
        plot_px(pb, drop_p.x.round() as i32, drop_p.y.round() as i32, water_hi);
        blend_px(pb, drop_p.x.round() as i32, drop_p.y.round() as i32 + 1, water_mid, 0.8);
    } else if let Pose::Recover(t) = pose {
        // Blown gasket: pressurized water fountain gushing straight up out of the brass nut!
        for k in 0..5 {
            let ph = (t * 4.5 + k as f32 * 0.2).fract();
            let sp_x = (k as f32 - 2.0) * (1.4 + ph * 3.0);
            let sp_y = -2.0 - ph * 13.0 + ph * ph * 5.0;
            let q = wheel_c + Vec2::new(sp_x, sp_y);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 2 == 0 { water_hi } else { water_mid });
        }
    }

    // 4. Oversized gleaming chrome PIPE-WRENCH FORELEG!
    //    Cocks back to -50° (-half_sweep) during Tell, whips -50° -> +50° during Strike!
    let (wrench_ang, wrench_len, jaw_open) = match pose {
        Pose::Idle => (-0.52 + (anim * 2.2).sin() * 0.14, 13.5, 0.28 + (anim * 3.5).sin().abs() * 0.16),
        Pose::Approach => (-0.42 + (anim * 7.0).sin() * 0.20, 14.0, 0.30),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            let a = -0.40 + (-half_sweep - 0.18 - (-0.40)) * c + q * 0.07;
            (a, 14.5 + c * 2.2, 0.36 + c * 0.36)
        }
        Pose::Strike(t) => (-half_sweep + 2.0 * half_sweep * t, 18.0, 0.20),
        Pose::Recover(_) => (half_sweep + 0.18, 15.0, 0.55),
        Pose::Retreat(_) => (-0.85, 11.5, 0.22),
        Pose::Entrance(_) => (-0.6, 12.5, 0.32),
        Pose::Death(_) => unreachable!(),
    };
    let w_dir = f.rotate(wrench_ang);
    let w_perp = w_dir.perp();
    let shoulder = wheel_c + f * 3.2 - s * 2.4;
    let jaw_base = shoulder + w_dir * wrench_len;

    // Strike motion smear arc following the exact 100° pipe-wrench whip!
    if let Pose::Strike(t) = pose {
        let base_a = f.angle();
        let cur_a = base_a + wrench_ang;
        let trail_a = (base_a - half_sweep).max(cur_a - 0.95);
        draw_smear_arc(pb, shoulder, trail_a, cur_a, 10.0, 20.5, water_mid, steel.hi, 0.78 * (1.0 - t * 0.25));
        for k in 0..4 {
            let ph = (t * 4.0 + k as f32 * 0.25).fract();
            let sp = jaw_base - w_perp * (ph * 8.0) + w_dir * ((k as f32 - 1.5) * 2.2);
            plot_px(pb, sp.x.round() as i32, sp.y.round() as i32, if k % 2 == 0 { water_hi } else { brass.hi });
        }
    }

    // Thick chrome wrench beam with red adjustment knurl collar & serrated open C-jaw!
    draw_tapered_capsule(pb, shoulder, jaw_base, 2.2, 1.6, steel.hi, steel.mid, steel.lo, flash);
    // Razor-bright chrome specular edge along the wrench beam (signature feature).
    let w_edge_a = shoulder + w_dir * 2.4 + w_perp * 0.8;
    let w_edge_b = jaw_base - w_dir * 1.0 + w_perp * 0.5;
    pb.line(
        w_edge_a.x.round() as i32,
        w_edge_a.y.round() as i32,
        w_edge_b.x.round() as i32,
        w_edge_b.y.round() as i32,
        (255, 255, 255),
    );
    // Red valve-wrench adjustment nut near the head.
    let knurl = shoulder + w_dir * (wrench_len - 3.6);
    draw_shaded_disc(pb, knurl.x, knurl.y, 1.9, valve.hi, valve.mid, valve.lo, flash);
    // Fixed upper jaw & hinged lower serrated hook jaw.
    let jaw_hi = jaw_base + w_dir.rotate(jaw_open) * 5.2;
    let jaw_lo = jaw_base + w_dir.rotate(-jaw_open) * 4.8;
    draw_tapered_capsule(pb, jaw_base, jaw_hi, 1.5, 0.95, steel.hi, steel.mid, steel.lo, flash);
    draw_tapered_capsule(pb, jaw_base, jaw_lo, 1.4, 0.9, steel.hi, steel.mid, steel.lo, flash);
    mb_a_glint(pb, jaw_hi, steel.hi);

    // Tell charge glints & pressurized hissing sparks on the cocked wrench head!
    if let Pose::Tell(t) = pose {
        let pulse = 0.55 + 0.45 * (t * 28.0).sin();
        mb_a_spark_cross(pb, jaw_base + w_dir * 2.2, 3, (255, 255, 240), water_mid, pulse * t.min(1.0));
        for k in 0..3 {
            let ph = (t * 5.0 + k as f32 * 0.33).fract();
            let sp = wheel_c + Vec2::new((k as f32 - 1.0) * 4.5 * ph, -3.0 - ph * 7.0);
            blend_px(pb, sp.x.round() as i32, sp.y.round() as i32, water_hi, 1.0 - ph);
        }
    }

    // 5. Three-eye arachnid cluster facing `f` on the front brass hub bezel!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 232, 102),
        _ => (255, 246, 206),
    };
    let face = wheel_c + f * 3.0;
    draw_eye(pb, face, 1.75, eye_col, valve.lo, f, expr, flash);
    // Two tiny glowing spider side-ocelli that survive low ambient light.
    let oc_col = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) { (255, 196, 72) } else { (255, 124, 92) };
    for side in [-1.0_f32, 1.0] {
        let oc = wheel_c + f * 2.4 + s * (side * 2.9);
        plot_px(pb, oc.x.round() as i32, oc.y.round() as i32, apply_flash(oc_col, flash));
    }

    if let Pose::Retreat(t) = pose {
        let ph = (t * 3.0).fract();
        mb_a_puff(pb, wheel_c + Vec2::new(0.0, -4.5 - ph * 6.0), 2, water_hi, (1.0 - ph) * 0.7);
    }
    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, wheel_c + Vec2::new(0.0, -9.5), 7.2, t);
        draw_recover_beacon(pb, wheel_c, t);
    }
    draw_hurt_burst(pb, base, f, flash, steel.hi);
}

// ---- 8. THE MUDLARK · "Both traps are occupied." ---------------------------
// 34–38 px broad mud-armoured sewer crab wearing two sprung wooden mousetraps
// on its carapace, eight wide splayed crab legs, elevated glowing eyestalks,
// and twin massive serrated steel trap-claws that throw wide in Tell (28+ px)
// and clap violently shut down the centre line on Strike.
fn draw_mudlark(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let mud = pal((244, 198, 136), (178, 126, 72), (86, 54, 28));
    let shell_lo = pal((198, 152, 98), (134, 92, 50), (62, 40, 22));
    let wood = pal((254, 240, 202), (202, 172, 122), (96, 74, 44));
    let steel = pal((248, 254, 255), (182, 200, 220), (74, 88, 108));
    let bait: Rgb = (255, 218, 76);
    let spark_hi: Rgb = (255, 232, 126);
    let spark_hot: Rgb = (255, 134, 68);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 17.0, 8.0, 0.52);
        // Dark mud splatter puddle.
        draw_shaded_ellipse(pb, center.x, center.y + 2.5, 10.0 + burst * 5.0, 5.5 + burst * 2.5, shell_lo.mid, shell_lo.lo, INK, 0.0);
        // Cracked mud carapace halves + flung broken wooden mousetraps!
        for side in [-1.0_f32, 1.0] {
            let off = s * (side * (5.5 + burst * 6.5)) - Vec2::new(0.0, bounce * 0.7);
            draw_rotated_box(pb, center + off, f.rotate(side * 0.35), 4.6, 3.0, wood.hi, wood.mid, wood.lo, flash);
            let cl = center + f * (5.0 + burst * 5.5) + s * (side * (4.0 + burst * 4.5));
            draw_tapered_capsule(pb, cl, cl + f * 5.5, 2.2, 0.9, steel.hi, steel.mid, steel.lo, flash);
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, spark_hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_rise, ent_mud) = if let Pose::Entrance(t) = pose {
        let ease = 1.0 - (1.0 - t).powi(3);
        ((1.0 - ease) * 14.0, (1.0 - t).clamp(0.0, 1.0))
    } else {
        (0.0, 0.0)
    };

    let (recoil, lift, sway, trap_rattle) = match pose {
        Pose::Entrance(t) => (0.0, ent_rise, (t * 18.0).sin() * 1.2, 1.4),
        Pose::Idle => (0.0, (anim * 2.4).sin() * 0.65, (anim * 1.8).sin() * 0.7, 0.0),
        Pose::Approach => ((anim * 7.5).sin() * 1.0, -(anim * 7.5).sin().abs() * 1.4, (anim * 7.5).cos() * 1.8, 0.5),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.8 * c, -1.6 * c, q * 1.2, 1.2 + q * 1.4)
        }
        Pose::Strike(t) => {
            let lunge = (t * PI).sin();
            (4.8 * lunge, 0.8 * lunge, 0.0, 1.8)
        }
        Pose::Recover(t) => (2.0, 2.0, (t * 10.0).sin() * 1.1, 0.4),
        Pose::Retreat(t) => (-2.5, -(t * 11.0).sin().abs() * 1.1, (t * 11.0).sin() * 1.6, 0.2),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + s * sway + jolt + Vec2::new(0.0, lift);
    draw_shadow(pb, base.x + 1.5, center.y + 6.5, 16.5, 6.8, 0.56);

    // Entrance mud eruption!
    if let Pose::Entrance(t) = pose {
        draw_shaded_ellipse(pb, center.x, center.y + 4.0, 13.0 * ent_mud + 4.0, 6.0 * ent_mud + 2.0, shell_lo.mid, shell_lo.lo, INK, 0.0);
        draw_entrance_ring(pb, center, t, mud.hi);
        for k in 0..6 {
            let a = k as f32 * (TAU / 6.0) + 0.3;
            let q = center + Vec2::new(a.cos() * (6.0 + t * 14.0), a.sin() * (4.0 + t * 8.0) - t * (1.0 - t) * 16.0);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 2 == 0 { mud.hi } else { wood.hi });
        }
    }

    // 1. Eight wide articulated crab legs (4 per side, ~32 px toe-to-toe!).
    for i in 0..4 {
        let u = i as f32 - 1.5;
        for side in [-1.0_f32, 1.0] {
            let ph = anim * 7.5 + i as f32 * 1.45 + if side > 0.0 { PI } else { 0.0 };
            let stride = match pose {
                Pose::Approach | Pose::Retreat(_) => ph.sin() * 2.6,
                Pose::Idle => (anim * 3.0 + i as f32 * 1.7 + side).sin() * 0.75,
                Pose::Recover(_) => (anim * 14.0 + i as f32 * 2.0).sin() * 1.6,
                _ => 0.0,
            };
            let hip = base + f * (u * 2.2 - 0.8) + s * (side * 6.0) + Vec2::new(0.0, 1.5);
            let knee = base + f * (u * 3.2 - 0.5 + stride * 0.4) + s * (side * 11.4) + Vec2::new(0.0, -2.0);
            let foot = base + f * (u * 4.2 + stride) + s * (side * 15.2) + Vec2::new(0.0, 4.4 - stride.max(0.0) * 0.5);
            mb_a_jointed_leg(pb, hip, knee, foot, 1.35, 0.65, shell_lo, mud.hi, flash);
        }
    }

    // 2. Sculpted broad mud-crab carapace (stadium across `s` + wet ridge highlights).
    draw_tapered_capsule(pb, base - s * 6.2, base + s * 6.2, 6.2, 6.2, mud.hi, mud.mid, mud.lo, flash);
    // Darker muddy central dorsal ridge seam + wet light rim along upper carapace edge.
    draw_tapered_capsule(pb, base - f * 3.5, base + f * 3.5, 2.4, 2.0, shell_lo.hi, shell_lo.mid, shell_lo.lo, flash);
    mb_a_glint(pb, base - s * 4.8 + Vec2::new(-1.0, -3.6), mud.hi);
    mb_a_glint(pb, base + s * 4.4 + Vec2::new(-1.0, -3.6), mud.hi);

    // 3. TWO BROKEN WOODEN MOUSETRAPS strapped onto the carapace!
    for side in [-1.0_f32, 1.0] {
        let rattle_off = Vec2::new(0.0, (anim * 32.0 + side).sin() * trap_rattle * 0.5);
        let at = base - f * 1.2 + s * (side * 5.2) + Vec2::new(0.0, -1.6) + rattle_off;
        draw_rotated_box(pb, at, f.rotate(side * 0.14), 4.8, 3.0, wood.hi, wood.mid, wood.lo, flash);
        // Yellow cheese-bait treadle near the front of each trap board!
        let treadle = at + f * 2.2;
        draw_shaded_disc(pb, treadle.x, treadle.y, 1.25, (255, 248, 178), bait, mud.lo, flash);
        // Coiled steel torsion spring & bent sprung wire bail rising across the plank!
        draw_shaded_disc(pb, at.x, at.y, 1.2, steel.hi, steel.mid, steel.lo, flash);
        let bail_a = at - f * 3.4 - s * (side * 2.0);
        let bail_b = at + f * 2.4 + s * (side * 2.8) + Vec2::new(0.0, -1.8 - trap_rattle * 0.7);
        draw_tapered_capsule(pb, bail_a, bail_b, 0.75, 0.7, steel.hi, steel.mid, steel.lo, flash);
        mb_a_glint(pb, bail_b, steel.hi);
    }

    // 4. TWIN MASSIVE SERRATED STEEL PINCER CLAWS!
    //    Throw WIDE apart in Tell (matching 28 px opening) and SLAM SHUT to ~4 px on Strike!
    let (sep, reach, gape, idle_snap) = match pose {
        Pose::Idle => (9.0, 9.5, 0.30, (anim * 3.2).sin()),
        Pose::Approach => (9.6, 10.2, 0.34, (anim * 6.5).sin()),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (10.8 + 2.2 * c + q * 0.5, 9.0 - 1.0 * c, 0.40 + 0.32 * c, 0.0)
        }
        Pose::Strike(t) => {
            // Fast accelerating clamp from wide open (13.0) to shut (2.2)!
            let close = (1.0 - t).powf(1.35);
            (2.2 + 10.8 * close, 12.2 + (t * PI).sin() * 2.2, 0.06 + 0.50 * close, 0.0)
        }
        Pose::Recover(_) => (3.0, 11.0, 0.05, 0.0),
        Pose::Retreat(_) => (7.8, 8.2, 0.24, 0.0),
        Pose::Entrance(_) => (10.5, 9.5, 0.45, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    // Strike dual curved closing-claw motion smears + central clap impact flash!
    if let Pose::Strike(t) = pose {
        let base_a = f.angle();
        for side in [-1.0_f32, 1.0] {
            let a0 = base_a + side * 0.95;
            let a1 = base_a + side * 0.12;
            draw_smear_arc(pb, base + f * 3.0, a0.min(a1), a0.max(a1), 11.0, 17.5, spark_hot, spark_hi, 0.72 * (1.0 - t * 0.35));
        }
        if t > 0.45 {
            let imp = base + f * 15.0;
            mb_a_spark_cross(pb, imp, 4, (255, 255, 236), spark_hot, (1.0 - (t - 0.45) / 0.55).clamp(0.0, 1.0));
        }
    }

    for side in [-1.0_f32, 1.0] {
        let alt_gape = if matches!(pose, Pose::Idle | Pose::Approach) {
            let sn = if side > 0.0 { idle_snap.max(0.0) } else { (-idle_snap).max(0.0) };
            (gape - sn * 0.20).max(0.08)
        } else {
            gape
        };
        let alt_lift = if matches!(pose, Pose::Idle) {
            (if side > 0.0 { idle_snap } else { -idle_snap }) * 1.1
        } else {
            0.0
        };
        let shoulder = base + f * 3.2 + s * (side * 6.8);
        let elbow = base + f * (reach * 0.55) + s * (side * (sep * 0.85)) + Vec2::new(0.0, -1.0 - alt_lift);
        let wrist = base + f * reach + s * (side * sep) + Vec2::new(0.0, -0.8 - alt_lift);
        draw_tapered_capsule(pb, shoulder, elbow, 2.5, 2.2, mud.hi, mud.mid, mud.lo, flash);
        draw_tapered_capsule(pb, elbow, wrist, 2.3, 2.8, mud.hi, mud.mid, mud.lo, flash);
        // Heavy curved steel serrated pincer jaws!
        let outer_tip = wrist + f.rotate(side * alt_gape) * 6.2;
        let inner_tip = wrist + f.rotate(-side * alt_gape * 0.85) * 5.4;
        draw_tapered_capsule(pb, wrist, outer_tip, 2.2, 0.9, steel.hi, steel.mid, steel.lo, flash);
        draw_tapered_capsule(pb, wrist, inner_tip, 1.7, 0.8, steel.hi, steel.mid, steel.lo, flash);
        mb_a_glint(pb, outer_tip, steel.hi);
        // Tell tension sparks between the gaping serrated jaws!
        if let Pose::Tell(t) = pose {
            if t > 0.45 {
                let mid_jaw = (outer_tip + inner_tip) * 0.5;
                mb_a_spark_cross(pb, mid_jaw, 2, (255, 252, 210), spark_hot, ((t - 0.45) / 0.55).clamp(0.0, 1.0));
            }
        }
    }

    // 5. Twin expressive elevated eyestalks + frothing crab mandibles!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 92),
        _ => (255, 250, 210),
    };
    let stalk_bob = if matches!(pose, Pose::Idle) { (anim * 2.8).sin() * 0.7 } else { 0.0 };
    for side in [-1.0_f32, 1.0] {
        let st_root = base + f * 4.5 + s * (side * 2.4);
        let st_tip = base + f * 6.4 + s * (side * 2.8) + Vec2::new(0.0, -2.6 + stalk_bob * side);
        draw_tapered_capsule(pb, st_root, st_tip, 1.1, 0.9, mud.hi, mud.mid, mud.lo, flash);
        draw_eye(pb, st_tip, 1.55, eye_col, (50, 28, 12), f, expr, flash);
    }
    // Tiny frothy mud bubbles at mouthparts in Idle!
    if matches!(pose, Pose::Idle) {
        let bph = (anim * 1.9).fract();
        let bub = base + f * (6.2 + bph * 2.0) + Vec2::new(0.0, 1.0 - bph * 3.5);
        blend_px(pb, bub.x.round() as i32, bub.y.round() as i32, (236, 246, 232), 1.0 - bph);
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, base + Vec2::new(0.0, -10.5), 7.6, t);
        draw_recover_beacon(pb, base + Vec2::new(0.0, -1.5), t);
    }
    draw_hurt_burst(pb, base, f, flash, steel.hi);
}

// ---- 9. PASTRY WHEEL · "Makes six equal portions." -------------------------
// 26–30 px gleaming mirror-silver 12-flute steel cutting wheel in a forked
// yoke on a turned cherry-mahogany handle with brass bolster & pommel ring.
// Rears up and spins to a sparkling blur in Tell; slams down the 46×8 cut in Strike.
fn draw_pastry_wheel(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let steel = pal((254, 255, 255), (186, 206, 226), (74, 90, 110));
    let cherry = pal((252, 182, 124), (204, 106, 62), (96, 44, 22));
    let brass = pal((255, 242, 168), (218, 166, 58), (108, 72, 22));
    let dough = pal((255, 252, 232), (234, 214, 164), (148, 126, 82));
    let flour: Rgb = (255, 252, 244);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.2;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 14.0, 6.5, 0.48);
        // Scattered flour dusting & dough curls.
        draw_shaded_ellipse(pb, center.x, center.y + 2.5, 9.0 + burst * 4.0, 4.5 + burst * 2.0, flour, dough.mid, dough.lo, 0.0);
        // Snapped cherry handle flung back.
        let h_c = center - f * (5.0 + burst * 6.0) + s * 3.0 - Vec2::new(0.0, bounce * 0.7);
        draw_tapered_capsule(pb, h_c - f * 5.0, h_c + f * 4.0, 2.3, 1.5, cherry.hi, cherry.mid, cherry.lo, flash);
        // Detached fluted steel wheel rolling away forward!
        let w_c = center + f * (5.5 + burst * 6.5) - s * 2.5 - Vec2::new(0.0, bounce);
        draw_shaded_disc(pb, w_c.x, w_c.y, 5.4, steel.hi, steel.mid, steel.lo, flash);
        draw_shaded_disc(pb, w_c.x, w_c.y, 1.8, brass.hi, brass.mid, brass.lo, flash);
        if t < 0.40 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.40, steel.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 22.0)
    } else {
        (0.0, 0.0)
    };

    let (recoil, lift, wheel_spin, pitch_y) = match pose {
        Pose::Entrance(t) => (0.0, ent_y, t * 28.0, 0.0),
        Pose::Idle => ((anim * 2.6).sin() * 1.1, (anim * 2.6).cos() * 0.6, (anim * 2.6).sin() * 0.9, 0.0),
        Pose::Approach => ((anim * 9.0).sin() * 1.3, -(anim * 9.0).sin().abs() * 1.3, anim * 11.5, 0.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-4.8 * c + q * 0.7, -3.0 * c, anim * 3.0 + t * 46.0, -3.2 * c)
        }
        Pose::Strike(t) => {
            let lunge = (t * PI).sin();
            (6.4 * lunge, 0.8 * lunge, anim * 3.0 + t * 72.0, 1.5 * lunge)
        }
        Pose::Recover(t) => (2.6, 1.8, 0.0, 4.6 + (t * 10.0).sin() * 0.9),
        Pose::Retreat(t) => (-3.0, -(t * 10.0).sin().abs() * 1.0, -t * 18.0, -1.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.46, 1.15);
    draw_shadow(pb, base.x + 1.2, center.y + 6.5, 13.0 * sh_s + ent_sq * 2.0, 5.4 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), flour);
        }
    }

    // Scored pastry cut seam under the jammed wheel during Recover!
    if matches!(pose, Pose::Recover(_)) {
        let seam = base + f * 7.5 + Vec2::new(0.0, 4.5);
        draw_tapered_capsule(pb, seam - f * 5.0, seam + f * 6.0, 1.6, 1.2, dough.lo, (64, 48, 32), INK, 0.0);
    }

    let handle_tail = base - f * 10.8 + Vec2::new(0.0, -pitch_y * 1.05);
    let bolster = base - f * 2.4 + Vec2::new(0.0, -pitch_y * 0.25);
    let wheel_c = base + f * 5.8 + Vec2::new(0.0, pitch_y * 0.50);

    // 1. Hanging brass pommel loop + turned cherry-wood grip + brass bolster collar!
    let pommel = handle_tail - f * 1.4;
    mb_a_ring(pb, pommel.x, pommel.y, 1.1, 2.2, brass, flash);
    draw_tapered_capsule(pb, handle_tail, bolster, 2.6, 1.8, cherry.hi, cherry.mid, cherry.lo, flash);
    draw_shaded_disc(pb, (handle_tail + f * 2.2).x, (handle_tail + f * 2.2).y, 2.9, cherry.hi, cherry.mid, cherry.lo, flash);
    mb_a_glint(pb, handle_tail + f * 1.8 + Vec2::new(-1.0, -1.5), cherry.hi);
    // Gleaming brass ferrule collar right where the steel fork begins.
    draw_shaded_disc(pb, bolster.x, bolster.y, 2.5, brass.hi, brass.mid, brass.lo, flash);

    // 2. Two-tine steel yoke fork embracing the fluted wheel!
    for side in [-1.0_f32, 1.0] {
        let tine_mid = bolster + f * 4.0 + s * (side * 3.1);
        let tine_end = wheel_c + s * (side * 2.6);
        draw_tapered_capsule(pb, bolster + s * (side * 1.1), tine_mid, 1.1, 0.95, steel.hi, steel.mid, steel.lo, flash);
        draw_tapered_capsule(pb, tine_mid, tine_end, 0.95, 0.85, steel.hi, steel.mid, steel.lo, flash);
    }

    // 3. Large 12-flute scalloped mirror-steel cutting coin!
    let wheel_r = 5.6;
    for i in 0..12 {
        let a = wheel_spin + i as f32 * (TAU / 12.0);
        let d = Vec2::from_angle(a);
        let flute = wheel_c + d * wheel_r;
        let gleam = (a * 2.0 - wheel_spin).sin() * 0.5 + 0.5;
        let tooth_hi = mix(steel.mid, steel.hi, 0.45 + 0.55 * gleam);
        draw_shaded_disc(pb, flute.x, flute.y, 1.45, tooth_hi, steel.mid, steel.lo, flash);
    }
    draw_shaded_disc(pb, wheel_c.x, wheel_c.y, 5.1, steel.hi, steel.mid, steel.lo, flash);
    // Two high-contrast radial machined slots so rotation reads unmistakably at 2× zoom-out!
    for k in 0..2 {
        let d = Vec2::from_angle(wheel_spin + k as f32 * PI);
        draw_tapered_capsule(pb, wheel_c + d * 1.5, wheel_c + d * 4.3, 0.8, 0.6, cherry.lo, INK, INK, flash);
    }
    // Bright brass center axle rivet.
    draw_shaded_disc(pb, wheel_c.x, wheel_c.y, 2.0, brass.hi, brass.mid, brass.lo, flash);
    mb_a_glint(pb, wheel_c + Vec2::new(-2.4, -2.6), steel.hi);

    // Approach: kicking tiny pale flour puffs behind the rolling cutter!
    if matches!(pose, Pose::Approach) {
        for k in 0..3 {
            let ph = (anim * 2.0 + k as f32 * 0.33).fract();
            let q = wheel_c - f * (4.0 + ph * 9.0) + s * ((k as f32 - 1.0) * 3.2) + Vec2::new(0.0, 3.0 - ph * 4.0);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, flour, (1.0 - ph) * 0.85);
        }
    }

    // Tell: curling golden pastry ribbon + revving friction sparks off the fluted teeth!
    if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        let curl_p = wheel_c + f * 5.6 + s * 3.4;
        draw_shaded_disc(pb, curl_p.x, curl_p.y, 1.4 + c * 1.8, dough.hi, dough.mid, dough.lo, flash);
        for k in 0..4 {
            let ph = (t * 5.5 + k as f32 * 0.25).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let sp = wheel_c + f * (4.5 - ph * 9.0) + s * (side * (3.0 + ph * 6.5)) + Vec2::new(0.0, -ph * 5.0);
            plot_px(pb, sp.x.round() as i32, sp.y.round() as i32, if k % 2 == 0 { brass.hi } else { steel.hi });
        }
        mb_a_spark_cross(pb, wheel_c + f * 6.2, 3, (255, 255, 236), brass.hi, c.min(1.0));
    }

    // Strike: screaming silver-gold circular speed smear + cutting-edge impact flare + dough spray!
    if let Pose::Strike(t) = pose {
        let a0 = f.angle() - 1.2 + t * 5.0;
        draw_smear_arc(pb, wheel_c, a0, a0 + 2.4, 6.0, 10.2, brass.hi, steel.hi, 0.82 * (1.0 - t * 0.3));
        mb_a_spark_cross(pb, wheel_c + f * 6.8, 4, (255, 255, 240), brass.hi, 1.0 - t * 0.35);
        for k in 0..5 {
            let ph = (t * 3.5 + k as f32 * 0.2).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let q = wheel_c + f * (4.0 - ph * 13.0) + s * (side * (2.5 + ph * 7.5)) + Vec2::new(0.0, -ph * 4.5);
            plot_px(pb, q.x.round() as i32, q.y.round() as i32, if k % 2 == 0 { dough.hi } else { flour });
        }
    }

    // 4. Twin fiery baker's-oven amber eyes peeking from the brass bolster!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 96),
        _ => (255, 252, 238),
    };
    for side in [-1.0_f32, 1.0] {
        let ep = bolster - f * 1.4 + s * (side * 2.1) + Vec2::new(0.0, -0.8);
        draw_eye(pb, ep, 1.4, eye_col, cherry.lo, f, expr, flash);
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, handle_tail + Vec2::new(0.0, -6.5), 6.8, t);
        draw_recover_beacon(pb, bolster, t);
    }
    draw_hurt_burst(pb, base, f, flash, steel.hi);
}

// ---- 10. SALT HEN · "Season to taste. Hers." -------------------------------
// 26–30 px glazed Delftware porcelain china hen with cobalt floral wing,
// scarlet comb/wattle, golden beak/feet, and a tall perforated 3-hole salt
// shaker fan-tail that tips forward in Tell and pours 3 rhythmic salt bursts
// synced to the 0 / 0.3 / 0.6 stamp beats in Strike.
fn draw_salt_hen(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let china = pal((255, 254, 250), (228, 224, 218), (132, 134, 146));
    let delft = pal((122, 180, 252), (48, 92, 182), (22, 44, 98));
    let comb = pal((255, 118, 108), (208, 38, 46), (108, 18, 24));
    let beak = pal((255, 226, 122), (220, 162, 52), (112, 74, 20));
    let salt: Rgb = (255, 255, 255);
    let hole_c: Rgb = (18, 28, 52);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 14.5, 6.8, 0.50);
        // Expanding white mound of spilled table salt!
        let sr = 7.5 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x, center.y + 2.8, sr, sr * 0.50, salt, (226, 232, 236), (156, 166, 174), 0.0);
        // Shattered Delft porcelain shards & red comb fragment!
        for k in 0..4 {
            let a = k as f32 * FRAC_PI_2 + 0.5;
            let sh = center + Vec2::from_angle(a) * (4.5 + burst * 6.5) - Vec2::new(0.0, bounce * 0.8);
            let p_col = if k % 2 == 0 { china } else { delft };
            draw_rotated_box(pb, sh, Vec2::from_angle(a + 0.4), 3.4, 2.0, p_col.hi, p_col.mid, p_col.lo, flash);
        }
        let comb_p = center + f * (5.5 + burst * 4.0) - Vec2::new(0.0, 2.5 + bounce);
        draw_shaded_disc(pb, comb_p.x, comb_p.y, 2.1, comb.hi, comb.mid, comb.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, salt);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 20.0)
    } else {
        (0.0, 0.0)
    };

    // Synced 3-beat rhythmic pour shake matching the 0 / 0.3 / 0.6 s stamp cadence!
    let (recoil, lift, waddle, tail_tip, wing_flare, pour_pulse) = match pose {
        Pose::Entrance(_) => (0.0, ent_y, 0.0, 0.0, 0.5, 0.0),
        Pose::Idle => (0.0, (anim * 2.6).sin() * 0.5, (anim * 2.0).sin() * 0.4, (anim * 2.2).sin() * 0.6, 0.0, 0.0),
        Pose::Approach => {
            let w = (anim * 8.2).sin();
            (w * 0.9, -w.abs() * 1.6, w * 1.8, w * 0.8, w.abs() * 0.4, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-2.6 * c, -2.0 * c, q * 1.1, 6.2 * c + q * 0.8, 2.6 * c, c * 0.55)
        }
        Pose::Strike(t) => {
            let beat = (t * 3.0).fract();
            let shake = if beat < 0.34 { 1.0 - beat / 0.34 } else { 0.0 };
            (3.6 * shake, -2.2 + 3.8 * shake, 0.0, 7.6 + 3.2 * shake, 3.2, shake)
        }
        Pose::Recover(t) => (2.4, 2.0, (t * 10.0).sin() * 1.1, -1.2, -1.2, 0.0),
        Pose::Retreat(t) => (-2.2, -(t * 10.0).sin().abs() * 1.2, (t * 10.0).sin() * 1.4, -0.8, 0.3, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(waddle, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.48, 1.15);
    draw_shadow(pb, center.x + 1.2, center.y + 7.4, 12.0 * sh_s + ent_sq * 2.0, 5.2 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), salt);
        }
    }

    // 1. Two tucked golden ceramic hen feet striding underneath!
    let foot_step = match pose {
        Pose::Approach => (anim * 8.2).sin() * 2.8,
        Pose::Retreat(t) => (t * 10.0).sin() * 2.2,
        _ => 0.0,
    };
    for side in [-1.0_f32, 1.0] {
        let lift_f = if side > 0.0 { foot_step.max(0.0) } else { (-foot_step).max(0.0) };
        let hip = base + s * (side * 2.6) + Vec2::new(0.0, 3.8);
        let toe = base + f * (1.2 + side * foot_step * 0.4) + s * (side * 3.4) + Vec2::new(0.0, 7.4 - lift_f);
        draw_tapered_capsule(pb, hip, toe, 1.1, 0.7, beak.hi, beak.mid, beak.lo, flash);
    }

    // 2. ICONIC TALL PERFORATED FAN TAIL (THE 3-HOLE SALT SHAKER CAP)!
    let tail_root = base - f * 5.2 + Vec2::new(0.0, -2.0);
    let tail_top = base - f * (7.2 - tail_tip) + Vec2::new(0.0, -12.2 + tail_tip * 0.34);
    draw_tapered_capsule(pb, tail_root, tail_top, 3.6, 4.8, china.hi, china.mid, china.lo, flash);
    // Delft-cobalt decorative glazed band around the shaker crown!
    let band_a = tail_top - s * 4.4 + Vec2::new(0.0, 2.0);
    let band_b = tail_top + s * 4.4 + Vec2::new(0.0, 2.0);
    draw_tapered_capsule(pb, band_a, band_b, 1.1, 1.1, delft.hi, delft.mid, delft.lo, flash);
    // THREE UNMISTAKABLE DARK SHAKER HOLES across the high porcelain tail rim!
    let mut holes = [Vec2::ZERO; 3];
    for (k, slot) in holes.iter_mut().enumerate() {
        let h_pos = tail_top + s * ((k as f32 - 1.0) * 2.7) + Vec2::new(0.0, -0.6);
        *slot = h_pos;
        draw_shaded_disc(pb, h_pos.x, h_pos.y, 1.25, delft.lo, hole_c, INK, flash);
        // Tiny white salt grain peeking at the lip when pouring/telling.
        if pour_pulse > 0.15 {
            plot_px(pb, h_pos.x.round() as i32, h_pos.y.round() as i32, salt);
        }
    }

    // 3. Plump glazed white porcelain hen body + Delft cobalt floral scalloped wing!
    draw_shaded_ellipse(pb, base.x, base.y + 0.4, 8.2 + ent_sq * 1.2, 6.6 - ent_sq * 0.6, china.hi, china.mid, china.lo, flash);
    // Crisp specular porcelain glaze glint on the chest!
    mb_a_glint(pb, base + Vec2::new(-3.2, -2.6), china.hi);
    // Painted Delft blue wing (flares outward on Tell/Strike!).
    let w_a = base - f * 2.6 + Vec2::new(0.0, 0.2 - wing_flare * 0.6);
    let w_b = base + f * 3.0 + Vec2::new(0.0, 2.2 - wing_flare * 0.9);
    draw_tapered_capsule(pb, w_a, w_b, 2.3, 1.4, delft.hi, delft.mid, delft.lo, flash);
    // Decorative cobalt tail-breast dots.
    plot_px(pb, (base.x - f.x * 1.0).round() as i32, (base.y + 3.4).round() as i32, apply_flash(delft.hi, flash));
    plot_px(pb, (base.x + f.x * 1.8).round() as i32, (base.y + 3.2).round() as i32, apply_flash(delft.hi, flash));

    // 4. Proud arched china neck, scarlet comb, crimson wattle & sharp golden beak!
    //    Sharp hen peck-bob in Idle!
    let peck = match pose {
        Pose::Idle => {
            let cyc = (anim * 1.3).fract();
            if cyc > 0.78 { ((cyc - 0.78) / 0.22 * PI).sin() * 2.2 } else { 0.0 }
        }
        Pose::Approach => (anim * 8.2).cos() * 1.2,
        Pose::Recover(_) => 3.0,
        _ => 0.0,
    };
    let head = base + f * (6.8 + peck) + Vec2::new(0.0, -4.6 + peck * 0.7);
    draw_tapered_capsule(pb, base + f * 3.8 + Vec2::new(0.0, -1.4), head, 3.0, 2.6, china.hi, china.mid, china.lo, flash);
    draw_shaded_disc(pb, head.x, head.y, 3.2, china.hi, china.mid, china.lo, flash);
    // Three-lobed scarlet porcelain comb on crown!
    for k in 0..3 {
        let u = k as f32 - 1.0;
        let cp = head + f * (u * 1.5) + Vec2::new(0.0, -3.2 - (1.0 - u.abs()) * 0.9);
        draw_shaded_disc(pb, cp.x, cp.y, 1.35, comb.hi, comb.mid, comb.lo, flash);
    }
    // Crimson chin wattle & sharp golden beak!
    let wattle = head + f * 2.2 + Vec2::new(0.0, 2.2);
    draw_shaded_disc(pb, wattle.x, wattle.y, 1.2, comb.hi, comb.mid, comb.lo, flash);
    let beak_tip = head + f * 5.6 + Vec2::new(0.0, 0.5);
    draw_tapered_capsule(pb, head + f * 2.2, beak_tip, 1.45, 0.45, beak.hi, beak.mid, beak.lo, flash);

    // 5. Sparkling white salt stream sifting in Tell & erupting in 3 cascades on Strike!
    if let Pose::Tell(t) = pose {
        if t > 0.22 {
            for (k, &hp) in holes.iter().enumerate() {
                let fall = ((anim * 3.6 + k as f32 * 0.33).fract()) * 8.5;
                let g = hp + f * (2.0 + t * 3.5) + Vec2::new(0.0, fall);
                plot_px(pb, g.x.round() as i32, g.y.round() as i32, salt);
                blend_px(pb, g.x.round() as i32, g.y.round() as i32 - 1, (226, 238, 250), 0.7);
            }
        }
    } else if let Pose::Strike(t) = pose {
        for (k, &hp) in holes.iter().enumerate() {
            for j in 0..3 {
                let ph = (t * 6.0 + k as f32 * 0.22 + j as f32 * 0.31).fract();
                let sp = hp + f * (3.0 + ph * 11.0) + Vec2::new(0.0, ph * 11.0 - (1.0 - ph) * 2.0);
                plot_px(pb, sp.x.round() as i32, sp.y.round() as i32, salt);
            }
        }
        if pour_pulse > 0.45 {
            mb_a_spark_cross(pb, tail_top + f * 3.2, 3, salt, delft.hi, pour_pulse);
        }
    }

    // Expressive glossy hen eye!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 224, 98),
        _ => (255, 255, 255),
    };
    draw_eye(pb, head + f * 1.1 + Vec2::new(0.0, -0.4), 1.45, eye_col, delft.lo, f, expr, flash);

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, base + Vec2::new(0.0, -13.0), 7.0, t);
        draw_recover_beacon(pb, base + Vec2::new(0.0, -2.5), t);
    }
    draw_hurt_burst(pb, base, f, flash, salt);
}

// ---- 11. POT BRUSH · "Scrub until the stain runs away." --------------------
// 34–38 px jointed beechwood handle with riveted brass knuckle, glaring wooden
// brush-block, 15-tuft golden-straw bristle fan, and iridescent mint-aqua soap
// suds. Coils to -55° in Tell with bristles splayed wide; whips 110° in Strike.
fn draw_pot_brush(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let wood = pal((252, 218, 162), (198, 138, 76), (96, 58, 28));
    let brass = pal((255, 232, 132), (208, 148, 50), (104, 64, 20));
    let bristle = pal((255, 242, 170), (216, 174, 78), (112, 82, 28));
    let soap = pal((238, 255, 252), (124, 230, 226), (48, 134, 144));

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.5, center.y + 5.5, 16.0, 7.2, 0.50);
        // Spreading iridescent aqua dish-soap puddle!
        let pr = 8.0 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x, center.y + 2.5, pr, pr * 0.52, soap.hi, soap.mid, soap.lo, 0.0);
        // Snapped beechwood handle halves & scattered bristle tufts!
        let h1 = center - f * (5.5 + burst * 5.5) + s * 3.5 - Vec2::new(0.0, bounce * 0.8);
        let h2 = center + f * (5.0 + burst * 5.5) - s * 3.0 - Vec2::new(0.0, bounce * 0.6);
        draw_tapered_capsule(pb, h1 - f * 5.5, h1 + f * 3.0, 1.8, 1.5, wood.hi, wood.mid, wood.lo, flash);
        draw_tapered_capsule(pb, h2 - f * 2.5, h2 + f * 5.0, 2.0, 1.2, bristle.hi, bristle.mid, bristle.lo, flash);
        draw_shaded_disc(pb, center.x - s.x * 4.0, center.y - 2.0 - bounce, 2.1, brass.hi, brass.mid, brass.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, soap.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let half_sweep = 55.0_f32.to_radians(); // exact half of 110° Cord sweep
    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 22.0)
    } else {
        (0.0, 0.0)
    };

    let (arm_ang, arm_reach, fan_spread, body_lift, knuckle_flex) = match pose {
        Pose::Entrance(_) => (-0.25, 9.2, 0.62, ent_y, 0.0),
        Pose::Idle => ((anim * 2.2).sin() * 0.24, 9.6, 0.60 + (anim * 3.2).sin() * 0.06, (anim * 2.2).cos() * 0.7, (anim * 2.2).sin() * 1.2),
        Pose::Approach => ((anim * 6.8).sin() * 0.52, 10.4, 0.68, -(anim * 6.8).sin().abs() * 1.4, (anim * 6.8).cos() * 2.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-half_sweep * c - 0.16 * c + q * 0.08, 9.6 + 2.4 * c, 0.65 + 0.45 * c, -2.2 * c, -2.8 * c)
        }
        Pose::Strike(t) => (-half_sweep + 2.0 * half_sweep * t, 12.0 + (t * PI).sin() * 1.4, 0.88, -0.6, 2.0),
        Pose::Recover(t) => (half_sweep + 0.16, 10.6, 0.42, 2.0 + (t * 9.0).sin() * 0.5, 2.6),
        Pose::Retreat(t) => (-0.55 + (t * 8.0).sin() * 0.2, 8.4, 0.50, -(t * 10.0).sin().abs() * 1.0, -1.4),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + jolt + Vec2::new(0.0, body_lift);
    let sh_s = (1.0 - (-body_lift).max(0.0) * 0.024).clamp(0.46, 1.15);
    draw_shadow(pb, base.x + 1.4, center.y + 6.8, 13.5 * sh_s + ent_sq * 2.0, 5.4 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), soap.mid);
        }
    }

    let arm_dir = f.rotate(arm_ang);
    let arm_perp = arm_dir.perp();
    // Long jointed handle: tail grip -> riveted brass elbow knuckle -> brush head (~34 px total)!
    let tail_end = base - f * 12.2 + s * (-2.0 + knuckle_flex * 0.35);
    let knuckle = base - f * (4.4 - knuckle_flex * 0.45) + s * 3.0;
    let head = base + arm_dir * arm_reach;

    // Strike frothy aqua & straw scrub smear arc across the full 110° sweep!
    if let Pose::Strike(t) = pose {
        let base_a = f.angle();
        let cur_a = base_a + arm_ang;
        let trail_a = (base_a - half_sweep).max(cur_a - 1.15);
        draw_smear_arc(pb, base, trail_a, cur_a, 10.0, 18.5, soap.mid, soap.hi, 0.80 * (1.0 - t * 0.25));
        draw_smear_arc(pb, base, trail_a, cur_a, 11.5, 16.5, bristle.mid, bristle.hi, 0.55 * (1.0 - t * 0.3));
    }

    // 1. Two-segment articulated beechwood lever-handle + brass hanging ring + elbow rivet!
    mb_a_ring(pb, (tail_end - f * 1.2).x, (tail_end - f * 1.2).y, 1.0, 2.0, brass, flash);
    draw_tapered_capsule(pb, tail_end, knuckle, 1.6, 2.0, wood.hi, wood.mid, wood.lo, flash);
    draw_tapered_capsule(pb, knuckle, head, 1.9, 2.3, wood.hi, wood.mid, wood.lo, flash);
    draw_shaded_disc(pb, knuckle.x, knuckle.y, 2.8, brass.hi, brass.mid, brass.lo, flash);
    draw_shaded_disc(pb, knuckle.x, knuckle.y, 1.1, wood.lo, INK, INK, flash);
    mb_a_glint(pb, knuckle + Vec2::new(-1.0, -1.2), brass.hi);

    // 2. Massive 15-tuft golden-straw bristle fan fanning out from the brush head!
    let head_tip = head + arm_dir * 1.8;
    for i in 0..15 {
        let u = i as f32 / 14.0 - 0.5;
        let rustle = if matches!(pose, Pose::Idle | Pose::Approach) {
            (anim * 6.0 + i as f32 * 0.7).sin() * 0.05
        } else {
            0.0
        };
        let b_dir = arm_dir.rotate(u * fan_spread * 2.0 + rustle);
        let root = head_tip + arm_perp * (u * 4.8);
        let len = 5.0 + (1.0 - u.abs() * 1.35) * 2.4;
        let tip = root + b_dir * len;
        let b_hi = if i % 3 == 0 { soap.hi } else { bristle.hi };
        let b_mid = if i % 3 == 0 { mix(bristle.mid, soap.mid, 0.45) } else { bristle.mid };
        draw_tapered_capsule(pb, root, tip, 1.2, 0.7, b_hi, b_mid, bristle.lo, flash);
    }

    // 3. Wooden oval brush-block head + copper-brass ferrule band!
    draw_tapered_capsule(pb, head - arm_perp * 3.4, head + arm_perp * 3.4, 3.0, 3.0, wood.hi, wood.mid, wood.lo, flash);
    draw_tapered_capsule(
        pb,
        head + arm_dir * 2.1 - arm_perp * 3.8,
        head + arm_dir * 2.1 + arm_perp * 3.8,
        1.25,
        1.25,
        brass.hi,
        brass.mid,
        brass.lo,
        flash,
    );
    mb_a_glint(pb, head - arm_dir * 1.0 - Vec2::new(1.5, 1.5), wood.hi);

    // 4. Floating iridescent mint-aqua soap bubbles (Idle/Approach/Tell/Strike)!
    let bub_count = if matches!(pose, Pose::Strike(_)) { 5 } else { 3 };
    for k in 0..bub_count {
        let ph = (anim * 1.5 + k as f32 * 0.31).fract();
        let spread_k = (k as f32 - 1.5) * 3.4;
        let bp = head + arm_dir * (7.5 + ph * 4.5) + arm_perp * spread_k + Vec2::new(0.0, -ph * 9.0);
        let br = if k % 2 == 0 { 1.45 } else { 1.1 };
        draw_shaded_disc(pb, bp.x, bp.y, br, soap.hi, soap.mid, soap.lo, flash);
        plot_px(pb, bp.x.round() as i32 - 1, bp.y.round() as i32 - 1, (255, 255, 255));
    }

    if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        mb_a_spark_cross(pb, head + arm_dir * 9.5, 3, (255, 255, 250), soap.mid, c.min(1.0));
    }

    // 5. Twin furious eyes set into the wooden brush block!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 232, 108),
        _ => (255, 252, 238),
    };
    for side in [-1.0_f32, 1.0] {
        let ep = head - arm_dir * 0.6 + arm_perp * (side * 2.2);
        draw_eye(pb, ep, 1.4, eye_col, wood.lo, arm_dir, expr, flash);
    }

    if let Pose::Recover(t) = pose {
        draw_shaded_ellipse(pb, (head + arm_dir * 6.0).x, (head + arm_dir * 6.0).y + 2.5, 6.2, 2.8, soap.hi, soap.mid, soap.lo, 0.0);
        draw_dizzy_stars(pb, knuckle + Vec2::new(0.0, -9.5), 7.0, t);
        draw_recover_beacon(pb, knuckle, t);
    }
    draw_hurt_burst(pb, base, f, flash, soap.hi);
}

// ---- 12. DOSE PUMP · "Read the label twice." ------------------------------
// 22–26 px borosilicate glass syringe cylinder on a springy rubber-and-steel
// plunger foot, filled with glowing radioactive neon-chartreuse serum and a
// scarlet warning collar. Pogo-hops on Approach, levels along aim & draws the
// plunger way back in Tell, then slams home shooting a glowing jet in Strike.
fn draw_dose_pump(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let glass = pal((248, 255, 255), (174, 224, 238), (68, 108, 128));
    let serum = pal((234, 255, 118), (132, 228, 52), (46, 112, 22));
    let steel = pal((246, 252, 255), (158, 176, 196), (62, 74, 92));
    let label = pal((255, 122, 112), (206, 44, 48), (102, 18, 24));

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 13.5, 6.2, 0.50);
        // Spreading radioactive chartreuse serum puddle!
        let pr = 7.5 + burst * 6.5;
        draw_shaded_ellipse(pb, center.x, center.y + 2.6, pr, pr * 0.50, serum.hi, serum.mid, serum.lo, 0.0);
        // Shattered glass barrel & bent steel needle!
        let g1 = center - f * (4.5 + burst * 5.0) - Vec2::new(0.0, bounce * 0.8);
        let g2 = center + f * (5.0 + burst * 5.5) - Vec2::new(0.0, bounce * 0.6);
        draw_rotated_box(pb, g1, f.rotate(-0.35), 4.2, 2.8, glass.hi, glass.mid, glass.lo, flash);
        draw_tapered_capsule(pb, g2, g2 + f.rotate(0.4) * 6.5, 1.1, 0.4, steel.hi, steel.mid, steel.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, serum.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 20.0)
    } else {
        (0.0, 0.0)
    };

    // Pogo-hop bounce on Approach; springy idle pulse on the plunger foot!
    let hop = if matches!(pose, Pose::Approach) {
        let cyc = (anim * 2.2).fract();
        if cyc < 0.68 { -(cyc / 0.68 * PI).sin() * 5.2 } else { 0.8 }
    } else if matches!(pose, Pose::Idle) {
        (anim * 3.2).sin() * 1.4
    } else {
        0.0
    };

    let (level, pull, dose_len, needle_ext, recoil, lift) = match pose {
        Pose::Entrance(_) => (0.15, 9.0, 2.2, 9.2, 0.0, ent_y),
        Pose::Idle => (0.18 + (anim * 1.6).sin() * 0.08, 8.8 - hop * 0.35, 2.6, 9.4, 0.0, hop),
        Pose::Approach => (0.42, 8.6, 2.8, 9.8, (anim * 7.0).sin() * 1.0, hop),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (0.25 + 0.75 * c.min(1.0), 9.0 + 3.2 * c + q * 0.5, 2.2 + 3.4 * c.min(1.0), 10.0 + c * 1.0, -3.0 * c, -1.6 * c)
        }
        Pose::Strike(t) => {
            let drive = (t * 1.35).clamp(0.0, 1.0);
            (1.0, 12.2 - 5.2 * drive, (5.6 * (1.0 - drive)).max(0.6), 11.8 + (t * PI).sin() * 1.2, 4.2 * (t * PI).sin(), 0.0)
        }
        Pose::Recover(t) => (0.85, 7.0, 0.6, 9.8, 1.6, 1.8 + (t * 9.0).sin() * 0.5),
        Pose::Retreat(t) => (0.35, 8.8, 1.4, 9.0, -2.2, -(t * 10.0).sin().abs() * 1.8),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.15);
    draw_shadow(pb, base.x + 1.0, center.y + 8.5, 9.5 * sh_s + ent_sq * 2.0, 4.4 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), serum.mid);
        }
    }

    // Barrel stands near-vertical on its plunger foot at rest (`up` = (0, -1),
    // nose pointing toward floor-front), levelling dead onto `f` through Tell/Strike!
    let rest_dir = safe_dir(Vec2::new(f.x * 0.35, 0.94), Vec2::new(0.0, 1.0));
    let nose = safe_dir(rest_dir * (1.0 - level) + f * level, f);
    let side = nose.perp();

    // 1. Wide flanged steel plunger rod & rubber T-foot behind the barrel!
    let rod_end = base - nose * pull;
    draw_tapered_capsule(pb, base - nose * 6.5, rod_end, 1.5, 1.2, steel.hi, steel.mid, steel.lo, flash);
    // Thumb-plate / hopping rubber foot at the back end of the plunger.
    draw_rotated_box(pb, rod_end, side, 4.2, 1.35, steel.hi, steel.mid, steel.lo, flash);
    // Wide finger-flange collar at the back of the glass barrel.
    draw_rotated_box(pb, base - nose * 7.2, side, 5.2, 1.15, steel.hi, steel.mid, steel.lo, flash);

    // 2. Crisp borosilicate glass cylinder barrel + glowing chartreuse serum!
    draw_rotated_box(pb, base, nose, 7.2, 4.1, glass.hi, glass.mid, glass.lo, flash);
    // Glowing neon-lime serum chamber (visibly fills during Tell and empties on Strike!).
    let fluid_c = base + nose * (6.2 - dose_len);
    draw_rotated_box(pb, fluid_c, nose, dose_len, 3.2, serum.hi, serum.mid, serum.lo, flash);
    // Bubbling serum motes inside the barrel!
    for k in 0..3 {
        let ph = (anim * 2.6 + k as f32 * 0.37).fract();
        let bp = fluid_c + nose * ((ph - 0.5) * dose_len * 1.4) + side * (((k as f32) - 1.0) * 1.4);
        plot_px(pb, bp.x.round() as i32, bp.y.round() as i32, (250, 255, 224));
    }
    // Scarlet hazard label collar & printed graduation ticks!
    draw_rotated_box(pb, base - nose * 2.2, nose, 1.4, 4.2, label.hi, label.mid, label.lo, flash);
    for k in 0..3 {
        let tick = base + nose * (-0.2 + k as f32 * 2.3) - side * 1.4;
        draw_tapered_capsule(pb, tick - side * 1.2, tick + side * 1.0, 0.45, 0.45, glass.lo, INK, INK, flash);
    }
    // Bright glass specular reflection streak running along the top-left barrel edge!
    let gl_a = base - nose * 5.2 - side * 2.3;
    let gl_b = base + nose * 5.2 - side * 2.3;
    pb.line(gl_a.x.round() as i32, gl_a.y.round() as i32, gl_b.x.round() as i32, gl_b.y.round() as i32, (255, 255, 255));

    // 3. Tapered luer-lock steel hub & razor surgical needle!
    let hub = base + nose * 7.6;
    let tip = base + nose * needle_ext;
    draw_tapered_capsule(pb, hub, hub + nose * 1.8, 2.2, 1.2, steel.hi, steel.mid, steel.lo, flash);
    draw_tapered_capsule(pb, hub + nose * 1.5, tip, 1.0, 0.45, steel.hi, steel.mid, steel.lo, flash);

    // Glowing chartreuse bead trembling at the needle tip in Idle / Tell; blazing jet on Strike!
    if matches!(pose, Pose::Idle | Pose::Approach) {
        let pulse = 0.8 + (anim * 4.0).sin().abs() * 0.55;
        draw_shaded_disc(pb, tip.x, tip.y, pulse, serum.hi, serum.mid, serum.lo, flash);
    } else if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        draw_shaded_disc(pb, tip.x, tip.y, 1.2 + c * 1.2, (255, 255, 232), serum.hi, serum.mid, flash);
        mb_a_spark_cross(pb, tip, 3, (255, 255, 230), serum.mid, c.min(1.0));
    } else if let Pose::Strike(t) = pose {
        // High-pressure neon-lime injection spray & speed streaks!
        let jet_len = 5.5 + (1.0 - t) * 3.5;
        draw_tapered_capsule(pb, tip, tip + nose * jet_len, 1.7, 0.55, (255, 255, 236), serum.hi, serum.mid, flash);
        mb_a_spark_cross(pb, tip + nose * 2.2, 3, (255, 255, 240), serum.hi, 1.0 - t * 0.4);
        for k in 0..4 {
            let ph = (t * 5.0 + k as f32 * 0.25).fract();
            let sp = tip + nose * (ph * 9.5) + side * ((k as f32 - 1.5) * 2.0 * ph);
            plot_px(pb, sp.x.round() as i32, sp.y.round() as i32, if k % 2 == 0 { serum.hi } else { (255, 255, 240) });
        }
    }

    // 4. Expressive glowing clinical eye mounted on the scarlet label band!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => serum.hi,
        _ => (248, 255, 255),
    };
    draw_eye(pb, base - nose * 1.8, 1.6, eye_col, serum.lo, f, expr, flash);

    if let Pose::Recover(t) = pose {
        // Hissing green vapor wisps leaking from the depleted plunger seal!
        for k in 0..2 {
            let ph = (t * 3.0 + k as f32 * 0.5).fract();
            let vp = rod_end + Vec2::new(((k as f32) - 0.5) * 4.0, -2.0 - ph * 8.0);
            mb_a_puff(pb, vp, 2, serum.hi, (1.0 - ph) * 0.72);
        }
        draw_dizzy_stars(pb, base + Vec2::new(0.0, -10.0), 6.5, t);
        draw_recover_beacon(pb, base, t);
    }
    draw_hurt_burst(pb, base, f, flash, serum.hi);
}

// ---- 13. PETRI MOTHER · "The blank sample had children." -------------------
// 26–30 px shallow borosilicate culture dish gliding on rippling translucent
// cilia, filled with warm gold agar and a pulsing bioluminescent magenta mother
// colony + budding daughter spores. Carves an unmistakable 75° broken sterile
// rim notch STRICTLY aligned to `f.rotate(RING_GAP_TURN)`!
fn draw_petri_mother(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let glass = pal((246, 254, 255), (170, 214, 226), (66, 104, 122));
    let agar = pal((246, 230, 162), (190, 168, 94), (98, 84, 42));
    let colony = pal((255, 206, 242), (242, 94, 178), (122, 32, 88));
    let cilia = pal((224, 252, 255), (132, 198, 214), (56, 98, 116));
    let gap_gold: Rgb = (255, 240, 146);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 3.8;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 15.0, 7.0, 0.50);
        // Spilled warm agar & deflated magenta spore pool!
        draw_shaded_ellipse(pb, center.x, center.y + 2.5, 9.5 + burst * 5.0, 4.8 + burst * 2.2, agar.mid, agar.lo, INK, 0.0);
        draw_shaded_ellipse(pb, center.x, center.y + 1.8, 5.5 + burst * 3.5, 3.0 + burst * 1.5, colony.mid, colony.lo, INK, 0.0);
        // Shattered curved glass dish shards flung apart!
        for k in 0..3 {
            let a = k as f32 * (TAU / 3.0) + 0.4;
            let sh = center + Vec2::from_angle(a) * (5.5 + burst * 5.5) - Vec2::new(0.0, bounce * 0.7);
            draw_rotated_box(pb, sh, Vec2::from_angle(a + FRAC_PI_2), 4.0, 1.3, glass.hi, glass.mid, glass.lo, flash);
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, colony.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 20.0)
    } else {
        (0.0, 0.0)
    };

    let (lift, sway, cilia_spd, colony_swell, tilt_y) = match pose {
        Pose::Entrance(_) => (ent_y, 0.0, 8.0, 0.3, 0.0),
        Pose::Idle => ((anim * 2.5).sin() * 0.9, (anim * 1.7).sin() * 0.7, 4.8, (anim * 2.8).sin() * 0.35, 0.0),
        Pose::Approach => ((anim * 6.5).sin() * 1.0 - 0.8, (anim * 5.0).cos() * 1.2, 10.5, 0.3, 0.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.2 * c, q * 1.1, 14.0, 2.8 * c, 0.0)
        }
        Pose::Strike(t) => {
            let ring = (1.0 - t) * (t * 48.0).sin() * 1.6;
            (0.6, ring, 16.0, 3.2 * (1.0 - t * 0.6), 0.0)
        }
        Pose::Recover(t) => (2.0, (t * 8.0).sin() * 0.9, 2.2, -1.2, 1.6),
        Pose::Retreat(t) => (-0.8, (t * 9.0).sin() * 1.1, -9.0, 0.0, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + jolt + Vec2::new(sway, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.028).clamp(0.48, 1.15);
    draw_shadow(pb, base.x + 1.2, center.y + 7.2, 14.0 * sh_s + ent_sq * 2.0, 6.2 * sh_s, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), colony.mid);
        }
    }

    // 1. Nine undulating translucent cyan cilia flagella rippling under the dish skirt!
    let dish_rx = 11.6 + ent_sq * 1.4;
    let dish_ry = 6.2 - ent_sq * 0.5;
    for i in 0..9 {
        let u = i as f32 / 8.0 - 0.5;
        let x_off = u * 20.0;
        let wave = (anim * cilia_spd + i as f32 * 0.95).sin();
        let root = base + Vec2::new(x_off, dish_ry * 0.55 + (1.0 - (u * 2.0).powi(2)) * 1.8);
        let tip = root + Vec2::new(wave * 2.2 + u * 2.6, 4.4 + (wave.abs() * 0.8));
        draw_tapered_capsule(pb, root, tip, 0.95, 0.45, cilia.hi, cilia.mid, cilia.lo, flash);
    }

    // 2. Shallow borosilicate glass dish base + warm golden agar interior!
    draw_shaded_ellipse(pb, base.x, base.y + tilt_y * 0.3, dish_rx, dish_ry, glass.hi, glass.mid, glass.lo, flash);
    draw_shaded_ellipse(pb, base.x, base.y - 0.9, dish_rx - 2.4, dish_ry - 1.8, agar.hi, agar.mid, agar.lo, flash);

    // 3. Broken glass Petri rim with UNMISTAKABLE 75° sterile gap aligned to `f.rotate(RING_GAP_TURN)`!
    let gap = f.rotate(RING_GAP_TURN);
    let cos_gap = 0.793_f32; // cos(37.5°) -> exact 75° sterile break!
    let gap_glow = matches!(pose, Pose::Tell(_) | Pose::Strike(_));
    let rim_steps = 28;
    for i in 0..rim_steps {
        let a = i as f32 * (TAU / rim_steps as f32);
        let d = Vec2::from_angle(a);
        let dot = d.dot(gap);
        if dot > cos_gap {
            continue;
        }
        let near_break = dot > 0.68;
        let pt = base + Vec2::new(d.x * (dish_rx + 0.5), d.y * (dish_ry + 0.3) - 0.6);
        let rim_p = if near_break && gap_glow {
            pal((255, 255, 232), gap_gold, glass.lo)
        } else {
            glass
        };
        draw_shaded_disc(pb, pt.x, pt.y, if near_break { 1.6 } else { 1.35 }, rim_p.hi, rim_p.mid, rim_p.lo, flash);
    }
    // Subtle golden-white fracture rays radiating from the two broken rim edges during Tell/Strike!
    if gap_glow {
        let ray_len = match pose {
            Pose::Tell(t) => 3.5 + t * 4.5,
            Pose::Strike(t) => 7.5 * (1.0 - t * 0.45),
            _ => 0.0,
        };
        for sign in [-1.0_f32, 1.0] {
            let edge = gap.rotate(sign * 0.654); // ±37.5°
            let p0 = base + Vec2::new(edge.x * dish_rx, edge.y * dish_ry - 0.6);
            let p1 = p0 + edge * ray_len;
            pb.line(p0.x.round() as i32, p0.y.round() as i32, p1.x.round() as i32, p1.y.round() as i32, gap_gold);
        }
    }

    // 4. Creeping culture spore front in Tell + 285° magenta spore eruption in Strike!
    if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        let front = 3.2 + c.min(1.0) * 7.6;
        for i in 0..14 {
            let d = Vec2::from_angle(i as f32 * (TAU / 14.0) + anim * 1.4);
            if d.dot(gap) > cos_gap {
                continue;
            }
            let pt = base + Vec2::new(d.x * front, d.y * front * 0.52 - 0.9);
            draw_shaded_disc(pb, pt.x, pt.y, 1.05, colony.hi, colony.mid, colony.lo, flash);
        }
    } else if let Pose::Strike(t) = pose {
        let expand = 9.5 + t * 7.5;
        for i in 0..20 {
            let d = Vec2::from_angle(i as f32 * (TAU / 20.0));
            if d.dot(gap) > cos_gap {
                continue;
            }
            let pt = base + Vec2::new(d.x * expand, d.y * expand * 0.56 - 0.8);
            let col = if i % 2 == 0 { colony.hi } else { colony.mid };
            blend_px(pb, pt.x.round() as i32, pt.y.round() as i32, col, (1.0 - t * 0.55) * 0.92);
            blend_px(pb, pt.x.round() as i32 + 1, pt.y.round() as i32, colony.hi, (1.0 - t * 0.65) * 0.65);
        }
    }

    // 5. Pulsing central bioluminescent magenta Mother Colony + 3 orbiting daughter buds!
    let col_c = base + Vec2::new(0.0, -1.2);
    let mother_r = (3.4 + colony_swell).max(2.1);
    // Budding daughter colonies orbiting gently inside the agar.
    for k in 0..3 {
        let a = anim * 1.8 + k as f32 * (TAU / 3.0);
        let orbit = 4.6 + colony_swell * 0.45;
        let d_pos = col_c + Vec2::new(a.cos() * orbit, a.sin() * orbit * 0.5);
        draw_shaded_disc(pb, d_pos.x, d_pos.y, 1.45, colony.hi, colony.mid, colony.lo, flash);
    }
    let m_hi = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) { (255, 236, 250) } else { colony.hi };
    draw_shaded_disc(pb, col_c.x, col_c.y, mother_r, m_hi, colony.mid, colony.lo, flash);
    mb_a_glint(pb, col_c + Vec2::new(-1.4, -1.6), colony.hi);

    // 6. Twin glowing mother-colony eyes nestled in the pink nucleus!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => (255, 240, 128),
        _ => (255, 254, 248),
    };
    for side in [-1.0_f32, 1.0] {
        let ep = col_c + f * 1.4 + s * (side * 2.1);
        draw_eye(pb, ep, 1.35, eye_col, colony.lo, f, expr, flash);
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, col_c + Vec2::new(0.0, -9.5), 7.2, t);
        draw_recover_beacon(pb, col_c, t);
    }
    draw_hurt_burst(pb, base, f, flash, colony.hi);
}

// ---- 14. CAGE COUNTER · "One rat short again." -----------------------------
// 26–30 px brushed-brass clicking tally wheel on a tripedal gunmetal frame,
// crowned by a spring-loaded crimson ratchet clicker flag and carrying a 3-lamp
// reader console bar underneath that counts up and pulses in exact simulation
// order (left, right, centre).
fn draw_cage_counter(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let brass = pal((255, 240, 158), (214, 160, 52), (106, 70, 22));
    let steel = pal((226, 238, 252), (132, 148, 172), (56, 66, 84));
    let ruby = pal((255, 174, 166), (238, 52, 66), (112, 18, 28));
    let lamp_off = pal((112, 124, 144), (66, 76, 92), (30, 36, 48));
    let ivory: Rgb = (250, 248, 236);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.8, 14.5, 6.8, 0.50);
        // Splayed broken steel tripod legs.
        for k in 0..3 {
            let a = k as f32 * (TAU / 3.0) + 0.5;
            let p0 = center + Vec2::from_angle(a) * 2.5;
            let p1 = center + Vec2::from_angle(a) * (8.5 + burst * 5.0);
            draw_tapered_capsule(pb, p0, p1, 1.3, 0.8, steel.hi, steel.mid, steel.lo, flash);
        }
        // Detached brass tally wheel lying sideways!
        let wc = center + f * (5.0 + burst * 5.5) - Vec2::new(0.0, 1.0 + bounce);
        mb_a_ring(pb, wc.x, wc.y, 3.0, 6.2, brass, flash);
        draw_shaded_disc(pb, wc.x, wc.y, 2.6, steel.hi, steel.mid, steel.lo, flash);
        // Snapped crimson ratchet flag.
        let fl_p = center - f * (5.0 + burst * 4.0) - Vec2::new(0.0, 2.0 + bounce * 0.7);
        draw_shaded_disc(pb, fl_p.x, fl_p.y, 1.9, ruby.hi, ruby.mid, ruby.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, brass.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 22.0)
    } else {
        (0.0, 0.0)
    };

    // Quantized mechanical ratchet click angle (discrete stepped clicks + tiny snap overshoot!).
    let raw_ticks = match pose {
        Pose::Idle => anim * 2.0,
        Pose::Approach => anim * 5.5,
        Pose::Tell(t) => anim * 2.0 + t * 14.0,
        Pose::Strike(t) => anim * 2.0 + 14.0 + (t * 3.0).floor() * 3.0,
        Pose::Retreat(t) => -t * 6.0,
        _ => anim * 1.5,
    };
    let tick_i = raw_ticks.floor();
    let tick_frac = raw_ticks - tick_i;
    let snap = if tick_frac < 0.22 {
        (tick_frac / 0.22 * PI).sin() * 0.16
    } else {
        0.0
    };
    let wheel_spin = (tick_i + snap) * (TAU / 10.0);
    let pawl_kick = if tick_frac < 0.24 { (1.0 - tick_frac / 0.24) * 2.2 } else { 0.0 };

    let (lift, sway, leg_brace, strike_beat_idx, strike_flash) = match pose {
        Pose::Entrance(_) => (ent_y, 0.0, 1.0, -1i32, 0.0),
        Pose::Idle => ((anim * 2.2).sin() * 0.45, 0.0, 1.0, -1, 0.0),
        Pose::Approach => (-(anim * 7.2).sin().abs() * 1.6, (anim * 7.2).sin() * 1.3, 1.05, -1, 0.0),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-1.8 * c, q * 0.9, 1.0 + 0.24 * c, -1, 0.0)
        }
        Pose::Strike(t) => {
            let idx = ((t * 3.0).floor() as i32).clamp(0, 2);
            let beat = (t * 3.0).fract();
            let fl = if beat < 0.45 { 1.0 - beat / 0.45 } else { 0.0 };
            (-2.6 * fl, 0.0, 1.24, idx, fl)
        }
        Pose::Recover(t) => (1.8, (t * 10.0).sin() * 1.0, 1.12, -1, 0.0),
        Pose::Retreat(t) => (-(t * 10.0).sin().abs() * 1.2, (t * 10.0).sin() * 1.2, 0.95, -1, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + jolt + Vec2::new(sway, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.46, 1.15);
    draw_shadow(pb, center.x + 1.2, center.y + 8.6, 12.2 * sh_s + ent_sq * 2.0, 5.2 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), ruby.hi);
        }
    }

    // 1. Three telescopic gunmetal tripod legs stepping in Approach & bracing wide in Tell!
    let leg_x = [-8.6_f32, 0.0, 8.6];
    let leg_y = [8.2_f32, 9.6, 8.2];
    for (k, (&lx, &ly)) in leg_x.iter().zip(leg_y.iter()).enumerate() {
        let ph = anim * 7.2 + k as f32 * (TAU / 3.0);
        let step_y = if matches!(pose, Pose::Approach | Pose::Retreat(_)) {
            ph.sin().max(0.0) * 2.6
        } else {
            0.0
        };
        let step_x = if matches!(pose, Pose::Approach | Pose::Retreat(_)) {
            ph.cos() * 1.6
        } else {
            0.0
        };
        let hip = base + Vec2::new(lx * 0.25, 2.2);
        let knee = base + Vec2::new(lx * 0.72 * leg_brace + step_x * 0.5, ly * 0.55 - step_y * 0.5);
        let foot = base + Vec2::new(lx * leg_brace + step_x, ly - step_y);
        mb_a_jointed_leg(pb, hip, knee, foot, 1.35, 0.85, steel, brass.hi, flash);
    }

    // 2. Large notched brushed-brass tally wheel (10 teeth + outer brass ring + dark odometer drum)!
    let wheel_c = base + Vec2::new(0.0, -3.2 + ent_sq * 0.8);
    for i in 0..10 {
        let a = wheel_spin + i as f32 * (TAU / 10.0);
        let d = Vec2::from_angle(a);
        let p0 = wheel_c + d * 5.2;
        let p1 = wheel_c + d * 8.2;
        draw_tapered_capsule(pb, p0, p1, 1.35, 0.85, brass.hi, brass.mid, brass.lo, flash);
    }
    mb_a_ring(pb, wheel_c.x, wheel_c.y, 4.4, 6.8, brass, flash);
    draw_shaded_disc(pb, wheel_c.x, wheel_c.y, 4.6, steel.hi, steel.mid, steel.lo, flash);
    // Crisp ivory tally index ticks on the inner odometer drum!
    for k in 0..4 {
        let a = wheel_spin + k as f32 * FRAC_PI_2;
        let q = wheel_c + Vec2::from_angle(a) * 3.5;
        plot_px(pb, q.x.round() as i32, q.y.round() as i32, apply_flash(ivory, flash));
    }
    mb_a_glint(pb, wheel_c + Vec2::new(-4.0, -4.2), brass.hi);

    // 3. Spring-loaded crimson mechanical ratchet clicker flag at 12 o'clock!
    let pawl_tilt = if matches!(pose, Pose::Recover(_)) { 4.2 } else { pawl_kick * 0.8 };
    let pawl_base = wheel_c + Vec2::new(0.0, -7.2);
    let pawl_tip = wheel_c + Vec2::new(pawl_tilt, -11.6 - pawl_kick * 0.5);
    draw_tapered_capsule(pb, pawl_base, pawl_tip, 1.3, 0.9, ruby.hi, ruby.mid, ruby.lo, flash);
    draw_shaded_disc(pb, pawl_tip.x, pawl_tip.y, 1.7, ruby.hi, ruby.mid, ruby.lo, flash);
    if let Pose::Tell(t) = pose {
        if pawl_kick > 0.8 || t > 0.6 {
            mb_a_spark_cross(pb, pawl_base + Vec2::new(0.0, -0.6), 2, (255, 252, 210), brass.hi, 0.85);
        }
    }

    // 4. Lower gunmetal reader console bar carrying THREE BOLD READER LAMPS
    //    (index 0 = left, 1 = right, 2 = centre — matching exact stamp pulse order!).
    let bar_c = base + Vec2::new(0.0, 4.4);
    draw_rotated_box(pb, bar_c, Vec2::new(1.0, 0.0), 8.0, 2.2, steel.hi, steel.mid, steel.lo, flash);
    let lit_count = match pose {
        Pose::Tell(t) => ((t * 3.0).floor() as i32 + 1).clamp(1, 3),
        Pose::Strike(_) => 3,
        _ => 0,
    };
    for (idx, lx) in [(0i32, -5.2_f32), (1, 5.2), (2, 0.0)] {
        let lp = bar_c + Vec2::new(lx, 0.0);
        let is_active_beat = strike_beat_idx == idx;
        let standby = matches!(pose, Pose::Idle) && ((anim * 2.0).floor() as i32).rem_euclid(3) == idx;
        let lp_pal = if is_active_beat {
            pal((255, 255, 248), ruby.hi, ruby.mid)
        } else if idx < lit_count || standby {
            ruby
        } else {
            lamp_off
        };
        draw_shaded_disc(pb, lp.x, lp.y, if is_active_beat { 2.0 } else { 1.6 }, lp_pal.hi, lp_pal.mid, lp_pal.lo, flash);
        if is_active_beat && strike_flash > 0.08 {
            mb_a_spark_cross(pb, lp, 4, (255, 255, 248), ruby.hi, strike_flash);
        }
    }

    // 5. Stern brass-bezeled cyclops optical scanner lens centered on the tally hub!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_c = wheel_c + f * 1.4;
    let eye_col = match pose {
        Pose::Tell(_) => (255, 224, 108),
        Pose::Strike(_) => (255, 168, 152),
        _ => (234, 252, 255),
    };
    draw_eye(pb, eye_c, 2.0, eye_col, steel.lo, f, expr, flash);

    if let Pose::Recover(t) = pose {
        // Overloaded circuit smoke puffs rising from the jammed tally housing!
        for k in 0..2 {
            let ph = (t * 3.0 + k as f32 * 0.5).fract();
            let sp = wheel_c + Vec2::new(((k as f32) - 0.5) * 5.0, -7.0 - ph * 8.0);
            mb_a_puff(pb, sp, 2, (206, 218, 232), (1.0 - ph) * 0.78);
        }
        draw_dizzy_stars(pb, wheel_c + Vec2::new(0.0, -12.5), 7.4, t);
        draw_recover_beacon(pb, wheel_c, t);
    }
    draw_hurt_burst(pb, base, f, flash, brass.hi);
}

// ---- 15. ROTOR TECHNICIAN · "Switch off before removing tail." -------------
// 34–38 px clinical ice-cobalt equilateral triangular centrifuge rotor head
// trailed by a 9-segment high-voltage hazard-yellow/black snaking power cable
// with a sparking two-prong brass plug, and a blazing plasma-edged 18 px steel
// blade that cocks to -75° in Tell and whips through 150° (-75° -> +75°) in Strike.
fn draw_rotor_technician(pb: &mut PixelBuffer, center: Vec2, f: Vec2, pose: Pose, anim: f32, flash: f32) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let shell = pal((228, 246, 255), (118, 176, 222), (42, 74, 118));
    let hazard = pal((255, 246, 152), (242, 196, 42), (126, 90, 16));
    let rubber = pal((98, 104, 122), (46, 50, 64), (20, 22, 32));
    let blade_p = pal((255, 255, 252), (168, 234, 252), (60, 102, 134));
    let spark_hi: Rgb = (255, 252, 220);
    let spark_gold: Rgb = (255, 214, 72);
    let cyan_glow: Rgb = (132, 244, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.2;
        draw_shadow(pb, center.x + 1.5, center.y + 5.8, 16.5, 7.2, 0.50);
        // Frayed hazard cable coiled limp around the wreckage.
        for i in 0..6 {
            let u = i as f32;
            let p_c = center - f * (3.0 + u * 2.4) + s * ((u * 1.1).sin() * 3.5);
            let cp = if i % 2 == 0 { hazard } else { rubber };
            draw_shaded_disc(pb, p_c.x, p_c.y + 2.0, 1.7, cp.hi, cp.mid, cp.lo, flash);
        }
        // Split cobalt rotor chassis rails & flung centrifuge blade!
        for side in [-1.0_f32, 1.0] {
            let r_c = center + s * (side * (4.5 + burst * 5.5)) - Vec2::new(0.0, bounce * 0.8);
            draw_tapered_capsule(pb, r_c - f * 5.0, r_c + f * 5.0, 1.9, 1.6, shell.hi, shell.mid, shell.lo, flash);
        }
        let bl_a = center + f * (5.0 + burst * 6.5) - Vec2::new(0.0, bounce);
        draw_tapered_capsule(pb, bl_a, bl_a + f.rotate(0.5) * 9.0, 1.9, 0.7, blade_p.hi, blade_p.mid, blade_p.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, cyan_glow);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let half_sweep = 75.0_f32.to_radians(); // exact half of 150° Cord sweep
    let (ent_y, ent_sq) = if let Pose::Entrance(t) = pose {
        mb_a_ent_drop(t, 22.0)
    } else {
        (0.0, 0.0)
    };

    let (recoil, lift, head_turn, cable_whip, blade_ang, blade_reach) = match pose {
        Pose::Entrance(t) => (0.0, ent_y, t * 14.0, 1.6, -PI, 9.5),
        Pose::Idle => (0.0, (anim * 2.4).sin() * 0.9, (anim * 1.8).sin() * 0.36, 0.9, -PI + (anim * 2.2).sin() * 0.12, 10.0),
        Pose::Approach => ((anim * 7.0).sin() * 0.9, (anim * 5.5).cos() * 1.0 - 0.8, anim * 2.6, 1.5, -PI, 10.2),
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            let k = c.min(1.0);
            // Head squares dead onto `f`; blade swings out cleanly from -PI to -75°!
            (-3.2 * c + q * 0.7, -2.0 * c, q * 0.08, 1.4 + c * 0.8, -PI + (PI - half_sweep) * k, 10.0 + 4.8 * k)
        }
        Pose::Strike(t) => {
            let ang = -half_sweep + 2.0 * half_sweep * t;
            (3.0 * (t * PI).sin(), -0.5, -ang * 0.22, 2.1, ang, 15.5)
        }
        Pose::Recover(t) => (-1.6, 1.8, 0.42 + (t * 8.0).sin() * 0.10, 0.35, half_sweep + 0.14, 12.2),
        Pose::Retreat(t) => (-2.4, -(t * 9.0).sin().abs() * 1.0, -t * 4.0, 1.2, -PI, 9.5),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let base = center + f * recoil + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.025).clamp(0.46, 1.15);
    draw_shadow(pb, base.x + 1.4, center.y + 7.2, 14.2 * sh_s + ent_sq * 2.0, 6.0 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.44 {
            draw_entrance_ring(pb, center, ((t - 0.44) / 0.56).clamp(0.0, 1.0), cyan_glow);
        }
    }

    // 1. Snaking 7-segment high-voltage HAZARD-YELLOW & BLACK power cable + sparking brass plug!
    let mut prev_c = base - f * 5.2;
    let mut tail_dir = -f;
    for i in 0..7 {
        let u = i as f32;
        let ph = anim * 5.0 - u * 0.95;
        let counter = if let Pose::Strike(t) = pose {
            (0.5 - t) * (2.0 + u * 0.75)
        } else {
            0.0
        };
        let amp = (1.0 + u * 0.52) * cable_whip;
        let q = base - f * (6.0 + u * 1.9) + s * (ph.sin() * amp + counter);
        tail_dir = safe_dir(q - prev_c, -f);
        let seg_p = if i % 2 == 0 { hazard } else { rubber };
        let rad = (1.95 - u * 0.08).max(1.2);
        draw_tapered_capsule(pb, prev_c, q, rad, rad * 0.92, seg_p.hi, seg_p.mid, seg_p.lo, flash);
        prev_c = q;
    }
    // Two-prong brass power plug at the cable's tail end with live static ticks!
    let plug_p = tail_dir.perp();
    for side in [-1.0_f32, 1.0] {
        let pr0 = prev_c + plug_p * (side * 1.1);
        let pr1 = pr0 + tail_dir * 2.4;
        draw_tapered_capsule(pb, pr0, pr1, 0.75, 0.55, hazard.hi, hazard.mid, hazard.lo, flash);
    }
    if (anim * 4.2).fract() < 0.35 || matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        mb_a_spark_cross(pb, prev_c + tail_dir * 2.8, 2, spark_hi, cyan_glow, 0.85);
    }

    // 2. Strike wide dazzling electric-cyan & gold 150° plasma blade smear!
    let blade_dir = f.rotate(blade_ang);
    let blade_tip = base + blade_dir * blade_reach;
    if let Pose::Strike(t) = pose {
        let base_a = f.angle();
        let cur_a = base_a + blade_ang;
        let trail_a = (base_a - half_sweep).max(cur_a - 1.35);
        draw_smear_arc(pb, base, trail_a, cur_a, 8.5, 18.0, cyan_glow, spark_hi, 0.82 * (1.0 - t * 0.2));
        draw_smear_arc(pb, base, trail_a, cur_a, 11.5, 17.0, spark_gold, spark_hi, 0.65 * (1.0 - t * 0.25));
    }

    // 3. Sculpted equilateral triangular ice-cobalt centrifuge rotor head!
    let corners: [Vec2; 3] =
        std::array::from_fn(|k| base + f.rotate(head_turn + k as f32 * (TAU / 3.0)) * 9.8);
    // Dark recessed turbine housing disc behind the delta frame.
    draw_shaded_disc(pb, base.x, base.y, 6.4, rubber.hi, rubber.mid, rubber.lo, flash);
    // Thick chamfered triangular frame rails + corner weight pods.
    for k in 0..3 {
        let a = corners[k];
        let b = corners[(k + 1) % 3];
        draw_tapered_capsule(pb, a, b, 2.0, 2.0, shell.hi, shell.mid, shell.lo, flash);
    }
    for c_pt in corners {
        draw_shaded_disc(pb, c_pt.x, c_pt.y, 2.3, shell.hi, shell.mid, shell.lo, flash);
        plot_px(pb, c_pt.x.round() as i32, c_pt.y.round() as i32, apply_flash(hazard.hi, flash));
    }
    // Pulsing central golden-cyan centrifuge motor core!
    let core_pulse = 0.5 + 0.5 * (anim * 6.0).sin();
    let core_hi = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        spark_hi
    } else {
        mix(cyan_glow, spark_gold, core_pulse)
    };
    draw_shaded_disc(pb, base.x, base.y, 4.0, shell.hi, shell.mid, shell.lo, flash);
    draw_shaded_disc(pb, base.x, base.y, 2.4, core_hi, spark_gold, hazard.lo, flash);
    mb_a_glint(pb, corners[0] + Vec2::new(-1.0, -1.0), shell.hi);

    // 4. Long razor centrifuge rotor blade with live plasma-spark tip!
    draw_tapered_capsule(pb, base + blade_dir * 2.5, blade_tip, 2.2, 0.9, blade_p.hi, blade_p.mid, blade_p.lo, flash);
    // Bright cyan edge highlight running down the blade's leading edge!
    let edge_a = base + blade_dir * 4.5 + blade_dir.perp() * 0.9;
    let edge_b = blade_tip + blade_dir.perp() * 0.4;
    pb.line(edge_a.x.round() as i32, edge_a.y.round() as i32, edge_b.x.round() as i32, edge_b.y.round() as i32, (255, 255, 255));
    if let Pose::Tell(t) = pose {
        let c = tell_coil(t);
        mb_a_spark_cross(pb, blade_tip + blade_dir * 1.0, 3, spark_hi, spark_gold, c.min(1.0));
        // Crackling electric arc nodes dancing along the cocked blade!
        for k in 0..3 {
            let u = (t * 4.5 + k as f32 * 0.33).fract();
            let sp = base + blade_dir * (5.0 + u * (blade_reach - 5.0)) + blade_dir.perp() * ((t * 40.0 + k as f32 * 3.0).sin() * 2.2);
            plot_px(pb, sp.x.round() as i32, sp.y.round() as i32, if k % 2 == 0 { spark_hi } else { cyan_glow });
        }
    } else if matches!(pose, Pose::Strike(_)) {
        mb_a_spark_cross(pb, blade_tip + blade_dir * 1.2, 4, spark_hi, cyan_glow, 0.95);
    }

    // 5. Twin cold glowing diagnostic visor eyes set on the forward bevel!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) | Pose::Strike(_) => spark_gold,
        _ => (226, 254, 255),
    };
    for side in [-1.0_f32, 1.0] {
        let ep = base + f * 3.4 + s * (side * 2.2);
        draw_eye(pb, ep, 1.35, eye_col, shell.lo, f, expr, flash);
    }

    if let Pose::Recover(t) = pose {
        // Stalled turbine smoking & flickering sparks!
        for k in 0..2 {
            let ph = (t * 3.2 + k as f32 * 0.5).fract();
            let sp = base + Vec2::new(((k as f32) - 0.5) * 4.5, -4.5 - ph * 8.0);
            mb_a_puff(pb, sp, 2, (206, 224, 238), (1.0 - ph) * 0.78);
        }
        draw_dizzy_stars(pb, base + Vec2::new(0.0, -11.5), 7.4, t);
        draw_recover_beacon(pb, base, t);
    }
    draw_hurt_burst(pb, base, f, flash, cyan_glow);
}

// ============================================================================
// Private helpers for minibosses 16–24 (Laundry, Printworks, Flatbed)
// ============================================================================

/// Three-stop shaded material palette (light rim, rich midtone, deep shadow).
#[derive(Clone, Copy)]
struct Mb2Pal {
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
}

#[inline]
const fn mb2_pal(hi: Rgb, mid: Rgb, lo: Rgb) -> Mb2Pal {
    Mb2Pal { hi, mid, lo }
}

/// Crisp dark-outlined warning "!" pip floating above a winding-up miniboss so
/// Tell reads cleanly in low ambient light, under Tell bloom, and at 2× zoom.
fn mb2_tell_pip(pb: &mut PixelBuffer, top: Vec2, t: f32) {
    if t < 0.58 && (t * 11.0).fract() >= 0.62 {
        return;
    }
    let x = top.x.round() as i32;
    let y = top.y.round() as i32;
    let col: Rgb = if t > 0.84 {
        (255, 250, 186)
    } else {
        (255, 214, 72)
    };
    // 1 px dark ink outline behind the exclamation stem & dot.
    pb.rect_fill(x - 1, y - 1, 4, 8, INK);
    for dy in 0..4 {
        plot_px(pb, x, y + dy, col);
        plot_px(pb, x + 1, y + dy, col);
    }
    plot_px(pb, x, y + 5, col);
    plot_px(pb, x + 1, y + 5, col);
}

/// Four-point impact cross-flash for Strike climaxes.
fn mb2_star_flash(pb: &mut PixelBuffer, at: Vec2, r: i32, core: Rgb, outer: Rgb) {
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    let rf = r.max(1) as f32;
    for d in -r..=r {
        let a = (1.0 - (d.abs() as f32 / (rf + 1.0))).clamp(0.0, 1.0);
        let col = if d.abs() <= 1 { core } else { outer };
        blend_px(pb, cx + d, cy, col, 0.92 * a);
        blend_px(pb, cx, cy + d, col, 0.92 * a);
    }
    let diag = (r / 2).max(1);
    for d in -diag..=diag {
        blend_px(pb, cx + d, cy + d, outer, 0.62);
        blend_px(pb, cx + d, cy - d, outer, 0.62);
    }
    plot_px(pb, cx, cy, (255, 255, 255));
}

/// Exact signed-distance rotated regular hexagon (inradius `h`) with crisp
/// dark outline, bright machined top-left chamfer bevel (`spec`), and shaded face.
fn mb2_draw_hex(
    pb: &mut PixelBuffer,
    center: Vec2,
    h: f32,
    angle: f32,
    pal: Mb2Pal,
    spec: Rgb,
    flash: f32,
) {
    let circum = h * 1.154_700_5;
    let pad = circum + 1.6;
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((center.x - pad).floor() as i32).max(0);
    let x1 = ((center.x + pad).ceil() as i32).min(bw - 1);
    let y0 = ((center.y - pad).floor() as i32).max(0);
    let y1 = ((center.y + pad).ceil() as i32).min(bh - 1);
    let (s, c) = angle.sin_cos();
    let inv_h = 1.0 / h.max(1.0);
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - center.y;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - center.x;
            let lx = (dx * c + dy * s).abs();
            let ly = (-dx * s + dy * c).abs();
            let d = ly.max(lx * 0.866_025_4 + ly * 0.5);
            if d > h + 0.95 {
                continue;
            }
            let col = if d > h - 0.25 {
                ink_c
            } else if d > h - 2.2 {
                // Machined chamfer bevel catches a crisp light rim all around the hex.
                let ldot = (dx * LIGHT.x + dy * LIGHT.y) * inv_h;
                let bev = if ldot > 0.08 {
                    spec
                } else if ldot > -0.35 {
                    pal.hi
                } else {
                    pal.mid
                };
                apply_flash(bev, flash)
            } else {
                let lit = (0.50 + 0.52 * ((dx * LIGHT.x + dy * LIGHT.y) * inv_h)).clamp(0.0, 1.0);
                let base = if lit > 0.56 {
                    mix(pal.mid, pal.hi, (lit - 0.56) / 0.44)
                } else {
                    mix(pal.lo, pal.mid, lit / 0.56)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, col);
        }
    }
}

/// Shaded outlined triangle (exact edge signed-distance; no per-pixel `sqrt`).
fn mb2_draw_poly_tri(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    c: Vec2,
    pal: Mb2Pal,
    flash: f32,
) {
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 0.5 {
        return;
    }
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    let edge_n = |p0: Vec2, p1: Vec2| -> Vec2 {
        let e = p1 - p0;
        let l = e.len().max(1e-4);
        Vec2::new((e.y / l) * sign, (-e.x / l) * sign)
    };
    let n0 = edge_n(a, b);
    let n1 = edge_n(b, c);
    let n2 = edge_n(c, a);
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((a.x.min(b.x).min(c.x) - 1.5).floor() as i32).max(0);
    let x1 = ((a.x.max(b.x).max(c.x) + 1.5).ceil() as i32).min(bw - 1);
    let y0 = ((a.y.min(b.y).min(c.y) - 1.5).floor() as i32).max(0);
    let y1 = ((a.y.max(b.y).max(c.y) + 1.5).ceil() as i32).min(bh - 1);
    let ctr = (a + b + c) * (1.0 / 3.0);
    let inv_rad = 1.0 / a.dist(ctr).max(b.dist(ctr)).max(c.dist(ctr)).max(1.5);
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let py = y as f32 + 0.5;
        for x in x0..=x1 {
            let px = x as f32 + 0.5;
            let d0 = (px - a.x) * n0.x + (py - a.y) * n0.y;
            let d1 = (px - b.x) * n1.x + (py - b.y) * n1.y;
            let d2 = (px - c.x) * n2.x + (py - c.y) * n2.y;
            let dmax = d0.max(d1).max(d2);
            if dmax > 0.90 {
                continue;
            }
            let col = if dmax > -0.25 {
                ink_c
            } else {
                let lit = (0.52 + 0.52 * (((px - ctr.x) * LIGHT.x + (py - ctr.y) * LIGHT.y) * inv_rad))
                    .clamp(0.0, 1.0);
                let base = if lit > 0.58 {
                    mix(pal.mid, pal.hi, (lit - 0.58) / 0.42)
                } else {
                    mix(pal.lo, pal.mid, lit / 0.58)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, col);
        }
    }
}

/// Oval forged-iron chain link from `a` to `b` with crisp `INK` outer contour
/// and a dark recessed center slot so links read as real chain metal.
fn mb2_draw_ring_link(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    r: f32,
    pal: Mb2Pal,
    flash: f32,
) {
    draw_capsule(pb, a, b, r, pal.hi, pal.mid, pal.lo, flash);
    let mid = (a + b) * 0.5;
    let dir = safe_dir(b - a, Vec2::new(1.0, 0.0));
    let slot_half = ((b - a).len() * 0.32).max(0.4);
    let s0 = mid - dir * slot_half;
    let s1 = mid + dir * slot_half;
    pb.line(
        s0.x.round() as i32,
        s0.y.round() as i32,
        s1.x.round() as i32,
        s1.y.round() as i32,
        apply_flash(INK, flash * 0.25),
    );
    // Crisp top-left metal specular gleam so forged links survive 60% ambient light.
    let glint = mid + Vec2::new(-0.9, -0.9);
    plot_px(pb, glint.x.round() as i32, glint.y.round() as i32, apply_flash((248, 252, 255), flash));
}

/// Elliptical soap dome with one clean flat broken cleavage face perpendicular
/// to `cut_dir` at normalized offset `cut_frac`, plus iridescent rim sheen.
#[allow(clippy::too_many_arguments)]
fn mb2_draw_sliced_dome(
    pb: &mut PixelBuffer,
    center: Vec2,
    rx: f32,
    ry: f32,
    cut_dir: Vec2,
    cut_frac: f32,
    pal: Mb2Pal,
    lip_col: Rgb,
    inner_col: Rgb,
    irid_phase: f32,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let ox = rx + 1.2;
    let oy = ry + 1.2;
    let x0 = ((center.x - ox).floor() as i32).max(0);
    let x1 = ((center.x + ox).ceil() as i32).min(bw - 1);
    let y0 = ((center.y - oy).floor() as i32).max(0);
    let y1 = ((center.y + oy).ceil() as i32).min(bh - 1);
    let inv_rx = 1.0 / rx.max(1.0);
    let inv_ry = 1.0 / ry.max(1.0);
    let ink_c = apply_flash(INK, flash * 0.35);
    let irid_pink: Rgb = (232, 162, 248);
    let irid_mint: Rgb = (144, 252, 232);
    for y in y0..=y1 {
        let ny = (y as f32 + 0.5 - center.y) * inv_ry;
        let ny2 = ny * ny;
        for x in x0..=x1 {
            let nx = (x as f32 + 0.5 - center.x) * inv_rx;
            let d2 = nx * nx + ny2;
            if d2 > 1.18 {
                continue;
            }
            let proj = nx * cut_dir.x + ny * cut_dir.y;
            if proj > cut_frac + 0.14 {
                continue;
            }
            let col = if d2 > 0.93 || proj > cut_frac + 0.04 {
                ink_c
            } else if proj > cut_frac - 0.15 {
                // Flat fractured soap slab face: razor-bright lip + dark recessed glass.
                let edge_c = if proj > cut_frac - 0.05 { lip_col } else { inner_col };
                apply_flash(edge_c, flash)
            } else {
                let lit = (0.48 + 0.56 * (nx * LIGHT.x + ny * LIGHT.y)).clamp(0.0, 1.0);
                let mut base = if lit > 0.58 {
                    mix(pal.mid, pal.hi, (lit - 0.58) / 0.42)
                } else {
                    mix(pal.lo, pal.mid, lit / 0.58)
                };
                // Subtle translucent soap-film iridescence along the upper outer rim.
                if d2 > 0.48 && ny < 0.35 {
                    let wave = (nx * 3.8 - ny * 2.4 + irid_phase).sin() * 0.5 + 0.5;
                    let tint = mix(irid_mint, irid_pink, wave);
                    base = mix(base, tint, 0.28 * ((d2 - 0.48) / 0.45).clamp(0.0, 1.0));
                }
                apply_flash(base, flash)
            };
            pb.plot(x, y, col);
        }
    }
}

/// Solid curved rotary printing plate arc with zinc-steel outer bevel, wet
/// royal-violet inked inner printing face, and a jagged torn registration notch
/// aligned with `gap_dir`.
#[allow(clippy::too_many_arguments)]
fn mb2_draw_curved_plate(
    pb: &mut PixelBuffer,
    hub: Vec2,
    f: Vec2,
    gap_dir: Vec2,
    r_in: f32,
    r_out: f32,
    half_span: f32,
    steel: Mb2Pal,
    ink_pal: Mb2Pal,
    tear_col: Rgb,
    flash: f32,
) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let pad = r_out + 1.6;
    let x0 = ((hub.x - pad).floor() as i32).max(0);
    let x1 = ((hub.x + pad).ceil() as i32).min(bw - 1);
    let y0 = ((hub.y - pad).floor() as i32).max(0);
    let y1 = ((hub.y + pad).ceil() as i32).min(bh - 1);
    let r_in_lo = (r_in - 1.1).max(0.5);
    let r_out_hi = r_out + 1.1;
    let r_in_lo2 = r_in_lo * r_in_lo;
    let r_out_hi2 = r_out_hi * r_out_hi;
    let cos_outer = (half_span + 0.08).cos();
    let cos_inner = half_span.cos();
    let cos_notch = 0.935_f32; // ~21° half-angle torn bite cleanly visible at 2× zoom
    let ink_c = apply_flash(INK, flash * 0.35);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - hub.y;
        let dy2 = dy * dy;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - hub.x;
            let d2 = dx * dx + dy2;
            if d2 < r_in_lo2 || d2 > r_out_hi2 {
                continue;
            }
            let d = d2.sqrt();
            let inv_d = 1.0 / d.max(1e-4);
            let ux = dx * inv_d;
            let uy = dy * inv_d;
            let fdot = ux * f.x + uy * f.y;
            if fdot < cos_outer {
                continue;
            }
            let gdot = ux * gap_dir.x + uy * gap_dir.y;
            let jag = (d * 2.1).sin() * 0.022;
            if gdot > cos_notch + jag {
                continue;
            }
            let on_tear = gdot > cos_notch - 0.055 + jag;
            let on_rim = d > r_out - 0.25 || d < r_in + 0.25 || fdot < cos_inner;
            let col = if on_tear {
                apply_flash(tear_col, flash)
            } else if on_rim {
                ink_c
            } else if d < r_in + 2.3 {
                // Wet violet press-ink face with embossed lead type-line ticks.
                let ridge = ((ux * f.y - uy * f.x) * 26.0).sin() > 0.35;
                let base = if ridge {
                    ink_pal.hi
                } else if d < r_in + 1.2 {
                    ink_pal.lo
                } else {
                    ink_pal.mid
                };
                apply_flash(base, flash)
            } else {
                let lit = (0.50 + 0.54 * (ux * LIGHT.x + uy * LIGHT.y)).clamp(0.0, 1.0);
                let base = if d > r_out - 1.55 {
                    // Continuous bright zinc-steel outer rim bevel so the curved plate reads on charcoal slabs.
                    if lit > 0.42 { (255, 255, 255) } else { steel.hi }
                } else if lit > 0.54 {
                    mix(steel.mid, steel.hi, (lit - 0.54) / 0.46)
                } else {
                    mix(steel.lo, steel.mid, lit / 0.54)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, col);
        }
    }
}

// ============================================================================
// 16. SUDS BAILIFF — "All delicates report to rinse." (Laundry 9:00)
//     27–30 px translucent cerulean soap dome with one flat broken cleavage
//     edge aligned to RING_GAP_TURN, constable cap, foaming mustache & boots
// ============================================================================

fn draw_suds_bailiff(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let gap = f.rotate(RING_GAP_TURN);
    let soap = mb2_pal((164, 246, 255), (72, 174, 210), (28, 86, 122));
    let foam = mb2_pal((246, 255, 255), (176, 224, 238), (74, 120, 142));
    let cap_navy = mb2_pal((138, 164, 224), (70, 88, 144), (28, 36, 66));
    let brass = mb2_pal((255, 236, 134), (206, 152, 48), (102, 66, 18));
    let boot = mb2_pal((116, 132, 162), (62, 74, 98), (28, 34, 48));
    let lip_col: Rgb = (224, 255, 255);
    let inner_col: Rgb = (20, 56, 78);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 16.0, 7.2, 0.48);
        // Spreading foamy soap puddle with iridescent bubbles!
        let pr = 8.5 + burst * 6.5;
        draw_shaded_ellipse(
            pb,
            center.x,
            center.y + 3.0,
            pr,
            pr * 0.52,
            foam.hi,
            soap.mid,
            soap.lo,
            flash,
        );
        // Two shattered soap-dome shards flung apart along the cleavage line.
        let sep = gap.perp() * (3.5 + burst * 6.0);
        let s1 = center - sep - Vec2::new(0.0, bounce * 0.7);
        let s2 = center + sep + Vec2::new(0.0, 1.0 - bounce * 0.5);
        draw_shaded_disc(pb, s1.x, s1.y, 4.6, soap.hi, soap.mid, soap.lo, flash);
        draw_shaded_disc(pb, s2.x, s2.y, 3.8, foam.hi, soap.mid, soap.lo, flash);
        // Upside-down navy bailiff cap with gleaming brass badge tumbled beside the chest!
        let cap_p = center + f * (5.0 + burst * 4.5) - Vec2::new(0.0, bounce * 1.1);
        draw_rotated_box(
            pb,
            cap_p,
            f.rotate(0.4),
            4.8,
            2.4,
            cap_navy.hi,
            cap_navy.mid,
            cap_navy.lo,
            flash,
        );
        draw_shaded_disc(pb, cap_p.x, cap_p.y, 1.5, brass.hi, brass.mid, brass.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, lip_col);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.45 {
            let u = t / 0.45;
            (1.0 - u * u) * -22.0
        } else {
            let u = (t - 0.45) / 0.55;
            (u * PI * 2.0).sin() * (1.0 - u) * 2.4
        }
    } else {
        0.0
    };

    let (lift, squash, rattle, cap_pop, coil) = match pose {
        Pose::Entrance(t) => (
            ent_drop,
            if (0.45..0.64).contains(&t) { 1.6 } else { 0.0 },
            0.0,
            if t > 0.45 { ((t - 0.45) * PI * 2.0).sin().abs() * 2.6 } else { 0.0 },
            0.0,
        ),
        Pose::Idle => ((anim * 2.6).sin() * 0.8, (anim * 2.6).cos() * 0.35, 0.0, 0.0, 0.0),
        Pose::Approach => {
            let st = (anim * 6.4).sin();
            (-st.abs() * 1.8, st.abs() * 0.5, st * 0.9, st.abs() * 0.6, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-1.2 * c, 1.6 * c, q * 1.3, 2.8 * c, c)
        }
        Pose::Strike(t) => {
            let slam = (1.0 - t) * (t * 48.0).sin() * 1.8;
            (1.6 * (1.0 - t * 0.5), 2.2 * (1.0 - t * 0.6), slam, 1.2 * (1.0 - t), 1.0)
        }
        Pose::Recover(t) => (
            2.2,
            2.4 - t * 0.8,
            (t * 9.0).sin() * 0.9,
            -0.5,
            0.0,
        ),
        Pose::Retreat(t) => (-(t * 9.0).sin().abs() * 1.8, 0.0, -1.2, 0.4, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + jolt + Vec2::new(rattle, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.15);
    draw_shadow(pb, center.x + 1.2, center.y + 8.0, 14.5 * sh_s, 6.4 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), soap.hi);
        }
    }

    // 1. Chunky rubber galosh boots marching beneath the foam skirt.
    let stride = match pose {
        Pose::Approach => (anim * 6.4).sin() * 3.0,
        Pose::Retreat(t) => (t * 9.0).sin() * 2.2,
        _ => 0.0,
    };
    for &side in &[-1.0_f32, 1.0] {
        let foot = body
            + f * (side * stride)
            + s * (side * 4.8)
            + Vec2::new(0.0, 7.6 - (side * stride).max(0.0) * 0.45);
        draw_shaded_ellipse(pb, foot.x, foot.y, 3.0, 2.0, boot.hi, boot.mid, boot.lo, flash);
        plot_px(pb, foot.x.round() as i32 - 1, foot.y.round() as i32 - 1, (166, 186, 214));
    }

    // 2. Churning foamy suds skirt at base of dome.
    let skirt_y = body.y + 5.8;
    for k in 0..7 {
        let u = (k as f32 - 3.0) / 3.0;
        let bob = (anim * 4.2 + k as f32 * 1.3).sin() * 0.8;
        let p_b = Vec2::new(body.x + u * (9.5 + squash), skirt_y + bob + u.abs() * 0.8);
        let r = 2.4 + (1.0 - u.abs() * 0.35) * 0.9;
        draw_shaded_disc(pb, p_b.x, p_b.y, r, foam.hi, foam.mid, foam.lo, flash);
    }

    // 3. Strike outward foaming splash arcs (skipping the 65° safe gap!).
    if let Pose::Strike(t) = pose {
        let gap_ang = gap.angle();
        let a0 = gap_ang + 0.58;
        let a1 = gap_ang + TAU - 0.58;
        let r0 = 9.5 + t * 5.0;
        let r1 = 14.5 + t * 7.5;
        draw_smear_arc(
            pb,
            body,
            a0,
            a1,
            r0,
            r1,
            soap.hi,
            (255, 255, 255),
            0.74 * (1.0 - t * 0.55),
        );
    }

    // 4. Main translucent cerulean soap dome with ONE FLAT BROKEN EDGE along `gap`!
    let dome_c = body + Vec2::new(0.0, -1.6 + squash * 0.35);
    let dome_rx = 10.6 + squash * 0.9;
    let dome_ry = (10.0 - squash * 0.7).max(6.8);
    let break_lip = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        (255, 248, 168)
    } else {
        lip_col
    };
    mb2_draw_sliced_dome(
        pb,
        dome_c,
        dome_rx,
        dome_ry,
        gap,
        0.50,
        soap,
        break_lip,
        inner_col,
        anim * 2.2,
        flash,
    );

    // Bright specular soap-bubble highlight crescent on top-left (away from broken face).
    if gap.x > -0.35 || gap.y > -0.35 {
        draw_capsule(
            pb,
            dome_c + Vec2::new(-5.8, -3.4),
            dome_c + Vec2::new(-3.2, -5.8),
            1.1,
            (255, 255, 255),
            lip_col,
            soap.hi,
            flash,
        );
    }

    // Fracture gap indicator rays on Tell & Strike pointing cleanly out the flat broken edge!
    if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        let t_ray = match pose {
            Pose::Tell(t) => t,
            Pose::Strike(t) => 1.0 - t * 0.5,
            _ => 0.0,
        };
        let gp = gap.perp();
        let chord_c = dome_c + Vec2::new(gap.x * dome_rx * 0.56, gap.y * dome_ry * 0.56);
        for &side in &[-1.0_f32, 1.0] {
            let p0 = chord_c + gp * (side * 5.2);
            let p1 = p0 + gap.rotate(side * 0.44) * (4.0 + t_ray * 4.5);
            pb.line(
                p0.x.round() as i32,
                p0.y.round() as i32,
                p1.x.round() as i32,
                p1.y.round() as i32,
                (255, 238, 128),
            );
        }
    }

    // 5. Pressurized foaming bubbles boiling around the intact rim (skipping the broken edge!).
    let n_bub = 5 + (coil * 7.0) as i32;
    for k in 0..n_bub {
        let ang = anim * (1.4 + coil * 2.4) + k as f32 * (TAU / n_bub as f32);
        let dir = Vec2::from_angle(ang);
        if dir.dot(gap) > 0.78 {
            continue;
        }
        let rad = 11.2 + (k as f32 * 1.9 + anim * 3.8).sin() * 1.4 + coil * 3.4;
        let bp = dome_c + Vec2::new(dir.x * rad, dir.y * rad * 0.88);
        let br = if k % 2 == 0 { 1.7 } else { 1.2 };
        draw_shaded_disc(pb, bp.x, bp.y, br, (255, 255, 255), foam.hi, soap.mid, flash);
    }

    // 6. Stern Bailiff face: piercing amber eyes + bristling foamy handlebar mustache!
    let expr = eye_expr_for(pose, anim, flash);
    let face_c = dome_c + f * 3.6 + Vec2::new(0.0, 0.2);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 232, 92),
        Pose::Strike(_) => (255, 138, 108),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 214, 102),
    };
    draw_eye(pb, face_c + s * 2.7 + Vec2::new(0.0, -1.6), 1.7, eye_col, INK, f, expr, flash);
    draw_eye(pb, face_c - s * 2.7 + Vec2::new(0.0, -1.6), 1.7, eye_col, INK, f, expr, flash);

    // Foamy handlebar mustache curls!
    let stache_flare = 1.0 + coil * 0.45 + (anim * 3.4).sin() * 0.08;
    for &side in &[-1.0_f32, 1.0] {
        let m0 = face_c + Vec2::new(0.0, 1.4) + s * (side * 0.8);
        let m1 = face_c + f * 1.4 + s * (side * 4.6 * stache_flare) + Vec2::new(0.0, 2.2 - coil * 1.0);
        draw_tapered_capsule(pb, m0, m1, 1.5, 0.9, (255, 255, 255), foam.hi, soap.mid, flash);
    }

    // 7. Tall constable navy peaked Bailiff's cap with glossy visor & gleaming brass badge!
    let cap_tilt = match pose {
        Pose::Recover(_) => 0.48,
        Pose::Approach => (anim * 6.4).sin() * 0.14,
        _ => 0.0,
    };
    let cap_off = if matches!(pose, Pose::Recover(_)) {
        s * 3.6 + Vec2::new(0.0, 1.8)
    } else {
        Vec2::new(0.0, -dome_ry - 1.2 - cap_pop)
    };
    let cap_c = dome_c + cap_off;
    let cap_axis = Vec2::from_angle(cap_tilt);
    // Cap crown
    draw_rotated_box(
        pb,
        cap_c + Vec2::new(0.0, -1.6),
        cap_axis,
        5.2,
        2.6,
        cap_navy.hi,
        cap_navy.mid,
        cap_navy.lo,
        flash,
    );
    // Glossy visor brim projecting toward `f` with bright silver piping highlight.
    draw_rotated_box(
        pb,
        cap_c + f * 1.6 + Vec2::new(0.0, 0.6),
        cap_axis,
        6.4,
        1.1,
        (164, 184, 214),
        (58, 68, 96),
        INK,
        flash,
    );
    // Gleaming golden Bailiff star badge centered on the crown!
    let badge = cap_c + f * 1.4 + Vec2::new(0.0, -1.6);
    draw_shaded_disc(pb, badge.x, badge.y, 1.8, (255, 252, 204), brass.hi, brass.lo, flash);

    // Retreat: indignant soap bubbles drifting off top of cap.
    if let Pose::Retreat(t) = pose {
        for k in 0..3 {
            let ph = (t * 2.8 + k as f32 * 0.33).fract();
            let q = cap_c + Vec2::new((k as f32 - 1.0) * 3.0, -3.5 - ph * 6.5);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, lip_col, (1.0 - ph) * 0.85);
        }
    }

    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, cap_c + Vec2::new(0.0, -9.5), t);
    }
    if let Pose::Recover(t) = pose {
        let broken_spot = dome_c + Vec2::new(gap.x * dome_rx * 0.55, gap.y * dome_ry * 0.55);
        draw_dizzy_stars(pb, dome_c + Vec2::new(0.0, -11.5), 7.4, t);
        draw_recover_beacon(pb, broken_spot, t);
    }
    draw_hurt_burst(pb, body, f, flash, lip_col);
}

// ============================================================================
// 17. THE WRINGER BOY — "Please remove hands before pressing." (Laundry 12:00)
//     36–38 px crimson cast-iron twin-roller mangle frame with spinning ribbed
//     steel rollers across `s`, brass crank, and limp yellow rubber glove in seam
// ============================================================================

fn draw_wringer_boy(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let iron = mb2_pal((244, 112, 114), (174, 58, 66), (78, 24, 32));
    let roller = mb2_pal((224, 240, 255), (138, 158, 188), (58, 70, 92));
    let rubber = mb2_pal((154, 144, 184), (88, 78, 114), (38, 34, 52));
    let brass = mb2_pal((255, 232, 128), (204, 148, 44), (98, 64, 18));
    let glove = mb2_pal((255, 224, 112), (234, 152, 42), (132, 72, 18));
    let chrome: Rgb = (244, 250, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 6.0, 18.0, 7.5, 0.50);
        // Snapped crimson iron bridge halves!
        for &side in &[-1.0_f32, 1.0] {
            let drift = s * (side * (7.0 + burst * 6.5)) + f * (side * burst * 2.5)
                - Vec2::new(0.0, bounce * 0.7);
            draw_rotated_box(
                pb,
                center + drift,
                s.rotate(side * 0.25),
                6.2,
                3.0,
                roller.hi,
                roller.mid,
                roller.lo,
                flash,
            );
            draw_shaded_disc(
                pb,
                (center + drift - s * (side * 5.0)).x,
                (center + drift - s * (side * 5.0)).y,
                2.8,
                iron.hi,
                iron.mid,
                iron.lo,
                flash,
            );
        }
        // Limp marigold rubber glove sprawled dramatically pointing at the chest!
        let gp = center + f * (4.0 + burst * 3.5) - Vec2::new(0.0, bounce * 0.9);
        draw_shaded_disc(pb, gp.x, gp.y, 2.6, glove.hi, glove.mid, glove.lo, flash);
        for k in -1..=2 {
            let tip = gp + f.rotate(k as f32 * 0.35) * 4.6;
            draw_tapered_capsule(pb, gp, tip, 1.0, 0.6, glove.hi, glove.mid, glove.lo, flash);
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, brass.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -22.0
        } else {
            let u = (t - 0.44) / 0.56;
            (u * PI * 2.0).sin() * (1.0 - u) * 2.2
        }
    } else {
        0.0
    };

    let (lunge, lift, seam_half, spin_rate, skew, coil) = match pose {
        Pose::Entrance(_) => (0.0, ent_drop, 4.2, 8.0, 0.0, 0.0),
        Pose::Idle => (
            (anim * 2.0).sin() * 0.6,
            (anim * 2.0).cos() * 0.4,
            4.2,
            2.6,
            0.0,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 6.0).sin();
            (st * 1.8, -st.abs() * 1.4, 4.2, 6.2, st * 0.10, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.8 * c + q * 0.8, -2.0 * c, 4.4 + c * 1.4, 14.0 + c * 18.0, q * 0.04, c)
        }
        Pose::Strike(t) => {
            let thrust = (t * PI).sin();
            (5.2 * thrust + 1.4, 0.6, 4.0, 26.0, 0.0, 1.0)
        }
        Pose::Recover(t) => (
            -2.4,
            1.4,
            5.0,
            0.0,
            0.22 * (1.0 - t * 0.3) + (t * 14.0).sin() * 0.04,
            0.0,
        ),
        Pose::Retreat(t) => (-2.6, -(t * 8.0).sin().abs() * 1.2, 4.2, -5.0, 0.0, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.028).clamp(0.45, 1.12);
    draw_shadow(pb, center.x + 1.0, center.y + 6.5, 18.5 * sh_s, 6.8 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), iron.hi);
        }
    }

    // 1. Heavy crimson cast-iron rear yoke & twin outer uprights spanning ~36 px!
    let span_outer = seam_half + 11.2;
    let yoke_c = body - f * 4.2;
    draw_rotated_box(pb, yoke_c, s, span_outer + 0.6, 2.4, iron.hi, iron.mid, iron.lo, flash);
    // Decorative brass tension bolts & springs on the rear iron yoke.
    for &side in &[-1.0_f32, 1.0] {
        let up_a = yoke_c + s * (side * span_outer);
        let up_b = body + f * 3.4 + s * (side * span_outer);
        draw_tapered_capsule(pb, up_a, up_b, 2.5, 2.2, iron.hi, iron.mid, iron.lo, flash);
        // Brass top pressure screw on each upright!
        let screw = up_a - f * 2.4;
        draw_shaded_disc(pb, screw.x, screw.y, 2.0, brass.hi, brass.mid, brass.lo, flash);
        // Inner seam bracket holding the roller shaft cleanly at the central gap.
        let inner_b = yoke_c + s * (side * seam_half);
        draw_capsule(
            pb,
            inner_b,
            body + f * 1.2 + s * (side * seam_half),
            1.6,
            iron.hi,
            iron.mid,
            iron.lo,
            flash,
        );
    }

    // 2. Spinning brass side crank-handle on the outer +s housing!
    let crank_hub = body + f * 1.2 + s * (span_outer + 2.4);
    let crank_ang = anim * spin_rate * 0.45;
    let crank_arm = crank_hub + f * (crank_ang.cos() * 4.5) + Vec2::new(0.0, crank_ang.sin() * 3.6);
    draw_capsule(pb, crank_hub, crank_arm, 1.1, brass.hi, brass.mid, brass.lo, flash);
    draw_shaded_disc(pb, crank_arm.x, crank_arm.y, 1.8, (255, 248, 198), brass.hi, brass.lo, flash);

    // 3. Strike motion smear trails forward of BOTH rollers (leaving central seam open!).
    if let Pose::Strike(t) = pose {
        let smear_a = (1.0 - t * 0.55) * 0.78;
        for &side in &[-1.0_f32, 1.0] {
            let rc = body + f * 2.2 + s * (side * (seam_half + 6.0));
            for step in 1..=5 {
                let d = step as f32 * 2.2;
                let col = if step <= 2 { chrome } else { iron.hi };
                let a_pt = rc + f * d - s * 4.8;
                let b_pt = rc + f * d + s * 4.8;
                let segs = 8;
                for j in 0..=segs {
                    let u = j as f32 / segs as f32;
                    let q = a_pt + (b_pt - a_pt) * u;
                    blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, smear_a * (1.0 - step as f32 * 0.15));
                }
            }
        }
    }

    // 4. Twin ribbed gunmetal-and-chrome wringer rollers mounted end-to-end across `s`!
    let roll_phase = (anim * spin_rate).rem_euclid(TAU);
    for &side in &[-1.0_f32, 1.0] {
        let r_axis = s.rotate(side * skew);
        let r_mid = body + f * 1.4 + s * (side * (seam_half + 5.4));
        // Main heavy cylinder body (~10.5 px long × 6.2 px diameter).
        draw_rotated_box(pb, r_mid, r_axis, 5.0, 3.1, roller.hi, roller.mid, roller.lo, flash);
        // Three scrolling rubber wringer ribs across the cylinder so rotation is unmistakable!
        for rib in 0..3 {
            let ph = (roll_phase + rib as f32 * (TAU / 3.0)).rem_euclid(TAU);
            if ph < PI {
                let off = (ph / PI * 2.0 - 1.0) * 2.3;
                let rib_c = r_mid + f * off;
                let is_glint = (off + 0.6).abs() < 0.7;
                draw_rotated_box(
                    pb,
                    rib_c,
                    r_axis,
                    4.6,
                    0.55,
                    if is_glint { chrome } else { rubber.hi },
                    if is_glint { roller.hi } else { rubber.mid },
                    rubber.lo,
                    flash,
                );
            }
        }
        // Crisp chrome specular rim along the leading edge of each roller!
        let sp_a = r_mid + f * 2.0 - r_axis * 4.4;
        let sp_b = r_mid + f * 2.0 + r_axis * 4.4;
        pb.line(sp_a.x.round() as i32, sp_a.y.round() as i32, sp_b.x.round() as i32, sp_b.y.round() as i32, apply_flash(chrome, flash));
        // Dark rubber end-collars framing both ends of each roller.
        let inner_cap = r_mid - r_axis * 4.8;
        let outer_cap = r_mid + r_axis * 4.8;
        draw_shaded_disc(pb, inner_cap.x, inner_cap.y, 2.9, rubber.hi, rubber.mid, rubber.lo, flash);
        draw_shaded_disc(pb, outer_cap.x, outer_cap.y, 2.9, brass.hi, brass.mid, brass.lo, flash);
    }

    // 5. Tell sparks spraying off the high-RPM rollers & brass crank!
    if let Pose::Tell(t) = pose {
        for k in 0..6 {
            let ph = (t * 4.5 + k as f32 * 0.17).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let origin = body + f * 1.4 + s * (side * (seam_half + 5.0 + (k as f32) * 1.1));
            let sp = origin - f * (ph * 7.0) + Vec2::new(0.0, -ph * 5.0);
            blend_px(pb, sp.x.round() as i32, sp.y.round() as i32, brass.hi, 1.0 - ph * 0.5);
        }
    }

    // 6. Central iron bridge face: twin riveted glowing furnace eyes!
    let bridge = body - f * 3.8;
    draw_shaded_disc(pb, bridge.x, bridge.y, 3.8, iron.hi, iron.mid, iron.lo, flash);
    let expr = eye_expr_for(pose, anim, flash);
    let eye_c = match pose {
        Pose::Tell(_) => (255, 224, 88),
        Pose::Strike(_) => (255, 108, 86),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 236, 176),
    };
    draw_eye(pb, bridge + f * 1.0 + s * 1.9, 1.5, eye_c, INK, f, expr, flash);
    draw_eye(pb, bridge + f * 1.0 - s * 1.9, 1.5, eye_c, INK, f, expr, flash);

    // 7. THE LIMP MARIGOLD RUBBER GLOVE dangling in the central 12 px seam!
    let glove_reach = match pose {
        Pose::Tell(_) => 3.6 - coil * 2.2, // sucked taut back toward the nip!
        Pose::Strike(t) => 2.0 + (t * 28.0).sin() * 1.4, // whipping wildly in slipstream!
        Pose::Recover(_) => 7.2, // flopped limply way out forward!
        _ => 5.6,
    };
    let glove_sway = match pose {
        Pose::Tell(t) => tell_quiver(t) * 1.4,
        Pose::Strike(t) => (t * 34.0).sin() * 2.4,
        Pose::Recover(_) => (anim * 2.2).sin() * 0.6,
        _ => (anim * 4.4).sin() * 1.5,
    };
    let cuff = body + f * 1.2;
    let palm = body + f * glove_reach + s * glove_sway;
    draw_tapered_capsule(pb, cuff, palm, 2.2, 2.5, glove.hi, glove.mid, glove.lo, flash);
    // Four splayed yellow rubber fingers!
    let splay = 0.34 + coil * 0.24;
    for k in 0..4 {
        let ang = (k as f32 - 1.5) * splay + glove_sway * 0.08;
        let flen = if k == 1 || k == 2 { 4.2 } else { 3.3 };
        let ftip = palm + f.rotate(ang) * (flen + coil * 0.8);
        draw_tapered_capsule(pb, palm, ftip, 0.95, 0.55, glove.hi, glove.mid, glove.lo, flash);
    }

    // Recover: popped zig-zag brass tension spring + steam hiss!
    if let Pose::Recover(t) = pose {
        let sp_base = yoke_c + s * (span_outer * 0.5);
        for k in 0..4 {
            let p0 = sp_base + Vec2::new((k as f32) * 1.2 - 1.8, -(k as f32) * 2.2);
            let p1 = sp_base + Vec2::new(((k + 1) as f32 % 2.0) * 2.4 - 1.2, -((k + 1) as f32) * 2.2);
            pb.line(p0.x.round() as i32, p0.y.round() as i32, p1.x.round() as i32, p1.y.round() as i32, brass.hi);
        }
        draw_dizzy_stars(pb, bridge + Vec2::new(0.0, -9.5), 7.5, t);
        draw_recover_beacon(pb, bridge, t);
    }
    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, bridge + Vec2::new(0.0, -11.5), t);
    }
    draw_hurt_burst(pb, body, f, flash, brass.hi);
}

// ============================================================================
// 18. QUOIN KEY — "Locks the story in place." (Printworks 6:00)
//     26–28 px blued-steel printer's fork jaw driven by a serrated hardwood &
//     copper walking wedge on brass ratchet feet; fork clamps shut over a 55° cone
// ============================================================================

fn draw_quoin_key(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let steel = mb2_pal((236, 248, 255), (136, 168, 202), (52, 72, 98));
    let wood = mb2_pal((255, 206, 128), (206, 132, 62), (98, 54, 24));
    let brass = mb2_pal((255, 234, 124), (206, 150, 46), (98, 66, 18));
    let leg_col = mb2_pal((176, 192, 214), (104, 118, 140), (44, 52, 68));
    let cyan_glow: Rgb = (118, 246, 255);
    let steel_spec: Rgb = (255, 255, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 14.5, 6.8, 0.48);
        // Two sheared blued-steel fork prongs crossed in front of the chest!
        for &side in &[-1.0_f32, 1.0] {
            let drift = s * (side * (4.0 + burst * 5.2)) - Vec2::new(0.0, bounce * 0.7);
            let dir = f.rotate(side * (0.32 + burst * 0.38));
            draw_tapered_capsule(
                pb,
                center + drift - dir * 5.0,
                center + drift + dir * 10.0,
                2.2,
                0.9,
                steel.hi,
                steel.mid,
                steel.lo,
                flash,
            );
        }
        // Split serrated copper-wood wedge tumbled behind!
        let w_pos = center - f * (5.0 + burst * 5.0) - Vec2::new(0.0, bounce * 0.9);
        mb2_draw_poly_tri(
            pb,
            w_pos - f * 6.5,
            w_pos + f * 3.0 + s * 4.0,
            w_pos + f * 3.0 - s * 4.0,
            wood,
            flash,
        );
        // Popped brass hex bolt rolling aside.
        let bolt = center + f * (4.0 + burst * 4.0) + s * 3.0;
        draw_shaded_disc(pb, bolt.x, bolt.y, 2.2, brass.hi, brass.mid, brass.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, cyan_glow);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -20.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    // `jaw_ang`: half-angle of the steel fork prongs (opens to ~0.58 rad in Tell, snaps shut in Strike).
    let (lunge, lift, jaw_ang, wedge_shift, coil) = match pose {
        Pose::Entrance(t) => {
            let open = if t < 0.44 { 0.10 } else { 0.10 + ((t - 0.44) / 0.56) * 0.36 };
            (0.0, ent_drop, open, 0.0, 0.0)
        }
        Pose::Idle => (
            (anim * 2.4).sin() * 0.6,
            (anim * 2.4).cos() * 0.4,
            0.24 + (anim * 3.2).sin().abs() * 0.09,
            (anim * 2.4).sin() * 0.8,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 6.6).sin();
            (st * 1.8, -st.abs() * 1.6, 0.22 + st.abs() * 0.16, st * 1.4, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.4 * c + q * 0.7, -2.2 * c, 0.24 + 0.42 * c + q * 0.05, -2.8 * c, c)
        }
        Pose::Strike(t) => {
            let close = (t / 0.38).clamp(0.0, 1.0);
            let ease = close * close;
            (4.8 * ease, 0.5 * ease, 0.66 * (1.0 - ease) + 0.04, 3.2 * ease, 1.0)
        }
        Pose::Recover(t) => {
            let wob = (t * 16.0).sin() * (1.0 - t * 0.5) * 0.05;
            (2.2 * (1.0 - t * 0.3), 1.4, 0.03 + wob, 1.8, 0.0)
        }
        Pose::Retreat(t) => (-2.8, -(t * 8.0).sin().abs() * 1.2, 0.16 + (t * 8.0).sin().abs() * 0.14, -1.6, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.12);
    draw_shadow(pb, center.x + 1.0, center.y + 6.5, 13.5 * sh_s, 6.2 * sh_s, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), cyan_glow);
        }
    }

    // 1. Articulated brass-shod ratchet legs striding under the walking wedge!
    let stride = match pose {
        Pose::Approach => (anim * 6.6).sin() * 3.2,
        Pose::Retreat(t) => (t * 8.0).sin() * 2.4,
        _ => 0.0,
    };
    for &side in &[-1.0_f32, 1.0] {
        let hip = body - f * 5.8 + s * (side * 3.2);
        let knee = hip - f * (1.5 - side * stride * 0.4) + s * (side * 2.6) + Vec2::new(0.0, 1.2);
        let foot = hip + f * (side * stride - 1.2) + s * (side * 4.4)
            + Vec2::new(0.0, 4.4 - (side * stride).max(0.0) * 0.55);
        draw_capsule(pb, hip, knee, 1.2, leg_col.hi, leg_col.mid, leg_col.lo, flash);
        draw_capsule(pb, knee, foot, 1.0, leg_col.hi, leg_col.mid, leg_col.lo, flash);
        draw_shaded_disc(pb, foot.x, foot.y, 1.5, brass.hi, brass.mid, brass.lo, flash);
    }

    // 2. Strike inward jaw-clamp smear arcs!
    if let Pose::Strike(t) = pose {
        if t < 0.62 {
            let a = f.angle();
            let alpha = (1.0 - t / 0.62).clamp(0.0, 1.0) * 0.80;
            draw_smear_arc(pb, body, a - 0.66, a - jaw_ang, 4.5, 15.5, cyan_glow, steel_spec, alpha);
            draw_smear_arc(pb, body, a + jaw_ang, a + 0.66, 4.5, 15.5, cyan_glow, steel_spec, alpha);
        }
    }

    // 3. The walking hardwood & copper driving wedge (the vulnerable heel behind `-f`)!
    let wedge_base = body + f * (-1.6 + wedge_shift);
    let heel_tip = wedge_base - f * 10.2;
    let w_left = wedge_base + s * 5.0;
    let w_right = wedge_base - s * 5.0;
    mb2_draw_poly_tri(pb, heel_tip, w_left, w_right, wood, flash);
    // Serrated brass rack teeth running along the driving wedge spine!
    for k in 0..4 {
        let u = 0.22 + k as f32 * 0.20;
        let pt = heel_tip + (wedge_base - heel_tip) * u;
        let tooth_w = 1.1 + u * 2.1;
        draw_rotated_box(pb, pt, s, tooth_w, 0.75, brass.hi, brass.mid, brass.lo, flash);
    }

    // 4. Heavy blued-steel T-bar yoke & twin caliper fork prongs projecting forward (+12.6 px)!
    let yoke = body - f * 0.6;
    draw_rotated_box(pb, yoke, s, 6.0, 2.2, steel.hi, steel.mid, steel.lo, flash);
    for &side in &[-1.0_f32, 1.0] {
        let pdir = f.rotate(side * jaw_ang);
        let root = yoke + s * (side * 3.4);
        let elbow = root + pdir * 6.6 + pdir.perp() * (side * 1.1);
        let tip = root + pdir * 12.6 - pdir.perp() * (side * 0.8); // hooked inward caliper claw!
        draw_tapered_capsule(pb, root, elbow, 2.2, 1.85, steel.hi, steel.mid, steel.lo, flash);
        draw_tapered_capsule(pb, elbow, tip, 1.85, 0.9, steel_spec, steel.hi, steel.lo, flash);
        // Continuous bright honed-steel inner edge along each fork prong (survives 60% ambient).
        let in_a = root + pdir * 1.5 - pdir.perp() * (side * 0.9);
        let in_b = tip - pdir.perp() * (side * 0.3);
        pb.line(
            in_a.x.round() as i32,
            in_a.y.round() as i32,
            in_b.x.round() as i32,
            in_b.y.round() as i32,
            apply_flash(steel_spec, flash),
        );
        // Honed inner jaw glint racing down the prong during Tell!
        let gu = if let Pose::Tell(t) = pose {
            (t * 2.8).fract()
        } else {
            (anim * 0.9).fract()
        };
        let gp = root + (tip - root) * (0.25 + gu * 0.68);
        plot_px(pb, gp.x.round() as i32, gp.y.round() as i32, steel_spec);
        // Tell cyan tension sparks bridging between the wide-open fork tips!
        if matches!(pose, Pose::Tell(_)) && coil > 0.45 {
            let spark_pt = tip - pdir.perp() * (side * (1.2 + (anim * 18.0).sin().abs() * 1.8));
            plot_px(pb, spark_pt.x.round() as i32, spark_pt.y.round() as i32, cyan_glow);
        }
    }

    // 5. Gleaming brass hex lock-pivot & piercing glowing eye right on the steel bridge!
    let expr = eye_expr_for(pose, anim, flash);
    draw_shaded_disc(pb, yoke.x, yoke.y, 3.6, brass.hi, brass.mid, brass.lo, flash);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 224, 96),
        Pose::Strike(_) => (255, 114, 92),
        Pose::Recover(_) => (148, 255, 206),
        _ => (232, 248, 255),
    };
    draw_eye(pb, yoke + f * 0.8, 2.1, eye_col, INK, f, expr, flash);

    // Strike metallic clash flash at the closed fork tips!
    if let Pose::Strike(t) = pose {
        if (0.26..0.78).contains(&t) {
            mb2_star_flash(pb, body + f * 13.5, 5, steel_spec, cyan_glow);
        }
    }

    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, yoke + Vec2::new(0.0, -11.5), t);
    }
    if let Pose::Recover(t) = pose {
        // Expose the rear wooden wedge heel ("Step around the heel")!
        let heel_spot = heel_tip + f * 3.0;
        draw_dizzy_stars(pb, yoke + Vec2::new(0.0, -9.5), 6.8, t);
        draw_recover_beacon(pb, heel_spot, t);
    }
    draw_hurt_burst(pb, body, f, flash, cyan_glow);
}

// ============================================================================
// 19. CARBON TWIN — "Your duplicate stayed behind." (Printworks 9:00)
//     28–30 px offset ivory ledger sheet & midnight-indigo carbon duplicate
//     peeling dramatically apart from a crimson folded spine, with twin eyes!
// ============================================================================

fn draw_carbon_twin(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let paper = mb2_pal((252, 244, 218), (206, 194, 164), (122, 112, 90));
    let carbon = mb2_pal((136, 166, 232), (68, 86, 144), (28, 36, 68));
    let spine = mb2_pal((236, 86, 98), (168, 42, 56), (82, 18, 28));
    let ink_bar = mb2_pal((72, 82, 108), (40, 46, 64), (20, 24, 36));
    let ghost_cyan: Rgb = (108, 232, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 16.0, 7.2, 0.48);
        // Carbon sheet and ivory original torn cleanly apart along the crimson spine!
        let c_pos = center - s * (5.0 + burst * 5.5) - f * 2.0 - Vec2::new(0.0, bounce * 0.6);
        let p_pos = center + s * (5.0 + burst * 5.5) + f * 2.0 - Vec2::new(0.0, bounce);
        draw_rotated_box(pb, c_pos, f.rotate(-0.35), 9.0, 4.8, carbon.hi, carbon.mid, carbon.lo, flash);
        draw_rotated_box(pb, p_pos, f.rotate(0.32), 9.0, 4.8, paper.hi, paper.mid, paper.lo, flash);
        // Torn crimson spine strip fluttering between them.
        draw_capsule(
            pb,
            center - s * 4.0,
            center + s * 4.0 + f * 2.0,
            1.1,
            spine.hi,
            spine.mid,
            spine.lo,
            flash,
        );
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, ghost_cyan);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        (1.0 - t) * (1.0 - t) * -22.0
    } else {
        0.0
    };

    let (lunge, lift, peel, flap, coil) = match pose {
        Pose::Entrance(t) => (0.0, ent_drop, (1.0 - t) * 6.0 + 2.4, (t * 14.0).sin() * 0.24, 0.0),
        Pose::Idle => (
            (anim * 2.2).sin() * 0.7,
            (anim * 2.2).cos() * 0.6,
            2.6 + (anim * 2.8).sin() * 0.7,
            (anim * 3.0).sin() * 0.08,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 6.2).sin();
            (st * 2.2, -st.abs() * 1.4, 2.8 + st.abs() * 1.4, st * 0.14, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.2 * c + q * 0.7, -2.4 * c, 3.0 + c * 5.2 + q * 0.5, -0.22 * c, c)
        }
        Pose::Strike(t) => {
            let slap = (t / 0.28).clamp(0.0, 1.0);
            let ease = slap * slap;
            (5.4 * ease, 1.0 * ease, 8.0 * (1.0 - ease) + 0.8, 0.14 * (1.0 - ease), 1.0)
        }
        Pose::Recover(t) => (
            -1.8,
            1.2,
            4.8 + (t * 7.0).sin() * 0.8,
            0.28,
            0.0,
        ),
        Pose::Retreat(t) => (-2.8, -(t * 8.0).sin().abs() * 1.6, 3.4, (t * 12.0).sin() * 0.16, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.028).clamp(0.45, 1.12);
    draw_shadow(pb, center.x + 1.2, center.y + 7.0, 14.5 * sh_s, 6.4 * sh_s, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), ghost_cyan);
        }
    }

    let expr = eye_expr_for(pose, anim, flash);

    // 1. LOWER SHEET: Deep indigo CARBON DUPLICATE with crisp cyan-silver light rim!
    let carb_axis = f.rotate(-flap * 0.7);
    let carb_c = body - f * (peel * 0.45) - s * (peel * 0.88) + Vec2::new(0.0, 1.4);
    draw_rotated_box(pb, carb_c, carb_axis, 11.2, 5.8, carbon.hi, carbon.mid, carbon.lo, flash);
    // Bright cyan-silver outer rim along the peeled carbon sheet so it never merges with dark slabs.
    let carb_s = carb_axis.perp();
    let rim_col = apply_flash((156, 226, 255), flash);
    let c_bl = carb_c - carb_axis * 10.6 - carb_s * 5.2;
    let c_fl = carb_c + carb_axis * 10.6 - carb_s * 5.2;
    let c_br = carb_c - carb_axis * 10.6 + carb_s * 5.2;
    pb.line(c_bl.x.round() as i32, c_bl.y.round() as i32, c_fl.x.round() as i32, c_fl.y.round() as i32, rim_col);
    pb.line(c_bl.x.round() as i32, c_bl.y.round() as i32, c_br.x.round() as i32, c_br.y.round() as i32, rim_col);
    // Shimmering electric-cyan carbon ghost-text lines across the carbon sheet!
    for k in -1..=1 {
        let lp = carb_c + carb_s * (k as f32 * 2.5);
        let a = lp - carb_axis * 7.2;
        let b = lp + carb_axis * 6.5;
        pb.line(
            a.x.round() as i32,
            a.y.round() as i32,
            b.x.round() as i32,
            b.y.round() as i32,
            apply_flash(if k == 0 { ghost_cyan } else { carbon.hi }, flash),
        );
    }
    // Glowing cyan DUPLICATE EYE on the exposed carbon twin!
    let carb_eye_pos = carb_c + carb_axis * 4.2 - carb_s * 1.4;
    draw_eye(pb, carb_eye_pos, 1.7, ghost_cyan, INK, f, expr, flash);

    // 2. Strike page-slap cyan-ivory shockwave!
    if let Pose::Strike(t) = pose {
        if t < 0.65 {
            let ang = f.angle();
            draw_smear_arc(
                pb,
                body + f * 6.0,
                ang - 0.95,
                ang + 0.95,
                5.0,
                14.5,
                ghost_cyan,
                paper.hi,
                0.76 * (1.0 - t / 0.65),
            );
        }
    }

    // 3. UPPER SHEET: Warm ivory ledger original peeling high upward/across!
    let top_axis = f.rotate(flap);
    let top_s = top_axis.perp();
    let top_c = body + f * (peel * 0.35) + s * (peel * 0.82) + Vec2::new(0.0, -1.6 - coil * 2.6);
    draw_rotated_box(pb, top_c, top_axis, 11.2, 5.8, paper.hi, paper.mid, paper.lo, flash);
    // Crisp dark printed ledger rows + vermilion margin rule!
    let margin_a = top_c + top_axis * 5.8 - top_s * 4.6;
    let margin_b = top_c + top_axis * 5.8 + top_s * 4.6;
    pb.line(
        margin_a.x.round() as i32,
        margin_a.y.round() as i32,
        margin_b.x.round() as i32,
        margin_b.y.round() as i32,
        apply_flash(spine.hi, flash),
    );
    for k in -1..=1 {
        let lp = top_c - top_axis * 1.2 + top_s * (k as f32 * 2.6);
        draw_rotated_box(pb, lp, top_axis, 5.6, 0.45, ink_bar.hi, ink_bar.mid, ink_bar.lo, flash);
    }
    // Dog-eared curling rear corner flapping upward as the page peels back!
    let corner_base = top_c - top_axis * 9.5 + top_s * 3.8;
    let corner_tip = corner_base - top_axis * (2.6 + coil * 2.2) + top_s * (1.6 + coil * 1.8)
        + Vec2::new(0.0, -1.4 - coil * 3.0);
    mb2_draw_poly_tri(
        pb,
        corner_base + top_axis * 2.4,
        corner_base - top_s * 2.8,
        corner_tip,
        paper,
        flash,
    );
    // Sharp ink-black ORIGINAL EYE on the ivory top sheet!
    let top_eye_col = match pose {
        Pose::Tell(_) => (255, 236, 128),
        Pose::Strike(_) => (255, 134, 122),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 255, 255),
    };
    draw_eye(pb, top_c + top_axis * 2.4 + top_s * 1.2, 1.8, top_eye_col, INK, f, expr, flash);

    // 4. Heavy crimson-bound FOLDED FRONT SPINE joining both sheets (where the transfer lays)!
    let spine_top = top_c + top_axis * 10.6;
    let spine_bot = carb_c + carb_axis * 10.6;
    draw_capsule(pb, spine_bot, spine_top, 1.8, spine.hi, spine.mid, spine.lo, flash);
    // Cyan carbon-transfer sparks fizzing between the peeling sheets during Tell!
    if let Pose::Tell(t) = pose {
        for k in 0..5 {
            let ph = (t * 4.2 + k as f32 * 0.2).fract();
            let q = carb_c + (top_c - carb_c) * ph + f * ((k as f32 - 2.0) * 3.4);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, ghost_cyan, 0.92);
        }
        mb2_tell_pip(pb, top_c + Vec2::new(0.0, -11.5), t);
    }

    if let Pose::Recover(t) = pose {
        // Expose the ruffled short edge!
        let short_edge = body - f * 10.5;
        draw_dizzy_stars(pb, top_c + Vec2::new(0.0, -8.5), 7.2, t);
        draw_recover_beacon(pb, short_edge, t);
    }
    draw_hurt_burst(pb, body, f, flash, ghost_cyan);
}

// ============================================================================
// 20. LATE PLATE — "That's all the room we have tonight." (Printworks 12:00)
//     32–34 px heavy curved zinc-steel rotary printing plate with royal press-
//     violet inked inner face, real jagged torn registration notch along
//     RING_GAP_TURN, and electric cyan crop-marks!
// ============================================================================

fn draw_late_plate(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let gap = f.rotate(RING_GAP_TURN);
    let steel = mb2_pal((240, 248, 255), (154, 170, 196), (62, 72, 92));
    let ink_pal = mb2_pal((222, 164, 255), (144, 82, 212), (68, 32, 112));
    let brass = mb2_pal((255, 228, 118), (202, 146, 44), (96, 62, 18));
    let reg_cyan: Rgb = (112, 246, 255);
    let tear_spec: Rgb = (242, 252, 255);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 6.0, 17.0, 7.6, 0.50);
        // Spilled pool of royal violet press ink!
        let pr = 8.0 + burst * 6.5;
        draw_shaded_ellipse(
            pb,
            center.x,
            center.y + 3.0,
            pr,
            pr * 0.54,
            ink_pal.hi,
            ink_pal.mid,
            ink_pal.lo,
            flash,
        );
        // Snapped curved zinc plate halves flying apart from the torn registration notch!
        for &side in &[-1.0_f32, 1.0] {
            let drift = s * (side * (5.5 + burst * 6.5)) - Vec2::new(0.0, bounce * 0.75);
            let dir = f.rotate(side * (0.55 + burst * 0.35));
            draw_tapered_capsule(
                pb,
                center + drift - dir.perp() * 7.5,
                center + drift + dir.perp() * 7.5,
                2.8,
                2.2,
                steel.hi,
                steel.mid,
                steel.lo,
                flash,
            );
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, ink_pal.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -24.0
        } else {
            let u = (t - 0.44) / 0.56;
            (u * PI * 2.0).sin() * (1.0 - u) * 2.6
        }
    } else {
        0.0
    };

    // Plate curvature (`r_out`, `half_span`) curls tighter through Tell & clamps in Strike!
    let (lunge, lift, rock, r_out, half_span, coil) = match pose {
        Pose::Entrance(_) => (0.0, ent_drop, 0.0, 14.8, 1.72, 0.0),
        Pose::Idle => (
            (anim * 2.0).sin() * 0.6,
            (anim * 2.0).cos() * 0.5,
            (anim * 2.2).sin() * 0.10,
            14.8,
            1.72,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 5.8).sin();
            (st * 1.6, -st.abs() * 1.4, st * 0.24, 14.8, 1.74, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            (-3.2 * c + q * 0.8, -2.2 * c, q * 0.06, 15.6 - c * 2.2, 1.66 + c * 0.36, c)
        }
        Pose::Strike(t) => {
            let clamp = (t * PI).sin();
            (4.0 * clamp, 0.8, (t * 42.0).sin() * 0.05, 13.0 - clamp * 1.6, 2.08, 1.0)
        }
        Pose::Recover(t) => (
            -2.0,
            1.8,
            0.34 * (1.0 - t * 0.25),
            16.0,
            1.42, // sprung awkwardly open & flat!
            0.0,
        ),
        Pose::Retreat(t) => (-2.6, -(t * 8.0).sin().abs() * 1.4, (t * 8.0).sin() * 0.16, 15.0, 1.68, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.028).clamp(0.45, 1.14);
    draw_shadow(pb, center.x + 1.2, center.y + 7.2, 16.0 * sh_s, 6.8 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), ink_pal.hi);
        }
    }

    let plate_f = f.rotate(rock);
    let hub = body - plate_f * 4.4;
    let r_in = (r_out - 5.4).max(5.5);

    // 1. Strike inward violet-white ink-spray smear arcs (skipping the torn notch!).
    if let Pose::Strike(t) = pose {
        let g_ang = gap.angle();
        draw_smear_arc(
            pb,
            hub,
            g_ang + 0.55,
            g_ang + TAU - 0.55,
            r_in - 2.0,
            r_out + 2.6,
            ink_pal.hi,
            reg_cyan,
            0.70 * (1.0 - t * 0.5),
        );
    }

    // 2. Main solid curved zinc-and-violet printing plate (~32 px tip-to-tip) with
    //    REAL JAGGED TORN REGISTRATION NOTCH along `gap`!
    let tear_c = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
        (255, 242, 148)
    } else {
        tear_spec
    };
    mb2_draw_curved_plate(
        pb,
        hub,
        plate_f,
        gap,
        r_in,
        r_out,
        half_span,
        steel,
        ink_pal,
        tear_c,
        flash,
    );

    // 3. Brass registration alignment pins at both curved plate tips!
    for &side in &[-1.0_f32, 1.0] {
        let tip_dir = plate_f.rotate(side * (half_span - 0.08));
        let pin = hub + tip_dir * ((r_in + r_out) * 0.5);
        draw_shaded_disc(pb, pin.x, pin.y, 1.8, brass.hi, brass.mid, brass.lo, flash);
    }

    // 4. Electric cyan registration crop-mark reticle (`⊕`) & optical pressman eye!
    let eye_hub = body + plate_f * 1.4;
    let cross_r = 4.6 + coil * 1.4;
    let ca = eye_hub - f * cross_r;
    let cb = eye_hub + f * cross_r;
    let cc = eye_hub - s * cross_r;
    let cd = eye_hub + s * cross_r;
    pb.line(ca.x.round() as i32, ca.y.round() as i32, cb.x.round() as i32, cb.y.round() as i32, reg_cyan);
    pb.line(cc.x.round() as i32, cc.y.round() as i32, cd.x.round() as i32, cd.y.round() as i32, reg_cyan);
    draw_shaded_disc(pb, eye_hub.x, eye_hub.y, 4.0, steel.hi, steel.mid, steel.lo, flash);
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 224, 96),
        Pose::Strike(_) => ink_pal.hi,
        Pose::Recover(_) => (148, 255, 206),
        _ => reg_cyan,
    };
    draw_eye(pb, eye_hub, 2.4, eye_col, INK, f, expr, flash);

    // 5. Torn notch beacon rays & flying ink droplets during Tell!
    let notch_pt = hub + gap * ((r_in + r_out) * 0.5);
    if let Pose::Tell(t) = pose {
        for &side in &[-1.0_f32, 1.0] {
            let e_dir = gap.rotate(side * 0.36);
            let p0 = hub + e_dir * r_out;
            let p1 = p0 + e_dir * (3.5 + t * 4.5);
            pb.line(p0.x.round() as i32, p0.y.round() as i32, p1.x.round() as i32, p1.y.round() as i32, reg_cyan);
        }
        mb2_tell_pip(pb, body + Vec2::new(0.0, -14.5), t);
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, body + Vec2::new(0.0, -12.5), 7.6, t);
        draw_recover_beacon(pb, notch_pt, t);
    }
    draw_hurt_burst(pb, body, f, flash, reg_cyan);
}

// ============================================================================
// 21. TARPAULIN PUP — "Contents may have shifted." (Flatbed 3:00)
//     22–25 px (32 px unfurled) army-olive folded canvas tarp hound with four
//     brass grommet-ring paws, crimson bungee collar, floppy ears & wagging tail
// ============================================================================

fn draw_tarpaulin_pup(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let tarp = mb2_pal((192, 224, 128), (118, 154, 72), (52, 76, 34));
    let khaki = mb2_pal((246, 228, 168), (192, 166, 102), (104, 86, 48));
    let brass = mb2_pal((255, 234, 120), (206, 150, 44), (98, 64, 18));
    let bungee = mb2_pal((252, 116, 102), (218, 56, 48), (116, 26, 26));

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.0;
        draw_shadow(pb, center.x + 1.0, center.y + 5.0, 15.0, 6.8, 0.48);
        // Rumpled collapsed olive tarp heap!
        draw_shaded_ellipse(
            pb,
            center.x - 1.5,
            center.y + 2.0 - bounce * 0.4,
            10.5,
            5.2,
            tarp.hi,
            tarp.mid,
            tarp.lo,
            flash,
        );
        draw_rotated_box(
            pb,
            center + f * 3.0,
            f.rotate(0.3),
            6.0,
            2.8,
            khaki.hi,
            khaki.mid,
            khaki.lo,
            flash,
        );
        // Four popped brass grommet rings scattered outward!
        for k in 0..4 {
            let a = k as f32 * FRAC_PI_2 + 0.55;
            let gp = center + Vec2::from_angle(a) * (6.5 + burst * 5.0);
            draw_shaded_disc(pb, gp.x, gp.y, 1.8, brass.hi, brass.mid, brass.lo, flash);
            plot_px(pb, gp.x.round() as i32, gp.y.round() as i32, INK);
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, khaki.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -20.0
        } else {
            let u = (t - 0.44) / 0.56;
            -(u * PI).sin() * (1.0 - u) * 5.0
        }
    } else {
        0.0
    };

    let (lunge, lift, unfurl, tail_wag, ear_flap, coil) = match pose {
        Pose::Entrance(_) => (0.0, ent_drop, 0.0, (anim * 12.0).sin() * 3.4, 0.4, 0.0),
        Pose::Idle => (
            (anim * 3.0).sin() * 0.7,
            -(anim * 3.0).sin().abs() * 0.9,
            0.0,
            (anim * 9.5).sin() * 3.2,
            (anim * 5.0).sin() * 0.25,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 7.8).sin();
            (st * 2.2, -st.abs() * 2.6, 0.8 + st.abs() * 1.0, (anim * 12.0).sin() * 3.8, -0.35 + st * 0.2, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Play-bow crouch: haunches high, front hem hunkered & puffing with wind!
            (-3.2 * c + q * 0.6, 1.4 * c, c * 4.4, (t * 28.0).sin() * 4.2, 0.55 * c, c)
        }
        Pose::Strike(t) => {
            let snap = (t / 0.26).clamp(0.0, 1.0);
            let billow = (t * 22.0).sin() * 1.1;
            (4.8 * snap, -0.6 * snap, 4.4 + snap * 7.2 + billow, (t * 20.0).sin() * 2.0, -0.55, 1.0)
        }
        Pose::Recover(t) => (
            1.4,
            2.0,
            6.2 * (1.0 - t * 0.35),
            (t * 5.0).sin() * 1.0,
            0.85,
            0.0,
        ),
        Pose::Retreat(t) => (-2.6, -(t * 9.0).sin().abs() * 1.8, 0.0, -(t * 10.0).sin() * 2.5, 0.2, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.032).clamp(0.45, 1.14);
    draw_shadow(pb, center.x + 1.0 + unfurl * f.x * 0.25, center.y + 6.5, (12.5 + unfurl * 0.45) * sh_s, 5.6 * sh_s, 0.52);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), tarp.hi);
        }
    }

    // 1. Wagging frayed rope-and-tarp tail behind `-f`!
    let tail_base = body - f * 7.2 + Vec2::new(0.0, -coil * 2.6);
    let tail_tip = tail_base - f * 5.6 + s * tail_wag + Vec2::new(0.0, -2.4 - coil * 2.2);
    draw_tapered_capsule(pb, tail_base, tail_tip, 1.6, 0.7, tarp.hi, tarp.mid, tarp.lo, flash);
    draw_shaded_disc(pb, tail_tip.x, tail_tip.y, 1.1, khaki.hi, khaki.mid, khaki.lo, flash);

    // 2. FOUR brass-ring grommet paws galloping under the folded tarp!
    let gallop = match pose {
        Pose::Approach => (anim * 7.8).sin() * 3.0,
        Pose::Retreat(t) => (t * 9.0).sin() * 2.2,
        _ => 0.0,
    };
    let paw_specs = [
        (-5.6_f32, -4.4_f32, -gallop, true),
        (-5.6, 4.4, -gallop, true),
        (5.0, -4.4, gallop, false),
        (5.0, 4.4, gallop, false),
    ];
    for &(ax, sx, step, is_rear) in &paw_specs {
        let rear_kick = if is_rear { -coil * 2.6 } else { coil * 0.8 };
        let recover_air = if matches!(pose, Pose::Recover(_)) && sx < 0.0 { -3.2 } else { 0.0 };
        let paw = body
            + f * (ax + step * 0.8)
            + s * sx
            + Vec2::new(0.0, 4.6 - step.max(0.0) * 0.55 + rear_kick + recover_air);
        draw_shaded_disc(pb, paw.x, paw.y, 2.2, brass.hi, brass.mid, brass.lo, flash);
        // Dark hollow eyelet center so every foot reads as a real tarp grommet!
        plot_px(pb, paw.x.round() as i32, paw.y.round() as i32, INK);
    }

    // 3. Base folded army-olive canvas body with crisp stitched cross-fold seams & light rim!
    let main_c = body - f * 1.4;
    draw_rotated_box(pb, main_c, f, 7.2, 5.2, tarp.hi, tarp.mid, tarp.lo, flash);
    let hem_a = main_c - f * 6.4 - s * 4.6;
    let hem_b = main_c + f * 6.4 - s * 4.6;
    pb.line(
        hem_a.x.round() as i32,
        hem_a.y.round() as i32,
        hem_b.x.round() as i32,
        hem_b.y.round() as i32,
        apply_flash((228, 246, 178), flash),
    );
    for k in -1..=1 {
        let seam = main_c + f * (k as f32 * 3.0);
        let a = seam - s * 4.6;
        let b = seam + s * 4.6;
        pb.line(a.x.round() as i32, a.y.round() as i32, b.x.round() as i32, b.y.round() as i32, apply_flash(tarp.lo, flash));
    }

    // 4. Crimson elastic tie-down bungee collar with gleaming silver S-hook clasp!
    let collar = body + f * 3.4;
    draw_rotated_box(pb, collar, s, 5.4, 1.0, bungee.hi, bungee.mid, bungee.lo, flash);
    let clasp = collar + f * 0.8;
    draw_shaded_disc(pb, clasp.x, clasp.y, 1.3, (255, 255, 255), brass.hi, brass.lo, flash);

    // 5. Strike billowing wind-smear lines as the canvas sheet snaps out!
    if let Pose::Strike(t) = pose {
        let ang = f.angle();
        draw_smear_arc(
            pb,
            body + f * 3.0,
            ang - 0.72,
            ang + 0.72,
            6.5,
            15.5,
            khaki.hi,
            (255, 255, 240),
            0.72 * (1.0 - t * 0.5),
        );
    }

    // 6. Upper khaki duck-cloth flap that puffs up in Tell and UNFURLS way forward in Strike!
    let flap_len = 5.2 + unfurl * 0.58;
    let flap_c = body + f * (2.2 + unfurl * 0.58) + Vec2::new(0.0, -1.2 - coil * 1.8);
    draw_rotated_box(pb, flap_c, f, flap_len, 4.4 + coil * 0.5, khaki.hi, khaki.mid, khaki.lo, flash);
    // Reinforced brass-grommeted leading hem edge!
    let hem_c = flap_c + f * (flap_len - 0.6);
    draw_rotated_box(pb, hem_c, s, 4.6 + coil * 0.4, 0.9, brass.hi, brass.mid, brass.lo, flash);
    for &side in &[-1.0_f32, 1.0] {
        let ey = hem_c + s * (side * 3.4);
        draw_shaded_disc(pb, ey.x, ey.y, 1.4, brass.hi, brass.mid, brass.lo, flash);
        plot_px(pb, ey.x.round() as i32, ey.y.round() as i32, INK);
    }

    // 7. Two big floppy triangular tarp-corner ears flapping in the highway slipstream!
    for &side in &[-1.0_f32, 1.0] {
        let e_root = body + f * 4.2 + s * (side * 3.4) + Vec2::new(0.0, -1.5);
        let e_back = body + f * 1.2 + s * (side * 4.8) + Vec2::new(0.0, -1.0);
        let e_tip = body
            + f * (2.6 - ear_flap * 2.5)
            + s * (side * (8.4 + coil * 1.4))
            + Vec2::new(0.0, -2.0 + ear_flap * 2.4 * side);
        mb2_draw_poly_tri(pb, e_root, e_back, e_tip, tarp, flash);
        // Tiny brass eyelet at each ear tip!
        plot_px(pb, e_tip.x.round() as i32, e_tip.y.round() as i32, brass.hi);
    }

    // 8. Eager bright amber pup eyes & dark rubber nose peering under the folded brim!
    let expr = eye_expr_for(pose, anim, flash);
    let head_c = body + f * 5.2 + Vec2::new(0.0, -1.0);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 224, 88),
        Pose::Strike(_) => (255, 134, 92),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 246, 202),
    };
    draw_eye(pb, head_c + s * 2.1, 1.5, eye_col, INK, f, expr, flash);
    draw_eye(pb, head_c - s * 2.1, 1.5, eye_col, INK, f, expr, flash);
    let nose = head_c + f * 2.6;
    draw_shaded_disc(pb, nose.x, nose.y, 1.3, (148, 142, 134), (58, 54, 52), INK, flash);
    plot_px(pb, nose.x.round() as i32 - 1, nose.y.round() as i32 - 1, apply_flash((248, 246, 238), flash));

    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, body + Vec2::new(0.0, -11.5), t);
    }
    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, body + Vec2::new(0.0, -10.5), 6.8, t);
        draw_recover_beacon(pb, body - f * 5.5, t);
    }
    draw_hurt_burst(pb, body, f, flash, khaki.hi);
}

// ============================================================================
// 22. AXLE NUT — "Last seen under the truck." (Flatbed 6:00)
//     26 px razor-sharp machined gunmetal-steel HEXAGONAL truck axle nut with
//     a deep pitch-black threaded central bore, glowing inner eyes & stud feet!
// ============================================================================

fn draw_axle_nut(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let steel = mb2_pal((234, 244, 255), (148, 164, 186), (62, 72, 92));
    let grease = mb2_pal((246, 188, 78), (176, 106, 36), (78, 44, 18));
    let stud = mb2_pal((204, 214, 232), (124, 134, 152), (52, 58, 72));
    let steel_spec: Rgb = (255, 255, 255);
    let spark_gold: Rgb = (255, 214, 96);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 15.5, 7.2, 0.52);
        // Dark axle-grease oil slick under the sheared nut!
        draw_shaded_ellipse(
            pb,
            center.x,
            center.y + 3.0,
            9.5 + burst * 4.5,
            5.0 + burst * 2.0,
            (58, 52, 46),
            (26, 24, 24),
            INK,
            0.0,
        );
        // Two sheared steel hex halves clattering apart left & right!
        for &side in &[-1.0_f32, 1.0] {
            let half_c = center + s * (side * (4.5 + burst * 5.8)) - Vec2::new(0.0, bounce * 0.75);
            draw_rotated_box(
                pb,
                half_c,
                f.rotate(side * 0.35),
                6.5,
                3.4,
                steel.hi,
                steel.mid,
                steel.lo,
                flash,
            );
        }
        // Bent brass cotter pin sprung loose beside the chest!
        let pin = center + f * (5.0 + burst * 4.5) - Vec2::new(0.0, bounce * 1.1);
        draw_capsule(pb, pin - s * 3.5, pin + s * 3.5, 1.1, grease.hi, grease.mid, grease.lo, flash);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, spark_gold);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.44 {
            let u = t / 0.44;
            (1.0 - u * u) * -22.0
        } else {
            let u = (t - 0.44) / 0.56;
            -(u * PI * 2.0).sin().abs() * (1.0 - u) * 4.0
        }
    } else {
        0.0
    };

    let (lunge, lift, spin, feet_tuck, coil) = match pose {
        Pose::Entrance(t) => (0.0, ent_drop, t * 18.0, 1.0 - t, 0.0),
        Pose::Idle => (
            0.0,
            (anim * 2.6).sin() * 0.6,
            (anim * 2.0).sin() * 0.14,
            0.0,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 6.2).sin();
            (st * 1.4, -st.abs() * 1.6, st * 0.28, 0.0, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Tucks feet, rears back, revs up spinning violently!
            (-4.6 * c + q * 0.9, -2.0 * c, t * (8.0 + c * 22.0), c, c)
        }
        Pose::Strike(t) => {
            let thrust = (t * PI).sin();
            (6.0 * thrust, 0.4, t * 44.0, 1.0, 1.0)
        }
        Pose::Recover(t) => (
            2.4 * (1.0 - t * 0.4),
            2.2,
            0.42 + (t * 10.0).sin() * 0.15,
            0.2,
            0.0,
        ),
        Pose::Retreat(t) => (-2.6, -(t * 8.0).sin().abs() * 1.2, -t * 5.0, 0.0, 0.0),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.15);
    draw_shadow(pb, center.x + 1.2, center.y + 8.5, 13.5 * sh_s, 6.2 * sh_s, 0.56);

    if let Pose::Entrance(t) = pose {
        if t >= 0.42 {
            draw_entrance_ring(pb, center, ((t - 0.42) / 0.58).clamp(0.0, 1.0), spark_gold);
        }
    }

    // 1. Chunky splayed steel axle-stud feet (retract into the nut as it revs up to roll!).
    let leg_vis = 1.0 - feet_tuck * 0.85;
    if leg_vis > 0.15 {
        let stride = match pose {
            Pose::Approach => (anim * 6.2).sin() * 3.0,
            Pose::Retreat(t) => (t * 8.0).sin() * 2.2,
            _ => 0.0,
        };
        for &side in &[-1.0_f32, 1.0] {
            let hip = body + s * (side * 5.2) + Vec2::new(0.0, 4.0);
            let foot = body
                + f * (side * stride)
                + s * (side * 6.6)
                + Vec2::new(0.0, 4.0 + 5.8 * leg_vis - (side * stride).max(0.0) * 0.5);
            draw_tapered_capsule(pb, hip, foot, 1.9, 1.4, stud.hi, stud.mid, stud.lo, flash);
            draw_shaded_ellipse(pb, foot.x, foot.y, 2.6, 1.5, grease.hi, grease.mid, grease.lo, flash);
        }
    }

    // 2. Strike high-RPM circular axle-spin blur arcs + impact flash!
    if let Pose::Strike(t) = pose {
        let ang = f.angle();
        draw_smear_arc(
            pb,
            body,
            ang - 2.3,
            ang + 2.3,
            10.2,
            15.6,
            spark_gold,
            steel_spec,
            0.76 * (1.0 - t * 0.4),
        );
        mb2_star_flash(pb, body + f * 12.0, 4, steel_spec, spark_gold);
    }

    // 3. EXACT RAZOR-CRISP SIX-SIDED MACHINED HEXAGONAL NUT BODY (~26 px across)!
    let hex_h = 10.6 + coil * 0.45;
    mb2_draw_hex(pb, body, hex_h, spin, steel, steel_spec, flash);

    // Subtle truck-grease & rust streak on the lower right facet for industrial character.
    let rust_p = body + Vec2::from_angle(spin + 0.7) * 7.8;
    draw_shaded_disc(pb, rust_p.x, rust_p.y, 2.0, grease.hi, grease.mid, grease.lo, flash);

    // 4. MASSIVE PITCH-BLACK CENTRAL THREADED BORE HOLE ringed by a bright machined collar!
    draw_shaded_disc(pb, body.x, body.y, 6.4, steel_spec, steel.hi, steel.mid, flash);
    draw_shaded_disc(pb, body.x, body.y, 5.2, (18, 20, 28), (10, 10, 16), (6, 6, 10), 0.0);
    // Concentric machined steel screw-thread glints curving inside the black bore!
    for k in 0..8 {
        let a = spin * 1.4 + k as f32 * (TAU / 8.0);
        let p_th = body + Vec2::from_angle(a) * (4.4 - coil * 0.4);
        if k % 2 == 0 {
            blend_px(pb, p_th.x.round() as i32, p_th.y.round() as i32, steel.hi, 0.85);
        }
    }

    // 5. Twin fierce glowing furnace-amber eyes peering OUT OF THE BLACK BORE HOLE!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 232, 92),
        Pose::Strike(_) => (255, 112, 74),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 198, 72),
    };
    draw_eye(pb, body + f * 1.4 + s * 2.1, 1.5, eye_col, INK, f, expr, flash);
    draw_eye(pb, body + f * 1.4 - s * 2.1, 1.5, eye_col, INK, f, expr, flash);

    // 6. Tell grinding friction spark shower spraying backward off the spinning flats!
    if let Pose::Tell(t) = pose {
        for k in 0..6 {
            let ph = (t * 5.0 + k as f32 * 0.17).fract();
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let sp = body - f * (5.5 + ph * 10.5)
                + s * (side * (4.5 + ph * 5.0))
                + Vec2::new(0.0, 3.0 - ph * 6.5);
            blend_px(
                pb,
                sp.x.round() as i32,
                sp.y.round() as i32,
                if k % 2 == 0 { spark_gold } else { steel_spec },
                1.0 - ph * 0.55,
            );
        }
        mb2_tell_pip(pb, body + Vec2::new(0.0, -14.5), t);
    }

    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, body + Vec2::new(0.0, -12.5), 7.2, t);
        draw_recover_beacon(pb, body, t);
    }
    draw_hurt_burst(pb, body, f, flash, spark_gold);
}

// ============================================================================
// 23. CHAIN DOG — "Stays where it's tied. Mostly." (Flatbed 9:00)
//     32 px forged-iron chain-link hound with spiked collar, snapping fangs,
//     and a blazing highway-orange triangular ratchet load-binder swinging 115°!
// ============================================================================

fn draw_chain_dog(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let iron = mb2_pal((238, 246, 255), (154, 168, 190), (66, 76, 96));
    let iron_dark = mb2_pal((192, 206, 226), (112, 124, 146), (48, 56, 72));
    let binder = mb2_pal((255, 192, 88), (238, 98, 30), (132, 42, 14));
    let hide = mb2_pal((214, 180, 142), (144, 112, 82), (66, 48, 34));
    let collar = mb2_pal((252, 102, 92), (206, 48, 44), (102, 20, 22));
    let brass = mb2_pal((255, 232, 124), (208, 152, 48), (102, 66, 20));

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 5.5, 16.5, 7.2, 0.50);
        // Scattered rattling forged iron chain links!
        for k in 0..7 {
            let a = k as f32 * (TAU / 7.0) + 0.3;
            let rad = 5.5 + burst * 5.8;
            let p0 = center + Vec2::from_angle(a) * rad - Vec2::new(0.0, bounce * 0.6);
            let p1 = p0 + Vec2::from_angle(a + 0.9) * 3.8;
            mb2_draw_ring_link(pb, p0, p1, 1.7, if k % 2 == 0 { iron } else { iron_dark }, flash);
        }
        // Broken orange triangular load binder clattered flat beside the chest!
        let bp = center - f * (5.5 + burst * 5.5) - Vec2::new(0.0, bounce * 0.9);
        for k in 0..3 {
            let c0 = bp + f.rotate(k as f32 * (TAU / 3.0) + 0.4) * 4.6;
            let c1 = bp + f.rotate(((k + 1) % 3) as f32 * (TAU / 3.0) + 0.4) * 4.6;
            draw_capsule(pb, c0, c1, 1.5, binder.hi, binder.mid, binder.lo, flash);
        }
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, binder.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        if t < 0.42 {
            let u = t / 0.42;
            (1.0 - u * u) * -20.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    // Exact 115° swing matching `MinibossKind::ChainDog` (`half_sweep = 57.5°`).
    let half_sweep = 57.5_f32.to_radians();
    let (lunge, lift, bind_ang, bind_reach, jaw_open, coil) = match pose {
        Pose::Entrance(t) => (
            0.0,
            ent_drop,
            -PI + (1.0 - t) * 0.6,
            11.5,
            1.4,
            0.0,
        ),
        Pose::Idle => (
            (anim * 2.8).sin() * 0.8,
            (anim * 2.8).cos() * 0.4,
            -PI + (anim * 2.6).sin() * 0.28,
            11.5,
            0.8 + (anim * 4.2).sin().abs() * 0.8,
            0.0,
        ),
        Pose::Approach => {
            let st = (anim * 7.2).sin();
            (st * 2.0, -st.abs() * 1.8, -PI + st * 0.36, 12.0, 1.4, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Hauls the blazing orange binder around to `-half_sweep` ready to whip!
            let k = c.min(1.0);
            (
                -3.0 * c + q * 0.7,
                -1.4 * c,
                -PI + (PI - half_sweep) * k + q * 0.05,
                11.8 + 2.8 * k,
                2.4 * c + 0.8,
                c,
            )
        }
        Pose::Strike(t) => (
            3.6 * (t * PI).sin(),
            0.4,
            -half_sweep + 2.0 * half_sweep * t,
            15.0,
            2.8,
            1.0,
        ),
        Pose::Recover(t) => (
            -1.4,
            1.6,
            half_sweep + 0.14 * (1.0 - t * 0.3),
            13.2,
            2.2,
            0.0,
        ),
        Pose::Retreat(t) => (
            -2.8,
            -(t * 8.0).sin().abs() * 1.4,
            -PI + (1.0 - t) * 0.5,
            11.2,
            1.0,
            0.0,
        ),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.03).clamp(0.45, 1.14);
    draw_shadow(pb, center.x + 1.0, center.y + 6.8, 15.5 * sh_s, 6.4 * sh_s, 0.54);

    if let Pose::Entrance(t) = pose {
        if t >= 0.40 {
            draw_entrance_ring(pb, center, ((t - 0.40) / 0.60).clamp(0.0, 1.0), binder.hi);
        }
    }

    let anchor = body - f * 2.0;
    let drag = f.rotate(bind_ang);
    let at_binder = anchor + drag * bind_reach;

    // 1. Strike fiery orange-and-white 115° chain-whip smear arc!
    if let Pose::Strike(t) = pose {
        let base_a = f.angle();
        let trail_a = (bind_ang - 0.72).max(-half_sweep);
        draw_smear_arc(
            pb,
            anchor,
            base_a + trail_a,
            base_a + bind_ang,
            7.5,
            bind_reach + 4.5,
            binder.mid,
            (255, 246, 198),
            0.80 * (1.0 - t * 0.35),
        );
    }

    // 2. Whipping heavy iron tether links + BLAZING SAFETY-ORANGE TRIANGULAR BINDER!
    let tether_links = 4;
    for k in 0..tether_links {
        let u0 = (k as f32 + 0.15) / tether_links as f32;
        let u1 = (k as f32 + 0.85) / tether_links as f32;
        let sag = if matches!(pose, Pose::Tell(_) | Pose::Strike(_)) {
            0.0
        } else {
            (u0 * PI).sin() * 2.2
        };
        let p0 = anchor + drag * (bind_reach * u0) + Vec2::new(0.0, sag);
        let p1 = anchor + drag * (bind_reach * u1) + Vec2::new(0.0, sag);
        mb2_draw_ring_link(pb, p0, p1, 1.6, if k % 2 == 0 { iron } else { iron_dark }, flash);
    }
    // Bold safety-orange steel triangle load binder + chrome ratchet lever!
    let b_corners: [Vec2; 3] = [
        at_binder + drag * 5.4,
        at_binder + drag.rotate(TAU / 3.0) * 5.0,
        at_binder + drag.rotate(2.0 * TAU / 3.0) * 5.0,
    ];
    for k in 0..3 {
        draw_capsule(
            pb,
            b_corners[k],
            b_corners[(k + 1) % 3],
            1.7,
            binder.hi,
            binder.mid,
            binder.lo,
            flash,
        );
    }
    // Bright chrome ratchet latch bar across the binder center!
    draw_capsule(
        pb,
        at_binder - drag * 2.6,
        at_binder + drag * 3.4,
        1.0,
        (255, 255, 255),
        iron.hi,
        iron.lo,
        flash,
    );

    // Tell friction sparks shooting off the hauled triangular binder!
    if let Pose::Tell(t) = pose {
        for k in 0..4 {
            let ph = (t * 4.5 + k as f32 * 0.25).fract();
            let sp = at_binder + drag.perp() * ((k as f32 - 1.5) * 2.8) - drag * (ph * 4.0)
                + Vec2::new(0.0, -ph * 4.5);
            blend_px(pb, sp.x.round() as i32, sp.y.round() as i32, (255, 244, 152), 1.0 - ph * 0.5);
        }
    }

    // 3. Four clanking chain-link hound paws!
    let stride = match pose {
        Pose::Approach => (anim * 7.2).sin() * 3.2,
        Pose::Retreat(t) => (t * 8.0).sin() * 2.2,
        _ => 0.0,
    };
    for &(ax, sx, phase) in &[(-5.5_f32, -4.2_f32, -1.0_f32), (-5.5, 4.2, 1.0), (3.6, -4.4, 1.0), (3.6, 4.4, -1.0)] {
        let step = stride * phase;
        let paw = body + f * (ax + step * 0.7) + s * sx + Vec2::new(0.0, 4.6 - step.max(0.0) * 0.5);
        draw_shaded_disc(pb, paw.x, paw.y, 1.8, iron.hi, iron.mid, iron.lo, flash);
    }

    // 4. Forged-iron interlocking CHAIN LOOP BODY (10 distinct oval chain links)!
    let links = 10;
    for i in 0..links {
        let a0 = i as f32 * (TAU / links as f32) + anim * 0.9;
        let a1 = (i as f32 + 0.78) * (TAU / links as f32) + anim * 0.9;
        let ripple = (anim * 5.0 + i as f32 * 0.9).sin() * 0.7;
        let p0 = body + f * (a0.cos() * 7.0 - 1.4) + s * (a0.sin() * 4.8 + ripple);
        let p1 = body + f * (a1.cos() * 7.0 - 1.4) + s * (a1.sin() * 4.8 + ripple);
        mb2_draw_ring_link(pb, p0, p1, 1.85, if i % 2 == 0 { iron } else { iron_dark }, flash);
    }

    // 5. Spiked crimson-and-brass guard collar at the neck!
    let neck = body + f * 4.8;
    draw_rotated_box(pb, neck, s, 4.2, 1.4, collar.hi, collar.mid, collar.lo, flash);
    for &side in &[-1.0_f32, 0.0, 1.0] {
        let sp = neck + s * (side * 2.8) + Vec2::new(0.0, -1.0);
        draw_shaded_disc(pb, sp.x, sp.y, 1.1, brass.hi, brass.mid, brass.lo, flash);
    }

    // 6. Snarling iron-hound head, swept pointed ears, snapping steel-fanged jaws & fiery eyes!
    let head = body + f * 7.8 + Vec2::new(0.0, -1.0 - coil * 0.8);
    for &side in &[-1.0_f32, 1.0] {
        let ear_a = head + s * (side * 2.2) - f * 0.8;
        let ear_b = head + s * (side * 3.4) - f * 2.8;
        let ear_tip = head - f * (3.6 + coil * 1.4) + s * (side * 5.6) + Vec2::new(0.0, -2.4);
        mb2_draw_poly_tri(pb, ear_a, ear_b, ear_tip, hide, flash);
    }
    draw_shaded_disc(pb, head.x, head.y, 3.8, hide.hi, hide.mid, hide.lo, flash);
    // Snapping upper & lower jaws with gleaming white steel fangs!
    let snout_top = head + f * 5.0 + Vec2::new(0.0, -0.5 - jaw_open * 0.35);
    let snout_bot = head + f * 4.4 + Vec2::new(0.0, 1.2 + jaw_open * 0.55);
    draw_tapered_capsule(pb, head + f * 1.4, snout_bot, 1.8, 1.0, hide.mid, hide.lo, INK, flash);
    draw_tapered_capsule(pb, head + f * 1.4, snout_top, 2.2, 1.3, hide.hi, hide.mid, hide.lo, flash);
    // Gleaming fangs between the parted jaws!
    if jaw_open > 0.7 {
        let f1 = head + f * 3.8 + Vec2::new(0.0, 0.5);
        let f2 = head + f * 5.2 + Vec2::new(0.0, 0.4);
        plot_px(pb, f1.x.round() as i32, f1.y.round() as i32, (255, 255, 255));
        plot_px(pb, f2.x.round() as i32, f2.y.round() as i32, (255, 255, 255));
    }
    // Fiery guard-dog eyes!
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 232, 92),
        Pose::Strike(_) => (255, 118, 72),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 204, 88),
    };
    draw_eye(pb, head + f * 1.2 + s * 1.9 + Vec2::new(0.0, -0.8), 1.5, eye_col, INK, f, expr, flash);
    draw_eye(pb, head + f * 1.2 - s * 1.9 + Vec2::new(0.0, -0.8), 1.5, eye_col, INK, f, expr, flash);

    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, head + Vec2::new(0.0, -11.5), t);
    }
    if let Pose::Recover(t) = pose {
        // Exposed rear latch behind the chain loop ("Slip behind the latch")!
        let latch_spot = body - f * 7.8;
        draw_dizzy_stars(pb, head + Vec2::new(0.0, -9.0), 7.0, t);
        draw_recover_beacon(pb, latch_spot, t);
    }
    draw_hurt_burst(pb, body, f, flash, binder.hi);
}

// ============================================================================
// 24. THE ROAD ATLAS — "Every exit has a bird on it." (Flatbed 12:00)
//     36–38 px coastal herring gull (slate wings, jet-black primary wingtips
//     with white mirrors, hooked yellow beak with red spot) draped in an
//     accordion-folded crimson-and-azure highway road map!
// ============================================================================

fn draw_road_atlas(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let f = safe_dir(f, Vec2::new(1.0, 0.0));
    let s = f.perp();
    let gull = mb2_pal((250, 250, 246), (204, 212, 222), (118, 130, 148));
    let slate = mb2_pal((178, 202, 228), (114, 138, 168), (52, 66, 90));
    let tip_blk = mb2_pal((96, 106, 126), (36, 42, 56), (16, 18, 26));
    let map_pal = mb2_pal((255, 242, 196), (220, 198, 138), (138, 118, 72));
    let beak = mb2_pal((255, 224, 84), (218, 146, 34), (118, 66, 14));
    let hwy_red: Rgb = (228, 54, 54);
    let river_blue: Rgb = (56, 152, 232);
    let park_green: Rgb = (74, 166, 88);

    if let Pose::Death(t) = pose {
        let burst = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
        let bounce = ((1.0 - t) * PI * 2.0).sin().abs() * (1.0 - t) * 4.5;
        draw_shadow(pb, center.x + 1.0, center.y + 6.0, 18.0, 7.6, 0.48);
        // Scattered slate & black gull feathers!
        for &side in &[-1.0_f32, 1.0] {
            let w_p = center + s * (side * (6.5 + burst * 6.0)) - Vec2::new(0.0, bounce * 0.7);
            mb2_draw_poly_tri(
                pb,
                w_p - f * 4.0,
                w_p + f * 3.0,
                w_p + s * (side * 8.0) - f * 2.0,
                slate,
                flash,
            );
            plot_px(pb, (w_p + s * (side * 7.0)).x.round() as i32, (w_p + s * (side * 7.0)).y.round() as i32, (255, 255, 255));
        }
        // Torn accordion highway road map sprawled flat beside the chest!
        let mp = center + f * (3.5 + burst * 4.0) - Vec2::new(0.0, bounce * 0.9);
        draw_rotated_box(pb, mp, f.rotate(0.28), 7.5, 4.8, map_pal.hi, map_pal.mid, map_pal.lo, flash);
        let ra = mp - f * 5.0 - s * 2.0;
        let rb = mp + f * 5.0 + s * 2.0;
        pb.line(ra.x.round() as i32, ra.y.round() as i32, rb.x.round() as i32, rb.y.round() as i32, hwy_red);
        if t < 0.42 {
            draw_hurt_burst(pb, center, f, 1.0 - t / 0.42, map_pal.hi);
        }
        draw_chest_pointer(pb, center, t);
        return;
    }

    let ent_drop = if let Pose::Entrance(t) = pose {
        (1.0 - t) * (1.0 - t) * -26.0
    } else {
        0.0
    };

    // `sweep`: wing angle from facing `f` (1.35 = broad soaring gull span; 2.05 = cocked far back in Tell; 0.78 = hammered forward in Strike!).
    // `bank`: lateral lean on each of the 3 V-stamp talon beats (left / tip / right)!
    let (lunge, lift, sweep, flap_y, bank, talons_out, coil) = match pose {
        Pose::Entrance(t) => (0.0, ent_drop, 1.20 + (1.0 - t) * 0.45, -(1.0 - t) * 5.0, 0.0, 1.0 - t, 0.0),
        Pose::Idle => {
            let beat = (anim * 3.0).sin();
            (0.0, -2.2 + beat * 1.4, 1.36 + beat * 0.14, -beat * 2.4, beat * 0.6, 0.0, 0.0)
        }
        Pose::Approach => {
            let beat = (anim * 6.4).sin();
            (beat * 1.8, -3.2 + beat * 1.8, 1.32 + beat * 0.24, -beat * 4.0, (anim * 3.2).cos() * 2.0, 0.25, 0.0)
        }
        Pose::Tell(t) => {
            let c = tell_coil(t);
            let q = tell_quiver(t);
            // Stoop wind-up: rises high, sweeps wings dramatically back, flares golden talons!
            (-3.4 * c + q * 0.8, -5.8 * c, 1.38 + 0.66 * c + q * 0.05, -5.2 * c, q * 1.0, c, c)
        }
        Pose::Strike(t) => {
            // Three-beat V-talon hammer matching left / tip / right stamp cadence!
            let (b_bank, b_u) = if t < 0.33 {
                (-3.6, t / 0.33)
            } else if t < 0.66 {
                (0.0, (t - 0.33) / 0.33)
            } else {
                (3.6, (t - 0.66) / 0.34)
            };
            let slam = (b_u * PI).sin();
            (4.6 * slam + 1.4, -3.8 * (1.0 - slam) + 1.2 * slam, 0.82 + (1.0 - slam) * 0.55, 3.6 * slam, b_bank * slam, 1.0, 1.0)
        }
        Pose::Recover(t) => (
            -1.4,
            2.2, // grounded & winded!
            1.12 + (t * 6.0).sin() * 0.08,
            2.8,
            (t * 7.0).sin() * 1.2,
            0.0,
            0.0,
        ),
        Pose::Retreat(t) => (
            -2.6,
            -2.2 - (t * 10.0).sin().abs() * 2.4,
            1.44 + (t * 10.0).sin() * 0.25,
            -(t * 10.0).sin() * 3.2,
            0.0,
            0.0,
            0.0,
        ),
        Pose::Death(_) => unreachable!(),
    };

    let jolt = hurt_jolt(f, flash, anim);
    let body = center + f * lunge + s * bank + jolt + Vec2::new(0.0, lift);
    let sh_s = (1.0 - (-lift).max(0.0) * 0.028).clamp(0.42, 1.12);
    draw_shadow(pb, center.x + 1.5, center.y + 8.5, 17.5 * sh_s, 6.8 * sh_s, 0.50);

    if let Pose::Entrance(t) = pose {
        if t >= 0.40 {
            draw_entrance_ring(pb, center, ((t - 0.40) / 0.60).clamp(0.0, 1.0), map_pal.hi);
        }
    }

    // 1. Strike sweeping wind-gust wing smear arcs!
    if let Pose::Strike(t) = pose {
        let ang = f.angle();
        let a = (1.0 - t * 0.4) * 0.72;
        draw_smear_arc(pb, body, ang - 1.95, ang - sweep, 8.0, 18.0, slate.hi, (255, 255, 250), a);
        draw_smear_arc(pb, body, ang + sweep, ang + 1.95, 8.0, 18.0, slate.hi, (255, 255, 250), a);
    }

    // 2. Spread golden-orange predatory talons flaring forward during Tell & Strike!
    if talons_out > 0.12 {
        for &side in &[-1.0_f32, 1.0] {
            let ankle = body + f * (2.6 + talons_out * 3.0) + s * (side * 3.6) + Vec2::new(0.0, 3.8);
            draw_capsule(pb, body + s * (side * 2.0), ankle, 1.1, beak.hi, beak.mid, beak.lo, flash);
            for k in -1..=1 {
                let claw = ankle + f.rotate(k as f32 * 0.45) * (2.4 + talons_out * 1.4) + Vec2::new(0.0, 1.4);
                draw_tapered_capsule(pb, ankle, claw, 0.9, 0.45, beak.hi, beak.mid, beak.lo, flash);
            }
        }
    }

    // 3. Broad coastal herring-gull WINGS (~36–38 px span!) with slate upper wings,
    //    jet-black primary tips & crisp white herring-gull wingtip mirrors!
    for &side in &[-1.0_f32, 1.0] {
        let root_f = body + f * 2.2 + s * (side * 2.4);
        let root_b = body - f * 4.4 + s * (side * 2.2);
        let w_dir = f.rotate(side * sweep);
        let wrist = body + w_dir * 10.2 + Vec2::new(0.0, flap_y * 0.55);
        let tip_dir = f.rotate(side * (sweep + 0.42));
        let tip = wrist + tip_dir * 9.2 + Vec2::new(0.0, flap_y);
        let trail = wrist - f * 4.2 + s * (side * 1.2) + Vec2::new(0.0, flap_y * 0.6);

        // Inner coastal-slate wing panel!
        mb2_draw_poly_tri(pb, root_f, root_b, wrist, slate, flash);
        mb2_draw_poly_tri(pb, root_b, trail, wrist, slate, flash);
        // Outer swept jet-black primary wingtip feather blade!
        mb2_draw_poly_tri(pb, wrist, trail, tip, tip_blk, flash);
        // Crisp white leading-edge feather highlight along the full wing + iconic white tip mirrors!
        pb.line(
            root_f.x.round() as i32,
            root_f.y.round() as i32,
            wrist.x.round() as i32,
            wrist.y.round() as i32,
            apply_flash(gull.hi, flash),
        );
        pb.line(
            wrist.x.round() as i32,
            wrist.y.round() as i32,
            tip.x.round() as i32,
            tip.y.round() as i32,
            apply_flash((216, 228, 244), flash),
        );
        let mirror1 = wrist + (tip - wrist) * 0.68;
        let mirror2 = wrist + (tip - wrist) * 0.84;
        plot_px(pb, mirror1.x.round() as i32, mirror1.y.round() as i32, (255, 255, 255));
        plot_px(pb, mirror2.x.round() as i32, mirror2.y.round() as i32, (255, 255, 255));
    }

    // 4. Fanned white & black-banded tail feathers behind `-f`!
    let tail_root = body - f * 5.2;
    mb2_draw_poly_tri(
        pb,
        tail_root,
        tail_root - f * 6.4 + s * 3.8,
        tail_root - f * 6.4 - s * 3.8,
        gull,
        flash,
    );
    draw_rotated_box(pb, tail_root - f * 5.6, s, 3.0, 0.7, tip_blk.hi, tip_blk.mid, tip_blk.lo, flash);

    // 5. Sleek sea-foam ivory gull torso & neck!
    draw_tapered_capsule(pb, body - f * 5.0, body + f * 5.6, 3.4, 3.0, gull.hi, gull.mid, gull.lo, flash);

    // 6. THE FOLDED HIGHWAY ROAD MAP draped across the gull's back & wings!
    //    Three crisp accordion-folded cream panels with scarlet highways, azure river & green park!
    let map_spread = 1.0 + coil * 0.35;
    let map_c = body - f * 1.4 + Vec2::new(0.0, -1.2 + flap_y * 0.25);
    let fold_ax = f.rotate(0.18);
    let fold_s = fold_ax.perp();
    // Center map panel + two angled accordion side-folds!
    for &panel_s in &[-1.0_f32, 0.0, 1.0] {
        let pc = map_c + fold_s * (panel_s * 4.4 * map_spread)
            + Vec2::new(0.0, panel_s.abs() * flap_y * 0.35);
        let p_ax = fold_ax.rotate(panel_s * (0.24 - coil * 0.12));
        let p_pal = if panel_s == 0.0 {
            map_pal
        } else {
            mb2_pal(map_pal.hi, mix(map_pal.mid, map_pal.lo, 0.25), map_pal.lo)
        };
        draw_rotated_box(pb, pc, p_ax, 4.4, 2.5, p_pal.hi, p_pal.mid, p_pal.lo, flash);
    }
    // Bold scarlet interstate highway zig-zag across the folded map!
    let r0 = map_c - fold_s * (6.2 * map_spread) - fold_ax * 1.8;
    let r1 = map_c + fold_ax * 1.6;
    let r2 = map_c + fold_s * (6.2 * map_spread) - fold_ax * 1.0;
    pb.line(r0.x.round() as i32, r0.y.round() as i32, r1.x.round() as i32, r1.y.round() as i32, apply_flash(hwy_red, flash));
    pb.line(r1.x.round() as i32, r1.y.round() as i32, r2.x.round() as i32, r2.y.round() as i32, apply_flash(hwy_red, flash));
    // Azure river line & green park blotch so it reads unmistakably as a road atlas!
    let rv0 = map_c - fold_s * (4.2 * map_spread) + fold_ax * 2.2;
    let rv1 = map_c + fold_s * (3.6 * map_spread) + fold_ax * 2.6;
    pb.line(rv0.x.round() as i32, rv0.y.round() as i32, rv1.x.round() as i32, rv1.y.round() as i32, apply_flash(river_blue, flash));
    let pk = map_c + fold_s * (3.8 * map_spread) - fold_ax * 2.2;
    plot_px(pb, pk.x.round() as i32, pk.y.round() as i32, apply_flash(park_green, flash));
    plot_px(pb, pk.x.round() as i32 + 1, pk.y.round() as i32, apply_flash(park_green, flash));

    // 7. Fierce gull head, hooked maritime-gold beak with iconic red gonys spot, & sharp eyes!
    let head = body + f * 6.8 + Vec2::new(0.0, -1.6 - coil * 0.8);
    draw_shaded_disc(pb, head.x, head.y, 3.2, gull.hi, gull.mid, gull.lo, flash);
    let beak_open = matches!(pose, Pose::Tell(_) | Pose::Strike(_) | Pose::Recover(_));
    let beak_tip = head + f * 6.2 + Vec2::new(0.0, if beak_open { -0.8 } else { 0.4 });
    draw_tapered_capsule(pb, head + f * 2.2, beak_tip, 1.5, 0.55, beak.hi, beak.mid, beak.lo, flash);
    if beak_open {
        let lower_tip = head + f * 5.2 + Vec2::new(0.0, 2.0);
        draw_tapered_capsule(pb, head + f * 2.0 + Vec2::new(0.0, 0.8), lower_tip, 1.1, 0.45, beak.mid, beak.lo, INK, flash);
        plot_px(pb, (head + f * 4.2 + Vec2::new(0.0, 1.6)).x.round() as i32, (head + f * 4.2 + Vec2::new(0.0, 1.6)).y.round() as i32, hwy_red);
    } else {
        // Signature crimson herring-gull spot on the lower bill!
        let spot = head + f * 4.6 + Vec2::new(0.0, 0.9);
        plot_px(pb, spot.x.round() as i32, spot.y.round() as i32, hwy_red);
    }
    let expr = eye_expr_for(pose, anim, flash);
    let eye_col = match pose {
        Pose::Tell(_) => (255, 232, 92),
        Pose::Strike(_) => (255, 118, 72),
        Pose::Recover(_) => (148, 255, 206),
        _ => (255, 238, 128),
    };
    draw_eye(pb, head + f * 0.8 + s * 1.6 + Vec2::new(0.0, -0.8), 1.4, eye_col, INK, f, expr, flash);
    draw_eye(pb, head + f * 0.8 - s * 1.6 + Vec2::new(0.0, -0.8), 1.4, eye_col, INK, f, expr, flash);

    if let Pose::Tell(t) = pose {
        mb2_tell_pip(pb, head + Vec2::new(0.0, -11.5), t);
    }
    if let Pose::Recover(t) = pose {
        draw_dizzy_stars(pb, head + Vec2::new(0.0, -9.0), 7.6, t);
        draw_recover_beacon(pb, map_c, t);
    }
    draw_hurt_burst(pb, body, f, flash, map_pal.hi);
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
    if let (MinibossShape::Cone { .. }, SignatureShape::Cord { len, half, sweep }) =
        (full_shape, mb.kind.signature().shape)
    {
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
        shape => crate::game::ecs::systems::miniboss_shapes::draw_telegraph(
            pb, mb, &shape, cam, Vec2::splat(1.0),
        ),
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

    #[test]
    fn all_eighteen_wave4_minibosses_render_with_contrast_and_bounds() {
        use rand::SeedableRng;
        let bg = (16, 18, 22);
        let mut pb = PixelBuffer::new(96, 96);
        let cam = Vec2::new(100.0, 100.0);
        let origin = Vec2::new(148.0, 148.0);
        let aim = Vec2::new(-1.0, 0.0);
        let wave4 = [
            MinibossKind::ValveWidow,
            MinibossKind::Mudlark,
            MinibossKind::PastryWheel,
            MinibossKind::SaltHen,
            MinibossKind::PotBrush,
            MinibossKind::DosePump,
            MinibossKind::PetriMother,
            MinibossKind::CageCounter,
            MinibossKind::RotorTechnician,
            MinibossKind::SudsBailiff,
            MinibossKind::WringerBoy,
            MinibossKind::QuoinKey,
            MinibossKind::CarbonTwin,
            MinibossKind::LatePlate,
            MinibossKind::TarpaulinPup,
            MinibossKind::AxleNut,
            MinibossKind::ChainDog,
            MinibossKind::RoadAtlas,
        ];
        let mut rng = rand::rngs::StdRng::seed_from_u64(19);
        for kind in wave4 {
            for state in [
                MinibossState::Approach,
                MinibossState::Tell,
                MinibossState::Active,
                MinibossState::Recover,
            ] {
                pb.clear(bg);
                let mut e = Enemy::spawn(crate::game::enemy::EnemyKind::Brute, origin, 1.0, &mut rng);
                e.hp = 160.0;
                e.max_hp = 160.0;
                e.radius = kind.body_radius();
                e.facing = aim;
                e.vel = if state == MinibossState::Approach { aim * 30.0 } else { Vec2::ZERO };
                e.anim = 1.4;
                let mut mb = Miniboss::new(kind, 0);
                mb.origin = origin;
                mb.aim = aim;
                mb.state = state;
                mb.timer = if state == MinibossState::Active {
                    kind.active_duration() * 0.5
                } else {
                    0.5
                };
                draw(&mut pb, &e, &mb, cam, (90, 220, 200));

                let mut count = 0usize;
                let mut max_lum = 0u32;
                let (mut x0, mut x1, mut y0, mut y1) = (96usize, 0usize, 96usize, 0usize);
                for y in 0..96usize {
                    for x in 0..96usize {
                        let c = pb.pixel_at(x, y);
                        if c != bg {
                            count += 1;
                            let l = c.0 as u32 + c.1 as u32 + c.2 as u32;
                            if l > max_lum {
                                max_lum = l;
                            }
                            x0 = x0.min(x);
                            x1 = x1.max(x);
                            y0 = y0.min(y);
                            y1 = y1.max(y);
                        }
                    }
                }
                assert!(count >= 40, "{kind:?}/{state:?}: too few pixels ({count})");
                assert!(
                    max_lum >= 540,
                    "{kind:?}/{state:?}: needs a bright rim or signature highlight (max_lum {max_lum})"
                );
                if state == MinibossState::Approach {
                    let w = x1 - x0 + 1;
                    let h = y1 - y0 + 1;
                    assert!(
                        w <= 44 && h <= 44,
                        "{kind:?} Approach body sprawls to {w}x{h} (limit 44x44)"
                    );
                }
            }
        }
    }
}
