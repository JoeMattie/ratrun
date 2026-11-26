//! Decorative aquatic plants and seabed floor patches on The Outfall: kelp
//! fronds, seagrass tufts, corner anemones / sea fans, and matte textured
//! gravel, shell-grit, pebble, silt and shingle beds. Pure drawing
//! (render-only).
//!
//! Seabed patches are drawn at floor level where puddles were drawn (after
//! floor stains, before the boss ground layer) with a matte, rough material
//! pen ([`seabed_pen`]) so dynamic lighting picks up stone bump relief without
//! ever treating them as standing water.
//!
//! Plants are drawn in `World::draw_scene` after the floor, patches, Outfall
//! hazards, walls and props, and **before** pools, loot, enemies, bosses and
//! the rat, so every body passes cleanly over them and the lighting pass shades
//! them with the rest of the scene.
//!
//! Plant animation reads **only** `World::elapsed` (continuous sine sway, never
//! stepped), so frozen frames are bit-identical. Seabed patches read no clock
//! and are strictly deterministic from `(patch.seed, world position)`.

use crate::game::collision::TileMap;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{PlantKind, SeabedKind, SeabedPatch, SeabedPlant};

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

// ============================================================================
// Seabed floor patches (Gravel, ShellGrit, Pebbles, Silt, Shingle)
// ============================================================================

/// Matte grey-brown mineral chip palette for [`SeabedKind::Gravel`].
const GRAVEL_CHIPS: [Rgb; 6] = [
    (52, 53, 50),   // dark flint
    (68, 70, 68),   // cool grey chip
    (84, 85, 81),   // pale grey chip
    (96, 84, 68),   // warm brown chip
    (76, 68, 56),   // umber chip
    (112, 103, 86), // pale quartz grit
];

/// Chalky broken shell fleck palette for [`SeabedKind::ShellGrit`].
const SHELL_FLECKS: [Rgb; 4] = [
    (142, 136, 120), // chalk ivory
    (130, 124, 108), // pale bone
    (122, 108, 92),  // warm buff shell
    (120, 118, 110), // pearl grey
];

/// Four tiny curved shell-rim arc templates: `(dx, dy, kind)` where `2` is the
/// bright outer rim, `1` is the mid-tone rim tip, and `0` is the shadowed
/// inner hollow.
const SHELL_ARCS: [[(i32, i32, u8); 6]; 4] = [
    // 0: smile crescent (cupped up)
    [(0, 0, 1), (1, 1, 2), (2, 1, 2), (3, 0, 1), (1, 0, 0), (2, 0, 0)],
    // 1: arch crescent (cupped down)
    [(0, 1, 1), (1, 0, 2), (2, 0, 2), (3, 1, 1), (1, 1, 0), (2, 1, 0)],
    // 2: C-left crescent
    [(1, 0, 2), (0, 1, 2), (0, 2, 1), (1, 3, 1), (1, 1, 0), (1, 2, 0)],
    // 3: C-right crescent
    [(0, 0, 2), (1, 1, 2), (1, 2, 1), (0, 3, 1), (0, 1, 0), (0, 2, 0)],
];

/// Rounded river/sea stone palettes `(lit_top, body, dark_under)` for
/// [`SeabedKind::Pebbles`].
const PEBBLE_STONES: [(Rgb, Rgb, Rgb); 5] = [
    ((118, 114, 103), (86, 84, 75), (49, 48, 43)), // river grey
    ((105, 112, 112), (73, 80, 82), (43, 48, 50)), // cool slate
    ((124, 108, 86), (93, 79, 61), (55, 47, 37)),  // warm ochre stone
    ((102, 104, 90), (71, 74, 63), (42, 44, 38)),  // olive basalt
    ((130, 123, 107), (99, 94, 81), (60, 56, 48)), // pale sandstone
];

/// Flat coastal slab palettes `(top_bevel, flat_face, overlap_seam)` for
/// [`SeabedKind::Shingle`].
const SHINGLE_STONES: [(Rgb, Rgb, Rgb); 5] = [
    ((92, 96, 96), (76, 80, 80), (44, 47, 48)),    // slate grey
    ((114, 113, 104), (99, 98, 90), (60, 60, 54)), // flat limestone
    ((104, 97, 86), (88, 82, 72), (52, 48, 42)),   // warm taupe
    ((74, 79, 81), (60, 65, 67), (36, 40, 42)),    // dark blue-shale
    ((110, 103, 88), (94, 87, 74), (56, 52, 44)),  // dusty sea-tan
];

/// The material pen for lighting (matte, rough), so the lights do not treat it
/// as water.
///
/// Stony patches return `MaterialKind::Stone` (height 0: zero gloss, zero env
/// reflection, strong bump relief so directional lights catch individual stones);
/// smooth fine silt returns `MaterialKind::Cloth` (height 0: ultra-matte, zero
/// reflection, gentler fine relief).
pub fn seabed_pen(kind: SeabedKind) -> u8 {
    match kind {
        SeabedKind::Silt => crate::render::lighting::cloth_pen(),
        SeabedKind::Gravel
        | SeabedKind::ShellGrit
        | SeabedKind::Pebbles
        | SeabedKind::Shingle => crate::render::lighting::stone_pen(),
    }
}

/// Draw one patch into the world buffer, in screen space via `cam`. Floor-level
/// decoration, drawn where puddles were drawn (after floor stains, before the
/// boss ground layer). Deterministic from `patch.seed` and world position; no
/// clock, zero allocation, and clipped to open floor tiles.
pub fn draw_seabed_patch(pb: &mut PixelBuffer, patch: &SeabedPatch, tiles: &TileMap, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 || patch.rx <= 0.0 || patch.ry <= 0.0 {
        return;
    }
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let min_wx = (patch.center.x - patch.rx).floor() as i32;
    let max_wx = (patch.center.x + patch.rx).ceil() as i32;
    let min_wy = (patch.center.y - patch.ry).floor() as i32;
    let max_wy = (patch.center.y + patch.ry).ceil() as i32;

    let x0 = (min_wx - cam_x).max(0);
    let x1 = (max_wx - cam_x).min(pb.w as i32 - 1);
    let y0 = (min_wy - cam_y).max(0);
    let y1 = (max_wy - cam_y).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let prev_pen = pb.set_pen(seabed_pen(patch.kind));
    for sy in y0..=y1 {
        let wy = sy + cam_y;
        let wy_f = wy as f32 + 0.5;
        for sx in x0..=x1 {
            let wx = sx + cam_x;
            let wx_f = wx as f32 + 0.5;
            let here = Vec2::new(wx_f, wy_f);
            if !tiles.get_at(here).is_open() {
                continue;
            }
            let depth = patch.depth(here);
            if depth <= 0.0 {
                continue;
            }
            let bg = pb.pixel_at(sx as usize, sy as usize);
            let col = match patch.kind {
                SeabedKind::Gravel => shade_gravel(bg, wx, wy, depth, patch.seed),
                SeabedKind::ShellGrit => shade_shell_grit(bg, wx, wy, depth, patch.seed),
                SeabedKind::Pebbles => shade_pebbles(bg, wx, wy, depth, patch.seed),
                SeabedKind::Silt => shade_silt(bg, wx, wy, depth, patch.seed),
                SeabedKind::Shingle => shade_shingle(bg, wx, wy, depth, patch.seed),
            };
            pb.plot(sx, sy, col);
        }
    }
    pb.set_pen(prev_pen);
}

#[inline]
fn hash3(x: i32, y: i32, z: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ z.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}

/// Bilinearly smoothed world-space value noise in `0..=255`.
#[inline]
fn smooth_noise_u8(wx: i32, wy: i32, cell: i32, seed: u32) -> i32 {
    let gx = wx.div_euclid(cell);
    let gy = wy.div_euclid(cell);
    let fx = wx - gx * cell;
    let fy = wy - gy * cell;
    let tx = (fx * 256) / cell;
    let ty = (fy * 256) / cell;
    let sx = (tx * tx * (768 - 2 * tx)) >> 16;
    let sy = (ty * ty * (768 - 2 * ty)) >> 16;

    let c00 = (hash3(gx, gy, seed) & 255) as i32;
    let c10 = (hash3(gx + 1, gy, seed) & 255) as i32;
    let c01 = (hash3(gx, gy + 1, seed) & 255) as i32;
    let c11 = (hash3(gx + 1, gy + 1, seed) & 255) as i32;
    let top = c00 + (((c10 - c00) * sx) >> 8);
    let bot = c01 + (((c11 - c01) * sx) >> 8);
    top + (((bot - top) * sy) >> 8)
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn shift_rgb((r, g, b): Rgb, delta: i32) -> Rgb {
    (
        (i32::from(r) + delta).clamp(0, 255) as u8,
        (i32::from(g) + delta).clamp(0, 255) as u8,
        (i32::from(b) + delta).clamp(0, 255) as u8,
    )
}

/// Mixed grey-brown mineral chips (1–2 px) tapering smoothly into surrounding sand.
fn shade_gravel(bg: Rgb, wx: i32, wy: i32, depth: f32, seed: u32) -> Rgb {
    let s = smoothstep(depth);
    let n = smooth_noise_u8(wx, wy, 10, seed ^ 0x11) as f32 / 255.0;
    let under = mix((58, 56, 49), (74, 70, 60), n);
    let mut col = mix(bg, under, 0.44 * s);

    // Layer 1: 2×2 world cells of 1–2 px angular chips.
    let cx = wx.div_euclid(2);
    let cy = wy.div_euclid(2);
    let lx = wx - cx * 2;
    let ly = wy - cy * 2;
    let h = hash3(cx, cy, seed ^ 0x6a09_e667);
    let chance = (depth * 220.0) as u32;
    if (h & 255) < chance {
        let chip = GRAVEL_CHIPS[((h >> 8) as usize) % GRAVEL_CHIPS.len()];
        let (hit, delta) = match (h >> 12) & 3 {
            0 => {
                let px_i = ((h >> 14) & 1) as i32;
                let py_i = ((h >> 15) & 1) as i32;
                (lx == px_i && ly == py_i, 0)
            }
            1 => {
                let row = ((h >> 14) & 1) as i32;
                (ly == row, if lx == 0 { 6 } else { -6 })
            }
            2 => {
                let col_i = ((h >> 14) & 1) as i32;
                (lx == col_i, if ly == 0 { 6 } else { -7 })
            }
            _ => {
                let second_y = ((h >> 14) & 1) as i32;
                let top_left = lx == 0 && ly == 0;
                (top_left || (lx == 1 && ly == second_y), if top_left { 5 } else { -6 })
            }
        };
        if hit {
            let alpha = (0.45 + 0.50 * s).clamp(0.0, 0.95);
            col = mix(col, shift_rgb(chip, delta), alpha);
        }
    }

    // Layer 2: sparse 1 px dark shadow / pale quartz grit on a 3×3 world grid.
    let gx = wx.div_euclid(3);
    let gy = wy.div_euclid(3);
    let h2 = hash3(gx, gy, seed ^ 0xbb67_ae85);
    if (h2 & 255) < (depth * 140.0) as u32 {
        let ox = ((h2 >> 8) % 3) as i32;
        let oy = ((h2 >> 10) % 3) as i32;
        if (wx - gx * 3) == ox && (wy - gy * 3) == oy {
            let speck = if (h2 & 0x1000) != 0 {
                (114, 106, 89)
            } else {
                (46, 46, 44)
            };
            col = mix(col, speck, 0.58 * s);
        }
    }
    col
}

/// Pale broken chalky shell flecks with scattered tiny curved shell arcs.
fn shade_shell_grit(bg: Rgb, wx: i32, wy: i32, depth: f32, seed: u32) -> Rgb {
    let s = smoothstep(depth);
    let n = smooth_noise_u8(wx, wy, 12, seed ^ 0x22) as f32 / 255.0;
    let under = mix((94, 88, 72), (112, 106, 90), n);
    let mut col = mix(bg, under, 0.44 * s);

    // Fine broken shell flecks on a jittered 2×2 world grid.
    let cx = wx.div_euclid(2);
    let cy = wy.div_euclid(2);
    let lx = wx - cx * 2;
    let ly = wy - cy * 2;
    let h = hash3(cx, cy, seed ^ 0x3c6e_f372);
    if (h & 255) < (depth * 112.0) as u32 {
        let ox = ((h >> 8) & 1) as i32;
        let oy = ((h >> 9) & 1) as i32;
        if lx == ox && ly == oy {
            let fleck = SHELL_FLECKS[((h >> 10) as usize) % SHELL_FLECKS.len()];
            col = mix(col, fleck, (0.45 + 0.48 * s).clamp(0.0, 0.92));
        } else if (h & 0x4000) != 0 && lx == (ox ^ 1) && ly == oy {
            col = mix(col, (64, 60, 49), 0.34 * s);
        }
    }

    // Scattered curved shell bits on a 7×6 world grid (3×3 neighbour search prevents clipping).
    if depth > 0.16 {
        let bx = wx.div_euclid(7);
        let by = wy.div_euclid(6);
        let arc_limit = (((depth - 0.10) * 1.15).clamp(0.0, 1.0) * 170.0) as u32;
        for ny in (by - 1)..=(by + 1) {
            for nx in (bx - 1)..=(bx + 1) {
                let hb = hash3(nx, ny, seed ^ 0xa54f_f53a);
                if (hb & 255) >= arc_limit {
                    continue;
                }
                let ax = nx * 7 + 1 + ((hb >> 8) % 3) as i32;
                let ay = ny * 6 + 1 + ((hb >> 11) % 2) as i32;
                let dx = wx - ax;
                let dy = wy - ay;
                if !(0..=3).contains(&dx) || !(0..=3).contains(&dy) {
                    continue;
                }
                let arc = &SHELL_ARCS[((hb >> 14) & 3) as usize];
                let warm = (hb & 0x1_0000) != 0;
                let rim_hi: Rgb = if warm { (152, 144, 126) } else { (148, 144, 132) };
                let rim_mid: Rgb = if warm { (134, 125, 108) } else { (130, 127, 116) };
                for &(px, py, kind) in arc {
                    if dx == px && dy == py {
                        return match kind {
                            2 => mix(col, rim_hi, 0.92 * s),
                            1 => mix(col, rim_mid, 0.82 * s),
                            _ => mix(col, (56, 52, 43), 0.56 * s),
                        };
                    }
                }
            }
        }
    }
    col
}

/// Rounded 2–4 px stones with matte lit top edges, dark undersides and contact
/// shadow creases in a compacted dark sand matrix.
fn shade_pebbles(bg: Rgb, wx: i32, wy: i32, depth: f32, seed: u32) -> Rgb {
    const CELL_W: i32 = 4;
    const CELL_H: i32 = 4;

    let s = smoothstep(depth);
    let n = smooth_noise_u8(wx, wy, 10, seed ^ 0x33) as f32 / 255.0;
    let matrix = mix((48, 47, 40), (64, 61, 52), n);
    let mut col = mix(bg, matrix, 0.56 * s);

    let bx = wx.div_euclid(CELL_W);
    let by = wy.div_euclid(CELL_H);
    let px_f = wx as f32 + 0.5;
    let py_f = wy as f32 + 0.5;

    let occ_limit = ((depth * 1.18).clamp(0.0, 1.0) * 215.0) as u32;
    let edge_shrink = 0.78 + 0.22 * s;
    let mut best_r2 = f32::INFINITY;
    let mut best_stone: Option<(f32, f32, usize)> = None;
    let mut in_shadow = false;

    for cy in (by - 1)..=(by + 1) {
        for cx in (bx - 1)..=(bx + 1) {
            let h = hash3(cx, cy, seed ^ 0x510e_527f);
            if (h & 255) >= occ_limit {
                continue;
            }
            let jx = ((h >> 8) & 15) as f32 / 15.0;
            let jy = ((h >> 12) & 15) as f32 / 15.0;
            let st_x = (cx * CELL_W) as f32 + 0.7 + jx * 2.6;
            let st_y = (cy * CELL_H) as f32 + 0.7 + jy * 2.6;

            let rx = (1.15 + ((h >> 16) & 15) as f32 * (0.85 / 15.0)) * edge_shrink;
            let ry = (1.00 + ((h >> 20) & 15) as f32 * (0.55 / 15.0)) * edge_shrink;
            let dx = px_f - st_x;
            let dy = py_f - st_y;
            let nx = dx / rx;
            let ny = dy / ry;
            let r2 = nx * nx + ny * ny;
            if r2 <= 1.0 && r2 < best_r2 {
                best_r2 = r2;
                best_stone = Some((nx, ny, ((h >> 24) as usize) % PEBBLE_STONES.len()));
            } else if dy > 0.0 {
                let snx = dx / (rx * 1.04);
                let sny = (dy - 0.65) / (ry * 0.96);
                if snx * snx + sny * sny <= 1.0 {
                    in_shadow = true;
                }
            }
        }
    }

    if let Some((nx, ny, pal_i)) = best_stone {
        let (lit_top, body, dark_under) = PEBBLE_STONES[pal_i];
        let lit = -ny * 0.82 - nx * 0.24;
        let st_col = if lit > 0.26 {
            lit_top
        } else if lit < -0.24 {
            dark_under
        } else if lit > 0.08 {
            mix(body, lit_top, 0.30)
        } else if lit < -0.08 {
            mix(body, dark_under, 0.35)
        } else {
            body
        };
        col = mix(col, st_col, (0.55 + 0.45 * s).clamp(0.0, 0.96));
    } else if in_shadow {
        col = scale(col, 1.0 - 0.26 * s);
    }
    col
}

/// Smooth darker, cooler fine silt field, almost flat, feathered softly into sand.
fn shade_silt(bg: Rgb, wx: i32, wy: i32, depth: f32, seed: u32) -> Rgb {
    let n_big = smooth_noise_u8(wx, wy, 22, seed ^ 0x9b05_688c);
    let n_mid = smooth_noise_u8(wx + 7, wy - 5, 11, seed ^ 0x1f83_d9ab);
    let edge_mod = ((n_mid - 128) as f32 / 128.0) * 0.14 * (1.0 - depth);
    let s = smoothstep((depth + edge_mod).clamp(0.0, 1.0));
    let tone = (n_big * 3 + n_mid) as f32 / (4.0 * 255.0);
    let silt_base = mix((40, 49, 51), (56, 65, 64), tone);

    let hf = hash3(wx, wy, seed ^ 0x5be0_cd19);
    let micro = if (hf & 7) < 4 {
        (hf & 3) as i32 - 1
    } else {
        0
    };
    mix(bg, shift_rgb(silt_base, micro), 0.86 * s)
}

/// Flat overlapping coastal shingle stones laid back-to-front with crisp dark
/// overlap seams.
fn shade_shingle(bg: Rgb, wx: i32, wy: i32, depth: f32, seed: u32) -> Rgb {
    const CELL_W: i32 = 4;
    const CELL_H: i32 = 3;

    let s = smoothstep(depth);
    let n = smooth_noise_u8(wx, wy, 10, seed ^ 0x44) as f32 / 255.0;
    let matrix = mix((46, 48, 45), (60, 61, 56), n);
    let mut col = mix(bg, matrix, 0.50 * s);

    let by = wy.div_euclid(CELL_H);
    let px_f = wx as f32 + 0.5;
    let py_f = wy as f32 + 0.5;

    let occ_limit = ((depth * 1.24).clamp(0.0, 1.0) * 230.0) as u32;
    let mut best_layer = f32::NEG_INFINITY;
    let mut hit_col: Option<Rgb> = None;

    for cy in (by - 1)..=(by + 1) {
        let row_shift = if (cy & 1) != 0 { 2.0 } else { 0.0 };
        let bx = ((px_f - row_shift) / CELL_W as f32).floor() as i32;
        for cx in (bx - 1)..=(bx + 1) {
            let h = hash3(cx, cy, seed ^ 0x7137_4491);
            if (h & 255) >= occ_limit {
                continue;
            }
            let jx = (((h >> 8) & 15) as f32 / 15.0 - 0.5) * 2.4;
            let jy = (((h >> 12) & 15) as f32 / 15.0 - 0.5) * 1.9;
            let st_x = (cx * CELL_W) as f32 + row_shift + 2.0 + jx;
            let st_y = (cy * CELL_H) as f32 + 1.5 + jy;

            let rx = 1.70 + ((h >> 16) & 15) as f32 * (0.80 / 15.0);
            let ry = 1.00 + ((h >> 20) & 15) as f32 * (0.45 / 15.0);
            let tilt = (((h >> 24) & 7) as f32 / 7.0 - 0.5) * 0.44;

            let dx = px_f - st_x;
            let dy = (py_f - st_y) + dx * tilt;
            let nx = dx / rx;
            let ny = dy / ry;
            let ax = nx.abs();
            let ay = ny.abs();
            let ax2 = ax * ax;
            let ay2 = ay * ay;
            let r_flat = ax2 + ay2 + 0.20 * (ax2 * ax2 + ay2 * ay2);
            if r_flat <= 1.06 {
                let layer = st_y + ry * 0.3 + ((h >> 4) & 7) as f32 * 0.02;
                if layer > best_layer {
                    best_layer = layer;
                    let (top_bevel, flat_face, overlap_seam) =
                        SHINGLE_STONES[((h >> 27) as usize) % SHINGLE_STONES.len()];
                    let st_col = if ny > 0.34 || (r_flat > 0.72 && ny > 0.02) {
                        overlap_seam
                    } else if ny < -0.50 && r_flat > 0.44 {
                        top_bevel
                    } else {
                        let grain = (hash3(wx, wy, seed ^ 0x99) & 3) as i32 - 1;
                        shift_rgb(flat_face, grain)
                    };
                    hit_col = Some(st_col);
                }
            }
        }
    }

    if let Some(st_col) = hit_col {
        col = mix(col, st_col, (0.55 + 0.45 * s).clamp(0.0, 0.96));
    }
    col
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

    const ALL_SEABED_KINDS: [SeabedKind; 5] = [
        SeabedKind::Gravel,
        SeabedKind::ShellGrit,
        SeabedKind::Pebbles,
        SeabedKind::Silt,
        SeabedKind::Shingle,
    ];

    #[test]
    fn draw_seabed_patch_stays_inside_bounding_box_and_skips_walls() {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;

        const SAND: Rgb = (84, 77, 60);
        let mut tiles = TileMap::new(10, 8, Theme::Outfall, 1);
        // Paint one solid wall cell overlapping the patch's right side.
        tiles.set(5, 3, TileKind::Wall(WallKind::Solid));

        for (idx, kind) in ALL_SEABED_KINDS.into_iter().enumerate() {
            let patch = SeabedPatch {
                center: Vec2::new(72.0, 54.0),
                rx: 26.0,
                ry: 18.0,
                kind,
                seed: 100 + idx as u32,
            };
            let mut pb = PixelBuffer::new(160, 120);
            pb.set_materials(true);
            pb.set_pen(0);
            pb.clear(SAND);
            draw_seabed_patch(&mut pb, &patch, &tiles, Vec2::ZERO);

            let min_x = (patch.center.x - patch.rx).floor() as usize;
            let max_x = (patch.center.x + patch.rx).ceil() as usize;
            let min_y = (patch.center.y - patch.ry).floor() as usize;
            let max_y = (patch.center.y + patch.ry).ceil() as usize;

            let expected_pen = seabed_pen(kind);
            let mut changed_inside = 0usize;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let changed = pb.pixel_at(x, y) != SAND;
                    let mat = pb.mat()[y * pb.w + x];
                    let in_bbox = (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y);
                    let wp = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if !in_bbox {
                        assert!(
                            !changed && mat == 0,
                            "{kind:?}: pixel ({x},{y}) outside bbox was modified"
                        );
                    }
                    if !tiles.get_at(wp).is_open() {
                        assert!(
                            !changed && mat == 0,
                            "{kind:?}: pixel ({x},{y}) on wall tile was modified"
                        );
                    } else if patch.depth(wp) > 0.0 {
                        assert_eq!(
                            mat, expected_pen,
                            "{kind:?}: inside pixel ({x},{y}) must wear seabed_pen"
                        );
                        if changed {
                            changed_inside += 1;
                        }
                    } else if !patch.contains(wp) {
                        assert!(
                            !changed && mat == 0,
                            "{kind:?}: pixel ({x},{y}) outside blob was modified"
                        );
                    }
                }
            }
            assert!(
                changed_inside > 120,
                "{kind:?}: expected visible patch interior, got {changed_inside} changed pixels"
            );
            assert_eq!(pb.pen(), 0, "draw_seabed_patch must restore caller pen");
        }
    }

    #[test]
    fn draw_seabed_patch_is_deterministic_per_seed_and_camera_stable() {
        const SAND: Rgb = (84, 77, 60);
        let tiles = TileMap::new(12, 10, Theme::Outfall, 3);
        for kind in ALL_SEABED_KINDS {
            let patch = SeabedPatch {
                center: Vec2::new(90.0, 70.0),
                rx: 24.0,
                ry: 16.0,
                kind,
                seed: 4242,
            };
            let mut a = PixelBuffer::new(140, 110);
            let mut b = PixelBuffer::new(140, 110);
            a.clear(SAND);
            b.clear(SAND);
            draw_seabed_patch(&mut a, &patch, &tiles, Vec2::ZERO);
            draw_seabed_patch(&mut b, &patch, &tiles, Vec2::ZERO);
            assert_eq!(a.pixels(), b.pixels(), "{kind:?}: same seed must draw identically");

            // Camera shift by (13, 9) must match the exact world crop pixel-for-pixel.
            let mut shifted = PixelBuffer::new(100, 80);
            shifted.clear(SAND);
            draw_seabed_patch(&mut shifted, &patch, &tiles, Vec2::new(13.0, 9.0));
            for sy in 0..shifted.h {
                for sx in 0..shifted.w {
                    assert_eq!(
                        shifted.pixel_at(sx, sy),
                        a.pixel_at(sx + 13, sy + 9),
                        "{kind:?}: camera-shifted pixel ({sx},{sy}) drifted"
                    );
                }
            }
        }
    }

    #[test]
    fn seabed_pen_is_matte_non_water_and_flat_floor_height() {
        let water = crate::render::lighting::pool_pen();
        for kind in ALL_SEABED_KINDS {
            let p = seabed_pen(kind);
            assert_ne!(p, 0, "{kind:?}: pen should tag an explicit matte material");
            assert_ne!(p, water, "{kind:?}: pen must never be standing water");
            assert_eq!(p & 0x10, 0, "{kind:?}: pen must not set CREATURE bit");
            assert_eq!(p >> 5, 0, "{kind:?}: pen must have height class 0 (floor)");
        }
    }
}
