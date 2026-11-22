//! The Kitchen's own bodies (`WAVE4_ROSTERS.md`, "Kitchen"): grease-dark
//! shapes against pale counter tile.
//!
//! | Kind | Silhouette | Walk frames | Tell pose |
//! |---|---|---|---|
//! | Crumb Ant | three dark beads carrying a pale square crumb ahead | legs swap sides | — |
//! | Fruit Fly | two upright seed wings over a round red eye | wings shut / spread, tight bob | — |
//! | Egg Timer | upright round clock with two key ears | feet swap, body rocks | hand sweeps the dial while winding, face glows while the ring is out, face dims asleep |
//! | Pepper Mill | upright waisted wooden pawn under a steel cap | base rocks | the cap twists side to side before the shot |
//! | Pie Weight | ceramic ball on a crimped tin skirt | crimps roll, ball sways | — (elite: a doubled gold pie rim on the skirt and a thick double gold shoulder stripe across the ball) |
//! | Sink Crab | broad flat shell, one steel pot-lid claw | legs scuttle | the lid swings up into a bar across the way it guards |
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

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_kitchen as sim;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

// ---- Shared helpers ---------------------------------------------------------

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

/// A body colour with the hit flash folded in.
#[inline]
fn tone(fr: &Frame, c: Rgb) -> Rgb {
    if fr.hit <= 0.0 {
        c
    } else {
        mix(c, (255, 236, 190), (0.30 + fr.hit * 0.56).min(0.86))
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

/// Screen pixel of the body-local point `(u forward, v sideways)`.
#[inline]
fn at(fr: &Frame, u: f32, v: f32) -> (i32, i32) {
    (
        (fr.pos.x + fr.f.x * u + fr.p.x * v).round() as i32,
        (fr.pos.y + fr.f.y * u + fr.p.y * v).round() as i32,
    )
}

/// The body's screen centre.
#[inline]
fn centre(fr: &Frame) -> (i32, i32) {
    (fr.pos.x.round() as i32, fr.pos.y.round() as i32)
}

/// Which of the two walk frames is showing.
#[inline]
fn step(fr: &Frame, rate: f32) -> bool {
    (fr.anim * rate).sin() > 0.0
}

/// Darken one pixel (the ground shadow), bounds-checked.
#[inline]
fn dim(pb: &mut PixelBuffer, x: i32, y: i32, keep: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let c = pb.pixel_at(x as usize, y as usize);
    pb.plot(
        x,
        y,
        (
            (c.0 as f32 * keep) as u8,
            (c.1 as f32 * keep) as u8,
            (c.2 as f32 * keep) as u8,
        ),
    );
}

/// A shaded ellipse centred at screen `(cx, cy)` with semi-axes `ra` along the
/// unit axis `ax` and `rb` across it. The upper-left edge takes the light
/// colour and the lower-right edge the dark one, so every body carries both.
fn blob(pb: &mut PixelBuffer, cx: f32, cy: f32, ax: Vec2, ra: f32, rb: f32, c: Pal) {
    let ra = ra.max(0.5);
    let rb = rb.max(0.5);
    let reach = ra.max(rb).ceil() as i32 + 1;
    let (ix, iy) = (cx.round() as i32, cy.round() as i32);
    let big = ra.max(rb);
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (fx, fy) = ((ix + dx) as f32 - cx, (iy + dy) as f32 - cy);
            let a = (fx * ax.x + fy * ax.y) / ra;
            let b = (-fx * ax.y + fy * ax.x) / rb;
            let d = a * a + b * b;
            if d > 1.0 {
                continue;
            }
            // Light comes from the upper left of the screen.
            let lit = -(fx + fy) / (big * 1.5);
            let edge = d > 0.45;
            let col = if edge && lit > 0.2 {
                c.hi
            } else if edge && lit < -0.2 {
                c.lo
            } else {
                c.mid
            };
            pb.plot(ix + dx, iy + dy, col);
        }
    }
}

/// A round shaded blob (screen-aligned).
#[inline]
fn ball(pb: &mut PixelBuffer, cx: f32, cy: f32, r: f32, c: Pal) {
    blob(pb, cx, cy, Vec2::new(1.0, 0.0), r, r, c);
}

/// A soft drop shadow one pixel down-right of a round footprint.
fn shadow(pb: &mut PixelBuffer, cx: i32, cy: i32, r: i32) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                dim(pb, cx + dx + 1, cy + dy + 1, 0.72);
            }
        }
    }
}

/// A horizontal run of pixels `x0..=x1` on row `y`, light at the left end and
/// dark at the right end.
fn row(pb: &mut PixelBuffer, x0: i32, x1: i32, y: i32, c: Pal) {
    for x in x0..=x1 {
        let col = if x == x0 && x1 > x0 {
            c.hi
        } else if x == x1 && x1 > x0 {
            c.lo
        } else {
            c.mid
        };
        pb.plot(x, y, col);
    }
}

// ---- Crumb Ant (V Skitterer, r 2.5) -----------------------------------------

const ANT_HI: Rgb = (150, 108, 72);
const ANT_MID: Rgb = (92, 66, 44);
const ANT_LO: Rgb = (40, 26, 16);
const CRUMB_HI: Rgb = (255, 244, 206);
const CRUMB_MID: Rgb = (236, 212, 156);
const CRUMB_LO: Rgb = (150, 118, 70);

/// Three beads and a square crumb, marching.
pub(crate) fn draw_crumbant(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 2.5;
    let body = pal(fr, ANT_HI, ANT_MID, ANT_LO);
    let leg = tone(fr, ANT_LO);
    let (cx, cy) = centre(fr);
    shadow(pb, cx, cy, (2.0 * s).round() as i32);

    // Six legs in two frames: the tripod swaps sides.
    let swap = step(fr, 1.0);
    for (i, u) in [1.2f32, 0.0, -1.4].into_iter().enumerate() {
        let lead = (i % 2 == 0) == swap;
        for side in [-1.0f32, 1.0] {
            let fwd = if lead == (side > 0.0) { 0.8 } else { -0.8 };
            let (x, y) = at(fr, (u + fwd) * s, side * 2.6 * s);
            pb.plot(x, y, leg);
            let (x, y) = at(fr, u * s, side * 1.7 * s);
            pb.plot(x, y, leg);
        }
    }

    // Abdomen, thorax, head: three separate beads.
    let (ax, ay) = (fr.pos.x - fr.f.x * 2.4 * s, fr.pos.y - fr.f.y * 2.4 * s);
    ball(pb, ax, ay, 1.6 * s, body);
    ball(pb, fr.pos.x, fr.pos.y, 1.0 * s, body);
    let (hx, hy) = (fr.pos.x + fr.f.x * 1.9 * s, fr.pos.y + fr.f.y * 1.9 * s);
    ball(pb, hx, hy, 1.2 * s, body);

    // The crumb: a pale square held ahead of the jaws, one bright corner and
    // one dark corner.
    let (qx, qy) = at(fr, 4.0 * s, 0.0);
    pb.plot(qx, qy, tone(fr, CRUMB_MID));
    pb.plot(qx - 1, qy - 1, tone(fr, CRUMB_HI));
    pb.plot(qx, qy - 1, tone(fr, CRUMB_MID));
    pb.plot(qx - 1, qy, tone(fr, CRUMB_MID));
    pb.plot(qx + 1, qy, tone(fr, CRUMB_LO));
    pb.plot(qx, qy + 1, tone(fr, CRUMB_LO));
    pb.plot(qx + 1, qy + 1, tone(fr, CRUMB_LO));
    pb.plot(qx + 1, qy - 1, tone(fr, CRUMB_MID));
    pb.plot(qx - 1, qy + 1, tone(fr, CRUMB_MID));
}

/// Far LOD: three dark dots and the pale crumb.
pub(crate) fn draw_crumbant_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 2.5;
    let (x, y) = at(fr, -2.0 * s, 0.0);
    dim(pb, x + 1, y + 1, 0.72);
    pb.plot(x, y, tone(fr, ANT_LO));
    let (x, y) = centre(fr);
    pb.plot(x, y, tone(fr, ANT_MID));
    let (x, y) = at(fr, 1.8 * s, 0.0);
    pb.plot(x, y, tone(fr, ANT_HI));
    let (x, y) = at(fr, 3.4 * s, 0.0);
    pb.plot(x, y, tone(fr, CRUMB_HI));
    // Two feet that swap, so the file still marches at distance.
    let side = if step(fr, 1.0) { 1.6 } else { -1.6 };
    let (x, y) = at(fr, 0.0, side * s);
    pb.plot(x, y, tone(fr, ANT_LO));
}

// ---- Fruit Fly (R Bat, r 2.0) -----------------------------------------------

const FLY_EYE_HI: Rgb = (255, 168, 136);
const FLY_EYE_MID: Rgb = (212, 56, 44);
const FLY_EYE_LO: Rgb = (104, 20, 22);
const FLY_WING_HI: Rgb = (240, 244, 232);
const FLY_WING_LO: Rgb = (132, 140, 128);
const FLY_BODY: Rgb = (58, 40, 30);

/// Two upright seed wings over a round red eye, tight bob.
pub(crate) fn draw_fruitfly(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 2.0;
    let (cx, cy0) = centre(fr);
    // The fly hangs above its shadow and bobs a pixel.
    dim(pb, cx, cy0 + 4, 0.72);
    dim(pb, cx + 1, cy0 + 4, 0.72);
    let bob = if (fr.anim * 0.35).sin() > 0.0 { 0 } else { -1 };
    let cy = cy0 + bob;

    // Wings: two tall seeds rising from the back, shut on one frame and
    // spread on the other.
    let spread = if step(fr, 1.0) { 2 } else { 1 };
    let tall = (4.0 * s).round() as i32;
    for side in [-1i32, 1] {
        for k in 1..=tall {
            let lean = if k > tall / 2 { spread } else { spread - 1 };
            let x = cx + side * (1 + lean);
            let y = cy - 1 - k;
            pb.plot(x, y, tone(fr, if side < 0 { FLY_WING_HI } else { FLY_WING_LO }));
        }
    }
    // A dark thorax under the eye, then the eye itself.
    pb.plot(cx - 1, cy + 2, tone(fr, FLY_BODY));
    pb.plot(cx, cy + 2, tone(fr, FLY_BODY));
    pb.plot(cx + 1, cy + 2, tone(fr, FLY_BODY));
    ball(
        pb,
        cx as f32,
        cy as f32,
        2.0 * s,
        pal(fr, FLY_EYE_HI, FLY_EYE_MID, FLY_EYE_LO),
    );
    pb.plot(cx - 1, cy - 1, tone(fr, (255, 232, 214)));
}

/// Far LOD: a red block with two wing pixels standing over it.
pub(crate) fn draw_fruitfly_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let open = if step(fr, 1.0) { 1 } else { 0 };
    pb.plot(cx - 1 - open, cy - 2, tone(fr, FLY_WING_HI));
    pb.plot(cx + 1 + open, cy - 2, tone(fr, FLY_WING_LO));
    pb.plot(cx - 1, cy - 1, tone(fr, FLY_WING_HI));
    pb.plot(cx + 1, cy - 1, tone(fr, FLY_WING_LO));
    pb.plot(cx, cy, tone(fr, FLY_EYE_HI));
    pb.plot(cx + 1, cy, tone(fr, FLY_EYE_MID));
    pb.plot(cx, cy + 1, tone(fr, FLY_EYE_MID));
    pb.plot(cx + 1, cy + 1, tone(fr, FLY_EYE_LO));
}

// ---- Egg Timer (N, r 5.0) ---------------------------------------------------

const EGG_HI: Rgb = (255, 250, 236);
const EGG_MID: Rgb = (222, 214, 196);
const EGG_LO: Rgb = (112, 100, 86);
const EGG_BRASS_HI: Rgb = (250, 214, 120);
const EGG_BRASS_LO: Rgb = (150, 104, 40);
const EGG_HAND: Rgb = (214, 44, 40);
const EGG_INK: Rgb = (44, 38, 36);
const RING_HOT: Rgb = (255, 96, 60);
const RING_EDGE: Rgb = (255, 226, 150);
const RING_TELL: Rgb = (255, 150, 90);

/// The dial's face colours for the current phase.
fn egg_face(fr: &Frame, e: &Enemy) -> Pal {
    match e.w4.phase {
        // Ringing: the face glows.
        sim::EGG_PHASE_TICK => pal(fr, (255, 252, 240), (255, 222, 170), (170, 96, 60)),
        // Asleep: the face is dull.
        sim::EGG_PHASE_SLEEP => pal(fr, (176, 172, 164), (140, 136, 128), (70, 66, 62)),
        _ => pal(fr, EGG_HI, EGG_MID, EGG_LO),
    }
}

/// Where the hand points, as a unit vector in screen space.
fn egg_hand(fr: &Frame, e: &Enemy) -> (f32, f32) {
    let turn = match e.w4.phase {
        // Winding: one full sweep back to twelve as the tell runs out.
        sim::EGG_PHASE_WIND => {
            (1.0 - e.w4.t / sim::EGG_WIND_SECS).clamp(0.0, 1.0) * std::f32::consts::TAU
        }
        // Ringing: stuck at twelve.
        sim::EGG_PHASE_TICK => 0.0,
        // Asleep: fallen to six.
        sim::EGG_PHASE_SLEEP => std::f32::consts::PI,
        // Walking: idling around the dial.
        _ => fr.anim * 0.25,
    };
    (turn.sin(), -turn.cos())
}

/// Round clock, key ears, waddles until the dial stops.
pub(crate) fn draw_eggtimer(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let (cx, cy0) = centre(fr);
    let walking = e.w4.phase == sim::EGG_PHASE_WALK;
    let left_up = walking && step(fr, 0.5);
    let right_up = walking && !step(fr, 0.5);
    // The waddle: the body leans a pixel toward the planted foot.
    let lean = if left_up {
        1
    } else if right_up {
        -1
    } else {
        0
    };
    // Winding: the whole clock shivers as the key turns.
    let shiver = if e.w4.phase == sim::EGG_PHASE_WIND && (e.w4.t * 30.0).sin() > 0.0 {
        1
    } else {
        0
    };
    let cx = cx + lean + shiver;
    let cy = cy0 - 1;
    let rad = (4.5 * s).round() as i32;
    shadow(pb, cx, cy + 1, rad);

    // Feet.
    let foot = tone(fr, EGG_BRASS_LO);
    let fy = cy + rad + 1;
    pb.plot(cx - 2, fy - i32::from(left_up), foot);
    pb.plot(cx - 3, fy - i32::from(left_up), foot);
    pb.plot(cx + 2, fy - i32::from(right_up), foot);
    pb.plot(cx + 3, fy - i32::from(right_up), foot);

    // Key ears: two brass loops standing off the top of the case.
    for side in [-1i32, 1] {
        let ex = cx + side * 3;
        let ey = cy - rad - 1;
        let wag = if e.w4.phase == sim::EGG_PHASE_WIND && (e.w4.t * 16.0).sin() * side as f32 > 0.0 {
            1
        } else {
            0
        };
        pb.plot(ex, ey + 1, tone(fr, EGG_BRASS_LO));
        pb.plot(ex - 1, ey - wag, tone(fr, EGG_BRASS_HI));
        pb.plot(ex + 1, ey - wag, tone(fr, EGG_BRASS_LO));
        pb.plot(ex, ey - 1 - wag, tone(fr, EGG_BRASS_HI));
    }

    // The case and the face.
    ball(pb, cx as f32, cy as f32, 4.5 * s, egg_face(fr, e));
    let ink = tone(fr, EGG_INK);
    let mark = rad - 1;
    pb.plot(cx, cy - mark, ink);
    pb.plot(cx, cy + mark, ink);
    pb.plot(cx - mark, cy, ink);
    pb.plot(cx + mark, cy, ink);
    // The hand.
    let (hx, hy) = egg_hand(fr, e);
    let hand = tone(fr, EGG_HAND);
    for k in 1..=(rad - 1).max(1) {
        pb.plot(
            cx + (hx * k as f32).round() as i32,
            cy + (hy * k as f32).round() as i32,
            hand,
        );
    }
    pb.plot(cx, cy, ink);
}

/// Far LOD: a pale disc, two ear pixels and the red hand.
pub(crate) fn draw_eggtimer_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let (cx, cy) = centre(fr);
    let rad = (3.0 * s).round().max(2.0) as i32;
    dim(pb, cx + rad, cy + rad, 0.72);
    pb.plot(cx - 2, cy - rad - 1, tone(fr, EGG_BRASS_HI));
    pb.plot(cx + 2, cy - rad - 1, tone(fr, EGG_BRASS_LO));
    ball(pb, cx as f32, cy as f32, 3.0 * s, egg_face(fr, e));
    let (hx, hy) = egg_hand(fr, e);
    pb.plot(cx, cy, tone(fr, EGG_HAND));
    pb.plot(
        cx + (hx * 1.6).round() as i32,
        cy + (hy * 1.6).round() as i32,
        tone(fr, EGG_HAND),
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
/// dotted, with the dots closing up as the wind runs out.
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
        for k in 0..48 {
            if k % skip != 0 {
                continue;
            }
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            let (sn, cs) = a.sin_cos();
            pb.plot(
                (c.x + cs * outer).round() as i32,
                (c.y + sn * outer).round() as i32,
                RING_TELL,
            );
            if k % 4 == 0 {
                pb.plot(
                    (c.x + cs * inner).round() as i32,
                    (c.y + sn * inner).round() as i32,
                    RING_TELL,
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
            // A bright leading edge on the outside of the band.
            let d = (p.x * p.x + p.y * p.y).sqrt();
            let col = if d > radius + half - 1.2 { RING_EDGE } else { RING_HOT };
            pb.plot(ix + dx, iy + dy, col);
        }
    }
}

// ---- Pepper Mill (V Spitter, r 3.0) -----------------------------------------

const MILL_HI: Rgb = (232, 184, 124);
const MILL_MID: Rgb = (168, 120, 72);
const MILL_LO: Rgb = (84, 52, 28);
const MILL_CAP_HI: Rgb = (244, 246, 250);
const MILL_CAP_MID: Rgb = (190, 196, 206);
const MILL_CAP_LO: Rgb = (96, 102, 116);
/// Seconds before the shot that the cap starts twisting.
const MILL_TELL_SECS: f32 = 0.6;

/// How far the cap is twisted this frame (−1, 0 or 1 pixels).
fn mill_twist(e: &Enemy) -> i32 {
    // Only the last stretch of a running cooldown: a mill that is primed but
    // out of range or behind cover (`fire_cd <= 0`) stands still.
    if e.fire_cd > MILL_TELL_SECS || e.fire_cd <= 0.0 {
        return 0;
    }
    if (e.fire_cd * 26.0).sin() > 0.0 {
        1
    } else {
        -1
    }
}

/// Waisted wooden pawn twists its cap.
pub(crate) fn draw_peppermill(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let wood = pal(fr, MILL_HI, MILL_MID, MILL_LO);
    let twist = mill_twist(e);
    let telling = twist != 0;
    let cap = if telling {
        // The cap heats up as it grinds.
        pal(fr, (255, 250, 220), (255, 214, 130), (176, 110, 50))
    } else {
        pal(fr, MILL_CAP_HI, MILL_CAP_MID, MILL_CAP_LO)
    };
    // The base rocks a pixel between the two walk frames.
    let rock = if step(fr, 0.5) { 1 } else { 0 };
    shadow(pb, cx, cy + 3, 3);

    // Base, lower body, waist, upper body, shoulder, neck.
    row(pb, cx - 3 + rock, cx + 3 + rock, cy + 4, wood);
    row(pb, cx - 2, cx + 2, cy + 3, wood);
    row(pb, cx - 2, cx + 2, cy + 2, wood);
    row(pb, cx - 1, cx + 1, cy + 1, wood);
    row(pb, cx - 2, cx + 2, cy, wood);
    row(pb, cx - 2, cx + 2, cy - 1, wood);
    row(pb, cx - 1, cx + 1, cy - 2, wood);
    // The cap: a steel dome with a knob, twisting off centre before a shot.
    row(pb, cx - 2 + twist, cx + 2 + twist, cy - 3, cap);
    row(pb, cx - 1 + twist, cx + 1 + twist, cy - 4, cap);
    pb.plot(cx + twist, cy - 5, cap.hi);
    if telling {
        // Two specks of pepper jump off the cap, one for each ray.
        pb.plot(cx - 3 - twist, cy - 5, tone(fr, (60, 44, 36)));
        pb.plot(cx + 3 - twist, cy - 5, tone(fr, (60, 44, 36)));
    }
}

/// Far LOD: a three-wide wooden column with a pinched waist and a steel cap.
pub(crate) fn draw_peppermill_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let (cx, cy) = centre(fr);
    let wood = pal(fr, MILL_HI, MILL_MID, MILL_LO);
    let twist = mill_twist(e);
    dim(pb, cx + 2, cy + 3, 0.72);
    row(pb, cx - 1, cx + 1, cy + 2, wood);
    row(pb, cx - 1, cx + 1, cy + 1, wood);
    pb.plot(cx, cy, wood.mid);
    row(pb, cx - 1, cx + 1, cy - 1, wood);
    let cap = if twist != 0 {
        tone(fr, (255, 214, 130))
    } else {
        tone(fr, MILL_CAP_MID)
    };
    row(
        pb,
        cx - 1 + twist,
        cx + 1 + twist,
        cy - 2,
        Pal {
            hi: tone(fr, MILL_CAP_HI),
            mid: cap,
            lo: tone(fr, MILL_CAP_LO),
        },
    );
    pb.plot(cx + twist, cy - 3, tone(fr, MILL_CAP_HI));
}

// ---- Pie Weight (R Brute, r 6.5) --------------------------------------------

const PIE_BALL_HI: Rgb = (255, 255, 250);
const PIE_BALL_MID: Rgb = (226, 222, 212);
const PIE_BALL_LO: Rgb = (128, 124, 132);
const PIE_TIN_HI: Rgb = (214, 220, 230);
const PIE_TIN_MID: Rgb = (150, 156, 168);
const PIE_TIN_LO: Rgb = (70, 74, 88);
const PIE_RIM: Rgb = (246, 204, 92);
const PIE_RIM_LO: Rgb = (150, 104, 36);

/// Crimped tin skirt under a ceramic ball, rolls ponderously.
pub(crate) fn draw_pieweight(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r / 6.5;
    let (cx, cy) = centre(fr);
    shadow(pb, cx, cy + 3, (6.0 * s).round() as i32);

    // The skirt: five rows flaring from 9 to 13 wide, crimped in alternating
    // light and dark columns that roll one step between the two walk frames.
    let roll = i32::from(step(fr, 0.4));
    let top = cy + 1;
    for k in 0..5i32 {
        let half = ((4.0 + k as f32 * 0.5) * s).round() as i32;
        let y = top + k;
        for x in (cx - half)..=(cx + half) {
            let crimp = (x - cx + roll).rem_euclid(2) == 0;
            let col = if x == cx - half {
                PIE_TIN_HI
            } else if x == cx + half {
                PIE_TIN_LO
            } else if crimp {
                PIE_TIN_HI
            } else {
                PIE_TIN_MID
            };
            pb.plot(x, y, tone(fr, if k == 4 && !crimp { PIE_TIN_LO } else { col }));
        }
    }
    if elite {
        // The doubled pie rim: two gold bands, top and hem, visible from
        // either side.
        let half_top = (4.0 * s).round() as i32;
        let half_hem = (6.0 * s).round() as i32;
        for x in (cx - half_top)..=(cx + half_top) {
            pb.plot(x, top, tone(fr, PIE_RIM));
            pb.plot(x, top + 1, tone(fr, PIE_RIM_LO));
        }
        for x in (cx - half_hem)..=(cx + half_hem) {
            pb.plot(x, top + 4, tone(fr, PIE_RIM));
            pb.plot(x, top + 5, tone(fr, PIE_RIM_LO));
        }
    }

    // The ceramic ball rides on top and sways against the roll.
    let sway = if roll == 1 { 0.6 } else { -0.6 };
    ball(
        pb,
        cx as f32 + sway,
        cy as f32 - 2.5 * s,
        4.2 * s,
        pal(fr, PIE_BALL_HI, PIE_BALL_MID, PIE_BALL_LO),
    );
    if elite {
        pie_shoulder_stripes(pb, fr, cx as f32 + sway, cy as f32 - 2.5 * s, 4.2 * s);
    }
    // One glaze glint.
    pb.plot(cx - 2, cy - (4.0 * s).round() as i32, tone(fr, (255, 255, 255)));
}

/// The elite's thick double shoulder stripe: two gold bands, each two pixels
/// deep with a dark lower edge, wrapped across the ball at `(bx, by)`.
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
        let half = (span.sqrt() - 0.6).floor() as i32;
        for x in (ix - half)..=(ix + half) {
            pb.plot(x, iy + dy, tone(fr, col));
        }
    }
}

/// Far LOD: a pale ball on a striped tin wedge.
pub(crate) fn draw_pieweight_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let s = fr.r / 6.5;
    let (cx, cy) = centre(fr);
    let roll = i32::from(step(fr, 0.4));
    dim(pb, cx + 4, cy + 4, 0.72);
    for k in 0..3i32 {
        let half = 3 + k / 2;
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
    ball(
        pb,
        cx as f32,
        cy as f32 - 1.5 * s,
        3.0 * s,
        pal(fr, PIE_BALL_HI, PIE_BALL_MID, PIE_BALL_LO),
    );
    if elite {
        // The shoulder stripe, one band at this size.
        let y = (cy as f32 - 1.5 * s).round() as i32;
        for x in (cx - 2)..=(cx + 2) {
            pb.plot(x, y, tone(fr, PIE_RIM));
        }
    }
}

// ---- Sink Crab (V Pillbug, r 5.0) -------------------------------------------

const CRAB_HI: Rgb = (244, 150, 112);
const CRAB_MID: Rgb = (184, 86, 64);
const CRAB_LO: Rgb = (88, 30, 26);
const LID_HI: Rgb = (246, 248, 252);
const LID_MID: Rgb = (188, 194, 204);
const LID_LO: Rgb = (92, 98, 112);

/// Broad flat shell, one pot-lid claw, scuttles sideways. Curled, the lid
/// swings up into a bar across `facing`, which is exactly the arc that blocks.
pub(crate) fn draw_sinkcrab(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let shell = pal(fr, CRAB_HI, CRAB_MID, CRAB_LO);
    let lid = pal(fr, LID_HI, LID_MID, LID_LO);
    let curled = e.w3.curl_t > 0.0;
    let (cx, cy) = centre(fr);
    shadow(pb, cx, cy, (4.0 * s).round() as i32);

    if !curled {
        // Legs: three a side, reaching out sideways, in two scuttle frames.
        let swap = step(fr, 0.8);
        for (i, u) in [1.4f32, 0.0, -1.4].into_iter().enumerate() {
            let out = if (i % 2 == 0) == swap { 6.5 } else { 5.5 };
            for side in [-1.0f32, 1.0] {
                let (x, y) = at(fr, u * s, side * out * s);
                pb.plot(x, y, shell.lo);
                let (x, y) = at(fr, u * s * 0.8, side * (out - 1.0) * s);
                pb.plot(x, y, shell.lo);
            }
        }
    }

    // The shell: twice as broad as it is long.
    blob(pb, fr.pos.x, fr.pos.y, fr.f, 2.6 * s, 4.8 * s, shell);
    // Eye stalks at the front edge.
    for side in [-1.0f32, 1.0] {
        let (x, y) = at(fr, 2.8 * s, side * 1.4 * s);
        pb.plot(x, y, tone(fr, (28, 18, 18)));
    }

    if curled {
        // The shield: the lid held flat across the facing, with its knob.
        for k in -4..=4i32 {
            let v = k as f32 * s;
            let (x, y) = at(fr, 5.2 * s, v);
            pb.plot(x, y, lid.hi);
            let (x, y) = at(fr, 4.2 * s, v);
            pb.plot(x, y, if k.abs() == 4 { lid.lo } else { lid.mid });
        }
        let (x, y) = at(fr, 6.2 * s, 0.0);
        pb.plot(x, y, lid.lo);
    } else {
        // The lid carried as a round claw at the front left, knob in the
        // middle.
        let (lx, ly) = (
            fr.pos.x + fr.f.x * 3.2 * s - fr.p.x * 3.6 * s,
            fr.pos.y + fr.f.y * 3.2 * s - fr.p.y * 3.6 * s,
        );
        ball(pb, lx, ly, 2.0 * s, lid);
        pb.plot(lx.round() as i32, ly.round() as i32, lid.lo);
        // The small bare claw on the other side.
        let (x, y) = at(fr, 3.0 * s, 3.4 * s);
        pb.plot(x, y, shell.hi);
    }
}

/// Far LOD: a broad red bar with a steel lid pixel pair (or the shield bar).
pub(crate) fn draw_sinkcrab_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let s = fr.r / 5.0;
    let (cx, cy) = centre(fr);
    dim(pb, cx + 2, cy + 2, 0.72);
    blob(
        pb,
        fr.pos.x,
        fr.pos.y,
        fr.f,
        1.6 * s,
        3.6 * s,
        pal(fr, CRAB_HI, CRAB_MID, CRAB_LO),
    );
    if e.w3.curl_t > 0.0 {
        for k in -3..=3i32 {
            let (x, y) = at(fr, 3.6 * s, k as f32 * s);
            pb.plot(x, y, tone(fr, LID_HI));
        }
    } else {
        let (x, y) = at(fr, 2.6 * s, -3.0 * s);
        pb.plot(x, y, tone(fr, LID_HI));
        pb.plot(x + 1, y, tone(fr, LID_MID));
        pb.plot(x, y + 1, tone(fr, LID_LO));
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
        assert_ne!(painted(&e, false), rest);
    }
}
