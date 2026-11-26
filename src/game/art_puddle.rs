//! Art for floor puddles and expanding splash ripples. Pure drawing.
//!
//! Puddles are painted right after floor stains and before the boss ground
//! layer, with the lighting material pen set to the wet-floor material
//! (`MaterialKind::Water`) so lamp reflections (`render::lighting::softshadow`)
//! land on every painted puddle pixel.
//!
//! Each puddle reads the existing floor pixel (`PixelBuffer::pixel_at`) and
//! blends toward a per-theme water tint weighted by [`Puddle::depth`], keeping
//! brick mortar and tile grout faintly visible underneath so puddles read as
//! standing water on the floor rather than opaque holes or carpets.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::level::{Puddle, Theme};

/// Lifetime of a puddle ripple ring, in seconds.
pub const RIPPLE_LIFE: f32 = 0.45;

/// With the lighting effects on (`glossy`), the unlit water is this much darker
/// than the plain look: a dark mirror the real glints and reflections of
/// `render::lighting::water` stand out from. 0.12 = 12 % deeper.
pub const GLOSS_DEEPEN: f32 = 0.12;

/// Per-theme surface look for standing water.
struct WaterStyle {
    /// Multiplicative floor darkening at full puddle depth.
    darken: f32,
    /// Target water tint blended over the darkened floor.
    tint: Rgb,
    /// Peak blend factor toward `tint` at full depth.
    body_alpha: f32,
    /// Darkened meniscus colour along the 1-px outer waterline rim.
    rim_tint: Rgb,
    /// Blend factor for the waterline rim.
    rim_alpha: f32,
    /// Broken upper-left meniscus highlight colour.
    ul_highlight: Rgb,
    /// Peak blend factor for the upper-left highlight arc.
    ul_alpha: f32,
}

fn style_for(theme: Theme) -> WaterStyle {
    // Tuned on captures (batch fourteen): the first pass darkened the floor
    // toward a near-black tint, which read as a shadow or a hole on the dark
    // floors and vanished in an unlit room. Water is now a *cooler, slightly
    // lighter* sheet than the stone under it, closed by a dark waterline and
    // a pale meniscus all the way round, so the shore is the shape.
    match theme {
        Theme::Sewer => WaterStyle {
            darken: 0.92,
            tint: (28, 72, 82), // cool teal sheet
            body_alpha: 0.58,
            rim_tint: (5, 14, 16),
            rim_alpha: 0.60,
            ul_highlight: (112, 184, 180),
            ul_alpha: 0.72,
        },
        Theme::Laundry => WaterStyle {
            darken: 0.92,
            tint: (62, 86, 108), // soapy pale blue-grey
            body_alpha: 0.50,
            rim_tint: (18, 24, 38),
            rim_alpha: 0.56,
            ul_highlight: (128, 160, 176),
            ul_alpha: 0.56,
        },
        Theme::Kitchen => WaterStyle {
            darken: 0.90,
            tint: (52, 66, 74), // thin pale spill
            body_alpha: 0.40,
            rim_tint: (16, 14, 14),
            rim_alpha: 0.50,
            ul_highlight: (120, 138, 144),
            ul_alpha: 0.50,
        },
        Theme::Outfall => WaterStyle {
            darken: 0.94,
            tint: (30, 62, 86), // cool slate-blue ocean shallows (distinct from black tidal drops)
            body_alpha: 0.56,
            rim_tint: (12, 24, 36),
            rim_alpha: 0.52,
            ul_highlight: (106, 168, 192),
            ul_alpha: 0.64,
        },
        Theme::Lab | Theme::Printworks | Theme::Trailer => WaterStyle {
            darken: 0.90,
            tint: (36, 54, 64),
            body_alpha: 0.44,
            rim_tint: (12, 18, 22),
            rim_alpha: 0.50,
            ul_highlight: (104, 132, 140),
            ul_alpha: 0.50,
        },
    }
}

#[inline]
fn hash3(x: i32, y: i32, z: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ z.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n
}

/// Paint one irregular puddle onto `pb`, blending over the floor pixels already
/// drawn beneath it so tile grout and brick mortar show faintly through.
///
/// Writes through [`PixelBuffer::plot`] so only pixels strictly inside the
/// puddle (`depth > 0.0`) receive the caller's active material pen.
pub fn draw_puddle(pb: &mut PixelBuffer, puddle: &Puddle, cam: Vec2, elapsed: f32, theme: Theme) {
    draw_puddle_among(pb, puddle, &[], cam, elapsed, theme);
}

/// [`draw_puddle`] for a puddle that overlaps others: `neighbours` are the
/// puddles on screen with it (it may be among them). A shore pixel that is deep
/// inside a neighbour is drawn as open water, so a run of merged puddles has
/// one waterline round the whole sheet and none across it.
pub fn draw_puddle_among(
    pb: &mut PixelBuffer,
    puddle: &Puddle,
    neighbours: &[Puddle],
    cam: Vec2,
    elapsed: f32,
    theme: Theme,
) {
    draw_puddle_among_with(pb, puddle, neighbours, cam, elapsed, theme, false);
}

/// [`draw_puddle_among`] with the look chosen: `glossy` is the dark-mirror
/// base the lighting effects' water stage (`render::lighting::water`) lights.
/// It deepens the sheet by [`GLOSS_DEEPEN`] and leaves the still wavelet dashes
/// to the plain look. Both keep the waterline rim, meniscus and floating specks.
pub fn draw_puddle_among_with(
    pb: &mut PixelBuffer,
    puddle: &Puddle,
    neighbours: &[Puddle],
    cam: Vec2,
    _elapsed: f32,
    theme: Theme,
    glossy: bool,
) {
    if pb.w == 0 || pb.h == 0 || puddle.rx <= 0.0 || puddle.ry <= 0.0 {
        return;
    }
    // Snap camera once, matching `art_floor::draw_floor`, so puddles stay locked
    // to the brick and tile grid as the camera scrolls.
    let (cam_x, cam_y) = (cam.x.round() as i32, cam.y.round() as i32);
    let min_wx = (puddle.pos.x - puddle.rx).floor() as i32;
    let max_wx = (puddle.pos.x + puddle.rx).ceil() as i32;
    let min_wy = (puddle.pos.y - puddle.ry).floor() as i32;
    let max_wy = (puddle.pos.y + puddle.ry).ceil() as i32;

    let x0 = (min_wx - cam_x).max(0);
    let x1 = (max_wx - cam_x).min(pb.w as i32 - 1);
    let y0 = (min_wy - cam_y).max(0);
    let y1 = (max_wy - cam_y).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let style = style_for(theme);
    let inv_rx = 1.0 / puddle.rx.max(1.0);
    let inv_ry = 1.0 / puddle.ry.max(1.0);
    let pen = pb.pen();
    let check_wet = pb.materials_on() && pen != 0;
    let bw = pb.w;

    for sy in y0..=y1 {
        let wy = sy + cam_y;
        let wy_f = wy as f32 + 0.5;
        let ny = (wy_f - puddle.pos.y) * inv_ry;
        let row_off = sy as usize * bw;
        for sx in x0..=x1 {
            let wx = sx + cam_x;
            let wx_f = wx as f32 + 0.5;
            let here = Vec2::new(wx_f, wy_f);
            let mut depth = puddle.depth(here);
            if depth <= 0.0 {
                continue;
            }
            if depth <= 0.37 {
                // The shore band: the sheet's depth here is the deepest of
                // the puddles that cover it.
                for other in neighbours {
                    if (wx_f - other.pos.x).abs() < other.rx
                        && (wy_f - other.pos.y).abs() < other.ry
                        && other != puddle
                    {
                        depth = depth.max(other.depth(here));
                    }
                }
            }

            let already_wet =
                check_wet && pb.mat().get(row_off + sx as usize).copied() == Some(pen);
            // When two puddles merge into a longer wet stretch, suppress the
            // new puddle's outer rim and highlight where the floor is already
            // wet so the combined silhouette fuses cleanly.
            if already_wet && depth <= 0.37 {
                continue;
            }

            let bg = pb.pixel_at(sx as usize, sy as usize);
            let mut col = if already_wet {
                mix(bg, style.tint, style.body_alpha * 0.28 * depth)
            } else {
                let dark_bg = scale(bg, 1.0 - (1.0 - style.darken) * depth);
                let mut c = mix(dark_bg, style.tint, style.body_alpha * (0.28 + 0.72 * depth));
                if glossy {
                    c = scale(c, 1.0 - GLOSS_DEEPEN * depth);
                }

                if depth <= 0.19 {
                    // 1-px darker waterline rim right at the wet meniscus.
                    c = mix(c, style.rim_tint, style.rim_alpha);
                } else if depth <= 0.37 {
                    // The pale meniscus just inside the waterline: solid on the
                    // upper-left arc (the key light), broken and fainter round
                    // the rest, so the whole shore is drawn.
                    let nx = (wx_f - puddle.pos.x) * inv_rx;
                    let ul = -(nx * 0.72 + ny * 0.70);
                    let h = hash3(wx.div_euclid(3), wy.div_euclid(2), puddle.seed ^ 0x51A);
                    if ul > 0.20 {
                        let strength = (0.55 + (ul - 0.20) * 1.2).clamp(0.0, 1.0);
                        c = mix(c, style.ul_highlight, style.ul_alpha * strength);
                    } else if (h & 7) < 5 {
                        c = mix(c, style.ul_highlight, style.ul_alpha * 0.42);
                    }
                }
                c
            };

            if depth >= 0.42 {
                // Fixed dashes give the plain sheet texture even when its
                // shoreline lies outside the view.
                if !glossy && !already_wet {
                    let ch = hash3(wx.div_euclid(10), wy.div_euclid(6), puddle.seed ^ 0x9E37);
                    if (ch >> 16).is_multiple_of(3) {
                        let wy_off = ((ch >> 20) % 6) as i32;
                        let wx_off = ((ch >> 24) % 7) as i32;
                        if wy.rem_euclid(6) == wy_off
                            && (wx_off..wx_off + 3).contains(&wx.rem_euclid(10))
                        {
                            col = mix(col, style.ul_highlight, 0.20 * depth);
                        }
                    }
                }

                // Stationary theme-specific floating accents.
                if !already_wet {
                    match theme {
                        Theme::Sewer => {
                            let fx = wx.div_euclid(11);
                            let fy = wy.div_euclid(9);
                            let fh = hash3(fx, fy, puddle.seed ^ 0xBEEF);
                            if (fh & 7) == 0 {
                                let sox = 2 + ((fh >> 4) % 7) as i32;
                                let soy = 2 + ((fh >> 8) % 5) as i32;
                                let rx = wx - fx * 11;
                                let ry = wy - fy * 9;
                                if rx == sox && ry == soy {
                                    let speck = if (fh & 16) != 0 {
                                        (34, 48, 24)
                                    } else {
                                        (18, 28, 16)
                                    };
                                    col = mix(col, speck, 0.65);
                                } else if (fh & 32) != 0 && rx == sox + 1 && ry == soy {
                                    col = mix(col, (22, 34, 20), 0.48);
                                }
                            }
                        }
                        Theme::Laundry => {
                            let fx = wx.div_euclid(9);
                            let fy = wy.div_euclid(7);
                            let fh = hash3(fx, fy, puddle.seed ^ 0x50A9);
                            if (fh & 7) < 2 {
                                let sox = 1 + ((fh >> 4) % 6) as i32;
                                let soy = 1 + ((fh >> 8) % 5) as i32;
                                let rx = wx - fx * 9;
                                let ry = wy - fy * 7;
                                if rx == sox && ry == soy {
                                    let suds = if (fh & 16) != 0 {
                                        (134, 158, 168)
                                    } else {
                                        (108, 132, 146)
                                    };
                                    col = mix(col, suds, 0.52);
                                } else if (fh & 32) != 0 && rx == sox + 1 && ry == soy {
                                    col = mix(col, (118, 142, 156), 0.36);
                                }
                            }
                        }
                        Theme::Outfall => {
                            let fx = wx.div_euclid(10);
                            let fy = wy.div_euclid(8);
                            let fh = hash3(fx, fy, puddle.seed ^ 0x0FA1);
                            if (fh & 7) == 0 {
                                let sox = 1 + ((fh >> 4) % 7) as i32;
                                let soy = 1 + ((fh >> 8) % 5) as i32;
                                let rx = wx - fx * 10;
                                let ry = wy - fy * 8;
                                if rx == sox && ry == soy {
                                    let accent = if (fh & 16) != 0 {
                                        (96, 152, 174) // faint sea-foam fleck
                                    } else {
                                        (18, 46, 38) // drifting dark kelp speck
                                    };
                                    col = mix(col, accent, 0.52);
                                } else if (fh & 32) != 0 && rx == sox + 1 && ry == soy {
                                    col = mix(col, (76, 128, 150), 0.36);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            pb.plot(sx, sy, col);
        }
    }
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, mix(bg, c, alpha));
}

#[inline]
fn emit_ripple_quad(pb: &mut PixelBuffer, cx: i32, cy: i32, x: i32, y: i32, alpha: f32) {
    const HI: Rgb = (126, 168, 174);
    const LO: Rgb = (88, 128, 136);
    if x == 0 && y == 0 {
        blend_px(pb, cx, cy, HI, alpha);
    } else if x == 0 {
        blend_px(pb, cx, cy - y, HI, alpha);
        blend_px(pb, cx, cy + y, LO, alpha * 0.85);
    } else if y == 0 {
        blend_px(pb, cx - x, cy, HI, alpha * 0.95);
        blend_px(pb, cx + x, cy, LO, alpha * 0.85);
    } else {
        blend_px(pb, cx - x, cy - y, HI, alpha);
        blend_px(pb, cx + x, cy - y, HI, alpha * 0.90);
        blend_px(pb, cx - x, cy + y, LO, alpha * 0.88);
        blend_px(pb, cx + x, cy + y, LO, alpha * 0.78);
    }
}

/// A ripple's ring radii `(rx, ry)` in px at `age` seconds, or `None` once it
/// has died: the ellipse [`draw_ripple`] draws, shared with the lighting's
/// water stage so its glints ride the same ring.
pub fn ripple_radii(age: f32, big: bool) -> Option<(f32, f32)> {
    if !age.is_finite() || age < 0.0 || age >= RIPPLE_LIFE {
        return None;
    }
    Some(ripple_radii_at((age / RIPPLE_LIFE).clamp(0.0, 1.0), big))
}

#[inline]
fn ripple_radii_at(t: f32, big: bool) -> (f32, f32) {
    let ease = 1.0 - (1.0 - t) * (1.0 - t);
    let rx = if big {
        4.0 + 12.0 * ease
    } else {
        2.0 + 7.0 * ease
    };
    (rx, (rx * 0.5).max(1.0))
}

/// Paint an expanding 1-px elliptical water ripple (2:1 wider than tall) over
/// [`RIPPLE_LIFE`] seconds (0.45 s), fading as it expands from radius 2→9 px
/// (`big`: 4→16 px plus three tiny arcing splash droplets at the start).
pub fn draw_ripple(pb: &mut PixelBuffer, pos_screen: (i32, i32), age: f32, big: bool, scale: f32) {
    if pb.w == 0 || pb.h == 0 || !age.is_finite() || age < 0.0 || age >= RIPPLE_LIFE {
        return;
    }
    let (cx, cy) = pos_screen;
    let t = (age / RIPPLE_LIFE).clamp(0.0, 1.0);
    let (rx, ry) = ripple_radii_at(t, big);
    let base_alpha = (1.0 - t) * (1.0 - 0.30 * t) * if big { 0.68 } else { 0.56 };
    if base_alpha <= 0.01 {
        return;
    }

    // Integer midpoint ellipse guarantees a continuous 1-px ring with no
    // double-blended pixels and zero heap allocation.
    let a_r = ((rx * scale).round() as i32).max(1);
    let b_r = ((ry * scale).round() as i32).max(1);
    let a2 = i64::from(a_r) * i64::from(a_r);
    let b2 = i64::from(b_r) * i64::from(b_r);

    let mut x: i64 = 0;
    let mut y: i64 = i64::from(b_r);
    let mut px_d: i64 = 0;
    let mut py_d: i64 = 2 * a2 * y;
    emit_ripple_quad(pb, cx, cy, x as i32, y as i32, base_alpha);

    let mut p1: i64 = b2 - a2 * y + (a2 + 2) / 4;
    while px_d < py_d {
        x += 1;
        px_d += 2 * b2;
        if p1 < 0 {
            p1 += b2 + px_d;
        } else {
            y -= 1;
            py_d -= 2 * a2;
            p1 += b2 + px_d - py_d;
        }
        emit_ripple_quad(pb, cx, cy, x as i32, y as i32, base_alpha);
    }

    let mut p2: i64 =
        (b2 * (2 * x + 1) * (2 * x + 1) + 2) / 4 + a2 * (y - 1) * (y - 1) - a2 * b2;
    while y > 0 {
        y -= 1;
        py_d -= 2 * a2;
        if p2 > 0 {
            p2 += a2 - py_d;
        } else {
            x += 1;
            px_d += 2 * b2;
            p2 += a2 - py_d + px_d;
        }
        emit_ripple_quad(pb, cx, cy, x as i32, y as i32, base_alpha);
    }

    // Three tiny splash droplets popping upward and outward at the start of a
    // heavy splash (dash through water or heavy landing).
    if big && t < 0.45 {
        let u = t / 0.45;
        let drop_alpha = (1.0 - u) * 0.80;
        const DROP_COL: Rgb = (146, 186, 192);
        let af = a_r as f32;
        let bf = b_r as f32;
        let droplets = [
            (
                -(af * 0.75 + scale * (2.0 + 4.0 * u)).round() as i32,
                -(bf + scale * (1.0 + 4.0 * u - 5.5 * u * u)).round() as i32,
            ),
            (
                (af * 0.75 + scale * (2.0 + 4.5 * u)).round() as i32,
                -(bf + scale * (0.5 + 4.5 * u - 6.0 * u * u)).round() as i32,
            ),
            (
                -(scale * (1.0 + 2.5 * u)).round() as i32,
                -(bf + scale * (2.0 + 3.5 * u - 5.0 * u * u)).round() as i32,
            ),
        ];
        for (dx, dy) in droplets {
            blend_px(pb, cx + dx, cy + dy, DROP_COL, drop_alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_puddle() -> Puddle {
        Puddle {
            pos: Vec2::new(70.0, 48.0),
            rx: 34.0,
            ry: 20.0,
            seed: 42,
        }
    }

    #[test]
    fn draw_puddle_changes_pixels_inside_and_none_outside_bounds() {
        let puddle = sample_puddle();
        let floor_col: Rgb = (22, 32, 30);
        for theme in [Theme::Sewer, Theme::Laundry, Theme::Kitchen, Theme::Outfall] {
            let mut pb = PixelBuffer::new(160, 100);
            pb.clear(floor_col);
            draw_puddle(&mut pb, &puddle, Vec2::ZERO, 1.0, theme);

            let min_x = (puddle.pos.x - puddle.rx).floor() as usize;
            let max_x = (puddle.pos.x + puddle.rx).ceil() as usize;
            let min_y = (puddle.pos.y - puddle.ry).floor() as usize;
            let max_y = (puddle.pos.y + puddle.ry).ceil() as usize;

            let mut changed_inside = 0usize;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let changed = pb.pixel_at(x, y) != floor_col;
                    let in_bbox = x >= min_x && x <= max_x && y >= min_y && y <= max_y;
                    if !in_bbox {
                        assert!(
                            !changed,
                            "{theme:?}: pixel ({x},{y}) outside puddle bbox changed"
                        );
                    }
                    let wp = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if puddle.contains(wp) {
                        if changed {
                            changed_inside += 1;
                        }
                    } else {
                        assert!(
                            !changed,
                            "{theme:?}: pixel ({x},{y}) outside puddle blob changed"
                        );
                    }
                }
            }
            assert!(
                changed_inside > 100,
                "{theme:?}: expected puddle to paint pixels inside, got {changed_inside}"
            );
        }
    }

    #[test]
    fn the_plain_sheet_is_identical_across_elapsed_times() {
        let puddle = sample_puddle();
        let floor_col: Rgb = (18, 28, 26);
        let draw = |elapsed| {
            let mut pb = PixelBuffer::new(160, 100);
            pb.clear(floor_col);
            draw_puddle(&mut pb, &puddle, Vec2::ZERO, elapsed, Theme::Sewer);
            pb
        };
        let first = draw(0.0);
        for elapsed in [0.25, 0.75, 1.5, 3.75] {
            assert_eq!(first.pixels(), draw(elapsed).pixels(), "plain at {elapsed}");
        }
    }

    #[test]
    fn the_glossy_sheet_is_darker_and_stays_still() {
        let puddle = sample_puddle();
        let floor_col: Rgb = (60, 66, 64);
        let draw = |glossy: bool, elapsed: f32| {
            let mut pb = PixelBuffer::new(160, 100);
            pb.clear(floor_col);
            draw_puddle_among_with(&mut pb, &puddle, &[], Vec2::ZERO, elapsed, Theme::Sewer, glossy);
            pb
        };
        let plain = draw(false, 1.0);
        let glossy = draw(true, 1.0);
        let centre = (puddle.pos.x as usize, puddle.pos.y as usize);
        let lum = |c: Rgb| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        assert!(lum(glossy.pixel_at(centre.0, centre.1)) < lum(plain.pixel_at(centre.0, centre.1)));
        for elapsed in [1.25, 1.5, 2.0, 3.75] {
            assert_eq!(glossy.pixels(), draw(true, elapsed).pixels(), "glossy at {elapsed}");
        }
    }

    #[test]
    fn ripple_radii_match_the_drawn_ring_and_end_with_its_life() {
        assert_eq!(ripple_radii(0.0, false), Some((2.0, 1.0)));
        assert_eq!(ripple_radii(0.0, true), Some((4.0, 2.0)));
        assert_eq!(ripple_radii(RIPPLE_LIFE, false), None);
        assert_eq!(ripple_radii(-0.1, true), None);
    }

    #[test]
    fn draw_ripple_paints_at_0_1_and_nothing_at_0_5() {
        let base: Rgb = (12, 26, 26);
        for big in [false, true] {
            let mut pb_active = PixelBuffer::new(80, 60);
            pb_active.clear(base);
            draw_ripple(&mut pb_active, (40, 30), 0.1, big, 1.0);
            let painted = pb_active.pixels().iter().filter(|&&c| c != base).count();
            assert!(
                painted > 0,
                "ripple (big={big}) should paint pixels at age 0.1"
            );

            let mut pb_expired = PixelBuffer::new(80, 60);
            pb_expired.clear(base);
            draw_ripple(&mut pb_expired, (40, 30), 0.5, big, 1.0);
            let painted_expired = pb_expired.pixels().iter().filter(|&&c| c != base).count();
            assert_eq!(
                painted_expired, 0,
                "ripple (big={big}) must paint nothing at age 0.5"
            );
        }
    }
}
