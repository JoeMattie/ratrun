//! Sewer boss — the Jungle Cat (Apex Jaguar).
//!
//! The art module [`crate::game::art_boss_cat`] is the gameplay spec: the state
//! enum, every per-state duration, damage number, hit radius and the view
//! struct are binding. This file only drives the transitions and queues the
//! art's telegraphed attacks as shared [`BossHazard`]s.
//!
//! ## The fight the designer asked for
//!
//! *Let the jaguar miss magnificently.* The core loop is
//! **stalk → crouch (landing ring) → pounce → recovery**, and the tail is
//! harmless — damage only comes through the three art circles from
//! [`art_boss_cat::body_hit_circles`]. The landing point tracks the rat until
//! [`art_boss_cat::CROUCH_LOCK_PROGRESS`] (`75 %` of the crouch), then locks, so
//! a rat that reads the ring and leaves walks out of a 22 px circle that is
//! already committed.
//!
//! **The home move.** A pounce only starts when the landing footprint fits and
//! a connected escape lane leads out of the 22 px hit zone (the leap is
//! *shortened*, or the cat stalks again, when it does not). When the rat is not
//! inside the ring at impact — it ducked through a rat-hole, stepped behind a
//! wall, or simply walked — the cat **overshoots**, skids along the leap
//! direction and is punishable for [`MISS_PUNISH`] (0.7 s): it cannot move or
//! attack until the daze ends. The skid uses large-body collision, so a cat
//! that leapt at a rat-hole slides past on its original side.
//!
//! The claw swipe answers a rat that hugs the cat; the roar is the half-health
//! phase change (and only then a ~12 s beat in phase 2). Phase 2 is the art's
//! Apex Frenzy: faster, a landing shockwave ring, sonic fangs and a possible
//! second swipe.
//!
//! ## State machine (art timings)
//!
//! | state | duration | telegraph / active window | damage |
//! |-------|----------|---------------------------|--------|
//! | `Stalk` | 2.0–3.5 s P1 / 1.4–2.4 s P2 | none | body contact 18 |
//! | `Crouch` | 0.95 s P1 / 0.75 s P2 | landing ring 22 px at `target` | body contact |
//! | `Pounce` | 0.70 s P1 / 0.58 s P2 | land at progress 0.80 | 28 crush + 14 shockwave |
//! | `SwipeWindup` | 0.55 s P1 / 0.42 s P2 | full windup, facing locks at 0.55 | body contact |
//! | `Swipe` | 0.50 s | active 0.05..0.45 | 22 |
//! | `Roar` | 1.30 s | blast 0.35..0.90 | 12 inside 26 px (+ 2 × 8 sonic fangs in P2), 0 but 85 px/s push out to 44 px |
//! | `Hurt` | 0.45 s (0.90 s on a trap-sized hit) | — | — |
//! | `Death` | 1.80 s | — | — |
//!
//! ## Per-kind scratch
//!
//! [`BossController`] is zero-sized; the cat keeps its memory in the shared
//! per-controller slots ([`Boss::flags`] / [`Boss::scratch`]):
//!
//! * `flags` — the state bit flags below (one-shots, the daze, the miss).
//! * `scratch[0]` — seconds since the last roar (the phase-2 territory call).
//! * `scratch[1]` — the hp observed last tick, which is how a single-tick burst
//!   (a trap snap, a nuke) is told apart from chip damage.
//!
//! Both are initialised on the first tick ([`F_READY`]) from the boss's own hp.
//! The art view's own fields (`pounce_origin`, `swipe_side`) are the leap arc
//! and the paw side and are therefore *not* scratch.
//!
//! ## Feedback
//!
//! The controller cannot touch `SfxQueue`/`Particles`; it pushes [`BossCue`]s
//! into [`BossOutput`] and the shared system plays the matching sound and
//! vfx after the tick.

use rand::RngExt;

use super::*;

// ============================================================================
// Flags and cat tuning
// ============================================================================

/// `Boss::flags` bit flags (state-local one-shots).
const F_READY: u32 = 1 << 0;
/// The post-miss punish window is running (in `Stalk`).
const F_DAZED: u32 = 1 << 1;
/// The last pounce missed (the rat left the landing ring).
const F_MISS: u32 = 1 << 2;
/// Generic once-per-state latch (slam queued, blast queued, chain used).
const F_LATCH: u32 = 1 << 3;
/// The crouch's landing point is committed.
const F_LOCKED: u32 = 1 << 4;
const F_FANGS_1: u32 = 1 << 5;
const F_FANGS_2: u32 = 1 << 6;
/// A phase-2 swipe already chained into a second windup.
const F_CHAINED: u32 = 1 << 7;
/// The death cue has been queued (the collapse plays it once).
const F_DEATH_CUE: u32 = 1 << 8;

/// `scratch[0]`: seconds since the last roar.
const S_ROAR_CD: usize = 0;
/// `scratch[1]`: the hp observed last tick (burst detection).
const S_LAST_HP: usize = 1;

/// Stalk beat bounds (art: `2.0–3.5 s` P1, `1.4–2.4 s` P2).
const STALK_BEAT_P1: (f32, f32) = (2.0, 3.5);
const STALK_BEAT_P2: (f32, f32) = (1.4, 2.4);

/// Readable minimum at the start of a beat before the cat may attack.
const STALK_REACT: f32 = 0.30;
/// Art: `Stalk` ends early and swipes when the rat stays within 40 px.
const STALK_NEAR: f32 = 40.0;
/// Art: `Stalk` ends early and crouches when the rat is 45..95 px away.
const STALK_POUNCE_LO: f32 = 45.0;
const STALK_POUNCE_HI: f32 = 95.0;

/// How far the landing point may be pulled back toward the cat to find a
/// landing that fits (designer: "shorten the leap").
const POUNCE_PULL_LADDER: [f32; 6] = [0.0, 8.0, 14.0, 20.0, 26.0, 32.0];
/// Clear floor wanted beyond the landing ring for the rat's escape lane.
const POUNCE_ESCAPE_CLEARANCE: f32 = 6.0;
/// Art: the slam stays dangerous until `progress = 0.95`.
const POUNCE_SLAM_END: f32 = 0.95;
const POUNCE_KNOCKBACK: f32 = 140.0;

/// The designer's punish window after a missed pounce.
const MISS_PUNISH: f32 = 0.70;
/// How far past the locked landing point the cat skids when it misses.
const POUNCE_SKID: f32 = 26.0;

/// Art: `SwipeWindup` locks the facing at this progress.
const SWIPE_FACING_LOCK: f32 = 0.55;
const SWIPE_KNOCKBACK: f32 = 130.0;
/// Phase 2 may chain one more swipe (art: "may chain a second `SwipeWindup`").
const SWIPE_CHAIN_CHANCE_P2: f32 = 0.45;

/// Phase 2 periodic roar cooldown (art: "periodically thereafter (~12 s)").
const ROAR_COOLDOWN: f32 = 12.0;
/// The roar only zones if the rat is anywhere near it.
const ROAR_ENGAGE: f32 = 90.0;
const ROAR_KNOCKBACK: f32 = 180.0;
/// The damage-free outer roar ring's sustained push, in px/s (the art's
/// `ROAR_INNER_RADIUS..ROAR_OUTER_RADIUS` band).
const ROAR_PUSH: f32 = 85.0;
/// Phase 2 sonic fang pulses, at these `Roar::progress` values (art).
const FANG_PULSES: [f32; 2] = [0.42, 0.68];
const FANG_COUNT: usize = 8;
const FANG_SPEED: f32 = 65.0;
const FANG_SPAWN_OFFSET: f32 = 4.0;

/// One tick of damage at (or above) this staggers the cat; double it is the
/// art's extended "trap stun" (`0.90 s`). Absolute, not a fraction of the
/// director-scaled pool, so a mousetrap snap (`80`) always reads.
const HURT_BURST: f32 = 55.0;
const HURT_STUN_BURST: f32 = 100.0;

/// Eight compass directions, in a fixed order, for the escape-lane test.
const LANE_DIRS: [Vec2; 8] = [
    Vec2 { x: 1.0, y: 0.0 },
    Vec2 { x: -1.0, y: 0.0 },
    Vec2 { x: 0.0, y: 1.0 },
    Vec2 { x: 0.0, y: -1.0 },
    Vec2 {
        x: std::f32::consts::FRAC_1_SQRT_2,
        y: std::f32::consts::FRAC_1_SQRT_2,
    },
    Vec2 {
        x: -std::f32::consts::FRAC_1_SQRT_2,
        y: std::f32::consts::FRAC_1_SQRT_2,
    },
    Vec2 {
        x: std::f32::consts::FRAC_1_SQRT_2,
        y: -std::f32::consts::FRAC_1_SQRT_2,
    },
    Vec2 {
        x: -std::f32::consts::FRAC_1_SQRT_2,
        y: -std::f32::consts::FRAC_1_SQRT_2,
    },
];

// ============================================================================
// Small helpers
// ============================================================================

#[inline]
fn flagged(boss: &Boss, flag: u32) -> bool {
    boss.flags & flag != 0
}

#[inline]
fn set_flag(boss: &mut Boss, flag: u32, on: bool) {
    if on {
        boss.flags |= flag;
    } else {
        boss.flags &= !flag;
    }
}

/// Distance from the landing point the rat must be able to reach to be safe:
/// the ring radius plus its own body plus a little floor.
#[inline]
fn pounce_escape_dist() -> f32 {
    art_boss_cat::POUNCE_LAND_RADIUS + PLAYER_RADIUS + POUNCE_ESCAPE_CLEARANCE
}

/// Is any straight escape lane of `dist` clear from `from` for the rat?
///
/// The "connected escape lane beyond the hit zone" test: the player must be
/// able to leave the forced-movement area without a wall in the way.
#[inline]
fn escape_lane(ctx: &BossContext<'_>, from: Vec2, dist: f32) -> bool {
    LANE_DIRS
        .iter()
        .any(|d| ctx.los_clear(from, from + *d * dist, PLAYER_RADIUS))
}

/// A pounce candidate: the art's `34..96 px` band, pulled toward the cat until
/// the landing footprint and the rat's escape lane both fit.
fn pounce_landing(ctx: &BossContext<'_>, boss: &Boss) -> Option<Vec2> {
    let to = ctx.player_pos - boss.pos;
    let distance = to.len();
    if distance <= 1.0 {
        return None;
    }
    let dir = to.normalized();
    for &pull in POUNCE_PULL_LADDER.iter() {
        // Never land past the rat (that would pounce *onto* the player) and
        // never outside the art's band.
        let leap = (distance - pull)
            .clamp(art_boss_cat::POUNCE_MIN_DIST, art_boss_cat::POUNCE_MAX_DIST)
            .min(distance);
        let landing = boss.pos + dir * leap;
        if !ctx.footprint_fits(landing, art_boss_cat::BODY_RADIUS) {
            continue;
        }
        if !escape_lane(ctx, landing, pounce_escape_dist()) {
            continue;
        }
        return Some(landing);
    }
    None
}

/// The swipe is a hug answer: it needs the rat in sight (a cat cannot claw
/// through a wall) and room to get out of the 30 px sector.
fn swipe_fits(ctx: &BossContext<'_>, boss: &Boss) -> bool {
    ctx.footprint_fits(boss.pos, art_boss_cat::BODY_RADIUS)
        && ctx.los_clear(boss.pos, ctx.player_pos, PLAYER_RADIUS)
        && escape_lane(
            ctx,
            ctx.player_pos,
            art_boss_cat::SWIPE_RADIUS + PLAYER_RADIUS + 4.0,
        )
}

/// The roar needs an escape lane out past its 44 px outer ring.
fn roar_fits(ctx: &BossContext<'_>, boss: &Boss) -> bool {
    ctx.footprint_fits(boss.pos, art_boss_cat::BODY_RADIUS)
        && escape_lane(
            ctx,
            ctx.player_pos,
            art_boss_cat::ROAR_OUTER_RADIUS + PLAYER_RADIUS + 4.0,
        )
}

/// Enter a fresh stalking beat (clears every state flag).
fn enter_stalk(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let (lo, hi) = if boss.phase >= 2 {
        STALK_BEAT_P2
    } else {
        STALK_BEAT_P1
    };
    let beat = lo + (hi - lo) * ctx.rng.random::<f32>();
    boss.flags = F_READY;
    boss.set_state(BossState::Cat(BossCatState::Stalk), beat);
}

/// Commit to a pounce, or refuse when no landing fits.
fn enter_crouch(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) -> bool {
    let Some(landing) = pounce_landing(ctx, boss) else {
        return false;
    };
    boss.target = landing;
    boss.pounce_origin = boss.pos;
    if landing.dist_sq(boss.pos) > 1e-6 {
        boss.facing = (landing - boss.pos).normalized();
    }
    boss.flags = F_READY;
    out.cue(BossCue::CatCrouch);
    let duration = if boss.phase >= 2 {
        art_boss_cat::CROUCH_DURATION_P2
    } else {
        art_boss_cat::CROUCH_DURATION_P1
    };
    boss.set_state(BossState::Cat(BossCatState::Crouch), duration);
    true
}

/// Raise the paw. `swipe_side` alternates so the player reads the wind-up.
fn enter_swipe_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    boss.swipe_side = if boss.swipe_side >= 0.0 { -1.0 } else { 1.0 };
    boss.flags &= !(F_DAZED | F_MISS | F_LATCH | F_LOCKED | F_FANGS_1 | F_FANGS_2);
    let duration = if boss.phase >= 2 {
        art_boss_cat::SWIPE_WINDUP_P2
    } else {
        art_boss_cat::SWIPE_WINDUP_P1
    };
    boss.set_state(BossState::Cat(BossCatState::SwipeWindup), duration);
}

/// The half-health set piece: plant, face the rat, queue the acoustic blast.
fn enter_roar(ctx: &BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    boss.flags = F_READY;
    boss.scratch[S_ROAR_CD] = 0.0; // roar cooldown restarts
    out.cue(BossCue::CatRoar);
    boss.set_state(
        BossState::Cat(BossCatState::Roar),
        art_boss_cat::ROAR_DURATION,
    );
    let origin = art_boss_cat::roar_origin(&boss.cat_view());
    let duration = art_boss_cat::ROAR_DURATION;
    let warn = art_boss_cat::ROAR_BLAST_START * duration;
    let active = (art_boss_cat::ROAR_BLAST_END - art_boss_cat::ROAR_BLAST_START) * duration;
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Circle {
                center: origin,
                radius: art_boss_cat::ROAR_INNER_RADIUS,
            },
            warn,
            active,
            art_boss_cat::ROAR_DAMAGE,
        )
        .with_knockback(ROAR_KNOCKBACK)
        .with_enemies(),
    );
    // The art's outer ring (`ROAR_INNER_RADIUS..ROAR_OUTER_RADIUS`) is a
    // damage-free shove: the blast hammers the inner 26 px, and everything out
    // to 44 px is pushed out of the cat's territory instead of hurt. A
    // `damage = 0` hazard applies its knockback as a sustained px/s push (see
    // `resolve_hazards`), so this is the ring the art draws and nothing else.
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Annulus {
                center: origin,
                inner: art_boss_cat::ROAR_INNER_RADIUS,
                outer: art_boss_cat::ROAR_OUTER_RADIUS,
            },
            warn,
            active,
            0.0,
        )
        .with_knockback(ROAR_PUSH),
    );
}

/// The impact: crush circle (and the phase-2 shockwave ring), both telegraphed
/// by the crouch's landing ring, so their own warn window is zero.
fn push_landing_hazards(boss: &mut Boss, phase2: bool) {
    let duration = boss.duration;
    let land = art_boss_cat::POUNCE_LAND_PROGRESS;
    let slam_active = (POUNCE_SLAM_END - land) * duration;
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Circle {
                center: boss.target,
                radius: art_boss_cat::POUNCE_LAND_RADIUS,
            },
            0.0,
            slam_active.max(0.05),
            art_boss_cat::POUNCE_DAMAGE,
        )
        .with_knockback(POUNCE_KNOCKBACK)
        .with_enemies(),
    );
    if phase2 {
        let wave_active = (1.0 - land) * duration;
        let _ = boss.push_hazard(
            BossHazard::new(
                HazardShape::Annulus {
                    center: boss.target,
                    inner: art_boss_cat::POUNCE_LAND_RADIUS,
                    outer: art_boss_cat::POUNCE_SHOCKWAVE_RADIUS,
                },
                0.0,
                wave_active.max(0.05),
                art_boss_cat::POUNCE_SHOCKWAVE_DAMAGE,
            )
            .with_knockback(0.6 * POUNCE_KNOCKBACK)
            .with_enemies(),
        );
    }
}

/// One radial fan of the phase-2 sonic fangs.
fn spawn_sonic_fangs(ctx: &mut BossContext<'_>, boss: &Boss, out: &mut BossOutput) {
    let origin = art_boss_cat::roar_origin(&boss.cat_view());
    // One rng draw per fan (deterministic; the two fans differ) so the gaps
    // do not line up with the rat's approach.
    let twist = ctx.rng.random::<f32>() * std::f32::consts::TAU / FANG_COUNT as f32;
    for i in 0..FANG_COUNT {
        let angle = twist + i as f32 * std::f32::consts::TAU / FANG_COUNT as f32;
        let dir = Vec2::from_angle(angle);
        out.projectiles.push(BossProjectileSpawn::new(
            BossProjectileKind::SonicFang,
            origin + dir * FANG_SPAWN_OFFSET,
            dir * FANG_SPEED,
        ));
    }
}

/// Pick and start the next attack. Only moves whose footprint *and* the rat's
/// escape lane fit where the cat stands are allowed; otherwise it stalks on.
fn start_attack(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) -> bool {
    let phase2 = boss.phase >= 2;
    let to = ctx.player_pos - boss.pos;
    let distance = to.len();

    // 1. A rat hugging the cat gets the claw.
    if distance <= STALK_NEAR && swipe_fits(ctx, boss) {
        enter_swipe_windup(ctx, boss);
        return true;
    }
    // 2. The signature pounce, from the art's band with a clear line.
    if (STALK_POUNCE_LO..=STALK_POUNCE_HI).contains(&distance)
        && ctx.los_clear(boss.pos, ctx.player_pos, PLAYER_RADIUS)
        && enter_crouch(ctx, boss, out)
    {
        return true;
    }
    // 3. Phase 2 only: the periodic territorial roar (phase 1 saves it for the
    //    half-health set piece).
    if phase2
        && boss.scratch[S_ROAR_CD] >= ROAR_COOLDOWN
        && distance <= ROAR_ENGAGE
        && roar_fits(ctx, boss)
    {
        enter_roar(ctx, boss, out);
        return true;
    }
    false
}

// ============================================================================
// Controller
// ============================================================================

/// The jaguar controller (zero-sized; the state lives on the [`Boss`]).
pub struct CatController;

impl BossController for CatController {
    fn kind(&self) -> BossKind {
        BossKind::Cat
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.dead {
            return;
        }
        let BossState::Cat(state) = boss.state else {
            return;
        };
        if state == BossCatState::Death {
            return;
        }

        // ---- per-spawn memory (see the module docs) ------------------------
        if !flagged(boss, F_READY) {
            boss.flags = F_READY;
            boss.scratch[S_ROAR_CD] = 0.0;
            boss.scratch[S_LAST_HP] = boss.hp;
        }
        // Single-tick burst (a trap snap, a nuke) versus chip damage.
        let burst = (boss.scratch[S_LAST_HP] - boss.hp).max(0.0);
        boss.scratch[S_LAST_HP] = boss.hp;
        boss.scratch[S_ROAR_CD] = (boss.scratch[S_ROAR_CD] + ctx.dt).min(ROAR_COOLDOWN + 1.0);
        let phase2 = boss.phase >= 2;

        // ---- crossing half health is the roar set piece (it outranks a
        //      stagger: the phase change must always read) ------------------
        if out.phase_changed && !matches!(state, BossCatState::Hurt | BossCatState::Roar) {
            enter_roar(ctx, boss, out);
            return;
        }

        // ---- heavy burst damage staggers; chip hits only flash ------------
        let airborne = state == BossCatState::Pounce
            && boss.progress() < art_boss_cat::POUNCE_LAND_PROGRESS;
        if burst >= HURT_BURST
            && !airborne
            && !matches!(state, BossCatState::Hurt | BossCatState::Roar)
        {
            let duration = if burst >= HURT_STUN_BURST {
                2.0 * art_boss_cat::HURT_DURATION
            } else {
                art_boss_cat::HURT_DURATION
            };
            boss.flags = F_READY;
            boss.target = ctx.player_pos;
            boss.set_state(BossState::Cat(BossCatState::Hurt), duration);
            out.cue(BossCue::CatHurt);
            return;
        }

        match state {
            // ---------------------------------------------------------------
            // Stalk: prowl, then choose an attack the room allows.
            // ---------------------------------------------------------------
            BossCatState::Stalk => {
                boss.target = ctx.player_pos;

                // The post-miss punish window: skid and do nothing else.
                if flagged(boss, F_DAZED) {
                    let left = 1.0 - boss.progress();
                    let speed = 2.0 * POUNCE_SKID * left / MISS_PUNISH;
                    if speed > 2.0 {
                        move_boss(ctx, boss, boss.facing, speed);
                    }
                    if boss.progress() >= 1.0 {
                        enter_stalk(ctx, boss);
                    }
                    return;
                }

                let speed = if phase2 {
                    art_boss_cat::STALK_SPEED_P2
                } else {
                    art_boss_cat::STALK_SPEED_P1
                };
                let seek = ctx.seek(boss);
                if move_boss(ctx, boss, seek, speed) && seek.len_sq() > 1e-6 {
                    boss.facing = seek.normalized();
                }
                // Eyes on the rat whenever the cat can see it (art: slit eyes
                // track `target` even while the flow field routes the body).
                let to = ctx.player_pos - boss.pos;
                if to.len_sq() > 1e-6 && ctx.los_clear(boss.pos, ctx.player_pos, PLAYER_RADIUS) {
                    boss.facing = to.normalized();
                }

                if boss.elapsed >= STALK_REACT && start_attack(ctx, boss, out) {
                    return;
                }
                if boss.progress() >= 1.0 {
                    enter_stalk(ctx, boss);
                }
            }

            // ---------------------------------------------------------------
            // Crouch: the landing ring tracks the rat, then locks at 75 %.
            // ---------------------------------------------------------------
            BossCatState::Crouch => {
                if !flagged(boss, F_LOCKED) {
                    let Some(landing) = pounce_landing(ctx, boss) else {
                        // No landing fits any more: stalk again instead of
                        // committing a leap the room cannot hold.
                        enter_stalk(ctx, boss);
                        return;
                    };
                    boss.target = landing;
                    boss.pounce_origin = boss.pos;
                    if landing.dist_sq(boss.pos) > 1e-6 {
                        boss.facing = (landing - boss.pos).normalized();
                    }
                    if boss.progress() >= art_boss_cat::CROUCH_LOCK_PROGRESS {
                        set_flag(boss, F_LOCKED, true);
                    }
                }
                if boss.progress() >= 1.0 {
                    boss.flags &= !(F_LOCKED | F_LATCH | F_MISS);
                    let duration = if phase2 {
                        art_boss_cat::POUNCE_DURATION_P2
                    } else {
                        art_boss_cat::POUNCE_DURATION_P1
                    };
                    boss.set_state(BossState::Cat(BossCatState::Pounce), duration);
                    out.cue(BossCue::CatPounce);
                }
            }

            // ---------------------------------------------------------------
            // Pounce: airborne to the locked point, slam, then a skid.
            // ---------------------------------------------------------------
            BossCatState::Pounce => {
                let land = art_boss_cat::POUNCE_LAND_PROGRESS;
                let progress = boss.progress();
                let origin = boss.pounce_origin;
                let target = boss.target;
                let travel = target - origin;
                if progress < land {
                    // In flight: the leap follows the committed line (the art
                    // draws the arc height from the same origin/target pair).
                    boss.pos = origin + travel * (progress / land).clamp(0.0, 1.0);
                    if travel.len_sq() > 1e-6 {
                        boss.facing = travel.normalized();
                    }
                } else {
                    boss.pos = target;
                    if !flagged(boss, F_LATCH) {
                        set_flag(boss, F_LATCH, true);
                        push_landing_hazards(boss, phase2);
                        out.cue(BossCue::CatLand);
                        // The home move: the rat left the ring in time.
                        let caught = target.dist(ctx.player_pos)
                            <= art_boss_cat::POUNCE_LAND_RADIUS + PLAYER_RADIUS;
                        set_flag(boss, F_MISS, !caught);
                    }
                }
                if progress >= 1.0 {
                    if flagged(boss, F_MISS) {
                        // Overshoot, then the 0.7 s punish window in `Stalk`.
                        boss.flags = F_READY | F_DAZED;
                        boss.set_state(BossState::Cat(BossCatState::Stalk), MISS_PUNISH);
                        out.cue(BossCue::CatSkid);
                    } else {
                        enter_stalk(ctx, boss);
                    }
                }
            }

            // ---------------------------------------------------------------
            // SwipeWindup: the raised paw; facing locks at `progress = 0.55`.
            // ---------------------------------------------------------------
            BossCatState::SwipeWindup => {
                boss.target = ctx.player_pos;
                if boss.progress() < SWIPE_FACING_LOCK {
                    let to = ctx.player_pos - boss.pos;
                    if to.len_sq() > 1e-6 {
                        boss.facing = to.normalized();
                    }
                }
                if boss.progress() >= 1.0 {
                    boss.flags &= !(F_DAZED | F_MISS | F_LATCH | F_LOCKED);
                    boss.set_state(
                        BossState::Cat(BossCatState::Swipe),
                        art_boss_cat::SWIPE_DURATION,
                    );
                    out.cue(BossCue::CatSwipe);
                    let duration = art_boss_cat::SWIPE_DURATION;
                    let origin = art_boss_cat::swipe_origin(&boss.cat_view());
                    let warn = art_boss_cat::SWIPE_ACTIVE_START * duration;
                    let active =
                        (art_boss_cat::SWIPE_ACTIVE_END - art_boss_cat::SWIPE_ACTIVE_START) * duration;
                    let _ = boss.push_hazard(
                        BossHazard::new(
                            HazardShape::Arc {
                                center: origin,
                                inner: art_boss_cat::SWIPE_INNER_RADIUS,
                                outer: art_boss_cat::SWIPE_RADIUS,
                                facing: boss.facing,
                                half_angle: art_boss_cat::SWIPE_HALF_ANGLE,
                            },
                            warn,
                            active,
                            art_boss_cat::SWIPE_DAMAGE,
                        )
                        .with_knockback(SWIPE_KNOCKBACK)
                        .with_enemies(),
                    );
                }
            }

            // ---------------------------------------------------------------
            // Swipe: the arc is queued; phase 2 may chain the other paw once.
            // ---------------------------------------------------------------
            BossCatState::Swipe => {
                boss.target = ctx.player_pos;
                if boss.progress() >= 1.0 {
                    let chain = phase2
                        && !flagged(boss, F_CHAINED)
                        && ctx.rng.random::<f32>() < SWIPE_CHAIN_CHANCE_P2
                        && swipe_fits(ctx, boss);
                    if chain {
                        set_flag(boss, F_CHAINED, true);
                        enter_swipe_windup(ctx, boss);
                    } else {
                        enter_stalk(ctx, boss);
                    }
                }
            }

            // ---------------------------------------------------------------
            // Roar: inhale, blast, settle; phase 2 adds the sonic fang fans.
            // ---------------------------------------------------------------
            BossCatState::Roar => {
                boss.target = ctx.player_pos;
                if phase2 {
                    for (i, &at) in FANG_PULSES.iter().enumerate() {
                        let fan = if i == 0 { F_FANGS_1 } else { F_FANGS_2 };
                        if !flagged(boss, fan) && boss.progress() >= at {
                            set_flag(boss, fan, true);
                            spawn_sonic_fangs(ctx, boss, out);
                            out.cue(BossCue::CatFangs);
                        }
                    }
                }
                if boss.progress() >= 1.0 {
                    enter_stalk(ctx, boss);
                }
            }

            // ---------------------------------------------------------------
            // Hurt: the art's stagger; a free punish window for the rat.
            // ---------------------------------------------------------------
            BossCatState::Hurt => {
                boss.target = ctx.player_pos;
                if boss.progress() >= 1.0 {
                    enter_stalk(ctx, boss);
                }
            }

            BossCatState::Death => {}
        }
    }

    /// The collapse: the art's death burst plays once, on the first tick of the
    /// `Death` state the framework entered for us. Nothing else happens — the
    /// level completes when this beat ends.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH_CUE == 0 {
            boss.flags |= F_DEATH_CUE;
            out.cue(BossCue::CatDeath);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallAxis;
    use crate::game::collision::{SizeClass, TileKind};
    use crate::game::art_walls::WallKind;
    use crate::game::ecs::systems::player::player_system;
    use crate::game::level::{Level, Theme};
    use crate::game::nav::{FlowField, NavBudget};
    use crate::game::player::Player;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;
    /// Sustained weapon damage for the "is the fight beatable" script.
    const SCRIPTED_DPS: f32 = 120.0;

    /// A controlled sewer: an empty tile grid, one cat, and the boss system.
    ///
    /// The generated map belongs to another worker's subsystem, so the cat's
    /// tests paint exactly the walls they need. Every tick round-trips the
    /// resources with `begin_tick`/`end_tick` and runs the real
    /// `boss_update_system`; the player's own system runs too so its i-frames
    /// and movement behave (the scripted rat *is* an input vector).
    struct Cat {
        w: World,
    }

    impl Cat {
        fn new(seed: u64) -> Self {
            let mut w = World::new(Theme::Sewer, seed);
            w.level = Level::empty(Theme::Sewer);
            w.flow = FlowField::new(&w.level.tiles);
            w.player.pos = Vec2::new(800.0, 800.0);
            let arena = TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows());
            let mut boss = Boss::new(
                BossKind::Cat,
                Vec2::new(740.0, 800.0),
                art_boss_cat::BOSS_BASE_HP,
                arena,
                Vec2::ZERO,
                false,
            );
            boss.id = 1;
            boss.facing = Vec2::new(1.0, 0.0);
            w.ecs.spawn((boss, SpawnId(0)));
            w.ecs.resource_mut::<BossFight>().rng = BossRng::for_run(seed, BossKind::Cat);
            Self { w }
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
            self.boss_opt().expect("the cat is in the world")
        }

        /// The cat, or `None` once its death beat has played out and the
        /// framework despawned it.
        fn boss_opt(&mut self) -> Option<Boss> {
            let mut query = self.w.ecs.query::<&Boss>();
            query.iter(&self.w.ecs).next().cloned()
        }

        fn edit<R>(&mut self, f: impl FnOnce(&mut Boss) -> R) -> R {
            let mut query = self.w.ecs.query::<&mut Boss>();
            let mut iter = query.iter_mut(&mut self.w.ecs);
            let mut boss = iter.next().expect("the cat is in the world");
            f(&mut boss)
        }

        fn set_player(&mut self, pos: Vec2) {
            self.w.player.pos = pos;
            self.w.player.vel = Vec2::ZERO;
        }

        fn player(&self) -> &Player {
            &self.w.player
        }

        fn cat_state(&mut self) -> BossCatState {
            match self.boss().state {
                BossState::Cat(s) => s,
                _ => unreachable!("the cat is a cat"),
            }
        }

        /// Run until the cat enters `want`, sampling after every tick.
        fn run_until(&mut self, ticks: usize, want: BossCatState) -> Option<Boss> {
            for _ in 0..ticks {
                self.tick();
                let boss = self.boss();
                if boss.state == BossState::Cat(want) {
                    return Some(boss);
                }
                if boss.dead {
                    return None;
                }
            }
            None
        }

        /// Ticks that force the rat's i-frames open, counting damage events.
        fn count_hits(&mut self, ticks: usize) -> usize {
            let mut events = 0;
            let mut prev = self.w.player.hp;
            for _ in 0..ticks {
                self.w.player.iframes = 0.0;
                self.tick();
                let hp = self.w.player.hp;
                if hp < prev {
                    events += 1;
                }
                prev = hp;
            }
            events
        }

        fn paint_wall(&mut self, cx: i32, y0: i32, y1: i32) {
            for cy in y0..=y1 {
                self.w
                    .level
                    .tiles
                    .set(cx, cy, TileKind::Wall(WallKind::Solid));
            }
            self.w.level.tiles.rebuild_masks();
        }

        fn paint_rat_hole(&mut self, cx: i32, cy: i32) {
            self.w.level.tiles.set(
                cx,
                cy,
                TileKind::Wall(WallKind::RatHole {
                    axis: WallAxis::Vertical,
                }),
            );
            self.w.level.tiles.rebuild_masks();
        }

        fn refresh_flow(&mut self) {
            let goal = self.w.player.pos;
            self.w.flow.update(&self.w.level.tiles, goal, NavBudget::FULL);
        }
    }

    /// Cell centre in world pixels.
    fn cell_center(cx: i32, cy: i32) -> Vec2 {
        Vec2::new(cx as f32 * 16.0 + 8.0, cy as f32 * 16.0 + 8.0)
    }

    // ---- state machine ---------------------------------------------------

    #[test]
    fn every_state_is_reachable_with_the_arts_durations() {
        // Spawn state: stalk, with the framework's opening beat.
        let mut c = Cat::new(3);
        let spawn = c.boss();
        assert_eq!(spawn.state, BossState::Cat(BossCatState::Stalk));
        assert_eq!(spawn.duration, BossKind::Cat.initial_duration());

        // Crouch + pounce from the art's pounce band.
        c.set_player(Vec2::new(870.0, 800.0));
        let crouch = c
            .run_until(400, BossCatState::Crouch)
            .expect("the cat crouches at 70 px");
        assert_eq!(crouch.duration, art_boss_cat::CROUCH_DURATION_P1);
        assert!(crouch.target.dist(crouch.pos) <= art_boss_cat::POUNCE_MAX_DIST);
        assert!(crouch.target.dist(crouch.pos) >= art_boss_cat::POUNCE_MIN_DIST);
        let pounce = c
            .run_until(120, BossCatState::Pounce)
            .expect("crouch leads to the pounce");
        assert_eq!(pounce.duration, art_boss_cat::POUNCE_DURATION_P1);
        assert!(
            c.run_until(120, BossCatState::Stalk).is_some(),
            "the pounce returns to stalk"
        );

        // Swipe windup + swipe when the rat hugs the cat.
        let mut c = Cat::new(4);
        c.set_player(Vec2::new(833.0, 800.0));
        let windup = c
            .run_until(400, BossCatState::SwipeWindup)
            .expect("the cat swipes at 33 px");
        assert_eq!(windup.duration, art_boss_cat::SWIPE_WINDUP_P1);
        let swipe = c
            .run_until(120, BossCatState::Swipe)
            .expect("windup leads to the swipe");
        assert_eq!(swipe.duration, art_boss_cat::SWIPE_DURATION);

        // The roar is the half-health set piece, once.
        let mut c = Cat::new(5);
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        let roar = c.boss();
        assert_eq!(roar.phase, 2);
        assert_eq!(roar.state, BossState::Cat(BossCatState::Roar));
        assert_eq!(roar.duration, art_boss_cat::ROAR_DURATION);
        assert!(
            c.run_until(200, BossCatState::Stalk).is_some(),
            "the roar settles back into stalk"
        );

        // A trap-sized hit staggers; the extended stun lasts 0.90 s.
        let mut c = Cat::new(6);
        c.tick();
        c.edit(|boss| boss.hp -= 60.0);
        c.tick();
        let hurt = c.boss();
        assert_eq!(hurt.state, BossState::Cat(BossCatState::Hurt));
        assert_eq!(hurt.duration, art_boss_cat::HURT_DURATION);

        let mut c = Cat::new(7);
        c.tick();
        c.edit(|boss| boss.hp -= 120.0);
        c.tick();
        assert_eq!(c.boss().duration, 2.0 * art_boss_cat::HURT_DURATION);

        // Death is the art's collapse.
        let mut c = Cat::new(8);
        c.edit(|boss| boss.hp = 0.0);
        c.tick();
        let death = c.boss();
        assert!(death.dead);
        assert_eq!(death.state, BossState::Cat(BossCatState::Death));
        assert_eq!(death.duration, art_boss_cat::DEATH_DURATION);
    }

    #[test]
    fn phase_two_uses_the_faster_art_timings() {
        let mut c = Cat::new(11);
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        assert!(c.run_until(200, BossCatState::Stalk).is_some());

        // Crouch / pounce are the phase-2 durations.
        let boss_pos = c.boss().pos;
        c.set_player(boss_pos + Vec2::new(70.0, 0.0));
        let crouch = c
            .run_until(400, BossCatState::Crouch)
            .expect("phase 2 crouch");
        assert_eq!(crouch.duration, art_boss_cat::CROUCH_DURATION_P2);
        let pounce = c
            .run_until(120, BossCatState::Pounce)
            .expect("phase 2 pounce");
        assert_eq!(pounce.duration, art_boss_cat::POUNCE_DURATION_P2);

        // ... and the swipe windup, at hugging range.
        let mut c = Cat::new(12);
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        assert!(c.run_until(200, BossCatState::Stalk).is_some());
        let boss_pos = c.boss().pos;
        c.set_player(boss_pos + Vec2::new(33.0, 0.0));
        let windup = c
            .run_until(400, BossCatState::SwipeWindup)
            .expect("phase 2 swipe windup");
        assert_eq!(windup.duration, art_boss_cat::SWIPE_WINDUP_P2);
    }

    #[test]
    fn stalk_beats_stay_inside_the_arts_ranges() {
        // Let the rat sit out of every attack's reach and watch the beats.
        let mut c = Cat::new(13);
        c.set_player(Vec2::new(1200.0, 800.0));
        let mut beats = Vec::new();
        for _ in 0..1600 {
            c.tick();
            let boss = c.boss();
            if boss.state == BossState::Cat(BossCatState::Stalk) && boss.elapsed < 2.0 * DT {
                beats.push(boss.duration);
            }
        }
        assert!(!beats.is_empty(), "the cat must keep stalking");
        for beat in beats {
            assert!(
                (STALK_BEAT_P1.0..=STALK_BEAT_P1.1).contains(&beat),
                "phase-1 stalk beat {beat} outside the art's 2.0..3.5 s"
            );
        }
    }

    // ---- hazards ---------------------------------------------------------

    #[test]
    fn pounce_slam_uses_the_art_windows_and_hits_once() {
        let mut c = Cat::new(21);
        c.set_player(Vec2::new(870.0, 800.0));
        let crouch = c.run_until(400, BossCatState::Crouch).expect("crouch");
        let landing = crouch.target;

        // Fly the pounce, watching the impact tick (the slam damages the rat
        // in the same tick it is queued).
        c.run_until(120, BossCatState::Pounce).expect("pounce");
        let hp_before = c.player().hp;
        let armor = c.player().loadout.stats.armor;
        let slam_damage = (art_boss_cat::POUNCE_DAMAGE - armor).max(1.0);
        // Count only the slam's own hits: the rat standing in the ring also
        // takes body contact while the cat sits on it.
        let mut events = 0;
        let mut prev = hp_before;
        let mut slam = None;
        for _ in 0..80 {
            c.w.player.iframes = 0.0;
            c.tick();
            let boss = c.boss();
            if slam.is_none() && boss.progress() >= art_boss_cat::POUNCE_LAND_PROGRESS {
                slam = boss
                    .hazards
                    .iter()
                    .find(|h| h.damage == art_boss_cat::POUNCE_DAMAGE)
                    .cloned();
                assert!(boss.pos.dist(landing) < 0.01, "the cat lands on target");
                assert!(
                    boss.hazards
                        .iter()
                        .all(|h| h.damage != art_boss_cat::POUNCE_SHOCKWAVE_DAMAGE),
                    "phase 1 has no shockwave"
                );
            }
            let hp = c.player().hp;
            if (prev - hp - slam_damage).abs() < 1e-3 {
                events += 1;
            }
            prev = hp;
            if boss.state != BossState::Cat(BossCatState::Pounce) {
                break;
            }
        }
        let slam = slam.expect("the slam hazard is queued at the impact");
        assert_eq!(slam.warn, 0.0, "the crouch ring is the warning");
        let expected = (POUNCE_SLAM_END - art_boss_cat::POUNCE_LAND_PROGRESS)
            * art_boss_cat::POUNCE_DURATION_P1;
        assert!((slam.active - expected).abs() < 1e-4);

        // The rat never leaves the ring: exactly one hit, of the art's damage.
        assert_eq!(events, 1, "the slam must deal damage once");
    }

    #[test]
    fn phase_two_pounce_adds_the_shockwave_ring() {
        let mut c = Cat::new(22);
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        assert!(c.run_until(200, BossCatState::Stalk).is_some());
        let boss_pos = c.boss().pos;
        c.set_player(boss_pos + Vec2::new(70.0, 0.0));
        c.run_until(400, BossCatState::Crouch)
            .expect("phase-2 crouch");
        let mut landed = None;
        for _ in 0..120 {
            c.tick();
            let boss = c.boss();
            if boss.state == BossState::Cat(BossCatState::Pounce)
                && boss.progress() >= art_boss_cat::POUNCE_LAND_PROGRESS
            {
                landed = Some(boss);
                break;
            }
        }
        let boss = landed.expect("the phase-2 pounce lands");
        let ring = boss
            .hazards
            .iter()
            .find(|h| h.damage == art_boss_cat::POUNCE_SHOCKWAVE_DAMAGE)
            .expect("the phase-2 shockwave is queued");
        match ring.shape {
            HazardShape::Annulus { inner, outer, .. } => {
                assert!((inner - art_boss_cat::POUNCE_LAND_RADIUS).abs() < 1e-4);
                assert!((outer - art_boss_cat::POUNCE_SHOCKWAVE_RADIUS).abs() < 1e-4);
            }
            _ => panic!("the shockwave is an annulus"),
        }
    }

    #[test]
    fn swipe_arc_uses_the_art_warning_active_and_damage() {
        let mut c = Cat::new(23);
        c.set_player(Vec2::new(833.0, 800.0));
        c.run_until(400, BossCatState::Swipe).expect("swipe");
        let boss = c.boss();
        let arc = boss
            .hazards
            .iter()
            .find(|h| h.damage == art_boss_cat::SWIPE_DAMAGE)
            .expect("the swipe arc is queued");
        let duration = art_boss_cat::SWIPE_DURATION;
        assert!((arc.warn - art_boss_cat::SWIPE_ACTIVE_START * duration).abs() < 1e-4);
        let expected = (art_boss_cat::SWIPE_ACTIVE_END - art_boss_cat::SWIPE_ACTIVE_START) * duration;
        assert!((arc.active - expected).abs() < 1e-4);
        match arc.shape {
            HazardShape::Arc {
                inner,
                outer,
                half_angle,
                ..
            } => {
                assert!((inner - art_boss_cat::SWIPE_INNER_RADIUS).abs() < 1e-4);
                assert!((outer - art_boss_cat::SWIPE_RADIUS).abs() < 1e-4);
                assert!((half_angle - art_boss_cat::SWIPE_HALF_ANGLE).abs() < 1e-4);
            }
            _ => panic!("the swipe is a sector"),
        }

        // Stand the rat in the outer sector (inside the 30 px arc, outside the
        // cat's body reach) and keep it there.
        let cat = c.boss();
        let standing = cat.pos + cat.facing * 40.0;
        c.set_player(standing);
        // The campain's base regen (0.4 HP/s) heals during the 14 ticks this
        // count runs for (0.09 HP), which used to make the exact-damage
        // assertion read 21.9 instead of 22. This test is about the swipe's
        // number, not about regen: switch regen off for the measurement.
        c.w.player.loadout.stats.regen = 0.0;
        let hp_before = c.player().hp;
        let armor = c.player().loadout.stats.armor;
        assert_eq!(c.count_hits(14), 1, "the swipe must deal damage once");
        let dealt = hp_before - c.player().hp;
        let expected = (art_boss_cat::SWIPE_DAMAGE - armor).max(1.0);
        assert!((dealt - expected).abs() < 1e-3, "dealt {dealt}, want {expected}");
        assert!(
            c.boss()
                .hazards
                .iter()
                .all(|h| h.damage != art_boss_cat::SWIPE_DAMAGE),
            "the arc expires with its active window"
        );
    }

    #[test]
    fn roar_blast_uses_the_art_window_and_hits_once() {
        let mut c = Cat::new(24);
        // 46 px ahead: inside the 26 px blast around the head origin (24 px),
        // outside body contact.
        c.set_player(Vec2::new(786.0, 800.0));
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        let boss = c.boss();
        assert_eq!(boss.state, BossState::Cat(BossCatState::Roar));
        let blast = boss
            .hazards
            .iter()
            .find(|h| h.damage == art_boss_cat::ROAR_DAMAGE)
            .expect("the roar blast is queued");
        let duration = art_boss_cat::ROAR_DURATION;
        assert!((blast.warn - art_boss_cat::ROAR_BLAST_START * duration).abs() < 1e-4);
        let expected = (art_boss_cat::ROAR_BLAST_END - art_boss_cat::ROAR_BLAST_START) * duration;
        assert!((blast.active - expected).abs() < 1e-4);

        // Inside the blast from its first active tick to the first sonic-fang
        // pulse: the rat never leaves the circle, and the i-frames are forced
        // open every tick, so a re-firing hazard would be visible.
        let hp_before = c.player().hp;
        let armor = c.player().loadout.stats.armor;
        let blast_damage = (art_boss_cat::ROAR_DAMAGE - armor).max(1.0);
        let mut events = 0;
        let mut prev = hp_before;
        let mut fired = false;
        for _ in 0..40 {
            c.w.player.iframes = 0.0;
            c.tick();
            let boss = c.boss();
            if boss
                .hazards
                .iter()
                .any(|h| h.damage == art_boss_cat::ROAR_DAMAGE && h.fired)
            {
                fired = true;
            }
            let hp = c.player().hp;
            if (prev - hp - blast_damage).abs() < 1e-3 {
                events += 1;
            }
            prev = hp;
            if boss.progress() >= FANG_PULSES[0] {
                break;
            }
        }
        assert!(fired, "the blast must land inside its active window");
        assert_eq!(events, 1, "the blast must deal damage once");
    }

    #[test]
    fn the_roar_outer_ring_pushes_without_hurting() {
        // The art's outer roar band (`26..44 px`) is a damage-free shove: the
        // blast hammers the inner 26 px and everything out to 44 px is pushed
        // out of the cat's territory instead of hurt.
        let mut c = Cat::new(24);
        c.set_player(Vec2::new(786.0, 800.0));
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        let boss = c.boss();
        assert_eq!(boss.state, BossState::Cat(BossCatState::Roar));
        let ring = boss
            .hazards
            .iter()
            .find(|h| h.damage == 0.0 && matches!(h.shape, HazardShape::Annulus { .. }))
            .copied()
            .expect("the outer roar ring is queued");
        let HazardShape::Annulus { center, inner, outer } = ring.shape else {
            unreachable!()
        };
        assert!((inner - art_boss_cat::ROAR_INNER_RADIUS).abs() < 1e-3);
        assert!((outer - art_boss_cat::ROAR_OUTER_RADIUS).abs() < 1e-3);
        assert!(ring.knockback > 0.0, "the ring must shove");
        let dir = boss.facing;
        let hp = c.player().hp;
        let mut pushed = 0.0f32;
        for _ in 0..40 {
            // Park the rat 30 px ahead of the head: inside the outer band,
            // outside the inner blast and clear of body contact.
            let prev = center + dir * 30.0;
            c.w.player.pos = prev;
            c.w.player.iframes = 0.0;
            c.tick();
            let p = c.player().pos;
            pushed += (p - prev).dot(dir);
            if c.boss().progress() >= FANG_PULSES[0] {
                break;
            }
        }
        assert!(
            (c.player().hp - hp).abs() < 1e-3,
            "the outer ring must not deal damage"
        );
        assert!(pushed > 1.0, "the outer ring must shove the rat outward ({pushed} px)");
    }

    #[test]
    fn phase_two_roar_fires_two_even_sonic_fang_fans() {
        let mut c = Cat::new(25);
        // Far away: the fans must not be eaten by the rat between pulses.
        c.set_player(Vec2::new(1100.0, 800.0));
        c.tick();
        c.edit(|boss| boss.hp = boss.max_hp * 0.45);
        c.tick();
        assert_eq!(c.boss().state, BossState::Cat(BossCatState::Roar));

        let count = |w: &mut World| -> usize {
            let mut count = 0usize;
            let mut query = w.ecs.query::<&BossProjectile>();
            for _ in query.iter(&w.ecs) {
                count += 1;
            }
            count
        };
        // First fan at progress 0.42.
        let mut first = 0;
        for _ in 0..200 {
            c.tick();
            if c.boss().progress() >= FANG_PULSES[0] {
                first = count(&mut c.w);
                break;
            }
        }
        assert_eq!(first, FANG_COUNT, "one fan of eight sonic fangs");
        // Second fan at 0.68.
        let mut second = 0;
        for _ in 0..200 {
            c.tick();
            if c.boss().progress() >= FANG_PULSES[1] {
                second = count(&mut c.w);
                break;
            }
        }
        assert_eq!(second, 2 * FANG_COUNT, "a second fan at 0.68");

        // Evenly spaced around the whole circle (the gaps are wide, not random).
        // Each fan is an even 8-gon (spawn order groups the fans).
        let mut ordered: Vec<(u64, f32, f32)> = Vec::new();
        let mut query = c.w.ecs.query::<(&BossProjectile, &SpawnId)>();
        for (p, id) in query.iter(&c.w.ecs) {
            assert_eq!(p.kind, BossProjectileKind::SonicFang);
            assert!((p.radius - 2.5).abs() < 1e-4);
            ordered.push((id.0, p.vel.len(), p.vel.angle()));
        }
        drop(query);
        assert_eq!(ordered.len(), 2 * FANG_COUNT);
        ordered.sort_by_key(|(id, _, _)| *id);
        let step = std::f32::consts::TAU / FANG_COUNT as f32;
        for fan in ordered.chunks(FANG_COUNT) {
            for (_, speed, _) in fan.iter() {
                assert!((speed - FANG_SPEED).abs() < 1e-3);
            }
            let mut angles: Vec<f32> = fan.iter().map(|(_, _, a)| *a).collect();
            angles.sort_by(|a, b| a.partial_cmp(b).expect("finite angles"));
            for i in 0..FANG_COUNT {
                let next = if i + 1 < FANG_COUNT {
                    angles[i + 1]
                } else {
                    angles[0] + std::f32::consts::TAU
                };
                let gap = next - angles[i];
                assert!(
                    (gap - step).abs() < 1e-3,
                    "fan spacing {gap} is not the even {step}"
                );
            }
        }
    }

    #[test]
    fn the_tail_is_harmless_and_the_body_is_three_art_circles() {
        let mut c = Cat::new(26);
        let boss = c.boss();
        let shapes = boss.hit_shapes();
        assert_eq!(shapes.iter().count(), 3);
        // Behind the haunch, where the 34 px tail is drawn, nothing is hittable.
        let tail_tip = boss.pos - boss.facing * (34.0 + 13.5);
        assert!(!shapes.circle_hits_grounded(tail_tip, 2.0));
        // The art's circles do cover the chest and head.
        assert!(shapes.circle_hits_grounded(boss.pos, 1.0));
        assert!(shapes
            .circle_hits_grounded(boss.pos + boss.facing * art_boss_cat::HEAD_OFFSET_U, 1.0));
    }


    // ---- the designer's home move ---------------------------------------

    #[test]
    fn missed_pounce_overshoots_and_punishes_for_0_7_seconds() {
        let mut c = Cat::new(31);
        c.set_player(Vec2::new(870.0, 800.0));
        c.run_until(400, BossCatState::Crouch).expect("crouch");

        // Lock the landing point, then duck out of the ring.
        let mut locked = None;
        for _ in 0..200 {
            c.tick();
            let boss = c.boss();
            if boss.progress() >= art_boss_cat::CROUCH_LOCK_PROGRESS {
                locked = Some(boss.target);
                break;
            }
        }
        let landing = locked.expect("the crouch locks its landing point");
        c.set_player(landing + Vec2::new(0.0, 140.0));
        let hp_at_dodge = c.player().hp;

        // The pounce commits to the locked point and misses.
        let mut daze = None;
        for _ in 0..200 {
            c.tick();
            let boss = c.boss();
            if boss.state == BossState::Cat(BossCatState::Stalk)
                && (boss.duration - MISS_PUNISH).abs() < 1e-4
            {
                daze = Some(boss);
                break;
            }
        }
        let boss = daze.expect("a miss puts the cat in the 0.7 s punish window");
        assert_eq!(boss.duration, MISS_PUNISH);
        assert_eq!(c.player().hp, hp_at_dodge, "the rat was never in the ring");

        // No attack and no pursuit while dazed; the cat skids forward instead.
        let skid_start = boss.pos;
        let mut states = Vec::new();
        for _ in 0..(MISS_PUNISH / DT).ceil() as usize {
            c.tick();
            let boss = c.boss();
            states.push(match boss.state {
                BossState::Cat(s) => s,
                _ => panic!("cat"),
            });
        }
        assert!(
            states
                .iter()
                .all(|s| *s == BossCatState::Stalk || *s == BossCatState::SwipeWindup),
            "only the follow-up stalk (or a swipe) may begin: {states:?}"
        );
        let skid_end = c.boss().pos;
        assert!(
            skid_end.dist(landing) > skid_start.dist(landing) + 4.0,
            "the miss must carry the cat past the locked point"
        );
        let forward = (skid_end - skid_start).dot(boss.facing);
        assert!(forward > 0.0, "the skid follows the leap direction");

        // The punish window ends and the cat re-engages with a normal beat.
        let after = c.boss();
        assert!(
            (STALK_BEAT_P1.0..=STALK_BEAT_P1.1).contains(&after.duration)
                || after.state != BossState::Cat(BossCatState::Stalk),
            "the daze hands over to a normal stalk beat ({} s)",
            after.duration
        );
    }

    #[test]
    fn missed_pounce_keeps_the_cat_on_its_side_of_a_rat_hole() {
        // A wall column with one rat-hole in it, right where the cat leaps.
        let mut c = Cat::new(32);
        let wall_x = 60;
        let row = 50;
        c.paint_wall(wall_x, row - 6, row + 6);
        c.paint_rat_hole(wall_x, row);

        let boss_pos = cell_center(wall_x - 6, row);
        let near_side = cell_center(wall_x - 2, row);
        let far_side = cell_center(wall_x + 2, row);
        c.edit(|boss| boss.pos = boss_pos);
        c.set_player(near_side);
        assert!(
            c.w.level
                .tiles
                .circle_clear(boss_pos, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "the cat's start fits"
        );
        assert!(
            c.w.level
                .tiles
                .circle_clear(near_side, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "the rat's spot under the pounce fits the cat's body"
        );

        c.run_until(600, BossCatState::Crouch).expect("crouch");
        let mut locked = None;
        for _ in 0..200 {
            c.tick();
            let boss = c.boss();
            if boss.progress() >= art_boss_cat::CROUCH_LOCK_PROGRESS {
                locked = Some(boss.target);
                break;
            }
        }
        let landing = locked.expect("the crouch locks its landing point");
        // Duck through the hole: the rat ends on the far side of the wall.
        c.set_player(far_side);
        let hp_at_dodge = c.player().hp;

        let mut daze = None;
        for _ in 0..200 {
            c.tick();
            let boss = c.boss();
            if boss.state == BossState::Cat(BossCatState::Stalk)
                && (boss.duration - MISS_PUNISH).abs() < 1e-4
            {
                daze = Some(boss);
                break;
            }
        }
        let boss = daze.expect("the hole turns the pounce into a miss");
        // The land was on the cat's side, and the skid cannot cross the hole.
        assert!(landing.x < 16.0 * wall_x as f32);
        assert!(
            boss.pos.x < 16.0 * wall_x as f32 - art_boss_cat::BODY_RADIUS,
            "the cat must skid only on its own side of the wall ({:?})",
            boss.pos
        );
        assert!(boss.pos.x > landing.x - 1.0, "the cat carried on past the land");
        assert!(
            c.w.level
                .tiles
                .circle_clear(boss.pos, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "the skid never enters a wall"
        );
        assert_eq!(c.player().hp, hp_at_dodge, "the rat was never in the ring");
        assert!(c.player().pos.x > 16.0 * wall_x as f32, "the rat is through the hole");
    }

    // ---- footprint / escape-lane checks ----------------------------------

    #[test]
    fn pounce_is_shortened_when_the_rats_spot_is_against_a_wall() {
        let mut c = Cat::new(41);
        let wall_x = 60;
        let row = 50;
        c.paint_wall(wall_x, row - 8, row + 8);
        let wall_face = 16.0 * wall_x as f32;
        let boss_pos = Vec2::new(wall_face - 80.0, cell_center(wall_x, row).y);
        // The rat hugs the wall face: a 10 px gap cannot hold the cat's body,
        // so the leap has to shorten.
        let player = Vec2::new(wall_face - 10.0, boss_pos.y);
        c.edit(|boss| boss.pos = boss_pos);
        c.set_player(player);
        assert!(
            !c.w.level
                .tiles
                .circle_clear(player, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "the rat's own spot cannot hold the cat"
        );

        let crouch = c.run_until(600, BossCatState::Crouch).expect("crouch");
        let leap = crouch.target.dist(crouch.pos);
        let rat = c.player().pos;
        assert!(
            leap < boss_pos.dist(rat) - 4.0,
            "the leap ({leap}) must be shortened from the rat's distance ({})",
            boss_pos.dist(rat)
        );
        assert!(
            c.w.level
                .tiles
                .circle_clear(crouch.target, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "the shortened landing fits"
        );
    }

    #[test]
    fn cat_refuses_attacks_through_a_wall_and_repositions() {
        let mut c = Cat::new(42);
        let wall_x = 60;
        let row = 50;
        c.paint_wall(wall_x, row - 20, row + 20);
        let boss_pos = cell_center(wall_x - 3, row);
        let player = cell_center(wall_x + 3, row);
        c.edit(|boss| boss.pos = boss_pos);
        c.set_player(player);

        let start = c.boss().pos;
        let mut seen = Vec::new();
        for _ in 0..240 {
            c.refresh_flow();
            c.tick();
            let boss = c.boss();
            seen.push(match boss.state {
                BossState::Cat(s) => s,
                _ => panic!("cat"),
            });
        }
        assert!(
            seen.iter().all(|s| *s == BossCatState::Stalk),
            "no attack may start through a wall: {seen:?}"
        );
        let end = c.boss().pos;
        let moved = end.dist(start);
        assert!(moved > 6.0, "the cat must reposition (moved {moved} px)");
        assert!(
            c.w.level
                .tiles
                .circle_clear(end, art_boss_cat::BODY_RADIUS, SizeClass::Large),
            "repositioning never ends inside a wall"
        );
    }

    // ---- the whole fight -------------------------------------------------

    /// A scripted rat: hold a mid range, dodge every telegraph, never dash.
    ///
    /// The pounce is dodged the way the design wants it: sideways, once the
    /// landing point is committed, instead of trying to out-run it.
    fn dodge_dir(boss: &Boss, player: Vec2) -> Vec2 {
        let state = match boss.state {
            BossState::Cat(s) => s,
            _ => BossCatState::Stalk,
        };
        let to_player = player - boss.pos;
        let away_from_boss = to_player.normalized();
        match state {
            BossCatState::Crouch | BossCatState::Pounce => {
                let line = (boss.target - boss.pos).normalized();
                if line.len_sq() <= 0.0 {
                    return away_from_boss;
                }
                // Step off the committed line, on the side the rat is on.
                if to_player.dot(line.perp()) >= 0.0 {
                    line.perp()
                } else {
                    -line.perp()
                }
            }
            BossCatState::SwipeWindup | BossCatState::Swipe => {
                if player.dist(boss.pos) < 48.0 {
                    away_from_boss
                } else {
                    Vec2::ZERO
                }
            }
            BossCatState::Roar => {
                let origin = boss.pos + boss.facing * art_boss_cat::ROAR_ORIGIN_U;
                if player.dist(origin) < 60.0 {
                    (player - origin).normalized()
                } else {
                    Vec2::ZERO
                }
            }
            _ => {
                let distance = player.dist(boss.pos);
                if distance < 66.0 {
                    away_from_boss
                } else if distance > 92.0 {
                    (boss.pos - player).normalized()
                } else {
                    // Strafe so the pounce reads as a commitment.
                    to_player.perp().normalized()
                }
            }
        }
    }

    /// One tick of the boss trace: position, hp and state as exact bits.
    type TraceTick = (u32, u32, u32, u32);

    /// What one scripted fight produced.
    struct FightOutcome {
        alive: bool,
        completions: usize,
        complete: bool,
        trace: Vec<TraceTick>,
    }

    fn scripted_fight(seed: u64, ticks: usize) -> FightOutcome {
        let mut c = Cat::new(seed);
        c.set_player(Vec2::new(900.0, 800.0));
        let mut trace = Vec::new();
        let mut completions = 0usize;
        let mut was_complete = false;
        for _ in 0..ticks {
            let Some(boss) = c.boss_opt() else { break };
            let dir = dodge_dir(&boss, c.w.player.pos);
            c.edit(|boss| {
                boss.take_damage(SCRIPTED_DPS * DT);
            });
            c.tick_with(dir);
            // The death beat ends by despawning the cat; the level-complete
            // signal rises on that same tick, so sample it before the boss
            // goes away.
            if let Some(boss) = c.boss_opt() {
                trace.push((
                    boss.pos.x.to_bits(),
                    boss.pos.y.to_bits(),
                    boss.hp.to_bits(),
                    match boss.state {
                        BossState::Cat(s) => s as u32,
                        _ => 0,
                    },
                ));
            }
            if c.w.level_complete && !was_complete {
                completions += 1;
                was_complete = true;
            }
            if !c.w.player.alive() || c.w.level_complete || c.boss_opt().is_none() {
                break;
            }
        }
        FightOutcome {
            alive: c.w.player.alive(),
            completions,
            complete: c.w.level_complete,
            trace,
        }
    }

    #[test]
    fn a_dodging_rat_survives_and_kills_the_cat_once() {
        let run = scripted_fight(51, (60.0 / DT) as usize);
        assert!(run.alive, "a rat that dodges every telegraph must survive");
        assert!(run.complete, "the cat must die under sustained damage");
        assert_eq!(
            run.completions, 1,
            "the level-complete signal rises exactly once"
        );
    }

    #[test]
    fn twin_scripted_fights_match_tick_for_tick() {
        let a = scripted_fight(4242, (30.0 / DT) as usize);
        let b = scripted_fight(4242, (30.0 / DT) as usize);
        assert_eq!(a.alive, b.alive);
        assert_eq!(a.completions, b.completions);
        assert_eq!(a.complete, b.complete);
        assert_eq!(a.trace.len(), b.trace.len());
        assert!(
            a.trace == b.trace,
            "the boss trace must be identical for one seed"
        );
        assert!(!a.trace.is_empty());
    }
}

