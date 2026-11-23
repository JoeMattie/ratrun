//! Art for the parallax foreground layer ([`crate::render::foreground`]).
//!
//! Draws eighteen overhead silhouettes (three per floor theme) that sit nearer
//! the viewer than the playfield and slide faster than the floor as the camera
//! moves. Every pixel is written exclusively through
//! [`crate::render::foreground::blend`] and [`crate::render::foreground::shadow`]
//! in a single pass per item so effective opacity never exceeds `d.alpha`,
//! coverage stays slim (6–14 px across at scale 1), and the hostile layer,
//! damage digits and ink pass remain cleanly on top.
//!
//! **Contract** (do not change these two signatures: `render::foreground`
//! calls them and `extent` is what the layer culls and budgets with):
//!
//! ```ignore
//! pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw)
//! pub fn extent(item: &ForeItem) -> (f32, f32)
//! ```

use crate::game::level::Theme;
use crate::render::foreground::{
    blend, rim_tint, shadow, ForeDraw, ForeItem, ForeKind, ALPHA_MAX,
};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{self, Rgb};

/// Half-extent `(long, across)` in world px at scale 1, for culling and the
/// coverage budget. Bounds every pixel `draw` can write at scale 1 including
/// maximum sway (`-1.0..=1.0`).
pub fn extent(item: &ForeItem) -> (f32, f32) {
    match item.kind {
        // Sewer
        ForeKind::PipeRun => (half_len(item, 40.0), 5.0),
        ForeKind::Grate => (half_len(item, 14.0), 5.0),
        ForeKind::DripChain => {
            let dh = drop_half(item);
            if item.horizontal {
                (5.5, dh)
            } else {
                (dh, 5.5)
            }
        }
        // Kitchen
        ForeKind::PotRack => (half_len(item, 36.0) + 1.5, 5.5),
        ForeKind::ShelfEdge => (half_len(item, 40.0), 4.5),
        ForeKind::HangingUtensils => (half_len(item, 18.0) + 2.0, 5.5),
        // Lab
        ForeKind::CableTray => (half_len(item, 40.0), 4.5),
        ForeKind::VentDuct => (half_len(item, 18.0), 5.0),
        ForeKind::HangingLampCord => {
            let dh = drop_half(item);
            if item.horizontal {
                (8.0, dh)
            } else {
                (dh, 8.0)
            }
        }
        // Laundry
        ForeKind::Clothesline => (half_len(item, 40.0), 4.5),
        ForeKind::SheetOnLine => (half_len(item, 32.0) + 2.5, 5.5),
        ForeKind::SteamBank => (half_len(item, 22.0) + 3.0, 5.5),
        // Printworks
        ForeKind::PaperWeb => (half_len(item, 44.0), 5.0),
        ForeKind::HangingProofs => (half_len(item, 34.0) + 2.0, 5.5),
        ForeKind::RollerBeam => (half_len(item, 20.0), 5.0),
        // Flatbed
        ForeKind::TarpStrap => (half_len(item, 40.0), 5.0),
        ForeKind::SignGantryShadow => (half_len(item, 36.0), 6.5),
        ForeKind::CargoNetEdge => (half_len(item, 40.0) + 1.5, 5.5),
    }
}

/// Draw one overhead foreground item into `pb` at `d.screen`, scaled by
/// `d.scale` and modulated by `d.alpha` and `d.sway`.
pub fn draw(pb: &mut PixelBuffer, item: &ForeItem, d: &ForeDraw) {
    if d.alpha == 0 || pb.w == 0 || pb.h == 0 {
        return;
    }
    let scale = d.scale.max(0.25);
    let sway = d.sway.clamp(-1.0, 1.0);
    let pal = ForePal::for_theme(d.theme);
    let (eu, ev) = extent(item);

    let box_for = |horizontal: bool, half_u: f32, half_v: f32| PaintBox {
        screen: d.screen,
        horizontal,
        scale,
        alpha: d.alpha,
        half_u,
        half_v,
    };

    match item.kind {
        // ---- Sewer ---------------------------------------------------------
        ForeKind::PipeRun => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_pipe_run(u, v, hl, item.seed, &pal)
            });
        }
        ForeKind::Grate => {
            let hl = half_len(item, 14.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_grate(u, v, hl, item.seed, &pal)
            });
        }
        ForeKind::DripChain => {
            // Hangs downward (+y) in screen space regardless of item.horizontal.
            let dh = drop_half(item);
            box_for(true, 5.5, dh).run(pb, |dx, dy, _, _| {
                eval_drip_chain(dx, dy, dh, sway, item.seed, &pal)
            });
        }

        // ---- Kitchen -------------------------------------------------------
        ForeKind::PotRack => {
            let hl = half_len(item, 36.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_pot_rack(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::ShelfEdge => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_shelf_edge(u, v, hl, item.seed, &pal)
            });
        }
        ForeKind::HangingUtensils => {
            let hl = half_len(item, 18.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_hanging_utensils(u, v, hl, sway, item.seed, &pal)
            });
        }

        // ---- Lab -----------------------------------------------------------
        ForeKind::CableTray => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_cable_tray(u, v, hl, item.seed, &pal)
            });
        }
        ForeKind::VentDuct => {
            let hl = half_len(item, 18.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_vent_duct(u, v, hl, item.seed, &pal)
            });
        }
        ForeKind::HangingLampCord => {
            // Hangs downward (+y) in screen space regardless of item.horizontal.
            let dh = drop_half(item);
            box_for(true, 8.0, dh).run(pb, |dx, dy, _, _| {
                eval_hanging_lamp_cord(dx, dy, dh, sway, &pal)
            });
        }

        // ---- Laundry -------------------------------------------------------
        ForeKind::Clothesline => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_clothesline(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::SheetOnLine => {
            let hl = half_len(item, 32.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_sheet_on_line(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::SteamBank => {
            let hl = half_len(item, 22.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, px, py| {
                eval_steam_bank(u, v, px, py, hl, sway, item.seed, &pal)
            });
        }

        // ---- Printworks ----------------------------------------------------
        ForeKind::PaperWeb => {
            let hl = half_len(item, 44.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_paper_web(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::HangingProofs => {
            let hl = half_len(item, 34.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_hanging_proofs(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::RollerBeam => {
            let hl = half_len(item, 20.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_roller_beam(u, v, hl, item.seed, &pal)
            });
        }

        // ---- Flatbed -------------------------------------------------------
        ForeKind::TarpStrap => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_tarp_strap(u, v, hl, sway, item.seed, &pal)
            });
        }
        ForeKind::SignGantryShadow => {
            let hl = half_len(item, 36.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_sign_gantry_shadow(u, v, hl, item.seed)
            });
        }
        ForeKind::CargoNetEdge => {
            let hl = half_len(item, 40.0);
            box_for(item.horizontal, eu, ev).run(pb, |u, v, _, _| {
                eval_cargo_net_edge(u, v, hl, sway, item.seed, &pal)
            });
        }
    }
}

// ---- Shared palette, math & single-pass rasterizer -------------------------

/// Theme-grounded dark silhouette palette. Note that `foreground::blend`
/// multiplies every colour by `DARKEN` (0.72) and clamps alpha to `ALPHA_MAX`
/// (150/255), so `rim` and `pale` read as crisp catchlights / translucent veils
/// while staying well below hostile-tell brightness.
struct ForePal {
    deep: Rgb,
    mid: Rgb,
    soft: Rgb,
    rim: Rgb,
    pale: Rgb,
    accent: Rgb,
}

impl ForePal {
    fn for_theme(theme: Theme) -> Self {
        // Fold a gentle touch of the floor's live ambient rim tint into `rim`.
        let tint = palette::scale(rim_tint(theme), 0.40);
        let lift = |base: Rgb| -> Rgb {
            let c = palette::add(base, tint);
            (c.0.min(218), c.1.min(218), c.2.min(218))
        };
        match theme {
            Theme::Sewer => Self {
                deep: (16, 26, 25),
                mid: (32, 54, 50),
                soft: (56, 86, 80),
                rim: lift((156, 186, 168)),
                pale: (178, 200, 192),
                accent: (132, 78, 46),
            },
            Theme::Kitchen => Self {
                deep: (26, 18, 14),
                mid: (54, 38, 28),
                soft: (88, 66, 46),
                rim: lift((192, 156, 102)),
                pale: (196, 178, 152),
                accent: (164, 100, 56),
            },
            Theme::Lab => Self {
                deep: (16, 20, 34),
                mid: (34, 44, 72),
                soft: (62, 78, 118),
                rim: lift((144, 182, 202)),
                pale: (168, 198, 212),
                accent: (92, 164, 184),
            },
            Theme::Laundry => Self {
                deep: (24, 30, 48),
                mid: (46, 58, 86),
                soft: (78, 100, 128),
                rim: lift((172, 190, 194)),
                pale: (204, 212, 216),
                accent: (142, 108, 78),
            },
            Theme::Printworks => Self {
                deep: (24, 25, 32),
                mid: (48, 50, 60),
                soft: (84, 86, 94),
                rim: lift((172, 178, 172)),
                pale: (202, 200, 186),
                accent: (86, 152, 156),
            },
            Theme::Trailer => Self {
                deep: (28, 22, 16),
                mid: (58, 44, 32),
                soft: (96, 74, 52),
                rim: lift((194, 160, 98)),
                pale: (188, 166, 130),
                accent: (162, 116, 52),
            },
        }
    }
}

/// Output of a per-pixel shape evaluator. Each pixel is written at most once.
#[derive(Clone, Copy)]
enum Paint {
    None,
    /// Alpha-blend `rgb` with local weight `0..=255` scaled by `d.alpha`.
    Blend(Rgb, u8),
    /// Darken underlying pixel with local strength `0.0..=1.0` scaled by
    /// `d.alpha / ALPHA_MAX`.
    Shadow(f32),
}

#[inline]
fn half_len(item: &ForeItem, default_len: f32) -> f32 {
    let l = if item.len > 0.0 { item.len } else { default_len };
    (l * 0.5).max(4.0)
}

#[inline]
fn drop_half(item: &ForeItem) -> f32 {
    if item.len > 0.0 {
        (item.len * 0.30).clamp(8.5, 11.5)
    } else {
        9.0
    }
}

#[inline]
fn mul_alpha(base: u8, weight: u8) -> u8 {
    ((base as u16 * weight as u16 + 127) / 255) as u8
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

/// Clipped single-pass box rasterizer. Iterates in scaled buffer coordinates
/// inside `[-half_u*scale .. +half_u*scale] × [-half_v*scale .. +half_v*scale]`
/// clipped to the buffer, guaranteeing zero gaps at any `scale`, O(visible area)
/// cost, no double-blending, and strict compliance with `extent`.
struct PaintBox {
    screen: (i32, i32),
    horizontal: bool,
    scale: f32,
    alpha: u8,
    half_u: f32,
    half_v: f32,
}

impl PaintBox {
    #[inline]
    fn run<F>(self, pb: &mut PixelBuffer, mut eval: F)
    where
        F: FnMut(f32, f32, i32, i32) -> Paint,
    {
        let (ox, oy) = self.screen;
        let su_lim = (self.half_u * self.scale).floor() as i32;
        let sv_lim = (self.half_v * self.scale).floor() as i32;
        let bw = pb.w as i32;
        let bh = pb.h as i32;

        let (su0, su1, sv0, sv1) = if self.horizontal {
            (
                (-su_lim).max(-ox),
                su_lim.min(bw - 1 - ox),
                (-sv_lim).max(-oy),
                sv_lim.min(bh - 1 - oy),
            )
        } else {
            (
                (-su_lim).max(-oy),
                su_lim.min(bh - 1 - oy),
                (-sv_lim).max(-ox),
                sv_lim.min(bw - 1 - ox),
            )
        };
        if su0 > su1 || sv0 > sv1 {
            return;
        }

        let inv_s = 1.0 / self.scale;
        let shadow_scale = (self.alpha as f32 / ALPHA_MAX as f32).clamp(0.0, 1.0);
        for sv in sv0..=sv1 {
            let v = sv as f32 * inv_s;
            for su in su0..=su1 {
                let u = su as f32 * inv_s;
                let (px, py) = if self.horizontal {
                    (ox + su, oy + sv)
                } else {
                    (ox + sv, oy + su)
                };
                match eval(u, v, px, py) {
                    Paint::None => {}
                    Paint::Blend(rgb, w) => {
                        let a = mul_alpha(self.alpha, w);
                        if a > 0 {
                            blend(pb, px, py, rgb, a);
                        }
                    }
                    Paint::Shadow(strength) => {
                        let s = strength * shadow_scale;
                        if s > 0.0 {
                            shadow(pb, px, py, s);
                        }
                    }
                }
            }
        }
    }
}

// ---- Sewer: PipeRun, Grate, DripChain --------------------------------------

/// Fat corroded overhead pipe with protruding bolted flanges, a crisp top-edge
/// lamp highlight, and rust streaks clustered near joints.
fn eval_pipe_run(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    let phase = (seed % 20) as f32;
    let fu = (u + phase).rem_euclid(20.0);
    let end_flange = (u + hl) < 2.6 || (hl - u) < 2.6;
    let mid_flange = fu < 2.6;
    let is_flange = end_flange || mid_flange;

    if is_flange {
        if v.abs() > 4.5 {
            return Paint::None;
        }
        let edge_u = if end_flange {
            (u + hl) < 0.9 || (1.7..2.6).contains(&(hl - u))
        } else {
            fu < 0.9
        };
        if v.abs() > 3.6 {
            return Paint::Blend(pal.deep, 255);
        }
        // Bolt heads on flange faces.
        if ((v + 2.4).abs() < 0.65 || (v - 2.2).abs() < 0.65) && !edge_u {
            return Paint::Blend(pal.deep, 255);
        }
        if v < -1.8 && edge_u {
            return Paint::Blend(pal.rim, 255);
        }
        if v < 0.4 {
            return Paint::Blend(pal.soft, 255);
        }
        return Paint::Blend(pal.mid, 255);
    }

    // Soft cast shadow lip just underneath the pipe shaft.
    if v > 3.4 && v <= 4.6 {
        return Paint::Shadow(0.45);
    }
    if v.abs() > 3.4 {
        return Paint::None;
    }

    // Cylindrical shaft cross-section (7 px tall at scale 1).
    if v < -2.5 {
        return Paint::Blend(pal.deep, 255);
    }
    if v < -1.5 {
        return Paint::Blend(pal.rim, 245);
    }
    if v < -0.5 {
        return Paint::Blend(pal.soft, 255);
    }

    // Rust streaks clinging near flange joints on the lower belly.
    let near_joint = fu < 6.5 || fu > 17.0;
    let h = hash_slot(
        (u * 0.5).floor() as i32,
        seed ^ (((v.round() as i32) as u32).wrapping_mul(31)),
    );
    if near_joint && (0.0..=2.4).contains(&v) && (h & 7) < 3 {
        return Paint::Blend(pal.accent, 240);
    }
    if v < 1.8 {
        return Paint::Blend(pal.mid, 255);
    }
    Paint::Blend(pal.deep, 255)
}

/// Heavy barred sewer grating seen from below: dark outer frame, crisp parallel
/// cross-bars with wide open gaps showing the lit scene beneath.
fn eval_grate(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl || v.abs() > 4.6 {
        return Paint::None;
    }
    let at_end = (hl - u.abs()) < 1.6;
    if v < -2.9 {
        return if v < -3.8 {
            Paint::Blend(pal.rim, 235)
        } else {
            Paint::Blend(pal.mid, 255)
        };
    }
    if v > 2.9 {
        return if v > 3.8 {
            Paint::Blend(pal.deep, 255)
        } else {
            Paint::Blend(pal.mid, 255)
        };
    }
    if at_end {
        return if u < 0.0 && (u + hl) < 0.85 {
            Paint::Blend(pal.rim, 225)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Periodic cross-bars (1.7 px solid, 2.8 px open air).
    let bu = (u + (seed % 4) as f32).rem_euclid(4.5);
    let on_bar = bu < 1.7;
    let on_spine = v.abs() < 0.55;
    if on_bar {
        if on_spine && (hash_slot((u / 4.5).floor() as i32, seed) & 3) == 0 {
            return Paint::Blend(pal.accent, 230);
        }
        if bu < 0.85 {
            if v < -1.8 {
                return Paint::Blend(pal.rim, 225);
            }
            return Paint::Blend(pal.soft, 250);
        }
        return Paint::Blend(pal.deep, 255);
    }
    if on_spine {
        return Paint::Blend(pal.mid, 235);
    }
    Paint::None
}

/// Hanging iron chain with interlocking links swaying gently and a glistening
/// water droplet forming at the bottom tip.
fn eval_drip_chain(
    dx: f32,
    dy: f32,
    dh: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    let top = -(dh - 0.5);
    let bracket_bot = top + 2.0;
    let drip_top = dh - 3.6;

    // Ceiling shackle / mount plate at top.
    if (top..=bracket_bot).contains(&dy) {
        if dx.abs() <= 2.2 {
            return if dy < top + 1.0 {
                Paint::Blend(pal.rim, 235)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }
        return Paint::None;
    }

    // Sway grows linearly from 0 at the shackle to ±2.8 px at the drip tip.
    let span = (dh - bracket_bot).max(1.0);
    let t = ((dy - bracket_bot) / span).clamp(0.0, 1.0);
    let cx = sway * 2.8 * t;
    let lx = dx - cx;

    // Interlocking chain links between bracket and droplet.
    if dy > bracket_bot && dy <= drip_top {
        let link_pos = dy - bracket_bot;
        let link_idx = (link_pos / 2.2).floor() as i32;
        let in_link = link_pos.rem_euclid(2.2);
        let broad = ((link_idx + (seed as i32 & 1)) & 1) == 0;
        if broad {
            if lx.abs() <= 1.65 {
                if lx.abs() < 0.55 && in_link > 0.55 && in_link < 1.65 {
                    return Paint::None;
                }
                if lx < -0.5 && in_link < 1.1 {
                    return Paint::Blend(pal.rim, 235);
                }
                if lx < 0.4 {
                    return Paint::Blend(pal.soft, 250);
                }
                return Paint::Blend(pal.deep, 255);
            }
        } else if lx.abs() <= 0.85 {
            return if lx < 0.0 {
                Paint::Blend(pal.soft, 250)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }
        return Paint::None;
    }

    // Forming glistening teardrop bead at the chain's bottom end.
    if dy > drip_top && dy <= dh - 0.2 {
        let dl = dy - drip_top;
        let r = if dl < 0.9 {
            0.7
        } else if dl < 2.4 {
            1.65
        } else {
            0.85
        };
        if lx.abs() <= r {
            if lx < -0.2 && (1.0..=2.1).contains(&dl) {
                return Paint::Blend(pal.pale, 240);
            }
            if dl > 2.4 {
                return Paint::Blend(pal.rim, 225);
            }
            return Paint::Blend(pal.soft, 235);
        }
    }
    Paint::None
}

// ---- Kitchen: PotRack, ShelfEdge, HangingUtensils --------------------------

/// Overhead cast-iron pot rack rail with hanging copper/steel skillets and
/// saucepots whose rims catch the warm kitchen lamp light.
fn eval_pot_rack(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    // End suspension chains/brackets above the rail.
    if (-5.0..-3.8).contains(&v) && u.abs() <= hl {
        let near_end = (u + hl - 2.5).abs() < 1.0 || (hl - u - 2.5).abs() < 1.0;
        if near_end {
            return Paint::Blend(pal.soft, 245);
        }
    }

    // Fixed cast-iron rail bar along v = -3.8..-2.3.
    if (-3.8..=-2.3).contains(&v) && u.abs() <= hl {
        return if v < -3.0 {
            Paint::Blend(pal.rim, 240)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Hanging pots and pans below the rail (v = -2.3..+4.9).
    if v > -2.3 && v <= 4.9 {
        let t = ((v + 2.3) / 7.2).clamp(0.0, 1.0);
        let u_s = u - sway * 1.3 * t;
        const SPACING: f32 = 10.5;
        let slot = (u_s / SPACING).round() as i32;
        let slot_u = slot as f32 * SPACING;
        if slot_u.abs() > (hl - 3.8).max(0.5) {
            return Paint::None;
        }
        let lu = u_s - slot_u;
        let h = hash_slot(slot, seed);

        // S-hook from rail down to pot handle.
        if v < -1.0 {
            return if lu.abs() <= 0.6 {
                Paint::Blend(pal.soft, 245)
            } else {
                Paint::None
            };
        }

        match h % 3 {
            0 => {
                // Round cast-iron skillet + top handle stub.
                if v < 0.0 && lu.abs() <= 0.65 {
                    return Paint::Blend(pal.deep, 255);
                }
                let cy = 2.1;
                let d2 = lu * lu + (v - cy) * (v - cy);
                if d2 <= 7.8 {
                    if d2 >= 4.2 && lu < 0.8 && v < cy + 0.4 {
                        return Paint::Blend(pal.rim, 245);
                    }
                    if d2 >= 4.6 {
                        return Paint::Blend(pal.deep, 255);
                    }
                    if lu < -0.4 && v < cy {
                        return Paint::Blend(pal.accent, 240);
                    }
                    return Paint::Blend(pal.mid, 255);
                }
            }
            1 => {
                // Copper saucepot with side ear handles and bright rim lip.
                let half_w = if v < 0.1 { 2.6 } else { 2.3 };
                if (-0.8..=4.2).contains(&v) && lu.abs() <= half_w {
                    if v < 0.0 {
                        return Paint::Blend(pal.rim, 245);
                    }
                    if lu.abs() > half_w - 0.8 || v > 3.3 {
                        return Paint::Blend(pal.deep, 255);
                    }
                    if lu < -0.3 {
                        return Paint::Blend(pal.accent, 240);
                    }
                    return Paint::Blend(pal.mid, 255);
                }
                if (0.4..=1.5).contains(&v) && lu.abs() > half_w && lu.abs() <= half_w + 1.0 {
                    return Paint::Blend(pal.soft, 245);
                }
            }
            _ => {
                // Shallow oval sauté pan with gleaming top rim.
                let cy = 1.8;
                let nx = lu / 3.2;
                let ny = (v - cy) / 2.2;
                let d2 = nx * nx + ny * ny;
                if d2 <= 1.0 {
                    if d2 >= 0.55 && v < cy + 0.3 {
                        return Paint::Blend(pal.rim, 245);
                    }
                    if d2 >= 0.62 {
                        return Paint::Blend(pal.deep, 255);
                    }
                    return Paint::Blend(pal.mid, 255);
                }
            }
        }
    }
    Paint::None
}

/// Dark wooden underside of an overhead kitchen shelf with a lit front moulding
/// edge and translucent circular jar bottoms sitting along the overhang.
fn eval_shelf_edge(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    if v > 2.9 && v <= 4.1 {
        return Paint::Shadow(0.42);
    }
    if !(-3.4..=2.9).contains(&v) {
        return Paint::None;
    }

    // Front moulding lip and warm lamp catchlight.
    if v >= 2.0 {
        return Paint::Blend(pal.rim, 240);
    }
    if v >= 1.2 {
        return Paint::Blend(pal.soft, 255);
    }
    if v <= -2.6 {
        return Paint::Blend(pal.deep, 255);
    }

    // Circular glass jar bottoms resting near the shelf edge.
    const JAR_STEP: f32 = 11.0;
    let slot = (u / JAR_STEP).floor() as i32;
    let h = hash_slot(slot, seed);
    let slot_u = (slot as f32 + 0.5) * JAR_STEP + ((h & 3) as f32 - 1.5);
    if slot_u.abs() <= hl - 3.2 && (h & 7) < 5 {
        let r = if (h & 8) == 0 { 1.9 } else { 1.55 };
        let jv = -0.6;
        let du = u - slot_u;
        let dv = v - jv;
        let d2 = du * du + dv * dv;
        if d2 <= r * r {
            let inner = (r - 0.85) * (r - 0.85);
            if d2 >= inner && du <= 0.35 && dv <= 0.35 {
                return Paint::Blend(pal.pale, 225);
            }
            if d2 >= inner {
                return Paint::Blend(pal.deep, 255);
            }
            return Paint::Blend(pal.accent, 200);
        }
    }

    // Shelf support brackets every 24 px.
    let bu = (u + (seed % 24) as f32).rem_euclid(24.0);
    if bu < 2.0 && v < -0.4 {
        return Paint::Blend(pal.deep, 255);
    }

    // Subtle longitudinal wood grain on the dark plank underside.
    let grain = hash_slot((u * 0.25).floor() as i32, seed ^ ((v.round() as i32) as u32));
    if (grain & 7) == 0 {
        return Paint::Blend(pal.deep, 255);
    }
    Paint::Blend(pal.mid, 255)
}

/// Swaying kitchen utensils (ladles, wire-loop whisks, slotted spatulas)
/// suspended from a slim overhead hook rail.
fn eval_hanging_utensils(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    // Slim overhead hook bar.
    if (-4.8..=-3.4).contains(&v) && u.abs() <= hl {
        return if v < -4.1 {
            Paint::Blend(pal.rim, 235)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }
    if v <= -3.4 || v > 5.0 {
        return Paint::None;
    }

    let t = ((v + 3.4) / 8.4).clamp(0.0, 1.0);
    let u_s = u - sway * 1.9 * t;
    const SPACING: f32 = 7.5;
    let slot = (u_s / SPACING).floor() as i32;
    let slot_u = (slot as f32 + 0.5) * SPACING;
    if slot_u.abs() > hl - 2.2 {
        return Paint::None;
    }
    let lu = u_s - slot_u;
    let kind = (slot.unsigned_abs().wrapping_add(seed)) % 3;

    match kind {
        0 => {
            // Ladle: slender handle + cupped bowl with top rim glint.
            if v < 0.9 {
                return if lu.abs() <= 0.6 {
                    Paint::Blend(if lu < 0.0 { pal.soft } else { pal.deep }, 250)
                } else {
                    Paint::None
                };
            }
            let d2 = lu * lu + (v - 2.9) * (v - 2.9);
            if d2 <= 4.4 {
                if v < 1.9 {
                    return Paint::Blend(pal.rim, 245);
                }
                if lu < -0.4 {
                    return Paint::Blend(pal.soft, 250);
                }
                return Paint::Blend(pal.deep, 255);
            }
        }
        1 => {
            // Whisk: copper/wood handle + hollow wire teardrop loop.
            if v < 0.2 {
                return if lu.abs() <= 0.72 {
                    Paint::Blend(pal.accent, 245)
                } else {
                    Paint::None
                };
            }
            let cy = 2.6;
            let half_w =
                (0.95 + (v - 0.2) * 0.42).min(2.1) * (1.0 - ((v - 3.2).max(0.0) / 1.9));
            if half_w > 0.3 {
                let d_edge = (lu.abs() - half_w).abs();
                let center_wire = lu.abs() < 0.45 && v < 4.2;
                if d_edge <= 0.62 || (v > 4.1 && lu.abs() <= half_w + 0.45) {
                    return if lu < 0.0 && v < cy {
                        Paint::Blend(pal.rim, 235)
                    } else {
                        Paint::Blend(pal.soft, 240)
                    };
                }
                if center_wire {
                    return Paint::Blend(pal.mid, 210);
                }
            }
        }
        _ => {
            // Slotted spatula: slender stem + flat blade with open center slot.
            if v < 0.6 {
                return if lu.abs() <= 0.6 {
                    Paint::Blend(pal.mid, 250)
                } else {
                    Paint::None
                };
            }
            if v <= 4.6 && lu.abs() <= 1.65 {
                if lu.abs() < 0.45 && v > 1.4 && v < 3.7 {
                    return Paint::None;
                }
                if lu < -0.85 {
                    return Paint::Blend(pal.rim, 240);
                }
                return Paint::Blend(pal.deep, 255);
            }
        }
    }
    Paint::None
}

// ---- Lab: CableTray, VentDuct, HangingLampCord -----------------------------

/// Industrial ladder cable tray with open rung gaps and bundled wires,
/// including a muted colour-coded lab cable strand.
fn eval_cable_tray(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl || v.abs() > 4.2 {
        return Paint::None;
    }
    // Steel side rails.
    if v < -3.1 {
        return Paint::Blend(pal.rim, 240);
    }
    if v < -2.3 {
        return Paint::Blend(pal.deep, 255);
    }
    if v > 3.1 {
        return Paint::Blend(pal.deep, 255);
    }
    if v > 2.3 {
        return Paint::Blend(pal.soft, 250);
    }

    // Periodic zip-tie clamps every 14 px.
    let z = (u + (seed % 14) as f32).rem_euclid(14.0);
    let wave = (((u * 0.18 + (seed & 7) as f32).floor() as i32 & 1) as f32 - 0.5) * 0.65;

    // Three bundled cable strands snaking subtly down the tray.
    let c1 = -1.1 + wave * 0.5;
    let c2 = 0.2 - wave * 0.4;
    let c3 = 1.3 + wave * 0.5;
    let on_bundle =
        (v - c1).abs() <= 0.62 || (v - c2).abs() <= 0.62 || (v - c3).abs() <= 0.56;
    if z < 1.0 && v.abs() <= 2.0 {
        return Paint::Blend(pal.rim, 215);
    }
    if (v - c3).abs() <= 0.56 {
        // Muted coloured lab wire (stays well below hostile-tell intensity).
        let wire_col = match seed % 3 {
            0 => (78, 166, 186), // clinical cyan
            1 => (174, 134, 60), // muted warning amber
            _ => (158, 72, 78),  // dull sensor red
        };
        return Paint::Blend(wire_col, 240);
    }
    if (v - c1).abs() <= 0.62 {
        return Paint::Blend(pal.deep, 255);
    }
    if (v - c2).abs() <= 0.62 {
        return Paint::Blend(pal.mid, 250);
    }

    // Open ladder cross-rungs every 7 px (visible between wires & side rails).
    let r = (u + (seed % 7) as f32).rem_euclid(7.0);
    if !on_bundle && r < 1.15 {
        return Paint::Blend(pal.soft, 235);
    }
    Paint::None
}

/// Boxy galvanized HVAC vent duct with bolted cross-seams, recessed louvre
/// slats on alternating bays, and a crisp top-edge specular bevel.
fn eval_vent_duct(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    if v > 3.6 && v <= 4.7 {
        return Paint::Shadow(0.45);
    }
    if !(-4.4..=3.6).contains(&v) {
        return Paint::None;
    }

    // Sharp top/bottom sheet-metal bevels.
    if v < -3.5 {
        return Paint::Blend(pal.rim, 245);
    }
    if v < -2.6 {
        return Paint::Blend(pal.soft, 255);
    }
    if v > 2.7 {
        return Paint::Blend(pal.deep, 255);
    }

    // Bolted cross-seams every 16 px.
    let bay_len = 16.0;
    let shifted = u + (seed % 16) as f32;
    let bay = (shifted / bay_len).floor() as i32;
    let bu = shifted.rem_euclid(bay_len);
    if bu < 1.0 {
        if (v.abs() - 2.1).abs() < 0.65 {
            return Paint::Blend(pal.rim, 235);
        }
        return Paint::Blend(pal.deep, 255);
    }
    if bu < 1.9 {
        return Paint::Blend(pal.soft, 255);
    }

    // Recessed louvre slats (always shown on short point sections, alternating
    // on longer duct runs).
    let has_louvre = hl <= 12.0 || ((bay.unsigned_abs().wrapping_add(seed)) & 1) == 0;
    if has_louvre && (3.6..=12.8).contains(&bu) && v.abs() <= 2.1 {
        let slat = (v + 2.1).rem_euclid(1.4);
        return if slat < 0.7 {
            Paint::Blend(pal.deep, 255)
        } else {
            Paint::Blend(pal.soft, 245)
        };
    }

    Paint::Blend(pal.mid, 255)
}

/// Overhead lab pendant cord and domed enamel shade swinging gently, with a
/// soft lit bottom rim catching the bulb underneath.
fn eval_hanging_lamp_cord(
    dx: f32,
    dy: f32,
    dh: f32,
    sway: f32,
    pal: &ForePal,
) -> Paint {
    let top = -(dh - 0.4);
    let rose_bot = top + 1.8;
    let neck_top = dh - 6.8;
    let hood_top = neck_top + 1.2;
    let rim_top = dh - 2.0;

    // Ceiling rose at top.
    if (top..=rose_bot).contains(&dy) {
        if dx.abs() <= 1.8 {
            return if dy < top + 0.9 {
                Paint::Blend(pal.soft, 245)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }
        return Paint::None;
    }

    let span = (dh - rose_bot).max(1.0);
    let t = ((dy - rose_bot) / span).clamp(0.0, 1.0);
    let cx = sway * 2.4 * t;
    let lx = dx - cx;

    // Slender power cord down to the shade neck.
    if dy > rose_bot && dy < neck_top {
        return if lx.abs() <= 0.65 {
            Paint::Blend(if lx < 0.0 { pal.mid } else { pal.deep }, 250)
        } else {
            Paint::None
        };
    }

    // Shade collar / neck.
    if (neck_top..hood_top).contains(&dy) {
        if lx.abs() <= 1.6 {
            return if lx < -0.4 {
                Paint::Blend(pal.rim, 240)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }
        return Paint::None;
    }

    // Conical dark enamel hood widening toward the rim.
    if (hood_top..=rim_top).contains(&dy) {
        let frac = ((dy - hood_top) / (rim_top - hood_top).max(1.0)).clamp(0.0, 1.0);
        let half_w = 1.8 + frac * 3.6;
        if lx.abs() <= half_w {
            if lx < -half_w + 1.35 {
                return Paint::Blend(pal.rim, 240);
            }
            if lx < 0.0 {
                return Paint::Blend(pal.soft, 250);
            }
            return Paint::Blend(pal.deep, 255);
        }
        return Paint::None;
    }

    // Lit inner bottom rim of the shade.
    if dy > rim_top && dy <= dh - 0.5 && lx.abs() <= 5.4 {
        if lx.abs() > 4.5 {
            return Paint::Blend(pal.deep, 255);
        }
        return Paint::Blend(pal.pale, 225);
    }
    Paint::None
}

// ---- Laundry: Clothesline, SheetOnLine, SteamBank --------------------------

/// Taut sagging laundry clothesline with wooden clothespins clipped along its
/// span. Very low coverage so it reads as an airy overhead wire.
fn eval_clothesline(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    let norm = (u / hl).clamp(-1.0, 1.0);
    let sag = (1.0 - norm * norm) * (1.6 + sway * 1.1);
    let line_v = -1.1 + sag;

    // Wooden clothespins spaced along the line.
    const PEG_STEP: f32 = 12.0;
    let slot = (u / PEG_STEP).floor() as i32;
    let h = hash_slot(slot, seed);
    let slot_u = (slot as f32 + 0.5) * PEG_STEP + ((h & 3) as f32 - 1.5);
    if slot_u.abs() <= hl - 2.8 && (h & 3) != 0 {
        let lu = u - slot_u;
        let dv = v - line_v;
        if lu.abs() <= 0.92 && (-2.1..=2.1).contains(&dv) {
            if dv < -1.1 {
                return Paint::Blend(pal.rim, 230);
            }
            if dv > 0.65 && lu.abs() < 0.3 {
                return Paint::Blend(pal.deep, 245);
            }
            return Paint::Blend(pal.accent, 245);
        }
    }

    // Braided cord itself (1–2 px thick).
    let dv = v - line_v;
    if (-0.65..=0.72).contains(&dv) {
        return if dv < 0.0 {
            Paint::Blend(pal.soft, 235)
        } else {
            Paint::Blend(pal.deep, 245)
        };
    }
    Paint::None
}

/// Pale linen sheet hanging from a clothesline, billowing gently with sway.
/// Drawn at low alpha so it is the most translucent solid foreground item.
fn eval_sheet_on_line(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    // Top tension line across full span.
    if (-4.8..=-3.8).contains(&v) && u.abs() <= hl {
        return Paint::Blend(pal.deep, 190);
    }
    if !(-4.9..=4.9).contains(&v) {
        return Paint::None;
    }

    // Clothespins gripping the sheet top edge.
    let peg_half = (hl - 3.5).max(2.0);
    if (-4.9..=-2.7).contains(&v) {
        let on_peg = (u + peg_half).abs() <= 0.92
            || (u - peg_half).abs() <= 0.92
            || (hl > 20.0 && u.abs() <= 0.92);
        if on_peg {
            return if v < -3.9 {
                Paint::Blend(pal.rim, 220)
            } else {
                Paint::Blend(pal.accent, 230);
            };
        }
    }

    // Billowing sheet body below the line (v = -3.8..+4.4).
    let t = ((v + 3.8) / 8.2).clamp(0.0, 1.0);
    let u_s = u - sway * 2.4 * t;
    let sheet_hl = (hl - 1.4).max(3.0);
    if u_s.abs() > sheet_hl {
        return Paint::None;
    }

    // Subtle scalloped hem ripple along the bottom edge.
    let ripple =
        (((u_s * 0.45 + sway * 1.5 + (seed & 3) as f32).floor() as i32 & 1) as f32) * 0.65;
    let hem_v = 4.3 - ripple;
    if v > hem_v {
        return Paint::None;
    }

    // Translucent linen folds and bright hem catchlight (low alpha: ~40%..56%).
    if v > hem_v - 0.95 || u_s < -sheet_hl + 1.05 {
        return Paint::Blend(pal.pale, 144);
    }
    let fold = (u_s + sway * 1.6 * t + (seed % 6) as f32).rem_euclid(6.0);
    if fold < 1.3 {
        return Paint::Blend(pal.soft, 118);
    }
    Paint::Blend(pal.pale, 102)
}

/// Soft drifting bank of rising laundry steam: Bayer-dithered overlapping puffs
/// at very low alpha so the scene stays crystal-clear underneath.
#[allow(clippy::too_many_arguments)]
fn eval_steam_bank(
    u: f32,
    v: f32,
    px: i32,
    py: i32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    let u_s = u - sway * 2.8;
    if u_s.abs() > hl + 0.5 || v.abs() > 5.3 {
        return Paint::None;
    }

    // Evaluate nearest tiled puff slot + neighbours for overlapping soft lobes.
    const PUFF_STEP: f32 = 8.5;
    let base_slot = (u_s / PUFF_STEP).floor() as i32;
    let mut best_r2 = 2.0f32;
    for ds in -1..=1 {
        let slot = base_slot + ds;
        let h = hash_slot(slot, seed);
        let cx = (slot as f32 + 0.5) * PUFF_STEP + ((h & 3) as f32 - 1.5);
        if cx.abs() > hl - 2.0 {
            continue;
        }
        let cy = (((h >> 2) & 3) as f32 - 1.5) * 0.75;
        let rx = 5.0 + ((h >> 4) & 3) as f32 * 0.6;
        let ry = 3.4 + ((h >> 6) & 1) as f32 * 0.95;
        let nx = (u_s - cx) / rx;
        let ny = (v - cy) / ry;
        let r2 = nx * nx + ny * ny;
        if r2 < best_r2 {
            best_r2 = r2;
        }
    }
    if best_r2 > 1.0 {
        return Paint::None;
    }

    // 2×2 ordered Bayer dither keeps the mist wispy and cheap.
    let bayer = ((px & 1) << 1) | (py & 1);
    if best_r2 <= 0.45 {
        if bayer != 3 {
            return Paint::Blend(pal.pale, 78);
        }
    } else if ((px ^ py) & 1) == 0 {
        return Paint::Blend(pal.pale, 52);
    }
    Paint::None
}

// ---- Printworks: PaperWeb, HangingProofs, RollerBeam -----------------------

/// Continuous overhead newsprint paper web running between dark steel guide
/// rollers, pale and translucent with ruled printed columns and subtle web
/// tension flutter when swaying.
fn eval_paper_web(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }

    // Steel guide rollers at both ends and every 34 px on long runs.
    let near_end = (hl - u.abs()) < 2.4;
    let ru = (u + (seed % 34) as f32).rem_euclid(34.0);
    let mid_roller = hl > 24.0 && ru < 2.2;
    if near_end || mid_roller {
        if v.abs() > 4.5 {
            return Paint::None;
        }
        if v.abs() > 3.5 {
            return Paint::Blend(pal.deep, 255);
        }
        let lit_face = if near_end {
            (u + hl) < 1.0 || (1.4..2.4).contains(&(hl - u))
        } else {
            ru < 0.95
        };
        return if lit_face {
            Paint::Blend(pal.rim, 245)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Subtle mid-span web flutter with sway (stays inside ±4.1).
    let norm = (u / hl).clamp(-1.0, 1.0);
    let lv = v - (1.0 - norm * norm) * sway * 0.65;
    if lv.abs() > 3.4 {
        return Paint::None;
    }
    if lv < -2.5 {
        return Paint::Blend(pal.pale, 194);
    }
    if lv > 2.5 {
        return Paint::Blend(pal.soft, 174);
    }

    // Printed newspaper column layout tiled every 14 px.
    let pu = (u + (seed % 14) as f32).rem_euclid(14.0);
    if (12.2..=13.2).contains(&pu) && (lv + 1.9).abs() < 0.6 {
        return Paint::Blend(pal.accent, 195);
    }
    if (1.5..=11.2).contains(&pu) {
        if (-2.0..=-1.1).contains(&lv) {
            return Paint::Blend(pal.deep, 172);
        }
        let in_col = (1.5..=5.6).contains(&pu) || (6.8..=11.0).contains(&pu);
        if in_col && (-0.2..=2.0).contains(&lv) {
            let line = (lv + 0.2).rem_euclid(1.2);
            let h = hash_slot(u.floor() as i32, seed ^ ((lv.round() as i32) as u32));
            if line < 0.65 && (h & 7) < 6 {
                return Paint::Blend(pal.deep, 155);
            }
        }
    }
    Paint::Blend(pal.pale, 146)
}

/// Galley proof sheets clipped along a taut overhead darkroom wire, swaying
/// slightly with per-sheet phase variance.
fn eval_hanging_proofs(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    // Taut overhead wire.
    if (-4.8..=-3.9).contains(&v) && u.abs() <= hl {
        return Paint::Blend(pal.soft, 230);
    }
    if !(-4.9..=4.9).contains(&v) {
        return Paint::None;
    }

    const PROOF_STEP: f32 = 10.5;
    let t = ((v + 3.9) / 8.8).clamp(0.0, 1.0);
    let approx_slot = ((u - sway * 1.7 * t) / PROOF_STEP).round() as i32;
    for ds in -1..=1 {
        let slot = approx_slot + ds;
        let slot_u = slot as f32 * PROOF_STEP;
        if slot_u.abs() > (hl - 3.4).max(0.5) {
            continue;
        }
        let h = hash_slot(slot, seed);
        let phase_scale = 0.78 + ((h & 3) as f32) * 0.08;
        let lu = u - (slot_u + sway * 1.8 * phase_scale * t);

        // Metal spring clip gripping the wire.
        if (-4.9..=-2.6).contains(&v) && lu.abs() <= 0.95 {
            return if v < -3.7 {
                Paint::Blend(pal.rim, 235)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }

        // Rectangular proof sheet (6.5 px wide × 7 px tall).
        let half_w = 3.1;
        let bot_v = 3.8 + ((h >> 2) & 1) as f32 * 0.7;
        if (-2.6..=bot_v).contains(&v) && lu.abs() <= half_w {
            if lu < -half_w + 0.9 {
                return Paint::Blend(pal.pale, 188);
            }
            if v > bot_v - 0.85 || lu > half_w - 0.8 {
                return Paint::Blend(pal.soft, 175);
            }
            if lu > half_w - 1.9 && v < -1.5 {
                return Paint::Blend(pal.accent, 200);
            }
            if lu.abs() <= half_w - 1.0 && (-1.3..=bot_v - 1.3).contains(&v) {
                let row = (v + 1.3).rem_euclid(1.4);
                if row < 0.7 {
                    return Paint::Blend(pal.deep, 165);
                }
            }
            return Paint::Blend(pal.pale, 150);
        }
    }
    Paint::None
}

/// Heavy cast-iron overhead I-beam gantry carrying a parallel glossy steel
/// offset roller separated by a narrow mechanical slot.
fn eval_roller_beam(u: f32, v: f32, hl: f32, seed: u32, pal: &ForePal) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    if v > 3.7 && v <= 4.7 {
        return Paint::Shadow(0.45);
    }
    if !(-4.5..=3.7).contains(&v) {
        return Paint::None;
    }

    // End bearing housings locking the beam and roller together.
    if (hl - u.abs()) < 2.2 {
        return if v < -3.2 {
            Paint::Blend(pal.rim, 240)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Steel I-beam section (v = -4.5..-0.7).
    if v < -0.7 {
        if v < -3.6 {
            return Paint::Blend(pal.rim, 240);
        }
        if v < -2.8 {
            return Paint::Blend(pal.soft, 255);
        }
        if v > -1.5 {
            return Paint::Blend(pal.mid, 255);
        }
        let gu = (u + (seed % 12) as f32).rem_euclid(12.0);
        if gu < 1.6 {
            return Paint::Blend(pal.soft, 250);
        }
        return Paint::Blend(pal.deep, 255);
    }

    // Narrow mechanical slot (v = -0.7..+0.5) bridged by hanger brackets.
    if v < 0.5 {
        let bu = (u + (seed % 14) as f32).rem_euclid(14.0);
        return if bu < 2.0 {
            Paint::Blend(pal.deep, 255)
        } else {
            Paint::None
        };
    }

    // Parallel inked steel roller (v = +0.5..+3.7).
    if v < 1.3 {
        return Paint::Blend(pal.rim, 245);
    }
    if v < 2.1 {
        return Paint::Blend(pal.accent, 235);
    }
    if v < 2.9 {
        return Paint::Blend(pal.mid, 255);
    }
    Paint::Blend(pal.deep, 255)
}

// ---- Flatbed: TarpStrap, SignGantryShadow, CargoNetEdge --------------------

/// Woven overhead cargo tie-down ratchet strap with a steel ratchet buckle
/// housing and subtle wind vibration.
fn eval_tarp_strap(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    if u.abs() > hl {
        return Paint::None;
    }
    let norm = (u / hl).clamp(-1.0, 1.0);
    let vib = (1.0 - norm * norm) * sway * 0.8;
    let lv = v - vib;

    // End hook anchors.
    if (hl - u.abs()) < 1.9 && lv.abs() <= 2.6 {
        return if lv < -1.3 {
            Paint::Blend(pal.rim, 235)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Steel ratchet buckle housing near center (and tiled every 44 px on long straps).
    let bu = (u + 22.0 + ((seed % 7) as f32 - 3.0)).rem_euclid(44.0) - 22.0;
    let on_buckle = bu.abs() <= 3.4 && u.abs() < hl - 4.5;
    if on_buckle && lv.abs() <= 3.8 {
        if lv.abs() > 2.9 || bu.abs() > 2.5 {
            return if lv < -1.8 {
                Paint::Blend(pal.rim, 245)
            } else {
                Paint::Blend(pal.deep, 255)
            };
        }
        if lv < -0.5 && bu.abs() < 1.7 {
            return Paint::Blend(pal.rim, 245);
        }
        if (bu.abs() - 1.5).abs() < 0.55 && lv.abs() < 1.1 {
            return Paint::Blend(pal.deep, 255);
        }
        return Paint::Blend(pal.soft, 255);
    }

    // Slim woven webbing band (v = -2.1..+2.1).
    if lv.abs() > 2.1 {
        return Paint::None;
    }
    if lv < -1.3 {
        let stitch = u.rem_euclid(3.0) < 1.5;
        return Paint::Blend(if stitch { pal.rim } else { pal.deep }, 240);
    }
    if lv > 1.3 {
        return Paint::Blend(pal.deep, 255);
    }
    let weave = (u + lv).rem_euclid(3.0);
    if weave < 1.0 {
        return Paint::Blend(pal.mid, 250);
    }
    Paint::Blend(pal.accent, 245)
}

/// Pure overhead shadow of a highway road-sign gantry sweeping across the
/// flatbed: dual truss chords, diagonal cross-braces, and rectangular sign
/// panel silhouettes with soft feathered edges. Uses `shadow` ONLY.
fn eval_sign_gantry_shadow(u: f32, v: f32, hl: f32, seed: u32) -> Paint {
    if u.abs() > hl || v.abs() > 6.1 {
        return Paint::None;
    }

    // Rectangular highway sign panels mounted along the truss.
    const PANEL_STEP: f32 = 24.0;
    let phase = (seed % 24) as f32;
    let shifted = u + phase;
    let slot = (shifted / PANEL_STEP).floor() as i32;
    let slot_u = (slot as f32 + 0.5) * PANEL_STEP - phase;
    let h = hash_slot(slot, seed);
    if slot_u.abs() <= hl - 6.0 && (h & 3) != 0 {
        let half_pw = 5.6 + ((h >> 2) & 1) as f32 * 1.1;
        let du = (u - slot_u).abs();
        let dv = v.abs();
        if du <= half_pw && dv <= 5.8 {
            let feather = du > half_pw - 1.1 || dv > 4.6;
            return Paint::Shadow(if feather { 0.38 } else { 0.72 });
        }
    }

    // Upper and lower girder chords with 1 px soft feather.
    let av = v.abs();
    if (2.3..=5.0).contains(&av) {
        let feather = av > 4.0 || av < 2.9;
        return Paint::Shadow(if feather { 0.34 } else { 0.66 });
    }

    // Open triangular diagonal truss cross-bracing between the chords.
    if av < 2.3 {
        let cell = u.rem_euclid(9.2);
        let diag_v = if cell < 4.6 {
            -2.3 + cell
        } else {
            2.3 - (cell - 4.6)
        };
        let dist = (v - diag_v).abs();
        if dist <= 0.8 {
            return Paint::Shadow(0.60);
        }
        if dist <= 1.35 {
            return Paint::Shadow(0.30);
        }
    }
    Paint::None
}

/// Overhanging lattice edge of a braided rope cargo net: thick knotted border
/// rope along `-v` and open diagonal diamond mesh letting the flatbed show
/// cleanly through the openings.
fn eval_cargo_net_edge(
    u: f32,
    v: f32,
    hl: f32,
    sway: f32,
    seed: u32,
    pal: &ForePal,
) -> Paint {
    let t = ((v + 4.6) / 9.6).clamp(0.0, 1.0);
    let u_s = u - sway * 1.3 * t;
    if u_s.abs() > hl || !(-4.8..=4.8).contains(&v) {
        return Paint::None;
    }

    // Thick braided border rope along v = -4.7..-3.1.
    if v <= -3.1 {
        let braid = (u_s + (seed % 4) as f32).rem_euclid(4.0);
        if v < -3.9 {
            return if braid < 2.0 {
                Paint::Blend(pal.rim, 235)
            } else {
                Paint::Blend(pal.soft, 245)
            };
        }
        return if braid >= 2.0 {
            Paint::Blend(pal.mid, 250)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }

    // Open diamond rope lattice across v = -3.1..+4.6 (8 px diagonal pitch).
    let phase = (seed % 8) as f32;
    let d1 = (u_s + v + phase).rem_euclid(8.0);
    let d2 = (u_s - v + phase).rem_euclid(8.0);
    let on_d1 = !(0.68..=7.32).contains(&d1);
    let on_d2 = !(0.68..=7.32).contains(&d2);

    if on_d1 && on_d2 {
        return if v < 1.2 {
            Paint::Blend(pal.rim, 240)
        } else {
            Paint::Blend(pal.deep, 255)
        };
    }
    if (on_d1 || on_d2) && v <= 4.2 {
        return if on_d1 {
            Paint::Blend(pal.soft, 235)
        } else {
            Paint::Blend(pal.mid, 240)
        };
    }
    Paint::None
}

// ---- Unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;

    const ALL_KINDS: [(ForeKind, Theme); 18] = [
        (ForeKind::PipeRun, Theme::Sewer),
        (ForeKind::Grate, Theme::Sewer),
        (ForeKind::DripChain, Theme::Sewer),
        (ForeKind::PotRack, Theme::Kitchen),
        (ForeKind::ShelfEdge, Theme::Kitchen),
        (ForeKind::HangingUtensils, Theme::Kitchen),
        (ForeKind::CableTray, Theme::Lab),
        (ForeKind::VentDuct, Theme::Lab),
        (ForeKind::HangingLampCord, Theme::Lab),
        (ForeKind::Clothesline, Theme::Laundry),
        (ForeKind::SheetOnLine, Theme::Laundry),
        (ForeKind::SteamBank, Theme::Laundry),
        (ForeKind::PaperWeb, Theme::Printworks),
        (ForeKind::HangingProofs, Theme::Printworks),
        (ForeKind::RollerBeam, Theme::Printworks),
        (ForeKind::TarpStrap, Theme::Trailer),
        (ForeKind::SignGantryShadow, Theme::Trailer),
        (ForeKind::CargoNetEdge, Theme::Trailer),
    ];

    fn make_item(kind: ForeKind, len: f32, horizontal: bool, seed: u32) -> ForeItem {
        ForeItem {
            kind,
            pos: Vec2::new(100.0, 100.0),
            len,
            horizontal,
            depth: 1.25,
            seed,
        }
    }

    #[test]
    fn extent_bounds_every_pixel_drawn_at_scale_one_including_sway() {
        let bg = (80, 80, 80);
        let (w, h) = (180usize, 180usize);
        let center = (90i32, 90i32);

        for &(kind, theme) in &ALL_KINDS {
            for &horizontal in &[true, false] {
                for &len in &[0.0f32, 16.0, 48.0] {
                    for &sway in &[-1.0f32, 0.0, 1.0] {
                        let item = make_item(kind, len, horizontal, 42);
                        let (el, es) = extent(&item);
                        assert!(el > 0.0 && es > 0.0, "{kind:?} extent must be positive");

                        let (hx, hy) = if horizontal { (el, es) } else { (es, el) };
                        let mut pb = PixelBuffer::new(w, h);
                        pb.clear(bg);
                        let d = ForeDraw {
                            screen: center,
                            scale: 1.0,
                            alpha: ALPHA_MAX,
                            sway,
                            theme,
                        };
                        draw(&mut pb, &item, &d);

                        let mut touched = 0usize;
                        for y in 0..h {
                            for x in 0..w {
                                if pb.pixel_at(x, y) != bg {
                                    touched += 1;
                                    let dx = (x as i32 - center.0).abs() as f32;
                                    let dy = (y as i32 - center.1).abs() as f32;
                                    assert!(
                                        dx <= hx && dy <= hy,
                                        "{kind:?} (horiz={horizontal}, len={len}, sway={sway}) \
                                         wrote pixel at ({dx}, {dy}) outside extent ({hx}, {hy})"
                                    );
                                }
                            }
                        }
                        assert!(
                            touched > 0,
                            "{kind:?} (horiz={horizontal}, len={len}) drew zero pixels"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn zero_alpha_writes_no_pixels() {
        let bg = (70, 90, 110);
        for &(kind, theme) in &ALL_KINDS {
            let item = make_item(kind, 48.0, true, 7);
            let mut pb = PixelBuffer::new(120, 60);
            pb.clear(bg);
            let d = ForeDraw {
                screen: (60, 30),
                scale: 1.3,
                alpha: 0,
                sway: 0.5,
                theme,
            };
            draw(&mut pb, &item, &d);
            assert!(
                pb.pixels().iter().all(|&p| p == bg),
                "{kind:?} modified pixels when alpha == 0"
            );
        }
    }

    #[test]
    fn sign_gantry_shadow_only_darkens() {
        let bg = (120, 110, 95);
        let item = make_item(ForeKind::SignGantryShadow, 56.0, true, 9);
        let mut pb = PixelBuffer::new(140, 40);
        pb.clear(bg);
        let d = ForeDraw {
            screen: (70, 20),
            scale: 1.2,
            alpha: ALPHA_MAX,
            sway: 0.0,
            theme: Theme::Trailer,
        };
        draw(&mut pb, &item, &d);

        let mut darkened = 0usize;
        for &p in pb.pixels() {
            assert!(
                p.0 <= bg.0 && p.1 <= bg.1 && p.2 <= bg.2,
                "SignGantryShadow brightened a pixel: {p:?} > {bg:?}"
            );
            if p != bg {
                darkened += 1;
            }
        }
        assert!(darkened > 20, "SignGantryShadow must darken pixels");
    }

    #[test]
    fn sheet_and_steam_are_more_translucent_than_solid_pipe() {
        let bg = (180, 180, 180);
        let measure_avg_delta = |kind: ForeKind, theme: Theme| -> f32 {
            let item = make_item(kind, 40.0, true, 5);
            let mut pb = PixelBuffer::new(120, 40);
            pb.clear(bg);
            let d = ForeDraw {
                screen: (60, 20),
                scale: 1.0,
                alpha: ALPHA_MAX,
                sway: 0.0,
                theme,
            };
            draw(&mut pb, &item, &d);
            let mut sum = 0u64;
            let mut count = 0u64;
            for &p in pb.pixels() {
                if p != bg {
                    sum += (p.0 as i32 - bg.0 as i32).unsigned_abs() as u64
                        + (p.1 as i32 - bg.1 as i32).unsigned_abs() as u64
                        + (p.2 as i32 - bg.2 as i32).unsigned_abs() as u64;
                    count += 1;
                }
            }
            assert!(count > 0);
            sum as f32 / count as f32
        };

        let pipe_delta = measure_avg_delta(ForeKind::PipeRun, Theme::Sewer);
        let sheet_delta = measure_avg_delta(ForeKind::SheetOnLine, Theme::Laundry);
        let steam_delta = measure_avg_delta(ForeKind::SteamBank, Theme::Laundry);
        assert!(
            sheet_delta < pipe_delta,
            "SheetOnLine ({sheet_delta}) should be more translucent than PipeRun ({pipe_delta})"
        );
        assert!(
            steam_delta < pipe_delta,
            "SteamBank ({steam_delta}) should be very translucent vs PipeRun ({pipe_delta})"
        );
    }

    #[test]
    fn swaying_kinds_respond_to_sway_and_rigid_kinds_stay_still() {
        let render = |kind: ForeKind, theme: Theme, sway: f32| -> Vec<Rgb> {
            let item = make_item(kind, 44.0, true, 13);
            let mut pb = PixelBuffer::new(120, 40);
            pb.clear((60, 60, 60));
            let d = ForeDraw {
                screen: (60, 20),
                scale: 1.2,
                alpha: 140,
                sway,
                theme,
            };
            draw(&mut pb, &item, &d);
            pb.pixels().to_vec()
        };

        for &(kind, theme) in &[
            (ForeKind::DripChain, Theme::Sewer),
            (ForeKind::HangingUtensils, Theme::Kitchen),
            (ForeKind::HangingLampCord, Theme::Lab),
            (ForeKind::Clothesline, Theme::Laundry),
            (ForeKind::SheetOnLine, Theme::Laundry),
            (ForeKind::SteamBank, Theme::Laundry),
            (ForeKind::PaperWeb, Theme::Printworks),
            (ForeKind::HangingProofs, Theme::Printworks),
        ] {
            assert_ne!(
                render(kind, theme, -1.0),
                render(kind, theme, 1.0),
                "{kind:?} should visibly respond to sway"
            );
        }

        for &(kind, theme) in &[
            (ForeKind::PipeRun, Theme::Sewer),
            (ForeKind::Grate, Theme::Sewer),
            (ForeKind::ShelfEdge, Theme::Kitchen),
            (ForeKind::CableTray, Theme::Lab),
            (ForeKind::VentDuct, Theme::Lab),
            (ForeKind::RollerBeam, Theme::Printworks),
            (ForeKind::SignGantryShadow, Theme::Trailer),
        ] {
            assert_eq!(
                render(kind, theme, -1.0),
                render(kind, theme, 1.0),
                "rigid {kind:?} should not move when sway changes"
            );
        }
    }

    #[test]
    fn nothing_drawn_is_brighter_than_hostile_tells() {
        // Blended over pure black at max foreground alpha (150), every channel
        // must stay well below hostile-tell peaks (~220..255).
        for &(kind, theme) in &ALL_KINDS {
            let item = make_item(kind, 48.0, true, 3);
            let mut pb = PixelBuffer::new(120, 40);
            pb.clear((0, 0, 0));
            let d = ForeDraw {
                screen: (60, 20),
                scale: 1.3,
                alpha: ALPHA_MAX,
                sway: 0.0,
                theme,
            };
            draw(&mut pb, &item, &d);
            for &p in pb.pixels() {
                let peak = p.0.max(p.1).max(p.2);
                assert!(
                    peak <= 130,
                    "{kind:?} produced too-bright pixel {p:?} over black at alpha={ALPHA_MAX}"
                );
            }
        }
    }
}
