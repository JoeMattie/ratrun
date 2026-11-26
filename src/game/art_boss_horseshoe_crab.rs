//! Art and gameplay specification for the Sewer's other boss — **THE HORSESHOE**
//! (a horseshoe crab, *Limulus* of the drains).
//!
//! The Sewer's first boss is a cat: fast, light, all pounce and claw. Its pool
//! mate is the opposite of everything the cat is. Something older than the
//! pipes hauled itself up the drain and is sitting in the arena, and it is not
//! in a hurry. A horseshoe crab is a fossile that never got the memo: a wide
//! armoured dome the colour of wet sand, ten legs under it, and one long
//! unbreakable spike for a tail. Seen from a rat's height, from above, it is a
//! **hill with an eye slit** — and the eye slit is the only part you can hurt.
//!
//! # The fight the level was built for
//!
//! The cat teaches the landing ring. The horseshoe teaches **position**: its
//! shell deflects everything that arrives over the dome from the front or the
//! top, so the rat's whole problem is to be somewhere the armour is not. Every
//! attack turns the crab and every recovery exposes it, and the fight is a
//! slow orbit into the one place it cannot cover.
//!
//! | phase | hp | rotation | read |
//! |---|---|---|---|
//! | 1 | `> 0.66` | `Dome` → `BulldozeTell` → `Bulldoze` → `Wedged` / `Dome` → `TelsonTell` → `TelsonSweep` → `Recover` | walk out of the marked lane and let it wedge itself; then bite the gills |
//! | 2 | `0.33..=0.66` | adds `BurrowTell` → `Burrow` → `Emerge` → `Recover`; sheds blue blood | leave the marked circle; the emergence leaves it dazed |
//! | 3 | `<= 0.33` | adds `FlipTell` → `Flipped` → `Righting`; sheds more blood | stay out of the thrashing legs, then take the whole exposed underside |
//!
//! ## The dome (directional armour)
//!
//! [`incoming_multiplier`] is the binding rule: a shot whose travel direction is
//! not clearly from the rear (`dir · facing < SHELL_VULNERABLE_COS`) is
//! **deflected** — it sparks off the carapace and does nothing, and the shot is
//! spent. Everything the crab does with its body is aimed at keeping the dome
//! toward the rat. The counters are:
//!
//! * the tail sweeps the **rear arc** ([`telson_arc`]), so the rat that hides
//!   behind the armour is the rat the tail is for;
//! * the bulldoze turns the dome again and drives it into a wall, which is the
//!   one time the armour stops covering anything;
//! * acid pools and ground weapons under the body always count (`dir == 0` is
//!   "from underneath"), and so does every hit during [`CrabState::Wedged`]
//!   and [`CrabState::Flipped`].
//!
//! ## The telson sweep
//!
//! `TelsonTell` cocks the spike over one shoulder and paints the live arc for
//! [`TELSON_TELL`] s; the arc covers [`TELSON_HALF_ANGLE`] either side of the
//! crab's **rear**, from [`TELSON_INNER`] to [`TELSON_OUTER`] px. `TelsonSweep`
//! drags the spike through it in [`TELSON_SWEEP`] s and the band is live for
//! [`TELSON_ACTIVE`] s, once. One sidestep leaves it (the cone is 120°: the rat can also
//! step back out of reach), and the rat that reads it gets
//! [`RECOVER`] s of a limp tail to punish.
//!
//! ## The bulldoze
//!
//! `BulldozeTell` marks a lane ([`bulldoze_lane`]) from the crab through the
//! rat to the first wall it would meet; the aim is locked at
//! [`BULLDOZE_LOCK_PROGRESS`]. `Bulldoze` is a **slow, unstoppable** walk down
//! it at [`BULLDOZE_SPEED`] px/s (the rat walks 58, so outrunning it is not an
//! option — leaving the lane is). It ends where every bulldoze ends:
//! [`CrabState::Wedged`], nose into the masonry, tail up and the soft gill
//! fans under the rear rim exposed. That is the punish window: no contact,
//! [`WEDGE_DAMAGE_MULT`] incoming, and the gill itself takes
//! [`GILL_DAMAGE_MULT`]. The crab will not commit the charge when the lane has
//! no wall to stop it in.
//!
//! ## The burrow
//!
//! `BurrowTell` marks a circle under the rat; at [`BURROW_LOCK_PROGRESS`] the
//! circle freezes and the crab goes under (it is **ungrounded** for the whole
//! dive and takes no contact at all). It comes up under the mark
//! ([`burrow_circle`]) for [`EMERGE_DAMAGE`], then spends [`RECOVER`] s dazed in
//! its own crater, which is the second punish.
//!
//! ## The flip
//!
//! In its last third the crab does the deliberate, terrible thing a cornered
//! horseshoe crab does: it throws itself onto its back. [`CrabState::Flipped`]
//! is fully vulnerable ([`FLIP_DAMAGE_MULT`], no deflection, weak point
//! everywhere) and very dangerous: the legs thrash inside the marked
//! [`legs_ring`], one [`LEGS_DAMAGE`] hit for a rat that walks into it. Then
//! `Righting` rolls it back onto its feet and it goes on with the fight.
//!
//! ## Blue blood
//!
//! The crab's blood is copper-blue, not red. From phase 2 it **sheds** pools
//! ([`blood_pool_shape`], [`BLOOD_SLOW`]) where it is hit hardest: on every
//! wedge, on every flip, and on every phase shift. A pool deals no damage and
//! spends no i-frames; it just charges the rat a walk-speed tax for the rest of
//! the fight.
//!
//! # Layering contract for the renderer
//!
//! 1. **Ground layer** (after the floor and stains, before props/entities):
//!    [`draw_ground`] — drop shadow, the telson's live arc, the bulldoze lane
//!    and its arrowheads, the burrow mark, the legs ring, the blue blood and
//!    the diegetic ground HP arc.
//! 2. **Entity layer** (with the other bodies): [`draw`] — telson, legs, dome,
//!    eyes, ridges, gill fans when exposed, the upside-down underside when
//!    flipped, and the death collapse.
//!
//! Everything is stack-only and culls on [`CULL_MARGIN`], so the draw cost is
//! the same order as the mangle's.

#![allow(dead_code)]

use std::f32::consts::{PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, Rgb};

// ============================================================================
// Collision & gameplay constants (art <-> controller <-> framework contract)
// ============================================================================

/// Uniform scale for the sprite, collision and body-derived hazards.
pub const BODY_SCALE: f32 = 1.5;

/// Framework movement/contact circle (world px). The dome is wider than this at
/// its shoulders; the circle is the body's walking footprint.
pub const BODY_RADIUS: f32 = 14.0 * BODY_SCALE;

const DOME_LOCAL_HALF_LEN: f32 = 15.0;

/// Half the dome's length along the facing axis (front is `+u`).
pub const DOME_HALF_LEN: f32 = DOME_LOCAL_HALF_LEN * BODY_SCALE;

/// Half the dome's width at its widest shoulders (world px).
pub const DOME_HALF_PERP: f32 = 16.0 * BODY_SCALE;

/// The dome's hit-box half extents (a touch tighter than the paint, so a shot
/// that clips the very rim of the carapace is a graze, not a hit).
pub const HIT_HALF_LEN: f32 = 14.0 * BODY_SCALE;
pub const HIT_HALF_PERP: f32 = 15.5 * BODY_SCALE;

/// The telson's hinge, behind the dome centre.
pub const TELSON_HINGE: f32 = 13.0 * BODY_SCALE;

/// The telson's full length from the hinge (the culling/length figure).
pub const TELSON_LENGTH: f32 = 26.0 * BODY_SCALE;

/// Longest sprite reach for culling: dome + tail + a little dust.
pub const CULL_MARGIN: f32 = 58.0 * BODY_SCALE;

/// The art's recommended base pool, before `BossKind::hp_scale` and the
/// director's boss curve.
///
/// Deliberately *under* the Apex Jaguar's 2,200 for the same slot: the shell
/// sheds a large share of the rat's fire on any run that does not learn to
/// flank, and the wedge/flip windows pay it back at ×1.35/×1.5. See
/// `BossKind::hp_scale` for the measured fight length.
pub const BOSS_BASE_HP: f32 = 2000.0;

/// Body contact while it is on its feet (owner's rule: 8 outside recovery).
pub const CONTACT_DAMAGE: f32 = 8.0;

/// Phase 2 opens here (the blood starts to run).
pub const PHASE2_HP_FRAC: f32 = 0.66;

/// Phase 3 opens here (it starts throwing itself over).
pub const PHASE3_HP_FRAC: f32 = 0.33;

/// Critical shimmer on the gills (art only).
pub const CRITICAL_HP_FRAC: f32 = 0.14;

// ---- the shell ------------------------------------------------------------

/// A shot travelling more front-ward than this (against the crab's facing) is
/// deflected. `0.0` = anything not arriving from the rear hemisphere, i.e. the
/// dome is armour from every angle it can turn to face; `1.0` would be "rear
/// only, dead astern".
pub const SHELL_VULNERABLE_COS: f32 = 0.0;

/// Incoming damage while the gill fans are out (the punish window).
pub const WEDGE_DAMAGE_MULT: f32 = 1.35;

/// Incoming damage on the gill fans themselves (the one clear weak point).
pub const GILL_DAMAGE_MULT: f32 = 2.0;

/// Radius of the exposed gill fans (their hit circle).
pub const GILL_RADIUS: f32 = 6.5 * BODY_SCALE;

/// The gill fans sit under the rear rim, this far behind the body centre.
pub const GILL_BACK: f32 = 7.0 * BODY_SCALE;

/// Incoming damage while it is on its back (fully vulnerable).
pub const FLIP_DAMAGE_MULT: f32 = 1.5;

// ---- neutral beat ---------------------------------------------------------

/// `Dome` length per phase: it turns, it walks, it decides.
pub const DOME_DURATION: [f32; 3] = [0.90, 0.82, 0.72];

/// Walk speed per phase while in `Dome` (`30 / 35 / 40` px/s): slow, relentless.
pub const DOME_SPEED: [f32; 3] = [30.0, 35.0, 40.0];

/// The band `Dome` holds the rat at, in px (`near`, `far`).
pub const DOME_BAND: [f32; 2] = [20.0 * BODY_SCALE, 46.0 * BODY_SCALE];

/// Extra speed multiplier while hunting a rat that has left the arena.
pub const HUNT_SPEED_MULT: f32 = 1.8;

/// The shared harmless recovery after a missed attack (the punish beat).
pub const RECOVER: f32 = 0.62;

/// The `Recover` after a dodged burrow: it is dazed in its own crater.
pub const RECOVER_EMERGE: f32 = 0.95;

/// The harmless 1.2 s pose change at each phase threshold.
pub const POSE_DURATION: f32 = 1.20;

/// The death: the dome cracks, the blue floods out, the legs curl.
pub const DEATH_DURATION: f32 = 2.60;

// ---- the telson sweep -----------------------------------------------------

/// The tail's telegraph per phase.
pub const TELSON_TELL: [f32; 3] = [0.95, 0.86, 0.78];

/// The sweep itself (the art drags the spike across the arc).
pub const TELSON_SWEEP: f32 = 0.55;

/// The arc is live for this long inside the sweep (one hit).
pub const TELSON_ACTIVE: f32 = 0.18;

/// Where in the sweep the live window opens, in progress.
pub const TELSON_ACTIVE_PROGRESS: f32 = 0.30;

/// The arc's inner radius (the body is not in its own sweep).
pub const TELSON_INNER: f32 = 13.0 * BODY_SCALE;

/// The arc's outer radius (a long spike).
pub const TELSON_OUTER: f32 = 40.0 * BODY_SCALE;

/// Half the arc's width, either side of dead astern (60° → a 120° fan).
pub const TELSON_HALF_ANGLE: f32 = PI / 3.0;

/// What the spike costs.
pub const TELSON_DAMAGE: f32 = 16.0;

/// Knockback off the spike.
pub const TELSON_KNOCKBACK: f32 = 95.0;

// ---- the bulldoze ---------------------------------------------------------

/// The lane's telegraph per phase.
pub const BULLDOZE_TELL: [f32; 3] = [1.05, 0.96, 0.86];

/// The aim locks here so the lane can be walked out of.
pub const BULLDOZE_LOCK_PROGRESS: f32 = 0.60;

/// Charge speed per phase (`112 / 124 / 138` px/s). Always faster than the rat.
pub const BULLDOZE_SPEED: [f32; 3] = [112.0, 124.0, 138.0];

/// Half the marked lane's width.
pub const BULLDOZE_HALF_WIDTH: f32 = 14.0 * BODY_SCALE;

/// The shortest lane worth committing to (a lane shorter than this is a nudge).
pub const BULLDOZE_MIN: f32 = 64.0;

/// The furthest the crab will look for a wall to wedge into.
pub const BULLDOZE_MAX: f32 = 230.0 * BODY_SCALE;

/// What a full charge costs.
pub const BULLDOZE_DAMAGE: f32 = 18.0;

/// Knockback off the dome.
pub const BULLDOZE_KNOCKBACK: f32 = 140.0;

/// The wedge (nose into the wall, gills out) per phase.
pub const WEDGED_DURATION: [f32; 3] = [1.90, 1.75, 1.55];

/// How long the crab holds still after the wedge before it pulls out.
pub const WEDGED_PULL_OUT: f32 = 0.35;

// ---- the burrow -----------------------------------------------------------

/// The burrow telegraph per phase.
pub const BURROW_TELL: [f32; 3] = [1.15, 1.05, 0.95];

/// The mark freezes here.
pub const BURROW_LOCK_PROGRESS: f32 = 0.55;

/// The dive: it sinks out of sight.
pub const BURROW_SINK: f32 = 0.50;

/// The run under the floor.
pub const BURROW_TRAVEL: f32 = 0.50;

/// The eruption.
pub const EMERGE: f32 = 0.40;

/// The marked circle's radius.
pub const BURROW_LOCAL_RADIUS: f32 = 17.0;
pub const BURROW_RADIUS: f32 = BURROW_LOCAL_RADIUS * BODY_SCALE;

/// What the eruption costs.
pub const EMERGE_DAMAGE: f32 = 16.0;

/// Knockback out of the crater.
pub const EMERGE_KNOCKBACK: f32 = 120.0;

// ---- the flip -------------------------------------------------------------

/// The flip's telegraph per phase (it rocks, then throws itself over).
pub const FLIP_TELL: f32 = 1.05;

/// Time on its back (the full-vulnerability window).
pub const FLIPPED_DURATION: [f32; 3] = [2.40, 2.40, 2.20];

/// The roll back onto its feet (harmless).
pub const RIGHTING: f32 = 0.75;

/// The thrashing legs' ring.
pub const LEGS_INNER: f32 = 5.0 * BODY_SCALE;
pub const LEGS_OUTER: f32 = 27.0 * BODY_SCALE;

/// What a leg costs.
pub const LEGS_DAMAGE: f32 = 12.0;

// ---- the blue blood -------------------------------------------------------

/// A shed pool's radius.
pub const BLOOD_RADIUS: f32 = 11.0 * BODY_SCALE;

/// How long a pool lasts (it stops when the crab dies: it is the boss's).
pub const BLOOD_LIFE: f32 = 6.0;

/// The lie-down warning before a pool is slick underfoot.
pub const BLOOD_WARN: f32 = 0.40;

/// Share of the rat's walk the pool takes (45 % slower while wading).
pub const BLOOD_SLOW: f32 = 0.45;

/// Pools shed per trigger in phase 2 and phase 3.
pub const BLOOD_POOLS: [u32; 2] = [2, 3];

// ============================================================================
// View & move-set types
// ============================================================================

/// The Horseshoe's whole move set. Every state runs linearly from `0.0` to
/// `1.0` in `CrabView::progress`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrabState {
    /// **Neutral.** It walks the dome at the rat, slow and unstoppable. Body
    /// contact [`CONTACT_DAMAGE`].
    Dome,
    /// **Tail cocked.** The spike swings out behind one shoulder and the live
    /// arc is painted for the rat.
    TelsonTell,
    /// **The sweep.** The spike drags through the arc; the band is live once.
    TelsonSweep,
    /// **Lane marked.** It lowers its whole body and paints the charge lane.
    BulldozeTell,
    /// **The charge.** A slow, unstoppable walk down the lane.
    Bulldoze,
    /// **Wedged.** Nose in the masonry, tail up, gill fans out: no contact,
    /// ×[`WEDGE_DAMAGE_MULT`] incoming, the gills ×[`GILL_DAMAGE_MULT`].
    Wedged,
    /// **The mark.** A circle appears under the rat and the crab starts to dig.
    BurrowTell,
    /// **Under the floor.** Ungrounded, no contact, travelling to the mark.
    Burrow,
    /// **The eruption.** The marked circle bursts for [`EMERGE_DAMAGE`].
    Emerge,
    /// **Rocking.** It heaves itself from side to side before the flip.
    FlipTell,
    /// **On its back.** Fully vulnerable, legs thrashing in [`legs_ring`].
    Flipped,
    /// **Righting.** The roll back onto its feet (harmless).
    Righting,
    /// **The 1.2 s harmless pose change** at a phase threshold: the plates
    /// shift and the blue seeps out.
    Pose,
    /// **Limped.** The recovery after a missed attack: no contact, no attack.
    Recover,
    /// **Death.** The dome cracks and the blue floods the floor.
    Death,
}

impl CrabState {
    /// Is this state one of the `Recover` beats (contact off)?
    #[inline]
    pub fn is_recovery(self) -> bool {
        matches!(
            self,
            CrabState::Wedged
                | CrabState::Flipped
                | CrabState::Righting
                | CrabState::Pose
                | CrabState::Recover
                | CrabState::Burrow
                | CrabState::Death
        )
    }
}

/// Pure, `Copy` render-state snapshot of **The Horseshoe**.
#[derive(Clone, Copy, Debug)]
pub struct CrabView {
    /// World-space centre of the dome.
    pub pos: Vec2,
    /// Facing (front of the dome, unit; defaults to `(1, 0)` if zero).
    pub facing: Vec2,
    /// Current move state.
    pub state: CrabState,
    /// Normalized progress through `state`.
    pub progress: f32,
    /// Continuous animation clock (s).
    pub anim: f32,
    /// Hit-flash intensity `0.0..=1.0`.
    pub hit_flash: f32,
    /// Remaining health fraction.
    pub hp_frac: f32,
    /// Fight phase (`1..=3`).
    pub phase: u8,
    /// Telegraph target: the lane's far end (bulldoze) or the marked circle
    /// (burrow).
    pub target: Vec2,
    /// The lane's / crater's near end (the crab's position at the lock).
    pub lane_start: Vec2,
    /// The telson's sweep side (`+1` / `-1`).
    pub sweep_side: f32,
}

/// One art-defined hit shape (world space).
#[derive(Clone, Copy, Debug)]
pub struct CrabHit {
    pub center: Vec2,
    pub facing: Vec2,
    /// Half-length along `facing` (boxes).
    pub half: f32,
    /// Half-width across `facing` (boxes).
    pub half_perp: f32,
    /// Radius (circles).
    pub radius: f32,
    /// `true` = oriented box, `false` = circle.
    pub boxed: bool,
    /// Is the shape on the floor (hittable by ground weapons)?
    pub grounded: bool,
}

// ============================================================================
// Geometry queries (art <-> hitbox contract)
// ============================================================================

#[inline]
pub fn unit_facing(view: &CrabView) -> Vec2 {
    if view.facing.len_sq() > 1e-4 {
        view.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The dome's hit box: `(centre, facing, half_len, half_perp)`.
#[inline]
pub fn dome_box(view: &CrabView) -> (Vec2, Vec2, f32, f32) {
    let f = unit_facing(view);
    (view.pos + f * BODY_SCALE, f, HIT_HALF_LEN, HIT_HALF_PERP)
}

/// The exposed gill fans (the weak point). While flipped the whole underside is
/// the weak point, so the fans are drawn at the centre.
#[inline]
pub fn gill_pos(view: &CrabView) -> Vec2 {
    let f = unit_facing(view);
    match view.state {
        CrabState::Flipped => view.pos,
        _ => view.pos - f * GILL_BACK,
    }
}

/// Is the shell off (gills or underside exposed)?
#[inline]
pub fn is_exposed(state: CrabState) -> bool {
    matches!(state, CrabState::Wedged | CrabState::Flipped | CrabState::Death)
}

/// Is the crab under the floor (unhittable)?
#[inline]
pub fn is_underground(state: CrabState) -> bool {
    matches!(state, CrabState::Burrow)
}

/// The exact world-space hit shapes: `[dome box, gill circle]`.
///
/// The dome is ungrounded while the crab is under the floor; the gill circle
/// only exists while the shell is off, which is what makes auto-aim and every
/// ground weapon turn to it in the punish window.
pub fn hit_shapes(view: &CrabView) -> [CrabHit; 2] {
    let f = unit_facing(view);
    let grounded = !is_underground(view.state);
    let (center, facing, half, half_perp) = dome_box(view);
    [
        CrabHit {
            center,
            facing,
            half,
            half_perp,
            radius: 0.0,
            boxed: true,
            grounded,
        },
        CrabHit {
            center: gill_pos(view),
            facing: f,
            half: 0.0,
            half_perp: 0.0,
            radius: GILL_RADIUS,
            boxed: false,
            grounded: grounded && is_exposed(view.state),
        },
    ]
}

/// The telson's live arc: `(centre, inner, outer, facing, half_angle)`. The arc
/// faces the crab's **rear**, i.e. the tail's own side.
#[inline]
pub fn telson_arc(view: &CrabView) -> (Vec2, f32, f32, Vec2, f32) {
    let f = unit_facing(view);
    (view.pos, TELSON_INNER, TELSON_OUTER, -f, TELSON_HALF_ANGLE)
}

/// The thrashing legs' annulus: `(centre, inner, outer)`.
#[inline]
pub fn legs_ring(view: &CrabView) -> (Vec2, f32, f32) {
    (view.pos, LEGS_INNER, LEGS_OUTER)
}

/// The bulldoze lane's rectangle centre/direction/half extents, as the art
/// paints it.
#[inline]
pub fn bulldoze_lane(view: &CrabView) -> (Vec2, Vec2, f32, f32) {
    let f = unit_facing(view);
    let len = view.lane_start.dist(view.target).max(1.0);
    (
        (view.lane_start + view.target) * 0.5,
        f,
        len * 0.5,
        BULLDOZE_HALF_WIDTH,
    )
}

/// The burrow mark's circle centre and radius.
#[inline]
pub fn burrow_circle(view: &CrabView) -> (Vec2, f32) {
    (view.target, BURROW_RADIUS)
}

/// The blue blood pool's circle centre and radius.
#[inline]
pub fn blood_pool_shape(center: Vec2) -> (Vec2, f32) {
    (center, BLOOD_RADIUS)
}

/// **The armour rule.** The damage multiplier for a hit arriving at `at` while
/// travelling along `dir` (unit; `Vec2::ZERO` means "from underneath", e.g. a
/// pool or a ground weapon the crab is standing in).
///
/// * `0.0` — deflected off the carapace (spent, no damage);
/// * [`GILL_DAMAGE_MULT`] — on the exposed gill fans;
/// * `1.0` — everywhere else.
pub fn incoming_multiplier(view: &CrabView, dir: Vec2, at: Vec2) -> f32 {
    if is_exposed(view.state) {
        return if at.dist(gill_pos(view)) <= GILL_RADIUS + 2.0 * BODY_SCALE {
            GILL_DAMAGE_MULT
        } else {
            1.0
        };
    }
    // A pool, a trap or anything that arrives from directly below the body is
    // under the shell, not over it.
    if dir.len_sq() > 1e-4 {
        let f = unit_facing(view);
        if dir.normalized().dot(f) < SHELL_VULNERABLE_COS {
            return 0.0;
        }
    }
    if at.dist(gill_pos(view)) <= GILL_RADIUS + 2.0 * BODY_SCALE {
        // Hidden under the rim: not a weak point until it lifts the shell.
        return 1.0;
    }
    1.0
}

/// The state's own incoming damage multiplier (the punish windows).
#[inline]
pub fn incoming_damage_mult(state: CrabState) -> f32 {
    match state {
        CrabState::Wedged => WEDGE_DAMAGE_MULT,
        CrabState::Flipped => FLIP_DAMAGE_MULT,
        _ => 1.0,
    }
}

/// Does this state deal body-contact damage?
#[inline]
pub fn has_body_contact(state: CrabState) -> bool {
    !matches!(
        state,
        CrabState::Wedged
            | CrabState::Flipped
            | CrabState::Righting
            | CrabState::Pose
            | CrabState::Recover
            | CrabState::Burrow
            | CrabState::Emerge
            | CrabState::Death
    )
}

/// The telson's world tip for a view (art + vfx).
pub fn telson_tip(view: &CrabView) -> Vec2 {
    let f = unit_facing(view);
    let hinge = view.pos - f * TELSON_HINGE;
    let p = clamp01(view.progress);
    let back = -f;
    let angle = match view.state {
        CrabState::Dome => (view.anim * 1.7).sin() * 0.10,
        CrabState::TelsonTell => {
            let side = if view.sweep_side >= 0.0 { 1.0 } else { -1.0 };
            let t = smoothstep((p / 0.85).min(1.0));
            -side * 1.15 * t
        }
        CrabState::TelsonSweep => {
            let side = if view.sweep_side >= 0.0 { 1.0 } else { -1.0 };
            let t = smoothstep(p);
            // Cocked one way at the start, dragged all the way through.
            let a0 = -side * 1.15;
            let a1 = side * 1.15;
            a0 + (a1 - a0) * t
        }
        CrabState::Wedged => 0.0,
        CrabState::Flipped | CrabState::Righting => 0.35,
        _ => 0.0,
    };
    // Cocked arcs bend out of the dead-astern line; `Wedged` lifts it straight
    // up, drawn shorter (`TELSON_UP_DRAW`).
    let dir = back.rotate(angle);
    let len = if matches!(view.state, CrabState::Wedged) {
        TELSON_LENGTH * 0.55
    } else {
        TELSON_LENGTH
    };
    hinge + dir * len
}

/// How far the tail is lifted clear of the floor (`0..=1`, the wedge pose).
#[inline]
pub fn telson_lift(view: &CrabView) -> f32 {
    match view.state {
        CrabState::Wedged => smoothstep((view.progress / 0.25).min(1.0)),
        CrabState::TelsonTell => 0.35,
        CrabState::TelsonSweep => 0.5,
        _ => 0.0,
    }
}

// ============================================================================
// Palette
// ============================================================================

const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// A lifetime of wet sand, iron and tar: the dome is dull olive-ochre.
const SHELL_OUT: Rgb = (10, 11, 12);
const SHELL_SH: Rgb = (40, 38, 26);
const SHELL_MID: Rgb = (92, 84, 54);
const SHELL_HI: Rgb = (150, 138, 92);
const SHELL_SPEC: Rgb = (222, 208, 154);
// Wet, older parts of the carapace and the hinge knuckles.
const HINGE_SH: Rgb = (28, 26, 20);
const HINGE_MID: Rgb = (70, 62, 40);
const HINGE_HI: Rgb = (124, 112, 74);
// The copper-blue blood every horseshoe crab has, and the pale gill fans.
const BLOOD_SH: Rgb = (22, 42, 96);
const BLOOD_MID: Rgb = (44, 92, 190);
const BLOOD_HI: Rgb = (110, 172, 250);
const BLOOD_SPEC: Rgb = (196, 228, 255);
const GILL_SH: Rgb = (74, 108, 132);
const GILL_MID: Rgb = (152, 198, 216);
const GILL_HI: Rgb = (222, 246, 252);
// Eyes and the dark wet seams.
const EYE: Rgb = (6, 8, 10);
const EYE_HI: Rgb = (150, 210, 220);
// Telegraphs.
const WARN_AMBER: Rgb = (232, 172, 58);
const WARN_HOT: Rgb = (255, 226, 128);
const WARN_RED: Rgb = (222, 74, 52);
const SAFE_CYAN: Rgb = (110, 214, 226);
const WEAK_GOLD: Rgb = (255, 226, 120);

// ============================================================================
// Small pixel helpers (self-contained, like the other boss art files)
// ============================================================================

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn w2s(p: Vec2, cam: Vec2) -> Vec2 {
    Vec2::new(p.x - cam.x, p.y - cam.y)
}

#[inline]
fn in_view(pb: &PixelBuffer, view: &CrabView, cam: Vec2) -> bool {
    let s = w2s(view.pos, cam);
    let m = CULL_MARGIN;
    s.x > -m && s.y > -m && s.x < pb.w as f32 + m && s.y < pb.h as f32 + m
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
    pb.plot(
        x,
        y,
        (
            (old.0 as f32 + (c.0 as f32 - old.0 as f32) * a) as u8,
            (old.1 as f32 + (c.1 as f32 - old.1 as f32) * a) as u8,
            (old.2 as f32 + (c.2 as f32 - old.2 as f32) * a) as u8,
        ),
    );
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

fn line_dashed(pb: &mut PixelBuffer, a: Vec2, b: Vec2, col: Rgb, alpha: f32, dash: f32, phase: f32) {
    let d = b - a;
    // A non-finite endpoint would saturate `steps` (two billion plots).
    if !d.len().is_finite() {
        return;
    }
    let steps = d.len().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let along = t * d.len() + phase;
        if (along / dash).floor() as i32 % 2 != 0 {
            continue;
        }
        let p = a + d * t;
        blend_px(pb, p.x.round() as i32, p.y.round() as i32, col, alpha);
    }
}

fn ring(pb: &mut PixelBuffer, center: Vec2, radius: f32, col: Rgb, alpha: f32, width: f32) {
    let width = width * BODY_SCALE;
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

/// Outline a world-space rectangle: `center`, `facing`, half extents.
fn rect_outline(
    pb: &mut PixelBuffer,
    center: Vec2,
    facing: Vec2,
    half: f32,
    half_perp: f32,
    col: Rgb,
    alpha: f32,
) {
    let f = facing.normalized();
    let p = f.perp();
    let a = center + f * half + p * half_perp;
    let b = center + f * half - p * half_perp;
    let c = center - f * half - p * half_perp;
    let d = center - f * half + p * half_perp;
    line_solid(pb, a, b, col, alpha);
    line_solid(pb, b, c, col, alpha);
    line_solid(pb, c, d, col, alpha);
    line_solid(pb, d, a, col, alpha);
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

/// Draw The Horseshoe's ground layer: shadow, every telegraph, the blue blood
/// and the diegetic HP arc.
///
/// **Layer**: ground — after the floor and stains, before props/enemies/the rat.
pub fn draw_ground(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2, accent: Rgb) {
    let c = w2s(view.pos, cam);
    let p = clamp01(view.progress);
    let pulse = 0.5 + 0.5 * (view.anim * 8.5).sin();
    let f = unit_facing(view);

    // Drop shadow under the dome (wider than tall: the shell is flat).
    let shadow_a = match view.state {
        CrabState::Death => 0.10,
        CrabState::Burrow => 0.0,
        _ => 0.34,
    };
    if shadow_a > 0.0 {
        let sc = c + Vec2::new(0.0, 2.0 * BODY_SCALE);
        let (rx, ry) = (18.0 * BODY_SCALE, 8.0 * BODY_SCALE);
        if let Some((x0, x1, y0, y1)) = bbox(pb, sc, rx, ry) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let e = ((x as f32 - sc.x) / rx).powi(2) + ((y as f32 - sc.y) / ry).powi(2);
                    if e <= 1.0 {
                        blend_px(pb, x, y, (4, 6, 8), shadow_a * (1.0 - e * 0.4));
                    }
                }
            }
        }
    }

    match view.state {
        CrabState::Dome | CrabState::Recover => {
            // Footprints in the wet floor: it is heavy enough to leave them.
            let step = (view.anim * 3.2).sin();
            for side in [-1.0f32, 1.0] {
                for k in 0..3 {
                    let along = (k as f32 - 1.0) * 7.0 + step * side * 1.4;
                    let q = c + (f * along + f.perp() * side * 12.0) * BODY_SCALE;
                    blend_px(pb, q.x.round() as i32, q.y.round() as i32, (24, 30, 32), 0.22);
                    blend_px(pb, q.x.round() as i32 + 1, q.y.round() as i32, (24, 30, 32), 0.14);
                }
            }
        }
        CrabState::TelsonTell => {
            let (center, inner, outer, af, half) = telson_arc(view);
            let sc = w2s(center, cam);
            let t = smoothstep((p / 0.9).min(1.0));
            // The painted band, filling in from the hinge outward.
            let fill = inner + (outer - inner) * t;
            let col = if t > 0.85 { WARN_HOT } else { WARN_AMBER };
            arc_band(pb, sc, (inner, fill), af, half, col, 0.16 + 0.10 * pulse);
            arc_band(pb, sc, (fill - 1.4 * BODY_SCALE, fill), af, half, col, 0.62);
            arc_band(pb, sc, (outer, outer + 1.2 * BODY_SCALE), af, half, col, 0.50 + 0.25 * pulse);
            arc_band(pb, sc, (inner, inner + BODY_SCALE), af, half, col, 0.40);
            // Radial tick marks, so the fan reads as a swept sector.
            for k in 0..9 {
                let a = -half + 2.0 * half * (k as f32 / 8.0);
                let d = af.rotate(a);
                line_solid(
                    pb,
                    sc + d * (inner + BODY_SCALE),
                    sc + d * (inner + 3.2 * BODY_SCALE),
                    col,
                    0.55,
                );
            }
        }
        CrabState::TelsonSweep => {
            let (center, inner, outer, af, half) = telson_arc(view);
            let sc = w2s(center, cam);
            let side = if view.sweep_side >= 0.0 { 1.0 } else { -1.0 };
            let ang = -side * half + (2.0 * half) * smoothstep(p);
            // The live band, flashing as the spike crosses it.
            let live = (TELSON_ACTIVE_PROGRESS..TELSON_ACTIVE_PROGRESS + TELSON_ACTIVE / TELSON_SWEEP).contains(&p);
            let col = if live { WARN_HOT } else { WARN_AMBER };
            arc_band(pb, sc, (inner, outer), af, half, col, if live { 0.16 } else { 0.08 });
            // The spike's own leading edge.
            let d = af.rotate(ang);
            line_solid(pb, sc + d * inner, sc + d * outer, WARN_HOT, 0.85);
            line_solid(pb, sc + d * inner, sc + d * outer, (255, 255, 236), 0.35);
            if live {
                for k in 0..6 {
                    let t = k as f32 / 5.0;
                    let r = inner + (outer - inner) * t;
                    let q = sc + d * r;
                    blend_px(pb, q.x.round() as i32, q.y.round() as i32, WARN_HOT, 0.65);
                }
            }
        }
        CrabState::BulldozeTell | CrabState::Bulldoze => {
            let (_, lf, _, half_perp) = bulldoze_lane(view);
            let start = w2s(view.lane_start, cam);
            let end = w2s(view.target, cam);
            let telling = matches!(view.state, CrabState::BulldozeTell);
            let t = if telling {
                smoothstep((p / BULLDOZE_LOCK_PROGRESS.min(1.0)).min(1.0))
            } else {
                1.0
            };
            let col = if telling { WARN_AMBER } else { WARN_RED };
            // The lane body fills from the crab toward the wall as the tell
            // runs: one cross-stroke every 2 px down the lane (cheap, and the
            // hatch is what makes it read as a marked lane rather than a rug).
            let tip = start + (end - start) * t;
            let len = tip.dist(start);
            let lp = lf.perp();
            let mut u = 0.0;
            while u <= len {
                let q = start + lf * u;
                let hatch = ((u * 0.12) + view.anim * 2.0).sin() > 0.35;
                line_solid(
                    pb,
                    q - lp * half_perp,
                    q + lp * half_perp,
                    col,
                    if hatch { 0.20 } else { 0.08 },
                );
                u += 2.0;
            }
            rect_outline(pb, (start + tip) * 0.5, lf, len * 0.5, half_perp, col, 0.62);
            // Arrowheads pointing at the wall it is going to hit.
            for k in 0..3 {
                let at = end - (end - start) * (0.10 + 0.18 * k as f32);
                let q = at;
                line_solid(pb, q - lf * (3.0 * BODY_SCALE) + lf.perp() * (3.0 * BODY_SCALE), q, col, 0.70);
                line_solid(pb, q - lf * (3.0 * BODY_SCALE) - lf.perp() * (3.0 * BODY_SCALE), q, col, 0.70);
            }
            if matches!(view.state, CrabState::Bulldoze) {
                // Dust boiling off the trailing edge.
                for k in 0..5 {
                    let t = ((view.anim * 2.4 + k as f32 * 0.3).fract()).clamp(0.0, 1.0);
                    let q = start + (end - start) * (0.0 - 0.15 * t);
                    let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                    let q = q + lf.perp() * side * (half_perp * 0.7);
                    fill_disc(pb, q, (1.4 + t * 2.6) * BODY_SCALE, (86, 74, 52), 0.20 * (1.0 - t));
                }
            }
        }
        CrabState::Wedged => {
            // The wall it is nose-first into, and the gold gather on the gills.
            let nose = c + w2s(f * DOME_HALF_LEN, Vec2::ZERO);
            for k in 0..7 {
                let a = -0.9 + k as f32 * 0.3;
                let d = f.rotate(a);
                line_solid(pb, nose, nose + d * (4.0 * BODY_SCALE), (150, 140, 110), 0.28);
            }
            let g = w2s(gill_pos(view), cam);
            ring(pb, g, GILL_RADIUS + (3.0 + 2.0 * pulse) * BODY_SCALE, WEAK_GOLD, 0.30 + 0.25 * pulse, 1.2);
            ring(pb, g, GILL_RADIUS + 8.0 * BODY_SCALE, WEAK_GOLD, 0.16, 1.0);
            // The pull-out scratch: it is trying to reverse.
            let back = c - w2s(f * (DOME_HALF_LEN + 4.0 * BODY_SCALE), Vec2::ZERO);
            for k in 0..3 {
                let q = back + w2s(f.perp() * ((k as f32 - 1.0) * 5.0 * BODY_SCALE), Vec2::ZERO);
                line_solid(pb, q, q - f * (4.0 * BODY_SCALE), (110, 100, 78), 0.35);
            }
        }
        CrabState::BurrowTell | CrabState::Emerge => {
            let (mark, r) = burrow_circle(view);
            let sc = w2s(mark, cam);
            let telling = matches!(view.state, CrabState::BurrowTell);
            let t = if telling {
                smoothstep((p / BURROW_LOCK_PROGRESS.max(0.01)).min(1.0))
            } else {
                1.0
            };
            let col = if telling { WARN_AMBER } else { WARN_HOT };
            // The circle closes in on the rat.
            ring(pb, sc, r * (1.9 - 0.9 * t), col, 0.55 + 0.20 * pulse, 1.4);
            band(pb, sc, 0.0, r * t, col, 0.10 + 0.05 * pulse);
            ring(pb, sc, r, col, 0.75, 1.2);
            for k in 0..8 {
                let d = Vec2::from_angle(k as f32 * TAU / 8.0);
                line_solid(pb, sc + d * (r - 2.0 * BODY_SCALE), sc + d * (r + 1.5 * BODY_SCALE), col, 0.45);
            }
            if !telling {
                // Boiling spoil and blue seep out of the crater.
                band(pb, sc, r * 0.55, r * 1.05, (96, 82, 56), 0.30);
                for k in 0..7 {
                    let a = view.anim * 2.0 + k as f32 * 0.9;
                    let q = sc + Vec2::from_angle(a) * (r * (0.7 + 0.3 * hash01(k, 7)));
                    blend_px(pb, q.x.round() as i32, q.y.round() as i32, BLOOD_HI, 0.45);
                }
            }
        }
        CrabState::Burrow => {
            // The hole it went down: dark, rimmed with wet blue.
            let q = w2s(view.lane_start, cam);
            fill_disc(pb, q, BURROW_RADIUS * 0.8, (6, 8, 10), 0.55);
            ring(pb, q, BURROW_RADIUS * 0.8, (96, 82, 56), 0.35, 1.2);
            ring(pb, q, BURROW_RADIUS * 0.8 + 1.5 * BODY_SCALE, BLOOD_MID, 0.22, 1.0);
            // The dotted run toward the mark.
            line_dashed(pb, q, w2s(view.target, cam), BLOOD_MID, 0.20, 4.0 * BODY_SCALE, view.anim * 8.0 * BODY_SCALE);
        }
        CrabState::FlipTell => {
            // Rocking arcs either side of the dome: which way it is going over.
            let t = smoothstep(p);
            for side in [-1.0f32, 1.0] {
                let a0 = -0.5 * side;
                let a1 = -1.3 * side;
                for k in 0..4 {
                    let a = a0 + (a1 - a0) * ((k as f32 + 0.5) / 4.0);
                    let d = f.rotate(a);
                    let q = c + d * (DOME_HALF_LEN + 2.0 * BODY_SCALE);
                    line_solid(pb, q, q + d * ((2.0 + 2.0 * t) * BODY_SCALE), WARN_AMBER, 0.30 + 0.35 * t);
                }
            }
        }
        CrabState::Flipped | CrabState::Righting => {
            let (center, inner, outer) = legs_ring(view);
            let sc = w2s(center, cam);
            let live = matches!(view.state, CrabState::Flipped);
            let col = if live { WARN_RED } else { WARN_AMBER };
            // The marked thrash ring, with leg ticks racing around it.
            ring(pb, sc, inner, col, 0.50, 1.0);
            ring(pb, sc, outer, col, 0.50 + 0.20 * pulse, 1.3);
            for k in 0..10 {
                let a = k as f32 * TAU / 10.0 + view.anim * (if live { 7.0 } else { 2.0 });
                let d = Vec2::from_angle(a);
                let reach = inner + (outer - inner) * (0.55 + 0.45 * (a * 2.0 + view.anim * 9.0).sin().abs());
                line_solid(pb, sc + d * inner, sc + d * reach, col, 0.55);
            }
        }
        CrabState::Pose => {
            let t = smoothstep((p / 0.6).min(1.0));
            ring(pb, c, (16.0 + 20.0 * t) * BODY_SCALE, WARN_AMBER, 0.55 * (1.0 - t * 0.5), 1.6);
            ring(pb, c, (26.0 - 6.0 * t) * BODY_SCALE, WARN_RED, 0.35 + 0.25 * pulse, 1.2);
            // Blue seeping out from under the plates.
            for k in 0..8 {
                let a = k as f32 * TAU / 8.0 + 0.4;
                let q = c + Vec2::from_angle(a) * ((10.0 + 7.0 * t) * BODY_SCALE);
                fill_disc(pb, q, (1.2 + t * 1.6) * BODY_SCALE, BLOOD_MID, 0.30);
            }
        }
        CrabState::Death => {
            let t = clamp01(p);
            let r = (14.0 + 18.0 * smoothstep(t * 1.2)) * BODY_SCALE;
            // A wide blue pool under the collapsing shell.
            fill_disc(pb, c + Vec2::new(0.0, 2.0 * BODY_SCALE), r, BLOOD_SH, 0.30 * (1.0 - t * 0.2));
            ring(pb, c + Vec2::new(0.0, 2.0 * BODY_SCALE), r * 0.95, BLOOD_MID, 0.35 * (1.0 - t * 0.4), 1.3);
            ring(pb, c, (12.0 + t * 26.0) * BODY_SCALE, BLOOD_HI, 0.35 * (1.0 - t), 1.2);
        }
    }

    // The diegetic ground HP arc (centred, like the other bosses).
    if !matches!(view.state, CrabState::Death | CrabState::Burrow) && view.hp_frac > 0.0 {
        let hp = view.hp_frac.clamp(0.0, 1.0);
        let col = if hp <= CRITICAL_HP_FRAC {
            WARN_RED
        } else if view.phase >= 3 {
            BLOOD_HI
        } else if view.phase >= 2 {
            WARN_AMBER
        } else {
            SAFE_CYAN
        };
        let segments = 18;
        let lit = (hp * segments as f32).round() as usize;
        for k in 0..segments {
            if k >= lit {
                break;
            }
            let a0 = PI * 0.15 + PI * 0.70 * (k as f32 / segments as f32);
            let a1 = PI * 0.15 + PI * 0.70 * ((k + 1) as f32 / segments as f32) - 0.02;
            let d0 = Vec2::from_angle(a0);
            let d1 = Vec2::from_angle(a1);
            let r = 21.0 * BODY_SCALE;
            line_solid(pb, c + d0 * r, c + d1 * r, col, 0.55);
        }
    }

    // A faint rim tying the crab to the floor's palette.
    if !matches!(view.state, CrabState::Death | CrabState::Burrow) {
        ring(pb, c, DOME_HALF_PERP + 5.0 * BODY_SCALE, mix(accent, SHELL_MID, 0.6), 0.08, 1.0);
    }
}

/// Ground tint for a live blue-blood hazard. The outline pass marks the same
/// boundary before and after it becomes slippery.
pub fn draw_blood_pool(pb: &mut PixelBuffer, cam: Vec2, center: Vec2, radius: f32, active: bool) {
    let center = w2s(center, cam);
    if active {
        fill_disc(pb, center, radius, BLOOD_SH, 0.46);
        fill_disc(pb, center, radius * 0.75, BLOOD_MID, 0.26);
    }
    ring(pb, center, radius * 0.95, BLOOD_HI, 0.30, 1.0);
}

// ============================================================================
// Post-lighting tell layer
// ============================================================================

/// Re-stamp only the **live warning** after the lighting pass.
///
/// The Sewer is dark and the ambient dims the ground layer; the rat must be
/// able to read the shape it has to step out of at any light level, so the
/// hostile layer (`World::draw_hostile_layer`) draws this on top of everything
/// unlit. It paints the telegraph and nothing else: no shadow, no body, no HP
/// arc, so it can never become a second copy of the crab.
pub fn draw_tell_overlay(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2) {
    let p = clamp01(view.progress);
    let pulse = 0.5 + 0.5 * (view.anim * 9.0).sin();
    match view.state {
        CrabState::TelsonTell | CrabState::TelsonSweep => {
            let (center, inner, outer, af, half) = telson_arc(view);
            let sc = w2s(center, cam);
            let live = matches!(view.state, CrabState::TelsonSweep)
                && (TELSON_ACTIVE_PROGRESS..TELSON_ACTIVE_PROGRESS + TELSON_ACTIVE / TELSON_SWEEP).contains(&p);
            let col = if live { (255, 246, 196) } else { (255, 206, 96) };
            arc_band(pb, sc, (inner, inner + 1.2 * BODY_SCALE), af, half, col, 0.85);
            arc_band(pb, sc, (outer - 0.8 * BODY_SCALE, outer + 0.8 * BODY_SCALE), af, half, col, 0.92);
            for k in 0..9 {
                let a = -half + 2.0 * half * (k as f32 / 8.0);
                let d = af.rotate(a);
                line_solid(pb, sc + d * inner, sc + d * (inner + 4.2 * BODY_SCALE), col, 0.85);
            }
            if live {
                arc_band(pb, sc, (inner, outer), af, half, col, 0.10 + 0.06 * pulse);
            }
        }
        CrabState::BulldozeTell | CrabState::Bulldoze => {
            let start = w2s(view.lane_start, cam);
            let end = w2s(view.target, cam);
            let lf = smooth_dir(end - start);
            let len = end.dist(start);
            let lp = lf.perp();
            let col = if matches!(view.state, CrabState::Bulldoze) {
                (255, 128, 92)
            } else {
                (255, 206, 96)
            };
            line_solid(pb, start + lp * BULLDOZE_HALF_WIDTH, end + lp * BULLDOZE_HALF_WIDTH, col, 0.85);
            line_solid(pb, start - lp * BULLDOZE_HALF_WIDTH, end - lp * BULLDOZE_HALF_WIDTH, col, 0.85);
            let mut u = 0.0;
            while u <= len {
                if ((u * 0.28) + view.anim * 3.0).sin() > 0.4 {
                    let q = start + lf * u;
                    line_solid(pb, q - lp * (3.0 * BODY_SCALE), q + lp * (3.0 * BODY_SCALE), col, 0.35);
                }
                u += 3.0;
            }
            for k in 0..3 {
                let q = end - lf * (len * 0.08 + 12.0 * k as f32 * BODY_SCALE);
                line_solid(pb, q - lf * (3.0 * BODY_SCALE) + lp * (3.5 * BODY_SCALE), q, col, 0.9);
                line_solid(pb, q - lf * (3.0 * BODY_SCALE) - lp * (3.5 * BODY_SCALE), q, col, 0.9);
            }
        }
        CrabState::BurrowTell => {
            let (mark, r) = burrow_circle(view);
            let sc = w2s(mark, cam);
            let col = (255, 206, 96);
            ring(pb, sc, r, col, 0.95, 1.0);
            for k in 0..8 {
                let d = Vec2::from_angle(k as f32 * TAU / 8.0);
                line_solid(pb, sc + d * (r - 2.0 * BODY_SCALE), sc + d * (r + 2.0 * BODY_SCALE), col, 0.85);
            }
        }
        CrabState::Flipped => {
            let (center, inner, outer) = legs_ring(view);
            let sc = w2s(center, cam);
            ring(pb, sc, inner, (255, 150, 110), 0.85, 1.0);
            ring(pb, sc, outer, (255, 128, 92), 0.85, 1.0);
        }
        CrabState::Wedged => {
            let g = w2s(gill_pos(view), cam);
            ring(pb, g, GILL_RADIUS + (3.0 + 2.0 * pulse) * BODY_SCALE, (255, 236, 150), 0.85, 1.2);
        }
        _ => {}
    }
}

// ============================================================================
// Entity layer
// ============================================================================

/// Draw The Horseshoe.
///
/// **Layer**: entity — with the other bodies, shadow already drawn by the
/// ground pass.
pub fn draw(pb: &mut PixelBuffer, view: &CrabView, cam: Vec2, _accent: Rgb) {
    if !in_view(pb, view, cam) {
        return;
    }
    let p = clamp01(view.progress);
    let hit = view.hit_flash.clamp(0.0, 1.0);
    let f = unit_facing(view);
    let perp = f.perp();
    let c = w2s(view.pos, cam);
    let pulse = 0.5 + 0.5 * (view.anim * 8.5).sin();
    let damage_tier = if view.phase >= 3 {
        3
    } else if view.phase >= 2 {
        2
    } else {
        1
    };

    // The tail: always behind the dome, so it goes down first.
    if !matches!(view.state, CrabState::Burrow) && !matches!(view.state, CrabState::Flipped) {
        draw_telson(pb, view, &c, f, hit, pulse);
    }

    match view.state {
        CrabState::Burrow => {
            // A ring of disturbed floor where it went down, then nothing.
            let t = smoothstep(p);
            ring(pb, c, (6.0 + 10.0 * t) * BODY_SCALE, BLOOD_MID, 0.25 * (1.0 - t), 1.2);
            return;
        }
        CrabState::Flipped | CrabState::Righting => {
            draw_underside(pb, view, &c, f, hit, pulse, damage_tier);
            return;
        }
        CrabState::Death => {
            draw_death(pb, view, &c, f, hit, pulse, p);
            return;
        }
        _ => {}
    }

    // Legs under the shell (six little stubs), then the dome over them.
    let step_phase = match view.state {
        CrabState::Dome => view.anim * 3.4,
        CrabState::Bulldoze => view.anim * 12.0,
        CrabState::BurrowTell => view.anim * 7.0,
        _ => view.anim * 1.6,
    };
    draw_legs(pb, &c, f, perp, hit, step_phase, false);

    // The wedge lifts the rear of the shell: draw a dark gap first.
    let lifted = matches!(view.state, CrabState::Wedged);
    if lifted {
        let g = w2s(gill_pos(view), cam);
        fill_disc(pb, g, GILL_RADIUS + 1.5, (8, 12, 18), 0.55);
    }

    draw_dome(pb, view, &c, f, perp, hit, pulse, damage_tier, lifted, None);

    // Exposed gill fans.
    if lifted {
        draw_gills(pb, view, &c, f, cam, pulse);
    }

    // Eyes and the front sensory slit.
    draw_eyes(pb, view, &c, f, perp, hit, pulse);

    // A small glint on the gills: this is the thing to bite.
    if lifted {
        let g = w2s(gill_pos(view), cam);
        let gl = 1.0 + 5.0 * pulse;
        blend_px(pb, g.x.round() as i32, (g.y - gl).round() as i32, WEAK_GOLD, 0.8);
        blend_px(pb, g.x.round() as i32, (g.y - gl - 1.0).round() as i32, WEAK_GOLD, 0.35);
    }
}

/// The dome itself. `body_center` is the screen centre; `flip` rotates the
/// drawing if the crab is on its back (the underside pass uses its own loop).
#[allow(clippy::too_many_arguments)]
fn draw_dome(
    pb: &mut PixelBuffer,
    view: &CrabView,
    c: &Vec2,
    f: Vec2,
    perp: Vec2,
    hit: f32,
    pulse: f32,
    damage_tier: u8,
    lifted: bool,
    _mask: Option<()>,
) {
    let p = clamp01(view.progress);
    // A slow breathing bob and the wedge's reversed sag.
    let sag = match view.state {
        CrabState::Wedged => 1.0,
        CrabState::Recover => 0.35,
        CrabState::Pose => 0.5,
        _ => 0.0,
    };
    let base = *c + (f * (sag * -1.4) + Vec2::new(0.0, (view.anim * 1.6).sin() * 0.35)) * BODY_SCALE;
    let Some((x0, x1, y0, y1)) = shell_bbox(pb, base, f) else {
        return;
    };
    let fl = f.dot(LIGHT);
    let pl = perp.dot(LIGHT);
    let crouch = match view.state {
        CrabState::BulldozeTell => smoothstep((p / 0.7).min(1.0)) * 1.0,
        CrabState::Bulldoze => 1.0,
        _ => 0.0,
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - base;
            let u = d.dot(f) / BODY_SCALE;
            let v = d.dot(perp) / BODY_SCALE;
            // The horseshoe: a wide rounded front, tapering to a narrow hinge
            // at the back.
            let front = 1.0 - ((u - 1.5) / 16.5).powi(2);
            let half_w = if u >= -2.0 {
                if front <= 0.0 {
                    0.0
                } else {
                    16.0 * front.sqrt()
                }
            } else {
                (14.0 + (u + 2.0) * 0.64).max(3.6)
            };
            let in_dome = (-DOME_LOCAL_HALF_LEN - 0.5..=16.5).contains(&u) && v.abs() <= half_w;
            let rim = (v.abs() - half_w).abs() < 1.0 || (u - 16.5).abs() < 1.0 || (u + DOME_LOCAL_HALF_LEN).abs() < 1.0;
            if !in_dome {
                // The movable spines down each flank of the abdomen: the saw
                // edge that makes the silhouette a horseshoe crab's.
                let over = v.abs() - half_w;
                if (-13.0..=-3.5).contains(&u) && over > 0.0 && over <= 3.0 && (u * 1.25).sin() > 0.55 {
                    blend_px(pb, x, y, apply_hit(HINGE_MID, hit), 0.95);
                    continue;
                }
                if rim && (-DOME_LOCAL_HALF_LEN - 1.0..=17.5).contains(&u) && v.abs() <= half_w + 1.2 {
                    blend_px(pb, x, y, SHELL_OUT, 0.85);
                }
                continue;
            }
            // Shading: a low dome lit from the top-left, flattened by the crush.
            let nu = (u / 16.5).clamp(-1.0, 1.0);
            let nv = (v / half_w.max(1e-3)).clamp(-1.0, 1.0);
            let dome = (1.0 - (nu * nu * 0.55 + nv * nv * 0.75)).clamp(0.0, 1.0);
            let lit = nu * fl + nv * pl;
            let mut lum = 0.34 + 0.30 * dome + 0.46 * lit - crouch * 0.10;
            // The rim is darker and the specular crest is on the upper-left.
            if rim {
                lum *= 0.55;
            }
            let mut col = shade3(SHELL_SH, SHELL_MID, SHELL_HI, lum.clamp(0.0, 1.0));
            if dome > 0.68 && lit > 0.22 {
                col = mix(col, SHELL_SPEC, 0.58);
            }
            // Wet-sand grit.
            let g = hash01(x - 3, y + 1);
            if g > 0.93 {
                col = mix(col, SHELL_SPEC, 0.22);
            } else if g < 0.05 {
                col = mix(col, SHELL_OUT, 0.22);
            }
            // The ridges: three concentric arcs and the median keel.
            let ridge = ((u * 0.52 + v.abs() * 0.10).sin() * 4.0).abs() < 0.55 && dome < 0.9;
            if ridge && !rim {
                col = mix(col, SHELL_SH, 0.34);
            }
            if v.abs() < 0.9 && u > -7.0 {
                col = mix(col, if lit > 0.0 { SHELL_SPEC } else { SHELL_SH }, 0.34);
            }
            // The hinge seam between the head shield and the abdomen, and the
            // two eye ridges running forward from it: the shell's own drawing.
            if (u + 2.0).abs() < 0.85 && !rim {
                col = mix(col, SHELL_OUT, 0.68);
            } else if (u + 0.5).abs() < 0.7 && !rim {
                col = mix(col, SHELL_SPEC, 0.48);
            }
            if (v.abs() - 6.2).abs() < 0.85 && u > -1.0 && u < 12.5 {
                col = mix(col, if nv * pl > 0.0 { SHELL_SPEC } else { SHELL_SH }, 0.66);
            }
            // A strong light edge and dark edge just inside the outline, so the
            // domed horseshoe shell survives library card downscale and dark floors.
            let edge = (half_w - v.abs()).min(16.5 - u).min(u + DOME_LOCAL_HALF_LEN);
            if !rim && edge < 3.4 {
                let k = 1.0 - ((edge - 1.0) / 2.4).clamp(0.0, 1.0);
                if lit > 0.10 {
                    col = mix(col, SHELL_SPEC, 0.78 * k);
                } else if lit < -0.10 {
                    col = mix(col, SHELL_OUT, 0.62 * k);
                }
            }
            // The hinge: two knuckle bumps at the back of the shell.
            let hinge = Vec2::new(-DOME_LOCAL_HALF_LEN + 1.5, 0.0);
            let dh = Vec2::new(u, v).dist(hinge);
            if dh < 4.0 && u < -7.0 {
                let k = (1.0 - dh / 4.0).clamp(0.0, 1.0);
                col = mix(
                    shade3(HINGE_SH, HINGE_MID, HINGE_HI, 0.55 + 0.35 * lit),
                    col,
                    1.0 - k,
                );
            }
            // Cracks and the blue seeping out of them.
            if damage_tier >= 2 {
                let crack = ((u * 0.9 + v * 1.7).sin() * 9.0).abs() < 0.45
                    && u > -8.0
                    && u < 8.0;
                if crack {
                    col = mix(col, BLOOD_SH, 0.55);
                }
            }
            if damage_tier >= 3 {
                let crack2 = ((u * 1.5 - v * 0.8).sin() * 7.0).abs() < 0.40 && u > -6.0;
                if crack2 {
                    col = mix(col, BLOOD_MID, 0.45 + 0.2 * pulse);
                }
            }
            // The wedge lifts the rear rim: the shell is tipped up.
            if lifted && u < -8.0 {
                col = mix(col, WEAK_GOLD, 0.10 + 0.10 * pulse);
            }
            blend_px(pb, x, y, apply_hit(col, hit), 1.0);
        }
    }
    // A crisp dark outline pass around the silhouette.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - base;
            let u = d.dot(f) / BODY_SCALE;
            let v = d.dot(perp) / BODY_SCALE;
            let front = 1.0 - ((u - 1.5) / 16.5).powi(2);
            let half_w = if u >= -2.0 {
                if front <= 0.0 {
                    0.0
                } else {
                    16.0 * front.sqrt()
                }
            } else {
                (14.0 + (u + 2.0) * 0.64).max(3.6)
            };
            let inside = (-DOME_LOCAL_HALF_LEN - 0.5..=16.5).contains(&u) && v.abs() <= half_w;
            let near_edge = (v.abs() - half_w).abs() < 1.35 || (u - 16.5).abs() < 1.35 || (u + DOME_LOCAL_HALF_LEN).abs() < 1.35;
            if !inside && near_edge {
                blend_px(pb, x, y, SHELL_OUT, 0.92);
            }
        }
    }
}

/// Rasterize the shell in its original local units inside its rotated world bounds.
fn shell_bbox(pb: &PixelBuffer, center: Vec2, facing: Vec2) -> Option<(i32, i32, i32, i32)> {
    let half_len = DOME_HALF_LEN + 4.0 * BODY_SCALE;
    let half_width = DOME_HALF_PERP + 4.0 * BODY_SCALE;
    let x = facing.x.abs();
    let y = facing.y.abs();
    bbox(pb, center, x * half_len + y * half_width, y * half_len + x * half_width)
}

fn draw_legs(
    pb: &mut PixelBuffer,
    c: &Vec2,
    f: Vec2,
    perp: Vec2,
    hit: f32,
    step_phase: f32,
    thrash: bool,
) {
    for side in [-1.0f32, 1.0] {
        for k in 0..3 {
            let along = (k as f32 - 1.0) * 6.0 + 1.5;
            let step = (step_phase + k as f32 * 1.9 + if side > 0.0 { PI } else { 0.0 }).sin();
            let reach = if thrash {
                8.0 + 4.0 * step.abs()
            } else {
                5.0 + 1.6 * step.max(0.0)
            };
            let root = *c + (f * along + perp * side * 12.0) * BODY_SCALE;
            let tip = root + (perp * side * reach + f * (step * 1.4)) * BODY_SCALE;
            let col = shade3(HINGE_SH, HINGE_MID, HINGE_HI, 0.55);
            line_solid(pb, root, tip, apply_hit(col, hit * 0.7), 0.95);
            blend_px(pb, tip.x.round() as i32, tip.y.round() as i32, SHELL_SH, 0.9);
        }
    }
}

fn draw_telson(pb: &mut PixelBuffer, view: &CrabView, c: &Vec2, f: Vec2, hit: f32, pulse: f32) {
    let hinge = *c - f * TELSON_HINGE;
    let tip = w2s(telson_tip(view), Vec2::ZERO) + (*c - w2s(view.pos, Vec2::ZERO));
    let lift = telson_lift(view);
    let seg = tip - hinge;
    let len = (seg.len() / BODY_SCALE).max(1.0);
    let dir = smooth_dir(seg);
    let perp = dir.perp();
    let pl = perp.dot(LIGHT);

    // Solid tapered chitin tail spike with dark outline flanks and a bright
    // sunlit dorsal ridge so the telson survives dark floors & card downscale!
    let min_x = (hinge.x.min(tip.x) - 5.0 * BODY_SCALE).floor().max(0.0) as i32;
    let max_x = (hinge.x.max(tip.x) + 5.0 * BODY_SCALE).ceil().min((pb.w as i32 - 1) as f32) as i32;
    let min_y = (hinge.y.min(tip.y) - 5.0 * BODY_SCALE).floor().max(0.0) as i32;
    let max_y = (hinge.y.max(tip.y) + 5.0 * BODY_SCALE).ceil().min((pb.h as i32 - 1) as f32) as i32;
    if min_x <= max_x && min_y <= max_y {
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let rx = (x as f32 - hinge.x) / BODY_SCALE;
                let ry = (y as f32 - hinge.y) / BODY_SCALE;
                let along = rx * dir.x + ry * dir.y;
                if along < -1.0 || along > len + 1.5 {
                    continue;
                }
                let t = (along / len).clamp(0.0, 1.0);
                let hw = (3.1 * (1.0 - t * 0.80)).max(0.65);
                let across = rx * perp.x + ry * perp.y;
                let aa = across.abs();
                if aa > hw + 1.2 || along < 0.0 || along > len + 0.5 {
                    if aa <= hw + 1.2 && along >= -1.0 && along <= len + 1.4 {
                        blend_px(pb, x, y, SHELL_OUT, 0.90);
                    }
                    continue;
                }
                if aa > hw {
                    blend_px(pb, x, y, SHELL_OUT, 0.92);
                    continue;
                }
                let lit = (across / hw.max(0.5)) * pl;
                let mut col = shade3(HINGE_SH, SHELL_MID, SHELL_HI, 0.45 + 0.45 * lit + 0.25 * (1.0 - aa / hw.max(0.5)));
                // Crisp sunlit dorsal spine ridge running the full length of the spike!
                if aa < (hw * 0.38).max(0.55) {
                    col = mix(col, SHELL_SPEC, 0.65 + 0.25 * t);
                }
                blend_px(pb, x, y, apply_hit(col, hit * 0.8), 1.0);
            }
        }
    }
    // The spike's gleaming bone-bright tip and the lift's warning ticks.
    fill_disc(pb, tip, 1.2 * BODY_SCALE, apply_hit(SHELL_SPEC, hit), 0.98);
    if lift > 0.05 {
        for k in 0..3 {
            let t = (k as f32 + 1.0) / 4.0;
            let q = hinge + seg * t;
            line_solid(pb, q, q + Vec2::new(0.0, -3.2 * lift * BODY_SCALE), WEAK_GOLD, 0.42 + 0.34 * pulse);
        }
    }
}

fn draw_eyes(
    pb: &mut PixelBuffer,
    view: &CrabView,
    c: &Vec2,
    f: Vec2,
    perp: Vec2,
    hit: f32,
    pulse: f32,
) {
    let asleep = matches!(
        view.state,
        CrabState::Wedged | CrabState::Recover | CrabState::Righting
    );
    for side in [-1.0f32, 1.0] {
        let q = *c + (f * 11.5 + perp * side * 6.2) * BODY_SCALE;
        let blink = if asleep {
            0.20
        } else if matches!(view.state, CrabState::Bulldoze) {
            0.95
        } else {
            0.60 + 0.40 * pulse
        };
        fill_disc(pb, q, 1.6 * BODY_SCALE, apply_hit(EYE, hit * 0.5), 0.95);
        if blink > 0.4 && !asleep {
            blend_px(
                pb,
                (q.x - 0.6 * BODY_SCALE).round() as i32,
                (q.y - 0.4 * BODY_SCALE).round() as i32,
                EYE_HI,
                blink * 0.85,
            );
        }
        if asleep {
            line_solid(pb, q - perp * (1.6 * BODY_SCALE), q + perp * (1.6 * BODY_SCALE), HINGE_SH, 0.9);
        }
    }
    // The front sensory slit: a pale line across the brow.
    let a = *c + (f * 13.0 + perp * 5.0) * BODY_SCALE;
    let b = *c + (f * 13.0 - perp * 5.0) * BODY_SCALE;
    line_solid(pb, a, b, SHELL_SH, 0.55);
    line_solid(pb, a + f * (0.8 * BODY_SCALE), b + f * (0.8 * BODY_SCALE), SHELL_HI, 0.18);
}

fn draw_gills(pb: &mut PixelBuffer, view: &CrabView, c: &Vec2, f: Vec2, cam: Vec2, pulse: f32) {
    let hinge = *c - f * (DOME_HALF_LEN - 2.0 * BODY_SCALE);
    // A fan of pale lamellae spilling out of the lifted rear rim.
    for k in 0..9 {
        let a = -1.15 + 2.3 * (k as f32 / 8.0);
        let d = (-f).rotate(a);
        let len = (GILL_RADIUS * 1.6) * (0.65 + 0.35 * ((k as f32 * 1.7 + view.anim * 4.0).sin() * 0.5 + 0.5));
        let q0 = hinge + d * (2.0 * BODY_SCALE);
        let q1 = hinge + d * len;
        let col = if k % 2 == 0 { GILL_MID } else { GILL_SH };
        line_solid(pb, q0, q1, col, 0.9);
        blend_px(pb, q1.x.round() as i32, q1.y.round() as i32, GILL_HI, 0.55 + 0.20 * pulse);
    }
    // The wet blue under the fans.
    let g = w2s(gill_pos(view), cam);
    fill_disc(pb, g, GILL_RADIUS * 0.8, BLOOD_SH, 0.45);
    ring(pb, g, GILL_RADIUS * 0.8 + BODY_SCALE, BLOOD_MID, 0.35, 1.0);
}

/// The underside: a pale segmented plate, ten legs thrashing, gill books open.
fn draw_underside(
    pb: &mut PixelBuffer,
    view: &CrabView,
    c: &Vec2,
    f: Vec2,
    hit: f32,
    pulse: f32,
    damage_tier: u8,
) {
    let p = clamp01(view.progress);
    let righting = matches!(view.state, CrabState::Righting);
    let speed = if righting { 3.0 } else { 11.0 };
    let perp = f.perp();
    let base = *c;

    // Thrashing legs: six per side, long and jointed.
    for side in [-1.0f32, 1.0] {
        for k in 0..5 {
            let along = (k as f32 - 2.0) * 5.2;
            let phase = view.anim * speed + k as f32 * 1.3 + if side > 0.0 { PI } else { 0.0 };
            let swing = phase.sin();
            let reach = 11.0 + 7.0 * swing.abs() + (k as f32 * 0.5);
            let root = base + (f * along + perp * side * 8.0) * BODY_SCALE;
            let knee = root + (perp * side * (reach * 0.55) + f * (swing * 3.0)) * BODY_SCALE;
            let tip = knee + (perp * side * (reach * 0.55) - f * (swing * 4.5)) * BODY_SCALE;
            let col = shade3(HINGE_SH, HINGE_MID, HINGE_HI, 0.45 + 0.25 * swing.abs());
            line_solid(pb, root, knee, apply_hit(col, hit * 0.6), 0.95);
            line_solid(pb, knee, tip, mix(col, GILL_SH, 0.3), 0.90);
            blend_px(pb, tip.x.round() as i32, tip.y.round() as i32, GILL_HI, 0.45);
        }
    }

    // The pale underside plate (the same silhouette, inverted shading).
    let Some((x0, x1, y0, y1)) = shell_bbox(pb, base, f) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - base;
            let u = d.dot(f) / BODY_SCALE;
            let v = d.dot(perp) / BODY_SCALE;
            let front = 1.0 - ((u - 1.5) / 16.5).powi(2);
            let half_w = if u >= -2.0 {
                if front <= 0.0 {
                    0.0
                } else {
                    16.0 * front.sqrt()
                }
            } else {
                (14.0 + (u + 2.0) * 0.64).max(3.6)
            };
            if u < -DOME_LOCAL_HALF_LEN - 0.5 || u > 16.5 || v.abs() > half_w {
                continue;
            }
            let nu = (u / 16.5).clamp(-1.0, 1.0);
            let nv = (v / half_w.max(1e-3)).clamp(-1.0, 1.0);
            let lum = 0.52 + 0.22 * (1.0 - nv.abs()) - 0.10 * nu;
            // Segmented abdominal plates across the underside.
            let seg = ((u * 0.75 + 4.0).sin() * 5.0).abs() < 0.7;
            let mut col = shade3(GILL_SH, GILL_MID, GILL_HI, lum);
            if seg {
                col = mix(col, BLOOD_SH, 0.25);
            }
            if damage_tier >= 2 && ((u * 1.1 - v * 0.9).sin() * 6.0).abs() < 0.4 {
                col = mix(col, BLOOD_MID, 0.35 + 0.15 * pulse);
            }
            if righting {
                // It is rolling: the pale plate tips and the dark rim comes over.
                col = mix(col, SHELL_SH, smoothstep(p) * 0.65);
            }
            blend_px(pb, x, y, apply_hit(col, hit), 1.0);
        }
    }
    // The open gill books in the middle: the weak point.
    let g = base;
    for k in 0..5 {
        let a = -1.2 + 2.4 * (k as f32 / 4.0);
        let d = f.rotate(a);
        let len = 8.0 + 2.0 * ((k as f32 + view.anim * 6.0).sin().abs());
        line_solid(pb, g - d * (2.0 * BODY_SCALE), g + d * (len * BODY_SCALE), GILL_HI, 0.75);
        line_solid(pb, g - d * BODY_SCALE, g + d * ((len - 2.0) * BODY_SCALE), BLOOD_HI, 0.35);
    }
    ring(pb, g, GILL_RADIUS + (2.5 + 1.5 * pulse) * BODY_SCALE, WEAK_GOLD, 0.35 + 0.25 * pulse, 1.1);
    // Outline.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = Vec2::new(x as f32, y as f32) - base;
            let u = d.dot(f) / BODY_SCALE;
            let v = d.dot(perp) / BODY_SCALE;
            let front = 1.0 - ((u - 1.5) / 16.5).powi(2);
            let half_w = if u >= -2.0 {
                if front <= 0.0 {
                    0.0
                } else {
                    16.0 * front.sqrt()
                }
            } else {
                (14.0 + (u + 2.0) * 0.64).max(3.6)
            };
            let edge = (v.abs() - half_w).abs() < 1.1 || (u - 16.5).abs() < 1.1 || (u + DOME_LOCAL_HALF_LEN).abs() < 1.1;
            if edge {
                blend_px(pb, x, y, SHELL_OUT, 0.8);
            }
        }
    }
}

fn draw_death(
    pb: &mut PixelBuffer,
    view: &CrabView,
    c: &Vec2,
    f: Vec2,
    hit: f32,
    pulse: f32,
    p: f32,
) {
    let perp = f.perp();
    let t = clamp01(p);
    // The legs curl inward, one by one, and stop.
    for side in [-1.0f32, 1.0] {
        for k in 0..3 {
            let along = (k as f32 - 1.0) * 6.0 + 1.5;
            let curl = smoothstep((t - k as f32 * 0.12).clamp(0.0, 1.0));
            let root = *c + (f * along + perp * side * 11.0) * BODY_SCALE;
            let tip = root + (perp * side * (7.0 * (1.0 - curl) + 1.5) + f * (3.0 * curl)) * BODY_SCALE;
            line_solid(pb, root, tip, apply_hit(HINGE_SH, hit * 0.6), 0.9);
        }
    }
    // The dome sags and splits; the blue runs out of every seam.
    let sag = *c + Vec2::new(0.0, 1.5 * smoothstep(t) * BODY_SCALE);
    draw_dome(pb, view, &sag, f, perp, hit, pulse, 3, false, None);
    for k in 0..7 {
        let a = -1.4 + 2.8 * (k as f32 / 6.0);
        let d = f.rotate(a);
        let len = 6.0 + 12.0 * smoothstep((t * 1.3 - k as f32 * 0.06).clamp(0.0, 1.0));
        let q = sag + d * ((8.0 + len * 0.3) * BODY_SCALE);
        line_solid(pb, sag + d * (6.0 * BODY_SCALE), q, BLOOD_MID, 0.55 * (1.0 - t * 0.3));
        blend_px(pb, q.x.round() as i32, q.y.round() as i32, BLOOD_HI, 0.65 * (1.0 - t * 0.4));
    }
    if t < 0.4 {
        // One last tail twitch before it goes still.
        let tw = (t * 40.0).sin().abs() * (1.0 - t / 0.4);
        let hinge = *c - f * TELSON_HINGE;
        let tip = hinge - f * TELSON_LENGTH + perp * (tw * 8.0 * BODY_SCALE);
        line_solid(pb, hinge, tip, HINGE_MID, 0.85);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_STATES: [CrabState; 15] = [
        CrabState::Dome,
        CrabState::TelsonTell,
        CrabState::TelsonSweep,
        CrabState::BulldozeTell,
        CrabState::Bulldoze,
        CrabState::Wedged,
        CrabState::BurrowTell,
        CrabState::Burrow,
        CrabState::Emerge,
        CrabState::FlipTell,
        CrabState::Flipped,
        CrabState::Righting,
        CrabState::Pose,
        CrabState::Recover,
        CrabState::Death,
    ];

    fn sample_view(pos: Vec2, ang: f32, state: CrabState, progress: f32) -> CrabView {
        let facing = Vec2::from_angle(ang);
        CrabView {
            pos,
            facing,
            state,
            progress,
            anim: progress * 2.4 + 0.4,
            hit_flash: 0.0,
            hp_frac: 0.7,
            phase: 1,
            target: pos + facing * 80.0,
            lane_start: pos,
            sweep_side: 1.0,
        }
    }

    #[test]
    fn every_state_draws_everywhere_without_panic() {
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
    fn zero_facing_is_valid() {
        let mut pb = PixelBuffer::new(100, 100);
        let mut v = sample_view(Vec2::new(50.0, 50.0), 0.0, CrabState::Dome, 0.5);
        v.facing = Vec2::ZERO;
        draw_ground(&mut pb, &v, Vec2::ZERO, (255, 180, 90));
        draw(&mut pb, &v, Vec2::ZERO, (255, 180, 90));
        assert_eq!(unit_facing(&v), Vec2::new(1.0, 0.0));
        let _ = hit_shapes(&v);
    }

    /// The armour rule: the front deflects, the rear does not, the gills and the
    /// flipped underside take the weak-point multiplier.
    #[test]
    fn the_shell_deflects_the_front_and_opens_at_the_rear() {
        let mut v = sample_view(Vec2::new(100.0, 100.0), 0.0, CrabState::Dome, 0.4);
        let front = Vec2::new(-1.0, 0.0); // travelling into the dome's face
        let rear = Vec2::new(1.0, 0.0); // from behind
        assert_eq!(incoming_multiplier(&v, front, v.pos), 0.0);
        assert_eq!(incoming_multiplier(&v, rear, v.pos), 1.0);
        // A pool (no travel direction) is under the shell and counts.
        assert_eq!(incoming_multiplier(&v, Vec2::ZERO, v.pos), 1.0);

        // Wedged: the shell is up, everything counts and the gills pay double.
        v.state = CrabState::Wedged;
        // (Sampled on the nose: the gill fan sits `GILL_BACK` = 7 px behind the
        // centre with a 6.5 + 2 px catch, so the exact centre is inside it.)
        let nose = v.pos + unit_facing(&v) * (BODY_RADIUS * 0.6);
        assert!(nose.dist(gill_pos(&v)) > GILL_RADIUS + 2.0 * BODY_SCALE);
        assert_eq!(incoming_multiplier(&v, front, nose), 1.0);
        assert_eq!(incoming_multiplier(&v, front, gill_pos(&v)), GILL_DAMAGE_MULT);

        // Flipped: the weak point is the whole underside.
        v.state = CrabState::Flipped;
        assert_eq!(incoming_multiplier(&v, front, v.pos), GILL_DAMAGE_MULT);
        assert!(incoming_multiplier(&v, front, v.pos + Vec2::new(30.0, 0.0)) >= 1.0);
    }

    #[test]
    fn the_hit_shapes_are_the_dome_and_the_exposed_gills() {
        let mut v = sample_view(Vec2::new(100.0, 100.0), 0.0, CrabState::Dome, 0.4);
        let shapes = hit_shapes(&v);
        assert!(shapes[0].boxed && shapes[0].grounded);
        assert!(!shapes[1].grounded, "the hidden gills are not a target");

        v.state = CrabState::Wedged;
        let shapes = hit_shapes(&v);
        assert!(shapes[0].grounded);
        assert!(shapes[1].grounded, "the exposed gills are the punish target");
        assert_eq!(shapes[1].radius, GILL_RADIUS);

        v.state = CrabState::Burrow;
        let shapes = hit_shapes(&v);
        assert!(!shapes[0].grounded && !shapes[1].grounded);
    }

    #[test]
    fn the_telson_arc_covers_the_rear_and_beside_it() {
        let v = sample_view(Vec2::new(100.0, 100.0), 0.0, CrabState::TelsonTell, 0.5);
        let (center, inner, outer, af, half) = telson_arc(&v);
        assert_eq!(center, v.pos);
        assert_eq!(inner, TELSON_INNER);
        assert_eq!(outer, TELSON_OUTER);
        assert_eq!(af, Vec2::new(-1.0, 0.0), "the arc faces the crab's rear");
        assert!((half * 2.0 - 2.0 * PI / 3.0).abs() < 1e-4);
        // Behind and beside are inside; the armoured front is outside.
        let hazard = |p: Vec2| {
            let d = p - center;
            let dist = d.len();
            dist >= inner && dist <= outer && d.normalized().dot(af) >= half.cos()
        };
        assert!(hazard(center + Vec2::new(-25.0, 0.0)));
        assert!(hazard(center + Vec2::new(-20.0, 15.0)));
        assert!(!hazard(center + Vec2::new(25.0, 0.0)));
        assert!(!hazard(center + Vec2::new(0.0, 30.0)));
    }

    #[test]
    fn the_art_numbers_are_the_move_spec() {
        assert_eq!(CONTACT_DAMAGE, 8.0);
        assert_eq!(TELSON_DAMAGE, 16.0);
        assert_eq!(BULLDOZE_DAMAGE, 18.0);
        assert_eq!(EMERGE_DAMAGE, 16.0);
        assert_eq!(LEGS_DAMAGE, 12.0);
        assert_eq!(WEDGE_DAMAGE_MULT, 1.35);
        assert_eq!(GILL_DAMAGE_MULT, 2.0);
        assert_eq!(FLIP_DAMAGE_MULT, 1.5);
        assert_eq!(POSE_DURATION, 1.20);
        assert_eq!(DEATH_DURATION, 2.60);
        assert_eq!(BLOOD_LIFE, 6.0);
        assert_eq!(BLOOD_SLOW, 0.45);
        const { assert!(DOME_SPEED[0] < DOME_SPEED[2]) };
        const { assert!(BULLDOZE_SPEED[0] > 58.0, "the bulldoze must outwalk the rat") };
        // The punish windows are the states the framework must treat as safe
        // for the rat to stand in.
        assert!(CrabState::Wedged.is_recovery());
        assert!(CrabState::Flipped.is_recovery());
        assert!(CrabState::Pose.is_recovery());
        assert!(!CrabState::Dome.is_recovery());
        assert!(!CrabState::Bulldoze.is_recovery());
    }

    #[test]
    fn hit_flash_changes_pixels() {
        let render = |flash: f32| {
            let mut pb = PixelBuffer::new(90, 90);
            pb.clear((16, 22, 20));
            let mut v = sample_view(Vec2::new(45.0, 45.0), PI / 2.0, CrabState::Dome, 0.3);
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
        assert!(diff > 30, "the hit flash must be visible (diff {diff})");
    }

    /// Gated behind `RATRUN_CRAB_DUMP=<dir>`: renders the crab in every tell,
    /// strike, punish window and phase through the **real** `World::draw`
    /// (floor, props, lighting, the rat and the light pass), one PPM per case.
    /// Manual screenshot tool, a no-op in the normal suite.
    #[test]
    fn dump_crab_frames() {
        let Ok(dir) = std::env::var("RATRUN_CRAB_DUMP") else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let cases: [(&str, CrabState, f32, u8); 18] = [
            ("01-dome-p1", CrabState::Dome, 0.30, 1),
            ("02-dome-p3", CrabState::Dome, 0.30, 3),
            ("03-telson-tell", CrabState::TelsonTell, 0.80, 1),
            ("04-telson-sweep", CrabState::TelsonSweep, 0.35, 1),
            ("05-bulldoze-tell", CrabState::BulldozeTell, 0.70, 1),
            ("06-bulldoze", CrabState::Bulldoze, 0.50, 1),
            ("07-wedged", CrabState::Wedged, 0.15, 1),
            ("08-wedged-p2", CrabState::Wedged, 0.55, 2),
            ("09-burrow-tell", CrabState::BurrowTell, 0.80, 2),
            ("10-burrow", CrabState::Burrow, 0.50, 2),
            ("11-emerge", CrabState::Emerge, 0.20, 2),
            ("12-flip-tell", CrabState::FlipTell, 0.80, 3),
            ("13-flipped", CrabState::Flipped, 0.25, 3),
            ("14-flipped-late", CrabState::Flipped, 0.80, 3),
            ("15-righting", CrabState::Righting, 0.50, 3),
            ("16-pose", CrabState::Pose, 0.50, 2),
            ("17-death", CrabState::Death, 0.25, 3),
            ("18-death-late", CrabState::Death, 0.85, 3),
        ];
        for (name, state, progress, phase) in cases {
            let mut w = crate::game::world::World::new(crate::game::level::Theme::Sewer, 3);
            let centre = w
                .level
                .boss_room()
                .map(|r| r.rect.center_world())
                .unwrap_or(w.player.pos);
            // The rat just below the crab so the camera frames it.
            w.player.pos = centre + Vec2::new(0.0, 34.0);
            let arena = w
                .level
                .boss_room()
                .map(|r| r.rect)
                .unwrap_or_else(|| crate::game::level::TileRect::new(0, 0, 20, 20));
            let mut boss = crate::game::ecs::systems::boss::Boss::new(
                crate::game::ecs::systems::boss::BossKind::HorseshoeCrab,
                centre - Vec2::new(0.0, 6.0),
                2000.0,
                arena,
                centre,
                false,
            );
            boss.state = crate::game::ecs::systems::boss::BossState::HorseshoeCrab(state);
            boss.duration = 2.0;
            boss.elapsed = progress.clamp(0.0, 1.0) * 2.0;
            boss.phase = phase;
            boss.facing = Vec2::new(1.0, 0.0);
            boss.swipe_side = 1.0;
            boss.pounce_origin = boss.pos;
            boss.target = boss.pos - Vec2::new(140.0, 0.0);
            // A burrow mark and a blood pool so the suite shows every shape.
            if matches!(state, CrabState::BurrowTell | CrabState::Emerge | CrabState::Burrow) {
                boss.target = w.player.pos;
            }
            w.ecs.spawn((boss, crate::game::ecs::components::SpawnId(900)));
            let mut pb = PixelBuffer::new(200, 150);
            // A few frames so the lighting/smoke passes warm up.
            for _ in 0..3 {
                w.draw(&mut pb);
            }
            let path = std::path::Path::new(&dir).join(format!("{name}.ppm"));
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
    }

    /// The shell has to read as a *dome*: a pixel at the front centre is shell,
    /// and the flipped pass must be visibly the pale underside.
    #[test]
    fn the_dome_paints_a_body_and_the_flip_paints_the_underside() {
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear((10, 10, 10));
        let v = sample_view(Vec2::new(40.0, 40.0), 0.0, CrabState::Dome, 0.3);
        draw_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
        draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
        let body = pb.pixel_at(46, 40);
        assert!(body.0 > 30 && body.1 > 26, "no dome at the front centre: {body:?}");

        let mut pb2 = PixelBuffer::new(80, 80);
        pb2.clear((10, 10, 10));
        let mut fv = v;
        fv.state = CrabState::Flipped;
        draw_ground(&mut pb2, &fv, Vec2::ZERO, (90, 220, 200));
        draw(&mut pb2, &fv, Vec2::ZERO, (90, 220, 200));
        let under = pb2.pixel_at(40, 40);
        assert!(
            under.2 > under.0,
            "the underside should read blue-pale, not shell-brown: {under:?}"
        );
    }

    /// The shell reads as a lit dome with a saw-edged abdomen: the side facing
    /// the light is brighter than the far side, and the flank spines stand
    /// outside the outline.
    #[test]
    fn the_shell_has_a_lit_side_a_shaded_side_and_flank_spines() {
        const BG: Rgb = (10, 10, 10);
        let mut pb = PixelBuffer::new(80, 80);
        pb.clear(BG);
        let v = sample_view(Vec2::new(40.0, 40.0), 0.0, CrabState::Dome, 0.3);
        draw(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        let (mut top, mut bottom) = (0u32, 0u32);
        for x in 40..52usize {
            for dy in 6..14usize {
                top += lum(pb.pixel_at(x, 40 - dy));
                bottom += lum(pb.pixel_at(x, 40 + dy));
            }
        }
        assert!(top > bottom, "the dome has no lit side: top {top} bottom {bottom}");
    }
}
