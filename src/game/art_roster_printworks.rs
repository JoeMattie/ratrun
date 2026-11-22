//! Printworks roster bodies (`WAVE4_ROSTERS.md` "Printworks", batch 6b / batch 8 art pass).
//!
//! Seven bodies, one full-detail and one far-LOD draw each, all with the
//! `BodyDraw` signature `art_enemy::bespoke_body`/`bespoke_lod` expect:
//!
//! | kind | silhouette |
//! |---|---|
//! | Silverfish | narrow rippling metallic spindle, 6 scraping feet, three-prong tail |
//! | Erratum Slip | torn L of proof paper tumbling edge over edge with a red erratum mark |
//! | Type Slug | machined lead block with a raised 3D T face, skating a ruled line |
//! | Staple Crawler | stamped steel inverted U on two needle feet, rearing/slamming on strike |
//! | Paper Wasp | folded origami arrow wings over a coiled newsprint abdomen that opens to fire |
//! | Paper Reel | wide rolling spool, kraft end caps, axle holes, flapping loose end (elite: metal flanges) |
//! | Ink Blot | ragged glossy black star in a porcelain halo, flowing and contracting |
//!
//! Render-only: nothing here writes the simulation. Bone paper and metal
//! edges catching register lamps above charcoal slabs is the floor's palette;
//! every body carries a bright specular light edge, a dark silhouette rim, and
//! a rim or eye glint that survives low ambient light, far LOD, and 16-colour
//! folding.
//!
//! The Staple Crawler's footprint ([`draw_staple_tell`]) is drawn from the
//! same two points and radius the simulation hits with
//! (`floor_printworks::STAPLE_FOOT_RADIUS`, `Enemy::w4.trail[0..2]`), and the
//! Ink Blot's travelling endpoint ([`draw_ink_endpoint`]) uses
//! `enemy_wave3::WICK_BURST_RADIUS`.

use std::f32::consts::TAU;

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_printworks as sim;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

// ---- Printworks palette ----------------------------------------------------

/// Shared directional key light (up-left), matching `art_enemy.rs` and `art_rat.rs`.
const LIGHT_X: f32 = -0.55;
const LIGHT_Y: f32 = -0.83;

const PAPER: Rgb = (232, 226, 204);
const PAPER_HI: Rgb = (254, 250, 236);
const PAPER_DARK: Rgb = (146, 138, 118);
const NEWSPRINT: Rgb = (210, 200, 170);
const CREASE: Rgb = (78, 70, 56);

const SILVER: Rgb = (192, 198, 210);
const SILVER_BRISTLE: Rgb = (210, 220, 234);
const STEEL: Rgb = (154, 170, 194);
const STEEL_HI: Rgb = (242, 248, 255);
const STEEL_DARK: Rgb = (52, 62, 84);

const LEAD: Rgb = (112, 118, 132);
const LEAD_HI: Rgb = (224, 232, 244);
const LEAD_DARK: Rgb = (42, 46, 58);

const INK: Rgb = (18, 14, 28);
const INK_SHEEN: Rgb = (92, 76, 144);
const PORCELAIN: Rgb = (246, 242, 230);
const PORCELAIN_DIM: Rgb = (152, 148, 168);

const KRAFT: Rgb = (156, 120, 82);
const KRAFT_HI: Rgb = (210, 170, 122);
const KRAFT_DARK: Rgb = (78, 54, 34);

const PROOF_RED: Rgb = (236, 70, 54);
const ELITE_STRIPE: Rgb = (206, 48, 44);
const ELITE_GOLD: Rgb = (248, 204, 84);
const GLINT_AMBER: Rgb = (255, 206, 86);
const WARN: Rgb = (255, 92, 60);
const WARN_HI: Rgb = (255, 232, 190);

// ---- Local drawing & shading helpers ---------------------------------------

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
fn scale_rgb(c: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (c.0 as f32 * f).min(255.0) as u8,
        (c.1 as f32 * f).min(255.0) as u8,
        (c.2 as f32 * f).min(255.0) as u8,
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

/// The warm hit flash every body shares (same numbers as `art_enemy`).
#[inline]
fn tone(fr: &Frame, c: Rgb) -> Rgb {
    if fr.hit <= 0.0 {
        return c;
    }
    let flash = if fr.hit > 0.6 { (255, 244, 218) } else { (255, 218, 152) };
    mix(c, flash, (0.30 + fr.hit * 0.56).min(0.86))
}

/// Dark silhouette rim colour (warms gently during a hit flash).
#[inline]
fn rim_tone(fr: &Frame) -> Rgb {
    if fr.hit > 0.4 {
        (132, 108, 84)
    } else {
        (8, 8, 14)
    }
}

#[inline]
fn put(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn add_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(
        x,
        y,
        (
            (bg.0 as u16 + c.0 as u16).min(255) as u8,
            (bg.1 as u16 + c.1 as u16).min(255) as u8,
            (bg.2 as u16 + c.2 as u16).min(255) as u8,
        ),
    );
}

/// A contact shadow pixel.
#[inline]
fn shade(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale_rgb(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let steps = dx.abs().max(dy.abs()).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (a.0 as f32 + dx as f32 * t).round() as i32;
        let y = (a.1 as f32 + dy as f32 * t).round() as i32;
        put(pb, x, y, c, alpha);
    }
}

fn disc(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: i32, c: Rgb, alpha: f32) {
    let r2 = rad * rad + rad;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            if dx * dx + dy * dy <= r2 {
                put(pb, cx + dx, cy + dy, c, alpha);
            }
        }
    }
}

/// Soft hovering ground shadow ellipse offset down-right from `fr.pos`.
fn hover_shadow(pb: &mut PixelBuffer, cx: i32, cy: i32, rx: f32, ry: f32, strength: f32) {
    let irx = rx.ceil() as i32;
    let iry = ry.ceil() as i32;
    let inv_rx = 1.0 / rx.max(0.5);
    let inv_ry = 1.0 / ry.max(0.5);
    for dy in -iry..=iry {
        let ny = dy as f32 * inv_ry;
        let ny2 = ny * ny;
        for dx in -irx..=irx {
            let nx = dx as f32 * inv_rx;
            let r2 = nx * nx + ny2;
            if r2 <= 1.0 {
                shade(pb, cx + dx, cy + dy, strength * (1.0 - r2 * 0.5));
            }
        }
    }
}

/// A dotted ring (every other pixel of a 1 px circle), for warned footprints.
fn dotted_ring(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: f32, c: Rgb, alpha: f32) {
    let n = ((rad * 6.3).ceil() as i32).max(8);
    for i in 0..n {
        if i % 2 == 1 {
            continue;
        }
        let a = i as f32 / n as f32 * TAU;
        put(
            pb,
            cx + (a.cos() * rad).round() as i32,
            cy + (a.sin() * rad).round() as i32,
            c,
            alpha,
        );
    }
}

/// A solid 1 px circle at exact radius `rad`, for live strike rims.
fn solid_ring(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: f32, c: Rgb, alpha: f32) {
    let n = ((rad * 8.0).ceil() as i32).max(16);
    for i in 0..n {
        let a = i as f32 / n as f32 * TAU;
        put(
            pb,
            cx + (a.cos() * rad).round() as i32,
            cy + (a.sin() * rad).round() as i32,
            c,
            alpha,
        );
    }
}

/// Local `(forward, lateral)` to a screen pixel.
#[inline]
fn at(fr: &Frame, u: f32, v: f32) -> (i32, i32) {
    (
        (fr.pos.x + fr.f.x * u + fr.p.x * v).round() as i32,
        (fr.pos.y + fr.f.y * u + fr.p.y * v).round() as i32,
    )
}

/// Screen pixel `(x, y)` into local `(u, v)` coordinates.
#[inline]
fn local_uv(fr: &Frame, x: i32, y: i32) -> (f32, f32) {
    let dx = x as f32 - fr.pos.x;
    let dy = y as f32 - fr.pos.y;
    (dx * fr.f.x + dy * fr.f.y, dx * fr.p.x + dy * fr.p.y)
}

/// Tight oriented screen bounding box for local `[u_min, u_max] x [-v_max, v_max]`,
/// expanded by `(pad_x, pad_y)` in `+x/+y` for drop shadows.
#[inline]
fn bbox_oriented(
    fr: &Frame,
    pb: &PixelBuffer,
    u_min: f32,
    u_max: f32,
    v_max: f32,
    pad_x: i32,
    pad_y: i32,
) -> Option<(i32, i32, i32, i32)> {
    let u_mid = 0.5 * (u_min + u_max);
    let hu = 0.5 * (u_max - u_min);
    let cx = fr.pos.x + fr.f.x * u_mid;
    let cy = fr.pos.y + fr.f.y * u_mid;
    let hx = hu * fr.f.x.abs() + v_max * fr.p.x.abs();
    let hy = hu * fr.f.y.abs() + v_max * fr.p.y.abs();
    let x0 = ((cx - hx).floor() as i32).max(0);
    let x1 = ((cx + hx).ceil() as i32 + pad_x).min(pb.w as i32 - 1);
    let y0 = ((cy - hy).floor() as i32).max(0);
    let y1 = ((cy + hy).ceil() as i32 + pad_y).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, x1, y0, y1))
    }
}

/// Projections of the shared up-left key light onto `(fr.f, fr.p)`.
#[inline]
fn light_axes(fr: &Frame) -> (f32, f32) {
    (
        fr.f.x * LIGHT_X + fr.f.y * LIGHT_Y,
        fr.p.x * LIGHT_X + fr.p.y * LIGHT_Y,
    )
}

/// `+1.0` when the body's `+v` flank faces the key light (up-left), else `-1.0`.
#[inline]
fn lit_side(fr: &Frame) -> f32 {
    if fr.p.x * LIGHT_X + fr.p.y * LIGHT_Y >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Two-frame gait clock.
#[inline]
fn step(fr: &Frame, rate: f32) -> bool {
    ((fr.anim * rate) as i32) & 1 == 0
}

#[inline]
fn centre(fr: &Frame) -> (i32, i32) {
    (fr.pos.x.round() as i32, fr.pos.y.round() as i32)
}

#[inline]
fn hp_frac(e: &Enemy) -> f32 {
    if e.max_hp > 0.0 {
        (e.hp / e.max_hp).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Signed distance to an axis-aligned box centered at `(cu, cv)` with half-extents `(hu, hv)`.
#[inline]
fn box_sdf(u: f32, v: f32, cu: f32, cv: f32, hu: f32, hv: f32) -> f32 {
    let du = (u - cu).abs() - hu;
    let dv = (v - cv).abs() - hv;
    let out = du.max(0.0).hypot(dv.max(0.0));
    out + du.max(dv).min(0.0)
}

// ---- Silverfish (A, r 2.5) -------------------------------------------------

/// Narrow metallic spindle with a three-prong tail, six scraping feet, and
/// rippling segmented plates as it streams through press gaps.
pub(crate) fn draw_silverfish(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let (fl, pl) = light_axes(fr);
    let lunge = if e.contact_cd > 0.30 { 0.45 * s } else { 0.0 };

    // Spine ripple: calm near the head, wider S-wave toward the tail.
    let spine = |u: f32| -> f32 {
        let flex = ((2.4 * s - u) / (5.6 * s)).clamp(0.0, 1.0);
        (fr.anim * 0.9 - u * (0.85 / s)).sin() * 0.60 * s * flex
    };

    let leg_c = tone(fr, STEEL);
    let leg_hi = tone(fr, STEEL_HI);
    let leg_dark = tone(fr, STEEL_DARK);

    // 1) Six crisp alternating tripod-gait bristle legs.
    for (i, &u_hip) in [1.45f32, 0.15, -1.25].iter().enumerate() {
        let uh = u_hip * s + lunge;
        let sw = (fr.anim * 1.05 + i as f32 * 2.1).sin() * 0.85 * s;
        let sp = spine(uh);
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let hip = at(fr, uh, sp + side * 0.95 * s);
            let knee = at(fr, uh + swing * 0.45, sp + side * 2.05 * s);
            let foot = at(fr, uh + swing - 0.35 * s, sp + side * 2.75 * s);
            shade(pb, foot.0 + 1, foot.1 + 1, 0.24);
            line(pb, hip, knee, leg_hi, 0.85);
            line(pb, knee, foot, leg_c, 0.90);
            put(pb, foot.0, foot.1, leg_dark, 0.75);
        }
    }

    // 2) Signature three-prong tail (cerci): pale metallic bristles with shadow
    //    backing so they stay crisp over dark charcoal slabs.
    let tail_u = -2.9 * s + lunge;
    let tail_v = spine(tail_u);
    let root = at(fr, tail_u, tail_v);
    let wag = (fr.anim * 0.9 + 2.6).sin() * 0.95 * s;
    let bristle = tone(fr, SILVER_BRISTLE);
    let tips = [
        at(fr, -5.7 * s + lunge, tail_v + wag),
        at(fr, -5.2 * s + lunge, tail_v + 2.15 * s + wag * 0.75),
        at(fr, -5.2 * s + lunge, tail_v - 2.15 * s + wag * 0.75),
    ];
    for &tip in &tips {
        line(pb, (root.0 + 1, root.1 + 1), (tip.0 + 1, tip.1 + 1), (0, 0, 0), 0.32);
        line(pb, root, tip, bristle, 0.92);
        put(pb, tip.0, tip.1, leg_hi, 0.95);
    }

    // 3) Two sweeping forward antennae (flare wide during a contact lunge).
    let ant_wave = (fr.anim * 0.75).sin() * 0.45 * s;
    let flare = if lunge > 0.0 { 0.55 * s } else { 0.0 };
    for side in [-1.0f32, 1.0] {
        let a0 = at(fr, 3.0 * s + lunge, side * 0.55 * s);
        let a1 = at(fr, 4.3 * s + lunge, side * (1.25 * s + flare) + ant_wave * side * 0.5);
        let a2 = at(fr, 5.2 * s + lunge, side * (1.85 * s + flare) + ant_wave * side);
        line(pb, a0, a1, leg_hi, 0.88);
        line(pb, a1, a2, bristle, 0.72);
    }

    // 4) Sculpted watertight silverfish spindle (oriented-bbox pixel pass).
    let Some((x0, x1, y0, y1)) = bbox_oriented(fr, pb, -4.1 * s, 4.4 * s, 2.8 * s, 1, 1) else {
        return;
    };
    let rim_c = rim_tone(fr);
    let seam_c = (44, 52, 70);
    let du_sh = -(fr.f.x + fr.f.y);
    let dv_sh = -(fr.p.x + fr.p.y);

    // Half-width profile: widest at thorax (+1.2*s), tapering cleanly to tail (-3.1*s) and rounded nose (+3.3*s).
    let spindle_sd = |u: f32, v: f32| -> f32 {
        let lu = u - lunge;
        let sp = spine(u);
        let dv = (v - sp).abs();
        let u_min = -3.1 * s;
        let u_max = 3.35 * s;
        let t = ((lu - u_min) / (u_max - u_min)).clamp(0.0, 1.0);
        // Teardrop spindle curve: peaks near t = 0.68 (thorax), tapers smoothly to 0 at tail.
        let profile = (t * (1.0 - t * 0.62)).max(0.0).sqrt() * (1.0 - ((t - 0.70).max(0.0) * 2.75).powi(2)).max(0.0);
        let hw = (2.55 * profile) * s;
        let du = (u_min - lu).max(lu - u_max);
        let d_lat = dv - hw;
        if du > 0.0 {
            du.hypot(d_lat.max(0.0))
        } else {
            d_lat.max(du)
        }
    };

    for y in y0..=y1 {
        let (mut u, mut v) = local_uv(fr, x0, y);
        for x in x0..=x1 {
            let d = spindle_sd(u, v);
            let cover = (0.48 - d * 1.25).clamp(0.0, 1.0);
            if cover > 0.0 {
                let sp = spine(u);
                let nu = ((u - lunge) / (3.3 * s)).clamp(-1.0, 1.0);
                let nv = ((v - sp) / (1.8 * s)).clamp(-1.0, 1.0);
                let dome = (1.0 - (nu * nu * 0.35 + nv * nv * 0.82)).clamp(0.0, 1.0);
                let lit = nu * fl + nv * pl;
                let mut lum = 0.42 + 0.36 * dome + 0.62 * lit;
                // Bright metallic dorsal spine gleam.
                if (v - sp).abs() < 0.48 * s && dome > 0.35 {
                    lum += 0.24;
                }
                let mut col = shade3(STEEL_DARK, SILVER, STEEL_HI, lum);
                // Three crisp transverse silverfish scale grooves.
                let lu = (u - lunge) / s;
                if (lu - 1.5).abs() < 0.24 || (lu - 0.1).abs() < 0.24 || (lu + 1.3).abs() < 0.24 {
                    col = mix(col, seam_c, 0.52);
                }
                let out_c = tone(fr, col);
                if cover >= 1.0 {
                    pb.plot(x, y, out_c);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, rim_c, 0.64 * (1.0 - cover));
                    pb.plot(x, y, mix(rimmed, out_c, cover));
                }
            } else if d < 0.85 * s {
                put(pb, x, y, rim_c, 0.62);
            } else if spindle_sd(u + du_sh, v + dv_sh) < 0.25 * s {
                shade(pb, x, y, 0.32);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 5) Twin warm amber eye glints at the snout so it survives pooled darkness.
    let eye_c = if fr.hit > 0.65 { INK } else { GLINT_AMBER };
    for side in [-1.0f32, 1.0] {
        let ep = at(fr, 2.35 * s + lunge, side * 0.78 * s);
        put(pb, ep.0, ep.1, eye_c, 0.95);
    }
}

pub(crate) fn draw_silverfish_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let lit = lit_side(fr);
    let body = tone(fr, e.kind.color());
    let hi = tone(fr, STEEL_HI);
    let dark = tone(fr, STEEL_DARK);
    let wag = if step(fr, 0.9) { 0.8 * s } else { -0.8 * s };
    // Three-prong tail readable even at far LOD.
    let root = at(fr, -2.5 * s, 0.0);
    let t_mid = at(fr, -4.5 * s, wag * 0.5);
    let t_l = at(fr, -4.0 * s, 1.8 * s + wag * 0.4);
    let t_r = at(fr, -4.0 * s, -1.8 * s + wag * 0.4);
    put(pb, t_mid.0, t_mid.1, tone(fr, SILVER_BRISTLE), 0.95);
    put(pb, t_l.0, t_l.1, tone(fr, SILVER_BRISTLE), 0.9);
    put(pb, t_r.0, t_r.1, tone(fr, SILVER_BRISTLE), 0.9);
    // Tapered spindle core with light & dark edges.
    let nose = at(fr, 3.3 * s, 0.0);
    line(pb, root, nose, body, 1.0);
    line(pb, at(fr, -1.0 * s, 1.0 * s * lit), at(fr, 2.2 * s, 1.0 * s * lit), hi, 1.0);
    line(pb, at(fr, -1.0 * s, -1.0 * s * lit), at(fr, 2.2 * s, -1.0 * s * lit), dark, 1.0);
    put(pb, nose.0, nose.1, GLINT_AMBER, 1.0);
}

// ---- Erratum Slip (B, r 2.0) -----------------------------------------------

/// Torn L of proof paper tumbling edge over edge above its own hover shadow,
/// flashing its pale printed face (with a proofreader's red mark) and its
/// cooler newsprint back.
pub(crate) fn draw_erratumslip(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.0);
    // `anim` runs at the Bat's 22/s buzz, so slow fractions produce a clean tumble.
    let turn = fr.anim * 0.14;
    let flip = (fr.anim * 0.22).cos();
    let bob = (fr.anim * 0.22).sin();
    // Never collapse below 44% width so the defining L-notch stays ≥ 2 px wide.
    let squash = 0.44 + 0.56 * flip.abs();
    let inv_squash = 1.0 / squash;
    let (sn, cs) = turn.sin_cos();

    // 1) Detached hover shadow shifted down-right so it reads as flying.
    let sh_cx = (fr.pos.x + 2.1 * s).round() as i32;
    let sh_cy = (fr.pos.y + (3.1 + 0.45 * bob) * s).round() as i32;
    hover_shadow(pb, sh_cx, sh_cy, (2.2 + 0.7 * squash) * s, 1.5 * s, 0.34);

    // 2) Watertight pixel-rasterized tumbling L-sheet with torn serrated end.
    let reach = (6.2 * s).ceil() as i32;
    let cx = fr.pos.x.round() as i32;
    let cy = fr.pos.y.round() as i32;
    let x0 = (cx - reach).max(0);
    let x1 = (cx + reach).min(pb.w as i32 - 1);
    let y0 = (cy - reach).max(0);
    let y1 = (cy + reach).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let front = flip >= 0.0;
    let (p_sh, p_mid, p_hi) = if front {
        (PAPER_DARK, PAPER, PAPER_HI)
    } else {
        ((118, 114, 102), (186, 182, 168), (228, 224, 210))
    };
    let rim_c = rim_tone(fr);

    // Sheet-space L SDF:
    //   horizontal foot: a in [-3.3*s, +3.1*s], b in [-3.1*s, -0.7*s]
    //   vertical upright: a in [-3.3*s, -0.7*s], b in [-1.0*s, torn_end]
    let sheet_sd = |a: f32, b: f32| -> f32 {
        let foot = box_sdf(a, b, -0.1 * s, -1.9 * s, 3.2 * s, 1.2 * s);
        let torn_bump = (a * (3.8 / s)).sin() * 0.38 * s;
        let b_end = 3.35 * s + torn_bump;
        let mid_b = 0.5 * (-1.0 * s + b_end);
        let half_b = 0.5 * (b_end + 1.0 * s);
        let upright = box_sdf(a, b, -2.0 * s, mid_b, 1.3 * s, half_b);
        foot.min(upright)
    };

    for y in y0..=y1 {
        let dy = y as f32 - fr.pos.y;
        for x in x0..=x1 {
            let dx = x as f32 - fr.pos.x;
            // Inverse rotate into tumbling sheet frame.
            let rx = dx * cs + dy * sn;
            let ry = -dx * sn + dy * cs;
            let a = rx;
            let b = ry * inv_squash;
            // Scale SDF back to approximate screen pixels along the squashed axis.
            let d = sheet_sd(a, b) * (0.65 + 0.35 * squash);
            let cover = (0.48 - d * 1.25).clamp(0.0, 1.0);
            if cover > 0.0 {
                let lit = ((dx * LIGHT_X + dy * LIGHT_Y) / (4.5 * s)).clamp(-1.0, 1.0);
                // Catch a bright specular fold edge whenever the sheet tilts edge-on.
                let sheen = (1.0 - flip.abs()) * 0.28;
                let lum = 0.56 + 0.38 * lit + sheen;
                let mut col = shade3(p_sh, p_mid, p_hi, lum);

                // Bright folded paper edge on the top/left border.
                if d > -0.85 * s && lit > -0.1 {
                    col = mix(col, p_hi, 0.72);
                } else if front {
                    // Printed proof lines on the bars + crimson proofreader's erratum mark at the corner!
                    let on_corner = a > -2.6 * s && a < -1.1 * s && b > -2.5 * s && b < -1.1 * s;
                    let on_foot_line = a > -0.3 * s && a < 2.4 * s && (b + 1.9 * s).abs() < 0.42 * s;
                    let on_up_line = b > 0.1 * s && b < 2.5 * s && (a + 2.0 * s).abs() < 0.42 * s;
                    if on_corner {
                        col = if lit > 0.2 { mix(PROOF_RED, WARN_HI, 0.32) } else { PROOF_RED };
                    } else if on_foot_line || on_up_line {
                        col = mix(col, CREASE, 0.72);
                    }
                } else if a > -2.4 * s && a < 1.8 * s && (b + 1.9 * s).abs() < 0.38 * s {
                    // Faint ink bleed-through on the back face.
                    col = mix(col, CREASE, 0.32);
                }

                let out_c = tone(fr, col);
                if cover >= 1.0 {
                    pb.plot(x, y, out_c);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, rim_c, 0.66 * (1.0 - cover));
                    pb.plot(x, y, mix(rimmed, out_c, cover));
                }
            } else if d < 0.85 * s {
                put(pb, x, y, rim_c, 0.62);
            }
        }
    }
}

pub(crate) fn draw_erratumslip_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let col = tone(fr, e.kind.color());
    let hi = tone(fr, PAPER_HI);
    let dark = tone(fr, PAPER_DARK);
    let (cx, cy) = centre(fr);
    // Detached hover shadow.
    shade(pb, cx + 2, cy + 3, 0.34);
    shade(pb, cx + 3, cy + 3, 0.25);
    // Crisp 2-px-thick tumbling L in four orientations with a red proof mark at the elbow.
    let q = ((fr.anim * 0.09) as i32).rem_euclid(4);
    let (dx, dy) = match q {
        0 => (1, 1),
        1 => (-1, 1),
        2 => (-1, -1),
        _ => (1, -1),
    };
    // Horizontal bar (3x2)
    for ix in 0..=2 {
        put(pb, cx + ix * dx, cy, if ix == 2 { hi } else { col }, 1.0);
        put(pb, cx + ix * dx, cy + dy, dark, 1.0);
    }
    // Vertical bar (2x4)
    for iy in 1..=3 {
        put(pb, cx, cy + iy * dy, col, 1.0);
        put(pb, cx + dx, cy + iy * dy, if iy == 3 { dark } else { hi }, 1.0);
    }
    // Proofreader's red erratum mark at the elbow.
    put(pb, cx, cy, tone(fr, PROOF_RED), 1.0);
}

// ---- Type Slug (C, r 4.0) --------------------------------------------------

/// Heavy cast-lead Linotype block with a raised 3D T face and side nick,
/// skating along a marching ruled lead line.
pub(crate) fn draw_typeslug(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 4.0);
    let (fl, pl) = light_axes(fr);
    let lit_v = lit_side(fr);
    let swiping = e.contact_cd > 0.30;
    let lunge = if swiping { 0.85 * s } else { 0.0 };
    let (hu, hv) = (4.0 * s, 3.1 * s);

    // 1) Ruled skate track behind the slug: crisp marching silver-chalk dashes
    //    with shadow edge, plus tiny skating sparks off the back corners.
    let phase = (fr.anim * 0.85).rem_euclid(2.6) * s;
    let rule_hi = tone(fr, (176, 186, 204));
    let mut u = -hu - 0.9 * s - phase;
    while u > -hu - 5.8 * s {
        let a = at(fr, u, 0.0);
        let b = at(fr, (u - 1.35 * s).max(-hu - 6.0 * s), 0.0);
        line(pb, (a.0 + 1, a.1 + 1), (b.0 + 1, b.1 + 1), (0, 0, 0), 0.38);
        line(pb, a, b, rule_hi, 0.78);
        u -= 2.6 * s;
    }
    // Tiny metallic friction sparks kicking off the trailing heels while skating.
    let spark_side = if step(fr, 1.1) { 1.0 } else { -1.0 };
    let sp1 = at(fr, -hu - 1.1 * s, spark_side * 2.1 * s);
    let sp2 = at(fr, -hu - 2.1 * s, -spark_side * 1.5 * s);
    put(pb, sp1.0, sp1.1, GLINT_AMBER, 0.85);
    put(pb, sp2.0, sp2.1, PAPER_HI, 0.65);

    // 2) Watertight oriented-bbox cast-lead block with machined bevels & raised T.
    let Some((x0, x1, y0, y1)) = bbox_oriented(fr, pb, -hu - 1.3 * s, hu + 2.2 * s, hv + 1.4 * s, 2, 2) else {
        return;
    };
    let rim_c = rim_tone(fr);
    let du_sh = -1.5 * (fr.f.x + fr.f.y);
    let dv_sh = -1.5 * (fr.p.x + fr.p.y);

    // Block SDF with a genuine letterpress side-nick notch carved on the `+v` flank.
    let slug_sd = |u: f32, v: f32| -> f32 {
        let lu = u - lunge;
        let base = box_sdf(lu, v, 0.05 * s, 0.0, hu, hv);
        let nick = (lu + 1.5 * s).hypot(v - (hv + 0.15 * s)) - 0.72 * s;
        base.max(-nick)
    };

    // Raised T letter SDF (crossbar near nose, stem down center, serif caps).
    let letter_t_sd = |u: f32, v: f32| -> f32 {
        let lu = u - lunge;
        let bar = box_sdf(lu, v, 2.05 * s, 0.0, 0.68 * s, 2.05 * s);
        let stem = box_sdf(lu, v, -0.35 * s, 0.0, 2.15 * s, 0.62 * s);
        let foot_serif = box_sdf(lu, v, -2.25 * s, 0.0, 0.35 * s, 1.05 * s);
        bar.min(stem).min(foot_serif)
    };

    for y in y0..=y1 {
        let (mut u, mut v) = local_uv(fr, x0, y);
        for x in x0..=x1 {
            let d = slug_sd(u, v);
            let cover = (0.48 - d * 1.25).clamp(0.0, 1.0);
            if cover > 0.0 {
                let lu = u - lunge;
                let nu = (lu / hu).clamp(-1.0, 1.0);
                let nv = (v / hv).clamp(-1.0, 1.0);
                let lit = nu * fl + nv * pl;
                let mut lum = 0.44 + 0.42 * lit;

                // Machined 45-degree edge bevels around the cast lead block.
                let edge_u = hu - lu.abs();
                let edge_v = hv - v.abs();
                let on_bevel = edge_u < 0.92 * s || edge_v < 0.92 * s;
                if on_bevel {
                    let b_lit = if edge_u < edge_v {
                        lu.signum() * fl
                    } else {
                        v.signum() * pl
                    };
                    lum = if b_lit >= 0.0 {
                        0.88 + 0.24 * b_lit
                    } else {
                        0.14 + 0.18 * (1.0 + b_lit)
                    };
                    // Extra flash along the leading ram edge on contact lunge.
                    if swiping && lu > hu - 1.1 * s {
                        lum = 1.15;
                    }
                } else if ((lu / s) * 2.0).sin() > 0.72 {
                    // Subtle cast-lead milling groove across the recessed shoulder.
                    lum -= 0.08;
                }

                let mut col = shade3(LEAD_DARK, LEAD, LEAD_HI, lum);

                // Raised 3D T relief: drop shadow tucked below-right, bright bone-silver face on top!
                let d_t = letter_t_sd(u, v);
                if d_t <= 0.12 * s {
                    let t_lit = 0.72 + 0.36 * lit;
                    col = shade3(LEAD_HI, PAPER, PAPER_HI, t_lit);
                } else {
                    // Offset shadow opposite to light so the T visibly stands proud of the block.
                    let d_t_sh = letter_t_sd(u + fl * 0.75 * s, v + pl * 0.75 * s);
                    if d_t_sh <= 0.15 * s {
                        col = mix(col, INK, 0.78);
                    }
                }

                let out_c = tone(fr, col);
                if cover >= 1.0 {
                    pb.plot(x, y, out_c);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, rim_c, 0.72 * (1.0 - cover));
                    pb.plot(x, y, mix(rimmed, out_c, cover));
                }
            } else if d < 0.92 * s {
                put(pb, x, y, rim_c, 0.70);
            } else if slug_sd(u + du_sh, v + dv_sh) < 0.35 * s {
                shade(pb, x, y, 0.36);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Twin brass-amber leading register pips (flare hot white on contact lunge).
    let pip_c = if fr.hit > 0.65 {
        INK
    } else if swiping {
        WARN_HI
    } else {
        GLINT_AMBER
    };
    for side in [-1.0f32, 1.0] {
        let p = at(fr, 3.35 * s + lunge, side * 2.15 * s);
        put(pb, p.0, p.1, pip_c, 0.96);
    }
    // Specular glint on the lit leading corner.
    let corner = at(fr, hu + lunge, hv * lit_v);
    put(pb, corner.0, corner.1, tone(fr, STEEL_HI), 0.9);
}

pub(crate) fn draw_typeslug_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 4.0);
    let lit = lit_side(fr);
    let body = tone(fr, e.kind.color());
    let hi = tone(fr, LEAD_HI);
    let dark = tone(fr, LEAD_DARK);
    // Compact watertight oriented lead block.
    let (hu, hv) = (2.6 * s, 2.0 * s);
    if let Some((x0, x1, y0, y1)) = bbox_oriented(fr, pb, -hu - 0.5, hu + 0.5, hv + 0.5, 0, 0) {
        for y in y0..=y1 {
            let (mut u, mut v) = local_uv(fr, x0, y);
            for x in x0..=x1 {
                if u.abs() <= hu && v.abs() <= hv {
                    let c = if v * lit > hv - 0.85 * s || u > hu - 0.8 * s {
                        hi
                    } else if v * lit < -hv + 0.85 * s || u < -hu + 0.8 * s {
                        dark
                    } else {
                        body
                    };
                    pb.plot(x, y, c);
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }
    // Crisp bright T on the face + trailing rule dash.
    let t_col = tone(fr, PAPER_HI);
    line(pb, at(fr, 1.2 * s, -1.2 * s), at(fr, 1.2 * s, 1.2 * s), t_col, 1.0);
    line(pb, at(fr, 1.2 * s, 0.0), at(fr, -1.4 * s, 0.0), t_col, 1.0);
    let r0 = at(fr, -3.5 * s, 0.0);
    let r1 = at(fr, -4.7 * s, 0.0);
    line(pb, r0, r1, tone(fr, LEAD_HI), 0.7);
}

// ---- Staple Crawler (I, r 2.0) ---------------------------------------------

/// The crawler's warned footprint: two radius-4 holes astride the locked rat.
///
/// Reads the same points and radius the simulation hits with. Safe to call
/// for any body: it draws nothing unless a Staple Crawler is in its tell,
/// strike or the first quarter-second of its rest. Stamped on the hostile
/// layer (after the lighting pass) through `art_enemy::draw_w3_tells`.
pub fn draw_staple_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if e.kind != crate::game::enemy::EnemyKind::StapleCrawler || !e.alive() {
        return;
    }
    let phase = e.w4.phase;
    if phase == sim::STAPLE_PHASE_WALK {
        return;
    }
    let rad = sim::STAPLE_FOOT_RADIUS;
    for &foot in &e.w4.trail[..2] {
        let p = foot - cam;
        let (cx, cy) = (p.x.round() as i32, p.y.round() as i32);
        match phase {
            sim::STAPLE_PHASE_TELL => {
                // Dotted rim at the true radius, a growing fill, and a tight
                // puncture crosshair at the centre as the needle descends.
                let k = 1.0 - (e.w4.t / sim::STAPLE_TELL_SECS).clamp(0.0, 1.0);
                dotted_ring(pb, cx, cy, rad, WARN, 0.96);
                let core = (rad * k).floor() as i32;
                if core >= 1 {
                    disc(pb, cx, cy, core - 1, WARN, 0.34 + 0.36 * k);
                }
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    put(pb, cx + dx, cy + dy, WARN, 0.55 + 0.35 * k);
                }
                put(pb, cx, cy, WARN_HI, 1.0);
            }
            sim::STAPLE_PHASE_STRIKE => {
                // Live: the whole hole at the exact radius, solid hot rim and
                // the driven steel needle point.
                disc(pb, cx, cy, rad as i32 - 1, WARN, 0.88);
                solid_ring(pb, cx, cy, rad, WARN_HI, 1.0);
                line(pb, (cx, cy - 2), (cx, cy + 1), STEEL_HI, 1.0);
                put(pb, cx - 1, cy - 1, STEEL, 0.85);
                put(pb, cx, cy + 2, STEEL_DARK, 1.0);
            }
            _ => {
                // Rest: a spent puncture mark that fades (cosmetic, no hazard).
                let age = sim::STAPLE_REST_SECS - e.w4.t;
                if age < 0.25 {
                    let a = 0.65 * (1.0 - age / 0.25);
                    disc(pb, cx, cy, 1, STEEL_DARK, a);
                    put(pb, cx, cy, INK, a);
                }
            }
        }
    }
}

/// Stamped-steel inverted U on two needle feet, drawn upright in screen space
/// so the silhouette never collapses to a bar.
///
/// - Walk: 4-phase mechanical stride with alternating lifted feet and gentle crown rock.
/// - Tell: rears high, splays both needle tips wide, crown rivets glow hot, jitters before slam.
/// - Strike: crown slammed low, needles punched deep with impact sparks at both feet.
/// - Rest: tilted and stuck in the floor, muted steel while exposed.
pub(crate) fn draw_staplecrawler(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.0);
    let (cx, cy) = centre(fr);
    let phase = e.w4.phase;
    let half = (4.5 * s).round().max(3.0) as i32;
    let ground = cy + (3.5 * s).round() as i32;

    let (crown_h, lift_l, lift_r, splay, tilt, stride_l, stride_r, jitter_x, tell_k) = match phase {
        sim::STAPLE_PHASE_TELL => {
            let k = 1.0 - (e.w4.t / sim::STAPLE_TELL_SECS).clamp(0.0, 1.0);
            let h = 6 + (2.0 * k).round() as i32;
            let lift = 3 + if k > 0.55 { 1 } else { 0 };
            let sp = 1 + if k > 0.45 { 1 } else { 0 };
            let jx = if k > 0.68 {
                if ((e.w4.warn_age * 45.0) as i32) & 1 == 0 { 1 } else { -1 }
            } else {
                0
            };
            (h, lift, lift, sp, 0, 0, 0, jx, k)
        }
        sim::STAPLE_PHASE_STRIKE => (3, 0, 0, 0, 0, 0, 0, 0, 1.0),
        sim::STAPLE_PHASE_REST => {
            // Tiny tug in the last 0.25 s before pulling free.
            let pry = if e.w4.t < 0.25 && ((e.w4.t * 24.0) as i32) & 1 == 0 { -1 } else { 0 };
            (4 - pry, 0, 0, -1, 1, 0, 0, 0, 0.0)
        }
        _ => {
            // Smooth 4-beat alternating needle-foot march.
            let sw = (fr.anim * 0.78).sin();
            if sw > 0.28 {
                (6, 3, 0, 0, -1, 1, 0, 0, 0.0)
            } else if sw < -0.28 {
                (6, 0, 3, 0, 1, 0, -1, 0, 0.0)
            } else {
                (5, 1, 1, 0, 0, 0, 0, 0, 0.0)
            }
        }
    };

    let bcx = cx + jitter_x;
    let top = ground - (crown_h as f32 * s).round().max(2.0) as i32;
    let dim = phase == sim::STAPLE_PHASE_REST;
    let steel = tone(fr, if dim { LEAD } else { STEEL });
    let hi = tone(fr, if dim { LEAD_HI } else { STEEL_HI });
    let dark = tone(fr, if dim { LEAD_DARK } else { STEEL_DARK });
    let rim_c = rim_tone(fr);

    // 1) Ground shadow beneath the span and deeper contact shadow under planted feet.
    line(pb, (bcx - half + 1, ground + 1), (bcx + half + 1, ground + 1), (0, 0, 0), 0.28);
    if lift_l == 0 {
        shade(pb, bcx - half - splay + 1, ground + 1, 0.48);
        shade(pb, bcx - half - splay, ground + 1, 0.42);
    }
    if lift_r == 0 {
        shade(pb, bcx + half + splay + 1, ground + 1, 0.48);
        shade(pb, bcx + half + splay, ground + 1, 0.42);
    }

    // 2) Two chunky stamped-steel legs tapering into gleaming razor needle feet.
    let legs = [(-1i32, lift_l, stride_l), (1i32, lift_r, stride_r)];
    for (side, lift, stride) in legs {
        let x_outer = bcx + side * half;
        let x_inner = x_outer - side;
        let x_foot = x_outer + side * splay + stride;
        let y_top = top + if side > 0 { tilt } else { 0 };
        let y_foot = ground - lift;
        let y_knee = y_foot - 1;

        // Outer & inner dark silhouette rim so the wire staple reads solidly.
        line(pb, (x_outer + side, y_top), (x_foot + side, y_knee), rim_c, 0.68);
        line(pb, (x_inner - side, y_top + 2), (x_foot - side, y_knee), rim_c, 0.55);

        // 2-px-wide upper shank (outer bright bevel, inner dark steel) tapering to 1-px needle.
        line(pb, (x_outer, y_top), (x_foot, y_knee), if side < 0 { hi } else { steel }, 1.0);
        line(pb, (x_inner, y_top + 1), (x_foot - side, (y_knee - 1).max(y_top + 1)), dark, 0.95);

        // Razor needle tip: glints hot when raised in tell, polished steel in walk, buried on slam.
        let tip_c = if phase == sim::STAPLE_PHASE_TELL {
            mix(STEEL_HI, WARN_HI, tell_k)
        } else if lift > 0 {
            hi
        } else {
            dark
        };
        put(pb, x_foot, y_foot, tip_c, 1.0);

        // Slam impact sparks right at ground entry during STRIKE!
        if phase == sim::STAPLE_PHASE_STRIKE {
            put(pb, x_foot - side, ground, WARN, 0.92);
            put(pb, x_foot + side, ground - 1, WARN_HI, 0.95);
        }
    }

    // 3) 2-px-thick horizontal stamped-steel crown bar with dark outline rim.
    let y_l = top;
    let y_r = top + tilt;
    line(pb, (bcx - half, y_l - 1), (bcx + half, y_r - 1), rim_c, 0.68);
    line(pb, (bcx - half + 1, y_l + 2), (bcx + half - 1, y_r + 2), rim_c, 0.62);
    // Gleaming top chrome bevel + brushed mid bar + chamfered shoulder corners.
    line(pb, (bcx - half, y_l), (bcx + half, y_r), hi, 1.0);
    line(pb, (bcx - half + 1, y_l + 1), (bcx + half - 1, y_r + 1), steel, 1.0);
    // Bent-wire corner highlights.
    put(pb, bcx - half, y_l, tone(fr, PAPER_HI), 1.0);
    put(pb, bcx + half, y_r, hi, 1.0);

    // 4) Central crown rivet / threat eye pips (survive low ambient & signal tell!).
    let mid_y = (y_l + y_r) / 2;
    if phase == sim::STAPLE_PHASE_TELL || phase == sim::STAPLE_PHASE_STRIKE {
        let eye = mix(WARN, WARN_HI, if phase == sim::STAPLE_PHASE_STRIKE { 0.85 } else { tell_k });
        put(pb, bcx - 1, mid_y + 1, eye, 1.0);
        put(pb, bcx + 1, mid_y + 1, eye, 1.0);
        put(pb, bcx, mid_y, WARN_HI, 0.95);
    } else if !dim {
        let eye = if fr.hit > 0.65 { INK } else { GLINT_AMBER };
        put(pb, bcx - 1, mid_y + 1, eye, 0.92);
        put(pb, bcx + 1, mid_y + 1, eye, 0.92);
    } else {
        put(pb, bcx, mid_y + 1, dark, 0.9);
    }
}

pub(crate) fn draw_staplecrawler_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let col = tone(fr, e.kind.color());
    let hi = tone(fr, STEEL_HI);
    let dark = tone(fr, STEEL_DARK);
    let phase = e.w4.phase;
    let top_y = match phase {
        sim::STAPLE_PHASE_TELL => cy - 3,
        sim::STAPLE_PHASE_STRIKE => cy,
        _ => cy - 2,
    };
    let splay = if phase == sim::STAPLE_PHASE_TELL { 1 } else { 0 };
    // Bold 2-px crown bar.
    line(pb, (cx - 3, top_y), (cx + 3, top_y), hi, 1.0);
    line(pb, (cx - 3, top_y + 1), (cx + 3, top_y + 1), col, 1.0);
    let (l_len, r_len) = match phase {
        sim::STAPLE_PHASE_TELL => (1, 1),
        sim::STAPLE_PHASE_STRIKE => (2, 2),
        _ if step(fr, 0.78) => (3, 1),
        _ => (1, 3),
    };
    line(pb, (cx - 3 - splay, top_y + 1), (cx - 3 - splay, top_y + 1 + l_len), col, 1.0);
    line(pb, (cx + 3 + splay, top_y + 1), (cx + 3 + splay, top_y + 1 + r_len), dark, 1.0);
    if phase == sim::STAPLE_PHASE_TELL || phase == sim::STAPLE_PHASE_STRIKE {
        put(pb, cx, top_y, WARN, 1.0);
    } else {
        put(pb, cx, top_y + 1, GLINT_AMBER, 0.9);
    }
}

// ---- Paper Wasp (R, r 3.0) -------------------------------------------------

/// Folded origami arrow wings over a coiled newsprint abdomen. During the
/// Spitter's 0.75 s shot wind-up the rolled abdomen uncoils straight, flares
/// open, and glows hot vermilion-amber; after firing it recoils sharply.
pub(crate) fn draw_paperwasp(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 3.0);
    let (fl, pl) = light_axes(fr);
    let lit_v = lit_side(fr);

    // Match Spitter's 0.75 s muzzle swell and 0.25 s post-shot recoil (`art_enemy::draw_spitter`).
    let charge = if e.fire_cd <= 0.0 {
        0.84 + 0.16 * (fr.anim * 2.0).sin().abs()
    } else if e.fire_cd < 0.75 {
        ((0.75 - e.fire_cd) * (1.0 / 0.75)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let recoil = if e.fire_cd > 1.95 {
        ((e.fire_cd - 1.95) * 4.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let flap = (fr.anim * 1.45).sin();
    // Wings flutter briskly while strafing, then pull tight & rigid during the shot tell.
    let wing_w = (3.85 + 0.80 * flap * (1.0 - 0.7 * charge) - 0.45 * charge) * s;
    let tail_len = (5.0 + 1.55 * charge - 0.85 * recoil) * s;
    let coil_amp = 0.88 * (1.0 - 0.85 * charge) * s;

    // 1) Detached hover shadow below the hovering dart.
    hover_shadow(
        pb,
        (fr.pos.x + 1.8 * s).round() as i32,
        (fr.pos.y + (2.9 + 0.35 * flap) * s).round() as i32,
        2.7 * s,
        1.7 * s,
        0.30,
    );

    // 2) Watertight oriented-bbox rasterization of BOTH the rolled newsprint
    //    abdomen and the crisp folded origami arrow wings.
    let u_min = -(tail_len + 1.6 * s);
    let u_max = 5.4 * s;
    let v_max = (wing_w + 1.4 * s).max(2.6 * s);
    let Some((x0, x1, y0, y1)) = bbox_oriented(fr, pb, u_min, u_max, v_max, 1, 1) else {
        return;
    };

    let rim_c = rim_tone(fr);
    let nose_u = (4.55 - 0.45 * recoil) * s;
    let wing_back_u = -2.25 * s;

    // Coiled abdomen centreline v(u).
    let tail_spine = |u: f32| -> f32 {
        let t = ((-1.0 * s - u) / (tail_len - 1.0 * s).max(0.5)).clamp(0.0, 1.0);
        (fr.anim * 0.95 + t * 3.8).sin() * coil_amp * t
    };

    // Rolled newsprint tube SDF.
    let tube_sd = |u: f32, v: f32| -> f32 {
        let t = ((-1.0 * s - u) / (tail_len - 1.0 * s).max(0.5)).clamp(0.0, 1.0);
        // Tapered rolled cone that flares wider at the open nozzle tip while charging.
        let rad = (1.30 * (1.0 - 0.42 * t) + 0.52 * charge * t * t) * s;
        let sp = tail_spine(u);
        let dv = (v - sp).abs() - rad;
        let du = (-tail_len - u).max(u + 0.9 * s);
        if du > 0.0 {
            du.hypot(dv.max(0.0))
        } else {
            dv.max(du)
        }
    };

    // Folded paper-airplane delta wing SDF with notched trailing V-tail.
    let wing_sd = |u: f32, v: f32| -> f32 {
        let av = v.abs();
        let span_t = ((nose_u - u) / (nose_u - wing_back_u).max(0.5)).clamp(0.0, 1.0);
        let half_w = wing_w * span_t;
        // Leading swept edges + V-notched trailing cutout (`u_trail` sits at -1.1*s at centre, -2.25*s at tips).
        let u_trail = -1.05 * s - 1.20 * s * (av / wing_w.max(0.5)).clamp(0.0, 1.0);
        let d_lead = (av - half_w) * 0.82;
        let d_nose = u - nose_u;
        let d_back = u_trail - u;
        let d_tip = av - wing_w;
        d_lead.max(d_nose).max(d_back).max(d_tip)
    };

    for y in y0..=y1 {
        let (mut u, mut v) = local_uv(fr, x0, y);
        for x in x0..=x1 {
            let dw = wing_sd(u, v);
            let dt = tube_sd(u, v);
            let d = dw.min(dt);
            let cover = (0.48 - d * 1.25).clamp(0.0, 1.0);
            if cover > 0.0 {
                let col = if dw <= dt + 0.15 * s {
                    // Folded origami arrow wing: crisp 2-facet roof shading split at the dorsal crease!
                    let on_lit_facet = v * lit_v >= 0.0;
                    let crease_dist = v.abs();
                    let mut lum = if on_lit_facet {
                        0.76 + 0.30 * (u / (4.5 * s)) * fl
                    } else {
                        0.32 + 0.22 * (u / (4.5 * s)) * fl
                    };
                    if dw > -0.75 * s && on_lit_facet {
                        lum += 0.18;
                    }
                    let mut c = shade3(CREASE, NEWSPRINT, PAPER_HI, lum);
                    // Razor-sharp folded centre ridge (dark crease + bright adjacent fold).
                    if crease_dist < 0.38 * s {
                        c = mix(c, CREASE, 0.78);
                    } else if crease_dist < 0.95 * s && on_lit_facet {
                        c = mix(c, PAPER_HI, 0.55);
                    } else if u < 1.8 * s && (((u / s) * 2.4).sin() > 0.68) && crease_dist > 1.1 * s {
                        // Subtle newsprint column print lines across the paper wings.
                        c = mix(c, CREASE, 0.35);
                    }
                    c
                } else {
                    // Rolled newsprint abdomen: spiralled paper rings + ink bands.
                    let sp = tail_spine(u);
                    let nv = ((v - sp) / (1.3 * s)).clamp(-1.0, 1.0);
                    let lit = nv * pl + 0.25 * fl;
                    let lum = 0.44 + 0.42 * (1.0 - nv * nv * 0.65) + 0.42 * lit;
                    let mut c = shade3(CREASE, NEWSPRINT, PAPER_HI, lum);
                    // Rolled newsprint spiral bands.
                    let band = ((u / s) * 2.7 + nv * 0.9).sin();
                    if band > 0.42 {
                        c = mix(c, CREASE, 0.62 * (1.0 - 0.35 * charge));
                    }
                    // Warm ember glow creeping up the open paper tube as it charges!
                    if charge > 0.10 && u < -2.6 * s {
                        let tip_t = ((-2.6 * s - u) / (tail_len - 2.6 * s).max(0.5)).clamp(0.0, 1.0);
                        c = mix(c, WARN, charge * tip_t * 0.80);
                    }
                    c
                };

                let out_c = tone(fr, col);
                if cover >= 1.0 {
                    pb.plot(x, y, out_c);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, rim_c, 0.64 * (1.0 - cover));
                    pb.plot(x, y, mix(rimmed, out_c, cover));
                }
            } else if d < 0.85 * s {
                put(pb, x, y, rim_c, 0.60);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }

    // 3) Crisp dart nose glint & fierce amber-orange compound eyes tucked under the fold.
    let nose = at(fr, nose_u, 0.0);
    put(pb, nose.0, nose.1, tone(fr, PAPER_HI), 1.0);
    let eye_c = if fr.hit > 0.65 { INK } else { (255, 118, 46) };
    for side in [-1.0f32, 1.0] {
        let ep = at(fr, 2.25 * s - 0.3 * recoil * s, side * 0.88 * s);
        put(pb, ep.0, ep.1, eye_c, 0.96);
    }

    // 4) Rolled abdomen aperture: dark ink mouth at rest, glowing 4-point
    //    charge flare when opening to shoot, ink puff on recoil.
    let tip = at(fr, -tail_len, tail_spine(-tail_len));
    if charge > 0.08 {
        let orb = mix(WARN, WARN_HI, 0.35 + 0.60 * charge);
        let glow = scale_rgb(orb, 0.45 + 0.50 * charge);
        put(pb, tip.0, tip.1, orb, 1.0);
        if charge > 0.30 {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                add_px(pb, tip.0 + dx, tip.1 + dy, glow);
            }
        }
        if charge > 0.68 {
            let tip_g = scale_rgb(orb, 0.55);
            for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
                add_px(pb, tip.0 + dx, tip.1 + dy, tip_g);
            }
        }
    } else if recoil > 0.15 {
        put(pb, tip.0, tip.1, INK, 0.95);
        let puff = at(fr, -tail_len - 1.1 * s, tail_spine(-tail_len));
        put(pb, puff.0, puff.1, PORCELAIN_DIM, 0.65 * recoil);
    } else {
        put(pb, tip.0, tip.1, INK, 0.95);
    }
}

pub(crate) fn draw_paperwasp_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 3.0);
    let lit = lit_side(fr);
    let paper = tone(fr, e.kind.color());
    let hi = tone(fr, PAPER_HI);
    let dark = tone(fr, CREASE);
    let nose = at(fr, 3.6 * s, 0.0);
    let tail = at(fr, -3.9 * s, 0.0);
    // Sharp paper-dart delta wings with bright lit half and dark shadowed half.
    let w_lit = at(fr, -1.2 * s, 2.7 * s * lit);
    let w_sh = at(fr, -1.2 * s, -2.7 * s * lit);
    line(pb, nose, w_lit, hi, 1.0);
    line(pb, at(fr, 1.2 * s, 0.8 * s * lit), w_lit, paper, 1.0);
    line(pb, nose, w_sh, paper, 1.0);
    line(pb, at(fr, 1.2 * s, -0.8 * s * lit), w_sh, dark, 1.0);
    // Spine & rolled newsprint tail.
    line(pb, tail, nose, dark, 0.95);
    put(pb, nose.0, nose.1, hi, 1.0);
    // Open nozzle telegraph matches full 0.75 s Spitter swell.
    let tip_col = if e.fire_cd < 0.75 { WARN_HI } else { INK };
    put(pb, tail.0, tail.1, tip_col, 1.0);
    if e.fire_cd < 0.45 {
        let g = at(fr, -4.6 * s, 0.0);
        put(pb, g.0, g.1, WARN, 0.95);
    }
}

// ---- Paper Reel (H, r 6.5) -------------------------------------------------

/// Wide heavy paper spool rolling broadside: curved newsprint cylinder with
/// scrolling wound-paper seams, warm kraft end caps with deep black axle
/// bores, and a rippling loose sheet flapping behind.
///
/// The elite wears a pair of proud bolted metal flanges (visible from either
/// side), a thick double crimson-and-gold shoulder stripe, and rotating brass
/// perimeter threat arcs.
pub(crate) fn draw_paperreel(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 6.5);
    let (fl, pl) = light_axes(fr);
    let lunge = if e.contact_cd > 0.30 { 0.65 * s } else { 0.0 };
    let heave = (fr.anim * 0.72).sin() * 0.32 * s;
    let (hu, hv) = (4.3 * s, 6.5 * s);
    let hp = hp_frac(e);

    // 1) Elite threat perimeter: crisp rotating brass-gold corner arcs (matching Brute elites).
    if elite {
        let pulse = 0.55 + 0.45 * (fr.anim * 1.4).sin();
        let aura_r = fr.r + 2.4 * s;
        let gold = scale_rgb(ELITE_GOLD, 0.54 + 0.34 * pulse);
        for i in 0..8 {
            let a0 = i as f32 * (TAU / 8.0) + fr.anim * 0.14;
            let mut last = (i32::MIN, i32::MIN);
            for st in 0..4 {
                let a = a0 + st as f32 * 0.075;
                let tx = (fr.pos.x + a.cos() * aura_r).round() as i32;
                let ty = (fr.pos.y + a.sin() * aura_r).round() as i32;
                if (tx, ty) != last {
                    put(pb, tx, ty, gold, 0.72);
                    last = (tx, ty);
                }
            }
        }
    }

    // 2) Flapping loose paper end unspooling off the back of the cylinder.
    //    Watertight oriented-bbox pass so it never develops diagonal gaps!
    let rim_c = rim_tone(fr);
    if let Some((tx0, tx1, ty0, ty1)) =
        bbox_oriented(fr, pb, -hu - 4.2 * s, -hu + 0.8 * s, 4.2 * s, 1, 2)
    {
        for y in ty0..=ty1 {
            let (mut u, mut v) = local_uv(fr, tx0, y);
            for x in tx0..=tx1 {
                let lu = u - lunge;
                let lv = v - heave;
                // Travelling flap wave along the loose paper tail.
                let wave = (fr.anim * 1.15 + lv * (0.55 / s)).sin() * 0.95 * s;
                let back_edge = -hu - 3.05 * s + wave;
                let half_w = 3.15 * s;
                let du = (back_edge - lu).max(lu + hu - 0.25 * s);
                let dv = lv.abs() - half_w;
                let d_tail = du.max(dv);
                if d_tail <= 0.15 * s {
                    let t_back = ((-hu - lu) / (3.4 * s)).clamp(0.0, 1.0);
                    let ripple_lit = (fr.anim * 1.15 + lv * (0.55 / s)).cos() * 0.24;
                    let lum = 0.62 - 0.18 * t_back + 0.26 * fl + ripple_lit;
                    let mut c = shade3(PAPER_DARK, PAPER, PAPER_HI, lum);
                    if (lv / s).abs() < 2.2 && ((lu / s) * 2.5).sin() > 0.55 {
                        c = mix(c, CREASE, 0.36);
                    }
                    put(pb, x, y, tone(fr, c), 0.98);
                } else if d_tail < 0.95 * s {
                    put(pb, x, y, rim_c, 0.58);
                } else {
                    // Shadow under the flapping sheet.
                    let su = lu - (fr.f.x + fr.f.y);
                    let sv = lv - (fr.p.x + fr.p.y);
                    let s_back = -hu - 3.05 * s + (fr.anim * 1.15 + sv * (0.55 / s)).sin() * 0.95 * s;
                    if su >= s_back && su <= -hu && sv.abs() <= half_w {
                        shade(pb, x, y, 0.28);
                    }
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }

    // 3) Watertight 3D paper cylinder + end caps (or elite bolted metal flanges!).
    let flange_hu = if elite { hu + 1.35 * s } else { hu };
    let flange_hv = if elite { hv + 1.35 * s } else { hv };
    let cap_inner = if elite { hv - 0.45 * s } else { hv - 1.45 * s };
    let Some((x0, x1, y0, y1)) = bbox_oriented(
        fr,
        pb,
        -flange_hu - 1.2 * s,
        flange_hu + 1.8 * s,
        flange_hv + 1.2 * s,
        2,
        2,
    ) else {
        return;
    };

    let roll_phase = fr.anim * 0.95;
    let du_sh = -1.6 * (fr.f.x + fr.f.y);
    let dv_sh = -1.6 * (fr.p.x + fr.p.y);

    // Combined SDF for the barrel and the two end caps / proud metal flanges.
    let spool_sd = |u: f32, v: f32| -> (f32, bool) {
        let lu = u - lunge;
        let lv = v - heave;
        let barrel = box_sdf(lu, lv, 0.0, 0.0, hu, hv);
        if elite {
            let f_mid = 0.5 * (cap_inner + flange_hv);
            let f_half = 0.5 * (flange_hv - cap_inner);
            let flange = box_sdf(lu, lv.abs(), 0.0, f_mid, flange_hu, f_half);
            let is_flange = flange <= barrel + 0.08 * s && lv.abs() >= cap_inner;
            (barrel.min(flange), is_flange)
        } else {
            (barrel, lv.abs() >= cap_inner)
        }
    };

    for y in y0..=y1 {
        let (mut u, mut v) = local_uv(fr, x0, y);
        for x in x0..=x1 {
            let (d, is_cap) = spool_sd(u, v);
            let cover = (0.48 - d * 1.25).clamp(0.0, 1.0);
            if cover > 0.0 {
                let lu = u - lunge;
                let lv = v - heave;
                let av = lv.abs();
                let col = if is_cap {
                    // End cap (warm kraft) or proud elite metal flange, both with deep circular axle bore.
                    let hub_v = if elite { hv + 0.45 * s } else { hv - 0.65 * s };
                    let axle_d = lu.hypot((av - hub_v) * 1.25);
                    if elite {
                        let nu = (lu / flange_hu).clamp(-1.0, 1.0);
                        let dome = (1.0 - nu * nu * 0.72).max(0.0).sqrt();
                        let lit = nu * fl + lv.signum() * 0.35 * pl;
                        let mut c = shade3(STEEL_DARK, STEEL, STEEL_HI, 0.42 + 0.36 * dome + 0.56 * lit);
                        // Gleaming outer machined flange rim.
                        if av > flange_hv - 0.72 * s || lu.abs() > flange_hu - 0.75 * s {
                            c = if lit >= -0.15 { STEEL_HI } else { STEEL_DARK };
                        }
                        // Dark recessed circular axle bore with steel collar ring.
                        if axle_d < 1.20 * s {
                            c = INK;
                        } else if axle_d < 1.85 * s {
                            c = if lit >= 0.0 { STEEL_HI } else { STEEL_DARK };
                        } else {
                            // Rotating brass flange bolts visible from either side!
                            let b_u = (roll_phase * 0.9).sin() * 3.1 * s;
                            let b1 = (lu - b_u).hypot(av - hub_v);
                            let b2 = (lu + b_u).hypot(av - hub_v);
                            if b1 < 0.72 * s || b2 < 0.72 * s {
                                c = ELITE_GOLD;
                            }
                        }
                        c
                    } else {
                        let nu = (lu / hu).clamp(-1.0, 1.0);
                        let dome = (1.0 - nu * nu * 0.75).max(0.0).sqrt();
                        let lit = nu * fl + lv.signum() * 0.35 * pl;
                        let mut c = shade3(KRAFT_DARK, KRAFT, KRAFT_HI, 0.38 + 0.36 * dome + 0.48 * lit);
                        if axle_d < 1.15 * s {
                            c = INK;
                        } else if axle_d < 1.75 * s {
                            c = if lit >= 0.0 { KRAFT_HI } else { KRAFT_DARK };
                        }
                        c
                    }
                } else {
                    // Curved cylindrical newsprint roll!
                    let nu = (lu / hu).clamp(-1.0, 1.0);
                    let dome = (1.0 - nu * nu * 0.88).max(0.0).sqrt();
                    let lit = nu * fl;
                    let mut lum = 0.24 + 0.58 * dome + 0.48 * lit;
                    if dome > 0.78 && lit > 0.05 {
                        lum += 0.22;
                    }
                    let mut c = shade3(PAPER_DARK, PAPER, PAPER_HI, lum);

                    // Rolling wound-paper lap seam & faint print columns that visibly turn!
                    let scroll = (lu / s + 4.5 + roll_phase).rem_euclid(3.2);
                    if scroll < 0.65 {
                        c = mix(c, CREASE, 0.42);
                    } else if scroll > 1.5 && scroll < 2.25 && ((lv / s) * 1.6).sin() > 0.35 {
                        c = mix(c, PAPER_DARK, 0.45);
                    }

                    // Elite thick double shoulder stripe (crimson + brass-gold trim) on both shoulders!
                    if elite {
                        let band1 = (av - 2.35 * s).abs() < 0.58 * s;
                        let band2 = (av - 3.85 * s).abs() < 0.58 * s;
                        let trim = (av - 3.10 * s).abs() < 0.36 * s;
                        if band1 || band2 {
                            let s_c = shade3((88, 22, 24), ELITE_STRIPE, (250, 118, 92), lum);
                            c = mix(c, s_c, 0.92);
                        } else if trim {
                            let g_c = shade3((120, 88, 28), ELITE_GOLD, PAPER_HI, lum);
                            c = mix(c, g_c, 0.90);
                        }
                    }

                    // Damage tear across the paper drum when wounded (`hp < 65%`).
                    if hp < 0.65 {
                        let tear = ((lu + 0.6 * s) - (lv - 0.8 * s) * 0.55).abs() < 0.42 * s
                            && lv > -1.2 * s
                            && lv < 2.6 * s;
                        if tear {
                            c = if hp < 0.35 { INK } else { CREASE };
                        }
                    }

                    // Leading-edge slam flash.
                    if lunge > 0.0 && lu > hu - 0.9 * s {
                        c = mix(c, PAPER_HI, 0.65);
                    }
                    c
                };

                let out_c = tone(fr, col);
                if cover >= 1.0 {
                    pb.plot(x, y, out_c);
                } else {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, rim_c, 0.72 * (1.0 - cover));
                    pb.plot(x, y, mix(rimmed, out_c, cover));
                }
            } else if d < 0.92 * s {
                put(pb, x, y, rim_c, 0.70);
            } else if spool_sd(u + du_sh, v + dv_sh).0 < 0.4 * s {
                shade(pb, x, y, 0.38);
            }
            u += fr.f.x;
            v += fr.p.x;
        }
    }
}

pub(crate) fn draw_paperreel_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 6.5);
    let col = tone(fr, e.kind.color());
    let (hu, hv) = (2.3 * s, 4.3 * s);
    let f_hu = if elite { 3.1 * s } else { hu };
    let f_hv = if elite { 5.0 * s } else { hv };

    // Watertight compact spool (~55 px ordinary, ~72 px elite — inside far-LOD budget).
    if let Some((x0, x1, y0, y1)) = bbox_oriented(fr, pb, -f_hu - 0.5, f_hu + 0.5, f_hv + 0.5, 0, 0) {
        for y in y0..=y1 {
            let (mut u, mut v) = local_uv(fr, x0, y);
            for x in x0..=x1 {
                let av = v.abs();
                let in_flange = elite && av >= hv - 0.4 * s && av <= f_hv && u.abs() <= f_hu;
                let in_barrel = av <= hv && u.abs() <= hu;
                if in_flange {
                    let c = if u.abs() < 0.8 * s { INK } else { tone(fr, STEEL_HI) };
                    pb.plot(x, y, c);
                } else if in_barrel {
                    let t = (u / hu).abs();
                    let mut c = if t < 0.35 {
                        tone(fr, PAPER_HI)
                    } else if t > 0.78 {
                        tone(fr, CREASE)
                    } else {
                        mix(col, tone(fr, PAPER), 0.55)
                    };
                    if elite && ((av - 1.6 * s).abs() < 0.55 * s || (av - 2.9 * s).abs() < 0.55 * s) {
                        c = tone(fr, ELITE_STRIPE);
                    } else if !elite && av > hv - 0.85 * s {
                        c = if u.abs() < 0.8 * s { INK } else { tone(fr, KRAFT) };
                    }
                    pb.plot(x, y, c);
                }
                u += fr.f.x;
                v += fr.p.x;
            }
        }
    }
    // Tiny flapping loose-end tab behind the roll.
    let flap = if step(fr, 0.5) { 0.8 * s } else { -0.8 * s };
    let t0 = at(fr, -hu - 0.8 * s, flap);
    put(pb, t0.0, t0.1, tone(fr, PAPER), 1.0);
}

// ---- Ink Blot (III, r 2.5) -------------------------------------------------

/// The blot's edge radius at angle `a`: a ragged five-armed star with jagged
/// secondary lobes that flows outward and contracts as it glides.
#[inline]
fn blot_radius(a: f32, anim: f32, s: f32, dir_dot: f32) -> f32 {
    let pulse = 0.78 + 0.28 * (anim * 0.42).sin();
    let spin = anim * 0.14;
    let arms = (a * 5.0 + spin).cos() * 0.38
        + (a * 3.0 - spin * 1.45).sin() * 0.16
        + (a * 7.0 + spin * 2.1).cos() * 0.08;
    let glide = 1.0 + 0.12 * dir_dot;
    3.55 * s * pulse * glide * (1.0 + arms)
}

/// Ragged glossy black star framed by a crisp porcelain outline, with a wet
/// indigo sheen, twin porcelain highlight glints, and satellite ink droplets.
pub(crate) fn draw_inkblot(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let (cx, cy) = centre(fr);
    let reach = (8.5 * s).ceil() as i32 + 1;
    let ink = tone(fr, INK);
    let sheen = tone(fr, mix(INK_SHEEN, e.kind.color(), 0.35));
    let rim_hi = tone(fr, PORCELAIN);
    let rim_lo = tone(fr, PORCELAIN_DIM);
    let hp = hp_frac(e);
    let volatile = if hp < 0.55 {
        (0.5 + 0.5 * (fr.anim * 2.4).sin()) * (1.0 - hp / 0.55)
    } else {
        0.0
    };

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let fx = dx as f32;
            let fy = dy as f32;
            let d = fx.hypot(fy);
            let a = fy.atan2(fx);
            let dir_dot = if d > 0.01 {
                (fx * fr.f.x + fy * fr.f.y) / d
            } else {
                0.0
            };
            let edge = blot_radius(a, fr.anim, s, dir_dot);
            let lit = fx * LIGHT_X + fy * LIGHT_Y;

            if d <= edge {
                let mut c = ink;
                // Glossy curved wet-ink crescent along the lit upper-left lobe.
                if lit > 0.75 * s && d > edge * 0.32 && d < edge * 0.84 {
                    c = sheen;
                }
                // Volatile crimson heartbeat in the core when close to popping its wick.
                if volatile > 0.05 && d < 1.8 * s {
                    let core_k = (1.0 - d / (1.8 * s)) * volatile;
                    c = mix(c, WARN, core_k * 0.85);
                }
                put(pb, cx + dx, cy + dy, c, 1.0);
            } else if d <= edge + 1.28 {
                // Crisp bone-porcelain halo: bright toward the key light, cooler porcelain away.
                let rim = if lit >= -0.35 { rim_hi } else { rim_lo };
                put(pb, cx + dx, cy + dy, rim, 1.0);
            } else if d <= edge + 2.05 && lit < 0.0 {
                // Subtle drop shadow under the porcelain halo so it separates over pale paper tiles.
                shade(pb, cx + dx, cy + dy, 0.30);
            }
        }
    }

    // Twin glossy porcelain wet-specular "eye" glints (survive low ambient & give creature life).
    let g1 = at(fr, 1.15 * s, -0.95 * s);
    let g2 = at(fr, 1.15 * s, 0.95 * s);
    put(pb, g1.0, g1.1, rim_hi, 0.96);
    put(pb, g2.0, g2.1, if volatile > 0.3 { WARN_HI } else { rim_hi }, 0.90);
    put(pb, cx - 1, cy - 1, rim_hi, 0.72);

    // Satellite ink droplets budding off two star tips when the blot flows wide.
    let flow = (fr.anim * 0.42).sin();
    if flow > 0.15 {
        let spin = fr.anim * 0.14;
        for k in [0usize, 2] {
            let a = (k as f32 * TAU / 5.0) - spin * 0.2;
            let dist = (5.4 + 1.1 * flow) * s;
            let sx = cx + (a.cos() * dist).round() as i32;
            let sy = cy + (a.sin() * dist).round() as i32;
            put(pb, sx, sy, ink, 1.0);
            put(pb, sx - 1, sy, rim_hi, 0.85);
        }
    }
}

pub(crate) fn draw_inkblot_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let wide = step(fr, 0.42);
    let rim = tone(fr, PORCELAIN);
    let rim_dim = tone(fr, PORCELAIN_DIM);
    let ink = tone(fr, INK);
    let arm = if wide { 2 } else { 1 };
    // Crisp ragged star: dark core + porcelain outer tips.
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        put(pb, cx + dx * (arm + 1), cy + dy * (arm + 1), if dx + dy <= 0 { rim } else { rim_dim }, 1.0);
        for k in 1..=arm {
            put(pb, cx + dx * k, cy + dy * k, ink, 1.0);
        }
    }
    // Diagonal shoulders so the star reads as a ragged blob rather than a thin plus.
    for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        put(pb, cx + dx, cy + dy, if wide { ink } else if dx + dy <= 0 { rim } else { rim_dim }, 1.0);
        if wide {
            put(pb, cx + dx * 2, cy + dy * 2, if dx + dy <= 0 { rim } else { rim_dim }, 0.9);
        }
    }
    put(pb, cx, cy, ink, 1.0);
    put(pb, cx - 1, cy - 1, rim, 0.95);
}

/// The Ink Blot's travelling burst: the marked endpoint, shown for the whole
/// fuse. `end` is `InkSlide::end` (world px) and `fuse_frac` runs 1 -> 0.
///
/// A porcelain ring at the true burst radius with a glossy black core that
/// fills as the fuse burns. Stamped by `World::draw_hostile_layer` for every
/// wick that carries an `InkSlide`.
pub fn draw_ink_endpoint(pb: &mut PixelBuffer, end: Vec2, cam: Vec2, fuse_frac: f32) {
    let p = end - cam;
    let (cx, cy) = (p.x.round() as i32, p.y.round() as i32);
    let rad = crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS;
    let k = 1.0 - fuse_frac.clamp(0.0, 1.0);
    // Porcelain perimeter at the exact burst radius (solidifies in the final 25%).
    if k > 0.75 {
        solid_ring(pb, cx, cy, rad, PORCELAIN, 0.96);
    } else {
        dotted_ring(pb, cx, cy, rad, PORCELAIN, 0.95);
    }
    // Expanding black ink puddle filling toward the porcelain ring.
    let core = (rad * k).floor() as i32;
    if core >= 1 {
        disc(pb, cx, cy, core, INK, 0.48 + 0.36 * k);
        if core >= 2 {
            dotted_ring(pb, cx, cy, core as f32, INK_SHEEN, 0.65);
        }
    }
    // Crisp vermilion target cross at the detonation centre.
    let cross_c = if k > 0.7 { WARN_HI } else { WARN };
    line(pb, (cx - 1, cy), (cx + 1, cy), cross_c, 1.0);
    line(pb, (cx, cy - 1), (cx, cy + 1), cross_c, 1.0);
    put(pb, cx, cy, WARN_HI, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const KINDS: [EnemyKind; 7] = [
        EnemyKind::Silverfish,
        EnemyKind::ErratumSlip,
        EnemyKind::TypeSlug,
        EnemyKind::StapleCrawler,
        EnemyKind::PaperWasp,
        EnemyKind::PaperReel,
        EnemyKind::InkBlot,
    ];

    fn body(kind: EnemyKind, anim: f32) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(32.0, 32.0), 1.0, &mut rng);
        e.anim = anim;
        e
    }

    fn painted(pb: &PixelBuffer, bg: Rgb) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) != bg {
                    out.push((x, y));
                }
            }
        }
        out
    }

    fn render(kind: EnemyKind, anim: f32, lod: bool) -> PixelBuffer {
        let bg = (40, 40, 44);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(bg);
        let e = body(kind, anim);
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (255, 255, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (255, 255, 255));
        }
        pb
    }

    fn render_enemy(e: &Enemy, lod: bool) -> PixelBuffer {
        let bg = (40, 40, 44);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(bg);
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        }
        pb
    }

    #[test]
    fn every_printworks_body_paints_a_light_and_a_dark_edge() {
        let bg = (40, 40, 44);
        for kind in KINDS {
            let pb = render(kind, 1.0, false);
            let px = painted(&pb, bg);
            assert!(px.len() >= 8, "{kind:?} drew only {} pixels", px.len());
            let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            let hi = px.iter().map(|&(x, y)| lum(pb.pixel_at(x, y))).max().unwrap();
            let lo = px.iter().map(|&(x, y)| lum(pb.pixel_at(x, y))).min().unwrap();
            assert!(hi >= 480, "{kind:?} has no light edge (max {hi})");
            assert!(lo <= 260, "{kind:?} has no dark edge (min {lo})");
        }
    }

    #[test]
    fn every_printworks_body_stays_inside_its_cull_box() {
        // The widest archetype bound here is the Brute's 13.7 px (the flea's is
        // wider still); nothing may paint further than 16 px from the centre.
        let bg = (40, 40, 44);
        for kind in KINDS {
            for lod in [false, true] {
                let pb = render(kind, 2.3, lod);
                for (x, y) in painted(&pb, bg) {
                    let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                    assert!(d <= 16.0, "{kind:?} lod={lod} painted {d} px out");
                }
            }
        }
    }

    #[test]
    fn every_printworks_body_stays_inside_cull_box_diagonal_and_elite() {
        let bg = (40, 40, 44);
        for kind in KINDS {
            for &elite in &[false, true] {
                let mut e = body(kind, 1.7);
                e.facing = Vec2::new(0.7071, 0.7071);
                e.elite = elite;
                e.fire_cd = 0.12;
                e.contact_cd = 0.45;
                if kind == EnemyKind::StapleCrawler {
                    e.w4.phase = sim::STAPLE_PHASE_TELL;
                    e.w4.t = 0.1;
                }
                for lod in [false, true] {
                    let pb = render_enemy(&e, lod);
                    for (x, y) in painted(&pb, bg) {
                        let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                        assert!(
                            d <= 16.0,
                            "{kind:?} elite={elite} lod={lod} painted {d} px out"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_printworks_body_has_two_walk_frames() {
        for kind in KINDS {
            let frames: Vec<PixelBuffer> =
                [0.0f32, 1.3, 2.9, 4.4, 7.0].iter().map(|&a| render(kind, a, false)).collect();
            let differs = frames.windows(2).any(|w| w[0].pixels() != w[1].pixels());
            assert!(differs, "{kind:?} never changes pose");
        }
    }

    #[test]
    fn far_bodies_are_small_and_present() {
        let bg = (40, 40, 44);
        for kind in KINDS {
            let pb = render(kind, 1.0, true);
            let n = painted(&pb, bg).len();
            assert!((3..=90).contains(&n), "{kind:?} far body is {n} pixels");
        }
    }

    #[test]
    fn tell_and_attack_poses_are_distinct() {
        // Staple Crawler: walk, tell, strike and rest all read differently.
        let mut sc = body(EnemyKind::StapleCrawler, 1.0);
        sc.w4.phase = sim::STAPLE_PHASE_WALK;
        let walk = render_enemy(&sc, false);
        sc.w4.phase = sim::STAPLE_PHASE_TELL;
        sc.w4.t = 0.2;
        let tell = render_enemy(&sc, false);
        sc.w4.phase = sim::STAPLE_PHASE_STRIKE;
        sc.w4.t = 0.2;
        let strike = render_enemy(&sc, false);
        sc.w4.phase = sim::STAPLE_PHASE_REST;
        sc.w4.t = 0.6;
        let rest = render_enemy(&sc, false);
        assert_ne!(walk.pixels(), tell.pixels(), "staple tell must differ from walk");
        assert_ne!(tell.pixels(), strike.pixels(), "staple strike must differ from tell");
        assert_ne!(strike.pixels(), rest.pixels(), "staple rest must differ from strike");

        // Paper Wasp: idle, 0.75 s shot tell, and post-shot recoil all differ.
        let mut pw = body(EnemyKind::PaperWasp, 1.0);
        pw.fire_cd = 1.4;
        let pw_idle = render_enemy(&pw, false);
        pw.fire_cd = 0.15;
        let pw_tell = render_enemy(&pw, false);
        pw.fire_cd = 2.15;
        let pw_recoil = render_enemy(&pw, false);
        assert_ne!(pw_idle.pixels(), pw_tell.pixels(), "wasp tell must differ from idle");
        assert_ne!(pw_idle.pixels(), pw_recoil.pixels(), "wasp recoil must differ from idle");

        // Type Slug: contact lunge shifts the block forward.
        let mut ts = body(EnemyKind::TypeSlug, 1.0);
        ts.contact_cd = 0.0;
        let ts_skate = render_enemy(&ts, false);
        ts.contact_cd = 0.45;
        let ts_lunge = render_enemy(&ts, false);
        assert_ne!(ts_skate.pixels(), ts_lunge.pixels(), "slug lunge must differ from skate");
    }

    #[test]
    fn elite_paper_reel_wears_proud_metal_flanges() {
        let bg = (40, 40, 44);
        let mut ord = body(EnemyKind::PaperReel, 1.0);
        ord.elite = false;
        let mut el = body(EnemyKind::PaperReel, 1.0);
        el.elite = true;

        for lod in [false, true] {
            let p_ord = render_enemy(&ord, lod);
            let p_el = render_enemy(&el, lod);
            assert_ne!(p_ord.pixels(), p_el.pixels(), "elite paper reel must look distinct (lod={lod})");
            let n_ord = painted(&p_ord, bg).len();
            let n_el = painted(&p_el, bg).len();
            assert!(n_el > n_ord, "elite flanges must add visible silhouette mass (lod={lod})");
            if lod {
                assert!((3..=90).contains(&n_el), "elite far reel is {n_el} pixels");
            }
        }
    }

    #[test]
    fn the_staple_footprint_is_the_radius_the_strike_uses() {
        let bg = (40, 40, 44);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(bg);
        let mut e = body(EnemyKind::StapleCrawler, 0.0);
        e.pos = Vec2::new(10.0, 10.0);
        e.w4.phase = sim::STAPLE_PHASE_STRIKE;
        e.w4.t = sim::STAPLE_STRIKE_SECS;
        e.w4.trail[0] = Vec2::new(30.0, 40.0);
        e.w4.trail[1] = Vec2::new(44.0, 40.0);
        draw_staple_tell(&mut pb, &e, Vec2::ZERO);
        let px = painted(&pb, bg);
        assert!(!px.is_empty());
        for (x, y) in px {
            let p = Vec2::new(x as f32, y as f32);
            let d = p.dist(e.w4.trail[0]).min(p.dist(e.w4.trail[1]));
            assert!(d <= sim::STAPLE_FOOT_RADIUS + 0.75, "footprint pixel {d} px from a foot");
        }
        // Walking: nothing at all.
        let mut clean = PixelBuffer::new(64, 64);
        clean.clear(bg);
        e.w4.phase = sim::STAPLE_PHASE_WALK;
        draw_staple_tell(&mut clean, &e, Vec2::ZERO);
        assert!(painted(&clean, bg).is_empty());
    }

    #[test]
    fn ink_endpoint_stays_inside_wick_burst_radius() {
        let bg = (40, 40, 44);
        let end = Vec2::new(32.0, 32.0);
        let rad = crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS;
        let mut early = PixelBuffer::new(64, 64);
        early.clear(bg);
        draw_ink_endpoint(&mut early, end, Vec2::ZERO, 0.9);
        let mut late = PixelBuffer::new(64, 64);
        late.clear(bg);
        draw_ink_endpoint(&mut late, end, Vec2::ZERO, 0.1);
        let late_px = painted(&late, bg);
        assert!(!late_px.is_empty());
        assert_ne!(early.pixels(), late.pixels(), "ink endpoint should fill as fuse burns");
        for (x, y) in late_px {
            let d = Vec2::new(x as f32, y as f32).dist(end);
            assert!(d <= rad + 1.0, "endpoint pixel {d} px from centre (rad {rad})");
        }
    }
}
