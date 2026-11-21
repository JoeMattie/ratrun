//! Art and gameplay specification for the Flatbed's boss — **THE MAGPIE**.
//!
//! Forty feet of oak deck doing a hundred klicks in the dark, and something has
//! been riding the slipstream over the cab since the last weigh station. It
//! comes down on the cargo, and it wants what the rat wants: the cab.
//!
//! A magpie is the biggest thing a rat could meet on a moving flatbed that is
//! still recognisably a *bird*: black-and-white, iridescent blue-green tail,
//! yellow-hilted bill, the swagger of something that steals shiny things. Read
//! from a rat's height it is a beak the length of the rat's body, a shadow that
//! crosses the planks, and a wash of wind that smells of road.
//!
//! # The fight the level was built for
//!
//! The flatbed's own grammar is *motion*: the deck runs west to east, the road
//! rushes underneath at 540 px/s, the slipstream drags light things toward the
//! tailgate, and the only way off is forward. The Magpie is the level's
//! load-bearing idea turned into a body: it **rides the wind**, so every one of
//! its attacks is aimed *down the deck* or *across it*, and the counterplay is
//! always a sidestep across the plank, never a retreat along it.
//!
//! | phase | hp | rotation | read |
//! |---|---|---|---|
//! | 1 | `> 0.62` | `Perch` → `PeckWindup` → `Peck` → `Stuck` | a beak line on the planks: sidestep it, then bite the buried head |
//! | 2 | `0.30..=0.62` | `Perch` → `Takeoff` → `StrafeWindup` → `StrafePass` → `SnatchDrop` → `Land` (alternating with `Peck`) | a shadow lane across the bed: be on the far side of it; then a marked square where the crate comes down |
//! | 3 | `<= 0.30` | `GustWindup` → `Gust` → `Peck` / `StrafePass` | wing-beat gusts shove everything tailward in pulses with calm gaps; pecks in the gaps |
//!
//! ## The deck-peck
//!
//! `PeckWindup` draws a **beak line** from the bill to a marked square and
//! freezes it at [`PECK_LOCK_PROGRESS`], exactly like the cat's landing ring, so
//! the rat can walk out of it. `Peck` lunges down the line and the line is live
//! for [`PECK_ACTIVE_HALF_WIDTH`] worth of corridor for [`PECK_ACTIVE`] s.
//!
//! A peck the rat dodges leaves the bill **in the plank**: `Stuck` is the
//! punish window (no body contact, [`STUCK_DAMAGE_MULT`] incoming damage, the
//! head — the smallest and brightest hit shape on the bird — parked out in the
//! open where the rat can reach it). A bird whose back is against cargo or a
//! bulkhead when it lunges cannot pull its own bill out: that is the
//! [`STUCK_WALLED`] window, half again as long. Fighting a magpie in a corner
//! is how you get a free combo.
//!
//! ## The strafing pass
//!
//! `StrafeWindup` paints a **lane** the length of the bed (two rails and a
//! hatched centre, [`STRAFE_LANE_HALF_WIDTH`]) and the bird flies it at
//! [`STRAFE_SPEED`]: the shadow crosses the planks, the wake is live, and the
//! safe floor is whichever side of the lane the rat is already on. It is
//! telegraphed for a full [`STRAFE_WINDUP`] s and the lane is only
//! [`STRAFE_LANE_HALF_WIDTH`] px wide in a 416 px hold, so leaving it costs one
//! sidestep.
//!
//! `SnatchDrop` is the payoff for standing in the open after a pass: the bird
//! wheels with a crate/strap in its claws and drops it on a square marked with a
//! [`CRATE_WARN`] s shadow. The crate bursts into splinters. (The design offered
//! "it stays as cover, or opens a new broken-plank pit"; a controller may queue
//! hazards but cannot spawn props or edit the deck, so the shipped version is a
//! *shatter* — the crate is a one-shot hazard and the art draws the fall, the
//! square and the burst.)
//!
//! ## The gust
//!
//! Phase 3 opens the slipstream up to the bird's wing-beats: [`GUST_PULSES`]
//! pulses of [`GUST_PUSH`] px/s down the deck (toward the tailgate — the level's
//! own `-x`), [`GUST_PULSE_ACTIVE`] s on and a calm gap between, all inside
//! [`GUST_DURATION`]. The gust deals **no damage at all** (a push-only hazard),
//! so a pulse can shove the rat without spending i-frames, and the average
//! push (`GUST_PUSH * ACTIVE / PERIOD` ≈ 39 px/s) is well under the rat's 58
//! px/s walk: the rat loses ground during a pulse and wins it back in the gap,
//! which is what makes the gusts a pressure rather than a loss of control.
//!
//! The gust is *suppressed* when the rat is standing on the lip of a hole in
//! the deck ([`GUST_PIT_CLEARANCE`] px of plank behind it) — the bird will not
//! blow the rat down a hole it cannot read — and the turn is spent on a peck
//! instead.
//!
//! ## The bird's own shapes (binding on the controller)
//!
//! * [`body_hit_circles`] — three circles: tail root, body, head. The head
//!   carries `grounded: true` always (the bird flies *low*: belly to the
//!   planks, claws trailing, so ground weapons reach it) but is **displaced to
//!   the strike point** while the bill is buried, which is the whole point of
//!   the punish window.
//! * [`is_airborne`] — `Takeoff`, `StrafeWindup`, `StrafePass`, `SnatchDrop`:
//!   body contact is off and the ground shadow goes soft and small; the hit
//!   circles keep hitting.
//! * [`incoming_damage_mult`] — [`STUCK_DAMAGE_MULT`] in `Stuck`,
//!   [`GUST_DAMAGE_MULT`] in the gust (the bird is on the deck with its wings
//!   committed), `0.0` in `Death`.
//!
//! # Layering contract for the renderer
//!
//! 1. **Ground layer** (after the floor stains and before props/entities):
//!    [`draw_magpie_ground`] — beak line, lane rails, crate square, gust
//!    streaks, dust rings and the bird's drop shadow.
//! 2. **Entity layer** (with the other bodies): [`draw_magpie`] — shadow first,
//!    tail, wings, body, head, bill, the weak-point glint on a buried head, and
//!    the death molts.
//!
//! Everything is stack-only and culls on the sprite margin, so the draw cost is
//! the same order as the cat's.

#![allow(dead_code)]

use std::f32::consts::PI;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Collision & gameplay constants (art <-> controller <-> framework contract)
// ============================================================================

/// Framework movement/contact circle for the standing bird (world px): the
/// width of the folded body plus a feather.
pub const BODY_RADIUS: f32 = 11.0;

/// Beak tip to tail tip, folded (the culling/length figure).
pub const BODY_LENGTH: f32 = 36.0;

/// Longest sprite reach (wings spread in a strafing pass) for culling.
pub const CULL_MARGIN: f32 = 54.0;

/// Head centre, along the facing axis from the body centre.
pub const HEAD_OFFSET: f32 = 14.0;

/// Radius of the head (the weak point while the bill is buried).
pub const HEAD_RADIUS: f32 = 5.8;

/// Bill length from the head centre to the tip.
pub const BEAK_LENGTH: f32 = 11.5;

/// Tail length behind the body centre.
pub const TAIL_LENGTH: f32 = 20.0;

/// Half-span of one wing when spread (a strafing bird is ~44 px across).
pub const WING_SPAN: f32 = 22.0;

/// The art's recommended base pool, before `BossKind::hp_scale` and the
/// director's boss curve. Sized like the mower's: the flatbed is a practice
/// floor reached with a full 15-minute build, so the *scale* carries the fight
/// length (see `BossKind::hp_scale`).
pub const BOSS_BASE_HP: f32 = 1600.0;

/// Body contact while perched (owner's rule: 8 outside a recovery window).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 opens here (the bird takes the fight into the air).
pub const PHASE2_HP_FRAC: f32 = 0.62;

/// Phase 3 opens here (the slipstream joins in).
pub const PHASE3_HP_FRAC: f32 = 0.30;

/// Critical shimmer on the head (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

// ---- neutral beat ---------------------------------------------------------

/// `Perch` length per phase: it stands, it watches, it hops closer.
pub const PERCH_DURATION: [f32; 3] = [1.30, 1.10, 0.95];

/// Walk speed per phase while perched (`34 / 40 / 46` px/s).
pub const PERCH_SPEED: [f32; 3] = [34.0, 40.0, 46.0];

/// The band `Perch` holds the rat at, in px (`near`, `far`).
pub const PERCH_BAND: [f32; 2] = [26.0, 62.0];

/// Extra speed multiplier while hunting a rat that has left the hold: the bird
/// runs (wings half open) after the rat down the deck.
pub const HUNT_SPEED_MULT: f32 = 1.9;

// ---- the deck-peck --------------------------------------------------------

/// The lunge's telegraph window (`PeckWindup`).
pub const PECK_WINDUP: [f32; 3] = [0.62, 0.56, 0.50];

/// The line freezes here so the rat can step out of it.
pub const PECK_LOCK_PROGRESS: f32 = 0.55;

/// The line's corridor half-width (the live beak width plus feathers).
pub const PECK_ACTIVE_HALF_WIDTH: f32 = 5.0;

/// The strike's live window (single hit: the hazard fires once).
pub const PECK_ACTIVE: f32 = 0.12;

/// The blade of the strike (`Peck`): the bill drives into the plank.
pub const PECK_STRIKE: f32 = 0.24;

/// Furthest the bird will commit a peck to (bill tip to strike point).
pub const PECK_REACH: f32 = 62.0;

/// Nearest (a peck at contact range is a body slam, not a peck).
pub const PECK_MIN: f32 = 6.0;

/// Peck damage (before armour).
pub const PECK_DAMAGE: f32 = 20.0;

/// The lunge's speed down the line (px/s).
pub const PECK_LUNGE_SPEED: f32 = 230.0;

/// The buried-bill punish window (`Stuck`).
pub const STUCK_DURATION: [f32; 3] = [1.00, 0.90, 0.85];

/// The longer window when the bird's back was against cargo or a bulkhead and
/// it could not pull its own bill out (the "cornered bird" rule).
pub const STUCK_WALLED: f32 = 1.55;

/// Incoming damage on the buried head.
pub const STUCK_DAMAGE_MULT: f32 = 1.35;

/// How far behind itself the bird looks for an obstruction that would keep it
/// from pulling the bill out.
pub const WALL_BACK_PROBE: f32 = 14.0;

// ---- the strafing pass ----------------------------------------------------

/// `Takeoff`: the bird gets off the deck (no contact, shadow drops away).
pub const TAKEOFF_DURATION: f32 = 0.45;

/// The lane is painted for this long before the pass.
pub const STRAFE_WINDUP: [f32; 3] = [0.75, 0.68, 0.62];

/// Pass speed per phase (`150 / 165 / 180` px/s).
pub const STRAFE_SPEED: [f32; 3] = [150.0, 165.0, 180.0];

/// Live corridor half-width of a pass (the shadow's width).
pub const STRAFE_LANE_HALF_WIDTH: f32 = 11.0;

/// What a clipped pass costs.
pub const STRAFE_DAMAGE: f32 = 16.0;

/// The shortest pass worth flying (a lane shorter than this is a hop).
pub const STRAFE_MIN_LEN: f32 = 80.0;

/// Clear floor the rat needs on the safe side of a lane before the pass is
/// allowed to run (the suppression rule: never sweep a lane with no exit).
pub const STRAFE_SAFE_SIDE: f32 = 26.0;

/// How far inside the hold's walls a lane starts and ends.
pub const STRAFE_END_MARGIN: f32 = 26.0;

// ---- the dropped crate ----------------------------------------------------

/// `SnatchDrop` length: the wheel, the drop, the burst.
pub const CRATE_DURATION: f32 = 0.95;

/// The marked square's warning before the crate lands.
pub const CRATE_WARN: f32 = 0.70;

/// The crate's landing square half-extent.
pub const CRATE_HALF: f32 = 9.0;

/// Crush damage.
pub const CRATE_DAMAGE: f32 = 16.0;

/// Splinter knockback.
pub const CRATE_KNOCKBACK: f32 = 130.0;

/// The crate lights (and bursts) here in `SnatchDrop`, in progress.
pub const CRATE_LAND_PROGRESS: f32 = 0.74;

/// `Land`: the bird settles back onto the deck.
pub const LAND_DURATION: f32 = 0.55;

// ---- the gust -------------------------------------------------------------

/// `GustWindup`: the wings come back and the streaks gather.
pub const GUST_WINDUP: [f32; 3] = [0.85, 0.78, 0.70];

/// The whole gust beat, three pulses inside it.
pub const GUST_DURATION: f32 = 1.80;

/// Wing-beat pulses in one gust.
pub const GUST_PULSES: u32 = 3;

/// Pulse period: [`GUST_PULSE_ACTIVE`] of push and a calm gap after it.
pub const GUST_PULSE_PERIOD: f32 = 0.55;

/// How long one pulse pushes for (0.23 s of calm gap after each).
pub const GUST_PULSE_ACTIVE: f32 = 0.32;

/// Push speed down the deck (px/s): under the rat's 58 px/s walk, so the calm
/// gaps are where it wins the ground back.
pub const GUST_PUSH: f32 = 68.0;

/// How far downwind of the bird the wing-beats reach.
pub const GUST_REACH: f32 = 190.0;

/// Half-width of the gust field across the deck.
pub const GUST_HALF_WIDTH: f32 = 400.0;

/// The push radiates from a point this far *upwind* of the bird, so every point
/// of the field is pushed along the deck rather than out in a fan.
pub const GUST_ANCHOR: f32 = 900.0;

/// The trailer's tailgate is this way (the level's own slipstream direction).
pub const GUST_AXIS: Vec2 = Vec2 { x: -1.0, y: 0.0 };

/// Incoming damage while the bird is on the deck beating its wings.
pub const GUST_DAMAGE_MULT: f32 = 1.15;

/// The bird will not gust when the rat has less plank than this behind it
/// (a hole is `16 px`; this leaves a walkable lip).
pub const GUST_PIT_CLEARANCE: f32 = 22.0;

// ---- phase change & death -------------------------------------------------

/// The harmless phase-change pose (owner's rule: 1.2 s, no contact, no attack).
pub const RUFFLE_DURATION: f32 = 1.20;

/// The death beat.
pub const DEATH_DURATION: f32 = 2.20;

/// Death dust settle ring radius.
pub const DEATH_RING_RADIUS: f32 = 30.0;

// ---- light ---------------------------------------------------------------

/// Key light, upper left (the same convention as the other boss art).
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// ============================================================================
// State spec
// ============================================================================

/// The Magpie's move set. Every variant documents its timing, telegraph,
/// lethal area, damage and the counterplay the controller builds to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagpieState {
    /// **Hop / watch.** The bird stands on the cargo and walks the rat down to
    /// its pecking band with the head-bob of a bird deciding where to stab.
    ///
    /// - Duration: [`PERCH_DURATION`] (`1.30 / 1.10 / 0.95` s).
    /// - Warning: none (it is the neutral beat).
    /// - Dangerous area: body circle, [`CONTACT_DAMAGE`] (8).
    Perch,

    /// **Ruffle.** The phase-change pose: the bird shakes its feathers out,
    /// half-opens its wings and resettles.
    ///
    /// - Duration: [`RUFFLE_DURATION`] (1.20 s), no contact, no attack.
    /// - Counter: free damage — the set piece beat at `0.62` and `0.30`.
    Ruffle,

    /// **The bill comes up.** The bird draws its head back and the beak line is
    /// painted from the bill to a marked square on the planks, freezing at
    /// [`PECK_LOCK_PROGRESS`].
    ///
    /// - Duration: [`PECK_WINDUP`] (`0.62 / 0.56 / 0.50` s).
    /// - Warning: the beak line, [`PECK_ACTIVE_HALF_WIDTH`] px either side.
    /// - Dangerous area: the line once `Peck` runs (not during the windup).
    PeckWindup,

    /// **The stab.** The bill drives down the frozen line.
    ///
    /// - Duration: [`PECK_STRIKE`] (0.24 s); the corridor is live for
    ///   [`PECK_ACTIVE`] (0.12 s).
    /// - Damage: [`PECK_DAMAGE`] (20), once per stab.
    Peck,

    /// **The bill is in the plank.** The punish window: no body contact, the
    /// head parked at the strike point, [`STUCK_DAMAGE_MULT`] incoming.
    ///
    /// - Duration: [`STUCK_DURATION`], or [`STUCK_WALLED`] when the bird had
    ///   cargo or a bulkhead behind it.
    Stuck,

    /// **Up.** The bird gets off the deck, wings opening.
    ///
    /// - Duration: [`TAKEOFF_DURATION`] (0.45 s), no contact.
    Takeoff,

    /// **The lane is painted.** The bird hovers at the near end of a lane down
    /// the bed and the rails + hatch of the pass appear.
    ///
    /// - Duration: [`STRAFE_WINDUP`] (`0.75 / 0.68 / 0.62` s).
    /// - Warning: the lane, [`STRAFE_LANE_HALF_WIDTH`] px either side.
    StrafeWindup,

    /// **The pass.** A straight flight the length of the bed at
    /// [`STRAFE_SPEED`]; the lane is live for the whole pass.
    ///
    /// - Damage: [`STRAFE_DAMAGE`] (16), once per pass.
    StrafePass,

    /// **Wheel and drop.** The bird brakes, snatches a crate or coil of strap
    /// and lets it go over a marked square.
    ///
    /// - Duration: [`CRATE_DURATION`] (0.95 s); the square is marked for
    ///   [`CRATE_WARN`] (0.70 s) and the crate lands at
    ///   [`CRATE_LAND_PROGRESS`].
    /// - Damage: [`CRATE_DAMAGE`] (16) plus splinter knockback, once.
    SnatchDrop,

    /// **Settle.** The bird comes back down onto the planks.
    ///
    /// - Duration: [`LAND_DURATION`] (0.55 s).
    Land,

    /// **Wings back.** The bird digs its claws in, sweeps its wings back and the
    /// streaks gather along the deck.
    ///
    /// - Duration: [`GUST_WINDUP`] (`0.85 / 0.78 / 0.70` s).
    GustWindup,

    /// **Wing-beats.** [`GUST_PULSES`] pulses of wind down the deck, with calm
    /// gaps between them.
    ///
    /// - Duration: [`GUST_DURATION`] (1.80 s).
    /// - Damage: none at all — push-only, [`GUST_PUSH`] px/s toward the
    ///   tailgate. No i-frames are spent, so the follow-up peck still hurts.
    Gust,

    /// **Death.** The bird seizes, keels over and molts; the Cab hold goes quiet.
    Death,
}

impl MagpieState {
    /// Is the bird off the deck (no body contact, soft shadow)?
    #[inline]
    pub fn flying(self) -> bool {
        matches!(
            self,
            MagpieState::Takeoff
                | MagpieState::StrafeWindup
                | MagpieState::StrafePass
                | MagpieState::SnatchDrop
        )
    }
}

/// Should the framework treat this state as airborne (contact off)?
#[inline]
pub fn is_airborne(view: &MagpieView) -> bool {
    view.state.flying()
}

/// Incoming damage multiplier (the art owns it, as for the other bosses).
#[inline]
pub fn incoming_damage_mult(state: MagpieState) -> f32 {
    match state {
        MagpieState::Stuck => STUCK_DAMAGE_MULT,
        MagpieState::Gust | MagpieState::GustWindup => GUST_DAMAGE_MULT,
        MagpieState::Death => 0.0,
        _ => 1.0,
    }
}

/// Does the bird deal body contact in this state?
#[inline]
pub fn has_body_contact(state: MagpieState) -> bool {
    !matches!(
        state,
        MagpieState::Ruffle
            | MagpieState::Peck
            | MagpieState::Stuck
            | MagpieState::Death
            | MagpieState::Gust
            | MagpieState::GustWindup
    ) && !state.flying()
}

/// Is a gust pulse pushing at `progress` (0..=1 through [`Gust`])?
///
/// The trailer's own wind reads this through `ecs::systems::trailer`, so the
/// gems and light bodies on the deck answer the same wing-beat the rat does.
#[inline]
pub fn gust_pulse_active(state: MagpieState, progress: f32) -> bool {
    if state != MagpieState::Gust {
        return false;
    }
    let t = progress.clamp(0.0, 1.0) * GUST_DURATION;
    let within = t.rem_euclid(GUST_PULSE_PERIOD);
    within < GUST_PULSE_ACTIVE && t < GUST_PULSE_PERIOD * GUST_PULSES as f32
}

/// Which pulse (0..[`GUST_PULSES`]) is due at `elapsed` seconds into `Gust`.
#[inline]
pub fn gust_pulse_due(elapsed: f32, fired: u32) -> bool {
    if fired >= GUST_PULSES {
        return false;
    }
    elapsed >= f32::from(fired as u16) * GUST_PULSE_PERIOD
}

/// The gust field: a push-only rectangle that covers everything downwind of the
/// bird. Its push radiates from [`GUST_ANCHOR`] px upwind, so every point of it
/// is pushed along the deck (within `atan(GUST_HALF_WIDTH / GUST_ANCHOR)` ≈ 24°
/// of straight tailward) rather than fanned out from the bird's body.
#[inline]
pub fn gust_shape(bird: Vec2) -> crate::game::ecs::systems::boss::HazardShape {
    use crate::game::ecs::systems::boss::HazardShape;
    HazardShape::Rect {
        center: bird - GUST_AXIS * GUST_ANCHOR,
        facing: GUST_AXIS,
        half: GUST_ANCHOR + GUST_REACH,
        half_perp: GUST_HALF_WIDTH,
    }
}

// ============================================================================
// View
// ============================================================================

/// A pure, `Copy` snapshot of everything the art needs. Built by
/// `Boss::magpie_view`; carries no simulation borrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagpieView {
    /// Body centre in world px (the ground footprint).
    pub pos: Vec2,
    /// Facing (normalised defensively).
    pub facing: Vec2,
    pub state: MagpieState,
    /// 0..=1 through the current state.
    pub progress: f32,
    /// Animation clock, seconds.
    pub anim: f32,
    /// Hit flash, 0..=1 (fades over ~0.12 s).
    pub hit_flash: f32,
    /// Remaining health fraction.
    pub hp_frac: f32,
    /// The marked point: the strike point of a peck, the far end of a lane, or
    /// the crate's landing square, depending on the state.
    pub target: Vec2,
    /// The lane's start / the lunge's origin.
    pub lane_start: Vec2,
    /// `+1`/`-1`: which way the lane sweeps.
    pub sweep: f32,
    /// The bird is out of its hold, running the deck after a rat that fled.
    pub hunting: bool,
}

impl MagpieView {
    /// A perched view looking at `target`.
    pub fn new(pos: Vec2, facing: Vec2, target: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: MagpieState::Perch,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
            hp_frac: 1.0,
            target,
            lane_start: pos,
            sweep: 1.0,
            hunting: false,
        }
    }

    /// Light-enraged (phase 2+) tint switch.
    #[inline]
    pub fn enraged(&self) -> bool {
        self.hp_frac <= PHASE2_HP_FRAC
    }

    /// Critical (phase 3) shimmer.
    #[inline]
    pub fn critical(&self) -> bool {
        self.hp_frac <= PHASE3_HP_FRAC
    }
}

// ============================================================================
// Hit shapes (the controller and the framework read these)
// ============================================================================

#[inline]
fn safe_facing(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-6 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// The head's world centre for this view. Normally [`HEAD_OFFSET`] along the
/// facing; while the bill is buried it is **parked at the strike point**, which
/// is what makes the punish window reachable.
#[inline]
pub fn head_center(view: &MagpieView) -> Vec2 {
    let f = safe_facing(view.facing);
    match view.state {
        MagpieState::Stuck | MagpieState::Peck => view.target,
        MagpieState::PeckWindup => view.pos + f * HEAD_OFFSET,
        _ => view.pos + f * HEAD_OFFSET,
    }
}

/// The bill tip (world). While stuck this is the strike point: the bill is the
/// line from the head to the planks.
#[inline]
pub fn beak_tip(view: &MagpieView) -> Vec2 {
    let f = safe_facing(view.facing);
    match view.state {
        MagpieState::Stuck | MagpieState::Peck => view.target,
        _ => view.pos + f * (HEAD_OFFSET + BEAK_LENGTH),
    }
}

/// Three body circles: tail root (behind), body, head. All grounded (the bird
/// flies low and can always be hit), which keeps a ground-only build able to
/// fight it through a strafing pass.
#[inline]
pub fn body_hit_circles(view: &MagpieView) -> [(Vec2, f32); 3] {
    let f = safe_facing(view.facing);
    [
        (view.pos - f * (TAIL_LENGTH * 0.45), 5.5),
        (view.pos, BODY_RADIUS * 0.82),
        (head_center(view), HEAD_RADIUS),
    ]
}

/// The lane a `StrafeWindup`/`StrafePass` is drawn along: `(from, to)`.
#[inline]
pub fn lane_segment(view: &MagpieView) -> (Vec2, Vec2) {
    (view.lane_start, view.target)
}

// ============================================================================
// Pose
// ============================================================================

/// Most a flying bird may lift off the deck while it is still drawn over its
/// own (grounded) hit shapes: the body radius is ~7 px, so a few pixels of lift
/// keeps the sprite on the circles the rat can actually hit.
const HIT_LIFT_MAX: f32 = 3.0;

#[derive(Clone, Copy)]
struct Pose {
    /// Screen-space body centre (already lifted by the flight altitude).
    body: Vec2,
    /// Screen-space ground footprint centre.
    ground: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    /// Wing spread 0 (folded) .. 1 (full span).
    spread: f32,
    /// Wing sweep: `>0` swept forward (braking), `<0` swept back (on the pass).
    sweep: f32,
    /// Wing beat clock.
    flap: f32,
    /// The head is drawn at `head` (screen).
    head: Vec2,
    /// The head is drawn at the strike point (bill buried).
    buried: bool,
    /// Head-up/head-down lean (positive = the head is down and forward).
    head_lunge: f32,
    /// Body stretch toward the strike.
    stretch: f32,
    /// Screen y lift of the whole bird.
    altitude: f32,
    /// Tail-up angle (`-1..1`).
    tail_lift: f32,
    /// Feather ruffle 0..1 (the phase-change pose).
    ruffle: f32,
    /// Shake jitter in px.
    jitter: Vec2,
    /// Fade out (death).
    alpha: f32,
    /// Death progress 0..1 (a limp, keeling bird).
    death: f32,
    hit: f32,
    /// The bird out of its hold, running.
    hunting: bool,
    /// Phase-2/3 plumage (the white wing patch spreads, the tail iridesces).
    enraged: f32,
    critical: f32,
    anim: f32,
    state: MagpieState,
    progress: f32,
    accent: Rgb,
    /// Screen-space strike/lane/crate target.
    target: Vec2,
    /// Screen-space lane start.
    lane_start: Vec2,
    sweep_side: f32,
}

impl Pose {
    fn from_view(view: &MagpieView, cam: Vec2, accent: Rgb) -> Self {
        let f = safe_facing(view.facing);
        let p = f.perp();
        let prog = view.progress.clamp(0.0, 1.0);
        let hit = view.hit_flash.clamp(0.0, 1.0);
        let hp = view.hp_frac.clamp(0.0, 1.0);
        let enraged = if hp <= PHASE2_HP_FRAC {
            let t = ((PHASE2_HP_FRAC - hp) / (PHASE2_HP_FRAC - PHASE3_HP_FRAC)).clamp(0.0, 1.0);
            0.45 + 0.55 * t
        } else {
            0.0
        };
        let pulse = 0.5 + 0.5 * (view.anim * 8.5).sin();
        let critical = if hp <= CRITICAL_HP_FRAC {
            0.5 + 0.5 * pulse
        } else {
            0.0
        };

        let ground = view.pos - cam;
        let spread: f32;
        let mut sweep = 0.0f32;
        let mut flap = 0.0f32;
        let mut altitude = 0.0f32;
        let mut head_lunge = 0.0f32;
        let mut stretch = 0.0f32;
        let tail_lift: f32;
        let mut ruffle = 0.0f32;
        let mut jitter = Vec2::ZERO;
        let mut alpha = 1.0f32;
        let mut death = 0.0f32;
        let mut buried = false;

        match view.state {
            MagpieState::Perch => {
                // A bird's walk: the head stabs forward and back with each hop.
                let bob = (view.anim * 3.6).sin();
                head_lunge = 0.30 + 0.22 * bob;
                tail_lift = 0.10 * bob;
                spread = if view.hunting { 0.28 } else { 0.0 };
                flap = if view.hunting {
                    (view.anim * 12.0).sin()
                } else {
                    0.0
                };
            }
            MagpieState::Ruffle => {
                // Feathers out: a hard shake, the wings half open, then settle.
                let t = (prog / 0.35).min(1.0);
                let settle = ((prog - 0.70) / 0.30).clamp(0.0, 1.0);
                ruffle = t * (1.0 - settle) * 1.0;
                spread = 0.55 * (1.0 - settle) + 0.10;
                let sh = (prog * 46.0).sin() * 2.2 * ruffle;
                jitter = p * sh;
                tail_lift = 0.35 * ruffle;
                head_lunge = -0.25 * ruffle;
            }
            MagpieState::PeckWindup => {
                // Draw the head back and coil: the reverse of the strike.
                let coil = (prog / PECK_LOCK_PROGRESS).min(1.0);
                head_lunge = -0.85 * coil;
                stretch = -0.35 * coil;
                tail_lift = -0.30 * coil;
                let tremble = if prog >= PECK_LOCK_PROGRESS {
                    (view.anim * 42.0).sin() * 0.8
                } else {
                    0.0
                };
                jitter = p * tremble;
                spread = 0.18 * coil;
            }
            MagpieState::Peck => {
                // The stab: the body follows the head down the line.
                let t = (prog / 0.45).min(1.0);
                head_lunge = 1.0;
                stretch = 1.1 * t;
                tail_lift = 0.55 * t;
                spread = 0.35;
                buried = true;
            }
            MagpieState::Stuck => {
                // The bill is in the plank: the bird pulls, the wings work.
                let tug = (view.anim * 9.0).sin();
                head_lunge = 0.85;
                stretch = 0.55;
                tail_lift = 0.65 + 0.18 * tug;
                spread = 0.62 + 0.16 * tug;
                flap = (view.anim * 17.0).sin();
                jitter = p * (tug * 1.3);
                buried = true;
            }
            MagpieState::Takeoff => {
                let t = (prog / 0.7).min(1.0);
                altitude = 12.0 * t * t;
                spread = t;
                flap = (view.anim * 22.0).sin();
                tail_lift = 0.4 * t;
                head_lunge = 0.15;
            }
            MagpieState::StrafeWindup => {
                altitude = 12.0;
                spread = 1.0;
                flap = (view.anim * 18.0).sin();
                tail_lift = 0.2;
                sweep = -0.25;
                head_lunge = 0.45;
            }
            MagpieState::StrafePass => {
                altitude = 12.0;
                spread = 1.0;
                // Wings go flat and swept back once the bird commits.
                sweep = -0.75;
                flap = (view.anim * 10.0).sin() * 0.35;
                tail_lift = 0.0;
                head_lunge = 0.55;
            }
            MagpieState::SnatchDrop => {
                // Brake, wheel, let go.
                let brake = (prog / 0.45).min(1.0);
                altitude = 12.0 + 5.0 * (1.0 - brake);
                spread = 1.0;
                sweep = 0.55 * brake;
                flap = (view.anim * 20.0).sin() * brake;
                tail_lift = -0.25 * brake;
                head_lunge = 0.35;
            }
            MagpieState::Land => {
                let t = (prog / 0.6).min(1.0);
                altitude = 12.0 * (1.0 - t) * (1.0 - t);
                spread = (1.0 - t) * 0.9;
                flap = (view.anim * 14.0).sin() * (1.0 - t);
                let squash = (1.0 - t).powi(2);
                jitter = Vec2::new(0.0, squash * 1.4);
                tail_lift = 0.25;
            }
            MagpieState::GustWindup => {
                // Wings back, chest out, claws in the planks.
                let c = (prog).min(1.0);
                spread = 0.75 * c;
                sweep = -0.6 * c;
                tail_lift = -0.35 * c;
                head_lunge = -0.35 * c;
                let gather = (view.anim * 30.0).sin() * 0.5 * c;
                jitter = p * gather;
            }
            MagpieState::Gust => {
                // Two full wing-beats per pulse, then a calm, spread glide.
                let pushing = gust_pulse_active(view.state, prog);
                let beat = if pushing {
                    (view.anim * 34.0).sin()
                } else {
                    0.15 * (view.anim * 4.0).sin()
                };
                spread = if pushing { 0.95 } else { 0.55 };
                flap = beat;
                sweep = if pushing { 0.45 * beat } else { -0.1 };
                tail_lift = if pushing { 0.45 + 0.2 * beat } else { -0.15 };
                head_lunge = if pushing { 0.65 } else { 0.2 };
                let shake = if pushing { beat * 1.1 } else { 0.0 };
                jitter = Vec2::new(0.0, shake);
            }
            MagpieState::Death => {
                death = prog;
                buried = false;
                if prog < 0.40 {
                    let t = prog / 0.40;
                    let spasm = (t * 30.0).sin() * 1.8 * (1.0 - t * 0.3);
                    spread = 0.4 + 0.6 * t;
                    flap = spasm;
                    tail_lift = 0.5 * t;
                    head_lunge = 0.4 * t;
                    jitter = p * spasm;
                } else {
                    let t = ((prog - 0.40) / 0.42).min(1.0);
                    let ease = 1.0 - (1.0 - t) * (1.0 - t);
                    spread = 1.0 - 0.45 * ease;
                    flap = (1.0 - t) * 0.4;
                    tail_lift = 0.5 - 0.9 * ease;
                    head_lunge = 0.4 + 0.6 * ease;
                    jitter = p * (1.0 - t) * 0.8;
                    if prog > 0.86 {
                        alpha = 1.0 - ((prog - 0.86) / 0.14) * 0.42;
                    }
                }
            }
        }

        // Death takes the whole bird over onto its side: rotating the pose's
        // own axes rolls the body, the wings and the head together, which is
        // what makes the defeat read as a fall rather than a freeze.
        let (f, p) = if view.state == MagpieState::Death && death > 0.0 {
            let roll = death * 0.62;
            let (s, c) = roll.sin_cos();
            (
                Vec2::new(f.x * c - f.y * s, f.x * s + f.y * c),
                Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c),
            )
        } else {
            (f, p)
        };
        let fl = f.dot(LIGHT);
        let pl = p.dot(LIGHT);

        // The head follows the strike only while the bill is *in* the deck;
        // in death it slumps off the neck where the bird fell.
        let head_world = if matches!(view.state, MagpieState::Peck | MagpieState::Stuck) {
            view.target
        } else {
            view.pos + f * HEAD_OFFSET
        };
        let slump = if view.state == MagpieState::Death {
            p * (death * 6.5) - f * (death * 2.5)
        } else {
            Vec2::ZERO
        };
        // **The bird is drawn where its hit shapes are.** Its shapes stay on
        // the deck in every state (`Boss::hit_shapes`: "a ground-only build
        // must always be able to fight it"), so the sprite may only lift as far
        // as those deck circles still reach — otherwise the rat was shooting
        // the planks a dozen pixels under a floating bird. The flight still
        // reads: the shadow shrinks and softens with the full `altitude`
        // (below), the wings spread and the body leans.
        let lift = altitude.min(HIT_LIFT_MAX);
        let head = head_world - cam + Vec2::new(0.0, -lift) + slump;

        Self {
            body: ground + jitter + Vec2::new(0.0, -lift),
            ground,
            f,
            p,
            fl,
            pl,
            spread,
            sweep,
            flap,
            head,
            buried,
            head_lunge,
            stretch,
            altitude,
            tail_lift,
            ruffle,
            jitter,
            alpha,
            death,
            hit,
            hunting: view.hunting,
            enraged,
            critical,
            anim: view.anim,
            state: view.state,
            progress: prog,
            accent,
            target: view.target - cam,
            lane_start: view.lane_start - cam,
            sweep_side: if view.sweep < 0.0 { -1.0 } else { 1.0 },
        }
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.body.x;
        let dy = y as f32 - self.body.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.body + self.f * u + self.p * v
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        (nu * self.fl + nv * self.pl).clamp(-1.0, 1.0)
    }
}

// ============================================================================
// Small pixel helpers (stack-only, bounds-checked by clamp_bbox)
// ============================================================================

#[inline]
fn clamp_bbox(pb: &PixelBuffer, center: Vec2, reach: f32) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0 || pb.h == 0 {
        return None;
    }
    let rr = reach.ceil() as i32;
    let cx = center.x.round() as i32;
    let cy = center.y.round() as i32;
    let x0 = (cx - rr).max(0);
    let y0 = (cy - rr).max(0);
    let x1 = (cx + rr).min(pb.w as i32 - 1);
    let y1 = (cy + rr).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return None;
    }
    Some((x0, x1, y0, y1))
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x >= pb.w as i32 || y >= pb.h as i32 {
        return;
    }
    let ia = a.clamp(0.0, 1.0);
    let i = y as usize * pb.w + x as usize;
    let d = pb.pixels()[i];
    pb.pixels_mut()[i] = mix(d, c, ia);
}

/// Soft round dot (used by the ground telegraphs).
#[inline]
fn soft_dot(pb: &mut PixelBuffer, at: Vec2, radius: f32, c: Rgb, a: f32) {
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, at, radius + 1.0) else {
        return;
    };
    let r2 = radius * radius;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - at.x;
            let dy = y as f32 + 0.5 - at.y;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 + 1.0 {
                continue;
            }
            let t = (1.0 - (d2.sqrt() - radius + 1.0)).clamp(0.0, 1.0);
            blend_px(pb, x, y, c, a * t);
        }
    }
}

/// Cheap 2-D value hash (procedural feather noise).
#[inline]
fn hash2(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x27D4_EB2F) ^ (iy as u32).wrapping_mul(0x1656_67B1);
    h ^= seed.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

#[inline]
fn global_vein(t: f32) -> f32 {
    // Linear vein (not `t.sin()`): dark in the seam, bright on the vane.
    (t * PI).sin()
}

#[inline]
fn hash_blocky(scale: f32, u: f32, v: f32, seed: u32) -> f32 {
    let s = scale.max(0.5);
    let rate = 1.05 / s.sqrt() * 0.96;
    let ix = (u * rate).round() as i32;
    let iy = (v * rate).round() as i32;
    let stagger = (iy & 1) as f32 * 0.5;
    let fx = (u * rate + stagger).fract();
    let fy = (v * rate).fract();
    let blob = |cx: i32, cy: i32| -> f32 {
        let dx = ((u * rate + stagger) - cx as f32).abs();
        let dy = ((v * rate) - cy as f32).abs();
        let d = (dx * dx + dy * dy).sqrt();
        (1.0 - d * 1.6).clamp(0.0, 1.0) * hash2(cx, cy, seed)
    };
    let a = blob(ix, iy);
    let b = blob(ix + 1, iy);
    let c = blob(ix, iy + 1);
    let d = blob(ix + 1, iy + 1);
    let wx = ((fx - ix as f32) + 0.5).clamp(0.0, 1.0);
    let wy = ((fy - iy as f32) + 0.5).clamp(0.0, 1.0);
    (a * (1.0 - wx) + b * wx) * (1.0 - wy) + (c * (1.0 - wx) + d * wx) * wy
}

#[inline]
fn ellipse_sdf(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let du = (u - cu) / ru;
    let dv = (v - cv) / rv;
    (du * du + dv * dv).sqrt() - 1.0
}

#[inline]
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let pax = u - ax;
    let pay = v - ay;
    let bax = bx - ax;
    let bay = by - ay;
    let denom = bax * bax + bay * bay;
    let h = if denom <= 1e-6 {
        0.0
    } else {
        ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0)
    };
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    let r = r0 + (r1 - r0) * h;
    (dx * dx + dy * dy).sqrt() - r
}

#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.01 {
        c
    } else {
        mix(c, (255, 245, 235), hit * 0.6)
    }
}

#[inline]
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    if lum < 0.0 {
        mix(mid, shadow, (-lum).clamp(0.0, 1.0))
    } else {
        mix(mid, hi, lum.clamp(0.0, 1.0))
    }
}

struct Plumage {
    /// Blue-black back.
    back: Rgb,
    /// Near-black coverts.
    covert: Rgb,
    /// The white scapular/patch.
    white: Rgb,
    /// Iridescent blue-green (tail, back sheen).
    sheen: Rgb,
    /// Bill + legs.
    bill: Rgb,
    /// Head shadow.
    shadow: Rgb,
    /// Rim light.
    rim: Rgb,
}

fn plumage(pose: &Pose) -> Plumage {
    let dead = ((pose.death - 0.32) / 0.68).clamp(0.0, 1.0);
    let e = pose.enraged;
    let mut back = mix((22, 26, 38), (16, 18, 30), e);
    let mut covert = mix((30, 32, 44), (24, 24, 36), e);
    let mut white = mix((226, 228, 236), (244, 240, 226), e * 0.7);
    let mut sheen = mix((40, 96, 120), (70, 40, 116), e * 0.65);
    let bill = mix((38, 36, 40), (28, 26, 30), e);
    let mut shadow = (12, 13, 20);
    let rim = if pose.hit > 0.55 {
        (150, 128, 96)
    } else {
        (96, 122, 148)
    };
    if dead > 0.0 {
        back = mix(back, (72, 70, 74), dead * 0.62);
        covert = mix(covert, (86, 84, 82), dead * 0.58);
        white = mix(white, (150, 148, 142), dead * 0.5);
        sheen = mix(sheen, (60, 60, 66), dead * 0.7);
        shadow = (34, 34, 38);
    }
    Plumage {
        back,
        covert,
        white,
        sheen,
        bill,
        shadow,
        rim,
    }
}

/// The folded-wing silhouette: a broad-shouldered blade along the flank that
/// opens into a swept, tapering span when the bird spreads.
///
/// A magpie's wing in flight: leading edge nearly straight from the shoulder,
/// trailing edge cut back, primaries splayed at the wrist. The root is the
/// widest part (a bird's wing is an arm, not a feather duster), which is what
/// stops the spread pose reading as a pair of flames.
#[inline]
fn wing_sdf(pose: &Pose, side: f32, u: f32, v: f32) -> f32 {
    let spread = pose.spread.clamp(0.0, 1.0);
    let root = (5.6, side * 4.6);
    // Folded the tip lies back along the tail; spread it swings out and
    // slightly forward of the shoulder.
    let tip = (
        3.0 - 20.0 * (1.0 - spread) + 6.0 * pose.sweep,
        side * (5.2 + 15.2 * spread),
    );
    let chord = 4.6 - 1.6 * spread;
    let d = sd_taper(u, v, root, tip, chord, 1.15);
    // The wrist bump (the carpal joint a bird folds its wing over).
    let wrist_u = root.0 + (tip.0 - root.0) * 0.30;
    let wrist_v = root.1 + (tip.1 - root.1) * 0.30;
    let bump = ellipse_sdf(u, v, wrist_u, wrist_v, 4.2, 4.0) * 7.5;
    d.min(bump)
}

/// The body silhouette (tail + torso + head), as an SDF in local units.
fn body_sdf(pose: &Pose, u: f32, v: f32) -> (f32, u8) {
    // Region ids: 0 tail, 1 body, 2 head.
    let mut best = f32::MAX;
    let mut id = 1u8;

    // Tail: a long flat fan behind, lifted by `tail_lift`.
    let lift = pose.tail_lift;
    let tail = sd_taper(
        u,
        v,
        (-4.0, 0.0),
        (-(4.0 + TAIL_LENGTH), lift * 3.4),
        3.2,
        3.9,
    );
    if tail < best {
        best = tail;
        id = 0;
    }

    // Torso: a teardrop, wider at the shoulders.
    let torso = ellipse_sdf(u, v, -0.5, 0.0, 8.6, 6.3);
    if torso < best {
        best = torso;
        id = 1;
    }

    // Head (drawn once, at its true centre, while the bill is buried; the neck
    // taper in `draw_head` joins it back to the shoulders).
    if !pose.buried {
        let hx = HEAD_OFFSET + pose.stretch * 3.4;
        let hy = pose.head_lunge * 1.1;
        let head = ellipse_sdf(u, v, hx, hy, 6.1, 5.0);
        if head < best {
            best = head;
            id = 2;
        }
        // The nape: a shaggy neck joining the head to the shoulders.
        let nape = sd_taper(u, v, (5.0, 0.0), (hx - 2.0, hy), 4.4, 3.6);
        if nape < best {
            best = nape;
            id = 2;
        }
    }

    // Ruff: the shoulder fluff that a ruffled bird puffs out.
    if pose.ruffle > 0.02 {
        let ruff = ellipse_sdf(u, v, 4.0, 0.0, 6.4 + 2.4 * pose.ruffle, 6.2 + 1.6 * pose.ruffle);
        if ruff < best {
            best = ruff;
            id = 1;
        }
    }
    (best, id)
}

// ============================================================================
// Ground layer
// ============================================================================

/// The boss's ground layer: the warnings the rat reads before anything lands,
/// plus the bird's own drop shadow.
pub fn draw_magpie_ground(pb: &mut PixelBuffer, view: &MagpieView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let vw = pb.w as f32;
    let vh = pb.h as f32;
    let body_vis = pose.ground.x >= -CULL_MARGIN
        && pose.ground.x <= vw + CULL_MARGIN
        && pose.ground.y >= -CULL_MARGIN
        && pose.ground.y <= vh + CULL_MARGIN;
    let target_vis = pose.target.x >= -64.0
        && pose.target.x <= vw + 64.0
        && pose.target.y >= -64.0
        && pose.target.y <= vh + 64.0;
    if !body_vis && !target_vis {
        return;
    }

    match view.state {
        MagpieState::PeckWindup => draw_beak_line(pb, &pose, false),
        MagpieState::Peck => draw_beak_line(pb, &pose, true),
        MagpieState::Stuck => draw_beak_line(pb, &pose, true),
        MagpieState::StrafeWindup => draw_lane(pb, &pose, false),
        MagpieState::StrafePass => draw_lane(pb, &pose, true),
        MagpieState::SnatchDrop => {
            draw_crate_square(pb, &pose);
            draw_lane(pb, &pose, true);
        }
        MagpieState::GustWindup => draw_gust(pb, &pose, 0.35),
        MagpieState::Gust => {
            let push = gust_pulse_active(view.state, view.progress);
            draw_gust(pb, &pose, if push { 1.0 } else { 0.22 });
        }
        MagpieState::Death
            if pose.death > 0.30 => {
                let t = ((pose.death - 0.30) / 0.40).clamp(0.0, 1.0);
                let fade = (1.0 - ((pose.death - 0.45) / 0.55).clamp(0.0, 1.0)) * 0.5;
                soft_dot(
                    pb,
                    pose.ground,
                    DEATH_RING_RADIUS * (0.5 + 0.6 * t),
                    (168, 156, 138),
                    fade * 0.5,
                );
            }
        _ => {}
    }

    if body_vis {
        draw_shadow(pb, &pose);
    }
}

/// The bird's own shadow: crisp on the deck, soft and small while it flies.
fn draw_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    let fly = (pose.altitude / 12.0).clamp(0.0, 1.4);
    let scale_ = 1.0 - 0.22 * fly;
    let alpha = (0.44 - 0.12 * fly) * pose.alpha;
    let centre = pose.ground + Vec2::new(2.0, 2.6);
    let reach = 26.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, centre, reach) else {
        return;
    };
    let ink: Rgb = (4, 5, 8);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let (d, _) = body_sdf(pose, u, v);
            // Wings too: a flying bird's shadow is the whole silhouette.
            let w = if pose.spread > 0.15 {
                wing_sdf(pose, 1.0, u, v).min(wing_sdf(pose, -1.0, u, v))
            } else {
                f32::MAX
            };
            let d = d.min(w);
            if d < 1.9 {
                let soft = ((1.9 - d) / 2.9).clamp(0.0, 1.0);
                blend_px(pb, x, y, ink, alpha * soft * scale_);
            }
        }
    }
}

/// Dashed line helper (ground telegraphs).
fn dashed_line(pb: &mut PixelBuffer, a: Vec2, b: Vec2, dash: f32, gap: f32, c: Rgb, alpha: f32) {
    let d = b - a;
    let len = d.len();
    if len <= 1.0 {
        return;
    }
    let dir = d * (1.0 / len);
    let mut t = 0.0f32;
    while t < len {
        let t1 = (t + dash).min(len);
        let p0 = a + dir * t;
        let p1 = a + dir * t1;
        let steps = ((t1 - t).ceil() as i32).max(1);
        for i in 0..=steps {
            let p = p0 + (p1 - p0) * (i as f32 / steps as f32);
            blend_px(pb, p.x.round() as i32, p.y.round() as i32, c, alpha);
        }
        t = t1 + gap;
    }
}

/// The marked strike square at the end of a beak line.
fn strike_square(pb: &mut PixelBuffer, at: Vec2, half: f32, c: Rgb, alpha: f32, solid: bool) {
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    for dy in -h..=h {
        for dx in -h..=h {
            let edge = dx.abs() == h || dy.abs() == h;
            let corner = dx.abs() >= h - 1 && dy.abs() >= h - 1;
            if edge && (solid || corner || (dx.abs() + dy.abs()) % 2 == 0) {
                blend_px(pb, cx + dx, cy + dy, c, alpha);
            } else if solid && (dx * dx + dy * dy) as f32 <= (half * half) {
                blend_px(pb, cx + dx, cy + dy, c, alpha * 0.35);
            }
        }
    }
}

/// The beak line: bill -> marked square, dashed while it can still be walked
/// out of, solid while it is live.
fn draw_beak_line(pb: &mut PixelBuffer, pose: &Pose, live: bool) {
    let bird = pose.ground + pose.f * (HEAD_OFFSET + 2.0);
    let tip = pose.target;
    let pulse = 0.5 + 0.5 * (pose.anim * 15.0).sin();
    let warm = mix((255, 96, 52), (255, 208, 96), 0.35 + 0.45 * pulse);
    let col = if live {
        mix((255, 246, 226), warm, 0.35)
    } else {
        mix((150, 66, 40), warm, 0.5)
    };
    let alpha = if live { 0.95 } else { 0.55 + 0.25 * pulse };

    let d = tip - bird;
    let len = d.len();
    if len > 1.0 {
        let dir = d * (1.0 / len);
        // The corridor's two rails, then the centre hatch.
        let perp = dir.perp();
        let half = PECK_ACTIVE_HALF_WIDTH;
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let p = bird + dir * (i as f32);
            for s in [-1.0f32, 1.0] {
                let q = p + perp * (half * s);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha * 0.8);
            }
            if i % 4 == 0 {
                blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, alpha * 0.45);
            }
        }
        if !live {
            dashed_line(
                pb,
                bird + dir * 3.0,
                tip - dir * 3.0,
                3.0,
                3.0,
                col,
                alpha * 0.7,
            );
        }
    }
    strike_square(pb, tip, 4.0, col, alpha, live);
}

/// The strafing lane: rails along the pass and a hatched centre, with the far
/// (safe) side marked.
fn draw_lane(pb: &mut PixelBuffer, pose: &Pose, live: bool) {
    let a = pose.lane_start;
    let b = pose.target;
    let d = b - a;
    let len = d.len();
    if len <= 1.0 {
        return;
    }
    let dir = d * (1.0 / len);
    let perp = dir.perp();
    let pulse = 0.5 + 0.5 * (pose.anim * 13.0).sin();
    let col = if live {
        mix((255, 250, 232), (255, 132, 64), 0.35)
    } else {
        mix((146, 74, 44), (255, 168, 84), 0.4 + 0.35 * pulse)
    };
    let alpha = if live { 0.9 } else { 0.5 + 0.25 * pulse };
    let half = STRAFE_LANE_HALF_WIDTH;

    let steps = len.ceil() as i32;
    for i in 0..=steps {
        let p = a + dir * (i as f32);
        for s in [-1.0f32, 1.0] {
            let q = p + perp * (half * s);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
        }
        // Hatch: a chevron every 6 px pointing the way the wings go.
        if i % 6 == 0 {
            for k in -2..=2 {
                let q = p + perp * (k as f32 * 3.0);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha * 0.30);
            }
        }
    }
    // The start and end lips.
    strike_square(pb, a, 3.0, col, alpha, live);
    strike_square(pb, b, 3.0, col, alpha, live);
}

/// The crate's marked landing square: a cornered box that fills in as the crate
/// falls, then the burst ring.
fn draw_crate_square(pb: &mut PixelBuffer, pose: &Pose) {
    let t = (pose.progress / CRATE_LAND_PROGRESS).clamp(0.0, 1.0);
    let at = pose.target;
    let half = CRATE_HALF;
    let pulse = 0.5 + 0.5 * (pose.anim * 18.0).sin();
    let col = mix((255, 214, 120), (255, 96, 64), t * 0.7 + 0.15 * pulse);
    let alpha = 0.45 + 0.5 * t;
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    for dy in -h..=h {
        for dx in -h..=h {
            let ax = dx.abs();
            let ay = dy.abs();
            let rim = ax == h || ay == h;
            // Corner ticks, then the fill creeping out from the centre.
            let corner = ax >= h - 2 && ay >= h - 2;
            if rim && (corner || (ax + ay) % 3 != 0) {
                blend_px(pb, cx + dx, cy + dy, col, alpha);
            } else if !rim && (dx * dx + dy * dy) as f32 <= (half * t) * (half * t) {
                blend_px(pb, cx + dx, cy + dy, col, alpha * 0.28);
            }
        }
    }
    if pose.progress >= CRATE_LAND_PROGRESS {
        let burst = ((pose.progress - CRATE_LAND_PROGRESS) / (1.0 - CRATE_LAND_PROGRESS))
            .clamp(0.0, 1.0);
        soft_dot(
            pb,
            at,
            half + 10.0 * burst,
            (255, 232, 186),
            (1.0 - burst) * 0.55,
        );
    }
}

/// The wing-beat wind: long streaks down the deck, bright in a pulse and faint
/// in the calm gap (the gap is the rat's window and it has to look like one).
fn draw_gust(pb: &mut PixelBuffer, pose: &Pose, strength: f32) {
    let origin = pose.ground;
    let axis = GUST_AXIS;
    let perp = axis.perp();
    let reach = GUST_REACH;
    let span = 150.0;
    // Streak lanes across the field, each with its own phase.
    let lanes = 13;
    for k in 0..lanes {
        let lane = (k as f32 / (lanes - 1) as f32 - 0.5) * 2.0 * span;
        let phase = (pose.anim * 2.6 + k as f32 * 0.37).fract();
        let len = 34.0 + 40.0 * hash2(k, 3, 7);
        let start = origin + perp * lane + axis * (phase * (reach - len) * strength.max(0.2));
        let alpha = strength * (0.20 + 0.42 * (phase * PI).sin());
        let col = if strength > 0.5 {
            (226, 238, 250)
        } else {
            (128, 146, 168)
        };
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let p = start + axis * (i as f32);
            blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, alpha);
        }
    }
    if strength > 0.5 {
        // The leading edge of the beat, a bright line just downwind of the bill.
        let edge = origin + axis * (16.0 + 26.0 * ((pose.anim * 6.4).fract()));
        let half = 78.0;
        let steps = (half * 2.0) as i32;
        for i in 0..=steps {
            let q = edge + perp * ((i as f32 / steps as f32) * 2.0 - 1.0) * half;
            blend_px(
                pb,
                q.x.round() as i32,
                q.y.round() as i32,
                (255, 252, 240),
                strength * 0.5,
            );
        }
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// Draw the bird itself.
pub fn draw_magpie(pb: &mut PixelBuffer, view: &MagpieView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let margin = CULL_MARGIN + 10.0;
    if pose.body.x < -margin
        || pose.body.x > pb.w as f32 + margin
        || pose.body.y < -margin
        || pose.body.y > pb.h as f32 + margin
    {
        return;
    }
    let pal = plumage(&pose);
    draw_wings(pb, &pose, &pal);
    draw_tail(pb, &pose, &pal);
    draw_torso(pb, &pose, &pal);
    draw_head(pb, &pose, &pal);
    match pose.state {
        MagpieState::Gust | MagpieState::GustWindup => draw_gust_wake(pb, &pose),
        MagpieState::Stuck => draw_buried_glint(pb, &pose),
        MagpieState::Ruffle => draw_fluff_motes(pb, &pose),
        MagpieState::Death => draw_death_molt(pb, &pose, &pal),
        _ => {}
    }
}

fn draw_wings(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    for side in [-1.0f32, 1.0] {
        let reach = 28.0;
        let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
            continue;
        };
        let flap = pose.flap * side * (if side > 0.0 { 1.0 } else { -1.0 });
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = pose.local(x, y);
                let d = wing_sdf(pose, side, u, v);
                if d > 0.0 {
                    continue;
                }
                // Shade across the wing: primaries dark, coverts iridescent,
                // the white patch on the mid-wing.
                let span = ((v * side) / (5.0 + 17.0 * pose.spread)).clamp(0.0, 1.2);
                let along = ((u + 20.0) / 34.0).clamp(0.0, 1.0);
                let feather = hash_blocky(3.0, u * 0.6 + side * 4.0, v * 0.9, 21);
                let lum = pose.lit(0.25, side * 0.35) * 0.55 + flap * 0.18;
                let mut c = shade3(pal.shadow, pal.covert, pal.back, lum);
                // The magpie's white scapular band: a wedge on the inner half
                // of the wing, dark coverts either side of it.
                let patch = smoothstep(0.22, 0.34, span)
                    * (1.0 - smoothstep(0.52, 0.72, span))
                    * (1.0 - smoothstep(0.55, 0.85, along));
                c = mix(c, pal.white, patch * (0.55 + 0.45 * feather));
                // White secondaries on the outer wing with dark tips: the
                // flicker a magpie shows in a bank.
                let secondary = smoothstep(0.42, 0.60, span) * (1.0 - smoothstep(0.72, 0.92, along));
                c = mix(c, pal.white, secondary * 0.35 * feather);
                // Iridescent trailing edge.
                if along > 0.62 {
                    c = mix(c, pal.sheen, (along - 0.62) * 1.6 * 0.5);
                }
                // Rim light along the leading edge.
                if d > -1.4 {
                    c = mix(c, pal.rim, 0.35);
                }
                blend_px(pb, x, y, apply_hit(c, pose.hit), pose.alpha);
            }
        }
        // Primary "fingers", splayed at the wrist once the wing is open.
        if pose.spread > 0.55 {
            let tip = pose.place(
                3.0 - 20.0 * (1.0 - pose.spread) + 6.0 * pose.sweep,
                side * (5.2 + 15.2 * pose.spread),
            );
            for k in 0..4 {
                let t = (k as f32 + 1.0) / 5.0;
                let base = tip + pose.f * ((t - 0.5) * 5.0) - pose.p * (side * t * 2.0);
                let end = base - pose.f * 2.0 + pose.p * (side * 2.2);
                draw_segment(pb, base, end, 0.7, mix(pal.covert, pal.white, 0.35), pose.alpha * 0.9);
            }
        }
    }
}

fn draw_tail(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 34.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            if u > -3.0 {
                continue;
            }
            let d = sd_taper(
                u,
                v,
                (-4.5, 0.0),
                (-(4.5 + TAIL_LENGTH), pose.tail_lift * 3.0),
                4.4,
                3.1,
            );
            if d > 0.0 {
                continue;
            }
            // The tail's iridescent green-blue and the pale band a magpie
            // carries at the very end of it.
            let along = ((-u - 4.0) / TAIL_LENGTH).clamp(0.0, 1.0);
            let band = hash2((v * 0.8) as i32, (u * 0.4) as i32, 5);
            let lum = pose.lit(-0.4, 0.0) * 0.5;
            let mut c = shade3(pal.shadow, pal.covert, pal.sheen, lum);
            c = mix(c, pal.sheen, (0.30 + 0.35 * band) * (1.0 - along) * 0.6);
            // The band is a stripe, two feathers wide, not a cuff.
            let stripe = smoothstep(0.80, 0.86, along) * (1.0 - smoothstep(0.93, 0.99, along));
            c = mix(c, pal.white, stripe * 0.55);
            if along < 0.08 {
                c = mix(c, pal.back, 0.5);
            }
            if d > -1.2 {
                c = mix(c, pal.rim, 0.24);
            }
            blend_px(pb, x, y, apply_hit(c, pose.hit), pose.alpha);
        }
    }
}

fn draw_torso(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 22.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let (d, id) = body_sdf(pose, u, v);
            if id != 1 || d > 0.0 {
                continue;
            }
            let n = (d + 1.0).clamp(0.0, 1.0);
            let lum = pose.lit(u / 12.0, v / 8.0);
            let feather = hash_blocky(2.4, u, v, 11);
            let mut c = shade3(pal.shadow, pal.back, mix(pal.sheen, pal.white, 0.25), lum * 0.8);
            // Back sheen: an iridescent wash over the mantle.
            let mantle = smoothstep(-9.0, 6.0, u);
            c = mix(c, pal.sheen, mantle * (0.18 + 0.22 * feather) * (0.6 + 0.4 * pose.enraged));
            // Pale flank.
            if v.abs() > 3.4 {
                c = mix(c, pal.white, (v.abs() - 3.4).clamp(0.0, 1.0) * 0.30);
            }
            // Belly plates / breast shading.
            if u > 2.0 {
                c = mix(c, pal.covert, 0.25 * feather);
            }
            if d > -1.3 {
                c = mix(c, pal.rim, 0.22 + 0.25 * n * 0.0);
            }
            blend_px(pb, x, y, apply_hit(c, pose.hit), pose.alpha);
        }
    }
}

fn draw_head(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    // The neck: always drawn, because the pose can take the head a long way
    // from the shoulders (a buried bill, a death slump) and a detached head
    // reads as a bug, not as a bird.
    let shoulder = pose.body + pose.f * 3.0;
    let join = pose.head - pose.f * 0.5;
    let d = join - shoulder;
    let len = d.len();
    if len > 2.0 {
        let dir = d * (1.0 / len);
        let perp = dir.perp();
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let w = 4.4 - 1.0 * t;
            let p = shoulder + dir * (i as f32);
            for k in -(w as i32)..=(w as i32) {
                let q = p + perp * (k as f32);
                let edge = (k as f32).abs() > w - 1.0;
                let c = if edge { pal.shadow } else { pal.back };
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, c, pose.alpha);
            }
        }
    }
    let reach = 26.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.head, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5) - pose.head.x;
            let dy = (y as f32 + 0.5) - pose.head.y;
            let d = (dx * dx + dy * dy).sqrt() - HEAD_RADIUS;
            if d > 0.0 {
                continue;
            }
            let lum = ((dx / HEAD_RADIUS) * pose.fl + (dy / HEAD_RADIUS) * pose.pl).clamp(-1.0, 1.0);
            // A magpie's head is the blackest part of it: the sheen only.
            let mut c = shade3(pal.shadow, pal.back, mix(pal.covert, pal.sheen, 0.55), lum * 0.75);
            if d > -1.2 {
                c = mix(c, pal.rim, 0.22);
            }
            // The shaggy nape catches the key light.
            if dx < -1.0 {
                c = mix(c, pal.covert, 0.35);
            }
            blend_px(pb, x, y, apply_hit(c, pose.hit), pose.alpha);
        }
    }

    // Eyes: two hot pinpricks either side of the bill; a bird's eye never blinks
    // first.
    let roll = if pose.buried { 0.4 } else { 1.0 };
    for side in [-1.0f32, 1.0] {
        let eye = pose.head + pose.f * 1.9 + pose.p * (side * 3.2);
        let col = if pose.death > 0.4 {
            (96, 88, 82)
        } else {
            mix((255, 210, 96), (255, 120, 60), 0.25 + 0.3 * (pose.anim * 3.0).sin().abs())
        };
        blend_px(
            pb,
            eye.x.round() as i32,
            eye.y.round() as i32,
            col,
            0.95 * roll * pose.alpha,
        );
        blend_px(
            pb,
            eye.x.round() as i32,
            eye.y.round() as i32 - 1,
            (255, 255, 240),
            0.5 * roll * pose.alpha,
        );
    }

    // The bill: a straight taper from the face, or a stub vanishing into the
    // plank while it is buried (`Stuck`: the rest of it is under the deck).
    let face = pose.head + pose.f * (HEAD_RADIUS * 0.6);
    let tip = if pose.buried {
        pose.target + pose.f * 4.0
    } else {
        face + pose.f * BEAK_LENGTH
    };
    let d = tip - face;
    let len = d.len().max(1.0);
    let dir = if d.len_sq() > 1e-4 {
        d * (1.0 / len)
    } else {
        pose.f
    };
    let perp = dir.perp();
    let steps = len.ceil() as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        // A straight corvid bill: a solid wedge from the face to a chisel tip.
        let w = (1.0 - t * 0.85) * 2.8;
        let p = face + dir * (i as f32);
        let edge = w.floor() as i32;
        for k in -edge..=edge {
            let q = p + perp * (k as f32);
            let rr = (k as f32).abs();
            let c = if rr > w - 0.9 {
                mix(pal.bill, (18, 18, 22), 0.7)
            } else if rr < 0.6 {
                // The culmen: one pale ridge along the top of the bill.
                mix(pal.bill, (176, 172, 170), 0.55)
            } else {
                pal.bill
            };
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, c, pose.alpha);
        }
    }
}

fn draw_buried_glint(pb: &mut PixelBuffer, pose: &Pose) {
    // The buried head is the weak point: a rotating bracket of hot pixels so the
    // rat can see where to bite.
    let c = pose.head;
    let pulse = 0.55 + 0.45 * (pose.anim * 11.0).sin();
    for k in 0..8 {
        let a = k as f32 * (PI / 4.0) + pose.anim * 1.4;
        let dir = Vec2::new(a.cos(), a.sin());
        let p = c + dir * (HEAD_RADIUS + 2.2);
        blend_px(
            pb,
            p.x.round() as i32,
            p.y.round() as i32,
            (255, 236, 150),
            pulse * pose.alpha,
        );
    }
}

fn draw_gust_wake(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.spread < 0.3 {
        return;
    }
    let from = pose.body - pose.f * 2.0;
    for side in [-1.0f32, 1.0] {
        let tip = pose.place(-6.0, side * (5.0 + 17.0 * pose.spread));
        for k in 0..3 {
            let t = (k as f32 + 1.0) / 4.0;
            let a = from + (tip - from) * t;
            let b = a + GUST_AXIS * (5.0 + 4.0 * t) - pose.p * (side * 1.5);
            draw_segment(
                pb,
                a,
                b,
                0.8,
                mix((226, 240, 255), (150, 176, 204), t),
                0.45 * pose.alpha,
            );
        }
    }
}

fn draw_fluff_motes(pb: &mut PixelBuffer, pose: &Pose) {
    let n = 7;
    for k in 0..n {
        let seed = hash2(k, 0, 31);
        let a = k as f32 * 2.4 + pose.progress * 8.0 + seed * std::f32::consts::TAU;
        let r = 6.0 + 10.0 * pose.progress + seed * 6.0;
        let p = pose.body + Vec2::new(a.cos(), a.sin() * 0.7) * r;
        blend_px(
            pb,
            p.x.round() as i32,
            p.y.round() as i32,
            (234, 230, 220),
            (1.0 - pose.progress) * 0.6,
        );
    }
}

fn draw_death_molt(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let t = pose.death;
    if t < 0.18 {
        return;
    }
    let spread = ((t - 0.18) / 0.8).clamp(0.0, 1.0);
    for k in 0..10 {
        let seed = hash2(k, 1, 17);
        let ang = seed * std::f32::consts::TAU + t * 2.0;
        let r = 4.0 + 26.0 * spread * (0.4 + seed);
        let p = pose.body + Vec2::new(ang.cos(), ang.sin() * 0.75) * r;
        let c = if k % 3 == 0 { pal.white } else { pal.covert };
        blend_px(
            pb,
            p.x.round() as i32,
            p.y.round() as i32,
            c,
            (1.0 - spread) * 0.8,
        );
    }
}

// ============================================================================
// Shape helpers
// ============================================================================

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if (b - a).abs() < 1e-6 {
        return if x >= b { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A one-pixel-wide line of blended dots.
fn draw_segment(pb: &mut PixelBuffer, a: Vec2, b: Vec2, radius: f32, c: Rgb, alpha: f32) {
    let d = b - a;
    let len = d.len_sq().sqrt();
    let steps = len.ceil() as i32;
    if steps <= 0 {
        blend_px(pb, a.x.round() as i32, a.y.round() as i32, c, alpha);
        return;
    }
    let r = radius.max(0.5);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = a + d * t;
        if r <= 1.0 {
            blend_px(pb, p.x.round() as i32, p.y.round() as i32, c, alpha);
        } else {
            soft_dot(pb, p, r, c, alpha);
        }
    }
}

/// A small crate sprite (the thing the bird drops). `drop` is 1 when it is at
/// the bill and 0 when it has hit the deck.
fn draw_crate(pb: &mut PixelBuffer, at: Vec2, drop: f32, alpha: f32) {
    let half = CRATE_HALF - 2.0;
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    let pick = |dx: i32, dy: i32| -> Rgb {
        let edge = dx.abs() == h || dy.abs() == h;
        let strap = dx.abs() <= 1 || dy.abs() <= 1;
        if edge {
            (86, 62, 38)
        } else if strap {
            (58, 44, 30)
        } else if (dx + dy) % 5 == 0 {
            (156, 116, 68)
        } else {
            (128, 92, 54)
        }
    };
    for dy in -h..=h {
        for dx in -h..=h {
            let c = pick(dx, dy);
            blend_px(pb, cx + dx, cy + dy, c, alpha * (0.55 + 0.45 * drop));
        }
    }
    // Motion streak: a couple of pale pixels above the crate while it falls.
    if drop > 0.15 {
        for k in 0..3 {
            let y = cy - h - 2 - k;
            blend_px(pb, cx, y, (222, 216, 196), alpha * drop * (0.5 - 0.13 * k as f32));
        }
    }
}

/// The dropped crate, drawn as part of the boss's entity layer (the crate is
/// the bird's payload: nothing else on the deck owns it).
pub fn draw_magpie_crate(pb: &mut PixelBuffer, view: &MagpieView, cam: Vec2) {
    if view.state != MagpieState::SnatchDrop {
        return;
    }
    let at = view.target - cam;
    let t = (view.progress / CRATE_LAND_PROGRESS).clamp(0.0, 1.0);
    // The crate starts under the bill and falls to the square.
    let start = view.pos - cam + Vec2::new(0.0, -16.0);
    let fall = t * t * (3.0 - 2.0 * t);
    let p = start + (at - start) * fall;
    if view.progress >= CRATE_LAND_PROGRESS {
        return;
    }
    draw_crate(pb, p, 1.0 - t, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The bird is drawn where it can be hit, in every state.** Its hit
    /// shapes are the deck circles in `Boss::hit_shapes` (the design's "a
    /// ground-only build must always be able to fight it"), so a flying state
    /// must not lift the sprite out of them: the rat was shooting planks a
    /// dozen pixels under a floating bird.
    #[test]
    fn every_magpie_state_draws_its_body_over_its_hit_shapes() {
        let states = [
            MagpieState::Perch,
            MagpieState::Ruffle,
            MagpieState::PeckWindup,
            MagpieState::Peck,
            MagpieState::Stuck,
            MagpieState::Takeoff,
            MagpieState::StrafeWindup,
            MagpieState::StrafePass,
            MagpieState::SnatchDrop,
            MagpieState::Land,
            MagpieState::GustWindup,
            MagpieState::Gust,
            MagpieState::Death,
        ];
        for state in states {
            for prog in [0.15f32, 0.55, 0.9] {
                let mut view = MagpieView::new(
                    Vec2::new(46.0, 58.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(104.0, 66.0),
                );
                view.state = state;
                view.progress = prog;
                view.anim = 0.7 + prog * 3.0;
                view.hunting = false;
                let pose = Pose::from_view(&view, Vec2::ZERO, (206, 150, 70));
                let circles = body_hit_circles(&view);
                let reach = circles.iter().map(|(c, r)| c.dist(pose.body) - r).fold(
                    f32::MAX,
                    f32::min,
                );
                assert!(
                    reach <= 0.0,
                    "{state:?} at {prog}: the body is drawn {reach:.1}px outside every hit circle"
                );
                // The lift is small enough to keep the whole sprite on deck.
                assert!(
                    pose.ground.y - pose.body.y <= HIT_LIFT_MAX + 0.01,
                    "{state:?} at {prog}: drawn {}px above the deck",
                    pose.ground.y - pose.body.y
                );
            }
        }
    }

    /// Development aid: `RATRUN_MAGPIE_SHEET=1 cargo test magpie_sheet`
    /// writes a contact sheet of every state of the fight to
    /// `/tmp/ratrun_magpie_sheet.ppm` (one cell per state, three progresses
    /// each). Inert unless the variable is set, so it never slows the suite.
    #[test]
    fn magpie_sheet() {
        if std::env::var("RATRUN_MAGPIE_SHEET").is_err() {
            return;
        }
        const CELL_W: usize = 150;
        const CELL_H: usize = 110;
        let states = [
            MagpieState::Perch,
            MagpieState::Ruffle,
            MagpieState::PeckWindup,
            MagpieState::Peck,
            MagpieState::Stuck,
            MagpieState::Takeoff,
            MagpieState::StrafeWindup,
            MagpieState::StrafePass,
            MagpieState::SnatchDrop,
            MagpieState::Land,
            MagpieState::GustWindup,
            MagpieState::Gust,
            MagpieState::Death,
        ];
        let cols = 3usize;
        let mut sheet = PixelBuffer::new(CELL_W * cols, CELL_H * states.len());
        for (i, state) in states.iter().enumerate() {
            for (j, prog) in [0.15f32, 0.55, 0.9].iter().enumerate() {
                let mut cell = PixelBuffer::new(CELL_W, CELL_H);
                // A plank floor so the ground telegraphs read.
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        let plank = (y / 9) % 2 == 0;
                        let grain = hash2((x / 3) as i32, (y / 9) as i32, 9);
                        let base = if plank { (74, 56, 38) } else { (66, 50, 34) };
                        let c = mix(base, (96, 74, 50), grain * 0.35);
                        cell.plot(x as i32, y as i32, c);
                    }
                }
                let mut view = MagpieView::new(
                    Vec2::new(46.0, 58.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(104.0, 66.0),
                );
                view.state = *state;
                view.progress = *prog;
                view.anim = 0.7 + prog * 3.0;
                view.hp_frac = match state {
                    MagpieState::Gust => 0.25,
                    MagpieState::Ruffle => 0.6,
                    _ => 1.0,
                };
                view.lane_start = Vec2::new(24.0, 66.0);
                view.target = Vec2::new(124.0, 66.0);
                view.hunting = false;
                draw_magpie_ground(&mut cell, &view, Vec2::ZERO, (206, 150, 70));
                draw_magpie(&mut cell, &view, Vec2::ZERO, (206, 150, 70));
                draw_magpie_crate(&mut cell, &view, Vec2::ZERO);
                // Blit into the sheet.
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        let c = cell.pixel_at(x, y);
                        sheet.plot(
                            (j * CELL_W + x) as i32,
                            (i * CELL_H + y) as i32,
                            c,
                        );
                    }
                }
            }
        }
        write_ppm("/tmp/ratrun_magpie_sheet.ppm", &sheet);
    }

    /// Development aid: `RATRUN_MAGPIE_WORLD=1 cargo test magpie_world`
    /// renders the real flatbed through `World::draw` with the bird in four
    /// poses, so the integration (deck, lighting, camera, telegraphs) can be
    /// looked at and not just the sprite. Inert unless the variable is set.
    #[test]
    fn magpie_world() {
        if std::env::var("RATRUN_MAGPIE_WORLD").is_err() {
            return;
        }
        use crate::game::ecs::components::SpawnId;
        use crate::game::level::{Theme, TileRect};
        use crate::game::world::World;

        const VIEW: (usize, usize) = (160, 168);
        let mut sheet = PixelBuffer::new(VIEW.0 * 2, VIEW.1 * 2);
        let poses = [
            (MagpieState::PeckWindup, 0.85f32),
            (MagpieState::StrafePass, 0.45),
            (MagpieState::SnatchDrop, 0.55),
            (MagpieState::Gust, 0.20),
        ];
        for (i, (state, prog)) in poses.iter().enumerate() {
            let mut w = World::new(Theme::Trailer, 11);
            w.level_index = crate::game::campaign::LEVEL_COUNT;
            w.lighting = crate::render::lighting::LightingMode::Full;
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .expect("the flatbed's cab hold");
            let centre = arena.center_world();
            w.player.pos = centre - Vec2::new(30.0, 0.0);
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut pb);
            let cam = w.cam;
            let pos = centre + Vec2::new(20.0, -6.0);
            let mut boss = crate::game::ecs::systems::boss::Boss::new(
                crate::game::ecs::systems::boss::BossKind::Magpie,
                pos,
                1600.0,
                TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows()),
                centre,
                false,
            );
            boss.facing = Vec2::new(1.0, 0.0);
            boss.target = pos + Vec2::new(70.0, 6.0);
            boss.pounce_origin = pos - Vec2::new(60.0, 0.0);
            boss.set_state(
                crate::game::ecs::systems::boss::BossState::Magpie(*state),
                1.4,
            );
            boss.elapsed = prog * 1.4;
            boss.hp = if *state == MagpieState::Gust {
                1600.0 * 0.22
            } else {
                1600.0
            };
            w.ecs.spawn((boss, SpawnId(9001)));
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut pb);
            let _ = cam;
            for y in 0..VIEW.1 {
                for x in 0..VIEW.0 {
                    let c = pb.pixel_at(x, y);
                    sheet.plot(
                        ((i % 2) * VIEW.0 + x) as i32,
                        ((i / 2) * VIEW.1 + y) as i32,
                        c,
                    );
                }
            }
        }
        write_ppm("/tmp/ratrun_magpie_world.ppm", &sheet);
    }

    fn write_ppm(path: &str, pb: &PixelBuffer) {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for c in pb.pixels() {
            out.push(c.0);
            out.push(c.1);
            out.push(c.2);
        }
        std::fs::write(path, out).expect("write the ppm");
    }

    #[test]
    fn phases_are_the_art_thresholds() {
        // The thresholds nest: phase 3 opens below phase 2, and the critical
        // shimmer only inside phase 3. (Checked at compile time.)
        const _: () = assert!(PHASE3_HP_FRAC < PHASE2_HP_FRAC);
        const _: () = assert!(CRITICAL_HP_FRAC < PHASE3_HP_FRAC);
    }

    #[test]
    fn hit_circles_cover_the_body_and_the_buried_head() {
        let mut view = MagpieView::new(Vec2::new(100.0, 100.0), Vec2::new(1.0, 0.0), Vec2::ZERO);
        let circles = body_hit_circles(&view);
        // The body circle sits on the bird; the head is 14 px ahead of it.
        assert!((circles[1].0 - view.pos).len() < 1.0);
        assert!((circles[2].0 - (view.pos + Vec2::new(HEAD_OFFSET, 0.0))).len() < 0.5);
        // Buried: the head is parked at the strike point, in the open.
        view.state = MagpieState::Stuck;
        view.target = Vec2::new(100.0, 140.0);
        let circles = body_hit_circles(&view);
        assert_eq!(circles[2].0, Vec2::new(100.0, 140.0));
        assert!((head_center(&view) - view.target).len() < 0.01);
        assert!((beak_tip(&view) - view.target).len() < 0.01);
    }

    #[test]
    fn the_gust_pulses_on_and_off() {
        // Three pulses of `GUST_PULSE_ACTIVE`, with calm gaps between.
        let n = 400;
        let mut on = 0;
        let mut was = false;
        let mut edges = 0;
        for i in 0..=n {
            let p = i as f32 / n as f32;
            let now = gust_pulse_active(MagpieState::Gust, p);
            if now && !was {
                edges += 1;
            }
            if now {
                on += 1;
            }
            was = now;
        }
        assert_eq!(edges, GUST_PULSES as i32, "one pulse per wing-beat");
        let frac = on as f32 / (n + 1) as f32;
        // Duty over the whole beat: three active windows and the calm tail
        // after the last of them.
        let expect = GUST_PULSE_ACTIVE * GUST_PULSES as f32 / GUST_DURATION;
        assert!(
            (frac - expect).abs() < 0.03,
            "duty {frac} should be about {expect}"
        );
        // The average push is well under the rat's 58 px/s walk.
        assert!(GUST_PUSH * expect < 58.0 * 0.8);
        // Other states never push.
        assert!(!gust_pulse_active(MagpieState::GustWindup, 0.5));
        assert!(!gust_pulse_active(MagpieState::Perch, 0.5));
    }

    #[test]
    fn the_gust_field_pushes_along_the_deck() {
        let bird = Vec2::new(400.0, 400.0);
        let shape = gust_shape(bird);
        let centre = shape.center();
        // A rat downwind of the bird is pushed tailward; the direction error
        // across the whole hold stays small.
        for dy in [-200.0f32, -100.0, 0.0, 100.0, 200.0] {
            let at = bird + Vec2::new(-60.0, dy);
            assert!(shape.contains(at, 3.0), "the field must cover {at:?}");
            let push = (at - centre).normalized();
            assert!(push.x < -0.85, "push {push:?} is not down the deck");
        }
        // The fan is tight because the anchor is far upwind: the whole field
        // reads as "down the deck", and it covers everything within a reach of
        // the bird on both sides of it (the gust fills the hold, it does not
        // radiate from the bird).
        assert!(shape.contains(bird + Vec2::new(-GUST_REACH, 0.0), 3.0));
        assert!(shape.contains(bird + Vec2::new(0.0, 200.0), 3.0));
        for dy in [-200.0f32, -80.0, 0.0, 80.0, 200.0] {
            let at = bird + Vec2::new(-GUST_REACH * 0.5, dy);
            let push = (at - centre).normalized();
            let fan = (push.dot(GUST_AXIS) / GUST_AXIS.len()).clamp(-1.0, 1.0);
            let deg = fan.acos().to_degrees();
            assert!(deg < 16.0, "the push fans {deg:.1}° off the deck");
        }
    }

    #[test]
    fn body_contact_is_off_in_the_recovery_states() {
        for s in [
            MagpieState::Stuck,
            MagpieState::Peck,
            MagpieState::Ruffle,
            MagpieState::Gust,
            MagpieState::Death,
        ] {
            assert!(!has_body_contact(s), "{s:?} must not bump the rat");
        }
        for s in [MagpieState::Perch, MagpieState::PeckWindup] {
            assert!(has_body_contact(s), "{s:?} must bump the rat");
        }
        assert_eq!(incoming_damage_mult(MagpieState::Stuck), STUCK_DAMAGE_MULT);
        assert_eq!(incoming_damage_mult(MagpieState::Gust), GUST_DAMAGE_MULT);
        assert_eq!(incoming_damage_mult(MagpieState::Death), 0.0);
    }

    #[test]
    fn the_bird_draws_without_panicking_in_every_state() {
        let mut pb = PixelBuffer::new(120, 100);
        pb.clear((30, 32, 40));
        let states = [
            MagpieState::Perch,
            MagpieState::Ruffle,
            MagpieState::PeckWindup,
            MagpieState::Peck,
            MagpieState::Stuck,
            MagpieState::Takeoff,
            MagpieState::StrafeWindup,
            MagpieState::StrafePass,
            MagpieState::SnatchDrop,
            MagpieState::Land,
            MagpieState::GustWindup,
            MagpieState::Gust,
            MagpieState::Death,
        ];
        for state in states {
            for step in 0..7 {
                let mut view =
                    MagpieView::new(Vec2::new(60.0, 50.0), Vec2::new(1.0, 0.0), Vec2::new(80.0, 60.0));
                view.state = state;
                view.progress = step as f32 / 6.0;
                view.anim = step as f32 * 0.31;
                view.lane_start = Vec2::new(30.0, 50.0);
                view.target = Vec2::new(90.0, 50.0);
                draw_magpie_ground(&mut pb, &view, Vec2::ZERO, (200, 120, 60));
                draw_magpie(&mut pb, &view, Vec2::ZERO, (200, 120, 60));
                draw_magpie_crate(&mut pb, &view, Vec2::ZERO);
            }
        }
        // Something was actually drawn for a perched bird.
        let mut fresh = PixelBuffer::new(120, 100);
        fresh.clear((0, 0, 0));
        let mut view =
            MagpieView::new(Vec2::new(60.0, 50.0), Vec2::new(1.0, 0.0), Vec2::new(80.0, 50.0));
        view.anim = 0.4;
        draw_magpie(&mut fresh, &view, Vec2::ZERO, (200, 120, 60));
        assert!(
            fresh.pixels().iter().any(|c| *c != (0, 0, 0)),
            "the bird must draw pixels"
        );
    }
}
