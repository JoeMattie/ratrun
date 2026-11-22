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

/// Longest the neck is drawn from the body centre to a buried head.
const BURIED_NECK_MAX: f32 = HEAD_OFFSET + 3.2;

/// The body's world centre for this view: `view.pos`, except while the bill is
/// down (`Peck`/`Stuck`), when the torso **leans after the head** so the neck
/// never stretches past [`BURIED_NECK_MAX`]. The sprite and the hit circles
/// both read this, so the bird is always drawn over the shapes the rat hits.
#[inline]
pub fn body_center(view: &MagpieView) -> Vec2 {
    if !matches!(view.state, MagpieState::Peck | MagpieState::Stuck) {
        return view.pos;
    }
    let to_head = head_center(view) - view.pos;
    let d = to_head.len();
    if d > BURIED_NECK_MAX {
        view.pos + to_head * ((d - BURIED_NECK_MAX) / d)
    } else {
        view.pos
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
    // The leaned centre (`body_center`): the tail root and the body follow the
    // torso the sprite draws, the head stays parked on the strike point.
    let body = body_center(view);
    [
        (body - f * (TAIL_LENGTH * 0.45), 5.5),
        (body, BODY_RADIUS * 0.82),
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
    /// Screen-space visual body centre (already lifted by flight altitude & lunge lean).
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
    /// Wing beat clock (`-1..1`).
    flap: f32,
    /// Head centre in screen space.
    head: Vec2,
    /// True when the bill is driven into the oak plank (`Peck` / `Stuck`).
    buried: bool,
    /// Head-up/head-down lean (`>0` forward lunge, `<0` coiled back).
    head_lunge: f32,
    /// Body stretch along `f`.
    stretch: f32,
    /// Screen `-y` lift of the whole bird.
    altitude: f32,
    /// Tail fan/lift (`-1..1`).
    tail_lift: f32,
    /// Tail fan spread (`0..1`, wide on Gust / Takeoff / Ruffle).
    tail_fan: f32,
    /// Feather ruffle `0..1` (phase-change pose).
    ruffle: f32,
    /// Shake jitter in px.
    jitter: Vec2,
    /// Fade out (death).
    alpha: f32,
    /// Death progress `0..1` (limp, rolling bird).
    death: f32,
    hit: f32,
    hp: f32,
    hunting: bool,
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
        let prog = if view.progress.is_finite() {
            view.progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hit = if view.hit_flash.is_finite() {
            view.hit_flash.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hp = if view.hp_frac.is_finite() {
            view.hp_frac.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let anim = if view.anim.is_finite() { view.anim } else { 0.0 };
        let enraged = if hp <= PHASE2_HP_FRAC {
            let t = ((PHASE2_HP_FRAC - hp) / (PHASE2_HP_FRAC - PHASE3_HP_FRAC)).clamp(0.0, 1.0);
            0.45 + 0.55 * t
        } else {
            0.0
        };
        let pulse = 0.5 + 0.5 * (anim * 9.0).sin();
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
        let tail_fan: f32;
        let mut ruffle = 0.0f32;
        let mut jitter = Vec2::ZERO;
        let mut alpha = 1.0f32;
        let mut death = 0.0f32;
        let mut buried = false;

        match view.state {
            MagpieState::Perch => {
                // Cocky corvid strut: quick head-bob and tail flick!
                let bob = (anim * 4.2).sin();
                head_lunge = 0.25 + 0.25 * bob;
                tail_lift = 0.16 * (anim * 2.1).sin();
                tail_fan = if view.hunting { 0.35 } else { 0.15 };
                spread = if view.hunting { 0.36 } else { 0.0 };
                flap = if view.hunting { (anim * 13.0).sin() } else { 0.0 };
            }
            MagpieState::Ruffle => {
                // Dramatic phase-change feather puff, wide wing flare & hard shake!
                let t = (prog / 0.32).min(1.0);
                let settle = ((prog - 0.72) / 0.28).clamp(0.0, 1.0);
                ruffle = t * (1.0 - settle);
                spread = 0.68 * (1.0 - settle) + 0.10;
                tail_fan = 0.85 * ruffle + 0.15;
                let sh = (prog * 52.0).sin() * 2.4 * ruffle;
                jitter = p * sh;
                tail_lift = 0.45 * ruffle;
                head_lunge = -0.35 * ruffle;
            }
            MagpieState::PeckWindup => {
                // Coiled spring anticipation: head pulls back, pied wings & diamond
                // tail flare immediately so both combat and library card show them!
                let coil = (prog / PECK_LOCK_PROGRESS).min(1.0);
                let ease = smoothstep(0.0, 1.0, coil);
                head_lunge = -1.15 * ease;
                stretch = -0.45 * ease;
                tail_lift = -0.35 * ease;
                tail_fan = 0.42 + 0.30 * ease;
                let tremble = if prog >= PECK_LOCK_PROGRESS {
                    (anim * 48.0).sin() * 0.95
                } else {
                    0.0
                };
                jitter = p * tremble;
                spread = 0.36 + 0.24 * ease;
            }
            MagpieState::Peck => {
                // Violent forward lunge down the beak line!
                let t = (prog / 0.45).min(1.0);
                head_lunge = 1.1;
                stretch = 0.9 * t;
                tail_lift = 0.55 * t;
                tail_fan = 0.55;
                spread = 0.46;
                buried = true;
            }
            MagpieState::Stuck => {
                // Bill jammed deep in the oak planks: frantic tugging & beating wings!
                let tug = (anim * 10.5).sin();
                head_lunge = 0.9;
                stretch = 0.45;
                tail_lift = 0.72 + 0.22 * tug;
                tail_fan = 0.68;
                spread = 0.70 + 0.18 * tug;
                flap = (anim * 19.0).sin();
                jitter = p * (tug * 1.4) - f * (tug.abs() * 1.2);
                buried = true;
            }
            MagpieState::Takeoff => {
                let t = smoothstep(0.0, 0.75, prog);
                altitude = 13.0 * t;
                spread = (prog / 0.55).min(1.0);
                flap = (anim * 22.0).sin();
                tail_lift = 0.4 * t;
                tail_fan = 0.75 * t;
                head_lunge = 0.20;
            }
            MagpieState::StrafeWindup => {
                altitude = 13.0;
                spread = 1.0;
                flap = (anim * 18.0).sin();
                tail_lift = 0.15;
                tail_fan = 0.50;
                sweep = -0.22;
                head_lunge = 0.48;
            }
            MagpieState::StrafePass => {
                altitude = 13.0;
                spread = 1.0;
                // Sleek swept-back stooping dive wings!
                sweep = -0.82;
                flap = (anim * 10.0).sin() * 0.28;
                tail_lift = 0.0;
                tail_fan = 0.25;
                head_lunge = 0.65;
            }
            MagpieState::SnatchDrop => {
                // Hard aerial brake with wings thrown forward to wheel and release the crate!
                let brake = smoothstep(0.0, 0.45, prog);
                altitude = 13.0 + 4.0 * (1.0 - brake);
                spread = 1.0;
                sweep = 0.58 * brake;
                flap = (anim * 22.0).sin() * brake;
                tail_lift = -0.35 * brake;
                tail_fan = 0.85 * brake;
                head_lunge = 0.35;
            }
            MagpieState::Land => {
                let t = smoothstep(0.0, 0.65, prog);
                altitude = 13.0 * (1.0 - t);
                spread = (1.0 - t) * 0.92;
                flap = (anim * 15.0).sin() * (1.0 - t);
                let squash = (1.0 - t).powi(2);
                jitter = Vec2::new(0.0, squash * 1.6);
                tail_lift = 0.28;
                tail_fan = 0.45 * (1.0 - t);
            }
            MagpieState::GustWindup => {
                // Claws dig into oak planks, chest puffs, huge wings haul far back!
                let c = smoothstep(0.0, 1.0, prog);
                spread = 0.85 * c;
                sweep = -0.65 * c;
                tail_lift = -0.35 * c;
                tail_fan = 0.60 * c;
                head_lunge = -0.45 * c;
                jitter = p * ((anim * 34.0).sin() * 0.65 * c);
            }
            MagpieState::Gust => {
                // Heavy rhythmic downstrokes on active pulses, braced glide in calm gaps!
                let pushing = gust_pulse_active(view.state, prog);
                let beat = if pushing {
                    (anim * 34.0).sin()
                } else {
                    0.12 * (anim * 4.5).sin()
                };
                spread = if pushing { 1.0 } else { 0.56 };
                flap = beat;
                sweep = if pushing { 0.52 * beat } else { -0.15 };
                tail_lift = if pushing { 0.45 + 0.22 * beat } else { -0.12 };
                tail_fan = if pushing { 0.95 } else { 0.35 };
                head_lunge = if pushing { 0.65 } else { 0.18 };
                jitter = Vec2::new(0.0, if pushing { beat * 1.3 } else { 0.0 });
            }
            MagpieState::Death => {
                death = prog;
                buried = false;
                if prog < 0.38 {
                    let t = prog / 0.38;
                    let spasm = (t * 34.0).sin() * 2.0 * (1.0 - t * 0.25);
                    spread = 0.45 + 0.55 * t;
                    flap = spasm;
                    tail_lift = 0.55 * t;
                    tail_fan = 0.80 * t;
                    head_lunge = 0.35 * t;
                    jitter = p * spasm;
                } else {
                    let t = ((prog - 0.38) / 0.44).min(1.0);
                    let ease = 1.0 - (1.0 - t) * (1.0 - t);
                    spread = 1.0 - 0.32 * ease;
                    flap = (1.0 - t) * 0.35;
                    tail_lift = 0.55 - 0.95 * ease;
                    tail_fan = 0.80 - 0.35 * ease;
                    head_lunge = 0.35 + 0.55 * ease;
                    jitter = p * (1.0 - t) * 0.8;
                    if prog > 0.86 {
                        alpha = 1.0 - ((prog - 0.86) / 0.14) * 0.45;
                    }
                }
            }
        }

        // Death rolls the entire bird over onto its flank in a dramatic collapse!
        let (f, p) = if view.state == MagpieState::Death && death > 0.0 {
            let roll = smoothstep(0.15, 0.82, death) * 0.92;
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

        let target_sc = view.target - cam;
        let head_world = if matches!(view.state, MagpieState::Peck | MagpieState::Stuck) {
            view.target
        } else {
            view.pos + f * (HEAD_OFFSET + head_lunge * 2.4 + stretch * 2.5)
        };
        let slump = if view.state == MagpieState::Death {
            p * (smoothstep(0.25, 0.85, death) * 6.5) - f * (death * 2.8)
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

        // Visual body centre: during Peck/Stuck, lean the torso forward so the neck stays
        // anatomically fierce (~13-17 px from chest to buried head) instead of stretching across the deck!
        // The lean is `body_center`, the same world point the hit circles are
        // built on, so the leaned torso stays over what the rat can hit.
        let body = ground + (body_center(view) - view.pos) + jitter + Vec2::new(0.0, -lift);

        Self {
            body,
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
            tail_fan,
            ruffle,
            jitter,
            alpha,
            death,
            hit,
            hp,
            hunting: view.hunting,
            enraged,
            critical,
            anim,
            state: view.state,
            progress: prog,
            accent,
            target: target_sc,
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
    if pb.w == 0 || pb.h == 0 || !center.x.is_finite() || !center.y.is_finite() {
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
    if a <= 0.004 || x < 0 || y < 0 || x >= pb.w as i32 || y >= pb.h as i32 {
        return;
    }
    let ia = a.clamp(0.0, 1.0);
    let i = y as usize * pb.w + x as usize;
    let d = pb.pixels()[i];
    pb.pixels_mut()[i] = mix(d, c, ia);
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.004 || x < 0 || y < 0 || x >= pb.w as i32 || y >= pb.h as i32 {
        return;
    }
    let i = y as usize * pb.w + x as usize;
    let d = pb.pixels()[i];
    let k = (1.0 - amount).clamp(0.0, 1.0);
    pb.pixels_mut()[i] = (
        (d.0 as f32 * k) as u8,
        (d.1 as f32 * k) as u8,
        (d.2 as f32 * k) as u8,
    );
}

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
fn ellipse_sdf(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let du = (u - cu) / ru;
    let dv = (v - cv) / rv;
    ((du * du + dv * dv).sqrt() - 1.0) * ru.min(rv)
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
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.01 {
        c
    } else {
        mix(c, (255, 244, 224), (hit * 0.62).min(0.72))
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
    /// Crisp ink exterior outline.
    outline: Rgb,
    /// Deep jet blue-black corvid shadow.
    shadow: Rgb,
    /// Glossy blue-black mantle & head.
    back: Rgb,
    /// Sleek blue-slate specular highlight on black feathers.
    gloss: Rgb,
    /// Blazing snow-white magpie scapulars & primary flash.
    white: Rgb,
    /// Cool silver-grey shadow on white feathers.
    white_sh: Rgb,
    /// Peacock iridescent teal-emerald (shifting royal indigo-violet in P2/P3!).
    sheen: Rgb,
    /// Bright turquoise/violet iridescence highlight.
    sheen_hi: Rgb,
    /// Charcoal-horn corvid chisel bill.
    bill: Rgb,
    /// Pale yellow-ivory bill cere/hilt!
    hilt: Rgb,
}

fn plumage(pose: &Pose) -> Plumage {
    let dead = ((pose.death - 0.30) / 0.70).clamp(0.0, 1.0);
    let e = pose.enraged;
    let mut back = mix((18, 22, 34), (14, 16, 28), e);
    let mut gloss = mix((54, 70, 102), (76, 56, 112), e * 0.7);
    let mut white = mix((244, 246, 252), (252, 246, 234), e * 0.5);
    let mut white_sh = mix((158, 168, 184), (168, 152, 150), e * 0.5);
    let mut sheen = mix((24, 134, 136), (92, 46, 158), e * 0.72);
    let mut sheen_hi = mix((78, 214, 198), (162, 108, 242), e * 0.72);
    let bill = (28, 28, 34);
    let hilt = mix((236, 204, 112), (255, 192, 82), e);
    let mut shadow = (8, 10, 16);
    let outline = if pose.hit > 0.55 {
        (92, 64, 42)
    } else {
        (4, 5, 8)
    };
    if dead > 0.0 {
        back = mix(back, (58, 56, 62), dead * 0.58);
        gloss = mix(gloss, (92, 90, 94), dead * 0.55);
        white = mix(white, (166, 164, 156), dead * 0.52);
        white_sh = mix(white_sh, (108, 106, 102), dead * 0.52);
        sheen = mix(sheen, (62, 68, 74), dead * 0.75);
        sheen_hi = mix(sheen_hi, (96, 102, 108), dead * 0.75);
        shadow = (22, 22, 26);
    }
    Plumage {
        outline,
        shadow,
        back,
        gloss,
        white,
        white_sh,
        sheen,
        sheen_hi,
        bill,
        hilt,
    }
}

/// Sculpted avian wing SDF: broad muscular inner wing-arm sweeping smoothly out to
/// a tapered wrist & fanned primaries — folded cleanly along the flanks when perched!
#[inline]
fn wing_sdf(pose: &Pose, side: f32, u: f32, v: f32) -> (f32, f32, f32) {
    let spread = pose.spread.clamp(0.0, 1.0);
    let flap_pull = pose.flap * 2.2 * spread;
    // Shoulder root & carpal wrist & wingtip coordinates in local `(u, v)`.
    let root = (4.8, side * 4.4);
    let wrist = (
        6.2 - 8.5 * (1.0 - spread) + 3.2 * pose.sweep,
        side * (5.6 + 8.0 * spread),
    );
    let tip = (
        1.5 - 17.5 * (1.0 - spread) + 8.5 * pose.sweep + flap_pull,
        side * (4.6 + 17.8 * spread),
    );
    // Broad inner secondary blade + sleek outer primary blade blended smoothly!
    let d_inner = sd_taper(u, v, root, wrist, 5.2, 4.6 - 0.8 * spread);
    let d_outer = sd_taper(u, v, wrist, tip, 4.5 - 0.8 * spread, 1.35);
    let d_trail = sd_taper(
        u,
        v,
        (root.0 - 4.2, side * 4.2),
        (wrist.0 - 4.0 * spread, side * (5.2 + 7.2 * spread)),
        3.8,
        2.6,
    );
    let mut d = smin(d_inner, d_outer, 2.2).min(d_trail);

    // Phase 2/3 ragged feather battle-notches along the trailing edge!
    if pose.enraged > 0.3 && u < wrist.0 - 1.2 {
        let notch = ((v * 1.45).sin()).max(0.0) * (0.85 * pose.enraged + 0.55 * pose.critical);
        d += notch;
    }

    let span = ((v * side - 3.5) / (3.0 + 19.0 * spread)).clamp(0.0, 1.2);
    let chord = ((u - (tip.0 - 5.0)) / 14.0).clamp(0.0, 1.0);
    (d, span, chord)
}

/// Graduated diamond-shaped corvid tail SDF!
#[inline]
fn tail_sdf(pose: &Pose, u: f32, v: f32) -> (f32, f32) {
    let lift_v = pose.tail_lift * 2.6;
    let fan = 2.6 + 3.4 * pose.tail_fan;
    // Central long tail rectrices + slightly shorter flanking fan feathers!
    let d_center = sd_taper(u, v, (-4.5, 0.0), (-(5.5 + TAIL_LENGTH), lift_v), 3.4, 1.7);
    let d_left = sd_taper(
        u,
        v,
        (-4.5, -1.2),
        (-(3.5 + TAIL_LENGTH * 0.84), lift_v - fan),
        2.6,
        1.45,
    );
    let d_right = sd_taper(
        u,
        v,
        (-4.5, 1.2),
        (-(3.5 + TAIL_LENGTH * 0.84), lift_v + fan),
        2.6,
        1.45,
    );
    let d = smin(d_center, d_left.min(d_right), 1.4);
    let along = ((-u - 4.5) / TAIL_LENGTH).clamp(0.0, 1.0);
    (d, along)
}

/// Muscular corvid torso SDF (broad chest, sleek tapered rump).
#[inline]
fn torso_sdf(pose: &Pose, u: f32, v: f32) -> f32 {
    let chest = ellipse_sdf(u, v, 1.2, 0.0, 8.8, 6.6);
    let rump = ellipse_sdf(u, v, -5.2, 0.0, 6.2, 4.4);
    let mut d = smin(chest, rump, 2.6);
    if pose.ruffle > 0.02 {
        let ruff = ellipse_sdf(
            u,
            v,
            3.5,
            0.0,
            6.5 + 2.8 * pose.ruffle,
            6.8 + 2.2 * pose.ruffle,
        );
        d = smin(d, ruff, 1.6);
    }
    d
}

// ============================================================================
// Ground layer
// ============================================================================

/// The boss's ground layer: warnings, sweeping flight shadow, plank splinters & HP arc.
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
        MagpieState::Peck => {
            draw_beak_line(pb, &pose, true);
            draw_plank_splinters(pb, pose.target, pose.f, 0.75 + 0.25 * pose.progress);
        }
        MagpieState::Stuck => {
            draw_beak_line(pb, &pose, true);
            draw_plank_splinters(pb, pose.target, pose.f, 1.0);
        }
        MagpieState::StrafeWindup => draw_lane(pb, &pose, false),
        MagpieState::StrafePass => draw_lane(pb, &pose, true),
        MagpieState::SnatchDrop => {
            draw_lane(pb, &pose, true);
            draw_crate_square(pb, &pose);
        }
        MagpieState::GustWindup => draw_gust(pb, &pose, 0.36),
        MagpieState::Gust => {
            let push = gust_pulse_active(view.state, view.progress);
            draw_gust(pb, &pose, if push { 1.0 } else { 0.18 });
        }
        MagpieState::Death if pose.death > 0.28 => {
            let t = ((pose.death - 0.28) / 0.44).clamp(0.0, 1.0);
            let fade = (1.0 - ((pose.death - 0.50) / 0.50).clamp(0.0, 1.0)) * 0.65;
            draw_ring(
                pb,
                pose.ground,
                DEATH_RING_RADIUS * (0.42 + 0.58 * t),
                (214, 198, 168),
                fade,
            );
            soft_dot(
                pb,
                pose.ground,
                DEATH_RING_RADIUS * (0.35 + 0.40 * t),
                (156, 140, 118),
                fade * 0.32,
            );
        }
        _ => {}
    }

    if body_vis {
        draw_shadow(pb, &pose);
        if pose.state != MagpieState::Death && pose.hp > 0.0 {
            draw_hp_arc(pb, &pose);
        }
    }
}

/// Crisp sweeping avian shadow on the oak planks (tight when perched, wide sweeping wings in flight!).
fn draw_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    let fly = (pose.altitude / 13.0).clamp(0.0, 1.4);
    let alpha = (0.50 - 0.14 * fly) * pose.alpha;
    let offset = Vec2::new(2.0 + 3.0 * fly, 3.0 + 5.0 * fly);
    let centre = pose.ground + offset;
    let reach = 29.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, centre, reach) else {
        return;
    };
    let ink: Rgb = (4, 5, 8);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - centre.x;
            let dy = y as f32 - centre.y;
            let u = dx * pose.f.x + dy * pose.f.y;
            let v = dx * pose.p.x + dy * pose.p.y;
            let d_t = torso_sdf(pose, u, v);
            let (d_tail, _) = tail_sdf(pose, u, v);
            let (d_wl, _, _) = wing_sdf(pose, -1.0, u, v);
            let (d_wr, _, _) = wing_sdf(pose, 1.0, u, v);
            let d = d_t.min(d_tail).min(d_wl).min(d_wr);
            let soft_edge = 1.6 + 1.4 * fly;
            if d < soft_edge {
                let soft = ((soft_edge - d) / (soft_edge + 1.2)).clamp(0.0, 1.0);
                blend_px(pb, x, y, ink, alpha * soft);
            }
        }
    }
}

/// Radial Boss HP arc beneath the Magpie (phase notches at 30% and 62%).
fn draw_hp_arc(pb: &mut PixelBuffer, pose: &Pose) {
    let r = 17.5;
    let steps = 48;
    let lit = (pose.hp * steps as f32).round() as i32;
    let col = if pose.hp > PHASE2_HP_FRAC {
        (98, 224, 212)
    } else if pose.hp > PHASE3_HP_FRAC {
        (255, 192, 68)
    } else {
        (255, 76, 56)
    };
    for i in 0..steps {
        let frac = i as f32 / steps as f32;
        let a = -std::f32::consts::FRAC_PI_2 + frac * std::f32::consts::TAU;
        let x = (pose.ground.x + a.cos() * r).round() as i32;
        let y = (pose.ground.y + a.sin() * r).round() as i32;
        if (frac - PHASE3_HP_FRAC).abs() < 0.015 || (frac - PHASE2_HP_FRAC).abs() < 0.015 {
            blend_px(pb, x, y, (8, 8, 12), 0.90);
            continue;
        }
        darken_px(pb, x, y + 1, 0.35);
        if i < lit {
            let lead = i + 1 == lit;
            blend_px(pb, x, y, if lead { (255, 252, 236) } else { col }, if lead { 0.95 } else { 0.78 });
        } else {
            blend_px(pb, x, y, (20, 22, 28), 0.42);
        }
    }
}

fn draw_ring(pb: &mut PixelBuffer, center: Vec2, radius: f32, c: Rgb, alpha: f32) {
    if radius < 1.0 || alpha <= 0.01 {
        return;
    }
    let steps = ((radius * 6.5).round() as i32).clamp(16, 220);
    for i in 0..steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let x = (center.x + a.cos() * radius).round() as i32;
        let y = (center.y + a.sin() * radius).round() as i32;
        blend_px(pb, x, y, c, alpha);
    }
}

/// Cracked oak plank splinters radiating around the buried beak strike point!
fn draw_plank_splinters(pb: &mut PixelBuffer, at: Vec2, dir: Vec2, strength: f32) {
    let dark_hole: Rgb = (14, 10, 8);
    let splinter_hi: Rgb = (218, 178, 114);
    let splinter_mid: Rgb = (148, 104, 58);
    soft_dot(pb, at, 3.6, dark_hole, 0.85 * strength);
    for k in 0..7 {
        let a = dir.angle() + (k as f32 - 3.0) * 0.55 + hash2(k, 2, 13) * 0.25;
        let v = Vec2::from_angle(a);
        let p0 = at + v * 2.4;
        let p1 = at + v * (6.2 + hash2(k, 5, 19) * 3.5);
        draw_segment(pb, p0 + Vec2::new(0.0, 1.0), p1 + Vec2::new(0.0, 1.0), 0.6, dark_hole, 0.65 * strength);
        draw_segment(pb, p0, p1, 0.6, if k % 2 == 0 { splinter_hi } else { splinter_mid }, 0.90 * strength);
    }
}

/// High-contrast marked strike square at the end of a beak line or lane!
fn strike_square(pb: &mut PixelBuffer, at: Vec2, half: f32, c: Rgb, alpha: f32, solid: bool) {
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    for dy in -(h + 1)..=(h + 1) {
        for dx in -(h + 1)..=(h + 1) {
            let ax = dx.abs();
            let ay = dy.abs();
            let outer = ax == h + 1 || ay == h + 1;
            let edge = ax == h || ay == h;
            let corner = ax >= h - 1 && ay >= h - 1 && ax <= h && ay <= h;
            if outer && (solid || corner) {
                darken_px(pb, cx + dx, cy + dy, 0.45 * alpha);
            } else if edge && (solid || corner || (ax + ay) % 2 == 0) {
                blend_px(pb, cx + dx, cy + dy, c, alpha);
            } else if ax < h && ay < h {
                blend_px(pb, cx + dx, cy + dy, c, alpha * if solid { 0.36 } else { 0.14 });
            }
        }
    }
}

/// High-contrast beak line on the oak planks: dark contrast backing + amber-to-crimson rails + marching chevrons!
fn draw_beak_line(pb: &mut PixelBuffer, pose: &Pose, live: bool) {
    let bird = pose.ground + pose.f * (HEAD_OFFSET + 2.0);
    let tip = pose.target;
    let locked = live || pose.progress >= PECK_LOCK_PROGRESS;
    let pulse = 0.5 + 0.5 * (pose.anim * 16.0).sin();
    let col = if live {
        mix((255, 250, 224), (255, 84, 52), 0.32)
    } else if locked {
        mix((255, 76, 52), (255, 232, 132), 0.35 + 0.45 * pulse)
    } else {
        mix((255, 168, 56), (255, 228, 128), 0.35 + 0.35 * pulse)
    };
    let fill_col = if locked { (214, 48, 36) } else { (210, 124, 34) };
    let alpha = if live { 0.96 } else if locked { 0.86 } else { 0.66 + 0.20 * pulse };

    let d = tip - bird;
    let len = d.len();
    if len > 1.0 {
        let dir = d * (1.0 / len);
        let perp = dir.perp();
        let half = PECK_ACTIVE_HALF_WIDTH;
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let p = bird + dir * (i as f32);
            // Translucent danger fill across the exact corridor width!
            for k in -(half as i32 - 1)..=(half as i32 - 1) {
                let q = p + perp * k as f32;
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, fill_col, if live { 0.26 } else { 0.15 });
            }
            // Crisp dark-backed warning rails on both edges!
            for s in [-1.0f32, 1.0] {
                let out_q = p + perp * ((half + 1.0) * s);
                darken_px(pb, out_q.x.round() as i32, out_q.y.round() as i32, 0.45 * alpha);
                let q = p + perp * (half * s);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
            }
        }
        // Marching strike chevrons racing down the beak line toward the strike square!
        let scroll = (pose.anim * 28.0) % 9.0;
        let mut s = scroll;
        while s < len - 4.0 {
            let q = bird + dir * s;
            let c_tip = q + dir * 2.5;
            draw_segment(pb, q - dir + perp * 2.6, c_tip, 0.6, col, alpha * 0.85);
            draw_segment(pb, q - dir - perp * 2.6, c_tip, 0.6, col, alpha * 0.85);
            s += 9.0;
        }
    }
    strike_square(pb, tip, 4.8, col, alpha, locked);
}

/// High-contrast strafing pass lane (`±11 px`) across the flatbed planks!
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
    let pulse = 0.5 + 0.5 * (pose.anim * 14.0).sin();
    let col = if live {
        mix((255, 248, 224), (255, 92, 52), 0.35)
    } else {
        mix((255, 168, 62), (255, 86, 54), 0.35 + 0.45 * pose.progress)
    };
    let fill_col = if live { (196, 46, 36) } else { (176, 88, 32) };
    let alpha = if live { 0.94 } else { 0.62 + 0.26 * pulse };
    let half = STRAFE_LANE_HALF_WIDTH;

    let steps = len.ceil() as i32;
    for i in 0..=steps {
        let p = a + dir * (i as f32);
        // Subtle darkened danger lane wash so the whole 22-px corridor reads clearly on warm oak!
        if i % 2 == 0 {
            for k in (-(half as i32 - 2)..=(half as i32 - 2)).step_by(2) {
                let q = p + perp * k as f32;
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, fill_col, if live { 0.24 } else { 0.14 });
            }
        }
        for s in [-1.0f32, 1.0] {
            let q_out = p + perp * ((half + 1.0) * s);
            darken_px(pb, q_out.x.round() as i32, q_out.y.round() as i32, 0.52 * alpha);
            let q = p + perp * (half * s);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
        }
    }
    // Bold sweeping chevrons pointing down the flight corridor!
    let scroll = (pose.anim * if live { 46.0 } else { 22.0 }) % 14.0;
    let mut s = scroll;
    while s < len - 5.0 {
        let q = a + dir * s;
        let tip = q + dir * 4.0;
        draw_segment(pb, q - dir + perp * 5.2, tip, 0.7, col, alpha * 0.78);
        draw_segment(pb, q - dir - perp * 5.2, tip, 0.7, col, alpha * 0.78);
        s += 14.0;
    }
    strike_square(pb, a, 3.5, col, alpha, live);
    strike_square(pb, b, 3.5, col, alpha, live);
}

/// The crate's marked landing square (`CRATE_HALF`), growing drop shadow & wood-splinter burst!
fn draw_crate_square(pb: &mut PixelBuffer, pose: &Pose) {
    let t = (pose.progress / CRATE_LAND_PROGRESS).clamp(0.0, 1.0);
    let at = pose.target;
    let half = CRATE_HALF;
    let pulse = 0.5 + 0.5 * (pose.anim * 18.0).sin();
    let col = mix((255, 206, 86), (255, 76, 52), t * 0.78 + 0.14 * pulse);
    let alpha = 0.58 + 0.40 * t;
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    for dy in -(h + 1)..=(h + 1) {
        for dx in -(h + 1)..=(h + 1) {
            let ax = dx.abs();
            let ay = dy.abs();
            let outer = ax == h + 1 || ay == h + 1;
            let rim = ax == h || ay == h;
            let corner = ax >= h - 2 && ay >= h - 2 && ax <= h && ay <= h;
            if outer {
                darken_px(pb, cx + dx, cy + dy, 0.42 * alpha);
            } else if rim && (corner || (ax + ay) % 3 != 0) {
                blend_px(pb, cx + dx, cy + dy, col, alpha);
            } else if ax < h && ay < h {
                // Growing dark impact shadow of the falling crate inside the target box!
                let sh_r = (half - 1.0) * (0.28 + 0.72 * t);
                if (dx.abs() as f32) <= sh_r && (dy.abs() as f32) <= sh_r {
                    darken_px(pb, cx + dx, cy + dy, 0.46 * t);
                }
                blend_px(pb, cx + dx, cy + dy, col, alpha * (0.14 + 0.16 * t));
            }
        }
    }
    // Impact splinter burst & shockwave ring when the crate shatters on the planks!
    if pose.progress >= CRATE_LAND_PROGRESS {
        let burst = ((pose.progress - CRATE_LAND_PROGRESS) / (1.0 - CRATE_LAND_PROGRESS))
            .clamp(0.0, 1.0);
        let fade = (1.0 - burst).clamp(0.0, 1.0);
        draw_ring(pb, at, half + 11.0 * burst, (255, 236, 176), fade * 0.90);
        soft_dot(pb, at, half * (1.0 - burst * 0.4), (255, 224, 152), fade * 0.48);
        // Eight tumbling wooden plank splinters flying radially outward!
        for k in 0..8 {
            let a = k as f32 / 8.0 * std::f32::consts::TAU + 0.3;
            let dir = Vec2::from_angle(a);
            let p0 = at + dir * (4.0 + 14.0 * burst);
            let p1 = p0 + dir * 4.5;
            draw_segment(pb, p0 + Vec2::new(0.0, 1.0), p1 + Vec2::new(0.0, 1.0), 0.7, (24, 16, 10), fade * 0.7);
            draw_segment(pb, p0, p1, 0.7, if k % 2 == 0 { (216, 166, 94) } else { (152, 106, 56) }, fade * 0.95);
        }
    }
}

/// Dynamic wing-beat wind gusts sweeping tailward (`-x`): tapered curved slipstream ribbons &
/// feathered wind eddies in active pulses, dropping cleanly to faint calm-air wisps in the gaps!
fn draw_gust(pb: &mut PixelBuffer, pose: &Pose, strength: f32) {
    let origin = pose.ground;
    let axis = GUST_AXIS;
    let perp = axis.perp();
    let reach = GUST_REACH.min(130.0);
    let active = strength > 0.5;
    let lanes = if active { 14 } else { 7 };

    for k in 0..lanes {
        // Pack inner 10 lanes cleanly inside the visible flatbed deck (`±48 px`),
        // with outer lanes extending wider for full-hold readability.
        let lane_norm = (k as f32 / (lanes - 1).max(1) as f32 - 0.5) * 2.0;
        let lane = lane_norm * lane_norm.abs().powf(0.25) * 68.0;
        let speed = if active { 2.8 } else { 1.0 };
        let phase = (pose.anim * speed + k as f32 * 0.29 + hash2(k, 1, 7) * 0.18).fract();
        let len = if active {
            22.0 + 22.0 * hash2(k, 3, 7)
        } else {
            10.0 + 12.0 * hash2(k, 3, 7)
        };
        let start = origin + perp * lane + axis * (6.0 + phase * (reach - len));
        let env = (phase * PI).sin();
        let alpha = strength * (0.45 + 0.52 * env);
        let col = if active {
            mix((168, 224, 255), (252, 254, 255), env * 0.78)
        } else {
            (142, 166, 192)
        };
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let wave = (t * PI * 2.0 + k as f32).sin() * if active { 1.6 } else { 0.5 };
            let taper = (t * PI).sin();
            let p = start + axis * (i as f32) + perp * wave;
            let px = p.x.round() as i32;
            let py = p.y.round() as i32;
            if active {
                // Subtle dark slipstream shadow under each ribbon so bright wind pops on oak planks!
                darken_px(pb, px, py + 1, 0.42 * alpha * taper);
            }
            blend_px(pb, px, py, col, alpha * (0.42 + 0.58 * taper));
        }
    }

    // Curved aerodynamic gust wave-front arcs pulsing outward from the wings (NO ugly vertical stick!).
    if active {
        for w in 0..2 {
            let wp = (pose.anim * 3.8 + w as f32 * 0.5).fract();
            let dist = 14.0 + wp * 68.0;
            let arc_half = 26.0 + wp * 34.0;
            let arc_alpha = (1.0 - wp) * 0.72 * strength;
            let segs = 28;
            for i in 0..=segs {
                let s = (i as f32 / segs as f32) * 2.0 - 1.0;
                // Bowed aerodynamic crescent curving tailward in the center!
                let bow = (1.0 - s * s) * (10.0 + wp * 6.0);
                let q = origin + axis * (dist + bow) + perp * (s * arc_half);
                let edge_fade = (1.0 - s * s).max(0.0).sqrt();
                blend_px(
                    pb,
                    q.x.round() as i32,
                    q.y.round() as i32,
                    (244, 250, 255),
                    arc_alpha * edge_fade,
                );
            }
        }
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// Draw the giant black-and-white Magpie boss!
pub fn draw_magpie(pb: &mut PixelBuffer, view: &MagpieView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let margin = CULL_MARGIN + 14.0;
    if pose.body.x < -margin
        || pose.body.x > pb.w as f32 + margin
        || pose.body.y < -margin
        || pose.body.y > pb.h as f32 + margin
    {
        return;
    }
    let pal = plumage(&pose);
    // 1) Scaly bird talons clutching the planks (or trailing under the belly in flight).
    draw_talons(pb, &pose, &pal);
    // 2) Long graduated iridescent peacock-sheen magpie tail!
    draw_tail(pb, &pose, &pal);
    // 3) Broad sculpted corvid wings with snow-white primary flash & splayed wingtip fingers!
    draw_wings(pb, &pose, &pal);
    // 4) Glossy blue-black torso with iconic blazing snow-white magpie scapular shoulder crescents!
    draw_torso(pb, &pose, &pal);
    // 5) Tapered corvid neck, glossy skull, piercing amber eyes & yellow-hilted chisel bill!
    draw_head(pb, &pose, &pal);
    // 6) State-specific flourishes & weak-point indicators!
    match pose.state {
        MagpieState::Gust | MagpieState::GustWindup => draw_gust_wake(pb, &pose),
        MagpieState::Stuck => draw_buried_glint(pb, &pose),
        MagpieState::Ruffle => draw_fluff_motes(pb, &pose, &pal),
        MagpieState::Death => draw_death_molt(pb, &pose, &pal),
        _ => {}
    }
    // Phase-3 moulting loose feather wisps drifting tailward!
    if pose.enraged > 0.75 && pose.state != MagpieState::Death {
        draw_phase3_feathers(pb, &pose, &pal);
    }
}

/// Dark scaly corvid claws/talons peeking beneath the belly on the planks (or clutching the crate).
fn draw_talons(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    if pose.death > 0.65 {
        return;
    }
    let leg_col: Rgb = (44, 42, 48);
    let claw_col: Rgb = (178, 174, 166);
    let tucked = pose.altitude > 4.0;
    for &side in &[-1.0f32, 1.0] {
        let step = if pose.state == MagpieState::Perch {
            (pose.anim * 4.2 + if side < 0.0 { PI } else { 0.0 }).sin() * 1.6
        } else {
            0.0
        };
        let u_base = if tucked { -2.5 } else { 1.2 + step };
        let foot = pose.place(u_base, side * 5.8) + Vec2::new(0.0, if tucked { 2.5 } else { 1.4 });
        // Three splayed dark corvid toes with pale horn talons!
        for &ang_off in &[-0.50f32, 0.0, 0.50] {
            let dir = pose.f.rotate(ang_off);
            let tip = foot + dir * 3.4;
            draw_segment(pb, foot, tip, 0.7, pal.outline, 0.80 * pose.alpha);
            draw_segment(pb, foot, foot + dir * 2.4, 0.5, leg_col, 0.95 * pose.alpha);
            blend_px(pb, tip.x.round() as i32, tip.y.round() as i32, claw_col, 0.90 * pose.alpha);
        }
    }
}

fn draw_wings(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 32.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    for &side in &[-1.0f32, 1.0] {
        let flap = pose.flap * 0.22;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = pose.local(x, y);
                if v * side < 1.8 {
                    continue;
                }
                let (d, span, chord) = wing_sdf(pose, side, u, v);
                let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    if d < 1.15 {
                        blend_px(pb, x, y, pal.outline, 0.82 * pose.alpha);
                    }
                    continue;
                }
                // Feather barb striping across the wing span!
                let barb = ((span * 14.0 + chord * 3.0).sin()) * 0.14;
                let lum = pose.lit(0.32, side * 0.45) * 0.72 + flap + barb;
                let mut c = shade3(pal.shadow, pal.back, pal.gloss, lum);

                // Iconic Magpie **bold snow-white wing primary flash** on the mid-outer flight feathers!
                let white_flash = if pose.spread > 0.20 {
                    let s_band = smoothstep(0.32, 0.44, span) * (1.0 - smoothstep(0.76, 0.90, span));
                    let c_band = smoothstep(0.18, 0.34, chord) * (1.0 - smoothstep(0.72, 0.88, chord));
                    s_band * c_band * (0.65 + 0.35 * pose.spread)
                } else {
                    // Folded wing shows crisp white scapular wing-bar edge!
                    smoothstep(0.15, 0.35, span) * (1.0 - smoothstep(0.55, 0.75, span)) * 0.75
                };
                if white_flash > 0.02 {
                    let wcol = shade3(pal.white_sh, pal.white, (255, 255, 255), lum * 0.8);
                    c = mix(c, wcol, white_flash.clamp(0.0, 0.96));
                }

                // Peacock iridescent teal-emerald (or P2/P3 violet!) sheen on the upper wing coverts!
                let covert_band = smoothstep(0.08, 0.25, span)
                    * (1.0 - smoothstep(0.45, 0.65, span))
                    * smoothstep(0.45, 0.75, chord);
                if covert_band > 0.05 {
                    c = mix(c, shade3(pal.sheen, pal.sheen_hi, pal.white, lum * 0.5), covert_band * 0.68);
                }

                // Sleek leading-edge specular gloss highlight!
                if d > -1.35 && chord > 0.52 {
                    c = mix(c, pal.gloss, 0.48);
                }
                // Phase-2/3 crimson battle nick near the shoulder!
                if pose.enraged > 0.4 && side > 0.0 && (u - 3.0).abs() < 1.2 && (v - 7.0).abs() < 1.4 {
                    c = mix(c, (214, 48, 52), 0.78);
                }

                let final_c = apply_hit(c, pose.hit);
                if cover < 1.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, pal.outline, 0.82 * pose.alpha);
                    pb.plot(x, y, mix(rimmed, final_c, cover * pose.alpha));
                } else {
                    blend_px(pb, x, y, final_c, pose.alpha);
                }
            }
        }

        // Five crisp splayed primary "finger" flight feathers at the spread wingtip!
        if pose.spread > 0.48 {
            let flap_pull = pose.flap * 2.2 * pose.spread;
            let tip = pose.place(
                1.5 - 17.5 * (1.0 - pose.spread) + 8.5 * pose.sweep + flap_pull,
                side * (4.6 + 17.8 * pose.spread),
            );
            for k in 0..5 {
                let t = k as f32 / 4.0;
                let base = tip + pose.f * ((0.5 - t) * 4.8) - pose.p * (side * t * 2.4);
                let fan_dir = (pose.p * side * (0.75 + 0.25 * (1.0 - t))
                    + pose.f * (0.25 - t * 0.85))
                    .normalized();
                let end = base + fan_dir * (3.6 + (1.0 - (t - 0.35).abs()) * 2.4);
                draw_segment(pb, base, end, 0.95, pal.outline, 0.85 * pose.alpha);
                draw_segment(
                    pb,
                    base,
                    base + fan_dir * 2.2,
                    0.55,
                    apply_hit(pal.white, pose.hit),
                    0.95 * pose.alpha,
                );
                draw_segment(
                    pb,
                    base + fan_dir * 2.0,
                    end,
                    0.55,
                    apply_hit(pal.back, pose.hit),
                    0.95 * pose.alpha,
                );
            }
        }
    }
}

/// Long graduated peacock-iridescent corvid tail with crisp dark outline & central quill lines!
fn draw_tail(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 32.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body - pose.f * 10.0, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            if u > -2.5 {
                continue;
            }
            let (d, along) = tail_sdf(pose, u, v);
            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.15 {
                    blend_px(pb, x, y, pal.outline, 0.82 * pose.alpha);
                }
                continue;
            }
            let lum = pose.lit(-0.35, (v / 6.0).clamp(-1.0, 1.0)) * 0.65;
            // Distinct individual rectrix feather quills running down the graduated tail!
            let quill = ((v * 1.45).sin()).abs();
            let iridescence = (0.35 + 0.55 * quill) * (1.0 - along * 0.35);
            let mut c = shade3(pal.shadow, pal.back, pal.sheen, lum);
            c = mix(c, shade3(pal.sheen, pal.sheen_hi, pal.white, lum * 0.6), iridescence * 0.72);
            // Crisp dark feather shaft seams between tail feathers.
            if quill < 0.16 {
                c = mix(c, pal.shadow, 0.58);
            }
            // Subtle glossy dark band near the diamond tail tip.
            if along > 0.84 {
                c = mix(c, pal.shadow, (along - 0.84) * 3.8);
            }
            let final_c = apply_hit(c, pose.hit);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, pal.outline, 0.82 * pose.alpha);
                pb.plot(x, y, mix(rimmed, final_c, cover * pose.alpha));
            } else {
                blend_px(pb, x, y, final_c, pose.alpha);
            }
        }
    }
}

/// Sculpted glossy blue-black corvid torso with iconic blazing snow-white scapular crescents!
fn draw_torso(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 20.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let d = torso_sdf(pose, u, v);
            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.3 {
                    blend_px(pb, x, y, pal.outline, 0.92 * pose.alpha);
                }
                continue;
            }
            let nu = (u / 9.0).clamp(-1.0, 1.0);
            let nv = (v / 6.6).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.65)).clamp(0.0, 1.0);
            let lit = pose.lit(nu, nv);
            let mut c = shade3(pal.shadow, pal.back, pal.gloss, lit * 0.85 + dome * 0.30);

            // Subtle peacock iridescence shimmer over the upper mantle!
            let mantle = smoothstep(-6.0, 4.0, u) * (1.0 - smoothstep(4.0, 8.5, u));
            c = mix(c, pal.sheen, mantle * 0.34 * (0.6 + 0.4 * pose.enraged));

            // THE ICONIC MAGPIE SIGNATURE: twin blazing snow-white scapular shoulder crescents
            // sweeping cleanly down both flanks of the back (`u ∈ [-5.5, 4.5], |v| ∈ [2.5, 6.2]`)!
            let scap_d = ellipse_sdf(u, v.abs(), -0.5, 4.5, 5.4, 2.2);
            if scap_d < 0.45 {
                let w_cov = (0.45 - scap_d).clamp(0.0, 1.0);
                let w_col = shade3(pal.white_sh, pal.white, (255, 255, 255), lit * 0.75 + dome * 0.25);
                c = mix(c, w_col, w_cov * 0.96);
            }
            // Crisp white lower rump band just above the iridescent tail!
            if u < -5.8 && u > -8.6 && v.abs() < 3.4 {
                let w_col = shade3(pal.white_sh, pal.white, (255, 255, 255), lit * 0.6);
                c = mix(c, w_col, 0.82);
            }
            // Phase-3 crimson chest scuff!
            if pose.enraged > 0.75 && (u - 4.2).abs() < 1.3 && (v + 1.5).abs() < 1.5 {
                c = mix(c, (218, 52, 48), 0.75);
            }

            let final_c = apply_hit(c, pose.hit);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, pal.outline, 0.84 * pose.alpha);
                pb.plot(x, y, mix(rimmed, final_c, cover * pose.alpha));
            } else {
                blend_px(pb, x, y, final_c, pose.alpha);
            }
        }
    }
}

/// Sleek tapered corvid neck, glossy black skull, piercing golden eyes & yellow-hilted chisel bill!
fn draw_head(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    // 1) Sleek tapered avian neck joining shoulders to skull (with crisp dark outline!).
    let shoulder = pose.body + pose.f * 5.2;
    let join = pose.head - pose.f * 1.2;
    let nd = join - shoulder;
    let nlen = nd.len();
    if nlen > 1.5 {
        let dir = nd * (1.0 / nlen);
        let perp = dir.perp();
        let steps = (nlen * 1.6).ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            // Graceful S-curve taper: thick feathered nape tapering into neck!
            let w = 4.6 - 1.3 * t;
            let p = shoulder + nd * t;
            let wi = (w + 1.2).ceil() as i32;
            for k in -wi..=wi {
                let kf = k as f32;
                let ak = kf.abs();
                if ak > w + 1.0 {
                    continue;
                }
                let q = p + perp * kf;
                let c = if ak > w {
                    pal.outline
                } else {
                    let lit = (kf / w.max(1.0)) * perp.dot(LIGHT);
                    apply_hit(shade3(pal.shadow, pal.back, pal.gloss, lit * 0.8), pose.hit)
                };
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, c, pose.alpha);
            }
        }
    }

    // 2) Sculpted glossy blue-black corvid skull with dark outline & bright crown sheen!
    let reach = 14.0;
    if let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.head, reach) {
        let head_f = if nlen > 2.0 { nd * (1.0 / nlen) } else { pose.f };
        let head_p = head_f.perp();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5) - pose.head.x;
                let dy = (y as f32 + 0.5) - pose.head.y;
                let u = dx * head_f.x + dy * head_f.y;
                let v = dx * head_p.x + dy * head_p.y;
                let d = ellipse_sdf(u, v, 0.0, 0.0, HEAD_RADIUS + 0.8, HEAD_RADIUS - 0.6);
                let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    if d < 1.15 {
                        blend_px(pb, x, y, pal.outline, 0.86 * pose.alpha);
                    }
                    continue;
                }
                let lit = ((dx / HEAD_RADIUS) * LIGHT.x + (dy / HEAD_RADIUS) * LIGHT.y).clamp(-1.0, 1.0);
                let dome = (1.0 - ((u / HEAD_RADIUS).powi(2) + (v / HEAD_RADIUS).powi(2)) * 0.55).clamp(0.0, 1.0);
                let mut c = shade3(pal.shadow, pal.back, mix(pal.gloss, pal.sheen_hi, 0.35), lit * 0.85 + dome * 0.30);
                // Bright glossy crown highlight catching top-left light!
                if u < 0.8 && u > -3.2 && lit > 0.35 && dome > 0.50 {
                    c = mix(c, pal.white, 0.32);
                }
                // Warm golden glow wash on the head while buried (weak point!).
                if pose.buried {
                    let pulse = 0.5 + 0.5 * (pose.anim * 12.0).sin();
                    c = mix(c, (255, 224, 128), 0.16 + 0.14 * pulse);
                }
                let final_c = apply_hit(c, pose.hit);
                if cover < 1.0 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    let rimmed = mix(bg, pal.outline, 0.86 * pose.alpha);
                    pb.plot(x, y, mix(rimmed, final_c, cover * pose.alpha));
                } else {
                    blend_px(pb, x, y, final_c, pose.alpha);
                }
            }
        }
    }

    // 3) Piercing corvid eyes (fierce gold-amber iris + white catchlight; closed in Death!).
    let dead_closed = pose.death > 0.42;
    let eye_col = if pose.enraged > 0.4 {
        mix((255, 178, 42), (255, 84, 38), 0.45 + 0.35 * pose.critical)
    } else {
        (255, 218, 86)
    };
    for &side in &[-1.0f32, 1.0] {
        let eye = pose.head + pose.f * 2.0 + pose.p * (side * 3.1);
        let ex = eye.x.round() as i32;
        let ey = eye.y.round() as i32;
        if dead_closed {
            blend_px(pb, ex - 1, ey, (72, 68, 72), 0.85 * pose.alpha);
            blend_px(pb, ex, ey, (72, 68, 72), 0.85 * pose.alpha);
            continue;
        }
        // Dark fierce brow shadow framing the eye.
        blend_px(pb, ex, ey + 1, pal.outline, 0.85 * pose.alpha);
        blend_px(pb, ex, ey, eye_col, 0.98 * pose.alpha);
        blend_px(pb, ex + (pose.f.x.round() as i32), ey + (pose.f.y.round() as i32), (255, 252, 236), 0.82 * pose.alpha);
    }

    // 4) Heavy yellow-hilted corvid chisel bill (or jammed stub disappearing into the oak plank!).
    let face = pose.head + pose.f * (HEAD_RADIUS * 0.65);
    let bill_len = if pose.buried { 5.2 } else { BEAK_LENGTH };
    let tip = face + pose.f * bill_len;
    let d = tip - face;
    let len = d.len().max(1.0);
    let dir = d * (1.0 / len);
    let perp = dir.perp();
    let steps = (len * 1.8).ceil() as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let w = (1.0 - t * if pose.buried { 0.45 } else { 0.88 }) * 3.0;
        let p = face + d * t;
        let edge = (w + 1.0).ceil() as i32;
        for k in -edge..=edge {
            let kf = k as f32;
            let ak = kf.abs();
            if ak > w + 0.85 {
                continue;
            }
            let q = p + perp * kf;
            let c = if ak > w {
                pal.outline
            } else if t < 0.22 {
                // Iconic pale yellow-ivory bill cere/hilt at the base of the beak!
                apply_hit(mix(pal.hilt, (255, 248, 212), (1.0 - ak / w.max(0.5)) * 0.5), pose.hit)
            } else if ak < 0.65 {
                // Gleaming pale culmen ridge running down the center of the chisel bill!
                apply_hit(mix(pal.bill, (218, 216, 224), 0.68), pose.hit)
            } else {
                let side_lit = (kf / w.max(0.5)) * perp.dot(LIGHT);
                apply_hit(shade3(pal.outline, pal.bill, (108, 110, 124), side_lit * 0.8), pose.hit)
            };
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, c, pose.alpha);
        }
    }
}

/// Unmistakable golden-white rotating weak-point reticle & glint around the buried head in `Stuck`!
fn draw_buried_glint(pb: &mut PixelBuffer, pose: &Pose) {
    let c = pose.head;
    let pulse = 0.55 + 0.45 * (pose.anim * 12.0).sin();
    let r = HEAD_RADIUS + 3.2 + pulse * 1.4;
    draw_ring(pb, c, r, (255, 222, 92), (0.65 + 0.30 * pulse) * pose.alpha);
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + pose.anim * 1.6;
        let dir = Vec2::new(a.cos(), a.sin());
        let p0 = c + dir * (r + 3.0);
        let p1 = c + dir * (r - 0.8);
        draw_segment(pb, p0, p1, 0.7, (255, 250, 212), 0.92 * pose.alpha);
    }
}

/// Tapered icy slipstream wake ribbons peeling off the beating wings during `Gust`!
fn draw_gust_wake(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.spread < 0.3 {
        return;
    }
    let from = pose.body - pose.f * 1.5;
    for &side in &[-1.0f32, 1.0] {
        let tip = pose.place(-3.0, side * (5.0 + 18.0 * pose.spread));
        for k in 0..4 {
            let t = (k as f32 + 1.0) / 4.5;
            let a = from + (tip - from) * t;
            let b = a + GUST_AXIS * (7.0 + 6.0 * t) - pose.p * (side * 1.8);
            draw_segment(
                pb,
                a,
                b,
                0.75,
                mix((240, 248, 255), (146, 186, 224), t),
                0.62 * pose.alpha,
            );
        }
    }
}

/// Floating moulted black-and-white feather quills bursting during `Ruffle`!
fn draw_fluff_motes(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    for k in 0..10 {
        let seed = hash2(k, 0, 31);
        let a = k as f32 * 0.62 + pose.progress * 4.5 + seed * 2.0;
        let r = 8.0 + 14.0 * pose.progress + seed * 6.0;
        let p = pose.body + Vec2::new(a.cos(), a.sin() * 0.75) * r;
        let q = p + Vec2::from_angle(a + 1.2) * 2.8;
        let c = if k % 2 == 0 { pal.white } else { pal.back };
        draw_segment(pb, p, q, 0.65, c, (1.0 - pose.progress) * 0.85);
    }
}

/// Phase-3 loose moulting feathers drifting tailward in the flatbed slipstream!
fn draw_phase3_feathers(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    for k in 0..4 {
        let phase = (pose.anim * 1.8 + k as f32 * 0.25).fract();
        let p = pose.body
            + GUST_AXIS * (8.0 + phase * 26.0)
            + Vec2::new(0.0, ((k as f32 - 1.5) * 6.5) + (phase * 7.0).sin() * 2.5);
        let q = p + Vec2::from_angle(phase * 5.0 + k as f32) * 2.5;
        let c = if k % 2 == 0 { pal.white } else { pal.sheen_hi };
        draw_segment(pb, p, q, 0.6, c, (1.0 - phase) * 0.78);
    }
}

/// Dramatic death moult: swirling black, white & iridescent magpie feathers drifting in the wind!
fn draw_death_molt(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let t = pose.death;
    if t < 0.14 {
        return;
    }
    let spread = ((t - 0.14) / 0.86).clamp(0.0, 1.0);
    for k in 0..16 {
        let seed = hash2(k, 1, 17);
        let ang = seed * std::f32::consts::TAU + t * 2.2;
        let r = 5.0 + 28.0 * spread * (0.45 + seed * 0.75);
        // Feathers drift slightly tailward (-x) in the flatbed slipstream!
        let drift = GUST_AXIS * (spread * 12.0 * (0.4 + seed));
        let p = pose.body + Vec2::new(ang.cos(), ang.sin() * 0.75) * r + drift;
        let spin = ang + t * 6.0;
        let dir = Vec2::from_angle(spin);
        let c = match k % 4 {
            0 | 1 => pal.white,
            2 => pal.sheen_hi,
            _ => pal.back,
        };
        let a = (1.0 - spread * 0.75) * 0.92;
        draw_segment(pb, p - dir * 1.8, p + dir * 1.8, 0.75, pal.outline, a * 0.6);
        draw_segment(pb, p - dir * 1.5, p + dir * 1.5, 0.55, c, a);
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

/// A crisp line of blended pixels/dots.
fn draw_segment(pb: &mut PixelBuffer, a: Vec2, b: Vec2, radius: f32, c: Rgb, alpha: f32) {
    let d = b - a;
    let len = d.len_sq().sqrt();
    let steps = (len * 1.4).ceil() as i32;
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

/// Detailed strapped wooden shipping crate plummeting from the Magpie's talons!
fn draw_crate(pb: &mut PixelBuffer, at: Vec2, drop: f32, alpha: f32) {
    let half = CRATE_HALF - 1.5;
    let h = half.round() as i32;
    let cx = at.x.round() as i32;
    let cy = at.y.round() as i32;
    let a = alpha * (0.72 + 0.28 * drop);

    for dy in -(h + 1)..=(h + 1) {
        for dx in -(h + 1)..=(h + 1) {
            let ax = dx.abs();
            let ay = dy.abs();
            if ax == h + 1 || ay == h + 1 {
                blend_px(pb, cx + dx, cy + dy, (10, 8, 6), a * 0.85);
                continue;
            }
            let lit = (-dx as f32 * 0.55 - dy as f32 * 0.83) / half.max(1.0);
            let corner = ax >= h - 2 && ay >= h - 2;
            let frame = ax >= h - 1 || ay >= h - 1;
            let diag = (dx - dy).abs() <= 1;
            let c = if corner {
                // Iron shipping crate corner brackets!
                shade3((28, 32, 38), (74, 82, 92), (146, 158, 172), lit * 0.8)
            } else if frame || diag {
                // Raised wooden framing slats & diagonal brace!
                shade3((76, 50, 26), (146, 102, 56), (212, 162, 98), lit * 0.7)
            } else {
                // Recessed oak plank panels with plank seam lines!
                let seam = dy % 3 == 0;
                let base = if seam { (88, 60, 34) } else { (128, 88, 48) };
                shade3((62, 42, 22), base, (186, 138, 82), lit * 0.5)
            };
            blend_px(pb, cx + dx, cy + dy, c, a);
        }
    }
    // Crisp falling wind-streaks streaming upward above the plummeting crate!
    if drop > 0.12 {
        for &ox in &[-4, 0, 4] {
            for k in 0..5 {
                let y = cy - h - 2 - k;
                blend_px(
                    pb,
                    cx + ox,
                    y,
                    (244, 236, 212),
                    a * drop * (0.68 - 0.12 * k as f32).max(0.0),
                );
            }
        }
    }
}

/// The dropped crate, drawn as part of the boss's entity layer.
pub fn draw_magpie_crate(pb: &mut PixelBuffer, view: &MagpieView, cam: Vec2) {
    if view.state != MagpieState::SnatchDrop {
        return;
    }
    if view.progress >= CRATE_LAND_PROGRESS {
        return;
    }
    let at = view.target - cam;
    let t = (view.progress / CRATE_LAND_PROGRESS).clamp(0.0, 1.0);
    let start = view.pos - cam + Vec2::new(0.0, -16.0);
    let fall = t * t * (3.0 - 2.0 * t);
    let p = start + (at - start) * fall;
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

    /// Capture every Magpie state through the real `World::draw` on the
    /// flatbed with `LightingMode::Full` and `LightingMode::Off`.
    /// Opt-in: `RATRUN_MAGPIE_GALLERY=<dir> cargo test magpie_world_gallery`.
    #[test]
    fn magpie_world_gallery() {
        let Ok(dir) = std::env::var("RATRUN_MAGPIE_GALLERY") else {
            return;
        };
        use crate::game::ecs::components::SpawnId;
        use crate::game::level::{Theme, TileRect};
        use crate::game::world::World;
        use crate::render::lighting::LightingMode;

        std::fs::create_dir_all(&dir).expect("create magpie gallery dir");
        let shots: &[(&str, MagpieState, f32, f32)] = &[
            ("01_perch", MagpieState::Perch, 0.45, 1.0),
            ("02_ruffle", MagpieState::Ruffle, 0.55, 1.0),
            ("03_peck_windup", MagpieState::PeckWindup, 0.85, 1.0),
            ("04_peck", MagpieState::Peck, 0.65, 1.0),
            ("05_stuck", MagpieState::Stuck, 0.45, 0.82),
            ("06_takeoff", MagpieState::Takeoff, 0.55, 0.72),
            ("07_strafe_windup", MagpieState::StrafeWindup, 0.80, 0.62),
            ("08_strafe_pass", MagpieState::StrafePass, 0.48, 0.58),
            ("09_snatch_drop", MagpieState::SnatchDrop, 0.45, 0.52),
            ("10_land", MagpieState::Land, 0.55, 0.45),
            ("11_gust_windup", MagpieState::GustWindup, 0.75, 0.32),
            ("12_gust_pulse", MagpieState::Gust, 0.08, 0.22),
            ("13_gust_calm", MagpieState::Gust, 0.22, 0.22),
            ("14_death", MagpieState::Death, 0.68, 0.0),
        ];
        let (w_px, h_px) = (160usize, 120usize);
        for &(name, state, prog, hp_frac) in shots {
            for &(lit, suffix) in &[(LightingMode::Full, ""), (LightingMode::Off, "_off")] {
                let mut w = World::new(Theme::Trailer, 11);
                w.level_index = crate::game::campaign::LEVEL_COUNT;
                w.lighting = lit;
                let arena = w
                    .level
                    .boss_room()
                    .map(|r| r.rect)
                    .expect("the flatbed's cab hold");
                let centre = arena.center_world();
                w.player.pos = centre + Vec2::new(-16.0, 22.0);
                let pos = centre + Vec2::new(-12.0, -4.0);
                let mut boss = crate::game::ecs::systems::boss::Boss::new(
                    crate::game::ecs::systems::boss::BossKind::Magpie,
                    pos,
                    1600.0,
                    TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows()),
                    centre,
                    false,
                );
                boss.facing = Vec2::new(1.0, 0.0);
                boss.target = match state {
                    MagpieState::Peck | MagpieState::Stuck => pos + Vec2::new(28.0, 4.0),
                    MagpieState::SnatchDrop => pos + Vec2::new(26.0, 12.0),
                    _ => pos + Vec2::new(42.0, 4.0),
                };
                boss.pounce_origin = pos - Vec2::new(42.0, 0.0);
                boss.set_state(
                    crate::game::ecs::systems::boss::BossState::Magpie(state),
                    1.4,
                );
                boss.elapsed = prog * 1.4;
                boss.hp = 1600.0 * hp_frac;
                boss.phase = if hp_frac <= PHASE3_HP_FRAC {
                    3
                } else if hp_frac <= PHASE2_HP_FRAC {
                    2
                } else {
                    1
                };
                w.ecs.spawn((boss, SpawnId(9001)));
                let mut pb = PixelBuffer::new(w_px, h_px);
                w.draw(&mut pb);
                let path = format!("{dir}/w_{name}{suffix}.ppm");
                write_ppm(&path, &pb);
            }
        }
    }
}
