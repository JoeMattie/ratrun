//! Pixel art for Hantavirus rat droppings on the floor.
//!
//! Pure render-only drawing (zero rng, zero heap allocation). Each dropping is
//! a small, deadpan cluster of 2–4 dark brown rice-grain pellets (2×1, 3×1,
//! 1×2, or angled 2–3 px grains) within a ~5×4 px footprint — clearly smaller
//! and looser than a floor gold coin — with a 1-px dark lower-right keyline so
//! pellets read crisply across all six floors (Sewer, Kitchen, Lab, Laundry,
//! Printworks, Flatbed).
//!
//! Three age reads match the weapon contract:
//!
//! 1. **Fresh / unarmed (`age < 0.25`)**: slightly brighter, glossier wet
//!    pellets with no infectious shimmer.
//! 2. **Live / armed (`0.25 <= age <= 12.0`)**: dark brown pellets with a faint
//!    sickly violet-green shimmer (`(206, 106, 190)` from `WeaponKind::Hantavirus`
//!    paired with pale green) drifting 2–3 px above in held 4 Hz steps.
//! 3. **Drying (`age > 12.0`, final 2 s of `DROPPING_LIFE`)**: pellets lighten
//!    toward chalky grey-brown and the shimmer stops.

use crate::game::ecs::components::Dropping;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Arming delay in seconds (contract: `weapon::DROPPING_ARM = 0.25`).
const ARM_AGE: f32 = 0.25;
/// Age in seconds at which a dropping begins drying (last 2 s of 14.0 s life).
const DRYING_AGE: f32 = 12.0;
/// Duration of the drying window in seconds (`14.0 - 12.0`).
const DRYING_SPAN: f32 = 2.0;

/// Hantavirus weapon colour (`WeaponKind::Hantavirus::color`).
const VIRUS_VIOLET: Rgb = (206, 106, 190);
/// Pale sickly green paired with the virus violet for the live shimmer tell.
const VIRUS_GREEN: Rgb = (150, 224, 128);
/// Deep neutral-warm dark keyline blended along the lower-right of each pellet.
const KEYLINE_DARK: Rgb = (8, 5, 5);

/// Armed pellet palette: `(body, shaded_end)` pairs of varied dark browns.
/// Kept darker than Flatbed oak planks `(112, 84, 52)` yet warm and lifted
/// enough to stay legible under 0.60× ambient lighting on Sewer, Kitchen & Lab.
const PAL_ARMED: [(Rgb, Rgb); 3] = [
    ((96, 62, 38), (68, 42, 26)),
    ((84, 54, 34), (58, 36, 23)),
    ((104, 70, 44), (74, 48, 30)),
];
const HI_ARMED: Rgb = (158, 120, 86);

/// Fresh / unarmed (`age < 0.25`): slightly brighter, wet-sheen browns.
const PAL_WET: [(Rgb, Rgb); 3] = [
    ((116, 76, 48), (82, 52, 32)),
    ((102, 66, 42), (72, 44, 28)),
    ((124, 84, 54), (88, 56, 36)),
];
const HI_WET: Rgb = (202, 168, 134);

/// Drying (`age > 12.0`): lighter, desaturated chalky grey-browns.
const PAL_DRY: [(Rgb, Rgb); 3] = [
    ((116, 106, 95), (82, 74, 67)),
    ((104, 96, 88), (74, 68, 62)),
    ((124, 114, 103), (88, 80, 72)),
];
const HI_DRY: Rgb = (162, 154, 142);

/// Eight hand-crafted rice-grain cluster layouts (2–4 pellets each).
///
/// Every coordinate sits inside `dx in -2..=2, dy in -1..=2` (at most 5×4 px),
/// with clean 1-px floor gaps between pellets so clusters never collapse into a
/// solid coin-sized disc. Combined with horizontal mirroring and brown-tone
/// permutation from `d.seed`, these yield varied deadpan scatters while staying
/// strictly inside a 9×9 box (`-4..=4` in both axes) including keyline and
/// rising shimmer.
const LAYOUTS: &[&[&[(i8, i8)]]] = &[
    // 0: 3 pellets — loose trio (3×1 upper-left, 2×1 mid-right, 2-px slant lower-left)
    &[
        &[(-2, -1), (-1, -1), (0, -1)],
        &[(1, 1), (2, 1)],
        &[(-2, 1), (-1, 2)],
    ],
    // 1: 2 pellets — sparse angled pair (3-px slant + 2×1)
    &[
        &[(-2, 0), (-1, -1), (0, -1)],
        &[(0, 1), (1, 1)],
    ],
    // 2: 4 pellets — airy 4-grain scatter
    &[
        &[(-2, -1), (-1, -1)],
        &[(1, -1), (2, -1)],
        &[(-2, 1), (-1, 1)],
        &[(1, 2), (2, 1)],
    ],
    // 3: 3 pellets — angled fan (2-px slant, 3×1 mid, 2×1 lower)
    &[
        &[(-2, 0), (-1, -1)],
        &[(0, 0), (1, 0), (2, 0)],
        &[(-1, 2), (0, 2)],
    ],
    // 4: 3 pellets — 1×2 vertical + 2×1 upper-right + 3-px gentle bend lower
    &[
        &[(-2, -1), (-2, 0)],
        &[(0, -1), (1, -1)],
        &[(-1, 2), (0, 1), (1, 1)],
    ],
    // 5: 2 pellets — compact asymmetric pair (3×1 + 2-px slant)
    &[
        &[(-1, -1), (0, -1), (1, -1)],
        &[(-2, 1), (-1, 2)],
    ],
    // 6: 4 pellets — staggered 4-grain cluster (4-disconnected 2×1 grains)
    &[
        &[(0, -1), (1, -1)],
        &[(-2, 0), (-1, 0)],
        &[(1, 1), (2, 1)],
        &[(-1, 2), (0, 2)],
    ],
    // 7: 3 pellets — oblique trio (2×1 top, 1×2 right, 3-px curved bottom-left)
    &[
        &[(-1, -1), (0, -1)],
        &[(2, 0), (2, 1)],
        &[(-2, 1), (-1, 1), (0, 2)],
    ],
];

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
fn mix_seed(seed: u32) -> u32 {
    let mut n = seed ^ 0x9e37_79b9;
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}

/// Bitmask position for local offsets `(dx, dy)` in `-4..=3 × -4..=3`.
#[inline]
fn mask_bit(dx: i32, dy: i32) -> u64 {
    debug_assert!((-4..=3).contains(&dx) && (-4..=3).contains(&dy));
    1u64 << (((dy + 4) * 8 + (dx + 4)) as u32)
}

/// Draw one Hantavirus dropping on the floor at `d.pos`.
///
/// Every painted pixel (keyline, pellets, and optional live shimmer) stays
/// strictly within the 9×9 screen box centered on `w2s(d.pos, cam)`.
pub fn draw_dropping(pb: &mut PixelBuffer, d: &Dropping, cam: Vec2, elapsed: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let (sx, sy) = w2s(d.pos, cam);
    if sx.saturating_add(4) < 0
        || sy.saturating_add(4) < 0
        || sx.saturating_sub(4) >= pb.w as i32
        || sy.saturating_sub(4) >= pb.h as i32
    {
        return;
    }

    let h = mix_seed(d.seed);
    let layout_idx = (d.seed.wrapping_add(d.seed >> 4) as usize) % LAYOUTS.len();
    let flip_x = (((d.seed >> 3) ^ (h >> 5)) & 1) != 0;
    let layout = LAYOUTS[layout_idx];

    let wet = d.age < ARM_AGE;
    let drying = d.age > DRYING_AGE;

    let (pal, hi_col, under_alpha, right_alpha) = if wet {
        (PAL_WET, HI_WET, 0.72f32, 0.44f32)
    } else if drying {
        let dry_t = ((d.age - DRYING_AGE) / DRYING_SPAN).clamp(0.0, 1.0);
        let k = 0.58 + 0.42 * dry_t;
        let p = [
            (
                palette::mix(PAL_ARMED[0].0, PAL_DRY[0].0, k),
                palette::mix(PAL_ARMED[0].1, PAL_DRY[0].1, k),
            ),
            (
                palette::mix(PAL_ARMED[1].0, PAL_DRY[1].0, k),
                palette::mix(PAL_ARMED[1].1, PAL_DRY[1].1, k),
            ),
            (
                palette::mix(PAL_ARMED[2].0, PAL_DRY[2].0, k),
                palette::mix(PAL_ARMED[2].1, PAL_DRY[2].1, k),
            ),
        ];
        (p, palette::mix(HI_ARMED, HI_DRY, k), 0.58, 0.34)
    } else {
        (PAL_ARMED, HI_ARMED, 0.74, 0.45)
    };

    // Track occupied pellet cells in a stack bitmask so keylines never paint
    // over a neighbouring pellet.
    let mut occupied = 0u64;
    for pellet in layout {
        for &(rx, ry) in *pellet {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);
            occupied |= mask_bit(dx, dy);
        }
    }

    // 1. Clean 1-px lower-right dark keyline directly under each pellet pixel
    //    and off its rightmost tip, keeping gaps between grains open and crisp.
    let mut key_done = 0u64;
    for pellet in layout {
        let mut max_x = i32::MIN;
        for &(rx, _) in *pellet {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            if dx > max_x {
                max_x = dx;
            }
        }
        for &(rx, ry) in *pellet {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);

            let ub = mask_bit(dx, dy + 1);
            if (occupied | key_done) & ub == 0 {
                key_done |= ub;
                blend_px(pb, sx + dx, sy + dy + 1, KEYLINE_DARK, under_alpha);
            }

            if dx == max_x {
                let rb = mask_bit(dx + 1, dy);
                if (occupied | key_done) & rb == 0 {
                    key_done |= rb;
                    blend_px(pb, sx + dx + 1, sy + dy, KEYLINE_DARK, right_alpha);
                }
            }
        }
    }

    // 2. Pellet bodies: slightly varied dark browns per grain, shaded toward
    //    the lower-right tip, with a 1-px top-left highlight on one grain
    //    (and a fainter wet sheen on a second grain while fresh).
    let hi_pellet = ((h >> 8) as usize) % layout.len();
    let wet_pellet = (hi_pellet + 1) % layout.len();
    let mut top_y = 2i32;

    for (pi, pellet) in layout.iter().enumerate() {
        let tone = pal[(pi + ((h >> 2) as usize)) % pal.len()];
        let mut tl = (i32::MAX, i32::MAX);
        let mut br = (i32::MIN, i32::MIN);
        let mut br_key = i32::MIN;

        for &(rx, ry) in *pellet {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);
            if dy < top_y {
                top_y = dy;
            }
            if (dy, dx) < (tl.1, tl.0) {
                tl = (dx, dy);
            }
            let k = dx + dy;
            if (k, dx) > (br_key, br.0) {
                br_key = k;
                br = (dx, dy);
            }
        }

        for &(rx, ry) in *pellet {
            let dx = if flip_x { -i32::from(rx) } else { i32::from(rx) };
            let dy = i32::from(ry);
            let col = if (dx, dy) == tl && pi == hi_pellet {
                hi_col
            } else if wet && (dx, dy) == tl && pi == wet_pellet && wet_pellet != hi_pellet {
                palette::mix(tone.0, hi_col, 0.58)
            } else if (dx, dy) == br {
                tone.1
            } else {
                tone.0
            };
            pb.plot(sx + dx, sy + dy, col);
        }
    }

    // 3. Infectious tell (armed only: 0.25 <= age <= 12.0):
    //    1–2 pixels of faint sickly violet-green shimmer drifting 2–3 px above
    //    the cluster in held 4 Hz steps from `elapsed` + `d.seed`.
    if !wet && !drying {
        let tick = if elapsed.is_finite() && elapsed > 0.0 {
            (elapsed * 4.0).floor() as u32
        } else {
            0
        };
        let step = tick.wrapping_add((h >> 12) & 0x1f) % 6;
        let base_x = ((h >> 16) % 3) as i32 - 1;
        let alt_x = if base_x >= 0 { -1 } else { 1 };
        let y_low = (top_y - 2).clamp(-4, 4);
        let y_hi = (top_y - 3).clamp(-4, 4);

        match step {
            0 => {
                blend_px(pb, sx + base_x, sy + y_low, VIRUS_VIOLET, 0.62);
            }
            1 => {
                blend_px(pb, sx + base_x, sy + y_hi, VIRUS_VIOLET, 0.46);
                blend_px(pb, sx + alt_x, sy + y_low, VIRUS_GREEN, 0.58);
            }
            2 => {
                blend_px(pb, sx + alt_x, sy + y_hi, VIRUS_GREEN, 0.48);
            }
            3 => {
                blend_px(pb, sx + alt_x, sy + y_low, VIRUS_VIOLET, 0.56);
            }
            4 => {
                blend_px(pb, sx + alt_x, sy + y_hi, VIRUS_VIOLET, 0.44);
                blend_px(pb, sx + base_x, sy + y_low, VIRUS_GREEN, 0.58);
            }
            _ => {
                blend_px(pb, sx + base_x, sy + y_hi, VIRUS_GREEN, 0.46);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dropping(pos: Vec2, age: f32, seed: u32) -> Dropping {
        Dropping {
            pos,
            age,
            damage: 3.0,
            jumps: 1,
            reach: 14.0,
            radius: 7.0,
            seed,
        }
    }

    #[test]
    fn draws_inside_nine_by_nine_box_and_nothing_outside() {
        const BG: Rgb = (28, 34, 32);
        let cam = Vec2::new(10.0, 5.0);
        let pos = Vec2::new(26.0, 21.0); // screen (16, 16)
        let (sx, sy) = (16i32, 16i32);

        for seed in 0..24u32 {
            for &age in &[0.05f32, 0.24, 0.25, 3.5, 12.0, 12.1, 13.8] {
                for &elapsed in &[0.0f32, 0.25, 0.5, 0.75, 1.0, 1.25] {
                    let mut pb = PixelBuffer::new(33, 33);
                    pb.clear(BG);
                    let d = sample_dropping(pos, age, seed);
                    draw_dropping(&mut pb, &d, cam, elapsed);

                    let mut inside = 0usize;
                    for y in 0..pb.h {
                        for x in 0..pb.w {
                            let changed = pb.pixel_at(x, y) != BG;
                            let in_box = (x as i32 - sx).abs() <= 4 && (y as i32 - sy).abs() <= 4;
                            if in_box {
                                if changed {
                                    inside += 1;
                                }
                            } else {
                                assert!(
                                    !changed,
                                    "seed={seed} age={age} elapsed={elapsed} painted outside 9x9 at ({x},{y})"
                                );
                            }
                        }
                    }
                    assert!(
                        inside >= 5,
                        "seed={seed} age={age} elapsed={elapsed} drew too few pixels ({inside})"
                    );
                }
            }
        }
    }

    #[test]
    fn different_seeds_differ() {
        const BG: Rgb = (24, 28, 26);
        let pos = Vec2::new(12.0, 12.0);
        let render = |seed: u32| -> Vec<Rgb> {
            let mut pb = PixelBuffer::new(25, 25);
            pb.clear(BG);
            let d = sample_dropping(pos, 3.0, seed);
            draw_dropping(&mut pb, &d, Vec2::ZERO, 0.0);
            pb.pixels().to_vec()
        };

        let frames: Vec<Vec<Rgb>> = (0..16u32).map(render).collect();
        for i in 0..frames.len() {
            for j in (i + 1)..frames.len() {
                assert_ne!(
                    frames[i], frames[j],
                    "seeds {i} and {j} produced identical output"
                );
            }
        }
    }

    #[test]
    fn age_past_twelve_changes_colours_and_stops_shimmer() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        let mut live_pb = PixelBuffer::new(25, 25);
        live_pb.clear(BG);
        draw_dropping(&mut live_pb, &sample_dropping(pos, 4.0, 0), Vec2::ZERO, 0.25);

        let mut dry_pb = PixelBuffer::new(25, 25);
        dry_pb.clear(BG);
        draw_dropping(&mut dry_pb, &sample_dropping(pos, 12.6, 0), Vec2::ZERO, 0.25);

        assert_ne!(
            live_pb.pixels(),
            dry_pb.pixels(),
            "drying dropping (age > 12) must differ from armed dropping"
        );

        // Pellets (rows 11..=14) lighten toward grey-brown when drying.
        let pellet_luma = |pb: &PixelBuffer| -> u32 {
            let mut sum = 0u32;
            for y in 11..=14usize {
                for x in 10..=14usize {
                    let c = pb.pixel_at(x, y);
                    sum += u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
                }
            }
            sum
        };
        assert!(
            pellet_luma(&dry_pb) > pellet_luma(&live_pb),
            "drying pellets should lighten toward grey-brown"
        );

        // Shimmer zone (rows 8..=9, 2–3 px above the cluster): present when live,
        // completely stopped once drying (age > 12).
        let shimmer_pixels = |pb: &PixelBuffer| -> usize {
            (8..=9usize)
                .flat_map(|y| (8..=16usize).map(move |x| (x, y)))
                .filter(|&(x, y)| pb.pixel_at(x, y) != BG)
                .count()
        };
        assert!(
            shimmer_pixels(&live_pb) >= 1,
            "live armed dropping should show 1–2 shimmer pixels above it"
        );
        assert_eq!(
            shimmer_pixels(&dry_pb),
            0,
            "drying dropping (age > 12) must stop its shimmer"
        );
    }

    #[test]
    fn fresh_unarmed_dropping_is_brighter_without_shimmer() {
        const BG: Rgb = (20, 24, 22);
        let pos = Vec2::new(12.0, 12.0);

        let mut wet_pb = PixelBuffer::new(25, 25);
        wet_pb.clear(BG);
        draw_dropping(&mut wet_pb, &sample_dropping(pos, 0.1, 3), Vec2::ZERO, 0.5);

        let mut live_pb = PixelBuffer::new(25, 25);
        live_pb.clear(BG);
        draw_dropping(&mut live_pb, &sample_dropping(pos, 2.0, 3), Vec2::ZERO, 0.5);

        let shimmer_wet = (8..=9usize)
            .flat_map(|y| (8..=16usize).map(move |x| (x, y)))
            .filter(|&(x, y)| wet_pb.pixel_at(x, y) != BG)
            .count();
        assert_eq!(shimmer_wet, 0, "fresh dropping (age < 0.25) has no shimmer");

        let pellet_luma = |pb: &PixelBuffer| -> u32 {
            (11..=14usize)
                .flat_map(|y| (10..=14usize).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let c = pb.pixel_at(x, y);
                    u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
                })
                .sum()
        };
        assert!(
            pellet_luma(&wet_pb) > pellet_luma(&live_pb),
            "fresh dropping (age < 0.25) should draw slightly brighter/wet"
        );
    }

    #[test]
    fn shimmer_holds_within_four_hz_step_and_advances_between_steps() {
        const BG: Rgb = (20, 24, 22);
        let d = sample_dropping(Vec2::new(12.0, 12.0), 4.0, 5);

        let render_at = |elapsed: f32| -> Vec<Rgb> {
            let mut pb = PixelBuffer::new(25, 25);
            pb.clear(BG);
            draw_dropping(&mut pb, &d, Vec2::ZERO, elapsed);
            pb.pixels().to_vec()
        };

        assert_eq!(
            render_at(1.00),
            render_at(1.20),
            "shimmer must hold steady inside one 4 Hz (0.25 s) step"
        );
        assert_ne!(
            render_at(1.00),
            render_at(1.25),
            "shimmer must step on the next 4 Hz tick"
        );
    }

    #[test]
    fn no_panic_at_buffer_edges_or_empty_buffer() {
        let mut pb = PixelBuffer::new(8, 8);
        for seed in 0..8u32 {
            for &pos in &[
                Vec2::new(0.0, 0.0),
                Vec2::new(7.0, 7.0),
                Vec2::new(0.0, 7.0),
                Vec2::new(7.0, 0.0),
                Vec2::new(-1.0, 4.0),
                Vec2::new(4.0, -1.0),
                Vec2::new(8.0, 4.0),
                Vec2::new(4.0, 8.0),
                Vec2::new(-40.0, -40.0),
                Vec2::new(200.0, 200.0),
            ] {
                let d = sample_dropping(pos, 3.0, seed);
                draw_dropping(&mut pb, &d, Vec2::ZERO, 0.5);
                draw_dropping(&mut pb, &d, Vec2::ZERO, f32::NAN);
            }
        }

        let mut zero = PixelBuffer::new(0, 0);
        draw_dropping(&mut zero, &sample_dropping(Vec2::ZERO, 3.0, 0), Vec2::ZERO, 0.0);
    }
}
