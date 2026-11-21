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

// Cast iron + polished cheeks.
const IRON_SH: Rgb = (24, 28, 34);
const IRON_MID: Rgb = (66, 76, 88);
const IRON_HI: Rgb = (140, 154, 168);
const IRON_SPEC: Rgb = (206, 220, 232);
// Porcelain enamel badge + paint.
const ENAMEL_SH: Rgb = (94, 118, 116);
const ENAMEL_MID: Rgb = (150, 172, 168);
const ENAMEL_HI: Rgb = (206, 224, 220);
// Candy-striped roller rubber.
const ROLLER_CREAM: Rgb = (198, 186, 158);
const ROLLER_DIM: Rgb = (126, 116, 96);
const ROLLER_RED: Rgb = (168, 42, 46);
const ROLLER_RED_HI: Rgb = (226, 84, 74);
// Brass screw + crank + wooden handle.
const BRASS_SH: Rgb = (104, 70, 22);
const BRASS_MID: Rgb = (196, 146, 54);
const BRASS_HI: Rgb = (250, 216, 116);
const WOOD_SH: Rgb = (76, 44, 22);
const WOOD_MID: Rgb = (158, 104, 54);
const WOOD_HI: Rgb = (226, 172, 112);
// Towel / lint.
const TOWEL_SH: Rgb = (138, 128, 112);
const TOWEL_MID: Rgb = (216, 208, 188);
const TOWEL_HI: Rgb = (250, 246, 232);
// Torn drive belt + springs.
const BELT_SH: Rgb = (22, 22, 26);
const BELT_MID: Rgb = (58, 58, 66);
const BELT_HI: Rgb = (104, 104, 116);
const SPRING_COL: Rgb = (186, 194, 206);
// Telegraphs and jam cues.
const WARN_AMBER: Rgb = (255, 186, 64);
const WARN_RED: Rgb = (255, 72, 62);
const WARN_HOT: Rgb = (255, 240, 190);
const JAM_GOLD: Rgb = (255, 210, 96);
const STEAM_COL: Rgb = (196, 214, 212);

// ============================================================================
// Internal Pose (Zero Allocation)
// ============================================================================

#[derive(Clone, Copy)]
struct Pose {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    /// Roller stripe phase in world px along the feed axis (negative = feeding).
    roll: f32,
    /// Roller spin speed (visual only).
    spin: f32,
    /// Crank arm angle (radians around the hub in the local frame).
    crank_angle: f32,
    /// 0..1 how far the crank has swung out of its seat (jam/death).
    crank_loose: f32,
    /// 0..1 how far the drive belt has slipped off (phase 3).
    belt_slip: f32,
    /// 0..1 frame sag (jam/death).
    sag: f32,
    /// Screen-pixel shudder offset.
    shake: Vec2,
    /// 0..1 crank weak-point glow.
    glow: f32,
    /// 0..1 towel jam spill (a cloth bulge at the nip).
    towel: f32,
    /// 0..1 steam wisp density.
    steam: f32,
    /// Body opacity.
    alpha: f32,
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
    let f = unit_facing(view);
    let perp = f.perp();
    let phase = view.phase.max(1);

    let mut pose = Pose {
        pos: view.pos,
        f,
        p: perp,
        roll: anim * 1.6,
        spin: 1.6,
        crank_angle: -FRAC_PI_2 + (anim * 1.6).sin() * 0.12,
        crank_loose: 0.0,
        belt_slip: 0.0,
        sag: 0.0,
        shake: Vec2::ZERO,
        glow: 0.0,
        towel: 0.0,
        steam: 0.0,
        alpha: 1.0,
        rollers_grounded: true,
    };

    // A torn belt hangs loose in the last quarter (and after the fight).
    if phase >= 3 || matches!(view.state, MangleState::Death) {
        pose.belt_slip = 0.85;
    }

    match view.state {
        MangleState::Idle => {
            // Waddling on the little iron feet; the rollers tick over.
            let step = (anim * 2.2).sin();
            pose.shake = perp * (step * 0.45) + f * (step.abs() * 0.3);
            pose.roll = anim * 2.4;
            if phase >= 2 {
                pose.steam = 0.10;
            }
        }
        MangleState::Intake { .. } => {
            let (tell, pull) = intake_windows(phase);
            let total = intake_duration(phase);
            let tell_p = (tell / total).clamp(0.0, 1.0);
            let pull_p = ((tell + pull) / total).clamp(0.0, 1.0);
            if p < tell_p {
                // Backwards: the stripes crawl out of the nip as the lane locks.
                let t = if tell_p > 0.0 { p / tell_p } else { 1.0 };
                pose.roll = 10.0 - t * 18.0;
                pose.spin = -1.0;
                pose.shake = perp * ((anim * 26.0).sin() * 0.4 * t);
                pose.steam = 0.05 + 0.10 * t;
            } else if p < pull_p {
                // Feeding: fast stripes into the nip, the frame leans in.
                let t = ((p - tell_p) / (pull_p - tell_p).max(1e-4)).clamp(0.0, 1.0);
                pose.roll = -(anim * 26.0) - t * 6.0;
                pose.spin = 3.0;
                pose.shake = f * (0.35 + 0.4 * (anim * 30.0).sin().abs());
                pose.steam = 0.10;
            } else {
                // The nip clenches: a short, heavy jolt.
                pose.roll = 0.0;
                pose.spin = 0.0;
                pose.shake = f * (0.9 * (1.0 - p).max(0.0));
            }
        }
        MangleState::WringOut => {
            let tell_p = (WRING_TELL / (WRING_TELL + WRING_TRAVEL + WRING_RECOVER)).clamp(0.0, 1.0);
            let travel_p =
                ((WRING_TELL + WRING_TRAVEL) / (WRING_TELL + WRING_TRAVEL + WRING_RECOVER))
                    .clamp(0.0, 1.0);
            if p < tell_p {
                let t = if tell_p > 0.0 { p / tell_p } else { 1.0 };
                // Two ratchet clicks at 0.45 s and 0.90 s.
                let clicks = (t * 2.0).floor();
                let within = (t * 2.0).fract();
                pose.crank_angle = -FRAC_PI_2 - clicks * (FRAC_PI_2 * 0.8) - within * 0.10;
                pose.shake = f * ((anim * 18.0).sin() * 0.25 * (1.0 - within));
                pose.roll = -t * 5.0;
                pose.spin = 0.6;
            } else if p < travel_p {
                pose.crank_angle = -FRAC_PI_2 - 2.0 * (FRAC_PI_2 * 0.8);
                pose.roll = -(anim * 20.0);
                pose.spin = 2.4;
                pose.shake = f * 0.3;
            } else {
                pose.crank_loose = 0.35;
                pose.glow = 0.55;
            }
        }
        MangleState::Jam => {
            pose.towel = 1.0;
            pose.crank_loose = 1.0;
            pose.sag = 0.25;
            pose.steam = 0.7;
            pose.glow = 1.0;
            pose.roll = (anim * 0.8).sin() * 1.2 - 1.0;
            pose.spin = 0.2;
            pose.shake = perp * ((anim * 34.0).sin() * 0.5);
        }
        MangleState::FinalJam => {
            pose.towel = 1.0;
            pose.crank_loose = 1.0;
            pose.sag = 0.55;
            pose.belt_slip = 1.0;
            pose.steam = 0.9;
            pose.glow = 1.0;
            pose.roll = (anim * 0.5).sin() * 2.0;
            pose.spin = 0.1;
            pose.shake = perp * ((anim * 40.0).sin() * 0.7);
            pose.rollers_grounded = false;
        }
        MangleState::PhaseShift => {
            // A heavy shudder, then the machine settles into its new pace.
            let t = smoothstep(p.min(0.55) / 0.55);
            let fade = 1.0 - smoothstep(((p - 0.55) / 0.45).clamp(0.0, 1.0));
            pose.shake = perp * ((anim * 46.0).sin() * 1.6 * fade);
            pose.sag = 0.20 * t;
            pose.steam = 0.55 * fade;
            pose.crank_loose = 0.5 * t;
            pose.roll = -t * 4.0;
            pose.spin = 0.4;
            pose.glow = 0.4 * t;
        }
        MangleState::Death => {
            let t = p.clamp(0.0, 1.0);
            pose.towel = 1.0 - smoothstep(((t - 0.2) / 0.5).clamp(0.0, 1.0));
            pose.crank_loose = 1.0;
            pose.sag = 0.9 * smoothstep((t / 0.6).clamp(0.0, 1.0));
            pose.belt_slip = 1.0;
            pose.steam = 0.8 * (1.0 - t);
            pose.glow = 1.0 - smoothstep((t / 0.35).clamp(0.0, 1.0));
            pose.spin = (1.0 - t) * 1.2;
            pose.roll = anim * pose.spin;
            pose.shake = f * ((anim * 30.0).sin() * 0.9 * (1.0 - t));
            pose.rollers_grounded = false;
            pose.alpha = if t < 0.55 {
                1.0
            } else {
                1.0 - smoothstep(((t - 0.55) / 0.45).clamp(0.0, 1.0))
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

/// Warm-gold hit flash that keeps local contrast.
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

/// Deterministic hash in `0..1` for floor dressing.
#[inline]
fn hash01(ix: i32, iy: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x9E37_79B1) ^ (iy as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Screen-space dashed line (world-space endpoints converted by the caller).
fn line_dashed(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, alpha: f32, dash: f32, phase: f32) {
    let d = b - a;
    let len = d.len();
    if len <= 0.1 {
        return;
    }
    let dir = d * (1.0 / len);
    let period = dash * 1.7;
    let mut t = phase.rem_euclid(period) - period;
    while t < len {
        let s0 = t.max(0.0);
        let s1 = (t + dash).min(len);
        let mut s = s0;
        while s < s1 {
            let q = a + dir * s;
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
            s += 0.8;
        }
        t += period;
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
    if radius <= 0.0 {
        return;
    }
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius + 1.5, radius + 1.5) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            let band = (d - radius).abs();
            if band <= thickness * 0.5 {
                let a = alpha * (1.0 - (band / (thickness * 0.5 + 1e-3)).clamp(0.0, 1.0));
                blend_px(pb, x, y, col, a);
            }
        }
    }
}

fn fill_disc(pb: &mut PixelBuffer, center: Vec2, radius: f32, col: Rgb, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox(pb, center, radius + 1.0, radius + 1.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= radius {
                let a = alpha * (1.0 - (d / radius).powi(2) * 0.4);
                blend_px(pb, x, y, col, a);
            }
        }
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
    let p = view.progress.clamp(0.0, 1.0);
    let pulse = 0.5 + 0.5 * (view.anim * 9.0).sin();
    let c = w2s(pose.pos, cam);

    // 1) The intake mat: the machine's own dressing (a faded rubber mat with a
    //    porcelain rim) — the Laundry's arena furniture, drawn under everything.
    draw_intake_mat(pb, &pose, cam);

    // 2) Loose suds and lint around the feet: floor dressing, deterministic.
    draw_floor_lint(pb, c, view.anim, pose.alpha);

    // 3) Diegetic ground HP arc.
    if !matches!(view.state, MangleState::Death) && view.hp_frac > 0.0 {
        draw_hp_arc(pb, c + Vec2::new(0.0, 16.5), view.hp_frac, view.phase, pulse);
    }

    // 4) State telegraphs and impacts.
    match view.state {
        MangleState::Intake { .. } => {
            let (tell, pull) = intake_windows(view.phase);
            let total = intake_duration(view.phase);
            let tell_s = tell;
            let pull_s = tell + pull;
            let lane = intake_lane(view);
            let bite = bite_strip(view);
            if (p * total) < tell_s {
                let t = ((p * total) / tell_s).clamp(0.0, 1.0);
                draw_intake_lane(pb, &lane, cam, t, 0.0, -1.0, pulse);
                draw_bite_tell(pb, &bite, cam, t, 0.0, pulse);
            } else if (p * total) < pull_s {
                let t = (((p * total) - tell_s) / pull).clamp(0.0, 1.0);
                draw_intake_lane(pb, &lane, cam, 1.0, t, view.anim * 26.0, pulse);
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
                draw_wring_rail(pb, &lane, cam, t, 0.0, pulse);
                draw_crank_clicks(pb, &pose, cam, t, pulse);
            } else if now < travel_s {
                let t = ((now - tell_s) / WRING_TRAVEL).clamp(0.0, 1.0);
                draw_wring_rail(pb, &lane, cam, 1.0, t, pulse);
                draw_crank_clicks(pb, &pose, cam, 1.0, pulse);
            } else {
                let t = ((now - travel_s) / WRING_RECOVER).clamp(0.0, 1.0);
                draw_wring_rail(pb, &lane, cam, 1.0 - t * 0.6, 1.0, pulse);
                draw_crank_gather(pb, &pose, cam, t, pulse);
            }
        }
        MangleState::Jam | MangleState::FinalJam => {
            draw_crank_gather(pb, &pose, cam, p, pulse);
            draw_towel_puddle(pb, &pose, cam, pulse);
            if matches!(view.state, MangleState::FinalJam) {
                draw_spring_ticks(pb, &pose, cam, p, pulse);
            }
        }
        MangleState::PhaseShift => {
            let t = (p / 0.55).clamp(0.0, 1.0);
            if p < 0.55 {
                ring(pb, c, 16.0 + t * 14.0, WARN_AMBER, 0.55 * (1.0 - t * 0.5), 1.2);
                ring(pb, c, 22.0, WARN_RED, 0.30 + 0.25 * pulse, 1.0);
            } else {
                let k = 1.0 - ((p - 0.55) / 0.45).clamp(0.0, 1.0);
                ring(pb, c, 16.0 + k * 10.0, WARN_HOT, 0.35 * k, 1.0);
            }
            // A rubber-belt slap on the floor beside the cheek.
            let cheek = c + pose.p * 15.0;
            for i in 0..3 {
                let q = cheek + Vec2::new(0.0, i as f32 * 2.4);
                blend_px(pb, q.x.round() as i32, q.y.round() as i32, WARN_AMBER, 0.35);
            }
        }
        MangleState::Death => {
            let t = p.clamp(0.0, 1.0);
            if t < 0.45 {
                ring(pb, c, 12.0 + t * 30.0, WARN_RED, 0.50 * (1.0 - t / 0.45), 1.4);
            }
            // The popped springs ping across the floor.
            for i in 0..4 {
                let a = -PI * 0.15 + i as f32 * 0.55;
                let r = 6.0 + t * (22.0 + i as f32 * 3.0);
                let q = c + Vec2::from_angle(a) * r;
                let x = q.x.round() as i32;
                let y = q.y.round() as i32;
                blend_px(pb, x, y, SPRING_COL, 0.65 * (1.0 - t));
            }
        }
        MangleState::Idle => {}
    }

    // 5) Drop shadow under the frame, and the iron feet's own shadows.
    draw_frame_shadow(pb, &pose, cam);
    if matches!(
        view.state,
        MangleState::Idle | MangleState::PhaseShift | MangleState::Death
    ) {
        draw_foot_prints(pb, &pose, cam, view.anim, pose.alpha);
    }

    // 6) Arena rim tint: a faint porcelain edge around the mat, tying the boss
    //    to the Laundry's floor.
    let rim = mix(view_hint_palette(accent), accent, 0.35);
    ring(pb, c, 21.0, rim, 0.10, 0.8);
}

#[inline]
fn view_hint_palette(accent: Rgb) -> Rgb {
    mix(accent, ENAMEL_MID, 0.5)
}

/// The faded rubber intake mat the machine lives on.
fn draw_intake_mat(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    let c = w2s(pose.pos, cam) + Vec2::new(0.0, 1.0);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, 22.0, 20.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * pose.f.x + dy * pose.f.y;
            let v = dx * pose.p.x + dy * pose.p.y;
            let d = sd_rbox(u, v, 0.0, 0.0, 15.0, 12.5, 4.0);
            if d < 1.2 {
                let a = (0.18 * (1.0 - (d.max(0.0) / 1.2))).clamp(0.0, 0.18);
                darken_px(pb, x, y, a);
            }
            // Ribbed rubber grooves.
            if d < -1.0 && (u * 1.2).sin().abs() > 0.88 {
                let a = 0.10 * pose.alpha;
                let bg = pb.pixel_at(x as usize, y as usize);
                pb.plot(x, y, mix(bg, IRON_SH, a));
            }
        }
    }
}

/// Loose soap suds and lint (deterministic, drawn once per frame).
fn draw_floor_lint(pb: &mut PixelBuffer, c: Vec2, anim: f32, alpha: f32) {
    if alpha <= 0.02 {
        return;
    }
    for i in 0..9 {
        let hx = hash01(i * 31 + 7, i * 17 + 3);
        let hy = hash01(i * 13 + 5, i * 29 + 11);
        let a = (0.10 + 0.14 * hx) * alpha;
        let r = 1.2 + 1.8 * hy;
        let drift = (anim * 0.4 + hx * TAU).sin() * 1.2;
        let q = c + Vec2::new((hx - 0.5) * 36.0 + drift, (hy - 0.5) * 30.0 + 6.0);
        fill_disc(pb, q, r, ENAMEL_MID, a);
    }
}

fn draw_hp_arc(pb: &mut PixelBuffer, c: Vec2, hp: f32, phase: u8, pulse: f32) {
    let col = if phase >= 3 {
        mix(WARN_RED, WARN_HOT, 0.25 + pulse * 0.2)
    } else if phase == 2 {
        mix(WARN_AMBER, WARN_RED, 0.35)
    } else {
        mix(ENAMEL_MID, WARN_AMBER, 0.5)
    };
    let r = 17.5;
    let a0 = -PI * 0.78;
    let span = PI * 1.56;
    // Segment ticks: 20 segments, the lit ones show the remaining pool.
    let segs = 20;
    for i in 0..segs {
        let frac = i as f32 / segs as f32;
        let lit = frac < hp;
        let a = a0 + span * frac;
        let q = c + Vec2::from_angle(a) * r;
        let x = q.x.round() as i32;
        let y = q.y.round() as i32;
        if lit {
            blend_px(pb, x, y, col, 0.85);
            if lit && frac + 1.0 / segs as f32 > hp {
                // The last sliver pulses.
                blend_px(pb, x, y, WARN_HOT, 0.35 + 0.35 * pulse);
            }
        } else {
            blend_px(pb, x, y, IRON_SH, 0.55);
        }
    }
}

/// The intake lane: dashed outline, chevrons crawling toward the nip, and a
/// feed-direction highlight during the pull.
fn draw_intake_lane(
    pb: &mut PixelBuffer,
    lane: &MangleLane,
    cam: Vec2,
    fill: f32,
    pull: f32,
    scroll: f32,
    pulse: f32,
) {
    let f = lane.facing;
    let p = f.perp();
    let near = w2s(lane.center - f * lane.half, cam);
    let far = w2s(lane.center + f * lane.half, cam);
    let col = if pull > 0.0 {
        mix(WARN_RED, WARN_HOT, 0.30 + 0.25 * pulse)
    } else {
        mix(WARN_AMBER, WARN_RED, fill * 0.45)
    };
    let edge = 0.28 + 0.42 * fill + 0.20 * pull;
    // Outline (two rails + the far mouth).
    line_dashed(pb, near + p * lane.half_perp, far + p * lane.half_perp, col, edge, 4.0, scroll);
    line_dashed(pb, near - p * lane.half_perp, far - p * lane.half_perp, col, edge, 4.0, scroll);
    line_dashed(
        pb,
        far - p * lane.half_perp,
        far + p * lane.half_perp,
        col,
        edge * 0.9,
        3.0,
        0.0,
    );
    // Faint lane fill.
    let Some((x0, x1, y0, y1)) = bbox(pb, w2s(lane.center, cam), lane.half + 2.0, lane.half_perp + 2.0)
    else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if lane.contains(s, 0.0) {
                let a = 0.10 + 0.14 * fill + 0.08 * pull;
                blend_px(pb, x, y, col, a);
            }
        }
    }
    // Chevrons: five arrows, scrolling toward the nip during the pull.
    let rows = 5;
    for i in 0..rows {
        let base = (i as f32 + 0.5) / rows as f32;
        let t = if pull > 0.0 {
            (base + scroll * 0.03).rem_euclid(1.0)
        } else {
            base
        };
        let u = -lane.half + t * lane.half * 2.0;
        let q = w2s(lane.center + f * u, cam);
        // Chevron pointing toward the nip (-f).
        let tip = q - f * 3.0;
        let a1 = q - f * 0.2 + p * 3.2;
        let a2 = q - f * 0.2 - p * 3.2;
        let a = (0.40 + 0.34 * fill) * (1.0 - pull * 0.10);
        draw_arrow(pb, tip, a1, a2, col, a);
        // The conveyor's rungs across the lane.
        let rail = w2s(lane.center + f * u, cam);
        for k in -2..=2 {
            let r = rail + p * (k as f32 * 3.0);
            blend_px(pb, r.x.round() as i32, r.y.round() as i32, col, a * 0.5);
        }
    }
}

fn draw_arrow(pb: &mut PixelBuffer, tip: Vec2, a: Vec2, b: Vec2, col: Rgb, alpha: f32) {
    for (from, to) in [(a, tip), (tip, b)] {
        let d = to - from;
        let steps = d.len().ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let q = from + d * t;
            blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, alpha);
        }
    }
}

/// The bite strip: a pale warning wash that snaps to a white flash.
fn draw_bite_tell(pb: &mut PixelBuffer, bite: &MangleLane, cam: Vec2, tell: f32, pull: f32, pulse: f32) {
    let f = bite.facing;
    let p = f.perp();
    let c = w2s(bite.center, cam);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, bite.half + 2.0, bite.half_perp + 1.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if bite.contains(s, 0.0) {
                let base = 0.18 + 0.24 * tell + 0.12 * pull;
                let flicker = if pull > 0.0 {
                    0.12 * pulse * pull
                } else {
                    0.06 * pulse * tell
                };
                blend_px(pb, x, y, WARN_RED, base + flicker);
            }
        }
    }
    // The nip's teeth ticks across the strip.
    let near = w2s(bite.center - f * bite.half, cam);
    for k in -4..=4 {
        let q = near + p * (k as f32 * 2.6);
        blend_px(
            pb,
            q.x.round() as i32,
            q.y.round() as i32,
            WARN_HOT,
            0.20 + 0.45 * pull,
        );
    }
}

fn draw_bite_flash(pb: &mut PixelBuffer, bite: &MangleLane, cam: Vec2, t: f32) {
    let f = bite.facing;
    let p = f.perp();
    let c = w2s(bite.center, cam);
    let a = (1.0 - t).clamp(0.0, 1.0);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, bite.half + 3.0, bite.half_perp + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = Vec2::new(x as f32, y as f32) + cam;
            if bite.contains(s, 0.0) {
                blend_px(pb, x, y, WARN_HOT, 0.55 * a);
            }
        }
    }
    // Spark teeth along the nip.
    let near = w2s(bite.center - f * bite.half, cam);
    for k in -5..=5 {
        let jitter = hash01(k, (t * 30.0) as i32) * 1.6;
        let q = near + p * (k as f32 * 2.2) - f * jitter;
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, WARN_HOT, 0.85 * a);
    }
}

/// The wring-out rail: two dashed rails and a scrolling roller tread.
fn draw_wring_rail(pb: &mut PixelBuffer, lane: &MangleLane, cam: Vec2, fill: f32, travel: f32, pulse: f32) {
    let f = lane.facing;
    let p = f.perp();
    let near = w2s(lane.center - f * lane.half, cam);
    let far = w2s(lane.center + f * lane.half, cam);
    let col = if travel > 0.0 {
        mix(WARN_HOT, WARN_AMBER, 0.25)
    } else {
        mix(WARN_AMBER, WARN_RED, fill * 0.35)
    };
    let a = 0.26 + 0.36 * fill + 0.20 * travel;
    line_dashed(pb, near + p * lane.half_perp, far + p * lane.half_perp, col, a, 3.0, 0.0);
    line_dashed(pb, near - p * lane.half_perp, far - p * lane.half_perp, col, a, 3.0, 0.0);
    // Rungs across the rail, scrolling outward as the towel travels.
    let rungs = 7;
    for i in 0..rungs {
        let base = (i as f32 + 0.5) / rungs as f32;
        let t = if travel > 0.0 {
            (base + travel).rem_euclid(1.0)
        } else {
            base
        };
        let u = -lane.half + t * lane.half * 2.0;
        let q = w2s(lane.center + f * u, cam);
        for k in -1..=1 {
            let r = q + p * (k as f32 * 2.4);
            blend_px(
                pb,
                r.x.round() as i32,
                r.y.round() as i32,
                col,
                (0.18 + 0.30 * fill) * (0.6 + 0.4 * pulse),
            );
        }
    }
    // End stop.
    let stop = w2s(lane.center + f * lane.half, cam);
    fill_disc(pb, stop, 1.8, col, 0.35 + 0.25 * fill);
}

/// Two concentric ratchet rings on the crank: the tell's two clicks.
fn draw_crank_clicks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, t: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    for (i, at) in [0.5f32, 1.0f32].iter().enumerate() {
        let d = (t - at).abs();
        if d < 0.35 {
            let k = 1.0 - d / 0.35;
            ring(pb, c, 6.0 + i as f32 * 3.0 + d * 6.0, JAM_GOLD, 0.55 * k, 1.0);
        }
    }
    ring(pb, c, 4.6, WARN_AMBER, 0.25 + 0.25 * pulse, 0.9);
}

/// The exposed crank's gather ring: "hit here".
fn draw_crank_gather(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, t: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    let r = 7.0 + pulse * 2.2;
    ring(pb, c, r, JAM_GOLD, 0.45 + 0.30 * pulse, 1.2);
    ring(pb, c, 3.6, WARN_HOT, 0.55, 1.0);
    // Four inward ticks, a weak-point reticle.
    let spin = t * TAU * 0.7;
    for i in 0..4 {
        let a = spin + i as f32 * FRAC_PI_2;
        let from = c + Vec2::from_angle(a) * (r + 2.5);
        let to = c + Vec2::from_angle(a) * (r - 1.0);
        draw_arrow(pb, to, from, from, JAM_GOLD, 0.5);
    }
}

/// The towel jam's spill on the floor under the nip.
fn draw_towel_puddle(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, pulse: f32) {
    if pose.towel <= 0.02 {
        return;
    }
    let nip = w2s(nip_pos_from_pose(pose), cam);
    for i in 0..5 {
        let side = (i as f32 - 2.0) * 2.2;
        let q = nip + pose.f * (1.8 + i as f32 * 1.1) + pose.p * side;
        fill_disc(pb, q, 1.6 + 0.5 * pulse, TOWEL_MID, 0.24 * pose.towel);
    }
}

/// The springs' ticks: the final jam spits short cable arcs.
fn draw_spring_ticks(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, p: f32, pulse: f32) {
    let c = w2s(crank_pos_from_pose(pose), cam);
    for i in 0..3 {
        let a = -FRAC_PI_2 + (i as f32 - 1.0) * 0.5 + (p * 6.0).sin() * 0.12;
        let from = c + Vec2::from_angle(a) * 3.0;
        let mid = c + Vec2::from_angle(a + 0.35) * 6.5;
        let to = c + Vec2::from_angle(a + 0.15) * 9.0;
        draw_arrow(pb, mid, from, from, SPRING_COL, 0.30 + 0.30 * pulse);
        draw_arrow(pb, to, mid, mid, SPRING_COL, 0.28 + 0.25 * pulse);
    }
}

fn draw_frame_shadow(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2) {
    if pose.alpha <= 0.02 {
        return;
    }
    let c = w2s(pose.pos, cam) + Vec2::new(2.0, 2.6);
    let Some((x0, x1, y0, y1)) = bbox(pb, c, 16.0, 15.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * pose.f.x + dy * pose.f.y;
            let v = dx * pose.p.x + dy * pose.p.y;
            let d = sd_rbox(u, v, -BODY_CENTRE_BACK, 0.0, BODY_HALF_LEN, ROLLER_HALF_WIDTH + 1.2, 2.4);
            if d < 1.4 {
                let a = (0.32 * (1.0 - (d.max(0.0) / 1.4))).clamp(0.0, 0.32) * pose.alpha;
                darken_px(pb, x, y, a);
            }
        }
    }
}

/// Little iron-feet prints: the waddle's footfalls.
fn draw_foot_prints(pb: &mut PixelBuffer, pose: &Pose, cam: Vec2, anim: f32, alpha: f32) {
    if alpha <= 0.02 {
        return;
    }
    for side in [-1.0f32, 1.0] {
        let step = (anim * 2.2 + if side < 0.0 { PI } else { 0.0 }).sin();
        let lift = step.max(0.0);
        let q = w2s(
            pose.pos - pose.f * (BODY_CENTRE_BACK + BODY_HALF_LEN - 1.0) + pose.p * (side * 9.0),
            cam,
        ) + Vec2::new(0.0, 3.0 + lift * 1.6);
        let a = (0.22 - lift * 0.12) * alpha;
        fill_disc(pb, q, 2.6 - lift * 0.6, IRON_SH, a);
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
// Public Entry Point 2: Entity Layer (Frame, Rollers, Crank, Towel, Belt)
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
    let hit = view.hit_flash.clamp(0.0, 1.0);
    let f = pose.f;
    let p = pose.p;
    let base = w2s(pose.pos, cam) + pose.shake;
    let fl = f.dot(LIGHT);
    let pl = p.dot(LIGHT);

    // 1) The iron feet first (they sit under the frame).
    for side in [-1.0f32, 1.0] {
        let step = (view.anim * 2.2 + if side < 0.0 { PI } else { 0.0 }).sin();
        let lift = step.max(0.0) * if matches!(view.state, MangleState::Idle) { 1.0 } else { 0.35 };
        draw_iron_foot(pb, base, f, p, side, lift, hit, pose.alpha);
    }

    // 2) Frame + rollers + screw in one local-space pass.
    let Some((x0, x1, y0, y1)) = bbox(pb, base, 20.0, 19.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - base.x;
            let dy = y as f32 - base.y;
            let v = dx * p.x + dy * p.y;
            // The frame shears as it sags (a broken machine slumps).
            let u = dx * f.x + dy * f.y - v * 0.05 * pose.sag;

            // The frame: cast iron with a porcelain-enamel badge on the back.
            let d_frame = sd_rbox(
                u,
                v,
                -BODY_CENTRE_BACK,
                0.0,
                BODY_HALF_LEN,
                ROLLER_HALF_WIDTH + 1.6,
                2.2,
            );
            if d_frame > 0.9 {
                continue;
            }
            let rim = (1.0 - (d_frame.max(0.0) / 0.9)).clamp(0.0, 1.0);
            let lum = 0.44 + 0.26 * (u / 9.0).clamp(-1.0, 1.0) * fl
                + 0.30 * (v / 12.0).clamp(-1.0, 1.0) * pl
                + 0.10 * rim;
            let mut col = shade3(IRON_SH, IRON_MID, IRON_HI, lum);

            // The roller well: a dark recess across the front half.
            let d_well = sd_rbox(u, v, 3.1, 0.0, 3.4, ROLLER_HALF_WIDTH + 0.2, 1.2);
            if d_well < 0.0 {
                let recess = shade3(IRON_SH, (40, 46, 52), IRON_MID, 0.24 + 0.18 * rim);
                col = mix(col, recess, 0.85);
            }

            // Two candy-striped rollers, turning (the stripe phase is `roll`).
            for (idx, ru) in [1.4f32, 4.7f32].iter().enumerate() {
                let d = sd_segment(
                    u,
                    v,
                    *ru,
                    -ROLLER_HALF_WIDTH,
                    *ru,
                    ROLLER_HALF_WIDTH,
                    2.0,
                );
                if d < 1.0 {
                    let cover = (1.0 - (d.max(0.0) / 1.0)).clamp(0.0, 1.0);
                    let spin_dir = if pose.spin < 0.0 { -1.0 } else { 1.0 };
                    // Candy stripes spiral around the roller: phase runs along
                    // its length (v) with a slight twist across the face (u).
                    let phase = (v * 0.95 + (u - *ru) * 0.55) + pose.roll * spin_dir;
                    let band = (phase * 0.75).floor();
                    let stripe = (band as i32) & 1 == 0;
                    let lit = 0.62 + 0.30 * (if stripe { 0.6 } else { 0.0 })
                        + 0.20 * (v / 11.0).clamp(-1.0, 1.0) * pl
                        + 0.14 * rim;
                    let base_col = if stripe {
                        shade3(ROLLER_RED, ROLLER_RED_HI, (255, 168, 150), lit)
                    } else {
                        shade3(ROLLER_DIM, ROLLER_CREAM, TOWEL_HI, lit)
                    };
                    // The roller's cylindrical shading across its width.
                    let cap = (1.0 - (d.max(0.0) / 0.95).powi(2) * 0.35).clamp(0.0, 1.0);
                    col = mix(col, scale(base_col, 0.94 + 0.10 * cap), cover);
                    let _ = idx;
                }
            }

            // The nip line between the rollers.
            if (u - 3.05).abs() < 0.45 && v.abs() <= ROLLER_HALF_WIDTH {
                col = mix(col, (10, 12, 14), 0.82);
            }

            // The feed lip: a bright steel comb of teeth where the bite lands.
            if (u - 6.9).abs() < 0.75 && v.abs() <= ROLLER_HALF_WIDTH {
                let tooth = ((v * 1.4).floor() as i32) & 1 == 0;
                let lit = if tooth { 0.78 + 0.18 * rim } else { 0.38 };
                col = shade3(IRON_MID, IRON_HI, IRON_SPEC, lit);
            }

            // A small porcelain maker's plate on the back plate (not a face:
            // one line of lettering above three brass rivets).
            let d_badge = sd_rbox(u, v, -6.8, 0.0, 1.5, 4.4, 0.7);
            if d_badge < 0.0 {
                let e = 0.46 + 0.22 * rim;
                col = shade3(ENAMEL_SH, ENAMEL_MID, ENAMEL_HI, e);
                if (u + 6.8).abs() < 0.35 && v.abs() < 2.8 {
                    col = mix(col, (46, 52, 54), 0.55);
                }
                if (u + 7.7).abs() < 0.4 && v.abs() < 0.5 {
                    col = mix(col, BRASS_HI, 0.7);
                }
            }

            // The brass wringer screw across the top: a threaded rod with a
            // nut at each cheek and a wing knob in the middle.
            let d_screw = sd_segment(u, v, -3.0, -10.2, -3.0, 10.2, 0.55);
            if d_screw < 0.15 {
                let lum_b = 0.60 + 0.30 * (v / 10.0).clamp(-1.0, 1.0) * pl;
                let thread = ((v * 1.9).sin() * 0.10).abs();
                col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, lum_b - thread);
            }
            for nut_v in [-10.2f32, 10.2] {
                if sd_circle(u, v, -3.0, nut_v, 0.95) < 0.05 {
                    col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.72 + 0.18 * rim);
                }
            }
            let d_knob = sd_circle(u, v, -3.0, 0.0, 1.35);
            if d_knob < 0.1 {
                col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.75 + 0.3 * rim);
            }
            // The knob's two wings.
            if sd_segment(u, v, -3.0, -1.2, -5.0, -2.0, 0.45) < 0.1
                || sd_segment(u, v, -3.0, 1.2, -5.0, 2.0, 0.45) < 0.1
            {
                col = shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.62);
            }

            // Side cheeks: polished edges that catch the light.
            let cheek_v = if v >= 0.0 {
                ROLLER_HALF_WIDTH + 1.2
            } else {
                -(ROLLER_HALF_WIDTH + 1.2)
            };
            let d_cheek = sd_rbox(u, v, -BODY_CENTRE_BACK, cheek_v, 7.0, 1.4, 0.8);
            if d_cheek < 0.4 {
                let c_lum = 0.56 + 0.26 * rim + 0.20 * pl * v.signum();
                col = shade3(IRON_MID, IRON_HI, IRON_SPEC, c_lum);
            }

            // Rivets along the cheeks.
            for k in -1..=1 {
                let rq = (u - (k as f32 * 4.2 + 0.5)).abs()
                    + (v.abs() - (ROLLER_HALF_WIDTH + 1.2)).abs();
                if rq < 0.75 {
                    col = mix(col, IRON_SPEC, 0.65);
                }
            }

            col = apply_hit(col, hit * 0.30);
            blend_px(pb, x, y, col, pose.alpha);
        }
    }

    // 2b) A death break: two dark cracks and the belt's slack.
    if matches!(view.state, MangleState::Death) {
        for (a, b) in [
            (-6.0f32, 2.0f32),
            (-2.0, -7.0),
        ] {
            let from = base + f * a + p * (-9.0);
            let to = base + f * b + p * 9.0;
            draw_arrow(pb, to, from, from, (12, 14, 16), 0.75);
        }
    }

    // 3) The drive belt: torn and hanging in the last quarter.
    if pose.belt_slip > 0.02 {
        draw_torn_belt(pb, base, f, p, pose.belt_slip, hit, pose.alpha, view.anim);
    }

    // 4) The towel jam spilling out of the nip.
    if pose.towel > 0.02 {
        draw_towel_jam(pb, base, f, p, pose.towel, hit, pose.alpha, view.anim);
    }

    // 5) The loose crank (front-most on the right flank).
    draw_crank(pb, base, f, p, &pose, hit);

    // 6) Steam wisps off a jammed nip.
    if pose.steam > 0.02 {
        draw_steam(pb, base, f, p, pose.steam, view.anim, pose.alpha);
    }

}

#[allow(clippy::too_many_arguments)]
fn draw_iron_foot(
    pb: &mut PixelBuffer,
    base: Vec2,
    f: Vec2,
    p: Vec2,
    side: f32,
    lift: f32,
    hit: f32,
    alpha: f32,
) {
    let center = base - f * (BODY_CENTRE_BACK + BODY_HALF_LEN - 0.5) + p * (side * 8.6)
        + Vec2::new(0.0, 3.4 - lift * 1.8);
    let Some((x0, x1, y0, y1)) = bbox(pb, center, 4.5, 4.5) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_rbox(u, v, 0.0, 0.0, 2.6, 2.2, 0.8);
            if d < 0.6 {
                let lum = 0.46 + 0.30 * (v / 2.2).clamp(-1.0, 1.0) * p.dot(LIGHT);
                let col = shade3(IRON_SH, IRON_MID, IRON_HI, lum);
                blend_px(pb, x, y, apply_hit(col, hit * 0.3), (1.0 - (d.max(0.0) / 0.6)) * alpha);
            }
            // Two bright bolts.
            if (u + 0.9).abs() < 0.4 && (v.abs() - 1.0).abs() < 0.4 {
                blend_px(pb, x, y, IRON_SPEC, 0.8 * alpha);
            }
        }
    }
}

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
    let nip = base + f * NIP_OFFSET;
    let Some((x0, x1, y0, y1)) = bbox(pb, nip + f * 3.0, 10.0, 9.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - nip.x;
            let dy = y as f32 - nip.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            if !(0.0..=9.0 * amount).contains(&u) {
                continue;
            }
            let wobble = (anim * 8.0 + v * 0.7).sin() * 0.5;
            let half_v = (7.2 - (u / 9.0) * 2.2 * amount).max(0.6);
            let dv = v.abs() - half_v;
            if dv < 0.8 {
                let folds = ((u * 1.7 + v * 0.4 + anim * 1.5).sin() * 0.5 + 0.5).powi(2);
                let col = shade3(TOWEL_SH, TOWEL_MID, TOWEL_HI, 0.55 + 0.30 * folds + 0.10 * wobble);
                let a = (1.0 - (dv.max(0.0) / 0.8)) * alpha * amount;
                blend_px(pb, x, y, apply_hit(col, hit * 0.2), a);
            }
            // The cloth's hem.
            if (u - 8.4 * amount).abs() < 0.6 && v.abs() < half_v {
                blend_px(pb, x, y, TOWEL_SH, 0.7 * alpha * amount);
            }
        }
    }
}

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
    let hub = base + f * (CRANK_ALONG + 2.0) + p * CRANK_SIDE_OFFSET;
    let swing = (anim * 2.6).sin() * 1.5 * slip;
    let a = hub + Vec2::new(0.0, 1.0);
    let b = hub + p * (7.0 + swing) + Vec2::new(0.0, 7.0 * slip);
    let c = b + p * (3.0 + swing * 0.6) + Vec2::new(0.0, 5.0 * slip);
    for (from, to, w) in [(a, b, 1.6), (b, c, 1.2)] {
        let d = to - from;
        let steps = (d.len() * 2.0).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let q = from + d * t;
            let perp = d.perp().normalized() * (w * 0.5 * (1.0 - t * 0.4));
            for k in [-1.0f32, 0.0, 1.0] {
                let r = q + perp * k;
                let lum = 0.55 + 0.35 * (k.abs() - 1.0).abs();
                let col = shade3(BELT_SH, BELT_MID, BELT_HI, lum);
                blend_px(
                    pb,
                    r.x.round() as i32,
                    r.y.round() as i32,
                    apply_hit(col, hit * 0.2),
                    alpha * 0.95,
                );
            }
        }
    }
    // The frayed end.
    for k in -1..=1 {
        let q = c + p * (k as f32 * 0.9) + Vec2::new(0.0, 1.2);
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, BELT_HI, 0.55 * alpha);
    }
}

fn draw_crank(pb: &mut PixelBuffer, base: Vec2, f: Vec2, p: Vec2, pose: &Pose, hit: f32) {
    let hub = base + f * CRANK_ALONG + p * CRANK_SIDE_OFFSET;
    let lift = pose.crank_loose;
    let drop = Vec2::new(0.0, 2.4 * lift) + p * (1.6 * lift);
    let hub_c = hub + drop;

    // Hub.
    fill_disc(pb, hub_c, 2.1, shade3(BRASS_SH, BRASS_MID, BRASS_HI, 0.8), 0.98 * pose.alpha);
    // In the jam the crank swings out and down.
    let angle = if lift > 0.4 {
        // The dropped crank hangs out and down from the hub.
        FRAC_PI_2 * 0.85 + 0.25 * lift
    } else {
        pose.crank_angle
    };
    let dir = Vec2::from_angle(angle);
    let arm_end = hub_c + dir * (5.2 + 2.2 * lift);
    // Arm.
    let d = arm_end - hub_c;
    let steps = (d.len() * 2.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let q = hub_c + d * t;
        for k in [-0.7f32, 0.0, 0.7] {
            let r = q + dir.perp() * k;
            let lum = 0.6 + 0.3 * (1.0 - (k.abs() / 0.7));
            blend_px(
                pb,
                r.x.round() as i32,
                r.y.round() as i32,
                apply_hit(shade3(BRASS_SH, BRASS_MID, BRASS_HI, lum), hit * 0.3),
                0.96 * pose.alpha,
            );
        }
    }
    // Wooden handle, crosswise to the arm.
    let handle = arm_end + dir * 1.4;
    let hd = dir.perp() * 2.6;
    let steps = 5;
    for i in -steps..=steps {
        let t = i as f32 / steps as f32;
        let q = handle + hd * t;
        let col = shade3(WOOD_SH, WOOD_MID, WOOD_HI, 0.7 + 0.25 * (1.0 - t.abs()));
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, col, 0.96 * pose.alpha);
    }
    // The weak-point sheen: a hot glint on the hub whenever it is exposed.
    if pose.glow > 0.02 {
        let g = pose.glow;
        blend_px(
            pb,
            hub_c.x.round() as i32 - 1,
            hub_c.y.round() as i32 - 1,
            WARN_HOT,
            0.55 * g,
        );
        blend_px(
            pb,
            hub_c.x.round() as i32,
            hub_c.y.round() as i32 - 1,
            (255, 255, 255),
            0.35 * g,
        );
    }
    // A dropped crank's spring coil on the floor (final jam / death).
    if lift > 0.6 {
        let coil = hub + Vec2::new(0.0, 5.0) + p * 3.0;
        for k in 0..3 {
            ring(pb, coil, 1.6 + k as f32 * 1.4, SPRING_COL, 0.35 * pose.alpha, 0.7);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_steam(pb: &mut PixelBuffer, base: Vec2, f: Vec2, p: Vec2, density: f32, anim: f32, alpha: f32) {
    for i in 0..5 {
        let phase = i as f32 * 0.7;
        let t = ((anim * 0.8 + phase) % 1.0).clamp(0.0, 1.0);
        let u = 4.0 + t * 7.0;
        let side = ((i as f32 * 2.3).sin()) * 7.0 + (t * 6.0).sin() * 1.5;
        let q = base + f * u + p * side + Vec2::new(0.0, -t * 6.0);
        let r = 1.2 + t * 2.0;
        let a = density * alpha * (1.0 - t) * 0.35;
        fill_disc(pb, q, r, STEAM_COL, a);
    }
}

/// The wring-out's rolled towel (a boss projectile).
pub fn draw_towel(pb: &mut PixelBuffer, pos: Vec2, vel: Vec2, anim: f32, cam: Vec2, _accent: Rgb) {
    let c = w2s(pos, cam);
    let f = if vel.len_sq() > 4.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let p = f.perp();
    let Some((x0, x1, y0, y1)) = bbox(pb, c, TOWEL_RADIUS + 2.0, TOWEL_RADIUS + 2.0) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - c.x;
            let dy = y as f32 - c.y;
            let u = dx * f.x + dy * f.y;
            let v = dx * p.x + dy * p.y;
            let d = sd_rbox(u, v, 0.0, 0.0, TOWEL_RADIUS, TOWEL_RADIUS - 0.6, 1.6);
            if d < 0.7 {
                // A spiral roll: cream with oxblood weft bands.
                let band = ((u * 1.5 + v * 2.2 + anim * 14.0) * 0.8).floor() as i32 & 1 == 0;
                let lum = 0.65 + 0.30 * (v / TOWEL_RADIUS).clamp(-1.0, 1.0) * p.dot(LIGHT);
                let col = if band {
                    shade3(TOWEL_SH, TOWEL_MID, TOWEL_HI, lum)
                } else {
                    shade3(ROLLER_RED, ROLLER_RED_HI, (255, 168, 150), lum)
                };
                blend_px(pb, x, y, col, 1.0 - (d.max(0.0) / 0.7));
            }
        }
    }
    // Two motion streaks behind it.
    for k in [-1.0f32, 1.0] {
        let q = c - f * 3.0 + p * (k * 1.6);
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, TOWEL_HI, 0.35);
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
}
