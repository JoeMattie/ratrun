//! Art and gameplay specification for the Lab's second fight — **KING COBRA**.
//!
//! The Lab's first boss is a machine that mows a lawn indoors. Its pool mate is
//! the other thing a facility like this keeps behind glass: an escaped
//! specimen, and far longer than the room it was kept in. Where the
//! Turf-Reaper is steel and lidar, this is a body — twenty feet of it, and the
//! part that matters is the part that is *around* you.
//!
//! # The fight the level was built for
//!
//! The mower teaches the value of a miss. The cobra teaches **where you are
//! standing**: its coils are the arena's moving walls, so every one of its big
//! moves is a shape you have to already be outside of. Each of them is marked,
//! each of them is one hit, and each of them leaves the animal somewhere it
//! cannot defend.
//!
//! | phase | hp | rotation | read |
//! |---|---|---|---|
//! | 1 | `> 0.66` | `Rear` → `HoodTell` → `Strike` → `FangsStuck` | step off the marked line; bite the open hood |
//! | | | `Rear` → `ConstrictTell` → `Constrict` → `Loosen` | the loop tightens on a marked clock: leave through the gap, or dash over the coil |
//! | | | `Rear` → `VenomTell` → `VenomSpit` | leave the three marked circles; the puddles left behind are slow and foul |
//! | | | `Rear` → `TailTell` → `TailSweep` → `TailRecover` | the whip takes the floor *in front* of the head: be behind it, or inside [`TAIL_INNER`] px of it |
//! | 2 | `0.33..=0.66` | adds `SwayTell` (the hypnotic beat) and a second strike | the strike follows the sway's beat, so it can be read and baited |
//! | 3 | `<= 0.33` | adds `SpiralCoil` → `SpiralChase` → `SpiralEnd` | run the lane while the head comes down it; it is spent at the end of the run |
//!
//! ## The hood and the strike
//!
//! `HoodTell` flares the hood — the two dark chevrons and the pale spectacle
//! mark on its back come up — and paints a line from the fangs to a marked
//! point. The line **freezes** at [`HOOD_LOCK_PROGRESS`], so the answer is
//! always "step off the line you can see". `Strike` drives the head down it for
//! [`STRIKE_DAMAGE`] once, and then the fangs are **in the floor**:
//! [`CobraState::FangsStuck`] is [`STUCK_DURATION`] s of no body contact,
//! ×[`STUCK_DAMAGE_MULT`] incoming damage and the open hood's back — the
//! [spectacle](weak_point) — as the fight's one clear weak point. From phase 2
//! the animal does not wait for the rat to punish it: `FangsStuck` is followed
//! by [`CobraState::ReStrikeTell`] and a second, shorter strike.
//!
//! ## The constriction
//!
//! The body is a fixed-length chain ([`COIL_MAX`] points) and the animal can
//! put all of it on the floor at once. `ConstrictTell` lays the loop around
//! where the rat is standing at the start radius [`COIL_START_R`]; `Constrict`
//! spends [`COIL_TIGHTEN`] s pulling it in to [`COIL_END_R`] on a smooth,
//! marked clock. The loop is a **moving wall** ([`coil_chain`]: the boss's own
//! separation pass pushes a *walking* rat out of it, a dashing one passes
//! over), it has exactly one opening ([`coil_gap_angle`], [`COIL_GAP_ARC`] px
//! wide at every radius, turning [`COIL_TURN`] rad as it tightens), and when it
//! closes the whole inside takes [`CONSTRICT_DAMAGE`] once. Leaving through the
//! gap is the honest answer; dashing the coil costs a dash and nothing else.
//!
//! ## Venom
//!
//! `VenomTell` marks the circles — the rat's, and two either side of it — and
//! `VenomSpit` lobs into them. Each landing is a splash ([`VENOM_SPLASH`]
//! once), then the puddle that stays: [`VENOM_POOL_LIFE`] s of [`VENOM_SLOW`]
//! of the rat's own walk, and one [`VENOM_DRIP`] bite at
//! [`VENOM_DRIP_DELAY`]. The puddles are the boss's own hazards, so they stop
//! with it.
//!
//! ## The sway
//!
//! From phase 2 the head sways on a three-beat rhythm ([`sway_phase`]) and the
//! strike is **the next beat**: the aim does not lock until
//! [`SWAY_LOCK_PROGRESS`] of [`SWAY_TELL`], so a rat that watches the rhythm —
//! and listens to the hood's dry rattle on each beat — can bait the strike
//! into the floor it just left.
//!
//! ## The spiral
//!
//! In its last third the animal stops fighting the rat and *furnishes the
//! room*: `SpiralCoil` lays the whole body down as one archimedean lane
//! ([`spiral_chain`], [`SPIRAL_TURN`] px between turns), `SpiralChase` comes
//! down it at [`SPIRAL_SPEED`] px/s, and `SpiralEnd` drops the coils and leaves
//! the head spent and open (×[`SPENT_DAMAGE_MULT`]). The lane's outer end is
//! open ([`SPIRAL_OPEN`] rad of the last turn), so the run always has an exit.
//!
//! # Layering contract for the renderer
//!
//! 1. **Ground layer** (after the floor and stains, before props/entities):
//!    [`draw_ground`] — the body's shadow, the resting coils' contact shade,
//!    the venom puddles, the marked strike line and its fang-cross, the
//!    constriction's inner clock ring and its gap arrows, and the sway's ghost
//!    heads.
//! 2. **Entity layer** (with the other bodies): [`draw`] — the neck, the body
//!    chain (resting loops or the laid coil/spiral), the hood, the head, the
//!    eyes, the fangs and the death sag.
//!
//! Everything culls per point, so a coil whose head is on screen costs only
//! the part of the body that is on screen. No art in this file allocates, and
//! none of it draws rng.

use crate::game::level::TileRect;
use crate::math::{lerp, Vec2};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Body, hits, pools
// ============================================================================

/// Framework movement/contact circle (world px). The head is the body: the
/// animal's mass is the coil, and the coil is not a walkable thing.
pub const BODY_RADIUS: f32 = 13.5;

/// The head's painted half-width at the jaw hinge.
pub const HEAD_RADIUS: f32 = 9.5;

/// The neck's radius where it leaves the hood.
pub const NECK_RADIUS: f32 = 7.0;

/// The flared hood's half-width when fully open.
pub const HOOD_RADIUS: f32 = 13.5;

/// The stance: the hood's centre sits this far behind the head's centre.
pub const HOOD_SETBACK: f32 = 4.0;

/// The spectacle mark's hit circle (the weak point; see [`weak_point`]).
pub const HOOD_WEAK_RADIUS: f32 = 8.5;

/// The tip of the fangs, forward of the head's centre.
pub const FANG_REACH: f32 = 10.0;

/// Longest sprite reach for culling. Large on purpose: the drawn body can be a
/// whole arena wide, and every part of it culls itself (see [`draw`]).
pub const CULL_MARGIN: f32 = 170.0;

/// The art's recommended base pool, before [`super::BossKind::hp_scale`] and
/// the director's boss curve.
///
/// The Lab is a stage-three floor and this is its second, later fight: a
/// near-complete build lands 250-600 dps into a boss (see
/// `BossKind::hp_scale`), and this one spends a real share of every rotation in
/// a 1.5x punish window, so the raw pool is a little over the Mower's 1,600.
/// `1800 x 2.0` lands the intended 45-80 s beat; a balance pass owns the
/// number.
pub const BOSS_BASE_HP: f32 = 1800.0;

/// Body contact outside recovery (the owner's rule: 8).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 opens here (the sway starts).
pub const PHASE2_HP_FRAC: f32 = 0.66;

/// Phase 3 opens here (it furnishes the room with itself).
pub const PHASE3_HP_FRAC: f32 = 0.33;

/// Critical shimmer on the spectacle mark (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

/// Incoming damage while the fangs are in the floor.
pub const STUCK_DAMAGE_MULT: f32 = 1.5;
/// Incoming damage while the laid spiral drops and the head is spent.
pub const SPENT_DAMAGE_MULT: f32 = 1.4;

/// The death beat: the body unspools.
pub const DEATH_DURATION: f32 = 2.60;

/// The harmless pose change at every phase threshold.
pub const RISE_DURATION: f32 = 1.20;

// ============================================================================
// Rear (the neutral beat)
// ============================================================================

/// `Rear` duration per phase: the head stays up and works the air.
pub const REAR_DURATION: [f32; 3] = [1.10, 1.00, 0.90];
/// `Rear` walk speed per phase (px/s).
pub const REAR_SPEED: [f32; 3] = [42.0, 48.0, 54.0];
/// The stand-off band it holds while it decides (`min`, `max` px).
pub const REAR_BAND: [f32; 2] = [70.0, 118.0];
/// Speed multiplier while the rat has left the arena.
pub const HUNT_SPEED_MULT: f32 = 1.35;

// ============================================================================
// The hood flare and the strike
// ============================================================================

/// `HoodTell` duration per phase.
pub const HOOD_TELL: [f32; 3] = [0.85, 0.75, 0.65];
/// Progress through the tell where the strike line freezes.
pub const HOOD_LOCK_PROGRESS: f32 = 0.55;
/// `Strike`: how long the head takes to come down.
pub const STRIKE_DURATION: f32 = 0.26;
/// `Strike`: how long the line is live inside it.
pub const STRIKE_ACTIVE: f32 = 0.16;
/// `Strike` damage, once.
pub const STRIKE_DAMAGE: f32 = 20.0;
/// Half-width of the live line.
pub const STRIKE_HALF_WIDTH: f32 = 5.0;
/// The line's shove.
pub const STRIKE_KNOCKBACK: f32 = 120.0;
/// Longest the fangs reach.
pub const STRIKE_REACH: f32 = 92.0;
/// Shortest strike: closer than this and it bites down at its own feet.
pub const STRIKE_MIN: f32 = 22.0;
/// How fast the head travels down the frozen line (px/s).
pub const STRIKE_SPEED: f32 = 520.0;
/// How far ahead of the fangs the "is there a wall in the mouth" probe sits.
pub const FANG_WALL_PROBE: f32 = 10.0;
/// A clear step off the line, for the suppression test.
pub const STRIKE_ESCAPE: f32 = 18.0;

/// `FangsStuck` duration per phase: the punish window.
pub const STUCK_DURATION: [f32; 3] = [1.10, 1.00, 0.90];
/// …and when the fangs went into a wall instead of the floor.
pub const STUCK_WALLED: f32 = 1.55;

/// The second strike's (shorter) tell, phase 2+.
pub const RESTRIKE_TELL: [f32; 3] = [0.60, 0.55, 0.50];
/// Progress through the second tell where *its* line freezes.
pub const RESTRIKE_LOCK_PROGRESS: f32 = 0.55;

// ============================================================================
// The sway (phase 2+)
// ============================================================================

/// `SwayTell` duration per phase: three beats, then the strike.
pub const SWAY_TELL: [f32; 3] = [1.70, 1.60, 1.50];
/// Beats per sway. The strike lands on the beat after the last one.
pub const SWAY_BEATS: f32 = 3.0;
/// Progress through the sway where the aim finally locks (`0..=1`).
pub const SWAY_LOCK_PROGRESS: f32 = 0.78;

// ============================================================================
// Venom
// ============================================================================

/// `VenomTell` duration per phase.
pub const VENOM_TELL: [f32; 3] = [0.80, 0.72, 0.64];
/// Progress through the tell where the circles commit.
pub const VENOM_LOCK_PROGRESS: f32 = 0.55;
/// The lob itself: the head throws, the puddles land.
pub const VENOM_SPIT: f32 = 0.22;
/// How many circles are marked.
pub const VENOM_MARKS: usize = 3;
/// Radius of one landing circle.
pub const VENOM_RADIUS: f32 = 13.0;
/// The landing splash's damage, once.
pub const VENOM_SPLASH: f32 = 14.0;
/// The puddle's single drip.
pub const VENOM_DRIP: f32 = 8.0;
/// Seconds after the landing that the drip bites.
pub const VENOM_DRIP_DELAY: f32 = 1.05;
/// How long a puddle stays foul.
pub const VENOM_POOL_LIFE: f32 = 3.60;
/// Share of the rat's own walk a puddle takes.
pub const VENOM_SLOW: f32 = 0.45;
/// The two flanking circles' distance either side of the rat's.
pub const VENOM_SPREAD: f32 = 24.0;
/// Puddles the view carries for the art (the hazard ring holds the rest).
pub const POOL_VIEW: usize = 3;

// ============================================================================
// The tail whip
// ============================================================================

/// `TailTell` duration per phase.
pub const TAIL_TELL: [f32; 3] = [0.80, 0.72, 0.64];
/// Progress through the tell where the arc freezes.
pub const TAIL_LOCK_PROGRESS: f32 = 0.55;
/// The sweep itself.
pub const TAIL_SWEEP: f32 = 0.34;
/// The off-balance beat after it.
pub const TAIL_RECOVER: [f32; 3] = [0.60, 0.55, 0.50];
/// The whip's dead zone: inside this of the head is safe.
pub const TAIL_INNER: f32 = 12.0;
/// How far the whip reaches.
pub const TAIL_OUTER: f32 = 52.0;
/// Half the swept angle (the whip takes the floor the head is looking at).
pub const TAIL_HALF_ANGLE: f32 = 1.31;
/// One hit.
pub const TAIL_DAMAGE: f32 = 14.0;
/// The whip throws what it catches.
pub const TAIL_KNOCKBACK: f32 = 170.0;
/// A clear step out of the arc, for the suppression test.
pub const TAIL_ESCAPE: f32 = 22.0;

// ============================================================================
// The constriction
// ============================================================================

/// `ConstrictTell` duration per phase: the loop is laid and read.
pub const COIL_TELL: [f32; 3] = [0.95, 0.90, 0.85];
/// `Constrict`: how long the loop takes to close, per phase.
pub const COIL_TIGHTEN: [f32; 3] = [2.40, 2.10, 1.80];
/// The beat where it lets go and the head comes off the coil.
pub const COIL_LOOSEN: f32 = 0.70;
/// The loop's radius when it is laid.
pub const COIL_START_R: f32 = 74.0;
/// …and when it has closed.
pub const COIL_END_R: f32 = 26.0;
/// The coil's own half-thickness (the wall the rat cannot walk through).
pub const COIL_BODY_HALF: f32 = 4.5;
/// Distance between two body points in the chain.
pub const COIL_SPACING: f32 = 6.0;
/// The opening's width in px at *every* radius, so it never becomes a gap the
/// rat does not fit through.
pub const COIL_GAP_ARC: f32 = 30.0;
/// How far the opening turns while the loop closes.
pub const COIL_TURN: f32 = 1.40;
/// What a rat still inside takes when the loop closes, once.
pub const CONSTRICT_DAMAGE: f32 = 18.0;
/// The squeeze's shove.
pub const CONSTRICT_KNOCKBACK: f32 = 90.0;
/// Bounds on the gap's half angle: a 30 px doorway at r = 26 is 0.58 rad, at
/// r = 74 it is 0.20. The clamp keeps the art's arrows sane.
pub const COIL_GAP_HALF_MIN: f32 = 0.12;
pub const COIL_GAP_HALF_MAX: f32 = 1.20;

/// Most body points one laid shape can hold. The ring needs `2*pi*74/6 = 78`;
/// the spiral needs about 80; the resting loops need 90.
pub const COIL_MAX: usize = 112;

// ============================================================================
// The spiral (phase 3)
// ============================================================================

/// `SpiralCoil`: the body lays the lane down.
pub const SPIRAL_TELL: f32 = 1.05;
/// `SpiralChase`: the head comes down it.
pub const SPIRAL_CHASE: f32 = 2.80;
/// `SpiralEnd`: the coils drop and the head is spent.
pub const SPIRAL_END: f32 = 1.00;
/// The spiral's first turn's radius.
pub const SPIRAL_R0: f32 = 15.0;
/// Clear lane between two turns (px). Wide enough for the 10 px rat with
/// room to be hurried.
pub const SPIRAL_TURN: f32 = 30.0;
/// Distance between two body points in the laid lane.
pub const SPIRAL_SPACING: f32 = 15.0;
/// How fast the head comes down the lane.
///
/// Only just over the rat's own 58 px/s walk: the lane is 21 px wide, so a rat
/// that simply outruns it survives, and one that hesitates is caught and
/// bumped. A pursuit the rat can win is the whole point — `SpiralEnd` pays it
/// back with the hood.
pub const SPIRAL_SPEED: f32 = 72.0;
/// Radians of the outermost turn left open (the way out).
pub const SPIRAL_OPEN: f32 = 1.10;
/// How far inside the arena's own edge the outermost turn stops.
pub const SPIRAL_MARGIN: f32 = 16.0;
/// The rat has to be at least this fraction of the way inside the spiral for
/// the move to be committed.
pub const SPIRAL_INSIDE: f32 = 0.85;

// ============================================================================
// Palette
// ============================================================================

const HIDE_SH: Rgb = (30, 44, 34);
const HIDE_MID: Rgb = (74, 100, 62);
const HIDE_HI: Rgb = (150, 176, 104);
const BELLY: Rgb = (198, 202, 148);
const HOOD_SH: Rgb = (38, 48, 36);
const HOOD_MID: Rgb = (100, 120, 70);
const HOOD_HI: Rgb = (190, 204, 130);
const SPEC_SH: Rgb = (44, 34, 24);
const SPEC_HI: Rgb = (238, 228, 186);
const EYE: Rgb = (252, 208, 72);
const EYE_HOT: Rgb = (255, 246, 176);
const EYE_SLIT: Rgb = (22, 14, 8);
const FANG: Rgb = (240, 242, 226);
const VENOM: Rgb = (122, 222, 94);
const VENOM_HI: Rgb = (200, 255, 152);
const VENOM_SH: Rgb = (26, 70, 40);
const WARN: Rgb = (255, 186, 64);
const WARN_HOT: Rgb = (255, 72, 62);
const WEAK: Rgb = (255, 238, 156);
const BONE: Rgb = (226, 220, 196);

// ============================================================================
// States
// ============================================================================

/// The cobra's art state. Each one is a pose the rat can read, and the file's
/// doc table is the contract for what it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CobraState {
    /// **Rear.** The head is up off the floor, tasting the air and tracking the
    /// rat while the body piles up behind it.
    ///
    /// - Duration: [`REAR_DURATION`] (`1.10 / 1.00 / 0.90` s).
    /// - Warning: none (the neutral beat).
    /// - Dangerous area: body circle, [`CONTACT_DAMAGE`] (8).
    Rear,

    /// **The hood goes up.** The neck spreads, the spectacle mark comes up on
    /// the hood's back, and the strike line is painted and then frozen at
    /// [`HOOD_LOCK_PROGRESS`].
    ///
    /// - Duration: [`HOOD_TELL`] (`0.85 / 0.75 / 0.65` s).
    /// - Warning: [`STRIKE_HALF_WIDTH`] either side of the line.
    HoodTell,
    /// **The strike.** The head drives down the frozen line to the marked
    /// point.
    ///
    /// - Duration: [`STRIKE_DURATION`] (0.26 s); the line is live for
    ///   [`STRIKE_ACTIVE`] (0.16 s).
    /// - Damage: [`STRIKE_DAMAGE`] (20), once, plus a [`STRIKE_KNOCKBACK`]
    ///   shove.
    Strike,
    /// **The fangs are in the floor.** The head is down, the hood is flat and
    /// the spectacle mark on its back is the one thing that takes extra damage.
    ///
    /// - Duration: [`STUCK_DURATION`], or [`STUCK_WALLED`] when a wall took the
    ///   fangs.
    /// - No body contact; everything landing counts ×[`STUCK_DAMAGE_MULT`].
    FangsStuck,

    /// **The second strike's tell.** From phase 2: the head tears the fangs out
    /// and comes straight back over a shorter line.
    ///
    /// - Duration: [`RESTRIKE_TELL`]; the line freezes at
    ///   [`RESTRIKE_LOCK_PROGRESS`].
    ReStrikeTell,

    /// **The sway.** From phase 2: the head swings on a three-beat rhythm and
    /// the aim does not lock until [`SWAY_LOCK_PROGRESS`], so the strike can be
    /// baited into the floor the rat has left.
    ///
    /// - Duration: [`SWAY_TELL`] (`1.70 / 1.60 / 1.50` s).
    SwayTell,

    /// **The circles are marked.** The throat works; three landing circles are
    /// painted (the rat's, and two flanking it) and freeze at
    /// [`VENOM_LOCK_PROGRESS`].
    ///
    /// - Duration: [`VENOM_TELL`] (`0.80 / 0.72 / 0.64` s).
    VenomTell,
    /// **The spit.** The head throws; the puddles land under the marks.
    ///
    /// - Duration: [`VENOM_SPIT`] (0.22 s). Damage is the puddles' own (see
    ///   [`VENOM_SPLASH`], [`VENOM_DRIP`], [`VENOM_SLOW`]).
    VenomSpit,

    /// **The tail comes round.** The whip is drawn back out of the coil and the
    /// arc it will take — the floor the head is looking at, from
    /// [`TAIL_INNER`] to [`TAIL_OUTER`] — is painted.
    ///
    /// - Duration: [`TAIL_TELL`] (`0.80 / 0.72 / 0.64` s).
    TailTell,
    /// **The whip.** The tail takes the painted arc in one pass.
    ///
    /// - Duration: [`TAIL_SWEEP`] (0.34 s); damage [`TAIL_DAMAGE`] (14) and a
    ///   [`TAIL_KNOCKBACK`] throw.
    TailSweep,
    /// **The tail drags.** It is off balance with a coil's worth of tail on the
    /// floor beside it.
    ///
    /// - Duration: [`TAIL_RECOVER`] (`0.60 / 0.55 / 0.50` s).
    TailRecover,

    /// **The loop is being laid.** The body comes off the pile and takes the
    /// floor around the rat; the gap is marked from the first frame.
    ///
    /// - Duration: [`COIL_TELL`] (`0.95 / 0.90 / 0.85` s).
    ConstrictTell,
    /// **The constriction.** The loop closes on a marked clock: radius
    /// [`COIL_START_R`] → [`COIL_END_R`], the gap turning [`COIL_TURN`] rad.
    /// The coil is a **wall** (a walking rat is pushed out of it, a dashed one
    /// passes over).
    ///
    /// - Duration: [`COIL_TIGHTEN`] (`2.40 / 2.10 / 1.80` s).
    /// - Damage: [`CONSTRICT_DAMAGE`] (18) once, to everything still inside
    ///   when it closes.
    Constrict,
    /// **It lets go.** The loop slides off and the head comes back up.
    ///
    /// - Duration: [`COIL_LOOSEN`] (0.70 s).
    Loosen,

    /// **The room is furnished.** The whole animal is laid down as one spiral
    /// lane with an open outer end.
    ///
    /// - Duration: [`SPIRAL_TELL`] (1.05 s).
    SpiralCoil,
    /// **Down the lane.** The head comes after the rat along the laid body,
    /// faster than the rat can fight but no faster than it can run.
    ///
    /// - Duration: [`SPIRAL_CHASE`] (2.80 s); body contact 8 applies.
    SpiralChase,
    /// **The coils drop.** The lane opens, the head is spent, and the open hood
    /// is the weak point again.
    ///
    /// - Duration: [`SPIRAL_END`] (1.00 s); no body contact, ×
    ///   [`SPENT_DAMAGE_MULT`].
    SpiralEnd,

    /// **The pose change.** The 1.2 s harmless beat at every phase threshold:
    /// it rises to its full height, opens the hood all the way and shows the
    /// spectacle mark.
    ///
    /// - Duration: 1.20 s. No new attacks.
    Rise,

    /// **The death.** The body unspools, the head comes down on the tile and
    /// the hood folds.
    ///
    /// - Duration: [`DEATH_DURATION`] (2.60 s).
    Death,
}

impl CobraState {
    /// The states the rat is meant to walk in on.
    #[inline]
    pub fn is_recovery(self) -> bool {
        matches!(self, CobraState::FangsStuck | CobraState::SpiralEnd)
    }

    /// The states in which the body is laid on the floor as a wall.
    #[inline]
    pub fn is_coil(self) -> bool {
        matches!(self, CobraState::ConstrictTell | CobraState::Constrict)
    }

    /// The states in which the spiral lane is laid.
    #[inline]
    pub fn is_spiral(self) -> bool {
        matches!(
            self,
            CobraState::SpiralCoil | CobraState::SpiralChase | CobraState::SpiralEnd
        )
    }

    /// The states in which any of the body blocks a walking rat.
    #[inline]
    pub fn is_laid(self) -> bool {
        self.is_coil() || self.is_spiral()
    }
}

/// Everything the art (and the coil's separation pass) needs about the fight,
/// snapshotted from the [`Boss`](super::Boss) each frame.
#[derive(Clone, Copy, Debug)]
pub struct CobraView {
    /// The head's centre (the boss entity's own position).
    pub pos: Vec2,
    /// Which way the head is pointing.
    pub facing: Vec2,
    pub state: CobraState,
    pub progress: f32,
    pub anim: f32,
    pub hit_flash: f32,
    pub hp_frac: f32,
    pub phase: u8,
    /// The marked point the head is working toward: the strike's bite point,
    /// the venom marks' centre, or — while the loop is laid — the loop's
    /// **anchor** (the point it is closing on).
    pub target: Vec2,
    /// The line's far end / the spiral's centre, depending on the state.
    pub lane_start: Vec2,
    /// The centre of the loop's opening, in radians.
    pub gap_angle: f32,
    /// The live venom puddles (copied out of the boss's own hazards, so the art
    /// and the damage never disagree and a dead boss draws no pools).
    pub pools: [Vec2; POOL_VIEW],
    pub pool_r: [f32; POOL_VIEW],
    pub pool_n: u8,
    /// Is the rat outside the arena?
    pub hunting: bool,
    /// The arena, for the spiral's outer radius.
    pub arena: TileRect,
}

// ============================================================================
// Geometry (the gameplay spec the controller and the art share)
// ============================================================================

#[inline]
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline]
pub fn smoothstep(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn unit_facing(view: &CobraView) -> Vec2 {
    if view.facing.len_sq() > 1e-6 {
        view.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// The head's centre (where the eyes are).
#[inline]
pub fn head_pos(view: &CobraView) -> Vec2 {
    view.pos
}

/// Where the fangs are, forward of the head.
#[inline]
pub fn fangs_pos(view: &CobraView) -> Vec2 {
    view.pos + unit_facing(view) * FANG_REACH
}

/// The hood's centre: behind the head, and the thing the spectacle mark is
/// painted on.
#[inline]
pub fn hood_pos(view: &CobraView) -> Vec2 {
    view.pos - unit_facing(view) * HOOD_SETBACK
}

/// How open the hood is (`0..=1`): flared for the strike, half-open while it
/// sways, breathing the rest of the time.
#[inline]
pub fn hood_open(view: &CobraView) -> f32 {
    let idle = 0.28 + 0.08 * (view.anim * 2.1).sin();
    match view.state {
        // Flared right from the start of the tell so both the combat read and
        // the library boss card show the king's signature hood immediately.
        CobraState::HoodTell => 0.74 + 0.26 * smoothstep(view.progress / 0.45),
        CobraState::Strike | CobraState::FangsStuck => 1.0,
        CobraState::ReStrikeTell => 0.58 + 0.42 * smoothstep(view.progress / 0.5),
        CobraState::SwayTell => 0.55 + 0.45 * sway_phase(view).abs(),
        CobraState::Rise => smoothstep(view.progress / 0.35) * (1.0 - smoothstep((view.progress - 0.8) / 0.2) * 0.2),
        CobraState::VenomTell | CobraState::VenomSpit => 0.70,
        CobraState::Constrict | CobraState::ConstrictTell => 0.45,
        CobraState::SpiralChase => 0.80,
        CobraState::SpiralEnd => 0.35,
        CobraState::Death => (1.0 - smoothstep(view.progress * 1.4)) * 0.4,
        _ => idle,
    }
    .clamp(0.0, 1.0)
}

/// The hypnotic rhythm in `-1..=1`: three beats across the sway, then the
/// strike. Positive is the head's left.
#[inline]
pub fn sway_phase(view: &CobraView) -> f32 {
    (view.progress * SWAY_BEATS * std::f32::consts::TAU).sin()
}

/// The weak point, when one is exposed: the spectacle mark on the open hood's
/// back. `None` at every other time — this boss has exactly one.
#[inline]
pub fn weak_point(view: &CobraView) -> Option<(Vec2, f32)> {
    match view.state {
        CobraState::FangsStuck | CobraState::SpiralEnd => Some((hood_pos(view), HOOD_WEAK_RADIUS)),
        _ => None,
    }
}

/// The three hit circles, in auto-aim priority order. While the hood is exposed
/// its mark **is** the first circle, so the rat's own weapons reach for the
/// window the moment it opens.
#[inline]
pub fn body_hit_circles(view: &CobraView) -> [(Vec2, f32); 3] {
    let head = (head_pos(view), HEAD_RADIUS);
    let neck = (
        view.pos - unit_facing(view) * (HEAD_RADIUS + NECK_RADIUS * 0.8),
        NECK_RADIUS,
    );
    let body = (hood_pos(view), HOOD_RADIUS * 0.86);
    match weak_point(view) {
        Some(w) => [w, head, neck],
        None => [head, neck, body],
    }
}

/// Incoming damage multiplier for a state.
#[inline]
pub fn incoming_damage_mult(state: CobraState) -> f32 {
    match state {
        CobraState::FangsStuck => STUCK_DAMAGE_MULT,
        CobraState::SpiralEnd => SPENT_DAMAGE_MULT,
        _ => 1.0,
    }
}

/// Does this state deal body contact?
#[inline]
pub fn has_body_contact(state: CobraState) -> bool {
    !matches!(
        state,
        CobraState::FangsStuck | CobraState::SpiralEnd | CobraState::Death
    )
}

/// The live strike line as `(from, to)`, for the art.
#[inline]
pub fn strike_line(view: &CobraView) -> (Vec2, Vec2) {
    (fangs_pos(view), view.target)
}

/// The loop's radius right now, or `0.0` when the body is not laid as a loop.
///
/// The tell lays it at [`COIL_START_R`] plus a hand's width that comes in as
/// the pose settles; the constriction pulls it to [`COIL_END_R`] on
/// [`smoothstep`], so the clock the rat reads is the clock it gets.
#[inline]
pub fn coil_radius(view: &CobraView) -> f32 {
    match view.state {
        CobraState::ConstrictTell => COIL_START_R + 10.0 * (1.0 - smoothstep(view.progress)),
        CobraState::Constrict => lerp(COIL_START_R, COIL_END_R, smoothstep(view.progress)),
        _ => 0.0,
    }
}

/// The centre of the loop's opening, in radians. It turns as the loop closes so
/// the exit has to be run for.
#[inline]
pub fn coil_gap_angle(view: &CobraView) -> f32 {
    view.gap_angle + COIL_TURN * smoothstep(view.progress)
}

/// Half the opening's angle: [`COIL_GAP_ARC`] px of doorway at whatever radius
/// the loop is at.
#[inline]
pub fn coil_gap_half(view: &CobraView) -> f32 {
    let r = coil_radius(view).max(1.0);
    (COIL_GAP_ARC * 0.5 / r).clamp(COIL_GAP_HALF_MIN, COIL_GAP_HALF_MAX)
}

/// The laid loop's body points, written into `out`; returns how many.
///
/// The chain walks the ring from one edge of the opening all the way round to
/// the other, so the opening is exactly `gap_half` either side of
/// [`coil_gap_angle`] and there is no point in it. Spacing is
/// [`COIL_SPACING`] px, i.e. a wall no 10 px rat can walk through.
pub fn coil_chain(view: &CobraView, out: &mut [Vec2]) -> usize {
    let r = coil_radius(view);
    if r <= 0.5 || out.is_empty() {
        return 0;
    }
    let half = coil_gap_half(view);
    let span = (std::f32::consts::TAU - 2.0 * half).max(0.05);
    let n = (((span * r) / COIL_SPACING).ceil() as usize).clamp(4, out.len());
    let start = coil_gap_angle(view) + half;
    for (i, slot) in out.iter_mut().take(n).enumerate() {
        // The last point must not land back on the first.
        let a = start + span * (i as f32 / (n - 1).max(1) as f32);
        *slot = view.target + Vec2::from_angle(a) * r;
    }
    n
}

/// The spiral's outermost radius: the arena's short side, less the wall and the
/// margin. `0.0` when the arena is too small for a lane.
#[inline]
pub fn spiral_radius(view: &CobraView) -> f32 {
    let half_w = view.arena.w as f32 * crate::game::collision::TILE_F * 0.5;
    let half_h = view.arena.h as f32 * crate::game::collision::TILE_F * 0.5;
    let r = half_w.min(half_h) - SPIRAL_MARGIN;
    if r <= SPIRAL_R0 + SPIRAL_TURN {
        0.0
    } else {
        r
    }
}

/// The laid spiral lane's body points, written into `out`; returns how many.
///
/// An archimedean spiral (`r = R0 + TURN * theta / TAU`) centred on
/// [`CobraView::lane_start`], sampled every [`SPIRAL_SPACING`] px of arc, with
/// the last [`SPIRAL_OPEN`] rad of the outermost turn left off so the lane has
/// an open end. Empty unless the state is a spiral state.
///
/// `progress` lays it: the tell draws it on from the inside out, and
/// [`CobraState::SpiralEnd`] retracts it from the outside in.
pub fn spiral_chain(view: &CobraView, out: &mut [Vec2]) -> usize {
    if !view.state.is_spiral() || out.is_empty() {
        return 0;
    }
    let r_max = spiral_radius(view);
    if r_max <= SPIRAL_R0 {
        return 0;
    }
    let laid_frac = match view.state {
        CobraState::SpiralCoil => smoothstep(view.progress),
        CobraState::SpiralChase => 1.0,
        _ => 1.0 - smoothstep(view.progress),
    };
    if laid_frac <= 0.02 {
        return 0;
    }
    // Arc length of the full lane, so the lay-on is even: the outer end is the
    // last to arrive and the first to leave.
    let full_len = arch_spiral_length(SPIRAL_R0, r_max, SPIRAL_TURN).max(1.0);
    let want_len = full_len * laid_frac;
    let base = unit_facing(view).angle();
    let mut theta = 0.0f32;
    let mut len = 0.0f32;
    let mut n = 0usize;
    while n < out.len() {
        let r = SPIRAL_R0 + SPIRAL_TURN * theta / std::f32::consts::TAU;
        if r > r_max || len > want_len {
            break;
        }
        // The outermost turn's opening: skip the last `SPIRAL_OPEN` rad of arc
        // on the last (partial) turn, so the lane's mouth is a real doorway.
        let turns_done = (r - SPIRAL_R0) / SPIRAL_TURN;
        let remaining_turn = (turns_done.fract() * std::f32::consts::TAU).max(0.0001);
        let near_edge = r > r_max - 2.0;
        if !(near_edge && remaining_turn < SPIRAL_OPEN) {
            out[n] = view.lane_start + Vec2::from_angle(base + theta) * r;
            n += 1;
        }
        let step = SPIRAL_SPACING / r.max(1.0);
        len += SPIRAL_SPACING;
        theta += step;
        if theta > std::f32::consts::TAU * 8.0 {
            break;
        }
    }
    n
}

/// Arc length of an archimedean spiral `r = r0 + turn * theta / TAU` from `r0`
/// to `r1`.
#[inline]
fn arch_spiral_length(r0: f32, r1: f32, turn: f32) -> f32 {
    if turn <= 0.0 || r1 <= r0 {
        return 0.0;
    }
    // L = (1/k) * 0.5 * (r * sqrt(1 + k^2 r^2) + asinh(k r)), k = TAU / turn.
    let k = std::f32::consts::TAU / turn;
    let f = |r: f32| {
        let kr = k * r;
        0.5 * (kr * (1.0 + kr * kr).sqrt() + (kr + (1.0 + kr * kr).sqrt()).ln())
    };
    (f(r1) - f(r0)) / k
}

/// The chain that actually blocks a walking rat: the loop while it is
/// constricting, the laid lane while the spiral is down. Empty otherwise —
/// which is what the separation pass keys off.
pub fn blocking_chain(view: &CobraView, out: &mut [Vec2]) -> usize {
    if view.state.is_coil() {
        coil_chain(view, out)
    } else if view.state.is_spiral() {
        spiral_chain(view, out)
    } else {
        0
    }
}

/// The slack body that is drawn when nothing is laid: two lazy loops trailing
/// the head, breathing with the animation clock. Purely art — it blocks nothing
/// (the animal is not committed to the floor) — but it is what makes the thing
/// read as *far longer than the room*.
pub fn rest_chain(view: &CobraView, out: &mut [Vec2]) -> usize {
    if out.is_empty() {
        return 0;
    }
    let f = unit_facing(view);
    let p = f.perp();
    let back = view.pos - f * 16.0;
    let n = (out.len()).min(88);
    let breath = 0.5 + 0.5 * (view.anim * 0.7).sin();
    #[allow(clippy::needless_range_loop)]
    for i in 0..n {
        let t = i as f32 / (n - 1).max(1) as f32;
        // A long loop: the first turn wraps left, the second right, and the
        // tail end drifts wherever the animal last dragged it.
        let phase = t * std::f32::consts::TAU * 1.35 + view.anim * 0.35;
        let wobble = (phase * 2.1).sin() * (6.0 + 10.0 * t);
        let spin = (18.0 + 66.0 * t + breath * 4.0) * phase.cos();
        let reach = (26.0 + 40.0 * t) * phase.sin();
        out[i] = back + p * (spin * 0.55 + wobble) - f * (reach * 0.5 + 14.0 + t * 8.0);
        // A little sag into the floor's perspective.
        out[i].y += (t * std::f32::consts::PI).sin() * 3.0;
    }
    n
}

/// What the art should paint as the living body: the laid shape when one is
/// laid, the slack loops otherwise.
pub fn draw_chain(view: &CobraView, out: &mut [Vec2]) -> usize {
    if view.state.is_laid() {
        let n = blocking_chain(view, out);
        if n > 0 {
            return n;
        }
        // A laid spiral that has already retracted (`SpiralEnd`) leaves the
        // slack body behind rather than a bodiless head.
    }
    rest_chain(view, out)
}

/// The venom puddles that are live this frame.
#[inline]
pub fn pool_count(view: &CobraView) -> usize {
    (view.pool_n as usize).min(POOL_VIEW)
}

// ============================================================================
// Drawing helpers
// ============================================================================

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> Vec2 {
    Vec2::new(p.x - cam.x, p.y - cam.y)
}

#[inline]
fn bbox(pb: &PixelBuffer, c: Vec2, rx: f32, ry: f32) -> Option<(i32, i32, i32, i32)> {
    let x0 = (c.x - rx).floor() as i32;
    let x1 = (c.x + rx).ceil() as i32;
    let y0 = (c.y - ry).floor() as i32;
    let y1 = (c.y + ry).ceil() as i32;
    if x1 < 0 || y1 < 0 || x0 >= pb.w as i32 || y0 >= pb.h as i32 {
        return None;
    }
    Some((
        x0.max(0),
        x1.min(pb.w as i32 - 1),
        y0.max(0),
        y1.min(pb.h as i32 - 1),
    ))
}

#[inline]
fn on_screen(pb: &PixelBuffer, s: Vec2, m: f32) -> bool {
    s.x + m >= 0.0 && s.y + m >= 0.0 && s.x - m < pb.w as f32 && s.y - m < pb.h as f32
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.004 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    if a >= 0.999 {
        pb.plot(x, y, c);
        return;
    }
    let old = pb.pixel_at(ux, uy);
    pb.plot(x, y, mix(old, c, a));
}

#[inline]
fn shade3(sh: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    if lum < 0.5 {
        mix(sh, mid, (lum * 2.0).clamp(0.0, 1.0))
    } else {
        mix(mid, hi, ((lum - 0.5) * 2.0).clamp(0.0, 1.0))
    }
}

#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit > 0.0 {
        mix(c, (255, 236, 206), (hit * 0.72).clamp(0.0, 0.9))
    } else {
        c
    }
}

#[inline]
fn hash01(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

fn line_solid(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, alpha: f32) {
    let d = b - a;
    // A non-finite endpoint would saturate `steps` (two billion plots).
    if !d.len().is_finite() {
        return;
    }
    let steps = d.len().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let p = a + d * t;
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, alpha);
    }
}

fn line_dashed(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    col: Rgb,
    alpha: f32,
    dash: f32,
    phase: f32,
) {
    let d = b - a;
    // A non-finite endpoint would saturate `steps` (two billion plots).
    if !d.len().is_finite() {
        return;
    }
    let steps = d.len().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let along = t * d.len() + phase;
        if (along / dash.max(0.5)).floor() as i32 % 2 != 0 {
            continue;
        }
        let p = a + d * t;
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, alpha);
    }
}

fn ring(pb: &mut PixelBuffer, center: Vec2, radius: f32, col: Rgb, alpha: f32, width: f32) {
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius + width, radius + width) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 - center.x, y as f32 - center.y).len();
            if (d - radius).abs() <= width * 0.5 {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

fn fill_disc(pb: &mut PixelBuffer, center: Vec2, radius: f32, col: Rgb, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius, radius) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 - center.x, y as f32 - center.y).len();
            if d <= radius {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

/// A filled annulus band between `inner` and `outer`.
fn band(pb: &mut PixelBuffer, center: Vec2, inner: f32, outer: f32, col: Rgb, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox(pb, center, outer, outer) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 - center.x, y as f32 - center.y).len();
            if d >= inner && d <= outer {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

/// A pie slice between the `(inner, outer)` radii, `half_angle` either side of
/// `facing`.
fn arc_band(
    pb: &mut PixelBuffer,
    center: Vec2,
    (inner, outer): (f32, f32),
    facing: Vec2,
    half_angle: f32,
    col: Rgb,
    alpha: f32,
) {
    let Some((x0, x1, y0, y1)) = bbox(pb, center, outer, outer) else {
        return;
    };
    let f = if facing.len_sq() > 1e-6 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let cos_lim = half_angle.cos();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 - center.x, y as f32 - center.y);
            let dist = d.len();
            if dist < inner || dist > outer {
                continue;
            }
            if dist > 1e-4 && d.normalized().dot(f) < cos_lim {
                continue;
            }
            blend_px(pb, x, y, col, alpha);
        }
    }
}

/// A filled, rotated ellipse: `rx` along `facing`, `ry` across it.
fn ellipse(
    pb: &mut PixelBuffer,
    center: Vec2,
    rx: f32,
    ry: f32,
    facing: Vec2,
    col: Rgb,
    alpha: f32,
) {
    let f = if facing.len_sq() > 1e-6 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = f.perp();
    let m = rx.max(ry);
    let Some((x0, x1, y0, y1)) = bbox(pb, center, m, m) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32 - center.x, y as f32 - center.y);
            let u = d.dot(f) / rx.max(0.1);
            let v = d.dot(p) / ry.max(0.1);
            if u * u + v * v <= 1.0 {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

/// A thick oriented capsule between two points: the snake's neck, fangs and
/// whip are all made of these.
fn capsule(pb: &mut PixelBuffer, a: Vec2, b: Vec2, r: f32, col: Rgb, alpha: f32) {
    let d = b - a;
    // A non-finite endpoint would saturate `steps` (two billion plots).
    if !d.len().is_finite() {
        return;
    }
    let steps = d.len().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let p = a + d * t;
        fill_disc(pb, p, r, col, alpha);
    }
}

#[inline]
fn smooth_dir(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-6 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

// ============================================================================
// Ground layer
// ============================================================================

/// Draw the cobra's ground layer: the body's contact shade, the laid loop's
/// closing clock and its opening, the spiral's mouth, the venom puddles, the
/// marked strike line and the sway's ghost heads.
///
/// **Layer**: ground — after the floor and stains, before props/enemies/the rat.
pub fn draw_ground(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2, accent: Rgb) {
    let mut pts = [Vec2::ZERO; COIL_MAX];
    let n = draw_chain(view, &mut pts);
    if n > 0 {
        draw_body_shade(pb, &pts[..n], cam, view.state.is_laid());
    }
    if view.state.is_coil() {
        draw_constrict_marks(pb, view, cam);
    }
    if view.state == CobraState::SpiralCoil || view.state == CobraState::SpiralChase {
        draw_spiral_mouth(pb, view, cam);
    }
    draw_pools(pb, view, cam);
    draw_strike_marks(pb, view, cam);
    let _ = accent;
}

/// The dark the body leaves under itself. Laid shapes get a heavier, wider
/// contact shade (they are pressed into the floor); the slack loops get a soft
/// one.
fn draw_body_shade(pb: &mut PixelBuffer, pts: &[Vec2], cam: Vec2, laid: bool) {
    let (step, r, alpha) = if laid { (1usize, 5.2f32, 0.42) } else { (2, 4.6, 0.30) };
    for (i, p) in pts.iter().enumerate().step_by(step) {
        let s = w2s(*p, cam) + Vec2::new(0.6, 1.6);
        if !on_screen(pb, s, r + 2.0) {
            continue;
        }
        let band = 0.85 + 0.15 * ((i as f32) * 0.7).sin();
        fill_disc(pb, s, r, (18, 26, 20), alpha * band);
    }
}

/// The constriction's own clock: the ring it is closing to (dashed, with the
/// opening left out), a radius line through the opening and two arrows at the
/// mouth of the gap.
fn draw_constrict_marks(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2) {
    let anchor = w2s(view.target, cam);
    let r = coil_radius(view);
    if r <= 1.0 {
        return;
    }
    let gap = coil_gap_angle(view);
    let half = coil_gap_half(view);
    let end_r = COIL_END_R;
    let closing = smoothstep(view.progress);
    // The clock: the loop's final radius, drawn as a dashed ring so the rat can
    // see what it is being squeezed into.
    let samples = ((end_r * 1.6) as usize).clamp(16, 44);
    let mut last: Option<Vec2> = None;
    for i in 0..=samples {
        let a = i as f32 / samples as f32 * std::f32::consts::TAU;
        // skip the opening
        let rel = ((a - gap).rem_euclid(std::f32::consts::TAU)).min(
            (gap - a).rem_euclid(std::f32::consts::TAU),
        );
        if rel < half {
            last = None;
            continue;
        }
        let p = anchor + Vec2::from_angle(a) * end_r;
        let dash = (i / 2) % 2 == 0;
        if let Some(l) = last {
            if dash {
                line_solid(pb, l, p, WARN, 0.42 + 0.25 * closing);
            }
        }
        last = Some(p);
    }
    // The way out: a radius line through the opening and two chevrons pointing
    // out of it. Green because it is the answer, not the threat.
    let out = Vec2::from_angle(gap);
    let inner = anchor + out * (r - COIL_BODY_HALF - 2.0);
    let outer = anchor + out * (r + 16.0);
    line_dashed(pb, inner, outer, VENOM_HI, 0.55, 3.0, view.anim * 22.0);
    let side = out.perp() * 4.0;
    let tip = anchor + out * (r + 15.0);
    line_solid(pb, tip - side - out * 5.0, tip, VENOM_HI, 0.85);
    line_solid(pb, tip + side - out * 5.0, tip, VENOM_HI, 0.85);
    // Both edges of the doorway: two short ticks, so the width of the gap is
    // never in doubt.
    for s in [1.0f32, -1.0] {
        let e = Vec2::from_angle(gap + s * half);
        line_solid(
            pb,
            anchor + e * (r - COIL_BODY_HALF - 3.0),
            anchor + e * (r + COIL_BODY_HALF + 3.0),
            VENOM_HI,
            0.7,
        );
    }
}

/// The laid body's **tell**, for the post-lighting hostile layer: the coil (or
/// the spiral lane) re-stamped as a thin lit line, plus the doorway it leaves.
///
/// The body itself is drawn in the entity pass and is therefore dimmable — and
/// a wall the rat is about to walk into must never be the dimmest thing on the
/// floor. This is the same contract the horseshoe crab's tail arc has: the
/// shape the rat has to respect is stamped after the lighting, in the boss's
/// own warning colours.
pub fn draw_coil_tell(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2) {
    if !view.state.is_laid() {
        return;
    }
    let mut pts = [Vec2::ZERO; COIL_MAX];
    let n = blocking_chain(view, &mut pts);
    if n < 2 {
        return;
    }
    let col = if view.state == CobraState::Constrict {
        WARN
    } else {
        VENOM_HI
    };
    for i in 0..n {
        let s = w2s(pts[i], cam);
        if !on_screen(pb, s, 6.0) {
            continue;
        }
        blend_px(pb, s.x.round() as i32, s.y.round() as i32, col, 0.50);
        if i + 1 < n {
            // Half a step between points, so a 6 px chain reads as one wall.
            let m = w2s((pts[i] + pts[i + 1]) * 0.5, cam);
            blend_px(pb, m.x.round() as i32, m.y.round() as i32, col, 0.34);
        }
    }
    if view.state.is_coil() {
        // The one opening, re-stated: the edges and the arrow out of it.
        let anchor = w2s(view.target, cam);
        let r = coil_radius(view);
        let gap = coil_gap_angle(view);
        let half = coil_gap_half(view);
        let out = Vec2::from_angle(gap);
        let tip = anchor + out * (r + 14.0);
        line_solid(pb, anchor + out * (r - 5.0), tip, VENOM_HI, 0.75);
        let side = out.perp() * 4.0;
        line_solid(pb, tip - side - out * 5.0, tip, VENOM_HI, 0.9);
        line_solid(pb, tip + side - out * 5.0, tip, VENOM_HI, 0.9);
        for s in [1.0f32, -1.0] {
            let e = Vec2::from_angle(gap + s * half);
            line_solid(
                pb,
                anchor + e * (r - COIL_BODY_HALF - 3.0),
                anchor + e * (r + COIL_BODY_HALF + 3.0),
                VENOM_HI,
                0.85,
            );
        }
    }
}

/// The spiral's mouth: the open end of the outermost turn, marked so the rat
/// knows which way the lane runs.
fn draw_spiral_mouth(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2) {
    let r_max = spiral_radius(view);
    if r_max <= SPIRAL_R0 {
        return;
    }
    let base = unit_facing(view).angle();
    // The outer end sits at the end of the laid lane's last full turn.
    let turns = (r_max - SPIRAL_R0) / SPIRAL_TURN;
    let theta = turns * std::f32::consts::TAU;
    let tip = w2s(
        view.lane_start + Vec2::from_angle(base + theta) * r_max,
        cam,
    );
    if !on_screen(pb, tip, 22.0) {
        return;
    }
    let out = Vec2::from_angle(base + theta);
    line_dashed(pb, tip - out * 4.0, tip + out * 14.0, VENOM_HI, 0.6, 3.0, view.anim * 20.0);
    let side = out.perp() * 4.0;
    line_solid(pb, tip + out * 14.0 - side - out * 5.0, tip + out * 14.0, VENOM_HI, 0.85);
    line_solid(pb, tip + out * 14.0 + side - out * 5.0, tip + out * 14.0, VENOM_HI, 0.85);
}

/// The venom puddles: a foul green disc, a bright meniscus, and a skin of
/// bubbles that breathes with the clock.
fn draw_pools(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2) {
    for i in 0..pool_count(view) {
        let r = view.pool_r[i].max(1.0);
        let c = w2s(view.pools[i], cam);
        if !on_screen(pb, c, r + 4.0) {
            continue;
        }
        // The wet ring bleeds a little past the damage circle.
        band(pb, c, r * 0.55, r + 1.4, VENOM_SH, 0.55);
        fill_disc(pb, c, r, (54, 116, 56), 0.34);
        ring(pb, c, r - 0.6, VENOM, 0.62, 1.0);
        ring(pb, c, r * 0.55, VENOM_HI, 0.24, 1.0);
        // Bubbles: a hash scatter so the puddle looks boiled rather than drawn.
        let steps = (r * 2.2) as i32;
        for k in 0..steps {
            let a = hash01(k * 7 + i as i32, k + 3) * std::f32::consts::TAU;
            let d = hash01(k + 11, i as i32 * 5 + k) * (r - 1.5);
            let breathe = 0.55 + 0.45 * (view.anim * 3.1 + k as f32 * 1.7).sin();
            let bp = c + Vec2::from_angle(a) * d;
            blend_px(
                pb,
                bp.x.round() as i32,
                bp.y.round() as i32,
                VENOM_HI,
                0.20 + 0.35 * breathe,
            );
        }
    }
}

/// The marked line and everything the head aims with: the dashed strike line
/// and the fang-cross where it will land, the sway's ghost heads and the
/// venom's circles.
fn draw_strike_marks(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2) {
    match view.state {
        CobraState::HoodTell | CobraState::ReStrikeTell | CobraState::Strike => {
            let (a, b) = strike_line(view);
            let a = w2s(a, cam);
            let b = w2s(b, cam);
            if !on_screen(pb, a, 30.0) && !on_screen(pb, b, 30.0) {
                return;
            }
            let live = view.state == CobraState::Strike;
            let col = if live { WARN_HOT } else { WARN };
            line_dashed(pb, a, b, col, 0.7, 4.0, -view.anim * 26.0);
            // The fang-cross at the bite point: where it is going to land.
            let d = smooth_dir(b - a);
            let p = d.perp();
            // The corridor's real width: the hazard is `STRIKE_HALF_WIDTH`
            // either side of the line, so both edges are drawn, not only the
            // centre line.
            for s in [-1.0f32, 1.0] {
                let o = p * (s * STRIKE_HALF_WIDTH);
                line_dashed(pb, a + o, b + o, col, 0.45, 3.0, -view.anim * 26.0);
            }
            line_solid(pb, b - d * 5.0 - p * 5.0, b + d * 5.0 + p * 5.0, col, 0.85);
            line_solid(pb, b - d * 5.0 + p * 5.0, b + d * 5.0 - p * 5.0, col, 0.85);
            if live {
                capsule(pb, a, b, 1.6, WARN_HOT, 0.35);
                // Stamped once per px of length, so a faint alpha builds to a
                // soft fill of the whole corridor.
                capsule(pb, a, b, STRIKE_HALF_WIDTH, WARN_HOT, 0.03);
            }
        }
        CobraState::SwayTell => {
            // The beat's ghosts: three faded heads either side of the line the
            // sway is drawing, so the rhythm is legible even in a still frame.
            let f = unit_facing(view);
            let p = f.perp();
            let hp = w2s(view.pos, cam);
            for k in 0..3 {
                let t = (k as f32 + 1.0) / 4.0;
                let swing = sway_phase(view) * (1.0 - t) * 12.0;
                let c = hp + p * swing + f * (t * 5.0);
                let ghost = 0.30 - 0.08 * k as f32;
                fill_disc(pb, c, 4.2 - t, WARN, ghost);
                ellipse(pb, c - f * 3.0, 4.0, 6.5 - t * 2.0, f, WARN, ghost * 0.8);
            }
            let (a, b) = strike_line(view);
            line_dashed(pb, w2s(a, cam), w2s(b, cam), WARN, 0.42, 5.0, view.anim * 18.0);
        }
        CobraState::VenomTell => {
            // The three circles the throat is choosing between.
            for k in 0..VENOM_MARKS {
                let at = venom_mark_centre(view, k, None);
                let c = w2s(at, cam);
                if !on_screen(pb, c, VENOM_RADIUS + 6.0) {
                    continue;
                }
                ring(pb, c, VENOM_RADIUS, WARN, 0.55, 1.0);
                ring(pb, c, VENOM_RADIUS * smoothstep(view.progress), WARN_HOT, 0.5, 1.0);
                let d = smooth_dir(c - w2s(view.pos, cam));
                line_solid(pb, c - d * (VENOM_RADIUS - 3.0), c, WARN, 0.4);
            }
            // The arc the glob will take: a lobbed dotted curve from the fangs.
            let a = w2s(fangs_pos(view), cam);
            let b = w2s(view.target, cam);
            let steps = 10;
            for i in 1..steps {
                let t = i as f32 / steps as f32;
                let p = a + (b - a) * t - Vec2::new(0.0, (t * std::f32::consts::PI).sin() * 14.0);
                blend_px(
                    pb,
                    p.x.round() as i32,
                    p.y.round() as i32,
                    VENOM_HI,
                    0.30 + 0.4 * t,
                );
            }
        }
        CobraState::TailTell => {
            // The whip's arc, painted from the head out: it takes the floor the
            // head is looking at, and the dead zone under the jaw is the tell.
            let f = unit_facing(view);
            let c = w2s(view.pos, cam);
            arc_band(pb, c, (TAIL_INNER, TAIL_OUTER), f, TAIL_HALF_ANGLE, WARN, 0.16);
            // Both edges of the annulus, dashed, so the width of the wedge is
            // as legible as its angle.
            for radius in [TAIL_INNER, TAIL_OUTER] {
                let steps = 22;
                let mut last: Option<Vec2> = None;
                for i in 0..=steps {
                    let a = -TAIL_HALF_ANGLE
                        + 2.0 * TAIL_HALF_ANGLE * (i as f32 / steps as f32);
                    let p = c + f.rotate(a) * radius;
                    if let Some(l) = last {
                        if i % 2 == 0 {
                            line_solid(pb, l, p, WARN, 0.5);
                        }
                    }
                    last = Some(p);
                }
            }
            // The dead zone: a dotted ring at the jaw, the safe pocket.
            ring(pb, c, TAIL_INNER, VENOM_HI, 0.35, 1.0);
        }
        _ => {}
    }
}

/// The centre of venom mark `k`. `rat` overrides the live rat position (the
/// controller passes its own so the commit and the paint agree).
pub fn venom_mark_centre(view: &CobraView, k: usize, rat: Option<Vec2>) -> Vec2 {
    let base = rat.unwrap_or(view.target);
    let f = unit_facing(view);
    let p = f.perp();
    match k {
        0 => base,
        1 => base + p * VENOM_SPREAD,
        _ => base - p * VENOM_SPREAD,
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// The painted half-thickness of the body at `t` along the chain (`0` = the
/// neck end, `1` = the tail).
#[inline]
fn body_radius_at(t: f32) -> f32 {
    let bulge = (t * std::f32::consts::PI).sin();
    2.5 + 2.0 * bulge
}

/// Draw the cobra: the neck, the body chain (slack loops, the constriction's
/// ring or the laid spiral), the hood with its spectacle mark, the head and the
/// fangs, and the death sag.
///
/// **Layer**: entity — with the other bodies, after the props and before the
/// bullets.
pub fn draw(pb: &mut PixelBuffer, view: &CobraView, cam: Vec2, accent: Rgb) {
    let f = unit_facing(view);
    let p = f.perp();
    let hit = view.hit_flash;
    let droop = if view.state == CobraState::Death {
        smoothstep(view.progress * 1.25)
    } else {
        0.0
    };
    let head = w2s(view.pos, cam) + Vec2::new(0.0, droop * 5.0);

    let mut pts = [Vec2::ZERO; COIL_MAX];
    let n = draw_chain(view, &mut pts);
    let mut nearest: Option<Vec2> = None;
    if n > 0 {
        draw_body(pb, &pts[..n], cam, f, hit, droop);
        let mut best = f32::MAX;
        for q in pts.iter().take(n) {
            let d = q.dist_sq(view.pos);
            if d < best {
                best = d;
                nearest = Some(*q);
            }
        }
    }
    // The neck: from under the hood out to the body it belongs to. It crosses
    // the loops it has laid, which is the whole point of the animal.
    let neck_from = head - f * (HOOD_SETBACK + 3.0);
    let neck_to = match nearest {
        Some(q) if q.dist(view.pos) > 12.0 => w2s(q, cam) + Vec2::new(0.0, droop * 3.0),
        _ => head - f * 22.0 + Vec2::new(0.0, droop * 3.0),
    };
    capsule(pb, neck_from, neck_to, NECK_RADIUS * 0.55, apply_hit(HIDE_SH, hit), 1.0);
    capsule(pb, neck_from, neck_to, NECK_RADIUS * 0.42, apply_hit(HIDE_MID, hit), 1.0);
    // The pale throat stripe, so the neck reads as a snake's and not a tube.
    let mid = (neck_from + neck_to) * 0.5;
    line_dashed(
        pb,
        neck_from + p * 1.2,
        mid + p * 1.2,
        apply_hit(BELLY, hit),
        0.35,
        3.0,
        view.anim * 8.0,
    );

    draw_head(pb, view, cam, head, f, p, hit, droop);
    let _ = accent;
}

fn draw_body(pb: &mut PixelBuffer, pts: &[Vec2], cam: Vec2, f: Vec2, hit: f32, droop: f32) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let p = f.perp();
    for (i, q) in pts.iter().enumerate() {
        let t = i as f32 / (n - 1) as f32;
        let r = body_radius_at(t);
        let s = w2s(*q, cam) + Vec2::new(0.0, droop * 3.0);
        if !on_screen(pb, s, r + 3.0) {
            continue;
        }
        // Crisp dark exterior outline so the coiled body survives dark floors and downscale.
        fill_disc(pb, s, r + 1.1, (8, 10, 8), 0.85);
        // Segments: the banding is the scale row, and it costs nothing.
        let seg = ((i as f32 * 0.55 + 0.4).sin() * 0.5 + 0.5).powf(1.6);
        let col = shade3(HIDE_SH, HIDE_MID, HIDE_HI, 0.34 + 0.52 * seg);
        fill_disc(pb, s, r, apply_hit(col, hit), 1.0);
        // The spine's shadow: a soft dark line down the middle.
        fill_disc(pb, s + Vec2::new(0.0, r * 0.28), r * 0.52, apply_hit(HIDE_SH, hit), 0.45);
        // Dorsal highlight, up and to the left of the light.
        if r > 2.4 {
            fill_disc(
                pb,
                s + Vec2::new(-r * 0.34, -r * 0.40),
                r * 0.34,
                apply_hit(HIDE_HI, hit),
                0.78,
            );
        }
        // A dark saddle every fifth segment, and a pale flank scale between.
        if i % 5 == 0 {
            let half = p * (r * 0.86);
            line_solid(pb, s - half, s + half, apply_hit(HIDE_SH, hit), 0.5);
        } else if i % 5 == 2 {
            let half = p * (r * 0.5);
            line_solid(pb, s - half + f * 0.6, s + half + f * 0.6, apply_hit(BELLY, hit), 0.16);
        }
    }
    // The tail's last few points get a taper mark so the end reads as a tip.
    if let Some(last) = pts.last() {
        let s = w2s(*last, cam);
        fill_disc(pb, s, 1.2, apply_hit(HIDE_HI, hit), 0.8);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_head(
    pb: &mut PixelBuffer,
    view: &CobraView,
    cam: Vec2,
    head: Vec2,
    f: Vec2,
    p: Vec2,
    hit: f32,
    droop: f32,
) {
    let open = hood_open(view) * (1.0 - droop * 0.85);
    let hood_c = w2s(hood_pos(view), cam) + Vec2::new(0.0, droop * 4.0);
    let dead = view.state == CobraState::Death;

    // ---- the hood ---------------------------------------------------------
    if open > 0.06 {
        // Broad flared hood spanning wide ACROSS the neck (`hy > hx`) so the
        // classic cobra silhouette is unmistakable even at library card scale!
        let hx = 3.4 + HOOD_RADIUS * 0.78 * open;
        let hy = 4.6 + HOOD_RADIUS * 1.15 * open;
        // Crisp dark exterior outline framing the flared hood.
        ellipse(pb, hood_c, hx + 1.35, hy + 1.35, f, (8, 10, 8), 0.92);
        ellipse(pb, hood_c, hx, hy, f, apply_hit(HOOD_SH, hit), 1.0);
        ellipse(
            pb,
            hood_c - f * 0.5 - p * 0.4,
            hx * 0.90,
            hy * 0.92,
            f,
            apply_hit(HOOD_MID, hit),
            1.0,
        );
        // Sunlit top-left rim bevel catching the flared hood wings!
        ellipse(
            pb,
            hood_c - f * 1.0 - p * 1.4,
            hx * 0.76,
            hy * 0.80,
            f,
            apply_hit(HOOD_HI, hit),
            0.58,
        );
        // Pale belly-gold rim lining along the flaring wing edges.
        for s in [-1.0f32, 1.0] {
            let rim_a = hood_c + f * (hx * 0.35) + p * (hy * 0.78 * s);
            let rim_b = hood_c - f * (hx * 0.45) + p * (hy * 0.72 * s);
            line_solid(pb, rim_a, rim_b, apply_hit(BELLY, hit), if s < 0.0 { 0.78 } else { 0.42 });
        }
        // Three dark chevrons opening forward, the king's own marking.
        for k in 0..3 {
            let t = k as f32 / 3.0;
            let back = hood_c - f * (hx * (0.12 + 0.38 * t));
            let w = hy * (0.82 - 0.20 * t);
            line_solid(pb, back - p * w, back + f * (hx * 0.28), apply_hit(HOOD_SH, hit), 0.65);
            line_solid(pb, back + p * w, back + f * (hx * 0.28), apply_hit(HOOD_SH, hit), 0.65);
        }
        // ---- the spectacle mark (the fight's one weak point) --------------
        let mark = 0.40 + 0.60 * open;
        let weak = weak_point(view).is_some();
        let ring_col = if weak {
            let pulse = 0.6 + 0.4 * (view.anim * 6.0).sin().abs();
            mix(BELLY, WEAK, pulse)
        } else {
            BELLY
        };
        let mark_c = hood_c - f * 0.3;
        let spec_off = hy * 0.34 * mark;
        let spec_r = (2.8 * mark).max(1.6);
        // Classic spectacled twin-ocelli mark bridged across the dorsal hood!
        line_solid(
            pb,
            mark_c - p * spec_off,
            mark_c + p * spec_off,
            apply_hit(ring_col, hit),
            0.92,
        );
        for s in [-1.0f32, 1.0] {
            let oc = mark_c + p * (spec_off * s);
            fill_disc(pb, oc, spec_r + 0.5, apply_hit(ring_col, hit), 0.94);
            fill_disc(pb, oc, (spec_r - 1.1).max(0.7), apply_hit(SPEC_SH, hit), 0.95);
        }
        if weak {
            let halo = 6.8 + 1.2 * (view.anim * 5.0).sin();
            ring(pb, mark_c, halo, WEAK, 0.65, 1.1);
            line_solid(pb, mark_c + f * halo, mark_c + f * (halo + 3.0), WEAK, 0.78);
            line_solid(pb, mark_c - f * halo, mark_c - f * (halo + 3.0), WEAK, 0.78);
        }
    }

    // ---- the skull --------------------------------------------------------
    let rx = HEAD_RADIUS * 1.00;
    let ry = HEAD_RADIUS * 0.80;
    ellipse(pb, head, rx + 1.2, ry + 1.2, f, (8, 10, 8), 0.92);
    ellipse(pb, head, rx, ry, f, apply_hit(HIDE_SH, hit), 1.0);
    ellipse(pb, head - f * 0.6 - p * 0.4, rx * 0.90, ry * 0.88, f, apply_hit(HIDE_MID, hit), 1.0);
    ellipse(
        pb,
        head - f * 2.0 - p * 1.4,
        rx * 0.62,
        ry * 0.56,
        f,
        apply_hit(HIDE_HI, hit),
        0.52,
    );
    // The snout: a blunt wedge of a king's head, wider than a viper's.
    let tip = head + f * (HEAD_RADIUS * 1.18);
    capsule(pb, head + f * 2.0, tip, 3.1, apply_hit(HIDE_MID, hit), 1.0);
    capsule(pb, head + f * 2.4 - p * 0.7, tip - p * 0.4, 1.9, apply_hit(HIDE_HI, hit), 0.30);
    // The jawline and the nostrils.
    for s in [1.0f32, -1.0] {
        line_solid(
            pb,
            head + f * 1.0 + p * (3.4 * s) + Vec2::new(0.0, droop * 2.0),
            head + f * 8.4 + p * (1.1 * s) + Vec2::new(0.0, droop * 2.0),
            apply_hit(HIDE_SH, hit),
            0.65,
        );
        blend_px(
            pb,
            (tip - f * 1.6 + p * (1.7 * s)).x.round() as i32,
            (tip - f * 1.6 + p * (1.7 * s)).y.round() as i32,
            apply_hit(SPEC_SH, hit),
            0.8,
        );
    }

    // ---- the eyes ---------------------------------------------------------
    for s in [1.0f32, -1.0] {
        let e = head + f * 1.6 + p * (4.1 * s);
        if dead {
            fill_disc(pb, e, 1.5, (86, 74, 48), 0.9);
            continue;
        }
        let critical = view.hp_frac <= CRITICAL_HP_FRAC;
        let glow = if critical {
            0.75 + 0.25 * (view.anim * 11.0).sin().abs()
        } else {
            1.0
        };
        fill_disc(pb, e, 2.1, mix(EYE, EYE_HOT, 1.0 - glow), 1.0);
        fill_disc(pb, e - f * 0.4 - p * 0.3 * s, 1.2, EYE_HOT, 0.9);
        line_solid(pb, e - f * 1.4, e + f * 1.4, EYE_SLIT, 1.0);
        ring(pb, e, 2.4, apply_hit((40, 34, 18), hit), 0.7, 1.0);
        if critical {
            ring(pb, e, 3.2 + glow, WEAK, 0.35, 1.0);
        }
    }

    // ---- the fangs --------------------------------------------------------
    let fangs_out = matches!(
        view.state,
        CobraState::Strike
            | CobraState::FangsStuck
            | CobraState::ReStrikeTell
            | CobraState::VenomSpit
            | CobraState::VenomTell
    );
    if fangs_out && !dead {
        let reach = if view.state == CobraState::FangsStuck {
            FANG_REACH * 0.55
        } else {
            FANG_REACH * 0.92
        };
        for s in [1.0f32, -1.0] {
            let root = head + f * 5.0 + p * (2.0 * s);
            let point = head + f * reach - p * (0.5 * s);
            capsule(pb, root, point, 0.75, apply_hit(FANG, hit), 1.0);
            blend_px(
                pb,
                point.x.round() as i32,
                point.y.round() as i32,
                FANG,
                1.0,
            );
            if view.state == CobraState::FangsStuck {
                // The fangs are in the floor: a splinter of tile under them.
                line_solid(pb, point - p * 1.6, point + p * 1.6, BONE, 0.55);
            }
        }
    }

    // ---- the throat's glob (the spit) -------------------------------------
    if view.state == CobraState::VenomSpit || view.state == CobraState::VenomTell {
        let g = head + f * (FANG_REACH - 2.0);
        let swell = if view.state == CobraState::VenomSpit {
            2.6
        } else {
            1.2 + 1.4 * smoothstep(view.progress)
        };
        fill_disc(pb, g, swell, VENOM_SH, 0.85);
        fill_disc(pb, g - f * 0.3, swell * 0.62, VENOM, 0.9);
        fill_disc(pb, g - f * 0.6 - p * 0.4, swell * 0.30, VENOM_HI, 0.9);
    }

    // ---- the death sag ----------------------------------------------------
    if dead {
        // The head sags off the neck and the hood folds over it.
        let sag = droop * 4.0;
        for k in 0..3 {
            let off = Vec2::new((k as f32 - 1.0) * 4.0, sag + k as f32 * 1.5);
            fill_disc(pb, head + off, 2.0 - k as f32 * 0.4, (26, 30, 24), 0.5);
        }
        if droop > 0.6 {
            let stain = head + Vec2::new(0.0, 3.0);
            ring(pb, stain, 4.0 * (droop - 0.6) * 6.0, VENOM_SH, 0.35, 1.0);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    //! The art file is the gameplay spec, so these pin the geometry the
    //! controller and the separation pass both read.

    use super::*;

    fn view(state: CobraState, progress: f32) -> CobraView {
        CobraView {
            pos: Vec2::new(120.0, 100.0),
            facing: Vec2::new(1.0, 0.0),
            state,
            progress,
            anim: 0.0,
            hit_flash: 0.0,
            hp_frac: 1.0,
            phase: 1,
            target: Vec2::new(160.0, 100.0),
            lane_start: Vec2::new(160.0, 100.0),
            gap_angle: 0.0,
            pools: [Vec2::ZERO; POOL_VIEW],
            pool_r: [0.0; POOL_VIEW],
            pool_n: 0,
            hunting: false,
            arena: TileRect::new(0, 0, 20, 16),
        }
    }

    #[test]
    fn the_loop_has_exactly_one_gap_and_it_is_wide_enough_to_walk() {
        for progress in [0.0f32, 0.5, 1.0] {
            let v = view(CobraState::Constrict, progress);
            let mut pts = [Vec2::ZERO; COIL_MAX];
            let n = coil_chain(&v, &mut pts);
            assert!(n > 8, "the loop is a chain of body points");
            let r = coil_radius(&v);
            assert!((COIL_END_R - 0.01..=COIL_START_R + 0.01).contains(&r));
            for q in pts.iter().take(n) {
                let d = q.dist(v.target);
                assert!((d - r).abs() < 0.5, "every point is on the loop ({d} vs {r})");
            }
            // The doorway is COIL_GAP_ARC px wide at every radius, so the rat
            // (10 px across) always fits through it.
            let half = coil_gap_half(&v);
            let door = 2.0 * r * half.sin();
            assert!(door >= COIL_GAP_ARC * 0.85, "the gap is {door:.1} px wide");
            // No point sits inside the opening.
            let gap = coil_gap_angle(&v);
            for q in pts.iter().take(n) {
                let a = (*q - v.target).angle();
                let mut rel = (a - gap).abs();
                if rel > std::f32::consts::PI {
                    rel = std::f32::consts::TAU - rel;
                }
                assert!(
                    rel >= half - 0.01,
                    "a body point is standing in the only exit ({rel} < {half})"
                );
            }
            assert!(coil_radius(&view(CobraState::Constrict, 1.0)) < coil_radius(&view(CobraState::Constrict, 0.0)));
        }
    }

    #[test]
    fn the_loop_turns_so_the_gap_has_to_be_run_for() {
        let a = coil_gap_angle(&view(CobraState::Constrict, 0.0));
        let b = coil_gap_angle(&view(CobraState::Constrict, 1.0));
        assert!((b - a - COIL_TURN).abs() < 1e-3);
    }

    #[test]
    fn only_the_laid_states_block() {
        for state in [
            CobraState::Rear,
            CobraState::HoodTell,
            CobraState::Strike,
            CobraState::FangsStuck,
            CobraState::VenomTell,
            CobraState::TailSweep,
            CobraState::Loosen,
            CobraState::Death,
        ] {
            let mut pts = [Vec2::ZERO; COIL_MAX];
            assert_eq!(
                blocking_chain(&view(state, 0.5), &mut pts),
                0,
                "{state:?} must not put a wall on the floor"
            );
        }
        let mut pts = [Vec2::ZERO; COIL_MAX];
        assert!(blocking_chain(&view(CobraState::Constrict, 0.5), &mut pts) > 0);
        assert!(blocking_chain(&view(CobraState::SpiralChase, 0.5), &mut pts) > 0);
    }

    #[test]
    fn the_spiral_lane_has_walkable_turns_and_an_open_end() {
        let v = view(CobraState::SpiralChase, 0.5);
        let mut pts = [Vec2::ZERO; COIL_MAX];
        let n = spiral_chain(&v, &mut pts);
        assert!(n > 8, "the lane is laid out of body points");
        let r_max = spiral_radius(&v);
        let mut prev_r = 0.0f32;
        for q in pts.iter().take(n) {
            let r = q.dist(v.lane_start);
            assert!(r <= r_max + 0.5, "the lane stays inside the arena");
            assert!(r >= prev_r - SPIRAL_TURN * 2.0);
            prev_r = r;
        }
        // Spacing is tight enough that a 10 px rat cannot walk between two
        // points, and the lane between turns is wider than it is.
        let mut doorways = 0;
        for i in 1..n {
            let d = pts[i].dist(pts[i - 1]);
            assert!(d > 1.0);
            // Spaced by arc length; the radius grows along the lane too, so
            // a chord near the centre runs a little over one spacing.
            if d <= SPIRAL_SPACING * 1.2 {
                assert!(d < 2.0 * (COIL_BODY_HALF + 5.0));
            } else {
                // The one opening in the outermost turn: a real doorway.
                doorways += 1;
                assert!(d >= 2.0 * (COIL_BODY_HALF + 5.0), "a {d:.1} px slit is not a door");
            }
        }
        assert!(doorways <= 1, "the lane has one open end, not {doorways}");
        // Turns stay walkable.
        const _: () = assert!(SPIRAL_TURN > 2.0 * (COIL_BODY_HALF + 5.0));
    }

    #[test]
    fn one_weak_point_and_it_is_the_hood() {
        for state in [
            CobraState::Rear,
            CobraState::HoodTell,
            CobraState::Strike,
            CobraState::VenomTell,
            CobraState::Constrict,
            CobraState::SpiralChase,
        ] {
            assert!(
                weak_point(&view(state, 0.5)).is_none(),
                "{state:?} has no weak point"
            );
        }
        for state in [CobraState::FangsStuck, CobraState::SpiralEnd] {
            let v = view(state, 0.5);
            let (at, r) = weak_point(&v).expect("the open hood is exposed");
            assert!((at - hood_pos(&v)).len() < 1e-3);
            assert!(r > 6.0);
            let want = if state == CobraState::FangsStuck {
                STUCK_DAMAGE_MULT
            } else {
                SPENT_DAMAGE_MULT
            };
            assert!((incoming_damage_mult(state) - want).abs() < 1e-6);
            assert!(!has_body_contact(state), "a prone head cannot bump");
            // The weak point is the first auto-aim circle.
            let circles = body_hit_circles(&v);
            assert!((circles[0].0 - at).len() < 1e-3);
        }
        assert!(has_body_contact(CobraState::Rear));
        assert_eq!(incoming_damage_mult(CobraState::Rear), 1.0);
    }

    #[test]
    fn the_sway_has_three_beats() {
        // Sampled at the six crests (three beats, each a left and a right).
        let beats = [1.0f32 / 12.0, 3.0 / 12.0, 5.0 / 12.0, 7.0 / 12.0, 9.0 / 12.0, 11.0 / 12.0]
            .iter()
            .filter(|t| sway_phase(&view(CobraState::SwayTell, **t)).abs() > 0.2)
            .count();
        assert!(beats >= 3, "the rhythm is legible");
        // The phase is periodic, not monotonic: the strike reads as a beat.
        let a = sway_phase(&view(CobraState::SwayTell, 0.1));
        let b = sway_phase(&view(CobraState::SwayTell, 0.1));
        assert_eq!(a, b, "pure function of the state, no hidden clock");
    }
}
