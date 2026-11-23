//! Art for the foreground layer v2 ([`crate::render::foreground`]).
//!
//! Draws eighteen large, dark, soft-edged overhead silhouettes (three per floor
//! theme) that sit close to the viewer and slide past the playfield with strong
//! parallax while casting soft floor shadows that stay with the world (1:1).
//!
//! ## Contract
//!
//! ```ignore
//! pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw)
//! pub fn extent(item: &ForeItem) -> (f32, f32)
//! ```
//!
//! * Every pixel write goes exclusively through [`crate::render::foreground::shadow`]
//!   (drawn first, shifted to `d.shadow`, strength `<= 0.35`, `<= 0.30` for
//!   [`ForeKind::Cloud`], 4 px soft edge) and [`crate::render::foreground::blend`]
//!   (drawn second, at `d.screen`, 2 px soft alpha edge, rim highlight in
//!   [`crate::render::foreground::rim_tint`] on the `d.toward_centre` side).
//! * **Upscaled by the layer.** In play the layer lends both writers an
//!   offscreen scratch ([`crate::render::foreground::target_rect`] is its window,
//!   in art coordinates), and blits it nearest-neighbour at an integer 2 or 3,
//!   so every pixel drawn here is a `k x k` block on screen. Nothing here knows:
//!   the art draws at `d.scale` around `d.screen` exactly as before, clipped to
//!   the target rect, and a Span/Edge is handed `len / k` so it still reaches its
//!   walls. Draw thin things one pixel wide; the upscale makes them chunky.
//! * Bodies are a **mid-dark per-floor colour lighter than the floor**, a darker
//!   underside/far edge and a rim about 35 % lighter on the `toward_centre` side
//!   ([`ForePal::for_theme`]); the near-black v2 bodies vanished on dark floors.
//! * Rasterized through a stack-allocated scanline strip buffer (zero heap
//!   allocations per frame, no per-pixel `sqrt`/trig, single write per pixel per
//!   pass so effective opacity never double-blends or exceeds `d.alpha`).
//! * Every item shows its attachment:
//!   - `Attach::Span` reaches `-len/2 ..= +len/2` with wall collars/brackets/knots
//!     at both ends;
//!   - `Attach::Edge(side)` roots solidly into `side` and tapers `len` px into
//!     the open;
//!   - `Attach::Drop` hangs from a chain/cord/cords running straight up `-y` for
//!     [`DROP_TAIL`] (400 px);
//!   - `Attach::Sky` renders the outdoor Flatbed movers (`TreeBranch`, `Cloud`,
//!     `PowerLine`).

use crate::game::level::Theme;
use crate::render::foreground::{
    blend, rim_tint, shadow, target_rect, Attach, ForeDraw, ForeItem, ForeKind, Side, ALPHA_MAX,
    DROP_TAIL, WIRE_RGB,
};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Width of one horizontal scanline strip on the stack. Wider buffers loop in
/// 256 px tiles so arbitrary terminal widths work with zero heap allocation.
const STRIP_W: usize = 256;
/// Stack capacity for primitives per pass (max emitted by any kind is <= 36).
const MAX_SH_PRIMS: usize = 64;
const MAX_BD_PRIMS: usize = 64;

/// 16-point unit circle lookup table for organic leaf lobes (zero trig).
const UNIT_RING: [(f32, f32); 16] = [
    (1.000, 0.000),
    (0.924, 0.383),
    (0.707, 0.707),
    (0.383, 0.924),
    (0.000, 1.000),
    (-0.383, 0.924),
    (-0.707, 0.707),
    (-0.924, 0.383),
    (-1.000, 0.000),
    (-0.924, -0.383),
    (-0.707, -0.707),
    (-0.383, -0.924),
    (0.000, -1.000),
    (0.383, -0.924),
    (0.707, -0.707),
    (0.924, -0.383),
];

/// Half-extent `(long, across)` in world px **including** `item.scale`, long
/// axis first. For [`Attach::Drop`] kinds this bounds the hanging body
/// (including sway) and excludes the [`DROP_TAIL`] suspension line.
pub fn extent(item: &ForeItem) -> (f32, f32) {
    let s = item.scale.max(0.25);
    match item.kind {
        // ---- Sewer ----
        ForeKind::MainPipe => (span_half(item, 44.0, s), 10.6 * s),
        ForeKind::CatwalkGrating => (span_half(item, 44.0, s), 6.8 * s),
        ForeKind::ChainHoist => {
            if item.horizontal {
                (6.2 * s, 10.5 * s)
            } else {
                (10.5 * s, 6.2 * s)
            }
        }
        // ---- Kitchen ----
        ForeKind::ExtractorHood => (edge_reach(item, 18.0, s), 14.0 * s),
        ForeKind::PotRail => (span_half(item, 44.0, s), 10.5 * s),
        ForeKind::HangingLamp => (10.5 * s, 6.2 * s),
        // ---- Lab ----
        ForeKind::CableTray => (span_half(item, 44.0, s), 8.2 * s),
        ForeKind::VentDuct => (span_half(item, 44.0, s), 5.8 * s),
        ForeKind::ArticulatedArm => (edge_reach(item, 22.0, s), 11.0 * s),
        // ---- Laundry ----
        ForeKind::Clothesline => (span_half(item, 44.0, s), 11.2 * s),
        ForeKind::SteamPipe => (span_half(item, 44.0, s), 12.2 * s),
        ForeKind::DryingRack => (13.0 * s, 6.8 * s),
        // ---- Printworks ----
        ForeKind::PaperWeb => (span_half(item, 44.0, s), 7.0 * s),
        ForeKind::RollerBeam => (span_half(item, 44.0, s), 6.0 * s),
        ForeKind::HangingProofs => (14.0 * s, 10.0 * s),
        // ---- Flatbed (Outdoors) ----
        ForeKind::TreeBranch => {
            let reach = edge_reach(item, 24.0, s);
            (reach, (reach * 0.62).max(12.0 * s))
        }
        ForeKind::Cloud => {
            let rx = cloud_base_rx(item.seed) * s;
            (rx * 1.15, rx * 0.42 * 1.15)
        }
        ForeKind::PowerLine => {
            if is_power_pole(item) {
                (23.0 * s, 12.0 * s)
            } else {
                let hl = power_wire_half(item, s);
                let hv = (4.2 * s + (3.4 * s).min(22.0) + 2.5).max(8.0);
                (hl, hv)
            }
        }
    }
}

/// Draw one foreground item into `pb`: soft floor shadow at `d.shadow` first
/// (via [`shadow`]), then the dark soft-edged silhouette at `d.screen` (via
/// [`blend`]).
///
/// While the layer is upscaling, the two writers land in its scratch and
/// [`target_rect`] is the scratch's window (art coordinates); otherwise it is
/// the whole of `pb`. Primitives are clipped to it.
pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw) {
    let (tx0, ty0, tw, th) = target_rect(pb);
    if d.alpha == 0 || tw <= 0 || th <= 0 {
        return;
    }
    let scale = d.scale.max(0.25);
    let sway = d.sway.clamp(-1.0, 1.0);
    let pal = ForePal::for_theme(d.theme);
    let tc = norm_tc(d.toward_centre);
    let alpha_norm = (d.alpha as f32 / (ALPHA_MAX as f32).max(1.0)).clamp(0.0, 1.0);

    let (body_alpha, max_shadow) = if item.kind == ForeKind::Cloud {
        let ca = d.alpha.min(52);
        let cloud_norm = (d.alpha as f32 / 60.0).clamp(0.0, 1.0);
        (ca.max(1), 0.30 * (0.75 + 0.25 * cloud_norm))
    } else {
        (
            d.alpha.min(ALPHA_MAX),
            0.35 * (0.50 + 0.50 * alpha_norm),
        )
    };

    let ctx = DrawCtx {
        sc: (d.screen.0 as f32, d.screen.1 as f32),
        sh: (d.shadow.0 as f32, d.shadow.1 as f32),
        s: scale,
        sway,
        seed: item.seed,
        pal,
        tc,
    };

    let mut list = PrimList::new(tx0, ty0, tw, th);

    match item.kind {
        // ---- Sewer ----
        ForeKind::MainPipe => {
            let hl = span_half(item, 44.0, scale);
            emit_main_pipe(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::CatwalkGrating => {
            let hl = span_half(item, 44.0, scale);
            emit_catwalk_grating(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::ChainHoist => {
            emit_chain_hoist(&mut list, &ctx);
        }
        // ---- Kitchen ----
        ForeKind::ExtractorHood => {
            let side = resolve_edge_side(item, d.toward_centre, false);
            let reach = edge_reach(item, 18.0, scale);
            emit_extractor_hood(&mut list, &ctx, side, reach);
        }
        ForeKind::PotRail => {
            let hl = span_half(item, 44.0, scale);
            emit_pot_rail(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::HangingLamp => {
            emit_hanging_lamp(&mut list, &ctx);
        }
        // ---- Lab ----
        ForeKind::CableTray => {
            let hl = span_half(item, 44.0, scale);
            emit_cable_tray(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::VentDuct => {
            let hl = span_half(item, 44.0, scale);
            emit_vent_duct(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::ArticulatedArm => {
            let side = resolve_edge_side(item, d.toward_centre, false);
            let reach = edge_reach(item, 22.0, scale);
            emit_articulated_arm(&mut list, &ctx, side, reach);
        }
        // ---- Laundry ----
        ForeKind::Clothesline => {
            let hl = span_half(item, 44.0, scale);
            emit_clothesline(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::SteamPipe => {
            let hl = span_half(item, 44.0, scale);
            emit_steam_pipe(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::DryingRack => {
            emit_drying_rack(&mut list, &ctx);
        }
        // ---- Printworks ----
        ForeKind::PaperWeb => {
            let hl = span_half(item, 44.0, scale);
            emit_paper_web(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::RollerBeam => {
            let hl = span_half(item, 44.0, scale);
            emit_roller_beam(&mut list, &ctx, hl, item.horizontal);
        }
        ForeKind::HangingProofs => {
            emit_hanging_proofs(&mut list, &ctx);
        }
        // ---- Flatbed (Outdoors) ----
        ForeKind::TreeBranch => {
            let side = resolve_edge_side(item, d.toward_centre, true);
            let reach = edge_reach(item, 24.0, scale);
            emit_tree_branch(&mut list, &ctx, side, reach);
        }
        ForeKind::Cloud => {
            emit_cloud(&mut list, &ctx);
        }
        ForeKind::PowerLine => {
            if is_power_pole(item) {
                emit_power_pole(&mut list, &ctx);
            } else {
                let hl = power_wire_half(item, scale);
                emit_power_wires(&mut list, &ctx, hl, item.horizontal, item.attach == Attach::Span);
            }
        }
    }

    // Pass 1: floor shadows (always before body).
    list.rasterize_shadows(pb, max_shadow);
    // Pass 2: dark overhead silhouette body.
    list.rasterize_body(pb, body_alpha);
}

// ---- Shared helpers & palette ----------------------------------------------

#[derive(Clone, Copy)]
struct ForePal {
    deep: Rgb,
    mid: Rgb,
    soft: Rgb,
    accent: Rgb,
    pale: Rgb,
    rim: Rgb,
    bark_deep: Rgb,
    bark_mid: Rgb,
}

/// How much lighter than the body its rim highlight is (about 35 %).
const RIM_LIGHTEN: f32 = 1.35;
/// The underside / far-edge shade, as a share of the body colour.
const DEEP_SHADE: f32 = 0.70;
/// The lit-side face, as a share of the body colour.
const SOFT_LIFT: f32 = 1.16;
/// How much of the floor's ambient character tints the rim.
const RIM_TINT_MIX: f32 = 0.15;

impl ForePal {
    /// The floor's foreground palette. Batch 13c: the v2 bodies were near-black
    /// and murky on the dark floors (Kitchen, Laundry, Sewer, Printworks), so
    /// each floor now has a **mid-dark body colour clearly lighter than its
    /// floor** (`mid`), a darker shade for the underside and far edge
    /// ([`DEEP_SHADE`]), a lit face ([`SOFT_LIFT`]) and a rim about 35 %
    /// lighter ([`RIM_LIGHTEN`], lightly tinted by the floor's ambient), which
    /// the emitters already put on the `toward_centre` side. With the layer's
    /// `DARKEN` (0.88) and alpha clamp it still reads as a dim overhead thing.
    fn for_theme(theme: Theme) -> Self {
        let rt = rim_tint(theme);
        let peak = rt.0.max(rt.1).max(rt.2).max(1) as f32;
        let gain = (195.0 / peak).clamp(1.0, 2.6);
        let tint = palette::scale(rt, gain);
        let shades = |body: Rgb| -> (Rgb, Rgb, Rgb, Rgb) {
            let rim = palette::mix(palette::scale(body, RIM_LIGHTEN), tint, RIM_TINT_MIX);
            (
                palette::scale(body, DEEP_SHADE),
                body,
                palette::scale(body, SOFT_LIFT),
                rim,
            )
        };
        let build = |body: Rgb, accent: Rgb, pale: Rgb, bark: Rgb| -> Self {
            let (deep, mid, soft, rim) = shades(body);
            Self {
                deep,
                mid,
                soft,
                accent,
                pale,
                rim,
                bark_deep: palette::scale(bark, DEEP_SHADE),
                bark_mid: bark,
            }
        };
        match theme {
            // Wet iron.
            Theme::Sewer => build((58, 74, 70), (128, 78, 44), (152, 174, 166), (58, 74, 70)),
            // Warm steel.
            Theme::Kitchen => build((96, 84, 70), (166, 102, 56), (184, 166, 140), (96, 84, 70)),
            // Cold grey-blue.
            Theme::Lab => build((70, 84, 104), (82, 154, 178), (156, 188, 204), (70, 84, 104)),
            // Pale enamel.
            Theme::Laundry => {
                build((120, 126, 136), (144, 112, 82), (196, 206, 214), (120, 126, 136))
            }
            // Oiled steel.
            Theme::Printworks => {
                build((92, 80, 84), (76, 144, 152), (188, 186, 174), (92, 80, 84))
            }
            // Outdoors: leaves in two greens, bark for the pole and the limb.
            Theme::Trailer => {
                let leaf_deep: Rgb = (40, 78, 44);
                let leaf: Rgb = (64, 108, 58);
                Self {
                    deep: leaf_deep,
                    mid: leaf,
                    soft: palette::scale(leaf, SOFT_LIFT),
                    accent: (98, 140, 64),
                    pale: (182, 194, 204),
                    rim: palette::mix(palette::scale(leaf, RIM_LIGHTEN), tint, RIM_TINT_MIX),
                    bark_deep: palette::scale((64, 48, 34), DEEP_SHADE),
                    bark_mid: (64, 48, 34),
                }
            }
        }
    }
}

struct DrawCtx {
    sc: (f32, f32),
    sh: (f32, f32),
    s: f32,
    sway: f32,
    seed: u32,
    pal: ForePal,
    tc: (i8, i8),
}

#[inline]
fn span_half(item: &ForeItem, default_len_at_1: f32, s: f32) -> f32 {
    if item.len > 0.0 {
        (item.len * 0.5).max(4.0)
    } else {
        (default_len_at_1 * 0.5 * s).max(4.0)
    }
}

#[inline]
fn edge_reach(item: &ForeItem, default_reach_at_1: f32, s: f32) -> f32 {
    if item.len > 0.0 {
        item.len.max(10.0)
    } else {
        (default_reach_at_1 * s).max(10.0)
    }
}

#[inline]
fn is_power_pole(item: &ForeItem) -> bool {
    item.len <= 0.0 && !item.horizontal
}

#[inline]
fn power_wire_half(item: &ForeItem, s: f32) -> f32 {
    if item.len > 0.0 {
        (item.len * 0.5).max(8.0)
    } else {
        140.0 + 12.0 * s
    }
}

#[inline]
fn cloud_base_rx(seed: u32) -> f32 {
    let h = hash_u32(seed ^ 0xC10D);
    82.0 + (h & 63) as f32 * (46.0 / 63.0)
}

#[inline]
fn resolve_edge_side(item: &ForeItem, toward_centre: (i8, i8), prefer_ns: bool) -> Side {
    match item.attach {
        Attach::Edge(side) => side,
        _ => {
            if prefer_ns || !item.horizontal {
                if toward_centre.1 < 0 {
                    Side::S
                } else {
                    Side::N
                }
            } else if toward_centre.0 < 0 {
                Side::E
            } else {
                Side::W
            }
        }
    }
}

#[inline]
fn edge_basis(side: Side) -> (f32, f32, f32, f32) {
    match side {
        Side::N => (0.0, 1.0, 1.0, 0.0),
        Side::S => (0.0, -1.0, 1.0, 0.0),
        Side::W => (1.0, 0.0, 0.0, 1.0),
        Side::E => (-1.0, 0.0, 0.0, 1.0),
    }
}

#[inline]
fn norm_tc(tc: (i8, i8)) -> (i8, i8) {
    if tc.0 == 0 && tc.1 == 0 {
        (1, 1)
    } else {
        tc
    }
}

#[inline]
fn uv_to_xy(o: (f32, f32), horiz: bool, u: f32, v: f32) -> (f32, f32) {
    if horiz {
        (o.0 + u, o.1 + v)
    } else {
        (o.0 + v, o.1 + u)
    }
}

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

#[inline]
fn hash_slot(slot: i32, seed: u32) -> u32 {
    hash_u32((slot as u32).wrapping_mul(0x9E37_79B1) ^ seed)
}

#[inline]
fn mul_alpha(base: u8, weight: u8) -> u8 {
    ((base as u16 * weight as u16 + 127) / 255) as u8
}

#[inline]
fn soft_u8(dist_in: f32, feather: f32, max_w: u8) -> u8 {
    if dist_in <= 0.0 {
        0
    } else if dist_in >= feather {
        max_w
    } else {
        ((dist_in / feather) * max_w as f32 + 0.5) as u8
    }
}

#[inline]
fn tri_wave(x: f32) -> f32 {
    let f = x - x.floor();
    if f < 0.5 {
        f * 4.0 - 1.0
    } else {
        3.0 - f * 4.0
    }
}

#[inline]
fn stamp_sh_px(sh: &mut [u8; STRIP_W], idx: usize, w: u8) {
    if w > sh[idx] {
        sh[idx] = w;
    }
}

#[inline]
fn paint_bd_px(col: &mut [Rgb; STRIP_W], alp: &mut [u8; STRIP_W], idx: usize, rgb: Rgb, w: u8) {
    if w == 0 {
        return;
    }
    let old = alp[idx];
    if old == 0 || w >= 250 {
        col[idx] = rgb;
        alp[idx] = old.max(w);
    } else {
        col[idx] = palette::mix(col[idx], rgb, w as f32 * (1.0 / 255.0));
        alp[idx] = old.max(w);
    }
}

// ---- Scanline primitive definitions & stack rasterizer ---------------------

#[derive(Clone, Copy)]
enum SheetMode {
    Bedsheet,
    Shirt,
    Proof,
}

#[derive(Clone, Copy)]
struct PrimBox {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

#[derive(Clone, Copy)]
struct ShPrim {
    bb: PrimBox,
    kind: ShKind,
}

#[derive(Clone, Copy)]
enum ShKind {
    Empty,
    Rect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        w: u8,
        feather: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        inv_rx2: f32,
        inv_ry2: f32,
        r_eff: f32,
        w: u8,
        feather: f32,
    },
    Capsule {
        ax: f32,
        ay: f32,
        ra: f32,
        dx: f32,
        dy: f32,
        dr: f32,
        inv_len2: f32,
        rmax: f32,
        w: u8,
        feather: f32,
    },
    Grating {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        rail: f32,
        end_w: f32,
        str_v: f32,
        str_w: f32,
        bar_pitch: f32,
        bar_w: f32,
    },
    CableTray {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        rail: f32,
        cw: f32,
        rung_step: f32,
        rung_w: f32,
        s: f32,
    },
    HoodTrap {
        o: (f32, f32),
        side: Side,
        root_back: f32,
        reach: f32,
        w_root: f32,
        w_tip: f32,
    },
    DrapedSheet {
        o: (f32, f32),
        horiz: bool,
        u_center: f32,
        v_top: f32,
        w_half: f32,
        h_len: f32,
        sw_bot: f32,
        mode: SheetMode,
    },
    LeafCluster {
        lobes: [(f32, f32, f32, f32, f32); 5],
        w: u8,
    },
    PowerWires {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        spacing: f32,
        sag: f32,
        wire_r: f32,
    },
    HoleCutCircle {
        cx: f32,
        cy: f32,
        r2: f32,
    },
}

#[derive(Clone, Copy)]
struct BdPrim {
    bb: PrimBox,
    kind: BdKind,
}

#[derive(Clone, Copy)]
enum BdKind {
    Empty,
    Rect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        rgb: Rgb,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        inv_rx2: f32,
        inv_ry2: f32,
        r_eff: f32,
        rgb: Rgb,
        inner: Option<Rgb>,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    },
    Capsule {
        ax: f32,
        ay: f32,
        ra: f32,
        dx: f32,
        dy: f32,
        dr: f32,
        inv_len2: f32,
        rmax: f32,
        dark: Rgb,
        mid: Rgb,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    },
    DonutRing {
        cx: f32,
        cy: f32,
        vr: f32,
        ir: f32,
        dark: Rgb,
        soft: Rgb,
        rim: Rgb,
        tc: (i8, i8),
    },
    PipeShaft {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        sr: f32,
        fr: f32,
        cw: f32,
        fw: f32,
        flange_step: f32,
        s: f32,
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    Grating {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        rail: f32,
        end_w: f32,
        str_v: f32,
        str_w: f32,
        bar_pitch: f32,
        bar_w: f32,
        s: f32,
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    ChainTail {
        x_bot: f32,
        y_bot: f32,
        y_top: f32,
        link_w: f32,
        link_h: f32,
        sway_dx: f32,
        pal: ForePal,
        tc: (i8, i8),
    },
    HoodTrap {
        o: (f32, f32),
        side: Side,
        root_back: f32,
        reach: f32,
        w_root: f32,
        w_tip: f32,
        s: f32,
        pal: ForePal,
        tc: (i8, i8),
    },
    CableTray {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        rail: f32,
        cw: f32,
        rung_step: f32,
        rung_w: f32,
        s: f32,
        pal: ForePal,
        tc: (i8, i8),
    },
    VentDuct {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        seam_hw: f32,
        cw: f32,
        seam_step: f32,
        seam_w: f32,
        has_louvre: bool,
        s: f32,
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    SteamPipe {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        sr: f32,
        strap_r: f32,
        cw: f32,
        strap_step: f32,
        strap_w: f32,
        leak_u: f32,
        has_leak: bool,
        s: f32,
        pal: ForePal,
        tc: (i8, i8),
    },
    PaperWeb {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        rol_hw: f32,
        cw: f32,
        rol_step: f32,
        rol_w: f32,
        s: f32,
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    RollerBeam {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        hw: f32,
        flange: f32,
        cw: f32,
        car_u: f32,
        car_hu: f32,
        s: f32,
        pal: ForePal,
        tc: (i8, i8),
    },
    DrapedSheet {
        o: (f32, f32),
        horiz: bool,
        u_center: f32,
        v_top: f32,
        w_half: f32,
        h_len: f32,
        sw_bot: f32,
        mode: SheetMode,
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    LeafCluster {
        cx: f32,
        cy: f32,
        min_r: f32,
        lobes: [(f32, f32, f32, f32, f32); 5],
        seed: u32,
        pal: ForePal,
        tc: (i8, i8),
    },
    PowerWires {
        o: (f32, f32),
        horiz: bool,
        hl: f32,
        spacing: f32,
        sag: f32,
        wire_r: f32,
        rim: Rgb,
        tc: (i8, i8),
    },
    HoleCutCircle {
        cx: f32,
        cy: f32,
        r2: f32,
    },
}

struct PrimList {
    /// The clip window: `[bx0, bx0 + bw) x [by0, by0 + bh)`.
    bx0: i32,
    by0: i32,
    bw: i32,
    bh: i32,
    sh: [ShPrim; MAX_SH_PRIMS],
    n_sh: usize,
    bd: [BdPrim; MAX_BD_PRIMS],
    n_bd: usize,
    sh_bb: PrimBox,
    bd_bb: PrimBox,
}

impl PrimList {
    fn new(bx0: i32, by0: i32, bw: i32, bh: i32) -> Self {
        let empty_bb = PrimBox {
            x0: bx0 + bw,
            y0: by0 + bh,
            x1: bx0 - 1,
            y1: by0 - 1,
        };
        Self {
            bx0,
            by0,
            bw,
            bh,
            sh: [ShPrim {
                bb: empty_bb,
                kind: ShKind::Empty,
            }; MAX_SH_PRIMS],
            n_sh: 0,
            bd: [BdPrim {
                bb: empty_bb,
                kind: BdKind::Empty,
            }; MAX_BD_PRIMS],
            n_bd: 0,
            sh_bb: empty_bb,
            bd_bb: empty_bb,
        }
    }

    #[inline]
    fn clip_bb(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Option<PrimBox> {
        let ix0 = (x0.floor() as i32).max(self.bx0);
        let iy0 = (y0.floor() as i32).max(self.by0);
        let ix1 = (x1.ceil() as i32).min(self.bx0 + self.bw - 1);
        let iy1 = (y1.ceil() as i32).min(self.by0 + self.bh - 1);
        if ix0 > ix1 || iy0 > iy1 {
            None
        } else {
            Some(PrimBox {
                x0: ix0,
                y0: iy0,
                x1: ix1,
                y1: iy1,
            })
        }
    }

    fn push_sh(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, kind: ShKind) {
        if self.n_sh >= MAX_SH_PRIMS {
            return;
        }
        let Some(bb) = self.clip_bb(x0, y0, x1, y1) else {
            return;
        };
        self.sh[self.n_sh] = ShPrim { bb, kind };
        self.n_sh += 1;
        self.sh_bb.x0 = self.sh_bb.x0.min(bb.x0);
        self.sh_bb.y0 = self.sh_bb.y0.min(bb.y0);
        self.sh_bb.x1 = self.sh_bb.x1.max(bb.x1);
        self.sh_bb.y1 = self.sh_bb.y1.max(bb.y1);
    }

    fn push_bd(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, kind: BdKind) {
        if self.n_bd >= MAX_BD_PRIMS {
            return;
        }
        let Some(bb) = self.clip_bb(x0, y0, x1, y1) else {
            return;
        };
        self.bd[self.n_bd] = BdPrim { bb, kind };
        self.n_bd += 1;
        self.bd_bb.x0 = self.bd_bb.x0.min(bb.x0);
        self.bd_bb.y0 = self.bd_bb.y0.min(bb.y0);
        self.bd_bb.x1 = self.bd_bb.x1.max(bb.x1);
        self.bd_bb.y1 = self.bd_bb.y1.max(bb.y1);
    }

    // ---- Convenience emitters ----

    fn sh_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: u8, feather: f32) {
        self.push_sh(
            x0,
            y0,
            x1,
            y1,
            ShKind::Rect {
                x0,
                y0,
                x1,
                y1,
                w,
                feather,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn sh_rect_uv(
        &mut self,
        o: (f32, f32),
        horiz: bool,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
        w: u8,
        feather: f32,
    ) {
        if horiz {
            self.sh_rect(o.0 + u0, o.1 + v0, o.0 + u1, o.1 + v1, w, feather);
        } else {
            self.sh_rect(o.0 + v0, o.1 + u0, o.0 + v1, o.1 + u1, w, feather);
        }
    }

    fn sh_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, w: u8, feather: f32) {
        if rx <= 0.5 || ry <= 0.5 {
            return;
        }
        self.push_sh(
            cx - rx,
            cy - ry,
            cx + rx,
            cy + ry,
            ShKind::Ellipse {
                cx,
                cy,
                rx,
                inv_rx2: 1.0 / (rx * rx),
                inv_ry2: 1.0 / (ry * ry),
                r_eff: rx.min(ry),
                w,
                feather,
            },
        );
    }

    fn sh_capsule(&mut self, a: (f32, f32), ra: f32, b: (f32, f32), rb: f32, w: u8, feather: f32) {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len2 = dx * dx + dy * dy;
        let inv_len2 = if len2 > 1e-4 { 1.0 / len2 } else { 0.0 };
        let rmax = ra.max(rb) + 1.0;
        self.push_sh(
            a.0.min(b.0) - rmax,
            a.1.min(b.1) - rmax,
            a.0.max(b.0) + rmax,
            a.1.max(b.1) + rmax,
            ShKind::Capsule {
                ax: a.0,
                ay: a.1,
                ra,
                dx,
                dy,
                dr: rb - ra,
                inv_len2,
                rmax,
                w,
                feather,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn bd_rect(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        rgb: Rgb,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        self.push_bd(
            x0,
            y0,
            x1,
            y1,
            BdKind::Rect {
                x0,
                y0,
                x1,
                y1,
                rgb,
                rim,
                tc,
                w,
                feather,
                rim_w,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn bd_rect_uv(
        &mut self,
        o: (f32, f32),
        horiz: bool,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
        rgb: Rgb,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        if horiz {
            self.bd_rect(o.0 + u0, o.1 + v0, o.0 + u1, o.1 + v1, rgb, rim, tc, w, feather, rim_w);
        } else {
            self.bd_rect(o.0 + v0, o.1 + u0, o.0 + v1, o.1 + u1, rgb, rim, tc, w, feather, rim_w);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn bd_ellipse(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        rgb: Rgb,
        inner: Option<Rgb>,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        if rx <= 0.5 || ry <= 0.5 {
            return;
        }
        self.push_bd(
            cx - rx,
            cy - ry,
            cx + rx,
            cy + ry,
            BdKind::Ellipse {
                cx,
                cy,
                rx,
                inv_rx2: 1.0 / (rx * rx),
                inv_ry2: 1.0 / (ry * ry),
                r_eff: rx.min(ry),
                rgb,
                inner,
                rim,
                tc,
                w,
                feather,
                rim_w,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn bd_capsule(
        &mut self,
        a: (f32, f32),
        ra: f32,
        b: (f32, f32),
        rb: f32,
        dark: Rgb,
        mid: Rgb,
        rim: Option<Rgb>,
        tc: (i8, i8),
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len2 = dx * dx + dy * dy;
        let inv_len2 = if len2 > 1e-4 { 1.0 / len2 } else { 0.0 };
        let rmax = ra.max(rb) + 1.0;
        self.push_bd(
            a.0.min(b.0) - rmax,
            a.1.min(b.1) - rmax,
            a.0.max(b.0) + rmax,
            a.1.max(b.1) + rmax,
            BdKind::Capsule {
                ax: a.0,
                ay: a.1,
                ra,
                dx,
                dy,
                dr: rb - ra,
                inv_len2,
                rmax,
                dark,
                mid,
                rim,
                tc,
                w,
                feather,
                rim_w,
            },
        );
    }

    // ---- Scanline rasterizers ----

    fn rasterize_shadows(&self, pb: &mut PixelBuffer, max_shadow: f32) {
        if self.n_sh == 0 || self.sh_bb.x0 > self.sh_bb.x1 || self.sh_bb.y0 > self.sh_bb.y1 {
            return;
        }
        let sh_scale = max_shadow * (1.0 / 255.0);
        let mut sh_row = [0u8; STRIP_W];
        let mut tile_x0 = self.sh_bb.x0;
        while tile_x0 <= self.sh_bb.x1 {
            let tile_x1 = (tile_x0 + STRIP_W as i32 - 1).min(self.sh_bb.x1);
            let span_len = (tile_x1 - tile_x0 + 1) as usize;
            for y in self.sh_bb.y0..=self.sh_bb.y1 {
                sh_row[..span_len].fill(0);
                for p in &self.sh[..self.n_sh] {
                    if y < p.bb.y0 || y > p.bb.y1 || p.bb.x1 < tile_x0 || p.bb.x0 > tile_x1 {
                        continue;
                    }
                    eval_sh_row(p, y, tile_x0, tile_x1, &mut sh_row);
                }
                for (i, &val) in sh_row[..span_len].iter().enumerate() {
                    if val > 0 {
                        shadow(pb, tile_x0 + i as i32, y, val as f32 * sh_scale);
                    }
                }
            }
            tile_x0 = tile_x1 + 1;
        }
    }

    fn rasterize_body(&self, pb: &mut PixelBuffer, body_alpha: u8) {
        if self.n_bd == 0
            || body_alpha == 0
            || self.bd_bb.x0 > self.bd_bb.x1
            || self.bd_bb.y0 > self.bd_bb.y1
        {
            return;
        }
        let mut alp_row = [0u8; STRIP_W];
        let mut col_row = [(0u8, 0u8, 0u8); STRIP_W];
        let mut tile_x0 = self.bd_bb.x0;
        while tile_x0 <= self.bd_bb.x1 {
            let tile_x1 = (tile_x0 + STRIP_W as i32 - 1).min(self.bd_bb.x1);
            let span_len = (tile_x1 - tile_x0 + 1) as usize;
            for y in self.bd_bb.y0..=self.bd_bb.y1 {
                alp_row[..span_len].fill(0);
                for p in &self.bd[..self.n_bd] {
                    if y < p.bb.y0 || y > p.bb.y1 || p.bb.x1 < tile_x0 || p.bb.x0 > tile_x1 {
                        continue;
                    }
                    eval_bd_row(p, y, tile_x0, tile_x1, &mut col_row, &mut alp_row);
                }
                for i in 0..span_len {
                    let wa = alp_row[i];
                    if wa > 0 {
                        let a = mul_alpha(body_alpha, wa);
                        if a > 0 {
                            blend(pb, tile_x0 + i as i32, y, col_row[i], a);
                        }
                    }
                }
            }
            tile_x0 = tile_x1 + 1;
        }
    }
}

// ---- Per-row primitive evaluators (zero inner-loop sqrt/trig) --------------

#[inline]
fn capsule_row_x_bounds(
    a: (f32, f32),
    d: (f32, f32),
    rmax: f32,
    yf: f32,
    tile: (i32, i32),
) -> (i32, i32) {
    let (ax, ay) = a;
    let (dx, dy) = d;
    let (x_lo, x_hi) = if dy.abs() > 0.5 {
        let inv_dy = 1.0 / dy;
        let t0 = ((yf - rmax - ay) * inv_dy).clamp(0.0, 1.0);
        let t1 = ((yf + rmax - ay) * inv_dy).clamp(0.0, 1.0);
        let xa = ax + t0 * dx;
        let xb = ax + t1 * dx;
        (xa.min(xb) - rmax, xa.max(xb) + rmax)
    } else {
        (ax.min(ax + dx) - rmax, ax.max(ax + dx) + rmax)
    };
    (
        (x_lo.floor() as i32).max(tile.0),
        (x_hi.ceil() as i32).min(tile.1),
    )
}

fn eval_sh_row(p: &ShPrim, y: i32, tile_x0: i32, tile_x1: i32, sh_row: &mut [u8; STRIP_W]) {
    let yf = y as f32;
    let xa_clip = p.bb.x0.max(tile_x0);
    let xb_clip = p.bb.x1.min(tile_x1);

    match p.kind {
        ShKind::Empty => {}
        ShKind::Rect {
            x0,
            y0,
            x1,
            y1,
            w,
            feather,
        } => {
            let dy = (yf - y0).min(y1 - yf);
            if dy <= 0.0 {
                return;
            }
            for x in xa_clip..=xb_clip {
                let xf = x as f32;
                let dx = (xf - x0).min(x1 - xf);
                if dx > 0.0 {
                    stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(dx.min(dy), feather, w));
                }
            }
        }
        ShKind::Ellipse {
            cx,
            cy,
            rx,
            inv_rx2,
            inv_ry2,
            r_eff,
            w,
            feather,
        } => {
            let dy = yf - cy;
            let ny2 = dy * dy * inv_ry2;
            if ny2 >= 1.0 {
                return;
            }
            let span_x = rx * (1.0 - ny2).sqrt();
            let xa = ((cx - span_x).floor() as i32).max(xa_clip);
            let xb = ((cx + span_x).ceil() as i32).min(xb_clip);
            for x in xa..=xb {
                let dx = x as f32 - cx;
                let e2 = dx * dx * inv_rx2 + ny2;
                if e2 < 1.0 {
                    let dist_in = (1.0 - e2) * r_eff * 0.5;
                    stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(dist_in, feather, w));
                }
            }
        }
        ShKind::Capsule {
            ax,
            ay,
            ra,
            dx,
            dy,
            dr,
            inv_len2,
            rmax,
            w,
            feather,
        } => {
            let (xa, xb) = capsule_row_x_bounds((ax, ay), (dx, dy), rmax, yf, (xa_clip, xb_clip));
            if xa > xb {
                return;
            }
            let base_y = (yf - ay) * dy;
            let dt_dx = dx * inv_len2;
            let mut t_raw = ((xa as f32 - ax) * dx + base_y) * inv_len2;
            for x in xa..=xb {
                let t = t_raw.clamp(0.0, 1.0);
                let ex = x as f32 - (ax + t * dx);
                let ey = yf - (ay + t * dy);
                let r = ra + t * dr;
                let r2 = r * r;
                let d2 = ex * ex + ey * ey;
                if d2 < r2 {
                    let dist_in = (r2 - d2) * (0.5 / r.max(0.5));
                    stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(dist_in, feather, w));
                }
                t_raw += dt_dx;
            }
        }
        ShKind::Grating {
            o,
            horiz,
            hl,
            hw,
            rail,
            end_w,
            str_v,
            str_w,
            bar_pitch,
            bar_w,
        } => {
            let half_bar = bar_w * 0.5;
            let half_str = str_w * 0.5;
            let end_hw = hw + rail * 0.45;
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > end_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                // End wall plates
                if hl - au <= end_w {
                    let d = (hl - au).min(end_hw - av).min(end_w - (hl - au) + 4.0);
                    stamp_sh_px(sh_row, idx, soft_u8(d, 3.5, 255));
                    continue;
                }
                if av > hw {
                    continue;
                }
                // Side rails
                if av >= hw - rail {
                    let d = (hw - av).min(av - (hw - rail));
                    stamp_sh_px(sh_row, idx, soft_u8(d, 3.2, 255));
                    continue;
                }
                // Cross-bars (see-through holes between them!)
                let bu = (u + hl - end_w).rem_euclid(bar_pitch);
                let d_bar = half_bar - (bu - bar_pitch * 0.5).abs();
                if d_bar > 0.0 {
                    stamp_sh_px(sh_row, idx, soft_u8(d_bar, 2.5, 220));
                }
                // Two longitudinal stringers
                let d_str = half_str - (av - str_v).abs();
                if d_str > 0.0 {
                    stamp_sh_px(sh_row, idx, soft_u8(d_str, 2.5, 205));
                }
            }
        }
        ShKind::CableTray {
            o,
            horiz,
            hl,
            hw,
            rail,
            cw,
            rung_step,
            rung_w,
            s,
        } => {
            let half_rung = rung_w * 0.5;
            let end_hw = hw + 0.6 * s;
            let cables = [(-hw * 0.42, 0.72 * s), (-hw * 0.04, 0.86 * s), (hw * 0.34, 0.68 * s)];
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > end_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                if hl - au <= cw {
                    let d = (hl - au).min(end_hw - av);
                    stamp_sh_px(sh_row, idx, soft_u8(d, 3.5, 255));
                    continue;
                }
                if av > hw {
                    continue;
                }
                if av >= hw - rail {
                    let d = (hw - av).min(av - (hw - rail));
                    stamp_sh_px(sh_row, idx, soft_u8(d, 3.2, 248));
                    continue;
                }
                for &(cv_off, cr) in &cables {
                    let dc = cr - (v - cv_off).abs();
                    if dc > 0.0 {
                        stamp_sh_px(sh_row, idx, soft_u8(dc, 2.8, 220));
                    }
                }
                let bu = (u + hl - cw).rem_euclid(rung_step);
                let d_r = half_rung - (bu - rung_step * 0.5).abs();
                if d_r > 0.0 {
                    stamp_sh_px(sh_row, idx, soft_u8(d_r, 2.5, 205));
                }
            }
        }
        ShKind::HoodTrap {
            o,
            side,
            root_back,
            reach,
            w_root,
            w_tip,
        } => {
            let (dx, dy, px, py) = edge_basis(side);
            let inv_span = 1.0 / (reach + root_back).max(1.0);
            for x in xa_clip..=xb_clip {
                let rx = x as f32 - o.0;
                let ry = yf - o.1;
                let u = rx * dx + ry * dy;
                if u < -root_back || u > reach {
                    continue;
                }
                let v = rx * px + ry * py;
                let t = (u + root_back) * inv_span;
                let hw = w_root + (w_tip - w_root) * t;
                let av = v.abs();
                if av <= hw {
                    let d = (hw - av).min(reach - u);
                    stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(d, 4.0, 255));
                }
            }
        }
        ShKind::DrapedSheet {
            o,
            horiz,
            u_center,
            v_top,
            w_half,
            h_len,
            sw_bot,
            mode,
        } => {
            let inv_h = 1.0 / h_len.max(1.0);
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                if v < v_top || v > v_top + h_len {
                    continue;
                }
                let t = (v - v_top) * inv_h;
                let du = sw_bot * t * t;
                let lu = u - (u_center + du);
                let hw = match mode {
                    SheetMode::Shirt if t < 0.36 => w_half * (1.0 + 0.85 * (t / 0.36)),
                    SheetMode::Shirt => w_half,
                    _ => w_half * (1.0 + 0.06 * t),
                };
                let au = lu.abs();
                if au > hw {
                    continue;
                }
                if matches!(mode, SheetMode::Shirt) && t < 0.14 && au < w_half * 0.36 {
                    continue;
                }
                let d = (hw - au).min(v - v_top).min((v_top + h_len) - v);
                stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(d, 4.0, 226));
            }
        }
        ShKind::LeafCluster { lobes, w } => {
            for x in xa_clip..=xb_clip {
                let xf = x as f32;
                let mut best_in = 0.0f32;
                for &(lx, ly, inv_rx2, inv_ry2, r_eff) in &lobes {
                    let dx = xf - lx;
                    let dy = yf - ly;
                    let e2 = dx * dx * inv_rx2 + dy * dy * inv_ry2;
                    if e2 < 1.0 {
                        let d_in = (1.0 - e2) * r_eff * 0.5;
                        if d_in > best_in {
                            best_in = d_in;
                        }
                    }
                }
                if best_in > 0.0 {
                    stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(best_in, 4.0, w));
                }
            }
        }
        ShKind::PowerWires {
            o,
            horiz,
            hl,
            spacing,
            sag,
            wire_r,
        } => {
            let inv_hl = 1.0 / hl.max(1.0);
            let r_sh = wire_r + 1.2;
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                if u.abs() > hl {
                    continue;
                }
                let t = u * inv_hl;
                let sag_f = 1.0 - t * t;
                for (wi, &vb) in [-spacing, 0.0, spacing].iter().enumerate() {
                    let wy = vb + sag * (0.9 + 0.1 * wi as f32) * sag_f;
                    let d = r_sh - (v - wy).abs();
                    if d > 0.0 {
                        stamp_sh_px(sh_row, (x - tile_x0) as usize, soft_u8(d, 2.8, 204));
                    }
                }
            }
        }
        ShKind::HoleCutCircle { cx, cy, r2 } => {
            let dy = yf - cy;
            let dy2 = dy * dy;
            if dy2 >= r2 {
                return;
            }
            let span_x = (r2 - dy2).sqrt();
            let xa = ((cx - span_x).ceil() as i32).max(xa_clip);
            let xb = ((cx + span_x).floor() as i32).min(xb_clip);
            for x in xa..=xb {
                sh_row[(x - tile_x0) as usize] = 0;
            }
        }
    }
}

fn eval_bd_row(
    p: &BdPrim,
    y: i32,
    tile_x0: i32,
    tile_x1: i32,
    col_row: &mut [Rgb; STRIP_W],
    alp_row: &mut [u8; STRIP_W],
) {
    let yf = y as f32;
    let xa_clip = p.bb.x0.max(tile_x0);
    let xb_clip = p.bb.x1.min(tile_x1);

    match p.kind {
        BdKind::Empty => {}
        BdKind::Rect {
            x0,
            y0,
            x1,
            y1,
            rgb,
            rim,
            tc,
            w,
            feather,
            rim_w,
        } => {
            let dy_top = yf - y0;
            let dy_bot = y1 - yf;
            let dy = dy_top.min(dy_bot);
            if dy <= 0.0 {
                return;
            }
            let rim_y = (tc.1 > 0 && dy_bot <= rim_w) || (tc.1 < 0 && dy_top <= rim_w);
            for x in xa_clip..=xb_clip {
                let xf = x as f32;
                let dx_l = xf - x0;
                let dx_r = x1 - xf;
                let dx = dx_l.min(dx_r);
                if dx <= 0.0 {
                    continue;
                }
                let wa = soft_u8(dx.min(dy), feather, w);
                let c = if let Some(rc) = rim {
                    let on_rim = (rim_y && dx >= 1.0)
                        || (tc.0 > 0 && dx_r <= rim_w && dy >= 1.0)
                        || (tc.0 < 0 && dx_l <= rim_w && dy >= 1.0);
                    if on_rim { rc } else { rgb }
                } else {
                    rgb
                };
                paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
            }
        }
        BdKind::Ellipse {
            cx,
            cy,
            rx,
            inv_rx2,
            inv_ry2,
            r_eff,
            rgb,
            inner,
            rim,
            tc,
            w,
            feather,
            rim_w,
        } => {
            let dy = yf - cy;
            let ny2 = dy * dy * inv_ry2;
            if ny2 >= 1.0 {
                return;
            }
            let span_x = rx * (1.0 - ny2).sqrt();
            let xa = ((cx - span_x).floor() as i32).max(xa_clip);
            let xb = ((cx + span_x).ceil() as i32).min(xb_clip);
            let tcx = tc.0 as f32;
            let tcy = tc.1 as f32;
            let rim_dot = 0.20 * r_eff;
            for x in xa..=xb {
                let dx = x as f32 - cx;
                let e2 = dx * dx * inv_rx2 + ny2;
                if e2 >= 1.0 {
                    continue;
                }
                let dist_in = (1.0 - e2) * r_eff * 0.5;
                let wa = soft_u8(dist_in, feather, w);
                let mut c = match inner {
                    Some(ic) if e2 < 0.55 => ic,
                    _ => rgb,
                };
                if let Some(rc) = rim {
                    if dist_in <= rim_w + 0.6 && (dx * tcx + dy * tcy) > rim_dot {
                        c = rc;
                    }
                }
                paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
            }
        }
        BdKind::Capsule {
            ax,
            ay,
            ra,
            dx,
            dy,
            dr,
            inv_len2,
            rmax,
            dark,
            mid,
            rim,
            tc,
            w,
            feather,
            rim_w,
        } => {
            let (xa, xb) = capsule_row_x_bounds((ax, ay), (dx, dy), rmax, yf, (xa_clip, xb_clip));
            if xa > xb {
                return;
            }
            let base_y = (yf - ay) * dy;
            let dt_dx = dx * inv_len2;
            let mut t_raw = ((xa as f32 - ax) * dx + base_y) * inv_len2;
            let tcx = tc.0 as f32;
            let tcy = tc.1 as f32;
            for x in xa..=xb {
                let t = t_raw.clamp(0.0, 1.0);
                let ex = x as f32 - (ax + t * dx);
                let ey = yf - (ay + t * dy);
                let r = ra + t * dr;
                let r2 = r * r;
                let d2 = ex * ex + ey * ey;
                if d2 < r2 {
                    let dist_in = (r2 - d2) * (0.5 / r.max(0.5));
                    let wa = soft_u8(dist_in, feather, w);
                    let dot = ex * tcx + ey * tcy;
                    let c = if let Some(rc) = rim {
                        if dist_in <= rim_w + 0.5 && dot > 0.25 * r {
                            rc
                        } else if dot > -0.1 * r && dist_in > 1.5 {
                            mid
                        } else {
                            dark
                        }
                    } else if dot > -0.1 * r && dist_in > 1.5 {
                        mid
                    } else {
                        dark
                    };
                    paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
                }
                t_raw += dt_dx;
            }
        }
        BdKind::DonutRing {
            cx,
            cy,
            vr,
            ir,
            dark,
            soft,
            rim,
            tc,
        } => {
            let dy = yf - cy;
            let vr2 = vr * vr;
            let dy2 = dy * dy;
            if dy2 >= vr2 {
                return;
            }
            let ir2 = ir * ir;
            let inv_2vr = 0.5 / vr.max(0.5);
            let inv_2ir = 0.5 / ir.max(0.5);
            let span_x = (vr2 - dy2).sqrt();
            let xa = ((cx - span_x).floor() as i32).max(xa_clip);
            let xb = ((cx + span_x).ceil() as i32).min(xb_clip);
            let tcx = tc.0 as f32;
            let tcy = tc.1 as f32;
            for x in xa..=xb {
                let dx = x as f32 - cx;
                let d2 = dx * dx + dy2;
                if d2 >= ir2 && d2 <= vr2 {
                    let d_out = (vr2 - d2) * inv_2vr;
                    let d_in = (d2 - ir2) * inv_2ir;
                    let wa = soft_u8(d_out.min(d_in), 1.6, 255);
                    let dot = dx * tcx + dy * tcy;
                    let c = if d_out <= 1.8 && dot > 0.2 * vr {
                        rim
                    } else if dot > 0.25 * vr {
                        soft
                    } else {
                        dark
                    };
                    paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
                }
            }
        }
        BdKind::PipeShaft {
            o,
            horiz,
            hl,
            sr,
            fr,
            cw,
            fw,
            flange_step,
            s,
            seed,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let riv_step = (5.5 * s).max(6.0);
            let riv_r = (0.38 * s).max(1.0);
            let bolt_r = (0.45 * s).max(1.1);
            let rim_band = (0.55 * s).max(2.2);
            let half_fw = fw * 0.5;
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > fr {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                // Check end wall collars & periodic bolted flanges
                let at_end = hl - au <= cw;
                let fu = (u + hl - flange_step * 0.7 + half_fw).rem_euclid(flange_step);
                let du_flange = (fu - half_fw).abs();
                let on_mid_flange = !at_end && (hl - au > cw * 1.3) && fu <= fw;
                if at_end || on_mid_flange {
                    let edge_u = if at_end {
                        (hl - au).min(cw - (hl - au) + 1.5)
                    } else {
                        fu.min(fw - fu)
                    };
                    let d = (fr - av).min(edge_u);
                    let wa = soft_u8(d, 1.8, 255);
                    // Bolt studs on flange face
                    let fc_dist = if at_end {
                        ((hl - au) - cw * 0.5).abs()
                    } else {
                        du_flange
                    };
                    let on_bolt = fc_dist <= bolt_r
                        && ((av - fr * 0.76).abs() <= bolt_r || (av - fr * 0.34).abs() <= bolt_r);
                    let on_rim = (tc_v > 0.0 && fr - v <= 1.8) || (tc_v < 0.0 && fr + v <= 1.8);
                    let c = if on_bolt {
                        if v * tc_v > 0.0 { pal.rim } else { pal.deep }
                    } else if on_rim {
                        pal.rim
                    } else if av < fr * 0.78 && edge_u > 1.0 {
                        pal.mid
                    } else {
                        pal.deep
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                    continue;
                }
                if av > sr {
                    continue;
                }
                let d = (sr - av).min(hl - au);
                let wa = soft_u8(d, 2.0, 255);
                let on_rim = (tc_v > 0.0 && sr - v <= rim_band && sr - v >= 0.8)
                    || (tc_v < 0.0 && sr + v <= rim_band && sr + v >= 0.8);
                // Rivet rows near edges + occasional rust patches
                let r_slot = ((u + hl) / riv_step).floor() as i32;
                let ru = (u + hl) - (r_slot as f32 + 0.5) * riv_step;
                let on_rivet = ru.abs() <= riv_r && (av - sr * 0.72).abs() <= riv_r;
                let hs = hash_slot(r_slot, seed);
                let rust_v = ((hs >> 2) % 5) as f32 * 0.24 * sr - 0.48 * sr;
                let on_rust = (hs & 3) == 0
                    && ru.abs() <= 1.3 * s
                    && (v - rust_v).abs() <= 0.65 * s
                    && av < sr * 0.68;
                let c = if on_rivet {
                    if ru < 0.0 { pal.soft } else { pal.deep }
                } else if on_rim {
                    pal.rim
                } else if on_rust {
                    pal.accent
                } else if v * tc_v > sr * 0.15 && av < sr * 0.72 {
                    pal.soft
                } else if av < sr * 0.68 {
                    pal.mid
                } else {
                    pal.deep
                };
                paint_bd_px(col_row, alp_row, idx, c, wa);
            }
        }
        BdKind::Grating {
            o,
            horiz,
            hl,
            hw,
            rail,
            end_w,
            str_v,
            str_w,
            bar_pitch,
            bar_w,
            s,
            seed,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let half_bar = bar_w * 0.5;
            let half_str = str_w * 0.5;
            let end_hw = hw + 0.5 * s;
            let riv_step = 6.5 * s;
            let riv_r = (0.38 * s).max(1.0);
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > end_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                // Wall end brackets
                if hl - au <= end_w {
                    let d = (hl - au).min(end_hw - av);
                    let wa = soft_u8(d, 1.8, 255);
                    let on_rim = (tc_v > 0.0 && end_hw - v <= 1.8) || (tc_v < 0.0 && end_hw + v <= 1.8);
                    let c = if on_rim { pal.rim } else { pal.deep };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                    continue;
                }
                if av > hw {
                    continue;
                }
                // Outer heavy I-beam side rails + rivets
                if av >= hw - rail {
                    let d = (hw - av).min(av - (hw - rail) + 1.0);
                    let wa = soft_u8(d, 1.8, 255);
                    let r_slot = ((u + hl) / riv_step).floor() as i32;
                    let ru = (u + hl) - (r_slot as f32 + 0.5) * riv_step;
                    let on_riv = ru.abs() <= riv_r && (av - (hw - rail * 0.5)).abs() <= riv_r;
                    let on_rim = (tc_v > 0.0 && (hw - v <= 1.8 || (v < 0.0 && (-v - (hw - rail)) <= 1.5)))
                        || (tc_v < 0.0 && (hw + v <= 1.8 || (v > 0.0 && (v - (hw - rail)) <= 1.5)));
                    let c = if on_riv || on_rim {
                        pal.rim
                    } else {
                        pal.deep
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                    continue;
                }
                // Cross-bars & stringers (holes between them are NOT drawn -> see-through!)
                let rel_u = u + hl - end_w;
                let slot = (rel_u / bar_pitch).floor() as i32;
                let bu = rel_u - (slot as f32 + 0.5) * bar_pitch;
                let d_bar = half_bar - bu.abs();
                let d_str = half_str - (av - str_v).abs();
                if d_str > 0.0 {
                    let wa = soft_u8(d_str, 1.2, 240);
                    let c = if (v - str_v * v.signum()) * tc_v > 0.0 && d_str <= 1.2 {
                        pal.soft
                    } else {
                        pal.deep
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                } else if d_bar > 0.0 {
                    let wa = soft_u8(d_bar, 1.2, 245);
                    let rusty = (hash_slot(slot, seed) & 7) == 0;
                    let c = if rusty {
                        pal.accent
                    } else if bu < 0.0 && d_bar <= 1.2 {
                        pal.soft
                    } else {
                        pal.mid
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                }
            }
        }
        BdKind::ChainTail {
            x_bot,
            y_bot,
            y_top,
            link_w,
            link_h,
            sway_dx,
            pal,
            tc,
        } => {
            if yf > y_bot + link_h * 0.6 || yf < y_top - link_h * 0.6 {
                return;
            }
            let step = (link_h * 0.72).max(3.0);
            let rx_b = link_w * 0.55;
            let ry_b = link_h * 0.55;
            let thick = (link_w * 0.30).max(1.6);
            let irx = (rx_b - thick).max(0.5);
            let iry = (ry_b - thick).max(0.5);
            let rx_n = (link_w * 0.24).max(1.3);
            let ry_n = link_h * 0.56;
            let decay = (link_h * 10.0).max(40.0);
            let tcx = tc.0 as f32;
            let tcy = tc.1 as f32;
            // Only the 2 links straddling row `y` can cover it!
            let idx_center = ((y_bot - yf) / step).round() as i32;
            for link_i in (idx_center - 1).max(0)..=(idx_center + 1) {
                let ly = y_bot - link_i as f32 * step;
                if ly < y_top - link_h {
                    continue;
                }
                let dy = yf - ly;
                let t = (1.0 - (y_bot - ly) / decay).clamp(0.0, 1.0);
                let cx = x_bot + sway_dx * t;
                if (link_i & 1) == 0 {
                    // Broad open-center oval link
                    let ny2 = (dy / ry_b) * (dy / ry_b);
                    if ny2 >= 1.0 {
                        continue;
                    }
                    let span_x = rx_b * (1.0 - ny2).sqrt();
                    let xa = ((cx - span_x).floor() as i32).max(xa_clip);
                    let xb = ((cx + span_x).ceil() as i32).min(xb_clip);
                    let iny2 = (dy / iry) * (dy / iry);
                    let inv_rx2 = 1.0 / (rx_b * rx_b);
                    let inv_irx2 = 1.0 / (irx * irx);
                    let r_out_eff = rx_b.min(ry_b) * 0.5;
                    let r_in_eff = irx.min(iry) * 0.5;
                    for x in xa..=xb {
                        let dx = x as f32 - cx;
                        let e2 = dx * dx * inv_rx2 + ny2;
                        if e2 >= 1.0 {
                            continue;
                        }
                        let ie2 = dx * dx * inv_irx2 + iny2;
                        if ie2 < 0.85 {
                            continue;
                        }
                        let d_out = (1.0 - e2) * r_out_eff;
                        let d_in = (ie2 - 0.85) * r_in_eff;
                        let wa = soft_u8(d_out.min(d_in), 1.4, 255);
                        let dot = dx * tcx + dy * tcy;
                        let c = if d_out <= 1.6 && dot > 0.1 * rx_b {
                            pal.rim
                        } else if dot > 0.0 {
                            pal.soft
                        } else {
                            pal.deep
                        };
                        paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
                    }
                } else {
                    // Edge-on vertical link
                    let dy_edge = ry_n - dy.abs();
                    if dy_edge <= 0.0 {
                        continue;
                    }
                    let xa = ((cx - rx_n).floor() as i32).max(xa_clip);
                    let xb = ((cx + rx_n).ceil() as i32).min(xb_clip);
                    for x in xa..=xb {
                        let dx = x as f32 - cx;
                        let dx_edge = rx_n - dx.abs();
                        if dx_edge <= 0.0 {
                            continue;
                        }
                        let wa = soft_u8(dx_edge.min(dy_edge), 1.2, 255);
                        let c = if dx * tcx > 0.0 && dx_edge <= 1.1 {
                            pal.rim
                        } else {
                            pal.mid
                        };
                        paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
                    }
                }
            }
        }
        BdKind::HoodTrap {
            o,
            side,
            root_back,
            reach,
            w_root,
            w_tip,
            s,
            pal,
            tc,
        } => {
            let (dx, dy, px, py) = edge_basis(side);
            let inv_span = 1.0 / (reach + root_back).max(1.0);
            let lip = (1.35 * s).max(2.5);
            let slat_pitch = (1.5 * s).max(3.0);
            let cross_pitch = (2.8 * s).max(4.0);
            let mullion = (0.55 * s).max(1.8);
            let rim_lip = (0.65 * s).max(2.2);
            let tc_p = px * tc.0 as f32 + py * tc.1 as f32;
            for x in xa_clip..=xb_clip {
                let rx = x as f32 - o.0;
                let ry = yf - o.1;
                let u = rx * dx + ry * dy;
                if u < -root_back || u > reach {
                    continue;
                }
                let v = rx * px + ry * py;
                let t = (u + root_back) * inv_span;
                let hw = w_root + (w_tip - w_root) * t;
                let av = v.abs();
                if av > hw {
                    continue;
                }
                let dist_edge = (hw - av).min(reach - u);
                let mut wa = soft_u8(dist_edge, 2.0, 255);
                if wa == 0 {
                    continue;
                }
                let in_filter = u > 2.2 * s && u < reach - lip * 1.9 && av < hw - lip * 1.7;
                let c = if in_filter {
                    if av < mullion
                        || (u - 2.2 * s).abs() < 1.5
                        || (u - (reach - lip * 1.9)).abs() < 1.5
                    {
                        pal.deep
                    } else {
                        let in_slat = av.rem_euclid(slat_pitch) / slat_pitch;
                        let in_cross = u.rem_euclid(cross_pitch) / cross_pitch;
                        if in_cross < 0.22 {
                            pal.deep
                        } else if in_slat < 0.34 {
                            wa = mul_alpha(wa, 166);
                            pal.deep
                        } else if in_slat < 0.56 {
                            pal.soft
                        } else {
                            pal.mid
                        }
                    }
                } else if reach - u <= lip * 1.15 {
                    if reach - u <= rim_lip { pal.rim } else { pal.soft }
                } else if (av - (hw - lip * 1.7)).abs() < 1.4 {
                    pal.deep
                } else {
                    let dot = rx * tc.0 as f32 + ry * tc.1 as f32;
                    if dist_edge <= 2.2 && dot > 0.0 {
                        pal.rim
                    } else if v * tc_p > 0.0 {
                        pal.soft
                    } else {
                        pal.mid
                    }
                };
                paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
            }
        }
        BdKind::CableTray {
            o,
            horiz,
            hl,
            hw,
            rail,
            cw,
            rung_step,
            rung_w,
            s,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let half_rung = rung_w * 0.5;
            let end_hw = hw + 0.5 * s;
            let cables = [
                (-hw * 0.42, (0.72 * s).max(1.6), pal.deep, pal.soft),
                (-hw * 0.04, (0.86 * s).max(1.8), pal.mid, pal.rim),
                (hw * 0.34, (0.68 * s).max(1.5), pal.accent, pal.rim),
            ];
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > end_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                if hl - au <= cw {
                    let d = (hl - au).min(end_hw - av);
                    let wa = soft_u8(d, 1.8, 255);
                    let on_rim = (tc_v > 0.0 && end_hw - v <= 1.6) || (tc_v < 0.0 && end_hw + v <= 1.6);
                    paint_bd_px(col_row, alp_row, idx, if on_rim { pal.rim } else { pal.deep }, wa);
                    continue;
                }
                if av > hw {
                    continue;
                }
                if av >= hw - rail {
                    let d = (hw - av).min(av - (hw - rail) + 1.0);
                    let wa = soft_u8(d, 1.8, 255);
                    let on_rim = (tc_v > 0.0 && hw - v <= 1.6) || (tc_v < 0.0 && hw + v <= 1.6);
                    paint_bd_px(col_row, alp_row, idx, if on_rim { pal.rim } else { pal.deep }, wa);
                    continue;
                }
                // Check bundled cables first (they sit on top of ladder rungs)
                let mut hit_cable = false;
                for &(cv_off, cr, c_body, c_rim) in &cables {
                    let dv = v - cv_off;
                    let dc = cr - dv.abs();
                    if dc > 0.0 {
                        let wa = soft_u8(dc, 1.4, 250);
                        let c = if dv * tc_v > 0.0 && dc <= 1.1 { c_rim } else { c_body };
                        paint_bd_px(col_row, alp_row, idx, c, wa);
                        hit_cable = true;
                        break;
                    }
                }
                if !hit_cable {
                    let bu = (u + hl - cw).rem_euclid(rung_step);
                    let d_r = half_rung - (bu - rung_step * 0.5).abs();
                    if d_r > 0.0 {
                        let wa = soft_u8(d_r, 1.2, 240);
                        let c = if bu < rung_step * 0.5 && d_r <= 1.0 { pal.soft } else { pal.mid };
                        paint_bd_px(col_row, alp_row, idx, c, wa);
                    }
                }
            }
        }
        BdKind::VentDuct {
            o,
            horiz,
            hl,
            hw,
            seam_hw,
            cw,
            seam_step,
            seam_w,
            has_louvre,
            s,
            seed,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let lv_c = (((hash_u32(seed) & 63) as f32 / 63.0) - 0.5) * (hl * 0.6).max(0.0);
            let lv_hl = (7.2 * s).min((hl * 0.25).max(4.0));
            let lv_hv = hw * 0.62;
            let slat_step = (1.45 * s).max(2.8);
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > seam_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                let at_end = hl - au <= cw;
                let su = (u + hl + seam_w * 0.5).rem_euclid(seam_step);
                let on_seam = has_louvre && !at_end && hl - au > cw * 1.2 && su <= seam_w;
                if at_end || on_seam {
                    let d = (seam_hw - av).min(if at_end { hl - au } else { su.min(seam_w - su) });
                    let wa = soft_u8(d, 1.8, 255);
                    let on_rim = (tc_v > 0.0 && seam_hw - v <= 1.6) || (tc_v < 0.0 && seam_hw + v <= 1.6);
                    let c = if on_rim {
                        pal.rim
                    } else if av < seam_hw * 0.84 && d > 1.0 {
                        pal.soft
                    } else {
                        pal.deep
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                    continue;
                }
                if av > hw {
                    continue;
                }
                let d = (hw - av).min(hl - au);
                let wa = soft_u8(d, 2.0, 255);
                let on_rim = (tc_v > 0.0 && hw - v <= 1.8) || (tc_v < 0.0 && hw + v <= 1.8);
                // Recessed ventilation louvre grille (dropped on short corridor spans)
                let in_lv = has_louvre && (u - lv_c).abs() <= lv_hl && av <= lv_hv;
                let c = if on_rim {
                    pal.rim
                } else if in_lv {
                    let in_frame = (u - lv_c).abs() >= lv_hl - 1.5 || av >= lv_hv - 1.5;
                    if in_frame {
                        pal.deep
                    } else {
                        let sp = (u - (lv_c - lv_hl)).rem_euclid(slat_step) / slat_step;
                        if sp < 0.42 {
                            pal.deep
                        } else if sp < 0.62 {
                            pal.rim
                        } else {
                            pal.soft
                        }
                    }
                } else if av > hw * 0.78 {
                    pal.deep
                } else if v * tc_v > 0.0 {
                    pal.soft
                } else {
                    pal.mid
                };
                paint_bd_px(col_row, alp_row, idx, c, wa);
            }
        }
        BdKind::SteamPipe {
            o,
            horiz,
            hl,
            sr,
            strap_r,
            cw,
            strap_step,
            strap_w,
            leak_u,
            has_leak,
            s,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let wrap_pitch = (3.2 * s).max(4.0);
            let collar_r = strap_r * 1.15;
            let rim_band = (0.55 * s).max(2.2);
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > collar_r {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                let at_end = hl - au <= cw;
                let d_leak = (u - leak_u).abs();
                let on_leak_flange = has_leak && d_leak >= 0.35 * s && d_leak <= 2.1 * s;
                if at_end || on_leak_flange {
                    let d = (collar_r - av).min(hl - au);
                    let wa = soft_u8(d, 1.7, 255);
                    let on_rim = (tc_v > 0.0 && collar_r - v <= 1.5) || (tc_v < 0.0 && collar_r + v <= 1.5);
                    paint_bd_px(col_row, alp_row, idx, if on_rim { pal.rim } else { pal.deep }, wa);
                    continue;
                }
                let su = (u + hl - cw - strap_step * 0.6).rem_euclid(strap_step);
                let on_strap = has_leak && d_leak > 3.2 * s && su <= strap_w && av <= strap_r;
                if on_strap {
                    let d = (strap_r - av).min(su.min(strap_w - su) + 0.8);
                    let wa = soft_u8(d, 1.4, 255);
                    let on_rim = (tc_v > 0.0 && strap_r - v <= 1.4) || (tc_v < 0.0 && strap_r + v <= 1.4);
                    paint_bd_px(col_row, alp_row, idx, if on_rim { pal.rim } else { pal.deep }, wa);
                    continue;
                }
                if av > sr {
                    continue;
                }
                let d = (sr - av).min(hl - au);
                let wa = soft_u8(d, 2.0, 255);
                let wrap = (u + v * 0.55).rem_euclid(wrap_pitch) / wrap_pitch;
                let on_rim = (tc_v > 0.0 && sr - v <= rim_band) || (tc_v < 0.0 && sr + v <= rim_band);
                let c = if on_rim {
                    pal.rim
                } else if av > sr * 0.72 || wrap < 0.16 {
                    pal.deep
                } else if v * tc_v > 0.0 {
                    pal.soft
                } else {
                    pal.mid
                };
                paint_bd_px(col_row, alp_row, idx, c, wa);
            }
        }
        BdKind::PaperWeb {
            o,
            horiz,
            hl,
            hw,
            rol_hw,
            cw,
            rol_step,
            rol_w,
            s,
            seed,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let pg_step = 11.0 * s;
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > rol_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                let at_end = hl - au <= cw;
                let ru = (u + hl - rol_step * 0.8 + rol_w * 0.5).rem_euclid(rol_step);
                let on_roller = !at_end && hl - au > cw * 1.4 && ru <= rol_w;
                if at_end || on_roller {
                    let d = (rol_hw - av).min(if at_end { hl - au } else { ru.min(rol_w - ru) });
                    let wa = soft_u8(d, 1.6, 255);
                    let on_rim = (tc_v > 0.0 && rol_hw - v <= 1.6) || (tc_v < 0.0 && rol_hw + v <= 1.6);
                    let c = if on_rim {
                        pal.rim
                    } else if on_roller && av < hw * 1.02 && (ru - rol_w * 0.5).abs() < rol_w * 0.25 {
                        pal.soft
                    } else {
                        pal.deep
                    };
                    paint_bd_px(col_row, alp_row, idx, c, wa);
                    continue;
                }
                if av > hw {
                    continue;
                }
                let d = (hw - av).min(hl - au);
                let wa = soft_u8(d, 2.0, 224);
                let on_rim = (tc_v > 0.0 && hw - v <= 1.8) || (tc_v < 0.0 && hw + v <= 1.8);
                let rel_u = u + hl - cw;
                let slot = (rel_u / pg_step).floor() as i32;
                let pu = rel_u - (slot as f32 + 0.5) * pg_step;
                let on_col1 = pu.abs() <= 3.2 * s && v >= -hw * 0.65 && v <= -hw * 0.15;
                let on_col2 = pu >= -3.2 * s && pu <= 2.4 * s && v >= hw * 0.08 && v <= hw * 0.58;
                let on_tick = (hash_slot(slot, seed) & 1) == 0
                    && pu.abs() <= 0.7 * s
                    && v >= hw * 0.66
                    && v <= hw * 0.82;
                let c = if on_rim {
                    pal.rim
                } else if on_tick {
                    pal.accent
                } else if on_col1 || on_col2 {
                    pal.deep
                } else if av < hw * 0.84 {
                    pal.soft
                } else {
                    pal.mid
                };
                paint_bd_px(col_row, alp_row, idx, c, wa);
            }
        }
        BdKind::RollerBeam {
            o,
            horiz,
            hl,
            hw,
            flange,
            cw,
            car_u,
            car_hu,
            s,
            pal,
            tc,
        } => {
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let tc_v = if tc_v == 0 { 1 } else { tc_v } as f32;
            let end_hw = hw * 1.35;
            let rib_step = 10.0 * s;
            let hole_r2 = (0.85 * s).max(1.0).powi(2);
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                let au = u.abs();
                let av = v.abs();
                if au > hl || av > end_hw {
                    continue;
                }
                let idx = (x - tile_x0) as usize;
                if hl - au <= cw {
                    let d = (hl - au).min(end_hw - av);
                    let wa = soft_u8(d, 1.8, 255);
                    let on_rim = (tc_v > 0.0 && end_hw - v <= 1.6) || (tc_v < 0.0 && end_hw + v <= 1.6);
                    paint_bd_px(col_row, alp_row, idx, if on_rim { pal.rim } else { pal.deep }, wa);
                    continue;
                }
                if av > hw {
                    continue;
                }
                // Web lightening holes (skip drawing so floor shines through!)
                let rel_u = u + hl - cw;
                let bay_u = rel_u.rem_euclid(rib_step) - rib_step * 0.5;
                if av < hw - flange
                    && (u - car_u).abs() > car_hu + 1.5 * s
                    && bay_u * bay_u + v * v <= hole_r2
                {
                    continue;
                }
                let d = (hw - av).min(hl - au);
                let wa = soft_u8(d, 1.8, 252);
                let on_rim = (tc_v > 0.0 && hw - v <= 1.6) || (tc_v < 0.0 && hw + v <= 1.6);
                let on_rib = rel_u.rem_euclid(rib_step) <= 1.0 * s;
                let c = if on_rim {
                    pal.rim
                } else if av >= hw - flange || on_rib {
                    pal.deep
                } else {
                    pal.mid
                };
                paint_bd_px(col_row, alp_row, idx, c, wa);
            }
        }
        BdKind::DrapedSheet {
            o,
            horiz,
            u_center,
            v_top,
            w_half,
            h_len,
            sw_bot,
            mode,
            seed,
            pal,
            tc,
        } => {
            let inv_h = 1.0 / h_len.max(1.0);
            let tc_u = if horiz { tc.0 } else { tc.1 };
            let tc_v = if horiz { tc.1 } else { tc.0 };
            let base_w: u8 = match mode {
                SheetMode::Shirt => 242,
                SheetMode::Bedsheet | SheetMode::Proof => 225,
            };
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                if v < v_top || v > v_top + h_len {
                    continue;
                }
                let t = (v - v_top) * inv_h;
                let du = sw_bot * t * t;
                let lu = u - (u_center + du);
                let hw = match mode {
                    SheetMode::Shirt if t < 0.36 => w_half * (1.0 + 0.85 * (t / 0.36)),
                    SheetMode::Shirt => w_half,
                    _ => w_half * (1.0 + 0.06 * t),
                };
                let au = lu.abs();
                if au > hw {
                    continue;
                }
                if matches!(mode, SheetMode::Shirt) && t < 0.14 && au < w_half * 0.36 {
                    continue;
                }
                let d = (hw - au).min(v - v_top).min((v_top + h_len) - v);
                let wa = soft_u8(d, 2.0, base_w);
                if wa == 0 {
                    continue;
                }
                let mut c = match mode {
                    SheetMode::Bedsheet => {
                        let fold = tri_wave((lu / w_half.max(1.0)) * 1.55 + t * 0.22);
                        if t > 0.88 || fold > 0.35 {
                            pal.soft
                        } else if fold < -0.40 {
                            pal.deep
                        } else {
                            pal.mid
                        }
                    }
                    SheetMode::Shirt => {
                        if t < 0.36 && au > w_half * 0.95 {
                            pal.mid
                        } else if au < w_half * 0.25 {
                            pal.soft
                        } else if au < w_half * 0.82 {
                            pal.mid
                        } else {
                            pal.deep
                        }
                    }
                    SheetMode::Proof => {
                        let mut pc = pal.soft;
                        if au < w_half * 0.78 {
                            if (0.08..=0.22).contains(&t) {
                                pc = pal.deep;
                            } else if (0.28..=0.88).contains(&t) && au > w_half * 0.08 {
                                let line_phase = ((t - 0.28) * 14.0).fract();
                                if line_phase < 0.58 {
                                    if lu > 0.0 && (0.44..=0.68).contains(&t) && (seed & 1) == 0 {
                                        pc = pal.accent;
                                    } else {
                                        pc = pal.mid;
                                    }
                                }
                            }
                        }
                        pc
                    }
                };
                let on_rim = (tc_v > 0 && (v_top + h_len) - v <= 1.8)
                    || (tc_v < 0 && v - v_top <= 1.8)
                    || (tc_u > 0 && (hw - lu) <= 1.8)
                    || (tc_u < 0 && (hw + lu) <= 1.8);
                if on_rim {
                    c = pal.rim;
                }
                paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
            }
        }
        BdKind::LeafCluster {
            cx,
            cy,
            min_r,
            lobes,
            seed,
            pal,
            tc,
        } => {
            let tcx = tc.0 as f32;
            let tcy = tc.1 as f32;
            let rim_dot = 0.25 * min_r;
            let soft_dot = 0.10 * min_r;
            let hy = ((y >> 1) as u32).wrapping_mul(19_349_663) ^ seed;
            for x in xa_clip..=xb_clip {
                let xf = x as f32;
                let mut best_in = 0.0f32;
                for &(lx, ly, inv_rx2, inv_ry2, r_eff) in &lobes {
                    let dx = xf - lx;
                    let dy = yf - ly;
                    let e2 = dx * dx * inv_rx2 + dy * dy * inv_ry2;
                    if e2 < 1.0 {
                        let d_in = (1.0 - e2) * r_eff * 0.5;
                        if d_in > best_in {
                            best_in = d_in;
                        }
                    }
                }
                if best_in <= 0.0 {
                    continue;
                }
                let wa = soft_u8(best_in, 2.0, 245);
                if wa == 0 {
                    continue;
                }
                let th = hash_u32(((x >> 1) as u32).wrapping_mul(73_856_093) ^ hy);
                let dot = (xf - cx) * tcx + (yf - cy) * tcy;
                let c = if best_in <= 2.4 && dot > rim_dot {
                    if (th & 3) != 0 { pal.accent } else { pal.rim }
                } else if dot > soft_dot && (th & 3) == 0 {
                    pal.soft
                } else if (th & 1) == 0 {
                    pal.mid
                } else {
                    pal.deep
                };
                paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
            }
        }
        BdKind::PowerWires {
            o,
            horiz,
            hl,
            spacing,
            sag,
            wire_r,
            rim,
            tc,
        } => {
            let inv_hl = 1.0 / hl.max(1.0);
            let tc_v = if horiz { tc.1 } else { tc.0 } as f32;
            for x in xa_clip..=xb_clip {
                let (u, v) = if horiz {
                    (x as f32 - o.0, yf - o.1)
                } else {
                    (yf - o.1, x as f32 - o.0)
                };
                if u.abs() > hl {
                    continue;
                }
                let t = u * inv_hl;
                let sag_f = 1.0 - t * t;
                for (wi, &vb) in [-spacing, 0.0, spacing].iter().enumerate() {
                    let wy = vb + sag * (0.9 + 0.1 * wi as f32) * sag_f;
                    let dv = v - wy;
                    let d = wire_r - dv.abs();
                    if d > 0.0 {
                        // Light grey-blue: the v2 near-black wires never
                        // showed on the dark bed.
                        let wa = soft_u8(d, 0.25, 255);
                        let c = if dv * tc_v > 0.0 && d <= 0.3 {
                            rim
                        } else {
                            WIRE_RGB
                        };
                        paint_bd_px(col_row, alp_row, (x - tile_x0) as usize, c, wa);
                    }
                }
            }
        }
        BdKind::HoleCutCircle { cx, cy, r2 } => {
            let dy = yf - cy;
            let dy2 = dy * dy;
            if dy2 >= r2 {
                return;
            }
            let span_x = (r2 - dy2).sqrt();
            let xa = ((cx - span_x).ceil() as i32).max(xa_clip);
            let xb = ((cx + span_x).floor() as i32).min(xb_clip);
            for x in xa..=xb {
                alp_row[(x - tile_x0) as usize] = 0;
            }
        }
    }
}

// ---- Per-kind emitters (18 kinds across 6 floor themes) --------------------

// 1. Sewer: MainPipe (Span)
fn emit_main_pipe(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let sr = 4.2 * s;
    let fr = 5.6 * s;
    let cw = (3.2 * s).min(hl * 0.36);
    let fw = 2.0 * s;
    let flange_step = 24.0 * s;
    let has_valve = hl >= 9.5 * s;

    let h = hash_u32(c.seed ^ 0x51A7);
    let valve_u = ((h & 255) as f32 / 255.0 - 0.5) * (hl * 0.85).max(0.0);
    let valve_side = if ((h >> 8) & 1) == 0 { 1.0 } else { -1.0 };
    let vr = 4.6 * s;
    let valve_vc = valve_side * (sr + vr * 0.55);

    // Shadow: shaft, wall collars, mid flanges, spoked wheel disc (dropped on short corridor spans)
    list.sh_rect_uv(c.sh, horiz, -hl, -sr, hl, sr, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, -hl, -fr, -hl + cw, fr, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, -fr, hl, fr, 255, 4.0);
    let mut fu = -hl + flange_step * 0.7;
    while fu < hl - cw * 1.4 {
        list.sh_rect_uv(c.sh, horiz, fu - fw * 0.5, -fr, fu + fw * 0.5, fr, 255, 4.0);
        fu += flange_step;
    }
    if has_valve {
        let (svx, svy) = uv_to_xy(c.sh, horiz, valve_u, valve_vc);
        list.sh_ellipse(svx, svy, vr, vr, 240, 4.0);
    }

    // Body: pipe shaft + collars + flanges + rivets + rust
    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - fr, c.sc.0 + hl, c.sc.1 + fr)
    } else {
        (c.sc.0 - fr, c.sc.1 - hl, c.sc.0 + fr, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::PipeShaft {
            o: c.sc,
            horiz,
            hl,
            sr,
            fr,
            cw,
            fw,
            flange_step,
            s,
            seed: c.seed,
            pal: c.pal,
            tc: c.tc,
        },
    );

    if !has_valve {
        return;
    }
    // Valve stem + spoked handwheel
    let stem_v0 = (valve_side * sr * 0.6).min(valve_vc);
    let stem_v1 = (valve_side * sr * 0.6).max(valve_vc);
    list.bd_rect_uv(
        c.sc,
        horiz,
        valve_u - 0.9 * s,
        stem_v0,
        valve_u + 0.9 * s,
        stem_v1,
        c.pal.deep,
        Some(c.pal.soft),
        c.tc,
        255,
        1.2,
        1.2,
    );
    let (vx, vy) = uv_to_xy(c.sc, horiz, valve_u, valve_vc);
    let sp_r = (0.42 * s).max(1.2);
    list.bd_capsule(
        (vx - vr + 1.0, vy),
        sp_r,
        (vx + vr - 1.0, vy),
        sp_r,
        c.pal.deep,
        c.pal.mid,
        Some(c.pal.rim),
        c.tc,
        255,
        1.0,
        1.2,
    );
    list.bd_capsule(
        (vx, vy - vr + 1.0),
        sp_r,
        (vx, vy + vr - 1.0),
        sp_r,
        c.pal.deep,
        c.pal.mid,
        Some(c.pal.rim),
        c.tc,
        255,
        1.0,
        1.2,
    );
    let ir = (vr - (0.95 * s).max(2.2)).max(1.0);
    list.push_bd(
        vx - vr,
        vy - vr,
        vx + vr,
        vy + vr,
        BdKind::DonutRing {
            cx: vx,
            cy: vy,
            vr,
            ir,
            dark: c.pal.deep,
            soft: c.pal.soft,
            rim: c.pal.rim,
            tc: c.tc,
        },
    );
    list.bd_ellipse(
        vx,
        vy,
        1.35 * s,
        1.35 * s,
        c.pal.accent,
        None,
        Some(c.pal.rim),
        c.tc,
        255,
        1.0,
        1.0,
    );
}

// 2. Sewer: CatwalkGrating (Span, see-through)
fn emit_catwalk_grating(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let hw = 6.0 * s;
    let rail = 1.35 * s;
    let end_w = (2.6 * s).min(hl * 0.34);
    let bar_pitch = (2.2 * s).max(4.0);
    let bar_w = (0.65 * s).max(1.6);
    let str_v = (hw - rail) * 0.36;
    let str_w = (0.45 * s).max(1.4);
    let end_hw = hw + 0.6 * s;

    let (sx0, sy0, sx1, sy1) = if horiz {
        (c.sh.0 - hl, c.sh.1 - end_hw, c.sh.0 + hl, c.sh.1 + end_hw)
    } else {
        (c.sh.0 - end_hw, c.sh.1 - hl, c.sh.0 + end_hw, c.sh.1 + hl)
    };
    list.push_sh(
        sx0,
        sy0,
        sx1,
        sy1,
        ShKind::Grating {
            o: c.sh,
            horiz,
            hl,
            hw,
            rail,
            end_w,
            str_v,
            str_w,
            bar_pitch,
            bar_w,
        },
    );

    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - end_hw, c.sc.0 + hl, c.sc.1 + end_hw)
    } else {
        (c.sc.0 - end_hw, c.sc.1 - hl, c.sc.0 + end_hw, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::Grating {
            o: c.sc,
            horiz,
            hl,
            hw,
            rail,
            end_w,
            str_v,
            str_w,
            bar_pitch,
            bar_w,
            s,
            seed: c.seed,
            pal: c.pal,
            tc: c.tc,
        },
    );
}

// 3. Sewer: ChainHoist (Drop)
fn emit_chain_hoist(list: &mut PrimList, c: &DrawCtx) {
    let s = c.s;
    let sw = c.sway * 1.8 * s;
    let bw = 3.6 * s;
    let bh = 4.4 * s;
    let link_w = (2.2 * s).max(2.8);
    let link_h = (3.0 * s).max(3.6);

    // Shadow: short chain stub + pulley block + J-hook
    let (sx, sy) = (c.sh.0 + sw * 0.5, c.sh.1);
    list.sh_capsule(
        (sx, sy - bh - 12.0 * s),
        link_w * 0.42,
        (sx, sy - bh),
        link_w * 0.42,
        195,
        3.5,
    );
    list.sh_ellipse(sx, sy - bh * 0.45, bw, bh * 0.68, 255, 4.0);
    list.sh_rect(sx - bw, sy - bh * 0.45, sx + bw, sy + bh * 0.58, 255, 4.0);
    let hr = 2.6 * s;
    list.sh_capsule((sx, sy + bh * 0.5), 0.9 * s, (sx, sy + bh + 1.5 * s), 0.9 * s, 235, 3.5);
    list.sh_ellipse(sx + 0.3 * s, sy + bh + hr * 1.1, hr, hr, 235, 3.5);

    // Body: heavy chain straight UP for DROP_TAIL
    let (ox, oy) = (c.sc.0 + sw, c.sc.1);
    let y_top = oy - bh - DROP_TAIL as f32;
    let ch_x0 = c.sc.0.min(ox) - link_w;
    let ch_x1 = c.sc.0.max(ox) + link_w;
    list.push_bd(
        ch_x0,
        y_top - link_h,
        ch_x1,
        oy - bh + link_h,
        BdKind::ChainTail {
            x_bot: c.sc.0,
            y_bot: oy - bh,
            y_top,
            link_w,
            link_h,
            sway_dx: sw,
            pal: c.pal,
            tc: c.tc,
        },
    );

    // Top shackle + cheek-plate housing + axle pin + bolts
    list.bd_rect(
        ox - bw * 0.48,
        oy - bh - 1.2 * s,
        ox + bw * 0.48,
        oy - bh + 0.8 * s,
        c.pal.mid,
        Some(c.pal.rim),
        c.tc,
        255,
        1.5,
        1.2,
    );
    list.bd_ellipse(
        ox,
        oy - bh * 0.35,
        bw,
        bh * 0.72,
        c.pal.deep,
        Some(c.pal.mid),
        Some(c.pal.rim),
        c.tc,
        255,
        2.0,
        1.8,
    );
    list.bd_rect(
        ox - bw,
        oy - bh * 0.35,
        ox + bw,
        oy + bh * 0.55,
        c.pal.deep,
        Some(c.pal.rim),
        c.tc,
        255,
        2.0,
        1.8,
    );
    list.bd_rect(
        ox - bw * 0.68,
        oy - bh * 0.55,
        ox + bw * 0.68,
        oy + bh * 0.42,
        c.pal.mid,
        None,
        c.tc,
        255,
        1.2,
        0.0,
    );
    list.bd_ellipse(
        ox,
        oy - 0.2 * s,
        1.45 * s,
        1.45 * s,
        c.pal.deep,
        Some(c.pal.accent),
        Some(c.pal.rim),
        c.tc,
        255,
        1.0,
        1.1,
    );
    let bolt_r = (0.38 * s).max(1.0);
    for &(dx, dy) in &[(-0.65, -0.55), (0.65, -0.55), (-0.65, 0.35), (0.65, 0.35)] {
        list.bd_ellipse(
            ox + dx * bw,
            oy + dy * bh,
            bolt_r,
            bolt_r,
            c.pal.deep,
            None,
            Some(c.pal.soft),
            c.tc,
            255,
            0.8,
            0.8,
        );
    }

    // Forged J-hook + safety latch
    let hy0 = oy + bh * 0.50;
    let hr_t = (0.78 * s).max(1.5);
    let pts = [
        (ox, hy0, hr_t),
        (ox, hy0 + 1.4 * s, hr_t * 0.95),
        (ox + 1.8 * s, hy0 + 2.6 * s, hr_t * 1.05),
        (ox + 1.4 * s, hy0 + 4.4 * s, hr_t * 1.10),
        (ox - 0.6 * s, hy0 + 4.8 * s, hr_t * 1.05),
        (ox - 2.1 * s, hy0 + 3.6 * s, hr_t * 0.80),
        (ox - 1.9 * s, hy0 + 2.3 * s, hr_t * 0.55),
    ];
    for pair in pts.windows(2) {
        let a = pair[0];
        let b = pair[1];
        list.bd_capsule(
            (a.0, a.1),
            a.2,
            (b.0, b.1),
            b.2,
            c.pal.deep,
            c.pal.soft,
            Some(c.pal.rim),
            c.tc,
            255,
            1.5,
            1.4,
        );
    }
    let latch_r = (0.28 * s).max(0.8);
    list.bd_capsule(
        (ox - 0.2 * s, hy0 + 1.4 * s),
        latch_r,
        (ox - 1.8 * s, hy0 + 2.4 * s),
        latch_r,
        c.pal.accent,
        c.pal.rim,
        Some(c.pal.rim),
        c.tc,
        235,
        1.0,
        1.0,
    );
}

// 4. Kitchen: ExtractorHood (Edge)
fn emit_extractor_hood(list: &mut PrimList, c: &DrawCtx, side: Side, reach: f32) {
    let s = c.s;
    let root_back = 10.0 * s;
    let w_root = 13.5 * s;
    let w_tip = 9.6 * s;
    let (dx, dy, px, py) = edge_basis(side);

    let bounds = |o: (f32, f32)| -> (f32, f32, f32, f32) {
        let c0 = (o.0 - dx * root_back - px * w_root, o.1 - dy * root_back - py * w_root);
        let c1 = (o.0 - dx * root_back + px * w_root, o.1 - dy * root_back + py * w_root);
        let c2 = (o.0 + dx * reach - px * w_tip, o.1 + dy * reach - py * w_tip);
        let c3 = (o.0 + dx * reach + px * w_tip, o.1 + dy * reach + py * w_tip);
        let x0 = c0.0.min(c1.0).min(c2.0).min(c3.0);
        let y0 = c0.1.min(c1.1).min(c2.1).min(c3.1);
        let x1 = c0.0.max(c1.0).max(c2.0).max(c3.0);
        let y1 = c0.1.max(c1.1).max(c2.1).max(c3.1);
        (x0, y0, x1, y1)
    };

    let (sx0, sy0, sx1, sy1) = bounds(c.sh);
    list.push_sh(
        sx0,
        sy0,
        sx1,
        sy1,
        ShKind::HoodTrap {
            o: c.sh,
            side,
            root_back,
            reach,
            w_root,
            w_tip,
        },
    );

    let (bx0, by0, bx1, by1) = bounds(c.sc);
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::HoodTrap {
            o: c.sc,
            side,
            root_back,
            reach,
            w_root,
            w_tip,
            s,
            pal: c.pal,
            tc: c.tc,
        },
    );
}

// 5. Kitchen: PotRail (Span, sways)
fn emit_pot_rail(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let rail_v = -4.2 * s;
    let rail_h = 1.15 * s;
    let cw = (2.6 * s).min(hl * 0.34);
    let spacing = 10.6 * s;

    // Rail + wall brackets (shadow & body)
    list.sh_rect_uv(c.sh, horiz, -hl, rail_v - rail_h, hl, rail_v + rail_h, 232, 3.5);
    list.sh_rect_uv(c.sh, horiz, -hl, rail_v - rail_h * 2.1, -hl + cw, rail_v + rail_h * 2.1, 248, 3.5);
    list.sh_rect_uv(c.sh, horiz, hl - cw, rail_v - rail_h * 2.1, hl, rail_v + rail_h * 2.1, 248, 3.5);

    list.bd_rect_uv(c.sc, horiz, -hl, rail_v - rail_h, hl, rail_v + rail_h, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.8, 1.5);
    list.bd_rect_uv(c.sc, horiz, -hl, rail_v - rail_h * 2.0, -hl + cw, rail_v + rail_h * 2.0, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.8, 1.5);
    list.bd_rect_uv(c.sc, horiz, hl - cw, rail_v - rail_h * 2.0, hl, rail_v + rail_h * 2.0, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.8, 1.5);

    // Short corridor spans: hang one compact centered skillet when there is room, else just the rail.
    let hook_r = (0.38 * s).max(1.0);
    if hl < cw + spacing * 0.65 {
        let clear = (hl - cw - 1.5).max(0.0);
        if clear >= 5.0 {
            let pr = (4.0 * s).min(clear * 0.9);
            let sw_u = c.sway * (1.2 * s).min(clear * 0.25);
            let sh0 = uv_to_xy(c.sh, horiz, 0.0, rail_v + rail_h * 0.6);
            let sh1 = uv_to_xy(c.sh, horiz, sw_u * 0.3, rail_v + 2.6 * s);
            let bh0 = uv_to_xy(c.sc, horiz, 0.0, rail_v + rail_h * 0.6);
            let bh1 = uv_to_xy(c.sc, horiz, sw_u * 0.3, rail_v + 2.6 * s);
            let sp = uv_to_xy(c.sh, horiz, sw_u, rail_v + 5.0 * s + pr * 0.6);
            let bp = uv_to_xy(c.sc, horiz, sw_u, rail_v + 5.0 * s + pr * 0.6);
            list.sh_capsule(sh0, hook_r, sh1, hook_r, 182, 2.5);
            list.sh_ellipse(sp.0, sp.1, pr, pr, 255, 4.0);
            list.bd_capsule(bh0, hook_r, bh1, hook_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.0, 1.0);
            list.bd_capsule(bh1, 0.5 * s, bp, 0.65 * s, c.pal.deep, c.pal.mid, Some(c.pal.rim), c.tc, 255, 1.3, 1.2);
            list.bd_ellipse(bp.0, bp.1, pr, pr, c.pal.deep, Some(c.pal.mid), Some(c.pal.rim), c.tc, 255, 2.0, 2.0);
            list.bd_ellipse(bp.0, bp.1, pr * 0.72, pr * 0.72, c.pal.mid, Some(c.pal.deep), None, c.tc, 255, 1.2, 0.0);
        }
        return;
    }

    // Clip hanging cookware slots to the visible buffer axis so huge spans stay fast
    // The clip window along the axis (the scratch window while upscaling).
    let (axis_o, axis_min, axis_max) = if horiz {
        (c.sc.0, list.bx0 as f32, (list.bx0 + list.bw) as f32)
    } else {
        (c.sc.1, list.by0 as f32, (list.by0 + list.bh) as f32)
    };
    let u_vis0 = (axis_min - axis_o - 20.0 * s).max(-hl + cw + spacing * 0.52);
    let u_vis1 = (axis_max - axis_o + 20.0 * s).min(hl - cw - spacing * 0.35);
    let u_origin = -hl + cw + spacing * 0.52;
    let first_slot = ((u_vis0 - u_origin) / spacing).floor().max(0.0) as i32;
    let last_slot = ((u_vis1 - u_origin) / spacing).ceil() as i32;

    for slot in first_slot..=last_slot {
        let u = u_origin + slot as f32 * spacing;
        if u < -hl + cw || u > hl - cw - spacing * 0.35 {
            continue;
        }
        let h = hash_slot(slot, c.seed);
        let sw_u = c.sway * 1.6 * s * (0.85 + 0.3 * ((h >> 4) & 1) as f32);
        let kind = (slot.rem_euclid(3) as u32 + (c.seed % 3)) % 3;

        // S-hook
        let sh0 = uv_to_xy(c.sh, horiz, u, rail_v + rail_h * 0.6);
        let sh1 = uv_to_xy(c.sh, horiz, u + sw_u * 0.3, rail_v + 2.8 * s);
        let bh0 = uv_to_xy(c.sc, horiz, u, rail_v + rail_h * 0.6);
        let bh1 = uv_to_xy(c.sc, horiz, u + sw_u * 0.3, rail_v + 2.8 * s);
        list.sh_capsule(sh0, hook_r, sh1, hook_r, 182, 2.5);
        list.bd_capsule(bh0, hook_r, bh1, hook_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.0, 1.0);

        match kind {
            0 => {
                // Big cast-iron skillet
                let pu = u + sw_u;
                let pv = rail_v + 7.6 * s;
                let pr = 4.0 * s;
                let sp = uv_to_xy(c.sh, horiz, pu, pv);
                let bp = uv_to_xy(c.sc, horiz, pu, pv);
                list.sh_capsule(sh1, 0.65 * s, sp, 0.75 * s, 218, 3.0);
                list.sh_ellipse(sp.0, sp.1, pr, pr, 255, 4.0);

                list.bd_capsule(bh1, 0.58 * s, bp, 0.78 * s, c.pal.deep, c.pal.mid, Some(c.pal.rim), c.tc, 255, 1.3, 1.2);
                list.bd_ellipse(bp.0, bp.1, pr, pr, c.pal.deep, Some(c.pal.mid), Some(c.pal.rim), c.tc, 255, 2.0, 2.0);
                list.bd_ellipse(bp.0, bp.1, pr * 0.72, pr * 0.72, c.pal.mid, Some(c.pal.deep), None, c.tc, 255, 1.2, 0.0);
            }
            1 => {
                // Copper stockpot with side ear handles
                let pu = u + sw_u;
                let pw = 3.7 * s;
                let pv0 = rail_v + 3.0 * s;
                let pv1 = rail_v + 9.8 * s;
                list.sh_rect_uv(c.sh, horiz, pu - pw, pv0, pu + pw, pv1, 255, 4.0);
                list.bd_rect_uv(c.sc, horiz, pu - pw - 1.15 * s, pv0 + 1.2 * s, pu + pw + 1.15 * s, pv0 + 2.6 * s, c.pal.deep, Some(c.pal.soft), c.tc, 255, 1.2, 1.0);
                list.bd_rect_uv(c.sc, horiz, pu - pw, pv0, pu + pw, pv1, c.pal.deep, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
                list.bd_rect_uv(c.sc, horiz, pu - pw * 0.76, pv0 + 0.9 * s, pu + pw * 0.76, pv1 - 0.9 * s, c.pal.accent, None, c.tc, 235, 1.5, 0.0);
                list.bd_rect_uv(c.sc, horiz, pu - pw * 1.06, pv0 - 0.4 * s, pu + pw * 1.06, pv0 + 0.8 * s, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.2, 1.2);
            }
            _ => {
                // Big slotted skimmer / ladle
                let pu = u + sw_u;
                let pv = rail_v + 8.6 * s;
                let pr = 3.0 * s;
                let sp = uv_to_xy(c.sh, horiz, pu, pv);
                let bp = uv_to_xy(c.sc, horiz, pu, pv);
                list.sh_capsule(sh1, 0.45 * s, sp, 0.55 * s, 204, 3.0);
                list.sh_ellipse(sp.0, sp.1, pr, pr * 0.88, 240, 3.5);

                list.bd_capsule(bh1, 0.42 * s, bp, 0.52 * s, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.2, 1.0);
                list.bd_ellipse(bp.0, bp.1, pr, pr * 0.88, c.pal.mid, Some(c.pal.accent), Some(c.pal.rim), c.tc, 255, 1.8, 1.8);
                let hr = (0.38 * s).max(0.8);
                let hr2 = hr * hr;
                for &(hx_o, hy_o) in &[(-0.42, -0.25), (0.42, -0.25), (0.0, 0.25)] {
                    let sxh = sp.0 + hx_o * pr;
                    let syh = sp.1 + hy_o * pr;
                    list.push_sh(sxh - hr, syh - hr, sxh + hr, syh + hr, ShKind::HoleCutCircle { cx: sxh, cy: syh, r2: hr2 });
                    let cx_h = bp.0 + hx_o * pr;
                    let cy_h = bp.1 + hy_o * pr;
                    list.push_bd(cx_h - hr, cy_h - hr, cx_h + hr, cy_h + hr, BdKind::HoleCutCircle { cx: cx_h, cy: cy_h, r2: hr2 });
                }
            }
        }
    }
}

// 6. Kitchen: HangingLamp (Drop, sways)
fn emit_hanging_lamp(list: &mut PrimList, c: &DrawCtx) {
    let s = c.s;
    let sw = c.sway * 2.4 * s;
    let rx = 7.6 * s;
    let ry = 4.6 * s;
    let cord_r = (0.42 * s).max(1.2);

    // Shadow
    let (sx, sy) = (c.sh.0 + sw * 0.5, c.sh.1);
    list.sh_capsule((c.sh.0, sy - ry - 12.0 * s), cord_r, (sx, sy - ry * 0.7), cord_r, 160, 3.0);
    list.sh_ellipse(sx, sy, rx, ry, 255, 4.0);

    // Body: cord rising straight UP for DROP_TAIL
    let (ox, oy) = (c.sc.0 + sw, c.sc.1);
    let top_y = oy - ry - DROP_TAIL as f32;
    let mid_y = oy - ry - 14.0 * s;
    list.bd_capsule((c.sc.0, top_y), cord_r, (c.sc.0, mid_y), cord_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.2, 1.0);
    list.bd_capsule((c.sc.0, mid_y), cord_r, (ox, oy - ry * 0.85), cord_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.2, 1.0);

    // Wide industrial enamel dome shade + inner warm glow lip + brass socket
    list.bd_ellipse(ox, oy + 0.5 * s, rx, ry * 0.82, c.pal.accent, Some(c.pal.pale), Some(c.pal.rim), c.tc, 235, 2.0, 2.2);
    list.bd_ellipse(ox, oy - 0.15 * s, rx, ry * 0.88, c.pal.deep, Some(c.pal.mid), Some(c.pal.rim), c.tc, 255, 2.0, 2.0);
    list.bd_ellipse(ox, oy - ry * 0.42, rx * 0.46, ry * 0.44, c.pal.mid, Some(c.pal.soft), Some(c.pal.rim), c.tc, 255, 1.6, 1.5);
    list.bd_rect(ox - 1.35 * s, oy - ry - 1.0 * s, ox + 1.35 * s, oy - ry * 0.55, c.pal.accent, Some(c.pal.rim), c.tc, 255, 1.4, 1.2);
}

// 7. Lab: CableTray (Span, see-through)
fn emit_cable_tray(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let hw = 5.0 * s;
    let rail = 1.15 * s;
    let cw = (2.6 * s).min(hl * 0.34);
    let rung_step = (4.8 * s).max(5.0);
    let rung_w = (0.65 * s).max(1.4);
    let end_hw = hw + 0.6 * s;

    let (sx0, sy0, sx1, sy1) = if horiz {
        (c.sh.0 - hl, c.sh.1 - end_hw, c.sh.0 + hl, c.sh.1 + end_hw)
    } else {
        (c.sh.0 - end_hw, c.sh.1 - hl, c.sh.0 + end_hw, c.sh.1 + hl)
    };
    list.push_sh(sx0, sy0, sx1, sy1, ShKind::CableTray { o: c.sh, horiz, hl, hw, rail, cw, rung_step, rung_w, s });

    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - end_hw, c.sc.0 + hl, c.sc.1 + end_hw)
    } else {
        (c.sc.0 - end_hw, c.sc.1 - hl, c.sc.0 + end_hw, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::CableTray {
            o: c.sc,
            horiz,
            hl,
            hw,
            rail,
            cw,
            rung_step,
            rung_w,
            s,
            pal: c.pal,
            tc: c.tc,
        },
    );

    if hl < 9.0 * s {
        return;
    }
    // Drooping catenary cable loop sagging out past one side rail
    let h = hash_u32(c.seed ^ 0xCAB1);
    let droop_u0 = -hl * 0.45 + ((h & 63) as f32 / 63.0) * hl * 0.35;
    let droop_u1 = (droop_u0 + 18.0 * s).min(hl - cw * 1.2);
    if droop_u1 > droop_u0 + 2.0 {
        let droop_side = if ((h >> 7) & 1) == 0 { 1.0 } else { -1.0 };
        let droop_amp = 3.2 * s;
        let cr = (0.52 * s).max(1.4);
        const STEPS: usize = 8;
        for i in 0..STEPS {
            let t0 = i as f32 / STEPS as f32;
            let t1 = (i + 1) as f32 / STEPS as f32;
            let u0 = droop_u0 + (droop_u1 - droop_u0) * t0;
            let u1 = droop_u0 + (droop_u1 - droop_u0) * t1;
            let v0 = droop_side * (hw * 0.45 + droop_amp * 4.0 * t0 * (1.0 - t0));
            let v1 = droop_side * (hw * 0.45 + droop_amp * 4.0 * t1 * (1.0 - t1));
            list.sh_capsule(uv_to_xy(c.sh, horiz, u0, v0), cr, uv_to_xy(c.sh, horiz, u1, v1), cr, 204, 3.0);
            list.bd_capsule(
                uv_to_xy(c.sc, horiz, u0, v0),
                cr,
                uv_to_xy(c.sc, horiz, u1, v1),
                cr,
                c.pal.deep,
                c.pal.accent,
                Some(c.pal.rim),
                c.tc,
                250,
                1.3,
                1.2,
            );
        }
    }
}

// 8. Lab: VentDuct (Span)
fn emit_vent_duct(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let hw = 4.8 * s;
    let seam_hw = 5.4 * s;
    let cw = (3.0 * s).min(hl * 0.35);
    let seam_step = 20.0 * s;
    let seam_w = (1.25 * s).max(1.8);
    let has_louvre = hl >= 8.5 * s;

    list.sh_rect_uv(c.sh, horiz, -hl, -hw, hl, hw, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, -hl, -seam_hw, -hl + cw, seam_hw, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, -seam_hw, hl, seam_hw, 255, 4.0);
    if has_louvre {
        let mut su = -hl + seam_step;
        while su < hl - cw * 1.4 {
            list.sh_rect_uv(c.sh, horiz, su - seam_w * 0.5, -seam_hw, su + seam_w * 0.5, seam_hw, 255, 4.0);
            su += seam_step;
        }
    }

    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - seam_hw, c.sc.0 + hl, c.sc.1 + seam_hw)
    } else {
        (c.sc.0 - seam_hw, c.sc.1 - hl, c.sc.0 + seam_hw, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::VentDuct {
            o: c.sc,
            horiz,
            hl,
            hw,
            seam_hw,
            cw,
            seam_step,
            seam_w,
            has_louvre,
            s,
            seed: c.seed,
            pal: c.pal,
            tc: c.tc,
        },
    );
}

// 9. Lab: ArticulatedArm (Edge)
fn emit_articulated_arm(list: &mut PrimList, c: &DrawCtx, side: Side, reach: f32) {
    let s = c.s;
    let (dx, dy, px, py) = edge_basis(side);
    let bend = if (c.seed & 1) == 0 { 1.0 } else { -1.0 };
    let root_back = 9.0 * s;
    let elbow_u = reach * 0.48;
    let elbow_v = bend * reach * 0.24;
    let wrist_u = reach * 0.86;
    let wrist_v = bend * reach * 0.08;
    let mid_u = wrist_u + (reach - wrist_u) * 0.55;

    let pt = |o: (f32, f32), u: f32, v: f32| (o.0 + dx * u + px * v, o.1 + dy * u + py * v);

    // Shadow
    let sb = pt(c.sh, -root_back, 0.0);
    let sr = pt(c.sh, 0.0, 0.0);
    let se = pt(c.sh, elbow_u, elbow_v);
    let sw = pt(c.sh, wrist_u, wrist_v);
    let sm1 = pt(c.sh, mid_u, wrist_v - 2.6 * s);
    let sj1 = pt(c.sh, reach, wrist_v - 2.2 * s);
    let sm2 = pt(c.sh, mid_u, wrist_v + 2.6 * s);
    let sj2 = pt(c.sh, reach, wrist_v + 2.2 * s);
    list.sh_capsule(sb, 4.2 * s, sr, 4.0 * s, 255, 4.0);
    list.sh_capsule(sr, 2.5 * s, se, 2.1 * s, 255, 4.0);
    list.sh_ellipse(se.0, se.1, 2.9 * s, 2.9 * s, 255, 4.0);
    list.sh_capsule(se, 1.8 * s, sw, 1.4 * s, 255, 4.0);
    list.sh_capsule(sw, 0.9 * s, sm1, 0.8 * s, 232, 3.5);
    list.sh_capsule(sm1, 0.8 * s, sj1, 0.6 * s, 232, 3.5);
    list.sh_capsule(sw, 0.9 * s, sm2, 0.8 * s, 232, 3.5);
    list.sh_capsule(sm2, 0.8 * s, sj2, 0.6 * s, 232, 3.5);

    // Body
    let bb = pt(c.sc, -root_back, 0.0);
    let br = pt(c.sc, 0.0, 0.0);
    let be = pt(c.sc, elbow_u, elbow_v);
    let bw = pt(c.sc, wrist_u, wrist_v);
    let bm1 = pt(c.sc, mid_u, wrist_v - 2.6 * s);
    let bj1 = pt(c.sc, reach, wrist_v - 2.2 * s);
    let bm2 = pt(c.sc, mid_u, wrist_v + 2.6 * s);
    let bj2 = pt(c.sc, reach, wrist_v + 2.2 * s);
    let bp0 = pt(c.sc, 2.0 * s, -bend * 2.5 * s);
    let bp1 = pt(c.sc, elbow_u * 0.85, elbow_v - bend * 2.1 * s);

    list.bd_capsule(bb, 4.2 * s, br, 3.8 * s, c.pal.deep, c.pal.mid, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_ellipse(br.0, br.1, 3.8 * s, 3.8 * s, c.pal.deep, Some(c.pal.mid), Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_capsule(br, 2.4 * s, be, 2.0 * s, c.pal.deep, c.pal.mid, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_capsule(bp0, 0.7 * s, bp1, 0.65 * s, c.pal.mid, c.pal.soft, Some(c.pal.rim), c.tc, 245, 1.2, 1.2);
    list.bd_capsule(be, 1.75 * s, bw, 1.35 * s, c.pal.deep, c.pal.mid, Some(c.pal.rim), c.tc, 255, 1.8, 1.6);
    list.bd_ellipse(be.0, be.1, 2.9 * s, 2.9 * s, c.pal.deep, Some(c.pal.soft), Some(c.pal.rim), c.tc, 255, 1.8, 1.8);
    list.bd_ellipse(be.0, be.1, 1.3 * s, 1.3 * s, c.pal.deep, Some(c.pal.accent), Some(c.pal.accent), c.tc, 255, 1.0, 1.0);
    list.bd_capsule(bw, 0.85 * s, bm1, 0.75 * s, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.4, 1.2);
    list.bd_capsule(bm1, 0.75 * s, bj1, 0.55 * s, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.3, 1.1);
    list.bd_capsule(bw, 0.85 * s, bm2, 0.75 * s, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.4, 1.2);
    list.bd_capsule(bm2, 0.75 * s, bj2, 0.55 * s, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 255, 1.3, 1.1);
    list.bd_ellipse(bw.0, bw.1, 1.8 * s, 1.8 * s, c.pal.deep, Some(c.pal.accent), Some(c.pal.rim), c.tc, 255, 1.5, 1.4);
}

// 10. Laundry: Clothesline (Span, sways)
fn emit_clothesline(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let line_v = -4.2 * s;
    let sag_max = (2.2 * s).min(hl * 0.08);
    let cw = (2.4 * s).min(hl * 0.34);
    let cord_r = (0.45 * s).max(1.2);
    let sag_at = |u: f32| {
        let t = (u / hl.max(1.0)).clamp(-1.0, 1.0);
        line_v + sag_max * (1.0 - t * t)
    };

    // Wall cleats / tied knots at both ends
    list.sh_rect_uv(c.sh, horiz, -hl, line_v - 1.5 * s, -hl + cw, line_v + 1.5 * s, 235, 3.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, line_v - 1.5 * s, hl, line_v + 1.5 * s, 235, 3.0);
    list.bd_rect_uv(c.sc, horiz, -hl, line_v - 1.4 * s, -hl + cw, line_v + 1.4 * s, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.5, 1.4);
    list.bd_rect_uv(c.sc, horiz, hl - cw, line_v - 1.4 * s, hl, line_v + 1.4 * s, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.5, 1.4);

    // Sagging rope in 8 smooth segments
    const SEGS: usize = 8;
    for i in 0..SEGS {
        let u0 = -hl + (2.0 * hl) * i as f32 / SEGS as f32;
        let u1 = -hl + (2.0 * hl) * (i + 1) as f32 / SEGS as f32;
        list.sh_capsule(uv_to_xy(c.sh, horiz, u0, sag_at(u0)), cord_r, uv_to_xy(c.sh, horiz, u1, sag_at(u1)), cord_r, 175, 2.5);
        list.bd_capsule(
            uv_to_xy(c.sc, horiz, u0, sag_at(u0)),
            cord_r,
            uv_to_xy(c.sc, horiz, u1, sag_at(u1)),
            cord_r,
            c.pal.deep,
            c.pal.soft,
            Some(c.pal.rim),
            c.tc,
            255,
            1.1,
            1.0,
        );
    }

    // Spaced big sheets and shirts along the visible portion of the line
    let step = 15.2 * s;
    let u_origin = -hl + cw + step * 0.55;
    if hl < cw + step * 0.65 {
        // Short corridor span: hang one compact draped towel if there is room.
        let clear = (hl - cw - 2.0).max(0.0);
        if clear >= 5.0 {
            let w_half = (3.4 * s).min(clear * 0.72);
            let h_len = 7.4 * s;
            let top_v = sag_at(0.0);
            let sw_bot = c.sway * (1.4 * s).min(clear * 0.28);
            let u0 = -w_half * 1.1 + sw_bot.min(0.0) - 2.0;
            let u1 = w_half * 1.1 + sw_bot.max(0.0) + 2.0;
            let v0 = top_v - 1.0;
            let v1 = top_v + h_len + 1.0;
            let (sx0, sy0, sx1, sy1) = if horiz {
                (c.sh.0 + u0, c.sh.1 + v0, c.sh.0 + u1, c.sh.1 + v1)
            } else {
                (c.sh.0 + v0, c.sh.1 + u0, c.sh.0 + v1, c.sh.1 + u1)
            };
            list.push_sh(
                sx0,
                sy0,
                sx1,
                sy1,
                ShKind::DrapedSheet {
                    o: c.sh,
                    horiz,
                    u_center: 0.0,
                    v_top: top_v,
                    w_half,
                    h_len,
                    sw_bot,
                    mode: SheetMode::Bedsheet,
                },
            );
            let (bx0, by0, bx1, by1) = if horiz {
                (c.sc.0 + u0, c.sc.1 + v0, c.sc.0 + u1, c.sc.1 + v1)
            } else {
                (c.sc.0 + v0, c.sc.1 + u0, c.sc.0 + v1, c.sc.1 + u1)
            };
            list.push_bd(
                bx0,
                by0,
                bx1,
                by1,
                BdKind::DrapedSheet {
                    o: c.sc,
                    horiz,
                    u_center: 0.0,
                    v_top: top_v,
                    w_half,
                    h_len,
                    sw_bot,
                    mode: SheetMode::Bedsheet,
                    seed: c.seed,
                    pal: c.pal,
                    tc: c.tc,
                },
            );
            for &sign in &[-1.0f32, 1.0] {
                let pu = sign * w_half * 0.65;
                list.bd_rect_uv(
                    c.sc,
                    horiz,
                    pu - 0.5 * s,
                    top_v - 1.1 * s,
                    pu + 0.5 * s,
                    top_v + 0.9 * s,
                    c.pal.accent,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    0.85,
                    0.8,
                );
            }
        }
        return;
    }
    // The clip window along the axis (the scratch window while upscaling).
    let (axis_o, axis_min, axis_max) = if horiz {
        (c.sc.0, list.bx0 as f32, (list.bx0 + list.bw) as f32)
    } else {
        (c.sc.1, list.by0 as f32, (list.by0 + list.bh) as f32)
    };
    let u_vis0 = (axis_min - axis_o - 22.0 * s).max(u_origin);
    let u_vis1 = (axis_max - axis_o + 22.0 * s).min(hl - cw - step * 0.45);
    let first_slot = ((u_vis0 - u_origin) / step).floor().max(0.0) as i32;
    let last_slot = ((u_vis1 - u_origin) / step).ceil() as i32;

    for slot in first_slot..=last_slot {
        let u = u_origin + slot as f32 * step;
        if u < -hl + cw || u > hl - cw - step * 0.45 {
            continue;
        }
        let h = hash_slot(slot, c.seed);
        let top_v = sag_at(u);
        let sw_bot = c.sway * 2.2 * s * (0.8 + 0.35 * ((h >> 3) & 1) as f32);
        let is_sheet = ((slot.rem_euclid(2) as u32 + (c.seed & 1)) & 1) == 0;
        let (mode, w_half, h_len, pin_off) = if is_sheet {
            (
                SheetMode::Bedsheet,
                (4.8 + ((h >> 1) & 3) as f32 * 0.45) * s,
                (8.4 + ((h >> 4) & 1) as f32 * 1.2) * s,
                0.72,
            )
        } else {
            (SheetMode::Shirt, 3.1 * s, 8.0 * s, 0.75)
        };
        let extra_w = if is_sheet { w_half * 1.1 } else { w_half * 1.9 };
        let u0 = u - extra_w + sw_bot.min(0.0) - 2.0;
        let u1 = u + extra_w + sw_bot.max(0.0) + 2.0;
        let v0 = top_v - 1.0;
        let v1 = top_v + h_len + 1.0;

        let (sx0, sy0, sx1, sy1) = if horiz {
            (c.sh.0 + u0, c.sh.1 + v0, c.sh.0 + u1, c.sh.1 + v1)
        } else {
            (c.sh.0 + v0, c.sh.1 + u0, c.sh.0 + v1, c.sh.1 + u1)
        };
        list.push_sh(
            sx0,
            sy0,
            sx1,
            sy1,
            ShKind::DrapedSheet {
                o: c.sh,
                horiz,
                u_center: u,
                v_top: top_v,
                w_half,
                h_len,
                sw_bot,
                mode,
            },
        );

        let (bx0, by0, bx1, by1) = if horiz {
            (c.sc.0 + u0, c.sc.1 + v0, c.sc.0 + u1, c.sc.1 + v1)
        } else {
            (c.sc.0 + v0, c.sc.1 + u0, c.sc.0 + v1, c.sc.1 + u1)
        };
        list.push_bd(
            bx0,
            by0,
            bx1,
            by1,
            BdKind::DrapedSheet {
                o: c.sc,
                horiz,
                u_center: u,
                v_top: top_v,
                w_half,
                h_len,
                sw_bot,
                mode,
                seed: h,
                pal: c.pal,
                tc: c.tc,
            },
        );
        // Wooden clothespins gripping the line
        for &sign in &[-1.0f32, 1.0] {
            let pu = u + sign * w_half * pin_off;
            list.bd_rect_uv(
                c.sc,
                horiz,
                pu - 0.55 * s,
                top_v - 1.15 * s,
                pu + 0.55 * s,
                top_v + 0.95 * s,
                c.pal.accent,
                Some(c.pal.rim),
                c.tc,
                255,
                0.85,
                0.8,
            );
        }
    }
}

// 11. Laundry: SteamPipe (Span)
fn emit_steam_pipe(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let sr = 4.6 * s;
    let strap_r = 5.1 * s;
    let collar_r = strap_r * 1.15;
    let cw = (3.2 * s).min(hl * 0.35);
    let strap_step = 13.0 * s;
    let strap_w = (1.05 * s).max(1.6);
    let has_leak = hl >= 8.0 * s;

    let h = hash_u32(c.seed ^ 0x57EA);
    let leak_u = ((h & 255) as f32 / 255.0 - 0.5) * (hl * 0.75).max(0.0);
    let leak_side = if ((h >> 8) & 1) == 0 { 1.0 } else { -1.0 };

    // Shadow: solid lagged pipe & collars (steam wisp casts no hard shadow)
    list.sh_rect_uv(c.sh, horiz, -hl, -sr, hl, sr, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, -hl, -collar_r, -hl + cw, collar_r, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, -collar_r, hl, collar_r, 255, 4.0);
    if has_leak {
        list.sh_rect_uv(c.sh, horiz, leak_u - 2.1 * s, -collar_r, leak_u + 2.1 * s, collar_r, 255, 4.0);
    }

    // Body: lagged diagonal-wrap pipe + metal straps + split leaking flange
    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - collar_r, c.sc.0 + hl, c.sc.1 + collar_r)
    } else {
        (c.sc.0 - collar_r, c.sc.1 - hl, c.sc.0 + collar_r, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::SteamPipe {
            o: c.sc,
            horiz,
            hl,
            sr,
            strap_r,
            cw,
            strap_step,
            strap_w,
            leak_u,
            has_leak,
            s,
            pal: c.pal,
            tc: c.tc,
        },
    );

    if !has_leak {
        return;
    }
    // Billowing translucent steam jetting from the leaking flange gap
    let puffs: [(f32, f32, f32, f32, u8); 6] = [
        (0.0, 0.8, 1.6, 1.3, 175),
        (0.7, 2.4, 2.7, 2.1, 145),
        (2.1, 4.3, 3.8, 2.8, 110),
        (-1.2, 3.4, 2.8, 2.2, 115),
        (3.8, 6.2, 4.8, 3.5, 78),
        (1.4, 6.8, 4.2, 3.0, 68),
    ];
    let puff_feather = (1.6 * s).max(3.5);
    for &(du, dv, rx, ry, wt) in &puffs {
        let (px, py) = uv_to_xy(c.sc, horiz, leak_u + du * s, leak_side * (sr + dv * s));
        list.bd_ellipse(px, py, rx * s, ry * s, c.pal.pale, None, None, c.tc, wt, puff_feather, 0.0);
    }
}

// 12. Laundry: DryingRack (Drop, see-through slatted airer on 4 cords)
fn emit_drying_rack(list: &mut PrimList, c: &DrawCtx) {
    let s = c.s;
    let sw = c.sway * 1.2 * s;
    let hw = 11.2 * s;
    let hh = 4.8 * s;
    let end_w = 1.6 * s;
    let slat_h = (0.72 * s).max(1.6);
    let cord_r = (0.34 * s).max(1.0);
    let cords = [
        (-hw * 0.82, -hh * 0.85),
        (-hw * 0.52, hh * 0.45),
        (hw * 0.52, hh * 0.45),
        (hw * 0.82, -hh * 0.85),
    ];

    // Shadow
    let (sx, sy) = (c.sh.0 + sw * 0.5, c.sh.1);
    for &(dx_c, dy_c) in &cords {
        list.sh_capsule((sx + dx_c, sy + dy_c), cord_r, (c.sh.0 + dx_c, sy + dy_c - 10.0 * s), cord_r, 145, 2.5);
    }
    for k in 0..4 {
        let t = (k as f32 / 3.0) * 2.0 - 1.0;
        let ry = sy + t * (hh - slat_h);
        list.sh_rect(sx - hw, ry - slat_h * 0.5, sx + hw, ry + slat_h * 0.5, 232, 3.0);
    }
    for &ex in &[-hw + end_w * 0.5, hw - end_w * 0.5] {
        list.sh_rect(sx + ex - end_w * 0.5, sy - hh - 0.5 * s, sx + ex + end_w * 0.5, sy + hh + 0.5 * s, 255, 3.5);
    }
    let t_off = if (c.seed & 1) == 0 { -0.32 } else { 0.28 } * hw;
    let tw = 2.6 * s;
    list.sh_rect(sx + t_off - tw, sy - hh * 0.35, sx + t_off + tw, sy + hh * 1.15, 218, 3.5);

    // Body: four cords rising straight UP for DROP_TAIL
    let (ox, oy) = (c.sc.0 + sw, c.sc.1);
    for &(dx_c, dy_c) in &cords {
        let top_y = oy + dy_c - DROP_TAIL as f32;
        list.bd_capsule((c.sc.0 + dx_c, top_y), cord_r, (ox + dx_c, oy + dy_c), cord_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 235, 1.0, 1.0);
    }
    // 4 parallel wooden slats (wide open gaps between them!)
    for k in 0..4 {
        let t = (k as f32 / 3.0) * 2.0 - 1.0;
        let ry = oy + t * (hh - slat_h);
        list.bd_rect(ox - hw, ry - slat_h * 0.5, ox + hw, ry + slat_h * 0.5, c.pal.accent, Some(c.pal.rim), c.tc, 250, 1.3, 1.1);
    }
    // Cast-iron end brackets
    for &ex in &[-hw + end_w * 0.5, hw - end_w * 0.5] {
        list.bd_rect(ox + ex - end_w * 0.5, oy - hh - 0.5 * s, ox + ex + end_w * 0.5, oy + hh + 0.5 * s, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.6, 1.5);
    }
    // One folded towel draped over a short section
    list.bd_rect(ox + t_off - tw, oy - hh * 0.35, ox + t_off + tw, oy + hh * 1.15, c.pal.mid, Some(c.pal.rim), c.tc, 225, 1.8, 1.5);
    list.bd_rect(ox + t_off - tw + 1.0, oy + hh * 0.65, ox + t_off + tw - 1.0, oy + hh * 0.85, c.pal.soft, None, c.tc, 220, 0.8, 0.0);
}

// 13. Printworks: PaperWeb (Span)
fn emit_paper_web(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let hw = 5.4 * s;
    let rol_hw = 6.6 * s;
    let cw = (3.0 * s).min(hl * 0.35);
    let rol_step = 22.0 * s;
    let rol_w = (1.55 * s).max(2.2);

    list.sh_rect_uv(c.sh, horiz, -hl, -hw, hl, hw, 218, 4.0);
    list.sh_rect_uv(c.sh, horiz, -hl, -rol_hw, -hl + cw, rol_hw, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, -rol_hw, hl, rol_hw, 255, 4.0);
    let mut ru = -hl + rol_step * 0.8;
    while ru < hl - cw * 1.5 {
        list.sh_rect_uv(c.sh, horiz, ru - rol_w * 0.5, -rol_hw, ru + rol_w * 0.5, rol_hw, 246, 3.5);
        ru += rol_step;
    }

    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - rol_hw, c.sc.0 + hl, c.sc.1 + rol_hw)
    } else {
        (c.sc.0 - rol_hw, c.sc.1 - hl, c.sc.0 + rol_hw, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::PaperWeb {
            o: c.sc,
            horiz,
            hl,
            hw,
            rol_hw,
            cw,
            rol_step,
            rol_w,
            s,
            seed: c.seed,
            pal: c.pal,
            tc: c.tc,
        },
    );
}

// 14. Printworks: RollerBeam (Span)
fn emit_roller_beam(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool) {
    let s = c.s;
    let hw = 3.5 * s;
    let flange = 1.1 * s;
    let cw = (3.0 * s).min(hl * 0.35);
    let h = hash_u32(c.seed ^ 0xB3A4);
    let car_u = ((h & 255) as f32 / 255.0 - 0.5) * (hl * 0.75).max(0.0);
    let car_hu = 6.2 * s;
    let car_hv = 5.4 * s;
    let end_hw = hw * 1.35;
    let has_carriage = hl >= car_hu + cw + 4.0;

    // Shadow: gantry beam, end gussets, trolley carriage (dropped on short corridor spans)
    list.sh_rect_uv(c.sh, horiz, -hl, -hw, hl, hw, 248, 4.0);
    list.sh_rect_uv(c.sh, horiz, -hl, -end_hw, -hl + cw, end_hw, 255, 4.0);
    list.sh_rect_uv(c.sh, horiz, hl - cw, -end_hw, hl, end_hw, 255, 4.0);
    if has_carriage {
        list.sh_rect_uv(c.sh, horiz, car_u - car_hu, -car_hv, car_u + car_hu, car_hv, 255, 4.0);
    }

    // Body: I-beam with ribs & punched lightening holes
    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 - end_hw, c.sc.0 + hl, c.sc.1 + end_hw)
    } else {
        (c.sc.0 - end_hw, c.sc.1 - hl, c.sc.0 + end_hw, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::RollerBeam {
            o: c.sc,
            horiz,
            hl,
            hw,
            flange,
            cw,
            car_u: if has_carriage { car_u } else { hl + 100.0 },
            car_hu,
            s,
            pal: c.pal,
            tc: c.tc,
        },
    );

    if !has_carriage {
        return;
    }
    // Wheeled roller carriage riding on the beam
    let wr = 1.85 * s;
    for &du in &[-car_hu * 0.68, car_hu * 0.68] {
        for &dv in &[-hw * 0.95, hw * 0.95] {
            let (wx, wy) = uv_to_xy(c.sc, horiz, car_u + du, dv);
            list.bd_ellipse(wx, wy, wr, wr, c.pal.deep, Some(c.pal.soft), Some(c.pal.rim), c.tc, 255, 1.4, 1.4);
        }
    }
    list.bd_rect_uv(c.sc, horiz, car_u - car_hu, -car_hv * 0.85, car_u + car_hu, car_hv * 0.85, c.pal.deep, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_rect_uv(c.sc, horiz, car_u - car_hu * 0.78, -car_hv * 0.60, car_u + car_hu * 0.78, car_hv * 0.60, c.pal.mid, None, c.tc, 255, 1.2, 0.0);
    list.bd_rect_uv(c.sc, horiz, car_u - car_hu * 0.45, -0.7 * s, car_u + car_hu * 0.45, 0.7 * s, c.pal.accent, Some(c.pal.rim), c.tc, 240, 1.0, 0.8);
}

// 15. Printworks: HangingProofs (Drop, sways)
fn emit_hanging_proofs(list: &mut PrimList, c: &DrawCtx) {
    let s = c.s;
    let sw = c.sway * 1.8 * s;
    let bar_hw = 11.4 * s;
    let bar_y = -5.4 * s;
    let cord_r = (0.36 * s).max(1.0);
    let sheets = [
        (-bar_hw * 0.64, 2.8 * s, 10.4 * s),
        (0.0, 3.0 * s, 11.4 * s),
        (bar_hw * 0.64, 2.8 * s, 10.0 * s),
    ];

    // Shadow
    let (sx, sy) = (c.sh.0 + sw * 0.4, c.sh.1);
    for &dx_c in &[-bar_hw * 0.78, bar_hw * 0.78] {
        list.sh_capsule((sx + dx_c, sy + bar_y), cord_r, (c.sh.0 + dx_c, sy + bar_y - 10.0 * s), cord_r, 160, 2.5);
    }
    list.sh_rect(sx - bar_hw, sy + bar_y - 0.8 * s, sx + bar_hw, sy + bar_y + 0.8 * s, 232, 3.0);

    // Body: twin cords straight UP for DROP_TAIL + top clip rail
    let (ox, oy) = (c.sc.0 + sw * 0.5, c.sc.1);
    for &dx_c in &[-bar_hw * 0.78, bar_hw * 0.78] {
        let top_y = oy + bar_y - DROP_TAIL as f32;
        list.bd_capsule((c.sc.0 + dx_c, top_y), cord_r, (ox + dx_c, oy + bar_y), cord_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 245, 1.0, 1.0);
    }
    list.bd_rect(ox - bar_hw, oy + bar_y - 0.85 * s, ox + bar_hw, oy + bar_y + 0.85 * s, c.pal.deep, Some(c.pal.rim), c.tc, 255, 1.4, 1.2);

    // 3 big hanging broadsheet proof pages + bulldog clips
    for (idx, &(sx_off, shw, sh_h)) in sheets.iter().enumerate() {
        let h = hash_slot(idx as i32, c.seed);
        let sheet_sw = c.sway * 2.0 * s * (0.85 + 0.3 * (idx & 1) as f32);
        let u0 = sx_off - shw * 1.1 + sheet_sw.min(0.0) - 2.0;
        let u1 = sx_off + shw * 1.1 + sheet_sw.max(0.0) + 2.0;
        let v0 = bar_y + 0.9 * s;
        let v1 = v0 + sh_h;

        list.push_sh(
            sx + u0,
            sy + v0 - 1.0,
            sx + u1,
            sy + v1 + 1.0,
            ShKind::DrapedSheet {
                o: (sx, sy),
                horiz: true,
                u_center: sx_off,
                v_top: v0,
                w_half: shw,
                h_len: sh_h,
                sw_bot: sheet_sw,
                mode: SheetMode::Proof,
            },
        );
        list.push_bd(
            ox + u0,
            oy + v0 - 1.0,
            ox + u1,
            oy + v1 + 1.0,
            BdKind::DrapedSheet {
                o: (ox, oy),
                horiz: true,
                u_center: sx_off,
                v_top: v0,
                w_half: shw,
                h_len: sh_h,
                sw_bot: sheet_sw,
                mode: SheetMode::Proof,
                seed: h,
                pal: c.pal,
                tc: c.tc,
            },
        );
        for &sign in &[-1.0f32, 1.0] {
            let cl_x = ox + sx_off + sign * shw * 0.62;
            list.bd_rect(cl_x - 0.55 * s, oy + bar_y - 1.1 * s, cl_x + 0.55 * s, oy + bar_y + 1.3 * s, c.pal.deep, Some(c.pal.rim), c.tc, 255, 0.9, 0.9);
        }
    }
}

// 16. Flatbed (Outdoors): TreeBranch (Edge / Sky)
fn build_leaf_lobes(cx: f32, cy: f32, lrx: f32, lry: f32, seed: u32) -> [(f32, f32, f32, f32, f32); 5] {
    let mut lobes = [(0.0, 0.0, 1.0, 1.0, 1.0); 5];
    for (k, slot) in lobes.iter_mut().enumerate() {
        let lh = hash_u32(seed ^ ((k as u32).wrapping_mul(977).wrapping_add(31)));
        let ring_idx = ((k * 3) + (lh & 1) as usize) & 15;
        let (ca, sa) = UNIT_RING[ring_idx];
        let dist = 0.25 + ((lh >> 5) & 15) as f32 * (0.32 / 15.0);
        let rad = 0.46 + ((lh >> 9) & 15) as f32 * (0.20 / 15.0);
        let rx = (lrx * rad).max(1.0);
        let ry = (lry * rad).max(1.0);
        *slot = (
            cx + ca * lrx * dist,
            cy + sa * lry * dist,
            1.0 / (rx * rx),
            1.0 / (ry * ry),
            rx.min(ry),
        );
    }
    lobes
}

fn emit_tree_branch(list: &mut PrimList, c: &DrawCtx, side: Side, reach: f32) {
    let s = c.s;
    let (dx, dy, px, py) = edge_basis(side);
    let h = hash_u32(c.seed ^ 0x72EE);
    let lean = ((h & 63) as f32 / 63.0 - 0.5) * 0.38;
    let root_back = 10.0 * s;
    let r_root = ((5.4 + ((h >> 6) & 3) as f32 * 0.35) * s).min(reach * 0.18);
    let r_f1 = r_root * 0.46;
    let r_f2 = r_root * 0.23;
    let r_tip = (r_root * 0.10).max(1.2);

    let inv_reach = 1.0 / reach.max(1.0);
    let sway_amp = c.sway * 2.2 * s;
    let pt = |o: (f32, f32), u: f32, v: f32| {
        let t = (u * inv_reach).clamp(0.0, 1.0);
        let ve = v + u * lean + sway_amp * t;
        (o.0 + dx * u + px * ve, o.1 + dy * u + py * ve)
    };

    let f1_v = reach * 0.04 * if (h & 256) != 0 { 1.0 } else { -1.0 };
    let p_back = (-root_back, 0.0);
    let p_root = (0.0, 0.0);
    let p_f1 = (reach * 0.38, f1_v);
    let p_mid = (reach * 0.21, f1_v * 0.55 - r_root * 0.35);
    let p_side = (reach * 0.46, -reach * 0.34);
    let p_a = (reach * 0.70, -reach * 0.26);
    let p_b = (reach * 0.68, reach * 0.28);

    let twigs = [
        (p_mid, p_side),
        (p_a, (reach * 0.96, -reach * 0.42)),
        (p_a, (reach * 0.92, -reach * 0.08)),
        (p_b, (reach * 0.95, reach * 0.10)),
        (p_b, (reach * 0.94, reach * 0.44)),
    ];

    let clusters = [
        (reach * 0.46, -reach * 0.36, 3.6 * s, 2.8 * s),
        (reach * 0.62, -reach * 0.24, 3.2 * s, 2.6 * s),
        (reach * 0.76, -reach * 0.36, 3.8 * s, 3.0 * s),
        (reach * 0.95, -reach * 0.43, 3.5 * s, 2.8 * s),
        (reach * 0.91, -reach * 0.10, 3.4 * s, 2.6 * s),
        (reach * 0.80, -reach * 0.15, 2.8 * s, 2.4 * s),
        (reach * 0.60, reach * 0.24, 3.4 * s, 2.7 * s),
        (reach * 0.76, reach * 0.35, 3.6 * s, 2.9 * s),
        (reach * 0.95, reach * 0.12, 3.3 * s, 2.6 * s),
        (reach * 0.94, reach * 0.44, 3.5 * s, 2.8 * s),
        (reach * 0.82, reach * 0.48, 2.8 * s, 2.3 * s),
    ];

    // Shadow: limbs + twigs + dappled ragged leaf clusters
    list.sh_capsule(pt(c.sh, p_back.0, p_back.1), r_root * 1.15, pt(c.sh, p_root.0, p_root.1), r_root, 255, 4.0);
    list.sh_capsule(pt(c.sh, p_root.0, p_root.1), r_root, pt(c.sh, p_f1.0, p_f1.1), r_f1, 255, 4.0);
    list.sh_capsule(pt(c.sh, p_f1.0, p_f1.1), r_f1, pt(c.sh, p_a.0, p_a.1), r_f2, 248, 4.0);
    list.sh_capsule(pt(c.sh, p_f1.0, p_f1.1), r_f1, pt(c.sh, p_b.0, p_b.1), r_f2, 248, 4.0);
    for &(ta, tb) in &twigs {
        list.sh_capsule(pt(c.sh, ta.0, ta.1), r_f2, pt(c.sh, tb.0, tb.1), r_tip, 220, 3.5);
    }
    for (ci, &(cu, cv_p, lrx, lry)) in clusters.iter().enumerate() {
        let (cx, cy) = pt(c.sh, cu, cv_p);
        let lobes = build_leaf_lobes(cx, cy, lrx, lry, hash_slot(ci as i32, c.seed));
        list.push_sh(
            cx - lrx * 1.15,
            cy - lry * 1.15,
            cx + lrx * 1.15,
            cy + lry * 1.15,
            ShKind::LeafCluster { lobes, w: 226 },
        );
    }

    // Body: twigs -> secondary limbs -> primary trunk (so trunk bark flows seamlessly!) -> leaf clusters
    for &(ta, tb) in &twigs {
        list.bd_capsule(
            pt(c.sc, ta.0, ta.1),
            r_f2,
            pt(c.sc, tb.0, tb.1),
            r_tip,
            c.pal.bark_deep,
            c.pal.bark_mid,
            Some(c.pal.rim),
            c.tc,
            255,
            1.5,
            1.1,
        );
    }
    list.bd_capsule(pt(c.sc, p_f1.0, p_f1.1), r_f1, pt(c.sc, p_a.0, p_a.1), r_f2, c.pal.bark_deep, c.pal.bark_mid, Some(c.pal.rim), c.tc, 255, 1.8, 1.4);
    list.bd_capsule(pt(c.sc, p_f1.0, p_f1.1), r_f1, pt(c.sc, p_b.0, p_b.1), r_f2, c.pal.bark_deep, c.pal.bark_mid, Some(c.pal.rim), c.tc, 255, 1.8, 1.4);
    list.bd_capsule(pt(c.sc, p_back.0, p_back.1), r_root * 1.15, pt(c.sc, p_root.0, p_root.1), r_root, c.pal.bark_deep, c.pal.bark_mid, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_capsule(pt(c.sc, p_root.0, p_root.1), r_root, pt(c.sc, p_f1.0, p_f1.1), r_f1, c.pal.bark_deep, c.pal.bark_mid, Some(c.pal.rim), c.tc, 255, 2.0, 1.6);

    for (ci, &(cu, cv_p, lrx, lry)) in clusters.iter().enumerate() {
        let (cx, cy) = pt(c.sc, cu, cv_p);
        let cseed = hash_slot(ci as i32, c.seed);
        let lobes = build_leaf_lobes(cx, cy, lrx, lry, cseed);
        list.push_bd(
            cx - lrx * 1.15,
            cy - lry * 1.15,
            cx + lrx * 1.15,
            cy + lry * 1.15,
            BdKind::LeafCluster {
                cx,
                cy,
                min_r: lrx.min(lry),
                lobes,
                seed: cseed,
                pal: c.pal,
                tc: c.tc,
            },
        );
    }
}

// 17. Flatbed (Outdoors): Cloud (Sky; floor shadow is the hero, faint haze body <= 60 alpha)
fn emit_cloud(list: &mut PrimList, c: &DrawCtx) {
    let base_rx = cloud_base_rx(c.seed) * c.s;
    let base_ry = base_rx * 0.42;
    let lobes: [(f32, f32, f32, f32); 7] = [
        (-0.56, 0.14, 0.44, 0.56),
        (-0.24, -0.18, 0.46, 0.72),
        (0.08, -0.26, 0.50, 0.78),
        (0.42, -0.06, 0.44, 0.64),
        (0.62, 0.18, 0.38, 0.50),
        (-0.08, 0.22, 0.58, 0.54),
        (0.28, 0.22, 0.52, 0.52),
    ];
    let sh_feather = (10.0 * c.s).max(5.0);
    let bd_feather = (12.0 * c.s).max(6.0);
    for &(ox_n, oy_n, rx_n, ry_n) in &lobes {
        list.sh_ellipse(
            c.sh.0 + ox_n * base_rx,
            c.sh.1 + oy_n * base_ry,
            rx_n * base_rx,
            ry_n * base_ry,
            255,
            sh_feather,
        );
        list.bd_ellipse(
            c.sc.0 + ox_n * base_rx,
            c.sc.1 + oy_n * base_ry,
            rx_n * base_rx,
            ry_n * base_ry,
            c.pal.pale,
            None,
            None,
            c.tc,
            255,
            bd_feather,
            0.0,
        );
    }
}

// 18a. Flatbed (Outdoors): PowerLine — 3 sagging catenary wires
fn emit_power_wires(list: &mut PrimList, c: &DrawCtx, hl: f32, horiz: bool, draw_knobs: bool) {
    let s = c.s;
    // One art pixel thick (the layer's upscale makes it `k` px on screen).
    let wire_r = 0.75;
    let spacing = 4.2 * s;
    let sag = (3.4 * s).min(22.0);
    let v_lo = -spacing - wire_r - 3.0;
    let v_hi = spacing + sag * 1.12 + wire_r + 3.0;

    let (sx0, sy0, sx1, sy1) = if horiz {
        (c.sh.0 - hl, c.sh.1 + v_lo, c.sh.0 + hl, c.sh.1 + v_hi)
    } else {
        (c.sh.0 + v_lo, c.sh.1 - hl, c.sh.0 + v_hi, c.sh.1 + hl)
    };
    list.push_sh(
        sx0,
        sy0,
        sx1,
        sy1,
        ShKind::PowerWires {
            o: c.sh,
            horiz,
            hl,
            spacing,
            sag,
            wire_r,
        },
    );

    let (bx0, by0, bx1, by1) = if horiz {
        (c.sc.0 - hl, c.sc.1 + v_lo, c.sc.0 + hl, c.sc.1 + v_hi)
    } else {
        (c.sc.0 + v_lo, c.sc.1 - hl, c.sc.0 + v_hi, c.sc.1 + hl)
    };
    list.push_bd(
        bx0,
        by0,
        bx1,
        by1,
        BdKind::PowerWires {
            o: c.sc,
            horiz,
            hl,
            spacing,
            sag,
            wire_r,
            rim: c.pal.rim,
            tc: c.tc,
        },
    );

    if draw_knobs {
        let kr = (0.75 * s).max(1.4);
        for &end_u in &[-hl + kr, hl - kr] {
            for &vb in &[-spacing, 0.0, spacing] {
                let (kx, ky) = uv_to_xy(c.sc, horiz, end_u, vb);
                list.bd_ellipse(kx, ky, kr, kr, c.pal.deep, Some(c.pal.accent), Some(c.pal.rim), c.tc, 255, 1.0, 1.0);
            }
        }
    }
}

// 18b. Flatbed (Outdoors): PowerLine — passing utility pole with crossarm & insulators
fn emit_power_pole(list: &mut PrimList, c: &DrawCtx) {
    let s = c.s;
    let pole_r_bot = (1.75 * s).max(2.0);
    let pole_r_top = (1.35 * s).max(1.6);
    let pole_top = -17.0 * s;
    let pole_bot = 22.0 * s;
    let arm_y = -6.5 * s;
    let arm_hw = 10.5 * s;
    let arm_hh = (1.15 * s).max(1.6);
    let ins_xs = [-arm_hw * 0.80, 0.0, arm_hw * 0.80];

    // Shadow
    let (sx, sy) = c.sh;
    list.sh_capsule((sx, sy + pole_top), pole_r_top, (sx, sy + pole_bot), pole_r_bot, 255, 4.0);
    list.sh_rect(sx - arm_hw, sy + arm_y - arm_hh, sx + arm_hw, sy + arm_y + arm_hh, 255, 4.0);
    list.sh_capsule((sx, sy + arm_y + 4.8 * s), 0.55 * s, (sx - arm_hw * 0.58, sy + arm_y + arm_hh), 0.55 * s, 218, 3.0);
    list.sh_capsule((sx, sy + arm_y + 4.8 * s), 0.55 * s, (sx + arm_hw * 0.58, sy + arm_y + arm_hh), 0.55 * s, 218, 3.0);

    // Body: diagonal V-braces, creosote wooden pole, timber crossarm, 3 ceramic insulators
    let (ox, oy) = c.sc;
    let br_r = (0.45 * s).max(1.0);
    list.bd_capsule((ox, oy + arm_y + 4.8 * s), br_r, (ox - arm_hw * 0.58, oy + arm_y + arm_hh * 0.6), br_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 250, 1.2, 1.0);
    list.bd_capsule((ox, oy + arm_y + 4.8 * s), br_r, (ox + arm_hw * 0.58, oy + arm_y + arm_hh * 0.6), br_r, c.pal.deep, c.pal.soft, Some(c.pal.rim), c.tc, 250, 1.2, 1.0);
    list.bd_capsule((ox, oy + pole_top), pole_r_top, (ox, oy + pole_bot), pole_r_bot, c.pal.bark_deep, c.pal.bark_mid, Some(c.pal.rim), c.tc, 255, 2.0, 1.8);
    list.bd_rect(ox - arm_hw, oy + arm_y - arm_hh, ox + arm_hw, oy + arm_y + arm_hh, c.pal.bark_deep, Some(c.pal.rim), c.tc, 255, 1.8, 1.6);
    list.bd_rect(ox - arm_hw + 1.5, oy + arm_y - arm_hh * 0.55, ox + arm_hw - 1.5, oy + arm_y + arm_hh * 0.55, c.pal.bark_mid, None, c.tc, 240, 1.0, 0.0);
    let ir = (0.85 * s).max(1.5);
    for &ix in &ins_xs {
        list.bd_rect(ox + ix - 0.35 * s, oy + arm_y - arm_hh - 1.6 * s, ox + ix + 0.35 * s, oy + arm_y - arm_hh, c.pal.deep, None, c.tc, 255, 0.8, 0.0);
        list.bd_ellipse(ox + ix, oy + arm_y - arm_hh - 1.7 * s, ir, ir * 1.15, c.pal.accent, Some(c.pal.pale), Some(c.pal.rim), c.tc, 255, 1.2, 1.2);
    }
}

// ---- Unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;

    const ALL_KINDS: [(ForeKind, Theme, Attach); 18] = [
        (ForeKind::MainPipe, Theme::Sewer, Attach::Span),
        (ForeKind::CatwalkGrating, Theme::Sewer, Attach::Span),
        (ForeKind::ChainHoist, Theme::Sewer, Attach::Drop),
        (ForeKind::ExtractorHood, Theme::Kitchen, Attach::Edge(Side::N)),
        (ForeKind::PotRail, Theme::Kitchen, Attach::Span),
        (ForeKind::HangingLamp, Theme::Kitchen, Attach::Drop),
        (ForeKind::CableTray, Theme::Lab, Attach::Span),
        (ForeKind::VentDuct, Theme::Lab, Attach::Span),
        (ForeKind::ArticulatedArm, Theme::Lab, Attach::Edge(Side::E)),
        (ForeKind::Clothesline, Theme::Laundry, Attach::Span),
        (ForeKind::SteamPipe, Theme::Laundry, Attach::Span),
        (ForeKind::DryingRack, Theme::Laundry, Attach::Drop),
        (ForeKind::PaperWeb, Theme::Printworks, Attach::Span),
        (ForeKind::RollerBeam, Theme::Printworks, Attach::Span),
        (ForeKind::HangingProofs, Theme::Printworks, Attach::Drop),
        (ForeKind::TreeBranch, Theme::Trailer, Attach::Edge(Side::N)),
        (ForeKind::Cloud, Theme::Trailer, Attach::Sky),
        (ForeKind::PowerLine, Theme::Trailer, Attach::Sky),
    ];

    fn make_item(
        kind: ForeKind,
        attach: Attach,
        len: f32,
        horizontal: bool,
        scale: f32,
        seed: u32,
    ) -> ForeItem {
        ForeItem {
            kind,
            pos: Vec2::new(120.0, 74.0),
            len,
            horizontal,
            attach,
            depth: 2.2,
            scale,
            seed,
        }
    }

    fn make_draw(screen: (i32, i32), scale: f32, alpha: u8, sway: f32, theme: Theme) -> ForeDraw {
        ForeDraw {
            screen,
            shadow: (screen.0 + 10, screen.1 + 14),
            scale,
            alpha,
            sway,
            theme,
            toward_centre: (1, 1),
        }
    }

    #[test]
    fn extent_grows_with_scale_for_every_kind() {
        for &(kind, _, attach) in &ALL_KINDS {
            let len = match attach {
                Attach::Span => 160.0,
                Attach::Edge(_) => 80.0,
                Attach::Drop | Attach::Sky => 0.0,
            };
            let item1 = make_item(kind, attach, len, true, 1.0, 42);
            let item5 = make_item(kind, attach, len, true, 5.0, 42);
            let (l1, a1) = extent(&item1);
            let (l5, a5) = extent(&item5);
            assert!(l1 > 0.0 && a1 > 0.0, "{kind:?} extent must be positive");
            assert!(
                l5 >= l1 && a5 > a1,
                "{kind:?} extent must grow with scale: scale 1 ({l1}, {a1}) vs scale 5 ({l5}, {a5})"
            );
            if kind != ForeKind::ChainHoist {
                assert!(
                    l1 >= a1 * 0.99,
                    "{kind:?} extent should report long axis first: ({l1}, {a1})"
                );
            }
        }
        // Also verify PowerLine pole mode grows with scale
        let p1 = make_item(ForeKind::PowerLine, Attach::Sky, 0.0, false, 1.0, 7);
        let p5 = make_item(ForeKind::PowerLine, Attach::Sky, 0.0, false, 5.0, 7);
        let (pl1, pa1) = extent(&p1);
        let (pl5, pa5) = extent(&p5);
        assert!(pl5 > pl1 && pa5 > pa1, "PowerLine pole extent must grow with scale");
    }

    #[test]
    fn every_kind_paints_something_and_respects_zero_alpha() {
        let bg = (76, 82, 88);
        for &(kind, theme, attach) in &ALL_KINDS {
            let len = match attach {
                Attach::Span => 180.0,
                Attach::Edge(_) => 90.0,
                Attach::Drop | Attach::Sky => 0.0,
            };
            let item = make_item(kind, attach, len, true, 4.0, 19);
            let mut pb = PixelBuffer::new(240, 148);
            pb.clear(bg);
            let anchor = match attach {
                Attach::Edge(Side::N) => (120, 10),
                Attach::Edge(Side::E) => (230, 74),
                _ => (120, 74),
            };
            draw(&mut pb, &item, &make_draw(anchor, 4.0, ALPHA_MAX, 0.25, theme));
            let touched = pb.pixels().iter().filter(|&&p| p != bg).count();
            assert!(touched > 30, "{kind:?} painted too few pixels ({touched})");

            // Zero alpha must leave the buffer untouched.
            let mut pb0 = PixelBuffer::new(240, 148);
            pb0.clear(bg);
            draw(&mut pb0, &item, &make_draw(anchor, 4.0, 0, 0.25, theme));
            assert!(
                pb0.pixels().iter().all(|&p| p == bg),
                "{kind:?} painted pixels when alpha == 0"
            );
        }

        // Also test PowerLine passing pole (len == 0 && !horizontal)
        let pole = make_item(ForeKind::PowerLine, Attach::Sky, 0.0, false, 4.0, 55);
        let mut pb = PixelBuffer::new(240, 148);
        pb.clear(bg);
        draw(&mut pb, &pole, &make_draw((120, 74), 4.0, ALPHA_MAX, 0.0, Theme::Trailer));
        assert!(
            pb.pixels().iter().filter(|&&p| p != bg).count() > 80,
            "PowerLine pole painted too few pixels"
        );
    }

    #[test]
    fn every_span_paints_near_both_ends() {
        let bg = (90, 90, 90);
        let len = 180.0f32;
        let hl = (len * 0.5) as i32; // 90
        let cx = 120i32;
        let cy = 120i32;

        for &(kind, theme, attach) in &ALL_KINDS {
            if !matches!(attach, Attach::Span) {
                continue;
            }
            for &horizontal in &[true, false] {
                let item = make_item(kind, Attach::Span, len, horizontal, 4.0, 29);
                let mut pb = PixelBuffer::new(240, 240);
                pb.clear(bg);
                let d = ForeDraw {
                    screen: (cx, cy),
                    shadow: (cx + 10, cy + 14),
                    scale: 4.0,
                    alpha: ALPHA_MAX,
                    sway: 0.0,
                    theme,
                    toward_centre: (0, 1),
                };
                draw(&mut pb, &item, &d);

                // Check a narrow band within 8 px inside BOTH ends (-hl and +hl)
                let mut near_neg = false;
                let mut near_pos = false;
                for off in 1..=8 {
                    for cross in -35..=35 {
                        let (nx, ny, px, py) = if horizontal {
                            (cx - hl + off, cy + cross, cx + hl - off, cy + cross)
                        } else {
                            (cx + cross, cy - hl + off, cx + cross, cy + hl - off)
                        };
                        if nx >= 0 && ny >= 0 && (nx as usize) < pb.w && (ny as usize) < pb.h
                            && pb.pixel_at(nx as usize, ny as usize) != bg
                        {
                            near_neg = true;
                        }
                        if px >= 0 && py >= 0 && (px as usize) < pb.w && (py as usize) < pb.h
                            && pb.pixel_at(px as usize, py as usize) != bg
                        {
                            near_pos = true;
                        }
                    }
                }
                assert!(
                    near_neg && near_pos,
                    "{kind:?} (horiz={horizontal}) did not paint near both ends (neg={near_neg}, pos={near_pos})"
                );
            }
        }
    }

    #[test]
    fn drop_kinds_paint_suspension_tail_to_top_of_buffer() {
        let bg = (84, 88, 92);
        for &(kind, theme, attach) in &ALL_KINDS {
            if !matches!(attach, Attach::Drop) {
                continue;
            }
            let item = make_item(kind, Attach::Drop, 0.0, true, 4.0, 13);
            let mut pb = PixelBuffer::new(240, 148);
            pb.clear(bg);
            draw(&mut pb, &item, &make_draw((120, 100), 4.0, ALPHA_MAX, 0.0, theme));
            // Top scanline y = 0 must receive the suspension chain/cord(s)
            let top_hits = (0..pb.w).filter(|&x| pb.pixel_at(x, 0) != bg).count();
            assert!(
                top_hits > 0,
                "{kind:?} did not extend its Drop tail all the way to the top edge"
            );
        }
    }

    #[test]
    fn cloud_body_haze_is_faint_and_shadow_darkens() {
        let bg = (120, 110, 96);
        let item = make_item(ForeKind::Cloud, Attach::Sky, 0.0, true, 1.1, 101);
        let mut pb = PixelBuffer::new(240, 148);
        pb.clear(bg);
        let d = ForeDraw {
            screen: (90, 50),
            shadow: (130, 90),
            scale: 1.1,
            alpha: ALPHA_MAX,
            sway: 0.0,
            theme: Theme::Trailer,
            toward_centre: (1, 1),
        };
        draw(&mut pb, &item, &d);
        // Shadow center must be darker than background
        let sh_px = pb.pixel_at(130, 90);
        assert!(
            sh_px.0 < bg.0 && sh_px.1 < bg.1 && sh_px.2 < bg.2,
            "Cloud shadow should darken floor: got {sh_px:?} vs bg {bg:?}"
        );
        // Body haze over pure black must stay very faint (<= 60/255 * 0.72 * ~200 < 38)
        let mut pb_black = PixelBuffer::new(240, 148);
        pb_black.clear((0, 0, 0));
        draw(&mut pb_black, &item, &d);
        let peak = pb_black
            .pixels()
            .iter()
            .map(|p| p.0.max(p.1).max(p.2))
            .max()
            .unwrap_or(0);
        assert!(
            peak <= 48,
            "Cloud body haze should stay faint (alpha <= 60), got peak {peak}"
        );
    }

    #[test]
    fn swaying_kinds_respond_to_sway() {
        let bg = (70, 72, 78);
        for &(kind, theme, attach) in &[
            (ForeKind::PotRail, Theme::Kitchen, Attach::Span),
            (ForeKind::HangingLamp, Theme::Kitchen, Attach::Drop),
            (ForeKind::Clothesline, Theme::Laundry, Attach::Span),
            (ForeKind::HangingProofs, Theme::Printworks, Attach::Drop),
            (ForeKind::TreeBranch, Theme::Trailer, Attach::Edge(Side::N)),
        ] {
            let len = match attach {
                Attach::Span => 180.0,
                Attach::Edge(_) => 100.0,
                _ => 0.0,
            };
            let anchor = if matches!(attach, Attach::Edge(Side::N)) {
                (120, 10)
            } else {
                (120, 74)
            };
            let item = make_item(kind, attach, len, !matches!(attach, Attach::Edge(_)), 4.0, 21);
            let mut pb_a = PixelBuffer::new(240, 148);
            let mut pb_b = PixelBuffer::new(240, 148);
            pb_a.clear(bg);
            pb_b.clear(bg);
            draw(&mut pb_a, &item, &make_draw(anchor, 4.0, ALPHA_MAX, -1.0, theme));
            draw(&mut pb_b, &item, &make_draw(anchor, 4.0, ALPHA_MAX, 1.0, theme));
            assert_ne!(pb_a.pixels(), pb_b.pixels(), "{kind:?} should respond to sway");
        }
    }

    #[test]
    fn short_corridor_spans_stay_clean_at_28_px() {
        let bg = (82, 84, 88);
        for &(kind, theme, attach) in &ALL_KINDS {
            if !matches!(attach, Attach::Span) {
                continue;
            }
            for &horizontal in &[true, false] {
                let item = make_item(kind, Attach::Span, 28.0, horizontal, 5.0, 17);
                let mut pb = PixelBuffer::new(200, 200);
                pb.clear(bg);
                let d = ForeDraw {
                    screen: (100, 100),
                    shadow: (110, 114),
                    scale: 5.0,
                    alpha: ALPHA_MAX,
                    sway: 0.25,
                    theme,
                    toward_centre: (0, 1),
                };
                draw(&mut pb, &item, &d);
                let touched = pb.pixels().iter().filter(|&&p| p != bg).count();
                assert!(
                    touched > 40,
                    "short 28 px corridor span {kind:?} (horiz={horizontal}) painted too little ({touched})"
                );
            }
        }
    }
}
