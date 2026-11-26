//! Pixel art for the three wave-5 weapons:
//! **Glass Bead**, **Plumb Bob**, and **Hold Point**.
//!
//! Pure render-only drawing from continuous inputs — zero simulation state,
//! zero rng and zero allocation. World-space
//! coordinates map through [`w2s`] with the same `(p - cam).round()` rule as
//! `art_wave3a` and `art_rat`.

#![allow(clippy::disallowed_methods)] // render-only: platform maths is fine

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::game::art_wave3a::RingDedup;
use crate::game::ecs::systems::weapons_wave5::{BOB_PASS, PlumbBob, SHED_PASS};
use crate::game::level::Level;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::LightMap;
use crate::render::palette::{self, Rgb};

/// Hit and drawn radius (px) of **Hold Point**'s floor circle.
pub const HOLD_RADIUS: f32 = 28.0;

// --- Glass Bead palette ----------------------------------------------------
const BEAD_CORE: Rgb = (255, 255, 252);
const BEAD_TRAIL_1: Rgb = (212, 222, 226);
const BEAD_TRAIL_2: Rgb = (146, 168, 180);
const BEAD_TRAIL_3: Rgb = (84, 106, 122);
const BEAD_SPARK_HI: Rgb = (255, 242, 184);
const BEAD_SPARK_WARM: Rgb = (255, 182, 82);

// --- Plumb Bob palette -----------------------------------------------------
const CHAIN_DARK: Rgb = (4, 4, 5);
const CHAIN_LINK: Rgb = (93, 83, 69);
const BOB_ARC: Rgb = (224, 186, 94);
const BRASS_HI: Rgb = (255, 246, 196);
const BRASS_HI_EVO: Rgb = (255, 252, 224);
const BRASS_LIT: Rgb = (234, 196, 98);
const BRASS_LIT_EVO: Rgb = (242, 206, 108);
const BRASS_MID: Rgb = (194, 150, 58);
const BRASS_SHADE: Rgb = (132, 94, 34);
const BRASS_RIM_HI: Rgb = (158, 120, 48);
const BRASS_RIM_LO: Rgb = (58, 40, 14);

// --- Hold Point palette ----------------------------------------------------
const HOLD_WHITE: Rgb = (238, 248, 255);
const HOLD_ICE: Rgb = (168, 218, 250);
const HOLD_TRAIL: Rgb = (136, 206, 252);
const HOLD_STEEL: Rgb = (88, 132, 170);
const HOLD_DARK: Rgb = (14, 22, 34);
const CRACK_CORE: Rgb = (255, 252, 236);
const CRACK_FAULT: Rgb = (255, 134, 58);
const CRACK_BACK: Rgb = (16, 8, 10);

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn unit_clamp(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
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
fn shade_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::scale(cur, 1.0 - amount));
}

/// Walk a deduplicated arc of radius `rad` around `c` from `a0` to `a1`.
fn walk_arc(
    c: (i32, i32),
    rad: f32,
    a0: f32,
    a1: f32,
    steps: usize,
    mut plot: impl FnMut(i32, i32),
) {
    if steps == 0 || rad <= 0.0 {
        return;
    }
    let mut seen = RingDedup::new();
    let denom = steps as f32;
    for i in 0..=steps {
        let t = i as f32 / denom;
        let a = a0 + (a1 - a0) * t;
        let (sa, ca) = a.sin_cos();
        let px = c.0 + (ca * rad).round() as i32;
        let py = c.1 + (sa * rad).round() as i32;
        if seen.repeat((px, py)) {
            continue;
        }
        plot(px, py);
    }
}

/// Walk an integer 1-px Bresenham segment from `p0` to `p1`.
fn walk_line(p0: (i32, i32), p1: (i32, i32), mut visit: impl FnMut(usize, i32, i32)) {
    let dx = (p1.0 - p0.0).abs();
    let dy = -(p1.1 - p0.1).abs();
    let sx = if p0.0 < p1.0 { 1 } else { -1 };
    let sy = if p0.1 < p1.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = p0;
    let mut idx = 0usize;
    loop {
        visit(idx, x, y);
        if x == p1.0 && y == p1.1 {
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
        idx += 1;
    }
}

/// **Glass Bead**: one bright 1-px glass bead at world position `pos` with a
/// 3-px fading translucent trail behind `vel` and a tiny detached warm spark
/// that shifts and intensifies on each new bounce count (`bounces`).
pub fn draw_bead(pb: &mut PixelBuffer, pos: Vec2, vel: Vec2, cam: Vec2, bounces: u8) {
    if !pos.x.is_finite() || !pos.y.is_finite() || !cam.x.is_finite() || !cam.y.is_finite() {
        return;
    }
    let c = w2s(pos, cam);
    let has_vel = vel.x.is_finite() && vel.y.is_finite() && vel.len_sq() > 1e-6;
    let dir = if has_vel {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();

    // 1. 3-px fading optical glass trail strictly behind velocity, graded well
    //    below the pure-white head so the 1-px bead never blurs into a bar.
    if has_vel {
        let mut prev = c;
        let trail: [(f32, Rgb, f32); 3] = [
            (1.0, BEAD_TRAIL_1, 0.62),
            (2.0, BEAD_TRAIL_2, 0.40),
            (3.0, BEAD_TRAIL_3, 0.22),
        ];
        for (step, col, gain) in trail {
            let s = w2s(pos - dir * step, cam);
            if s != prev && s != c {
                pb.plot_add(s.0, s.1, palette::scale(col, gain));
                prev = s;
            }
        }
    }

    // 2. Tiny detached spark driven purely by `bounces`: flips parity and adds
    //    glint flecks with each bounce (1, 2, 3+) while keeping clean 1-px air
    //    around the bead core.
    if bounces > 0 {
        let parity = if bounces & 1 == 1 { 1.0f32 } else { -1.0f32 };
        match bounces {
            1 => {
                let sp = w2s(pos - dir * 0.6 + perp * (1.9 * parity), cam);
                if sp != c {
                    pb.plot_add(sp.0, sp.1, palette::scale(BEAD_SPARK_WARM, 0.78));
                }
            }
            2 => {
                let pts = [
                    (pos + dir * 0.5 + perp * (1.9 * parity), BEAD_SPARK_HI, 0.76),
                    (pos - dir * 1.5 - perp * (1.9 * parity), BEAD_SPARK_WARM, 0.72),
                ];
                for (wp, col, gain) in pts {
                    let s = w2s(wp, cam);
                    if s != c {
                        pb.plot_add(s.0, s.1, palette::scale(col, gain));
                    }
                }
            }
            _ => {
                let pts = [
                    (pos - dir * 0.7 + perp * 2.0, BEAD_SPARK_HI, 0.82),
                    (pos - dir * 0.7 - perp * 2.0, BEAD_SPARK_HI, 0.82),
                    (pos + dir * 1.3 + perp * (1.8 * parity), BEAD_SPARK_WARM, 0.68),
                ];
                for (wp, col, gain) in pts {
                    let s = w2s(wp, cam);
                    if s != c {
                        pb.plot_add(s.0, s.1, palette::scale(col, gain));
                    }
                }
            }
        }
    }

    // 3. Single bright 1-px glass bead core plotted last so it stays crisp.
    pb.plot(c.0, c.1, BEAD_CORE);
}

const BOB_LINKS: usize = 8;
const CHAIN_DT: f32 = 1.0 / 240.0;

/// Replay the pinned Verlet chain relative to the rat at fixed substeps.
/// The cast clock determines the whole pose, including fractional substeps,
/// so a repeated or skipped draw has no effect on the picture.
pub(crate) fn bob_chain(bob: &PlumbBob, face: Vec2) -> [Vec2; BOB_LINKS + 1] {
    let age = bob.age();
    let (swing, roll) = bob.offsets_at(age);
    let tip = bob.pos - bob.pivot;
    let reach_scale = (tip.len() / (swing + roll).len()).min(1.0);
    let face = face.normalized();
    let sample = |t| {
        let (swing, roll) = bob.offsets_at(t);
        (swing + roll) * reach_scale
    };

    let first_tip = sample(0.0);
    let first_paw = bob_paw(face, first_tip, 0.0);
    let mut nodes = std::array::from_fn(|i| {
        let u = i as f32 / BOB_LINKS as f32;
        // Seed a loose curve behind the weight at release.
        first_paw + (first_tip - first_paw) * u
            - first_tip.normalized().perp() * (20.0 * reach_scale * u * (1.0 - u))
    });
    let mut previous = nodes;
    let steps = age / CHAIN_DT;
    for step in 1..=steps.ceil() as usize {
        let t = step as f32 * CHAIN_DT;
        let next_tip = sample(t);
        let speed = bob_speed(bob, t);
        let next_paw = bob_paw(face, next_tip, speed);
        let chord = next_tip - next_paw;
        let slack = 3.4 * (1.0 - speed.abs()) * reach_scale;
        let link_len = (chord.len() + slack) / BOB_LINKS as f32;
        let drag = -chord.normalized().perp() * (420.0 * speed);
        let force = (drag + Vec2::new(0.0, 120.0)) * (CHAIN_DT * CHAIN_DT);
        let before = nodes;
        for i in 1..BOB_LINKS {
            nodes[i] += (before[i] - previous[i]) * 0.91 + force;
        }
        previous = before;
        for _ in 0..6 {
            nodes[0] = next_paw;
            nodes[BOB_LINKS] = next_tip;
            for i in 0..BOB_LINKS {
                let delta = nodes[i + 1] - nodes[i];
                let correction = delta - delta.normalized() * link_len;
                let left = if i == 0 { 0.0 } else if i + 1 == BOB_LINKS { 1.0 } else { 0.5 };
                nodes[i] += correction * left;
                nodes[i + 1] -= correction * (1.0 - left);
            }
        }
    }

    let speed = bob_speed(bob, age);
    let paw = bob_paw(face, tip, speed);
    let fraction = steps.fract();
    let taut = speed.abs().powi(6);
    for (i, node) in nodes.iter_mut().enumerate() {
        let u = i as f32 / BOB_LINKS as f32;
        let straight = paw + (tip - paw) * u;
        if fraction > 0.0 {
            *node = previous[i] + (*node - previous[i]) * fraction;
        }
        *node = bob.pivot + (*node + (straight - *node) * taut);
    }
    nodes[0] = bob.pivot + paw;
    nodes[BOB_LINKS] = bob.pos;
    nodes
}

/// Signed fraction of peak swing speed. Ease the smear at the ends of
/// Shed Bob’s linear extension while preserving the path of the weight.
fn bob_speed(bob: &PlumbBob, age: f32) -> f32 {
    let phase = bob.swing_phase(age);
    if phase.fract() != 0.0 {
        (PI * phase).sin()
    } else if bob.evolved {
        let roll = ((age - BOB_PASS) / SHED_PASS).clamp(0.0, 2.0);
        if roll.fract() == 0.0 { 0.0 } else { (PI * roll).sin() * 0.65 }
    } else {
        0.0
    }
}

fn bob_paw(face: Vec2, tip: Vec2, speed: f32) -> Vec2 {
    let reach = tip.normalized();
    let paw = face * 2.5 + reach * (1.25 + speed.abs());
    paw.clamp_len(tip.len() * 0.5)
}

/// Draw the flexible iron chain and its machined brass weight. Returns the
/// rat’s render-only lean in world pixels.
pub(crate) fn draw_bob(
    pb: &mut PixelBuffer,
    lights: &mut LightMap,
    level: &Level,
    bob: &PlumbBob,
    face: Vec2,
    cam: Vec2,
) -> Vec2 {
    let age = bob.age();
    let speed = bob_speed(bob, age);
    let tip = w2s(bob.pos, cam);

    // Trace only behind the bob on its current leg. Read the same wall-stopped
    // endpoint as the hits so the smear follows the swing and the Shed roll.
    let leg_start = if bob.swing_phase(age) > 1.0 {
        bob.turn_end()
    } else if bob.evolved && age > BOB_PASS + SHED_PASS {
        BOB_PASS + SHED_PASS
    } else if age > BOB_PASS {
        BOB_PASS
    } else {
        0.0
    };
    let span = (0.072 * speed.abs()).min(age - leg_start);
    if span > 0.0 {
        let mut seen = RingDedup::new();
        for i in 1..=16 {
            let u = i as f32 / 16.0;
            let (x, y) = w2s(bob.endpoint_at(level, age - u * span), cam);
            let (dx, dy) = (x - tip.0, y - tip.1);
            if dx * dx + dy * dy > 8 && !seen.repeat((x, y)) {
                pb.plot_add(x, y, palette::scale(BOB_ARC, 0.32 * speed.abs() * (1.0 - u)));
            }
        }
    }

    let nodes = bob_chain(bob, face);
    let mut index = 0usize;
    for link in nodes.windows(2) {
        walk_line(w2s(link[0], cam), w2s(link[1], cam), |step, x, y| {
            if step > 0 || index == 0 {
                if index.is_multiple_of(2) {
                    blend_px(pb, x, y, CHAIN_DARK, 0.64);
                } else {
                    blend_px(pb, x, y, CHAIN_LINK, 0.82);
                    // Keep a muted iron edge visible on a dark floor and
                    // through the widest zoom's box filter.
                    lights.mark_glint(pb, x, y, CHAIN_LINK);
                }
                index += 1;
            }
        });
    }

    let swing = bob.angle_at(age) - bob.angle_at(BOB_PASS * 0.5);
    let (_, roll) = bob.offsets_at(age);
    paint_bob(pb, tip, bob.evolved, swing * 0.9 + roll.len() / 3.0);
    (bob.pos - bob.pivot).normalized() * speed.abs()
}

/// Brass stays inside the 5 px silhouette. The highlight slides across the
/// metal as it rotates, with fractional coverage between pixels.
fn paint_bob(pb: &mut PixelBuffer, (cx, cy): (i32, i32), evolved: bool, turn: f32) {
    let hi = if evolved { BRASS_HI_EVO } else { BRASS_HI };
    let lit = if evolved { BRASS_LIT_EVO } else { BRASS_LIT };
    let glint = Vec2::new(-0.95, -0.90).rotate(turn);

    shade_px(pb, cx - 1, cy + 3, 0.24);
    shade_px(pb, cx, cy + 3, 0.38);
    shade_px(pb, cx + 1, cy + 3, 0.36);
    shade_px(pb, cx + 2, cy + 2, 0.30);

    let pattern: [(i32, i32, Rgb); 21] = [
        (-1, -2, BRASS_RIM_HI),
        (0, -2, BRASS_MID),
        (1, -2, BRASS_SHADE),
        (-2, -1, BRASS_RIM_HI),
        (-1, -1, lit),
        (0, -1, lit),
        (1, -1, BRASS_MID),
        (2, -1, BRASS_RIM_LO),
        (-2, 0, BRASS_MID),
        (-1, 0, lit),
        (0, 0, BRASS_MID),
        (1, 0, BRASS_SHADE),
        (2, 0, BRASS_RIM_LO),
        (-2, 1, BRASS_SHADE),
        (-1, 1, BRASS_MID),
        (0, 1, BRASS_SHADE),
        (1, 1, BRASS_SHADE),
        (2, 1, BRASS_RIM_LO),
        (-1, 2, BRASS_RIM_LO),
        (0, 2, BRASS_RIM_LO),
        (1, 2, BRASS_RIM_LO),
    ];
    for (ox, oy, col) in pattern {
        let glint_gain = (1.0 - glint.dist(Vec2::new(ox as f32, oy as f32)) * 0.82).max(0.0);
        pb.plot(cx + ox, cy + oy, palette::mix(col, hi, glint_gain));
    }
}

/// **Hold Point**: a 28-px cold-white floor circle centred on `centre`.
///
/// - Charging (`charge` in `0..1`, `!cracked`): the 28-px perimeter fills
///   clockwise from top while an inner ring with four short lock ticks rises
///   continuously to meet it.
/// - Firing (`fired` in `0..1`): a bright cold-white expanding shock ring with
///   a trailing harmonic and a brief ignition rim flash.
/// - Cracked (`cracked == true`): a shattered, displaced 2-px broken outline
///   with dark contrast keylines and jagged lightning cracks — deliberately
///   louder than both the charging fill and the fire.
pub fn draw_hold(
    pb: &mut PixelBuffer,
    centre: Vec2,
    cam: Vec2,
    charge: f32,
    fired: f32,
    cracked: bool,
) {
    if !centre.x.is_finite() || !centre.y.is_finite() || !cam.x.is_finite() || !cam.y.is_finite() {
        return;
    }
    let charge = unit_clamp(charge);
    let fired = unit_clamp(fired);
    if charge <= 0.0 && fired <= 0.0 && !cracked {
        return;
    }

    let c = w2s(centre, cam);
    let r = HOLD_RADIUS;

    // 1. Cracked / shattered 28-px outline — reads louder than the fill and fire.
    if cracked {
        const SHARDS: usize = 6;
        let shard_span = TAU / SHARDS as f32;
        let gap = 0.38f32;
        let steps = 36usize;

        for s_idx in 0..SHARDS {
            let a0 = -FRAC_PI_2 + s_idx as f32 * shard_span + gap * 0.5;
            let a1 = -FRAC_PI_2 + (s_idx + 1) as f32 * shard_span - gap * 0.5;
            let sign = if s_idx.is_multiple_of(2) { 1.0f32 } else { -1.0f32 };
            let r_off = 1.5 * sign;
            let tilt = 1.2 * sign;

            // Separate passes so dark backing never checkers the bright shard core.
            let mut seen_b = RingDedup::new();
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let a = a0 + (a1 - a0) * t;
                let sr = r + r_off + (t - 0.5) * tilt + 1.2;
                let (sa, ca) = a.sin_cos();
                let p = (c.0 + (ca * sr).round() as i32, c.1 + (sa * sr).round() as i32);
                if !seen_b.repeat(p) {
                    blend_px(pb, p.0, p.1, CRACK_BACK, 0.90);
                }
            }

            let mut seen_f = RingDedup::new();
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let a = a0 + (a1 - a0) * t;
                let sr = r + r_off + (t - 0.5) * tilt - 1.0;
                let (sa, ca) = a.sin_cos();
                let p = (c.0 + (ca * sr).round() as i32, c.1 + (sa * sr).round() as i32);
                if !seen_f.repeat(p) {
                    blend_px(pb, p.0, p.1, CRACK_FAULT, 0.95);
                }
            }

            let mut seen_c = RingDedup::new();
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let a = a0 + (a1 - a0) * t;
                let sr = r + r_off + (t - 0.5) * tilt;
                let (sa, ca) = a.sin_cos();
                let p = (c.0 + (ca * sr).round() as i32, c.1 + (sa * sr).round() as i32);
                if !seen_c.repeat(p) {
                    pb.plot(p.0, p.1, CRACK_CORE);
                }
            }

            // Sharp 1-px Bresenham lightning-crack fork shooting inward at each break.
            let ba = -FRAC_PI_2 + s_idx as f32 * shard_span;
            let (bs, bc) = ba.sin_cos();
            let bdir = Vec2::new(bc, bs);
            let bperp = bdir.perp();
            let jag_local: [(f32, f32); 4] = [
                (r + 1.5, 0.0),
                (r - 1.2, 1.6),
                (r - 3.8, -1.5),
                (r - 6.6, 0.9),
            ];
            let mut jag_px = [(0i32, 0i32); 4];
            for (slot, &(rr, po)) in jag_px.iter_mut().zip(&jag_local) {
                let off = bdir * rr + bperp * po;
                *slot = (c.0 + off.x.round() as i32, c.1 + off.y.round() as i32);
            }
            for seg in jag_px.windows(2) {
                walk_line(seg[0], seg[1], |_, fx, fy| {
                    blend_px(pb, fx + 1, fy + 1, CRACK_BACK, 0.78);
                });
            }
            for (j, seg) in jag_px.windows(2).enumerate() {
                let col = if j == 0 { CRACK_CORE } else { CRACK_FAULT };
                walk_line(seg[0], seg[1], |_, fx, fy| {
                    pb.plot(fx, fy, col);
                });
            }
        }
    } else if charge > 0.0 {
        // 2. Charging 28-px cold-white circle filling continuously as charge rises.
        let fill_col = palette::mix(HOLD_ICE, HOLD_WHITE, charge);
        let fill_alpha = 0.50 + 0.42 * charge;
        let a_start = -FRAC_PI_2;
        let a_fill = a_start + charge * TAU;
        let fill_steps = (200.0 * charge).round().clamp(4.0, 210.0) as usize;

        // Pass A: subtle dark keyline outside the filled arc.
        let back_alpha = 0.30 + 0.22 * charge;
        walk_arc(c, r + 1.0, a_start, a_fill, fill_steps, |x, y| {
            blend_px(pb, x, y, HOLD_DARK, back_alpha);
        });

        // Pass B: quiet dotted guide along the unfilled remainder of the 28-px rim.
        if charge < 0.995 {
            let rem_steps = (200.0 * (1.0 - charge)).round().clamp(4.0, 200.0) as usize;
            let rem_span = (1.0 - charge) * TAU;
            let guide_alpha = 0.26 + 0.14 * charge;
            let mut seen_g = RingDedup::new();
            for i in 0..rem_steps {
                if !i.is_multiple_of(6) {
                    continue;
                }
                let a = a_fill + (i as f32 / rem_steps as f32) * rem_span;
                let (sa, ca) = a.sin_cos();
                let p = (c.0 + (ca * r).round() as i32, c.1 + (sa * r).round() as i32);
                if !seen_g.repeat(p) {
                    blend_px(pb, p.0, p.1, HOLD_STEEL, guide_alpha);
                }
            }
        }

        // Pass C: soft inner cold-ice glow as charge approaches full.
        let inner_glow = ((charge - 0.40) / 0.60).clamp(0.0, 1.0) * 0.34;
        if inner_glow > 0.02 {
            let glow_col = palette::scale(HOLD_ICE, inner_glow);
            walk_arc(c, r - 1.0, a_start, a_fill, fill_steps, |x, y| {
                pb.plot_add(x, y, glow_col);
            });
        }

        // Pass D: crisp solid cold-white filled arc drawn last.
        walk_arc(c, r, a_start, a_fill, fill_steps, |x, y| {
            blend_px(pb, x, y, fill_col, fill_alpha);
        });

        // Pass E: rising inner concentric ring (glides 5.0 -> 26.5 px smoothly).
        let r_in = 5.0 + 21.5 * charge;
        let in_steps = (r_in * 6.8).round().clamp(24.0, 190.0) as usize;
        let in_col = palette::scale(
            palette::mix(HOLD_STEEL, HOLD_ICE, charge),
            0.26 + 0.42 * charge,
        );
        walk_arc(c, r_in, 0.0, TAU, in_steps, |x, y| {
            pb.plot_add(x, y, in_col);
        });

        // Pass F: four short inward cardinal lock ticks riding on the inner ring.
        let tick_len = (1.0 + 3.0 * charge).round() as i32;
        let tick_col = palette::scale(HOLD_ICE, 0.35 + 0.45 * charge);
        let r_in_i = r_in.round() as i32;
        for k in 1..=tick_len {
            let d = r_in_i - k;
            if d >= 5 {
                pb.plot_add(c.0, c.1 - d, tick_col);
                pb.plot_add(c.0, c.1 + d, tick_col);
                pb.plot_add(c.0 - d, c.1, tick_col);
                pb.plot_add(c.0 + d, c.1, tick_col);
            }
        }
    }

    // 3. Fired shock ring (bright expanding cold-white shockwave for fired 0..1).
    if fired > 0.0 {
        let ease = 1.0 - (1.0 - fired) * (1.0 - fired);
        let r_lead = 5.0 + (r - 5.0) * ease;
        let fade = (1.0 - fired * 0.72).clamp(0.24, 1.0);
        let lead_steps = (r_lead * 6.8).round().clamp(24.0, 210.0) as usize;

        // Trailing harmonic ring.
        let r_trail = r_lead * 0.70;
        if r_trail >= 4.0 {
            let t_steps = (r_trail * 6.5).round().clamp(20.0, 160.0) as usize;
            let t_col = palette::scale(HOLD_TRAIL, 0.46 * fade);
            walk_arc(c, r_trail, 0.0, TAU, t_steps, |x, y| {
                pb.plot_add(x, y, t_col);
            });
        }

        // Inner ice band first, crisp pure-white lead shock ring last.
        if r_lead >= 6.0 {
            let band_col = palette::scale(HOLD_ICE, 0.72 * fade);
            walk_arc(c, r_lead - 1.0, 0.0, TAU, lead_steps, |x, y| {
                pb.plot_add(x, y, band_col);
            });
        }
        walk_arc(c, r_lead, 0.0, TAU, lead_steps, |x, y| {
            blend_px(pb, x, y, (255, 255, 255), 0.96 * fade);
        });

        // Brief crisp ignition rim flash on the 28-px boundary.
        if fired < 0.45 {
            let rim_k = (1.0 - fired / 0.45) * 0.50;
            let rim_col = palette::scale(HOLD_WHITE, rim_k);
            walk_arc(c, r, 0.0, TAU, 190, |x, y| {
                pb.plot_add(x, y, rim_col);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted_pixels(pb: &PixelBuffer, bg: Rgb) -> Vec<(i32, i32, Rgb)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                if c != bg {
                    out.push((x as i32, y as i32, c));
                }
            }
        }
        out
    }

    fn luma(c: Rgb) -> u32 {
        (u32::from(c.0) * 77 + u32::from(c.1) * 150 + u32::from(c.2) * 29) >> 8
    }

    #[test]
    fn bead_paints_inside_expected_box_and_sparks_on_bounces() {
        let bg = (14, 22, 20);
        let cam = Vec2::new(100.0, 50.0);
        let pos = Vec2::new(124.0, 74.0); // screen (24, 24)
        let vel = Vec2::new(120.0, 0.0);

        let mut snapshots = Vec::new();
        for bounces in 0..=3u8 {
            let mut pb = PixelBuffer::new(48, 48);
            pb.clear(bg);
            draw_bead(&mut pb, pos, vel, cam, bounces);

            assert_eq!(pb.pixel_at(24, 24), BEAD_CORE, "bright 1-px core at w2s(pos)");
            let pts = painted_pixels(&pb, bg);
            assert!(
                pts.len() >= 4,
                "bead (bounces={bounces}) should paint core + 3-px trail, got {}",
                pts.len()
            );
            for (x, y, _) in &pts {
                assert!(
                    (x - 24).abs() <= 4 && (y - 24).abs() <= 4,
                    "bead pixel ({x}, {y}) escaped ±4 box around (24, 24) for bounces={bounces}"
                );
            }
            snapshots.push(pb.pixels().to_vec());
        }

        // Each new bounce count produces a distinct spark frame.
        assert_ne!(snapshots[0], snapshots[1], "bounce 1 must add a spark");
        assert_ne!(snapshots[1], snapshots[2], "bounce 2 must differ from bounce 1");
        assert_ne!(snapshots[2], snapshots[3], "bounce 3 must differ from bounce 2");
    }

    #[test]
    fn bob_glint_rolls_inside_the_brass_silhouette() {
        let floor = (16, 24, 22);
        let mut base = PixelBuffer::new(16, 16);
        base.clear(floor);
        paint_bob(&mut base, (8, 8), false, 0.0);
        assert!(luma(base.pixel_at(7, 7)) > 180);

        let mut turned = PixelBuffer::new(16, 16);
        turned.clear(floor);
        paint_bob(&mut turned, (8, 8), false, FRAC_PI_2);
        assert!(luma(turned.pixel_at(9, 7)) > 180);
        assert_ne!(base.pixels(), turned.pixels());
        let mask = |pb: &PixelBuffer| {
            painted_pixels(pb, floor).into_iter().map(|(x, y, _)| (x, y)).collect::<Vec<_>>()
        };
        assert_eq!(mask(&base), mask(&turned));
    }

    #[test]
    fn hold_paints_inside_expected_box_grows_with_charge_and_cracks_distinctly() {
        let bg = (16, 20, 22);
        let cam = Vec2::new(200.0, 100.0);
        let centre = Vec2::new(240.0, 140.0); // screen (40, 40)

        // Idle draws nothing.
        let mut idle = PixelBuffer::new(80, 80);
        idle.clear(bg);
        draw_hold(&mut idle, centre, cam, 0.0, 0.0, false);
        assert!(painted_pixels(&idle, bg).is_empty(), "idle hold paints nothing");

        // Charge 0.2 vs charge 1.0: both stay inside ±30 px, and 1.0 paints more.
        let mut lo = PixelBuffer::new(80, 80);
        lo.clear(bg);
        draw_hold(&mut lo, centre, cam, 0.2, 0.0, false);
        let lo_pts = painted_pixels(&lo, bg);
        assert!(!lo_pts.is_empty());
        for &(x, y, _) in &lo_pts {
            assert!(
                (x - 40).abs() <= 30 && (y - 40).abs() <= 30,
                "hold charge=0.2 pixel ({x}, {y}) escaped ±30 box"
            );
        }

        let mut hi = PixelBuffer::new(80, 80);
        hi.clear(bg);
        draw_hold(&mut hi, centre, cam, 1.0, 0.0, false);
        let hi_pts = painted_pixels(&hi, bg);
        for &(x, y, _) in &hi_pts {
            assert!(
                (x - 40).abs() <= 30 && (y - 40).abs() <= 30,
                "hold charge=1.0 pixel ({x}, {y}) escaped ±30 box"
            );
        }
        assert!(
            hi_pts.len() > lo_pts.len(),
            "charge 1.0 ({}) must paint more pixels than charge 0.2 ({})",
            hi_pts.len(),
            lo_pts.len()
        );

        // Fired shock ring expands inside ±30 px.
        let mut f_early = PixelBuffer::new(80, 80);
        f_early.clear(bg);
        draw_hold(&mut f_early, centre, cam, 0.0, 0.2, false);
        let mut f_late = PixelBuffer::new(80, 80);
        f_late.clear(bg);
        draw_hold(&mut f_late, centre, cam, 0.0, 0.75, false);
        let early_pts = painted_pixels(&f_early, bg);
        let late_pts = painted_pixels(&f_late, bg);
        assert!(!early_pts.is_empty() && !late_pts.is_empty());
        for &(x, y, _) in early_pts.iter().chain(&late_pts) {
            assert!(
                (x - 40).abs() <= 30 && (y - 40).abs() <= 30,
                "hold fired pixel ({x}, {y}) escaped ±30 box"
            );
        }
        assert_ne!(f_early.pixels(), f_late.pixels(), "fired shock ring must expand over 0..1");

        // Cracked differs clearly from uncracked and stays inside ±33 px.
        let mut uncracked = PixelBuffer::new(80, 80);
        uncracked.clear(bg);
        draw_hold(&mut uncracked, centre, cam, 0.5, 0.0, false);

        let mut cracked = PixelBuffer::new(80, 80);
        cracked.clear(bg);
        draw_hold(&mut cracked, centre, cam, 0.5, 0.0, true);
        let cracked_pts = painted_pixels(&cracked, bg);
        assert!(!cracked_pts.is_empty());
        for &(x, y, _) in &cracked_pts {
            assert!(
                (x - 40).abs() <= 33 && (y - 40).abs() <= 33,
                "cracked hold pixel ({x}, {y}) escaped ±33 box"
            );
        }
        assert_ne!(
            cracked.pixels(),
            uncracked.pixels(),
            "cracked hold must differ from uncracked hold"
        );

        // Cracked still draws loudly even when charge is 0.0.
        let mut cracked_zero = PixelBuffer::new(80, 80);
        cracked_zero.clear(bg);
        draw_hold(&mut cracked_zero, centre, cam, 0.0, 0.0, true);
        assert!(
            painted_pixels(&cracked_zero, bg).len() >= 200,
            "cracked hold at charge 0 must still draw the full shattered outline"
        );
    }

    #[test]
    fn wave5_draw_functions_survive_degenerate_buffers_and_inputs() {
        let mut zero = PixelBuffer::new(0, 0);
        draw_bead(&mut zero, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, 0);
        draw_bead(&mut zero, Vec2::new(5.0, 5.0), Vec2::new(1.0, 0.0), Vec2::ZERO, 4);
        paint_bob(&mut zero, (0, 0), false, 0.0);
        paint_bob(&mut zero, (34, 0), true, 0.0);
        draw_hold(&mut zero, Vec2::ZERO, Vec2::ZERO, 0.5, 0.5, false);
        draw_hold(&mut zero, Vec2::ZERO, Vec2::ZERO, f32::NAN, f32::INFINITY, true);

        let mut pb = PixelBuffer::new(16, 16);
        draw_bead(&mut pb, Vec2::new(8.0, 8.0), Vec2::ZERO, Vec2::ZERO, 2);
        paint_bob(&mut pb, (8, 8), true, 0.0);
        draw_hold(&mut pb, Vec2::new(8.0, 8.0), Vec2::ZERO, -1.0, 2.0, false);
        draw_hold(&mut pb, Vec2::new(f32::NAN, 8.0), Vec2::ZERO, 1.0, 0.0, true);
    }
}
