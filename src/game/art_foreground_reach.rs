//! Art for foreground items that reach in from off screen (`fore-reach-contract`).
//!
//! Two geometric families share one local raster API:
//!
//! * **Strips** ([`ReachShape::Strip`] — `LightPole`, `SignArm`, `CraneHook`,
//!   `HangingLamp`, `VineTendril`, `KelpFrond`, `PipeElbow`): narrow near-camera
//!   silhouettes anchored beyond one viewport edge. Their `x <= 0` stem tiles
//!   seamlessly forever (period [`STEM_PERIOD`] = 16 art px) and transitions
//!   smoothly into a free tip ending at `x = spec.tip_len`.
//! * **2D Tree Canopy** ([`ReachShape::Canopy`] — [`ReachKind::TreeCanopy`]): one
//!   whole tree crown seen from below or the side, anchored flat along one
//!   viewport edge (`x = 0`) with its trunk strictly off-screen (`x = CANOPY_TRUNK_X < 0`).
//!   Its silhouette is an irregular, gappy half-disc of radius [`CANOPY_RADIUS`]
//!   (`36.0` art px; `72 x 144` world px at `k = 2`, `108 x 216` at `k = 3`),
//!   covering most of an 80-px short axis while leaving ~45 % open light windows.
//!
//! ## Contract
//!
//! ```ignore
//! pub enum AnchorSide { Top, Bottom, Left, Right }
//! pub enum ReachShape { Strip, Canopy }
//! pub enum ReachKind { TreeCanopy, LightPole, SignArm, CraneHook, HangingLamp, VineTendril, KelpFrond, PipeElbow }
//! pub struct ReachSpec {
//!     pub tip_len: f32,
//!     pub thickness: f32,
//!     pub organic: bool,
//!     pub shape: ReachShape,
//!     pub sides: &'static [AnchorSide],
//!     pub floors: &'static [Theme],
//! }
//! pub fn reach_spec(kind: ReachKind) -> ReachSpec;
//! pub fn draw_reach(kind: ReachKind, body: &mut PixelBuffer, shadow: &mut PixelBuffer, len: i32, t: f32, theme: Theme);
//! ```
//!
//! ## Fixed Entry Sides & Anchor-Side Mapping
//!
//! Read `reach_spec(kind).sides` (or `kind.allowed_sides()`) when placing items:
//!
//! * [`ReachKind::LightPole`] — [`AnchorSide::Bottom`] only (mast rises from bottom edge).
//! * [`ReachKind::HangingLamp`], [`ReachKind::CraneHook`] — [`AnchorSide::Top`] only
//!   (cord / cable hangs from top edge).
//! * [`ReachKind::TreeCanopy`], [`ReachKind::SignArm`], [`ReachKind::VineTendril`],
//!   [`ReachKind::KelpFrond`], [`ReachKind::PipeElbow`] — any of
//!   [`AnchorSide::ALL`] (`Top | Bottom | Left | Right`).
//!
//! Local coordinates `(local_x, local_y)` map to screen coordinates `(sx, sy)`
//! around the anchor point along the chosen viewport edge:
//!
//! * `AnchorSide::Left`:   anchor edge `sx = 0`,      inward `+local_x -> +sx`, lateral `local_y -> +sy`.
//! * `AnchorSide::Right`:  anchor edge `sx = view_w`, inward `+local_x -> -sx`, lateral `local_y -> +sy`.
//! * `AnchorSide::Top`:    anchor edge `sy = 0`,      inward `+local_x -> +sy`, lateral `local_y -> +sx`.
//! * `AnchorSide::Bottom`: anchor edge `sy = view_h`, inward `+local_x -> -sy`, lateral `local_y -> +sx`.
//!
//! ## 2D Canopy Coordinate Contract (`ReachKind::TreeCanopy`)
//!
//! `TreeCanopy` uses the **exact same buffer mapping** as the strip kinds, with
//! `spec.tip_len = CANOPY_RADIUS` (`36.0`) and `spec.thickness = 2.0 * CANOPY_RADIUS` (`72.0`):
//!
//! * **Anchor edge**: the flat diameter of the half-disc lies on `local_x = 0.0`.
//! * **Depth into the view**: `0.0 < local_x <= CANOPY_RADIUS` (`36.0` art px).
//!   At `k = 2` (standard parallax band) this spans `72` world px (`0.90x` of an
//!   80-px short axis); at `k = 3` (near parallax) it spans `108` world px.
//! * **Lateral extent**: `|local_y| <= CANOPY_RADIUS` (`72.0` art px wide,
//!   centred on `local_y = 0.0` / `py = h * 0.5`; `144` world px at `k = 2`,
//!   `216` world px at `k = 3`).
//! * **Sliding off-screen in its one direction**: pass `len in 0..=CANOPY_RADIUS as i32`
//!   (`0..=36`). At `len = 36` the full half-disc sits flush against the anchor edge;
//!   smaller `len` clips cleanly from the anchor edge so the outer crown glides
//!   off-screen. The placer should clamp `len <= spec.tip_len.round() as i32`.
//! * **Trunk always off-screen**: the trunk hub lives at `local_x = CANOPY_TRUNK_X`
//!   (`-11.0`); `draw_reach` never paints any pixel with `local_x <= 0.0`, even if
//!   a caller passes `len > tip_len`.
//! * **Buffer sizing**: width `>= len + blur_pad`, height `>= 72 + 2 * blur_pad`.
//! * **Light-through**: 35 %–65 % of the half-disc area is filled (~55 %), keeping
//!   at least ~45 % open dappled sky so enemies and projectiles stay readable.
//!
//! ## Raster Conventions
//!
//! * `len` local art pixels are rasterised into `px in 0..len` (clipped to the
//!   buffer width), mapping `px` to `local_x = px as f32 + 0.5 + (spec.tip_len - len as f32)`
//!   and `py` to `local_y = py as f32 + 0.5 - h as f32 * 0.5`, so `px = len - 1`
//!   sits at the tip/rim (`local_x = spec.tip_len - 0.5`) and any extra columns
//!   `px >= len` remain empty `(0, 0, 0)` for blur padding.
//! * `body` receives full un-premultiplied silhouette colours (empty pixels stay
//!   `(0, 0, 0)`) and material pens via [`PixelBuffer::set_pen`] (warm lamp lenses
//!   on [`ReachKind::LightPole`] and [`ReachKind::HangingLamp`] stamp
//!   [`EMISSIVE_PEN`] = `14`, matching `MaterialKind::Emissive`).
//! * `shadow` receives a soft grayscale shadow mask `(s, s, s)` with pen `0`
//!   (`0` = transparent, `255` = full shadow weight).
//! * Pure render-only drawing: zero heap allocations, zero RNG, and smooth C1
//!   sway in `t` for organic kinds (anchored rigid at `x <= 0`, with outer twigs
//!   swaying substantially more than inner limbs on `TreeCanopy`).

use std::f32::consts::TAU;

use crate::game::level::Theme;
use crate::render::foreground::rim_tint;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Exact seamless repeat period of the `x <= 0` anchor stem in local art pixels
/// for [`ReachShape::Strip`] kinds. A power of two so `pos_mod(x, 16.0)` is
/// IEEE-754 exact for every half-integer sample coordinate.
pub const STEM_PERIOD: i32 = 16;

/// Semicircular radius of [`ReachKind::TreeCanopy`] in local art pixels.
/// At `k = 2` this yields `72` world px (`0.90x` of an 80-px short axis) deep
/// by `144` world px laterally; at `k = 3`, `108 x 216` world px.
pub const CANOPY_RADIUS: f32 = 36.0;

/// Off-screen trunk hub coordinate for [`ReachKind::TreeCanopy`] in local art
/// pixels. Strictly negative so the trunk always lies behind the anchor edge
/// (`x = 0`) and never enters the drawn extent.
pub const CANOPY_TRUNK_X: f32 = -11.0;

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

/// Viewport edge from which a foreground reach item may enter the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnchorSide {
    Top,
    Bottom,
    Left,
    Right,
}

impl AnchorSide {
    /// All four viewport edges, in canonical order.
    pub const ALL: &'static [Self] = &[Self::Top, Self::Bottom, Self::Left, Self::Right];
}

const TOP_ONLY: &[AnchorSide] = &[AnchorSide::Top];
const BOTTOM_ONLY: &[AnchorSide] = &[AnchorSide::Bottom];

/// Geometric family of a [`ReachKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReachShape {
    /// Narrow strip with an infinitely tiling stem (`x <= 0`, period [`STEM_PERIOD`])
    /// and a free tip (`0 < x <= tip_len`).
    Strip,
    /// 2D half-disc canopy anchored flat along `x = 0`, extending `0 < x <= tip_len`
    /// into the view and `|y| <= thickness * 0.5` laterally, with its trunk strictly
    /// off-screen (`x < 0`) and nothing drawn for `x <= 0`.
    Canopy,
}

/// Foreground item kinds that reach into the view from off screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReachKind {
    TreeCanopy,
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
            Self::TreeCanopy,
            Self::LightPole,
            Self::SignArm,
            Self::CraneHook,
            Self::HangingLamp,
            Self::VineTendril,
            Self::KelpFrond,
            Self::PipeElbow,
        ]
    }

    /// Viewport edges from which this kind is allowed to enter.
    pub const fn allowed_sides(self) -> &'static [AnchorSide] {
        match self {
            Self::LightPole => BOTTOM_ONLY,
            Self::HangingLamp | Self::CraneHook => TOP_ONLY,
            Self::TreeCanopy
            | Self::SignArm
            | Self::VineTendril
            | Self::KelpFrond
            | Self::PipeElbow => AnchorSide::ALL,
        }
    }

    /// Whether this kind is a repeating-stem strip or a 2D edge-anchored canopy.
    pub const fn shape(self) -> ReachShape {
        match self {
            Self::TreeCanopy => ReachShape::Canopy,
            Self::LightPole
            | Self::SignArm
            | Self::CraneHook
            | Self::HangingLamp
            | Self::VineTendril
            | Self::KelpFrond
            | Self::PipeElbow => ReachShape::Strip,
        }
    }
}

/// Geometry, motion, entry-side, and floor eligibility for a [`ReachKind`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReachSpec {
    pub tip_len: f32,
    pub thickness: f32,
    pub organic: bool,
    pub shape: ReachShape,
    pub sides: &'static [AnchorSide],
    pub floors: &'static [Theme],
}

const FLOORS_TREE_CANOPY: &[Theme] = &[Theme::Trailer, Theme::Kitchen];
const FLOORS_TRAILER_ONLY: &[Theme] = &[Theme::Trailer];
const FLOORS_CRANE_HOOK: &[Theme] = &[Theme::Printworks, Theme::Lab, Theme::Sewer];
const FLOORS_HANGING_LAMP: &[Theme] = &[Theme::Kitchen, Theme::Laundry, Theme::Lab];
const FLOORS_SEWER_ONLY: &[Theme] = &[Theme::Sewer];
const FLOORS_OUTFALL_ONLY: &[Theme] = &[Theme::Outfall];
const FLOORS_PIPE_ELBOW: &[Theme] = &[Theme::Sewer, Theme::Lab, Theme::Printworks];

/// Static placement and sizing specification for `kind` in local art pixels.
pub fn reach_spec(kind: ReachKind) -> ReachSpec {
    let shape = kind.shape();
    let sides = kind.allowed_sides();
    match kind {
        ReachKind::TreeCanopy => ReachSpec {
            tip_len: CANOPY_RADIUS,
            thickness: CANOPY_RADIUS * 2.0,
            organic: true,
            shape,
            sides,
            floors: FLOORS_TREE_CANOPY,
        },
        ReachKind::LightPole => ReachSpec {
            tip_len: 26.0,
            thickness: 18.0,
            organic: false,
            shape,
            sides,
            floors: FLOORS_TRAILER_ONLY,
        },
        ReachKind::SignArm => ReachSpec {
            tip_len: 28.0,
            thickness: 20.0,
            organic: false,
            shape,
            sides,
            floors: FLOORS_TRAILER_ONLY,
        },
        ReachKind::CraneHook => ReachSpec {
            tip_len: 24.0,
            thickness: 16.0,
            organic: false,
            shape,
            sides,
            floors: FLOORS_CRANE_HOOK,
        },
        ReachKind::HangingLamp => ReachSpec {
            tip_len: 20.0,
            thickness: 18.0,
            organic: false,
            shape,
            sides,
            floors: FLOORS_HANGING_LAMP,
        },
        ReachKind::VineTendril => ReachSpec {
            tip_len: 28.0,
            thickness: 16.0,
            organic: true,
            shape,
            sides,
            floors: FLOORS_SEWER_ONLY,
        },
        ReachKind::KelpFrond => ReachSpec {
            tip_len: 32.0,
            thickness: 16.0,
            organic: true,
            shape,
            sides,
            floors: FLOORS_OUTFALL_ONLY,
        },
        ReachKind::PipeElbow => ReachSpec {
            tip_len: 24.0,
            thickness: 18.0,
            organic: false,
            shape,
            sides,
            floors: FLOORS_PIPE_ELBOW,
        },
    }
}

/// Rasterise the item in its LOCAL frame: `x` runs from the anchor side toward
/// the tip/rim at `x = spec.tip_len`; `y` runs parallel to the anchor edge with
/// `0` on the centre line (`|y| <= spec.thickness * 0.5`).
///
/// For [`ReachShape::Strip`] kinds, `x <= 0` continues forever via a repeating
/// stem segment of period [`STEM_PERIOD`]. For [`ReachKind::TreeCanopy`]
/// ([`ReachShape::Canopy`]), the trunk lies strictly off-screen (`x < 0`) and
/// `x <= 0` is never drawn; pass `len in 0..=CANOPY_RADIUS as i32`.
///
/// `body` and `shadow` are the kept rasters the foreground layer blurs and
/// composites (same conventions as `art_foreground`'s bands: 2x/3x pixel
/// blocks, body + shadow, pens). `len` is how many local px to draw from the
/// screen edge to the tip/rim; `t` drives smooth sway for organic kinds.
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
            ReachKind::TreeCanopy => paint_tree_canopy(&mut canvas, &pal, t),
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

// ---- 1. TreeCanopy (2D half-disc crown, off-screen trunk, organic) ---------

/// Radius-preserving, C1-anchored whole-canopy breeze with stronger outer-twig sway.
#[inline]
fn canopy_sway(x: f32, y: f32, t: f32) -> (f32, f32) {
    if x <= 0.0 {
        return (x, y);
    }
    let r = (x * x + y * y).sqrt();
    if r < 1e-4 {
        return (x, y);
    }
    let sr = (r / CANOPY_RADIUS).clamp(0.0, 1.0);
    let ang = y.atan2(x);
    // Smoothstep to 0 at x = 0 so primary limbs stay C1-rooted to the off-screen trunk
    // and never rotate backward across x = 0.
    let sx = (x / 10.0).clamp(0.0, 1.0);
    let edge_guard = sx * sx * (3.0 - 2.0 * sx);
    let whole = (sr * sr) * (0.085 * (t * 1.05).sin() + 0.022 * (t * 1.80 + 0.7).sin());
    let twig = (sr * sr * sr) * (0.032 * (t * 1.45 + ang * 1.6).sin());
    let new_ang = ang + edge_guard * (whole + twig);
    (r * new_ang.cos(), r * new_ang.sin())
}

fn fill_canopy_lobe(c: &mut ReachCanvas, lobe: [f32; 4], style: PaintStyle) {
    let [cx, cy, rx, ry] = lobe;
    if rx <= 0.1 || ry <= 0.1 {
        return;
    }
    let Some((x_lo, x_hi)) = c.px_range(cx - rx - 1.0, cx + rx + 1.0, true) else {
        return;
    };
    let Some((y_lo, y_hi)) = c.py_range(cy - ry - 1.0, cy + ry + 1.0) else {
        return;
    };
    let r_eff = rx.min(ry);
    let r_max2 = CANOPY_RADIUS * CANOPY_RADIUS;
    for py in y_lo..=y_hi {
        let y = c.ly(py);
        let ny = (y - cy) / ry;
        let ny2 = ny * ny;
        if ny2 >= 1.0 {
            continue;
        }
        let y2 = y * y;
        for px in x_lo..=x_hi {
            let x = c.lx(px);
            if x <= 0.0 || x * x + y2 > r_max2 {
                continue;
            }
            let nx = (x - cx) / rx;
            let q = nx * nx + ny2;
            if q >= 1.0 {
                continue;
            }
            let d_in = (1.0 - q.sqrt()) * r_eff;
            c.put_shadow(px, py, soft_u8(d_in, 1.6, style.sh_w));
            // Directional crown lighting: outer/upper edge brighter, inner underside shaded.
            let side = -0.62 * ny + 0.38 * nx;
            let col = if let Some(rim) = style.rim {
                if d_in < 1.05 && side > 0.14 {
                    rim
                } else if side < -0.22 {
                    style.dark
                } else {
                    palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                }
            } else if side < -0.22 {
                style.dark
            } else {
                palette::mix(style.dark, style.mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
            };
            c.put_body(px, py, col, style.pen);
        }
    }
}

fn paint_tree_canopy(c: &mut ReachCanvas, pal: &ReachPal, t: f32) {
    let pt = |x: f32, y: f32| -> (f32, f32) { canopy_sway(x, y, t) };

    // Pass 1: Back-crown deep shadow foliage behind forks & sub-crowns.
    let back_style = PaintStyle {
        dark: pal.leaf_deep,
        mid: pal.leaf_deep,
        rim: None,
        pen: FOLIAGE_PEN,
        sh_w: 188,
    };
    let back_foliage: [[f32; 4]; 16] = [
        // Sub-crown 1 (far upper flank)
        [4.6, -28.6, 3.6, 3.3],
        [11.4, -27.2, 4.0, 3.5],
        [17.8, -24.2, 3.8, 3.2],
        // Mid-crown pocket tucked behind Limb 1/2 forks
        [10.8, -16.8, 3.3, 2.8],
        // Sub-crown 2 (dominant upper-central + apex crown)
        [21.2, -14.8, 3.7, 3.2],
        [26.8, -12.6, 4.1, 3.4],
        [23.8, -5.6, 3.6, 3.1],
        [30.0, -5.2, 4.0, 3.5],
        [31.2, 0.4, 3.8, 3.4],
        // Mid-crown pockets on Limb 2/3
        [13.6, -2.4, 3.1, 2.7],
        [15.2, 9.6, 3.2, 2.8],
        // Sub-crown 3 (lower-central crown)
        [27.8, 12.6, 3.9, 3.3],
        [23.2, 16.4, 3.8, 3.3],
        [24.6, 21.8, 3.7, 3.2],
        // Sub-crown 4 (lower flank)
        [14.8, 28.4, 3.9, 3.3],
        [7.8, 29.8, 3.8, 3.4],
    ];
    for [cx, cy, rx, ry] in back_foliage {
        let (px_c, py_c) = pt(cx, cy);
        fill_canopy_lobe(c, [px_c, py_c, rx, ry], back_style);
    }

    // Pass 2: Gnarled asymmetric 4-bough wood skeleton (3 levels of forks).
    // Trunk sits strictly off-screen at CANOPY_TRUNK_X (-11.0); all limbs clip to x > 0.
    let tx = CANOPY_TRUNK_X;
    let bark = PaintStyle {
        dark: pal.bark_deep,
        mid: pal.bark_mid,
        rim: Some(pal.bark_rim),
        pen: WOOD_PEN,
        sh_w: 232,
    };
    let limbs: [[f32; 6]; 43] = [
        // Limb 1: Sweeping upper-flank oak bough
        [tx, -2.2, 3.1, 0.4, -12.2, 2.25],
        [0.4, -12.2, 2.25, 6.4, -16.8, 1.80],
        [6.4, -16.8, 1.80, 12.8, -22.8, 1.35],
        // Fork 1a (flank sweep toward anchor rim)
        [6.4, -16.8, 1.35, 5.2, -24.2, 0.98],
        [5.2, -24.2, 0.98, 3.6, -30.2, 0.58],
        [5.2, -24.2, 0.80, 9.4, -27.8, 0.52],
        // Fork 1b (upper-outer crown)
        [12.8, -22.8, 1.15, 17.2, -26.4, 0.80],
        [17.2, -26.4, 0.80, 20.2, -25.4, 0.50],
        // Fork 1c (mid-upper twigs)
        [12.8, -22.8, 0.95, 18.8, -20.8, 0.65],
        [6.4, -16.8, 1.05, 12.4, -14.8, 0.72],
        // Limb 2: Dominant upper-central hero bough (reaches apex x = 36)
        [tx, -0.6, 3.4, 0.4, -3.6, 2.55],
        [0.4, -3.6, 2.55, 9.0, -5.8, 2.05],
        [9.0, -5.8, 2.05, 17.6, -4.2, 1.55],
        [17.6, -4.2, 1.55, 25.4, -3.2, 1.12],
        // Fork 2a (sweeping upper-mid sub-branch)
        [9.0, -5.8, 1.45, 15.8, -11.6, 1.08],
        [15.8, -11.6, 1.08, 22.4, -14.8, 0.80],
        [22.4, -14.8, 0.80, 27.4, -13.0, 0.52],
        [22.4, -14.8, 0.72, 24.4, -18.8, 0.48],
        // Fork 2b (upper apex twig)
        [17.6, -4.2, 1.10, 23.6, -8.8, 0.78],
        [23.6, -8.8, 0.78, 29.2, -9.2, 0.50],
        // Fork 2c (outermost apex crown twigs)
        [25.4, -3.2, 0.95, 31.4, -4.8, 0.55],
        [25.4, -3.2, 0.98, 32.4, 0.2, 0.58],
        [17.6, -4.2, 0.95, 22.8, 1.8, 0.65],
        [22.8, 1.8, 0.65, 27.8, 3.6, 0.48],
        // Limb 3: Sinuous lower-central bough
        [tx, 1.0, 3.2, 0.4, 5.2, 2.40],
        [0.4, 5.2, 2.40, 8.6, 7.6, 1.90],
        [8.6, 7.6, 1.90, 16.4, 12.6, 1.42],
        [16.4, 12.6, 1.42, 23.6, 14.8, 1.00],
        // Fork 3a (inner-mid branchlet)
        [8.6, 7.6, 1.18, 15.4, 7.2, 0.80],
        [15.4, 7.2, 0.80, 20.4, 9.2, 0.50],
        // Fork 3b (outer lower-central twigs)
        [23.6, 14.8, 0.88, 29.0, 12.4, 0.52],
        [23.6, 14.8, 0.84, 27.2, 18.8, 0.50],
        // Fork 3c (downward sub-branch)
        [16.4, 12.6, 1.15, 20.4, 19.6, 0.82],
        [20.4, 19.6, 0.82, 23.6, 23.4, 0.50],
        [20.4, 19.6, 0.72, 16.6, 23.2, 0.48],
        // Limb 4: Lower flank sweeping oak bough
        [tx, 2.4, 3.0, 0.4, 13.8, 2.15],
        [0.4, 13.8, 2.15, 6.2, 20.0, 1.65],
        [6.2, 20.0, 1.65, 12.0, 25.4, 1.20],
        // Fork 4a (flank sweep toward bottom anchor rim)
        [6.2, 20.0, 1.25, 4.8, 27.0, 0.88],
        [4.8, 27.0, 0.88, 3.0, 32.0, 0.52],
        // Fork 4b (lower-outer twigs)
        [12.0, 25.4, 0.98, 15.6, 29.2, 0.70],
        [15.6, 29.2, 0.70, 18.0, 28.4, 0.48],
        [12.0, 25.4, 0.85, 9.6, 29.8, 0.52],
    ];
    for [ax, ay, ra, bx, by, rb] in limbs {
        let (pax, pay) = pt(ax, ay);
        let (pbx, pby) = pt(bx, by);
        c.fill_capsule([pax, pay, ra, pbx, pby, rb], bark, true);
    }

    // Pass 3: Mid-crown + outer billowing leaf clusters with deep sky clefts.
    let mid_foliage: [(f32, f32, f32, f32, bool); 24] = [
        // Sub-crown 1 (far upper flank)
        (3.8, -30.4, 3.1, 3.1, false),
        (8.6, -28.4, 3.4, 3.0, true),
        (13.8, -26.8, 3.3, 2.8, false),
        (18.6, -25.6, 3.2, 2.8, true),
        // Mid-crown foliage cluster on Limb 1 fork
        (11.6, -15.6, 3.0, 2.5, false),
        (15.2, -13.2, 2.6, 2.3, true),
        // Sub-crown 2 (dominant upper-central + apex; rightmost puff reaches x = 36.0 exact)
        (24.6, -17.6, 3.0, 2.6, false),
        (27.6, -13.2, 3.6, 3.1, true),
        (29.6, -8.8, 3.2, 2.8, false),
        (25.2, -5.2, 3.1, 2.7, false),
        (30.8, -4.6, 3.6, 3.1, true),
        (32.8, 0.2, 3.2, 3.1, true),
        (28.4, 3.6, 3.1, 2.7, false),
        // Mid-crown dappled tufts (keep inner sky channels open)
        (13.8, -2.6, 2.8, 2.4, false),
        (15.6, 8.8, 2.8, 2.4, false),
        (9.2, 22.4, 2.3, 2.0, false),
        // Sub-crown 3 (lower-central; deep sky cleft above it at y = +6..+10)
        (28.6, 12.4, 3.4, 2.9, true),
        (24.4, 15.8, 3.4, 2.9, false),
        (26.8, 19.6, 3.3, 2.9, true),
        (22.6, 23.2, 3.2, 2.8, false),
        (17.2, 22.8, 2.6, 2.3, false),
        // Sub-crown 4 (lower flank; sky notch between 3 and 4 at y = +24..+26)
        (16.2, 28.6, 3.3, 2.9, true),
        (10.8, 29.6, 3.4, 2.9, false),
        (4.8, 31.6, 3.2, 3.1, true),
    ];
    for (cx, cy, rx, ry, use_soft) in mid_foliage {
        let (px_c, py_c) = pt(cx, cy);
        let style = PaintStyle {
            dark: pal.leaf_deep,
            mid: if use_soft { pal.leaf_soft } else { pal.leaf_mid },
            rim: Some(pal.leaf_rim),
            pen: FOLIAGE_PEN,
            sh_w: 210,
        };
        fill_canopy_lobe(c, [px_c, py_c, rx, ry], style);
    }

    // Pass 4: Sunlit crown highlight puffs (dappled outer tips, survive blur).
    let sun_style = PaintStyle {
        dark: pal.leaf_mid,
        mid: pal.leaf_accent,
        rim: Some(pal.leaf_rim),
        pen: FOLIAGE_PEN,
        sh_w: 198,
    };
    let sun_puffs: [[f32; 4]; 12] = [
        [4.8, -30.2, 2.1, 2.0],
        [9.8, -28.2, 2.3, 2.0],
        [19.2, -25.4, 2.1, 1.9],
        [12.6, -15.4, 1.9, 1.7],
        [28.4, -13.0, 2.5, 2.1],
        [30.4, -8.6, 2.1, 1.9],
        [31.6, -4.4, 2.4, 2.1],
        [33.5, 0.2, 2.3, 2.1],
        [29.2, 12.2, 2.3, 2.0],
        [27.4, 19.6, 2.2, 1.9],
        [16.8, 28.6, 2.2, 1.9],
        [5.6, 31.4, 2.1, 2.0],
    ];
    for [cx, cy, rx, ry] in sun_puffs {
        let (px_c, py_c) = pt(cx, cy);
        fill_canopy_lobe(c, [px_c, py_c, rx, ry], sun_style);
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
    type KelpBlade = ((f32, f32), (f32, f32), (f32, f32), f32);
    let blades: [KelpBlade; 4] = [
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

    fn default_len(spec: ReachSpec) -> i32 {
        match spec.shape {
            ReachShape::Canopy => spec.tip_len.round() as i32,
            ReachShape::Strip => spec.tip_len.round() as i32 + STEM_PERIOD * 2,
        }
    }

    #[test]
    fn allowed_entry_sides_match_owner_spec() {
        assert_eq!(ReachKind::LightPole.allowed_sides(), &[AnchorSide::Bottom]);
        assert_eq!(ReachKind::HangingLamp.allowed_sides(), &[AnchorSide::Top]);
        assert_eq!(ReachKind::CraneHook.allowed_sides(), &[AnchorSide::Top]);

        for kind in [
            ReachKind::TreeCanopy,
            ReachKind::SignArm,
            ReachKind::VineTendril,
            ReachKind::KelpFrond,
            ReachKind::PipeElbow,
        ] {
            assert_eq!(
                kind.allowed_sides(),
                AnchorSide::ALL,
                "{kind:?} should allow all four anchor sides"
            );
        }

        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            assert_eq!(spec.sides, kind.allowed_sides());
            assert_eq!(spec.shape, kind.shape());
            assert!(!spec.sides.is_empty());
        }
        assert_eq!(ReachKind::TreeCanopy.shape(), ReachShape::Canopy);
    }

    #[test]
    fn every_kind_draws_non_empty_body_and_shadow() {
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            assert!(spec.tip_len > 0.0, "{kind:?} tip_len must be positive");
            assert!(spec.thickness > 0.0, "{kind:?} thickness must be positive");
            assert!(!spec.floors.is_empty(), "{kind:?} must appear on at least one floor");

            let len = default_len(spec);
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
            if spec.shape != ReachShape::Strip {
                continue;
            }
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
    fn canopy_coverage_fraction_leaves_light_through_across_sway() {
        let spec = reach_spec(ReachKind::TreeCanopy);
        let len = spec.tip_len.round() as i32;
        let h = spec.thickness.ceil() as usize + 4;
        let cy = h as f32 * 0.5;
        let x_origin = spec.tip_len - len as f32;
        let r2 = CANOPY_RADIUS * CANOPY_RADIUS;

        for &theme in spec.floors {
            for &t in &[0.0_f32, 0.8, 1.5, 2.5, 3.8, 4.7] {
                let (mut body, mut shadow) = make_buffers(len as usize + 4, h, true);
                draw_reach(ReachKind::TreeCanopy, &mut body, &mut shadow, len, t, theme);

                let mut in_disc = 0usize;
                let mut filled = 0usize;
                let mut outside_disc = 0usize;
                let mut wood_px = 0usize;
                let mut foliage_px = 0usize;

                for py in 0..h {
                    let ly = (py as f32 + 0.5) - cy;
                    for px in 0..body.w {
                        let lx = (px as f32 + 0.5) + x_origin;
                        let has_body = body.pixel_at(px, py) != (0, 0, 0);
                        if lx > 0.0 && lx * lx + ly * ly <= r2 {
                            in_disc += 1;
                            if has_body {
                                filled += 1;
                                if body.mat()[py * body.w + px] == WOOD_PEN {
                                    wood_px += 1;
                                } else {
                                    foliage_px += 1;
                                }
                            }
                        } else if has_body {
                            outside_disc += 1;
                        }
                    }
                }

                let frac = filled as f32 / in_disc.max(1) as f32;
                assert_eq!(
                    outside_disc, 0,
                    "TreeCanopy ({theme:?}, t={t}) painted {outside_disc} px outside half-disc"
                );
                assert!(
                    (0.35..=0.65).contains(&frac),
                    "TreeCanopy ({theme:?}, t={t}) coverage {frac:.3} outside 0.35..=0.65"
                );
                assert!(wood_px >= 120, "TreeCanopy needs visible branching limbs ({wood_px})");
                assert!(foliage_px >= 350, "TreeCanopy needs substantial leaf clusters ({foliage_px})");
            }
        }
    }

    #[test]
    fn canopy_trunk_is_never_inside_drawn_extent() {
        const { assert!(CANOPY_TRUNK_X < 0.0, "trunk hub must sit strictly off-screen (x < 0)") };

        let spec = reach_spec(ReachKind::TreeCanopy);
        // Pass len > tip_len to verify local_x <= 0 stays strictly empty even if over-extended.
        let extra_len = spec.tip_len.round() as i32 + 24;
        let h = spec.thickness.ceil() as usize + 4;
        let x_origin = spec.tip_len - extra_len as f32;

        let (mut body, mut shadow) = make_buffers(extra_len as usize + 4, h, true);
        draw_reach(ReachKind::TreeCanopy, &mut body, &mut shadow, extra_len, 0.7, Theme::Trailer);

        for py in 0..h {
            for px in 0..body.w {
                let lx = (px as f32 + 0.5) + x_origin;
                if lx <= 0.0 {
                    assert_eq!(
                        body.pixel_at(px, py),
                        (0, 0, 0),
                        "TreeCanopy painted trunk/stem at local_x={lx} <= 0"
                    );
                    assert_eq!(
                        shadow.pixel_at(px, py),
                        (0, 0, 0),
                        "TreeCanopy painted shadow at local_x={lx} <= 0"
                    );
                }
            }
        }

        // At the anchor edge (first drawn column, local_x = 0.5), primary limbs are
        // splayed apart with open gaps between them — never a solid trunk block.
        let edge_px = (0.0 - x_origin).floor() as usize;
        let mut runs = 0usize;
        let mut in_run = false;
        for py in 0..h {
            let filled = body.pixel_at(edge_px, py) != (0, 0, 0);
            if filled && !in_run {
                runs += 1;
            }
            in_run = filled;
        }
        assert!(
            runs >= 3,
            "expected splayed primary boughs (>= 3 separate runs) at anchor edge, got {runs}"
        );
    }

    #[test]
    fn tip_lies_at_tip_len_and_stays_within_bounds() {
        let pad = 8usize;
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let theme = spec.floors[0];
            let len = match spec.shape {
                ReachShape::Canopy => spec.tip_len.round() as i32,
                ReachShape::Strip => spec.tip_len.round() as i32 + STEM_PERIOD,
            };
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
            let len = match spec.shape {
                ReachShape::Canopy => spec.tip_len.round() as i32,
                ReachShape::Strip => spec.tip_len.round() as i32 + STEM_PERIOD,
            };
            let h = spec.thickness.ceil() as usize + 4;
            let cy = h as f32 * 0.5;
            let x_origin = spec.tip_len - len as f32;

            let sample_centroids = |t: f32| -> (PixelBuffer, f32, f32) {
                let (mut body, mut shadow) = make_buffers(len as usize, h, false);
                draw_reach(kind, &mut body, &mut shadow, len, t, theme);
                let mut sum_out = 0.0_f32;
                let mut cnt_out = 0.0_f32;
                let mut sum_in = 0.0_f32;
                let mut cnt_in = 0.0_f32;
                let outer_start = match spec.shape {
                    ReachShape::Canopy => 0usize,
                    ReachShape::Strip => STEM_PERIOD as usize + (spec.tip_len as usize / 2),
                };
                for py in 0..h {
                    let ly = (py as f32 + 0.5) - cy;
                    for px in outer_start..len as usize {
                        if body.pixel_at(px, py) != (0, 0, 0) {
                            if spec.shape == ReachShape::Canopy {
                                let lx = (px as f32 + 0.5) + x_origin;
                                let r = (lx * lx + ly * ly).sqrt();
                                if r < 14.0 {
                                    sum_in += py as f32;
                                    cnt_in += 1.0;
                                } else if r >= 22.0 {
                                    sum_out += py as f32;
                                    cnt_out += 1.0;
                                }
                            } else {
                                sum_out += py as f32;
                                cnt_out += 1.0;
                            }
                        }
                    }
                }
                (body, sum_in / cnt_in.max(1.0), sum_out / cnt_out.max(1.0))
            };

            if spec.organic {
                let (base_pb, init_in, mut prev_out) = sample_centroids(0.0);
                let (mut min_out, mut max_out) = (prev_out, prev_out);
                let (mut min_in, mut max_in) = (init_in, init_in);

                // Step finely across a full sway cycle: adjacent steps move smoothly.
                for step in 1..=100 {
                    let tv = step as f32 * 0.05;
                    let (pb, c_in, c_out) = sample_centroids(tv);
                    assert!(
                        (c_out - prev_out).abs() < 0.65,
                        "{kind:?} jumped between t={} and t={tv}: {prev_out} -> {c_out}",
                        tv - 0.05
                    );
                    if spec.shape == ReachShape::Strip {
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
                    }
                    min_out = min_out.min(c_out);
                    max_out = max_out.max(c_out);
                    min_in = min_in.min(c_in);
                    max_in = max_in.max(c_in);
                    prev_out = c_out;
                }
                let out_amp = max_out - min_out;
                assert!(
                    out_amp >= 0.9,
                    "{kind:?} organic sway amplitude too small ({min_out}..{max_out})"
                );
                if spec.shape == ReachShape::Canopy {
                    let in_amp = max_in - min_in;
                    assert!(
                        out_amp > in_amp * 2.5,
                        "TreeCanopy outer twigs ({out_amp:.3}) must sway more than inner limbs ({in_amp:.3})"
                    );
                }
            } else {
                let (pb_a, _, _) = sample_centroids(0.0);
                let (pb_b, _, _) = sample_centroids(2.3);
                assert_eq!(
                    pb_a.pixels(),
                    pb_b.pixels(),
                    "{kind:?} is man-made and should not sway with t"
                );
            }
        }
    }

    #[test]
    fn draw_reach_is_deterministic_across_kinds_and_themes() {
        for kind in ReachKind::all() {
            let spec = reach_spec(kind);
            let len = default_len(spec);
            let h = spec.thickness.ceil() as usize + 4;
            for &theme in spec.floors {
                let (mut body_a, mut shadow_a) = make_buffers(len as usize + 4, h, true);
                let (mut body_b, mut shadow_b) = make_buffers(len as usize + 4, h, true);
                draw_reach(kind, &mut body_a, &mut shadow_a, len, 1.73, theme);
                draw_reach(kind, &mut body_b, &mut shadow_b, len, 1.73, theme);
                assert_eq!(body_a.pixels(), body_b.pixels(), "{kind:?} ({theme:?}) body non-deterministic");
                assert_eq!(body_a.mat(), body_b.mat(), "{kind:?} ({theme:?}) mat non-deterministic");
                assert_eq!(shadow_a.pixels(), shadow_b.pixels(), "{kind:?} ({theme:?}) shadow non-deterministic");
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
