//! Art and gameplay specification for **The Mangle** (Laundry boss).
//!
//! # Rat's-Eye Concept
//! A cast-iron laundry mangle on four little iron feet, seen from above at the
//! rat's scale: two long **candy-striped rollers** set in a pair of heavy iron
//! cheeks, a brass wringer screw across the top, and a loose wooden **crank**
//! hanging off the right cheek. A frayed towel is forever feeding into the nip;
//! when the machine jams, the towel bulges out and the crank swings loose and
//! glows. In its last quarter the drive belt has torn and hangs off the frame.
//!
//! The rollers are the body: the intake lane runs out of the **nip** in front
//! of them, and the feed direction is the machine's facing. Everything the rat
//! has to read — the lane, the bite strip, the towel rail, the exposed crank —
//! is drawn on the floor in the telegraph pass.
//!
//! # Rendering Layers
//! 1. [`draw_ground`] — call after the floor and stains, before props/enemies/
//!    the rat. Draws the machine's drop shadow and rubber intake mat, the
//!    intake lane and its bite strip, the wring-out rail, the cable/spring
//!    ticks of a jam, the crank's weak-point gather ring, the phase-change
//!    stamp and its dust, and the diegetic ground HP arc.
//! 2. [`draw`] — call in the entity pass. Draws the frame, rollers, screw,
//!    crank, towel jam and torn belt.
//!
//! # Combat & Hitbox Contract
//! [`hit_shapes`] is the binding geometry: the **body box** covers the frame
//! and both rollers, the **crank circle** is the loose crank at the right
//! flank, and both are grounded except while the machine is slumped into its
//! final jam (the rollers are slack and slip past ground weapons; the crank and
//! feet still take hits). [`intake_lane`] / [`bite_strip`] / [`wring_lane`] are
//! the exact world rectangles the controller turns into hazards, so the paint
//! and the damage are one geometry.
//!
//! # Move-set numbers (this file is the spec)
//!
//! | state | duration | tell → window | damage |
//! |-------|----------|---------------|--------|
//! | `Idle` | 0.55 / 0.45 / 0.40 s | — | body contact 8 |
//! | `Intake { second }` | 2.40 s (P1/P2), 1.70 s (P3) | lane 1.10 s (P3 0.90) → pull 1.00 s (P3 0.50) → bite 0.10 s | 16 bite |
//! | `WringOut` | 2.00 s | two crank clicks 0.90 s → towel 0.70 s | 18 towel |
//! | `Jam` | 1.50 s | harmless; crank exposed ×1.5 | 0 |
//! | `FinalJam` | 2.20 s | harmless; crank exposed ×1.5, rollers slack | 0 |
//! | `PhaseShift` | 1.20 s | harmless pose change | 0 |
//! | `Death` | 2.40 s | springs pop, machine stops | 0 |
//!
//! The pull is a 22 px/s drag toward the nip inside [`intake_lane`]; the rat
//! walks 58 px/s, so it can always out-walk it or step across the lane.

#![allow(dead_code)]

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

// ============================================================================
// Public Collision & Attack Geometry Constants
// ============================================================================

/// The framework's base pool before the director's boss curve (wave-3 pool
/// scaling: see `BossKind::hp_scale`). Expressed like the Broom and the Mower
/// so the Laundry's 15-minute fight lands in the same 45-80 s band.
pub const BOSS_BASE_HP: f32 = 1600.0;

/// Single-circle movement/contact radius (world px).
pub const BODY_RADIUS: f32 = 13.0;

/// Half the roller length: the intake lane is `2 × 11 = 22 px` wide.
pub const ROLLER_HALF_WIDTH: f32 = 11.0;

/// Half-length of the body box along the facing axis (world px).
pub const BODY_HALF_LEN: f32 = 9.5;

/// The frame's box centre sits this far behind `view.pos`.
pub const BODY_CENTRE_BACK: f32 = 1.5;

/// The nip plane (the front face the rat is fed into) ahead of `view.pos`.
pub const NIP_OFFSET: f32 = 7.5;

/// Length of the intake lane, measured out from the nip (world px).
pub const INTAKE_LANE_LEN: f32 = 40.0;

/// The last strip of the lane: the single bite crosses this depth (world px).
pub const INTAKE_BITE_DEPTH: f32 = 8.0;

/// The intake's drag toward the nip (px/s). The rat walks 58 px/s.
pub const PULL_SPEED: f32 = 22.0;

/// The bite's damage.
pub const BITE_DAMAGE: f32 = 16.0;

/// Radius of the loose crank's hit circle (world px).
pub const CRANK_RADIUS: f32 = 4.5;
/// The crank hub's offset across the machine (world px, +perp = right cheek).
pub const CRANK_SIDE_OFFSET: f32 = 13.8;
/// The crank hub's offset along the machine (world px).
pub const CRANK_ALONG: f32 = -1.0;

/// Length of the wring-out rail measured from the nip (world px).
pub const TOWEL_LANE_LEN: f32 = 56.0;
/// Half-width of the wring-out rail: the lane is `56 × 10 px`.
pub const TOWEL_LANE_HALF_WIDTH: f32 = 5.0;
/// A rolled towel's collision radius.
pub const TOWEL_RADIUS: f32 = 4.0;
/// The rolled towel's travel speed (px/s).
pub const TOWEL_SPEED: f32 = 88.0;
/// The rolled towel's damage.
pub const TOWEL_DAMAGE: f32 = 18.0;

/// Body-contact damage outside recovery (the framework's per-kind number).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// The crank takes ×1.5 damage while the machine is jammed.
pub const JAM_DAMAGE_MULT: f32 = 1.5;

/// Culling margin around `view.pos` (world px).
pub const CULL_MARGIN: f32 = 60.0;

// ---- state durations (seconds) --------------------------------------------

/// Neutral recovery beat per phase.
pub const IDLE_P1: f32 = 0.55;
pub const IDLE_P2: f32 = 0.45;
pub const IDLE_P3: f32 = 0.40;

/// Intake tell / pull per phase (the lane outlines, then the rollers feed).
pub const INTAKE_TELL_P1: f32 = 1.10;
pub const INTAKE_PULL_P1: f32 = 1.00;
pub const INTAKE_TELL_P3: f32 = 0.90;
pub const INTAKE_PULL_P3: f32 = 0.50;

/// The single bite's active window (one hit).
pub const BITE_WINDOW: f32 = 0.10;

/// The settle beat after the bite before the next state.
pub const INTAKE_SETTLE: f32 = 0.20;

/// The break between the two lanes of a phase-3 two-part intake.
pub const INTAKE_BREAK: f32 = 0.50;

/// The wring-out: two crank clicks, then the towel crosses the rail.
pub const WRING_TELL: f32 = 0.90;
pub const WRING_TRAVEL: f32 = 0.70;
pub const WRING_RECOVER: f32 = 0.40;

/// The phase-1/2 towel jam: the punish window.
pub const JAM: f32 = 1.50;
/// The phase-3 final jam: the long punish window ("until the springs pop").
pub const FINAL_JAM: f32 = 2.20;

/// The harmless pose change at a phase transition.
pub const PHASE_SHIFT: f32 = 1.20;

/// The death beat (springs pop, the machine stops).
pub const DEATH_DURATION: f32 = 2.40;

/// Full intake duration for a phase.
#[inline]
pub fn intake_duration(phase: u8) -> f32 {
    let (tell, pull) = intake_windows(phase);
    tell + pull + BITE_WINDOW + INTAKE_SETTLE
}

/// The `(tell, pull)` windows for an intake in `phase`.
#[inline]
pub fn intake_windows(phase: u8) -> (f32, f32) {
    if phase >= 3 {
        (INTAKE_TELL_P3, INTAKE_PULL_P3)
    } else {
        (INTAKE_TELL_P1, INTAKE_PULL_P1)
    }
}

/// Neutral beat for a phase.
#[inline]
pub fn idle_duration(phase: u8) -> f32 {
    match phase {
        1 => IDLE_P1,
        2 => IDLE_P2,
        _ => IDLE_P3,
    }
}

// ============================================================================
// Public View & Move-Set State Types
// ============================================================================

/// Complete move set for **The Mangle**.
///
/// Every state progresses linearly via [`MangleView::progress`] from `0.0` to
/// `1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MangleState {
    /// **Tick-over (neutral recovery).** The rollers idle, the crank hangs and
    /// sways, the little iron feet take a waddling step toward the rat.
    /// Duration `0.55 / 0.45 / 0.40 s` per phase. Body contact `8` (also the
    /// machine's whole walk).
    Idle,

    /// **Intake.** The rollers turn **backwards** while the `22 × 40 px`
    /// intake lane outlines out of the nip. Then they reverse and the lane
    /// drags anything inside it toward the rollers at [`PULL_SPEED`] for the
    /// pull window, and a single [`BITE_DAMAGE`] bite crosses the last
    /// [`INTAKE_BITE_DEPTH`] px. `second` marks the second lane of the phase-3
    /// two-part intake (a fresh tell after a [`INTAKE_BREAK`] break).
    /// Duration `2.40 / 2.40 / 1.70 s` (see [`intake_duration`]).
    Intake { second: bool },

    /// **Wring-out.** The crank clicks over twice during the `0.90 s` tell,
    /// then a rolled towel is ejected along the fixed `56 × 10 px` rail for
    /// [`TOWEL_DAMAGE`]. The rail is where the towel goes: one clear flank
    /// beside it.
    WringOut,

    /// **Towel jam (phase 1/2).** The nip is stuffed: the frame shudders, the
    /// crank drops loose and takes ×[`JAM_DAMAGE_MULT`], body contact is off.
    /// `1.50 s` of free damage.
    Jam,

    /// **Final jam (phase 3).** The torn belt slips off and the rollers go
    /// slack (ungrounded): the dropped crank and the feet are the targets.
    /// `2.20 s` — "strike the dropped crank until the springs pop".
    FinalJam,

    /// **Phase shift.** A harmless `1.20 s` pose change: the frame shudders
    /// and settles, the belt slaps the cheek, the crank swings out.
    PhaseShift,

    /// **Death.** The springs pop, the belt unspools, the rollers coast to a
    /// stop and the frame sags. `2.40 s`, then the framework reaps it.
    Death,
}

/// Pure, `Copy` render-state snapshot of **The Mangle**.
#[derive(Clone, Copy, Debug)]
pub struct MangleView {
    /// World-space centre of the frame.
    pub pos: Vec2,
    /// Feed direction (unit; defaults to `(1, 0)` if zero).
    pub facing: Vec2,
    /// Current move / behavioural state.
    pub state: MangleState,
    /// Normalized progress through `state`, clamped to `0.0..=1.0`.
    pub progress: f32,
    /// Continuous animation clock in seconds.
    pub anim: f32,
    /// Hit-flash intensity `0.0..=1.0`.
    pub hit_flash: f32,
    /// Remaining health fraction.
    pub hp_frac: f32,
    /// World-space telegraph target (the lane end for the intake, the rail end
    /// for the wring-out).
    pub target: Vec2,
    /// Fight phase (`1..=3`).
    pub phase: u8,
}

/// One art-defined hit shape (world space).
#[derive(Clone, Copy, Debug)]
pub struct MangleHit {
    /// Shape centre.
    pub center: Vec2,
    /// Box orientation (unit along the facing axis); unused for circles.
    pub facing: Vec2,
    /// Half-length along `facing` (boxes).
    pub half: f32,
    /// Half-width across `facing` (boxes).
    pub half_perp: f32,
    /// Radius (circles).
    pub radius: f32,
    /// `true` = oriented box, `false` = circle.
    pub boxed: bool,
    /// Is this shape on the floor (hittable by ground weapons)?
    pub grounded: bool,
}

/// An oriented floor rectangle (the lane / rail geometry).
#[derive(Clone, Copy, Debug)]
pub struct MangleLane {
    /// Rectangle centre.
    pub center: Vec2,
    /// Unit direction from the nip outward.
    pub facing: Vec2,
    /// Half-length along `facing`.
    pub half: f32,
    /// Half-width across `facing`.
    pub half_perp: f32,
}

impl MangleLane {
    /// Does a circle at `point` touch the rectangle?
    #[inline]
    pub fn contains(&self, point: Vec2, radius: f32) -> bool {
        let d = point - self.center;
        let u = d.dot(self.facing);
        let v = d.dot(self.facing.perp());
        u.abs() <= self.half + radius && v.abs() <= self.half_perp + radius
    }
}

// ============================================================================
// Public Geometry Queries (Art <-> Hitbox Contract)
// ============================================================================

#[inline]
pub fn unit_facing(view: &MangleView) -> Vec2 {
    if view.facing.len_sq() > 1e-4 {
        view.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// The nip: the front face of the rollers, where the feed enters.
#[inline]
pub fn nip_pos(view: &MangleView) -> Vec2 {
    view.pos + unit_facing(view) * NIP_OFFSET
}

/// The `22 × 40 px` intake lane out of the nip.
#[inline]
pub fn intake_lane(view: &MangleView) -> MangleLane {
    let f = unit_facing(view);
    MangleLane {
        center: view.pos + f * (NIP_OFFSET + INTAKE_LANE_LEN * 0.5),
        facing: f,
        half: INTAKE_LANE_LEN * 0.5,
        half_perp: ROLLER_HALF_WIDTH,
    }
}

/// The last [`INTAKE_BITE_DEPTH`] px of the lane: the single bite's strip.
#[inline]
pub fn bite_strip(view: &MangleView) -> MangleLane {
    let f = unit_facing(view);
    MangleLane {
        center: view.pos + f * (NIP_OFFSET + INTAKE_BITE_DEPTH * 0.5),
        facing: f,
        half: INTAKE_BITE_DEPTH * 0.5,
        half_perp: ROLLER_HALF_WIDTH,
    }
}

/// The `56 × 10 px` wring-out rail out of the nip.
#[inline]
pub fn wring_lane(view: &MangleView) -> MangleLane {
    let f = unit_facing(view);
    MangleLane {
        center: view.pos + f * (NIP_OFFSET + TOWEL_LANE_LEN * 0.5),
        facing: f,
        half: TOWEL_LANE_LEN * 0.5,
        half_perp: TOWEL_LANE_HALF_WIDTH,
    }
}

/// How far the crank has dropped out of its seat (0 seated, 1 hanging).
#[inline]
pub fn crank_lift(view: &MangleView) -> f32 {
    let p = if view.progress.is_finite() {
        view.progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    match view.state {
        MangleState::Jam | MangleState::FinalJam | MangleState::Death => 1.0,
        MangleState::WringOut => {
            let total = WRING_TELL + WRING_TRAVEL + WRING_RECOVER;
            if p * total >= WRING_TELL + WRING_TRAVEL {
                0.35
            } else {
                0.0
            }
        }
        MangleState::PhaseShift => 0.5 * smoothstep((p.min(0.55) / 0.55).clamp(0.0, 1.0)),
        _ => 0.0,
    }
}

/// The loose crank (the ×1.5 weak point): the hub, dropped out of its seat
/// exactly as the art draws it.
#[inline]
pub fn crank_pos(view: &MangleView) -> Vec2 {
    let f = unit_facing(view);
    let lift = crank_lift(view);
    let hub = view.pos + f * CRANK_ALONG + f.perp() * CRANK_SIDE_OFFSET;
    hub + Vec2::new(0.0, 2.4 * lift) + f.perp() * (1.6 * lift)
}

/// The exact world-space hit shapes: `[body box, crank circle]`.
///
/// The body box is the iron frame + both rollers. In [`MangleState::FinalJam`]
/// the torn belt lets the rollers slip, so the body is **ungrounded** (ground
/// weapons pass through the slack rollers) while the crank and feet stay
/// hittable.
pub fn hit_shapes(view: &MangleView) -> [MangleHit; 2] {
    let f = unit_facing(view);
    let grounded = !matches!(view.state, MangleState::FinalJam | MangleState::Death);
    [
        MangleHit {
            center: view.pos - f * BODY_CENTRE_BACK,
            facing: f,
            half: BODY_HALF_LEN,
            half_perp: ROLLER_HALF_WIDTH + 1.4,
            radius: 0.0,
            boxed: true,
            grounded,
        },
        MangleHit {
            center: crank_pos(view),
            facing: f,
            half: 0.0,
            half_perp: 0.0,
            radius: CRANK_RADIUS,
            boxed: false,
            grounded: !matches!(view.state, MangleState::Death),
        },
    ]
}

/// Where the rolled towel is at `t` seconds into its travel (art + vfx).
#[inline]
pub fn towel_pos(view: &MangleView, t: f32) -> Vec2 {
    nip_pos(view) + unit_facing(view) * (TOWEL_SPEED * t).min(TOWEL_LANE_LEN)
}

// ============================================================================
// Palette & Lighting Constants
// ============================================================================

/// Top-left directional light shared with the rest of the art.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// Heavy Victorian cast iron + machined steel cheeks.
const IRON_OUTLINE: Rgb = (8, 10, 14);
const IRON_SH: Rgb = (24, 30, 40);
const IRON_MID: Rgb = (62, 74, 92);
const IRON_HI: Rgb = (134, 150, 170);
const IRON_SPEC: Rgb = (218, 232, 246);
// Vintage mint-porcelain enamel badge + trim.
const ENAMEL_SH: Rgb = (68, 106, 106);
const ENAMEL_MID: Rgb = (144, 186, 180);
const ENAMEL_HI: Rgb = (220, 244, 238);
// Glossy candy-striped wringer rubber rollers.
const ROLLER_DIM: Rgb = (130, 118, 94);
const ROLLER_CREAM: Rgb = (224, 212, 184);
const ROLLER_CREAM_HI: Rgb = (255, 250, 234);
const ROLLER_RED_SH: Rgb = (118, 20, 28);
const ROLLER_RED: Rgb = (196, 38, 44);
const ROLLER_RED_HI: Rgb = (248, 98, 86);
// Turned brass screw, gears, rivets & mahogany crank handle.
const BRASS_SH: Rgb = (96, 62, 18);
const BRASS_MID: Rgb = (206, 152, 48);
const BRASS_HI: Rgb = (255, 228, 126);
const WOOD_SH: Rgb = (68, 34, 16);
const WOOD_MID: Rgb = (156, 92, 42);
const WOOD_HI: Rgb = (228, 166, 102);
// Crumpled laundry towel / suds.
const TOWEL_SH: Rgb = (126, 118, 106);
const TOWEL_MID: Rgb = (218, 212, 194);
const TOWEL_HI: Rgb = (252, 249, 238);
const TOWEL_STRIPE: Rgb = (182, 58, 66);
// Torn rubber drive belt + coiled springs.
const BELT_SH: Rgb = (18, 20, 26);
const BELT_MID: Rgb = (52, 56, 68);
const BELT_HI: Rgb = (106, 114, 130);
const SPRING_SH: Rgb = (108, 118, 134);
const SPRING_COL: Rgb = (204, 214, 228);
const SPRING_HI: Rgb = (246, 250, 255);
// Telegraphs, weak-point gold, steam & heat.
const WARN_AMBER: Rgb = (255, 186, 56);
const WARN_RED: Rgb = (255, 68, 56);
const WARN_HOT: Rgb = (255, 244, 196);
const JAM_GOLD: Rgb = (255, 216, 84);
const HEAT_GLOW: Rgb = (255, 112, 48);
const STEAM_SH: Rgb = (122, 140, 154);
const STEAM_MID: Rgb = (196, 214, 224);
const STEAM_HI: Rgb = (248, 252, 255);

// ============================================================================
// Internal Pose (Zero Allocation)
// ============================================================================

#[derive(Clone, Copy)]
struct Pose {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    /// Roller stripe phase in world px along the roller axis.
    roll: f32,
    /// Roller spin speed & direction (<0 backwards tell, >0 feeding).
    spin: f32,
    /// Crank arm angle (radians around the hub in local frame).
    crank_angle: f32,
    /// 0..1 how far the crank has swung out of its seat (jam/death).
    crank_loose: f32,
    /// 0..1 how far the drive belt has slipped/torn off (phase 3).
    belt_slip: f32,
    /// 0..1 frame sag/slump (jam/death).
    sag: f32,
    /// Forward/back anticipation lean along `f` (px).
    lean: f32,
    /// Waddle rock along `p` (px).
    rock: f32,
    /// Screen-pixel shudder offset.
    shake: Vec2,
    /// 0..1 crank weak-point glow.
    glow: f32,
    /// 0..1 towel jam spill (a cloth bulge at the nip).
    towel: f32,
    /// 0..1 wring-out cloth peek before ejection.
    wring_bulge: f32,
    /// 0..1 steam plume density.
    steam: f32,
    /// 0..1 roller slack tilt (FinalJam / Death).
    slack: f32,
    /// Body opacity.
    alpha: f32,
    /// 0..1 death progression.
    death: f32,
    /// Effective visual damage tier (1..=3).
    damage_tier: u8,
    /// Roller grounding (final jam / death).
    rollers_grounded: bool,
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn compute_pose(view: &MangleView) -> Pose {
    let p = if view.progress.is_finite() {
        view.progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let anim = if view.anim.is_finite() { view.anim } else { 0.0 };
    let hp = if view.hp_frac.is_finite() {
        view.hp_frac.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let f = unit_facing(view);
    let perp = f.perp();
    let phase = view.phase.max(1);
    let damage_tier = if phase >= 3 || hp <= 0.25 {
        3
    } else if phase >= 2 || hp <= 0.60 {
        2
    } else {
        1
    };

    let mut pose = Pose {
        pos: view.pos,
        f,
        p: perp,
        roll: anim * 1.8,
        spin: 1.6,
        crank_angle: -FRAC_PI_2 + (anim * 2.2).sin() * 0.16,
        crank_loose: 0.0,
        belt_slip: 0.0,
        sag: 0.0,
        lean: 0.0,
        rock: 0.0,
        shake: Vec2::ZERO,
        glow: 0.0,
        towel: 0.0,
        wring_bulge: 0.0,
        steam: if damage_tier >= 3 {
            0.35
        } else if damage_tier == 2 {
            0.16
        } else {
            0.0
        },
        slack: 0.0,
        alpha: 1.0,
        death: 0.0,
        damage_tier,
        rollers_grounded: true,
    };

    // Torn belt hangs loose in phase 3 and death.
    if damage_tier >= 3 || matches!(view.state, MangleState::Death) {
        pose.belt_slip = 0.88;
    }

    match view.state {
        MangleState::Idle => {
            // Heavy waddling gait on the four little iron feet; rollers lazily tick over.
            let gait = anim * (2.6 + (phase as f32 - 1.0) * 0.5);
            let step = gait.sin();
            pose.rock = step * 0.85;
            pose.lean = step.abs() * 0.45;
            pose.roll = anim * 2.4;
            pose.crank_angle = -FRAC_PI_2 + step * 0.22;
        }
        MangleState::Intake { .. } => {
            let (tell, pull) = intake_windows(phase);
            let total = intake_duration(phase);
            let tell_p = (tell / total).clamp(0.0, 1.0);
            let pull_p = ((tell + pull) / total).clamp(0.0, 1.0);
            if p < tell_p {
                // Anticipation: frame leans back on its heels, rollers spin BACKWARDS.
                let t = if tell_p > 0.0 { p / tell_p } else { 1.0 };
                let ease = smoothstep(t);
                pose.roll = 12.0 - t * 22.0;
                pose.spin = -1.8;
                pose.lean = -1.6 * ease;
                pose.shake = perp * ((anim * 32.0).sin() * 0.55 * t);
                pose.crank_angle = -FRAC_PI_2 + t * 1.4;
                pose.steam = pose.steam.max(0.18 + 0.25 * t);
            } else if p < pull_p {
                // Pull: frame lunges forward hungrily, rollers whirl inward at high RPM!
                let t = ((p - tell_p) / (pull_p - tell_p).max(1e-4)).clamp(0.0, 1.0);
                pose.roll = -(anim * 30.0) - t * 8.0;
                pose.spin = 3.8;
                pose.lean = 1.5 + 0.5 * t;
                pose.shake = f * (0.4 + 0.55 * (anim * 38.0).sin().abs())
                    + perp * ((anim * 28.0).sin() * 0.35);
                pose.crank_angle = -FRAC_PI_2 - anim * 14.0;
                pose.steam = pose.steam.max(0.28);
            } else {
                // Bite clench: violent forward snap + recoil!
                let bt = ((p - pull_p) / (1.0 - pull_p).max(1e-4)).clamp(0.0, 1.0);
                let snap = (1.0 - bt).powi(2);
                pose.roll = 0.0;
                pose.spin = 0.0;
                pose.lean = 2.6 * snap - 0.5 * bt;
                pose.shake = f * (1.2 * snap);
                pose.steam = pose.steam.max(0.45 * snap);
            }
        }
        MangleState::WringOut => {
            let total = WRING_TELL + WRING_TRAVEL + WRING_RECOVER;
            let tell_p = (WRING_TELL / total).clamp(0.0, 1.0);
            let travel_p = ((WRING_TELL + WRING_TRAVEL) / total).clamp(0.0, 1.0);
            if p < tell_p {
                let t = if tell_p > 0.0 { p / tell_p } else { 1.0 };
                // Two heavy mechanical ratchet clicks at t = 0.5 and t = 1.0.
                let clicks = (t * 2.0).floor();
                let within = (t * 2.0).fract();
                let recoil = (1.0 - (within * 4.0).min(1.0)).powi(2);
                pose.crank_angle =
                    -FRAC_PI_2 - clicks * (FRAC_PI_2 * 0.88) - smoothstep(within) * 0.22;
                pose.lean = -1.2 * t + 0.65 * recoil;
                pose.shake = perp * ((anim * 24.0).sin() * 0.45 * recoil);
                pose.roll = -t * 6.5;
                pose.spin = 0.8;
                pose.wring_bulge = smoothstep(t);
            } else if p < travel_p {
                let t = ((p - tell_p) / (travel_p - tell_p).max(1e-4)).clamp(0.0, 1.0);
                pose.crank_angle = -FRAC_PI_2 - 2.0 * (FRAC_PI_2 * 0.88) - t * 0.6;
                pose.lean = 1.8 * (1.0 - t).powi(2);
                pose.roll = -(anim * 24.0);
                pose.spin = 2.8;
                pose.shake = f * (0.65 * (1.0 - t));
                pose.wring_bulge = (1.0 - t * 3.0).max(0.0);
            } else {
                let t = ((p - travel_p) / (1.0 - travel_p).max(1e-4)).clamp(0.0, 1.0);
                pose.crank_loose = 0.35;
                pose.glow = 0.55 + 0.20 * (1.0 - t);
                pose.sag = 0.12;
            }
        }
        MangleState::Jam => {
            pose.towel = 1.0;
            pose.crank_loose = 1.0;
            pose.sag = 0.34;
            pose.steam = 0.85;
            pose.glow = 1.0;
            pose.roll = (anim * 1.2).sin() * 1.2 - 1.0;
            pose.spin = 0.15;
            pose.shake = perp * ((anim * 38.0).sin() * 0.65);
        }
        MangleState::FinalJam => {
            pose.towel = 1.0;
            pose.crank_loose = 1.0;
            pose.sag = 0.62;
            pose.slack = 1.0;
            pose.belt_slip = 1.0;
            pose.steam = 1.0;
            pose.glow = 1.0;
            pose.roll = (anim * 0.7).sin() * 2.2;
            pose.spin = 0.05;
            pose.shake = perp * ((anim * 44.0).sin() * 0.85);
            pose.rollers_grounded = false;
        }
        MangleState::PhaseShift => {
            let t = smoothstep(p.min(0.55) / 0.55);
            let fade = 1.0 - smoothstep(((p - 0.55) / 0.45).clamp(0.0, 1.0));
            pose.shake = perp * ((anim * 52.0).sin() * 1.8 * fade)
                + f * ((anim * 41.0).cos() * 0.8 * fade);
            pose.sag = 0.28 * t;
            pose.steam = 0.90 * fade + 0.25;
            pose.crank_loose = 0.5 * t;
            pose.roll = -t * 5.0;
            pose.spin = 0.4;
            pose.glow = 0.45 * t;
        }
        MangleState::Death => {
            let t = p.clamp(0.0, 1.0);
            pose.death = t;
            pose.towel = (1.0 - smoothstep(((t - 0.25) / 0.45).clamp(0.0, 1.0))).max(0.25);
            pose.crank_loose = 1.0;
            pose.sag = 0.95 * smoothstep((t / 0.55).clamp(0.0, 1.0));
            pose.slack = smoothstep((t / 0.45).clamp(0.0, 1.0));
            pose.belt_slip = 1.0;
            pose.steam = (1.0 - t * 0.65).clamp(0.2, 1.0);
            pose.glow = 1.0 - smoothstep((t / 0.30).clamp(0.0, 1.0));
            pose.spin = (1.0 - t).powi(2) * 1.4;
            pose.roll = anim * pose.spin;
            pose.shake = f * ((anim * 42.0).sin() * 1.4 * (1.0 - t))
                + perp * ((anim * 35.0).cos() * 1.1 * (1.0 - t));
            pose.rollers_grounded = false;
            pose.alpha = if t < 0.62 {
                1.0
            } else {
                1.0 - smoothstep(((t - 0.62) / 0.38).clamp(0.0, 1.0))
            };
        }
    }
    pose
}

// ============================================================================
// Low-Level Pixel Helpers (Zero Allocation)
// ============================================================================

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> Vec2 {
    p - cam
}

#[inline]
fn in_view(pb: &PixelBuffer, view: &MangleView, cam: Vec2) -> bool {
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
    t.x >= -70.0 && t.x <= w + 70.0 && t.y >= -70.0 && t.y <= h + 70.0
}

#[inline]
fn bbox(pb: &PixelBuffer, center: Vec2, rx: f32, ry: f32) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0 || pb.h == 0 || !center.x.is_finite() || !center.y.is_finite() {
        return None;
    }
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

/// Tight screen-space AABB around an oriented [`MangleLane`].
#[inline]
fn bbox_lane(
    pb: &PixelBuffer,
    lane: &MangleLane,
    cam: Vec2,
    pad: f32,
) -> Option<(i32, i32, i32, i32)> {
    let f = lane.facing;
    let rx = f.x.abs() * lane.half + f.y.abs() * lane.half_perp + pad;
    let ry = f.y.abs() * lane.half + f.x.abs() * lane.half_perp + pad;
    bbox(pb, w2s(lane.center, cam), rx, ry)
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
    if l < 0.56 {
        mix(sh, mid, l / 0.56)
    } else {
        mix(mid, hi, (l - 0.56) / 0.59)
    }
}

/// Warm-gold hit flash that keeps local contrast.
#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.01 {
        return c;
    }
    let lum = (c.0 as f32 * 0.30 + c.1 as f32 * 0.59 + c.2 as f32 * 0.11) / 255.0;
    let flash = mix((238, 168, 92), (255, 248, 224), (0.35 + lum * 0.65).clamp(0.0, 1.0));
    mix(c, flash, (0.28 + hit * 0.50).min(0.76))
}

#[inline]
fn sd_rbox(u: f32, v: f32, cu: f32, cv: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let du = (u - cu).abs() - (hu - r).max(0.0);
    let dv = (v - cv).abs() - (hv - r).max(0.0);
    let dx = du.max(0.0);
    let dy = dv.max(0.0);
    let outside = (dx * dx + dy * dy).sqrt();
    outside + du.max(dv).min(0.0) - r
}

#[inline]
fn sd_circle(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> f32 {
    ((u - cu) * (u - cu) + (v - cv) * (v - cv)).sqrt() - r
}

#[inline]
fn sd_segment(u: f32, v: f32, au: f32, av: f32, bu: f32, bv: f32, r: f32) -> f32 {
    let pax = u - au;
    let pay = v - av;
    let bax = bu - au;
    let bay = bv - av;
    let denom = (bax * bax + bay * bay).max(1e-6);
    let h = ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt() - r
}

/// Deterministic hash in `0..1` for floor dressing and damage detail.
#[inline]
fn hash01(ix: i32, iy: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x9E37_79B1) ^ (iy as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Screen-space dashed line with optional dark contrast backing.
fn line_dashed(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, alpha: f32, dash: f32, phase: f32) {
    let d = b - a;
    let len = d.len();
    // A non-positive dash would never advance `t` below (an endless loop), and
    // a non-finite length would never end it.
    if len <= 0.1 || !len.is_finite() || !dash.is_finite() || dash < 0.25 {
        return;
    }
    let dir = d * (1.0 / len);
    let period = dash * 1.65;
    let mut t = phase.rem_euclid(period) - period;
    while t < len {
        let s0 = t.max(0.0);
        let s1 = (t + dash).min(len);
        let mut s = s0;
        while s < s1 {
            let q = a + dir * s;
            let x = q.x.round() as i32;
            let y = q.y.round() as i32;
            darken_px(pb, x, y + 1, 0.25 * alpha);
            blend_px(pb, x, y, col, alpha);
            s += 0.75;
        }
        t += period;
    }
}

fn line_solid(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, alpha: f32) {
    let d = b - a;
    if !d.len().is_finite() {
        return;
    }
    let steps = (d.len() * 1.5).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let q = a + d * t;
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
    }
}

fn ring(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    col: Rgb,
    alpha: f32,
    thickness: f32,
) {
    if radius <= 0.0 || alpha <= 0.005 {
        return;
    }
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius + thickness + 1.0, radius + thickness + 1.0)
    else {
        return;
    };
    let half_t = (thickness * 0.5).max(0.5);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            let band = (d - radius).abs();
            if band <= half_t {
                let a = alpha * (1.0 - (band / (half_t + 1e-3)).clamp(0.0, 1.0));
                blend_px(pb, x, y, col, a);
            }
        }
    }
}

fn fill_disc(pb: &mut PixelBuffer, center: Vec2, radius: f32, col: Rgb, alpha: f32) {
    if radius <= 0.0 || alpha <= 0.005 {
        return;
    }
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius + 1.0, radius + 1.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= radius {
                let a = alpha * (1.0 - (d / radius).powi(2) * 0.38);
                blend_px(pb, x, y, col, a);
            }
        }
    }
}

/// Cel-shaded steam puff with top-left directional highlight and dark rim.
fn draw_steam_puff(pb: &mut PixelBuffer, center: Vec2, r: f32, alpha: f32) {
    if r <= 0.6 || alpha <= 0.02 {
        return;
    }
    let Some((x0, x1, y0, y1)) = bbox(pb, center, r + 1.5, r + 1.5) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d > r + 0.9 {
                continue;
            }
            if d > r {
                darken_px(pb, x, y, 0.28 * alpha * (1.0 - (d - r) / 0.9));
                continue;
            }
            let nx = dx / r;
            let ny = dy / r;
            let lit = nx * LIGHT.x + ny * LIGHT.y;
            let dome = (1.0 - (d / r).powi(2) * 0.55).clamp(0.0, 1.0);
            let col = shade3(STEAM_SH, STEAM_MID, STEAM_HI, 0.45 + 0.35 * dome + 0.48 * lit);
            blend_px(pb, x, y, col, alpha * (1.0 - (d / r).powi(3) * 0.25));
        }
    }
}

/// Draw a visible helical coil spring on the floor or popping off the machine.
fn draw_coil_spring(pb: &mut PixelBuffer, a: Vec2, b: Vec2, loops: i32, width: f32, alpha: f32) {
    let seg = b - a;
    let len = seg.len();
    if len < 1.0 || alpha <= 0.02 {
        return;
    }
    let dir = seg * (1.0 / len);
    let perp = dir.perp();
    let steps = (len * 2.5).ceil().max(6.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let wave = (t * loops as f32 * TAU).sin();
        let q = a + seg * t + perp * (wave * width);
        let x = q.x.round() as i32;
        let y = q.y.round() as i32;
        darken_px(pb, x, y + 1, 0.48 * alpha);
        let col = if wave < -0.2 {
            SPRING_HI
        } else if wave > 0.3 {
            SPRING_SH
        } else {
            SPRING_COL
        };
        blend_px(pb, x, y, col, alpha);
    }
}

// ============================================================================
// Public Entry Point 1: Ground Layer (Telegraphs, Mat, HP Arc)
// ============================================================================

/// Draw The Mangle's ground layer.
///
/// **Layer**: ground — after the floor and stains, before props/enemies/the
/// rat.
pub fn draw_ground(pb: &mut PixelBuffer, view: &MangleView, cam: Vec2, accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let pose = compute_pose(view);
    let p = if view.progress.is_finite() {
        view.progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let anim = if view.anim.is_finite() { view.anim } else { 0.0 };
    let hp_frac = if view.hp_frac.is_finite() {
        view.hp_frac.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let pulse = 0.5 + 0.5 * (anim * 9.5).sin();
    let c = w2s(pose.pos, cam);

    // 1) Faded ribbed rubber laundry mat under the machine.
    draw_intake_mat(pb, &pose, cam);

    // 2) Deterministic soap suds and floor lint around the mat.
    draw_floor_lint(pb, c, anim, pose.alpha);

    // 3) State telegraphs and impact flashes.
    match view.state {
        MangleState::Intake { second } => {
            let (tell, pull) = intake_windows(view.phase);
            let total = intake_duration(view.phase);
            let tell_s = tell;
            let pull_s = tell + pull;
            let lane = intake_lane(view);
            let bite = bite_strip(view);
            if (p * total) < tell_s {
                let t = ((p * total) / tell_s).clamp(0.0, 1.0);
                draw_intake_lane(pb, &lane, cam, t, 0.0, -anim * 10.0, pulse, second);
                draw_bite_tell(pb, &bite, cam, t, 0.0, pulse);
            } else if (p * total) < pull_s {
                let t = (((p * total) - tell_s) / pull).clamp(0.0, 1.0);
                draw_intake_lane(pb, &lane, cam, 1.0, t, anim * 34.0, pulse, second);
                draw_bite_tell(pb, &bite, cam, 1.0, t, pulse);
            } else {
                let t = (((p * total) - pull_s) / (BITE_WINDOW + INTAKE_SETTLE)).clamp(0.0, 1.0);
                draw_bite_flash(pb, &bite, cam, t);
            }
        }
        MangleState::WringOut => {
            let lane = wring_lane(view);
            let tell_s = WRING_TELL;
            let travel_s = WRING_TELL + WRING_TRAVEL;
            let total = travel_s + WRING_RECOVER;
            let now = p * total;
            if now < tell_s {
                let t = (now / tell_s).clamp(0.0, 1.0);
                draw_wring_rail(pb, &lane, cam, t, 0.0, pulse, anim);
                draw_crank_clicks(pb, &pose, cam, t, pulse);
            } else if now < travel_s {
                let t = ((now - tell_s) / WRING_TRAVEL).clamp(0.0, 1.0);
                draw_wring_rail(pb, &lane, cam, 1.0, t, pulse, anim);
                // Advance click time past 1.0 so the second ratchet click smoothly expands and fades.
                draw_crank_clicks(pb, &pose, cam, 1.0 + t * (WRING_TRAVEL / WRING_TELL), pulse);
            } else {
                let t = ((now - travel_s) / WRING_RECOVER).clamp(0.0, 1.0);
                draw_wring_rail(pb, &lane, cam, 1.0 - t * 0.65, 1.0, pulse, anim);
                draw_crank_gather(pb, &pose, cam, t, pulse);
            }
        }
        MangleState::Jam | MangleState::FinalJam => {
            draw_towel_puddle(pb, &pose, cam, pulse);
            draw_crank_gather(pb, &pose, cam, p, pulse);
            if matches!(view.state, MangleState::FinalJam) {
                draw_spring_ticks(pb, &pose, cam, p, pulse);
            }
        }
        MangleState::PhaseShift => {
            let t = (p / 0.55).clamp(0.0, 1.0);
            if p < 0.55 {
                ring(pb, c, 15.0 + t * 18.0, WARN_AMBER, 0.72 * (1.0 - t * 0.4), 1.8);
                ring(pb, c, 23.0 - t * 4.0, WARN_RED, 0.40 + 0.35 * pulse, 1.3);
            } else {
                let k = 1.0 - ((p - 0.55) / 0.45).clamp(0.0, 1.0);
                ring(pb, c, 18.0 + (1.0 - k) * 14.0, WARN_HOT, 0.55 * k, 1.4);
            }
            // Rubber belt slap sparks beside the cheek.
            let cheek = c + pose.p * 15.5;
            for i in -1..=1 {
                let q = cheek + pose.f * (i as f32 * 2.5);
                line_solid(pb, q, q + pose.p * 3.5, WARN_AMBER, 0.55);
            }
        }
        MangleState::Death => {
            let t = p.clamp(0.0, 1.0);
            // Spreading wet suds & rust-water puddle beneath the dying machine.
            let puddle_r = 12.0 + 14.0 * smoothstep(t * 1.4);
            fill_disc(pb, c + Vec2::new(0.0, 2.0), puddle_r, (32, 48, 62), 0.42 * (1.0 - t * 0.35));
            ring(pb, c + Vec2::new(0.0, 2.0), puddle_r * 0.92, ENAMEL_MID, 0.28 * (1.0 - t * 0.5), 1.2);
            if t < 0.45 {
                let bt = t / 0.45;
                ring(pb, c, 12.0 + bt * 30.0, WARN_RED, 0.72 * (1.0 - bt), 1.8);
                ring(pb, c, 8.0 + bt * 20.0, WARN_HOT, 0.55 * (1.0 - bt), 1.2);
            }
            // Four popped helical springs pinging outward across the floor!
            for i in 0..4 {
                let a = -0.35 + i as f32 * 1.45;
                let dir = Vec2::from_angle(a);
                let dist = 9.0 + smoothstep((t * 1.35).min(1.0)) * (16.0 + i as f32 * 2.8);
                let bounce = ((t * 4.5 + i as f32).sin().abs()) * 3.5 * (1.0 - t);
                let base = c + dir * dist + Vec2::new(0.0, -bounce);
                let tip = base + Vec2::from_angle(a + t * 5.0) * 5.2;
                draw_coil_spring(pb, base, tip, 3, 1.3, (1.0 - t * 0.65).clamp(0.0, 1.0));
            }
        }
        MangleState::Idle => {}
    }

    // 4) Grounded drop shadow under frame and iron feet.
    draw_frame_shadow(pb, &pose, cam);
    if matches!(
        view.state,
        MangleState::Idle | MangleState::PhaseShift | MangleState::Death
    ) {
        draw_foot_prints(pb, &pose, cam, anim, pose.alpha);
    }

    // 5) Centered, clean diegetic ground HP arc (matches Mower & Broom bosses).
    if !matches!(view.state, MangleState::Death) && hp_frac > 0.0 {
        draw_hp_arc(pb, c, hp_frac, view.phase, pulse);
    }

    // Subtle porcelain rim tying the mat to the Laundry palette.
    let rim = mix(view_hint_palette(accent), accent, 0.35);
    ring(pb, c, 22.0, rim, 0.09, 0.9);
}

#[inline]
fn view_hint_palette(accent: Rgb) -> Rgb {
    mix(accent, ENAMEL_MID, 0.5)
}

/// The faded ribbed rubber mat beneath the machine.
fn draw_intake_mat(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    let c = w2s(pose.pos, cam) + Vec2::new(0.0, 1.0);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, 21.0, 19.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * pose.f.x + dy * pose.f.y;
            let v = dx * pose.p.x + dy * pose.p.y;
            let d = sd_rbox(u, v, -1.0, 0.0, 14.5, 13.5, 3.5);
            if d < 1.2 {
                let a = (0.24 * (1.0 - (d.max(0.0) / 1.2))).clamp(0.0, 0.24) * pose.alpha;
                darken_px(pb, x, y, a);
            }
            if d < -0.8 && (u * 1.35).sin() > 0.72 {
                blend_px(pb, x, y, IRON_SH, 0.14 * pose.alpha);
            }
        }
    }
}

/// Loose soap suds and lint (deterministic, drawn once per frame).
fn draw_floor_lint(pb: &mut PixelBuffer, c: Vec2, anim: f32, alpha: f32) {
    if alpha <= 0.02 {
        return;
    }
    for i in 0..8 {
        let hx = hash01(i * 31 + 7, i * 17 + 3);
        let hy = hash01(i * 13 + 5, i * 29 + 11);
        let a = (0.12 + 0.14 * hx) * alpha;
        let r = 1.1 + 1.4 * hy;
        let drift = (anim * 0.45 + hx * TAU).sin() * 1.0;
        let q = c + Vec2::new((hx - 0.5) * 34.0 + drift, (hy - 0.5) * 28.0 + 3.0);
        fill_disc(pb, q, r, ENAMEL_HI, a);
    }
}

/// Clean, centered segmented 3-phase ground HP arc around the Mangle.
fn draw_hp_arc(pb: &mut PixelBuffer, c: Vec2, hp: f32, phase: u8, pulse: f32) {
    let col = if phase >= 3 || hp <= 0.25 {
        mix(WARN_RED, WARN_HOT, 0.25 + pulse * 0.25)
    } else if phase == 2 || hp <= 0.60 {
        mix(WARN_AMBER, WARN_RED, 0.35)
    } else {
        mix(ENAMEL_HI, JAM_GOLD, 0.45)
    };
    let r = 18.5;
    let a0 = PI * 0.18;
    let span = PI * 0.64;
    let steps = 42;
    let lit_n = (hp.clamp(0.0, 1.0) * steps as f32).round() as i32;
    for i in 0..=steps {
        let frac = i as f32 / steps as f32;
        let a = a0 + span * frac;
        let q = c + Vec2::from_angle(a) * r;
        let x = q.x.round() as i32;
        let y = q.y.round() as i32;
        // Phase-threshold tick marks at 25% and 60% HP.
        let is_tick = (frac - 0.25).abs() < 0.018 || (frac - 0.60).abs() < 0.018;
        if is_tick {
            blend_px(pb, x, y, IRON_OUTLINE, 0.90);
            continue;
        }
        darken_px(pb, x, y + 1, 0.42);
        if i <= lit_n {
            let lead = i == lit_n;
            blend_px(pb, x, y, if lead { WARN_HOT } else { col }, if lead { 0.95 } else { 0.86 });
        } else {
            blend_px(pb, x, y, IRON_SH, 0.55);
        }
    }
}

/// The intake lane: high-contrast dashed rails, scrolling suction chevrons, and
/// live pull wind-streaks toward the nip.
#[allow(clippy::too_many_arguments)]
fn draw_intake_lane(
    pb: &mut PixelBuffer,
    lane: &MangleLane,
    cam: Vec2,
    fill: f32,
    pull: f32,
    scroll: f32,
    pulse: f32,
    second: bool,
) {
    let f = lane.facing;
    let p = f.perp();
    let near = w2s(lane.center - f * lane.half, cam);
    let far = w2s(lane.center + f * lane.half, cam);
    let col = if pull > 0.0 {
        mix(WARN_RED, WARN_HOT, 0.25 + 0.30 * pulse)
    } else if second {
        mix(WARN_RED, WARN_AMBER, 0.45 + 0.20 * pulse)
    } else {
        mix(WARN_AMBER, WARN_RED, fill * 0.55)
    };
    let fill_col = if pull > 0.0 { WARN_RED } else { WARN_AMBER };

    // Faint tinted conveyor carpet inside the exact `22 x 40` footprint.
    let Some((x0, x1, y0, y1)) = bbox_lane(pb, lane, cam, 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if lane.contains(s, 0.0) {
                let d = s - lane.center;
                let u = d.dot(f);
                // Stronger danger tint closer to the nip (-u).
                let near_nip = (1.0 - ((u + lane.half) / (lane.half * 2.0)).clamp(0.0, 1.0)) * 0.10;
                let a = 0.11 + 0.12 * fill + 0.11 * pull + near_nip;
                blend_px(pb, x, y, fill_col, a);
            }
        }
    }

    // Crisp side rails & far mouth lip.
    let edge = 0.48 + 0.38 * fill + 0.14 * pull;
    line_dashed(pb, near + p * lane.half_perp, far + p * lane.half_perp, col, edge, 4.0, scroll);
    line_dashed(pb, near - p * lane.half_perp, far - p * lane.half_perp, col, edge, 4.0, scroll);
    line_dashed(
        pb,
        far - p * lane.half_perp,
        far + p * lane.half_perp,
        col,
        edge * 0.92,
        3.0,
        0.0,
    );

    // Marching suction chevrons pointing INWARD toward the nip (-f).
    let rows = 5;
    for i in 0..rows {
        let base = (i as f32 + 0.5) / rows as f32;
        let t = if pull > 0.0 {
            (base - scroll * 0.025).rem_euclid(1.0)
        } else {
            (base + scroll * 0.012).rem_euclid(1.0)
        };
        let u = -lane.half + t * lane.half * 2.0;
        let q = w2s(lane.center + f * u, cam);
        let tip = q - f * 3.4;
        let a1 = q + f * 0.4 + p * 4.4;
        let a2 = q + f * 0.4 - p * 4.4;
        let a = (0.45 + 0.38 * fill) * (0.85 + 0.15 * pulse);
        draw_arrow(pb, tip, a1, a2, if pull > 0.0 { WARN_HOT } else { col }, a);
    }

    // Active suction wind streaks dragging toward the nip during pull!
    if pull > 0.0 {
        for k in -2..=2 {
            let phase = (scroll * 0.035 + (k + 2) as f32 * 0.23).fract();
            let u = lane.half - phase * (lane.half * 1.8);
            let q = w2s(lane.center + f * u + p * (k as f32 * 3.6), cam);
            line_solid(pb, q, q - f * 5.5, WARN_HOT, 0.55 * (1.0 - (phase - 0.5).abs() * 1.8).max(0.0));
        }
    }
}

fn draw_arrow(pb: &mut PixelBuffer, tip: Vec2, a: Vec2, b: Vec2, col: Rgb, alpha: f32) {
    for (from, to) in [(a, tip), (tip, b)] {
        let d = to - from;
        let steps = (d.len() * 1.4).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let q = from + d * t;
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
        }
    }
}

/// The bite strip: crimson danger zone across the last 8 px with gleaming jaw-teeth ticks.
fn draw_bite_tell(pb: &mut PixelBuffer, bite: &MangleLane, cam: Vec2, tell: f32, pull: f32, pulse: f32) {
    let f = bite.facing;
    let p = f.perp();
    let Some((x0, x1, y0, y1)) = bbox_lane(pb, bite, cam, 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if bite.contains(s, 0.0) {
                let d = s - bite.center;
                let u = d.dot(f);
                let v = d.dot(p);
                let stripe = ((u + v) * 0.85).sin() > 0.2;
                let base = 0.22 + 0.22 * tell + 0.16 * pull + if stripe { 0.08 } else { 0.0 };
                let flicker = (0.08 + 0.12 * pull) * pulse;
                blend_px(pb, x, y, WARN_RED, (base + flicker).min(0.68));
            }
        }
    }
    // Crisp far boundary of the 8-px bite strip so the lethal line is unmistakable.
    let far = w2s(bite.center + f * bite.half, cam);
    line_solid(
        pb,
        far - p * bite.half_perp,
        far + p * bite.half_perp,
        mix(WARN_RED, WARN_HOT, 0.35 + 0.35 * pull),
        0.55 + 0.35 * pull,
    );
    // Gleaming nip jaw-teeth ticks along the front edge.
    let near = w2s(bite.center - f * bite.half, cam);
    for k in -4..=4 {
        let q = near + p * (k as f32 * 2.4);
        let tip = q + f * (1.2 + 1.4 * pull);
        line_solid(pb, q, tip, WARN_HOT, 0.35 + 0.55 * pull);
    }
}

fn draw_bite_flash(pb: &mut PixelBuffer, bite: &MangleLane, cam: Vec2, t: f32) {
    let f = bite.facing;
    let p = f.perp();
    let a = (1.0 - t).powi(2).clamp(0.0, 1.0);
    let Some((x0, x1, y0, y1)) = bbox_lane(pb, bite, cam, 4.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if bite.contains(s, 0.0) {
                blend_px(pb, x, y, WARN_HOT, 0.72 * a);
            }
        }
    }
    // Blazing metallic clench sparks bursting across the bite strip!
    let near = w2s(bite.center - f * bite.half, cam);
    for k in -5..=5 {
        let jitter = hash01(k + 13, (t * 24.0) as i32) * 3.5;
        let q = near + p * (k as f32 * 2.1) + f * (t * 6.5 + jitter);
        line_solid(pb, q, q + f * 2.5, WARN_HOT, 0.92 * a);
    }
}

/// The wring-out rail: crisp `56 × 10` bowling-lane rails, forward chevrons & brass stop.
fn draw_wring_rail(
    pb: &mut PixelBuffer,
    lane: &MangleLane,
    cam: Vec2,
    fill: f32,
    travel: f32,
    pulse: f32,
    anim: f32,
) {
    let f = lane.facing;
    let p = f.perp();
    let near = w2s(lane.center - f * lane.half, cam);
    let far = w2s(lane.center + f * lane.half, cam);
    let col = if travel > 0.0 {
        mix(WARN_HOT, WARN_AMBER, 0.25)
    } else {
        mix(WARN_AMBER, WARN_RED, fill * 0.45)
    };
    // Subtle dark-tinted lane wash so the 10 px rail pops clearly on both lit and unlit floors.
    let Some((x0, x1, y0, y1)) = bbox_lane(pb, lane, cam, 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if lane.contains(s, 0.0) {
                blend_px(pb, x, y, col, 0.09 + 0.11 * fill + 0.08 * travel);
            }
        }
    }

    let a = 0.42 + 0.36 * fill + 0.18 * travel;
    let scroll = if travel > 0.0 { -anim * 34.0 } else { -anim * 12.0 };
    line_dashed(pb, near + p * lane.half_perp, far + p * lane.half_perp, col, a, 3.5, scroll);
    line_dashed(pb, near - p * lane.half_perp, far - p * lane.half_perp, col, a, 3.5, scroll);

    // Outward-pointing chevrons along the rail showing launch direction (+f).
    let chevrons = 6;
    for i in 0..chevrons {
        let base = (i as f32 + 0.5) / chevrons as f32;
        let t = (base + anim * (if travel > 0.0 { 1.4 } else { 0.45 })).fract();
        let u = -lane.half + t * lane.half * 2.0;
        let q = w2s(lane.center + f * u, cam);
        let tip = q + f * 2.8;
        let a1 = q - f * 0.6 + p * 2.8;
        let a2 = q - f * 0.6 - p * 2.8;
        draw_arrow(pb, tip, a1, a2, col, (0.35 + 0.38 * fill) * (0.7 + 0.3 * pulse));
    }

    // Far end bumper stop.
    let stop = w2s(lane.center + f * lane.half, cam);
    line_solid(pb, stop - p * lane.half_perp, stop + p * lane.half_perp, WARN_HOT, 0.50 + 0.35 * fill);
    fill_disc(pb, stop, 1.8, col, 0.55 + 0.30 * fill);
}

/// Two crisp brass ratchet click rings + radial click ticks around the crank during WringOut tell.
fn draw_crank_clicks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, t: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    for (i, at) in [0.46f32, 0.92f32].iter().enumerate() {
        let d = t - at;
        if (0.0..0.38).contains(&d) {
            let k = 1.0 - d / 0.38;
            let r = 4.5 + i as f32 * 2.5 + d * 18.0;
            ring(pb, c, r, WARN_HOT, 0.80 * k, 1.4);
            for s in 0..4 {
                let a = s as f32 * FRAC_PI_2 + 0.4;
                let dir = Vec2::from_angle(a);
                line_solid(pb, c + dir * (r - 1.5), c + dir * (r + 2.2), JAM_GOLD, 0.72 * k);
            }
        }
    }
    ring(pb, c, 5.0, WARN_AMBER, 0.35 + 0.30 * pulse, 1.1);
}

/// Unmistakable golden weak-point gather ring & rotating corner brackets on the dropped crank!
fn draw_crank_gather(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, t: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    let r = 6.8 + pulse * 1.8;
    // Soft warm gold glow pool beneath the crank.
    fill_disc(pb, c, r + 2.0, JAM_GOLD, 0.16 + 0.12 * pulse);
    ring(pb, c, r, JAM_GOLD, 0.65 + 0.30 * pulse, 1.5);
    ring(pb, c, 4.2, WARN_HOT, 0.72, 1.1);
    // Four rotating inward reticle chevrons ("HIT HERE x1.5").
    let spin = t * TAU * 0.8;
    for i in 0..4 {
        let a = spin + i as f32 * FRAC_PI_2;
        let dir = Vec2::from_angle(a);
        let from = c + dir * (r + 3.2);
        let to = c + dir * (r - 0.6);
        line_solid(pb, from, to, WARN_HOT, 0.82);
    }
}

/// Soapy towel puddle spilling onto the floor under a jammed nip.
fn draw_towel_puddle(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, pulse: f32) {
    if pose.towel <= 0.02 {
        return;
    }
    let nip = w2s(nip_pos_from_pose(pose), cam);
    for i in 0..5 {
        let side = (i as f32 - 2.0) * 2.6;
        let q = nip + pose.f * (2.5 + (i % 2) as f32 * 2.2) + pose.p * side;
        fill_disc(pb, q, 2.0 + 0.4 * pulse, ENAMEL_MID, 0.26 * pose.towel);
    }
}

/// Exposed springing coils popping outward from the right cheek beside the dropped crank in FinalJam.
fn draw_spring_ticks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, p: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    for i in 0..3 {
        let a = -0.65 + i as f32 * 0.65 + (p * 10.0).sin() * 0.16;
        let dir = pose.p * a.cos() + pose.f * a.sin();
        let from = c + dir * 2.5;
        let to = c + dir * (9.5 + 1.8 * pulse);
        draw_coil_spring(pb, from, to, 3, 1.2, 0.72 + 0.25 * pulse);
    }
}

fn draw_frame_shadow(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    if pose.alpha <= 0.02 {
        return;
    }
    let c = w2s(pose.pos, cam) + Vec2::new(2.2, 3.0);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, 19.0, 18.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * pose.f.x + dy * pose.f.y;
            let v = dx * pose.p.x + dy * pose.p.y;
            let d = sd_rbox(
                u,
                v,
                -BODY_CENTRE_BACK,
                0.0,
                BODY_HALF_LEN + 0.6,
                ROLLER_HALF_WIDTH + 2.0,
                2.8,
            );
            if d < 1.8 {
                let a = (0.42 * (1.0 - (d.max(0.0) / 1.8))).clamp(0.0, 0.42) * pose.alpha;
                darken_px(pb, x, y, a);
            }
        }
    }
}

/// Little iron-feet contact prints on the floor during waddle / death.
fn draw_foot_prints(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, anim: f32, alpha: f32) {
    if alpha <= 0.02 {
        return;
    }
    for (i, &(u_off, side)) in [
        (-(BODY_CENTRE_BACK + BODY_HALF_LEN - 0.5), -1.0f32),
        (-(BODY_CENTRE_BACK + BODY_HALF_LEN - 0.5), 1.0),
        (5.5, -1.0),
        (5.5, 1.0),
    ]
    .iter()
    .enumerate()
    {
        let phase = if (i == 0) || (i == 3) { 0.0 } else { PI };
        let lift = (anim * 2.6 + phase).sin().max(0.0);
        let q = w2s(pose.pos + pose.f * u_off + pose.p * (side * 10.6), cam)
            + Vec2::new(1.0, 2.2 + lift * 1.0);
        fill_disc(pb, q, 2.5 - lift * 0.5, IRON_OUTLINE, (0.30 - lift * 0.14) * alpha);
    }
}

fn nip_pos_from_pose(pose: &Pose) -> Vec2 {
    pose.pos + pose.f * NIP_OFFSET
}

fn crank_pos_from_pose(pose: &Pose) -> Vec2 {
    let hub = pose.pos + pose.f * CRANK_ALONG + pose.p * CRANK_SIDE_OFFSET;
    hub + Vec2::new(0.0, 2.4 * pose.crank_loose) + pose.p * (1.6 * pose.crank_loose)
}

// ============================================================================
// Public Entry Point 2: Entity Layer (Feet, Frame, Rollers, Screw, Crank, Belt)
// ============================================================================

/// Draw The Mangle.
///
/// **Layer**: entity — during the entity pass.
pub fn draw(pb: &mut PixelBuffer, view: &MangleView, cam: Vec2, _accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let pose = compute_pose(view);
    if pose.alpha <= 0.02 {
        return;
    }
    let hit = if view.hit_flash.is_finite() {
        view.hit_flash.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let anim = if view.anim.is_finite() { view.anim } else { 0.0 };
    let f = pose.f;
    let p = pose.p;
    let base = w2s(pose.pos, cam) + pose.shake + f * pose.lean + p * pose.rock;
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);
    let pulse = 0.5 + 0.5 * (anim * 9.5).sin();
    let rim_col = if hit > 0.45 { (56, 36, 22) } else { IRON_OUTLINE };

    // 1) Four chunky cast-iron feet peeking cleanly out under the corners so the
    //    waddling posture reads immediately in every facing!
    let waddle_scale = if matches!(view.state, MangleState::Idle) {
        1.0
    } else if matches!(view.state, MangleState::Death) {
        0.0
    } else {
        0.25
    };
    for (idx, &(u_foot, side)) in [
        (-(BODY_CENTRE_BACK + BODY_HALF_LEN + 0.4), -1.0f32),
        (-(BODY_CENTRE_BACK + BODY_HALF_LEN + 0.4), 1.0),
        (5.8, -1.0),
        (5.8, 1.0),
    ]
    .iter()
    .enumerate()
    {
        let phase = if idx == 0 || idx == 3 { 0.0 } else { PI };
        let step = (anim * 2.6 + phase).sin();
        let lift = step.max(0.0) * waddle_scale;
        let stride = step * 1.4 * waddle_scale;
        draw_iron_foot(pb, base, f, p, u_foot + stride, side, lift, hit, pose.alpha);
    }

    // 2) Cast-iron frame, cheeks, recessed roller well, two glossy candy-striped
    //    rollers, meshed brass gears, porcelain maker's badge, tension springs &
    //    threaded brass wringer screw — all with crisp dark exterior outline!
    let Some((x0, x1, y0, y1)) = bbox(pb, base, 21.0, 21.0) else {
        return;
    };
    let phase2 = pose.damage_tier >= 2;
    let phase3 = pose.damage_tier >= 3;
    let screw_skew = if phase3 { 0.08 } else { 0.0 } + pose.death * 0.25;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - base.x;
            let dy = y as f32 - base.y;
            let v = dx * p.x + dy * p.y;
            // Frame shears slightly when sagging into a jam or death collapse.
            let u = dx * f.x + dy * f.y - v * 0.06 * pose.sag;

            // Main cast-iron body box + protruding ornate curved side cheek ears.
            let d_body = sd_rbox(
                u,
                v,
                -BODY_CENTRE_BACK,
                0.0,
                BODY_HALF_LEN,
                ROLLER_HALF_WIDTH + 1.8,
                2.2,
            );
            let d_cheek_l = sd_rbox(u, v, -0.8, -(ROLLER_HALF_WIDTH + 1.6), 7.8, 1.7, 1.1);
            let d_cheek_r = sd_rbox(u, v, -0.8, ROLLER_HALF_WIDTH + 1.6, 7.8, 1.7, 1.1);
            let d_frame = d_body.min(d_cheek_l).min(d_cheek_r);

            let cover = (0.55 - d_frame * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d_frame < 1.25 {
                    blend_px(pb, x, y, rim_col, 0.92 * pose.alpha);
                }
                continue;
            }

            let nu = ((u + BODY_CENTRE_BACK) / BODY_HALF_LEN).clamp(-1.0, 1.0);
            let nv = (v / (ROLLER_HALF_WIDTH + 2.0)).clamp(-1.0, 1.0);
            let lit = nu * fl + nv * pl;
            let dome = (1.0 - (nu * nu * 0.35 + nv * nv * 0.45)).clamp(0.0, 1.0);

            // Base cast-iron housing with crisp beveled outer rim!
            let bevel = if d_frame > -1.45 {
                if lit > 0.10 { 0.30 } else { -0.24 }
            } else {
                0.0
            };
            let mut col = shade3(IRON_SH, IRON_MID, IRON_HI, 0.40 + 0.26 * dome + 0.54 * lit + bevel);
            if d_frame > -1.35 && lit > 0.35 {
                col = mix(col, IRON_SPEC, 0.55);
            }

            // Recessed roller well across the front half (`u ∈ [-2.4, 7.2]`) framed in deep shadow
            // so the twin rollers separate unmistakably even when downscaled on library cards.
            let d_well = sd_rbox(u, v, 2.40, 0.0, 4.80, ROLLER_HALF_WIDTH + 0.25, 0.9);
            if d_well < 0.0 {
                col = (8, 10, 15);
                // Meshed brass end-gears glimpsed at the cheek ends of the roller well!
                if v.abs() > ROLLER_HALF_WIDTH - 1.3 {
                    let gear_tooth = ((u * 2.4 + pose.roll * 1.2).sin()) * 0.5 + 0.5;
                    col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.40 + 0.55 * gear_tooth);
                }
            }

            // Two thick glossy candy-striped wringer rollers with bold dark nip gap between them!
            // In FinalJam / Death, rollers tilt crookedly (slack).
            let roller_tilt = v * 0.055 * pose.slack;
            for (idx, &ru_base) in [-0.25f32, 4.95f32].iter().enumerate() {
                let ru = ru_base + if idx == 0 { roller_tilt } else { -roller_tilt };
                let d_r = sd_segment(
                    u,
                    v,
                    ru,
                    -(ROLLER_HALF_WIDTH - 0.45),
                    ru,
                    ROLLER_HALF_WIDTH - 0.45,
                    2.15,
                );
                if d_r < 0.55 {
                    let r_cover = (0.55 - d_r).clamp(0.0, 1.0);
                    // Cylindrical cross-section normal across the roller's width (`u - ru`).
                    let cyl_u = ((u - ru) / 2.15).clamp(-1.0, 1.0);
                    let cyl_z = (1.0 - cyl_u * cyl_u).max(0.0).sqrt();
                    let cyl_lit = cyl_u * fl * 0.72 + nv * pl * 0.26 + cyl_z * 0.48;

                    // Bold barber-pole candy stripes wide enough to survive 2x card downscale;
                    // opposite rollers counter-rotate!
                    let spin_sign = if idx == 0 { 1.0 } else { -1.0 };
                    let phase = v * 0.62 + (u - ru) * 0.55 * spin_sign + pose.roll * spin_sign;
                    let stripe_wave = (phase * 1.05).sin();
                    let is_red = stripe_wave > 0.0;

                    let mut rcol = if is_red {
                        shade3(ROLLER_RED_SH, ROLLER_RED, ROLLER_RED_HI, 0.46 + 0.70 * cyl_lit)
                    } else {
                        shade3(ROLLER_DIM, ROLLER_CREAM, ROLLER_CREAM_HI, 0.52 + 0.72 * cyl_lit)
                    };
                    // Crisp glossy rubber specular ridge along the sunlit crown of each roller!
                    if cyl_z > 0.80 && cyl_lit > 0.30 {
                        rcol = mix(rcol, ROLLER_CREAM_HI, 0.68);
                    }
                    // Phase 2/3 rubber scuffs & nicks on the rollers!
                    if phase2 && ((v + 3.5).abs() < 1.1 && idx == 1 || (v - 4.2).abs() < 0.9 && idx == 0) {
                        rcol = mix(rcol, IRON_SH, 0.52);
                    }
                    // Crisp dark seam along the roller's top/bottom boundary.
                    if d_r > -0.20 {
                        rcol = mix(rcol, IRON_OUTLINE, 0.78);
                    }
                    col = mix(col, rcol, r_cover);
                }
            }

            // Deep dark nip crevice between the two rollers (with hot glint on Intake tell).
            let nip_u = 2.35;
            if (u - nip_u).abs() < 0.78 && v.abs() <= ROLLER_HALF_WIDTH - 0.3 {
                col = if pose.spin < -0.5 && ((v + anim * 12.0).sin() > 0.55) {
                    mix(WARN_AMBER, WARN_HOT, 0.55)
                } else {
                    (6, 7, 11)
                };
            }

            // Machined steel feed-comb lip along the front mouth (`u ≈ 7.35`).
            if (u - 7.35).abs() < 0.82 && v.abs() <= ROLLER_HALF_WIDTH {
                let tooth = ((v * 1.55).floor() as i32) & 1 == 0;
                let comb_lit = if tooth { 0.82 + 0.25 * lit } else { 0.28 };
                col = shade3(IRON_SH, IRON_HI, IRON_SPEC, comb_lit);
                if phase2 && tooth && matches!(view.state, MangleState::Intake { .. }) {
                    col = mix(col, HEAT_GLOW, 0.35 + 0.25 * pulse);
                }
            }

            // Vintage porcelain-enamel maker's plate on the back housing (`u ∈ [-10.0, -6.4]`).
            let d_badge = sd_rbox(u, v, -8.2, 0.0, 1.7, 5.4, 0.8);
            if d_badge < 0.35 {
                if d_badge > -0.35 {
                    col = IRON_OUTLINE;
                } else {
                    let e_lum = 0.48 + 0.42 * lit + 0.16 * dome;
                    col = shade3(ENAMEL_SH, ENAMEL_MID, ENAMEL_HI, e_lum);
                    // Crisp dark enamel stencil bar + brass end rivets.
                    if (u + 8.2).abs() < 0.42 && v.abs() < 3.4 {
                        col = mix(col, (26, 38, 46), 0.72);
                    }
                    if (u + 8.2).abs() < 0.55 && (v.abs() - 4.3).abs() < 0.55 {
                        col = BRASS_HI;
                    }
                    // Phase 2/3 chipped enamel crack!
                    if phase2 && (u + 7.9 - v * 0.45).abs() < 0.55 && v > -1.5 && v < 3.2 {
                        col = IRON_SH;
                    }
                }
            }

            // Rear housing battle-damage crack in Phase 3 (glowing hot internal steam/embers!).
            if phase3 && u < -2.6 && u > -10.0 && v > -7.5 && v < 4.5 {
                let crack = ((u + 6.2) - (v + 1.5) * 0.55 + (v * 1.4).sin() * 0.45).abs();
                if crack < 0.65 {
                    col = mix(IRON_OUTLINE, HEAT_GLOW, 0.55 + 0.40 * pulse);
                } else if crack < 1.3 {
                    col = mix(col, IRON_SH, 0.55);
                }
            }

            // Heavy raised cast-iron side cheeks with machined specular bevels & rivets!
            if d_cheek_l < 0.2 || d_cheek_r < 0.2 {
                let side_sign = v.signum();
                let c_lit = nu * fl * 0.45 + side_sign * pl * 0.75;
                let mut cc = shade3(IRON_SH, IRON_MID, IRON_HI, 0.48 + 0.52 * c_lit);
                if (v.abs() - (ROLLER_HALF_WIDTH + 1.5)).abs() < 0.65 && c_lit > 0.05 {
                    cc = mix(cc, IRON_SPEC, 0.68);
                }
                // Exposed helical steel tension springs seated in the cheeks at `u ≈ -4.2`!
                if (u + 4.2).abs() < 1.35 && v.abs() > ROLLER_HALF_WIDTH + 0.2 {
                    let coil = (u * 4.2).sin() * 0.5 + 0.5;
                    cc = shade3(SPRING_SH, SPRING_COL, SPRING_HI, 0.35 + 0.65 * coil);
                }
                // Brass cheek bolts!
                for &bu in &[-6.6f32, 0.8, 5.2] {
                    if sd_circle(u, v.abs(), bu, ROLLER_HALF_WIDTH + 1.6, 0.75) < 0.0 {
                        cc = BRASS_HI;
                    }
                }
                col = cc;
            }

            // Threaded brass wringer pressure screw across the top (`u ≈ -4.2`) + central T-wing knob!
            let su = -4.2 + v * screw_skew;
            let d_screw = sd_segment(u, v, su, -10.8, su, 10.8, 0.75);
            if d_screw < 0.4 {
                if d_screw > 0.05 {
                    col = mix(col, IRON_OUTLINE, 0.72);
                } else {
                    let thread = ((v * 2.4).sin() * 0.22).abs();
                    let b_lit = 0.56 + 0.42 * lit - thread;
                    col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, b_lit);
                }
            }
            for &nut_v in &[-10.6f32, 10.6] {
                if sd_circle(u, v, su, nut_v, 1.35) < 0.0 {
                    col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.65 + 0.35 * lit);
                }
            }
            // Central brass T-wing pressure knob (detaches in late death!).
            if pose.death < 0.35 {
                let d_knob = sd_circle(u, v, su, 0.0, 1.75);
                let d_w1 = sd_segment(u, v, su, -1.2, su - 2.2, -3.2, 0.65);
                let d_w2 = sd_segment(u, v, su, 1.2, su - 2.2, 3.2, 0.65);
                let d_k = d_knob.min(d_w1).min(d_w2);
                if d_k < 0.45 {
                    if d_k > 0.0 {
                        col = mix(col, IRON_OUTLINE, 0.78);
                    } else {
                        col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.68 + 0.38 * lit);
                        if d_knob < -0.7 && lit > 0.0 {
                            col = WARN_HOT;
                        }
                    }
                }
            }

            // Intact taut black rubber drive belt riding the left-flank pulley in Phase 1/2!
            if !phase3 && pose.belt_slip < 0.1 {
                let d_belt = sd_rbox(u, v, -0.5, -(ROLLER_HALF_WIDTH + 2.7), 5.6, 0.75, 0.3);
                if d_belt < 0.25 {
                    let rib = ((u + pose.roll * 1.4) * 1.6).sin() * 0.16;
                    col = shade3(BELT_SH, BELT_MID, BELT_HI, 0.48 + 0.35 * lit + rib);
                }
            }

            let final_col = apply_hit(col, hit * 0.42);
            if cover < 1.0 {
                let bg = pb.pixel_at(x as usize, y as usize);
                let rimmed = mix(bg, rim_col, 0.80 * pose.alpha);
                pb.plot(x, y, mix(rimmed, final_col, cover * pose.alpha));
            } else {
                blend_px(pb, x, y, final_col, pose.alpha);
            }
        }
    }

    // 2b) Death collapse fractures & tumbling detached brass wringer knob!
    if matches!(view.state, MangleState::Death) {
        let t = pose.death;
        for (a, b) in [(-6.5f32, 3.5f32), (-1.5, -8.0)] {
            let from = base + f * a + p * (-9.5);
            let to = base + f * b + p * 9.5;
            line_solid(pb, from, to, IRON_OUTLINE, 0.88 * pose.alpha);
            line_solid(pb, from + f, to + f, HEAT_GLOW, 0.45 * (1.0 - t) * pose.alpha);
        }
        // Detached brass wing-knob tumbling off the top!
        if t >= 0.25 {
            let kt = ((t - 0.25) / 0.75).clamp(0.0, 1.0);
            let knob = base - f * (5.0 + kt * 16.0) - p * (kt * 9.0) + Vec2::new(0.0, -(1.0 - kt) * 5.0 * (kt * PI).sin());
            fill_disc(pb, knob, 2.1, IRON_OUTLINE, 0.85 * pose.alpha);
            fill_disc(pb, knob, 1.5, BRASS_HI, 0.95 * pose.alpha);
        }
    }

    // 3) Torn rubber drive belt whipping loose off the flank in Phase 3 & Death!
    if pose.belt_slip > 0.02 {
        draw_torn_belt(pb, base, f, p, pose.belt_slip, hit, pose.alpha, anim);
    }

    // 4) Cloth bulge peeking out during WringOut tell, or HUGE jammed towel in Jam/FinalJam!
    if pose.wring_bulge > 0.05 {
        draw_towel_jam(pb, base, f, p, pose.wring_bulge * 0.42, hit, pose.alpha, anim);
    }
    if pose.towel > 0.02 {
        draw_towel_jam(pb, base, f, p, pose.towel, hit, pose.alpha, anim);
    }

    // 5) The loose brass-and-wood crank on the right flank (x1.5 weak point!).
    draw_crank(pb, base, f, p, &pose, hit, anim);

    // 6) Cel-shaded pressurized steam plumes billowing off the jammed/damaged nip!
    if pose.steam > 0.02 {
        draw_steam(pb, base, f, p, pose.steam, anim, pose.alpha);
    }

    // 7) Metallic hit-sparks when struck!
    if hit > 0.12 {
        draw_hit_sparks(pb, base, hit, anim);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_iron_foot(
    pb: &mut PixelBuffer,
    base: Vec2,
    f: Vec2,
    p: Vec2,
    u_off: f32,
    side: f32,
    lift: f32,
    hit: f32,
    alpha: f32,
) {
    let center = base + f * u_off + p * (side * 12.0) + Vec2::new(0.0, 2.5 - lift * 2.2);
    let Some((x0, x1, y0, y1)) = bbox(pb, center, 5.0, 5.0) else {
        return;
    };
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            // Chunky flared Victorian iron foot with little toe pads.
            let d_pad = sd_rbox(u, v, 0.0, side * 0.6, 2.7, 2.5, 1.0);
            let cover = (0.55 - d_pad * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d_pad < 1.05 {
                    blend_px(pb, x, y, IRON_OUTLINE, 0.80 * alpha);
                }
                continue;
            }
            let lit = (u / 2.7) * fl + (v / 2.5) * pl;
            let mut col = shade3(IRON_SH, IRON_MID, IRON_HI, 0.45 + 0.50 * lit);
            // Raised cast-iron ankle flange & gleaming brass mounting bolt!
            if sd_circle(u, v, 0.0, side * 0.4, 0.75) < 0.0 {
                col = BRASS_HI;
            } else if d_pad > -0.65 && lit > 0.1 {
                col = mix(col, IRON_SPEC, 0.40);
            }
            blend_px(pb, x, y, apply_hit(col, hit * 0.35), cover * alpha);
        }
    }
}

/// Crumpled, billowing striped laundry towel jammed & bulging out of the nip!
#[allow(clippy::too_many_arguments)]
fn draw_towel_jam(
    pb: &mut PixelBuffer,
    base: Vec2,
    f: Vec2,
    p: Vec2,
    amount: f32,
    hit: f32,
    alpha: f32,
    anim: f32,
) {
    let nip = base + f * (NIP_OFFSET - 1.2);
    let len = 9.5 * amount;
    let Some((x0, x1, y0, y1)) = bbox(pb, nip + f * (len * 0.5), 12.0, 12.0) else {
        return;
    };
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - nip.x;
            let dy = y as f32 - nip.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            if u < -0.5 || u > len + 1.5 {
                continue;
            }
            let t = (u / len.max(1.0)).clamp(0.0, 1.0);
            // Organic crumpled cloth lobes billowing outward from the rollers.
            let ruffle = (v * 0.85 + anim * 5.0).sin() * 0.75 * t;
            let half_v = (8.2 * (1.0 - t * 0.28) + (t * PI).sin() * 1.4).max(1.2);
            let du = (0.0 - u).max(u - (len + ruffle));
            let dv = v.abs() - half_v;
            let d = du.max(dv);
            let cover = (0.55 - d * 1.2).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.0 {
                    blend_px(pb, x, y, IRON_OUTLINE, 0.72 * alpha * amount);
                }
                continue;
            }
            // Sculpted cloth folds & classic woven red laundry-towel hem stripe!
            let fold = ((v * 1.35 + u * 0.55).sin() * 0.5 + 0.5).powi(2);
            let lit = (u / len.max(1.0) - 0.5) * fl * 0.4 + (v / half_v) * pl * 0.5 + fold * 0.55;
            let mut col = shade3(TOWEL_SH, TOWEL_MID, TOWEL_HI, 0.35 + 0.65 * lit);
            // Classic red woven stripe near the towel's hem!
            if (t - 0.74).abs() < 0.11 || (t - 0.88).abs() < 0.05 {
                col = mix(col, TOWEL_STRIPE, 0.78);
            }
            // Shadow right where the rollers pinch the cloth.
            if u < 1.4 {
                col = scale(col, 0.68 + 0.32 * (u / 1.4).clamp(0.0, 1.0));
            }
            blend_px(pb, x, y, apply_hit(col, hit * 0.25), cover * alpha * amount.min(1.0));
        }
    }
}

/// Torn rubber drive belt whipping loose off the side pulley in Phase 3 / Death.
#[allow(clippy::too_many_arguments)]
fn draw_torn_belt(
    pb: &mut PixelBuffer,
    base: Vec2,
    f: Vec2,
    p: Vec2,
    slip: f32,
    hit: f32,
    alpha: f32,
    anim: f32,
) {
    let hub = base + f * (CRANK_ALONG + 1.5) - p * (ROLLER_HALF_WIDTH + 2.2);
    let swing1 = (anim * 4.2).sin() * 2.2 * slip;
    let swing2 = (anim * 4.2 - 1.2).sin() * 3.2 * slip;
    let a = hub;
    let b = hub - p * (6.0 + swing1 * 0.4) - f * (2.5 + swing1 * 0.3) + Vec2::new(0.0, 4.5 * slip);
    let c = b - p * (4.5 + swing2 * 0.5) - f * (1.5 - swing2 * 0.4) + Vec2::new(0.0, 4.0 * slip);

    // Exposed grooved brass pulley wheel where the belt tore off!
    fill_disc(pb, hub, 2.5, IRON_OUTLINE, 0.85 * alpha);
    fill_disc(pb, hub, 1.8, BRASS_MID, 0.95 * alpha);
    blend_px(pb, hub.x.round() as i32, hub.y.round() as i32, IRON_SH, alpha);

    // Whipping torn rubber strap with dark outline & fabric-cord highlight!
    for &(from, to, w) in &[(a, b, 1.8f32), (b, c, 1.35)] {
        let d = to - from;
        let len = d.len();
        if len < 0.2 {
            continue;
        }
        let perp = d.perp() * (1.0 / len);
        let steps = (len * 2.2).ceil().max(2.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let q = from + d * t;
            let hw = w * 0.55 * (1.0 - t * 0.3);
            for k in -2..=2 {
                let off = k as f32 * 0.55;
                if off.abs() > hw + 0.6 {
                    continue;
                }
                let r = q + perp * off;
                let col = if off.abs() > hw {
                    IRON_OUTLINE
                } else if k == 0 {
                    BELT_HI
                } else {
                    BELT_MID
                };
                blend_px(pb, r.x.round() as i32, r.y.round() as i32, apply_hit(col, hit * 0.2), alpha * 0.96);
            }
        }
    }
    // Frayed rubber & white canvas cord strands at the torn tip.
    for k in -1i32..=1 {
        let q = c - p * (1.4 + (k.abs() as f32) * 0.6) + f * (k as f32 * 1.1);
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, TOWEL_MID, 0.85 * alpha);
    }
}

/// Outward unit direction of the right-flank crank arm in screen space.
/// Always projects strictly outside the right cheek (`dir.dot(p) > 0.0`) so the
/// crank never stabs back into the rollers or cast-iron housing in any facing.
#[inline]
fn crank_arm_dir(f: Vec2, p: Vec2, pose: &Pose, anim: f32) -> Vec2 {
    let lift = pose.crank_loose;
    if lift > 0.4 {
        let sway = 0.18 * lift + (anim * 5.0).sin() * 0.10;
        (p * sway.cos() + f * sway.sin()).normalized()
    } else {
        // Foreshortened top-down view of a crank turning on the right (`+p`) cheek:
        // extends strictly outward along `+p` while the grip orbits fore-and-aft along `f`.
        let a = pose.crank_angle + FRAC_PI_2;
        (p * (0.72 + 0.28 * a.cos()) + f * (a.sin() * 0.72)).normalized()
    }
}

/// The loose brass-and-wood crank on the right flank (the ×1.5 weak point!).
fn draw_crank(pb: &mut PixelBuffer, base: Vec2, f: Vec2, p: Vec2, pose: &Pose, hit: f32, anim: f32) {
    let hub = base + f * CRANK_ALONG + p * CRANK_SIDE_OFFSET;
    let lift = pose.crank_loose;
    let drop = Vec2::new(0.0, 2.4 * lift) + p * (1.6 * lift);
    let hub_c = hub + drop;
    let pulse = 0.5 + 0.5 * (anim * 11.0).sin();

    // Exposed brass mounting spindle linking cheek to dropped hub!
    if lift > 0.15 {
        line_solid(pb, hub - p * 1.0, hub_c, IRON_OUTLINE, 0.85 * pose.alpha);
        line_solid(pb, hub - p * 0.5, hub_c, BRASS_SH, 0.95 * pose.alpha);
    }

    // Exposed coiled spring beside the dropped crank in Jam/FinalJam.
    if lift > 0.6 {
        let coil_a = hub + Vec2::new(0.0, 3.2) + p * 1.2;
        let coil_b = coil_a + p * 4.2 + Vec2::new(0.0, 2.5);
        draw_coil_spring(pb, coil_a, coil_b, 3, 1.3, 0.85 * pose.alpha);
    }

    // Heavy brass gear-hub with crisp dark outline.
    fill_disc(pb, hub_c, 3.1, IRON_OUTLINE, 0.86 * pose.alpha);
    fill_disc(
        pb,
        hub_c,
        2.3,
        apply_hit(shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.78 + 0.25 * pose.glow), hit * 0.4),
        0.98 * pose.alpha,
    );
    blend_px(
        pb,
        (hub_c.x - 0.7).round() as i32,
        (hub_c.y - 0.7).round() as i32,
        WARN_HOT,
        (0.55 + 0.45 * pose.glow) * pose.alpha,
    );

    // Crank arm: points cleanly outward from the right cheek in every facing!
    let dir = crank_arm_dir(f, p, pose, anim);
    let arm_end = hub_c + dir * (5.6 + 2.4 * lift);
    let d = arm_end - hub_c;
    let steps = (d.len() * 2.2).ceil().max(2.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let q = hub_c + d * t;
        for k in -2i32..=2 {
            let off = k as f32 * 0.6;
            let r = q + dir.perp() * off;
            let col = if k.abs() == 2 {
                IRON_OUTLINE
            } else {
                let lum = 0.62 + 0.35 * (1.0 - (k.abs() as f32 * 0.45)) + 0.22 * pose.glow;
                apply_hit(shade3(BRASS_SH, BRASS_MID, BRASS_HI, lum), hit * 0.35)
            };
            blend_px(pb, r.x.round() as i32, r.y.round() as i32, col, 0.96 * pose.alpha);
        }
    }

    // Turned mahogany wooden crank handle with dark outline & bright lacquer highlight!
    let handle = arm_end + dir * 1.3;
    let hd = dir.perp() * 3.1;
    let hsteps: i32 = 6;
    for i in -hsteps..=hsteps {
        let t = i as f32 / hsteps as f32;
        let q = handle + hd * t;
        for b in -1..=1 {
            let r = q + dir * (b as f32 * 0.85);
            let col = if i.abs() == hsteps {
                IRON_OUTLINE
            } else if b == -1 {
                WOOD_HI
            } else if b == 0 {
                shade3(WOOD_SH, WOOD_MID, WOOD_HI, 0.72 + 0.25 * (1.0 - t.abs()))
            } else {
                WOOD_SH
            };
            blend_px(pb, r.x.round() as i32, r.y.round() as i32, apply_hit(col, hit * 0.3), 0.98 * pose.alpha);
        }
    }

    // Unmistakable hot weak-point glint & sparkle cross on the exposed crank!
    if pose.glow > 0.05 {
        let g = pose.glow * (0.65 + 0.35 * pulse);
        let hx = hub_c.x.round() as i32;
        let hy = hub_c.y.round() as i32;
        pb.plot_add(hx, hy, scale(WARN_HOT, 0.75 * g));
        for &(ox, oy) in &[(-1, 0), (1, 0), (0, -1), (0, 1)] {
            pb.plot_add(hx + ox, hy + oy, scale(JAM_GOLD, 0.65 * g));
        }
        for &(ox, oy) in &[(-2, 0), (2, 0), (0, -2), (0, 2)] {
            pb.plot_add(hx + ox, hy + oy, scale(WARN_HOT, 0.38 * g));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_steam(pb: &mut PixelBuffer, base: Vec2, f: Vec2, p: Vec2, density: f32, anim: f32, alpha: f32) {
    let count = if density > 0.65 { 5 } else { 3 };
    for i in 0..count {
        let phase = i as f32 / count as f32;
        let t = (anim * 0.95 + phase).fract();
        let lift = 0.15 + 0.85 * t;
        let u = 2.8 + (i as f32 - 2.0) * 0.6;
        let side = ((i as f32 * 2.1).sin()) * 6.5 + (t * 6.0).sin() * 1.6;
        let q = base + f * u + p * side + Vec2::new(lift * 2.2, -2.0 - lift * 11.5);
        let r = 1.6 + lift * 2.3;
        let a = density * alpha * (1.0 - t * 0.68) * 0.85;
        draw_steam_puff(pb, q, r, a);
    }
}

/// Crackling brass & steel hit sparks when the Mangle takes damage.
fn draw_hit_sparks(pb: &mut PixelBuffer, base: Vec2, hit: f32, anim: f32) {
    let seed = (anim * 24.0) as i32;
    for k in 0..6 {
        let a = k as f32 / 6.0 * TAU + hash01(seed + k * 7, 13) * 0.5;
        let dir = Vec2::from_angle(a);
        let p0 = base + dir * 10.5;
        let p1 = base + dir * (14.0 + hit * 4.5);
        line_solid(pb, p0, p1, if k % 2 == 0 { WARN_HOT } else { JAM_GOLD }, 0.9 * hit);
    }
}

/// The wring-out's rolled towel (a boss projectile) — spiral candy-striped cloth cylinder!
pub fn draw_towel(pb: &mut PixelBuffer, pos: Vec2, vel: Vec2, anim: f32, cam: Vec2, _accent: Rgb) {
    let c = w2s(pos, cam);
    let f = if vel.len_sq() > 4.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = f.perp();
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, TOWEL_RADIUS + 3.0, TOWEL_RADIUS + 3.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_rbox(u, v, 0.0, 0.0, TOWEL_RADIUS - 0.4, TOWEL_RADIUS + 0.6, 1.8);
            let cover = (0.55 - d * 1.25).clamp(0.0, 1.0);
            if cover <= 0.0 {
                if d < 1.0 {
                    blend_px(pb, x, y, IRON_OUTLINE, 0.78);
                }
                continue;
            }
            let band = ((u * 1.6 + v * 0.9 + anim * 16.0) * 0.95).sin() > 0.1;
            let lit = (u / TOWEL_RADIUS) * fl * 0.55 + (v / TOWEL_RADIUS) * pl * 0.55;
            let col = if band {
                shade3(TOWEL_SH, TOWEL_MID, TOWEL_HI, 0.55 + 0.55 * lit)
            } else {
                shade3(ROLLER_RED_SH, ROLLER_RED, ROLLER_RED_HI, 0.55 + 0.55 * lit)
            };
            blend_px(pb, x, y, col, cover);
        }
    }
    // Crisp white motion streaks & soap droplets trailing behind the spinning towel.
    for &k in &[-2.0f32, 0.0, 2.0] {
        let q = c - f * (TOWEL_RADIUS + 1.2) + p * k;
        line_solid(pb, q, q - f * 3.5, TOWEL_HI, 0.55);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_STATES: [MangleState; 8] = [
        MangleState::Idle,
        MangleState::Intake { second: false },
        MangleState::Intake { second: true },
        MangleState::WringOut,
        MangleState::Jam,
        MangleState::FinalJam,
        MangleState::PhaseShift,
        MangleState::Death,
    ];

    fn sample_view(pos: Vec2, ang: f32, state: MangleState, progress: f32) -> MangleView {
        let facing = Vec2::from_angle(ang);
        MangleView {
            pos,
            facing,
            state,
            progress,
            anim: progress * 2.4 + 0.4,
            hit_flash: 0.0,
            hp_frac: 0.7,
            target: pos + facing * INTAKE_LANE_LEN,
            phase: 1,
        }
    }

    #[test]
    fn the_mangle_draws_everywhere_without_panic() {
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
                                v.phase = 3;
                                v.hp_frac = 0.2;
                            } else if ang_i % 3 == 0 {
                                v.phase = 2;
                                v.hp_frac = 0.45;
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
    fn zero_facing_and_geometry_are_valid() {
        let mut pb = PixelBuffer::new(100, 100);
        let mut v = sample_view(Vec2::new(50.0, 50.0), 0.0, MangleState::Idle, 0.5);
        v.facing = Vec2::ZERO;
        draw_ground(&mut pb, &v, Vec2::ZERO, (255, 180, 90));
        draw(&mut pb, &v, Vec2::ZERO, (255, 180, 90));

        let shapes = hit_shapes(&v);
        assert_eq!(shapes.len(), 2);
        assert!(shapes[0].boxed && shapes[0].grounded);
        assert!(!shapes[1].boxed && shapes[1].grounded);
        // The crank sits at the right flank.
        let off = shapes[1].center - v.pos;
        assert!((off.len() - (CRANK_ALONG * CRANK_ALONG + CRANK_SIDE_OFFSET * CRANK_SIDE_OFFSET).sqrt()).abs() < 1e-3);

        // The final jam leaves the rollers slack but keeps the crank hittable.
        let mut fj = v;
        fj.state = MangleState::FinalJam;
        let shapes = hit_shapes(&fj);
        assert!(!shapes[0].grounded, "the slack rollers slip past ground weapons");
        assert!(shapes[1].grounded, "the dropped crank is the target");
    }

    #[test]
    fn the_lane_is_22_by_40_and_the_bite_is_the_last_8_px() {
        let v = sample_view(Vec2::new(100.0, 100.0), 0.0, MangleState::Intake { second: false }, 0.3);
        let lane = intake_lane(&v);
        assert_eq!(lane.half_perp * 2.0, 22.0);
        assert_eq!(lane.half * 2.0, 40.0);
        assert_eq!(lane.center, v.pos + Vec2::new(NIP_OFFSET + INTAKE_LANE_LEN * 0.5, 0.0));
        let bite = bite_strip(&v);
        assert_eq!(bite.half * 2.0, 8.0);
        // The bite is the lane's first 8 px.
        assert!(lane.contains(bite.center + Vec2::new(bite.half, 0.0), 0.0));
        assert!((bite.center.x - (v.pos.x + NIP_OFFSET + 4.0)).abs() < 1e-4);
        // The wring rail is 56 x 10.
        let rail = wring_lane(&v);
        assert_eq!(rail.half * 2.0, 56.0);
        assert_eq!(rail.half_perp * 2.0, 10.0);
        // A point on the rail but outside the intake lane is the towel's.
        let far = v.pos + Vec2::new(NIP_OFFSET + 50.0, 0.0);
        assert!(rail.contains(far, 0.0) && !lane.contains(far, 0.0));
    }

    #[test]
    fn the_art_numbers_are_the_move_spec() {
        assert_eq!(BITE_DAMAGE, 16.0);
        assert_eq!(TOWEL_DAMAGE, 18.0);
        assert_eq!(CONTACT_DAMAGE, 8.0);
        assert_eq!(JAM_DAMAGE_MULT, 1.5);
        assert_eq!(PULL_SPEED, 22.0);
        assert_eq!(INTAKE_TELL_P1, 1.10);
        assert_eq!(INTAKE_PULL_P1, 1.00);
        assert_eq!(INTAKE_TELL_P3, 0.90);
        assert_eq!(INTAKE_PULL_P3, 0.50);
        assert_eq!(WRING_TELL, 0.90);
        assert_eq!(JAM, 1.50);
        assert_eq!(FINAL_JAM, 2.20);
        assert_eq!(PHASE_SHIFT, 1.20);
        assert_eq!(DEATH_DURATION, 2.40);
        assert!((intake_duration(1) - 2.40).abs() < 1e-4);
        assert!((intake_duration(3) - 1.70).abs() < 1e-4);
    }

    #[test]
    fn hit_flash_changes_pixels() {
        let render = |flash: f32| {
            let mut pb = PixelBuffer::new(90, 90);
            pb.clear((16, 22, 20));
            let mut v = sample_view(Vec2::new(45.0, 45.0), FRAC_PI_2, MangleState::Idle, 0.3);
            v.hit_flash = flash;
            draw_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            pb
        };
        let a = render(0.0);
        let b = render(0.95);
        let mut diff = 0usize;
        for y in 0..90 {
            for x in 0..90 {
                if a.pixel_at(x, y) != b.pixel_at(x, y) {
                    diff += 1;
                }
            }
        }
        assert!(diff > 40, "the hit flash changed only {diff} pixels");
    }

    /// Gated behind `RATRUN_MANGLE_CAPTURES=<dir>`: renders every attack's
    /// tell and strike, each phase, the jams and the death through the real
    /// world draw stack (floor, lighting, HUD-free viewport). A no-op unless
    /// the variable is set, like `RATRUN_DUMP`.
    #[test]
    fn capture_mangle_gallery() {
        use crate::game::ecs::components::SpawnId;
        use crate::game::ecs::systems::boss::{Boss, BossKind};
        use crate::game::level::{MAP_COLS, MAP_ROWS, Theme, TileRect};
        use crate::game::world::World;
        use crate::render::lighting::LightingMode;

        let Some(dir) = std::env::var_os("RATRUN_MANGLE_CAPTURES") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).ok();

        fn write_ppm(path: &std::path::Path, pb: &PixelBuffer) {
            let mut data = Vec::with_capacity(32 + pb.w * pb.h * 3);
            use std::io::Write;
            writeln!(data, "P6\n{} {}\n255", pb.w, pb.h).unwrap();
            for &(r, g, b) in pb.pixels() {
                data.push(r);
                data.push(g);
                data.push(b);
            }
            std::fs::write(path, data).unwrap();
        }

        // (name, state, progress, phase, hp fraction)
        let shots: &[(&str, MangleState, f32, u8, f32)] = &[
            ("00_idle", MangleState::Idle, 0.35, 1, 1.0),
            ("01_intake_tell", MangleState::Intake { second: false }, 0.45, 1, 0.95),
            ("02_intake_pull", MangleState::Intake { second: false }, 0.80, 1, 0.95),
            ("03_intake_bite", MangleState::Intake { second: false }, 0.945, 1, 0.95),
            ("04_towel_jam", MangleState::Jam, 0.35, 1, 0.75),
            ("05_wring_tell", MangleState::WringOut, 0.30, 2, 0.55),
            ("06_wring_travel", MangleState::WringOut, 0.72, 2, 0.55),
            ("07_pair_first_lane", MangleState::Intake { second: false }, 0.55, 3, 0.22),
            ("08_pair_second_lane", MangleState::Intake { second: true }, 0.40, 3, 0.22),
            ("09_final_jam", MangleState::FinalJam, 0.50, 3, 0.18),
            ("10_phase_shift", MangleState::PhaseShift, 0.35, 2, 0.58),
            ("11_death", MangleState::Death, 0.42, 3, 0.0),
            ("12_intake_tell_east", MangleState::Intake { second: false }, 0.45, 1, 0.95),
        ];
        for &(name, state, p, phase, hp) in shots {
            let mut w = World::new(Theme::Laundry, 5);
            w.viewport = Vec2::new(176.0, 104.0);
            w.lighting = LightingMode::Full;
            w.smoke_enabled = true;
            let anchor = w.player.pos;
            let east = name.contains("east");
            let pos = if east {
                anchor - Vec2::new(26.0, 0.0)
            } else {
                anchor - Vec2::new(0.0, 26.0)
            };
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .unwrap_or(TileRect::new(0, 0, MAP_COLS, MAP_ROWS));
            let mut boss = Boss::new(BossKind::Mangle, pos, 42880.0, arena, arena.center_world(), false);
            boss.facing = if east {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::new(0.0, 1.0)
            };
            boss.phase = phase;
            boss.hp = boss.max_hp * hp;
            let duration = match state {
                MangleState::Idle => IDLE_P1,
                MangleState::Intake { .. } => {
                    if phase >= 3 {
                        intake_duration(3)
                    } else {
                        intake_duration(1)
                    }
                }
                MangleState::WringOut => WRING_TELL + WRING_TRAVEL + WRING_RECOVER,
                MangleState::Jam => JAM,
                MangleState::FinalJam => FINAL_JAM,
                MangleState::PhaseShift => PHASE_SHIFT,
                MangleState::Death => DEATH_DURATION,
            };
            boss.state = crate::game::ecs::systems::boss::BossState::Mangle(state);
            boss.duration = duration;
            boss.elapsed = duration * p;
            boss.anim = 2.2;
            boss.target = match state {
                MangleState::Intake { .. } => {
                    pos + boss.facing * (NIP_OFFSET + INTAKE_LANE_LEN)
                }
                MangleState::WringOut => pos + boss.facing * (NIP_OFFSET + TOWEL_LANE_LEN),
                _ => anchor,
            };
            w.ecs.spawn((boss, SpawnId(9001)));
            if name == "06_wring_travel" {
                // The rolled towel itself, halfway down the rail.
                let p = pos + Vec2::new(0.0, NIP_OFFSET + TOWEL_LANE_LEN * 0.45);
                w.ecs.spawn((
                    crate::game::ecs::systems::boss::BossProjectile {
                        kind: crate::game::ecs::systems::boss::BossProjectileKind::RolledTowel,
                        pos: p,
                        prev: p,
                        vel: Vec2::new(0.0, TOWEL_SPEED),
                        radius: TOWEL_RADIUS,
                        damage: TOWEL_DAMAGE,
                        life: 0.6,
                        bounces: 0,
                        anim: 0.3,
                    },
                    SpawnId(9002),
                ));
            }
            // Two frames so the lighting pass settles onto the new boss.
            let mut pb = PixelBuffer::new(176, 104);
            w.draw(&mut pb);
            w.elapsed = 30.0 + 0.08;
            w.draw(&mut pb);
            write_ppm(&dir.join(format!("{name}.ppm")), &pb);
            // A flat-lit twin, so the raw art can be read without the light pass.
            w.lighting = LightingMode::Off;
            w.smoke_enabled = false;
            w.draw(&mut pb);
            write_ppm(&dir.join(format!("{name}_off.ppm")), &pb);
        }
    }

    #[test]
    fn the_towel_projectile_draws() {
        let mut pb = PixelBuffer::new(60, 60);
        draw_towel(
            &mut pb,
            Vec2::new(30.0, 30.0),
            Vec2::new(88.0, 0.0),
            0.4,
            Vec2::ZERO,
            (90, 220, 200),
        );
        let mut lit = 0;
        for y in 0..60 {
            for x in 0..60 {
                if pb.pixel_at(x, y) != (0, 0, 0) {
                    lit += 1;
                }
            }
        }
        assert!(lit > 12, "the rolled towel drew {lit} pixels");
    }

    #[test]
    fn crank_arm_points_outward_from_cheek_in_every_facing_and_state() {
        for &state in &ALL_STATES {
            for prog in [0.0f32, 0.25, 0.55, 0.85] {
                for ang_i in 0..8 {
                    let ang = ang_i as f32 / 8.0 * TAU;
                    let v = sample_view(Vec2::new(60.0, 60.0), ang, state, prog);
                    let pose = compute_pose(&v);
                    let dir = crank_arm_dir(pose.f, pose.p, &pose, v.anim);
                    assert!(
                        dir.dot(pose.p) > 0.35,
                        "crank arm pointed inward for state={state:?} prog={prog} ang={ang}: dot={}",
                        dir.dot(pose.p)
                    );
                }
            }
        }
    }

    #[test]
    fn nonfinite_view_fields_do_not_panic() {
        let mut pb = PixelBuffer::new(90, 90);
        let mut v = sample_view(Vec2::new(45.0, 45.0), 0.0, MangleState::WringOut, 0.5);
        v.progress = f32::NAN;
        v.anim = f32::INFINITY;
        v.hit_flash = f32::NAN;
        v.hp_frac = f32::NAN;
        draw_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
        draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
    }
}
