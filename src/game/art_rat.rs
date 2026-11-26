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
use crate::render::lighting::{rat_helmet_pen, rat_visor_pen};
use crate::render::palette::{mix, scale, Rgb};

use super::collision::TileMap;
use super::ecs::components::{BiteSnap, Lash, LashPhase, WhipCrack};
use super::ecs::constants::PLAYER_SUBSTEP;
use super::ecs::systems::player::{dash_duration_from, DASH_BOOST};
use super::ecs::systems::weapons_wave3b::super_dash_radius;
use super::level::Theme;
use super::player::Player;
use super::weapon::{
    WeaponKind, LASH_DRAG_GAP, LASH_RECOIL_AT, LASH_TOTAL, LASH_WINDUP, WHIP_ARC_HALF,
};

/// Visual outfit worn by the player's rat. Render-only: never changes
/// collision, speed, hurt radius, or any simulated number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RatOutfit {
    /// Ordinary lab rat on every non-Outfall floor.
    #[default]
    Standard,
    /// Classic brass diving helmet, rubber diving suit, flippers, and
    /// swimming animation on the Outfall.
    Diver,
    /// The same suit with its feet down inside a closed crab pot.
    CagedDiver,
}

impl RatOutfit {
    /// Outfit for `theme`: diving suit on [`Theme::Outfall`], standard lab rat
    /// everywhere else.
    #[inline]
    pub const fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Outfall => Self::Diver,
            _ => Self::Standard,
        }
    }
}

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

// --- Outfall diving suit palette -------------------------------------------
// Classic copper-brass helmet with aqua porthole over a rubbery slate-teal
// diving suit and brass-trimmed flippers: warm brass contrasts cleanly against
// the Outfall's cool blue seabed and caustics.
const SUIT_SHADOW: Rgb = (52, 74, 88);
const SUIT_BASE: Rgb = (108, 138, 152);
const SUIT_LIGHT: Rgb = (176, 206, 214);
const SUIT_SEAM: Rgb = (34, 48, 60);
const BRASS_DEEP: Rgb = (116, 66, 22);
const BRASS_SHADOW: Rgb = (170, 102, 32);
const BRASS_BASE: Rgb = (224, 160, 58);
const BRASS_LIGHT: Rgb = (255, 220, 122);
const BRASS_SPEC: Rgb = (255, 248, 210);
const VISOR_DARK: Rgb = (30, 82, 106);
const VISOR_BASE: Rgb = (78, 174, 206);
const VISOR_LIGHT: Rgb = (168, 236, 248);
const VISOR_GLINT: Rgb = (242, 254, 255);
const FLIPPER_BASE: Rgb = (58, 78, 94);
const FLIPPER_TIP: Rgb = (218, 168, 56);
const TANK_BODY: Rgb = (214, 166, 58);
const TANK_SHADOW: Rgb = (138, 92, 28);
const HOSE_DARK: Rgb = (38, 48, 56);
const BUBBLE_RING: Rgb = (158, 224, 242);
const BUBBLE_SPEC: Rgb = (236, 252, 255);

/// Local spine station and radius of the round diving helmet (normalised rat units).
const HELMET_U: f32 = 3.2;
const HELMET_R: f32 = 2.30;

/// Base upward float altitude (world px at 1×) of the diving rat above its
/// true ground position on the Outfall seabed. Render-only.
pub const DIVER_LIFT: f32 = 5.0;

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
    /// True on the Outfall (`RatOutfit::Diver`): underwater swimming kinematics.
    diver: bool,
    /// True ground/hitbox world position (`player.pos`), where the detached
    /// seabed shadow stays anchored while the diver floats above it.
    ground: Vec2,
    /// Current screen-upward float altitude above `ground` (world px).
    lift: f32,
    /// Continuous analytic swim stroke clock (radians).
    swim_phase: f32,
    /// Continuous analytic idle hover clock (radians).
    hover_phase: f32,
}

impl Pose {
    /// Lateral offset of the spine at station `u`: a wave that runs down the
    /// body while scurrying, fading out toward the head so the eyes stay put.
    #[inline]
    fn spine(&self, u: f32) -> f32 {
        if self.diver {
            // Keep the rigid brass helmet steady while the rubber suit undulates behind it.
            let steady = ((2.4 - u) * 0.24).clamp(0.0, 1.0);
            let idle_w = 0.14 * (self.hover_phase - u * 0.55).sin() * (1.0 - self.moving);
            let swim_w = 0.58 * (self.swim_phase - u * 0.75).sin() * self.moving;
            (idle_w + swim_w) * steady * (1.0 - 0.85 * self.dash)
        } else {
            let steady = ((3.2 - u) * 0.22).clamp(0.0, 1.0);
            self.wig * steady * (self.phase - u * 0.7).sin()
        }
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

    #[inline]
    fn in_helmet(&self, u: f32, v: f32, margin: f32) -> bool {
        let r = HELMET_R + margin;
        let du = u - HELMET_U;
        du * du + v * v <= r * r
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
    if player.buffs.weapons_suspended() {
        return None;
    }
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
    white_coat: bool,
}

impl Tint {
    #[inline]
    fn apply(&self, c: Rgb) -> Rgb {
        let c = if self.white_coat {
            mix(c, (255, 255, 248), 0.58)
        } else if self.hurt > 0.0 {
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

/// Number of point masses in the render-only Spiked Tail Verlet whip chain.
pub const WHIP_NODES: usize = 16;

/// Fixed integration sub-step (s) for [`WhipChain`]. Divides `LASH_WINDUP`
/// (0.12 = 30 steps), `LASH_RECOIL_AT` (0.20 = 50 steps) and `LASH_TOTAL`
/// (0.34 = 85 steps) cleanly so the chain is frame-rate independent and
/// deterministic for single-frame captures.
const WHIP_FIXED_DT: f32 = 0.004;
const WHIP_CONSTRAINT_ITERS: usize = 8;
const WHIP_MAX_SUBSTEPS: usize = 110;
const WHIP_MAX_STEP_MOVE: f32 = 10.0;
const WHIP_SETTLE_SECS: f32 = 0.16;

/// Render-only position-based / Verlet chain simulation for **Spiked Tail**
/// (and its **Barbed Wire** evolution).
///
/// Lives on `World` alongside `tail_ready_at`: never touched by the simulation
/// schedule, never draws from `World::rng`, and uses fixed-size arrays (zero
/// per-frame allocation). Reset deterministically whenever a new `Lash` starts
/// (`SpawnId` changes), stepped with fixed sub-steps to `lash.elapsed()`, and
/// smoothly settled back into the resting tail curl once the lash ends.
#[derive(Clone, Debug)]
pub struct WhipChain {
    active_id: Option<u64>,
    active: bool,
    settling: bool,
    sim_time: f32,
    settle_t: f32,
    last_elapsed: f32,
    pos: [Vec2; WHIP_NODES],
    prev: [Vec2; WHIP_NODES],
    /// Two recent chain snapshots for supersonic whip-crack motion smear.
    ghost_a: [Vec2; WHIP_NODES],
    ghost_b: [Vec2; WHIP_NODES],
    ghost_count: u8,
    swing_side: f32,
    seg_len: f32,
    barbed: bool,
    bristle: f32,
    flash: f32,
    smear: f32,
    star: f32,
    shock_t: f32,
    radius: f32,
    point: Vec2,
    drag_progress: f32,
    catch_pos: Option<Vec2>,
}

impl Default for WhipChain {
    fn default() -> Self {
        Self::new()
    }
}

impl WhipChain {
    pub fn new() -> Self {
        Self {
            active_id: None,
            active: false,
            settling: false,
            sim_time: 0.0,
            settle_t: 0.0,
            last_elapsed: 0.0,
            pos: [Vec2::ZERO; WHIP_NODES],
            prev: [Vec2::ZERO; WHIP_NODES],
            ghost_a: [Vec2::ZERO; WHIP_NODES],
            ghost_b: [Vec2::ZERO; WHIP_NODES],
            ghost_count: 0,
            swing_side: 1.0,
            seg_len: 1.0,
            barbed: false,
            bristle: 0.0,
            flash: 0.0,
            smear: 0.0,
            star: 0.0,
            shock_t: 0.0,
            radius: 0.0,
            point: Vec2::ZERO,
            drag_progress: 0.0,
            catch_pos: None,
        }
    }

    /// True while a lash is in flight or the chain is settling back into the
    /// rat's normal idle tail curl.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// World position of the whip's tip node.
    #[inline]
    pub fn tip(&self) -> Vec2 {
        self.pos[WHIP_NODES - 1]
    }

    /// World position of the whip's midpoint node.
    #[inline]
    pub fn mid(&self) -> Vec2 {
        self.pos[WHIP_NODES / 2]
    }

    /// World positions of all simulated nodes (for tests & inspection).
    #[inline]
    pub fn nodes(&self) -> &[Vec2; WHIP_NODES] {
        &self.pos
    }

    /// Inverse mass for node `i`: node 0 is pinned (`0.0`), mass tapers down
    /// toward the tip so a travelling wave accelerates into the crack.
    #[inline]
    fn inv_mass(i: usize) -> f32 {
        if i == 0 {
            return 0.0;
        }
        let u = i as f32 / (WHIP_NODES - 1) as f32;
        let mass = (1.0 - 0.85 * u).powf(1.8).max(0.05);
        1.0 / mass
    }

    /// Synchronise and sub-step the render-only Verlet chain against the live
    /// `Lash` component (if any). `catch_pos` is the live position of the
    /// caught body for an evolved Barbed Wire lash.
    pub fn sync(
        &mut self,
        player: &Player,
        elapsed: f32,
        lash: Option<(u64, &Lash, Option<Vec2>)>,
    ) {
        self.sync_with_outfit(player, elapsed, lash, RatOutfit::Standard);
    }

    /// [`WhipChain::sync`] with the active [`RatOutfit`] so the whip root and
    /// resting settle match the Outfall's swimming pose.
    pub fn sync_with_outfit(
        &mut self,
        player: &Player,
        elapsed: f32,
        lash: Option<(u64, &Lash, Option<Vec2>)>,
        outfit: RatOutfit,
    ) {
        let pose = pose_of_with_outfit(player, elapsed, outfit);
        let tail = tail_charge(player);
        let rest_len = tail.map_or(TAIL_LEN, |t| t.tail_len()) + 3.0 * pose.dash;
        let rest_pts = sample_rest_tail(&pose, player.vel, elapsed, rest_len);
        let rump = rest_pts[0];

        let Some((lash_id, lash, catch_pos)) = lash else {
            // No live lash: if we were active, transition into a short damped
            // settle back into the rat's procedural idle tail curl.
            if self.active {
                if self.active_id.is_some() {
                    self.active_id = None;
                    self.settling = true;
                    self.settle_t = 0.0;
                    self.flash = 0.0;
                    self.smear = 0.0;
                    self.star = 0.0;
                    self.shock_t = 0.0;
                    self.drag_progress = 0.0;
                    self.catch_pos = None;
                }
                let dt = if elapsed >= self.last_elapsed {
                    (elapsed - self.last_elapsed).clamp(0.0, 0.1)
                } else {
                    // Clock jumped backwards (reset/capture seek): snap clean.
                    self.active = false;
                    self.settling = false;
                    self.last_elapsed = elapsed;
                    return;
                };
                self.last_elapsed = elapsed;
                self.step_settle(dt, &rest_pts, rest_len);
            } else {
                self.last_elapsed = elapsed;
            }
            return;
        };

        let target = lash.tip();
        let to_target = target - rump;
        let dist = to_target.len().max(6.0);
        let dir = if to_target.len_sq() > 1e-4 {
            to_target * (1.0 / dist)
        } else {
            pose.face
        };

        // Reset deterministically whenever a brand-new lash begins (or if a
        // harness rewinds the lash clock).
        let lash_t = lash.elapsed().clamp(0.0, LASH_TOTAL);
        if self.active_id != Some(lash_id) || lash_t + 1e-4 < self.sim_time {
            self.active_id = Some(lash_id);
            self.active = true;
            self.settling = false;
            self.sim_time = 0.0;
            self.settle_t = 0.0;
            self.pos = rest_pts;
            self.prev = rest_pts;
            self.ghost_a = rest_pts;
            self.ghost_b = rest_pts;
            self.ghost_count = 0;
            self.seg_len = (rest_len / (WHIP_NODES - 1) as f32).max(0.5);
            // Pick the throw flank on the same side as the resting tail curl
            // relative to `dir`, so the loop swings cleanly AROUND the rat.
            let rest_tip_rel = rest_pts[WHIP_NODES - 1] - rump;
            let side_dot = rest_tip_rel.dot(dir.perp());
            self.swing_side = if side_dot.abs() > 0.25 {
                side_dot.signum()
            } else {
                let back_cross = (-pose.face).x * dir.y - (-pose.face).y * dir.x;
                if back_cross >= 0.0 { 1.0 } else { -1.0 }
            };
        }

        self.last_elapsed = elapsed;
        self.barbed = lash.barbed_wire();
        self.radius = lash.radius;
        self.point = target;
        self.catch_pos = catch_pos;

        // Snapshot previous frame positions for motion smear before stepping.
        let need_steps = lash_t > self.sim_time + WHIP_FIXED_DT * 0.45;
        if need_steps {
            self.ghost_b = self.ghost_a;
            self.ghost_a = self.pos;
            self.ghost_count = self.ghost_count.saturating_add(1).min(2);
        }

        // Fixed-dt sub-stepping: deterministic at any frame rate and even when
        // a capture harness jumps directly to a later tick without drawing in
        // between.
        let mut steps = 0usize;
        while self.sim_time + WHIP_FIXED_DT * 0.45 < lash_t && steps < WHIP_MAX_SUBSTEPS {
            self.sim_time = (self.sim_time + WHIP_FIXED_DT).min(LASH_TOTAL);
            self.step_lash_substep(self.sim_time, rump, target, dir, dist, rest_len, &rest_pts);
            // Refresh ghost snapshots during jump-ahead so a single-frame
            // crack capture still has crisp whip-speed smear behind it.
            if steps > 0 && steps.is_multiple_of(3) {
                self.ghost_b = self.ghost_a;
                self.ghost_a = self.pos;
                self.ghost_count = self.ghost_count.saturating_add(1).min(2);
            }
            steps += 1;
        }

        // Guarantee exact tip landing on the crack beat (even if `lash_t` is
        // right on `LASH_WINDUP` or `lash.cracked()` just became true).
        let phase = lash.phase();
        let pp = lash.phase_progress();
        if lash.cracked() && phase == LashPhase::Crack && pp < 0.45 {
            self.pos[0] = rump;
            self.pos[WHIP_NODES - 1] = target;
            self.solve_constraints(true);
        }

        // Update visual cues from the simulation's own lash clock so the white
        // crack flash, tiny tip shock ring, spikes and Barbed Wire drag never
        // drift from the hit tick.
        let total = LASH_TOTAL.max(1e-4);
        let t_norm = (lash_t / total).clamp(0.0, 1.0);
        let after = |window: f32| -> f32 {
            if !lash.cracked() && lash_t < LASH_WINDUP - 1e-4 {
                0.0
            } else {
                (1.0 - (lash_t - LASH_WINDUP).max(0.0) / window.max(1e-4)).clamp(0.0, 1.0)
            }
        };
        self.bristle = match phase {
            LashPhase::WindUp => (0.25 + 0.75 * pp).clamp(0.0, 1.0),
            LashPhase::Crack => 1.0,
            LashPhase::Recoil => {
                if self.barbed {
                    (1.0 - pp * 0.35).clamp(0.0, 1.0)
                } else {
                    (1.0 - (t_norm - 0.80).max(0.0) / 0.20).clamp(0.0, 1.0)
                }
            }
            LashPhase::Over => 0.0,
        };
        self.flash = after(0.10);
        self.shock_t = if lash.cracked() || lash_t >= LASH_WINDUP - 1e-4 {
            ((lash_t - LASH_WINDUP).max(0.0) / 0.11).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.star = after(0.16);
        self.smear = if phase == LashPhase::WindUp && pp > 0.65 {
            ((pp - 0.65) / 0.35).clamp(0.0, 1.0) * 0.85
        } else {
            after(0.075)
        };
        self.drag_progress = if self.barbed && phase == LashPhase::Recoil {
            pp
        } else {
            0.0
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn step_lash_substep(
        &mut self,
        t: f32,
        rump: Vec2,
        target: Vec2,
        dir: Vec2,
        dist: f32,
        rest_len: f32,
        rest_pts: &[Vec2; WHIP_NODES],
    ) {
        let dt = WHIP_FIXED_DT;
        let dt2 = dt * dt;
        let segs = (WHIP_NODES - 1) as f32;
        let flank = dir.perp() * self.swing_side;
        let smooth = |x: f32| {
            let u = x.clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };

        let in_windup = t < LASH_WINDUP;
        let in_crack = (LASH_WINDUP..LASH_RECOIL_AT).contains(&t);
        let wu = (t / LASH_WINDUP.max(1e-4)).clamp(0.0, 1.0);
        let cu = ((t - LASH_WINDUP) / (LASH_RECOIL_AT - LASH_WINDUP).max(1e-4)).clamp(0.0, 1.0);
        let ru = ((t - LASH_RECOIL_AT) / (LASH_TOTAL - LASH_RECOIL_AT).max(1e-4)).clamp(0.0, 1.0);

        // Segment length: unfurls with slight loop slack during wind-up, snaps
        // taut to exact target distance at crack, then reels back in recoil.
        let chain_len = if in_windup {
            let grow = smooth(wu / 0.52);
            let slack = 1.0 + 0.12 * (1.0 - smooth((wu - 0.58) / 0.42));
            rest_len + (dist * slack - rest_len) * grow
        } else if in_crack {
            dist
        } else if self.barbed {
            // Barbed Wire reels the caught target smoothly back toward the rat.
            let reel = smooth(ru / 0.88);
            let end_len = (LASH_DRAG_GAP + 3.0).max(rest_len * 0.85).min(dist);
            dist + (end_len - dist) * reel
        } else {
            let retract = smooth(ru);
            dist + (rest_len - dist) * retract
        };
        self.seg_len = (chain_len / segs).max(0.5);

        // Handle (node 0) throw motion: cocks back/outward, throws hard forward
        // toward target and checks back on the rump to launch a travelling loop.
        let handle = if in_windup {
            if wu < 0.42 {
                let c = (wu / 0.42 * std::f32::consts::FRAC_PI_2).sin();
                rump - dir * (2.2 * c) + flank * (3.4 * c)
            } else if wu < 0.74 {
                let th = smooth((wu - 0.42) / 0.32);
                let cock = -dir * 2.2 + flank * 3.4;
                let fwd = dir * 2.6 - flank * 1.4;
                rump + cock + (fwd - cock) * th
            } else {
                let chk = smooth((wu - 0.74) / 0.26);
                let fwd = dir * 2.6 - flank * 1.4;
                rump + fwd * (1.0 - chk)
            }
        } else {
            rump
        };

        let damp = if in_windup {
            0.986
        } else if in_crack {
            0.972
        } else {
            0.952
        };

        // Verlet integration for nodes 1..WHIP_NODES.
        for i in 1..WHIP_NODES {
            let u = i as f32 / segs;
            let mut vel = (self.pos[i] - self.prev[i]) * damp;
            if !vel.x.is_finite() || !vel.y.is_finite() {
                vel = Vec2::ZERO;
            }
            vel = vel.clamp_len(WHIP_MAX_STEP_MOVE);

            let mut acc = Vec2::ZERO;
            if in_windup {
                if wu < 0.44 {
                    // Cock the chain out onto the throw flank to load a wide loop.
                    let w = (1.0 - 0.55 * u).max(0.25);
                    acc += (-dir * 4200.0 + flank * 12500.0) * w;
                } else {
                    // Snap the proximal base straight toward the target while
                    // the distal loop rolls forward and accelerates!
                    let th = ((wu - 0.44) / 0.56).clamp(0.0, 1.0);
                    let base_drive = (1.0 - u / 0.55).clamp(0.0, 1.0) * (1.0 - th * 0.5);
                    acc += (dir * 22000.0 - flank * 7500.0) * base_drive;
                    // Subtle travelling-wave kick peaking as the wave passes `u`.
                    let wave_front = 0.25 + 0.85 * th;
                    let wave_env = (1.0 - ((u - wave_front) / 0.22).abs()).clamp(0.0, 1.0);
                    acc += dir * (11000.0 * wave_env * u);
                }
            } else if in_crack {
                if cu >= 0.45 {
                    // Organic rebound off the impact after the brief taut crack hold.
                    acc += (-dir * 2600.0 - flank * 3200.0) * u;
                }
            } else if self.barbed {
                // Barbed Wire: stay tensioned along the reel axis with gritty pull.
                let reel = smooth(ru / 0.88);
                let end_len = (LASH_DRAG_GAP + 3.0).max(rest_len * 0.85).min(dist);
                let cur_len = dist + (end_len - dist) * reel;
                let jitter = (ru * 26.0 + u * 5.0).sin() * 1.1 * (1.0 - ru) * u;
                let goal = rump + dir * (cur_len * u) + flank * jitter;
                acc += (goal - self.pos[i]) * 2600.0;
            } else {
                // Smooth recoil back into the rat's live resting tail curl.
                let spring = 900.0 + 3800.0 * ru * ru;
                acc += (rest_pts[i] - self.pos[i]) * spring;
            }

            let next = self.pos[i] + vel + acc * dt2;
            self.prev[i] = self.pos[i];
            self.pos[i] = if next.x.is_finite() && next.y.is_finite() {
                next
            } else {
                rump + dir * (self.seg_len * i as f32)
            };
        }

        self.pos[0] = handle;
        self.prev[0] = handle;

        // Distance constraint satisfaction (symmetric Gauss-Seidel passes).
        let pin_tip = in_crack && cu < 0.45;
        if pin_tip {
            self.pos[WHIP_NODES - 1] = target;
            self.prev[WHIP_NODES - 1] = target;
        } else if self.barbed && !in_windup && !in_crack {
            let reel = smooth(ru / 0.88);
            let end_len = (LASH_DRAG_GAP + 3.0).max(rest_len * 0.85).min(dist);
            let reel_pt = rump + dir * (dist + (end_len - dist) * reel);
            self.pos[WHIP_NODES - 1] =
                self.pos[WHIP_NODES - 1] + (reel_pt - self.pos[WHIP_NODES - 1]) * 0.65;
        }

        self.solve_constraints(pin_tip);

        // Final-frames target arrival blend (last ~3 frames of wind-up) so the
        // whipping tip NEVER visibly misses the simulation's crack target!
        if in_windup && wu > 0.70 {
            let k = ((wu - 0.70) / 0.30).clamp(0.0, 1.0);
            let pull = k * k * 0.62;
            let tip_i = WHIP_NODES - 1;
            self.pos[tip_i] = self.pos[tip_i] + (target - self.pos[tip_i]) * pull;
            if wu >= 0.96 {
                self.pos[tip_i] = target;
            }
            self.solve_constraints(wu >= 0.96);
        }
    }

    fn step_settle(&mut self, dt: f32, rest_pts: &[Vec2; WHIP_NODES], rest_len: f32) {
        if dt <= 0.0 {
            return;
        }
        let segs = (WHIP_NODES - 1) as f32;
        self.seg_len = (rest_len / segs).max(0.5);
        let dt2 = WHIP_FIXED_DT * WHIP_FIXED_DT;
        let steps = ((dt / WHIP_FIXED_DT).round() as usize).clamp(1, 32);
        for _ in 0..steps {
            self.settle_t += WHIP_FIXED_DT;
            let u = (self.settle_t / WHIP_SETTLE_SECS).clamp(0.0, 1.0);
            for i in 1..WHIP_NODES {
                let vel = ((self.pos[i] - self.prev[i]) * 0.90).clamp_len(WHIP_MAX_STEP_MOVE);
                let acc = (rest_pts[i] - self.pos[i]) * (2200.0 + 4800.0 * u);
                let next = self.pos[i] + vel + acc * dt2;
                self.prev[i] = self.pos[i];
                self.pos[i] = next + (rest_pts[i] - next) * (u * u * 0.35);
            }
            self.pos[0] = rest_pts[0];
            self.prev[0] = rest_pts[0];
            self.solve_constraints(false);
        }
        if self.settle_t >= WHIP_SETTLE_SECS {
            self.active = false;
            self.settling = false;
        }
    }

    fn solve_constraints(&mut self, pin_tip: bool) {
        let rest = self.seg_len;
        let last_seg = WHIP_NODES - 1;
        for iter in 0..WHIP_CONSTRAINT_ITERS {
            let forward = iter % 2 == 0;
            for k in 0..last_seg {
                let i = if forward { k } else { last_seg - 1 - k };
                let j = i + 1;
                let a = self.pos[i];
                let b = self.pos[j];
                let delta = b - a;
                let d_len = delta.len();
                if d_len <= 1e-4 {
                    continue;
                }
                let w_a = Self::inv_mass(i);
                let w_b = if pin_tip && j == last_seg {
                    0.0
                } else {
                    Self::inv_mass(j)
                };
                let w_sum = w_a + w_b;
                if w_sum <= 1e-6 {
                    continue;
                }
                let diff = (d_len - rest) / d_len;
                let corr = delta * diff;
                if w_a > 0.0 {
                    self.pos[i] += corr * (w_a / w_sum);
                }
                if w_b > 0.0 {
                    self.pos[j] -= corr * (w_b / w_sum);
                }
            }
        }
    }
}

/// Sample `WHIP_NODES` world-space points along the rat's procedural resting /
/// running tail curl (matching `draw_tail` so whip reset and settle are seamless).
fn sample_rest_tail(pose: &Pose, vel: Vec2, elapsed: f32, rest_len: f32) -> [Vec2; WHIP_NODES] {
    let m = pose.moving;
    let lag = if vel.len_sq() > 4.0 {
        let vd = vel.normalized();
        let cross = pose.face.x * vd.y - pose.face.y * vd.x;
        (cross.atan2(pose.face.dot(vd)) * 1.5).clamp(-2.0, 2.0)
    } else {
        0.0
    };
    let whip = 1.0 - 0.85 * pose.dash;
    let back = pose.face * -1.0;
    let step = rest_len / (WHIP_NODES - 1) as f32;
    let mut out = [Vec2::ZERO; WHIP_NODES];
    let mut p = pose.place(-3.2, 0.0);
    out[0] = p;
    for (i, slot) in out.iter_mut().enumerate().skip(1) {
        let s = (i as f32 - 0.5) / (WHIP_NODES - 1) as f32;
        let (idle, run) = if pose.diver {
            (
                0.35 * s + 0.52 * s * (elapsed * 2.6 - s * 3.4).sin(),
                0.10 * s + 0.85 * s * (elapsed * 8.5 - s * 4.8).sin() * whip,
            )
        } else {
            (
                1.75 * s.powf(1.3) + 0.24 * (elapsed * 1.7 - s * 2.2).sin(),
                0.22 * s + 0.6 * s * (elapsed * 13.0 - s * 5.5).sin() * whip,
            )
        };
        let theta = lag * s + idle * (1.0 - m) + run * m;
        p += back.rotate(theta) * step;
        *slot = p;
    }
    out
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
    draw_rat_with_whip(pb, player, elapsed, cam, tail_flash, lash, None);
}

/// [`draw_rat_lashed`], driven by the render-only physics [`WhipChain`] when a
/// **Spiked Tail** / **Barbed Wire** lash is active or settling back to idle.
pub fn draw_rat_with_whip(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    tail_flash: f32,
    lash: Option<LashVisual>,
    whip: Option<&WhipChain>,
) {
    draw_rat_with_outfit(
        pb,
        player,
        elapsed,
        cam,
        tail_flash,
        lash,
        whip,
        RatOutfit::Standard,
    );
}

/// [`draw_rat_with_whip`], selecting the rat's outfit (`RatOutfit::Diver` on
/// the Outfall, `RatOutfit::Standard` everywhere else). Render-only.
#[allow(clippy::too_many_arguments)]
pub fn draw_rat_with_outfit(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    tail_flash: f32,
    lash: Option<LashVisual>,
    whip: Option<&WhipChain>,
    outfit: RatOutfit,
) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = pose_of_with_outfit(player, elapsed, outfit);
    let tint = tint_of(player, elapsed);

    if pose.dash > 0.0 {
        draw_dash_streaks(pb, &pose, cam);
    }
    let mut tail = tail_charge(player);
    if let Some(t) = tail.as_mut() {
        t.flash = tail_flash.clamp(0.0, 1.0);
    }
    if pose.diver {
        if !tint.ghost {
            draw_diver_shadow(pb, &pose, tint, cam);
        }
        draw_tail(pb, &pose, player.vel, elapsed, tint, cam, tail, lash, whip);
        draw_diver_limbs(pb, &pose, tint, cam);
        draw_diver_body(pb, &pose, tint, cam);
        draw_diver_face(pb, &pose, elapsed, tint, cam);
        draw_diver_bubbles(pb, &pose, player.vel, elapsed, tint, cam);
    } else {
        draw_tail(pb, &pose, player.vel, elapsed, tint, cam, tail, lash, whip);
        draw_feet(pb, &pose, tint, cam);
        draw_body(pb, &pose, tint, cam);
        draw_face(pb, &pose, elapsed, tint, cam);
    }
    // Under the rat, so it never hides the body: the Super Dash charge ring
    // (nothing at all unless a charge is being held).
    draw_dash_charge(pb, player, elapsed, cam);
}

/// The live Spiked Tail chain alone, at its ordinary size: the Growth Serum's
/// giant draws its body through a small offscreen window the 120 px lash
/// cannot live in, so `World::draw_giant_rat` draws the lash with this.
pub fn draw_live_whip(
    pb: &mut PixelBuffer,
    player: &Player,
    elapsed: f32,
    cam: Vec2,
    tail_flash: f32,
    whip: &WhipChain,
) {
    if !whip.is_active() {
        return;
    }
    let tint = tint_of(player, elapsed);
    let mut tail = tail_charge(player);
    if let Some(t) = tail.as_mut() {
        t.flash = tail_flash.clamp(0.0, 1.0);
    }
    draw_whip_chain(pb, elapsed, tint, cam, tail, whip);
}

/// Everything about this frame's pose, derived once (split out so the tests can
/// redraw one layer — the body mask for the tail's "never covers the rat"
/// check — from the same numbers the real pass uses).
fn pose_of(player: &Player, elapsed: f32) -> Pose {
    pose_of_with_outfit(player, elapsed, RatOutfit::Standard)
}

fn pose_of_with_outfit(player: &Player, elapsed: f32, outfit: RatOutfit) -> Pose {
    let raw_face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let speed = player.vel.len();
    let top = player.loadout.stats.move_speed.max(1.0);
    let moving = (speed / top).clamp(0.0, 1.0);
    let dash = (player.dash_active / player.dash_base_duration()).clamp(0.0, 1.0).sqrt();
    // The Super Dash charge pose: the rat coils (shorter and wider, the exact
    // opposite of the dash stretch) as the meter fills, so the release reads
    // as a spring let go. Zero unless the dash is being held, and a tap never
    // shows it (the meter has barely moved). Render-only.
    let coil = if player.dash_charging {
        player.dash_charge.clamp(0.0, 1.0) * (1.0 - dash)
    } else {
        0.0
    };
    if outfit != RatOutfit::Standard {
        let float = if outfit == RatOutfit::Diver { 1.0 } else { 0.0 };
        let hover_phase = elapsed * 2.5;
        let swim_phase = elapsed * 8.5;
        // Smooth analytic float lift + buoyancy bob and stroke surge
        // (render-only: never alters `player.pos` or the hitbox).
        let bob_y = hover_phase.sin() * (0.95 - 0.35 * moving) * (1.0 - 0.65 * dash);
        let bob_x = (elapsed * 1.3 + 0.7).sin() * 0.25 * (1.0 - moving) * (1.0 - dash);
        let surge = swim_phase.sin() * 0.35 * moving * (1.0 - dash);
        let lift = (DIVER_LIFT - bob_y) * float;
        let pos = player.pos + Vec2::new(bob_x * float, -lift) + raw_face * (surge * float);

        // Slight continuous tilt into the direction of travel plus gentle
        // swim-kick yaw.
        let turn_lean = if player.vel.len_sq() > 4.0 {
            let vd = player.vel.normalized();
            let cross = raw_face.x * vd.y - raw_face.y * vd.x;
            cross.atan2(raw_face.dot(vd)).clamp(-0.55, 0.55) * 0.22 * moving
        } else {
            0.0
        };
        let swim_yaw = 0.09 * moving * (1.0 - dash) * swim_phase.sin()
            + 0.045 * (1.0 - moving) * (hover_phase + 0.6).cos();
        let face = raw_face.rotate(turn_lean + swim_yaw);
        let perp = face.perp();
        return Pose {
            pos,
            face,
            perp,
            su: 1.0 + 0.06 * moving + 0.36 * dash - CHARGE_COIL_SQUASH * coil,
            sv: 1.0 - 0.04 * moving - 0.16 * dash + CHARGE_COIL_SPREAD * coil,
            phase: swim_phase,
            wig: (0.14 * (1.0 - moving) + 0.58 * moving) * (1.0 - 0.85 * dash),
            breathe: 0.12 * (1.0 - moving) * hover_phase.sin(),
            moving,
            dash,
            diver: true,
            ground: player.pos,
            lift,
            swim_phase,
            hover_phase,
        };
    }
    let perp = raw_face.perp();
    Pose {
        pos: player.pos,
        face: raw_face,
        perp,
        su: 1.0 + 0.32 * dash - CHARGE_COIL_SQUASH * coil,
        sv: 1.0 - 0.14 * dash + CHARGE_COIL_SPREAD * coil,
        // Keyed to distance along the facing: feet step with the ground and
        // stop dead when the rat does.
        phase: player.pos.dot(raw_face) * 1.15,
        wig: 0.45 * moving * (1.0 - dash),
        breathe: 0.18 * (1.0 - moving) * (elapsed * 3.2).sin(),
        moving,
        dash,
        diver: false,
        ground: player.pos,
        lift: 0.0,
        swim_phase: 0.0,
        hover_phase: 0.0,
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
        ghost: player.buffs.white_coat <= 0.0
            && player.iframes > 0.0 && ((elapsed * 20.0) as i32 % 2 == 0),
        white_coat: player.buffs.white_coat > 0.0,
    }
}

// ---- Dash charge ring -----------------------------------------------------

/// How far out the charge ring sits (px). Screen-space, so it does not spin
/// with the facing.
const GAUGE_RADIUS: f32 = 10.0;

/// The Super Dash **charge pose**: how much the body shortens along the facing
/// and widens across it at a full meter (see [`pose_of`]).
const CHARGE_COIL_SQUASH: f32 = 0.16;
const CHARGE_COIL_SPREAD: f32 = 0.12;

/// The Super Dash charge ring: how many pips it has and its colours — the
/// weapon's own blue-white, so "charging the dash" never reads as "the dash is
/// ready" (the HUD's amber module) or as a refusal (red).
const CHARGE_PIPS: i32 = 18;
const CHARGE_COLOR: Rgb = (198, 230, 255);
const CHARGE_FULL: Rgb = (255, 255, 255);
const CHARGE_SOCKET: Rgb = (46, 58, 84);

/// The preview's colour: the weapon's blue-white, drawn faint. Friendly
/// information, never a telegraph (those are red or amber and drawn on the
/// hostile layer).
const PREVIEW_COLOR: Rgb = (150, 196, 255);

/// The **Super Dash charge ring**, attached to the rat: while the dash is held
/// for a Super Dash the ring closes around it and fills clockwise from the top
/// in the weapon's own blue-white, pulsing at a full meter — the hold-to-charge
/// read, and the only on-rat dash read-out there is.
///
/// The cooldown pips that used to sit under the rat are **gone** (batch 12, the
/// owner's call): the HUD's dash module already carries the cooldown, the ready
/// flash and the refusal, and a second gauge in the corner of the eye under the
/// sprite was clutter the player had to look through. So a cooling, ready,
/// dashing or refused dash draws nothing here at all; the ring appears only
/// while a charge is being held.
///
/// Purely visual: no rng is drawn here, so the draw pass stays deterministic.
pub fn draw_dash_charge(pb: &mut PixelBuffer, player: &Player, elapsed: f32, cam: Vec2) {
    if !player.dash_charging {
        return;
    }
    let centre = w2s(player.pos, cam);
    let scale = player.buffs.draw_scale();
    draw_charge_ring(pb, centre, player.dash_charge, elapsed, scale);
}

/// The **Super Dash charge ring** itself: the on-rat read-out closes into a
/// circle that fills clockwise from the top as the meter fills, in the weapon's
/// own blue-white (never the cooldown's amber, never a refusal's red).
///
/// At a full meter the lit pips pulse, which is the "held long enough" cue in
/// the corner of the eye — the simulated meter keeps filling (it caps at 1.0)
/// and never fires on its own, so the pulse is information, not a countdown.
pub(super) fn draw_charge_ring(
    pb: &mut PixelBuffer,
    centre: (i32, i32),
    charge: f32,
    elapsed: f32,
    scale: f32,
) {
    let charge = charge.clamp(0.0, 1.0);
    let full = charge >= 1.0;
    let pulse = if full {
        0.62 + 0.38 * (elapsed * 9.0).sin().abs()
    } else {
        1.0
    };
    let radius = GAUGE_RADIUS * scale;
    let pips = (CHARGE_PIPS as f32 * scale).round() as i32;
    for i in 0..pips {
        let t = i as f32 / pips as f32;
        // Clockwise from the top, so the fill reads like every other meter.
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * t;
        let dir = Vec2::from_angle(a);
        let x = centre.0 + (dir.x * radius).round() as i32;
        let y = centre.1 + (dir.y * radius).round() as i32;
        if t < charge {
            let colour = if full { CHARGE_FULL } else { CHARGE_COLOR };
            blend(pb, x, y, colour, (0.45 + 0.55 * pulse).min(1.0));
            blend(pb, x, y + 1, colour, (0.32 + 0.42 * pulse).min(1.0));
            if full {
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    blend(pb, x + dx, y + dy, CHARGE_COLOR, 0.22 * pulse);
                }
            }
        } else {
            blend(pb, x, y, CHARGE_SOCKET, 0.7);
            blend(pb, x, y + 1, CHARGE_SOCKET, 0.45);
        }
    }
}

/// The **Super Dash lane preview**: a faint blue-white line from the rat along
/// its dash direction, at the current charge's steady-speed reach, stopped
/// by the first wall — so "where would this go" is answered before the release
/// that spends the dash.
///
/// Friendly information: the colour is the weapon's own blue-white at low
/// alpha, deliberately unlike the red/amber enemy telegraphs, and it is drawn
/// under the rat. Pure drawing from player state + the tile grid; no rng, so
/// the draw pass stays deterministic.
pub fn draw_dash_preview(pb: &mut PixelBuffer, player: &Player, tiles: &TileMap, cam: Vec2) {
    if !player.dash_charging {
        return;
    }
    let Some(weapon) = player.loadout.weapons.iter().find(|w| w.kind == WeaponKind::SuperDash)
    else {
        return;
    };
    let charge = player.dash_charge.clamp(0.0, 1.0);
    let scale = player.buffs.draw_scale();
    let sweep_radius = super_dash_radius(weapon.dash_radius(), charge, scale);
    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let speed = player.loadout.stats.move_speed
        * player.speed_mult.max(0.0)
        * player.buffs.move_speed_scale()
        * DASH_BOOST;
    let len = speed * dash_duration_from(player.dash_base_duration(), charge);
    let alpha = 0.15 + 0.27 * charge;
    let (radius, class) = player.movement_body(tiles);
    // Two-pixel steps: a lane that reads as a line without drawing the whole
    // corridor pixel by pixel every frame.
    let mut d = PLAYER_SUBSTEP;
    while d <= len {
        let p = player.pos + face * d;
        let resolved = tiles.resolve_circle(p, radius, class, Some(face));
        if resolved != p {
            // The wall is where the dash stops: mark it faintly and stop.
            let s = w2s(p, cam);
            blend(pb, s.0, s.1, PREVIEW_COLOR, alpha + 0.18);
            break;
        }
        if d >= 8.0 * scale {
            super::art_wave3a::draw_dash_lane_slice(
                pb, p - cam, face.perp(), sweep_radius, PREVIEW_COLOR, alpha,
            );
        }
        d += PLAYER_SUBSTEP;
    }
}

/// One frame of the **Laser Eyes** read, from `World::laser_visual`:
/// where the beams start, which way each one points, how long each is after its
/// own wall stop, and how bright the bars and the muzzle dots are right now.
#[derive(Clone, Copy, Debug)]
pub struct LaserBeam {
    /// World position the beams leave from (the rat's eye line).
    pub origin: Vec2,
    /// World angle of the first beam (it starts at `aim - half` and closes
    /// inward on the aim line), in radians.
    pub angle: f32,
    /// Its length in px, already stopped by a wall.
    pub len: f32,
    /// The second beam's angle and length: the mirror of the first across the
    /// aim line, closing inward from the other edge. Both beams exist at every
    /// rank.
    pub angle_b: f32,
    pub len_b: f32,
    /// The aim line the pair closes on (and crosses, from L4).
    pub aim: f32,
    /// 0..1: the bars are lit for the whole sweep and fade at its very end.
    pub brightness: f32,
    /// 0..1: the twin muzzle dots stay on for the whole sweep, dimmed when the
    /// bars fade so the source is never lost.
    pub muzzle: f32,
}

/// The angular gap (radians) inside which the two beams read as **meeting**: a
/// brief hot flash is drawn where they coincide on the aim line. At L1–L3 they
/// land there as the sweep ends; at L4–L5 they cross it at the halfway mark.
const LASER_MEET_ARC: f32 = 0.30;
/// Where on the aim line the meeting flash sits, as a fraction of the shorter
/// beam's length: far enough out to read as the pair closing, not a mark on the
/// rat's own eyes.
const LASER_MEET_AT: f32 = 0.55;

/// Paint the two eye dots over the rat, with a compact red halo.
pub fn draw_laser_muzzle(pb: &mut PixelBuffer, player: &Player, muzzle: f32, cam: Vec2) {
    let face = if player.face.len_sq() > 1e-4 {
        player.face.normalized()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let hot = crate::game::particle::vfx::LASER_CORE;
    let glow = crate::game::particle::vfx::LASER_GLOW;
    let center = player.pos + face * crate::game::weapon::LASER_EYE_OFFSET;
    for side in [-0.85, 0.85] {
        let (x, y) = w2s(center + face.perp() * side, cam);
        pb.plot(x, y, mix(glow, hot, 0.45 + 0.55 * muzzle));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            plot_add(pb, x + dx, y + dy, scale(glow, 0.35 * muzzle));
        }
    }
}

/// Draw the live pair as self-lit hairlines from the stopped simulation rays.
pub fn draw_laser_eyes(pb: &mut PixelBuffer, beam: LaserBeam, cam: Vec2) {
    let b = beam.brightness;
    for (angle, len) in [(beam.angle, beam.len), (beam.angle_b, beam.len_b)] {
        draw_laser_bar(pb, beam.origin, angle, len, b, cam);
    }

    // A compact flash where the pair meets (L1–L3) or crosses (L4–L5).
    let len = beam.len.min(beam.len_b);
    let sep = (beam.angle - beam.angle_b).abs();
    let flash = (1.0 - sep / LASER_MEET_ARC).clamp(0.0, 1.0) * b;
    if len > 1.0 && flash > 0.02 {
        let at = beam.origin + Vec2::from_angle(beam.aim) * (len * LASER_MEET_AT);
        let (x, y) = w2s(at, cam);
        let hot = crate::game::particle::vfx::LASER_CORE;
        blend(pb, x, y, hot, 0.92 * flash);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            plot_add(pb, x + dx, y + dy, scale(hot, 0.40 * flash));
        }
    }
}

/// One core pixel per step along the major axis, with a faint 1 px shoulder
/// on either side. Live and Carbon Copy beams use the same raster.
pub fn draw_laser_bar(pb: &mut PixelBuffer, origin: Vec2, angle: f32, len: f32, b: f32, cam: Vec2) {
    if b <= 0.02 || len <= 0.0 {
        return;
    }
    let hot = crate::game::particle::vfx::LASER_CORE;
    let glow = scale(crate::game::particle::vfx::LASER_GLOW, 0.18 * b);
    let (x, y) = w2s(origin, cam);
    let (end_x, end_y) = w2s(origin + Vec2::from_angle(angle) * len, cam);
    let (dx, dy) = (end_x - x, end_y - y);
    let steps = dx.abs().max(dy.abs());
    let inv_steps = 1.0 / steps.max(1) as f32;
    let (gx, gy) = if dx.abs() >= dy.abs() { (0, 1) } else { (1, 0) };
    for step in 0..=steps {
        let t = step as f32 * inv_steps;
        let px = x + (dx as f32 * t).round() as i32;
        let py = y + (dy as f32 * t).round() as i32;
        blend(pb, px, py, hot, b);
        plot_add(pb, px - gx, py - gy, glow);
        plot_add(pb, px + gx, py + gy, glow);
    }
}

/// The **Tail Thrash** full-circle spin: a broad pink sweep trail making a full
/// disc around the rat, with a hot rose-white leading blade and outer dust rim.
///
/// `visual` is `(angle, reach, brightness)` from `World::thrash_visual`: the
/// sweep angle in world radians, the disc's radius in px, and a 0..1 fade.
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
    let pink_hot: Rgb = (255, 242, 250);
    let pink_bright: Rgb = (255, 102, 184);
    let pink_deep: Rgb = (196, 44, 128);
    let dust_rim: Rgb = (204, 192, 178);

    // 1. Broad pink full-disc sweep trail spanning ~300° behind `angle` across
    //    concentric motion bands (cheap, zero alloc, crisp silhouette at 1x & 2x).
    let trail_span = 5.25f32;
    let r_min = (reach * 0.28).max(5.0);
    let bands = 6i32;
    for b in 0..bands {
        let bt = b as f32 / (bands - 1) as f32;
        let r = r_min + (reach - r_min) * bt;
        let is_outer = b == bands - 1;
        // Outer rings span wider and brighter so the full spinning disc reads clearly.
        let span = trail_span * (0.65 + 0.35 * bt);
        let steps = (r * span * 0.55).ceil().clamp(16.0, 160.0) as i32;
        let mut last = (i32::MIN, i32::MIN);
        for i in 0..=steps {
            let frac = i as f32 / steps as f32; // 0 = leading edge, 1 = tail end
            let a = angle - frac * span;
            let (sa, ca) = a.sin_cos();
            let px = origin.0 + (ca * r).round() as i32;
            let py = origin.1 + (sa * r).round() as i32;
            if (px, py) == last {
                continue;
            }
            last = (px, py);
            let trail_fade = (1.0 - frac).powf(1.35) * fade;
            let col = if frac < 0.14 {
                mix(pink_hot, pink_bright, frac / 0.14)
            } else {
                mix(pink_bright, pink_deep, ((frac - 0.14) / 0.86).clamp(0.0, 1.0))
            };
            let gain = if is_outer {
                (0.95 * trail_fade).clamp(0.0, 1.0)
            } else {
                ((0.36 + 0.34 * bt) * trail_fade).clamp(0.0, 0.88)
            };
            if is_outer && frac < 0.45 {
                blend(pb, px, py, col, (0.85 * gain).clamp(0.0, 0.95));
            } else {
                plot_add(pb, px, py, scale(col, gain));
            }
            // Faint dust kicked outward just outside the leading half of the outer rim.
            if is_outer && i % 4 == 0 && frac < 0.65 {
                let dx = origin.0 + (ca * (r + 2.4)).round() as i32;
                let dy = origin.1 + (sa * (r + 2.4)).round() as i32;
                plot_add(pb, dx, dy, scale(dust_rim, 0.38 * trail_fade));
            }
        }
    }

    // 2. Blazing rose-white leading tail blade cutting across the disc at `angle`.
    let steps = (reach * 1.25).ceil().max(2.0) as i32;
    let mut last = (i32::MIN, i32::MIN);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let r = 4.0 + (reach - 4.0) * t;
        let a = angle - t * 0.24;
        let p = Vec2::from_angle(a);
        let nx = -p.y;
        let ny = p.x;
        let bx = origin.0 + (p.x * r).round() as i32;
        let by = origin.1 + (p.y * r).round() as i32;
        if (bx, by) == last {
            continue;
        }
        last = (bx, by);
        let weight = fade * (0.45 + 0.55 * t);
        let core = mix(pink_bright, pink_hot, 0.35 + 0.65 * t);
        blend(pb, bx, by, core, (0.92 * weight).clamp(0.0, 1.0));
        for side in [-1.0f32, 1.0] {
            let sx = origin.0 + (p.x * r + nx * side * 1.2).round() as i32;
            let sy = origin.1 + (p.y * r + ny * side * 1.2).round() as i32;
            plot_add(pb, sx, sy, scale(pink_bright, 0.80 * weight));
        }
    }
}

/// **Gnaw** melee bite: two bone-white tooth crescents closing like a mouth in
/// front of the rat's nose, with a tiny bone-white snap spark at closure.
pub fn draw_gnaw_bite(pb: &mut PixelBuffer, snap: &BiteSnap, cam: Vec2) {
    let total = snap.total.max(1e-4);
    let progress = (1.0 - snap.t / total).clamp(0.0, 1.0);
    let fade = if progress < 0.72 {
        1.0
    } else {
        (1.0 - (progress - 0.72) / 0.28).clamp(0.0, 1.0)
    };
    if fade <= 0.02 {
        return;
    }
    let dir = if snap.dir.len_sq() > 1e-4 {
        snap.dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let perp = dir.perp();
    // Mouth centre sits just in front of the rat's snout.
    let center = snap.pos + dir * 4.5;

    // Jaw closes briskly from open (`5.2 px` half-gap) to shut (`0.4 px`) by t=0.52.
    let close = (progress / 0.52).clamp(0.0, 1.0);
    let ease = close * close;
    let gap = 5.2 * (1.0 - ease) + 0.4;

    let white: Rgb = (255, 255, 250);
    let bone: Rgb = (238, 224, 192);
    let outline: Rgb = (24, 18, 22);

    // Two opposing curved tooth crescents (upper `+perp`, lower `-perp`).
    for side in [-1.0f32, 1.0] {
        for step in -3..=3i32 {
            let u = step as f32 * 1.25;
            // Crescent curves outward at the corners (`step = ±3`) and closes inward at centre.
            let arch = (step.abs() as f32 / 3.0).powi(2) * 1.9;
            let v = side * (gap + arch);
            let p = center + dir * u + perp * v;
            let s = w2s(p, cam);
            // Dark outer rim behind the crescent so white teeth pop on bright floors.
            let rim = w2s(p + perp * (side * 1.2), cam);
            if rim != s {
                blend(pb, rim.0, rim.1, outline, 0.65 * fade);
            }
            let col = if step.abs() <= 1 { white } else { bone };
            blend(pb, s.0, s.1, col, (0.96 * fade).clamp(0.0, 1.0));
            // Three sharp inward-pointing white fangs on each jaw (`step = -2, 0, +2`).
            if step % 2 == 0 {
                let fang = w2s(p - perp * (side * 1.35), cam);
                blend(pb, fang.0, fang.1, white, (0.98 * fade).clamp(0.0, 1.0));
            }
        }
    }

    // Crisp tiny bone-white clack spark at the bite seam when the jaws snap shut.
    if (0.42..=0.88).contains(&progress) {
        let spark = (1.0 - ((progress - 0.56).abs() / 0.32)).clamp(0.0, 1.0);
        let sc = w2s(center + dir * 0.5, cam);
        pb.plot(sc.0, sc.1, white);
        for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
            plot_add(pb, sc.0 + dx, sc.1 + dy, scale(white, 0.85 * spark));
        }
    }
}

/// **Tail Whip** rear swipe: a quick pink crescent on the cast's recorded
/// centre line (`dir ± WHIP_ARC_HALF`), hot rose-white along the leading edge.
pub fn draw_tail_whip_crack(pb: &mut PixelBuffer, crack: &WhipCrack, cam: Vec2) {
    let total = crack.total.max(1e-4);
    let progress = (1.0 - crack.t / total).clamp(0.0, 1.0);
    let fade = (1.0 - progress * progress).clamp(0.0, 1.0);
    if fade <= 0.02 {
        return;
    }
    let span = crack.to - crack.from;
    let reach = span.len();
    if reach <= 1.0 {
        return;
    }
    let dir = span * (1.0 / reach);
    let center_ang = dir.angle();
    let half = WHIP_ARC_HALF;

    let pink_hot: Rgb = (255, 240, 250);
    let pink_bright: Rgb = (255, 108, 192);
    let pink_deep: Rgb = (196, 44, 128);
    let rim_dark: Rgb = (36, 14, 28);
    let dust: Rgb = (204, 192, 178);

    // Quick sweep from -half across to +half within the recorded rear arc.
    let lead_frac = (0.58 + 0.42 * (progress / 0.36).min(1.0)).clamp(0.0, 1.0);
    let tail_frac = ((progress - 0.34) / 0.66).clamp(0.0, 0.72);
    let a_start = center_ang - half + tail_frac * (2.0 * half);
    let a_end = center_ang - half + lead_frac * (2.0 * half);
    let arc_span = (a_end - a_start).max(0.05);

    // Crescent thickness: plumpest at the centre, tapering to crisp 1 px ends.
    let max_thick = (reach * 0.16).clamp(3.0, 5.5);
    let steps = (reach * arc_span * 1.15).ceil().clamp(14.0, 128.0) as i32;

    for i in 0..=steps {
        let local_t = i as f32 / steps as f32; // 0 = trailing end, 1 = leading edge
        let a = a_start + local_t * arc_span;
        let full_u = ((a - (center_ang - half)) / (2.0 * half)).clamp(0.0, 1.0);
        let belly = (full_u * std::f32::consts::PI).sin().powf(0.75);
        let radial = Vec2::from_angle(a);
        let lead_heat = local_t.powf(1.6);

        // Subtle dark outer edge so the pink crescent stays crisp on bright floors.
        let outer_rim = w2s(crack.from + radial * (reach + 1.1), cam);
        blend(pb, outer_rim.0, outer_rim.1, rim_dark, 0.36 * fade * (0.4 + 0.6 * belly));

        // Crescent body: radial layers from `reach` inward by `belly * max_thick`.
        let depth = (belly * max_thick).round().max(1.0) as i32;
        for d in 0..depth {
            let rf = reach - d as f32;
            if rf <= 4.0 {
                break;
            }
            let p = w2s(crack.from + radial * rf, cam);
            let inner_t = if depth > 1 {
                d as f32 / (depth - 1) as f32
            } else {
                0.0
            };
            let col = if d == 0 {
                mix(pink_bright, pink_hot, 0.35 + 0.65 * lead_heat)
            } else {
                mix(pink_bright, pink_deep, (inner_t * 0.85).clamp(0.0, 1.0))
            };
            let alpha = ((0.96 - inner_t * 0.38) * (0.45 + 0.55 * local_t) * fade).clamp(0.0, 1.0);
            if d == 0 {
                blend(pb, p.0, p.1, col, alpha);
                plot_add(pb, p.0, p.1, scale(pink_hot, 0.28 * lead_heat * fade));
            } else {
                blend(pb, p.0, p.1, col, alpha * 0.82);
            }
        }
    }

    // Crisp leading tip sparkle right at the crescent's cutting head.
    let head_pt = w2s(crack.from + Vec2::from_angle(a_end) * reach, cam);
    blend(pb, head_pt.0, head_pt.1, pink_hot, (0.95 * fade).clamp(0.0, 1.0));
    for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
        plot_add(pb, head_pt.0 + dx, head_pt.1 + dy, scale(pink_hot, 0.45 * fade));
    }

    // A few pale dust motes kicked off both ends of the crescent.
    for side in [-1.0f32, 1.0] {
        let horn_dir = Vec2::from_angle(center_ang + side * half);
        let drift = 1.5 + progress * 4.5;
        for &(r_ofs, fwd_ofs, gain) in &[
            (drift, 0.8f32, 0.68f32),
            (drift + 2.2, 2.6, 0.48),
            (drift - 1.2, 3.8, 0.38),
        ] {
            let mp = crack.from + horn_dir * (reach + r_ofs) + dir * fwd_ofs;
            let s = w2s(mp, cam);
            plot_add(pb, s.0, s.1, scale(dust, gain * fade));
        }
    }
}

/// Additive plot with bounds check (the arc can sweep past the viewport).
#[inline]
fn plot_add(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb) {
    pb.plot_add(x, y, c);
}

/// Speed lines peeling off the haunches during a dash (hydro-wake streaks underwater).
fn draw_dash_streaks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    if pose.diver {
        let prev_pen = pb.set_pen(0);
        for side in [-1.0f32, 1.0] {
            for j in 0..7 {
                let t = j as f32 / 7.0;
                let p = pose.place(-3.4 - j as f32 * 0.95, side * (2.0 - 0.7 * t));
                let s = w2s(p, cam);
                let f = pose.dash * (1.0 - t) * 0.85;
                pb.plot_add(s.0, s.1, scale((110, 200, 235), f));
            }
        }
        pb.set_pen(prev_pen);
        return;
    }
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
    whip: Option<&WhipChain>,
) {
    if let Some(chain) = whip.filter(|c| c.is_active()) {
        draw_whip_chain(pb, elapsed, tint, cam, tail, chain);
        return;
    }

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
        let (idle, run) = if pose.diver {
            (
                0.35 * s + 0.52 * s * (elapsed * 2.6 - s * 3.4).sin(),
                0.10 * s + 0.85 * s * (elapsed * 8.5 - s * 4.8).sin() * whip,
            )
        } else {
            (
                1.75 * s.powf(1.3) + 0.24 * (elapsed * 1.7 - s * 2.2).sin(),
                0.22 * s + 0.6 * s * (elapsed * 13.0 - s * 5.5).sin() * whip,
            )
        };
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
    // On the Outfall the diver floats above the seabed and casts one detached
    // ground shadow under `player.pos`, so the floating tail skips a mid-water shadow.
    if !tint.ghost && !pose.diver {
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

/// Draw the render-only physics-simulated [`WhipChain`] (Spiked Tail & evolved
/// Barbed Wire): long, thin rat tail carrying a real travelling wave, bristling
/// steel spikes (backward fishhook barbs + visible catch drag for Barbed Wire),
/// supersonic motion smear, and a crisp white crack flash + tiny shock ring at
/// the target tip.
fn draw_whip_chain(
    pb: &mut PixelBuffer,
    elapsed: f32,
    tint: Tint,
    cam: Vec2,
    tail: Option<TailCharge>,
    chain: &WhipChain,
) {
    let nodes = &chain.pos;
    let charge = tail.map_or(0.0, |t| t.charge);
    let ready = tail.is_some_and(|t| t.ready);
    let ready_flash = tail.map_or(0.0, |t| t.flash);
    let pulse = 0.5 + 0.5 * (elapsed * 6.2).sin();
    let a = tint.alpha();
    let segs = WHIP_NODES - 1;

    // 1. Drop shadow under the chain first so it never darkens the tail itself.
    if !tint.ghost {
        let mut last = (i32::MIN, i32::MIN);
        for i in 0..segs {
            let p0 = nodes[i];
            let p1 = nodes[i + 1];
            let steps = (p0.dist(p1) * 1.35).ceil().clamp(1.0, 24.0) as i32;
            for k in 0..=steps {
                let f = k as f32 / steps as f32;
                let s = w2s(p0 + (p1 - p0) * f, cam);
                if s != last {
                    darken(pb, s.0 + 1, s.1 + 1, 0.30);
                    last = s;
                }
            }
        }
    }

    // 2. Supersonic whip-crack smear: 1–2 faint ghost arcs of the distal half
    //    behind the live chain so even a single still frame reads as whip speed.
    if chain.smear > 0.05 && chain.ghost_count > 0 {
        let ghosts: [(&[Vec2; WHIP_NODES], f32); 2] = [
            (&chain.ghost_a, 0.38 * chain.smear),
            (&chain.ghost_b, 0.20 * chain.smear),
        ];
        for (gi, &(g_nodes, gain)) in ghosts.iter().enumerate() {
            if gi >= chain.ghost_count as usize || gain <= 0.03 {
                continue;
            }
            for i in (WHIP_NODES / 3)..segs {
                let p0 = g_nodes[i];
                let p1 = g_nodes[i + 1];
                let steps = (p0.dist(p1) * 0.9).ceil().clamp(1.0, 16.0) as i32;
                let mut last = (i32::MIN, i32::MIN);
                for k in 0..=steps {
                    let f = k as f32 / steps as f32;
                    let s = w2s(p0 + (p1 - p0) * f, cam);
                    if s != last {
                        let u = (i as f32 + f) / segs as f32;
                        plot_add(pb, s.0, s.1, scale(SPIKE_STEEL, gain * (0.4 + 0.6 * u)));
                        last = s;
                    }
                }
            }
        }
    }

    // 3. The rat's own physical tail chain: long & thin, rose-pink grading to
    //    bright tip (with optional Tail Thrash charge glow and Barbed Wire
    //    steel-wrap highlights), flashing hot white at the tip on the crack.
    let mut pixel_idx = 0u32;
    let mut last_px = (i32::MIN, i32::MIN);
    for i in 0..segs {
        let p0 = nodes[i];
        let p1 = nodes[i + 1];
        let seg_vec = p1 - p0;
        let seg_d = seg_vec.len();
        let steps = (seg_d * 1.45).ceil().clamp(1.0, 28.0) as i32;
        let normal = if seg_d > 1e-4 {
            (seg_vec * (1.0 / seg_d)).perp()
        } else {
            Vec2::new(0.0, 1.0)
        };
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            let s_norm = ((i as f32 + f) / segs as f32).clamp(0.0, 1.0);
            let wp = p0 + seg_vec * f;
            let sc = w2s(wp, cam);
            if sc == last_px {
                continue;
            }
            last_px = sc;
            pixel_idx = pixel_idx.wrapping_add(1);

            let mut col = mix(TAIL_BASE, TAIL_TIP, s_norm);
            col = mix(col, THRASH_GLOW, 0.5 * charge);
            if ready {
                let heat = ((0.15 + 0.55 * pulse + 0.45 * ready_flash)
                    * (0.15 + 0.85 * s_norm * s_norm))
                    .min(1.0);
                col = mix(col, THRASH_HOT, heat);
            }
            if chain.barbed && pixel_idx.is_multiple_of(3) {
                col = mix(col, SPIKE_STEEL, 0.72);
            }
            if chain.flash > 0.0 && s_norm > 0.65 {
                let tip_hot = ((s_norm - 0.65) / 0.35).clamp(0.0, 1.0) * chain.flash;
                col = mix(col, SPIKE_HOT, tip_hot);
            }
            blend(pb, sc.0, sc.1, tint.apply(col), a);
            if chain.flash > 0.2 && s_norm > 0.75 {
                plot_add(pb, sc.0, sc.1, scale(SPIKE_HOT, 0.35 * chain.flash * a));
            }
            // Keep the whip long & thin: thicken only near the muscular base
            // (or when Thrash-charged) so it tapers to a razor-thin 1 px lash.
            if (s_norm < 0.28 && chain.bristle > 0.35) || (charge > 0.5 && s_norm < 0.45) {
                let nx = normal.x.round() as i32;
                let ny = normal.y.round() as i32;
                if (nx, ny) != (0, 0) {
                    let edge = if chain.barbed {
                        mix(col, SPIKE_STEEL, 0.4)
                    } else {
                        col
                    };
                    blend(pb, sc.0 + nx, sc.1 + ny, tint.apply(edge), a * 0.75);
                }
            }
        }
    }

    // 4. Steel spikes standing out along the chain while bristling / taut
    //    (for Barbed Wire: bilateral backward-angled fishhook barbs).
    if chain.bristle > 0.05 {
        for i in 2..segs {
            let s_norm = i as f32 / segs as f32;
            let tang = nodes[i + 1] - nodes[i - 1];
            if tang.len_sq() <= 1e-4 {
                continue;
            }
            let d = tang.normalized();
            let n = d.perp();
            let sc = w2s(nodes[i], cam);
            let spike_len = (1.0 + (0.9 + 2.1 * s_norm) * chain.bristle)
                .round()
                .clamp(1.0, 3.0) as i32;
            let sides: &[f32] = if chain.barbed {
                &[-1.0, 1.0]
            } else if i % 2 == 0 {
                &[1.0]
            } else {
                &[-1.0]
            };
            for &side in sides {
                let barb_dir = if chain.barbed {
                    // Angled backward toward the rump like real barbed wire.
                    (n * (side * 0.82) - d * 0.57).normalized()
                } else {
                    n * side
                };
                for k in 1..=spike_len {
                    let px = sc.0 + (barb_dir.x * k as f32).round() as i32;
                    let py = sc.1 + (barb_dir.y * k as f32).round() as i32;
                    let hot = k == spike_len;
                    let c = if hot { SPIKE_HOT } else { SPIKE_STEEL };
                    blend(pb, px, py, tint.apply(c), a);
                    if hot && !tint.ghost {
                        darken(pb, px + 1, py + 1, 0.22);
                    }
                }
            }
        }
    }

    // 5. Barbed Wire visible catch drag along the reeling chain during Recoil.
    if chain.barbed && chain.drag_progress > 0.0 {
        let reel_fade = (1.0 - chain.drag_progress * 0.75).clamp(0.25, 1.0);
        let tip_w = nodes[WHIP_NODES - 1];
        let ts = w2s(tip_w, cam);
        let tang = (nodes[WHIP_NODES - 1] - nodes[WHIP_NODES - 2]).normalized();
        let perp = tang.perp();
        // Hooked barbed collar + snag cluster riding the reeling tip home.
        blend(pb, ts.0, ts.1, SPIKE_HOT, 0.95 * reel_fade);
        for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
            blend(pb, ts.0 + dx, ts.1 + dy, SPIKE_STEEL, 0.90 * reel_fade);
        }
        for side in [-1.0f32, 1.0] {
            let hook = (perp * side * 1.8 - tang * 1.4).normalized();
            for k in 1..=2 {
                let hx = ts.0 + (hook.x * k as f32).round() as i32;
                let hy = ts.1 + (hook.y * k as f32).round() as i32;
                blend(pb, hx, hy, SPIKE_HOT, 0.88 * reel_fade);
            }
        }
        // Subtle tension glint back to the caught body if it is alive nearby.
        if let Some(cp) = chain.catch_pos {
            if cp.dist(tip_w) <= 18.0 {
                let cs = w2s(cp, cam);
                let steps = (tip_w.dist(cp) * 0.8).ceil().clamp(1.0, 12.0) as i32;
                for k in 1..steps {
                    let f = k as f32 / steps as f32;
                    let mx = ts.0 + ((cs.0 - ts.0) as f32 * f).round() as i32;
                    let my = ts.1 + ((cs.1 - ts.1) as f32 * f).round() as i32;
                    plot_add(pb, mx, my, scale(SPIKE_STEEL, 0.55 * reel_fade));
                }
            }
        }
    }

    // 6. White crack flash + tiny crisp shock ring at the tip (`chain.point`).
    let tip_s = w2s(chain.point, cam);
    if chain.flash > 0.02 {
        let g = chain.flash.clamp(0.0, 1.0) * a;
        let dark: Rgb = (24, 18, 28);
        for &(dx, dy) in &[(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            blend(pb, tip_s.0 + dx, tip_s.1 + dy, dark, 0.45 * g);
        }
        blend(pb, tip_s.0, tip_s.1, SPIKE_HOT, g);
        let ray_len = if g > 0.55 { 4 } else { 3 };
        for k in 1..=ray_len {
            let ra = ((1.0 - (k as f32 - 0.5) / (ray_len as f32 + 0.5)) * g).clamp(0.0, 1.0);
            let c = if k <= 2 { SPIKE_HOT } else { SPIKE_STEEL };
            for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
                blend(pb, tip_s.0 + dx * k, tip_s.1 + dy * k, c, ra);
            }
        }
        for k in 1..=(ray_len - 1).max(1) {
            let da = ((0.85 - k as f32 * 0.22).max(0.2) * g).clamp(0.0, 1.0);
            for &(dx, dy) in &[(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                plot_add(pb, tip_s.0 + dx * k, tip_s.1 + dy * k, scale(SPIKE_HOT, da));
            }
        }
    }
    // Tiny expanding white-steel shock ring right around the crack tip.
    if chain.shock_t > 0.0 && chain.shock_t < 1.0 {
        let rf = 2.0 + 5.5 * chain.shock_t;
        let fade = (1.0 - chain.shock_t).powf(1.2) * a;
        let seg_count = ((rf * 4.5).round() as i32).clamp(10, 36);
        let mut last = (i32::MIN, i32::MIN);
        for i in 0..seg_count {
            let ang = i as f32 / seg_count as f32 * std::f32::consts::TAU;
            let px = tip_s.0 + (ang.cos() * rf).round() as i32;
            let py = tip_s.1 + (ang.sin() * rf).round() as i32;
            if (px, py) == last {
                continue;
            }
            last = (px, py);
            let col = if chain.shock_t < 0.45 {
                SPIKE_HOT
            } else {
                SPIKE_STEEL
            };
            blend(pb, px, py, col, (0.85 * fade).clamp(0.0, 1.0));
        }
    }
    // Small burst of thorns + subtle dashed burst radius at `chain.point`
    // (the hit effect at the impact point, staying put while the whip recoils).
    if chain.star > 0.02 && chain.radius > 0.5 {
        let g = chain.star.clamp(0.0, 1.0);
        let r = chain.radius.max(1.5);
        for i in 0..6 {
            let ang = i as f32 * std::f32::consts::TAU / 6.0 + 0.26;
            for (k, fade) in [(1.0f32, 0.90f32), (0.62, 0.50)] {
                let reach = r * k;
                plot_add(
                    pb,
                    tip_s.0 + (ang.cos() * reach).round() as i32,
                    tip_s.1 + (ang.sin() * reach).round() as i32,
                    scale(SPIKE_HOT, g * fade),
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
                tip_s.0 + (ang.cos() * r).round() as i32,
                tip_s.1 + (ang.sin() * r).round() as i32,
                scale(SPIKE_STEEL, g * 0.65),
            );
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
            if cover <= 0.0 && dark < 0.5 {
                // The drop shadow alone is floor, not body (the lighting's
                // material pen): no rim or sheen on it.
                let body_pen = pb.set_pen(0);
                pb.plot(x, y, out);
                pb.set_pen(body_pen);
                continue;
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

// ---------------------------------------------------------------------------
// Outfall diving suit & swimming (1× scale)
// ---------------------------------------------------------------------------

/// Deterministic integer hash in `[0, 1]` for analytic helmet valve bubbles.
#[inline]
pub(super) fn bubble_hash(slot: u32, cycle: u32, salt: u32) -> f32 {
    let mut n = slot
        .wrapping_mul(374_761_393)
        .wrapping_add(cycle.wrapping_mul(668_265_263))
        .wrapping_add(salt.wrapping_mul(1_442_695_041));
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    n ^= n >> 16;
    (n & 0xFFFF) as f32 / 65535.0
}

/// Soft detached seabed shadow left on the floor under the rat's TRUE ground
/// position (`pose.ground`), shrinking and fading as the rat floats higher.
/// Stamped with pen `0` so the lighting pass treats it as seabed floor.
fn draw_diver_shadow(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let prev_pen = pb.set_pen(0);
    let g = pose.ground - cam;
    let h_frac = (pose.lift / DIVER_LIFT).clamp(0.70, 1.35);
    let r_scale = 1.0 / (0.80 + 0.20 * h_frac);
    let rx = 3.6 * r_scale;
    let ry = 2.2 * r_scale;
    let core = (0.32 / (0.70 + 0.30 * h_frac)).clamp(0.14, 0.36) * tint.alpha();

    let cx = g.x.round() as i32;
    let cy = g.y.round() as i32;
    let px_r = rx.ceil() as i32 + 1;
    let py_r = ry.ceil() as i32 + 1;
    let x0 = (cx - px_r).max(0);
    let x1 = (cx + px_r).min(pb.w as i32 - 1);
    let y0 = (cy - py_r).max(0);
    let y1 = (cy + py_r).min(pb.h as i32 - 1);
    if x0 <= x1 && y0 <= y1 {
        let inv_rx = 1.0 / rx.max(0.5);
        let inv_ry = 1.0 / ry.max(0.5);
        for y in y0..=y1 {
            let dy = (y as f32 - g.y) * inv_ry;
            let dy2 = dy * dy;
            if dy2 >= 1.0 {
                continue;
            }
            for x in x0..=x1 {
                let dx = (x as f32 - g.x) * inv_rx;
                let r2 = dx * dx + dy2;
                if r2 < 1.0 {
                    let fall = 1.0 - r2;
                    darken(pb, x, y, core * (0.35 + 0.65 * fall));
                }
            }
        }
    }
    pb.set_pen(prev_pen);
}

/// Paddling rubber front mitts and kicking brass-trimmed diving flippers.
fn draw_diver_limbs(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let a = tint.alpha();
    let m = pose.moving;
    let g = pose.dash;

    // 1. Front paddling mitts: gentle tread when idle, smooth stroke when
    //    swimming, tucked tight along the flanks during a kick-glide dash.
    for side in [-1.0f32, 1.0] {
        let ph_i = pose.hover_phase + side * 0.9;
        let ph_s = pose.swim_phase + if side > 0.0 { 0.0 } else { 0.85 };
        let u_idle = 0.65 + 0.32 * ph_i.sin();
        let v_idle = side * (2.05 + 0.30 * ph_i.cos());
        let u_swim = 0.50 + 0.90 * ph_s.cos();
        let v_swim = side * (2.15 + 0.68 * ph_s.sin().max(-0.25));
        let u = (u_idle * (1.0 - m) + u_swim * m) * (1.0 - g) + (-0.30) * g;
        let v = (v_idle * (1.0 - m) + v_swim * m) * (1.0 - g) + (side * 2.05) * g;
        let s = w2s(pose.place(u, v), cam);
        let root = w2s(pose.place(u * 0.5 + 0.3, v * 0.72), cam);
        blend(pb, root.0, root.1, tint.apply(SUIT_SHADOW), a);
        blend(pb, s.0, s.1, tint.apply(FLIPPER_BASE), a);
    }

    // 2. Hind webbed diving flippers: alternating flutter kick while swimming,
    //    slow hover tread when idle, swept back together in a kick-glide dash.
    for side in [-1.0f32, 1.0] {
        let off = if side > 0.0 { 0.0 } else { std::f32::consts::PI };
        let kick = (pose.hover_phase + off).sin() * (1.0 - m) + (pose.swim_phase + off).sin() * m;
        let u_root = -2.75 - 0.35 * g;
        let v_root = side * (1.75 - 0.45 * g + 0.18 * kick * (1.0 - g));
        let u_tip = -4.25 - 0.65 * m - 0.55 * g - 0.45 * kick * (1.0 - g);
        let v_tip = side * (2.10 - 0.72 * g + 0.55 * kick * (1.0 - g));

        let p0 = pose.place(u_root, v_root);
        let p1 = pose.place(
            (u_root + u_tip) * 0.5,
            (v_root + v_tip) * 0.5 + side * 0.25 * (1.0 - 0.4 * g),
        );
        let p2 = pose.place(u_tip, v_tip);
        let p2_edge = pose.place(u_tip + 0.25, v_tip - side * 0.55);

        let s0 = w2s(p0, cam);
        let s1 = w2s(p1, cam);
        let s2 = w2s(p2, cam);
        let s2e = w2s(p2_edge, cam);
        blend(pb, s0.0, s0.1, tint.apply(SUIT_SEAM), a);
        blend(pb, s1.0, s1.1, tint.apply(FLIPPER_BASE), a);
        blend(pb, s2e.0, s2e.1, tint.apply(FLIPPER_BASE), a);
        blend(pb, s2.0, s2.1, tint.apply(FLIPPER_TIP), a);
    }
}

/// Diving suit body, dorsal air tank + hose, brass neck corselet, round brass
/// diving helmet, and aqua glass porthole faceplate (with Metal and Glass
/// material pens so the lighting effects make the helmet and visor gleam).
fn draw_diver_body(pb: &mut PixelBuffer, pose: &Pose, tint: Tint, cam: Vec2) {
    let c = w2s(pose.pos, cam);
    let x0 = (c.0 - REACH).max(0);
    let x1 = (c.0 + REACH).min(pb.w as i32 - 1);
    let y0 = (c.1 - REACH).max(0);
    let y1 = (c.1 + REACH).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let body_pen = pb.pen();
    let helmet_pen = rat_helmet_pen(false);
    let visor_pen = rat_visor_pen(false);
    let a_all = tint.alpha();
    let rim_colour: Rgb = if tint.hurt > 0.0 { (90, 0, 10) } else { (8, 14, 22) };
    let light_u = pose.face.dot(LIGHT);
    let light_v = pose.perp.dot(LIGHT);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 + cam.x, y as f32 + cam.y) - pose.pos;
            if d.len_sq() > (REACH * REACH) as f32 {
                continue;
            }
            let (u, v) = pose.local(d);

            // 3x3 coverage: round brass diving helmet over rubber suit torso.
            let mut body = 0u32;
            let mut helmet = 0u32;
            for sy in -1..=1 {
                for sx in -1..=1 {
                    let o = Vec2::new(sx as f32 * 0.33, sy as f32 * 0.33);
                    let (su, sv) = pose.local(d + o);
                    if pose.in_helmet(su, sv, 0.0) {
                        helmet += 1;
                    } else if pose.in_body(su, sv, 0.0) {
                        body += 1;
                    }
                }
            }
            let total = body + helmet;
            let cover = ((total as f32 / 9.0 - 0.18) / 0.5).clamp(0.0, 1.0);

            // Crisp dark silhouette rim only: the seabed ground shadow is drawn
            // detached under `pose.ground` (`draw_diver_shadow`).
            let dark = if cover < 1.0 && (pose.in_body(u, v, 0.95) || pose.in_helmet(u, v, 0.90)) {
                0.64
            } else {
                0.0
            };
            if cover <= 0.0 && dark <= 0.0 {
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = mix(bg, rim_colour, dark * a_all);
            let mut px_pen = body_pen;

            if cover > 0.0 {
                let colour = if helmet * 2 >= total && helmet > 0 {
                    // Front-facing oval porthole faceplate inside the round brass dome.
                    let vu = (u - 4.25) / 0.95;
                    let vv = v / 1.30;
                    let vp = vu * vu + vv * vv;
                    if vp <= 1.0 {
                        px_pen = visor_pen;
                        let v_lit = (0.5 + 0.5 * (vu * light_u + vv * light_v)).clamp(0.0, 1.0);
                        let mut c = if v_lit < 0.55 {
                            mix(VISOR_DARK, VISOR_BASE, v_lit / 0.55)
                        } else {
                            mix(VISOR_BASE, VISOR_LIGHT, (v_lit - 0.55) / 0.45)
                        };
                        if vp > 0.78 {
                            c = mix(c, VISOR_DARK, 0.45);
                        }
                        c
                    } else {
                        px_pen = helmet_pen;
                        let hu = (u - HELMET_U) / HELMET_R;
                        let hv = v / HELMET_R;
                        let r2 = (hu * hu + hv * hv).min(1.0);
                        let hz = (1.0 - r2).sqrt();
                        let lit = (0.42 + 0.42 * hz + 0.58 * (hu * light_u + hv * light_v))
                            .clamp(0.0, 1.25);
                        let mut c = if lit < 0.35 {
                            mix(BRASS_DEEP, BRASS_SHADOW, lit / 0.35)
                        } else if lit < 0.75 {
                            mix(BRASS_SHADOW, BRASS_BASE, (lit - 0.35) / 0.40)
                        } else if lit < 1.04 {
                            mix(BRASS_BASE, BRASS_LIGHT, (lit - 0.75) / 0.29)
                        } else {
                            mix(BRASS_LIGHT, BRASS_SPEC, ((lit - 1.04) / 0.21).clamp(0.0, 1.0))
                        };
                        // Beveled copper rim framing the porthole.
                        if vp <= 1.55 {
                            c = mix(c, BRASS_DEEP, 0.32);
                        }
                        c
                    }
                } else {
                    let uc = u.clamp(U_TAIL + 0.3, U_NOSE - 0.3);
                    let w = half_width(uc).max(0.7);
                    let vs = v - pose.spine(uc);
                    let n = (vs / w).clamp(-1.0, 1.0);
                    let along = ((u - 0.6) / 4.4).clamp(-1.0, 1.0);
                    let dome = 1.0 - n * n;
                    let lit = n * light_v + 0.55 * along * along * along.signum() * light_u;
                    let lum = (0.48 + 0.34 * dome + 0.58 * lit).clamp(0.0, 1.15);

                    if (0.95..=1.68).contains(&u) && vs.abs() <= 1.85 {
                        // Brass neck corselet collar joining helmet and suit.
                        px_pen = helmet_pen;
                        let clit = (0.5 + 0.45 * n * light_v).clamp(0.0, 1.0);
                        if clit < 0.5 {
                            mix(BRASS_SHADOW, BRASS_BASE, clit * 2.0)
                        } else {
                            mix(BRASS_BASE, BRASS_LIGHT, (clit - 0.5) * 2.0)
                        }
                    } else if (-2.05..=0.35).contains(&u) && vs.abs() <= 1.05 {
                        // Compact brass/ochre diving air tank strapped on the back.
                        if (-0.95..=-0.48).contains(&u) {
                            SUIT_SEAM
                        } else {
                            px_pen = helmet_pen;
                            let tlit = (0.52 + 0.48 * (vs / 1.05) * light_v).clamp(0.0, 1.0);
                            if tlit < 0.55 {
                                mix(TANK_SHADOW, TANK_BODY, tlit / 0.55)
                            } else {
                                mix(TANK_BODY, BRASS_LIGHT, (tlit - 0.55) / 0.45)
                            }
                        }
                    } else if (0.25..=1.35).contains(&u)
                        && (-1.50..=-0.55).contains(&vs)
                        && ((u - 0.8) * 0.65 + (vs + 1.0)).abs() < 0.42
                    {
                        // Curved dark rubber air hose linking tank to helmet rear.
                        HOSE_DARK
                    } else if u <= -3.0 && vs.abs() <= 0.95 {
                        // Brass tail-gland cuff at the rump.
                        px_pen = helmet_pen;
                        mix(BRASS_SHADOW, BRASS_BASE, (0.5 + 0.4 * lit).clamp(0.0, 1.0))
                    } else {
                        // Rubbery slate-teal diving suit with subtle wet highlight & seam.
                        let mut c = if lum < 0.70 {
                            mix(SUIT_SHADOW, SUIT_BASE, lum / 0.70)
                        } else {
                            mix(SUIT_BASE, SUIT_LIGHT, (lum - 0.70) / 0.45)
                        };
                        if (-0.75..=-0.45).contains(&u) && vs.abs() > 1.05 {
                            c = mix(c, SUIT_SEAM, 0.55);
                        }
                        c
                    }
                };
                out = mix(out, tint.apply(colour), cover * a_all);
            } else if dark < 0.5 {
                // Buoyant seabed shadow alone is floor, not creature body.
                px_pen = 0;
            }
            pb.set_pen(px_pen);
            pb.plot(x, y, out);
        }
    }
    pb.set_pen(body_pen);
}

/// Rat eyes peeking through the diving helmet's porthole, glass highlight,
/// exhaust valve rivet, and gently swaying whiskers poking past the helmet.
fn draw_diver_face(pb: &mut PixelBuffer, pose: &Pose, elapsed: f32, tint: Tint, cam: Vec2) {
    let prev_pen = pb.pen();
    let a = tint.alpha();

    // Eyes visible inside the aqua porthole + crisp top-left glass glint.
    pb.set_pen(rat_visor_pen(false));
    for side in [-1.0f32, 1.0] {
        let e = w2s(pose.place(4.05, side * 0.62), cam);
        blend(pb, e.0, e.1, tint.apply(EYE), a);
    }
    let glint = w2s(pose.place(4.35, 0.0) + LIGHT * 0.65, cam);
    blend(pb, glint.0, glint.1, tint.apply(VISOR_GLINT), 0.78 * a);

    // Helmet exhaust valve on the rear-left dome (origin of the bubble stream).
    pb.set_pen(rat_helmet_pen(false));
    let valve = w2s(pose.place(1.85, -1.75), cam);
    blend(pb, valve.0, valve.1, tint.apply(BRASS_LIGHT), 0.85 * a);

    // Subtle whiskers poking out beside the helmet, drifting gently in the water.
    pb.set_pen(0);
    let sway = 0.28 * (elapsed * 4.5).sin();
    for side in [-1.0f32, 1.0] {
        for (k, &(u, v)) in [(4.85f32, 2.0f32), (5.15, 2.95)].iter().enumerate() {
            let w = w2s(pose.place(u + sway * (k as f32 + 1.0) * 0.5, side * v), cam);
            let g = if k == 0 { 64 } else { 36 };
            pb.plot_add(w.0, w.1, (g, g, g));
        }
    }
    pb.set_pen(prev_pen);
}

/// Sparse analytic stream of tiny rising bubbles from the helmet valve (and a
/// short bubble burst during a dash). Zero rng, zero state, drawn pre-lighting
/// with pen `0` so the underwater scene tints and lights them naturally.
fn draw_diver_bubbles(
    pb: &mut PixelBuffer,
    pose: &Pose,
    vel: Vec2,
    elapsed: f32,
    tint: Tint,
    cam: Vec2,
) {
    let prev_pen = pb.set_pen(0);
    let valve = pose.place(1.85, -1.75);
    let alpha_mul = tint.alpha();

    for slot in 0..8u32 {
        let (weight, period) = if slot < 3 {
            (1.0, 1.15 + 0.55 * bubble_hash(slot, 0, 3))
        } else if slot < 5 {
            (
                ((pose.moving - 0.10) / 0.45).clamp(0.0, 1.0),
                0.78 + 0.35 * bubble_hash(slot, 0, 5),
            )
        } else {
            (
                (pose.dash * 1.4).clamp(0.0, 1.0),
                0.38 + 0.20 * bubble_hash(slot, 0, 7),
            )
        };
        if weight <= 0.02 {
            continue;
        }
        let offset = bubble_hash(slot, 0, 13) * period;
        let phase = (elapsed + offset) / period;
        let floor_p = phase.floor();
        let cycle = (floor_p as i32) as u32;
        let t = phase - floor_p;
        let env = (t / 0.14).min(1.0) * ((1.0 - t) / 0.32).clamp(0.0, 1.0);
        let a = env * weight * 0.85 * alpha_mul;
        if a <= 0.04 {
            continue;
        }
        let h1 = bubble_hash(slot, cycle, 19);
        let h2 = bubble_hash(slot, cycle, 31);
        let origin = match slot {
            6 => pose.place(-3.8, -1.2),
            7 => pose.place(-3.8, 1.2),
            _ => valve,
        };
        let rise = -t * (6.5 + 5.5 * h1);
        let drag = (vel * (-0.13 * t)).clamp_len(11.0);
        let wobble = (t * 7.2 + h2 * std::f32::consts::TAU).sin() * (0.5 + 1.1 * t);
        let bp = origin + drag + Vec2::new(wobble, rise);
        let s = w2s(bp, cam);
        blend(pb, s.0, s.1, tint.apply(BUBBLE_SPEC), a);
        if t < 0.55 && h1 > 0.45 {
            blend(pb, s.0 + 1, s.1 + 1, tint.apply(BUBBLE_RING), a * 0.40);
        }
    }
    pb.set_pen(prev_pen);
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

    #[test]
    fn dash_charge_ring_follows_the_size_buff() {
        use crate::game::ecs::systems::powerups::{GROWTH_SECS, SHRINK_SECS};

        let radius = |giant, tiny| {
            let mut player = rat(Vec2::splat(64.0), 0.0);
            player.buffs.giant = giant;
            player.buffs.tiny = tiny;
            player.dash_charging = true;
            player.dash_charge = 0.5;
            let mut pb = PixelBuffer::new(128, 128);
            draw_dash_charge(&mut pb, &player, 0.0, Vec2::ZERO);
            let top = (0..64).find(|&y| pb.pixel_at(64, y) != (0, 0, 0)).unwrap();
            (64 - top) as f32
        };
        let normal = radius(0.0, 0.0);
        assert_eq!(normal, 10.0);
        assert!((radius(GROWTH_SECS - 1.0, 0.0) - normal * 5.0).abs() <= 0.5);
        assert!((radius(0.0, SHRINK_SECS - 1.0) - normal * 0.35).abs() <= 0.5);
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

    /// The Super Dash charge pose: the rat coils as the meter fills, an idle
    /// rat (and a held dash with an empty meter) is exactly the pose it always
    /// was, and a dash in flight still stretches.
    #[test]
    fn charging_a_super_dash_coils_the_rat() {
        let mut p = rat(rig_pos(), 0.0);
        let idle = pose_of(&p, 0.0);
        assert_eq!((idle.su, idle.sv), (1.0, 1.0));

        p.dash_charging = true;
        p.dash_charge = 0.0;
        let held = pose_of(&p, 0.0);
        assert_eq!((held.su, held.sv), (1.0, 1.0), "an empty meter shows no coil");

        p.dash_charge = 0.5;
        let half = pose_of(&p, 0.0);
        p.dash_charge = 7.0; // out of range: clamped, never a negative stretch
        let full = pose_of(&p, 0.0);
        assert!(half.su < 1.0 && full.su < half.su, "shorter as it fills");
        assert!(half.sv > 1.0 && full.sv > half.sv, "wider as it fills");
        assert!((full.su - (1.0 - CHARGE_COIL_SQUASH)).abs() < 1e-6);
        assert!(full.su > 0.5, "the body never collapses");

        // The meter alone does nothing: the pose needs the held flag.
        p.dash_charging = false;
        let stale = pose_of(&p, 0.0);
        assert_eq!((stale.su, stale.sv), (1.0, 1.0));

        // The coiled rat still draws, clipped, at a buffer corner.
        p.dash_charging = true;
        p.pos = Vec2::new(0.0, 0.0);
        let mut pb = PixelBuffer::new(8, 8);
        draw_rat(&mut pb, &p, 0.3, Vec2::ZERO);
    }

    /// Every pixel of the rig, so two frames can be compared exactly.
    fn rig_pixels(p: &Player, elapsed: f32) -> Vec<Rgb> {
        let pb = render_rat(p, elapsed);
        let mut out = Vec::with_capacity(RIG_W * RIG_H);
        for y in 0..RIG_H {
            for x in 0..RIG_W {
                out.push(pb.pixel_at(x, y));
            }
        }
        out
    }

    /// The owner asked for the dash charge indicator **under the rat** to go:
    /// a cooling, ready, ready-popping or refused dash draws exactly the rat
    /// and nothing else, so no pip sits in the corner of the eye any more. (The
    /// HUD's dash module is where the cooldown lives now.)
    #[test]
    fn no_dash_pips_are_drawn_under_the_rat() {
        let base = rig_pixels(&rat(rig_pos(), 1.1), 0.4);
        // Each state used to light its own arc: cooling amber, the ready pop's
        // halo, the dashing drain, the refused red shake.
        for (cd, ready_flash, denied, dashing) in [
            (0.0, 0.0, 0.0, 0.0),  // ready
            (0.8, 0.0, 0.0, 0.0),  // cooling, part way back up
            (0.8, 0.4, 0.0, 0.0),  // the ready pop
            (1.4, 0.0, 0.3, 0.0),  // a refused press
        ] {
            let mut p = rat(rig_pos(), 1.1);
            p.dash_cd = cd;
            p.dash_ready_flash = ready_flash;
            p.dash_denied = denied;
            p.dash_active = dashing;
            assert_eq!(
                rig_pixels(&p, 0.4),
                base,
                "cd={cd} flash={ready_flash} denied={denied}: the gauge is back"
            );
        }
        // The dashing state changes the *pose*, so it is compared with the same
        // pose and no cooldown: the drain pip it used to add is gone too.
        let mut dashing = rat(rig_pos(), 1.1);
        dashing.dash_active = 0.16;
        let mut dashing_cooling = rat(rig_pos(), 1.1);
        dashing_cooling.dash_active = 0.16;
        dashing_cooling.dash_cd = 0.2;
        assert_eq!(rig_pixels(&dashing_cooling, 0.4), rig_pixels(&dashing, 0.4));
    }

    /// …but the Super Dash charge ring still appears while a charge is held,
    /// above the rat where the ring's first pip sits.
    #[test]
    fn the_super_dash_charge_ring_still_appears_while_charging() {
        let centre = rig_pos();
        // The top pip of the ring: `CHARGE_PIPS` start clockwise from straight
        // up, at `GAUGE_RADIUS`.
        let top = (centre.x as i32, (centre.y - GAUGE_RADIUS).round() as i32);
        let idle = render_rat(&rat(centre, 0.0), 0.25);
        let mut p = rat(centre, 0.0);
        p.dash_charging = true;
        p.dash_charge = 1.0;
        let charging = render_rat(&p, 0.25);
        let mut changed = 0;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let at = ((top.0 + dx) as usize, (top.1 + dy) as usize);
                if idle.pixel_at(at.0, at.1) != charging.pixel_at(at.0, at.1) {
                    changed += 1;
                }
            }
        }
        assert!(
            changed >= 2,
            "the charge ring no longer draws above the rat ({changed} pixels changed)"
        );
        // A held dash with an empty meter draws the sockets, so it still reads
        // as a ring rather than nothing.
        let mut empty = rat(centre, 0.0);
        empty.dash_charging = true;
        empty.dash_charge = 0.0;
        let cold = render_rat(&empty, 0.25);
        let mut socket = 0;
        for y in 0..RIG_H as i32 {
            for x in 0..RIG_W as i32 {
                if cold.pixel_at(x as usize, y as usize) != idle.pixel_at(x as usize, y as usize) {
                    socket += 1;
                }
            }
        }
        assert!(socket >= 18, "an empty held meter drew no ring: {socket} pixels");
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

    /// Batch 12: Gnaw bite crescents, Tail Whip frontal half-circle crescent
    /// (front only + dust motes), Tail Thrash broad pink full disc, and Spiked
    /// Tail physics Verlet whip (wave during wind-up, tip landing on target at
    /// the exact crack tick, spikes + white crack flash + tiny shock ring,
    /// recoil & settle, and Barbed Wire catch drag) all render cleanly and leave
    /// `lash == None` untouched.
    #[test]
    fn batch12_bite_whip_thrash_and_spiked_tail_render() {
        let mut pb = PixelBuffer::new(96, 96);
        let bg = (18, 22, 24);
        let count_lit = |buf: &PixelBuffer| -> usize {
            (0..buf.h)
                .flat_map(|y| (0..buf.w).map(move |x| (x, y)))
                .filter(|&(x, y)| buf.pixel_at(x, y) != bg)
                .count()
        };

        // Gnaw bite: closing white tooth crescents.
        pb.clear(bg);
        let snap = BiteSnap {
            pos: Vec2::new(40.0, 48.0),
            dir: Vec2::new(1.0, 0.0),
            t: 0.055,
            total: 0.11,
        };
        draw_gnaw_bite(&mut pb, &snap, Vec2::ZERO);
        assert!(count_lit(&pb) >= 16, "gnaw bite should draw closing tooth crescents");

        // Tail Whip: the recorded crescent stays behind a rat facing +X.
        pb.clear(bg);
        let crack = WhipCrack {
            from: Vec2::new(48.0, 48.0),
            to: Vec2::new(18.0, 48.0),
            t: 0.075,
            total: 0.12,
        };
        draw_tail_whip_crack(&mut pb, &crack, Vec2::ZERO);
        assert!(count_lit(&pb) >= 60, "tail whip should draw a visible rear crescent");
        let rear_centre = pb.pixel_at(18, 48);
        assert!(
            rear_centre != bg && rear_centre.0 >= 150,
            "tail whip crescent should be bright pink/white at rear centre, got {rear_centre:?}"
        );
        let ahead_lit = (0..pb.h)
            .flat_map(|y| (48..pb.w).map(move |x| (x, y)))
            .filter(|&(x, y)| pb.pixel_at(x, y) != bg)
            .count();
        assert_eq!(
            ahead_lit, 0,
            "tail whip crescent must stay behind the rat, found {ahead_lit} pixels ahead"
        );

        // Tail Thrash: broad pink full-disc sweep trail.
        pb.clear(bg);
        let p = rat(Vec2::new(48.0, 48.0), 0.0);
        draw_tail_thrash(&mut pb, &p, Some((0.4, 34.0, 0.9)), Vec2::ZERO);
        assert!(count_lit(&pb) >= 180, "tail thrash should draw a broad full-circle disc trail");

        // Spiked Tail: Verlet physics whip simulation across wind-up, crack,
        // recoil, and post-lash settle.
        let target = Vec2::new(82.0, 48.0);
        let mut lash = Lash {
            origin: p.pos,
            target,
            primary: None,
            boss: false,
            reach: 44.0,
            radius: 10.0,
            dmg: 22.0,
            shove: 120.0,
            shards: 0,
            evolved: false,
            timer: 0.0,
            struck: false,
            dragged: false,
        };
        let mut chain = WhipChain::new();
        assert!(!chain.is_active(), "idle chain starts inactive");

        // Step through wind-up at 60 Hz: verify a genuine lateral whip wave
        // forms off the rump->target chord before the crack.
        let mut max_lateral = 0.0f32;
        let dt = 1.0 / 60.0;
        for tick in 1..=7 {
            lash.timer = tick as f32 * dt;
            chain.sync(&p, lash.timer, Some((1, &lash, None)));
            assert!(chain.is_active());
            for node in chain.nodes().iter().skip(2).take(WHIP_NODES - 3) {
                max_lateral = max_lateral.max((node.y - 48.0).abs());
            }
        }
        assert!(
            max_lateral >= 4.0,
            "wind-up throw should produce a visible lateral whip wave, got max_lateral={max_lateral}"
        );

        // Exact crack tick (t >= LASH_WINDUP): tip must land squarely on target
        // and render white crack flash + tiny shock ring + spikes.
        lash.timer = LASH_WINDUP;
        lash.struck = true;
        chain.sync(&p, lash.timer, Some((1, &lash, None)));
        assert!(
            chain.tip().dist(target) <= 0.5,
            "whip tip must land on target at crack tick, got dist={}",
            chain.tip().dist(target)
        );
        pb.clear(bg);
        draw_rat_with_whip(&mut pb, &p, lash.timer, Vec2::ZERO, 0.0, None, Some(&chain));
        assert!(count_lit(&pb) >= 80, "spiked tail physics whip should render chain, spikes & flash");
        let tip_px = pb.pixel_at(82, 48);
        assert!(
            tip_px.0 >= 230 && tip_px.1 >= 230 && tip_px.2 >= 230,
            "crack tip should have a pure-white crack flash, got {tip_px:?}"
        );

        // Determinism check: a fresh chain jumping straight to the crack tick
        // (capture harness snapshot) must also land on target without NaN.
        let mut snap_chain = WhipChain::new();
        snap_chain.sync(&p, LASH_WINDUP, Some((1, &lash, None)));
        assert!(snap_chain.tip().dist(target) <= 0.5);
        for n in snap_chain.nodes() {
            assert!(n.x.is_finite() && n.y.is_finite());
        }

        // Recoil + post-lash settle returns cleanly to inactive idle tail.
        lash.timer = LASH_TOTAL - 0.01;
        chain.sync(&p, lash.timer, Some((1, &lash, None)));
        for step in 1..=15 {
            chain.sync(&p, LASH_TOTAL + step as f32 * dt, None);
        }
        assert!(!chain.is_active(), "chain must settle back to inactive idle tail");

        // Barbed Wire evolution: reels the hooked catch back along the chain.
        let mut barbed = Lash {
            evolved: true,
            timer: LASH_RECOIL_AT + 0.08,
            struck: true,
            dragged: true,
            ..lash
        };
        let mut bchain = WhipChain::new();
        bchain.sync(&p, barbed.timer, Some((2, &barbed, Some(Vec2::new(62.0, 48.0)))));
        assert!(
            bchain.tip().dist(p.pos) < target.dist(p.pos) - 5.0,
            "barbed wire recoil should visibly reel tip back toward the rat"
        );
        pb.clear(bg);
        draw_rat_with_whip(&mut pb, &p, barbed.timer, Vec2::ZERO, 0.0, None, Some(&bchain));
        assert!(count_lit(&pb) >= 60);

        // Zero-size buffer and degenerate lash must never panic.
        barbed.target = p.pos;
        bchain.sync(&p, 0.5, Some((3, &barbed, None)));
        let mut zero = PixelBuffer::new(0, 0);
        draw_gnaw_bite(&mut zero, &snap, Vec2::ZERO);
        draw_tail_whip_crack(&mut zero, &crack, Vec2::ZERO);
        draw_tail_thrash(&mut zero, &p, Some((0.4, 34.0, 0.9)), Vec2::ZERO);
        draw_rat_with_whip(&mut zero, &p, 0.5, Vec2::ZERO, 0.0, None, Some(&bchain));
    }

    #[test]
    fn rat_outfit_selects_diver_only_on_outfall() {
        assert_eq!(RatOutfit::for_theme(Theme::Outfall), RatOutfit::Diver);
        for theme in Theme::playable() {
            if theme != Theme::Outfall {
                assert_eq!(
                    RatOutfit::for_theme(theme),
                    RatOutfit::Standard,
                    "{theme:?} must keep the standard rat"
                );
            }
        }
    }

    #[test]
    fn outfall_diver_differs_from_standard_with_brass_helmet_and_materials() {
        let p = rat(rig_pos(), 0.0);
        let base_pen = crate::render::lighting::rat_pen();
        let helmet_pen = crate::render::lighting::rat_helmet_pen(false);
        let visor_pen = crate::render::lighting::rat_visor_pen(false);

        let mut plain = PixelBuffer::new(RIG_W, RIG_H);
        plain.set_materials(true);
        plain.clear(RIG_BG);
        plain.set_pen(base_pen);
        draw_rat(&mut plain, &p, 0.4, Vec2::ZERO);

        let mut std_explicit = PixelBuffer::new(RIG_W, RIG_H);
        std_explicit.set_materials(true);
        std_explicit.clear(RIG_BG);
        std_explicit.set_pen(base_pen);
        draw_rat_with_outfit(
            &mut std_explicit,
            &p,
            0.4,
            Vec2::ZERO,
            0.0,
            None,
            None,
            RatOutfit::Standard,
        );
        assert_eq!(
            plain.pixels(),
            std_explicit.pixels(),
            "Standard outfit must be pixel-identical to draw_rat"
        );
        assert_eq!(
            plain.mat(),
            std_explicit.mat(),
            "Standard outfit must be material-identical to draw_rat"
        );

        let mut diver = PixelBuffer::new(RIG_W, RIG_H);
        diver.set_materials(true);
        diver.clear(RIG_BG);
        diver.set_pen(base_pen);
        draw_rat_with_outfit(
            &mut diver,
            &p,
            0.4,
            Vec2::ZERO,
            0.0,
            None,
            None,
            RatOutfit::Diver,
        );

        let mut diff_px = 0usize;
        let mut brass_px = 0usize;
        let mut helmet_mat_px = 0usize;
        let mut visor_mat_px = 0usize;
        for (i, (&a, &b)) in plain.pixels().iter().zip(diver.pixels().iter()).enumerate() {
            if a != b {
                diff_px += 1;
            }
            if b.0 >= 165 && b.1 >= 95 && b.0 > b.1 && b.0 >= b.2.saturating_add(70) {
                brass_px += 1;
            }
            let m = diver.mat()[i];
            if m == helmet_pen {
                helmet_mat_px += 1;
            } else if m == visor_pen {
                visor_mat_px += 1;
            }
        }
        assert!(diff_px >= 20, "diver should visibly differ from standard rat ({diff_px} px)");
        assert!(brass_px >= 6, "diver helmet should paint warm brass pixels ({brass_px} px)");
        assert!(helmet_mat_px >= 6, "diver helmet should stamp Metal pen ({helmet_mat_px} px)");
        assert!(visor_mat_px >= 2, "diver porthole should stamp Glass pen ({visor_mat_px} px)");
    }

    #[test]
    fn diver_swim_bob_changes_pixels_without_moving_hitbox_or_position() {
        let p = rat(rig_pos(), 0.25);
        let orig_pos = p.pos;
        let orig_hurt = p.buffs.hurt_radius();

        let render_diver = |player: &Player, t: f32| {
            let mut pb = PixelBuffer::new(RIG_W, RIG_H);
            pb.clear(RIG_BG);
            draw_rat_with_outfit(&mut pb, player, t, Vec2::ZERO, 0.0, None, None, RatOutfit::Diver);
            pb
        };
        let a = render_diver(&p, 0.0);
        let b = render_diver(&p, 0.65);
        assert_eq!(p.pos, orig_pos, "render must never move player.pos");
        assert_eq!(p.buffs.hurt_radius(), orig_hurt, "render must never alter hurt_radius");
        let changed = a
            .pixels()
            .iter()
            .zip(b.pixels().iter())
            .filter(|(pa, pb)| pa != pb)
            .count();
        assert!(
            changed >= 8,
            "idle hover bob + bubbles should smoothly animate the diver over time ({changed} px)"
        );

        // Moving swim + dash kick-glide + edge/zero-size clipping must not panic.
        let mut swimmer = p;
        swimmer.vel = swimmer.face * 55.0;
        let swim_a = render_diver(&swimmer, 0.10);
        let swim_b = render_diver(&swimmer, 0.35);
        assert_ne!(swim_a.pixels(), swim_b.pixels(), "swimming cycle must advance smoothly");
        swimmer.dash_active = 0.14;
        let _dash = render_diver(&swimmer, 0.35);

        let mut zero = PixelBuffer::new(0, 0);
        draw_rat_with_outfit(&mut zero, &swimmer, 0.35, Vec2::ZERO, 0.0, None, None, RatOutfit::Diver);
    }

    #[test]
    fn diver_rat_floats_above_detached_ground_shadow_at_true_position() {
        let p = rat(rig_pos(), 0.0);
        let base_pen = crate::render::lighting::rat_pen();
        let helmet_pen = crate::render::lighting::rat_helmet_pen(false);

        let render = |t: f32| {
            let mut pb = PixelBuffer::new(RIG_W, RIG_H);
            pb.set_materials(true);
            pb.clear(RIG_BG);
            pb.set_pen(base_pen);
            draw_rat_with_outfit(&mut pb, &p, t, Vec2::ZERO, 0.0, None, None, RatOutfit::Diver);
            pb
        };
        let pb = render(0.0);
        // True ground point is darkened by the detached seabed shadow (pen 0).
        let gx = p.pos.x as usize;
        let gy = p.pos.y as usize;
        let ground_px = pb.pixel_at(gx, gy);
        assert!(
            ground_px.0 < RIG_BG.0 && ground_px.1 < RIG_BG.1 && ground_px.2 < RIG_BG.2,
            "true ground position should carry the detached shadow, got {ground_px:?} vs {RIG_BG:?}"
        );
        assert_eq!(pb.mat()[gy * pb.w + gx], 0, "detached seabed shadow must use floor pen 0");

        // Helmet pixels sit clearly above `player.pos.y`.
        let mut sum_y = 0.0f32;
        let mut n = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.mat()[y * pb.w + x] == helmet_pen {
                    sum_y += y as f32;
                    n += 1;
                }
            }
        }
        assert!(n >= 4, "expected helmet pixels");
        let mean_y = sum_y / n as f32;
        assert!(
            mean_y <= p.pos.y - 3.0,
            "lifted diver helmet should sit well above ground y={} (mean_y={mean_y:.2})",
            p.pos.y
        );

        // Higher bob (`sin(2.5*t) < 0` -> larger lift) casts a fainter shadow than lower dip.
        let high = render(std::f32::consts::PI * 1.5 / 2.5);
        let low = render(std::f32::consts::FRAC_PI_2 / 2.5);
        assert!(
            high.pixel_at(gx, gy).0 > low.pixel_at(gx, gy).0,
            "higher bob should cast a fainter seabed shadow ({:?} vs {:?})",
            high.pixel_at(gx, gy),
            low.pixel_at(gx, gy)
        );
    }
}
