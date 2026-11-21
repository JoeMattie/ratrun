//! Art for the player's rat. Pure drawing from the player's state.
//!
//! The rat is rasterised in its own local frame (`u` along the facing, `v`
//! across it), so it holds its shape at any of 360 degrees instead of being a
//! sprite that only looks right on the axes. Each pixel in a small box around
//! the player is supersampled against a spine + half-width profile (a pear:
//! wide haunches, narrow shoulders, wedge head) and blended over whatever the
//! floor already drew, with a dark rim and a down-right drop shadow so the
//! pale fur reads on every theme.
//!
//! Nothing here is stored between frames: the gait is keyed to the distance
//! travelled along the facing, and the tail's lag comes from `vel` trailing
//! `face` during a turn (the sim eases velocity toward the input).
//!
//! The one "over time" effect is the Tail Thrash charge (see [`TailCharge`]):
//! the tail is the weapon's charge meter, so it stretches as the charge fills
//! and glows once it is ready. The art still keeps no state of its own — the
//! charge is read off the weapon, and the single-frame "just charged" flash is
//! aged by the caller from a frame-cache slot it owns
//! ([`tail_ready_flash`] + `World::tail_ready_at`).

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

use super::player::Player;
use super::weapon::WeaponKind;

/// Collision radius of the rat in world pixels (the sim's `PLAYER_RADIUS`).
/// The sprite may extend past it (tail, ears, whiskers) but the body should
/// read as roughly this size so hits look fair.
pub const BODY_RADIUS: f32 = 3.0;

// --- Palette ---------------------------------------------------------------
const FUR_SHADOW: Rgb = (112, 104, 138);
const FUR_BASE: Rgb = (212, 206, 218);
const FUR_LIGHT: Rgb = (255, 255, 248);
const EAR_PINK: Rgb = (255, 172, 190);
const EAR_RIM: Rgb = (204, 104, 134);
const NOSE_PINK: Rgb = (255, 120, 150);
const PAW_PINK: Rgb = (250, 206, 204);
const TAIL_BASE: Rgb = (214, 122, 146);
const TAIL_TIP: Rgb = (255, 188, 198);
const EYE: Rgb = (18, 12, 22);
const HURT: Rgb = (255, 56, 64);
const GHOST: Rgb = (130, 200, 255);

/// Direction *toward* the light in screen space (up-left), matching the
/// down-right drop shadow.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Half-width of the body at each station along the spine, nose at `+u`.
/// Haunches peak just behind the hit-circle centre; the widest point stays
/// under `BODY_RADIUS` so the silhouette is honest about what can be hit.
const PROFILE: [(f32, f32); 10] = [
    (-3.6, 0.0),
    (-3.1, 1.7),
    (-2.0, 2.7),
    (-0.7, 2.65),
    (0.6, 2.0),
    (1.5, 1.3),
    (2.6, 1.9),
    (3.6, 1.35),
    (4.7, 0.6),
    (5.4, 0.0),
];
const U_TAIL: f32 = -3.6;
const U_NOSE: f32 = 5.4;

/// Tail length in world pixels (at rest) and the number of steps along it.
const TAIL_LEN: f32 = 13.0;
const TAIL_STEPS: usize = 28;
/// Extra tail length (px) at a full Tail Thrash charge, before the level bonus:
/// a charged thrash needs a tail that visibly says "this is the weapon"
/// (the burst's own reach is 60-84 px, four to seven times the sprites here).
const TAIL_GROW: f32 = 14.0;
/// Extra reach per Tail Thrash level: the tail tracks `active_reach`, which is
/// +6 px a level, at about a fifth of that in drawn length.
const TAIL_LEVEL_GROW: f32 = 0.05;
/// The most points a fully charged (and dashing, max level) tail ever uses.
const TAIL_MAX_STEPS: usize = 88;

/// Seconds the "just became ready" flash lasts (the curve lives in
/// [`tail_ready_flash`]; the caller only owns the timestamp).
pub const TAIL_READY_FLASH: f32 = 0.55;

// --- Tail Thrash palette ---------------------------------------------------
// The charge glow is painted in the thrash's own colours so the charged tail
// matches the arc it becomes: `THRASH_GLOW` is `WeaponKind::TailThrash::color`,
// `THRASH_HOT` the near-white its leading edge runs at (`draw_tail_thrash`).
const THRASH_GLOW: Rgb = (255, 178, 92);
const THRASH_HOT: Rgb = (255, 246, 214);
const THRASH_HALO: Rgb = (255, 138, 56);

// --- Spiked Tail palette ---------------------------------------------------
// The weapon is steel (`WeaponKind::SpikedTail::color`), the one weapon colour
// in the game without a hue: the spikes read apart from the tail's own rose and
// from the thrash's amber even when all three are on screen at once.
const SPIKE_STEEL: Rgb = (206, 214, 226);
const SPIKE_HOT: Rgb = (255, 255, 255);

/// Half-size of the box of pixels rasterised around the player.
const REACH: i32 = 10;

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> (i32, i32) {
    ((p.x - cam.x).round() as i32, (p.y - cam.y).round() as i32)
}

/// Shortest-arc interpolation between two angles (`t = 0` → `a`, `1` → `b`).
///
/// Used to swing the tail from its resting curl onto a lash's target line: the
/// shortest way round, so a whip aimed just past the rat's back does not take
/// the long way about the screen.
#[inline]
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    } else if d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    a + d * t.clamp(0.0, 1.0)
}

/// Body half-width at spine station `u` (0 outside the body).
fn half_width(u: f32) -> f32 {
    if u <= U_TAIL || u >= U_NOSE {
        return 0.0;
    }
    for i in 1..PROFILE.len() {
        let (u1, w1) = PROFILE[i];
        if u <= u1 {
            let (u0, w0) = PROFILE[i - 1];
            return w0 + (w1 - w0) * (u - u0) / (u1 - u0);
        }
    }
    0.0
}

/// Everything about this frame's pose, derived once.
#[derive(Clone, Copy)]
struct Pose {
    pos: Vec2,
    face: Vec2,
    perp: Vec2,
    /// Stretch along / squash across the facing (dash).
    su: f32,
    sv: f32,
    /// Gait phase (radians) and spine wiggle amplitude (px).
    phase: f32,
    wig: f32,
    /// Idle breathing, added to the haunch width (px).
    breathe: f32,
    /// 0 = standing, 1 = full run.
    moving: f32,
    /// 0..1 while dashing.
    dash: f32,
}

impl Pose {
    /// Lateral offset of the spine at station `u`: a wave that runs down the
    /// body while scurrying, fading out toward the head so the eyes stay put.
    #[inline]
    fn spine(&self, u: f32) -> f32 {
        let steady = ((3.2 - u) * 0.22).clamp(0.0, 1.0);
        self.wig * steady * (self.phase - u * 0.7).sin()
    }

    /// World offset → local `(u, v)` in unstretched body units.
    #[inline]
    fn local(&self, d: Vec2) -> (f32, f32) {
        (d.dot(self.face) / self.su, d.dot(self.perp) / self.sv)
    }

    /// Local `(u, v)` (v measured from the wiggling spine) → world position.
    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.face * (u * self.su) + self.perp * ((v + self.spine(u)) * self.sv)
    }

    #[inline]
    fn ear_centre(&self, side: f32) -> (f32, f32) {
        // Ears fold back and in while dashing.
        (2.0 - 0.6 * self.dash, side * (1.85 - 0.3 * self.dash))
    }

    /// Is local point `(u, v)` inside the body, grown by `margin`?
    #[inline]
    fn in_body(&self, u: f32, v: f32, margin: f32) -> bool {
        if u < U_TAIL - margin || u > U_NOSE + margin {
            return false;
        }
        let uc = u.clamp(U_TAIL + 0.3, U_NOSE - 0.3);
        let mut w = half_width(uc);
        if uc < 0.5 {
            w += self.breathe;
        }
        let cap = margin - (u - uc).abs();
        (v - self.spine(uc)).abs() <= w + cap.min(margin)
    }

    #[inline]
    fn in_ear(&self, u: f32, v: f32, margin: f32) -> bool {
        let r = 1.15 + margin;
        let (eu, ev) = self.ear_centre(1.0);
        let du = u - eu;
        let dv = v.abs() - ev;
        du * du + dv * dv <= r * r
    }
}

// ---------------------------------------------------------------------------
// Tail Thrash charge
// ---------------------------------------------------------------------------

/// The Tail Thrash's charge, as the art needs it. The tail *is* the charge
/// meter: at `charge == 0.0` (the frame the thrash fires) it is exactly the
/// tail the rat has always had, at `1.0` it is nearly twice as long, thicker
/// and (once `ready`) glowing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TailCharge {
    /// `1 - timer / cooldown`, the same fraction the HUD's cooldown pip uses.
    /// `0.0` = just fired, `1.0` = fully charged.
    pub charge: f32,
    /// The weapon's level (1-5): a levelled thrash reaches further, so its
    /// drawn tail does too.
    pub level: u8,
    /// Fully charged: the tail glows and Space will fire it.
    pub ready: bool,
    /// The one-off "just became ready" flash, `0.0..=1.0`. [`tail_charge`]
    /// always leaves it at zero: only the draw pass's caller can age it (see
    /// [`tail_ready_flash`]), and it is not a property of the weapon.
    pub flash: f32,
}

impl TailCharge {
    /// Drawn tail length in world pixels for this much charge.
    ///
    /// Linear in the charge so the growth reads as a filling meter, and it is
    /// already a third longer by half charge (half of a 10 s cooldown is 5 s of
    /// watching the tail grow, which is the whole point of the read-out).
    pub fn tail_len(&self) -> f32 {
        let level = 1.0 + TAIL_LEVEL_GROW * self.level.saturating_sub(1) as f32;
        (TAIL_LEN + TAIL_GROW * self.charge.clamp(0.0, 1.0)) * level
    }
}

/// The rat's Tail Thrash charge, or `None` when it does not own the weapon
/// (in which case the tail is drawn exactly as it always was).
///
/// `ready` is `timer <= 0` — the HUD's rule (`HudState::sync_active_ready`),
/// so the tail's glow and the HUD's READY pip turn on together.
pub fn tail_charge(player: &Player) -> Option<TailCharge> {
    let weapon = player
        .loadout
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::TailThrash)?;
    // The charge time the sim actually serves this weapon (level and the
    // fire-rate passive), exactly as `active_weapons_system` spends it.
    let fire_rate = player.loadout.stats.fire_rate_mult.max(0.1);
    let charge_time = (weapon.cooldown() / fire_rate).max(0.05);
    Some(TailCharge {
        charge: (1.0 - weapon.timer / charge_time).clamp(0.0, 1.0),
        level: weapon.level,
        ready: weapon.timer <= 0.0,
        flash: 0.0,
    })
}

/// Age the one-off "the thrash just finished charging" flash and return it,
/// `0.0..=1.0`.
///
/// `ready_at` is a single frame-cache slot owned by the caller (`NEG_INFINITY`
/// until the charge first tops out): the art may not keep state, and the
/// crossing has to be remembered somewhere. It is stamped with `elapsed`, the
/// run clock, so the flash freezes with the rest of the picture on a paused
/// frame and clears the moment the weapon is spent (charge back to zero).
/// No rng, no allocation, and nothing here is ever simulated.
pub fn tail_ready_flash(charge: Option<TailCharge>, elapsed: f32, ready_at: &mut f32) -> f32 {
    if !charge.is_some_and(|c| c.ready) {
        *ready_at = f32::NEG_INFINITY;
        return 0.0;
    }
    if !ready_at.is_finite() {
        *ready_at = elapsed;
    }
    let age = (elapsed - *ready_at).max(0.0);
    let left = (1.0 - age / TAIL_READY_FLASH).clamp(0.0, 1.0);
    // Snap on, ease out: a flat linear fade reads as a slow dim, not a pop.
    left * left
}

/// Final tint for every rat pixel: red on hurt, cold and see-through on the
/// "off" beat of the invulnerability blink.
#[derive(Clone, Copy)]
struct Tint {
    hurt: f32,
    ghost: bool,
}

impl Tint {
    #[inline]
    fn apply(&self, c: Rgb) -> Rgb {
        let c = if self.hurt > 0.0 {
            mix(c, HURT, self.hurt)
        } else {
            c
        };
        if self.ghost {
            mix(c, GHOST, 0.45)
        } else {
            c
        }
    }

    /// Opacity of the rat over the floor.
    #[inline]
    fn alpha(&self) -> f32 {
        if self.ghost {
            0.5
        } else {
            1.0
        }
    }
}

/// Blend `c` over the pixel already at `(x, y)`.
#[inline]
fn blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, mix(bg, c, a));
}

/// Darken the pixel already at `(x, y)` by `amount` (0..1).
#[inline]
fn darken(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if x < 0 || y < 0 || x as usize >= pb.w || y as usize >= pb.h {
        return;
    }
    let bg = pb.pixel_at(x as usize, y as usize);
    pb.plot(x, y, scale(bg, 1.0 - amount));
}

/// Draw the rat at `player.pos`, facing `player.face`. `elapsed` is the run
/// clock in seconds, for animation and the invulnerability blink.
pub fn draw_rat(pb: &mut PixelBuffer, player: &Player, elapsed: f32, cam: Vec2) {
    draw_rat_flash(pb, player, elapsed, cam, 0.0);
}

/// The **Spiked Tail**'s lash, as the art needs it.
///
/// Purely derived from the sim's `Lash` component and the run clock (see
/// `World::lash_visual`), so no layer of the renderer ever reads or writes
/// simulation state, and nothing here draws rng. Every field is at its
/// no-op default when no lash is in flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LashVisual {
    /// Rump -> impact point, unit (ZERO for a degenerate shot).
    pub dir: Vec2,
    /// Rump -> impact point, px. The tail's drawn length out at full extension.
    pub dist: f32,
    /// `0..=1` how far the chain has snapped out of its curl (`0` = the tail
    /// the rat has always had).
    pub ext: f32,
    /// `0..=1` how far the spikes stand out along the chain.
    pub bristle: f32,
    /// `0..=1` the white-hot flash at the tip just after the crack.
    pub flash: f32,
    /// `0..=1` the snap's smear (ghost chains beside the live one).
    pub smear: f32,
    /// `0..=1` the impact star: 1 on the crack, fading out.
    pub star: f32,
    /// The burst radius the star is drawn at (px).
    pub radius: f32,
    /// The impact point in world space (the star stays where the tail landed,
    /// even while the chain recoils or the rat walks away).
    pub point: Vec2,
}

/// [`draw_rat`], plus the Tail Thrash's one-off "just charged" flash
/// (`0.0..=1.0`, see [`tail_ready_flash`]).
///
/// The only caller that has a frame-to-frame slot to age the flash from is the
/// real draw pass (`World::draw_player`); every other caller — the composite
/// art sheets, boss set-pieces, tests — keeps using [`draw_rat`], which is this
/// with no flash at all.
pub fn draw_rat_flash(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    tail_flash: f32,
) {
    draw_rat_lashed(pb, player, elapsed, cam, tail_flash, None);
}

/// [`draw_rat_flash`], plus the **Spiked Tail**'s lash: the tail bristles with
/// spikes (on the real chain, so the weapon is the tail the rat already has)
/// and then snaps straight out at the target, with the impact star where it
/// lands.
///
/// The two tail-weapon visuals coexist: the Tail Thrash's charge owns the
/// tail's *colour and glow* (it is a charge meter), the Spiked Tail's lash owns
/// its *silhouette* (spike length, thickening, the snap). A rat that owns both
/// therefore reads as a glowing tail growing spikes before a lash — which is
/// exactly what it is, and neither effect has to be suppressed.
pub fn draw_rat_lashed(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    tail_flash: f32,
    lash: Option<LashVisual>,
) {
    let pose = pose_of(player, elapsed);
    let tint = tint_of(player, elapsed);

    if pose.dash > 0.0 {
        draw_dash_streaks(pb, &pose, cam);
    }
    let mut tail = tail_charge(player);
    if let Some(t) = tail.as_mut() {
        t.flash = tail_flash.clamp(0.0, 1.0);
    }
    draw_tail(pb, &pose, player.vel, elapsed, tint, cam, tail, lash);
    draw_feet(pb, &pose, tint, cam);
    draw_body(pb, &pose, tint, cam);
    draw_face(pb, &pose, elapsed, tint, cam);
    // Under the rat, so it never hides the body: the dash cooldown read-out.
    draw_dash_gauge(pb, player, elapsed, cam);
}

/// Everything about this frame's pose, derived once (split out so the tests can
/// redraw one layer — the body mask for the tail's "never covers the rat"
/// check — from the same numbers the real pass uses).
fn pose_of(player: &Player, elapsed: f32) -> Pose {
    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let perp = face.perp();
    let speed = player.vel.len();
    let top = player.loadout.stats.move_speed.max(1.0);
    let moving = (speed / top).clamp(0.0, 1.0);
    let dash = (player.dash_active / 0.16).clamp(0.0, 1.0).sqrt();
    Pose {
        pos: player.pos,
        face,
        perp,
        su: 1.0 + 0.32 * dash,
        sv: 1.0 - 0.14 * dash,
        // Keyed to distance along the facing: feet step with the ground and
        // stop dead when the rat does.
        phase: player.pos.dot(face) * 1.15,
        wig: 0.45 * moving * (1.0 - dash),
        breathe: 0.18 * (1.0 - moving) * (elapsed * 3.2).sin(),
        moving,
        dash,
    }
}

/// The hurt flash and the blink, for the frame being drawn.
fn tint_of(player: &Player, elapsed: f32) -> Tint {
    Tint {
        hurt: if player.hurt_flash > 0.0 {
            // Snap to near-solid red, then ease back as the flash runs out.
            (0.55 + player.hurt_flash * 3.0).min(0.85)
        } else {
            0.0
        },
        ghost: player.iframes > 0.0 && ((elapsed * 20.0) as i32 % 2 == 0),
    }
}

// ---- Dash gauge ------------------------------------------------------------

/// How many pips the gauge has, how far out it sits (px) and the screen arc it
/// spans (radians, `0` = due right, `PI/2` = straight down: the arc under the
/// rat). Screen-space, so it does not spin with the facing.
const GAUGE_PIPS: i32 = 7;
const GAUGE_RADIUS: f32 = 10.0;
// Left-to-right on screen so the refill reads like the HUD's charge bar.
const GAUGE_ARC: (f32, f32) = (0.70, 0.30);

const PIP_EMPTY: Rgb = (86, 62, 84);
const PIP_CHARGING: Rgb = (236, 172, 108);
const PIP_READY: Rgb = (255, 246, 214);
const PIP_DENIED: Rgb = (238, 74, 84);

/// The dash cooldown, attached to the rat: a shallow arc of five pips under it
/// that fills back up as the cooldown runs out.
///
/// States, all of them impossible to miss without any extra clutter:
/// - **recharging**: pips fill left to right in warm amber (partly-lit pips are
///   drawn at fractional brightness, so it reads as a smooth refill);
/// - **ready** (`dash_ready_flash` running): the whole arc pops near-white and
///   grows a one-pixel halo for a third of a second;
/// - **dashing**: the arc flashes bright as it drains;
/// - **refused** (`dash_denied` running): the arc turns red and jitters one
///   pixel side to side, so a press that could not be spent is never silent
///   (the matching `Sfx::DashDenied` blip comes from `player_system`).
///
/// Purely visual: no rng is drawn here (the jitter is keyed to the run clock),
/// so the draw pass stays deterministic.
pub fn draw_dash_gauge(pb: &mut PixelBuffer, player: &Player, elapsed: f32, cam: Vec2) {
    let max = player.loadout.stats.dash_cd.max(0.1);
    let ready = player.dash_cd <= 0.0;
    let dashing = player.dash_active > 0.0;
    let fill = if ready || dashing {
        1.0
    } else {
        (1.0 - player.dash_cd / max).clamp(0.0, 1.0)
    };
    let ready_pop = player.dash_ready_flash > 0.0;
    let denied = player.dash_denied > 0.0 && !ready;
    let centre = w2s(player.pos, cam);
    // A refused press shakes the gauge one pixel side to side at ~25 Hz.
    let shake = if denied {
        if (elapsed * 50.0) as i32 % 2 == 0 {
            1
        } else {
            -1
        }
    } else {
        0
    };

    for i in 0..GAUGE_PIPS {
        let t = if GAUGE_PIPS > 1 {
            i as f32 / (GAUGE_PIPS - 1) as f32
        } else {
            0.5
        };
        let a = GAUGE_ARC.0 + (GAUGE_ARC.1 - GAUGE_ARC.0) * t;
        let dir = Vec2::from_angle(std::f32::consts::PI * a);
        let x = centre.0 + (dir.x * GAUGE_RADIUS).round() as i32 + shake;
        let y = centre.1 + (dir.y * GAUGE_RADIUS).round() as i32;
        // 0..1 how full this pip is: fractional for the one the cooldown is
        // currently filling, so the refill rolls instead of stepping.
        let level = (fill * GAUGE_PIPS as f32 - i as f32).clamp(0.0, 1.0);
        let (color, lit) = if denied {
            (PIP_DENIED, 0.55)
        } else if ready || dashing {
            (PIP_READY, 1.0)
        } else {
            (PIP_CHARGING, level)
        };
        if lit > 0.0 {
            // Two pixels tall: one is a speck, two read as a pip at a glance.
            blend(pb, x, y, color, 0.35 + 0.65 * lit);
            blend(pb, x, y + 1, color, 0.25 + 0.55 * lit);
        } else {
            // Empty socket: a dim pip so the gauge's length is always readable.
            blend(pb, x, y, PIP_EMPTY, 0.7);
            blend(pb, x, y + 1, PIP_EMPTY, 0.5);
        }
        // The ready pop: a halo ring around each pip for the flash.
        if ready_pop {
            let halo = (255, 238, 196);
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                blend(pb, x + dx, y + dy, halo, 0.35);
            }
        }
    }
    // While dashing the arc leaves a hot streak behind it: a single bright pixel
    // each side keeps the drain visible without adding particles.
    if dashing {
        let a = std::f32::consts::PI * 0.5;
        let dir = Vec2::from_angle(a);
        let x = centre.0 + (dir.x * (GAUGE_RADIUS + 1.0)).round() as i32;
        let y = centre.1 + (dir.y * (GAUGE_RADIUS + 1.0)).round() as i32;
        pb.plot_add(x, y, (120, 120, 130));
    }
}

/// One frame of the **Laser Eyes** read, from `World::laser_visual`:
/// where the beam starts, which way it points, how long it is after the wall
/// stop, and how bright the bar and the muzzle dots are right now.
#[derive(Clone, Copy, Debug)]
pub struct LaserBeam {
    /// World position the beam leaves from (the rat's eye line).
    pub origin: Vec2,
    /// World angle of the pulse being drawn (radians).
    pub angle: f32,
    /// Beam length in px, already stopped by a wall.
    pub len: f32,
    /// 0..1: the bar blinks (lit just after a pulse, dark before the next).
    pub brightness: f32,
    /// 0..1: the twin muzzle dots stay on for the whole burst, dimmed in the
    /// dark interval so the source is never lost.
    pub muzzle: f32,
}

/// Laser Eyes' **twin amber muzzle dots** and its **blinking beam**.
///
/// The dots sit on the rat's eyes (the same local `(3.4, ±0.85)`
/// [`draw_face`] plots), so the beam visibly leaves the rat rather than the
/// floor, and they stay lit through the dark interval while the bar blinks —
/// that alternation is what reads as "laser eyes" in one half-block frame.
/// The bar is drawn additively in three passes (a white core and two amber
/// shoulders) because a 1-px line disappears against a busy floor.
pub fn draw_laser_eyes(pb: &mut PixelBuffer, player: &Player, beam: LaserBeam, cam: Vec2) {
    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let perp = face.perp();
    let eye_u = 3.4;
    let hot: Rgb = (255, 252, 224);
    let amber: Rgb = (255, 182, 42);
    let orange: Rgb = (242, 108, 22);

    // 1. Twin amber muzzle dots on the eyes (always live through the whole burst,
    //    brightening when a pulse fires and glowing warm amber in dark intervals).
    let m = beam.muzzle.clamp(0.0, 1.0);
    let eye_l = player.pos + face * eye_u + perp * -0.85;
    let eye_r = player.pos + face * eye_u + perp * 0.85;
    for eye_w in [eye_l, eye_r] {
        let e = w2s(eye_w, cam);
        pb.plot(e.0, e.1, mix(amber, hot, 0.45 + 0.55 * m));
        for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
            plot_add(pb, e.0 + dx, e.1 + dy, scale(amber, 0.35 + 0.45 * m));
        }
        if m > 0.65 {
            for &(dx, dy) in &[(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                plot_add(pb, e.0 + dx, e.1 + dy, scale(orange, 0.38 * m));
            }
        }
    }

    // 2. The blinking optical beam (pure additive — never overwrites floor with muddy dark brown).
    let b = beam.brightness.clamp(0.0, 1.0);
    if b <= 0.02 || beam.len <= 1.0 {
        return;
    }
    let dir = Vec2::from_angle(beam.angle);
    let perp_b = dir.perp();
    let start = player.pos + face * eye_u;
    let fuse_dist = beam.len.min(5.5);
    let fuse_pt = start + dir * fuse_dist;
    let end = start + dir * beam.len;

    // 2a. Twin converging filaments from each eye into the fused beam neck.
    for eye_w in [eye_l, eye_r] {
        let steps = 6i32;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let p = eye_w + (fuse_pt - eye_w) * t;
            let s = w2s(p, cam);
            plot_add(pb, s.0, s.1, scale(hot, 0.72 * b));
            plot_add(pb, s.0, s.1 + 1, scale(amber, 0.30 * b));
        }
    }

    // 2b. Main optical beam: white-hot core + golden-amber sheath + soft orange corona.
    let steps = (beam.len * 1.4).ceil().max(2.0) as i32;
    let mut last_core = (i32::MIN, i32::MIN);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let dist = t * beam.len;
        let p = start + dir * dist;
        let sc = w2s(p, cam);
        if sc == last_core {
            continue;
        }
        last_core = sc;
        // Subtle standing-wave intensity shimmer along the ray.
        let node = 0.85 + 0.15 * (dist * 0.7).sin().abs();
        let gain = b * node;
        // Core pixel: crisp white-gold blend lifted brightly above any floor.
        blend(pb, sc.0, sc.1, hot, (0.78 * gain).clamp(0.0, 0.95));
        plot_add(pb, sc.0, sc.1, scale(hot, 0.45 * gain));
        // Golden-amber inner shoulders (±1 px).
        for side in [-1.0f32, 1.0] {
            let s1 = w2s(p + perp_b * side, cam);
            if s1 != sc {
                plot_add(pb, s1.0, s1.1, scale(amber, 0.78 * gain));
            }
            let s2 = w2s(p + perp_b * (side * 2.0), cam);
            if s2 != s1 && s2 != sc {
                plot_add(pb, s2.0, s2.1, scale(orange, 0.34 * gain));
            }
        }
    }

    // 2c. Focal scorch starburst where the beam terminates (wall stop or max reach).
    let se = w2s(end, cam);
    blend(pb, se.0, se.1, hot, (0.90 * b).clamp(0.0, 1.0));
    for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
        plot_add(pb, se.0 + dx, se.1 + dy, scale(hot, 0.70 * b));
        plot_add(pb, se.0 + dx * 2, se.1 + dy * 2, scale(amber, 0.42 * b));
    }
    for &(dx, dy) in &[(1, 1), (-1, 1), (1, -1), (-1, -1)] {
        plot_add(pb, se.0 + dx, se.1 + dy, scale(amber, 0.55 * b));
    }
}

/// The **Tail Thrash** arc: the rat's tail stretched into a long whipping arc
/// that sweeps around it.
///
/// `visual` is `(angle, reach, brightness)` from `World::thrash_visual`: the
/// sweep angle in world radians, the arc's radius in px, and a 0..1 fade that
/// snaps the arc on with the trigger's screen shake and lingers out as the
/// burst winds down. The arc is drawn as a tapering whip: a bright leading edge
/// at `angle` with two dimmer ``ghost`` arcs trailing behind it (where the tip
/// just was), so a still frame reads as motion rather than a hoop.
pub fn draw_tail_thrash(
    pb: &mut PixelBuffer,
    player: &Player,
    visual: Option<(f32, f32, f32)>,
    cam: Vec2,
) {
    let Some((angle, reach, fade)) = visual else {
        return;
    };
    let fade = fade.clamp(0.0, 1.0);
    if fade <= 0.0 || reach <= 1.0 {
        return;
    }
    let origin = w2s(player.pos, cam);
    // Three passes: the live leading edge and two ghosts behind it (where the
    // tip just was), so one frame reads as motion, not as a hoop.
    for pass in 0..3 {
        let lead = -(pass as f32) * 0.17;
        let atten = if pass == 0 { 1.0 } else { 0.34 / pass as f32 };
        let steps = (reach * 1.3) as i32;
        for i in 0..steps.max(2) {
            let t = i as f32 / steps.max(2) as f32;
            // Radius from the rat's rump (4 px) out to the full reach.
            let r = 4.0 + (reach - 4.0) * t;
            // The arc lags behind the leading edge as it unwinds from the root,
            // which is what makes a still frame look like a whip in motion.
            let a = angle + lead - t * 0.42;
            let p = Vec2::from_angle(a);
            let nx = -p.y;
            let ny = p.x;
            let bx = origin.0 + (p.x * r).round() as i32;
            let by = origin.1 + (p.y * r).round() as i32;
            let weight = atten * fade * (0.32 + 0.68 * t);
            let hot = if pass == 0 {
                // Near-white at the tip, tail-rose at the root.
                (
                    (205.0 + 50.0 * t) as u8,
                    (95.0 + 145.0 * t) as u8,
                    (120.0 + 110.0 * t) as u8,
                )
            } else {
                TAIL_BASE
            };
            // Core plus a two-step band each side: the whip has mass, and a
            // 1-px line disappears against a busy floor. The 1.8x gain keeps
            // the additive blend above the floor's own dark tones (a raw
            // weight reads brown on the Sewer's stone).
            let glow = (weight * 1.8).min(1.4);
            plot_add(pb, bx, by, scale(hot, glow));
            for (k, gain) in [(1.0f32, 0.6f32), (2.0, 0.26)] {
                plot_add(
                    pb,
                    origin.0 + (p.x * r + nx * k).round() as i32,
                    origin.1 + (p.y * r + ny * k).round() as i32,
                    scale(hot, glow * gain),
                );
                plot_add(
                    pb,
                    origin.0 + (p.x * r - nx * k).round() as i32,
                    origin.1 + (p.y * r - ny * k).round() as i32,
                    scale(hot, glow * gain),
                );
            }
        }
    }
}

/// Additive plot with bounds check (the arc can sweep past the viewport).
#[inline]
fn plot_add(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb) {
    pb.plot_add(x, y, c);
}

/// Speed lines peeling off the haunches during a dash.
fn draw_dash_streaks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    for side in [-1.0f32, 1.0] {
        for j in 0..7 {
            let t = j as f32 / 7.0;
            let p = pose.place(-3.0 - j as f32 * 0.9, side * (2.2 - 0.9 * t));
            let s = w2s(p, cam);
            let f = pose.dash * (1.0 - t) * 0.9;
            pb.plot_add(s.0, s.1, scale((90, 120, 170), f));
        }
    }
}

/// The tail: a chain of half-pixel steps whose heading curls away from the
/// spine. Standing still it hooks lazily to one side and sways; running it
/// straightens and a wave whips down it; through a turn it swings toward
/// where the rat came from (the direction `vel` still points).
///
/// The tail is also the **Tail Thrash's charge meter**. With the weapon owned
/// it stretches from `TAIL_LEN` to `TAIL_LEN + TAIL_GROW` as the charge fills
/// (a little further per level), thickens toward the tip as it grows, and once
/// it is ready it glows in the thrash's own colours: a pulsing warm body, a
/// soft additive halo on the floor, a one-off flash at the moment it tops out,
/// and a twinkle at the tip while it waits. With no Tail Thrash
/// (`tail == None`) every one of those terms is exactly zero and the tail is
/// pixel for pixel the tail it always was.
///
/// A longer tail keeps the at-rest step and simply uses more points, so it
/// follows the same curl and sway (both are functions of the normalised
/// distance along the tail) and stays hooked to the rump.
#[allow(clippy::too_many_arguments)]
fn draw_tail(
    pb: &mut PixelBuffer,
    pose: &Pose,
    vel: Vec2,
    elapsed: f32,
    tint: Tint,
    cam: Vec2,
    tail: Option<TailCharge>,
    lash: Option<LashVisual>,
) {
    let m = pose.moving;
    // Signed angle from the facing to the velocity: non-zero only mid-turn.
    let lag = if vel.len_sq() > 4.0 {
        let vd = vel.normalized();
        let cross = pose.face.x * vd.y - pose.face.y * vd.x;
        (cross.atan2(pose.face.dot(vd)) * 1.5).clamp(-2.0, 2.0)
    } else {
        0.0
    };
    let whip = 1.0 - 0.85 * pose.dash;
    // The lash's straightening, and the spikes standing out along the chain
    // (both zero for a rat that does not own the Spiked Tail, so every term
    // below leaves the tail exactly the tail it always was).
    let snap = lash.map_or(0.0, |l| l.ext.clamp(0.0, 1.0));
    let bristle = lash.map_or(0.0, |l| l.bristle.clamp(0.0, 1.0));
    let lash_angle = lash.map_or(0.0, |l| l.dir.angle());
    // Length: the charge meter. With nothing to grow the chain keeps exactly
    // the at-rest tail's `TAIL_STEPS` points and step (so an uncharged tail is
    // the tail it has always been, to the pixel); a longer tail adds points at
    // the same spacing, which keeps the curve's shape as it stretches. A lash
    // stretches it all the way out to its target instead.
    let rest_len = tail.map_or(TAIL_LEN, |t| t.tail_len()) + 3.0 * pose.dash;
    let len = match lash {
        Some(l) if snap > 0.0 => (rest_len + (l.dist - rest_len) * snap).max(TAIL_LEN * 0.6),
        _ => rest_len,
    };
    let rest_step = (TAIL_LEN + 3.0 * pose.dash) / TAIL_STEPS as f32;
    let steps = ((len / rest_step).round() as usize).clamp(TAIL_STEPS, TAIL_MAX_STEPS);
    let step = len / steps as f32;
    let back = pose.face * -1.0;

    let mut pts = [(0i32, 0i32); TAIL_MAX_STEPS];
    let mut p = pose.place(-3.2, 0.0);
    for (i, slot) in pts.iter_mut().take(steps).enumerate() {
        let s = (i as f32 + 0.5) / steps as f32;
        let idle = 1.75 * s.powf(1.3) + 0.24 * (elapsed * 1.7 - s * 2.2).sin();
        let run = 0.22 * s + 0.6 * s * (elapsed * 13.0 - s * 5.5).sin() * whip;
        let theta = lag * s + idle * (1.0 - m) + run * m;
        let mut dir = back.rotate(theta);
        if snap > 0.0 {
            // A thrown whip leaves the rump pointing *back* and curls onto its
            // target over the first quarter of its length, so the tail never
            // has to pass through the rat to reach something in front of it.
            let bend = (s / 0.26).min(1.0) * snap;
            dir = Vec2::from_angle(lerp_angle(dir.angle(), lash_angle, bend));
        }
        p += dir * step;
        *slot = w2s(p, cam);
    }
    let pts = &pts[..steps];

    let charge = tail.map_or(0.0, |t| t.charge);
    let ready = tail.is_some_and(|t| t.ready);
    let flash = tail.map_or(0.0, |t| t.flash);
    // The idle pulse: about one beat a second, so a charged tail breathes
    // instead of sitting at a flat maximum (a still frame reads as "live").
    let pulse = 0.5 + 0.5 * (elapsed * 6.2).sin();
    let a = tint.alpha();

    // Shadow pass first so it never lands on the tail itself.
    if !tint.ghost {
        let mut last = (i32::MIN, i32::MIN);
        for &(x, y) in pts.iter() {
            if (x, y) != last {
                darken(pb, x + 1, y + 1, 0.3);
                last = (x, y);
            }
        }
    }

    // The tail itself: rose at the rump to near-white at the tip, warming
    // through the charge, and thick as well as long once it is ready.
    for (i, &(x, y)) in pts.iter().enumerate() {
        let s = i as f32 / (steps - 1) as f32;
        let mut colour = mix(TAIL_BASE, TAIL_TIP, s);
        // Warm through the charge: by half full the tail is already wearing the
        // thrash's orange, so it reads as "the weapon" long before it glows.
        colour = mix(colour, THRASH_GLOW, 0.5 * charge);
        // The widening band is the whip's skin (saturated orange); the chain
        // itself is its white-hot core, hottest at the tip, so a charged tail
        // reads as *light* rather than as a flat yellow sausage.
        let edge = if ready {
            mix(colour, THRASH_GLOW, 0.35 + 0.3 * pulse)
        } else {
            colour
        };
        if ready {
            let heat =
                ((0.15 + 0.55 * pulse + 0.45 * flash) * (0.15 + 0.85 * s * s)).min(1.0);
            colour = mix(colour, THRASH_HOT, heat);
        }
        blend(pb, x, y, tint.apply(colour), a);
        if ready {
            pb.plot_add(x, y, scale(THRASH_GLOW, (0.12 + 0.30 * pulse) * a));
        }

        // Width: the whip is thin at the rump and thickens toward the tip, so
        // growing the tail is visible from across a busy screen, not just at
        // the last pixel. The Spiked Tail's bristle thickens it too — a tail
        // that has just grown spikes is a *thicker* tail, thrash charge or not.
        let band = charge * (0.10 + 0.95 * s * s) + bristle * 0.85 * (0.35 + 0.65 * s);
        let kmax = band.round() as i32;
        if kmax <= 0 {
            continue;
        }
        let (ax, ay) = if i == 0 { pts[1] } else { pts[i - 1] };
        let d = Vec2::new((x - ax) as f32, (y - ay) as f32);
        if d.len_sq() <= 1e-4 {
            continue;
        }
        let d = d.normalized();
        let (nx, ny) = (-d.y, d.x);
        // Several radii often land on the same pixel (a diagonal normal steps
        // by one in both axes), and a second blend would double-tint the
        // blink's half alpha — so each offset is plotted once.
        let mut used = [(0i32, 0i32); 5];
        let mut n = 0;
        for k in 1..=kmax {
            let (dx, dy) = ((nx * k as f32).round() as i32, (ny * k as f32).round() as i32);
            if (dx, dy) == (0, 0) || used[..n].contains(&(dx, dy)) {
                continue;
            }
            used[n] = (dx, dy);
            n += 1;
            blend(pb, x + dx, y + dy, tint.apply(edge), a);
            blend(pb, x - dx, y - dy, tint.apply(edge), a);
        }
    }

    // The spikes: drawn *on the real chain* (every third point, alternating
    // sides, short at the rump and long at the tip), so the weapon reads as the
    // tail the rat already has growing a weapon, not as a second tail sprite.
    if bristle > 0.0 {
        for i in (1..steps).step_by(3) {
            let s = i as f32 / (steps - 1) as f32;
            let (x, y) = pts[i];
            let (ax, ay) = pts[i - 1];
            let d = Vec2::new((x - ax) as f32, (y - ay) as f32);
            if d.len_sq() <= 1e-4 {
                continue;
            }
            let d = d.normalized();
            let (nx, ny) = (-d.y, d.x);
            let side = if (i / 3) % 2 == 0 { 1.0 } else { -1.0 };
            let spike_len = (1.0 + (0.8 + 2.2 * s) * bristle).round().max(1.0) as i32;
            for k in 1..=spike_len {
                let px = x + (nx * side * k as f32).round() as i32;
                let py = y + (ny * side * k as f32).round() as i32;
                let hot = k == spike_len;
                let c = if hot { SPIKE_HOT } else { SPIKE_STEEL };
                blend(pb, px, py, tint.apply(c), a);
                // A one-pixel under-shadow keeps the steel off a busy floor.
                if hot && !tint.ghost {
                    darken(pb, px + 1, py + 1, 0.22);
                }
            }
        }
    }

    // The lash's own extras. None of this touches the tail a rat without the
    // Spiked Tail owns: with `lash == None` every term above and below is zero.
    if let Some(l) = lash {
        let (tx, ty) = pts[steps - 1];
        let root = w2s(pose.place(-3.2, 0.0), cam);
        // The smear: two ghost chains just behind the live one, only in the
        // frames the tail is actually travelling. Cheap (six plots each) and it
        // is what makes a still frame read as *speed* rather than a long tail.
        if l.smear > 0.0 && snap > 0.45 {
            let v = Vec2::new((tx - root.0) as f32, (ty - root.1) as f32);
            for (k, ghost) in [(1i32, 0.10f32), (2, 0.20)] {
                let atten = 0.55 * l.smear.clamp(0.0, 1.0) / k as f32;
                for side in [-1.0f32, 1.0] {
                    let r = v.rotate(ghost * side);
                    for step in 0..4 {
                        let t = 0.45 + 0.18 * step as f32;
                        let px = root.0 + (r.x * t).round() as i32;
                        let py = root.1 + (r.y * t).round() as i32;
                        plot_add(pb, px, py, scale(SPIKE_STEEL, atten));
                    }
                }
            }
        }
        // The white tip flash on the crack frame.
        if l.flash > 0.0 && snap > 0.25 {
            let g = l.flash.clamp(0.0, 1.0) * a;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    plot_add(pb, tx + dx, ty + dy, scale(SPIKE_HOT, 0.5 * g));
                }
            }
            for i in 0..8 {
                let ang = i as f32 * std::f32::consts::TAU / 8.0;
                let r = 1.0 + 4.0 * g;
                plot_add(
                    pb,
                    tx + (ang.cos() * r).round() as i32,
                    ty + (ang.sin() * r).round() as i32,
                    scale(SPIKE_HOT, 0.45 * g),
                );
            }
        }
        // The impact star: six spikes and a dashed ring at the burst radius, so
        // the small AoE is legible in the one frame the tail lands in. It stays
        // where the tail landed while the chain recoils.
        if l.star > 0.0 && l.radius > 0.5 {
            let g = l.star.clamp(0.0, 1.0);
            let c = w2s(l.point, cam);
            let r = l.radius.max(1.5);
            for i in 0..6 {
                let ang = i as f32 * std::f32::consts::TAU / 6.0 + 0.26;
                for (k, fade) in [(1.0f32, 1.0f32), (0.62, 0.5)] {
                    let reach = r * k;
                    plot_add(
                        pb,
                        c.0 + (ang.cos() * reach).round() as i32,
                        c.1 + (ang.sin() * reach).round() as i32,
                        scale(SPIKE_HOT, g * fade * 0.9),
                    );
                }
            }
            let dashes = ((r * 1.4) as i32).max(6);
            for i in 0..dashes {
                if i % 2 == 1 {
                    continue;
                }
                let ang = i as f32 / dashes as f32 * std::f32::consts::TAU + 0.12;
                plot_add(
                    pb,
                    c.0 + (ang.cos() * r).round() as i32,
                    c.1 + (ang.sin() * r).round() as i32,
                    scale(SPIKE_STEEL, g * 0.8),
                );
            }
        }
    }

    // Ready halo: a soft additive spill *around* the tail, drawn after the
    // tail's own pixels (a ring closer in would simply be buried by the two
    // pixel-thick tail) and before the rat's body, so it glows on the floor and
    // never washes out the rat. `pulse` breathes it, `flash` doubles it for one
    // beat.
    if ready {
        let gain = 0.028 * (0.55 + 0.9 * pulse + 1.2 * flash);
        for (i, &(x, y)) in pts.iter().enumerate() {
            let s = i as f32 / (steps - 1) as f32;
            let hot = gain * (0.55 + 0.45 * s);
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let d2 = dx * dx + dy * dy;
                    if !(2..=12).contains(&d2) {
                        continue;
                    }
                    // The inner ring is nearly white (light spilling off the
                    // tail), the outer one the thrash's orange, so the glow
                    // reads as *light* rather than as a brown smear.
                    let tint = if d2 <= 5 {
                        mix(THRASH_HALO, THRASH_HOT, 0.6)
                    } else {
                        THRASH_HALO
                    };
                    pb.plot_add(x + dx, y + dy, scale(tint, hot));
                }
            }
        }
    }

    if !ready {
        return;
    }
    // The tip is the part of the weapon that does the killing, so it gets the
    // read: a white-hot twinkle while it waits, and a one-off starburst at the
    // instant the charge tops out.
    let (tx, ty) = pts[steps - 1];
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        pb.plot_add(tx + dx, ty + dy, scale(THRASH_HOT, (0.30 + 0.45 * pulse) * a));
    }
    if flash > 0.0 {
        // A pop, not just a sparkle: a hot disc at the tip plus a twelve-ray
        // starburst that shrinks and fades over `TAIL_READY_FLASH`.
        let r = 1.0 + 5.0 * flash;
        let gain = (0.35 + 0.65 * flash).min(1.0);
        for dy in -1..=1 {
            for dx in -1..=1 {
                pb.plot_add(tx + dx, ty + dy, scale(THRASH_HOT, 0.45 * flash));
            }
        }
        for i in 0..12 {
            let ang = i as f32 * (std::f32::consts::TAU / 12.0) + 0.26;
            let long = if i % 2 == 0 { 1.0 } else { 0.55 };
            for (k, fade) in [(1.0f32, 1.0f32), (0.62, 0.45)] {
                let reach = r * long * k;
                let (dx, dy) = (
                    (ang.cos() * reach).round() as i32,
                    (ang.sin() * reach).round() as i32,
                );
                pb.plot_add(tx + dx, ty + dy, scale(THRASH_HOT, gain * fade));
            }
        }
    }
}

/// Four pink paws that poke out past the flanks in diagonal pairs.
fn draw_feet(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let reach = 0.25 + 0.75 * pose.moving;
    let a = tint.alpha();
    for (k, &(u0, side)) in [(1.0f32, 1.0f32), (-2.2, -1.0), (1.0, -1.0), (-2.2, 1.0)]
        .iter()
        .enumerate()
    {
        // Diagonal pairs move together (trot); a dash throws all four back.
        let ph = pose.phase + if k < 2 { 0.0 } else { std::f32::consts::PI };
        let swing = ph.sin() * 1.0 * pose.moving * (1.0 - pose.dash) - 0.8 * pose.dash;
        let u = u0 + swing;
        let v = side * (half_width(u0) + reach * (0.55 + 0.3 * ph.cos().max(0.0)));
        let s = w2s(pose.place(u, v), cam);
        if !tint.ghost {
            darken(pb, s.0 + 1, s.1 + 1, 0.3);
        }
        blend(pb, s.0, s.1, tint.apply(PAW_PINK), a);
    }
}

/// Fur colour at local `(u, v)`: a dome across the back lit from up-left.
fn fur_at(pose: &Pose, u: f32, v: f32) -> Rgb {
    let uc = u.clamp(U_TAIL + 0.3, U_NOSE - 0.3);
    let w = half_width(uc).max(0.7);
    let n = ((v - pose.spine(uc)) / w).clamp(-1.0, 1.0);
    let along = ((u - 0.6) / 4.4).clamp(-1.0, 1.0);
    let dome = 1.0 - n * n;
    let lit = n * pose.perp.dot(LIGHT) + 0.55 * along * along * along.signum() * pose.face.dot(LIGHT);
    let lum = (0.5 + 0.34 * dome + 0.6 * lit).clamp(0.0, 1.15);

    if lum < 0.75 {
        mix(FUR_SHADOW, FUR_BASE, lum / 0.75)
    } else {
        mix(FUR_BASE, FUR_LIGHT, (lum - 0.75) / 0.4)
    }
}

/// Body and ears: supersampled coverage blended over the floor, inside a dark
/// rim and a drop shadow.
fn draw_body(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let c = w2s(pose.pos, cam);
    let x0 = (c.0 - REACH).max(0);
    let x1 = (c.0 + REACH).min(pb.w as i32 - 1);
    let y0 = (c.1 - REACH).max(0);
    let y1 = (c.1 + REACH).min(pb.h as i32 - 1);
    let a_all = tint.alpha();
    let rim_colour: Rgb = if tint.hurt > 0.0 { (90, 0, 10) } else { (0, 0, 0) };

    for y in y0..=y1 {
        for x in x0..=x1 {
            // Pixel centre relative to the player, in world pixels.
            let d = Vec2::new(x as f32 + cam.x, y as f32 + cam.y) - pose.pos;
            if d.len_sq() > (REACH * REACH) as f32 {
                continue;
            }
            let (u, v) = pose.local(d);

            // 3x3 coverage.
            let mut body = 0u32;
            let mut ear = 0u32;
            for sy in -1..=1 {
                for sx in -1..=1 {
                    let o = Vec2::new(sx as f32 * 0.33, sy as f32 * 0.33);
                    let (su, sv) = pose.local(d + o);
                    if pose.in_ear(su, sv, 0.0) {
                        ear += 1;
                    } else if pose.in_body(su, sv, 0.0) {
                        body += 1;
                    }
                }
            }
            let total = body + ear;
            // Firm the edge up: this is pixel art, not a blur.
            let cover = ((total as f32 / 9.0 - 0.18) / 0.5).clamp(0.0, 1.0);

            // Rim (grown silhouette) and drop shadow (silhouette shifted
            // down-right, i.e. sampled up-left of this pixel).
            let mut dark = 0.0f32;
            if cover < 1.0 {
                if pose.in_body(u, v, 0.95) || pose.in_ear(u, v, 0.8) {
                    dark = 0.62;
                } else {
                    let (hu, hv) = pose.local(d - Vec2::new(1.3, 1.3));
                    if pose.in_body(hu, hv, 0.5) || pose.in_ear(hu, hv, 0.4) {
                        dark = 0.38;
                    }
                }
            }
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_colour, dark * a_all);
            if cover > 0.0 {
                let fur = fur_at(pose, u, v);
                let colour = if ear * 2 > total {
                    // Ear: pink bowl with a darker rim toward the outside.
                    let (eu, ev) = pose.ear_centre(1.0);
                    let r = ((u - eu).powi(2) + (v.abs() - ev).powi(2)).sqrt();
                    mix(EAR_PINK, EAR_RIM, ((r - 0.5) / 0.9).clamp(0.0, 1.0))
                } else {
                    fur
                };
                out = mix(out, tint.apply(colour), cover * a_all);
            }
            pb.plot(x, y, out);
        }
    }
}

/// Eyes, nose and whiskers: single pixels placed on top of the head.
fn draw_face(pb: &mut PixelBuffer, pose: &Pose, elapsed: f32, tint: Tint, cam: Vec2) {
    for side in [-1.0f32, 1.0] {
        let e = w2s(pose.place(3.4, side * 0.85), cam);
        pb.plot(e.0, e.1, EYE);
    }
    let n = w2s(pose.place(5.0, 0.0), cam);
    blend(pb, n.0, n.1, tint.apply(NOSE_PINK), tint.alpha());

    // Whiskers: faint additive flecks fanning from the snout, twitching.
    let twitch = 0.35 * (elapsed * 9.0).sin();
    for side in [-1.0f32, 1.0] {
        for (k, &(u, v)) in [(4.5f32, 1.9f32), (4.9, 2.9)].iter().enumerate() {
            let w = w2s(pose.place(u + twitch * (k as f32 + 1.0) * 0.5, side * v), cam);
            let g = if k == 0 { 70 } else { 40 };
            pb.plot_add(w.0, w.1, (g, g, g));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::{Level, Theme};
    use crate::game::weapon::WeaponKind;

    fn rat(pos: Vec2, ang: f32) -> Player {
        let mut p = Player::new(pos, WeaponKind::Gnaw);
        p.face = Vec2::from_angle(ang);
        p
    }

    /// A rat that owns a Tail Thrash at `charge` (0 = just fired, 1 = ready).
    /// `level` is the weapon level: the drawn tail scales a little with it.
    fn thrash_rat(pos: Vec2, ang: f32, charge: f32, level: u8) -> Player {
        let mut p = rat(pos, ang);
        p.loadout
            .apply(&crate::game::loadout::Upgrade::NewWeapon(WeaponKind::TailThrash));
        let i = p
            .loadout
            .weapons
            .iter()
            .position(|w| w.kind == WeaponKind::TailThrash)
            .unwrap();
        let w = &mut p.loadout.weapons[i];
        w.level = level;
        let cd = w.cooldown();
        // `ready` is `timer <= 0`, so a full charge is exactly 0 and `charge`
        // below that maps straight back onto the fraction the art reads.
        w.timer = if charge >= 1.0 { 0.0 } else { (1.0 - charge) * cd };
        p
    }

    /// Fixed rig for the pixel tests: one buffer size and one rat position, so
    /// several tests can compare the same pixels.
    const RIG_W: usize = 96;
    const RIG_H: usize = 76;
    const RIG_BG: Rgb = (24, 28, 30);
    fn rig_pos() -> Vec2 {
        Vec2::new(48.0, 38.0)
    }

    fn render_rat(p: &Player, elapsed: f32) -> PixelBuffer {
        let mut pb = PixelBuffer::new(RIG_W, RIG_H);
        pb.clear(RIG_BG);
        draw_rat(&mut pb, p, elapsed, Vec2::ZERO);
        pb
    }

    /// Everything the tail added: the pixels that differ from the same rat with
    /// no Tail Thrash at all. Empty at zero charge — that *is* the promise.
    fn tail_pixels(charge: f32, level: u8) -> Vec<(i32, i32)> {
        let plain = render_rat(&rat(rig_pos(), 2.2), 0.0);
        let charged = render_rat(&thrash_rat(rig_pos(), 2.2, charge, level), 0.0);
        let mut out = Vec::new();
        for y in 0..RIG_H as i32 {
            for x in 0..RIG_W as i32 {
                let a = plain.pixel_at(x as usize, y as usize);
                let b = charged.pixel_at(x as usize, y as usize);
                if a != b {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// How far the whole rat reaches from its own centre (px) and how many
    /// pixels it covers: the tail is its longest limb, so both grow with the
    /// charge.
    fn silhouette(charge: f32, level: u8) -> (f32, usize) {
        let pb = render_rat(&thrash_rat(rig_pos(), 2.2, charge, level), 0.0);
        let c = rig_pos();
        let (mut far, mut count) = (0.0f32, 0usize);
        for y in 0..RIG_H as i32 {
            for x in 0..RIG_W as i32 {
                if pb.pixel_at(x as usize, y as usize) == RIG_BG {
                    continue;
                }
                count += 1;
                far = far.max(Vec2::new(x as f32, y as f32).dist(c));
            }
        }
        (far, count)
    }

    /// The tail is the Tail Thrash's charge meter: it grows and thickens
    /// smoothly from the tail every rat has to a clearly longer, fatter,
    /// glowing one, and the growth is already obvious by half charge.
    #[test]
    fn tail_grows_with_charge_and_level() {
        let mut prev = 0.0f32;
        for i in 0..=10 {
            let charge = i as f32 / 10.0;
            let far = silhouette(charge, 1).0;
            assert!(
                far > prev,
                "charge {charge}: the rat reaches {far} px, not past {prev}"
            );
            prev = far;
        }
        // …and there is more tail on screen at the end than at the start, not
        // just one pixel further out (the tail thickens as it grows).
        assert!(silhouette(1.0, 1).1 > silhouette(0.0, 1).1 + 40);
        // Half charged is already visibly longer (roughly a third of the way to
        // the full stretch), not a rounding difference to hunt for.
        let rest = silhouette(0.0, 1).0;
        let half = silhouette(0.5, 1).0;
        let full = silhouette(1.0, 1).0;
        assert!(
            half > rest * 1.15,
            "half charge reaches {half} px vs {rest} at rest"
        );
        assert!(full > rest * 1.4, "full charge reaches {full} px vs {rest}");
        // …and the tail itself is where the growth is: at half charge it adds
        // far more than a pixel or two of tail.
        assert!(tail_pixels(0.5, 1).len() > 30);
        assert!(tail_pixels(1.0, 1).len() > tail_pixels(0.5, 1).len());
        // A levelled weapon reaches further, matching its `active_reach`.
        assert!(silhouette(1.0, 5).0 > full);
        // The length is linear in the charge (a filling meter, not a
        // last-second surprise).
        let len = |c: f32| {
            tail_charge(&thrash_rat(Vec2::ZERO, 0.0, c, 1))
                .unwrap()
                .tail_len()
        };
        assert!((len(0.0) - TAIL_LEN).abs() < 1e-4);
        assert!(len(0.5) > (len(0.0) + len(1.0)) * 0.48);
        assert!(len(1.0) >= TAIL_LEN + TAIL_GROW - 1e-4);
    }

    /// Without the weapon nothing changes at all: the rat's charge source says
    /// so, and the pixels of a rat with no Tail Thrash are exactly those of a
    /// rat that owns one with a full (zero) charge freshly spent — which draws
    /// the untouched tail, since every charged term is multiplied by zero.
    #[test]
    fn no_tail_thrash_draws_the_unchanged_tail() {
        let pos = Vec2::new(30.0, 30.0);
        let plain = rat(pos, 0.4);
        assert!(tail_charge(&plain).is_none());
        // Built by hand (no `Upgrade`), so the rat owns nothing at all.
        let bare = {
            let mut p = rat(pos, 0.4);
            p.loadout.weapons.clear();
            p
        };
        assert!(tail_charge(&bare).is_none());

        let shot = thrash_rat(pos, 0.4, 0.0, 1);
        assert_eq!(tail_charge(&shot).unwrap().charge, 0.0);
        let render = |p: &Player| {
            let mut pb = PixelBuffer::new(64, 60);
            pb.clear((24, 28, 30));
            draw_rat(&mut pb, p, 1.7, Vec2::ZERO);
            pb
        };
        let (a, b, c) = (render(&plain), render(&bare), render(&shot));
        for y in 0..60 {
            for x in 0..64 {
                assert_eq!(
                    a.pixel_at(x, y),
                    b.pixel_at(x, y),
                    "an empty loadout changed pixel {x},{y}"
                );
                assert_eq!(
                    a.pixel_at(x, y),
                    c.pixel_at(x, y),
                    "a freshly spent thrash changed pixel {x},{y}"
                );
            }
        }
    }

    /// The glow is a *full charge* state: nothing below ready gains the hot
    /// pulse, the halo or the tip twinkle, and a ready tail has all three — in
    /// 16-colour space too, where the fold has to land on a different (bright)
    /// ANSI slot than the resting tail.
    #[test]
    fn tail_glows_only_when_fully_charged() {
        use crate::term::{map_rgb_with, ColorDepth};
        use ratatui::style::Color;
        let near = tail_pixels(0.99, 1);
        let ready = tail_pixels(1.0, 1);
        // Below ready the tail is just a long tail. Ready, it also gains the
        // additive halo ring around itself, so it has strictly more pixels.
        assert!(
            !tail_charge(&thrash_rat(Vec2::ZERO, 0.0, 0.99, 1)).unwrap().ready,
            "0.99 must not count as ready"
        );
        assert!(
            tail_charge(&thrash_rat(Vec2::ZERO, 0.0, 1.0, 1)).unwrap().ready,
            "a spent-out timer is ready"
        );
        assert!(
            ready.len() > near.len() + 40,
            "the ready tail adds a halo: {} vs {}",
            ready.len(),
            near.len()
        );
        // In 16 colours the ready tail must contain pixels that fold to the
        // brightest slots, and the resting tail must contain none: the glow has
        // to read on a terminal without 24-bit colour.
        let is_bright = |c: Rgb| {
            matches!(
                map_rgb_with(ColorDepth::Ansi16, c),
                Color::LightYellow | Color::Yellow | Color::White
            )
        };
        let bright = |px: &[(i32, i32)], pb: &PixelBuffer| {
            px.iter()
                .filter(|&&(x, y)| is_bright(pb.pixel_at(x as usize, y as usize)))
                .count()
        };
        let rest_pb = render_rat(&rat(rig_pos(), 2.2), 0.0);
        let ready_pb = render_rat(&thrash_rat(rig_pos(), 2.2, 1.0, 1), 0.0);
        assert_eq!(
            bright(&tail_pixels(0.0, 1), &rest_pb),
            0,
            "the resting tail must not fold to a hot colour"
        );
        assert!(
            bright(&ready, &ready_pb) >= 6,
            "the ready tail needs hot pixels that survive the 16-colour fold"
        );
        // The pulse is *in* the colour, not just the presence of the glow: the
        // two opposite phases of the sine give different tail pixels.
        let phase = |t: f32| {
            let pb = render_rat(&thrash_rat(rig_pos(), 2.2, 1.0, 1), t);
            ready
                .iter()
                .map(|&(x, y)| pb.pixel_at(x as usize, y as usize))
                .collect::<Vec<_>>()
        };
        let low = phase(std::f32::consts::FRAC_PI_2 / 6.2);
        let high = phase(std::f32::consts::FRAC_PI_2 / 6.2 + std::f32::consts::PI / 6.2);
        assert_ne!(low, high, "the glow must pulse between its two phases");
    }

    /// Drawing is pure: same rat, same clock, same pixels (twice), and the rat
    /// itself is untouched — nothing here advances the sim or its rng.
    #[test]
    fn tail_draw_is_pure_and_touches_no_sim_state() {
        let pos = Vec2::new(30.0, 30.0);
        let p = thrash_rat(pos, 1.1, 1.0, 2);
        let render = || {
            let mut pb = PixelBuffer::new(64, 60);
            pb.clear((24, 28, 30));
            draw_rat(&mut pb, &p, 2.5, Vec2::ZERO);
            pb
        };
        let (a, b) = (render(), render());
        for y in 0..60 {
            for x in 0..64 {
                assert_eq!(a.pixel_at(x, y), b.pixel_at(x, y), "pixel {x},{y}");
            }
        }
        // The rat is borrowed immutably above, so it cannot have been written
        // to; assert the charge state the art reads is stable too.
        assert_eq!(tail_charge(&p), tail_charge(&p));
    }

    /// The pixels the rat's own body, feet and face paint at **full opacity**:
    /// the ones that come out the same whatever is underneath, and therefore
    /// the ones the tail can never show through.
    ///
    /// Found by drawing that layer over black and over white and keeping the
    /// pixels that are identical — a partially covered pixel (or the rim, or
    /// the drop shadow) is mixed with the background, so it is excluded without
    /// guessing at a threshold.
    fn body_mask(p: &Player, elapsed: f32) -> Vec<bool> {
        let layer = |bg: Rgb| {
            let pose = pose_of(p, elapsed);
            let tint = tint_of(p, elapsed);
            let mut pb = PixelBuffer::new(RIG_W, RIG_H);
            pb.clear(bg);
            draw_feet(&mut pb, &pose, tint, Vec2::ZERO);
            draw_body(&mut pb, &pose, tint, Vec2::ZERO);
            draw_face(&mut pb, &pose, elapsed, tint, Vec2::ZERO);
            pb
        };
        let (black, white) = (layer((0, 0, 0)), layer((255, 255, 255)));
        let mut mask = vec![false; RIG_W * RIG_H];
        for y in 0..RIG_H {
            for x in 0..RIG_W {
                mask[y * RIG_W + x] = black.pixel_at(x, y) == white.pixel_at(x, y);
            }
        }
        mask
    }

    /// The longer tail still comes out of the rump and never covers the rat:
    /// it is drawn before the body, so every body pixel comes out identical —
    /// at any heading, and with a swaying run on top.
    #[test]
    fn charged_tail_stays_attached_and_off_the_body() {
        for i in 0..16 {
            let ang = i as f32 / 16.0 * std::f32::consts::TAU;
            let mut plain_rat = rat(rig_pos(), ang);
            let mut charged_rat = thrash_rat(rig_pos(), ang, 1.0, 3);
            for p in [&mut plain_rat, &mut charged_rat] {
                p.vel = Vec2::from_angle(ang + 0.5) * 58.0;
            }
            let plain = render_rat(&plain_rat, 0.9);
            let charged = render_rat(&charged_rat, 0.9);
            let mask = body_mask(&plain_rat, 0.9);

            let mut added = 0;
            let (mut near, mut far) = (f32::MAX, 0.0f32);
            for y in 0..RIG_H as i32 {
                for x in 0..RIG_W as i32 {
                    let a = plain.pixel_at(x as usize, y as usize);
                    let b = charged.pixel_at(x as usize, y as usize);
                    if mask[y as usize * RIG_W + x as usize] {
                        assert_eq!(
                            a, b,
                            "heading {ang}: the tail changed body pixel {x},{y}"
                        );
                    }
                    if a == b {
                        continue;
                    }
                    added += 1;
                    // `a` is the resting tail's pixel here (the longer one took
                    // a different path), so only the charged side counts.
                    let d = Vec2::new(x as f32, y as f32).dist(rig_pos());
                    if b != RIG_BG {
                        near = near.min(d);
                        far = far.max(d);
                    }
                }
            }
            // It is drawn, it starts at the rump and it stops at the tip: a
            // chain from the body (not a detached blob) inside the reach the
            // charged length allows (tail + rump offset + the two-pixel halo).
            let len = tail_charge(&charged_rat).unwrap().tail_len();
            assert!(added > 40, "heading {ang}: the charged tail drew nothing");
            assert!(
                near < 8.0,
                "heading {ang}: nearest tail pixel is {near} px out — detached"
            );
            assert!(
                far < 3.5 + len + 3.0,
                "heading {ang}: tail reaches {far} px, past its {len} px length"
            );
        }
    }

    /// The art is render-only, so a run that *owns* a charged Tail Thrash must
    /// simulate exactly like the same run without it, even with every frame
    /// drawn (glow, flash, pulse and all). Two same-seed worlds, one of each,
    /// walk the same scripted path and are compared every tick: one rng draw
    /// out of the art would desync them on the first enemy spawn.
    #[test]
    fn drawing_a_charged_tail_draws_no_sim_state() {
        use crate::game::level::Theme;
        use crate::game::loadout::Upgrade;
        use crate::game::world::World;

        let mut plain = World::new(Theme::Sewer, 42);
        let mut tail = World::new(Theme::Sewer, 42);
        tail.player
            .loadout
            .apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
        let i = tail
            .player
            .loadout
            .weapons
            .iter()
            .position(|w| w.kind == WeaponKind::TailThrash)
            .unwrap();
        // Fully charged: the glow, the halo, the tip and the flash all run.
        tail.player.loadout.weapons[i].timer = 0.0;
        let mut pb = PixelBuffer::new(80, 40);
        for tick in 0..180 {
            let mv = Vec2::from_angle(tick as f32 * 0.07);
            plain.update(1.0 / 60.0, mv, false);
            tail.update(1.0 / 60.0, mv, false);
            plain.draw(&mut pb);
            tail.draw(&mut pb);
            assert_eq!(plain.kills, tail.kills, "tick {tick}: kills diverged");
            assert_eq!(
                plain.player.pos, tail.player.pos,
                "tick {tick}: the rat diverged"
            );
            let a: Vec<_> = plain.enemies().iter().map(|e| (e.pos, e.hp)).collect();
            let b: Vec<_> = tail.enemies().iter().map(|e| (e.pos, e.hp)).collect();
            assert_eq!(a, b, "tick {tick}: enemies diverged");
        }
    }

    /// The capture sheets for the tail's charge read-out.
    ///
    /// `RATRUN_DUMP=tailcharge cargo test dump_tail_charge_sheet` writes
    /// `/tmp/ratrun_tailcharge.ppm` (six 112 x 56 px panels, whole-screen feel)
    /// and `/tmp/ratrun_tailclose.ppm` (the same six at 76 x 46 px, for a hard
    /// look at the tail itself). Both are no-ops unless `RATRUN_DUMP` says
    /// `tailcharge`.
    #[test]
    fn dump_tail_charge_sheet() {
        use crate::game::enemy::EnemyKind;
        use crate::game::level::Theme;
        use crate::game::loadout::Upgrade;
        use crate::game::world::World;
        use rand::SeedableRng;

        if std::env::var("RATRUN_DUMP").as_deref() != Ok("tailcharge") {
            return;
        }
        // Where the idle pulse tops out / bottoms out (sin(elapsed * 6.2)).
        const PULSE_UP: f32 = std::f32::consts::FRAC_PI_2 / 6.2;
        const PULSE_DOWN: f32 = PULSE_UP + std::f32::consts::PI / 6.2;

        // One panel: a real world (floor, horde and all) with the thrash
        // charged to `charge` at weapon `level`. `prime` draws one throwaway
        // frame far in the past first, so the ready flash has already decayed —
        // without it the panel catches the flash itself.
        struct Shot {
            charge: f32,
            level: u8,
            elapsed: f32,
            prime: bool,
            horde: bool,
            theme: Theme,
            dash: bool,
        }
        let shot = |charge: f32| Shot {
            charge,
            level: 1,
            elapsed: 0.0,
            prime: true,
            horde: false,
            theme: Theme::Sewer,
            dash: false,
        };
        let panel = |w: usize, h: usize, s: &Shot| {
            let mut world = World::new(s.theme, 7);
            if !world.player.loadout.has(WeaponKind::TailThrash) {
                world
                    .player
                    .loadout
                    .apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
            }
            let i = world
                .player
                .loadout
                .weapons
                .iter()
                .position(|x| x.kind == WeaponKind::TailThrash)
                .expect("the thrash was just granted");
            {
                let weapon = &mut world.player.loadout.weapons[i];
                weapon.level = s.level;
                weapon.timer = if s.charge >= 1.0 {
                    0.0
                } else {
                    (1.0 - s.charge) * weapon.cooldown()
                };
            }
            world.player.face = Vec2::from_angle(-0.62);
            if s.horde {
                world.player.vel = world.player.face * 58.0;
                if s.dash {
                    world.player.dash_active = 0.16;
                }
                let mut rng = rand::rngs::StdRng::seed_from_u64(11);
                let p = world.player.pos;
                for k in 0..26 {
                    let a = k as f32 * 0.37;
                    let at = p + Vec2::from_angle(a) * (14.0 + (k % 5) as f32 * 9.0);
                    let kind = if k % 6 == 3 {
                        EnemyKind::Cat
                    } else {
                        EnemyKind::Skitterer
                    };
                    world.spawn_enemy_with_rng(kind, at, 30.0, &mut rng);
                }
            }
            let mut pb = PixelBuffer::new(w, h);
            if s.prime {
                world.elapsed = -10.0;
                world.draw(&mut pb);
            }
            world.elapsed = s.elapsed;
            world.draw(&mut pb);
            pb
        };

        // Nine panels, 3 x 3: nothing / half / ready on the bright pulse; ready
        // on the dim pulse, the flash frame, and a mid-fight frame; then the
        // same read on the two brighter themes and a dashing rat.
        let sheet = |w: usize, h: usize| -> Vec<u8> {
            let shots = [
                shot(0.0),
                shot(0.5),
                Shot {
                    elapsed: PULSE_UP,
                    ..shot(1.0)
                },
                Shot {
                    elapsed: PULSE_DOWN,
                    ..shot(1.0)
                },
                Shot {
                    elapsed: PULSE_UP,
                    prime: false,
                    ..shot(1.0)
                },
                Shot {
                    level: 3,
                    elapsed: PULSE_UP,
                    horde: true,
                    ..shot(1.0)
                },
                Shot {
                    level: 3,
                    elapsed: PULSE_UP,
                    horde: true,
                    theme: Theme::Kitchen,
                    ..shot(1.0)
                },
                Shot {
                    level: 3,
                    elapsed: PULSE_DOWN,
                    horde: true,
                    theme: Theme::Lab,
                    ..shot(1.0)
                },
                Shot {
                    level: 5,
                    elapsed: PULSE_UP,
                    horde: true,
                    dash: true,
                    charge: 0.7,
                    ..shot(1.0)
                },
            ];
            let panels: Vec<PixelBuffer> = shots.iter().map(|s| panel(w, h, s)).collect();
            let (cols, rows) = (3, 3);
            let sw = w * cols;
            let mut px = vec![0u8; sw * h * rows * 3];
            for (n, p) in panels.iter().enumerate() {
                let (ox, oy) = ((n % cols) * w, (n / cols) * h);
                for y in 0..h {
                    for x in 0..w {
                        let c = p.pixel_at(x, y);
                        let i = ((oy + y) * sw + ox + x) * 3;
                        px[i..i + 3].copy_from_slice(&[c.0, c.1, c.2]);
                    }
                }
            }
            let mut out = format!("P6\n{} {}\n255\n", sw, h * rows).into_bytes();
            out.extend_from_slice(&px);
            out
        };
        std::fs::write("/tmp/ratrun_tailcharge.ppm", sheet(112, 56)).unwrap();
        std::fs::write("/tmp/ratrun_tailclose.ppm", sheet(76, 46)).unwrap();
        // The same two sheets folded through the 16-colour terminal palette the
        // way `PixelBuffer::render_to` does, so the glow can be checked on the
        // worst colour depth the game supports.
        let fold = |mut ppm: Vec<u8>| -> Vec<u8> {
            // The body starts after the header's third newline
            // ("P6\n<w> <h>\n255\n").
            let mut start = 0;
            for _ in 0..3 {
                start += ppm[start..].iter().position(|&b| b == b'\n').unwrap() + 1;
            }
            for px in ppm[start..].chunks_mut(3) {
                let c = crate::term::map_rgb_with(
                    crate::term::ColorDepth::Ansi16,
                    (px[0], px[1], px[2]),
                );
                let rgb = match c {
                    ratatui::style::Color::Black => (0, 0, 0),
                    ratatui::style::Color::Red => (128, 0, 0),
                    ratatui::style::Color::Green => (0, 128, 0),
                    ratatui::style::Color::Yellow => (128, 128, 0),
                    ratatui::style::Color::Blue => (0, 0, 128),
                    ratatui::style::Color::Magenta => (128, 0, 128),
                    ratatui::style::Color::Cyan => (0, 128, 128),
                    ratatui::style::Color::Gray => (192, 192, 192),
                    ratatui::style::Color::DarkGray => (128, 128, 128),
                    ratatui::style::Color::LightRed => (255, 0, 0),
                    ratatui::style::Color::LightGreen => (0, 255, 0),
                    ratatui::style::Color::LightYellow => (255, 255, 0),
                    ratatui::style::Color::LightBlue => (0, 0, 255),
                    ratatui::style::Color::LightMagenta => (255, 0, 255),
                    ratatui::style::Color::LightCyan => (0, 255, 255),
                    ratatui::style::Color::White => (255, 255, 255),
                    _ => (0, 0, 0),
                };
                px.copy_from_slice(&[rgb.0, rgb.1, rgb.2]);
            }
            ppm
        };
        std::fs::write("/tmp/ratrun_tailcharge16.ppm", fold(sheet(112, 56))).unwrap();
        std::fs::write("/tmp/ratrun_tailclose16.ppm", fold(sheet(76, 46))).unwrap();
    }

    /// Every pose at every angle, including off-buffer positions: must never
    /// panic or write outside the buffer.
    #[test]
    fn rat_draws_everywhere_without_panic() {
        let mut pb = PixelBuffer::new(40, 30);
        for i in 0..64 {
            let ang = i as f32 / 64.0 * std::f32::consts::TAU;
            for &(x, y) in &[(20.0, 15.0), (0.0, 0.0), (39.5, 29.5), (-30.0, 12.0), (500.0, 500.0)] {
                let mut p = rat(Vec2::new(x, y), ang);
                p.vel = Vec2::from_angle(ang + 0.7) * 58.0;
                p.dash_active = if i % 3 == 0 { 0.16 } else { 0.0 };
                p.hurt_flash = if i % 5 == 0 { 0.2 } else { 0.0 };
                p.iframes = if i % 2 == 0 { 0.5 } else { 0.0 };
                draw_rat(&mut pb, &p, i as f32 * 0.37, Vec2::ZERO);
            }
        }
    }

    /// The hurt flash and the blink must change what is drawn.
    #[test]
    fn hurt_and_blink_are_visible() {
        let render = |hurt: f32, iframes: f32| {
            let mut pb = PixelBuffer::new(32, 32);
            pb.clear((20, 30, 28));
            let mut p = rat(Vec2::new(16.0, 16.0), 0.3);
            p.hurt_flash = hurt;
            p.iframes = iframes;
            draw_rat(&mut pb, &p, 0.0, Vec2::ZERO);
            pb.pixel_at(16, 16)
        };
        let normal = render(0.0, 0.0);
        let hurt = render(0.2, 0.0);
        let blink = render(0.0, 0.5);
        assert!(hurt.0 > hurt.1 + 60, "hurt should be red: {hurt:?}");
        assert_ne!(normal, blink);
    }

    /// Pose sheet: `RATRUN_DUMP=rat cargo test dump_rat_sheet` writes
    /// `/tmp/ratrun_rat.ppm` — each theme's floor with rows of idle / run /
    /// turn / dash / hurt / blink across 16 headings.
    #[test]
    fn dump_rat_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("rat") {
            return;
        }
        const CELL: usize = 26;
        const COLS: usize = 16;
        const ROWS: usize = 6;
        let (w, h) = (CELL * COLS, CELL * ROWS);
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();
        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let level = Level::new(theme);
            let cam = Vec2::new(300.0, 300.0);
            let mut pb = PixelBuffer::new(w, h);
            crate::game::art_floor::draw_floor(&mut pb, &level, cam, 0.0);
            for row in 0..ROWS {
                for col in 0..COLS {
                    let ang = col as f32 / COLS as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let pos = cam
                        + Vec2::new(
                            (col * CELL + CELL / 2) as f32 + 0.3 * col as f32,
                            (row * CELL + CELL / 2) as f32 + 0.2 * col as f32,
                        );
                    let mut p = rat(pos, ang);
                    let run = p.face * 58.0;
                    let t = 0.4 + col as f32 * 0.11;
                    match row {
                        0 => {}
                        1 => p.vel = run,
                        2 => p.vel = Vec2::from_angle(ang - 0.9) * 58.0,
                        3 => {
                            p.vel = run * 3.2;
                            p.dash_active = 0.12;
                        }
                        4 => {
                            p.vel = run;
                            p.hurt_flash = 0.02 + 0.012 * col as f32;
                        }
                        _ => {
                            p.vel = run;
                            p.iframes = 0.5;
                        }
                    }
                    // Blink row: alternate the on/off beat per column.
                    let t = if row == 5 { col as f32 * 0.05 } else { t };
                    draw_rat(&mut pb, &p, t, cam);
                }
            }
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_rat.ppm", out).unwrap();
    }
}
