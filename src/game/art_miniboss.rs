//! Art for the six named minibosses and their ground telegraphs (wave 3, §8).
//!
//! Each miniboss has a distinct 14–20 px directional silhouette with top-left
//! lighting, dark contour rims, grounded shadows, full pose animation
//! (`Idle`, `Approach`, `Tell`, `Strike`, `Recover`, `Retreat`, `Death`), a
//! hurt flash, and ground telegraphs drawn strictly from the simulation's
//! [`MinibossShape`] geometry (never hard-coded sizes).
//!
//! Zero heap allocations; early-culled to [`PixelBuffer`] bounds.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::game::ecs::systems::miniboss::{
    Miniboss, MinibossKind, MinibossShape, MinibossState, RECOVER_TIME, TELL_TIME,
};
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Top-left directional light shared across `art_rat`, `art_enemy`, `art_props`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// Deep ink outline shared across miniboss silhouettes.
const INK: Rgb = (18, 15, 22);
/// Warm ivory hurt-flash tint.
const FLASH_COL: Rgb = (255, 246, 222);

/// Animation pose derived from `Enemy` + `Miniboss` state.
#[derive(Clone, Copy, Debug)]
enum Pose {
    Idle,
    Approach,
    Tell(f32),
    Strike(f32),
    Recover(f32),
    Retreat(f32),
    Death,
}

impl Pose {
    #[inline]
    fn from_state(e: &Enemy, mb: &Miniboss) -> Self {
        if !e.alive() {
            return Pose::Death;
        }
        match mb.state {
            MinibossState::Approach => {
                if e.vel.len_sq() < 4.0 {
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
fn apply_flash(c: Rgb, flash: f32) -> Rgb {
    if flash > 0.01 {
        mix(c, FLASH_COL, (flash * 0.78).clamp(0.0, 0.88))
    } else {
        c
    }
}

/// Soft elliptical floor shadow cast down-right.
fn draw_shadow(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(bw - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(bh - 1);
    for y in y0..=y1 {
        let ny = (y as f32 + 0.5 - cy) / ry.max(0.5);
        for x in x0..=x1 {
            let nx = (x as f32 + 0.5 - cx) / rx.max(0.5);
            let d2 = nx * nx + ny * ny;
            if d2 < 1.0 {
                let falloff = (1.0 - d2).sqrt();
                darken_px(pb, x, y, 1.0 - strength * falloff);
            }
        }
    }
}

/// Draw a shaded disc with dark contour outline & top-left directional highlight.
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
    let ir = r.ceil() as i32 + 1;
    let x0 = (cx.floor() as i32 - ir).max(0);
    let x1 = (cx.ceil() as i32 + ir).min(bw - 1);
    let y0 = (cy.floor() as i32 - ir).max(0);
    let y1 = (cy.ceil() as i32 + ir).min(bh - 1);
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - cy;
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt();
            if d > r + 0.85 {
                continue;
            }
            let c = if d > r - 0.15 {
                apply_flash(INK, flash * 0.4)
            } else {
                let nx = dx / r.max(0.5);
                let ny = dy / r.max(0.5);
                let lit = (0.52 + 0.52 * (nx * LIGHT.x + ny * LIGHT.y)).clamp(0.0, 1.0);
                let base = if lit > 0.55 {
                    mix(mid, hi, (lit - 0.55) / 0.45)
                } else {
                    mix(lo, mid, lit / 0.55)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Draw a thick oriented capsule segment from `a` to `b` with outline & shading.
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
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let pad = r + 1.5;
    let x0 = (a.x.min(b.x) - pad).floor().max(0.0) as i32;
    let x1 = (a.x.max(b.x) + pad).ceil().min((bw - 1) as f32) as i32;
    let y0 = (a.y.min(b.y) - pad).floor().max(0.0) as i32;
    let y1 = (a.y.max(b.y) + pad).ceil().min((bh - 1) as f32) as i32;
    let ab = b - a;
    let len2 = ab.len_sq();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let t = if len2 > 1e-6 {
                ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = a + ab * t;
            let v = p - q;
            let d = v.len();
            if d > r + 0.85 {
                continue;
            }
            let c = if d > r - 0.15 {
                apply_flash(INK, flash * 0.4)
            } else {
                let n = if d > 1e-4 { v * (1.0 / r.max(0.5)) } else { Vec2::ZERO };
                let lit = (0.54 + 0.50 * n.dot(LIGHT)).clamp(0.0, 1.0);
                let base = if lit > 0.55 {
                    mix(mid, hi, (lit - 0.55) / 0.45)
                } else {
                    mix(lo, mid, lit / 0.55)
                };
                apply_flash(base, flash)
            };
            pb.plot(x, y, c);
        }
    }
}

/// Subtle golden-white vulnerability pulse during `Pose::Recover` so the
/// player's punish window reads immediately.
fn draw_recover_glint(pb: &mut PixelBuffer, pos: Vec2, t: f32) {
    let pulse = 0.55 + 0.45 * (t * TAU * 3.0).sin();
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    let col = mix((255, 224, 128), (255, 255, 255), pulse * 0.65);
    blend_px(pb, cx, cy, col, 0.90);
    blend_px(pb, cx - 1, cy, col, 0.55 * pulse);
    blend_px(pb, cx + 1, cy, col, 0.55 * pulse);
    blend_px(pb, cx, cy - 1, col, 0.55 * pulse);
    blend_px(pb, cx, cy + 1, col, 0.55 * pulse);
}

// ============================================================================
// Public Miniboss Body Entry Point
// ============================================================================

/// Draw one miniboss body at `e.pos` with full pose animation, hurt flash,
/// death state, and radial HP ring.
pub fn draw(pb: &mut PixelBuffer, e: &Enemy, mb: &Miniboss, cam: Vec2, accent: Rgb) {
    let p = e.pos - cam;
    if !p.x.is_finite() || !p.y.is_finite() {
        return;
    }
    if p.x < -26.0 || p.y < -26.0 || p.x > pb.w as f32 + 26.0 || p.y > pb.h as f32 + 26.0 {
        return;
    }

    let pose = Pose::from_state(e, mb);
    let flash = (e.flash / 0.12).clamp(0.0, 1.0);
    let facing = match pose {
        Pose::Tell(_) | Pose::Strike(_) | Pose::Recover(_) => safe_dir(mb.aim, e.facing),
        _ => safe_dir(e.facing, mb.aim),
    };
    let anim = if e.anim.is_finite() { e.anim } else { 0.0 };

    match mb.kind {
        MinibossKind::Corkscrew => draw_corkscrew(pb, p, facing, pose, anim, flash),
        MinibossKind::SisterSnip => draw_sister_snip(pb, p, facing, pose, anim, flash),
        MinibossKind::Bobbin => draw_bobbin(pb, p, facing, pose, anim, flash),
        MinibossKind::BellDiver => draw_bell_diver(pb, p, facing, pose, anim, flash),
        MinibossKind::Foreman => draw_foreman(pb, p, facing, pose, anim, flash),
        MinibossKind::MilkCarton => draw_milk_carton(pb, p, facing, pose, anim, flash),
    }

    // Crisp radial HP ring (hidden on Death so the collapsed corpse reads cleanly).
    if e.alive() && e.max_hp > 0.0 {
        let frac = (e.hp / e.max_hp).clamp(0.0, 1.0);
        let r = e.radius.max(6.5) + 4.0;
        let segs = 20;
        let lit = (frac * segs as f32).ceil() as i32;
        let ring_col = match pose {
            Pose::Tell(_) => (255, 212, 92),
            Pose::Strike(_) => (255, 96, 78),
            Pose::Recover(_) => (148, 255, 198),
            _ => accent,
        };
        for i in 0..segs {
            let a = -FRAC_PI_2 + (i as f32 / segs as f32) * TAU;
            let px = (p.x + a.cos() * r).round() as i32;
            let py = (p.y + a.sin() * r).round() as i32;
            if i < lit {
                blend_px(pb, px, py, ring_col, 0.85);
            } else {
                blend_px(pb, px, py, (28, 30, 38), 0.55);
            }
        }
    }
}

// ---- 1. Corkscrew: brass auger under a pink worm ---------------------------

fn draw_corkscrew(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let brass_hi: Rgb = (255, 224, 122);
    let brass_mid: Rgb = (212, 162, 62);
    let brass_lo: Rgb = (128, 88, 28);
    let worm_hi: Rgb = (252, 178, 196);
    let worm_mid: Rgb = (224, 124, 152);
    let worm_lo: Rgb = (146, 62, 90);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 9.0, 5.0, 0.45);
        // Snapped brass auger halves & curled pale pink worm segments.
        draw_capsule(pb, center - f * 7.0 + p * 2.0, center - f * 1.0 + p * 1.0, 2.2, brass_hi, brass_mid, brass_lo, flash);
        draw_capsule(pb, center + f * 2.0 - p * 2.0, center + f * 8.0 - p * 4.0, 1.8, brass_hi, brass_mid, brass_lo, flash);
        for (i, off) in [(-4.0, -3.0), (-1.0, -4.0), (2.0, -2.5), (3.5, 0.5)].iter().enumerate() {
            let pos = center + f * off.0 + p * off.1;
            draw_shaded_disc(pb, pos.x, pos.y, 2.2 - i as f32 * 0.2, (232, 184, 194), (176, 114, 134), worm_lo, flash);
        }
        return;
    }

    let (spin, recoil, lift, wiggle) = match pose {
        Pose::Idle => (anim * 1.5, 0.0, 0.0, (anim * 2.4).sin() * 0.8),
        Pose::Approach => (anim * 6.0, 0.0, 0.0, (anim * 6.5).sin() * 1.5),
        Pose::Tell(t) => (t * 24.0, -2.6 * t, -t, (t * 28.0).sin() * 0.6),
        Pose::Strike(t) => (t * 42.0, 3.2 * (1.0 - (t * 2.0 - 1.0).abs()), 0.5, 0.0),
        Pose::Recover(t) => (0.0, 2.0 * (1.0 - t * 0.4), 1.0, (t * 8.0).sin() * 0.5),
        Pose::Retreat(t) => (-t * 10.0, -1.8, 0.0, (anim * 4.0).sin() * 1.0),
        Pose::Death => unreachable!(),
    };

    let base = center + f * recoil + Vec2::new(0.0, lift);
    draw_shadow(pb, base.x + 1.0, base.y + 3.5, 9.5, 5.2, 0.50);

    // 1. Spiraled brass auger underneath, tapering from heavy rear collar to razor nose tip!
    let tip = base + f * 9.5;
    let tail = base - f * 7.5;
    let flutes = 6;
    for i in 0..flutes {
        let u_frac = i as f32 / (flutes - 1) as f32;
        let pos = tail + (tip - tail) * u_frac;
        let r = (4.0 * (1.0 - u_frac * 0.68)).max(1.2);
        // Spiral flute ridge slides along the shaft as `spin` turns.
        let phase = (u_frac * 5.0 - spin).sin();
        let hi = if phase > 0.2 { (255, 242, 174) } else { brass_hi };
        let mid = if phase < -0.2 { brass_lo } else { brass_mid };
        draw_shaded_disc(pb, pos.x, pos.y + 1.0, r, hi, mid, brass_lo, flash);
    }
    // Gleaming sharpened auger drill point.
    let tip_px = (tip.x.round() as i32, (tip.y + 0.8).round() as i32);
    pb.plot(tip_px.0, tip_px.1, apply_flash((255, 248, 200), flash));

    // Tell spin sparks flying off the brass bit.
    if let Pose::Tell(t) = pose {
        let sp = tip + p * ((t * 18.0).sin() * 3.0);
        blend_px(pb, sp.x.round() as i32, sp.y.round() as i32, (255, 246, 168), 0.92);
    }

    // 2. Segmented fleshy pink worm draped over the top of the brass auger!
    let head_ext = if matches!(pose, Pose::Recover(_)) { 3.5 } else { 0.0 };
    for seg in 0..5 {
        let s = seg as f32 / 4.0;
        let u = -5.5 + s * (10.5 + head_ext);
        let sway = ((s * 4.2 + anim * 3.0).sin() * wiggle) * (1.0 - s * 0.35);
        let arch = -2.0 - (s * PI).sin() * 1.4;
        let pos = base + f * u + p * sway + Vec2::new(0.0, arch);
        let r = if seg == 4 { 3.1 } else { 2.4 + (s * PI).sin() * 0.8 };
        draw_shaded_disc(pb, pos.x, pos.y, r, worm_hi, worm_mid, worm_lo, flash);
    }
    // Worm head eyes & little mandible pips.
    let head = base + f * (5.2 + head_ext) + Vec2::new(0.0, -2.2);
    let eye_l = head + f * 1.2 + p * 1.3;
    let eye_r = head + f * 1.2 - p * 1.3;
    pb.plot(eye_l.x.round() as i32, eye_l.y.round() as i32, INK);
    pb.plot(eye_r.x.round() as i32, eye_r.y.round() as i32, INK);

    if let Pose::Recover(t) = pose {
        draw_recover_glint(pb, head, t);
    }
}

// ---- 2. Sister Snip: tall tailor's shears ----------------------------------

fn draw_sister_snip(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let steel_hi: Rgb = (242, 248, 255);
    let steel_mid: Rgb = (176, 190, 210);
    let steel_lo: Rgb = (96, 108, 128);
    let grip_hi: Rgb = (188, 174, 206);
    let grip_mid: Rgb = (124, 108, 146);
    let grip_lo: Rgb = (64, 54, 82);
    let brass: Rgb = (232, 188, 88);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 9.0, 5.5, 0.45);
        // Unhinged blades lying askew + popped brass pivot screw.
        draw_capsule(pb, center - f * 5.0 - p * 3.0, center + f * 7.0 - p * 5.0, 1.5, steel_hi, steel_mid, steel_lo, flash);
        draw_capsule(pb, center - f * 4.0 + p * 4.0, center + f * 6.0 + p * 2.0, 1.5, steel_hi, steel_mid, steel_lo, flash);
        draw_shaded_disc(pb, center.x + 2.0, center.y + 4.0, 1.4, (255, 232, 142), brass, (124, 86, 30), flash);
        return;
    }

    // Blade spread angle in radians: wide 45° yawn during Tell, razor snap-shut during Strike!
    let (spread, stride, lift) = match pose {
        Pose::Idle => (0.28 + (anim * 2.0).sin() * 0.06, 0.0, 0.0),
        Pose::Approach => (0.20 + (anim * 7.0).sin().abs() * 0.30, (anim * 7.0).sin() * 1.4, -0.5),
        Pose::Tell(t) => (0.28 + 0.56 * t, 0.0, -1.6 * t),
        Pose::Strike(t) => {
            // Closes fast across first 45% of Strike, then holds clamped shut!
            let close = (t / 0.45).clamp(0.0, 1.0);
            (0.84 * (1.0 - close) + 0.04, 2.2 * close, 0.0)
        }
        Pose::Recover(t) => (0.04, 1.6 * (1.0 - t * 0.5), 0.6),
        Pose::Retreat(_) => (0.34, -1.6, 0.0),
        Pose::Death => unreachable!(),
    };

    let pivot = center + f * stride + Vec2::new(0.0, lift);
    draw_shadow(pb, pivot.x + 1.0, pivot.y + 4.0, 8.5, 5.5, 0.48);

    // 1. Twin enameled finger-bow handle loops behind the pivot (`-f`),
    //    widely spaced so both scissor rings pop cleanly on every dark floor.
    for &side in &[-1.0_f32, 1.0] {
        let dir = f.rotate(side * spread * 0.72);
        let bow_c = pivot - dir * 5.8 + p * (side * 2.5);
        // Shank from pivot to handle ring.
        draw_capsule(pb, pivot, bow_c, 1.4, grip_hi, grip_mid, grip_lo, flash);
        // Finger ring (outer rim + dark hollow eye).
        draw_shaded_disc(pb, bow_c.x, bow_c.y, 2.9, grip_hi, grip_mid, grip_lo, flash);
        pb.plot(bow_c.x.round() as i32, bow_c.y.round() as i32, (16, 18, 26));
        pb.plot(bow_c.x.round() as i32, bow_c.y.round() as i32 - 1, (16, 18, 26));
    }

    // 2. Twin honed tall steel shear blades projecting forward (`+f`).
    for &side in &[-1.0_f32, 1.0] {
        let bdir = f.rotate(side * spread);
        let tip = pivot + bdir * 10.2;
        let mid_pt = pivot + bdir * 5.2 + bdir.perp() * (side * 0.9);
        draw_capsule(pb, pivot, mid_pt, 1.8, steel_hi, steel_mid, steel_lo, flash);
        draw_capsule(pb, mid_pt, tip, 1.2, steel_hi, steel_mid, steel_lo, flash);
        // Razor catchlight along inner cutting edge!
        let edge_pt = pivot + bdir * 6.5;
        pb.plot(edge_pt.x.round() as i32, edge_pt.y.round() as i32, apply_flash((255, 255, 255), flash));
    }

    // 3. Prominent brass pivot screw right at the hinge crossing.
    draw_shaded_disc(pb, pivot.x, pivot.y, 2.2, (255, 236, 152), brass, (132, 92, 32), flash);
    pb.plot(pivot.x.round() as i32, pivot.y.round() as i32, INK);

    // Metallic snap glint when blades close in Strike!
    if let Pose::Strike(t) = pose {
        if (0.35..0.75).contains(&t) {
            let glint = pivot + f * 7.5;
            blend_px(pb, glint.x.round() as i32, glint.y.round() as i32, (255, 255, 255), 0.95);
            blend_px(pb, glint.x.round() as i32 + 1, glint.y.round() as i32, (208, 240, 255), 0.75);
            blend_px(pb, glint.x.round() as i32 - 1, glint.y.round() as i32, (208, 240, 255), 0.75);
        }
    }
    if let Pose::Recover(t) = pose {
        draw_recover_glint(pb, pivot, t);
    }
}

// ---- 3. Bobbin: spindle rat reeling crimson thread -------------------------

fn draw_bobbin(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let wood_hi: Rgb = (224, 174, 112);
    let wood_mid: Rgb = (168, 116, 62);
    let wood_lo: Rgb = (96, 62, 32);
    let thread_hi: Rgb = (248, 134, 168);
    let thread_mid: Rgb = (198, 72, 112);
    let thread_lo: Rgb = (118, 34, 64);
    let brass: Rgb = (232, 192, 94);
    let ear_col: Rgb = (232, 206, 206);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 8.5, 5.0, 0.45);
        // Toppled wooden spool on its side + loose crimson thread curls.
        draw_shaded_disc(pb, center.x - 3.0, center.y, 4.2, wood_hi, wood_mid, wood_lo, flash);
        draw_shaded_disc(pb, center.x + 3.0, center.y + 1.0, 3.8, thread_hi, thread_mid, thread_lo, flash);
        for k in 0..7 {
            let a = k as f32 * 0.9;
            let q = center + Vec2::new(a.cos() * (5.0 + k as f32 * 0.5), a.sin() * 4.5);
            pb.plot(q.x.round() as i32, q.y.round() as i32, thread_hi);
        }
        return;
    }

    let (spin, tilt, bob) = match pose {
        Pose::Idle => (anim * 1.6, 0.0, (anim * 2.5).sin() * 0.6),
        Pose::Approach => (anim * 8.0, (anim * 8.0).sin() * 1.1, (anim * 8.0).cos() * 0.8),
        Pose::Tell(t) => (t * 22.0, -1.2 * t, -0.8 * t),
        Pose::Strike(t) => (t * 36.0, (t * PI).sin() * 2.0, 0.0),
        Pose::Recover(t) => (0.0, 1.8 * (1.0 - t * 0.4), 1.0),
        Pose::Retreat(t) => (-t * 14.0, -1.4, 0.0),
        Pose::Death => unreachable!(),
    };

    let base = center + p * tilt + Vec2::new(0.0, bob);
    draw_shadow(pb, base.x + 1.0, base.y + 4.5, 7.5, 4.8, 0.50);

    // 1. Bottom wooden bobbin flange resting near the floor.
    draw_shaded_disc(pb, base.x, base.y + 4.4, 5.4, wood_hi, wood_mid, wood_lo, flash);

    // 2. Lush, prominently exposed wound crimson-rose thread spindle drum!
    for row in -3..=3 {
        let ry = base.y + row as f32 * 1.1;
        let strand_lit = (row as f32 + spin).sin() * 0.5 + 0.5;
        let hi = mix(thread_mid, thread_hi, 0.45 + 0.55 * strand_lit);
        draw_capsule(
            pb,
            Vec2::new(base.x - 4.8, ry),
            Vec2::new(base.x + 4.8, ry),
            1.6,
            hi,
            thread_mid,
            thread_lo,
            flash,
        );
    }

    // 3. Little spindle-rat snout & twin round pale ears peeking out along `+f`!
    let snout = base + f * 6.8;
    draw_shaded_disc(pb, snout.x, snout.y, 2.3, (248, 240, 236), (198, 186, 184), (118, 108, 114), flash);
    let nose = base + f * 8.6;
    pb.plot(nose.x.round() as i32, nose.y.round() as i32, (252, 146, 178));
    for &side in &[-1.0_f32, 1.0] {
        let ear = base + f * 3.8 + p * (side * 4.4) + Vec2::new(0.0, -2.8);
        draw_shaded_disc(pb, ear.x, ear.y, 2.0, ear_col, (214, 152, 170), (120, 74, 92), flash);
    }

    // 4. Compact turned-wood top bobbin cap & brass spindle pin on top.
    draw_shaded_disc(pb, base.x, base.y - 4.8, 4.4, wood_hi, wood_mid, wood_lo, flash);
    draw_shaded_disc(pb, base.x, base.y - 5.0, 1.7, (255, 238, 158), brass, (132, 92, 34), flash);

    // Trailing loose crimson thread tail behind `-f`.
    for k in 1..=5 {
        let sway = (anim * 4.0 + k as f32 * 0.9).sin() * 1.4;
        let q = base - f * (5.0 + k as f32 * 1.1) + p * sway;
        pb.plot(q.x.round() as i32, q.y.round() as i32, apply_flash(thread_hi, flash));
    }

    if let Pose::Recover(t) = pose {
        draw_recover_glint(pb, base + Vec2::new(0.0, -4.0), t);
    }
}

// ---- 4. Bell Diver: verdigris diving-bell beetle with chipped rim ----------

fn draw_bell_diver(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let p = f.perp();
    let bell_hi: Rgb = (156, 228, 212);
    let bell_mid: Rgb = (86, 172, 156);
    let bell_lo: Rgb = (42, 96, 92);
    let brass_hi: Rgb = (242, 210, 118);
    let brass_mid: Rgb = (188, 146, 62);
    let brass_lo: Rgb = (108, 78, 30);
    let leg_col: Rgb = (34, 40, 48);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 9.0, 5.5, 0.45);
        // Cracked verdigris dome split down the chipped rim + loose brass clapper!
        draw_shaded_disc(pb, center.x - 2.0, center.y, 5.5, bell_hi, bell_mid, bell_lo, flash);
        draw_shaded_disc(pb, center.x + 4.0, center.y + 2.0, 3.2, bell_mid, bell_lo, (24, 48, 48), flash);
        pb.line((center.x - 1.0) as i32, (center.y - 4.0) as i32, (center.x + 2.0) as i32, (center.y + 4.0) as i32, INK);
        draw_shaded_disc(pb, center.x + 6.5, center.y - 2.5, 1.8, brass_hi, brass_mid, brass_lo, flash);
        return;
    }

    let (lift, rattle, tilt) = match pose {
        Pose::Idle => ((anim * 2.2).sin() * 0.5, 0.0, 0.0),
        Pose::Approach => (-0.8 - (anim * 7.0).sin().abs() * 0.9, 0.0, (anim * 7.0).sin() * 0.8),
        Pose::Tell(t) => (-3.5 * t, (t * 28.0).sin() * 0.6 * t, 0.0),
        Pose::Strike(t) => {
            // Slams flat onto the floor and rings violently!
            let ring = (1.0 - t) * (t * 44.0).sin() * 1.4;
            (1.2, ring, 0.0)
        }
        Pose::Recover(t) => (0.5, 0.0, 2.2 * (1.0 - t * 0.4)),
        Pose::Retreat(_) => (-1.2, 0.0, -1.0),
        Pose::Death => unreachable!(),
    };

    let body = center + Vec2::new(rattle + tilt * 0.4, lift);
    let shadow_r = if matches!(pose, Pose::Tell(_)) { 9.2 } else { 8.0 };
    draw_shadow(pb, center.x + 1.0, center.y + 4.5, shadow_r, 5.0, 0.52);

    // 1. Six dark tapping beetle legs tucked around the bell's lower skirt.
    for i in 0..3 {
        let side_ang = -0.8 + i as f32 * 0.8;
        let step = (anim * 7.0 + i as f32 * 1.8).sin() * if matches!(pose, Pose::Approach) { 1.4 } else { 0.3 };
        for &s in &[-1.0_f32, 1.0] {
            let ldir = f.rotate(side_ang * s + s * FRAC_PI_2 * 0.45);
            let foot = center + ldir * (7.5 + step * 0.4) + Vec2::new(0.0, 3.0);
            let hip = body + ldir * 4.5 + Vec2::new(0.0, 2.5);
            pb.line(hip.x.round() as i32, hip.y.round() as i32, foot.x.round() as i32, foot.y.round() as i32, apply_flash(leg_col, flash));
            pb.plot(foot.x.round() as i32, foot.y.round() as i32, apply_flash(brass_lo, flash));
        }
    }

    // Glowing brass clapper peeking under the lifted bell during Tell!
    if let Pose::Tell(t) = pose {
        let clap = center + Vec2::new(0.0, 3.0);
        let col = mix(brass_hi, (255, 255, 220), t);
        draw_shaded_disc(pb, clap.x, clap.y, 2.2, col, brass_hi, brass_mid, flash);
    }

    // 2. Flared brass diving-bell rim with an UNMISTAKABLE chipped notch along `f` (`gap_dir`)!
    let rim_y = body.y + 2.6;
    let rx = 7.8_f32;
    let ry = 3.8_f32;
    let bx0 = (body.x - rx - 1.5).floor() as i32;
    let bx1 = (body.x + rx + 1.5).ceil() as i32;
    let by0 = (rim_y - ry - 1.5).floor() as i32;
    let by1 = (rim_y + ry + 1.5).ceil() as i32;
    for y in by0..=by1 {
        let dy = (y as f32 + 0.5 - rim_y) / ry;
        for x in bx0..=bx1 {
            let dx = (x as f32 + 0.5 - body.x) / rx;
            let d = (dx * dx + dy * dy).sqrt();
            if d > 1.08 {
                continue;
            }
            // Carve out the iconic 60° chipped rim notch along `+f`!
            let vdir = Vec2::new(dx, dy);
            let in_chip = vdir.len_sq() > 0.25 && vdir.normalized().dot(f) > 0.84;
            if in_chip {
                // Bright jagged broken-brass lip right on the chip edge.
                if d > 0.65 && d < 0.86 {
                    pb.plot(x, y, apply_flash((255, 244, 186), flash));
                }
                continue;
            }
            let c = if d > 0.90 {
                INK
            } else if dx + dy < -0.2 {
                brass_hi
            } else if dx + dy < 0.35 {
                brass_mid
            } else {
                brass_lo
            };
            pb.plot(x, y, apply_flash(c, flash));
        }
    }

    // 3. Domed verdigris-bronze bell shell + brass crown lug + amber porthole eye!
    draw_shaded_disc(pb, body.x, body.y - 1.4, 6.2, bell_hi, bell_mid, bell_lo, flash);
    // Brass crown suspension ring at top of dome.
    draw_shaded_disc(pb, body.x, body.y - 6.8, 2.0, brass_hi, brass_mid, brass_lo, flash);
    // Amber diving-helmet porthole eye facing `f`.
    let eye = body + f * 3.2 + Vec2::new(0.0, -1.2);
    draw_shaded_disc(pb, eye.x, eye.y, 1.8, (255, 240, 152), (234, 168, 56), (112, 68, 24), flash);
    // Acoustic resonance halo during Strike!
    if let Pose::Strike(t) = pose {
        let a_col = mix((148, 248, 232), (255, 255, 255), 1.0 - t);
        for k in 0..8 {
            let ang = k as f32 * TAU / 8.0 + t * 4.0;
            let q = body + Vec2::from_angle(ang) * 8.5;
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, a_col, 0.75 * (1.0 - t));
        }
    }
    let _ = p;
    if let Pose::Recover(t) = pose {
        draw_recover_glint(pb, body + Vec2::new(0.0, -5.5), t);
    }
}

// ---- 5. The Foreman: walking wooden-handled rubber stamp -------------------

fn draw_foreman(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let wood_hi: Rgb = (224, 154, 96);
    let wood_mid: Rgb = (174, 102, 56);
    let wood_lo: Rgb = (102, 54, 28);
    let knob_hi: Rgb = (242, 182, 122);
    let rubber_hi: Rgb = (176, 62, 82);
    let rubber_lo: Rgb = (88, 26, 42);
    let brass: Rgb = (232, 194, 92);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 9.5, 5.5, 0.45);
        // Smudge of red stamp ink + cracked wooden base + snapped knob handle!
        draw_shaded_disc(pb, center.x + 1.0, center.y + 2.5, 6.0, rubber_hi, rubber_lo, (42, 12, 22), flash);
        pb.rect_fill((center.x - 6.0) as i32, (center.y - 3.0) as i32, 11, 7, apply_flash(INK, flash * 0.4));
        pb.rect_fill((center.x - 5.0) as i32, (center.y - 2.0) as i32, 9, 5, apply_flash(wood_mid, flash));
        draw_shaded_disc(pb, center.x + 6.5, center.y - 3.5, 3.0, knob_hi, wood_mid, wood_lo, flash);
        return;
    }

    // Heavy pompous waddle on Approach, hoisted high on Tell, 3 crisp slams on Strike!
    let (lift, lean_x, squash) = match pose {
        Pose::Idle => (0.0, 0.0, 0.0),
        Pose::Approach => (-(anim * 6.0).sin().abs() * 1.4, (anim * 6.0).sin() * 1.4, 0.0),
        Pose::Tell(t) => (-3.8 * t, f.x * 1.8 * t, 0.0),
        Pose::Strike(t) => {
            // Three 0.3 s stamp beats across the 0.9 s window!
            let beat = (t * 3.0).fract();
            if beat < 0.28 {
                (1.6, f.x * 2.4, 1.0) // slammed flat!
            } else {
                (-2.8 * ((beat - 0.28) / 0.72), f.x * 1.2, 0.0)
            }
        }
        Pose::Recover(t) => (0.8, 2.2 * (1.0 - t * 0.3), 0.0),
        Pose::Retreat(_) => (-0.8, -f.x * 1.8, 0.0),
        Pose::Death => unreachable!(),
    };

    let bx = (center.x + lean_x).round() as i32;
    let by = (center.y + lift).round() as i32;
    let hw = 7 + squash as i32;
    let hh = 5;

    // Grounded shadow widens slightly when hoisted high during Tell.
    draw_shadow(pb, center.x + 1.0, center.y + 6.0, 8.5 + (-lift).max(0.0) * 0.5, 4.5, 0.52);

    // 1. Two stubby ink-stained rubber marching feet beneath the stamp block.
    let stride = if matches!(pose, Pose::Approach) {
        (anim * 6.0).sin() * 2.0
    } else {
        0.0
    };
    for &sx in &[-4i32, 4] {
        let fy = center.y as i32 + 5 + if sx < 0 { stride as i32 } else { -stride as i32 } / 2;
        pb.rect_fill(bx + sx - 2, fy - 1, 4, 3, apply_flash(INK, flash * 0.4));
        pb.rect_fill(bx + sx - 1, fy, 2, 1, apply_flash(rubber_hi, flash));
    }

    // 2. Crimson ink-soaked rubber die pad peeking below the wooden mount.
    pb.rect_fill(bx - hw, by + hh - 1, hw * 2 + 1, 4, apply_flash(INK, flash * 0.4));
    pb.rect_fill(bx - hw + 1, by + hh, hw * 2 - 1, 2, apply_flash(rubber_lo, flash));
    pb.rect_fill(bx - hw + 1, by + hh, hw * 2 - 2, 1, apply_flash(rubber_hi, flash));

    // 3. Beveled rectangular mahogany stamp block with brass front index plate!
    pb.rect_fill(bx - hw, by - hh, hw * 2 + 1, hh * 2, apply_flash(INK, flash * 0.4));
    for dy in (-hh + 1)..hh {
        for dx in (-hw + 1)..hw {
            let col = if dy == -hh + 1 || dx == -hw + 1 {
                wood_hi
            } else if dy >= hh - 2 || dx == hw - 1 {
                wood_lo
            } else {
                wood_mid
            };
            pb.plot(bx + dx, by + dy, apply_flash(col, flash));
        }
    }
    // Gleaming brass label bar on the front face of the stamp block.
    pb.rect_fill(bx - 4, by + 2, 9, 2, apply_flash(brass, flash));
    pb.rect_fill(bx - 3, by + 2, 7, 1, apply_flash((255, 236, 152), flash));

    // 4. Tapered turned-wood knob handle rising from the top center!
    let neck_y = by - hh - 2;
    pb.rect_fill(bx - 2, neck_y, 5, 4, apply_flash(INK, flash * 0.4));
    pb.rect_fill(bx - 1, neck_y, 3, 4, apply_flash(wood_lo, flash));
    pb.plot(bx - 1, neck_y + 1, apply_flash(wood_mid, flash));
    // Rounded glossy top handle bulb.
    draw_shaded_disc(pb, bx as f32, (neck_y - 3) as f32, 3.8, knob_hi, wood_mid, wood_lo, flash);

    if let Pose::Recover(t) = pose {
        draw_recover_glint(pb, Vec2::new(bx as f32, (by + 2) as f32), t);
    }
}

// ---- 6. Milk Carton: four square feet, gable top, pouring spout ------------

fn draw_milk_carton(
    pb: &mut PixelBuffer,
    center: Vec2,
    f: Vec2,
    pose: Pose,
    anim: f32,
    flash: f32,
) {
    let wax_hi: Rgb = (248, 246, 236);
    let wax_mid: Rgb = (218, 214, 200);
    let wax_lo: Rgb = (158, 154, 144);
    let blue_hi: Rgb = (114, 168, 224);
    let blue_mid: Rgb = (64, 114, 174);
    let blue_lo: Rgb = (36, 68, 114);
    let barn_red: Rgb = (206, 68, 72);
    let sour_milk: Rgb = (244, 246, 214);

    if matches!(pose, Pose::Death) {
        draw_shadow(pb, center.x + 1.0, center.y + 3.0, 9.5, 5.5, 0.45);
        // Chalky curdled milk puddle + crumpled flat carton on its side.
        draw_shaded_disc(pb, center.x + 2.0, center.y + 3.0, 6.5, (248, 250, 232), sour_milk, (186, 192, 154), flash);
        pb.rect_fill((center.x - 7.0) as i32, (center.y - 3.0) as i32, 13, 7, apply_flash(INK, flash * 0.4));
        pb.rect_fill((center.x - 6.0) as i32, (center.y - 2.0) as i32, 11, 5, apply_flash(wax_mid, flash));
        pb.rect_fill((center.x - 6.0) as i32, (center.y) as i32, 11, 2, apply_flash(blue_mid, flash));
        return;
    }

    let face_x = if f.x >= 0.0 { 1i32 } else { -1 };
    let (tilt_x, lift, spout_open) = match pose {
        Pose::Idle => (0, (anim * 2.0).sin().round() as i32 / 2, false),
        Pose::Approach => ((anim * 6.0).sin().round() as i32, -(anim * 6.0).sin().abs().round() as i32, false),
        Pose::Tell(t) => ((face_x as f32 * 2.8 * t).round() as i32, -(t * 1.8).round() as i32, true),
        Pose::Strike(_) => (face_x * 4, 1, true),
        Pose::Recover(_) => (-face_x * 2, 0, false),
        Pose::Retreat(_) => (-face_x * 2, 0, false),
        Pose::Death => unreachable!(),
    };

    let cx = center.x.round() as i32 + tilt_x;
    let cy = center.y.round() as i32 + lift;
    draw_shadow(pb, center.x + 1.0, center.y + 6.5, 8.5, 4.2, 0.50);

    // 1. FOUR distinct square cardboard corner feet!
    let foot_step = if matches!(pose, Pose::Approach) {
        (anim * 6.0).sin().round() as i32
    } else {
        0
    };
    let foot_positions = [
        (-5, 5 + foot_step),
        (-2, 6 - foot_step),
        (2, 6 + foot_step),
        (5, 5 - foot_step),
    ];
    for (idx, &(fx, fy)) in foot_positions.iter().enumerate() {
        // Rear feet lift up when tilting forward in Tell/Strike!
        let rear_lift = if spout_open && ((face_x > 0 && idx < 2) || (face_x < 0 && idx >= 2)) {
            -2
        } else {
            0
        };
        let px = center.x.round() as i32 + fx;
        let py = center.y.round() as i32 + fy + rear_lift;
        pb.rect_fill(px - 1, py - 1, 3, 3, apply_flash(INK, flash * 0.4));
        pb.plot(px, py, apply_flash(wax_lo, flash));
    }

    // 2. Waxed cardboard rectangular carton body (`13 × 11 px`) with bold dairy-blue band!
    let hw = 6;
    let top_y = cy - 6;
    let bot_y = cy + 5;
    pb.rect_fill(cx - hw, top_y, hw * 2 + 1, bot_y - top_y + 1, apply_flash(INK, flash * 0.4));
    for py in (top_y + 1)..bot_y {
        let rel_y = py - top_y;
        let on_stripe = (4..=7).contains(&rel_y);
        for px in (cx - hw + 1)..(cx + hw) {
            let right_side = px > cx + 1;
            let col = if on_stripe {
                if px == cx - hw + 1 {
                    blue_hi
                } else if right_side {
                    blue_lo
                } else {
                    blue_mid
                }
            } else if px == cx - hw + 1 || rel_y == 1 {
                wax_hi
            } else if right_side {
                wax_lo
            } else {
                wax_mid
            };
            pb.plot(px, py, apply_flash(col, flash));
        }
    }
    // Printed red barn/dairy emblem pip on the carton front label.
    pb.rect_fill(cx - 2, top_y + 5, 3, 2, apply_flash(barn_red, flash));
    pb.plot(cx - 1, top_y + 4, apply_flash(wax_hi, flash));
    // Vertical folded seam crease on the rear flank (`-face_x`) — the player's opening!
    let seam_x = cx - face_x * (hw - 2);
    for py in (top_y + 1)..bot_y {
        pb.plot(seam_x, py, apply_flash(wax_lo, flash));
    }

    // 3. Creased gable roof peak & printed open triangular pour spout toward `+face_x`!
    for dy in 1..=4 {
        let rhw = (hw - dy).max(1);
        let py = top_y - dy;
        pb.rect_fill(cx - rhw - 1, py, rhw * 2 + 3, 1, apply_flash(INK, flash * 0.4));
        for px in (cx - rhw)..=(cx + rhw) {
            let c = if dy == 4 {
                blue_hi // crimped top fin ridge
            } else if px < cx {
                wax_hi
            } else {
                wax_mid
            };
            pb.plot(px, py, apply_flash(c, flash));
        }
    }

    // Open triangular spout beak sticking out toward `+f`!
    let spout_base = Vec2::new((cx + face_x * (hw - 1)) as f32, (top_y - 1) as f32);
    let spout_tip = spout_base + f * if spout_open { 3.6 } else { 2.0 } + Vec2::new(0.0, if spout_open { 1.2 } else { 0.0 });
    draw_capsule(pb, spout_base, spout_tip, 1.3, wax_hi, blue_hi, blue_lo, flash);

    // Swelling milk drop on Tell, arcing downward pour stream + froth splash on Strike!
    match pose {
        Pose::Tell(t) => {
            let drop = spout_tip + f * (1.2 + t * 1.2) + Vec2::new(0.0, 1.0 + t * 1.4);
            pb.plot(drop.x.round() as i32, drop.y.round() as i32, (255, 255, 240));
        }
        Pose::Strike(_) => {
            let splash = spout_tip + f * 5.0 + Vec2::new(0.0, 5.2);
            draw_capsule(pb, spout_tip, splash, 1.3, (255, 255, 248), sour_milk, (214, 218, 178), 0.0);
            draw_shaded_disc(pb, splash.x, splash.y, 2.2, (255, 255, 250), sour_milk, (206, 212, 166), 0.0);
        }
        Pose::Recover(t) => {
            draw_recover_glint(pb, Vec2::new(seam_x as f32, cy as f32), t);
        }
        _ => {}
    }
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
            draw_cord_telegraph(pb, spool, tip, half, active_shape, cam, is_active, progress);
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
    }
}

/// Corkscrew: dotted brass trench filling from tail to tip!
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
    let warn_fill: Rgb = (212, 152, 54);
    let warn_edge: Rgb = (255, 218, 114);
    let hot_fill: Rgb = (255, 134, 62);
    let hot_core: Rgb = (255, 246, 186);

    // Bounding box of full trench.
    let pad = half + 2.0;
    let x0 = (sa.x.min(sb.x) - pad).floor().max(0.0) as i32;
    let x1 = (sa.x.max(sb.x) + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (sa.y.min(sb.y) - pad).floor().max(0.0) as i32;
    let y1 = (sa.y.max(sb.y) + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;

    let active_len = match active_shape {
        Some(MinibossShape::Trench { a: aa, b: ab, .. }) => (ab - aa).len(),
        _ => 0.0,
    };
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - sa;
            let along = q.dot(dir);
            let across = q.dot(perp).abs();
            if along < -1.0 || along > len + 1.0 || across > half + 1.0 {
                continue;
            }
            let in_active = is_active && along >= 0.0 && along <= active_len && across <= half;
            if in_active {
                let near_head = (active_len - along).abs() < 4.0;
                let churn = ((along - progress * 24.0) * 0.9 + across).sin() * 0.5 + 0.5;
                let col = if near_head {
                    hot_core
                } else if across > half - 1.2 {
                    warn_edge
                } else {
                    mix(hot_fill, warn_edge, churn * 0.55)
                };
                blend_px(pb, x, y, col, 0.84);
            } else if across >= half - 0.8 || along <= 0.6 || along >= len - 0.6 {
                // Dotted brass trench outline (marches tail->tip during Tell!).
                let dash = ((along - progress * 14.0).rem_euclid(5.0)) < 2.8;
                if dash || is_active {
                    blend_px(pb, x, y, warn_edge, if is_active { 0.45 } else { 0.88 * pulse });
                }
            } else {
                // Soft translucent danger tint + subtle tail-to-tip chevron pulses.
                let chev = ((along - across - progress * 18.0).rem_euclid(9.0)) < 1.6;
                let a = if chev { 0.34 * pulse } else { 0.16 + 0.12 * progress };
                blend_px(pb, x, y, if chev { warn_edge } else { warn_fill }, a);
            }
        }
    }
}

/// Sister Snip: 45° shear wedge with converging steel blade-closure sweep!
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
    let pad = len + 2.0;
    let x0 = (o.x - pad).floor().max(0.0) as i32;
    let x1 = (o.x + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (o.y - pad).floor().max(0.0) as i32;
    let y1 = (o.y + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;

    let warn_fill: Rgb = (146, 178, 224);
    let warn_edge: Rgb = (232, 242, 255);
    let hot_fill: Rgb = (255, 114, 128);
    let blade_white: Rgb = (255, 255, 255);

    // During Strike, the twin blades snap inward from ±half_angle to 0!
    let blade_ang = if is_active {
        half_angle * (1.0 - (progress / 0.55).clamp(0.0, 1.0))
    } else {
        half_angle
    };
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let v = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - o;
            let d = v.len();
            if d < 3.0 || d > len + 1.0 {
                continue;
            }
            let ang = v.normalized().dot(dir).clamp(-1.0, 1.0).acos();
            if ang > half_angle + 0.06 {
                continue;
            }
            let arc_edge = (d - len).abs() <= 0.9;
            let side_edge = (ang - half_angle).abs() * d <= 1.05;
            let on_blade = (ang - blade_ang).abs() * d <= 1.25;

            if is_active {
                if on_blade {
                    blend_px(pb, x, y, blade_white, 0.95);
                } else if ang >= blade_ang {
                    // Already swept/snipped zone glows bright hot slash!
                    blend_px(pb, x, y, mix(hot_fill, blade_white, 0.35 * (1.0 - progress)), 0.58 * (1.0 - progress * 0.45));
                } else if arc_edge || side_edge {
                    blend_px(pb, x, y, warn_edge, 0.75);
                } else {
                    blend_px(pb, x, y, warn_fill, 0.22);
                }
            } else if side_edge {
                blend_px(pb, x, y, warn_edge, 0.92 * pulse);
            } else if arc_edge {
                let dash = ((ang * len) as i32) % 4 < 2;
                if dash {
                    blend_px(pb, x, y, warn_edge, 0.85 * pulse);
                }
            } else {
                blend_px(pb, x, y, warn_fill, 0.16 + 0.16 * progress);
            }
        }
    }
}

/// Bobbin: anchored spool pin + taut rose cord + 90° sweep arc preview!
#[allow(clippy::too_many_arguments)]
fn draw_cord_telegraph(
    pb: &mut PixelBuffer,
    spool: Vec2,
    tip: Vec2,
    half: f32,
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
    let rose_edge: Rgb = (255, 166, 206);
    let rose_fill: Rgb = (212, 82, 134);
    let hot_core: Rgb = (255, 244, 250);
    let brass: Rgb = (238, 198, 98);

    // 1. Subtle 90° sweep-arc preview (±45° around `base_dir` at exact `len`) so
    //    the rat immediately sees the swing range during Tell & Active!
    let arc_steps = ((len * 1.4).ceil() as i32).clamp(16, 72);
    for i in 0..=arc_steps {
        let u = i as f32 / arc_steps as f32;
        let ang = -std::f32::consts::FRAC_PI_4 + u * FRAC_PI_2;
        let pt = s + base_dir.rotate(ang) * len;
        if i % 2 == 0 || is_active {
            blend_px(pb, pt.x.round() as i32, pt.y.round() as i32, rose_edge, if is_active { 0.42 } else { 0.65 });
        }
    }

    // 2. Active sweeping cord (from `active_shape`) or Tell initial cord.
    let (live_s, live_t, live_half) = match active_shape {
        Some(MinibossShape::Cord {
            spool: aspool,
            tip: atip,
            half: ahalf,
        }) => (aspool - cam, atip - cam, ahalf),
        _ => (s, tip - cam, half),
    };

    // During Active, paint a soft crimson motion-blur wake behind the sweeping cord.
    if is_active {
        let cur_dir = safe_dir(live_t - live_s, base_dir);
        for trail in 1..=4 {
            let lag_dir = cur_dir.rotate(-(trail as f32) * 0.09);
            let lag_tip = live_s + lag_dir * len;
            let a = 0.34 * (1.0 - trail as f32 * 0.20);
            let steps = (len.ceil() as i32).max(1);
            for k in 0..=steps {
                let q = live_s + (lag_tip - live_s) * (k as f32 / steps as f32);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, rose_fill, a);
            }
        }
    }

    // Main cord band (exact half-thickness `live_half`).
    let ldelta = live_t - live_s;
    let llen = ldelta.len().max(0.5);
    let ldir = ldelta * (1.0 / llen);
    let lperp = ldir.perp();
    let pad = live_half + 1.5;
    let x0 = (live_s.x.min(live_t.x) - pad).floor().max(0.0) as i32;
    let x1 = (live_s.x.max(live_t.x) + pad).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (live_s.y.min(live_t.y) - pad).floor().max(0.0) as i32;
    let y1 = (live_s.y.max(live_t.y) + pad).ceil().min(((pb.h as i32) - 1) as f32) as i32;
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - live_s;
            let along = q.dot(ldir);
            let across = q.dot(lperp).abs();
            if along < 0.0 || along > llen || across > live_half + 0.6 {
                continue;
            }
            if is_active {
                let col = if across < 0.85 {
                    hot_core
                } else if across < live_half - 0.5 {
                    rose_edge
                } else {
                    rose_fill
                };
                blend_px(pb, x, y, col, 0.90);
            } else if across < 0.75 {
                let dash = ((along - progress * 16.0).rem_euclid(5.0)) < 3.0;
                if dash {
                    blend_px(pb, x, y, rose_edge, 0.92 * pulse);
                }
            } else if across >= live_half - 0.6 {
                blend_px(pb, x, y, rose_fill, 0.52 * pulse);
            } else {
                blend_px(pb, x, y, rose_fill, 0.20 + 0.12 * progress);
            }
        }
    }

    // Gleaming brass spool anchor pin at `live_s` and needle weight at `live_t`.
    draw_shaded_disc(pb, live_t.x, live_t.y, 2.0, (255, 242, 178), brass, (132, 92, 34), 0.0);
}

/// Bell Diver: unmistakable expanding acoustic ring with a clearly capped 60° safe gap!
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
    let teal_fill: Rgb = (78, 198, 184);
    let teal_edge: Rgb = (168, 250, 238);
    let hot_core: Rgb = (242, 255, 252);
    let gap_cap: Rgb = (255, 224, 120);

    let (live_r, live_half) = match active_shape {
        Some(MinibossShape::RingGap {
            radius: ar,
            half: ah,
            ..
        }) => (ar, ah),
        _ => (radius, half),
    };

    let max_r = (radius + half).max(live_r + live_half) + 2.0;
    let x0 = (o.x - max_r).floor().max(0.0) as i32;
    let x1 = (o.x + max_r).ceil().min(((pb.w as i32) - 1) as f32) as i32;
    let y0 = (o.y - max_r).floor().max(0.0) as i32;
    let y1 = (o.y + max_r).ceil().min(((pb.h as i32) - 1) as f32) as i32;
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let v = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - o;
            let d = v.len();
            if d < 2.0 || d > max_r {
                continue;
            }
            let dir = v * (1.0 / d);
            let in_gap = dir.dot(gdir) >= cos_gap;
            if in_gap {
                continue;
            }

            // Outer target ring silhouette (always shown softly in Active, bold in Tell).
            let dist_target = (d - radius).abs();
            if dist_target <= half {
                let on_rim = (dist_target - half).abs() < 1.0;
                if is_active {
                    if on_rim {
                        blend_px(pb, x, y, teal_edge, 0.28);
                    }
                } else if on_rim {
                    blend_px(pb, x, y, teal_edge, 0.88 * pulse);
                } else {
                    blend_px(pb, x, y, teal_fill, 0.16 + 0.16 * progress);
                }
            }

            // Live expanding sonic wave ring during Active!
            if is_active {
                let dist_live = (d - live_r).abs();
                if dist_live <= live_half {
                    let core = 1.0 - (dist_live / live_half.max(0.5));
                    let col = if core > 0.58 {
                        hot_core
                    } else if core > 0.22 {
                        teal_edge
                    } else {
                        teal_fill
                    };
                    blend_px(pb, x, y, col, 0.48 + 0.44 * core);
                }
            }
        }
    }

    // Crisp gold gap-edge end-caps marking the exact boundaries of the safe 60° corridor!
    for &sign in &[-1.0_f32, 1.0] {
        let edge_dir = gdir.rotate(sign * gap_half);
        let r_cap = if is_active { live_r } else { radius };
        let p_in = o + edge_dir * (r_cap - half).max(2.0);
        let p_out = o + edge_dir * (r_cap + half);
        pb.line(
            p_in.x.round() as i32,
            p_in.y.round() as i32,
            p_out.x.round() as i32,
            p_out.y.round() as i32,
            gap_cap,
        );
    }
}

/// The Foreman: three sequential 12×12 bureaucratic stamp squares with 1-2-3 pips!
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
    let warn_edge: Rgb = (255, 178, 102);
    let warn_fill: Rgb = (196, 86, 62);
    let hot_fill: Rgb = (255, 86, 76);
    let hot_core: Rgb = (255, 240, 186);
    let spent_col: Rgb = (118, 42, 54);

    // Faint dotted sequence line linking stamp 0 -> 1 -> 2.
    let s0 = centers[0] - cam;
    let s2 = centers[2] - cam;
    let seg = s2 - s0;
    let slen = seg.len();
    if slen > 1.0 {
        let steps = (slen / 3.0).ceil() as i32;
        for i in 0..=steps {
            if i % 2 == 0 {
                let q = s0 + seg * (i as f32 / steps as f32);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, warn_edge, 0.42);
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
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for (idx, &c) in centers.iter().enumerate() {
        let p = c - cam;
        let cx = p.x.round() as i32;
        let cy = p.y.round() as i32;
        if cx + s < 0 || cy + s < 0 || cx - s >= pb.w as i32 || cy - s >= pb.h as i32 {
            continue;
        }
        let is_now = is_active && idx == active_idx;
        let is_done = is_active && idx < active_idx;

        // Fill square interior.
        for dy in -s..s {
            for dx in -s..s {
                let on_border = dx == -s || dx == s - 1 || dy == -s || dy == s - 1;
                if is_now {
                    let col = if on_border { hot_core } else { hot_fill };
                    blend_px(pb, cx + dx, cy + dy, col, if on_border { 0.96 } else { 0.62 });
                } else if is_done {
                    let on_x = (dx - dy).abs() <= 0 || (dx + dy).abs() <= 0;
                    if on_border || on_x {
                        blend_px(pb, cx + dx, cy + dy, spent_col, 0.65);
                    } else {
                        blend_px(pb, cx + dx, cy + dy, spent_col, 0.18);
                    }
                } else if on_border {
                    let corner = dx.abs() >= s - 3 && dy.abs() >= s - 3;
                    let dash = ((dx + dy).rem_euclid(3)) != 0;
                    if corner || dash {
                        blend_px(pb, cx + dx, cy + dy, warn_edge, 0.90 * pulse);
                    }
                } else {
                    blend_px(pb, cx + dx, cy + dy, warn_fill, 0.16 + 0.14 * progress);
                }
            }
        }

        // Sequence pips (`1`, `2`, `3` dots) inside waiting/telling squares, or
        // bold red stamped bars inside the live active square!
        if is_now {
            for dx in -(s - 2)..(s - 2) {
                blend_px(pb, cx + dx, cy - 2, hot_core, 0.90);
                blend_px(pb, cx + dx, cy + 1, hot_core, 0.90);
            }
        } else if !is_done {
            let count = (idx + 1) as i32;
            let start_x = cx - (count - 1);
            for k in 0..count {
                blend_px(pb, start_x + k * 2, cy, hot_core, 0.88);
            }
        }
    }
}

/// Milk Carton: exact oriented 20×32 spill rectangle (fixing the old bowtie!)
/// with pour chevrons in Tell and surging sour-milk foam in Active.
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

    // Exact clockwise corners (fixes the placeholder's bowtie ordering!).
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

    let warn_fill: Rgb = (224, 214, 174);
    let warn_edge: Rgb = (255, 246, 206);
    let milk_body: Rgb = (242, 244, 224);
    let milk_foam: Rgb = (255, 255, 248);
    let curd_yellow: Rgb = (214, 218, 146);
    let hot_rim: Rgb = (255, 214, 128);
    let pulse = 0.72 + 0.28 * (progress * TAU * 3.0).sin();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let v = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
            let along = v.dot(a);
            let across = v.dot(p);
            if along.abs() > half_len + 0.6 || across.abs() > half_w + 0.6 {
                continue;
            }
            let on_border = (along.abs() >= half_len - 0.9) || (across.abs() >= half_w - 0.9);
            if is_active {
                if on_border {
                    blend_px(pb, x, y, hot_rim, 0.92);
                } else {
                    // Surging frothy sour-milk wave scrolling from spout to far edge!
                    let wave = (along * 0.55 - progress * 18.0 + (across * 0.35).sin() * 2.0).sin() * 0.5 + 0.5;
                    let curd = ((along as i32 * 3 + across as i32 * 5).rem_euclid(11)) == 0;
                    let col = if curd {
                        curd_yellow
                    } else if wave > 0.72 {
                        milk_foam
                    } else {
                        milk_body
                    };
                    blend_px(pb, x, y, col, 0.78);
                }
            } else if on_border {
                let dash = ((along + across - progress * 14.0).rem_euclid(5.0)) < 3.0;
                if dash {
                    blend_px(pb, x, y, warn_edge, 0.90 * pulse);
                }
            } else {
                // Pour-direction chevrons guiding the eye away from the spout!
                let chev = ((along - across.abs() * 0.8 - progress * 16.0).rem_euclid(8.0)) < 1.5;
                blend_px(pb, x, y, if chev { warn_edge } else { warn_fill }, if chev { 0.36 * pulse } else { 0.16 + 0.12 * progress });
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
}
