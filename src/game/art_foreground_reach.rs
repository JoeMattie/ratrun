//! Art for foreground items that reach in from off screen (`fore-reach-contract`).
//!
//! Each reach item is a near-camera silhouette anchored beyond one viewport edge:
//! the `x <= 0` stem tiles seamlessly forever (period [`STEM_PERIOD`] = 16 art px)
//! and transitions smoothly into a free tip ending at `x = spec.tip_len`.
//!
//! ## Contract
//!
//! ```ignore
//! pub enum ReachKind { TreeBranch, LightPole, SignArm, CraneHook, HangingLamp, VineTendril, KelpFrond, PipeElbow }
//! pub struct ReachSpec { pub tip_len: f32, pub thickness: f32, pub organic: bool, pub floors: &'static [Theme] }
//! pub fn reach_spec(kind: ReachKind) -> ReachSpec;
//! pub fn draw_reach(kind: ReachKind, body: &mut PixelBuffer, shadow: &mut PixelBuffer, len: i32, t: f32, theme: Theme);
//! ```
//!
//! ## Raster Conventions
//!
//! * Local coordinates: `x` runs from the off-screen anchor (`x <= 0`) toward the
//!   tip at `x = spec.tip_len`; `y` runs across the item with `0.0` on the centre
//!   line (`|y| <= spec.thickness * 0.5`).
//! * `len` local art pixels are rasterised into `px in 0..len` (clipped to the
//!   buffer width), mapping `px` to `local_x = px as f32 + 0.5 + (spec.tip_len - len as f32)`
//!   so `px = len - 1` sits at the tip (`local_x = spec.tip_len - 0.5`) and any
//!   extra columns `px >= len` remain empty `(0, 0, 0)` for blur padding.
//! * `body` receives full un-premultiplied silhouette colours (empty pixels stay
//!   `(0, 0, 0)`) and material pens via [`PixelBuffer::set_pen`] (warm lamp lenses
//!   on [`ReachKind::LightPole`] and [`ReachKind::HangingLamp`] stamp
//!   [`EMISSIVE_PEN`] = `14`, matching `MaterialKind::Emissive`).
//! * `shadow` receives a soft grayscale shadow mask `(s, s, s)` with pen `0`
//!   (`0` = transparent, `255` = full shadow weight).
//! * Pure render-only drawing: zero heap allocations, zero RNG, and smooth C1
//!   cantilever sway in `t` for organic kinds (anchored rigid at `x <= 0` so the
//!   off-screen stem always tiles seamlessly).

use std::f32::consts::TAU;

use crate::game::level::Theme;
use crate::render::foreground::rim_tint;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Exact seamless repeat period of the `x <= 0` anchor stem in local art pixels.
/// A power of two so `pos_mod(x, 16.0)` is IEEE-754 exact for every half-integer
/// sample coordinate.
pub const STEM_PERIOD: i32 = 16;

/// Material pen byte for `MaterialKind::Emissive` (`14`, non-creature, flat) in
/// `crate::render::lighting::material`, stamped on warm lamp lenses and bulbs.
pub const EMISSIVE_PEN: u8 = 14;

/// Material pen byte for `MaterialKind::Metal` (`5`).
const METAL_PEN: u8 = 5;
/// Material pen byte for `MaterialKind::Wood` (`4`).
const WOOD_PEN: u8 = 4;
/// Material pen byte for matte organic foliage (`0` = default).
const FOLIAGE_PEN: u8 = 0;

/// How much lighter than the body its rim highlight is (~38 %, matching bands).
const RIM_LIGHTEN: f32 = 1.38;
/// Underside / far-edge shade as a share of the body colour.
const DEEP_SHADE: f32 = 0.68;
/// Lit-side face colour as a share of the body colour.
const SOFT_LIFT: f32 = 1.18;
/// Share of the floor's ambient character tint mixed into the rim.
const RIM_TINT_MIX: f32 = 0.16;

/// Foreground item kinds that reach into the view from off screen and end in a
/// free tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReachKind {
    TreeBranch,
    LightPole,
    SignArm,
    CraneHook,
    HangingLamp,
    VineTendril,
    KelpFrond,
    PipeElbow,
}

impl ReachKind {
    /// Every reach kind in declaration order.
    pub const fn all() -> [Self; 8] {
        [
            Self::TreeBranch,
            Self::LightPole,
            Self::SignArm,
            Self::CraneHook,
            Self::HangingLamp,
            Self::VineTendril,
            Self::KelpFrond,
            Self::PipeElbow,
        ]
    }
}

/// Geometry, motion, and floor eligibility for a [`ReachKind`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReachSpec {
    pub tip_len: f32,
    pub thickness: f32,
    pub organic: bool,
    pub floors: &'static [Theme],
}

const FLOORS_TREE_BRANCH: &[Theme] = &[Theme::Trailer, Theme::Kitchen];
const FLOORS_TRAILER_ONLY: &[Theme] = &[Theme::Trailer];
const FLOORS_CRANE_HOOK: &[Theme] = &[Theme::Printworks, Theme::Lab, Theme::Sewer];
const FLOORS_HANGING_LAMP: &[Theme] = &[Theme::Kitchen, Theme::Laundry, Theme::Lab];
const FLOORS_SEWER_ONLY: &[Theme] = &[Theme::Sewer];
const FLOORS_OUTFALL_ONLY: &[Theme] = &[Theme::Outfall];
const FLOORS_PIPE_ELBOW: &[Theme] = &[Theme::Sewer, Theme::Lab, Theme::Printworks];

/// Static placement and sizing specification for `kind` in local art pixels.
pub fn reach_spec(kind: ReachKind) -> ReachSpec {
    match kind {
        ReachKind::TreeBranch => ReachSpec {
            tip_len: 30.0,
            thickness: 20.0,
            organic: true,
            floors: FLOORS_TREE_BRANCH,
        },
        ReachKind::LightPole => ReachSpec {
            tip_len: 26.0,
            thickness: 18.0,
            organic: false,
            floors: FLOORS_TRAILER_ONLY,
        },
        ReachKind::SignArm => ReachSpec {
            tip_len: 28.0,
            thickness: 20.0,
            organic: false,
            floors: FLOORS_TRAILER_ONLY,
        },
        ReachKind::CraneHook => ReachSpec {
            tip_len: 24.0,
            thickness: 16.0,
            organic: false,
            floors: FLOORS_CRANE_HOOK,
        },
        ReachKind::HangingLamp => ReachSpec {
            tip_len: 20.0,
            thickness: 18.0,
            organic: false,
            floors: FLOORS_HANGING_LAMP,
        },
        ReachKind::VineTendril => ReachSpec {
            tip_len: 28.0,
            thickness: 16.0,
            organic: true,
            floors: FLOORS_SEWER_ONLY,
        },
        ReachKind::KelpFrond => ReachSpec {
            tip_len: 32.0,
            thickness: 16.0,
            organic: true,
            floors: FLOORS_OUTFALL_ONLY,
        },
        ReachKind::PipeElbow => ReachSpec {
            tip_len: 24.0,
            thickness: 18.0,
            organic: false,
            floors: FLOORS_PIPE_ELBOW,
        },
    }
}

/// Rasterise the item in its LOCAL frame: `x` runs from the anchor side
/// (`x <= 0` continues forever via a repeating stem/trunk/pole/cable/pipe
/// segment) toward the tip at `x = spec.tip_len`; `y` runs across the item
/// with `0` on the centre line.
///
/// `body` and `shadow` are the kept rasters the foreground layer blurs and
/// composites (same conventions as `art_foreground`'s bands: 2x/3x pixel
/// blocks, body + shadow, pens). `len` is how many local px to draw from the
/// screen edge to the tip; `t` drives smooth sway for organic kinds.
pub fn draw_reach(
    kind: ReachKind,
    body: &mut PixelBuffer,
    shadow: &mut PixelBuffer,
    len: i32,
    t: f32,
    theme: Theme,
) {
    if len <= 0 || (body.w == 0 && shadow.w == 0) || (body.h == 0 && shadow.h == 0) {
        return;
    }
    let spec = reach_spec(kind);
    let pal = ReachPal::for_theme(theme);
    let prev_body_pen = body.pen();
    let prev_shadow_pen = shadow.pen();
    shadow.set_pen(0);

    {
        let mut canvas = ReachCanvas::new(body, shadow, spec, len);
        match kind {
            ReachKind::TreeBranch => paint_tree_branch(&mut canvas, &pal, t),
            ReachKind::LightPole => paint_light_pole(&mut canvas, &pal),
            ReachKind::SignArm => paint_sign_arm(&mut canvas, &pal),
            ReachKind::CraneHook => paint_crane_hook(&mut canvas, &pal),
            ReachKind::HangingLamp => paint_hanging_lamp(&mut canvas, &pal),
            ReachKind::VineTendril => paint_vine_tendril(&mut canvas, &pal, t),
            ReachKind::KelpFrond => paint_kelp_frond(&mut canvas, &pal, t),
            ReachKind::PipeElbow => paint_pipe_elbow(&mut canvas, &pal),
        }
    }

    body.set_pen(prev_body_pen);
    shadow.set_pen(prev_shadow_pen);
}

// ---- Per-floor palette -----------------------------------------------------

#[derive(Clone, Copy)]
struct ReachPal {
    metal_deep: Rgb,
    metal_mid: Rgb,
    metal_soft: Rgb,
    metal_rim: Rgb,
    accent: Rgb,
    pale: Rgb,
    leaf_deep: Rgb,
    leaf_mid: Rgb,
    leaf_soft: Rgb,
    leaf_accent: Rgb,
    leaf_rim: Rgb,
    bark_deep: Rgb,
    bark_mid: Rgb,
    bark_rim: Rgb,
    lamp_core: Rgb,
    lamp_halo: Rgb,
    sign_face: Rgb,
}

impl ReachPal {
    fn for_theme(theme: Theme) -> Self {
        let rt = rim_tint(theme);
        let peak = rt.0.max(rt.1).max(rt.2).max(1) as f32;
        let gain = (195.0 / peak).clamp(1.0, 2.6);
        let tint = palette::scale(rt, gain);
        let rim_of = |c: Rgb| palette::mix(palette::scale(c, RIM_LIGHTEN), tint, RIM_TINT_MIX);

        let (
            metal,
            accent,
            pale,
            leaf_deep,
            leaf_mid,
            leaf_soft,
            leaf_accent,
            bark_deep,
            bark_mid,
            bark_rim,
            lamp_core,
            lamp_halo,
            sign_face,
        ) = match theme {
            // Damp iron, rust, and mossy sewer foliage.
            Theme::Sewer => (
                (64, 82, 78),
                (156, 86, 48),
                (168, 190, 182),
                (26, 54, 42),
                (46, 86, 60),
                (68, 114, 78),
                (94, 142, 88),
                (36, 38, 32),
                (62, 64, 52),
                (96, 98, 82),
                (255, 224, 134),
                (232, 160, 60),
                (38, 66, 60),
            ),
            // Warm brushed steel, copper, incandescent amber, and window-herb greens.
            Theme::Kitchen => (
                (106, 92, 78),
                (192, 114, 58),
                (236, 202, 134),
                (40, 68, 38),
                (68, 104, 54),
                (92, 130, 68),
                (124, 152, 80),
                (52, 38, 28),
                (88, 66, 46),
                (126, 102, 76),
                (255, 232, 152),
                (242, 178, 76),
                (62, 54, 46),
            ),
            // Cold steel-blue, clinical cyan, and cool-warm lab lamp glow.
            Theme::Lab => (
                (74, 92, 118),
                (82, 174, 204),
                (178, 208, 224),
                (32, 66, 58),
                (54, 102, 86),
                (78, 132, 112),
                (108, 164, 140),
                (52, 56, 68),
                (86, 92, 108),
                (124, 132, 150),
                (232, 246, 224),
                (176, 224, 198),
                (38, 56, 82),
            ),
            // Pale enamel, warm brass/wood, and soft cream bulb.
            Theme::Laundry => (
                (114, 122, 138),
                (166, 126, 84),
                (210, 216, 224),
                (40, 74, 62),
                (66, 108, 88),
                (90, 136, 112),
                (122, 162, 136),
                (62, 54, 48),
                (98, 84, 72),
                (138, 122, 106),
                (250, 238, 192),
                (228, 202, 136),
                (54, 72, 92),
            ),
            // Oiled steel, registration cyan, cream newsprint.
            Theme::Printworks => (
                (92, 82, 90),
                (74, 162, 172),
                (202, 198, 184),
                (36, 62, 52),
                (58, 92, 74),
                (82, 118, 96),
                (112, 146, 120),
                (42, 38, 44),
                (72, 64, 72),
                (108, 98, 106),
                (252, 228, 164),
                (228, 176, 92),
                (46, 54, 64),
            ),
            // Flatbed outdoors: galvanized highway steel, warm amber sodium lens,
            // highway sign green, roadside oak bark & two-tone leaf canopy.
            Theme::Trailer => (
                (78, 88, 100),
                (212, 162, 68),
                (182, 194, 206),
                (34, 72, 38),
                (66, 120, 56),
                (82, 142, 66),
                (114, 162, 72),
                (46, 34, 24),
                (96, 76, 56),
                (138, 118, 92),
                (255, 226, 132),
                (240, 166, 58),
                (34, 82, 58),
            ),
            // Outfall underwater: deep bull-kelp teal-greens and pale sea-foam.
            Theme::Outfall => (
                (58, 78, 78),
                (84, 156, 122),
                (158, 224, 232),
                (16, 46, 40),
                (32, 86, 68),
                (52, 118, 92),
                (84, 156, 122),
                (32, 24, 18),
                (66, 52, 40),
                (106, 88, 70),
                (232, 244, 186),
                (166, 214, 148),
                (28, 62, 64),
            ),
        };

        Self {
            metal_deep: palette::scale(metal, DEEP_SHADE),
            metal_mid: metal,
            metal_soft: palette::scale(metal, SOFT_LIFT),
            metal_rim: rim_of(metal),
            accent,
            pale,
            leaf_deep,
            leaf_mid,
            leaf_soft,
            leaf_accent,
            leaf_rim: rim_of(leaf_mid),
            bark_deep,
            bark_mid,
            bark_rim,
            lamp_core,
            lamp_halo,
            sign_face,
        }
    }
}

// ---- Math & raster helpers -------------------------------------------------

#[inline]
fn pos_mod(x: f32, m: f32) -> f32 {
    x - (x / m).floor() * m
}

#[inline]
fn soft_u8(dist_in: f32, feather: f32, max_w: u8) -> u8 {
    if dist_in <= 0.0 {
        0
    } else if feather <= 0.0 || dist_in >= feather {
        max_w
    } else {
        ((dist_in / feather) * max_w as f32 + 0.5) as u8
    }
}

#[derive(Clone, Copy)]
struct PaintStyle {
    dark: Rgb,
    mid: Rgb,
    rim: Option<Rgb>,
    pen: u8,
    sh_w: u8,
}

struct ReachCanvas<'a> {
    body: &'a mut PixelBuffer,
    shadow: &'a mut PixelBuffer,
    tip_len: f32,
    half_t: f32,
    draw_len: i32,
    h: i32,
    x_origin: f32,
    cy: f32,
}

impl<'a> ReachCanvas<'a> {
    fn new(
        body: &'a mut PixelBuffer,
        shadow: &'a mut PixelBuffer,
        spec: ReachSpec,
        len: i32,
    ) -> Self {
        let max_w = body.w.max(shadow.w) as i32;
        let max_h = body.h.max(shadow.h) as i32;
        let draw_len = len.min(max_w);
        Self {
            body,
            shadow,
            tip_len: spec.tip_len,
            half_t: spec.thickness * 0.5,
            draw_len,
            h: max_h,
            x_origin: spec.tip_len - len as f32,
            cy: max_h as f32 * 0.5,
        }
    }

    #[inline]
    fn lx(&self, px: i32) -> f32 {
        (px as f32 + 0.5) + self.x_origin
    }

    #[inline]
    fn ly(&self, py: i32) -> f32 {
        (py as f32 + 0.5) - self.cy
    }

    fn px_range(&self, mut x0: f32, mut x1: f32, tip_only: bool) -> Option<(i32, i32)> {
        if tip_only {
            x0 = x0.max(0.0);
        }
        x1 = x1.min(self.tip_len);
        if x1 <= x0 || self.draw_len <= 0 {
            return None;
        }
        let lo = ((x0 - self.x_origin).floor() as i32).max(0);
        let hi = ((x1 - self.x_origin).ceil() as i32).min(self.draw_len - 1);
        (lo <= hi).then_some((lo, hi))
    }

    fn py_range(&self, y0: f32, y1: f32) -> Option<(i32, i32)> {
        let y0 = y0.max(-self.half_t);
        let y1 = y1.min(self.half_t);
        if y1 <= y0 || self.h <= 0 {
            return None;
        }
        let lo = ((y0 + self.cy).floor() as i32).max(0);
        let hi = ((y1 + self.cy).ceil() as i32).min(self.h - 1);
        (lo <= hi).then_some((lo, hi))
    }

    #[inline]
    fn put_body(&mut self, px: i32, py: i32, rgb: Rgb, pen: u8) {
        if px < 0 || py < 0 || px >= self.draw_len || px >= self.body.w as i32 || py >= self.body.h as i32 {
            return;
        }
        let lx = self.lx(px);
        let ly = self.ly(py);
        if lx > self.tip_len || ly.abs() > self.half_t {
            return;
        }
        self.body.set_pen(pen);
        self.body.plot(px, py, rgb);
    }

    #[inline]
    fn put_shadow(&mut self, px: i32, py: i32, s: u8) {
        if s == 0 || px < 0 || py < 0 || px >= self.draw_len || px >= self.shadow.w as i32 || py >= self.shadow.h as i32 {
            return;
        }
        let lx = self.lx(px);
        let ly = self.ly(py);
        if lx > self.tip_len || ly.abs() > self.half_t {
            return;
        }
        let cur = self.shadow.pixel_at(px as usize, py as usize).0;
        if s > cur {
            self.shadow.plot(px, py, (s, s, s));
        }
    }

    fn fill_capsule(&mut self, cap: [f32; 6], style: PaintStyle, tip_only: bool) {
        let [ax, ay, ra, bx, by, rb] = cap;
        let rmax = ra.max(rb);
        let Some((x_lo, x_hi)) = self.px_range(
            ax.min(bx) - rmax - 1.0,
            ax.max(bx) + rmax + 1.0,
            tip_only,
        ) else {
            return;
        };
        let Some((y_lo, y_hi)) = self.py_range(ay.min(by) - rmax - 1.0, ay.max(by) + rmax + 1.0)
        else {
            return;
        };
        let dx = bx - ax;
        let dy = by - ay;
        let l2 = dx * dx + dy * dy;
        let inv_l2 = if l2 > 1e-5 { 1.0 / l2 } else { 0.0 };
        for py in y_lo..=y_hi {
            let y = self.ly(py);
            for px in x_lo..=x_hi {
                let x = self.lx(px);
                if tip_only && x <= 0.0 {
                    continue;
                }
                let u = (((x - ax) * dx + (y - ay) * dy) * inv_l2).clamp(0.0, 1.0);
                let cx = ax + dx * u;
                let cy = ay + dy * u;
                let r = ra + (rb - ra) * u;
                let dist = ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt();
                let d_in = r - dist;
                if d_in <= 0.0 {
                    continue;
                }
                self.put_shadow(px, py, soft_u8(d_in, 1.5, style.sh_w));
                let side = -(y - cy) / r.max(0.5);
                let col = if let Some(rim) = style.rim {
                    if d_in < 0.95 && side > 0.15 {
                        rim
                    } else if side < -0.25 {
                        style.dark
                    } else {
                        palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                    }
                } else if side < -0.25 {
                    style.dark
                } else {
                    palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                };
                self.put_body(px, py, col, style.pen);
            }
        }
    }

    fn fill_ellipse(&mut self, e: [f32; 4], style: PaintStyle, tip_only: bool) {
        let [cx, cy, rx, ry] = e;
        if rx <= 0.1 || ry <= 0.1 {
            return;
        }
        let Some((x_lo, x_hi)) = self.px_range(cx - rx - 1.0, cx + rx + 1.0, tip_only) else {
            return;
        };
        let Some((y_lo, y_hi)) = self.py_range(cy - ry - 1.0, cy + ry + 1.0) else {
            return;
        };
        let r_eff = rx.min(ry);
        for py in y_lo..=y_hi {
            let y = self.ly(py);
            let ny = (y - cy) / ry;
            let ny2 = ny * ny;
            if ny2 >= 1.0 {
                continue;
            }
            for px in x_lo..=x_hi {
                let x = self.lx(px);
                if tip_only && x <= 0.0 {
                    continue;
                }
                let nx = (x - cx) / rx;
                let q = nx * nx + ny2;
                if q >= 1.0 {
                    continue;
                }
                let d_in = (1.0 - q.sqrt()) * r_eff;
                self.put_shadow(px, py, soft_u8(d_in, 1.6, style.sh_w));
                let side = -ny;
                let col = if let Some(rim) = style.rim {
                    if d_in < 0.95 && side > 0.15 {
                        rim
                    } else if side < -0.25 {
                        style.dark
                    } else {
                        palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                    }
                } else if side < -0.25 {
                    style.dark
                } else {
                    palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                };
                self.put_body(px, py, col, style.pen);
            }
        }
    }

    fn fill_rect(&mut self, r: [f32; 4], style: PaintStyle, tip_only: bool) {
        let [x0, y0, x1, y1] = r;
        let Some((x_lo, x_hi)) = self.px_range(x0, x1, tip_only) else {
            return;
        };
        let Some((y_lo, y_hi)) = self.py_range(y0, y1) else {
            return;
        };
        for py in y_lo..=y_hi {
            let y = self.ly(py);
            let dy_lo = y - y0;
            let dy_hi = y1 - y;
            let dy_in = dy_lo.min(dy_hi);
            if dy_in <= 0.0 {
                continue;
            }
            for px in x_lo..=x_hi {
                let x = self.lx(px);
                if tip_only && x <= 0.0 {
                    continue;
                }
                let d_in = (x - x0).min(x1 - x).min(dy_in);
                if d_in <= 0.0 {
                    continue;
                }
                self.put_shadow(px, py, soft_u8(d_in, 1.4, style.sh_w));
                let col = if let Some(rim) = style.rim {
                    if dy_lo < 1.0 {
                        rim
                    } else if dy_hi < 1.1 {
                        style.dark
                    } else {
                        style.mid
                    }
                } else if dy_hi < 1.1 {
                    style.dark
                } else {
                    style.mid
                };
                self.put_body(px, py, col, style.pen);
            }
        }
    }
}

// ---- 1. TreeBranch (Flatbed + Kitchen window plant, organic) ---------------

fn paint_tree_branch(c: &mut ReachCanvas, pal: &ReachPal, t: f32) {
    // 1. Seamless x <= 0 gnarled bark bough (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-5.5, 5.5))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let ang = u * (TAU / STEM_PERIOD as f32);
            let bv = 0.65 * ang.sin();
            let knot = 0.45 * (1.0 - (u - 8.0).abs() * 0.45).max(0.0);
            let br = 2.55 + 0.30 * ang.cos() + knot;
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = br - (y - bv).abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.5, 230));
                let side = -(y - bv) / br.max(0.5);
                let grain = (u * (TAU / 8.0) + y * 1.4).sin();
                let col = if d_in < 0.9 && side > 0.15 {
                    pal.bark_rim
                } else if side < -0.25 || grain < -0.68 {
                    pal.bark_deep
                } else {
                    pal.bark_mid
                };
                c.put_body(px, py, col, WOOD_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 30.0): asymmetric forked branch + twigs + 3 natural leaf sprays.
    let sway = |x: f32| -> f32 {
        let s = (x / 30.0).clamp(0.0, 1.0);
        (s * s) * (1.8 * (t * 1.4).sin() + 0.4 * (t * 2.3 + 1.1).sin())
    };
    let pt = |x: f32, y: f32| -> (f32, f32) { (x, y + sway(x)) };

    let p0 = pt(0.0, 0.0);
    let p1 = pt(6.0, 0.3);
    let p2 = pt(13.0, -0.3);
    let p3 = pt(21.0, 0.2);
    let p_tip = pt(26.8, -0.4);
    let p_u1 = pt(12.0, -4.4);
    let p_u2 = pt(17.5, -6.0);
    let p_d1 = pt(17.5, 4.6);
    let p_d2 = pt(23.5, 5.6);
    let p_ur = pt(24.5, -4.2);

    let bark = PaintStyle {
        dark: pal.bark_deep,
        mid: pal.bark_mid,
        rim: Some(pal.bark_rim),
        pen: WOOD_PEN,
        sh_w: 230,
    };
    let limbs = [
        [p1.0, p1.1, 1.55, p_u1.0, p_u1.1, 1.05],
        [p_u1.0, p_u1.1, 1.05, p_u2.0, p_u2.1, 0.70],
        [p2.0, p2.1, 1.45, p_d1.0, p_d1.1, 1.00],
        [p_d1.0, p_d1.1, 1.00, p_d2.0, p_d2.1, 0.65],
        [p3.0, p3.1, 1.10, p_ur.0, p_ur.1, 0.65],
        [p3.0, p3.1, 1.25, p_tip.0, p_tip.1, 0.70],
        [p2.0, p2.1, 1.90, p3.0, p3.1, 1.25],
        [p1.0, p1.1, 2.35, p2.0, p2.1, 1.90],
        [p0.0, p0.1, 2.85, p1.0, p1.1, 2.35],
    ];
    for cap in limbs {
        c.fill_capsule(cap, bark, true);
    }

    // Three distinct lush leaf sprays with dappled sky gaps; rightmost cluster kisses x = 30.0.
    let clusters: [(f32, f32, f32, f32, bool); 10] = [
        // Upper-left spray
        (12.2, -5.2, 2.9, 2.2, false),
        (16.0, -6.2, 3.2, 2.3, true),
        (19.0, -5.4, 2.6, 2.1, true),
        // Lower spray
        (17.8, 5.2, 3.0, 2.2, false),
        (21.8, 5.8, 3.2, 2.4, false),
        (24.8, 4.8, 2.6, 2.0, true),
        // Terminal crown spray reaching x = 30.0
        (22.4, -1.6, 3.0, 2.4, false),
        (24.8, -3.8, 2.9, 2.3, true),
        (25.2, 1.2, 3.1, 2.4, false),
        (26.8, -0.6, 3.2, 2.5, true),
    ];
    for (cx, cy, rx, ry, sunlit) in clusters {
        let (px_c, py_c) = pt(cx, cy);
        let style = PaintStyle {
            dark: pal.leaf_deep,
            mid: if sunlit { pal.leaf_accent } else { pal.leaf_mid },
            rim: Some(pal.leaf_rim),
            pen: FOLIAGE_PEN,
            sh_w: 215,
        };
        c.fill_ellipse([px_c, py_c, rx, ry], style, true);
    }
}

// ---- 2. LightPole (Flatbed, rigid, emissive warm sodium lens) --------------

fn paint_light_pole(c: &mut ReachCanvas, pal: &ReachPal) {
    // 1. Seamless x <= 0 galvanized sectional steel mast (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-3.5, 3.5))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let du_c = (u - 8.0).abs();
            let on_collar = du_c < 1.15;
            let r = if on_collar { 2.95 } else { 2.25 };
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = r - y.abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.4, if on_collar { 235 } else { 220 }));
                let side = -y / r;
                let col = if on_collar && du_c < 0.45 {
                    pal.metal_deep
                } else if d_in < 0.85 && side > 0.2 {
                    pal.metal_rim
                } else if (side - 0.45).abs() < 0.22 {
                    palette::mix(pal.metal_soft, pal.pale, 0.45)
                } else if side > 0.1 {
                    pal.metal_soft
                } else if side < -0.25 {
                    pal.metal_deep
                } else {
                    pal.metal_mid
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 26.0): tapered pole arm + gusset strut + cobra-head hood + emissive lens.
    let metal = PaintStyle {
        dark: pal.metal_deep,
        mid: pal.metal_mid,
        rim: Some(pal.metal_rim),
        pen: METAL_PEN,
        sh_w: 225,
    };
    let metal_soft = PaintStyle {
        mid: pal.metal_soft,
        sh_w: 230,
        ..metal
    };
    // Diagonal gusset brace underneath the luminaire arm
    c.fill_capsule(
        [3.5, 1.8, 0.9, 13.8, 4.4, 0.85],
        PaintStyle { sh_w: 200, ..metal },
        true,
    );
    // Tapered arm shaft
    c.fill_capsule([0.0, 0.0, 2.25, 15.5, 0.0, 1.75], metal, true);
    // Knuckle collar
    c.fill_rect([12.2, -2.8, 14.8, 2.8], metal_soft, true);
    // Cobra-head lamp housing (reaches x = 26.0)
    c.fill_ellipse(
        [20.0, 0.0, 6.0, 6.4],
        PaintStyle { sh_w: 235, ..metal },
        true,
    );
    c.fill_rect(
        [14.2, -3.8, 19.5, 3.8],
        PaintStyle { sh_w: 230, ..metal },
        true,
    );
    // Cooling fin ribs on hood rear
    let fin = PaintStyle {
        dark: pal.metal_deep,
        mid: pal.metal_rim,
        rim: None,
        pen: METAL_PEN,
        sh_w: 220,
    };
    for fy in [-2.4_f32, 0.0, 2.4] {
        c.fill_rect([15.2, fy - 0.5, 17.6, fy + 0.5], fin, true);
    }
    // Warm sodium-vapor emissive lens (stamped with EMISSIVE_PEN)
    c.fill_ellipse(
        [20.6, 0.0, 4.1, 4.1],
        PaintStyle {
            dark: pal.lamp_halo,
            mid: pal.lamp_halo,
            rim: None,
            pen: EMISSIVE_PEN,
            sh_w: 130,
        },
        true,
    );
    c.fill_ellipse(
        [20.8, 0.0, 2.6, 2.6],
        PaintStyle {
            dark: pal.lamp_core,
            mid: pal.lamp_core,
            rim: None,
            pen: EMISSIVE_PEN,
            sh_w: 110,
        },
        true,
    );
}

// ---- 3. SignArm (Flatbed, rigid) -------------------------------------------

fn paint_sign_arm(c: &mut ReachCanvas, pal: &ReachPal) {
    // 1. Seamless x <= 0 twin-chord gantry truss (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-4.0, 4.0))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_top = 1.15 - (y + 2.5).abs();
                let d_bot = 1.15 - (y - 2.5).abs();
                let d_chord = d_top.max(d_bot);
                let d_post = (0.95 - (u - 8.0).abs()).min(3.4 - y.abs());
                let d_diag = if (1.8..=8.0).contains(&u) {
                    let tu = (u - 1.8) / 6.2;
                    let y_diag = -2.5 + 5.0 * tu;
                    0.80 - (y - y_diag).abs() * 0.72
                } else if u > 8.0 && u <= 14.2 {
                    let tu = (u - 8.0) / 6.2;
                    let y_diag = 2.5 - 5.0 * tu;
                    0.80 - (y - y_diag).abs() * 0.72
                } else {
                    -1.0
                };
                let d_in = d_chord.max(d_post).max(d_diag);
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.2, 220));
                let col = if d_chord >= d_post.max(d_diag) {
                    let cy_c = if d_top >= d_bot { -2.5 } else { 2.5 };
                    let side = -(y - cy_c) / 1.15;
                    if side > 0.35 {
                        pal.metal_rim
                    } else if side < -0.25 {
                        pal.metal_deep
                    } else {
                        pal.metal_mid
                    }
                } else if d_post >= d_diag {
                    if y < 0.0 {
                        pal.metal_soft
                    } else {
                        pal.metal_deep
                    }
                } else {
                    pal.metal_mid
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 28.0): truss lead-in -> mounting clamps -> highway sign board.
    let metal = PaintStyle {
        dark: pal.metal_deep,
        mid: pal.metal_mid,
        rim: Some(pal.metal_rim),
        pen: METAL_PEN,
        sh_w: 220,
    };
    c.fill_rect([0.0, -3.65, 9.5, -1.35], metal, true);
    c.fill_rect([0.0, 1.35, 9.5, 3.65], metal, true);
    c.fill_capsule(
        [1.5, -2.5, 0.75, 7.5, 2.5, 0.75],
        PaintStyle {
            rim: None,
            sh_w: 200,
            ..metal
        },
        true,
    );
    c.fill_rect(
        [6.8, -5.8, 9.5, 5.8],
        PaintStyle {
            mid: pal.metal_soft,
            sh_w: 230,
            ..metal
        },
        true,
    );

    // Chamfered highway sign plate (x in 9.0..=28.0, |y| <= 9.2)
    let sign_dark = palette::scale(pal.sign_face, 0.82);
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(9.0, 28.0, true), c.py_range(-9.2, 9.2))
    {
        for py in y_lo..=y_hi {
            let y = c.ly(py);
            let ay = y.abs();
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                if x <= 0.0 {
                    continue;
                }
                let dx_in = (x - 9.0).min(28.0 - x);
                let dy_in = 9.2 - ay;
                let d_in = dx_in.min(dy_in);
                if dx_in + dy_in < 1.8 || d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.5, 240));
                let col = if d_in < 1.1 {
                    if y < -6.0 {
                        pal.metal_rim
                    } else {
                        pal.metal_deep
                    }
                } else if d_in < 2.1 {
                    pal.pale
                } else {
                    let on_bar1 = (12.2..=20.5).contains(&x) && (-5.2..=-2.8).contains(&y);
                    let on_bar2 = (12.2..=18.8).contains(&x) && (2.6..=4.8).contains(&y);
                    let on_chev = (21.5..=25.6).contains(&x) && ay <= ((25.6 - x) * 1.15).max(0.0);
                    let on_stem = (19.6..=22.0).contains(&x) && ay <= 1.25;
                    if on_bar1 || on_bar2 {
                        pal.pale
                    } else if on_chev || on_stem {
                        pal.accent
                    } else if y < 2.0 {
                        pal.sign_face
                    } else {
                        sign_dark
                    }
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }
}

// ---- 4. CraneHook (Printworks / Lab / Sewer, rigid) ------------------------

fn paint_crane_hook(c: &mut ReachCanvas, pal: &ReachPal) {
    // 1. Seamless x <= 0 braided steel wire cable with periodic crimped ferrule (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-2.6, 2.6))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let on_ferrule = (u - 8.0).abs() < 1.3;
            let r = if on_ferrule { 2.15 } else { 1.55 };
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = r - y.abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.3, 215));
                let side = -y / r;
                let col = if on_ferrule {
                    if side > 0.25 {
                        pal.metal_rim
                    } else if side < -0.25 {
                        pal.metal_deep
                    } else {
                        pal.metal_soft
                    }
                } else {
                    let strand = (u * (TAU / 8.0) + y * 0.9).sin();
                    if strand > 0.45 && side > -0.2 {
                        pal.metal_rim
                    } else if strand < -0.45 || side < -0.35 {
                        pal.metal_deep
                    } else {
                        pal.metal_mid
                    }
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 24.0): shackle -> compact pulley cheek-plate -> open forged J-hook.
    let metal = PaintStyle {
        dark: pal.metal_deep,
        mid: pal.metal_mid,
        rim: Some(pal.metal_rim),
        pen: METAL_PEN,
        sh_w: 225,
    };
    c.fill_rect(
        [0.0, -1.55, 3.2, 1.55],
        PaintStyle { sh_w: 215, ..metal },
        true,
    );
    c.fill_rect(
        [1.8, -2.6, 4.5, 2.6],
        PaintStyle {
            mid: pal.metal_soft,
            ..metal
        },
        true,
    );
    // Compact pulley cheek-plate block
    c.fill_ellipse(
        [6.0, 0.0, 2.4, 4.6],
        PaintStyle { sh_w: 235, ..metal },
        true,
    );
    c.fill_rect(
        [5.0, -4.9, 11.6, 4.9],
        PaintStyle { sh_w: 240, ..metal },
        true,
    );
    let stripe = PaintStyle {
        dark: pal.accent,
        mid: pal.accent,
        rim: None,
        pen: METAL_PEN,
        sh_w: 235,
    };
    c.fill_rect([6.2, -3.9, 10.4, -2.4], stripe, true);
    c.fill_rect([6.2, 2.4, 10.4, 3.9], stripe, true);
    // Central axle pin
    c.fill_ellipse(
        [8.3, 0.0, 1.65, 1.65],
        PaintStyle {
            mid: pal.pale,
            sh_w: 235,
            ..metal
        },
        true,
    );

    // Forged open J-hook sweeping cleanly to x = 24.0 with wide open mouth on -y side.
    let hook_style = PaintStyle {
        mid: pal.metal_soft,
        sh_w: 230,
        ..metal
    };
    let hook_pts: [(f32, f32, f32); 8] = [
        (11.2, 0.0, 1.60),
        (14.6, 0.4, 1.55),
        (17.2, 4.4, 1.65),
        (20.8, 5.2, 1.70),
        (22.6, 1.6, 1.40), // 22.6 + 1.40 = 24.0
        (22.2, -3.2, 1.25),
        (19.6, -5.6, 1.05),
        (17.8, -5.0, 0.80),
    ];
    for pair in hook_pts.windows(2) {
        let a = pair[0];
        let b = pair[1];
        c.fill_capsule([a.0, a.1, a.2, b.0, b.1, b.2], hook_style, true);
    }
    // Short recessed safety latch nub (keeps the hook throat clearly open when blurred)
    c.fill_capsule(
        [14.5, -0.9, 0.55, 15.8, -2.5, 0.50],
        PaintStyle {
            dark: pal.accent,
            mid: pal.pale,
            rim: None,
            pen: METAL_PEN,
            sh_w: 160,
        },
        true,
    );
}

// ---- 5. HangingLamp (Kitchen / Laundry / Lab, rigid, emissive bulb) --------

fn paint_hanging_lamp(c: &mut ReachCanvas, pal: &ReachPal) {
    // 1. Seamless x <= 0 braided electrical cord (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-2.4, 2.4))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let on_bead = (u - 8.0).abs() < 0.85;
            let r = if on_bead { 1.60 } else { 1.25 };
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = r - y.abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.2, 195));
                let side = -y / r;
                let col = if on_bead {
                    if side > 0.0 {
                        pal.accent
                    } else {
                        pal.metal_deep
                    }
                } else {
                    let weave = (u * (TAU / 8.0) + y * 1.1).sin();
                    if side > 0.3 {
                        pal.metal_rim
                    } else if side < -0.35 || weave < -0.55 {
                        pal.metal_deep
                    } else {
                        pal.metal_mid
                    }
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 20.0): brass socket -> flared enamel dome -> emissive warm bulb.
    c.fill_rect(
        [0.0, -1.25, 3.2, 1.25],
        PaintStyle {
            dark: pal.metal_deep,
            mid: pal.metal_mid,
            rim: Some(pal.metal_rim),
            pen: METAL_PEN,
            sh_w: 195,
        },
        true,
    );
    c.fill_rect(
        [2.4, -2.6, 5.6, 2.6],
        PaintStyle {
            dark: pal.metal_deep,
            mid: pal.accent,
            rim: Some(pal.pale),
            pen: METAL_PEN,
            sh_w: 225,
        },
        true,
    );

    // Flared enamel dome shade (x in 5.0..17.4) with warm emissive reflector mouth
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(5.0, 17.4, true), c.py_range(-8.8, 8.8))
    {
        for py in y_lo..=y_hi {
            let y = c.ly(py);
            let ay = y.abs();
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                if x <= 0.0 {
                    continue;
                }
                let u = ((x - 5.0) / 12.2).clamp(0.0, 1.0);
                let hw = 2.6 + 6.0 * (1.0 - (1.0 - u) * (1.0 - u));
                let d_in = (hw - ay).min(17.4 - x);
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.6, 235));
                if x >= 15.2 && ay < hw - 1.1 {
                    c.put_body(px, py, pal.lamp_halo, EMISSIVE_PEN);
                } else {
                    let side = -y / hw.max(1.0);
                    let col = if d_in < 1.0 && side > 0.15 {
                        pal.metal_rim
                    } else if x >= 16.0 {
                        if side > 0.0 {
                            pal.metal_soft
                        } else {
                            pal.metal_deep
                        }
                    } else if (side - 0.35).abs() < 0.22 {
                        pal.metal_soft
                    } else if side < -0.25 {
                        pal.metal_deep
                    } else {
                        pal.metal_mid
                    };
                    c.put_body(px, py, col, METAL_PEN);
                }
            }
        }
    }

    // Protruding warm bulb reaching x = 20.0 exact (EMISSIVE_PEN)
    c.fill_ellipse(
        [17.2, 0.0, 2.8, 3.0],
        PaintStyle {
            dark: pal.lamp_halo,
            mid: pal.lamp_halo,
            rim: None,
            pen: EMISSIVE_PEN,
            sh_w: 130,
        },
        true,
    );
    c.fill_ellipse(
        [17.5, 0.0, 1.8, 1.9],
        PaintStyle {
            dark: pal.lamp_core,
            mid: pal.lamp_core,
            rim: None,
            pen: EMISSIVE_PEN,
            sh_w: 110,
        },
        true,
    );
}

// ---- 6. VineTendril (Sewer, organic) ---------------------------------------

fn paint_vine_tendril(c: &mut ReachCanvas, pal: &ReachPal, t: f32) {
    // 1. Seamless x <= 0 damp sewer vine stalk + alternating tucked leaves (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-6.5, 6.5))
    {
        let stem_leaves: [(f32, f32, f32, f32); 2] =
            [(4.8, -3.0, 2.0, 1.5), (11.2, 3.0, 2.0, 1.5)];
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let ang = u * (TAU / STEM_PERIOD as f32);
            let sv = 1.3 * ang.sin();
            let sr = 1.55 + 0.18 * ang.cos();
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = sr - (y - sv).abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.4, 215));
                let side = -(y - sv) / sr;
                let col = if d_in < 0.7 && side > 0.2 {
                    pal.leaf_rim
                } else if side < -0.25 {
                    pal.leaf_deep
                } else {
                    pal.leaf_mid
                };
                c.put_body(px, py, col, FOLIAGE_PEN);
            }
            for (lu, lv, lrx, lry) in stem_leaves {
                if (u - lu).abs() >= lrx {
                    continue;
                }
                let nx = (u - lu) / lrx;
                let nx2 = nx * nx;
                let r_eff = lrx.min(lry);
                for py in y_lo..=y_hi {
                    let y = c.ly(py);
                    let ny = (y - lv) / lry;
                    let q = nx2 + ny * ny;
                    if q >= 1.0 {
                        continue;
                    }
                    let d_in = (1.0 - q.sqrt()) * r_eff;
                    c.put_shadow(px, py, soft_u8(d_in, 1.3, 200));
                    let col = if ny < -0.15 {
                        pal.leaf_accent
                    } else if ny > 0.35 {
                        pal.leaf_deep
                    } else {
                        pal.leaf_soft
                    };
                    c.put_body(px, py, col, FOLIAGE_PEN);
                }
            }
        }
    }

    // 2. Tip (0 < x <= 28.0) with gentle smooth C1 sway and open spiral curl tip.
    let sway = |x: f32| -> f32 {
        let s = (x / 28.0).clamp(0.0, 1.0);
        (s * s) * (2.0 * (t * 1.55 + 0.4).sin() + 0.35 * (t * 2.7).sin())
    };
    let pt = |x: f32, y: f32| -> (f32, f32) { (x, y + sway(x)) };

    let stalk_style = PaintStyle {
        dark: pal.leaf_deep,
        mid: pal.leaf_mid,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 215,
    };
    let stalk_pts: [(f32, f32, f32); 11] = [
        (0.0, 0.0, 1.73),
        (4.5, 1.4, 1.58),
        (9.5, 0.5, 1.45),
        (14.5, -1.2, 1.30),
        (19.0, 0.8, 1.15),
        (23.2, 3.2, 1.02),
        (26.4, 1.8, 0.92),
        (27.15, -1.4, 0.85), // 27.15 + 0.85 = 28.0
        (25.2, -4.2, 0.78),
        (22.2, -4.0, 0.72),
        (21.4, -1.8, 0.65),
    ];
    for pair in stalk_pts.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let pa = pt(a.0, a.1);
        let pb = pt(b.0, b.1);
        c.fill_capsule([pa.0, pa.1, a.2, pb.0, pb.1, b.2], stalk_style, true);
    }

    // Bold alternating ivy leaves leaving the spiral curl wide open
    let leaves: [(f32, f32, f32, f32, bool); 4] = [
        (5.5, -3.6, 2.8, 2.0, true),
        (10.0, 3.8, 2.9, 2.1, false),
        (15.0, -4.2, 2.8, 2.1, true),
        (18.0, 4.0, 2.5, 1.9, false),
    ];
    for (lx, ly, rx, ry, hi) in leaves {
        let pl = pt(lx, ly);
        let style = PaintStyle {
            dark: pal.leaf_deep,
            mid: if hi { pal.leaf_accent } else { pal.leaf_soft },
            rim: Some(pal.leaf_rim),
            pen: FOLIAGE_PEN,
            sh_w: 205,
        };
        c.fill_ellipse([pl.0, pl.1, rx, ry], style, true);
    }
}

// ---- 7. KelpFrond (Outfall, organic) ---------------------------------------

fn paint_kelp_frond(c: &mut ReachCanvas, pal: &ReachPal, t: f32) {
    // 1. Seamless x <= 0 bull-kelp stipe + periodic gas bladders & swept blades (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-6.8, 6.8))
    {
        let stem_parts: [(f32, f32, f32, f32, bool); 4] = [
            (4.5, -2.4, 1.35, 1.1, false),
            (7.0, -4.0, 2.8, 1.5, true),
            (10.5, 2.4, 1.35, 1.1, false),
            (12.2, 4.0, 2.2, 1.4, true),
        ];
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let ang = u * (TAU / STEM_PERIOD as f32);
            let sv = 1.15 * ang.sin();
            let sr = 1.35_f32;
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let d_in = sr - (y - sv).abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.3, 195));
                let side = -(y - sv) / sr;
                let col = if side > 0.3 {
                    pal.leaf_rim
                } else if side < -0.25 {
                    pal.leaf_deep
                } else {
                    pal.leaf_mid
                };
                c.put_body(px, py, col, FOLIAGE_PEN);
            }
            for (bu, bv, brx, bry, is_blade) in stem_parts {
                if (u - bu).abs() >= brx {
                    continue;
                }
                let nx = (u - bu) / brx;
                let nx2 = nx * nx;
                let r_eff = brx.min(bry);
                for py in y_lo..=y_hi {
                    let y = c.ly(py);
                    let ny = (y - bv) / bry;
                    let q = nx2 + ny * ny;
                    if q >= 1.0 {
                        continue;
                    }
                    let d_in = (1.0 - q.sqrt()) * r_eff;
                    c.put_shadow(px, py, soft_u8(d_in, 1.2, 185));
                    let col = if !is_blade {
                        if ny < 0.0 {
                            pal.leaf_accent
                        } else {
                            pal.leaf_mid
                        }
                    } else if d_in < 0.7 && ny < 0.0 {
                        pal.leaf_rim
                    } else if ny.abs() < 0.35 {
                        pal.leaf_soft
                    } else {
                        pal.leaf_mid
                    };
                    c.put_body(px, py, col, FOLIAGE_PEN);
                }
            }
        }
    }

    // 2. Tip (0 < x <= 32.0): travelling ocean-current wave damped C1 to zero at x = 0.
    let sway = |x: f32| -> f32 {
        let s = (x / 32.0).clamp(0.0, 1.0);
        (s * s) * 2.2 * (t * 1.25 - x * 0.14).sin()
    };
    let pt = |x: f32, y: f32| -> (f32, f32) { (x, y + sway(x)) };

    let stipe_style = PaintStyle {
        dark: pal.leaf_deep,
        mid: pal.leaf_mid,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 195,
    };
    let stipe: [(f32, f32, f32); 5] = [
        (0.0, 0.0, 1.35),
        (5.5, 0.9, 1.25),
        (11.5, -0.6, 1.15),
        (17.5, 0.7, 1.00),
        (23.5, -0.3, 0.85),
    ];
    for pair in stipe.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let pa = pt(a.0, a.1);
        let pb = pt(b.0, b.1);
        c.fill_capsule([pa.0, pa.1, a.2, pb.0, pb.1, b.2], stipe_style, true);
    }

    // Sweeping ruffled side blades + gas bladders
    let bladder_style = PaintStyle {
        dark: pal.leaf_mid,
        mid: pal.leaf_accent,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 180,
    };
    let blade_base_style = PaintStyle {
        dark: pal.leaf_deep,
        mid: pal.leaf_soft,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 190,
    };
    let blade_tip_style = PaintStyle {
        dark: pal.leaf_mid,
        mid: pal.leaf_accent,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 185,
    };
    let blades: [((f32, f32), (f32, f32), (f32, f32), f32); 4] = [
        ((4.0, -1.6), (9.5, -4.6), (14.5, -4.0), 1.9),
        ((7.5, 1.8), (13.5, 4.8), (19.0, 4.2), 1.9),
        ((13.0, -1.6), (19.0, -4.6), (25.0, -3.6), 1.8),
        ((16.5, 1.6), (22.0, 4.4), (27.5, 3.2), 1.7),
    ];
    for (p_base, p_mid, p_end, w_mid) in blades {
        let pb0 = pt(p_base.0, p_base.1);
        let pm = pt(p_mid.0, p_mid.1);
        let pe = pt(p_end.0, p_end.1);
        c.fill_ellipse([pb0.0, pb0.1, 1.35, 1.1], bladder_style, true);
        c.fill_capsule([pb0.0, pb0.1, 1.0, pm.0, pm.1, w_mid], blade_base_style, true);
        c.fill_capsule([pm.0, pm.1, w_mid, pe.0, pe.1, 0.85], blade_tip_style, true);
    }

    // Terminal hero undulating kelp frond reaching x = 32.0 exact
    let hero_style = PaintStyle {
        dark: pal.leaf_deep,
        mid: pal.leaf_accent,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 195,
    };
    let hero: [(f32, f32, f32); 4] = [
        (21.0, 0.0, 1.6),
        (25.0, -1.0, 2.2),
        (28.5, 0.5, 1.8),
        (31.2, -0.3, 0.8), // 31.2 + 0.8 = 32.0
    ];
    for pair in hero.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let pa = pt(a.0, a.1);
        let pb = pt(b.0, b.1);
        c.fill_capsule([pa.0, pa.1, a.2, pb.0, pb.1, b.2], hero_style, true);
    }
}

// ---- 8. PipeElbow (Sewer / Lab / Printworks, rigid) ------------------------

fn paint_pipe_elbow(c: &mut ReachCanvas, pal: &ReachPal) {
    // 1. Seamless x <= 0 riveted industrial pipe barrel with bolted flanges (period 16).
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(c.x_origin, 0.0, false), c.py_range(-5.5, 5.5))
    {
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x > 0.0 {
                continue;
            }
            let u = pos_mod(x, STEM_PERIOD as f32);
            let du_f = (u - 8.0).abs();
            let on_flange = du_f < 1.75;
            let r = if on_flange { 5.0 } else { 3.6 };
            for py in y_lo..=y_hi {
                let y = c.ly(py);
                let ay = y.abs();
                let d_in = r - ay;
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.6, if on_flange { 235 } else { 220 }));
                let col = shade_pipe_cross(pal, y, r, d_in, on_flange, du_f);
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 2. Tip (0 < x <= 24.0): shoulder valve nub -> straight barrel + collar -> 90° elbow -> symmetric end flange.
    c.fill_rect(
        [5.4, -5.2, 6.6, -3.2],
        PaintStyle {
            dark: pal.metal_deep,
            mid: pal.metal_mid,
            rim: None,
            pen: METAL_PEN,
            sh_w: 210,
        },
        true,
    );
    c.fill_rect(
        [3.6, -6.5, 8.4, -5.0],
        PaintStyle {
            dark: pal.metal_deep,
            mid: pal.accent,
            rim: Some(pal.pale),
            pen: METAL_PEN,
            sh_w: 215,
        },
        true,
    );

    let ecx = 14.6_f32;
    let ecy = 4.8_f32;
    let r_arc = 4.8_f32;
    let r_tube = 3.6_f32;
    let exit_x = ecx + r_arc; // 19.4; terminal flange half-width 4.6 -> reaches 24.0 exact

    // Straight barrel leading into elbow bend (x in 0..14.6)
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) =
        (c.px_range(0.0, ecx, true), c.py_range(-5.2, 5.2))
    {
        for py in y_lo..=y_hi {
            let y = c.ly(py);
            let ay = y.abs();
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                if x <= 0.0 {
                    continue;
                }
                let du_f = (x - 11.4).abs();
                let on_flange = du_f < 1.6;
                let r = if on_flange { 4.9 } else { r_tube };
                let d_in = r - ay;
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.6, if on_flange { 235 } else { 220 }));
                let col = shade_pipe_cross(pal, y, r, d_in, on_flange, du_f);
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // 90° cast-iron toroidal elbow bend
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) = (
        c.px_range(ecx, ecx + r_arc + r_tube + 0.5, true),
        c.py_range(ecy - r_arc - r_tube, ecy + 1.5),
    ) {
        for py in y_lo..=y_hi {
            let y = c.ly(py);
            if y > ecy {
                continue;
            }
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                if x < ecx {
                    continue;
                }
                let dist = ((x - ecx) * (x - ecx) + (y - ecy) * (y - ecy)).sqrt();
                let d_in = r_tube - (dist - r_arc).abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.6, 235));
                let radial = (dist - r_arc) / r_tube;
                let col = if d_in < 0.95 && radial > 0.25 {
                    pal.metal_rim
                } else if (radial - 0.35).abs() < 0.18 {
                    palette::mix(pal.metal_soft, pal.pale, 0.42)
                } else if radial > 0.1 {
                    pal.metal_soft
                } else if radial < -0.35 {
                    pal.metal_deep
                } else {
                    pal.metal_mid
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // Vertical exit stub (y in 4.8..6.4)
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) = (
        c.px_range(exit_x - r_tube, exit_x + r_tube, true),
        c.py_range(4.8, 6.4),
    ) {
        for py in y_lo..=y_hi {
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                let d_in = r_tube - (x - exit_x).abs();
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.5, 230));
                let side = (x - exit_x) / r_tube;
                let col = if d_in < 0.9 && side > 0.2 {
                    pal.metal_rim
                } else if side > 0.1 {
                    pal.metal_soft
                } else if side < -0.25 {
                    pal.metal_deep
                } else {
                    pal.metal_mid
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }

    // Symmetric terminal bolted flange face (y in 6.2..8.7, x in 14.8..=24.0)
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) = (
        c.px_range(exit_x - 4.6, 24.0, true),
        c.py_range(6.2, 8.7),
    ) {
        for py in y_lo..=y_hi {
            let y = c.ly(py);
            for px in x_lo..=x_hi {
                let x = c.lx(px);
                let d_in = (x - (exit_x - 4.6))
                    .min((exit_x + 4.6) - x)
                    .min(y - 6.2)
                    .min(8.7 - y);
                if d_in <= 0.0 {
                    continue;
                }
                c.put_shadow(px, py, soft_u8(d_in, 1.4, 240));
                let col = if (y - 7.45).abs() < 0.45 {
                    pal.metal_deep
                } else if ((x - exit_x).abs() - 3.0).abs() < 0.7 && y < 7.1 {
                    pal.pale
                } else if y < 7.2 {
                    pal.metal_soft
                } else {
                    pal.metal_deep
                };
                c.put_body(px, py, col, METAL_PEN);
            }
        }
    }
}

#[inline]
fn shade_pipe_cross(
    pal: &ReachPal,
    y: f32,
    r: f32,
    d_in: f32,
    on_flange: bool,
    du_f: f32,
) -> Rgb {
    let ay = y.abs();
    let side = -y / r;
    if on_flange {
        if du_f < 0.45 {
            pal.metal_deep
        } else if (ay - 3.6).abs() < 0.75 && du_f > 0.6 {
            if side > 0.0 {
                pal.pale
            } else {
                pal.accent
            }
        } else if d_in < 0.9 && side > 0.2 {
            pal.metal_rim
        } else if side > 0.2 {
            pal.metal_soft
        } else if side < -0.25 {
            pal.metal_deep
        } else {
            pal.metal_mid
        }
    } else if d_in < 0.95 && side > 0.2 {
        pal.metal_rim
    } else if (side - 0.42).abs() < 0.16 {
        palette::mix(pal.metal_soft, pal.pale, 0.45)
    } else if side > 0.15 {
        pal.metal_soft
    } else if side < -0.25 {
        pal.metal_deep
    } else {
        pal.metal_mid
    }
}

// ---- Unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buffers(w: usize, h: usize, materials: bool) -> (PixelBuffer, PixelBuffer) {
        let mut body = PixelBuffer::new(w, h);
        let mut shadow = PixelBuffer::new(w, h);
        if materials {
            body.set_materials(true);
            shadow.set_materials(true);
        }
        body.clear((0, 0, 0));
        shadow.clear((0, 0, 0));
        (body, shadow)
    }

    #[test]
    fn every_kind_draws_non_empty_body_and_shadow() {
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            assert!(spec.tip_len > 0.0, "{kind:?} tip_len must be positive");
            assert!(spec.thickness > 0.0, "{kind:?} thickness must be positive");
            assert!(!spec.floors.is_empty(), "{kind:?} must appear on at least one floor");

            let len = spec.tip_len.round() as i32 + STEM_PERIOD * 2;
            let h = spec.thickness.ceil() as usize + 4;
            for &theme in spec.floors {
                let (mut body, mut shadow) = make_buffers(len as usize + 4, h, true);
                draw_reach(kind, &mut body, &mut shadow, len, 0.5, theme);

                let body_px = body.pixels().iter().filter(|&&p| p != (0, 0, 0)).count();
                let shadow_px = shadow.pixels().iter().filter(|&&p| p != (0, 0, 0)).count();
                assert!(
                    body_px > 60,
                    "{kind:?} on {theme:?} painted too few body pixels ({body_px})"
                );
                assert!(
                    shadow_px > 60,
                    "{kind:?} on {theme:?} painted too few shadow pixels ({shadow_px})"
                );
            }
        }
    }

    #[test]
    fn negative_x_stem_tiles_seamlessly_across_adjacent_segments() {
        let p = STEM_PERIOD as usize;
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let theme = spec.floors[0];
            let len = spec.tip_len.round() as i32 + STEM_PERIOD * 2;
            let h = spec.thickness.ceil() as usize + 4;
            let (mut body, mut shadow) = make_buffers(len as usize, h, true);
            draw_reach(kind, &mut body, &mut shadow, len, 1.3, theme);

            // Segment 0 (`px in 0..P`) and Segment 1 (`px in P..2*P`) are both
            // strictly in `x <= 0` and must be bit-identical in RGB, mat, and shadow.
            for py in 0..h {
                for px in 0..p {
                    assert_eq!(
                        body.pixel_at(px, py),
                        body.pixel_at(px + p, py),
                        "{kind:?} body RGB tile mismatch at ({px}, {py})"
                    );
                    assert_eq!(
                        body.mat()[py * body.w + px],
                        body.mat()[py * body.w + (px + p)],
                        "{kind:?} body mat tile mismatch at ({px}, {py})"
                    );
                    assert_eq!(
                        shadow.pixel_at(px, py),
                        shadow.pixel_at(px + p, py),
                        "{kind:?} shadow tile mismatch at ({px}, {py})"
                    );
                }
            }

            // Seam continuity: both sides of the tile seam (`px = P - 1` vs `P`)
            // and the stem->tip seam (`px = 2*P - 1` vs `2*P`) must be non-empty
            // and share overlapping vertical coverage (zero break/gap at the seam).
            for seam_right in [p, 2 * p] {
                let seam_left = seam_right - 1;
                let overlap = (0..h)
                    .filter(|&py| {
                        body.pixel_at(seam_left, py) != (0, 0, 0)
                            && body.pixel_at(seam_right, py) != (0, 0, 0)
                    })
                    .count();
                assert!(
                    overlap >= 2,
                    "{kind:?} broke across seam {seam_left}|{seam_right} (overlap={overlap})"
                );
            }
        }
    }

    #[test]
    fn tip_lies_at_tip_len_and_stays_within_bounds() {
        let pad = 8usize;
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let theme = spec.floors[0];
            let len = spec.tip_len.round() as i32 + STEM_PERIOD;
            let w = len as usize + pad;
            let h = spec.thickness.ceil() as usize + 8;
            let cy = h as f32 * 0.5;
            let half_t = spec.thickness * 0.5;

            for &t in &[0.0_f32, 0.9, 2.2] {
                let (mut body, mut shadow) = make_buffers(w, h, false);
                draw_reach(kind, &mut body, &mut shadow, len, t, theme);

                let mut max_px: Option<usize> = None;
                for py in 0..h {
                    let ly = (py as f32 + 0.5) - cy;
                    for px in 0..w {
                        let b = body.pixel_at(px, py) != (0, 0, 0);
                        let s = shadow.pixel_at(px, py) != (0, 0, 0);
                        if b || s {
                            assert!(
                                px < len as usize,
                                "{kind:?} painted past tip_len at px={px} (len={len})"
                            );
                            assert!(
                                ly.abs() <= half_t + 0.001,
                                "{kind:?} exceeded thickness at ly={ly} (half_t={half_t})"
                            );
                            if b {
                                max_px = Some(max_px.map_or(px, |m| m.max(px)));
                            }
                        }
                    }
                }
                let tip_px = max_px.expect("body must paint pixels");
                let tip_x = (tip_px as f32 + 0.5) + (spec.tip_len - len as f32);
                assert!(
                    (spec.tip_len - tip_x).abs() <= 1.5,
                    "{kind:?} (t={t}): rightmost tip pixel at x={tip_x}, expected near tip_len={}",
                    spec.tip_len
                );
            }
        }
    }

    #[test]
    fn organic_sway_is_smooth_in_t_and_rigid_kinds_stay_still() {
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let theme = spec.floors[0];
            let len = spec.tip_len.round() as i32 + STEM_PERIOD;
            let h = spec.thickness.ceil() as usize + 4;

            let tip_centroid_y = |t: f32| -> (PixelBuffer, f32) {
                let (mut body, mut shadow) = make_buffers(len as usize, h, false);
                draw_reach(kind, &mut body, &mut shadow, len, t, theme);
                let mut sum_y = 0.0_f32;
                let mut cnt = 0.0_f32;
                for py in 0..h {
                    for px in (STEM_PERIOD as usize + (spec.tip_len as usize / 2))..len as usize {
                        if body.pixel_at(px, py) != (0, 0, 0) {
                            sum_y += py as f32;
                            cnt += 1.0;
                        }
                    }
                }
                (body, sum_y / cnt.max(1.0))
            };

            if spec.organic {
                let mut min_c = f32::INFINITY;
                let mut max_c = f32::NEG_INFINITY;
                let (base_pb, mut prev_c) = tip_centroid_y(0.0);
                min_c = min_c.min(prev_c);
                max_c = max_c.max(prev_c);

                // Step finely across a full sway cycle: adjacent steps move smoothly.
                for step in 1..=100 {
                    let tv = step as f32 * 0.05;
                    let (pb, c) = tip_centroid_y(tv);
                    assert!(
                        (c - prev_c).abs() < 0.65,
                        "{kind:?} jumped between t={} and t={tv}: {prev_c} -> {c}",
                        tv - 0.05
                    );
                    // Anchor stem (x <= 0) stays completely fixed during sway.
                    for py in 0..h {
                        for px in 0..STEM_PERIOD as usize {
                            assert_eq!(
                                base_pb.pixel_at(px, py),
                                pb.pixel_at(px, py),
                                "{kind:?} anchor stem shifted at ({px}, {py}) for t={tv}"
                            );
                        }
                    }
                    min_c = min_c.min(c);
                    max_c = max_c.max(c);
                    prev_c = c;
                }
                assert!(
                    max_c - min_c >= 0.9,
                    "{kind:?} organic sway amplitude too small ({min_c}..{max_c})"
                );
            } else {
                let (pb_a, _) = tip_centroid_y(0.0);
                let (pb_b, _) = tip_centroid_y(2.3);
                assert_eq!(
                    pb_a.pixels(),
                    pb_b.pixels(),
                    "{kind:?} is man-made and should not sway with t"
                );
            }
        }
    }

    #[test]
    fn lamp_lenses_mark_emissive_pen_and_restore_previous_pen() {
        for (kind, theme) in [
            (ReachKind::LightPole, Theme::Trailer),
            (ReachKind::HangingLamp, Theme::Kitchen),
        ] {
            let spec = reach_spec(kind);
            let len = spec.tip_len.round() as i32 + STEM_PERIOD;
            let h = spec.thickness.ceil() as usize + 4;
            let (mut body, mut shadow) = make_buffers(len as usize, h, true);
            body.set_pen(3);
            shadow.set_pen(5);

            draw_reach(kind, &mut body, &mut shadow, len, 0.0, theme);

            assert_eq!(body.pen(), 3, "{kind:?} must restore body pen");
            assert_eq!(shadow.pen(), 5, "{kind:?} must restore shadow pen");
            let emissive_count = body.mat().iter().filter(|&&m| m == EMISSIVE_PEN).count();
            let non_emissive_body = body
                .pixels()
                .iter()
                .zip(body.mat().iter())
                .filter(|pair| *pair.0 != (0, 0, 0) && *pair.1 != EMISSIVE_PEN)
                .count();
            assert!(
                emissive_count >= 16,
                "{kind:?} expected >= 16 emissive lens pixels, got {emissive_count}"
            );
            assert!(
                non_emissive_body >= 30,
                "{kind:?} expected non-emissive housing/stem pixels, got {non_emissive_body}"
            );
            assert!(
                shadow.mat().iter().all(|&m| m == 0),
                "{kind:?} shadow raster must stamp pen 0"
            );
        }
    }

    #[test]
    fn degenerate_lengths_and_partial_tip_clip_cleanly() {
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let theme = spec.floors[0];
            let h = spec.thickness.ceil() as usize + 4;

            // len <= 0 writes nothing
            let (mut body, mut shadow) = make_buffers(32, h, false);
            draw_reach(kind, &mut body, &mut shadow, 0, 0.0, theme);
            draw_reach(kind, &mut body, &mut shadow, -10, 0.0, theme);
            assert!(body.pixels().iter().all(|&p| p == (0, 0, 0)));
            assert!(shadow.pixels().iter().all(|&p| p == (0, 0, 0)));

            // Partial tip (len < tip_len) draws the outermost slice without panicking
            let partial = 6i32;
            let (mut pb_p, mut sh_p) = make_buffers(partial as usize, h, false);
            draw_reach(kind, &mut pb_p, &mut sh_p, partial, 0.2, theme);
            assert!(
                pb_p.pixels().iter().any(|&p| p != (0, 0, 0)),
                "{kind:?} partial tip len={partial} drew empty"
            );

            // Zero-sized buffers never panic
            let (mut z_b, mut z_s) = make_buffers(0, 0, false);
            draw_reach(kind, &mut z_b, &mut z_s, 40, 0.0, theme);
        }
    }
}
