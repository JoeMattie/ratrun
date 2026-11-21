//! Art for the solid obstacles ("props"): pipes, crates, tanks and the rest.
//! Pure drawing: each function paints one prop into the pixel buffer inside
//! its collision footprint `(x, y, w, h)` in screen pixels, with consistent
//! top-left lighting, 3/4 volume, surface detail and soft contact shadows.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

use super::level::{Palette, Prop, PropKind};

pub fn draw_prop(pb: &mut PixelBuffer, prop: &Prop, cam: Vec2, pal: &Palette) {
    let x = (prop.rect.x - cam.x).round() as i32;
    let y = (prop.rect.y - cam.y).round() as i32;
    let w = prop.rect.w.round() as i32;
    let h = prop.rect.h.round() as i32;
    if w <= 0 || h <= 0 {
        return;
    }
    let seed = hash2(prop.rect.x.round() as i32, prop.rect.y.round() as i32);
    match prop.kind {
        PropKind::PipeV => pipe_v(pb, x, y, w, h, seed, pal),
        PropKind::PipeH => pipe_h(pb, x, y, w, h, seed, pal),
        PropKind::Barrier => barrier(pb, x, y, w, h, seed, pal),
        PropKind::Valve => valve(pb, x, y, w, h, seed, pal),
        PropKind::Grate => grate(pb, x, y, w, h, seed, pal),
        PropKind::Table => table(pb, x, y, w, h, seed, pal),
        PropKind::Crate => crate_box(pb, x, y, w, h, seed, pal),
        PropKind::Barrel => barrel(pb, x, y, w, h, seed, pal),
        PropKind::Counter => counter(pb, x, y, w, h, seed, pal),
        PropKind::Console => console(pb, x, y, w, h, seed, pal),
        PropKind::Tank => tank(pb, x, y, w, h, seed, pal),
        PropKind::Washer => washer(pb, x, y, w, h, seed, pal),
        PropKind::TowelRoll => towel_roll(pb, x, y, w, h, seed, pal),
        PropKind::Clothespin => clothespin(pb, x, y, w, h, seed, pal),
        PropKind::PressIsland => press_island(pb, x, y, w, h, seed, pal),
        PropKind::PaperReel => paper_reel(pb, x, y, w, h, seed, pal),
        PropKind::TypeBlock => type_block(pb, x, y, w, h, seed, pal),
        // The flatbed's cargo is its own art module (planks, tarps, straps).
        PropKind::Pallet
        | PropKind::TarpLump
        | PropKind::Tyre
        | PropKind::Toolbox
        | PropKind::PipeStack
        | PropKind::ChainCoil => {
            crate::game::art_trailer::draw_cargo(pb, prop.kind, x, y, w, h, seed, pal)
        }
    }
}

// ---- Shared drawing primitives ---------------------------------------------

#[inline]
fn hash2(a: i32, b: i32) -> u32 {
    let mut h = (a as u32).wrapping_mul(0x9E37_79B1) ^ (b as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

#[inline]
fn hash_f32(a: i32, b: i32) -> f32 {
    (hash2(a, b) & 0xFFFF) as f32 / 65535.0
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
    pb.plot(x, y, palette::scale(cur, factor));
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let cur = pb.pixel_at(ux, uy);
    pb.plot(x, y, palette::mix(cur, c, alpha));
}

/// Soft grounded contact shadow cast toward bottom-right (top-left light).
/// Spills 3px below and 3px to the right of `(x, y, w, h)`.
fn shadow_rect(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    for dy in 0..3 {
        let sy = y + h + dy;
        if sy < 0 || sy >= bh {
            continue;
        }
        let factor = match dy {
            0 => 0.40,
            1 => 0.62,
            _ => 0.80,
        };
        let x0 = (x + 1 + dy).max(0);
        let x1 = (x + w + 2 - (dy / 2)).min(bw);
        for sx in x0..x1 {
            darken_px(pb, sx, sy, factor);
        }
    }
    for dx in 0..3 {
        let sx = x + w + dx;
        if sx < 0 || sx >= bw {
            continue;
        }
        let factor = match dx {
            0 => 0.44,
            1 => 0.66,
            _ => 0.82,
        };
        let y0 = (y + 2 + dx).max(0);
        let y1 = (y + h).min(bh);
        for sy in y0..y1 {
            darken_px(pb, sx, sy, factor);
        }
    }
}

/// Soft elliptical contact shadow over the floor or surface.
fn shadow_ellipse(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, strength: f32) {
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(bw - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(bh - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    for sy in y0..=y1 {
        let ny = (sy as f32 + 0.5 - cy) / ry.max(0.5);
        for sx in x0..=x1 {
            let nx = (sx as f32 + 0.5 - cx) / rx.max(0.5);
            let d2 = nx * nx + ny * ny;
            if d2 <= 1.0 {
                let falloff = (1.0 - d2).sqrt();
                darken_px(pb, sx, sy, 1.0 - strength * falloff);
            }
        }
    }
}

/// Crisp 1px beveled rectangle border (top/left lit, bottom/right shaded).
fn bevel_frame(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, hi: Rgb, lo: Rgb) {
    if w <= 1 || h <= 1 {
        return;
    }
    pb.rect_fill(x, y, w, 1, hi);
    pb.rect_fill(x, y, 1, h, hi);
    pb.rect_fill(x, y + h - 1, w, 1, lo);
    pb.rect_fill(x + w - 1, y, 1, h, lo);
}

#[inline]
fn bolt(pb: &mut PixelBuffer, x: i32, y: i32, hi: Rgb, lo: Rgb) {
    pb.plot(x, y, hi);
    pb.plot(x + 1, y + 1, lo);
}

/// Blend a tiny hint of theme ambient into dark outlines so props feel grounded.
#[inline]
fn ambient_shade(c: Rgb, pal: &Palette, k: f32) -> Rgb {
    palette::mix(c, pal.bg, k)
}

// ---- 1. PipeV (14 x 72) ----------------------------------------------------

fn pipe_v(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    // Contact shadow follows the shaft + protruding flanges
    shadow_rect(pb, x + 1, y, (w - 2).max(1), h);

    let metal = (96, 122, 118);
    let spec = (192, 214, 208);
    let rust = (118, 74, 48);
    let verdigris = (66, 124, 108);
    let outline = ambient_shade((24, 34, 32), pal, 0.25);

    let flanges = [0, h / 3 - 2, (h * 2) / 3 - 1, h - 5];

    // 1. Recessed cylindrical pipe shaft (inset 1px left/right so flanges visibly protrude!)
    let sx0 = if w >= 8 { x + 1 } else { x };
    let sw = if w >= 8 { w - 2 } else { w };
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let ya = y.max(0);
    let yb = (y + h).min(bh);
    let xa = sx0.max(0);
    let xb = (sx0 + sw).min(bw);

    for py in ya..yb {
        let ly = py - y;
        // Distance to nearest flange for cohesive corrosion/grime clustering
        let mut near_flange = 99;
        for &fy in &flanges {
            let d = (ly - (fy + 2)).abs();
            if d < near_flange {
                near_flange = d;
            }
        }
        for px in xa..xb {
            let lx = px - sx0;
            if lx == 0 || lx == sw - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let t = lx as f32 / (sw.max(2) - 1) as f32;
            let diff = (1.0 - ((t - 0.27) * 1.65).abs()).clamp(0.0, 1.0);
            let bounce = if lx == sw - 2 { 0.14 } else { 0.0 };
            // Subtle longitudinal brushed-pipe seam variation
            let streak = (hash_f32(lx, (seed as i32 & 15) + 7) - 0.5) * 0.06;
            let mut c = palette::scale(metal, 0.40 + 0.90 * diff + bounce + streak);

            // Crisp specular line along lit top-left curve
            if (t - 0.24).abs() < 0.085 {
                c = palette::mix(c, spec, 0.52);
            }
            // Cohesive patina/rust staining clustered near couplings
            if near_flange <= 5 {
                let stain = hash_f32(lx / 2 + (seed as i32 & 15), ly / 3 + 3);
                if stain > 0.62 {
                    let k = (1.0 - near_flange as f32 / 6.0) * 0.36;
                    let pat = if (seed ^ (lx as u32)) & 1 == 0 { rust } else { verdigris };
                    c = palette::mix(c, pat, k);
                }
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Protruding bolted flange collars (full width `w`)
    for (idx, &fy) in flanges.iter().enumerate() {
        let fh = if idx == 0 || idx == 3 { 5 } else { 4 };
        let ftop = y + fy;
        // Extra shadow cast by protruding right flange ear onto floor
        for dy in 1..=fh {
            darken_px(pb, x + w, ftop + dy, 0.48);
            darken_px(pb, x + w + 1, ftop + dy, 0.68);
        }
        // Cast shadow below flange onto the pipe shaft
        if ftop + fh < y + h {
            for lx in 1..(sw - 1) {
                darken_px(pb, sx0 + lx, ftop + fh, 0.46);
                if ftop + fh + 1 < y + h {
                    darken_px(pb, sx0 + lx, ftop + fh + 1, 0.72);
                }
            }
        }
        for dy in 0..fh {
            let py = ftop + dy;
            if py < y || py >= y + h {
                continue;
            }
            for lx in 0..w {
                if lx == 0 || lx == w - 1 || (idx == 0 && dy == 0) || (idx == 3 && dy == fh - 1) {
                    pb.plot(x + lx, py, outline);
                    continue;
                }
                let t = lx as f32 / (w.max(2) - 1) as f32;
                let cyl = (1.0 - ((t - 0.25) * 1.55).abs()).clamp(0.0, 1.0);
                let vert = if dy == 0 || (idx == 0 && dy == 1) {
                    1.34
                } else if dy == fh - 1 {
                    0.52
                } else {
                    0.96
                };
                let mut fc = palette::scale((86, 108, 106), (0.48 + 0.80 * cyl) * vert);
                if lx == 1 {
                    fc = palette::mix(fc, spec, 0.28);
                }
                pb.plot(x + lx, py, fc);
            }
        }
        // 3 raised flange bolts
        if w >= 10 && ftop + 2 < y + h {
            let bhi = (176, 198, 192);
            let blo = (32, 42, 40);
            let by = if idx == 0 { ftop + 2 } else { ftop + 1 };
            pb.plot(x + 2, by, bhi);
            pb.plot(x + 2, by + 1, blo);
            pb.plot(x + w / 2, by, bhi);
            pb.plot(x + w / 2, by + 1, blo);
            pb.plot(x + w - 3, by, palette::scale(bhi, 0.65));
            pb.plot(x + w - 3, by + 1, blo);
        }
    }

    // 3. Leaking joint trickle + damp puddle sheen beside the pipe
    let leak_flange = if seed & 1 == 0 { flanges[1] } else { flanges[2] };
    let lx_drip = x + w - 2;
    let ly_drip = y + leak_flange + 3;
    // Wet trickle down the side of the flange + droplet bead
    pb.plot(lx_drip, ly_drip, (72, 134, 114));
    pb.plot(lx_drip + 1, ly_drip + 1, (92, 158, 136));
    pb.plot(lx_drip + 1, ly_drip + 2, (148, 212, 192));
    // Small wet sheen puddle at the base of the pipe
    let px_p = x + w / 2;
    let py_p = y + h + 1;
    for dx in -2..=2 {
        blend_px(pb, px_p + dx, py_p, (40, 78, 68), 0.55 - dx.abs() as f32 * 0.12);
    }
    blend_px(pb, px_p - 1, py_p, (96, 156, 138), 0.45);
}

// ---- 2. PipeH (72 x 14) ----------------------------------------------------

fn pipe_h(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y + 1, w, (h - 2).max(1));

    let metal = (96, 122, 118);
    let spec = (192, 214, 208);
    let rust = (118, 74, 48);
    let verdigris = (66, 124, 108);
    let outline = ambient_shade((24, 34, 32), pal, 0.25);

    let flanges = [0, w / 3 - 2, (w * 2) / 3 - 1, w - 5];

    // 1. Recessed horizontal shaft (inset 1px top/bottom so flanges visibly protrude!)
    let sy0 = if h >= 8 { y + 1 } else { y };
    let sh = if h >= 8 { h - 2 } else { h };
    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let ya = sy0.max(0);
    let yb = (sy0 + sh).min(bh);
    let xa = x.max(0);
    let xb = (x + w).min(bw);

    for py in ya..yb {
        let ly = py - sy0;
        let t = ly as f32 / (sh.max(2) - 1) as f32;
        let diff = (1.0 - ((t - 0.26) * 1.65).abs()).clamp(0.0, 1.0);
        let bounce = if ly == sh - 2 { 0.14 } else { 0.0 };
        let streak = (hash_f32((seed as i32 & 15) + 7, ly) - 0.5) * 0.06;
        for px in xa..xb {
            let lx = px - x;
            if ly == 0 || ly == sh - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let mut near_flange = 99;
            for &fx in &flanges {
                let d = (lx - (fx + 2)).abs();
                if d < near_flange {
                    near_flange = d;
                }
            }
            let mut c = palette::scale(metal, 0.38 + 0.92 * diff + bounce + streak);
            if (t - 0.23).abs() < 0.085 {
                c = palette::mix(c, spec, 0.52);
            }
            if near_flange <= 5 {
                let stain = hash_f32(lx / 3 + 3, ly / 2 + (seed as i32 & 15));
                if stain > 0.62 {
                    let k = (1.0 - near_flange as f32 / 6.0) * 0.36;
                    let pat = if (seed ^ (ly as u32)) & 1 == 0 { rust } else { verdigris };
                    c = palette::mix(c, pat, k);
                }
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Protruding bolted flange collars (full height `h`)
    for (idx, &fx) in flanges.iter().enumerate() {
        let fw = if idx == 0 || idx == 3 { 5 } else { 4 };
        let fleft = x + fx;
        // Shadow under protruding flange bottom
        for dx in 1..=fw {
            darken_px(pb, fleft + dx, y + h, 0.44);
            darken_px(pb, fleft + dx, y + h + 1, 0.68);
        }
        // Cast shadow to the right of each flange on the pipe shaft
        if fleft + fw < x + w {
            for ly in 1..(sh - 1) {
                darken_px(pb, fleft + fw, sy0 + ly, 0.46);
                if fleft + fw + 1 < x + w {
                    darken_px(pb, fleft + fw + 1, sy0 + ly, 0.72);
                }
            }
        }
        for dx in 0..fw {
            let px = fleft + dx;
            if px < x || px >= x + w {
                continue;
            }
            for ly in 0..h {
                if ly == 0 || ly == h - 1 || (idx == 0 && dx == 0) || (idx == 3 && dx == fw - 1) {
                    pb.plot(px, y + ly, outline);
                    continue;
                }
                let t = ly as f32 / (h.max(2) - 1) as f32;
                let cyl = (1.0 - ((t - 0.25) * 1.55).abs()).clamp(0.0, 1.0);
                let horiz = if dx == 0 || (idx == 0 && dx == 1) {
                    1.34
                } else if dx == fw - 1 {
                    0.52
                } else {
                    0.96
                };
                let mut fc = palette::scale((86, 108, 106), (0.48 + 0.80 * cyl) * horiz);
                if ly == 1 {
                    fc = palette::mix(fc, spec, 0.28);
                }
                pb.plot(px, y + ly, fc);
            }
        }
        // 3 flange bolts
        if h >= 10 && fleft + 2 < x + w {
            let bhi = (176, 198, 192);
            let blo = (32, 42, 40);
            let bx = if idx == 0 { fleft + 2 } else { fleft + 1 };
            pb.plot(bx, y + 2, bhi);
            pb.plot(bx + 1, y + 2, blo);
            pb.plot(bx, y + h / 2, bhi);
            pb.plot(bx + 1, y + h / 2, blo);
            pb.plot(bx, y + h - 3, palette::scale(bhi, 0.65));
            pb.plot(bx + 1, y + h - 3, blo);
        }
    }

    // 3. Hanging water droplet + small wet puddle underneath a middle joint
    let leak_x = x + if seed & 1 == 0 { flanges[1] } else { flanges[2] } + 2;
    let leak_y = y + h;
    pb.plot(leak_x, leak_y - 1, (84, 152, 132));
    pb.plot(leak_x, leak_y, (148, 212, 192));
    for dx in -2..=2 {
        blend_px(pb, leak_x + dx, leak_y + 2, (42, 82, 70), 0.62 - dx.abs() as f32 * 0.14);
    }
    blend_px(pb, leak_x - 1, leak_y + 2, (112, 174, 154), 0.45);
}

// ---- 3. Barrier (16 x 66) --------------------------------------------------

fn barrier(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let steel = palette::mix((78, 86, 104), pal.wall, 0.18);
    let steel_hi = palette::scale(steel, 1.48);
    let ochre = (190, 156, 70);
    let dark_stripe = palette::mix((42, 48, 62), pal.bg, 0.22);
    let foot_col = palette::scale(steel, 0.84);
    let outline = ambient_shade((20, 24, 32), pal, 0.25);

    let bw = pb.w as i32;
    let bh = pb.h as i32;
    let ya = y.max(0);
    let yb = (y + h).min(bh);
    let xa = x.max(0);
    let xb = (x + w).min(bw);

    let front_h = 5.min(h / 4);
    let top_h = (h - front_h).max(1);
    let mid_y = top_h / 2;

    for py in ya..yb {
        let ly = py - y;
        let on_foot = ly < 6 || ly >= top_h - 5 || (ly >= mid_y - 2 && ly <= mid_y + 2);
        let is_front = ly >= top_h;
        // Beam steps inward by 1px between the protruding bolted steel feet!
        let inset = if on_foot || is_front || w < 12 { 0 } else { 1 };

        for px in xa..xb {
            let lx = px - x;
            if lx < inset || lx >= w - inset {
                // Soft side shadow along recessed waist between feet
                if lx == w - 1 {
                    darken_px(pb, px, py, 0.52);
                }
                continue;
            }
            if lx == inset || lx == w - 1 - inset || ly == 0 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }

            if is_front {
                // Chunky 3/4 south vertical face of the barricade block with structural gusset ribs
                let shade = if ly == top_h {
                    0.86
                } else if ly == h - 2 {
                    0.44
                } else {
                    0.62
                };
                let side_grad = 1.12 - 0.34 * (lx as f32 / w.max(1) as f32);
                let mut c = palette::scale(steel, shade * side_grad);
                if lx == 1 {
                    c = palette::mix(c, steel_hi, 0.25);
                } else if lx == 4 || lx == w - 5 {
                    c = palette::scale(c, 0.72); // vertical steel gusset seam
                } else if lx == 5 || lx == w - 4 {
                    c = palette::scale(c, 1.18); // gusset bevel catchlight
                }
                pb.plot(px, py, c);
                continue;
            }

            if on_foot {
                let left_lit = 1.22 - 0.44 * (lx as f32 / w.max(1) as f32);
                let top_lit = if ly == 1 || ly == mid_y - 2 || ly == top_h - 5 {
                    1.28
                } else if ly == 5 || ly == mid_y + 2 || ly == top_h - 1 {
                    0.62
                } else {
                    1.0
                };
                pb.plot(px, py, palette::scale(foot_col, left_lit * top_lit));
                continue;
            }

            // 3D jersey-barricade body with sharp central top crest & sloped sides
            let stripe_band = lx >= 3 && lx <= w - 4;
            let slope_light = if lx == 2 {
                1.30 // bright lit left bevel
            } else if lx == 3 {
                1.18
            } else if lx == w / 2 - 1 {
                1.14 // subtle top-left ridge crest catchlight
            } else if lx >= w - 3 {
                0.52 // deep shaded right slope
            } else if lx == w - 4 {
                0.72
            } else {
                1.04 - 0.20 * ((lx - 3) as f32 / (w - 6).max(1) as f32)
            };

            let mut base = if stripe_band {
                let cx = lx - w / 2;
                let chev = (ly + cx.abs()).rem_euclid(10);
                if chev < 4 {
                    ochre
                } else {
                    dark_stripe
                }
            } else {
                steel
            };

            // Subtle weathered paint scuffs
            let wear = hash_f32(lx + (seed as i32 & 31), ly / 2 + ((seed >> 6) as i32 & 31));
            if wear > 0.91 {
                base = palette::mix(base, steel, 0.50);
            } else if wear < 0.07 {
                base = palette::scale(base, 0.78);
            }

            pb.plot(px, py, palette::scale(base, slope_light));
        }
    }

    // Cast shadow just below top & middle anchor brackets onto the chevron beam
    for sy in [y + 6, y + mid_y + 3] {
        for lx in 2..(w - 2) {
            darken_px(pb, x + lx, sy, 0.58);
        }
    }

    // Anchor plate bolts
    let bhi = (176, 188, 204);
    let blo = (28, 32, 42);
    for by in [y + 2, y + mid_y - 1, y + top_h - 4] {
        if by > y && by + 1 < y + top_h {
            bolt(pb, x + 2, by, bhi, blo);
            bolt(pb, x + w - 4, by, palette::scale(bhi, 0.68), blo);
        }
    }
    // Muted hazard reflector strip on top & bottom cap centers
    let pip = palette::mix(pal.accent, (215, 178, 96), 0.55);
    pb.rect_fill(x + w / 2 - 2, y + 3, 4, 1, palette::scale(pip, 0.78));
    pb.rect_fill(x + w / 2 - 2, y + top_h - 3, 4, 1, palette::scale(pip, 0.68));
}

// ---- 4. Valve (20 x 20) ----------------------------------------------------

fn valve(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let iron = (86, 104, 102);
    let iron_hi = (144, 166, 160);
    let iron_dark = (40, 50, 48);
    let outline = ambient_shade((22, 30, 28), pal, 0.25);

    let front_h = 3.min(h / 5);
    let top_h = (h - front_h).max(1);

    // 1. Cast-iron square manifold plate (fills full 20x20 collision footprint)
    for ly in 0..h {
        for lx in 0..w {
            let px = x + lx;
            let py = y + ly;
            if (lx == 0 || lx == w - 1) && (ly == 0 || ly == h - 1) {
                darken_px(pb, px, py, 0.56);
                continue;
            }
            if lx == 0 || lx == w - 1 || ly == 0 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            if ly >= top_h {
                let f = if ly == top_h { 0.78 } else { 0.50 };
                let g = 1.10 - 0.28 * (lx as f32 / w.max(1) as f32);
                pb.plot(px, py, palette::scale(iron, f * g));
                continue;
            }
            let gx = lx as f32 / w.max(1) as f32;
            let gy = ly as f32 / top_h.max(1) as f32;
            let light = 1.18 - 0.38 * (0.55 * gx + 0.45 * gy);
            let mut c = palette::scale(iron, light);
            let n = hash_f32(lx / 2 + (seed as i32 & 15), ly / 2 + ((seed >> 4) as i32 & 15));
            if n > 0.85 {
                c = palette::mix(c, (112, 72, 48), 0.22);
            }
            pb.plot(px, py, c);
        }
    }

    pb.rect_fill(x + 1, y + 1, w - 2, 1, iron_hi);
    pb.rect_fill(x + 1, y + 1, 1, top_h - 1, iron_hi);
    pb.rect_fill(x + 1, y + top_h - 1, w - 2, 1, iron_dark);

    // Corner mounting bolts
    let bhi = (172, 192, 186);
    let blo = (28, 36, 34);
    bolt(pb, x + 2, y + 2, bhi, blo);
    bolt(pb, x + w - 4, y + 2, palette::scale(bhi, 0.78), blo);
    bolt(pb, x + 2, y + top_h - 4, palette::scale(bhi, 0.80), blo);
    bolt(pb, x + w - 4, y + top_h - 4, palette::scale(bhi, 0.66), blo);

    // 2. Dark recessed stem well + raised wheel cast shadow
    let cx = x as f32 + w as f32 * 0.5;
    let cy_base = y as f32 + top_h as f32 * 0.53;
    let cy_wheel = cy_base - 0.9;
    let r_outer = (w.min(top_h) as f32 * 0.44).clamp(4.0, 7.6);
    let r_inner = (r_outer - 1.65).max(2.6);

    shadow_ellipse(pb, cx + 1.2, cy_base + 1.5, r_outer + 0.4, r_outer * 0.90 + 0.4, 0.58);

    // Recessed circular valve bonnet well underneath the spokes (high contrast!)
    for ly in 1..(top_h - 1) {
        for lx in 1..(w - 1) {
            let dx = x as f32 + lx as f32 + 0.5 - cx;
            let dy = y as f32 + ly as f32 + 0.5 - cy_base;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= r_inner + 0.4 {
                let c = if d <= 2.6 {
                    // Brass packing collar under the hub
                    (86, 74, 48)
                } else {
                    (24, 32, 30)
                };
                pb.plot(x + lx, y + ly, c);
            }
        }
    }

    // 3. Crisp sculpted 4-spoke industrial oxide-red handwheel
    let wheel_base = (158, 66, 54);
    let wheel_hi = (212, 122, 98);
    let wheel_lo = (74, 28, 24);
    let rot = if seed & 1 == 0 {
        0.0_f32
    } else {
        std::f32::consts::FRAC_PI_4
    };

    let x0 = (cx - r_outer - 1.0).floor() as i32;
    let x1 = (cx + r_outer + 1.0).ceil() as i32;
    let y0 = (cy_wheel - r_outer - 1.0).floor() as i32;
    let y1 = (cy_wheel + r_outer + 1.0).ceil() as i32;

    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let dy = (py as f32 + 0.5 - cy_wheel) * 1.08;
            let d = (dx * dx + dy * dy).sqrt();

            let on_rim = d <= r_outer && d >= r_inner;
            let on_halo = d > r_outer && d <= r_outer + 0.85;
            let ang = dy.atan2(dx) - rot;
            let on_spoke = d < r_inner + 0.35 && {
                let s = (ang.sin().abs() * d).min(ang.cos().abs() * d);
                s <= 0.72
            };

            if on_halo {
                // Subtle dark outline halo so the handwheel separates crisply from the top-left metal plate
                darken_px(pb, px, py, 0.56);
            } else if on_rim || on_spoke {
                let light = ((-dx - dy) / (r_outer * 1.414)).clamp(-1.0, 1.0);
                let mut c = if light > 0.0 {
                    palette::mix(wheel_base, wheel_hi, light * 0.76)
                } else {
                    palette::mix(wheel_base, wheel_lo, (-light) * 0.80)
                };
                if on_spoke && !on_rim {
                    c = palette::scale(c, 0.84);
                }
                if on_rim && light > 0.68 {
                    c = palette::mix(c, (242, 194, 174), 0.52);
                }
                pb.plot(px, py, c);
            }
        }
    }

    // Central brass hex nut
    let hx = cx.round() as i32;
    let hy = cy_wheel.round() as i32;
    pb.rect_fill(hx - 1, hy - 1, 3, 3, (130, 108, 62));
    pb.plot(hx - 1, hy - 1, (212, 188, 126));
    pb.plot(hx, hy, (172, 148, 90));
    pb.plot(hx + 1, hy + 1, (64, 50, 26));
}

// ---- 5. Grate (30 x 30) ----------------------------------------------------

fn grate(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let frame_col = palette::mix((88, 110, 104), pal.wall_edge, 0.22);
    let frame_hi = palette::scale(frame_col, 1.40);
    let frame_lo = palette::scale(frame_col, 0.48);
    let outline = ambient_shade((18, 26, 24), pal, 0.25);

    let border = 3;
    let front_curb = 2; // Extra 3/4 vertical height at bottom of outer curb
    let inner_x0 = x + border;
    let inner_y0 = y + border;
    let inner_w = (w - border * 2).max(1);
    let inner_h = (h - border * 2 - front_curb).max(1);

    // 1. Deep dark storm-drain pit with visible masonry drop walls & submerged tunnel/water
    for dy in 0..inner_h {
        let py = inner_y0 + dy;
        for dx in 0..inner_w {
            let px = inner_x0 + dx;
            // 5px stepped interior north & west pit walls receding into darkness
            if dy < 5 || dx < 4 {
                let depth_t = if dy < 5 && dx < 4 {
                    (dy as f32 / 5.0).max(dx as f32 / 4.0)
                } else if dy < 5 {
                    dy as f32 / 5.0
                } else {
                    dx as f32 / 4.0
                };
                let mut wall_c = palette::scale((42, 60, 56), 0.92 - 0.65 * depth_t);
                // Masonry course mortar seams on the inner pit wall
                if dy == 2 || (dy < 5 && (dx + (dy / 2) * 3) % 6 == 0) {
                    wall_c = palette::scale(wall_c, 0.62);
                }
                pb.plot(px, py, wall_c);
                continue;
            }
            // Deep abyss water far below
            let mut water = (7, 15, 16);
            // Dark submerged cross-pipe running deep under the grate
            let pipe_cy = inner_h / 2 + 3;
            let pd = (dy - pipe_cy).abs();
            if pd <= 3 {
                let lit = if dy < pipe_cy { 1.15 } else { 0.65 };
                let pt = 1.0 - pd as f32 / 4.0;
                water = palette::mix(water, palette::scale((22, 42, 40), lit), pt * 0.72);
            }
            // Crisp faint diagonal water shimmer deep in the pit
            let diag = (dx + dy + (seed as i32 & 3)).rem_euclid(13);
            if (diag == 0 || diag == 1) && dy > 7 && dx > 6 && pd > 2 {
                let s = if diag == 0 { 0.42 } else { 0.22 };
                water = palette::mix(water, (44, 88, 82), s);
            }
            pb.plot(px, py, water);
        }
    }

    // 2. Heavy iron grate bars spanning the pit with crisp shifted cast shadows below
    let bar_hi = palette::scale(frame_col, 1.32);
    let bar_lo = palette::scale(frame_col, 0.42);

    // Shadow pass first (offset +2, +2 down into the pit for strong floating-bar depth!)
    let mut bx = 3;
    while bx < inner_w - 2 {
        for dy in 2..inner_h {
            darken_px(pb, inner_x0 + bx + 2, inner_y0 + dy, 0.42);
        }
        bx += 5;
    }
    let mut by = 4;
    while by < inner_h - 2 {
        for dx in 2..inner_w {
            darken_px(pb, inner_x0 + dx, inner_y0 + by + 2, 0.42);
        }
        by += 6;
    }

    // Draw vertical bars
    let mut bx = 3;
    while bx < inner_w - 2 {
        let px = inner_x0 + bx;
        for dy in 0..inner_h {
            let py = inner_y0 + dy;
            let ao = if dy == 0 || dy == inner_h - 1 { 0.74 } else { 1.0 };
            pb.plot(px, py, palette::scale(bar_hi, ao));
            pb.plot(px + 1, py, palette::scale(bar_lo, ao));
        }
        bx += 5;
    }
    // Draw horizontal cross-bars
    let mut by = 4;
    while by < inner_h - 2 {
        let py = inner_y0 + by;
        for dx in 0..inner_w {
            let px = inner_x0 + dx;
            let ao = if dx == 0 || dx == inner_w - 1 { 0.74 } else { 1.0 };
            pb.plot(px, py, palette::scale(bar_hi, ao));
            pb.plot(px, py + 1, palette::scale(bar_lo, ao));
        }
        by += 6;
    }
    // Bright weld speculars at bar crossings
    let mut bx2 = 3;
    while bx2 < inner_w - 2 {
        let mut by2 = 4;
        while by2 < inner_h - 2 {
            pb.plot(inner_x0 + bx2, inner_y0 + by2, palette::scale(frame_hi, 1.15));
            by2 += 6;
        }
        bx2 += 5;
    }

    // 3. Heavy raised cast-iron curb frame + 3/4 vertical front skirt
    let top_h = h - front_curb;
    for ly in 0..h {
        for lx in 0..w {
            let in_pit = lx >= border && lx < w - border && ly >= border && ly < top_h - border;
            if in_pit {
                continue;
            }
            let px = x + lx;
            let py = y + ly;
            if lx == 0 || lx == w - 1 || ly == 0 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            if ly >= top_h {
                // Darker 3/4 vertical front curb wall
                let g = 1.08 - 0.26 * (lx as f32 / w.max(1) as f32);
                pb.plot(px, py, palette::scale(frame_lo, g));
                continue;
            }
            let c = if ly == 1 || lx == 1 {
                frame_hi
            } else if ly == top_h - 1 || lx == w - 2 {
                palette::scale(frame_col, 0.62)
            } else {
                let g = 1.12 - 0.28 * ((lx + ly) as f32 / (w + top_h).max(1) as f32);
                palette::scale(frame_col, g)
            };
            pb.plot(px, py, c);
        }
    }
    // Inner lip bevel around the pit opening
    bevel_frame(
        pb,
        inner_x0 - 1,
        inner_y0 - 1,
        inner_w + 2,
        inner_h + 2,
        palette::scale(frame_col, 0.44),
        palette::scale(frame_col, 1.18),
    );

    // Corner anchor bolts + sewer moss detail
    let bhi = (170, 192, 184);
    let blo = (28, 38, 36);
    bolt(pb, x + 1, y + 1, bhi, blo);
    bolt(pb, x + w - 3, y + 1, palette::scale(bhi, 0.76), blo);
    bolt(pb, x + 1, y + top_h - 3, palette::scale(bhi, 0.78), blo);
    bolt(pb, x + w - 3, y + top_h - 3, palette::scale(bhi, 0.64), blo);

    let moss = (58, 92, 66);
    let mx = if seed & 1 == 0 { x + 2 } else { x + w - 5 };
    let my = y + top_h - 3;
    blend_px(pb, mx, my, moss, 0.60);
    blend_px(pb, mx + 1, my, moss, 0.50);
    blend_px(pb, mx + 1, my - 1, moss, 0.35);
}

// ---- 6. Table (40 x 26) ----------------------------------------------------

fn table(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let wood_base = (160, 116, 70);
    let wood_hi = (206, 164, 110);
    let wood_seam = (96, 66, 36);
    let apron_col = (98, 66, 38);
    let leg_col = (114, 78, 44);
    let under_dark = (26, 17, 11);
    let outline = ambient_shade((34, 22, 12), pal, 0.25);

    let leg_h = 4.min(h / 5);
    let apron_h = 3.min(h / 6);
    let top_h = (h - leg_h - apron_h).max(4);

    // 1. Tabletop planks
    let plank_h = 4;
    for ly in 0..top_h {
        let py = y + ly;
        let plank_idx = ly / plank_h;
        let in_plank = ly % plank_h;
        let plank_tint = match plank_idx % 3 {
            0 => 1.04,
            1 => 0.95,
            _ => 1.00,
        };
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            if ly == 1 || lx == 1 {
                let g = 1.04 - 0.18 * ((lx + ly) as f32 / (w + top_h).max(1) as f32);
                pb.plot(px, py, palette::scale(wood_hi, g));
                continue;
            }
            if ly == top_h - 1 {
                pb.plot(px, py, palette::scale(wood_base, 0.80));
                continue;
            }
            if lx == w - 2 {
                pb.plot(px, py, palette::scale(wood_base, 0.66));
                continue;
            }
            if in_plank == 0 && ly > 1 {
                pb.plot(px, py, wood_seam);
                continue;
            }
            let joint_x = 10 + ((plank_idx * 13 + (seed as i32 & 7)) % (w - 16).max(4));
            if lx == joint_x {
                pb.plot(px, py, wood_seam);
                continue;
            }

            let grad = 1.10
                - 0.22
                    * (0.6 * (lx as f32 / w.max(1) as f32)
                        + 0.4 * (ly as f32 / top_h.max(1) as f32));
            let grain =
                (hash_f32(lx / 3 + plank_idx * 17, ly + (seed as i32 & 31)) - 0.5) * 0.14;
            pb.plot(
                px,
                py,
                palette::scale(wood_base, (plank_tint * grad + grain).max(0.4)),
            );
        }
    }

    // 2. Front vertical apron with crisp overhang shadow at dy == 0
    for dy in 0..apron_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let s = if dy == 0 {
                0.68 // overhang cast shadow
            } else if dy == apron_h - 1 {
                0.76
            } else {
                1.04
            };
            let g = 1.10 - 0.26 * (lx as f32 / w.max(1) as f32);
            pb.plot(px, py, palette::scale(apron_col, s * g));
        }
    }

    // 3. Sturdy front table legs contrasting crisply against dark recessed underside + stretcher bar
    let leg_y0 = y + top_h + apron_h;
    for dy in 0..leg_h {
        let py = leg_y0 + dy;
        for lx in 0..w {
            let px = x + lx;
            if py == y + h - 1 || lx == 0 || lx == w - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let is_left_leg = (1..=4).contains(&lx);
            let is_right_leg = (w - 5..=w - 2).contains(&lx);
            if is_left_leg || is_right_leg {
                let lit = if (is_left_leg && lx == 1) || (is_right_leg && lx == w - 5) {
                    1.28
                } else if (is_left_leg && lx == 4) || (is_right_leg && lx == w - 2) {
                    0.62
                } else {
                    0.98
                };
                let vert = if dy == 0 { 0.80 } else { 1.0 };
                pb.plot(px, py, palette::scale(leg_col, lit * vert));
            } else {
                // Dark under-table void with dim recessed lower shelf / cross-rail
                let c = if dy == leg_h - 2 {
                    (58, 39, 24)
                } else {
                    under_dark
                };
                pb.plot(px, py, c);
            }
        }
    }

    // 4. Tabletop items (cutting board + knife, folded kitchen towel/bowl, porcelain plate + roll)
    let flip = (seed & 1) != 0;
    if w >= 32 && top_h >= 15 {
        // Maple cutting board + steel kitchen knife
        let bx = if flip { x + w - 16 } else { x + 4 };
        let by = y + 4;
        let bw_b = 11;
        let bh_b = 8;
        for dx in 1..=bw_b {
            darken_px(pb, bx + dx, by + bh_b, 0.58);
        }
        for dy in 1..bh_b {
            darken_px(pb, bx + bw_b, by + dy, 0.60);
        }
        pb.rect_fill(bx, by, bw_b, bh_b, (190, 152, 98));
        bevel_frame(pb, bx, by, bw_b, bh_b, (220, 184, 130), (122, 90, 52));
        pb.plot(bx + bw_b - 2, by + bh_b / 2, (94, 66, 36));
        // Diagonal chef's knife with shadow
        pb.line(bx + 2, by + 3, bx + 6, by + 6, (108, 82, 48)); // shadow
        pb.line(bx + 2, by + 2, bx + 6, by + 5, (204, 212, 222)); // blade
        pb.plot(bx + 3, by + 2, (234, 240, 246)); // glint
        pb.line(bx + 7, by + 5, bx + 8, by + 6, (62, 42, 28)); // handle

        // Ceramic plate with inner recess & warm crusty bread roll
        let px_c = if flip { x + 10 } else { x + w - 10 };
        let py_c = y + 8;
        shadow_ellipse(pb, px_c as f32 + 1.0, py_c as f32 + 1.2, 4.5, 3.8, 0.42);
        for dy in -4..=4 {
            for dx in -4..=4 {
                let d2 = dx * dx + dy * dy;
                if d2 <= 16 {
                    let c = if d2 >= 10 {
                        if dx + dy < 0 {
                            (212, 208, 200)
                        } else {
                            (146, 142, 134)
                        }
                    } else if d2 >= 6 && dx + dy < 0 {
                        (158, 154, 146) // inner rim shadow
                    } else {
                        (190, 186, 178)
                    };
                    pb.plot(px_c + dx, py_c + dy, c);
                }
            }
        }
        // Small warm toasted roll sitting in center of plate (muted tan)
        pb.rect_fill(px_c - 1, py_c - 1, 3, 2, (166, 118, 64));
        pb.plot(px_c - 1, py_c - 1, (204, 162, 104));
        pb.plot(px_c + 1, py_c, (112, 74, 38));

        // Center item: folded striped kitchen cloth or stoneware mug
        let cx_m = x + w / 2 - 2;
        let cy_m = y + 5;
        if (seed & 2) == 0 {
            pb.rect_fill(cx_m, cy_m, 6, 8, (162, 156, 144));
            pb.rect_fill(cx_m, cy_m, 6, 1, (194, 188, 176));
            pb.rect_fill(cx_m, cy_m + 7, 6, 1, (104, 98, 88));
            pb.rect_fill(cx_m + 1, cy_m + 2, 4, 1, (118, 108, 102));
            pb.rect_fill(cx_m + 1, cy_m + 5, 4, 1, (118, 108, 102));
            for dy in 1..8 {
                darken_px(pb, cx_m + 6, cy_m + dy, 0.64);
            }
        } else {
            shadow_ellipse(pb, (cx_m + 3) as f32, (cy_m + 4) as f32, 3.6, 3.4, 0.36);
            pb.filled_circle(cx_m + 2, cy_m + 3, 3, (138, 124, 108));
            pb.filled_circle(cx_m + 2, cy_m + 3, 2, (74, 54, 38));
            pb.plot(cx_m + 1, cy_m + 1, (186, 174, 158));
        }
    }
}

// ---- 7. Crate (22 x 22) ----------------------------------------------------

fn crate_box(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let wood_top = (162, 120, 70);
    let wood_hi = (206, 164, 108);
    let wood_recess = (128, 92, 50);
    let wood_front = (104, 72, 38);
    let wood_seam = (68, 44, 24);
    let iron = (68, 74, 84);
    let iron_hi = (152, 160, 172);
    let outline = ambient_shade((32, 22, 12), pal, 0.30);

    let front_h = 6.min(h / 3);
    let top_h = (h - front_h).max(6);
    let rail = 3;

    // 1. Top lid face
    for ly in 0..top_h {
        for lx in 0..w {
            let px = x + lx;
            let py = y + ly;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let on_rail = lx < rail || lx >= w - rail || ly < rail || ly >= top_h - rail;
            let g = 1.12
                - 0.24
                    * (0.55 * (lx as f32 / w.max(1) as f32)
                        + 0.45 * (ly as f32 / top_h.max(1) as f32));
            let grain = (hash_f32(
                lx / 2 + (seed as i32 & 31),
                ly + ((seed >> 5) as i32 & 31),
            ) - 0.5)
                * 0.12;

            if on_rail {
                let mut c = palette::scale(wood_top, g + grain);
                if ly == 1 || lx == 1 {
                    c = palette::mix(c, wood_hi, 0.62);
                } else if ly == top_h - 1 || lx == w - 2 {
                    c = palette::scale(c, 0.70);
                }
                pb.plot(px, py, c);
            } else {
                let mut c = palette::scale(wood_recess, g + grain);
                if (ly - rail) % 3 == 0 && ly > rail {
                    c = palette::scale(c, 0.74);
                }
                if ly == rail || lx == rail {
                    c = palette::scale(c, 0.62);
                } else if ly == top_h - rail - 1 || lx == w - rail - 1 {
                    c = palette::scale(c, 1.18);
                }
                pb.plot(px, py, c);
            }
        }
    }

    // Diagonal wooden cross-brace across the recessed lid
    let ix0 = x + rail;
    let iy0 = y + rail;
    let iw = (w - rail * 2).max(1);
    let ih = (top_h - rail * 2).max(1);
    let diag_flip = (seed & 1) != 0;
    for dy in 0..ih {
        for dx in 0..iw {
            let u = if diag_flip { iw - 1 - dx } else { dx };
            let target = (u * ih) / iw.max(1);
            let dist = dy - target;
            if dist.abs() <= 1 {
                let c = if dist == -1 {
                    wood_hi
                } else if dist == 0 {
                    wood_top
                } else {
                    palette::scale(wood_top, 0.72)
                };
                pb.plot(ix0 + dx, iy0 + dy, c);
            } else if dist == 2 {
                darken_px(pb, ix0 + dx, iy0 + dy, 0.62);
            }
        }
    }

    // 2. Front vertical side face (3/4 cube volume!)
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let side_g = 1.12 - 0.30 * (lx as f32 / w.max(1) as f32);
            let vert_g = if dy == 0 {
                0.62 // crisp lid overhang cast shadow
            } else if dy == front_h - 2 {
                0.76
            } else {
                1.02
            };
            let mut c = palette::scale(wood_front, side_g * vert_g);
            if lx == rail || lx == w - rail - 1 || lx == w / 2 {
                c = wood_seam;
            }
            pb.plot(px, py, c);
        }
    }

    // 3. Iron corner brackets on lid + bottom front corners
    for (cx, cy) in [
        (x + 1, y + 1),
        (x + w - 3, y + 1),
        (x + 1, y + top_h - 3),
        (x + w - 3, y + top_h - 3),
    ] {
        pb.rect_fill(cx, cy, 2, 2, iron);
        pb.plot(cx, cy, iron_hi);
    }
    // Wrapped iron corner straps at the bottom of front wall
    pb.rect_fill(x + 1, y + h - 3, 2, 2, palette::scale(iron, 0.78));
    pb.plot(x + 1, y + h - 3, palette::scale(iron_hi, 0.78));
    pb.rect_fill(x + w - 3, y + h - 3, 2, 2, palette::scale(iron, 0.62));

    // Tiny nail heads along top rail midpoints and brace ends
    let nail = (46, 34, 24);
    pb.plot(x + w / 2, y + 1, nail);
    pb.plot(x + w / 2, y + top_h - 2, nail);
    pb.plot(x + 1, y + top_h / 2, nail);
    pb.plot(x + w - 2, y + top_h / 2, nail);
    if iw >= 8 && ih >= 6 {
        if diag_flip {
            pb.plot(ix0 + iw - 2, iy0 + 1, nail);
            pb.plot(ix0 + 1, iy0 + ih - 2, nail);
        } else {
            pb.plot(ix0 + 1, iy0 + 1, nail);
            pb.plot(ix0 + iw - 2, iy0 + ih - 2, nail);
        }
    }

    // 4. Single crisp shipping stencil (upward cargo chevron + stripe) on front left
    if front_h >= 5 && w >= 16 {
        let stencil = (48, 32, 18);
        let sx = x + 5;
        let sy = y + top_h + 2;
        pb.plot(sx + 1, sy, stencil);
        pb.rect_fill(sx, sy + 1, 3, 1, stencil);
        pb.rect_fill(sx + 1, sy + 2, 1, 1, stencil);
        // Cargo code bars on right
        pb.rect_fill(x + w - 8, sy + 1, 4, 1, stencil);
        pb.rect_fill(x + w - 7, sy + 2, 2, 1, stencil);
    }
}

// ---- 8. Barrel (20 x 20) ---------------------------------------------------

fn barrel(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    let cx = x as f32 + w as f32 * 0.5;
    let cy_bot = y as f32 + h as f32 - 3.5;
    let rx_base = (w as f32 * 0.47).max(4.0);

    shadow_ellipse(pb, cx + 1.8, cy_bot + 1.6, rx_base + 1.2, 4.8, 0.58);

    let oak_body = (148, 100, 56);
    let oak_lid = (164, 118, 70);
    let oak_hi = (206, 162, 108);
    let hoop_col = (54, 60, 70);
    let hoop_hi = (186, 196, 210);
    let outline = ambient_shade((32, 20, 10), pal, 0.25);

    let lid_cy = y as f32 + (h as f32 * 0.34);
    let lid_rx = (w as f32 * 0.44).max(3.5);
    let lid_ry = (h as f32 * 0.26).max(2.5);
    let bot_cy = y as f32 + h as f32 - 4.2;
    let bot_ry = 3.8;

    // 1. Coopered curved barrel belly
    for ly in 0..h {
        let py = y + ly;
        let fy = py as f32 + 0.5;
        if fy < lid_cy {
            continue;
        }
        let vt = ((fy - lid_cy) / (bot_cy - lid_cy).max(1.0)).clamp(0.0, 1.4);
        let bulge = 1.0 + 0.12 * (vt.clamp(0.0, 1.0) * std::f32::consts::PI).sin();
        let row_rx = if fy <= bot_cy {
            lid_rx * bulge
        } else {
            let bny = (fy - bot_cy) / bot_ry;
            if bny >= 1.0 {
                continue;
            }
            lid_rx * (1.0 - bny * bny).sqrt()
        };

        for lx in 0..w {
            let px = x + lx;
            let dx = px as f32 + 0.5 - cx;
            if dx.abs() > row_rx + 0.4 {
                continue;
            }
            if dx.abs() > row_rx - 0.85
                || (fy > bot_cy && {
                    let nx = dx / lid_rx;
                    let ny = (fy - bot_cy) / bot_ry;
                    nx * nx + ny * ny > 0.80
                })
            {
                pb.plot(px, py, outline);
                continue;
            }

            let nx = (dx / row_rx.max(1.0)).clamp(-1.0, 1.0);
            let cyl = (1.0 - ((nx + 0.38) * 0.92).abs()).clamp(0.0, 1.0);
            let vert_ao = 1.0 - 0.18 * vt.clamp(0.0, 1.0);

            let arc_drop = (1.0 - nx * nx).max(0.0).sqrt() * 2.3;
            let hoop1_y = lid_cy + lid_ry * 0.72 + arc_drop;
            let hoop2_y = bot_cy - 1.6 + arc_drop * 0.75;
            let on_hoop = (fy - hoop1_y).abs() <= 0.95 || (fy - hoop2_y).abs() <= 0.95;

            if on_hoop {
                let mut hc = palette::scale(hoop_col, 0.55 + 0.95 * cyl);
                if (nx + 0.44).abs() < 0.18 {
                    hc = palette::mix(hc, hoop_hi, 0.70);
                } else if (nx - 0.10).abs() < 0.08 {
                    // Subtle iron hoop rivet glint
                    hc = palette::mix(hc, hoop_hi, 0.36);
                }
                pb.plot(px, py, hc);
            } else {
                // Alternating subtle stave wood tone + crisp stave seams
                let angle = nx.asin();
                let stave_f = angle * 3.8 + 4.0;
                let stave_idx = stave_f.floor() as i32;
                let stave_tint = if stave_idx % 2 == 0 { 1.04 } else { 0.94 };
                let mut c = palette::scale(oak_body, (0.40 + 0.88 * cyl) * vert_ao * stave_tint);
                if stave_f.fract() < 0.16 {
                    c = palette::scale(c, 0.68);
                }
                if (fy - hoop1_y > 0.95 && fy - hoop1_y < 2.15)
                    || (fy - hoop2_y > 0.95 && fy - hoop2_y < 2.05)
                {
                    c = palette::scale(c, 0.66);
                }
                pb.plot(px, py, c);
            }
        }
    }

    // 2. Elliptical wooden lid recessed inside raised stave rim
    for ly in 0..h {
        let py = y + ly;
        let fy = py as f32 + 0.5;
        let ny = (fy - lid_cy) / lid_ry;
        if ny.abs() > 1.12 {
            continue;
        }
        for lx in 0..w {
            let px = x + lx;
            let nx = (px as f32 + 0.5 - cx) / lid_rx;
            let d = (nx * nx + ny * ny).sqrt();
            if d > 1.08 {
                continue;
            }
            if d > 0.94 {
                pb.plot(px, py, outline);
                continue;
            }
            if d > 0.76 {
                let lip_light = (-nx * 0.65 - ny * 0.75).clamp(-1.0, 1.0);
                let c = if lip_light > 0.0 {
                    palette::mix(oak_lid, oak_hi, lip_light * 0.72)
                } else {
                    palette::scale(oak_lid, 0.58 + 0.42 * (1.0 + lip_light))
                };
                pb.plot(px, py, c);
                continue;
            }
            let light = 1.08 - 0.24 * (0.5 * nx + 0.5 * ny);
            let mut c = palette::scale(oak_lid, light * 0.92);
            if (px - x) % 4 == 0 {
                c = palette::scale(c, 0.72);
            }
            // Inner lip depth lighting: shadowed top-left, lit bottom-right
            if d > 0.56 {
                if (nx + ny) < -0.1 {
                    c = palette::scale(c, 0.60);
                } else if (nx + ny) > 0.2 {
                    c = palette::mix(c, oak_hi, 0.24);
                }
            }
            pb.plot(px, py, c);
        }
    }

    // Wooden bung plug
    let bx = (cx + if seed & 1 == 0 { 2.2 } else { -2.2 }).round() as i32;
    let by = lid_cy.round() as i32;
    pb.plot(bx, by, (62, 40, 20));
    pb.plot(bx - 1, by, (112, 76, 44));
    pb.plot(bx, by - 1, (194, 152, 100));
}

// ---- 9. Counter (66 x 22) --------------------------------------------------

fn counter(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let steel = (146, 154, 164);
    let steel_hi = (208, 216, 226);
    let steel_lo = (78, 84, 96);
    let fascia = (76, 82, 94);
    let outline = ambient_shade((28, 32, 40), pal, 0.25);

    let back_h = 3;
    let front_h = 5.min(h / 4);
    let top_y0 = y + back_h;
    let top_h = (h - back_h - front_h).max(6);
    let front_y0 = top_y0 + top_h;

    // 1. Raised stainless backsplash
    for ly in 0..back_h {
        let py = y + ly;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let c = if ly == 1 {
                palette::scale(steel_hi, 1.0 - 0.14 * (lx as f32 / w.max(1) as f32))
            } else {
                palette::scale(steel, 0.70)
            };
            pb.plot(px, py, c);
        }
    }

    // 2. Brushed stainless countertop
    for dy in 0..top_h {
        let py = top_y0 + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let diag = (lx as f32 * 0.22 + dy as f32 * 0.7).sin() * 0.05;
            let grad = 1.08
                - 0.20
                    * (0.65 * (lx as f32 / w.max(1) as f32)
                        + 0.35 * (dy as f32 / top_h.max(1) as f32));
            let mut c = palette::scale(steel, grad + diag);
            if dy == 0 {
                c = palette::scale(c, 0.74);
            } else if dy == top_h - 1 {
                c = palette::mix(c, steel_hi, 0.58);
            } else if lx == 1 {
                c = palette::mix(c, steel_hi, 0.48);
            } else if lx == w - 2 {
                c = palette::scale(c, 0.70);
            }
            pb.plot(px, py, c);
        }
    }

    // 3. Front stainless cabinet fascia with recessed bays & chrome handles
    for dy in 0..front_h {
        let py = front_y0 + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let vert = if dy == 0 {
                0.66
            } else if dy == front_h - 2 {
                0.76
            } else {
                1.04
            };
            let grad = 1.08 - 0.24 * (lx as f32 / w.max(1) as f32);
            let mut c = palette::scale(fascia, vert * grad);
            if lx == w / 3 || lx == (w * 2) / 3 {
                c = palette::scale(fascia, 0.44);
            } else if lx == w / 3 + 1 || lx == (w * 2) / 3 + 1 {
                c = palette::scale(fascia, 1.18); // door edge highlight
            }
            pb.plot(px, py, c);
        }
    }
    if front_h >= 4 {
        let hy = front_y0 + 2;
        for bay in 0..3 {
            let hx = x + bay * (w / 3) + (w / 6) - 3;
            pb.rect_fill(hx, hy, 6, 1, steel_hi);
            pb.rect_fill(hx, hy + 1, 6, 1, (36, 40, 48));
        }
    }

    // 4. Recessed sink + gooseneck faucet, center gastro pan, maple butcher block + cleaver
    let sink_left = (seed & 1) == 0;
    if w >= 50 && top_h >= 11 {
        // Recessed stainless sink basin (cool steel-blue interior depth)
        let sw = 16;
        let sh = (top_h - 4).clamp(7, 10);
        let sx = if sink_left { x + 5 } else { x + w - 5 - sw };
        let sy = top_y0 + 2;

        for dy in 0..sh {
            for dx in 0..sw {
                let px = sx + dx;
                let py = sy + dy;
                let c = if dy == 0 || dx == 0 {
                    (46, 54, 64)
                } else if dy == 1 || dx == 1 {
                    (66, 78, 92)
                } else if dy == sh - 1 || dx == sw - 1 {
                    (172, 182, 194)
                } else {
                    let sheen = if (dx + dy) % 5 == 0 { 1.10 } else { 1.0 };
                    palette::scale((86, 104, 120), sheen)
                };
                pb.plot(px, py, c);
            }
        }
        // Drain strainer
        let dcx = sx + sw / 2;
        let dcy = sy + sh / 2 + 1;
        pb.rect_fill(dcx - 1, dcy - 1, 3, 2, (34, 40, 48));
        pb.plot(dcx, dcy - 1, (124, 138, 152));

        // Chrome gooseneck faucet + tap knobs + water glint at spout tip
        let fx = sx + sw / 2;
        pb.rect_fill(fx - 3, y + 1, 2, 2, steel_hi);
        pb.rect_fill(fx + 2, y + 1, 2, 2, steel_hi);
        pb.rect_fill(fx, y + 1, 1, 5, (224, 232, 240));
        pb.rect_fill(fx + 1, y + 2, 1, 4, steel_lo);
        pb.plot(fx, y + 6, (150, 198, 218)); // tiny droplet sheen

        // Maple butcher chopping board + cleaver
        let bw_b = 17;
        let bh_b = (top_h - 4).clamp(7, 10);
        let bx = if sink_left { x + w - 5 - bw_b } else { x + 5 };
        let by = top_y0 + 2;
        for dx in 1..=bw_b {
            darken_px(pb, bx + dx, by + bh_b, 0.62);
        }
        for dy in 1..bh_b {
            darken_px(pb, bx + bw_b, by + dy, 0.62);
        }
        for dy in 0..bh_b {
            for dx in 0..bw_b {
                let grain = if dy % 2 == 0 { 1.03 } else { 0.96 };
                pb.plot(bx + dx, by + dy, palette::scale((180, 138, 86), grain));
            }
        }
        bevel_frame(pb, bx, by, bw_b, bh_b, (214, 174, 118), (110, 78, 42));
        // Butcher's cleaver with cast shadow & hanging hole
        pb.rect_fill(bx + 4, by + 5, 7, 1, (118, 86, 48)); // shadow
        pb.rect_fill(bx + 4, by + 2, 6, 3, (192, 200, 210)); // blade
        pb.rect_fill(bx + 4, by + 4, 6, 1, (124, 132, 144)); // bevel edge
        pb.plot(bx + 5, by + 2, (234, 240, 246)); // highlight
        pb.plot(bx + 4, by + 2, (112, 82, 46)); // hanging hole
        pb.rect_fill(bx + 10, by + 3, 4, 1, (66, 44, 28)); // wood grip

        // Center recessed steel gastro prep pan with side lip handles
        let mx = x + w / 2;
        let my = top_y0 + top_h / 2;
        pb.rect_fill(mx - 7, my - 1, 1, 3, steel_hi); // left ear handle
        pb.rect_fill(mx + 6, my - 1, 1, 3, steel_lo); // right ear handle
        pb.rect_fill(mx - 6, my - 3, 12, 7, (114, 124, 136));
        bevel_frame(pb, mx - 6, my - 3, 12, 7, (184, 192, 204), (62, 68, 78));
        pb.rect_fill(mx - 4, my - 1, 8, 3, (86, 94, 106));
    }
}

// ---- 10. Console (46 x 18) -------------------------------------------------

fn console(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    // Brighter brushed cool lab-steel chassis so it reads crisply off the dark lab floor
    let chassis = (68, 78, 100);
    let chassis_hi = (122, 138, 168);
    let chassis_front = (46, 54, 72);
    let outline = ambient_shade((16, 20, 30), pal, 0.25);

    let front_h = 4.min(h / 4);
    let top_h = (h - front_h).max(8);

    // 1. Slanted terminal chassis + louvered front panel
    for ly in 0..h {
        let py = y + ly;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            if ly >= top_h {
                let shade = if ly == top_h { 0.88 } else { 0.68 };
                let g = 1.10 - 0.26 * (lx as f32 / w.max(1) as f32);
                let mut c = palette::scale(chassis_front, shade * g);
                if ly == top_h + 1 && lx >= 4 && lx < w - 4 && (lx % 2 == 0) {
                    c = (20, 26, 38);
                }
                pb.plot(px, py, c);
                continue;
            }
            let g = 1.14
                - 0.26
                    * (0.55 * (lx as f32 / w.max(1) as f32)
                        + 0.45 * (ly as f32 / top_h.max(1) as f32));
            let mut c = palette::scale(chassis, g);
            if ly == 1 || lx == 1 {
                c = palette::mix(c, chassis_hi, 0.65);
            } else if ly == top_h - 1 || lx == w - 2 {
                c = palette::scale(c, 0.68);
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Primary wide oscilloscope CRT monitor
    let scr_accent = palette::scale(pal.accent, 0.88);
    let scr_bg = palette::mix((12, 24, 36), pal.accent, 0.09);
    let sw1 = (w / 2).clamp(12, 24);
    let sh1 = (top_h - 6).clamp(5, 8);
    let sx1 = x + 3;
    let sy1 = y + 2;

    bevel_frame(
        pb,
        sx1 - 1,
        sy1 - 1,
        sw1 + 2,
        sh1 + 2,
        (24, 28, 40),
        (94, 108, 132),
    );
    for dy in 0..sh1 {
        for dx in 0..sw1 {
            let scan = if dy % 2 == 0 { 1.0 } else { 0.82 };
            let mut c = palette::scale(scr_bg, scan);
            // Faint calibration grid line across screen center
            if dy == sh1 / 2 && dx % 3 == 0 {
                c = palette::mix(c, scr_accent, 0.16);
            }
            // Diagonal CRT glass reflection glare
            if dx + dy == 2 || dx + dy == 3 {
                c = palette::mix(c, (176, 224, 244), 0.22);
            }
            pb.plot(sx1 + dx, sy1 + dy, c);
        }
    }
    // Crisp glowing waveform trace
    let phase = (seed & 7) as i32;
    for dx in 1..(sw1 - 1) {
        let wave = match (dx + phase) % 8 {
            0 => sh1 / 2,
            1 => (sh1 / 2 - 1).max(1),
            2 => (sh1 / 2 - 2).max(1),
            3 => sh1 / 2,
            4 => (sh1 / 2 + 1).min(sh1 - 2),
            5 => (sh1 / 2 + 2).min(sh1 - 2),
            6 => (sh1 / 2 + 1).min(sh1 - 2),
            _ => sh1 / 2,
        };
        pb.plot(sx1 + dx, sy1 + wave, scr_accent);
        if wave + 1 < sh1 {
            blend_px(pb, sx1 + dx, sy1 + wave + 1, scr_accent, 0.30);
        }
    }

    // 3. Secondary telemetry bar-chart monitor
    let sx2 = sx1 + sw1 + 3;
    let sw2 = (x + w - 4 - sx2).min(12);
    if sw2 >= 6 {
        bevel_frame(
            pb,
            sx2 - 1,
            sy1 - 1,
            sw2 + 2,
            sh1 + 2,
            (24, 28, 40),
            (94, 108, 132),
        );
        pb.rect_fill(sx2, sy1, sw2, sh1, scr_bg);
        let bars = [3, 6, 2, 5, 6, 3];
        let mut bx = 1;
        let mut bi = (seed as usize) % bars.len();
        while bx < sw2 - 1 {
            let bh_bar = bars[bi % bars.len()].min(sh1 - 1);
            for dy in 0..bh_bar {
                let col = if dy == bh_bar - 1 {
                    palette::scale(scr_accent, 1.18)
                } else {
                    palette::scale(scr_accent, 0.56)
                };
                pb.plot(sx2 + bx, sy1 + sh1 - 1 - dy, col);
            }
            bx += 2;
            bi += 1;
        }
    }

    // 4. Recessed keyboard deck + crisp 1px status LEDs
    let deck_y = sy1 + sh1 + 2;
    if deck_y + 1 < y + top_h {
        // Dark recessed tray under keys
        pb.rect_fill(sx1 - 1, deck_y, sw1 + 1, 2.min(y + top_h - deck_y), (28, 34, 48));
        for kx in (sx1..sx1 + sw1 - 1).step_by(2) {
            pb.plot(kx, deck_y, (124, 136, 158));
            if deck_y + 1 < y + top_h - 1 {
                pb.plot(kx, deck_y + 1, (96, 106, 128));
            }
        }
        // Subtle cyan monitor bounce on the keyboard
        for kx in sx1..(sx1 + sw1) {
            pb.plot_add(kx, deck_y, palette::scale(pal.accent, 0.09));
        }

        // 1px hardware LEDs
        if sx2 + 6 < x + w - 2 {
            let leds: [(i32, Rgb); 4] = [
                (0, (204, 96, 84)),
                (2, (206, 180, 88)),
                (4, (96, 192, 138)),
                (6, pal.accent),
            ];
            for (ofs, c) in leds {
                let lx = sx2 + ofs;
                pb.plot(lx, deck_y, c);
                pb.plot_add(lx, deck_y - 1, palette::scale(c, 0.14));
            }
        }
    }

    // Chassis screws & recessed vertical power/toggle strip on right bezel
    pb.plot(x + 2, y + 2, chassis_hi);
    if w >= 42 && top_h >= 12 {
        let px_s = x + w - 3;
        pb.rect_fill(px_s, y + 2, 1, 7, (24, 28, 40));
        pb.plot(px_s, y + 3, pal.accent);
        pb.plot(px_s, y + 5, (148, 162, 186));
        pb.plot(px_s, y + 7, (186, 152, 74));
    } else {
        pb.plot(x + w - 3, y + 2, (110, 186, 212));
    }
}

// ---- 11. Tank (24 x 24) ----------------------------------------------------

fn tank(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    let cx = x as f32 + w as f32 * 0.5;
    let rx = (w as f32 * 0.39).max(5.5);

    // Pedestal anchors the lower 3/4 footprint (y + base_y0 .. y + h) so the
    // domed glass cylinder rises cleanly above the back rim with zero corner clutter!
    let base_y0 = 6.min(h / 3);
    shadow_rect(pb, x, y + base_y0, w, (h - base_y0).max(1));

    let metal = (74, 86, 108);
    let metal_hi = (152, 170, 198);
    let metal_lo = (34, 40, 56);
    let outline = ambient_shade((14, 18, 28), pal, 0.25);

    // 1. Heavy bolted octagonal steel pedestal base slab (y + base_y0 .. y + h)
    let front_h = 4.min(h / 5);
    let top_h = (h - front_h).max(base_y0 + 4);
    for ly in base_y0..h {
        let py = y + ly;
        let local_y = ly - base_y0;
        let base_h = h - base_y0;
        for lx in 0..w {
            let px = x + lx;
            let corner_dist = lx.min(w - 1 - lx) + local_y.min(base_h - 1 - local_y);
            if corner_dist < 2 {
                darken_px(pb, px, py, 0.62);
                continue;
            }
            if corner_dist == 2 || lx == 0 || lx == w - 1 || local_y == 0 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            if ly >= top_h {
                let s = if ly == top_h { 0.86 } else { 0.56 };
                let g = 1.12 - 0.30 * (lx as f32 / w.max(1) as f32);
                let mut c = palette::scale(metal, s * g);
                // Subtle recessed maintenance vent slot on center of front pedestal face
                if ly == top_h + 1 && (lx - w / 2).abs() <= 3 {
                    c = (24, 30, 42);
                }
                pb.plot(px, py, c);
                continue;
            }
            let g = 1.16
                - 0.36
                    * (0.55 * (lx as f32 / w.max(1) as f32)
                        + 0.45 * (local_y as f32 / (top_h - base_y0).max(1) as f32));
            let mut c = palette::scale(metal, g);
            if local_y == 1 || lx == 1 {
                c = palette::mix(c, metal_hi, 0.55);
            } else if ly == top_h - 1 || lx == w - 2 {
                c = palette::scale(c, 0.62);
            }
            pb.plot(px, py, c);
        }
    }
    // Pedestal mounting bolts on the visible corners
    let bhi = (168, 184, 210);
    let blo = (24, 30, 42);
    bolt(pb, x + 2, y + base_y0 + 1, bhi, blo);
    bolt(pb, x + w - 4, y + base_y0 + 1, palette::scale(bhi, 0.76), blo);
    bolt(pb, x + 2, y + top_h - 3, palette::scale(bhi, 0.80), blo);
    bolt(pb, x + w - 4, y + top_h - 3, palette::scale(bhi, 0.64), blo);

    // 2. Tall, gleaming cylindrical glass containment vat rising cleanly from the pedestal
    let cap_cy = y as f32 + 3.4;
    let cap_ry = 2.6;
    let collar_cy = y as f32 + (h as f32 * 0.76);
    let collar_ry = 2.8;
    let meniscus_y = cap_cy + cap_ry + 1.2;

    // Cast shadow from tall cylinder onto right/bottom of pedestal & floor
    shadow_ellipse(pb, cx + 2.4, collar_cy + 0.8, rx + 1.2, collar_ry + 1.0, 0.48);

    let liq_deep = palette::mix((18, 64, 90), pal.accent, 0.24);
    let liq_mid = palette::mix((46, 140, 170), pal.accent, 0.46);
    let liq_hi = palette::mix((124, 218, 240), pal.accent, 0.58);
    let glass_spec = (220, 246, 255);

    let y_start = cap_cy.floor() as i32;
    let y_end = (collar_cy + collar_ry + 1.0).ceil() as i32;

    for py in y_start..=y_end {
        let fy = py as f32 + 0.5;
        let is_collar = fy >= collar_cy - 1.0;
        for lx in 0..w {
            let px = x + lx;
            let dx = px as f32 + 0.5 - cx;
            let nx = dx / rx;
            if nx.abs() > 1.06 {
                continue;
            }

            if is_collar {
                let ny = (fy - collar_cy) / collar_ry;
                if ny > 0.0 && nx * nx + ny * ny > 1.04 {
                    continue;
                }
                if nx.abs() > 0.92 || (ny > 0.0 && nx * nx + ny * ny > 0.80) {
                    pb.plot(px, py, outline);
                    continue;
                }
                let cyl = (1.0 - ((nx + 0.36) * 0.90).abs()).clamp(0.0, 1.0);
                let mut mc = palette::scale(metal, 0.64 + 0.98 * cyl);
                if (fy - (collar_cy - 0.8)).abs() < 0.7 {
                    mc = palette::mix(mc, metal_hi, 0.55);
                }
                pb.plot(px, py, mc);
                continue;
            }

            if fy < cap_cy + cap_ry * 0.55 {
                continue;
            }

            if nx.abs() > 0.92 {
                pb.plot(px, py, outline);
                continue;
            }

            let arc_drop = (1.0 - nx * nx).max(0.0).sqrt() * 1.45;
            let surf_y = meniscus_y + arc_drop;

            if fy < surf_y - 0.55 {
                let cyl = (1.0 - ((nx + 0.36) * 0.92).abs()).clamp(0.0, 1.0);
                let mut air = palette::scale((36, 58, 76), 0.62 + 0.64 * cyl);
                if (nx + 0.56).abs() < 0.14 {
                    air = palette::mix(air, glass_spec, 0.65);
                }
                pb.plot(px, py, air);
            } else if (fy - surf_y).abs() <= 0.68 {
                let c = palette::mix(liq_hi, glass_spec, 0.50);
                pb.plot(px, py, c);
            } else {
                let cyl = (1.0 - ((nx + 0.32) * 0.94).abs()).clamp(0.0, 1.0);
                let depth = ((fy - surf_y) / (collar_cy - surf_y).max(1.0)).clamp(0.0, 1.0);
                let mut c = palette::mix(liq_mid, liq_deep, depth * 0.40 + (1.0 - cyl) * 0.54);

                // Suspended internal specimen core / coil silhouette floating in center
                let core_r = 0.24 + 0.06 * (depth * std::f32::consts::PI).sin();
                if nx.abs() < core_r && depth > 0.16 && depth < 0.86 {
                    c = palette::scale(c, 0.72);
                } else if nx > -0.45 && nx < 0.28 {
                    c = palette::mix(c, liq_hi, (1.0 - depth * 0.42) * 0.34);
                }

                // Bold curved glass specular reflection on left + subtle rim glint on right
                if (nx + 0.56).abs() < 0.14 {
                    c = palette::mix(c, glass_spec, 0.74);
                } else if (nx - 0.68).abs() < 0.08 {
                    c = palette::mix(c, glass_spec, 0.34);
                }
                pb.plot(px, py, c);
            }
        }
    }

    // 3. Crisp rising bubbles suspended in the fluid
    let b_shift = (seed % 3) as i32 - 1;
    let bubbles = [
        (cx as i32 - 3 + b_shift, (meniscus_y + 3.0) as i32, true),
        (cx as i32 + 2 - b_shift, (meniscus_y + 5.4) as i32, true),
        (cx as i32 - 1, (meniscus_y + 7.8) as i32, false),
        (cx as i32 + 3, (meniscus_y + 9.2) as i32, false),
    ];
    for (bx, by, big) in bubbles {
        if by < collar_cy as i32 - 1 {
            if big {
                pb.plot(bx, by, glass_spec);
                pb.plot(bx + 1, by, liq_hi);
                pb.plot(bx, by + 1, liq_hi);
                pb.plot(bx + 1, by + 1, liq_deep);
            } else {
                pb.plot(bx, by, glass_spec);
                pb.plot(bx, by + 1, liq_deep);
            }
        }
    }

    // 4. Twin vertical metal containment struts flanking the glass
    let rib_l = (cx - rx + 0.6).round() as i32;
    let rib_r = (cx + rx - 0.6).round() as i32;
    let rib_y0 = (cap_cy + cap_ry * 0.55).round() as i32;
    let rib_y1 = (collar_cy - 1.0).round() as i32;
    for py in rib_y0..=rib_y1 {
        pb.plot(rib_l, py, metal_hi);
        pb.plot(rib_r, py, metal_lo);
    }

    // 5. Sleek gleaming brushed-metal pressure dome top cap (clean silhouette!)
    for ly in 0..h {
        let py = y + ly;
        let fy = py as f32 + 0.5;
        let ny = (fy - cap_cy) / cap_ry;
        if ny.abs() > 1.24 {
            continue;
        }
        for lx in 0..w {
            let px = x + lx;
            let nx = (px as f32 + 0.5 - cx) / (rx + 0.6);
            let d = (nx * nx + ny * ny).sqrt();
            if d > 1.08 {
                continue;
            }
            if d > 0.90 {
                pb.plot(px, py, outline);
                continue;
            }
            let dome = (-nx * 0.58 - ny * 0.68).clamp(-1.0, 1.0);
            let mut c = if dome > 0.0 {
                palette::mix(metal, (196, 214, 236), dome * 0.88)
            } else {
                palette::mix(metal, metal_lo, (-dome) * 0.78)
            };
            if (d - 0.54).abs() < 0.14 {
                c = palette::scale(c, 0.72);
            }
            pb.plot(px, py, c);
        }
    }
    let gx = cx.round() as i32;
    let gy = cap_cy.round() as i32;
    pb.plot(gx, gy, palette::scale(pal.accent, 0.92));
    pb.plot(gx - 1, gy - 1, (224, 238, 252));

    // Faint caustic glow along front collar rim
    let cy_c = (collar_cy + collar_ry) as i32;
    pb.plot_add(gx - 2, cy_c, palette::scale(pal.accent, 0.10));
    pb.plot_add(gx, cy_c, palette::scale(pal.accent, 0.14));
    pb.plot_add(gx + 2, cy_c, palette::scale(pal.accent, 0.09));
}

// ---- 12. Laundry props: washer, towel roll, clothespin ----------------------

/// A paired commercial front-loading washer bank (58 × 30 px): two porcelain-white
/// enamel machines side-by-side under a brushed control console, with twin
/// chrome-bezeled glass porthole doors showing glowing aqua wash water and
/// ruined-pink laundry tumbling inside.
fn washer(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 1, y, (w - 2).max(1), h);

    let enamel_hi: Rgb = (224, 234, 232);
    let enamel: Rgb = (182, 198, 198);
    let enamel_lo: Rgb = (118, 136, 142);
    let deck: Rgb = (146, 166, 172);
    let chrome_hi: Rgb = (238, 246, 246);
    let chrome: Rgb = (148, 166, 174);
    let chrome_lo: Rgb = (74, 88, 100);
    let brass: Rgb = (194, 158, 82);
    let water_deep: Rgb = (20, 42, 62);
    let water_mid: Rgb = (48, 108, 128);
    let water_hi: Rgb = (132, 206, 212);
    let pink_wash: Rgb = (218, 130, 166);
    let outline = ambient_shade((14, 18, 28), pal, 0.28);

    // 3/4 split: raised control backsplash + top lid (`0..top_h`) over tall
    // angled front enamel fascia (`top_h..h`) housing twin circular portholes!
    let top_h = (h / 3).clamp(7, 11);
    let mid_x = x + w / 2;

    // 1. Top lid & rear control console backsplash (`ly in 0..top_h`).
    for ly in 0..top_h {
        let py = y + ly;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            if px == mid_x {
                // Crisp vertical seam between the twin commercial washers.
                pb.plot(px, py, if ly == 1 { chrome_lo } else { outline });
                continue;
            }
            let grad = 1.08 - 0.18 * (lx as f32 / w.max(1) as f32);
            let mut c = if ly <= 3 {
                // Raised stainless/denim control strip along the back edge.
                let base = if ly == 1 { enamel_hi } else { deck };
                palette::scale(base, grad)
            } else if ly == 4 {
                // Shadow groove under the control console lip.
                chrome_lo
            } else if ly == top_h - 1 {
                // Crisp sunlit front coping edge of the washer lid.
                enamel_hi
            } else {
                palette::scale(enamel, grad * (1.03 - 0.08 * (ly - 4) as f32 / (top_h - 4).max(1) as f32))
            };
            if lx == 1 {
                c = palette::mix(c, enamel_hi, 0.55);
            }
            pb.plot(px, py, c);
        }
    }

    // Control console details for each of the two bays: detergent drawer,
    // rotary brass cycle dial, and glowing status LEDs.
    for bay in 0..2 {
        let bx = if bay == 0 { x + 2 } else { mid_x + 2 };
        let bw = (w / 2 - 4).max(8);
        // Recessed detergent dispenser drawer on left of each bay.
        pb.rect_fill(bx + 1, y + 2, 6.min(bw / 3), 2, chrome_lo);
        pb.rect_fill(bx + 2, y + 2, 4.min(bw / 3), 1, chrome_hi);
        // Rotary cycle dial + glowing aqua/amber pilot LEDs on right of bay.
        let dx = bx + bw - 8;
        if dx > bx + 5 {
            pb.rect_fill(dx, y + 1, 3, 3, outline);
            pb.plot(dx + 1, y + 2, brass);
            pb.plot(dx, y + 1, chrome_hi);
            // Status LEDs (aqua wash light + warm amber spin pip).
            pb.plot(dx + 5, y + 2, if (seed as i32 + bay) & 1 == 0 { pal.accent } else { (246, 198, 96) });
            pb.plot(dx + 7, y + 2, (118, 226, 218));
        }
    }

    // 2. Angled front porcelain enamel cabinet fascia (`ly in top_h..h`).
    for ly in top_h..h {
        let py = y + ly;
        let dy = ly - top_h;
        let fh = (h - top_h).max(1);
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            if px == mid_x {
                pb.plot(px, py, outline);
                continue;
            }
            let grad = 1.04 - 0.20 * (lx as f32 / w.max(1) as f32);
            let mut c = if dy == 0 {
                // Cast shadow directly under top lid lip.
                palette::scale(enamel_lo, 0.72 * grad)
            } else if ly >= h - 4 {
                // Recessed dark kick-plate plinth with ventilation louver slots.
                let base = ambient_shade((48, 60, 74), pal, 0.2);
                if ly == h - 3 && (lx % 4) == 0 && (px - mid_x).abs() > 2 {
                    outline
                } else {
                    palette::scale(base, grad)
                }
            } else {
                palette::scale(palette::mix(enamel, enamel_lo, dy as f32 / fh as f32 * 0.45), grad)
            };
            if lx == 1 || px == mid_x + 1 {
                c = palette::mix(c, enamel_hi, 0.42);
            }
            pb.plot(px, py, c);
        }
    }

    // 3. Twin gleaming chrome-bezeled front-loading glass porthole doors!
    let front_h = (h - top_h - 3).max(8);
    let door_cy = y + top_h + front_h / 2;
    let door_r = ((front_h as f32 * 0.46).min(w as f32 * 0.16)).clamp(4.5, 7.5);
    let bay_centers = [x + w / 4, x + (w * 3) / 4];

    for (bay, &cx) in bay_centers.iter().enumerate() {
        let ir = door_r.ceil() as i32 + 1;
        let phase = ((seed as usize + bay * 3) & 3) as f32 * 1.1;
        for dy in -ir..=ir {
            for dx in -ir..=ir {
                let nx = dx as f32 / door_r;
                let ny = dy as f32 / (door_r * 0.92);
                let d = (nx * nx + ny * ny).sqrt();
                let px = cx + dx;
                let py = door_cy + dy;
                if d > 1.18 || py <= y + top_h || py >= y + h - 2 {
                    continue;
                }
                if d > 1.04 {
                    pb.plot(px, py, outline);
                } else if d > 0.80 {
                    // Beveled chrome porthole bezel lit top-left.
                    let lit = (-nx * 0.62 - ny * 0.78).clamp(-1.0, 1.0);
                    let col = if lit > 0.25 {
                        chrome_hi
                    } else if lit > -0.25 {
                        chrome
                    } else {
                        chrome_lo
                    };
                    pb.plot(px, py, col);
                } else if d > 0.72 {
                    pb.plot(px, py, outline);
                } else {
                    // Glass drum interior: aqua soapy water, white foam crest,
                    // and a swirl of ruined-pink / white laundry tumbling inside!
                    let swirl = (nx * 2.2 + phase).sin() * 0.22;
                    let water_line = 0.08 + swirl;
                    let mut col = if ny > water_line + 0.10 {
                        // Submerged soapy aqua water + tumbling laundry mass.
                        let lump = ((nx - 0.18 + bay as f32 * 0.25).powi(2) + (ny - 0.28).powi(2)).sqrt();
                        if lump < 0.36 {
                            if bay == 1 || (seed & 1) == 0 { pink_wash } else { enamel_hi }
                        } else {
                            palette::mix(water_mid, water_deep, (ny - water_line).clamp(0.0, 1.0) * 0.8)
                        }
                    } else if (ny - water_line).abs() <= 0.14 {
                        // Bright frothy soap meniscus.
                        water_hi
                    } else {
                        // Upper dark glass cavity with subtle aqua drum glow.
                        palette::mix(water_deep, water_mid, 0.26)
                    };
                    // Curved specular glass glare arc in upper-left quadrant.
                    if d > 0.44 && d < 0.64 && nx < -0.10 && ny < -0.05 {
                        col = palette::mix(col, (244, 252, 255), 0.72);
                    }
                    pb.plot(px, py, col);
                }
            }
        }
        // Chrome door latch tab on the right side of each porthole.
        let lx = cx + door_r.round() as i32;
        pb.plot(lx, door_cy - 1, chrome_hi);
        pb.plot(lx, door_cy, chrome_lo);
    }

    // 4. Heavy rubber anti-vibration feet at the corners & centre seam.
    let foot = ambient_shade((24, 28, 38), pal, 0.2);
    pb.rect_fill(x + 2, y + h - 2, 4, 2, foot);
    pb.rect_fill(mid_x - 2, y + h - 2, 4, 2, foot);
    pb.rect_fill(x + w - 6, y + h - 2, 4, 2, foot);
}

/// A plush rolled terrycloth towel standing on end (26 × 26 px): sculpted 3/4
/// cream-porcelain cylinder, faded-denim & ruined-pink woven stripe band, and a
/// deeply shaded spiral coil on its elliptical top cap.
fn towel_roll(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    let cx = x as f32 + w as f32 * 0.5;
    let rx = (w as f32 * 0.42).clamp(4.0, 11.5);
    let cap_ry = (rx * 0.48).clamp(2.8, 5.5);
    let cap_cy = y as f32 + cap_ry + 1.5;
    let bot_cy = y as f32 + h as f32 - cap_ry * 0.72 - 1.2;

    shadow_ellipse(pb, cx + 2.0, bot_cy + 1.6, rx + 1.4, cap_ry * 0.95, 0.54);

    let pink_towel = (seed & 3) == 1;
    let denim_towel = (seed & 3) == 2;
    let (cloth_hi, cloth_mid, cloth_lo, stripe_a, stripe_b) = if pink_towel {
        ((244, 214, 224), (212, 162, 182), (144, 96, 118), (236, 238, 232), (176, 108, 138))
    } else if denim_towel {
        ((178, 206, 216), (118, 152, 168), (66, 94, 112), (222, 230, 226), (206, 128, 160))
    } else {
        ((242, 244, 236), (202, 208, 202), (134, 144, 146), (82, 120, 140), (214, 128, 162))
    };
    let crease = palette::scale(cloth_lo, 0.58);
    let outline = ambient_shade((18, 22, 32), pal, 0.25);

    // 1. Sculpted vertical terrycloth cylinder body.
    let y0 = cap_cy.floor() as i32;
    let y1 = (bot_cy + cap_ry + 1.0).ceil() as i32;
    for py in y0..=y1 {
        let fy = py as f32 + 0.5;
        let vt = ((fy - cap_cy) / (bot_cy - cap_cy).max(1.0)).clamp(0.0, 1.3);
        let row_rx = if fy <= bot_cy {
            rx * (1.0 + 0.03 * (vt * std::f32::consts::PI).sin())
        } else {
            let bny = (fy - bot_cy) / (cap_ry * 0.72);
            if bny >= 1.0 {
                continue;
            }
            rx * (1.0 - bny * bny).sqrt()
        };
        for lx in 0..w {
            let px = x + lx;
            let dx = px as f32 + 0.5 - cx;
            if dx.abs() > row_rx + 0.4 {
                continue;
            }
            let on_bot_rim = fy > bot_cy && {
                let nx = dx / rx;
                let ny = (fy - bot_cy) / (cap_ry * 0.72);
                nx * nx + ny * ny > 0.78
            };
            if dx.abs() > row_rx - 0.85 || on_bot_rim {
                pb.plot(px, py, outline);
                continue;
            }
            let nx = (dx / row_rx.max(1.0)).clamp(-1.0, 1.0);
            let cyl = (1.0 - ((nx + 0.36) * 0.90).abs()).clamp(0.0, 1.0);
            let arc_drop = (1.0 - nx * nx).max(0.0).sqrt() * (cap_ry * 0.65);
            let band_y = cap_cy + (bot_cy - cap_cy) * 0.46 + arc_drop;
            let dist_band = (fy - band_y).abs();

            // Base woven terrycloth colour or decorative woven dobby band.
            let base = if dist_band < 1.15 {
                stripe_b
            } else if dist_band < 2.25 {
                stripe_a
            } else if cyl > 0.62 {
                palette::mix(cloth_mid, cloth_hi, (cyl - 0.62) / 0.38)
            } else {
                palette::mix(cloth_lo, cloth_mid, cyl / 0.62)
            };
            let mut c = palette::scale(base, 0.72 + 0.38 * cyl - 0.12 * vt.min(1.0));
            // Subtle terrycloth loop texture + tucked vertical fold hem on right.
            if ((px + py * 2) & 3) == 0 {
                c = palette::scale(c, 0.92);
            }
            if (nx - 0.44).abs() < 0.11 {
                c = palette::mix(c, crease, 0.62);
            } else if (nx - 0.32).abs() < 0.09 {
                c = palette::mix(c, cloth_hi, 0.32);
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Elliptical top cap with clear Archimedean spiral coil!
    let c_y0 = (cap_cy - cap_ry - 1.5).floor() as i32;
    let c_y1 = (cap_cy + cap_ry + 1.5).ceil() as i32;
    for py in c_y0..=c_y1 {
        let fy = py as f32 + 0.5;
        let ny = (fy - cap_cy) / cap_ry;
        for lx in 0..w {
            let px = x + lx;
            let nx = (px as f32 + 0.5 - cx) / rx;
            let d = (nx * nx + ny * ny).sqrt();
            if d > 1.06 {
                continue;
            }
            if d > 0.88 {
                pb.plot(px, py, outline);
                continue;
            }
            let ang = ny.atan2(nx); // -PI..PI
            // 2.5 turns of spiral groove across the cap!
            let turns = d * 2.7 - (ang / std::f32::consts::TAU);
            let groove = turns.fract();
            let lit = (0.55 - 0.35 * nx - 0.30 * ny).clamp(0.0, 1.0);
            let c = if d < 0.16 {
                crease
            } else if groove < 0.24 {
                palette::mix(crease, cloth_lo, 0.35)
            } else if groove < 0.52 {
                palette::mix(cloth_mid, cloth_hi, 0.45 + 0.50 * lit)
            } else {
                palette::mix(cloth_lo, cloth_mid, 0.35 + 0.55 * lit)
            };
            pb.plot(px, py, c);
        }
    }
}

/// A giant wooden spring clothespin (16 × 34 px): two sculpted beechwood legs
/// with curved top finger-grips, deep central pinch slot, gleaming coiled steel
/// wire torsion spring around the waist, and tapered clamping jaws gripping a
/// scrap of ruined-pink / denim cloth.
fn clothespin(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 2, y + 1, (w - 4).max(1), (h - 1).max(1));

    let wood_hi: Rgb = (232, 196, 138);
    let wood_mid: Rgb = (188, 146, 92);
    let wood_lo: Rgb = (122, 88, 50);
    let grain_col: Rgb = (146, 106, 62);
    let spring_hi: Rgb = (236, 244, 250);
    let spring_mid: Rgb = (146, 160, 174);
    let spring_lo: Rgb = (68, 78, 92);
    let outline = ambient_shade((24, 16, 10), pal, 0.25);

    let cx = x as f32 + w as f32 * 0.5;
    let spring_y = y + (h * 56) / 100;

    // 1. Sculpted twin beechwood peg halves (wider top finger grips, pinched
    //    spring waist, flared clamp jaws tapering cleanly at the bottom).
    for ly in 0..h {
        let py = y + ly;
        let t = ly as f32 / (h - 1).max(1) as f32;
        // Authentic clothespin contour profile along height `t in 0..1`.
        let half_w = if t < 0.14 {
            // Rounded top finger-grip tips.
            let u = (0.14 - t) / 0.14;
            w as f32 * (0.43 - 0.10 * u * u)
        } else if t < 0.54 {
            // Concave finger-grip waist narrowing gently toward the spring.
            let u = (t - 0.14) / 0.40;
            w as f32 * (0.43 - 0.09 * (u * std::f32::consts::PI).sin())
        } else {
            // Lower clamping jaws tapering smoothly to the tip.
            let u = (t - 0.54) / 0.46;
            w as f32 * (0.41 - 0.13 * u)
        };

        // Central split gap widens subtly into a diamond notch between the jaws.
        let gap_half = if t < 0.06 {
            0.0
        } else if (t - 0.74).abs() < 0.08 {
            1.25
        } else if t < 0.94 {
            0.55
        } else {
            0.0
        };

        for lx in 0..w {
            let px = x + lx;
            let dx = px as f32 + 0.5 - cx;
            let adx = dx.abs();
            if adx > half_w + 0.4 {
                continue;
            }
            if ly == 0 || ly == h - 1 || adx > half_w - 0.95 {
                pb.plot(px, py, outline);
                continue;
            }
            if adx <= gap_half {
                // Dark central pinch gap (with a tiny cloth scrap gripped in the jaw notch).
                let col = if (t - 0.74).abs() < 0.06 && adx < 0.9 {
                    if (seed & 1) == 0 { pal.blood } else { pal.wall }
                } else {
                    outline
                };
                pb.plot(px, py, col);
                continue;
            }
            // Directional lighting across each wood half + subtle vertical grain.
            let side_t = ((dx + half_w) / (2.0 * half_w).max(1.0)).clamp(0.0, 1.0);
            let mut c = if side_t < 0.30 {
                palette::mix(wood_hi, wood_mid, side_t / 0.30)
            } else {
                palette::mix(wood_mid, wood_lo, (side_t - 0.30) / 0.70)
            };
            // Top bevel & inner split highlight on the right jaw edge.
            if ly == 1 || (dx > 0.0 && adx < gap_half + 1.1) {
                c = palette::mix(c, wood_hi, 0.42);
            }
            if ((px * 3 + (seed as i32 & 7)) % 5) == 0 && (py & 1) == 0 {
                c = palette::mix(c, grain_col, 0.30);
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Gleaming coiled steel wire torsion spring wrapped cleanly around the waist!
    let sw = (w - 2).max(6);
    let sx0 = x + 1;
    // Cast shadow on wood directly below the spring coils.
    for lx in 1..(w - 1) {
        darken_px(pb, x + lx, spring_y + 4, 0.52);
        darken_px(pb, x + lx, spring_y + 5, 0.74);
    }
    // Protruding coiled steel spring ears on left & right.
    pb.rect_fill(sx0 - 1, spring_y, 2, 4, outline);
    pb.rect_fill(sx0 + sw - 1, spring_y, 2, 4, outline);
    pb.plot(sx0, spring_y + 1, spring_hi);
    pb.plot(sx0, spring_y + 2, spring_mid);
    pb.plot(sx0 + sw - 1, spring_y + 1, spring_mid);
    pb.plot(sx0 + sw - 1, spring_y + 2, spring_lo);

    // Three tightly wound horizontal steel wire bands across the waist.
    for dy in 0..4 {
        let py = spring_y + dy;
        for lx in 0..sw {
            let px = sx0 + lx;
            let lit = 1.0 - (lx as f32 / sw.max(1) as f32) * 0.55;
            let c = match dy {
                0 => palette::scale(spring_hi, 0.65 + 0.35 * lit),
                1 => palette::scale(spring_mid, 0.55 + 0.45 * lit),
                2 => palette::scale(spring_hi, 0.58 + 0.35 * lit),
                _ => spring_lo,
            };
            pb.plot(px, py, c);
        }
    }
    // Crisp top-left metallic glint across the steel coil.
    pb.plot(sx0 + 2, spring_y, (252, 255, 255));
    pb.plot(sx0 + 3, spring_y, spring_hi);
}

// ---- 13. Printworks props: press island, paper reel, type block ------------

/// An offset printing press island (74 × 46 px): heavy riveted charcoal cast-iron
/// bed, gleaming steel impression cylinder, glossy violet ink-fountain roller,
/// printed broadsheet web emerging onto the delivery tray, twin cyan registration
/// lamps, and a gleaming brass-bezeled **Press Clock** mounted over the press bed.
fn press_island(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let iron_hi: Rgb = (108, 112, 126);
    let iron: Rgb = (58, 62, 74);
    let iron_lo: Rgb = (32, 34, 44);
    let bed_dark: Rgb = (22, 24, 32);
    let steel_hi: Rgb = (236, 240, 246);
    let steel_mid: Rgb = (162, 168, 178);
    let steel_lo: Rgb = (84, 88, 98);
    let ink_hi: Rgb = (132, 98, 158);
    let ink_mid: Rgb = pal.blood; // #554064 dried ink
    let ink_lo: Rgb = (34, 22, 44);
    let paper_hi: Rgb = (224, 222, 208);
    let paper_mid: Rgb = (182, 180, 165);
    let paper_lo: Rgb = (132, 130, 120);
    let brass_hi: Rgb = (238, 206, 122);
    let brass_mid: Rgb = (186, 148, 72);
    let brass_lo: Rgb = (112, 86, 40);
    let cyan: Rgb = (96, 202, 208);
    let outline = ambient_shade((10, 11, 16), pal, 0.28);

    let front_h = (h / 3).clamp(10, 15);
    let top_h = (h - front_h).max(16);

    // 1. Cast-iron press bed chassis top (`ly in 0..top_h`).
    for ly in 0..top_h {
        let py = y + ly;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let on_frame = lx <= 3 || lx >= w - 4 || ly <= 2 || ly >= top_h - 3;
            let grad = 1.08 - 0.22 * (lx as f32 / w.max(1) as f32 + ly as f32 / top_h.max(1) as f32) * 0.5;
            let mut c = if on_frame {
                let mut f = palette::scale(iron, grad);
                if ly == 1 || lx == 1 {
                    f = palette::mix(f, iron_hi, 0.65);
                } else if ly == top_h - 1 || lx == w - 2 {
                    f = iron_lo;
                }
                f
            } else {
                // Recessed dark press bed interior with subtle machined rail grooves.
                let base = if (lx % 8) == 0 { iron_lo } else { bed_dark };
                palette::scale(base, 0.92 + 0.16 * grad)
            };
            // Bolted corner frame rivets.
            if (lx == 2 || lx == w - 3) && (ly == 2 || ly == top_h - 3) {
                c = iron_hi;
            }
            pb.plot(px, py, c);
        }
    }

    // 2. Full-width gleaming steel impression cylinder across the upper bed.
    let roll_y = y + 4;
    let roll_h = 8.min((top_h / 3).max(5));
    for dy in 0..roll_h {
        let ny = (dy as f32 + 0.5) / roll_h as f32 - 0.5;
        let py = roll_y + dy;
        for lx in 4..(w - 4) {
            let px = x + lx;
            let end_cap = lx <= 6 || lx >= w - 7;
            let c = if end_cap {
                // Brass cylinder journal bearings at left & right ends.
                if ny < -0.15 { brass_hi } else if ny < 0.25 { brass_mid } else { brass_lo }
            } else if ny < -0.26 {
                steel_hi
            } else if ny < -0.05 {
                palette::mix(steel_mid, steel_hi, 0.45)
            } else if ny < 0.22 {
                steel_mid
            } else {
                steel_lo
            };
            pb.plot(px, py, c);
        }
    }
    // Cast shadow beneath the impression cylinder onto the press bed.
    for lx in 4..(w - 4) {
        darken_px(pb, x + lx, roll_y + roll_h, 0.48);
        darken_px(pb, x + lx, roll_y + roll_h + 1, 0.72);
    }

    // 3. Glossy dried-ink fountain roller + trough on the left half of the bed.
    let ink_y = roll_y + roll_h + 3;
    let ink_w = (w / 3 - 4).max(12);
    let ink_h = 6.min((top_h - (ink_y - y) - 4).max(4));
    pb.rect_fill(x + 5, ink_y - 1, ink_w + 2, ink_h + 2, outline);
    for dy in 0..ink_h {
        let ny = (dy as f32 + 0.5) / ink_h as f32 - 0.5;
        for lx in 0..ink_w {
            let c = if ny < -0.18 && lx < ink_w - 3 {
                ink_hi
            } else if ny < 0.22 {
                ink_mid
            } else {
                ink_lo
            };
            pb.plot(x + 6 + lx, ink_y + dy, c);
        }
    }
    // Specular wet ink sheen bar.
    pb.rect_fill(x + 8, ink_y + 1, (ink_w / 2).max(4), 1, (186, 152, 212));

    // 4. Printed broadsheet newsprint web feeding out onto the right delivery tray!
    let web_x = x + w / 2 + 2;
    let web_y = roll_y + roll_h + 2;
    let web_w = (w - (web_x - x) - 6).max(14);
    let web_h = (top_h - (web_y - y) - 3).clamp(7, 13);
    pb.rect_fill(web_x + 1, web_y + web_h, web_w, 1, outline);
    pb.rect_fill(web_x, web_y, web_w, web_h, paper_mid);
    pb.rect_fill(web_x, web_y, web_w, 1, paper_hi);
    pb.rect_fill(web_x, web_y + web_h - 1, web_w, 1, paper_lo);
    // Printed newspaper masthead + two-column text lines + cyan registration bar!
    pb.rect_fill(web_x + 2, web_y + 2, (web_w - 4).max(4), 1, ink_lo);
    let col_w = ((web_w - 6) / 2).max(3);
    for line in 0..3 {
        let ly = web_y + 4 + line * 2;
        if ly < web_y + web_h - 1 {
            pb.rect_fill(web_x + 2, ly, col_w, 1, ink_mid);
            pb.rect_fill(web_x + 4 + col_w, ly, (col_w - 1).max(2), 1, ink_mid);
        }
    }
    pb.rect_fill(web_x + web_w - 5, web_y + web_h - 2, 3, 1, cyan);

    // 5. THE PRESS CLOCK over the press bed! Circular brass-bezeled clock dial
    //    mounted centrally on a cast-iron gantry right over the press bed.
    let clk_cx = x + w / 2 - 2;
    let clk_cy = y + top_h / 2 + 1;
    let clk_r = 6.2_f32;
    // Soft cast shadow behind clock casing.
    shadow_ellipse(pb, clk_cx as f32 + 1.5, clk_cy as f32 + 2.0, clk_r + 1.0, clk_r * 0.9, 0.55);
    let dial_hi: Rgb = (248, 242, 224);
    let dial_mid: Rgb = (224, 214, 190);
    let dial_sh: Rgb = (178, 168, 146);
    let hand: Rgb = (26, 22, 30);
    for dy in -7i32..=7 {
        for dx in -7i32..=7 {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d > clk_r + 1.0 {
                continue;
            }
            let px = clk_cx + dx;
            let py = clk_cy + dy;
            if d > clk_r {
                pb.plot(px, py, outline);
            } else if d > clk_r - 1.55 {
                // Gleaming beveled brass clock bezel.
                let lit = (-dx as f32 * 0.6 - dy as f32 * 0.8) / clk_r;
                let col = if lit > 0.25 {
                    brass_hi
                } else if lit > -0.25 {
                    brass_mid
                } else {
                    brass_lo
                };
                pb.plot(px, py, col);
            } else {
                // Warm cream-ivory illuminated clock face.
                let lit = (-dx as f32 * 0.5 - dy as f32 * 0.7) / clk_r;
                let col = if lit > 0.2 {
                    dial_hi
                } else if lit > -0.3 {
                    dial_mid
                } else {
                    dial_sh
                };
                pb.plot(px, py, col);
            }
        }
    }
    // 12 / 3 / 6 / 9 cardinal dial ticks + hands (seeded minute hand near midnight!).
    pb.plot(clk_cx, clk_cy - 4, hand);
    pb.plot(clk_cx, clk_cy + 4, hand);
    pb.plot(clk_cx - 4, clk_cy, hand);
    pb.plot(clk_cx + 4, clk_cy, hand);
    // Hour hand pointing toward ~11 o'clock & minute hand pointing toward ~12/1.
    pb.plot(clk_cx - 1, clk_cy - 1, hand);
    pb.plot(clk_cx - 2, clk_cy - 2, hand);
    let min_dx = if (seed & 1) == 0 { 1 } else { 0 };
    pb.plot(clk_cx + min_dx, clk_cy - 1, hand);
    pb.plot(clk_cx + min_dx, clk_cy - 2, hand);
    pb.plot(clk_cx + min_dx, clk_cy - 3, hand);
    pb.plot(clk_cx, clk_cy, brass_mid); // center brass pin

    // 6. Heavy charcoal cast-iron front apron (`ly in top_h..h`): drive housing,
    //    cooling louvers, brass builder's plate, and twin cyan registration lamps!
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let grad = 1.04 - 0.26 * (lx as f32 / w.max(1) as f32);
            let mut c = if dy == 0 {
                palette::scale(iron_lo, 0.72)
            } else {
                palette::scale(iron_lo, 0.96 * grad)
            };
            // Recessed center gearbox housing.
            if lx >= w / 3 && lx <= (w * 2) / 3 && dy >= 2 && dy < front_h - 2 {
                c = if lx == w / 3 || dy == 2 {
                    iron_hi
                } else if lx == (w * 2) / 3 || dy == front_h - 3 {
                    outline
                } else if dy % 2 == 0 && lx > w / 3 + 3 && lx < (w * 2) / 3 - 3 {
                    bed_dark
                } else {
                    palette::scale(iron, grad)
                };
            }
            pb.plot(px, py, c);
        }
    }
    // Bolted brass maker's nameplate on front left.
    pb.rect_fill(x + 5, y + top_h + 3, 9, 3, brass_lo);
    pb.rect_fill(x + 5, y + top_h + 3, 8, 2, brass_mid);
    pb.rect_fill(x + 6, y + top_h + 3, 6, 1, brass_hi);
    // Twin gleaming registration-cyan indicator lamps on front right!
    let lamp_y = y + top_h + 4;
    for &lx in &[x + w - 11, x + w - 7] {
        pb.rect_fill(lx - 1, lamp_y - 1, 3, 3, outline);
        pb.plot(lx, lamp_y, cyan);
        pb.plot(lx, lamp_y - 1, (190, 246, 250));
    }
}

/// A standing giant newsprint paper reel (26 × 34 px): thick wound concentric
/// newsprint roll on a bolted cast-iron A-frame spindle, with a loose broadsheet
/// ribbon unspooling down onto the floor.
fn paper_reel(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 2, y + 3, (w - 4).max(1), (h - 3).max(1));

    let paper_hi: Rgb = (242, 240, 228);
    let paper_mid: Rgb = (204, 200, 184);
    let paper_lo: Rgb = (146, 142, 130);
    let paper_ring: Rgb = (168, 164, 150);
    let core_col: Rgb = (98, 76, 52);
    let stand_hi: Rgb = (92, 96, 108);
    let stand_mid: Rgb = (52, 54, 64);
    let stand_lo: Rgb = (26, 28, 36);
    let ink: Rgb = pal.blood;
    let cyan: Rgb = (88, 184, 190);
    let outline = ambient_shade((12, 13, 18), pal, 0.28);

    let cx = x + w / 2;
    let r = (w as f32 * 0.45).clamp(5.0, 12.0);
    let cy = y + r.ceil() as i32 + 1;

    // 1. Cast-iron A-frame axle stand & bolted floor footplate behind/under the reel.
    let foot_y = y + h - 4;
    let leg_top = cy + 2;
    for py in leg_top..foot_y {
        let t = (py - leg_top) as f32 / (foot_y - leg_top).max(1) as f32;
        let spread = (3.0 + t * 5.0).round() as i32;
        pb.rect_fill(cx - spread - 1, py, 3, 1, outline);
        pb.plot(cx - spread, py, stand_hi);
        pb.rect_fill(cx + spread - 1, py, 3, 1, outline);
        pb.plot(cx + spread, py, stand_mid);
    }
    // Cross-brace & floor footplate.
    let mid_brace = (leg_top + foot_y) / 2 + 2;
    pb.rect_fill(cx - 5, mid_brace, 11, 2, stand_mid);
    pb.rect_fill(cx - 5, mid_brace, 11, 1, stand_hi);
    pb.rect_fill(cx - 8, foot_y, 17, 4, outline);
    pb.rect_fill(cx - 7, foot_y + 1, 15, 2, stand_mid);
    pb.rect_fill(cx - 7, foot_y + 1, 15, 1, stand_hi);
    pb.plot(cx - 6, foot_y + 1, paper_hi);
    pb.plot(cx + 6, foot_y + 1, stand_lo);

    // 2. Thick wound concentric newsprint roll disc with directional 3/4 shading.
    let ir = r.ceil() as i32 + 1;
    for dy in -ir..=ir {
        for dx in -ir..=ir {
            let d = ((dx * dx + dy * dy) as f32).sqrt() / r;
            if d > 1.08 {
                continue;
            }
            let px = cx + dx;
            let py = cy + dy;
            if d > 0.93 {
                pb.plot(px, py, outline);
                continue;
            }
            let lit = (0.55 - (dx as f32 / r) * 0.40 - (dy as f32 / r) * 0.45).clamp(0.0, 1.0);
            let col = if d < 0.16 {
                // Dark axle bore + metallic spindle pin.
                if dx == 0 && dy == 0 { stand_hi } else { stand_lo }
            } else if d < 0.28 {
                // Kraft cardboard core tube.
                palette::scale(core_col, 0.72 + 0.45 * lit)
            } else {
                // Concentric wound newsprint layers with subtle spiral seam shading.
                let ring = ((d * 10.5) as i32) & 1 == 0;
                let base = if ring { paper_mid } else { paper_ring };
                if lit > 0.62 {
                    palette::mix(base, paper_hi, (lit - 0.62) / 0.38 * 0.85)
                } else {
                    palette::mix(paper_lo, base, lit / 0.62)
                }
            };
            pb.plot(px, py, col);
        }
    }

    // 3. Loose printed newsprint ribbon peeling smoothly down off the bottom of the reel!
    let peel_h = (6 + (seed as i32 & 1)).min(h - (cy + r as i32 - y) - 1).max(3);
    let peel_y0 = cy + r as i32 - 2;
    let peel_w = 10;
    let peel_x0 = cx - peel_w / 2;
    for k in 0..peel_h {
        let py = peel_y0 + k;
        let curl = k / 2;
        pb.plot(peel_x0 + curl - 1, py, outline);
        pb.plot(peel_x0 + peel_w + curl, py, outline);
        for lx in 0..peel_w {
            let px = peel_x0 + curl + lx;
            let c = if k == 0 {
                paper_hi
            } else if k == peel_h - 1 {
                paper_lo
            } else if (k == 2 || k == 4) && (2..peel_w - 2).contains(&lx) {
                palette::mix(paper_mid, ink, 0.52)
            } else {
                paper_mid
            };
            pb.plot(px, py, c);
        }
    }
    // Tiny registration cyan corner mark on the peeling sheet.
    pb.plot(peel_x0 + peel_w - 2 + (peel_h / 2), peel_y0 + peel_h - 2, cyan);
}

/// 6 bold 7×7 reversed serif capital letters (`Я`, `Ǝ`, `И`, `ꟼ`, `Ƨ`, `ꓶ`)
/// for giant cast-lead display type blocks. Bit 6 is leftmost pixel.
const BIG_REVERSED_SORTS: [[u8; 7]; 6] = [
    // 0: mirrored R (Я)
    [0b0111111, 0b1100011, 0b1100011, 0b0111111, 0b0011011, 0b0110011, 0b1100011],
    // 1: mirrored E (Ǝ)
    [0b1111111, 0b0000011, 0b0000011, 0b0111111, 0b0000011, 0b0000011, 0b1111111],
    // 2: mirrored N (И)
    [0b1100011, 0b1100111, 0b1101111, 0b1111011, 0b1110011, 0b1100011, 0b1100011],
    // 3: mirrored P (ꟼ)
    [0b0111111, 0b1100011, 0b1100011, 0b0111111, 0b0000011, 0b0000011, 0b0000011],
    // 4: mirrored S (Ƨ)
    [0b0111110, 0b0000011, 0b0000011, 0b0111110, 0b1100000, 0b1100000, 0b0111111],
    // 5: mirrored L (ꓶ)
    [0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b1111111],
];

/// A giant loose cast-lead letterpress display sort (30 × 30 px): beveled
/// metallic shoulder, dark ink-stained type bed, a bold **reversed serif
/// capital letter** standing in gleaming 3D relief, and the authentic
/// printer's nick & registration-cyan paint dab on the front face.
fn type_block(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);

    let lead_hi: Rgb = (196, 200, 210);
    let lead_mid: Rgb = (124, 128, 140);
    let lead_lo: Rgb = (64, 68, 80);
    let bed_ink: Rgb = (42, 34, 54);
    let face_ink: Rgb = (74, 54, 94);
    let face_hi: Rgb = (224, 224, 216);
    let cyan: Rgb = (90, 194, 198);
    let outline = ambient_shade((10, 11, 16), pal, 0.28);

    let front_h = (h / 4).clamp(6, 9);
    let top_h = (h - front_h).max(14);

    // 1. Chamfered metallic lead shoulder & recessed inked type bed (`0..top_h`).
    for ly in 0..top_h {
        let py = y + ly;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let grad = 1.10 - 0.24 * (lx as f32 / w.max(1) as f32 + ly as f32 / top_h.max(1) as f32) * 0.5;
            let on_rim = lx <= 2 || lx >= w - 3 || ly <= 2 || ly >= top_h - 2;
            let c = if ly == 1 || lx == 1 {
                lead_hi
            } else if ly == top_h - 1 || lx == w - 2 {
                lead_lo
            } else if on_rim {
                palette::scale(lead_mid, grad)
            } else {
                // Dark ink-stained metal counter-bed around the raised sort.
                palette::scale(bed_ink, 0.90 + 0.22 * grad)
            };
            pb.plot(px, py, c);
        }
    }

    // 2. Bold 2×-scaled reversed capital letterpress glyph standing proud in 3D relief!
    let glyph = BIG_REVERSED_SORTS[(seed % 6) as usize];
    let gw = 14;
    let gh = 14;
    let gx0 = x + (w - gw) / 2;
    let gy0 = y + (top_h - gh) / 2 - 1;

    // First pass: dark cast shadow & extruded lower-right metallic side walls.
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0i32..7 {
            if ((bits >> (6 - col)) & 1) == 0 {
                continue;
            }
            let bx = gx0 + col * 2;
            let by = gy0 + (row as i32) * 2;
            pb.rect_fill(bx + 1, by + 2, 2, 2, outline);
            pb.rect_fill(bx, by + 1, 2, 2, lead_lo);
        }
    }
    // Second pass: proud top letter face (gleaming brushed lead + dried-ink sheen).
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0i32..7 {
            if ((bits >> (6 - col)) & 1) == 0 {
                continue;
            }
            let bx = gx0 + col * 2;
            let by = gy0 + (row as i32) * 2;
            pb.rect_fill(bx, by, 2, 2, face_ink);
            // Crisp top-left metallic ink-brushed edge on every proud stroke.
            let top_free = row == 0 || ((glyph[row - 1] >> (6 - col)) & 1) == 0;
            let left_free = col == 0 || ((bits >> (6 - (col - 1))) & 1) == 0;
            if top_free {
                pb.rect_fill(bx, by, 2, 1, face_hi);
            }
            if left_free {
                pb.plot(bx, by, face_hi);
                pb.plot(bx, by + 1, lead_hi);
            }
        }
    }

    // 3. Heavy vertical lead front face with authentic semicircular printer's nick
    //    (used by typesetters to feel orientation by thumb) & cyan registration dab.
    let nick_y = y + top_h + front_h / 2;
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let grad = 1.06 - 0.28 * (lx as f32 / w.max(1) as f32);
            let mut c = if dy == 0 {
                palette::scale(lead_lo, 0.70)
            } else if py == nick_y && lx > 3 && lx < w - 4 {
                // Recessed horizontal printer's nick groove.
                outline
            } else if py == nick_y + 1 && lx > 3 && lx < w - 4 {
                lead_mid
            } else {
                palette::scale(lead_lo, grad)
            };
            if lx == 1 && dy > 0 {
                c = palette::mix(c, lead_mid, 0.48);
            }
            pb.plot(px, py, c);
        }
    }
    // Crisp registration-cyan ink dab on the bottom-left corner of the sort.
    pb.rect_fill(x + 3, y + h - 3, 3, 1, cyan);
    pb.plot(x + 3, y + h - 4, palette::mix(cyan, (230, 250, 252), 0.45));
}
