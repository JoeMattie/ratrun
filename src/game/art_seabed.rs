//! Decorative aquatic plants on the Outfall seabed: kelp fronds, seagrass
//! tufts and corner anemones / sea fans. Pure drawing (render-only).
//!
//! Drawn in `World::draw_scene` after the floor, puddles, Outfall hazards,
//! walls and props, and **before** pools, loot, enemies, bosses and the rat,
//! so every body passes cleanly over them and the lighting pass shades them
//! with the rest of the scene.
//!
//! Animation reads **only** `World::elapsed` (continuous sine sway, never
//! stepped), so frozen frames are bit-identical. Sway amplitudes are tiny and
//! the number of swaying plants per frame is capped by [`MAX_SWAY_PER_FRAME`]
//! so terminal diff churn stays minimal.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{PlantKind, SeabedPlant};

/// Maximum number of on-screen plants allowed to animate sway in a single
/// frame (caps moving pixel area at wide 2× zoom-out for terminal byte budget).
pub const MAX_SWAY_PER_FRAME: usize = 20;

/// Maximum tip sway amplitude of a kelp frond, in world px.
pub const KELP_MAX_SWAY_PX: f32 = 1.75;

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, col: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, mix(cur, col, alpha.clamp(0.0, 1.0)));
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

/// Draw one seabed plant onto `pb`.
///
/// `cam` is the world position of the buffer's top-left pixel; `elapsed` is
/// `World::elapsed` in seconds. Pass `sway_scale = 0.0` once the per-frame
/// swaying cap is reached to keep distant extra plants still.
pub fn draw_plant(
    pb: &mut PixelBuffer,
    plant: &SeabedPlant,
    cam: Vec2,
    elapsed: f32,
    sway_scale: f32,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let rx = plant.pos.x.round() as i32 - cam_x;
    let ry = plant.pos.y.round() as i32 - cam_y;

    // Conservative clip margin covering the tallest swaying kelp frond.
    if rx < -12 || ry < -16 || rx >= pb.w as i32 + 12 || ry >= pb.h as i32 + 12 {
        return;
    }

    match plant.kind {
        PlantKind::Kelp => draw_kelp(pb, rx, ry, plant, elapsed, sway_scale),
        PlantKind::Seagrass => draw_seagrass(pb, rx, ry, plant, elapsed, sway_scale),
        PlantKind::Anemone => draw_anemone(pb, rx, ry, plant, elapsed, sway_scale),
    }
}

/// Tall olive/brown kelp frond with smooth continuous underwater sway.
fn draw_kelp(
    pb: &mut PixelBuffer,
    rx: i32,
    ry: i32,
    plant: &SeabedPlant,
    elapsed: f32,
    sway_scale: f32,
) {
    const PALETTES: [(Rgb, Rgb, Rgb); 3] = [
        ((34, 42, 24), (56, 66, 36), (78, 90, 48)), // olive kelp
        ((42, 36, 22), (68, 56, 34), (92, 76, 44)), // warm brown kelp
        ((28, 42, 30), (46, 66, 42), (68, 90, 56)), // deep sea-olive
    ];
    let (dark, mid, hi) = PALETTES[(plant.seed % 3) as usize];
    let size = plant.size.min(2);
    let height = 7 + i32::from(size) * 2; // 7, 9, or 11 px
    let lean_dir = if (plant.seed & 1) == 0 { 1 } else { -1 };
    let lean_bias = lean_dir as f32 * (0.45 + ((plant.seed >> 2) & 3) as f32 * 0.16);
    let omega = 0.85 + ((plant.seed >> 4) & 7) as f32 * 0.04;
    let amp = (1.15 + f32::from(size) * 0.28).min(KELP_MAX_SWAY_PX) * sway_scale;

    // Static ground contact shadow + holdfast root.
    for dx in -1..=1 {
        darken_px(pb, rx + dx, ry + 1, 0.76);
    }
    pb.plot(rx, ry, dark);
    blend_px(pb, rx + lean_dir, ry, dark, 0.72);

    for i in 1..=height {
        let t = i as f32 / height as f32;
        let sway = (elapsed * omega + plant.phase - t * 1.15).sin() * amp * (t * t);
        let x = rx + (lean_bias * t + sway).round() as i32;
        let y = ry - i;
        let stem_col = if i == 1 {
            dark
        } else if i >= height - 2 {
            hi
        } else {
            mid
        };
        pb.plot(x, y, stem_col);

        // Alternating 1 px leafy blades along mid-to-upper frond.
        if (2..height).contains(&i) && (i & 1) == 0 {
            let side = if (((i >> 1) + (plant.seed & 1) as i32) & 1) == 0 {
                1
            } else {
                -1
            };
            let leaf_col = if i >= height - 3 { hi } else { mid };
            blend_px(pb, x + side, y, leaf_col, 0.84);
            if i <= height - 3 && (plant.seed & 2) != 0 && (i & 3) == 0 {
                blend_px(pb, x + side, y + 1, dark, 0.56);
            }
        }
    }
}

/// Short green seagrass tuft on open sand that barely sways.
fn draw_seagrass(
    pb: &mut PixelBuffer,
    rx: i32,
    ry: i32,
    plant: &SeabedPlant,
    elapsed: f32,
    sway_scale: f32,
) {
    const PALETTES: [(Rgb, Rgb, Rgb); 2] = [
        ((30, 50, 32), (46, 74, 46), (66, 98, 58)),
        ((28, 48, 36), (42, 70, 50), (62, 94, 66)),
    ];
    let (base, mid, tip) = PALETTES[(plant.seed & 1) as usize];
    let size = plant.size.min(2);
    let omega = 0.65 + ((plant.seed >> 3) & 3) as f32 * 0.04;
    let amp = 0.55 * sway_scale; // Barely sways: at most ±1 px at the very tip.

    for dx in -2..=2 {
        darken_px(pb, rx + dx, ry + 1, 0.82);
    }

    let offsets: &[i32] = if size >= 2 {
        &[-3, -1, 1, 3]
    } else {
        &[-2, 0, 2]
    };
    for (b_idx, &bx_off) in offsets.iter().enumerate() {
        let bh = 4
            + ((plant.seed >> (b_idx * 2)) & 1) as i32
            + i32::from(b_idx == 1 && size >= 1);
        let by_off = (b_idx & 1) as i32;
        let splay = if bx_off < 0 {
            -0.7
        } else if bx_off > 0 {
            0.7
        } else if (plant.seed & 8) == 0 {
            0.15
        } else {
            -0.15
        };
        let denom = (bh - 1).max(1) as f32;
        for i in 0..bh {
            let t = i as f32 / denom;
            let sway = (elapsed * omega + plant.phase + b_idx as f32 * 0.5).sin() * amp * (t * t);
            let x = rx + bx_off + (splay * t + sway).round() as i32;
            let y = ry + by_off - i;
            let col = if i == 0 {
                base
            } else if i == bh - 1 {
                tip
            } else {
                mid
            };
            pb.plot(x, y, col);
        }
    }
}

/// Compact muted coral-pink / sea-orange anemone or sea fan tucked in a corner.
fn draw_anemone(
    pb: &mut PixelBuffer,
    rx: i32,
    ry: i32,
    plant: &SeabedPlant,
    elapsed: f32,
    sway_scale: f32,
) {
    const PALETTES: [(Rgb, Rgb, Rgb); 2] = [
        ((54, 32, 38), (98, 56, 64), (132, 82, 88)), // muted coral-pink
        ((58, 36, 24), (106, 66, 40), (138, 90, 56)), // muted sea-orange
    ];
    let (base, body, tip) = PALETTES[(plant.seed & 1) as usize];
    let omega = 0.75 + ((plant.seed >> 2) & 3) as f32 * 0.05;
    let amp = 0.55 * sway_scale;

    for dx in -2..=2 {
        darken_px(pb, rx + dx, ry + 1, 0.76);
    }
    for dx in -1..=1 {
        pb.plot(rx + dx, ry, base);
    }
    pb.plot(rx, ry - 1, body);

    const BRANCHES: [(f32, f32); 6] = [
        (-2.6, -1.5),
        (-1.8, -3.1),
        (-0.5, -3.7),
        (0.6, -3.7),
        (1.8, -3.1),
        (2.6, -1.5),
    ];
    let scale_b = match plant.size.min(2) {
        0 => 0.90,
        1 => 1.00,
        _ => 1.12,
    };
    for (idx, &(tx, ty)) in BRANCHES.iter().enumerate() {
        let sway = (elapsed * omega + plant.phase + idx as f32 * 0.95).sin() * amp;
        let mx = rx + (tx * scale_b * 0.55).round() as i32;
        let my = ry + (ty * scale_b * 0.55).round() as i32;
        let ex = rx + (tx * scale_b + sway).round() as i32;
        let ey = ry + (ty * scale_b).round() as i32;
        pb.plot(mx, my, body);
        pb.plot(ex, ey, tip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::world::World;

    #[test]
    fn all_plant_kinds_paint_within_bounds_and_clip_at_edges() {
        const BG: Rgb = (84, 77, 60);
        for (idx, kind) in [PlantKind::Kelp, PlantKind::Seagrass, PlantKind::Anemone]
            .into_iter()
            .enumerate()
        {
            let mut pb = PixelBuffer::new(40, 40);
            pb.clear(BG);
            let plant = SeabedPlant {
                pos: Vec2::new(20.0, 26.0),
                kind,
                phase: 0.7,
                size: 2,
                seed: idx as u32 + 1,
            };
            draw_plant(&mut pb, &plant, Vec2::ZERO, 1.2, 1.0);
            let painted = pb.pixels().iter().filter(|&&c| c != BG).count();
            assert!(painted >= 8, "{kind:?} painted only {painted} pixels");

            // Clipping near and past buffer edges must never panic.
            let mut tiny = PixelBuffer::new(8, 8);
            for pos in [
                Vec2::new(0.0, 0.0),
                Vec2::new(7.0, 7.0),
                Vec2::new(-5.0, 4.0),
                Vec2::new(15.0, 4.0),
                Vec2::new(4.0, -5.0),
                Vec2::new(4.0, 15.0),
            ] {
                let p = SeabedPlant { pos, ..plant };
                draw_plant(&mut tiny, &p, Vec2::ZERO, 2.5, 1.0);
            }
        }
    }

    #[test]
    fn kelp_sways_smoothly_over_elapsed_time_within_bounded_footprint() {
        const BG: Rgb = (84, 77, 60);
        let plant = SeabedPlant {
            pos: Vec2::new(20.0, 26.0),
            kind: PlantKind::Kelp,
            phase: 0.2,
            size: 2,
            seed: 5,
        };
        let render = |t: f32| {
            let mut pb = PixelBuffer::new(40, 40);
            pb.clear(BG);
            draw_plant(&mut pb, &plant, Vec2::ZERO, t, 1.0);
            pb
        };
        let a = render(0.0);
        let b = render(2.2);
        let c = render(4.0);
        assert!(
            a.pixels() != b.pixels() || b.pixels() != c.pixels(),
            "kelp should sway across elapsed time"
        );
        // Every painted pixel stays inside a tight bounding box around the root.
        for pb in [&a, &b, &c] {
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if pb.pixel_at(x, y) != BG {
                        assert!(
                            (14..=26).contains(&x) && (13..=28).contains(&y),
                            "kelp pixel ({x}, {y}) exceeded compact sway box"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn frozen_outfall_frame_with_plants_is_bit_identical_and_bodies_pass_over_plants() {
        let mut w = World::new(Theme::Outfall, 7);
        assert!(
            !w.level.plants.is_empty(),
            "Outfall world must generate seabed plants"
        );
        // Stand the rat right near an interior placed plant so plants are on
        // screen and the camera is not clamped to a map edge.
        let target = w
            .level
            .plants
            .iter()
            .find(|p| {
                p.pos.x > 120.0
                    && p.pos.x < w.level.arena.x - 120.0
                    && p.pos.y > 120.0
                    && p.pos.y < w.level.arena.y - 120.0
            })
            .expect("Outfall should have interior seabed plants")
            .pos;
        w.player.pos = target + Vec2::new(16.0, 0.0);
        w.elapsed = 3.25;

        let mut pb_a = PixelBuffer::new(120, 80);
        let mut pb_b = PixelBuffer::new(120, 80);
        w.draw(&mut pb_a);
        w.draw(&mut pb_b);
        assert_eq!(
            pb_a.pixels(),
            pb_b.pixels(),
            "frozen Outfall frame must be bit-identical"
        );

        // Confirm the plants actually contribute pixels to the scene.
        let saved = std::mem::take(&mut w.level.plants);
        let mut pb_no_plants = PixelBuffer::new(120, 80);
        w.draw(&mut pb_no_plants);
        assert_ne!(
            pb_a.pixels(),
            pb_no_plants.pixels(),
            "Outfall scene with plants must differ from scene without plants"
        );

        // A body (the rat) positioned directly over a plant passes OVER it.
        w.flat_lit = true;
        w.level.plants = saved;
        w.player.pos = target;
        let mut pb_over = PixelBuffer::new(120, 80);
        w.draw(&mut pb_over);
        let sx = (target.x - w.cam.x).round() as usize;
        let sy = (target.y - w.cam.y).round() as usize;
        w.level.plants.clear();
        let mut pb_rat_only = PixelBuffer::new(120, 80);
        w.draw(&mut pb_rat_only);
        assert_eq!(
            pb_over.pixel_at(sx, sy),
            pb_rat_only.pixel_at(sx, sy),
            "the rat must draw over seabed plants"
        );
    }
}
