//! Art and gameplay specification for the Flatbed's second boss — **THE
//! REDTAIL**.
//!
//! The magpie owns the wind over the cab: it steals, it pecks, it beats its
//! wings against the trailer's own slipstream. This is the other bird. It is
//! not interested in the cargo. It has been following the truck since the last
//! weigh station, and from a rat's height the first thing you ever see of it is
//! **a shadow crossing the planks**.
//!
//! # The fight the owner asked for
//!
//! > "hawk (comes in as shadow at first then dive bombs player, sometimes
//! > hovers) ca caww!"
//!
//! Three beats, and every state below belongs to one of them:
//!
//! | beat | states | what the rat reads |
//! |---|---|---|
//! | **the shadow** | [`HawkState::Circle`] | a small, faint bird-shape sliding over the oak, lagging a half-step behind the rat |
//! | **the dive** | [`HawkState::Dive`] → [`HawkState::TalonBury`] / [`HawkState::Wheel`] | the shadow stops, a **talon reticle** snaps onto the deck, a keening scream, then one frame of impact and a shockwave ring. A miss buries the talons: the nape is the punish. |
//! | **the hover** | [`HawkState::Stoop`] → [`HawkState::Hover`] → [`HawkState::DartWindup`] / [`HawkState::GrabWindup`] / [`HawkState::RakeWindup`] | wings beating two body-lengths over the planks, downdraft rings shoving everything loose outward, and three ways to reach down at a rat that is used to looking up |
//!
//! ## 1. The shadow (overhead)
//!
//! While the bird is high it **cannot be hit at all**: [`is_overhead`] is true,
//! `Boss::hit_shapes` is empty for those states and body contact is off. The
//! plate says so (`render::hud::boss_plate` returns [`OVERHEAD_TAG`]), and the
//! shadow is the boss on the screen: it is drawn **always** and its position is
//! the entity's position, so where the fight is happening is never a guess. The
//! shadow shrinks and pales as the bird climbs and grows, darkens and sharpens
//! as it comes down — that gradient *is* the altitude read-out.
//!
//! It circles: [`CIRCLE_DURATION`] of a tangent-plus-inward blend that holds a
//! [`CIRCLE_BAND`] px ring around the rat at [`CIRCLE_SPEED`] (deliberately
//! slower than the rat's 58 px/s walk, so the shadow always trails him a
//! little). A rat that runs a straight line outruns the shadow and can see it
//! coming across the planks.
//!
//! ## 2. The dive
//!
//! [`DIVE_LOCK_PROGRESS`] is the tell: the shadow **stops** and a **talon
//! reticle** — the three-print landing mark the flatbed's Road Atlas appointment
//! teaches (`WAVE4_ROSTERS` §Flatbed) — snaps onto the deck where the rat is
//! standing and freezes. From there the fall takes the rest of
//! [`DIVE_DURATION`]; the impact is a single [`DIVE_DAMAGE`] hit inside
//! [`DIVE_RADIUS`] plus a damage-free shockwave ring ([`DIVE_RING_OUTER`]) that
//! shoves the deck around it. The ring deals nothing, so it costs no i-frames
//! and a rat standing on its inner edge can still be caught by the next strike.
//!
//! **A miss buries the talons.** If the rat is not inside the impact circle the
//! bird ends [`HawkState::TalonBury`]: nose down, tail up, wings across the
//! planks, no body contact and ×[`TALON_BURY_DAMAGE_MULT`] incoming, with the
//! **back of the neck** ([`nape_pos`], radius [`NAPE_RADIUS`]) replacing the
//! first body circle — so it is also the first auto-aim point while the window
//! is open. A *hit* dive instead wheels straight back up ([`HawkState::Wheel`]).
//!
//! The feint (phase 2+): the shadow **jinks once** before it locks. The reticle
//! flashes at a decoy point [`FEINT_OFFSET`] px off the rat's shoulder
//! (`DIVE_FEINT_PROGRESS`..`DIVE_LOCK_PROGRESS`) and only then snaps to the
//! real mark. Everything about the real lock — freeze, ring, fall — is
//! unchanged; the feint only punishes a rat that dodges the first print.
//!
//! ## 3. The hover
//!
//! Sometimes it stoops instead ([`HawkState::Stoop`]) and stays down. While low
//! it is **fully hittable** (all three circles grounded, [`CONTACT_DAMAGE`] on
//! the body) and it is watching: [`HawkState::Hover`] holds a
//! [`HOVER_BAND`] ring and beats its wings, and each beat is a **downdraft
//! ring** ([`DOWNDRAFT_INNER`]..[`DOWNDRAFT_OUTER`], push-only) that shoves the
//! rat — and, through `ecs::systems::trailer`, the loose gems and light bodies
//! on the deck — away from under it. From the hover it picks one of:
//!
//! * **Feather darts** ([`HawkState::DartWindup`] → [`HawkState::DartFan`]): a
//!   [`DART_HALF_ANGLE`] wedge is painted out of the bill for
//!   [`DART_WINDUP`] s and then fills with darts; one hit inside the wedge,
//!   [`DART_DAMAGE`].
//! * **The grab** ([`HawkState::GrabWindup`] → [`HawkState::Grab`]): a
//!   [`GRAB_HALF_WIDTH`] px talon reach is marked out to the rat. A rat still on
//!   the mark when it closes is **carried** ([`GRAB_CARRY_DRAG`] px/s toward the
//!   bird) and **dropped** for [`GRAB_DAMAGE`]; a rat that steps or dashes off
//!   the mark spends nothing. The bird holds still for the whole carry, so the
//!   drop point is the one the tell showed.
//! * **The talon rake** ([`HawkState::RakeWindup`] → [`HawkState::RakePass`]):
//!   a lane down the bed at the rat's row, flown at [`RAKE_SPEED`], one
//!   [`RAKE_DAMAGE`] hit anywhere in it.
//!
//! A wounded bird hovers **lower and longer** ([`HOVER_DURATION`] grows with the
//! phase, [`hover_altitude`] shrinks) and gets more hover attacks before it
//! climbs ([`HOVER_LIMIT`]).
//!
//! ## Suppression (every attack, every time)
//!
//! No move is committed when the rat could not read it or leave it, and every
//! refusal falls through to a cheaper move and finally back to `Circle`:
//!
//! * the **dive** needs the mark on open floor, line of sight from the shadow,
//!   a clear step out of [`DIVE_RADIUS`], and — the flatbed's own rule
//!   ([`DIVE_PIT_CLEARANCE`]) — no hole within knockback reach of the mark: the
//!   bird will not slam the rat toward a broken plank;
//! * the **darts** need the rat inside [`DART_REACH`] with a clear line from the
//!   bill and a clear step out of the wedge;
//! * the **grab** needs the rat inside [`GRAB_REACH`] with a clear step off the
//!   reach (a grabbed rat must always be able to have left);
//! * the **rake** needs a lane at least [`RAKE_MIN_LEN`] long that the bird's
//!   whole body fits along, with [`RAKE_SAFE_SIDE`] px of clear deck on one side
//!   of the rat.
//!
//! ## Determinism
//!
//! Nothing here draws rng. The controller takes one boss-rng draw per `Circle`
//! entry (which way it circles) and nothing else; every other choice is a fixed
//! rotation in `Boss::scratch`. The art is stack-only and culls on
//! [`CULL_MARGIN`], which has to cover the overhead sprite's lift *and* the
//! reticle's reach, since the ground layer is culled on the body **and** on the
//! mark.
//!
//! # Layering contract for the renderer
//!
//! 1. **Ground layer** (with the other bosses' floor marks, before the bodies):
//!    [`draw_hawk_ground`] — the shadow, the talon reticle and its decoy, the
//!    dart wedge, the grab reach, the rake lane, the downdraft rings, the
//!    impact burst and the HP arc.
//! 2. **Entity layer**: [`draw_hawk`] — the bird itself, lifted by its altitude.
//!    Overhead it is drawn faint and high (a silhouette a rat would glance at);
//!    it is never drawn over a hit shape it does not have.
//! 3. **Post-lighting**: [`draw_tell_overlay`] re-stamps the live reticle, the
//!    nape glint and the rake lane, so the flatbed's ambient can never dim the
//!    shape the rat has to leave.

#![allow(dead_code)]

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Collision & gameplay constants (art <-> controller <-> framework contract)
// ============================================================================

/// Framework movement/contact circle for the low bird (world px): the width of
/// the folded body.
pub const BODY_RADIUS: f32 = 10.0;

/// Bill tip to tail tip, folded (the culling/length figure).
pub const BODY_LENGTH: f32 = 38.0;

/// Longest sprite reach: the high bird's lift plus the spread wings, and the
/// reticle's ring on the far side of the mark.
pub const CULL_MARGIN: f32 = 92.0;

/// Head centre, along the facing axis from the body centre.
pub const HEAD_OFFSET: f32 = 12.5;

/// Radius of the head circle.
pub const HEAD_RADIUS: f32 = 5.0;

/// Bill length from the head centre to the hooked tip.
pub const BEAK_LENGTH: f32 = 9.0;

/// Tail length behind the body centre (a redtail's tail is long).
pub const TAIL_LENGTH: f32 = 19.0;

/// Half-span of one wing when spread.
pub const WING_SPAN: f32 = 27.0;

/// The exposed nape (the weak point): how far behind the head it sits, and its
/// radius.
pub const NAPE_BACK: f32 = 4.5;
pub const NAPE_RADIUS: f32 = 6.2;

/// The art's recommended base pool, before `BossKind::hp_scale` and the
/// director's boss curve. Sized like the Magpie's: the flatbed is a practice
/// floor reached with a full 15-minute build, so the *scale* carries the fight
/// length (see `BossKind::hp_scale`).
pub const BOSS_BASE_HP: f32 = 1600.0;

/// Body contact while it is down on the deck (owner's rule: 8 outside a
// recovery window).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 opens here (the feint, faster shadow).
pub const PHASE2_HP_FRAC: f32 = 0.62;

/// Phase 3 opens here (the third attack in the hover rotation).
pub const PHASE3_HP_FRAC: f32 = 0.30;

/// Critical shimmer on the nape (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

/// The plate's second line while the bird is out of reach: the fight tells the
/// rat *why* its weapons are finding nothing.
pub const OVERHEAD_TAG: &str = "OVERHEAD — OUT OF REACH";

// ---- the shadow (overhead neutral) ---------------------------------------

/// `Circle` length per phase: how long the shadow reads the floor before it
/// commits.
pub const CIRCLE_DURATION: [f32; 3] = [1.05, 0.95, 0.85];

/// Shadow speed per phase (`52 / 60 / 68` px/s). Phase 1 is deliberately under
/// the rat's 58 px/s walk, so a straight line always outruns the shadow.
pub const CIRCLE_SPEED: [f32; 3] = [52.0, 60.0, 68.0];

/// The ring `Circle` holds the rat at, in px (`near`, `far`).
pub const CIRCLE_BAND: [f32; 2] = [34.0, 92.0];

/// Extra speed multiplier while hunting a rat that has left the hold.
pub const HUNT_SPEED_MULT: f32 = 1.8;

/// How far the shadow's own drift leads the rat's motion while circling (px of
/// tangential bias): a bird reads a running rat, it does not follow its tail.
pub const CIRCLE_LEAD: f32 = 10.0;

// ---- the dive ------------------------------------------------------------

/// The whole dive: seek, lock, fall, impact.
pub const DIVE_DURATION: [f32; 3] = [1.30, 1.18, 1.06];

/// The shadow stops and the reticle locks here (the tell).
pub const DIVE_LOCK_PROGRESS: f32 = 0.55;

/// The reticle is at the decoy from here until the lock (phase 2+ feint).
pub const DIVE_FEINT_PROGRESS: f32 = 0.30;

/// The impact lands here; after it the state hands over to `TalonBury`/`Wheel`.
pub const DIVE_IMPACT_PROGRESS: f32 = 0.90;

/// The feint's decoy point, px off the rat's shoulder.
pub const FEINT_OFFSET: f32 = 30.0;

/// The impact circle's radius (the talon spread).
pub const DIVE_RADIUS: f32 = 13.0;

/// The impact's damage, once.
pub const DIVE_DAMAGE: f32 = 22.0;

/// The impact's live window (a single frame of impact).
pub const DIVE_ACTIVE: f32 = 0.12;

/// Impact knockback (px/s, applied once, radial).
pub const DIVE_KNOCKBACK: f32 = 88.0;

/// The damage-free shockwave ring outside the impact circle.
pub const DIVE_RING_OUTER: f32 = 34.0;

/// How long the shockwave ring shoves for.
pub const DIVE_RING_ACTIVE: f32 = 0.16;

/// The ring's outward push (px/s). Small on purpose: it is a read, not a shove,
/// and the flatbed has holes in it.
pub const DIVE_RING_PUSH: f32 = 46.0;

/// The bird will not dive at a mark with a hole this close to it (px): a
/// knocked-back rat must never be dropped into a broken plank it never saw.
pub const DIVE_PIT_CLEARANCE: f32 = 26.0;

/// The step the rat needs out of the reticle (px): the mark is frozen with
/// [`DIVE_LOCK_PROGRESS`] of the dive left, so a walk clears it comfortably.
pub const DIVE_ESCAPE_STEP: f32 = 26.0;

/// `TalonBury` length per phase: the punish window a dodged dive opens.
pub const TALON_BURY_DURATION: [f32; 3] = [1.15, 1.05, 0.95];

/// Incoming damage on the buried bird (the back of the neck).
pub const TALON_BURY_DAMAGE_MULT: f32 = 1.40;

/// `Wheel`: the climb back up after a dive the rat did not dodge.
pub const WHEEL_DURATION: f32 = 0.55;

// ---- the hover -----------------------------------------------------------

/// `Stoop`: the descent from overhead to the low hover.
pub const STOOP_DURATION: f32 = 0.70;

/// How fast it comes down while stooping (px/s, horizontal).
pub const STOOP_SPEED: f32 = 120.0;

/// `Hover` length per phase. A wounded bird hovers lower and longer.
pub const HOVER_DURATION: [f32; 3] = [0.90, 1.15, 1.50];

/// Hover drift speed per phase (`40 / 48 / 56` px/s).
pub const HOVER_SPEED: [f32; 3] = [40.0, 48.0, 56.0];

/// The ring the hover holds the rat at, in px (`near`, `far`).
pub const HOVER_BAND: [f32; 2] = [26.0, 58.0];

/// How many hover attacks it takes before it climbs back out of reach.
pub const HOVER_LIMIT: [u32; 3] = [1, 2, 3];

/// Downdraft rings (wing-beats) in one hover, spread evenly through it.
pub const DOWNDRAFT_BEATS: u32 = 3;

/// The share of each beat a downdraft ring pushes for (the rest is the wing
/// coming back up: the calm gap that keeps the rings escapable).
pub const DOWNDRAFT_DUTY: f32 = 0.42;

/// How long one downdraft ring pushes for, in seconds (the ring's own window;
/// the controller sizes it from the hover it belongs to).
pub const DOWNDRAFT_ACTIVE: f32 = 0.20;

/// The ring's inner/outer radius (it pushes outward, away from the bird).
pub const DOWNDRAFT_INNER: f32 = 12.0;
pub const DOWNDRAFT_OUTER: f32 = 40.0;

/// The downdraft's outward push (px/s): under the rat's walk, so the rings are
/// pressure on the position, not a loss of control.
pub const DOWNDRAFT_PUSH: f32 = 52.0;

/// Most rings one hover can beat out (a bound on the flags word).
pub const DOWNDRAFT_MAX: u32 = DOWNDRAFT_BEATS;

/// `Climb`: back up out of reach (harmless, no attack).
pub const CLIMB_DURATION: f32 = 0.60;

// ---- the feather darts ---------------------------------------------------

/// `DartWindup` per phase: the wedge is painted, then it fills.
pub const DART_WINDUP: [f32; 3] = [0.74, 0.66, 0.58];

/// The wedge freezes here so the rat can step out of it.
pub const DART_LOCK_PROGRESS: f32 = 0.55;

/// Extra grace between the freeze and the darts (the hazard's own warn).
pub const DART_GAP: f32 = 0.16;

/// The wedge's reach from the bill (px).
pub const DART_REACH: f32 = 118.0;

/// The wedge's inner radius (it starts at the bill, not inside the bird).
pub const DART_INNER: f32 = 9.0;

/// Half the wedge's angle (radians): a `34°` fan.
pub const DART_HALF_ANGLE: f32 = 0.30;

/// Darts landing inside the wedge, once.
pub const DART_DAMAGE: f32 = 11.0;

/// The darts' live window.
pub const DART_ACTIVE: f32 = 0.14;

/// `DartFan`: the darts cross the deck.
pub const DART_FAN_DURATION: f32 = 0.34;

/// `GrabWindup` per phase: the talon reach is marked.
pub const GRAB_WINDUP: [f32; 3] = [0.86, 0.78, 0.70];

/// The reach freezes here.
pub const GRAB_LOCK_PROGRESS: f32 = 0.55;

/// Nearest and furthest the reach is committed (px from the talons).
pub const GRAB_MIN: f32 = 14.0;
pub const GRAB_REACH: f32 = 52.0;

/// The reach's half-width (px): narrow enough that one step leaves it.
pub const GRAB_HALF_WIDTH: f32 = 8.0;

/// `Grab`: the talons close and the carry runs.
pub const GRAB_DURATION: f32 = 0.80;

/// The carry: seconds the grabbed rat is dragged, and how fast (px/s).
pub const GRAB_CARRY_SECS: f32 = 0.45;
pub const GRAB_CARRY_DRAG: f32 = 58.0;

/// What being carried and dropped costs.
pub const GRAB_DAMAGE: f32 = 16.0;

/// The drop's circle at the talon point.
pub const GRAB_DROP_RADIUS: f32 = 11.0;

/// The drop's live window.
pub const GRAB_ACTIVE: f32 = 0.12;

/// The step the rat needs off the reach (the suppression rule).
pub const GRAB_ESCAPE_STEP: f32 = 24.0;

// ---- the talon rake ------------------------------------------------------

/// `RakeWindup` per phase: the lane is painted.
pub const RAKE_WINDUP: [f32; 3] = [0.80, 0.72, 0.64];

/// The lane freezes here.
pub const RAKE_LOCK_PROGRESS: f32 = 0.55;

/// Pass speed per phase (`190 / 210 / 230` px/s).
pub const RAKE_SPEED: [f32; 3] = [190.0, 210.0, 230.0];

/// The live corridor's half-width (the talons' trail).
pub const RAKE_HALF_WIDTH: f32 = 9.0;

/// What a clipped rake costs.
pub const RAKE_DAMAGE: f32 = 15.0;

/// The shortest rake worth flying.
pub const RAKE_MIN_LEN: f32 = 90.0;

/// Clear deck the rat needs on the safe side of a lane.
pub const RAKE_SAFE_SIDE: f32 = 26.0;

/// How far inside the deck's ends a lane starts and stops.
pub const RAKE_END_MARGIN: f32 = 24.0;

// ---- phase change & death ------------------------------------------------

/// The harmless phase-change pose (owner's rule: 1.2 s, no contact, no attack).
pub const ALIGHT_DURATION: f32 = 1.20;

/// The death beat: it falls out of the sky and stays down.
pub const DEATH_DURATION: f32 = 2.40;

/// The death's dust ring.
pub const DEATH_RING_RADIUS: f32 = 36.0;

// ---- art only ------------------------------------------------------------

/// Screen lift of the bird when it is high overhead (px).
pub const HIGH_LIFT: f32 = 58.0;

/// Screen lift of a phase-1 hover (px).
pub const HOVER_LIFT: f32 = 11.0;

/// Key light, upper left (the same convention as the other boss art).
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// Where the light throws the shadow, in px of ground offset per unit of lift.
const SHADOW_THROW: Vec2 = Vec2 { x: 0.22, y: 0.30 };

// ============================================================================
// State spec
// ============================================================================

/// The Redtail's move set. Every variant documents its timing, telegraph,
/// lethal area, damage and the counterplay the controller builds to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HawkState {
    /// **Circling overhead.** Only the shadow is on the deck and the bird
    /// cannot be hit at all.
    ///
    /// - Duration: [`CIRCLE_DURATION`] (`1.05 / 0.95 / 0.85` s).
    /// - Warning: the shadow itself (always drawn, always the boss's position).
    /// - Dangerous area: none. This is the beat that lets the rat read where
    ///   the next attack will come from.
    Circle,

    /// **The dive.** The shadow stops at [`DIVE_LOCK_PROGRESS`], the talon
    /// reticle snaps onto the deck and freezes, the bird screams and falls on
    /// the mark.
    ///
    /// - Duration: [`DIVE_DURATION`] (`1.30 / 1.18 / 1.06` s).
    /// - Warning: the reticle, from the lock to the impact.
    /// - Dangerous area: a [`DIVE_RADIUS`] circle at the mark, once for
    ///   [`DIVE_DAMAGE`], and a damage-free [`DIVE_RING_OUTER`] shockwave.
    Dive,

    /// **The talons are in the planks.** The punish window a dodged dive opens:
    /// no body contact, ×[`TALON_BURY_DAMAGE_MULT`] incoming, and the back of
    /// the neck ([`Nape`](Self::TalonBury)) exposed as the first body circle.
    ///
    /// - Duration: [`TALON_BURY_DURATION`] (`1.15 / 1.05 / 0.95` s).
    /// - Counter: hit the nape; the window is the whole point of dodging.
    TalonBury,

    /// **Wheel.** It bounces out of a dive that *did* connect and climbs back
    /// out of reach. No contact, no attack.
    ///
    /// - Duration: [`WHEEL_DURATION`] (0.55 s).
    Wheel,

    /// **The stoop.** It comes down out of the sky to a low hover: hittable,
    /// watching, not yet striking.
    ///
    /// - Duration: [`STOOP_DURATION`] (0.70 s).
    /// - Dangerous area: the body, [`CONTACT_DAMAGE`] (8).
    Stoop,

    /// **The hover.** Wings beating two body-lengths over the planks, holding a
    /// [`HOVER_BAND`] ring around the rat and beating out downdraft rings.
    ///
    /// - Duration: [`HOVER_DURATION`] (`0.90 / 1.15 / 1.50` s; a wounded bird
    ///   hovers lower and longer).
    /// - Dangerous area: the body, [`CONTACT_DAMAGE`] (8); the downdraft rings
    ///   push without damage ([`DOWNDRAFT_PUSH`]).
    Hover,

    /// **The fan is painted.** Three darts' worth of wedge marks out of the
    /// bill and freezes at [`DART_LOCK_PROGRESS`].
    ///
    /// - Duration: [`DART_WINDUP`] (`0.74 / 0.66 / 0.58` s).
    DartWindup,

    /// **The darts fly.** The wedge is live once for [`DART_DAMAGE`].
    ///
    /// - Duration: [`DART_FAN_DURATION`] (0.34 s).
    DartFan,

    /// **The reach is marked.** A talon's-length corridor is drawn to the rat
    /// and freezes at [`GRAB_LOCK_PROGRESS`].
    ///
    /// - Duration: [`GRAB_WINDUP`] (`0.86 / 0.78 / 0.70` s).
    GrabWindup,

    /// **The grab.** The talons close: a rat still on the mark is carried
    /// toward the bird for [`GRAB_CARRY_SECS`] and dropped for [`GRAB_DAMAGE`].
    ///
    /// - Duration: [`GRAB_DURATION`] (0.80 s).
    /// - Counter: step or dash off the marked reach during the tell.
    Grab,

    /// **The rake's lane is painted** down the bed at the rat's row.
    ///
    /// - Duration: [`RAKE_WINDUP`] (`0.80 / 0.72 / 0.64` s).
    RakeWindup,

    /// **The rake.** A low strafing flight the length of the lane, talons
    /// trailing; one [`RAKE_DAMAGE`] hit anywhere in it.
    RakePass,

    /// **Alight.** The 1.2 s harmless pose change at a phase threshold: it
    /// drops to the planks and shakes its hackles out.
    ///
    /// - Duration: [`ALIGHT_DURATION`] (1.20 s), no contact, no attack.
    Alight,

    /// **Climb.** Back up out of reach, wings working.
    ///
    /// - Duration: [`CLIMB_DURATION`] (0.60 s).
    Climb,

    /// **Death.** It comes down out of the sky for the last time.
    Death,
}

/// Is the bird out of reach in this state (only its shadow on the deck)?
///
/// While this holds the boss has **no hit shapes at all**, so no weapon can
/// touch it; the shadow is still drawn, so its position is always readable.
#[inline]
pub fn is_overhead(state: HawkState) -> bool {
    matches!(
        state,
        HawkState::Circle | HawkState::Dive | HawkState::Wheel
    )
}

/// Is the bird down where a ground weapon can reach it?
#[inline]
pub fn is_grounded(state: HawkState) -> bool {
    !is_overhead(state) && state != HawkState::Death
}

/// Incoming damage multiplier (the art owns it, as for the other bosses).
#[inline]
pub fn incoming_damage_mult(state: HawkState) -> f32 {
    match state {
        HawkState::TalonBury => TALON_BURY_DAMAGE_MULT,
        HawkState::Death => 0.0,
        _ => 1.0,
    }
}

/// Does the bird deal body contact in this state?
#[inline]
pub fn has_body_contact(state: HawkState) -> bool {
    matches!(
        state,
        HawkState::Stoop
            | HawkState::Hover
            | HawkState::DartWindup
            | HawkState::DartFan
            | HawkState::GrabWindup
            | HawkState::Grab
            | HawkState::RakeWindup
            | HawkState::RakePass
    )
}

/// Is downdraft ring `fired` due, `elapsed` seconds into a `duration` s hover?
#[inline]
pub fn downdraft_due(elapsed: f32, duration: f32, fired: u32) -> bool {
    if fired >= DOWNDRAFT_MAX {
        return false;
    }
    let beat = (duration / DOWNDRAFT_BEATS as f32).max(0.01);
    elapsed >= f32::from(fired as u16) * beat
}

/// Is a downdraft beat pulling at `progress` (0..=1 through `Hover`)?
///
/// The flatbed's own trailer pass reads this through `BossState`, so the loose
/// gems and light bodies answer the same wing-beat the rat does.
#[inline]
pub fn downdraft_active(state: HawkState, progress: f32) -> bool {
    if state != HawkState::Hover {
        return false;
    }
    let within = (progress.clamp(0.0, 1.0) * DOWNDRAFT_BEATS as f32).fract();
    within < DOWNDRAFT_DUTY
}

/// The downdraft ring a hover's wing-beat pushes with (damage-free).
#[inline]
pub fn downdraft_shape(at: Vec2) -> crate::game::ecs::systems::boss::HazardShape {
    use crate::game::ecs::systems::boss::HazardShape;
    HazardShape::Annulus {
        center: at,
        inner: DOWNDRAFT_INNER,
        outer: DOWNDRAFT_OUTER,
    }
}

/// Screen lift of the bird in this state, in px: the altitude read.
///
/// The shadow is drawn at the ground point and the sprite is lifted by this, so
/// the *pair* is the altitude. A wounded bird hovers lower ([`hover_altitude`]).
#[inline]
pub fn altitude(state: HawkState, phase: u8, progress: f32) -> f32 {
    let p = clamp01(progress);
    match state {
        HawkState::Circle => HIGH_LIFT,
        HawkState::Dive => {
            if p < DIVE_LOCK_PROGRESS {
                HIGH_LIFT
            } else {
                let t = ((p - DIVE_LOCK_PROGRESS)
                    / (DIVE_IMPACT_PROGRESS - DIVE_LOCK_PROGRESS).max(0.01))
                .clamp(0.0, 1.0);
                // A stoop is a dive: the last third is the fall.
                HIGH_LIFT * (1.0 - t * t)
            }
        }
        HawkState::TalonBury | HawkState::Death => 0.0,
        HawkState::Wheel => HIGH_LIFT * smoothstep(0.0, 0.8, p),
        HawkState::Climb => hover_altitude(phase)
            + (HIGH_LIFT - hover_altitude(phase)) * smoothstep(0.0, 0.9, p),
        HawkState::Stoop => {
            HIGH_LIFT + (hover_altitude(phase) - HIGH_LIFT) * smoothstep(0.1, 1.0, p)
        }
        HawkState::Alight => hover_altitude(phase) * 0.4,
        _ => hover_altitude(phase),
    }
}

/// The hover's screen lift for a phase: lower as the bird is wounded.
#[inline]
pub fn hover_altitude(phase: u8) -> f32 {
    match phase {
        0 | 1 => HOVER_LIFT,
        2 => HOVER_LIFT - 3.0,
        _ => HOVER_LIFT - 5.5,
    }
}

// ============================================================================
// View
// ============================================================================

/// A pure, `Copy` snapshot of everything the art needs. Built by
/// `Boss::hawk_view`; carries no simulation borrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HawkView {
    /// Body centre in world px (the ground footprint: over the shadow while
    /// overhead, at the bird while it is down).
    pub pos: Vec2,
    /// Facing (normalised defensively).
    pub facing: Vec2,
    pub state: HawkState,
    /// 0..=1 through the current state.
    pub progress: f32,
    /// Animation clock, seconds.
    pub anim: f32,
    /// Hit flash, 0..=1 (fades over ~0.12 s).
    pub hit_flash: f32,
    /// Remaining health fraction.
    pub hp_frac: f32,
    /// Fight phase (1..=3).
    pub phase: u8,
    /// The marked point: the reticle's centre, a lane's far end, the grab's far
    /// end, or the drop point, depending on the state.
    pub target: Vec2,
    /// Where the mark started: the shadow's stop point (dive), the bill (darts),
    /// the talons (grab) or the lane's near end (rake).
    pub lane_start: Vec2,
    /// The feint's decoy point (drawn hollow; only meaningful in a feint dive).
    pub decoy: Vec2,
    /// `+1`/`-1`: which way the shadow circles, and which shoulder a feint jinks
    /// to.
    pub sweep: f32,
    /// The bird is out of its hold, running the deck after a rat that fled.
    pub hunting: bool,
}

impl HawkView {
    /// A circling view looking at `target`.
    pub fn new(pos: Vec2, facing: Vec2, target: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: HawkState::Circle,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
            hp_frac: 1.0,
            phase: 1,
            target,
            lane_start: pos,
            decoy: pos,
            sweep: 1.0,
            hunting: false,
        }
    }

    /// Is the bird out of reach right now?
    #[inline]
    pub fn overhead(&self) -> bool {
        is_overhead(self.state)
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

    /// Was this dive a feint (does the decoy mean anything)?
    #[inline]
    pub fn feint(&self) -> bool {
        self.state == HawkState::Dive
            && self.progress >= DIVE_FEINT_PROGRESS
            && self.progress < DIVE_LOCK_PROGRESS
            && self.decoy.dist(self.target) > 0.5
    }
}

// ============================================================================
// Geometry queries (art <-> controller <-> framework contract)
// ============================================================================

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Facing, normalised defensively.
#[inline]
pub fn unit_facing(view: &HawkView) -> Vec2 {
    if view.facing.len_sq() > 1e-4 {
        view.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// The bill tip: the origin of the dart wedge.
#[inline]
pub fn bill_pos(view: &HawkView) -> Vec2 {
    let f = unit_facing(view);
    match view.state {
        HawkState::TalonBury => view.pos + f * (HEAD_OFFSET + BEAK_LENGTH * 0.35),
        _ => view.pos + f * (HEAD_OFFSET + BEAK_LENGTH),
    }
}

/// The exposed nape (the weak point while the talons are buried).
///
/// The bird is nose-down in the planks, so the nape is the raised point behind
/// the skull — the one place on a raptor a rat can bite.
#[inline]
pub fn nape_pos(view: &HawkView) -> Vec2 {
    let f = unit_facing(view);
    view.pos + f * (HEAD_OFFSET - NAPE_BACK)
}

/// The weak point, when it is exposed.
#[inline]
pub fn weak_point(view: &HawkView) -> Option<(Vec2, f32)> {
    match view.state {
        HawkState::TalonBury => Some((nape_pos(view), NAPE_RADIUS)),
        _ => None,
    }
}

/// Three body circles: tail root, body, head.
///
/// While the talons are buried the first circle is the **nape** instead, which
/// is what makes it the first auto-aim point and the punish window's target.
#[inline]
pub fn body_hit_circles(view: &HawkView) -> [(Vec2, f32); 3] {
    let f = unit_facing(view);
    let body = view.pos;
    let head = body + f * HEAD_OFFSET;
    let tail = body - f * (TAIL_LENGTH * 0.45);
    match weak_point(view) {
        Some((nape, r)) => [
            (nape, r),
            (body - f * 2.0, BODY_RADIUS * 0.84),
            (
                body + f * (HEAD_OFFSET * 0.55),
                HEAD_RADIUS * 0.95,
            ),
        ],
        None => [
            (tail, 5.0),
            (body, BODY_RADIUS * 0.84),
            (head, HEAD_RADIUS),
        ],
    }
}

/// The impact circle of a locked dive: `(centre, radius)`.
#[inline]
pub fn impact_circle(view: &HawkView) -> (Vec2, f32) {
    (view.target, DIVE_RADIUS)
}

/// The dart wedge of a `DartWindup`/`DartFan`: `(origin, reach, facing, half
/// angle)`.
#[inline]
pub fn dart_wedge(view: &HawkView) -> (Vec2, f32, Vec2, f32) {
    let origin = view.lane_start;
    let to = view.target - origin;
    let facing = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        unit_facing(view)
    };
    let reach = to.len().clamp(DART_INNER + 4.0, DART_REACH);
    (origin, reach, facing, DART_HALF_ANGLE)
}

/// The talon reach of a `GrabWindup`/`Grab`: `(near, far, half width)`.
#[inline]
pub fn grab_reach(view: &HawkView) -> (Vec2, Vec2, f32) {
    (view.lane_start, view.target, GRAB_HALF_WIDTH)
}

/// The rake's lane: `(near, far)`.
#[inline]
pub fn rake_lane(view: &HawkView) -> (Vec2, Vec2) {
    (view.lane_start, view.target)
}

/// Where the drop lands (the talon point the tell showed).
#[inline]
pub fn drop_point(view: &HawkView) -> Vec2 {
    view.target
}

// ============================================================================
// Pose
// ============================================================================

#[derive(Clone, Copy)]
struct Pose {
    /// Screen-space ground footprint (where the shadow is / where it can be
    /// hit).
    ground: Vec2,
    /// Screen-space body centre (the ground point, lifted by the altitude).
    body: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    /// Wing spread 0 (folded) .. 1 (full span).
    spread: f32,
    /// Wing sweep: `>0` forward (braking), `<0` swept back (stooping).
    sweep: f32,
    /// Wing beat clock (`-1..1`).
    flap: f32,
    /// Tail fan spread 0..1.
    tail_fan: f32,
    /// Tail lift `-1..1` (a redtail uses it as a rudder).
    tail_lift: f32,
    /// Head centre in screen space.
    head: Vec2,
    /// Head-up/head-down lean (`>0` forward, `<0` coiled back).
    head_lunge: f32,
    /// Body stretch along `f`.
    stretch: f32,
    /// Screen lift of the whole bird.
    lift: f32,
    /// Hackle ruffle 0..1 (the phase-change pose and phase 2+).
    ruffle: f32,
    /// Shake jitter in px.
    jitter: Vec2,
    /// Fade out (death).
    alpha: f32,
    /// Death progress 0..1.
    death: f32,
    hit: f32,
    hp: f32,
    enraged: f32,
    critical: f32,
    anim: f32,
    state: HawkState,
    progress: f32,
    accent: Rgb,
    /// Screen-space mark.
    target: Vec2,
    /// Screen-space mark origin.
    lane_start: Vec2,
    /// Screen-space decoy.
    decoy: Vec2,
    sweep_side: f32,
    lift01: f32,
    hunting: bool,
    phase: u8,
}

impl Pose {
    fn from_view(view: &HawkView, cam: Vec2, accent: Rgb) -> Self {
        let f = unit_facing(view);
        let p = f.perp();
        let prog = clamp01(view.progress);
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

        let lift = altitude(view.state, view.phase, prog);
        let lift01 = (lift / HIGH_LIFT).clamp(0.0, 1.0);
        let ground = view.pos - cam;

        let mut spread = 0.0f32;
        let mut sweep = 0.0f32;
        let mut flap = 0.0f32;
        let mut head_lunge = 0.0f32;
        let mut stretch = 0.0f32;
        let mut tail_lift = 0.0f32;
        let mut tail_fan = 0.15f32;
        let mut ruffle = 0.0f32;
        let mut jitter = Vec2::ZERO;
        let mut alpha = 1.0f32;
        let mut death = 0.0f32;

        match view.state {
            HawkState::Circle => {
                // A long gliding circle: wings out, tail steering, one slow
                // beat per pass.
                spread = 0.82;
                flap = (anim * 2.6).sin() * 0.35;
                sweep = -0.12;
                tail_fan = 0.42;
                tail_lift = 0.22 * (anim * 1.7).sin();
                head_lunge = 0.18;
            }
            HawkState::Dive => {
                if prog < DIVE_LOCK_PROGRESS {
                    // The shadow stops reading the floor: wings come up, the
                    // head drops, the whole body coils over the mark.
                    let c = smoothstep(0.0, DIVE_LOCK_PROGRESS, prog);
                    spread = 0.82 - 0.30 * c;
                    flap = (anim * 6.0).sin() * 0.25 * (1.0 - c);
                    sweep = -0.12 + 0.20 * c;
                    tail_fan = 0.42 + 0.30 * c;
                    head_lunge = 0.18 + 0.55 * c;
                    stretch = 0.25 * c;
                } else {
                    // The stoop: wings folded to the wrist, tail shut, the body
                    // a dart. The last third of the lift is gone before the
                    // impact.
                    let t = ((prog - DIVE_LOCK_PROGRESS)
                        / (DIVE_IMPACT_PROGRESS - DIVE_LOCK_PROGRESS).max(0.01))
                    .clamp(0.0, 1.0);
                    spread = 0.52 - 0.42 * t;
                    sweep = -0.55 - 0.45 * t;
                    flap = 0.0;
                    tail_fan = 0.72 - 0.50 * t;
                    tail_lift = 0.30 * t;
                    head_lunge = 0.73 + 0.35 * t;
                    stretch = 0.25 + 0.85 * t;
                    jitter = p * ((anim * 46.0).sin() * 0.6 * t);
                }
            }
            HawkState::TalonBury => {
                // Nose down in the oak, tail up, wings thrown across the planks
                // and both feet planted. The nape is the raised point.
                spread = 0.72;
                sweep = 0.30;
                flap = 0.0;
                tail_fan = 0.30;
                tail_lift = -0.55;
                head_lunge = 1.0;
                stretch = 0.35;
                let tug = (anim * 8.0).sin();
                jitter = p * (tug * 1.2);
            }
            HawkState::Wheel => {
                // Wings catch the air: spread, beat, rise.
                let t = smoothstep(0.0, 0.8, prog);
                spread = 0.55 + 0.45 * t;
                flap = (anim * 20.0).sin();
                sweep = -0.30 * t;
                tail_fan = 0.55 * t + 0.2;
                tail_lift = 0.25 * t;
                head_lunge = 0.35 * (1.0 - t);
            }
            HawkState::Stoop => {
                // Braking on the way down: wings wide, tail down, legs out.
                let t = smoothstep(0.1, 1.0, prog);
                spread = 0.62 + 0.38 * t;
                flap = (anim * 16.0).sin() * (0.4 + 0.6 * t);
                sweep = 0.22 * t;
                tail_fan = 0.55 + 0.35 * t;
                tail_lift = -0.35 * t;
                head_lunge = 0.45;
            }
            HawkState::Hover => {
                // Beating hard, feet trailing: a raptor deciding.
                let pushing = downdraft_active(view.state, prog);
                spread = 1.0;
                flap = (anim * 15.0).sin();
                sweep = 0.10 + 0.22 * flap;
                tail_fan = 0.62 + 0.18 * flap;
                tail_lift = -0.20;
                head_lunge = 0.30;
                jitter = Vec2::new(0.0, -flap * 0.8);
                if pushing {
                    flap = -0.85;
                }
            }
            HawkState::DartWindup => {
                let c = smoothstep(0.0, DART_LOCK_PROGRESS, prog);
                // Head thrown forward, wings half-cupped around the aim.
                head_lunge = 0.35 + 0.55 * c;
                spread = 0.86;
                flap = (anim * 13.0).sin() * 0.5;
                sweep = 0.35 * c;
                tail_fan = 0.70;
                tail_lift = -0.28;
                ruffle = 0.25 * c;
            }
            HawkState::DartFan => {
                head_lunge = 0.95;
                spread = 0.92;
                flap = (anim * 26.0).sin();
                sweep = 0.42;
                tail_fan = 0.75;
                tail_lift = -0.30;
            }
            HawkState::GrabWindup => {
                let c = smoothstep(0.0, GRAB_LOCK_PROGRESS, prog);
                // Feet down and forward, wings flared as brakes.
                head_lunge = 0.30 + 0.30 * c;
                spread = 0.90 + 0.10 * c;
                sweep = 0.40 * c;
                flap = (anim * 12.0).sin() * (0.3 + 0.4 * c);
                tail_fan = 0.80;
                tail_lift = -0.45;
            }
            HawkState::Grab => {
                // The talons close and the bird holds: wings wide, body still.
                head_lunge = 0.55;
                spread = 1.0;
                sweep = 0.30;
                flap = (anim * 9.0).sin() * 0.35;
                tail_fan = 0.85;
                tail_lift = -0.50;
                jitter = Vec2::new(0.0, (anim * 30.0).sin() * 0.4);
            }
            HawkState::RakeWindup => {
                let c = smoothstep(0.0, RAKE_LOCK_PROGRESS, prog);
                // Banked over the lane, feet trailing in the deck.
                head_lunge = 0.45;
                spread = 0.75;
                sweep = -0.15 - 0.25 * c;
                flap = (anim * 14.0).sin() * 0.6;
                tail_fan = 0.62;
                tail_lift = 0.35;
            }
            HawkState::RakePass => {
                head_lunge = 0.70;
                spread = 0.62;
                sweep = -0.72;
                flap = (anim * 8.0).sin() * 0.22;
                tail_fan = 0.45;
                tail_lift = 0.30;
                jitter = p * ((anim * 40.0).sin() * 0.5);
            }
            HawkState::Alight => {
                // Down on the planks, hackles up, shaking the road out.
                let t = (prog / 0.32).min(1.0);
                let settle = ((prog - 0.70) / 0.30).clamp(0.0, 1.0);
                ruffle = t * (1.0 - settle);
                spread = 0.55 + 0.35 * ruffle;
                flap = (anim * 18.0).sin() * 0.4 * ruffle;
                tail_fan = 0.30 + 0.55 * ruffle;
                tail_lift = 0.20 * ruffle;
                head_lunge = 0.30 - 0.45 * ruffle;
                jitter = p * ((prog * 46.0).sin() * 2.0 * ruffle);
            }
            HawkState::Climb => {
                let t = smoothstep(0.0, 0.9, prog);
                spread = 1.0 - 0.15 * t;
                flap = (anim * 19.0).sin();
                sweep = -0.20 * t;
                tail_fan = 0.70;
                tail_lift = 0.15 * t;
                head_lunge = 0.25;
            }
            HawkState::Death => {
                death = prog;
                // The fall, then the crumple.
                if prog < 0.34 {
                    let t = prog / 0.34;
                    spread = 0.55 + 0.45 * t;
                    flap = (anim * 22.0).sin() * (1.0 - t);
                    head_lunge = 0.55;
                    jitter = p * ((anim * 50.0).sin() * 1.2);
                } else {
                    let t = ((prog - 0.34) / 0.66).min(1.0);
                    spread = 1.0 - 0.45 * t;
                    flap = 0.0;
                    tail_fan = 0.25;
                    tail_lift = -0.6;
                    head_lunge = 0.9 - 0.4 * t;
                    if prog > 0.80 {
                        alpha = 1.0 - ((prog - 0.80) / 0.20) * 0.40;
                    }
                }
            }
        }

        // Phase 2+ ruffles the hackles permanently; phase 3 is ragged.
        ruffle = ruffle.max(enraged * 0.55);

        let body = ground + Vec2::new(0.0, -lift) + jitter;
        let head = body + f * (HEAD_OFFSET + head_lunge * 2.2 + stretch * 2.0);

        Self {
            ground,
            body,
            f,
            p,
            fl: f.dot(LIGHT),
            pl: p.dot(LIGHT),
            spread,
            sweep,
            flap,
            tail_fan,
            tail_lift,
            head,
            head_lunge,
            stretch,
            lift,
            ruffle,
            jitter,
            alpha,
            death,
            hit,
            hp,
            enraged,
            critical,
            anim,
            state: view.state,
            progress: prog,
            accent,
            target: view.target - cam,
            lane_start: view.lane_start - cam,
            decoy: view.decoy - cam,
            sweep_side: if view.sweep < 0.0 { -1.0 } else { 1.0 },
            lift01,
            hunting: view.hunting,
            phase: view.phase.clamp(1, 3),
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
// Small pixel helpers (stack-only, bounds-checked by bbox)
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
fn fill_disc(pb: &mut PixelBuffer, at: Vec2, radius: f32, c: Rgb, a: f32) {
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, at, radius + 1.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - at.x;
            let dy = y as f32 + 0.5 - at.y;
            if dx * dx + dy * dy <= radius * radius {
                blend_px(pb, x, y, c, a);
            }
        }
    }
}

fn ring(pb: &mut PixelBuffer, at: Vec2, radius: f32, c: Rgb, a: f32, width: f32) {
    if radius < 0.5 || a <= 0.01 {
        return;
    }
    let steps = ((radius * 6.0).round() as i32).clamp(16, 220);
    let w = width.max(0.8);
    for i in 0..steps {
        let ang = i as f32 / steps as f32 * std::f32::consts::TAU;
        let d = Vec2::from_angle(ang);
        for k in 0..w.round().max(1.0) as i32 {
            let r = radius + k as f32;
            let x = (at.x + d.x * r).round() as i32;
            let y = (at.y + d.y * r).round() as i32;
            blend_px(pb, x, y, c, a);
        }
    }
}

fn line_solid(pb: &mut PixelBuffer, a: Vec2, b: Vec2, c: Rgb, alpha: f32) {
    let d = b - a;
    let len = d.len();
    if len <= 0.01 {
        blend_px(pb, a.x.round() as i32, a.y.round() as i32, c, alpha);
        return;
    }
    let steps = len.ceil() as i32;
    for i in 0..=steps {
        let p = a + d * (i as f32 / steps as f32);
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, c, alpha);
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

// ---- palette -------------------------------------------------------------

/// The redtail's own colours: chocolate mantle, barred cream belly, a rufous
/// tail, yellow cere and feet.
const INK: Rgb = (10, 9, 11);
const MANTLE: Rgb = (86, 58, 36);
const MANTLE_HI: Rgb = (140, 100, 58);
const BACK_HI: Rgb = (182, 138, 82);
const CREAM: Rgb = (236, 220, 184);
const CREAM_SH: Rgb = (172, 146, 108);
const BAR: Rgb = (104, 76, 52);
const TAIL: Rgb = (178, 88, 50);
const TAIL_HI: Rgb = (222, 132, 74);
const TAIL_BAND: Rgb = (58, 34, 26);
const CERE: Rgb = (226, 186, 86);
const BEAK: Rgb = (74, 62, 54);
const BEAK_HI: Rgb = (128, 110, 92);
const FOOT: Rgb = (232, 198, 104);
const CLAW: Rgb = (44, 38, 34);
const EYE: Rgb = (255, 202, 66);
const NAPE_GOLD: Rgb = (255, 232, 138);

struct Plumage {
    outline: Rgb,
    shadow: Rgb,
    mantle: Rgb,
    mantle_hi: Rgb,
    back_hi: Rgb,
    cream: Rgb,
    cream_sh: Rgb,
    bar: Rgb,
    tail: Rgb,
    tail_hi: Rgb,
    tail_band: Rgb,
    cere: Rgb,
    beak: Rgb,
    foot: Rgb,
    claw: Rgb,
}

fn plumage(pose: &Pose) -> Plumage {
    let dead = ((pose.death - 0.30) / 0.70).clamp(0.0, 1.0);
    let e = pose.enraged;
    let mut mantle = mix(MANTLE, (104, 46, 30), e * 0.75);
    let mut mantle_hi = mix(MANTLE_HI, (162, 74, 42), e * 0.7);
    let mut back_hi = mix(BACK_HI, (198, 108, 58), e * 0.65);
    let mut cream = mix(CREAM, (248, 214, 178), e * 0.5);
    let mut cream_sh = mix(CREAM_SH, (186, 132, 104), e * 0.5);
    let mut tail = mix(TAIL, (206, 62, 34), e * 0.8);
    let mut tail_hi = mix(TAIL_HI, (240, 108, 52), e * 0.8);
    let mut bar = mix(BAR, (126, 48, 34), e * 0.6);
    let mut shadow = (12, 10, 12);
    let outline = if pose.hit > 0.55 { (96, 62, 40) } else { INK };
    if dead > 0.0 {
        mantle = mix(mantle, (62, 56, 52), dead * 0.6);
        mantle_hi = mix(mantle_hi, (94, 86, 78), dead * 0.6);
        back_hi = mix(back_hi, (116, 104, 88), dead * 0.6);
        cream = mix(cream, (168, 160, 146), dead * 0.55);
        cream_sh = mix(cream_sh, (110, 102, 92), dead * 0.55);
        tail = mix(tail, (92, 62, 54), dead * 0.75);
        tail_hi = mix(tail_hi, (118, 84, 70), dead * 0.75);
        bar = mix(bar, (84, 76, 68), dead * 0.6);
        shadow = (26, 24, 24);
    }
    Plumage {
        outline,
        shadow,
        mantle,
        mantle_hi,
        back_hi,
        cream,
        cream_sh,
        bar,
        tail,
        tail_hi,
        tail_band: TAIL_BAND,
        cere: CERE,
        beak: BEAK,
        foot: FOOT,
        claw: CLAW,
    }
}

// ---- the bird's own body, as distance fields ----------------------------

/// One wing's field, in body-local `(u, v)`. Returns `(distance, span, chord)`.
///
/// Folded, the wing lies along the flank; spread, it is a broad secondary blade
/// out to a wrist and then fanned primaries — a raptor's wing, not a songbird's.
#[inline]
fn wing_sdf(pose: &Pose, side: f32, u: f32, v: f32) -> (f32, f32, f32) {
    let spread = pose.spread.clamp(0.0, 1.0);
    let flap_pull = pose.flap * 2.4 * spread;
    let root = (3.6, side * 4.2);
    let wrist = (
        3.0 - 9.0 * (1.0 - spread) + 3.4 * pose.sweep,
        side * (5.2 + 10.5 * spread),
    );
    let tip = (
        -2.5 - 16.0 * (1.0 - spread) + 8.0 * pose.sweep + flap_pull,
        side * (4.4 + 21.5 * spread),
    );
    let d_inner = sd_taper(u, v, root, wrist, 5.4, 4.6 - 0.8 * spread);
    let d_outer = sd_taper(u, v, wrist, tip, 4.4 - 0.9 * spread, 1.25);
    // The barred trailing edge, smeared back as one blade.
    let d_trail = sd_taper(
        u,
        v,
        (root.0 - 4.6, side * 3.8),
        (wrist.0 - 5.0 * spread, side * (4.8 + 9.0 * spread)),
        3.9,
        2.5,
    );
    let mut d = smin(d_inner, d_outer, 2.2).min(d_trail);

    // Phase 2/3 ragged primaries (a bird that has been fighting).
    if pose.enraged > 0.3 && u < wrist.0 - 1.0 {
        let notch = (v * side * 1.55).sin().max(0.0) * (0.8 * pose.enraged + 0.5 * pose.critical);
        d += notch;
    }

    let span = ((v * side - 3.4) / (2.8 + 22.0 * spread)).clamp(0.0, 1.2);
    let chord = ((u - (tip.0 - 5.0)) / 15.0).clamp(0.0, 1.0);
    (d, span, chord)
}

/// The tail: a redtail's long rufous fan with dark terminal bands.
#[inline]
fn tail_sdf(pose: &Pose, u: f32, v: f32) -> (f32, f32) {
    let lift_v = pose.tail_lift * 2.8;
    let fan = 1.8 + 3.6 * pose.tail_fan;
    let d_center = sd_taper(u, v, (-4.0, 0.0), (-(4.0 + TAIL_LENGTH), lift_v), 3.4, 2.1);
    let d_left = sd_taper(
        u,
        v,
        (-4.0, -1.0),
        (-(3.4 + TAIL_LENGTH * 0.86), lift_v - fan),
        2.7,
        1.5,
    );
    let d_right = sd_taper(
        u,
        v,
        (-4.0, 1.0),
        (-(3.4 + TAIL_LENGTH * 0.86), lift_v + fan),
        2.7,
        1.5,
    );
    let d = smin(d_center, d_left.min(d_right), 1.5);
    let along = ((-u - 4.0) / TAIL_LENGTH).clamp(0.0, 1.0);
    (d, along)
}

/// The torso: a raptor's barrel chest tapering to the tail root.
#[inline]
fn torso_sdf(pose: &Pose, u: f32, v: f32) -> f32 {
    let chest = ellipse_sdf(u, v, 1.4, 0.0, 8.6, 6.4);
    let rump = ellipse_sdf(u, v, -5.4, 0.0, 6.0, 4.4);
    let mut d = smin(chest, rump, 2.4);
    if pose.ruffle > 0.02 {
        let ruff = ellipse_sdf(
            u,
            v,
            3.0,
            0.0,
            6.2 + 3.0 * pose.ruffle,
            6.6 + 2.4 * pose.ruffle,
        );
        d = smin(d, ruff, 1.6);
    }
    d
}

// ============================================================================
// Ground layer: the shadow, the reticle, the marks
// ============================================================================

/// The boss's ground layer: **the shadow** (the overhead read), the talon
/// reticle and its decoy, the dart wedge, the grab reach, the rake lane, the
/// downdraft rings, the impact burst and the HP arc.
pub fn draw_hawk_ground(pb: &mut PixelBuffer, view: &HawkView, cam: Vec2, accent: Rgb) {
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
    let mark_vis = pose.target.x >= -CULL_MARGIN
        && pose.target.x <= vw + CULL_MARGIN
        && pose.target.y >= -CULL_MARGIN
        && pose.target.y <= vh + CULL_MARGIN;
    if !body_vis && !mark_vis {
        return;
    }

    // The tell first (it is under the rat's feet), then the shadow over it so
    // the bird's position never hides.
    match view.state {
        HawkState::Dive => {
            draw_dive_mark(pb, &pose, view);
        }
        HawkState::TalonBury => {
            draw_buried_dust(pb, &pose);
        }
        HawkState::Stoop => {
            draw_stoop_dust(pb, &pose);
        }
        HawkState::Hover => {
            draw_downdraft(pb, &pose);
        }
        HawkState::DartWindup | HawkState::DartFan => {
            draw_dart_wedge(pb, &pose, view);
        }
        HawkState::GrabWindup | HawkState::Grab => {
            draw_grab_reach(pb, &pose, view);
        }
        HawkState::RakeWindup | HawkState::RakePass => {
            draw_rake_lane(pb, &pose, view);
        }
        HawkState::Alight => {
            draw_alight_ring(pb, &pose);
        }
        HawkState::Death => {
            draw_death_dust(pb, &pose);
        }
        _ => {}
    }

    if body_vis {
        draw_shadow(pb, &pose);
        if view.state != HawkState::Death && pose.hp > 0.0 {
            draw_hp_arc(pb, &pose);
        }
    }
}

/// **The shadow.** A raptor silhouette on the planks whose size, darkness and
/// edge hardness are the bird's altitude: faint and small high, dark and crisp
/// on the deck.
fn draw_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    let low = 1.0 - pose.lift01;
    let alpha = (0.17 + 0.40 * low) * pose.alpha;
    let centre = pose.ground + SHADOW_THROW * pose.lift;
    let scale = 0.62 + 0.52 * low;
    let soft = 3.0 - 2.0 * low;
    let reach = 34.0 * scale + soft + 2.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, centre, reach) else {
        return;
    };
    let ink: Rgb = (4, 4, 6);
    // The shadow's own body frame (same axis as the bird, at the ground).
    let (f, p) = (pose.f, pose.p);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - centre.x;
            let dy = y as f32 + 0.5 - centre.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = shadow_sdf(pose, u / scale, v / scale) * scale;
            if d < soft {
                let t = ((soft - d) / (soft + 1.1)).clamp(0.0, 1.0);
                blend_px(pb, x, y, ink, alpha * t);
            }
        }
    }
}

/// The silhouette: wings, torso and tail blended, in shadow-local space.
#[inline]
fn shadow_sdf(pose: &Pose, u: f32, v: f32) -> f32 {
    let d_t = torso_sdf(pose, u, v);
    let (d_tail, _) = tail_sdf(pose, u, v);
    let (d_wl, _, _) = wing_sdf(pose, -1.0, u, v);
    let (d_wr, _, _) = wing_sdf(pose, 1.0, u, v);
    let head = ellipse_sdf(u, v, HEAD_OFFSET, 0.0, 4.0, 3.6);
    d_t.min(d_tail).min(d_wl).min(d_wr).min(head)
}

/// The talon reticle: three prints in a V (the flatbed's landing-mark grammar)
/// plus the ring the impact will take. Frozen from the lock, live on the fall.
fn draw_dive_mark(pb: &mut PixelBuffer, pose: &Pose, view: &HawkView) {
    let at = pose.target;
    let p = pose.progress;
    let locked = p >= DIVE_LOCK_PROGRESS;
    let live = p >= DIVE_IMPACT_PROGRESS - 0.06;
    // The decoy print of a feint: hollow, dashed and dim, a beat behind the
    // real mark. It is cleared as soon as the real lock lands.
    if view.feint() {
        draw_talon_prints(pb, pose.decoy, pose.f, 0.55, (196, 176, 150), 0.38, false);
    }
    let tighten = if locked {
        let t = ((p - DIVE_LOCK_PROGRESS) / (DIVE_IMPACT_PROGRESS - DIVE_LOCK_PROGRESS).max(0.01))
            .clamp(0.0, 1.0);
        1.0 - 0.35 * t
    } else {
        // Seeking: the ring breathes while the shadow still reads the floor.
        1.08 + 0.10 * (pose.anim * 9.0).sin()
    };
    let col: Rgb = if live { (255, 246, 196) } else { (255, 202, 96) };
    let hot: Rgb = (255, 120, 78);
    let r = DIVE_RADIUS * tighten;
    ring(pb, at, r, if locked { col } else { (255, 190, 110) }, if locked { 0.95 } else { 0.55 }, 1.0);
    // The inner cross: the talons' aim.
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + if locked { 0.0 } else { pose.anim * 1.6 };
        let d = Vec2::from_angle(a);
        line_solid(
            pb,
            at + d * (r * 0.35),
            at + d * (r * 0.95),
            if locked { col } else { (255, 176, 96) },
            0.75,
        );
    }
    draw_talon_prints(pb, at, pose.f, 0.9, if live { (255, 250, 214) } else { col }, 0.95, true);
    if live {
        ring(pb, at, DIVE_RING_OUTER * (0.6 + 0.4 * pose.progress), hot, 0.5, 1.2);
    }
    // A faint spent mark where the shadow stopped reading and committed: the
    // bird left the sky from *there*.
    if locked {
        let spent = pose.lane_start;
        ring(pb, spent, 9.0, (206, 190, 164), 0.22, 0.8);
        ring(pb, spent, 5.0, (206, 190, 164), 0.16, 0.8);
    }
}

/// Three talon prints in a V, tip toward `f`.
fn draw_talon_prints(
    pb: &mut PixelBuffer,
    at: Vec2,
    f: Vec2,
    scale: f32,
    c: Rgb,
    alpha: f32,
    solid: bool,
) {
    let perp = f.perp();
    // The tip print is ahead; the flank prints are behind and 14 px to each
    // side: the shape the Road Atlas's appointment taught.
    let tip = at + f * (6.0 * scale);
    let left = at - f * (9.0 * scale) - perp * (13.0 * scale);
    let right = at - f * (9.0 * scale) + perp * (13.0 * scale);
    for (i, p) in [left, tip, right].iter().enumerate() {
        // Each print is a claw: two short strokes opening backward.
        let a = 3.2 * scale;
        line_solid(pb, *p, *p + f * a + perp * (a * 0.55), c, alpha);
        line_solid(pb, *p, *p + f * a - perp * (a * 0.55), c, alpha);
        if solid {
            blend_px(
                pb,
                p.x.round() as i32,
                p.y.round() as i32,
                c,
                alpha * (0.9 - 0.12 * i as f32),
            );
        }
    }
}

/// The splinters and dust where the talons went into the oak.
fn draw_buried_dust(pb: &mut PixelBuffer, pose: &Pose) {
    let t = (pose.progress / 0.35).clamp(0.0, 1.0);
    let at = pose.ground;
    ring(pb, at, DIVE_RADIUS * (1.0 + 0.8 * t), (196, 168, 128), 0.35 * (1.0 - t) + 0.10, 1.0);
    for k in 0..9 {
        let a = k as f32 * 0.7 + pose.anim * 0.4;
        let d = Vec2::from_angle(a);
        let q = at + d * (DIVE_RADIUS * (0.7 + 0.9 * t) + (k % 3) as f32 * 2.0);
        let len = 2.0 + (k % 4) as f32 * 1.4;
        line_solid(pb, q, q + d * len, (206, 182, 140), 0.30 * (1.0 - t) + 0.08);
    }
    // The nape: the one bright thing on a bird with its face in the planks.
    // (`nape_pos` in this same frame: the body is drawn over the ground mark.)
    let nape = pose.ground + pose.f * (HEAD_OFFSET - NAPE_BACK);
    let pulse = 0.5 + 0.5 * (pose.anim * 9.0).sin();
    ring(pb, nape, NAPE_RADIUS + 2.0 + pulse, NAPE_GOLD, 0.55, 1.0);
}

/// The wash of dust where the bird sets down out of the sky.
fn draw_stoop_dust(pb: &mut PixelBuffer, pose: &Pose) {
    let t = smoothstep(0.2, 1.0, pose.progress);
    let at = pose.ground;
    ring(pb, at, DOWNDRAFT_OUTER * (0.4 + 0.9 * t), (206, 186, 152), 0.30 * t, 1.0);
}

/// The downdraft rings: three beats a hover, each an expanding ring of dust
/// driven off the planks (push-only, and drawn as such).
fn draw_downdraft(pb: &mut PixelBuffer, pose: &Pose) {
    let p = pose.progress.clamp(0.0, 1.0);
    for k in 0..DOWNDRAFT_BEATS {
        let b0 = k as f32 / DOWNDRAFT_BEATS as f32;
        let t = (p - b0) / DOWNDRAFT_DUTY.max(0.05);
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let r = DOWNDRAFT_INNER + (DOWNDRAFT_OUTER - DOWNDRAFT_INNER) * t;
        ring(pb, pose.ground + SHADOW_THROW * pose.lift, r, (214, 200, 170), 0.34 * (1.0 - t), 1.0);
        // Four flecks of deck dust riding the ring out.
        for j in 0..4 {
            let a = j as f32 * std::f32::consts::FRAC_PI_2 + k as f32 * 0.5;
            let d = Vec2::from_angle(a);
            let q = pose.ground + d * r;
            blend_px(
                pb,
                q.x.round() as i32,
                q.y.round() as i32,
                (196, 178, 148),
                0.30 * (1.0 - t),
            );
        }
    }
}

/// The dart wedge: three converging strokes out of the bill, lit in order as
/// the darts cross.
fn draw_dart_wedge(pb: &mut PixelBuffer, pose: &Pose, view: &HawkView) {
    // The mark is world geometry; the pose carries it in screen space, so the
    // body's lift cannot bend the wedge the rat has to leave.
    let o = pose.lane_start;
    let far = pose.target;
    let d = far - o;
    let len = d.len();
    let f = if len > 0.5 {
        d * (1.0 / len)
    } else {
        unit_facing(view)
    };
    let reach = len.clamp(DART_INNER + 4.0, DART_REACH);
    let half = DART_HALF_ANGLE;
    let live = view.state == HawkState::DartFan;
    let t = pose.progress.clamp(0.0, 1.0);
    let col: Rgb = if live { (255, 244, 190) } else { (255, 206, 104) };
    // The wedge's two edges and its cap of hatch marks.
    for s in [-1.0f32, 1.0] {
        let edge = f.rotate(s * half);
        let end = o + edge * reach;
        line_solid(pb, o + edge * DART_INNER, end, col, if live { 0.9 } else { 0.7 });
    }
    let steps = 9;
    for i in 0..=steps {
        let a = -half + 2.0 * half * (i as f32 / steps as f32);
        let dir = f.rotate(a);
        let inner = o + dir * (reach - 3.0);
        let outer = o + dir * reach;
        line_solid(pb, inner, outer, col, if live { 0.8 } else { 0.45 });
    }
    // Inside the wedge: the darts themselves, three strokes that run out with
    // the fan's own clock.
    let travel = if live { smoothstep(0.0, 1.0, t) } else { 0.0 };
    for k in 0..3 {
        let a = -half * 0.72 + half * 0.72 * k as f32;
        let dir = f.rotate(a);
        let head = o + dir * (DART_INNER + (reach - DART_INNER) * travel);
        let tail = head - dir * 9.0;
        line_solid(pb, tail, head, (255, 250, 210), if live { 0.7 } else { 0.12 });
    }
}

/// The grab reach: a talon's reach corridor with three claw marks at its end.
fn draw_grab_reach(pb: &mut PixelBuffer, pose: &Pose, view: &HawkView) {
    let live = view.state == HawkState::Grab;
    let near = pose.lane_start;
    let far = pose.target;
    let d = far - near;
    let len = d.len();
    let f = if len > 0.5 {
        d * (1.0 / len)
    } else {
        pose.f
    };
    let perp = f.perp();
    let col: Rgb = if live { (255, 240, 190) } else { (255, 198, 108) };
    // Two rails and a cross-hatch: a reach, not a lane.
    line_solid(pb, near + perp * GRAB_HALF_WIDTH, far + perp * GRAB_HALF_WIDTH, col, 0.72);
    line_solid(pb, near - perp * GRAB_HALF_WIDTH, far - perp * GRAB_HALF_WIDTH, col, 0.72);
    let steps = (len / 5.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let q = near + d * (i as f32 / steps as f32);
        let hatch = ((i as f32 * 0.5) + pose.anim * 2.0).sin() > 0.2;
        if hatch {
            line_solid(pb, q - perp * 2.4, q + perp * 2.4, col, 0.30);
        }
    }
    // The closing claws at the far end.
    for s in [-1.0f32, 0.0, 1.0] {
        let tip = far + perp * (s * GRAB_HALF_WIDTH * 0.8);
        line_solid(pb, tip - f * 6.0 + perp * 2.2, tip, col, 0.92);
        line_solid(pb, tip - f * 6.0 - perp * 2.2, tip, col, 0.92);
    }
    if live {
        ring(pb, far, GRAB_DROP_RADIUS + 1.0, (255, 236, 160), 0.55, 1.0);
    }
}

/// The rake's lane: rails down the deck at the rat's row, live on the pass.
fn draw_rake_lane(pb: &mut PixelBuffer, pose: &Pose, view: &HawkView) {
    let live = view.state == HawkState::RakePass;
    let near = pose.lane_start;
    let far = pose.target;
    let d = far - near;
    let len = d.len();
    let f = if len > 0.5 { d * (1.0 / len) } else { pose.f };
    let perp = f.perp();
    let col: Rgb = if live { (255, 132, 94) } else { (255, 200, 104) };
    line_solid(pb, near + perp * RAKE_HALF_WIDTH, far + perp * RAKE_HALF_WIDTH, col, 0.82);
    line_solid(pb, near - perp * RAKE_HALF_WIDTH, far - perp * RAKE_HALF_WIDTH, col, 0.82);
    let steps = (len / 4.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        if ((i as f32 * 0.6) + pose.anim * 4.0).sin() > 0.35 {
            let q = near + d * (i as f32 / steps as f32);
            line_solid(pb, q - perp * 3.2, q + perp * 3.2, col, 0.28);
        }
    }
    // The talon marks in the deck at the far end: what the pass is for.
    draw_talon_prints(pb, far, f, 0.8, col, if live { 0.95 } else { 0.6 }, true);
}

/// The phase-change pose: it is down on the planks and the road dust settles.
fn draw_alight_ring(pb: &mut PixelBuffer, pose: &Pose) {
    let t = smoothstep(0.0, 0.5, pose.progress);
    ring(pb, pose.ground, 16.0 + 10.0 * t, (206, 184, 150), 0.26 * t, 1.0);
}

/// The dust of the last fall.
fn draw_death_dust(pb: &mut PixelBuffer, pose: &Pose) {
    let t = smoothstep(0.30, 0.95, pose.death);
    if t <= 0.0 {
        return;
    }
    ring(
        pb,
        pose.ground,
        DEATH_RING_RADIUS * (0.4 + 0.7 * t),
        (208, 188, 158),
        0.45 * (1.0 - t) + 0.10,
        1.0,
    );
    soft_dot(pb, pose.ground, DEATH_RING_RADIUS * (0.3 + 0.4 * t), (150, 130, 104), 0.22 * (1.0 - t));
}

/// The radial HP arc under the bird (the phase notches at the thresholds).
fn draw_hp_arc(pb: &mut PixelBuffer, pose: &Pose) {
    let r = 18.0;
    let steps = 48;
    let lit = (pose.hp * steps as f32).round() as i32;
    let col = if pose.hp > PHASE2_HP_FRAC {
        (255, 206, 118)
    } else if pose.hp > PHASE3_HP_FRAC {
        (255, 168, 88)
    } else {
        (255, 92, 66)
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
            blend_px(
                pb,
                x,
                y,
                if lead { (255, 252, 236) } else { col },
                if lead { 0.95 } else { 0.74 },
            );
        } else {
            blend_px(pb, x, y, (20, 22, 28), 0.42);
        }
    }
}

// ============================================================================
// Entity layer: the bird
// ============================================================================

/// Is the sprite (or its marks) worth drawing at all?
#[inline]
fn in_view(pb: &PixelBuffer, view: &HawkView, cam: Vec2) -> bool {
    let m = CULL_MARGIN;
    let p = view.pos - cam;
    let t = view.target - cam;
    let l = view.lane_start - cam;
    let inside = |q: Vec2| {
        q.x >= -m && q.x <= pb.w as f32 + m && q.y >= -m && q.y <= pb.h as f32 + m
    };
    inside(p) || inside(t) || inside(l)
}

/// Draw **The Redtail**.
///
/// **Layer**: entity — with the other bodies. The shadow is the ground pass's.
/// While the bird is overhead it is drawn faint and high (`HIGH_LIFT` px above
/// its own shadow), which is exactly the read the fight is about: a shape a rat
/// can track and cannot shoot.
pub fn draw_hawk(pb: &mut PixelBuffer, view: &HawkView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 || !in_view(pb, view, cam) {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let pal = plumage(&pose);
    // Overhead: a silhouette against the dark, not a body. Low: solid.
    let alpha = if view.overhead() {
        0.52 - 0.16 * pose.lift01
    } else {
        1.0
    } * pose.alpha;
    if alpha <= 0.02 {
        return;
    }

    if pose.state == HawkState::Death {
        draw_bird(pb, &pose, &pal, alpha, true);
        draw_death_feathers(pb, &pose, &pal, alpha);
        return;
    }

    draw_bird(pb, &pose, &pal, alpha, false);

    // The nape glint while the talons are buried: the weak point, unmistakable.
    if pose.state == HawkState::TalonBury {
        let nape = pose.ground + pose.f * (HEAD_OFFSET - NAPE_BACK) + Vec2::new(0.0, -1.0);
        let pulse = 0.5 + 0.5 * (pose.anim * 10.0).sin();
        soft_dot(pb, nape, NAPE_RADIUS * 0.9, NAPE_GOLD, 0.30 + 0.18 * pulse);
        blend_px(
            pb,
            nape.x.round() as i32,
            (nape.y - 1.0).round() as i32,
            (255, 252, 226),
            0.85,
        );
    }

    // Talon sparks while the feet are on the deck (a buried dive, a landing).
    if matches!(pose.state, HawkState::TalonBury | HawkState::Alight) {
        let f = pose.f;
        let p = pose.p;
        for s in [-1.0f32, 1.0] {
            let q = pose.ground + f * 8.0 + p * (s * 4.0);
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, pal.foot, 0.75);
        }
    }
}

/// One pass over the bird's own pixels: silhouette, outline, plumage shading,
/// barring, eye and bill.
fn draw_bird(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage, alpha: f32, dead: bool) {
    let reach = 46.0;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, reach) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let (dwl, spanl, chordl) = wing_sdf(pose, -1.0, u, v);
            let (dwr, spanr, chordr) = wing_sdf(pose, 1.0, u, v);
            let d_wing = dwl.min(dwr);
            let (d_tail, tail_along) = tail_sdf(pose, u, v);
            let d_torso = torso_sdf(pose, u, v);
            let d_head = ellipse_sdf(u, v, HEAD_OFFSET, 0.0, 4.6, 4.0);
            let d_beak = sd_taper(
                u,
                v,
                (HEAD_OFFSET + 2.6, 0.0),
                (HEAD_OFFSET + BEAK_LENGTH, 1.1),
                2.0,
                0.75,
            );

            // Nearest layer wins; the torso and head sit over the wings, the
            // wings over the tail (0 torso, 1 wing, 2 tail, 3 head, 4 beak).
            let (mut d, mut layer) = if d_tail < d_wing {
                (d_tail, 2u8)
            } else {
                (d_wing, 1u8)
            };
            if d_torso < d {
                d = d_torso;
                layer = 0;
            }
            if d_head < d {
                d = d_head;
                layer = 3;
            }
            if d_beak < d {
                d = d_beak;
                layer = 4;
            }
            if d >= 1.0 {
                continue;
            }
            if d > 0.0 {
                // Feather-edge softness.
                blend_px(pb, x, y, pal.outline, alpha * (0.9 - d * 0.8));
                continue;
            }
            if d > -1.15 {
                blend_px(pb, x, y, pal.outline, alpha);
                continue;
            }
            // Surface normal, roughly, for the key light: the bird's own
            // cross-axis lean, plus a little shading off the spine.
            let nv = (v / 10.0).clamp(-1.0, 1.0);
            let lum = pose.lit(-0.22, nv);
            let c = match layer {
                0 => {
                    // Torso: dark mantle over a barred cream chest, split down
                    // the middle of the back.
                    let belly = (u / 9.0).clamp(-1.0, 1.0);
                    let base = if belly > 0.15 {
                        mix(pal.cream_sh, pal.cream, (nv.abs() * -0.6 + 0.7).clamp(0.0, 1.0))
                    } else {
                        mix(pal.mantle, pal.mantle_hi, 0.5 + 0.5 * lum)
                    };
                    if belly > 0.2 {
                        // Chest bars.
                        let band = ((u * 1.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                        mix(base, pal.bar, band * 0.35)
                    } else {
                        base
                    }
                }
                1 => {
                    // Wings: broad secondaries, pale leading edge, barred tips.
                    let span = if dwl <= dwr { spanl } else { spanr };
                    let chord = if dwl <= dwr { chordl } else { chordr };
                    let (lo, hi) = if span < 0.35 {
                        (pal.mantle, pal.mantle_hi)
                    } else if span < 0.85 {
                        (pal.cream_sh, pal.cream)
                    } else {
                        (pal.bar, pal.cream_sh)
                    };
                    let mut c = shade3(pal.shadow, lo, hi, lum);
                    // Barring across the primaries.
                    if span > 0.72 {
                        let band = ((span * 26.0 - pose.anim * 0.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                        c = mix(c, pal.bar, band * 0.45);
                    }
                    // The shoulder patch that gives a redtail its name.
                    if span < 0.22 && chord > 0.2 && chord < 0.8 {
                        c = mix(c, pal.tail, 0.35 + 0.3 * pose.enraged);
                    }
                    c
                }
                2 => {
                    // Tail: rufous, with the dark terminal band.
                    let mut c = shade3(pal.shadow, pal.tail, pal.tail_hi, lum);
                    if tail_along > 0.74 {
                        c = mix(c, pal.tail_band, 0.75);
                    } else if tail_along > 0.60 {
                        c = mix(c, pal.tail_band, 0.30);
                    }
                    // Fine barring between the bands.
                    if tail_along > 0.2 && tail_along < 0.6 {
                        let band = ((tail_along * 40.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                        c = mix(c, pal.bar, band * 0.16);
                    }
                    c
                }
                3 => {
                    // Head: the dark hood and the pale throat.
                    let throat = (v / 4.0).clamp(-1.0, 1.0);
                    if throat > 0.35 {
                        shade3(pal.cream_sh, pal.cream, pal.cream, lum)
                    } else {
                        shade3(pal.shadow, pal.mantle, pal.mantle_hi, lum * 1.2)
                    }
                }
                _ => shade3(pal.shadow, pal.cere, pal.foot, lum),
            };
            let mut c = c;
            // Hackles up: a little more red in the mantle, phase 2+.
            if pose.enraged > 0.0 && layer == 0 {
                c = mix(c, pal.tail, 0.10 * pose.enraged);
            }
            if dead {
                c = mix(c, (96, 88, 78), (pose.death * 0.45).min(0.6));
            }
            c = apply_hit(c, pose.hit);
            blend_px(pb, x, y, c, alpha);
        }
    }

    // The eye: a raptor's one bright point, forward of the hood.
    let eye = pose.place(HEAD_OFFSET + 1.4, -2.4);
    blend_px(
        pb,
        eye.x.round() as i32,
        eye.y.round() as i32,
        apply_hit(EYE, pose.hit),
        alpha,
    );
    blend_px(
        pb,
        eye.x.round() as i32,
        (eye.y + 1.0).round() as i32,
        apply_hit((122, 84, 24), pose.hit),
        alpha * 0.8,
    );
    // The hooked bill tip.
    let tip = pose.place(HEAD_OFFSET + BEAK_LENGTH - 0.5, 1.2);
    blend_px(
        pb,
        tip.x.round() as i32,
        tip.y.round() as i32,
        apply_hit(pal.beak, pose.hit),
        alpha,
    );
    // The critical shimmer rides the nape in phase 3.
    if pose.critical > 0.0 {
        let nape = pose.place(HEAD_OFFSET - NAPE_BACK, 0.0);
        soft_dot(
            pb,
            nape,
            3.0,
            (255, 226, 150),
            0.22 * pose.critical * alpha,
        );
    }
}

/// The last fall: feathers off the wing cases, settling.
fn draw_death_feathers(pb: &mut PixelBuffer, pose: &Pose, pal: &Plumage, alpha: f32) {
    let t = smoothstep(0.1, 0.75, pose.death);
    if t <= 0.0 {
        return;
    }
    for k in 0..7 {
        let a = k as f32 * 0.9 + 0.4;
        let d = Vec2::from_angle(a);
        let q = pose.body + d * (12.0 + 22.0 * t) + Vec2::new(0.0, 4.0 * t);
        let c = if k % 2 == 0 { pal.tail } else { pal.cream };
        soft_dot(pb, q, 1.6, c, 0.5 * (1.0 - t * 0.5) * alpha);
    }
}

// ============================================================================
// Post-lighting tell overlay
// ============================================================================

/// Re-stamp the marks the rat has to *step out of* after the lighting pass, so
/// a dark deck can never dim the shape that matters: the locked reticle, the
/// live rake lane and the nape.
pub fn draw_tell_overlay(pb: &mut PixelBuffer, view: &HawkView, cam: Vec2) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, (0, 0, 0));
    match view.state {
        HawkState::Dive => {
            let locked = pose.progress >= DIVE_LOCK_PROGRESS;
            if !locked {
                return;
            }
            let live = pose.progress >= DIVE_IMPACT_PROGRESS - 0.06;
            let col: Rgb = if live { (255, 248, 210) } else { (255, 210, 120) };
            ring(pb, pose.target, DIVE_RADIUS, col, 0.85, 1.0);
            draw_talon_prints(pb, pose.target, pose.f, 0.95, col, 0.95, true);
            ring(pb, pose.target, DIVE_RADIUS * 0.55, col, 0.45, 0.8);
        }
        HawkState::TalonBury => {
            let nape = pose.ground + pose.f * (HEAD_OFFSET - NAPE_BACK);
            ring(pb, nape, NAPE_RADIUS + 1.0, NAPE_GOLD, 0.70, 1.0);
            soft_dot(pb, nape, NAPE_RADIUS * 0.8, NAPE_GOLD, 0.18);
        }
        HawkState::RakeWindup | HawkState::RakePass => {
            let live = view.state == HawkState::RakePass;
            let col: Rgb = if live { (255, 148, 104) } else { (255, 208, 118) };
            let near = pose.lane_start;
            let far = pose.target;
            let d = far - near;
            let len = d.len();
            let f = if len > 0.5 { d * (1.0 / len) } else { pose.f };
            let perp = f.perp();
            line_solid(pb, near + perp * RAKE_HALF_WIDTH, far + perp * RAKE_HALF_WIDTH, col, 0.9);
            line_solid(pb, near - perp * RAKE_HALF_WIDTH, far - perp * RAKE_HALF_WIDTH, col, 0.9);
        }
        HawkState::GrabWindup | HawkState::Grab => {
            let col: Rgb = if view.state == HawkState::Grab {
                (255, 244, 200)
            } else {
                (255, 206, 112)
            };
            ring(pb, pose.target, GRAB_DROP_RADIUS, col, 0.7, 1.0);
        }
        _ => {}
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: HawkState) -> HawkView {
        let mut v = HawkView::new(
            Vec2::new(60.0, 60.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(96.0, 64.0),
        );
        v.state = state;
        v.lane_start = Vec2::new(60.0, 60.0);
        v.decoy = Vec2::new(96.0, 40.0);
        v.anim = 0.8;
        v
    }

    #[test]
    fn phases_are_the_art_thresholds() {
        const _: () = assert!(PHASE3_HP_FRAC < PHASE2_HP_FRAC);
        const _: () = assert!(CRITICAL_HP_FRAC < PHASE3_HP_FRAC);
        const _: () = assert!(DIVE_LOCK_PROGRESS < DIVE_IMPACT_PROGRESS);
        const _: () = assert!(DART_LOCK_PROGRESS < 1.0);
        const _: () = assert!(GRAB_LOCK_PROGRESS < 1.0);
    }

    /// The overhead states have **no** hit shapes at all, which is what the
    /// plate's [`OVERHEAD_TAG`] promises; every other state is reachable.
    #[test]
    fn overhead_states_are_hitless_and_everything_else_is_grounded() {
        for state in [
            HawkState::Circle,
            HawkState::Dive,
            HawkState::Wheel,
        ] {
            assert!(is_overhead(state), "{state:?} is out of reach");
            assert!(!is_grounded(state));
            assert!(!has_body_contact(state), "{state:?} cannot bump a rat");
        }
        for state in [
            HawkState::TalonBury,
            HawkState::Stoop,
            HawkState::Hover,
            HawkState::DartWindup,
            HawkState::DartFan,
            HawkState::GrabWindup,
            HawkState::Grab,
            HawkState::RakeWindup,
            HawkState::RakePass,
            HawkState::Alight,
            HawkState::Climb,
        ] {
            assert!(is_grounded(state), "{state:?} is down where the rat can reach");
        }
        // The recovery windows deal no contact (the owner's rule: contact 8
        // *outside* recovery).
        for state in [HawkState::TalonBury, HawkState::Alight] {
            assert!(!has_body_contact(state), "{state:?} is a recovery pose");
        }
        // The two hover attacks and the hover itself all bump.
        for state in [
            HawkState::Hover,
            HawkState::DartFan,
            HawkState::Grab,
            HawkState::RakePass,
        ] {
            assert!(has_body_contact(state));
        }
        assert_eq!(incoming_damage_mult(HawkState::TalonBury), TALON_BURY_DAMAGE_MULT);
        assert_eq!(incoming_damage_mult(HawkState::Death), 0.0);
        assert_eq!(incoming_damage_mult(HawkState::Hover), 1.0);
    }

    /// The altitude read: high while circling, on the deck while buried, and a
    /// wounded bird hovers lower.
    #[test]
    fn the_lift_is_the_altitude_read() {
        assert_eq!(altitude(HawkState::Circle, 1, 0.5), HIGH_LIFT);
        assert_eq!(altitude(HawkState::TalonBury, 1, 0.5), 0.0);
        assert_eq!(altitude(HawkState::Hover, 1, 0.5), hover_altitude(1));
        assert!(hover_altitude(3) < hover_altitude(2));
        assert!(hover_altitude(2) < hover_altitude(1));
        // A dive loses its lift through the fall.
        let early = altitude(HawkState::Dive, 1, DIVE_LOCK_PROGRESS + 0.01);
        let late = altitude(HawkState::Dive, 1, DIVE_IMPACT_PROGRESS - 0.01);
        assert!(late < early, "the stoop must come down: {early} -> {late}");
        assert!(late < HIGH_LIFT * 0.2);
        // A stoop ends where the hover starts.
        let landed = altitude(HawkState::Stoop, 2, 1.0);
        assert!((landed - hover_altitude(2)).abs() < 0.01);
    }

    #[test]
    fn hit_circles_cover_the_body_and_the_nape_is_the_punish() {
        let mut v = view(HawkState::Hover);
        let circles = body_hit_circles(&v);
        assert!((circles[1].0 - v.pos).len() < 1.0, "the body circle sits on the bird");
        assert!(
            (circles[2].0 - (v.pos + Vec2::new(HEAD_OFFSET, 0.0))).len() < 0.5,
            "the head circle sits on the head"
        );
        assert!(weak_point(&v).is_none(), "a hover has no exposed nape");

        // Buried: the first circle *is* the nape, so it is the first auto-aim
        // point and the punish window's target.
        v.state = HawkState::TalonBury;
        let (nape, r) = weak_point(&v).expect("a buried bird shows its nape");
        assert_eq!(nape, nape_pos(&v));
        assert!((r - NAPE_RADIUS).abs() < 1e-6);
        let circles = body_hit_circles(&v);
        assert_eq!(circles[0].0, nape);
        // The nape is behind the head and in front of the tail root.
        assert!(nape.dist(v.pos) < HEAD_OFFSET);
        assert!(nape.x > v.pos.x);
    }

    /// The sprite must sit over the shapes the rat hits, in every state where it
    /// can be hit at all — the magpie lesson (a bird drawn a dozen pixels above
    /// its circles is a bird the rat shoots the planks under).
    #[test]
    fn the_low_bird_is_drawn_over_its_hit_shapes() {
        for state in [
            HawkState::TalonBury,
            HawkState::Stoop,
            HawkState::Hover,
            HawkState::DartWindup,
            HawkState::DartFan,
            HawkState::GrabWindup,
            HawkState::Grab,
            HawkState::RakeWindup,
            HawkState::RakePass,
            HawkState::Alight,
            HawkState::Death,
        ] {
            for prog in [0.2f32, 0.6, 0.95] {
                let mut v = view(state);
                v.progress = prog;
                let pose = Pose::from_view(&v, Vec2::ZERO, (206, 150, 70));
                let reach = body_hit_circles(&v)
                    .iter()
                    .map(|(c, r)| c.dist(pose.body) - r)
                    .fold(f32::MAX, f32::min);
                assert!(
                    reach <= 6.0,
                    "{state:?} at {prog}: the body is drawn {reach:.1}px off every hit circle"
                );
                // …and the shadow is under it, never above it.
                assert!(pose.body.y <= pose.ground.y + 0.01);
            }
        }
    }

    /// The shadow is drawn in **every** state (it is the boss's readable
    /// position while the bird is out of reach).
    #[test]
    fn the_shadow_is_drawn_in_every_state() {
        for state in [
            HawkState::Circle,
            HawkState::Dive,
            HawkState::TalonBury,
            HawkState::Wheel,
            HawkState::Stoop,
            HawkState::Hover,
            HawkState::DartWindup,
            HawkState::DartFan,
            HawkState::GrabWindup,
            HawkState::Grab,
            HawkState::RakeWindup,
            HawkState::RakePass,
            HawkState::Alight,
            HawkState::Climb,
        ] {
            let v = view(state);
            let mut pb = PixelBuffer::new(200, 160);
            // A bright deck so a faint shadow still shows.
            for y in 0..160 {
                for x in 0..200 {
                    pb.plot(x, y, (196, 172, 132));
                }
            }
            // The shadow is thrown by the bird's altitude, so sample its centre
            // rather than the bird's ground point.
            let centre = v.pos + SHADOW_THROW * altitude(v.state, v.phase, v.progress);
            let (px, py) = (centre.x.round().max(0.0) as usize, centre.y.round().max(0.0) as usize);
            let before = pb.pixel_at(px, py);
            draw_hawk_ground(&mut pb, &v, Vec2::ZERO, (206, 150, 70));
            let after = pb.pixel_at(px, py);
            assert!(
                after.0 < before.0 && after.1 < before.1,
                "{state:?}: no shadow on the deck ({before:?} -> {after:?})"
            );
        }
    }

    /// The higher the bird, the fainter and smaller its shadow: the gradient is
    /// the altitude read-out.
    #[test]
    fn the_shadow_grows_and_darkens_as_the_bird_comes_down() {
        let mut high = view(HawkState::Circle);
        let mut low = view(HawkState::Hover);
        high.pos = Vec2::new(100.0, 80.0);
        low.pos = Vec2::new(100.0, 80.0);
        let mut a = PixelBuffer::new(200, 160);
        let mut b = PixelBuffer::new(200, 160);
        for y in 0..160 {
            for x in 0..200 {
                a.plot(x, y, (200, 176, 136));
                b.plot(x, y, (200, 176, 136));
            }
        }
        draw_hawk_ground(&mut a, &high, Vec2::ZERO, (206, 150, 70));
        draw_hawk_ground(&mut b, &low, Vec2::ZERO, (206, 150, 70));
        let dark_a = (0..160)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|&(x, y)| a.pixel_at(x, y).0 < 190)
            .count();
        let dark_b = (0..160)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|&(x, y)| b.pixel_at(x, y).0 < 190)
            .count();
        assert!(
            dark_b > dark_a,
            "the low bird's shadow must be the bigger, darker one ({dark_a} vs {dark_b})"
        );
    }

    #[test]
    fn the_marks_are_the_controllers_geometry() {
        // The reticle is the impact circle.
        let mut v = view(HawkState::Dive);
        v.progress = DIVE_LOCK_PROGRESS + 0.1;
        let (at, r) = impact_circle(&v);
        assert_eq!(at, v.target);
        assert_eq!(r, DIVE_RADIUS);

        // The wedge starts at the mark origin and stops at the mark.
        let mut v = view(HawkState::DartWindup);
        v.lane_start = Vec2::new(60.0, 60.0);
        v.target = Vec2::new(150.0, 60.0);
        let (origin, reach, facing, half) = dart_wedge(&v);
        assert_eq!(origin, v.lane_start);
        assert!((reach - 90.0).abs() < 1e-3);
        assert!((facing - Vec2::new(1.0, 0.0)).len() < 1e-4);
        assert!((half - DART_HALF_ANGLE).abs() < 1e-6);

        // The grab reach and the rake lane are the two marks, unchanged.
        let v = view(HawkState::Grab);
        let (near, far, w) = grab_reach(&v);
        assert_eq!((near, far), (v.lane_start, v.target));
        assert_eq!(w, GRAB_HALF_WIDTH);
        let v = view(HawkState::RakePass);
        assert_eq!(rake_lane(&v), (v.lane_start, v.target));
        assert_eq!(drop_point(&view(HawkState::Grab)), view(HawkState::Grab).target);
    }

    #[test]
    fn the_downdraft_beats_are_the_hover_s_own_clock() {
        assert!(downdraft_active(HawkState::Hover, 0.02));
        assert!(!downdraft_active(HawkState::Dive, 0.02));
        // Due at the top of each beat of a 1.2 s hover.
        assert!(downdraft_due(0.0, 1.2, 0));
        assert!(!downdraft_due(0.3, 1.2, 1));
        assert!(downdraft_due(0.41, 1.2, 1));
        assert!(!downdraft_due(10.0, 1.2, DOWNDRAFT_MAX));
    }

    #[test]
    fn the_feint_mark_only_means_something_while_the_shadow_jinks() {
        let mut v = view(HawkState::Dive);
        v.progress = DIVE_FEINT_PROGRESS - 0.01;
        assert!(!v.feint(), "the jink has not started");
        v.progress = DIVE_FEINT_PROGRESS + 0.05;
        assert!(v.feint(), "the decoy is showing");
        v.progress = DIVE_LOCK_PROGRESS;
        assert!(!v.feint(), "the real lock is in");
    }

    /// Development aid: `RATRUN_HAWK_SHEET=1 cargo test hawk_sheet` writes a
    /// contact sheet of every state of the fight (three progresses each) into
    /// the wave-3 scratch directory. Inert unless the variable is set.
    #[test]
    fn hawk_sheet() {
        if std::env::var("RATRUN_HAWK_SHEET").is_err() {
            return;
        }
        const CELL_W: usize = 190;
        const CELL_H: usize = 150;
        let states = [
            HawkState::Circle,
            HawkState::Dive,
            HawkState::TalonBury,
            HawkState::Wheel,
            HawkState::Stoop,
            HawkState::Hover,
            HawkState::DartWindup,
            HawkState::DartFan,
            HawkState::GrabWindup,
            HawkState::Grab,
            HawkState::RakeWindup,
            HawkState::RakePass,
            HawkState::Alight,
            HawkState::Climb,
            HawkState::Death,
        ];
        let cols = 3usize;
        let mut sheet = PixelBuffer::new(CELL_W * cols, CELL_H * states.len());
        for (i, state) in states.iter().enumerate() {
            for (j, prog) in [0.2f32, 0.6, 0.95].iter().enumerate() {
                let mut cell = PixelBuffer::new(CELL_W, CELL_H);
                // The flatbed's oak, so the ground marks read.
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        let plank = (y / 11) % 2 == 0;
                        let grain = hash2((x / 3) as i32, (y / 11) as i32, 9);
                        let base = if plank { (86, 62, 40) } else { (74, 52, 34) };
                        cell.plot(x as i32, y as i32, mix(base, (108, 82, 52), grain * 0.4));
                    }
                }
                let mut v = HawkView::new(
                    Vec2::new(88.0, 96.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(120.0, 70.0),
                );
                v.state = *state;
                v.progress = *prog;
                v.anim = 0.9 + prog * 4.0;
                v.hp_frac = match state {
                    HawkState::Dive => 0.5,
                    HawkState::TalonBury => 0.34,
                    HawkState::Death => 0.0,
                    _ => 1.0,
                };
                v.lane_start = Vec2::new(88.0, 96.0);
                v.decoy = Vec2::new(120.0, 52.0);
                draw_hawk_ground(&mut cell, &v, Vec2::ZERO, (206, 150, 70));
                draw_hawk(&mut cell, &v, Vec2::ZERO, (206, 150, 70));
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
        write_ppm(&scratch("hawk_sheet.ppm"), &sheet);
    }

    /// Development aid: `RATRUN_HAWK_WORLD=1 cargo test hawk_world` renders the
    /// real flatbed through `World::draw` with the bird in eight poses, so the
    /// integration (deck, lighting, camera, tells) can be looked at and not just
    /// the sprite. Inert unless the variable is set.
    #[test]
    fn hawk_world() {
        if std::env::var("RATRUN_HAWK_WORLD").is_err() {
            return;
        }
        use crate::game::ecs::components::SpawnId;
        use crate::game::ecs::systems::boss::{Boss, BossKind, BossState};
        use crate::game::level::{Theme, TileRect};
        use crate::game::world::World;

        const VIEW: (usize, usize) = (160, 168);
        let mut sheet = PixelBuffer::new(VIEW.0 * 2, VIEW.1 * 4);
        let poses = [
            (HawkState::Circle, 0.5f32, 1.0f32),
            (HawkState::Dive, 0.70, 1.0),
            (HawkState::TalonBury, 0.40, 1.0),
            (HawkState::Hover, 0.30, 1.0),
            (HawkState::DartFan, 0.40, 1.0),
            (HawkState::Grab, 0.50, 1.0),
            (HawkState::RakePass, 0.45, 0.34),
            (HawkState::Death, 0.60, 0.0),
        ];
        for (i, (state, prog, hp_frac)) in poses.iter().enumerate() {
            let mut w = World::new(Theme::Trailer, 11);
            w.level_index = crate::game::campaign::LEVEL_COUNT;
            w.lighting = crate::render::lighting::LightingMode::Full;
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .expect("the flatbed's cab hold");
            let centre = arena.center_world();
            w.player.pos = centre - Vec2::new(34.0, 0.0);
            // One draw settles the camera before the bird is placed.
            let mut warm = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut warm);
            let mut boss = Boss::new(
                BossKind::Hawk,
                centre + Vec2::new(16.0, -8.0),
                1600.0,
                TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows()),
                centre,
                false,
            );
            boss.facing = Vec2::new(1.0, 0.0);
            boss.target = centre + Vec2::new(38.0, 10.0);
            boss.pounce_origin = centre + Vec2::new(-40.0, -8.0);
            boss.lane_start = boss.pounce_origin;
            boss.hp = 1600.0 * hp_frac;
            boss.set_state(BossState::Hawk(*state), 1.4);
            boss.elapsed = prog * 1.4;
            w.ecs.spawn((boss, SpawnId(9002)));
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut pb);
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
        write_ppm(&scratch("hawk_world.ppm"), &sheet);
    }

    /// The wave-3 scratch directory this worker was given, created on demand.
    fn scratch(name: &str) -> String {
        let dir = "/private/tmp/claude-501/-Users-joe-dev-ratrun/a0cccbef-9e45-4849-814a-e727185739a1/scratchpad/w3/boss-hawk";
        let _ = std::fs::create_dir_all(dir);
        format!("{dir}/{name}")
    }

    fn write_ppm(path: &str, pb: &PixelBuffer) {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for c in pb.pixels() {
            out.push(c.0);
            out.push(c.1);
            out.push(c.2);
        }
        match std::fs::write(path, out) {
            Ok(()) => println!("wrote {path}"),
            // A capture that cannot write is not a test failure: the suite must
            // pass on a machine with no scratch directory.
            Err(e) => eprintln!("could not write {path}: {e}"),
        }
    }
}
