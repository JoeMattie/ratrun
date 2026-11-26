//! Pixel art for the wave-5 weapons:
//! **Glass Bead** and **Plumb Bob**.
//!
//! Pure render-only drawing from continuous inputs — zero simulation state,
//! zero rng and zero allocation. World-space
//! coordinates map through [`w2s`] with the same `(p - cam).round()` rule as
//! `art_wave3a` and `art_rat`.

#![allow(clippy::disallowed_methods)] // render-only: platform maths is fine

use std::f32::consts::PI;

use crate::game::art_wave3a::RingDedup;
use crate::game::ecs::systems::weapons_wave5::{BOB_PASS, PlumbBob, SHED_PASS};
use crate::game::level::Level;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::LightMap;
use crate::render::palette::{self, Rgb};

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
fn shade_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, palette::scale(cur, 1.0 - amount));
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

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;
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
    fn wave5_draw_functions_survive_degenerate_buffers_and_inputs() {
        let mut zero = PixelBuffer::new(0, 0);
        draw_bead(&mut zero, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, 0);
        draw_bead(&mut zero, Vec2::new(5.0, 5.0), Vec2::new(1.0, 0.0), Vec2::ZERO, 4);
        paint_bob(&mut zero, (0, 0), false, 0.0);
        paint_bob(&mut zero, (34, 0), true, 0.0);

        let mut pb = PixelBuffer::new(16, 16);
        draw_bead(&mut pb, Vec2::new(8.0, 8.0), Vec2::ZERO, Vec2::ZERO, 2);
        paint_bob(&mut pb, (8, 8), true, 0.0);
    }
}
