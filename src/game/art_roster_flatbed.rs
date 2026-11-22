//! The Flatbed's own bodies (`WAVE4_ROSTERS.md`, "Flatbed trailer"): Gravel
//! Tick, June Bug, Tar Rat, Loose Can, Toll Gull and Loose Bungee.
//!
//! Road-yellow, bare aluminium and white ties over dark oak. Every body here
//! is a full-detail draw plus a far-LOD draw with the `BodyDraw` signature
//! `art_enemy::bespoke_body`/`bespoke_lod` expect. The draws are
//! self-contained: they read only the `&Enemy` and the camera (the `Frame`
//! argument is accepted for the signature and ignored), so nothing in
//! `art_enemy` had to open up beyond naming `Frame`.
//!
//! Each kind has a 6–12 px core, a light edge (up-left, the game's light) and
//! a dark edge, and two walk frames chosen from `Enemy::anim`. Kinds with a
//! tell have a tell pose: the Loose Can rocks ([`can_pose`]), the Toll Gull
//! opens its beak before the shot, the Loose Bungee stretches its loop while
//! the crossbar is telling.
//!
//! ## Public draws for the hostile layer
//!
//! One tell is drawn outside the body, after the lighting pass:
//!
//! * [`draw_loose_can_tell`]`(pb, e, cam)`: the Loose Can's marked roll (lane
//!   edges on each leg, a pip at the bounce, a cross at the stop). Call it for
//!   every live `EnemyKind::LooseCan`; it draws nothing unless the can is
//!   rocking or rolling ([`can_pose`]). It does its own geometry from world
//!   coordinates and must **not** be culled by the body's screen box: the path
//!   reaches 48 px from the can. `art_enemy::draw_wave4_tells` already calls
//!   it, so a second call from the hostile-layer gate would only repaint the
//!   same pixels.
//!
//! The other tells are poses inside the body draw and need no call: the Toll
//! Gull's open beak ([`gull_beak_open`]) and the Loose Bungee's taut loop (its
//! crossbar itself is the Turnkey's, drawn by `art_enemy::draw_w3_tells`).
//! [`can_pose`], [`gull_beak_open`] and [`walk_frame`] are public readers.
//!
//! The Flatbed's elite is the existing Brute with its plated crest, so no
//! body here draws an elite attachment (the `elite` argument is ignored).
//!
//! Render-only: nothing here draws rng or writes simulation state.

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

use super::art_enemy::Frame;
use super::ecs::systems::floor_flatbed as rules;
use super::enemy::Enemy;

/// Seconds of hit flash that read as a full flash (`art_enemy::FLASH_MAX`).
const FLASH_MAX: f32 = 0.12;

/// Road yellow (beak, ring pull, hazard marks).
const ROAD_YELLOW: Rgb = (236, 196, 62);
/// Bare aluminium, lit.
const ALU_HI: Rgb = (236, 240, 244);
/// Bare aluminium, shaded.
const ALU_LO: Rgb = (96, 102, 112);
/// White tie.
const TIE_WHITE: Rgb = (240, 238, 228);

// ---- the local frame -------------------------------------------------------

/// One body's screen frame, rebuilt from the `&Enemy` (see the module doc).
#[derive(Clone, Copy)]
struct Fx {
    /// Screen position of the body's centre.
    pos: Vec2,
    /// Unit facing.
    f: Vec2,
    /// Unit lateral (facing rotated a quarter turn).
    p: Vec2,
    /// Pixel scale: 1.0 at a 3 px radius, never below it (a 6 px core floor).
    k: f32,
    /// Walk frame: 0 or 1.
    frame: u8,
    /// Hit flash, 0..=1.
    hit: f32,
}

impl Fx {
    fn of(e: &Enemy, cam: Vec2) -> Fx {
        let len_sq = e.facing.x * e.facing.x + e.facing.y * e.facing.y;
        let f = if len_sq > 1e-4 {
            let inv = 1.0 / len_sq.sqrt();
            Vec2::new(e.facing.x * inv, e.facing.y * inv)
        } else {
            Vec2::new(1.0, 0.0)
        };
        Fx {
            pos: Vec2::new(e.pos.x - cam.x, e.pos.y - cam.y),
            f,
            p: Vec2::new(-f.y, f.x),
            k: (e.radius.max(3.0) / 3.0).min(2.0),
            frame: walk_frame(e.anim),
            hit: (e.flash / FLASH_MAX).clamp(0.0, 1.0),
        }
    }

    /// Body-local `(u forward, v lateral)` in design pixels → a screen pixel.
    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        let (u, v) = (u * self.k, v * self.k);
        (
            (self.pos.x + self.f.x * u + self.p.x * v).round() as i32,
            (self.pos.y + self.f.y * u + self.p.y * v).round() as i32,
        )
    }

    /// The body's centre pixel.
    #[inline]
    fn centre(&self) -> (i32, i32) {
        (self.pos.x.round() as i32, self.pos.y.round() as i32)
    }

    /// A colour with the hit flash folded in.
    #[inline]
    fn c(&self, col: Rgb) -> Rgb {
        if self.hit <= 0.0 {
            col
        } else {
            mix(col, (255, 240, 210), (0.30 + self.hit * 0.56).min(0.86))
        }
    }
}

/// The two-frame walk cycle every body here shares: which half of the gait
/// phase `anim` is in.
#[inline]
pub fn walk_frame(anim: f32) -> u8 {
    if anim.sin() >= 0.0 {
        0
    } else {
        1
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
fn seg(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), col: Rgb) {
    pb.line(a.0, a.1, b.0, b.1, col);
}

/// Darken one pixel in place (a contact shadow), bounds-checked.
#[inline]
fn shadow(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, (0, 0, 0), amount));
}

/// A filled body-local box `u0..=u1 × v0..=v1`, with a light edge on the
/// screen-up-left side and a dark edge on the far side (one pixel each).
fn lit_box(pb: &mut PixelBuffer, fx: &Fx, u: (f32, f32), v: (f32, f32), mid: Rgb, hi: Rgb, lo: Rgb) {
    let corners = [fx.px(u.0, v.0), fx.px(u.1, v.0), fx.px(u.0, v.1), fx.px(u.1, v.1)];
    let x0 = corners.iter().map(|c| c.0).min().unwrap_or(0);
    let x1 = corners.iter().map(|c| c.0).max().unwrap_or(0);
    let y0 = corners.iter().map(|c| c.1).min().unwrap_or(0);
    let y1 = corners.iter().map(|c| c.1).max().unwrap_or(0);
    let k = fx.k.max(1e-3);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - fx.pos.x;
            let dy = y as f32 - fx.pos.y;
            let lu = (dx * fx.f.x + dy * fx.f.y) / k;
            let lv = (dx * fx.p.x + dy * fx.p.y) / k;
            if lu < u.0 - 0.5 || lu > u.1 + 0.5 || lv < v.0 - 0.5 || lv > v.1 + 0.5 {
                continue;
            }
            // The light comes from up-left in screen space: a pixel whose
            // up-left neighbour is outside the box is lit, one whose
            // down-right neighbour is outside is shaded.
            let inside = |ox: f32, oy: f32| {
                let (ddx, ddy) = (dx + ox, dy + oy);
                let a = (ddx * fx.f.x + ddy * fx.f.y) / k;
                let b = (ddx * fx.p.x + ddy * fx.p.y) / k;
                a >= u.0 - 0.5 && a <= u.1 + 0.5 && b >= v.0 - 0.5 && b <= v.1 + 0.5
            };
            let col = if !inside(-1.0, -1.0) {
                hi
            } else if !inside(1.0, 1.0) {
                lo
            } else {
                mid
            };
            pb.plot(x, y, fx.c(col));
        }
    }
}

// ---- Gravel Tick -----------------------------------------------------------

/// Gravel Tick: a squared pebble with splayed feet that crab-walks.
///
/// The pebble is drawn **screen-square** whatever the facing (a chip of
/// gravel does not turn its corners), 6×5 px with a pale top-left edge and a
/// dark bottom-right one. Four feet splay from the corners; the two walk
/// frames extend opposite diagonals, which is the crab-walk.
pub(crate) fn draw_gravel_tick(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    gravel_tick(pb, e, cam);
}

fn gravel_tick(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let (cx, cy) = fx.centre();
    let body = e.kind.color();
    let hi = mix(body, (255, 255, 250), 0.6);
    let lo = mix(body, (20, 18, 16), 0.65);
    let foot = fx.c((70, 62, 54));
    // Feet first, so the pebble covers the hips. Frame 0 reaches with the
    // up-left and down-right feet, frame 1 with the other diagonal.
    for (sx, sy) in [(-1i32, -1i32), (1, -1), (-1, 1), (1, 1)] {
        let long = (sx == sy) == (fx.frame == 0);
        let reach = if long { 5 } else { 4 };
        let hip = (cx + sx * 3, cy + sy * 2);
        let toe = (cx + sx * reach, cy + sy * (reach - 1));
        seg(pb, hip, toe, foot);
    }
    shadow(pb, cx - 1, cy + 3, 0.35);
    shadow(pb, cx, cy + 3, 0.35);
    shadow(pb, cx + 1, cy + 3, 0.35);
    // The pebble: x in -3..=2, y in -2..=2.
    for y in -2i32..=2 {
        for x in -3i32..=2 {
            let col = if y == -2 || x == -3 {
                hi
            } else if y == 2 || x == 2 {
                lo
            } else {
                body
            };
            pb.plot(cx + x, cy + y, fx.c(col));
        }
    }
    // One chipped corner keeps it a pebble, not a crate.
    pb.plot(cx + 2, cy - 2, fx.c(lo));
    // Two eye pips on the side it is walking toward.
    let ex = if fx.f.x >= 0.0 { 1 } else { -2 };
    pb.plot(cx + ex, cy - 1, fx.c((24, 20, 18)));
    pb.plot(cx + ex, cy + 1, fx.c((24, 20, 18)));
}

/// Far LOD: a 3×3 grey square with one lit corner and two diagonal feet.
pub(crate) fn draw_gravel_tick_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    gravel_tick_lod(pb, e, cam);
}

fn gravel_tick_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let (cx, cy) = fx.centre();
    let body = fx.c(e.kind.color());
    pb.rect_fill(cx - 1, cy - 1, 3, 3, body);
    pb.plot(cx - 1, cy - 1, fx.c(mix(e.kind.color(), (255, 255, 250), 0.6)));
    pb.plot(cx + 1, cy + 1, fx.c(mix(e.kind.color(), (20, 18, 16), 0.65)));
    let d = if fx.frame == 0 { 1 } else { -1 };
    pb.plot(cx - 2, cy - 2 * d, fx.c((70, 62, 54)));
    pb.plot(cx + 2, cy + 2 * d, fx.c((70, 62, 54)));
}

// ---- June Bug --------------------------------------------------------------

/// June Bug: split wing cases like an open book above a dangling belly.
///
/// The two elytra hinge at the shoulders and open backwards into a V; the
/// frames are the book wide open and half shut. The belly is a round bulb
/// hanging **below** the hinge in screen space (it dangles whatever the
/// heading), with its shadow one row under: the belly shadow the field note
/// tells the player to step across.
pub(crate) fn draw_june_bug(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    june_bug(pb, e, cam);
}

fn june_bug(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let (cx, cy) = fx.centre();
    let shell = e.kind.color();
    let hi = mix(shell, (255, 226, 170), 0.55);
    let lo = mix(shell, (18, 10, 6), 0.6);
    let belly = mix(shell, (226, 190, 130), 0.5);
    // Belly shadow on the deck, then the dangling belly.
    for x in -2i32..=2 {
        shadow(pb, cx + x, cy + 6, 0.4);
    }
    pb.filled_circle(cx, cy + 2, 2, fx.c(belly));
    pb.plot(cx - 1, cy + 1, fx.c(mix(belly, (255, 250, 230), 0.5)));
    pb.plot(cx + 1, cy + 4, fx.c(lo));
    // The open book: each case is two parallel strokes (light spine, dark
    // outer edge) from the hinge back and outward.
    let spread = if fx.frame == 0 { 5.0 } else { 3.0 };
    for side in [-1.0f32, 1.0] {
        let hinge = fx.px(1.0, side * 0.5);
        let tip = fx.px(-3.5, side * spread);
        let hinge2 = fx.px(0.0, side * 1.5);
        let tip2 = fx.px(-4.0, side * (spread + 1.0));
        seg(pb, hinge2, tip2, fx.c(lo));
        seg(pb, hinge, tip, fx.c(if side < 0.0 { hi } else { shell }));
    }
    // Head: a dark pip ahead of the hinge.
    let head = fx.px(2.5, 0.0);
    pb.plot(head.0, head.1, fx.c((30, 20, 12)));
    let hinge = fx.px(1.0, 0.0);
    pb.plot(hinge.0, hinge.1, fx.c(hi));
}

/// Far LOD: a brown V over one pale belly pixel.
pub(crate) fn draw_june_bug_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    june_bug_lod(pb, e, cam);
}

fn june_bug_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let (cx, cy) = fx.centre();
    let shell = e.kind.color();
    let w = if fx.frame == 0 { 2 } else { 1 };
    pb.plot(cx, cy, fx.c(mix(shell, (255, 226, 170), 0.55)));
    pb.plot(cx - w, cy - 1, fx.c(shell));
    pb.plot(cx + w, cy - 1, fx.c(mix(shell, (18, 10, 6), 0.5)));
    pb.plot(cx, cy + 1, fx.c(mix(shell, (226, 190, 130), 0.5)));
    shadow(pb, cx, cy + 3, 0.35);
}

// ---- Tar Rat ---------------------------------------------------------------

/// Tar Rat: a low black rat with a stiff pennant tail, galloping.
///
/// Long body (10 px) with the nose pushed forward and down, a grey lit back so
/// it reads on dark oak, and a dead-straight tail ending in a triangular
/// pennant: the one shape nothing else on the deck has. Frames: legs flung
/// fore and aft, legs gathered under the belly.
pub(crate) fn draw_tar_rat(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    tar_rat(pb, e, cam);
}

fn tar_rat(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let tar = e.kind.color();
    let hi = (156, 148, 136);
    let lo = (12, 10, 10);
    // Legs under the body.
    let (fore, aft) = if fx.frame == 0 { (4.5, -4.5) } else { (1.5, -1.0) };
    for side in [-1.0f32, 1.0] {
        let a = fx.px(2.0, side * 1.5);
        let b = fx.px(fore, side * 3.0);
        seg(pb, a, b, fx.c(lo));
        let c = fx.px(-2.0, side * 1.5);
        let d = fx.px(aft, side * 3.0);
        seg(pb, c, d, fx.c(lo));
    }
    // The stiff tail and its pennant.
    let root = fx.px(-4.0, 0.0);
    let tip = fx.px(-10.0, 0.0);
    seg(pb, root, tip, fx.c(lo));
    let flag_a = fx.px(-7.0, -2.0);
    let flag_b = fx.px(-7.0, 2.0);
    seg(pb, flag_a, tip, fx.c(tar));
    seg(pb, flag_b, tip, fx.c(tar));
    seg(pb, flag_a, flag_b, fx.c(hi));
    // Body, then the low head.
    lit_box(pb, &fx, (-4.0, 3.0), (-2.0, 2.0), tar, hi, lo);
    lit_box(pb, &fx, (3.0, 5.5), (-1.0, 1.0), tar, hi, lo);
    let nose = fx.px(6.5, 0.0);
    pb.plot(nose.0, nose.1, fx.c((214, 150, 140)));
    for side in [-1.0f32, 1.0] {
        let eye = fx.px(4.5, side);
        pb.plot(eye.0, eye.1, fx.c(ROAD_YELLOW));
        let ear = fx.px(3.0, side * 2.0);
        pb.plot(ear.0, ear.1, fx.c(hi));
    }
}

/// Far LOD: a black dash, a grey back pixel and the pennant's cross stroke.
pub(crate) fn draw_tar_rat_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    tar_rat_lod(pb, e, cam);
}

fn tar_rat_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let a = fx.px(-2.0, 0.0);
    let b = fx.px(2.5, 0.0);
    seg(pb, a, b, fx.c(e.kind.color()));
    let back = fx.px(0.5, 0.0);
    pb.plot(back.0, back.1 - 1, fx.c((156, 148, 136)));
    let tail = fx.px(-4.5, 0.0);
    seg(pb, a, tail, fx.c((12, 10, 10)));
    let (fa, fb) = (fx.px(-4.5, -1.0), fx.px(-4.5, 1.0));
    seg(pb, fa, fb, fx.c((156, 148, 136)));
    if fx.frame == 0 {
        let leg = fx.px(2.5, 1.5);
        pb.plot(leg.0, leg.1, fx.c((12, 10, 10)));
    }
}

// ---- Loose Can -------------------------------------------------------------

/// What the can is doing, for the body's pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanPose {
    /// Trundling toward the rat.
    Walk,
    /// The tell: rocking in place, ring pull beating against the deck.
    Rock,
    /// Rolling along its marked path.
    Roll,
    /// Lying still after the roll (the punish window).
    Rest,
}

/// The pose a Loose Can is in, read from the phase `floor_flatbed` mirrors
/// onto the body.
#[inline]
pub fn can_pose(e: &Enemy) -> CanPose {
    match e.w4.phase {
        rules::CAN_PHASE_ROCK => CanPose::Rock,
        rules::CAN_PHASE_ROLL => CanPose::Roll,
        rules::CAN_PHASE_REST => CanPose::Rest,
        _ => CanPose::Walk,
    }
}

/// Loose Can: a dented cylinder lying on its side, ring pull at one end.
///
/// The can's axis is **across** its heading (a can rolls sideways): an 8×5 px
/// aluminium box with darker end caps, a specular stripe that steps between
/// two rows as it turns (the walk frames), a dent, and a road-yellow ring
/// pull. Tell pose: the whole can rocks a pixel back and forth and the ring
/// pull flips ends every beat. Rest pose: the stripe is gone and the ring
/// pull lies flat beside it.
pub(crate) fn draw_loose_can(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_can(pb, e, cam);
}

fn loose_can(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let mut fx = Fx::of(e, cam);
    let pose = can_pose(e);
    let beat = match pose {
        // The rock beats faster than the walk: its own clock, not the gait's.
        CanPose::Rock => ((e.w4.warn_age * 10.0) as i32 & 1) as u8,
        _ => fx.frame,
    };
    if pose == CanPose::Rock {
        let rock = if beat == 0 { 1.0 } else { -1.0 };
        fx.pos = Vec2::new(fx.pos.x + fx.f.x * rock, fx.pos.y + fx.f.y * rock);
    }
    let alu = e.kind.color();
    let (cx, cy) = fx.centre();
    for x in -3i32..=3 {
        shadow(pb, cx + x, cy + 3, 0.35);
    }
    lit_box(pb, &fx, (-2.0, 2.0), (-4.0, 4.0), alu, ALU_HI, ALU_LO);
    // End caps.
    for u in [-1.0f32, 0.0, 1.0] {
        let a = fx.px(u, -4.0);
        let b = fx.px(u, 4.0);
        pb.plot(a.0, a.1, fx.c(ALU_LO));
        pb.plot(b.0, b.1, fx.c(ALU_LO));
    }
    // The specular stripe along the axis: it steps as the can turns.
    if pose != CanPose::Rest {
        let u = if beat == 0 { -1.0 } else { 1.0 };
        seg(pb, fx.px(u, -3.0), fx.px(u, 3.0), fx.c(ALU_HI));
    }
    // The dent.
    let dent = fx.px(0.0, -1.5);
    pb.plot(dent.0, dent.1, fx.c(ALU_LO));
    // The ring pull.
    let ring = fx.c(ROAD_YELLOW);
    match pose {
        CanPose::Rest => {
            let a = fx.px(0.0, 5.0);
            let b = fx.px(0.0, 6.0);
            seg(pb, a, b, ring);
        }
        _ => {
            let u = if beat == 0 { 1.5 } else { -1.5 };
            let a = fx.px(0.0, 5.0);
            let b = fx.px(u, 6.0);
            seg(pb, a, b, ring);
        }
    }
}

/// Far LOD: a pale bar across the heading with one bright pixel and a yellow
/// end.
pub(crate) fn draw_loose_can_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_can_lod(pb, e, cam);
}

fn loose_can_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    seg(pb, fx.px(0.0, -2.0), fx.px(0.0, 2.0), fx.c(e.kind.color()));
    let s = fx.px(0.0, if fx.frame == 0 { -1.0 } else { 1.0 });
    pb.plot(s.0, s.1, fx.c(ALU_HI));
    let r = fx.px(0.0, 3.0);
    pb.plot(r.0, r.1, fx.c(ROAD_YELLOW));
}

/// The Loose Can's warned footprint, drawn from the **same** path the roll
/// hits along ([`rules::can_path`]): the outline of the 6 px lane on each leg,
/// a bright pip at the bounce, a cross at the stop.
///
/// Call it from the hostile layer (`art_enemy::draw_wave4_tells`) for a body
/// whose [`can_pose`] is `Rock` or `Roll`.
pub fn draw_loose_can_tell(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let pose = can_pose(e);
    if pose != CanPose::Rock && pose != CanPose::Roll {
        return;
    }
    let Some(path) = rules::can_path(e) else {
        return;
    };
    // The outline brightens through the tell and holds while the can rolls.
    let t = if pose == CanPose::Rock {
        (e.w4.warn_age / rules::CAN_ROCK_SECS).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let edge = mix((150, 112, 36), ROAD_YELLOW, t);
    let half = rules::CAN_LANE_HALF_WIDTH;
    let to_px = |p: Vec2| ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32);
    for leg in 0..path.legs() {
        let (a, b) = path.leg(leg);
        let d = Vec2::new(b.x - a.x, b.y - a.y);
        let len = (d.x * d.x + d.y * d.y).sqrt();
        if len < 0.5 {
            continue;
        }
        let n = Vec2::new(-d.y / len * half, d.x / len * half);
        for side in [-1.0f32, 1.0] {
            let o = Vec2::new(n.x * side, n.y * side);
            seg(
                pb,
                to_px(Vec2::new(a.x + o.x, a.y + o.y)),
                to_px(Vec2::new(b.x + o.x, b.y + o.y)),
                edge,
            );
        }
    }
    if path.legs() > 1 {
        let (bx, by) = to_px(path.points[1]);
        pb.plot(bx, by, TIE_WHITE);
        pb.plot(bx + 1, by, ROAD_YELLOW);
        pb.plot(bx - 1, by, ROAD_YELLOW);
        pb.plot(bx, by + 1, ROAD_YELLOW);
        pb.plot(bx, by - 1, ROAD_YELLOW);
    }
    let (ex, ey) = to_px(path.end());
    for d in -2i32..=2 {
        pb.plot(ex + d, ey + d, TIE_WHITE);
        pb.plot(ex + d, ey - d, TIE_WHITE);
    }
}

// ---- Toll Gull -------------------------------------------------------------

/// Toll Gull: white elbow wings, a long flat beak, hovering broadside.
///
/// Each wing is two strokes bent at the elbow (shoulder → elbow forward and
/// out → black tip swept back), so the bird is a shallow white M from above.
/// The beak is a long road-yellow bar. Frames: wingtips out, wingtips tucked.
/// Tell pose: in the last [`rules::GULL_BEAK_OPEN_SECS`] before a shot the
/// beak splits into an open V: "reverse as the beak opens".
pub(crate) fn draw_toll_gull(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    toll_gull(pb, e, cam);
}

fn toll_gull(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let (cx, cy) = fx.centre();
    let white = e.kind.color();
    let grey = (128, 136, 148);
    let tip_col = (28, 28, 32);
    for x in -2i32..=2 {
        shadow(pb, cx + x, cy + 7, 0.35);
    }
    let span = if fx.frame == 0 { 8.0 } else { 6.0 };
    for side in [-1.0f32, 1.0] {
        let shoulder = fx.px(0.0, side);
        let elbow = fx.px(2.0, side * 4.0);
        let tip = fx.px(-2.5, side * span);
        // Dark trailing edge one pixel behind each stroke.
        let shoulder_b = fx.px(-1.0, side);
        let elbow_b = fx.px(1.0, side * 4.0);
        seg(pb, shoulder_b, elbow_b, fx.c(grey));
        seg(pb, shoulder, elbow, fx.c(white));
        seg(pb, elbow, tip, fx.c(white));
        pb.plot(tip.0, tip.1, fx.c(tip_col));
        pb.plot(elbow.0, elbow.1, fx.c((255, 255, 255)));
    }
    // Body and tail.
    lit_box(pb, &fx, (-2.0, 2.0), (-1.0, 1.0), white, (255, 255, 255), grey);
    let tail = fx.px(-3.5, 0.0);
    pb.plot(tail.0, tail.1, fx.c(grey));
    // The long flat beak; open just before the shot.
    let beak = fx.c(ROAD_YELLOW);
    if gull_beak_open(e) {
        seg(pb, fx.px(2.5, 0.0), fx.px(6.5, -1.5), beak);
        seg(pb, fx.px(2.5, 0.0), fx.px(6.5, 1.5), beak);
        let throat = fx.px(4.0, 0.0);
        pb.plot(throat.0, throat.1, fx.c((196, 60, 48)));
    } else {
        seg(pb, fx.px(2.5, 0.0), fx.px(6.5, 0.0), beak);
    }
    let eye = fx.px(1.5, -1.0);
    pb.plot(eye.0, eye.1, fx.c(tip_col));
}

/// Is the gull's beak open (the shot is less than
/// [`rules::GULL_BEAK_OPEN_SECS`] away)?
#[inline]
pub fn gull_beak_open(e: &Enemy) -> bool {
    e.stun <= 0.0 && e.fire_cd <= rules::GULL_BEAK_OPEN_SECS
}

/// Far LOD: a white chevron with black tips and one yellow beak pixel.
pub(crate) fn draw_toll_gull_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    toll_gull_lod(pb, e, cam);
}

fn toll_gull_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let white = fx.c(e.kind.color());
    let span = if fx.frame == 0 { 3.5 } else { 2.5 };
    let nose = fx.px(1.0, 0.0);
    for side in [-1.0f32, 1.0] {
        let tip = fx.px(-1.0, side * span);
        seg(pb, nose, tip, white);
        pb.plot(tip.0, tip.1, fx.c((28, 28, 32)));
    }
    let beak = fx.px(2.5, 0.0);
    pb.plot(beak.0, beak.1, fx.c(ROAD_YELLOW));
    if gull_beak_open(e) {
        let b2 = fx.px(3.5, 1.0);
        pb.plot(b2.0, b2.1, fx.c(ROAD_YELLOW));
    }
}

// ---- Loose Bungee ----------------------------------------------------------

/// Loose Bungee: a steel J hook dragging a loop of striped rubber.
///
/// The hook leads: a straight shank with the J's return curling to one side.
/// The loop trails behind as a ring of alternating red and white dashes that
/// swings side to side (the walk frames). Tell pose: while its crossbar is
/// telling the loop is pulled taut into a long thin ellipse and the hook
/// glints.
pub(crate) fn draw_loose_bungee(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_bungee(pb, e, cam);
}

fn loose_bungee(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let rubber = e.kind.color();
    let taut = e.w3.bar_telling();
    // The loop: 12 dashes round an ellipse behind the hook.
    let (ru, rv) = if taut { (4.5, 1.2) } else { (3.0, 2.2) };
    let sway = if taut {
        0.0
    } else if fx.frame == 0 {
        1.0
    } else {
        -1.0
    };
    let centre_u = -1.0 - ru;
    for i in 0..12 {
        let a = i as f32 * (std::f32::consts::TAU / 12.0);
        let (sin, cos) = a.sin_cos();
        // The far end of the loop swings; the end tied to the hook does not.
        let swing = sway * (0.5 - 0.5 * cos);
        let p = fx.px(centre_u + cos * ru, sin * rv + swing);
        let col = if i % 2 == 0 { rubber } else { TIE_WHITE };
        pb.plot(p.0, p.1, fx.c(col));
        if sin > 0.5 {
            shadow(pb, p.0, p.1 + 1, 0.3);
        }
    }
    // The J hook: shank, bend, return.
    let steel = (176, 182, 190);
    let steel_lo = (70, 76, 86);
    seg(pb, fx.px(-1.0, 0.0), fx.px(4.0, 0.0), fx.c(steel));
    seg(pb, fx.px(4.0, 0.0), fx.px(5.0, 1.5), fx.c(steel));
    seg(pb, fx.px(5.0, 1.5), fx.px(4.0, 3.0), fx.c(steel_lo));
    seg(pb, fx.px(4.0, 3.0), fx.px(2.0, 3.0), fx.c(steel_lo));
    let glint = fx.px(4.0, 0.0);
    pb.plot(
        glint.0,
        glint.1,
        fx.c(if taut { (255, 255, 255) } else { ALU_HI }),
    );
    if taut {
        let g2 = fx.px(5.0, 1.5);
        pb.plot(g2.0, g2.1, fx.c(ROAD_YELLOW));
    }
}

/// Far LOD: a grey hook corner leading two red-and-white loop pixels.
pub(crate) fn draw_loose_bungee_lod(pb: &mut PixelBuffer, _fr: &Frame, e: &Enemy, cam: Vec2, _elite: bool) {
    loose_bungee_lod(pb, e, cam);
}

fn loose_bungee_lod(pb: &mut PixelBuffer, e: &Enemy, cam: Vec2) {
    let fx = Fx::of(e, cam);
    let steel = fx.c((176, 182, 190));
    let a = fx.px(0.0, 0.0);
    let b = fx.px(2.0, 0.0);
    seg(pb, a, b, steel);
    let hook = fx.px(2.0, 1.0);
    pb.plot(hook.0, hook.1, fx.c((70, 76, 86)));
    let sway = if fx.frame == 0 { 1.0 } else { -1.0 };
    let l1 = fx.px(-1.5, sway);
    let l2 = fx.px(-3.0, 0.0);
    pb.plot(l1.0, l1.1, fx.c(e.kind.color()));
    pb.plot(l2.0, l2.1, fx.c(TIE_WHITE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::enemy::EnemyKind;
    use rand::SeedableRng;

    const FLOOR: Rgb = (40, 30, 22);
    type Inner = fn(&mut PixelBuffer, &Enemy, Vec2);

    const FULL: [(EnemyKind, Inner); 6] = [
        (EnemyKind::GravelTick, gravel_tick),
        (EnemyKind::JuneBug, june_bug),
        (EnemyKind::TarRat, tar_rat),
        (EnemyKind::LooseCan, loose_can),
        (EnemyKind::TollGull, toll_gull),
        (EnemyKind::LooseBungee, loose_bungee),
    ];
    const LOD: [(EnemyKind, Inner); 6] = [
        (EnemyKind::GravelTick, gravel_tick_lod),
        (EnemyKind::JuneBug, june_bug_lod),
        (EnemyKind::TarRat, tar_rat_lod),
        (EnemyKind::LooseCan, loose_can_lod),
        (EnemyKind::TollGull, toll_gull_lod),
        (EnemyKind::LooseBungee, loose_bungee_lod),
    ];

    fn body(kind: EnemyKind, anim: f32) -> Enemy {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut e = Enemy::spawn(kind, Vec2::new(24.0, 24.0), 1.0, &mut rng);
        e.anim = anim;
        e.fire_cd = 2.0;
        e
    }

    /// The painted mask of one draw on a flat floor.
    fn mask(draw: Inner, e: &Enemy) -> Vec<bool> {
        let mut pb = PixelBuffer::new(48, 48);
        pb.clear(FLOOR);
        draw(&mut pb, e, Vec2::new(0.0, 0.0));
        pb.pixels().iter().map(|&c| c != FLOOR).collect()
    }

    fn extent(m: &[bool]) -> (usize, usize, usize) {
        let (mut x0, mut x1, mut y0, mut y1, mut n) = (48, 0, 48, 0, 0);
        for (i, &on) in m.iter().enumerate() {
            if on {
                let (x, y) = (i % 48, i / 48);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
                n += 1;
            }
        }
        (x1.saturating_sub(x0) + 1, y1.saturating_sub(y0) + 1, n)
    }

    #[test]
    fn every_flatbed_body_paints_a_readable_core() {
        for (kind, draw) in FULL {
            let (w, h, n) = extent(&mask(draw, &body(kind, 0.5)));
            assert!(n >= 14, "{kind:?} paints only {n} pixels");
            assert!(w.max(h) >= 6, "{kind:?} core is {w}x{h}");
            assert!(w.max(h) <= 28, "{kind:?} sprawls to {w}x{h}");
        }
    }

    #[test]
    fn every_flatbed_body_has_two_walk_frames() {
        assert_ne!(walk_frame(0.5), walk_frame(4.0));
        for (kind, draw) in FULL.into_iter().chain(LOD) {
            let a = mask(draw, &body(kind, 0.5));
            let b = mask(draw, &body(kind, 4.0));
            let mut pa = PixelBuffer::new(48, 48);
            let mut pbuf = PixelBuffer::new(48, 48);
            pa.clear(FLOOR);
            pbuf.clear(FLOOR);
            draw(&mut pa, &body(kind, 0.5), Vec2::new(0.0, 0.0));
            draw(&mut pbuf, &body(kind, 4.0), Vec2::new(0.0, 0.0));
            assert!(
                a != b || pa.pixels() != pbuf.pixels(),
                "{kind:?} draws the same picture on both walk frames"
            );
        }
    }

    #[test]
    fn silhouettes_are_pairwise_distinct() {
        for table in [FULL, LOD] {
            for i in 0..table.len() {
                for j in (i + 1)..table.len() {
                    let a = mask(table[i].1, &body(table[i].0, 0.5));
                    let b = mask(table[j].1, &body(table[j].0, 0.5));
                    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
                    assert!(
                        differ >= 3,
                        "{:?} and {:?} differ in only {differ} pixels",
                        table[i].0,
                        table[j].0
                    );
                }
            }
        }
    }

    #[test]
    fn far_lod_is_smaller_than_the_full_body() {
        for ((kind, full), (_, lod)) in FULL.into_iter().zip(LOD) {
            let (_, _, n_full) = extent(&mask(full, &body(kind, 0.5)));
            let (_, _, n_lod) = extent(&mask(lod, &body(kind, 0.5)));
            assert!(n_lod >= 3, "{kind:?} LOD paints {n_lod} pixels");
            assert!(n_lod < n_full, "{kind:?} LOD ({n_lod}) is not lighter than full ({n_full})");
        }
    }

    #[test]
    fn each_body_has_a_light_and_a_dark_edge() {
        for (kind, draw) in FULL {
            let mut pb = PixelBuffer::new(48, 48);
            pb.clear(FLOOR);
            draw(&mut pb, &body(kind, 0.5), Vec2::new(0.0, 0.0));
            let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
            let painted: Vec<u32> = pb
                .pixels()
                .iter()
                .filter(|&&c| c != FLOOR)
                .map(|&c| lum(c))
                .collect();
            let hi = painted.iter().copied().max().unwrap_or(0);
            let lo = painted.iter().copied().min().unwrap_or(0);
            assert!(hi >= lo + 150, "{kind:?} has no edge contrast ({lo}..{hi})");
        }
    }

    #[test]
    fn the_gull_opens_its_beak_before_the_shot() {
        let mut e = body(EnemyKind::TollGull, 0.5);
        let closed = mask(toll_gull, &e);
        e.fire_cd = rules::GULL_BEAK_OPEN_SECS * 0.5;
        assert!(gull_beak_open(&e));
        let open = mask(toll_gull, &e);
        assert_ne!(closed, open, "the open beak is the tell");
        e.stun = 1.0;
        assert!(!gull_beak_open(&e), "a stunned gull cannot fire, so no tell");
    }

    #[test]
    fn the_can_rocks_and_rests_in_different_poses() {
        let mut e = body(EnemyKind::LooseCan, 0.5);
        assert_eq!(can_pose(&e), CanPose::Walk);
        let walk = mask(loose_can, &e);
        e.w4.phase = rules::CAN_PHASE_ROCK;
        e.w4.warn_age = 0.05;
        let rock_a = mask(loose_can, &e);
        e.w4.warn_age = 0.15;
        let rock_b = mask(loose_can, &e);
        assert_ne!(rock_a, rock_b, "the rock beats");
        e.w4.phase = rules::CAN_PHASE_REST;
        assert_eq!(can_pose(&e), CanPose::Rest);
        let _ = walk;
    }

    #[test]
    fn the_bungee_pulls_taut_while_its_bar_tells() {
        let mut e = body(EnemyKind::LooseBungee, 0.5);
        let slack = mask(loose_bungee, &e);
        e.w3.bar_t = crate::game::ecs::systems::enemy_wave3::CROSSBAR_TELL_SECS
            + crate::game::ecs::systems::enemy_wave3::CROSSBAR_LIVE_SECS;
        assert!(e.w3.bar_telling());
        assert_ne!(slack, mask(loose_bungee, &e));
    }

    #[test]
    fn the_can_tell_draws_only_while_warned() {
        let mut e = body(EnemyKind::LooseCan, 0.5);
        let mut pb = PixelBuffer::new(96, 96);
        pb.clear(FLOOR);
        draw_loose_can_tell(&mut pb, &e, Vec2::new(0.0, 0.0));
        assert!(pb.pixels().iter().all(|&c| c == FLOOR), "no tell while walking");
        rules::store_can_path(
            &mut e,
            &rules::CanPath::straight(Vec2::new(24.0, 24.0), Vec2::new(72.0, 24.0)),
        );
        e.w4.phase = rules::CAN_PHASE_ROCK;
        draw_loose_can_tell(&mut pb, &e, Vec2::new(0.0, 0.0));
        let lit = pb.pixels().iter().filter(|&&c| c != FLOOR).count();
        assert!(lit >= 80, "two 48 px lane edges, got {lit}");
    }
}
