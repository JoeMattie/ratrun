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
//! | 1 | `> 0.62` | `Soar` → (`Swoop`) → `Land` → `PeckWindup` → `Peck` → `Stuck` | the shadow circling, then a descent: a beak line on the planks — sidestep it, then bite the buried head |
//! | 2 | `0.30..=0.62` | `Soar` → `Takeoff` → `StrafeWindup` → `StrafePass` → `SnatchDrop` → `Land` (alternating with `Peck`) | a shadow lane across the bed: be on the far side of it; then a marked square where the crate comes down |
//! | 3 | `<= 0.30` | `Soar` → `Land` → `GustWindup` → `Gust` → `Peck` / `StrafePass` | wing-beat gusts shove everything tailward in pulses with calm gaps; pecks in the gaps |
//!
//! ## The bird flies
//!
//! `Soar` is the neutral beat and it is **airborne**: the bird circles and
//! banks over the deck around the rat, holding [`SOAR_RING`] px, riding the
//! slipstream tailward and flapping back upwind for it, and it crosses cargo
//! stacks and holes in the planks as if they were not there. Flight is
//! [`SOAR_SPEED`] (110-140 px/s), faster than the rat's 58 px/s walk at every
//! phase, so a bird that wants the rat always has it.
//!
//! It comes down only to work. A peck is preceded by `Land`: a short, visible
//! descent onto a perch spot in the pecking band ([`DESCENT_MIN`]..
//! [`DESCENT_MAX`] s at the bird's own flight speed), then the unchanged
//! `PeckWindup` tell. The gust is flown the same way (it needs its wings braced
//! against the planks). A strafing pass is flown from the air. Every one of
//! those attack beats is drawn, telegraphed and timed exactly as it was before
//! the bird learned to fly.
//!
//! Now and then a soaring beat ends in [`MagpieState::Swoop`]: a low, fast
//! wing-over that skims past the rat at [`SWOOP_PASS_OFFSET`] px and climbs back
//! out. It deals nothing (the fly-by is not an attack), and it keeps the fight
//! moving when the rat has learnt to read the ring.
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
//! * [`body_hit_circles`] — three circles: tail root, body, head. On the deck
//!   they are **grounded** (the rat's game reaches them, and the buried head is
//!   **displaced to the strike point**, which is the whole point of the punish
//!   window). Airborne they are drawn but ungrounded: nothing on the deck
//!   reaches a bird in the air, so the landing is the damage window. The plate
//!   says so while it flies ([`OVERHEAD_TAG`]).
//! * [`is_airborne`] / [`out_of_reach`] — [`MagpieState::flying`]: `Soar`,
//!   `Swoop`, `Takeoff`, `StrafeWindup`, `StrafePass`, `SnatchDrop`. Body
//!   contact is off, the sprite is drawn at [`FLIGHT_LIFT`] over its own ground
//!   shadow, and the ground shadow goes soft and small.
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

use std::f32::consts::{PI, TAU};

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
///
/// Only `Perch`, the landed watch, still reads it: the neutral beat is flown
/// (`Soar`) and the flight is already faster than the rat's walk.
pub const HUNT_SPEED_MULT: f32 = 1.9;

// ---- the neutral beat: flight ---------------------------------------------

/// The bird's beat between attacks (`Soar`): the old perched watch, flown.
pub const SOAR_DURATION: [f32; 3] = PERCH_DURATION;

/// Flight speed per phase (`110 / 125 / 140` px/s). Over the rat's 58 px/s
/// walk at every phase and under the strafing pass, so a bird that wants the
/// rat can always have it: a boss cannot be outrun out of its own fight.
pub const SOAR_SPEED: [f32; 3] = [110.0, 125.0, 140.0];

/// The ring `Soar` holds the rat at, in px (`near`, `far`): the pecking band's
/// flight equivalent, wide enough that the rat can read the shadow and always
/// inside the bird's `PECK_REACH` after one more descent.
pub const SOAR_RING: [f32; 2] = [56.0, 84.0];

/// Past the ring by more than this, the bird stops arcing and comes straight
/// at the rat at full flight speed (the recovery from an arrival across the
/// deck).
pub const SOAR_CHASE_BAND: f32 = 40.0;

/// The slipstream's share of a soaring bird's ground track (px/s, toward the
/// tailgate): it rides the deck's own wind and flaps back upwind for it, which
/// is why the ring trails slack on the downwind side.
pub const SOAR_DRIFT: f32 = 34.0;

/// The low fly-by (`Swoop`): speed per phase and its beat.
pub const SWOOP_SPEED: [f32; 3] = [140.0, 150.0, 160.0];
pub const SWOOP_DURATION: f32 = 0.55;

/// How far abeam of the rat the fly-by passes (px). Outside the rat's hurt
/// radius by a wide margin, so the pass is never a hit — but close enough to
/// be a pass.
pub const SWOOP_PASS_OFFSET: f32 = 48.0;

/// How far along the rat the fly-by's line ends (px).
pub const SWOOP_RUN: f32 = 80.0;

/// How high a flying bird is drawn above its own shadow (px): the lift
/// `Takeoff` climbs to, and the height every airborne state flies at. `Land`
/// ramps it back down to the planks.
pub const FLIGHT_LIFT: f32 = 13.0;

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

/// The descent into a peck (`Land` flown at the bird's own flight speed): the
/// beat is the distance over the speed, held inside these bounds so the
/// descent is always short and always flies no faster than the bird can.
pub const DESCENT_MIN: f32 = 0.35;
pub const DESCENT_MAX: f32 = 1.20;

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
    /// **The bird in the air.** The neutral beat: it circles and banks over the
    /// deck around the rat, holding [`SOAR_RING`] px, riding the slipstream
    /// tailward and flapping back upwind for it, and crossing cargo and holes as
    /// if they were not there — because, from up here, they are not.
    ///
    /// It is **airborne**: nothing grounded reaches it (`Boss::hit_shapes`),
    /// and body contact is off. The counterplay is the descent: it lands to
    /// peck, and that is the damage window.
    ///
    /// - Duration: [`SOAR_DURATION`] (`1.30 / 1.10 / 0.95` s) per beat.
    /// - Warning: the shadow, always (`draw_magpie_ground`).
    /// - Dangerous area: none (it does not touch the rat from up here).
    Soar,

    /// **The wing-over.** Now and then a beat ends in a low, fast pass: the bird
    /// drops to the planks' height and skims past the rat at
    /// [`SWOOP_PASS_OFFSET`] px, then climbs back out into the next `Soar`.
    ///
    /// - Duration: [`SWOOP_DURATION`] (0.55 s).
    /// - Warning: the shadow tightens onto the planks and the wings sweep back.
    /// - Dangerous area: none — it is a fly-by, not an attack.
    Swoop,

    /// **Hop / watch.** The landed pose: the bird stands on the deck, close to
    /// the rat, with the head-bob of a bird deciding where to stab.
    ///
    /// The neutral beat is [`Soar`](Self::Soar) now, so this is the rig's
    /// ground watch — the pose the landing settles out of, the Library card and
    /// the behaviour tests that drive the deck-peck by hand.
    ///
    /// - Duration: [`PERCH_DURATION`] (`1.30 / 1.10 / 0.95` s).
    /// - Warning: none.
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
    /// Is the bird off the deck (no body contact, no grounded hit shape, soft
    /// shadow)?
    ///
    /// Airborne is a **contract**, not a pose: every state listed here is drawn
    /// at [`FLIGHT_LIFT`] px over its own shadow and is out of every grounded
    /// weapon's reach (`Boss::is_airborne`, `Boss::hit_shapes`), and no state
    /// outside the list may be drawn off the planking.
    #[inline]
    pub fn flying(self) -> bool {
        matches!(
            self,
            MagpieState::Soar
                | MagpieState::Swoop
                | MagpieState::Takeoff
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

/// Is the bird out of every grounded weapon's reach right now?
///
/// The same rule as [`is_airborne`], named for the HUD: the plate says *why*
/// the rat's ground game is finding nothing (see
/// [`OVERHEAD_TAG`] and `render::hud::boss_plate`).
#[inline]
pub fn out_of_reach(view: &MagpieView) -> bool {
    view.state.flying()
}

/// The plate's second line while the bird is flying: the fight tells the rat
/// *why* its weapons are finding nothing.
pub const OVERHEAD_TAG: &str = "IN THE AIR — OUT OF REACH";

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
    /// `-1..=1`: the bank of a turning bird. Read only by the flying states;
    /// the controller hands it the side of the bird's own orbit.
    pub bank: f32,
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
            bank: 0.0,
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

/// Most a **grounded** bird may lift off the deck while it is still drawn over
/// its own hit shapes: the body radius is ~7 px, so a few pixels of lift keeps
/// the sprite on the circles the rat can actually hit.
///
/// Airborne states (`MagpieState::flying`) are not bound by it: their shapes are
/// ungrounded, so nothing on the deck can reach them and the sprite is drawn at
/// its true [`FLIGHT_LIFT`]. Neither is `Land`: it is the descent, drawn from
/// the flight lift down to the planking while its shapes are already the rat's
/// to hit again.
const HIT_LIFT_MAX: f32 = 3.0;

/// Is this state drawn in the air? Every airborne state, plus the descent that
/// flies the bird back down onto the deck.
#[inline]
fn drawn_airborne(state: MagpieState) -> bool {
    state.flying() || state == MagpieState::Land
}

/// Period of one complete `Soar` burst-flap + glide cycle, in seconds.
///
/// Two articulated wingbeats run over the first 68 % of the cycle and ease
/// smoothly into a full-span soaring glide over the remaining 32 %, keyed only
/// to `MagpieView::anim`.
const SOAR_CYCLE_SECS: f32 = 1.50;

/// Articulated wing kinematics sampled from the continuous `Soar` flap-and-glide cycle.
#[derive(Clone, Copy, Debug)]
struct FlapState {
    /// Power stroke (`+` downstroke, `-` upstroke, `0` level glide).
    stroke: f32,
    /// Span multiplier (`~0.62..1.11`: full reach on downstroke/glide, tucked on upstroke recovery).
    span_scale: f32,
    /// Fore/aft sweep offset (`+` forward catch, `-` recovery/glide sweep).
    sweep_delta: f32,
    /// Carpal wrist bend (`0..1`: straight on downstroke/glide, bent inward on upstroke).
    wrist_fold: f32,
    /// Wingtip primary finger splay (`0.28..1.0`: wide on downstroke/glide, closed on upstroke).
    finger_splay: f32,
    /// Vertical lift bob in px (`+` lifts body on the power downstroke).
    body_bob: f32,
    /// Wing shading bias across the beat.
    shade_bias: f32,
    /// Glide fraction (`0` during active flap burst, `1` during pure soaring glide).
    glide: f32,
}

/// Sample the `C¹`-continuous periodic burst-flap and glide cycle for `Soar`.
#[inline]
fn soar_flap_kinematics(anim: f32) -> FlapState {
    let a = if anim.is_finite() { anim } else { 0.0 };
    let t = (a / SOAR_CYCLE_SECS).rem_euclid(1.0);
    let env_in = smoothstep(0.0, 0.07, t);
    let env_out = 1.0 - smoothstep(0.59, 0.68, t);
    let env = env_in * env_out;
    let theta = (t / 0.68).clamp(0.0, 1.0) * 2.0 * TAU;
    let (s, c) = theta.sin_cos();
    let up_tuck = (0.5 - 0.5 * (theta - 0.35).sin()).powi(2);
    let stroke = env * (s + 0.18 * (2.0 * theta).sin());
    let wrist_fold = env * up_tuck;
    let span_scale = 1.0 + env * (0.11 * s - 0.38 * up_tuck);
    let sweep_delta = env * (0.28 * c - 0.30 * up_tuck) - 0.08 * (1.0 - env);
    let finger_splay = (1.0 - 0.68 * wrist_fold).clamp(0.28, 1.0);
    let body_bob = env * (1.45 * s - 0.35 * c) + (1.0 - env) * (0.35 * (t * TAU).sin());
    let shade_bias = env * (-0.22 * s + 0.16 * c);
    FlapState {
        stroke,
        span_scale,
        sweep_delta,
        wrist_fold,
        finger_splay,
        body_bob,
        shade_bias,
        glide: 1.0 - env,
    }
}

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
    /// Dynamic wing span multiplier from the flap cycle (`~0.62..1.11`).
    span_scale: f32,
    /// Carpal wrist fold (`0..1`) during upstroke recovery / sleek dive tuck.
    wrist_fold: f32,
    /// Cupped braking wing camber (`0..1`) during `Land` / `SnatchDrop`.
    wing_cup: f32,
    /// Primary wingtip finger splay (`0.2..1.0`).
    finger_splay: f32,
    /// Effective bank roll (`-1..=1`): inner wing (`side * bank > 0`) foreshortens,
    /// outer wing extends long, tail fans and twists into the turn.
    bank: f32,
    /// Lateral rudder twist of the tail tip along `+p` (px).
    tail_twist: f32,
    /// Downward pitch of the fanned tail during landing brake (`0..1`).
    tail_drop: f32,
    /// Landing gear leg drop extension (`0..1`) during `Land` / `Takeoff` / `SnatchDrop`.
    leg_drop: f32,
    /// Flap-phase shading bias on the wings.
    shade_bias: f32,
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
    /// Tail fan spread (`0..1`, wide on Gust / Takeoff / Ruffle / Land / banked Soar).
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
        let raw_bank = if view.bank.is_finite() {
            view.bank.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let spread: f32;
        let mut sweep = 0.0f32;
        let mut flap = 0.0f32;
        let mut span_scale = 1.0f32;
        let mut wrist_fold = 0.0f32;
        let mut wing_cup = 0.0f32;
        let mut finger_splay = 0.85f32;
        let mut eff_bank = 0.0f32;
        let mut tail_twist = 0.0f32;
        let mut tail_drop = 0.0f32;
        let mut leg_drop = 0.0f32;
        let mut shade_bias = 0.0f32;
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
            MagpieState::Soar => {
                // Real burst-flap + glide cycle with true roll banking and rudder tail!
                let fk = soar_flap_kinematics(anim);
                eff_bank = raw_bank;
                let b_abs = eff_bank.abs();
                altitude = FLIGHT_LIFT + fk.body_bob;
                spread = 0.96;
                span_scale = fk.span_scale;
                sweep = fk.sweep_delta;
                flap = fk.stroke;
                wrist_fold = fk.wrist_fold;
                finger_splay = fk.finger_splay;
                shade_bias = fk.shade_bias;
                tail_fan = 0.44 + 0.38 * b_abs + 0.10 * fk.glide;
                tail_twist = eff_bank * 5.0;
                tail_lift = 0.14 * (anim * 1.4).sin() + 0.18 * fk.stroke;
                head_lunge = 0.32;
            }
            MagpieState::Swoop => {
                // Low fast diving wing-over: wings swept sharply back, streamlined
                // silhouette dipping close to the planks and climbing back out.
                let dip = (prog * PI).sin();
                let exit_flare = smoothstep(0.76, 1.0, prog);
                eff_bank = raw_bank * (0.45 + 0.30 * (1.0 - dip));
                altitude = FLIGHT_LIFT * (1.0 - 0.72 * dip);
                spread = 0.94 - 0.18 * dip + 0.04 * exit_flare;
                span_scale = 0.94 - 0.12 * dip;
                sweep = -0.78 - 0.26 * dip + 0.44 * exit_flare;
                wrist_fold = 0.34 * dip;
                finger_splay = 0.22 + 0.50 * exit_flare;
                flap = (anim * 12.0).sin() * 0.20 * (1.0 - dip);
                tail_fan = 0.12 + 0.24 * exit_flare + 0.16 * eff_bank.abs();
                tail_twist = eff_bank * 3.0;
                tail_lift = -0.18 * dip;
                head_lunge = 0.76 + 0.32 * dip;
                stretch = 0.32 + 0.28 * dip;
                jitter = Vec2::new(0.0, dip * 0.45);
            }
            MagpieState::Takeoff => {
                let t = smoothstep(0.0, 0.75, prog);
                altitude = FLIGHT_LIFT * t;
                spread = (prog / 0.55).min(1.0);
                flap = (anim * 22.0).sin();
                span_scale = 0.96 + 0.08 * flap;
                wrist_fold = (0.5 - 0.5 * flap) * 0.45 * t;
                finger_splay = 0.90 - 0.35 * wrist_fold;
                tail_lift = 0.4 * t;
                tail_fan = 0.75 * t;
                leg_drop = (1.0 - smoothstep(0.15, 0.75, prog)) * 0.85;
                head_lunge = 0.20;
            }
            MagpieState::StrafeWindup => {
                altitude = FLIGHT_LIFT;
                spread = 1.0;
                flap = (anim * 18.0).sin();
                span_scale = 0.96 + 0.06 * flap;
                finger_splay = 0.90;
                tail_lift = 0.15;
                tail_fan = 0.50;
                sweep = -0.22;
                head_lunge = 0.48;
            }
            MagpieState::StrafePass => {
                altitude = FLIGHT_LIFT;
                spread = 0.96;
                span_scale = 0.92;
                // Sleek swept-back stooping dive wings!
                sweep = -0.84;
                wrist_fold = 0.25;
                finger_splay = 0.28;
                flap = (anim * 10.0).sin() * 0.24;
                tail_lift = 0.0;
                tail_fan = 0.22;
                head_lunge = 0.68;
                stretch = 0.35;
            }
            MagpieState::SnatchDrop => {
                // Hard aerial brake with wings thrown forward to wheel and release the crate!
                let brake = smoothstep(0.0, 0.45, prog);
                altitude = FLIGHT_LIFT + 4.0 * (1.0 - brake);
                spread = 1.0;
                sweep = 0.56 * brake;
                wing_cup = 0.78 * brake;
                flap = (anim * 22.0).sin() * brake;
                finger_splay = 0.95;
                tail_lift = -0.35 * brake;
                tail_fan = 0.85 * brake;
                tail_drop = 0.55 * brake;
                leg_drop = 0.70 * brake;
                head_lunge = 0.35;
            }
            MagpieState::Land => {
                // Full-span cupped braking flare, tail fanned down, legs dropping,
                // then a clean settle bridging into Perch and PeckWindup with no pop.
                let desc = smoothstep(0.0, 0.78, prog);
                let flare = ((prog / 0.82).clamp(0.0, 1.0) * PI).sin();
                let fold = smoothstep(0.56, 1.0, prog);
                eff_bank = raw_bank * (1.0 - smoothstep(0.0, 0.45, prog));
                altitude = FLIGHT_LIFT * (1.0 - desc);
                spread = 0.96 * (1.0 - fold) + 0.18 * fold + 0.04 * flare;
                sweep = 0.56 * flare;
                wing_cup = 0.95 * flare;
                flap = (prog * TAU * 1.5).sin() * 0.55 * flare;
                span_scale = 1.0 + 0.05 * flare - 0.08 * fold;
                finger_splay = 0.96 * (1.0 - 0.55 * fold);
                tail_fan = 0.44 * (1.0 - fold) + 0.55 * flare + 0.28 * fold;
                tail_drop = 0.90 * flare;
                tail_twist = eff_bank * 3.5;
                tail_lift = -0.32 * flare;
                leg_drop =
                    smoothstep(0.05, 0.38, prog) * (1.0 - smoothstep(0.76, 0.96, prog));
                let touch_squash = (((prog - 0.70) / 0.30).clamp(0.0, 1.0) * PI).sin();
                jitter = Vec2::new(0.0, touch_squash * 1.1);
                head_lunge = 0.32 * (1.0 - fold) + 0.12 * fold - 0.22 * flare;
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

        // Death rolls the entire bird over onto its flank in a dramatic collapse.
        // Flying states keep `(f, p)` strictly orthonormal; `eff_bank` drives a
        // true 3D-projected roll (foreshortened inner wing, long outer wing,
        // rolled torso scapulars and twisted rudder tail).
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
            view.pos
                + f * (HEAD_OFFSET + head_lunge * 2.4 + stretch * 2.5)
                + p * (eff_bank * 1.2)
        };
        let slump = if view.state == MagpieState::Death {
            p * (smoothstep(0.25, 0.85, death) * 6.5) - f * (death * 2.8)
        } else {
            Vec2::ZERO
        };
        // **A grounded bird is drawn where its hit shapes are.** On the deck
        // the shapes are the rat's to hit, so the sprite may only lift as far as
        // those deck circles still reach — otherwise the rat was shooting the
        // planks a dozen pixels under a floating bird. Airborne, the shapes are
        // ungrounded and nothing on the deck can reach them, so the sprite is
        // drawn at its true altitude over its own detached ground shadow.
        let lift = if drawn_airborne(view.state) {
            altitude
        } else {
            altitude.min(HIT_LIFT_MAX)
        };
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
            span_scale,
            wrist_fold,
            wing_cup,
            finger_splay,
            bank: eff_bank,
            tail_twist,
            tail_drop,
            leg_drop,
            shade_bias,
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
    let mut gloss = mix((58, 78, 118), (82, 58, 122), e * 0.7);
    let mut white = mix((244, 246, 252), (252, 246, 234), e * 0.5);
    let mut white_sh = mix((156, 168, 188), (168, 152, 150), e * 0.5);
    let mut sheen = mix((22, 112, 168), (92, 46, 158), e * 0.72);
    let mut sheen_hi = mix((74, 198, 234), (162, 108, 242), e * 0.72);
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

/// Per-side projected wing landmarks and bank roll metrics in local `(u, v)`.
#[derive(Clone, Copy)]
struct WingGeom {
    root: (f32, f32),
    wrist: (f32, f32),
    tip: (f32, f32),
    eff_span: f32,
    bank_span: f32,
    bank_chord: f32,
    inner: f32,
    cup: f32,
    wb: f32,
}

#[inline]
fn wing_geom(pose: &Pose, side: f32) -> WingGeom {
    let spread = pose.spread.clamp(0.0, 1.0);
    // `inner > 0`: wing rolled down inside the turn (foreshortened);
    // `inner < 0`: wing raised on the outside of the turn (long and bright).
    let inner = side * pose.bank;
    let bank_span = 1.0 - 0.27 * inner - 0.13 * inner * inner;
    let bank_chord = 1.0 - 0.12 * inner - 0.04 * inner * inner;
    let bank_sweep = -0.14 * inner;

    let eff_span = spread * pose.span_scale * bank_span;
    let eff_sweep = pose.sweep + bank_sweep;
    let wb = pose.wrist_fold * spread;
    let cup = pose.wing_cup * spread;

    let root = (4.6, side * 4.2);
    let wrist = (
        6.2 - 8.4 * (1.0 - spread) + 3.6 * eff_sweep + 2.4 * wb + 2.6 * cup,
        side * (5.2 + 8.8 * eff_span - 1.4 * wb),
    );
    let tip = (
        1.8 - 17.0 * (1.0 - spread) + 8.8 * eff_sweep - 5.6 * wb
            + 3.2 * cup
            + pose.flap * 2.0 * spread,
        side * (4.4 + 18.6 * eff_span - 3.2 * wb),
    );
    WingGeom {
        root,
        wrist,
        tip,
        eff_span,
        bank_span,
        bank_chord,
        inner,
        cup,
        wb,
    }
}

/// Sculpted avian wing SDF: broad rounded magpie paddle wings in flight (with
/// articulated carpal fold, cupped braking camber, and true roll foreshortening),
/// sweeping into a sleek dive blade on `Swoop`/`StrafePass` and folding cleanly
/// along the flanks when perched.
#[inline]
fn wing_sdf(pose: &Pose, side: f32, u: f32, v: f32) -> (f32, f32, f32) {
    let spread = pose.spread.clamp(0.0, 1.0);
    let g = wing_geom(pose, side);
    let sleek = (-pose.sweep).clamp(0.0, 1.0);
    let tip_r = (2.35 - 0.95 * sleek + 0.55 * g.cup) * g.bank_chord;
    let d_inner = sd_taper(
        u,
        v,
        g.root,
        g.wrist,
        5.5 * g.bank_chord,
        (5.2 - 0.4 * spread) * g.bank_chord,
    );
    let d_outer = sd_taper(
        u,
        v,
        g.wrist,
        g.tip,
        (5.1 - 0.4 * spread) * g.bank_chord,
        tip_r,
    );
    let trail_r = (g.root.0 - 5.2, side * 4.2);
    let trail_w = (
        g.wrist.0 - (5.6 - 1.4 * sleek + 1.8 * g.cup) * spread,
        side * (4.8 + 8.4 * g.eff_span - 1.0 * g.wb),
    );
    let trail_m = (
        g.tip.0 - (4.6 - 2.2 * sleek + 1.2 * g.cup) * spread,
        side * (4.4 + 14.8 * g.eff_span - 2.4 * g.wb),
    );
    let d_trail = sd_taper(u, v, trail_r, trail_w, 4.3 * g.bank_chord, 3.6 * g.bank_chord);
    let d_mid = sd_taper(
        u,
        v,
        trail_w,
        trail_m,
        3.6 * g.bank_chord,
        (2.5 - 0.7 * sleek) * g.bank_chord,
    );
    let mut d = smin(smin(d_inner, d_outer, 2.4), smin(d_trail, d_mid, 1.8), 1.9);

    // Phase 2/3 ragged feather battle-notches along the trailing edge!
    if pose.enraged > 0.3 && u < g.wrist.0 - 1.2 {
        let notch = ((v * 1.45).sin()).max(0.0) * (0.85 * pose.enraged + 0.55 * pose.critical);
        d += notch;
    }

    let max_v = (g.tip.1.abs() - 3.0).max(2.8);
    let span = ((v * side - 3.0) / max_v).clamp(0.0, 1.2);
    let spine_u = if span < 0.45 {
        g.root.0 + (g.wrist.0 - g.root.0) * (span / 0.45)
    } else {
        g.wrist.0 + (g.tip.0 - g.wrist.0) * ((span - 0.45) / 0.55).clamp(0.0, 1.0)
    };
    let chord_w = (11.2 - 3.8 * span) * g.bank_chord;
    let chord = ((u - (spine_u - chord_w * 0.68)) / chord_w.max(3.2)).clamp(0.0, 1.0);
    (d, span, chord)
}

/// Graduated 5-feather diamond-wedge corvid tail SDF with rudder bank twist and
/// downward landing brake fan.
#[inline]
fn tail_sdf(pose: &Pose, u: f32, v: f32) -> (f32, f32) {
    let tlen = TAIL_LENGTH * (1.0 - 0.24 * pose.tail_drop);
    let lift_v = pose.tail_lift * 2.2 + pose.tail_twist;
    let fan = 2.6 + 3.8 * pose.tail_fan + 1.6 * pose.tail_drop;
    let b = pose.bank;
    let fan_neg = fan * (1.0 + 0.26 * b);
    let fan_pos = fan * (1.0 - 0.26 * b);
    let d_center = sd_taper(u, v, (-4.5, 0.0), (-(5.5 + tlen), lift_v), 3.4, 1.75);
    let d_l1 = sd_taper(
        u,
        v,
        (-4.5, -1.1),
        (-(3.8 + tlen * 0.87), lift_v - fan_neg * 0.55),
        2.5,
        1.45,
    );
    let d_r1 = sd_taper(
        u,
        v,
        (-4.5, 1.1),
        (-(3.8 + tlen * 0.87), lift_v + fan_pos * 0.55),
        2.5,
        1.45,
    );
    let d_l2 = sd_taper(
        u,
        v,
        (-4.5, -1.5),
        (-(2.8 + tlen * 0.72), lift_v - fan_neg),
        2.2,
        1.30,
    );
    let d_r2 = sd_taper(
        u,
        v,
        (-4.5, 1.5),
        (-(2.8 + tlen * 0.72), lift_v + fan_pos),
        2.2,
        1.30,
    );
    let d = smin(d_center, d_l1.min(d_r1).min(d_l2.min(d_r2)), 1.3);
    let along = ((-u - 4.5) / tlen.max(6.0)).clamp(0.0, 1.0);
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
        MagpieState::Land if pose.progress >= 0.62 => {
            draw_land_dust(pb, &pose);
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

/// Subtle oak-dust settle ring as the claws touch down at the end of `Land`.
fn draw_land_dust(pb: &mut PixelBuffer, pose: &Pose) {
    let t = ((pose.progress - 0.62) / 0.38).clamp(0.0, 1.0);
    let fade = (1.0 - t) * 0.42 * pose.alpha;
    if fade <= 0.01 {
        return;
    }
    let r = 8.0 + 10.0 * t;
    let steps = 36;
    for i in 0..steps {
        let a = i as f32 / steps as f32 * TAU;
        let x = (pose.ground.x + a.cos() * r).round() as i32;
        let y = (pose.ground.y + a.sin() * r * 0.78).round() as i32;
        blend_px(pb, x, y, (206, 186, 152), fade);
    }
}

/// Shadow scale, penumbra softness and opacity as a function of flight altitude.
///
/// Airborne, the shadow sits at the true ground position, soft and wide; as the
/// bird descends onto the planks it shrinks, sharpens and darkens.
#[inline]
fn shadow_params(altitude: f32) -> (f32, f32, f32) {
    let fly = (altitude / FLIGHT_LIFT).clamp(0.0, 1.35);
    let scale = 0.84 + 0.24 * fly;
    let soft_edge = 0.80 + 2.20 * fly;
    let alpha = 0.58 - 0.24 * fly;
    (scale, soft_edge, alpha)
}

/// Detached ground shadow on the oak planks at the bird's true position
/// (shrinks, sharpens and darkens as the bird descends).
fn draw_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    // A cast shadow is floor, not body: stamp it with the floor pen so the
    // lighting's rim and sheen never treat it as a creature (batch 13).
    let prev_pen = pb.set_pen(0);
    draw_shadow_body(pb, pose);
    pb.set_pen(prev_pen);
}

fn draw_shadow_body(pb: &mut PixelBuffer, pose: &Pose) {
    let fly = (pose.altitude / FLIGHT_LIFT).clamp(0.0, 1.35);
    let (scale, soft_edge, base_alpha) = shadow_params(pose.altitude);
    let alpha = base_alpha * pose.alpha;
    let off_k = 1.0 - fly.clamp(0.0, 1.0);
    let centre = pose.ground + Vec2::new(1.4 * off_k, 1.8 * off_k);
    let reach = 36.0 * scale + soft_edge;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, centre, reach) else {
        return;
    };
    let ink: Rgb = (4, 5, 8);
    let inv_s = 1.0 / scale;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - centre.x;
            let dy = y as f32 - centre.y;
            let u = (dx * pose.f.x + dy * pose.f.y) * inv_s;
            let v = (dx * pose.p.x + dy * pose.p.y) * inv_s;
            let d_t = torso_sdf(pose, u, v);
            let (d_tail, _) = tail_sdf(pose, u, v);
            let (d_wl, _, _) = wing_sdf(pose, -1.0, u, v);
            let (d_wr, _, _) = wing_sdf(pose, 1.0, u, v);
            let d_hd = ellipse_sdf(u, v, HEAD_OFFSET, pose.bank * 1.0, 4.8, 4.0);
            let d_bk = sd_taper(
                u,
                v,
                (HEAD_OFFSET + 3.0, pose.bank * 1.0),
                (HEAD_OFFSET + BEAK_LENGTH * 0.9, pose.bank * 1.0),
                1.8,
                0.5,
            );
            let d = d_t.min(d_tail).min(d_wl).min(d_wr).min(d_hd).min(d_bk) * scale;
            if d < soft_edge {
                let soft = ((soft_edge - d) / (soft_edge + 1.1)).clamp(0.0, 1.0);
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
    // 0) Cool slipstream air streaks behind a low fast Swoop pass (harmless).
    if pose.state == MagpieState::Swoop {
        draw_swoop_streaks(pb, &pose);
    }
    // 1) Scaly bird talons clutching the planks (or dropping in Land / tucked in flight).
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

/// Cool silver-white slipstream streaks peeling off the swept wingtips and tail
/// during a harmless `Swoop` pass (strictly cool air ribbons, no damage cues).
fn draw_swoop_streaks(pb: &mut PixelBuffer, pose: &Pose) {
    let dip = (pose.progress * PI).sin();
    let strength = (0.45 + 0.55 * dip) * pose.alpha;
    if strength <= 0.04 {
        return;
    }
    let hi_col: Rgb = (242, 248, 255);
    let tail_col: Rgb = (158, 188, 218);
    for &side in &[-1.0f32, 1.0] {
        let g = wing_geom(pose, side);
        let tip_sc = pose.place(g.tip.0, g.tip.1);
        let length = (16.0 + 12.0 * dip) * (0.78 + 0.22 * g.bank_span);
        let steps = (length * 1.4).ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let wobble = (t * PI * 1.6 + pose.anim * 16.0 + side).sin() * (0.7 * t);
            let q = tip_sc - pose.f * (1.0 + t * length) + pose.p * (side * 0.5 * t + wobble);
            let a = strength * (1.0 - t).powf(1.2) * 0.85;
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, mix(hi_col, tail_col, t), a);
        }
        let mid_sc = pose.place(g.wrist.0 - 2.0, g.wrist.1 * 0.92);
        let m_len = 10.0 + 8.0 * dip;
        let m_steps = (m_len * 1.3).ceil() as i32;
        for i in 0..=m_steps {
            let t = i as f32 / m_steps.max(1) as f32;
            let q = mid_sc - pose.f * (2.5 + t * m_len);
            let a = strength * (1.0 - t).powf(1.35) * 0.52;
            blend_px(
                pb,
                q.x.round() as i32,
                q.y.round() as i32,
                mix(hi_col, tail_col, 0.35 + 0.65 * t),
                a,
            );
        }
    }
    let t_start = pose.place(-(5.5 + TAIL_LENGTH), pose.tail_twist);
    let t_len = 10.0 + 7.0 * dip;
    let t_steps = (t_len * 1.3).ceil() as i32;
    for i in 0..=t_steps {
        let t = i as f32 / t_steps.max(1) as f32;
        let q = t_start - pose.f * (1.0 + t * t_len);
        let a = strength * (1.0 - t).powf(1.4) * 0.45;
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, tail_col, a);
    }
}

/// Dark scaly corvid claws/talons clutching the planks, dropping forward in `Land`,
/// or tucked cleanly beneath the rump in cruising flight.
fn draw_talons(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    if pose.death > 0.65 {
        return;
    }
    let leg_col: Rgb = (52, 50, 58);
    let leg_hi: Rgb = (118, 114, 126);
    let claw_col: Rgb = (212, 206, 192);
    let drop = pose.leg_drop;
    let tucked = pose.altitude > 4.0 && drop < 0.08;
    for &side in &[-1.0f32, 1.0] {
        if drop > 0.05 {
            // Dropping landing gear: scaly tarsus reaching forward under the chest & down toward the planks.
            let hip = pose.place(3.5, side * 3.8);
            let reach_y = 1.6 + drop * 2.4;
            let u_foot = 7.8 + 5.6 * drop;
            let foot = pose.place(u_foot, side * (4.8 + 1.4 * drop)) + Vec2::new(0.0, reach_y);
            draw_segment(pb, hip, foot, 1.05, pal.outline, 0.90 * pose.alpha);
            draw_segment(pb, hip, foot, 0.68, leg_col, 0.98 * pose.alpha);
            let mid = (hip + foot) * 0.5;
            blend_px(
                pb,
                mid.x.round() as i32,
                mid.y.round() as i32,
                leg_hi,
                0.85 * pose.alpha,
            );
            let splay = 0.55 + 0.16 * drop;
            for &ang_off in &[-splay, 0.0, splay] {
                let dir = pose.f.rotate(ang_off);
                let tlen = 3.6 + 1.0 * drop;
                let tip = foot + dir * tlen + Vec2::new(0.0, 0.6 * drop);
                let toe = foot + dir * (tlen * 0.72) + Vec2::new(0.0, 0.4 * drop);
                draw_segment(pb, foot, tip, 0.75, pal.outline, 0.88 * pose.alpha);
                draw_segment(pb, foot, toe, 0.55, leg_col, 0.96 * pose.alpha);
                blend_px(
                    pb,
                    tip.x.round() as i32,
                    tip.y.round() as i32,
                    claw_col,
                    0.96 * pose.alpha,
                );
            }
            let back_tip = foot - pose.f * 2.5 + Vec2::new(0.0, 0.4 * drop);
            draw_segment(pb, foot, back_tip, 0.55, leg_col, 0.88 * pose.alpha);
        } else if tucked {
            let foot = pose.place(-4.8, side * 2.8) + Vec2::new(0.0, 1.2);
            let tip = foot - pose.f * 2.2;
            draw_segment(pb, foot, tip, 0.55, leg_col, 0.55 * pose.alpha);
        } else {
            let step = if pose.state == MagpieState::Perch {
                (pose.anim * 4.2 + if side < 0.0 { PI } else { 0.0 }).sin() * 1.6
            } else {
                0.0
            };
            let foot = pose.place(1.2 + step, side * 5.8) + Vec2::new(0.0, 1.4);
            // Three splayed dark corvid toes with pale horn talons!
            for &ang_off in &[-0.50f32, 0.0, 0.50] {
                let dir = pose.f.rotate(ang_off);
                let tip = foot + dir * 3.4;
                draw_segment(pb, foot, tip, 0.7, pal.outline, 0.80 * pose.alpha);
                draw_segment(pb, foot, foot + dir * 2.4, 0.5, leg_col, 0.95 * pose.alpha);
                blend_px(
                    pb,
                    tip.x.round() as i32,
                    tip.y.round() as i32,
                    claw_col,
                    0.90 * pose.alpha,
                );
            }
        }
    }
}

fn draw_wings(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 38.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    // Paint the dropped inner wing first and the raised outer wing on top when banked.
    let sides = if pose.bank > 0.0 {
        [1.0f32, -1.0]
    } else {
        [-1.0f32, 1.0]
    };
    for &side in &sides {
        let g = wing_geom(pose, side);
        let flap_l = pose.flap * 0.20 + pose.shade_bias;
        let bank_lum = -0.26 * g.inner;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = pose.local(x, y);
                if v * side < 1.6 {
                    continue;
                }
                let (d, span, chord) = wing_sdf(pose, side, u, v);
                let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    if d < 1.15 {
                        blend_px(pb, x, y, pal.outline, 0.84 * pose.alpha);
                    }
                    continue;
                }
                // Individual primary/secondary feather-barb striping across the wing span!
                let barb = ((span * 15.0 + chord * 2.6).sin()) * 0.14;
                // Cupped braking wing camber shading (bright arch leading edge, shaded hollow).
                let cup_shade = g.cup
                    * (0.34 * smoothstep(0.55, 0.90, chord)
                        - 0.38
                            * smoothstep(0.12, 0.45, chord)
                            * (1.0 - smoothstep(0.50, 0.78, chord)));
                let lum =
                    pose.lit(0.30, side * 0.42) * 0.62 + flap_l + bank_lum + barb + cup_shade;
                let mut c = shade3(pal.shadow, pal.back, pal.gloss, lum);

                // Iconic Magpie iridescent cobalt-blue / peacock-teal secondary speculum!
                let speculum = smoothstep(0.06, 0.20, span)
                    * (1.0 - smoothstep(0.46, 0.62, span))
                    * smoothstep(0.14, 0.36, chord)
                    * (1.0 - smoothstep(0.76, 0.92, chord));
                if speculum > 0.04 {
                    let sh_lum = lum * 0.55 + if g.inner < 0.0 { 0.20 } else { -0.10 };
                    let sh_col = shade3(pal.sheen, pal.sheen_hi, pal.white, sh_lum);
                    c = mix(c, sh_col, (speculum * 0.86).clamp(0.0, 0.94));
                }

                // Iconic Magpie bold snow-white primary flash framed by jet-black tips & leading edge!
                let white_flash = if pose.spread > 0.20 {
                    let s_band =
                        smoothstep(0.34, 0.46, span) * (1.0 - smoothstep(0.76, 0.89, span));
                    let c_band =
                        smoothstep(0.14, 0.28, chord) * (1.0 - smoothstep(0.72, 0.85, chord));
                    s_band * c_band * (0.76 + 0.24 * pose.spread)
                } else {
                    // Folded wing shows crisp white scapular wing-bar edge!
                    smoothstep(0.15, 0.35, span) * (1.0 - smoothstep(0.55, 0.75, span)) * 0.75
                };
                if white_flash > 0.02 {
                    let vane = if (span * 19.0).sin().abs() > 0.16 {
                        0.0
                    } else {
                        0.22
                    };
                    let wcol = shade3(pal.white_sh, pal.white, (255, 255, 255), lum * 0.75 - vane);
                    c = mix(c, wcol, white_flash.clamp(0.0, 0.96));
                }

                // Sleek leading-edge specular gloss highlight!
                if d > -1.45 && chord > 0.56 {
                    c = mix(
                        c,
                        pal.gloss,
                        0.45 + 0.22 * (-g.inner).max(0.0) + 0.18 * g.cup,
                    );
                }
                // Phase-2/3 crimson battle nick near the shoulder!
                if pose.enraged > 0.4
                    && side > 0.0
                    && (u - 3.0).abs() < 1.2
                    && (v - 7.0).abs() < 1.4
                {
                    c = mix(c, (214, 48, 52), 0.78);
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

        // Five crisp jet-black splayed primary "finger" flight feathers at the spread wingtip!
        if pose.spread > 0.42 {
            let tip_sc = pose.place(g.tip.0, g.tip.1);
            let splay = pose.finger_splay;
            let flen_scale = (0.62 + 0.38 * g.bank_span) * (0.70 + 0.30 * splay);
            for k in 0..5 {
                let t = k as f32 / 4.0;
                let base = tip_sc
                    + pose.f * ((0.5 - t) * 4.6 * (0.65 + 0.35 * splay))
                    - pose.p * (side * t * 2.2 * g.bank_span);
                let out_w = (0.45 + 0.55 * splay) * (0.75 + 0.25 * (1.0 - t));
                let aft_w = (0.28 - t * 0.85) * splay - (0.55 + 0.35 * t) * (1.0 - splay);
                let fan_dir = (pose.p * (side * out_w) + pose.f * aft_w).normalized();
                let flen = (3.4 + (1.0 - (t - 0.35).abs()) * 2.4) * flen_scale;
                let end = base + fan_dir * flen;
                let mid = base + fan_dir * (flen * 0.36);
                draw_segment(pb, base, end, 0.95, pal.outline, 0.88 * pose.alpha);
                draw_segment(
                    pb,
                    base,
                    mid,
                    0.55,
                    apply_hit(pal.gloss, pose.hit),
                    0.92 * pose.alpha,
                );
                draw_segment(
                    pb,
                    mid,
                    end,
                    0.55,
                    apply_hit(pal.back, pose.hit),
                    0.96 * pose.alpha,
                );
            }
        }
    }
}

/// Long graduated peacock-iridescent corvid tail with rudder twist & crisp dark quill seams!
fn draw_tail(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 34.0;
    let center = pose.body - pose.f * 10.0 + pose.p * (pose.tail_twist * 0.5);
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, center, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            if u > -2.2 {
                continue;
            }
            let (d, along) = tail_sdf(pose, u, v);
            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.15 {
                    blend_px(pb, x, y, pal.outline, 0.84 * pose.alpha);
                }
                continue;
            }
            let v_rel = v - pose.tail_twist * along;
            let twist_lum = -0.22 * pose.bank * (v_rel / 5.0).clamp(-1.0, 1.0);
            let drop_shade = -0.28 * pose.tail_drop * along;
            let lum = pose.lit(-0.35, (v / 6.0).clamp(-1.0, 1.0)) * 0.62 + twist_lum + drop_shade;
            // Distinct individual rectrix feather quills running down the graduated tail!
            let quill = ((v_rel * 1.45).sin()).abs();
            let iridescence = (0.42 + 0.52 * quill) * (1.0 - along * 0.30);
            let mut c = shade3(pal.shadow, pal.back, pal.sheen, lum);
            c = mix(
                c,
                shade3(pal.sheen, pal.sheen_hi, pal.white, lum * 0.58),
                (iridescence * 0.78).clamp(0.0, 0.92),
            );
            // Crisp dark feather shaft seams between tail feathers.
            if quill < 0.16 {
                c = mix(c, pal.shadow, 0.58);
            }
            // Subtle glossy dark band near the diamond tail tip.
            if along > 0.82 {
                c = mix(c, pal.shadow, ((along - 0.82) * 3.6).clamp(0.0, 0.88));
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

/// Sculpted glossy blue-black corvid torso with iconic blazing snow-white scapular crescents!
fn draw_torso(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage) {
    let reach = 20.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    let v_roll = pose.bank * 1.15;
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
            let nv = ((v - v_roll) / 6.6).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.45 + nv * nv * 0.65)).clamp(0.0, 1.0);
            let lit = pose.lit(nu, nv) - 0.18 * pose.bank * nv;
            let mut c = shade3(pal.shadow, pal.back, pal.gloss, lit * 0.85 + dome * 0.30);

            // Subtle iridescent blue-teal shimmer over the upper mantle!
            let mantle = smoothstep(-6.0, 4.0, u) * (1.0 - smoothstep(4.0, 8.5, u));
            c = mix(c, pal.sheen, mantle * 0.36 * (0.6 + 0.4 * pose.enraged));

            // THE ICONIC MAGPIE SIGNATURE: twin blazing snow-white scapular shoulder crescents
            // rolling across the flanks when the bird banks!
            let v_shifted = v - v_roll;
            let scap_d = ellipse_sdf(u, v_shifted.abs(), -0.5, 4.4, 5.4, 2.25);
            if scap_d < 0.45 {
                let w_cov = (0.45 - scap_d).clamp(0.0, 1.0);
                let w_col = shade3(
                    pal.white_sh,
                    pal.white,
                    (255, 255, 255),
                    lit * 0.75 + dome * 0.25,
                );
                c = mix(c, w_col, w_cov * 0.96);
            }
            // Crisp white lower rump band just above the iridescent tail!
            if u < -5.8 && u > -8.6 && v_shifted.abs() < 3.4 {
                let w_col = shade3(pal.white_sh, pal.white, (255, 255, 255), lit * 0.6);
                c = mix(c, w_col, 0.84);
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

    /// **A grounded bird is drawn where it can be hit.** On the deck its hit
    /// shapes are the grounded circles in `Boss::hit_shapes`, so a grounded
    /// state must not lift the sprite out of them: the rat was shooting planks a
    /// dozen pixels under a floating bird.
    #[test]
    fn every_grounded_magpie_state_draws_its_body_over_its_hit_shapes() {
        let states = [
            MagpieState::Perch,
            MagpieState::Ruffle,
            MagpieState::PeckWindup,
            MagpieState::Peck,
            MagpieState::Stuck,
            MagpieState::GustWindup,
            MagpieState::Gust,
            MagpieState::Death,
        ];
        for state in states {
            assert!(!state.flying(), "{state:?} is in the grounded list");
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

    /// **A flying bird is drawn in the air, over its own shadow.** The airborne
    /// states are ungrounded (`Boss::hit_shapes`), so nothing on the deck can
    /// reach them and the sprite is drawn at its true altitude — the shadow
    /// stays on the planking where the rat can read it.
    #[test]
    fn a_flying_bird_is_drawn_above_its_shadow() {
        for state in [
            MagpieState::Soar,
            MagpieState::Swoop,
            MagpieState::Takeoff,
            MagpieState::StrafeWindup,
            MagpieState::StrafePass,
            MagpieState::SnatchDrop,
        ] {
            assert!(state.flying(), "{state:?} must be airborne");
            assert!(!has_body_contact(state), "{state:?} must not bump the rat");
            let mut fullest = 0.0f32;
            for prog in [0.2f32, 0.5, 0.8, 1.0] {
                let mut view = MagpieView::new(
                    Vec2::new(46.0, 58.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(104.0, 66.0),
                );
                view.state = state;
                view.progress = prog;
                view.anim = 0.7 + prog * 3.0;
                view.bank = 1.0;
                let pose = Pose::from_view(&view, Vec2::ZERO, (206, 150, 70));
                let lift = pose.ground.y - pose.body.y;
                assert!(
                    lift <= FLIGHT_LIFT + 4.5,
                    "{state:?} at {prog}: drawn {lift:.1}px off the deck, past the flight lift"
                );
                assert!(lift >= 0.0, "{state:?} at {prog}: drawn under the deck");
                fullest = fullest.max(lift);
                // The shadow never leaves the bird's own footprint.
                assert!(pose.ground.dist(view.pos) < 1e-3);
            }
            // **The descent is the way back down**: it starts at the flight's
            // own height with its shapes already grounded (the rat can hit a
            // bird that is landing) and settles onto the planks.
            let lift_at = |prog: f32| {
                let mut view =
                    MagpieView::new(Vec2::new(46.0, 58.0), Vec2::new(1.0, 0.0), Vec2::new(104.0, 66.0));
                view.state = MagpieState::Land;
                view.progress = prog;
                let pose = Pose::from_view(&view, Vec2::ZERO, (206, 150, 70));
                pose.ground.y - pose.body.y
            };
            assert!(!MagpieState::Land.flying(), "the descent is hittable");
            assert!(
                lift_at(0.0) >= FLIGHT_LIFT * 0.99 && lift_at(1.0) < 0.5,
                "the descent must run from the flight lift ({:.1}) down to the deck ({:.1})",
                lift_at(0.0),
                lift_at(1.0)
            );
            // Every airborne state climbs clear of the deck (and of the lift a
            // grounded sprite is held to) at some point in its beat. `Takeoff`
            // is the exception on entry by design: it starts on the planking.
            if state != MagpieState::Takeoff {
                assert!(
                    fullest > HIT_LIFT_MAX + 0.5,
                    "{state:?} is never drawn clear of the deck ({fullest:.1}px)"
                );
            } else {
                let mut view = MagpieView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(104.0, 0.0));
                view.state = MagpieState::Takeoff;
                view.progress = 1.0;
                let pose = Pose::from_view(&view, Vec2::ZERO, (206, 150, 70));
                assert!(
                    pose.ground.y - pose.body.y >= FLIGHT_LIFT * 0.99,
                    "a finished takeoff must be at the flight lift"
                );
            }
        }
        // A bank rolls the bird cleanly without shearing `(f, p)`: the inner
        // wing foreshortens, the outer wing extends long, the tail twists into
        // the turn, and the body centre stays over the footprint.
        let mut flat = MagpieView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(60.0, 0.0));
        flat.state = MagpieState::Soar;
        let mut banked = flat;
        banked.bank = 1.0;
        let a = Pose::from_view(&flat, Vec2::ZERO, (206, 150, 70));
        let b = Pose::from_view(&banked, Vec2::ZERO, (206, 150, 70));
        assert!(b.f.dot(b.p).abs() < 1e-4, "the bank roll must keep (f, p) orthonormal");
        let inner = wing_geom(&b, 1.0);
        let outer = wing_geom(&b, -1.0);
        assert!(
            outer.tip.1.abs() > inner.tip.1.abs() * 1.45,
            "outer wing ({:.1}) must read noticeably longer than foreshortened inner wing ({:.1})",
            outer.tip.1.abs(),
            inner.tip.1.abs()
        );
        assert!(
            b.tail_twist > 2.0 && b.tail_fan > a.tail_fan,
            "the banked tail must fan wider and twist into the turn"
        );
    }

    #[test]
    fn every_state_and_bank_draws_without_panic_and_within_bounds() {
        let states = [
            MagpieState::Soar,
            MagpieState::Swoop,
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
        let banks = [-1.5f32, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, f32::NAN, f32::INFINITY];
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.707, -0.707),
            Vec2::ZERO,
        ];

        // Degenerate 0x0 buffer must not panic for any state or bank.
        let mut empty = PixelBuffer::new(0, 0);
        for &state in &states {
            for &bank in &banks {
                let mut v = MagpieView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(20.0, 0.0));
                v.state = state;
                v.bank = bank;
                draw_magpie_ground(&mut empty, &v, Vec2::ZERO, (206, 150, 70));
                draw_magpie(&mut empty, &v, Vec2::ZERO, (206, 150, 70));
                draw_magpie_crate(&mut empty, &v, Vec2::ZERO);
            }
        }

        // Every state and bank value draws safely without panicking.
        let mut scratch = PixelBuffer::new(160, 140);
        for &state in &states {
            for &bank in &banks {
                for &facing in &facings {
                    for &prog in &[0.0f32, 0.35, 0.72, 1.0] {
                        let mut v = MagpieView::new(
                            Vec2::new(80.0, 72.0),
                            facing,
                            Vec2::new(105.0, 76.0),
                        );
                        v.state = state;
                        v.bank = bank;
                        v.progress = prog;
                        v.anim = prog * SOAR_CYCLE_SECS;
                        v.lane_start = Vec2::new(50.0, 72.0);
                        draw_magpie_ground(&mut scratch, &v, Vec2::ZERO, (206, 150, 70));
                        draw_magpie(&mut scratch, &v, Vec2::ZERO, (206, 150, 70));
                        draw_magpie_crate(&mut scratch, &v, Vec2::ZERO);
                    }
                }
            }
        }

        // Flight states stay strictly inside `CULL_MARGIN` of `view.pos` (no pixels escape).
        let bg: Rgb = (255, 0, 255);
        let center = Vec2::new(90.0, 90.0);
        let bound = (CULL_MARGIN + 2.0) as i32;
        for &state in &[
            MagpieState::Soar,
            MagpieState::Swoop,
            MagpieState::Land,
            MagpieState::Takeoff,
        ] {
            for &bank in &[-1.0f32, 0.0, 1.0] {
                for &prog in &[0.1f32, 0.5, 0.9] {
                    let mut pb = PixelBuffer::new(180, 180);
                    pb.clear(bg);
                    let mut v = MagpieView::new(center, Vec2::new(1.0, 0.0), center + Vec2::new(40.0, 0.0));
                    v.state = state;
                    v.bank = bank;
                    v.progress = prog;
                    v.anim = prog * SOAR_CYCLE_SECS;
                    draw_magpie_ground(&mut pb, &v, Vec2::ZERO, (206, 150, 70));
                    draw_magpie(&mut pb, &v, Vec2::ZERO, (206, 150, 70));
                    let mut drawn = 0usize;
                    for y in 0..pb.h as i32 {
                        for x in 0..pb.w as i32 {
                            if pb.pixel_at(x as usize, y as usize) != bg {
                                drawn += 1;
                                let dx = (x - center.x as i32).abs();
                                let dy = (y - center.y as i32).abs();
                                assert!(
                                    dx <= bound && dy <= bound,
                                    "{state:?} (bank={bank}, prog={prog}) drew out of bounds at ({x},{y}), delta=({dx},{dy}) > {bound}"
                                );
                            }
                        }
                    }
                    assert!(drawn > 80, "{state:?} (bank={bank}) must draw a visible bird ({drawn} px)");
                }
            }
        }
    }

    #[test]
    fn soar_flap_cycle_is_continuous_across_phases_and_wrap() {
        let steps = 600usize;
        let dt = (2.0 * SOAR_CYCLE_SECS) / steps as f32;
        let mut prev = soar_flap_kinematics(0.0);
        let mut min_span = prev.span_scale;
        let mut max_span = prev.span_scale;
        let mut max_fold = prev.wrist_fold;
        let mut min_glide = prev.glide;
        let mut max_glide = prev.glide;

        for i in 1..=steps {
            let t = i as f32 * dt;
            let cur = soar_flap_kinematics(t);
            assert!(cur.stroke.is_finite() && cur.span_scale.is_finite() && cur.body_bob.is_finite());
            // Adjacent 5 ms samples must move smoothly with no jump or pop.
            assert!(
                (cur.stroke - prev.stroke).abs() < 0.16,
                "stroke popped at t={t:.4}: {} -> {}",
                prev.stroke,
                cur.stroke
            );
            assert!(
                (cur.span_scale - prev.span_scale).abs() < 0.06,
                "span_scale popped at t={t:.4}: {} -> {}",
                prev.span_scale,
                cur.span_scale
            );
            assert!(
                (cur.sweep_delta - prev.sweep_delta).abs() < 0.09,
                "sweep_delta popped at t={t:.4}: {} -> {}",
                prev.sweep_delta,
                cur.sweep_delta
            );
            assert!(
                (cur.wrist_fold - prev.wrist_fold).abs() < 0.08,
                "wrist_fold popped at t={t:.4}: {} -> {}",
                prev.wrist_fold,
                cur.wrist_fold
            );
            assert!(
                (cur.body_bob - prev.body_bob).abs() < 0.22,
                "body_bob popped at t={t:.4}: {} -> {}",
                prev.body_bob,
                cur.body_bob
            );
            min_span = min_span.min(cur.span_scale);
            max_span = max_span.max(cur.span_scale);
            max_fold = max_fold.max(cur.wrist_fold);
            min_glide = min_glide.min(cur.glide);
            max_glide = max_glide.max(cur.glide);
            prev = cur;
        }

        // The cycle must actually visit both articulated flaps (tuck + full downstroke) and glides.
        assert!(max_span > 1.06, "downstroke must open past level span ({max_span:.2})");
        assert!(min_span < 0.70, "upstroke tuck must fold inward ({min_span:.2})");
        assert!(max_fold > 0.75, "upstroke must articulate the carpal wrist ({max_fold:.2})");
        assert!(min_glide < 0.02 && max_glide > 0.99, "cycle must alternate flapping bursts and glides");

        // Exact periodic wrap across SOAR_CYCLE_SECS and safety for negative anim.
        let at0 = soar_flap_kinematics(0.0);
        let at_end = soar_flap_kinematics(SOAR_CYCLE_SECS - 1e-4);
        assert!((at0.span_scale - at_end.span_scale).abs() < 1e-3);
        assert!((at0.stroke - at_end.stroke).abs() < 1e-3);
        assert!((at0.glide - at_end.glide).abs() < 1e-3);
        let neg = soar_flap_kinematics(-0.35);
        assert!(neg.span_scale.is_finite() && neg.span_scale > 0.5);
    }

    #[test]
    fn banking_rolls_asymmetrically_and_land_blends_with_sharpening_shadow() {
        // Bank -1, 0, +1 produce distinct silhouettes and mirrored roll geometry.
        let render_soar = |bank: f32| {
            let mut pb = PixelBuffer::new(110, 110);
            pb.clear((60, 48, 34));
            let mut v = MagpieView::new(Vec2::new(55.0, 62.0), Vec2::new(1.0, 0.0), Vec2::new(90.0, 62.0));
            v.state = MagpieState::Soar;
            v.anim = 0.84 * SOAR_CYCLE_SECS;
            v.bank = bank;
            draw_magpie(&mut pb, &v, Vec2::ZERO, (206, 150, 70));
            pb
        };
        let left = render_soar(-1.0);
        let level = render_soar(0.0);
        let right = render_soar(1.0);
        assert_ne!(left.pixels(), level.pixels(), "bank=-1 must differ from level flight");
        assert_ne!(right.pixels(), level.pixels(), "bank=+1 must differ from level flight");
        assert_ne!(left.pixels(), right.pixels(), "bank=-1 and bank=+1 must roll opposite ways");

        // Land flares with cupped wings, dropped tail and legs mid-descent, then
        // settles cleanly into the range between Perch and PeckWindup.
        let land_pose = |prog: f32| {
            let mut v = MagpieView::new(Vec2::new(55.0, 62.0), Vec2::new(1.0, 0.0), Vec2::new(90.0, 62.0));
            v.state = MagpieState::Land;
            v.progress = prog;
            v.bank = 0.8;
            Pose::from_view(&v, Vec2::ZERO, (206, 150, 70))
        };
        let mid = land_pose(0.42);
        let end = land_pose(1.0);
        assert!(mid.wing_cup > 0.70, "Land must cup wings mid-descent ({:.2})", mid.wing_cup);
        assert!(mid.tail_drop > 0.70, "Land must fan tail down mid-descent ({:.2})", mid.tail_drop);
        assert!(mid.leg_drop > 0.70, "Land must drop legs mid-descent ({:.2})", mid.leg_drop);
        assert!(end.altitude < 0.01 && end.wing_cup < 0.01 && end.leg_drop < 0.01);
        assert!(end.bank.abs() < 0.01, "Land must level out bank before touching down");
        assert!(
            (0.0..=0.36).contains(&end.spread),
            "Land end spread ({:.2}) must bridge Perch (0.0) and PeckWindup (0.36)",
            end.spread
        );

        // Shadow shrinks, sharpens and darkens as altitude descends from FLIGHT_LIFT to 0.
        let (scale_hi, soft_hi, alpha_hi) = shadow_params(FLIGHT_LIFT);
        let (scale_mid, soft_mid, alpha_mid) = shadow_params(FLIGHT_LIFT * 0.5);
        let (scale_lo, soft_lo, alpha_lo) = shadow_params(0.0);
        assert!(scale_hi > scale_mid && scale_mid > scale_lo, "shadow must shrink on descent");
        assert!(soft_hi > soft_mid && soft_mid > soft_lo, "shadow must sharpen on descent");
        assert!(alpha_hi < alpha_mid && alpha_mid < alpha_lo, "shadow must darken on descent");
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
            MagpieState::Soar,
            MagpieState::Swoop,
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
            w.level_index = crate::game::campaign::LEVEL_COUNT - 1;
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
            MagpieState::Soar,
            MagpieState::Swoop,
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
            MagpieState::Soar,
            MagpieState::Swoop,
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
                w.level_index = crate::game::campaign::LEVEL_COUNT - 1;
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
