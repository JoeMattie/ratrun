//! Kitchen boss — The Sweeper (giant broom + slippers).
//!
//! The art module [`crate::game::art_boss_broom`] is the gameplay spec: its
//! state enum, every per-state duration, damage number, hit radius and the
//! view struct are binding. This file only drives the transitions and queues
//! the art's telegraphed attacks as shared [`BossHazard`]s /
//! [`BossProjectile`]s; everything the rat sees is drawn from the art's
//! `compute_pose` through [`Boss::view`].
//!
//! ## State machine (art timings; Phase 1 / Phase 2 `enraged`)
//!
//! | state | duration | telegraph → active window | damage |
//! |-------|----------|---------------------------|--------|
//! | `Idle` | 0.55 / 0.32 s | — | foot contact 10 |
//! | `StompWalk` | 1.15 / 0.85 s | rise 0.00..0.58, shadow to 0.66 → 0.66..0.78 | 26 crush (30) + 16 shock (20) |
//! | `BroomSweep` | 1.65 / 1.25 s | 0.00..0.46 → 0.46..0.72 | 20 (24) |
//! | `BroomJab` | 1.80 / 1.40 s | 0.17/0.49/0.81 → +0.07 | 14 (18) ×3 — **cut, never scheduled** |
//! | `DustpanDrop` | 1.45 / 1.15 s | 0.00..0.46 → 0.46..0.58 | 22 (26) + 6 × 10 shards |
//! | `Hurt` | 1.20 s | — | bump only |
//! | `PhaseChange` | 1.80 s | 0.00..0.38 → 0.38..0.54 | 18 (+50 % damage cut on the boss) |
//! | `Defeat` | 2.40 s | — | none (the framework owns it) |
//!
//! ## The designer's direction (takes precedence where it narrows the art)
//!
//! * **The safe place is under the human.** The rake's danger band is the art's
//!   annulus `r ∈ [12, 38] px` (`SWEEP_INNER_RADIUS..SWEEP_OUTER_RADIUS`), so
//!   the rat runs *toward* the boots: the pocket inside the inner radius is
//!   the home move, and the planted feet stop crushing for the whole rake so
//!   the pocket really is safe. A rake is only started when the rat has a
//!   clear lane into that pocket (`los_clear`) and a clear lane back out
//!   (`attack_fits`), which is the "keep an approach to the inner pocket clear"
//!   requirement expressed as a fit test.
//! * **Opening.** Alternating stomps first, then one rake, then two more
//!   stomps and a second rake; only after the rat has *seen two rakes* does the
//!   dustpan drop enter the rotation (`dustpan_unlocked`).
//! * **The three-jab combo is cut from this build.** The state is fully
//!   implemented (hazards, beats, reticles) and reachable through [`enter`],
//!   but [`JAB_ENABLED`] is `false` so it is never scheduled.
//! * **The hurt hop and the walk-off matter more than another attack.** A heavy
//!   burst on one foot (`STAGGER_HP_FRAC`) trips the art's comic
//!   `Hurt { left_foot_hit }` reprieve — the foot nearest the rat is the one it
//!   stubbed. Batch 13 put the hop on the shared control budget
//!   (`Boss::grant_control`), so it is 1.20 s when the budget is full and at
//!   most `CONTROL_BUDGET_SECS` (0.30 s) inside one 4 s window, and it never
//!   lands inside a committed beat: a rake cancelled on its first frame can
//!   never fire, which is exactly what a high-burst build used to do to every
//!   telegraph. `poise_broken` counts from the beat's own anchor, so a refused
//!   hop stays owed. The indignant retreat is the art's `Defeat` pose, driven
//!   by the shared death state.
//! * **The planted foot is the target.** [`Boss::hit_shapes`] takes both feet
//!   straight from `foot_hitboxes`, so the lifted foot is `grounded: false` and
//!   ground weapons (bullets, acid pools, Tail Whip) skip it automatically.
//!   Auto-aim's [`Boss::target_points`] is the framework's to change — it
//!   currently returns every shape: filtering on `grounded()` there is the one
//!   edit this file needs from `boss/mod.rs` (see the report).
//!
//! ## Determinism and allocation
//!
//! The only randomness is one `f32` per open-ended attack decision from
//! `ctx.rng` (the boss's own stream), so the draw *count* is reproducible for
//! a seed. Every attack starts on a decision index, never on a timer, and all
//! geometry comes from the art helpers. Nothing here allocates: the hazard ring
//! is a pre-sized `Vec` and projectiles go into the reused `BossOutput`.
//!
//! ## Per-controller memory
//!
//! The art view reads only `pos`/`facing`/`state`/`progress`/`anim`/`hit_flash`/
//! `hp_frac`/`target`/`enraged`, so everything else the Sweeper needs lives in
//! its own shared scratch slots ([`Boss::flags`] + [`Boss::scratch`]):
//!
//! * `flags` — bit 0 `ARMED` ("this beat already queued its hazard"), bit 1 the
//!   alternating stomping foot, bit 2 `CUE` ("this beat already played its
//!   footfall/rake/impacts cue"), bits 8..13 the attack-decision index (the
//!   opening order).
//! * `scratch[0]` — stagger cooldown seconds.
//! * `scratch[1]` — rakes the rat has seen.
//! * `scratch[2]` — the hp anchor of the current beat (the poise budget the
//!   art's heavy-stagger burst is measured against).
//!
//! All three start at zero, which is exactly what the old borrowed fields were
//! cleared to on the first tick, so the behaviour is unchanged.
//!
//! ## Feedback
//!
//! Controllers cannot touch `SfxQueue`/`Particles`; the Sweeper queues
//! [`BossCue`]s and the shared system plays them (footfall at `0.66`, the
//! rake's active start at `0.46`, the pan's impact at `0.48`, the tantrum slam
//! at `0.38`, the hurt-hop entry and both touchdowns, and the defeat walk-off).

use std::f32::consts::TAU;

use super::*;

/// The Sweeper controller (zero-sized; the state lives on the [`Boss`]).
pub struct BroomController;

// ============================================================================
// Timings, damage and geometry (art_boss_broom's per-state doc comments)
// ============================================================================

/// `Idle` recovery beat (art: `0.55 s` P1 / `0.32 s` P2).
const IDLE_P1: f32 = 0.55;
const IDLE_P2: f32 = 0.32;

/// `StompWalk` (art: `1.15 s` / `0.85 s`).
const STOMP_P1: f32 = 1.15;
const STOMP_P2: f32 = 0.85;
/// The foot starts coming down (art's land phase) and the active window ends.
const STOMP_FOOTFALL: f32 = 0.58;
const STOMP_WARN_END: f32 = 0.66;
const STOMP_ACTIVE_END: f32 = 0.78;
/// Art: `26 dmg` direct crush (`30` in phase 2), `16 dmg` shockwave (`20`).
const STOMP_CRUSH_P1: f32 = 26.0;
const STOMP_CRUSH_P2: f32 = 30.0;
const STOMP_SHOCK_P1: f32 = 16.0;
const STOMP_SHOCK_P2: f32 = 20.0;
const STOMP_KNOCKBACK: f32 = 110.0;
/// Longest step one stomp covers (art: "recommended stride `14..24 px`").
const STOMP_STRIDE_MAX: f32 = 24.0;
/// Closest landing: nearer than `3 px` makes the art fall back to its
/// `foot + facing * 18` pose, which would put the slam off the telegraphed ring.
const STOMP_LAND_MIN: f32 = 6.0;
/// The body marches after the planted foot once it has landed.
const STOMP_SETTLE_SPEED_MAX: f32 = 72.0;

/// `BroomSweep` (art: `1.65 s` / `1.25 s`, active `0.46..0.72`).
const SWEEP_P1: f32 = 1.65;
const SWEEP_P2: f32 = 1.25;
const SWEEP_WARN_END: f32 = 0.46;
const SWEEP_ACTIVE_END: f32 = 0.72;
const SWEEP_DAMAGE_P1: f32 = 20.0;
const SWEEP_DAMAGE_P2: f32 = 24.0;
const SWEEP_KNOCKBACK: f32 = 220.0;
/// The art's `Hurt` bump: the single hopping slipper still shoves a rat that
/// walks straight into it (`6 dmg`), far below the shared `10` foot bump.
const HURT_CONTACT_DAMAGE: f32 = 6.0;

/// `BroomJab` (art: `1.80 s` / `1.40 s`, three beats, active 20 % of each).
const JAB_P1: f32 = 1.80;
const JAB_P2: f32 = 1.40;
const JAB_DAMAGE_P1: f32 = 14.0;
const JAB_DAMAGE_P2: f32 = 18.0;
const JAB_KNOCKBACK: f32 = 80.0;
/// Art: beats start at `0.00 / 0.32 / 0.64`, each active at `+0.17..+0.24`.
const JAB_BEATS: [f32; 3] = [0.0, 0.32, 0.64];
const JAB_WARN_END: [f32; 3] = [0.17, 0.49, 0.81];
const JAB_ACTIVE_END: [f32; 3] = [0.24, 0.56, 0.88];
/// The three-jab combo is implemented but **cut from this build** (designer):
/// while this stays `false` the state is never scheduled.
const JAB_ENABLED: bool = false;
/// Lateral offsets of the first two jabs (art: `-7.5` / `+7.5` across facing).
const JAB_SIDE: f32 = 7.5;
const JAB_REACH_1: f32 = 20.0;
const JAB_REACH_2: f32 = 24.0;

/// `DustpanDrop` (art: `1.45 s` / `1.15 s`, active `0.46..0.58`, shards at
/// `0.48`, `48`/`60 px/s`, recommended range `22..56 px`).
const DUSTPAN_P1: f32 = 1.45;
const DUSTPAN_P2: f32 = 1.15;
const DUSTPAN_WARN_END: f32 = 0.46;
const DUSTPAN_ACTIVE_END: f32 = 0.58;
const DUSTPAN_DAMAGE_P1: f32 = 22.0;
const DUSTPAN_DAMAGE_P2: f32 = 26.0;
const DUSTPAN_KNOCKBACK: f32 = 120.0;
const DUSTPAN_SHARD_PROGRESS: f32 = 0.48;
const DUSTPAN_SHARD_SPEED_P1: f32 = 48.0;
const DUSTPAN_SHARD_SPEED_P2: f32 = 60.0;
/// Art: shard angles are `k * TAU / 6 + 0.25`.
const DUSTPAN_SHARD_PHASE: f32 = 0.25;
const DUSTPAN_MIN_RANGE: f32 = 22.0;
const DUSTPAN_MAX_RANGE: f32 = 56.0;
/// The pan is unlocked only after the rat has seen this many rakes.
const DUSTPAN_RAKES_REQUIRED: f32 = 2.0;

/// `Hurt` hop (art: `1.20 s`). This is what the hop *asks* the shared control
/// budget for (`Boss::grant_control`); under sustained fire it runs for the
/// granted remainder, which is at most `CONTROL_BUDGET_SECS` a window.
const HURT: f32 = 1.20;
/// A burst of this fraction of max hp **in one beat** trips the art's heavy
/// stagger.
///
/// The pool it is a fraction of is the campaign boss's, not the art's base: the
/// anchor resets on every state entry (`enter`), so the burst window is one
/// beat (1.15-1.65 s), and the rat's measured damage into the Sweeper is
/// 200-480 dps — a beat is worth 250-700 HP, i.e. 1.0-2.8 % of the ~25 k pool.
/// At the old `0.18` the threshold was ~4 550 HP in one beat (~3 000 dps): the
/// hop never once fired on the whole bot ladder (0 of 72 Sweeper fights booked
/// any damage in `Hurt`), which left the boss's poise/stagger beat dead code.
/// `0.015` (~380 HP) reads a genuinely heavy burst from the build the campaign
/// actually produces, and the 6 s cooldown keeps it a reprieve rather than a
/// stun-lock.
const STAGGER_HP_FRAC: f32 = 0.015;
/// Minimum spacing between two hops, so a high-dps build cannot stun-lock
/// the human out of every attack (the reprieve stays a *reprieve*).
const STAGGER_COOLDOWN: f32 = 6.0;

/// `PhaseChange` (art: `1.80 s`, active `0.38..0.54`, `18 dmg`, `200 px/s`).
const PHASE_CHANGE: f32 = 1.80;
const PHASE_WARN_END: f32 = 0.38;
const PHASE_ACTIVE_END: f32 = 0.54;
const PHASE_SLAM_DAMAGE: f32 = 18.0;
const PHASE_SLAM_KNOCKBACK: f32 = 200.0;
/// The art's recommended incoming-damage cut during the slam.
const PHASE_SLAM_DAMAGE_MULT: f32 = 0.5;

/// Attack choice in the open-ended part of the rotation: one draw per
/// decision, below `SWEEP_ROLL` the human rakes, above `DUSTPAN_ROLL` it
/// fumbles for the pan (when unlocked and in range).
const SWEEP_ROLL: f32 = 0.55;
const DUSTPAN_ROLL: f32 = 0.85;
/// Decisions `0..OPENING_DECISIONS` are the scripted opening.
const OPENING_DECISIONS: u8 = 6;
/// A rake may start with the rat this far outside its outer radius: the
/// telegraph is long enough for it to step in by the active window.
const SWEEP_FIT_SLACK: f32 = 8.0;

/// [`Boss::flags`] layout.
const ARMED_BIT: u32 = 1 << 0;
/// The alternating stomping foot (`1` = right).
const FOOT_RIGHT_BIT: u32 = 1 << 1;
/// This beat already played its footfall/rake/impact cue.
const CUE_BIT: u32 = 1 << 2;
/// Two touchdown latches for the hurt hop.
const HOP_1_BIT: u32 = 1 << 3;
const HOP_2_BIT: u32 = 1 << 4;
/// The defeat clatter has been cued. Its own bit: `CUE_BIT` is the living
/// human's once-per-beat latch and is usually still set by the beat it died in
/// (the framework's death hand-over does not touch `flags`), which silenced
/// the clatter whenever the kill landed after that beat's own cue.
const DEFEAT_CUE_BIT: u32 = 1 << 5;
/// The half-health threshold was crossed during a beat that must not be
/// interrupted (the `Hurt` hop): the tantrum is owed and starts as soon as the
/// human is free. `BossOutput::phase_changed` is a one-tick pulse, so without
/// this the set piece was skipped for the whole fight whenever the rat crossed
/// 50 % inside its own punish window.
const PHASE_PENDING_BIT: u32 = 1 << 6;
/// The attack-decision index (the opening order) sits above the flags.
const DECISION_SHIFT: u32 = 8;
const DECISION_MASK: u32 = 0x3F;

/// [`Boss::scratch`] layout.
const S_STAGGER_CD: usize = 0;
const S_RAKES: usize = 1;
const S_POISE: usize = 2;

/// The hurt hop's two touchdown progresses (the art's `|sin(p·τ·2.5)|`
/// bounce bottoms out at `0.4` and `0.8`).
const HOP_TOUCHDOWNS: [f32; 2] = [0.4, 0.8];
/// The defeat walk-off's broom-drop beat (the vfx wants "once, ≈ `0.16`").
const DEFEAT_DROP_PROGRESS: f32 = 0.16;

// ============================================================================
// Controller
// ============================================================================

impl BossController for BroomController {
    fn kind(&self) -> BossKind {
        BossKind::Broom
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        // Stagger cooldown, seconds: the hop is available once it reaches zero.
        let stagger_ready = boss.scratch[S_STAGGER_CD] <= 0.0;
        boss.scratch[S_STAGGER_CD] = (boss.scratch[S_STAGGER_CD] - ctx.dt).max(0.0);

        let aim = Aim::new(ctx.player_pos, boss);
        // The arena is the boss room; outside it the human hunts through the
        // large-class flow field (see `BossContext::seek`).
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // ------------------------------------------------------------------
        // Phase 2 set piece: the art's anger tantrum, once per fight.
        // ------------------------------------------------------------------
        if out.phase_changed {
            boss.flags |= PHASE_PENDING_BIT;
        }
        if boss.flags & PHASE_PENDING_BIT != 0 && !busy(boss) {
            boss.flags &= !PHASE_PENDING_BIT;
            // Any wind-up it interrupts is cancelled with the pose it belonged
            // to (the same rule as the `Hurt` hop below).
            boss.hazards.clear();
            enter(boss, BroomBossState::PhaseChange, PHASE_CHANGE);
            return;
        }

        // ------------------------------------------------------------------
        // Heavy burst on a foot → the comic single-foot reprieve. Any wind-up
        // it interrupts is cancelled (`hazards.clear`), so a telegraph never
        // fires from a pose the rat can no longer see.
        //
        // Batch 13: the hop is *control*, so the shared budget pays for it
        // (`Boss::grant_control`) — the art's 1.20 s is what we ask for, the
        // granted remainder (at most `CONTROL_BUDGET_SECS` per 4 s) is what the
        // hop runs — and it may never land inside a committed beat: a rake that
        // is cancelled on its first frame can never fire, and a build with
        // enough burst dps was cancelling every tell. `poise_broken` is measured
        // from the beat's own anchor, so a refused hop leaves the burst owed. 
        // ------------------------------------------------------------------
        if stagger_ready && poise_broken(boss) {
            let granted = boss.grant_control(HURT);
            if granted > 0.0 {
                let feet = art_boss_broom::foot_hitboxes(&boss.broom_view());
                let left_foot_hit =
                    ctx.player_pos.dist_sq(feet[0].pos) <= ctx.player_pos.dist_sq(feet[1].pos);
                boss.hazards.clear();
                boss.scratch[S_STAGGER_CD] = STAGGER_COOLDOWN;
                enter(boss, BroomBossState::Hurt { left_foot_hit }, granted);
                return;
            }
        }

        match boss.state {
            // --------------------------------------------------------------
            // Idle: both slippers planted, weight shifting, broom resting.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::Idle) => {
                boss.facing = aim.toward;
                if boss.progress() >= 1.0 {
                    choose_and_begin(ctx, boss, &aim);
                }
            }

            // --------------------------------------------------------------
            // StompWalk: shadow where the foot will land, then the slam.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::StompWalk { .. }) => {
                boss.facing = aim.toward;
                if !armed(boss) {
                    arm(boss);
                    push_stomp_hazards(boss, aim.enraged);
                }
                let p = boss.progress();
                // The framework's body contact uses only grounded shapes, so
                // the raised slipper can neither bump the rat from overhead nor
                // attract auto-aim. The stomp's own crush + shockwave is the
                // whole damage of this beat, so the landing foot does not also
                // bump (a bump would spend the rat's i-frames and mask the
                // telegraphed slam).
                boss.no_contact();
                // The shadow hits the floor at `0.66`: the footfall cue.
                if p >= STOMP_WARN_END && !cue_fired(boss) {
                    fire_cue(boss, out, BossCue::BroomStomp);
                }
                // From the footfall on, the body marches after the planted
                // foot, so a stomp is also the human's walk.
                if p >= STOMP_WARN_END {
                    let to_land = boss.target - boss.pos;
                    if to_land.len_sq() > 1.0 {
                        let remaining = (boss.duration - boss.elapsed).max(1e-3);
                        let speed = (to_land.len() / remaining).min(STOMP_SETTLE_SPEED_MAX);
                        move_boss(ctx, boss, to_land, speed);
                    }
                }
                if boss.progress() >= 1.0 {
                    go_idle(boss, aim.enraged);
                }
            }

            // --------------------------------------------------------------
            // BroomSweep: the rake. Heading is locked for the whole beat, so
            // the painted sector, the hazard and the art agree and the rat can
            // circle behind the heels or tuck into the pocket.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::BroomSweep { clockwise }) => {
                if !armed(boss) {
                    arm(boss);
                    push_sweep_hazard(boss, aim.enraged);
                }
                // Home move: the art's safe pocket — the planted feet do not
                // crush while the bristles rake, so "under the human" is
                // genuinely safe.
                boss.no_contact();
                // The bristles sweep the telegraphed sector from `0.46`.
                if boss.progress() >= SWEEP_WARN_END && !cue_fired(boss) {
                    fire_cue(boss, out, BossCue::BroomSweep);
                }
                if boss.progress() >= 1.0 {
                    // Phase 2 answers a clockwise rake with a counter-clockwise
                    // one re-aimed at where the rat fled (the art's optional
                    // chained sweep).
                    if aim.enraged && clockwise {
                        begin_sweep(ctx, boss, &aim);
                    } else {
                        go_idle(boss, aim.enraged);
                    }
                }
            }

            // --------------------------------------------------------------
            // BroomJab: implemented, never scheduled in this build.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::BroomJab) => {
                if !armed(boss) {
                    arm(boss);
                    push_jab_hazards(boss, aim.enraged);
                }
                if boss.progress() >= 1.0 {
                    go_idle(boss, aim.enraged);
                }
            }

            // --------------------------------------------------------------
            // DustpanDrop: the tin pan falls onto `target`, then clatters.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::DustpanDrop) => {
                boss.facing = aim.toward;
                if !armed(boss) {
                    arm(boss);
                    push_dustpan_hazard(boss, aim.enraged);
                }
                // The shard ring is spawned on the frame that crosses the art's
                // impact progress, exactly once.
                let prev = previous_progress(boss, ctx.dt);
                if prev < DUSTPAN_SHARD_PROGRESS && boss.progress() >= DUSTPAN_SHARD_PROGRESS {
                    spawn_dustpan_shards(boss, aim.enraged, out);
                    fire_cue(boss, out, BossCue::BroomDustpan);
                }
                if boss.progress() >= 1.0 {
                    go_idle(boss, aim.enraged);
                }
            }

            // --------------------------------------------------------------
            // Hurt: the hopping reprieve. No attack, no hazard.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::Hurt { .. }) => {
                // The stubbed toe's bump: the art's `6 dmg`, not the shared 10.
                boss.set_contact_damage(HURT_CONTACT_DAMAGE);
                if !cue_fired(boss) {
                    fire_cue(boss, out, BossCue::BroomHurtHop);
                }
                // Two indignant touchdowns (`0.4` and `0.8`).
                let p = boss.progress();
                for (i, &at) in HOP_TOUCHDOWNS.iter().enumerate() {
                    let bit = if i == 0 { HOP_1_BIT } else { HOP_2_BIT };
                    if p >= at && boss.flags & bit == 0 {
                        boss.flags |= bit;
                        out.cue(BossCue::BroomHop);
                    }
                }
                if boss.progress() >= 1.0 {
                    go_idle(boss, aim.enraged);
                }
            }

            // --------------------------------------------------------------
            // PhaseChange: stamp, slam both feet and the broom butt down.
            // --------------------------------------------------------------
            BossState::Broom(BroomBossState::PhaseChange) => {
                if !armed(boss) {
                    arm(boss);
                    push_phase_slam_hazard(boss);
                }
                let p = boss.progress();
                if p >= PHASE_WARN_END && !cue_fired(boss) {
                    fire_cue(boss, out, BossCue::BroomSlam);
                }
                boss.damage_mult = if (PHASE_WARN_END..PHASE_ACTIVE_END).contains(&p) {
                    PHASE_SLAM_DAMAGE_MULT
                } else {
                    1.0
                };
                if p >= 1.0 {
                    go_idle(boss, true);
                }
            }

            // The shared system owns the defeat walk-off; its one cue lives in
            // `death_tick`.
            BossState::Broom(BroomBossState::Defeat) => {}
            // A boss of another kind never runs this controller.
            _ => {}
        }
    }

    /// The defeat walk-off: the human drops the broom and shuffles off. The
    /// art's beat is `2.40 s`; the clatter is cued once as the broom hits the
    /// floor (`progress ≈ 0.16`), then the framework reaps the boss.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.progress() >= DEFEAT_DROP_PROGRESS && boss.flags & DEFEAT_CUE_BIT == 0 {
            boss.flags |= DEFEAT_CUE_BIT;
            out.cue(BossCue::BroomDefeat);
        }
    }
}

// ============================================================================
// Per-tick inputs
// ============================================================================

/// The geometry one controller tick needs.
struct Aim {
    /// Unit direction to the rat (falls back to the current facing).
    toward: Vec2,
    /// Distance to the rat.
    dist: f32,
    /// Phase 2 (`hp_frac <= 0.50`).
    enraged: bool,
}

impl Aim {
    fn new(player: Vec2, boss: &Boss) -> Self {
        let to = player - boss.pos;
        Self {
            toward: unit_or(to, boss.facing),
            dist: to.len(),
            enraged: boss.phase >= 2,
        }
    }
}

// ============================================================================
// Scratch helpers (per-controller memory — see the module docs)
// ============================================================================

/// Has this beat already queued its one-shot (hazard / shards)?
#[inline]
fn armed(boss: &Boss) -> bool {
    boss.flags & ARMED_BIT != 0
}

/// Mark the one-shot as done for this beat.
#[inline]
fn arm(boss: &mut Boss) {
    boss.flags |= ARMED_BIT;
}

/// Has this beat already played its progress cue?
#[inline]
fn cue_fired(boss: &Boss) -> bool {
    boss.flags & CUE_BIT != 0
}

/// Play a once-per-beat cue and latch it.
#[inline]
fn fire_cue(boss: &mut Boss, out: &mut BossOutput, cue: BossCue) {
    boss.flags |= CUE_BIT;
    out.cue(cue);
}

/// The attack-decision index (the opening order).
#[inline]
fn decision(boss: &Boss) -> u8 {
    ((boss.flags >> DECISION_SHIFT) & DECISION_MASK) as u8
}

/// Store the next attack-decision index.
#[inline]
fn set_decision(boss: &mut Boss, index: u8) {
    boss.flags = (boss.flags & !(DECISION_MASK << DECISION_SHIFT))
        | (u32::from(index & DECISION_MASK as u8) << DECISION_SHIFT);
}

/// Which slipper is next to stomp. Consecutive stomps always swap sides, even
/// when other attacks sit between them (the art's "alternate `left_foot`").
#[inline]
fn next_stomp_is_right(boss: &Boss) -> bool {
    boss.flags & FOOT_RIGHT_BIT != 0
}

/// Rakes the rat has seen since the fight started.
#[inline]
fn rakes_seen(boss: &Boss) -> f32 {
    boss.scratch[S_RAKES]
}

/// Has the rat seen enough rakes to make the dustpan legible?
#[inline]
fn dustpan_unlocked(boss: &Boss) -> bool {
    rakes_seen(boss) >= DUSTPAN_RAKES_REQUIRED
}

/// Is the boss in a state that must not be interrupted (set pieces)?
#[inline]
fn busy(boss: &Boss) -> bool {
    matches!(
        boss.state,
        BossState::Broom(BroomBossState::Defeat)
            | BossState::Broom(BroomBossState::Hurt { .. })
            | BossState::Broom(BroomBossState::PhaseChange)
    )
}

/// Has a heavy burst landed since this beat started (the art's stagger)?
#[inline]
fn poise_broken(boss: &Boss) -> bool {
    (boss.scratch[S_POISE] - boss.hp) >= STAGGER_HP_FRAC * boss.max_hp
}

/// Enter a beat: reset its clock, its per-beat flags, the damage multiplier and
/// the poise anchor.
fn enter(boss: &mut Boss, state: BroomBossState, duration: f32) {
    boss.set_state(BossState::Broom(state), duration);
    boss.flags &= !(ARMED_BIT | CUE_BIT | HOP_1_BIT | HOP_2_BIT);
    boss.damage_mult = 1.0;
    boss.scratch[S_POISE] = boss.hp;
}

/// Recovery beat.
fn go_idle(boss: &mut Boss, enraged: bool) {
    enter(boss, BroomBossState::Idle, if enraged { IDLE_P2 } else { IDLE_P1 });
}

// ============================================================================
// Attack selection
// ============================================================================

/// Which attack the next decision picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Stomp,
    Sweep,
    Dustpan,
    Jab,
}

/// End of `Idle`: pick the next beat and start it. A beat whose footprint or
/// escape lane does not fit is replaced by a stomp, which is the human's walk
/// (it always fits: the landing point is snapped onto open floor).
fn choose_and_begin(ctx: &mut BossContext<'_>, boss: &mut Boss, aim: &Aim) {
    let index = decision(boss);
    let choice = pick(ctx, boss, aim, index);
    match choice {
        Choice::Sweep if sweep_fits(ctx, boss, aim) => begin_sweep(ctx, boss, aim),
        Choice::Dustpan if dustpan_point(ctx, boss, aim).is_some() => {
            begin_dustpan(ctx, boss, aim)
        }
        Choice::Jab if JAB_ENABLED && jab_fits(ctx, boss, aim) => begin_jab(boss, aim),
        _ => begin_stomp(ctx, boss, aim),
    }
    set_decision(boss, index.wrapping_add(1));
}

/// The decision rule.
///
/// The opening is scripted (designer: alternating stomps, then a rake, twice);
/// afterwards exactly one draw from the boss's own stream decides the beat, so
/// the number of rng draws per decision is constant and seed-reproducible.
fn pick(ctx: &mut BossContext<'_>, boss: &Boss, aim: &Aim, index: u8) -> Choice {
    if index < OPENING_DECISIONS {
        return match index {
            2 | 5 => Choice::Sweep,
            _ => Choice::Stomp,
        };
    }
    let roll = ctx.rng.random::<f32>();
    if JAB_ENABLED && aim.dist <= art_boss_broom::JAB_MAX_REACH && roll < 0.20 {
        Choice::Jab
    } else if aim.dist <= art_boss_broom::SWEEP_OUTER_RADIUS && roll < SWEEP_ROLL {
        Choice::Sweep
    } else if dustpan_unlocked(boss)
        && (DUSTPAN_MIN_RANGE..=DUSTPAN_MAX_RANGE).contains(&aim.dist)
        && roll < DUSTPAN_ROLL
    {
        Choice::Dustpan
    } else {
        Choice::Stomp
    }
}

// ============================================================================
// Fit tests — never start a move that does not fit where the boss stands
// ============================================================================

/// A rake needs the rat inside the annulus's reach, a lane *into* the pocket
/// and a lane back *out* of the annulus, plus the human's own footprint.
fn sweep_fits(ctx: &BossContext<'_>, boss: &Boss, aim: &Aim) -> bool {
    let radius = boss.kind.body_radius();
    aim.dist <= art_boss_broom::SWEEP_OUTER_RADIUS + SWEEP_FIT_SLACK
        && ctx.footprint_fits(boss.pos, radius)
        && ctx.attack_fits(
            boss.pos,
            radius,
            aim.toward,
            art_boss_broom::SWEEP_OUTER_RADIUS + PLAYER_RADIUS,
        )
        // "Leave an approach to that inner pocket clear" (designer).
        && ctx.los_clear(ctx.player_pos, boss.pos, PLAYER_RADIUS)
}

/// The pan needs an open 15 px drop circle and a clear lane off it.
fn dustpan_point(ctx: &BossContext<'_>, boss: &Boss, aim: &Aim) -> Option<Vec2> {
    let reach = aim.dist.clamp(DUSTPAN_MIN_RANGE, DUSTPAN_MAX_RANGE);
    let raw = boss.pos + aim.toward * reach;
    let lane = art_boss_broom::DUSTPAN_IMPACT_RADIUS + PLAYER_RADIUS * 4.0;
    [raw, ctx.nearest_open(raw)].into_iter().find(|point| {
        ctx.footprint_fits(*point, art_boss_broom::DUSTPAN_IMPACT_RADIUS)
            && ctx.los_clear(*point, *point + aim.toward * lane, PLAYER_RADIUS)
    })
}

/// The jab combo only fires at point-blank range with the rat in the open.
fn jab_fits(ctx: &BossContext<'_>, boss: &Boss, aim: &Aim) -> bool {
    aim.dist <= art_boss_broom::JAB_MAX_REACH
        && ctx.los_clear(boss.pos, ctx.player_pos, PLAYER_RADIUS)
        && ctx.footprint_fits(boss.pos, boss.kind.body_radius())
}

// ============================================================================
// Attack starts
// ============================================================================

/// `StompWalk`: raise one slipper and slam it onto the target the shadow has
/// been growing on. The foot lands where the rat *was* when the beat started.
fn begin_stomp(ctx: &BossContext<'_>, boss: &mut Boss, aim: &Aim) {
    // Alternate the foot on its own bit, so consecutive stomps always swap
    // sides even when other attacks sit between them.
    let left_foot = !next_stomp_is_right(boss);
    boss.flags ^= FOOT_RIGHT_BIT;
    let reach = aim.dist.clamp(STOMP_LAND_MIN, STOMP_STRIDE_MAX);
    let mut land = boss.pos + aim.toward * reach;
    // While both the human and the rat are in the arena the landing stays on
    // the encounter floor; a rat that has left the room is chased normally.
    if boss.arena.contains_world(boss.pos) && boss.arena.contains_world(ctx.player_pos) {
        land = clamp_to_arena(boss.arena, land, art_boss_broom::STOMP_SHOCKWAVE_RADIUS);
    }
    if !ctx.footprint_fits(land, art_boss_broom::STOMP_CRUSH_RADIUS) {
        land = ctx.nearest_open(land);
    }
    boss.target = land;
    boss.facing = unit_or(land - boss.pos, aim.toward);
    let duration = if aim.enraged { STOMP_P2 } else { STOMP_P1 };
    enter(boss, BroomBossState::StompWalk { left_foot }, duration);
}

/// `BroomSweep`: plant both feet and rake the annulus, starting on the flank
/// `clockwise` names.
fn begin_sweep(_ctx: &BossContext<'_>, boss: &mut Boss, aim: &Aim) {
    // Rakes alternate direction; the art draws the wind-up on the start flank.
    let clockwise = (rakes_seen(boss) as u32).is_multiple_of(2);
    boss.facing = aim.toward;
    boss.scratch[S_RAKES] = rakes_seen(boss) + 1.0;
    let duration = if aim.enraged { SWEEP_P2 } else { SWEEP_P1 };
    enter(boss, BroomBossState::BroomSweep { clockwise }, duration);
}

/// `DustpanDrop`: fumble the pan onto the point `dustpan_point` accepted.
fn begin_dustpan(ctx: &BossContext<'_>, boss: &mut Boss, aim: &Aim) {
    if let Some(point) = dustpan_point(ctx, boss, aim) {
        boss.target = point;
        boss.facing = unit_or(point - boss.pos, aim.toward);
    }
    let duration = if aim.enraged { DUSTPAN_P2 } else { DUSTPAN_P1 };
    enter(boss, BroomBossState::DustpanDrop, duration);
}

/// `BroomJab`: the cut three-hit combo (kept intact for the follow-up build).
#[cfg_attr(not(test), allow(dead_code))]
fn begin_jab(boss: &mut Boss, aim: &Aim) {
    boss.target = boss.pos + aim.toward * art_boss_broom::JAB_MAX_REACH;
    boss.facing = aim.toward;
    let duration = if aim.enraged { JAB_P2 } else { JAB_P1 };
    enter(boss, BroomBossState::BroomJab, duration);
}

// ============================================================================
// Hazards and projectiles
// ============================================================================

/// The art's crush circle and shockwave ring, both on the landing point.
fn push_stomp_hazards(boss: &mut Boss, enraged: bool) {
    let warn = STOMP_WARN_END * boss.duration;
    let active = (STOMP_ACTIVE_END - STOMP_WARN_END) * boss.duration;
    let crush = if enraged {
        STOMP_CRUSH_P2
    } else {
        STOMP_CRUSH_P1
    };
    let shock = if enraged {
        STOMP_SHOCK_P2
    } else {
        STOMP_SHOCK_P1
    };
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Circle {
                center: boss.target,
                radius: art_boss_broom::STOMP_CRUSH_RADIUS,
            },
            warn,
            active,
            crush,
        )
        .with_knockback(STOMP_KNOCKBACK)
        .with_enemies(),
    );
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Annulus {
                center: boss.target,
                inner: art_boss_broom::STOMP_CRUSH_RADIUS,
                outer: art_boss_broom::STOMP_SHOCKWAVE_RADIUS,
            },
            warn,
            active,
            shock,
        )
        .with_knockback(STOMP_KNOCKBACK)
        .with_enemies(),
    );
}

/// The rake's sector: the union of the art's broom-head travel.
///
/// `broom_sweep_angle` runs from `facing ± SWEEP_HALF_ARC` to the other flank
/// over the active window, so the static `Arc` below is exactly the painted
/// warning sector (and exactly where the head will be at some point).
fn push_sweep_hazard(boss: &mut Boss, enraged: bool) {
    let warn = SWEEP_WARN_END * boss.duration;
    let active = (SWEEP_ACTIVE_END - SWEEP_WARN_END) * boss.duration;
    let damage = if enraged {
        SWEEP_DAMAGE_P2
    } else {
        SWEEP_DAMAGE_P1
    };
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Arc {
                center: boss.pos,
                inner: art_boss_broom::SWEEP_INNER_RADIUS,
                outer: art_boss_broom::SWEEP_OUTER_RADIUS,
                facing: boss.facing,
                half_angle: art_boss_broom::SWEEP_HALF_ARC,
            },
            warn,
            active,
            damage,
        )
        .with_knockback(SWEEP_KNOCKBACK)
        .with_enemies(),
    );
}

/// The three jab impacts, one circle each on the art's reticles.
fn push_jab_hazards(boss: &mut Boss, enraged: bool) {
    let f = boss.facing;
    let perp = f.perp();
    let third = if (boss.target - boss.pos).len_sq() > 64.0 {
        boss.pos + (boss.target - boss.pos).clamp_len(art_boss_broom::JAB_MAX_REACH)
    } else {
        boss.pos + f * art_boss_broom::JAB_MAX_REACH
    };
    let spots = [
        boss.pos + f * JAB_REACH_1 - perp * JAB_SIDE,
        boss.pos + f * JAB_REACH_2 + perp * JAB_SIDE,
        third,
    ];
    let damage = if enraged {
        JAB_DAMAGE_P2
    } else {
        JAB_DAMAGE_P1
    };
    for (i, spot) in spots.iter().enumerate() {
        let warn = JAB_WARN_END[i] * boss.duration;
        let active = (JAB_ACTIVE_END[i] - JAB_WARN_END[i]) * boss.duration;
        let _ = boss.push_hazard(
            BossHazard::new(
                HazardShape::Circle {
                    center: *spot,
                    radius: art_boss_broom::JAB_HEAD_RADIUS,
                },
                warn,
                active,
                damage,
            )
            .with_knockback(JAB_KNOCKBACK),
        );
    }
}

/// The tin pan's slam circle.
fn push_dustpan_hazard(boss: &mut Boss, enraged: bool) {
    let warn = DUSTPAN_WARN_END * boss.duration;
    let active = (DUSTPAN_ACTIVE_END - DUSTPAN_WARN_END) * boss.duration;
    let damage = if enraged {
        DUSTPAN_DAMAGE_P2
    } else {
        DUSTPAN_DAMAGE_P1
    };
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Circle {
                center: boss.target,
                radius: art_boss_broom::DUSTPAN_IMPACT_RADIUS,
            },
            warn,
            active,
            damage,
        )
        .with_knockback(DUSTPAN_KNOCKBACK)
        .with_enemies(),
    );
}

/// The pan's radial clatter, on the art's `k * TAU / 6 + 0.25` angles.
fn spawn_dustpan_shards(boss: &Boss, enraged: bool, out: &mut BossOutput) {
    let speed = if enraged {
        DUSTPAN_SHARD_SPEED_P2
    } else {
        DUSTPAN_SHARD_SPEED_P1
    };
    let count = art_boss_broom::DUSTPAN_SHARD_COUNT;
    for k in 0..count {
        let angle = k as f32 * TAU / count as f32 + DUSTPAN_SHARD_PHASE;
        out.projectiles.push(BossProjectileSpawn::new(
            BossProjectileKind::DustpanShard,
            boss.target,
            Vec2::from_angle(angle) * speed,
        ));
    }
}

/// The double-foot tantrum slam around the human.
fn push_phase_slam_hazard(boss: &mut Boss) {
    let warn = PHASE_WARN_END * boss.duration;
    let active = (PHASE_ACTIVE_END - PHASE_WARN_END) * boss.duration;
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Circle {
                center: boss.pos,
                radius: art_boss_broom::PHASE_SLAM_RADIUS,
            },
            warn,
            active,
            PHASE_SLAM_DAMAGE,
        )
        .with_knockback(PHASE_SLAM_KNOCKBACK)
        .with_enemies(),
    );
}

// ============================================================================
// Small helpers
// ============================================================================

/// Unit vector, or `fallback` when `v` is degenerate.
#[inline]
fn unit_or(v: Vec2, fallback: Vec2) -> Vec2 {
    let u = v.normalized();
    if u.len_sq() > 0.0 {
        u
    } else {
        fallback
    }
}

/// Progress at the *start* of this tick (the framework has already advanced
/// the state clock by `dt`).
#[inline]
fn previous_progress(boss: &Boss, dt: f32) -> f32 {
    if boss.duration <= 0.0 {
        return 1.0;
    }
    ((boss.elapsed - dt) / boss.duration).clamp(0.0, 1.0)
}

/// Keep a landing point on the encounter floor, `margin` inside the arena.
fn clamp_to_arena(arena: TileRect, land: Vec2, margin: f32) -> Vec2 {
    let r = arena.to_world();
    if r.w <= margin * 2.0 + 1.0 || r.h <= margin * 2.0 + 1.0 {
        return land;
    }
    Vec2::new(
        land.x.clamp(r.x + margin, r.right() - margin),
        land.y.clamp(r.y + margin, r.bottom() - margin),
    )
}


// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;
    use crate::game::ecs::systems::player::player_system;
    use crate::game::level::{Level, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::nav::{FlowField, NavBudget};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    const TICKS_PER_SEC: usize = 60;
    /// The arena's middle in world pixels (the empty grid is 200 × 160 cells).
    const CENTER: Vec2 = Vec2 { x: 1600.0, y: 1280.0 };
    /// Sustained damage the "is it beatable" script pours into the boss.
    const SCRIPTED_DPS: f32 = 400.0;

    /// A controlled kitchen: an empty tile grid, one Sweeper and the two
    /// systems that matter (the rat's own and the boss's), ticked through the
    /// narrow `begin_tick`/`end_tick` harness.
    struct Rig {
        w: World,
        arena: TileRect,
    }

    impl Rig {
        fn new(seed: u64) -> Self {
            let mut w = World::with_tuning(
                Theme::Kitchen, seed,
                crate::tuning::RunTuning::with_base_boss_hazards(BossKind::Broom),
            );
            w.level = Level::empty(Theme::Kitchen);
            w.flow = FlowField::new(&w.level.tiles);
            w.flow.update(&w.level.tiles, CENTER, NavBudget::FULL);
            w.ecs.resource_mut::<BossFight>().rng = BossRng::for_run(seed, BossKind::Broom);
            let arena = TileRect::new(0, 0, MAP_COLS, MAP_ROWS);
            let mut boss = Boss::new(
                BossKind::Broom,
                CENTER,
                BossKind::Broom.base_hp(),
                arena,
                arena.center_world(),
                false,
            );
            boss.id = 1;
            boss.facing = Vec2::new(1.0, 0.0);
            w.ecs.spawn((boss, SpawnId(0)));
            w.player.pos = CENTER + Vec2::new(30.0, 0.0);
            Self { w, arena }
        }

        fn tick_with(&mut self, move_dir: Vec2) {
            self.w.begin_tick(DT, move_dir, false);
            self.w
                .ecs
                .run_system_cached(player_system)
                .expect("player system");
            self.w
                .ecs
                .run_system_cached(boss_update_system)
                .expect("boss update system");
            self.w.end_tick();
        }

        fn tick(&mut self) {
            self.tick_with(Vec2::ZERO);
        }

        fn boss(&mut self) -> Boss {
            let mut query = self.w.ecs.query::<&Boss>();
            query
                .iter(&self.w.ecs)
                .next()
                .cloned()
                .expect("the Sweeper is in the world")
        }

        fn has_boss(&mut self) -> bool {
            let mut query = self.w.ecs.query::<&Boss>();
            query.iter(&self.w.ecs).next().is_some()
        }

        fn edit<R>(&mut self, f: impl FnOnce(&mut Boss) -> R) -> R {
            let mut query = self.w.ecs.query::<&mut Boss>();
            let mut iter = query.iter_mut(&mut self.w.ecs);
            let mut boss = iter.next().expect("the Sweeper is in the world");
            f(&mut boss)
        }

        fn state(&mut self) -> BroomBossState {
            match self.boss().state {
                BossState::Broom(s) => s,
                other => panic!("the Sweeper is in a {other:?} state"),
            }
        }

        fn set_player(&mut self, pos: Vec2) {
            self.w.player.pos = pos;
            self.w.player.vel = Vec2::ZERO;
        }

        fn player_pos(&self) -> Vec2 {
            self.w.player.pos
        }

        fn hp(&self) -> f32 {
            self.w.player.hp
        }

        /// Paint solid rock over a block of cells (and refresh the masks).
        fn paint(&mut self, cx: i32, cy: i32, cw: i32, ch: i32) {
            self.w
                .level
                .tiles
                .fill_cells(cx, cy, cw, ch, TileKind::Wall(WallKind::Solid));
            self.w.level.tiles.rebuild_masks();
            let center = self.player_pos();
            self.w.flow = FlowField::new(&self.w.level.tiles);
            self.w.flow.update(&self.w.level.tiles, center, NavBudget::FULL);
        }

        /// Tick until `done` holds; returns the number of ticks it took.
        fn run_until(&mut self, max: usize, mut done: impl FnMut(&mut Self) -> bool) -> usize {
            for i in 0..max {
                if done(self) {
                    return i;
                }
                self.tick();
            }
            max
        }

        /// Attack states only (Idle skipped), for cadence assertions.
        fn attack_states(&mut self, ticks: usize) -> Vec<BroomBossState> {
            let mut seen = Vec::new();
            let mut last = self.state();
            for _ in 0..ticks {
                self.tick();
                let now = self.state();
                if now != last && now != BroomBossState::Idle {
                    seen.push(now);
                }
                last = now;
            }
            seen
        }

        /// Hold the rat a fixed distance in front of the human (a kiting
        /// script: the cadence tests need the distance to stay in range).
        fn tick_kiting(&mut self, dist: f32) {
            let boss = self.boss();
            let target = boss.pos + Vec2::new(dist, 0.0);
            self.set_player(target);
            self.tick();
        }

        fn shards(&mut self) -> usize {
            let mut query = self.w.ecs.query::<&BossProjectile>();
            query
                .iter(&self.w.ecs)
                .filter(|p| p.kind == BossProjectileKind::DustpanShard)
                .count()
        }
    }

    /// Damage the rat takes this tick, if any.
    fn hp_drop(before: f32, after: f32) -> f32 {
        (before - after).max(0.0)
    }

    // ---- the art contract ---------------------------------------------------

    #[test]
    fn beat_timings_and_numbers_are_the_arts() {
        // Durations (art_boss_broom's per-state doc comments).
        assert_eq!(IDLE_P1, 0.55);
        assert_eq!(IDLE_P2, 0.32);
        assert_eq!(STOMP_P1, 1.15);
        assert_eq!(STOMP_P2, 0.85);
        assert_eq!(SWEEP_P1, 1.65);
        assert_eq!(SWEEP_P2, 1.25);
        assert_eq!(JAB_P1, 1.80);
        assert_eq!(JAB_P2, 1.40);
        assert_eq!(DUSTPAN_P1, 1.45);
        assert_eq!(DUSTPAN_P2, 1.15);
        assert_eq!(HURT, 1.20);
        assert_eq!(PHASE_CHANGE, 1.80);
        assert_eq!(BossKind::Broom.death_duration(), 2.40);
        // Damage.
        assert_eq!(STOMP_CRUSH_P1, 26.0);
        assert_eq!(STOMP_CRUSH_P2, 30.0);
        assert_eq!(STOMP_SHOCK_P1, 16.0);
        assert_eq!(STOMP_SHOCK_P2, 20.0);
        assert_eq!(SWEEP_DAMAGE_P1, 20.0);
        assert_eq!(SWEEP_DAMAGE_P2, 24.0);
        assert_eq!(JAB_DAMAGE_P1, 14.0);
        assert_eq!(JAB_DAMAGE_P2, 18.0);
        assert_eq!(DUSTPAN_DAMAGE_P1, 22.0);
        assert_eq!(DUSTPAN_DAMAGE_P2, 26.0);
        assert_eq!(PHASE_SLAM_DAMAGE, 18.0);
        // Warning windows, in seconds, as the art quotes them.
        // The art quotes 0.67 s of *rise* (progress 0.00..0.58); the shadow
        // keeps growing until the slam at 0.66, so the telegraph is longer.
        assert!((STOMP_FOOTFALL * STOMP_P1 - 0.67).abs() < 0.02, "0.67 s stomp warn");
        // Bound through a binding so this stays a runtime assertion.
        let stomp_telegraph = STOMP_WARN_END * STOMP_P1;
        assert!(
            stomp_telegraph >= 0.67,
            "the stomp telegraphs for {stomp_telegraph:.2} s, under the art's 0.67 s"
        );
        assert!((SWEEP_WARN_END * SWEEP_P1 - 0.76).abs() < 0.02, "0.76 s rake warn");
        assert!((DUSTPAN_WARN_END * DUSTPAN_P1 - 0.67).abs() < 0.02, "0.67 s pan warn");
        assert!((PHASE_WARN_END * PHASE_CHANGE - 0.68).abs() < 0.02, "0.68 s slam warn");
        // Active windows, in seconds.
        assert!(((STOMP_ACTIVE_END - STOMP_WARN_END) * STOMP_P1 - 0.14).abs() < 0.02);
        assert!(((SWEEP_ACTIVE_END - SWEEP_WARN_END) * SWEEP_P1 - 0.43).abs() < 0.02);
        assert!(((DUSTPAN_ACTIVE_END - DUSTPAN_WARN_END) * DUSTPAN_P1 - 0.17).abs() < 0.02);
        assert!(((PHASE_ACTIVE_END - PHASE_WARN_END) * PHASE_CHANGE - 0.29).abs() < 0.02);
        // The three-jab combo is cut from this build (designer).
        let jab_enabled = JAB_ENABLED;
        assert!(!jab_enabled, "the jab combo must stay unscheduled");
    }

    #[test]
    fn every_state_runs_for_its_art_duration() {
        let mut rig = Rig::new(7);
        // Keep the rat far away and harmless: no hazards can land on it here.
        rig.set_player(CENTER + Vec2::new(300.0, 0.0));
        let cases = [
            (BroomBossState::Idle, IDLE_P1),
            (BroomBossState::StompWalk { left_foot: true }, STOMP_P1),
            (BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1),
            (BroomBossState::BroomJab, JAB_P1),
            (BroomBossState::DustpanDrop, DUSTPAN_P1),
            (BroomBossState::Hurt { left_foot_hit: true }, HURT),
            (BroomBossState::PhaseChange, PHASE_CHANGE),
        ];
        for (state, duration) in cases {
            rig.edit(|b| {
                b.target = b.pos + Vec2::new(20.0, 0.0);
                b.facing = Vec2::new(1.0, 0.0);
                enter(b, state, duration)
            });
            let start = rig.state();
            let ticks = rig.run_until(400, |r| r.state() != start);
            let ran = ticks as f32 * DT;
            assert!(
                (ran - duration).abs() <= 2.0 * DT + 1e-4,
                "{state:?} ran {ran:.3} s, the art says {duration:.2} s"
            );
            // Every attack hands back to the recovery beat (`Idle` itself hands
            // straight on to the next attack).
            assert_ne!(rig.state(), start, "{state:?} must end");
        }
    }

    // ---- cadence ------------------------------------------------------------

    #[test]
    fn opening_leads_with_alternating_stomps_then_a_rake_twice() {
        let mut rig = Rig::new(11);
        let mut seen = Vec::new();
        // Six beats of the opening, held at 26 px so a rake is always legal.
        let mut last = rig.state();
        for _ in 0..60 * TICKS_PER_SEC {
            rig.tick_kiting(26.0);
            let now = rig.state();
            if now != last {
                if now != BroomBossState::Idle {
                    seen.push(now);
                }
                last = now;
            }
            if seen.len() >= 6 {
                break;
            }
        }
        assert!(seen.len() >= 6, "only saw {seen:?}");
        assert_eq!(
            seen[..6],
            [
                BroomBossState::StompWalk { left_foot: true },
                BroomBossState::StompWalk { left_foot: false },
                BroomBossState::BroomSweep { clockwise: true },
                BroomBossState::StompWalk { left_foot: true },
                BroomBossState::StompWalk { left_foot: false },
                BroomBossState::BroomSweep { clockwise: false },
            ],
            "the opening order is the designer's: stomp, stomp, rake ×2"
        );
    }

    #[test]
    fn the_dustpan_waits_for_two_rakes_and_the_jab_never_appears() {
        for seed in 0..6 {
            let mut rig = Rig::new(seed);
            let mut rakes = 0;
            let mut pans = 0;
            let mut last = rig.state();
            for _ in 0..90 * TICKS_PER_SEC {
                rig.tick_kiting(30.0);
                let now = rig.state();
                if now != last {
                    match now {
                        BroomBossState::BroomSweep { .. } => rakes += 1,
                        BroomBossState::DustpanDrop => {
                            pans += 1;
                            assert!(
                                rakes >= DUSTPAN_RAKES_REQUIRED as usize,
                                "seed {seed}: the pan dropped after {rakes} rake(s)"
                            );
                        }
                        BroomBossState::BroomJab => {
                            panic!("seed {seed}: the cut jab combo was scheduled")
                        }
                        _ => {}
                    }
                }
                last = now;
            }
            assert!(rakes >= 2, "seed {seed} never raked twice");
            if seed == 0 {
                assert!(pans > 0, "seed 0 never dropped the dustpan in 90 s");
            }
        }
    }

    // ---- stomp --------------------------------------------------------------

    #[test]
    fn stomp_warns_lands_once_and_only_inside_the_ring() {
        let mut rig = Rig::new(21);
        // Decision 0 is a stomp; the rat sits one stride ahead.
        rig.edit(|b| set_decision(b, 0));
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.run_until(120, |r| r.state() == BroomBossState::StompWalk { left_foot: true });
        let boss = rig.boss();
        let land = boss.target;
        assert!(
            land.dist(CENTER + Vec2::new(20.0, 0.0)) < 1e-3,
            "the shadow locks on the rat's position at the start of the beat"
        );
        // Stand on the landing point and count what lands.
        rig.set_player(land);
        let mut hits = 0;
        let mut total = 0.0;
        let mut hit_at = None;
        for _ in 0..((STOMP_P1 / DT) as usize + 4) {
            let before = rig.hp();
            rig.tick();
            let drop = hp_drop(before, rig.hp());
            if drop > 0.0 {
                hits += 1;
                total += drop;
                hit_at.get_or_insert(rig.boss().progress());
            }
        }
        assert_eq!(hits, 1, "the slam must land exactly once");
        assert!((total - STOMP_CRUSH_P1).abs() < 1e-3, "crush damage was {total}");
        let p = hit_at.expect("the slam landed");
        assert!(
            (STOMP_WARN_END..=STOMP_ACTIVE_END + 0.02).contains(&p),
            "the crush landed at progress {p:.3}, outside 0.66..0.78"
        );
    }

    #[test]
    fn stomp_shockwave_ring_catches_12_px_but_not_25_px() {
        for (offset, want) in [(12.0, STOMP_SHOCK_P1), (25.0, 0.0)] {
            let mut rig = Rig::new(23);
            rig.set_player(CENTER + Vec2::new(20.0, 0.0));
            rig.run_until(120, |r| r.state() == BroomBossState::StompWalk { left_foot: true });
            let land = rig.boss().target;
            rig.set_player(land + Vec2::new(offset, 0.0));
            let mut total = 0.0;
            for _ in 0..((STOMP_P1 / DT) as usize + 4) {
                let before = rig.hp();
                rig.tick();
                total += hp_drop(before, rig.hp());
            }
            assert!(
                (total - want).abs() < 1e-3,
                "at {offset} px the stomp dealt {total}, expected {want}"
            );
        }
    }

    #[test]
    fn walking_out_of_the_telegraphed_ring_avoids_the_stomp() {
        let mut rig = Rig::new(27);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.run_until(120, |r| r.state() == BroomBossState::StompWalk { left_foot: true });
        let land = rig.boss().target;
        // The rat is on the shadow and walks away at its own speed, exactly as
        // the art's counterplay promises (0.67 s covers ~39 px).
        let mut total = 0.0;
        for _ in 0..((STOMP_P1 / DT) as usize + 4) {
            let before = rig.hp();
            let away = unit_or(rig.player_pos() - land, Vec2::new(0.0, 1.0));
            rig.tick_with(away);
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "the walking rat still took {total}");
    }

    // ---- rake ---------------------------------------------------------------

    #[test]
    fn rake_hazard_spans_the_arts_broom_sweep_angle() {
        let mut rig = Rig::new(31);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        rig.tick();
        let boss = rig.boss();
        let hazard = boss
            .hazards
            .iter()
            .find(|h| matches!(h.shape, HazardShape::Arc { .. }))
            .copied()
            .expect("the rake queues an arc");
        match hazard.shape {
            HazardShape::Arc {
                center,
                inner,
                outer,
                facing,
                half_angle,
            } => {
                assert_eq!(center, boss.pos);
                assert_eq!(inner, art_boss_broom::SWEEP_INNER_RADIUS);
                assert_eq!(outer, art_boss_broom::SWEEP_OUTER_RADIUS);
                assert_eq!(half_angle, art_boss_broom::SWEEP_HALF_ARC);
                assert_eq!(facing, boss.facing);
            }
            _ => unreachable!(),
        }
        // The art's broom head starts on one flank at the top of the active
        // window and finishes on the other: the arc above is exactly its
        // travel, and `broom_sweep_angle` proves it.
        let mut at = |progress: f32| {
            let mut b = rig.boss();
            b.elapsed = progress * b.duration;
            let view = b.broom_view();
            (art_boss_broom::broom_sweep_angle(&view) - b.facing.angle()).abs() % TAU
        };
        let start = at(SWEEP_WARN_END);
        // The art's ease ends exactly at the active window's end; sample just
        // inside it so the settled angle is the one the arrival pose holds.
        let end = at(SWEEP_ACTIVE_END - 1e-3);
        assert!(
            (start - art_boss_broom::SWEEP_HALF_ARC).abs() < 1e-3,
            "the head starts {start:.3} rad off facing"
        );
        assert!(
            (end - art_boss_broom::SWEEP_HALF_ARC).abs() < 1e-3,
            "the head ends {end:.3} rad off facing"
        );
    }

    #[test]
    fn the_home_move_pocket_is_safe_under_the_human_during_a_rake() {
        // (a) Sheltering inside the inner radius: the designer's home move.
        let mut rig = Rig::new(37);
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        let mut total = 0.0;
        for _ in 0..((SWEEP_P1 / DT) as usize + 4) {
            let anchor = rig.boss().pos;
            rig.set_player(anchor);
            let before = rig.hp();
            rig.tick();
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "the pocket under the human took {total}");

        // (b) Standing in the annulus in front: the rake hits, once.
        let mut rig = Rig::new(37);
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        let anchor = rig.boss().pos + Vec2::new(25.0, 0.0);
        let mut hits = 0;
        let mut total = 0.0;
        for _ in 0..((SWEEP_P1 / DT) as usize + 4) {
            rig.set_player(anchor);
            let before = rig.hp();
            rig.tick();
            let drop = hp_drop(before, rig.hp());
            if drop > 0.0 {
                hits += 1;
                total += drop;
            }
        }
        assert_eq!(hits, 1, "the rake must hit once, not {hits} times");
        assert!((total - SWEEP_DAMAGE_P1).abs() < 1e-3, "rake damage {total}");
    }

    #[test]
    fn the_rake_ignores_the_rat_behind_the_heels() {
        let mut rig = Rig::new(41);
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        let behind = rig.boss().pos - Vec2::new(25.0, 0.0);
        let mut total = 0.0;
        for _ in 0..((SWEEP_P1 / DT) as usize + 4) {
            rig.set_player(behind);
            let before = rig.hp();
            rig.tick();
            total += hp_drop(before, rig.hp());
        }
        assert!(total == 0.0, "behind the heels took {total}");
    }

    #[test]
    fn phase_two_chains_one_counter_rake() {
        // Phase 2: a clockwise rake is answered by a counter-clockwise one.
        let mut rig = Rig::new(43);
        rig.edit(|b| {
            b.phase = 2;
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P2)
        });
        rig.run_until(200, |r| r.state() != BroomBossState::BroomSweep { clockwise: true });
        assert_eq!(rig.state(), BroomBossState::BroomSweep { clockwise: false });
        // …and the counter-rake recovers instead of chaining forever.
        rig.run_until(200, |r| !matches!(r.state(), BroomBossState::BroomSweep { .. }));
        assert_eq!(rig.state(), BroomBossState::Idle);

        // Phase 1 does not chain.
        let mut rig = Rig::new(43);
        rig.edit(|b| {
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        rig.run_until(200, |r| !matches!(r.state(), BroomBossState::BroomSweep { .. }));
        assert_eq!(rig.state(), BroomBossState::Idle);
    }

    // ---- jab (implemented, never scheduled) --------------------------------

    #[test]
    fn the_cut_jab_combo_still_works_when_driven_directly() {
        let mut rig = Rig::new(47);
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            b.target = b.pos + Vec2::new(25.0, 0.0);
            enter(b, BroomBossState::BroomJab, JAB_P1)
        });
        rig.tick();
        let hazards: Vec<BossHazard> = rig
            .boss()
            .hazards
            .iter()
            .filter(|h| matches!(h.shape, HazardShape::Circle { .. }))
            .copied()
            .collect();
        assert_eq!(hazards.len(), 3, "one impact per jab");
        for (i, hazard) in hazards.iter().enumerate() {
            assert!((hazard.warn - JAB_WARN_END[i] * JAB_P1).abs() < 1e-3);
            assert!((hazard.active - (JAB_ACTIVE_END[i] - JAB_WARN_END[i]) * JAB_P1).abs() < 1e-3);
            assert_eq!(hazard.damage, JAB_DAMAGE_P1);
            match hazard.shape {
                HazardShape::Circle { radius, .. } => {
                    assert_eq!(radius, art_boss_broom::JAB_HEAD_RADIUS)
                }
                _ => unreachable!(),
            }
        }
        // Each impact lands inside its own beat, as the art's beat table says.
        for i in 0..JAB_BEATS.len() {
            let beat_end = if i + 1 < JAB_BEATS.len() {
                JAB_BEATS[i + 1]
            } else {
                1.0
            };
            assert!(JAB_BEATS[i] <= JAB_WARN_END[i]);
            assert!(JAB_ACTIVE_END[i] <= beat_end);
        }
        // Second jab is on the other side of the facing axis, third is the
        // deep lunge clamped to the art's reach.
        let boss = rig.boss();
        let f = boss.facing;
        let spots: Vec<Vec2> = hazards
            .iter()
            .map(|h| h.shape.center())
            .collect();
        assert!((spots[0] - boss.pos).dot(f.perp()) < 0.0);
        assert!((spots[1] - boss.pos).dot(f.perp()) > 0.0);
        assert!((spots[2] - boss.pos).len() <= art_boss_broom::JAB_MAX_REACH + 1e-3);
        // It runs its beat and recovers.
        rig.run_until(200, |r| r.state() == BroomBossState::Idle);
    }

    // ---- dustpan ------------------------------------------------------------

    #[test]
    fn dustpan_slams_once_and_scatters_six_shards() {
        let mut rig = Rig::new(53);
        let drop = CENTER + Vec2::new(34.0, 0.0);
        rig.edit(|b| {
            b.target = drop;
            enter(b, BroomBossState::DustpanDrop, DUSTPAN_P1)
        });
        let mut hits = 0;
        let mut total = 0.0;
        let mut shards_at = None;
        for i in 0..((DUSTPAN_P1 / DT) as usize + 4) {
            rig.set_player(drop);
            let before = rig.hp();
            rig.tick();
            let drop_hp = hp_drop(before, rig.hp());
            if drop_hp > 0.0 {
                hits += 1;
                total += drop_hp;
            }
            if shards_at.is_none() && rig.shards() > 0 {
                shards_at = Some((i, rig.shards()));
            }
        }
        assert_eq!(hits, 1, "the pan must slam exactly once");
        assert!((total - DUSTPAN_DAMAGE_P1).abs() < 1e-3, "pan damage {total}");
        let (tick, count) = shards_at.expect("the pan must clatter");
        let at = tick as f32 * DT;
        assert!(
            (at - DUSTPAN_SHARD_PROGRESS * DUSTPAN_P1).abs() < 2.0 * DT,
            "shards spawned at {at:.3} s, the art says {:.3} s",
            DUSTPAN_SHARD_PROGRESS * DUSTPAN_P1
        );
        assert_eq!(count, art_boss_broom::DUSTPAN_SHARD_COUNT);
        // The art's shard numbers ride on the shared projectile kind.
        assert_eq!(BossProjectileKind::DustpanShard.radius(), 2.0);
        assert_eq!(BossProjectileKind::DustpanShard.damage(), 10.0);
    }

    // ---- hurt ---------------------------------------------------------------

    #[test]
    fn a_heavy_burst_trips_the_hurt_hop_in_the_idle_beat_only() {
        // Batch 13: the hop is control on the shared budget, and a committed
        // beat is never cancelled. Both halves are pinned here.
        let mut rig = Rig::new(59);
        // With `facing = (1, 0)` the left slipper sits on `-perp`, i.e. up-screen.
        rig.set_player(CENTER + Vec2::new(25.0, -12.0));
        rig.edit(|b| {
            b.facing = Vec2::new(1.0, 0.0);
            enter(b, BroomBossState::BroomSweep { clockwise: true }, SWEEP_P1)
        });
        rig.tick();
        assert!(!rig.boss().hazards.is_empty(), "the rake queued its telegraph");
        // 1.5 % of max hp in one burst — the art's heavy stagger — on the rat's
        // side of the stance. It must NOT cancel the rake.
        rig.edit(|b| {
            b.hp -= STAGGER_HP_FRAC * b.max_hp + 1.0;
        });
        rig.tick();
        assert!(
            matches!(rig.state(), BroomBossState::BroomSweep { .. }),
            "a heavy burst cancelled a committed rake (the bosslock bug)"
        );
        assert!(
            !rig.boss().hazards.is_empty(),
            "the rake's telegraph is still owed"
        );

        // From the neutral beat the hop does fire, for what the budget pays.
        // (`enter` re-anchors the poise, so the burst lands after it.)
        rig.edit(|b| {
            enter(b, BroomBossState::Idle, IDLE_P1);
            b.hp -= STAGGER_HP_FRAC * b.max_hp + 1.0;
        });
        rig.tick();
        let boss = rig.boss();
        assert!(
            matches!(
                boss.state,
                BossState::Broom(BroomBossState::Hurt { left_foot_hit: true })
            ),
            "the rat is on the left slipper's side ({:?})",
            boss.state
        );
        assert!(
            boss.scratch[S_STAGGER_CD] <= STAGGER_COOLDOWN + 1e-3,
            "the stagger cooldown restarts"
        );
        // The art's 1.20 s is clipped to the control budget.
        let hop = boss.duration;
        assert!(
            (hop - CONTROL_BUDGET_SECS).abs() <= DT + 1e-4,
            "the hop runs for the granted budget ({hop})"
        );
        let ticks = rig.run_until(200, |r| !matches!(r.state(), BroomBossState::Hurt { .. }));
        assert!((ticks as f32 * DT - hop).abs() <= 2.0 * DT + 1e-4);
        // A second burst inside the cooldown does not re-trip it.
        rig.edit(|b| b.hp -= STAGGER_HP_FRAC * b.max_hp + 1.0);
        rig.tick();
        assert!(!matches!(rig.state(), BroomBossState::Hurt { .. }));
    }

    // ---- phase change -------------------------------------------------------

    #[test]
    fn the_anger_tantrum_plays_once_and_halves_incoming_damage() {
        let mut rig = Rig::new(61);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.edit(|b| b.hp = b.max_hp * 0.49);
        rig.tick();
        assert_eq!(rig.state(), BroomBossState::PhaseChange);
        assert_eq!(rig.boss().phase, 2);
        rig.tick();
        let hazard = rig
            .boss()
            .hazards
            .iter()
            .find(|h| matches!(h.shape, HazardShape::Circle { .. }))
            .copied()
            .expect("the tantrum queues its slam");
        match hazard.shape {
            HazardShape::Circle { center, radius } => {
                assert_eq!(radius, art_boss_broom::PHASE_SLAM_RADIUS);
                assert_eq!(center.x, CENTER.x);
            }
            _ => unreachable!(),
        }
        assert!((hazard.warn - PHASE_WARN_END * PHASE_CHANGE).abs() < 1e-3);
        assert_eq!(hazard.damage, PHASE_SLAM_DAMAGE);
        // The art's damage cut covers the slam window only.
        for _ in 0..((PHASE_CHANGE / DT) as usize) {
            let (p, mult) = {
                let boss = rig.boss();
                (boss.progress(), boss.damage_mult)
            };
            if (PHASE_WARN_END..PHASE_ACTIVE_END).contains(&p) {
                assert_eq!(mult, PHASE_SLAM_DAMAGE_MULT, "phase slam must halve incoming");
            } else {
                assert_eq!(mult, 1.0);
            }
            rig.tick();
        }
        rig.run_until(200, |r| r.state() == BroomBossState::Idle);
        assert_eq!(rig.state(), BroomBossState::Idle);
        assert_eq!(rig.boss().damage_mult, 1.0);
        // Exactly one tantrum per fight.
        for _ in 0..5 * TICKS_PER_SEC {
            rig.tick();
            assert_ne!(rig.state(), BroomBossState::PhaseChange);
        }
    }

    /// Crossing half health *inside* the `Hurt` hop — the rat's own punish
    /// window, the likeliest place to cross it — used to drop the tantrum for
    /// the whole fight: the framework's `phase_changed` is a one-tick pulse and
    /// the hop must not be interrupted. The set piece is owed instead and
    /// starts on the first free tick.
    #[test]
    fn a_threshold_crossed_mid_hop_still_owes_the_tantrum() {
        let mut rig = Rig::new(62);
        rig.set_player(CENTER + Vec2::new(20.0, 0.0));
        rig.edit(|b| {
            b.set_state(
                BossState::Broom(BroomBossState::Hurt { left_foot_hit: true }),
                HURT,
            );
            b.hp = b.max_hp * 0.49;
        });
        rig.tick();
        assert!(
            matches!(rig.state(), BroomBossState::Hurt { .. }),
            "the hop is never interrupted"
        );
        assert_eq!(rig.boss().phase, 2);
        let ticks = rig.run_until(200, |r| !matches!(r.state(), BroomBossState::Hurt { .. }));
        assert!(ticks < 200, "the hop ends");
        // The hop hands over to `Idle` on its last tick; the owed tantrum takes
        // the very next one.
        rig.tick();
        assert_eq!(rig.state(), BroomBossState::PhaseChange);
        // And it is still exactly one tantrum.
        rig.run_until(400, |r| r.state() != BroomBossState::PhaseChange);
        for _ in 0..3 * TICKS_PER_SEC {
            rig.tick();
            assert_ne!(rig.state(), BroomBossState::PhaseChange);
        }
    }

    // ---- fit tests ----------------------------------------------------------

    #[test]
    fn a_walled_off_rat_makes_the_human_walk_instead_of_raking() {
        let mut rig = Rig::new(67);
        // A solid block between the human (cell 100,80) and the rat.
        rig.paint(104, 70, 3, 20);
        rig.set_player(CENTER + Vec2::new(120.0, 0.0));
        // Decision 2 is the opening's rake.
        rig.edit(|b| set_decision(b, 2));
        let start = rig.boss().pos;
        rig.run_until(120, |r| r.state() != BroomBossState::Idle);
        assert_eq!(
            rig.state(),
            BroomBossState::StompWalk { left_foot: true },
            "a rake with no pocket lane must become a stomping walk"
        );
        // …and the walk *is* the reposition: the human closes the distance.
        for _ in 0..TICKS_PER_SEC {
            rig.tick();
        }
        assert!(
            rig.boss().pos.x > start.x + 2.0,
            "the human must reposition toward the rat"
        );
    }

    #[test]
    fn no_beat_starts_without_its_footprint() {
        // A rat behind a wall is never raked or pan-dropped while the pocket
        // lane is blocked: the human walks around instead. (It may of course
        // rake once it has walked into the open.)
        let mut rig = Rig::new(71);
        rig.paint(104, 70, 3, 20);
        rig.set_player(CENTER + Vec2::new(120.0, 0.0));
        let mut stomps = 0;
        let mut last = rig.state();
        for _ in 0..20 * TICKS_PER_SEC {
            rig.tick();
            let now = rig.state();
            if now != last {
                match now {
                    BroomBossState::BroomSweep { .. } | BroomBossState::DustpanDrop => {
                        let rat = rig.player_pos();
                        let anchor = rig.boss().pos;
                        assert!(
                            rig.w
                                .level
                                .tiles
                                .segment_hit(rat, anchor, PLAYER_RADIUS)
                                .is_none(),
                            "started a beat with the pocket lane walled off"
                        );
                    }
                    BroomBossState::StompWalk { .. } => stomps += 1,
                    _ => {}
                }
                last = now;
            }
        }
        assert!(stomps > 0, "the human must walk instead of attacking");
    }

    // ---- the fight ----------------------------------------------------------

    /// The scripted rat: dodge on the telegraph, never teleport.
    fn dodge_dir(boss: &Boss, player: Vec2) -> Vec2 {
        let away = unit_or(player - boss.pos, Vec2::new(1.0, 0.0));
        let target = match boss.state {
            // Home move: shelter under the human, inside the rake's pocket.
            BossState::Broom(BroomBossState::BroomSweep { .. }) => boss.pos,
            // Out of the crush circle and the shockwave ring.
            BossState::Broom(BroomBossState::StompWalk { .. }) => {
                boss.target + away * (art_boss_broom::STOMP_SHOCKWAVE_RADIUS + PLAYER_RADIUS + 3.0)
            }
            // Out of the pan and off its shard lanes.
            BossState::Broom(BroomBossState::DustpanDrop) => {
                boss.target + away * (art_boss_broom::DUSTPAN_IMPACT_RADIUS + PLAYER_RADIUS + 5.0)
            }
            // Out of the tantrum slam.
            BossState::Broom(BroomBossState::PhaseChange) => {
                boss.pos + away * (art_boss_broom::PHASE_SLAM_RADIUS + PLAYER_RADIUS + 6.0)
            }
            _ => return Vec2::ZERO,
        };
        let to = target - player;
        if to.len() > 1.5 {
            to.normalized()
        } else {
            Vec2::ZERO
        }
    }

    #[test]
    fn a_dodging_rat_survives_a_minute_of_the_fight() {
        let mut rig = Rig::new(73);
        for _ in 0..60 * TICKS_PER_SEC {
            let dir = {
                let boss = rig.boss();
                dodge_dir(&boss, rig.player_pos())
            };
            rig.tick_with(dir);
            assert!(rig.has_boss(), "the boss must not die on its own");
        }
        assert!(rig.hp() > 0.0, "the dodging rat died (hp {})", rig.hp());
    }

    #[test]
    fn sustained_damage_kills_the_boss_and_completes_the_level_once() {
        let mut rig = Rig::new(79);
        let mut completions = 0;
        let mut was_complete = false;
        let mut died_at = None;
        for _ in 0..40 * TICKS_PER_SEC {
            let dir = {
                let boss = rig.boss();
                dodge_dir(&boss, rig.player_pos())
            };
            rig.edit(|b| {
                if !b.dead {
                    b.hp -= SCRIPTED_DPS * DT;
                }
            });
            rig.tick_with(dir);
            if rig.w.level_complete && !was_complete {
                completions += 1;
            }
            was_complete = rig.w.level_complete;
            if died_at.is_none() && !rig.has_boss() {
                died_at = Some(true);
            }
            if died_at.is_some() {
                break;
            }
        }
        assert_eq!(completions, 1, "the level completed {completions} times");
        assert!(rig.w.level_complete);
        assert!(rig.w.ecs.resource::<BossFight>().level_complete);
        // The defeat state ran for the art's 2.40 s before the entity went.
        assert!(died_at.is_some(), "the boss never despawned");
        assert!(!rig.has_boss(), "the walk-off ends with the entity gone");
    }

    #[test]
    fn the_defeat_walk_off_is_not_targetable_and_lasts_2_4_s() {
        let mut rig = Rig::new(83);
        rig.edit(|b| b.hp = 0.0);
        rig.tick();
        let boss = rig.boss();
        assert!(boss.dead);
        assert_eq!(boss.state, BossState::Broom(BroomBossState::Defeat));
        assert!(
            boss.hit_shapes().iter().all(|s| !s.grounded()),
            "the abandoning human cannot be shot"
        );
        let ticks = rig.run_until(400, |r| !r.has_boss());
        assert!((ticks as f32 * DT - 2.40).abs() <= 2.0 * DT + 1e-4);
    }

    // ---- determinism --------------------------------------------------------

    /// Compact tag for a state (so a trace is comparable and printable).
    fn tag(state: BroomBossState) -> u8 {
        match state {
            BroomBossState::Idle => 0,
            BroomBossState::StompWalk { left_foot: true } => 1,
            BroomBossState::StompWalk { left_foot: false } => 2,
            BroomBossState::BroomSweep { clockwise: true } => 3,
            BroomBossState::BroomSweep { clockwise: false } => 4,
            BroomBossState::BroomJab => 5,
            BroomBossState::DustpanDrop => 6,
            BroomBossState::Hurt { left_foot_hit: true } => 7,
            BroomBossState::Hurt { left_foot_hit: false } => 8,
            BroomBossState::PhaseChange => 9,
            BroomBossState::Defeat => 10,
        }
    }

    /// One scripted run's trace: the human's state and both hp pools.
    fn trace(seed: u64) -> Vec<(u8, u32, u32, u32, u32, u32, usize)> {
        let mut rig = Rig::new(seed);
        let mut out = Vec::new();
        for _ in 0..20 * TICKS_PER_SEC {
            let dir = {
                let boss = rig.boss();
                dodge_dir(&boss, rig.player_pos())
            };
            rig.tick_with(dir);
            let boss = rig.boss();
            out.push((
                tag(match boss.state {
                    BossState::Broom(s) => s,
                    _ => BroomBossState::Idle,
                }),
                boss.pos.x.to_bits(),
                boss.pos.y.to_bits(),
                boss.hp.to_bits(),
                rig.w.player.hp.to_bits(),
                boss.flags,
                boss.hazards.len(),
            ));
        }
        out
    }

    #[test]
    fn twin_runs_with_the_boss_active_are_identical() {
        for seed in [3, 91, 4242] {
            assert_eq!(trace(seed), trace(seed), "seed {seed} diverged");
        }
    }
}
