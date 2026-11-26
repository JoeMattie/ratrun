//! Art for decorative Laundry clothes piles and overflowing baskets.
//!
//! Pure static drawing: heaps of mixed garments (socks, shirts, towels, a
//! draped bedsheet) and woven baskets overflowing with laundry, painted onto
//! the floor right after puddles and before bodies so the rat and enemies walk
//! cleanly over them.
//!
//! Contrast is deliberately muted and dusty (harmonizing with the Laundry's
//! indigo tile floor) so heaps never compete visually with bright enemies,
//! pickups or hostile telegraphs.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::level::{ClothesPile, ClothesPileKind, Palette};

/// Half-extent draw margin in pixels, used for on-screen culling.
pub const MAX_PILE_RADIUS_PX: f32 = 14.0;

/// Muted laundry fabric colour triplets: `(shadow, mid, highlight)`.
/// Kept below peak luminance ~155 so heaps never pop like enemies or telegraphs.
const CLOTH_SWATCHES: [(Rgb, Rgb, Rgb); 7] = [
    // 0: Faded denim blue
    ((46, 62, 86), (72, 96, 124), (96, 122, 150)),
    // 1: Ruined dusty pink
    ((86, 54, 72), (126, 84, 106), (154, 108, 130)),
    // 2: Greyed off-white cotton / sheet
    ((80, 88, 100), (118, 126, 136), (146, 154, 162)),
    // 3: Muted scrub teal / sage
    ((46, 70, 74), (72, 104, 104), (96, 130, 128)),
    // 4: Dusty warm ochre towel
    ((80, 64, 48), (120, 98, 74), (146, 122, 94)),
    // 5: Deep plum / navy sock
    ((46, 42, 68), (74, 66, 102), (98, 88, 128)),
    // 6: Muted brick / faded coral shirt
    ((78, 48, 48), (118, 76, 74), (144, 96, 92)),
];

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

#[inline]
fn swatch(seed: u32, slot: u32, pal: &Palette) -> (Rgb, Rgb, Rgb) {
    let idx = (hash_u32(seed.wrapping_add(slot.wrapping_mul(0x9E37_79B9))) as usize)
        % CLOTH_SWATCHES.len();
    let (lo, mid, hi) = CLOTH_SWATCHES[idx];
    // Gently pull shadows toward the floor indigo so piles feel grounded.
    (
        palette::mix(lo, pal.bg, 0.24),
        palette::mix(mid, pal.bg, 0.10),
        palette::mix(hi, pal.bg, 0.06),
    )
}

#[inline]
fn darken_floor(pb: &mut PixelBuffer, x: i32, y: i32, factor: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, palette::scale(cur, factor));
}

/// Soft contact shadow under a heap or basket, stamped with floor pen `0`.
fn ground_shadow(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
    let prev = pb.set_pen(0);
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(bw - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(bh - 1);
    for sy in y0..=y1 {
        let ny = (sy as f32 + 0.5 - cy) / ry.max(0.5);
        for sx in x0..=x1 {
            let nx = (sx as f32 + 0.5 - cx) / rx.max(0.5);
            let d2 = nx * nx + ny * ny;
            if d2 <= 1.0 {
                let fall = (1.0 - d2).sqrt();
                darken_floor(pb, sx, sy, 1.0 - strength * fall);
            }
        }
    }
    pb.set_pen(prev);
}

/// Paint one soft folded cloth mound (ellipse with organic lobe wobble, top-left
/// shading, and subtle diagonal fold creases).
#[allow(clippy::too_many_arguments)]
fn draw_cloth_lump(
    pb: &mut PixelBuffer,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    colors: (Rgb, Rgb, Rgb),
    outline: Rgb,
    fold_phase: f32,
    fold_dir: f32,
) {
    let (lo, mid, hi) = colors;
    let x0 = (cx - rx - 1.5).floor() as i32;
    let x1 = (cx + rx + 1.5).ceil() as i32;
    let y0 = (cy - ry - 1.5).floor() as i32;
    let y1 = (cy + ry + 1.5).ceil() as i32;
    for py in y0..=y1 {
        let ny = (py as f32 + 0.5 - cy) / ry.max(0.5);
        for px in x0..=x1 {
            let nx = (px as f32 + 0.5 - cx) / rx.max(0.5);
            let ang = ny.atan2(nx);
            let wobble = 0.88 + 0.12 * (ang * 3.0 + fold_phase).sin();
            let d = (nx * nx + ny * ny).sqrt() / wobble;
            if d > 1.08 {
                continue;
            }
            if d > 0.88 {
                pb.plot(px, py, outline);
                continue;
            }
            // Top-left key light + gentle cloth fold wave across the mound.
            let lit = (-nx * 0.52 - ny * 0.68).clamp(-1.0, 1.0);
            let fold = ((nx * fold_dir + ny * 1.15) * 3.4 + fold_phase).sin();
            let tone = (lit * 0.65 + fold * 0.35).clamp(-1.0, 1.0);
            let col = if tone > 0.28 {
                palette::mix(mid, hi, ((tone - 0.28) / 0.72).clamp(0.0, 1.0))
            } else if tone < -0.22 {
                palette::mix(mid, lo, ((-tone - 0.22) / 0.78).clamp(0.0, 1.0))
            } else {
                mid
            };
            pb.plot(px, py, col);
        }
    }
}

/// Paint a small stray sock with a contrasting heel/toe cuff peeking out of a pile.
fn draw_stray_sock(
    pb: &mut PixelBuffer,
    sx: i32,
    sy: i32,
    flip_x: i32,
    body: Rgb,
    tip: Rgb,
    outline: Rgb,
) {
    // 4-pixel curved L-shape: cuff -> ankle -> heel -> toe.
    let pts = [
        (0, 0, tip),
        (flip_x, 0, body),
        (flip_x * 2, 1, body),
        (flip_x * 3, 1, tip),
    ];
    // Soft underside pixel so the sock separates from the tile floor.
    pb.plot(sx + flip_x, sy + 1, outline);
    pb.plot(sx + flip_x * 2, sy + 2, outline);
    for (dx, dy, col) in pts {
        pb.plot(sx + dx, sy + dy, col);
    }
}

/// Paint one decorative clothes pile or overflowing basket at its world position.
///
/// Static (reads no clock) and deterministic from `pile.seed`. Caller sets the
/// material pen (`render::lighting::cloth_pen()`).
pub fn draw_clothes_pile(pb: &mut PixelBuffer, pile: &ClothesPile, cam: Vec2, pal: &Palette) {
    let cx = (pile.pos.x - cam.x).round() as i32;
    let cy = (pile.pos.y - cam.y).round() as i32;
    if cx < -16 || cy < -16 || cx >= pb.w as i32 + 16 || cy >= pb.h as i32 + 16 {
        return;
    }

    let seed = pile.seed;
    let flip = if (seed & 1) == 0 { 1 } else { -1 };
    let phase = ((seed >> 2) & 63) as f32 * 0.10;
    let outline = palette::mix((22, 24, 38), pal.bg, 0.35);

    let c0 = swatch(seed, 0, pal);
    let mut c1 = swatch(seed, 1, pal);
    if c1.1 == c0.1 {
        c1 = swatch(seed, 3, pal);
    }
    let mut c2 = swatch(seed, 2, pal);
    if c2.1 == c0.1 || c2.1 == c1.1 {
        c2 = swatch(seed, 5, pal);
    }
    let sheet = (
        palette::mix(CLOTH_SWATCHES[2].0, pal.bg, 0.24),
        palette::mix(CLOTH_SWATCHES[2].1, pal.bg, 0.12),
        palette::mix(CLOTH_SWATCHES[2].2, pal.bg, 0.08),
    );

    match pile.kind {
        ClothesPileKind::SmallHeap => {
            let fcx = cx as f32;
            let fcy = cy as f32;
            ground_shadow(pb, fcx + 0.8, fcy + 1.4, 5.4, 3.2, 0.34);
            // Main folded garment + smaller tucked garment + stray sock.
            draw_cloth_lump(pb, fcx - flip as f32 * 0.6, fcy + 0.3, 4.4, 2.7, c0, outline, phase, flip as f32);
            draw_cloth_lump(pb, fcx + flip as f32 * 1.6, fcy - 0.8, 2.6, 1.9, c1, outline, phase + 1.4, -flip as f32);
            draw_stray_sock(pb, cx + flip * 2, cy + 1, flip, c2.1, c2.2, outline);
        }
        ClothesPileKind::MediumHeap => {
            let fcx = cx as f32;
            let fcy = cy as f32;
            ground_shadow(pb, fcx + 1.0, fcy + 2.0, 7.8, 4.6, 0.38);
            // Layered 3-garment heap: wide towel base, contrasting shirt, top fold + stray sock.
            draw_cloth_lump(pb, fcx - flip as f32 * 1.4, fcy + 0.8, 6.2, 3.6, c0, outline, phase, flip as f32);
            draw_cloth_lump(pb, fcx + flip as f32 * 1.8, fcy + 0.2, 4.8, 3.1, c1, outline, phase + 2.1, -flip as f32);
            draw_cloth_lump(pb, fcx - flip as f32 * 0.3, fcy - 1.7, 3.8, 2.5, c2, outline, phase + 4.0, flip as f32);
            draw_stray_sock(pb, cx - flip * 5, cy + 2, -flip, c1.1, sheet.2, outline);
        }
        ClothesPileKind::LargeHeap => {
            let fcx = cx as f32;
            let fcy = cy as f32;
            ground_shadow(pb, fcx + 1.2, fcy + 2.4, 10.2, 5.8, 0.40);
            // Draped greyed bedsheet corner spread underneath, two tumbled garments atop.
            draw_cloth_lump(pb, fcx + flip as f32 * 1.2, fcy + 1.2, 8.6, 4.6, sheet, outline, phase + 0.6, flip as f32);
            draw_cloth_lump(pb, fcx - flip as f32 * 2.6, fcy + 0.2, 5.4, 3.6, c0, outline, phase + 2.3, -flip as f32);
            draw_cloth_lump(pb, fcx + flip as f32 * 2.2, fcy - 1.2, 4.9, 3.2, c1, outline, phase + 4.1, flip as f32);
            draw_cloth_lump(pb, fcx - flip as f32 * 0.6, fcy - 2.4, 3.6, 2.3, c2, outline, phase + 5.2, -flip as f32);
            draw_stray_sock(pb, cx + flip * 5, cy + 3, flip, c0.1, c0.2, outline);
        }
        ClothesPileKind::Basket => {
            draw_overflowing_basket(pb, cx, cy, flip, phase, c0, c1, c2, outline, pal);
        }
    }
}

/// Woven rattan/wire laundry basket overflowing with mixed clothes and a
/// trailing shirt sleeve draped over its front rim (~14 × 13 px).
#[allow(clippy::too_many_arguments)]
fn draw_overflowing_basket(
    pb: &mut PixelBuffer,
    cx: i32,
    cy: i32,
    flip: i32,
    phase: f32,
    c0: (Rgb, Rgb, Rgb),
    c1: (Rgb, Rgb, Rgb),
    c2: (Rgb, Rgb, Rgb),
    outline: Rgb,
    pal: &Palette,
) {
    let fcx = cx as f32;
    let fcy = cy as f32;
    ground_shadow(pb, fcx + 1.2, fcy + 3.6, 6.8, 3.0, 0.42);

    let wicker_hi = palette::mix((142, 118, 86), pal.bg, 0.14);
    let wicker_mid = palette::mix((108, 88, 64), pal.bg, 0.18);
    let wicker_lo = palette::mix((68, 54, 42), pal.bg, 0.24);
    let rim_hi = palette::mix((152, 130, 98), pal.bg, 0.12);

    // 1. Tapered woven basket body (`ly in -1..=5` relative to `cy`).
    for ly in -1..=5 {
        let py = cy + ly;
        let taper = ((ly + 1) as f32 / 6.0).clamp(0.0, 1.0);
        let half_w = (6.2 - 1.3 * taper).round() as i32;
        for lx in -half_w..=half_w {
            let px = cx + lx;
            let on_edge = lx.abs() == half_w || ly == 5;
            if on_edge {
                pb.plot(px, py, outline);
                continue;
            }
            let shade = 1.0 - ((lx as f32 / half_w.max(1) as f32) * 0.26);
            let weave = ((lx + ly) & 1) == 0;
            let base = if ly == -1 {
                rim_hi
            } else if ly == 0 {
                wicker_lo
            } else if weave {
                if lx < 0 { wicker_hi } else { wicker_mid }
            } else {
                wicker_lo
            };
            pb.plot(px, py, palette::scale(base, shade.clamp(0.74, 1.12)));
        }
    }
    // Side carry handles at the basket rim.
    pb.plot(cx - 7, cy - 1, rim_hi);
    pb.plot(cx - 7, cy, outline);
    pb.plot(cx + 7, cy - 1, wicker_mid);
    pb.plot(cx + 7, cy, outline);

    // 2. Overflowing mound of mixed clothes heaped above the rim.
    draw_cloth_lump(pb, fcx - flip as f32 * 2.0, fcy - 2.4, 3.6, 2.3, c0, outline, phase, flip as f32);
    draw_cloth_lump(pb, fcx + flip as f32 * 2.2, fcy - 2.6, 3.5, 2.2, c1, outline, phase + 1.9, -flip as f32);
    draw_cloth_lump(pb, fcx, fcy - 4.0, 3.4, 2.1, c2, outline, phase + 3.7, flip as f32);

    // 3. One trailing shirt/towel corner spilling down over the front rim.
    let spill_x = cx + flip * 2;
    pb.plot(spill_x, cy - 1, c1.2);
    pb.plot(spill_x + flip, cy - 1, c1.1);
    pb.plot(spill_x, cy, c1.1);
    pb.plot(spill_x + flip, cy, c1.0);
    pb.plot(spill_x, cy + 1, c1.0);
    pb.plot(spill_x + flip, cy + 1, outline);
    pb.plot(spill_x, cy + 2, outline);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;

    #[test]
    fn all_pile_kinds_paint_bounded_low_contrast_silhouettes() {
        let pal = Palette::for_theme(Theme::Laundry);
        let bg = pal.bg;
        for kind in [
            ClothesPileKind::SmallHeap,
            ClothesPileKind::MediumHeap,
            ClothesPileKind::LargeHeap,
            ClothesPileKind::Basket,
        ] {
            let pile = ClothesPile {
                pos: Vec2::new(32.0, 32.0),
                kind,
                seed: 0x1234_5678,
            };
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear(bg);
            draw_clothes_pile(&mut pb, &pile, Vec2::ZERO, &pal);

            let mut painted = 0usize;
            let mut max_chan = 0u8;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    if c != bg {
                        painted += 1;
                        max_chan = max_chan.max(c.0.max(c.1).max(c.2));
                        let dx = (x as i32 - 32).abs();
                        let dy = (y as i32 - 32).abs();
                        assert!(
                            dx <= MAX_PILE_RADIUS_PX as i32 && dy <= MAX_PILE_RADIUS_PX as i32,
                            "{kind:?}: painted outside bounds at ({x},{y})"
                        );
                    }
                }
            }
            assert!(painted >= 25, "{kind:?}: drew too few pixels ({painted})");
            assert!(
                max_chan <= 165,
                "{kind:?}: contrast too bright ({max_chan}) for background floor decoration"
            );

            // Edge clipping safety.
            draw_clothes_pile(&mut pb, &pile, Vec2::new(30.0, 30.0), &pal);
            draw_clothes_pile(&mut pb, &pile, Vec2::new(-30.0, -30.0), &pal);
        }
    }

    #[test]
    fn clothes_piles_are_static_and_deterministic() {
        let pal = Palette::for_theme(Theme::Laundry);
        let pile = ClothesPile {
            pos: Vec2::new(24.0, 24.0),
            kind: ClothesPileKind::Basket,
            seed: 99,
        };
        let mut a = PixelBuffer::new(48, 48);
        let mut b = PixelBuffer::new(48, 48);
        a.clear(pal.bg);
        b.clear(pal.bg);
        draw_clothes_pile(&mut a, &pile, Vec2::ZERO, &pal);
        draw_clothes_pile(&mut b, &pile, Vec2::ZERO, &pal);
        assert_eq!(a.pixels(), b.pixels());
    }
}
