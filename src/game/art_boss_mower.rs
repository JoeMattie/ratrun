//! Art and gameplay specification for the Robot Lawnmower Boss ("TURF-REAPER MK.IV").
//!
//! A domestic autonomous lawnmower turned murderous: low sculpted orange-and-gunmetal
//! shell seen from above, rubberized hazard-striped wrap-around impact bumper, chunky
//! treaded rear drive wheels, big red top-deck emergency STOP button, smoked-glass
//! status LED visor with scanning lidar eye, and whirling steel rotary blades glimpsed
//! beneath the deck skirt.
//!
//! # Layering contract for implementers
//! Call the draw functions in this order inside `World::draw`:
//! 1. **Ground layer** (after `art_floor::draw_floor`, stains, and props; before gems,
//!    bullets, enemies, and the player):
//!    - [`draw_dock_pad`] (optional persistent call if the arena's charging dock stays
//!      visible throughout the entire boss fight).
//!    - [`draw_mower_ground`] (mown/scorched charge trail, scan lidar cone, charge laser
//!      sight & corridor rails, ricochet bank preview, blade-fling sector telegraphs,
//!      docking power tether, death blast ring/crater, and the boss's drop shadow).
//! 2. **Entity layer** (alongside `art_enemy::draw_enemy` and `art_rat::draw_rat`):
//!    - [`draw_mower`] (spinning under-deck blades, wheels, bumper, sculpted chassis,
//!      progressive phase damage, hit sparks, flipped turtle underbelly, death explosion
//!      sequence, and radial boss HP arc).
//! 3. **Projectile layer** (in the enemy-bullet pass):
//!    - [`draw_blade_disc`] and [`draw_grass_shrapnel`] for projectiles spawned during
//!      [`MowerState::FlingBlades`].
//!
//! # Boss fight pacing & phases
//! - Recommended base HP: `1600.0` (comparable to the existing Boss at `1400.0`,
//!   offset by vulnerable `1.5×` / `2.0×` punish windows).
//! - **Phase 1 (`hp_frac > 0.66`)**: Pristine shell. Alternates `Patrol` → `Scan` →
//!   `ChargeWindup` → `Charge` (1 straight pass) and occasional `FlingBlades` (6-way).
//!   Hitting a wall at the end of `Charge` flips it briefly into `Flipped` (or triggers
//!   `Overheat` after `FlingBlades`). Triggers a `Docking` retreat once at `<= 0.66` HP.
//! - **Phase 2 (`0.33 < hp_frac <= 0.66`)**: Cracked cowl, smoking louver, wider exposed
//!   blade gap. `Charge` chains into `Ricochet` (up to 3 wall bounces), `FlingBlades`
//!   fires 8-way spiral discs + grass shrapnel. Triggers a second `Docking` retreat at
//!   `<= 0.33` HP.
//! - **Phase 3 (`hp_frac <= 0.33`)**: Rear hood blown off exposing glowing motor coils,
//!   crackling arcs, flickering eye. Faster `Scan` lock, 4-bounce `Ricochet` ending in
//!   a longer `Flipped` turtle window, denser `FlingBlades` waves.
//! - **Zero-allocation & 60 FPS**: Every draw function uses only stack storage and culls
//!   early when outside `PixelBuffer` bounds.

#![allow(dead_code)]

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::{mix, scale, Rgb};

// ============================================================================
// Collision & Gameplay Constants (Art <-> Hitbox Contract)
// ============================================================================

/// Primary circular collision radius of the mower chassis in world pixels.
/// Matches the outer perimeter of the shell and rear drive wheels (`~26×28 px`).
pub const BODY_RADIUS: f32 = 13.0;

/// Tighter collision radius while flipped belly-up ([`MowerState::Flipped`]),
/// when the wheels kick inward and the mower rocks on its rounded crown.
pub const FLIPPED_RADIUS: f32 = 11.5;

/// Oriented chassis half-length along `facing` (`+u` nose to `-u` rear tail).
pub const CHASSIS_HALF_LEN: f32 = 13.5;

/// Oriented chassis half-width across `facing.perp()`.
pub const CHASSIS_HALF_WIDTH: f32 = 11.5;

/// Half-width of the lethal straight-line charge corridor (matches the ground
/// telegraph rails drawn during [`MowerState::ChargeWindup`]).
pub const CHARGE_CORRIDOR_HALF_WIDTH: f32 = 12.5;

/// Effective mowing/blade deck radius beneath the chassis (active during
/// `Patrol`, `Charge`, `Ricochet`, and `FlingBlades`).
pub const BLADE_DECK_RADIUS: f32 = 12.0;

/// Maximum reach of the rotating lidar sensor cone in [`MowerState::Scan`].
pub const SCAN_CONE_RANGE: f32 = 72.0;

/// Half-angle (radians) of the lidar sensor cone in [`MowerState::Scan`]
/// (~24 degrees each side, ~48 degrees total sweep width).
pub const SCAN_CONE_HALF_ANGLE: f32 = 0.42;

/// Half-extents `(half_w, half_h)` of the axis-aligned charging dock floor pad
/// centered at `dock_pos` (total footprint `32 × 30` world pixels).
pub const DOCK_HALF_SIZE: Vec2 = Vec2 { x: 16.0, y: 15.0 };

/// Collision radius of a flung spinning steel blade disc ([`draw_blade_disc`]).
pub const BLADE_DISC_RADIUS: f32 = 3.5;

/// Collision radius of a razor grass-clipping shrapnel dart ([`draw_grass_shrapnel`]).
pub const SHRAPNEL_RADIUS: f32 = 2.0;

/// Radius of the final meltdown explosion during [`MowerState::Dying`] (`progress ≈ 0.45`).
pub const DEATH_BLAST_RADIUS: f32 = 28.0;

/// Active collision radius for the mower's current state. Returns `0.0` once the
/// chassis has detonated past the blast apex in [`MowerState::Dying`].
#[inline]
pub fn collision_radius(state: MowerState, progress: f32) -> f32 {
    match state {
        MowerState::Flipped => FLIPPED_RADIUS,
        MowerState::Dying if progress >= 0.48 => 0.0,
        _ => BODY_RADIUS,
    }
}

/// Incoming damage multiplier for the mower's current state.
/// Rewards players for baiting charges into walls (`Flipped = 2.0×`) and
/// punishing exhaust windows (`Overheat = 1.5×`).
#[inline]
pub fn incoming_damage_mult(state: MowerState) -> f32 {
    match state {
        MowerState::Overheat => 1.5,
        MowerState::Flipped => 2.0,
        MowerState::Dying => 0.0,
        _ => 1.0,
    }
}

// ============================================================================
// Public View & State Spec
// ============================================================================

/// Move set and state machine for the Robot Lawnmower Boss.
///
/// Every variant documents the exact timing, telegraph, lethal area, damage
/// (relative to Skitterer = `6`, Brute = `22`, Hornet Boss = `30`, Player = `100 HP`),
/// and counterplay the gameplay implementers should build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MowerState {
    /// **Domestic Cruise / Repositioning**
    ///
    /// - **Behavior**: Cruises steadily (`24 px/s` Phase 1, `30 px/s` Phase 2, `36 px/s` Phase 3)
    ///   using flow-field steering to keep mid-range (`45..85 px`) from the rat between attacks.
    ///   LED bar glows calm cyan-green; blades hum under the skirt.
    /// - **Duration**: `1.6s..2.4s` (shorter in later phases), then transitions to `Scan`
    ///   or `FlingBlades` (or `Docking` when crossing a phase threshold).
    /// - **Warning**: Passive state; no attack telegraph.
    /// - **Dangerous area**: Body contact circle of radius [`BODY_RADIUS`] (`13.0 px`) around `pos`.
    /// - **Damage**: `14` contact damage (moderate; slightly above a Cat's `12`).
    /// - **Counter**: Free chip-damage window while kiting; watch for the visor switching to
    ///   radar-green (`Scan`) or side chutes flaring (`FlingBlades`).
    Patrol,

    /// **Lidar Target Acquisition Cone**
    ///
    /// - **Behavior**: Stops or slows to `8 px/s` and sweeps its front lidar sensor cone
    ///   `±0.65 rad` around `facing` (`progress 0.0..0.70`), then locks onto the player's
    ///   position (`target`) for the final beat (`progress 0.70..1.0`).
    /// - **Duration**: `1.35s` (Phase 1), `1.10s` (Phase 2), `0.90s` (Phase 3).
    /// - **Warning**: Translucent emerald radar cone (`SCAN_CONE_RANGE = 72 px`, half-angle
    ///   `SCAN_CONE_HALF_ANGLE = 0.42 rad`) sweeping on the ground layer; turns amber-crimson
    ///   with a corner-bracket lock reticle at `target` during the last `0.35s`.
    /// - **Dangerous area**: Cone itself deals **0 damage** (sensor only). Chassis body
    ///   circle (`BODY_RADIUS = 13.0 px`) still deals `14` contact damage.
    /// - **Damage**: `0` from cone (`14` body contact).
    /// - **Counter**: Duck behind a solid wall/prop or rat-hole, or circle behind the
    ///   mower's rear (`-facing`) before lock completes. If the player evades the cone for
    ///   the full scan, the mower misses its lock and briefly vents (`Overheat`, `1.2s`);
    ///   if locked, it chains immediately into `ChargeWindup`.
    Scan,

    /// **Laser Lock & Motor Rev (Charge Telegraph)**
    ///
    /// - **Behavior**: Anchors in place, spins its rear drive wheels against the floor
    ///   kicking up sparks, tilts slightly back on its shocks, and holds a locked aim line
    ///   from `pos` to `target` (tracking stops at `progress = 0.45` so the player can
    ///   cleanly sidestep or dash out of the lane!).
    /// - **Duration**: `0.85s` (Phase 1), `0.72s` (Phase 2), `0.60s` (Phase 3).
    /// - **Warning**: Full duration (`0.60s..0.85s`). Ground layer paints a pulsing crimson
    ///   laser sightline from nose to `target`, flanking corridor rails at
    ///   `±CHARGE_CORRIDOR_HALF_WIDTH` (`±12.5 px`), marching chevron arrows, and an `X`
    ///   impact marker at `target`. Top LED bar fills left-to-right to blazing red.
    /// - **Dangerous area**: Body contact circle (`BODY_RADIUS = 13.0 px`) only; laser and
    ///   corridor rails deal **0 damage** during windup.
    /// - **Damage**: `14` body contact only.
    /// - **Counter**: Wait until aim freezes at `progress = 0.45`, then move perpendicular
    ///   (`58 px/s` clears `12.5 + 3.0 = 15.5 px` in `0.27s`, well inside the remaining
    ///   `0.33s..0.47s` window!) or dash across the lane. Baiting the line into a thick wall
    ///   sets up a `Flipped` punish!
    ChargeWindup,

    /// **High-Speed Mowing Ram**
    ///
    /// - **Behavior**: Rockets straight along `trail_start -> target` at `165 px/s` (Phase 1),
    ///   `190 px/s` (Phase 2), `215 px/s` (Phase 3) with blades roaring under the front skirt,
    ///   leaving a mown/scorched trail strip behind it. Stops upon reaching `target` or hitting
    ///   an arena wall/prop.
    /// - **Duration**: `0.55s..0.95s` (distance-bounded, max travel `135 px`).
    /// - **Warning**: Preceded by full `ChargeWindup` (`0.60s..0.85s`); during travel, a faint
    ///   forward guide line shows the remaining stopping point at `target`.
    /// - **Dangerous area**:
    ///   - Chassis ram circle of radius [`BODY_RADIUS`] (`13.0 px`) around `pos` (swept capsule
    ///     of half-width [`CHARGE_CORRIDOR_HALF_WIDTH`] `12.5 px` per sub-step).
    ///   - Fresh scorched blade trail along `trail_start -> pos` (half-width `9.0 px`) stays
    ///     hot for `0.6s` as a mild hazard (`4 DPS` / `4` tick damage, avoidable by not walking
    ///     directly down its wake).
    /// - **Damage**: `26` on direct charge impact (harder than a Brute's `22`, fair for a
    ///   heavily telegraphed boss ram; `4` tick damage from glowing scorch wake).
    /// - **Counter**: Sidestep perpendicular or dash (`0.18s` i-frames) through it. In Phase 1,
    ///   slamming a wall transitions straight into `Flipped` (turtle punish!); in Phase 2/3,
    ///   wall impact chains into `Ricochet`.
    Charge,

    /// **Wall-Bounce Pinball Barrage (Phase 2 & 3)**
    ///
    /// - **Behavior**: On wall impact after `Charge`, rebounds off the surface normal at
    ///   `155..185 px/s` for up to `3` bounces (Phase 2) or `4` bounces (Phase 3), tracking
    ///   the current bounce index in `bounce_count`. Each wall slam pauses `0.18s` (hitstop +
    ///   spark ring at `trail_start`) while painting the next dashed reflection line to `target`.
    /// - **Duration**: `0.45s..0.75s` per bounce leg (`~1.8s..2.8s` total chain).
    /// - **Warning**: `0.18s` wall-impact freeze + bright amber-red reflection dashed line from
    ///   `pos` to the next wall `target` before each leg accelerates.
    /// - **Dangerous area**: Swept chassis circle of radius [`BODY_RADIUS`] (`13.0 px`) along
    ///   each bounce leg.
    /// - **Damage**: `22` per ram hit (same as Brute; slightly below primary `Charge` since
    ///   bounces chain quickly).
    /// - **Counter**: Read the dashed rebound vector during the wall-impact pause and duck into
    ///   a side pocket or rat-hole. Surviving the final bounce (`bounce_count == max`) throws
    ///   the mower off balance directly into [`MowerState::Flipped`]!
    Ricochet,

    /// **Rotary Blade-Disc & Grass-Shrapnel Volley**
    ///
    /// - **Behavior**: Brakes in place, revs the under-deck saw to white-hot RPM (`progress
    ///   0.0..0.35` telegraph), pops open its side discharge chutes, and launches rotating waves
    ///   of spinning steel blade discs (`BLADE_DISC_RADIUS = 3.5 px`, speed `72 px/s`) interleaved
    ///   with fan sprays of razor grass-clipping shrapnel (`SHRAPNEL_RADIUS = 2.0 px`, speed
    ///   `92 px/s`) during `progress 0.35..0.90`.
    ///   - Phase 1: `6` discs in 2 alternating waves + narrow clipping fan.
    ///   - Phase 2: `8` discs in 2 spiral waves (`+22.5°` offset) + 3-way clipping fans.
    ///   - Phase 3: `8` discs in 3 rapid spiral waves + bouncing discs off outer walls once.
    /// - **Duration**: `1.80s` (`0.63s` windup telegraph + `1.00s` firing + `0.17s` spin-down).
    /// - **Warning**: First `0.63s` (`progress 0.0..0.35`) projects glowing radial ejection
    ///   spokes and a `22 px` danger ring on the ground showing exact disc launch vectors,
    ///   while the mower's under-deck flashes bright cyan-white.
    /// - **Dangerous area**:
    ///   - Close-range spinning deck aura: radius `15.5 px` around `pos` during `0.35..0.90`
    ///     (`18` damage — discourages hugging the boss while it flings).
    ///   - Spawned projectiles: blade discs (`r = 3.5 px`, `16` damage) and grass shrapnel
    ///     (`r = 2.0 px`, `9` damage).
    /// - **Damage**: `16` per blade disc, `9` per grass-clipping dart, `18` close-range deck.
    /// - **Counter**: Back off outside `24 px`, read the ground spokes to stand in a safe lane,
    ///   and weave between the rotating spiral gaps (projectiles travel slowly enough to thread
    ///   at `58 px/s`). Transitions cleanly into `Overheat` after Phase 2/3 volleys.
    FlingBlades,

    /// **Thermal Shutdown / Exhaust Vent (Vulnerable Window)**
    ///
    /// - **Behavior**: Stalls upright after heavy attacks (`FlingBlades` or an interrupted
    ///   `Docking` / missed `Scan`). Rear louvers pop open venting grey-white steam clouds,
    ///   top red STOP button pops & blinks, status LED flashes ERROR-RED, and blades sputter
    ///   harmlessly to a stop. Moves at `0 px/s`.
    /// - **Duration**: `2.2s` (`1.2s` if triggered by a missed `Scan`).
    /// - **Warning**: Instant visual shift to blinking ERROR-RED LED + billowing steam plumes.
    /// - **Dangerous area**: Body still blocks movement (`BODY_RADIUS = 13.0 px`), but blades
    ///   are disengaged — **0 contact damage** (only gentle steam knockback if pressed into).
    /// - **Damage**: `0` (pure punish window!).
    /// - **Counter**: Rush in and dump close-range weapons (`Gnaw`, `TailWhip`, `CheeseSpray`)
    ///   — the mower takes **`1.5×` damage** (`incoming_damage_mult(Overheat)`) while venting!
    Overheat,

    /// **Flipped-Over Turtle Moment (Prime Vulnerable Window)**
    ///
    /// - **Behavior**: Triggered when a `Charge` slams hard into a wall (Phase 1) or at the end
    ///   of a `Ricochet` chain (Phase 2/3).
    ///   - `progress 0.00..0.15`: Rolls violently onto its rounded back (foreshortened flip).
    ///   - `progress 0.15..0.85`: Helplessly rocks belly-up like a turtle, kicking its 4 wheels
    ///     in the air, sputtering stalled underside blades, blinking ERROR-RED, and exposing its
    ///     glowing green-gold battery pack & PCB belly core!
    ///   - `progress 0.85..1.00`: Hydraulic bumper kick self-rights back onto its wheels with a
    ///     small dust puff.
    /// - **Duration**: `3.0s` (Phase 1), `2.6s` (Phase 2), `2.3s` (Phase 3).
    /// - **Warning**: Dramatic roll animation + blinking red error light + bright glowing
    ///   exposed battery belly; during `0.85..1.00` the roll-back animation telegraphs recovery.
    /// - **Dangerous area**: **0 damage** during `0.15..0.85` (blades are stalled!). During
    ///   the self-righting slam (`0.92..1.00`), a tight `16 px` radius knockback wave deals `10`
    ///   damage to push greedy players back.
    /// - **Damage**: `0` while flipped (`10` on final self-righting thud if still hugging it).
    /// - **Counter**: **Prime punish window** — deals **`2.0×` damage**
    ///   (`incoming_damage_mult(Flipped)`) to the exposed belly core; back off past `17 px`
    ///   when it starts rolling upright at `progress > 0.82`.
    Flipped,

    /// **Charging Dock Retreat & Recharge**
    ///
    /// - **Behavior**: Triggered once per phase transition (`hp_frac <= 0.66` and `<= 0.33`).
    ///   Drives briskly (`48 px/s`) to `dock_pos` (`progress 0.0..0.30`), aligns its rear brass
    ///   prongs onto the Charging Dock pad, and channels a cyan-gold induction recharge
    ///   (`progress 0.30..1.0`), regenerating `4%` max HP per second (up to `12%` total) and
    ///   pulsing protective perimeter sparks if left uncontested.
    /// - **Duration**: Up to `4.0s` (`~1.0s` transit + `3.0s` charging), or ends immediately
    ///   when interrupted.
    /// - **Warning**: Dock pad lights up bright cyan-gold on the floor (`DOCK_HALF_SIZE = 16×15`)
    ///   with a pulsing energy tether connecting dock to mower and a circular recharge progress
    ///   ring filling around the pad.
    /// - **Dangerous area**: Body contact circle (`BODY_RADIUS = 13.0 px`, `12` contact damage);
    ///   at `progress = 0.65` emits one slow defensive shock ring (`r = 26 px`, `12` damage,
    ///   telegraphed by bright gold arcs `0.45s` prior).
    /// - **Damage**: `12` contact / `12` defensive surge pulse.
    /// - **Counter**: Chase the mower to `dock_pos` and deal `120` burst damage during charging
    ///   (or dash onto the dock's glowing front kill-switch) to **short-circuit the recharge**,
    ///   cancelling healing and forcing an immediate `2.2s` [`MowerState::Overheat`] stun!
    Docking,

    /// **Multi-Stage Mechanical Meltdown & Explosion Sequence**
    ///
    /// - **Behavior**: Triggered when `hp_frac <= 0.0`.
    ///   - `progress 0.00..0.40`: Runaway meltdown — violent chassis jitter, electric arcs
    ///     snapping from cracked seams, ERROR-WHITE/RED strobe, ground blast warning ring
    ///     expanding to [`DEATH_BLAST_RADIUS`] (`28.0 px`).
    ///   - `progress 0.40..0.72`: Core capacitor detonation — blinding fireball bursts out of
    ///     the shell, launching detached wheel, bent rotary blade, and orange cowl fragments
    ///     outward.
    ///   - `progress 0.72..1.00`: Expanding ember shockwave fades over a blackened scorch
    ///     crater and smoking wreckage ring; boss is reaped at `progress >= 1.0`.
    /// - **Duration**: `2.4s`.
    /// - **Warning**: `0.96s` (`progress 0.0..0.40`) expanding dashed crimson blast radius ring
    ///   at `DEATH_BLAST_RADIUS = 28.0 px` while the chassis sparks wildly.
    /// - **Dangerous area**: Final detonation at `progress = 0.42..0.52` hits within
    ///   [`DEATH_BLAST_RADIUS`] (`28.0 px`) — fair last-gasp hazard clearly warned for almost
    ///   a full second. Harmless after `progress > 0.52`.
    /// - **Damage**: `18` from final detonation blast (non-lethal if player respects the ring).
    /// - **Counter**: Step outside the `28 px` red ring while it melts down, then scoop the
    ///   boss XP and pickup drops from the smoking crater!
    Dying,
}

/// Pure, `Copy` view snapshot of the Robot Lawnmower Boss for zero-allocation drawing.
///
/// Decoupled from simulation types (`Enemy`, `World`, `Player`) so art and gameplay
/// can be developed and tested independently.
#[derive(Clone, Copy, Debug)]
pub struct MowerView {
    /// World-space center of the mower chassis.
    pub pos: Vec2,
    /// Unit forward vector (`+u` = front bumper & lidar eye; `-u` = rear wheels & louvers).
    pub facing: Vec2,
    /// Active boss state in the move set.
    pub state: MowerState,
    /// Normalized progress `0.0..=1.0` within the current `state`.
    pub progress: f32,
    /// Continuous animation clock in seconds (drives wheel treads, spinning blades,
    /// LED pulses, smoke drift, sparks, and phase-3 eye flicker).
    pub anim: f32,
    /// Hit-flash intensity `0.0..=1.0` (decays quickly after taking damage; also spawns
    /// crackling metallic impact sparks around the chassis).
    pub hit_flash: f32,
    /// Remaining health fraction `0.0..=1.0`. Drives 3 visual damage phases:
    /// - `> 0.66`: pristine industrial shell
    /// - `0.33..=0.66`: cracked orange cowl, rear smoke wisp, wider blade gap
    /// - `<= 0.33`: blown-off rear hood exposing hot motor coils & flickering eye
    pub hp_frac: f32,
    /// World-space aim / endpoint for telegraphs (`Scan` lock point, `ChargeWindup` /
    /// `Charge` wall stop, `Ricochet` next wall bounce target, or `FlingBlades` aim point).
    pub target: Vec2,
    /// World-space start point of the current `Charge` or `Ricochet` segment (used to
    /// paint the mown/scorched ground trail strip behind the mower).
    pub trail_start: Vec2,
    /// World-space center of the arena's Charging Dock pad (rendered during `Docking`,
    /// or anytime via [`draw_dock_pad`]).
    pub dock_pos: Vec2,
    /// Completed wall bounces (`0..=4`) during the current [`MowerState::Ricochet`] chain.
    pub bounce_count: u8,
}

impl MowerView {
    /// Convenience constructor for a healthy patrolling mower facing `facing`.
    pub fn patrol(pos: Vec2, facing: Vec2, anim: f32) -> Self {
        Self {
            pos,
            facing,
            state: MowerState::Patrol,
            progress: 0.0,
            anim,
            hit_flash: 0.0,
            hp_frac: 1.0,
            target: pos + facing * 64.0,
            trail_start: pos,
            dock_pos: pos + Vec2::new(40.0, 0.0),
            bounce_count: 0,
        }
    }
}

// ============================================================================
// Palette & Lighting Constants
// ============================================================================

/// Screen-space directional light (top-left), matching `art_rat.rs` & `art_enemy.rs`.
const LIGHT: Vec2 = Vec2 { x: -0.55, y: -0.83 };

// Safety-orange molded ABS shell (Husqvarna/Worx industrial lawnmower silhouette).
const SHELL_SHADOW: Rgb = (118, 38, 16);
const SHELL_MID: Rgb = (224, 88, 22);
const SHELL_HI: Rgb = (255, 176, 74);
const SHELL_SPEC: Rgb = (255, 232, 166);

// Anodized gunmetal-steel lower chassis, trim, and bumper brackets.
const STEEL_DARK: Rgb = (30, 36, 44);
const STEEL_MID: Rgb = (78, 90, 106);
const STEEL_HI: Rgb = (148, 164, 184);
const STEEL_SPEC: Rgb = (212, 226, 240);

// Rubber tires & impact bumper.
const RUBBER_DARK: Rgb = (18, 20, 24);
const RUBBER_MID: Rgb = (44, 48, 56);
const RUBBER_HI: Rgb = (80, 86, 98);

// Hazard decals, brass contacts, emergency STOP button, underbelly PCB.
const HAZARD_YELLOW: Rgb = (246, 200, 44);
const HAZARD_DARK: Rgb = (24, 22, 20);
const DECAL_WHITE: Rgb = (226, 230, 236);
const STOP_DARK: Rgb = (118, 18, 24);
const STOP_MID: Rgb = (226, 36, 44);
const STOP_HI: Rgb = (255, 128, 118);
const BRASS_DARK: Rgb = (128, 88, 30);
const BRASS_HI: Rgb = (244, 206, 98);
const PCB_DARK: Rgb = (18, 62, 42);
const PCB_MID: Rgb = (36, 134, 78);
const PCB_TRACE: Rgb = (112, 248, 162);

// Status LED & VFX colors.
const LED_PATROL: Rgb = (72, 248, 184);
const LED_SCAN: Rgb = (84, 255, 136);
const LED_WARN: Rgb = (255, 188, 52);
const LED_DANGER: Rgb = (255, 52, 48);
const LED_BLADE: Rgb = (160, 242, 255);
const LED_DOCK: Rgb = (86, 224, 255);
const SPARK_HOT: Rgb = (255, 248, 198);
const SPARK_ELEC: Rgb = (140, 236, 255);
const HEAT_GLOW: Rgb = (255, 112, 34);
const SCORCH_DARK: Rgb = (12, 10, 10);
const GRASS_CLIP: Rgb = (96, 196, 62);
const GRASS_BRIGHT: Rgb = (176, 244, 104);

// ============================================================================
// Internal Frame & Math Helpers (Zero Allocation)
// ============================================================================

#[derive(Clone, Copy)]
struct Frame {
    pos: Vec2,
    f: Vec2,
    p: Vec2,
    fl: f32,
    pl: f32,
    hit: f32,
    hp: f32,
    prog: f32,
    anim: f32,
    accent: Rgb,
}

impl Frame {
    fn from_view(view: &MowerView, cam: Vec2, accent: Rgb) -> Self {
        let f = if view.facing.len_sq() > 1e-4 {
            view.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
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
        let anim = if view.anim.is_finite() {
            view.anim
        } else {
            0.0
        };
        Self {
            pos: view.pos - cam,
            f,
            p,
            fl: f.dot(LIGHT),
            pl: p.dot(LIGHT),
            hit,
            hp,
            prog,
            anim,
            accent,
        }
    }

    #[inline]
    fn place(&self, u: f32, v: f32) -> Vec2 {
        self.pos + self.f * u + self.p * v
    }

    #[inline]
    fn px(&self, u: f32, v: f32) -> (i32, i32) {
        let q = self.place(u, v);
        (q.x.round() as i32, q.y.round() as i32)
    }

    #[inline]
    fn local(&self, x: i32, y: i32) -> (f32, f32) {
        let dx = x as f32 - self.pos.x;
        let dy = y as f32 - self.pos.y;
        (dx * self.f.x + dy * self.f.y, dx * self.p.x + dy * self.p.y)
    }

    #[inline]
    fn lit(&self, nu: f32, nv: f32) -> f32 {
        nu * self.fl + nv * self.pl
    }

    #[inline]
    fn bbox(&self, pb: &PixelBuffer, reach: f32) -> Option<(i32, i32, i32, i32)> {
        bbox_around(pb, self.pos, reach)
    }
}

#[inline]
fn bbox_around(pb: &PixelBuffer, center: Vec2, reach: f32) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0 || pb.h == 0 || !center.x.is_finite() || !center.y.is_finite() {
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
fn bbox_segment(
    pb: &PixelBuffer,
    a: Vec2,
    b: Vec2,
    pad: f32,
) -> Option<(i32, i32, i32, i32)> {
    if pb.w == 0
        || pb.h == 0
        || !a.x.is_finite()
        || !a.y.is_finite()
        || !b.x.is_finite()
        || !b.y.is_finite()
        || !pad.is_finite()
    {
        return None;
    }
    let x0 = (a.x.min(b.x) - pad).floor() as i32;
    let x1 = (a.x.max(b.x) + pad).ceil() as i32;
    let y0 = (a.y.min(b.y) - pad).floor() as i32;
    let y1 = (a.y.max(b.y) + pad).ceil() as i32;
    let cx0 = x0.max(0);
    let cx1 = x1.min(pb.w as i32 - 1);
    let cy0 = y0.max(0);
    let cy1 = y1.min(pb.h as i32 - 1);
    if cx0 > cx1 || cy0 > cy1 {
        None
    } else {
        Some((cx0, cx1, cy0, cy1))
    }
}

#[inline]
fn blend_px(pb: &mut PixelBuffer, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 {
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
    if amount <= 0.0 || x < 0 || y < 0 {
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
    let l = lum.clamp(0.0, 1.15);
    if l < 0.62 {
        mix(shadow, mid, l / 0.62)
    } else {
        mix(mid, hi, (l - 0.62) / 0.53)
    }
}

/// Crisp metallic hit flash: snaps hot white-gold on impact while preserving
/// panel seams, decals, and silhouette form contrast.
#[inline]
fn apply_hit(c: Rgb, hit: f32) -> Rgb {
    if hit <= 0.0 {
        return c;
    }
    let flash = if hit > 0.55 {
        (255, 244, 206)
    } else {
        (255, 214, 146)
    };
    mix(c, flash, (0.20 + hit * 0.42).min(0.62))
}

/// Deterministic fast hash `0.0..1.0` for pixel grit, cracks, and sparks.
#[inline]
fn hash01(ix: i32, iy: i32) -> f32 {
    let mut n = (ix.wrapping_mul(374_761_393))
        .wrapping_add(iy.wrapping_mul(668_265_263)) as u32;
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    ((n ^ (n >> 16)) & 0x00ff_ffff) as f32 / 16_777_215.0
}

/// Signed distance to a rounded axis-aligned box centered at `(cu, cv)` with
/// half-extents `(hu, hv)` and corner radius `r`.
#[inline]
fn sd_rbox(u: f32, v: f32, cu: f32, cv: f32, hu: f32, hv: f32, r: f32) -> f32 {
    let qu = (u - cu).abs() - (hu - r);
    let qv = (v - cv).abs() - (hv - r);
    let out = (qu.max(0.0).powi(2) + qv.max(0.0).powi(2)).sqrt();
    let inn = qu.max(qv).min(0.0);
    out + inn - r
}

#[inline]
fn sd_circle(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> f32 {
    ((u - cu).powi(2) + (v - cv).powi(2)).sqrt() - r
}

/// Signed distance to the sculpted tapered mower main hull (excluding front bumper
/// and protruding rear wheels).
#[inline]
fn sd_mower_hull(u: f32, v: f32) -> f32 {
    // Smoothly tapers from wide rear (half-width 10.6 at u=-11) to sleeker nose
    // (half-width 8.4 at u=+10.5), with crisp rounded corners.
    let t = ((u + 12.0) / 23.0).clamp(0.0, 1.0);
    let half_w = 10.6 - 2.2 * t * t;
    let qu = (u + 0.6).abs() - 9.0;
    let qv = v.abs() - (half_w - 2.4);
    let out = (qu.max(0.0).powi(2) + qv.max(0.0).powi(2)).sqrt();
    let inn = qu.max(qv).min(0.0);
    out + inn - 2.4
}

/// Signed distance to the curved wrap-around front impact bumper (`u ≈ +10.2..+14.0`).
#[inline]
fn sd_front_bumper(u: f32, v: f32, push_back: f32) -> f32 {
    // Arc of radius ~12.4 centered at u = +1.8 - push_back, cropped to the nose band.
    let cu = 1.8 - push_back;
    let r = ((u - cu).powi(2) + (v * 1.12).powi(2)).sqrt();
    let d_arc = (r - 11.4).abs() - 1.15;
    let d_front = (9.6 - push_back) - u;
    let d_side = v.abs() - 9.1;
    d_arc.max(d_front).max(d_side)
}

/// Signed distance to the two chunky rear drive wheels (`u ∈ [-9.6, -2.0], |v| ∈ [9.2, 12.8]`).
#[inline]
fn sd_rear_wheels(u: f32, v: f32) -> f32 {
    sd_rbox(u, v.abs(), -5.8, 11.0, 3.8, 1.8, 0.7)
}

/// Signed distance to the two small front swivel casters (tucked near front flanks).
#[inline]
fn sd_front_casters(u: f32, v: f32) -> f32 {
    sd_rbox(u, v.abs(), 6.8, 9.2, 1.8, 1.1, 0.5)
}

/// Draw a blended line clipped naturally by `blend_px` (with dash support).
#[allow(clippy::too_many_arguments)]
fn draw_line_styled(
    pb: &mut PixelBuffer,
    a: Vec2,
    b: Vec2,
    c: Rgb,
    alpha: f32,
    dash_on: i32,
    dash_period: i32,
    phase: i32,
    additive: bool,
) {
    if !a.x.is_finite()
        || !a.y.is_finite()
        || !b.x.is_finite()
        || !b.y.is_finite()
        || pb.w == 0
        || pb.h == 0
        || bbox_segment(pb, a, b, 1.0).is_none()
    {
        return;
    }
    let ax = a.x.clamp(-1e6, 1e6).round() as i64;
    let ay = a.y.clamp(-1e6, 1e6).round() as i64;
    let bx = b.x.clamp(-1e6, 1e6).round() as i64;
    let by = b.y.clamp(-1e6, 1e6).round() as i64;
    let dx = (bx - ax).abs();
    let dy = -(by - ay).abs();
    let sx: i64 = if ax < bx { 1 } else { -1 };
    let sy: i64 = if ay < by { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (ax, ay);
    let mut step = 0i32;
    let max_steps = (dx - dy + 2).min(640) as i32;
    while step <= max_steps {
        let on = dash_period <= 0 || ((step + phase).rem_euclid(dash_period) < dash_on);
        if on && x >= 0 && y >= 0 && x < pb.w as i64 && y < pb.h as i64 {
            if additive {
                pb.plot_add(x as i32, y as i32, scale(c, alpha));
            } else {
                blend_px(pb, x as i32, y as i32, c, alpha);
            }
        }
        if x == bx && y == by {
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
        step += 1;
    }
}

/// Soft radial glow puff (used for additive fireball cores and hot blooms).
fn draw_puff(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    core: Rgb,
    edge: Rgb,
    alpha: f32,
    additive: bool,
) {
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, radius + 1.0) else {
        return;
    };
    let inv_r = 1.0 / radius.max(0.5);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let r = (dx * dx + dy * dy).sqrt() * inv_r;
            if r >= 1.0 {
                continue;
            }
            let falloff = (1.0 - r * r).clamp(0.0, 1.0);
            let col = mix(edge, core, falloff);
            let a = alpha * falloff;
            if additive {
                pb.plot_add(x, y, scale(col, a));
            } else {
                blend_px(pb, x, y, col, a);
            }
        }
    }
}

/// Crisp cel-shaded pixel-art smoke/steam puff with top-left light and dark rim,
/// matching Rat Run's hard-edged pixel aesthetic.
fn draw_smoke_puff(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    shadow: Rgb,
    mid: Rgb,
    hi: Rgb,
    alpha: f32,
) {
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, radius + 1.5) else {
        return;
    };
    let inv_r = 1.0 / radius.max(0.6);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let r = (dx * dx + dy * dy).sqrt() * inv_r;
            if r > 1.18 {
                continue;
            }
            if r > 0.92 {
                let rim_a = ((1.18 - r) / 0.26).clamp(0.0, 1.0) * 0.55 * alpha;
                blend_px(pb, x, y, (14, 16, 20), rim_a);
            } else {
                let lit = (dx * LIGHT.x + dy * LIGHT.y) * inv_r;
                let col = shade3(shadow, mid, hi, 0.48 + 0.52 * lit);
                blend_px(pb, x, y, col, alpha);
            }
        }
    }
}

/// Crisp circle ring on ground or air (optionally dashed and/or additive).
#[allow(clippy::too_many_arguments)]
fn draw_ring_styled(
    pb: &mut PixelBuffer,
    center: Vec2,
    radius: f32,
    col: Rgb,
    alpha: f32,
    dash_mod: i32,
    phase: f32,
    additive: bool,
) {
    if radius < 1.0 || pb.w == 0 || pb.h == 0 {
        return;
    }
    if bbox_around(pb, center, radius + 2.0).is_none() {
        return;
    }
    let steps = ((radius * 6.5).round() as i32).clamp(16, 260);
    let mut last = (i32::MIN, i32::MIN);
    for i in 0..steps {
        if dash_mod > 0 && ((i / dash_mod) % 2 == 1) {
            continue;
        }
        let a = phase + i as f32 / steps as f32 * TAU;
        let x = (center.x + a.cos() * radius).round() as i32;
        let y = (center.y + a.sin() * radius).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        if additive {
            pb.plot_add(x, y, scale(col, alpha));
        } else {
            blend_px(pb, x, y, col, alpha);
        }
    }
}

// ============================================================================
// Public Drawing API — Ground Layer
// ============================================================================

/// **Ground Layer**: Draw the arena's industrial Charging Dock floor pad at `dock_pos`.
///
/// Implementers may call this every frame during the mower boss fight so the dock
/// remains a readable arena landmark, or let [`draw_mower_ground`] draw it automatically
/// whenever `view.state == MowerState::Docking`.
///
/// - `charging`: `0.0` when idle standby, `0.0..=1.0` when actively recharging the mower.
pub fn draw_dock_pad(
    pb: &mut PixelBuffer,
    dock_pos: Vec2,
    anim: f32,
    charging: f32,
    cam: Vec2,
    accent: Rgb,
) {
    let c = dock_pos - cam;
    let hw = DOCK_HALF_SIZE.x;
    let hh = DOCK_HALF_SIZE.y;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, c, hw.max(hh) + 3.0) else {
        return;
    };
    let pulse = 0.5 + 0.5 * (anim * 5.0).sin();
    let chg = charging.clamp(0.0, 1.0);
    let led_col = if chg > 0.05 {
        mix(LED_DOCK, (255, 245, 160), 0.35 * pulse)
    } else {
        mix(accent, LED_PATROL, 0.55)
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let lx = x as f32 - c.x;
            let ly = y as f32 - c.y;
            let d = sd_rbox(lx, ly, 0.0, 0.0, hw, hh, 2.5);
            if d > 1.8 {
                continue;
            }
            if d > 0.0 {
                // Grounded dark rim + subtle drop shadow on bottom-right.
                let sh = if lx > 0.0 && ly > 0.0 { 0.45 } else { 0.62 };
                darken_px(pb, x, y, sh * (1.0 - d / 1.8));
                continue;
            }

            // Outer bevel frame with hazard yellow/black stripes along top & bottom edges.
            let in_border = d > -2.3;
            let mut col = if in_border {
                let is_side_strip = ly.abs() > hh - 2.6;
                if is_side_strip {
                    let stripe = ((lx + ly) * 0.55).floor() as i32;
                    if stripe.rem_euclid(2) == 0 {
                        mix(HAZARD_YELLOW, (175, 138, 28), 0.35)
                    } else {
                        HAZARD_DARK
                    }
                } else {
                    let lit = (lx * LIGHT.x + ly * LIGHT.y) / 18.0;
                    shade3(STEEL_DARK, STEEL_MID, STEEL_HI, 0.48 + 0.45 * lit)
                }
            } else {
                // Dark ribbed steel docking bay deck.
                let rib = ((ly + 15.0) * 0.5).floor() as i32 % 2 == 0;
                let base = if rib { (26, 32, 38) } else { (34, 42, 50) };
                // Inner recessed bay bevel.
                if lx < -hw + 3.5 || ly < -hh + 3.5 {
                    scale(base, 0.72)
                } else if lx > hw - 3.5 || ly > hh - 3.5 {
                    mix(base, STEEL_MID, 0.28)
                } else {
                    base
                }
            };

            // Back charging tower block on the left (`lx ∈ [-13.5, -8.5]`).
            if (-13.5..=-8.0).contains(&lx) && ly.abs() <= 10.5 {
                let lit = ((lx + 11.0) * LIGHT.x + ly * LIGHT.y) / 10.0;
                col = shade3((36, 44, 54), (82, 96, 114), (156, 174, 196), 0.52 + 0.4 * lit);
                // Dual glowing brass contact prongs projecting right (`lx ∈ [-8.8, -6.2]`).
                if lx > -9.2 && (ly.abs() - 4.0).abs() < 1.35 {
                    col = mix(BRASS_DARK, BRASS_HI, 0.55 + 0.45 * chg);
                }
                // Status LED bar along the back spine.
                if lx < -11.4 && ly.abs() < 6.0 {
                    let glow = if chg > 0.05 { 0.72 + 0.28 * pulse } else { 0.45 };
                    col = mix(STEEL_DARK, led_col, glow);
                }
            } else if (-8.0..=-4.5).contains(&lx) && (ly.abs() - 4.0).abs() < 1.25 {
                // Protruding brass contact pins.
                col = mix(BRASS_DARK, BRASS_HI, 0.5 + 0.5 * pulse * chg);
            }

            // Center induction coil ring on the deck.
            let rc = ((lx - 2.0).powi(2) + ly * ly).sqrt();
            if (rc - 6.2).abs() < 0.85 {
                let ring_a = 0.28 + 0.65 * chg * (0.65 + 0.35 * pulse);
                col = mix(col, led_col, ring_a);
            } else if rc < 2.2 && chg > 0.05 {
                col = mix(col, SPARK_HOT, 0.45 * chg * pulse);
            }

            // Guide alignment chevron arrows pointing left into the prongs.
            if lx > 1.0 && lx < 11.0 && ly.abs() < 5.5 {
                let chev = ((lx - ly.abs() * 0.9) + anim * if chg > 0.05 { 8.0 } else { 2.0 })
                    .rem_euclid(5.0);
                if chev < 1.1 {
                    col = mix(col, led_col, 0.24 + 0.46 * chg);
                }
            }

            pb.plot(x, y, col);
        }
    }

    // Active charging glow ring around the pad.
    if chg > 0.05 {
        let r_ring = 18.0 + (anim * 10.0).rem_euclid(6.0);
        let fade = 1.0 - ((r_ring - 18.0) / 6.0);
        draw_ring_styled(pb, c, r_ring, LED_DOCK, 0.55 * chg * fade, 0, anim, true);
    }
}

/// **Ground Layer**: Draw ground-level telegraphs, trails, dock tether, and drop shadow.
///
/// Must be called **before** [`draw_mower`], `draw_enemy`, and `draw_rat` so the white
/// player rat and the mower always sit crisply on top of telegraphs and tracks.
pub fn draw_mower_ground(pb: &mut PixelBuffer, view: &MowerView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let fr = Frame::from_view(view, cam, accent);

    // 1) State-specific floor features & telegraphs.
    match view.state {
        MowerState::Patrol => {}
        MowerState::Scan => draw_scan_cone_ground(pb, &fr, view.target - cam),
        MowerState::ChargeWindup => draw_charge_windup_ground(pb, &fr, view.target - cam),
        MowerState::Charge => {
            draw_mown_trail_ground(pb, view.trail_start - cam, fr.pos, fr.anim, false);
            draw_charge_forward_guide(pb, &fr, view.target - cam);
        }
        MowerState::Ricochet => {
            draw_mown_trail_ground(pb, view.trail_start - cam, fr.pos, fr.anim, true);
            draw_ricochet_telegraph_ground(
                pb,
                &fr,
                view.trail_start - cam,
                view.target - cam,
                view.bounce_count,
            );
        }
        MowerState::FlingBlades => draw_fling_telegraph_ground(pb, &fr),
        MowerState::Overheat => draw_vulnerable_ground_ring(pb, &fr, LED_DANGER, 15.5),
        MowerState::Flipped => {
            draw_vulnerable_ground_ring(pb, &fr, PCB_TRACE, 14.5);
            // Self-righting slam warning ring (`r = 16 px`, matches `FLIP_SLAM_RADIUS`) during the
            // final 15% roll-back window so players see the slam coming.
            if fr.prog >= 0.85 {
                let t = ((fr.prog - 0.85) / 0.15).clamp(0.0, 1.0);
                let slam_col = mix(LED_WARN, LED_DANGER, t);
                draw_ring_styled(pb, fr.pos, 16.0, slam_col, 0.45 + 0.45 * t, 2, fr.anim * 5.0, true);
            }
        }
        MowerState::Docking => {
            let chg = ((fr.prog - 0.25) / 0.75).clamp(0.0, 1.0);
            draw_dock_pad(pb, view.dock_pos, fr.anim, chg.max(0.25), cam, accent);
            draw_docking_tether_ground(pb, &fr, view.dock_pos - cam, chg);
            // Defensive surge annulus telegraph (`r ∈ [8, 26] px` around `dock_pos`) before release.
            if fr.prog >= 0.54 {
                let t = ((fr.prog - 0.54) / 0.46).clamp(0.0, 1.0);
                let dock_s = view.dock_pos - cam;
                let surge_col = mix(LED_DOCK, LED_DANGER, t);
                draw_ring_styled(pb, dock_s, 26.0, surge_col, 0.40 + 0.48 * t, 2, fr.anim * 4.5, true);
                draw_ring_styled(pb, dock_s, 8.0, surge_col, 0.35 + 0.35 * t, 0, 0.0, true);
            }
        }
        MowerState::Dying => draw_dying_ground(pb, &fr),
    }

    // 2) Grounded down-right drop shadow (omitted once chassis explodes away).
    if view.state != MowerState::Dying || fr.prog < 0.52 {
        draw_mower_shadow(pb, &fr, view.state);
    }
}

fn draw_mower_shadow(pb: &mut PixelBuffer, fr: &Frame, state: MowerState) {
    let sh_ofs = match state {
        MowerState::Flipped => Vec2::new(1.5, 2.2),
        MowerState::Charge | MowerState::Ricochet => Vec2::new(2.5, 3.0),
        _ => Vec2::new(2.0, 2.5),
    };
    let sc = fr.pos + sh_ofs;
    let Some((x0, x1, y0, y1)) = bbox_around(pb, sc, 16.5) else {
        return;
    };
    let fade = if state == MowerState::Dying {
        (1.0 - (fr.prog / 0.52).clamp(0.0, 1.0)).max(0.0)
    } else {
        1.0
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - sc.x;
            let dy = y as f32 - sc.y;
            let u = dx * fr.f.x + dy * fr.f.y;
            let v = dx * fr.p.x + dy * fr.p.y;
            let d = sd_mower_hull(u, v)
                .min(sd_rear_wheels(u, v))
                .min(sd_front_bumper(u, v, 0.0));
            if d < 1.2 {
                let a = (0.44 * (1.0 - (d.max(0.0) / 1.2))).clamp(0.0, 0.44) * fade;
                darken_px(pb, x, y, a);
            }
        }
    }
}

/// Sweeping lidar cone (`progress 0.0..0.70`) snapping to lock-on reticle (`0.70..1.0`).
fn draw_scan_cone_ground(pb: &mut PixelBuffer, fr: &Frame, target: Vec2) {
    let locked = fr.prog >= 0.70;
    let lock_t = ((fr.prog - 0.70) / 0.30).clamp(0.0, 1.0);
    let origin = fr.place(10.5, 0.0);
    // Keep the drawn cone ending at `SCAN_CONE_RANGE` from `fr.pos` (matching the sim check).
    let cone_reach = (SCAN_CONE_RANGE - 10.5).max(12.0);

    let (cone_dir, half_ang, range) = if locked {
        let to = target - origin;
        let dir = if to.len_sq() > 1.0 {
            to.normalized()
        } else {
            fr.f
        };
        let r = to.len().clamp(4.0, cone_reach);
        // Cone narrows sharply as it locks onto the target!
        let ha = SCAN_CONE_HALF_ANGLE * (1.0 - 0.62 * lock_t);
        (dir, ha, r)
    } else {
        let sweep_t = fr.prog / 0.70;
        let angle_ofs = (sweep_t * PI * 2.0 - FRAC_PI_2).sin() * 0.58;
        (fr.f.rotate(angle_ofs), SCAN_CONE_HALF_ANGLE, cone_reach)
    };

    let cone_col = if locked {
        mix(LED_WARN, LED_DANGER, lock_t)
    } else {
        LED_SCAN
    };

    // Tight bounding box covering only the cone wedge.
    let tip_c = origin + cone_dir * range;
    let tip_l = origin + cone_dir.rotate(-half_ang) * range;
    let tip_r = origin + cone_dir.rotate(half_ang) * range;
    let min_x = origin.x.min(tip_c.x).min(tip_l.x).min(tip_r.x).floor() as i32;
    let max_x = origin.x.max(tip_c.x).max(tip_l.x).max(tip_r.x).ceil() as i32;
    let min_y = origin.y.min(tip_c.y).min(tip_l.y).min(tip_r.y).floor() as i32;
    let max_y = origin.y.max(tip_c.y).max(tip_l.y).max(tip_r.y).ceil() as i32;
    let x0 = min_x.max(0);
    let x1 = max_x.min(pb.w as i32 - 1);
    let y0 = min_y.max(0);
    let y1 = max_y.min(pb.h as i32 - 1);

    let cos_lim = half_ang.cos();
    let wave = (fr.anim * 28.0).rem_euclid(18.0);

    if x0 <= x1 && y0 <= y1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - origin.x;
                let dy = y as f32 - origin.y;
                let d2 = dx * dx + dy * dy;
                if d2 < 9.0 || d2 > range * range {
                    continue;
                }
                let dist = d2.sqrt();
                let dot = (dx * cone_dir.x + dy * cone_dir.y) / dist;
                if dot < cos_lim {
                    continue;
                }
                let edge_dist = (dot - cos_lim) * dist;
                let fade = (1.0 - dist / range).clamp(0.15, 1.0);
                let is_edge = edge_dist < 1.15 || (range - dist) < 1.2;
                let is_pulse = ((dist - wave).rem_euclid(18.0)) < 1.25;
                let alpha = if is_edge {
                    0.52 * fade
                } else if is_pulse {
                    0.34 * fade
                } else {
                    (0.15 + 0.08 * lock_t) * fade
                };
                blend_px(pb, x, y, cone_col, alpha);
            }
        }
    }

    // Crisp centerline laser & target lock-on corner brackets.
    draw_line_styled(
        pb,
        origin,
        origin + cone_dir * range,
        mix(cone_col, SPARK_HOT, if locked { 0.45 } else { 0.15 }),
        if locked { 0.72 } else { 0.35 },
        if locked { 0 } else { 3 },
        if locked { 0 } else { 6 },
        (fr.anim * 16.0) as i32,
        true,
    );

    if locked {
        let r = (7.5 - 3.0 * lock_t).round() as i32;
        let tx = target.x.round() as i32;
        let ty = target.y.round() as i32;
        let brk = mix(LED_WARN, LED_DANGER, lock_t);
        for &(sx, sy) in &[(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let cx = tx + sx * r;
            let cy = ty + sy * r;
            pb.plot(cx, cy, brk);
            blend_px(pb, cx - sx, cy, brk, 0.85);
            blend_px(pb, cx, cy - sy, brk, 0.85);
        }
        pb.plot_add(tx, ty, scale(SPARK_HOT, 0.7 * lock_t));
    }
}

/// Crimson laser sightline + exact `±CHARGE_CORRIDOR_HALF_WIDTH` warning rails + chevrons.
fn draw_charge_windup_ground(pb: &mut PixelBuffer, fr: &Frame, target: Vec2) {
    let origin = fr.place(11.5, 0.0);
    let to = target - fr.pos;
    let dist = to.len().clamp(1.0, 150.0);
    let dir = if to.len_sq() > 1.0 {
        to.normalized()
    } else {
        fr.f
    };
    let perp = dir.perp();
    let end = fr.pos + dir * dist;

    let pulse = 0.55 + 0.45 * (fr.anim * 18.0).sin();
    let heat = fr.prog;
    let danger_fill = mix((160, 24, 24), LED_DANGER, heat);

    // Subtle red hazard tint across the exact charge corridor.
    if let Some((x0, x1, y0, y1)) = bbox_segment(pb, fr.pos, end, CHARGE_CORRIDOR_HALF_WIDTH + 2.0) {
        let chevron_scroll = fr.anim * 34.0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let rx = x as f32 - fr.pos.x;
                let ry = y as f32 - fr.pos.y;
                let along = rx * dir.x + ry * dir.y;
                if along < 8.0 || along > dist {
                    continue;
                }
                let across = (rx * perp.x + ry * perp.y).abs();
                if across > CHARGE_CORRIDOR_HALF_WIDTH {
                    continue;
                }
                // Animated warning chevrons marching toward target.
                let chev = (along - across * 0.75 - chevron_scroll).rem_euclid(11.0);
                let is_chev = chev < 1.35 && across < CHARGE_CORRIDOR_HALF_WIDTH - 2.0;
                let a = if is_chev {
                    0.26 + 0.24 * heat
                } else {
                    0.07 + 0.09 * heat
                };
                blend_px(pb, x, y, danger_fill, a);
            }
        }
    }

    // Crisp dashed corridor edge rails at ±CHARGE_CORRIDOR_HALF_WIDTH.
    let rail_col = mix(LED_WARN, LED_DANGER, 0.4 + 0.6 * heat);
    let phase = -(fr.anim * 24.0) as i32;
    for side in [-1.0f32, 1.0] {
        let off = perp * (side * CHARGE_CORRIDOR_HALF_WIDTH);
        draw_line_styled(pb, fr.pos + dir * 10.0 + off, end + off, rail_col, 0.68, 4, 7, phase, false);
    }

    // Blazing central laser sightline from cyclops nose eye to target.
    let laser_core = mix(LED_DANGER, SPARK_HOT, (heat * 0.85).clamp(0.0, 0.85));
    let dashed = heat < 0.45;
    draw_line_styled(
        pb,
        origin,
        end,
        laser_core,
        0.72 + 0.28 * pulse,
        if dashed { 4 } else { 0 },
        if dashed { 6 } else { 0 },
        phase,
        true,
    );

    // Impact X crosshair at `target`.
    let ex = end.x.round() as i32;
    let ey = end.y.round() as i32;
    for d in -3..=3 {
        blend_px(pb, ex + d, ey + d, laser_core, 0.85);
        blend_px(pb, ex + d, ey - d, laser_core, 0.85);
    }
    draw_ring_styled(pb, end, 4.5 + 2.5 * (1.0 - heat), LED_DANGER, 0.7, 0, 0.0, true);
}

/// Mown & scorched turf/tire skid strip left behind along `start -> end`.
fn draw_mown_trail_ground(pb: &mut PixelBuffer, start: Vec2, end: Vec2, anim: f32, scorched: bool) {
    let seg = end - start;
    let len = seg.len();
    if len < 2.0 {
        return;
    }
    let dir = seg * (1.0 / len);
    let perp = dir.perp();
    let half_w = 11.4;
    let Some((x0, x1, y0, y1)) = bbox_segment(pb, start, end, half_w + 1.5) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let rx = x as f32 - start.x;
            let ry = y as f32 - start.y;
            let along = rx * dir.x + ry * dir.y;
            if along < -2.0 || along > len - 4.0 {
                continue;
            }
            let across = (rx * perp.x + ry * perp.y).abs();
            if across > half_w {
                continue;
            }
            let recency = (along / len).clamp(0.0, 1.0); // 1 near mower, 0 near start
            let h = hash01(x, y);

            // Outer ash-singe curb highlighting the burnt tire skid edge so it reads on dark floors.
            if across > 10.2 && across <= 11.4 {
                let singe = if scorched { (96, 62, 42) } else { (64, 78, 52) };
                blend_px(pb, x, y, singe, (0.34 + 0.22 * recency) * (0.65 + 0.35 * h));
            } else if (7.8..=10.2).contains(&across) {
                // Twin burned rubber rear-wheel tread marks!
                let tread = ((along * 0.65).floor() as i32).rem_euclid(2) == 0;
                if tread {
                    blend_px(pb, x, y, SCORCH_DARK, 0.72 * (0.45 + 0.55 * recency));
                } else {
                    blend_px(pb, x, y, (52, 42, 34), 0.38 * (0.45 + 0.55 * recency));
                }
            } else if across < 7.8 {
                // Central mowed/scorched blade swath: diagonal rotary-cut stripe bands!
                let swath_band = ((along * 0.45 + across * 0.35).floor() as i32).rem_euclid(2) == 0;
                let base_col = if scorched {
                    if swath_band { (54, 24, 14) } else { (24, 14, 12) }
                } else if swath_band {
                    (46, 74, 34)
                } else {
                    (22, 34, 18)
                };
                let swath_a = (0.38 + 0.16 * h) * (0.40 + 0.60 * recency);
                blend_px(pb, x, y, base_col, swath_a);

                // Central glowing blade-friction scorch streaks + torn grass clippings!
                let scrape = (across - 2.6).abs() < 0.75 || across < 0.85;
                if scrape && recency > 0.35 {
                    let glow_col = if scorched || recency > 0.72 {
                        HEAT_GLOW
                    } else {
                        mix(GRASS_CLIP, HEAT_GLOW, 0.45)
                    };
                    let flicker = 0.55 + 0.45 * (anim * 15.0 + along * 0.4).sin();
                    blend_px(pb, x, y, glow_col, 0.48 * recency * flicker);
                } else if h > 0.86 {
                    let fleck = if scorched {
                        HEAT_GLOW
                    } else if h > 0.93 {
                        GRASS_BRIGHT
                    } else {
                        GRASS_CLIP
                    };
                    blend_px(pb, x, y, fleck, 0.62 * (0.35 + 0.65 * recency));
                }
            }
        }
    }
}

fn draw_charge_forward_guide(pb: &mut PixelBuffer, fr: &Frame, target: Vec2) {
    let origin = fr.place(13.0, 0.0);
    if origin.dist(target) < 8.0 {
        return;
    }
    draw_line_styled(
        pb,
        origin,
        target,
        LED_DANGER,
        0.38,
        3,
        6,
        -(fr.anim * 28.0) as i32,
        true,
    );
}

fn draw_ricochet_telegraph_ground(
    pb: &mut PixelBuffer,
    fr: &Frame,
    bounce_origin: Vec2,
    next_target: Vec2,
    bounce_count: u8,
) {
    // Wall-impact shock ring fading at `bounce_origin`.
    let ring_t = fr.prog;
    if ring_t < 0.75 {
        let r = 4.0 + ring_t * 14.0;
        let fade = 1.0 - ring_t / 0.75;
        let col = if bounce_count >= 2 { LED_DANGER } else { LED_WARN };
        draw_ring_styled(pb, bounce_origin, r, col, 0.75 * fade, 0, 0.0, true);
    }

    // Dashed amber-red bank-shot preview line + `±CHARGE_CORRIDOR_HALF_WIDTH` corridor rails
    // during the bounce pause so the full rebound sweep width reads clearly.
    let origin = fr.place(12.0, 0.0);
    let col = if bounce_count >= 2 { LED_DANGER } else { LED_WARN };
    let to = next_target - fr.pos;
    let dir = if to.len_sq() > 1.0 {
        to.normalized()
    } else {
        fr.f
    };
    let perp = dir.perp();
    let phase = -(fr.anim * 26.0) as i32;
    if fr.prog < 0.45 {
        let rail_a = 0.58 * (1.0 - fr.prog / 0.45 * 0.35);
        for side in [-1.0f32, 1.0] {
            let off = perp * (side * CHARGE_CORRIDOR_HALF_WIDTH);
            draw_line_styled(
                pb,
                fr.pos + dir * 10.0 + off,
                next_target + off,
                col,
                rail_a,
                4,
                7,
                phase,
                false,
            );
        }
    }
    draw_line_styled(
        pb,
        origin,
        next_target,
        col,
        0.65,
        4,
        7,
        phase,
        true,
    );
    // Target rebound marker.
    draw_ring_styled(pb, next_target, 4.0, col, 0.65, 2, fr.anim * 4.0, true);
}

/// Radial ejection sector spokes (`progress < 0.35`) and flying disc/shrapnel preview.
fn draw_fling_telegraph_ground(pb: &mut PixelBuffer, fr: &Frame) {
    let spokes = if fr.hp <= 0.66 { 8 } else { 6 };
    let base_ang = fr.f.angle() + if fr.prog >= 0.35 { (fr.prog - 0.35) * 1.1 } else { 0.0 };

    if fr.prog < 0.42 {
        let t = (fr.prog / 0.35).clamp(0.0, 1.0);
        let warn_r = 16.0 + 7.5 * t;
        let spoke_col = mix(LED_BLADE, LED_WARN, t * 0.65);
        draw_ring_styled(pb, fr.pos, warn_r, spoke_col, 0.42 + 0.32 * t, 2, fr.anim * 3.0, true);

        for k in 0..spokes {
            let a = base_ang + k as f32 / spokes as f32 * TAU;
            let dir = Vec2::from_angle(a);
            let p0 = fr.pos + dir * 13.0;
            let p1 = fr.pos + dir * (22.0 + 18.0 * t);
            draw_line_styled(
                pb,
                p0,
                p1,
                spoke_col,
                0.55 + 0.35 * t,
                3,
                5,
                -(fr.anim * 22.0) as i32,
                true,
            );
        }
    } else {
        // Active volley ring & ground danger aura (`15.5 px` close-range deck).
        let fire_t = ((fr.prog - 0.35) / 0.55).clamp(0.0, 1.0);
        draw_ring_styled(pb, fr.pos, 15.5, LED_BLADE, 0.65 * (1.0 - 0.3 * fire_t), 0, 0.0, true);
        // Subtle directional streak guides behind launched discs.
        for k in 0..spokes {
            let a = base_ang + k as f32 / spokes as f32 * TAU;
            let dir = Vec2::from_angle(a);
            let d = 14.0 + fire_t * 20.0;
            let p0 = fr.pos + dir * (d - 6.0).max(12.0);
            let p1 = fr.pos + dir * d;
            draw_line_styled(pb, p0, p1, GRASS_CLIP, 0.45 * (1.0 - fire_t * 0.6), 0, 0, 0, true);
        }
    }
}

/// Subtle pulsing vulnerable halo under `Overheat` or `Flipped` so players spot punish windows.
fn draw_vulnerable_ground_ring(pb: &mut PixelBuffer, fr: &Frame, col: Rgb, base_r: f32) {
    let pulse = 0.5 + 0.5 * (fr.anim * 7.5).sin();
    let r = base_r + 1.5 * pulse;
    draw_ring_styled(pb, fr.pos, r, col, 0.34 + 0.24 * pulse, 3, fr.anim * 1.4, true);
}

/// Pulsing induction energy arcs between Charging Dock and mower during `Docking`.
fn draw_docking_tether_ground(pb: &mut PixelBuffer, fr: &Frame, dock: Vec2, chg: f32) {
    let pulse = 0.5 + 0.5 * (fr.anim * 9.0).sin();
    let prong_left = dock + Vec2::new(-6.0, -4.0);
    let prong_right = dock + Vec2::new(-6.0, 4.0);
    let rear_l = fr.place(-12.5, -3.2);
    let rear_r = fr.place(-12.5, 3.2);
    let arc_col = mix(LED_DOCK, BRASS_HI, pulse * 0.45);

    if fr.pos.dist(dock) > 2.0 {
        draw_line_styled(pb, prong_left, rear_l, arc_col, 0.85, 0, 0, 0, true);
        draw_line_styled(pb, prong_right, rear_r, arc_col, 0.85, 0, 0, 0, true);
    }
    // Crisp crackling induction sparks at the contact prongs!
    for &(pt, side) in &[(rear_l, -1.0f32), (rear_r, 1.0)] {
        let p = (pt.x.round() as i32, pt.y.round() as i32);
        pb.plot_add(p.0, p.1, SPARK_HOT);
        let tip = pt + Vec2::new(-2.0, side * (2.5 + 1.5 * pulse));
        draw_line_styled(pb, pt, tip, LED_DOCK, 0.75 * chg.max(0.4), 0, 0, 0, true);
    }
}

/// Expanding blast warning ring (`0.0..0.44`) transitioning into a charred scorch crater (`0.42..1.0`).
fn draw_dying_ground(pb: &mut PixelBuffer, fr: &Frame) {
    if fr.prog < 0.46 {
        let t = (fr.prog / 0.42).clamp(0.0, 1.0);
        let r = 10.0 + (DEATH_BLAST_RADIUS - 10.0) * t;
        let pulse = 0.5 + 0.5 * (fr.anim * 24.0).sin();
        let col = mix(LED_WARN, LED_DANGER, t);
        draw_ring_styled(pb, fr.pos, r, col, 0.55 + 0.35 * pulse, 2, fr.anim * 4.0, true);
        if t > 0.55 {
            draw_ring_styled(pb, fr.pos, DEATH_BLAST_RADIUS, LED_DANGER, 0.45 * pulse, 3, 0.0, true);
        }
    }
    if fr.prog >= 0.40 {
        let ct = ((fr.prog - 0.40) / 0.25).clamp(0.0, 1.0);
        let fade = 1.0 - ((fr.prog - 0.78) / 0.22).clamp(0.0, 1.0) * 0.25;
        let crater_r = 16.5 * (0.55 + 0.45 * ct);
        if let Some((x0, x1, y0, y1)) = bbox_around(pb, fr.pos, crater_r + 2.0) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dx = x as f32 - fr.pos.x;
                    let dy = y as f32 - fr.pos.y;
                    let r = (dx * dx + dy * dy).sqrt();
                    let jitter = (hash01(x, y) - 0.5) * 3.0;
                    if r + jitter > crater_r {
                        continue;
                    }
                    let rn = (r / crater_r).clamp(0.0, 1.0);
                    let dark_a = (0.72 * (1.0 - rn * 0.7)) * ct * fade;
                    blend_px(pb, x, y, SCORCH_DARK, dark_a);
                    // Glowing hot ember rim inside crater.
                    if rn > 0.42 && rn < 0.78 && hash01(x + 17, y - 9) > 0.78 {
                        let glow = (1.0 - fr.prog).clamp(0.15, 1.0);
                        pb.plot_add(x, y, scale(HEAT_GLOW, 0.65 * glow));
                    }
                }
            }
        }
    }
}

// ============================================================================
// Public Drawing API — Entity Layer
// ============================================================================

/// **Entity Layer**: Draw the Robot Lawnmower Boss body, spinning blades, phase damage,
/// hit sparks, state animations, and radial boss HP arc.
///
/// Call after [`draw_mower_ground`] in the entity rendering pass alongside `draw_enemy`
/// and `draw_rat`.
pub fn draw_mower(pb: &mut PixelBuffer, view: &MowerView, cam: Vec2, accent: Rgb) {
    if pb.w == 0 || pb.h == 0 {
        return;
    }
    let mut fr = Frame::from_view(view, cam, accent);

    // Quick off-screen cull covering body, flung preview discs, explosion debris, and HP arc.
    if bbox_around(pb, fr.pos, DEATH_BLAST_RADIUS + 8.0).is_none() {
        return;
    }

    // Death explosion sequence takes over completely.
    if view.state == MowerState::Dying {
        draw_dying_sequence(pb, &fr);
        return;
    }

    // Subtle mechanical vibration / recoil offset during revving, overheat, or meltdown.
    let jitter = match view.state {
        MowerState::ChargeWindup => {
            let amp = 0.25 + 0.75 * fr.prog;
            let back = -0.9 * fr.prog; // squats back onto rear shocks while revving
            fr.f * back + fr.p * ((fr.anim * 46.0).sin() * amp)
        }
        MowerState::Overheat => fr.p * ((fr.anim * 32.0).sin() * 0.55),
        MowerState::Flipped => {
            // Helpless turtle rocking side-to-side while belly-up!
            let rock = (fr.anim * 7.5).sin() * 0.85;
            fr.p * rock
        }
        _ => Vec2::ZERO,
    };
    fr.pos += jitter;

    // 1) Flipped turtle state has its own 2.5D roll + exposed underside renderer!
    if view.state == MowerState::Flipped {
        draw_flipped_mower(pb, &fr);
        draw_overheat_smoke(pb, &fr, 0.85);
        if fr.hit > 0.05 {
            draw_hit_sparks(pb, &fr);
        }
        draw_boss_hp_arc(pb, &fr);
        return;
    }

    // 2) Spinning under-deck rotary blades glimpsed beneath front & side deck cutouts.
    draw_exposed_blades(pb, &fr, view.state);

    // 3) Exhaust jets / wheel kick-up sparks behind rear wheels during Charge / Windup.
    match view.state {
        MowerState::ChargeWindup => draw_rev_sparks(pb, &fr),
        MowerState::Charge | MowerState::Ricochet => draw_charge_exhaust(pb, &fr, view.bounce_count),
        MowerState::FlingBlades if fr.prog >= 0.35 => draw_fling_volley_preview(pb, &fr),
        _ => {}
    }

    // 4) Main sculpted hard-surface mower body (wheels, bumper, gunmetal skirt,
    //    safety-orange cowl, panel seams, STOP button, visor LEDs, progressive phase cracks).
    draw_upright_chassis(pb, &fr, view.state, view.bounce_count, 1.0);

    // 5) Overheat steam / Phase 2-3 damage smoke & arcing sparks.
    if view.state == MowerState::Overheat {
        draw_overheat_smoke(pb, &fr, 1.0);
    } else if fr.hp <= 0.66 {
        let dmg_smoke = if fr.hp <= 0.33 { 0.72 } else { 0.38 };
        draw_overheat_smoke(pb, &fr, dmg_smoke);
    }

    // 6) Phase-3 erratic electrical arc sparks + hit-flash metallic sparks.
    if fr.hp <= 0.33 {
        draw_phase3_arcs(pb, &fr);
    }
    if fr.hit > 0.05 {
        draw_hit_sparks(pb, &fr);
    }

    // 7) Clean segmented 3-phase radial Boss Health Arc (matches `art_enemy.rs`).
    draw_boss_hp_arc(pb, &fr);
}

/// Spinning rotary blades glimpsed under the front mouth and side skirt cutouts.
fn draw_exposed_blades(pb: &mut PixelBuffer, fr: &Frame, state: MowerState) {
    let rpm = match state {
        MowerState::FlingBlades => 28.0,
        MowerState::Charge | MowerState::Ricochet => 22.0,
        MowerState::ChargeWindup => 10.0 + 16.0 * fr.prog,
        MowerState::Overheat => 2.5,
        _ => 11.0,
    };
    let spin = fr.anim * rpm;
    let fast = rpm >= 18.0;
    let phase3 = fr.hp <= 0.33;
    let phase2 = fr.hp <= 0.66;

    // Blade deck center sits slightly forward of chassis center (`u = +2.8`).
    let deck_c = fr.place(2.8, 0.0);
    let blade_r = if state == MowerState::FlingBlades && fr.prog >= 0.30 {
        14.0
    } else if state == MowerState::Charge || state == MowerState::Ricochet {
        13.5
    } else if phase3 {
        13.2
    } else if phase2 {
        12.7
    } else {
        12.3
    };

    let Some((x0, x1, y0, y1)) = bbox_around(pb, deck_c, blade_r + 2.0) else {
        return;
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v) = fr.local(x, y);
            // Blades peek cleanly out of side skirt cutouts (between rear drive wheels and
            // front casters) AND beneath the front mowing mouth under the bumper!
            let in_front_gap = u > 10.8 && u < 15.2 && v.abs() < 7.2;
            let in_side_gap = u > -1.2 && u < 7.2 && v.abs() > 8.6;
            if !in_front_gap && !in_side_gap && state != MowerState::FlingBlades {
                continue;
            }
            let du = u - 2.8;
            let dv = v;
            let r = (du * du + dv * dv).sqrt();
            if r < 8.0 || r > blade_r + 1.0 {
                continue;
            }
            let ang = dv.atan2(du) - spin;
            // 4 crisp curved steel saw-blade teeth (`4 * ang`) so teeth are ALWAYS visible!
            let tooth = (ang * 4.0).sin();
            if r > blade_r {
                if tooth > 0.10 {
                    darken_px(pb, x, y, 0.62);
                }
                continue;
            }
            if tooth > 0.08 {
                let edge_hi = r > blade_r - 1.6 || (ang * 4.0).cos() > 0.35;
                let col = if state == MowerState::FlingBlades && fr.prog >= 0.30 {
                    mix(STEEL_SPEC, LED_BLADE, 0.58)
                } else if state == MowerState::Charge || state == MowerState::Ricochet {
                    mix(STEEL_SPEC, SPARK_HOT, if edge_hi { 0.45 } else { 0.15 })
                } else if edge_hi {
                    STEEL_SPEC
                } else {
                    STEEL_HI
                };
                blend_px(pb, x, y, apply_hit(col, fr.hit), 0.96);
            } else if fast && r > blade_r - 2.4 {
                // High-RPM steel motion-blur ring!
                let blur_col = if state == MowerState::FlingBlades {
                    LED_BLADE
                } else {
                    STEEL_HI
                };
                blend_px(pb, x, y, blur_col, 0.46);
            }
        }
    }
}

/// Sculpted hard-surface upright mower chassis, bumper, wheels, LEDs, STOP button & phase damage.
fn draw_upright_chassis(
    pb: &mut PixelBuffer,
    fr: &Frame,
    state: MowerState,
    bounce_count: u8,
    scale_v: f32,
) {
    let Some((x0, x1, y0, y1)) = fr.bbox(pb, 16.0) else {
        return;
    };
    let inv_scale_v = 1.0 / scale_v.max(0.20);

    // Slight front bumper spring compression during wall ricochet or charge impact.
    let bumper_push = match state {
        MowerState::Ricochet if fr.prog < 0.25 => (1.0 - fr.prog / 0.25) * 1.4,
        MowerState::ChargeWindup => 0.35 * fr.prog,
        _ => 0.0,
    };

    // Wheel tread scroll offset.
    let wheel_speed = match state {
        MowerState::Charge | MowerState::Ricochet => 22.0,
        MowerState::ChargeWindup => -18.0, // spinning tires digging in
        MowerState::Overheat => 0.0,
        MowerState::Scan => 3.0,
        _ => 8.0,
    };
    let tread_scroll = fr.anim * wheel_speed;

    // Status LED color & pattern based on state.
    let pulse = 0.5 + 0.5 * (fr.anim * 8.0).sin();
    let fast_blink = (fr.anim * 12.0).sin() > 0.0;
    let phase2 = fr.hp <= 0.66;
    let phase3 = fr.hp <= 0.33;
    // Phase-3 flickering visor dropout!
    let eye_flicker = !phase3 || hash01((fr.anim * 18.0) as i32, 7) > 0.22;

    let (led_col, led_fill) = match state {
        MowerState::Patrol => (LED_PATROL, 1.0),
        MowerState::Scan => {
            if fr.prog >= 0.70 {
                (mix(LED_WARN, LED_DANGER, (fr.prog - 0.70) / 0.30), 1.0)
            } else {
                (LED_SCAN, 1.0)
            }
        }
        MowerState::ChargeWindup => (mix(LED_WARN, LED_DANGER, fr.prog), fr.prog.max(0.15)),
        MowerState::Charge => (mix(LED_DANGER, SPARK_HOT, 0.35 * pulse), 1.0),
        MowerState::Ricochet => {
            let c = if fast_blink || bounce_count >= 2 {
                LED_DANGER
            } else {
                LED_WARN
            };
            (c, 1.0)
        }
        MowerState::FlingBlades => (mix(LED_BLADE, SPARK_HOT, 0.4 * pulse), 1.0),
        MowerState::Overheat => (
            if fast_blink { LED_DANGER } else { (70, 12, 14) },
            1.0,
        ),
        MowerState::Docking => (mix(LED_DOCK, LED_PATROL, pulse), fr.prog.max(0.25)),
        MowerState::Flipped | MowerState::Dying => (LED_DANGER, 1.0),
    };

    // Cyclops eye lateral sweep position in the smoked-glass front visor.
    let eye_v = match state {
        MowerState::Scan if fr.prog < 0.70 => {
            ((fr.prog / 0.70) * PI * 2.0 - FRAC_PI_2).sin() * 3.4
        }
        MowerState::Overheat => (fr.anim * 3.0).sin() * 1.2,
        _ => 0.0,
    };

    // Heat glow intensity for rear engine louvers.
    let heat_lv = match state {
        MowerState::Overheat => 0.75 + 0.25 * pulse,
        MowerState::Charge | MowerState::Ricochet => 0.65 + 0.35 * pulse,
        MowerState::ChargeWindup => 0.25 + 0.75 * fr.prog,
        _ if phase3 => 0.45 + 0.35 * pulse,
        _ => 0.0,
    };

    let rim_col = if fr.hit > 0.45 {
        (38, 22, 16)
    } else {
        (0, 0, 0)
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v_raw) = fr.local(x, y);
            let v = v_raw * inv_scale_v;

            let d_hull = sd_mower_hull(u, v);
            let d_bump = sd_front_bumper(u, v, bumper_push);
            let d_wheel = sd_rear_wheels(u, v);
            let d_cast = sd_front_casters(u, v);

            // Shock-absorber struts connecting front bumper to hull (`u ∈ [9.5, 12.2], |v| ≈ 4.6`).
            let d_strut = sd_rbox(u, v.abs(), 10.9 - bumper_push * 0.5, 4.6, 1.3, 0.75, 0.2);

            // Rear brass charging contact prongs (`u ∈ [-13.6, -11.8], |v| ≈ 3.2`).
            let d_prong = sd_rbox(u, v.abs(), -12.6, 3.2, 0.9, 0.7, 0.2);

            let d_all = d_hull.min(d_bump).min(d_wheel).min(d_cast).min(d_strut).min(d_prong);
            let cover = (0.55 - d_all * 1.25).clamp(0.0, 1.0);

            if cover <= 0.0 {
                if d_all < 1.2 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    pb.plot(x, y, mix(bg, rim_col, 0.90));
                }
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = if cover < 1.0 {
                mix(bg, rim_col, 0.90 * (1.0 - cover))
            } else {
                bg
            };

            let col: Rgb = if d_bump <= d_hull && d_bump <= d_wheel && d_bump <= d_strut {
                // --- WRAP-AROUND FRONT IMPACT BUMPER ---
                let lit = fr.lit(0.75, (v / 9.5).clamp(-1.0, 1.0));
                let mut c = shade3(RUBBER_DARK, RUBBER_MID, RUBBER_HI, 0.45 + 0.5 * lit);
                // Painted yellow/black safety chevron stripes on front bumper face!
                if u > 11.4 - bumper_push && v.abs() < 6.8 {
                    let band = ((v * 0.72 + 10.0).floor() as i32) % 2 == 0;
                    if band {
                        c = shade3((140, 102, 18), HAZARD_YELLOW, (255, 238, 126), 0.55 + 0.45 * lit);
                    } else {
                        c = HAZARD_DARK;
                    }
                    // Chipped bumper scuff in Phase 2/3.
                    if phase2 && v > 2.2 && v < 4.5 {
                        c = STEEL_MID;
                    }
                }
                // Brief impact spark flash on bumper during ricochet bounce!
                if state == MowerState::Ricochet && fr.prog < 0.22 {
                    c = mix(c, SPARK_HOT, (1.0 - fr.prog / 0.22) * 0.75);
                }
                c
            } else if d_wheel <= d_hull && d_wheel <= d_cast {
                // --- CHUNKY TREADED REAR DRIVE WHEELS ---
                let tread = ((u - tread_scroll) * 0.85).sin();
                let lit = fr.lit((u + 5.8) / 4.2, v.signum() * 0.75);
                let mut c = if tread > 0.12 {
                    shade3(RUBBER_DARK, RUBBER_MID, RUBBER_HI, 0.55 + 0.45 * lit)
                } else {
                    RUBBER_DARK
                };
                // Brushed steel hubcap glint on outer flank edge.
                if (u + 5.8).abs() < 1.4 && v.abs() > 11.6 {
                    c = STEEL_HI;
                }
                c
            } else if d_cast < d_hull {
                // --- FRONT SWIVEL CASTER ---
                let lit = fr.lit(0.3, v.signum() * 0.7);
                shade3(RUBBER_DARK, RUBBER_MID, STEEL_MID, 0.45 + 0.45 * lit)
            } else if d_strut < d_hull {
                // --- SILVER BUMPER SHOCK STRUT ---
                let lit = fr.lit(0.4, v.signum() * 0.5);
                shade3(STEEL_DARK, STEEL_HI, STEEL_SPEC, 0.5 + 0.45 * lit)
            } else if d_prong < d_hull {
                // --- REAR BRASS CHARGING PRONGS ---
                let glow = if state == MowerState::Docking { 0.55 + 0.45 * pulse } else { 0.2 };
                mix(BRASS_DARK, BRASS_HI, 0.55 + 0.45 * glow)
            } else {
                // --- SCULPTED MAIN CHASSIS & ORANGE MOLDED SHELL ---
                shade_upright_hull(
                    fr,
                    u,
                    v,
                    d_hull,
                    state,
                    led_col,
                    led_fill,
                    eye_v,
                    eye_flicker,
                    heat_lv,
                    pulse,
                    phase2,
                    phase3,
                )
            };

            out = mix(out, apply_hit(col, fr.hit), cover);
            pb.plot(x, y, out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn shade_upright_hull(
    fr: &Frame,
    u: f32,
    v: f32,
    d_hull: f32,
    state: MowerState,
    led_col: Rgb,
    led_fill: f32,
    eye_v: f32,
    eye_flicker: bool,
    heat_lv: f32,
    pulse: f32,
    phase2: bool,
    phase3: bool,
) -> Rgb {
    let av = v.abs();
    let nu = (u / 12.0).clamp(-1.0, 1.0);
    let nv = (v / 10.6).clamp(-1.0, 1.0);
    let lit = fr.lit(nu, nv);

    // 1) Outer gunmetal chassis skirt (`d_hull > -1.85`) + open side blade chutes in FlingBlades
    //    + front corner headlight LEDs.
    if d_hull > -1.85 {
        // Twin crisp front-corner headlight/hazard LEDs (`u ≈ 8.2..9.8, |v| ≈ 6.0..7.5`).
        if u > 7.6 && u < 9.8 && av > 5.6 && av < 7.6 {
            let hl = match state {
                MowerState::Overheat | MowerState::Ricochet => LED_DANGER,
                MowerState::Charge | MowerState::ChargeWindup => LED_WARN,
                MowerState::Docking => LED_DOCK,
                _ => (244, 248, 255),
            };
            return hl;
        }
        // Side discharge chute flaps flare open with cyan-steel glint during FlingBlades!
        if state == MowerState::FlingBlades && u > -1.5 && u < 5.5 && av > 7.5 {
            return mix(STEEL_DARK, LED_BLADE, 0.45 + 0.45 * pulse);
        }
        let bevel_lum = 0.40 + 0.52 * lit + if lit > 0.35 { 0.14 } else { 0.0 };
        let mut skirt = shade3(STEEL_DARK, STEEL_MID, STEEL_HI, bevel_lum);
        // Subtle corner rivets on the gunmetal fender skirt.
        if ((u + 8.0).abs() < 0.7 || (u - 6.0).abs() < 0.7) && av > 7.4 && av < 9.2 {
            skirt = STEEL_SPEC;
        }
        return skirt;
    }

    // 2) Phase 3 blown-off rear-right hood panel exposing glowing motor coils,
    //    spinning notched steel rotor/belt, and dark recessed chassis cavity!
    if phase3 && u > -10.2 && u < 2.2 && v > 1.8 && v < 8.4 {
        let jagged_front = 1.2 + (v * 1.6).sin() * 1.1;
        let jagged_inner = 2.4 + (u * 1.1).cos() * 0.8;
        if u < jagged_front && v > jagged_inner {
            // Torn blackened metal rim right around the hole!
            if (u - jagged_front).abs() < 0.85 || (v - jagged_inner).abs() < 0.85 {
                return (22, 16, 16);
            }
            // Spinning internal toothed drive belt + glowing molten motor coils!
            let belt_scroll = ((u + fr.anim * 16.0) * 1.5).sin();
            if (v - 4.8).abs() < 1.35 {
                return if belt_scroll > 0.0 { STEEL_SPEC } else { (26, 30, 38) };
            }
            let coil = ((u * 2.1).sin() * 0.5 + 0.5) * (0.55 + 0.45 * pulse);
            return mix((36, 14, 14), HEAT_GLOW, coil);
        }
    }

    // 3) Recessed smoked-glass front sensor visor + LED bar + lidar cyclops eye!
    // Curves across the front brow at `u ∈ [5.8, 9.0]`, `|v| < 5.8`.
    let visor_u = u - (7.4 - av * av * 0.055);
    if visor_u.abs() < 1.60 && av < 5.7 {
        // Visor bezel shadow edge.
        if visor_u.abs() > 1.08 || av > 5.05 {
            return (16, 20, 26);
        }
        // Dark smoked glass interior with subtle reflection.
        let mut vc = (14, 18, 26);
        if lit > 0.2 && visor_u > 0.35 {
            vc = (36, 46, 62);
        }
        // Phase-3 cracked visor: right half (`v > 1.4`) flickers dark/sparking, left burns hot red!
        if phase3 && v > 1.4 {
            return if hash01((fr.anim * 22.0) as i32, (v * 3.0) as i32) > 0.82 {
                SPARK_ELEC
            } else {
                (18, 16, 22)
            };
        }
        if !eye_flicker {
            return vc;
        }
        let active_led = if phase3 && state == MowerState::Patrol {
            mix(LED_WARN, LED_DANGER, 0.5 + 0.5 * pulse)
        } else {
            led_col
        };
        // Central scanning lidar cyclops optic!
        let eye_dist = ((v - eye_v).powi(2) + (visor_u * 1.25).powi(2)).sqrt();
        if eye_dist < 1.30 {
            return mix(active_led, SPARK_HOT, (1.0 - eye_dist / 1.30) * 0.72);
        }
        // Segmented status LED bar pips (`5` pips spanning `v ∈ [-4.2, +4.2]`).
        let frac = ((v + 4.4) / 8.8).clamp(0.0, 1.0);
        let pip_pos = (v * 0.62).fract().abs();
        let on_pip = pip_pos > 0.18 && pip_pos < 0.82 && visor_u.abs() < 0.72;
        if on_pip && frac <= led_fill {
            return active_led;
        }
        return vc;
    }

    // 4) Rear cooling ventilation louvers (`u ∈ [-10.6, -6.4], |v| < 5.5`).
    if (-10.6..=-6.4).contains(&u) && av < 5.5 {
        let slat = ((u + 11.0) * 1.12).fract();
        if slat < 0.52 {
            // Recessed vent slot — glows blazing molten orange-red when hot or Overheated!
            let glow_col = if state == MowerState::Overheat {
                mix(LED_DANGER, SPARK_HOT, 0.45 * pulse)
            } else {
                HEAT_GLOW
            };
            return mix((14, 16, 20), glow_col, heat_lv * (0.72 + 0.28 * (1.0 - av / 5.5)));
        } else {
            return shade3(STEEL_DARK, STEEL_MID, STEEL_HI, 0.42 + 0.45 * lit);
        }
    }

    // 5) Big red physical Emergency STOP Button on top deck (`u = -1.8, v = 0.0`)!
    let stop_r = sd_circle(u, v, -1.8, 0.0, 2.55);
    if stop_r < 0.90 {
        if stop_r >= 0.0 {
            // Recessed gunmetal collar — flashes red warning halo during Overheat!
            if state == MowerState::Overheat && (fr.anim * 12.0).sin() > 0.0 {
                return LED_DANGER;
            }
            return shade3((18, 20, 26), STEEL_MID, STEEL_HI, 0.35 - 0.45 * lit);
        }
        // Glossy raised red STOP cap (strobes warning during Overheat!).
        let b_nu = ((u + 1.8) / 2.5).clamp(-1.0, 1.0);
        let b_nv = (v / 2.5).clamp(-1.0, 1.0);
        let b_lit = fr.lit(b_nu, b_nv);
        let b_dome = (1.0 - (b_nu * b_nu + b_nv * b_nv) * 0.65).clamp(0.0, 1.0);
        let mut btn = shade3(STOP_DARK, STOP_MID, STOP_HI, 0.42 + 0.35 * b_dome + 0.55 * b_lit);
        if b_lit > 0.45 && b_dome > 0.45 {
            btn = mix(btn, (255, 224, 218), 0.52);
        }
        if state == MowerState::Overheat && (fr.anim * 12.0).sin() > 0.0 {
            btn = mix(btn, SPARK_HOT, 0.55);
        }
        // Crisp white center stop icon square.
        if (u + 1.8).abs() < 0.65 && av < 0.65 {
            btn = mix(btn, DECAL_WHITE, 0.72);
        }
        return btn;
    }

    // 6) Sculpted Safety-Orange ABS Molded Upper Cowl with restrained hard-surface chamfers!
    let bevel_edge = (d_hull + 1.85).abs();
    let dome = (1.0 - (nu * nu * 0.32 + nv * nv * 0.48)).clamp(0.0, 1.0);
    let mut lum = 0.40 + 0.24 * dome + 0.58 * lit;
    if bevel_edge < 1.35 {
        if lit > 0.16 {
            lum += 0.28;
        } else if lit < -0.15 {
            lum -= 0.26;
        }
    }
    let mut c = shade3(SHELL_SHADOW, SHELL_MID, SHELL_HI, lum);
    if lum > 0.98 {
        c = mix(c, SHELL_SPEC, ((lum - 0.98) * 2.1).clamp(0.0, 0.62));
    }

    // Industrial decals: twin crisp white-silver racing stripes down the cowl flanks!
    if av > 3.6 && av < 4.75 && u > -5.8 && u < 4.8 {
        c = mix(c, DECAL_WHITE, 0.78);
    }

    // Crisp recessed mechanical panel seam lines (1px dark groove + subtle lit lip).
    let hood_seam = (u - 3.6).abs();
    let rear_seam = (u + 5.8).abs();
    let side_seam = (av - 6.3).abs();
    if hood_seam < 0.42
        || (rear_seam < 0.42 && av < 6.4)
        || (side_seam < 0.38 && u > -6.0 && u < 5.2)
    {
        c = mix(c, (22, 16, 14), 0.76);
    } else if (u - 4.1).abs() < 0.35 && lit > 0.0 {
        c = mix(c, SHELL_SPEC, 0.24);
    }

    // Phase 2 & 3 bold jagged battle-damage cracks & scorch soot!
    if phase2 {
        let crack_f1 = ((u - 1.0) - (v + 2.4) * 0.72 + (v * 1.5).sin() * 0.45).abs();
        let in_c1 = u > -4.2 && u < 5.2 && v < -0.6 && v > -7.2;
        let crack_f2 = ((u + 2.8) + (v - 1.8) * 0.80).abs();
        let in_c2 = u > -6.0 && u < 3.6 && v > 0.6 && v < 6.8;

        // Subtle charcoal scorch smear around cracks.
        if (in_c1 && crack_f1 < 1.5) || (phase3 && in_c2 && crack_f2 < 1.6) {
            c = mix(c, (34, 20, 16), 0.42);
        }
        if (in_c1 && crack_f1 < 0.62) || (phase3 && in_c2 && crack_f2 < 0.62) {
            c = if phase3 {
                mix((20, 12, 12), HEAT_GLOW, 0.55 + 0.45 * pulse)
            } else {
                (20, 14, 14)
            };
        }
    }

    c
}

/// Flipped-over turtle moment (`MowerState::Flipped`):
/// 2.5D roll foreshortening on entry/exit, exposed spinning-stalled underside rotary
/// blades, glowing vulnerable green-gold battery & PCB core, kicking wheels, error LED!
fn draw_flipped_mower(pb: &mut PixelBuffer, fr: &Frame) {
    // 2.5D roll foreshortening during roll-over (0..0.15) and self-righting (0.85..1.0).
    let roll_scale_v = if fr.prog < 0.15 {
        let t = fr.prog / 0.15; // 0 -> 1
        (t * PI).cos().abs().max(0.28)
    } else if fr.prog > 0.85 {
        let t = (fr.prog - 0.85) / 0.15;
        (t * PI).cos().abs().max(0.28)
    } else {
        1.0
    };
    let show_belly = fr.prog >= 0.075 && fr.prog <= 0.925;
    if !show_belly {
        // Mid-flip upright half — compress lateral axis smoothly into the roll.
        draw_upright_chassis(pb, fr, MowerState::Overheat, 0, roll_scale_v);
        return;
    }

    let Some((x0, x1, y0, y1)) = fr.bbox(pb, 16.0) else {
        return;
    };
    let pulse = 0.5 + 0.5 * (fr.anim * 10.0).sin();
    let err_blink = (fr.anim * 12.0).sin() > 0.0;
    // Frantic wheel kick & stuttering stalled blade angle!
    let kick_tread = fr.anim * 26.0;
    let stutter_blade = (fr.anim * 4.5).sin() * 0.55 + fr.anim * 0.6;
    let rim_col = if fr.hit > 0.45 { (38, 22, 16) } else { (0, 0, 0) };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (u, v_raw) = fr.local(x, y);
            let v = v_raw / roll_scale_v;
            let av = v.abs();

            let d_hull = sd_mower_hull(u, v);
            let d_bump = sd_front_bumper(u, v, 0.0);
            let d_wheel = sd_rear_wheels(u, v);
            let d_cast = sd_front_casters(u, v);
            let d_all = d_hull.min(d_bump).min(d_wheel).min(d_cast);
            let cover = (0.55 - d_all * 1.25).clamp(0.0, 1.0);

            if cover <= 0.0 {
                if d_all < 1.1 {
                    let bg = pb.pixel_at(x as usize, y as usize);
                    pb.plot(x, y, mix(bg, rim_col, 0.72));
                }
                continue;
            }

            let bg = pb.pixel_at(x as usize, y as usize);
            let mut out = if cover < 1.0 {
                mix(bg, rim_col, 0.72 * (1.0 - cover))
            } else {
                bg
            };

            let lit = fr.lit((u / 12.0).clamp(-1.0, 1.0), (v / 10.6).clamp(-1.0, 1.0));

            let col: Rgb = if d_wheel <= d_hull && d_wheel <= d_bump {
                // Kicking rear tires spinning wildly in mid-air!
                let tread = ((u - kick_tread) * 0.95).sin();
                if tread > 0.0 {
                    shade3(RUBBER_DARK, RUBBER_HI, STEEL_HI, 0.5 + 0.45 * lit)
                } else {
                    RUBBER_DARK
                }
            } else if d_cast <= d_hull {
                // Spinning little front caster wheels!
                let tread = ((u + kick_tread * 1.3) * 1.2).sin();
                if tread > 0.0 { STEEL_HI } else { RUBBER_MID }
            } else if d_bump <= d_hull {
                // Underside of front hazard bumper.
                shade3(RUBBER_DARK, RUBBER_MID, RUBBER_HI, 0.42 + 0.45 * lit)
            } else if d_hull > -1.45 {
                // Thin rim of the safety-orange dome peeking around the flipped baseplate!
                shade3(SHELL_SHADOW, SHELL_MID, SHELL_HI, 0.48 + 0.5 * lit)
            } else {
                // --- EXPOSED FLAT UNDERBELLY CHASSIS ---
                let mut c = shade3((20, 24, 30), (42, 48, 58), (74, 84, 98), 0.45 + 0.45 * lit);

                // 1) Huge circular underside rotary blade cutting deck (`u = +2.8, v = 0, r = 6.8`)!
                let du = u - 2.8;
                let r_deck = (du * du + v * v).sqrt();
                if r_deck < 6.8 {
                    // Recessed dark blade well.
                    c = (14, 16, 20);
                    if r_deck > 5.9 {
                        c = STEEL_MID;
                    } else if r_deck < 1.9 {
                        // Central yellow-steel rotor hub with nut!
                        c = shade3((112, 86, 20), HAZARD_YELLOW, SPARK_HOT, 0.5 + 0.45 * lit);
                    } else {
                        // 3 gleaming razor steel blades mounted on the rotor hub (stuttering!).
                        let a = v.atan2(du) - stutter_blade;
                        let arm = (a * 3.0).cos();
                        if arm > 0.62 {
                            let sharp = (a * 3.0).sin() > 0.0;
                            c = if sharp { STEEL_SPEC } else { STEEL_MID };
                        }
                    }
                }

                // 2) Exposed glowing vulnerable Battery Pack & Green PCB Belly Core (`u ∈ [-9.2, -4.4]`)!
                if (-9.2..=-4.4).contains(&u) && av <= 5.4 {
                    if !(-8.6..=-5.0).contains(&u) || av > 4.8 {
                        // Metallic battery bay frame.
                        c = STEEL_HI;
                    } else {
                        // Pulsing bright green-cyan PCB & glowing lithium cell (2x crit target!).
                        let trace = (((u * 1.4).floor() as i32 + (v * 1.4).floor() as i32) & 1) == 0;
                        let base_pcb = if trace { PCB_MID } else { PCB_DARK };
                        c = mix(base_pcb, PCB_TRACE, 0.35 + 0.55 * pulse);
                        // Bright yellow-white glowing core bar in the center of the battery.
                        if (u + 6.8).abs() < 1.1 && av < 2.6 {
                            c = mix(PCB_TRACE, SPARK_HOT, 0.45 + 0.45 * pulse);
                        }
                    }
                }

                // 2b) Copper power leads bridging battery pack to rotary motor hub!
                if u > -4.6 && u < -2.4 && (av - 2.1).abs() < 0.65 {
                    c = mix(BRASS_DARK, BRASS_HI, 0.55 + 0.45 * pulse);
                }

                // 3) Underside blinking ERROR-RED diagnostics beacon near nose (`u ≈ +10.2, v = 0`).
                if (u - 10.2).abs() < 1.05 && av < 1.8 {
                    c = if err_blink {
                        mix(LED_DANGER, SPARK_HOT, 0.25)
                    } else {
                        (64, 14, 16)
                    };
                }

                c
            };

            out = mix(out, apply_hit(col, fr.hit), cover);
            pb.plot(x, y, out);
        }
    }

    // Crisp additive red glow on the underside error LED when blinking ON.
    if err_blink {
        let ep = fr.px(10.2, 0.0);
        pb.plot_add(ep.0, ep.1, scale(LED_DANGER, 0.7));
        pb.plot_add(ep.0 + 1, ep.1, scale(LED_DANGER, 0.35));
        pb.plot_add(ep.0 - 1, ep.1, scale(LED_DANGER, 0.35));
        pb.plot_add(ep.0, ep.1 + 1, scale(LED_DANGER, 0.35));
        pb.plot_add(ep.0, ep.1 - 1, scale(LED_DANGER, 0.35));
    }
}

/// Wheel-spin rev sparks kicked backward during `ChargeWindup`.
fn draw_rev_sparks(pb: &mut PixelBuffer, fr: &Frame) {
    let intensity = fr.prog;
    for side in [-1.0f32, 1.0] {
        for k in 0..4 {
            let phase = (fr.anim * 9.0 + k as f32 * 0.25 + side * 0.3).fract();
            let u = -9.5 - phase * (5.5 + 4.0 * intensity);
            let v = side * (10.5 + (phase * 7.0 + k as f32).sin() * 1.8);
            let p = fr.px(u, v);
            let col = if phase < 0.4 { SPARK_HOT } else { HEAT_GLOW };
            pb.plot_add(p.0, p.1, scale(col, (1.0 - phase) * (0.5 + 0.5 * intensity)));
        }
    }
}

/// Twin roaring rear turbo exhaust jets during `Charge` and `Ricochet`.
fn draw_charge_exhaust(pb: &mut PixelBuffer, fr: &Frame, bounce_count: u8) {
    let hot = if bounce_count >= 2 { LED_DANGER } else { HEAT_GLOW };
    for side in [-1.0f32, 1.0] {
        let len = 5.5 + 2.5 * (fr.anim * 26.0 + side).sin().abs();
        let base = fr.place(-12.2, side * 3.4);
        let tip = fr.place(-12.2 - len, side * 3.4);
        draw_line_styled(pb, base, tip, hot, 0.88, 0, 0, 0, true);
        let core = fr.px(-13.2, side * 3.4);
        pb.plot_add(core.0, core.1, SPARK_HOT);
    }
}

/// Muzzle-burst ejector glints inside the real `15.5 px` deck aura during `FlingBlades`.
/// (Live flung discs are rendered on the projectile layer by [`draw_blade_disc`] so no
/// phantom non-damaging projectiles appear outside the deck.)
fn draw_fling_volley_preview(pb: &mut PixelBuffer, fr: &Frame) {
    let fire_t = ((fr.prog - 0.35) / 0.65).clamp(0.0, 1.0);
    let fade = (1.0 - fire_t).clamp(0.0, 1.0);
    if fade <= 0.04 {
        return;
    }
    let spokes = if fr.hp <= 0.66 { 8 } else { 6 };
    let base_ang = fr.f.angle() + fire_t * 1.1;

    for k in 0..spokes {
        let a = base_ang + k as f32 / spokes as f32 * TAU;
        let dir = Vec2::from_angle(a);
        let p0 = fr.pos + dir * 10.5;
        let p1 = fr.pos + dir * (12.0 + fire_t * 3.2);
        draw_line_styled(pb, p0, p1, mix(LED_BLADE, SPARK_HOT, 0.45), 0.80 * fade, 0, 0, 0, true);
    }
}

/// Billowing cel-shaded steam & smoke puffs drifting cleanly upward from the rear engine vents.
fn draw_overheat_smoke(pb: &mut PixelBuffer, fr: &Frame, density: f32) {
    let vent = fr.place(-8.6, 0.0);
    let count = if density > 0.8 { 5 } else { 3 };
    for i in 0..count {
        // Start slightly lifted (`0.18..1.0`) so smoke plumes drift cleanly off the rear louvers
        // without muddying the central red STOP button.
        let t = (fr.anim * 0.95 + i as f32 / count as f32).fract();
        let lift = 0.20 + 0.80 * t;
        let drift_x = (t * 5.8 + i as f32 * 1.4).sin() * 2.6 + lift * 4.5;
        let drift_y = -2.5 - lift * 13.5;
        let center = vent + Vec2::new(drift_x, drift_y);
        let r = 1.9 + lift * 2.8;
        if density > 0.8 {
            // Bright pressurized white-cyan steam clouds for Overheat & Flipped!
            let alpha = (1.0 - t * 0.58) * 0.94;
            draw_smoke_puff(pb, center, r, (124, 138, 154), (202, 214, 226), (250, 253, 255), alpha);
        } else {
            // Darker oily engine smoke wisps for Phase 2 & 3 damage!
            let alpha = (1.0 - t * 0.82) * 0.85 * density;
            draw_smoke_puff(pb, center, r, (30, 32, 38), (68, 74, 84), (118, 126, 138), alpha);
        }
    }
}

/// Phase-3 intermittent cyan-yellow electrical arc sparks snapping off damaged internals.
fn draw_phase3_arcs(pb: &mut PixelBuffer, fr: &Frame) {
    let step = (fr.anim * 14.0) as i32;
    if hash01(step, 31) > 0.72 {
        return;
    }
    let ang = hash01(step, 47) * TAU;
    let p0 = fr.place(-4.5, 4.8);
    let p1 = p0 + Vec2::from_angle(ang) * 5.8;
    let mid = (p0 + p1) * 0.5 + Vec2::from_angle(ang + FRAC_PI_2) * 2.0;
    draw_line_styled(pb, p0, mid, SPARK_ELEC, 0.95, 0, 0, 0, true);
    draw_line_styled(pb, mid, p1, SPARK_HOT, 0.95, 0, 0, 0, true);
}

/// Deterministic crackling 2-segment electric & metallic sparks bursting off the chassis when struck.
fn draw_hit_sparks(pb: &mut PixelBuffer, fr: &Frame) {
    let seed = (fr.anim * 24.0) as i32;
    for k in 0..7 {
        let a = k as f32 / 7.0 * TAU + hash01(seed + k * 7, 11) * 0.55;
        let dir = Vec2::from_angle(a);
        let perp = dir.perp();
        let r0 = 10.5 + hash01(seed + k, 23) * 2.0;
        let len = 4.0 + fr.hit * 5.0;
        let p0 = fr.pos + dir * r0;
        let zig = if k % 2 == 0 { 1.8 } else { -1.8 };
        let mid = fr.pos + dir * (r0 + len * 0.5) + perp * zig;
        let p1 = fr.pos + dir * (r0 + len);
        let col = if k % 2 == 0 { SPARK_HOT } else { SPARK_ELEC };
        draw_line_styled(pb, p0, mid, col, 0.95 * fr.hit, 0, 0, 0, true);
        draw_line_styled(pb, mid, p1, SPARK_HOT, 0.85 * fr.hit, 0, 0, 0, true);
    }
}

/// Multi-stage mechanical death explosion (`progress 0.0..1.0`).
fn draw_dying_sequence(pb: &mut PixelBuffer, fr: &Frame) {
    let p = fr.prog;

    if p < 0.44 {
        // Stage 1: Violent meltdown jitter + electrical arcs + building white-hot core!
        let t = p / 0.44;
        let mut jf = *fr;
        let jx = (fr.anim * 73.0).sin() * (0.7 + 1.8 * t);
        let jy = (fr.anim * 61.0).cos() * (0.7 + 1.8 * t);
        jf.pos += Vec2::new(jx, jy);
        jf.hit = (t * 0.55 + if (fr.anim * 24.0).sin() > 0.0 { 0.30 } else { 0.0 }).min(0.85);
        jf.hp = 0.05;

        draw_exposed_blades(pb, &jf, MowerState::Overheat);
        draw_upright_chassis(pb, &jf, MowerState::Overheat, 0, 1.0);
        draw_overheat_smoke(pb, &jf, 0.9);
        draw_hit_sparks(pb, &jf);

        // White-hot reactor breach building under the cracked cowl before detonation!
        let core_r = 2.5 + t * 8.5;
        draw_puff(pb, jf.pos, core_r, (255, 255, 240), HEAT_GLOW, 0.35 + 0.65 * t, true);
        if t > 0.65 {
            // Crisp jagged pre-detonation crack rays bursting from the shell!
            for k in 0..6 {
                let a = k as f32 / 6.0 * TAU + 0.3;
                let ray = Vec2::from_angle(a) * (10.0 + t * 8.0);
                draw_line_styled(pb, jf.pos, jf.pos + ray, SPARK_HOT, 0.8 * t, 0, 0, 0, true);
            }
        }
    } else if p < 0.78 {
        // Stage 2: Main capacitor fireball + jagged blast spikes + cel-shaded smoke + flying parts!
        let t = (p - 0.44) / 0.34; // 0..1

        // Expanding fiery shockwave ring out to DEATH_BLAST_RADIUS.
        let ring_r = 7.0 + t * (DEATH_BLAST_RADIUS - 5.0);
        let ring_col = mix(SPARK_HOT, HEAT_GLOW, t);
        draw_ring_styled(pb, fr.pos, ring_r, ring_col, (1.0 - t * 0.65) * 0.95, 0, 0.0, true);

        // Organic asymmetric blast smoke clouds (outer soot puffs + hot inner fire billows).
        const CLOUDS: [(f32, f32, f32, bool); 7] = [
            (0.18, 1.05, 6.4, false),
            (1.12, 0.82, 5.2, true),
            (2.05, 1.15, 6.8, false),
            (3.15, 0.76, 5.6, true),
            (4.10, 1.08, 6.0, false),
            (5.18, 0.90, 5.0, true),
            (5.85, 0.52, 4.6, true),
        ];
        for &(ang, dist_mul, base_r, hot) in &CLOUDS {
            let dist = (3.8 + t * 9.8) * dist_mul;
            let lobe = fr.pos + Vec2::from_angle(ang) * dist;
            let lr = (base_r - t * 1.35).max(2.5);
            let sm_a = (1.0 - t * 0.45).clamp(0.28, 0.94);
            if hot {
                draw_smoke_puff(pb, lobe, lr, (78, 26, 14), (186, 76, 24), (255, 188, 76), sm_a);
            } else {
                draw_smoke_puff(pb, lobe, lr, (26, 22, 24), (66, 52, 48), (128, 96, 78), sm_a);
            }
        }

        // Jagged radiant detonation spikes!
        for k in 0..8 {
            let a = k as f32 / 8.0 * TAU + 0.15;
            let len = (17.5 - t * 7.0) * if k % 2 == 0 { 1.12 } else { 0.82 };
            let tip = fr.pos + Vec2::from_angle(a) * len;
            draw_line_styled(pb, fr.pos, tip, SPARK_HOT, (1.0 - t * 0.75) * 0.95, 0, 0, 0, true);
        }

        // Blazing incandescent center fireball.
        let main_r = (13.5 * (1.0 - t * 0.55)).max(3.5);
        draw_puff(pb, fr.pos, main_r, (255, 255, 232), HEAT_GLOW, (1.0 - t * 0.5) * 0.98, true);

        // Recognizable detached machine debris flying outward!
        draw_flying_debris(pb, fr, t);
    } else {
        // Stage 3: Charred cracked wreckage hub, tumbling parts settling, & rising smoke column.
        let t = (p - 0.78) / 0.22; // 0..1
        draw_ring_styled(
            pb,
            fr.pos,
            DEATH_BLAST_RADIUS + t * 3.5,
            HEAT_GLOW,
            0.38 * (1.0 - t),
            3,
            0.0,
            true,
        );

        // Blackened bent chassis husk left inside the scorch crater!
        draw_wreckage_husk(pb, fr.pos, 1.0 - t * 0.25);
        draw_flying_debris(pb, fr, 1.0 + t * 0.2);

        // Crisp rising cel-shaded smoke column drifting upward from the crater.
        for i in 0..4 {
            let st = (t * 0.65 + i as f32 * 0.24).min(1.0);
            let sc = fr.pos + Vec2::new(((i as f32) - 1.5) * 2.8 + st * 2.5, -2.0 - st * 12.5);
            draw_smoke_puff(
                pb,
                sc,
                2.6 + st * 2.6,
                (26, 28, 32),
                (58, 62, 70),
                (108, 114, 124),
                (1.0 - st * 0.75) * 0.85,
            );
        }
    }
}

/// Blackened bent chassis husk remaining in the crater during the final death beat.
fn draw_wreckage_husk(pb: &mut PixelBuffer, center: Vec2, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, 8.5) else {
        return;
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let d = sd_rbox(dx, dy, 0.0, 0.0, 6.2, 5.2, 1.5);
            if d > 1.0 {
                continue;
            }
            if d > 0.0 {
                darken_px(pb, x, y, 0.65 * alpha);
                continue;
            }
            // Jagged burnt-out center cavity.
            let hole = (dx * dx + dy * dy).sqrt() < 2.6 + (dx.atan2(dy) * 3.0).sin() * 0.9;
            let col = if hole {
                if hash01(x, y) > 0.72 { HEAT_GLOW } else { (16, 12, 12) }
            } else {
                let lit = (dx * LIGHT.x + dy * LIGHT.y) / 7.0;
                shade3((20, 22, 26), (48, 42, 40), (88, 76, 68), 0.45 + 0.45 * lit)
            };
            blend_px(pb, x, y, col, 0.92 * alpha);
        }
    }
}

/// Recognizable flying detached parts: orange hood shard w/ white stripe, STOP cap,
/// chunky treaded rubber tire w/ hubcap, and spinning 3-spoke steel rotary blade!
fn draw_flying_debris(pb: &mut PixelBuffer, fr: &Frame, t: f32) {
    let fade = (1.0 - (t - 0.88).max(0.0) * 1.6).clamp(0.35, 1.0);

    // 1) Chunky orange cowl shard with white stripe flying up-left.
    let s1 = fr.pos + Vec2::new(-0.92, -0.62).normalized() * (6.5 + t * 16.5);
    draw_cowl_shard(pb, s1, fade);

    // 2) Popped red emergency STOP button cap flying down-right.
    let s2 = fr.pos + Vec2::new(0.85, 0.60).normalized() * (6.0 + t * 15.0);
    draw_stop_debris(pb, s2, fade);

    // 3) Detached chunky rubber wheel with silver hubcap tumbling up-right.
    let w1 = fr.pos + Vec2::new(0.78, -0.78).normalized() * (7.5 + t * 17.5);
    draw_wheel_debris(pb, w1, fade);

    // 4) Spinning 3-spoke steel rotary blade flying down-left!
    let b1 = fr.pos + Vec2::new(-0.82, 0.72).normalized() * (7.0 + t * 17.0);
    let spin = t * 12.0;
    for k in 0..3 {
        let a = spin + k as f32 / 3.0 * TAU;
        let tip = b1 + Vec2::from_angle(a) * 3.8;
        draw_line_styled(pb, b1, tip, STEEL_SPEC, 0.95 * fade, 0, 0, 0, false);
    }
    blend_px(pb, b1.x.round() as i32, b1.y.round() as i32, HAZARD_YELLOW, fade);
}

fn draw_cowl_shard(pb: &mut PixelBuffer, pos: Vec2, alpha: f32) {
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    for dy in -3i32..=3 {
        for dx in -3i32..=3 {
            if dx * dx + dy * dy <= 7 && dx + dy < 3 {
                let c = if dx == 0 && dy.abs() <= 2 {
                    DECAL_WHITE
                } else if dx < 0 || dy < 0 {
                    SHELL_HI
                } else {
                    SHELL_MID
                };
                blend_px(pb, cx + dx, cy + dy, c, alpha);
            } else if dx * dx + dy * dy <= 10 && dx + dy < 4 {
                darken_px(pb, cx + dx, cy + dy, 0.55 * alpha);
            }
        }
    }
}

fn draw_stop_debris(pb: &mut PixelBuffer, pos: Vec2, alpha: f32) {
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let r2 = dx * dx + dy * dy;
            if r2 <= 4 {
                let c = if r2 == 0 {
                    DECAL_WHITE
                } else if dx < 0 && dy < 0 {
                    STOP_HI
                } else {
                    STOP_MID
                };
                blend_px(pb, cx + dx, cy + dy, c, alpha);
            } else if r2 <= 6 {
                darken_px(pb, cx + dx, cy + dy, 0.55 * alpha);
            }
        }
    }
}

fn draw_wheel_debris(pb: &mut PixelBuffer, pos: Vec2, alpha: f32) {
    let cx = pos.x.round() as i32;
    let cy = pos.y.round() as i32;
    for dy in -3i32..=3 {
        for dx in -2i32..=2 {
            let on_tire = dx.abs() <= 2 && dy.abs() <= 3 && !(dx.abs() == 2 && dy.abs() == 3);
            if on_tire {
                let c = if dx.abs() <= 1 && dy.abs() <= 1 {
                    STEEL_HI
                } else if (dy & 1) == 0 {
                    RUBBER_HI
                } else {
                    RUBBER_DARK
                };
                blend_px(pb, cx + dx, cy + dy, c, alpha);
            }
        }
    }
}

/// Clean segmented 3-phase radial Boss Health Arc (ticks at 33% and 66% matching damage phases).
fn draw_boss_hp_arc(pb: &mut PixelBuffer, fr: &Frame) {
    if fr.hp <= 0.0 {
        return;
    }
    let arc_r = BODY_RADIUS + 4.5;
    if bbox_around(pb, fr.pos, arc_r + 2.0).is_none() {
        return;
    }
    let pulse = 0.5 + 0.5 * (fr.anim * 6.0).sin();
    let hp_col = if fr.hp > 0.66 {
        mix(HAZARD_YELLOW, SHELL_HI, (fr.hp - 0.66) / 0.34)
    } else if fr.hp > 0.33 {
        mix(LED_DANGER, HAZARD_YELLOW, (fr.hp - 0.33) / 0.33)
    } else {
        mix((185, 24, 30), (255, 92, 68), pulse)
    };
    let track_col = (18, 20, 24);
    let steps = (arc_r * 7.0).round() as i32;
    let mut last = (i32::MIN, i32::MIN);

    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let near_tick = (t - 0.333).abs() < 0.015 || (t - 0.666).abs() < 0.015;
        let a = -FRAC_PI_2 + t * TAU;
        let x = (fr.pos.x + a.cos() * arc_r).round() as i32;
        let y = (fr.pos.y + a.sin() * arc_r).round() as i32;
        if (x, y) == last {
            continue;
        }
        last = (x, y);
        if near_tick {
            blend_px(pb, x, y, (8, 10, 12), 0.85);
        } else if t <= fr.hp {
            let ox = (fr.pos.x + a.cos() * (arc_r + 1.0)).round() as i32;
            let oy = (fr.pos.y + a.sin() * (arc_r + 1.0)).round() as i32;
            darken_px(pb, ox, oy, 0.45);
            blend_px(pb, x, y, hp_col, 0.94);
        } else {
            blend_px(pb, x, y, track_col, 0.52);
        }
    }
}

// ============================================================================
// Public Drawing API — Projectile Layer (Blade Discs & Grass Shrapnel)
// ============================================================================

/// **Projectile Layer**: Draw a spinning steel blade disc flung by [`MowerState::FlingBlades`].
///
/// Matches collision radius [`BLADE_DISC_RADIUS`] (`3.5 px`).
pub fn draw_blade_disc(pb: &mut PixelBuffer, pos: Vec2, anim: f32, cam: Vec2, _accent: Rgb) {
    draw_blade_disc_screen(pb, pos - cam, anim, 1.0);
}

fn draw_blade_disc_screen(pb: &mut PixelBuffer, center: Vec2, anim: f32, alpha: f32) {
    let Some((x0, x1, y0, y1)) = bbox_around(pb, center, BLADE_DISC_RADIUS + 2.0) else {
        return;
    };
    let spin = anim * 22.0;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - center.x;
            let dy = y as f32 - center.y;
            let r = (dx * dx + dy * dy).sqrt();
            if r > BLADE_DISC_RADIUS + 1.0 {
                continue;
            }
            if r > BLADE_DISC_RADIUS {
                darken_px(pb, x, y, 0.55 * alpha);
                continue;
            }
            let col = if r < 1.35 {
                // Bright safety-orange / red-hot spinning hub.
                mix(SHELL_MID, SPARK_HOT, 0.45)
            } else {
                let a = dy.atan2(dx) - spin;
                let saw = (a * 4.0).sin();
                if saw > 0.15 {
                    mix(STEEL_SPEC, LED_BLADE, 0.35)
                } else {
                    STEEL_MID
                }
            };
            blend_px(pb, x, y, col, 0.95 * alpha);
        }
    }
}

/// **Projectile Layer**: Draw a razor grass-clipping shrapnel dart flung by [`MowerState::FlingBlades`].
///
/// Matches collision radius [`SHRAPNEL_RADIUS`] (`2.0 px`).
pub fn draw_grass_shrapnel(pb: &mut PixelBuffer, pos: Vec2, dir: Vec2, cam: Vec2, _accent: Rgb) {
    let d = if dir.len_sq() > 1e-4 {
        dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    draw_grass_shrapnel_screen(pb, pos - cam, d, 1.0);
}

fn draw_grass_shrapnel_screen(pb: &mut PixelBuffer, center: Vec2, dir: Vec2, alpha: f32) {
    if bbox_around(pb, center, 4.0).is_none() {
        return;
    }
    let perp = dir.perp();
    let tip = center + dir * 2.4;
    let tail = center - dir * 2.2;
    let fl = center + perp * 1.1;
    let fr = center - perp * 1.1;

    darken_px(pb, center.x.round() as i32 + 1, center.y.round() as i32 + 1, 0.4 * alpha);
    draw_line_styled(pb, tail, tip, GRASS_CLIP, 0.95 * alpha, 0, 0, 0, false);
    blend_px(pb, fl.x.round() as i32, fl.y.round() as i32, GRASS_CLIP, 0.85 * alpha);
    blend_px(pb, fr.x.round() as i32, fr.y.round() as i32, GRASS_CLIP, 0.85 * alpha);
    blend_px(pb, tip.x.round() as i32, tip.y.round() as i32, GRASS_BRIGHT, alpha);
}

// ============================================================================
// Tests & Sprite Sheet Generator
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_floor;
    use crate::game::art_rat;
    use crate::game::level::{Level, Theme};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;

    const ALL_STATES: [MowerState; 10] = [
        MowerState::Patrol,
        MowerState::Scan,
        MowerState::ChargeWindup,
        MowerState::Charge,
        MowerState::Ricochet,
        MowerState::FlingBlades,
        MowerState::Overheat,
        MowerState::Flipped,
        MowerState::Docking,
        MowerState::Dying,
    ];

    /// Drawing must never panic for any state, progress (including out-of-range),
    /// facing (including ZERO), buffer size (including 0x0 and 1x1), or off-screen position.
    #[test]
    fn mower_draws_everywhere_without_panic() {
        for (bw, bh) in [(0usize, 0usize), (1, 1), (8, 8), (80, 60)] {
            let mut pb = PixelBuffer::new(bw, bh);
            for &state in &ALL_STATES {
                for &prog in &[-0.25f32, 0.0, 0.08, 0.35, 0.5, 0.78, 1.0, 1.4] {
                    for &pos in &[
                        Vec2::new(40.0, 30.0),
                        Vec2::new(0.0, 0.0),
                        Vec2::new(-80.0, -80.0),
                        Vec2::new(500.0, 500.0),
                        Vec2::new(79.5, 59.5),
                    ] {
                        for &facing in &[
                            Vec2::ZERO,
                            Vec2::new(1.0, 0.0),
                            Vec2::new(-0.6, 0.8),
                            Vec2::new(0.0, -1.0),
                        ] {
                            let view = MowerView {
                                pos,
                                facing,
                                state,
                                progress: prog,
                                anim: 1.7 + prog * 3.0,
                                hit_flash: if prog > 0.6 { 0.85 } else { 0.0 },
                                hp_frac: 1.0 - prog.clamp(0.0, 1.0) * 0.85,
                                target: pos + Vec2::new(42.0, 18.0),
                                trail_start: pos - Vec2::new(36.0, 12.0),
                                dock_pos: pos + Vec2::new(-10.0, 4.0),
                                bounce_count: 2,
                            };
                            draw_mower_ground(&mut pb, &view, Vec2::ZERO, (90, 220, 200));
                            draw_mower(&mut pb, &view, Vec2::ZERO, (90, 220, 200));
                            draw_blade_disc(&mut pb, pos, 1.3, Vec2::ZERO, (90, 220, 200));
                            draw_grass_shrapnel(&mut pb, pos, facing, Vec2::ZERO, (90, 220, 200));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hit_flash_and_phases_are_visually_distinct() {
        let render = |hp: f32, flash: f32, state: MowerState| {
            let mut pb = PixelBuffer::new(64, 64);
            pb.clear((14, 22, 20));
            let mut v = MowerView::patrol(Vec2::new(32.0, 32.0), Vec2::new(1.0, 0.2), 0.8);
            v.hp_frac = hp;
            v.hit_flash = flash;
            v.state = state;
            v.progress = 0.5;
            draw_mower_ground(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            draw_mower(&mut pb, &v, Vec2::ZERO, (90, 220, 200));
            let mut sum = 0u64;
            let mut hash = 0u64;
            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    sum += c.0 as u64 + c.1 as u64 + c.2 as u64;
                    hash = hash
                        .wrapping_mul(31)
                        .wrapping_add(c.0 as u64 + (c.1 as u64) * 257 + (c.2 as u64) * 65537);
                }
            }
            (sum, hash)
        };
        let (p1_sum, p1_h) = render(1.0, 0.0, MowerState::Patrol);
        let (hit_sum, _) = render(1.0, 0.9, MowerState::Patrol);
        let (_, p3_h) = render(0.2, 0.0, MowerState::Patrol);
        let (_, flip_h) = render(0.8, 0.0, MowerState::Flipped);

        assert!(hit_sum > p1_sum + 1000, "hit flash should brighten chassis");
        assert_ne!(p1_h, p3_h, "phase 3 damage should visibly alter chassis");
        assert_ne!(p1_h, flip_h, "flipped turtle state must differ from upright");
    }

    #[test]
    fn collision_helpers_match_spec() {
        assert_eq!(collision_radius(MowerState::Patrol, 0.5), BODY_RADIUS);
        assert_eq!(collision_radius(MowerState::Flipped, 0.5), FLIPPED_RADIUS);
        assert_eq!(collision_radius(MowerState::Dying, 0.8), 0.0);
        assert_eq!(incoming_damage_mult(MowerState::Overheat), 1.5);
        assert_eq!(incoming_damage_mult(MowerState::Flipped), 2.0);
        assert_eq!(incoming_damage_mult(MowerState::Dying), 0.0);
    }

    /// Full multi-state & 3-theme sprite sheet:
    /// `RATRUN_DUMP=mower cargo test dump_mower_sheet` writes `/tmp/ratrun_mower.ppm`.
    #[test]
    fn dump_mower_sheet() {
        if std::env::var("RATRUN_DUMP").as_deref() != Ok("mower") {
            return;
        }
        let w = 400usize;
        let h = 232usize;
        let mut out = format!("P6\n{} {}\n255\n", w, h * 3).into_bytes();

        for theme in Theme::all() {
            let level = Level::new(theme);
            let cam = Vec2::new(300.0, 300.0);
            let mut pb = PixelBuffer::new(w, h);
            art_floor::draw_floor(&mut pb, &level, cam, 0.0);
            let acc = level.palette.accent;

            let p = |x: f32, y: f32| cam + Vec2::new(x, y);

            // --- ROW 1 (y = 28): Scale (Rat), Facings, Damage Phases 1/2/3, Hit Sparks, Projectiles & Idle Dock ---
            let mut rat = Player::new(p(16.0, 28.0), WeaponKind::Gnaw);
            rat.face = Vec2::new(0.45, 0.89).normalized();
            art_rat::draw_rat(&mut pb, &rat, 0.4, cam);

            let row1 = [
                // Phase 1 clean (facing down-right)
                MowerView {
                    pos: p(56.0, 28.0),
                    facing: Vec2::from_angle(0.45),
                    state: MowerState::Patrol,
                    progress: 0.2,
                    anim: 0.5,
                    hit_flash: 0.0,
                    hp_frac: 1.0,
                    target: p(110.0, 48.0),
                    trail_start: p(56.0, 28.0),
                    dock_pos: p(56.0, 28.0),
                    bounce_count: 0,
                },
                // Phase 1 facing up-left
                MowerView {
                    pos: p(104.0, 28.0),
                    facing: Vec2::from_angle(-2.45),
                    state: MowerState::Patrol,
                    progress: 0.5,
                    anim: 1.3,
                    hit_flash: 0.0,
                    hp_frac: 0.82,
                    target: p(60.0, 10.0),
                    trail_start: p(104.0, 28.0),
                    dock_pos: p(104.0, 28.0),
                    bounce_count: 0,
                },
                // Phase 2 cracked cowl + rear oily smoke + wider exposed blades
                MowerView {
                    pos: p(152.0, 28.0),
                    facing: Vec2::from_angle(0.35),
                    state: MowerState::Patrol,
                    progress: 0.6,
                    anim: 2.1,
                    hit_flash: 0.0,
                    hp_frac: 0.48,
                    target: p(190.0, 50.0),
                    trail_start: p(152.0, 28.0),
                    dock_pos: p(152.0, 28.0),
                    bounce_count: 0,
                },
                // Phase 3 blown-off side-rear hood + glowing coils + flickering cracked visor
                MowerView {
                    pos: p(202.0, 28.0),
                    facing: Vec2::from_angle(0.0),
                    state: MowerState::Patrol,
                    progress: 0.8,
                    anim: 0.35,
                    hit_flash: 0.0,
                    hp_frac: 0.18,
                    target: p(260.0, 28.0),
                    trail_start: p(202.0, 28.0),
                    dock_pos: p(202.0, 28.0),
                    bounce_count: 0,
                },
                // Hit flash + crackling electric sparks!
                MowerView {
                    pos: p(254.0, 28.0),
                    facing: Vec2::from_angle(2.5),
                    state: MowerState::Patrol,
                    progress: 0.3,
                    anim: 1.1,
                    hit_flash: 0.88,
                    hp_frac: 0.72,
                    target: p(210.0, 48.0),
                    trail_start: p(254.0, 28.0),
                    dock_pos: p(254.0, 28.0),
                    bounce_count: 0,
                },
            ];
            for v in &row1 {
                draw_mower_ground(&mut pb, v, cam, acc);
            }
            for v in &row1 {
                draw_mower(&mut pb, v, cam, acc);
            }
            // Standalone projectile showcase + empty Charging Dock pad on right of Row 1.
            draw_blade_disc(&mut pb, p(296.0, 19.0), 0.4, cam, acc);
            draw_blade_disc(&mut pb, p(312.0, 19.0), 1.2, cam, acc);
            draw_blade_disc(&mut pb, p(328.0, 19.0), 2.1, cam, acc);
            for k in 0..4 {
                let d = Vec2::from_angle(-0.4 + k as f32 * 0.35);
                draw_grass_shrapnel(&mut pb, p(294.0 + k as f32 * 12.0, 37.0), d, cam, acc);
            }
            draw_dock_pad(&mut pb, p(368.0, 28.0), 0.8, 0.0, cam, acc);

            // --- ROW 2 (y = 84): Scan (Sweep + Lock), ChargeWindup, Charge (Trail), Ricochet ---
            let row2 = [
                // Scan early radar sweep (angled slightly upward-right, generous clearance!)
                MowerView {
                    pos: p(26.0, 86.0),
                    facing: Vec2::from_angle(-0.24),
                    state: MowerState::Scan,
                    progress: 0.25,
                    anim: 1.2,
                    hit_flash: 0.0,
                    hp_frac: 0.92,
                    target: p(88.0, 68.0),
                    trail_start: p(26.0, 86.0),
                    dock_pos: p(26.0, 86.0),
                    bounce_count: 0,
                },
                // Scan locked onto target!
                MowerView {
                    pos: p(122.0, 84.0),
                    facing: Vec2::from_angle(0.16),
                    state: MowerState::Scan,
                    progress: 0.90,
                    anim: 2.4,
                    hit_flash: 0.0,
                    hp_frac: 0.84,
                    target: p(160.0, 90.0),
                    trail_start: p(122.0, 84.0),
                    dock_pos: p(122.0, 84.0),
                    bounce_count: 0,
                },
                // ChargeWindup laser + corridor rails
                MowerView {
                    pos: p(188.0, 84.0),
                    facing: Vec2::from_angle(0.04),
                    state: MowerState::ChargeWindup,
                    progress: 0.78,
                    anim: 3.1,
                    hit_flash: 0.0,
                    hp_frac: 0.74,
                    target: p(238.0, 86.0),
                    trail_start: p(188.0, 84.0),
                    dock_pos: p(188.0, 84.0),
                    bounce_count: 0,
                },
                // Charge leaving long crisp mown & scorched trail behind it!
                MowerView {
                    pos: p(306.0, 84.0),
                    facing: Vec2::from_angle(0.0),
                    state: MowerState::Charge,
                    progress: 0.72,
                    anim: 4.0,
                    hit_flash: 0.0,
                    hp_frac: 0.60,
                    target: p(326.0, 84.0),
                    trail_start: p(254.0, 84.0),
                    dock_pos: p(254.0, 84.0),
                    bounce_count: 0,
                },
                // Ricochet wall bounce + bank-shot line!
                MowerView {
                    pos: p(362.0, 88.0),
                    facing: Vec2::from_angle(-2.45),
                    state: MowerState::Ricochet,
                    progress: 0.28,
                    anim: 2.2,
                    hit_flash: 0.0,
                    hp_frac: 0.44,
                    target: p(332.0, 58.0),
                    trail_start: p(382.0, 102.0),
                    dock_pos: p(382.0, 102.0),
                    bounce_count: 2,
                },
            ];
            for v in &row2 {
                draw_mower_ground(&mut pb, v, cam, acc);
            }
            for v in &row2 {
                draw_mower(&mut pb, v, cam, acc);
            }

            // --- ROW 3 (y = 144): FlingBlades (Windup + Volley), Overheat, Flipped (Roll + Belly-Up), Docking ---
            let row3 = [
                // FlingBlades windup spokes
                MowerView {
                    pos: p(38.0, 144.0),
                    facing: Vec2::from_angle(0.2),
                    state: MowerState::FlingBlades,
                    progress: 0.24,
                    anim: 1.5,
                    hit_flash: 0.0,
                    hp_frac: 0.58,
                    target: p(70.0, 144.0),
                    trail_start: p(38.0, 144.0),
                    dock_pos: p(38.0, 144.0),
                    bounce_count: 0,
                },
                // FlingBlades active disc & grass-shrapnel volley
                MowerView {
                    pos: p(106.0, 144.0),
                    facing: Vec2::from_angle(0.5),
                    state: MowerState::FlingBlades,
                    progress: 0.62,
                    anim: 2.7,
                    hit_flash: 0.0,
                    hp_frac: 0.52,
                    target: p(140.0, 144.0),
                    trail_start: p(106.0, 144.0),
                    dock_pos: p(106.0, 144.0),
                    bounce_count: 0,
                },
                // Overheat upright steaming + glowing red louvers + ERROR-RED LED & STOP strobe
                MowerView {
                    pos: p(172.0, 144.0),
                    facing: Vec2::from_angle(0.25),
                    state: MowerState::Overheat,
                    progress: 0.50,
                    anim: 0.1, // blink ON
                    hit_flash: 0.0,
                    hp_frac: 0.45,
                    target: p(200.0, 144.0),
                    trail_start: p(172.0, 144.0),
                    dock_pos: p(172.0, 144.0),
                    bounce_count: 0,
                },
                // Flipped turtle mid-roll
                MowerView {
                    pos: p(228.0, 144.0),
                    facing: Vec2::from_angle(0.3),
                    state: MowerState::Flipped,
                    progress: 0.10,
                    anim: 1.8,
                    hit_flash: 0.0,
                    hp_frac: 0.36,
                    target: p(258.0, 144.0),
                    trail_start: p(228.0, 144.0),
                    dock_pos: p(228.0, 144.0),
                    bounce_count: 0,
                },
                // Flipped belly-up turtle (exposed rotary blades & glowing battery core!)
                MowerView {
                    pos: p(286.0, 144.0),
                    facing: Vec2::from_angle(-0.35),
                    state: MowerState::Flipped,
                    progress: 0.50,
                    anim: 0.1,
                    hit_flash: 0.0,
                    hp_frac: 0.30,
                    target: p(316.0, 144.0),
                    trail_start: p(286.0, 144.0),
                    dock_pos: p(286.0, 144.0),
                    bounce_count: 0,
                },
                // Docking actively recharging on Charging Dock Pad!
                MowerView {
                    pos: p(358.0, 144.0),
                    facing: Vec2::from_angle(0.0),
                    state: MowerState::Docking,
                    progress: 0.65,
                    anim: 1.4,
                    hit_flash: 0.0,
                    hp_frac: 0.35,
                    target: p(350.0, 144.0),
                    trail_start: p(358.0, 144.0),
                    dock_pos: p(352.0, 144.0),
                    bounce_count: 0,
                },
            ];
            for v in &row3 {
                draw_mower_ground(&mut pb, v, cam, acc);
            }
            for v in &row3 {
                draw_mower(&mut pb, v, cam, acc);
            }

            // --- ROW 4 (y = 200): Death Explosion Sequence across 5 progress keyframes ---
            let dying_frames = [
                (44.0, 0.14),
                (120.0, 0.38),
                (198.0, 0.54),
                (276.0, 0.72),
                (354.0, 0.90),
            ];
            for &(x, prog) in &dying_frames {
                let v = MowerView {
                    pos: p(x, 200.0),
                    facing: Vec2::from_angle(0.4),
                    state: MowerState::Dying,
                    progress: prog,
                    anim: 1.2 + prog * 4.0,
                    hit_flash: 0.0,
                    hp_frac: 0.0,
                    target: p(x + 30.0, 200.0),
                    trail_start: p(x, 200.0),
                    dock_pos: p(x, 200.0),
                    bounce_count: 0,
                };
                draw_mower_ground(&mut pb, &v, cam, acc);
                draw_mower(&mut pb, &v, cam, acc);
            }

            for y in 0..pb.h {
                for x in 0..pb.w {
                    let c = pb.pixel_at(x, y);
                    out.extend_from_slice(&[c.0, c.1, c.2]);
                }
            }
        }
        std::fs::write("/tmp/ratrun_mower.ppm", out).unwrap();
    }

    #[test]
    fn mower_audit_fixes_nonfinite_and_telegraph_geometry() {
        // Non-finite and far off-screen coordinates must never panic or overflow.
        let mut pb = PixelBuffer::new(96, 96);
        let bad = MowerView {
            pos: Vec2::new(48.0, 48.0),
            facing: Vec2::new(1.0, 0.0),
            state: MowerState::ChargeWindup,
            progress: f32::NAN,
            anim: f32::INFINITY,
            hit_flash: f32::NAN,
            hp_frac: 0.0,
            target: Vec2::new(48.0, f32::NAN),
            trail_start: Vec2::new(-1e12, 1e12),
            dock_pos: Vec2::new(48.0, 48.0),
            bounce_count: 1,
        };
        draw_mower_ground(&mut pb, &bad, Vec2::ZERO, (255, 160, 40));
        draw_mower(&mut pb, &bad, Vec2::ZERO, (255, 160, 40));

        // Scan cone stays within SCAN_CONE_RANGE from pos.
        let mut scan_pb = PixelBuffer::new(220, 120);
        let scan_v = MowerView {
            pos: Vec2::new(40.0, 60.0),
            facing: Vec2::new(1.0, 0.0),
            state: MowerState::Scan,
            progress: 0.35,
            anim: 0.5,
            hit_flash: 0.0,
            hp_frac: 1.0,
            target: Vec2::new(160.0, 60.0),
            trail_start: Vec2::new(40.0, 60.0),
            dock_pos: Vec2::new(40.0, 60.0),
            bounce_count: 0,
        };
        draw_mower_ground(&mut scan_pb, &scan_v, Vec2::ZERO, (255, 160, 40));
        for x in (40 + SCAN_CONE_RANGE as usize + 3)..220 {
            assert_eq!(
                scan_pb.pixel_at(x, 60),
                (0, 0, 0),
                "scan cone overshot SCAN_CONE_RANGE at x={x}"
            );
        }

        // Flipped self-righting slam warning appears for prog >= 0.85.
        let mut early = PixelBuffer::new(96, 96);
        let mut late = PixelBuffer::new(96, 96);
        let mut fv = scan_v;
        fv.pos = Vec2::new(48.0, 48.0);
        fv.state = MowerState::Flipped;
        fv.progress = 0.50;
        draw_mower_ground(&mut early, &fv, Vec2::ZERO, (255, 160, 40));
        fv.progress = 0.92;
        draw_mower_ground(&mut late, &fv, Vec2::ZERO, (255, 160, 40));
        let mut diff = 0usize;
        for y in 0..96 {
            for x in 0..96 {
                if early.pixel_at(x, y) != late.pixel_at(x, y) {
                    diff += 1;
                }
            }
        }
        assert!(diff > 10, "expected self-righting slam warning ring at prog=0.92");
    }
}
