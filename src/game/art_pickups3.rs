//! Pixel art for the three new pocket-find pickups:
//! [`PickupKind::ShrinkDraught`], [`PickupKind::WhiteCoat`], and
//! [`PickupKind::TheCollar`].
//!
//! Pure render-only drawing (zero rng, zero heap allocation). Each find is a
//! crisp still image with top-left directional lighting, a 1 px dark contour
//! rim, and a grounded floor contact shadow matching the style and shared
//! 13×13 px footprint (`dx, dy in -6..=6`) of [`super::art_floor::draw_pickup`].
//! The pulsing floor halo is drawn by the caller (`World::draw_scene`).

use crate::game::gem::PickupKind;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Subtle floor shadow helper (darkens an existing pixel in bounds), matching
/// `art_floor::shade_floor`.
#[inline]
fn shade_floor(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let cur = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(cur, 1.0 - amount));
}

/// Draw one of the three batch pocket finds at screen position `s` (already
/// `w2s`-mapped). Any other [`PickupKind`] is a no-op so the caller can chain
/// this alongside `art_floor::draw_pickup`.
pub fn draw_pickup3(pb: &mut PixelBuffer, s: (i32, i32), kind: PickupKind, elapsed: f32) {
    let _ = elapsed;
    match kind {
        PickupKind::ShrinkDraught => draw_shrink_draught(pb, s),
        PickupKind::WhiteCoat => draw_white_coat(pb, s),
        PickupKind::TheCollar => draw_the_collar(pb, s),
        _ => {}
    }
}

/// Shrink Draught: a small pinched-neck clinical vial of luminous cold-white
/// draught with a brushed-steel crimp cap and a tiny dosage tick on the glass.
///
/// Deliberately narrower and shorter (`7×10` px body, `dx in -3..=3`,
/// `dy in -5..=5` including shadow) than `GrowthSerum`'s squat corked pink
/// flask so the pair read as opposites at a glance.
fn draw_shrink_draught(pb: &mut PixelBuffer, s: (i32, i32)) {
    let (cx, cy) = s;
    for dx in -2..=3 {
        shade_floor(pb, cx + dx, cy + 5, 0.44);
    }

    let out: Rgb = (20, 26, 38);
    let cap_hi: Rgb = (196, 212, 228);
    let cap_mid: Rgb = (128, 146, 168);
    let cap_lo: Rgb = (74, 88, 110);
    let glass_hi: Rgb = (222, 242, 255);
    let glass_air: Rgb = (42, 58, 78);
    let glass_lo: Rgb = (104, 138, 170);
    let liq_hi: Rgb = (254, 255, 255);
    let liq_mid: Rgb = (206, 236, 252);
    let liq_lo: Rgb = (134, 178, 212);

    // 1. Dark contour: flared clinical cap over a pinched neck and slender cylinder.
    pb.rect_fill(cx - 2, cy - 5, 5, 2, out);
    pb.rect_fill(cx - 1, cy - 3, 3, 1, out);
    pb.rect_fill(cx - 2, cy - 2, 5, 1, out);
    pb.rect_fill(cx - 3, cy - 1, 7, 5, out);
    pb.rect_fill(cx - 2, cy + 4, 5, 1, out);

    // 2. Brushed steel/slate crimp cap (clinical contrast to Growth Serum's cork).
    pb.plot(cx - 1, cy - 5, cap_hi);
    pb.plot(cx, cy - 5, cap_mid);
    pb.plot(cx + 1, cy - 5, cap_lo);
    pb.plot(cx - 1, cy - 4, cap_mid);
    pb.plot(cx, cy - 4, cap_lo);
    pb.plot(cx + 1, cy - 4, cap_lo);

    // 3. Pinched glass neck and small dark headspace above the meniscus.
    pb.plot(cx, cy - 3, glass_hi);
    pb.plot(cx - 1, cy - 2, glass_hi);
    pb.plot(cx, cy - 2, glass_air);
    pb.plot(cx + 1, cy - 2, glass_lo);

    // 4. Cold-white draught body: bright white meniscus, luminous core, cool
    //    ice-blue glass shading, and a 1 px dosage graduation tick on the right.
    pb.plot(cx - 2, cy - 1, glass_hi);
    pb.plot(cx - 1, cy - 1, liq_hi);
    pb.plot(cx, cy - 1, liq_hi);
    pb.plot(cx + 1, cy - 1, liq_mid);
    pb.plot(cx + 2, cy - 1, glass_lo);

    pb.plot(cx - 2, cy, liq_hi);
    pb.plot(cx - 1, cy, liq_hi);
    pb.plot(cx, cy, liq_hi);
    pb.plot(cx + 1, cy, liq_mid);
    pb.plot(cx + 2, cy, glass_lo);

    pb.plot(cx - 2, cy + 1, glass_hi);
    pb.plot(cx - 1, cy + 1, liq_hi);
    pb.plot(cx, cy + 1, liq_mid);
    pb.plot(cx + 1, cy + 1, liq_mid);
    pb.plot(cx + 2, cy + 1, cap_lo);

    pb.plot(cx - 2, cy + 2, glass_hi);
    pb.plot(cx - 1, cy + 2, liq_mid);
    pb.plot(cx, cy + 2, liq_mid);
    pb.plot(cx + 1, cy + 2, liq_lo);
    pb.plot(cx + 2, cy + 2, glass_lo);

    pb.plot(cx - 2, cy + 3, glass_lo);
    pb.plot(cx - 1, cy + 3, liq_lo);
    pb.plot(cx, cy + 3, liq_lo);
    pb.plot(cx + 1, cy + 3, glass_lo);
    pb.plot(cx + 2, cy + 3, cap_lo);

    // 5. Faint cold-white emissive core glint (parallel to Growth Serum's glow).
    pb.plot_add(cx, cy, (36, 48, 60));
}

/// White Coat: a neatly folded paper-white lab coat (`11×9` px body,
/// `dx in -5..=5`, `dy in -4..=5` including shadow) with a dipped V-neck
/// collar notch, turned lapels, center button placket, clipped blue lab pen in
/// the breast pocket, and a crisp horizontal fold crease across the lower hem.
fn draw_white_coat(pb: &mut PixelBuffer, s: (i32, i32)) {
    let (cx, cy) = s;
    for dx in -4..=5 {
        shade_floor(pb, cx + dx, cy + 5, 0.44);
    }
    shade_floor(pb, cx + 5, cy + 3, 0.30);
    shade_floor(pb, cx + 5, cy + 4, 0.34);

    let out: Rgb = (24, 30, 42);
    let paper_hi: Rgb = (254, 254, 252);
    let paper_mid: Rgb = (232, 236, 242);
    let paper_lo: Rgb = (176, 184, 198);
    let crease: Rgb = (120, 130, 148);
    let neck_dark: Rgb = (42, 50, 66);
    let button: Rgb = (96, 106, 126);
    let pen_teal: Rgb = (62, 152, 214);

    // 1. Outer contour with shoulder peaks and a center V-collar dip at y = -4.
    pb.rect_fill(cx - 4, cy - 4, 4, 1, out);
    pb.rect_fill(cx + 1, cy - 4, 4, 1, out);
    pb.rect_fill(cx - 5, cy - 3, 11, 7, out);
    pb.rect_fill(cx - 4, cy + 4, 9, 1, out);

    // 2. Crisp paper-white folded cloth torso, lit from top-left.
    pb.rect_fill(cx - 4, cy - 3, 9, 6, paper_mid);
    pb.rect_fill(cx - 4, cy - 3, 6, 4, paper_hi);

    // Tucked-under sleeve edges along the outer columns.
    pb.plot(cx - 4, cy - 2, paper_hi);
    pb.plot(cx - 4, cy - 1, paper_mid);
    pb.plot(cx - 4, cy, paper_mid);
    pb.plot(cx - 4, cy + 1, paper_lo);
    pb.plot(cx + 4, cy - 2, paper_lo);
    pb.plot(cx + 4, cy - 1, paper_lo);
    pb.plot(cx + 4, cy, paper_lo);
    pb.plot(cx + 4, cy + 1, crease);

    // 3. Notched V-neck opening and broad turned lapels.
    pb.plot(cx, cy - 3, out);
    pb.plot(cx - 1, cy - 3, neck_dark);
    pb.plot(cx + 1, cy - 3, neck_dark);
    pb.plot(cx, cy - 2, neck_dark);

    pb.plot(cx - 3, cy - 3, paper_hi);
    pb.plot(cx - 2, cy - 3, paper_hi);
    pb.plot(cx - 2, cy - 2, paper_hi);
    pb.plot(cx - 1, cy - 2, paper_hi);
    pb.plot(cx + 2, cy - 3, paper_hi);
    pb.plot(cx + 3, cy - 3, paper_mid);
    pb.plot(cx + 1, cy - 2, paper_hi);
    pb.plot(cx + 2, cy - 2, paper_mid);

    pb.plot(cx - 2, cy - 1, paper_lo);
    pb.plot(cx - 1, cy - 1, crease);
    pb.plot(cx + 1, cy - 1, crease);
    pb.plot(cx + 2, cy - 1, paper_lo);

    // 4. Center front placket seam and two small coat buttons.
    pb.plot(cx, cy - 1, button);
    pb.plot(cx, cy, paper_lo);
    pb.plot(cx, cy + 1, button);

    // 5. Right breast pocket slit with a clipped blue lab pen.
    pb.plot(cx + 2, cy, pen_teal);
    pb.plot(cx + 2, cy + 1, crease);
    pb.plot(cx + 3, cy + 1, crease);

    // 6. Horizontal fold crease and folded lower cloth edge.
    pb.rect_fill(cx - 4, cy + 2, 9, 1, crease);
    pb.plot(cx - 4, cy + 2, paper_lo);
    pb.plot(cx - 3, cy + 2, paper_lo);
    pb.rect_fill(cx - 4, cy + 3, 9, 1, paper_lo);
    pb.rect_fill(cx - 3, cy + 3, 4, 1, paper_mid);
    pb.plot(cx + 4, cy + 3, crease);
}

/// The Collar: an open dark slate-charcoal leather collar loop (`11×9` px,
/// `dx in -5..=5`, `dy in -4..=5` including shadow) with a hollow neck opening
/// that shows the floor through its center, a brass keeper rivet, an eyelet
/// hole, and a gleaming pale-gold front buckle catching a sharp neck-glint.
fn draw_the_collar(pb: &mut PixelBuffer, s: (i32, i32)) {
    let (cx, cy) = s;
    for dx in -4..=-2 {
        shade_floor(pb, cx + dx, cy + 4, 0.42);
    }
    for dx in 2..=4 {
        shade_floor(pb, cx + dx, cy + 4, 0.42);
    }
    for dx in -1..=2 {
        shade_floor(pb, cx + dx, cy + 5, 0.46);
    }
    shade_floor(pb, cx + 5, cy, 0.32);
    shade_floor(pb, cx + 5, cy + 1, 0.36);

    let out: Rgb = (14, 14, 22);
    let band_hi: Rgb = (110, 116, 134);
    let band_mid: Rgb = (60, 64, 78);
    let band_lo: Rgb = (34, 38, 48);
    let glint: Rgb = (255, 255, 244);
    let bkl_hi: Rgb = (252, 228, 144);
    let bkl_mid: Rgb = (208, 164, 66);
    let bkl_lo: Rgb = (132, 94, 34);

    // 1. Oval collar ring contour with an open hollow center (x in -2..=2, y in -1..=0).
    pb.rect_fill(cx - 3, cy - 4, 7, 1, out);
    pb.rect_fill(cx - 5, cy - 3, 11, 1, out);
    pb.rect_fill(cx - 3, cy - 2, 7, 1, out);
    pb.rect_fill(cx - 5, cy - 2, 3, 4, out);
    pb.rect_fill(cx + 3, cy - 2, 3, 4, out);
    pb.rect_fill(cx - 4, cy + 1, 9, 2, out);
    pb.rect_fill(cx - 3, cy + 3, 7, 1, out);
    pb.rect_fill(cx - 2, cy + 4, 5, 1, out);

    // 2. Dark oiled leather strap body with top-left bevel lighting.
    for (dx, col) in [
        (-3, band_hi),
        (-2, band_hi),
        (-1, band_hi),
        (0, band_mid),
        (1, band_mid),
        (2, band_mid),
        (3, band_lo),
    ] {
        pb.plot(cx + dx, cy - 3, col);
    }

    // Left curve.
    pb.plot(cx - 4, cy - 2, band_hi);
    pb.plot(cx - 3, cy - 2, band_mid);
    pb.plot(cx - 4, cy - 1, band_hi);
    pb.plot(cx - 3, cy - 1, band_lo);
    pb.plot(cx - 4, cy, band_mid);
    pb.plot(cx - 3, cy, band_lo);
    pb.plot(cx - 4, cy + 1, band_lo);
    pb.plot(cx - 3, cy + 1, band_mid);

    // Right curve.
    pb.plot(cx + 3, cy - 2, band_mid);
    pb.plot(cx + 4, cy - 2, band_lo);
    pb.plot(cx + 3, cy - 1, band_lo);
    pb.plot(cx + 4, cy - 1, band_mid);
    pb.plot(cx + 3, cy, band_lo);
    pb.plot(cx + 4, cy, band_mid);
    pb.plot(cx + 3, cy + 1, band_mid);
    pb.plot(cx + 4, cy + 1, band_lo);

    // Front leather band flanking the buckle, plus brass keeper stud & eyelet hole.
    pb.plot(cx - 3, cy + 2, band_lo);
    pb.plot(cx - 2, cy + 1, band_hi);
    pb.plot(cx - 2, cy + 2, band_mid);
    pb.plot(cx + 2, cy + 1, band_mid);
    pb.plot(cx + 2, cy + 2, band_lo);
    pb.plot(cx + 3, cy + 2, band_lo);
    pb.plot(cx - 3, cy + 1, bkl_mid);
    pb.plot(cx + 3, cy + 1, out);

    // 3. Gleaming front neck-glint buckle with dark pin slot and sharp corner glint.
    pb.plot(cx - 1, cy + 1, glint);
    pb.plot(cx, cy + 1, bkl_hi);
    pb.plot(cx + 1, cy + 1, bkl_hi);
    pb.plot(cx - 1, cy + 2, bkl_hi);
    pb.plot(cx, cy + 2, out);
    pb.plot(cx + 1, cy + 2, bkl_mid);
    pb.plot(cx - 1, cy + 3, bkl_mid);
    pb.plot(cx, cy + 3, bkl_mid);
    pb.plot(cx + 1, cy + 3, bkl_lo);

    // Razor-sharp 1 px vertical glint spike off the buckle's lit corner.
    pb.plot(cx - 1, cy, mix(bkl_hi, glint, 0.72));
    pb.plot_add(cx - 1, cy + 1, (48, 44, 28));
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_KINDS: [PickupKind; 3] = [
        PickupKind::ShrinkDraught,
        PickupKind::WhiteCoat,
        PickupKind::TheCollar,
    ];

    /// Each of the three new pocket finds paints a non-empty sprite strictly
    /// inside the shared 13×13 pickup footprint (`10..=22` around `(16, 16)`),
    /// matching `art_floor::tests::pickups_have_pairwise_distinct_silhouettes`.
    #[test]
    fn each_kind_paints_inside_the_shared_footprint() {
        const BG: Rgb = (1, 2, 3);
        for kind in NEW_KINDS {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear(BG);
            draw_pickup3(&mut pb, (16, 16), kind, 0.0);

            let mut painted = 0usize;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let changed = pb.pixel_at(x, y) != BG;
                    let inside = (10..=22).contains(&x) && (10..=22).contains(&y);
                    assert!(
                        inside || !changed,
                        "{kind:?} painted outside shared 13x13 footprint at ({x}, {y})"
                    );
                    if changed {
                        painted += 1;
                    }
                }
            }
            assert!(
                painted >= 40,
                "{kind:?} painted too few pixels ({painted})"
            );
        }
    }

    /// The three new finds have pairwise distinct silhouettes and do not share
    /// a silhouette with any of the eight earlier pickups.
    #[test]
    fn new_finds_have_pairwise_distinct_silhouettes() {
        const BG: Rgb = (1, 2, 3);
        let new_shapes: Vec<(PickupKind, Vec<bool>)> = NEW_KINDS
            .iter()
            .map(|&kind| {
                let mut pb = PixelBuffer::new(32, 32);
                pb.clear(BG);
                draw_pickup3(&mut pb, (16, 16), kind, 0.0);
                (kind, pb.pixels().iter().map(|&p| p != BG).collect())
            })
            .collect();

        for i in 0..new_shapes.len() {
            for j in i + 1..new_shapes.len() {
                assert_ne!(
                    new_shapes[i].1, new_shapes[j].1,
                    "{:?} and {:?} share a silhouette",
                    new_shapes[i].0, new_shapes[j].0
                );
            }
        }

        let earlier = [
            PickupKind::Heal,
            PickupKind::Magnet,
            PickupKind::Nuke,
            PickupKind::BottleCap,
            PickupKind::WindupMouse,
            PickupKind::PocketWatch,
            PickupKind::CleanSocks,
            PickupKind::GrowthSerum,
        ];
        for old_kind in earlier {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear(BG);
            crate::game::art_floor::draw_pickup(&mut pb, old_kind, (16, 16), 0.0);
            let old_shape: Vec<bool> = pb.pixels().iter().map(|&p| p != BG).collect();
            for (new_kind, new_shape) in &new_shapes {
                assert_ne!(
                    new_shape, &old_shape,
                    "{new_kind:?} shares a silhouette with {old_kind:?}"
                );
            }
        }
    }

    /// Any pickup kind outside the three new finds falls through to a no-op.
    #[test]
    fn other_kinds_fall_through_to_nothing() {
        const BG: Rgb = (14, 22, 20);
        let others = [
            PickupKind::Heal,
            PickupKind::Magnet,
            PickupKind::Nuke,
            PickupKind::BottleCap,
            PickupKind::WindupMouse,
            PickupKind::PocketWatch,
            PickupKind::CleanSocks,
            PickupKind::GrowthSerum,
        ];
        for kind in others {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear(BG);
            draw_pickup3(&mut pb, (16, 16), kind, 1.5);
            assert!(
                pb.pixels().iter().all(|&p| p == BG),
                "{kind:?} should be a no-op in draw_pickup3"
            );
        }
    }

    /// All three sprites are still images (no stepped animation or clock drift).
    #[test]
    fn still_images_are_invariant_over_elapsed() {
        const BG: Rgb = (14, 22, 20);
        for kind in NEW_KINDS {
            let mut base = PixelBuffer::new(32, 32);
            base.clear(BG);
            draw_pickup3(&mut base, (16, 16), kind, 0.0);
            for elapsed in [0.25f32, 1.37, 8.9, 120.0] {
                let mut pb = PixelBuffer::new(32, 32);
                pb.clear(BG);
                draw_pickup3(&mut pb, (16, 16), kind, elapsed);
                assert_eq!(
                    pb.pixels(),
                    base.pixels(),
                    "{kind:?} changed at elapsed={elapsed}"
                );
            }
        }
    }

    /// Drawing at and past every buffer edge (and on an empty buffer) clips
    /// cleanly without panicking.
    #[test]
    fn clips_at_buffer_edges_and_empty_buffer() {
        let mut pb = PixelBuffer::new(8, 8);
        for kind in NEW_KINDS {
            for s in [
                (0, 0),
                (7, 7),
                (-3, 4),
                (4, -3),
                (11, 4),
                (4, 11),
                (-40, -40),
            ] {
                draw_pickup3(&mut pb, s, kind, 1.3);
            }
        }
        let mut empty = PixelBuffer::new(0, 0);
        for kind in NEW_KINDS {
            draw_pickup3(&mut empty, (0, 0), kind, 0.0);
        }
    }
}
