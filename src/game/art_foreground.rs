//! Art for the foreground layer v3 ([`crate::render::foreground`]).
//!
//! Every foreground item is a continuous **band** crossing the entire view edge
//! to edge along one axis (horizontal or vertical), rendered in art pixels that
//! the layer upscales nearest-neighbour by `k = 2` or `3`.
//!
//! ## Contract
//!
//! ```ignore
//! pub fn thickness(item: &ForeItem) -> f32
//! pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw)
//! ```
//!
//! * Eighteen overhead band kinds, three per floor theme:
//!   - **Sewer**: `MainPipe`, `CatwalkGrating`, `ChainRun`
//!   - **Kitchen**: `ExtractorDuct`, `PotRail`, `CableRun`
//!   - **Lab**: `CableTray`, `VentDuct`, `Gantry`
//!   - **Laundry**: `Clothesline`, `SteamPipe`, `RackBeam`
//!   - **Printworks**: `PaperWeb`, `RollerBeam`, `ProofLine`
//!   - **Flatbed (`Theme::Trailer`)**: `BranchCanopy`, `PowerLines`, `CloudBand`
//! * [`thickness`] returns the band's full cross-axis thickness in art pixels
//!   (`base * scale`, landing in `10..=30` art px for `scale` in `1.0..=1.6`).
//!   Every kind stays strictly inside `[-thickness/2, +thickness/2]` and matches
//!   [`thickness`] within 1 art px.
//! * Every pixel write goes exclusively through [`shadow`] (pass 1, centred on
//!   `d.shadow_cross`) and [`blend`] (pass 2, centred on `d.screen_cross`).
//!   `ForeKind::CloudBand` is a soft cloud shadow only (zero [`blend`] writes).
//! * All repeating detail (flanges, rungs, chain links, pans, lamp shades,
//!   cables, duct seams, carriages, laundry, steam clamps, slats, rollers,
//!   proof sheets, leaf clusters, power crossarms, cloud billows) is laid out
//!   from `wu = u + d.along_offset` so it stays world-anchored and never crawls
//!   or pops as the camera moves.
//! * Rasterized through a stack-allocated tile canvas (`BandTile`, zero heap
//!   allocations per frame, single write per pixel per pass so effective opacity
//!   never double-blends or exceeds `d.alpha`).

use crate::game::level::Theme;
use crate::render::foreground::{
    blend, rim_tint, shadow, ForeDraw, ForeItem, ForeKind, ALPHA_MAX,
};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Along-axis tile width on the stack (art px). Covers an entire 1x view plus
/// overhang in one pass; wider extents loop in `TILE_U`-wide chunks.
const TILE_U: usize = 96;
/// Cross-axis tile capacity on the stack (art px). Comfortably bounds any band
/// up to ~60 art px thick with zero heap allocation.
const MAX_V: usize = 64;
const TILE_CELLS: usize = TILE_U * MAX_V;

/// How much lighter than the body its rim highlight is (~38 %).
const RIM_LIGHTEN: f32 = 1.38;
/// Underside / far-edge shade as a share of the body colour.
const DEEP_SHADE: f32 = 0.68;
/// Lit-side face colour as a share of the body colour.
const SOFT_LIFT: f32 = 1.18;
/// Share of the floor's ambient character tint mixed into the rim.
const RIM_TINT_MIX: f32 = 0.16;

/// Base band thickness in art pixels at `scale = 1.0`.
/// At `scale` in `1.0..=1.6`, `base * scale` lands in `13.0..=29.6` art px
/// (`26..=89` screen px after `k = 2..=3` upscale).
#[inline]
const fn base_thickness(kind: ForeKind) -> f32 {
    match kind {
        // Sewer
        ForeKind::MainPipe => 13.0,
        ForeKind::CatwalkGrating => 15.0,
        ForeKind::ChainRun => 16.0,
        // Kitchen
        ForeKind::ExtractorDuct => 16.5,
        ForeKind::PotRail => 16.0,
        ForeKind::CableRun => 16.0,
        // Lab
        ForeKind::CableTray => 14.0,
        ForeKind::VentDuct => 14.5,
        ForeKind::Gantry => 15.5,
        // Laundry
        ForeKind::Clothesline => 16.5,
        ForeKind::SteamPipe => 13.5,
        ForeKind::RackBeam => 15.0,
        // Printworks
        ForeKind::PaperWeb => 15.0,
        ForeKind::RollerBeam => 14.0,
        ForeKind::ProofLine => 16.5,
        // Flatbed (Outdoors)
        ForeKind::BranchCanopy => 18.0,
        ForeKind::PowerLines => 13.5,
        ForeKind::CloudBand => 18.5,
    }
}

/// Full cross-axis thickness of `item` in **art** pixels (for culling, scratch
/// sizing, and the rat/threat fade).
pub fn thickness(item: &ForeItem) -> f32 {
    base_thickness(item.kind) * item.scale.max(0.25)
}

/// Draw one foreground band into `pb`: soft floor shadow first (via [`shadow`]
/// around `d.shadow_cross`), then the overhead silhouette body (via [`blend`]
/// around `d.screen_cross`).
///
/// Every coordinate in `d` (`from`, `to`, `screen_cross`, `shadow_cross`) is in
/// art pixels; the layer upscales the result nearest-neighbour.
pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw) {
    if d.alpha == 0 || d.from > d.to {
        return;
    }
    let scale = d.scale.max(0.25);
    let th = base_thickness(item.kind) * scale;
    let ht = th * 0.5;
    // Align integer pixel centres to the continuous [-ht, +ht] interval so the
    // drawn cross-axis span matches `th.round()` (error <= 0.5 art px for all th).
    let v_c = if (th.round() as i32) & 1 == 0 {
        -0.5
    } else {
        0.0
    };
    let half_i = (ht.ceil() as i32 + 1).min((MAX_V as i32 / 2) - 1);
    let dv0 = -half_i;
    let vh = ((half_i * 2 + 1) as usize).min(MAX_V);

    let tc = if d.toward_centre < 0 { -1.0 } else { 1.0 };
    let pal = ForePal::for_theme(d.theme);
    let alpha_norm = (d.alpha as f32 / (ALPHA_MAX as f32).max(1.0)).clamp(0.0, 1.0);
    let body_alpha = d.alpha.min(ALPHA_MAX);
    let max_shadow = if item.kind == ForeKind::CloudBand {
        0.42 * alpha_norm
    } else {
        0.34 * (0.55 + 0.45 * alpha_norm)
    };
    // Indoor bands stay still when the camera is still; band detail is driven
    // by `along_offset` and `sway`.
    let _ = d.elapsed_step;

    let ctx = BandCtx {
        s: scale,
        ht,
        along: d.along_offset as f32,
        sway: d.sway.clamp(-1.0, 1.0),
        seed: item.seed,
        pal,
        tc,
    };

    let total_u = (d.to as i64 - d.from as i64 + 1) as usize;
    let mut tile = BandTile::new(d.from, total_u.min(TILE_U), dv0, vh, v_c, ht);

    if total_u <= TILE_U {
        // Common 1x path: entire band fits in one stack tile -> paint once,
        // flush shadow pass completely before body pass.
        paint_kind(item.kind, &mut tile, &ctx);
        tile.flush_shadow(pb, item.horizontal, d.shadow_cross, max_shadow);
        if item.kind != ForeKind::CloudBand {
            tile.flush_body(pb, item.horizontal, d.screen_cross, body_alpha);
        }
    } else {
        // Wide extent (zoomed out / large terminal): two chunked passes so
        // every shadow write strictly precedes every body blend write.
        let mut u0 = d.from;
        while u0 <= d.to {
            let uw = ((d.to - u0 + 1) as usize).min(TILE_U);
            tile.reset(u0, uw, dv0, vh, v_c, ht);
            paint_kind(item.kind, &mut tile, &ctx);
            tile.flush_shadow(pb, item.horizontal, d.shadow_cross, max_shadow);
            u0 += uw as i32;
        }
        if item.kind != ForeKind::CloudBand {
            let mut u0 = d.from;
            while u0 <= d.to {
                let uw = ((d.to - u0 + 1) as usize).min(TILE_U);
                tile.reset(u0, uw, dv0, vh, v_c, ht);
                paint_kind(item.kind, &mut tile, &ctx);
                tile.flush_body(pb, item.horizontal, d.screen_cross, body_alpha);
                u0 += uw as i32;
            }
        }
    }
}

// ---- Shared palette & math helpers -----------------------------------------

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
    bark_rim: Rgb,
}

impl ForePal {
    /// Mid-dark per-floor body palette clearly lighter than the floor, with a
    /// darker underside (`deep`), lit face (`soft`), and ambient-tinted rim
    /// (`rim`) placed on the `toward_centre` side.
    fn for_theme(theme: Theme) -> Self {
        let rt = rim_tint(theme);
        let peak = rt.0.max(rt.1).max(rt.2).max(1) as f32;
        let gain = (195.0 / peak).clamp(1.0, 2.6);
        let tint = palette::scale(rt, gain);

        let build = |body: Rgb, accent: Rgb, pale: Rgb, bark: Rgb| -> Self {
            let rim = palette::mix(palette::scale(body, RIM_LIGHTEN), tint, RIM_TINT_MIX);
            let bark_rim = palette::mix(palette::scale(bark, RIM_LIGHTEN), tint, RIM_TINT_MIX);
            Self {
                deep: palette::scale(body, DEEP_SHADE),
                mid: body,
                soft: palette::scale(body, SOFT_LIFT),
                accent,
                pale,
                rim,
                bark_deep: palette::scale(bark, DEEP_SHADE),
                bark_mid: bark,
                bark_rim,
            }
        };

        match theme {
            // Wet iron & rust.
            Theme::Sewer => build((64, 82, 78), (156, 86, 48), (168, 190, 182), (64, 82, 78)),
            // Warm brushed steel & copper.
            Theme::Kitchen => build(
                (106, 92, 78),
                (192, 114, 58),
                (236, 202, 134),
                (106, 92, 78),
            ),
            // Cold steel-blue, clinical cyan & amber cable/hazard accent.
            Theme::Lab => build(
                (74, 92, 118),
                (82, 174, 204),
                (178, 208, 224),
                (182, 138, 64),
            ),
            // Pale enamel, warm wood/brass, linen white & wash-pink accent.
            Theme::Laundry => build(
                (120, 128, 144),
                (166, 126, 84),
                (210, 216, 224),
                (198, 146, 162),
            ),
            // Oiled steel, registration cyan, cream newsprint & charcoal ink.
            Theme::Printworks => build(
                (92, 82, 90),
                (74, 162, 172),
                (202, 198, 184),
                (54, 48, 56),
            ),
            // Outdoors: two leaf greens + sunlit tips, high-contrast grey-brown bark.
            Theme::Trailer => {
                let leaf_deep: Rgb = (34, 72, 38);
                let leaf: Rgb = (66, 120, 56);
                Self {
                    deep: leaf_deep,
                    mid: leaf,
                    soft: palette::scale(leaf, SOFT_LIFT),
                    accent: (114, 162, 72),
                    pale: (162, 176, 194),
                    rim: palette::mix(palette::scale(leaf, RIM_LIGHTEN), tint, RIM_TINT_MIX),
                    bark_deep: (46, 34, 24),
                    bark_mid: (96, 76, 56),
                    bark_rim: (138, 118, 92),
                }
            }
        }
    }
}

struct BandCtx {
    s: f32,
    ht: f32,
    along: f32,
    sway: f32,
    seed: u32,
    pal: ForePal,
    tc: f32,
}

#[inline]
fn pos_mod(x: f32, m: f32) -> f32 {
    x - (x / m).floor() * m
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
fn soft_u8(dist_in: f32, feather: f32, max_w: u8) -> u8 {
    if dist_in <= 0.0 {
        0
    } else if feather <= 0.0 || dist_in >= feather {
        max_w
    } else {
        ((dist_in / feather) * max_w as f32 + 0.5) as u8
    }
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn mul_alpha(base: u8, weight: u8) -> u8 {
    ((base as u16 * weight as u16 + 127) / 255) as u8
}

// ---- Stack-allocated tile rasterizer ---------------------------------------

struct BandTile {
    u0: i32,
    uw: usize,
    dv0: i32,
    vh: usize,
    v_c: f32,
    ht: f32,
    shd: [u8; TILE_CELLS],
    alp: [u8; TILE_CELLS],
    rgb: [Rgb; TILE_CELLS],
}

impl BandTile {
    fn new(u0: i32, uw: usize, dv0: i32, vh: usize, v_c: f32, ht: f32) -> Self {
        Self {
            u0,
            uw: uw.min(TILE_U),
            dv0,
            vh: vh.min(MAX_V),
            v_c,
            ht,
            shd: [0; TILE_CELLS],
            alp: [0; TILE_CELLS],
            rgb: [(0, 0, 0); TILE_CELLS],
        }
    }

    fn reset(&mut self, u0: i32, uw: usize, dv0: i32, vh: usize, v_c: f32, ht: f32) {
        self.u0 = u0;
        self.uw = uw.min(TILE_U);
        self.dv0 = dv0;
        self.vh = vh.min(MAX_V);
        self.v_c = v_c;
        self.ht = ht;
        let used = self.uw * self.vh;
        self.shd[..used].fill(0);
        self.alp[..used].fill(0);
    }

    #[inline]
    fn v_at(&self, iv: usize) -> f32 {
        (self.dv0 + iv as i32) as f32 - self.v_c
    }

    #[inline]
    fn wu_at(&self, iu: usize, along: f32) -> f32 {
        (self.u0 + iu as i32) as f32 + along
    }

    #[inline]
    fn sh_px(&mut self, iu: usize, iv: usize, w: u8) {
        if w == 0 || iu >= self.uw || iv >= self.vh {
            return;
        }
        if self.v_at(iv).abs() >= self.ht {
            return;
        }
        let idx = iv * self.uw + iu;
        if w > self.shd[idx] {
            self.shd[idx] = w;
        }
    }

    #[inline]
    fn bd_px(&mut self, iu: usize, iv: usize, col: Rgb, w: u8) {
        if w == 0 || iu >= self.uw || iv >= self.vh {
            return;
        }
        if self.v_at(iv).abs() >= self.ht {
            return;
        }
        let idx = iv * self.uw + iu;
        let old = self.alp[idx];
        if old == 0 || w >= 248 {
            self.rgb[idx] = col;
            self.alp[idx] = old.max(w);
        } else {
            self.rgb[idx] = palette::mix(self.rgb[idx], col, w as f32 * (1.0 / 255.0));
            self.alp[idx] = old.max(w);
        }
    }

    fn flush_shadow(&self, pb: &mut PixelBuffer, horizontal: bool, shadow_cross: i32, max_sh: f32) {
        if max_sh <= 0.0 {
            return;
        }
        let inv255 = max_sh * (1.0 / 255.0);
        for iv in 0..self.vh {
            let cross = shadow_cross + self.dv0 + iv as i32;
            let row = iv * self.uw;
            for iu in 0..self.uw {
                let sv = self.shd[row + iu];
                if sv == 0 {
                    continue;
                }
                let u = self.u0 + iu as i32;
                let (x, y) = if horizontal { (u, cross) } else { (cross, u) };
                shadow(pb, x, y, sv as f32 * inv255);
            }
        }
    }

    fn flush_body(&self, pb: &mut PixelBuffer, horizontal: bool, screen_cross: i32, alpha: u8) {
        if alpha == 0 {
            return;
        }
        for iv in 0..self.vh {
            let cross = screen_cross + self.dv0 + iv as i32;
            let row = iv * self.uw;
            for iu in 0..self.uw {
                let av = self.alp[row + iu];
                if av == 0 {
                    continue;
                }
                let a = mul_alpha(alpha, av);
                if a == 0 {
                    continue;
                }
                let u = self.u0 + iu as i32;
                let (x, y) = if horizontal { (u, cross) } else { (cross, u) };
                blend(pb, x, y, self.rgb[row + iu], a);
            }
        }
    }

    #[inline]
    fn u_range(&self, wu0: f32, wu1: f32, along: f32) -> Option<(usize, usize)> {
        let lo = ((wu0 - along).floor() as i32 - self.u0).max(0);
        let hi = ((wu1 - along).ceil() as i32 - self.u0).min(self.uw as i32 - 1);
        if lo > hi {
            None
        } else {
            Some((lo as usize, hi as usize))
        }
    }

    #[inline]
    fn v_range(&self, v0: f32, v1: f32) -> Option<(usize, usize)> {
        let lo = ((v0 + self.v_c).floor() as i32 - self.dv0).max(0);
        let hi = ((v1 + self.v_c).ceil() as i32 - self.dv0).min(self.vh as i32 - 1);
        if lo > hi {
            None
        } else {
            Some((lo as usize, hi as usize))
        }
    }

    fn fill_rect_sh(&mut self, r: [f32; 4], along: f32, w: u8, feather: f32) {
        let [wu0, v0, wu1, v1] = r;
        let Some((u_lo, u_hi)) = self.u_range(wu0, wu1, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(v0, v1) else {
            return;
        };
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            let dv_in = (v - v0).min(v1 - v);
            if dv_in <= 0.0 {
                continue;
            }
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let d_in = (wu - wu0).min(wu1 - wu).min(dv_in);
                if d_in > 0.0 {
                    self.sh_px(iu, iv, soft_u8(d_in, feather, w));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_rect_bd(
        &mut self,
        r: [f32; 4],
        along: f32,
        rgb: Rgb,
        w: u8,
        feather: f32,
        rim: Option<Rgb>,
        deep: Option<Rgb>,
        tc: f32,
        rim_w: f32,
    ) {
        let [wu0, v0, wu1, v1] = r;
        let Some((u_lo, u_hi)) = self.u_range(wu0, wu1, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(v0, v1) else {
            return;
        };
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            let dv_lo = v - v0;
            let dv_hi = v1 - v;
            let dv_in = dv_lo.min(dv_hi);
            if dv_in <= 0.0 {
                continue;
            }
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let d_in = (wu - wu0).min(wu1 - wu).min(dv_in);
                if d_in <= 0.0 {
                    continue;
                }
                let aw = soft_u8(d_in, feather, w);
                if aw == 0 {
                    continue;
                }
                let mut col = rgb;
                if let Some(rc) = rim {
                    let d_rim = if tc > 0.0 { dv_hi } else { dv_lo };
                    if d_rim < rim_w {
                        col = rc;
                    } else if let Some(dc) = deep {
                        let d_far = if tc > 0.0 { dv_lo } else { dv_hi };
                        if d_far < rim_w * 1.1 {
                            col = dc;
                        }
                    }
                }
                self.bd_px(iu, iv, col, aw);
            }
        }
    }

    fn fill_ellipse_sh(&mut self, e: [f32; 4], along: f32, w: u8, feather: f32) {
        let [cw, cv, rx, ry] = e;
        if rx <= 0.1 || ry <= 0.1 {
            return;
        }
        let Some((u_lo, u_hi)) = self.u_range(cw - rx, cw + rx, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(cv - ry, cv + ry) else {
            return;
        };
        let r_eff = rx.min(ry);
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            let dy = (v - cv) / ry;
            let dy2 = dy * dy;
            if dy2 >= 1.0 {
                continue;
            }
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let dx = (wu - cw) / rx;
                let q = dx * dx + dy2;
                if q >= 1.0 {
                    continue;
                }
                let d_in = (1.0 - q.sqrt()) * r_eff;
                self.sh_px(iu, iv, soft_u8(d_in, feather, w));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_ellipse_bd(
        &mut self,
        e: [f32; 4],
        along: f32,
        dark: Rgb,
        mid: Rgb,
        rim: Option<Rgb>,
        tc: f32,
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        let [cw, cv, rx, ry] = e;
        if rx <= 0.1 || ry <= 0.1 {
            return;
        }
        let Some((u_lo, u_hi)) = self.u_range(cw - rx, cw + rx, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(cv - ry, cv + ry) else {
            return;
        };
        let r_eff = rx.min(ry);
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            let dy = (v - cv) / ry;
            let dy2 = dy * dy;
            if dy2 >= 1.0 {
                continue;
            }
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let dx = (wu - cw) / rx;
                let q = dx * dx + dy2;
                if q >= 1.0 {
                    continue;
                }
                let d_in = (1.0 - q.sqrt()) * r_eff;
                let aw = soft_u8(d_in, feather, w);
                if aw == 0 {
                    continue;
                }
                let side = dy * tc;
                let col = if let Some(rc) = rim {
                    if d_in < rim_w && side > 0.12 {
                        rc
                    } else if side < -0.25 {
                        dark
                    } else {
                        palette::mix(dark, mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                    }
                } else if side < -0.25 {
                    dark
                } else {
                    palette::mix(dark, mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                };
                self.bd_px(iu, iv, col, aw);
            }
        }
    }

    fn fill_capsule_sh(&mut self, cap: [f32; 6], along: f32, w: u8, feather: f32) {
        let [au, av, ra, bu, bv, rb] = cap;
        let rmax = ra.max(rb);
        let Some((u_lo, u_hi)) = self.u_range(au.min(bu) - rmax, au.max(bu) + rmax, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(av.min(bv) - rmax, av.max(bv) + rmax) else {
            return;
        };
        let dx = bu - au;
        let dy = bv - av;
        let l2 = dx * dx + dy * dy;
        let inv_l2 = if l2 > 1e-5 { 1.0 / l2 } else { 0.0 };
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let t = (((wu - au) * dx + (v - av) * dy) * inv_l2).clamp(0.0, 1.0);
                let px = au + dx * t;
                let py = av + dy * t;
                let r = ra + (rb - ra) * t;
                let d_in = r - ((wu - px) * (wu - px) + (v - py) * (v - py)).sqrt();
                if d_in > 0.0 {
                    self.sh_px(iu, iv, soft_u8(d_in, feather, w));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_capsule_bd(
        &mut self,
        cap: [f32; 6],
        along: f32,
        dark: Rgb,
        mid: Rgb,
        rim: Option<Rgb>,
        tc: f32,
        w: u8,
        feather: f32,
        rim_w: f32,
    ) {
        let [au, av, ra, bu, bv, rb] = cap;
        let rmax = ra.max(rb);
        let Some((u_lo, u_hi)) = self.u_range(au.min(bu) - rmax, au.max(bu) + rmax, along) else {
            return;
        };
        let Some((v_lo, v_hi)) = self.v_range(av.min(bv) - rmax, av.max(bv) + rmax) else {
            return;
        };
        let dx = bu - au;
        let dy = bv - av;
        let l2 = dx * dx + dy * dy;
        let inv_l2 = if l2 > 1e-5 { 1.0 / l2 } else { 0.0 };
        for iv in v_lo..=v_hi {
            let v = self.v_at(iv);
            for iu in u_lo..=u_hi {
                let wu = self.wu_at(iu, along);
                let t = (((wu - au) * dx + (v - av) * dy) * inv_l2).clamp(0.0, 1.0);
                let px = au + dx * t;
                let py = av + dy * t;
                let r = ra + (rb - ra) * t;
                let d_in = r - ((wu - px) * (wu - px) + (v - py) * (v - py)).sqrt();
                if d_in <= 0.0 {
                    continue;
                }
                let aw = soft_u8(d_in, feather, w);
                if aw == 0 {
                    continue;
                }
                let side = ((v - py) / r.max(0.5)) * tc;
                let col = if let Some(rc) = rim {
                    if d_in < rim_w && side > 0.1 {
                        rc
                    } else {
                        palette::mix(dark, mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                    }
                } else {
                    palette::mix(dark, mid, (0.5 + 0.5 * side).clamp(0.0, 1.0))
                };
                self.bd_px(iu, iv, col, aw);
            }
        }
    }
}

// ---- Dispatch to the 18 band painters --------------------------------------

fn paint_kind(kind: ForeKind, tile: &mut BandTile, c: &BandCtx) {
    match kind {
        // Sewer
        ForeKind::MainPipe => paint_main_pipe(tile, c),
        ForeKind::CatwalkGrating => paint_catwalk_grating(tile, c),
        ForeKind::ChainRun => paint_chain_run(tile, c),
        // Kitchen
        ForeKind::ExtractorDuct => paint_extractor_duct(tile, c),
        ForeKind::PotRail => paint_pot_rail(tile, c),
        ForeKind::CableRun => paint_cable_run(tile, c),
        // Lab
        ForeKind::CableTray => paint_cable_tray(tile, c),
        ForeKind::VentDuct => paint_vent_duct(tile, c),
        ForeKind::Gantry => paint_gantry(tile, c),
        // Laundry
        ForeKind::Clothesline => paint_clothesline(tile, c),
        ForeKind::SteamPipe => paint_steam_pipe(tile, c),
        ForeKind::RackBeam => paint_rack_beam(tile, c),
        // Printworks
        ForeKind::PaperWeb => paint_paper_web(tile, c),
        ForeKind::RollerBeam => paint_roller_beam(tile, c),
        ForeKind::ProofLine => paint_proof_line(tile, c),
        // Flatbed (Outdoors)
        ForeKind::BranchCanopy => paint_branch_canopy(tile, c),
        ForeKind::PowerLines => paint_power_lines(tile, c),
        ForeKind::CloudBand => paint_cloud_band(tile, c),
    }
}

// ---- Floor 1: Sewer --------------------------------------------------------

/// 1. Fat riveted sewer main with bolted flanges every ~38*s art px and an
///    intermittent spoked rust valve wheel.
fn paint_main_pipe(tile: &mut BandTile, c: &BandCtx) {
    let sr = c.ht * 0.78;
    let fr = c.ht;
    let pitch = 38.0 * c.s;
    let flange_hw = 2.6 * c.s;
    let bolt_step = (fr * 0.48).max(2.8);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / sr.max(1.0)) * c.tc;
        let nf = (v / fr.max(1.0)) * c.tc;
        let bv = (pos_mod(v + bolt_step * 0.5, bolt_step) - bolt_step * 0.5).abs();
        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let lu = pos_mod(wu, pitch);
            let du_f = lu.min(pitch - lu);
            let on_flange = du_f < flange_hw;
            let r_here = if on_flange { fr } else { sr };
            let d_in = r_here - av;
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(iu, iv, soft_u8(d_in, 2.2, if on_flange { 235 } else { 215 }));
            let aw = soft_u8(d_in, 1.8, 255);
            let col = if on_flange {
                if du_f < 0.75 {
                    c.pal.deep
                } else if du_f > 1.0 && du_f < flange_hw - 0.5 && bv < 0.85 && av < fr - 1.2 {
                    if nf > -0.15 {
                        c.pal.pale
                    } else {
                        c.pal.accent
                    }
                } else if d_in < 1.5 && nf > 0.2 {
                    c.pal.rim
                } else if nf > 0.35 {
                    c.pal.soft
                } else if nf < -0.30 {
                    c.pal.deep
                } else {
                    c.pal.mid
                }
            } else {
                let mut col = if d_in < 1.6 && nv > 0.25 {
                    c.pal.rim
                } else if nv > 0.52 {
                    palette::mix(c.pal.soft, c.pal.rim, 0.55)
                } else if nv > 0.18 {
                    c.pal.soft
                } else if nv > -0.22 {
                    c.pal.mid
                } else {
                    c.pal.deep
                };
                if (nv - 0.45).abs() < 0.14 {
                    col = palette::mix(col, c.pal.pale, 0.45);
                }
                let wui = wu.floor() as i32;
                let h = hash_slot((wu * 0.35).floor() as i32, c.seed ^ 0x511);
                if av < 0.8 && wui.rem_euclid(7) == 0 {
                    col = if nv > -0.2 { c.pal.pale } else { c.pal.deep };
                } else if (h & 15) == 0 && nv < 0.15 && nv.abs() < 0.72 {
                    col = palette::mix(col, c.pal.accent, 0.38);
                }
                col
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }

    // Crisp spoked iron/rust valve wheel on every second span.
    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let slot0 = ((wu_min - 12.0 * c.s) / pitch).floor() as i32;
    let slot1 = ((wu_max + 12.0 * c.s) / pitch).ceil() as i32;
    for slot in slot0..=slot1 {
        if (hash_slot(slot, c.seed ^ 0xA1E) & 1) != 0 {
            continue;
        }
        let cu = (slot as f32 + 0.5) * pitch;
        let vr = (c.ht * 0.64).min(4.4 * c.s);
        let Some((u_lo, u_hi)) = tile.u_range(cu - vr - 1.0, cu + vr + 1.0, c.along) else {
            continue;
        };
        let Some((v_lo, v_hi)) = tile.v_range(-vr - 1.0, vr + 1.0) else {
            continue;
        };
        for iv in v_lo..=v_hi {
            let v = tile.v_at(iv);
            for iu in u_lo..=u_hi {
                let du = tile.wu_at(iu, c.along) - cu;
                let r = (du * du + v * v).sqrt();
                let on_ring = (vr * 0.58..=vr).contains(&r);
                let on_spoke = r < vr * 0.68 && (du.abs() < 0.85 || v.abs() < 0.85);
                let on_hub = r < 1.35 * c.s;
                if !(on_ring || on_spoke || on_hub) {
                    continue;
                }
                let col = if on_hub {
                    c.pal.pale
                } else if on_ring {
                    let side = (v / vr.max(1.0)) * c.tc;
                    if side > 0.35 {
                        palette::mix(c.pal.accent, c.pal.pale, 0.45)
                    } else if side > -0.25 {
                        c.pal.accent
                    } else {
                        palette::scale(c.pal.accent, 0.68)
                    }
                } else {
                    palette::scale(c.pal.accent, 0.85)
                };
                tile.bd_px(iu, iv, col, 250);
            }
        }
    }
}

/// 2. Open industrial steel catwalk grating with side handrails, structural
///    ribs, and real see-through rectangular holes.
fn paint_catwalk_grating(tile: &mut BandTile, c: &BandCtx) {
    let rail_w = (1.8 * c.s).max(1.8);
    let bar_pitch = 4.6 * c.s;
    let bar_hw = (0.65 * c.s).max(0.65);
    let rib_pitch = 23.0 * c.s;
    let rib_hw = (1.3 * c.s).max(1.3);
    let mid_hw = (0.65 * c.s).max(0.7);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / c.ht.max(1.0)) * c.tc;
        let in_rail = av >= c.ht - rail_w;
        let in_mid = av <= mid_hw;
        let d_outer = c.ht - av;
        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let r_lu = pos_mod(wu, rib_pitch);
            let du_rib = r_lu.min(rib_pitch - r_lu);
            let in_rib = du_rib < rib_hw;

            let b_lu = pos_mod(wu, bar_pitch);
            let du_bar = b_lu.min(bar_pitch - b_lu);
            let in_bar = du_bar < bar_hw;

            if !(in_rail || in_mid || in_rib || in_bar) {
                continue;
            }
            let (sh_w, aw, col) = if in_rail {
                let d_edge = d_outer.min(av - (c.ht - rail_w) + 0.4);
                let mut col = if d_outer < 1.3 && nv > 0.0 {
                    c.pal.rim
                } else if nv > 0.0 {
                    c.pal.soft
                } else {
                    c.pal.deep
                };
                if du_rib < rib_hw * 1.1 {
                    col = if nv > 0.0 {
                        c.pal.pale
                    } else {
                        palette::mix(c.pal.mid, c.pal.accent, 0.45)
                    };
                }
                (soft_u8(d_edge, 1.6, 225), soft_u8(d_edge, 1.5, 255), col)
            } else if in_rib {
                let d_edge = d_outer.min(rib_hw - du_rib + 0.4);
                let mut col = if nv > 0.1 {
                    c.pal.soft
                } else if nv < -0.2 {
                    c.pal.deep
                } else {
                    c.pal.mid
                };
                if du_rib < 0.6 && (av - c.ht * 0.5).abs() < 1.0 {
                    col = if nv > 0.0 { c.pal.pale } else { c.pal.accent };
                }
                (soft_u8(d_edge, 1.4, 218), soft_u8(d_edge, 1.2, 250), col)
            } else if in_mid {
                let d_edge = mid_hw - av + 0.4;
                let col = if nv >= 0.0 { c.pal.mid } else { c.pal.deep };
                (soft_u8(d_edge, 1.1, 185), soft_u8(d_edge, 1.0, 232), col)
            } else {
                let d_edge = bar_hw - du_bar + 0.35;
                let col = if du_bar < 0.35 && nv > -0.2 {
                    c.pal.soft
                } else {
                    c.pal.mid
                };
                (soft_u8(d_edge, 1.0, 175), soft_u8(d_edge, 1.0, 228), col)
            };
            tile.sh_px(iu, iv, sh_w);
            tile.bd_px(iu, iv, col, aw);
        }
    }
}

/// 3. Sagging heavy interlocking chain with periodic hoist block & J-hook.
fn paint_chain_run(tile: &mut BandTile, c: &BandCtx) {
    let hoist_pitch = 42.0 * c.s;
    let link_step = 5.4 * c.s;
    let sw_u = c.sway * 2.0 * c.s;

    let chain_v = |wu: f32| -> f32 {
        let u_rel = pos_mod(wu, hoist_pitch) / hoist_pitch;
        let sag = 4.0 * u_rel * (1.0 - u_rel);
        (-c.ht + 2.6 * c.s) + sag * (3.2 * c.s)
    };

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;

    // Interlocking oval and edge-on chain links along the catenary.
    let slot0 = ((wu_min - link_step) / link_step).floor() as i32;
    let slot1 = ((wu_max + link_step) / link_step).ceil() as i32;
    for slot in slot0..=slot1 {
        let cu = (slot as f32 + 0.5) * link_step;
        let cv = chain_v(cu);
        if (slot & 1) == 0 {
            let rx = link_step * 0.62;
            let ry = 2.5 * c.s;
            let Some((u_lo, u_hi)) = tile.u_range(cu - rx, cu + rx, c.along) else {
                continue;
            };
            let Some((v_lo, v_hi)) = tile.v_range(cv - ry, cv + ry) else {
                continue;
            };
            for iv in v_lo..=v_hi {
                let v = tile.v_at(iv);
                let dy = (v - cv) / ry;
                let dy2 = dy * dy;
                for iu in u_lo..=u_hi {
                    let dx = (tile.wu_at(iu, c.along) - cu) / rx;
                    let q = (dx * dx + dy2).sqrt();
                    if !(0.40..1.0).contains(&q) {
                        continue;
                    }
                    let d_in = (1.0 - q).min(q - 0.40) * ry;
                    tile.sh_px(iu, iv, soft_u8(d_in, 1.3, 215));
                    let aw = soft_u8(d_in, 0.9, 255);
                    let side = dy * c.tc;
                    let col = if side > 0.35 {
                        c.pal.pale
                    } else if side > 0.0 {
                        c.pal.rim
                    } else if side > -0.40 {
                        c.pal.soft
                    } else {
                        c.pal.deep
                    };
                    tile.bd_px(iu, iv, col, aw);
                }
            }
        } else {
            let hl = link_step * 0.48;
            let hr = 1.15 * c.s;
            tile.fill_capsule_sh([cu - hl, cv, hr, cu + hl, cv, hr], c.along, 210, 1.4);
            tile.fill_capsule_bd(
                [cu - hl, cv, hr, cu + hl, cv, hr],
                c.along,
                c.pal.mid,
                c.pal.pale,
                Some(c.pal.rim),
                c.tc,
                255,
                0.9,
                0.8,
            );
        }
    }

    // Periodic hoist sheave block & curved steel J-hook reaching full +-ht.
    let h0 = ((wu_min - 14.0 * c.s) / hoist_pitch).floor() as i32;
    let h1 = ((wu_max + 14.0 * c.s) / hoist_pitch).ceil() as i32;
    for hs in h0..=h1 {
        let cu = hs as f32 * hoist_pitch;
        let top_v = -c.ht + 0.4;
        tile.fill_rect_sh(
            [cu - 2.2 * c.s, top_v, cu + 2.2 * c.s, top_v + 3.2 * c.s],
            c.along,
            230,
            1.6,
        );
        tile.fill_rect_bd(
            [cu - 2.2 * c.s, top_v, cu + 2.2 * c.s, top_v + 3.0 * c.s],
            c.along,
            c.pal.soft,
            255,
            1.0,
            Some(c.pal.pale),
            Some(c.pal.deep),
            c.tc,
            1.0,
        );

        let blk_u = cu + sw_u * 0.6;
        let blk_v = 0.6 * c.s;
        let drop = [cu, top_v + 2.5 * c.s, 1.1 * c.s, blk_u, blk_v - 3.0 * c.s, 1.1 * c.s];
        tile.fill_capsule_sh(drop, c.along, 215, 1.4);
        tile.fill_capsule_bd(
            drop,
            c.along,
            c.pal.mid,
            c.pal.pale,
            Some(c.pal.rim),
            c.tc,
            255,
            0.9,
            0.8,
        );

        let bw = 3.9 * c.s;
        let bh = 3.6 * c.s;
        tile.fill_rect_sh(
            [blk_u - bw, blk_v - bh, blk_u + bw, blk_v + bh],
            c.along,
            235,
            1.8,
        );
        tile.fill_rect_bd(
            [blk_u - bw, blk_v - bh, blk_u + bw, blk_v + bh],
            c.along,
            c.pal.soft,
            255,
            1.2,
            Some(c.pal.pale),
            Some(c.pal.deep),
            c.tc,
            1.2,
        );
        tile.fill_ellipse_bd(
            [blk_u, blk_v, 2.1 * c.s, 2.1 * c.s],
            c.along,
            c.pal.deep,
            c.pal.accent,
            Some(c.pal.pale),
            c.tc,
            255,
            0.9,
            0.8,
        );

        let hook_u = cu + sw_u;
        let hook_top = blk_v + bh - 0.5;
        let hook_bot = c.ht - 0.8;
        let shank = [blk_u, hook_top, 1.15 * c.s, hook_u, hook_bot - 1.4 * c.s, 1.25 * c.s];
        let bend = [
            hook_u,
            hook_bot - 1.2 * c.s,
            1.2 * c.s,
            hook_u + 3.0 * c.s,
            hook_bot - 0.1,
            1.15 * c.s,
        ];
        let tip = [
            hook_u + 3.0 * c.s,
            hook_bot - 0.1,
            1.05 * c.s,
            hook_u + 3.6 * c.s,
            hook_bot - 2.6 * c.s,
            0.9 * c.s,
        ];
        tile.fill_capsule_sh(shank, c.along, 220, 1.5);
        tile.fill_capsule_sh(bend, c.along, 220, 1.5);
        tile.fill_capsule_sh(tip, c.along, 210, 1.4);
        tile.fill_capsule_bd(
            shank,
            c.along,
            c.pal.mid,
            c.pal.pale,
            Some(c.pal.rim),
            c.tc,
            255,
            0.9,
            0.9,
        );
        tile.fill_capsule_bd(
            bend,
            c.along,
            c.pal.soft,
            c.pal.pale,
            Some(c.pal.rim),
            c.tc,
            255,
            0.9,
            0.9,
        );
        tile.fill_capsule_bd(
            tip,
            c.along,
            c.pal.soft,
            c.pal.pale,
            Some(c.pal.rim),
            c.tc,
            255,
            0.8,
            0.8,
        );
    }
}

// ---- Floor 2: Kitchen ------------------------------------------------------

/// 4. Wide brushed-stainless kitchen extractor canopy duct with standing seams
///    and recessed metallic grease-filter cassettes.
fn paint_extractor_duct(tile: &mut BandTile, c: &BandCtx) {
    let seam_pitch = 24.0 * c.s;
    let seam_hw = 1.7 * c.s;
    let lip_w = 3.2 * c.s;
    let slat_step = 2.6 * c.s;

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / c.ht.max(1.0)) * c.tc;
        let in_lip = av >= c.ht - lip_w;
        let d_in = c.ht - av;
        let rivet_hit = (pos_mod(v + 2.5, 4.8) - 2.4).abs() < 0.65 && av < c.ht - 1.5;

        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let lu = pos_mod(wu, seam_pitch);
            let du_seam = lu.min(seam_pitch - lu);
            let on_seam = du_seam < seam_hw;

            tile.sh_px(iu, iv, soft_u8(d_in, 2.2, 235));
            let aw = soft_u8(d_in, 1.8, 255);
            let col = if on_seam {
                if du_seam < 0.55 {
                    c.pal.deep
                } else if rivet_hit {
                    c.pal.pale
                } else if d_in < 1.6 && nv > 0.0 {
                    c.pal.rim
                } else if nv > -0.1 {
                    c.pal.soft
                } else {
                    c.pal.mid
                }
            } else if in_lip {
                if (av - (c.ht - lip_w)).abs() < 0.85 {
                    if nv > 0.0 {
                        c.pal.deep
                    } else {
                        c.pal.rim
                    }
                } else if d_in < 1.6 && nv > 0.0 {
                    c.pal.rim
                } else if nv > 0.35 {
                    palette::mix(c.pal.soft, c.pal.rim, 0.45)
                } else if nv < -0.35 {
                    c.pal.deep
                } else {
                    c.pal.mid
                }
            } else if av < 0.8 {
                if c.tc > 0.0 {
                    c.pal.soft
                } else {
                    c.pal.deep
                }
            } else if du_seam < seam_hw + 1.0 {
                c.pal.deep
            } else {
                let slat_phase = pos_mod(lu - seam_hw, slat_step) / slat_step;
                if slat_phase < 0.28 {
                    palette::mix(c.pal.mid, c.pal.deep, 0.65)
                } else if slat_phase < 0.66 {
                    if nv < 0.1 {
                        c.pal.mid
                    } else {
                        c.pal.soft
                    }
                } else {
                    palette::mix(c.pal.soft, c.pal.pale, 0.25)
                }
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }
}

/// 5. Wrought-iron kitchen pot rail with swinging cast-iron skillets, twin-handle
///    copper stockpots, and ladles separated by real open gaps.
fn paint_pot_rail(tile: &mut BandTile, c: &BandCtx) {
    let rail_v = -c.ht + 2.0 * c.s;
    let rail_hr = 1.45 * c.s;
    let item_pitch = 14.5 * c.s;
    let brkt_pitch = 30.0 * c.s;

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;

    tile.fill_rect_sh(
        [wu_min - 2.0, rail_v - rail_hr, wu_max + 2.0, rail_v + rail_hr],
        c.along,
        225,
        1.8,
    );
    tile.fill_rect_bd(
        [wu_min - 2.0, rail_v - rail_hr, wu_max + 2.0, rail_v + rail_hr],
        c.along,
        c.pal.mid,
        255,
        1.2,
        Some(c.pal.rim),
        Some(c.pal.deep),
        c.tc,
        1.0,
    );

    let b0 = ((wu_min - 6.0 * c.s) / brkt_pitch).floor() as i32;
    let b1 = ((wu_max + 6.0 * c.s) / brkt_pitch).ceil() as i32;
    for bs in b0..=b1 {
        let bu = bs as f32 * brkt_pitch;
        let br = [bu - 1.5 * c.s, -c.ht, bu + 1.5 * c.s, rail_v + rail_hr + 0.8];
        tile.fill_rect_sh(br, c.along, 230, 1.4);
        tile.fill_rect_bd(
            br,
            c.along,
            c.pal.deep,
            255,
            0.9,
            Some(c.pal.rim),
            None,
            c.tc,
            0.9,
        );
    }

    let s0 = ((wu_min - 12.0 * c.s) / item_pitch).floor() as i32;
    let s1 = ((wu_max + 12.0 * c.s) / item_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let h = hash_slot(slot, c.seed ^ 0x907);
        let jitter = (((h >> 4) & 7) as f32 - 3.5) * 0.30 * c.s;
        let cu = (slot as f32 + 0.5) * item_pitch + jitter;
        let sw = c.sway * 1.8 * c.s * if (h & 8) == 0 { 1.0 } else { -0.75 };
        let kind_i = (slot + (c.seed & 3) as i32).rem_euclid(3);

        let hook_bot = rail_v + 2.6 * c.s;
        let hook_cap = [cu, rail_v, 0.75 * c.s, cu + sw * 0.25, hook_bot, 0.75 * c.s];
        tile.fill_capsule_sh(hook_cap, c.along, 200, 1.2);
        tile.fill_capsule_bd(
            [cu, rail_v, 0.7 * c.s, cu + sw * 0.25, hook_bot, 0.7 * c.s],
            c.along,
            c.pal.deep,
            c.pal.pale,
            Some(c.pal.rim),
            c.tc,
            255,
            0.8,
            0.7,
        );

        match kind_i {
            0 => {
                // Cast-iron skillet with copper base sheen.
                let pan_r = (4.3 * c.s).min((c.ht - hook_bot) * 0.45);
                let pan_v = c.ht - pan_r - 0.15;
                let pan_u = cu + sw;
                let hdl = [
                    cu + sw * 0.25,
                    hook_bot,
                    0.95 * c.s,
                    pan_u,
                    pan_v - pan_r + 1.0,
                    1.15 * c.s,
                ];
                tile.fill_capsule_sh(hdl, c.along, 215, 1.4);
                tile.fill_capsule_bd(
                    hdl,
                    c.along,
                    c.pal.deep,
                    c.pal.mid,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    1.0,
                    0.8,
                );
                tile.fill_ellipse_sh([pan_u, pan_v, pan_r, pan_r], c.along, 235, 1.8);
                tile.fill_ellipse_bd(
                    [pan_u, pan_v, pan_r, pan_r],
                    c.along,
                    c.pal.deep,
                    c.pal.mid,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    1.3,
                    1.2,
                );
                tile.fill_ellipse_bd(
                    [pan_u, pan_v, pan_r * 0.64, pan_r * 0.64],
                    c.along,
                    c.pal.deep,
                    c.pal.accent,
                    Some(c.pal.pale),
                    c.tc,
                    240,
                    1.0,
                    0.9,
                );
            }
            1 => {
                // Copper stockpot with twin ear handles & vertical metallic sheen.
                let pw = 3.5 * c.s;
                let ptop = hook_bot + 1.2 * c.s;
                let pbot = c.ht - 0.25;
                let pu = cu + sw * 0.85;
                tile.fill_capsule_bd(
                    [cu + sw * 0.25, hook_bot, 0.7 * c.s, pu - pw * 0.65, ptop + 0.8, 0.65 * c.s],
                    c.along,
                    c.pal.deep,
                    c.pal.pale,
                    None,
                    c.tc,
                    240,
                    0.8,
                    0.0,
                );
                tile.fill_capsule_bd(
                    [cu + sw * 0.25, hook_bot, 0.7 * c.s, pu + pw * 0.65, ptop + 0.8, 0.65 * c.s],
                    c.along,
                    c.pal.deep,
                    c.pal.pale,
                    None,
                    c.tc,
                    240,
                    0.8,
                    0.0,
                );
                tile.fill_rect_bd(
                    [pu - pw - 1.5 * c.s, ptop + 1.3 * c.s, pu + pw + 1.5 * c.s, ptop + 2.8 * c.s],
                    c.along,
                    c.pal.mid,
                    255,
                    0.8,
                    Some(c.pal.rim),
                    None,
                    c.tc,
                    0.8,
                );
                tile.fill_rect_sh([pu - pw, ptop, pu + pw, pbot - 1.0], c.along, 235, 1.8);
                tile.fill_ellipse_sh([pu, pbot - 1.4, pw, 1.5], c.along, 230, 1.4);
                tile.fill_rect_bd(
                    [pu - pw, ptop, pu + pw, pbot - 0.8],
                    c.along,
                    c.pal.accent,
                    255,
                    1.3,
                    Some(c.pal.rim),
                    Some(c.pal.deep),
                    c.tc,
                    1.2,
                );
                tile.fill_ellipse_bd(
                    [pu, pbot - 1.3, pw, 1.3],
                    c.along,
                    c.pal.deep,
                    c.pal.accent,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    1.0,
                    0.9,
                );
                tile.fill_rect_bd(
                    [pu + pw * 0.2, ptop + 1.2, pu + pw * 0.55, pbot - 1.2],
                    c.along,
                    c.pal.pale,
                    165,
                    0.9,
                    None,
                    None,
                    c.tc,
                    0.0,
                );
                tile.fill_rect_bd(
                    [pu - pw - 0.5, ptop, pu + pw + 0.5, ptop + 1.3 * c.s],
                    c.along,
                    c.pal.pale,
                    255,
                    0.8,
                    None,
                    None,
                    c.tc,
                    0.0,
                );
            }
            _ => {
                // Hanging kitchen ladle / strainer scoop.
                let bowl_r = (3.2 * c.s).min((c.ht - hook_bot) * 0.36);
                let bowl_v = c.ht - bowl_r - 0.2;
                let bowl_u = cu + sw;
                let hdl = [
                    cu + sw * 0.25,
                    hook_bot,
                    0.75 * c.s,
                    bowl_u,
                    bowl_v - bowl_r + 0.5,
                    0.8 * c.s,
                ];
                tile.fill_capsule_sh(hdl, c.along, 205, 1.3);
                tile.fill_capsule_bd(
                    hdl,
                    c.along,
                    c.pal.deep,
                    c.pal.pale,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    0.9,
                    0.7,
                );
                tile.fill_ellipse_sh([bowl_u, bowl_v, bowl_r * 1.1, bowl_r], c.along, 225, 1.6);
                tile.fill_ellipse_bd(
                    [bowl_u, bowl_v, bowl_r * 1.1, bowl_r],
                    c.along,
                    c.pal.deep,
                    c.pal.soft,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    1.2,
                    1.1,
                );
                tile.fill_ellipse_bd(
                    [bowl_u, bowl_v, bowl_r * 0.52, bowl_r * 0.42],
                    c.along,
                    c.pal.deep,
                    c.pal.deep,
                    None,
                    c.tc,
                    220,
                    0.8,
                    0.0,
                );
            }
        }
    }
}

/// 6. Strung conduit + festooned sagging cable with wide swinging enamel lamp
///    shades and warm glowing inner rims.
fn paint_cable_run(tile: &mut BandTile, c: &BandCtx) {
    let lamp_pitch = 23.0 * c.s;
    let top_v = -c.ht + 1.8 * c.s;
    let conduit_v = -c.ht + 1.4 * c.s;

    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let u_rel = pos_mod(wu, lamp_pitch) / lamp_pitch;
        let sag = 4.0 * u_rel * (1.0 - u_rel);
        let cv = top_v + sag * (2.6 * c.s);
        for iv in 0..tile.vh {
            let v = tile.v_at(iv);
            let d_top = 1.15 * c.s - (v - conduit_v).abs();
            let d_sag = 0.95 * c.s - (v - cv).abs();
            let d_in = d_top.max(d_sag);
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(iu, iv, soft_u8(d_in, 1.4, 200));
            let aw = soft_u8(d_in, 1.0, 248);
            let col = if d_sag > d_top {
                c.pal.deep
            } else if v > conduit_v && c.tc > 0.0 {
                c.pal.rim
            } else {
                c.pal.mid
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 12.0 * c.s) / lamp_pitch).floor() as i32;
    let s1 = ((wu_max + 12.0 * c.s) / lamp_pitch).ceil() as i32;
    const BULB_GLOW: Rgb = (248, 224, 146);

    for slot in s0..=s1 {
        let cu = slot as f32 * lamp_pitch;
        let h = hash_slot(slot, c.seed ^ 0xCA8);
        let sw = c.sway * 1.9 * c.s * if (h & 1) == 0 { 1.0 } else { -0.8 };
        let jbox = [cu - 1.6 * c.s, -c.ht, cu + 1.6 * c.s, top_v + 1.2 * c.s];
        tile.fill_rect_sh(jbox, c.along, 225, 1.3);
        tile.fill_rect_bd(
            jbox,
            c.along,
            c.pal.deep,
            255,
            0.9,
            Some(c.pal.rim),
            None,
            c.tc,
            0.8,
        );

        let sh_u = cu + sw;
        let shade_top = top_v + 3.4 * c.s;
        let shade_bot = c.ht - 0.15;
        tile.fill_capsule_sh([cu, top_v, 0.75 * c.s, sh_u, shade_top + 1.0, 0.75 * c.s], c.along, 200, 1.2);
        tile.fill_capsule_bd(
            [cu, top_v, 0.7 * c.s, sh_u, shade_top + 1.0, 0.7 * c.s],
            c.along,
            c.pal.deep,
            c.pal.mid,
            Some(c.pal.rim),
            c.tc,
            255,
            0.8,
            0.6,
        );
        tile.fill_rect_bd(
            [sh_u - 1.5 * c.s, shade_top - 1.0 * c.s, sh_u + 1.5 * c.s, shade_top + 1.0 * c.s],
            c.along,
            c.pal.accent,
            255,
            0.9,
            Some(c.pal.pale),
            None,
            c.tc,
            0.8,
        );

        let sh_h = (shade_bot - shade_top).max(3.5);
        let Some((u_lo, u_hi)) = tile.u_range(sh_u - 6.4 * c.s, sh_u + 6.4 * c.s, c.along) else {
            continue;
        };
        let Some((v_lo, v_hi)) = tile.v_range(shade_top, shade_bot) else {
            continue;
        };
        for iv in v_lo..=v_hi {
            let v = tile.v_at(iv);
            let dv_in = (v - shade_top).min(shade_bot - v);
            if dv_in <= 0.0 {
                continue;
            }
            let t = ((v - shade_top) / sh_h).clamp(0.0, 1.0);
            let hw = (1.8 + 4.4 * t.powf(0.82)) * c.s;
            for iu in u_lo..=u_hi {
                let du = tile.wu_at(iu, c.along) - sh_u;
                let d_in = (hw - du.abs()).min(dv_in);
                if d_in <= 0.0 {
                    continue;
                }
                tile.sh_px(iu, iv, soft_u8(d_in, 1.8, 235));
                let aw = soft_u8(d_in, 1.3, 255);
                let col = if shade_bot - v < 1.65 * c.s {
                    if du.abs() < 2.2 * c.s {
                        BULB_GLOW
                    } else {
                        c.pal.pale
                    }
                } else if d_in < 1.25 && ((c.tc > 0.0 && t > 0.45) || (c.tc < 0.0 && t < 0.55)) {
                    c.pal.rim
                } else if du > hw * 0.22 {
                    c.pal.soft
                } else if du < -hw * 0.28 {
                    c.pal.deep
                } else {
                    c.pal.mid
                };
                tile.bd_px(iu, iv, col, aw);
            }
        }
    }
}

// ---- Floor 3: Lab ----------------------------------------------------------

/// 7. Open ladder cable tray carrying three colour-coded weaving cable bundles
///    and zip-tie collars, with real open gaps between rungs.
fn paint_cable_tray(tile: &mut BandTile, c: &BandCtx) {
    let rail_w = (1.8 * c.s).max(1.9);
    let rung_pitch = 6.8 * c.s;
    let rung_hw = (0.75 * c.s).max(0.75);
    let tie_pitch = 13.6 * c.s;
    let seed_f = (c.seed & 1023) as f32;

    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let r_lu = pos_mod(wu, rung_pitch);
        let du_rung = r_lu.min(rung_pitch - r_lu);
        let in_rung = du_rung < rung_hw;

        let t_lu = pos_mod(wu, tie_pitch);
        let on_tie = t_lu.min(tie_pitch - t_lu) < 0.65;

        let c1 = -c.ht * 0.42 + 0.9 * (wu * 0.08 + seed_f).sin();
        let c2 = 1.0 * (wu * 0.11 + seed_f * 0.3 + 2.1).sin();
        let c3 = c.ht * 0.40 + 0.85 * (wu * 0.09 + seed_f * 0.7).cos();

        for iv in 0..tile.vh {
            let v = tile.v_at(iv);
            let av = v.abs();
            if av >= c.ht {
                continue;
            }
            let nv = (v / c.ht.max(1.0)) * c.tc;
            if av >= c.ht - rail_w {
                let d_edge = (c.ht - av).min(av - (c.ht - rail_w) + 0.4);
                tile.sh_px(iu, iv, soft_u8(d_edge, 1.6, 230));
                let col = if c.ht - av < 1.2 && nv > 0.0 {
                    c.pal.pale
                } else if nv > 0.0 {
                    c.pal.rim
                } else {
                    c.pal.deep
                };
                tile.bd_px(iu, iv, col, soft_u8(d_edge, 1.4, 255));
                continue;
            }

            let d_c1 = 1.25 * c.s - (v - c1).abs();
            let d_c2 = 1.05 * c.s - (v - c2).abs();
            let d_c3 = 1.10 * c.s - (v - c3).abs();

            if d_c1 > 0.0 || d_c2 > 0.0 || d_c3 > 0.0 {
                let (d_in, mut col) = if d_c2 >= d_c1 && d_c2 >= d_c3 {
                    let col = if (v - c2) * c.tc > 0.25 {
                        c.pal.pale
                    } else {
                        c.pal.accent
                    };
                    (d_c2, col)
                } else if d_c3 >= d_c1 {
                    let col = if (v - c3) * c.tc > -0.2 {
                        c.pal.bark_mid
                    } else {
                        c.pal.bark_deep
                    };
                    (d_c3, col)
                } else {
                    let col = if (v - c1) * c.tc > 0.25 {
                        c.pal.soft
                    } else {
                        c.pal.deep
                    };
                    (d_c1, col)
                };
                if on_tie {
                    col = c.pal.pale;
                }
                tile.sh_px(iu, iv, soft_u8(d_in, 1.4, 210));
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.0, 250));
            } else if in_rung {
                let d_edge = rung_hw - du_rung + 0.35;
                tile.sh_px(iu, iv, soft_u8(d_edge, 1.1, 180));
                let col = if nv >= 0.0 { c.pal.mid } else { c.pal.deep };
                tile.bd_px(iu, iv, col, soft_u8(d_edge, 1.0, 230));
            }
        }
    }
}

/// 8. Riveted galvanized square ventilation duct with bolted seam flanges
///    alternating slatted air-intake louvres and embossed X-creased sheet metal.
fn paint_vent_duct(tile: &mut BandTile, c: &BandCtx) {
    let body_hw = c.ht * 0.84;
    let seam_pitch = 24.0 * c.s;
    let seam_hw = 1.9 * c.s;
    let bay_w = (seam_pitch - 2.0 * seam_hw).max(1.0);
    let slat_dark = palette::scale(c.pal.deep, 0.60);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / body_hw.max(1.0)) * c.tc;
        let pv = (v + body_hw) / (2.0 * body_hw).max(1.0);
        let bolt_hit = av < c.ht - 1.2 && (av < 0.8 || (av - c.ht * 0.55).abs() < 0.8);

        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let slot = (wu / seam_pitch).floor() as i32;
            let lu = wu - slot as f32 * seam_pitch;
            let du_seam = lu.min(seam_pitch - lu);
            let on_seam = du_seam < seam_hw;

            let r_here = if on_seam { c.ht } else { body_hw };
            let d_in = r_here - av;
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(iu, iv, soft_u8(d_in, 2.2, if on_seam { 235 } else { 220 }));
            let aw = soft_u8(d_in, 1.7, 255);

            let col = if on_seam {
                if du_seam < 0.6 {
                    c.pal.deep
                } else if bolt_hit {
                    c.pal.pale
                } else if d_in < 1.5 && v * c.tc > 0.0 {
                    c.pal.rim
                } else if v * c.tc > 0.0 {
                    c.pal.soft
                } else {
                    c.pal.mid
                }
            } else {
                let has_louvre = ((slot + (c.seed & 1) as i32) & 1) == 0;
                let pu = (lu - seam_hw) / bay_w;
                if pu > 0.03 && pu < 0.09 && pv > 0.40 && pv < 0.60 {
                    c.pal.accent
                } else if d_in < 1.5 && nv > 0.0 {
                    c.pal.rim
                } else if d_in < 1.7 && nv < 0.0 {
                    c.pal.deep
                } else if has_louvre && pu > 0.12 && pu < 0.88 && pv > 0.18 && pv < 0.82 {
                    let slat = pos_mod(lu * 0.45, 1.0);
                    if slat < 0.44 {
                        slat_dark
                    } else if slat < 0.72 {
                        c.pal.mid
                    } else if nv > 0.0 {
                        c.pal.rim
                    } else {
                        c.pal.soft
                    }
                } else {
                    let d_x = (pu - pv).abs().min((pu - (1.0 - pv)).abs());
                    let top_tri = pv < pu && pv < 1.0 - pu;
                    let bot_tri = pv > pu && pv > 1.0 - pu;
                    if d_x < 0.045 {
                        if pv * c.tc > 0.35 || pu < 0.5 {
                            c.pal.rim
                        } else {
                            c.pal.deep
                        }
                    } else if (bot_tri && c.tc > 0.0) || (top_tri && c.tc < 0.0) {
                        c.pal.soft
                    } else if (top_tri && c.tc > 0.0) || (bot_tri && c.tc < 0.0) {
                        c.pal.deep
                    } else {
                        c.pal.mid
                    }
                }
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }
}

/// 9. Rolled-steel laboratory I-beam gantry with electrified cyan busbar and a
///    4-wheeled sliding trolley carriage.
fn paint_gantry(tile: &mut BandTile, c: &BandCtx) {
    let beam_hw = c.ht * 0.72;
    let flange_w = (2.0 * c.s).max(2.1);
    let rib_pitch = 18.0 * c.s;
    let car_pitch = 42.0 * c.s;
    let web_dark = palette::scale(c.pal.deep, 0.85);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= beam_hw {
            continue;
        }
        let nv = (v / beam_hw.max(1.0)) * c.tc;
        let in_flange = av >= beam_hw - flange_w;
        let d_in = beam_hw - av;
        let sh_w = soft_u8(d_in, 2.0, 225);
        let aw = soft_u8(d_in, 1.5, 255);
        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let r_lu = pos_mod(wu, rib_pitch);
            let on_rib = r_lu.min(rib_pitch - r_lu) < 1.0 * c.s;
            tile.sh_px(iu, iv, sh_w);
            let col = if in_flange {
                if d_in < 1.4 && nv > 0.0 {
                    c.pal.rim
                } else if nv > 0.0 {
                    c.pal.soft
                } else {
                    c.pal.deep
                }
            } else if on_rib {
                if nv >= 0.0 {
                    c.pal.mid
                } else {
                    c.pal.deep
                }
            } else if av < 0.52 {
                c.pal.accent
            } else {
                web_dark
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 14.0 * c.s) / car_pitch).floor() as i32;
    let s1 = ((wu_max + 14.0 * c.s) / car_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let cu = (slot as f32 + 0.5) * car_pitch;
        let cw = 6.8 * c.s;
        tile.fill_rect_sh([cu - cw, -c.ht, cu + cw, c.ht], c.along, 240, 2.0);
        tile.fill_rect_bd(
            [cu - cw, -c.ht, cu + cw, c.ht],
            c.along,
            c.pal.mid,
            255,
            1.4,
            Some(c.pal.rim),
            Some(c.pal.deep),
            c.tc,
            1.4,
        );
        for &wx in &[-cw * 0.65, cw * 0.65] {
            for &wy in &[-beam_hw * 0.82, beam_hw * 0.82] {
                tile.fill_ellipse_bd(
                    [cu + wx, wy, 1.8 * c.s, 1.8 * c.s],
                    c.along,
                    c.pal.deep,
                    c.pal.pale,
                    Some(c.pal.rim),
                    c.tc,
                    255,
                    0.8,
                    0.7,
                );
            }
        }
        tile.fill_rect_bd(
            [cu - cw * 0.55, -2.2 * c.s, cu + cw * 0.55, 2.2 * c.s],
            c.along,
            c.pal.bark_mid,
            255,
            0.8,
            None,
            None,
            c.tc,
            0.0,
        );
        tile.fill_rect_bd(
            [cu - cw * 0.22, -1.3 * c.s, cu + cw * 0.22, 1.3 * c.s],
            c.along,
            c.pal.accent,
            255,
            0.6,
            None,
            None,
            c.tc,
            0.0,
        );
    }
}

// ---- Floor 4: Laundry ------------------------------------------------------

/// 10. Strung laundry rope with clothespins holding billowing linen sheets,
///     sleeved shirts, and striped towels with real gaps between garments.
fn paint_clothesline(tile: &mut BandTile, c: &BandCtx) {
    let rope_v = -c.ht + 2.2 * c.s;
    let item_pitch = 16.0 * c.s;
    let sag_span = item_pitch * 2.0;

    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let u_rel = pos_mod(wu, sag_span) / sag_span;
        let rv = rope_v + 4.0 * u_rel * (1.0 - u_rel) * 1.3 * c.s;
        for iv in 0..tile.vh {
            let v = tile.v_at(iv);
            let d_in = 0.85 * c.s - (v - rv).abs();
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(iu, iv, soft_u8(d_in, 1.1, 185));
            tile.bd_px(iu, iv, c.pal.accent, soft_u8(d_in, 0.8, 250));
        }
    }

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 12.0 * c.s) / item_pitch).floor() as i32;
    let s1 = ((wu_max + 12.0 * c.s) / item_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let h = hash_slot(slot, c.seed ^ 0xC10);
        let cu = (slot as f32 + 0.5) * item_pitch;
        let u_rel = pos_mod(cu, sag_span) / sag_span;
        let v_top = rope_v + 4.0 * u_rel * (1.0 - u_rel) * 1.3 * c.s;
        let kind_i = (slot + (c.seed & 3) as i32).rem_euclid(3);
        let sw_bot = c.sway * 2.4 * c.s * if (h & 4) == 0 { 1.0 } else { -0.8 };

        let pin_sep = if kind_i == 0 { 3.4 } else { 2.2 } * c.s;
        for &px in &[-pin_sep, pin_sep] {
            tile.fill_rect_sh(
                [cu + px - 0.75 * c.s, -c.ht, cu + px + 0.75 * c.s, v_top + 1.1 * c.s],
                c.along,
                210,
                1.1,
            );
            tile.fill_rect_bd(
                [cu + px - 0.7 * c.s, -c.ht, cu + px + 0.7 * c.s, v_top + 1.0 * c.s],
                c.along,
                c.pal.accent,
                255,
                0.7,
                None,
                None,
                c.tc,
                0.0,
            );
        }

        let (hw, v_bot, base_col, fold_col) = match kind_i {
            0 => (5.3 * c.s, c.ht - 0.25, c.pal.pale, c.pal.mid),
            1 => (5.2 * c.s, c.ht - 0.8 * c.s, c.pal.bark_mid, c.pal.bark_deep),
            _ => (
                3.5 * c.s,
                c.ht - 0.35,
                palette::mix(c.pal.pale, c.pal.soft, 0.40),
                c.pal.deep,
            ),
        };

        let gh = (v_bot - v_top).max(4.0);
        let pad = hw + sw_bot.abs() + 1.5;
        let Some((u_lo, u_hi)) = tile.u_range(cu - pad, cu + pad, c.along) else {
            continue;
        };
        let Some((v_lo, v_hi)) = tile.v_range(v_top, v_bot + 0.8 * c.s) else {
            continue;
        };
        for iv in v_lo..=v_hi {
            let v = tile.v_at(iv);
            let t = ((v - v_top) / gh).clamp(0.0, 1.0);
            let c_here = cu + sw_bot * (t * t);
            let hw_here = if kind_i == 1 {
                if t < 0.42 {
                    hw
                } else {
                    hw * 0.58
                }
            } else {
                hw * (0.96 + 0.08 * t)
            };
            for iu in u_lo..=u_hi {
                let du = tile.wu_at(iu, c.along) - c_here;
                if kind_i == 1 && t < 0.14 && du.abs() < 1.3 * c.s {
                    continue;
                }
                let hem_wave = 0.7 * c.s * (du * 0.9 + slot as f32).sin();
                let v_end = (v_bot + hem_wave).min(c.ht - 0.1);
                let d_in = (hw_here - du.abs()).min(v - v_top).min(v_end - v);
                if d_in <= 0.0 {
                    continue;
                }
                tile.sh_px(iu, iv, soft_u8(d_in, 1.8, 225));
                let aw = soft_u8(d_in, 1.4, 250);
                let pleat = (du * 1.15 + slot as f32 * 1.7).sin();
                let col = if d_in < 1.25 && ((c.tc > 0.0 && t > 0.72) || (c.tc < 0.0 && t < 0.28)) {
                    c.pal.rim
                } else if kind_i == 2 && t > 0.72 && t < 0.84 {
                    c.pal.accent
                } else if pleat < -0.35 {
                    palette::mix(base_col, fold_col, 0.42)
                } else if pleat > 0.45 {
                    palette::mix(base_col, c.pal.rim, 0.30)
                } else {
                    base_col
                };
                tile.bd_px(iu, iv, col, aw);
            }
        }
    }
}

/// 11. Canvas-lagged insulated steam pipe with clamping bands, brass union
///     collars, and subtle steam wisps at the joints.
fn paint_steam_pipe(tile: &mut BandTile, c: &BandCtx) {
    let sr = c.ht * 0.80;
    let strap_pitch = 18.0 * c.s;
    let strap_hw = 1.35 * c.s;
    let union_pitch = 42.0 * c.s;
    let wrap_step = 5.2 * c.s;

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / sr.max(1.0)) * c.tc;
        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let u_lu = pos_mod(wu, union_pitch);
            let du_u = u_lu.min(union_pitch - u_lu);
            let on_union = du_u < 2.8 * c.s;

            let s_lu = pos_mod(wu, strap_pitch);
            let du_s = s_lu.min(strap_pitch - s_lu);
            let on_strap = du_s < strap_hw;

            let r_here = if on_union || on_strap { c.ht } else { sr };
            let d_in = r_here - av;
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(
                iu,
                iv,
                soft_u8(d_in, 2.2, if on_union || on_strap { 230 } else { 210 }),
            );
            let aw = soft_u8(d_in, 1.7, 255);
            let col = if on_union {
                if du_u < 0.7 {
                    c.pal.deep
                } else if d_in < 1.4 && v * c.tc > 0.0 {
                    c.pal.rim
                } else if nv > 0.35 {
                    c.pal.pale
                } else {
                    c.pal.accent
                }
            } else if on_strap {
                if d_in < 1.2 && v * c.tc > 0.0 {
                    c.pal.rim
                } else if nv > 0.0 {
                    c.pal.mid
                } else {
                    c.pal.deep
                }
            } else if d_in < 1.5 && nv > 0.2 {
                c.pal.rim
            } else {
                let base = if nv > 0.35 {
                    c.pal.pale
                } else if nv > -0.18 {
                    c.pal.soft
                } else {
                    c.pal.mid
                };
                let wrap = pos_mod(wu + v * 0.55, wrap_step) / wrap_step;
                if wrap < 0.16 {
                    palette::mix(base, c.pal.deep, 0.48)
                } else {
                    base
                }
            };
            tile.bd_px(iu, iv, col, aw);
        }
    }

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 10.0 * c.s) / union_pitch).floor() as i32;
    let s1 = ((wu_max + 10.0 * c.s) / union_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let cu = slot as f32 * union_pitch + 2.8 * c.s;
        tile.fill_ellipse_bd(
            [cu + 2.0 * c.s, sr * 0.75 * c.tc, 3.2 * c.s, 1.6 * c.s],
            c.along,
            c.pal.soft,
            c.pal.pale,
            None,
            c.tc,
            125,
            2.0,
            0.0,
        );
    }
}

/// 12. Overhead ceiling drying-rack beam: four parallel wooden slats with real
///     air gaps, bound by cast-iron pulley cross-yokes and draped washcloths.
fn paint_rack_beam(tile: &mut BandTile, c: &BandCtx) {
    let yoke_pitch = 22.0 * c.s;
    let yoke_hw = 1.8 * c.s;
    let slat_vs = [-c.ht * 0.76, -c.ht * 0.26, c.ht * 0.26, c.ht * 0.76];
    let slat_hr = (1.15 * c.s).max(1.15);
    let wood_lit = palette::mix(c.pal.accent, c.pal.pale, 0.35);
    let wood_dark = palette::mix(c.pal.accent, c.pal.deep, 0.45);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / c.ht.max(1.0)) * c.tc;
        let mut d_slat = -1.0_f32;
        let mut nearest_sv = slat_vs[0];
        for &sv in &slat_vs {
            let d = slat_hr - (v - sv).abs();
            if d > d_slat {
                d_slat = d;
                nearest_sv = sv;
            }
        }
        let local_side = (v - nearest_sv) * c.tc;

        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let y_lu = pos_mod(wu, yoke_pitch);
            let du_y = y_lu.min(yoke_pitch - y_lu);
            let d_yoke = yoke_hw - du_y;

            if d_yoke > 0.0 {
                let d_in = (d_yoke + 0.3).min(c.ht - av);
                tile.sh_px(iu, iv, soft_u8(d_in, 1.6, 230));
                let col = if c.ht - av < 1.4 && nv > 0.0 {
                    c.pal.rim
                } else if av < 1.4 * c.s {
                    c.pal.pale
                } else if nv > -0.1 {
                    c.pal.mid
                } else {
                    c.pal.deep
                };
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.3, 255));
            } else if d_slat > 0.0 {
                tile.sh_px(iu, iv, soft_u8(d_slat, 1.3, 200));
                let col = if d_slat < 0.85 && local_side > 0.0 {
                    c.pal.rim
                } else if local_side > 0.0 {
                    wood_lit
                } else {
                    wood_dark
                };
                tile.bd_px(iu, iv, col, soft_u8(d_slat, 1.1, 250));
            }
        }
    }

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 10.0 * c.s) / yoke_pitch).floor() as i32;
    let s1 = ((wu_max + 10.0 * c.s) / yoke_pitch).ceil() as i32;
    for slot in s0..=s1 {
        if (hash_slot(slot, c.seed ^ 0x8AC) & 1) == 0 {
            let cu = (slot as f32 + 0.5) * yoke_pitch;
            let cloth = [cu - 3.0 * c.s, -c.ht * 0.35, cu + 3.0 * c.s, c.ht * 0.85];
            tile.fill_rect_sh(cloth, c.along, 215, 1.5);
            tile.fill_rect_bd(
                cloth,
                c.along,
                c.pal.pale,
                245,
                1.1,
                Some(c.pal.rim),
                Some(c.pal.mid),
                c.tc,
                1.0,
            );
        }
    }
}

// ---- Floor 5: Printworks ---------------------------------------------------

/// 13. Continuous printed newsprint ribbon running over oiled-steel guide
///     rollers with broadsheet columns and cyan registration crop marks.
fn paint_paper_web(tile: &mut BandTile, c: &BandCtx) {
    let paper_hw = c.ht * 0.78;
    let rol_pitch = 28.0 * c.s;
    let rol_hw = 2.4 * c.s;
    let page_pitch = 20.0 * c.s;

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / c.ht.max(1.0)) * c.tc;
        let pv = (v + paper_hw) / (2.0 * paper_hw).max(1.0);

        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let r_lu = pos_mod(wu, rol_pitch);
            let du_r = r_lu.min(rol_pitch - r_lu);
            let on_rol = du_r < rol_hw;

            if on_rol {
                let d_in = (c.ht - av).min(rol_hw - du_r + 0.5);
                tile.sh_px(iu, iv, soft_u8(d_in, 1.8, 235));
                let col = if av >= paper_hw {
                    if av < c.ht - 1.2 {
                        c.pal.accent
                    } else {
                        c.pal.rim
                    }
                } else if du_r < 0.75 {
                    c.pal.rim
                } else if r_lu < rol_hw {
                    c.pal.soft
                } else {
                    c.pal.deep
                };
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.4, 255));
            } else if av < paper_hw {
                let d_in = paper_hw - av;
                tile.sh_px(iu, iv, soft_u8(d_in, 2.0, 215));
                let p_lu = pos_mod(wu, page_pitch) / page_pitch;
                let col = if d_in < 1.3 && nv > 0.0 {
                    c.pal.rim
                } else if du_r < rol_hw + 1.5 {
                    c.pal.mid
                } else if p_lu > 0.10 && p_lu < 0.90 && pv > 0.18 && pv < 0.82 {
                    if p_lu < 0.24 {
                        c.pal.deep
                    } else if p_lu > 0.30 && p_lu < 0.86 && (pv - 0.50).abs() > 0.06 {
                        if ((p_lu * 18.0).floor() as i32 & 1) == 0 {
                            c.pal.mid
                        } else {
                            c.pal.pale
                        }
                    } else {
                        c.pal.pale
                    }
                } else if (p_lu < 0.07 || p_lu > 0.93)
                    && ((pv - 0.14).abs() < 0.06 || (pv - 0.86).abs() < 0.06)
                {
                    c.pal.accent
                } else {
                    c.pal.pale
                };
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.6, 250));
            }
        }
    }
}

/// 14. Cast-iron press gantry beam carrying twin steel & cyan-inked rollers
///     and bolted bearing pillow blocks.
fn paint_roller_beam(tile: &mut BandTile, c: &BandCtx) {
    let beam_hw = c.ht * 0.76;
    let blk_pitch = 26.0 * c.s;
    let blk_hw = 3.2 * c.s;
    let inked_sheen = palette::mix(c.pal.soft, c.pal.accent, 0.45);

    for iv in 0..tile.vh {
        let v = tile.v_at(iv);
        let av = v.abs();
        if av >= c.ht {
            continue;
        }
        let nv = (v / c.ht.max(1.0)) * c.tc;
        for iu in 0..tile.uw {
            let wu = tile.wu_at(iu, c.along);
            let b_lu = pos_mod(wu, blk_pitch);
            let du_b = b_lu.min(blk_pitch - b_lu);
            let on_blk = du_b < blk_hw;

            if on_blk {
                let d_in = (c.ht - av).min(blk_hw - du_b + 0.5);
                tile.sh_px(iu, iv, soft_u8(d_in, 1.9, 240));
                let col = if d_in < 1.4 && nv > 0.0 {
                    c.pal.rim
                } else if av < 1.8 * c.s && du_b < 1.8 * c.s {
                    c.pal.pale
                } else if (av - c.ht * 0.65).abs() < 0.9 && du_b < 1.5 {
                    c.pal.accent
                } else if nv > 0.0 {
                    c.pal.soft
                } else {
                    c.pal.deep
                };
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.5, 255));
            } else if av < beam_hw {
                let d_in = beam_hw - av;
                tile.sh_px(iu, iv, soft_u8(d_in, 2.0, 225));
                let col = if d_in < 1.4 && nv > 0.0 {
                    c.pal.rim
                } else if av > beam_hw - 1.9 * c.s {
                    if nv < 0.0 {
                        c.pal.deep
                    } else {
                        c.pal.mid
                    }
                } else if av < 0.9 * c.s {
                    c.pal.bark_deep
                } else if v * c.tc > 0.0 {
                    if (av - beam_hw * 0.42).abs() < 1.0 {
                        inked_sheen
                    } else {
                        c.pal.accent
                    }
                } else if (av - beam_hw * 0.42).abs() < 0.85 {
                    c.pal.pale
                } else {
                    c.pal.mid
                };
                tile.bd_px(iu, iv, col, soft_u8(d_in, 1.6, 255));
            }
        }
    }
}

/// 15. Overhead drying line of broadsheet press proof sheets held by twin
///     spring clips with cyan colour registration bars and real gaps between pages.
fn paint_proof_line(tile: &mut BandTile, c: &BandCtx) {
    let rail_v = -c.ht + 2.0 * c.s;
    let sheet_pitch = 16.0 * c.s;
    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;

    tile.fill_rect_sh(
        [wu_min - 2.0, rail_v - 0.9 * c.s, wu_max + 2.0, rail_v + 0.9 * c.s],
        c.along,
        200,
        1.2,
    );
    tile.fill_rect_bd(
        [wu_min - 2.0, rail_v - 0.9 * c.s, wu_max + 2.0, rail_v + 0.9 * c.s],
        c.along,
        c.pal.mid,
        255,
        0.9,
        Some(c.pal.rim),
        Some(c.pal.deep),
        c.tc,
        0.8,
    );

    let s0 = ((wu_min - 12.0 * c.s) / sheet_pitch).floor() as i32;
    let s1 = ((wu_max + 12.0 * c.s) / sheet_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let h = hash_slot(slot, c.seed ^ 0x9F0);
        let cu = (slot as f32 + 0.5) * sheet_pitch;
        let hw = (4.6 + (h & 3) as f32 * 0.45) * c.s;
        let v_top = rail_v + 1.4 * c.s;
        let v_bot = c.ht - 0.25;
        let sw_bot = c.sway * 2.1 * c.s * if (h & 4) == 0 { 1.0 } else { -0.85 };
        let tilt = (((h >> 3) & 3) as f32 - 1.5) * 0.45 * c.s;

        for &px in &[-hw * 0.55, hw * 0.55] {
            tile.fill_rect_sh(
                [cu + px - 0.9 * c.s, -c.ht, cu + px + 0.9 * c.s, v_top + 0.9 * c.s],
                c.along,
                220,
                1.2,
            );
            tile.fill_rect_bd(
                [cu + px - 0.85 * c.s, -c.ht, cu + px + 0.85 * c.s, v_top + 0.9 * c.s],
                c.along,
                c.pal.soft,
                255,
                0.8,
                Some(c.pal.rim),
                None,
                c.tc,
                0.8,
            );
        }

        let sh_h = (v_bot - v_top).max(4.0);
        let pad = hw + sw_bot.abs() + tilt.abs() + 1.5;
        let Some((u_lo, u_hi)) = tile.u_range(cu - pad, cu + pad, c.along) else {
            continue;
        };
        let Some((v_lo, v_hi)) = tile.v_range(v_top, v_bot) else {
            continue;
        };
        for iv in v_lo..=v_hi {
            let v = tile.v_at(iv);
            let dv_in = (v - v_top).min(v_bot - v);
            if dv_in <= 0.0 {
                continue;
            }
            let t = ((v - v_top) / sh_h).clamp(0.0, 1.0);
            let c_here = cu + (sw_bot + tilt) * t;
            let row_bit = ((t * 14.0).floor() as i32) & 1;
            for iu in u_lo..=u_hi {
                let du = tile.wu_at(iu, c.along) - c_here;
                let d_in = (hw - du.abs()).min(dv_in);
                if d_in <= 0.0 {
                    continue;
                }
                tile.sh_px(iu, iv, soft_u8(d_in, 1.8, 225));
                let aw = soft_u8(d_in, 1.3, 250);
                let pu = (du + hw) / (2.0 * hw);
                let col = if d_in < 1.2 && ((c.tc > 0.0 && t > 0.7) || (c.tc < 0.0 && t < 0.3)) {
                    c.pal.rim
                } else if t > 0.14 && t < 0.28 && pu > 0.12 && pu < 0.88 {
                    c.pal.bark_deep
                } else if t > 0.34
                    && t < 0.78
                    && pu > 0.12
                    && pu < 0.88
                    && (pu - 0.50).abs() > 0.06
                {
                    if row_bit == 0 {
                        c.pal.mid
                    } else {
                        c.pal.pale
                    }
                } else if t > 0.83 && t < 0.93 && pu > 0.14 && pu < 0.86 {
                    c.pal.accent
                } else {
                    c.pal.pale
                };
                tile.bd_px(iu, iv, col, aw);
            }
        }
    }
}

// ---- Floor 6: Flatbed (Outdoors) -------------------------------------------

/// 16. Sweeping gnarled roadside bough crossing the view with forked twigs and
///     lush multi-lobe leaf clusters in two greens plus sunlit tips, keeping open
///     sky/deck holes between clusters.
fn paint_branch_canopy(tile: &mut BandTile, c: &BandCtx) {
    let seed_f = (c.seed & 1023) as f32;
    let bough_v = |wu: f32| -> f32 {
        2.1 * c.s * (wu * 0.068 + seed_f * 0.7).sin()
            + 1.0 * c.s * (wu * 0.145 + seed_f * 1.3).cos()
    };
    let bough_r = |wu: f32| -> f32 { (2.05 + 0.55 * (wu * 0.09 + seed_f * 0.4).sin()) * c.s };

    // Continuous sinuous bark bough.
    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let bv = bough_v(wu);
        let br = bough_r(wu);
        for iv in 0..tile.vh {
            let v = tile.v_at(iv);
            let d_in = br - (v - bv).abs();
            if d_in <= 0.0 {
                continue;
            }
            tile.sh_px(iu, iv, soft_u8(d_in, 1.8, 235));
            let side = ((v - bv) / br.max(0.5)) * c.tc;
            let col = if d_in < 1.15 && side > 0.12 {
                c.pal.bark_rim
            } else if side < -0.25 {
                c.pal.bark_deep
            } else {
                c.pal.bark_mid
            };
            tile.bd_px(iu, iv, col, soft_u8(d_in, 1.3, 255));
        }
    }

    // Alternating forked twigs and 5-lobe leaf clusters anchored to `along`.
    let cluster_pitch = 9.0 * c.s;
    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 16.0 * c.s) / cluster_pitch).floor() as i32;
    let s1 = ((wu_max + 16.0 * c.s) / cluster_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let h = hash_slot(slot, c.seed ^ 0xB8A);
        let jitter = (((h >> 2) & 7) as f32 - 3.5) * 0.40 * c.s;
        let cu = (slot as f32 + 0.5) * cluster_pitch + jitter;
        let bv = bough_v(cu);
        let sign = if (slot & 1) == 0 { 1.0 } else { -1.0 };
        let reach = (c.ht - 3.0 * c.s) * (0.76 + ((h >> 6) & 3) as f32 * 0.08);
        let tip_u = cu + if (h & 2) != 0 { 2.4 } else { -2.0 } * c.s + c.sway * 1.4 * c.s;
        let lim = (c.ht - 2.8 * c.s).max(0.5);
        let tip_v = (bv + sign * reach).clamp(-lim, lim);

        let twig = [cu, bv, 1.15 * c.s, tip_u, tip_v, 0.75 * c.s];
        tile.fill_capsule_sh(twig, c.along, 215, 1.4);
        tile.fill_capsule_bd(
            twig,
            c.along,
            c.pal.bark_deep,
            c.pal.bark_mid,
            Some(c.pal.bark_rim),
            c.tc,
            255,
            1.0,
            0.85,
        );

        for li in 0u32..5 {
            let lh = hash_u32(h ^ li.wrapping_mul(0x045D_9F3B));
            let ang = li as f32 * 1.256 + (lh & 15) as f32 * 0.10;
            let dist = (1.6 + ((lh >> 4) & 3) as f32 * 0.55) * c.s;
            let lu = tip_u + ang.cos() * dist;
            let lrx = (2.8 + ((lh >> 6) & 3) as f32 * 0.45) * c.s;
            let lry = (2.3 + ((lh >> 8) & 3) as f32 * 0.40) * c.s;
            let lv_lim = (c.ht - lry - 0.05).max(0.0);
            let lv = (tip_v + ang.sin() * dist * 0.82).clamp(-lv_lim, lv_lim);
            let l_mid = if li >= 3 && lv * c.tc > -0.5 {
                c.pal.accent
            } else {
                c.pal.mid
            };
            tile.fill_ellipse_sh([lu, lv, lrx, lry], c.along, 225, 2.0);
            tile.fill_ellipse_bd(
                [lu, lv, lrx, lry],
                c.along,
                c.pal.deep,
                l_mid,
                Some(c.pal.rim),
                c.tc,
                254,
                1.3,
                1.1,
            );
        }
    }
}

/// 17. Three razor-crisp 1-art-px sagging utility wires with periodic creosote
///     timber crossarms and glazed ceramic insulators.
fn paint_power_lines(tile: &mut BandTile, c: &BandCtx) {
    const WIRE_COL: Rgb = (156, 170, 188);
    const WIRE_RIM: Rgb = (206, 220, 236);
    const INS_COL: Rgb = (104, 182, 198);

    let pole_pitch = 48.0 * c.s;
    let sag_amt = (2.4 * c.s).min(c.ht * 0.34);
    let wire_vs = [-c.ht + 0.6, -0.6 * c.s, c.ht - 0.6 - sag_amt];

    let wu_min = tile.u0 as f32 + c.along;
    let wu_max = wu_min + tile.uw as f32;
    let s0 = ((wu_min - 8.0 * c.s) / pole_pitch).floor() as i32;
    let s1 = ((wu_max + 8.0 * c.s) / pole_pitch).ceil() as i32;
    for slot in s0..=s1 {
        let cu = slot as f32 * pole_pitch;
        let arm_hw = 1.45 * c.s;
        tile.fill_rect_sh([cu - arm_hw, -c.ht, cu + arm_hw, c.ht], c.along, 220, 1.5);
        tile.fill_rect_bd(
            [cu - arm_hw, -c.ht, cu + arm_hw, c.ht],
            c.along,
            c.pal.bark_mid,
            255,
            1.0,
            Some(c.pal.bark_rim),
            Some(c.pal.bark_deep),
            c.tc,
            0.9,
        );
        for &wv in &wire_vs {
            tile.fill_ellipse_bd(
                [cu, wv, 1.35 * c.s, 1.35 * c.s],
                c.along,
                c.pal.deep,
                INS_COL,
                Some(WIRE_RIM),
                c.tc,
                255,
                0.7,
                0.7,
            );
        }
    }

    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let u_rel = pos_mod(wu, pole_pitch) / pole_pitch;
        let sag = 4.0 * u_rel * (1.0 - u_rel) * sag_amt;
        for &wv_base in &wire_vs {
            let wv = (wv_base + sag).clamp(-c.ht + 0.5, c.ht - 0.5);
            let iv_i = (wv + tile.v_c).round() as i32 - tile.dv0;
            if iv_i >= 0 && (iv_i as usize) < tile.vh {
                let iv = iv_i as usize;
                tile.sh_px(iu, iv, 165);
                let col = if wv * c.tc > 0.0 { WIRE_RIM } else { WIRE_COL };
                tile.bd_px(iu, iv, col, 240);
            }
        }
    }
}

/// 18. Long billowing cumulus cloud shadow band — **soft shadow only** (writes
///     exclusively via [`shadow`], zero [`blend`] pixels).
fn paint_cloud_band(tile: &mut BandTile, c: &BandCtx) {
    let seed_f = (c.seed & 1023) as f32;
    let feather = (c.ht * 0.46).max(3.6);

    for iu in 0..tile.uw {
        let wu = tile.wu_at(iu, c.along);
        let b1 = 0.24 * (wu * 0.125 + seed_f * 0.9).sin()
            + 0.14 * (wu * 0.235 + seed_f * 0.3).cos();
        let b2 = 0.24 * (wu * 0.112 + seed_f * 1.4 + 1.1).cos()
            + 0.14 * (wu * 0.218 + seed_f * 0.6).sin();
        let top_edge = -c.ht * (0.78 + b1).clamp(0.44, 1.0);
        let bot_edge = c.ht * (0.78 + b2).clamp(0.44, 1.0);

        for iv in 0..tile.vh {
            let v = tile.v_at(iv);
            let d_in = (v - top_edge).min(bot_edge - v);
            if d_in <= 0.0 {
                continue;
            }
            let tn = (d_in / feather).clamp(0.0, 1.0);
            let t = 0.28 * tn + 0.72 * smoothstep(tn);
            let mottle = 0.84 + 0.16 * (wu * 0.12 + v * 0.28 + seed_f).sin();
            let w = (t * mottle * 255.0 + 0.5).clamp(1.0, 255.0) as u8;
            tile.sh_px(iu, iv, w);
        }
    }
}

// ---- Unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [(ForeKind, Theme); 18] = [
        (ForeKind::MainPipe, Theme::Sewer),
        (ForeKind::CatwalkGrating, Theme::Sewer),
        (ForeKind::ChainRun, Theme::Sewer),
        (ForeKind::ExtractorDuct, Theme::Kitchen),
        (ForeKind::PotRail, Theme::Kitchen),
        (ForeKind::CableRun, Theme::Kitchen),
        (ForeKind::CableTray, Theme::Lab),
        (ForeKind::VentDuct, Theme::Lab),
        (ForeKind::Gantry, Theme::Lab),
        (ForeKind::Clothesline, Theme::Laundry),
        (ForeKind::SteamPipe, Theme::Laundry),
        (ForeKind::RackBeam, Theme::Laundry),
        (ForeKind::PaperWeb, Theme::Printworks),
        (ForeKind::RollerBeam, Theme::Printworks),
        (ForeKind::ProofLine, Theme::Printworks),
        (ForeKind::BranchCanopy, Theme::Trailer),
        (ForeKind::PowerLines, Theme::Trailer),
        (ForeKind::CloudBand, Theme::Trailer),
    ];

    fn make_item(kind: ForeKind, horizontal: bool, scale: f32, seed: u32) -> ForeItem {
        ForeItem {
            kind,
            horizontal,
            cross: 60.0,
            depth: 2.2,
            scale,
            seed,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_draw(
        screen_cross: i32,
        shadow_cross: i32,
        from: i32,
        to: i32,
        scale: f32,
        alpha: u8,
        sway: f32,
        theme: Theme,
        along_offset: i32,
        toward_centre: i8,
    ) -> ForeDraw {
        ForeDraw {
            screen_cross,
            shadow_cross,
            from,
            to,
            scale,
            alpha,
            sway,
            theme,
            along_offset,
            elapsed_step: 0,
            toward_centre,
        }
    }

    #[test]
    fn thickness_is_within_band_bounds_and_scales_linearly() {
        for &(kind, _) in &ALL_KINDS {
            let t_lo = thickness(&make_item(kind, true, 1.0, 7));
            let t_hi = thickness(&make_item(kind, true, 1.6, 7));
            assert!(
                (10.0..=30.0).contains(&t_lo),
                "{kind:?} scale 1.0 thickness {t_lo} outside 10..=30 art px"
            );
            assert!(
                (10.0..=30.0).contains(&t_hi),
                "{kind:?} scale 1.6 thickness {t_hi} outside 10..=30 art px"
            );
            assert!(
                (t_hi - t_lo * 1.6).abs() < 1e-4,
                "{kind:?} thickness should scale linearly: {t_lo} vs {t_hi}"
            );
        }
    }

    #[test]
    fn every_kind_paints_across_full_from_to_extent_and_respects_zero_alpha() {
        let bg = (92, 96, 102);
        let from = 6i32;
        let to = 113i32;
        for &(kind, theme) in &ALL_KINDS {
            for &horizontal in &[true, false] {
                let item = make_item(kind, horizontal, 1.2, 17);
                let mut pb = PixelBuffer::new(120, 120);
                pb.clear(bg);
                let d = make_draw(60, 60, from, to, 1.2, ALPHA_MAX, 0.25, theme, 13, 1);
                draw(&mut pb, &item, &d);

                for u in from..=to {
                    let hit = (0..120usize).any(|v| {
                        let (x, y) = if horizontal { (u as usize, v) } else { (v, u as usize) };
                        pb.pixel_at(x, y) != bg
                    });
                    assert!(
                        hit,
                        "{kind:?} (horiz={horizontal}) left gap at along coord {u} in {from}..={to}"
                    );
                }

                // Zero alpha must write nothing.
                let mut pb0 = PixelBuffer::new(120, 120);
                pb0.clear(bg);
                let d0 = make_draw(60, 60, from, to, 1.2, 0, 0.25, theme, 13, 1);
                draw(&mut pb0, &item, &d0);
                assert!(
                    pb0.pixels().iter().all(|&p| p == bg),
                    "{kind:?} painted pixels when alpha == 0"
                );
            }
        }
    }

    #[test]
    fn thickness_matches_drawn_cross_extent_within_one_art_px() {
        let bg = (96, 100, 106);
        for &scale in &[1.0_f32, 1.25, 1.6] {
            for &(kind, theme) in &ALL_KINDS {
                for &horizontal in &[true, false] {
                    let item = make_item(kind, horizontal, scale, 19);
                    let expected = thickness(&item);
                    let mut pb = PixelBuffer::new(120, 120);
                    pb.clear(bg);
                    let d = make_draw(60, 60, 0, 119, scale, ALPHA_MAX, 0.2, theme, 13, 1);
                    draw(&mut pb, &item, &d);

                    let mut min_c = i32::MAX;
                    let mut max_c = i32::MIN;
                    for v in 0..120i32 {
                        let row_hit = (0..120usize).any(|u| {
                            let (x, y) = if horizontal { (u, v as usize) } else { (v as usize, u) };
                            pb.pixel_at(x, y) != bg
                        });
                        if row_hit {
                            min_c = min_c.min(v);
                            max_c = max_c.max(v);
                        }
                    }
                    assert!(
                        max_c >= min_c,
                        "{kind:?} (scale={scale}, horiz={horizontal}) drew nothing"
                    );
                    let drawn = (max_c - min_c + 1) as f32;
                    assert!(
                        (drawn - expected).abs() <= 1.0,
                        "{kind:?} (scale={scale}, horiz={horizontal}): drawn {drawn} vs thickness {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn repeating_detail_shifts_rigidly_with_along_offset() {
        let bg = (88, 92, 98);
        let shift = 10i32;
        for &(kind, theme) in &ALL_KINDS {
            let item = make_item(kind, true, 1.15, 29);
            let mut pb_a = PixelBuffer::new(120, 80);
            let mut pb_b = PixelBuffer::new(120, 80);
            pb_a.clear(bg);
            pb_b.clear(bg);
            draw(
                &mut pb_a,
                &item,
                &make_draw(40, 40, 0, 119, 1.15, ALPHA_MAX, 0.0, theme, 0, 1),
            );
            draw(
                &mut pb_b,
                &item,
                &make_draw(40, 40, 0, 119, 1.15, ALPHA_MAX, 0.0, theme, shift, 1),
            );
            assert_ne!(
                pb_a.pixels(),
                pb_b.pixels(),
                "{kind:?} did not change when along_offset shifted"
            );
            // World-anchored rigidity: pixel at (u, y) in `pb_b` (along + shift)
            // must equal pixel at (u + shift, y) in `pb_a` (along).
            for y in 0..80usize {
                for u in 16..96usize {
                    assert_eq!(
                        pb_b.pixel_at(u, y),
                        pb_a.pixel_at(u + shift as usize, y),
                        "{kind:?} detail crawled at ({u}, {y}) when along_offset shifted by {shift}"
                    );
                }
            }
        }
    }

    #[test]
    fn cloud_band_is_soft_shadow_only() {
        let item = make_item(ForeKind::CloudBand, true, 1.2, 41);

        // Over pure black, shadow() leaves every pixel (0, 0, 0); any blend() would brighten it.
        let mut pb_black = PixelBuffer::new(120, 80);
        pb_black.clear((0, 0, 0));
        draw(
            &mut pb_black,
            &item,
            &make_draw(40, 40, 0, 119, 1.2, ALPHA_MAX, 0.0, Theme::Trailer, 5, 1),
        );
        assert!(
            pb_black.pixels().iter().all(|&p| p == (0, 0, 0)),
            "CloudBand must write soft shadow only (zero blend writes)"
        );

        // Over a mid-tone deck, it must darken without brightening any channel.
        let deck = (110, 96, 82);
        let mut pb_deck = PixelBuffer::new(120, 80);
        pb_deck.clear(deck);
        draw(
            &mut pb_deck,
            &item,
            &make_draw(40, 40, 0, 119, 1.2, ALPHA_MAX, 0.0, Theme::Trailer, 5, 1),
        );
        let darkened = pb_deck
            .pixels()
            .iter()
            .filter(|&&p| p.0 < deck.0 && p.1 < deck.1 && p.2 < deck.2)
            .count();
        assert!(darkened > 400, "CloudBand darkened too few pixels ({darkened})");
        assert!(
            pb_deck
                .pixels()
                .iter()
                .all(|&p| p.0 <= deck.0 && p.1 <= deck.1 && p.2 <= deck.2),
            "CloudBand must never brighten any floor channel"
        );
    }

    #[test]
    fn see_through_kinds_keep_real_holes() {
        let bg = (90, 94, 100);
        let open_kinds = [
            (ForeKind::CatwalkGrating, Theme::Sewer),
            (ForeKind::ChainRun, Theme::Sewer),
            (ForeKind::PotRail, Theme::Kitchen),
            (ForeKind::CableRun, Theme::Kitchen),
            (ForeKind::CableTray, Theme::Lab),
            (ForeKind::Clothesline, Theme::Laundry),
            (ForeKind::RackBeam, Theme::Laundry),
            (ForeKind::ProofLine, Theme::Printworks),
            (ForeKind::BranchCanopy, Theme::Trailer),
            (ForeKind::PowerLines, Theme::Trailer),
        ];
        for &(kind, theme) in &open_kinds {
            let item = make_item(kind, true, 1.2, 23);
            let ht = (thickness(&item) * 0.5).floor() as i32;
            let mut pb = PixelBuffer::new(120, 120);
            pb.clear(bg);
            // Separate shadow_cross from screen_cross so we inspect pure body holes.
            draw(
                &mut pb,
                &item,
                &make_draw(30, 95, 0, 119, 1.2, ALPHA_MAX, 0.0, theme, 7, 1),
            );
            let mut total = 0usize;
            let mut holes = 0usize;
            for y in (30 - ht + 1)..=(30 + ht - 1) {
                for x in 10..110usize {
                    total += 1;
                    if pb.pixel_at(x, y as usize) == bg {
                        holes += 1;
                    }
                }
            }
            assert!(
                holes * 10 >= total,
                "{kind:?} should keep >= 10% real see-through holes inside its body band (got {holes}/{total})"
            );
        }
    }

    #[test]
    fn rim_highlight_flips_with_toward_centre() {
        let bg = (40, 44, 48);
        for &(kind, theme) in &[
            (ForeKind::MainPipe, Theme::Sewer),
            (ForeKind::ExtractorDuct, Theme::Kitchen),
            (ForeKind::VentDuct, Theme::Lab),
            (ForeKind::SteamPipe, Theme::Laundry),
        ] {
            let item = make_item(kind, true, 1.2, 11);
            let mut pb_pos = PixelBuffer::new(80, 80);
            let mut pb_neg = PixelBuffer::new(80, 80);
            pb_pos.clear(bg);
            pb_neg.clear(bg);
            draw(
                &mut pb_pos,
                &item,
                &make_draw(40, 40, 0, 79, 1.2, ALPHA_MAX, 0.0, theme, 0, 1),
            );
            draw(
                &mut pb_neg,
                &item,
                &make_draw(40, 40, 0, 79, 1.2, ALPHA_MAX, 0.0, theme, 0, -1),
            );
            assert_ne!(
                pb_pos.pixels(),
                pb_neg.pixels(),
                "{kind:?} rim highlight should flip with toward_centre"
            );
        }
    }

    #[test]
    fn swaying_kinds_respond_to_sway() {
        let bg = (82, 86, 92);
        for &(kind, theme) in &[
            (ForeKind::ChainRun, Theme::Sewer),
            (ForeKind::PotRail, Theme::Kitchen),
            (ForeKind::CableRun, Theme::Kitchen),
            (ForeKind::Clothesline, Theme::Laundry),
            (ForeKind::ProofLine, Theme::Printworks),
            (ForeKind::BranchCanopy, Theme::Trailer),
        ] {
            let item = make_item(kind, true, 1.2, 31);
            let mut pb_l = PixelBuffer::new(100, 80);
            let mut pb_r = PixelBuffer::new(100, 80);
            pb_l.clear(bg);
            pb_r.clear(bg);
            draw(
                &mut pb_l,
                &item,
                &make_draw(40, 40, 0, 99, 1.2, ALPHA_MAX, -1.0, theme, 9, 1),
            );
            draw(
                &mut pb_r,
                &item,
                &make_draw(40, 40, 0, 99, 1.2, ALPHA_MAX, 1.0, theme, 9, 1),
            );
            assert_ne!(pb_l.pixels(), pb_r.pixels(), "{kind:?} should respond to sway");
        }
    }

    #[test]
    fn nothing_written_outside_blend_and_shadow() {
        // 1. Direct PixelBuffer writers (plot / rect_fill / plot_add) stamp the
        //    material plane when materials are enabled; foreground::blend and
        //    foreground::shadow do not. Also, blend clamps alpha to ALPHA_MAX
        //    and multiplies by DARKEN, so over pure black no channel can reach
        //    opaque full-white.
        for &(kind, theme) in &ALL_KINDS {
            let item = make_item(kind, true, 1.2, 53);
            let mut pb = PixelBuffer::new(100, 80);
            pb.set_materials(true);
            pb.set_pen(0);
            pb.clear((0, 0, 0));
            pb.set_pen(7);
            draw(
                &mut pb,
                &item,
                &make_draw(40, 40, 0, 99, 1.2, 255, 0.3, theme, 11, 1),
            );
            assert!(
                pb.mat().iter().all(|&m| m == 0),
                "{kind:?} stamped material plane (wrote directly to PixelBuffer instead of blend/shadow)"
            );
            let peak = pb
                .pixels()
                .iter()
                .map(|p| p.0.max(p.1).max(p.2))
                .max()
                .unwrap_or(0);
            assert!(
                peak <= ALPHA_MAX,
                "{kind:?} bypassed blend's ALPHA_MAX/DARKEN clamp (peak={peak})"
            );
        }

        // 2. Source-level guard: production code (above `#[cfg(test)]`) never
        //    invokes direct PixelBuffer writers.
        let src = include_str!("art_foreground.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap_or(src);
        for forbidden in ["pb.plot", "pb.rect_fill", "pb.pixels_mut", "pb.clear", "pb.line"] {
            assert!(
                !prod.contains(forbidden),
                "art_foreground production code must not call `{forbidden}`"
            );
        }
    }
}
