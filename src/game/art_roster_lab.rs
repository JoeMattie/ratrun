//! The Lab's own bodies (batch 6b): Swab Foot, Spinner Lid, Glass Runner,
//! Walking Syringe and Autoclave Cart.
//!
//! Palette (`WAVE4_ROSTERS.md`, Lab): white ceramic, black rubber and specimen
//! pink against console glow. Every body here has a 6-12 px core, one light
//! edge (toward the up-left light) and one dark edge, two walk frames, and a
//! far-LOD twin that keeps the silhouette in a handful of pixels.
//!
//! Each draw has the `BodyDraw` signature `art_enemy::bespoke_body` expects.
//! The shared `Frame` is accepted and ignored: these bodies build their own
//! small local frame from the `Enemy`, so this file depends on nothing private
//! in `art_enemy` beyond the type's name.
//!
//! | kind | silhouette | walk frames | tell pose |
//! |---|---|---|---|
//! | Swab Foot | cotton club on a bent black stem | the stem's kink swaps sides, the head paddles forward | none |
//! | Spinner Lid | slanted disk, bright rim | broad face ↔ thin edge (read from the weave clock) | edge-on at the flip |
//! | Glass Runner | clear ampoule on two long rubber feet | the feet vault fore/aft in opposition | none |
//! | Walking Syringe | needle on stilt legs | stilts alternate | planted wide, plunger drawn back, barrel fills pink |
//! | Autoclave Cart | box door on two big wheels | wheel spokes turn, the latch slides | none |
//!
//! The Walking Syringe's ray is drawn by [`draw_syringe_tell`] from the same
//! segment the hit test uses (`floor_lab::syringe_ray`).

use crate::game::ecs::systems::floor_lab;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

use super::art_enemy::Frame;
use super::enemy::Enemy;

/// Direction toward the light in screen space (matches `art_enemy::LIGHT`).
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };
/// Hit-flash peak (matches `art_enemy::FLASH_MAX`).
const FLASH_MAX: f32 = 0.12;

const RUBBER: Rgb = (22, 22, 28);
const RUBBER_HI: Rgb = (70, 72, 84);
const CERAMIC: Rgb = (240, 242, 246);
const CERAMIC_SHADE: Rgb = (150, 156, 170);
const SPECIMEN_PINK: Rgb = (236, 120, 160);
const STEEL_HI: Rgb = (226, 232, 240);
const STEEL_DARK: Rgb = (58, 62, 74);

/// A body's local screen frame.
#[derive(Clone, Copy)]
struct Local {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    hit: f32,
}

impl Local {
    fn of(e: &Enemy, cam: Vec2) -> Local {
        let len_sq = e.facing.x * e.facing.x + e.facing.y * e.facing.y;
        let f = if len_sq > 1e-4 {
            let inv = 1.0 / len_sq.sqrt();
            Vec2::new(e.facing.x * inv, e.facing.y * inv)
        } else {
            Vec2::new(1.0, 0.0)
        };
        Local {
            pos: e.pos - cam,
            f,
            p: Vec2::new(-f.y, f.x),
            hit: (e.flash / FLASH_MAX).clamp(0.0, 1.0),
        }
    }

    /// Body-space `(u forward, v sideways)` to a screen pixel.
    fn at(&self, u: f32, v: f32) -> (i32, i32) {
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// A body colour with the warm hit flash folded in.
    fn tint(&self, c: Rgb) -> Rgb {
        if self.hit <= 0.0 {
            c
        } else {
            mix(c, (255, 240, 210), (0.30 + self.hit * 0.56).min(0.86))
        }
    }
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
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

/// A soft contact shadow pixel.
#[inline]
fn shade(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    blend(pb, x, y, (0, 0, 0), amount);
}

/// Two walk frames from the body's animation phase.
#[inline]
fn step(e: &Enemy) -> bool {
    (e.anim * 0.5).sin() > 0.0
}

/// Light (true) or dark (false) side for a body-space offset.
#[inline]
fn lit(l: &Local, u: f32, v: f32) -> bool {
    let sx = l.f.x * u + l.p.x * v;
    let sy = l.f.y * u + l.p.y * v;
    sx * LIGHT.x + sy * LIGHT.y > 0.0
}

// ---- Swab Foot ---------------------------------------------------------------

/// Cotton club on a bent black stem: a round white head leading, a kinked
/// rubber stem trailing. The kink swaps sides each step and the head paddles
/// a pixel forward, so a column of them reads as a row of nodding swabs.
pub(crate) fn draw_swabfoot(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let a = step(e);
    let side = if a { 1.2 } else { -1.2 };
    let head_u = if a { 2.5 } else { 1.5 };
    let tail = l.at(-4.0, -side * 0.5);
    let knee = l.at(-1.5, side);
    let neck = l.at(head_u - 1.0, 0.0);
    // Shadow under the head.
    let (hx, hy) = l.at(head_u, 0.0);
    for dx in -1..=2 {
        shade(pb, hx + dx, hy + 3, 0.30);
    }
    // The bent stem (black rubber with one lit pixel at the knee).
    let stem = l.tint(RUBBER);
    pb.line(tail.0, tail.1, knee.0, knee.1, stem);
    pb.line(knee.0, knee.1, neck.0, neck.1, stem);
    pb.plot(knee.0, knee.1, l.tint(RUBBER_HI));
    // The cotton head: a radius-2 puff, light edge up-left, dark edge
    // down-right.
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let d2 = dx * dx + dy * dy;
            if d2 > 5 {
                continue;
            }
            let lum = -(dx as f32 * 0.55 + dy as f32 * 0.83);
            let c = if d2 >= 4 && lum < -0.5 {
                CERAMIC_SHADE
            } else if lum > 1.0 {
                (255, 255, 255)
            } else {
                e.kind.color()
            };
            pb.plot(hx + dx, hy + dy, l.tint(c));
        }
    }
    // A faint pink smear on the leading tip (it has been somewhere).
    let tip = l.at(head_u + 2.0, 0.0);
    blend(pb, tip.0, tip.1, SPECIMEN_PINK, 0.55);
}

pub(crate) fn draw_swabfoot_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let tail = l.at(-3.0, 0.0);
    let mid = l.at(-1.0, if step(e) { 1.0 } else { -1.0 });
    let head = l.at(1.0, 0.0);
    pb.line(tail.0, tail.1, mid.0, mid.1, l.tint(RUBBER));
    let c = l.tint(e.kind.color());
    pb.plot(head.0, head.1, c);
    pb.plot(head.0 + 1, head.1, c);
    pb.plot(head.0, head.1 - 1, l.tint((255, 255, 255)));
    pb.plot(head.0 + 1, head.1 + 1, l.tint(CERAMIC_SHADE));
}

// ---- Spinner Lid ------------------------------------------------------------

/// The lid's disk: major axis fixed on a slant in screen space, minor axis
/// breathing with the weave clock (`floor_lab::lid_face`: 1 = broad face,
/// 0 = edge-on, which is the instant the weave flips).
fn lid_disk(pb: &mut PixelBuffer, l: &Local, e: &Enemy, major: f32, lift: i32) {
    let face = floor_lab::lid_face(e);
    let minor = (major * 0.75 * face).max(0.5);
    // Slanted major axis.
    let (ax, ay) = (0.93_f32, -0.37_f32);
    let (bx, by) = (0.37_f32, 0.93_f32);
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32 - lift;
    let reach = major.ceil() as i32 + 1;
    let body = e.kind.color();
    let rim_hi = (232, 252, 255);
    let rim_dark = (34, 84, 104);
    let inner = mix(body, (20, 60, 80), 0.25);
    // Floating shadow on the floor below.
    for dx in -(reach - 2)..=(reach - 2) {
        shade(pb, cx + dx, cy + lift + 3, 0.28);
    }
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let a = dx as f32 * ax + dy as f32 * ay;
            let b = dx as f32 * bx + dy as f32 * by;
            let q = (a / major) * (a / major) + (b / minor) * (b / minor);
            if q > 1.0 {
                continue;
            }
            let c = if q > 0.55 || minor < 1.0 {
                // The bright rim: lit up-left, dark down-right.
                if (dx as f32) * LIGHT.x + (dy as f32) * LIGHT.y >= -0.2 {
                    rim_hi
                } else {
                    rim_dark
                }
            } else if q < 0.12 {
                // The knob at the lid's centre.
                rim_hi
            } else {
                inner
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }
    if minor < 1.0 {
        // Edge-on: guarantee an unbroken bright sliver.
        let m = major.round() as i32;
        for k in -m..=m {
            let x = cx + (k as f32 * ax).round() as i32;
            let y = cy + (k as f32 * ay).round() as i32;
            pb.plot(x, y, l.tint(rim_hi));
            blend(pb, x, y + 1, rim_dark, 0.8);
        }
    }
}

/// Floating slanted disk whose bright rim alternates broad face and thin
/// edge. It is edge-on exactly when its weave flips handedness.
pub(crate) fn draw_spinnerlid(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    lid_disk(pb, &l, e, 4.0, 2);
}

pub(crate) fn draw_spinnerlid_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    lid_disk(pb, &l, e, 2.0, 1);
}

// ---- Glass Runner -----------------------------------------------------------

/// Clear ampoule on two long rubber feet: a teardrop bulb, fat behind and
/// drawn out to a neck in front, on exactly two legs with heavy boots (the
/// Walking Syringe is the thin one on four stilts). The glass is half transparent (the
/// floor shows through), with a hard highlight down the lit side, a dark
/// outline on the other and a drawn-out neck in front. The feet vault fore and
/// aft in opposition.
pub(crate) fn draw_glassrunner(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let a = step(e);
    let swing = if a { 3.0 } else { -3.0 };
    // Vault: the body rides a pixel forward on the leading foot.
    let ride = if a { 0.6 } else { -0.6 };
    // Long rubber legs, hip to foot.
    for (side, sw) in [(1.0_f32, swing), (-1.0_f32, -swing)] {
        let hip = l.at(ride - 0.5, side * 1.5);
        let knee = l.at(ride + sw * 0.3, side * 3.5);
        let foot = l.at(sw, side * 5.0);
        shade(pb, foot.0 + 1, foot.1 + 2, 0.35);
        pb.line(hip.0, hip.1, knee.0, knee.1, l.tint(RUBBER));
        pb.line(knee.0, knee.1, foot.0, foot.1, l.tint(RUBBER));
        pb.plot(knee.0, knee.1, l.tint(RUBBER_HI));
        // A big rubber boot: a 2×2 block plus a toe, so the two feet are the
        // heaviest marks on the body.
        for (du, dv) in [(0.0_f32, 0.0_f32), (1.0, 0.0), (0.0, side), (1.0, side), (2.0, 0.0)] {
            let (x, y) = l.at(sw + du, side * 5.0 + dv);
            pb.plot(x, y, l.tint(RUBBER));
        }
    }
    // The ampoule: a capsule 9 long, 3 wide, then the neck.
    let glass = e.kind.color();
    let reach = 7i32;
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let u = dx as f32 * l.f.x + dy as f32 * l.f.y - ride;
            let v = dx as f32 * l.p.x + dy as f32 * l.p.y;
            // Teardrop: a fat bulb at the rear (radius 2.6 at u = -2)
            // tapering to radius 1.0 at the shoulder (u = 3).
            let cu = u.clamp(-2.0, 3.0);
            let rad = 2.6 - (cu + 2.0) * 0.32;
            let d2 = (u - cu) * (u - cu) + v * v;
            if d2 > rad * rad {
                continue;
            }
            let edge = d2 > (rad - 0.9) * (rad - 0.9);
            if edge {
                let c = if lit(&l, u - cu, v) { (250, 255, 252) } else { (48, 78, 82) };
                pb.plot(cx + dx, cy + dy, l.tint(c));
            } else {
                blend(pb, cx + dx, cy + dy, l.tint(glass), 0.55);
            }
        }
    }
    // Neck and snapped tip.
    let n0 = l.at(ride + 5.0, 0.0);
    let n1 = l.at(ride + 6.5, 0.0);
    pb.line(n0.0, n0.1, n1.0, n1.1, l.tint((250, 255, 252)));
    // A drop of specimen left inside.
    let drop = l.at(ride - 1.5, 0.0);
    blend(pb, drop.0, drop.1, SPECIMEN_PINK, 0.7);
}

pub(crate) fn draw_glassrunner_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let sw = if step(e) { 2.0 } else { -2.0 };
    let back = l.at(-2.5, 0.0);
    let front = l.at(3.5, 0.0);
    pb.line(back.0, back.1, front.0, front.1, l.tint(e.kind.color()));
    let hi = l.at(1.0, 0.0);
    pb.plot(hi.0, hi.1, l.tint((250, 255, 252)));
    for v in [-1.0_f32, 1.0] {
        let (x, y) = l.at(-1.5, v);
        pb.plot(x, y, l.tint(e.kind.color()));
    }
    let f1 = l.at(sw, 3.0);
    let f2 = l.at(-sw, -3.0);
    pb.plot(f1.0, f1.1, l.tint(RUBBER));
    pb.plot(f2.0, f2.1, l.tint(RUBBER));
}

// ---- Walking Syringe --------------------------------------------------------

/// How far the plunger is drawn back (0..=3 px) and how full the barrel is
/// (0..=1), from the behaviour's phase.
fn plunger_pose(e: &Enemy) -> (f32, f32, bool) {
    match floor_lab::syringe_pose(e) {
        floor_lab::SyringePose::Walk => (1.0, 0.0, false),
        floor_lab::SyringePose::Tell(k) => (1.0 + 2.5 * k, k, true),
        floor_lab::SyringePose::Thrust => (0.0, 0.0, true),
        floor_lab::SyringePose::Retract(k) => (k, 0.0, true),
    }
}

/// Needle on stilt legs. Barrel, a long bright needle in front, a T plunger
/// behind and four thin stilts. Planted (tell, thrust, retract) the stilts
/// splay wide and stop stepping; during the tell the plunger draws back and
/// the barrel fills specimen pink.
pub(crate) fn draw_walkingsyringe(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let (pull, fill, planted) = plunger_pose(e);
    let a = step(e);
    // Stilts: front pair and rear pair, alternating unless planted.
    let spread = if planted { 5.0 } else { 4.0 };
    let (fa, fb) = if planted {
        (0.0, 0.0)
    } else if a {
        (1.5, -1.5)
    } else {
        (-1.5, 1.5)
    };
    let stilts = [
        (1.5_f32, 1.0_f32, 2.0 + fa),
        (1.5, -1.0, 2.0 + fb),
        (-2.0, 1.0, -2.5 + fb),
        (-2.0, -1.0, -2.5 + fa),
    ];
    for (hu, side, fu) in stilts {
        let hip = l.at(hu, side);
        let foot = l.at(fu, side * spread);
        shade(pb, foot.0 + 1, foot.1 + 1, 0.30);
        pb.line(hip.0, hip.1, foot.0, foot.1, l.tint(STEEL_DARK));
        pb.plot(foot.0, foot.1, l.tint(RUBBER));
    }
    // Barrel: u in [-3, 2], v in [-1, 1]; light edge on the lit side.
    let barrel = e.kind.color();
    for ui in -3i32..=2 {
        for vi in -1i32..=1 {
            let (x, y) = l.at(ui as f32, vi as f32);
            let c = if vi != 0 {
                if lit(&l, 0.0, vi as f32) { STEEL_HI } else { STEEL_DARK }
            } else if fill > 0.0 && (ui as f32) < -3.0 + 5.0 * fill {
                SPECIMEN_PINK
            } else {
                barrel
            };
            pb.plot(x, y, l.tint(c));
        }
    }
    // Finger flange: a 7-wide crossbar at the barrel's rear, the syringe's
    // signature T (with the thumb plate behind it, a double bar).
    for vi in -3i32..=3 {
        let (x, y) = l.at(-3.0, vi as f32);
        let c = if vi.abs() == 3 { STEEL_DARK } else { STEEL_HI };
        pb.plot(x, y, l.tint(c));
    }
    // Collar and needle (the needle tip sits at `floor_lab::SYRINGE_TIP`).
    let collar = l.at(3.0, 0.0);
    pb.plot(collar.0, collar.1, l.tint(STEEL_DARK));
    let n0 = l.at(4.0, 0.0);
    let n1 = l.at(floor_lab::SYRINGE_TIP, 0.0);
    pb.line(n0.0, n0.1, n1.0, n1.1, l.tint((246, 250, 255)));
    // Plunger: rod and thumb plate, drawn back by `pull`.
    let r0 = l.at(-4.0, 0.0);
    let r1 = l.at(-4.0 - pull, 0.0);
    pb.line(r0.0, r0.1, r1.0, r1.1, l.tint(CERAMIC_SHADE));
    for vi in -1i32..=1 {
        let (x, y) = l.at(-5.0 - pull, vi as f32);
        pb.plot(x, y, l.tint(CERAMIC));
    }
}

pub(crate) fn draw_walkingsyringe_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    let l = Local::of(e, cam);
    let (_, fill, planted) = plunger_pose(e);
    let back = l.at(-2.0, 0.0);
    let front = l.at(1.0, 0.0);
    let body = if fill > 0.3 { SPECIMEN_PINK } else { e.kind.color() };
    pb.line(back.0, back.1, front.0, front.1, l.tint(body));
    let n0 = l.at(2.0, 0.0);
    let n1 = l.at(floor_lab::SYRINGE_TIP - 2.0, 0.0);
    pb.line(n0.0, n0.1, n1.0, n1.1, l.tint((246, 250, 255)));
    for v in [-2.0_f32, 2.0] {
        let (x, y) = l.at(-2.0, v);
        pb.plot(x, y, l.tint(STEEL_HI));
    }
    let s = if planted { 3.0 } else if step(e) { 2.0 } else { 1.0 };
    let f1 = l.at(0.0, s);
    let f2 = l.at(0.0, -s);
    pb.plot(f1.0, f1.1, l.tint(RUBBER));
    pb.plot(f2.0, f2.1, l.tint(RUBBER));
}

/// The Walking Syringe's ray: dotted while it is a warning, solid while it
/// bites. Drawn from `floor_lab::syringe_ray`, the segment the hit test reads,
/// so what is shown is exactly what hurts. Belongs in the hostile layer
/// (self-lit, after the lighting pass).
pub(crate) fn draw_syringe_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let Some(ray) = floor_lab::syringe_ray(e) else {
        return;
    };
    let a = ray.from - cam;
    let d = ray.to - ray.from;
    let len = d.len();
    if len < 1.0 {
        return;
    }
    let dir = d * (1.0 / len);
    let side = Vec2::new(-dir.y, dir.x);
    let steps = len.round() as i32;
    let half = (floor_lab::SYRINGE_RAY_WIDTH * 0.5).floor() as i32;
    let warn: Rgb = (255, 150, 70);
    let live: Rgb = (255, 244, 214);
    let edge: Rgb = (255, 92, 120);
    for k in 0..=steps {
        let c = a + dir * k as f32;
        if ray.live {
            // The thrust fills the whole 26×3 lane.
            for w in -half..=half {
                let x = (c.x + side.x * w as f32).round() as i32;
                let y = (c.y + side.y * w as f32).round() as i32;
                pb.plot(x, y, if w == 0 { live } else { edge });
            }
        } else if (k / 2) % 2 == 0 {
            // Dotted warning: 2 px on, 2 px off, brightening as the plunger
            // comes back; the lane's two edges are marked at the far end.
            let x = c.x.round() as i32;
            let y = c.y.round() as i32;
            blend(pb, x, y, warn, 0.55 + 0.45 * ray.progress);
            if k >= steps - 1 {
                for w in [-half, half] {
                    let x = (c.x + side.x * w as f32).round() as i32;
                    let y = (c.y + side.y * w as f32).round() as i32;
                    blend(pb, x, y, warn, 0.8);
                }
            }
        }
    }
}

// ---- Autoclave Cart ---------------------------------------------------------

/// Box door on two big wheels. A steel box (9 long, 8 wide) with a recessed
/// door and a porthole, two black wheels proud of the flanks whose spoke pixel
/// turns with the walk, and a bright latch that slides across the front face.
/// The elite wears an open crown of safety latches and a double shoulder
/// stripe (`WAVE4_ROSTERS.md`, the elite rule).
pub(crate) fn draw_autoclavecart(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let l = Local::of(e, cam);
    let a = step(e);
    let body = e.kind.color();
    let door = mix(body, (30, 34, 46), 0.45);
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    // Shadow slab.
    for dx in -4..=5 {
        shade(pb, cx + dx, cy + 6, 0.32);
    }
    // Wheels first (the box overlaps their inner halves).
    for side in [1.0_f32, -1.0] {
        let (wx, wy) = l.at(-0.5, side * 5.0);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx * dx + dy * dy <= 5 {
                    pb.plot(wx + dx, wy + dy, l.tint(RUBBER));
                }
            }
        }
        // Hub and one turning spoke.
        pb.plot(wx, wy, l.tint(RUBBER_HI));
        let (sx, sy) = if a { (1, 0) } else { (0, 1) };
        pb.plot(wx + sx, wy + sy, l.tint(RUBBER_HI));
        pb.plot(wx - sx, wy - sy, l.tint(RUBBER_HI));
    }
    // The box.
    let reach = 8i32;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let u = dx as f32 * l.f.x + dy as f32 * l.f.y;
            let v = dx as f32 * l.p.x + dy as f32 * l.p.y;
            if u.abs() > 4.6 || v.abs() > 4.1 {
                continue;
            }
            let on_edge = u.abs() > 3.6 || v.abs() > 3.1;
            let c = if on_edge {
                // Light edge up-left, dark edge down-right.
                let eu = if u.abs() > 3.6 { u } else { 0.0 };
                let ev = if v.abs() > 3.1 { v } else { 0.0 };
                if lit(&l, eu, ev) { STEEL_HI } else { STEEL_DARK }
            } else if u.abs() < 2.6 && v.abs() < 2.3 {
                // The recessed door with its porthole.
                if u * u + v * v < 1.3 {
                    (120, 220, 236)
                } else {
                    door
                }
            } else {
                body
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }
    // The latch slides across the front face.
    let slide = (e.anim * 0.35).sin() * 2.5;
    let latch = l.at(4.0, slide);
    pb.plot(latch.0, latch.1, l.tint((255, 214, 96)));
    let latch2 = l.at(3.0, slide);
    pb.plot(latch2.0, latch2.1, l.tint((196, 150, 48)));
    if elite {
        // Double shoulder stripe across the box...
        // (two bands, each 2 px thick, flank to flank over the wheels' tops)
        for vi in -4i32..=4 {
            for u in [-0.5_f32, -1.5, -3.0, -4.0] {
                let (x, y) = l.at(u, vi as f32);
                let c = if u < -2.0 { (255, 206, 84) } else { (255, 232, 140) };
                pb.plot(x, y, l.tint(c));
            }
        }
        // ...and an open crown of safety latches: six prongs standing proud of
        // the box, three a side, open at the front and the back, so the crown
        // reads from either flank. Each prong is a bright tip on a dark root.
        for side in [1.0_f32, -1.0] {
            for u in [-3.5_f32, 0.5, 3.5] {
                let root = l.at(u, side * 5.0);
                let tip = l.at(u, side * 7.0);
                pb.line(root.0, root.1, tip.0, tip.1, l.tint((150, 104, 28)));
                pb.plot(tip.0, tip.1, l.tint((255, 240, 160)));
            }
        }
    }
}

pub(crate) fn draw_autoclavecart_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, elite: bool) {
    let l = Local::of(e, cam);
    let cx = l.pos.x.round() as i32;
    let cy = l.pos.y.round() as i32;
    let body = e.kind.color();
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let c = if dx == -2 || dy == -2 {
                STEEL_HI
            } else if dx == 2 || dy == 2 {
                STEEL_DARK
            } else {
                body
            };
            pb.plot(cx + dx, cy + dy, l.tint(c));
        }
    }
    pb.plot(cx, cy, l.tint((120, 220, 236)));
    for side in [1.0_f32, -1.0] {
        let (wx, wy) = l.at(0.0, side * 3.5);
        pb.plot(wx, wy, l.tint(RUBBER));
        pb.plot(wx + 1, wy, l.tint(RUBBER));
    }
    if elite {
        // Far LOD: the double stripe as two gold columns across the block and
        // one crown prong on each flank.
        for vi in -2i32..=2 {
            for u in [-0.5_f32, -2.0] {
                let (x, y) = l.at(u, vi as f32);
                pb.plot(x, y, l.tint((255, 206, 84)));
            }
        }
        for side in [1.0_f32, -1.0] {
            let (x, y) = l.at(1.5, side * 4.5);
            pb.plot(x, y, l.tint((255, 240, 160)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    fn body(kind: EnemyKind) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(32.0, 32.0), 1.0, &mut rng);
        e.facing = Vec2::new(1.0, 0.0);
        e
    }

    fn painted(kind: EnemyKind, anim: f32, full: bool) -> Vec<Rgb> {
        let mut pb = PixelBuffer::new(64, 32);
        let floor: Rgb = (9, 9, 9);
        pb.clear(floor);
        let mut e = body(kind);
        e.anim = anim;
        // Through the public entry points, so the table rows are exercised.
        if full {
            crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
        } else {
            crate::game::art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
        }
        pb.pixels().to_vec()
    }

    const LAB: [EnemyKind; 5] = [
        EnemyKind::SwabFoot,
        EnemyKind::SpinnerLid,
        EnemyKind::GlassRunner,
        EnemyKind::WalkingSyringe,
        EnemyKind::AutoclaveCart,
    ];

    #[test]
    fn every_lab_body_paints_a_light_and_a_dark_edge() {
        for kind in LAB {
            let px = painted(kind, 0.4, true);
            let body: Vec<&Rgb> = px.iter().filter(|c| **c != (9, 9, 9)).collect();
            assert!(body.len() >= 6, "{kind:?} paints a readable core ({})", body.len());
            let lum = |c: &Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            assert!(body.iter().any(|c| lum(c) > 600), "{kind:?} has a light edge");
            assert!(body.iter().any(|c| lum(c) < 260), "{kind:?} has a dark edge");
        }
    }

    #[test]
    fn every_lab_body_has_a_far_lod() {
        for kind in LAB {
            let px = painted(kind, 0.4, false);
            let n = px.iter().filter(|c| **c != (9, 9, 9)).count();
            assert!(n >= 3, "{kind:?} far LOD is visible ({n})");
            let full = painted(kind, 0.4, true).iter().filter(|c| **c != (9, 9, 9)).count();
            assert!(n <= full, "{kind:?} far LOD is no bigger than the full body");
        }
    }

    #[test]
    fn walkers_have_two_distinct_frames() {
        // `step` flips with the sign of sin(anim / 2).
        for kind in [
            EnemyKind::SwabFoot,
            EnemyKind::GlassRunner,
            EnemyKind::WalkingSyringe,
            EnemyKind::AutoclaveCart,
        ] {
            assert_ne!(painted(kind, 1.0, true), painted(kind, 8.0, true), "{kind:?}");
        }
    }

    #[test]
    fn the_elite_cart_wears_its_crown_and_stripe_at_both_lods() {
        let gold = |c: &Rgb| c.0 > 240 && c.1 > 190 && c.2 < 170;
        for full in [true, false] {
            let count = |elite: bool| {
                let mut pb = PixelBuffer::new(64, 32);
                pb.clear((9, 9, 9));
                let mut e = body(EnemyKind::AutoclaveCart);
                e.elite = elite;
                if full {
                    crate::game::art_enemy::draw_enemy(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
                } else {
                    crate::game::art_enemy::draw_enemy_lod(&mut pb, &e, Vec2::ZERO, (200, 200, 200));
                }
                pb.pixels().iter().filter(|c| gold(c)).count()
            };
            let (plain, elite) = (count(false), count(true));
            assert!(elite >= plain + 6, "full={full}: the elite is marked in gold ({plain} -> {elite})");
        }
    }

    #[test]
    fn the_five_silhouettes_differ() {
        for (i, a) in LAB.iter().enumerate() {
            for b in LAB.iter().skip(i + 1) {
                assert_ne!(painted(*a, 0.4, true), painted(*b, 0.4, true), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn the_syringe_ray_is_dotted_then_solid() {
        let count = |phase: u8| {
            let mut pb = PixelBuffer::new(96, 32);
            pb.clear((9, 9, 9));
            let mut e = body(EnemyKind::WalkingSyringe);
            floor_lab::pose_for_test(&mut e, phase, Vec2::new(1.0, 0.0));
            draw_syringe_tell(&mut pb, &e, Vec2::ZERO);
            pb.pixels().iter().filter(|c| **c != (9, 9, 9)).count()
        };
        assert_eq!(count(floor_lab::SYRINGE_WALK), 0, "no ray while walking");
        let dotted = count(floor_lab::SYRINGE_TELL);
        let solid = count(floor_lab::SYRINGE_THRUST);
        assert!(dotted > 8, "the warning is visible ({dotted})");
        assert!(solid > dotted * 2, "the thrust fills the lane ({solid} vs {dotted})");
        assert_eq!(count(floor_lab::SYRINGE_RETRACT), 0, "no ray while retracting");
    }
}
