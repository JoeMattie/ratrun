//! Flatbed boss — **THE MAGPIE** (the getaway ride's own fight).
//!
//! The art module [`crate::game::art_boss_magpie`] is the gameplay spec: the
//! state enum, every per-state duration, damage number, hit-circle helper
//! (`body_hit_circles`) and the gust field's constants are binding. This file
//! drives the transitions, chooses the strike points and lanes, and queues the
//! art's telegraphed attacks as shared [`BossHazard`]s.
//!
//! ## The fight the level asked for
//!
//! The flatbed is the trailer level: a long deck running west to east, the road
//! rushing underneath, cargo stacks forcing a weave, holes in the planks and a
//! slipstream that drags light things toward the tailgate. Its boss is the one
//! animal that *is* that motion — a magpie riding the wind over the cab.
//!
//! 1. **`Perch`** holds the rat in the pecking band (`26..62 px`) and circles,
//!    wings tucked; out of its hold it runs the deck at
//!    [`art_boss_magpie::HUNT_SPEED_MULT`] with its wings half open.
//! 2. **Phase 1: the deck-peck.** `PeckWindup` paints the beak line from the
//!    bill through the rat to a marked square and freezes it at
//!    [`art_boss_magpie::PECK_LOCK_PROGRESS`]; `Peck` runs it (single hit);
//!    `Stuck` is the punish window — the bill in the planks, no body contact,
//!    [`art_boss_magpie::STUCK_DAMAGE_MULT`] incoming, the head parked out in
//!    the open where it can be bitten. A bird that lunged with cargo or a
//!    bulkhead behind it cannot pull its bill out: [`art_boss_magpie::STUCK_WALLED`].
//! 3. **Phase 2: the strafing pass.** `Takeoff` → `StrafeWindup` paints a lane
//!    down the bed through the rat → `StrafePass` flies it (the lane is live) →
//!    `SnatchDrop` marks a square under the rat and drops a crate on it.
//! 4. **Phase 3: the gust.** `GustWindup` → `Gust` (three push-only pulses of
//!    [`art_boss_magpie::GUST_PUSH`] px/s toward the tailgate with calm gaps
//!    between, no damage at all), then a peck in the gaps; the rotation is
//!    gust → peck → strafe.
//! 5. **Phase changes** (`0.62`, `0.30`) are a harmless `Ruffle`
//!    ([`art_boss_magpie::RUFFLE_DURATION`] s, no contact, no attack).
//!
//! ## Suppression (every attack, every time)
//!
//! No move is committed when the rat could not read or leave it, or when the
//! bird could not execute it where it stands:
//!
//! * the peck needs line of sight on the rat, a strike between
//!   [`art_boss_magpie::PECK_MIN`] and [`art_boss_magpie::PECK_REACH`], and a
//!   clear step out of the line on at least one side;
//! * the pass needs a lane at least [`art_boss_magpie::STRAFE_MIN_LEN`] long
//!   that the bird's whole body fits along, and
//!   [`art_boss_magpie::STRAFE_SAFE_SIDE`] px of clear floor on one side of the
//!   rat;
//! * the crate needs its marked square on open floor with a clear step off it;
//! * the gust is refused when the rat has no plank behind it (a hole within
//!   [`art_boss_magpie::GUST_PIT_CLEARANCE`]) — the bird will not blow the rat
//!   down a hole it cannot read.
//!
//! A refused move falls through to the cheaper one and finally back to `Perch`
//! (which re-seeks), so the bird never idles on a move it cannot finish.
//!
//! ## Determinism
//!
//! One rng draw per `Perch` entry (which way it paces) and no others; every
//! attack is chosen from a fixed rotation in [`Boss::scratch`]. Nothing
//! allocates per tick: hazards go into the boss's pre-reserved ring and the
//! lane/line probes walk the stack.
//!
//! ## Spawns
//!
//! The controller takes no `Commands`: it moves the boss, queues hazards and
//! cues. No `Enemy` entity is spawned or despawned inside the `EnemyIndex`
//! window by this fight.

use super::*;

// ---- scratch layout -------------------------------------------------------

/// The attack rotation counter (`0..3`, persists across states).
const S_ROT: usize = 0;
/// `1.0` when the committed peck had cargo or a bulkhead behind the bird.
const S_WALLED: usize = 1;
/// The committed lane's `y`.
const S_LANE_Y: usize = 2;
/// The committed lane's direction (`+1`/`-1` along the deck).
pub(crate) const S_LANE_DIR: usize = 3;

// ---- flags layout ---------------------------------------------------------

/// The committed line/lane/crate has been queued.
const F_LOCK: u32 = 1 << 0;
/// The crate has burst (one cue per drop).
const F_CRATE: u32 = 1 << 1;
/// The death beat has taken its one shot (its own bit: `F_LOCK` is the living
/// bird's commit latch and may well be set when it dies).
const F_DEATH: u32 = 1 << 2;
/// Gust pulse `k` has been fired: bits 8..8+[`art_boss_magpie::GUST_PULSES`].
const F_PULSE0: u32 = 1 << 8;

/// How fast the bird slides sideways during a strafe's windup (px/s).
const HOVER_SPEED: f32 = 46.0;

/// The extra lane the bird flies past the rat so the pass always crosses it.
const LANE_OVERSHOOT: f32 = 28.0;

/// How far a lane continues when the bird is outside its hold (hunting).
const LANE_FREE_LEN: f32 = 190.0;

/// The step the rat needs to leave a line or a lane (px).
const ESCAPE_STEP: f32 = 24.0;

// ============================================================================
// Controller
// ============================================================================

/// The Magpie's controller (zero-sized; the state lives on the [`Boss`]).
pub struct MagpieController;

impl BossController for MagpieController {
    fn kind(&self) -> BossKind {
        BossKind::Magpie
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = magpie_state(boss);
        // The bird follows the rat out of the hold (the flow field routes it
        // through the mouth); refreshed every tick, like the mower's.
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase threshold is a set piece: the harmless ruffle.
        if out.phase_changed && !matches!(boss.state, BossState::Magpie(MagpieState::Death)) {
            enter(boss, MagpieState::Ruffle, art_boss_magpie::RUFFLE_DURATION);
            boss.scratch[S_ROT] = 0.0;
        }

        match boss.state {
            BossState::Magpie(MagpieState::Perch) => tick_perch(ctx, boss, out),
            BossState::Magpie(MagpieState::Ruffle) => tick_ruffle(ctx, boss),
            BossState::Magpie(MagpieState::PeckWindup) => tick_peck_windup(ctx, boss),
            BossState::Magpie(MagpieState::Peck) => tick_peck(ctx, boss),
            BossState::Magpie(MagpieState::Stuck) => tick_stuck(ctx, boss),
            BossState::Magpie(MagpieState::Takeoff) => tick_takeoff(ctx, boss),
            BossState::Magpie(MagpieState::StrafeWindup) => tick_strafe_windup(ctx, boss),
            BossState::Magpie(MagpieState::StrafePass) => tick_strafe_pass(ctx, boss),
            BossState::Magpie(MagpieState::SnatchDrop) => tick_snatch_drop(ctx, boss, out),
            BossState::Magpie(MagpieState::Land) => tick_land(ctx, boss),
            BossState::Magpie(MagpieState::GustWindup) => tick_gust_windup(ctx, boss),
            BossState::Magpie(MagpieState::Gust) => tick_gust(ctx, boss, out),
            BossState::Magpie(MagpieState::Death)
            | BossState::Cat(_)
            | BossState::Broom(_)
            | BossState::Mower(_)
            | BossState::Mangle(_)
            | BossState::NightEdition(_) => {}
        }

        // The art owns the multiplier and the contact rule; apply them after the
        // transitions so a punish window is live on the tick it opens.
        let now = magpie_state(boss);
        boss.damage_mult = art_boss_magpie::incoming_damage_mult(now);
        if !art_boss_magpie::has_body_contact(now) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        let after = magpie_state(boss);
        if after != before {
            if let Some(cue) = state_cue(after) {
                out.cue(cue);
            }
        }
    }

    /// The death beat: the bird seizes, keels over and molts.
    ///
    /// Its telegraphs stop with it: the first tick of the beat clears every
    /// hazard the living bird had queued, so a beak line or a lane cannot land
    /// on the rat after the fight is already won. Nothing new is dealt here.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH == 0 {
            boss.flags |= F_DEATH;
            boss.hazards.clear();
            out.cue(BossCue::MagpieDeath);
        }
    }
}

/// The sound/effect cue for entering a state (within-state beats — the gust
/// pulses and the crate's burst — are cued where they happen).
#[inline]
fn state_cue(state: MagpieState) -> Option<BossCue> {
    match state {
        MagpieState::Perch => Some(BossCue::MagpieCry),
        MagpieState::Ruffle => Some(BossCue::MagpieRuffle),
        MagpieState::PeckWindup => Some(BossCue::MagpieWindup),
        MagpieState::Stuck => Some(BossCue::MagpieStuck),
        MagpieState::Takeoff => Some(BossCue::MagpieTakeoff),
        MagpieState::GustWindup => Some(BossCue::MagpieGustGather),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

fn tick_perch(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    let dir = if dist < art_boss_magpie::PERCH_BAND[0] {
        -boss.facing
    } else if dist > art_boss_magpie::PERCH_BAND[1] {
        // Chase through the shared steering: the direct bearing when the rat is
        // in the open, the large-class flow field when cargo is between them.
        ctx.seek(boss)
    } else {
        // Hold the band by pacing around the rat (a bird never stands still).
        boss.facing.perp() * boss.scratch[S_ROT].signum().max(1.0)
    };
    let speed = art_boss_magpie::PERCH_SPEED[slot]
        * if boss.hunting {
            art_boss_magpie::HUNT_SPEED_MULT
        } else {
            1.0
        };
    move_boss(ctx, boss, dir, speed);
    if boss.progress() >= 1.0 {
        next_attack(ctx, boss, out);
    }
}

fn tick_ruffle(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The phase-change pose: no contact, no attack, free damage (the art rule
    // `has_body_contact` owns the contact side).
    if boss.progress() >= 1.0 {
        enter_perch(boss);
    }
}

fn tick_peck_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The line follows the rat until the art's lock, then freezes so it can be
    // stepped out of (the cat's landing ring rule).
    let locked = boss.progress() >= art_boss_magpie::PECK_LOCK_PROGRESS;
    if !locked {
        if let Some(strike) = pick_strike(ctx, boss) {
            boss.target = strike;
            let to = strike - boss.pos;
            if to.len_sq() > 1e-6 {
                boss.facing = to.normalized();
            }
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        // Commit: the strike point the rat was shown a moment ago.
        let Some(strike) = pick_strike(ctx, boss) else {
            // The rat left the line's reach (walked behind cargo): the lunge is
            // refused and the bird repositions instead of stabbing at nothing.
            enter_perch(boss);
            return;
        };
        boss.flags |= F_LOCK;
        boss.target = strike;
        let to = strike - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        // Cornered-bird rule: cargo or a bulkhead behind the bird means it will
        // not get its bill back out.
        let behind = boss.pos - boss.facing * art_boss_magpie::WALL_BACK_PROBE;
        let walled = !ctx.footprint_fits(behind, art_boss_magpie::BODY_RADIUS);
        boss.scratch[S_WALLED] = if walled { 1.0 } else { 0.0 };

        let beak = boss.pos + boss.facing * (art_boss_magpie::HEAD_OFFSET + art_boss_magpie::BEAK_LENGTH);
        let warn = (boss.duration * (1.0 - art_boss_magpie::PECK_LOCK_PROGRESS)).max(0.02);
        // The line is a corridor from the bill to the marked square, only as
        // wide as the bill: a kick over its edge is enough to live.
        let line = BossHazard::new(
            HazardShape::Corridor {
                from: beak,
                to: strike,
                half_width: art_boss_magpie::PECK_ACTIVE_HALF_WIDTH,
            },
            warn,
            art_boss_magpie::PECK_ACTIVE,
            art_boss_magpie::PECK_DAMAGE,
        )
        .with_knockback(90.0);
        let _ = boss.push_hazard(line);
    }
    if boss.progress() >= 1.0 {
        boss.pounce_origin = boss.pos;
        enter(boss, MagpieState::Peck, art_boss_magpie::PECK_STRIKE);
    }
}

fn tick_peck(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Drive the head down the line: the body follows the strike point until the
    // bill reaches it.
    let want = boss.target - boss.facing * (art_boss_magpie::HEAD_OFFSET + art_boss_magpie::BEAK_LENGTH);
    let to = want - boss.pos;
    if to.len() > 1.5 {
        move_boss(ctx, boss, to, art_boss_magpie::PECK_LUNGE_SPEED);
    }
    if boss.progress() >= 1.0 {
        let walled = boss.scratch[S_WALLED] > 0.5;
        let duration = if walled {
            art_boss_magpie::STUCK_WALLED
        } else {
            art_boss_magpie::STUCK_DURATION[phase_slot(boss)]
        };
        enter(boss, MagpieState::Stuck, duration);
    }
}

fn tick_stuck(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The bill is in the plank: no lunging, no turning — the bird is nailed to
    // its own strike. (No contact is applied by the art rule.)
    let _ = ctx;
    if boss.progress() >= 1.0 {
        enter_perch(boss);
    }
}

fn tick_takeoff(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        // The lane is planned on entry so the art paints it on the first frame
        // of the windup; `queue_lane` re-plans with the same inputs at the lock.
        let slot = phase_slot(boss);
        plan_lane(ctx, boss);
        enter(boss, MagpieState::StrafeWindup, art_boss_magpie::STRAFE_WINDUP[slot]);
    }
}

fn tick_strafe_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Hover onto the lane's line (it passes through the rat), then hold.
    let lane_y = boss.scratch[S_LANE_Y];
    let dy = lane_y - boss.pos.y;
    if dy.abs() > 1.0 {
        move_boss(ctx, boss, Vec2::new(0.0, dy), HOVER_SPEED);
    }
    let dir = boss.scratch[S_LANE_DIR];
    boss.facing = Vec2::new(dir.signum(), 0.0);
    boss.target = Vec2::new(boss.target.x, lane_y);
    if boss.flags & F_LOCK == 0 && boss.progress() >= 0.55 {
        boss.flags |= F_LOCK;
        queue_lane(ctx, boss);
    }
    if boss.progress() >= 1.0 {
        // If the lane was never queued (it lost its fit), fall back to a peck.
        if boss.flags & F_LOCK == 0 {
            enter_perch(boss);
            return;
        }
        let slot = phase_slot(boss);
        let len = boss.target.dist(boss.pos);
        let secs = (len / art_boss_magpie::STRAFE_SPEED[slot]).max(0.12);
        boss.pounce_origin = boss.pos;
        enter(boss, MagpieState::StrafePass, secs);
    }
}

fn tick_strafe_pass(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let dir = Vec2::new(boss.scratch[S_LANE_DIR].signum(), 0.0);
    let slot = phase_slot(boss);
    let moved = move_boss(ctx, boss, dir, art_boss_magpie::STRAFE_SPEED[slot]);
    let past = (boss.pos.x - boss.target.x) * dir.x >= 0.0;
    if !moved || past || boss.progress() >= 1.0 {
        enter(boss, MagpieState::SnatchDrop, art_boss_magpie::CRATE_DURATION);
        boss.pounce_origin = boss.pos;
        boss.flags &= !F_CRATE;
        commit_crate(ctx, boss);
    }
}

fn tick_snatch_drop(_ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    // The crate bursts at the art's beat; after that the bird just wheels.
    if boss.flags & F_CRATE == 0 && boss.progress() >= art_boss_magpie::CRATE_LAND_PROGRESS {
        boss.flags |= F_CRATE;
        out.cue(BossCue::MagpieCrate);
    }
    if boss.progress() >= 1.0 {
        enter(boss, MagpieState::Land, art_boss_magpie::LAND_DURATION);
    }
}

fn tick_land(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        enter_perch(boss);
    }
}

fn tick_gust_windup(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Face down the deck: the gust goes tailward, not at the rat.
    boss.facing = -art_boss_magpie::GUST_AXIS;
    if boss.progress() >= 1.0 {
        enter(boss, MagpieState::Gust, art_boss_magpie::GUST_DURATION);
    }
}

fn tick_gust(_ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let t = boss.progress() * art_boss_magpie::GUST_DURATION;
    let mut fired = 0u32;
    for k in 0..art_boss_magpie::GUST_PULSES {
        if boss.flags & (F_PULSE0 << k) != 0 {
            fired += 1;
        }
    }
    for k in fired..art_boss_magpie::GUST_PULSES {
        if !art_boss_magpie::gust_pulse_due(t, k) {
            break;
        }
        boss.flags |= F_PULSE0 << k;
        out.cue(BossCue::MagpieGust);
        // Push-only: zero damage, so no i-frames are spent and the follow-up
        // peck in the calm gap still hurts.
        let pulse = BossHazard::new(
            art_boss_magpie::gust_shape(boss.pos),
            0.0,
            art_boss_magpie::GUST_PULSE_ACTIVE,
            0.0,
        )
        .with_knockback(art_boss_magpie::GUST_PUSH);
        let _ = boss.push_hazard(pulse);
    }
    if boss.progress() >= 1.0 {
        // A peck out of the gust: the rat is downwind and simplified.
        enter_peck(boss);
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
fn magpie_state(boss: &Boss) -> MagpieState {
    match boss.state {
        BossState::Magpie(s) => s,
        _ => MagpieState::Perch,
    }
}

/// Enter an art state, clearing the per-state latch and the pulse bits.
#[inline]
fn enter(boss: &mut Boss, state: MagpieState, duration: f32) {
    boss.set_state(BossState::Magpie(state), duration);
    boss.flags = 0;
}

/// Enter the peck's telegraph (the line is picked at the lock).
fn enter_peck(boss: &mut Boss) {
    let slot = phase_slot(boss);
    enter(boss, MagpieState::PeckWindup, art_boss_magpie::PECK_WINDUP[slot]);
}

fn enter_perch(boss: &mut Boss) {
    let slot = phase_slot(boss);
    enter(boss, MagpieState::Perch, art_boss_magpie::PERCH_DURATION[slot]);
}

/// Choose the next attack from the phase's rotation (no rng).
fn next_attack(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let _ = out;
    let rot = boss.scratch[S_ROT] as u32;
    boss.scratch[S_ROT] = ((rot + 1) % 3) as f32;
    let phase = boss.phase.clamp(1, 3);
    let want = match phase {
        1 => Attack::Peck,
        2 => {
            if rot.is_multiple_of(2) {
                Attack::Strafe
            } else {
                Attack::Peck
            }
        }
        _ => match rot % 3 {
            0 => Attack::Gust,
            1 => Attack::Peck,
            _ => Attack::Strafe,
        },
    };
    // Fall through whatever the floor refuses, then re-seek.
    let mut pick = want;
    for _ in 0..3 {
        let ok = match pick {
            Attack::Peck => commit_peck(ctx, boss),
            Attack::Strafe => commit_strafe(ctx, boss),
            Attack::Gust => commit_gust(ctx, boss),
        };
        if ok {
            return;
        }
        pick = match pick {
            Attack::Peck => {
                // The peck is the cheapest and the last resort of the three; a
                // refused peck means the rat is not reachable — reposition.
                enter_perch(boss);
                return;
            }
            Attack::Strafe => Attack::Peck,
            Attack::Gust => Attack::Peck,
        };
    }
    enter_perch(boss);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attack {
    Peck,
    Strafe,
    Gust,
}

/// Start a peck if the rat is visible, in reach and has somewhere to step.
fn commit_peck(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(strike) = pick_strike(ctx, boss) else {
        return false;
    };
    let dir = (strike - boss.pos).normalized();
    // The tell is a line on the planks: the rat has to be able to see it, and
    // to step off it on at least one side.
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir) {
        return false;
    }
    boss.target = strike;
    if dir.len_sq() > 1e-6 {
        boss.facing = dir;
    }
    enter(boss, MagpieState::PeckWindup, art_boss_magpie::PECK_WINDUP[phase_slot(boss)]);
    true
}

/// The point the bill would strike: `PECK_MIN..=PECK_REACH` px down the line to
/// the rat, stopped by the first solid the bill would hit.
fn pick_strike(ctx: &BossContext<'_>, boss: &Boss) -> Option<Vec2> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist < art_boss_magpie::BEAK_LENGTH + 2.0 {
        return None;
    }
    let dir = to * (1.0 / dist);
    let want = dist.min(art_boss_magpie::PECK_REACH);
    if want < art_boss_magpie::PECK_MIN {
        return None;
    }
    // The bill stops at the first solid it would meet (`segment_hit` reports
    // the hit as a 0..=1 parameter along the sweep).
    let mut reach = want;
    if let Some(t) = ctx
        .level
        .tiles
        .segment_hit(boss.pos, boss.pos + dir * want, 1.0)
    {
        reach = reach.min(want * t - 2.0);
    }
    if reach < art_boss_magpie::PECK_MIN {
        return None;
    }
    Some(boss.pos + dir * reach)
}

/// Is there a clear step off a line with direction `dir` at `at`?
fn escape_side_clear(ctx: &BossContext<'_>, at: Vec2, dir: Vec2) -> bool {
    let perp = dir.perp();
    [1.0f32, -1.0].iter().any(|s| {
        let p = at + perp * (*s * ESCAPE_STEP);
        ctx.los_clear(at, p, ctx.player_radius)
    })
}

/// Start a strafing pass if a lane can be flown through the rat.
fn commit_strafe(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    let Some((start, end, dir)) = lane_plan(ctx, boss) else {
        return false;
    };
    // Keep the committed lane on the boss for the art from the takeoff on.
    boss.scratch[S_LANE_Y] = start.y;
    boss.scratch[S_LANE_DIR] = dir;
    boss.pounce_origin = start;
    boss.target = end;
    enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
    true
}

/// Choose a lane: a straight run down the bed at the rat's row, flown toward
/// the side of the hold the rat is on. Returns `(start, end, dir)` or `None`
/// when the run would be too short, clip a stack, or leave the rat no exit.
fn lane_plan(ctx: &BossContext<'_>, boss: &Boss) -> Option<(Vec2, Vec2, f32)> {
    let arena = ctx.arena;
    let inner_min = arena.x as f32 * collision::TILE_F + art_boss_magpie::STRAFE_END_MARGIN;
    let inner_max = arena.right() as f32 * collision::TILE_F - art_boss_magpie::STRAFE_END_MARGIN;
    if inner_max - inner_min < art_boss_magpie::STRAFE_MIN_LEN {
        return None;
    }
    // The lane is flown at the rat's row (so the pass crosses it).
    let lane_y = ctx.player_pos.y.clamp(
        arena.y as f32 * collision::TILE_F + 18.0,
        arena.bottom() as f32 * collision::TILE_F - 18.0,
    );
    // Fly toward whichever side of the hold the rat is on; if it is close
    // enough to be ambiguous, fly the long way.
    let dx = ctx.player_pos.x - boss.pos.x;
    let dir = if dx.abs() > 24.0 {
        dx.signum()
    } else if boss.pos.x > (inner_min + inner_max) * 0.5 {
        -1.0
    } else {
        1.0
    };
    let end_x = if dir > 0.0 { inner_max } else { inner_min };
    let start = Vec2::new(boss.pos.x, lane_y);
    let end = Vec2::new(end_x, lane_y);
    if (end.x - start.x).abs() < art_boss_magpie::STRAFE_MIN_LEN {
        return None;
    }
    // The whole flight has to fit the bird's body, and the rat has to be able
    // to step off the lane.
    let steps = (((end - start).len()) / 8.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let p = start + (end - start) * (i as f32 / steps as f32);
        if !ctx.footprint_fits(p, art_boss_magpie::BODY_RADIUS) {
            return None;
        }
    }
    let lane_dir = Vec2::new(dir, 0.0);
    if !escape_side_clear(ctx, Vec2::new(ctx.player_pos.x, lane_y), lane_dir) {
        return None;
    }
    Some((start, end, dir))
}

/// Queue the pass's live corridor once the windup has shown it.
fn queue_lane(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let Some((start, end, dir)) = lane_plan(ctx, boss) else {
        return;
    };
    boss.scratch[S_LANE_Y] = start.y;
    boss.scratch[S_LANE_DIR] = dir;
    boss.pounce_origin = start;
    boss.target = end;
    let len = (end - start).len();
    let pass = (len / art_boss_magpie::STRAFE_SPEED[slot]).max(0.12);
    let warn = (boss.duration * (1.0 - 0.55)).max(0.02);
    let lane = BossHazard::new(
        HazardShape::Corridor {
            from: start,
            to: end,
            half_width: art_boss_magpie::STRAFE_LANE_HALF_WIDTH,
        },
        warn,
        pass,
        art_boss_magpie::STRAFE_DAMAGE,
    )
    .with_knockback(110.0);
    let _ = boss.push_hazard(lane);
}

/// Plan (again) and remember the lane on entering `StrafeWindup`.
fn plan_lane(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if let Some((start, end, dir)) = lane_plan(ctx, boss) {
        boss.scratch[S_LANE_Y] = start.y;
        boss.scratch[S_LANE_DIR] = dir;
        boss.pounce_origin = start;
        boss.target = end;
    }
}

/// Start a gust if the deck behind the rat is walkable.
fn commit_gust(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    // Never blow the rat down a hole it cannot read: the pulse is refused when
    // the plank behind the rat is already a hole.
    if gust_pit_ahead(ctx).is_some() {
        return false;
    }
    enter(
        boss,
        MagpieState::GustWindup,
        art_boss_magpie::GUST_WINDUP[phase_slot(boss)],
    );
    true
}

/// The pit the gust would push the rat into, if any.
fn gust_pit_ahead(ctx: &BossContext<'_>) -> Option<()> {
    let deck = &ctx.level.trailer;
    if deck.is_empty() {
        return None;
    }
    let p = ctx.player_pos;
    for k in 1..=3 {
        let t = art_boss_magpie::GUST_PIT_CLEARANCE * k as f32 / 3.0;
        if deck.pit_at(p + art_boss_magpie::GUST_AXIS * t).is_some() {
            return Some(());
        }
    }
    None
}

/// The marked square for the dropped crate: under the rat, if the deck there is
/// open and the rat can step off the square.
fn commit_crate(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    boss.flags &= !F_CRATE;
    let at = ctx.player_pos;
    let centre = boss.pos;
    let dir = if ctx.los_clear(centre, ctx.player_pos, 1.0) {
        (ctx.player_pos - centre).normalized()
    } else {
        Vec2::new(0.0, 1.0)
    };
    boss.target = if ctx.footprint_fits(at, art_boss_magpie::CRATE_HALF)
        && escape_side_clear(ctx, at, dir)
    {
        at
    } else {
        // No fair square: the bird drops it on its own feet instead (the square
        // then sits beside it, still readable and still dodgeable).
        boss.pos + dir * (art_boss_magpie::CRATE_HALF + 6.0)
    };
    boss.facing = dir;
    let warn = boss.duration * art_boss_magpie::CRATE_LAND_PROGRESS;
    let crate_hazard = BossHazard::new(
        HazardShape::Circle {
            center: boss.target,
            radius: art_boss_magpie::CRATE_HALF,
        },
        warn,
        0.12,
        art_boss_magpie::CRATE_DAMAGE,
    )
    .with_knockback(art_boss_magpie::CRATE_KNOCKBACK)
    .with_enemies();
    let _ = boss.push_hazard(crate_hazard);
}



#[cfg(test)]
mod tests {
    //! Narrow tests for the Magpie fight.
    //!
    //! `cargo test boss::magpie` runs these with the framework's suite. Every
    //! attack test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, contact and the death
    //! hand-off are the real ones; the two suppression tests call the same
    //! `commit_*` functions the controller calls, with the same `BossContext`
    //! the system builds.

    use super::*;
    use crate::game::art_boss_magpie as art;
    use crate::game::campaign;
    use crate::game::collision::{TileKind, TILE_F};
    use crate::game::level::{Pit, PitKind, Theme};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    // ---- harness ---------------------------------------------------------

    fn trailer_world(seed: u64) -> World {
        let mut w = World::new(Theme::Trailer, seed);
        w.level_index = campaign::LEVEL_COUNT;
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

    /// Spawn the level's boss through the real spawn system.
    fn spawn_magpie(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the magpie");
        assert_eq!(boss.kind, BossKind::Magpie);
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

    fn set_state(w: &mut World, state: MagpieState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::Magpie(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
        });
    }

    fn state_of(w: &mut World) -> MagpieState {
        magpie_state(&boss_of(w).expect("boss alive"))
    }

    /// The arena's centre: the cab hold, which is flat open deck.
    fn arena_centre(w: &World) -> Vec2 {
        w.level
            .boss_room()
            .map(|r| r.rect.center_world())
            .expect("the trailer has a cab hold")
    }

    /// Put the bird at `pos` and the rat at `rat`, both inside the hold.
    fn place(w: &mut World, pos: Vec2, rat: Vec2) {
        with_boss(w, |b| {
            b.pos = pos;
            b.target = rat;
            b.facing = (rat - pos).normalized();
            b.pounce_origin = pos;
        });
        w.player.pos = rat;
        w.player.vel = Vec2::ZERO;
    }

    /// Tick until the state changes or `limit` seconds pass; returns the first
    /// state the bird entered.
    fn drive_until_change(w: &mut World, limit: f32) -> MagpieState {
        let start = state_of(w);
        let mut t = 0.0;
        while t < limit {
            tick(w);
            t += DT;
            let now = state_of(w);
            if now != start {
                return now;
            }
        }
        panic!("{start:?} never changed within {limit:.2} s");
    }

    fn boss_context<'a>(
        w: &'a World,
        rng: &'a mut rand::rngs::StdRng,
        arena: crate::game::level::TileRect,
    ) -> BossContext<'a> {
        let boss_arena = w
            .level
            .boss_room()
            .map(|r| r.rect)
            .unwrap_or(arena);
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            arena: boss_arena,
            level: &w.level,
            flow: &w.flow,
            rng,
        }
    }

    // ---- registration ----------------------------------------------------

    #[test]
    fn the_flatbed_fields_the_magpie() {
        assert_eq!(BossKind::for_theme(Theme::Trailer), BossKind::Magpie);
        assert_eq!(trailer_boss(), BossKind::Magpie);
        // The campaign floors are untouched.
        assert_eq!(BossKind::for_theme(Theme::Sewer), BossKind::Cat);
        assert_eq!(BossKind::for_level(0), BossKind::Cat);
        let mut w = trailer_world(5);
        let boss = spawn_magpie(&mut w);
        assert_eq!(boss.kind, BossKind::Magpie);
        assert!(boss.hp > 0.0);
        // The pool is the framework's: art pool x scale x the director's curve.
        let expect = art::BOSS_BASE_HP
            * BossKind::Magpie.hp_scale()
            * crate::game::director::boss_hp_mult_for(campaign::LEVEL_COUNT, crate::game::director::BOSS_TIME);
        assert!((boss.max_hp - expect).abs() < 1.0, "pool {} vs {expect}", boss.max_hp);
    }

    /// The boss bar, the pre-boss countdown and the level-complete banner all
    /// take their name from `hud::boss_codename`, which follows
    /// `BossKind::for_theme` — so the flatbed's plate names the bird.
    #[test]
    fn the_name_plate_names_the_magpie() {
        assert_eq!(
            crate::render::hud::boss_codename(Theme::Trailer),
            ("THE MAGPIE", "CLASS-V CORVID THIEF")
        );
        // The campaign plates are unchanged.
        assert_eq!(crate::render::hud::boss_codename(Theme::Sewer).0, "APEX JAGUAR");
        assert_eq!(crate::render::hud::boss_codename(Theme::Kitchen).0, "THE SWEEPER");
        assert_eq!(crate::render::hud::boss_codename(Theme::Lab).0, "TURF-REAPER MK.IV");
    }

    /// The real tick schedule (director → `BossDue` → framework) fields the
    /// bird on the flatbed at 15:00, in the cab hold, with the HUD's accessors
    /// and the boss bar's name answering for it.
    #[test]
    fn the_real_schedule_spawns_the_magpie_on_the_flatbed() {
        let mut w = trailer_world(41);
        w.elapsed = crate::game::director::BOSS_TIME;
        w.update(DT, Vec2::ZERO, false);
        assert!(w.boss_time_reached());
        let boss = boss_of(&mut w).expect("the schedule spawned the boss");
        assert_eq!(boss.kind, BossKind::Magpie);
        let arena = w.level.boss_room().expect("hold").rect;
        assert!(
            arena.contains_world(boss.pos),
            "the bird spawns in the cab hold ({:?})",
            boss.pos
        );
        let (hp, max_hp) = w.boss_hp().expect("the HUD accessor reports the boss");
        assert!((hp - boss.hp).abs() < 1e-3 && (max_hp - boss.max_hp).abs() < 1e-3);
        assert!(w.boss_offset().is_some());
        assert!(w.boss_marker().is_some());
        // Keep the fight running for a few seconds of real ticks.
        for _ in 0..240 {
            w.update(DT, Vec2::ZERO, false);
        }
        assert!(boss_of(&mut w).is_some());
    }

    #[test]
    fn phases_follow_the_art() {
        assert_eq!(BossKind::Magpie.phase_for(1.0), 1);
        assert_eq!(BossKind::Magpie.phase_for(art::PHASE2_HP_FRAC), 2);
        assert_eq!(BossKind::Magpie.phase_for(0.5), 2);
        assert_eq!(BossKind::Magpie.phase_for(art::PHASE3_HP_FRAC), 3);
        assert_eq!(BossKind::Magpie.phase_for(0.0), 3);
    }

    // ---- the deck-peck ---------------------------------------------------

    #[test]
    fn the_peck_lands_after_its_tell_and_only_once() {
        let mut w = trailer_world(11);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        // Bird west, rat east: the line runs west -> east along the deck.
        place(&mut w, centre - Vec2::new(40.0, 0.0), centre + Vec2::new(24.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp0 = w.player.hp;
        set_state(&mut w, MagpieState::PeckWindup, art::PECK_WINDUP[0]);

        // The tell: no damage during the windup's first half.
        let mut t = 0.0;
        while t < art::PECK_WINDUP[0] * art::PECK_LOCK_PROGRESS {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0, "the line hurt before it was live");
        }
        // The committed line is on the deck and points at the rat.
        let boss = boss_of(&mut w).expect("alive");
        let strike = boss.target;
        assert!(
            (strike - w.player.pos).len() < 6.0,
            "the strike should be on the rat ({strike:?} vs {:?})",
            w.player.pos
        );

        // Drive the rest of the attack: exactly one hit, the art's number.
        let mut hits = 0;
        for _ in 0..(1.4 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!(
                    (before - w.player.hp - art::PECK_DAMAGE).abs() < 0.6,
                    "peck hit for {} not {}",
                    before - w.player.hp,
                    art::PECK_DAMAGE
                );
            }
            if state_of(&mut w) == MagpieState::Perch {
                break;
            }
        }
        assert_eq!(hits, 1, "a peck is a single hit");
    }

    #[test]
    fn a_dodged_peck_opens_the_punish_window() {
        let mut w = trailer_world(12);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(50.0, 0.0));
        set_state(&mut w, MagpieState::PeckWindup, art::PECK_WINDUP[0]);
        // Lock the line at the rat, then have the rat step out of it (south).
        while boss_of(&mut w).expect("alive").flags & F_LOCK == 0 {
            tick(&mut w);
        }
        w.player.pos = centre + Vec2::new(50.0, 34.0);
        let hp = w.player.hp;
        // Run through the strike.
        let mut t = 0.0;
        while state_of(&mut w) != MagpieState::Stuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MagpieState::Stuck);
        assert_eq!(w.player.hp, hp, "the rat stepped off the line");
        let boss = boss_of(&mut w).expect("alive");
        // The punish window: the art's multiplier, no body contact, and the
        // head parked out at the strike point where the rat can bite it.
        assert!((boss.damage_mult - art::STUCK_DAMAGE_MULT).abs() < 1e-6);
        assert!(boss.contact_damage <= 0.0, "a buried bird cannot bump");
        assert!((boss.duration - art::STUCK_DURATION[0]).abs() < 1e-3);
        let view = super::super::Boss::magpie_view(&boss);
        let head = art::head_center(&view);
        assert!(
            (head - boss.target).len() < 0.5,
            "the weak point is the buried head"
        );
        assert!(boss.hit_shapes().circle_hits_grounded(head, 2.0));
        // ...and it lasts exactly its beat, then goes back to work.
        let mut t = 0.0;
        while state_of(&mut w) == MagpieState::Stuck && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert!((t - art::STUCK_DURATION[0]).abs() < 0.10, "stuck {t}");
        assert_eq!(state_of(&mut w), MagpieState::Perch);
    }

    #[test]
    fn a_cornered_bird_stays_stuck_longer() {
        let mut w = trailer_world(13);
        spawn_magpie(&mut w);
        // A cargo column right behind the bird: it can lunge but it cannot get
        // any purchase to pull its bill back out.
        let centre = arena_centre(&w);
        let arena = w.level.boss_room().expect("hold").rect;
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + 2,
            1,
            arena.h - 4,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        let row = (arena.y + arena.h / 2) as f32 * TILE_F + TILE_F * 0.5;
        // 24 px east of the cargo face: the back-probe reaches into it.
        let wall_x = (arena.x + 3) as f32 * TILE_F + 24.0;
        place(
            &mut w,
            Vec2::new(wall_x, row),
            Vec2::new(wall_x + 44.0, row),
        );
        let _ = centre;
        set_state(&mut w, MagpieState::PeckWindup, art::PECK_WINDUP[0]);
        let mut t = 0.0;
        while state_of(&mut w) != MagpieState::Stuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MagpieState::Stuck);
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.duration - art::STUCK_WALLED).abs() < 1e-3,
            "cornered window {} should be {}",
            boss.duration,
            art::STUCK_WALLED
        );
    }

    // ---- the strafing pass -----------------------------------------------

    #[test]
    fn a_pass_paints_a_lane_through_the_rat_and_hits_in_it() {
        let mut w = trailer_world(14);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        let rat = centre + Vec2::new(60.0, 20.0);
        place(&mut w, centre - Vec2::new(70.0, 0.0), rat);
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp0 = w.player.hp;

        // Enter the windup with the lane planned the way the controller does.
        {
            let mut boss = boss_of(&mut w).expect("alive");
            let mut rng = rand::rngs::StdRng::seed_from_u64(1);
            let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
            plan_lane(&mut ctx, &mut boss);
            with_boss(&mut w, |b| {
                b.pounce_origin = boss.pounce_origin;
                b.target = boss.target;
                b.scratch[S_LANE_DIR] = boss.scratch[S_LANE_DIR];
                b.scratch[S_LANE_Y] = boss.scratch[S_LANE_Y];
            });
        }
        let lane_end = boss_of(&mut w).expect("alive").target;
        let lane_start = boss_of(&mut w).expect("alive").pounce_origin;
        set_state(&mut w, MagpieState::StrafeWindup, art::STRAFE_WINDUP[0]);
        // The lane passes through the rat's row.
        assert!(
            (lane_start.y - w.player.pos.y).abs() < 20.0
                && (lane_end.y - w.player.pos.y).abs() < 20.0,
            "lane {lane_start:?}..{lane_end:?} misses the rat at {:?}",
            w.player.pos
        );
        assert!((lane_end.x - lane_start.x).abs() >= art::STRAFE_MIN_LEN);

        // Run the windup and the pass: the rat stands in the lane and is hit
        // once, for the art's number.
        let mut hits = 0;
        for _ in 0..(4.0 / DT) as usize {
            let before = w.player.hp;
            let state = state_of(&mut w);
            if state != MagpieState::StrafeWindup && state != MagpieState::StrafePass {
                break;
            }
            // Keep the rat standing in the lane (it is a test dummy).
            tick(&mut w);
            w.player.pos = rat;
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!((before - w.player.hp - art::STRAFE_DAMAGE).abs() < 0.6);
            }
        }
        assert_eq!(hits, 1, "a pass is a single hit");
        let _ = hp0;
    }

    // ---- the gust --------------------------------------------------------

    #[test]
    fn the_gust_pushes_without_damaging() {
        let mut w = trailer_world(15);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        // The bird upwind (east) of the rat, facing down the deck.
        place(
            &mut w,
            centre + Vec2::new(70.0, 0.0),
            centre - Vec2::new(20.0, 0.0),
        );
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp0 = w.player.hp;
        w.player.iframes = 0.0;
        set_state(&mut w, MagpieState::Gust, art::GUST_DURATION);
        let start = w.player.pos;
        let mut pushed = 0.0f32;
        let mut last = start.x;
        for _ in 0..(art::GUST_DURATION / DT) as usize {
            tick(&mut w);
            pushed += last - w.player.pos.x;
            last = w.player.pos.x;
            // The push-only hazard never spends a hit point.
            assert_eq!(w.player.hp, hp0, "the gust must not damage");
            w.player.iframes = 0.0;
        }
        assert!(
            w.player.pos.x < start.x - 10.0,
            "the gust must shove the rat tailward ({start:?} -> {:?}, total {pushed})",
            w.player.pos
        );
        // Escapable: the average push over a full beat is far under a walk.
        let average = (start.x - w.player.pos.x) / art::GUST_DURATION;
        assert!(
            average < 58.0 * 0.85,
            "average push {average} px/s must be out-walkable"
        );
    }

    #[test]
    fn the_gust_is_refused_over_a_hole() {
        let mut w = trailer_world(16);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        let rat = centre - Vec2::new(10.0, 0.0);
        place(&mut w, centre + Vec2::new(60.0, 0.0), rat);
        // A hole in the plank just tailward of the rat.
        let cell = (
            ((rat.x - art::GUST_PIT_CLEARANCE + 8.0) / TILE_F).floor() as i32,
            (rat.y / TILE_F).floor() as i32,
        );
        w.level
            .trailer
            .pits
            .push(Pit::new(crate::game::level::TileRect::new(cell.0, cell.1, 1, 1), PitKind::BrokenPlank));
        assert!(w.level.trailer.pit_at(rat - Vec2::new(art::GUST_PIT_CLEARANCE - 8.0, 0.0)).is_some());
        let mut rng = rand::rngs::StdRng::seed_from_u64(2);
        let mut boss = boss_of(&mut w).expect("alive");
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(
            !commit_gust(&mut ctx, &mut boss),
            "the bird must not blow the rat down a hole"
        );
        // With the hole gone the same turn is a gust.
        w.level.trailer.pits.clear();
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(commit_gust(&mut ctx, &mut boss));
        assert_eq!(magpie_state(&boss), MagpieState::GustWindup);
    }

    // ---- suppression -----------------------------------------------------

    #[test]
    fn a_peck_with_no_way_out_is_refused() {
        let mut w = trailer_world(17);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        let arena = w.level.boss_room().expect("hold").rect;
        // A one-cell slot: walls north and south of the rat, so there is no
        // sidestep off the line.
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + (arena.h / 2) - 1,
            arena.w - 4,
            1,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + (arena.h / 2) + 1,
            arena.w - 4,
            1,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        let row = Vec2::new(centre.x, (arena.y + (arena.h / 2)) as f32 * TILE_F + 8.0);
        place(&mut w, row - Vec2::new(40.0, 0.0), row + Vec2::new(16.0, 0.0));
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut boss = boss_of(&mut w).expect("alive");
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(
            !commit_peck(&mut ctx, &mut boss),
            "a line the rat cannot leave is not a fair attack"
        );
        assert_eq!(magpie_state(&boss), MagpieState::Perch);
    }

    // ---- phase change ----------------------------------------------------

    #[test]
    fn a_phase_threshold_is_a_harmless_ruffle() {
        let mut w = trailer_world(18);
        spawn_magpie(&mut w);
        let max = boss_of(&mut w).expect("alive").max_hp;
        with_boss(&mut w, |b| b.hp = max * (art::PHASE2_HP_FRAC - 0.01));
        tick(&mut w);
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(boss.phase, 2);
        assert_eq!(magpie_state(&boss), MagpieState::Ruffle);
        assert!((boss.duration - art::RUFFLE_DURATION).abs() < 1e-3);
        assert!(boss.contact_damage <= 0.0, "the ruffle is harmless");
        assert!(boss.hazards.is_empty(), "the ruffle queues nothing");
        // ...and it lasts its beat, then the bird goes back to work.
        let mut t = 0.0;
        while state_of(&mut w) == MagpieState::Ruffle && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert!((t - art::RUFFLE_DURATION).abs() < 0.08, "ruffle {t}");
        assert_eq!(state_of(&mut w), MagpieState::Perch);
    }

    // ---- death -----------------------------------------------------------

    #[test]
    fn the_birds_hazards_stop_when_it_dies() {
        let mut w = trailer_world(19);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(40.0, 0.0));
        // A live beak line on the planks...
        set_state(&mut w, MagpieState::PeckWindup, art::PECK_WINDUP[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        assert!(!boss_of(&mut w).expect("alive").hazards.is_empty());
        // ...and the bird killed on it.
        with_boss(&mut w, |b| b.hp = 0.0);
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp = w.player.hp;
        tick(&mut w);
        let boss = boss_of(&mut w).expect("dying");
        assert!(boss.dead);
        assert!(boss.hazards.is_empty(), "a dead boss owns no telegraphs");
        let mut t = 0.0;
        while boss_of(&mut w).is_some() && t < art::DEATH_DURATION + 1.0 {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp, "the death beat cannot hurt the rat");
        }
        assert!(boss_of(&mut w).is_none(), "the death beat reaps the bird");
        assert!(w.level_complete());
    }

    // ---- the hunt --------------------------------------------------------

    #[test]
    fn the_bird_hunts_a_rat_that_leaves_the_hold() {
        let mut w = trailer_world(20);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        // The rat out on the deck, west of the cab mouth.
        let deck = w.level.trailer.deck;
        let rat = Vec2::new(
            (deck.x as f32 + 6.0) * TILE_F,
            (deck.y as f32 + deck.h as f32 * 0.5) * TILE_F,
        );
        place(&mut w, centre, rat);
        w.clear_enemies();
        let d0 = boss_of(&mut w).expect("alive").pos.dist(rat);
        for _ in 0..180 {
            w.begin_tick(DT, Vec2::ZERO, false);
            let _ = w
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::flow_refresh_system);
            let _ = w.ecs.run_system_cached(boss_update_system);
            w.end_tick();
        }
        let boss = boss_of(&mut w).expect("alive");
        assert!(boss.hunting, "the bird should be hunting");
        let d1 = boss.pos.dist(rat);
        assert!(
            d1 < d0 - 40.0,
            "the bird must close on a rat outside the hold ({d0:.0} -> {d1:.0})"
        );
        // ...and it still attacks out there: the states run through the peck.
        assert!(d1 > 0.0);
    }

    // ---- determinism -----------------------------------------------------

    #[test]
    fn twin_runs_of_a_full_fight_match() {
        let run = |seed: u64| {
            let mut w = trailer_world(seed);
            spawn_magpie(&mut w);
            let centre = arena_centre(&w);
            place(&mut w, centre, centre + Vec2::new(40.0, 0.0));
            let mut rng = rand::rngs::StdRng::seed_from_u64(7);
            for i in 0..(12.0 / DT) as usize {
                // A scripted rat: a slow circle around the bird, so every attack
                // fires and the hazards matter.
                let a = i as f32 * DT * 2.2;
                let boss = boss_of(&mut w).expect("alive");
                w.player.pos = boss.pos + Vec2::new(a.cos(), a.sin()) * 44.0;
                w.player.pos = w.player.pos.clamp_box(vec2_min(), w.level.arena - vec2_min());
                let _ = rng.random::<u32>();
                tick(&mut w);
            }
            let boss = boss_of(&mut w).expect("alive");
            (boss.pos, boss.hp, boss.state, boss.phase, w.player.hp)
        };
        let a = run(31);
        let b = run(31);
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
        assert_eq!(a.2, b.2);
        assert_eq!(a.3, b.3);
        assert_eq!(a.4, b.4);
    }

    fn vec2_min() -> Vec2 {
        Vec2::splat(20.0)
    }

    // ---- the headless fight ---------------------------------------------

    /// A whole fight on the level's real deck, fought by the real bot with a
    /// build a player actually carries to the boss: the boss must die and the
    /// clock must not stall.
    #[test]
    fn a_bot_kills_the_magpie_on_the_real_deck() {
        use crate::autoplay::CarryPreset;
        use crate::game::bot::{Bot, Skill};

        const SEEDS: [u64; 5] = [3, 7, 21, 99, 4242];
        let mut lengths = Vec::new();
        for seed in SEEDS {
            let mut w = trailer_world(seed);
            spawn_magpie(&mut w);
            // A near-complete build (what a survivor of the 15-minute build-up
            // carries to a boss) and the arena.
            w.player.loadout = CarryPreset::Typical.loadout();
            let max = w.player.max_hp();
            w.player.hp = max;
            let centre = arena_centre(&w);
            w.player.pos = centre;
            w.clear_enemies();

            let mut bot = Bot::new(Skill::Expert, seed);
            let mut min_frac = 1.0f32;
            let mut t = 0.0f32;
            let limit = 150.0;
            while t < limit && !w.finished() {
                let input = bot.decide(&w, DT);
                w.update(DT, input.move_dir, input.dash);
                // Spend the cards the way the App does.
                let mut guard = 0;
                while w.pending_levelups > 0 && guard < 32 {
                    guard += 1;
                    let choices = w.player.loadout.generate_choices(&mut w.rng);
                    if choices.is_empty() {
                        w.pending_levelups = 0;
                        break;
                    }
                    let pick = bot.pick_upgrade(&w, &choices);
                    let heal = w.player.loadout.apply(&choices[pick]);
                    w.player.heal(heal);
                    w.pending_levelups = w.pending_levelups.saturating_sub(1);
                }
                min_frac = min_frac.min(w.player.hp / max.max(1.0));
                if w.player.hp < max * 0.85 {
                    w.player.heal(max);
                }
                t += DT;
            }
            assert!(
                w.level_complete() || w.finished(),
                "seed {seed}: the magpie was still alive after {t:.1} s"
            );
            assert!(
                boss_of(&mut w).is_none(),
                "seed {seed}: the death beat did not finish"
            );
            lengths.push((t, min_frac));
        }
        let total: f32 = lengths.iter().map(|(s, _)| *s).sum();
        let worst = lengths.iter().map(|(_, h)| *h).fold(1.0f32, f32::min);
        println!(
            "magpie fights: {} seeds, mean {:.1} s, lowest rat hp {:.0}% ({:?})",
            lengths.len(),
            total / lengths.len() as f32,
            worst * 100.0,
            lengths.iter().map(|(s, _)| *s).collect::<Vec<_>>()
        );
        assert!(
            lengths.iter().all(|(s, _)| *s > 4.0),
            "a fight that ends in four seconds is not a fight: {lengths:?}"
        );
    }
}
