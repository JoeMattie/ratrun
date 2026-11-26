//! Pixel art for the **Headlamp** pocket find: floor pickup sprite, worn lamp
//! strapped to the rat's head (standard and Outfall diver helmet, smoothly
//! scaled from `0.35×` Shrink Draught to `5×` Growth Serum), and the `8×8`
//! HUD/Library icon.
//!
//! Pure render-only drawing: zero simulation state, zero rng, and zero heap
//! allocation. Every function restores the caller's [`PixelBuffer`] material
//! pen before returning.

use crate::game::level::PropKind;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::{
    cloth_pen, prop_pen, rat_helmet_pen, rat_visor_pen, washer_glass_pen,
};
use crate::render::palette::{mix, scale as scale_rgb, Rgb};

/// Warm-white halogen/tungsten colour of the headlamp lens and its beam light.
pub const HEADLAMP_LENS_COLOR: Rgb = (255, 244, 206);

/// Non-creature emissive material byte matching `render::lighting::material::MaterialKind::Emissive`
/// (`14`, creature bit clear, height 0) so the glowing lens stays self-lit and
/// untouched by bump, shadows, specular overwrite, motes, and underwater grade.
const EMISSIVE_PEN: u8 = 14;

/// Top-left directional screen light matching `art_rat` and `art_rat_giant`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// --- Shared headlamp palette -----------------------------------------------
const OUTLINE: Rgb = (16, 18, 26);

// Elastic headband strap (matte charcoal-slate webbing).
const STRAP_LO: Rgb = (34, 38, 48);
const STRAP_MID: Rgb = (54, 60, 72);
const STRAP_HI: Rgb = (96, 104, 120);

// Gunmetal-black lamp casing (rear housing).
const CASE_DEEP: Rgb = (26, 30, 38);
const CASE_MID: Rgb = (54, 62, 74);
const CASE_HI: Rgb = (102, 112, 128);

// Miner's brass bezel & adjustment clasp (harmonises with the Outfall brass helmet).
const BRASS_DEEP: Rgb = (118, 72, 24);
const BRASS_MID: Rgb = (204, 146, 50);
const BRASS_HI: Rgb = (248, 212, 114);

// Lens highlights & dying-filament tone.
const LENS_CORE: Rgb = (255, 255, 250);
const LENS_RIM: Rgb = (252, 220, 136);
const LENS_DIM: Rgb = (152, 98, 42);
const SPILL_WARM: Rgb = (255, 232, 168);

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Darken a floor pixel directly without overwriting its underlying material
/// byte (so puddles, tiles, and sand keep their own lighting tags under the
/// pickup's contact shadow).
#[inline]
fn shade_floor(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let idx = y as usize * pb.w + x as usize;
    let cur = pb.pixels()[idx];
    pb.pixels_mut()[idx] = scale_rgb(cur, 1.0 - amount.clamp(0.0, 1.0));
}

/// Blend `c` over the pixel at `(x, y)` while stamping the active material pen.
#[inline]
fn blend_pen(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

/// Blend a soft warm light spill into the framebuffer without overwriting the
/// underlying floor/rat material tag.
#[inline]
fn add_spill(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, factor: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let f = factor.clamp(0.0, 1.0);
    if f <= 0.01 {
        return;
    }
    let idx = y as usize * pb.w + x as usize;
    let cur = pb.pixels()[idx];
    let add = scale_rgb(c, f);
    pb.pixels_mut()[idx] = (
        cur.0.saturating_add(add.0),
        cur.1.saturating_add(add.1),
        cur.2.saturating_add(add.2),
    );
}

/// Smooth, organic lens brightness in `[0.22, 1.0]`.
///
/// When `fading` is true (last 3 s), combines three incommensurate sine waves
/// so the filament sputters and dips continuously with no discrete stepping.
#[inline]
fn lens_brightness(t: f32, fading: bool) -> f32 {
    let t = if t.is_finite() { t } else { 0.0 };
    if !fading {
        return 0.96 + 0.04 * (t * 2.8).sin();
    }
    let w1 = 0.5 + 0.5 * (t * 7.5).sin();
    let w2 = 0.5 + 0.5 * (t * 12.8 + 1.1).sin();
    let w3 = 0.5 + 0.5 * (t * 4.1 + 2.4).sin();
    let pulse = (w1 * 0.50 + w2 * 0.35 + w3 * 0.15).powf(1.6);
    0.22 + 0.78 * pulse
}

/// Lens colour at radial fraction `r_frac` (`0` = centre, `1` = rim) and
/// filament `brightness` (`0.22..=1.0`).
#[inline]
fn lens_color(r_frac: f32, brightness: f32) -> Rgb {
    let r = r_frac.clamp(0.0, 1.0);
    let hot = if r < 0.45 {
        mix(LENS_CORE, HEADLAMP_LENS_COLOR, r / 0.45)
    } else {
        mix(HEADLAMP_LENS_COLOR, LENS_RIM, (r - 0.45) / 0.55)
    };
    let b = brightness.clamp(0.0, 1.0);
    // Warm tungsten shift as the filament dims during the final 3 s flicker.
    let cool_dim = mix(LENS_DIM, hot, smoothstep(0.18, 0.92, b));
    scale_rgb(cool_dim, 0.48 + 0.52 * b)
}

/// Floor pickup sprite: a small miner's headlamp on a looped dark elastic strap
/// lying on the floor, with a round gunmetal-and-brass housing, a bright warm
/// emissive lens, and a slow, smooth diagonal glint driven by `t`.
///
/// Strictly stays inside the shared `13×13` pickup footprint (`dx, dy in -6..=6`)
/// and uses an asymmetric strap-left / lamp-right silhouette distinct from every
/// other pocket find.
pub fn draw_headlamp_pickup(pb: &mut PixelBuffer, pos: Vec2, cam: Vec2, t: f32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let (cx, cy) = w2s(pos, cam);
    if cx + 6 < 0 || cy + 6 < 0 || cx - 6 >= pb.w as i32 || cy - 6 >= pb.h as i32 {
        return;
    }

    let t = if t.is_finite() { t } else { 0.0 };
    let prev_pen = pb.pen();
    let strap_pen = cloth_pen();
    let metal_pen = prop_pen(PropKind::Toolbox);
    let glass_pen = washer_glass_pen();

    // 1. Grounded contact shadow shifted slightly down-right (preserves floor mat).
    for dx in -4..=4 {
        let amt = if (-2..=3).contains(&dx) { 0.46 } else { 0.34 };
        shade_floor(pb, cx + dx, cy + 4, amt);
    }
    shade_floor(pb, cx + 4, cy + 3, 0.38);
    shade_floor(pb, cx + 5, cy + 2, 0.28);

    // 2. Looped dark elastic headband on the left (`dx in -6..=-1, dy in -3..=+3`)
    //    with an open hollow centre (`dx in -4..=-2, dy in -1..=0`) so floor shows
    //    through and the silhouette reads immediately as a strap + lamp.
    pb.set_pen(strap_pen);
    // Outer contour around the strap loop.
    pb.rect_fill(cx - 4, cy - 3, 5, 1, OUTLINE);
    pb.plot(cx - 5, cy - 2, OUTLINE);
    pb.plot(cx - 6, cy - 1, OUTLINE);
    pb.plot(cx - 6, cy, OUTLINE);
    pb.plot(cx - 6, cy + 1, OUTLINE);
    pb.plot(cx - 5, cy + 2, OUTLINE);
    pb.rect_fill(cx - 4, cy + 3, 5, 1, OUTLINE);
    // Inner contour framing the hollow loop opening.
    pb.plot(cx - 4, cy - 1, OUTLINE);
    pb.plot(cx - 1, cy - 1, OUTLINE);
    pb.plot(cx - 1, cy, OUTLINE);
    pb.plot(cx - 2, cy + 1, OUTLINE);
    pb.plot(cx - 1, cy + 1, OUTLINE);

    // Elastic strap webbing body with top-left directional highlight.
    pb.plot(cx - 4, cy - 2, STRAP_HI);
    pb.plot(cx - 3, cy - 2, STRAP_HI);
    pb.plot(cx - 2, cy - 2, STRAP_MID);
    pb.plot(cx - 1, cy - 2, STRAP_MID);

    pb.plot(cx - 5, cy - 1, STRAP_HI);
    pb.plot(cx - 5, cy, STRAP_MID);
    pb.plot(cx - 5, cy + 1, STRAP_LO);

    pb.plot(cx - 4, cy + 1, STRAP_LO);
    pb.plot(cx - 4, cy + 2, STRAP_MID);
    pb.plot(cx - 3, cy + 2, STRAP_MID);
    pb.plot(cx - 2, cy + 2, STRAP_LO);
    pb.plot(cx - 1, cy + 2, STRAP_LO);

    // 3. Lamp housing: gunmetal-black rear barrel, beveled brass bezel ring,
    //    and tiny brass strap adjuster buckle (Metal material pen).
    pb.set_pen(metal_pen);
    // Brass strap slide clasp on the upper strap.
    pb.plot(cx - 2, cy - 2, BRASS_HI);
    pb.plot(cx - 2, cy - 3, BRASS_MID);

    // Dark housing outline framing the protruding round miner's lamp on the right.
    pb.rect_fill(cx + 1, cy - 4, 3, 1, OUTLINE);
    pb.plot(cx, cy - 3, OUTLINE);
    pb.plot(cx + 4, cy - 3, OUTLINE);
    pb.plot(cx + 5, cy - 2, OUTLINE);
    pb.plot(cx + 5, cy + 1, OUTLINE);
    pb.plot(cx + 4, cy + 2, OUTLINE);
    pb.rect_fill(cx + 1, cy + 3, 3, 1, OUTLINE);

    // Gunmetal rear cap linking the strap to the bezel (`dx = 0`).
    pb.plot(cx, cy - 2, CASE_HI);
    pb.plot(cx, cy - 1, CASE_MID);
    pb.plot(cx, cy, CASE_MID);
    pb.plot(cx, cy + 1, CASE_DEEP);
    pb.plot(cx, cy + 2, CASE_DEEP);

    // Round beveled brass bezel housing (`dx in 1..=4, dy in -3..=+2`).
    pb.plot(cx + 1, cy - 3, BRASS_HI);
    pb.plot(cx + 2, cy - 3, BRASS_HI);
    pb.plot(cx + 3, cy - 3, BRASS_MID);
    pb.plot(cx + 1, cy - 2, BRASS_HI);
    pb.plot(cx + 4, cy - 2, BRASS_MID);
    pb.plot(cx + 1, cy - 1, BRASS_MID);
    pb.plot(cx + 1, cy, BRASS_MID);
    pb.plot(cx + 1, cy + 1, BRASS_DEEP);
    pb.plot(cx + 2, cy + 1, BRASS_MID);
    pb.plot(cx + 4, cy + 1, BRASS_DEEP);
    pb.plot(cx + 1, cy + 2, BRASS_DEEP);
    pb.plot(cx + 2, cy + 2, BRASS_MID);
    pb.plot(cx + 3, cy + 2, BRASS_DEEP);

    // 4. Bright warm-white lens with a slow, continuous diagonal glint sweep.
    //    `band` sweeps smoothly from -2.2 to +2.2 (entering & exiting cleanly
    //    past the lens edges so wrap-around is invisible and never stepped).
    let cycle = (t * 0.55).rem_euclid(1.0);
    let band = -2.2 + cycle * 4.4;
    let breath = 0.92 + 0.08 * (t * 2.2).sin();

    // 8-pixel circular/oval lens face centred near `(cx + 3, cy - 1)`, marked
    // emissive so it stays bright on dark floors and under lighting effects.
    const LENS_PX: [(i32, i32, f32, bool); 8] = [
        (2, -2, 0.65, false),
        (3, -2, 0.55, true),
        (2, -1, 0.35, true),
        (3, -1, 0.00, true),
        (4, -1, 0.45, true),
        (2, 0, 0.55, true),
        (3, 0, 0.30, true),
        (4, 0, 0.60, false),
    ];

    for &(dx, dy, r_frac, core) in &LENS_PX {
        let diag = (dx - 3) as f32 * 0.72 + (dy + 1) as f32 * 0.68;
        let dist = (diag - band).abs();
        let g = 1.0 - (dist / 1.15).min(1.0);
        let glint = g * g * (3.0 - 2.0 * g);

        let base = if r_frac < 0.25 {
            mix(LENS_CORE, HEADLAMP_LENS_COLOR, r_frac * 2.0)
        } else {
            mix(HEADLAMP_LENS_COLOR, LENS_RIM, ((r_frac - 0.25) / 0.55).clamp(0.0, 1.0))
        };
        let col = mix(base, LENS_CORE, glint * 0.72);
        pb.set_pen(if core { EMISSIVE_PEN } else { glass_pen });
        pb.plot(cx + dx, cy + dy, col);
    }

    // Extra glowing lower-lens rim pixel (`(3, 1)` -> glass rim).
    pb.set_pen(glass_pen);
    pb.plot(cx + 3, cy + 1, mix(LENS_RIM, HEADLAMP_LENS_COLOR, 0.45 * breath));

    // 5. Soft warm light kiss right in front of the lens (`dx = 5`, inside 13×13).
    let spill = 0.28 * breath + 0.14 * (1.0 - ((band - 0.6).abs() / 1.4).min(1.0));
    add_spill(pb, cx + 5, cy - 1, SPILL_WARM, spill);
    add_spill(pb, cx + 5, cy, SPILL_WARM, spill * 0.85);

    pb.set_pen(prev_pen);
}

/// Detect whether the pixels at `center` already belong to the Outfall diver
/// helmet (`rat_helmet_pen` or `rat_visor_pen`), so the lamp strap widens to
/// clasp cleanly onto the round brass diving helmet.
fn on_diver_helmet(pb: &PixelBuffer, center: Vec2) -> bool {
    if !pb.materials_on() {
        return false;
    }
    let h0 = rat_helmet_pen(false);
    let h1 = rat_helmet_pen(true);
    let v0 = rat_visor_pen(false);
    let v1 = rat_visor_pen(true);
    let cx = center.x.round() as i32;
    let cy = center.y.round() as i32;
    let mat = pb.mat();
    for dy in -1..=1 {
        let py = cy + dy;
        if py < 0 || py as usize >= pb.h {
            continue;
        }
        let row = py as usize * pb.w;
        for dx in -1..=1 {
            let px = cx + dx;
            if px < 0 || px as usize >= pb.w {
                continue;
            }
            let b = mat[row + px as usize];
            if b == h0 || b == h1 || b == v0 || b == v1 {
                return true;
            }
        }
    }
    false
}

/// Draw the headlamp strapped onto the rat's head (or the Outfall diver helmet),
/// with the bright lens aimed along `facing`, scaled smoothly by `scale`
/// (`0.35×` Shrink Draught through `1×` normal up to `5×` Growth Serum).
///
/// When `fading` is true (last 3 s of the powerup), the lens flickers smoothly
/// without discrete steps.
pub fn draw_rat_headlamp(
    pb: &mut PixelBuffer,
    head: Vec2,
    facing: Vec2,
    scale: f32,
    cam: Vec2,
    t: f32,
    fading: bool,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let scale = if scale.is_finite() {
        scale.clamp(0.35, 8.0)
    } else {
        1.0
    };
    let face = if facing.len_sq() > 1e-4 {
        facing.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let perp = face.perp();
    let center = head - cam;

    let reach = (3.2 * scale).ceil() as i32 + 4;
    let cx = center.x.round() as i32;
    let cy = center.y.round() as i32;
    if cx + reach < 0 || cy + reach < 0 || cx - reach >= pb.w as i32 || cy - reach >= pb.h as i32 {
        return;
    }

    let prev_pen = pb.pen();
    let giant_pen = scale > 1.5;
    let metal_pen = rat_helmet_pen(giant_pen);
    let visor_pen = rat_visor_pen(giant_pen);
    let strap_pen = cloth_pen();
    let brightness = lens_brightness(t, fading);
    let helmet = on_diver_helmet(pb, center);

    // Tiny rat (`0.35×` Shrink Draught): guarantee crisp, distinct strap,
    // metallic housing, and forward-facing emissive lens pixels.
    if scale < 0.65 {
        pb.set_pen(strap_pen);
        for side in [-1.0f32, 1.0] {
            let sp = center - face * 0.45 + perp * (side * 0.90);
            blend_pen(pb, sp.x.round() as i32, sp.y.round() as i32, STRAP_LO, 0.90);
        }

        pb.set_pen(metal_pen);
        let hp = center + face * 0.15;
        let hx = hp.x.round() as i32;
        let hy = hp.y.round() as i32;
        pb.plot(hx, hy, BRASS_MID);

        pb.set_pen(EMISSIVE_PEN);
        let lp = center + face * 1.25;
        let mut lx = lp.x.round() as i32;
        let mut ly = lp.y.round() as i32;
        if (lx, ly) == (hx, hy) {
            lx += face.x.round() as i32;
            ly += face.y.round() as i32;
        }
        pb.plot(lx, ly, lens_color(0.0, brightness));

        pb.set_pen(prev_pen);
        return;
    }

    let light_u = face.dot(LIGHT);
    let light_v = perp.dot(LIGHT);
    let detail = ((scale - 1.15) / 1.85).clamp(0.0, 1.0);

    // Strap geometry: wraps cleanly across the rat's forehead (just in front
    // of the ears and behind the eyes) or widens flush onto the round brass
    // Outfall diving helmet without overhanging its rim.
    let v_span = if helmet { 2.02 } else { 1.50 };
    let strap_u0 = if helmet { -0.18 } else { -0.05 };
    let strap_curve = if helmet { 0.12 } else { 0.10 };
    let strap_half_u = (0.28f32).max(0.46 / scale);

    // Lamp housing & lens ellipses in normalised rat units around `head`.
    let case_u = 0.20f32;
    let case_ru = (0.52f32).max(0.55 / scale);
    let case_rv = (0.74f32).max(0.70 / scale);

    let bezel_u = 0.56f32;
    let bezel_ru = (0.50f32).max(0.58 / scale);
    let bezel_rv = (0.80f32).max(0.75 / scale);

    let lens_u = 0.88f32;
    let lens_ru = (0.38f32).max(0.52 / scale);
    let lens_rv = (0.60f32).max(0.62 / scale);

    let x0 = (cx - reach).max(0);
    let x1 = (cx + reach).min(pb.w as i32 - 1);
    let y0 = (cy - reach).max(0);
    let y1 = (cy + reach).min(pb.h as i32 - 1);

    // Subtle grounded contact shadow beneath the giant lamp housing on the head.
    if scale >= 2.0 {
        let sh_off = Vec2::new(0.28 * scale, 0.34 * scale);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = Vec2::new(x as f32, y as f32) - center - sh_off;
                let lu = d.dot(face) / scale;
                let lv = d.dot(perp) / scale;
                let du = (lu - bezel_u) / (bezel_ru + 0.18);
                let dv = lv / (bezel_rv + 0.14);
                let r = du.hypot(dv);
                if r < 1.0 {
                    shade_floor(pb, x, y, 0.26 * (1.0 - r));
                }
            }
        }
    }

    // Rasterise strap, housing (gunmetal rear + brass bezel), and glowing lens
    // in one crisp analytical pass per pixel.
    let outline_px = if scale >= 1.8 { 0.95 } else { 0.0 };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - center;
            let lu = d.dot(face) / scale;
            let lv = d.dot(perp) / scale;

            // 1. Distance to curved elastic strap.
            let lv_c = lv.clamp(-v_span, v_span);
            let u_arc = strap_u0 - strap_curve * lv_c * lv_c;
            let d_strap_px = ((lu - u_arc).hypot(lv - lv_c) - strap_half_u) * scale;

            // 2. Distance to gunmetal rear barrel & brass bezel collar.
            let c_norm = ((lu - case_u) / case_ru).hypot(lv / case_rv);
            let d_case_px = (c_norm - 1.0) * case_ru.min(case_rv) * scale;

            let b_norm = ((lu - bezel_u) / bezel_ru).hypot(lv / bezel_rv);
            let d_bezel_px = (b_norm - 1.0) * bezel_ru.min(bezel_rv) * scale;

            // 3. Distance to convex front lens.
            let l_du = (lu - lens_u) / lens_ru;
            let l_dv = lv / lens_rv;
            let l_norm = l_du.hypot(l_dv);
            let d_lens_px = (l_norm - 1.0) * lens_ru.min(lens_rv) * scale;

            let d_lamp_px = d_case_px.min(d_bezel_px).min(d_lens_px);

            // Priority: lens > brass bezel > gunmetal rear > elastic strap > outline.
            if d_lens_px <= 0.15 {
                let r_frac = l_norm.clamp(0.0, 1.0);
                let mut col = lens_color(r_frac, brightness);
                // Smooth top-left curved glass highlight on larger scales.
                if detail > 0.15 && r_frac < 0.85 {
                    let gu = l_du - 0.32 * light_u;
                    let gv = l_dv - 0.32 * light_v;
                    let spec = (1.0 - (gu * gu + gv * gv) / 0.24).clamp(0.0, 1.0);
                    col = mix(col, LENS_CORE, spec * 0.55 * detail * brightness);
                }
                let is_core = r_frac <= 0.80;
                pb.set_pen(if is_core { EMISSIVE_PEN } else { visor_pen });
                if d_lens_px > -0.35 && scale >= 2.0 {
                    let edge = (-d_lens_px / 0.35).clamp(0.0, 1.0);
                    blend_pen(pb, x, y, col, 0.55 + 0.45 * edge);
                } else {
                    pb.plot(x, y, col);
                }
            } else if d_bezel_px <= 0.15 {
                let bu = (lu - bezel_u) / bezel_ru;
                let bv = lv / bezel_rv;
                let bz = (1.0 - (bu * bu + bv * bv).min(1.0)).sqrt();
                let lit = (0.38 + 0.40 * bz + 0.56 * (bu * light_u + bv * light_v)).clamp(0.0, 1.15);
                let mut col = if lit < 0.42 {
                    mix(BRASS_DEEP, BRASS_MID, lit / 0.42)
                } else {
                    mix(BRASS_MID, BRASS_HI, ((lit - 0.42) / 0.65).clamp(0.0, 1.0))
                };
                if d_bezel_px > -0.45 && scale >= 2.0 {
                    let et = (-d_bezel_px / 0.45).clamp(0.0, 1.0);
                    col = mix(OUTLINE, col, 0.40 + 0.60 * et);
                }
                pb.set_pen(metal_pen);
                pb.plot(x, y, col);
            } else if d_case_px <= 0.15 {
                let cu = (lu - case_u) / case_ru;
                let cv = lv / case_rv;
                let cz = (1.0 - (cu * cu + cv * cv).min(1.0)).sqrt();
                let lit = (0.32 + 0.42 * cz + 0.52 * (cu * light_u + cv * light_v)).clamp(0.0, 1.10);
                let mut col = if lit < 0.48 {
                    mix(CASE_DEEP, CASE_MID, lit / 0.48)
                } else {
                    mix(CASE_MID, CASE_HI, ((lit - 0.48) / 0.62).clamp(0.0, 1.0))
                };
                if d_case_px > -0.45 && scale >= 2.0 {
                    let et = (-d_case_px / 0.45).clamp(0.0, 1.0);
                    col = mix(OUTLINE, col, 0.35 + 0.65 * et);
                }
                pb.set_pen(metal_pen);
                pb.plot(x, y, col);
            } else if d_strap_px <= 0.15 {
                let lit = (0.45 + 0.45 * ((lv / v_span) * light_v + 0.25 * light_u)).clamp(0.0, 1.0);
                let mut col = if lit < 0.50 {
                    mix(STRAP_LO, STRAP_MID, lit * 2.0)
                } else {
                    mix(STRAP_MID, STRAP_HI, (lit - 0.50) * 2.0)
                };
                let mut pen = strap_pen;
                // Woven ribbing & brass temple clamp clips at larger scales.
                if detail > 0.20 {
                    let rib = 0.5 + 0.5 * (lv * 9.0).sin();
                    col = mix(col, STRAP_LO, 0.20 * rib * detail);
                    let clip_v = v_span * 0.72;
                    if (lv.abs() - clip_v).abs() < 0.20 && (lu - u_arc).abs() < strap_half_u * 0.85 {
                        col = if lit > 0.55 { BRASS_HI } else { BRASS_MID };
                        pen = metal_pen;
                    }
                }
                pb.set_pen(pen);
                pb.plot(x, y, col);
            } else if outline_px > 0.0 && d_lamp_px.min(d_strap_px) <= outline_px {
                let d_min = d_lamp_px.min(d_strap_px);
                let oa = ((outline_px - d_min) / outline_px).clamp(0.0, 1.0) * 0.88;
                pb.set_pen(if d_lamp_px <= d_strap_px {
                    metal_pen
                } else {
                    strap_pen
                });
                blend_pen(pb, x, y, OUTLINE, oa);
            }
        }
    }

    // Readability guarantee at 1× (any 360° angle): anchor the brass housing
    // pixel and the bright emissive lens pixel cleanly ahead of it so neither
    // drops out on diagonals.
    if scale < 1.5 {
        let hp = center + face * (0.35 * scale);
        let hx = hp.x.round() as i32;
        let hy = hp.y.round() as i32;
        pb.set_pen(metal_pen);
        pb.plot(hx, hy, BRASS_MID);

        let lp = center + face * (1.05 * scale);
        let mut lx = lp.x.round() as i32;
        let mut ly = lp.y.round() as i32;
        if (lx, ly) == (hx, hy) {
            lx += face.x.round() as i32;
            ly += face.y.round() as i32;
        }
        pb.set_pen(EMISSIVE_PEN);
        pb.plot(lx, ly, lens_color(0.0, brightness));
    }

    // Subtle forward warm lens bloom right in front of the glass (`+face`).
    let spill_dist = (1.42 * scale).max(1.8);
    let spill_pos = center + face * spill_dist;
    let spill_r = (0.38 * scale).round() as i32;
    let sx0 = spill_pos.x.round() as i32;
    let sy0 = spill_pos.y.round() as i32;
    for dy in -spill_r..=spill_r {
        for dx in -spill_r..=spill_r {
            let d2 = (dx * dx + dy * dy) as f32;
            let max_r = (spill_r as f32 + 0.6).max(1.0);
            if d2 <= max_r * max_r {
                let fall = (1.0 - d2.sqrt() / max_r).clamp(0.0, 1.0);
                add_spill(pb, sx0 + dx, sy0 + dy, SPILL_WARM, 0.30 * brightness * fall);
            }
        }
    }

    pb.set_pen(prev_pen);
}

/// `8 × 8` HUD/Library icon for the Headlamp (matching `art_icons::LARGE_PX_W × LARGE_PX_H`),
/// drawn with its top-left corner at `(x, y)`. Transparent background pixels
/// are preserved.
pub fn draw_headlamp_icon(pb: &mut PixelBuffer, x: i32, y: i32) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }

    // 8×8 hand-authored sprite:
    //   K / S : dark elastic headband loop on the left
    //   M / D : gunmetal lamp casing
    //   H / B / R : beveled brass bezel collar
    //   C / W / L : bright warm-white emissive lens
    //   G     : diverging warm beam rays on the right
    const ICON: [&[u8; 8]; 8] = [
        b"..KKK...",
        b".KS.HB.G",
        b"KS.MBWL.",
        b"K..MHCWG",
        b"K..MBCWG",
        b"KS.DBWL.",
        b".KS.RB.G",
        b"..KKK...",
    ];

    let prev_pen = pb.pen();
    let strap_pen = cloth_pen();
    let metal_pen = prop_pen(PropKind::Toolbox);
    let glass_pen = washer_glass_pen();

    for (ry, row) in ICON.iter().enumerate() {
        for (rx, &ch) in row.iter().enumerate() {
            let Some((col, pen)) = (match ch {
                b'K' => Some((STRAP_LO, strap_pen)),
                b'S' => Some((STRAP_HI, strap_pen)),
                b'M' => Some((CASE_MID, metal_pen)),
                b'D' => Some((CASE_DEEP, metal_pen)),
                b'H' => Some((BRASS_HI, metal_pen)),
                b'B' => Some((BRASS_MID, metal_pen)),
                b'R' => Some((BRASS_DEEP, metal_pen)),
                b'C' => Some((LENS_CORE, EMISSIVE_PEN)),
                b'W' => Some((HEADLAMP_LENS_COLOR, EMISSIVE_PEN)),
                b'L' => Some((LENS_RIM, glass_pen)),
                b'G' => Some((SPILL_WARM, glass_pen)),
                _ => None,
            }) else {
                continue;
            };
            pb.set_pen(pen);
            pb.plot(x + rx as i32, y + ry as i32, col);
        }
    }

    pb.set_pen(prev_pen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor::draw_pickup;
    use crate::game::art_icons::{LARGE_PX_H, LARGE_PX_W};
    use crate::game::gem::PickupKind;
    use crate::render::lighting::luminance;

    const OTHER_FINDS: [PickupKind; 11] = [
        PickupKind::Heal,
        PickupKind::Magnet,
        PickupKind::Nuke,
        PickupKind::BottleCap,
        PickupKind::WindupMouse,
        PickupKind::PocketWatch,
        PickupKind::CleanSocks,
        PickupKind::GrowthSerum,
        PickupKind::ShrinkDraught,
        PickupKind::WhiteCoat,
        PickupKind::TheCollar,
    ];

    #[test]
    fn each_function_draws_non_empty_and_respects_bounds() {
        const BG: Rgb = (1, 2, 3);

        // 1. Floor pickup: non-empty and strictly inside the 13×13 shared footprint.
        let mut pb = PixelBuffer::new(32, 32);
        pb.clear(BG);
        draw_headlamp_pickup(&mut pb, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.4);
        let mut painted = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let changed = pb.pixel_at(x, y) != BG;
                let inside = (10..=22).contains(&x) && (10..=22).contains(&y);
                assert!(
                    inside || !changed,
                    "pickup painted outside 13×13 footprint at ({x}, {y})"
                );
                if changed {
                    painted += 1;
                }
            }
        }
        assert!(painted >= 40, "pickup painted too few pixels ({painted})");

        // 2. Rat headlamp: non-empty across scales and facings.
        for &scale in &[0.35f32, 1.0, 5.0] {
            for facing in [
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, -1.0),
                Vec2::new(-1.0, 0.0),
                Vec2::new(0.707, 0.707),
            ] {
                let mut rpb = PixelBuffer::new(64, 64);
                rpb.clear(BG);
                draw_rat_headlamp(
                    &mut rpb,
                    Vec2::new(32.0, 32.0),
                    facing,
                    scale,
                    Vec2::ZERO,
                    0.5,
                    false,
                );
                let count = rpb.pixels().iter().filter(|&&p| p != BG).count();
                assert!(
                    count > 0,
                    "draw_rat_headlamp drew nothing at scale={scale} facing={facing:?}"
                );
            }
        }

        // 3. Icon: non-empty and strictly inside 8×8 (`LARGE_PX_W × LARGE_PX_H`).
        let mut ipb = PixelBuffer::new(24, 24);
        ipb.clear(BG);
        draw_headlamp_icon(&mut ipb, 8, 8);
        let mut icon_px = 0usize;
        for y in 0..ipb.h {
            for x in 0..ipb.w {
                let changed = ipb.pixel_at(x, y) != BG;
                let inside = (8..8 + LARGE_PX_W).contains(&x) && (8..8 + LARGE_PX_H).contains(&y);
                assert!(
                    inside || !changed,
                    "icon painted outside 8×8 box at ({x}, {y})"
                );
                if changed {
                    icon_px += 1;
                }
            }
        }
        assert!(icon_px >= 20, "icon painted too few pixels ({icon_px})");
        assert!(
            icon_px < LARGE_PX_W * LARGE_PX_H,
            "icon should preserve transparent background corners"
        );
    }

    #[test]
    fn lens_pixel_is_brighter_than_housing() {
        const BG: Rgb = (10, 12, 16);

        // 1. Floor pickup: emissive lens centre `(16 + 3, 16 - 1)` vs gunmetal/brass housing.
        let mut pb = PixelBuffer::new(32, 32);
        pb.set_materials(true);
        pb.clear(BG);
        draw_headlamp_pickup(&mut pb, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.5);
        let metal = prop_pen(PropKind::Toolbox);
        let mut max_housing_lum = 0u8;
        let mut min_lens_lum = u8::MAX;
        let mut lens_count = 0usize;
        let mut housing_count = 0usize;
        for (px, &m) in pb.pixels().iter().zip(pb.mat().iter()) {
            if m == EMISSIVE_PEN {
                min_lens_lum = min_lens_lum.min(luminance(*px));
                lens_count += 1;
            } else if m == metal {
                max_housing_lum = max_housing_lum.max(luminance(*px));
                housing_count += 1;
            }
        }
        assert!(lens_count >= 4, "pickup should mark lens pixels emissive");
        assert!(housing_count >= 8, "pickup should stamp housing with metal pen");
        assert!(
            min_lens_lum > max_housing_lum,
            "pickup lens ({min_lens_lum}) must be brighter than housing ({max_housing_lum})"
        );

        // 2. On the rat at every scale (0.35×, 1×, 5×): lens strictly brighter than housing.
        for &scale in &[0.35f32, 1.0, 5.0] {
            let mut rpb = PixelBuffer::new(64, 64);
            rpb.set_materials(true);
            rpb.clear(BG);
            draw_rat_headlamp(
                &mut rpb,
                Vec2::new(32.0, 32.0),
                Vec2::new(1.0, 0.0),
                scale,
                Vec2::ZERO,
                0.25,
                false,
            );
            let hpen = rat_helmet_pen(scale > 1.5);
            let mut max_h = 0u8;
            let mut max_l = 0u8;
            let mut h_n = 0usize;
            let mut l_n = 0usize;
            for (px, &m) in rpb.pixels().iter().zip(rpb.mat().iter()) {
                if m == EMISSIVE_PEN {
                    max_l = max_l.max(luminance(*px));
                    l_n += 1;
                } else if m == hpen {
                    max_h = max_h.max(luminance(*px));
                    h_n += 1;
                }
            }
            assert!(l_n >= 1 && h_n >= 1, "scale={scale}: missing lens ({l_n}) or housing ({h_n})");
            assert!(
                max_l > max_h,
                "scale={scale}: rat lamp lens ({max_l}) must be brighter than housing ({max_h})"
            );
        }

        // 3. Icon: lens core brighter than housing.
        let mut ipb = PixelBuffer::new(16, 16);
        ipb.clear(BG);
        draw_headlamp_icon(&mut ipb, 4, 4);
        let lens_lum = luminance(ipb.pixel_at(9, 7));
        let case_lum = luminance(ipb.pixel_at(7, 7));
        let brass_lum = luminance(ipb.pixel_at(8, 6));
        assert!(
            lens_lum > brass_lum && lens_lum > case_lum,
            "icon lens ({lens_lum}) must outshine brass ({brass_lum}) and casing ({case_lum})"
        );
    }

    #[test]
    fn rat_lamp_scales_and_faces_direction() {
        const BG: Rgb = (5, 7, 9);

        let measure = |scale: f32, facing: Vec2| -> (usize, usize, f32) {
            let mut pb = PixelBuffer::new(96, 96);
            pb.set_materials(true);
            pb.clear(BG);
            draw_rat_headlamp(
                &mut pb,
                Vec2::new(48.0, 48.0),
                facing,
                scale,
                Vec2::ZERO,
                0.0,
                false,
            );
            let mut count = 0usize;
            let mut min_x = usize::MAX;
            let mut max_x = 0usize;
            let mut min_y = usize::MAX;
            let mut max_y = 0usize;
            let mut lens_dot_sum = 0.0f32;
            let mut lens_n = 0.0f32;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if pb.pixel_at(x, y) != BG {
                        count += 1;
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                    if pb.mat()[y * pb.w + x] == EMISSIVE_PEN {
                        let rel = Vec2::new(x as f32 - 48.0, y as f32 - 48.0);
                        lens_dot_sum += rel.dot(facing);
                        lens_n += 1.0;
                    }
                }
            }
            let span = (max_x - min_x + 1).max(max_y - min_y + 1);
            (count, span, lens_dot_sum / lens_n.max(1.0))
        };

        let (c_tiny, s_tiny, dot_tiny) = measure(0.35, Vec2::new(1.0, 0.0));
        let (c_one, s_one, dot_one) = measure(1.0, Vec2::new(1.0, 0.0));
        let (c_giant, s_giant, dot_giant) = measure(5.0, Vec2::new(1.0, 0.0));

        assert!(c_tiny > 0 && c_one > c_tiny && c_giant > c_one * 6);
        assert!(s_one >= s_tiny && s_giant > s_one * 3);
        assert!(dot_tiny > 0.0 && dot_one > 0.0 && dot_giant > 0.0);

        // Lens also faces upward when facing up.
        let (_, _, dot_up) = measure(1.0, Vec2::new(0.0, -1.0));
        assert!(dot_up > 0.0, "lens should sit on the +facing (up) side of head");
    }

    #[test]
    fn fading_flickers_smoothly_and_pickup_glint_sweeps_smoothly() {
        const BG: Rgb = (10, 12, 16);

        // 1. Rat headlamp fading flicker varies smoothly over time.
        let mut prev_lum: Option<u8> = None;
        let mut min_lum = u8::MAX;
        let mut max_lum = 0u8;
        for step in 0..150 {
            let t = step as f32 * 0.02;
            let mut pb = PixelBuffer::new(32, 32);
            pb.set_materials(true);
            pb.clear(BG);
            draw_rat_headlamp(
                &mut pb,
                Vec2::new(16.0, 16.0),
                Vec2::new(1.0, 0.0),
                1.0,
                Vec2::ZERO,
                t,
                true,
            );
            let peak = pb
                .pixels()
                .iter()
                .zip(pb.mat().iter())
                .filter(|(_, &m)| m == EMISSIVE_PEN)
                .map(|(px, _)| luminance(*px))
                .max()
                .unwrap_or(0);
            min_lum = min_lum.min(peak);
            max_lum = max_lum.max(peak);
            if let Some(p) = prev_lum {
                assert!(
                    peak.abs_diff(p) <= 45,
                    "fading flicker stepped too abruptly at t={t:.2}: {p} -> {peak}"
                );
            }
            prev_lum = Some(peak);
        }
        assert!(
            max_lum - min_lum >= 50,
            "fading flicker should visibly modulate brightness ({min_lum}..{max_lum})"
        );

        // 2. Pickup glint sweeps smoothly across the lens with `t`.
        let mut a = PixelBuffer::new(32, 32);
        let mut b = PixelBuffer::new(32, 32);
        a.clear(BG);
        b.clear(BG);
        draw_headlamp_pickup(&mut a, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.10);
        draw_headlamp_pickup(&mut b, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.85);
        assert_ne!(a.pixels(), b.pixels(), "pickup lens glint should sweep with t");
    }

    #[test]
    fn pickup_silhouette_differs_from_all_other_finds() {
        const BG: Rgb = (1, 2, 3);
        const MIN_SILHOUETTE_DIFF: usize = 18;

        let mut head_pb = PixelBuffer::new(32, 32);
        head_pb.clear(BG);
        draw_headlamp_pickup(&mut head_pb, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.0);
        let head_mask: Vec<bool> = head_pb.pixels().iter().map(|&p| p != BG).collect();

        for other in OTHER_FINDS {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear(BG);
            draw_pickup(&mut pb, other, (16, 16), 0.0);
            let other_mask: Vec<bool> = pb.pixels().iter().map(|&p| p != BG).collect();
            let diff = head_mask
                .iter()
                .zip(other_mask.iter())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                diff >= MIN_SILHOUETTE_DIFF,
                "Headlamp silhouette too close to {other:?} (diff={diff}, need >= {MIN_SILHOUETTE_DIFF})"
            );
        }
    }

    #[test]
    fn sits_on_diver_helmet_and_restores_caller_pen() {
        const CALLER_PEN: u8 = 42;
        let mut pb = PixelBuffer::new(64, 64);
        pb.set_materials(true);
        pb.clear((18, 30, 40));
        // Pre-stamp a round Outfall diver helmet under the head position.
        pb.set_pen(rat_helmet_pen(false));
        pb.rect_fill(29, 29, 6, 6, BRASS_MID);
        pb.set_pen(CALLER_PEN);

        draw_rat_headlamp(
            &mut pb,
            Vec2::new(32.0, 32.0),
            Vec2::new(1.0, 0.0),
            1.0,
            Vec2::ZERO,
            0.3,
            false,
        );
        assert_eq!(pb.pen(), CALLER_PEN, "draw_rat_headlamp must restore caller pen");
        assert!(pb.mat().contains(&EMISSIVE_PEN), "helmet lamp lens must be emissive");

        draw_headlamp_pickup(&mut pb, Vec2::new(16.0, 16.0), Vec2::ZERO, 0.3);
        assert_eq!(pb.pen(), CALLER_PEN, "draw_headlamp_pickup must restore caller pen");

        draw_headlamp_icon(&mut pb, 4, 4);
        assert_eq!(pb.pen(), CALLER_PEN, "draw_headlamp_icon must restore caller pen");
    }

    #[test]
    fn clips_at_edges_and_empty_buffer() {
        let mut pb = PixelBuffer::new(8, 8);
        for &(x, y) in &[(-10, -10), (0, 0), (7, 7), (12, 4), (4, 12)] {
            draw_headlamp_pickup(&mut pb, Vec2::new(x as f32, y as f32), Vec2::ZERO, 1.0);
            draw_rat_headlamp(
                &mut pb,
                Vec2::new(x as f32, y as f32),
                Vec2::new(1.0, 0.0),
                1.0,
                Vec2::ZERO,
                1.0,
                false,
            );
            draw_headlamp_icon(&mut pb, x, y);
        }
        let mut empty = PixelBuffer::new(0, 0);
        draw_headlamp_pickup(&mut empty, Vec2::ZERO, Vec2::ZERO, 0.0);
        draw_rat_headlamp(&mut empty, Vec2::ZERO, Vec2::new(1.0, 0.0), 1.0, Vec2::ZERO, 0.0, false);
        draw_headlamp_icon(&mut empty, 0, 0);
    }
}
