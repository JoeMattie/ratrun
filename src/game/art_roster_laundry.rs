//! The Laundry's own bodies (`WAVE4_ROSTERS.md`, "Laundry"): ivory yarn and
//! copper fittings against indigo grout.
//!
//! Five registrations get a body here, each a full-detail draw and a far-LOD
//! draw with `art_enemy`'s `BodyDraw` signature
//! (`fn(&mut PixelBuffer, &Frame, &Enemy, Vec2, bool)`), registered by one row
//! each in `art_enemy::bespoke_body` / `bespoke_lod`:
//!
//! | kind | silhouette | motion | tell pose |
//! |---|---|---|---|
//! | Wool Pill | pale round tuft on a trailing stitch | gathers and stretches | bunched, strand taut (its 0.15 s stall) |
//! | Clothes Moth | broad triangular shawl over a blunt head | shawl opens and shuts | none |
//! | Peg Snapper | forked wooden jaw on a bright hinge | walks open | prongs flung wide, then shut; the floor jaws |
//! | Starch Bulb | rubber pear on a tripod, crooked nozzle | nozzle rocks | squeezed bulb, lit nozzle tip |
//! | Lint Golem | broad uneven shoulders, dark dryer-button eye | shambles | none (elite: two brass coat buttons) |
//!
//! Everything is drawn in the body's local frame (`fr.f` forward, `fr.p`
//! sideways) and scaled by `fr.r` over the kind's registered radius, so a
//! body that spawns larger keeps its proportions. The light comes from the
//! upper left of the screen: every mass has a light edge there and a dark
//! edge opposite. Render-only: nothing here reads an rng or writes the world.
//!
//! The Peg Snapper's floor jaws are drawn from
//! [`floor_laundry::jaw_view`](crate::game::ecs::systems::floor_laundry::jaw_view),
//! the same geometry its hit test uses.

use crate::game::art_enemy::Frame;
use crate::game::ecs::systems::floor_laundry as rules;
use crate::game::enemy::Enemy;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

// ---- Palette ----------------------------------------------------------------

const WOOL_DARK: Rgb = (120, 112, 104);
const WOOL_MID: Rgb = (226, 218, 198);
const WOOL_HI: Rgb = (255, 250, 236);
const WOOL_THREAD: Rgb = (188, 176, 150);

const MOTH_DARK: Rgb = (84, 70, 52);
const MOTH_MID: Rgb = (170, 150, 120);
const MOTH_HI: Rgb = (222, 206, 176);
const MOTH_HEAD: Rgb = (62, 52, 44);

const PEG_DARK: Rgb = (96, 68, 38);
const PEG_MID: Rgb = (190, 146, 92);
const PEG_HI: Rgb = (236, 204, 150);
const PEG_HINGE: Rgb = (255, 214, 96);
const PEG_HINGE_HOT: Rgb = (255, 248, 210);

const BULB_DARK: Rgb = (84, 96, 116);
const BULB_MID: Rgb = (198, 206, 214);
const BULB_HI: Rgb = (250, 252, 255);
const BULB_LEG: Rgb = (58, 60, 70);
const BULB_NOZZLE: Rgb = (150, 108, 72);
const BULB_CHARGE: Rgb = (255, 255, 255);

const LINT_DARK: Rgb = (66, 62, 64);
const LINT_MID: Rgb = (140, 136, 130);
const LINT_HI: Rgb = (192, 188, 180);
const LINT_FLECK: Rgb = (214, 210, 204);
const LINT_EYE: Rgb = (18, 16, 22);
const LINT_EYE_RIM: Rgb = (96, 92, 100);
const BRASS: Rgb = (240, 198, 96);

const JAW_WARN: Rgb = (255, 196, 64);
const JAW_WARN_HOT: Rgb = (255, 236, 150);
const JAW_LIVE: Rgb = (255, 88, 60);
const JAW_LIVE_CORE: Rgb = (255, 240, 220);
const JAW_SHUT: Rgb = (150, 116, 78);

/// Seconds before the shot at which the Starch Bulb shows its charge.
const BULB_CHARGE_SECS: f32 = 0.4;

// ---- Small local helpers ----------------------------------------------------

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

/// The warm-white hit flash, the same read as the older bodies.
#[inline]
fn hit(c: Rgb, fr: &Frame) -> Rgb {
    if fr.hit <= 0.0 {
        c
    } else {
        mix(c, (255, 240, 214), (0.30 + fr.hit * 0.56).min(0.86))
    }
}

#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

/// Screen pixel of local `(u, v)`: `u` along the facing, `v` across it.
#[inline]
fn at(fr: &Frame, u: f32, v: f32) -> (i32, i32) {
    (
        (fr.pos.x + fr.f.x * u + fr.p.x * v).round() as i32,
        (fr.pos.y + fr.f.y * u + fr.p.y * v).round() as i32,
    )
}

/// A filled disc with a light edge to the upper left and a dark edge to the
/// lower right, over a soft ground shadow.
fn disc(pb: &mut PixelBuffer, c: (i32, i32), rad: i32, dark: Rgb, mid: Rgb, hi: Rgb) {
    let rad = rad.max(1);
    let r2 = rad * rad;
    let inner = (rad - 1) * (rad - 1);
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let col = if d2 == 0 || (d2 <= inner && rad > 1) {
                mid
            } else if dx + dy > 0 {
                dark
            } else if dx + dy < 0 {
                hi
            } else {
                mid
            };
            pb.plot(c.0 + dx, c.1 + dy, col);
        }
    }
}

/// A one-pixel ground shadow blob under a body.
fn shadow(pb: &mut PixelBuffer, c: (i32, i32), rad: i32) {
    for dx in -rad..=rad {
        blend(pb, c.0 + dx + 1, c.1 + rad + 1, (0, 0, 0), 0.30);
    }
}

#[inline]
fn seg(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb) {
    pb.line(a.0, a.1, b.0, b.1, c);
}

/// Two walk frames from the gait phase (`rate` = how fast the frames swap).
#[inline]
fn walk_frame(fr: &Frame, rate: f32) -> bool {
    ((fr.anim * rate) as i32) & 1 == 1
}

#[inline]
fn px(v: f32) -> i32 {
    v.round().max(1.0) as i32
}

// ---- Wool Pill --------------------------------------------------------------

/// Pale round tuft on a trailing stitch. Frame A is the gathered ball, frame B
/// the stretch (two lobes along the heading); the stall bunches it tight and
/// pulls the strand taut and bright.
pub(crate) fn draw_wool_pill(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let k = fr.r / 2.5;
    let gather = rules::wool_pill_gather(e);
    let stalled = rules::wool_pill_paused(e);
    let stretch = !stalled && walk_frame(fr, 0.25);
    let (dark, mid, hi) = (hit(WOOL_DARK, fr), hit(WOOL_MID, fr), hit(WOOL_HI, fr));

    // The trailing stitch: a dashed strand behind the tuft, kinked while it
    // walks, drawn straight and bright while the pill pulls it free.
    let tail: f32 = (if stalled { 7.0 } else { 5.5 }) * k;
    let mut i = 0;
    let mut u = 3.0 * k;
    while u <= 3.0 * k + tail {
        let kink = if stalled {
            0.0
        } else if i % 2 == 0 {
            0.8 * k
        } else {
            -0.8 * k
        };
        let p = at(fr, -u, kink);
        if stalled {
            pb.plot(p.0, p.1, mix(WOOL_THREAD, WOOL_HI, 0.5 + gather * 0.5));
        } else if i % 3 != 2 {
            pb.plot(p.0, p.1, WOOL_THREAD);
        }
        u += 1.0;
        i += 1;
    }
    // The knot at the end of the strand.
    let knot = at(fr, -(3.0 * k + tail), 0.0);
    pb.plot(knot.0, knot.1, dark);

    let c = at(fr, 0.0, 0.0);
    shadow(pb, c, px(3.0 * k));
    if stretch {
        // Stretched: a rear lobe and a front lobe.
        disc(pb, at(fr, -1.6 * k, 0.0), px(2.0 * k), dark, mid, hi);
        disc(pb, at(fr, 1.4 * k, 0.0), px(3.0 * k), dark, mid, hi);
    } else {
        disc(pb, c, px(3.0 * k) + if stalled { 1 } else { 0 }, dark, mid, hi);
    }
    // Two fibre ticks so the ball reads as wool and not a pebble.
    let t0 = at(fr, 0.6 * k, -1.2 * k);
    let t1 = at(fr, -0.8 * k, 1.0 * k);
    pb.plot(t0.0, t0.1, hi);
    pb.plot(t1.0, t1.1, mix(mid, dark, 0.45));
    if elite {
        let b = at(fr, 0.0, 0.0);
        pb.plot(b.0, b.1, BRASS);
    }
}

pub(crate) fn draw_wool_pill_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let c = at(fr, 0.0, 0.0);
    let mid = hit(WOOL_MID, fr);
    blend(pb, c.0 + 1, c.1 + 2, (0, 0, 0), 0.3);
    // A plus-shaped tuft with a lit corner and a two-pixel strand.
    pb.plot(c.0, c.1, mid);
    pb.plot(c.0 + 1, c.1, hit(WOOL_DARK, fr));
    pb.plot(c.0, c.1 + 1, hit(WOOL_DARK, fr));
    pb.plot(c.0 - 1, c.1, hit(WOOL_HI, fr));
    pb.plot(c.0, c.1 - 1, hit(WOOL_HI, fr));
    let taut = rules::wool_pill_paused(e);
    for (n, d) in [2.6f32, 3.8].into_iter().enumerate() {
        let p = at(fr, -d, if taut || n == 0 { 0.0 } else { 1.0 });
        pb.plot(p.0, p.1, if taut { WOOL_HI } else { WOOL_THREAD });
    }
}

// ---- Clothes Moth -----------------------------------------------------------

/// How wide the shawl is right now, 0 (shut) to 1 (open).
#[inline]
fn shawl_open(fr: &Frame) -> f32 {
    0.5 + 0.5 * (fr.anim * 0.55).sin()
}

/// Broad triangular shawl that opens and shuts over a blunt head: the apex is
/// at the head, the hem trails wide behind it.
pub(crate) fn draw_clothes_moth(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let k = fr.r / 2.0;
    let open = shawl_open(fr);
    let half = (1.6 + 4.2 * open) * k;
    let len = 6.5 * k;
    let (dark, mid, hi) = (hit(MOTH_DARK, fr), hit(MOTH_MID, fr), hit(MOTH_HI, fr));

    // The shawl: scan the triangle in local space (half-pixel steps so no
    // pixel is skipped at any heading).
    let steps = (len * 2.0).ceil() as i32;
    for iu in 0..=steps {
        let back = iu as f32 * 0.5;
        let w = half * (back / len);
        let rows = (w * 2.0).ceil() as i32;
        for iv in -rows..=rows {
            let v = iv as f32 * 0.5;
            let p = at(fr, 1.5 * k - back, v);
            let edge = w - v.abs() < 0.8;
            let hem = back > len - 1.0;
            let col = if hem {
                dark
            } else if edge {
                // Leading edges: lit on the screen's upper-left side.
                if (p.0 as f32 - fr.pos.x) + (p.1 as f32 - fr.pos.y) < 0.0 { hi } else { dark }
            } else if (iu + iv).rem_euclid(5) == 0 {
                // A dusting of moth-eaten holes.
                mix(mid, dark, 0.35)
            } else {
                mid
            };
            pb.plot(p.0, p.1, col);
        }
    }
    // The fold down the middle shows when the shawl is nearly shut.
    if open < 0.4 {
        seg(pb, at(fr, 0.5 * k, 0.0), at(fr, 1.5 * k - len, 0.0), dark);
    }
    // The blunt head, forward of the apex, with two pale feelers.
    let head = at(fr, 2.4 * k, 0.0);
    disc(pb, head, px(1.0 * k), MOTH_HEAD, hit(MOTH_HEAD, fr), mix(MOTH_HEAD, hi, 0.5));
    for side in [-1.0f32, 1.0] {
        let tip = at(fr, 4.2 * k, side * 1.4 * k);
        pb.plot(tip.0, tip.1, hi);
    }
    if elite {
        let b = at(fr, 0.0, 0.0);
        pb.plot(b.0, b.1, BRASS);
    }
}

pub(crate) fn draw_clothes_moth_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, _elite: bool) {
    let open = shawl_open(fr) > 0.5;
    let (dark, mid) = (hit(MOTH_DARK, fr), hit(MOTH_MID, fr));
    let head = at(fr, 1.5, 0.0);
    pb.plot(head.0, head.1, MOTH_HEAD);
    let near = at(fr, 0.0, 0.0);
    pb.plot(near.0, near.1, mid);
    // The hem row: three wide when open, one when shut.
    let w: i32 = if open { 2 } else { 1 };
    for iv in -w..=w {
        let p = at(fr, -1.5, iv as f32);
        pb.plot(p.0, p.1, if iv.abs() == w { dark } else { mid });
    }
    if open {
        for side in [-1.0f32, 1.0] {
            let p = at(fr, -2.6, side * 3.0);
            pb.plot(p.0, p.1, dark);
        }
    }
}

// ---- Peg Snapper ------------------------------------------------------------

/// The floor jaws: two `16 × 3` bars drawn from the same view the hit test
/// reads. Hollow amber while they warn, solid red-white while they slide,
/// dull wood while they rest shut.
fn draw_floor_jaws(pb: &mut PixelBuffer, view: &rules::JawView, cam: Vec2) {
    let side = view.axis.perp();
    let half_len = rules::PEG_JAW_LEN * 0.5;
    let half_thick = rules::PEG_JAW_THICK * 0.5;
    let pulse = ((view.t * 10.0) as i32) & 1 == 0;
    for c in view.jaws() {
        let nu = (half_len * 2.0) as i32;
        let nv = (half_thick * 2.0).ceil() as i32;
        for iu in -nu..=nu {
            for iv in -nv..=nv {
                let (u, v) = (iu as f32 * 0.5, (iv as f32 * 0.5).clamp(-half_thick, half_thick));
                let w = c + view.axis * u + side * v - cam;
                let (x, y) = (w.x.round() as i32, w.y.round() as i32);
                let rim = u.abs() > half_len - 0.8 || v.abs() > half_thick - 0.6;
                match view.phase {
                    rules::PEG_PHASE_TELL => {
                        if rim {
                            pb.plot(x, y, if pulse { JAW_WARN_HOT } else { JAW_WARN });
                        } else {
                            blend(pb, x, y, JAW_WARN, 0.28);
                        }
                    }
                    rules::PEG_PHASE_CLOSE => {
                        pb.plot(x, y, if rim { JAW_LIVE } else { JAW_LIVE_CORE });
                    }
                    _ => {
                        if rim {
                            blend(pb, x, y, PEG_DARK, 0.85);
                        } else {
                            blend(pb, x, y, JAW_SHUT, 0.8);
                        }
                    }
                }
            }
        }
    }
    // The warned travel: a faint dotted line between the open jaws shows
    // which way they will slide.
    if view.phase == rules::PEG_PHASE_TELL {
        let mut d = -view.offset + 2.5;
        while d <= view.offset - 2.5 {
            let w = view.centre + side * d - cam;
            blend(pb, w.x.round() as i32, w.y.round() as i32, JAW_WARN, 0.45);
            d += 2.0;
        }
    }
}

/// The hostile-layer stamp (`art_enemy::draw_w3_tells`): the floor jaws alone,
/// re-drawn after the lighting pass so the warning is self-lit above every
/// friendly effect. Same view, same pixels as the body pass.
pub(crate) fn draw_peg_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
}

/// How far each prong tip sits off the body's centre line for the pose.
fn prong_spread(fr: &Frame, e: &Enemy) -> (f32, bool) {
    match rules::jaw_view(e) {
        Some(v) if v.phase == rules::PEG_PHASE_TELL => (5.0, true),
        Some(v) if v.phase == rules::PEG_PHASE_CLOSE => {
            let t = (v.t / rules::PEG_CLOSE_SECS).clamp(0.0, 1.0);
            (5.0 - 4.2 * t, true)
        }
        Some(_) => (0.8, false),
        // Walking open: the fork breathes between two frames.
        None => (if walk_frame(fr, 0.3) { 3.2 } else { 2.2 }, false),
    }
}

/// Forked wooden jaw on a bright hinge. It walks open; the tell flings the
/// prongs wide with the hinge white-hot, the strike claps them shut.
pub(crate) fn draw_peg_snapper(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
    let k = fr.r / 2.0;
    let (spread, hot) = prong_spread(fr, e);
    let (dark, mid, hi) = (hit(PEG_DARK, fr), hit(PEG_MID, fr), hit(PEG_HI, fr));
    let hinge = at(fr, -2.5 * k, 0.0);
    shadow(pb, at(fr, 1.0 * k, 0.0), px(3.0 * k));
    for side in [-1.0f32, 1.0] {
        let root = at(fr, -2.0 * k, side * 0.8 * k);
        let tip = at(fr, 5.5 * k, side * spread * k);
        // Each prong is two pixels thick: the wood, and an inner dark edge
        // (toward the bite) so the fork reads as a gap, not a wedge.
        let root_in = at(fr, -2.0 * k, side * 0.1 * k);
        let tip_in = at(fr, 5.5 * k, side * (spread - 1.0).max(0.0) * k);
        seg(pb, root_in, tip_in, dark);
        seg(pb, root, tip, mid);
        // Lit outer edge on whichever prong faces the screen's upper left.
        let outer = at(fr, 2.0 * k, side * (spread * 0.55 + 1.0) * k);
        if (outer.0 as f32 - fr.pos.x) + (outer.1 as f32 - fr.pos.y) < 0.0 {
            let o0 = at(fr, -k, side * 1.4 * k);
            let o1 = at(fr, 4.5 * k, side * (spread + 0.6) * k);
            seg(pb, o0, o1, hi);
        }
        // The blunt gripping tooth at the tip.
        pb.plot(tip.0, tip.1, hi);
    }
    // Two stub feet behind the hinge, alternating.
    let step: f32 = if walk_frame(fr, 0.3) { 0.8 } else { -0.8 };
    for side in [-1.0f32, 1.0] {
        let foot = at(fr, (-3.4 + step * side) * k, side * 1.8 * k);
        pb.plot(foot.0, foot.1, dark);
    }
    // The bright hinge: a brass coil, white-hot through the tell and strike.
    let coil = if hot { PEG_HINGE_HOT } else { PEG_HINGE };
    disc(pb, hinge, px(1.0 * k), mix(PEG_HINGE, PEG_DARK, 0.5), hit(coil, fr), PEG_HINGE_HOT);
    if elite {
        let b = at(fr, 1.0 * k, 0.0);
        pb.plot(b.0, b.1, BRASS);
    }
}

pub(crate) fn draw_peg_snapper_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    // The floor jaws are gameplay, so the far body keeps them whole.
    if let Some(view) = rules::jaw_view(e) {
        draw_floor_jaws(pb, &view, cam);
    }
    let (spread, hot) = prong_spread(fr, e);
    let mid = hit(PEG_MID, fr);
    let hinge = at(fr, -1.5, 0.0);
    for side in [-1.0f32, 1.0] {
        let a = at(fr, 0.2, side * (spread * 0.3).max(0.6));
        let b = at(fr, 2.4, side * (spread * 0.6).max(0.6));
        pb.plot(a.0, a.1, mid);
        pb.plot(b.0, b.1, hit(PEG_HI, fr));
    }
    pb.plot(hinge.0, hinge.1, if hot { PEG_HINGE_HOT } else { PEG_HINGE });
}

// ---- Starch Bulb ------------------------------------------------------------

/// 0..1 as the shot comes due (`fire_cd` runs down to 0 and holds there while
/// cover blocks the line).
#[inline]
fn bulb_charge(e: &Enemy) -> f32 {
    (1.0 - e.fire_cd / BULB_CHARGE_SECS).clamp(0.0, 1.0)
}

/// Rubber pear on a tripod with a thin crooked nozzle that rocks. The charge
/// squeezes the bulb and lights the nozzle tip.
pub(crate) fn draw_starch_bulb(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, elite: bool) {
    let k = fr.r / 3.0;
    let charge = bulb_charge(e);
    let (dark, mid, hi) = (hit(BULB_DARK, fr), hit(BULB_MID, fr), hit(BULB_HI, fr));

    // The tripod: three splayed legs, the feet stepping in two frames.
    let step: f32 = if walk_frame(fr, 0.3) { 0.9 } else { -0.9 };
    let legs: [(f32, f32, f32); 3] = [(-4.2, 0.0, step), (2.2, -4.4, -step), (2.2, 4.4, step)];
    for (u, v, s) in legs {
        let hip = at(fr, u * 0.3 * k, v * 0.3 * k);
        let foot = at(fr, (u + s) * k, v * k);
        seg(pb, hip, foot, BULB_LEG);
        pb.plot(foot.0, foot.1, mix(BULB_LEG, hi, 0.35));
    }

    // The pear: a fat belly behind and a narrower shoulder toward the nozzle.
    // Charging squeezes the belly a pixel thinner.
    let belly = at(fr, -1.2 * k, 0.0);
    let neck = at(fr, 2.2 * k, 0.0);
    shadow(pb, belly, px(3.0 * k));
    let squeeze = if charge > 0.5 { 1 } else { 0 };
    disc(pb, belly, (px(3.4 * k) - squeeze).max(1), dark, mid, hi);
    disc(pb, neck, px(2.0 * k), dark, mid, hi);
    // A rubber seam across the belly.
    let s0 = at(fr, -1.2 * k, -2.0 * k);
    let s1 = at(fr, -1.2 * k, 2.0 * k);
    seg(pb, s0, s1, mix(mid, dark, 0.4));

    // The thin crooked nozzle: two segments with a kink, rocking side to side.
    let rock = (fr.anim * 0.35).sin() * 1.2 * k * (1.0 - charge);
    let n0 = at(fr, 3.6 * k, 0.0);
    let n1 = at(fr, 5.6 * k, 1.2 * k + rock);
    let n2 = at(fr, 8.0 * k, rock * 0.4);
    seg(pb, n0, n1, BULB_NOZZLE);
    seg(pb, n1, n2, BULB_NOZZLE);
    if charge > 0.0 {
        // The charged muzzle: the tip whitens, then a bead swells on it.
        pb.plot(n2.0, n2.1, mix(BULB_NOZZLE, BULB_CHARGE, charge));
        if charge > 0.6 {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                blend(pb, n2.0 + dx, n2.1 + dy, rules::STARCH_JET_COLOR, 0.7);
            }
        }
    }
    if elite {
        pb.plot(belly.0, belly.1, BRASS);
    }
}

pub(crate) fn draw_starch_bulb_lod(pb: &mut PixelBuffer, fr: &Frame, e: &Enemy, _cam: Vec2, _elite: bool) {
    let (dark, mid) = (hit(BULB_DARK, fr), hit(BULB_MID, fr));
    let c = at(fr, -0.5, 0.0);
    blend(pb, c.0 + 1, c.1 + 2, (0, 0, 0), 0.3);
    // A 2×2 bulb, one nozzle pixel ahead, three leg pixels around.
    pb.plot(c.0, c.1, mid);
    pb.plot(c.0 - 1, c.1, hit(BULB_HI, fr));
    pb.plot(c.0, c.1 + 1, dark);
    pb.plot(c.0 - 1, c.1 + 1, mid);
    for (u, v) in [(-3.0f32, 0.0f32), (1.2, -2.6), (1.2, 2.6)] {
        let p = at(fr, u, v);
        pb.plot(p.0, p.1, BULB_LEG);
    }
    let tip = at(fr, 3.0, 0.0);
    pb.plot(tip.0, tip.1, mix(BULB_NOZZLE, BULB_CHARGE, bulb_charge(e)));
}

// ---- Lint Golem -------------------------------------------------------------

/// Broad uneven shoulders and a dark dryer-button eye; it shambles, one
/// shoulder leading and then the other. The elite wears two brass coat
/// buttons and a thick double stripe across each shoulder.
pub(crate) fn draw_lint_golem(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let k = fr.r / 6.5;
    let (dark, mid, hi) = (hit(LINT_DARK, fr), hit(LINT_MID, fr), hit(LINT_HI, fr));
    let lead: f32 = if walk_frame(fr, 0.2) { 1.2 } else { -1.2 };

    let torso = at(fr, -0.5 * k, 0.0);
    shadow(pb, torso, px(6.0 * k));
    // Shoulders first (uneven: the left is the big one), then the torso over
    // the join, then the low head lump.
    let big = at(fr, (0.4 + lead) * k, -5.2 * k);
    let small = at(fr, (0.2 - lead) * k, 4.6 * k);
    disc(pb, big, px(4.2 * k), dark, mid, hi);
    disc(pb, small, px(3.0 * k), dark, mid, hi);
    disc(pb, torso, px(5.0 * k), dark, mid, hi);
    let head = at(fr, 3.6 * k, 0.4 * k);
    disc(pb, head, px(2.6 * k), dark, mid, hi);

    // Lint flecks: fixed spots in the local frame, so they ride the body.
    for (u, v) in [(-2.5f32, -1.5f32), (-1.0, 2.6), (1.2, -3.8), (-3.2, 1.0), (0.6, 5.0), (0.2, -6.4)] {
        let p = at(fr, u * k, v * k);
        pb.plot(p.0, p.1, hit(LINT_FLECK, fr));
    }

    // The dryer-button eye: a dark disc with a rim and one thread hole lit.
    let eye = at(fr, 3.8 * k, 0.4 * k);
    disc(pb, eye, px(1.2 * k), LINT_EYE, LINT_EYE, LINT_EYE_RIM);
    pb.plot(eye.0, eye.1, LINT_EYE);

    if elite {
        // Two brass coat buttons down the chest.
        for u in [1.0f32, -2.2] {
            let b = at(fr, u * k, 0.0);
            pb.plot(b.0, b.1, BRASS);
            pb.plot(b.0 + 1, b.1, mix(BRASS, LINT_DARK, 0.4));
        }
        // The thick double shoulder stripe: two dark bands along the heading
        // on each shoulder, one pixel of lint showing between them.
        for (u0, v0, half) in [(0.4 + lead, -5.2f32, 3.0f32), (0.2 - lead, 4.6, 2.2)] {
            for dv in [-1.2f32, 1.2] {
                let a = at(fr, (u0 - half) * k, v0 * k + dv);
                let b = at(fr, (u0 + half) * k, v0 * k + dv);
                seg(pb, a, b, LINT_DARK);
            }
        }
    }
}

pub(crate) fn draw_lint_golem_lod(pb: &mut PixelBuffer, fr: &Frame, _e: &Enemy, _cam: Vec2, elite: bool) {
    let k = fr.r / 6.5;
    let (dark, mid, hi) = (hit(LINT_DARK, fr), hit(LINT_MID, fr), hit(LINT_HI, fr));
    let c = at(fr, 0.0, 0.0);
    blend(pb, c.0 + 1, c.1 + px(3.0 * k) + 1, (0, 0, 0), 0.3);
    // A wide bar across the heading (the shoulders) over a small core.
    disc(pb, c, px(3.0 * k), dark, mid, hi);
    for v in [-5.0f32, -4.0, 4.0] {
        let p = at(fr, 0.0, v * k);
        pb.plot(p.0, p.1, if v < 0.0 { hi } else { dark });
    }
    let eye = at(fr, 2.4 * k, 0.0);
    pb.plot(eye.0, eye.1, LINT_EYE);
    if elite {
        // Two brass buttons and a dark stripe pixel on each shoulder.
        for u in [0.6f32, -1.6] {
            let b = at(fr, u * k, 0.0);
            pb.plot(b.0, b.1, BRASS);
        }
        for v in [-3.0f32, 3.0] {
            let p = at(fr, 0.0, v * k);
            pb.plot(p.0, p.1, LINT_DARK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_enemy;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const KINDS: [EnemyKind; 5] = [
        EnemyKind::WoolPill,
        EnemyKind::ClothesMoth,
        EnemyKind::PegSnapper,
        EnemyKind::StarchBulb,
        EnemyKind::LintGolem,
    ];
    const BG: Rgb = (20, 22, 40);

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(kind, Vec2::new(40.0, 40.0), 1.0, &mut rng);
        e.facing = Vec2::new(1.0, 0.0);
        e.fire_cd = 2.0;
        e
    }

    /// Every pixel a draw changed, as a mask.
    fn mask(e: &Enemy, lod: bool) -> Vec<bool> {
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear(BG);
        if lod {
            art_enemy::draw_enemy_lod(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        } else {
            art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, (255, 255, 255));
        }
        pb.pixels().iter().map(|&p| p != BG).collect()
    }

    fn count(m: &[bool]) -> usize {
        m.iter().filter(|&&b| b).count()
    }

    #[test]
    fn every_laundry_kind_draws_a_body_and_a_smaller_far_body() {
        for kind in KINDS {
            let e = body(kind);
            let (full, far) = (mask(&e, false), mask(&e, true));
            assert!(count(&full) >= 12, "{kind:?} full body is {} px", count(&full));
            assert!(count(&far) >= 4, "{kind:?} far body is {} px", count(&far));
            assert!(count(&far) < count(&full), "{kind:?} far body is not smaller");
        }
    }

    #[test]
    fn the_five_silhouettes_are_distinct() {
        let masks: Vec<Vec<bool>> = KINDS.iter().map(|&k| mask(&body(k), false)).collect();
        for i in 0..masks.len() {
            for j in i + 1..masks.len() {
                let differ = masks[i].iter().zip(&masks[j]).filter(|(a, b)| a != b).count();
                assert!(differ >= 8, "{:?} and {:?} differ by {differ} px", KINDS[i], KINDS[j]);
            }
        }
    }

    #[test]
    fn each_body_has_two_walk_frames() {
        for kind in KINDS {
            let a = body(kind);
            let mut seen = false;
            for step in 1..40 {
                let mut b = body(kind);
                b.anim = a.anim + step as f32 * 0.5;
                if mask(&a, false) != mask(&b, false) {
                    seen = true;
                    break;
                }
            }
            assert!(seen, "{kind:?} never changes pose as its gait advances");
        }
    }

    #[test]
    fn each_body_has_a_light_and_a_dark_edge() {
        for kind in KINDS {
            let e = body(kind);
            let mut pb = PixelBuffer::new(80, 80);
            pb.clear(BG);
            art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (255, 255, 255));
            let lum = |p: Rgb| p.0 as u32 + p.1 as u32 + p.2 as u32;
            let painted: Vec<u32> = pb.pixels().iter().filter(|&&p| p != BG).map(|&p| lum(p)).collect();
            let (lo, hi) = (painted.iter().min().unwrap(), painted.iter().max().unwrap());
            assert!(hi - lo >= 150, "{kind:?} has no light/dark contrast ({lo}..{hi})");
        }
    }

    #[test]
    fn the_elite_golem_wears_brass_buttons_and_a_shoulder_stripe() {
        let plain = body(EnemyKind::LintGolem);
        let mut elite = body(EnemyKind::LintGolem);
        elite.elite = true;
        for lod in [false, true] {
            let draw = |e: &Enemy| {
                let mut pb = PixelBuffer::new(80, 80);
                pb.clear(BG);
                if lod {
                    art_enemy::draw_enemy_lod(&mut pb, e, Vec2::ZERO, (255, 255, 255));
                } else {
                    art_enemy::draw_enemy(&mut pb, e, Vec2::ZERO, (255, 255, 255));
                }
                pb.pixels().to_vec()
            };
            let (a, b) = (draw(&plain), draw(&elite));
            assert_ne!(a, b, "elite reads the same (lod {lod})");
            assert!(b.contains(&BRASS), "no brass button (lod {lod})");
            assert!(!a.contains(&BRASS), "a plain golem has no brass (lod {lod})");
        }
    }

    #[test]
    fn the_wool_pill_bunches_when_it_stalls() {
        let walking = body(EnemyKind::WoolPill);
        let mut stalled = body(EnemyKind::WoolPill);
        stalled.w3.flea_t = rules::WOOL_PILL_PERIOD - rules::WOOL_PILL_PAUSE * 0.5;
        assert!(rules::wool_pill_paused(&stalled));
        assert_ne!(mask(&walking, false), mask(&stalled, false));
    }

    #[test]
    fn the_starch_bulb_shows_its_charge() {
        let idle = body(EnemyKind::StarchBulb);
        let mut charged = body(EnemyKind::StarchBulb);
        charged.fire_cd = 0.02;
        assert_ne!(mask(&idle, false), mask(&charged, false));
    }

    #[test]
    fn the_peg_snapper_draws_its_floor_jaws_from_the_hit_geometry() {
        let mut e = body(EnemyKind::PegSnapper);
        let walking = mask(&e, false);
        e.w3.bar_pos = e.pos + Vec2::new(15.0, 0.0);
        e.w3.bar_dir = Vec2::new(1.0, 0.0);
        e.w3.home = Vec2::new(rules::PEG_PHASE_TELL as f32, 0.2);
        let view = rules::jaw_view(&e).expect("jaws set");
        let tell = mask(&e, false);
        assert!(count(&tell) > count(&walking) + 40, "the tell paints two bars");
        // Every pixel under either jaw's centre line is painted.
        let pb_w = 80usize;
        for c in view.jaws() {
            for du in [-7.0f32, 0.0, 7.0] {
                let p = c + view.axis * du;
                let idx = p.y.round() as usize * pb_w + p.x.round() as usize;
                assert!(tell[idx], "jaw pixel at {p:?} is not drawn");
            }
        }
        // The strike pose is its own frame, and so is the far body's tell.
        e.w3.home = Vec2::new(rules::PEG_PHASE_CLOSE as f32, 0.15);
        assert_ne!(mask(&e, false), tell);
        assert!(count(&mask(&e, true)) > 40, "the far body keeps the jaws");
    }
}
