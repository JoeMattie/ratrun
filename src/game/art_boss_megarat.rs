//! Art and gameplay specification for the Kitchen's second boss —
//! **SUBJECT R-01**, the Megarat.
//!
//! The facility made one rat before it made you. It is still in the kitchens,
//! four hundred pounds of it, collared and tagged and *proud*: the first
//! success. It fights with your kit, because the kit was copied off it — the
//! gnaw, the tail, the squeak, the dash — but every one of those moves is
//! bigger, slower and telegraphed, and every one of them leaves it open.
//!
//! Read from a rat's height it is a wall of matted fur with a mouth at one end
//! and a whip at the other, a steel collar worn like a medal and one ear torn
//! half off. It grooms itself between attacks. It sniffs the air where you are
//! standing. It chitters like it is laughing.
//!
//! # The fight the level was built for
//!
//! | phase | hp | rotation | read |
//! |---|---|---|---|
//! | 1 | `> 0.62` | `Groom` → `GnawWindup` → `GnawLunge` → `GnawStuck` (opening: two gnaws, then a thrash) | a teeth line on the tile: step off it, then bite the reared-up belly |
//! | 2 | `0.34..=0.62` | `+ Call` at the threshold, `+ DashWindup/DashPass`; **two moves per grooming** | the arc, the lane, the ring — and one more move straight after it |
//! | 3 | `<= 0.34` | all four, `+ Call` at the threshold, tighter timings | everything at once, but every window is still a window |
//!
//! ## The gnaw lunge (the mirror of Gnaw)
//!
//! `GnawWindup` paints a **teeth line** from the muzzle to a marked bite point
//! and freezes it at [`GNAW_LOCK_PROGRESS`], exactly like the cat's landing
//! ring: whatever it locked is where the incisors go. `GnawLunge` drives the
//! whole body down the line and the line is live for
//! [`GNAW_ACTIVE_HALF_WIDTH`] either side for [`GNAW_ACTIVE`] s.
//!
//! Then the teeth are in the floor. `GnawStuck` is the punish window: the
//! rat's own Gnaw does this to everything else in the building, and R-01 does
//! it to a cabinet. It **rears up**, which is the whole point — the belly is a
//! separate, bright, soft hit shape ([`weak_point`]), the body deals no
//! contact, and everything landing on it counts ×[`GNAW_STUCK_DAMAGE_MULT`].
//! A lunge that ended against a wall or a cabinet is the [`GNAW_STUCK_WALLED`]
//! window: fighting R-01 in a corner is how you get a free combo.
//!
//! ## The tail thrash (the mirror of Tail Thrash)
//!
//! `ThrashWindup` paints a **huge arc** ([`THRASH_HALF_ARC`] either side of
//! the facing, out to [`THRASH_OUTER`] px) split into two halves. The tail
//! sweeps the rat's own half first ([`MegaRatState::ThrashPass1`]), then there
//! is a **gap** ([`THRASH_GAP`], no live hazard at all), then it comes back
//! through the other half ([`MegaRatState::ThrashPass2`]). The counter is the
//! one the player's own thrash teaches: step *into* the half that has already
//! been swept, during the gap.
//!
//! The swing costs it: `ThrashRecover` drags the tail on the floor and the
//! **tail base** is the weak point (behind it, ×[`THRASH_RECOVER_MULT`]).
//!
//! ## The sonic squeak (the mirror of Sonic Squeak)
//!
//! `SqueakWindup` rears the head back and paints a **ring** round its body:
//! [`SQUEAK_INNER`]..[`SQUEAK_OUTER`] px, live for [`SQUEAK_ACTIVE`] s. The
//! ring has exactly one gap — a [`SQUEAK_WEDGE`] half-angle wedge **behind**
//! it, drawn with a chevron and a "safe" chevron wash — and it is refused when
//! that wedge is wall (the boss will not scream a rat into a corner it cannot
//! read). Standing *inside* the inner radius is also safe: the shout leaves
//! its own feet, exactly like the rat's.
//!
//! ## The dash (the mirror of the dash)
//!
//! `DashWindup` paints a **lane** ([`DASH_LANE_HALF`] px either side) from the
//! body through the rat to the lane's end; `DashPass` runs it at
//! [`DASH_SPEED`] with afterimages, and `DashSkid` is the stop. The dash has a
//! **cooldown the player can read**: [`DASH_COOLDOWN`] seconds, drawn as a
//! gauge of four pips under the boss (dim while cooling, white when ready), the
//! same grammar as the HUD's own dash gauge.
//!
//! ## The call
//!
//! Every phase threshold is a [`CALL_DURATION`] s **harmless pose change**: it
//! stands up, throws its head back and chitters for the room. At
//! [`CALL_SUMMON_PROGRESS`] the walls answer — [`CALL_RATS`] Skitterers come
//! out of the tilework, called by the first success, to hold the rat still
//! while it recovers.
//!
//! # The boss's own shapes (binding on the controller)
//!
//! * [`body_hit_circles`] — tail root, haunch, head. All grounded: it never
//!   leaves the floor, so a ground-only build can always fight it.
//! * [`weak_point`] — `Some` only in the two recovery windows: the **belly**
//!   (in front, under the chest) while it rears out of a stuck gnaw, the
//!   **tail base** (behind) while it drags the tail. This is the one clear weak
//!   point per window, and `Boss::hit_shapes` adds it as a fourth shape.
//! * [`incoming_damage_mult`] — [`GNAW_STUCK_DAMAGE_MULT`] in `GnawStuck`,
//!   [`THRASH_RECOVER_MULT`] in `ThrashRecover`, `0.0` in `Death`.
//! * [`has_body_contact`] — 8 everywhere except its own attack windows, the
//!   two recoveries, the harmless call and the death beat.
//!
//! # Layering contract for the renderer
//!
//! 1. **Ground layer** (after the floor stains, before props/entities):
//!    [`draw_megarat_ground`] — teeth line, thrash arc (marked, live, gap),
//!    squeak ring + safe wedge, dash lane and afterimages, the call's scurry
//!    marks, the dash gauge, the drop shadow and the death pool.
//! 2. **Entity layer** (with the other bodies): [`draw_megarat`] — tail,
//!    haunches, paws, body, head, incisors, collar and tag, scars, the belly /
//!    tail-base weak-point glint and the death sprawl.
//!
//! Everything is stack-only and culls on the sprite margin, so the draw cost is
//! the same order as the other bosses.

#![allow(dead_code)]

use std::f32::consts::PI;
use std::f32::consts::TAU;

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Collision & gameplay constants (art <-> controller <-> framework contract)
// ============================================================================

/// Framework movement/contact circle (world px). Bigger than the Jaguar's
/// 13.5: R-01 is the biggest body the rat fights, and the arena is sized for
/// it (a Large-class body is 27 px across at the cat's radius; 30 is the
/// widest that still walks the Kitchen's boss room's doorways).
pub const BODY_RADIUS: f32 = 15.0;

/// Muzzle to tail tip, hunched (the length figure).
pub const BODY_LENGTH: f32 = 58.0;

/// Longest sprite reach (the tail across a thrash) for culling.
pub const CULL_MARGIN: f32 = 96.0;

/// Haunch centre, behind the body centre along the facing.
pub const HAUNCH_OFFSET: f32 = -6.0;
/// Haunch hit radius.
pub const HAUNCH_RADIUS: f32 = 12.0;
/// Head centre, forward along the facing.
pub const HEAD_OFFSET: f32 = 15.0;
/// Head hit radius.
pub const HEAD_RADIUS: f32 = 8.5;
/// Tail root, behind the haunch.
pub const TAIL_OFFSET: f32 = 21.0;
/// Tail-root hit radius (also the tail-base weak point's radius).
pub const TAIL_RADIUS: f32 = 7.5;
/// How far forward of the body centre the exposed belly sits while it rears.
pub const BELLY_OFFSET: f32 = 10.0;
/// The belly weak point's radius (generous: it is the whole reward).
pub const BELLY_RADIUS: f32 = 9.5;
/// How far behind the body centre the exposed tail base sits.
pub const TAIL_BASE_OFFSET: f32 = 19.0;
/// The tail base's radius while exposed.
pub const TAIL_BASE_RADIUS: f32 = 8.5;

/// The art's recommended base pool, before `BossKind::hp_scale` and the
/// director's boss curve. Sized like the other stage-2 fights (the Sweeper and
/// the Mangle are both 1600 here) because the *scale* carries the beat.
pub const BOSS_BASE_HP: f32 = 1600.0;

/// Body contact outside a recovery window (the owner's rule: 8).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 opens here.
pub const PHASE2_HP_FRAC: f32 = 0.62;
/// Phase 3 opens here. Lower than the Magpie's 0.30 because this fight spends
/// a third of every rotation in a punish window: the last phase has to be a
/// real one.
pub const PHASE3_HP_FRAC: f32 = 0.34;
/// Critical shimmer (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

// ---- the neutral beat (grooming) -----------------------------------------

/// `Groom`: how long it cleans itself between moves.
pub const GROOM_DURATION: [f32; 3] = [1.10, 0.95, 0.80];

/// Walk speed while grooming (`46 / 56 / 66` px/s — a heavy body).
pub const GROOM_SPEED: [f32; 3] = [46.0, 56.0, 66.0];

/// The band `Groom` holds the rat at, in px (`near`, `far`): close enough to
/// lunge from, far enough that the rat can see the whole animal.
pub const GROOM_BAND: [f32; 2] = [30.0, 92.0];

/// Extra speed while hunting a rat that has left the arena.
pub const HUNT_SPEED_MULT: f32 = 1.7;

// (The pause between a phase-change `Call` and the fight resuming is inside
// `Call` itself; nothing extra is needed here.)

// ---- the gnaw lunge -------------------------------------------------------

/// The lunge's telegraph (`GnawWindup`).
pub const GNAW_WINDUP: [f32; 3] = [0.70, 0.62, 0.55];

/// The teeth line freezes here so the rat can step out of it.
pub const GNAW_LOCK_PROGRESS: f32 = 0.62;

/// The line's corridor half-width (the two incisors plus a whisker).
pub const GNAW_ACTIVE_HALF_WIDTH: f32 = 7.0;

/// The strike's live window (single hit: the hazard fires once).
pub const GNAW_ACTIVE: f32 = 0.16;

/// The blade of the strike (`GnawLunge`): the body drives down the line.
pub const GNAW_LUNGE: f32 = 0.26;

/// Furthest it will commit a lunge to (muzzle to bite point).
pub const GNAW_REACH: f32 = 104.0;

/// Nearest (a lunge at contact range is a body slam, not a bite).
pub const GNAW_MIN: f32 = 22.0;

/// Bite damage (before armour).
pub const GNAW_DAMAGE: f32 = 22.0;

/// The charge's speed down the line (px/s).
pub const GNAW_LUNGE_SPEED: f32 = 340.0;

/// How far it skids past the bite point when the rat was not there.
pub const GNAW_OVERSHOOT: f32 = 16.0;

/// The reared-up punish window (`GnawStuck`).
pub const GNAW_STUCK: [f32; 3] = [1.10, 1.00, 0.90];

/// The longer window when a wall or a cabinet took the teeth: it has to brace
/// on its hind legs and pull, which takes half again as long.
pub const GNAW_STUCK_WALLED: f32 = 1.55;

/// Incoming damage on the exposed belly.
pub const GNAW_STUCK_DAMAGE_MULT: f32 = 1.35;

/// How far in front of its neck it looks for the solid that eats its teeth.
pub const GNAW_WALL_PROBE: f32 = 10.0;

/// A step the rat needs off the teeth line before the lunge is allowed.
pub const GNAW_ESCAPE: f32 = 26.0;

// ---- the tail thrash ------------------------------------------------------

/// The arc is painted for this long (`ThrashWindup`).
pub const THRASH_WINDUP: [f32; 3] = [0.85, 0.75, 0.66];

/// Half-angle of the whole marked arc, either side of the facing.
pub const THRASH_HALF_ARC: f32 = 1.30;

/// The arc's inner edge: right at its feet, so there is no hugging it.
pub const THRASH_INNER: f32 = 16.0;

/// The arc's outer reach.
pub const THRASH_OUTER: f32 = 78.0;

/// One pass of the tail across its half of the arc.
pub const THRASH_PASS: f32 = 0.40;

/// The gap between the two passes: the window the rat dashes through.
pub const THRASH_GAP: f32 = 0.34;

/// What a clipped pass costs (each pass hits at most once).
pub const THRASH_DAMAGE: f32 = 18.0;

/// Tail knockback.
pub const THRASH_KNOCKBACK: f32 = 240.0;

/// The dragging-tail recovery (`ThrashRecover`).
pub const THRASH_RECOVER: [f32; 3] = [0.90, 0.80, 0.72];

/// Incoming damage on the exposed tail base.
pub const THRASH_RECOVER_MULT: f32 = 1.25;

/// A step the rat needs out of the arc before a thrash is allowed.
pub const THRASH_ESCAPE: f32 = 24.0;

// ---- the sonic squeak -----------------------------------------------------

/// The shout's telegraph (`SqueakWindup`).
pub const SQUEAK_WINDUP: [f32; 3] = [0.80, 0.72, 0.62];

/// The live ring's inner edge: closer than this the shout passes overhead.
pub const SQUEAK_INNER: f32 = 26.0;

/// The live ring's outer edge.
pub const SQUEAK_OUTER: f32 = 44.0;

/// How far the drawn wavefront travels after the shout (art only).
pub const SQUEAK_REACH: f32 = 132.0;

/// The live window (single hit).
pub const SQUEAK_ACTIVE: f32 = 0.22;

/// Half-angle of the one safe wedge, directly **behind** the boss.
pub const SQUEAK_WEDGE: f32 = 0.62;

/// Shout damage.
pub const SQUEAK_DAMAGE: f32 = 16.0;

/// The shout's shove.
pub const SQUEAK_KNOCKBACK: f32 = 260.0;

/// Clear floor the safe wedge needs behind the boss before it will shout
/// (never scream a rat into a corner it cannot read).
pub const SQUEAK_WEDGE_CLEAR: f32 = 10.0;

// ---- the dash -------------------------------------------------------------

/// The lane is painted for this long (`DashWindup`).
pub const DASH_WINDUP: [f32; 3] = [0.62, 0.55, 0.48];

/// The lane freezes here.
pub const DASH_LOCK_PROGRESS: f32 = 0.58;

/// Live corridor half-width of the pass (the body's own width).
pub const DASH_LANE_HALF: f32 = 10.0;

/// What a clipped pass costs.
pub const DASH_DAMAGE: f32 = 20.0;

/// Lane knockback.
pub const DASH_KNOCKBACK: f32 = 150.0;

/// The shortest lane worth running.
pub const DASH_MIN_LEN: f32 = 96.0;

/// The furthest lane it will run (body through the rat, plus the overshoot).
pub const DASH_REACH: f32 = 210.0;

/// The extra lane past the rat so the pass always crosses it.
pub const DASH_OVERSHOOT: f32 = 42.0;

/// Pass speed per phase (`430 / 470 / 510` px/s).
pub const DASH_SPEED: [f32; 3] = [430.0, 470.0, 510.0];

/// The skid at the end of the pass.
pub const DASH_SKID: f32 = 0.32;

/// The dash cooldown, seconds: the read the owner asked for, mirroring the
/// HUD's own dash gauge.
pub const DASH_COOLDOWN: f32 = 7.0;

/// A step the rat needs off the lane before the dash is allowed.
pub const DASH_ESCAPE: f32 = 24.0;

// ---- the call -------------------------------------------------------------

/// The harmless pose change at every phase threshold.
pub const CALL_DURATION: f32 = 1.20;

/// The chitter that calls the rats, in progress through `Call`.
pub const CALL_SUMMON_PROGRESS: f32 = 0.45;

/// How many ordinary bodies answer (Skitterers — "a few", per the owner).
pub const CALL_RATS: u8 = 3;

/// How far out the answering rats come in (px, ring radius).
pub const CALL_RING: f32 = 34.0;

// ---- the death ------------------------------------------------------------

/// The death beat.
pub const DEATH_DURATION: f32 = 2.60;

/// The blood pool's radius (art only).
pub const DEATH_POOL_RADIUS: f32 = 44.0;

// ---- palette anchors ------------------------------------------------------

/// Key light, upper left (the same convention as the other boss art).
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

/// The kitchen's own warning colours: hot fat, steel and blood.
const WARN: Rgb = (255, 168, 84);
const HOT: Rgb = (255, 96, 62);
const STEEL: Rgb = (188, 198, 206);

// ============================================================================
// State spec
// ============================================================================

/// R-01's move set. Every variant documents its timing, telegraph, lethal
/// area, damage and the counterplay the controller builds to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MegaRatState {
    /// **Groom.** It sits back on its haunches, licks a paw and runs it over
    /// the torn ear, sniffs the air in the rat's direction, and chitters.
    ///
    /// - Duration: [`GROOM_DURATION`] (`1.10 / 0.95 / 0.80` s).
    /// - Warning: none (the neutral beat).
    /// - Dangerous area: body circle, [`CONTACT_DAMAGE`] (8).
    Groom,

    /// **The teeth come up.** The head draws back, the jaw opens and the teeth
    /// line is painted from the muzzle to a marked bite point, freezing at
    /// [`GNAW_LOCK_PROGRESS`].
    ///
    /// - Duration: [`GNAW_WINDUP`] (`0.70 / 0.62 / 0.55` s).
    /// - Warning: the teeth line, [`GNAW_ACTIVE_HALF_WIDTH`] either side.
    GnawWindup,
    /// **The bite.** The whole body drives down the frozen line.
    ///
    /// - Duration: [`GNAW_LUNGE`] (0.26 s); the line is live for
    ///   [`GNAW_ACTIVE`] (0.16 s).
    /// - Damage: [`GNAW_DAMAGE`] (22), once.
    GnawLunge,
    /// **The teeth are in the floor.** It rears up and braces: no body
    /// contact, the belly exposed ([`BELLY_RADIUS`]) and everything landing
    /// counts ×[`GNAW_STUCK_DAMAGE_MULT`].
    ///
    /// - Duration: [`GNAW_STUCK`], or [`GNAW_STUCK_WALLED`] when a wall took
    ///   the teeth.
    GnawStuck,
    /// **The tail winds up.** It plants its forepaws and swings the tail back;
    /// the huge arc ([`THRASH_HALF_ARC`] either side, to [`THRASH_OUTER`]) is
    /// painted in its two halves.
    ///
    /// - Duration: [`THRASH_WINDUP`] (`0.85 / 0.75 / 0.66` s).
    ThrashWindup,
    /// **First pass.** The tail sweeps the rat's own half of the arc.
    ///
    /// - Duration: [`THRASH_PASS`] (0.40 s); damage [`THRASH_DAMAGE`] (18).
    ThrashPass1,
    /// **The gap.** The tail is up at the end of its first pass: nothing is
    /// live and the swept half is the safe floor. This is the window to dash
    /// through.
    ///
    /// - Duration: [`THRASH_GAP`] (0.34 s).
    ThrashGap,
    /// **Second pass.** The tail comes back through the other half.
    ThrashPass2,
    /// **The tail drags.** It is off balance with the tail on the floor: the
    /// tail base is exposed ([`TAIL_BASE_RADIUS`]) and everything landing
    /// counts ×[`THRASH_RECOVER_MULT`].
    ///
    /// - Duration: [`THRASH_RECOVER`] (`0.90 / 0.80 / 0.72` s).
    ThrashRecover,
    /// **The throat fills.** It rears, the head goes back and the ring is
    /// painted round it with the one safe wedge behind.
    ///
    /// - Duration: [`SQUEAK_WINDUP`] (`0.80 / 0.72 / 0.62` s).
    SqueakWindup,
    /// **The squeak.** The ring [`SQUEAK_INNER`]..[`SQUEAK_OUTER`] is live for
    /// [`SQUEAK_ACTIVE`] s, except the wedge behind it.
    ///
    /// - Damage: [`SQUEAK_DAMAGE`] (16) and a [`SQUEAK_KNOCKBACK`] shove.
    SqueakShout,
    /// **The lane is painted.** It crouches, the shoulders bunch, and the lane
    /// through the rat is drawn with the dash gauge ticking down under it.
    ///
    /// - Duration: [`DASH_WINDUP`] (`0.62 / 0.55 / 0.48` s).
    DashWindup,
    /// **The dash.** It runs the lane at [`DASH_SPEED`] with afterimages.
    ///
    /// - Damage: [`DASH_DAMAGE`] (20), once, for the length of the pass.
    DashPass,
    /// **The skid.** It skates to a stop on its claws; the cooldown gauge
    /// restarts at [`DASH_COOLDOWN`].
    ///
    /// - Duration: [`DASH_SKID`] (0.32 s).
    DashSkid,
    /// **The call.** The harmless pose change at a phase threshold: it stands
    /// up, throws its head back and chitters, and at
    /// [`CALL_SUMMON_PROGRESS`] the walls answer with [`CALL_RATS`] rats.
    ///
    /// - Duration: [`CALL_DURATION`] (1.20 s), no contact, no attack.
    Call,
    /// **Death.** The first success goes down: the collar rings, the tail
    /// gives one last sweep and the kitchen comes off the walls.
    Death,
}

impl MegaRatState {
    /// Is this one of the two punish windows (nothing live, free damage)?
    #[inline]
    pub fn is_recovery(self) -> bool {
        matches!(self, MegaRatState::GnawStuck | MegaRatState::ThrashRecover)
    }

    /// Is this state harmless by design (no attack, no contact)?
    #[inline]
    pub fn is_harmless(self) -> bool {
        matches!(self, MegaRatState::Call | MegaRatState::Death)
    }

    /// Is the tail off the floor dragging (the second weak point)?
    #[inline]
    pub fn tail_dragging(self) -> bool {
        self == MegaRatState::ThrashRecover
    }
}

/// Incoming damage multiplier (the art owns it, as for the other bosses).
#[inline]
pub fn incoming_damage_mult(state: MegaRatState) -> f32 {
    match state {
        MegaRatState::GnawStuck => GNAW_STUCK_DAMAGE_MULT,
        MegaRatState::ThrashRecover => THRASH_RECOVER_MULT,
        MegaRatState::Death => 0.0,
        _ => 1.0,
    }
}

/// Does the body bump the rat in this state?
///
/// Off in its own attack windows (the hazard is the damage; a bump would spend
/// the rat's i-frames and mask the tell), in both recoveries (the whole point
/// is that the rat can walk in) and in the harmless call and the death beat.
#[inline]
pub fn has_body_contact(state: MegaRatState) -> bool {
    matches!(
        state,
        MegaRatState::Groom
            | MegaRatState::GnawWindup
            | MegaRatState::ThrashWindup
            | MegaRatState::SqueakWindup
            | MegaRatState::DashWindup
            | MegaRatState::DashSkid
            | MegaRatState::ThrashGap
    )
}

/// Is this state one the art draws as a telegraph rather than a strike?
#[inline]
pub fn is_telegraph(state: MegaRatState) -> bool {
    matches!(
        state,
        MegaRatState::GnawWindup
            | MegaRatState::ThrashWindup
            | MegaRatState::SqueakWindup
            | MegaRatState::DashWindup
    )
}

// ============================================================================
// View
// ============================================================================

/// A pure, `Copy` snapshot of everything the art needs. Built by
/// `Boss::megarat_view`; carries no simulation borrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MegaRatView {
    /// Body centre in world px.
    pub pos: Vec2,
    /// Facing (normalised defensively).
    pub facing: Vec2,
    pub state: MegaRatState,
    /// 0..=1 through the current state.
    pub progress: f32,
    /// Animation clock, seconds.
    pub anim: f32,
    /// Hit flash, 0..=1.
    pub hit_flash: f32,
    /// Remaining health fraction.
    pub hp_frac: f32,
    /// The marked point: the bite point of a lunge, the far end of a dash lane
    /// or the answering rats' ring, depending on the state.
    pub target: Vec2,
    /// The lunge's / the lane's origin.
    pub lane_start: Vec2,
    /// `+1`/`-1`: which half of the arc is swept first, or which way the lane
    /// runs across the facing.
    pub sweep: f32,
    /// Fight phase (1..=3).
    pub phase: u8,
    /// The rat is out of the arena and R-01 is running it down.
    pub hunting: bool,
    /// Dash cooldown remaining fraction (`0` = ready, `1` = just spent).
    pub dash_cd: f32,
    /// The teeth were swallowed by a wall: the stuck window is the long one.
    pub walled: bool,
}

impl MegaRatView {
    /// A grooming view looking at `target`.
    pub fn new(pos: Vec2, facing: Vec2, target: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: MegaRatState::Groom,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
            hp_frac: 1.0,
            target,
            lane_start: pos,
            sweep: 1.0,
            phase: 1,
            hunting: false,
            dash_cd: 0.0,
            walled: false,
        }
    }

    /// Phase 2+ (the collar glints and the fur stands up).
    #[inline]
    pub fn enraged(&self) -> bool {
        self.phase >= 2
    }

    /// Phase 3 shimmer.
    #[inline]
    pub fn critical(&self) -> bool {
        self.phase >= 3
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

/// The head's world centre. While the teeth are in the floor it is **down at
/// the bite point**, which is why the reared body's head is not in the way of
/// the belly.
#[inline]
pub fn head_center(view: &MegaRatView) -> Vec2 {
    let f = safe_facing(view.facing);
    match view.state {
        MegaRatState::GnawLunge | MegaRatState::GnawStuck => view.target,
        _ => view.pos + f * HEAD_OFFSET,
    }
}

/// The muzzle tip (world).
#[inline]
pub fn muzzle(view: &MegaRatView) -> Vec2 {
    let f = safe_facing(view.facing);
    match view.state {
        MegaRatState::GnawLunge | MegaRatState::GnawStuck => view.target,
        _ => view.pos + f * (HEAD_OFFSET + HEAD_RADIUS),
    }
}

/// Three body circles: tail root (behind), haunch, head. All grounded — R-01
/// never leaves the floor.
#[inline]
pub fn body_hit_circles(view: &MegaRatView) -> [(Vec2, f32); 3] {
    let f = safe_facing(view.facing);
    [
        (view.pos - f * TAIL_OFFSET, TAIL_RADIUS),
        (view.pos - f * HAUNCH_OFFSET, HAUNCH_RADIUS),
        (head_center(view), HEAD_RADIUS),
    ]
}

/// The exposed weak point for this state, if any: the belly while it rears out
/// of a stuck gnaw, the tail base while it drags the tail.
#[inline]
pub fn weak_point(view: &MegaRatView) -> Option<(Vec2, f32)> {
    let f = safe_facing(view.facing);
    match view.state {
        MegaRatState::GnawStuck => Some((view.pos + f * BELLY_OFFSET, BELLY_RADIUS)),
        MegaRatState::ThrashRecover => Some((view.pos - f * TAIL_BASE_OFFSET, TAIL_BASE_RADIUS)),
        _ => None,
    }
}

/// The teeth line a `GnawWindup`/`GnawLunge` is drawn along: `(muzzle, bite)`.
#[inline]
pub fn gnaw_line(view: &MegaRatView) -> (Vec2, Vec2) {
    (muzzle(view), view.target)
}

/// The whole marked thrash arc: `(centre, inner, outer, facing, half_angle)`.
#[inline]
pub fn thrash_arc(view: &MegaRatView) -> (Vec2, f32, f32, Vec2, f32) {
    (
        view.pos,
        THRASH_INNER,
        THRASH_OUTER,
        safe_facing(view.facing),
        THRASH_HALF_ARC,
    )
}

/// One pass's half of the arc: `side` is `+1` (the facing's left) or `-1`.
///
/// The half is centred at `side * THRASH_HALF_ARC * 0.5` from the facing with
/// a half-angle of `THRASH_HALF_ARC * 0.5`, so the two passes tile the marked
/// arc exactly.
#[inline]
pub fn thrash_half(view: &MegaRatView, side: f32) -> (Vec2, f32) {
    let f = safe_facing(view.facing);
    let shift = side.signum() * THRASH_HALF_ARC * 0.5;
    (f.rotate(shift), THRASH_HALF_ARC * 0.5)
}

/// The squeak ring: `(centre, inner, outer, wedge_dir, wedge_half)`.
#[inline]
pub fn squeak_ring(view: &MegaRatView) -> (Vec2, f32, f32, Vec2, f32) {
    (
        view.pos,
        SQUEAK_INNER,
        SQUEAK_OUTER,
        -safe_facing(view.facing),
        SQUEAK_WEDGE,
    )
}

/// The dash lane: `(from, to)`.
#[inline]
pub fn dash_lane(view: &MegaRatView) -> (Vec2, Vec2) {
    (view.lane_start, view.target)
}

/// How full the dash gauge reads: `1.0` = ready, `0.0` = just spent.
#[inline]
pub fn dash_gauge(view: &MegaRatView) -> f32 {
    (1.0 - view.dash_cd.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

// ============================================================================
// Pose
// ============================================================================

#[derive(Clone, Copy)]
struct Pose {
    /// Screen-space ground footprint centre.
    ground: Vec2,
    /// Screen-space visual body centre (lifted/leaned by the state).
    body: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    /// Screen-space head centre.
    head: Vec2,
    /// Rearing amount `0..1`: it comes up on its hind legs, belly out.
    rear: f32,
    /// Crouch amount `0..1` (the dash's bunch, the thrash's plant).
    crouch: f32,
    /// Body stretch along the facing (`>0` lunging).
    stretch: f32,
    /// Head lunge (`>0` forward, `<0` drawn back over the shoulders).
    head_lunge: f32,
    /// Jaw open `0..1`.
    jaw: f32,
    /// Tail: `(angle offset from behind, length scale, lift)`.
    tail_swing: f32,
    tail_len: f32,
    tail_lift: f32,
    /// Groom pose phase `0..1` (paw at the muzzle), or -1 for none.
    groom: f32,
    /// Chin-up sniff `0..1`.
    sniff: f32,
    /// Chitter/flex `0..1` (the call, the mock).
    chitter: f32,
    /// Afterimage count/strength for a dash (0 = none).
    after: f32,
    /// Death `0..1` (collapse, roll).
    death: f32,
    /// Fade.
    alpha: f32,
    /// Jitter (the call's shudder).
    jitter: Vec2,
    hit: f32,
    hp: f32,
    phase: u8,
    anim: f32,
    state: MegaRatState,
    progress: f32,
    accent: Rgb,
    /// Screen-space target (bite point / lane end / call ring).
    target: Vec2,
    /// Screen-space lane origin.
    lane_start: Vec2,
    sweep: f32,
    walled: bool,
    hunting: bool,
    dash_cd: f32,
}

impl Pose {
    // Every pose field is written by the `match` below; a couple of them are
    // re-assigned inside a branch before they are read, which is exactly what
    // the state families are for.
    #[allow(unused_assignments)]
    fn from_view(view: &MegaRatView, cam: Vec2, accent: Rgb) -> Self {
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
        let phase = view.phase.clamp(1, 3);
        let target = view.target - cam;
        let ground = view.pos - cam;

        // The state's pose family. The transitions are deliberately continuous:
        // every state starts where the previous one ended, so the eye reads
        // "the same animal" through a whole rotation.
        let mut rear = 0.0f32;
        let mut crouch = 0.0f32;
        let mut stretch = 0.0f32;
        let mut head_lunge = 0.0f32;
        let mut jaw = 0.0f32;
        let mut tail_swing = 0.0f32;
        let mut tail_len = 1.0f32;
        let mut tail_lift = 0.0f32;
        let mut groom = -1.0f32;
        let mut sniff = 0.0f32;
        let mut chitter = 0.0f32;
        let mut after = 0.0f32;
        let mut death = 0.0f32;
        let mut alpha = 1.0f32;
        let mut jitter = Vec2::ZERO;
        let bob = (anim * 2.6).sin() * 0.8;

        match view.state {
            MegaRatState::Groom => {
                // It grooms, then sniffs in the rat's direction: the two-beat
                // neutral that gives it a personality instead of an idle.
                let t = prog;
                if t < 0.55 {
                    groom = (t / 0.55).clamp(0.0, 1.0);
                } else {
                    sniff = ((t - 0.55) / 0.45).clamp(0.0, 1.0);
                }
                rear = 0.25;
                head_lunge = 0.10 * sniff;
                tail_swing = (anim * 1.4).sin() * 0.18;
            }
            MegaRatState::GnawWindup => {
                // The head draws back, the jaw opens, the shoulders bunch.
                let draw = (prog / GNAW_LOCK_PROGRESS).clamp(0.0, 1.0);
                crouch = 0.35 + 0.45 * draw;
                head_lunge = -0.55 * draw;
                jaw = draw;
                stretch = -0.10 * draw;
                tail_swing = -0.35 * draw;
                tail_len = 1.0 + 0.10 * draw;
                rear = 0.05;
            }
            MegaRatState::GnawLunge => {
                // The whole animal down the line, jaw wide.
                stretch = 0.42;
                head_lunge = 0.85;
                jaw = 0.9;
                crouch = 0.15;
                tail_swing = 0.25;
                tail_len = 0.86;
            }
            MegaRatState::GnawStuck => {
                // Reared up on the hind legs, front paws bracing on the floor,
                // muzzle pinned at the bite point, belly out and bright.
                let pull = (prog * 6.0).sin().abs();
                rear = 1.0;
                head_lunge = 0.55 + 0.12 * pull;
                jaw = 0.55;
                crouch = 0.0;
                tail_swing = -0.15 + 0.1 * pull;
                tail_len = 1.0;
                tail_lift = 0.4;
                jitter = p * (pull * 0.6);
            }
            MegaRatState::ThrashWindup => {
                // Forepaws planted, tail swung right back (the wind-up), weight
                // over the shoulders.
                let wind = (prog / 0.85).clamp(0.0, 1.0);
                crouch = 0.55 + 0.25 * wind;
                rear = 0.10;
                tail_swing = -view.sweep.signum() * (0.55 + 0.55 * wind);
                tail_len = 1.0 + 0.22 * wind;
                tail_lift = 0.15 * wind;
                head_lunge = -0.15;
            }
            MegaRatState::ThrashPass1 => {
                // The tail crosses its half of the arc.
                let s = (prog * 2.0 - 1.0).clamp(-1.0, 1.0);
                crouch = 0.6;
                tail_swing = view.sweep.signum() * (1.25 - 1.05 * (0.5 + 0.5 * s));
                tail_len = 1.22;
                tail_lift = 0.10;
                jitter = p * 0.4;
                head_lunge = -0.05;
            }
            MegaRatState::ThrashGap => {
                // The tail is up at the end of the pass; the body is catching
                // its balance. Nothing is live.
                crouch = 0.45;
                tail_swing = -view.sweep.signum() * 1.15;
                tail_len = 1.15;
                tail_lift = 0.35;
                head_lunge = 0.0;
            }
            MegaRatState::ThrashPass2 => {
                let s = (prog * 2.0 - 1.0).clamp(-1.0, 1.0);
                crouch = 0.6;
                tail_swing = -view.sweep.signum() * (1.25 - 1.05 * (0.5 + 0.5 * s));
                tail_len = 1.22;
                tail_lift = 0.10;
                jitter = p * 0.4;
                head_lunge = 0.10;
            }
            MegaRatState::ThrashRecover => {
                // The tail drops and drags; it is off balance over its front.
                let settle = (prog * 2.4).sin().abs();
                crouch = 0.35 - 0.15 * settle;
                tail_swing = 0.55 + 0.25 * settle;
                tail_len = 1.1;
                tail_lift = -0.3;
                head_lunge = 0.2;
                jitter = p * (0.5 * settle);
            }
            MegaRatState::SqueakWindup => {
                // The throat inflates, the head goes back and up.
                let swell = prog.clamp(0.0, 1.0);
                rear = 0.45 + 0.35 * swell;
                head_lunge = -0.5 * swell;
                jaw = 0.35 + 0.55 * swell;
                chitter = swell;
                tail_swing = (anim * 7.0).sin() * 0.2;
                tail_lift = 0.25;
            }
            MegaRatState::SqueakShout => {
                // Head thrown back, jaw wide, whole body rigid.
                rear = 0.8;
                head_lunge = -0.65;
                jaw = 1.0;
                chitter = 1.0;
                jitter = p * (1.0 - prog) * 1.4;
                tail_swing = 0.1;
                tail_lift = 0.3;
            }
            MegaRatState::DashWindup => {
                // Shoulders down, haunches loaded like a spring.
                let load = (prog / DASH_LOCK_PROGRESS).clamp(0.0, 1.0);
                crouch = 0.7 + 0.3 * load;
                head_lunge = 0.35 * load;
                jaw = 0.4;
                tail_swing = -0.2;
                tail_len = 0.85;
                stretch = -0.12;
            }
            MegaRatState::DashPass => {
                // Flat out, body horizontal, tail streaming behind.
                stretch = 0.55;
                crouch = 0.85;
                head_lunge = 0.9;
                jaw = 0.75;
                tail_swing = 0.0;
                tail_len = 0.75;
                after = 1.0;
            }
            MegaRatState::DashSkid => {
                let skid = (1.0 - prog).clamp(0.0, 1.0);
                stretch = 0.30 * skid;
                crouch = 0.6 + 0.2 * skid;
                head_lunge = 0.3 * skid;
                jaw = 0.3;
                tail_swing = 0.35 * skid;
                tail_len = 0.95;
                after = 0.4 * skid;
                jitter = p * (0.6 * skid);
            }
            MegaRatState::Call => {
                // Up on the hind legs, chest out, head thrown back: the pose
                // the walls answer.
                let rise = (prog / 0.30).clamp(0.0, 1.0);
                rear = rise;
                head_lunge = -0.75 * rise;
                jaw = 0.8 * rise;
                chitter = 0.6 + 0.4 * (anim * 11.0).sin().abs();
                jitter = p * ((anim * 26.0).sin() * 0.7);
                tail_lift = 0.5 * rise;
                tail_swing = 0.2;
            }
            MegaRatState::Death => {
                // It goes over sideways, the collar ringing on the tile.
                death = prog.clamp(0.0, 1.0);
                rear = (1.0 - death * 1.6).max(0.0);
                crouch = 0.6 + 0.4 * death;
                tail_swing = death * 0.9;
                tail_lift = -0.6 * death;
                head_lunge = 0.3 * death;
                jaw = 0.5 * death;
                alpha = 1.0 - (death * death_fade()).clamp(0.0, 0.55);
            }
        }

        // Rebuilding the body centre from the pose: rearing lifts the chest,
        // crouching pulls it down, a lunge stretches it forward.
        let lift = rear * 5.5 - crouch * 3.0;
        let body = ground + Vec2::new(0.0, -lift) + f * (stretch * 3.0) + Vec2::new(0.0, bob * 0.4);
        // The drawn head sits a little forward of its hit circle: a rat's head
        // has a neck, and a head that overlaps the shoulders reads as a lump.
        let head = body
            + f * ((HEAD_OFFSET + 5.0 + head_lunge * 6.0) * (1.0 + stretch * 0.15))
            - Vec2::new(0.0, rear * 9.0)
            + jitter * 0.5
            + Vec2::new(0.0, -sniff * 3.0 + death * 6.0);

        Self {
            ground,
            body,
            f,
            p,
            fl: f.x,
            pl: p.x,
            head,
            rear,
            crouch,
            stretch,
            head_lunge,
            jaw,
            tail_swing,
            tail_len,
            tail_lift,
            groom,
            sniff,
            chitter,
            after,
            death,
            alpha,
            jitter,
            hit,
            hp,
            phase,
            anim,
            state: view.state,
            progress: prog,
            accent,
            target,
            lane_start: view.lane_start - cam,
            sweep: if view.sweep < 0.0 { -1.0 } else { 1.0 },
            walled: view.walled,
            hunting: view.hunting,
            dash_cd: view.dash_cd.clamp(0.0, 1.0),
        }
    }

    /// Local body frame of a screen point: `(u along facing, v across)`.
    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - self.body;
        (d.dot(self.f), d.dot(self.p))
    }

    /// Screen point from a body-frame offset.
    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.body + self.f * u + self.p * v
    }
}

/// How much of the body fades in the death beat (art only).
#[inline]
fn death_fade() -> f32 {
    0.55
}

/// Lerp two screen points.
#[inline]
fn vlerp(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

// ============================================================================
// Small pixel helpers (stack-only, bounds-checked)
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

/// A soft world-space segment (telegraph rails, tail, whiskers).
fn draw_segment(pb: &mut PixelBuffer, a: Vec2, b: Vec2, width: f32, c: Rgb, alpha: f32) {
    if alpha <= 0.004 {
        return;
    }
    let ab = b - a;
    let len = ab.len();
    if !len.is_finite() || len < 0.01 {
        return;
    }
    let steps = (len * 2.0).ceil() as i32;
    let steps = steps.clamp(1, 240);
    let r = (width * 0.5).max(0.5);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        soft_dot(pb, a + ab * t, r, c, alpha);
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
    let du = (u - cu) / ru.max(0.001);
    let dv = (v - cv) / rv.max(0.001);
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

/// Hermite ramp from `edge0` to `edge1`.
#[inline]
fn smoothstep(x: f32, edge0: f32, edge1: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

/// Is `angle` inside a sector of `half` either side of `facing`?
#[inline]
fn in_sector(dir: Vec2, facing: Vec2, half: f32) -> bool {
    let d = dir.normalized();
    let f = safe_facing(facing);
    d.dot(f) >= half.cos()
}

// ============================================================================
// Fur palette (the rat's own colours, gone wrong)
// ============================================================================

struct Coat {
    outline: Rgb,
    shadow: Rgb,
    base: Rgb,
    light: Rgb,
    /// The greasy dark stripe down the spine.
    dorsal: Rgb,
    /// The pale underbelly and the rubbed-bare patches.
    belly: Rgb,
    belly_shadow: Rgb,
    /// The face's own colour (lighter than the back, as a rat's is).
    face: Rgb,
    scar: Rgb,
    ear_pink: Rgb,
    ear_inner: Rgb,
    nose: Rgb,
    paw: Rgb,
    tail: Rgb,
    tail_tip: Rgb,
    incisor: Rgb,
    /// The orange front of an incisor.
    tooth: Rgb,
    eye: Rgb,
    collar: Rgb,
    tag: Rgb,
}

fn coat(pose: &Pose) -> Coat {
    // The player's rat is pale winter fur (`212,206,218`); R-01 is the same
    // animal kept in a cage for a decade: filthy, greasy, rubbed bare at the
    // shoulders. Phase 2+ flushes the skin; phase 3 runs it red at the seams.
    let e = (pose.phase as f32 - 1.0) / 2.0 * 0.7;
    let dead = pose.death.clamp(0.0, 1.0) * 0.55;
    let base = mix(mix((124, 110, 96), (142, 106, 86), e), (86, 74, 70), dead);
    Coat {
        outline: mix((16, 12, 12), (32, 16, 20), e),
        shadow: mix((52, 44, 38), (68, 38, 34), e),
        base,
        light: mix((186, 176, 156), (206, 168, 134), e),
        dorsal: mix((40, 34, 28), (54, 30, 26), e),
        belly: mix((196, 178, 152), (214, 166, 138), e),
        belly_shadow: mix((104, 92, 78), (118, 78, 66), e),
        face: mix((204, 186, 162), (220, 180, 150), e),
        scar: mix((236, 170, 160), (248, 138, 124), e),
        ear_pink: mix((186, 118, 124), (206, 104, 106), e),
        ear_inner: mix((216, 150, 156), (232, 130, 130), e),
        nose: mix((224, 122, 140), (240, 108, 118), e),
        paw: (216, 188, 172),
        tail: mix((168, 122, 128), (176, 104, 104), e),
        tail_tip: mix((196, 158, 160), (208, 132, 128), e),
        incisor: (255, 250, 232),
        tooth: (236, 176, 92),
        eye: (18, 10, 14),
        collar: (152, 160, 170),
        tag: (232, 204, 118),
    }
}

// ============================================================================
// Ground layer
// ============================================================================

/// The telegraphs, the dash gauge and the drop shadow, drawn on the floor
/// before the props and the bodies.
pub fn draw_megarat_ground(pb: &mut PixelBuffer, view: &MegaRatView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let margin = CULL_MARGIN + 16.0;
    if pose.ground.x < -margin
        || pose.ground.x > pb.w as f32 + margin
        || pose.ground.y < -margin
        || pose.ground.y > pb.h as f32 + margin
    {
        return;
    }
    draw_shadow(pb, &pose);
    draw_dash_gauge(pb, &pose);
    match view.state {
        MegaRatState::GnawWindup | MegaRatState::GnawLunge => draw_teeth_line(pb, &pose),
        MegaRatState::GnawStuck => draw_bite_mark(pb, &pose),
        MegaRatState::ThrashWindup
        | MegaRatState::ThrashPass1
        | MegaRatState::ThrashGap
        | MegaRatState::ThrashPass2 => draw_thrash_arc(pb, &pose),
        MegaRatState::ThrashRecover => draw_tail_drag(pb, &pose),
        MegaRatState::SqueakWindup | MegaRatState::SqueakShout => draw_squeak_ring(pb, &pose),
        MegaRatState::DashWindup | MegaRatState::DashPass | MegaRatState::DashSkid => {
            draw_dash_lane(pb, &pose)
        }
        MegaRatState::Call => draw_call_marks(pb, &pose),
        MegaRatState::Death => draw_death_pool(pb, &pose),
        MegaRatState::Groom => {}
    }
}

fn draw_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    // A big low body: the shadow is a wide soft ellipse, weaker while rearing
    // (the chest comes off the floor) and gone in the death beat.
    let widen = 1.0 + pose.stretch * 0.5;
    let a = 0.30 * pose.alpha;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.ground, 30.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let d = ellipse_sdf(u, v, 1.0, 1.6, 22.0 * widen, 11.0);
            if d > 0.0 {
                continue;
            }
            let fall = (1.0 - (-d / 12.0)).clamp(0.0, 1.0);
            blend_px(pb, x, y, (12, 10, 14), a * fall);
        }
    }
    // Contact shade right under the belly.
    soft_dot(pb, pose.ground + Vec2::new(0.0, 4.0), 12.0, (10, 8, 12), a * 0.5);
}

/// The dash cooldown gauge: four pips under the boss, dim while cooling and
/// white when the dash is ready — the same read as the HUD's own gauge.
fn draw_dash_gauge(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.death > 0.5 {
        return;
    }
    let ready = pose.dash_cd <= 0.02;
    let base = pose.ground + Vec2::new(0.0, 20.0);
    let pip_gap = 5.0;
    let filled = ((1.0 - pose.dash_cd) * 4.0).ceil().clamp(0.0, 4.0) as i32;
    for k in 0..4 {
        let at = base - Vec2::new((k as f32 - 1.5) * pip_gap, 0.0);
        let on = 4 - k <= filled;
        let (col, a) = if ready {
            (
                (246, 252, 255),
                if (pose.anim * 3.0).sin() > 0.0 { 0.95 } else { 0.65 },
            )
        } else if on {
            (WARN, 0.80)
        } else {
            ((86, 82, 88), 0.55)
        };
        soft_dot(pb, at, 2.1, col, a);
    }
    // A ready dash gets a bright chevron in front of the gauge.
    if ready {
        let tip = base + Vec2::new(0.0, -1.0);
        draw_segment(
            pb,
            tip + Vec2::new(-5.0, 3.0),
            tip,
            1.2,
            (255, 252, 232),
            0.7,
        );
        draw_segment(
            pb,
            tip + Vec2::new(5.0, 3.0),
            tip,
            1.2,
            (255, 252, 232),
            0.7,
        );
    }
}

/// The teeth line: two rails to the bite point, chevrons for the incisors and
/// the live corridor once the lunge runs.
fn draw_teeth_line(pb: &mut PixelBuffer, pose: &Pose) {
    let live = pose.state == MegaRatState::GnawLunge;
    let (from, to) = (pose.head, pose.target);
    let dir = (to - from).normalized();
    if dir.len_sq() <= 0.0 {
        return;
    }
    let perp = dir.perp();
    let pulse = 0.5 + 0.5 * (pose.anim * 16.0).sin();
    let rail = if live { HOT } else { WARN };
    let a = if live { 0.75 } else { 0.42 + 0.18 * pulse };
    // The two rails: the incisors' width.
    for &side in &[-1.0f32, 1.0] {
        draw_segment(
            pb,
            from + perp * (side * GNAW_ACTIVE_HALF_WIDTH),
            to + perp * (side * GNAW_ACTIVE_HALF_WIDTH),
            1.0,
            rail,
            a,
        );
    }
    // The corridor fill appears once it is live, so the tell stays a line.
    if live {
        let steps = 26;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let at = from + (to - from) * t;
            soft_dot(pb, at, GNAW_ACTIVE_HALF_WIDTH * 0.8, HOT, 0.16 * (1.0 - t * 0.4));
        }
    }
    // Tooth chevrons marching to the bite point.
    let teeth = 5;
    for k in 0..teeth {
        let t = 0.18 + 0.16 * k as f32;
        let at = from + (to - from) * t;
        let w = 2.4 + 1.2 * k as f32;
        draw_segment(
            pb,
            at - perp * w,
            at + dir * 2.2 - perp * (w - 1.0),
            1.0,
            rail,
            a * 0.9,
        );
        draw_segment(
            pb,
            at + perp * w,
            at + dir * 2.2 + perp * (w - 1.0),
            1.0,
            rail,
            a * 0.9,
        );
    }
    // The bite ring: the point the rat has to leave.
    let ring_r = GNAW_ACTIVE_HALF_WIDTH + 2.0;
    let seg = 20;
    for i in 0..seg {
        let a0 = i as f32 / seg as f32 * TAU;
        let a1 = (i + 1) as f32 / seg as f32 * TAU;
        draw_segment(
            pb,
            to + Vec2::from_angle(a0) * ring_r,
            to + Vec2::from_angle(a1) * ring_r,
            1.0,
            rail,
            a,
        );
    }
}

/// The floor it bit: a splinter star at the bite point plus the braced paws.
fn draw_bite_mark(pb: &mut PixelBuffer, pose: &Pose) {
    let at = pose.target;
    let pulse = 0.5 + 0.5 * (pose.anim * 9.0).sin();
    // Splintered tile around the incisors.
    for k in 0..9 {
        let a = k as f32 / 9.0 * TAU + 0.3;
        let len = 5.0 + 4.0 * hash2(k, 3, 17);
        draw_segment(
            pb,
            at + Vec2::from_angle(a) * 3.0,
            at + Vec2::from_angle(a) * len,
            1.0,
            (52, 44, 40),
            0.55,
        );
    }
    soft_dot(pb, at, 4.0, (32, 26, 24), 0.55);
    soft_dot(pb, at, 2.0, (255, 250, 232), 0.30 * pulse);
    // Four claws scrabbling for purchase (it is bracing to pull free).
    for &side in &[-1.0f32, 1.0] {
        for k in 0..2 {
            let at2 = pose.place(6.0 + k as f32 * 4.0, side * (10.0 + k as f32 * 2.0));
            draw_segment(
                pb,
                at2,
                at2 + pose.f * 4.0 + pose.p * (side * 1.5),
                1.0,
                (216, 202, 186),
                0.55,
            );
        }
    }
}

/// The thrash arc: the marked sector, split into the two halves, with the
/// swept one dimmed during the gap and the live one hot during a pass.
fn draw_thrash_arc(pb: &mut PixelBuffer, pose: &Pose) {
    let (center, inner, outer, facing, half) = (
        pose.ground,
        THRASH_INNER,
        THRASH_OUTER,
        safe_facing(pose.f),
        THRASH_HALF_ARC,
    );
    let state = pose.state;
    let rat_side = pose.sweep;
    let pulse = 0.5 + 0.5 * (pose.anim * 14.0).sin();
    let segments = 64;

    // Which half is live (if any) and which has already been swept.
    let (live_side, swept_side) = match state {
        MegaRatState::ThrashWindup => (0.0, 0.0),
        MegaRatState::ThrashPass1 => (rat_side, 0.0),
        MegaRatState::ThrashGap => (0.0, rat_side),
        MegaRatState::ThrashPass2 => (-rat_side, rat_side),
        _ => (0.0, 0.0),
    };

    for i in 0..segments {
        let t0 = i as f32 / segments as f32;
        let t1 = (i + 1) as f32 / segments as f32;
        let a0 = -half + 2.0 * half * t0;
        let a1 = -half + 2.0 * half * t1;
        let dir0 = facing.rotate(a0);
        let dir1 = facing.rotate(a1);
        let mid_side = if (a0 + a1) * 0.5 >= 0.0 { 1.0 } else { -1.0 };
        let is_live = live_side.abs() > 0.5 && mid_side * live_side > 0.0;
        let is_swept = swept_side.abs() > 0.5 && mid_side * swept_side > 0.0;
        let (col, alpha) = if is_live {
            (HOT, 0.55 + 0.25 * pulse)
        } else if is_swept {
            ((70, 62, 66), 0.30)
        } else {
            (WARN, 0.26 + 0.10 * pulse)
        };
        // The band's two rails.
        draw_segment(
            pb,
            center + dir0 * inner,
            center + dir1 * inner,
            1.0,
            col,
            alpha,
        );
        draw_segment(
            pb,
            center + dir0 * outer,
            center + dir1 * outer,
            1.0,
            col,
            alpha,
        );
        // Radial ticks every 8th segment, plus the fill in the live half.
        if i % 8 == 0 {
            draw_segment(
                pb,
                center + dir0 * inner,
                center + dir0 * outer,
                1.0,
                col,
                alpha * 0.7,
            );
        }
        if is_live || is_swept {
            for k in 1..4 {
                let r = inner + (outer - inner) * k as f32 / 4.0;
                soft_dot(
                    pb,
                    center + (dir0 + dir1) * 0.5 * r,
                    1.2,
                    col,
                    alpha * 0.22,
                );
            }
        }
    }
    // The sector's own edges: two radii, so the arc reads as a swept *sector*
    // and not as two circles.
    for &side in &[-1.0f32, 1.0] {
        let edge = facing.rotate(half * side);
        draw_segment(
            pb,
            center + edge * inner,
            center + edge * outer,
            1.2,
            if live_side.abs() > 0.5 || swept_side.abs() > 0.5 {
                WARN
            } else {
                (255, 200, 120)
            },
            0.45,
        );
    }
    // The tail's head marker: a bright wedge where the tail is right now.
    let swing = match state {
        MegaRatState::ThrashWindup => -rat_side * 0.55,
        MegaRatState::ThrashPass1 => rat_side * (1.10 - 0.9 * pose.progress),
        MegaRatState::ThrashGap => -rat_side * 1.15,
        MegaRatState::ThrashPass2 => -rat_side * (1.10 - 0.9 * pose.progress),
        _ => rat_side * 0.55,
    };
    let head_dir = facing.rotate(swing.clamp(-half, half));
    let tip = center + head_dir * outer;
    soft_dot(pb, tip, 2.6, (255, 236, 202), 0.55);
}

/// The dragging tail's smear: the arc the tail is lying in, and the bright
/// tail base where it is exposed.
fn draw_tail_drag(pb: &mut PixelBuffer, pose: &Pose) {
    let side = pose.sweep.signum();
    let centre = pose.ground;
    let dir = pose.f.rotate(side * 0.9);
    let perp = dir.perp();
    let from = centre + dir * (THRASH_INNER + 4.0) - perp * 10.0;
    let to = centre + dir * (THRASH_OUTER - 10.0) + perp * 14.0;
    let pulse = 0.5 + 0.5 * (pose.anim * 8.0).sin();
    draw_segment(pb, from, to, 4.0, (40, 32, 30), 0.30);
    draw_segment(pb, from, to, 2.0, (86, 70, 62), 0.30 + 0.10 * pulse);
    // Scuffed tile where the tail has been dragging.
    for k in 0..7 {
        let t = k as f32 / 7.0;
        let at = from + (to - from) * t;
        soft_dot(pb, at + perp * ((k % 2) as f32 * 3.0 - 1.5), 1.2, (28, 24, 22), 0.25);
    }
}

/// The squeak ring: the marked band, the safe wedge behind it and, while the
/// shout runs, the expanding wavefronts.
fn draw_squeak_ring(pb: &mut PixelBuffer, pose: &Pose) {
    let (center, inner, outer, wedge_dir, wedge) = (
        pose.ground,
        SQUEAK_INNER,
        SQUEAK_OUTER,
        safe_facing(-pose.f),
        SQUEAK_WEDGE,
    );
    let shouting = pose.state == MegaRatState::SqueakShout;
    let pulse = 0.5 + 0.5 * (pose.anim * 15.0).sin();
    let segments = 56;
    let (col, alpha) = if shouting {
        ((214, 246, 255), 0.72)
    } else {
        ((146, 226, 250), 0.30 + 0.16 * pulse)
    };
    for i in 0..segments {
        let a0 = -PI + TAU * i as f32 / segments as f32;
        let a1 = -PI + TAU * (i + 1) as f32 / segments as f32;
        let dir0 = Vec2::from_angle(a0);
        let dir1 = Vec2::from_angle(a1);
        let mid = Vec2::from_angle((a0 + a1) * 0.5);
        let in_wedge = in_sector(mid, wedge_dir, wedge);
        if in_wedge {
            continue;
        }
        draw_segment(pb, center + dir0 * inner, center + dir1 * inner, 1.0, col, alpha);
        draw_segment(pb, center + dir0 * outer, center + dir1 * outer, 1.0, col, alpha);
        // Tick the band so it reads as a live ring, not decoration.
        if i % 7 == 0 {
            draw_segment(
                pb,
                center + dir0 * inner,
                center + dir0 * outer,
                0.9,
                col,
                alpha * 0.55,
            );
        }
    }
    // The safe wedge: two rails and a chevron pointing into it, in the rat's
    // own "safe" blue-white so the read is "go here".
    let safe = (150, 246, 210);
    for &side in &[-1.0f32, 1.0] {
        let edge = wedge_dir.rotate(side * wedge);
        draw_segment(
            pb,
            center + edge * inner,
            center + edge * (outer + 14.0),
            1.0,
            safe,
            0.45 + 0.15 * pulse,
        );
    }
    let mark = center + wedge_dir * (outer + 4.0);
    draw_segment(pb, mark + wedge_dir.perp() * 5.0, mark, 1.4, safe, 0.75);
    draw_segment(pb, mark - wedge_dir.perp() * 5.0, mark, 1.4, safe, 0.75);
    if shouting {
        // The expanding wavefronts: two rings on their way out.
        for k in 0..2 {
            let t = (pose.progress * 1.6 - k as f32 * 0.35).clamp(0.0, 1.0);
            let r = inner + (SQUEAK_REACH - inner) * t;
            let a = 0.55 * (1.0 - t);
            for i in 0..segments {
                let a0 = -PI + TAU * i as f32 / segments as f32;
                let a1 = -PI + TAU * (i + 1) as f32 / segments as f32;
                let mid = Vec2::from_angle((a0 + a1) * 0.5);
                if in_sector(mid, wedge_dir, wedge) {
                    continue;
                }
                draw_segment(
                    pb,
                    center + Vec2::from_angle(a0) * r,
                    center + Vec2::from_angle(a1) * r,
                    1.0,
                    (236, 252, 255),
                    a,
                );
            }
        }
    }
}

/// The dash lane: two rails, hatched centre, the arrow at the end, and the
/// afterimages it leaves behind while it runs.
fn draw_dash_lane(pb: &mut PixelBuffer, pose: &Pose) {
    let (from, to) = (pose.lane_start, pose.target);
    let dir = (to - from).normalized();
    if dir.len_sq() <= 0.0 {
        return;
    }
    let perp = dir.perp();
    let len = from.dist(to);
    let running = pose.state == MegaRatState::DashPass;
    let pulse = 0.5 + 0.5 * (pose.anim * 15.0).sin();
    let (col, a) = if running {
        ((255, 250, 226), 0.90)
    } else {
        ((226, 238, 248), 0.52 + 0.22 * pulse)
    };
    for &side in &[-1.0f32, 1.0] {
        draw_segment(
            pb,
            from + perp * (side * DASH_LANE_HALF),
            to + perp * (side * DASH_LANE_HALF),
            1.0,
            col,
            a,
        );
    }
    // Hatch across the lane.
    let hatch = 11;
    for k in 1..hatch {
        let t = k as f32 / hatch as f32;
        let at = from + (to - from) * t;
        draw_segment(
            pb,
            at - perp * DASH_LANE_HALF,
            at + perp * DASH_LANE_HALF,
            0.9,
            col,
            a * 0.35,
        );
    }
    // The arrow at the far end.
    let tip = to;
    for &side in &[-1.0f32, 1.0] {
        draw_segment(
            pb,
            tip - dir * 6.0 + perp * (side * 5.0),
            tip,
            1.3,
            col,
            a,
        );
    }
    if running {
        // Afterimages: three ghost-bodied copies trailing the run.
        for k in 0..3 {
            let back = (k as f32 + 1.0) * 12.0;
            let at = pose.body - dir * back;
            let fade = (0.24 - 0.06 * k as f32).max(0.0);
            for &side in &[-1.0f32, 1.0] {
                soft_dot(pb, at + perp * (side * 7.0), 4.5, (198, 226, 255), fade);
            }
            soft_dot(pb, at, 5.0, (226, 240, 255), fade * 0.8);
        }
        // Scuffed tile under the pass.
        for k in 0..((len / 10.0) as i32).clamp(1, 18) {
            let t = k as f32 / 18.0;
            let at = from + (to - from) * t;
            soft_dot(pb, at + perp * 4.0, 1.4, (46, 40, 36), 0.30);
        }
    }
}

/// The call: scurry arrows converging on the boss from the walls.
fn draw_call_marks(pb: &mut PixelBuffer, pose: &Pose) {
    let armed = pose.progress >= CALL_SUMMON_PROGRESS;
    let pulse = 0.5 + 0.5 * (pose.anim * 12.0).sin();
    let ring = CALL_RING;
    for k in 0..CALL_RATS as i32 {
        let a = TAU * k as f32 / CALL_RATS as f32 + 0.4;
        let out = Vec2::from_angle(a);
        let near = pose.ground + out * ring;
        let far = pose.ground + out * (ring + 26.0);
        let col = if armed { (255, 216, 150) } else { (156, 132, 116) };
        let alpha = if armed { 0.55 + 0.25 * pulse } else { 0.22 };
        draw_segment(pb, far, near, 1.2, col, alpha);
        for &side in &[-1.0f32, 1.0] {
            let tip = near;
            let back = near + out * 6.0;
            draw_segment(pb, back + out.perp() * (side * 4.0), tip, 1.0, col, alpha * 0.8);
        }
    }
}

fn draw_death_pool(pb: &mut PixelBuffer, pose: &Pose) {
    let grow = (pose.death * 1.4).clamp(0.0, 1.0);
    let r = DEATH_POOL_RADIUS * grow;
    if r < 1.0 {
        return;
    }
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.ground, r + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let d = ellipse_sdf(u, v, -2.0, 3.0, r * 0.9, r * 0.5);
            if d > 0.0 {
                continue;
            }
            let rim = (1.0 - (-d / 3.0)).clamp(0.0, 1.0);
            blend_px(pb, x, y, (86, 18, 24), 0.30 + 0.25 * rim);
            blend_px(pb, x, y, (132, 30, 34), 0.16 * rim);
        }
    }
}

// ============================================================================
// Body layer
// ============================================================================

/// The Megarat itself: tail, legs, one great hunched body, head, incisors,
/// collar and the weak-point glint, drawn with the other bodies.
pub fn draw_megarat(pb: &mut PixelBuffer, view: &MegaRatView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);
    let margin = CULL_MARGIN + 16.0;
    if pose.ground.x < -margin
        || pose.ground.x > pb.w as f32 + margin
        || pose.ground.y < -margin
        || pose.ground.y > pb.h as f32 + margin
    {
        return;
    }
    let pal = coat(&pose);
    draw_tail(pb, &pose, &pal);
    draw_hind_legs(pb, &pose, &pal);
    draw_body(pb, &pose, &pal);
    draw_front_legs(pb, &pose, &pal);
    draw_collar(pb, &pose, &pal);
    draw_head(pb, &pose, &pal);
    match pose.state {
        MegaRatState::GnawStuck => draw_belly_glint(pb, &pose),
        MegaRatState::ThrashRecover => draw_tail_base_glint(pb, &pose),
        MegaRatState::Call => draw_call_breath(pb, &pose),
        MegaRatState::Groom => draw_groom_motes(pb, &pose, &pal),
        MegaRatState::Death => draw_death_tell_tale(pb, &pose, &pal),
        _ => {}
    }
    if pose.phase >= 3 && pose.state != MegaRatState::Death {
        draw_phase3_heat(pb, &pose);
    }
}

/// The tail: a thick, hairless, ringed whip. It swings with `tail_swing`,
/// streams behind a dash and goes limp as it dies.
fn draw_tail(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    let root = pose.place(-16.0, 1.5);
    let base_angle = PI + pose.tail_swing;
    let len = 38.0 * pose.tail_len;
    let segs = 22;
    let mut prev = root;
    for i in 1..=segs {
        let t = i as f32 / segs as f32;
        // A slight S-curve so the tail is never a ruler.
        let bend = (t * 2.4 + pose.anim * 1.7).sin() * 0.24 * t;
        let ang = base_angle + bend;
        let dir = Vec2::from_angle(ang);
        let next = root + dir * (len * t) + Vec2::new(0.0, pose.tail_lift * t * 5.0);
        let r = 4.0 * (1.0 - t) + 1.2;
        let col = mix(pal.tail, pal.tail_tip, t);
        draw_segment(pb, prev, next, r * 2.0, pal.outline, 0.85 * pose.alpha);
        draw_segment(pb, prev, next, r * 1.3, apply_hit(col, pose.hit), pose.alpha);
        prev = next;
    }
    // Scale rings all the way down the tail (a real rat's tail, thick at the
    // root and naked to the tip).
    for k in 1..9 {
        let t = k as f32 / 9.0;
        let bend = (t * 2.4 + pose.anim * 1.7).sin() * 0.24 * t;
        let at = root + Vec2::from_angle(base_angle + bend) * (len * t)
            + Vec2::new(0.0, pose.tail_lift * t * 5.0);
        let perp = Vec2::from_angle(base_angle + bend).perp();
        let w = (4.0 * (1.0 - t) + 1.2) * 0.9;
        draw_segment(pb, at - perp * w, at + perp * w, 0.8, pal.outline, 0.30 * pose.alpha);
    }
    soft_dot(pb, prev, 1.8, pal.tail_tip, 0.9 * pose.alpha);
}

/// The hind legs: a heavy thigh, a hock, and a long foot with four toes —
/// planted while it grooms, splayed while it braces a stuck lunge.
fn draw_hind_legs(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    if pose.death > 0.75 {
        return;
    }
    let brace = if pose.state == MegaRatState::GnawStuck {
        1.0
    } else {
        0.0
    };
    for &side in &[-1.0f32, 1.0] {
        let near = side < 0.0;
        let thigh = pose.place(-12.0, side * (8.0 + brace * 2.0));
        let foot = pose.place(
            -18.0 - brace * 3.0,
            side * (12.0 + brace * 3.0) + pose.crouch * 1.5,
        );
        // Thigh (a dark matted mass under the haunch).
        soft_dot(pb, thigh, 8.6, pal.outline, 0.75 * pose.alpha);
        soft_dot(pb, thigh, 7.2, mix(pal.dorsal, pal.shadow, 0.5), 0.95 * pose.alpha);
        // Shin to the foot.
        draw_segment(pb, thigh, foot, 5.6, pal.outline, 0.85 * pose.alpha);
        draw_segment(pb, thigh, foot, 3.8, mix(pal.dorsal, pal.shadow, 0.4), 0.98 * pose.alpha);
        // Foot pad and toes.
        soft_dot(pb, foot, 3.6, pal.outline, 0.80 * pose.alpha);
        soft_dot(pb, foot, 2.6, mix(pal.paw, pal.shadow, 0.35), 0.95 * pose.alpha);
        let toes = 4;
        for k in 0..toes {
            let spread = 0.30 - 0.20 * k as f32;
            let dir = pose.f.rotate(spread * side);
            let tip = foot + dir * 4.6;
            draw_segment(pb, foot, tip, 1.4, pal.outline, 0.7 * pose.alpha);
            soft_dot(pb, tip, 0.9, (255, 244, 226), 0.75 * pose.alpha);
        }
        if !near {
            continue;
        }
        // The near hip crease: one dark line so the legs read on a big body.
        draw_segment(
            pb,
            pose.place(-6.0, side * 9.0),
            thigh,
            1.0,
            pal.outline,
            0.5 * pose.alpha,
        );
    }
}

/// The front legs. Planted in front of the chest; while it rears (`GnawStuck`,
/// `SqueakWindup`, `Call`) they come off the floor and paw at the air; while
/// it grooms one of them is at its face.
fn draw_front_legs(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    if pose.death > 0.75 {
        return;
    }
    let grooming = pose.groom >= 0.0 && pose.groom < 0.9;
    for &side in &[-1.0f32, 1.0] {
        let shoulder = pose.place(6.0, side * 7.5);
        let (elbow, paw) = if grooming && side < 0.0 {
            let face = pose.head + pose.p * (side * 2.5);
            let t = (pose.groom * 1.2).clamp(0.0, 1.0);
            (
                vlerp(pose.place(8.0, side * 6.0), face, 0.5 * t),
                vlerp(pose.place(11.0, side * 7.0), face, t),
            )
        } else if pose.rear > 0.45 {
            // Off the floor, hanging/pawing.
            let lift = (pose.rear - 0.45) / 0.55;
            (
                pose.place(8.0 + lift * 2.0, side * 7.0),
                pose.place(11.0 + lift * 4.0, side * (8.0 + lift * 2.0))
                    - Vec2::new(0.0, lift * 4.0),
            )
        } else {
            (
                pose.place(9.0, side * 8.5),
                pose.place(12.0, side * (9.5 + pose.crouch * 1.5)) + Vec2::new(0.0, 2.0),
            )
        };
        draw_segment(pb, shoulder, elbow, 4.4, pal.outline, 0.65 * pose.alpha);
        draw_segment(pb, shoulder, elbow, 3.0, mix(pal.shadow, pal.base, 0.6), 0.9 * pose.alpha);
        draw_segment(pb, elbow, paw, 3.2, pal.outline, 0.7 * pose.alpha);
        draw_segment(pb, elbow, paw, 2.1, pal.paw, 0.9 * pose.alpha);
        soft_dot(pb, paw, 2.3, pal.outline, 0.75 * pose.alpha);
        soft_dot(pb, paw, 1.6, mix(pal.paw, pal.shadow, 0.3), 0.95 * pose.alpha);
        for k in 0..4 {
            let spread = 0.32 - 0.21 * k as f32;
            let dir = if grooming { pose.p } else { pose.f };
            let tip = paw + dir.rotate(spread * side) * 3.4;
            draw_segment(pb, paw, tip, 1.0, pal.outline, 0.6 * pose.alpha);
            soft_dot(pb, tip, 0.7, (255, 246, 230), 0.6 * pose.alpha);
        }
    }
}

/// The body's silhouette as one signed distance, from the parts: haunch, back
/// hump, belly and neck. Kept as its own function so the head can carve its own
/// outline out of the neck and so the shading can tell the regions apart.
#[derive(Clone, Copy)]
struct Parts {
    haunch: f32,
    back: f32,
    belly: f32,
    neck: f32,
}

#[inline]
fn body_parts(pose: &Pose, u: f32, v: f32) -> Parts {
    let rear = pose.rear;
    let haunch = ellipse_sdf(u, v, -9.0, 1.5, 14.0, 11.5);
    // The back rises into a hump over the shoulders and drops to the neck.
    let back = sd_taper(u, v, (-12.0, -2.0), (4.0, -3.0), 9.5, 8.0);
    // The gut sags lower (and further out) while it rears up.
    let belly = ellipse_sdf(u, v, -2.0, 4.5 + rear * 1.5, 12.0, 6.8 + rear * 2.5);
    // A real pinch at the neck, so the head can read as its own mass.
    let neck = sd_taper(u, v, (2.0, -2.0), (8.0, -2.5 - rear * 2.5), 7.0, 5.6);
    Parts {
        haunch,
        back,
        belly,
        neck,
    }
}

/// The torso: one long matted slab, lit from the upper left, dark along the
/// spine and pale under the gut, with a combed fur grain and tufted edges.
fn draw_body(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body, 34.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let p = body_parts(pose, u, v);
            let d = smin(smin(p.haunch, p.back, 3.0), smin(p.belly, p.neck, 3.0), 3.5);
            if d > 0.0 {
                continue;
            }
            // The coat's own two-tone: a greasy dark back over a pale gut. The
            // ramp is the strongest read at this size, so it carries colour,
            // light and fur all at once.
            let dorsal_t = smoothstep(-v, -6.0, 3.0);
            let ventral_t = smoothstep(v, 2.0, 7.5);
            let mut fur = mix(pal.dorsal, pal.base, dorsal_t);
            fur = mix(fur, pal.belly, ventral_t * 0.55);
            let lum = ((x as f32 - pose.body.x) * LIGHT.x + (y as f32 - pose.body.y) * LIGHT.y) / 22.0;
            fur = shade3(pal.shadow, fur, pal.light, lum.clamp(-1.0, 1.0));
            // Ticking: a coarse matted mottle (a caged rat's coat is not smooth).
            let mottle = hash2((u * 0.42) as i32, (v * 0.42) as i32, 19);
            fur = mix(fur, pal.dorsal, (0.55 * mottle).min(0.55));
            let fine = hash2((u * 1.5) as i32, (v * 1.5) as i32, 37);
            fur = mix(fur, if fine > 0.5 { pal.light } else { pal.shadow }, 0.16);
            // Ribs while it stretches up on its hind legs.
            if pose.rear > 0.4 {
                let rib = ((u * 0.75 + 1.3).sin().abs() * pose.rear).powi(8);
                fur = mix(fur, pal.belly_shadow, 0.40 * rib);
            }
            // The rim is the outline; a body this big needs a heavy one.
            let edge = (-d / 3.4).clamp(0.0, 1.0);
            blend_px(pb, x, y, mix(pal.outline, apply_hit(fur, pose.hit), edge), pose.alpha);
        }
    }

    // Combed fur: short strands that follow the coat (backward along the spine,
    // splaying round the haunch), dark under the light ones. This is what makes
    // the body fur instead of a painted bean.
    let mut gy = y0;
    while gy <= y1 {
        let mut gx = x0;
        while gx <= x1 {
            let (u, v) = pose.local(gx, gy);
            let p = body_parts(pose, u, v);
            let d = smin(smin(p.haunch, p.back, 3.0), smin(p.belly, p.neck, 3.0), 3.5);
            if d > -1.2 {
                gx += 3;
                continue;
            }
            let grain = hash2(gx, gy, 71);
            // Flow: backward and out from the spine, so the coat lies down.
            let flow = (-pose.f * 0.75 + pose.p * (v.signum() * 0.55) + pose.f * (u / 30.0))
                .normalized();
            let len = 2.2 + 3.0 * grain;
            let from = Vec2::new(gx as f32 + 0.5, gy as f32 + 0.5);
            let (col, a) = if grain > 0.5 {
                (pal.light, 0.34)
            } else {
                (pal.dorsal, 0.42)
            };
            if pose.death < 0.9 {
                draw_segment(pb, from, from + flow * len, 0.8, col, a * pose.alpha);
            }
            gx += 3;
        }
        gy += 3;
    }

    // Creases: the haunch meets the back, the shoulder meets the neck. Two dark
    // arcs are what turn a bean into an animal.
    for (at_u, radius, alpha) in [(-4.0, 11.0, 0.34), (4.0, 8.0, 0.30)] {
        let centre = pose.place(at_u, 0.0);
        let seg = 16;
        for i in 0..seg {
            let a0 = -1.1 + 2.2 * i as f32 / seg as f32;
            let a1 = -1.1 + 2.2 * (i + 1) as f32 / seg as f32;
            let r = radius * 0.62;
            draw_segment(
                pb,
                centre + pose.f.rotate(a0) * r * 0.7 + pose.p * r,
                centre + pose.f.rotate(a1) * r * 0.7 + pose.p * r,
                1.2,
                pal.outline,
                alpha * pose.alpha,
            );
        }
    }
    // Ragged fur tufts on the silhouette: a dozen short spikes off the rim.
    let spikes = 16;
    for k in 0..spikes {
        let a = TAU * k as f32 / spikes as f32 + 0.17;
        let base = pose.place(-4.0 + a.cos() * 11.0, a.sin() * 9.5);
        let dir = Vec2::from_angle(a + 0.35);
        draw_segment(
            pb,
            base,
            base + dir * (2.5 + 2.0 * hash2(k, 5, 13)),
            0.9,
            pal.base,
            0.55 * pose.alpha,
        );
    }
    // Healed claw scars across the flank — this is a rat that has fought.
    for k in 0..3 {
        let a = -0.65 + k as f32 * 0.38;
        let from = pose.place(-9.0 + k as f32 * 3.5, -4.0);
        let to = from + pose.f.rotate(a) * 10.0;
        draw_segment(pb, from, to, 0.8, pal.scar, 0.55 * pose.alpha);
    }
    // Two pale patches where the coat has been rubbed bare by the collar.
    soft_dot(pb, pose.place(8.0, -3.0), 3.0, pal.belly, 0.22 * pose.alpha);
}

/// The head: a broad wedge skull, blunt muzzle, split incisors, one whole ear
/// and one torn, one milk-white eye and one live one, coarse whiskers, and the
/// scar across the muzzle that the facility left on its first success.
fn draw_head(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    let h = pose.head;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, h, 22.0) else {
        return;
    };
    let f = pose.f;
    let p = pose.p;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let at = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let d = at - h;
            let u = d.dot(f);
            let v = d.dot(p);
            let skull = ellipse_sdf(u, v, -0.8, 0.0, 9.0, 8.0);
            let muzzle = sd_taper(u, v, (2.0, 0.0), (11.5, 0.4), 6.0, 3.4);
            let cheek = ellipse_sdf(u, v, 2.0, 4.9, 4.6, 3.4);
            let cheek2 = ellipse_sdf(u, v, 2.0, -4.9, 4.6, 3.4);
            let dd = smin(smin(skull, muzzle, 3.0), smin(cheek, cheek2, 2.0), 2.5);
            // A dark skull shadow two pixels wider than the head: that ring is
            // what separates the head from the neck on a body this big.
            let halo = smin(
                ellipse_sdf(u, v, -0.8, 0.0, 10.2, 9.2),
                sd_taper(u, v, (2.0, 0.0), (11.5, 0.4), 7.0, 4.4),
                3.0,
            );
            if halo > 0.0 {
                continue;
            }
            if dd > 0.0 {
                let edge = (-halo / 1.6).clamp(0.0, 1.0);
                blend_px(pb, x, y, pal.outline, 0.85 * edge * pose.alpha);
                continue;
            }
            let lum = (v / 7.0) * 0.9 + 0.15;
            let mut fur = shade3(pal.shadow, pal.face, pal.light, lum.clamp(-1.0, 1.0));
            // Darker crown, pale muzzle.
            let muzzle_t = ((u - 1.0) / 6.0).clamp(0.0, 1.0);
            fur = mix(fur, pal.belly, 0.45 * muzzle_t);
            let streak = hash2((u * 0.9) as i32, (v * 0.9) as i32, 61);
            fur = mix(fur, pal.shadow, (streak * 0.18).min(0.18));
            let edge = (-dd / 2.0).clamp(0.0, 1.0);
            blend_px(pb, x, y, mix(pal.outline, apply_hit(fur, pose.hit), edge), pose.alpha);
        }
    }
    // Ears: big, round, one bitten through.
    for &side in &[-1.0f32, 1.0] {
        let base = h - f * 2.0 + p * (side * 6.0);
        let torn = side > 0.0;
        let tip = base - f * (3.0 + if torn { 0.5 } else { 4.0 }) + p * (side * 3.6);
        draw_segment(pb, base, tip, 5.4, pal.outline, 0.92 * pose.alpha);
        draw_segment(pb, base, tip, 4.0, mix(pal.ear_pink, pal.shadow, 0.25), 0.95 * pose.alpha);
        soft_dot(pb, vlerp(base, tip, 0.62), 1.6, pal.ear_inner, 0.8 * pose.alpha);
        if torn {
            let notch = vlerp(base, tip, 0.5);
            soft_dot(pb, notch, 1.9, pal.outline, 0.95 * pose.alpha);
            soft_dot(pb, notch + p * (side * 0.6), 1.3, pal.shadow, 0.9 * pose.alpha);
        }
    }
    // Eyes: the near one is the facility's first attempt (milk-white, blind);
    // the far one is alive and red in the last phase.
    let eye_near = h + f * 1.0 - p * 4.0;
    let eye_far = h + f * 0.4 + p * 3.6;
    soft_dot(pb, eye_near, 2.4, pal.outline, 0.85 * pose.alpha);
    soft_dot(pb, eye_near, 1.7, (206, 208, 210), 0.95 * pose.alpha);
    soft_dot(pb, eye_near + f * 0.5, 0.8, (255, 255, 255), 0.8 * pose.alpha);
    soft_dot(pb, eye_far, 2.2, pal.outline, 0.85 * pose.alpha);
    let glow = if pose.phase >= 3 {
        mix((255, 84, 72), (255, 214, 176), 0.35)
    } else {
        (255, 228, 196)
    };
    soft_dot(pb, eye_far, 1.5, pal.eye, 0.95 * pose.alpha);
    soft_dot(pb, eye_far + f * 0.5, 0.9, glow, 0.95 * pose.alpha);
    // A cut across the muzzle and one over the blind eye.
    draw_segment(
        pb,
        h + f * 3.0 - p * 4.5,
        h + f * 5.2 + p * 2.0,
        0.7,
        pal.scar,
        0.55 * pose.alpha,
    );
    draw_segment(
        pb,
        h - f * 1.0 - p * 5.4,
        h + f * 1.6 - p * 2.6,
        0.7,
        pal.scar,
        0.5 * pose.alpha,
    );
    // The nose: a blunt pink pad at the muzzle tip, split in two.
    let nose = h + f * 11.2 + p * 0.3;
    soft_dot(pb, nose, 2.4, pal.outline, 0.7 * pose.alpha);
    soft_dot(pb, nose, 1.9, pal.nose, 0.95 * pose.alpha);
    soft_dot(pb, nose + p * 0.9, 0.7, (255, 206, 214), 0.6 * pose.alpha);
    soft_dot(pb, nose - p * 0.9, 0.7, (255, 206, 214), 0.6 * pose.alpha);
    // Whiskers: six long coarse hairs, asymmetric as a real rat's.
    for &side in &[-1.0f32, 1.0] {
        for k in 0..3 {
            let root = h + f * (6.4 + k as f32 * 1.3) + p * (side * 2.4);
            let spread = 0.45 + k as f32 * 0.22;
            let len = 14.0 + k as f32 * 4.0 - if side > 0.0 { 2.0 } else { 0.0 };
            let dir = (f + p * (side * spread)).normalized();
            let tip = root + dir * len + Vec2::new(0.0, 1.6);
            draw_segment(pb, root, tip, 0.7, (236, 230, 218), 0.38 * pose.alpha);
        }
    }
    // Incisors: two curved, orange-fronted teeth, always showing — longer the
    // wider the jaw is open.
    let jaw_open = pose.jaw.clamp(0.0, 1.0);
    let drop = 0.6 + 1.4 * jaw_open;
    for &side in &[-1.0f32, 1.0] {
        let root = h + f * 9.0 + p * (side * 1.6) + Vec2::new(0.0, drop * 0.4);
        let len = 3.2 + 5.0 * jaw_open;
        let dir = (f * 0.45 + p * (side * 0.30) + Vec2::new(0.0, 0.82)).normalized();
        let tip = root + dir * len;
        draw_segment(pb, root, tip, 2.2, pal.outline, 0.85 * pose.alpha);
        draw_segment(pb, root, tip, 1.3, pal.incisor, 0.98 * pose.alpha);
        soft_dot(pb, tip, 0.9, (255, 255, 246), 0.9 * pose.alpha);
        // The orange front a rat's incisor wears.
        soft_dot(pb, root + dir * (len * 0.35), 0.8, pal.tooth, 0.55 * pose.alpha);
        if jaw_open > 0.65 {
            soft_dot(pb, tip + Vec2::new(0.0, 1.6), 0.7, (206, 236, 244), 0.45 * pose.alpha);
        }
    }
    // The mouth line between the incisors.
    draw_segment(
        pb,
        h + f * 8.0 - p * 1.9 + Vec2::new(0.0, drop * 0.5),
        h + f * 8.0 + p * 1.9 + Vec2::new(0.0, drop * 0.5),
        0.8,
        pal.outline,
        0.6 * pose.alpha,
    );
}

/// The collar: a wide steel band at the neck with the facility's tag hanging
/// under the chin, and a chain of two links. It glints in phase 2+.
fn draw_collar(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    let neck = pose.place(5.0, 0.0);
    let half = 6.6;
    let steps = 13;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let v = -half + 2.0 * half * t;
        let at = neck + pose.p * v - pose.f * (v.abs() * 0.16);
        soft_dot(pb, at, 1.9, pal.outline, 0.85 * pose.alpha);
        let steel = if pose.phase >= 2 {
            mix(pal.collar, (255, 252, 240), 0.22 + 0.22 * (pose.anim * 6.0).sin().abs())
        } else {
            pal.collar
        };
        soft_dot(pb, at, 1.35, steel, 0.95 * pose.alpha);
        soft_dot(pb, at - pose.p * 0.0 + Vec2::new(0.0, -0.4), 0.6, (238, 244, 250), 0.55 * pose.alpha);
    }
    // The tag: a brass plate hanging from two links, swinging with the body.
    let swing = (pose.anim * 3.1).sin() * 1.4;
    let hang = neck + pose.f * 1.0 + Vec2::new(swing, 4.4);
    draw_segment(pb, neck + pose.f * 0.6, hang, 0.9, pal.collar, 0.8 * pose.alpha);
    soft_dot(pb, hang, 2.8, pal.outline, 0.85 * pose.alpha);
    soft_dot(pb, hang, 2.0, pal.tag, 0.97 * pose.alpha);
    soft_dot(pb, hang + Vec2::new(-0.6, -0.7), 0.8, (255, 244, 198), 0.8 * pose.alpha);
    soft_dot(pb, hang + Vec2::new(0.7, 0.9), 0.6, (122, 96, 40), 0.65 * pose.alpha);
}

/// The exposed belly: warm, bright, pulsing — the "hit me" of the fight.
fn draw_belly_glint(pb: &mut PixelBuffer, pose: &Pose) {
    let at = pose.place(BELLY_OFFSET, 0.0);
    let pulse = 0.5 + 0.5 * (pose.anim * 10.0).sin();
    // A pale under-belly wash.
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, at, BELLY_RADIUS + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let d = ellipse_sdf(u, v, BELLY_OFFSET, 0.0, BELLY_RADIUS, BELLY_RADIUS * 0.85);
            if d > 0.0 {
                continue;
            }
            let t = (1.0 + d / BELLY_RADIUS).clamp(0.0, 1.0);
            blend_px(pb, x, y, (236, 186, 170), 0.42 * t);
            blend_px(pb, x, y, (255, 226, 196), 0.18 * t * t);
        }
    }
    // The ring marker and a chevron: unmistakable.
    let r = BELLY_RADIUS + 2.0;
    let seg = 18;
    for i in 0..seg {
        let a0 = TAU * i as f32 / seg as f32;
        let a1 = TAU * (i + 1) as f32 / seg as f32;
        draw_segment(
            pb,
            at + Vec2::from_angle(a0) * r,
            at + Vec2::from_angle(a1) * r,
            1.0,
            (255, 236, 180),
            0.4 + 0.3 * pulse,
        );
    }
    soft_dot(pb, at, 2.2, (255, 250, 226), 0.5 + 0.35 * pulse);
}

/// The exposed tail base: the scarred stub where the tail meets the fur.
fn draw_tail_base_glint(pb: &mut PixelBuffer, pose: &Pose) {
    let at = pose.place(-TAIL_BASE_OFFSET + HAUNCH_OFFSET, 0.0);
    let pulse = 0.5 + 0.5 * (pose.anim * 10.0).sin();
    soft_dot(pb, at, TAIL_BASE_RADIUS + 1.0, (255, 214, 168), 0.20 + 0.14 * pulse);
    soft_dot(pb, at, TAIL_BASE_RADIUS * 0.55, (255, 240, 206), 0.28 + 0.18 * pulse);
    let r = TAIL_BASE_RADIUS + 2.0;
    let seg = 16;
    for i in 0..seg {
        let a0 = TAU * i as f32 / seg as f32;
        let a1 = TAU * (i + 1) as f32 / seg as f32;
        draw_segment(
            pb,
            at + Vec2::from_angle(a0) * r,
            at + Vec2::from_angle(a1) * r,
            1.0,
            (255, 236, 180),
            0.35 + 0.3 * pulse,
        );
    }
}

/// The call's breath: steam out of the open mouth, and the spit of a shout.
fn draw_call_breath(pb: &mut PixelBuffer, pose: &Pose) {
    let mouth = pose.head + pose.f * 6.0;
    for k in 0..4 {
        let t = k as f32 / 4.0;
        let rise = (pose.anim * 1.6 + t).fract();
        let at = mouth + Vec2::new(0.0, -rise * 12.0) + pose.p * ((t - 0.5) * 5.0);
        soft_dot(pb, at, 2.0 + rise * 2.2, (228, 226, 220), 0.20 * (1.0 - rise));
    }
}

/// The grooming motes: dust and loose fur coming off the coat.
fn draw_groom_motes(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    if pose.groom < 0.0 {
        return;
    }
    for k in 0..5 {
        let t = (pose.anim * 0.9 + k as f32 * 0.21).fract();
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let at = pose.body
            + pose.p * (side * (8.0 + t * 10.0))
            + Vec2::new(0.0, -t * 8.0);
        soft_dot(pb, at, 1.1, pal.light, 0.30 * (1.0 - t));
    }
}

/// The death: a last long tail sweep, the paw letting go.
fn draw_death_tell_tale(pb: &mut PixelBuffer, pose: &Pose, pal: &Coat) {
    // Blood in the fur and a slack paw.
    let at = pose.head + pose.p * 4.0;
    soft_dot(pb, at, 3.2, (150, 34, 38), 0.35 * pose.alpha);
    soft_dot(pb, pose.place(-6.0, 12.0), 2.6, pal.paw, 0.5 * pose.alpha);
}

/// Phase 3: the heat coming off it (the mirror of a fully-levelled rat).
fn draw_phase3_heat(pb: &mut PixelBuffer, pose: &Pose) {
    let pulse = 0.5 + 0.5 * (pose.anim * 5.0).sin();
    for k in 0..3 {
        let t = k as f32 / 3.0;
        let at = pose.body + pose.f * ((t - 0.5) * 24.0) + Vec2::new(0.0, -6.0 - t * 3.0);
        soft_dot(pb, at, 5.0, (255, 120, 90), 0.05 + 0.05 * pulse);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// The two passes must tile the marked arc exactly, or the fight lies about
    /// where the tail is.
    #[test]
    fn the_two_thrash_passes_tile_the_marked_arc() {
        let mut view = MegaRatView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(40.0, 0.0));
        view.state = MegaRatState::ThrashPass1;
        let (centre, inner, outer, facing, half) = thrash_arc(&view);
        assert_eq!(centre, view.pos);
        assert!((inner - THRASH_INNER).abs() < 1e-6 && (outer - THRASH_OUTER).abs() < 1e-6);
        for side in [1.0f32, -1.0] {
            let (dir, h) = thrash_half(&view, side);
            assert!((h * 2.0 - half).abs() < 1e-6, "each pass covers half the arc");
            // The half's outer edge (the side away from the facing) must reach
            // the marked arc's edge, and its inner edge the facing itself.
            let edge = dir.rotate(side * h).angle() - facing.angle();
            let seam = dir.rotate(-side * h).angle() - facing.angle();
            assert!(seam.abs() < 1e-5, "pass {side} leaves a seam of {seam} at the facing");
            assert!(
                (edge.abs() - half).abs() < 1e-5,
                "pass {side} reaches {edge} but the arc is {half}"
            );
        }
    }

    /// The safe wedge is the *only* part of the ring the squeak leaves open,
    /// and it is behind the boss.
    #[test]
    fn the_squeak_ring_leaves_exactly_the_wedge_behind_it_open() {
        let view = MegaRatView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(40.0, 0.0));
        let (centre, inner, outer, wedge_dir, wedge) = squeak_ring(&view);
        assert_eq!(centre, view.pos);
        assert!(inner < outer);
        // Behind is open, in front is not.
        let behind = -Vec2::new(1.0, 0.0);
        assert!((wedge_dir - behind).len() < 1e-6);
        assert!(in_sector(behind, wedge_dir, wedge));
        assert!(!in_sector(Vec2::new(1.0, 0.0), wedge_dir, wedge));
        // The live band's mid-radius is inside the drawn band.
        let mid = (inner + outer) * 0.5;
        assert!(mid > SQUEAK_INNER && mid < SQUEAK_OUTER);
    }

    /// The weak point is somewhere a rat standing in front of / behind the boss
    /// can actually reach, and it is only there in the two recoveries.
    #[test]
    fn the_weak_point_is_exposed_only_in_the_two_recoveries() {
        let mut view = MegaRatView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(40.0, 0.0));
        let states = [
            MegaRatState::Groom,
            MegaRatState::GnawWindup,
            MegaRatState::GnawLunge,
            MegaRatState::GnawStuck,
            MegaRatState::ThrashWindup,
            MegaRatState::ThrashPass1,
            MegaRatState::ThrashGap,
            MegaRatState::ThrashPass2,
            MegaRatState::ThrashRecover,
            MegaRatState::SqueakWindup,
            MegaRatState::SqueakShout,
            MegaRatState::DashWindup,
            MegaRatState::DashPass,
            MegaRatState::DashSkid,
            MegaRatState::Call,
            MegaRatState::Death,
        ];
        for state in states {
            view.state = state;
            let weak = weak_point(&view);
            assert_eq!(
                weak.is_some(),
                state.is_recovery(),
                "{state:?}: weak point exposure must be exactly the recoveries"
            );
            if let Some((at, r)) = weak {
                assert!(r > 4.0);
                // The weak point is a body-length from the centre at most: a
                // rat that walked in can reach it.
                assert!(at.dist(view.pos) < BODY_RADIUS + 10.0, "{state:?}: {at:?}");
            }
        }
    }

    /// Body contact is off exactly in the windows the art says it is.
    #[test]
    fn body_contact_is_off_in_every_window_the_art_names() {
        for state in [
            MegaRatState::Groom,
            MegaRatState::GnawWindup,
            MegaRatState::ThrashWindup,
            MegaRatState::SqueakWindup,
            MegaRatState::DashWindup,
            MegaRatState::DashSkid,
            MegaRatState::ThrashGap,
        ] {
            assert!(has_body_contact(state), "{state:?} should bump the rat");
        }
        for state in [
            MegaRatState::GnawLunge,
            MegaRatState::GnawStuck,
            MegaRatState::ThrashPass1,
            MegaRatState::ThrashPass2,
            MegaRatState::ThrashRecover,
            MegaRatState::SqueakShout,
            MegaRatState::DashPass,
            MegaRatState::Call,
            MegaRatState::Death,
        ] {
            assert!(!has_body_contact(state), "{state:?} must not bump the rat");
        }
    }

    /// The punish windows are the only ×damage states and the death is inert.
    #[test]
    fn the_incoming_multiplier_is_the_recovery_and_nothing_else() {
        assert!(incoming_damage_mult(MegaRatState::GnawStuck) > 1.0);
        assert!(incoming_damage_mult(MegaRatState::ThrashRecover) > 1.0);
        assert_eq!(incoming_damage_mult(MegaRatState::Death), 0.0);
        for state in [
            MegaRatState::Groom,
            MegaRatState::GnawWindup,
            MegaRatState::GnawLunge,
            MegaRatState::ThrashWindup,
            MegaRatState::ThrashPass1,
            MegaRatState::ThrashGap,
            MegaRatState::ThrashPass2,
            MegaRatState::SqueakWindup,
            MegaRatState::SqueakShout,
            MegaRatState::DashWindup,
            MegaRatState::DashPass,
            MegaRatState::DashSkid,
            MegaRatState::Call,
        ] {
            assert_eq!(incoming_damage_mult(state), 1.0, "{state:?}");
        }
    }

    /// The strike point is what the rat must leave: the teeth line ends on it
    /// and the head is parked on it while the teeth are in the floor.
    #[test]
    fn the_bite_point_is_where_the_head_and_the_line_end() {
        let mut view = MegaRatView::new(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(64.0, 0.0));
        view.state = MegaRatState::GnawWindup;
        let (from, to) = gnaw_line(&view);
        assert!((to - view.target).len() < 1e-6);
        assert!(from.x > view.pos.x, "the line starts at the muzzle");
        view.state = MegaRatState::GnawStuck;
        assert!((head_center(&view) - view.target).len() < 1e-6);
    }

    // ---- development captures (inert unless the env var is set) ------------

    fn write_ppm(path: &str, pb: &PixelBuffer) {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for p in pb.pixels() {
            out.push(p.0);
            out.push(p.1);
            out.push(p.2);
        }
        let _ = std::fs::write(path, out);
    }

    /// Development aid: `RATRUN_MEGARAT_WORLD=1 cargo test megarat_world`
    /// renders the real Kitchen through `World::draw` with R-01 in eight poses
    /// (every telegraph and strike, both phase flushes, both weak points and the
    /// death), so the integration — tile, lighting, camera, telegraphs — can be
    /// looked at and not just the sprite. Inert unless the variable is set.
    #[test]
    fn megarat_world() {
        if std::env::var("RATRUN_MEGARAT_WORLD").is_err() {
            return;
        }
        use crate::game::ecs::components::SpawnId;
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        use crate::game::level::{Theme, TileRect};
        use crate::game::world::World;

        const VIEW: (usize, usize) = (160, 168);
        let mut sheet = PixelBuffer::new(VIEW.0 * 2, VIEW.1 * 4);
        let poses = [
            (MegaRatState::GnawWindup, 0.85f32, 1u8),
            (MegaRatState::GnawStuck, 0.45, 1),
            (MegaRatState::ThrashWindup, 0.90, 2),
            (MegaRatState::ThrashPass1, 0.40, 2),
            (MegaRatState::SqueakShout, 0.20, 3),
            (MegaRatState::DashWindup, 0.90, 3),
            (MegaRatState::Call, 0.55, 2),
            (MegaRatState::Death, 0.70, 1),
        ];
        // A seed whose Kitchen pool pick is the Megarat.
        let mut seed = 3u64;
        while BossKind::for_theme_seeded(Theme::Kitchen, seed) != BossKind::MegaRat {
            seed += 1;
        }
        for (i, (state, prog, phase)) in poses.iter().enumerate() {
            let mut w = World::new(Theme::Kitchen, seed);
            w.lighting = crate::render::lighting::LightingMode::Full;
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .expect("the Kitchen's boss room");
            let centre = arena.center_world();
            w.player.pos = centre - Vec2::new(34.0, 0.0);
            // One draw so the camera settles on the rat before the boss exists.
            let mut warm = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut warm);
            let pos = centre + Vec2::new(16.0, -6.0);
            let mut boss = Boss::new(
                BossKind::MegaRat,
                pos,
                BOSS_BASE_HP,
                TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows()),
                centre,
                false,
            );
            boss.state = crate::game::ecs::systems::boss::BossState::MegaRat(*state);
            boss.duration = 1.0;
            boss.elapsed = *prog;
            boss.phase = *phase;
            boss.anim = 1.7;
            boss.facing = Vec2::new(0.94, 0.34);
            boss.target = pos + boss.facing * 54.0;
            boss.pounce_origin = pos;
            boss.scratch[1] = if *state == MegaRatState::DashWindup {
                DASH_COOLDOWN * 0.5
            } else {
                0.0
            };
            boss.scratch[4] = if *state == MegaRatState::GnawStuck {
                1.0
            } else {
                0.0
            };
            w.ecs.spawn((boss, SpawnId(9_000)));
            let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
            w.draw(&mut pb);
            for y in 0..VIEW.1 {
                for x in 0..VIEW.0 {
                    sheet.plot(
                        ((i % 2) * VIEW.0 + x) as i32,
                        ((i / 2) * VIEW.1 + y) as i32,
                        pb.pixel_at(x, y),
                    );
                }
            }
        }
        write_ppm("/tmp/ratrun_megarat_world.ppm", &sheet);
    }

    /// `RATRUN_MEGARAT_SHEET=1 cargo test megarat_sheet` writes a contact sheet
    /// of every state (three progresses each) to
    /// `/tmp/ratrun_megarat_sheet.ppm`.
    #[test]
    fn megarat_sheet() {
        if std::env::var("RATRUN_MEGARAT_SHEET").is_err() {
            return;
        }
        const CELL_W: usize = 190;
        const CELL_H: usize = 140;
        let states = [
            MegaRatState::Groom,
            MegaRatState::GnawWindup,
            MegaRatState::GnawLunge,
            MegaRatState::GnawStuck,
            MegaRatState::ThrashWindup,
            MegaRatState::ThrashPass1,
            MegaRatState::ThrashGap,
            MegaRatState::ThrashPass2,
            MegaRatState::ThrashRecover,
            MegaRatState::SqueakWindup,
            MegaRatState::SqueakShout,
            MegaRatState::DashWindup,
            MegaRatState::DashPass,
            MegaRatState::DashSkid,
            MegaRatState::Call,
            MegaRatState::Death,
        ];
        let cols = 3usize;
        let mut sheet = PixelBuffer::new(CELL_W * cols, CELL_H * states.len());
        for (i, state) in states.iter().enumerate() {
            for (j, prog) in [0.15f32, 0.55, 0.90].iter().enumerate() {
                let mut cell = PixelBuffer::new(CELL_W, CELL_H);
                // Kitchen tile: cream checks with grout.
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        let tile = ((x / 16) + (y / 16)) % 2 == 0;
                        let grain = hash2((x / 4) as i32, (y / 4) as i32, 7);
                        let base = if tile { (86, 92, 88) } else { (72, 78, 76) };
                        let c = mix(base, (110, 118, 112), grain * 0.4);
                        cell.plot(x as i32, y as i32, c);
                    }
                }
                let mut view = MegaRatView::new(
                    Vec2::new(62.0, 74.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(134.0, 74.0),
                );
                view.state = *state;
                view.progress = *prog;
                view.anim = 0.7 + prog * 4.0;
                view.phase = match state {
                    MegaRatState::Call => 2,
                    MegaRatState::SqueakShout | MegaRatState::DashPass => 3,
                    _ => 1,
                };
                view.hp_frac = 1.0 - *prog * 0.4;
                view.lane_start = Vec2::new(40.0, 74.0);
                view.sweep = 1.0;
                view.dash_cd = if *state == MegaRatState::DashSkid {
                    0.9
                } else {
                    0.0
                };
                view.walled = *state == MegaRatState::GnawStuck && *prog > 0.5;
                draw_megarat_ground(&mut cell, &view, Vec2::ZERO, (206, 150, 70));
                draw_megarat(&mut cell, &view, Vec2::ZERO, (206, 150, 70));
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        sheet.plot(
                            (j * CELL_W + x) as i32,
                            (i * CELL_H + y) as i32,
                            cell.pixel_at(x, y),
                        );
                    }
                }
            }
        }
        write_ppm("/tmp/ratrun_megarat_sheet.ppm", &sheet);
    }
}
