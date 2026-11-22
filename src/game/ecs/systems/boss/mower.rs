//! Lab boss — TURF-REAPER MK.IV (the robot mower).
//!
//! The art module [`crate::game::art_boss_mower`] is the gameplay spec: the
//! state enum, every per-state duration, damage number, hit radius helper
//! (`collision_radius`), incoming-damage multiplier (`incoming_damage_mult`)
//! and the accountant's constants are binding. This file drives the
//! transitions, picks the charge lanes and queues the art's telegraphed
//! attacks as shared [`BossHazard`]s and [`BossProjectile`]s.
//!
//! ## The fight the owner asked for — "make the rat steer the mower"
//!
//! The read is **scan → lock → charge → impact → exposed belly**:
//!
//! 1. `Patrol` holds the rat at mid-range (`45..85 px`) and circles.
//! 2. `Scan` sweeps the lidar cone and locks at `progress = 0.70` — but only
//!    through open air (`los_clear`), so a wall, prop or gate breaks the lock
//!    and the mower vents instead of charging.
//! 3. `ChargeWindup` tracks the rat until `progress = 0.45`, then freezes a
//!    lane that ends on a real solid (arena wall, solid prop or closed gate)
//!    *within a screenful of the rat*. Aiming goes through [`select_lane`],
//!    which only accepts a lane when the run-up is at least
//!    [`CHARGE_MIN_TRAVEL`] and at most the art's `135 px`, when the chassis
//!    can stand at the impact point, and when the rat has an
//!    [`IMPACT_SIDE_LANE`] escape lane beside that point. Anything else is
//!    refused: the mower repositions (`Patrol`) rather than starting an attack
//!    it cannot finish where it stands.
//! 4. `Charge` runs the frozen lane at the art's speed; a wall impact is the
//!    **home move**: phase 1 flips it onto its back (`Flipped`, 2× damage),
//!    phases 2 and 3 chain the design's shortened `Ricochet` (1 / 2 bounces,
//!    `0.40 s` rebound pause) and end flipped. A clean run that touches nothing
//!    vents (`Overheat`, 1.5× damage) instead.
//! 5. Two HP thresholds (`0.66`, `0.33`) retreat to the dock (`Docking`) once
//!    each, healing up to 12% unless the rat short-circuits the recharge.
//!
//! ## State machine (art timings; phases at hp > 0.66 / 0.33..0.66 / ≤ 0.33)
//!
//! | state | duration | telegraph / active window | damage |
//! |-------|----------|---------------------------|--------|
//! | `Patrol` | 2.40 / 2.00 / 1.60 s | none | body contact 14 |
//! | `Scan` | 1.35 / 1.10 / 0.90 s | lidar cone 72 px, ±0.42 rad, lock at 0.70 | 0 (sensor) |
//! | `ChargeWindup` | 0.85 / 0.72 / 0.60 s | aim freezes at 0.45, ram queued there | body contact |
//! | `Charge` | 0.55–0.95 s (≤135 px) | corridor 12.5 px half-width | 26 ram + 4 scorch |
//! | `Ricochet` | 0.40 s pause + 0.45–0.75 s leg | dashed rebound preview + 12.5 px corridor | 22 |
//! | `FlingBlades` | 1.80 s | deck aura from 0.63 s | 18 deck + 16 disc |
//! | `Overheat` | 2.20 s (1.20 s after a missed scan) | venting (punish window) | 0 |
//! | `Flipped` | 3.00 / 2.60 / 2.30 s | wheels up (2× damage) | 10 self-right slam |
//! | `Docking` | 4.00 s | dock pad + tether, shock ring at 0.65 | 12 shock |
//! | `Dying` | 2.40 s | death blast ring 28 px (art) | 18 (framework: see note) |
//!
//! ## Deferred by the design review (kept, but out of the rotation)
//!
//! `shrapnel fans` and `bouncing blade discs` are implemented behind
//! [`DEFER_SHRAPNEL`] / [`DEFER_BOUNCING_DISCS`], and the whole `FlingBlades`
//! state is out of the attack rotation behind [`DEFER_FLING_BLADES`], so the
//! first playable build is charge-and-flip only. The state, its telegraph and
//! its deck aura are fully implemented and unit-tested; flipping one constant
//! puts it back in the rotation.
//!
//! ## Cues
//!
//! The controller queues [`BossCue`]s; the shared system drains them into the
//! matching `Sfx` and `vfx::emit_*` call. The art's beats are cued on entry
//! (`Scan`, `ChargeWindup`, `Charge`, `Ricochet`/`Flipped` = wall clang, `Overheat`,
//! `Docking`), the scan's snap-lock when it fires, and the meltdown in `Dying`.
//!
//! ## Determinism
//!
//! One rng draw per `Patrol` entry (which way it circles) and no others unless
//! the deferred FlingBlades rotation is enabled. Nothing allocates per tick:
//! [`select_lane`] walks [`AIM_OFFSETS`] on the stack and hazards go into the
//! boss's pre-reserved ring.

use super::*;
use crate::game::ecs::systems::projectiles::{los_clear, wall_hit};

// ============================================================================
// Art timings, speeds and damage (src/game/art_boss_mower.rs)
// ============================================================================

/// Domestic-cruise speed per phase (art: 24 / 30 / 36 px/s).
const PATROL_SPEED: [f32; 3] = [24.0, 30.0, 36.0];
/// Cruise beat per phase (art: `1.6 s..2.4 s`, shorter later).
const PATROL_DURATION: [f32; 3] = [2.40, 2.00, 1.60];
/// The mid-range band patrol holds the rat at (art: `45..85 px`).
const PATROL_BAND: [f32; 2] = [45.0, 85.0];

/// Scan beat per phase (art: 1.35 / 1.10 / 0.90 s).
const SCAN_DURATION: [f32; 3] = [1.35, 1.10, 0.90];
/// `progress` where the sweep snaps to a lock (art: sweep `0.0..0.70`).
const SCAN_LOCK_PROGRESS: f32 = 0.70;
/// Slow creep while scanning (art: "Stops or slows to `8 px/s`").
const SCAN_CREEP_SPEED: f32 = 8.0;

/// Charge telegraph per phase (art: 0.85 / 0.72 / 0.60 s).
const WINDUP_DURATION: [f32; 3] = [0.85, 0.72, 0.60];
/// The aim freezes here so the rat can sidestep the lane (art: 0.45).
const WINDUP_LOCK_PROGRESS: f32 = 0.45;

/// Ram speed per phase (art: 165 / 190 / 215 px/s).
const CHARGE_SPEED: [f32; 3] = [165.0, 190.0, 215.0];
/// Longest charge run (art: max travel `135 px`).
const CHARGE_MAX_TRAVEL: f32 = 135.0;
/// Shortest run worth committing to (a charge shorter than this is a nudge).
const CHARGE_MIN_TRAVEL: f32 = 24.0;
/// Charge duration band (art: `0.55 s..0.95 s`, distance-bounded).
const CHARGE_DURATION: [f32; 2] = [0.55, 0.95];
/// Direct ram damage (art: 26).
const CHARGE_DAMAGE: f32 = 26.0;
/// Hot blade-wake left behind the charge (art: 4, half-width 9 px, 0.6 s).
const SCORCH_DAMAGE: f32 = 4.0;
const SCORCH_HALF_WIDTH: f32 = 9.0;
const SCORCH_LINGER: f32 = 0.60;

/// Wall-impact hitstop before a rebound leg (design: `0.40 s`, art: 0.18 s).
const RICOCHET_PAUSE: f32 = 0.40;
/// Completed wall bounces per phase (design build: 0 / 1 / 2; art: 0 / 3 / 4).
const RICOCHET_BOUNCES: [u8; 3] = [0, 1, 2];
/// Rebound leg speed per phase (art: `155..185 px/s`).
const RICOCHET_SPEED: [f32; 3] = [155.0, 170.0, 185.0];
/// Per-leg duration band (art: `0.45 s..0.75 s`).
const RICOCHET_LEG: [f32; 2] = [0.45, 0.75];
/// Rebound damage (art: 22 — under the primary ram because legs chain).
const RICOCHET_DAMAGE: f32 = 22.0;

/// Open floor required beside an impact point (owner: "leave a 32 px side lane").
const IMPACT_SIDE_LANE: f32 = 32.0;
/// The impact marker has to sit within a screenful of the rat (owner: a
/// 135 px charge is wider than the view, so the flip must land on screen).
const IMPACT_NEAR_RAT: f32 = 88.0;
/// Aim steering offsets tried around the rat direction, centre-out.
const AIM_OFFSETS: [f32; 7] = [0.0, 0.22, -0.22, 0.45, -0.45, 0.68, -0.68];

/// `FlingBlades` out of the rotation until charge-and-flip is good (design).
const DEFER_FLING_BLADES: bool = true;
/// Grass-clipping fans deferred (design: "defer shrapnel fans").
const DEFER_SHRAPNEL: bool = true;
/// Wall-bouncing blade discs deferred (design: "and bouncing blade discs").
const DEFER_BOUNCING_DISCS: bool = true;
/// Volley beat (art: `1.80 s` = 0.63 s telegraph + 1.00 s firing + spin-down).
const FLING_DURATION: f32 = 1.80;
/// Telegraph end (art: `progress 0.0..0.35`).
const FLING_TELEGRAPH: f32 = 0.35;
/// Firing end (art: `progress 0.35..0.90`).
const FLING_ACTIVE_END: f32 = 0.90;
/// Close-range deck aura (art: radius `15.5 px`, `18` damage).
const FLING_DECK_RADIUS: f32 = 15.5;
const FLING_DECK_DAMAGE: f32 = 18.0;
/// Blade-disc throw speed (art: `72 px/s`).
const DISC_SPEED: f32 = 72.0;
/// Grass-shrapnel throw speed (art: `92 px/s`).
const SHRAPNEL_SPEED: f32 = 92.0;
/// Half a disc wave's spiral step (art: `+22.5°` per wave).
const DISC_SPIRAL: f32 = std::f32::consts::FRAC_PI_8;

/// Thermal shutdown (art: `2.2 s`, `1.2 s` after a missed scan).
const OVERHEAT_DURATION: f32 = 2.20;
const OVERHEAT_MISSED: f32 = 1.20;

/// Turtle window per phase (art: 3.00 / 2.60 / 2.30 s).
const FLIPPED_DURATION: [f32; 3] = [3.00, 2.60, 2.30];
/// Roll-back begins (art: `0.85`), the slam lands at `0.92..1.00`.
const FLIP_SLAM_START: f32 = 0.85;
const FLIP_SLAM_AT: f32 = 0.92;
/// Self-righting thud (art: radius `16 px`, `10` damage, knockback).
const FLIP_SLAM_RADIUS: f32 = 16.0;
const FLIP_SLAM_DAMAGE: f32 = 10.0;

/// Dock retreat (art: up to `4.0 s`, transit `48 px/s`).
const DOCK_DURATION: f32 = 4.00;
const DOCK_SPEED: f32 = 48.0;
/// How close the prongs have to be before the induction channel starts.
const DOCK_ARRIVE: f32 = 6.0;
/// Recharge (art: `4%` max HP per second, `12%` total).
const DOCK_HEAL_PER_S: f32 = 0.04;
const DOCK_HEAL_MAX: f32 = 0.12;
/// Damage that short-circuits the recharge (art: `120` burst during charging).
const DOCK_INTERRUPT: f32 = 120.0;
/// The rat standing on the pad's kill switch short-circuits it too (art).
const DOCK_SWITCH_RADIUS: f32 = 11.0;
/// Defensive surge ring (art: `r = 26 px`, `12` damage, `0.45 s` of warning).
const DOCK_SHOCK: [f32; 2] = [8.0, 26.0];
const DOCK_SHOCK_PROGRESS: f32 = 0.65;
const DOCK_SHOCK_DAMAGE: f32 = 12.0;
const SHOCK_TELEGRAPH: f32 = 0.45;

/// The art's `Docking` body contact (`12`).
const DOCK_CONTACT: f32 = 12.0;
/// The art's last-gasp meltdown: `18` damage inside `DEATH_BLAST_RADIUS`.
const DEATH_BLAST_DAMAGE: f32 = 18.0;
/// The art's detonation window in `Dying` (`progress 0.42..0.52`).
const DEATH_BLAST_AT: f32 = 0.42;
const DEATH_BLAST_END: f32 = 0.52;

/// [`Boss::flags`] layout.
/// This beat's one-shot latch already fired (lock-on, death blast).
const F_LATCH: u32 = 1 << 0;
/// The dock recharge channel is live.
const F_DOCK_CHARGING: u32 = 1 << 1;

/// [`Boss::scratch`] layout.
/// HP when the dock channel started.
const S_DOCK_BASE: usize = 0;
/// The circling direction (`+1`/`-1`), one rng draw per patrol entry.
const S_CIRCLE: usize = 1;

// ============================================================================
// Controller
// ============================================================================

/// The TURF-REAPER controller (zero-sized; the state lives on the [`Boss`]).
pub struct MowerController;

impl BossController for MowerController {
    fn kind(&self) -> BossKind {
        BossKind::Mower
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = mower_state(boss);
        // The rat outside the boss room makes the mower roam (the flow field
        // routes it out through the door); the flag is refreshed every tick like
        // the Sweeper's, so a rat that walks back in is fought normally. It used
        // to be latched at spawn, which is what starved this fight of attacks
        // (see `tick_patrol`).
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);
        // A phase threshold is a set piece: retreat to the dock and recharge.
        // `Boss::phase` only ever increases, so this fires at 0.66 and 0.33
        // and nowhere else.
        if out.phase_changed && !matches!(boss.state, BossState::Mower(MowerState::Dying)) {
            enter_docking(boss);
        }

        match boss.state {
            BossState::Mower(MowerState::Patrol) => tick_patrol(ctx, boss),
            BossState::Mower(MowerState::Scan) => tick_scan(ctx, boss, out),
            BossState::Mower(MowerState::ChargeWindup) => tick_charge_windup(ctx, boss),
            BossState::Mower(MowerState::Charge) => tick_charge(ctx, boss),
            BossState::Mower(MowerState::Ricochet) => tick_ricochet(ctx, boss),
            BossState::Mower(MowerState::FlingBlades) => tick_fling(ctx, boss, out),
            BossState::Mower(MowerState::Overheat) => {
                tick_overheat(ctx, boss);
            }
            BossState::Mower(MowerState::Flipped) => tick_flipped(ctx, boss),
            BossState::Mower(MowerState::Docking) => tick_docking(ctx, boss),
            BossState::Mower(MowerState::Dying)
            | BossState::Cat(_)
            | BossState::Broom(_)
            | BossState::Mangle(_)
            | BossState::NightEdition(_)
            | BossState::Magpie(_)
            | BossState::HorseshoeCrab(_)
            | BossState::MegaRat(_)
            | BossState::Hawk(_) => {}
        }

        // The art owns the multiplier; set it after the transitions so the
        // punish windows are live on the tick they open.
        boss.damage_mult = art_boss_mower::incoming_damage_mult(mower_state(boss));

        // One cue per state entry, fired on the tick the transition happened so
        // the sound lands with the first frame of the new pose.
        let after = mower_state(boss);
        if after != before {
            if let Some(cue) = state_cue(after) {
                out.cue(cue);
            }
        }
    }

    /// The meltdown's last gasp: the art's `18`-damage, `28 px` blast, plus the
    /// death cue. The framework runs this for every tick of `Dying` and reaps
    /// the boss when the beat ends.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_LATCH == 0 {
            boss.flags |= F_LATCH;
            out.cue(BossCue::MowerDeath);
            let duration = boss.duration;
            let warn = (duration * DEATH_BLAST_AT).max(0.0);
            let active = (duration * (DEATH_BLAST_END - DEATH_BLAST_AT)).max(0.0);
            let blast = BossHazard::new(
                HazardShape::Circle {
                    center: boss.pos,
                    radius: art_boss_mower::DEATH_BLAST_RADIUS,
                },
                warn,
                active,
                DEATH_BLAST_DAMAGE,
            )
            .with_knockback(180.0)
            .with_enemies();
            let _ = boss.push_hazard(blast);
        }
    }
}

/// The sound/effect cue for entering a state (the scan's snap-lock is a
/// within-state event and is cued in [`tick_scan`]).
#[inline]
fn state_cue(state: MowerState) -> Option<BossCue> {
    match state {
        MowerState::Scan => Some(BossCue::MowerScan),
        MowerState::ChargeWindup => Some(BossCue::MowerWindup),
        MowerState::Charge => Some(BossCue::MowerCharge),
        // A wall impact either banks into a rebound leg or flips the mower.
        MowerState::Ricochet => Some(BossCue::MowerWallImpact),
        MowerState::Flipped => Some(BossCue::MowerFlip),
        MowerState::Overheat => Some(BossCue::MowerOverheat),
        MowerState::Docking => Some(BossCue::MowerDock),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

fn tick_patrol(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 0.0 {
        boss.facing = to.normalized();
    }
    // There is no "off-stage" freeze. This used to hold the mower in `Patrol`
    // forever (resetting its beat) whenever the *rat* was outside the boss
    // room's rect — and the rat's 32 px orbit crosses that rect constantly, so
    // measured on the bot ladder **20 of 21 Expert mower fights landed zero
    // attacks** and lasted 51-106 s as a stationary punching bag. The arena rect
    // is not a wall (the mower's own collision is), so the fight simply runs
    // wherever the two of them are.
    let dist = to.len();
    let dir = if dist < PATROL_BAND[0] {
        -boss.facing
    } else if dist > PATROL_BAND[1] {
        // Chase through the shared steering: the straight bearing when the rat
        // is in the open, the large-class flow field when a wall is between
        // (a charge-and-flip boss that cannot find the rat has no fight).
        ctx.seek(boss)
    } else {
        // Hold mid-range: circle one way for this whole beat.
        boss.facing.perp() * boss.scratch[S_CIRCLE]
    };
    move_boss(ctx, boss, dir, PATROL_SPEED[slot]);
    if boss.progress() < 1.0 {
        return;
    }
    if !DEFER_FLING_BLADES && ctx.rng.random::<f32>() < 0.35 {
        enter_fling(boss);
    } else {
        enter(boss, MowerState::Scan, SCAN_DURATION[slot]);
    }
}

fn tick_scan(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if to.len_sq() > 0.0 {
        boss.facing = to.normalized();
    }
    move_boss(ctx, boss, boss.facing, SCAN_CREEP_SPEED);

    // One lock decision per scan, at the art's snap point. A solid wall, prop
    // or gate breaks the lidar line: no lock, no charge.
    if boss.bounce_count == 0 && boss.progress() >= SCAN_LOCK_PROGRESS {
        let locked = dist <= art_boss_mower::SCAN_CONE_RANGE
            && los_clear(ctx.level, boss.pos, ctx.player_pos, ctx.player_radius);
        boss.bounce_count = if locked { 1 } else { 2 };
        if locked {
            boss.target = ctx.player_pos;
            out.cue(BossCue::MowerLockOn);
        }
    }
    if boss.progress() < 1.0 {
        return;
    }
    if boss.bounce_count == 1 {
        if let Some(lane) = select_lane(ctx, boss) {
            boss.facing = lane.dir;
            boss.target = lane.contact;
            enter(boss, MowerState::ChargeWindup, WINDUP_DURATION[slot]);
            return;
        }
        // No chargeable lane here: reposition instead of starting a move that
        // would not fit where the mower stands.
        enter_patrol(ctx, boss);
        return;
    }
    // The rat broke the lock: vent (the art's `1.2 s` punish window).
    enter(boss, MowerState::Overheat, OVERHEAT_MISSED);
}

fn tick_charge_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    if boss.progress() < WINDUP_LOCK_PROGRESS {
        // Track the rat so it can steer the mower toward a bumper...
        if let Some(lane) = select_lane(ctx, boss) {
            boss.facing = lane.dir;
            boss.target = lane.contact;
        }
    } else if boss.bounce_count == 0 {
        // ...then freeze, exactly at the art's lock progress, so the rat can
        // sidestep. Re-selecting here keeps the committed lane coherent with
        // the dashed line the player was just shown.
        boss.bounce_count = 1;
        if let Some(lane) = select_lane(ctx, boss) {
            boss.facing = lane.dir;
            boss.target = lane.contact;
        }
        let mut travel = boss.target.dist(boss.pos);
        if !(CHARGE_MIN_TRAVEL..=CHARGE_MAX_TRAVEL).contains(&travel) {
            // No lane was ever committed (or it lost its target): aim at the
            // rat for a run the art allows, and vent when it touches nothing.
            let to = ctx.player_pos - boss.pos;
            let aim = to.normalized();
            travel = to.len().clamp(CHARGE_MIN_TRAVEL, CHARGE_MAX_TRAVEL);
            boss.facing = aim;
            boss.target = boss.pos + aim * travel;
        }
        let warn = (boss.duration * (1.0 - WINDUP_LOCK_PROGRESS)).max(0.0);
        let charge = charge_time(travel, slot);
        let from = boss.pos;
        let to = from + boss.facing * travel;
        // The ram is queued here so its own warning window covers the rest of
        // the telegraph and the art's corridor rails are exact.
        let ram = BossHazard::new(
            HazardShape::Corridor {
                from,
                to,
                half_width: art_boss_mower::CHARGE_CORRIDOR_HALF_WIDTH,
            },
            warn,
            charge,
            CHARGE_DAMAGE,
        )
        .with_knockback(150.0)
        .with_enemies();
        let _ = boss.push_hazard(ram);
        // The mown wake stays hot for a beat after the mower has passed.
        let scorch = BossHazard::new(
            HazardShape::Corridor {
                from,
                to,
                half_width: SCORCH_HALF_WIDTH,
            },
            warn + charge,
            SCORCH_LINGER,
            SCORCH_DAMAGE,
        );
        let _ = boss.push_hazard(scorch);
    }
    if boss.progress() < 1.0 {
        return;
    }
    let travel = boss.target.dist(boss.pos);
    boss.pounce_origin = boss.pos;
    enter(boss, MowerState::Charge, charge_time(travel, slot));
}

fn tick_charge(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let moved = move_boss(ctx, boss, boss.facing, CHARGE_SPEED[slot]);
    if !moved {
        // Wall impact — the home move.
        impact(ctx, boss);
        return;
    }
    if boss.progress() >= 1.0 {
        // Ran the whole lane over open floor: the motors stall (punish).
        enter(boss, MowerState::Overheat, OVERHEAT_MISSED);
    }
}

fn tick_ricochet(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let pause = (RICOCHET_PAUSE / boss.duration).clamp(0.0, 1.0);
    if boss.progress() < pause {
        // Hitstop: the art paints the next dashed rebound line here.
        return;
    }
    let moved = move_boss(ctx, boss, boss.facing, RICOCHET_SPEED[slot]);
    if !moved || boss.progress() >= 1.0 {
        boss.pounce_origin = boss.pos;
        let completed = boss.bounce_count;
        if !begin_leg(ctx, boss, completed) {
            enter(boss, MowerState::Flipped, FLIPPED_DURATION[slot]);
        }
    }
}

fn tick_fling(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let waves: u8 = if boss.phase >= 3 { 3 } else { 2 };
    let discs: usize = if boss.phase >= 2 { 4 } else { 3 };
    let step = if waves >= 3 { 0.15 } else { 0.25 };
    if boss.bounce_count < waves {
        let due = FLING_TELEGRAPH + 0.05 + step * f32::from(boss.bounce_count);
        if boss.progress() >= due {
            // Each spiral wave starts `22.5°` past the last (art).
            let base = boss.facing.angle() + f32::from(boss.bounce_count) * DISC_SPIRAL;
            for i in 0..discs {
                let angle = base + i as f32 * std::f32::consts::TAU / discs as f32;
                let mut spawn = BossProjectileSpawn::new(
                    BossProjectileKind::BladeDisc,
                    boss.pos,
                    Vec2::from_angle(angle) * DISC_SPEED,
                );
                if DEFER_BOUNCING_DISCS {
                    spawn.bounces = 0;
                }
                out.projectiles.push(spawn);
            }
            if !DEFER_SHRAPNEL {
                let aim = (ctx.player_pos - boss.pos).normalized();
                for k in -1..=1 {
                    let dir = aim.rotate(k as f32 * 0.28);
                    out.projectiles.push(BossProjectileSpawn::new(
                        BossProjectileKind::GrassShrapnel,
                        boss.pos,
                        dir * SHRAPNEL_SPEED,
                    ));
                }
            }
            boss.bounce_count += 1;
        }
    }
    if boss.progress() >= 1.0 {
        enter(boss, MowerState::Overheat, OVERHEAT_DURATION);
    }
}

fn tick_overheat(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Blades disengaged: the body blocks but deals no contact damage (art).
    boss.no_contact();
    if boss.progress() >= 1.0 {
        enter_patrol(ctx, boss);
    }
}

fn tick_flipped(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Wheels up: the art gives `Flipped` **0** body damage (the belly is the
    // punish window; only the self-righting thud at the end hurts).
    boss.no_contact();
    if boss.bounce_count == 0 && boss.progress() >= FLIP_SLAM_START {
        boss.bounce_count = 1;
        let duration = boss.duration;
        let warn = (duration * (FLIP_SLAM_AT - FLIP_SLAM_START)).max(0.0);
        let active = (duration * (1.0 - FLIP_SLAM_AT)).max(0.0);
        // The self-righting thud pushes greedy players off the belly.
        let slam = BossHazard::new(
            HazardShape::Circle {
                center: boss.pos,
                radius: FLIP_SLAM_RADIUS,
            },
            warn,
            active,
            FLIP_SLAM_DAMAGE,
        )
        .with_knockback(140.0);
        let _ = boss.push_hazard(slam);
    }
    if boss.progress() >= 1.0 {
        let slot = phase_slot(boss);
        enter(boss, MowerState::Patrol, PATROL_DURATION[slot]);
        boss.scratch[S_CIRCLE] = 1.0;
    }
}

fn tick_docking(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The art's `Docking` body contact is `12` (the exposed prongs).
    boss.set_contact_damage(DOCK_CONTACT);
    // The channel's own memory lives in the controller's scratch, not on the
    // art's `trail_start`.
    let charging = boss.flags & F_DOCK_CHARGING != 0;
    if !charging {
        let to_dock = boss.dock - boss.pos;
        if to_dock.len() > DOCK_ARRIVE {
            if to_dock.len_sq() > 0.0 {
                boss.facing = to_dock.normalized();
            }
            move_boss(ctx, boss, to_dock, DOCK_SPEED);
        } else {
            boss.scratch[S_DOCK_BASE] = boss.hp;
            boss.flags |= F_DOCK_CHARGING;
        }
    } else {
        let base = boss.scratch[S_DOCK_BASE];
        let cap = (base + DOCK_HEAL_MAX * boss.max_hp).min(boss.max_hp);
        if boss.hp < cap {
            boss.hp = (boss.hp + DOCK_HEAL_PER_S * boss.max_hp * ctx.dt).min(cap);
        }
        // Short circuit: enough burst damage, or the rat reaching the kill
        // switch on the pad, cancels the recharge into a long vent.
        let burst = base - boss.hp >= DOCK_INTERRUPT;
        let switch = ctx.player_pos.dist(boss.dock) <= DOCK_SWITCH_RADIUS;
        if burst || switch {
            enter(boss, MowerState::Overheat, OVERHEAT_DURATION);
            return;
        }
    }
    if boss.bounce_count == 0 && boss.progress() >= DOCK_SHOCK_PROGRESS {
        boss.bounce_count = 1;
        // One defensive surge ring around the pad, telegraphed by gold arcs.
        let surge = BossHazard::new(
            HazardShape::Annulus {
                center: boss.dock,
                inner: DOCK_SHOCK[0],
                outer: DOCK_SHOCK[1],
            },
            SHOCK_TELEGRAPH,
            0.12,
            DOCK_SHOCK_DAMAGE,
        )
        .with_knockback(120.0);
        let _ = boss.push_hazard(surge);
    }
    if boss.progress() >= 1.0 {
        enter_patrol(ctx, boss);
    }
}

// ============================================================================
// Transitions
// ============================================================================

#[inline]
fn phase_slot(boss: &Boss) -> usize {
    (boss.phase.clamp(1, 3) - 1) as usize
}

#[inline]
fn mower_state(boss: &Boss) -> MowerState {
    match boss.state {
        BossState::Mower(s) => s,
        _ => MowerState::Patrol,
    }
}

/// Enter an art state and clear the per-state latch (`bounce_count`).
#[inline]
fn enter(boss: &mut Boss, state: MowerState, duration: f32) {
    boss.set_state(BossState::Mower(state), duration);
    boss.bounce_count = 0;
}

fn enter_patrol(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    enter(boss, MowerState::Patrol, PATROL_DURATION[slot]);
    // Circle the rat one way or the other for this whole beat (the boss's own
    // rng stream, one draw per entry).
    boss.scratch[S_CIRCLE] = if ctx.rng.random::<f32>() < 0.5 { 1.0 } else { -1.0 };
}

fn enter_docking(boss: &mut Boss) {
    enter(boss, MowerState::Docking, DOCK_DURATION);
    // Not charging yet: the transit to the pad runs first.
    boss.scratch[S_DOCK_BASE] = boss.hp;
    boss.flags &= !F_DOCK_CHARGING;
}

fn enter_fling(boss: &mut Boss) {
    enter(boss, MowerState::FlingBlades, FLING_DURATION);
    // The deck aura is live for the firing window; braking in place means the
    // circle never moves after it is queued.
    let warn = FLING_DURATION * FLING_TELEGRAPH;
    let active = FLING_DURATION * (FLING_ACTIVE_END - FLING_TELEGRAPH);
    let deck = BossHazard::new(
        HazardShape::Circle {
            center: boss.pos,
            radius: FLING_DECK_RADIUS,
        },
        warn,
        active,
        FLING_DECK_DAMAGE,
    )
    .with_knockback(80.0)
    .with_enemies();
    let _ = boss.push_hazard(deck);
}

/// Charge time for a committed run (art: distance-bounded, `0.55..0.95 s`).
#[inline]
fn charge_time(travel: f32, slot: usize) -> f32 {
    (travel / CHARGE_SPEED[slot]).clamp(CHARGE_DURATION[0], CHARGE_DURATION[1])
}

/// A wall impact: bounce (phases 2 and 3, up to the design's count) or flip.
fn impact(ctx: &BossContext<'_>, boss: &mut Boss) {
    boss.pounce_origin = boss.pos;
    if begin_leg(ctx, boss, 0) {
        return;
    }
    let slot = phase_slot(boss);
    enter(boss, MowerState::Flipped, FLIPPED_DURATION[slot]);
}

/// Queue the next rebound leg after `completed` bounces. Returns false when the
/// phase's bounce budget is spent or there is nowhere to rebound to, which is
/// when the mower ends up on its back.
fn begin_leg(ctx: &BossContext<'_>, boss: &mut Boss, completed: u8) -> bool {
    let slot = phase_slot(boss);
    if completed >= RICOCHET_BOUNCES[slot] {
        return false;
    }
    let Some(leg) = rebound(ctx, boss) else {
        return false;
    };
    let leg_time = (leg.travel / RICOCHET_SPEED[slot]).clamp(RICOCHET_LEG[0], RICOCHET_LEG[1]);
    let ram = BossHazard::new(
        HazardShape::Corridor {
            from: boss.pos,
            to: leg.contact,
            half_width: art_boss_mower::CHARGE_CORRIDOR_HALF_WIDTH,
        },
        RICOCHET_PAUSE,
        leg_time,
        RICOCHET_DAMAGE,
    )
    .with_knockback(140.0)
    .with_enemies();
    let _ = boss.push_hazard(ram);
    boss.facing = leg.dir;
    boss.target = leg.contact;
    enter(boss, MowerState::Ricochet, RICOCHET_PAUSE + leg_time);
    boss.bounce_count = completed + 1;
    true
}

/// The next bounce leg after a wall impact: reflect off the surface normal
/// (axis test, since the ram stopped on an axis-aligned tile face) and look for
/// a solid to bank into.
fn rebound(ctx: &BossContext<'_>, boss: &Boss) -> Option<Lane> {
    let radius = art_boss_mower::BODY_RADIUS;
    let dir = boss.facing.normalized();
    // Step back off the wall face we are touching so the probes are sampled
    // from open floor (`segment_hit` reports t = 0 for a touching start).
    let origin = boss.pos - dir * 1.0;
    let step = radius + 2.0;
    let blocked_x = !ctx.footprint_fits(boss.pos + Vec2::new(dir.x.signum() * step, 0.0), radius);
    let blocked_y = !ctx.footprint_fits(boss.pos + Vec2::new(0.0, dir.y.signum() * step), radius);
    let mut dirs = [dir; 3];
    let (mut n, mut x, mut y) = (0usize, 1.0f32, 1.0f32);
    if blocked_x {
        x = -1.0;
    }
    if blocked_y {
        y = -1.0;
    }
    dirs[n] = Vec2::new(dir.x * x, dir.y * y);
    n += 1;
    if blocked_x && !blocked_y {
        dirs[n] = Vec2::new(-dir.x, dir.y);
        n += 1;
    } else if blocked_y && !blocked_x {
        dirs[n] = Vec2::new(dir.x, -dir.y);
        n += 1;
    }
    for dir in dirs.iter().take(n) {
        let Some(lane) = probe_lane(ctx, origin, *dir) else {
            continue;
        };
        if lane.wall {
            return Some(lane);
        }
    }
    None
}

// ============================================================================
// Charge lanes
// ============================================================================

/// A charge lane: where it starts pointing, where it stops and why.
#[derive(Clone, Copy, Debug)]
struct Lane {
    /// Unit direction of the run.
    dir: Vec2,
    /// World position of the chassis centre when it stops (the art's `target`).
    contact: Vec2,
    /// Distance the chassis centre travels.
    travel: f32,
    /// True when the run ends on an arena wall, solid prop or shut gate
    /// (`false` = a clean run over open floor that only vents the motors).
    wall: bool,
    /// True when the impact point sits within a screenful of the rat.
    near_rat: bool,
}

/// Probe one direction for a chargeable lane from `origin`.
///
/// Refused when the run is too short to commit to, when the chassis cannot
/// stand at the impact point, or when the rat has no
/// [`IMPACT_SIDE_LANE`]-wide escape lane beside it.
fn probe_lane(ctx: &BossContext<'_>, origin: Vec2, dir: Vec2) -> Option<Lane> {
    let radius = art_boss_mower::BODY_RADIUS;
    let dir = dir.normalized();
    if dir.len_sq() <= 0.0 {
        return None;
    }
    let ray_len = CHARGE_MAX_TRAVEL + radius;
    let hit = wall_hit(ctx.level, origin, origin + dir * ray_len, radius)
        .map(|t| t * ray_len)
        .filter(|d| *d <= CHARGE_MAX_TRAVEL);
    let wall = hit.is_some();
    let travel = hit.unwrap_or(CHARGE_MAX_TRAVEL);
    if travel < CHARGE_MIN_TRAVEL {
        return None;
    }
    let contact = origin + dir * travel;
    // The chassis has to fit where it lands (`- 0.5` keeps the exact touch
    // point off the collider's epsilon) and the rat needs a side lane out of
    // the corridor.
    let stand = contact - dir * 0.5;
    let side = dir.perp();
    if !ctx.attack_fits(stand, radius, side, IMPACT_SIDE_LANE)
        && !ctx.attack_fits(stand, radius, -side, IMPACT_SIDE_LANE)
    {
        return None;
    }
    Some(Lane {
        dir,
        contact,
        travel,
        wall,
        near_rat: contact.dist(ctx.player_pos) <= IMPACT_NEAR_RAT,
    })
}

/// Pick the lane the mower commits to: the rat direction, steered by a few
/// small offsets so the rat can walk the mower into a bumper.
///
/// Preference order (all deterministic, first match wins):
/// 1. a lane that ends on a solid *within a screenful of the rat* — the home
///    move, and the one the player has to engineer;
/// 2. any lane that ends on a solid;
/// 3. an open-floor run at the rat (no flip, but a threat to dodge).
fn select_lane(ctx: &BossContext<'_>, boss: &Boss) -> Option<Lane> {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() <= 1.0 {
        return None;
    }
    let base = to.normalized();
    for pass in 0..3u8 {
        for offset in AIM_OFFSETS {
            let dir = base.rotate(offset);
            let Some(lane) = probe_lane(ctx, boss.pos, dir) else {
                continue;
            };
            // It has to be a threat, not decoration: the rat must be ahead of
            // the mower and inside (or just behind) the swept corridor.
            let rel = ctx.player_pos - boss.pos;
            let along = rel.dot(lane.dir);
            let across = rel.dot(lane.dir.perp()).abs();
            if along <= 0.0
                || across > art_boss_mower::CHARGE_CORRIDOR_HALF_WIDTH + PLAYER_RADIUS
            {
                continue;
            }
            let take = match pass {
                0 => lane.wall && lane.near_rat,
                1 => lane.wall,
                _ => !lane.wall,
            };
            if take {
                return Some(lane);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! Narrow tests for the TURF-REAPER fight.
    //!
    //! `cargo test boss::` runs these with the framework's suite. Every test
    //! drives the shared boss system through `begin_tick` → `run_system_cached`
    //! → `end_tick`, so hazards, contact and the death hand-off are the real
    //! ones. The only shim is the rat's i-frame clock (the player system does
    //! not run here).

    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TILE_F};
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    /// Room sizes used to build controlled arenas inside the Lab boss room.
    const ROOM: (i32, i32) = (12, 10);
    const BIG_ROOM: (i32, i32) = (20, 18);
    const TINY_ROOM: (i32, i32) = (3, 3);

    // ---- harness ---------------------------------------------------------

    fn lab_world(seed: u64) -> World {
        let mut w = World::new(Theme::Lab, seed);
        // Campaign slot 2 is the Lab, i.e. the mower.
        w.level_index = 2;
        w
    }

    fn boss_of(w: &mut World) -> Option<Boss> {
        let mut q = w.ecs.query::<&Boss>();
        q.iter(&w.ecs).next().cloned()
    }

    fn with_boss(w: &mut World, mut f: impl FnMut(&mut Boss)) {
        let mut q = w.ecs.query::<&mut Boss>();
        for mut b in q.iter_mut(&mut w.ecs) {
            f(&mut b);
        }
    }

    fn spawn_mower(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the mower");
        assert_eq!(boss.kind, BossKind::Mower);
        boss
    }

    /// One tick of the shared boss system, plus the rat's i-frame decay that
    /// the player system would otherwise do.
    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_update_system)
            .expect("boss update system must run");
        w.end_tick();
        w.player.iframes = (w.player.iframes - DT).max(0.0);
    }

    fn set_state(w: &mut World, state: MowerState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::Mower(state), duration);
            b.bounce_count = 0;
            b.hurt = 0.0;
        });
    }

    fn state_of(w: &mut World) -> MowerState {
        mower_state(&boss_of(w).expect("boss alive"))
    }

    /// Carve a sealed room into the boss arena; returns the interior rect.
    fn carve_room(w: &mut World, cells: (i32, i32)) -> TileRect {
        let arena = w.level.boss_room().expect("the Lab has a boss room").rect;
        let x = arena.x + 2;
        let y = arena.y + 2;
        w.level.tiles.fill_cells(
            x - 1,
            y - 1,
            cells.0 + 2,
            cells.1 + 2,
            TileKind::Wall(WallKind::Solid),
        );
        w.level
            .tiles
            .fill_cells(x, y, cells.0, cells.1, TileKind::Open);
        w.level.tiles.rebuild_masks();
        TileRect::new(x, y, cells.0, cells.1)
    }

    /// A controlled arena: the mower 60 px off the west wall and the rat 50 px
    /// west of the mower, so the charge lane ends on a bumper 3 px past the rat.
    fn lane_setup(seed: u64, phase: u8) -> (World, TileRect) {
        lane_setup_in(seed, phase, ROOM)
    }

    /// As [`lane_setup`], in a room of the given cell size. A room narrower than
    /// 270 px keeps the opposite wall inside the art's 135 px reach, which is
    /// what the bounce chain needs.
    fn lane_setup_in(seed: u64, phase: u8, cells: (i32, i32)) -> (World, TileRect) {
        let mut w = lab_world(seed);
        spawn_mower(&mut w);
        let room = carve_room(&mut w, cells);
        let west = f32::from(room.x as i16) * TILE_F;
        let pos = Vec2::new(west + 60.0, room.center_world().y);
        with_boss(&mut w, |b| {
            b.pos = pos;
            b.phase = phase;
            b.facing = Vec2::new(-1.0, 0.0);
            b.target = pos;
            b.pounce_origin = pos;
        });
        w.player.pos = pos - Vec2::new(50.0, 0.0);
        (w, room)
    }

    /// Drive the boss until its state changes; returns the seconds it lasted.
    fn drive_until_change(w: &mut World, limit: f32) -> (f32, MowerState) {
        let start = state_of(w);
        let mut t = 0.0;
        while t < limit {
            tick(w);
            t += DT;
            let now = state_of(w);
            if now != start {
                return (t, now);
            }
        }
        panic!("{start:?} never changed within {limit:.2} s");
    }

    /// Tick `n` times and record every drop in the rat's HP.
    fn hp_drops(w: &mut World, n: usize) -> Vec<f32> {
        let mut drops = Vec::new();
        let mut hp = w.player.hp;
        for _ in 0..n {
            tick(w);
            let now = w.player.hp;
            if now < hp - 1e-3 {
                drops.push(hp - now);
            }
            hp = now;
        }
        drops
    }

    /// Put the rat inside a hazard's footprint (an annulus needs a mid-radius
    /// point, everything else its centre).
    fn place_in(w: &mut World, hazard: &BossHazard) {
        let c = hazard.shape.center();
        w.player.pos = match hazard.shape {
            HazardShape::Annulus { inner, outer, .. } => c + Vec2::new((inner + outer) * 0.5, 0.0),
            _ => c,
        };
    }

    /// Wait for a hazard worth `damage`, step the rat into it on the tick its
    /// active window opens, and count the drops until it expires.
    ///
    /// Parking on the opening tick is deliberate: hazards resolve before body
    /// contact, so the attack's own damage lands first and its i-frames then
    /// mask the body. Callers keep the rat out of the footprint until then.
    fn hazard_damage_once(w: &mut World, damage: f32, ticks: usize) -> Vec<f32> {
        let mut parked = false;
        let mut drops = Vec::new();
        let mut hp = w.player.hp;
        for _ in 0..ticks {
            if !parked {
                let due = boss_of(w).and_then(|b| {
                    b.hazards
                        .iter()
                        .find(|h| h.damage == damage && !h.fired && h.warn - h.elapsed <= DT + 1e-6)
                        .copied()
                });
                if let Some(hazard) = due {
                    place_in(w, &hazard);
                    parked = true;
                }
            }
            tick(w);
            let now = w.player.hp;
            if now < hp - 1e-3 {
                drops.push(hp - now);
            }
            hp = now;
        }
        assert!(parked, "no {damage}-damage hazard was ever queued");
        drops
    }

    fn count(drops: &[f32], want: f32) -> usize {
        drops.iter().filter(|d| (**d - want).abs() < 1e-3).count()
    }

    /// Build a [`BossContext`] for the pure lane helpers.
    fn with_ctx<T>(w: &World, boss: &Boss, f: impl FnOnce(&BossContext<'_>) -> T) -> T {
        let mut rng = StdRng::seed_from_u64(0x5EED_1234);
        let ctx = BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            arena: boss.arena,
            level: &w.level,
            flow: &w.flow,
            rng: &mut rng,
        };
        f(&ctx)
    }

    fn lane_for(w: &mut World) -> Option<Lane> {
        let boss = {
            let mut q = w.ecs.query::<&Boss>();
            q.iter(&w.ecs).next().cloned()
        };
        boss.and_then(|b| with_ctx(w, &b, |ctx| select_lane(ctx, &b)))
    }

    // ---- every beat lasts what the art says ------------------------------

    #[test]
    fn mower_every_state_keeps_its_art_duration() {
        let cases: [(MowerState, u8, f32); 13] = [
            (MowerState::Patrol, 1, 2.40),
            (MowerState::Patrol, 2, 2.00),
            (MowerState::Patrol, 3, 1.60),
            (MowerState::Scan, 1, 1.35),
            (MowerState::Scan, 2, 1.10),
            (MowerState::Scan, 3, 0.90),
            (MowerState::ChargeWindup, 1, 0.85),
            (MowerState::ChargeWindup, 2, 0.72),
            (MowerState::ChargeWindup, 3, 0.60),
            (MowerState::Flipped, 1, 3.00),
            (MowerState::Flipped, 2, 2.60),
            (MowerState::Flipped, 3, 2.30),
            (MowerState::Overheat, 1, 1.20),
        ];
        for (state, phase, expected) in cases {
            let (mut w, room) = lane_setup(11, phase);
            w.player.pos = room.center_world() + Vec2::new(30.0, 0.0);
            set_state(&mut w, state, expected);
            let (elapsed, next) = drive_until_change(&mut w, expected + 1.0);
            assert!(
                (elapsed - expected).abs() <= 2.0 * DT,
                "{state:?} phase {phase} lasted {elapsed:.3} s, the art says {expected}"
            );
            assert_ne!(next, state);
        }
    }

    #[test]
    fn mower_docking_and_venting_keep_their_art_beat() {
        // Overheat 2.2 s (art) when it follows a volley; Docking is "up to
        // 4.0 s", which is the state's own clock.
        let (mut w, room) = lane_setup(12, 1);
        w.player.pos = room.center_world();
        with_boss(&mut w, enter_docking);
        let (elapsed, next) = drive_until_change(&mut w, DOCK_DURATION + 1.0);
        assert!((elapsed - DOCK_DURATION).abs() <= 2.0 * DT);
        assert_eq!(next, MowerState::Patrol);

        set_state(&mut w, MowerState::Overheat, OVERHEAT_DURATION);
        let (elapsed, next) = drive_until_change(&mut w, OVERHEAT_DURATION + 1.0);
        assert!((elapsed - OVERHEAT_DURATION).abs() <= 2.0 * DT);
        assert_eq!(next, MowerState::Patrol);
    }

    // ---- the charge: telegraph, ram, scorch ------------------------------

    #[test]
    fn mower_charge_queues_the_arts_corridor_and_windows() {
        let (mut w, _room) = lane_setup(21, 1);
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[0]);
        // Ride to the art's lock progress (0.45) where the aim freezes.
        let mut queued = None;
        for _ in 0..200 {
            tick(&mut w);
            let b = boss_of(&mut w).unwrap();
            if let Some(h) = b.hazards.iter().find(|h| h.damage == CHARGE_DAMAGE) {
                queued = Some((*h, b.progress(), b.hazards.len()));
                break;
            }
        }
        let (ram, progress, hazard_count) = queued.expect("the ram is queued at the freeze");
        assert!(
            (progress - WINDUP_LOCK_PROGRESS).abs() < 0.05,
            "ram queued at {progress:.3}, the art freezes the aim at 0.45"
        );
        assert_eq!(hazard_count, 2, "ram + scorched wake");
        match ram.shape {
            HazardShape::Corridor { from, to, half_width } => {
                assert!((half_width - art_boss_mower::CHARGE_CORRIDOR_HALF_WIDTH).abs() < 1e-4);
                let travel = from.dist(to);
                assert!(
                    (24.0..=CHARGE_MAX_TRAVEL).contains(&travel),
                    "committed run of {travel:.1} px"
                );
                // The queue happens at the freeze, so the warning window is
                // exactly the rest of the art's windup.
                let rest = WINDUP_DURATION[0] * (1.0 - WINDUP_LOCK_PROGRESS);
                assert!((ram.warn - rest).abs() < 1e-3);
                assert!(
                    (ram.active - charge_time(travel, 0)).abs() < 1e-3,
                    "active window is the charge itself"
                );
            }
            _ => panic!("the ram must be a corridor"),
        }
        assert_eq!(ram.damage, CHARGE_DAMAGE);
        assert!(ram.hit_enemies, "the mower mows the horde on its way through");
    }

    #[test]
    fn mower_ram_hits_the_rat_once_for_26() {
        let (mut w, _room) = lane_setup(22, 1);
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[0]);
        let drops = hazard_damage_once(&mut w, CHARGE_DAMAGE, 200);
        assert_eq!(count(&drops, CHARGE_DAMAGE), 1, "drops: {drops:?}");
    }

    #[test]
    fn mower_scorched_wake_hits_once_for_4() {
        let (mut w, _room) = lane_setup(23, 1);
        // Stand clear of the lane: only the wake may touch the rat here.
        let lane_pos = w.player.pos;
        w.player.pos = lane_pos + Vec2::new(0.0, 40.0);
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[0]);
        let drops = hazard_damage_once(&mut w, SCORCH_DAMAGE, 260);
        assert_eq!(count(&drops, SCORCH_DAMAGE), 1, "drops: {drops:?}");
    }

    // ---- the home move ---------------------------------------------------

    #[test]
    fn mower_charge_into_a_bumper_flips_it_onto_its_back() {
        let (mut w, _room) = lane_setup(31, 1);
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[0]);
        let mut flipped = None;
        for i in 0..180 {
            tick(&mut w);
            let b = boss_of(&mut w).unwrap();
            if mower_state(&b) == MowerState::Flipped {
                flipped = Some((i, b));
                break;
            }
        }
        let (_, boss) = flipped.expect("a charge that hits the arena wall flips the mower");
        assert_eq!(boss.bounce_count, 0);
        // The belly is the punish window: 2x incoming damage, no body damage.
        assert!((boss.damage_mult - 2.0).abs() < 1e-6);
        assert!(
            (art_boss_mower::collision_radius(mower_state(&boss), boss.progress())
                - art_boss_mower::FLIPPED_RADIUS)
                .abs()
                < 1e-6,
            "the art's tighter flipped hit radius is used"
        );
        // Hug the belly: the stalled blades deal no *body* damage (only the
        // scorched wake left by the charge may still be hot there).
        w.player.pos = boss.pos + Vec2::new(2.0, 0.0);
        let drops = hp_drops(&mut w, 30);
        assert_eq!(
            count(&drops, BossKind::Mower.contact_damage()),
            0,
            "flipped body dealt contact damage: {drops:?}"
        );
    }

    #[test]
    fn mower_ricochet_bounces_once_in_phase_two_and_twice_in_phase_three() {
        for (phase, want_legs) in [(2u8, 1u8), (3, 2)] {
            let (mut w, _room) = lane_setup_in(41 + u64::from(phase), phase, (9, 10));
            let slot = (phase - 1) as usize;
            set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[slot]);
            let mut legs = 0;
            let mut last = 0u8;
            for _ in 0..1200 {
                let b = boss_of(&mut w).unwrap();
                if mower_state(&b) == MowerState::Ricochet && b.bounce_count != last {
                    last = b.bounce_count;
                    legs += 1;
                    let leg = b
                        .hazards
                        .iter()
                        .find(|h| h.damage == RICOCHET_DAMAGE)
                        .copied()
                        .expect("each rebound leg queues its corridor");
                    assert!((leg.warn - RICOCHET_PAUSE).abs() < 1e-6);
                    assert!(
                        (RICOCHET_LEG[0]..=RICOCHET_LEG[1]).contains(&leg.active),
                        "leg active window {:.2} s",
                        leg.active
                    );
                    // The design's 0.40 s rebound pause: the chassis holds
                    // still while the art paints the dashed rebound line, then
                    // the leg accelerates. Measure only the ticks strictly
                    // inside the pause (the state's own clock, not progress,
                    // because one tick can straddle the boundary).
                    let mut moved = 0.0;
                    let mut prev = b.pos;
                    loop {
                        let bb = boss_of(&mut w).unwrap();
                        if mower_state(&bb) != MowerState::Ricochet
                            || bb.elapsed + DT >= RICOCHET_PAUSE - 1e-3
                        {
                            break;
                        }
                        tick(&mut w);
                        let next = boss_of(&mut w).unwrap();
                        moved += (next.pos - prev).len();
                        prev = next.pos;
                    }
                    assert!(
                        moved < 1.0,
                        "the {RICOCHET_PAUSE} s rebound pause must hold still (moved {moved:.2} px)"
                    );
                    // ...and the leg itself has to accelerate afterwards.
                    let mut leg = 0.0;
                    let mut prev = boss_of(&mut w).unwrap().pos;
                    for _ in 0..12 {
                        tick(&mut w);
                        let next = boss_of(&mut w).unwrap();
                        leg += (next.pos - prev).len();
                        prev = next.pos;
                    }
                    assert!(leg > 3.0, "the rebound leg must actually travel ({leg:.2} px)");
                    continue;
                }
                tick(&mut w);
                if mower_state(&boss_of(&mut w).unwrap()) == MowerState::Flipped {
                    break;
                }
            }
            assert_eq!(
                legs, want_legs,
                "phase {phase} should chain {want_legs} rebound leg(s)"
            );
            assert_eq!(
                state_of(&mut w),
                MowerState::Flipped,
                "the chain ends on its back (the art's final bounce)"
            );
        }
    }

    #[test]
    fn mower_rebound_and_self_righting_hazards_hit_once() {
        // Rebound leg (phase 2): 22, once.
        let (mut w, _room) = lane_setup_in(42, 2, (9, 10));
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[1]);
        let drops = hazard_damage_once(&mut w, RICOCHET_DAMAGE, 400);
        assert_eq!(count(&drops, RICOCHET_DAMAGE), 1, "drops: {drops:?}");

        // The self-righting thud: 10, once, in the art's 0.92..1.0 window.
        let (mut w, room) = lane_setup(43, 1);
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        w.player.pos = c + Vec2::new(60.0, 0.0);
        set_state(&mut w, MowerState::Flipped, FLIPPED_DURATION[0]);
        let drops = hazard_damage_once(&mut w, FLIP_SLAM_DAMAGE, 260);
        assert_eq!(count(&drops, FLIP_SLAM_DAMAGE), 1, "drops: {drops:?}");
    }

    // ---- lock, refusal, venting ------------------------------------------

    #[test]
    fn mower_scan_lock_is_rejected_through_a_solid() {
        let (mut w, room) = lane_setup(51, 1);
        // A one-cell steel column 24 px in front of the mower, rat 10 px past
        // it: in range, but the lidar line is broken.
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        let block = (c.x / TILE_F).floor() as i32 + 1;
        let row = (c.y / TILE_F).floor() as i32;
        w.level
            .tiles
            .fill_cells(block, row - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        w.player.pos = Vec2::new((block as f32 + 1.5) * TILE_F, c.y);
        assert!(
            !los_clear(&w.level, c, w.player.pos, PLAYER_RADIUS),
            "test setup: the column must block the line"
        );
        set_state(&mut w, MowerState::Scan, SCAN_DURATION[0]);
        let (_, next) = drive_until_change(&mut w, SCAN_DURATION[0] + 0.5);
        assert_eq!(
            next,
            MowerState::Overheat,
            "a broken lock vents instead of charging"
        );
        // ...and the vent is the art's short one.
        let b = boss_of(&mut w).unwrap();
        assert!((b.duration - OVERHEAT_MISSED).abs() < 1e-6);
    }

    #[test]
    fn mower_refuses_a_charge_that_cannot_fit_and_repositions() {
        // A 3x3-cell (48 px) pocket: the mower fits, but there is no 24 px
        // run-up, let alone a 32 px side lane at the impact.
        let mut w = lab_world(61);
        spawn_mower(&mut w);
        let room = carve_room(&mut w, TINY_ROOM);
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        w.player.pos = c + Vec2::new(16.0, 0.0);
        assert!(
            lane_for(&mut w).is_none(),
            "no lane in a 48 px pocket may be accepted"
        );
        set_state(&mut w, MowerState::Scan, SCAN_DURATION[0]);
        let (_, next) = drive_until_change(&mut w, SCAN_DURATION[0] + 0.5);
        assert_eq!(
            next,
            MowerState::Patrol,
            "with no chargeable lane it repositions instead of attacking"
        );

        // Companion check (the same code path accepts a lane when one fits).
        let (mut w2, _room) = lane_setup(62, 1);
        assert!(
            lane_for(&mut w2).is_some(),
            "the lane helper must accept a real bumper lane"
        );
    }

    #[test]
    fn mower_charge_across_open_floor_vents_instead_of_hitting() {
        // 320 x 288 px of open floor: no wall inside the art's 135 px reach,
        // so the lane is a clean run and the mower vents afterwards.
        let mut w = lab_world(63);
        spawn_mower(&mut w);
        let room = carve_room(&mut w, BIG_ROOM);
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        w.player.pos = c + Vec2::new(60.0, 0.0);
        let lane = lane_for(&mut w).expect("an open-floor run is a legal charge");
        assert!(!lane.wall, "no solid within 135 px");
        set_state(&mut w, MowerState::ChargeWindup, WINDUP_DURATION[0]);
        let (_, next) = drive_until_change(&mut w, 3.0);
        assert_eq!(next, MowerState::Charge);
        let (_, next) = drive_until_change(&mut w, 3.0);
        assert_eq!(next, MowerState::Overheat);
    }

    #[test]
    fn mower_refuses_a_lane_without_the_32px_side_lane() {
        // A two-cell (32 px) corridor: the chassis fits, the run-up fits, but
        // there is no escape lane beside the impact point — so no charge.
        let mut w = lab_world(64);
        spawn_mower(&mut w);
        let room = carve_room(&mut w, (2, 12));
        let c = room.center_world();
        with_boss(&mut w, |b| {
            b.pos = Vec2::new(c.x, f32::from(room.y as i16) * TILE_F + 60.0);
        });
        // 50 px away, inside lidar range, so the refusal is the lane's.
        w.player.pos = Vec2::new(c.x, f32::from(room.y as i16) * TILE_F + 110.0);
        assert!(
            lane_for(&mut w).is_none(),
            "a lane with no side lane out of the corridor must be refused"
        );
        set_state(&mut w, MowerState::Scan, SCAN_DURATION[0]);
        let (_, next) = drive_until_change(&mut w, SCAN_DURATION[0] + 0.5);
        assert_eq!(next, MowerState::Patrol);
    }

    #[test]
    fn mower_venting_body_deals_no_contact_damage() {
        let (mut w, room) = lane_setup(65, 1);
        with_boss(&mut w, |b| b.pos = room.center_world());
        set_state(&mut w, MowerState::Overheat, OVERHEAT_DURATION);
        // Sit on the venting shell for half a second: the blades are off.
        w.player.pos = room.center_world() + Vec2::new(2.0, 0.0);
        let drops = hp_drops(&mut w, 30);
        assert!(drops.is_empty(), "venting body dealt {drops:?}");
    }

    #[test]
    fn mower_never_ends_a_tick_inside_a_wall() {
        let (mut w, room) = lane_setup_in(66, 1, (9, 10));
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        let radius = BossKind::Mower.body_radius();
        for i in 0..900 {
            let a = i as f32 * 0.02;
            w.player.pos = c + Vec2::from_angle(a) * 46.0;
            tick(&mut w);
            let b = boss_of(&mut w).unwrap();
            assert!(
                w.level
                    .tiles
                    .circle_clear(b.pos, radius, SizeClass::Large),
                "the chassis left the passable cells at {:?} ({:?})",
                b.pos,
                mower_state(&b)
            );
        }
    }

    // ---- docking ---------------------------------------------------------

    #[test]
    fn mower_docking_heals_capped_and_the_shock_ring_hits_once() {
        let (mut w, room) = lane_setup(71, 1);
        // The dock is where the arena puts it; park it inside the test room and
        // start the mower 40 px away so the transit fits in the beat.
        let dock = room.center_world() + Vec2::new(40.0, 0.0);
        with_boss(&mut w, |b| {
            b.dock = dock;
            b.pos = room.center_world() - Vec2::new(0.0, 30.0);
            b.hp = b.max_hp * 0.5;
            enter_docking(b);
        });
        w.player.pos = dock + Vec2::new(0.0, 20.0);
        let hp0 = boss_of(&mut w).unwrap().hp;
        let drops = hp_drops(&mut w, (DOCK_DURATION / DT) as usize + 4);
        let boss = boss_of(&mut w).unwrap();
        let healed = boss.hp - hp0;
        assert!(healed > 0.0, "the dock recharges it");
        assert!(
            healed <= DOCK_HEAL_MAX * boss.max_hp + 1e-3,
            "healed {healed:.1}, cap is {:.1}",
            DOCK_HEAL_MAX * boss.max_hp
        );
        assert_eq!(count(&drops, DOCK_SHOCK_DAMAGE), 1, "drops: {drops:?}");
    }

    #[test]
    fn mower_docking_is_short_circuited_by_burst_damage() {
        let (mut w, room) = lane_setup(72, 1);
        let dock = room.center_world() + Vec2::new(40.0, 0.0);
        with_boss(&mut w, |b| {
            b.dock = dock;
            b.pos = room.center_world() - Vec2::new(0.0, 30.0);
            b.hp = b.max_hp * 0.5;
            enter_docking(b);
        });
        w.player.pos = dock + Vec2::new(0.0, 60.0);
        let mut interrupted = false;
        for _ in 0..(DOCK_DURATION / DT) as usize {
            let charging = boss_of(&mut w).unwrap().flags & F_DOCK_CHARGING != 0;
            if charging {
                with_boss(&mut w, |b| {
                    b.take_damage(DOCK_INTERRUPT * 0.5);
                    b.take_damage(DOCK_INTERRUPT * 0.5);
                });
            }
            tick(&mut w);
            let b = boss_of(&mut w).unwrap();
            if mower_state(&b) == MowerState::Overheat {
                assert!((b.duration - OVERHEAT_DURATION).abs() < 1e-6);
                interrupted = true;
                break;
            }
        }
        assert!(interrupted, "120 burst damage must short-circuit the recharge");
    }

    // ---- deferred volley -------------------------------------------------

    #[test]
    fn mower_fling_blades_state_and_deck_aura_are_implemented() {
        let (mut w, room) = lane_setup(81, 2);
        w.player.pos = room.center_world() + Vec2::new(60.0, 0.0);
        with_boss(&mut w, enter_fling);
        let b = boss_of(&mut w).unwrap();
        assert!((b.duration - FLING_DURATION).abs() < 1e-6);
        let deck = b
            .hazards
            .iter()
            .find(|h| h.damage == FLING_DECK_DAMAGE)
            .copied()
            .expect("the deck aura is queued on entry");
        assert!((deck.warn - FLING_DURATION * FLING_TELEGRAPH).abs() < 1e-6);
        assert!(
            (deck.active - FLING_DURATION * (FLING_ACTIVE_END - FLING_TELEGRAPH)).abs() < 1e-6
        );
        // The state runs its full beat and vents afterwards.
        let (elapsed, next) = drive_until_change(&mut w, FLING_DURATION + 1.0);
        assert!((elapsed - FLING_DURATION).abs() <= 2.0 * DT);
        assert_eq!(next, MowerState::Overheat);
        // ...and the aura is a punish threat, not a hug.
        let (mut w2, room2) = lane_setup(82, 2);
        w2.player.pos = room2.center_world() + Vec2::new(60.0, 0.0);
        with_boss(&mut w2, enter_fling);
        let drops = hazard_damage_once(&mut w2, FLING_DECK_DAMAGE, (FLING_DURATION / DT) as usize);
        assert_eq!(count(&drops, FLING_DECK_DAMAGE), 1, "drops: {drops:?}");
    }

    #[test]
    fn mower_rotation_defers_the_volley_in_this_build() {
        const { assert!(DEFER_FLING_BLADES, "design: charge-and-flip first") };
        let (mut w, room) = lane_setup(83, 1);
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        let mut states = Vec::new();
        for i in 0..600 {
            let a = i as f32 * 0.02;
            w.player.pos = c + Vec2::from_angle(a) * 50.0;
            tick(&mut w);
            let s = state_of(&mut w);
            if states.last() != Some(&s) {
                states.push(s);
            }
        }
        assert!(
            !states.contains(&MowerState::FlingBlades),
            "the volley is out of the rotation in this build: {states:?}"
        );
    }

    // ---- the whole fight -------------------------------------------------

    #[test]
    fn mower_all_states_are_reachable_in_the_fight() {
        let (mut w, room) = lane_setup_in(91, 1, (9, 12));
        let c = room.center_world();
        with_boss(&mut w, |b| b.pos = c);
        let dock = c + Vec2::new(40.0, 0.0);
        with_boss(&mut w, |b| b.dock = dock);
        let mut seen: Vec<MowerState> = Vec::new();
        for i in 0..1200 {
            // Orbit the rat so lanes keep opening, and drop the HP through the
            // two thresholds so the phases (and the docking retreats) fire.
            let a = i as f32 * 0.015;
            w.player.pos = c + Vec2::from_angle(a) * 52.0;
            if i == 500 {
                with_boss(&mut w, |b| {
                    let cut = b.max_hp * 0.35;
                    b.take_damage(cut);
                });
            }
            if i == 900 {
                with_boss(&mut w, |b| {
                    let cut = b.max_hp * 0.35;
                    b.take_damage(cut);
                });
            }
            tick(&mut w);
            let s = state_of(&mut w);
            if seen.last() != Some(&s) {
                seen.push(s);
            }
        }
        for want in [
            MowerState::Patrol,
            MowerState::Scan,
            MowerState::ChargeWindup,
            MowerState::Charge,
            MowerState::Flipped,
            MowerState::Ricochet,
            MowerState::Docking,
        ] {
            // Overheat's two live paths (missed lock, spent charge) are asserted
            // by `mower_scan_lock_is_rejected_through_a_solid` and
            // `mower_charge_across_open_floor_vents_instead_of_hitting`.
            assert!(seen.contains(&want), "{want:?} never happened: {seen:?}");
        }
        // The framework owns the death state: killing it plays `Dying` first.
        with_boss(&mut w, |b| b.hp = 0.0);
        tick(&mut w);
        assert_eq!(state_of(&mut w), MowerState::Dying);
        assert!(boss_of(&mut w).unwrap().dead);
    }

    #[test]
    fn mower_fight_is_beatable_by_a_dodging_rat() {
        let mut w = lab_world(101);
        spawn_mower(&mut w);
        // Hold the art's 1600 HP pool: the director's level-2 curve multiplies
        // it, which is a balance question, not this test's subject.
        with_boss(&mut w, |b| {
            b.hp = 1600.0;
            b.max_hp = 1600.0;
        });
        let arena = w.level.boss_room().expect("boss room").rect;
        let arena_world = arena.to_world();
        let boss = boss_of(&mut w).unwrap();
        w.player.pos = boss.pos + Vec2::new(70.0, 0.0);
        let speed = w.player.loadout.stats.move_speed;
        let dps = 1600.0 / 30.0;
        let mut min_hp = w.player.hp;
        let mut killed_at = None;
        let mut saw_death_beat = false;
        for i in 0..(60.0 / DT) as usize {
            let b = match boss_of(&mut w) {
                Some(b) => b,
                None => break,
            };
            let dir = dodge_dir(&w, &b);
            let next = w.player.pos + dir * speed * DT;
            let next = w
                .level
                .tiles
                .resolve_circle(next, PLAYER_RADIUS, SizeClass::Small, None);
            w.player.pos = Vec2::new(
                next.x.clamp(arena_world.x + 8.0, arena_world.right() - 8.0),
                next.y.clamp(arena_world.y + 8.0, arena_world.bottom() - 8.0),
            );
            with_boss(&mut w, |b| {
                let amount = dps * DT * b.damage_mult;
                b.take_damage(amount);
            });
            tick(&mut w);
            if let Some(b) = boss_of(&mut w) {
                if b.dead {
                    killed_at.get_or_insert(i as f32 * DT);
                    saw_death_beat |= matches!(b.state, BossState::Mower(MowerState::Dying));
                }
            }
            min_hp = min_hp.min(w.player.hp);
            if w.player.hp <= 0.0 {
                break;
            }
            if w.level_complete {
                break;
            }
            // A dodging rat never stands in the corpses' way.
            w.player.iframes = w.player.iframes.max(0.0);
        }
        assert!(min_hp > 0.0, "the scripted dodger died (min hp {min_hp})");
        assert!(w.level_complete, "sustained damage must kill the mower");
        // Exactly once, no matter how many ticks follow.
        for _ in 0..30 {
            tick(&mut w);
        }
        assert!(w.level_complete);
        assert!(w.ecs.resource::<BossFight>().level_complete);
        // The design's death beat: `Dying` plays (its 18-damage blast included)
        // and the level-complete signal rises only when it ends, so the boss is
        // gone by the time the loop breaks.
        assert!(saw_death_beat, "the mower's Dying beat never played");
        assert!(
            boss_of(&mut w).is_none(),
            "the death beat despawns the mower before the signal settles"
        );
        let killed_at = killed_at.expect("the mower died inside the 60 s window");
        assert!(killed_at < 60.0, "killed at {killed_at:.1} s");
    }

    #[test]
    fn mower_runs_inside_the_real_schedule() {
        // The real tick schedule (director hand-off, horde, boss systems) must
        // drive the mower without panicking, from the spawn beat onwards.
        let mut w = lab_world(111);
        w.elapsed = crate::game::director::BOSS_TIME;
        w.update(DT, Vec2::ZERO, false);
        let boss = boss_of(&mut w).expect("the director spawned the Lab boss");
        assert_eq!(boss.kind, BossKind::Mower);
        assert!(boss.hp > 0.0);
        for _ in 0..120 {
            w.update(DT, Vec2::ZERO, false);
        }
        let boss = boss_of(&mut w).expect("the mower survives two seconds");
        assert_eq!(boss.kind, BossKind::Mower);
        assert!(w.boss_hp().is_some());
    }

    /// The scripted rat: break out of the laser lane on the telegraph, keep
    /// mid-range otherwise, and step in to punish a flipped or venting mower.
    fn dodge_dir(w: &World, b: &Boss) -> Vec2 {
        let to = w.player.pos - b.pos;
        let dist = to.len();
        let away = if to.len_sq() > 0.0 {
            to.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let perp = b.facing.perp();
        let off = to.dot(perp);
        match mower_state(b) {
            MowerState::ChargeWindup if b.progress() > 0.35 => perp * if off >= 0.0 { 1.0 } else { -1.0 },
            MowerState::Charge | MowerState::Ricochet => {
                perp * if off >= 0.0 { 1.0 } else { -1.0 }
            }
            MowerState::Docking => {
                // Stay clear of the surge ring while it recharges.
                if dist > 34.0 {
                    -away
                } else {
                    away
                }
            }
            MowerState::Flipped | MowerState::Overheat => {
                // Punish range: inside 26 px, outside the body.
                if dist > 26.0 {
                    -away
                } else {
                    Vec2::ZERO
                }
            }
            _ => {
                if dist < 46.0 {
                    away
                } else if dist > 66.0 {
                    -away
                } else {
                    perp
                }
            }
        }
    }


    #[test]
    fn mower_home_move_is_available_when_the_rat_fights_near_a_wall() {
        // Property over seeds: with the rat within 48 px of a **solid** arena
        // wall and the mower 30..80 px away, a lane that ends on that wall *a
        // screenful from the rat* is always on offer — that is the flip the rat
        // can bait.
        //
        // The wall has to be solid for that to mean anything, and the arena's
        // wall ring is no longer continuous: the generator punches the arena's
        // mouth(s) through it as a `WallKind::Doorway` band
        // (`layout::BOSS_MOUTH_MIN` cells wide). On some seeds one mouth sits at
        // the arena's mid-height — exactly where this fixture puts the rat — so
        // the straight lane runs out through the arch instead of into a wall.
        // That is map geometry, not a gap in the fight: a lane angled far enough
        // to end on the mouth's jamb would pass beside the rat, and
        // `probe_lane`'s `attack_fits` side-lane/corridor test rightly refuses
        // it. So the fixture only asserts the property where the rat is really
        // beside a wall, and counts the mouth set-ups it skipped.
        let mut near = 0;
        let mut total = 0;
        let mut at_mouth = 0;
        for seed in 0..8u64 {
            let mut w = lab_world(seed);
            let boss = spawn_mower(&mut w);
            let arena = boss.arena;
            let west = f32::from(arena.x as i16) * TILE_F;
            let y = arena.center_world().y;
            // The wall ring column beside the arena's interior, and the rows
            // the chassis actually spans at the rat's height.
            let wall_x = arena.x - 1;
            let band = ((y - art_boss_mower::BODY_RADIUS) / TILE_F).floor() as i32
                ..=((y + art_boss_mower::BODY_RADIUS) / TILE_F).floor() as i32;
            let solid_wall = band.clone().all(|cy| {
                matches!(
                    w.level.tiles.get(wall_x, cy),
                    TileKind::Wall(WallKind::Solid)
                )
            });
            for off_wall in [24.0f32, 48.0] {
                if !solid_wall {
                    // The arena mouth (a doorway band, not a wall).
                    at_mouth += 1;
                    continue;
                }
                let rat = Vec2::new(west + off_wall, y);
                for step in [30.0f32, 50.0, 80.0] {
                    with_boss(&mut w, |b| b.pos = rat + Vec2::new(step, 0.0));
                    w.player.pos = rat;
                    total += 1;
                    if let Some(lane) = lane_for(&mut w) {
                        if lane.wall && lane.near_rat {
                            near += 1;
                        }
                    }
                }
            }
        }
        assert!(total >= 40, "the fixture lost its wall set-ups: {total}");
        assert_eq!(
            near, total,
            "the flip was on offer in {near}/{total} solid-wall setups \
             ({at_mouth} set-ups were beside the arena mouth, where there is no \
             wall for a lane to end on)"
        );
    }

    #[test]
    fn mower_home_move_is_available_from_the_clear_floor_with_the_bumpers() {
        // The arena worker's contract: the Lab's clear floor has a solid within
        // 64 px on both sides, so a boss standing on it always has a charge lane
        // that ends on a bumper a screenful from the rat. Before the bumpers
        // the same measurement from the geometric arena centre was 0 of 96.
        let mut near = 0;
        let mut total = 0;
        for seed in 0..8u64 {
            let mut w = lab_world(seed);
            let boss = spawn_mower(&mut w);
            let floor = w
                .level
                .boss_clear_floor()
                .expect("the Lab exports its clear encounter floor");
            let centre = floor.center_world();
            let _ = boss;
            // The rat in every cell of the arena worker's clear fighting floor
            // (the 4 × 3 central floor), the mower on it: 12 set-ups per seed.
            for cy in floor.y..floor.y + floor.h {
                for cx in floor.x..floor.x + floor.w {
                    let rat = crate::game::collision::TileMap::tile_rect(cx, cy).center();
                    with_boss(&mut w, |b| b.pos = centre);
                    w.player.pos = rat;
                    total += 1;
                    if let Some(lane) = lane_for(&mut w) {
                        if lane.wall && lane.near_rat {
                            near += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(near, total, "the flip was on offer in {near}/{total} setups");
    }

    // ---- determinism -----------------------------------------------------

    #[test]
    fn mower_twin_runs_match() {
        let run = |seed: u64| {
            let (mut w, room) = lane_setup(seed, 1);
            let c = room.center_world();
            with_boss(&mut w, |b| b.pos = c);
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            let mix = |v: u64, hash: &mut u64| {
                *hash ^= v;
                *hash = hash.wrapping_mul(0x100_0000_01b3);
            };
            for i in 0..900 {
                let a = i as f32 * 0.021;
                w.player.pos = c + Vec2::from_angle(a) * 50.0;
                if i % 5 == 0 {
                    with_boss(&mut w, |b| {
                        b.take_damage(6.0);
                    });
                }
                if i % 211 == 0 {
                    with_boss(&mut w, |b| {
                        let cut = b.max_hp * 0.2;
                        b.take_damage(cut);
                    });
                }
                tick(&mut w);
                let b = match boss_of(&mut w) {
                    Some(b) => b,
                    None => {
                        mix(u64::MAX, &mut hash);
                        break;
                    }
                };
                mix(b.pos.x.to_bits() as u64, &mut hash);
                mix(b.pos.y.to_bits() as u64, &mut hash);
                mix(b.hp.to_bits() as u64, &mut hash);
                mix(mower_state(&b) as u64, &mut hash);
                mix(u64::from(b.phase), &mut hash);
                mix(u64::from(b.bounce_count), &mut hash);
                mix(b.hazards.len() as u64, &mut hash);
                mix(w.player.hp.to_bits() as u64, &mut hash);
            }
            hash
        };
        assert_eq!(run(4242), run(4242), "the same seed must replay exactly");
        assert_eq!(run(7), run(7));
    }

    #[test]
    fn mower_twin_runs_match_with_the_volley_forced_on() {
        // The deferred volley still has to be deterministic if it is enabled:
        // drive the state directly (its projectile order is fixed by phase).
        let run = |seed: u64| {
            let (mut w, room) = lane_setup(seed, 3);
            let c = room.center_world();
            with_boss(&mut w, |b| b.pos = c);
            w.player.pos = c + Vec2::new(40.0, 10.0);
            with_boss(&mut w, enter_fling);
            let mut hash = 0u64;
            for i in 0..150 {
                w.player.pos = c + Vec2::from_angle(i as f32 * 0.05) * 45.0;
                tick(&mut w);
                let b = boss_of(&mut w).unwrap();
                hash = hash
                    .wrapping_mul(31)
                    .wrapping_add(b.pos.x.to_bits() as u64)
                    .wrapping_add(u64::from(b.bounce_count));
            }
            hash
        };
        assert_eq!(run(88), run(88));
    }
}
