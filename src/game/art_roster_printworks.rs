//! Printworks roster bodies (`WAVE4_ROSTERS.md` "Printworks", batch 6b).
//!
//! Seven bodies, one full-detail and one far-LOD draw each, all with the
//! `BodyDraw` signature `art_enemy::bespoke_body`/`bespoke_lod` expect:
//!
//! | kind | silhouette |
//! |---|---|
//! | Silverfish | narrow rippling spindle, three-prong tail |
//! | Erratum Slip | torn L of paper tumbling edge over edge |
//! | Type Slug | lead block with a raised T face, ruled skate line |
//! | Staple Crawler | inverted U on two needle feet, planted alternately |
//! | Paper Wasp | folded arrow wings over a rolled newsprint abdomen |
//! | Paper Reel | wide spool, dark axle holes, a loose flapping end |
//! | Ink Blot | ragged black star in a porcelain outline |
//!
//! Render-only: nothing here writes the simulation. Bone paper and metal
//! edges over charcoal is the floor's palette; every body carries one light
//! and one dark edge so it survives the lighting pass and far zoom.
//!
//! The Staple Crawler's footprint ([`draw_staple_tell`]) is drawn from the
//! same two points and radius the simulation hits with
//! (`floor_printworks::STAPLE_FOOT_RADIUS`, `Enemy::w4.trail[0..2]`).

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_printworks as sim;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

// ---- Palette ---------------------------------------------------------------

const PAPER: Rgb = (232, 228, 208);
const PAPER_HI: Rgb = (250, 247, 232);
const PAPER_DARK: Rgb = (150, 144, 124);
const CREASE: Rgb = (96, 88, 70);
const STEEL: Rgb = (150, 166, 190);
const STEEL_HI: Rgb = (232, 240, 252);
const STEEL_DARK: Rgb = (56, 66, 88);
const LEAD: Rgb = (108, 110, 118);
const LEAD_HI: Rgb = (212, 214, 224);
const LEAD_DARK: Rgb = (44, 46, 54);
const INK: Rgb = (22, 16, 32);
const PORCELAIN: Rgb = (238, 234, 224);
const PORCELAIN_DIM: Rgb = (150, 146, 162);
const KRAFT: Rgb = (140, 112, 80);
const WARN: Rgb = (255, 92, 64);
const WARN_HI: Rgb = (255, 226, 190);

// ---- Small local helpers (kept here so `art_enemy` exports nothing more) ----

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

/// The warm hit flash every body shares (same numbers as `art_enemy`).
#[inline]
fn tone(fr: &Frame, c: Rgb) -> Rgb {
    if fr.hit <= 0.0 {
        return c;
    }
    let flash = if fr.hit > 0.6 { (255, 244, 218) } else { (255, 218, 152) };
    mix(c, flash, (0.30 + fr.hit * 0.56).min(0.86))
}

#[inline]
fn put(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

/// A contact shadow pixel.
#[inline]
fn shade(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    put(pb, x, y, (0, 0, 0), amount);
}

fn line(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let steps = dx.abs().max(dy.abs()).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (a.0 as f32 + dx as f32 * t).round() as i32;
        let y = (a.1 as f32 + dy as f32 * t).round() as i32;
        put(pb, x, y, c, alpha);
    }
}

fn disc(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: i32, c: Rgb, alpha: f32) {
    let r2 = rad * rad + rad;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            if dx * dx + dy * dy <= r2 {
                put(pb, cx + dx, cy + dy, c, alpha);
            }
        }
    }
}

/// A dotted ring (every other pixel of a 1 px circle), for warned footprints.
fn dotted_ring(pb: &mut PixelBuffer, cx: i32, cy: i32, rad: f32, c: Rgb, alpha: f32) {
    let n = ((rad * 6.3).ceil() as i32).max(8);
    for i in 0..n {
        if i % 2 == 1 {
            continue;
        }
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        put(
            pb,
            cx + (a.cos() * rad).round() as i32,
            cy + (a.sin() * rad).round() as i32,
            c,
            alpha,
        );
    }
}

/// Local `(forward, lateral)` to a screen pixel.
#[inline]
fn at(fr: &Frame, u: f32, v: f32) -> (i32, i32) {
    (
        (fr.pos.x + fr.f.x * u + fr.p.x * v).round() as i32,
        (fr.pos.y + fr.f.y * u + fr.p.y * v).round() as i32,
    )
}

/// `+1.0` when the body's `+v` flank faces the key light (up-left), else `-1.0`.
#[inline]
fn lit_side(fr: &Frame) -> f32 {
    if fr.p.x * -0.55 + fr.p.y * -0.83 >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Two-frame gait clock.
#[inline]
fn step(fr: &Frame, rate: f32) -> bool {
    ((fr.anim * rate) as i32) & 1 == 0
}

#[inline]
fn centre(fr: &Frame) -> (i32, i32) {
    (fr.pos.x.round() as i32, fr.pos.y.round() as i32)
}

// ---- Silverfish (A, r 2.5) -------------------------------------------------

/// Narrow spindle with a three-prong tail; its sides ripple as it streams.
pub(crate) fn draw_silverfish(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let lit = lit_side(fr);
    let body = tone(fr, (196, 200, 206));
    let hi = tone(fr, STEEL_HI);
    let dark = tone(fr, STEEL_DARK);
    // Three-prong tail first, so the spindle overlaps its root.
    let root = at(fr, -3.5 * s, 0.0);
    let wag = (fr.anim * 0.9).sin() * 0.8;
    line(pb, root, at(fr, -6.5 * s, wag * s), dark, 0.9);
    line(pb, root, at(fr, -5.8 * s, (2.2 + wag) * s), dark, 0.8);
    line(pb, root, at(fr, -5.8 * s, (-2.2 + wag) * s), dark, 0.8);
    // The spindle: widest behind the head, tapering to the tail; the ripple
    // runs down the body (two readable frames at walking cadence).
    let mut u = -3.5f32;
    while u <= 4.0 {
        let t = (u - 1.0) / 4.6;
        let half = (1.7 * (1.0 - t * t)).max(0.0) * s;
        let ripple = (fr.anim * 0.9 + u * 1.3).sin() * 0.55 * s;
        let c = at(fr, u * s, ripple);
        shade(pb, c.0 + 1, c.1 + 1, 0.3);
        if half >= 0.6 {
            let a = at(fr, u * s, ripple + half * lit);
            let b = at(fr, u * s, ripple - half * lit);
            line(pb, a, b, body, 1.0);
            put(pb, a.0, a.1, hi, 1.0);
            put(pb, b.0, b.1, dark, 1.0);
        } else {
            put(pb, c.0, c.1, body, 1.0);
        }
        u += 0.75;
    }
    // Segment bands and the two feelers.
    for k in 0..3 {
        let c = at(fr, (-2.0 + k as f32 * 1.7) * s, 0.0);
        put(pb, c.0, c.1, dark, 0.45);
    }
    let nose = at(fr, 4.2 * s, 0.0);
    line(pb, nose, at(fr, 6.4 * s, 1.6 * s), hi, 0.7);
    line(pb, nose, at(fr, 6.4 * s, -1.6 * s), hi, 0.7);
}

pub(crate) fn draw_silverfish_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let col = tone(fr, e.kind.color());
    let a = at(fr, -2.5 * s, 0.0);
    let b = at(fr, 3.5 * s, 0.0);
    line(pb, a, b, col, 1.0);
    put(pb, b.0, b.1, tone(fr, STEEL_HI), 1.0);
    let fork = at(fr, -4.0 * s, if step(fr, 0.9) { 1.0 } else { -1.0 });
    put(pb, fork.0, fork.1, tone(fr, STEEL_DARK), 1.0);
}

// ---- Erratum Slip (B, r 2.0) -----------------------------------------------

/// The L's two bars in sheet space: `(a0, a1, b0, b1)`.
const SLIP_BARS: [(f32, f32, f32, f32); 2] = [(-3.0, 3.0, -3.0, -1.0), (-3.0, -1.0, -1.0, 3.5)];

/// Torn L of paper tumbling edge over edge: the sheet turns slowly and flips
/// through its own plane, showing its grey back every other half-turn.
pub(crate) fn draw_erratumslip(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.0);
    // `anim` runs at the Bat's 22/s buzz, so these are slow fractions of it.
    let turn = fr.anim * 0.11;
    let flip = (fr.anim * 0.16).cos();
    let squash = flip.abs().max(0.22);
    let (sn, cs) = turn.sin_cos();
    let face = if flip >= 0.0 { PAPER } else { PAPER_DARK };
    let edge_hi = if flip >= 0.0 { PAPER_HI } else { PAPER };
    let c = fr.pos;
    // Shadow pass, then the sheet, so the L keeps a dark lower-right edge.
    for pass in 0..2 {
        for &(a0, a1, b0, b1) in SLIP_BARS.iter() {
            let mut a = a0;
            while a <= a1 {
                let mut b = b0;
                while b <= b1 {
                    // The torn end of the long bar: a ragged, fixed bite.
                    let torn = b > 2.6 && ((a * 2.0) as i32) % 2 == 0;
                    if !torn {
                        let (x, y) = (a * s, b * squash * s);
                        let px = (c.x + x * cs - y * sn).round() as i32;
                        let py = (c.y + x * sn + y * cs).round() as i32;
                        if pass == 0 {
                            shade(pb, px + 1, py + 1, 0.35);
                        } else {
                            let rim = b <= b0 + 0.01 || a <= a0 + 0.01;
                            let col = if rim { edge_hi } else { face };
                            put(pb, px, py, tone(fr, col), 1.0);
                        }
                    }
                    b += 0.5;
                }
                a += 0.5;
            }
        }
    }
    // One printed line across the short bar: the correction itself.
    let (x0, x1, y) = (-2.0 * s, 2.0 * s, -2.0 * squash * s);
    let p0 = ((c.x + x0 * cs - y * sn).round() as i32, (c.y + x0 * sn + y * cs).round() as i32);
    let p1 = ((c.x + x1 * cs - y * sn).round() as i32, (c.y + x1 * sn + y * cs).round() as i32);
    if flip >= 0.0 {
        line(pb, p0, p1, tone(fr, CREASE), 0.55);
    }
}

pub(crate) fn draw_erratumslip_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let col = tone(fr, e.kind.color());
    let (cx, cy) = centre(fr);
    // A three-pixel L in one of four tumbling orientations.
    let q = ((fr.anim * 0.07) as i32).rem_euclid(4);
    let (dx, dy) = match q {
        0 => (1, 1),
        1 => (-1, 1),
        2 => (-1, -1),
        _ => (1, -1),
    };
    shade(pb, cx + 1, cy + 1, 0.3);
    put(pb, cx, cy, col, 1.0);
    put(pb, cx + dx, cy, tone(fr, PAPER_HI), 1.0);
    put(pb, cx, cy + dy, col, 1.0);
    put(pb, cx, cy + 2 * dy, tone(fr, PAPER_DARK), 1.0);
}

// ---- Type Slug (C, r 4.0) --------------------------------------------------

/// Lead block with a raised T on its face, skating a straight ruled line.
pub(crate) fn draw_typeslug(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 4.0);
    let lit = lit_side(fr);
    let (hu, hv) = (4.0 * s, 3.0 * s);
    // The ruled line it skates: dashes behind it that march with the gait.
    let phase = if step(fr, 0.5) { 0.0 } else { 1.5 };
    let mut u = -hu - 1.5 - phase;
    while u > -hu - 8.0 {
        let a = at(fr, u, 0.0);
        let b = at(fr, u - 1.2, 0.0);
        line(pb, a, b, LEAD_DARK, 0.5);
        u -= 3.0;
    }
    // The block, row by row along `v`, with a contact shadow.
    let rows = (hv * 2.0).ceil() as i32;
    for i in 0..=rows {
        let v = -hv + i as f32 * (2.0 * hv / rows.max(1) as f32);
        let a = at(fr, -hu, v);
        let b = at(fr, hu, v);
        line(pb, (a.0 + 1, a.1 + 2), (b.0 + 1, b.1 + 2), (0, 0, 0), 0.22);
    }
    for i in 0..=rows {
        let v = -hv + i as f32 * (2.0 * hv / rows.max(1) as f32);
        let lit_row = (v * lit) > hv - 0.9;
        let dark_row = (v * lit) < -hv + 0.9;
        let col = if lit_row {
            LEAD_HI
        } else if dark_row {
            LEAD_DARK
        } else {
            LEAD
        };
        line(pb, at(fr, -hu, v), at(fr, hu, v), tone(fr, col), 1.0);
    }
    // Nose bevel (light) and heel (dark) so the heading reads.
    line(pb, at(fr, hu, -hv), at(fr, hu, hv), tone(fr, mix(LEAD, LEAD_HI, 0.55)), 1.0);
    line(pb, at(fr, -hu, -hv), at(fr, -hu, hv), tone(fr, LEAD_DARK), 1.0);
    // The raised T: crossbar toward the nose, stem down the middle.
    let t_col = tone(fr, PAPER_HI);
    line(pb, at(fr, 2.0 * s, -1.9 * s), at(fr, 2.0 * s, 1.9 * s), t_col, 1.0);
    line(pb, at(fr, 2.0 * s, 0.0), at(fr, -2.2 * s, 0.0), t_col, 1.0);
}

pub(crate) fn draw_typeslug_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 4.0);
    let col = tone(fr, e.kind.color());
    for v in [-1.5f32, -0.5, 0.5, 1.5] {
        line(pb, at(fr, -2.5 * s, v * s), at(fr, 2.5 * s, v * s), col, 1.0);
    }
    let lit = lit_side(fr);
    line(pb, at(fr, -2.5 * s, 2.0 * s * lit), at(fr, 2.5 * s, 2.0 * s * lit), tone(fr, LEAD_HI), 1.0);
    line(pb, at(fr, -2.5 * s, -2.0 * s * lit), at(fr, 2.5 * s, -2.0 * s * lit), tone(fr, LEAD_DARK), 1.0);
    line(pb, at(fr, 1.5 * s, -s), at(fr, 1.5 * s, 1.0 * s), tone(fr, PAPER_HI), 1.0);
}

// ---- Staple Crawler (I, r 2.0) ---------------------------------------------

/// The crawler's warned footprint: two radius-4 holes astride the locked rat.
///
/// Reads the same points and radius the simulation hits with. Safe to call
/// for any body: it draws nothing unless a Staple Crawler is in its tell,
/// strike or the first quarter-second of its rest. Stamped on the hostile
/// layer (after the lighting pass) through `art_enemy::draw_w3_tells`.
pub fn draw_staple_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if e.kind != crate::game::enemy::EnemyKind::StapleCrawler || !e.alive() {
        return;
    }
    let phase = e.w4.phase;
    if phase == sim::STAPLE_PHASE_WALK {
        return;
    }
    let rad = sim::STAPLE_FOOT_RADIUS;
    for i in 0..2 {
        let p = e.w4.trail[i] - cam;
        let (cx, cy) = (p.x.round() as i32, p.y.round() as i32);
        match phase {
            sim::STAPLE_PHASE_TELL => {
                // Dotted rim at the true radius, a core that grows to fill it
                // as the feet come down.
                let k = 1.0 - (e.w4.t / sim::STAPLE_TELL_SECS).clamp(0.0, 1.0);
                dotted_ring(pb, cx, cy, rad, WARN, 0.95);
                let core = (rad * k).floor() as i32;
                if core >= 1 {
                    disc(pb, cx, cy, core - 1, WARN, 0.35 + 0.3 * k);
                }
                put(pb, cx, cy, WARN_HI, 1.0);
            }
            sim::STAPLE_PHASE_STRIKE => {
                // Live: the whole hole, a hard bright rim and the needle.
                disc(pb, cx, cy, rad as i32 - 1, WARN, 0.85);
                dotted_ring(pb, cx, cy, rad, WARN_HI, 1.0);
                dotted_ring(pb, cx, cy, rad - 0.01, WARN_HI, 1.0);
                line(pb, (cx, cy - 2), (cx, cy + 1), STEEL_HI, 1.0);
                put(pb, cx, cy + 2, STEEL_DARK, 1.0);
            }
            _ => {
                // Rest: a spent puncture that fades (cosmetic, no hazard).
                let age = sim::STAPLE_REST_SECS - e.w4.t;
                if age < 0.25 {
                    let a = 0.6 * (1.0 - age / 0.25);
                    disc(pb, cx, cy, 1, STEEL_DARK, a);
                }
            }
        }
    }
}

/// Inverted U on two needle feet. Drawn upright on screen (a staple seen side
/// on), so the silhouette never turns into a bar when it walks up the screen.
/// Walk: the feet plant alternately. Tell: both feet lifted and splayed.
/// Strike: crown slammed low. Rest: crown tilted, feet stuck.
pub(crate) fn draw_staplecrawler(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.0);
    let (cx, cy) = centre(fr);
    let phase = e.w4.phase;
    let half = (4.0 * s).round().max(2.0) as i32;
    // Crown height above the ground line and per-foot lift.
    let (crown, lift_l, lift_r, splay, tilt) = match phase {
        sim::STAPLE_PHASE_TELL => (6, 3, 3, 1, 0),
        sim::STAPLE_PHASE_STRIKE => (2, 0, 0, 0, 0),
        sim::STAPLE_PHASE_REST => (4, 0, 0, 0, 1),
        _ => {
            if step(fr, 0.6) {
                (5, 2, 0, 0, 0)
            } else {
                (5, 0, 2, 0, 0)
            }
        }
    };
    let ground = cy + 3;
    let top = ground - (crown as f32 * s).round().max(2.0) as i32;
    let steel = tone(fr, STEEL);
    let hi = tone(fr, STEEL_HI);
    let dark = tone(fr, STEEL_DARK);
    // Ground shadow under the span.
    line(pb, (cx - half + 1, ground + 1), (cx + half + 1, ground + 1), (0, 0, 0), 0.3);
    // Legs (left then right), each ending in a needle point.
    let legs = [(-1, lift_l), (1, lift_r)];
    for (side, lift) in legs {
        let x_top = cx + side * half;
        let x_foot = x_top + side * splay;
        let y_top = top + if side > 0 { tilt } else { 0 };
        let y_foot = ground - lift;
        line(pb, (x_top, y_top), (x_foot, y_foot), steel, 1.0);
        // Inner dark edge, then the needle.
        line(pb, (x_top - side, y_top + 1), (x_foot - side, y_foot - 1), dark, 0.75);
        put(pb, x_foot, y_foot, if lift == 0 { dark } else { hi }, 1.0);
    }
    // Crown: light top edge over a steel underside.
    line(pb, (cx - half, top), (cx + half, top + tilt), hi, 1.0);
    line(pb, (cx - half + 1, top + 1), (cx + half - 1, top + 1 + tilt), steel, 1.0);
}

pub(crate) fn draw_staplecrawler_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let col = tone(fr, e.kind.color());
    let hi = tone(fr, STEEL_HI);
    line(pb, (cx - 2, cy - 2), (cx + 2, cy - 2), hi, 1.0);
    let raised = e.w4.phase == sim::STAPLE_PHASE_TELL;
    let (l, r) = if raised {
        (0, 0)
    } else if step(fr, 0.6) {
        (2, 1)
    } else {
        (1, 2)
    };
    line(pb, (cx - 2, cy - 1), (cx - 2, cy - 1 + l), col, 1.0);
    line(pb, (cx + 2, cy - 1), (cx + 2, cy - 1 + r), col, 1.0);
}

// ---- Paper Wasp (R, r 3.0) -------------------------------------------------

/// Folded arrow wings (a paper dart) over a rolled newsprint abdomen. The
/// tube's tip opens and glows in the last 0.4 s before the shot.
pub(crate) fn draw_paperwasp(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 3.0);
    let lit = lit_side(fr);
    let opening = e.fire_cd < 0.4;
    let wing_w = if step(fr, 0.35) { 4.2 } else { 3.0 };
    let paper = tone(fr, (206, 196, 164));
    let hi = tone(fr, PAPER_HI);
    let crease = tone(fr, CREASE);
    // Rolled abdomen: a striped tube coiling behind the dart.
    let reach = if opening { 6.8 } else { 5.6 };
    let mut u = -1.5f32;
    let mut k = 0;
    while u >= -reach {
        let coil = ((u * 1.4) + fr.anim * 0.1).sin() * 0.9 * s;
        let c = at(fr, u * s, coil);
        shade(pb, c.0 + 1, c.1 + 1, 0.28);
        let band = if k % 2 == 0 { paper } else { crease };
        put(pb, c.0, c.1, band, 1.0);
        let side = at(fr, u * s, coil + 1.0 * lit);
        put(pb, side.0, side.1, hi, 0.9);
        let under = at(fr, u * s, coil - 1.0 * lit);
        put(pb, under.0, under.1, crease, 0.8);
        u -= 0.8;
        k += 1;
    }
    let tip = at(fr, -reach * s, 0.0);
    if opening {
        disc(pb, tip.0, tip.1, 1, WARN, 0.9);
        put(pb, tip.0, tip.1, WARN_HI, 1.0);
    } else {
        put(pb, tip.0, tip.1, crease, 1.0);
    }
    // The dart: nose forward, two folded wings, a centre crease.
    let nose = at(fr, 5.0 * s, 0.0);
    let heel = at(fr, -1.8 * s, 0.0);
    for side in [-1.0f32, 1.0] {
        let wing = at(fr, -2.2 * s, side * wing_w * s);
        // Fill the wing with a short fan of lines from the nose.
        for j in 0..=4 {
            let t = j as f32 / 4.0;
            let m = (
                heel.0 + ((wing.0 - heel.0) as f32 * t).round() as i32,
                heel.1 + ((wing.1 - heel.1) as f32 * t).round() as i32,
            );
            line(pb, nose, m, paper, 1.0);
        }
        let edge = if side * lit > 0.0 { hi } else { crease };
        line(pb, nose, wing, edge, 1.0);
    }
    line(pb, nose, heel, crease, 0.8);
    put(pb, nose.0, nose.1, hi, 1.0);
}

pub(crate) fn draw_paperwasp_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 3.0);
    let col = tone(fr, e.kind.color());
    let nose = at(fr, 3.5 * s, 0.0);
    let tail = at(fr, -3.5 * s, 0.0);
    line(pb, tail, nose, col, 1.0);
    for side in [-1.0f32, 1.0] {
        let w = at(fr, -0.5 * s, side * 2.0 * s);
        put(pb, w.0, w.1, tone(fr, PAPER_HI), 1.0);
    }
    let tip_col = if e.fire_cd < 0.4 { WARN } else { tone(fr, CREASE) };
    put(pb, tail.0, tail.1, tip_col, 1.0);
}

// ---- Paper Reel (H, r 6.5) -------------------------------------------------

/// Wide spool rolling broadside: banded paper, kraft end caps with a dark
/// axle hole, and a loose end that flaps behind. The elite wears a pair of
/// metal flanges and a thick double shoulder stripe.
pub(crate) fn draw_paperreel(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 6.5);
    let (hu, hv) = (4.0 * s, 6.5 * s);
    // Loose end: a strip trailing from the back, flapping in two frames.
    let flap = if step(fr, 0.45) { 1.6 } else { -1.2 };
    for v in [-2.0f32, -1.0, 0.0, 1.0, 2.0] {
        let a = at(fr, -hu, v * s);
        let b = at(fr, -hu - 3.2 * s, (v + flap) * s);
        line(pb, a, b, tone(fr, if v == 2.0 { PAPER_DARK } else { PAPER }), 0.95);
    }
    // Contact shadow.
    let cols = (hu * 2.0).ceil() as i32;
    for i in 0..=cols {
        let u = -hu + i as f32 * (2.0 * hu / cols.max(1) as f32);
        let a = at(fr, u, -hv);
        let b = at(fr, u, hv);
        line(pb, (a.0 + 1, a.1 + 2), (b.0 + 1, b.1 + 2), (0, 0, 0), 0.22);
    }
    // The roll: columns along `u`, shaded as a cylinder (bright crown, dark
    // flanks) with a wound-paper line that crawls as it turns.
    let wind = (fr.anim * 0.35).rem_euclid(3.0);
    for i in 0..=cols {
        let u = -hu + i as f32 * (2.0 * hu / cols.max(1) as f32);
        let t = (u / hu).clamp(-1.0, 1.0);
        let round = 1.0 - t * t;
        let mut col = mix(PAPER_DARK, PAPER_HI, 0.15 + 0.85 * round);
        if ((u + hu + wind) % 3.0) < 0.9 {
            col = mix(col, CREASE, 0.35);
        }
        line(pb, at(fr, u, -hv), at(fr, u, hv), tone(fr, col), 1.0);
    }
    // Leading (light) and trailing (dark) edges.
    line(pb, at(fr, hu, -hv), at(fr, hu, hv), tone(fr, PAPER_DARK), 1.0);
    line(pb, at(fr, -hu, -hv), at(fr, -hu, hv), tone(fr, CREASE), 1.0);
    // End caps with the dark axle hole.
    for side in [-1.0f32, 1.0] {
        let cap_col = if elite { STEEL } else { KRAFT };
        let reach = if elite { hu + 1.5 * s } else { hu };
        line(pb, at(fr, -reach, side * hv), at(fr, reach, side * hv), tone(fr, cap_col), 1.0);
        line(
            pb,
            at(fr, -reach, side * (hv + 1.0)),
            at(fr, reach, side * (hv + 1.0)),
            tone(fr, if elite { STEEL_HI } else { mix(KRAFT, PAPER, 0.35) }),
            1.0,
        );
        let hole = at(fr, 0.0, side * (hv + 0.5));
        put(pb, hole.0, hole.1, INK, 1.0);
        let hole2 = at(fr, 0.6, side * (hv + 0.5));
        put(pb, hole2.0, hole2.1, INK, 1.0);
    }
    if elite {
        // Thick double shoulder stripe.
        for v in [-2.4f32, -1.6, 1.6, 2.4] {
            line(pb, at(fr, -hu, v * s), at(fr, hu, v * s), tone(fr, (196, 60, 48)), 0.9);
        }
    }
}

pub(crate) fn draw_paperreel_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r * (1.0 / 6.5);
    let col = tone(fr, e.kind.color());
    let (hu, hv) = (2.5 * s, 5.0 * s);
    for u in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
        let c = if u == 0.0 { tone(fr, PAPER_HI) } else { mix(col, PAPER, 0.5 - 0.3 * u.abs()) };
        line(pb, at(fr, u * hu, -hv), at(fr, u * hu, hv), c, 1.0);
    }
    for side in [-1.0f32, 1.0] {
        let cap = at(fr, 0.0, side * hv);
        put(pb, cap.0, cap.1, if elite { tone(fr, STEEL_HI) } else { INK }, 1.0);
    }
}

// ---- Ink Blot (III, r 2.5) -------------------------------------------------

/// The blot's edge radius at screen angle `a`: a five-armed ragged star that
/// flows out, then contracts.
#[inline]
fn blot_radius(a: f32, anim: f32, s: f32) -> f32 {
    let pulse = 0.82 + 0.26 * (anim * 0.35).sin();
    let arms = (a * 5.0 + anim * 0.12).cos() * 0.34 + (a * 3.0 - anim * 0.2).sin() * 0.12;
    3.6 * s * pulse * (1.0 + arms)
}

/// Ragged black star with a porcelain outline.
pub(crate) fn draw_inkblot(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r * (1.0 / 2.5);
    let (cx, cy) = centre(fr);
    let reach = (6.0 * s).ceil() as i32 + 1;
    let ink = tone(fr, INK);
    let sheen = tone(fr, mix(INK, e.kind.color(), 0.8));
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let a = (dy as f32).atan2(dx as f32);
            let edge = blot_radius(a, fr.anim, s);
            if d <= edge {
                // A violet sheen on the upper-left of the pool.
                let lit = dx + dy < -1 && d > edge * 0.35;
                put(pb, cx + dx, cy + dy, if lit { sheen } else { ink }, 1.0);
            } else if d <= edge + 1.15 {
                // Porcelain outline: bright toward the light, dim away from it.
                let rim = if dx + dy <= 0 { PORCELAIN } else { PORCELAIN_DIM };
                put(pb, cx + dx, cy + dy, tone(fr, rim), 1.0);
            }
        }
    }
    put(pb, cx - 1, cy - 1, tone(fr, PORCELAIN), 0.8);
}

pub(crate) fn draw_inkblot_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let wide = step(fr, 0.35);
    let rim = tone(fr, PORCELAIN);
    let ink = tone(fr, INK);
    let arm = if wide { 2 } else { 1 };
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        put(pb, cx + dx * (arm + 1), cy + dy * (arm + 1), rim, 1.0);
        for k in 1..=arm {
            put(pb, cx + dx * k, cy + dy * k, ink, 1.0);
        }
    }
    put(pb, cx, cy, ink, 1.0);
    put(pb, cx - 1, cy - 1, rim, 1.0);
    put(pb, cx + 1, cy + 1, tone(fr, PORCELAIN_DIM), 1.0);
}

/// The Ink Blot's travelling burst: the marked endpoint, shown for the whole
/// fuse. `end` is `InkSlide::end` (world px) and `fuse_frac` runs 1 → 0.
///
/// A porcelain ring at the true burst radius with a black centre that fills as
/// the fuse burns. `World::draw_hostile_layer` should stamp it for every wick
/// that carries an `InkSlide` (see the batch report).
pub fn draw_ink_endpoint(pb: &mut PixelBuffer, end: Vec2, cam: Vec2, fuse_frac: f32) {
    let p = end - cam;
    let (cx, cy) = (p.x.round() as i32, p.y.round() as i32);
    let rad = crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS;
    let k = 1.0 - fuse_frac.clamp(0.0, 1.0);
    dotted_ring(pb, cx, cy, rad, PORCELAIN, 0.95);
    let core = (rad * k).floor() as i32;
    if core >= 1 {
        disc(pb, cx, cy, core, INK, 0.45 + 0.35 * k);
    }
    line(pb, (cx - 1, cy), (cx + 1, cy), WARN, 1.0);
    line(pb, (cx, cy - 1), (cx, cy + 1), WARN, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const KINDS: [EnemyKind; 7] = [
        EnemyKind::Silverfish,
        EnemyKind::ErratumSlip,
        EnemyKind::TypeSlug,
        EnemyKind::StapleCrawler,
        EnemyKind::PaperWasp,
        EnemyKind::PaperReel,
        EnemyKind::InkBlot,
    ];

    fn body(kind: EnemyKind, anim: f32) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(32.0, 32.0), 1.0, &mut rng);
        e.anim = anim;
        e
    }

    fn painted(pb: &PixelBuffer, bg: Rgb) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) != bg {
                    out.push((x, y));
                }
            }
        }
        out
    }

    fn render(kind: EnemyKind, anim: f32, lod: bool) -> PixelBuffer {
        let bg = (40, 40, 44);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(bg);
        let e = body(kind, anim);
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (255, 255, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (255, 255, 255));
        }
        pb
    }

    #[test]
    fn every_printworks_body_paints_a_light_and_a_dark_edge() {
        let bg = (40, 40, 44);
        for kind in KINDS {
            let pb = render(kind, 1.0, false);
            let px = painted(&pb, bg);
            assert!(px.len() >= 8, "{kind:?} drew only {} pixels", px.len());
            let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            let hi = px.iter().map(|&(x, y)| lum(pb.pixel_at(x, y))).max().unwrap();
            let lo = px.iter().map(|&(x, y)| lum(pb.pixel_at(x, y))).min().unwrap();
            assert!(hi >= 480, "{kind:?} has no light edge (max {hi})");
            assert!(lo <= 260, "{kind:?} has no dark edge (min {lo})");
        }
    }

    #[test]
    fn every_printworks_body_stays_inside_its_cull_box() {
        // The widest archetype bound here is the Brute's 13.7 px (the flea's is
        // wider still); nothing may paint further than 16 px from the centre.
        let bg = (40, 40, 44);
        for kind in KINDS {
            for lod in [false, true] {
                let pb = render(kind, 2.3, lod);
                for (x, y) in painted(&pb, bg) {
                    let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                    assert!(d <= 16.0, "{kind:?} lod={lod} painted {d} px out");
                }
            }
        }
    }

    #[test]
    fn every_printworks_body_has_two_walk_frames() {
        for kind in KINDS {
            let frames: Vec<PixelBuffer> =
                [0.0f32, 1.3, 2.9, 4.4, 7.0].iter().map(|&a| render(kind, a, false)).collect();
            let differs = frames.windows(2).any(|w| w[0].pixels() != w[1].pixels());
            assert!(differs, "{kind:?} never changes pose");
        }
    }

    #[test]
    fn far_bodies_are_small_and_present() {
        let bg = (40, 40, 44);
        for kind in KINDS {
            let pb = render(kind, 1.0, true);
            let n = painted(&pb, bg).len();
            assert!((3..=90).contains(&n), "{kind:?} far body is {n} pixels");
        }
    }

    #[test]
    fn the_staple_footprint_is_the_radius_the_strike_uses() {
        let bg = (40, 40, 44);
        let mut pb = PixelBuffer::new(64, 64);
        pb.clear(bg);
        let mut e = body(EnemyKind::StapleCrawler, 0.0);
        e.pos = Vec2::new(10.0, 10.0);
        e.w4.phase = sim::STAPLE_PHASE_STRIKE;
        e.w4.t = sim::STAPLE_STRIKE_SECS;
        e.w4.trail[0] = Vec2::new(30.0, 40.0);
        e.w4.trail[1] = Vec2::new(44.0, 40.0);
        draw_staple_tell(&mut pb, &e, Vec2::ZERO);
        let px = painted(&pb, bg);
        assert!(!px.is_empty());
        for (x, y) in px {
            let p = Vec2::new(x as f32, y as f32);
            let d = p.dist(e.w4.trail[0]).min(p.dist(e.w4.trail[1]));
            assert!(d <= sim::STAPLE_FOOT_RADIUS + 0.75, "footprint pixel {d} px from a foot");
        }
        // Walking: nothing at all.
        let mut clean = PixelBuffer::new(64, 64);
        clean.clear(bg);
        e.w4.phase = sim::STAPLE_PHASE_WALK;
        draw_staple_tell(&mut clean, &e, Vec2::ZERO);
        assert!(painted(&clean, bg).is_empty());
    }
}
