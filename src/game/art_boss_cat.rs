//! Art and gameplay design specification for the **Jungle Cat Boss** (Apex Jaguar).
//!
//! Pure zero-allocation drawing from [`BossCatView`] into the half-block
//! [`PixelBuffer`]. Unlike the small slate-grey alley cat in `art_enemy.rs`
//! (radius 4 px, ~9 px body), this boss is an apex golden-ochre jaguar roughly
//! **51 px nose-to-rump plus a 34 px rosette-banded tail** (~5.5× the player
//! rat's body length).
//!
//! # Visual Identity & Phase Change
//! - **Phase 1 (`hp_frac > 0.50`)**: Sunlit golden-ochre coat with organic
//!   dark-espresso rosettes enclosing warm cinnamon cores and central spots,
//!   broad heavy rolling shoulders, rounded dark-backed ears with cream jaguar
//!   ocellus spots, and piercing emerald-jade almond eyes whose slit pupils
//!   track [`BossCatView::target`].
//! - **Phase 2 — Apex Frenzy (`hp_frac <= 0.50`)**: Coat scorches darker toward
//!   burnt copper-amber, neck and shoulder hackles bristle outward into jagged
//!   fur spikes, crimson battle scars mark the shoulder and haunch, claws stay
//!   permanently unsheathed, and eyes ignite to molten amber-crimson with subtle
//!   additive glow. Below `0.25` HP, critical embers pulse along the scars.
//!
//! # Render Layering Contract
//! Callers must invoke two functions in different passes of `World::draw`:
//! 1. [`draw_boss_cat_ground`] — **Ground / Telegraph Layer**: call *after*
//!    `art_floor::draw_floor` and stains, *before* props, enemies, and the
//!    player. Draws the jaguar's drop shadow (including the growing pounce
//!    shadow), pounce landing reticle & predator sightline, swipe warning fan,
//!    roar acoustic ring, and impact shockwaves.
//! 2. [`draw_boss_cat`] — **Entity Layer**: normally call alongside enemies.
//!    When [`is_airborne`] returns `true` (mid-`Pounce`), draw **above props
//!    and the player** so the leaping jaguar visibly passes overhead.
//!
//! # Hitboxes & Gameplay Integration
//! Use [`body_hit_circles`] (3 overlapping circles hugging haunches, chest, and
//! head) for fair projectile and contact collision against the ~51 px body, or
//! [`BODY_RADIUS`] if a single-circle fallback is needed. All attack radii,
//! angles, durations, and damage values are exported as `pub const`s below so
//! simulation logic and rendered telegraphs stay locked together.

#![allow(dead_code)]

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

// ============================================================================
// Gameplay Spec Constants (Collision, Timings, Hitboxes & Damage)
// ============================================================================

/// Primary single-circle collision radius fallback (world pixels).
/// Prefer [`body_hit_circles`] for accurate elongated-body hit detection.
pub const BODY_RADIUS: f32 = 13.5;

/// Approximate visual body length from rump to snout (world pixels).
pub const BODY_LENGTH: f32 = 51.0;

/// Approximate visual tail length behind the rump (world pixels).
pub const TAIL_LENGTH: f32 = 34.0;

/// Spine offsets (along `facing`) and radii for the 3-circle body hitbox.
pub const HAUNCH_OFFSET_U: f32 = -13.5;
pub const HAUNCH_RADIUS: f32 = 9.5;
pub const CHEST_OFFSET_U: f32 = 5.5;
pub const CHEST_RADIUS: f32 = 11.0;
pub const HEAD_OFFSET_U: f32 = 20.0;
pub const HEAD_RADIUS: f32 = 7.5;

/// Health fraction threshold at which the jaguar enters Phase 2 (Apex Frenzy).
/// Trigger a mandatory [`BossCatState::Roar`] when crossing this threshold.
pub const PHASE2_HP_FRAC: f32 = 0.50;

/// Health fraction threshold for critical visual frenzy (pulsing scars/embers).
pub const CRITICAL_HP_FRAC: f32 = 0.25;

/// Recommended total boss HP pool (scales with run `hp_mult`; compare old Boss 1400).
pub const BOSS_BASE_HP: f32 = 2200.0;

/// Melee body-contact damage (Skitterer = 6, Alley Cat = 12, Brute = 22, Player = 100 HP).
pub const CONTACT_DAMAGE: f32 = 18.0;

/// Stalk pursuit movement speeds (world px/s; player base speed is 58 px/s).
pub const STALK_SPEED_P1: f32 = 24.0;
pub const STALK_SPEED_P2: f32 = 32.0;

/// Pounce crouch telegraph duration in seconds.
pub const CROUCH_DURATION_P1: f32 = 0.95;
pub const CROUCH_DURATION_P2: f32 = 0.75;
/// Progress in `Crouch` when `target` locks permanently and the reticle flashes red-gold.
pub const CROUCH_LOCK_PROGRESS: f32 = 0.75;

/// Legal pounce distance band from `pos` to `target` (world pixels).
pub const POUNCE_MIN_DIST: f32 = 34.0;
pub const POUNCE_MAX_DIST: f32 = 96.0;
/// Total `Pounce` state duration (airborne flight + impact skid).
pub const POUNCE_DURATION_P1: f32 = 0.70;
pub const POUNCE_DURATION_P2: f32 = 0.58;
/// Progress in `Pounce` when the jaguar slams down onto `target`.
pub const POUNCE_LAND_PROGRESS: f32 = 0.80;
/// Visual peak height above ground during `Pounce` (screen `-y` pixels).
pub const POUNCE_PEAK_HEIGHT: f32 = 19.0;
/// Circular slam hitbox radius centered on `target` at landing.
pub const POUNCE_LAND_RADIUS: f32 = 22.0;
/// Direct slam damage inside `POUNCE_LAND_RADIUS`.
pub const POUNCE_DAMAGE: f32 = 28.0;
/// Phase 2 expanding landing shockwave outer radius and damage.
pub const POUNCE_SHOCKWAVE_RADIUS: f32 = 38.0;
pub const POUNCE_SHOCKWAVE_DAMAGE: f32 = 14.0;

/// Paw swipe windup telegraph duration.
pub const SWIPE_WINDUP_P1: f32 = 0.55;
pub const SWIPE_WINDUP_P2: f32 = 0.42;
/// Active swipe + follow-through duration.
pub const SWIPE_DURATION: f32 = 0.50;
/// Active damage window within `Swipe::progress`.
pub const SWIPE_ACTIVE_START: f32 = 0.05;
pub const SWIPE_ACTIVE_END: f32 = 0.45;
/// Forward offset along `facing` for the center of the claw-swipe sector.
pub const SWIPE_ORIGIN_U: f32 = 12.0;
/// Inner and outer radii of the claw-swipe damage sector.
pub const SWIPE_INNER_RADIUS: f32 = 6.0;
pub const SWIPE_RADIUS: f32 = 30.0;
/// Half-angle of the claw-swipe sector in radians (`65°` -> `130°` total arc).
pub const SWIPE_HALF_ANGLE: f32 = 1.134_464;
/// Damage dealt by a paw swipe (matches a Brute hit).
pub const SWIPE_DAMAGE: f32 = 22.0;

/// Roar state duration and active blast progress window.
pub const ROAR_DURATION: f32 = 1.30;
pub const ROAR_BLAST_START: f32 = 0.35;
pub const ROAR_BLAST_END: f32 = 0.90;
/// Forward offset along `facing` for the mouth / acoustic wave origin.
pub const ROAR_ORIGIN_U: f32 = 24.0;
/// Inner damaging blast radius and outer knockback/warning radius around the head.
pub const ROAR_INNER_RADIUS: f32 = 26.0;
pub const ROAR_OUTER_RADIUS: f32 = 44.0;
/// Damage dealt inside `ROAR_INNER_RADIUS` (or by Phase 2 radial sonic fangs).
pub const ROAR_DAMAGE: f32 = 12.0;

/// Stagger reaction duration (longer if stunned by a trap or wall slam).
pub const HURT_DURATION: f32 = 0.45;
/// Dramatic defeat collapse duration before despawning / dropping boss loot.
pub const DEATH_DURATION: f32 = 1.80;

/// Direction *toward* the light in screen space (up-left), matching `art_rat.rs`,
/// `art_enemy.rs`, and `art_props.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// ============================================================================
// Public View Data & Move-Set State Spec
// ============================================================================

/// Move set and behavioral state machine of the Jungle Cat Boss.
///
/// Every attack state has a clear telegraph before its active danger window so
/// the player (move speed `58 px/s`, dash `~30 px` over `0.16 s` with `0.18 s`
/// i-frames, `100` base HP) can read and dodge every move fairly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossCatState {
    /// **Predatory Prowl / Chase.**
    ///
    /// - **Behavior**: Stalks toward the player with rolling shoulder blades,
    ///   flexing spine, swaying rosette tail, and slit eyes fixed on `target`.
    ///   Moves at [`STALK_SPEED_P1`] (`24 px/s`) in Phase 1 and [`STALK_SPEED_P2`]
    ///   (`32 px/s`) in Phase 2, routing around obstacles via flow field.
    /// - **Duration**: `2.0–3.5 s` (Phase 1) or `1.4–2.4 s` (Phase 2), ending
    ///   early if the player stays within `40 px` (triggers `SwipeWindup`) or
    ///   enters `45..95 px` with clear line of sight (triggers `Crouch`).
    /// - **Warning**: None (neutral movement state).
    /// - **Dangerous area**: Body collision only — 3 circles from
    ///   [`body_hit_circles`] (haunch `r=9.5`, chest `r=11.0`, head `r=7.5`).
    /// - **Damage**: [`CONTACT_DAMAGE`] (`18`) with `0.5 s` contact cooldown.
    /// - **Counter**: Maintain medium range, duck through mouse-holes in walls
    ///   that the jaguar cannot enter, and deal sustained weapon DPS.
    Stalk,

    /// **Pounce Telegraph (Low Crouch & Wiggling Haunches).**
    ///
    /// - **Behavior**: Flattens low, pins ears flat back, plants front paws,
    ///   and wiggles hindquarters rapidly while coiling to spring. For
    ///   `progress` in `0.0..0.75`, `target` softly tracks the rat (clamped to
    ///   [`POUNCE_MIN_DIST`]..=[`POUNCE_MAX_DIST`] from `pos`). At
    ///   [`CROUCH_LOCK_PROGRESS`] (`0.75`), `target` locks permanently and
    ///   `pounce_origin` records `pos`.
    /// - **Duration**: [`CROUCH_DURATION_P1`] (`0.95 s`) in Phase 1;
    ///   [`CROUCH_DURATION_P2`] (`0.75 s`) in Phase 2. Transitions directly
    ///   into [`BossCatState::Pounce`].
    /// - **Warning**: Full duration (`0.75–0.95 s`). Player sees wiggling
    ///   haunches, twitching tail tip, dotted predator sightline, and a pulsed
    ///   ground landing ring of radius [`POUNCE_LAND_RADIUS`] (`22 px`) at
    ///   `target` that flashes bright crimson-gold once locked (`progress >= 0.75`).
    /// - **Dangerous area**: Body contact only (`18` dmg). The marked landing
    ///   zone is **safe** during `Crouch`.
    /// - **Damage**: [`CONTACT_DAMAGE`] (`18`) on body touch; `0` in landing zone.
    /// - **Counter**: Bait the target lock at `progress = 0.75`, then walk
    ///   perpendicular (`58 px/s` clears `22 px` in `0.38 s`) or dash out.
    Crouch,

    /// **Airborne Leap & Crushing Slam.**
    ///
    /// - **Behavior**: Launches from `pounce_origin` to `target`. Implementer
    ///   should linearly interpolate `view.pos` from `pounce_origin` to `target`
    ///   over `progress = 0.0..0.80` (`POUNCE_LAND_PROGRESS`), then hold `pos = target`
    ///   for the landing skid (`0.80..1.0`). Drawing elevates the sprite along
    ///   screen `-y` up to [`POUNCE_PEAK_HEIGHT`] (`19 px`) while the ground
    ///   shadow **grows larger and darker** beneath the descending cat.
    /// - **Duration**: [`POUNCE_DURATION_P1`] (`0.70 s`) in Phase 1;
    ///   [`POUNCE_DURATION_P2`] (`0.58 s`) in Phase 2.
    /// - **Warning**: Landing circle at `target` remains lit throughout flight
    ///   with a shrinking timing ring + growing dark shadow (`~0.55 s` flight
    ///   on top of the `Crouch` telegraph).
    /// - **Dangerous area**:
    ///   - Airborne (`progress < 0.80`): passes **over** props, traps, and the
    ///     player (disable contact damage while [`is_airborne`] is true).
    ///   - Slam (`progress = 0.80..0.95`): circle of radius
    ///     [`POUNCE_LAND_RADIUS`] (`22 px`) centered at `target`.
    ///   - Phase 2 bonus (`hp_frac <= 0.50`, `progress = 0.80..1.0`): expanding
    ///     shockwave ring from `22 px` to [`POUNCE_SHOCKWAVE_RADIUS`] (`38 px`),
    ///     ring thickness `5 px`.
    /// - **Damage**: [`POUNCE_DAMAGE`] (`28`) on direct slam;
    ///   [`POUNCE_SHOCKWAVE_DAMAGE`] (`14`) on Phase 2 shockwave ring.
    /// - **Counter**: Step outside the `22 px` ring before `progress = 0.80`, or
    ///   dash (`0.18 s` i-frames) through the slam/shockwave. Punish during the
    ///   `0.14 s` landing skid (or dodge a Phase 2 follow-up `Swipe`).
    Pounce,

    /// **Claw Swipe Telegraph (Raised Paw & Ground Arc).**
    ///
    /// - **Behavior**: Plants opposite forepaw, shifts weight back, coils
    ///   shoulders away from `swipe_side` (`+1.0` = right paw, `-1.0` = left paw),
    ///   and hoists the striking paw wide with four gleaming ivory claws splayed.
    ///   Facing locks at `progress = 0.55`.
    /// - **Duration**: [`SWIPE_WINDUP_P1`] (`0.55 s`) in Phase 1;
    ///   [`SWIPE_WINDUP_P2`] (`0.42 s`) in Phase 2. Transitions into
    ///   [`BossCatState::Swipe`].
    /// - **Warning**: Full duration (`0.42–0.55 s`). Ground layer paints the
    ///   exact crescent sector ahead of the cat (origin [`swipe_origin`],
    ///   radii `6..30 px`, `130°` arc) filling radially outward as `progress -> 1.0`.
    /// - **Dangerous area**: Body contact only (`18` dmg); warning fan is safe.
    /// - **Damage**: [`CONTACT_DAMAGE`] (`18`) on body touch; `0` in fan.
    /// - **Counter**: Back out beyond `42 px` from `pos`, sidestep behind the
    ///   cat's shoulders, or prepare a dash through the active strike.
    SwipeWindup,

    /// **Ferocious Claw Swipe (Triple Claw-Slash Arc).**
    ///
    /// - **Behavior**: Whips the raised forepaw across the front `130°` arc
    ///   (`progress = 0.0..0.45`) with three luminous claw-slash ribbons, then
    ///   decelerates into a follow-through recovery (`0.45..1.0`). In Phase 2,
    ///   the boss may chain a second `SwipeWindup` with negated `swipe_side`.
    /// - **Duration**: [`SWIPE_DURATION`] (`0.50 s` — `0.20 s` active strike +
    ///   `0.30 s` follow-through recovery).
    /// - **Warning**: Preceded by [`BossCatState::SwipeWindup`].
    /// - **Dangerous area**: Active during `progress` in `0.05..0.45`. Annular
    ///   sector centered at [`swipe_origin`] (`pos + facing * 12`), inner radius
    ///   [`SWIPE_INNER_RADIUS`] (`6 px`), outer radius [`SWIPE_RADIUS`] (`30 px`),
    ///   within `±SWIPE_HALF_ANGLE` (`±65°`) of `facing`.
    /// - **Damage**: [`SWIPE_DAMAGE`] (`22`, equal to a Brute).
    /// - **Counter**: Dash through (`0.18 s` i-frames covers nearly the full
    ///   `0.20 s` active swing) or flank behind the shoulders during recovery.
    Swipe,

    /// **Apex Territorial Roar (Phase Transition & Acoustic Zoning).**
    ///
    /// - **Behavior**: Plants all four paws wide, arches neck, pins ears flat,
    ///   and opens fanged jaws wide. Inhales (`progress = 0.0..0.35`), blasts
    ///   three concentric acoustic pressure rings (`0.35..0.90`), then settles
    ///   (`0.90..1.0`). Always triggered once when HP drops below [`PHASE2_HP_FRAC`],
    ///   and periodically thereafter (`~12 s` cooldown).
    /// - **Duration**: [`ROAR_DURATION`] (`1.30 s`).
    /// - **Warning**: `0.45 s` inhale (`progress = 0.0..0.35`) with contracting
    ///   breath streaks and a pulsing ground warning ring of radius
    ///   [`ROAR_OUTER_RADIUS`] (`44 px`) around the head (`pos + facing * 24`).
    /// - **Dangerous area**: Active during `progress = 0.35..0.90` around
    ///   `pos + facing * ROAR_ORIGIN_U`:
    ///   - Inner circle `r <= 26 px` ([`ROAR_INNER_RADIUS`]): deals [`ROAR_DAMAGE`]
    ///     (`12`) and pushes the rat outward.
    ///   - Outer ring `26..44 px` ([`ROAR_OUTER_RADIUS`]): gentle radial pushback
    ///     (`85 px/s`, no damage).
    ///   - Phase 2 bonus: at `progress = 0.42` and `0.68`, spawns `8` slow radial
    ///     sonic fang bullets (`speed = 65 px/s`, `radius = 2.5`, `dmg = 12`)
    ///     evenly spaced around `TAU` with wide weaving gaps.
    /// - **Damage**: [`ROAR_DAMAGE`] (`12`) inside `26 px` or per sonic bullet.
    /// - **Counter**: Retreat past `44 px` during the `0.45 s` inhale for a free
    ///   ranged DPS window, or weave/dash through the Phase 2 sonic projectiles.
    Roar,

    /// **Heavy Stagger / Flinch Reaction.**
    ///
    /// - **Behavior**: Head snaps sideways, eyes squint shut, jaws hiss, ears
    ///   flatten, and body shudders backward. Triggered by heavy burst-damage
    ///   thresholds, giant traps (mousetrap / sticky trap), or phase stagger —
    ///   normal chip hits should only pulse [`BossCatView::hit_flash`] without
    ///   interrupting attacks.
    /// - **Duration**: [`HURT_DURATION`] (`0.45 s`, extendable to `0.90 s` on
    ///   trap stun).
    /// - **Warning**: Immediate reaction.
    /// - **Dangerous area**: Reduced/disabled contact danger during stagger.
    /// - **Damage**: `0` (safe punish window).
    /// - **Counter**: Reward state — pour concentrated weapon damage into the
    ///   boss while it recovers.
    Hurt,

    /// **Defeat Collapse.**
    ///
    /// - **Behavior**: Convulsive final arch and silent snarl (`0.0..0.45`),
    ///   heavy collapse onto flank/belly with limbs splayed, spine slumped in a
    ///   limp curve, and eyes closing (`0.45..0.80`), then fading embers into
    ///   defeated stillness (`0.80..1.0`).
    /// - **Duration**: [`DEATH_DURATION`] (`1.80 s`).
    /// - **Warning**: None.
    /// - **Dangerous area**: None (disable all hitboxes immediately at `0.0`).
    /// - **Damage**: `0`.
    /// - **Counter**: Victory — drop boss XP burst / chest on completion.
    Death,
}

/// Pure, `Copy` render view for the Jungle Cat Boss.
///
/// Carries everything the renderer needs with zero references to simulation
/// structs so gameplay code can build it on the fly each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossCatView {
    /// World-space center of the jaguar's torso (ground footprint center; during
    /// `Pounce`, linearly interpolated from `pounce_origin` to `target` over
    /// `progress = 0.0..=POUNCE_LAND_PROGRESS`).
    pub pos: Vec2,
    /// World-space facing vector (normalized defensively if non-unit or zero).
    pub facing: Vec2,
    /// Current state in the boss's move set.
    pub state: BossCatState,
    /// Normalized progress through `state`, clamped to `0.0..=1.0`.
    pub progress: f32,
    /// Continuous animation clock in seconds (drives gait stride, tail wave,
    /// breathing, and eye shimmer).
    pub anim: f32,
    /// Recent hit-flash intensity (`0.0..=1.0`; fades out over ~`0.12 s`).
    pub hit_flash: f32,
    /// Remaining health fraction (`0.0..=1.0`; Phase 2 visual shift at `<= 0.50`).
    pub hp_frac: f32,
    /// World-space target point: rat position for eye-tracking in neutral
    /// states, or the marked pounce landing zone during `Crouch` and `Pounce`.
    pub target: Vec2,
    /// World-space launch anchor for `Pounce` (used to orient leap streaks and
    /// trailing shadow alignment).
    pub pounce_origin: Vec2,
    /// Which forepaw strikes during `SwipeWindup`/`Swipe`: `>= 0.0` = right paw
    /// (`+perp`), `< 0.0` = left paw (`-perp`).
    pub swipe_side: f32,
}

impl BossCatView {
    /// Construct a default stalking view looking toward `target`.
    pub fn new(pos: Vec2, facing: Vec2, target: Vec2) -> Self {
        Self {
            pos,
            facing,
            state: BossCatState::Stalk,
            progress: 0.0,
            anim: 0.0,
            hit_flash: 0.0,
            hp_frac: 1.0,
            target,
            pounce_origin: pos,
            swipe_side: 1.0,
        }
    }
}

// ============================================================================
// Hitbox & Geometry Helpers for Implementers
// ============================================================================

#[inline]
fn safe_facing(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-6 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// Returns the three overlapping `(world_center, radius)` collision circles
/// hugging the jaguar's haunches, chest/shoulders, and broad head.
pub fn body_hit_circles(view: &BossCatView) -> [(Vec2, f32); 3] {
    let f = safe_facing(view.facing);
    [
        (view.pos + f * HAUNCH_OFFSET_U, HAUNCH_RADIUS),
        (view.pos + f * CHEST_OFFSET_U, CHEST_RADIUS),
        (view.pos + f * HEAD_OFFSET_U, HEAD_RADIUS),
    ]
}

/// World-space origin of the claw-swipe damage sector (`pos + facing * SWIPE_ORIGIN_U`).
pub fn swipe_origin(view: &BossCatView) -> Vec2 {
    view.pos + safe_facing(view.facing) * SWIPE_ORIGIN_U
}

/// World-space origin of the roar acoustic waves (`pos + facing * ROAR_ORIGIN_U`).
pub fn roar_origin(view: &BossCatView) -> Vec2 {
    view.pos + safe_facing(view.facing) * ROAR_ORIGIN_U
}

/// Returns `true` while the jaguar is high enough mid-`Pounce` to leap over
/// obstacles, traps, and the player (draw above player/props; suppress body
/// contact damage).
#[inline]
pub fn is_airborne(view: &BossCatView) -> bool {
    view.state == BossCatState::Pounce && view.progress > 0.12 && view.progress < POUNCE_LAND_PROGRESS
}

/// Visual height lift (in screen `-y` pixels) during `Pounce` at `progress`.
#[inline]
pub fn pounce_altitude(progress: f32) -> f32 {
    let p = progress.clamp(0.0, 1.0);
    if p >= POUNCE_LAND_PROGRESS {
        0.0
    } else {
        let t = p / POUNCE_LAND_PROGRESS;
        let arc = (4.0 * t * (1.0 - t)).max(0.0).powf(0.85);
        arc * POUNCE_PEAK_HEIGHT
    }
}

// ============================================================================
// Internal Frame, Pose & Palette Structures
// ============================================================================

#[derive(Clone, Copy)]
struct Pose {
    body_pos: Vec2,
    ground_pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    su: f32,
    sv: f32,
    altitude: f32,
    sh_roll_l: f32,
    sh_roll_r: f32,
    haunch_wiggle: f32,
    spine_wave_amp: f32,
    spine_phase: f32,
    torso_twist: f32,
    ear_pin: f32,
    jaw_open: f32,
    head_turn: f32,
    /// Limp sideways slump curve during `Death` (`0.0..1.0`).
    death_slump: f32,
    /// Mane/hackle flare intensity (Phase 2 or `Roar`).
    hackle_flare: f32,
    enraged: f32,
    critical_pulse: f32,
    hit: f32,
    death: f32,
    alpha: f32,
    state: BossCatState,
    progress: f32,
    anim: f32,
    swipe_side: f32,
    accent: Rgb,
    target_screen: Vec2,
}

impl Pose {
    fn from_view(view: &BossCatView, cam: Vec2, accent: Rgb) -> Self {
        let f = safe_facing(view.facing);
        let p = f.perp();
        let prog = view.progress.clamp(0.0, 1.0);
        let hit = view.hit_flash.clamp(0.0, 1.0);
        let hp = view.hp_frac.clamp(0.0, 1.0);
        let swipe_side = if view.swipe_side < 0.0 { -1.0 } else { 1.0 };

        let enraged = if hp <= PHASE2_HP_FRAC { 1.0 } else { 0.0 };
        let pulse = 0.5 + 0.5 * (view.anim * 7.5).sin();
        let critical_pulse = if hp <= CRITICAL_HP_FRAC {
            (0.45 + 0.55 * pulse) * (1.0 - (hp / CRITICAL_HP_FRAC) * 0.4)
        } else {
            0.0
        };

        let ground_pos = view.pos - cam;
        let mut body_pos = ground_pos;
        let mut su = 1.0f32;
        let mut sv = 1.0f32;
        let mut altitude = 0.0f32;
        let mut sh_roll_l = 0.0f32;
        let mut sh_roll_r = 0.0f32;
        let mut haunch_wiggle = 0.0f32;
        let mut spine_wave_amp = 0.0f32;
        let spine_phase = view.anim * if enraged > 0.5 { 4.8 } else { 3.6 };
        let mut torso_twist = 0.0f32;
        let mut ear_pin = 0.12 * enraged;
        let mut jaw_open = 0.0f32;
        let mut head_turn = 0.0f32;
        let mut death_slump = 0.0f32;
        let mut hackle_flare = enraged;
        let mut death = 0.0f32;
        let mut alpha = 1.0f32;

        match view.state {
            BossCatState::Stalk => {
                // Deep predatory stalking gait: shoulders roll strongly forward & back!
                let stride = spine_phase.sin();
                sh_roll_l = stride * 3.4;
                sh_roll_r = -stride * 3.4;
                spine_wave_amp = 1.85;
                sv = 1.0 + 0.025 * (view.anim * 2.6).sin();
            }
            BossCatState::Crouch => {
                // Compress lengthwise, widen haunches, pin ears flat, wiggle hindquarters!
                let coil = (prog / 0.35).min(1.0);
                let locked = if prog >= CROUCH_LOCK_PROGRESS { 1.0 } else { 0.0 };
                su = 1.0 - 0.18 * coil;
                sv = 1.0 + 0.14 * coil;
                ear_pin = 0.85 + 0.15 * locked;
                let freq = 2.6 + 3.2 * prog;
                haunch_wiggle = (prog * TAU * freq).sin() * (3.1 + 2.6 * prog) * coil;
                body_pos -= f * (2.6 * coil);
                sh_roll_l = -1.4 * coil;
                sh_roll_r = -1.4 * coil;
            }
            BossCatState::Pounce => {
                altitude = pounce_altitude(prog);
                let lift_t = altitude / POUNCE_PEAK_HEIGHT;
                body_pos.y -= altitude;
                if prog < POUNCE_LAND_PROGRESS {
                    let t = prog / POUNCE_LAND_PROGRESS;
                    let stretch = (t * PI).sin();
                    su = 1.08 + 0.18 * stretch + 0.08 * lift_t;
                    sv = 0.94 + 0.11 * lift_t;
                    ear_pin = 0.95;
                    jaw_open = 0.55 + 0.35 * stretch;
                    sh_roll_l = 2.8 * stretch;
                    sh_roll_r = 2.8 * stretch;
                } else {
                    let imp = (prog - POUNCE_LAND_PROGRESS) / (1.0 - POUNCE_LAND_PROGRESS);
                    let squash = (1.0 - imp).powf(0.7);
                    su = 1.0 - 0.13 * squash;
                    sv = 1.0 + 0.17 * squash;
                    ear_pin = 0.72 * squash;
                    jaw_open = 0.38 * squash;
                    let shake = (imp * 28.0).sin() * 1.5 * squash;
                    body_pos += p * shake;
                }
            }
            BossCatState::SwipeWindup => {
                let c = prog.powf(0.75);
                su = 1.0 - 0.09 * c;
                sv = 1.0 + 0.06 * c;
                torso_twist = -swipe_side * 3.2 * c;
                ear_pin = 0.68 * c;
                jaw_open = 0.35 * c;
                if swipe_side > 0.0 {
                    sh_roll_r = -3.4 * c;
                    sh_roll_l = 1.8 * c;
                } else {
                    sh_roll_l = -3.4 * c;
                    sh_roll_r = 1.8 * c;
                }
                body_pos -= f * (2.2 * c);
            }
            BossCatState::Swipe => {
                let swing = if prog < SWIPE_ACTIVE_END {
                    let t = prog / SWIPE_ACTIVE_END;
                    -1.0 + 2.0 * (1.0 - (1.0 - t).powi(3))
                } else {
                    let t = (prog - SWIPE_ACTIVE_END) / (1.0 - SWIPE_ACTIVE_END);
                    1.0 - 0.75 * t
                };
                su = 1.0 + 0.09 * swing.max(0.0);
                torso_twist = swipe_side * 3.6 * swing;
                ear_pin = 0.85 - 0.45 * prog;
                jaw_open = 0.72 * (1.0 - prog * 0.65);
                if swipe_side > 0.0 {
                    sh_roll_r = 3.8 * swing;
                    sh_roll_l = -1.8 * swing;
                } else {
                    sh_roll_l = 3.8 * swing;
                    sh_roll_r = -1.8 * swing;
                }
                body_pos += f * (3.2 * swing.max(0.0));
            }
            BossCatState::Roar => {
                let (arch, open) = if prog < ROAR_BLAST_START {
                    let t = prog / ROAR_BLAST_START;
                    (t, 0.28 * t)
                } else if prog < ROAR_BLAST_END {
                    let t = (prog - ROAR_BLAST_START) / (ROAR_BLAST_END - ROAR_BLAST_START);
                    let vib = (t * 42.0).sin() * 0.06;
                    (1.0 + vib, 1.0)
                } else {
                    let t = (prog - ROAR_BLAST_END) / (1.0 - ROAR_BLAST_END);
                    (1.0 - t, 1.0 - t)
                };
                su = 1.0 - 0.08 * arch;
                sv = 1.0 + 0.14 * arch;
                ear_pin = 0.98 * arch;
                jaw_open = open;
                hackle_flare = (enraged + 0.85 * arch).min(1.25);
                if (ROAR_BLAST_START..=ROAR_BLAST_END).contains(&prog) {
                    let vib_x = (view.anim * 54.0 + prog * 31.0).sin() * 1.1;
                    let vib_y = (view.anim * 47.0 + prog * 23.0).cos() * 0.9;
                    body_pos += Vec2::new(vib_x, vib_y);
                }
            }
            BossCatState::Hurt => {
                let decay = 1.0 - prog;
                let jolt = (prog * PI).sin();
                su = 1.0 - 0.12 * jolt;
                sv = 1.0 + 0.09 * jolt;
                ear_pin = 1.0;
                jaw_open = 0.72 * decay;
                head_turn = 4.4 * jolt;
                let jitter = (prog * 36.0).sin() * 1.8 * decay;
                body_pos += -f * (4.5 * jolt) + p * jitter;
            }
            BossCatState::Death => {
                death = prog;
                if prog < 0.40 {
                    let t = prog / 0.40;
                    let spasm = (t * 28.0).sin() * 1.7 * (1.0 - t * 0.35);
                    su = 1.0 - 0.09 * (t * PI).sin();
                    sv = 1.0 + 0.11 * (t * PI).sin();
                    ear_pin = 1.0;
                    jaw_open = 0.90 * (1.0 - t * 0.35);
                    head_turn = 3.2 * t;
                    death_slump = 0.22 * t;
                    body_pos += p * spasm;
                } else {
                    let t = ((prog - 0.40) / 0.38).min(1.0);
                    let ease = 1.0 - (1.0 - t) * (1.0 - t);
                    su = 1.0 - 0.04 * ease;
                    sv = 1.0 + 0.22 * ease;
                    ear_pin = 0.92;
                    jaw_open = 0.22 * (1.0 - ease);
                    head_turn = 3.2 + 3.4 * ease;
                    death_slump = 0.22 + 0.78 * ease;
                    if prog > 0.84 {
                        alpha = 1.0 - ((prog - 0.84) / 0.16) * 0.48;
                    }
                }
            }
        }

        Self {
            body_pos,
            ground_pos,
            f,
            p,
            fl: f.dot(LIGHT),
            pl: p.dot(LIGHT),
            su,
            sv,
            altitude,
            sh_roll_l,
            sh_roll_r,
            haunch_wiggle,
            spine_wave_amp,
            spine_phase,
            torso_twist,
            ear_pin,
            jaw_open,
            head_turn,
            death_slump,
            hackle_flare,
            enraged,
            critical_pulse,
            hit,
            death,
            alpha,
            state: view.state,
            progress: prog,
            anim: view.anim,
            swipe_side,
            accent,
            target_screen: view.target - cam,
        }
    }

    /// Lateral spine offset `v(u)` in unscaled local units.
    #[inline]
    fn spine_v(&self, u: f32) -> f32 {
        let mut v = 0.0f32;
        if self.spine_wave_amp > 0.001 {
            let flex = ((18.0 - u) / 36.0).clamp(0.0, 1.0);
            v += self.spine_wave_amp * flex * (self.spine_phase - u * 0.11).sin();
        }
        if self.haunch_wiggle.abs() > 0.001 {
            let rear = ((-u - 2.0) / 18.0).clamp(0.0, 1.0);
            v += self.haunch_wiggle * rear * rear;
        }
        if self.torso_twist.abs() > 0.001 {
            let front = ((u + 6.0) / 28.0).clamp(0.0, 1.0);
            v += self.torso_twist * front;
        }
        if self.head_turn.abs() > 0.001 {
            let head_w = ((u - 10.0) / 14.0).clamp(0.0, 1.0);
            v += self.head_turn * head_w;
        }
        if self.death_slump > 0.001 {
            // Limp C-curve slump onto its flank in defeat.
            let norm = (u / 24.0).clamp(-1.0, 1.0);
            v += self.death_slump * (4.2 * norm * norm - 2.4);
        }
        v
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.body_pos.x;
        let dy = y as f32 - self.body_pos.y;
        (
            (dx * self.f.x + dy * self.f.y) / self.su,
            (dx * self.p.x + dy * self.p.y) / self.sv,
        )
    }

    #[inline]
    fn local_ground(&self, x: i32, y: i32, su: f32, sv: f32) -> (f32, f32) {
        let dx = x as f32 - self.ground_pos.x;
        let dy = y as f32 - self.ground_pos.y;
        (
            (dx * self.f.x + dy * self.f.y) / su,
            (dx * self.p.x + dy * self.p.y) / sv,
        )
    }

    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.body_pos + self.f * (u * self.su) + self.p * ((v + self.spine_v(u)) * self.sv)
    }

    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        let q = self.place(u, v);
        (q.x.round() as i32, q.y.round() as i32)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }
}

// ============================================================================
// Pixel, Blending, SDF & Procedural Jaguar Rosette Helpers
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
    let x1 = (cx + rr).min(pb.w as i32 - 1);
    let y0 = (cy - rr).max(0);
    let y1 = (cy + rr).min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        None
    } else {
        Some((x0, x1, y0, y1))
    }
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.001 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if a >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let bg = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(bg, c, a));
    }
}

#[inline]
fn darken_px(pb: &mut PixelBuffer, x: i32, y: i32, amount: f32) {
    if amount <= 0.001 || x < 0 || y < 0 {
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
fn shade3(shadow: Rgb, mid: Rgb, hi: Rgb, lum: f32) -> Rgb {
    let l = lum.clamp(0.0, 1.18);
    if l < 0.62 {
        mix(shadow, mid, l / 0.62)
    } else {
        mix(mid, hi, (l - 0.62) / 0.56)
    }
}

/// Sculptural golden-ivory hit flash that preserves dark rosette relief so the
/// struck jaguar flashes unmistakably bright without bleaching into a pale rat.
#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.001 {
        return c;
    }
    let luma = (c.0 as f32 * 0.3 + c.1 as f32 * 0.55 + c.2 as f32 * 0.15) / 255.0;
    // Dark rosettes warm toward glowing bronze-crimson; gold coat flashes bright amber-ivory.
    let target = if luma < 0.24 {
        (168, 76, 46)
    } else if hit > 0.6 {
        (255, 240, 186)
    } else {
        (255, 214, 128)
    };
    let strength = (0.22 + hit * 0.42).min(0.60);
    mix(c, target, strength)
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
fn sd_taper(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r0: f32, r1: f32) -> f32 {
    let pax = u - a.0;
    let pay = v - a.1;
    let bax = b.0 - a.0;
    let bay = b.1 - a.1;
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    let rad = r0 + (r1 - r0) * h;
    (dx * dx + dy * dy).sqrt() - rad
}

#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

/// Deterministic fast integer hash returning `0.0..1.0`.
#[inline]
fn hash2(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((iy as u32).wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(1_442_695_041));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

/// Evaluates the authentic jaguar coat pattern at local body coordinates `(u, vs)`:
/// returns `(dark_spot_weight, warm_core_weight)` in `0.0..=1.0`.
fn jaguar_pattern(u: f32, vs: f32) -> (f32, f32) {
    // Broken dark dorsal spot line down the spine.
    if u > -20.5 && u < 12.5 && vs.abs() < 1.45 {
        let wave = (u * 0.72).sin();
        if wave > -0.15 {
            let core = (1.0 - vs.abs() / 1.45).clamp(0.0, 1.0);
            let w = ((wave + 0.15) / 1.15 * core).clamp(0.0, 1.0);
            if w > 0.32 {
                return ((w * 0.92).min(0.95), 0.0);
            }
        }
    }

    // Crisp forehead, cheek & temple spots on the skull (keep muzzle bridge clean).
    if u >= 12.0 {
        if u > 24.0 && vs.abs() < 3.8 {
            return (0.0, 0.0);
        }
        // Subtle twin dark brow stripes flanking the nasal bridge.
        if u > 18.0 && u < 23.8 && (vs.abs() - 1.55).abs() < 0.55 {
            return (0.72, 0.0);
        }
        let cell = 3.4f32;
        let gx = (u / cell).floor() as i32;
        let gy = (vs / cell).floor() as i32;
        let mut best_d2 = 999.0f32;
        let mut best_r = 0.9f32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx_i = gx + dx;
                let cy_i = gy + dy;
                let sym_y = cy_i.abs();
                let jx = (cx_i as f32 + 0.22 + 0.56 * hash2(cx_i, sym_y, 1)) * cell;
                let jy = (cy_i as f32 + 0.22 + 0.56 * hash2(cx_i, sym_y, 2)) * cell;
                let du = u - jx;
                let dv = vs - jy;
                let d2 = du * du + dv * dv;
                if d2 < best_d2 {
                    best_d2 = d2;
                    best_r = 0.72 + 0.45 * hash2(cx_i, sym_y, 3);
                }
            }
        }
        let d = best_d2.sqrt();
        let spot = ((best_r - d) * 1.6).clamp(0.0, 1.0);
        return (spot * 0.84, 0.0);
    }

    // Torso, shoulders & haunches: bold broken jaguar rosettes with warm cinnamon cores!
    let cell = 6.3f32;
    let gx = (u / cell).floor() as i32;
    let gy = (vs / cell).floor() as i32;
    let mut best_d2 = 999.0f32;
    let mut best_du = 0.0f32;
    let mut best_dv = 0.0f32;
    let mut best_r = 2.0f32;
    let mut best_ang = 0.0f32;
    let mut best_has_dot = false;

    for dy in -1..=1 {
        for dx in -1..=1 {
            let cx_i = gx + dx;
            let cy_i = gy + dy;
            let jx = (cx_i as f32 + 0.18 + 0.64 * hash2(cx_i, cy_i, 11)) * cell;
            let jy = (cy_i as f32 + 0.18 + 0.64 * hash2(cx_i, cy_i, 17)) * cell;
            let jy = if jy.abs() < 2.2 {
                if jy >= 0.0 { 2.5 } else { -2.5 }
            } else {
                jy
            };
            let du = u - jx;
            let dv = vs - jy;
            let d2 = du * du + dv * dv;
            if d2 < best_d2 {
                best_d2 = d2;
                best_du = du;
                best_dv = dv;
                best_r = 1.85 + 0.72 * hash2(cx_i, cy_i, 23);
                best_ang = hash2(cx_i, cy_i, 29) * TAU;
                best_has_dot = hash2(cx_i, cy_i, 37) > 0.25;
            }
        }
    }

    let d = best_d2.sqrt();
    if d > best_r + 1.0 {
        return (0.0, 0.0);
    }

    let theta = best_dv.atan2(best_du) - best_ang;
    let petal = (theta * 2.0).cos() * 0.42 + (theta * 3.0 + 0.8).sin() * 0.28;
    let ring_dist = (d - best_r).abs();
    let ring_half_w = 0.68 + 0.16 * petal;

    let mut dark = 0.0f32;
    let mut warm_core = 0.0f32;

    if ring_dist < ring_half_w && petal > -0.34 {
        dark = ((ring_half_w - ring_dist) / 0.42).clamp(0.0, 1.0) * 0.94;
    } else if d < best_r - 0.35 {
        warm_core = 0.72;
        if best_has_dot && d < 0.78 {
            dark = ((0.78 - d) / 0.45).clamp(0.0, 1.0) * 0.82;
        }
    }

    (dark, warm_core)
}

/// Evaluates the jaguar's sculpted top-down body SDF in local `(u, v)` units.
///
/// Returns `(signed_distance, part_id)`:
/// - `0` = torso (haunches, waist, rolling chest/shoulders, neck)
/// - `1` = broad jaguar skull & muzzle
/// - `2` = rounded spotted jaguar ear
/// - `3` = Phase 2 / Roar bristled dorsal hackle spikes
fn body_sdf(pose: &Pose, u: f32, v: f32) -> (f32, u8) {
    let vs = v - pose.spine_v(u);
    let av = vs.abs();

    let haunch = sd_ellipse(u, vs, -14.8, 0.0, 8.6, 9.2);
    let waist = sd_capsule(u, vs, -12.0, 0.0, 1.0, 0.0, 6.8);
    let chest = sd_ellipse(u, vs, 3.8, 0.0, 8.2, 9.8);
    let sh_l = sd_ellipse(u, vs, 5.4 + pose.sh_roll_l, -5.3, 6.4, 5.3);
    let sh_r = sd_ellipse(u, vs, 5.4 + pose.sh_roll_r, 5.3, 6.4, 5.3);
    let shoulders = smin(chest, sh_l.min(sh_r), 2.2);
    let neck = sd_capsule(u, vs, 9.5, 0.0, 15.8, 0.0, 6.5);

    let rear = smin(haunch, waist, 2.6);
    let mut torso = smin(rear, shoulders, 2.8);
    torso = smin(torso, neck, 2.2);

    // Broad powerful jaguar skull + cheek ruffs + square whisker-pad muzzle.
    let cranium = sd_ellipse(u, vs, 19.6, 0.0, 6.2, 8.2);
    let cheeks = sd_ellipse(u, vs, 18.0, 0.0, 4.4, 8.9);
    let muzzle_len = 25.8 + 1.5 * pose.jaw_open;
    let muzzle = sd_taper(u, vs, (21.5, 0.0), (muzzle_len, 0.0), 5.5, 3.9);
    let wpad = sd_ellipse(u, av, 24.8, 2.2, 2.2, 2.1);
    let skull = smin(smin(cranium, cheeks, 1.8), smin(muzzle, wpad, 1.1), 1.5);

    let core = smin(torso, skull, 1.5);

    // Prominent rounded jaguar ears standing proud of the rear skull corners
    // (folding cleanly flat along the neck when `ear_pin -> 1.0`).
    let eu_base = 15.8 - 1.9 * pose.ear_pin;
    let ev_base = 6.3 - 0.9 * pose.ear_pin;
    let eu_tip = 14.3 - 3.9 * pose.ear_pin;
    let ev_tip = 10.1 - 2.3 * pose.ear_pin;
    let ear = sd_taper(u, av, (eu_base, ev_base), (eu_tip, ev_tip), 2.85, 1.9);

    // Bristled mane/hackle fur spikes during Phase 2 or Roar!
    let mut d_hackle = 999.0f32;
    if pose.hackle_flare > 0.25 && u > -1.0 && u < 17.5 && av > 4.8 {
        let hf = pose.hackle_flare.min(1.25);
        let h0 = sd_taper(u, av, (14.2, 6.2), (11.4, 9.6 + 1.4 * hf), 1.45, 0.28);
        let h1 = sd_taper(u, av, (10.6, 7.6), (7.6, 11.4 + 1.8 * hf), 1.6, 0.30);
        let h2 = sd_taper(u, av, (6.5, 8.6), (3.4, 12.2 + 1.9 * hf), 1.6, 0.32);
        let h3 = sd_taper(u, av, (2.2, 8.4), (-0.6, 11.6 + 1.6 * hf), 1.45, 0.30);
        d_hackle = h0.min(h1).min(h2).min(h3);
    }

    if ear < core && ear <= d_hackle {
        (ear, 2)
    } else if d_hackle < core {
        (d_hackle, 3)
    } else if skull < torso {
        (core, 1)
    } else {
        (core, 0)
    }
}

/// Anatomical height bump (rolling shoulder blades, haunch domes, brow ridge)
/// to enrich top-left directional lighting across the big cat's musculature.
#[inline]
fn anatomy_bump(pose: &Pose, u: f32, vs: f32) -> (f32, f32, f32) {
    let mut dnu = 0.0f32;
    let mut dnv = 0.0f32;
    let mut lift = 0.0f32;

    for &(cu, cv) in &[
        (5.6 + pose.sh_roll_l, -4.9f32),
        (5.6 + pose.sh_roll_r, 4.9f32),
    ] {
        let du = (u - cu) / 5.4;
        let dv = (vs - cv) / 4.2;
        let r2 = du * du + dv * dv;
        if r2 < 1.0 {
            let w = 1.0 - r2;
            dnu += du * 0.56 * w;
            dnv += dv * 0.56 * w;
            lift += 0.28 * w;
        }
    }

    for &cv in &[-4.4f32, 4.4] {
        let du = (u + 14.5) / 6.2;
        let dv = (vs - cv) / 4.2;
        let r2 = du * du + dv * dv;
        if r2 < 1.0 {
            let w = 1.0 - r2;
            dnu += du * 0.36 * w;
            dnv += dv * 0.36 * w;
            lift += 0.18 * w;
        }
    }

    if u > 15.0 {
        let du = (u - 20.4) / 5.4;
        let dv = vs / 5.0;
        let r2 = du * du + dv * dv;
        if r2 < 1.0 {
            let w = 1.0 - r2;
            dnu += du * 0.38 * w;
            dnv += dv * 0.38 * w;
            lift += 0.24 * w;
        }
    }

    (dnu, dnv, lift)
}

// ============================================================================
// Public Ground Layer Drawing (Shadows & Telegraphs)
// ============================================================================

/// Draw the Jungle Cat Boss's **ground layer**: attack telegraphs, impact
/// shockwaves, and grounded/airborne drop shadow.
///
/// **Layer**: Call *after* `art_floor::draw_floor` and stains, and *before*
/// enemies, player, and [`draw_boss_cat`].
pub fn draw_boss_cat_ground(pb: &mut PixelBuffer, view: &BossCatView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);

    let vw = pb.w as f32;
    let vh = pb.h as f32;
    let body_vis = pose.ground_pos.x >= -78.0
        && pose.ground_pos.x <= vw + 78.0
        && pose.ground_pos.y >= -78.0
        && pose.ground_pos.y <= vh + 78.0;
    let target_vis = pose.target_screen.x >= -48.0
        && pose.target_screen.x <= vw + 48.0
        && pose.target_screen.y >= -48.0
        && pose.target_screen.y <= vh + 48.0;
    if !body_vis && !target_vis {
        return;
    }

    // 1) State-specific ground telegraphs & impact decals.
    match pose.state {
        BossCatState::Crouch => {
            draw_pounce_telegraph(pb, &pose, false);
        }
        BossCatState::Pounce => {
            draw_pounce_telegraph(pb, &pose, true);
            if pose.progress >= POUNCE_LAND_PROGRESS {
                draw_pounce_impact_ring(pb, &pose);
            }
        }
        BossCatState::SwipeWindup => {
            draw_swipe_telegraph(pb, &pose, false);
        }
        BossCatState::Swipe => {
            if pose.progress < SWIPE_ACTIVE_END + 0.14 {
                draw_swipe_telegraph(pb, &pose, true);
            }
        }
        BossCatState::Roar => {
            draw_roar_ground(pb, &pose);
        }
        BossCatState::Death if pose.death > 0.42 => {
            // Subtle dusty floor settle ring beneath the collapsed boss.
            let settle = ((pose.death - 0.42) / 0.35).clamp(0.0, 1.0);
            let fade = (1.0 - ((pose.death - 0.55) / 0.45).clamp(0.0, 1.0)) * 0.45;
            draw_soft_ring(pb, pose.ground_pos, 15.0 + 11.0 * settle, 2.2, (165, 148, 128), fade);
        }
        _ => {}
    }

    // 2) Predator ground drop shadow (visibly grows larger & darker as Pounce dives!).
    if body_vis {
        draw_ground_shadow(pb, &pose);
    }
}

/// Ground drop shadow: crisp contact shadow while grounded; clearly **grows
/// larger and darker** directly underneath the plunging jaguar during `Pounce`.
fn draw_ground_shadow(pb: &mut PixelBuffer, pose: &Pose) {
    // A cast shadow is floor, not body: stamp it with the floor pen so the
    // lighting's rim and sheen never treat it as a creature (batch 13).
    let prev_pen = pb.set_pen(0);
    draw_ground_shadow_body(pb, pose);
    pb.set_pen(prev_pen);
}

fn draw_ground_shadow_body(pb: &mut PixelBuffer, pose: &Pose) {
    let (sh_center, sh_su, sh_sv, sh_alpha) = if pose.state == BossCatState::Pounce {
        let p = pose.progress;
        let approach = (p / POUNCE_LAND_PROGRESS).clamp(0.0, 1.0);
        // Dramatic readable shadow growth: compact at launch (0.50x) -> huge looming (1.24x) at impact!
        let growth = 0.50 + 0.74 * approach.powf(0.8);
        let ofs = Vec2::new(1.2 + 2.2 * (1.0 - approach), 1.8 + 3.0 * (1.0 - approach));
        let alpha = (0.48 + 0.44 * approach) * pose.alpha;
        (pose.ground_pos + ofs, pose.su * growth, pose.sv * growth * 1.14, alpha)
    } else if pose.state == BossCatState::Death {
        let flat = 1.0 + 0.14 * pose.death;
        (
            pose.ground_pos + Vec2::new(2.0, 2.5),
            pose.su * flat,
            pose.sv * flat,
            0.52 * pose.alpha,
        )
    } else {
        (
            pose.ground_pos + Vec2::new(2.5, 3.2),
            pose.su,
            pose.sv,
            0.48 * pose.alpha,
        )
    };

    let reach = 35.0 * sh_su.max(sh_sv);
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, sh_center, reach) else {
        return;
    };

    let sh_dx = sh_center.x - pose.ground_pos.x;
    let sh_dy = sh_center.y - pose.ground_pos.y;
    let shadow_ink: Rgb = (3, 4, 6);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local_ground(
                x - sh_dx.round() as i32,
                y - sh_dy.round() as i32,
                sh_su,
                sh_sv,
            );
            if !(-26.0..=30.0).contains(&u) || v.abs() > 15.0 {
                continue;
            }
            let (d, _) = body_sdf(pose, u, v);
            let edge = if pose.state == BossCatState::Pounce { 2.4 } else { 1.8 };
            if d < edge {
                let soft = ((edge - d) / (edge + 1.0)).clamp(0.0, 1.0);
                blend_px(pb, x, y, shadow_ink, sh_alpha * soft);
            }
        }
    }
}

/// Pounce marked landing zone at `view.target` + predator sightline from snout.
fn draw_pounce_telegraph(pb: &mut PixelBuffer, pose: &Pose, airborne: bool) {
    let tc = pose.target_screen;
    let r = POUNCE_LAND_RADIUS;
    let locked = airborne || pose.progress >= CROUCH_LOCK_PROGRESS;
    let pulse = 0.5 + 0.5 * (pose.anim * 16.0).sin();

    let ring_col = if locked {
        mix((255, 76, 48), (255, 224, 110), 0.35 + 0.45 * pulse)
    } else {
        mix((232, 148, 44), (255, 208, 92), 0.3 + 0.3 * pulse)
    };
    let fill_col = if locked { (195, 42, 32) } else { (170, 96, 26) };

    let fill_t = if airborne {
        (pose.progress / POUNCE_LAND_PROGRESS).clamp(0.0, 1.0)
    } else {
        pose.progress * 0.45
    };
    let inner_r = (r * (1.0 - fill_t * 0.85)).max(3.0);

    if let Some((x0, x1, y0, y1)) = clamp_bbox(pb, tc, r + 2.5) {
        let rot = pose.anim * if locked { 2.8 } else { 1.4 };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - tc.x;
                let dy = y as f32 - tc.y;
                let d2 = dx * dx + dy * dy;
                if d2 > (r + 1.6) * (r + 1.6) {
                    continue;
                }
                let d = d2.sqrt();
                let ang = dy.atan2(dx);

                if (d - r).abs() <= 0.95 {
                    let dash = ((ang + rot) * 8.0).sin() > -0.25;
                    if locked || dash {
                        blend_px(pb, x, y, ring_col, if locked { 0.92 } else { 0.72 });
                    }
                } else if d < r {
                    let radial = 1.0 - d / r;
                    let fill_a = if locked {
                        0.15 + 0.16 * fill_t + 0.08 * radial
                    } else {
                        0.09 + 0.08 * radial
                    };
                    blend_px(pb, x, y, fill_col, fill_a);

                    // Growing dark predator impact shadow right inside the landing zone!
                    if airborne {
                        let grow_r = r * (0.24 + 0.72 * fill_t);
                        if d < grow_r {
                            let core_a = 0.42 * fill_t * (1.0 - (d / grow_r).powi(2));
                            blend_px(pb, x, y, (4, 4, 8), core_a);
                        }
                    }

                    if (d - inner_r).abs() <= 0.75 {
                        blend_px(pb, x, y, (255, 232, 168), if locked { 0.78 } else { 0.48 });
                    }

                    let card = (ang * 2.0).sin().abs();
                    if card < 0.16 && d > r - 4.5 && d < r - 1.2 {
                        blend_px(pb, x, y, ring_col, 0.80);
                    }
                }
            }
        }
    }

    if !airborne {
        let start = pose.place(26.0, 0.0);
        let delta = tc - start;
        let dist = delta.len();
        if dist > r + 4.0 {
            let dir = delta * (1.0 / dist);
            let steps = ((dist - r) / 3.5).clamp(2.0, 36.0) as i32;
            let scroll = (pose.anim * 10.0) % 3.5;
            for i in 0..steps {
                let s = i as f32 * 3.5 + scroll;
                if s >= dist - r {
                    break;
                }
                let q = start + dir * s;
                let a = if locked { 0.72 } else { 0.40 };
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, ring_col, a);
            }
        }
    }
}

/// Impact dust & Phase 2 expanding shockwave ring when `Pounce` lands.
fn draw_pounce_impact_ring(pb: &mut PixelBuffer, pose: &Pose) {
    let imp = ((pose.progress - POUNCE_LAND_PROGRESS) / (1.0 - POUNCE_LAND_PROGRESS))
        .clamp(0.0, 1.0);
    let fade = 1.0 - imp;
    let tc = pose.target_screen;

    let r1 = 14.0 + 13.0 * imp;
    let col1 = mix((255, 224, 152), pose.accent, 0.18);
    draw_soft_ring(pb, tc, r1, 2.3, col1, 0.82 * fade);

    if pose.enraged > 0.5 {
        let r2 = POUNCE_LAND_RADIUS + (POUNCE_SHOCKWAVE_RADIUS - POUNCE_LAND_RADIUS) * imp;
        let col2 = mix((255, 84, 42), (255, 232, 130), fade);
        draw_soft_ring(pb, tc, r2, 3.2, col2, 0.90 * fade);
    }
}

/// Ground-layer crescent sector warning for `SwipeWindup` (and brief fade in `Swipe`).
fn draw_swipe_telegraph(pb: &mut PixelBuffer, pose: &Pose, active: bool) {
    let origin = pose.ground_pos + pose.f * SWIPE_ORIGIN_U;
    let r_out = SWIPE_RADIUS;
    let r_in = SWIPE_INNER_RADIUS;
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, origin, r_out + 2.0) else {
        return;
    };

    let fill_prog = if active { 1.0 } else { pose.progress };
    let fade = if active {
        (1.0 - (pose.progress / (SWIPE_ACTIVE_END + 0.14)).powi(2)).max(0.0) * 0.55
    } else {
        1.0
    };
    let sweep_r = r_in + (r_out - r_in) * fill_prog;
    let edge_col = if active {
        (255, 212, 128)
    } else {
        mix((240, 136, 46), (255, 76, 52), pose.progress)
    };
    let fill_col = if active { (235, 86, 42) } else { (196, 68, 34) };
    let cos_lim = SWIPE_HALF_ANGLE.cos();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - origin.x;
            let dy = y as f32 - origin.y;
            let d2 = dx * dx + dy * dy;
            if d2 < (r_in - 1.0) * (r_in - 1.0) || d2 > (r_out + 1.2) * (r_out + 1.2) {
                continue;
            }
            let d = d2.sqrt();
            if d < 0.01 {
                continue;
            }
            let cos_a = (dx * pose.f.x + dy * pose.f.y) / d;
            if cos_a < cos_lim - 0.05 {
                continue;
            }
            let in_angle = cos_a >= cos_lim;
            if active {
                // During the active slash, drop the rigid geometric border so the
                // dynamic curved claw-slash ribbons completely own the silhouette.
                if in_angle && d >= r_in && d <= r_out {
                    let radial = (d - r_in) / (r_out - r_in);
                    blend_px(pb, x, y, fill_col, (0.14 + 0.14 * radial) * fade);
                }
                continue;
            }
            let near_side = (cos_a - cos_lim).abs() < 0.055 && d >= r_in && d <= r_out;
            let near_outer = in_angle && (d - r_out).abs() <= 0.95;
            let near_front = in_angle && (d - sweep_r).abs() <= 0.85;

            if near_outer || near_side {
                blend_px(pb, x, y, edge_col, (0.58 + 0.34 * fill_prog) * fade);
            } else if near_front {
                blend_px(pb, x, y, (255, 232, 160), 0.74 * fade);
            } else if in_angle && d >= r_in && d <= r_out {
                let inside_fill = if d <= sweep_r { 0.25 } else { 0.10 };
                blend_px(pb, x, y, fill_col, inside_fill * fade);
            }
        }
    }
}

/// Ground-layer acoustic warning circle for `Roar` (`ROAR_OUTER_RADIUS = 44 px`).
fn draw_roar_ground(pb: &mut PixelBuffer, pose: &Pose) {
    let origin = pose.ground_pos + pose.f * ROAR_ORIGIN_U;
    let p = pose.progress;
    if p < ROAR_BLAST_START {
        let t = p / ROAR_BLAST_START;
        let col = mix((235, 168, 66), (255, 96, 54), t);
        draw_dashed_ring(pb, origin, ROAR_OUTER_RADIUS, col, 0.48 + 0.32 * t, pose.anim * 1.4);
        draw_soft_ring(pb, origin, ROAR_INNER_RADIUS, 1.3, (255, 118, 62), 0.36 + 0.44 * t);
        let r_in = ROAR_OUTER_RADIUS * (1.0 - t * 0.78);
        draw_soft_ring(pb, origin, r_in, 1.4, (255, 224, 148), 0.58 * t);
    } else if p <= ROAR_BLAST_END {
        let t = (p - ROAR_BLAST_START) / (ROAR_BLAST_END - ROAR_BLAST_START);
        let fade = (1.0 - t * 0.45).clamp(0.0, 1.0);
        let col = if pose.enraged > 0.5 {
            (255, 86, 48)
        } else {
            (242, 174, 68)
        };
        draw_soft_ring(pb, origin, ROAR_INNER_RADIUS, 1.6, col, 0.58 * fade);
        draw_dashed_ring(
            pb,
            origin,
            ROAR_OUTER_RADIUS,
            mix(col, (255, 224, 148), 0.35),
            0.55 * fade,
            pose.anim * 2.2,
        );
    }
}

fn draw_dashed_ring(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    col: Rgb,
    alpha: f32,
    rot: f32,
) {
    if alpha <= 0.01 || radius <= 1.0 {
        return;
    }
    let steps = (radius * 7.5).round().max(24.0) as i32;
    for i in 0..steps {
        let a = i as f32 / steps as f32 * TAU;
        if ((a + rot) * 12.0).sin() > -0.15 {
            let x = (center.x + a.cos() * radius).round() as i32;
            let y = (center.y + a.sin() * radius).round() as i32;
            blend_px(pb, x, y, col, alpha);
        }
    }
}

fn draw_soft_ring(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    thickness: f32,
    col: Rgb,
    alpha: f32,
) {
    if alpha <= 0.01 || radius <= 0.5 {
        return;
    }
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, center, radius + thickness + 1.0) else {
        return;
    };
    let r_min = (radius - thickness).max(0.0);
    let r_max = radius + thickness;
    let r_min2 = r_min * r_min;
    let r_max2 = r_max * r_max;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d2 = dx * dx + dy * dy;
            if d2 < r_min2 || d2 > r_max2 {
                continue;
            }
            let dist = (d2.sqrt() - radius).abs();
            let fall = (1.0 - dist / thickness).clamp(0.0, 1.0);
            blend_px(pb, x, y, col, alpha * fall);
        }
    }
}

// ============================================================================
// Public Entity Layer Drawing (Jaguar Body, Limbs, Tail, Face & Attack VFX)
// ============================================================================

/// Draw the Jungle Cat Boss's **body and active attack VFX**: expressive
/// rosette-banded tail, articulated clawed paws, sculpted golden-ochre jaguar
/// torso with rolling shoulders & rosettes, broad spotted ears, rat-tracking
/// eyes, roaring jaws/fangs, triple claw-slash arcs, and acoustic roar rings.
///
/// **Layer**: Call in the entity pass alongside enemies. When [`is_airborne`]
/// returns `true`, call *after* props and the player so the leaping jaguar
/// passes visibly overhead.
pub fn draw_boss_cat(pb: &mut PixelBuffer, view: &BossCatView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let pose = Pose::from_view(view, cam, accent);

    let margin = match pose.state {
        BossCatState::Roar => 72.0,
        BossCatState::Swipe | BossCatState::SwipeWindup => 58.0,
        _ => 64.0,
    };
    if pose.body_pos.x < -margin
        || pose.body_pos.x > pb.w as f32 + margin
        || pose.body_pos.y < -margin
        || pose.body_pos.y > pb.h as f32 + margin
    {
        return;
    }

    // 0) Airborne wind/speed streaks peeling off the flanks during Pounce flight.
    if pose.state == BossCatState::Pounce && pose.progress > 0.08 && pose.progress < POUNCE_LAND_PROGRESS {
        draw_pounce_speed_streaks(pb, &pose);
    }

    // 1) Expressive rosette-ringed jaguar tail.
    draw_tail(pb, &pose);

    // 2) Articulated hindlegs & non-striking forelegs.
    draw_paws(pb, &pose, false);

    // 3) Sculpted muscular jaguar body, rolling shoulders, skull, ears & rosettes.
    draw_body(pb, &pose);

    // 4) Open fanged jaws (Roar / Hurt / Pounce / early Death).
    if pose.jaw_open > 0.15 {
        draw_jaws(pb, &pose);
    }

    // 5) Fierce rat-tracking almond eyes, jaguar nose & pale whisker flecks.
    draw_face(pb, &pose);

    // 6) Raised / lunging forepaw overlay (SwipeWindup, Swipe, Pounce).
    draw_paws(pb, &pose, true);

    // 7) Active attack VFX on top: crouch wiggle arcs, claw-slash ribbons, roar shockwaves, hurt sparks, death embers.
    match pose.state {
        BossCatState::Crouch => draw_crouch_wiggle_arcs(pb, &pose),
        BossCatState::Swipe => draw_swipe_claw_arcs(pb, &pose),
        BossCatState::Roar => draw_roar_waves(pb, &pose),
        BossCatState::Hurt => draw_hurt_impact_ticks(pb, &pose),
        BossCatState::Death => draw_death_embers(pb, &pose),
        _ => {}
    }
}

struct CoatPalette {
    shadow: Rgb,
    mid: Rgb,
    hi: Rgb,
    cinnamon: Rgb,
    spot: Rgb,
    cream: Rgb,
    ear_back: Rgb,
    ear_inner: Rgb,
    rim: Rgb,
}

fn coat_palette(pose: &Pose) -> CoatPalette {
    let e = pose.enraged;
    let dead = ((pose.death - 0.30) / 0.70).clamp(0.0, 1.0);
    let mut shadow = mix((96, 48, 18), (86, 28, 14), e);
    shadow = mix(shadow, scale(pose.accent, 0.38), 0.14);
    let mut mid = mix((222, 152, 48), (206, 110, 32), e);
    let mut hi = mix((254, 222, 138), (255, 198, 98), e);
    let mut cinnamon = mix((166, 94, 28), (144, 56, 20), e);
    let spot = mix((28, 15, 11), (18, 9, 9), e);
    let mut cream = mix((248, 232, 190), (242, 206, 158), e);

    if dead > 0.0 {
        let ash_sh = (34, 34, 40);
        let ash_mid = (96, 86, 76);
        let ash_hi = (142, 132, 118);
        shadow = mix(shadow, ash_sh, dead * 0.68);
        mid = mix(mid, ash_mid, dead * 0.64);
        hi = mix(hi, ash_hi, dead * 0.62);
        cinnamon = mix(cinnamon, ash_sh, dead * 0.58);
        cream = mix(cream, ash_hi, dead * 0.56);
    }

    let rim = if pose.hit > 0.55 {
        (146, 86, 44)
    } else if pose.enraged > 0.5 && pose.critical_pulse > 0.3 {
        (36, 8, 6)
    } else {
        (0, 0, 0)
    };

    CoatPalette {
        shadow,
        mid,
        hi,
        cinnamon,
        spot,
        cream,
        ear_back: (24, 15, 13),
        ear_inner: (188, 108, 90),
        rim,
    }
}

/// Subtle directional wind streaks peeling off the leaping jaguar mid-`Pounce`.
fn draw_pounce_speed_streaks(pb: &mut PixelBuffer, pose: &Pose) {
    let t = pose.progress / POUNCE_LAND_PROGRESS;
    let vis = (t * PI).sin();
    let streak_col = mix((235, 222, 186), pose.accent, 0.32);
    for &(u0, v0, len) in &[
        (8.0f32, -13.0f32, 13.0f32),
        (8.0, 13.0, 13.0),
        (-12.0, -11.5, 11.0),
        (-12.0, 11.5, 11.0),
    ] {
        for j in 0..8 {
            let frac = j as f32 / 8.0;
            let q = pose.place(u0 - frac * len, v0);
            let f = vis * (1.0 - frac) * 0.42;
            pb.plot_add(q.x.round() as i32, q.y.round() as i32, scale(streak_col, f));
        }
    }
}

/// Long (~34 px) muscular jaguar tail with dark rosette rings & charcoal brush tip.
fn draw_tail(pb: &mut PixelBuffer, pose: &Pose) {
    const STEPS: usize = 30;
    let pal = coat_palette(pose);
    let len = match pose.state {
        BossCatState::Pounce => TAIL_LENGTH * 1.12,
        BossCatState::Crouch => TAIL_LENGTH * 0.94,
        _ => TAIL_LENGTH,
    };
    let step = len / STEPS as f32;
    let back = pose.f * -1.0;

    let mut pts = [Vec2::ZERO; STEPS];
    let mut p = pose.place(-21.5, 0.0);
    for (i, slot) in pts.iter_mut().enumerate() {
        let s = (i as f32 + 0.5) / STEPS as f32;
        let theta = match pose.state {
            BossCatState::Stalk => {
                0.78 * (s * PI * 1.15).sin()
                    + 0.42 * s * (pose.anim * 2.8 - s * 4.2).sin()
            }
            BossCatState::Crouch => {
                let tip = s.powf(2.2);
                let twitch = (pose.progress * TAU * (4.0 + 4.5 * pose.progress)).sin();
                0.35 * s + 1.38 * tip * twitch
            }
            BossCatState::Pounce => 0.14 * s * (pose.anim * 9.0 - s * 5.0).sin(),
            BossCatState::SwipeWindup | BossCatState::Swipe => {
                let dir = -pose.swipe_side;
                dir * (0.95 * s.powf(1.1)) + 0.35 * s * (pose.progress * PI * 2.0).sin()
            }
            BossCatState::Roar => {
                0.65 * s + 0.34 * s * (pose.anim * 24.0 - s * 6.0).sin()
            }
            BossCatState::Hurt => -1.15 * s * (1.0 - pose.progress * 0.4),
            BossCatState::Death => 1.55 * s.powf(1.2),
        };
        p += back.rotate(theta) * step;
        *slot = p;
    }

    let bristle = if pose.state == BossCatState::Roar || pose.enraged > 0.5 {
        0.38
    } else {
        0.0
    };
    let a = pose.alpha;
    for (i, &pt) in pts.iter().enumerate() {
        let s = i as f32 / (STEPS - 1) as f32;
        // Slight brush flare at the dark jaguar tail tip!
        let tip_flare = if s > 0.85 { 0.35 } else { 0.0 };
        let rad = (2.4 * (1.0 - s * 0.40) + bristle * (1.0 - s * 0.5) + tip_flare).max(1.2);
        let band = ((s * 6.5).sin() > 0.18) || s > 0.83;
        let ri = (rad + 1.1).ceil() as i32;
        let cx = pt.x.round() as i32;
        let cy = pt.y.round() as i32;

        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d > rad + 1.05 {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if d > rad {
                    blend_px(pb, x, y, pal.rim, 0.62 * a);
                } else {
                    let nx = dx as f32 / rad.max(1.0);
                    let ny = dy as f32 / rad.max(1.0);
                    let lit = nx * LIGHT.x + ny * LIGHT.y;
                    let dome = (1.0 - (d / rad).powi(2)).clamp(0.0, 1.0);
                    let lum = 0.44 + 0.34 * dome + 0.60 * lit;
                    let mut col = shade3(pal.shadow, pal.mid, pal.hi, lum);
                    if band {
                        col = mix(col, pal.spot, if s > 0.83 { 0.86 } else { 0.74 });
                    }
                    blend_px(pb, x, y, apply_hit(col, pose.hit), a);
                }
            }
        }
    }
}

/// Articulated 2-segment jaguar limbs (hip/shoulder -> elbow/knee -> broad clawed paw).
struct LimbDesc {
    is_front: bool,
    side: f32,
    hip: (f32, f32),
    elbow: (f32, f32),
    paw: (f32, f32),
    claws_out: bool,
    overlay: bool,
}

fn draw_paws(pb: &mut PixelBuffer, pose: &Pose, overlay: bool) {
    let pal = coat_palette(pose);
    let claws_always = pose.enraged > 0.5;

    let mut limbs = [
        LimbDesc { is_front: true, side: -1.0, hip: (6.0, -7.2), elbow: (9.5, -10.0), paw: (13.0, -10.5), claws_out: claws_always, overlay: false },
        LimbDesc { is_front: true, side: 1.0, hip: (6.0, 7.2), elbow: (9.5, 10.0), paw: (13.0, 10.5), claws_out: claws_always, overlay: false },
        LimbDesc { is_front: false, side: -1.0, hip: (-14.0, -6.8), elbow: (-16.8, -9.8), paw: (-19.5, -10.2), claws_out: false, overlay: false },
        LimbDesc { is_front: false, side: 1.0, hip: (-14.0, 6.8), elbow: (-16.8, 9.8), paw: (-19.5, 10.2), claws_out: false, overlay: false },
    ];

    match pose.state {
        BossCatState::Stalk => {
            let st = pose.spine_phase;
            let fl_sw = st.sin() * 5.6;
            let fr_sw = -fl_sw;
            let hl_sw = (st + PI * 0.85).sin() * 4.8;
            let hr_sw = -hl_sw;
            limbs[0] = LimbDesc {
                is_front: true, side: -1.0,
                hip: (6.0 + pose.sh_roll_l, -7.2),
                elbow: (8.8 + fl_sw * 0.55, -10.6),
                paw: (12.4 + fl_sw, -10.4 - 1.2 * st.cos().max(0.0)),
                claws_out: claws_always, overlay: false,
            };
            limbs[1] = LimbDesc {
                is_front: true, side: 1.0,
                hip: (6.0 + pose.sh_roll_r, 7.2),
                elbow: (8.8 + fr_sw * 0.55, 10.6),
                paw: (12.4 + fr_sw, 10.4 + 1.2 * (-st.cos()).max(0.0)),
                claws_out: claws_always, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -6.8),
                elbow: (-16.2 + hl_sw * 0.45, -10.4),
                paw: (-18.8 + hl_sw, -10.2),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 6.8),
                elbow: (-16.2 + hr_sw * 0.45, 10.4),
                paw: (-18.8 + hr_sw, 10.2),
                claws_out: false, overlay: false,
            };
        }
        BossCatState::Crouch => {
            let c = (pose.progress / 0.35).min(1.0);
            limbs[0] = LimbDesc {
                is_front: true, side: -1.0,
                hip: (5.0, -7.8), elbow: (10.0, -12.4), paw: (16.0 + 1.5 * c, -11.4),
                claws_out: true, overlay: false,
            };
            limbs[1] = LimbDesc {
                is_front: true, side: 1.0,
                hip: (5.0, 7.8), elbow: (10.0, 12.4), paw: (16.0 + 1.5 * c, 11.4),
                claws_out: true, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.2), elbow: (-15.5, -12.2), paw: (-11.0, -11.2),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.2), elbow: (-15.5, 12.2), paw: (-11.0, 11.2),
                claws_out: false, overlay: false,
            };
        }
        BossCatState::Pounce => {
            if pose.progress < POUNCE_LAND_PROGRESS {
                // Curved predatory grabbing forelimbs (elbows flare out, clawed paws curve in to snatch!).
                let t = pose.progress / POUNCE_LAND_PROGRESS;
                let reach = (t * PI).sin();
                limbs[0] = LimbDesc {
                    is_front: true, side: -1.0,
                    hip: (8.0, -7.2),
                    elbow: (18.5 + 1.8 * reach, -11.6 - 1.2 * reach),
                    paw: (27.5 + 3.0 * reach, -7.8 - 1.0 * reach),
                    claws_out: true, overlay: true,
                };
                limbs[1] = LimbDesc {
                    is_front: true, side: 1.0,
                    hip: (8.0, 7.2),
                    elbow: (18.5 + 1.8 * reach, 11.6 + 1.2 * reach),
                    paw: (27.5 + 3.0 * reach, 7.8 + 1.0 * reach),
                    claws_out: true, overlay: true,
                };
                limbs[2] = LimbDesc {
                    is_front: false, side: -1.0,
                    hip: (-14.5, -6.4),
                    elbow: (-21.0, -9.8),
                    paw: (-27.8 - 2.2 * reach, -7.6),
                    claws_out: true, overlay: false,
                };
                limbs[3] = LimbDesc {
                    is_front: false, side: 1.0,
                    hip: (-14.5, 6.4),
                    elbow: (-21.0, 9.8),
                    paw: (-27.8 - 2.2 * reach, 7.6),
                    claws_out: true, overlay: false,
                };
            } else {
                limbs[0] = LimbDesc {
                    is_front: true, side: -1.0,
                    hip: (7.0, -8.0), elbow: (13.0, -13.6), paw: (20.0, -12.4),
                    claws_out: true, overlay: false,
                };
                limbs[1] = LimbDesc {
                    is_front: true, side: 1.0,
                    hip: (7.0, 8.0), elbow: (13.0, 13.6), paw: (20.0, 12.4),
                    claws_out: true, overlay: false,
                };
                limbs[2] = LimbDesc {
                    is_front: false, side: -1.0,
                    hip: (-14.0, -7.6), elbow: (-17.5, -12.0), paw: (-20.0, -11.4),
                    claws_out: false, overlay: false,
                };
                limbs[3] = LimbDesc {
                    is_front: false, side: 1.0,
                    hip: (-14.0, 7.6), elbow: (-17.5, 12.0), paw: (-20.0, 11.4),
                    claws_out: false, overlay: false,
                };
            }
        }
        BossCatState::SwipeWindup => {
            let c = pose.progress.powf(0.75);
            let s = pose.swipe_side;
            let strike_idx = if s < 0.0 { 0 } else { 1 };
            let brace_idx = 1 - strike_idx;
            limbs[strike_idx] = LimbDesc {
                is_front: true, side: s,
                hip: (5.5, s * 7.8),
                elbow: (6.5, s * (12.8 + 2.8 * c)),
                paw: (11.5 - 1.5 * c, s * (12.0 + 7.4 * c)),
                claws_out: true, overlay: true,
            };
            limbs[brace_idx] = LimbDesc {
                is_front: true, side: -s,
                hip: (6.5, -s * 7.4),
                elbow: (11.0, -s * 11.2),
                paw: (15.8, -s * 10.6),
                claws_out: claws_always, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.2), elbow: (-16.8, -10.8), paw: (-19.2, -10.6),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.2), elbow: (-16.8, 10.8), paw: (-19.2, 10.6),
                claws_out: false, overlay: false,
            };
        }
        BossCatState::Swipe => {
            let s = pose.swipe_side;
            let strike_idx = if s < 0.0 { 0 } else { 1 };
            let brace_idx = 1 - strike_idx;
            let arc_t = if pose.progress < SWIPE_ACTIVE_END {
                let t = pose.progress / SWIPE_ACTIVE_END;
                1.0 - (1.0 - t).powi(3)
            } else {
                1.0 - ((pose.progress - SWIPE_ACTIVE_END) / (1.0 - SWIPE_ACTIVE_END)) * 0.25
            };
            // Sweep muscular foreleg smoothly around the front of the snout (never over the eyes!).
            let ang = s * (1.22 - 1.88 * arc_t);
            let paw_u = 18.0 + ang.cos() * 14.5;
            let paw_v = ang.sin() * 16.5;
            // Curved muscular elbow arc following the strike smoothly.
            let elbow_u = 13.8 + 6.4 * arc_t;
            let elbow_v = s * (13.2 - 3.2 * arc_t);
            limbs[strike_idx] = LimbDesc {
                is_front: true, side: s,
                hip: (6.5, s * 7.6),
                elbow: (elbow_u, elbow_v),
                paw: (paw_u, paw_v),
                claws_out: true, overlay: true,
            };
            limbs[brace_idx] = LimbDesc {
                is_front: true, side: -s,
                hip: (5.5, -s * 7.4),
                elbow: (9.5, -s * 11.4),
                paw: (13.8, -s * 11.4),
                claws_out: claws_always, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.2), elbow: (-17.0, -10.8), paw: (-19.5, -10.8),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.2), elbow: (-17.0, 10.8), paw: (-19.5, 10.8),
                claws_out: false, overlay: false,
            };
        }
        BossCatState::Roar => {
            limbs[0] = LimbDesc {
                is_front: true, side: -1.0,
                hip: (6.0, -8.0), elbow: (11.5, -13.8), paw: (17.2, -13.4),
                claws_out: true, overlay: false,
            };
            limbs[1] = LimbDesc {
                is_front: true, side: 1.0,
                hip: (6.0, 8.0), elbow: (11.5, 13.8), paw: (17.2, 13.4),
                claws_out: true, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.6), elbow: (-18.0, -12.6), paw: (-21.2, -12.4),
                claws_out: true, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.6), elbow: (-18.0, 12.6), paw: (-21.2, 12.4),
                claws_out: true, overlay: false,
            };
        }
        BossCatState::Hurt => {
            limbs[0] = LimbDesc {
                is_front: true, side: -1.0,
                hip: (5.5, -7.5), elbow: (10.5, -12.6), paw: (15.5, -12.4),
                claws_out: true, overlay: false,
            };
            limbs[1] = LimbDesc {
                is_front: true, side: 1.0,
                hip: (5.5, 7.5), elbow: (9.5, 12.2), paw: (13.8, 12.2),
                claws_out: true, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.0), elbow: (-18.0, -11.4), paw: (-21.5, -11.4),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.0), elbow: (-17.5, 11.4), paw: (-20.5, 11.4),
                claws_out: false, overlay: false,
            };
        }
        BossCatState::Death => {
            let splay = ((pose.death - 0.32) / 0.45).clamp(0.0, 1.0);
            limbs[0] = LimbDesc {
                is_front: true, side: -1.0,
                hip: (5.5, -7.5), elbow: (9.5, -13.2 - 2.5 * splay), paw: (12.0 - 3.0 * splay, -12.0 - 5.2 * splay),
                claws_out: false, overlay: false,
            };
            limbs[1] = LimbDesc {
                is_front: true, side: 1.0,
                hip: (5.5, 7.5), elbow: (11.5, 12.8 + 2.2 * splay), paw: (16.5 + 1.5 * splay, 11.2 + 4.4 * splay),
                claws_out: false, overlay: false,
            };
            limbs[2] = LimbDesc {
                is_front: false, side: -1.0,
                hip: (-14.0, -7.0), elbow: (-17.0, -11.8 - 2.2 * splay), paw: (-19.0 - 2.5 * splay, -11.5 - 4.0 * splay),
                claws_out: false, overlay: false,
            };
            limbs[3] = LimbDesc {
                is_front: false, side: 1.0,
                hip: (-14.0, 7.0), elbow: (-17.5, 11.2 + 1.8 * splay), paw: (-20.5 - 1.5 * splay, 10.5 + 3.2 * splay),
                claws_out: false, overlay: false,
            };
        }
    }

    for limb in &limbs {
        if limb.overlay != overlay {
            continue;
        }
        draw_single_limb(pb, pose, &pal, limb);
    }
}

fn draw_single_limb(pb: &mut PixelBuffer, pose: &Pose, pal: &CoatPalette, limb: &LimbDesc) {
    let p0 = pose.place(limb.hip.0, limb.hip.1);
    let pm = pose.place(limb.elbow.0, limb.elbow.1);
    let p1 = pose.place(limb.paw.0, limb.paw.1);

    let min_x = (p0.x.min(pm.x).min(p1.x) - 7.0).floor() as i32;
    let max_x = (p0.x.max(pm.x).max(p1.x) + 7.0).ceil() as i32;
    let min_y = (p0.y.min(pm.y).min(p1.y) - 7.0).floor() as i32;
    let max_y = (p0.y.max(pm.y).max(p1.y) + 7.0).ceil() as i32;
    let x0 = min_x.max(0);
    let x1 = max_x.min(pb.w as i32 - 1);
    let y0 = min_y.max(0);
    let y1 = max_y.min(pb.h as i32 - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let r_hip = if limb.is_front { 4.0 } else { 3.8 };
    let r_elb = if limb.is_front { 2.8 } else { 2.5 };
    let r_wri = if limb.is_front { 2.2 } else { 1.9 };
    let r_paw = if limb.is_front { 3.2 } else { 2.7 };
    let a = pose.alpha;

    // Axis directions for clean cylindrical top-left lighting along the forearm & thigh!
    let dir0 = (pm - p0).normalized();
    let perp0 = dir0.perp();
    let dir1 = (p1 - pm).normalized();
    let perp1 = dir1.perp();

    for y in y0..=y1 {
        for x in x0..=x1 {
            let fx = x as f32;
            let fy = y as f32;
            let d_upper = sd_taper(fx, fy, (p0.x, p0.y), (pm.x, pm.y), r_hip, r_elb);
            let d_lower = sd_taper(fx, fy, (pm.x, pm.y), (p1.x, p1.y), r_elb, r_wri);
            let d_arm = smin(d_upper, d_lower, 2.2);
            let d_paw = ((fx - p1.x).powi(2) + (fy - p1.y).powi(2)).sqrt() - r_paw;
            let d = smin(d_arm, d_paw, 1.1);
            if d > 1.2 {
                continue;
            }
            let cover = (0.48 - d * 1.15).clamp(0.0, 1.0);
            if cover <= 0.0 {
                blend_px(pb, x, y, pal.rim, 0.68 * a);
                continue;
            }
            let bg = pb.pixel_at(x as usize, y as usize);
            let out_base = if cover < 1.0 {
                mix(bg, pal.rim, 0.68 * (1.0 - cover) * a)
            } else {
                bg
            };

            // Compute cylindrical normal from whichever segment is closest.
            let (nx, ny) = if d_paw < d_arm {
                (((fx - p1.x) / r_paw).clamp(-1.0, 1.0), ((fy - p1.y) / r_paw).clamp(-1.0, 1.0))
            } else if d_upper < d_lower {
                let s_off = ((fx - p0.x) * perp0.x + (fy - p0.y) * perp0.y) / r_hip;
                let s_c = s_off.clamp(-1.0, 1.0);
                (perp0.x * s_c, perp0.y * s_c)
            } else {
                let s_off = ((fx - pm.x) * perp1.x + (fy - pm.y) * perp1.y) / r_elb;
                let s_c = s_off.clamp(-1.0, 1.0);
                (perp1.x * s_c, perp1.y * s_c)
            };
            let lit = nx * LIGHT.x + ny * LIGHT.y;
            let dome = (1.0 - (nx * nx + ny * ny).min(1.0) * 0.55).clamp(0.0, 1.0);
            let lum = 0.42 + 0.30 * dome + 0.54 * lit;
            let mut col = shade3(pal.shadow, pal.mid, pal.hi, lum);

            if d_paw < -0.15 {
                col = mix(col, pal.cream, 0.36);
            } else {
                // Crisp dark jaguar leg spots along the upper thigh/forearm!
                let along = ((fx - p0.x) * (p1.x - p0.x) + (fy - p0.y) * (p1.y - p0.y))
                    / ((p1 - p0).len_sq().max(1.0));
                let cross = (fx - pm.x) * perp1.x + (fy - pm.y) * perp1.y;
                if (along * 6.0).sin() > 0.42 && (cross * 0.9).cos() > 0.15 {
                    col = mix(col, pal.spot, 0.72);
                }
            }

            let final_col = apply_hit(col, pose.hit);
            pb.plot(x, y, mix(out_base, final_col, cover * a));
        }
    }

    // Gleaming ivory-white jaguar claws (3 fanned curved tips!).
    if limb.claws_out && limb.is_front {
        let dir = if dir1.len_sq() > 0.1 {
            (dir1 * 0.55 + pose.f * 0.45).normalized()
        } else {
            pose.f
        };
        let perp = dir.perp();
        let claw_len = if matches!(pose.state, BossCatState::Swipe | BossCatState::SwipeWindup) {
            4.5
        } else {
            3.3
        };
        let claw_col = apply_hit((252, 248, 236), pose.hit * 0.35);
        for &offset in &[-1.7f32, 0.0, 1.7] {
            let base = p1 + dir * (r_paw * 0.65) + perp * offset;
            let tip = base + (dir + perp * (offset * 0.20 + limb.side * 0.10)).normalized() * claw_len;
            draw_claw_needle(pb, base, tip, claw_col, pal.rim, a);
        }
    }
}

fn draw_claw_needle(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, rim: Rgb, alpha: f32) {
    let steps = (a.dist(b) * 2.0).ceil().max(2.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let q = a + (b - a) * t;
        let x = q.x.round() as i32;
        let y = q.y.round() as i32;
        blend_px(pb, x + 1, y + 1, rim, 0.58 * alpha);
        blend_px(pb, x, y, mix(col, (255, 255, 255), t * 0.5), alpha);
    }
}

/// Sculpted muscular jaguar body, rolling shoulders, skull, ears, rosettes & Phase 2 scars.
fn draw_body(pb: &mut PixelBuffer, pose: &Pose) {
    let reach = 35.0 * pose.su.max(pose.sv);
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, pose.body_pos, reach) else {
        return;
    };

    let pal = coat_palette(pose);
    let a = pose.alpha;
    let scar_col = mix((192, 34, 48), (255, 118, 72), pose.critical_pulse * 0.68);
    let scar_edge = mix((235, 165, 138), (255, 224, 168), pose.critical_pulse * 0.5);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            if !(-26.0..=32.0).contains(&u) || v.abs() > 16.5 {
                continue;
            }
            let (d, part) = body_sdf(pose, u, v);
            let cover = (0.52 - d * 1.15).clamp(0.0, 1.0);

            if cover <= 0.0 {
                if d < 1.38 {
                    let rim_a = ((1.38 - d) / 0.88).clamp(0.0, 1.0) * 0.92 * a;
                    blend_px(pb, x, y, pal.rim, rim_a);
                }
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = if cover < 1.0 {
                mix(bg, pal.rim, 0.92 * (1.0 - cover) * a)
            } else {
                bg
            };

            let vs = v - pose.spine_v(u);
            let col = match part {
                2 => {
                    // Rounded dark-backed jaguar ear with unmistakable pale-cream ocellus spot!
                    let eu_base = 15.8 - 1.9 * pose.ear_pin;
                    let ev_base = 6.3 - 0.9 * pose.ear_pin;
                    let eu_tip = 14.3 - 3.9 * pose.ear_pin;
                    let ev_tip = 10.1 - 2.3 * pose.ear_pin;
                    let oc_u = eu_base * 0.42 + eu_tip * 0.58 - 0.35;
                    let oc_v = ev_base * 0.42 + ev_tip * 0.58 + 0.15;
                    let d_oc = ((u - oc_u).powi(2) + (vs.abs() - oc_v).powi(2)).sqrt();
                    if d_oc < 1.55 {
                        pal.cream
                    } else if u > eu_base + 0.55 && vs.abs() > ev_base + 0.3 && vs.abs() < ev_tip - 0.5 {
                        pal.ear_inner
                    } else {
                        let lit = pose.lit(0.15, vs.signum() * 0.65);
                        shade3((16, 10, 10), pal.ear_back, (66, 46, 32), 0.45 + 0.45 * lit)
                    }
                }
                3 => {
                    // Jagged bristled hackle spikes: ember-lit base tapering to dark tips!
                    let lit = pose.lit(0.12, vs.signum() * 0.82);
                    let base = shade3(pal.shadow, pal.mid, pal.hi, 0.48 + 0.52 * lit);
                    let tip_t = ((vs.abs() - 9.0) / 3.8).clamp(0.0, 0.82);
                    mix(base, pal.spot, tip_t)
                }
                _ => {
                    let (cu, ru, rv) = if part == 1 {
                        (19.6, 7.6, 8.6)
                    } else if u > -3.5 {
                        (4.5, 11.0, 10.6)
                    } else {
                        (-14.5, 9.2, 9.5)
                    };
                    let (dnu, dnv, bump_lift) = anatomy_bump(pose, u, vs);
                    let nu = ((u - cu) / ru + dnu).clamp(-1.0, 1.0);
                    let nv = (vs / rv + dnv).clamp(-1.0, 1.0);
                    let dome = (1.0 - (nu * nu * 0.42 + nv * nv * 0.78)).clamp(0.0, 1.0);
                    let lit = pose.lit(nu, nv);
                    let mut lum = 0.38 + 0.36 * dome + 0.60 * lit + bump_lift * (0.58 + 0.42 * lit);
                    if lit > 0.34 && dome > 0.28 {
                        lum += 0.26;
                    }
                    let mut fur = shade3(pal.shadow, pal.mid, pal.hi, lum);

                    // Pale cream muzzle pads & flaring cheek ruffs.
                    if part == 1 {
                        if u > 23.0 && vs.abs() < 4.4 {
                            let snout_t = ((u - 23.0) / 3.6).clamp(0.0, 1.0);
                            fur = mix(fur, pal.cream, 0.48 * snout_t);
                        } else if u > 15.5 && u < 20.4 && vs.abs() > 6.6 {
                            fur = mix(fur, pal.cream, 0.36);
                        }
                    }

                    // Organic jaguar rosettes & spots!
                    let (spot_w, warm_w) = jaguar_pattern(u, vs);
                    if warm_w > 0.0 {
                        let core_col = shade3(scale(pal.cinnamon, 0.55), pal.cinnamon, pal.mid, lum);
                        fur = mix(fur, core_col, warm_w * 0.72);
                    }
                    if spot_w > 0.0 {
                        let spot_lit = mix(
                            pal.spot,
                            scale(pal.shadow, 0.65),
                            (lit * 0.5 + 0.5).clamp(0.0, 1.0) * 0.34,
                        );
                        fur = mix(fur, spot_lit, spot_w);
                    }

                    // Neck-shoulder sculptural shadow crease.
                    if (u - 13.5).abs() < 1.25 && vs.abs() < 6.4 {
                        fur = scale(fur, 0.85);
                    }

                    // Phase 2 battle scars (crimson core + subtle pale scar lip for 3-theme pop).
                    if pose.enraged > 0.5 {
                        let d_s1 = ((u - 5.5) - (vs + 4.2) * 0.85).abs();
                        let s1_in = u > 1.2 && u < 10.2 && vs < -0.4;
                        let d_s2 = ((u + 13.5) + (vs - 3.6) * 0.78).abs();
                        let s2_in = u > -18.2 && u < -9.2 && vs > 0.4;
                        let d_s3 = ((u - 20.5) - (vs + 2.8) * 1.1).abs();
                        let s3_in = pose.critical_pulse > 0.01 && u > 18.0 && u < 23.2;

                        if (s1_in && d_s1 < 0.62) || (s2_in && d_s2 < 0.58) || (s3_in && d_s3 < 0.48) {
                            fur = mix(fur, scar_col, 0.90);
                        } else if (s1_in && d_s1 < 1.05) || (s2_in && d_s2 < 1.0) {
                            fur = mix(fur, scar_edge, 0.35);
                        }
                    }

                    fur
                }
            };

            out = mix(out, apply_hit(col, pose.hit), cover * a);
            pb.plot(x, y, out);
        }
    }
}

/// Wide-open roaring / hissing predator jaws protruding cleanly beneath the
/// upper snout bridge, showing crimson maw, tongue & 4 sharp ivory saber fangs.
fn draw_jaws(pb: &mut PixelBuffer, pose: &Pose) {
    let open = pose.jaw_open;
    let maw_u = 25.8 + 2.3 * open;
    let maw_ru = 1.9 + 2.2 * open;
    let maw_rv = 2.3 + 1.55 * open;
    let center = pose.place(maw_u, 0.0);
    let Some((x0, x1, y0, y1)) = clamp_bbox(pb, center, 8.5) else {
        return;
    };

    let a = pose.alpha;
    let maw_dark = (58, 8, 16);
    let maw_mid = (164, 24, 42);
    let tongue = (228, 76, 94);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = pose.local(x, y);
            let vs = v - pose.spine_v(u);
            // Let the upper nose bridge cleanly overhang the back edge of the jaws!
            if u < 24.8 && vs.abs() < 2.2 {
                continue;
            }
            let d = sd_ellipse(u, vs, maw_u, 0.0, maw_ru, maw_rv);
            if d > 0.85 {
                continue;
            }
            if d > 0.0 {
                blend_px(pb, x, y, (18, 8, 10), 0.84 * a);
            } else {
                let inner = (u - maw_u) / maw_ru.max(0.5);
                let col = if inner < -0.10 && vs.abs() < maw_rv * 0.48 {
                    tongue
                } else {
                    mix(maw_dark, maw_mid, (0.5 + 0.5 * inner).clamp(0.0, 1.0))
                };
                blend_px(pb, x, y, apply_hit(col, pose.hit * 0.35), a);
            }
        }
    }

    // Four prominent 2-pixel tapered ivory saber fangs framing the roaring maw!
    let fang_hi = (255, 254, 246);
    let fang_mid = (232, 224, 206);
    for side in [-1.0f32, 1.0] {
        // Upper saber canines (pointing inward-forward from upper jaw rim).
        let u0 = pose.px(maw_u + maw_ru * 0.62, side * (maw_rv * 0.66));
        let u1 = pose.px(maw_u + maw_ru * 0.25, side * (maw_rv * 0.42));
        blend_px(pb, u0.0, u0.1, fang_hi, a);
        blend_px(pb, u1.0, u1.1, fang_mid, a * 0.96);
        // Lower canines (sharp inner teeth visible when roaring wide).
        if open > 0.42 {
            let l0 = pose.px(maw_u - maw_ru * 0.35, side * (maw_rv * 0.64));
            let l1 = pose.px(maw_u - maw_ru * 0.12, side * (maw_rv * 0.44));
            blend_px(pb, l0.0, l0.1, fang_hi, a * 0.95);
            blend_px(pb, l1.0, l1.1, fang_mid, a * 0.90);
        }
    }
}

/// Fierce angled almond predator eyes tracking `view.target`, dark-rose jaguar
/// nose leather, and crisp cheek whiskers.
fn draw_face(pb: &mut PixelBuffer, pose: &Pose) {
    let a = pose.alpha;
    let dead_closed = pose.state == BossCatState::Death && pose.death > 0.40;
    let squinted = pose.state == BossCatState::Hurt;

    let iris_col = if pose.hit > 0.72 {
        (36, 20, 14)
    } else if pose.enraged > 0.5 {
        mix((255, 86, 28), (255, 226, 98), 0.34 + 0.46 * pose.critical_pulse)
    } else {
        (88, 252, 134)
    };
    let iris_hi = if pose.enraged > 0.5 {
        (255, 248, 186)
    } else {
        (216, 255, 186)
    };
    let pupil_col = (12, 8, 10);

    for side in [-1.0f32, 1.0] {
        let eu = 21.6;
        let ev = side * 3.35;
        let eye_world = pose.place(eu, ev);

        if dead_closed {
            let p0 = pose.px(eu - 0.6, ev - side * 1.15);
            let p1 = pose.px(eu + 0.4, ev + side * 1.15);
            line_blend(pb, p0, p1, (22, 14, 14), 0.88 * a);
            continue;
        }

        if squinted {
            let p0 = pose.px(eu - 0.8, ev - side * 1.1);
            let p1 = pose.px(eu + 0.8, ev + side * 0.9);
            line_blend(pb, p0, p1, (20, 10, 10), 0.95 * a);
            let glint = pose.px(eu + 0.1, ev);
            blend_px(pb, glint.0, glint.1, iris_col, 0.72 * a);
            continue;
        }

        // Compute normalized direction from this eye to `view.target` in local `(u, v)`.
        let to_t = pose.target_screen - eye_world;
        let (look_u, look_v) = if to_t.len_sq() > 1.0 {
            let d = to_t.normalized();
            (
                (d.dot(pose.f) * 0.72).clamp(-0.62, 0.72),
                (d.dot(pose.p) * 0.85).clamp(-0.85, 0.85),
            )
        } else {
            (0.42, 0.0)
        };

        // Sleek steeply-canted fierce predator almond eyes (narrower & meaner than Iter 1!).
        let Some((x0, x1, y0, y1)) = clamp_bbox(pb, eye_world, 3.0) else {
            continue;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (lu, lv) = pose.local(x, y);
                let lvs = lv - pose.spine_v(lu);
                let du = lu - eu;
                let dv = lvs - ev;
                // Steep predatory slant: outer corner swept back, inner corner forward!
                let cu = du * 0.84 + dv * side * 0.54;
                let cv = -du * side * 0.54 + dv * 0.84;
                let d_sock = sd_ellipse(cu, cv, 0.0, 0.0, 0.96, 1.55);
                if d_sock > 0.65 {
                    continue;
                }
                if d_sock > -0.14 {
                    // Heavy dark feline eyeliner & scowling brow shadow.
                    blend_px(pb, x, y, (16, 9, 9), 0.92 * a);
                } else {
                    let pu = du - look_u * 0.60;
                    let pv = dv - look_v * 0.70;
                    let in_pupil = pu.abs() < 0.72 && pv.abs() < 0.36;
                    let col = if in_pupil {
                        pupil_col
                    } else if pu > 0.18 && pv * side < 0.05 {
                        iris_hi
                    } else {
                        iris_col
                    };
                    blend_px(pb, x, y, col, a);
                }
            }
        }

        // Phase 2 molten eye-gleam halo!
        if pose.enraged > 0.5 && pose.hit < 0.5 {
            let ep = pose.px(eu + 0.35, ev);
            let glow = scale((255, 108, 36), (0.42 + 0.30 * pose.critical_pulse) * a);
            pb.plot_add(ep.0, ep.1, glow);
            pb.plot_add(ep.0 + 1, ep.1, scale(glow, 0.45));
            pb.plot_add(ep.0 - 1, ep.1, scale(glow, 0.45));
            pb.plot_add(ep.0, ep.1 + 1, scale(glow, 0.45));
            pb.plot_add(ep.0, ep.1 - 1, scale(glow, 0.45));
        }
    }

    // Broad triangular jaguar nose leather at snout tip (unless jaws are wide open).
    if pose.jaw_open < 0.65 {
        let nose_u = 26.2;
        let nose_c = pose.place(nose_u, 0.0);
        if let Some((x0, x1, y0, y1)) = clamp_bbox(pb, nose_c, 2.5) {
            let nose_col = apply_hit((68, 30, 34), pose.hit);
            let nose_hi = apply_hit((172, 92, 92), pose.hit);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (lu, lv) = pose.local(x, y);
                    let lvs = lv - pose.spine_v(lu);
                    let d = sd_ellipse(lu, lvs, nose_u, 0.0, 1.15, 1.75);
                    if d <= 0.1 {
                        let c = if lvs < 0.0 { nose_hi } else { nose_col };
                        blend_px(pb, x, y, c, a);
                    }
                }
            }
        }
    }

    // Crisp pale feline muzzle whiskers fanning from both whisker pads.
    let twitch = 0.42 * (pose.anim * 7.0).sin();
    for side in [-1.0f32, 1.0] {
        for &(wu, wv, g) in &[
            (24.6f32, 5.0f32, 82u8),
            (25.2, 7.0, 56),
            (23.5, 5.5, 76),
            (23.2, 7.8, 44),
            (25.8, 4.8, 72),
            (26.6, 6.6, 48),
        ] {
            let q = pose.px(wu + twitch * 0.25, side * (wv + twitch * 0.2));
            let gv = (g as f32 * a) as u8;
            pb.plot_add(q.0, q.1, (gv, gv, (gv as f32 * 0.88) as u8));
        }
    }
}

/// Subtle kinetic wiggle arcs beside the crouching jaguar's hindquarters,
/// telegraphing the accelerating pre-pounce butt-wiggle clearly at low res!
fn draw_crouch_wiggle_arcs(pb: &mut PixelBuffer, pose: &Pose) {
    let locked = pose.progress >= CROUCH_LOCK_PROGRESS;
    let col = if locked {
        (255, 198, 96)
    } else {
        (246, 214, 146)
    };
    let strength = (0.35 + 0.55 * pose.progress).clamp(0.0, 0.92);
    // Draw twin tiny curved motion brackets outside the wiggling haunches.
    for side in [-1.0f32, 1.0] {
        for &(du_arc, v_off, a_mul) in &[(0.0f32, 12.6f32, 0.85f32), (-0.5, 15.0, 0.45)] {
            let p0 = pose.px(-18.5 + du_arc, side * (v_off - 1.2));
            let p1 = pose.px(-14.5 + du_arc, side * v_off);
            let p2 = pose.px(-10.5 + du_arc, side * (v_off - 1.2));
            line_blend(pb, p0, p1, col, strength * a_mul);
            line_blend(pb, p1, p2, col, strength * a_mul);
        }
    }
}

/// Three bold, tapered crescent claw-slash ribbons tearing across the front `130°` arc!
fn draw_swipe_claw_arcs(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.progress > 0.78 {
        return;
    }
    let origin = pose.body_pos + pose.f * SWIPE_ORIGIN_U;
    let s = pose.swipe_side;

    let head_t = (pose.progress / SWIPE_ACTIVE_END).clamp(0.0, 1.0);
    let head_eased = 1.0 - (1.0 - head_t).powi(3);
    let tail_t = ((pose.progress - 0.10) / 0.64).clamp(0.0, 1.0);
    if head_eased <= tail_t + 0.02 {
        return;
    }

    let a_start = s * SWIPE_HALF_ANGLE;
    let a_end = -s * SWIPE_HALF_ANGLE;
    let base_ang = pose.f.angle();
    let fade = (1.0 - tail_t).clamp(0.0, 1.0);

    // Three wide sweeping tapered slash crescents across radii 17..30 px!
    let radii = [17.5f32, 23.8, 29.8];
    let steps = 48usize;

    for (k, &rad) in radii.iter().enumerate() {
        let lead_shift = if k == 1 { 0.05 } else { 0.0 };
        let t0 = (tail_t + lead_shift * 0.4).clamp(0.0, 1.0);
        let t1 = (head_eased + lead_shift).clamp(0.0, 1.0);

        for i in 0..=steps {
            let frac = i as f32 / steps as f32;
            let t = t0 + (t1 - t0) * frac;
            let ang = base_ang + a_start + (a_end - a_start) * t;
            let r_here = rad + 1.8 * (t * PI).sin();
            let dir = Vec2::from_angle(ang);
            let q = origin + dir * r_here;

            // Crescent thickness swells to 2.2 px near the cutting tip, tapers to 0.5 px at tail.
            let thick = (0.4 + 1.9 * frac.powf(0.75) * (1.0 - (frac - 0.88).max(0.0) * 5.0)).max(0.4);
            let col = if frac > 0.72 {
                (255, 254, 240)
            } else if frac > 0.36 {
                if pose.enraged > 0.5 {
                    (255, 134, 48)
                } else {
                    (255, 214, 112)
                }
            } else if pose.enraged > 0.5 {
                (238, 46, 38)
            } else {
                (230, 114, 46)
            };
            let strength = (0.28 + 0.72 * frac) * fade;

            // Paint radial thickness across the ribbon so it reads bold & razor-sharp!
            let half_i = thick.ceil() as i32;
            for dr in -half_i..=half_i {
                let d_abs = (dr as f32).abs();
                if d_abs > thick {
                    continue;
                }
                let p_rad = q + dir * (dr as f32 * 0.75);
                let px = p_rad.x.round() as i32;
                let py = p_rad.y.round() as i32;
                let w = 1.0 - d_abs / (thick + 0.35);
                let c_here = if d_abs < 0.6 && frac > 0.58 {
                    (255, 255, 246)
                } else {
                    col
                };
                blend_px(pb, px, py, c_here, strength * w);
                if frac > 0.55 && d_abs < 0.8 {
                    pb.plot_add(px, py, scale(col, 0.36 * strength));
                }
            }
        }
    }
}

/// Thunderous concentric expanding acoustic rings, radial roar streaks & Phase 2 sonic fangs!
fn draw_roar_waves(pb: &mut PixelBuffer, pose: &Pose) {
    let origin = pose.body_pos + pose.f * ROAR_ORIGIN_U;
    let p = pose.progress;

    if p < ROAR_BLAST_START {
        let t = p / ROAR_BLAST_START;
        let streak_col = mix((255, 214, 126), (255, 118, 68), pose.enraged);
        for k in 0..8 {
            let ang = k as f32 / 8.0 * TAU + pose.anim * 0.8;
            let r_outer = 19.0 * (1.0 - t * 0.72) + 4.0;
            let r_inner = (r_outer - 6.0).max(2.0);
            let p0 = origin + Vec2::from_angle(ang) * r_outer;
            let p1 = origin + Vec2::from_angle(ang) * r_inner;
            line_blend(
                pb,
                (p0.x.round() as i32, p0.y.round() as i32),
                (p1.x.round() as i32, p1.y.round() as i32),
                streak_col,
                0.62 * t,
            );
        }
        return;
    }

    if p > ROAR_BLAST_END + 0.08 {
        return;
    }

    let blast_t = ((p - ROAR_BLAST_START) / (ROAR_BLAST_END - ROAR_BLAST_START)).clamp(0.0, 1.0);

    // Directional shock-burst rays blasting outward from the roaring jaws!
    let ray_col = if pose.enraged > 0.5 {
        (255, 138, 64)
    } else {
        (255, 226, 138)
    };
    for k in 0..12 {
        let a = k as f32 / 12.0 * TAU + 0.18;
        let r0 = 7.0 + blast_t * 14.0;
        let r1 = r0 + 7.0 + 4.0 * (1.0 - blast_t);
        let p0 = origin + Vec2::from_angle(a) * r0;
        let p1 = origin + Vec2::from_angle(a) * r1;
        line_blend(
            pb,
            (p0.x.round() as i32, p0.y.round() as i32),
            (p1.x.round() as i32, p1.y.round() as i32),
            ray_col,
            0.38 * (1.0 - blast_t * 0.55),
        );
    }

    // Three bold concentric acoustic pressure rings (2 px luminous band!).
    for wave_idx in 0..3 {
        let local_t = (blast_t * 1.35 - wave_idx as f32 * 0.26).clamp(0.0, 1.0);
        if local_t <= 0.01 || local_t >= 0.99 {
            continue;
        }
        let rad = 5.5 + local_t * (ROAR_OUTER_RADIUS - 5.5);
        let fade = (1.0 - local_t).powf(0.6);
        let base_col = if pose.enraged > 0.5 {
            mix((255, 248, 200), (255, 76, 42), local_t)
        } else {
            mix((255, 252, 224), (242, 168, 56), local_t)
        };
        let steps = (rad * 7.5).round().max(24.0) as i32;
        for i in 0..steps {
            let a = i as f32 / steps as f32 * TAU;
            let ripple = 1.0 + 0.045 * (a * 8.0 + pose.anim * 18.0).sin();
            let dir = Vec2::from_angle(a);
            let q = origin + dir * (rad * ripple);
            let x = q.x.round() as i32;
            let y = q.y.round() as i32;
            blend_px(pb, x, y, base_col, 0.92 * fade);
            // Second outer rim pixel for crisp 2 px shockwave thickness.
            let q2 = origin + dir * (rad * ripple + 1.0);
            blend_px(pb, q2.x.round() as i32, q2.y.round() as i32, scale(base_col, 0.8), 0.55 * fade);
            if local_t < 0.52 {
                pb.plot_add(x, y, scale(base_col, 0.36 * fade));
            }
        }
    }

    // Phase 2 radial sonic fang pips (8 directions telegraphing radial projectiles).
    if pose.enraged > 0.5 {
        let pip_r = 11.0 + blast_t * 27.0;
        let pip_col = mix((255, 250, 220), (255, 86, 48), blast_t * 0.6);
        for k in 0..8 {
            let a = k as f32 / 8.0 * TAU + FRAC_PI_2 * 0.25;
            let q = origin + Vec2::from_angle(a) * pip_r;
            let x = q.x.round() as i32;
            let y = q.y.round() as i32;
            blend_px(pb, x, y, (255, 255, 240), 0.96 * (1.0 - blast_t * 0.4));
            pb.plot_add(x + 1, y, scale(pip_col, 0.55));
            pb.plot_add(x - 1, y, scale(pip_col, 0.55));
            pb.plot_add(x, y + 1, scale(pip_col, 0.55));
            pb.plot_add(x, y - 1, scale(pip_col, 0.55));
        }
    }
}

/// Crisp stagger-shock radial ticks around the recoiling jaguar skull during `Hurt`.
fn draw_hurt_impact_ticks(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.progress > 0.65 {
        return;
    }
    let fade = 1.0 - pose.progress / 0.65;
    let head = pose.place(20.0, 0.0);
    let col = (255, 232, 162);
    for k in 0..5 {
        let a = -1.1 + k as f32 * 0.55 + pose.f.angle();
        let p0 = head + Vec2::from_angle(a) * (9.5 + pose.progress * 3.0);
        let p1 = head + Vec2::from_angle(a) * (13.5 + pose.progress * 5.0);
        line_blend(
            pb,
            (p0.x.round() as i32, p0.y.round() as i32),
            (p1.x.round() as i32, p1.y.round() as i32),
            col,
            0.82 * fade,
        );
    }
}

/// Rising golden-amber embers during the boss's defeat collapse.
fn draw_death_embers(pb: &mut PixelBuffer, pose: &Pose) {
    if pose.death < 0.22 {
        return;
    }
    let t = (pose.death - 0.22) / 0.78;
    for i in 0..16 {
        let fi = i as f32;
        let phase = (t * 1.4 + hash2(i, 7, 91) * 0.6).fract();
        let u = -18.0 + hash2(i, 13, 53) * 40.0;
        let v = -9.0 + hash2(i, 29, 71) * 18.0;
        let base = pose.place(u, v);
        let drift_x = (fi * 2.1 + pose.death * 8.0).sin() * 2.5;
        let rise_y = phase * 19.0;
        let x = (base.x + drift_x).round() as i32;
        let y = (base.y - rise_y).round() as i32;
        let col = mix((255, 224, 124), (235, 78, 42), phase);
        let fade = (1.0 - phase) * (1.0 - t * 0.35);
        pb.plot_add(x, y, scale(col, fade));
    }
}

fn line_blend(pb: &mut PixelBuffer, a: (i32, i32), b: (i32, i32), c: Rgb, alpha: f32) {
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = a;
    loop {
        blend_px(pb, x, y, c, alpha);
        if x == b.0 && y == b.1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

// ============================================================================
// Unit Tests & Multi-Theme Visual Pose Sheet
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor;
    use crate::game::art_rat;
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;

    const ALL_STATES: [BossCatState; 8] = [
        BossCatState::Stalk,
        BossCatState::Crouch,
        BossCatState::Pounce,
        BossCatState::SwipeWindup,
        BossCatState::Swipe,
        BossCatState::Roar,
        BossCatState::Hurt,
        BossCatState::Death,
    ];

    fn buffer_hash(pb: &PixelBuffer) -> u64 {
        let mut h = 0u64;
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                h = h
                    .wrapping_mul(31)
                    .wrapping_add(c.0 as u64 + (c.1 as u64) * 257 + (c.2 as u64) * 65537);
            }
        }
        h
    }

    #[test]
    fn draws_every_state_angle_progress_and_offscreen_without_panic() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (8, 8), (120, 90)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for &state in &ALL_STATES {
                for &progress in &[0.0f32, 0.25, 0.5, 0.78, 0.9, 1.0, -0.5, 1.5] {
                    for i in 0..6 {
                        let ang = i as f32 / 6.0 * TAU;
                        let facing = if i == 0 {
                            Vec2::ZERO
                        } else {
                            Vec2::from_angle(ang)
                        };
                        for &(x, y) in &[
                            (60.0, 45.0),
                            (0.0, 0.0),
                            (-90.0, -90.0),
                            (500.0, 500.0),
                            (119.5, 89.5),
                        ] {
                            let pos = Vec2::new(x, y);
                            let mut v = BossCatView::new(pos, facing, pos + Vec2::new(50.0, 12.0));
                            v.state = state;
                            v.progress = progress;
                            v.anim = progress * 3.7 + i as f32;
                            v.hit_flash = if i % 2 == 0 { 0.85 } else { 0.0 };
                            v.hp_frac = if i % 3 == 0 { 0.2 } else { 0.85 };
                            v.swipe_side = if i % 2 == 0 { 1.0 } else { -1.0 };
                            v.pounce_origin = pos - Vec2::new(40.0, 0.0);
                            draw_boss_cat_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
                            draw_boss_cat(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn phase_change_hit_flash_and_eye_tracking_change_pixels() {
        let render = |hp_frac: f32, hit_flash: f32, target: Vec2| {
            let mut pb = PixelBuffer::new(110, 90);
            pb.clear((14, 22, 20));
            let mut v = BossCatView::new(Vec2::new(55.0, 45.0), Vec2::new(1.0, 0.0), target);
            v.hp_frac = hp_frac;
            v.hit_flash = hit_flash;
            draw_boss_cat_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            draw_boss_cat(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            buffer_hash(&pb)
        };

        let p1 = render(0.95, 0.0, Vec2::new(95.0, 45.0));
        let p2 = render(0.35, 0.0, Vec2::new(95.0, 45.0));
        let flashed = render(0.95, 0.9, Vec2::new(95.0, 45.0));
        let track_up = render(0.95, 0.0, Vec2::new(75.0, 5.0));
        let track_down = render(0.95, 0.0, Vec2::new(75.0, 85.0));

        assert_ne!(p1, p2, "Phase 2 must visibly change the jaguar's look");
        assert_ne!(p1, flashed, "hit_flash must visibly brighten the jaguar");
        assert_ne!(track_up, track_down, "eyes must track the target point");
    }

    #[test]
    fn hit_circles_align_with_painted_body() {
        let bg = (14, 22, 20);
        let mut pb = PixelBuffer::new(120, 90);
        pb.clear(bg);
        let v = BossCatView::new(
            Vec2::new(60.0, 45.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(100.0, 45.0),
        );
        draw_boss_cat(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
        for (center, _r) in body_hit_circles(&v) {
            let c = pb.pixel_at(center.x.round() as usize, center.y.round() as usize);
            assert_ne!(c, bg, "hit circle center {center:?} should lie on painted body");
        }
    }

    #[test]
    fn every_state_renders_distinctly() {
        let mut hashes = Vec::new();
        for &state in &ALL_STATES {
            let mut pb = PixelBuffer::new(140, 120);
            pb.clear((14, 22, 20));
            let pos = Vec2::new(62.0, 62.0);
            let mut v = BossCatView::new(pos, Vec2::new(1.0, 0.0), pos + Vec2::new(42.0, 0.0));
            v.state = state;
            v.progress = 0.42;
            v.anim = 1.2;
            v.pounce_origin = pos - Vec2::new(26.0, 0.0);
            draw_boss_cat_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            draw_boss_cat(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            hashes.push((state, buffer_hash(&pb)));
        }
        for i in 0..hashes.len() {
            for j in (i + 1)..hashes.len() {
                assert_ne!(
                    hashes[i].1, hashes[j].1,
                    "{:?} and {:?} rendered identically",
                    hashes[i].0, hashes[j].0
                );
            }
        }
    }

    /// Multi-theme visual showcase sheet:
    /// `RATRUN_DUMP=boss_cat cargo test dump_boss_cat_sheet` writes `/tmp/ratrun_boss_cat.ppm`.
    #[test]
    fn dump_boss_cat_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("boss_cat") {
            return;
        }
        const W: usize = 590;
        const H: usize = 300;
        let mut out = format!("P6\n{} {}\n255\n", W, H * 3).into_bytes();

        for theme in Theme::all() {
            let level = Level::new(theme);
            let cam = Vec2::new(260.0, 240.0);
            let mut pb = PixelBuffer::new(W, H);
            art_floor::draw_floor(&mut pb, &level, cam, 0.0);
            let acc = level.palette.accent;

            let col_x = [60.0f32, 176.0, 294.0, 412.0, 530.0];
            let row_y = [50.0f32, 150.0, 248.0];

            let draw_cell = |pb: &mut PixelBuffer, view: &BossCatView| {
                draw_boss_cat_ground(pb, view, cam, acc);
                draw_boss_cat(pb, view, cam, acc);
            };

            // ---- Row 1: Stalk, Rolling Shoulders, Phase 2, Hurt, Hit Flash ----
            {
                let pos = cam + Vec2::new(col_x[0] - 8.0, row_y[0]);
                let rat_pos = pos + Vec2::new(36.0, 14.0);
                let mut v = BossCatView::new(pos, Vec2::from_angle(0.32), rat_pos);
                v.state = BossCatState::Stalk;
                v.anim = 0.42;
                draw_boss_cat_ground(&mut pb, &v, cam, acc);
                let mut rat = Player::new(rat_pos, WeaponKind::Gnaw);
                rat.face = (pos - rat_pos).normalized();
                art_rat::draw_rat(&mut pb, &rat, 0.4, cam);
                draw_boss_cat(&mut pb, &v, cam, acc);

                let pos = cam + Vec2::new(col_x[1], row_y[0]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(2.35), pos + Vec2::new(-32.0, 30.0));
                v.state = BossCatState::Stalk;
                v.anim = 1.3;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[2], row_y[0]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(-0.42), pos + Vec2::new(40.0, -18.0));
                v.state = BossCatState::Stalk;
                v.hp_frac = 0.36;
                v.anim = 0.85;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[3], row_y[0]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(0.55), pos + Vec2::new(35.0, 22.0));
                v.state = BossCatState::Hurt;
                v.progress = 0.32;
                v.hit_flash = 0.30;
                v.hp_frac = 0.62;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[4] - 6.0, row_y[0]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(0.1), pos + Vec2::new(40.0, 4.0));
                v.state = BossCatState::Stalk;
                v.hp_frac = 0.18;
                v.hit_flash = 0.85;
                v.anim = 2.1;
                draw_cell(&mut pb, &v);
            }

            // ---- Row 2: Pounce Sequence (Crouch -> Locked -> Launch -> Dive -> Slam) ----
            {
                let pos = cam + Vec2::new(col_x[0] - 16.0, row_y[1]);
                let tgt = pos + Vec2::new(42.0, 8.0);
                let mut v = BossCatView::new(pos, (tgt - pos).normalized(), tgt);
                v.state = BossCatState::Crouch;
                v.progress = 0.28;
                v.anim = 0.9;
                draw_boss_cat_ground(&mut pb, &v, cam, acc);
                let mut rat = Player::new(tgt + Vec2::new(4.0, -3.0), WeaponKind::Gnaw);
                rat.face = Vec2::new(-1.0, 0.2).normalized();
                art_rat::draw_rat(&mut pb, &rat, 0.9, cam);
                draw_boss_cat(&mut pb, &v, cam, acc);

                let pos = cam + Vec2::new(col_x[1] - 14.0, row_y[1]);
                let tgt = pos + Vec2::new(42.0, 8.0);
                let mut v = BossCatView::new(pos, (tgt - pos).normalized(), tgt);
                v.state = BossCatState::Crouch;
                v.progress = 0.88;
                v.anim = 1.6;
                draw_cell(&mut pb, &v);

                // Early airborne spring (compact shadow, high launch).
                let orig = cam + Vec2::new(col_x[2] - 24.0, row_y[1] + 6.0);
                let tgt = orig + Vec2::new(46.0, 0.0);
                let p = 0.24;
                let pos = orig + (tgt - orig) * (p / POUNCE_LAND_PROGRESS);
                let mut v = BossCatView::new(pos, (tgt - orig).normalized(), tgt);
                v.state = BossCatState::Pounce;
                v.progress = p;
                v.pounce_origin = orig;
                v.anim = 2.0;
                draw_cell(&mut pb, &v);

                // Diving approach (HUGE growing shadow directly over the landing zone!).
                let orig = cam + Vec2::new(col_x[3] - 24.0, row_y[1] + 6.0);
                let tgt = orig + Vec2::new(46.0, 0.0);
                let p = 0.64;
                let pos = orig + (tgt - orig) * (p / POUNCE_LAND_PROGRESS);
                let mut v = BossCatView::new(pos, (tgt - orig).normalized(), tgt);
                v.state = BossCatState::Pounce;
                v.progress = p;
                v.pounce_origin = orig;
                v.anim = 2.4;
                draw_cell(&mut pb, &v);

                // Phase 2 Slam Impact with fiery expanding shockwave.
                let orig = cam + Vec2::new(col_x[4] - 32.0, row_y[1] + 4.0);
                let tgt = orig + Vec2::new(30.0, 0.0);
                let mut v = BossCatView::new(tgt, (tgt - orig).normalized(), tgt);
                v.state = BossCatState::Pounce;
                v.progress = 0.88;
                v.pounce_origin = orig;
                v.hp_frac = 0.35;
                v.anim = 2.9;
                draw_cell(&mut pb, &v);
            }

            // ---- Row 3: SwipeWindup, Swipe P1, Swipe P2 Left, Roar P2, Death ----
            {
                let pos = cam + Vec2::new(col_x[0] - 8.0, row_y[2]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(-0.22), pos + Vec2::new(40.0, -8.0));
                v.state = BossCatState::SwipeWindup;
                v.progress = 0.78;
                v.swipe_side = 1.0;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[1] - 8.0, row_y[2]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(-0.22), pos + Vec2::new(40.0, -8.0));
                v.state = BossCatState::Swipe;
                v.progress = 0.30;
                v.swipe_side = 1.0;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[2] + 8.0, row_y[2]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(2.75), pos + Vec2::new(-38.0, 14.0));
                v.state = BossCatState::Swipe;
                v.progress = 0.34;
                v.swipe_side = -1.0;
                v.hp_frac = 0.38;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[3] - 10.0, row_y[2]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(0.28), pos + Vec2::new(40.0, 12.0));
                v.state = BossCatState::Roar;
                v.progress = 0.58;
                v.hp_frac = 0.32;
                v.anim = 1.7;
                draw_cell(&mut pb, &v);

                let pos = cam + Vec2::new(col_x[4] - 6.0, row_y[2]);
                let mut v = BossCatView::new(pos, Vec2::from_angle(0.35), pos + Vec2::new(30.0, 10.0));
                v.state = BossCatState::Death;
                v.progress = 0.72;
                v.hp_frac = 0.0;
                v.anim = 3.2;
                draw_cell(&mut pb, &v);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_boss_cat.ppm", out).unwrap();
    }
}
