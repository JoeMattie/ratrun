//! The Kitchen's own bodies (`WAVE4_ROSTERS.md`, "Kitchen"): grease-dark
//! shapes, warm walnut wood, baked stoneware and stainless steel against the
//! Kitchen's dark-lit counter tiles.
//!
//! | Kind | Silhouette | Walk frames | Tell / attack pose |
//! |---|---|---|---|
//! | Crumb Ant | three pinched mahogany beads + mandibles gripping a pale square biscuit crumb | 6 articulated tripod-gait legs sweep smoothly; crumb bobs | lunges crumb forward & spreads mandibles on contact |
//! | Fruit Fly | two translucent upright teardrop seed wings over a glossy ruby compound eye & banded rump | continuous wingbeat flutter (shut → splayed V) + tight vertical hover bob | wings flare wide & eye heats on contact |
//! | Egg Timer | brass-bezeled round enamel clock with twin chunky brass key ears & peg feet | weighted waddle (alternating feet, body tilt & bob, ticking hand) | winding: ears crank, casing shivers, red countdown wedge sweeps dial; ringing: ears rattle with golden sparks & glowing face; asleep: slumped dim pewter dial, drooping ears, hand at 6 |
//! | Pepper Mill | tall lathe-turned walnut hourglass pawn with brass collar & fluted steel dome cap | tilting chess-piece rock & hop | winding: cap twists with rotating grip flutes, heats amber & flings twin peppercorn sparks; recoil: cap pops with pepper puff |
//! | Pie Weight | glazed ceramic stoneware ball with kiln-vent eyes over a scalloped, crimped tin skirt | fluted tin crimps & scalloped hem roll continuously while ball sways | heavy compression slam on contact; damaged glaze crack; elite: projecting doubled gold pie rims on both sides + thick double gold shoulder stripes + rotating gold pips |
//! | Sink Crab | broad flat burnt-rust carapace, 6 bent sideways legs, open right pincer & round steel pot-lid left claw | sideways leg-wave scuttle + lateral body rock | curled: legs tuck, pot lid swings flat across `facing` as an arched steel barricade with protruding handle, soft pulsing coral rear exposed (2× cue) |
//!
//! ## Entry points
//!
//! * `draw_<kind>` / `draw_<kind>_lod` for `crumbant`, `fruitfly`, `eggtimer`,
//!   `peppermill`, `pieweight`, `sinkcrab`: the `art_enemy` table rows.
//! * [`draw_egg_ring`]`(pb, e, cam)`: **public**, for the post-lighting hostile
//!   layer in `world.rs`. Call it for every enemy; it paints only for an Egg
//!   Timer that is winding or ringing.
//!
//! Every draw has the `art_enemy::BodyDraw` shape and is listed in
//! `art_enemy::bespoke_body` / `bespoke_lod`. Each stays inside its
//! archetype's `cull_reach` (the screen bound still keys on the archetype).
//! The Egg Timer, the Pepper Mill, the Fruit Fly and the Pie Weight are
//! upright objects, so they are drawn screen-aligned and do not rotate with
//! the facing; the ant and the crab are top-down and do.
//!
//! The Egg Timer's ring is drawn here from the same geometry that hits
//! (`floor_kitchen::egg_ring_hits`, `egg_ring_radius`, `EGG_RING_WIDTH`), by
//! [`draw_egg_ring`] only: the hostile layer calls it after the lighting pass
//! (`world.rs`), so the body draws do not paint it a second time.
//!
//! Render-only: nothing here draws rng or writes simulation state.

use std::f32::consts::{PI, TAU};

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_kitchen as sim;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

// ---- Shared helpers ---------------------------------------------------------

/// Direction *toward* the scene light in screen space (up-left), matching
/// `art_enemy.rs`, `art_rat.rs` and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Light, body and dark edge of one material.
#[derive(Clone, Copy)]
struct Pal {
    hi: Rgb,
    mid: Rgb,
    lo: Rgb,
}

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
fn scale_rgb(c: Rgb, f: f32) -> Rgb {
    let f = f.max(0.0);
    (
        (c.0 as f32 * f).min(255.0) as u8,
        (c.1 as f32 * f).min(255.0) as u8,
        (c.2 as f32 * f).min(255.0) as u8,
    )
}

/// Three-stop directional shading ramp matching `art_enemy::shade3`.
#[inline]
fn shade3(p: Pal, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(p.lo, p.mid, l * (1.0 / 0.62))
    } else {
        mix(p.mid, p.hi, (l - 0.62) * (1.0 / 0.53))
    }
}

/// A body colour with the warm hit flash folded in.
#[inline]
fn tone(fr: &Frame, c: Rgb) -> Rgb {
    if fr.hit <= 0.0 {
        c
    } else {
        let fc = if fr.hit > 0.6 {
            (255, 244, 218)
        } else {
            (255, 218, 152)
        };
        mix(c, fc, (0.30 + fr.hit * 0.56).min(0.86))
    }
}

#[inline]
fn pal(fr: &Frame, hi: Rgb, mid: Rgb, lo: Rgb) -> Pal {
    Pal {
        hi: tone(fr, hi),
        mid: tone(fr, mid),
        lo: tone(fr, lo),
    }
}

/// Exterior silhouette rim colour: crisp dark outline normally (transparent on
/// pure-black test canvases), warm-tinted when hit-flashing.
#[inline]
fn rim_tint(fr: &Frame) -> Rgb {
    if fr.hit > 0.4 {
        (148, 112, 74)
    } else {
        (0, 0, 0)
    }
}

/// Screen pixel of the body-local point `(u forward, v sideways)`.
#[inline]
fn at(fr: &Frame, u: f32, v: f32) -> (i32, i32) {
    (
        (fr.pos.x + fr.f.x * u + fr.p.x * v).round() as i32,
        (fr.pos.y + fr.f.y * u + fr.p.y * v).round() as i32,
    )
}

/// Screen-space position of the body-local point `(u, v)`.
#[inline]
fn place(fr: &Frame, u: f32, v: f32) -> Vec2 {
    Vec2::new(
        fr.pos.x + fr.f.x * u + fr.p.x * v,
        fr.pos.y + fr.f.y * u + fr.p.y * v,
    )
}

/// The body's screen centre.
#[inline]
fn centre(fr: &Frame) -> (i32, i32) {
    (fr.pos.x.round() as i32, fr.pos.y.round() as i32)
}

/// Bounds-checked alpha blend onto the framebuffer.
#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if alpha >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, alpha));
    }
}

/// Darken one pixel (shadows and dark silhouette rims), bounds-checked.
#[inline]
fn dim(pb: &mut PixelBuffer, x: i32, y: i32, keep: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let c = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale_rgb(c, keep));
}

/// Plot a 1 px exterior silhouette rim pixel (darkens floor tiles cleanly).
#[inline]
fn plot_rim(pb: &mut PixelBuffer, x: i32, y: i32, fr: &Frame, strength: f32) {
    blend_px(pb, x, y, rim_tint(fr), strength);
}

/// Blended Bresenham line for articulated legs, pincers and antennae.
fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let ady = (b.1 - a.1).abs();
    if dx <= 1 && ady <= 1 {
        blend_px(pb, a.0, a.1, c, alpha);
        if a != b {
            blend_px(pb, b.0, b.1, c, alpha);
        }
        return;
    }
    let dy = -ady;
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    loop {
        blend_px(pb, x, y, c, alpha);
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

/// Soft elliptical ground drop shadow centred at `(cx, cy)`.
fn soft_shadow(pb: &mut PixelBuffer, cx: f32, cy: f32, rx: f32, ry: f32, depth: f32) {
    let rx = rx.max(0.5);
    let ry = ry.max(0.5);
    let ix = cx.round() as i32;
    let iy = cy.round() as i32;
    let irx = rx.ceil() as i32;
    let iry = ry.ceil() as i32;
    let inv_rx = 1.0 / rx;
    let inv_ry = 1.0 / ry;
    for dy in -iry..=iry {
        let ny = (iy + dy) as f32 - cy;
        let ny2 = (ny * inv_ry) * (ny * inv_ry);
        for dx in -irx..=irx {
            let nx = (ix + dx) as f32 - cx;
            let r2 = (nx * inv_rx) * (nx * inv_rx) + ny2;
            if r2 <= 1.0 {
                let amt = depth * (1.0 - r2 * 0.45);
                dim(pb, ix + dx, iy + dy, 1.0 - amt);
            }
        }
    }
}

/// Smoothly dome-shaded oriented ellipse centred at screen `(cx, cy)` with
/// semi-axes `ra` along unit axis `ax` and `rb` across it, plus a crisp 1 px
/// dark exterior silhouette rim.
fn shaded_ellipse(
    pb: &mut PixelBuffer,
    fr: &Frame,
    center: Vec2,
    ax: Vec2,
    radii: (f32, f32),
    c: Pal,
) {
    let ra = radii.0.max(0.5);
    let rb = radii.1.max(0.5);
    let reach = ra.max(rb).ceil() as i32 + 2;
    let (ix, iy) = (center.x.round() as i32, center.y.round() as i32);
    let inv_ra = 1.0 / ra;
    let inv_rb = 1.0 / rb;
    let rim_pad = 1.0 + 0.9 / ra.min(rb).max(1.0);
    let rim_r2 = rim_pad * rim_pad;
    let rim_c = rim_tint(fr);

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let x = ix + dx;
            let y = iy + dy;
            let fx = x as f32 - center.x;
            let fy = y as f32 - center.y;
            let a = (fx * ax.x + fy * ax.y) * inv_ra;
            let b = (-fx * ax.y + fy * ax.x) * inv_rb;
            let r2 = a * a + b * b;
            if r2 <= 1.0 {
                let dome = (1.0 - r2 * 0.72).clamp(0.0, 1.0);
                let nx = (fx * inv_ra.min(inv_rb)).clamp(-1.0, 1.0);
                let ny = (fy * inv_ra.min(inv_rb)).clamp(-1.0, 1.0);
                let lit = nx * LIGHT.x + ny * LIGHT.y;
                let mut lum = 0.42 + 0.34 * dome + 0.56 * lit;
                if lit > 0.42 && dome > 0.35 {
                    lum += 0.22;
                }
                pb.plot(x, y, shade3(c, lum));
            } else if r2 <= rim_r2 {
                blend_px(pb, x, y, rim_c, 0.58);
            }
        }
    }
}

/// Smoothly dome-shaded screen-aligned ball with a crisp exterior rim.
#[inline]
fn shaded_ball(pb: &mut PixelBuffer, fr: &Frame, cx: f32, cy: f32, r: f32, c: Pal) {
    shaded_ellipse(pb, fr, Vec2::new(cx, cy), Vec2::new(1.0, 0.0), (r, r), c);
}

/// One horizontal row of a lathe-turned vertical body: dark 1 px outline just
/// outside both ends, smooth cylindrical left-lit shading across `x0..=x1`.
fn turned_row(pb: &mut PixelBuffer, fr: &Frame, x0: i32, x1: i32, y: i32, c: Pal) {
    plot_rim(pb, x0 - 1, y, fr, 0.64);
    plot_rim(pb, x1 + 1, y, fr, 0.64);
    if x0 >= x1 {
        pb.plot(x0, y, c.mid);
        return;
    }
    let span = (x1 - x0) as f32;
    for x in x0..=x1 {
        let t = (x - x0) as f32 / span; // 0 = left (lit), 1 = right (shadow)
        // Cylindrical specular highlight sits slightly inset from the left edge.
        let spec = (1.0 - ((t - 0.22) * 3.8).abs()).clamp(0.0, 1.0);
        let lum = 1.04 - t * 0.92 + spec * 0.28;
        pb.plot(x, y, shade3(c, lum));
    }
}

// ---- Crumb Ant (V Skitterer, r 2.5) -----------------------------------------

const ANT_HI: Rgb = (214, 146, 86);
const ANT_MID: Rgb = (122, 70, 38);
const ANT_LO: Rgb = (48, 24, 14);
const ANT_KNEE: Rgb = (176, 112, 62);
const ANT_EYE: Rgb = (255, 208, 112);
const CRUMB_HI: Rgb = (255, 248, 214);
const CRUMB_MID: Rgb = (238, 208, 146);
const CRUMB_LO: Rgb = (158, 116, 62);
const CRUMB_CRUST: Rgb = (122, 82, 40);

/// Three grease-dark mahogany beads with pinched waists, six articulated
/// marching legs, and a toasted square biscuit crumb clamped in its jaws.
pub(crate) fn draw_crumbant(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 2.5;
    let body = pal(fr, ANT_HI, ANT_MID, ANT_LO);
    let leg_lo = tone(fr, ANT_LO);
    let leg_knee = tone(fr, ANT_KNEE);
    let lunge = if e.contact_cd > 0.25 { 0.55 * s } else { 0.0 };

    // Ground shadow beneath the three beads and crumb.
    let sh_c = place(fr, 0.4 * s, 0.0);
    soft_shadow(pb, sh_c.x + 1.0, sh_c.y + 1.0, 3.6 * s, 2.2 * s, 0.34);

    // Six articulated tripod-gait legs (hip -> bright knee -> foot tap).
    for (i, u_hip) in [0.95f32, -0.25, -1.55].into_iter().enumerate() {
        let phase = fr.anim * 1.15 + i as f32 * 2.1;
        let sw = phase.sin() * 0.95 * s;
        let lift = phase.cos().max(0.0) * 0.35 * s;
        for (side, swing) in [(-1.0f32, sw), (1.0, -sw)] {
            let hip = at(fr, u_hip * s, side * 0.75 * s);
            let knee = at(fr, u_hip * s + swing * 0.45, side * (2.05 * s + lift));
            let foot = at(fr, u_hip * s + swing - 0.35 * s, side * 3.05 * s);
            dim(pb, foot.0 + 1, foot.1 + 1, 0.74);
            line_blend(pb, hip, knee, leg_knee, 0.92);
            line_blend(pb, knee, foot, leg_lo, 0.92);
        }
    }

    // Bent antennae sweeping out ahead of the head.
    let ant_wave = (fr.anim * 0.85).sin() * 0.38 * s;
    let ant_c = tone(fr, ANT_KNEE);
    for side in [-1.0f32, 1.0] {
        let a0 = at(fr, 2.1 * s, side * 0.75 * s);
        let a1 = at(fr, 2.9 * s + lunge * 0.4, side * (1.85 * s + ant_wave * side));
        line_blend(pb, a0, a1, ant_c, 0.80);
    }

    // Three distinct beads with crisp pinched petiole waists:
    // 1) Gaster (plump rear abdomen with a subtle dark chitin band).
    let gaster = place(fr, -2.55 * s, 0.0);
    shaded_ellipse(pb, fr, gaster, fr.f, (1.65 * s, 1.35 * s), body);
    let band_a = at(fr, -2.75 * s, -0.75 * s);
    let band_b = at(fr, -2.75 * s, 0.75 * s);
    line_blend(pb, band_a, band_b, tone(fr, ANT_LO), 0.62);

    // 2) Pinched narrow thorax (middle bead).
    let thorax = place(fr, -0.15 * s, 0.0);
    shaded_ellipse(pb, fr, thorax, fr.f, (0.95 * s, 0.80 * s), body);

    // 3) Head bead (front).
    let head = place(fr, (1.75 * s) + lunge * 0.35, 0.0);
    shaded_ellipse(pb, fr, head, fr.f, (1.05 * s, 1.05 * s), body);

    // Warm amber eye glints so the ant's face survives low ambient light.
    let eye = if fr.hit > 0.65 {
        (36, 20, 14)
    } else {
        ANT_EYE
    };
    for side in [-1.0f32, 1.0] {
        let ep = at(fr, 2.05 * s + lunge * 0.35, side * 0.72 * s);
        pb.plot(ep.0, ep.1, eye);
    }

    // Dark mandibles clamping the sides of the square crumb.
    let jaw_spread = if lunge > 0.0 { 1.35 } else { 1.05 };
    for side in [-1.0f32, 1.0] {
        let j0 = at(fr, 2.35 * s + lunge * 0.35, side * 0.65 * s);
        let j1 = at(fr, 3.25 * s + lunge, side * jaw_spread * s);
        line_blend(pb, j0, j1, leg_lo, 0.96);
    }

    // The square crumb: crisp toasted 3x3 biscuit held ahead of the jaws,
    // bobbing gently sideways with the march.
    let bob_v = (fr.anim * 1.15).sin() * 0.30 * s;
    let (qx, qy) = at(fr, 3.95 * s + lunge, bob_v);
    for dy in -2..=2i32 {
        for dx in -2..=2i32 {
            if dx.abs() == 2 && dy.abs() == 2 {
                continue;
            }
            if dx.abs() == 2 || dy.abs() == 2 {
                plot_rim(pb, qx + dx, qy + dy, fr, 0.56);
            }
        }
    }
    let c_hi = tone(fr, CRUMB_HI);
    let c_mid = tone(fr, CRUMB_MID);
    let c_lo = tone(fr, CRUMB_LO);
    let c_crust = tone(fr, CRUMB_CRUST);
    pb.plot(qx - 1, qy - 1, c_hi);
    pb.plot(qx, qy - 1, c_hi);
    pb.plot(qx + 1, qy - 1, c_mid);
    pb.plot(qx - 1, qy, c_hi);
    pb.plot(qx, qy, c_mid);
    pb.plot(qx + 1, qy, c_lo);
    pb.plot(qx - 1, qy + 1, c_mid);
    pb.plot(qx, qy + 1, c_lo);
    pb.plot(qx + 1, qy + 1, c_crust);
}

/// Far LOD: three pinched dark-rimmed beads, swapping feet and the pale crumb.
pub(crate) fn draw_crumbant_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    _e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let s = fr.r / 2.5;
    let swap = (fr.anim * 1.15).sin() > 0.0;
    let side = if swap { 1.75 } else { -1.75 };
    let (fx, fy) = at(fr, 0.2 * s, side * s);
    pb.plot(fx, fy, tone(fr, ANT_KNEE));
    let (bx, by) = at(fr, -1.6 * s, -side * s);
    pb.plot(bx, by, tone(fr, ANT_LO));

    let (gx, gy) = at(fr, -2.3 * s, 0.0);
    dim(pb, gx + 1, gy + 1, 0.70);
    pb.plot(gx, gy, tone(fr, ANT_MID));
    pb.plot(gx - 1, gy, tone(fr, ANT_LO));
    let (tx, ty) = centre(fr);
    pb.plot(tx, ty, tone(fr, ANT_LO));
    let (hx, hy) = at(fr, 1.7 * s, 0.0);
    pb.plot(hx, hy, tone(fr, ANT_HI));

    // Crisp 2x2 pale square crumb at the front so the column reads at 2x zoom.
    let (qx, qy) = at(fr, 3.6 * s, 0.0);
    pb.plot(qx, qy, tone(fr, CRUMB_HI));
    pb.plot(qx + 1, qy, tone(fr, CRUMB_MID));
    pb.plot(qx, qy + 1, tone(fr, CRUMB_MID));
    pb.plot(qx + 1, qy + 1, tone(fr, CRUMB_LO));
}

// ---- Fruit Fly (R Bat, r 2.0) -----------------------------------------------

const FLY_EYE_HI: Rgb = (255, 168, 134);
const FLY_EYE_MID: Rgb = (228, 48, 40);
const FLY_EYE_LO: Rgb = (102, 16, 20);
const FLY_GLINT: Rgb = (255, 240, 224);
const FLY_WING_HI: Rgb = (248, 250, 238);
const FLY_WING_MID: Rgb = (206, 222, 210);
const FLY_WING_VEIN: Rgb = (96, 108, 104);
const FLY_BODY_LO: Rgb = (44, 28, 20);
const FLY_BODY_STRIPE: Rgb = (168, 118, 56);

/// Two upright translucent teardrop seed wings over a glossy round red eye and
/// banded fly abdomen, hovering in a tight vertical bob.
pub(crate) fn draw_fruitfly(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 2.0;
    let (cx0, cy0) = centre(fr);
    let biting = e.contact_cd > 0.25;

    // Tight vertical hover bob + faint weave jitter.
    let bob_f = (fr.anim * 1.45).sin();
    let bob_y = (bob_f * 1.15).round() as i32;
    let bob_x = ((fr.anim * 0.95).cos() * 0.55).round() as i32;
    let cx = cx0 + bob_x;
    let cy = cy0 + bob_y;

    // Elevated ground shadow shrinks slightly as the fly rises.
    let sh_rx = (2.2 - bob_f * 0.35) * s;
    soft_shadow(pb, cx0 as f32 + 0.5, cy0 as f32 + 4.4 * s, sh_rx, 1.15 * s, 0.34);

    // Banded Drosophila rump tucked slightly behind/below the big red eye,
    // shifted opposite horizontal facing so heading still reads at a glance.
    let back_x = if fr.f.x >= 0.0 { -1 } else { 1 };
    let rump_x = cx + back_x;
    let rump_y = cy + 2;
    plot_rim(pb, rump_x + back_x, rump_y + 1, fr, 0.62);
    pb.plot(rump_x - 1, rump_y, tone(fr, FLY_BODY_LO));
    pb.plot(rump_x, rump_y, tone(fr, FLY_BODY_STRIPE));
    pb.plot(rump_x + 1, rump_y, tone(fr, FLY_BODY_LO));
    pb.plot(rump_x + back_x, rump_y + 1, tone(fr, FLY_BODY_LO));
    // Two tiny dangling dark fly legs beneath the thorax.
    let leg_sw = if (fr.anim * 1.8).sin() > 0.0 { 1 } else { 0 };
    pb.plot(cx - 1, cy + 3, tone(fr, FLY_BODY_LO));
    pb.plot(cx + leg_sw, cy + 3, tone(fr, FLY_BODY_LO));

    // Two upright teardrop/seed wings rising from the top of the thorax.
    // Continuous multi-phase beat from shut (upright) to wide V-splay.
    let beat = if biting {
        1.0
    } else {
        0.5 + 0.5 * (fr.anim * 2.5).sin()
    };
    let w_hi = tone(fr, FLY_WING_HI);
    let w_mid = tone(fr, FLY_WING_MID);
    let w_vein = tone(fr, FLY_WING_VEIN);
    let tall = (4.6 * s).round().max(3.0) as i32;

    for side in [-1i32, 1] {
        let sf = side as f32;
        // Splay angle shifts from nearly vertical (0.28) to open seed V (0.95).
        let splay = 0.28 + beat * 0.68;
        for k in 1..=tall {
            let t = k as f32 / tall as f32; // 0 = hinge, 1 = rounded seed tip
            // Seed envelope: narrow at hinge, widest at t ~ 0.65, tapered tip.
            let belly = (t * PI).sin().powf(0.75);
            let half_w = (0.35 + 0.95 * belly) * s;
            let wx_c = cx as f32 + sf * (0.75 + k as f32 * splay);
            let wy = cy - 1 - k;
            let x_min = (wx_c - half_w).round() as i32;
            let x_max = (wx_c + half_w).round() as i32;

            // Subtle dark edge behind the outer seed margin so wings pop on pale tiles.
            plot_rim(pb, if side < 0 { x_min - 1 } else { x_max + 1 }, wy, fr, 0.44);
            for x in x_min..=x_max {
                let outer = (side < 0 && x == x_min) || (side > 0 && x == x_max);
                let inner = (side < 0 && x == x_max) || (side > 0 && x == x_min);
                if outer {
                    blend_px(pb, x, wy, w_hi, 0.94);
                } else if inner && k > 1 && k < tall {
                    blend_px(pb, x, wy, w_vein, 0.78);
                } else {
                    blend_px(pb, x, wy, w_mid, 0.72);
                }
            }
        }
        // Motion-blur ghost pip on peak downstroke so fast buzz reads immediately.
        if beat > 0.72 {
            let gx = (cx as f32 + sf * (0.9 + tall as f32 * (splay + 0.28))).round() as i32;
            blend_px(pb, gx, cy - tall + 1, w_hi, 0.42);
        }
    }

    // Oversized faceted ruby-red compound eye (the fly's visual anchor).
    let eye_pal = if biting {
        pal(fr, (255, 214, 168), (248, 72, 48), FLY_EYE_LO)
    } else {
        pal(fr, FLY_EYE_HI, FLY_EYE_MID, FLY_EYE_LO)
    };
    shaded_ball(pb, fr, cx as f32, cy as f32, 2.25 * s, eye_pal);
    // Crisp specular eye glint at upper-left (survives low ambient pooled light)
    // and one dark compound-facet dot at lower-right.
    pb.plot(cx - 1, cy - 1, tone(fr, FLY_GLINT));
    pb.plot(cx + 1, cy + 1, tone(fr, FLY_EYE_LO));
}

/// Far LOD: dark-rimmed ruby eye with specular glint and opening seed wings.
pub(crate) fn draw_fruitfly_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    _e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let (cx, cy0) = centre(fr);
    let bob = if (fr.anim * 1.45).sin() > 0.0 { 0 } else { -1 };
    let cy = cy0 + bob;
    let open = if (fr.anim * 2.5).sin() > 0.0 { 1 } else { 0 };
    dim(pb, cx, cy0 + 3, 0.70);

    // Seed-wing buds (2 px tall, opening into a clear V).
    pb.plot(cx - 1 - open, cy - 3, tone(fr, FLY_WING_HI));
    pb.plot(cx - 2 - open, cy - 2, tone(fr, FLY_WING_MID));
    pb.plot(cx - 1, cy - 2, tone(fr, FLY_WING_HI));
    pb.plot(cx + 1 + open, cy - 3, tone(fr, FLY_WING_HI));
    pb.plot(cx + 2 + open, cy - 2, tone(fr, FLY_WING_MID));
    pb.plot(cx + 1, cy - 2, tone(fr, FLY_WING_VEIN));

    // Compact 3x3 ruby eye with bright glint and dark underside.
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            let col = if dx == -1 && dy == -1 {
                FLY_GLINT
            } else if dx + dy <= -1 {
                FLY_EYE_HI
            } else if dx + dy >= 1 {
                FLY_EYE_LO
            } else {
                FLY_EYE_MID
            };
            pb.plot(cx + dx, cy + dy, tone(fr, col));
        }
    }
    pb.plot(cx, cy + 2, tone(fr, FLY_BODY_LO));
}

// ---- Egg Timer (N, r 5.0) ---------------------------------------------------

const EGG_HI: Rgb = (255, 250, 236);
const EGG_MID: Rgb = (224, 214, 194);
const EGG_LO: Rgb = (134, 122, 106);
const EGG_BEZEL_HI: Rgb = (255, 232, 146);
const EGG_BEZEL_MID: Rgb = (206, 152, 62);
const EGG_BEZEL_LO: Rgb = (102, 66, 26);
const EGG_SLEEP_BEZEL_HI: Rgb = (164, 148, 120);
const EGG_SLEEP_BEZEL_MID: Rgb = (114, 102, 84);
const EGG_SLEEP_BEZEL_LO: Rgb = (60, 52, 44);
const EGG_HAND: Rgb = (224, 42, 38);
const EGG_HAND_HI: Rgb = (255, 134, 108);
const EGG_INK: Rgb = (42, 34, 32);
const EGG_WIND_WEDGE: Rgb = (242, 132, 74);
const RING_INNER: Rgb = (232, 64, 44);
const RING_HOT: Rgb = (255, 108, 56);
const RING_EDGE: Rgb = (255, 234, 156);
const RING_TELL: Rgb = (255, 150, 90);
const RING_TELL_HOT: Rgb = (255, 218, 128);

/// The dial's enamel face colours for the current phase.
fn egg_face(fr: &Frame, e: &Enemy) -> Pal {
    match e.w4.phase {
        // Winding: warms slightly with building tension.
        sim::EGG_PHASE_WIND => {
            let p = (1.0 - e.w4.t / sim::EGG_WIND_SECS).clamp(0.0, 1.0);
            pal(
                fr,
                mix(EGG_HI, (255, 246, 214), p * 0.6),
                mix(EGG_MID, (246, 206, 154), p * 0.55),
                mix(EGG_LO, (166, 106, 68), p * 0.55),
            )
        }
        // Ringing: incandescent glowing clock face.
        sim::EGG_PHASE_TICK => pal(fr, (255, 254, 244), (255, 224, 156), (192, 104, 52)),
        // Asleep: cool muted pewter punish window.
        sim::EGG_PHASE_SLEEP => pal(fr, (172, 170, 166), (132, 130, 128), (74, 72, 74)),
        _ => pal(fr, EGG_HI, EGG_MID, EGG_LO),
    }
}

/// The outer brass bezel colours for the current phase.
fn egg_bezel(fr: &Frame, e: &Enemy) -> Pal {
    match e.w4.phase {
        sim::EGG_PHASE_TICK => pal(fr, (255, 250, 204), (252, 198, 84), (150, 88, 30)),
        sim::EGG_PHASE_SLEEP => pal(
            fr,
            EGG_SLEEP_BEZEL_HI,
            EGG_SLEEP_BEZEL_MID,
            EGG_SLEEP_BEZEL_LO,
        ),
        _ => pal(fr, EGG_BEZEL_HI, EGG_BEZEL_MID, EGG_BEZEL_LO),
    }
}

/// Angle (radians clockwise from 12 o'clock) of the Egg Timer's hand.
fn egg_hand_angle(fr: &Frame, e: &Enemy) -> f32 {
    match e.w4.phase {
        // Winding: one full clockwise sweep back to twelve as the tell runs out.
        sim::EGG_PHASE_WIND => (1.0 - e.w4.t / sim::EGG_WIND_SECS).clamp(0.0, 1.0) * TAU,
        // Ringing: vibrating sharply at twelve.
        sim::EGG_PHASE_TICK => (e.w4.t * 44.0).sin() * 0.14,
        // Asleep: fallen limp to six.
        sim::EGG_PHASE_SLEEP => PI,
        // Walking: ticking steadily around the dial.
        _ => (fr.anim * 0.45).rem_euclid(TAU),
    }
}

/// Round brass-bezeled clock, chunky key ears, waddles until the dial stops.
pub(crate) fn draw_eggtimer(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let (cx0, cy0) = centre(fr);
    let phase = e.w4.phase;
    let walking = phase == sim::EGG_PHASE_WALK;
    let winding = phase == sim::EGG_PHASE_WIND;
    let ringing = phase == sim::EGG_PHASE_TICK;
    let sleeping = phase == sim::EGG_PHASE_SLEEP;

    // Smooth 4-phase waddle while walking; shiver while winding; rattle while
    // ringing; slump 1 px down while asleep.
    let waddle = if walking { (fr.anim * 0.9).sin() } else { 0.0 };
    let left_lift = if walking { (-waddle).max(0.0) } else { 0.0 };
    let right_lift = if walking { waddle.max(0.0) } else { 0.0 };
    let lean_x = if walking {
        (waddle * 1.15).round() as i32
    } else if winding {
        if (e.w4.t * 32.0).sin() > 0.0 { 1 } else { -1 }
    } else if ringing {
        if (e.w4.t * 48.0).sin() > 0.0 { 1 } else { -1 }
    } else {
        0
    };
    let bob_y = if walking {
        -((waddle.abs() * 0.9).round() as i32)
    } else if ringing {
        if (e.w4.t * 48.0).cos() > 0.0 { -1 } else { 0 }
    } else if sleeping {
        1
    } else {
        0
    };

    let cx = cx0 + lean_x;
    let cy = cy0 - 1 + bob_y;
    let outer_r = 4.9 * s;
    let face_r = 3.7 * s;
    let rad = outer_r.round() as i32;

    soft_shadow(pb, cx0 as f32 + 0.5, cy0 as f32 + rad as f32 + 0.5, 5.2 * s, 2.0 * s, 0.36);

    let bezel = egg_bezel(fr, e);
    let face = egg_face(fr, e);

    // Sturdy angled brass peg feet underneath the clock.
    let foot_y_base = cy0 + rad;
    for (side, lift) in [(-1i32, left_lift), (1, right_lift)] {
        let fx = cx + side * 2;
        let fy = foot_y_base - (lift * 1.4).round() as i32;
        plot_rim(pb, fx + side, fy + 1, fr, 0.58);
        pb.plot(fx, fy - 1, bezel.lo);
        pb.plot(fx, fy, bezel.mid);
        pb.plot(fx + side, fy, if side < 0 { bezel.hi } else { bezel.lo });
    }

    // Twin chunky brass key ears (alarm bells / butterfly wind-up keys) on top.
    let wind_prog = if winding {
        (1.0 - e.w4.t / sim::EGG_WIND_SECS).clamp(0.0, 1.0)
    } else {
        0.0
    };
    for side in [-1i32, 1] {
        let droop = if sleeping { 1 } else { 0 };
        let rattle = if ringing {
            if (e.w4.t * 44.0).sin() * side as f32 > 0.0 { -1 } else { 0 }
        } else if winding {
            if (e.w4.t * 22.0).sin() * side as f32 > 0.0 { -1 } else { 0 }
        } else if walking {
            if waddle * side as f32 > 0.4 { -1 } else { 0 }
        } else {
            0
        };
        let ex = cx + side * (3 - droop);
        let ey = cy - rad - 1 + droop + rattle;
        // Stem joining ear to bezel.
        pb.plot(ex - side, ey + 2, bezel.lo);
        pb.plot(ex, ey + 1, bezel.lo);
        // Key loop: visibly narrows/widens while cranking during wind-up!
        let flat_turn = winding && ((e.w4.t * 18.0 + side as f32).cos().abs() < 0.35);
        plot_rim(pb, ex, ey - 2, fr, 0.55);
        if flat_turn {
            pb.plot(ex, ey - 1, bezel.hi);
            pb.plot(ex, ey, bezel.mid);
        } else {
            plot_rim(pb, ex - 2, ey, fr, 0.50);
            plot_rim(pb, ex + 2, ey, fr, 0.50);
            pb.plot(ex - 1, ey - 1, bezel.hi);
            pb.plot(ex, ey - 1, bezel.hi);
            pb.plot(ex + 1, ey - 1, bezel.mid);
            pb.plot(ex - 1, ey, bezel.mid);
            pb.plot(ex + 1, ey, bezel.lo);
            // Tiny hollow loop eye in the middle of the key ear.
            pb.plot(ex, ey, tone(fr, EGG_INK));
        }
        // Rattle alarm arcs flickering outside the ears while ringing!
        if ringing && (e.w4.t * 36.0 + side as f32).sin() > -0.2 {
            let sx = ex + side * 2;
            pb.plot(sx, ey - 1, (255, 242, 168));
            pb.plot(sx + side, ey, (255, 198, 92));
        }
    }

    // Sculpted clock casing: dark exterior rim -> metallic brass bezel ->
    // inner dark recess -> domed cream enamel dial face!
    let reach = rad + 2;
    let rim_c = rim_tint(fr);
    let ink = tone(fr, EGG_INK);
    let wedge_col = tone(fr, EGG_WIND_WEDGE);
    let hand_ang = egg_hand_angle(fr, e);

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let x = cx + dx;
            let y = cy + dy;
            let fx = dx as f32;
            let fy = dy as f32;
            let d = (fx * fx + fy * fy).sqrt();
            if d > outer_r + 1.05 {
                continue;
            }
            if d > outer_r {
                blend_px(pb, x, y, rim_c, 0.68);
                continue;
            }
            let nx = (fx / outer_r).clamp(-1.0, 1.0);
            let ny = (fy / outer_r).clamp(-1.0, 1.0);
            let lit = nx * LIGHT.x + ny * LIGHT.y;
            if d > face_r {
                // Brushed brass outer bezel ring.
                let lum = 0.52 + 0.58 * lit + if lit > 0.45 { 0.22 } else { 0.0 };
                pb.plot(x, y, shade3(bezel, lum));
            } else {
                // Enamel dial face with subtle inner shadow at top edge.
                let dome = (1.0 - (d / face_r) * (d / face_r) * 0.45).clamp(0.0, 1.0);
                let mut col = shade3(face, 0.48 + 0.34 * dome + 0.32 * lit);
                // While winding, shade the swept countdown sector warm red-amber!
                if winding && d > 0.8 && wind_prog > 0.04 {
                    let ang = fx.atan2(-fy).rem_euclid(TAU);
                    if ang <= hand_ang {
                        col = mix(col, wedge_col, 0.46 + 0.24 * wind_prog);
                    }
                }
                pb.plot(x, y, col);
            }
        }
    }

    // Crisp 12-3-6-9 hour ticks; 12 o'clock is marked red so the countdown goal reads!
    let tick = (face_r - 0.5).round().max(2.0) as i32;
    pb.plot(cx, cy - tick, tone(fr, EGG_HAND));
    pb.plot(cx, cy + tick, ink);
    pb.plot(cx - tick, cy, ink);
    pb.plot(cx + tick, cy, ink);

    // Tapered red clock hand + brass centre boss.
    let (hx, hy) = (hand_ang.sin(), -hand_ang.cos());
    let hand_c = if sleeping {
        tone(fr, (144, 72, 68))
    } else {
        tone(fr, EGG_HAND)
    };
    let hand_hi = tone(fr, EGG_HAND_HI);
    let hand_len = (face_r - 0.4).round().max(1.0) as i32;
    for k in 1..=hand_len {
        let px = cx + (hx * k as f32).round() as i32;
        let py = cy + (hy * k as f32).round() as i32;
        pb.plot(px, py, if k == 1 && !sleeping { hand_hi } else { hand_c });
    }
    pb.plot(cx, cy, if sleeping { ink } else { bezel.hi });
}

/// Far LOD: dark-rimmed brass-bezeled dial, two ear buds, feet and red hand.
pub(crate) fn draw_eggtimer_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let s = fr.r / 5.0;
    let (cx0, cy0) = centre(fr);
    let lean = if e.w4.phase == sim::EGG_PHASE_WALK && (fr.anim * 0.9).sin() > 0.0 {
        1
    } else {
        0
    };
    let cx = cx0 + lean;
    let cy = cy0 + i32::from(e.w4.phase == sim::EGG_PHASE_SLEEP);
    let rad = (3.4 * s).round().max(2.0) as i32;
    let bezel = egg_bezel(fr, e);
    let face = egg_face(fr, e);

    dim(pb, cx + rad, cy + rad, 0.70);
    // Brass ears & feet.
    pb.plot(cx - 2, cy - rad - 1, bezel.hi);
    pb.plot(cx + 2, cy - rad - 1, bezel.lo);
    pb.plot(cx - 1, cy + rad + 1, bezel.lo);
    pb.plot(cx + 1, cy + rad + 1, bezel.lo);

    shaded_ball(pb, fr, cx as f32, cy as f32, 3.4 * s, bezel);
    shaded_ball(pb, fr, cx as f32, cy as f32, 2.3 * s, face);

    let ang = egg_hand_angle(fr, e);
    let hand = tone(fr, EGG_HAND);
    pb.plot(cx, cy, hand);
    pb.plot(
        cx + (ang.sin() * 1.8).round() as i32,
        cy + (-ang.cos() * 1.8).round() as i32,
        hand,
    );
}

/// The Egg Timer's footprint for the hostile layer: the dotted tell while it
/// winds and the live band while the ring is out. `cam` is the world camera.
pub fn draw_egg_ring(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    draw_egg_ring_at(pb, Vec2::new(e.pos.x - cam.x, e.pos.y - cam.y), e);
}

/// The footprint around the screen-space centre `c`.
///
/// The live band is every pixel `floor_kitchen::egg_ring_hits` would hit, so
/// what is drawn is what bites. The tell is the band's first and last outline,
/// dotted, with the dots closing up and brightening as the wind runs out.
fn draw_egg_ring_at(pb: &mut PixelBuffer, c: Vec2, e: &Enemy) {
    if let Some((_, progress)) = sim::egg_tell_of(e) {
        let outer = sim::EGG_RING_END + sim::EGG_RING_WIDTH * 0.5;
        let inner = (sim::EGG_RING_START - sim::EGG_RING_WIDTH * 0.5).max(1.0);
        // 48 dots round the outer extent; every third early, every second
        // late, all of them in the last fifth.
        let skip = if progress > 0.8 {
            1
        } else if progress > 0.45 {
            2
        } else {
            3
        };
        let tell_col = mix(RING_TELL, RING_TELL_HOT, progress * 0.75);
        for k in 0..48 {
            if k % skip != 0 {
                continue;
            }
            let a = k as f32 / 48.0 * TAU;
            let (sn, cs) = a.sin_cos();
            pb.plot(
                (c.x + cs * outer).round() as i32,
                (c.y + sn * outer).round() as i32,
                if k % 6 == 0 { RING_TELL_HOT } else { tell_col },
            );
            if k % 4 == 0 {
                pb.plot(
                    (c.x + cs * inner).round() as i32,
                    (c.y + sn * inner).round() as i32,
                    tell_col,
                );
            }
        }
        return;
    }
    let Some((_, radius)) = sim::egg_ring_of(e) else {
        return;
    };
    let half = sim::EGG_RING_WIDTH * 0.5;
    let reach = (radius + half).ceil() as i32 + 1;
    let (ix, iy) = (c.x.round() as i32, c.y.round() as i32);
    let origin = Vec2::new(0.0, 0.0);
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let p = Vec2::new((ix + dx) as f32 - c.x, (iy + dy) as f32 - c.y);
            if !sim::egg_ring_hits(origin, radius, p) {
                continue;
            }
            // Three-band fiery wave: crimson trailing edge -> molten orange
            // core -> blazing cream-gold leading edge.
            let d = (p.x * p.x + p.y * p.y).sqrt();
            let col = if d > radius + half - 1.2 {
                RING_EDGE
            } else if d < radius - half + 1.0 {
                RING_INNER
            } else {
                RING_HOT
            };
            pb.plot(ix + dx, iy + dy, col);
        }
    }
}

// ---- Pepper Mill (V Spitter, r 3.0) -----------------------------------------

const MILL_HI: Rgb = (242, 192, 128);
const MILL_MID: Rgb = (166, 102, 52);
const MILL_LO: Rgb = (68, 36, 18);
const MILL_RING_HI: Rgb = (255, 224, 134);
const MILL_RING_MID: Rgb = (206, 152, 62);
const MILL_RING_LO: Rgb = (108, 70, 26);
const MILL_CAP_HI: Rgb = (248, 250, 255);
const MILL_CAP_MID: Rgb = (184, 194, 210);
const MILL_CAP_LO: Rgb = (82, 90, 108);
const MILL_PEPPER: Rgb = (46, 32, 26);
/// Seconds before the shot that the cap starts twisting (matches Spitter tell).
const MILL_TELL_SECS: f32 = 0.75;
/// Just-fired recoil window (shots reset `fire_cd` to 2.2 s).
const MILL_RECOIL_START: f32 = 2.02;

/// Twist charge `0..1` while winding up for a split shot, else `0`.
/// A mill that is primed behind cover (`fire_cd <= 0`) stands still.
#[inline]
fn mill_charge(e: &Enemy) -> f32 {
    if e.fire_cd > MILL_TELL_SECS || e.fire_cd <= 0.0 {
        0.0
    } else {
        ((MILL_TELL_SECS - e.fire_cd) / MILL_TELL_SECS).clamp(0.0, 1.0)
    }
}

/// How far the cap is twisted this frame (`-1`, `0` or `+1` pixels).
#[inline]
fn mill_twist(e: &Enemy) -> i32 {
    if mill_charge(e) <= 0.0 {
        return 0;
    }
    if (e.fire_cd * 26.0).sin() >= 0.0 {
        1
    } else {
        -1
    }
}

/// Waisted lathe-turned walnut pawn that rocks as it walks and twists its
/// fluted steel cap before firing twin split rays.
pub(crate) fn draw_peppermill(
    pb: &mut PixelBuffer,
    fr: &Frame,
    e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let (cx0, cy0) = centre(fr);
    let wood = pal(fr, MILL_HI, MILL_MID, MILL_LO);
    let brass = pal(fr, MILL_RING_HI, MILL_RING_MID, MILL_RING_LO);
    let charge = mill_charge(e);
    let twist = mill_twist(e);
    let telling = twist != 0;
    let recoil = e.fire_cd > MILL_RECOIL_START;

    // Tilting chess-piece walk gait (4-phase lean & tiny mid-stride hop).
    let gait = (fr.anim * 0.95).sin();
    let tilt = if telling {
        0
    } else {
        (gait * 1.1).round() as i32
    };
    let hop = if !telling && gait.abs() > 0.65 { -1 } else { 0 };
    let cx = cx0;
    let cy = cy0 + hop;

    soft_shadow(pb, cx0 as f32 + 0.5, cy0 as f32 + 5.2, 3.8, 1.4, 0.36);

    // Pedestal base (shifts slightly opposite the upper tilt like a rocking pawn).
    let base_x = cx - tilt.signum() * i32::from(tilt != 0 && hop == 0);
    // Bottom rim outline beneath pedestal.
    for x in (base_x - 3)..=(base_x + 3) {
        plot_rim(pb, x, cy + 5, fr, 0.58);
    }
    turned_row(pb, fr, base_x - 3, base_x + 3, cy + 4, brass);
    // Lower turned walnut belly.
    turned_row(pb, fr, cx - 2, cx + 2, cy + 3, wood);
    turned_row(pb, fr, cx - 2, cx + 2, cy + 2, wood);
    // Pronounced pinched hourglass waist (3 px wide, dark notch shading).
    let waist_x = cx + (tilt / 2);
    turned_row(pb, fr, waist_x - 1, waist_x + 1, cy + 1, wood);
    turned_row(pb, fr, waist_x - 1, waist_x + 1, cy, wood);
    // Upper turned walnut shoulder.
    let top_x = cx + tilt;
    turned_row(pb, fr, top_x - 2, top_x + 2, cy - 1, wood);

    // Brass grind collar between body and cap (glows hot during cap-twist tell!).
    let collar_pal = if telling {
        pal(
            fr,
            (255, 250, 212),
            mix(MILL_RING_HI, (255, 174, 72), charge),
            (176, 94, 34),
        )
    } else {
        brass
    };
    turned_row(pb, fr, top_x - 2, top_x + 2, cy - 2, collar_pal);

    // Fluted steel dome cap + brass finial nut (twists side-to-side during tell,
    // pops 1 px up on shot recoil).
    let cap_x = top_x + twist;
    let cap_y = cy - 3 - i32::from(recoil);
    let cap_pal = if telling {
        pal(
            fr,
            mix(MILL_CAP_HI, (255, 252, 224), charge),
            mix(MILL_CAP_MID, (255, 198, 102), charge * 0.85),
            mix(MILL_CAP_LO, (166, 94, 42), charge * 0.85),
        )
    } else {
        pal(fr, MILL_CAP_HI, MILL_CAP_MID, MILL_CAP_LO)
    };
    // Rotating flute groove on the dome cap so the twist reads mechanically!
    let flute_phase = if telling {
        ((e.fire_cd * 26.0) as i32).rem_euclid(3)
    } else {
        1
    };
    turned_row(pb, fr, cap_x - 2, cap_x + 2, cap_y, cap_pal);
    turned_row(pb, fr, cap_x - 2, cap_x + 2, cap_y - 1, cap_pal);
    // Vertical grip flutes across the steel dome.
    for dx in -1..=1i32 {
        if (dx + 2 + flute_phase) % 2 == 0 {
            pb.plot(cap_x + dx, cap_y, cap_pal.lo);
            pb.plot(cap_x + dx, cap_y - 1, cap_pal.mid);
        }
    }
    turned_row(pb, fr, cap_x - 1, cap_x + 1, cap_y - 2, cap_pal);
    // Brass finial knob on top of the cap.
    plot_rim(pb, cap_x, cap_y - 4, fr, 0.56);
    pb.plot(cap_x, cap_y - 3, collar_pal.hi);

    if telling {
        // Twin split-shot peppercorn & ember sparks jumping off both sides of
        // the twisting cap (telegraphs the ±10° twin ray release).
        let spark_col = mix((255, 184, 82), (255, 248, 206), charge);
        let spread = 3 + i32::from(charge > 0.55);
        pb.plot(cap_x - spread, cap_y - 1, spark_col);
        pb.plot(cap_x + spread, cap_y - 1, spark_col);
        pb.plot(cap_x - spread - twist.abs(), cap_y - 2, tone(fr, MILL_PEPPER));
        pb.plot(cap_x + spread + twist.abs(), cap_y - 2, tone(fr, MILL_PEPPER));
    } else if recoil {
        // Brief puff of freshly ground pepper specks on release.
        let p_col = tone(fr, MILL_PEPPER);
        pb.plot(cap_x - 3, cap_y - 2, p_col);
        pb.plot(cap_x + 3, cap_y - 2, p_col);
        pb.plot(cap_x, cap_y - 4, (255, 224, 152));
    }
}

/// Far LOD: dark-rimmed hourglass pawn with pinched waist, brass collar and
/// steel (or hot amber) twisting cap.
pub(crate) fn draw_peppermill_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let (cx, cy0) = centre(fr);
    let wood = pal(fr, MILL_HI, MILL_MID, MILL_LO);
    let twist = mill_twist(e);
    let hop = if twist == 0 && (fr.anim * 0.95).sin().abs() > 0.65 {
        -1
    } else {
        0
    };
    let cy = cy0 + hop;
    dim(pb, cx + 2, cy0 + 3, 0.70);

    turned_row(pb, fr, cx - 2, cx + 2, cy + 2, wood);
    turned_row(pb, fr, cx - 1, cx + 1, cy + 1, wood);
    // Crisp 1-px pinched waist.
    plot_rim(pb, cx - 1, cy, fr, 0.55);
    plot_rim(pb, cx + 1, cy, fr, 0.55);
    pb.plot(cx, cy, wood.mid);
    turned_row(pb, fr, cx - 1, cx + 1, cy - 1, wood);

    let cap = if twist != 0 {
        pal(fr, (255, 250, 214), (255, 200, 104), (168, 96, 40))
    } else {
        pal(fr, MILL_CAP_HI, MILL_CAP_MID, MILL_CAP_LO)
    };
    turned_row(pb, fr, cx - 1 + twist, cx + 1 + twist, cy - 2, cap);
    pb.plot(cx + twist, cy - 3, tone(fr, MILL_RING_HI));
}

// ---- Pie Weight (R Brute, r 6.5) --------------------------------------------

const PIE_BALL_HI: Rgb = (255, 252, 244);
const PIE_BALL_MID: Rgb = (222, 214, 200);
const PIE_BALL_LO: Rgb = (122, 116, 124);
const PIE_TIN_HI: Rgb = (236, 242, 250);
const PIE_TIN_MID: Rgb = (148, 156, 170);
const PIE_TIN_LO: Rgb = (58, 64, 78);
const PIE_RIM: Rgb = (246, 204, 92);
const PIE_RIM_LO: Rgb = (150, 104, 36);
const PIE_VENT: Rgb = (42, 34, 34);
const PIE_EYE_GLINT: Rgb = (255, 192, 92);
const PIE_ELITE_EYE: Rgb = (255, 128, 224);

/// Crimped tin skirt under a glazed ceramic stoneware ball, rolling ponderously.
/// Elites wear a doubled gold pie rim projecting on both sides and a thick
/// double gold shoulder stripe across the ceramic ball.
pub(crate) fn draw_pieweight(
    pb: &mut PixelBuffer,
    fr: &Frame,
    e: &Enemy,
    _cam: Vec2,
    elite: bool,
) {
    let s = fr.r / 6.5;
    let (cx, cy) = centre(fr);
    let slam = if e.contact_cd > 0.25 { 1 } else { 0 };
    let roll_phase = fr.anim * 1.15;
    let sway_x = (fr.anim * 0.72).sin() * 1.15 * s;
    let sway_y = (fr.anim * 1.44).cos().abs() * 0.45 * s + slam as f32;

    // Elite rotating gilded threat pips (matches Brute elite grammar, inside cull).
    if elite {
        let pulse = 0.55 + 0.45 * (fr.anim * 1.6).sin();
        let aura_r = fr.r + 1.9 * s;
        let pip_col = scale_rgb((255, 206, 72), 0.58 + 0.34 * pulse);
        for i in 0..8 {
            let a = i as f32 * (TAU / 8.0) + fr.anim * 0.16;
            let (sa, ca) = a.sin_cos();
            let px = (fr.pos.x + ca * aura_r).round() as i32;
            let py = (fr.pos.y + sa * aura_r).round() as i32;
            blend_px(pb, px, py, pip_col, 0.78);
        }
    }

    // Heavy grounded elliptical shadow beneath the tin skirt.
    soft_shadow(
        pb,
        cx as f32 + 0.5,
        cy as f32 + 4.8 * s,
        (7.2 + slam as f32 * 0.6) * s,
        2.3 * s,
        0.42,
    );

    let tin = pal(fr, PIE_TIN_HI, PIE_TIN_MID, PIE_TIN_LO);
    let rim_hi = tone(fr, PIE_RIM);
    let rim_lo = tone(fr, PIE_RIM_LO);

    // Fluted/crimped tin pie-pan skirt (5 rows flaring from ~9 to ~14 px wide,
    // with rolling metallic ridges and a physically scalloped bottom hem!).
    let top = cy + 1;
    let skirt_rows = 5i32;
    let crimp_shift = (roll_phase * 0.85).floor() as i32;
    for k in 0..skirt_rows {
        let half = ((4.1 + k as f32 * 0.58 + slam as f32 * 0.4) * s).round() as i32;
        let y = top + k;
        plot_rim(pb, cx - half - 1, y, fr, 0.64);
        plot_rim(pb, cx + half + 1, y, fr, 0.64);
        let span = (2 * half).max(1) as f32;
        for x in (cx - half)..=(cx + half) {
            let t = (x - (cx - half)) as f32 / span; // 0 = left lit, 1 = right dark
            let flute = ((x - cx) as f32 * 1.45 + roll_phase).sin(); // -1..1 rolling crimp
            let lit = (1.0 - t) * 0.72 + 0.18;
            let lum = (lit + flute * 0.34 - (k as f32 * 0.04)).clamp(0.05, 1.15);
            let mut col = shade3(tin, lum);
            // Dark baked-grease collar groove right under the ceramic ball.
            if k == 0 {
                col = mix(col, tin.lo, 0.42);
            }
            pb.plot(x, y, col);
        }
    }
    // Scalloped crimped hem row: teeth bob down on alternating rolling flutes
    // so the skirt's bottom silhouette physically ripples as it rolls!
    let half_hem = ((4.1 + 4.0 * 0.58 + slam as f32 * 0.4) * s).round() as i32;
    let hem_y = top + skirt_rows;
    for x in (cx - half_hem)..=(cx + half_hem) {
        let tooth = (x - cx + crimp_shift).rem_euclid(2) == 0;
        if tooth {
            let t = (x - (cx - half_hem)) as f32 / (2 * half_hem).max(1) as f32;
            pb.plot(x, hem_y, shade3(tin, 0.75 - t * 0.55));
            plot_rim(pb, x, hem_y + 1, fr, 0.56);
        } else {
            plot_rim(pb, x, hem_y, fr, 0.60);
        }
    }

    if elite {
        // Distinct elite attachment: DOUBLED GOLD PIE RIM projecting 2 px proud
        // on BOTH left and right at both the upper collar and the skirt hem!
        let half_top = (4.1 * s).round() as i32 + 2;
        let half_bot = half_hem + 2;
        plot_rim(pb, cx - half_top - 1, top, fr, 0.64);
        plot_rim(pb, cx + half_top + 1, top, fr, 0.64);
        for x in (cx - half_top)..=(cx + half_top) {
            pb.plot(x, top, rim_hi);
            pb.plot(x, top + 1, rim_lo);
        }
        plot_rim(pb, cx - half_bot - 1, top + 4, fr, 0.64);
        plot_rim(pb, cx + half_bot + 1, top + 4, fr, 0.64);
        for x in (cx - half_bot)..=(cx + half_bot) {
            pb.plot(x, top + 4, rim_hi);
            pb.plot(x, top + 5, rim_lo);
            plot_rim(pb, x, top + 6, fr, 0.56);
        }
    }

    // Glazed stoneware ceramic ball riding atop the tin skirt, swaying heavily.
    let bx = cx as f32 + sway_x;
    let by = cy as f32 - 2.6 * s + sway_y;
    let br = 4.4 * s;
    let ball_pal = pal(fr, PIE_BALL_HI, PIE_BALL_MID, PIE_BALL_LO);
    shaded_ball(pb, fr, bx, by, br, ball_pal);

    // Glaze crack when battered below 55% HP.
    let (ibx, iby) = (bx.round() as i32, by.round() as i32);
    if e.max_hp > 0.0 && e.hp < e.max_hp * 0.55 {
        let crack = tone(fr, PIE_VENT);
        pb.plot(ibx + 1, iby - 2, crack);
        pb.plot(ibx + 2, iby - 1, crack);
        pb.plot(ibx + 1, iby, crack);
    }

    if elite {
        pie_shoulder_stripes(pb, fr, bx, by, br);
    }

    // Recessed kiln-vent eyes facing slightly toward `fr.f.x`, with a warm
    // glaze/ember glint that survives low ambient pooled light.
    let look = (fr.f.x * 1.1).round() as i32;
    let vent = tone(fr, PIE_VENT);
    let eye_glint = if fr.hit > 0.65 {
        vent
    } else if elite {
        PIE_ELITE_EYE
    } else {
        PIE_EYE_GLINT
    };
    let ey = iby;
    pb.plot(ibx - 2 + look, ey, vent);
    pb.plot(ibx - 1 + look, ey, eye_glint);
    pb.plot(ibx + 1 + look, ey, eye_glint);
    pb.plot(ibx + 2 + look, ey, vent);

    // Crisp 2-pixel pure-white porcelain glaze glint at upper-left.
    let glint = tone(fr, (255, 255, 255));
    pb.plot(ibx - 2, iby - 3, glint);
    pb.plot(ibx - 3, iby - 2, glint);
}

/// The elite's thick double shoulder stripe: two gold bands, each two pixels
/// deep (`PIE_RIM` upper ridge + `PIE_RIM_LO` lower bevel), wrapped across the
/// ceramic ball at `(bx, by)`.
fn pie_shoulder_stripes(pb: &mut PixelBuffer, fr: &Frame, bx: f32, by: f32, r: f32) {
    let (ix, iy) = (bx.round() as i32, by.round() as i32);
    for (dy, col) in [
        (-2i32, PIE_RIM),
        (-1, PIE_RIM_LO),
        (1, PIE_RIM),
        (2, PIE_RIM_LO),
    ] {
        let span = r * r - (dy * dy) as f32;
        if span <= 0.0 {
            continue;
        }
        let half = (span.sqrt() - 0.5).floor() as i32;
        for x in (ix - half)..=(ix + half) {
            pb.plot(x, iy + dy, tone(fr, col));
        }
    }
}

/// Far LOD: dark-rimmed ceramic ball on a crimped tin skirt (with projecting
/// doubled gold rims and shoulder stripe when elite).
pub(crate) fn draw_pieweight_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    _e: &Enemy,
    _cam: Vec2,
    elite: bool,
) {
    let s = fr.r / 6.5;
    let (cx, cy) = centre(fr);
    let roll = (fr.anim * 1.15).floor() as i32;
    let sway = if roll.rem_euclid(2) == 0 { 0.5 } else { -0.5 };
    dim(pb, cx + 4, cy + 4, 0.70);

    for k in 0..3i32 {
        let half = 3 + k / 2 + i32::from(elite && k != 1);
        plot_rim(pb, cx - half - 1, cy + 1 + k, fr, 0.58);
        plot_rim(pb, cx + half + 1, cy + 1 + k, fr, 0.58);
        for x in (cx - half)..=(cx + half) {
            let crimp = (x - cx + roll).rem_euclid(2) == 0;
            let col = if elite && k != 1 {
                PIE_RIM
            } else if crimp {
                PIE_TIN_HI
            } else {
                PIE_TIN_LO
            };
            pb.plot(x, cy + 1 + k, tone(fr, col));
        }
    }
    shaded_ball(
        pb,
        fr,
        cx as f32 + sway,
        cy as f32 - 1.6 * s,
        3.1 * s,
        pal(fr, PIE_BALL_HI, PIE_BALL_MID, PIE_BALL_LO),
    );
    if elite {
        let y = (cy as f32 - 1.6 * s).round() as i32;
        for x in (cx - 2)..=(cx + 2) {
            pb.plot(x, y, tone(fr, PIE_RIM));
        }
    }
    pb.plot(cx - 1, cy - 3, tone(fr, (255, 255, 255)));
}

// ---- Sink Crab (V Pillbug, r 5.0) -------------------------------------------

const CRAB_HI: Rgb = (254, 170, 122);
const CRAB_MID: Rgb = (194, 84, 54);
const CRAB_LO: Rgb = (86, 26, 22);
const CRAB_KNEE: Rgb = (232, 124, 82);
const CRAB_EYE: Rgb = (255, 226, 126);
const CRAB_REAR_LO: Rgb = (218, 92, 108);
const CRAB_REAR_HI: Rgb = (255, 184, 174);
const LID_HI: Rgb = (248, 252, 255);
const LID_MID: Rgb = (184, 194, 208);
const LID_LO: Rgb = (82, 90, 108);
const LID_KNOB: Rgb = (44, 34, 32);
const LID_KNOB_HI: Rgb = (246, 204, 112);

/// Broad flat crustacean carapace with six sideways-scuttling bent legs, a
/// snapping right pincer and a stainless-steel pot-lid left claw. Curled, the
/// legs tuck and the pot lid swings flat across `facing` (the exact 120° arc
/// that blocks) while exposing its soft pulsing coral rear.
pub(crate) fn draw_sinkcrab(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let shell = pal(fr, CRAB_HI, CRAB_MID, CRAB_LO);
    let lid = pal(fr, LID_HI, LID_MID, LID_LO);
    let curled = e.w3.curl_t > 0.0;
    let pinching = !curled && e.contact_cd > 0.25;

    // Slight lateral crab scuttle rock when walking.
    let scuttle_v = if curled {
        0.0
    } else {
        (fr.anim * 1.4).sin() * 0.50 * s
    };
    let origin = place(fr, 0.0, scuttle_v);
    let mut body_fr = *fr;
    body_fr.pos = origin;

    soft_shadow(
        pb,
        origin.x + 1.0,
        origin.y + 1.2,
        if curled { 4.2 * s } else { 5.2 * s },
        if curled { 4.2 * s } else { 3.8 * s },
        0.38,
    );

    // 1) Six bent sideways crab legs (tucked cleanly underneath when curled!).
    if !curled {
        let leg_hi = tone(fr, CRAB_KNEE);
        let leg_lo = shell.lo;
        for (i, u_hip) in [1.45f32, -0.05, -1.55].into_iter().enumerate() {
            let phase = fr.anim * 1.45 + i as f32 * 1.85;
            let reach_wave = phase.sin();
            let sweep_u = phase.cos() * 0.65 * s;
            for (side, w) in [(-1.0f32, reach_wave), (1.0, -reach_wave)] {
                let out = (6.4 + 0.95 * w) * s;
                let hip = at(&body_fr, u_hip * s, side * 4.0 * s);
                let knee = at(&body_fr, (u_hip + 0.35) * s + sweep_u * 0.5, side * (out - 1.1 * s));
                let tip = at(&body_fr, (u_hip - 0.55) * s + sweep_u, side * out);
                dim(pb, tip.0 + 1, tip.1 + 1, 0.72);
                line_blend(pb, hip, knee, leg_hi, 0.94);
                line_blend(pb, knee, tip, leg_lo, 0.95);
            }
        }
    }

    // 2) Broad flat sculpted crab carapace (wider at front shoulders, tapered
    // at the rear apron, with top-left dome lighting and dark silhouette rim).
    let ru = if curled { 2.6 * s } else { 2.9 * s };
    let rv = if curled { 4.3 * s } else { 4.9 * s };
    let reach = rv.ceil() as i32 + 2;
    let (ix, iy) = (origin.x.round() as i32, origin.y.round() as i32);
    let rim_c = rim_tint(fr);
    let pulse = 0.5 + 0.5 * (fr.anim * 5.0 + e.w3.curl_t * 7.0).sin();
    let rear_col = tone(fr, mix(CRAB_REAR_LO, CRAB_REAR_HI, 0.35 + 0.55 * pulse));

    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let x = ix + dx;
            let y = iy + dy;
            let fx = x as f32 - origin.x;
            let fy = y as f32 - origin.y;
            let u = fx * body_fr.f.x + fy * body_fr.f.y;
            let v = fx * body_fr.p.x + fy * body_fr.p.y;
            let nu = u / ru;
            // Crab-shell taper: slightly broader at front shoulders (+u), narrower aft (-u).
            let width_mod = (1.0 + 0.16 * nu.clamp(-1.0, 1.0)).max(0.65);
            let nv = v / (rv * width_mod);
            let r2 = nu * nu + nv * nv;
            if r2 <= 1.0 {
                // Curled: exposed soft pulsing coral rear apron (2× damage cue!).
                if curled && nu < -0.46 && nv.abs() < 0.62 {
                    pb.plot(x, y, rear_col);
                    continue;
                }
                let dome = (1.0 - r2 * 0.68).clamp(0.0, 1.0);
                let lit = (fx * LIGHT.x + fy * LIGHT.y) / rv;
                let mut lum = 0.40 + 0.36 * dome + 0.56 * lit;
                if lit > 0.35 && dome > 0.35 {
                    lum += 0.22;
                }
                let mut col = shade3(shell, lum);
                // Transverse carapace ridge & notched side plates.
                if (u - 0.35 * s).abs() < 0.38 * s && nv.abs() < 0.72 {
                    col = mix(col, shell.lo, 0.45);
                } else if r2 > 0.76 && nu > 0.0 {
                    col = mix(col, shell.hi, 0.32);
                }
                pb.plot(x, y, col);
            } else if r2 <= 1.48 {
                blend_px(pb, x, y, rim_c, 0.62);
            }
        }
    }

    // 3) Stalked crab eyes with bright amber glints at the front edge.
    let eye_glint = if fr.hit > 0.65 {
        (32, 18, 18)
    } else {
        CRAB_EYE
    };
    let eye_u = if curled { 2.5 * s } else { 3.25 * s };
    for side in [-1.0f32, 1.0] {
        let stalk = at(&body_fr, eye_u - 0.65 * s, side * 1.45 * s);
        let tip = at(&body_fr, eye_u, side * 1.65 * s);
        pb.plot(stalk.0, stalk.1, shell.lo);
        pb.plot(tip.0, tip.1, eye_glint);
    }

    if curled {
        // 4a) CURLED: The stainless-steel pot lid swings flat across `facing`
        // as an arched 120° barricade with a protruding central lid handle!
        let sheen = tone(fr, mix(LID_HI, (255, 255, 255), 0.45 + 0.45 * pulse));
        for k in -5..=5i32 {
            let v = k as f32 * 0.95 * s;
            let arch = -((k * k) as f32) * 0.042 * s;
            let u_front = (5.45 + arch) * s;
            let u_mid = (4.45 + arch) * s;
            let p_rim = at(&body_fr, u_front + 0.95 * s, v);
            plot_rim(pb, p_rim.0, p_rim.1, fr, 0.56);
            let p_front = at(&body_fr, u_front, v);
            let p_back = at(&body_fr, u_mid, v);
            let edge = k.abs() >= 4;
            pb.plot(p_front.0, p_front.1, if edge { lid.mid } else { sheen });
            pb.plot(p_back.0, p_back.1, if edge { lid.lo } else { lid.mid });
        }
        // Protruding central pot-lid arched loop handle (steel + brass/bakelite).
        let knob_dark = tone(fr, LID_KNOB);
        let knob_hi = tone(fr, LID_KNOB_HI);
        for dv in [-1.0f32, 0.0, 1.0] {
            let p_bar = at(&body_fr, 6.55 * s, dv * s);
            pb.plot(p_bar.0, p_bar.1, if dv == 0.0 { knob_hi } else { knob_dark });
        }
        for side in [-1.0f32, 1.0] {
            let post = at(&body_fr, 5.95 * s, side * 1.15 * s);
            pb.plot(post.0, post.1, knob_dark);
        }
        // Pulsing vulnerable rear pip behind the apron so the flank cue pops.
        let rear_pip = at(&body_fr, -3.2 * s, 0.0);
        pb.plot(rear_pip.0, rear_pip.1, tone(fr, CRAB_REAR_HI));
    } else {
        // 4b) WALKING: Asymmetric claws — gleaming round stainless pot lid on
        // the left (-v) and an open snapping V-pincer on the right (+v)!
        let claw_bob = (fr.anim * 1.4).cos() * 0.35 * s;
        let lid_c = place(&body_fr, (3.1 + claw_bob) * s, -3.9 * s);
        let arm_l = at(&body_fr, 1.9 * s, -3.2 * s);
        let arm_l2 = at(&body_fr, (2.6 + claw_bob) * s, -3.6 * s);
        line_blend(pb, arm_l, arm_l2, shell.lo, 0.94);

        // Round brushed-steel pot lid with rolled rim and crown knob.
        shaded_ball(pb, fr, lid_c.x, lid_c.y, 2.45 * s, lid);
        let (lx, ly) = (lid_c.x.round() as i32, lid_c.y.round() as i32);
        pb.plot(lx - 1, ly - 1, tone(fr, (255, 255, 255)));
        pb.plot(lx, ly, tone(fr, LID_KNOB));
        pb.plot(lx + 1, ly, tone(fr, LID_KNOB_HI));

        // Nimble bare right pincer with a readable open V-notch (snaps on contact).
        let snap = if pinching { 0.35 } else { 0.95 };
        let r_base = at(&body_fr, 2.2 * s, 3.3 * s);
        let r_palm = at(&body_fr, (3.3 - claw_bob) * s, 3.9 * s);
        let r_outer = at(&body_fr, (4.4 - claw_bob) * s, (3.9 + snap) * s);
        let r_inner = at(&body_fr, (4.3 - claw_bob) * s, (3.9 - snap) * s);
        line_blend(pb, r_base, r_palm, shell.mid, 0.95);
        line_blend(pb, r_palm, r_outer, shell.hi, 0.95);
        line_blend(pb, r_palm, r_inner, shell.hi, 0.95);
    }
}

/// Far LOD: broad dark-rimmed burnt-orange crab with steel pot-lid disc (or
/// gleaming frontal steel shield bar + pink rear dot when curled).
pub(crate) fn draw_sinkcrab_lod(
    pb: &mut PixelBuffer,
    fr: &Frame,
    e: &Enemy,
    _cam: Vec2,
    _elite: bool,
) {
    let s = fr.r / 5.0;
    let curled = e.w3.curl_t > 0.0;
    let (cx, cy) = centre(fr);
    dim(pb, cx + 2, cy + 2, 0.70);

    if !curled {
        let out = if (fr.anim * 1.45).sin() > 0.0 { 4.8 } else { 4.0 };
        for side in [-1.0f32, 1.0] {
            let (x, y) = at(fr, 0.6 * s, side * out * s);
            pb.plot(x, y, tone(fr, CRAB_LO));
            let (x, y) = at(fr, -0.9 * s, -side * out * s);
            pb.plot(x, y, tone(fr, CRAB_LO));
        }
    }

    shaded_ellipse(
        pb,
        fr,
        fr.pos,
        fr.f,
        (1.8 * s, 3.7 * s),
        pal(fr, CRAB_HI, CRAB_MID, CRAB_LO),
    );
    if curled {
        for k in -3..=3i32 {
            let (x, y) = at(fr, 3.8 * s, k as f32 * s);
            pb.plot(x, y, tone(fr, LID_HI));
        }
        let (hx, hy) = at(fr, 4.8 * s, 0.0);
        pb.plot(hx, hy, tone(fr, LID_KNOB_HI));
        let (rx, ry) = at(fr, -2.0 * s, 0.0);
        pb.plot(rx, ry, tone(fr, CRAB_REAR_HI));
    } else {
        let (x, y) = at(fr, 2.5 * s, -3.1 * s);
        pb.plot(x, y, tone(fr, LID_HI));
        pb.plot(x + 1, y, tone(fr, LID_MID));
        pb.plot(x, y + 1, tone(fr, LID_LO));
        pb.plot(x + 1, y + 1, tone(fr, LID_KNOB));
        let (px, py) = at(fr, 2.7 * s, 3.1 * s);
        pb.plot(px, py, tone(fr, CRAB_HI));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const KINDS: [EnemyKind; 6] = [
        EnemyKind::CrumbAnt,
        EnemyKind::FruitFly,
        EnemyKind::EggTimer,
        EnemyKind::PepperMill,
        EnemyKind::PieWeight,
        EnemyKind::SinkCrab,
    ];

    fn body(kind: EnemyKind) -> Enemy {
        let mut w = World::new(Theme::Kitchen, 3);
        let entity = w.spawn_enemy(kind, Vec2::new(40.0, 40.0), 1.0);
        w.ecs.get::<Enemy>(entity).expect("spawned").clone()
    }

    fn painted(e: &Enemy, lod: bool) -> Vec<(i32, i32)> {
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear((0, 0, 0));
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, e, Vec2::new(0.0, 0.0), (255, 0, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, e, Vec2::new(0.0, 0.0), (255, 0, 255));
        }
        let mut out = Vec::new();
        for y in 0..80usize {
            for x in 0..80usize {
                if pb.pixel_at(x, y) != (0, 0, 0) {
                    out.push((x as i32, y as i32));
                }
            }
        }
        out
    }

    fn painted_rgb(e: &Enemy, lod: bool) -> Vec<Rgb> {
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear((0, 0, 0));
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, e, Vec2::new(0.0, 0.0), (255, 0, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, e, Vec2::new(0.0, 0.0), (255, 0, 255));
        }
        pb.pixels().to_vec()
    }

    #[test]
    fn every_kitchen_kind_paints_a_body_at_both_distances() {
        for kind in KINDS {
            let e = body(kind);
            assert!(painted(&e, false).len() >= 6, "{kind:?} full body");
            assert!(painted(&e, true).len() >= 4, "{kind:?} far body");
        }
    }

    #[test]
    fn the_six_silhouettes_differ() {
        let shapes: Vec<Vec<(i32, i32)>> = KINDS.iter().map(|k| painted(&body(*k), false)).collect();
        for i in 0..shapes.len() {
            for j in (i + 1)..shapes.len() {
                assert_ne!(shapes[i], shapes[j], "{:?} and {:?}", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn walk_frames_differ() {
        for kind in KINDS {
            let mut e = body(kind);
            e.anim = 0.4;
            let a = painted(&e, false);
            let mut moved = false;
            for k in 1..40 {
                e.anim = 0.4 + k as f32 * 0.5;
                if painted(&e, false) != a {
                    moved = true;
                    break;
                }
            }
            assert!(moved, "{kind:?} has a second walk frame");
        }
    }

    #[test]
    fn walk_cycles_have_multiple_distinct_frames() {
        for kind in KINDS {
            let mut e = body(kind);
            let mut seen: Vec<Vec<(i32, i32)>> = Vec::new();
            for k in 0..24 {
                e.anim = k as f32 * 0.35;
                let frame = painted(&e, false);
                if !seen.contains(&frame) {
                    seen.push(frame);
                }
            }
            assert!(
                seen.len() >= 3,
                "{kind:?} should have at least 3 distinct walk frames, got {}",
                seen.len()
            );
        }
    }

    #[test]
    fn the_egg_ring_is_drawn_where_it_hits() {
        let mut e = body(EnemyKind::EggTimer);
        e.w4.phase = sim::EGG_PHASE_TICK;
        e.w4.t = sim::EGG_RING_SECS * 0.5;
        let (centre, radius) = sim::egg_ring_of(&e).expect("live ring");
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear((0, 0, 0));
        draw_egg_ring(&mut pb, &e, Vec2::new(0.0, 0.0));
        let mut lit = 0;
        for y in 0..80usize {
            for x in 0..80usize {
                let on = pb.pixel_at(x, y) != (0, 0, 0);
                let hits = sim::egg_ring_hits(centre, radius, Vec2::new(x as f32, y as f32));
                assert_eq!(on, hits, "pixel ({x}, {y})");
                lit += usize::from(on);
            }
        }
        assert!(lit > 100);
    }

    #[test]
    fn the_egg_tell_marks_the_ring_extent_before_it_is_live() {
        let mut e = body(EnemyKind::EggTimer);
        e.w4.phase = sim::EGG_PHASE_WIND;
        e.w4.t = sim::EGG_WIND_SECS * 0.5;
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear((0, 0, 0));
        draw_egg_ring(&mut pb, &e, Vec2::new(0.0, 0.0));
        let outer = sim::EGG_RING_END + sim::EGG_RING_WIDTH * 0.5;
        let mut far = 0;
        for y in 0..80usize {
            for x in 0..80usize {
                if pb.pixel_at(x, y) != (0, 0, 0) {
                    let d = e.pos.dist(Vec2::new(x as f32, y as f32));
                    assert!(d <= outer + 1.0);
                    far += usize::from(d >= outer - 1.0);
                }
            }
        }
        assert!(far >= 12, "the outer extent is dotted");
    }

    #[test]
    fn egg_timer_phases_look_distinct() {
        let mut e = body(EnemyKind::EggTimer);
        e.anim = 0.0;
        e.w4.phase = sim::EGG_PHASE_WALK;
        let walk = painted_rgb(&e, false);

        e.w4.phase = sim::EGG_PHASE_WIND;
        e.w4.t = sim::EGG_WIND_SECS * 0.4;
        let wind = painted_rgb(&e, false);

        e.w4.phase = sim::EGG_PHASE_TICK;
        e.w4.t = sim::EGG_RING_SECS * 0.5;
        let tick = painted_rgb(&e, false);

        e.w4.phase = sim::EGG_PHASE_SLEEP;
        e.w4.t = sim::EGG_SLEEP_SECS * 0.5;
        let sleep = painted_rgb(&e, false);

        assert_ne!(walk, wind);
        assert_ne!(wind, tick);
        assert_ne!(tick, sleep);
        assert_ne!(walk, sleep);
    }

    #[test]
    fn a_curled_sink_crab_raises_its_lid_toward_its_facing() {
        let mut e = body(EnemyKind::SinkCrab);
        e.facing = Vec2::new(0.0, -1.0);
        let open = painted(&e, false);
        e.w3.curl_t = 1.0;
        let curled = painted(&e, false);
        assert_ne!(open, curled);
        // The bar sits north of the body, where the facing points.
        let north = curled.iter().filter(|(_, y)| *y <= 40 - 5).count();
        assert!(north >= 7, "lid bar toward the facing, got {north}");
        // Far LOD also switches silhouette when curled.
        e.w3.curl_t = 0.0;
        let lod_open = painted(&e, true);
        e.w3.curl_t = 1.0;
        assert_ne!(lod_open, painted(&e, true));
    }

    #[test]
    fn the_elite_pie_weight_wears_its_rim_and_stripes() {
        let mut e = body(EnemyKind::PieWeight);
        e.anim = 0.4;
        let plain = painted_rgb(&e, false);
        e.elite = true;
        let elite = painted_rgb(&e, false);
        assert_ne!(plain, elite);
        let gold = |px: &Vec<Rgb>| px.iter().filter(|c| **c == PIE_RIM).count();
        assert!(gold(&elite) >= gold(&plain) + 20, "rim and stripes are gold");
        assert_ne!(painted_rgb(&e, true), {
            e.elite = false;
            painted_rgb(&e, true)
        });
    }

    #[test]
    fn the_pepper_mill_cap_twists_before_the_shot() {
        let mut e = body(EnemyKind::PepperMill);
        e.fire_cd = 2.0;
        let rest = painted(&e, false);
        e.fire_cd = 0.3;
        let tell = painted(&e, false);
        assert_ne!(tell, rest);
        // Recoil right after release differs from resting too.
        e.fire_cd = 2.15;
        assert_ne!(painted(&e, false), rest);
    }

    #[test]
    fn every_kitchen_body_stays_inside_its_archetype_cull_bound() {
        // Conservative archetype cull reaches from `art_enemy::cull_reach`.
        let bounds: [(EnemyKind, f32); 6] = [
            (EnemyKind::CrumbAnt, 7.5),
            (EnemyKind::FruitFly, 10.5),
            (EnemyKind::EggTimer, 13.5),
            (EnemyKind::PepperMill, 9.6),
            (EnemyKind::PieWeight, 13.7),
            (EnemyKind::SinkCrab, 13.5),
        ];
        for (kind, reach) in bounds {
            let mut e = body(kind);
            if kind == EnemyKind::PieWeight {
                e.elite = true;
            }
            for (x, y) in painted(&e, false) {
                let d = Vec2::new(x as f32, y as f32).dist(e.pos);
                assert!(
                    d <= reach,
                    "{kind:?} pixel ({x}, {y}) at dist {d:.2} exceeds cull reach {reach}"
                );
            }
        }
    }
}
