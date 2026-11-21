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

/// A front-loading washer: a porcelain-white cabinet with a dark porthole, a
/// detergent drawer and the little feet that make it read as a machine rather
/// than a crate. 58 × 30 px.
fn washer(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 1, y, (w - 2).max(1), h);

    let casing = (176, 190, 190);
    let casing_hi = (214, 226, 224);
    let casing_lo = (108, 124, 128);
    let chrome = (150, 164, 168);
    let chrome_hi = (226, 236, 236);
    let glass = ambient_shade((18, 24, 34), pal, 0.35);
    let outline = ambient_shade((16, 20, 26), pal, 0.30);

    let front_h = 7.min(h / 3);
    let top_h = (h - front_h).max(8);

    // Top face: porcelain with a soft sheen and the drum's dark recess.
    for ly in 0..top_h {
        for lx in 0..w {
            let px = x + lx;
            let py = y + ly;
            if lx == 0 || lx == w - 1 || ly == 0 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = 1.10 - 0.20 * (lx as f32 / w.max(1) as f32 + ly as f32 / top_h.max(1) as f32) * 0.5;
            let mut c = palette::scale(casing, g);
            if ly == 1 {
                c = casing_hi;
            } else if ly == top_h - 1 {
                c = casing_lo;
            }
            pb.plot(px, py, c);
        }
    }

    // The porthole on the top face, offset toward the left third.
    let cx = x + w / 3;
    let cy = y + top_h / 2;
    let r = (top_h.min(w) as f32 * 0.34).clamp(4.0, 9.0);
    for dy in -(r as i32) - 1..=(r as i32) + 1 {
        for dx in -(r as i32) - 1..=(r as i32) + 1 {
            let nx = dx as f32 / r;
            let ny = dy as f32 / r * 1.08;
            let d = (nx * nx + ny * ny).sqrt();
            let px = cx + dx;
            let py = cy + dy;
            if d > 1.16 {
                continue;
            }
            if d > 0.94 {
                // Chrome bezel
                pb.plot(px, py, if nx + ny < -0.4 { chrome_hi } else { chrome });
            } else if d > 0.34 {
                let sheen = (-nx * 0.6 - ny * 0.7).clamp(-1.0, 1.0);
                pb.plot(px, py, palette::mix(glass, chrome, 0.10 + sheen.max(0.0) * 0.30));
            } else {
                // The drum's throat and a hint of grey wash water.
                pb.plot(px, py, palette::mix(glass, pal.blood, 0.16));
            }
        }
    }
    // Handle pip and detergent drawer on the right of the top.
    pb.rect_fill(x + w - 12, y + 3, 7, 2, chrome);
    pb.plot(x + w - 12, y + 3, chrome_hi);
    pb.rect_fill(x + w - 5, y + 2, 2, 4, palette::scale(chrome, 0.72));

    // Front face: kick plate, soap drawer seam, a lint of hair in the corner.
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = if dy == 0 { 0.58 } else { 1.02 - 0.22 * (lx as f32 / w.max(1) as f32) };
            let mut c = palette::scale(casing_lo, g);
            if lp_dash(lx, seed) {
                c = palette::mix(c, casing_hi, 0.24);
            }
            pb.plot(px, py, c);
        }
    }
    // Feet.
    pb.rect_fill(x + 2, y + h - 2, 3, 2, ambient_shade((30, 34, 40), pal, 0.2));
    pb.rect_fill(x + w - 5, y + h - 2, 3, 2, ambient_shade((24, 28, 34), pal, 0.2));
}

/// A single narrow dash pattern so the front seam reads as a stamped vent.
#[inline]
fn lp_dash(lx: i32, seed: u32) -> bool {
    let phase = (seed as i32 & 3) + lx;
    phase.rem_euclid(6) < 2
}

/// A rolled towel standing on end: cream cylinder, spiral coil on the cap and a
/// coloured band in the laundry's own palette. 26 × 26 px.
fn towel_roll(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_ellipse(pb, x as f32 + w as f32 * 0.55, y as f32 + h as f32 * 0.94, w as f32 * 0.52, 4.0, 0.5);
    let cloth = (206, 208, 200);
    let cloth_lo = (150, 152, 148);
    let cloth_hi = (238, 238, 230);
    let band = palette::mix(cloth, pal.blood, 0.30);
    let outline = ambient_shade((18, 20, 26), pal, 0.3);
    let cx = x + w / 2;
    let rx = (w as f32 * 0.42).max(3.0);
    let cap_cy = y as f32 + rx * 0.9;
    let body_top = y + (rx * 0.9) as i32;
    let body_bot = y + h - 3;
    for py in body_top..=body_bot {
        let t = (py - body_top) as f32 / (body_bot - body_top).max(1) as f32;
        let rx_row = rx * (1.0 - 0.05 * t);
        for px in (cx - rx_row as i32)..=(cx + rx_row as i32) {
            let nx = (px as f32 + 0.5 - cx as f32) / rx_row.max(1.0);
            let shade = 1.0 - 0.34 * (nx * 0.6 + 0.5).max(0.0);
            let mut c = palette::scale(cloth, shade);
            // Rolled bands every third row.
            if (py - body_top).rem_euclid(4) == 0 {
                c = palette::mix(c, cloth_lo, 0.35);
            }
            if (py as f32 - (body_top as f32 + 6.0)).abs() < 1.5 {
                c = band;
            }
            pb.plot(px, py, c);
            if nx.abs() > 0.88 {
                pb.plot(px, py, outline);
            }
        }
    }
    // Spiral coil on the cap.
    for (i, (dx, dy)) in [(0, 0), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (2, 0), (2, 1), (2, 2), (1, 2), (0, 2)]
        .into_iter()
        .enumerate()
    {
        let c = if i % 2 == 0 { cloth_hi } else { cloth_lo };
        pb.plot(cx + dx, cap_cy as i32 + dy, c);
    }
    // A loose thread end, seeded so rolls in a bank differ.
    let tail = 1 + (seed as i32 & 3);
    for k in 0..tail {
        pb.plot(cx + 2 + k, body_bot - 1 - k % 2, cloth_hi);
    }
}

/// A giant wooden clothespin leaning in the corner: two tapered legs, a spring
/// band, and the split that makes it read as a peg. 16 × 34 px.
fn clothespin(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 1, y, (w - 2).max(1), h);
    let wood = (182, 148, 96);
    let wood_hi = (218, 186, 132);
    let wood_lo = (118, 92, 56);
    let spring = (140, 148, 158);
    let outline = ambient_shade((26, 18, 10), pal, 0.32);
    let split = w / 2;
    for ly in 0..h {
        let t = ly as f32 / h.max(1) as f32;
        // The peg is wider at the head and tapers to the pinch at the bottom.
        let inset = (t * (w as f32 * 0.22)) as i32;
        for lx in inset..(w - inset).max(inset + 1) {
            let px = x + lx;
            let py = y + ly;
            let g = 1.08 - 0.26 * (lx as f32 / w.max(1) as f32);
            let mut c = palette::scale(wood, g);
            if lx <= inset + 1 {
                c = wood_hi;
            } else if lx >= w - inset - 2 {
                c = wood_lo;
            }
            // The pinch gap between the two halves.
            if (lx - split).abs() <= 0 && ly > h / 5 && ly < h - h / 8 {
                c = outline;
            }
            pb.plot(px, py, c);
        }
    }
    // Spring band: two bright rings low on the peg.
    let ring_y = y + h * 3 / 5;
    pb.rect_fill(x + 2, ring_y, w - 4, 2, spring);
    pb.rect_fill(x + 2, ring_y + 3, w - 4, 1, palette::scale(spring, 0.7));
    pb.plot(x + 3, ring_y, (214, 222, 230));
    // A gum-ring of old starch at the head.
    let gum = mix_hint(wood, pal.blood, 0.22);
    pb.rect_fill(x + split - 1, y + 2, 2, 3, gum);
    let _ = seed;
}

#[inline]
fn mix_hint(a: Rgb, b: Rgb, t: f32) -> Rgb {
    palette::mix(a, b, t)
}

// ---- 13. Printworks props: press island, paper reel, type block ------------

/// An offset press island: a dark cast-iron bed with a bright cylinder roller,
/// an ink drum and a paper tongue running out of the nip. 74 × 46 px.
fn press_island(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);
    let iron = (58, 62, 72);
    let iron_hi = (104, 110, 122);
    let iron_lo = (30, 32, 40);
    let roller = (168, 174, 182);
    let roller_hi = (228, 232, 238);
    let ink = pal.blood;
    let paper = palette::mix((200, 198, 186), pal.bg_alt, 0.18);
    let outline = ambient_shade((10, 11, 16), pal, 0.3);
    let front_h = (h / 3).max(6);
    let top_h = h - front_h;

    // Bed top with a bolted frame.
    for ly in 0..top_h {
        for lx in 0..w {
            let px = x + lx;
            let py = y + ly;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = 1.06 - 0.22 * (ly as f32 / top_h.max(1) as f32);
            let mut c = palette::scale(iron, g);
            if lx % 9 == 0 {
                c = palette::mix(c, iron_lo, 0.5);
            }
            if lx % 9 == 4 && ly % 7 == 3 {
                c = iron_hi; // bolt heads
            }
            pb.plot(px, py, c);
        }
    }

    // The cylinder: a bright horizontal roller across the upper third.
    let roll_y = y + 3;
    let roll_h = (top_h / 3).max(3);
    for dy in 0..roll_h {
        for lx in 2..w - 2 {
            let ny = (dy as f32 + 0.5) / roll_h as f32 - 0.5;
            let c = if ny < -0.22 {
                roller_hi
            } else if ny < 0.2 {
                roller
            } else {
                palette::scale(roller, 0.62)
            };
            pb.plot(x + lx, roll_y + dy, c);
        }
    }

    // Ink drum on the left, paper tongue out of the nip to the right.
    let drum_cx = x + w / 5;
    let drum_cy = y + top_h / 2 + 2;
    for dy in -4i32..=4 {
        for dx in -4i32..=4 {
            if dx * dx + dy * dy <= 16 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let c = if d > 3.2 {
                    palette::scale(ink, 0.6)
                } else if dx + dy < -2 {
                    palette::mix(ink, (210, 210, 220), 0.18)
                } else {
                    ink
                };
                pb.plot(drum_cx + dx, drum_cy + dy, c);
            }
        }
    }
    let tongue_x = x + w / 2;
    let tongue_y = y + top_h - 5;
    for k in 0..(w / 3) {
        pb.plot(tongue_x + k, tongue_y + (k / 6), paper);
        pb.plot(tongue_x + k, tongue_y + 1 + (k / 6), palette::scale(paper, 0.82));
    }
    // A printed rule already on the tongue.
    for k in 0..(w / 4) {
        pb.plot(tongue_x + 2 + k, tongue_y, palette::scale(ink, 0.9));
    }

    // Front face: legs and the drive housing.
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = if dy == 0 { 0.55 } else { 0.92 - 0.22 * (lx as f32 / w.max(1) as f32) };
            let mut c = palette::scale(iron_lo, g);
            if lx > w / 3 && lx < w / 3 + 10 && dy > 1 {
                c = palette::scale(iron, g + 0.25);
            }
            pb.plot(px, py, c);
        }
    }
    // The little brass plate every machine in this building has.
    let plate = palette::mix(iron_hi, (198, 168, 92), 0.55);
    pb.rect_fill(x + 4, y + h - 4, 6, 2, plate);
    let _ = seed;
}

/// A standing paper reel: a pale disc of newsprint on a dark stand, with the
/// loose edge peeling down. 26 × 34 px.
fn paper_reel(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x + 1, y, (w - 2).max(1), h);
    let paper = palette::mix((206, 204, 190), pal.bg_alt, 0.10);
    let paper_lo = palette::scale(paper, 0.78);
    let paper_hi = (238, 236, 226);
    let stand = ambient_shade((42, 44, 52), pal, 0.25);
    let outline = ambient_shade((12, 13, 18), pal, 0.3);
    let cx = x + w / 2;
    let cy = y + w / 2 + 1;
    let r = (w as f32 * 0.46).max(4.0);
    for dy in -(r as i32) - 1..=(r as i32) + 1 {
        for dx in -(r as i32) - 1..=(r as i32) + 1 {
            let d = (((dx * dx + dy * dy) as f32).sqrt()) / r;
            if d > 1.06 {
                continue;
            }
            let c = if d > 0.92 {
                outline
            } else if d > 0.52 {
                let ring = ((d * 7.0) as i32) & 1 == 0;
                if ring {
                    paper
                } else {
                    paper_lo
                }
            } else if d > 0.18 {
                paper_hi
            } else {
                // The core's dark hole.
                palette::scale(stand, 0.6)
            };
            pb.plot(cx + dx, cy + dy, c);
        }
    }
    // The loose edge peels off the bottom of the reel and curls on the floor.
    let peel = 2 + (seed as i32 % 3);
    for k in 0..peel {
        let py = cy + r as i32 - 1 + k;
        let wide = 6 - k;
        for lx in 0..wide {
            pb.plot(cx - 3 + lx, py, if k == 0 { paper_hi } else { paper });
        }
    }
    // Stand legs.
    pb.rect_fill(cx - 1, cy + r as i32, 2, h - (cy + r as i32 - y), stand);
    pb.rect_fill(cx - 5, y + h - 3, 10, 2, stand);
    pb.plot(cx - 5, y + h - 3, palette::scale(stand, 1.5));
}

/// A loose piece of type: a lead block with a reversed capital standing proud of
/// its top face. 30 × 30 px.
fn type_block(pb: &mut PixelBuffer, x: i32, y: i32, w: i32, h: i32, seed: u32, pal: &Palette) {
    shadow_rect(pb, x, y, w, h);
    let lead = (112, 116, 126);
    let lead_hi = (168, 172, 182);
    let lead_lo = (56, 58, 68);
    let outline = ambient_shade((10, 11, 15), pal, 0.3);
    let front_h = (h / 4).max(4);
    let top_h = h - front_h;
    for ly in 0..top_h {
        for lx in 0..w {
            let px = x + lx;
            let py = y + ly;
            if lx == 0 || lx == w - 1 || ly == 0 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = 1.08 - 0.20 * (lx as f32 / w.max(1) as f32 + ly as f32 / top_h.max(1) as f32) * 0.5;
            let edge = lx == 1 || ly == 1;
            pb.plot(px, py, if edge { lead_hi } else { palette::scale(lead, g) });
        }
    }
    // The letter: mirrored, so it reads as type rather than paint.
    // Two of four seeded glyphs: an E and an L (flipped horizontally).
    let gx = x + 4;
    let gy = y + 3;
    let gw = (w - 8).max(5);
    let gh = (top_h - 6).max(5);
    let bar = (gw / 3).max(2);
    let glyph = seed % 2;
    let mirror = |lx: i32, glyph_w: i32| glyph_w - 1 - lx;
    for ly in 0..gh {
        for lx in 0..gw {
            let mlx = mirror(lx, gw);
            let on = if glyph == 0 {
                // E
                ly == 0
                    || ly == gh - 1
                    || ly == gh / 2
                    || mlx < bar
            } else {
                // L
                mlx < bar || ly == gh - 1
            };
            if on {
                pb.plot(gx + lx, gy + ly, lead_hi);
                pb.plot(gx + lx, gy + ly + 1, lead_lo);
            }
        }
    }
    // Front face with the printer's nick.
    for dy in 0..front_h {
        let py = y + top_h + dy;
        for lx in 0..w {
            let px = x + lx;
            if lx == 0 || lx == w - 1 || py == y + h - 1 {
                pb.plot(px, py, outline);
                continue;
            }
            let g = if dy == 0 { 0.55 } else { 0.9 - 0.2 * (lx as f32 / w.max(1) as f32) };
            pb.plot(px, py, palette::scale(lead_lo, g));
        }
    }
    pb.plot(x + 2, y + h - 2, pal.accent);
}
