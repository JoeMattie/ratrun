//! Art and gameplay specification for The Outfall's boss — **THE LANTERN**
//! (a giant abyssal anglerfish lurking in the storm-gate basin).
//!
//! # Concept & Visual Reference
//! Calibrated directly against the orthographic concept plates
//! (`scene-g1-ocean-lure.png` and `scene-g2-ocean-anglerfish.png`):
//! - **In darkness (`ArrivalLure`, `DarkTell`, `DarkLunge`)**: a blazing `6 px`
//!   cyan-white esca bulb suspended on a dark arched fishing-rod stalk, a
//!   single glowing golden-yellow `+` eye, and razor-thin `1 px` self-lit
//!   curved needle teeth catching the lure spill out of an almost-black abyss.
//! - **Revealed (`Swim`, `Feint*`, `Gape*`)**: a huge ~88 px deep-sea
//!   anglerfish with a bulbous dark mottled forehead dome, protruding underbite
//!   lower jaw, pitch-black cavernous maw framed by spaced vertical needle
//!   fangs, spiny dorsal ridge, lateral pectoral fins, and a tapering sinuous
//!   caudal tail.
//! - **360° orthographic readability**: horizontal and diagonal headings render
//!   the iconic 3/4 deep-sea profile (never upside-down); vertical headings
//!   (`±90°`) smoothly transition to a symmetrical top-down crescent maw with
//!   paired golden brow eyes and flared pectoral fins.
//!
//! # Move-set contract (this file owns the view/state types & constants)
//!
//! | Phase | States | Key geometry |
//! |---|---|---|
//! | Arrival | `ArrivalLure` (2.0 s) | bulb leads up to 36 px (r40), faint teeth in lure spill, no contact |
//! | 1 (`> 65%`) | `Swim`, `FeintApproach`, `FeintTell` (0.7 s, lock 0.5 s), `JawSnap` (0.12 s, 22 dmg), `FeintRecover` (0.8 s) | `16×8` jaw crescent 12 px behind real bulb; 3 Hz flicker; flank full / front 120° half dmg |
//! | 2 (`35..=65%`) | `DarkSwim`, `DecoyLeapTell` (0.5 s) → `DecoyLeap` (up to 80 px, 12 px trench, 20 dmg) → `DecoyRecover` (0.8 s); `DarkTell` (0.8 s, bulb out 1.6 s) → `DarkLunge` (56×12 trench, 0.3 s, 22 dmg) → `DarkRecover` (0.5 s) | steady r22 decoy bulb (hit r4, no stalk, no jaw); looming outlines visible at zero FX |
//! | 3 (`<= 35%`) | `DropLamp`, `GapeSwim`, `GapeTell` (0.7 s, lock 0.5 s), `GapePull` (1.4 s, 28 px/s), `GapeStamp` (0.2 s, 24 dmg), `GapeRecover` (1.6 s) | real bulb planted on seabed (r52) with consistent anchor; 90 px × 50° pull cone with readable negative space; 24×24 mouth stamp |
//! | Transition | `PhaseShift` | clean threshold handoff |
//! | Death | `Death` (2.4 s) | eye & teeth go dark first, hull sinks, planted bulb keeps glowing |
//!
//! # Layering contract for the renderer
//! 1. **Ground layer** (`draw_ground`): seabed drop shadow (stamped with pen `0`
//!    so the bump/specular pass never embosses it), planted bulb anchor cradle,
//!    subtle cone edge wash, and death silt rings.
//! 2. **Entity layer** (`draw`): the ~88 px anglerfish silhouette, self-lit
//!    needle teeth (`>= 232` channel peaks for `mark_emissive_highlights`),
//!    golden `+` eye, arched illicium stalk (or snapped stump), and both
//!    world-space bulbs.
//! 3. **Hostile / post-lighting overlay** (`draw_tell_overlay`): runs after
//!    lighting and foreground; keeps bulb cores, dark-state teeth/eye glints,
//!    and every fixed attack outline (16×8 jaw crescent, 12 px leap trench,
//!    56×12 dark lunge trench, 90 px 50° Gape cone with negative space, and
//!    24×24 mouth stamp) bright and readable with lighting/FX disabled or at
//!    reduced terminal colour depth.
//!
//! Everything is stack-only, deterministic (draws no RNG), and clips cleanly to
//! the framebuffer so narrow terminals (`57×20`) crop rear tail overhang
//! without losing the head, teeth, or lure.

#![allow(dead_code)]

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::game::ecs::systems::boss::HazardShape;
use crate::math::{lerp, Vec2};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Public Collision, Visual & Gameplay Constants (art <-> boss <-> lighting)
// ============================================================================

/// The head collider radius (world px): `r18` Large is the only grounded hit
/// shape, auto-aim point, and damage-number anchor.
pub const HEAD_RADIUS: f32 = 18.0;

/// Alias for framework movement/contact radius (matches [`HEAD_RADIUS`]).
pub const BODY_RADIUS: f32 = HEAD_RADIUS;

/// Total visible body extent from front jaw lip (`+16`) to caudal tail tip
/// (`-72`), within the `80–96 px` plan budget.
pub const BODY_LENGTH: f32 = 88.0;

/// Alias for visible body extent.
pub const BODY_EXTENT: f32 = BODY_LENGTH;

/// Flexible resting illicium stalk length (world px).
pub const STALK_LENGTH: f32 = 28.0;

/// Visual diameter of the cyan esca bulb (`6 px`, radius `3 px`).
pub const BULB_DIAMETER: f32 = 6.0;
pub const BULB_RADIUS: f32 = 3.0;

/// Swept projectile hit radius for the Phase 2 floating decoy bulb (`1 HP`).
pub const DECOY_HIT_RADIUS: f32 = 4.0;

/// Culling margin around `view.pos` (plan requires `>= 108`).
pub const CULL_MARGIN: f32 = 120.0;

/// Base HP pool before per-kind scale and campaign/director curves.
pub const BOSS_BASE_HP: f32 = 1600.0;

/// Per-kind HP scale (first tuning point: ~11,916 Sector 2 / ~4,166 practice).
pub const BOSS_HP_SCALE: f32 = 1.25;

/// Contact damage outside suppressed attack / recovery windows.
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 threshold (`> 35%..=65%`).
pub const PHASE2_HP_FRAC: f32 = 0.65;

/// Phase 3 threshold (`<= 35%`).
pub const PHASE3_HP_FRAC: f32 = 0.35;

/// Critical health visual cue threshold (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

// ---- Light & Flicker constants --------------------------------------------

/// Real lure dynamic light radius during arrival and normal combat.
pub const BULB_LIGHT_RADIUS: f32 = 40.0;

/// Real lure dimmed light radius during recovery windows.
pub const BULB_RECOVER_LIGHT_RADIUS: f32 = 20.0;

/// Real lure light radius once dropped/planted on the seabed in Phase 3.
pub const BULB_PLANTED_LIGHT_RADIUS: f32 = 52.0;

/// Phase 2 floating decoy steady light radius.
pub const DECOY_LIGHT_RADIUS: f32 = 22.0;

/// Cosmetic surface flicker frequency for the real lure during `FeintTell` (Hz).
pub const LURE_FLICKER_HZ: f32 = 3.0;

// ---- Arrival & Phase 1 (Feint) --------------------------------------------

pub const ARRIVAL_DURATION: f32 = 2.0;
pub const ARRIVAL_LURE_LEAD: f32 = 36.0;

pub const SWIM_SPEED: f32 = 40.0;
pub const FEINT_LURE_SPEED: f32 = 30.0;
pub const FEINT_STOP_SHORT: f32 = 36.0;
pub const LURE_TETHER_MAX: f32 = 48.0;
pub const FEINT_TELL_DURATION: f32 = 0.7;
pub const FEINT_LOCK_TIME: f32 = 0.5;
pub const FEINT_LOCK_PROGRESS: f32 = FEINT_LOCK_TIME / FEINT_TELL_DURATION;
/// `16×8` jaw crescent centred `12 px` behind the real bulb along locked aim.
pub const JAW_CRESCENT_LEN: f32 = 16.0;
pub const JAW_CRESCENT_WIDTH: f32 = 8.0;
pub const JAW_CRESCENT_BACK: f32 = 12.0;
pub const JAW_SNAP_DURATION: f32 = 0.12;
pub const JAW_SNAP_DAMAGE: f32 = 22.0;
pub const FEINT_RECOVER_DURATION: f32 = 0.8;
/// Frontal 120° arc (`±60°`) takes half damage during `FeintRecover`.
pub const FEINT_FRONT_HALF_ANGLE: f32 = PI / 3.0;
pub const FEINT_FRONT_DAMAGE_MULT: f32 = 0.5;
pub const FEINT_COOLDOWN: f32 = 2.4;

// ---- Phase 2 (Snuff, Decoy & Dark Lunge) ----------------------------------

pub const SNUFF_INTERVAL: f32 = 7.0;
pub const SNUFF_REACH: f32 = 160.0;
pub const DECOY_SPEED: f32 = 24.0;
pub const DECOY_LIFE: f32 = 3.0;
pub const DECOY_LEAP_TELL_DURATION: f32 = 0.5;
pub const DECOY_LEAP_MAX_DIST: f32 = 80.0;
pub const DECOY_LEAP_SPEED: f32 = 300.0;
pub const DECOY_LEAP_WIDTH: f32 = 12.0;
pub const DECOY_LEAP_DAMAGE: f32 = 20.0;
pub const DECOY_RECOVER_DURATION: f32 = 0.8;

pub const DARK_EXTINGUISH_DURATION: f32 = 1.6;
pub const DARK_TELL_DURATION: f32 = 0.8;
pub const DARK_LOCK_PROGRESS: f32 = 0.5;
pub const DARK_LUNGE_LEN: f32 = 56.0;
pub const DARK_LUNGE_WIDTH: f32 = 12.0;
pub const DARK_LUNGE_DURATION: f32 = 0.3;
pub const DARK_LUNGE_DAMAGE: f32 = 22.0;
pub const DARK_RECOVER_DURATION: f32 = 0.5;

// ---- Phase 3 (Planted Lamp & Gape) ----------------------------------------

pub const DROP_LAMP_DURATION: f32 = 0.8;
pub const GAPE_ORBIT_DIST: f32 = 40.0;
pub const GAPE_SWIM_SPEED: f32 = 50.0;
pub const GAPE_TELL_DURATION: f32 = 0.7;
pub const GAPE_LOCK_TIME: f32 = 0.5;
pub const GAPE_LOCK_PROGRESS: f32 = GAPE_LOCK_TIME / GAPE_TELL_DURATION;
/// `90 px` cone with `50°` total aperture (`±25°`) anchored to the mouth.
pub const GAPE_CONE_RANGE: f32 = 90.0;
pub const GAPE_CONE_TOTAL_DEG: f32 = 50.0;
pub const GAPE_CONE_HALF_ANGLE: f32 = 25.0 * PI / 180.0;
pub const GAPE_PULL_DURATION: f32 = 1.4;
pub const GAPE_PULL_SPEED: f32 = 28.0;
/// `24×24` mouth stamp box (`half = 12 px`), centre-point tested once.
pub const GAPE_STAMP_SIZE: f32 = 24.0;
pub const GAPE_STAMP_DURATION: f32 = 0.2;
pub const GAPE_STAMP_DAMAGE: f32 = 24.0;
pub const GAPE_RECOVER_DURATION: f32 = 1.6;
pub const GAPE_RECOVER_SPEED: f32 = 24.0;
pub const GAPE_COOLDOWN: f32 = 5.5;

// ---- Transition & Death ---------------------------------------------------

pub const PHASE_SHIFT_DURATION: f32 = 0.8;
pub const DEATH_DURATION: f32 = 2.4;

// ============================================================================
// Public State & View Types (boss controller <-> art <-> lighting <-> library)
// ============================================================================

/// Complete state machine for **The Lantern**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanternState {
    /// 2.0 s arrival: real cyan bulb steps forward up to 36 px; head silhouette
    /// reads as faint teeth in the lure spill. No body contact.
    ArrivalLure,
    /// Phase 1 neutral pursuit at 40 px/s.
    Swim,
    /// Real lure reaches ahead (up to 30 px/s, 48 px tether) toward the rat.
    FeintApproach,
    /// 0.7 s tell (3 Hz flicker, lock at 0.5 s): 16×8 jaw crescent 12 px behind bulb.
    FeintTell,
    /// 0.12 s active snap across the locked 16×8 jaw crescent (22 damage once).
    JawSnap,
    /// 0.8 s recovery: bulb dimmed (r20), flank takes full / frontal 120° half.
    FeintRecover,
    /// Phase 2 neutral pursuit (snuffs nearest sconce every 7 s, spawns decoy).
    DarkSwim,
    /// 0.5 s retaliation tell after a player projectile pops the decoy (12 px trench).
    DecoyLeapTell,
    /// High-speed leap up to 80 px at 300 px/s along the locked 12 px trench (20 dmg).
    DecoyLeap,
    /// 0.8 s recovery after `DecoyLeap`.
    DecoyRecover,
    /// Once-per-phase dark ambush tell (0.8 s, bulb extinguished 1.6 s, 56×12 trench).
    DarkTell,
    /// 0.3 s lunge covering the locked 56×12 head-anchored trench (22 dmg).
    DarkLunge,
    /// 0.5 s recovery after `DarkLunge`.
    DarkRecover,
    /// Phase 3 entry: snaps stalk and plants the real bulb on legal sand (r52).
    DropLamp,
    /// Phase 3 neutral pursuit / orbit (~40 px around planted bulb, up to 50 px/s).
    GapeSwim,
    /// 0.7 s tell (lock at 0.5 s): 90 px × 50° pull cone + 24×24 mouth stamp warn.
    GapeTell,
    /// 1.4 s fixed head & cone pulling rat and loose gems toward mouth at 28 px/s.
    GapePull,
    /// 0.2 s filled 24×24 mouth crush (24 dmg once, centre-point test).
    GapeStamp,
    /// 1.6 s exposed head recovery (up to 24 px/s) while planted bulb stays bright.
    GapeRecover,
    /// Clean threshold transition beat so phase shifts never clip a locked move.
    PhaseShift,
    /// 2.4 s death: head goes dark first, hull sinks, real bulb glows on.
    Death,
}

impl LanternState {
    /// Neutral movement/pursuit states where new attacks or phase shifts may start.
    #[inline]
    pub const fn is_neutral(self) -> bool {
        matches!(
            self,
            LanternState::Swim | LanternState::DarkSwim | LanternState::GapeSwim
        )
    }

    /// Punish / recovery beats after a committed strike.
    #[inline]
    pub const fn is_recovery(self) -> bool {
        matches!(
            self,
            LanternState::FeintRecover
                | LanternState::DecoyRecover
                | LanternState::DarkRecover
                | LanternState::GapeRecover
        )
    }

    /// Wind-up telegraph states.
    #[inline]
    pub const fn is_tell(self) -> bool {
        matches!(
            self,
            LanternState::FeintTell
                | LanternState::DecoyLeapTell
                | LanternState::DarkTell
                | LanternState::GapeTell
        )
    }

    /// Active strike / pull states.
    #[inline]
    pub const fn is_active_strike(self) -> bool {
        matches!(
            self,
            LanternState::JawSnap
                | LanternState::DecoyLeap
                | LanternState::DarkLunge
                | LanternState::GapePull
                | LanternState::GapeStamp
        )
    }

    /// Committed states that hold their plan under stagger and shove.
    #[inline]
    pub const fn is_committed(self) -> bool {
        !self.is_neutral() && !matches!(self, LanternState::Death)
    }
}

/// World-space light & visual snapshot of one Lantern esca bulb (either the
/// real lure or the Phase 2 floating decoy).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LanternBulbView {
    /// Exact world-space centre of the bulb.
    pub pos: Vec2,
    /// Dynamic light radius in world pixels (`40` live, `20` recover, `52` planted, `22` decoy).
    pub radius: f32,
    /// Relative light emission gain (`0.0` when extinguished, ~`1.0` live).
    pub gain: f32,
    /// Cosmetic surface flicker phase (radians / offset). Steady bulbs keep `0.0`.
    pub flicker: f32,
    /// `true` when this bulb is the Phase 2 decoy rather than the real lure.
    pub is_decoy: bool,
    /// `true` once the real bulb has been dropped/planted on the seabed (Phase 3 / Death).
    pub planted: bool,
}

/// Pure, `Copy` render-state snapshot of **The Lantern**.
#[derive(Clone, Copy, Debug)]
pub struct LanternView {
    /// World-space centre of the head (`HEAD_RADIUS = 18` hit circle & aim point).
    pub pos: Vec2,
    /// Unit facing direction of the head (defaults to `(-1, 0)` if zero).
    pub facing: Vec2,
    /// Current state of the anglerfish controller.
    pub state: LanternState,
    /// Normalized progress through `state`, clamped to `0.0..=1.0`.
    pub progress: f32,
    /// Continuous animation clock in seconds.
    pub anim: f32,
    /// Hit-flash intensity `0.0..=1.0`.
    pub hit_flash: f32,
    /// Remaining health fraction `0.0..=1.0`.
    pub hp_frac: f32,
    /// Controller presentation phase (`1`, `2`, or `3`).
    pub phase: u8,
    /// Locked world-space attack anchor (presentation: jaw/mouth pose).
    pub attack_origin: Vec2,
    /// Locked world-space attack endpoint (presentation: jaw/mouth pose).
    pub attack_target: Vec2,
    /// Locked unit attack facing direction (presentation: jaw/mouth angle).
    pub attack_facing: Vec2,
    /// `true` once the current tell has locked its geometry.
    pub attack_locked: bool,
    /// Authoritative simulation footprint of the live/locked attack (`None` when none).
    /// Tells, bot, and ink read this directly; art never reconstructs a footprint.
    pub attack: Option<HazardShape>,
    /// Authoritative simulation footprint of the Gape mouth stamp (`None` when none).
    pub stamp: Option<HazardShape>,
    /// Real lure bulb snapshot (`None` if absent; `gain == 0.0` when extinguished).
    pub real_bulb: Option<LanternBulbView>,
    /// Phase 2 floating decoy bulb snapshot (`None` when no decoy is active).
    pub decoy: Option<LanternBulbView>,
}

// ============================================================================
// Palette — calibrated directly to scene-g1-ocean-lure & scene-g2-ocean-anglerfish
// ============================================================================

const HIDE_ABYSS: Rgb = (5, 11, 16);
const HIDE_DARK: Rgb = (9, 20, 27);
const HIDE_MID: Rgb = (16, 34, 44);
const HIDE_RIM: Rgb = (30, 62, 74);
const HIDE_SHEEN: Rgb = (52, 108, 122);
const HIDE_MOTTLE: Rgb = (12, 26, 35);
const FIN_DARK: Rgb = (7, 15, 21);
const FIN_EDGE: Rgb = (24, 50, 60);
const MAW_BLACK: Rgb = (2, 4, 7);
const GUM_DARK: Rgb = (11, 22, 29);

/// Self-lit tooth peaks (`>= 232` so `LightMap::mark_emissive_highlights` keeps
/// leading fangs blazing bright even in deep darkness).
const TOOTH_GLINT_WHITE: Rgb = (238, 255, 255);
const TOOTH_GLINT_HI: Rgb = (146, 248, 254);
const TOOTH_GLINT_MID: Rgb = (76, 198, 214);
const TOOTH_GLINT_DIM: Rgb = (38, 116, 134);
const TOOTH_DEAD: Rgb = (66, 74, 80);

/// Golden-yellow `+` diamond eye (matching `scene-g1` and `scene-g2`, `>= 232`).
const EYE_BRIGHT: Rgb = (252, 232, 112);
const EYE_GOLD: Rgb = (214, 174, 58);
const EYE_PUPIL: Rgb = (10, 12, 14);

const STALK_DARK: Rgb = (8, 18, 24);
const STALK_RIM: Rgb = (34, 82, 96);

const BULB_CORE: Rgb = (244, 255, 255);
const BULB_RING: Rgb = (94, 244, 255);
const BULB_OUTER: Rgb = (34, 152, 184);

const DECOY_CORE: Rgb = (218, 244, 255);
const DECOY_RING: Rgb = (82, 198, 242);
const DECOY_HALO: Rgb = (24, 96, 138);

const SHADOW_COL: Rgb = (3, 7, 11);
const CRADLE_SHADOW: Rgb = (4, 10, 15);
const CRADLE_RING: Rgb = (28, 108, 126);
const CRADLE_TENDRIL: Rgb = (18, 68, 84);

/// High-contrast hostile tell colors (legible at zero FX and ANSI-16).
const TELL_AMBER: Rgb = (255, 196, 86);
const TELL_HOT: Rgb = (255, 84, 72);
const TELL_CYAN: Rgb = (104, 246, 255);
const TELL_WHITE: Rgb = (248, 252, 255);
const FLANK_GOLD: Rgb = (255, 224, 126);

// ============================================================================
// Shared Geometry & Rule Helpers (single source of truth)
// ============================================================================

#[inline]
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

/// Safe unit facing direction (defaults to `(-1, 0)` if zero).
#[inline]
pub fn unit_facing(view: &LanternView) -> Vec2 {
    if view.facing.len_sq() > 1e-6 {
        view.facing.normalized()
    } else {
        Vec2::new(-1.0, 0.0)
    }
}

/// Safe unit locked attack facing direction (falls back to [`unit_facing`]).
#[inline]
pub fn unit_attack_facing(view: &LanternView) -> Vec2 {
    if view.attack_facing.len_sq() > 1e-6 {
        view.attack_facing.normalized()
    } else {
        unit_facing(view)
    }
}

/// The single grounded hit circle & auto-aim anchor (`HEAD_RADIUS = 18`).
#[inline]
pub const fn head_hit_circle(view: &LanternView) -> (Vec2, f32) {
    (view.pos, HEAD_RADIUS)
}

/// Front centre of the gaping maw (anchor for the Phase 3 Gape cone & stamp).
#[inline]
pub fn mouth_pos(view: &LanternView) -> Vec2 {
    view.pos + unit_facing(view) * 14.0
}

/// Forehead ridge where the arched illicium stalk emerges.
#[inline]
pub fn brow_anchor(view: &LanternView) -> Vec2 {
    let f = unit_facing(view);
    let (side, d_up) = dorsal_frame(f);
    view.pos + f * 4.5 + d_up * (14.5 * side)
}

/// Whether the boss deals ordinary body-contact damage (`8`) in `state`.
/// Suppressed during arrival, active hazard strikes, recovery windows,
/// phase shifts, and death.
#[inline]
pub const fn has_body_contact(state: LanternState) -> bool {
    matches!(
        state,
        LanternState::Swim
            | LanternState::FeintApproach
            | LanternState::FeintTell
            | LanternState::DarkSwim
            | LanternState::DecoyLeapTell
            | LanternState::DarkTell
            | LanternState::DropLamp
            | LanternState::GapeSwim
            | LanternState::GapeTell
    )
}

/// Returns `true` when an incoming attack arriving from `attack_dir` (vector
/// from boss head toward attacker) strikes the armoured frontal `120°` jaw arc
/// during [`LanternState::FeintRecover`] (which takes half damage; flanks take full).
#[inline]
pub fn is_frontal_armored(view: &LanternView, toward_attacker: Vec2) -> bool {
    if !matches!(view.state, LanternState::FeintRecover) || toward_attacker.len_sq() <= 1e-6 {
        return false;
    }
    let dir = toward_attacker.normalized();
    dir.dot(unit_facing(view)) >= FEINT_FRONT_HALF_ANGLE.cos()
}

/// Oriented rectangle `(center, facing, half_along, half_perp)` for the
/// `16×8` Phase 1/2 feint jaw crescent (reads `view.attack` directly when present).
#[inline]
pub fn jaw_crescent_rect(view: &LanternView) -> (Vec2, Vec2, f32, f32) {
    if let Some(HazardShape::Rect {
        center,
        facing,
        half,
        half_perp,
    }) = view.attack
    {
        return (center, safe_unit(facing, unit_facing(view)), half, half_perp);
    }
    let af = unit_attack_facing(view);
    let bulb_pos = view
        .real_bulb
        .map(|b| b.pos)
        .unwrap_or_else(|| view.attack_origin + af * JAW_CRESCENT_BACK);
    let center = bulb_pos - af * JAW_CRESCENT_BACK;
    (center, af, JAW_CRESCENT_WIDTH * 0.5, JAW_CRESCENT_LEN * 0.5)
}

/// Oriented rectangle `(center, facing, half_along, half_perp)` for the
/// Phase 2 `56×12` head-anchored `DarkLunge` trench (reads `view.attack` when present).
#[inline]
pub fn dark_lunge_rect(view: &LanternView) -> (Vec2, Vec2, f32, f32) {
    if let Some(HazardShape::Rect {
        center,
        facing,
        half,
        half_perp,
    }) = view.attack
    {
        return (center, safe_unit(facing, unit_facing(view)), half, half_perp);
    }
    let af = unit_attack_facing(view);
    let origin = if view.attack_origin.len_sq() > 1e-3 {
        view.attack_origin
    } else {
        view.pos
    };
    let center = origin + af * (DARK_LUNGE_LEN * 0.5);
    (center, af, DARK_LUNGE_LEN * 0.5, DARK_LUNGE_WIDTH * 0.5)
}

/// `(origin, facing, range, half_angle)` for the Phase 3 Gape pull cone
/// (reads `view.attack` directly when present).
#[inline]
pub fn gape_cone(view: &LanternView) -> (Vec2, Vec2, f32, f32) {
    if let Some(HazardShape::Arc {
        center,
        outer,
        facing,
        half_angle,
        ..
    }) = view.attack
    {
        return (center, safe_unit(facing, unit_facing(view)), outer, half_angle);
    }
    let af = unit_attack_facing(view);
    let origin = if view.attack_origin.len_sq() > 1e-3 {
        view.attack_origin
    } else {
        mouth_pos(view)
    };
    (origin, af, GAPE_CONE_RANGE, GAPE_CONE_HALF_ANGLE)
}

/// Oriented rectangle `(center, facing, half_along, half_perp)` for the
/// Phase 3 `24×24` mouth stamp box (reads `view.stamp` directly when present).
#[inline]
pub fn gape_stamp_rect(view: &LanternView) -> (Vec2, Vec2, f32, f32) {
    if let Some(HazardShape::Rect {
        center,
        facing,
        half,
        half_perp,
    }) = view.stamp
    {
        return (center, safe_unit(facing, unit_facing(view)), half, half_perp);
    }
    let (origin, af, _, _) = gape_cone(view);
    let half = GAPE_STAMP_SIZE * 0.5;
    (origin, af, half, half)
}

/// Cosmetic surface flicker multiplier for one esca bulb.
/// Decoys stay steady; real bulbs follow the controller's radian `flicker`
/// phase when non-zero (else step at `3 Hz` during `FeintTell`).
#[inline]
pub fn bulb_flicker_mult(bulb: &LanternBulbView, anim: f32, is_feint_tell: bool) -> f32 {
    if bulb.is_decoy || bulb.gain <= 0.0 {
        return bulb.gain.clamp(0.0, 1.5);
    }
    if is_feint_tell {
        let wave = if bulb.flicker.abs() > 1e-4 {
            bulb.flicker.sin()
        } else {
            (anim * LURE_FLICKER_HZ * TAU).sin()
        };
        bulb.gain * if wave >= 0.0 { 1.22 } else { 0.58 }
    } else {
        bulb.gain * (0.94 + 0.08 * (anim * 3.2 + bulb.flicker).sin())
    }
}

/// Screen-space culling test: returns `true` whenever the head, real bulb,
/// decoy bulb, or authoritative attack/stamp footprint can contribute pixels to `pb`.
pub fn in_view(pb: &PixelBuffer, view: &LanternView, cam: Vec2) -> bool {
    if pb.w == 0 || pb.h == 0 {
        return false;
    }
    let w = pb.w as f32;
    let h = pb.h as f32;
    let hit = |world_pt: Vec2, margin: f32| {
        let sx = world_pt.x - cam.x;
        let sy = world_pt.y - cam.y;
        (-margin..=w + margin).contains(&sx) && (-margin..=h + margin).contains(&sy)
    };
    if hit(view.pos, CULL_MARGIN) {
        return true;
    }
    if view.real_bulb.is_some_and(|b| hit(b.pos, 48.0)) {
        return true;
    }
    if view.decoy.is_some_and(|d| hit(d.pos, 32.0)) {
        return true;
    }
    if view
        .attack
        .is_some_and(|shape| hit(shape.center(), GAPE_CONE_RANGE + 16.0))
    {
        return true;
    }
    if view
        .stamp
        .is_some_and(|shape| hit(shape.center(), GAPE_STAMP_SIZE + 16.0))
    {
        return true;
    }
    false
}

#[inline]
fn safe_unit(v: Vec2, fallback: Vec2) -> Vec2 {
    if v.len_sq() > 1e-6 {
        v.normalized()
    } else {
        fallback
    }
}

// ============================================================================
// Internal Projection & Animation Helpers
// ============================================================================

/// Returns `(side_factor, dorsal_up)`:
/// - `side_factor` in `0.0..=1.0`: `1.0` on horizontal/diagonal headings (3/4
///   iconic `scene-g1`/`scene-g2` profile), `0.0` on straight vertical headings
///   (symmetrical top-down crescent jaw with paired dorsal eyes).
/// - `dorsal_up`: unit vector perpendicular to `f`, always flipped toward
///   screen-up (`-Y`) so the fish is never upside down when swimming left/right.
#[inline]
fn dorsal_frame(f: Vec2) -> (f32, Vec2) {
    let side = smoothstep((f.x.abs() - 0.16) / 0.44);
    let p = f.perp();
    let d_up = if p.y > 0.0 {
        -p
    } else if p.y < 0.0 {
        p
    } else {
        Vec2::new(0.0, -1.0)
    };
    (side, d_up)
}

#[inline]
fn jaw_open_factor(view: &LanternView) -> f32 {
    let p = clamp01(view.progress);
    let breath = 0.56 + 0.07 * (view.anim * 2.4).sin();
    match view.state {
        LanternState::ArrivalLure => lerp(0.46, 0.62, smoothstep(p)),
        LanternState::Swim
        | LanternState::DarkSwim
        | LanternState::GapeSwim
        | LanternState::PhaseShift => breath + if view.phase >= 3 { 0.08 } else { 0.0 },
        LanternState::FeintApproach => 0.60 + 0.10 * p,
        LanternState::FeintTell => lerp(0.62, 0.90, smoothstep(p)),
        LanternState::JawSnap => {
            if p < 0.65 {
                0.06
            } else {
                lerp(0.06, 0.35, (p - 0.65) / 0.35)
            }
        }
        LanternState::FeintRecover | LanternState::DecoyRecover | LanternState::DarkRecover => {
            0.40 + 0.06 * (view.anim * 6.0).sin()
        }
        LanternState::DecoyLeapTell | LanternState::DarkTell => lerp(0.54, 0.88, smoothstep(p)),
        LanternState::DecoyLeap | LanternState::DarkLunge => {
            if p < 0.70 {
                0.94
            } else {
                lerp(0.94, 0.22, (p - 0.70) / 0.30)
            }
        }
        LanternState::DropLamp => 0.60 + 0.14 * (p * PI).sin(),
        LanternState::GapeTell => lerp(0.64, 1.0, smoothstep(p)),
        LanternState::GapePull => 1.0,
        LanternState::GapeStamp => {
            if p < 0.55 {
                0.05
            } else {
                lerp(0.05, 0.38, (p - 0.55) / 0.45)
            }
        }
        LanternState::GapeRecover => 0.66 + 0.06 * (view.anim * 5.0).sin(),
        LanternState::Death => lerp(0.54, 0.22, smoothstep(p)),
    }
}

#[inline]
fn head_visibility(view: &LanternView) -> f32 {
    let p = clamp01(view.progress);
    match view.state {
        LanternState::ArrivalLure => smoothstep((p - 0.26) / 0.74),
        LanternState::DarkTell | LanternState::DarkLunge => 0.28,
        // Hull stays dimly visible while sinking after eye & teeth darken.
        LanternState::Death => 1.0 - smoothstep((p - 0.28) / 0.56),
        _ => 1.0,
    }
}

#[inline]
fn tooth_glow_factor(view: &LanternView) -> f32 {
    let p = clamp01(view.progress);
    match view.state {
        LanternState::ArrivalLure => lerp(0.48, 0.96, smoothstep(p)),
        // Teeth fade to dead bone-grey before the hull dissolves.
        LanternState::Death => 1.0 - smoothstep((p - 0.08) / 0.46),
        LanternState::DarkTell | LanternState::DarkLunge => 0.96,
        LanternState::GapeTell
        | LanternState::GapePull
        | LanternState::JawSnap
        | LanternState::GapeStamp => 1.0,
        _ => 0.92,
    }
}

#[inline]
fn eye_glow_factor(view: &LanternView) -> f32 {
    let p = clamp01(view.progress);
    match view.state {
        // Golden eye goes dark very first on Death.
        LanternState::Death => 1.0 - smoothstep(p / 0.32),
        LanternState::ArrivalLure => lerp(0.75, 1.0, smoothstep(p)),
        _ => 1.0,
    }
}

#[inline]
fn hash2(ix: i32, iy: i32) -> f32 {
    let mut h = (ix.wrapping_mul(374_761_393) as u32) ^ (iy.wrapping_mul(668_265_263) as u32);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xFF) as f32 / 255.0
}

#[inline]
fn w2s(pt: Vec2, cam: Vec2) -> Vec2 {
    Vec2::new(pt.x - cam.x, pt.y - cam.y)
}

// ============================================================================
// Stack-Only Pixel Primitives
// ============================================================================

#[inline]
fn plot_blend(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 {
        return;
    }
    let (ux, uy) = (x as usize, y as usize);
    if ux >= pb.w || uy >= pb.h {
        return;
    }
    if alpha >= 0.999 {
        pb.plot(x, y, c);
    } else {
        let cur = pb.pixel_at(ux, uy);
        pb.plot(x, y, mix(cur, c, alpha));
    }
}

/// Screen-space body basis shared by hull, teeth, eyes, and stalk helpers.
#[derive(Clone, Copy)]
struct BodyFrame {
    sc: Vec2,
    f: Vec2,
    d_up: Vec2,
    side: f32,
}

/// Line/outline stroke parameters (keeps helper arities clippy-clean).
#[derive(Clone, Copy)]
struct Stroke {
    col: Rgb,
    alpha: f32,
    dashed: i32,
    phase: i32,
}

impl Stroke {
    #[inline]
    const fn solid(col: Rgb, alpha: f32) -> Self {
        Self {
            col,
            alpha,
            dashed: 0,
            phase: 0,
        }
    }

    #[inline]
    const fn styled(col: Rgb, alpha: f32, dashed: i32, phase: i32) -> Self {
        Self {
            col,
            alpha,
            dashed,
            phase,
        }
    }
}

fn line_blend(pb: &mut PixelBuffer, a: Vec2, b: Vec2, stroke: Stroke) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
    let inv = 1.0 / steps as f32;
    for i in 0..=steps {
        if stroke.dashed > 0 && ((i + stroke.phase).div_euclid(stroke.dashed)) % 2 != 0 {
            continue;
        }
        let t = i as f32 * inv;
        let x = (a.x + dx * t).round() as i32;
        let y = (a.y + dy * t).round() as i32;
        plot_blend(pb, x, y, stroke.col, stroke.alpha);
    }
}

fn disc_blend(pb: &mut PixelBuffer, center: Vec2, r: f32, c: Rgb, alpha: f32) {
    if r <= 0.0 || alpha <= 0.0 {
        return;
    }
    let ri = r.ceil() as i32;
    let r2 = r * r;
    let ix = center.x.round() as i32;
    let iy = center.y.round() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if (dx * dx + dy * dy) as f32 <= r2 {
                plot_blend(pb, ix + dx, iy + dy, c, alpha);
            }
        }
    }
}

fn ring_blend(pb: &mut PixelBuffer, center: Vec2, r: f32, stroke: Stroke) {
    if r <= 0.5 || stroke.alpha <= 0.0 {
        return;
    }
    let steps = ((r * 6.5).ceil() as i32).max(16);
    for i in 0..steps {
        if stroke.dashed > 0 && ((i + stroke.phase).div_euclid(stroke.dashed)) % 2 != 0 {
            continue;
        }
        let a = i as f32 / steps as f32 * TAU;
        let d = Vec2::from_angle(a);
        let x = (center.x + d.x * r).round() as i32;
        let y = (center.y + d.y * r).round() as i32;
        plot_blend(pb, x, y, stroke.col, stroke.alpha);
    }
}

fn draw_oriented_rect_outline(
    pb: &mut PixelBuffer,
    center: Vec2,
    facing: Vec2,
    half_along: f32,
    half_perp: f32,
    stroke: Stroke,
) {
    let u = facing * half_along;
    let v = facing.perp() * half_perp;
    let pts = [center + u + v, center + u - v, center - u - v, center - u + v];
    for i in 0..4 {
        let edge_stroke = Stroke::styled(
            stroke.col,
            stroke.alpha,
            stroke.dashed,
            stroke.phase + i as i32 * 3,
        );
        line_blend(pb, pts[i], pts[(i + 1) % 4], edge_stroke);
    }
}

// ============================================================================
// 1. Ground Layer (shadow, planted bulb anchor, subtle floor cues)
// ============================================================================

/// Draw The Lantern's seabed ground layer (called after floor stains, before
/// props and entities).
pub fn draw_ground(pb: &mut PixelBuffer, view: &LanternView, cam: Vec2, _accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let f = unit_facing(view);
    let (side, d_up) = dorsal_frame(f);
    let vis = head_visibility(view);
    let p = clamp01(view.progress);
    let sc = w2s(view.pos, cam);

    // Stamp seabed drop shadow with material pen 0 so surface bump never embosses it.
    let prev_pen = pb.set_pen(0);
    if vis > 0.06 {
        let shadow_alpha = 0.42 * vis;
        for i in 0..9 {
            let t = i as f32 / 8.0;
            let u = lerp(0.0, -58.0, t);
            let sway = (view.anim * 2.6 - t * 3.4).sin() * (1.5 + 5.0 * t * t);
            let r = lerp(16.0, 4.0, t.powf(0.85));
            let pos = sc + f * u + d_up * (-sway * (1.0 - side * 0.55) - 2.0) + Vec2::new(0.0, 3.0);
            disc_blend(pb, pos, r, SHADOW_COL, shadow_alpha * lerp(1.0, 0.55, t));
        }
    }

    // Planted real bulb consistent anchor cradle on seabed (Phase 3 & Death).
    if let Some(b) = view.real_bulb.filter(|b| b.planted) {
        let bs = w2s(b.pos, cam);
        disc_blend(pb, bs + Vec2::new(0.0, 1.0), 7.2, CRADLE_SHADOW, 0.58);
        ring_blend(pb, bs, 6.4, Stroke::solid(CRADLE_RING, 0.58));
        for k in 0..5 {
            let a = k as f32 * TAU / 5.0 + 0.35;
            let d = Vec2::from_angle(a);
            line_blend(
                pb,
                bs + d * 2.5,
                bs + d * 7.5,
                Stroke::solid(CRADLE_TENDRIL, 0.65),
            );
        }
        if matches!(view.state, LanternState::DropLamp) {
            let rr = 6.0 + 24.0 * smoothstep(p);
            ring_blend(pb, bs, rr, Stroke::solid(BULB_RING, 0.58 * (1.0 - p)));
        }
    }

    // Subtle seabed edge wash for the authoritative Gape cone (`view.attack`).
    if let Some(HazardShape::Arc {
        center,
        inner,
        outer,
        facing,
        half_angle,
    }) = view.attack
    {
        let ms = w2s(center, cam);
        let base_ang = safe_unit(facing, f).angle();
        let r_start = inner.max(8.0);
        for sign in [-1.0_f32, 1.0_f32] {
            let d = Vec2::from_angle(base_ang + sign * half_angle);
            line_blend(
                pb,
                ms + d * r_start,
                ms + d * outer,
                Stroke::solid((20, 82, 98), 0.42),
            );
        }
    }

    // Death dissolving seabed silt ring.
    if matches!(view.state, LanternState::Death) {
        let rr = 14.0 + 28.0 * smoothstep(p);
        ring_blend(pb, sc, rr, Stroke::solid((22, 46, 56), 0.42 * (1.0 - p)));
    }

    pb.set_pen(prev_pen);
}

// ============================================================================
// 2. Entity Layer (giant anglerfish silhouette + teeth + eyes + stalk + bulbs)
// ============================================================================

/// Cross-section `(v_min, v_max)` along local spine coordinate `u in [-72, +16]`.
/// Total length = `88 px` (snout/chin `+16` to notched caudal tail tip `-72`).
#[inline]
fn body_profile_bounds(u: f32, side: f32) -> (f32, f32) {
    if !(-72.0..=16.0).contains(&u) {
        return (1.0, -1.0);
    }
    if u >= 0.0 {
        let tu_top = (u / lerp(15.0, 14.2, side)).clamp(0.0, 1.0);
        let tu_bot = (u / 16.0).clamp(0.0, 1.0);
        let v_max = lerp(19.5, 20.5, side) * (1.0 - (tu_top * 0.96).powi(2)).max(0.0).sqrt();
        let v_min = -lerp(19.5, 19.0, side) * (1.0 - (tu_bot * 0.95).powi(2)).max(0.0).sqrt();
        return (v_min, v_max);
    }
    if u >= -54.0 {
        let t = -u / 54.0;
        let half = lerp(19.5, 3.2, t.powf(1.15)) + 2.0 * (t * PI).sin() * (1.0 - t);
        let v_max = half + 1.2 * side * (1.0 - t);
        let v_min = -half + 0.6 * side * t;
        return (v_min, v_max);
    }
    let t = (-u - 54.0) / 18.0;
    let taper = 1.0 - 0.22 * ((t - 0.72).max(0.0) / 0.28);
    let half = lerp(3.2, 13.5, smoothstep(t)) * taper;
    (-half, half)
}

/// Draw **The Lantern** in the entity pass (with the caller's creature material
/// pen active).
pub fn draw(pb: &mut PixelBuffer, view: &LanternView, cam: Vec2, _accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let f = unit_facing(view);
    let (side, d_up) = dorsal_frame(f);
    let vis = head_visibility(view);
    let gape = jaw_open_factor(view);
    let tglow = tooth_glow_factor(view);
    let hit = clamp01(view.hit_flash);
    let sc = w2s(view.pos, cam);

    // Bulb screen position drives subtle cyan rim sheen across the slimy brow & hide.
    let (bulb_s, bulb_lit) = match view.real_bulb {
        Some(b) if b.gain > 0.05 => (w2s(b.pos, cam), b.gain.clamp(0.0, 1.2)),
        _ => (sc + f * 30.0, 0.14),
    };

    // Screen-space bounding box of the 88 px hull, clipped directly to `pb` so
    // narrow terminals cleanly crop rear tail overhang without disturbing head/teeth.
    let corners = [
        sc + f * 20.0 + d_up * 26.0,
        sc + f * 20.0 - d_up * 26.0,
        sc - f * 74.0 + d_up * 26.0,
        sc - f * 74.0 - d_up * 26.0,
    ];
    let mut min_xf = corners[0].x;
    let mut max_xf = corners[0].x;
    let mut min_yf = corners[0].y;
    let mut max_yf = corners[0].y;
    for c in corners.iter().skip(1) {
        min_xf = min_xf.min(c.x);
        max_xf = max_xf.max(c.x);
        min_yf = min_yf.min(c.y);
        max_yf = max_yf.max(c.y);
    }
    let min_x = (min_xf.floor() as i32).max(0);
    let max_x = (max_xf.ceil() as i32).min(pb.w as i32 - 1);
    let min_y = (min_yf.floor() as i32).max(0);
    let max_y = (max_yf.ceil() as i32).min(pb.h as i32 - 1);

    let jaw_top = lerp(5.2, 11.4, gape);
    let jaw_bot = -lerp(6.4, 14.8, gape);
    let maw_back_u = lerp(2.5, -5.0, gape);

    let swim_freq = match view.state {
        LanternState::DecoyLeap | LanternState::DarkLunge | LanternState::GapeSwim => 4.6,
        _ => 2.5,
    };
    let swim_amp = match view.state {
        LanternState::Death => 0.15,
        LanternState::DecoyLeap | LanternState::DarkLunge => 1.35,
        _ => 1.0,
    };
    let recover_shimmer = matches!(
        view.state,
        LanternState::FeintRecover | LanternState::GapeRecover
    );
    let recover_pulse = 0.5 + 0.5 * (view.anim * 9.0).sin();

    if vis > 0.03 && min_x <= max_x && min_y <= max_y {
        for sy in min_y..=max_y {
            let ry = sy as f32 - sc.y;
            for sx in min_x..=max_x {
                let rx = sx as f32 - sc.x;
                let u = rx * f.x + ry * f.y;
                if !(-73.0..=17.0).contains(&u) {
                    continue;
                }
                let v_raw = rx * d_up.x + ry * d_up.y;

                // Sinuous tail swim undulation behind head (`u < -14`).
                let v = if u < -14.0 {
                    let bt = clamp01((-u - 14.0) / 58.0);
                    let sway = (view.anim * swim_freq - bt * 4.2).sin() * (6.2 * bt * bt) * swim_amp;
                    v_raw - sway * (1.0 - side * 0.45)
                } else {
                    v_raw
                };

                let (v_min, v_max) = body_profile_bounds(u, side);
                if v_min > v_max {
                    continue;
                }

                // Notched caudal tail fin at the far rear (`u < -62`).
                if u < -62.0 {
                    let notch = (-u - 62.0) / 10.0;
                    if v.abs() < notch * 4.8 {
                        continue;
                    }
                }

                // Spiny dorsal fin crest along upper back in profile view.
                let mut in_dorsal_fin = false;
                if side > 0.35 && (-48.0..=-10.0).contains(&u) && v > v_max - 0.5 {
                    let spine_wave = 0.5 + 0.5 * (u * 0.95).sin();
                    let fin_h = (2.8 + 3.0 * spine_wave)
                        * side
                        * (((-u - 10.0) / 38.0) * PI).sin();
                    if v <= v_max + fin_h {
                        in_dorsal_fin = true;
                    }
                }

                // Lateral pectoral fins (flared both sides in top-down, ventral nub in profile).
                let mut in_pec_fin = false;
                if (-26.0..=-8.0).contains(&u) {
                    let ft = (-u - 8.0) / 18.0;
                    let flap = (view.anim * 3.4 + ft * 1.4).sin() * 1.5;
                    let arc = (ft * PI).sin();
                    if side < 0.72 {
                        let pec_reach = v_max + (7.0 + flap) * (1.0 - side * 0.65) * arc;
                        if v.abs() <= pec_reach && (v > v_max - 1.0 || v < v_min + 1.0) {
                            in_pec_fin = true;
                        }
                    } else if v < v_min + 1.0 && v >= v_min - 3.4 * arc {
                        in_pec_fin = true;
                    }
                }

                let in_body = (v_min..=v_max).contains(&v);
                if !(in_body || in_dorsal_fin || in_pec_fin) {
                    continue;
                }

                // Gaping front abyssal maw cutout (no nose wall blocking the mouth!).
                if side >= 0.45 {
                    if u >= maw_back_u && (jaw_bot..=jaw_top).contains(&v) {
                        let v_mid = 0.5 * (jaw_top + jaw_bot);
                        let v_half = (0.5 * (jaw_top - jaw_bot)).max(1.0);
                        let vn = (v - v_mid) / v_half;
                        let back_curve = maw_back_u + (9.5 - maw_back_u) * (vn * vn * 0.45);
                        // Sloped upper lip + protruding underbite lower jaw.
                        let front_edge = lerp(15.2, 11.2, (vn + 1.0) * 0.5);
                        if u >= back_curve {
                            if u > front_edge {
                                continue;
                            }
                            if v >= jaw_top - 1.3 || v <= jaw_bot + 1.3 {
                                let mut col = GUM_DARK;
                                if hit > 0.0 {
                                    col = mix(col, (235, 246, 250), hit * 0.7);
                                }
                                plot_blend(pb, sx, sy, col, vis);
                            } else {
                                plot_blend(pb, sx, sy, MAW_BLACK, vis);
                            }
                            continue;
                        }
                    }
                } else {
                    let jaw_hw = lerp(9.0, 14.2, gape);
                    if u >= 3.5 && v.abs() <= jaw_hw {
                        let vn = v / jaw_hw.max(1.0);
                        let back_u = lerp(7.5, 4.0, gape) + 5.5 * vn * vn;
                        let front_u = 14.8 - 2.2 * vn * vn;
                        if u >= back_u {
                            if u > front_u {
                                continue;
                            }
                            plot_blend(pb, sx, sy, MAW_BLACK, vis);
                            continue;
                        }
                    }
                }

                // Slimy abyssal hide shading + cyan lure rim sheen.
                let mut col = if in_dorsal_fin || in_pec_fin {
                    let edge = if in_dorsal_fin && (u.abs() as i32) % 3 == 0 {
                        0.65
                    } else {
                        0.30
                    };
                    mix(FIN_DARK, FIN_EDGE, edge * (0.4 + 0.6 * bulb_lit))
                } else {
                    let half_h = (0.5 * (v_max - v_min)).max(1.0);
                    let v_center = 0.5 * (v_max + v_min);
                    let nv = (v - v_center) / half_h;
                    let m = hash2((u * 0.55).floor() as i32, (v * 0.55).floor() as i32);
                    let bdist = ((sx as f32 - bulb_s.x).powi(2) + (sy as f32 - bulb_s.y).powi(2))
                        .sqrt();
                    let bulb_kiss = clamp01(1.0 - bdist / 54.0).powf(1.65) * bulb_lit;

                    let dorsal_shade = clamp01(0.42 + 0.45 * nv * (0.45 + 0.55 * side));
                    let tail_fade =
                        (1.0 - ((-u - 16.0).max(0.0) / 56.0) * 0.68).clamp(0.25, 1.0);

                    let mut base = mix(HIDE_ABYSS, HIDE_MID, dorsal_shade * tail_fade);
                    if m > 0.66 {
                        base = mix(base, HIDE_MOTTLE, 0.45);
                    }

                    let rim_dist = (v_max - v).min(v - v_min);
                    let mut c = if rim_dist < 1.8 {
                        let rim_strength = clamp01(0.28 + 0.72 * bulb_kiss) * tail_fade;
                        if u > -18.0 {
                            mix(HIDE_RIM, HIDE_SHEEN, rim_strength)
                        } else {
                            mix(HIDE_ABYSS, HIDE_RIM, rim_strength * 0.55)
                        }
                    } else {
                        let mut inner = mix(base, HIDE_RIM, bulb_kiss * 0.52);
                        if (-10.0..=10.0).contains(&u) && (0.38..=0.80).contains(&nv) {
                            inner = mix(inner, HIDE_SHEEN, bulb_kiss * 0.48);
                        }
                        inner
                    };

                    // Painted lateral pectoral fin on flank in profile view.
                    if side >= 0.55 && (-26.0..=-12.0).contains(&u) && (-7.0..=1.0).contains(&v) {
                        let fu = (-u - 12.0) / 14.0;
                        if (v + 3.0).abs() < 3.6 * (fu * PI).sin() {
                            let ray = if ((v * 1.5).abs() as i32) % 2 == 0 {
                                0.65
                            } else {
                                0.20
                            };
                            c = mix(FIN_DARK, FIN_EDGE, ray);
                        }
                    }

                    // Subtle gill slit crease behind the head cheek.
                    if (-14.5..=-11.5).contains(&u) && v.abs() < half_h * 0.55 {
                        c = mix(c, HIDE_ABYSS, 0.65);
                    }

                    // Exposed flank punish rim highlight during FeintRecover / GapeRecover.
                    if recover_shimmer && (-14.0..=0.0).contains(&u) && rim_dist < 4.0 {
                        let edge_t = (1.0 - rim_dist / 4.0) * (((-u) / 14.0) * PI).sin();
                        c = mix(c, FLANK_GOLD, edge_t * (0.45 + 0.30 * recover_pulse));
                    }

                    // Phase 2+ pale battle scar on cheek.
                    if view.phase >= 2 && (-10.0..=2.0).contains(&u) && (v - 3.0).abs() < 0.7 {
                        c = mix(c, (40, 78, 92), 0.50);
                    }
                    c
                };

                if hit > 0.0 {
                    col = mix(col, (238, 250, 255), hit * 0.78);
                }

                if vis >= 0.99 {
                    pb.plot(sx, sy, col);
                } else {
                    plot_blend(pb, sx, sy, col, vis * 0.92);
                }
            }
        }
    }

    let frame = BodyFrame { sc, f, d_up, side };
    draw_teeth(pb, view, frame, gape, tglow, hit, false);
    draw_eyes(pb, view, frame, hit);
    draw_stalk_and_bulbs(pb, view, cam, frame);
}

fn draw_teeth(
    pb: &mut PixelBuffer,
    view: &LanternView,
    frame: BodyFrame,
    gape: f32,
    tglow: f32,
    hit: f32,
    overlay_only: bool,
) {
    let BodyFrame { sc, f, d_up, side } = frame;
    let dead = matches!(view.state, LanternState::Death);
    if tglow <= 0.02 && !dead {
        return;
    }
    if dead && (overlay_only || head_visibility(view) <= 0.05) {
        return;
    }

    if side >= 0.45 {
        // Profile view: 5 tall spaced upper needle fangs & 6 protruding underbite lower fangs.
        let jaw_top = lerp(5.2, 11.4, gape);
        let jaw_bot = -lerp(6.4, 14.8, gape);
        for (row_sign, count) in [(1.0_f32, 5usize), (-1.0_f32, 6usize)] {
            for i in 0..count {
                if overlay_only && i >= 4 {
                    continue;
                }
                let t = i as f32 / (count - 1).max(1) as f32;
                let u_front = if row_sign < 0.0 { 13.8 } else { 11.8 };
                let u_base = lerp(u_front, -1.5, t);
                let v_lip = if row_sign > 0.0 { jaw_top } else { jaw_bot };
                let v_base = v_lip - row_sign * (0.9 * t * t);

                let bell = 1.0 - 0.34 * (t - 0.25).abs();
                let mut base_len =
                    lerp(4.2, 10.2, gape) * bell * if row_sign < 0.0 { 1.06 } else { 0.94 };
                if i % 2 == 1 {
                    base_len *= 0.88;
                }
                let u_tip = u_base - lerp(0.5, 1.4, t);
                let v_tip = v_base - row_sign * base_len;

                let front_bright = (1.0 - t * 0.56).clamp(0.26, 1.0) * tglow;
                let (col_base, mut col_tip) = if dead {
                    let fade = clamp01(view.progress * 1.4);
                    let c = mix(TOOTH_DEAD, HIDE_ABYSS, fade);
                    (c, c)
                } else {
                    let tip = if front_bright > 0.72 {
                        mix(
                            TOOTH_GLINT_HI,
                            TOOTH_GLINT_WHITE,
                            (front_bright - 0.72) / 0.28,
                        )
                    } else {
                        mix(TOOTH_GLINT_DIM, TOOTH_GLINT_HI, front_bright / 0.72)
                    };
                    let base = mix(TOOTH_GLINT_DIM, TOOTH_GLINT_MID, front_bright);
                    (base, tip)
                };
                if hit > 0.0 {
                    col_tip = mix(col_tip, (255, 255, 255), hit * 0.65);
                }

                let u_ctrl = u_base + 0.6;
                let v_ctrl = (v_base + v_tip) * 0.5;
                let steps = ((base_len * 1.35).ceil() as usize).max(5);
                let start_s = if overlay_only { steps / 3 } else { 0 };
                let overlay_alpha = clamp01(0.65 + 0.35 * tglow);
                for s in start_s..=steps {
                    let st = s as f32 / steps as f32;
                    let omt = 1.0 - st;
                    let uu = omt * omt * u_base + 2.0 * omt * st * u_ctrl + st * st * u_tip;
                    let vv = omt * omt * v_base + 2.0 * omt * st * v_ctrl + st * st * v_tip;
                    let pt = sc + f * uu + d_up * vv;
                    let c = mix(col_base, col_tip, st);
                    let (ix, iy) = (pt.x.round() as i32, pt.y.round() as i32);
                    if overlay_only {
                        plot_blend(pb, ix, iy, c, overlay_alpha);
                    } else {
                        pb.plot(ix, iy, c);
                    }
                }
            }
        }
    } else {
        // Top-down vertical heading: 7 curved fangs fanning across the front crescent jaw.
        let jaw_hw = lerp(9.0, 14.2, gape);
        let count = 7usize;
        for i in 0..count {
            let vn = -1.0 + 2.0 * (i as f32 / (count - 1) as f32);
            let v_base = vn * (jaw_hw - 1.0);
            let u_base = 14.5 - 2.6 * vn * vn;
            let length = lerp(4.0, 7.6, gape) * (1.0 - 0.25 * vn.abs());
            let u_tip = u_base - length * 0.85;
            let v_tip = v_base * 0.82;
            let front_bright = (1.0 - vn.abs() * 0.42).clamp(0.35, 1.0) * tglow;
            let (col_base, mut col_tip) = if dead {
                let c = mix(TOOTH_DEAD, HIDE_ABYSS, clamp01(view.progress * 1.4));
                (c, c)
            } else {
                let tip = mix(
                    TOOTH_GLINT_HI,
                    TOOTH_GLINT_WHITE,
                    clamp01((front_bright - 0.55) / 0.45),
                );
                let base = mix(TOOTH_GLINT_DIM, TOOTH_GLINT_MID, front_bright);
                (base, tip)
            };
            if hit > 0.0 {
                col_tip = mix(col_tip, (255, 255, 255), hit * 0.65);
            }
            let steps = ((length * 1.3).ceil() as usize).max(4);
            let start_s = if overlay_only { steps / 2 } else { 0 };
            for s in start_s..=steps {
                let st = s as f32 / steps as f32;
                let pt = sc + f * lerp(u_base, u_tip, st) + d_up * lerp(v_base, v_tip, st);
                let c = mix(col_base, col_tip, st);
                pb.plot(pt.x.round() as i32, pt.y.round() as i32, c);
            }
        }
    }
}

fn draw_eyes(pb: &mut PixelBuffer, view: &LanternView, frame: BodyFrame, hit: f32) {
    let BodyFrame { sc, f, d_up, side } = frame;
    let eglow = eye_glow_factor(view);
    if eglow <= 0.04 {
        return;
    }

    let eye_u = lerp(-0.5, 2.8, side);
    let eye_v = lerp(9.5, 11.8, side);
    let mut col_hi = mix(HIDE_ABYSS, EYE_BRIGHT, eglow);
    let col_ring = mix(HIDE_ABYSS, EYE_GOLD, eglow);
    if hit > 0.0 {
        col_hi = mix(col_hi, (255, 255, 255), hit * 0.7);
    }

    let eyes = [
        (sc + f * eye_u + d_up * eye_v, 1.0_f32),
        (
            sc + f * eye_u - d_up * eye_v,
            clamp01((0.62 - side) / 0.44),
        ),
    ];

    for (ep, alpha) in eyes {
        if alpha <= 0.05 {
            continue;
        }
        let ex = ep.x.round() as i32;
        let ey = ep.y.round() as i32;
        for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
            plot_blend(pb, ex + dx, ey + dy, HIDE_ABYSS, 0.75 * alpha);
        }
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            plot_blend(pb, ex + dx, ey + dy, col_hi, alpha);
        }
        for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            plot_blend(pb, ex + dx, ey + dy, col_ring, 0.45 * alpha);
        }
        plot_blend(pb, ex, ey, EYE_PUPIL, alpha);
    }
}

fn draw_bulb_sprite(
    pb: &mut PixelBuffer,
    center: Vec2,
    bulb: &LanternBulbView,
    anim: f32,
    is_feint_tell: bool,
) {
    let eff = bulb_flicker_mult(bulb, anim, is_feint_tell);
    let cx = center.x.round() as i32;
    let cy = center.y.round() as i32;

    if eff <= 0.02 {
        // Extinguished dark bulb bud (`DarkTell` / `DarkLunge`).
        disc_blend(pb, center, 2.2, (14, 24, 30), 0.95);
        pb.plot(cx, cy, (26, 44, 52));
        return;
    }

    if bulb.is_decoy {
        // Phase 2 steady cooler blue-cyan decoy: smooth 5-6 px, no jagged flare.
        disc_blend(pb, center, 3.2, DECOY_HALO, 0.45 * eff.min(1.0));
        disc_blend(pb, center, 2.2, DECOY_RING, 0.92);
        pb.plot(cx, cy, DECOY_CORE);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            pb.plot(cx + dx, cy + dy, DECOY_RING);
        }
        return;
    }

    // Real cyan esca lantern (`6 px` diameter + soft aqua halo + blazing `+` core).
    let halo_r = if bulb.planted {
        4.2
    } else if is_feint_tell {
        4.6
    } else {
        3.8
    };
    disc_blend(pb, center, halo_r, BULB_OUTER, (0.42 * eff).clamp(0.0, 0.72));
    disc_blend(pb, center, 2.8, BULB_RING, (0.85 * eff).clamp(0.25, 1.0));
    let core = mix(BULB_RING, BULB_CORE, eff.clamp(0.35, 1.0));
    pb.plot(cx, cy, core);
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        pb.plot(cx + dx, cy + dy, core);
    }
    if !bulb.planted && eff > 0.6 {
        plot_blend(pb, cx, cy + 3, BULB_RING, 0.45);
    }
}

fn draw_stalk_and_bulbs(pb: &mut PixelBuffer, view: &LanternView, cam: Vec2, frame: BodyFrame) {
    let BodyFrame { f, d_up, side, .. } = frame;
    let brow_s = w2s(brow_anchor(view), cam);
    let is_feint_tell = matches!(view.state, LanternState::FeintTell);

    if let Some(b) = view.real_bulb {
        let bs = w2s(b.pos, cam);
        if !b.planted {
            // Flexible quadratic Bezier fishing-rod arch over the forehead & teeth.
            let dist = brow_s.dist(bs);
            let arch_h = (dist * 0.48).clamp(10.0, 20.0) * (0.45 + 0.55 * side);
            let wobble = (view.anim * 3.1).sin() * 1.4;
            let mid = (brow_s + bs) * 0.5;
            let ctrl = mid + d_up * (arch_h + wobble) + f * 3.0;
            let steps = ((dist * 1.65).ceil() as usize).max(14);
            let gain = clamp01(b.gain);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let omt = 1.0 - t;
                let px = omt * omt * brow_s.x + 2.0 * omt * t * ctrl.x + t * t * bs.x;
                let py = omt * omt * brow_s.y + 2.0 * omt * t * ctrl.y + t * t * bs.y;
                let tip_lit = t * t * gain;
                let col = mix(STALK_DARK, STALK_RIM, 0.25 + 0.75 * tip_lit);
                let (ix, iy) = (px.round() as i32, py.round() as i32);
                pb.plot(ix, iy, col);
                if t < 0.24 {
                    plot_blend(pb, ix, iy + 1, STALK_DARK, 0.85);
                }
            }
        } else if head_visibility(view) > 0.10 {
            // Snapped stalk stump on the brow once the bulb is planted on the seabed.
            let stump_end = brow_s + f * 5.0 + d_up * (5.0 * side + 2.0);
            line_blend(pb, brow_s, stump_end, Stroke::solid(STALK_DARK, 0.90));
            plot_blend(
                pb,
                stump_end.x.round() as i32,
                stump_end.y.round() as i32,
                STALK_RIM,
                0.60,
            );
        }

        draw_bulb_sprite(pb, bs, &b, view.anim, is_feint_tell);
    }

    // Phase 2 floating decoy bulb (drawn at its exact world position, never stalked).
    if let Some(d) = view.decoy {
        let ds = w2s(d.pos, cam);
        draw_bulb_sprite(pb, ds, &d, view.anim, false);
    }
}

// ============================================================================
// 3. Hostile / Post-Lighting Tell Overlay (above lighting & foreground)
// ============================================================================

fn draw_jaw_crescent_outline(
    pb: &mut PixelBuffer,
    center: Vec2,
    facing: Vec2,
    half_extents: (f32, f32),
    stroke: Stroke,
    tooth_col: Rgb,
    live: bool,
) {
    let (half_depth, half_span) = half_extents;
    let ap = facing.perp();
    let arc_steps = 16usize;
    let mut prev_f: Option<Vec2> = None;
    let mut prev_b: Option<Vec2> = None;
    for i in 0..=arc_steps {
        let vn = -1.0 + 2.0 * (i as f32 / arc_steps as f32);
        let uf = half_depth * (1.0 - 0.42 * vn * vn);
        let ub = -half_depth * (1.0 - 0.15 * vn * vn);
        let pf = center + facing * uf + ap * (vn * half_span);
        let pb_pt = center + facing * ub + ap * (vn * half_span);
        if let (Some(af_prev), Some(ab_prev)) = (prev_f, prev_b) {
            let s_f = Stroke::styled(stroke.col, 1.0, stroke.dashed, stroke.phase + i as i32);
            let s_b = Stroke::styled(stroke.col, 0.92, stroke.dashed, stroke.phase + i as i32);
            line_blend(pb, af_prev, pf, s_f);
            line_blend(pb, ab_prev, pb_pt, s_b);
        }
        prev_f = Some(pf);
        prev_b = Some(pb_pt);
    }
    for s in [-1.0_f32, 1.0_f32] {
        let cf = center + facing * (half_depth * 0.58) + ap * (s * half_span);
        let cb = center - facing * (half_depth * 0.85) + ap * (s * half_span);
        line_blend(pb, cb, cf, Stroke::solid(stroke.col, 1.0));
    }
    for k in [-5.6_f32, -2.8, 0.0, 2.8, 5.6] {
        let vn = k / half_span.max(1.0);
        let uf = half_depth * (1.0 - 0.42 * vn * vn);
        let ub = -half_depth * 0.72;
        let base = center + facing * ub + ap * k;
        let tip = center + facing * (uf - 0.5) + ap * (k * 0.92);
        line_blend(pb, base, tip, Stroke::solid(tooth_col, 0.92));
    }
    if live {
        for u_off in [-2.0_f32, 0.0, 2.0] {
            line_blend(
                pb,
                center + facing * u_off - ap * (half_span - 1.0),
                center + facing * u_off + ap * (half_span - 1.0),
                Stroke::solid(TELL_HOT, 0.58),
            );
        }
    }
}

/// Generic fallback boundary draw for any `HazardShape` variant so art always
/// honours the controller's exact simulation footprint.
fn draw_hazard_outline(pb: &mut PixelBuffer, shape: HazardShape, cam: Vec2, stroke: Stroke) {
    match shape {
        HazardShape::Circle { center, radius } => {
            ring_blend(pb, w2s(center, cam), radius, stroke);
        }
        HazardShape::Annulus {
            center,
            inner,
            outer,
        } => {
            let cs = w2s(center, cam);
            ring_blend(pb, cs, outer, stroke);
            ring_blend(pb, cs, inner, stroke);
        }
        HazardShape::Arc {
            center,
            inner,
            outer,
            facing,
            half_angle,
        } => {
            let cs = w2s(center, cam);
            let base = safe_unit(facing, Vec2::new(1.0, 0.0)).angle();
            for sign in [-1.0_f32, 1.0_f32] {
                let d = Vec2::from_angle(base + sign * half_angle);
                line_blend(pb, cs + d * inner, cs + d * outer, stroke);
            }
            let steps = 24usize;
            let mut prev: Option<Vec2> = None;
            for i in 0..=steps {
                let a = base - half_angle + 2.0 * half_angle * (i as f32 / steps as f32);
                let pt = cs + Vec2::from_angle(a) * outer;
                if let Some(p_prev) = prev {
                    line_blend(pb, p_prev, pt, stroke);
                }
                prev = Some(pt);
            }
        }
        HazardShape::Rect {
            center,
            facing,
            half,
            half_perp,
        } => {
            let dir = safe_unit(facing, Vec2::new(1.0, 0.0));
            draw_oriented_rect_outline(pb, w2s(center, cam), dir, half, half_perp, stroke);
        }
        HazardShape::Corridor {
            from,
            to,
            half_width,
        } => {
            let a = w2s(from, cam);
            let b = w2s(to, cam);
            let dir = safe_unit(b - a, Vec2::new(1.0, 0.0));
            let lp = dir.perp() * half_width;
            line_blend(pb, a + lp, b + lp, stroke);
            line_blend(pb, a - lp, b - lp, stroke);
            line_blend(pb, a - lp, a + lp, Stroke::solid(stroke.col, stroke.alpha));
            line_blend(pb, b - lp, b + lp, Stroke::solid(stroke.col, stroke.alpha));
        }
    }
}

/// Post-lighting, post-foreground hostile layer for **The Lantern**.
///
/// Re-stamps self-lit bulb cores and dark-state teeth/eye glints, and draws
/// bright fixed outlines directly from the authoritative simulation footprints
/// (`view.attack` and `view.stamp`) so the fight remains crystal-clear with
/// lighting/FX disabled, through foreground kelp, and at reduced colour depth.
pub fn draw_tell_overlay(pb: &mut PixelBuffer, view: &LanternView, cam: Vec2, _accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let f = unit_facing(view);
    let (side, d_up) = dorsal_frame(f);
    let sc = w2s(view.pos, cam);
    let frame = BodyFrame { sc, f, d_up, side };
    let dash_phase = (view.anim * 10.0) as i32;

    // 1. Self-lit real & decoy bulb cores above foreground / darkness.
    if let Some(b) = view.real_bulb.filter(|b| b.gain > 0.05) {
        let bs = w2s(b.pos, cam);
        let is_feint_tell = matches!(view.state, LanternState::FeintTell);
        let eff = bulb_flicker_mult(&b, view.anim, is_feint_tell);
        let bx = bs.x.round() as i32;
        let by = bs.y.round() as i32;
        let core = mix(BULB_RING, BULB_CORE, eff.clamp(0.40, 1.0));
        pb.plot(bx, by, core);
        let arm_a = (0.85 * eff).clamp(0.35, 1.0);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            plot_blend(pb, bx + dx, by + dy, BULB_RING, arm_a);
        }
        if b.planted {
            // Consistent bright anchor ring + diagonal cradle ticks on planted lamp.
            ring_blend(pb, bs, 5.2, Stroke::solid(TELL_CYAN, 0.68));
            for k in 0..4 {
                let a = k as f32 * FRAC_PI_2 + PI * 0.25;
                let d = Vec2::from_angle(a);
                line_blend(
                    pb,
                    bs + d * 4.5,
                    bs + d * 6.8,
                    Stroke::solid(TELL_CYAN, 0.78),
                );
            }
        }
    }

    if let Some(d) = view.decoy {
        let ds = w2s(d.pos, cam);
        let dx0 = ds.x.round() as i32;
        let dy0 = ds.y.round() as i32;
        // Steady cool decoy core with NO looming jaw outline.
        pb.plot(dx0, dy0, DECOY_CORE);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            plot_blend(pb, dx0 + dx, dy0 + dy, DECOY_RING, 0.80);
        }
    }

    // 2. Self-lit front tooth-rim glints & golden eye in arrival / dark / attack beats.
    if matches!(
        view.state,
        LanternState::ArrivalLure
            | LanternState::DarkTell
            | LanternState::DarkLunge
            | LanternState::FeintTell
            | LanternState::JawSnap
            | LanternState::GapeTell
            | LanternState::GapePull
            | LanternState::GapeStamp
    ) {
        let tglow = tooth_glow_factor(view);
        let hit = clamp01(view.hit_flash);
        if tglow > 0.15 {
            draw_teeth(pb, view, frame, jaw_open_factor(view), tglow, hit, true);
        }
        if matches!(
            view.state,
            LanternState::ArrivalLure | LanternState::DarkTell | LanternState::DarkLunge
        ) {
            draw_eyes(pb, view, frame, hit);
        }
    }

    // 3. Authoritative live/locked attack footprint (`view.attack`) — never reconstructed!
    if let Some(shape) = view.attack {
        match (view.state, shape) {
            (
                LanternState::FeintTell | LanternState::JawSnap,
                HazardShape::Rect {
                    center,
                    facing,
                    half,
                    half_perp,
                },
            ) => {
                let cs = w2s(center, cam);
                let af = safe_unit(facing, f);
                let live = matches!(view.state, LanternState::JawSnap);
                let col = if live {
                    TELL_HOT
                } else if view.attack_locked {
                    TELL_WHITE
                } else {
                    TELL_AMBER
                };
                let tooth_col = if live { TELL_WHITE } else { TELL_CYAN };
                let dashed = if live || view.attack_locked { 0 } else { 2 };
                let stroke = Stroke::styled(col, 1.0, dashed, dash_phase);
                draw_jaw_crescent_outline(pb, cs, af, (half, half_perp), stroke, tooth_col, live);
            }
            (
                LanternState::DecoyLeapTell | LanternState::DecoyLeap,
                HazardShape::Rect {
                    center,
                    facing,
                    half,
                    half_perp,
                },
            ) => {
                let ld = safe_unit(facing, f);
                let lp = ld.perp();
                let start_s = w2s(center - ld * half, cam);
                let end_s = w2s(center + ld * half, cam);
                let len = half * 2.0;
                if len > 2.0 {
                    let live = matches!(view.state, LanternState::DecoyLeap);
                    let col = if live { TELL_HOT } else { TELL_AMBER };
                    let rail = Stroke::styled(col, 1.0, if live { 0 } else { 2 }, dash_phase);
                    let cap = Stroke::solid(col, 1.0);
                    line_blend(pb, start_s + lp * half_perp, end_s + lp * half_perp, rail);
                    line_blend(pb, start_s - lp * half_perp, end_s - lp * half_perp, rail);
                    line_blend(pb, start_s - lp * half_perp, start_s + lp * half_perp, cap);
                    line_blend(pb, end_s - lp * half_perp, end_s + lp * half_perp, cap);
                    let chev = Stroke::solid(if live { TELL_WHITE } else { TELL_CYAN }, 0.90);
                    let chev_hw = (half_perp - 1.5).max(1.5);
                    let mut d_cur = 10.0_f32;
                    while d_cur < len - 4.0 {
                        let q = start_s + ld * d_cur;
                        line_blend(pb, q - ld * 4.0 + lp * chev_hw, q, chev);
                        line_blend(pb, q - ld * 4.0 - lp * chev_hw, q, chev);
                        d_cur += 14.0;
                    }
                }
            }
            (
                LanternState::DecoyLeapTell | LanternState::DecoyLeap,
                HazardShape::Corridor {
                    from,
                    to,
                    half_width,
                },
            ) => {
                let start_s = w2s(from, cam);
                let end_s = w2s(to, cam);
                let axis = end_s - start_s;
                let len = axis.len();
                if len > 2.0 {
                    let ld = axis.normalized();
                    let lp = ld.perp();
                    let live = matches!(view.state, LanternState::DecoyLeap);
                    let col = if live { TELL_HOT } else { TELL_AMBER };
                    let rail = Stroke::styled(col, 1.0, if live { 0 } else { 2 }, dash_phase);
                    let cap = Stroke::solid(col, 1.0);
                    line_blend(pb, start_s + lp * half_width, end_s + lp * half_width, rail);
                    line_blend(pb, start_s - lp * half_width, end_s - lp * half_width, rail);
                    line_blend(pb, start_s - lp * half_width, start_s + lp * half_width, cap);
                    line_blend(pb, end_s - lp * half_width, end_s + lp * half_width, cap);
                    let chev = Stroke::solid(if live { TELL_WHITE } else { TELL_CYAN }, 0.90);
                    let chev_hw = (half_width - 1.5).max(1.5);
                    let mut d_cur = 10.0_f32;
                    while d_cur < len - 4.0 {
                        let q = start_s + ld * d_cur;
                        line_blend(pb, q - ld * 4.0 + lp * chev_hw, q, chev);
                        line_blend(pb, q - ld * 4.0 - lp * chev_hw, q, chev);
                        d_cur += 14.0;
                    }
                }
            }
            (
                LanternState::DarkTell | LanternState::DarkLunge,
                HazardShape::Rect {
                    center,
                    facing,
                    half,
                    half_perp,
                },
            ) => {
                let af = safe_unit(facing, f);
                let center_s = w2s(center, cam);
                let origin_s = w2s(center - af * half, cam);
                let live = matches!(view.state, LanternState::DarkLunge);
                let col = if live {
                    TELL_HOT
                } else if view.attack_locked {
                    TELL_WHITE
                } else {
                    TELL_AMBER
                };
                let dashed = if live || view.attack_locked { 0 } else { 2 };
                let stroke = Stroke::styled(col, 1.0, dashed, dash_phase);
                draw_oriented_rect_outline(pb, center_s, af, half, half_perp, stroke);
                let ap = af.perp();
                let tick = Stroke::solid(col, 0.85);
                let len = half * 2.0;
                let outer_t = (half_perp - 0.5).max(1.0);
                let inner_t = (half_perp - 3.5).max(0.5);
                let mut dist = 11.0_f32;
                while dist < len - 5.0 {
                    let q = origin_s + af * dist;
                    line_blend(pb, q + ap * outer_t, q + ap * inner_t, tick);
                    line_blend(pb, q - ap * outer_t, q - ap * inner_t, tick);
                    dist += 11.0;
                }
            }
            (
                LanternState::GapeTell | LanternState::GapePull,
                HazardShape::Arc {
                    center,
                    inner,
                    outer,
                    facing,
                    half_angle,
                },
            ) => {
                let ms = w2s(center, cam);
                let af = safe_unit(facing, f);
                let base_ang = af.angle();
                let pulling = matches!(view.state, LanternState::GapePull);
                let edge_col = if pulling {
                    TELL_CYAN
                } else if view.attack_locked {
                    TELL_WHITE
                } else {
                    TELL_AMBER
                };
                let dashed = if pulling || view.attack_locked { 0 } else { 2 };
                let ray_stroke = Stroke::styled(edge_col, 1.0, dashed, dash_phase);
                let r_start = inner.max(10.0).min(outer);
                // Outer cone rays (clean negative space inside!).
                for sign in [-1.0_f32, 1.0_f32] {
                    let d = Vec2::from_angle(base_ang + sign * half_angle);
                    line_blend(pb, ms + d * r_start, ms + d * outer, ray_stroke);
                }
                // Outer range arc.
                let arc_steps = 28usize;
                let mut prev: Option<Vec2> = None;
                for i in 0..=arc_steps {
                    let a = base_ang - half_angle
                        + 2.0 * half_angle * (i as f32 / arc_steps as f32);
                    let pt = ms + Vec2::from_angle(a) * outer;
                    if let Some(p_prev) = prev {
                        let arc_s = Stroke::styled(edge_col, 0.95, dashed, dash_phase + i as i32);
                        line_blend(pb, p_prev, pt, arc_s);
                    }
                    prev = Some(pt);
                }
                if pulling {
                    // Sparse inward suction current streaks (keeping negative space open).
                    let r_lo = (inner + 24.0).min(outer);
                    let r_hi = (outer - 6.0).max(r_lo);
                    for wave in 0..3 {
                        let frac = 1.0 - (view.anim * 1.4 + wave as f32 / 3.0).rem_euclid(1.0);
                        let r_cur = lerp(r_lo, r_hi, frac);
                        let w_alpha = 0.78 * (frac * PI).sin();
                        let streak = Stroke::solid(TELL_CYAN, w_alpha);
                        for ray_idx in [-0.62_f32, 0.0, 0.62] {
                            let d = Vec2::from_angle(base_ang + ray_idx * half_angle);
                            let p0 = ms + d * (r_cur + 6.0).min(outer);
                            let p1 = ms + d * (r_cur - 3.0).max(18.0);
                            line_blend(pb, p0, p1, streak);
                        }
                    }
                } else {
                    let tick = Stroke::solid(edge_col, 0.80);
                    for r_tick in [outer * 0.53, outer * 0.80] {
                        for sign in [-1.0_f32, 1.0_f32] {
                            let d = Vec2::from_angle(base_ang + sign * half_angle);
                            let inward = Vec2::from_angle(base_ang + sign * (half_angle - 0.12));
                            line_blend(pb, ms + d * r_tick, ms + inward * r_tick, tick);
                        }
                    }
                }
            }
            (_, other) => {
                let live = view.state.is_active_strike();
                let col = if live {
                    TELL_HOT
                } else if view.attack_locked {
                    TELL_WHITE
                } else {
                    TELL_AMBER
                };
                let dashed = if live || view.attack_locked { 0 } else { 2 };
                draw_hazard_outline(pb, other, cam, Stroke::styled(col, 1.0, dashed, dash_phase));
            }
        }
    }

    // 4. Authoritative Gape mouth stamp footprint (`view.stamp`) — never reconstructed!
    if let Some(stamp_shape) = view.stamp {
        let stamp_live = matches!(view.state, LanternState::GapeStamp);
        let stamp_pull = matches!(view.state, LanternState::GapePull);
        let stamp_col = if stamp_live || stamp_pull {
            TELL_HOT
        } else {
            TELL_AMBER
        };
        let stamp_dashed = if stamp_live || stamp_pull || view.attack_locked {
            0
        } else {
            2
        };
        let box_stroke = Stroke::styled(stamp_col, 1.0, stamp_dashed, dash_phase);

        if let HazardShape::Rect {
            center,
            facing,
            half,
            half_perp,
        } = stamp_shape
        {
            let stamp_s = w2s(center, cam);
            let stamp_f = safe_unit(facing, f);
            let ap = stamp_f.perp();
            draw_oriented_rect_outline(pb, stamp_s, stamp_f, half, half_perp, box_stroke);
            if stamp_live {
                let fang = Stroke::solid(TELL_WHITE, 0.88);
                let reach_u = (half - 1.5).max(2.0);
                let gap_u = (half * 0.21).max(1.5);
                let span_step = (half_perp * 0.33).max(2.0);
                for idx in -2i32..=2 {
                    let k = idx as f32 * span_step;
                    line_blend(
                        pb,
                        stamp_s - stamp_f * reach_u + ap * k,
                        stamp_s - stamp_f * gap_u + ap * k,
                        fang,
                    );
                    line_blend(
                        pb,
                        stamp_s + stamp_f * reach_u + ap * k,
                        stamp_s + stamp_f * gap_u + ap * k,
                        fang,
                    );
                }
                let cross = Stroke::solid(TELL_HOT, 0.65);
                let cu = (half - 1.0).max(1.0);
                let cv = (half_perp - 1.0).max(1.0);
                line_blend(
                    pb,
                    stamp_s - stamp_f * cu - ap * cv,
                    stamp_s + stamp_f * cu + ap * cv,
                    cross,
                );
                line_blend(
                    pb,
                    stamp_s - stamp_f * cu + ap * cv,
                    stamp_s + stamp_f * cu - ap * cv,
                    cross,
                );
                disc_blend(pb, stamp_s, half.min(half_perp) * 0.62, TELL_HOT, 0.30);
            } else if stamp_pull {
                let bracket = Stroke::solid(TELL_WHITE, 0.90);
                let cu = (half - 1.0).max(1.0);
                let cv = (half_perp - 1.0).max(1.0);
                let leg = half.min(half_perp).min(4.0);
                for su in [-1.0_f32, 1.0] {
                    for sv in [-1.0_f32, 1.0] {
                        let corner = stamp_s + stamp_f * (su * cu) + ap * (sv * cv);
                        line_blend(pb, corner, corner - stamp_f * (su * leg), bracket);
                        line_blend(pb, corner, corner - ap * (sv * leg), bracket);
                    }
                }
            }
        } else {
            draw_hazard_outline(pb, stamp_shape, cam, box_stroke);
        }
    }

    // 5. Exposed flank punish chevrons during recovery windows.
    if matches!(
        view.state,
        LanternState::FeintRecover | LanternState::GapeRecover
    ) {
        let pulse = 0.5 + 0.5 * (view.anim * 8.0).sin();
        let chev = Stroke::solid(FLANK_GOLD, 0.65 + 0.25 * pulse);
        for sign in [-1.0_f32, 1.0] {
            let flank = sc - f * 5.0 + d_up * (sign * (HEAD_RADIUS + 3.0));
            let in_d = -(d_up * sign);
            line_blend(pb, flank + f * 3.0 - in_d * 2.0, flank + in_d * 1.5, chev);
            line_blend(pb, flank - f * 3.0 - in_d * 2.0, flank + in_d * 1.5, chev);
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_STATES: [LanternState; 21] = [
        LanternState::ArrivalLure,
        LanternState::Swim,
        LanternState::FeintApproach,
        LanternState::FeintTell,
        LanternState::JawSnap,
        LanternState::FeintRecover,
        LanternState::DarkSwim,
        LanternState::DecoyLeapTell,
        LanternState::DecoyLeap,
        LanternState::DecoyRecover,
        LanternState::DarkTell,
        LanternState::DarkLunge,
        LanternState::DarkRecover,
        LanternState::DropLamp,
        LanternState::GapeSwim,
        LanternState::GapeTell,
        LanternState::GapePull,
        LanternState::GapeStamp,
        LanternState::GapeRecover,
        LanternState::PhaseShift,
        LanternState::Death,
    ];

    fn sample_view(pos: Vec2, angle: f32, state: LanternState, progress: f32) -> LanternView {
        let facing = Vec2::from_angle(angle);
        let planted = matches!(
            state,
            LanternState::DropLamp
                | LanternState::GapeSwim
                | LanternState::GapeTell
                | LanternState::GapePull
                | LanternState::GapeStamp
                | LanternState::GapeRecover
                | LanternState::Death
        );
        let extinguished = matches!(state, LanternState::DarkTell | LanternState::DarkLunge);
        let bulb_pos = if planted {
            pos + facing * 26.0 + Vec2::new(0.0, 18.0)
        } else {
            pos + facing * 28.0
        };
        let mouth = pos + facing * 14.0;
        let (attack, stamp) = match state {
            LanternState::FeintTell | LanternState::JawSnap => (
                Some(HazardShape::Rect {
                    center: bulb_pos - facing * JAW_CRESCENT_BACK,
                    facing,
                    half: JAW_CRESCENT_WIDTH * 0.5,
                    half_perp: JAW_CRESCENT_LEN * 0.5,
                }),
                None,
            ),
            LanternState::DecoyLeapTell | LanternState::DecoyLeap => (
                Some(HazardShape::Rect {
                    center: pos + facing * 36.0,
                    facing,
                    half: 36.0,
                    half_perp: DECOY_LEAP_WIDTH * 0.5,
                }),
                None,
            ),
            LanternState::DarkTell | LanternState::DarkLunge => (
                Some(HazardShape::Rect {
                    center: pos + facing * (DARK_LUNGE_LEN * 0.5),
                    facing,
                    half: DARK_LUNGE_LEN * 0.5,
                    half_perp: DARK_LUNGE_WIDTH * 0.5,
                }),
                None,
            ),
            LanternState::GapeTell | LanternState::GapePull => (
                Some(HazardShape::Arc {
                    center: mouth,
                    inner: 0.0,
                    outer: GAPE_CONE_RANGE,
                    facing,
                    half_angle: GAPE_CONE_HALF_ANGLE,
                }),
                Some(HazardShape::Rect {
                    center: mouth,
                    facing,
                    half: GAPE_STAMP_SIZE * 0.5,
                    half_perp: GAPE_STAMP_SIZE * 0.5,
                }),
            ),
            LanternState::GapeStamp => (
                None,
                Some(HazardShape::Rect {
                    center: mouth,
                    facing,
                    half: GAPE_STAMP_SIZE * 0.5,
                    half_perp: GAPE_STAMP_SIZE * 0.5,
                }),
            ),
            _ => (None, None),
        };
        LanternView {
            pos,
            facing,
            state,
            progress,
            anim: 1.25,
            hit_flash: 0.0,
            hp_frac: 0.8,
            phase: if planted { 3 } else { 1 },
            attack_origin: mouth,
            attack_target: pos + facing * 74.0,
            attack_facing: facing,
            attack_locked: progress >= 0.5,
            attack,
            stamp,
            real_bulb: Some(LanternBulbView {
                pos: bulb_pos,
                radius: if extinguished {
                    0.0
                } else if planted {
                    BULB_PLANTED_LIGHT_RADIUS
                } else {
                    BULB_LIGHT_RADIUS
                },
                gain: if extinguished { 0.0 } else { 1.0 },
                flicker: 0.0,
                is_decoy: false,
                planted,
            }),
            decoy: None,
        }
    }

    #[test]
    fn spec_constants_match_outfall_plan() {
        assert_eq!(HEAD_RADIUS, 18.0);
        assert_eq!(BODY_RADIUS, 18.0);
        assert!((80.0..=96.0).contains(&BODY_LENGTH));
        const { assert!(CULL_MARGIN >= 108.0) };
        assert_eq!(BOSS_BASE_HP, 1600.0);
        assert_eq!(BOSS_HP_SCALE, 1.25);
        assert_eq!(CONTACT_DAMAGE, 8.0);
        assert_eq!(PHASE2_HP_FRAC, 0.65);
        assert_eq!(PHASE3_HP_FRAC, 0.35);
        assert_eq!(JAW_CRESCENT_LEN, 16.0);
        assert_eq!(JAW_CRESCENT_WIDTH, 8.0);
        assert_eq!(JAW_CRESCENT_BACK, 12.0);
        assert_eq!(DARK_LUNGE_LEN, 56.0);
        assert_eq!(DARK_LUNGE_WIDTH, 12.0);
        assert_eq!(DECOY_LEAP_WIDTH, 12.0);
        assert_eq!(GAPE_CONE_RANGE, 90.0);
        assert_eq!(GAPE_CONE_TOTAL_DEG, 50.0);
        assert_eq!(GAPE_STAMP_SIZE, 24.0);
        assert_eq!(DEATH_DURATION, 2.4);
        assert!(LanternState::Swim.is_neutral());
        assert!(LanternState::DarkSwim.is_neutral());
        assert!(LanternState::GapeSwim.is_neutral());
        assert!(LanternState::FeintRecover.is_recovery());
        assert!(LanternState::GapeRecover.is_recovery());
        assert!(!has_body_contact(LanternState::ArrivalLure));
        assert!(!has_body_contact(LanternState::Death));
        assert!(has_body_contact(LanternState::Swim));
    }

    #[test]
    fn all_states_render_deterministically_across_headings() {
        const BG: Rgb = (10, 18, 24);
        const ACCENT: Rgb = (94, 244, 255);
        for &state in &ALL_STATES {
            for step in 0..8 {
                let angle = step as f32 * TAU / 8.0;
                let view = sample_view(Vec2::new(80.0, 60.0), angle, state, 0.55);
                let mut a = PixelBuffer::new(160, 120);
                let mut b = PixelBuffer::new(160, 120);
                a.clear(BG);
                b.clear(BG);
                draw_ground(&mut a, &view, Vec2::ZERO, ACCENT);
                draw(&mut a, &view, Vec2::ZERO, ACCENT);
                draw_tell_overlay(&mut a, &view, Vec2::ZERO, ACCENT);
                draw_ground(&mut b, &view, Vec2::ZERO, ACCENT);
                draw(&mut b, &view, Vec2::ZERO, ACCENT);
                draw_tell_overlay(&mut b, &view, Vec2::ZERO, ACCENT);
                assert_eq!(a.pixels(), b.pixels(), "non-deterministic render for {state:?}");
                let changed = a.pixels().iter().filter(|&&p| p != BG).count();
                assert!(changed > 20, "state {state:?} at step {step} drew too few pixels ({changed})");
            }
        }
    }

    #[test]
    fn arrival_lure_paints_self_lit_bulb_teeth_and_golden_eye() {
        const BG: Rgb = (4, 8, 12);
        let mut pb = PixelBuffer::new(120, 80);
        pb.clear(BG);
        let view = sample_view(Vec2::new(82.0, 42.0), PI, LanternState::ArrivalLure, 0.30);
        draw_ground(&mut pb, &view, Vec2::ZERO, (94, 244, 255));
        draw(&mut pb, &view, Vec2::ZERO, (94, 244, 255));

        let bulb = view.real_bulb.unwrap().pos;
        let bc = pb.pixel_at(bulb.x.round() as usize, bulb.y.round() as usize);
        assert!(
            bc.1 >= 232 && bc.2 >= 232,
            "real lure core must be blazing cyan-white (>=232): {bc:?}"
        );

        let mut self_lit_teeth = 0usize;
        let mut golden_eye = 0usize;
        for y in 20..65usize {
            for x in 60..92usize {
                let (r, g, b) = pb.pixel_at(x, y);
                if g >= 160 && b >= 180 && r <= g {
                    self_lit_teeth += 1;
                }
                if r >= 180 && g >= 150 && b < 130 {
                    golden_eye += 1;
                }
            }
        }
        assert!(self_lit_teeth >= 12, "arrival teeth too sparse ({self_lit_teeth})");
        assert!(golden_eye >= 2, "golden eye missing ({golden_eye})");
    }

    #[test]
    fn revealed_body_spans_80_to_96_px_and_crops_cleanly_at_57x20() {
        const BG: Rgb = (8, 16, 22);
        let mut pb = PixelBuffer::new(180, 100);
        pb.clear(BG);
        let mut view = sample_view(Vec2::new(60.0, 50.0), PI, LanternState::Swim, 0.4);
        // Hide bulb so we measure pure body silhouette extent (snout to caudal tail tip).
        view.real_bulb = None;
        draw(&mut pb, &view, Vec2::ZERO, (94, 244, 255));
        let mut min_x = pb.w;
        let mut max_x = 0usize;
        for y in 0..pb.h {
            for x in 0..pb.w {
                if pb.pixel_at(x, y) != BG {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
        }
        let span = max_x.saturating_sub(min_x) + 1;
        assert!(
            (80..=96).contains(&span),
            "visible body extent must be 80..=96 px, measured {span}"
        );

        // Narrow 57x20 terminal (57x40 pixel grid): tail crops cleanly off right edge,
        // head/teeth/lure stay bright and unclipped on screen.
        let mut narrow = PixelBuffer::new(57, 40);
        narrow.clear(BG);
        let v_narrow = sample_view(Vec2::new(40.0, 20.0), PI, LanternState::FeintTell, 0.75);
        draw_ground(&mut narrow, &v_narrow, Vec2::ZERO, (94, 244, 255));
        draw(&mut narrow, &v_narrow, Vec2::ZERO, (94, 244, 255));
        draw_tell_overlay(&mut narrow, &v_narrow, Vec2::ZERO, (94, 244, 255));
        let bright = narrow
            .pixels()
            .iter()
            .filter(|&&(r, g, b)| r.max(g).max(b) >= 200)
            .count();
        assert!(bright >= 20, "narrow 57x40 view lost bright lure/jaw/teeth ({bright})");
    }

    #[test]
    fn real_and_decoy_bulbs_draw_at_exact_positions_and_only_real_has_jaw_crescent() {
        const BG: Rgb = (6, 12, 18);
        let mut pb = PixelBuffer::new(160, 100);
        pb.clear(BG);
        let real_pos = Vec2::new(66.0, 52.0);
        let decoy_pos = Vec2::new(44.0, 22.0);
        let mut view = sample_view(Vec2::new(106.0, 52.0), PI, LanternState::FeintTell, 0.75);
        view.phase = 2;
        view.attack_locked = true;
        view.attack = Some(HazardShape::Rect {
            center: Vec2::new(78.0, 52.0),
            facing: Vec2::new(-1.0, 0.0),
            half: JAW_CRESCENT_WIDTH * 0.5,
            half_perp: JAW_CRESCENT_LEN * 0.5,
        });
        view.real_bulb = Some(LanternBulbView {
            pos: real_pos,
            radius: BULB_LIGHT_RADIUS,
            gain: 1.0,
            flicker: 0.0,
            is_decoy: false,
            planted: false,
        });
        view.decoy = Some(LanternBulbView {
            pos: decoy_pos,
            radius: DECOY_LIGHT_RADIUS,
            gain: 1.0,
            flicker: 0.0,
            is_decoy: true,
            planted: false,
        });
        draw_tell_overlay(&mut pb, &view, Vec2::ZERO, (94, 244, 255));

        let r_px = pb.pixel_at(real_pos.x as usize, real_pos.y as usize);
        let d_px = pb.pixel_at(decoy_pos.x as usize, decoy_pos.y as usize);
        assert!(r_px.2 >= 220, "real bulb missing at exact pos: {r_px:?}");
        assert!(d_px.2 >= 220, "decoy bulb missing at exact pos: {d_px:?}");

        // 12 px behind real bulb along facing (-1, 0) => x = 78, y = 52 has bright crescent outline;
        // 12 px behind decoy (x = 56, y = 22) has NO jaw outline.
        let mut real_jaw_px = 0usize;
        let mut decoy_jaw_px = 0usize;
        for dy in -8i32..=8 {
            for dx in -4i32..=4 {
                if pb.pixel_at((78 + dx) as usize, (52 + dy) as usize) != BG {
                    real_jaw_px += 1;
                }
                if pb.pixel_at((56 + dx) as usize, (22 + dy) as usize) != BG {
                    decoy_jaw_px += 1;
                }
            }
        }
        assert!(real_jaw_px >= 20, "real bulb jaw crescent missing ({real_jaw_px})");
        assert_eq!(decoy_jaw_px, 0, "decoy must not draw a jaw crescent ({decoy_jaw_px})");
    }

    #[test]
    fn dark_tell_remains_visible_with_bulb_extinguished_and_no_lighting() {
        const BG: Rgb = (4, 8, 12);
        let mut pb = PixelBuffer::new(160, 100);
        pb.clear(BG);
        let mut view = sample_view(Vec2::new(110.0, 50.0), PI, LanternState::DarkTell, 0.70);
        view.attack_origin = Vec2::new(110.0, 50.0);
        view.attack_locked = true;
        draw(&mut pb, &view, Vec2::ZERO, (94, 244, 255));
        draw_tell_overlay(&mut pb, &view, Vec2::ZERO, (94, 244, 255));
        // 56x12 trench spans x in [54..110], y in [44..56].
        let mut trench_bright = 0usize;
        for y in 43..=57usize {
            for x in 53..=111usize {
                let (r, g, b) = pb.pixel_at(x, y);
                if r.max(g).max(b) >= 220 {
                    trench_bright += 1;
                }
            }
        }
        assert!(trench_bright >= 40, "DarkTell 56x12 trench must stay bright at zero FX ({trench_bright})");
    }

    #[test]
    fn gape_cone_preserves_negative_space_and_stamp_fills_mouth_box() {
        const BG: Rgb = (6, 12, 18);
        let mut pb_pull = PixelBuffer::new(180, 120);
        pb_pull.clear(BG);
        let mut v_pull = sample_view(Vec2::new(140.0, 60.0), PI, LanternState::GapePull, 0.5);
        v_pull.attack_origin = Vec2::new(126.0, 60.0);
        v_pull.attack_locked = true;
        draw_tell_overlay(&mut pb_pull, &v_pull, Vec2::ZERO, (94, 244, 255));

        // Sample the middle interior of the 90 px cone (x in 52..100, y in 48..72):
        // must preserve >= 70% negative space so player/gems stay readable!
        let mut total = 0usize;
        let mut open = 0usize;
        for y in 48..72usize {
            for x in 52..100usize {
                total += 1;
                if pb_pull.pixel_at(x, y) == BG {
                    open += 1;
                }
            }
        }
        assert!(
            open * 100 >= total * 70,
            "Gape cone interior must preserve readable negative space ({open}/{total} open)"
        );

        // GapeStamp fills the 24x24 mouth box centred on mouth (126, 60) => x in [114..=138], y in [48..=72].
        let mut pb_stamp = PixelBuffer::new(180, 120);
        pb_stamp.clear(BG);
        let mut v_stamp = v_pull;
        v_stamp.state = LanternState::GapeStamp;
        v_stamp.attack = None;
        draw_tell_overlay(&mut pb_stamp, &v_stamp, Vec2::ZERO, (94, 244, 255));
        let mut hot = 0usize;
        for y in 48..=72usize {
            for x in 114..=138usize {
                let (r, g, _) = pb_stamp.pixel_at(x, y);
                if r >= 200 && r > g {
                    hot += 1;
                }
            }
        }
        assert!(hot >= 80, "GapeStamp 24x24 box should be clearly marked hot ({hot})");
    }

    #[test]
    fn tell_overlay_draws_exact_simulation_attack_and_stamp_without_reconstructing() {
        const BG: Rgb = (6, 12, 18);
        // 1. With attack = None and stamp = None, FeintTell must NOT reconstruct a jaw behind the bulb.
        let mut pb_none = PixelBuffer::new(160, 100);
        pb_none.clear(BG);
        let mut v_none = sample_view(Vec2::new(110.0, 50.0), PI, LanternState::FeintTell, 0.75);
        v_none.attack = None;
        v_none.stamp = None;
        v_none.real_bulb = Some(LanternBulbView {
            pos: Vec2::new(66.0, 50.0),
            radius: BULB_LIGHT_RADIUS,
            gain: 1.0,
            flicker: FRAC_PI_2,
            is_decoy: false,
            planted: false,
        });
        draw_tell_overlay(&mut pb_none, &v_none, Vec2::ZERO, (94, 244, 255));
        let mut phantom = 0usize;
        for dy in -6i32..=6 {
            for dx in -3i32..=3 {
                if pb_none.pixel_at((78 + dx) as usize, (50 + dy) as usize) != BG {
                    phantom += 1;
                }
            }
        }
        assert_eq!(phantom, 0, "art must not reconstruct a jaw when view.attack is None");

        // 2. Explicit simulation shapes draw at their exact coordinates (decoupled from view.pos).
        let mut pb_exact = PixelBuffer::new(160, 100);
        pb_exact.clear(BG);
        let mut v_exact = sample_view(Vec2::new(140.0, 50.0), PI, LanternState::GapeTell, 0.8);
        v_exact.attack_locked = true;
        v_exact.real_bulb = None;
        v_exact.attack = Some(HazardShape::Rect {
            center: Vec2::new(36.0, 26.0),
            facing: Vec2::new(1.0, 0.0),
            half: 10.0,
            half_perp: 6.0,
        });
        v_exact.stamp = Some(HazardShape::Rect {
            center: Vec2::new(84.0, 74.0),
            facing: Vec2::new(1.0, 0.0),
            half: 12.0,
            half_perp: 12.0,
        });
        draw_tell_overlay(&mut pb_exact, &v_exact, Vec2::ZERO, (94, 244, 255));
        let mut at_attack = 0usize;
        for y in 19..=33usize {
            for x in 25..=47usize {
                if pb_exact.pixel_at(x, y) != BG {
                    at_attack += 1;
                }
            }
        }
        let mut at_stamp = 0usize;
        for y in 61..=87usize {
            for x in 71..=97usize {
                if pb_exact.pixel_at(x, y) != BG {
                    at_stamp += 1;
                }
            }
        }
        assert!(at_attack >= 24, "view.attack must draw at exact shape coords ({at_attack})");
        assert!(at_stamp >= 32, "view.stamp must draw at exact shape coords ({at_stamp})");
    }

    #[test]
    fn death_darkens_head_first_while_planted_bulb_stays_lit() {
        const BG: Rgb = (4, 8, 12);
        let render = |prog: f32| {
            let mut pb = PixelBuffer::new(140, 90);
            pb.clear(BG);
            let mut v = sample_view(Vec2::new(90.0, 45.0), PI, LanternState::Death, prog);
            v.real_bulb = Some(LanternBulbView {
                pos: Vec2::new(50.0, 55.0),
                radius: BULB_PLANTED_LIGHT_RADIUS,
                gain: 0.90,
                flicker: 0.0,
                is_decoy: false,
                planted: true,
            });
            draw_ground(&mut pb, &v, Vec2::ZERO, (94, 244, 255));
            draw(&mut pb, &v, Vec2::ZERO, (94, 244, 255));
            draw_tell_overlay(&mut pb, &v, Vec2::ZERO, (94, 244, 255));
            pb
        };
        let early = render(0.05);
        let late = render(0.95);
        let head_luma = |pb: &PixelBuffer| -> u32 {
            let mut sum = 0u32;
            for y in 26..64usize {
                for x in 72..108usize {
                    let (r, g, b) = pb.pixel_at(x, y);
                    sum += r as u32 + g as u32 + b as u32;
                }
            }
            sum
        };
        assert!(
            head_luma(&early) > head_luma(&late) + 2000,
            "head should go dark during Death (early {}, late {})",
            head_luma(&early),
            head_luma(&late)
        );
        let late_bulb = late.pixel_at(50, 55);
        assert!(
            late_bulb.1 >= 210 && late_bulb.2 >= 220,
            "planted bulb must stay glowing through Death clear beat: {late_bulb:?}"
        );
    }

    #[test]
    fn hit_flash_brightens_body_and_ground_shadow_uses_pen_zero() {
        const BG: Rgb = (10, 18, 24);
        let mut a = PixelBuffer::new(140, 100);
        let mut b = PixelBuffer::new(140, 100);
        a.set_materials(true);
        a.set_pen(0);
        a.clear(BG);
        b.clear(BG);

        let v0 = sample_view(Vec2::new(80.0, 50.0), PI, LanternState::Swim, 0.3);
        let mut v1 = v0;
        v1.hit_flash = 0.9;

        a.set_pen(42);
        draw_ground(&mut a, &v0, Vec2::ZERO, (94, 244, 255));
        assert_eq!(a.pen(), 42, "draw_ground must restore caller pen");
        draw(&mut a, &v0, Vec2::ZERO, (94, 244, 255));

        draw_ground(&mut b, &v1, Vec2::ZERO, (94, 244, 255));
        draw(&mut b, &v1, Vec2::ZERO, (94, 244, 255));

        let diff = a
            .pixels()
            .iter()
            .zip(b.pixels())
            .filter(|(p, q)| p != q)
            .count();
        assert!(diff > 100, "hit_flash must visibly brighten the body (diff {diff})");

        let tagged_body = a.mat().iter().filter(|&&m| m == 42).count();
        assert!(tagged_body > 400, "body pixels should inherit caller material pen ({tagged_body})");
    }
}
