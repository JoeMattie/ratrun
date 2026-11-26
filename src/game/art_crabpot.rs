//! Art for the Outfall **Crab Pot** — a rusted-iron and sea-teal wire-mesh
//! lobster/crab cage with a funnel door on the `+facing` approach side, a
//! swaying buoy rope rising into the water column, and a silver bait fish
//! skewered inside.
//!
//! # Render Pass Order
//!
//! 1. [`draw_crab_pot`] — scene pass, **before** bodies: ground drop shadow
//!    (pen `0`), dark interior cavity, floor wire mesh, rusted frame, bait fish
//!    (absent in [`CrabPotState::Open`]), funnel mouth + state-driven door, and
//!    swaying buoy rope + rising micro-bubbles.
//! 2. Bodies (enemies and the rat).
//! 3. [`draw_crab_pot_front`] — foreground pass, **after** the rat: overhead
//!    wire-mesh roof bars, central rusted crossbar, and corner rivets drawn over
//!    the centre **only** when `view.caught_rat` is true so a trapped rat sits
//!    visibly behind the bars.
//! 4. [`draw_crab_pot_tell`] — hostile layer, **after** lighting: a clear warm
//!    amber-to-coral outline of the exact [`POT_HALF`] square footprint during
//!    [`CrabPotState::Closing`].
//!
//! Pure render-only drawing: zero heap allocations, zero RNG, smooth continuous
//! animation from `view.anim` and `view.progress`.

use std::f32::consts::{PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

/// Direction *toward* the light in screen space (up-left), matching `art_enemy`
/// and `art_traps`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Half-width of the square cage footprint in world pixels (hit test reads this).
pub const POT_HALF: f32 = 9.0;

/// How far the funnel door mouth reaches out along `+facing` past [`POT_HALF`].
pub const DOOR_DEPTH: f32 = 4.0;

/// Lifecycle state of an Outfall crab pot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrabPotState {
    /// Baited, funnel door propped open on the `+facing` side.
    Set,
    /// Triggered warning: the funnel door is falling across `progress` (`0..1`).
    Closing,
    /// Door latched shut, holding whatever is inside (`caught` / `caught_rat`).
    Shut,
    /// Spent / reopened: door hanging limp outward, bait gone.
    Open,
}

/// Pure render input for one Outfall crab pot.
#[derive(Clone, Copy, Debug)]
pub struct CrabPotView {
    /// World centre of the pot.
    pub pos: Vec2,
    /// Unit direction; the funnel door faces the public approach side (`+facing`).
    pub facing: Vec2,
    /// Current pot state.
    pub state: CrabPotState,
    /// Normalized progress `0.0..=1.0` through the current state (`Closing`: door falling).
    pub progress: f32,
    /// The rat is inside (`Shut`): [`draw_crab_pot_front`] draws the cage bars over it.
    pub caught_rat: bool,
    /// Horde bodies held inside (`0..=4`) for a smooth continuous rattle offset.
    pub caught: u8,
    /// Free-running seconds for smooth rope sway, bait bob, bubbles, and rattle.
    pub anim: f32,
}

// ---- Lighting material pens (`crate::render::lighting::material`) ----------
// Layout: bits 0..=3 = MaterialKind, bit 4 = CREATURE, bits 5..=7 = height (0..=7).

/// Floor drop shadow: untagged flat default (`MaterialKind::Default = 0`).
const PEN_SHADOW: u8 = 0;
/// Rusted iron frame and wire mesh: `MaterialKind::Metal = 5`, non-creature, height 2.
const PEN_METAL: u8 = (2 << 5) | 5;
/// Wet bait fish, glistening buoy rope, and specular droplets:
/// `MaterialKind::Slime = 11` (double-lobe wet sheen), non-creature, height 1.
const PEN_WET: u8 = (1 << 5) | 11;

// ---- Outfall Crab Pot Palette (rusted iron + oxidized sea-teal wire) -------

const POT_OUTLINE: Rgb = (10, 18, 24);
const RUST_LO: Rgb = (74, 38, 26);
const RUST_MID: Rgb = (152, 78, 44);
const RUST_HI: Rgb = (218, 138, 84);
const RUST_SPEC: Rgb = (244, 202, 154);

const MESH_LO: Rgb = (28, 64, 70);
const MESH_MID: Rgb = (66, 134, 140);
const MESH_HI: Rgb = (134, 206, 208);
const MESH_GLINT: Rgb = (204, 248, 246);

const CAVITY_DARK: Rgb = (11, 22, 30);
const CAVITY_MID: Rgb = (20, 38, 46);

const BAIT_LO: Rgb = (52, 98, 122);
const BAIT_MID: Rgb = (152, 220, 236);
const BAIT_HI: Rgb = (236, 252, 255);
const BAIT_FIN: Rgb = (218, 128, 86);
const BAIT_EYE: Rgb = (255, 222, 96);

const ROPE_LO: Rgb = (74, 60, 38);
const ROPE_MID: Rgb = (150, 124, 78);
const ROPE_HI: Rgb = (214, 190, 134);

const BUBBLE_COL: Rgb = (176, 242, 248);
const BUBBLE_HI: Rgb = (238, 254, 255);
const BRASS_PIN: Rgb = (242, 200, 96);

// ---- Small pure helpers ----------------------------------------------------

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn safe_dir(v: Vec2) -> Vec2 {
    let len_sq = v.x * v.x + v.y * v.y;
    if len_sq > 1e-4 {
        let inv = 1.0 / len_sq.sqrt();
        Vec2::new(v.x * inv, v.y * inv)
    } else {
        Vec2::new(1.0, 0.0)
    }
}

#[inline]
fn shade3(sh: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(sh, mid, l * (1.0 / 0.62))
    } else {
        mix(mid, hi, (l - 0.62) * (1.0 / 0.53))
    }
}

#[inline]
fn mute_open(c: Rgb, is_open: bool) -> Rgb {
    if !is_open {
        c
    } else {
        mix(scale(c, 0.78), (38, 52, 56), 0.30)
    }
}

#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(x as usize, y as usize);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amt: f32) {
    if amt <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(bg, (1.0 - amt).clamp(0.0, 1.0)));
}

fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    for _ in 0..256 {
        blend(pb, x, y, c, alpha);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[inline]
fn sd_ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let nu = (u - cu) / ru;
    let nv = (v - cv) / rv;
    (nu.hypot(nv) - 1.0) * ru.min(rv)
}

#[inline]
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let (pax, pay) = (u - a.0, v - a.1);
    let (bax, bay) = (b.0 - a.0, b.1 - a.1);
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    (pax - bax * h).hypot(pay - bay * h) - (r0 + (r1 - r0) * h)
}

#[inline]
fn sd_round_box(u: f32, v: f32, cu: f32, cv: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let qx = (u - cu).abs() - (hu - r).max(0.0);
    let qy = (v - cv).abs() - (hv - r).max(0.0);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

#[inline]
fn sd_box(u: f32, v: f32, hu: f32, hv: f32) -> f32 {
    let dx = u.abs() - hu;
    let dy = v.abs() - hv;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
}

/// Smooth rattle displacement applied identically to back and front passes.
fn pot_rattle(view: &CrabPotView, f: Vec2, p: Vec2) -> Vec2 {
    let prog = view.progress.clamp(0.0, 1.0);
    match view.state {
        CrabPotState::Shut => {
            let load =
                (view.caught.min(4) as f32) * 0.30 + if view.caught_rat { 0.42 } else { 0.0 };
            if load <= 0.0 {
                return Vec2::ZERO;
            }
            let ru = (view.anim * 23.0).sin() * load;
            let rv = (view.anim * 31.0 + 1.1).cos() * load * 0.85;
            f * ru + p * rv
        }
        CrabPotState::Closing => {
            let trem = (view.anim * 36.0 + prog * 14.0).sin() * (0.20 * prog);
            p * trem
        }
        CrabPotState::Set | CrabPotState::Open => Vec2::ZERO,
    }
}

/// Door local `u` coordinate and screen-space vertical lift (`-y` is up) for
/// `Set`, `Closing`, and `Shut`.
#[inline]
fn pot_door_pose(view: &CrabPotView) -> (f32, f32) {
    let open_u = POT_HALF + DOOR_DEPTH - 0.5;
    let shut_u = POT_HALF - 0.1;
    match view.state {
        CrabPotState::Set => (open_u, -2.0),
        CrabPotState::Closing => {
            let e = smoothstep(view.progress);
            (lerp(open_u, shut_u, e), lerp(-2.0, 0.0, e))
        }
        CrabPotState::Shut | CrabPotState::Open => (shut_u, 0.0),
    }
}

// ---- Public draw entry points ----------------------------------------------

/// Scene pass, drawn **before** bodies: ground drop shadow, dark interior
/// cavity, bottom wire mesh, rusted frame, bait fish (except in `Open`), funnel
/// mouth + state-driven door, and swaying buoy rope + rising bubbles.
pub fn draw_crab_pot(pb: &mut PixelBuffer, view: &CrabPotView, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let f = safe_dir(view.facing);
    let p = f.perp();
    let rattle = pot_rattle(view, f, p);
    let sx = view.pos.x + rattle.x - cam.x;
    let sy = view.pos.y + rattle.y - cam.y;

    // Conservative screen-bounds early out (includes 17 px rising buoy rope).
    let reach = POT_HALF + DOOR_DEPTH + 22.0;
    if sx + reach < -1.0
        || sx - reach > pb.w as f32 + 1.0
        || sy + reach < -1.0
        || sy - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let prev_pen = pb.pen();
    let fl = f.x * LIGHT.x + f.y * LIGHT.y;
    let pl = p.x * LIGHT.x + p.y * LIGHT.y;
    let is_open = view.state == CrabPotState::Open;

    let to_px = |u: f32, vv: f32| -> (i32, i32) {
        (
            (sx + f.x * u + p.x * vv).round() as i32,
            (sy + f.y * u + p.y * vv).round() as i32,
        )
    };
    let to_px_lift = |u: f32, vv: f32, lift_y: f32| -> (i32, i32) {
        (
            (sx + f.x * u + p.x * vv).round() as i32,
            (sy + f.y * u + p.y * vv + lift_y).round() as i32,
        )
    };

    // 1) Drop shadow + rounded-square cage frame + floor mesh + funnel cheeks.
    let pad = (POT_HALF + DOOR_DEPTH + 4.0).ceil() as i32;
    let x0 = (sx.floor() as i32 - pad).max(0);
    let x1 = (sx.ceil() as i32 + pad).min(pb.w as i32 - 1);
    let y0 = (sy.floor() as i32 - pad).max(0);
    let y1 = (sy.ceil() as i32 + pad).min(pb.h as i32 - 1);

    let sh_dx = 1.8f32;
    let sh_dy = 2.2f32;
    let du_sh = -(sh_dx * f.x + sh_dy * f.y);
    let dv_sh = -(sh_dx * p.x + sh_dy * p.y);

    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            let dx0 = x0 as f32 - sx;
            let dy = y as f32 - sy;
            let mut u = dx0 * f.x + dy * f.y;
            let mut vv = dx0 * p.x + dy * p.y;
            for x in x0..=x1 {
                let d = sd_round_box(u, vv, 0.0, 0.0, POT_HALF, POT_HALF, 2.2);
                let mouth_t = ((u - (POT_HALF - 1.2)) / (DOOR_DEPTH + 1.2)).clamp(0.0, 1.0);
                let mouth_hw = lerp(4.4, 5.8, mouth_t);
                let in_mouth_u = (POT_HALF - 1.2..=POT_HALF + DOOR_DEPTH).contains(&u);
                let in_mouth = in_mouth_u && vv.abs() <= mouth_hw;
                let on_cheek = in_mouth_u
                    && (mouth_hw - 1.1..=mouth_hw + 0.7).contains(&vv.abs());

                if d > 0.95 && !in_mouth && !on_cheek {
                    let su = u + du_sh;
                    let sv = vv + dv_sh;
                    if sd_round_box(su, sv, 0.0, 0.0, POT_HALF + 0.3, POT_HALF + 0.3, 2.4) <= 0.4 {
                        pb.set_pen(PEN_SHADOW);
                        darken(pb, x, y, 0.42);
                    }
                } else if d > 0.0 && !in_mouth && !on_cheek {
                    pb.set_pen(PEN_SHADOW);
                    blend(pb, x, y, POT_OUTLINE, 0.92);
                } else if d <= 0.0 {
                    let lit_val = (u / POT_HALF) * fl + (vv / POT_HALF) * pl;
                    if d > -1.55 {
                        let corner = (u.abs() - (POT_HALF - 1.5)).abs() < 1.2
                            && (vv.abs() - (POT_HALF - 1.5)).abs() < 1.2;
                        let mouth_gap = u > POT_HALF - 1.7
                            && vv.abs() < 4.5
                            && view.state != CrabPotState::Shut;
                        if mouth_gap {
                            pb.set_pen(PEN_SHADOW);
                            blend(pb, x, y, CAVITY_DARK, 0.78);
                        } else {
                            pb.set_pen(PEN_METAL);
                            let lum = 0.56 + 0.44 * lit_val + if corner { 0.18 } else { 0.0 };
                            let mut col = shade3(RUST_LO, RUST_MID, RUST_HI, lum);
                            if corner && lit_val > 0.15 {
                                col = mix(col, RUST_SPEC, 0.55);
                            }
                            if d < -0.95 && (((u + vv).round() as i32) & 3) == 0 {
                                col = mix(col, MESH_MID, 0.45);
                            }
                            pb.plot(x, y, mute_open(col, is_open));
                        }
                    } else {
                        // Dark seabed cavity + floor wire-mesh grid.
                        pb.set_pen(PEN_SHADOW);
                        let bg_dark = mix(
                            CAVITY_DARK,
                            CAVITY_MID,
                            (0.45 + 0.35 * lit_val).clamp(0.0, 1.0),
                        );
                        blend(pb, x, y, bg_dark, 0.76);

                        let gu = (u.rem_euclid(3.2) - 1.6).abs();
                        let gv = (vv.rem_euclid(3.2) - 1.6).abs();
                        if gu < 0.42 || gv < 0.42 {
                            pb.set_pen(PEN_METAL);
                            let w_col = shade3(MESH_LO, MESH_MID, MESH_HI, 0.48 + 0.36 * lit_val);
                            blend(
                                pb,
                                x,
                                y,
                                mute_open(w_col, is_open),
                                if is_open { 0.45 } else { 0.62 },
                            );
                        }
                    }
                } else if on_cheek {
                    let lit_val = (u / (POT_HALF + DOOR_DEPTH)) * fl + (vv / 6.0) * pl;
                    if vv.abs() > mouth_hw + 0.15 {
                        pb.set_pen(PEN_SHADOW);
                        blend(pb, x, y, POT_OUTLINE, 0.88);
                    } else {
                        pb.set_pen(PEN_METAL);
                        let col = shade3(RUST_LO, RUST_MID, RUST_HI, 0.58 + 0.42 * lit_val);
                        pb.plot(x, y, mute_open(col, is_open));
                    }
                } else if in_mouth {
                    pb.set_pen(PEN_SHADOW);
                    blend(pb, x, y, CAVITY_DARK, 0.65);
                    if (u.rem_euclid(2.4) - 1.2).abs() < 0.35 {
                        pb.set_pen(PEN_METAL);
                        blend(pb, x, y, mute_open(MESH_LO, is_open), 0.55);
                    }
                }

                u += f.x;
                vv += p.x;
            }
        }
    }

    // 2) Bait fish inside (Set, Closing; faintly visible in empty Shut; GONE in Open).
    let show_bait = matches!(view.state, CrabPotState::Set | CrabPotState::Closing)
        || (view.state == CrabPotState::Shut && !view.caught_rat && view.caught == 0);
    if show_bait {
        let bob_v = (view.anim * 2.4).sin() * 0.65;
        let wag = (view.anim * 4.2).sin() * 0.60;
        pb.set_pen(PEN_METAL);
        line(pb, to_px(-1.0, -3.4), to_px(-1.0, 3.4), RUST_LO, 0.72);

        pb.set_pen(PEN_WET);
        let bu = -1.2f32;
        let bv = bob_v;
        for iy in -5..=5i32 {
            for ix in -5..=5i32 {
                let px_u = bu + ix as f32 * 0.65;
                let px_v = bv + iy as f32 * 0.65;
                let du = px_u - bu;
                let dv = px_v - bv;
                let lu = du * 0.92 + dv * 0.38;
                let lv = -du * 0.38 + dv * 0.92;
                let d_body = sd_ellipse(lu, lv, 0.3, 0.0, 2.9, 1.25);
                let d_tail = sd_taper(lu, lv, (-2.1, 0.0), (-3.7, wag), 0.65, 1.05);
                let df = d_body.min(d_tail);
                let sp = to_px(px_u, px_v);
                if df <= 0.0 {
                    let lum = 0.55 + 0.45 * ((lu / 3.0) * fl + (lv / 1.4) * pl);
                    let col = if lu < -2.3 {
                        mix(BAIT_MID, BAIT_FIN, 0.55)
                    } else if lv < -0.25 {
                        shade3(BAIT_MID, BAIT_HI, (255, 255, 255), lum)
                    } else {
                        shade3(BAIT_LO, BAIT_MID, BAIT_HI, lum)
                    };
                    pb.plot(sp.0, sp.1, col);
                } else if df <= 0.75 {
                    blend(pb, sp.0, sp.1, POT_OUTLINE, 0.75);
                }
            }
        }
        let fin_p = to_px(bu - 0.5, bv - 1.3);
        pb.plot(fin_p.0, fin_p.1, BAIT_FIN);
        let eye_p = to_px(bu + 1.9, bv + 0.35);
        pb.plot(eye_p.0, eye_p.1, BAIT_EYE);
    } else if is_open {
        // Spent pot: bare bent skewer wire and dark kelp flecks; bait is gone.
        pb.set_pen(PEN_METAL);
        line(
            pb,
            to_px(-1.4, -2.5),
            to_px(-0.6, 2.2),
            mute_open(RUST_LO, true),
            0.72,
        );
        pb.set_pen(PEN_SHADOW);
        for &(ku, kv) in &[(-2.6f32, 1.5f32), (1.8, -2.2), (-0.4, -3.1)] {
            let kp = to_px(ku, kv);
            blend(pb, kp.0, kp.1, (28, 62, 48), 0.65);
        }
    }

    // 3) Shut without caught_rat: show squirming horde silhouettes + closed roof mesh.
    if view.state == CrabPotState::Shut && !view.caught_rat {
        if view.caught > 0 {
            pb.set_pen(PEN_SHADOW);
            let n_blobs = view.caught.min(4);
            for k in 0..n_blobs {
                let kf = k as f32;
                let ang = kf * 1.65 + view.anim * (2.6 + kf * 0.4);
                let cu = ang.cos() * 3.1;
                let cv = (ang * 1.2).sin() * 3.1;
                let bp = to_px(cu, cv);
                for dy in -1..=1i32 {
                    for dx in -1..=1i32 {
                        if dx.abs() + dy.abs() <= 1 {
                            blend(pb, bp.0 + dx, bp.1 + dy, (44, 58, 64), 0.78);
                        }
                    }
                }
                pb.plot(bp.0, bp.1, (224, 152, 86));
            }
        }
        pb.set_pen(PEN_METAL);
        for &b in &[-5.0f32, -1.6, 1.8, 5.2] {
            line(pb, to_px(b, -POT_HALF + 1.2), to_px(b, POT_HALF - 1.2), MESH_MID, 0.58);
            line(pb, to_px(-POT_HALF + 1.2, b), to_px(POT_HALF - 1.2, b), MESH_MID, 0.58);
        }
    }

    // 4) State-driven funnel door on the +facing side.
    match view.state {
        CrabPotState::Set | CrabPotState::Closing | CrabPotState::Shut => {
            let (door_u, lift_y) = pot_door_pose(view);
            let d_top = to_px_lift(door_u, -5.0, lift_y);
            let d_bot = to_px_lift(door_u, 5.0, lift_y);
            if lift_y < -0.2 {
                pb.set_pen(PEN_SHADOW);
                line(pb, to_px(door_u + 0.5, -4.8), to_px(door_u + 0.5, 4.8), POT_OUTLINE, 0.55);
            }
            pb.set_pen(PEN_METAL);
            line(
                pb,
                (d_top.0 + 1, d_top.1 + 1),
                (d_bot.0 + 1, d_bot.1 + 1),
                POT_OUTLINE,
                0.90,
            );
            let bar_col = if view.state == CrabPotState::Closing {
                RUST_HI
            } else {
                RUST_MID
            };
            line(pb, d_top, d_bot, bar_col, 0.98);

            let slat_u = door_u - if view.state == CrabPotState::Shut { 0.9 } else { 0.7 };
            line(
                pb,
                to_px_lift(slat_u, -4.4, lift_y * 0.75),
                to_px_lift(slat_u, 4.4, lift_y * 0.75),
                MESH_HI,
                0.85,
            );

            let mid_p = to_px_lift(door_u, 0.0, lift_y);
            match view.state {
                CrabPotState::Set => {
                    line(pb, to_px(-0.8, 0.0), mid_p, BRASS_PIN, 0.75);
                    pb.plot(mid_p.0, mid_p.1, RUST_SPEC);
                }
                CrabPotState::Closing => {
                    pb.plot(mid_p.0, mid_p.1, (255, 224, 136));
                    pb.plot(mid_p.0 - 1, mid_p.1, RUST_HI);
                }
                CrabPotState::Shut => {
                    line(pb, to_px(door_u - 1.0, 0.0), to_px(door_u + 1.0, 0.0), RUST_SPEC, 0.98);
                }
                CrabPotState::Open => {}
            }
        }
        CrabPotState::Open => {
            // Hanging limp/askew outward past the mouth, swaying gently in current.
            let sway = (view.anim * 1.7).sin() * 0.55;
            let hinge = to_px(POT_HALF + 0.5, 4.8);
            let tip = to_px(POT_HALF + DOOR_DEPTH + 1.2, -1.8 + sway);
            let slat_h = to_px(POT_HALF + 1.5, 4.2);
            let slat_t = to_px(POT_HALF + DOOR_DEPTH + 0.3, -1.5 + sway);
            pb.set_pen(PEN_SHADOW);
            line(pb, (hinge.0 + 1, hinge.1 + 2), (tip.0 + 1, tip.1 + 2), POT_OUTLINE, 0.45);
            pb.set_pen(PEN_METAL);
            line(pb, slat_h, slat_t, mute_open(MESH_MID, true), 0.80);
            line(pb, hinge, tip, mute_open(RUST_MID, true), 0.95);
            pb.plot(hinge.0, hinge.1, mute_open(RUST_HI, true));
        }
    }

    // 5) Rising buoy rope (screen-up `-y`, swaying smoothly) + rising micro-bubbles.
    let ring_x = sx - 1.5;
    let ring_y = sy - POT_HALF * 0.65;
    let rx_i = ring_x.round() as i32;
    let ry_i = ring_y.round() as i32;
    pb.set_pen(PEN_METAL);
    pb.plot(rx_i - 1, ry_i, POT_OUTLINE);
    pb.plot(rx_i + 1, ry_i, POT_OUTLINE);
    pb.plot(rx_i, ry_i, mute_open(RUST_SPEC, is_open));

    pb.set_pen(PEN_WET);
    const ROPE_LEN: i32 = 17;
    for step in 1..=ROPE_LEN {
        let t = step as f32 / ROPE_LEN as f32;
        let sway_x = (view.anim * 1.85 + t * 2.5).sin() * (2.2 * t * t)
            + (view.anim * 1.1).cos() * (0.55 * t);
        let px_r = (ring_x + sway_x).round() as i32;
        let py_r = (ring_y - step as f32).round() as i32;
        let alpha = if t < 0.60 {
            0.94
        } else {
            0.94 * (1.0 - (t - 0.60) / 0.40)
        };
        if alpha <= 0.02 {
            continue;
        }
        blend(pb, px_r + 1, py_r, POT_OUTLINE, alpha * 0.55);
        let c_rope = match step % 3 {
            0 => ROPE_HI,
            1 => ROPE_MID,
            _ => ROPE_LO,
        };
        blend(pb, px_r, py_r, mute_open(c_rope, is_open), alpha);
    }

    if !is_open {
        let n_bub = if view.state == CrabPotState::Shut && (view.caught > 0 || view.caught_rat) {
            3
        } else {
            2
        };
        for k in 0..n_bub {
            let kf = k as f32;
            let ph = (view.anim * (0.52 + 0.12 * kf) + kf * 0.37).rem_euclid(1.0);
            let fade = (ph * PI).sin();
            let bx = (ring_x + (kf - 1.0) * 3.2 + (ph * TAU + kf).sin() * 1.1).round() as i32;
            let by = (ring_y + 3.0 - ph * 14.0).round() as i32;
            blend(
                pb,
                bx,
                by,
                if k == 0 { BUBBLE_HI } else { BUBBLE_COL },
                0.72 * fade,
            );
        }
    }

    pb.set_pen(prev_pen);
}

/// Foreground pass, drawn **after** the rat: overhead wire-mesh roof bars,
/// central rusted crossbar, and rim struts over a caught rat. No-op unless
/// `view.caught_rat` is true so the centre stays clear otherwise.
pub fn draw_crab_pot_front(pb: &mut PixelBuffer, view: &CrabPotView, cam: Vec2) {
    if !view.caught_rat || pb.w == 0 || pb.h == 0 {
        return;
    }
    let f = safe_dir(view.facing);
    let p = f.perp();
    let rattle = pot_rattle(view, f, p);
    let sx = view.pos.x + rattle.x - cam.x;
    let sy = view.pos.y + rattle.y - cam.y;

    let reach = POT_HALF + 4.0;
    if sx + reach < -1.0
        || sx - reach > pb.w as f32 + 1.0
        || sy + reach < -1.0
        || sy - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let prev_pen = pb.pen();
    let fl = f.x * LIGHT.x + f.y * LIGHT.y;
    let pl = p.x * LIGHT.x + p.y * LIGHT.y;

    let to_px = |u: f32, vv: f32| -> (i32, i32) {
        (
            (sx + f.x * u + p.x * vv).round() as i32,
            (sy + f.y * u + p.y * vv).round() as i32,
        )
    };

    const BAR_OFFSETS: [f32; 3] = [-5.4, 0.0, 5.4];

    // Subtle bar shadows over the caught rat.
    pb.set_pen(PEN_SHADOW);
    for &b in &BAR_OFFSETS {
        let a0 = to_px(b, -POT_HALF + 0.8);
        let a1 = to_px(b, POT_HALF - 0.8);
        line(pb, (a0.0 + 1, a0.1 + 1), (a1.0 + 1, a1.1 + 1), POT_OUTLINE, 0.52);
        let b0 = to_px(-POT_HALF + 0.8, b);
        let b1 = to_px(POT_HALF - 0.8, b);
        line(pb, (b0.0 + 1, b0.1 + 1), (b1.0 + 1, b1.1 + 1), POT_OUTLINE, 0.52);
    }

    // Crisscross roof wire-mesh bars leaving clear windows over the rat.
    pb.set_pen(PEN_METAL);
    for &b in &BAR_OFFSETS {
        let cu = shade3(MESH_LO, MESH_MID, MESH_HI, 0.55 + 0.35 * (b / POT_HALF * fl));
        let cv = shade3(MESH_LO, MESH_MID, MESH_HI, 0.55 + 0.35 * (b / POT_HALF * pl));
        line(pb, to_px(b, -POT_HALF + 0.8), to_px(b, POT_HALF - 0.8), cu, 0.94);
        line(pb, to_px(-POT_HALF + 0.8, b), to_px(POT_HALF - 0.8, b), cv, 0.94);
    }

    // Heavy central rusted crossbar right across the centre (u = 0).
    let c0 = to_px(0.0, -POT_HALF + 0.4);
    let c1 = to_px(0.0, POT_HALF - 0.4);
    line(pb, (c0.0 + 1, c0.1 + 1), (c1.0 + 1, c1.1 + 1), POT_OUTLINE, 0.72);
    line(pb, c0, c1, RUST_HI, 0.98);

    // Top rusted perimeter rim bars and corner rivets.
    let corners = [
        to_px(-POT_HALF + 0.5, -POT_HALF + 0.5),
        to_px(POT_HALF - 0.5, -POT_HALF + 0.5),
        to_px(POT_HALF - 0.5, POT_HALF - 0.5),
        to_px(-POT_HALF + 0.5, POT_HALF - 0.5),
    ];
    for (i, &c) in corners.iter().enumerate() {
        line(pb, c, corners[(i + 1) % 4], RUST_MID, 0.95);
        pb.plot(c.0, c.1, RUST_SPEC);
    }

    // Wet specular glints where the roof wires cross the central bar.
    pb.set_pen(PEN_WET);
    for &v_g in &BAR_OFFSETS {
        let gp = to_px(0.0, v_g);
        pb.plot(gp.0, gp.1, MESH_GLINT);
    }

    pb.set_pen(prev_pen);
}

/// Hostile-layer tell, drawn **after** lighting: clear warm amber-coral outline
/// of the [`POT_HALF`] square footprint during [`CrabPotState::Closing`].
pub fn draw_crab_pot_tell(pb: &mut PixelBuffer, view: &CrabPotView, cam: Vec2) {
    if view.state != CrabPotState::Closing || pb.w == 0 || pb.h == 0 {
        return;
    }
    let f = safe_dir(view.facing);
    let p = f.perp();
    let sx = view.pos.x - cam.x;
    let sy = view.pos.y - cam.y;

    let reach = POT_HALF + 3.5;
    if sx + reach < -1.0
        || sx - reach > pb.w as f32 + 1.0
        || sy + reach < -1.0
        || sy - reach > pb.h as f32 + 1.0
    {
        return;
    }

    let prog = view.progress.clamp(0.0, 1.0);
    let pulse = 0.68 + 0.32 * (view.anim * 14.0 + prog * 8.0).sin();
    let warn_col = mix((255, 196, 68), (255, 96, 48), prog);
    let hot_col = mix(warn_col, (255, 244, 186), 0.55 * pulse);
    let sweep_u = lerp(POT_HALF, -POT_HALF, prog);

    let pad = (POT_HALF + 3.0).ceil() as i32;
    let x0 = (sx.floor() as i32 - pad).max(0);
    let x1 = (sx.ceil() as i32 + pad).min(pb.w as i32 - 1);
    let y0 = (sy.floor() as i32 - pad).max(0);
    let y1 = (sy.ceil() as i32 + pad).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    for y in y0..=y1 {
        let dx0 = x0 as f32 - sx;
        let dy = y as f32 - sy;
        let mut u = dx0 * f.x + dy * f.y;
        let mut vv = dx0 * p.x + dy * p.y;
        for x in x0..=x1 {
            let d = sd_box(u, vv, POT_HALF, POT_HALF);
            if (-0.75..=0.75).contains(&d) {
                let on_door = u > POT_HALF - 1.2 && vv.abs() <= 5.5;
                blend(pb, x, y, if on_door { hot_col } else { warn_col }, 0.96);
            } else if (-1.55..=1.55).contains(&d) {
                blend(pb, x, y, (12, 16, 22), 0.78);
            } else if d < -0.75 && u >= sweep_u {
                blend(pb, x, y, warn_col, 0.16 * pulse);
            }
            u += f.x;
            vv += p.x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgb = (18, 30, 38);

    fn sample_view(state: CrabPotState, facing: Vec2) -> CrabPotView {
        CrabPotView {
            pos: Vec2::new(32.0, 32.0),
            facing,
            state,
            progress: 0.5,
            caught_rat: false,
            caught: 0,
            anim: 0.6,
        }
    }

    fn count_painted(pb: &PixelBuffer) -> usize {
        pb.pixels().iter().filter(|&&c| c != BG).count()
    }

    fn lum(c: Rgb) -> u32 {
        c.0 as u32 + c.1 as u32 + c.2 as u32
    }

    #[test]
    fn every_state_and_facing_draws_non_empty() {
        let states = [
            CrabPotState::Set,
            CrabPotState::Closing,
            CrabPotState::Shut,
            CrabPotState::Open,
        ];
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.707, 0.707),
        ];
        for state in states {
            for facing in facings {
                let mut pb = PixelBuffer::new(64, 64);
                pb.clear(BG);
                let v = sample_view(state, facing);
                draw_crab_pot(&mut pb, &v, Vec2::ZERO);
                let n = count_painted(&pb);
                assert!(
                    n > 120,
                    "expected rich crab pot sprite for state={state:?} facing={facing:?}, got {n} px"
                );
            }
        }
    }

    #[test]
    fn closing_door_position_moves_with_progress() {
        let mut pb0 = PixelBuffer::new(64, 64);
        let mut pb_mid = PixelBuffer::new(64, 64);
        let mut pb1 = PixelBuffer::new(64, 64);
        pb0.clear(BG);
        pb_mid.clear(BG);
        pb1.clear(BG);

        let mut v = sample_view(CrabPotState::Closing, Vec2::new(1.0, 0.0));
        v.progress = 0.0;
        draw_crab_pot(&mut pb0, &v, Vec2::ZERO);
        v.progress = 0.5;
        draw_crab_pot(&mut pb_mid, &v, Vec2::ZERO);
        v.progress = 1.0;
        draw_crab_pot(&mut pb1, &v, Vec2::ZERO);

        assert_ne!(pb0.pixels(), pb_mid.pixels());
        assert_ne!(pb_mid.pixels(), pb1.pixels());
        assert_ne!(pb0.pixels(), pb1.pixels());

        // Bright falling door bar centroid in the +x mouth corridor moves inward as progress grows.
        let door_cx = |pb: &PixelBuffer| -> f32 {
            let mut sum = 0.0f32;
            let mut cnt = 0usize;
            for y in 27..=37 {
                for x in 39..=47 {
                    if lum(pb.pixel_at(x, y)) > 350 {
                        sum += x as f32;
                        cnt += 1;
                    }
                }
            }
            assert!(cnt > 0, "expected bright door pixels in mouth corridor");
            sum / cnt as f32
        };

        let c0 = door_cx(&pb0);
        let cm = door_cx(&pb_mid);
        let c1 = door_cx(&pb1);
        assert!(
            c0 > cm && cm > c1 && c0 - c1 > 1.5,
            "closing door should move monotonically inward with progress: c0={c0:.2}, cm={cm:.2}, c1={c1:.2}"
        );
    }

    #[test]
    fn front_pass_draws_over_centre_only_when_caught_rat() {
        for state in [
            CrabPotState::Set,
            CrabPotState::Closing,
            CrabPotState::Shut,
            CrabPotState::Open,
        ] {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear(BG);
            let mut v = sample_view(state, Vec2::new(1.0, 0.0));
            v.caught_rat = false;
            draw_crab_pot_front(&mut pb, &v, Vec2::ZERO);
            let centre_hits = (29..=35)
                .flat_map(|y| (29..=35).map(move |x| (x, y)))
                .filter(|&(x, y)| pb.pixel_at(x, y) != BG)
                .count();
            assert_eq!(
                centre_hits, 0,
                "front pass must not draw over centre when caught_rat=false (state={state:?})"
            );
        }

        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(BG);
        let mut v = sample_view(CrabPotState::Shut, Vec2::new(1.0, 0.0));
        v.caught_rat = true;
        draw_crab_pot_front(&mut pb, &v, Vec2::ZERO);
        let centre_hits = (29..=35)
            .flat_map(|y| (29..=35).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != BG)
            .count();
        assert!(
            centre_hits >= 10,
            "front pass must draw cage bars over the centre when caught_rat=true, got {centre_hits}"
        );
    }

    #[test]
    fn tell_draws_footprint_only_during_closing() {
        for state in [CrabPotState::Set, CrabPotState::Shut, CrabPotState::Open] {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear(BG);
            draw_crab_pot_tell(&mut pb, &sample_view(state, Vec2::new(1.0, 0.0)), Vec2::ZERO);
            assert_eq!(
                count_painted(&pb),
                0,
                "tell must be silent in {state:?}"
            );
        }

        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(BG);
        draw_crab_pot_tell(
            &mut pb,
            &sample_view(CrabPotState::Closing, Vec2::new(1.0, 0.0)),
            Vec2::ZERO,
        );
        let painted = count_painted(&pb);
        assert!(painted > 50, "closing tell must draw POT_HALF outline, got {painted} px");
        // Verify warm pixels appear right on the POT_HALF (9 px) perimeter.
        let edge_px = pb.pixel_at(32 + POT_HALF as usize, 32);
        assert!(
            edge_px.0 > 180 && edge_px.0 > edge_px.2,
            "closing tell edge should be warm amber/coral, got {edge_px:?}"
        );
    }

    #[test]
    fn shut_rattles_when_caught_and_lighting_pens_restore() {
        let mut a = sample_view(CrabPotState::Shut, Vec2::new(1.0, 0.0));
        a.caught = 3;
        a.anim = 0.05;
        let mut b = a;
        b.anim = 0.12;
        let mut pba = PixelBuffer::new(64, 64);
        let mut pbb = PixelBuffer::new(64, 64);
        pba.clear(BG);
        pbb.clear(BG);
        draw_crab_pot(&mut pba, &a, Vec2::ZERO);
        draw_crab_pot(&mut pbb, &b, Vec2::ZERO);
        assert_ne!(pba.pixels(), pbb.pixels(), "Shut pot with caught > 0 should rattle smoothly");

        let mut pb = PixelBuffer::new(64, 64);
        pb.set_materials(true);
        pb.clear(BG);
        pb.set_pen(42);
        draw_crab_pot(&mut pb, &sample_view(CrabPotState::Set, Vec2::new(1.0, 0.0)), Vec2::ZERO);
        assert_eq!(pb.pen(), 42, "draw_crab_pot must restore caller pen");
        assert!(pb.mat().contains(&PEN_METAL), "cage frame/mesh must stamp metal pen");
        assert!(pb.mat().contains(&PEN_WET), "bait/rope must stamp wet sheen pen");
    }
}
