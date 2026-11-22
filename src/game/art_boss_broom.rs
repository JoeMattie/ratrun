//! Art and gameplay specification for **The Sweeper** (Broom Boss — lower half only).
//!
//! # Rat's-Eye Concept
//! The boss is a human so enormous that only their lower half fits in the world:
//! two chunky scuffed oxblood leather house-slippers, ribbed cream gym socks
//! (with a frayed hole on the right ankle showing warm skin), faded indigo denim
//! trouser legs with turned-up light-wash cuffs that rise upward (`screen -y`)
//! and fade cleanly out of the top of the camera frustum, and a wide corn-straw
//! wooden broom (plus a dropped galvanized tin dustpan) wielded from off-screen
//! above.
//!
//! # Rendering Layers
//! Call the two draw entry points in this order inside `World::draw`:
//! 1. [`draw_ground`] — **Ground layer**: call *after* `art_floor::draw_floor`
//!    and floor stains, *before* props, enemies, bullets, and the player. Draws
//!    planted/lifted foot shadows, stomp landing rings, sweep arc telegraphs,
//!    jab target reticles, dustpan drop zones, impact shockwaves, floor dust,
//!    and the boss's diegetic ground HP arc.
//! 2. [`draw`] — **Entity layer**: call during the entity pass (with or just
//!    after regular enemies). Draws the two slippers, ribbed socks (with frayed
//!    hole), cuffed rising trouser legs (fading seamlessly toward `-y`), the
//!    corn-straw broom head & rising handle, the falling/bouncing tin dustpan,
//!    hurt-hop pain stars, and Phase 2 anger cues.
//!
//! # Combat & Hitbox Contract
//! **The feet are the hittable part.** Use [`foot_hitboxes`] to query the two
//! feet's exact world-space centers, collision radii, and grounded flags:
//! - Grounded feet (`grounded == true`) take weapon damage and block/bump the
//!   player.
//! - A lifted foot (`grounded == false`, e.g. mid-stomp or the injured foot
//!   during `Hurt`) is high above the rat and cannot be hit by ground weapons.
//! - Use [`broom_head_pos`] and [`broom_sweep_angle`] so attack hitboxes align
//!   pixel-for-pixel with the drawn bristles.
//!
//! # Reference Scale & Fairness Numbers
//! - Player rat: `radius = 3.0 px`, `100 HP`, walk speed `58 px/s`, dash
//!   `0.16 s` @ `3.2x` (`~27 px` burst, `0.18 s` i-frames, `2.0 s` cd).
//! - Damage benchmarks: Skitterer `6`, Cat `12`, Brute `22`, Old Boss `30`.
//! - Suggested boss HP pool: `~1600 HP` (Phase 2 triggers at `hp_frac <= 0.50`).

#![allow(dead_code)]

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

// ============================================================================
// Public Collision & Attack Geometry Constants
// ============================================================================

/// Half-distance between the left and right foot centers along `perp` (world px).
/// Total stance center-to-center width is `21.0 px`, leaving a narrow dashable
/// gap between the shoes for a `3.0 px`-radius rat.
pub const STANCE_HALF_WIDTH: f32 = 10.5;

/// Single-circle bounding hit radius of one planted foot (world px).
/// Matches the chunky slipper footprint (`17.5 px` long × `9.2 px` wide) for
/// fair bullet-to-foot collision checks.
pub const FOOT_HIT_RADIUS: f32 = 6.0;

/// Shoe capsule half-length along the foot's facing axis (world px).
/// Combine with [`FOOT_CAPSULE_RADIUS`] for exact shoe-shaped collision.
pub const FOOT_CAPSULE_HALF_LEN: f32 = 4.2;

/// Shoe capsule radius across the foot's width (world px).
pub const FOOT_CAPSULE_RADIUS: f32 = 4.5;

/// Direct crush radius beneath a landing stomp foot (world px).
pub const STOMP_CRUSH_RADIUS: f32 = 7.5;

/// Outer shockwave radius of a `StompWalk` impact (world px).
pub const STOMP_SHOCKWAVE_RADIUS: f32 = 16.0;

/// Inner radius of the `BroomSweep` danger band (world px).
/// Inside `r < 12.0` right beside the slippers is a tight safe pocket.
pub const SWEEP_INNER_RADIUS: f32 = 12.0;

/// Outer reach of the `BroomSweep` danger band (world px).
pub const SWEEP_OUTER_RADIUS: f32 = 38.0;

/// Half-angle of the `BroomSweep` arc on each side of `facing` (radians;
/// `1.22 rad ≈ 70°`, total sweep arc `140°`).
pub const SWEEP_HALF_ARC: f32 = 1.22;

/// Radius of the active bristle head during `BroomJab` impacts (world px).
pub const JAB_HEAD_RADIUS: f32 = 8.5;

/// Maximum forward reach of the 3rd lunge in `BroomJab` (world px).
pub const JAB_MAX_REACH: f32 = 32.0;

/// Impact blast radius of the dropped tin dustpan in `DustpanDrop` (world px).
pub const DUSTPAN_IMPACT_RADIUS: f32 = 15.0;

/// Number of evenly spaced radial clatter shards spawned on `DustpanDrop` impact.
pub const DUSTPAN_SHARD_COUNT: usize = 6;

/// Shockwave pushback radius of the `PhaseChange` double-foot tantrum slam (world px).
pub const PHASE_SLAM_RADIUS: f32 = 26.0;

/// Recommended outer culling margin around `view.pos` (and `view.target`) in world px.
pub const CULL_MARGIN: f32 = 60.0;

// ============================================================================
// Public View & Move-Set State Types
// ============================================================================

/// Complete move set and behavioural specification for **The Sweeper**.
///
/// Every state progresses linearly via `BroomBossView::progress` from `0.0` to
/// `1.0`. Timings below list **Phase 1 / Phase 2 (`enraged == true`)** values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BroomBossState {
    /// **Standing & Shifting Weight (Recovery / Neutral)**
    /// - **Action**: Both slippers planted on the floor, subtle weight-shift
    ///   breathing (and an impatient toe-tap when `enraged`), broom resting
    ///   upright by the right flank.
    /// - **Duration**: `0.55 s` (Phase 1) / `0.32 s` (Phase 2).
    /// - **Warning**: None (non-attacking recovery beat).
    /// - **Dangerous area**: Planted foot hitboxes only ([`foot_hitboxes`],
    ///   `radius = 6.0 px` each) deal light contact bump damage.
    /// - **Damage**: `10 dmg` contact bump (standard contact cooldown).
    /// - **Counterplay**: Prime close-range DPS window on either foot before
    ///   the boss initiates its next move.
    Idle,

    /// **Alternating Giant Stomp-Step (`left_foot`: `true` = left, `false` = right)**
    /// - **Action**: The human marches toward the player by lifting one huge
    ///   foot high off the floor and slamming it down onto `view.target`
    ///   (recommended stride: `14..24 px` toward player, offset `±STANCE_HALF_WIDTH`
    ///   laterally from the new `pos`). Alternate `left_foot` each step.
    /// - **Duration**: `1.15 s` (Phase 1) / `0.85 s` (Phase 2).
    /// - **Warning (`progress 0.00..0.58`, `0.67 s` P1 / `0.49 s` P2)**:
    ///   The stomping foot rises high toward `-y` (unhittable while aloft) while
    ///   a darkening shoe-shaped shadow and a pulsing [`STOMP_SHOCKWAVE_RADIUS`]
    ///   (`16 px`) landing ring with an expanding inner timing gauge lock onto
    ///   `view.target`.
    /// - **Dangerous area (`progress 0.66..0.78`, `0.14 s` active window)**:
    ///   - Inner crush circle around `view.target`, `radius = 7.5 px` ([`STOMP_CRUSH_RADIUS`]).
    ///   - Outer shockwave ring around `view.target`, `7.5..16.0 px` ([`STOMP_SHOCKWAVE_RADIUS`]).
    /// - **Damage**: `26 dmg` direct crush (`30 dmg` P2); `16 dmg` shockwave
    ///   (`20 dmg` P2) + `110 px/s` radial knockback.
    /// - **Counterplay**: Walk out of the `16 px` circle during the `0.67 s`
    ///   telegraph (rat covers `~39 px` walking) while shooting the opposite
    ///   planted foot.
    StompWalk { left_foot: bool },

    /// **Wide Bristle Rake (`clockwise`: `true` = left-to-right, `false` = right-to-left)**
    /// - **Action**: Both feet plant firmly as the broom head draws far back to
    ///   one flank, then rakes across a `140°` arc in front of `facing`, shoving
    ///   everything it touches. In Phase 2, the implementer may immediately chain
    ///   a second `BroomSweep` with `!clockwise`.
    /// - **Duration**: `1.65 s` (Phase 1) / `1.25 s` (Phase 2).
    /// - **Warning (`progress 0.00..0.46`, `0.76 s` P1 / `0.58 s` P2)**:
    ///   Bristles pull back and quiver at the start angle while `draw_ground`
    ///   paints a translucent warning sector (`r = 12..38 px`, `±1.22 rad`
    ///   around `facing`) with a filling progress edge showing sweep direction.
    /// - **Dangerous area (`progress 0.46..0.72`, `0.43 s` P1 / `0.33 s` P2)**:
    ///   Moving annular wedge swept by the broom head: `r ∈ [12.0, 38.0]` px
    ///   ([`SWEEP_INNER_RADIUS`]..[`SWEEP_OUTER_RADIUS`]), angular width
    ///   `±0.28 rad` around [`broom_sweep_angle`].
    /// - **Damage**: `20 dmg` (`24 dmg` P2) + heavy `220 px/s` shove impulse
    ///   along the sweep tangent + outward (also shoves minions/pickups).
    /// - **Counterplay**: Tuck inside the `r < 12 px` pocket right beside the
    ///   slippers, circle behind the heels, or dash (`0.18 s` i-frames) straight
    ///   through the sweeping bristles.
    BroomSweep { clockwise: bool },

    /// **Frantic Jab-Jab-Jab (3-Hit Rapid Poke Combo)**
    /// - **Action**: Startled by the rat, the human stabs the broom head straight
    ///   down three rapid times in front of `facing`: Jab 1 slightly left
    ///   (`+20 u, -7.5 v`), Jab 2 slightly right (`+24 u, +7.5 v`), Jab 3 deep
    ///   lunge (`+32 u, 0 v` or toward `view.target` clamped to [`JAB_MAX_REACH`]).
    /// - **Duration**: `1.80 s` (Phase 1) / `1.40 s` (Phase 2), split into three
    ///   beats (`0.00..0.32`, `0.32..0.64`, `0.64..1.00`).
    /// - **Warning (first `55%` of each beat, `~0.32 s` P1 / `~0.25 s` P2)**:
    ///   Broom head lifts `11.5 px` above the next impact spot while a crisp
    ///   `radius = 8.5 px` target reticle locks on the floor beneath it.
    /// - **Dangerous area (middle `20%` of each beat: `0.17..0.24`, `0.49..0.56`,
    ///   `0.81..0.88`)**:
    ///   Circle of `radius = 8.5 px` ([`JAB_HEAD_RADIUS`]) centered at
    ///   [`broom_head_pos`].
    /// - **Damage**: `14 dmg` per jab (`18 dmg` P2) + `80 px/s` knockback.
    /// - **Counterplay**: Strafe laterally across `facing` (each jab is narrow)
    ///   or flank behind the heels; after Jab 3 (`0.88..1.00`) the broom pauses
    ///   briefly stuck in the floor for free foot damage.
    BroomJab,

    /// **Dropped Tin Dustpan Slam & Clatter (Ranged Bullet-Hell Beat)**
    /// - **Action**: The human fumbles/drops their heavy galvanized tin dustpan
    ///   onto `view.target` (recommended range `22..56 px` from `view.pos`, aimed
    ///   at the player's position at state start). Impact rattles the pan and
    ///   scatters [`DUSTPAN_SHARD_COUNT`] (`6`) radial debris projectiles.
    /// - **Duration**: `1.45 s` (Phase 1) / `1.15 s` (Phase 2).
    /// - **Warning (`progress 0.00..0.46`, `0.67 s` P1 / `0.53 s` P2)**:
    ///   A `radius = 15.0 px` ([`DUSTPAN_IMPACT_RADIUS`]) warning ring with `6`
    ///   radial lane ticks pulses at `view.target` while the tumbling tin dustpan
    ///   descends from `44 px` above (`screen -y`).
    /// - **Dangerous area (`progress 0.46..0.58`)**:
    ///   - Slam circle at `view.target`, `radius = 15.0 px`.
    ///   - At `progress == 0.48`, spawn `6` enemy projectiles (`radius = 2.0 px`,
    ///     speed `48 px/s` P1 / `60 px/s` P2, angles `k * TAU / 6 + 0.25`).
    /// - **Damage**: `22 dmg` (`26 dmg` P2) on dustpan slam; `10 dmg` per shard.
    /// - **Counterplay**: Step out of the `15 px` drop circle, weave through the
    ///   wide `60°` gaps between shards (previewed by the ground ticks!), and
    ///   punish the frozen planted feet while the human yanks the pan back up.
    DustpanDrop,

    /// **Stubbed Toe / Hurt Stagger (`left_foot_hit`: which foot was struck)**
    /// - **Action**: Triggered when a foot sustains a heavy stagger threshold
    ///   (e.g. `~18%` max HP burst). The injured foot jerks high off the floor
    ///   tilted and throbbing with orbiting pain stars while the boss hops
    ///   comically on the other foot twice and waves the broom off-balance.
    /// - **Duration**: `1.20 s`.
    /// - **Warning**: Immediate comedic reaction (interrupts recovery or weak wind-up).
    /// - **Dangerous area**: None active (only a tiny `6 dmg` bump if the player
    ///   walks straight into the single hopping foot).
    /// - **Damage**: `0 dmg` (bump `6 dmg`).
    /// - **Counterplay**: Bonus punish window — track the hopping planted foot
    ///   for extra damage while the boss cannot attack.
    Hurt { left_foot_hit: bool },

    /// **Phase 2 Anger Tantrum (Triggers Once at `hp_frac <= 0.50`)**
    /// - **Action**: The human loses patience, stamps rapidly in place, then
    ///   slams both feet and the broom butt down together, entering `enraged = true`.
    /// - **Duration**: `1.80 s`.
    /// - **Warning (`progress 0.00..0.38`, `0.68 s`)**:
    ///   Rapid alternating ankle stamps, crimson-amber anger steam rising from
    ///   the cuffs, and an expanding `radius = 26.0 px` ([`PHASE_SLAM_RADIUS`])
    ///   warning ring charging around `view.pos`.
    /// - **Dangerous area (`progress 0.38..0.54`, `0.29 s`)**:
    ///   Circle around `view.pos`, `radius = 26.0 px`.
    /// - **Damage**: `18 dmg` + heavy `200 px/s` radial knockback (clears nearby
    ///   minions/player projectiles for breathing room). Recommend `50%` damage
    ///   reduction during `0.38..0.54` so the transition always reads cleanly.
    /// - **Counterplay**: Back outside `26 px` when the rapid stamping starts;
    ///   after `0.54`, resume attacking during the `0.83 s` enraged stance settle.
    PhaseChange,

    /// **Defeat — Gives Up & Shuffles Off**
    /// - **Action**: At `0 HP`, the human drops the broom flat onto the floor
    ///   (`0.00..0.32`), then turns and shuffles heavily away (`0.32..1.00`),
    ///   fading out over `0.70..1.00` while the abandoned broom stays behind.
    /// - **Duration**: `2.40 s`.
    /// - **Warning**: None.
    /// - **Dangerous area**: None (`0 dmg`, non-colliding, untargetable).
    /// - **Damage**: `0 dmg`.
    /// - **Counterplay**: Victory lap! Collect XP/loot once the boss despawns.
    Defeat,
}

/// Pure, `Copy` render-state snapshot of **The Sweeper**.
///
/// Decoupled from all simulation types so the art and gameplay specs compile
/// cleanly standalone.
#[derive(Clone, Copy, Debug)]
pub struct BroomBossView {
    /// World-space anchor: midpoint between the two feet on the floor.
    pub pos: Vec2,
    /// Facing direction (need not be unit length; defaults to `(0, 1)` if zero).
    pub facing: Vec2,
    /// Current move / behavioural state.
    pub state: BroomBossState,
    /// Normalized progress through `state`, clamped to `0.0..=1.0`.
    pub progress: f32,
    /// Continuous animation clock in seconds (idle sway, anger sparks, pulses).
    pub anim: f32,
    /// Hit-flash intensity in `0.0..=1.0` (`1.0` right when a foot is struck).
    pub hit_flash: f32,
    /// Remaining health fraction in `0.0..=1.0`.
    pub hp_frac: f32,
    /// World-space telegraph/aim target (stomp landing point in `StompWalk`,
    /// lunge focus in `BroomJab`, drop point in `DustpanDrop`).
    pub target: Vec2,
    /// `true` once the boss has completed `PhaseChange` (`hp_frac <= 0.50`).
    pub enraged: bool,
}

/// World-space hitbox for one of the boss's feet.
#[derive(Clone, Copy, Debug)]
pub struct FootHitbox {
    /// World-space floor projection of the foot center.
    pub pos: Vec2,
    /// Single-circle collision radius ([`FOOT_HIT_RADIUS`]).
    pub radius: f32,
    /// `true` when planted on the floor (hittable by player weapons & solid).
    /// `false` when lifted high into the air (mid-stomp or injured hurt foot).
    pub grounded: bool,
    /// `true` for the left foot, `false` for the right foot.
    pub is_left: bool,
}

// ============================================================================
// Public Geometry Queries (Art <-> Hitbox Contract)
// ============================================================================

/// Returns the exact world-space hitboxes of `[left_foot, right_foot]` matching
/// the rendered slippers for the given `view`.
pub fn foot_hitboxes(view: &BroomBossView) -> [FootHitbox; 2] {
    let pose = compute_pose(view);
    [
        FootHitbox {
            pos: pose.left.ground,
            radius: FOOT_HIT_RADIUS,
            grounded: pose.left.grounded,
            is_left: true,
        },
        FootHitbox {
            pos: pose.right.ground,
            radius: FOOT_HIT_RADIUS,
            grounded: pose.right.grounded,
            is_left: false,
        },
    ]
}

/// Returns the exact world-space center of the broom's bristle head for the
/// current state and progress. Use this for `BroomJab` and `BroomSweep` hits.
pub fn broom_head_pos(view: &BroomBossView) -> Vec2 {
    compute_pose(view).broom.ground
}

/// Returns the current world-space angle (radians) of the broom relative to
/// `view.pos` during `BroomSweep` (or the resting broom angle otherwise).
pub fn broom_sweep_angle(view: &BroomBossView) -> f32 {
    let pose = compute_pose(view);
    (pose.broom.ground - view.pos).angle()
}

// ============================================================================
// Palette & Lighting Constants
// ============================================================================

/// Consistent top-left directional light shared with `art_rat.rs` & `art_enemy.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// Chunky scuffed oxblood-chestnut leather slippers.
const SHOE_SH: Rgb = (54, 22, 20);
const SHOE_MID: Rgb = (150, 62, 46);
const SHOE_HI: Rgb = (220, 126, 84);
const SHOE_SCUFF: Rgb = (230, 194, 154);
const SHOE_SOLE: Rgb = (32, 22, 20);
const SHOE_FLEECE_SH: Rgb = (140, 128, 114);
const SHOE_FLEECE_MID: Rgb = (196, 184, 164);
const SHOE_FLEECE_HI: Rgb = (234, 222, 202);

// Ribbed cream gym socks + double athletic stripe + frayed skin hole.
const SOCK_SH: Rgb = (132, 126, 118);
const SOCK_MID: Rgb = (208, 202, 188);
const SOCK_HI: Rgb = (244, 240, 230);
const SOCK_STRIPE: Rgb = (182, 48, 54);
const SOCK_HOLE_RIM: Rgb = (60, 44, 40);
const SKIN_SH: Rgb = (192, 118, 94);
const SKIN_HI: Rgb = (248, 178, 148);

// Faded indigo denim trousers + light-wash turned-up cuff + gold topstitch.
const PANT_SH: Rgb = (30, 42, 70);
const PANT_MID: Rgb = (68, 92, 138);
const PANT_HI: Rgb = (128, 158, 206);
const CUFF_SH: Rgb = (86, 106, 148);
const CUFF_MID: Rgb = (146, 170, 208);
const CUFF_HI: Rgb = (200, 218, 244);
const STITCH_GOLD: Rgb = (214, 170, 88);

// Corn-straw broom + barn-red shroud + steel wire + lacquered oak handle.
const STRAW_SH: Rgb = (96, 62, 24);
const STRAW_MID: Rgb = (196, 146, 56);
const STRAW_HI: Rgb = (250, 218, 118);
const CAP_SH: Rgb = (92, 24, 24);
const CAP_MID: Rgb = (186, 48, 44);
const CAP_HI: Rgb = (240, 104, 84);
const WIRE_COL: Rgb = (208, 218, 228);
const WOOD_SH: Rgb = (78, 46, 24);
const WOOD_MID: Rgb = (166, 112, 58);
const WOOD_HI: Rgb = (230, 180, 118);

// Galvanized tin dustpan + rubber lip.
const PAN_SH: Rgb = (68, 82, 98);
const PAN_MID: Rgb = (144, 166, 188);
const PAN_HI: Rgb = (226, 238, 250);
const PAN_RUBBER: Rgb = (28, 30, 36);

// Telegraphs, dust & anger cues.
const WARN_AMBER: Rgb = (255, 184, 62);
const WARN_RED: Rgb = (255, 70, 64);
const WARN_HOT: Rgb = (255, 242, 184);
const DUST_COL: Rgb = (204, 196, 180);
const ANGER_COL: Rgb = (255, 84, 48);

// ============================================================================
// Internal Pose Kinematics (Zero Allocation)
// ============================================================================

#[derive(Clone, Copy)]
struct FootPose {
    /// World-space floor projection (where the shadow sits).
    ground: Vec2,
    /// Vertical lift off the ground in world pixels (shifts drawn foot `-y`).
    lift: f32,
    /// Unit facing direction of this specific foot.
    face: Vec2,
    /// Slight perspective scale-up when lifted toward the overhead camera.
    scale: f32,
    /// Whether this foot is grounded (hittable).
    grounded: bool,
    /// Whether this foot is currently hurt-shaking (shows toe-stub stars + red throb).
    hurt_shake: bool,
    /// Visual tilt of slipper on the ankle (radians; askew when hurt/enraged).
    skew: f32,
}

#[derive(Clone, Copy)]
struct BroomPose {
    /// World-space floor projection of the broom head center.
    ground: Vec2,
    /// Vertical lift off the floor in world pixels (`-y` offset).
    lift: f32,
    /// Unit direction pointing from the red cap toward the bristle tips.
    dir: Vec2,
    /// Bristle splay multiplier (`1.0` normal, `>1.0` squashed on jab impact).
    splay: f32,
    /// Bristle tip bend along `dir.perp()` during sweeps (`-1..1`).
    bend: f32,
    /// `true` when dropped flat on the floor (Defeat).
    dropped_flat: bool,
    /// World-space top point where the handle aims toward the unseen hands.
    hand_target: Vec2,
}

#[derive(Clone, Copy)]
struct DustpanPose {
    visible: bool,
    ground: Vec2,
    lift: f32,
    angle: f32,
    tilt: f32,
    alpha: f32,
}

#[derive(Clone, Copy)]
struct BossPose {
    pos: Vec2,
    face: Vec2,
    perp: Vec2,
    left: FootPose,
    right: FootPose,
    broom: BroomPose,
    dustpan: DustpanPose,
    /// Overall body opacity (`1.0` normally, fades to `0.0` at end of `Defeat`).
    body_alpha: f32,
    /// Broom opacity (stays `1.0` longer during `Defeat` after being dropped).
    broom_alpha: f32,
    /// Intensity of enraged visual cues (`0.0..1.0`).
    anger: f32,
}

#[inline]
fn norm_face(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-4 {
        v.normalized()
    } else {
        Vec2::new(0.0, 1.0)
    }
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[inline]
fn lerp_vec(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    a + (b - a) * t
}

fn compute_pose(view: &BroomBossView) -> BossPose {
    let p = view.progress.clamp(0.0, 1.0);
    let f = norm_face(view.facing);
    let perp = f.perp();
    let sway = (view.anim * 2.4).sin();
    let anger = if view.enraged {
        1.0
    } else if matches!(view.state, BroomBossState::PhaseChange) {
        (p * 2.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Slightly wider, natural outward toe-splay so shoes read clearly at every heading
    // (including facing UP, where splayed toes peek past the rising cuffs!).
    let stance = STANCE_HALF_WIDTH + if anger > 0.5 { 1.0 } else { 0.0 };
    let base_l = view.pos - perp * stance + f * (sway * 0.6);
    let base_r = view.pos + perp * stance - f * (sway * 0.6);
    let toe_out = 0.22;
    let face_l = f.rotate(-toe_out);
    let face_r = f.rotate(toe_out);

    let mut left = FootPose {
        ground: base_l,
        lift: 0.0,
        face: face_l,
        scale: 1.0,
        grounded: true,
        hurt_shake: false,
        skew: 0.0,
    };
    let mut right = FootPose {
        ground: base_r,
        lift: 0.0,
        face: face_r,
        scale: 1.0,
        grounded: true,
        hurt_shake: false,
        skew: if anger > 0.5 { 0.16 } else { 0.0 },
    };

    // Default resting broom beside the right flank, angled slightly forward.
    let rest_broom_pos = view.pos + perp * (stance + 9.5) + f * (6.5 + sway * 1.1);
    let mut broom = BroomPose {
        ground: rest_broom_pos,
        lift: 0.0,
        dir: f.rotate(0.28),
        splay: 1.0,
        bend: 0.0,
        dropped_flat: false,
        hand_target: view.pos + perp * 7.5 + Vec2::new(0.0, -35.0),
    };

    let mut dustpan = DustpanPose {
        visible: false,
        ground: view.target,
        lift: 0.0,
        angle: f.angle(),
        tilt: 0.0,
        alpha: 0.0,
    };

    let mut body_alpha = 1.0f32;
    let mut broom_alpha = 1.0f32;

    match view.state {
        BroomBossState::Idle => {
            if anger > 0.5 {
                // Impatient toe-tap on the right slipper when enraged.
                let tap = (view.anim * 9.0).sin().max(0.0);
                right.lift = tap * 2.4;
                broom.ground += perp * ((view.anim * 6.0).sin() * 1.1);
            }
        }

        BroomBossState::StompWalk { left_foot } => {
            let default_land = if left_foot {
                base_l + f * 18.0
            } else {
                base_r + f * 18.0
            };
            let land = if view.target.dist_sq(view.pos) > 9.0 {
                view.target
            } else {
                default_land
            };
            let start = if left_foot { base_l } else { base_r };
            let active = if left_foot { &mut left } else { &mut right };

            if p < 0.58 {
                let t = p / 0.58;
                let lift_env = (t * PI * 0.72).sin().clamp(0.0, 1.0);
                active.ground = lerp_vec(start, land, smoothstep(t));
                active.lift = 16.5 * lift_env;
                active.scale = 1.0 + 0.13 * lift_env;
                active.grounded = active.lift < 3.0;
            } else if p < 0.66 {
                let t = (p - 0.58) / 0.08;
                let rem = 1.0 - t * t;
                active.ground = land;
                active.lift = 12.8 * rem;
                active.scale = 1.0 + 0.10 * rem;
                active.grounded = t > 0.7;
            } else {
                let settle = ((p - 0.66) / 0.34).clamp(0.0, 1.0);
                let squash = (1.0 - settle).powi(2);
                active.ground = land;
                active.lift = 0.0;
                active.scale = 1.0 + 0.07 * squash;
                active.grounded = true;
            }

            let b_lift = if p < 0.66 {
                3.2 * (p / 0.66 * PI).sin()
            } else {
                0.0
            };
            broom.lift = b_lift;
            broom.ground = rest_broom_pos + f * (if p < 0.66 { -2.5 } else { 2.0 });
        }

        BroomBossState::BroomSweep { clockwise } => {
            let sign = if clockwise { 1.0 } else { -1.0 };
            let start_ang = -sign * SWEEP_HALF_ARC;
            let end_ang = sign * SWEEP_HALF_ARC;

            let (rel_ang, reach, lift, bend, splay) = if p < 0.46 {
                let t = smoothstep(p / 0.46);
                let quiver = (view.anim * 34.0).sin() * 0.055 * t;
                let a = start_ang - sign * 0.24 * t + quiver;
                (a, 24.0 + 3.0 * t, 3.8 * t, -sign * 0.55 * t, 1.0 + 0.15 * t)
            } else if p < 0.72 {
                let t = (p - 0.46) / 0.26;
                let ease = t * t * (3.0 - 2.0 * t);
                let a = start_ang + (end_ang - start_ang) * ease;
                let r = 27.0 + 4.5 * (t * PI).sin();
                (a, r, 0.0, -sign * 1.05, 1.24)
            } else {
                let t = smoothstep((p - 0.72) / 0.28);
                let a = end_ang + sign * 0.12 * (1.0 - t);
                (a, 27.0 - 5.0 * t, 2.0 * t, -sign * 0.35 * (1.0 - t), 1.05)
            };

            let sweep_dir = f.rotate(rel_ang);
            broom.ground = view.pos + sweep_dir * reach;
            broom.lift = lift;
            broom.dir = sweep_dir;
            broom.bend = bend;
            broom.splay = splay;
            broom.hand_target = view.pos + sweep_dir * 4.0 + Vec2::new(0.0, -35.0);

            left.ground = view.pos - perp * (stance + 1.5) - f * (1.5 * sign);
            right.ground = view.pos + perp * (stance + 1.5) + f * (1.5 * sign);
        }

        BroomBossState::BroomJab => {
            let (beat, bt) = if p < 0.32 {
                (0usize, p / 0.32)
            } else if p < 0.64 {
                (1usize, (p - 0.32) / 0.32)
            } else {
                (2usize, (p - 0.64) / 0.36)
            };

            let jab3_pos = {
                let to_t = view.target - view.pos;
                if to_t.len_sq() > 64.0 {
                    view.pos + to_t.clamp_len(JAB_MAX_REACH)
                } else {
                    view.pos + f * JAB_MAX_REACH
                }
            };
            let targets = [
                view.pos + f * 20.0 - perp * 7.5,
                view.pos + f * 24.0 + perp * 7.5,
                jab3_pos,
            ];
            let prev = if beat == 0 {
                rest_broom_pos
            } else {
                targets[beat - 1]
            };
            let curr = targets[beat];

            if bt < 0.52 {
                let t = smoothstep(bt / 0.52);
                broom.ground = lerp_vec(prev, curr, t);
                broom.lift = 11.5 * (t * PI * 0.75).sin().max(0.25);
                broom.splay = 0.92;
            } else if bt < 0.74 {
                let t = (bt - 0.52) / 0.22;
                let impact = (1.0 - t).powi(2);
                broom.ground = curr;
                broom.lift = 0.0;
                broom.splay = 1.0 + 0.42 * impact;
            } else {
                let t = (bt - 0.74) / 0.26;
                broom.ground = curr;
                broom.lift = if beat == 2 { 2.0 * t } else { 5.5 * t };
                broom.splay = 1.0;
            }
            broom.dir = (broom.ground - view.pos).normalized();
            if broom.dir.len_sq() < 0.1 {
                broom.dir = f;
            }
            broom.hand_target = view.pos + f * 6.0 + Vec2::new(0.0, -35.0);

            if beat == 2 {
                let lunge = smoothstep(bt.min(0.6) / 0.6) * 3.0;
                right.ground += f * lunge;
                left.ground -= f * (lunge * 0.5);
            }
        }

        BroomBossState::DustpanDrop => {
            let drop_pos = if view.target.dist_sq(view.pos) > 100.0 {
                view.target
            } else {
                view.pos + f * 36.0
            };

            dustpan.visible = true;
            dustpan.ground = drop_pos;
            dustpan.angle = (drop_pos - view.pos).angle();

            if p < 0.46 {
                let t = p / 0.46;
                let fall = 1.0 - t * t;
                dustpan.lift = 44.0 * fall;
                dustpan.tilt = (1.0 - t) * 1.4;
                dustpan.alpha = (t * 2.4).clamp(0.0, 1.0);
            } else if p < 0.82 {
                let t = (p - 0.46) / 0.36;
                let bounce = if t < 0.35 {
                    (t / 0.35 * PI).sin() * 5.5
                } else if t < 0.62 {
                    ((t - 0.35) / 0.27 * PI).sin() * 2.0
                } else {
                    0.0
                };
                dustpan.lift = bounce;
                dustpan.tilt = bounce * 0.08 * (view.anim * 26.0).sin();
                dustpan.alpha = 1.0;
            } else {
                let t = (p - 0.82) / 0.18;
                dustpan.lift = 40.0 * t * t;
                dustpan.tilt = -0.5 * t;
                dustpan.alpha = (1.0 - t * 1.15).clamp(0.0, 1.0);
            }

            let recoil = (p * PI).sin();
            broom.lift = 4.0 * recoil;
            broom.ground = rest_broom_pos - f * (4.0 * recoil);
        }

        BroomBossState::Hurt { left_foot_hit } => {
            // Classic cartoon "OW! MY TOE!" hop on the uninjured foot.
            let hop_wave = (p * TAU * 2.5).sin().abs();
            let hop_shift = (p * TAU * 1.25).sin() * 3.5;
            let wobble = (p * TAU * 7.5).sin();
            let side_dir = if left_foot_hit { -1.0 } else { 1.0 };

            let (hurt_f, hop_f) = if left_foot_hit {
                (&mut left, &mut right)
            } else {
                (&mut right, &mut left)
            };

            // Injured foot yanked high & cocked awkwardly outward while throbbing!
            let hold_env = (p * PI).sin().powf(0.42).clamp(0.0, 1.0);
            hurt_f.lift = 14.0 * hold_env + wobble.abs() * 1.8;
            hurt_f.scale = 1.0 + 0.12 * hold_env;
            hurt_f.grounded = false;
            hurt_f.hurt_shake = hold_env > 0.20;
            hurt_f.skew = side_dir * 0.52 * hold_env + 0.22 * wobble;
            hurt_f.ground += f * (-3.8 * hold_env) + perp * (side_dir * 2.2 * hold_env + wobble * 1.2);

            // Standing foot hops up & down awkwardly!
            hop_f.lift = hop_wave * 5.2 * hold_env;
            hop_f.grounded = hop_f.lift < 2.2;
            hop_f.ground += perp * hop_shift;
            hop_f.face = hop_f.face.rotate(wobble * 0.18);

            // Broom waves wildly out to the side for balance.
            broom.lift = 7.0 * hold_env + hop_wave * 3.2;
            broom.ground = rest_broom_pos + perp * (6.5 * hold_env) + f * (wobble * 4.2);
            broom.dir = f.rotate(0.65 + wobble * 0.38);
        }

        BroomBossState::PhaseChange => {
            if p < 0.38 {
                let t = p / 0.38;
                let stamp = (t * TAU * 3.0).sin();
                left.lift = stamp.max(0.0) * 9.0;
                right.lift = (-stamp).max(0.0) * 9.0;
                left.grounded = left.lift < 2.5;
                right.grounded = right.lift < 2.5;
                broom.lift = 5.5 * t;
                broom.ground = view.pos + f * 15.5 + perp * (stamp * 3.2);
                broom.dir = f;
            } else if p < 0.54 {
                let t = (p - 0.38) / 0.16;
                let squash = (1.0 - t).powi(2);
                left.scale = 1.0 + 0.10 * squash;
                right.scale = 1.0 + 0.10 * squash;
                left.ground = view.pos - perp * (stance + 2.5);
                right.ground = view.pos + perp * (stance + 2.5);
                broom.ground = view.pos + f * 18.0;
                broom.splay = 1.40;
                broom.dir = f;
            } else {
                let t = smoothstep((p - 0.54) / 0.46);
                left.ground = view.pos - perp * (stance + 2.5 - 1.5 * t);
                right.ground = view.pos + perp * (stance + 2.5 - 1.5 * t);
                right.skew = 0.18;
                broom.ground = lerp_vec(view.pos + f * 18.0, rest_broom_pos + f * 3.0, t);
                broom.splay = 1.16;
            }
        }

        BroomBossState::Defeat => {
            let drop_t = (p / 0.32).clamp(0.0, 1.0);
            broom.dropped_flat = drop_t > 0.35;
            broom.ground = view.pos + perp * (stance + 9.0) + f * (4.0 + 4.5 * drop_t);
            broom.lift = if drop_t < 0.35 {
                (1.0 - drop_t / 0.35) * 6.5
            } else if drop_t < 0.58 {
                ((drop_t - 0.35) / 0.23 * PI).sin() * 2.4
            } else {
                0.0
            };
            broom.dir = f.rotate(0.85);
            broom_alpha = if p > 0.88 {
                (1.0 - (p - 0.88) / 0.12).clamp(0.0, 1.0)
            } else {
                1.0
            };

            if p > 0.32 {
                let walk_t = (p - 0.32) / 0.68;
                let retreat_dir = (f * -0.5 + Vec2::new(0.0, -0.86)).normalized();
                let r_perp = retreat_dir.perp();
                let dist = walk_t * 28.0;
                let step_ph = walk_t * TAU * 2.5;
                let l_stride = step_ph.sin() * 5.2;
                let r_stride = (-step_ph).sin() * 5.2;
                let base = view.pos + retreat_dir * dist;

                left.ground = base - r_perp * (stance - 1.0) + retreat_dir * l_stride;
                right.ground = base + r_perp * (stance - 1.0) + retreat_dir * r_stride;
                left.lift = step_ph.cos().max(0.0) * 4.5;
                right.lift = (-step_ph).cos().max(0.0) * 4.5;
                left.face = retreat_dir.rotate(-0.18);
                right.face = retreat_dir.rotate(0.18);
                left.grounded = false;
                right.grounded = false;
                if p > 0.70 {
                    body_alpha = (1.0 - (p - 0.70) / 0.30).clamp(0.0, 1.0);
                }
            } else {
                left.grounded = false;
                right.grounded = false;
            }
        }
    }

    BossPose {
        pos: view.pos,
        face: f,
        perp,
        left,
        right,
        broom,
        dustpan,
        body_alpha,
        broom_alpha,
        anger,
    }
}

// ============================================================================
// Low-Level Pixel, Colour & SDF Helpers (Zero Allocation)
// ============================================================================

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> Vec2 {
    p - cam
}

#[inline]
fn in_view(pb: &PixelBuffer, view: &BroomBossView, cam: Vec2) -> bool {
    if pb.w == 0 || pb.h == 0 {
        return false;
    }
    let w = pb.w as f32;
    let h = pb.h as f32;
    let s = view.pos - cam;
    let main_vis = s.x >= -CULL_MARGIN
        && s.x <= w + CULL_MARGIN
        && s.y >= -CULL_MARGIN
        && s.y <= h + CULL_MARGIN;
    if main_vis {
        return true;
    }
    let t = view.target - cam;
    t.x >= -36.0 && t.x <= w + 36.0 && t.y >= -56.0 && t.y <= h + 36.0
}

#[inline]
fn bbox(pb: &PixelBuffer, center: Vec2, rx: f32, ry: f32) -> Option<(i32, i32, i32, i32)> {
    let x0 = (center.x - rx).floor() as i32;
    let x1 = (center.x + rx).ceil() as i32;
    let y0 = (center.y - ry).floor() as i32;
    let y1 = (center.y + ry).ceil() as i32;
    let x0 = x0.max(0);
    let x1 = x1.min(pb.w as i32 - 1);
    let y0 = y0.max(0);
    let y1 = y1.min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, x1, y0, y1))
    }
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
    if a >= 0.995 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.004 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    let bg = pb.pixel_at(ux, uy);
    pb.plot(x, y, scale(bg, (1.0 - amount).clamp(0.0, 1.0)));
}

#[inline]
fn shade3(sh: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(sh, mid, l / 0.62)
    } else {
        mix(mid, hi, (l - 0.62) / 0.53)
    }
}

/// Sculptural warm-gold hit flash that preserves local luminance & rim contrast.
#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.01 {
        return c;
    }
    let lum = (c.0 as f32 * 0.30 + c.1 as f32 * 0.59 + c.2 as f32 * 0.11) / 255.0;
    let flash = mix((235, 162, 92), (255, 248, 224), (0.35 + lum * 0.65).clamp(0.0, 1.0));
    mix(c, flash, (0.24 + hit * 0.48).min(0.72))
}

#[inline]
fn sd_ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> f32 {
    let nu = (u - cu) / ru;
    let nv = (v - cv) / rv;
    let d = (nu * nu + nv * nv).sqrt();
    (d - 1.0) * ru.min(rv)
}

#[inline]
fn sd_capsule(u: f32, v: f32, au: f32, av: f32, bu: f32, bv: f32, rad: f32) -> f32 {
    let pax = u - au;
    let pay = v - av;
    let bax = bu - au;
    let bay = bv - av;
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt() - rad
}

#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

/// Chunky anatomical slipper SDF in foot-local `(u, v)` coordinates
/// (`u` from `-7.0` heel to `+10.5` rounded toe box; `v * side` positive toward
/// outer edge, negative toward inner arch / big toe).
#[inline]
fn sd_slipper(u: f32, v: f32, side: f32, sc: f32) -> f32 {
    let vo = v * side;
    let heel = sd_ellipse(u, vo, -3.8 * sc, 0.1 * sc, 3.4 * sc, 3.55 * sc);
    let mid = sd_capsule(u, vo, -3.2 * sc, 0.4 * sc, 3.8 * sc, 0.25 * sc, 3.6 * sc);
    let toe = sd_ellipse(u, vo, 5.4 * sc, -0.35 * sc, 5.1 * sc, 4.45 * sc);
    smin(smin(heel, mid, 1.3 * sc), toe, 1.6 * sc)
}

/// Shortest signed angular difference `a - b` in `[-PI, PI]`.
#[inline]
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % TAU;
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    d
}

// ============================================================================
// Public Entry Point 1: Ground Layer (Shadows, Telegraphs, Shockwaves, HP Arc)
// ============================================================================

/// Draw the ground-layer elements for **The Sweeper**: foot shadows, attack
/// telegraphs (stomp landing ring, sweep wedge, jab reticles, dustpan drop
/// zone), impact shockwaves, floor dust puffs, and the boss's diegetic ground
/// HP arc.
///
/// **Layer**: Ground — call *after* `art_floor::draw_floor` and floor stains,
/// and *before* props, enemies, bullets, and the player rat.
pub fn draw_ground(pb: &mut PixelBuffer, view: &BroomBossView, cam: Vec2, accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let pose = compute_pose(view);
    let p = view.progress.clamp(0.0, 1.0);
    let pulse = 0.5 + 0.5 * (view.anim * 9.0).sin();

    // 1) Diegetic segmented ground HP arc beneath the boss (hidden during Defeat).
    if !matches!(view.state, BroomBossState::Defeat) && view.hp_frac > 0.0 {
        draw_ground_hp_arc(pb, w2s(pose.pos, cam), view.hp_frac, pose.anger, pulse);
    }

    // 2) State-specific attack telegraphs, shockwaves, and floor dust.
    match view.state {
        BroomBossState::StompWalk { left_foot } => {
            let foot = if left_foot { pose.left } else { pose.right };
            let c = w2s(foot.ground, cam);
            if p < 0.66 {
                let charge = (p / 0.66).clamp(0.0, 1.0);
                draw_stomp_telegraph(pb, c, STOMP_SHOCKWAVE_RADIUS, charge, pose.anger, pulse);
                // The inner crush circle is its own, heavier hit
                // (`STOMP_CRUSH_RADIUS`): mark its edge so the telegraph
                // shows both of the shapes the controller pushes.
                if let Some((x0, x1, y0, y1)) =
                    bbox(pb, c, STOMP_CRUSH_RADIUS + 1.0, STOMP_CRUSH_RADIUS + 1.0)
                {
                    let col = mix(WARN_RED, WARN_HOT, 0.25 + 0.35 * pulse);
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let dx = x as f32 - c.x;
                            let dy = y as f32 - c.y;
                            if ((dx * dx + dy * dy).sqrt() - STOMP_CRUSH_RADIUS).abs() < 0.6 {
                                blend_px(pb, x, y, col, 0.50 + 0.35 * charge);
                            }
                        }
                    }
                }
            } else {
                let imp_t = ((p - 0.66) / 0.34).clamp(0.0, 1.0);
                draw_shockwave_and_dust(pb, c, STOMP_SHOCKWAVE_RADIUS, imp_t, accent, true);
            }
        }
        BroomBossState::BroomSweep { clockwise } => {
            let c = w2s(pose.pos, cam);
            let face_ang = pose.face.angle();
            if p < 0.46 {
                let charge = (p / 0.46).clamp(0.0, 1.0);
                draw_sweep_telegraph(pb, c, face_ang, clockwise, charge, pose.anger, pulse);
            } else if p < 0.86 {
                let sweep_t = ((p - 0.46) / 0.26).clamp(0.0, 1.0);
                let fade = if p > 0.72 {
                    1.0 - (p - 0.72) / 0.14
                } else {
                    1.0
                };
                draw_sweep_trail(pb, c, face_ang, clockwise, sweep_t, fade);
            }
        }
        BroomBossState::BroomJab => {
            let bt = if p < 0.32 {
                p / 0.32
            } else if p < 0.64 {
                (p - 0.32) / 0.32
            } else {
                (p - 0.64) / 0.36
            };
            let c = w2s(pose.broom.ground, cam);
            if bt < 0.54 {
                let charge = (bt / 0.54).clamp(0.0, 1.0);
                draw_stomp_telegraph(pb, c, JAB_HEAD_RADIUS, charge, pose.anger, pulse);
            } else if bt < 0.92 {
                let imp_t = ((bt - 0.54) / 0.38).clamp(0.0, 1.0);
                draw_shockwave_and_dust(pb, c, JAB_HEAD_RADIUS + 3.0, imp_t, STRAW_HI, true);
            }
        }
        BroomBossState::DustpanDrop => {
            let c = w2s(pose.dustpan.ground, cam);
            if p < 0.48 {
                let charge = (p / 0.48).clamp(0.0, 1.0);
                draw_dustpan_telegraph(pb, c, DUSTPAN_IMPACT_RADIUS, charge, pose.anger, pulse);
            } else if p < 0.84 {
                let imp_t = ((p - 0.48) / 0.36).clamp(0.0, 1.0);
                draw_shockwave_and_dust(pb, c, DUSTPAN_IMPACT_RADIUS + 2.0, imp_t, accent, true);
            }
        }
        BroomBossState::Hurt { left_foot_hit } => {
            // Subtle comic hop dust puff when the hopping foot touches down!
            let hop_f = if left_foot_hit { pose.right } else { pose.left };
            if hop_f.lift < 1.8 {
                let c = w2s(hop_f.ground, cam);
                let puff_t = (hop_f.lift / 1.8).clamp(0.0, 1.0);
                draw_shockwave_and_dust(pb, c, 9.5, 0.35 + 0.5 * puff_t, DUST_COL, false);
            }
        }
        BroomBossState::PhaseChange => {
            let c = w2s(pose.pos, cam);
            if p < 0.38 {
                let charge = (p / 0.38).clamp(0.0, 1.0);
                draw_stomp_telegraph(pb, c, PHASE_SLAM_RADIUS, charge, 1.0, pulse);
            } else if p < 0.78 {
                let imp_t = ((p - 0.38) / 0.40).clamp(0.0, 1.0);
                draw_shockwave_and_dust(pb, c, PHASE_SLAM_RADIUS, imp_t, ANGER_COL, true);
            }
        }
        BroomBossState::Defeat => {
            // Little dust puff when the dropped broom clatters onto the floor!
            if p > 0.12 && p < 0.36 {
                let t = (p - 0.12) / 0.24;
                draw_shockwave_and_dust(
                    pb,
                    w2s(pose.broom.ground, cam),
                    11.0,
                    t,
                    DUST_COL,
                    false,
                );
            }
        }
        BroomBossState::Idle => {}
    }

    // 3) Soft floor contact shadows for both feet, the broom head/dropped handle, and the dustpan.
    draw_foot_shadow(pb, &pose.left, -1.0, cam, pose.body_alpha);
    draw_foot_shadow(pb, &pose.right, 1.0, cam, pose.body_alpha);
    draw_broom_shadow(pb, &pose.broom, cam, pose.broom_alpha);
    if pose.dustpan.visible {
        draw_dustpan_shadow(pb, &pose.dustpan, cam);
    }
}

fn draw_ground_hp_arc(pb: &mut PixelBuffer, c: Vec2, hp_frac: f32, anger: f32, pulse: f32) {
    let r = 19.5f32;
    let steps = 116;
    let hp_col = if anger > 0.5 {
        mix((224, 48, 42), (255, 136, 60), 0.35 + 0.65 * pulse)
    } else if hp_frac > 0.5 {
        mix((238, 178, 68), (116, 206, 168), (hp_frac - 0.5) * 2.0)
    } else {
        mix((236, 74, 58), (238, 178, 68), hp_frac * 2.0)
    };
    let mut last = (i32::MIN, i32::MIN);
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let a = -FRAC_PI_2 + t * TAU;
        let x = (c.x + a.cos() * r).round() as i32;
        let y = (c.y + a.sin() * r).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        let is_phase_tick = (t - 0.50).abs() < 0.015;
        let is_q_tick = (t - 0.25).abs() < 0.01 || (t - 0.75).abs() < 0.01;
        if is_phase_tick {
            blend_px(pb, x, y, (255, 224, 128), 0.76);
        } else if is_q_tick {
            darken_px(pb, x, y, 0.5);
        } else if t <= hp_frac {
            blend_px(pb, x, y, hp_col, 0.54);
        } else {
            darken_px(pb, x, y, 0.24);
        }
    }
}

fn draw_foot_shadow(
    pb: &mut PixelBuffer,
    foot: &FootPose,
    side: f32,
    cam: Vec2,
    alpha: f32,
) {
    if alpha <= 0.02 {
        return;
    }
    let lift_norm = (foot.lift / 18.0).clamp(0.0, 1.0);
    // When stomping high, keep the shadow centered under the landing target and
    // scale it dynamically as the giant foot approaches the floor!
    let sc = w2s(foot.ground, cam) + Vec2::new(1.8 + lift_norm * 1.4, 2.2 + lift_norm * 1.0);
    let sh_scale = (1.0 - 0.34 * lift_norm) * foot.scale;
    let strength = (0.60 - 0.16 * lift_norm) * alpha;
    let f = foot.face.rotate(foot.skew);
    let p = f.perp();

    let Some((x0, x1, y0, y1)) = bbox(pb, sc, 13.0, 13.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - sc.x;
            let dy = y as f32 - sc.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_slipper(u, v, side, sh_scale);
            if d < 1.6 {
                let edge = (1.0 - ((d + 0.4) / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                darken_px(pb, x, y, strength * edge);
            }
        }
    }
}

fn draw_broom_shadow(pb: &mut PixelBuffer, broom: &BroomPose, cam: Vec2, alpha: f32) {
    if alpha <= 0.02 {
        return;
    }
    let lift_norm = (broom.lift / 14.0).clamp(0.0, 1.0);
    let sc = w2s(broom.ground, cam) + Vec2::new(2.0 + lift_norm * 1.5, 2.2 + lift_norm * 1.2);
    let f = norm_face(broom.dir);
    let p = f.perp();
    let strength = (0.48 - 0.2 * lift_norm) * alpha;

    if let Some((x0, x1, y0, y1)) = bbox(pb, sc, 12.5, 12.5) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - sc.x;
                let dy = y as f32 - sc.y;
                let u = dx * f.x + dy * f.y;
                let v = dx * p.x + dy * p.y;
                let d = sd_ellipse(u, v, 0.5, 0.0, 4.8, 8.8 * broom.splay.min(1.25));
                if d < 1.4 {
                    let edge = (1.0 - ((d + 0.3) / 1.7).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    darken_px(pb, x, y, strength * edge);
                }
            }
        }
    }

    // When dropped flat in Defeat, cast a crisp shadow under the horizontal wooden handle too.
    if broom.dropped_flat {
        let a = sc - f * 3.8;
        let b = a - f * 24.0;
        let steps = 26;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let q = lerp_vec(a, b, t);
            let qx = q.x.round() as i32;
            let qy = q.y.round() as i32;
            darken_px(pb, qx, qy, 0.36 * alpha);
            darken_px(pb, qx, qy + 1, 0.26 * alpha);
        }
    }
}

fn draw_dustpan_shadow(pb: &mut PixelBuffer, pan: &DustpanPose, cam: Vec2) {
    if pan.alpha <= 0.02 {
        return;
    }
    let lift_norm = (pan.lift / 44.0).clamp(0.0, 1.0);
    let sc = w2s(pan.ground, cam) + Vec2::new(2.0 + lift_norm * 2.5, 2.4 + lift_norm * 2.0);
    let rx = 9.0 * (1.0 - 0.42 * lift_norm);
    let ry = 7.2 * (1.0 - 0.42 * lift_norm);
    let strength = (0.58 - 0.28 * lift_norm) * pan.alpha;

    let Some((x0, x1, y0, y1)) = bbox(pb, sc, rx + 2.0, ry + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let nx = (x as f32 - sc.x) / rx.max(1.0);
            let ny = (y as f32 - sc.y) / ry.max(1.0);
            let r2 = nx * nx + ny * ny;
            if r2 <= 1.0 {
                darken_px(pb, x, y, strength * (1.0 - r2 * 0.55));
            }
        }
    }
}

/// Crisp circular landing telegraph: outer dashed perimeter + subtle warm fill +
/// sharp 1px expanding inner timing ring.
fn draw_stomp_telegraph(
    pb: &mut PixelBuffer,
    c: Vec2,
    radius: f32,
    charge: f32,
    anger: f32,
    pulse: f32,
) {
    let Some((x0, x1, y0, y1)) = bbox(pb, c, radius + 2.0, radius + 2.0) else {
        return;
    };
    let warn_col = mix(
        WARN_AMBER,
        WARN_RED,
        (charge * 0.72 + anger * 0.28).clamp(0.0, 1.0),
    );
    let fill_r = radius * (0.24 + 0.76 * charge);
    let imminent = charge > 0.84;
    let flash = if imminent { 0.35 + 0.45 * pulse } else { 0.0 };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > radius + 0.9 {
                continue;
            }
            let ang = dy.atan2(dx);
            let on_outer = (dist - radius).abs() < 0.65;
            let dash = (ang * 6.0).sin() > -0.25 || charge > 0.78;
            if on_outer && dash {
                let col = mix(warn_col, WARN_HOT, flash);
                blend_px(pb, x, y, col, 0.76 + 0.18 * charge);
            } else if !imminent && (dist - fill_r).abs() < 0.62 {
                blend_px(pb, x, y, mix(warn_col, WARN_HOT, 0.48), 0.72);
            } else if dist < fill_r {
                let radial = (dist / radius).clamp(0.0, 1.0);
                let a = (0.06 + 0.14 * charge) * (0.45 + 0.55 * radial);
                blend_px(pb, x, y, warn_col, a);
            }
        }
    }
}

/// Dustpan drop zone: circular telegraph + 6 radial lane ticks previewing the shard burst!
fn draw_dustpan_telegraph(
    pb: &mut PixelBuffer,
    c: Vec2,
    radius: f32,
    charge: f32,
    anger: f32,
    pulse: f32,
) {
    draw_stomp_telegraph(pb, c, radius, charge, anger, pulse);
    let tick_col = mix(WARN_AMBER, WARN_HOT, 0.35 + 0.45 * charge);
    for k in 0..DUSTPAN_SHARD_COUNT {
        let a = k as f32 / DUSTPAN_SHARD_COUNT as f32 * TAU + 0.25;
        let dir = Vec2::from_angle(a);
        for step in 0..4 {
            let r = radius + 1.5 + step as f32 * 1.1;
            let p = c + dir * r;
            let fade = (1.0 - step as f32 * 0.22) * (0.45 + 0.5 * charge);
            blend_px(pb, p.x.round() as i32, p.y.round() as i32, tick_col, fade);
        }
    }
}

/// Annular sweep-arc sector telegraph (`SWEEP_INNER_RADIUS..SWEEP_OUTER_RADIUS`).
fn draw_sweep_telegraph(
    pb: &mut PixelBuffer,
    c: Vec2,
    face_ang: f32,
    clockwise: bool,
    charge: f32,
    anger: f32,
    pulse: f32,
) {
    let r_in = SWEEP_INNER_RADIUS;
    let r_out = SWEEP_OUTER_RADIUS;
    let Some((x0, x1, y0, y1)) = bbox(pb, c, r_out + 2.0, r_out + 2.0) else {
        return;
    };
    let warn_col = mix(
        WARN_AMBER,
        WARN_RED,
        (charge * 0.65 + anger * 0.30).clamp(0.0, 1.0),
    );
    let sign = if clockwise { 1.0 } else { -1.0 };
    let front_ang = -sign * SWEEP_HALF_ARC + sign * (2.0 * SWEEP_HALF_ARC) * charge;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let r = (dx * dx + dy * dy).sqrt();
            if r < r_in - 0.7 || r > r_out + 0.7 {
                continue;
            }
            let rel = angle_diff(dy.atan2(dx), face_ang);
            if rel.abs() > SWEEP_HALF_ARC + 0.04 {
                continue;
            }
            let on_r_edge = (r - r_out).abs() < 0.72 || (r - r_in).abs() < 0.65;
            let on_a_edge = (rel.abs() - SWEEP_HALF_ARC).abs() * r < 0.85;
            let on_front = (rel - front_ang).abs() * r < 1.05;

            if on_front {
                blend_px(pb, x, y, WARN_HOT, 0.82);
            } else if on_r_edge || on_a_edge {
                let col = mix(warn_col, WARN_HOT, if charge > 0.75 { 0.4 * pulse } else { 0.0 });
                blend_px(pb, x, y, col, 0.72 + 0.18 * charge);
            } else {
                let stripe = ((r * 0.45) as i32) % 2 == 0;
                let a = (if stripe { 0.14 } else { 0.08 }) * (0.45 + 0.55 * charge);
                blend_px(pb, x, y, warn_col, a);
            }
        }
    }
}

/// Active golden-dust motion arc + crisp wind streaks smeared behind the raking broom.
fn draw_sweep_trail(
    pb: &mut PixelBuffer,
    c: Vec2,
    face_ang: f32,
    clockwise: bool,
    sweep_t: f32,
    fade: f32,
) {
    if fade <= 0.02 {
        return;
    }
    let sign = if clockwise { 1.0 } else { -1.0 };
    let start_ang = -sign * SWEEP_HALF_ARC;
    let cur_ang = start_ang + sign * (2.0 * SWEEP_HALF_ARC) * sweep_t;
    let r_in = SWEEP_INNER_RADIUS + 2.0;
    let r_out = SWEEP_OUTER_RADIUS - 0.5;

    let Some((x0, x1, y0, y1)) = bbox(pb, c, r_out + 2.0, r_out + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let r = (dx * dx + dy * dy).sqrt();
            if r < r_in || r > r_out {
                continue;
            }
            let rel = angle_diff(dy.atan2(dx), face_ang);
            let behind = (cur_ang - rel) * sign;
            if !(0.0..=1.45).contains(&behind) {
                continue;
            }
            let trail = (1.0 - behind / 1.45).powi(2) * fade;
            // Crisp concentric whoosh speed-lines at 3 bristle radii + soft dust wash.
            let streak = (r - 19.5).abs() < 0.75
                || (r - 26.5).abs() < 0.85
                || (r - 33.5).abs() < 0.75;
            let bristle_band = (r * 1.15).sin() * 0.5 + 0.5;
            if streak {
                let col = mix(STRAW_HI, WARN_HOT, 0.55);
                blend_px(pb, x, y, col, trail * 0.82);
            } else {
                let col = mix(DUST_COL, STRAW_HI, bristle_band * 0.65);
                blend_px(pb, x, y, col, trail * (0.22 + 0.34 * bristle_band));
            }
        }
    }
}

/// Expanding shockwave ring + heavy floor dust puffs (and optional floor shock cracks).
fn draw_shockwave_and_dust(
    pb: &mut PixelBuffer,
    c: Vec2,
    max_r: f32,
    t: f32,
    accent: Rgb,
    heavy: bool,
) {
    let fade = (1.0 - t).clamp(0.0, 1.0);
    if fade <= 0.02 {
        return;
    }
    let ring_r = max_r * (0.28 + 0.82 * t.sqrt());
    let ring_col = mix(DUST_COL, mix(WARN_HOT, accent, 0.35), fade);

    // Outer primary shockwave ring.
    let steps = ((ring_r * 6.5) as i32).max(16);
    let mut last = (i32::MIN, i32::MIN);
    for i in 0..steps {
        let a = i as f32 / steps as f32 * TAU;
        let x = (c.x + a.cos() * ring_r).round() as i32;
        let y = (c.y + a.sin() * ring_r).round() as i32;
        if (x, y) != last {
            blend_px(pb, x, y, ring_col, fade * 0.90);
            last = (x, y);
        }
    }

    // Secondary inner shock ring + brief radial impact streaks for heavy slams!
    if heavy {
        let inner_r = ring_r * 0.68;
        let isteps = ((inner_r * 6.0) as i32).max(12);
        let mut ilast = (i32::MIN, i32::MIN);
        for i in 0..isteps {
            let a = i as f32 / isteps as f32 * TAU;
            let x = (c.x + a.cos() * inner_r).round() as i32;
            let y = (c.y + a.sin() * inner_r).round() as i32;
            if (x, y) != ilast {
                blend_px(pb, x, y, WARN_HOT, fade * 0.58);
                ilast = (x, y);
            }
        }
        if t < 0.55 {
            let streak_a = (1.0 - t / 0.55) * 0.65;
            for k in 0..6 {
                let a = k as f32 / 6.0 * TAU + 0.18;
                let dir = Vec2::from_angle(a);
                for s in 0..4 {
                    let q = c + dir * (max_r * 0.32 + s as f32 * 1.4);
                    darken_px(pb, q.x.round() as i32, q.y.round() as i32, streak_a);
                }
            }
        }
    }

    // Outward-rolling floor dust puffs!
    let puffs = if heavy { 10 } else { 7 };
    for k in 0..puffs {
        let a = k as f32 / puffs as f32 * TAU + 0.3;
        let pr = max_r * (0.32 + 0.72 * t);
        let px = c.x + a.cos() * pr;
        let py = c.y + a.sin() * pr;
        let puff_r = (if heavy { 3.1 } else { 2.4 } - 1.1 * t).max(0.9);
        let pr_i = puff_r.ceil() as i32;
        for dy in -pr_i..=pr_i {
            for dx in -pr_i..=pr_i {
                let d2 = (dx * dx + dy * dy) as f32;
                if d2 <= puff_r * puff_r {
                    let soft = 1.0 - (d2 / (puff_r * puff_r)).sqrt();
                    blend_px(
                        pb,
                        px.round() as i32 + dx,
                        py.round() as i32 + dy,
                        DUST_COL,
                        fade * soft * if heavy { 0.76 } else { 0.60 },
                    );
                }
            }
        }
    }
}

// ============================================================================
// Public Entry Point 2: Entity Layer (Feet, Socks, Trousers, Broom, Dustpan)
// ============================================================================

/// Draw **The Sweeper** (lower half of a giant human + corn-straw broom +
/// optional falling/bouncing tin dustpan).
///
/// **Layer**: Entity — call during the entity pass after [`draw_ground`] (and
/// after props). The rising trouser legs and broom handle fade smoothly upward
/// toward `screen -y` so the boss looms above the camera frustum without hard
/// clipping edges.
pub fn draw(pb: &mut PixelBuffer, view: &BroomBossView, cam: Vec2, accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let pose = compute_pose(view);
    let hit = view.hit_flash.clamp(0.0, 1.0);

    // Draw back-to-front by screen Y so legs, broom, and dustpan occlude cleanly.
    let ly = pose.left.ground.y;
    let ry = pose.right.ground.y;
    let by = pose.broom.ground.y;

    let broom_behind = by < ly.min(ry) - 2.0;
    if broom_behind {
        draw_broom(
            pb,
            &pose.broom,
            cam,
            hit * 0.25,
            pose.broom_alpha,
            pose.anger,
            view.anim,
        );
    }

    if ly <= ry {
        draw_leg_assembly(
            pb,
            &pose.left,
            -1.0,
            false,
            cam,
            hit,
            pose.body_alpha,
            pose.anger,
            view.anim,
        );
        draw_leg_assembly(
            pb,
            &pose.right,
            1.0,
            true,
            cam,
            hit,
            pose.body_alpha,
            pose.anger,
            view.anim,
        );
    } else {
        draw_leg_assembly(
            pb,
            &pose.right,
            1.0,
            true,
            cam,
            hit,
            pose.body_alpha,
            pose.anger,
            view.anim,
        );
        draw_leg_assembly(
            pb,
            &pose.left,
            -1.0,
            false,
            cam,
            hit,
            pose.body_alpha,
            pose.anger,
            view.anim,
        );
    }

    if !broom_behind {
        draw_broom(
            pb,
            &pose.broom,
            cam,
            hit * 0.25,
            pose.broom_alpha,
            pose.anger,
            view.anim,
        );
    }

    if pose.dustpan.visible {
        draw_dustpan(pb, &pose.dustpan, cam, accent);
    }
}

// ============================================================================
// Leg Assembly: Slipper -> Ribbed Sock (+ Hole!) -> Cuffed Fading Trouser Leg
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_leg_assembly(
    pb: &mut PixelBuffer,
    foot: &FootPose,
    side: f32,
    has_sock_hole: bool,
    cam: Vec2,
    hit: f32,
    alpha: f32,
    anger: f32,
    anim: f32,
) {
    if alpha <= 0.02 {
        return;
    }
    let foot_sc = w2s(foot.ground, cam) + Vec2::new(0.0, -foot.lift);

    // 1) Chunky scuffed oxblood leather slipper.
    draw_slipper(pb, foot_sc, foot, side, hit, alpha, anger);

    // 2) Rising ankle (ribbed cream sock + athletic stripe + frayed skin hole)
    //    and turned-up denim trouser cuff + fading upper trouser leg.
    //    Anchored over the heel opening; when facing away (north, `-y`), shift
    //    the ankle slightly upward so the rounded leather heel cup & rubber
    //    heel sole stay unmistakably visible beneath the sock!
    let foot_dir = foot.face.rotate(foot.skew);
    let north_heel_reveal = (-foot_dir.y).max(0.0) * 2.8 * foot.scale;
    let ankle_origin = foot_sc - foot_dir * (3.2 * foot.scale) + Vec2::new(0.0, -north_heel_reveal);
    draw_sock_and_trouser(
        pb,
        ankle_origin,
        foot.scale,
        has_sock_hole,
        side,
        hit,
        alpha,
        anger,
        anim,
    );

    // 3) Comedic toe-stub pain stars + radial throb lines during `Hurt`!
    if foot.hurt_shake {
        let toe = foot_sc + foot_dir * (10.2 * foot.scale);
        draw_hurt_stars(pb, toe, anim);
    }
}

fn draw_slipper(
    pb: &mut PixelBuffer,
    sc: Vec2,
    foot: &FootPose,
    side: f32,
    hit: f32,
    alpha: f32,
    anger: f32,
) {
    let sc_r = foot.scale;
    let f = foot.face.rotate(foot.skew);
    let p = f.perp();
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);
    let rim_col = if hit > 0.45 {
        (135, 74, 52)
    } else if anger > 0.5 {
        (46, 10, 10)
    } else {
        (0, 0, 0)
    };

    let Some((x0, x1, y0, y1)) = bbox(pb, sc, 13.5 * sc_r, 13.5 * sc_r) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - sc.x;
            let dy = y as f32 - sc.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_slipper(u, v, side, sc_r);
            let cover = (0.52 - d * 1.25).clamp(0.0, 1.0);

            if cover <= 0.0 {
                if d < 1.05 * sc_r {
                    blend_px(pb, x, y, rim_col, 0.74 * alpha);
                }
                continue;
            }

            let vo = v * side;
            let nu = ((u - 1.2 * sc_r) / (9.2 * sc_r)).clamp(-1.0, 1.0);
            let nv = (v / (4.5 * sc_r)).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.42 + nv * nv * 0.72)).clamp(0.0, 1.0);
            let lit = nu * fl + nv * pl;
            let lum = 0.36 + 0.38 * dome + 0.60 * lit;

            // Plush cream fleece collar wrapping around the heel ankle opening (`u ≈ -3.2`).
            let d_collar = sd_ellipse(u, vo, -3.2 * sc_r, 0.05 * sc_r, 2.75 * sc_r, 2.65 * sc_r);

            let mut col = if d_collar < 1.1 * sc_r {
                // Soft warm fleece collar — no harsh black crescent under the sock!
                let fleece_lum = if d_collar < -0.2 * sc_r {
                    0.28 + 0.25 * lit
                } else {
                    0.48 + 0.52 * lit
                };
                shade3(SHOE_FLEECE_SH, SHOE_FLEECE_MID, SHOE_FLEECE_HI, fleece_lum)
            } else if d > -0.95 * sc_r {
                // Dark rubber sole welt framing the perimeter.
                let welt = if lit < 0.15 { 0.84 } else { 0.46 };
                mix(shade3(SHOE_SH, SHOE_MID, SHOE_HI, lum), SHOE_SOLE, welt)
            } else {
                // Main oxblood leather upper.
                let mut c = shade3(SHOE_SH, SHOE_MID, SHOE_HI, lum);

                // Raised stitched moccasin vamp U-seam around the forefoot.
                let vamp_d = sd_ellipse(u, vo, 4.2 * sc_r, -0.2 * sc_r, 4.1 * sc_r, 2.8 * sc_r);
                if u > 0.6 * sc_r && vamp_d.abs() < 0.46 * sc_r {
                    let stitch = (u * 1.35 + v * 1.35).sin() > 0.22;
                    c = if stitch {
                        mix(c, SHOE_SCUFF, 0.62)
                    } else {
                        mix(c, SHOE_SOLE, 0.68)
                    };
                } else if u > 1.0 * sc_r && vamp_d < -0.45 * sc_r && vamp_d > -1.3 * sc_r && lit > 0.1 {
                    // Subtle top-left specular ridge just inside the moccasin apron seam!
                    c = mix(c, SHOE_HI, 0.32);
                }

                // Worn pale leather scuff marks across the front toe-cap & outer flank!
                let scuff_toe = u > 6.8 * sc_r && (vo + 0.6 * sc_r).abs() < 2.5 * sc_r;
                let scuff_side = u > 2.2 * sc_r && u < 5.6 * sc_r && vo > 2.1 * sc_r;
                if scuff_toe {
                    let grit = (u * 2.1 - v * 1.7).sin() * 0.5 + 0.5;
                    c = mix(c, SHOE_SCUFF, 0.36 + 0.44 * grit);
                } else if scuff_side {
                    c = mix(c, SHOE_SCUFF, 0.44);
                }

                // Throbbing red toe-stub tint during `Hurt`!
                if foot.hurt_shake && u > 5.0 * sc_r {
                    let throb = ((u - 5.0 * sc_r) / (5.5 * sc_r)).clamp(0.0, 1.0);
                    c = mix(c, WARN_RED, throb * 0.65);
                }
                c
            };

            col = apply_hit(col, hit);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, rim_col, 0.74 * alpha);
                pb.plot(x, y, mix(rimmed, col, cover * alpha));
            } else {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_sock_and_trouser(
    pb: &mut PixelBuffer,
    ankle: Vec2,
    sc: f32,
    has_sock_hole: bool,
    side: f32,
    hit: f32,
    alpha: f32,
    anger: f32,
    anim: f32,
) {
    // Vertical height coordinate `h = ankle.y - y` rises screen-up (`-y`).
    //   h ∈ [ 0.0,  7.0]*sc : ribbed cream sock + double stripe + frayed outer hole
    //   h ∈ [ 6.8, 11.8]*sc : turned-up light-wash denim cuff with gold topstitch
    //   h ∈ [11.6, 29.5]*sc : rising indigo trouser leg, fading out from h=16..29.5
    let h_max = 29.5 * sc;
    let half_w_max = 8.4 * sc;

    let x0 = (ankle.x - half_w_max).floor() as i32;
    let x1 = (ankle.x + half_w_max).ceil() as i32;
    let y0 = (ankle.y - h_max - 1.0).floor() as i32;
    let y1 = (ankle.y + 1.5 * sc).ceil() as i32;

    let x0 = x0.max(0);
    let x1 = x1.min(pb.w as i32 - 1);
    let y0 = y0.max(0);
    let y1 = y1.min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let rim_col = if hit > 0.45 { (110, 72, 60) } else { (0, 0, 0) };

    for y in y0..=y1 {
        let h = (ankle.y - y as f32) / sc;
        if !(-0.8..=29.5).contains(&h) {
            continue;
        }

        // Smooth fade-out as the giant leg rises above the camera plane!
        let top_fade = if h > 16.0 {
            1.0 - smoothstep((h - 16.0) / 13.5)
        } else {
            1.0
        };
        let row_alpha = alpha * top_fade;
        if row_alpha <= 0.015 {
            continue;
        }

        let lean_x = -side * (h * 0.055 * sc);
        let (hw, part) = if h < 6.8 {
            let tuck = if h < 0.7 {
                (1.0 - ((0.7 - h) / 1.8).powi(2)).max(0.0).sqrt()
            } else {
                1.0
            };
            ((3.15 + 0.14 * (h / 6.8)) * tuck, 0u8)
        } else if h < 11.7 {
            let ct = (h - 6.8) / 4.9;
            (5.25 + 0.35 * (ct * PI).sin(), 1u8)
        } else {
            let lt = ((h - 11.7) / 17.8).clamp(0.0, 1.0);
            (5.20 + 1.60 * lt, 2u8)
        };

        for x in x0..=x1 {
            let dx = (x as f32 - (ankle.x + lean_x)) / sc;
            let adx = dx.abs();
            let d = adx - hw;

            // Curved bottom lip of cuff overhangs the sock cleanly.
            if part == 1 && h < 7.6 {
                let bow = (1.0 - (dx / 5.5).powi(2)).clamp(0.0, 1.0) * 0.85;
                if h + bow < 7.1 {
                    continue;
                }
            }

            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 0.95 && h > 0.2 {
                    blend_px(pb, x, y, rim_col, 0.70 * row_alpha);
                }
                continue;
            }

            let nx = (dx / hw.max(0.5)).clamp(-1.0, 1.0);
            let nz = (1.0 - nx * nx).max(0.0).sqrt();
            let cyl_lit = (nx * LIGHT.x + nz * 0.42).clamp(-0.6, 1.0);

            let mut col = match part {
                0 => {
                    // Ribbed cream gym sock!
                    let rib = (dx * 2.8).cos() * 0.09;
                    let cuff_shadow = if h > 5.4 {
                        ((h - 5.4) / 1.4).clamp(0.0, 1.0) * 0.44
                    } else {
                        0.0
                    };
                    let collar_crease = if h < 0.5 { 0.22 } else { 0.0 };
                    let lum = (0.50 + 0.48 * cyl_lit + rib) * (1.0 - cuff_shadow - collar_crease);
                    let mut c = shade3(SOCK_SH, SOCK_MID, SOCK_HI, lum);

                    // Classic double crimson athletic stripe near the cuff top.
                    if (h - 5.1).abs() < 0.48 || (h - 3.9).abs() < 0.34 {
                        c = mix(
                            c,
                            scale(SOCK_STRIPE, 0.72 + 0.38 * (0.5 + 0.5 * cyl_lit)),
                            0.88,
                        );
                    }

                    // THE HOLE IN THE SOCK (right ankle, offset cleanly to the outer side
                    // so it reads unmistakably as a torn ribbed sock exposing peach skin!).
                    if has_sock_hole {
                        let hx = dx - 1.35;
                        let hy = h - 2.25;
                        let ragged = (hx * 3.0 + hy * 2.4).sin() * 0.14;
                        let hr = (hx * hx / (1.55 * 1.55) + hy * hy / (1.20 * 1.20)).sqrt() + ragged;
                        if hr < 0.80 {
                            c = mix(SKIN_SH, SKIN_HI, (0.45 + 0.55 * cyl_lit).clamp(0.0, 1.0));
                        } else if hr < 1.15 {
                            c = SOCK_HOLE_RIM;
                        }
                    }
                    c
                }
                1 => {
                    // Turned-up light-wash denim cuff!
                    let fold = (dx * 0.95).sin() * 0.08;
                    let lum = 0.46 + 0.54 * cyl_lit + fold;
                    let mut c = shade3(CUFF_SH, CUFF_MID, CUFF_HI, lum);
                    if (h - 7.85).abs() < 0.42 && (dx * 2.2).sin() > -0.15 {
                        c = mix(c, STITCH_GOLD, 0.74);
                    }
                    if h > 10.9 {
                        c = mix(c, PANT_SH, 0.65);
                    }
                    if anger > 0.5 && adx > hw - 1.2 {
                        let glow = 0.32 + 0.25 * (anim * 10.0).sin().abs();
                        c = mix(c, ANGER_COL, glow);
                    }
                    c
                }
                _ => {
                    // Rising indigo denim trouser leg with vertical fabric folds!
                    let crease1 = ((dx + 1.4) * 0.95).sin() * 0.14;
                    let crease2 = ((dx - 1.8) * 1.5).cos() * 0.08;
                    let lum = 0.40 + 0.56 * cyl_lit + crease1 + crease2;
                    let mut c = shade3(PANT_SH, PANT_MID, PANT_HI, lum);
                    if (dx * side - (hw * 0.65)).abs() < 0.42 {
                        c = mix(c, PANT_SH, 0.55);
                    }
                    if h > 16.5 {
                        c = scale(c, 1.0 - 0.28 * ((h - 16.5) / 13.0).clamp(0.0, 1.0));
                    }
                    c
                }
            };

            let hit_weight = if part == 0 {
                hit * 0.72
            } else if part == 1 {
                hit * 0.38
            } else {
                hit * 0.12
            };
            col = apply_hit(col, hit_weight);

            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, rim_col, 0.70 * row_alpha);
                pb.plot(x, y, mix(rimmed, col, cover * row_alpha));
            } else {
                blend_px(pb, x, y, col, row_alpha);
            }
        }
    }

    // Rising anger-steam wisps off the trouser cuff when Phase 2 enraged!
    if anger > 0.5 && alpha > 0.5 {
        for k in 0..3 {
            let phase = (anim * 2.4 + k as f32 * 0.33 + side * 0.2).fract();
            let sx = (ankle.x + side * (5.0 + (phase * 7.0 + k as f32).sin() * 1.5)).round() as i32;
            let sy = (ankle.y - (9.5 + phase * 13.5) * sc).round() as i32;
            let a = (1.0 - phase) * 0.88 * alpha;
            let col = if k == 0 { WARN_AMBER } else { ANGER_COL };
            blend_px(pb, sx, sy, col, a);
            blend_px(pb, sx, sy - 1, WARN_HOT, a * 0.45);
        }
    }
}

/// Comedic orbiting pain stars & radial "OUCH!" impact lines over the stubbed toe.
fn draw_hurt_stars(pb: &mut PixelBuffer, toe: Vec2, anim: f32) {
    // Crisp radial "zing!" pain rays around the stubbed toe.
    for k in 0..4 {
        let a = -0.9 + k as f32 * 0.62 + (anim * 12.0).sin() * 0.08;
        let dir = Vec2::from_angle(a);
        let p0 = toe + dir * 2.8;
        let p1 = toe + dir * 5.0;
        blend_px(pb, p0.x.round() as i32, p0.y.round() as i32, WARN_HOT, 0.90);
        blend_px(pb, p1.x.round() as i32, p1.y.round() as i32, WARN_RED, 0.78);
    }
    // Orbiting bright yellow/orange cartoon stars.
    for k in 0..3 {
        let a = anim * 8.0 + k as f32 * (TAU / 3.0);
        let sx = (toe.x + a.cos() * 5.6).round() as i32;
        let sy = (toe.y - 2.8 + a.sin() * 3.0).round() as i32;
        let col = if k % 2 == 0 {
            (255, 238, 92)
        } else {
            (255, 118, 72)
        };
        pb.plot(sx, sy, col);
        pb.plot_add(sx + 1, sy, scale(col, 0.5));
        pb.plot_add(sx - 1, sy, scale(col, 0.5));
        pb.plot_add(sx, sy - 1, scale(col, 0.5));
        pb.plot_add(sx, sy + 1, scale(col, 0.35));
    }
}

// ============================================================================
// Corn-Straw Broom: Flared Bristles + Barn-Red Wire-Bound Cap + Wooden Handle
// ============================================================================

fn draw_broom(
    pb: &mut PixelBuffer,
    broom: &BroomPose,
    cam: Vec2,
    hit: f32,
    alpha: f32,
    anger: f32,
    anim: f32,
) {
    if alpha <= 0.02 {
        return;
    }
    let head_sc = w2s(broom.ground, cam) + Vec2::new(0.0, -broom.lift);
    let f = norm_face(broom.dir);
    let p = f.perp();
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);

    let splay = broom.splay;
    let Some((x0, x1, y0, y1)) = bbox(pb, head_sc, 13.5 * splay, 13.5 * splay) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - head_sc.x;
            let dy = y as f32 - head_sc.y;
            let u = dx * f.x + dy * f.y;
            let v_raw = dx * p.x + dy * p.y;
            let tip_t = ((u + 4.2) / 8.8).clamp(0.0, 1.0);
            let v = v_raw - broom.bend * 2.2 * tip_t * tip_t;

            let hw = (4.3 + 4.4 * tip_t) * splay;
            let notch = if u > 2.2 {
                (v * 2.35).sin() * 0.55 + (v * 5.1).cos() * 0.35
            } else {
                0.0
            };
            let u_max = 4.6 * splay.sqrt() + notch;
            let u_min = -4.2;

            let du = (u_min - u).max(u - u_max);
            let dv = v.abs() - hw;
            let d = if du > 0.0 && dv > 0.0 {
                (du * du + dv * dv).sqrt()
            } else {
                du.max(dv)
            };

            let cover = (0.52 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.0 {
                    blend_px(pb, x, y, (0, 0, 0), 0.70 * alpha);
                }
                continue;
            }

            let nu = (u / 5.0).clamp(-1.0, 1.0);
            let nv = (v / hw.max(1.0)).clamp(-1.0, 1.0);
            let lit = nu * fl * 0.55 + nv * pl * 0.85;
            let dome = (1.0 - nv * nv * 0.55).clamp(0.0, 1.0);

            let mut col = if u < -1.3 {
                let lum = 0.42 + 0.35 * dome + 0.55 * lit;
                let mut c = shade3(CAP_SH, CAP_MID, CAP_HI, lum);
                if u < -3.2 {
                    c = mix(c, CAP_SH, 0.45);
                }
                c
            } else if (u - -1.0).abs() < 0.52 || (u - 0.55).abs() < 0.45 {
                let wire_lum = (0.55 + 0.5 * lit).clamp(0.25, 1.15);
                scale(WIRE_COL, wire_lum)
            } else {
                let strand = (v * 2.55 + u * 0.15).sin();
                let groove = if strand < -0.58 {
                    -0.36
                } else if strand > 0.52 {
                    0.22
                } else {
                    0.0
                };
                let tip_wear = if u > 3.2 { 0.14 } else { 0.0 };
                let lum = 0.46 + 0.28 * dome + 0.48 * lit + groove + tip_wear;
                let mut c = shade3(STRAW_SH, STRAW_MID, STRAW_HI, lum);
                if anger > 0.5 && u > 2.6 {
                    let flick = 0.28 + 0.24 * (anim * 14.0 + v).sin().abs();
                    c = mix(c, ANGER_COL, flick);
                }
                c
            };

            col = apply_hit(col, hit);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, (0, 0, 0), 0.70 * alpha);
                pb.plot(x, y, mix(rimmed, col, cover * alpha));
            } else {
                blend_px(pb, x, y, col, alpha);
            }
        }
    }

    // Wooden broomstick handle: rises smoothly toward the unseen hands (or lies flat on Defeat).
    let socket = head_sc - f * 3.8;
    let tip = if broom.dropped_flat {
        socket - f * 24.0
    } else {
        let hand_sc = w2s(broom.hand_target, cam);
        let to_hand = hand_sc - socket;
        let len = to_hand.len().clamp(24.0, 36.0);
        socket + norm_face(to_hand) * len
    };

    draw_wooden_handle(pb, socket, tip, !broom.dropped_flat, alpha);
}

fn draw_wooden_handle(pb: &mut PixelBuffer, a: Vec2, b: Vec2, fade_top: bool, alpha: f32) {
    let min_x = a.x.min(b.x).floor() as i32 - 3;
    let max_x = a.x.max(b.x).ceil() as i32 + 3;
    let min_y = a.y.min(b.y).floor() as i32 - 3;
    let max_y = a.y.max(b.y).ceil() as i32 + 3;

    let x0 = min_x.max(0);
    let x1 = max_x.min(pb.w as i32 - 1);
    let y0 = min_y.max(0);
    let y1 = max_y.min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let ab = b - a;
    let len_sq = ab.len_sq().max(1.0);
    let dir = norm_face(ab);
    let perp = dir.perp();
    let pl = perp.dot(LIGHT);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let ap = Vec2::new(x as f32 - a.x, y as f32 - a.y);
            let t = (ap.dot(ab) / len_sq).clamp(0.0, 1.0);
            let closest = a + ab * t;
            let off = Vec2::new(x as f32 - closest.x, y as f32 - closest.y);
            let dist = off.len();
            let rad = 1.25;
            let d = dist - rad;

            let top_fade = if fade_top && t > 0.45 {
                1.0 - smoothstep((t - 0.45) / 0.55)
            } else {
                1.0
            };
            let px_alpha = alpha * top_fade;
            if px_alpha <= 0.015 {
                continue;
            }

            let cover = (0.55 - d * 1.35).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 0.85 {
                    blend_px(pb, x, y, (0, 0, 0), 0.65 * px_alpha);
                }
                continue;
            }

            let nv = (off.dot(perp) / rad).clamp(-1.0, 1.0);
            let lit = nv * pl;
            let grain = (t * 14.0).sin() * 0.06;
            let col = shade3(WOOD_SH, WOOD_MID, WOOD_HI, 0.52 + 0.55 * lit + grain);

            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, (0, 0, 0), 0.65 * px_alpha);
                pb.plot(x, y, mix(rimmed, col, cover * px_alpha));
            } else {
                blend_px(pb, x, y, col, px_alpha);
            }
        }
    }
}

// ============================================================================
// Galvanized Tin Dustpan (`DustpanDrop`)
// ============================================================================

fn draw_dustpan(pb: &mut PixelBuffer, pan: &DustpanPose, cam: Vec2, accent: Rgb) {
    if pan.alpha <= 0.02 {
        return;
    }
    let sc = w2s(pan.ground, cam) + Vec2::new(0.0, -pan.lift);
    let f = Vec2::from_angle(pan.angle + pan.tilt * 0.4);
    let p = f.perp();
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);

    let Some((x0, x1, y0, y1)) = bbox(pb, sc, 15.0, 15.0) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - sc.x;
            let dy = y as f32 - sc.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;

            let scoop_t = ((u + 5.0) / 10.5).clamp(0.0, 1.0);
            let hw = 5.4 + 2.4 * scoop_t;
            let du = (-5.0 - u).max(u - 5.5);
            let dv = v.abs() - hw;
            let d_scoop = if du > 0.0 && dv > 0.0 {
                (du * du + dv * dv).sqrt()
            } else {
                du.max(dv)
            };
            let d_handle = sd_capsule(u, v, -5.0, 0.0, -10.8, 0.0, 1.45);
            let d = d_scoop.min(d_handle);

            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.0 {
                    blend_px(pb, x, y, (0, 0, 0), 0.72 * pan.alpha);
                }
                continue;
            }

            let lit =
                (u / 6.0).clamp(-1.0, 1.0) * fl * 0.45 + (v / 7.0).clamp(-1.0, 1.0) * pl * 0.65;
            let col = if d_handle < d_scoop {
                let grip_lit = (v / 1.5).clamp(-1.0, 1.0) * pl;
                shade3(CAP_SH, CAP_MID, CAP_HI, 0.5 + 0.55 * grip_lit)
            } else if u > 4.1 {
                PAN_RUBBER
            } else if v.abs() > hw - 1.3 || u < -3.6 {
                let wall_lum = 0.62 + 0.52 * lit;
                shade3(PAN_SH, PAN_MID, PAN_HI, wall_lum)
            } else {
                let brushed = (u * 1.6 + v * 0.6).sin() * 0.06;
                let bed = shade3(PAN_SH, PAN_MID, PAN_HI, 0.36 + 0.38 * lit + brushed);
                mix(bed, accent, 0.08)
            };

            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, (0, 0, 0), 0.72 * pan.alpha);
                pb.plot(x, y, mix(rimmed, col, cover * pan.alpha));
            } else {
                blend_px(pb, x, y, col, pan.alpha);
            }
        }
    }

    // Metallic CLANG specular glint star on the dustpan rim right around impact!
    if pan.lift < 4.5 && pan.alpha > 0.9 {
        let g = sc + f * -2.2 - p * 4.2;
        let gx = g.x.round() as i32;
        let gy = g.y.round() as i32;
        blend_px(pb, gx, gy, (255, 255, 255), 0.92);
        pb.plot_add(gx + 1, gy, (140, 155, 175));
        pb.plot_add(gx - 1, gy, (140, 155, 175));
        pb.plot_add(gx, gy + 1, (140, 155, 175));
        pb.plot_add(gx, gy - 1, (140, 155, 175));
    }
}

// ============================================================================
// Tests & Visual Verification Sheet
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor;
    use crate::game::art_rat;
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;

    const ALL_STATES: [BroomBossState; 11] = [
        BroomBossState::Idle,
        BroomBossState::StompWalk { left_foot: true },
        BroomBossState::StompWalk { left_foot: false },
        BroomBossState::BroomSweep { clockwise: true },
        BroomBossState::BroomSweep { clockwise: false },
        BroomBossState::BroomJab,
        BroomBossState::DustpanDrop,
        BroomBossState::Hurt { left_foot_hit: true },
        BroomBossState::Hurt { left_foot_hit: false },
        BroomBossState::PhaseChange,
        BroomBossState::Defeat,
    ];

    fn sample_view(pos: Vec2, ang: f32, state: BroomBossState, progress: f32) -> BroomBossView {
        let facing = Vec2::from_angle(ang);
        let target = match state {
            BroomBossState::StompWalk { left_foot } => {
                let side = if left_foot { -1.0 } else { 1.0 };
                pos + facing * 16.0 + facing.perp() * (side * STANCE_HALF_WIDTH)
            }
            BroomBossState::DustpanDrop => pos + facing * 34.0,
            _ => pos + facing * 28.0,
        };
        BroomBossView {
            pos,
            facing,
            state,
            progress,
            anim: progress * 2.5 + 0.4,
            hit_flash: 0.0,
            hp_frac: 0.75,
            target,
            enraged: false,
        }
    }

    #[test]
    fn broom_boss_draws_everywhere_without_panic() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (8, 8), (120, 90)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for &state in &ALL_STATES {
                for prog in [-0.2f32, 0.0, 0.25, 0.5, 0.75, 1.0, 1.4] {
                    for ang_i in 0..8 {
                        let ang = ang_i as f32 / 8.0 * TAU;
                        for &(x, y) in &[
                            (60.0, 45.0),
                            (0.0, 0.0),
                            (119.5, 89.5),
                            (-80.0, -80.0),
                            (300.0, 300.0),
                        ] {
                            let mut v = sample_view(Vec2::new(x, y), ang, state, prog);
                            if ang_i % 2 == 0 {
                                v.enraged = true;
                                v.hp_frac = 0.35;
                            }
                            if ang_i % 3 == 0 {
                                v.hit_flash = 0.85;
                            }
                            draw_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
                            draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn zero_facing_and_hitboxes_are_valid() {
        let mut pb = PixelBuffer::new(100, 100);
        let mut v = sample_view(Vec2::new(50.0, 50.0), 0.0, BroomBossState::Idle, 0.5);
        v.facing = Vec2::ZERO;
        draw_ground(&mut pb, &v, Vec2::ZERO, (255, 180, 90));
        draw(&mut pb, &v, Vec2::ZERO, (255, 180, 90));

        let feet = foot_hitboxes(&v);
        assert!(feet[0].grounded && feet[1].grounded);
        assert!((feet[0].pos.dist(feet[1].pos) - STANCE_HALF_WIDTH * 2.0).abs() < 2.5);

        let hurt_v = sample_view(
            Vec2::new(50.0, 50.0),
            FRAC_PI_2,
            BroomBossState::Hurt { left_foot_hit: true },
            0.5,
        );
        let hurt_feet = foot_hitboxes(&hurt_v);
        assert!(!hurt_feet[0].grounded);

        let sweep_v = sample_view(
            Vec2::new(50.0, 50.0),
            FRAC_PI_2,
            BroomBossState::BroomSweep { clockwise: true },
            0.59,
        );
        let br_dist = broom_head_pos(&sweep_v).dist(sweep_v.pos);
        assert!(
            (SWEEP_INNER_RADIUS..=SWEEP_OUTER_RADIUS).contains(&br_dist),
            "broom head dist {br_dist} outside sweep band"
        );
    }

    #[test]
    fn hit_flash_and_enraged_visibly_change_pixels() {
        let render = |flash: f32, enraged: bool| {
            let mut pb = PixelBuffer::new(90, 90);
            pb.clear((16, 22, 20));
            let mut v = sample_view(Vec2::new(45.0, 55.0), FRAC_PI_2, BroomBossState::Idle, 0.3);
            v.hit_flash = flash;
            v.enraged = enraged;
            v.hp_frac = if enraged { 0.35 } else { 0.85 };
            draw_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            pb
        };
        let a = render(0.0, false);
        let b = render(0.95, false);
        let c = render(0.0, true);
        let mut diff_flash = 0usize;
        let mut diff_enraged = 0usize;
        for y in 0..90 {
            for x in 0..90 {
                if a.pixel_at(x, y) != b.pixel_at(x, y) {
                    diff_flash += 1;
                }
                if a.pixel_at(x, y) != c.pixel_at(x, y) {
                    diff_enraged += 1;
                }
            }
        }
        assert!(diff_flash > 40, "hit flash changed only {diff_flash} px");
        assert!(diff_enraged > 25, "enraged changed only {diff_enraged} px");
    }

    /// Sheet dump: `RATRUN_DUMP=boss_broom cargo test dump_boss_broom_sheet`
    /// writes `/tmp/ratrun_boss_broom.ppm` showing every state across key
    /// progress frames, facings, enraged/hit states, and rat scale comparison
    /// stacked across all 3 map themes (Sewer, Kitchen, Lab).
    #[test]
    fn dump_boss_broom_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("boss_broom") {
            return;
        }
        const CELL_W: usize = 98;
        const CELL_H: usize = 94;
        const COLS: usize = 5;
        const ROWS: usize = 8;
        let (w, h) = (CELL_W * COLS, CELL_H * ROWS);
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();

        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let level = Level::new(theme);
            let accent = level.palette.accent;
            // Keep all 8 rows (752 px tall) comfortably inside the 1020x800 arena floor!
            let cam = Vec2::new(80.0, 24.0);
            let mut pb = PixelBuffer::new(w, h);
            art_floor::draw_floor(&mut pb, &level, cam, 0.0);

            for r in 1..ROWS {
                let yy = (r * CELL_H) as i32;
                for x in 0..w as i32 {
                    blend_px(&mut pb, x, yy, (110, 118, 128), 0.16);
                }
            }
            for c in 1..COLS {
                let xx = (c * CELL_W) as i32;
                for y in 0..h as i32 {
                    blend_px(&mut pb, xx, y, (110, 118, 128), 0.16);
                }
            }

            let cell_anchor = |col: usize, row: usize| -> Vec2 {
                cam + Vec2::new(
                    (col * CELL_W) as f32 + CELL_W as f32 * 0.5,
                    (row * CELL_H) as f32 + CELL_H as f32 * 0.56,
                )
            };

            // Row 0: Idle & Facings (South + Rat scale, East, North, SW, Enraged + Hit Flash)
            for col in 0..COLS {
                let pos = cell_anchor(col, 0);
                let ang = match col {
                    0 => FRAC_PI_2,
                    1 => 0.0,
                    2 => -FRAC_PI_2,
                    3 => FRAC_PI_2 + 0.7,
                    _ => FRAC_PI_2 - 0.35,
                };
                let mut v = sample_view(pos, ang, BroomBossState::Idle, 0.25 * col as f32);
                if col == 4 {
                    v.enraged = true;
                    v.hp_frac = 0.32;
                    v.hit_flash = 0.85;
                }
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
                if col == 0 {
                    let mut rat = Player::new(pos + Vec2::new(-25.0, 13.0), WeaponKind::Gnaw);
                    rat.face = Vec2::new(0.8, -0.5).normalized();
                    art_rat::draw_rat(&mut pb, &rat, 0.6, cam);
                }
            }

            // Row 1: StompWalk
            let stomp_ps = [0.15, 0.42, 0.61, 0.72, 0.90];
            for (col, &pr) in stomp_ps.iter().enumerate() {
                let pos = cell_anchor(col, 1) + Vec2::new(0.0, -4.0);
                let v = sample_view(
                    pos,
                    FRAC_PI_2 + 0.15,
                    BroomBossState::StompWalk { left_foot: true },
                    pr,
                );
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 2: BroomSweep
            let sweep_ps = [0.18, 0.42, 0.56, 0.68, 0.86];
            for (col, &pr) in sweep_ps.iter().enumerate() {
                let pos = cell_anchor(col, 2) + Vec2::new(0.0, -5.0);
                let v = sample_view(
                    pos,
                    FRAC_PI_2,
                    BroomBossState::BroomSweep { clockwise: true },
                    pr,
                );
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 3: BroomJab
            let jab_ps = [0.12, 0.19, 0.52, 0.74, 0.85];
            for (col, &pr) in jab_ps.iter().enumerate() {
                let pos = cell_anchor(col, 3) + Vec2::new(0.0, -6.0);
                let v = sample_view(pos, FRAC_PI_2, BroomBossState::BroomJab, pr);
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 4: DustpanDrop
            let pan_ps = [0.14, 0.36, 0.50, 0.64, 0.93];
            for (col, &pr) in pan_ps.iter().enumerate() {
                let pos = cell_anchor(col, 4) + Vec2::new(-12.0, -2.0);
                let mut v = sample_view(pos, 0.18, BroomBossState::DustpanDrop, pr);
                v.target = pos + Vec2::new(30.0, 5.0);
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 5: Hurt (hopping on one foot)
            let hurt_cases = [
                (true, 0.15),
                (true, 0.42),
                (true, 0.72),
                (false, 0.35),
                (false, 0.65),
            ];
            for (col, &(left_hit, pr)) in hurt_cases.iter().enumerate() {
                let pos = cell_anchor(col, 5);
                let mut v = sample_view(
                    pos,
                    FRAC_PI_2,
                    BroomBossState::Hurt {
                        left_foot_hit: left_hit,
                    },
                    pr,
                );
                v.hit_flash = if col == 0 || col == 3 { 0.65 } else { 0.0 };
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 6: PhaseChange
            let phase_ps = [0.12, 0.34, 0.42, 0.58, 0.92];
            for (col, &pr) in phase_ps.iter().enumerate() {
                let pos = cell_anchor(col, 6);
                let mut v = sample_view(pos, FRAC_PI_2, BroomBossState::PhaseChange, pr);
                v.hp_frac = 0.48;
                v.enraged = pr >= 0.42;
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            // Row 7: Defeat
            let def_ps = [0.12, 0.30, 0.52, 0.72, 0.90];
            for (col, &pr) in def_ps.iter().enumerate() {
                let pos = cell_anchor(col, 7) + Vec2::new(0.0, 8.0);
                let mut v = sample_view(pos, FRAC_PI_2, BroomBossState::Defeat, pr);
                v.hp_frac = 0.0;
                draw_ground(&mut pb, &v, cam, accent);
                draw(&mut pb, &v, cam, accent);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_boss_broom.ppm", out).unwrap();
    }
}
