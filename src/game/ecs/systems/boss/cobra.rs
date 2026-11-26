//! The Lab's second boss — **KING COBRA**.
//!
//! The art module [`crate::game::art_boss_cobra`] is the gameplay spec: its
//! state enum, every per-state duration, damage number, geometry helper and
//! the view struct are binding. This file drives the transitions, picks the
//! strike line and the loop's anchor, and queues the art's telegraphed attacks
//! as shared [`BossHazard`]s.
//!
//! ## The fight
//!
//! An escaped specimen, and far longer than the arena is wide. The mower it
//! shares the floor with teaches the value of a miss; this teaches **where you
//! are standing** — the coils are the arena's moving walls, so every big move
//! is a shape the rat has to already be outside of.
//!
//! | state | duration | telegraph → active | damage |
//! |-------|----------|--------------------|--------|
//! | `Rear` | 1.10 / 1.00 / 0.90 s | — | contact 8 |
//! | `HoodTell` | 0.85 / 0.75 / 0.65 s | line freezes at 0.55 | — |
//! | `Strike` | 0.26 s | line live 0.16 s | 20 |
//! | `FangsStuck` | 1.10 / 1.00 / 0.90 s (1.55 walled) | — | **hood ×1.5** |
//! | `ReStrikeTell` | 0.60 / 0.55 / 0.50 s | the second line | — |
//! | `SwayTell` | 1.70 / 1.60 / 1.50 s | the rhythm; locks at 0.78 | — |
//! | `VenomTell` | 0.80 / 0.72 / 0.64 s | three circles at 0.55 | — |
//! | `VenomSpit` | 0.22 s | — | splash 14, drip 8, slow 0.45 |
//! | `TailTell` | 0.80 / 0.72 / 0.64 s | the arc, locks at 0.55 | — |
//! | `TailSweep` | 0.34 s | the arc 12..52 px | 14 |
//! | `TailRecover` | 0.60 / 0.55 / 0.50 s | — | contact 8 |
//! | `ConstrictTell` | 0.95 / 0.90 / 0.85 s | the loop, laid at r 74 | — |
//! | `Constrict` | 2.40 / 2.10 / 1.80 s | closes to r 26 | 18 once, at the close |
//! | `Loosen` | 0.70 s | — | contact 8 |
//! | `SpiralCoil` | 1.05 s | the lane, drawn on | — |
//! | `SpiralChase` | 2.80 s | the head down the lane | contact 8 |
//! | `SpiralEnd` | 1.00 s | — | **hood ×1.4** |
//! | `Rise` | 1.20 s | **harmless pose change** (phase threshold) | contact 8 |
//! | `Death` | 2.60 s | — | none |
//!
//! ## The coil is a wall
//!
//! The loop and the spiral are laid as a fixed chain of body points
//! ([`art::coil_chain`], [`art::spiral_chain`]) which the art paints and
//! [`coil_block_system`] enforces: a **walking** rat is pushed out of the
//! nearest body point, a dashing one passes over it — the same rule the Big
//! Snake's own body uses (`enemy_wave4::snake_tick`), and the reason the
//! constriction has a counter other than "run at the wall".
//!
//! The pass is its own system because a controller has no access to the player
//! (`BossContext` hands it a `&Level` and a `&mut StdRng`, nothing more) and
//! because the coil must not be a tile edit: the map pin folds tiles, and a
//! boss that breaks tiles would move it. It is registered in
//! `schedule::build_schedule` right after `boss::boss_update_system`, so the
//! coil laid this tick separates this tick. A boss with no coil returns before
//! touching anything, so every other fight is byte-identical.
//!
//! ## Suppression (every attack, every time)
//!
//! The owner's grammar: no move is committed when the rat could not read or
//! leave it, or when the animal could not execute it where it stands.
//!
//! * the **strike** needs line of sight, a bite between [`art::STRIKE_MIN`] and
//!   [`art::STRIKE_REACH`], and a clear step off the line;
//! * the **sway** needs the rat inside strike reach (it does not cross the
//!   arena to sway at nothing);
//! * the **venom** needs all three landing circles on open floor;
//! * the **tail** needs the rat inside the arc's reach and a clear step out of
//!   the wedge;
//! * the **constriction** needs the rat inside the loop's start radius and the
//!   whole loop (and the pocket it closes to) on clear floor — otherwise the
//!   "gap" is a wall and the move lies;
//! * the **spiral** needs the rat at least [`art::SPIRAL_INSIDE`] of the way
//!   inside the lane and every turn of the lane on the encounter floor.
//!
//! A refused move falls through the rotation to a cheaper one and finally back
//! to `Rear`, which re-seeks. A phase change, a hunt, or a dead rat never
//! leaves it standing on an unreadable move.
//!
//! ## Determinism and allocation
//!
//! One `f32` draw per open-ended attack decision from `ctx.rng` (the boss's own
//! stream); the opening three decisions are scripted so the fight teaches its
//! own counterplay first. Nothing else draws rng, nothing allocates: hazards go
//! into the boss's pre-reserved ring, and the coil geometry is a pure function
//! of the view (so the art and the separation pass can never disagree).
//!
//! ## Per-controller memory
//!
//! The art view reads `pos`/`facing`/`state`/`progress`/`anim`/`hit_flash`/
//! `hp_frac`/`phase`/`target`/`pounce_origin` plus two scratch slots, so the
//! rest lives in [`Boss::flags`] and [`Boss::scratch`]:
//!
//! * `flags` — bit 0 `LOCK` ("the committed line/arc/clock is queued"), bit 1
//!   `DEATH` (the death beat has cleared its hazards).
//! * `scratch[0]` the loop's opening (radians, read by the art), `scratch[1]`
//!   the rotation index, `scratch[2]` the decision counter (the scripted
//!   opening), `scratch[3]` the second strike's budget, `scratch[4]` `1.0` when
//!   the last strike went into a wall, `scratch[5]` the side the tail comes
//!   round.

use super::*;
use crate::game::art_boss_cobra as art;
use crate::game::art_boss_cobra::CobraState;

/// The King Cobra's controller (zero-sized; the state lives on the [`Boss`]).
pub struct CobraController;

// ============================================================================
// Scratch and flag layout (see the module docs)
// ============================================================================

/// The loop's opening, in radians (the art reads it through the view).
pub(crate) const S_GAP: usize = 0;
/// The attack-rotation index (the neutral beat's pace sign).
const S_ROT: usize = 1;
/// Attack decisions taken (the scripted opening).
const S_DECISIONS: usize = 2;
/// The second strike's budget: `1.0` = this strike is followed by another.
const S_CHAIN: usize = 3;
/// `1.0` when the last strike ended against a wall.
const S_WALLED: usize = 4;
/// `+1`/`-1`: which shoulder the tail comes round.
const S_SIDE: usize = 5;

/// The committed line / arc / clock has been queued.
const F_LOCK: u32 = 1 << 0;
/// The death beat has cleared the living animal's telegraphs.
const F_DEATH: u32 = 1 << 1;

// ============================================================================
// Controller
// ============================================================================

impl BossController for CobraController {
    fn kind(&self) -> BossKind {
        BossKind::Cobra
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = cobrastate(boss);
        // It follows the rat out of the arena on reachable floor (the flow
        // field routes it; `BossContext::seek` prefers the direct bearing).
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase threshold is the art's set piece: the harmless `Rise`.
        // Anything it was winding up is cancelled with the move it belonged to,
        // so a telegraph never fires out of a pose the rat can no longer see.
        if out.phase_changed && cobrastate(boss) != CobraState::Death {
            boss.hazards.clear();
            boss.scratch[S_CHAIN] = 0.0;
            enter(boss, CobraState::Rise, art::RISE_DURATION);
        }

        match cobrastate(boss) {
            CobraState::Rear => tick_rear(ctx, boss),
            CobraState::Rise => tick_rise(ctx, boss),
            CobraState::HoodTell => tick_strike_tell(ctx, boss, art::HOOD_LOCK_PROGRESS),
            CobraState::ReStrikeTell => {
                tick_strike_tell(ctx, boss, art::RESTRIKE_LOCK_PROGRESS)
            }
            CobraState::Strike => tick_strike(ctx, boss),
            CobraState::FangsStuck => tick_stuck(ctx, boss),
            CobraState::SwayTell => tick_sway_tell(ctx, boss),
            CobraState::VenomTell => tick_venom_tell(ctx, boss),
            CobraState::VenomSpit => tick_venom_spit(boss),
            CobraState::TailTell => tick_tail_tell(ctx, boss),
            CobraState::TailSweep => tick_tail_sweep(ctx, boss),
            CobraState::TailRecover => tick_tail_recover(ctx, boss),
            CobraState::ConstrictTell => tick_constrict_tell(ctx, boss),
            CobraState::Constrict => tick_constrict(ctx, boss),
            CobraState::Loosen => tick_loosen(ctx, boss),
            CobraState::SpiralCoil => tick_spiral_coil(ctx, boss),
            CobraState::SpiralChase => tick_spiral_chase(ctx, boss),
            CobraState::SpiralEnd => tick_spiral_end(ctx, boss),
            // The death beat is the framework's, and `death_tick` owns its cue.
            CobraState::Death => {}
        }

        // The art owns the multiplier and the contact rule; apply them after
        // the transitions so a punish window is live on the tick it opens.
        let now = cobrastate(boss);
        boss.damage_mult = art::incoming_damage_mult(now);
        if !art::has_body_contact(now) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        if cobrastate(boss) != before {
            if let Some(cue) = state_cue(cobrastate(boss)) {
                out.cue(cue);
            }
        }
    }

    /// The death beat: the body unspools, the head comes down on the tile.
    ///
    /// Its telegraphs stop with it: the first tick of the beat clears every
    /// hazard the living animal had queued (the loop's closing clock, the venom
    /// puddles, the live lines), so nothing can land on a rat that has already
    /// won. Nothing new is dealt here.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH == 0 {
            boss.flags |= F_DEATH;
            boss.hazards.clear();
            out.cue(BossCue::CobraDeath);
        }
    }
}

/// The sound/effect cue for entering a state (the within-state beats — the
/// spit's landing, the loop's close — are cued where they happen).
#[inline]
fn state_cue(state: CobraState) -> Option<BossCue> {
    match state {
        CobraState::Rear => Some(BossCue::CobraTaste),
        CobraState::Rise => Some(BossCue::CobraRise),
        CobraState::HoodTell => Some(BossCue::CobraHood),
        CobraState::Strike => Some(BossCue::CobraStrike),
        CobraState::FangsStuck => Some(BossCue::CobraStuck),
        CobraState::ReStrikeTell => Some(BossCue::CobraHood),
        CobraState::SwayTell => Some(BossCue::CobraSway),
        CobraState::VenomTell => Some(BossCue::CobraVenomTell),
        CobraState::VenomSpit => Some(BossCue::CobraSpit),
        CobraState::TailTell => Some(BossCue::CobraTailTell),
        CobraState::TailSweep => Some(BossCue::CobraTailSweep),
        CobraState::ConstrictTell => Some(BossCue::CobraCoil),
        CobraState::Constrict => Some(BossCue::CobraTighten),
        CobraState::Loosen => Some(BossCue::CobraLoosen),
        CobraState::SpiralCoil => Some(BossCue::CobraCoil),
        CobraState::SpiralChase => Some(BossCue::CobraSpiral),
        CobraState::SpiralEnd => Some(BossCue::CobraStuck),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

/// `Rear`: the head is up off the floor, tasting the air and tracking the rat
/// with its head while the body piles up behind it.
fn tick_rear(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    let dir = if dist < art::REAR_BAND[0] {
        -boss.facing
    } else if dist > art::REAR_BAND[1] {
        ctx.seek(boss)
    } else {
        // Hold the band by pacing around the rat: a caged animal never stands
        // still, and a circling head keeps the strike's angle moving.
        boss.facing.perp() * pace_sign(boss)
    };
    let speed = art::REAR_SPEED[slot(boss)]
        * if boss.hunting {
            art::HUNT_SPEED_MULT
        } else {
            1.0
        };
    move_boss(ctx, boss, dir, speed);
    if boss.progress() >= 1.0 {
        next_attack(ctx, boss);
    }
}

/// `Rise`: the phase pose change. Harmless, and it keeps the head on the rat.
fn tick_rise(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    let dir = if to.len() > art::REAR_BAND[1] {
        ctx.seek(boss)
    } else {
        Vec2::ZERO
    };
    move_boss(ctx, boss, dir, art::REAR_SPEED[slot(boss)] * 0.6);
    if boss.progress() >= 1.0 {
        enter_rear(boss);
    }
}

/// `HoodTell` / `ReStrikeTell`: the line follows the rat until the art's lock,
/// then freezes so it can be stepped out of (the cat's landing-ring rule).
fn tick_strike_tell(ctx: &mut BossContext<'_>, boss: &mut Boss, lock: f32) {
    if boss.progress() < lock {
        if let Some(p) = strike_point(ctx, boss) {
            boss.target = p;
            let to = p - boss.pos;
            if to.len_sq() > 1e-6 {
                boss.facing = to.normalized();
            }
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        // Commit the line the rat was shown a moment ago; if it can no longer
        // be struck (out of reach, walled off, nowhere to step) the strike is
        // refused and the animal repositions rather than biting at nothing.
        if !commit_strike_line(ctx, boss) {
            enter_rear(boss);
            return;
        }
    }
    if boss.progress() >= 1.0 {
        let duration = art::STRIKE_DURATION;
        enter(boss, CobraState::Strike, duration);
    }
}

/// `Strike`: the head drives down the frozen line to the marked point.
fn tick_strike(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let want = boss.target - boss.facing * art::FANG_REACH;
    let to = want - boss.pos;
    let arrived = to.len() <= 2.5;
    if !arrived
        && !move_boss(ctx, boss, to, art::STRIKE_SPEED) {
            boss.scratch[S_WALLED] = 1.0;
        }
    if boss.progress() >= 1.0 || arrived {
        let walled = boss.scratch[S_WALLED] > 0.5;
        let duration = if walled {
            art::STUCK_WALLED
        } else {
            art::STUCK_DURATION[slot(boss)]
        };
        enter(boss, CobraState::FangsStuck, duration);
    }
}

/// `FangsStuck`: the fangs are in the floor. No movement, no contact, the hood's
/// back exposed — and from phase 2 the animal does not wait to be punished.
fn tick_stuck(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < 1.0 {
        return;
    }
    if boss.phase >= 2 && boss.scratch[S_CHAIN] > 0.5 {
        boss.scratch[S_CHAIN] = 0.0;
        enter(
            boss,
            CobraState::ReStrikeTell,
            art::RESTRIKE_TELL[slot(boss)],
        );
    } else {
        after_attack(boss);
    }
}

/// `SwayTell`: the hypnotic rhythm. The head swings on the beat and the aim
/// does not lock until the art's late beat, so the strike can be read — and
/// baited into the floor the rat has already left.
fn tick_sway_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if boss.progress() < art::SWAY_LOCK_PROGRESS {
        if to.len_sq() > 1e-6 {
            let swing = art::sway_phase(&boss.cobra_view()) * 0.55;
            boss.facing = to.normalized().rotate(swing);
        }
        boss.target = ctx.player_pos;
        return;
    }
    if boss.flags & F_LOCK == 0 {
        // The beat lands where the rat is *now*: three beats of warning and a
        // late lock is the whole tell.
        if !commit_strike_line(ctx, boss) {
            enter_rear(boss);
            return;
        }
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::Strike, art::STRIKE_DURATION);
    }
}

/// `VenomTell`: the three landing circles are painted, then frozen at the lock.
fn tick_venom_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < art::VENOM_LOCK_PROGRESS {
        let to = ctx.player_pos - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        boss.target = ctx.player_pos;
        return;
    }
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        boss.target = ctx.player_pos;
        let to = ctx.player_pos - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        push_venom(boss);
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::VenomSpit, art::VENOM_SPIT);
    }
}

/// `VenomSpit`: the head throws, and the puddles that were marked land. The
/// landing itself was queued at the lock (that is the tell), so this beat only
/// spends its time.
fn tick_venom_spit(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        after_attack(boss);
    }
}

/// `TailTell`: the whip is drawn out of the coil and the arc it will take is
/// painted round the head.
fn tick_tail_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < art::TAIL_LOCK_PROGRESS {
        let to = ctx.player_pos - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        boss.scratch[S_SIDE] = rat_side(ctx, boss);
        return;
    }
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        let warn = (boss.duration - boss.elapsed).max(0.02);
        let _ = boss.push_hazard(
            BossHazard::new(
                HazardShape::Arc {
                    center: boss.pos,
                    inner: art::TAIL_INNER,
                    outer: art::TAIL_OUTER,
                    facing: boss.facing,
                    half_angle: art::TAIL_HALF_ANGLE,
                },
                warn,
                art::TAIL_SWEEP,
                art::TAIL_DAMAGE,
            )
            .with_knockback(art::TAIL_KNOCKBACK),
        );
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::TailSweep, art::TAIL_SWEEP);
    }
}

/// `TailSweep`: the whip takes the arc in one pass. The head is planted.
fn tick_tail_sweep(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let _ = ctx;
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::TailRecover, art::TAIL_RECOVER[slot(boss)]);
    }
}

/// `TailRecover`: it drags a coil's worth of tail back in, off balance.
fn tick_tail_recover(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        after_attack(boss);
    }
}

/// `ConstrictTell`: the loop is on the floor from the first frame (the ring is
/// the tell). The head glides round to its post on the far side of the opening
/// and the closing clock is queued.
fn tick_constrict_tell(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let view = boss.cobra_view();
    let r = art::coil_radius(&view);
    let gap = art::coil_gap_angle(&view);
    // Its post: just outside the loop, opposite the one opening, so the rat has
    // to run past its own exit rather than into the animal.
    let post = boss.target - Vec2::from_angle(gap) * (r + 26.0);
    let _ = move_boss(ctx, boss, post - boss.pos, art::REAR_SPEED[slot(boss)] * 1.5);
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    if boss.flags & F_LOCK == 0 && boss.progress() >= 0.5 {
        boss.flags |= F_LOCK;
        let warn = (boss.duration - boss.elapsed).max(0.02) + art::COIL_TIGHTEN[slot(boss)];
        let _ = boss.push_hazard(
            BossHazard::new(
                HazardShape::Circle {
                    center: boss.target,
                    radius: art::COIL_END_R - art::COIL_BODY_HALF,
                },
                warn,
                0.16,
                art::CONSTRICT_DAMAGE,
            )
            .with_knockback(art::CONSTRICT_KNOCKBACK)
            .with_enemies(),
        );
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::Constrict, art::COIL_TIGHTEN[slot(boss)]);
    }
}

/// `Constrict`: the loop closes. The clock is the hazard's warn window; the head
/// holds its post and watches.
fn tick_constrict(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let view = boss.cobra_view();
    let r = art::coil_radius(&view);
    let gap = art::coil_gap_angle(&view);
    let post = boss.target - Vec2::from_angle(gap) * (r + 24.0);
    let _ = move_boss(ctx, boss, post - boss.pos, art::REAR_SPEED[slot(boss)]);
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::Loosen, art::COIL_LOOSEN);
    }
}

/// `Loosen`: the loop slides off and the head comes back over it.
fn tick_loosen(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        after_attack(boss);
    }
}

/// `SpiralCoil`: it slides back to the arena's centre and lays the whole body
/// down as one lane.
fn tick_spiral_coil(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let _ = move_boss(
        ctx,
        boss,
        boss.pounce_origin - boss.pos,
        art::REAR_SPEED[slot(boss)],
    );
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::SpiralChase, art::SPIRAL_CHASE);
    }
}

/// `SpiralChase`: the head comes down the lane after the rat.
fn tick_spiral_chase(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let dir = ctx.seek(boss);
    move_boss(ctx, boss, dir, art::SPIRAL_SPEED);
    if boss.progress() >= 1.0 {
        enter(boss, CobraState::SpiralEnd, art::SPIRAL_END);
    }
}

/// `SpiralEnd`: the coils drop and the head is spent. The rat that ran the lane
/// gets the hood's back.
fn tick_spiral_end(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    if boss.progress() >= 1.0 {
        after_attack(boss);
    }
}

// ============================================================================
// Transitions
// ============================================================================

#[inline]
fn slot(boss: &Boss) -> usize {
    (boss.phase.clamp(1, 3) - 1) as usize
}

#[inline]
fn cobrastate(boss: &Boss) -> CobraState {
    match boss.state {
        BossState::Cobra(s) => s,
        _ => CobraState::Rear,
    }
}

/// `+1`/`-1`, alternating per attack decision, for the rearing pace.
#[inline]
fn pace_sign(boss: &Boss) -> f32 {
    if (boss.scratch[S_ROT] as i32) % 2 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// Which side of its own facing the rat is on (`+1` left, `-1` right).
#[inline]
fn rat_side(ctx: &BossContext<'_>, boss: &Boss) -> f32 {
    let to = ctx.player_pos - boss.pos;
    if boss.facing.perp().dot(to) >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Enter a state, clearing the per-state latches (but not the death bit).
#[inline]
fn enter(boss: &mut Boss, state: CobraState, duration: f32) {
    boss.set_state(BossState::Cobra(state), duration);
    boss.flags &= !F_LOCK;
    boss.damage_mult = art::incoming_damage_mult(state);
}

/// The rearing beat.
#[inline]
fn enter_rear(boss: &mut Boss) {
    enter(boss, CobraState::Rear, art::REAR_DURATION[slot(boss)]);
}

/// An attack has finished: back to the rearing beat, which picks the next one.
#[inline]
fn after_attack(boss: &mut Boss) {
    enter_rear(boss);
}

// ============================================================================
// Attack selection
// ============================================================================

/// Which attack a decision picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attack {
    Strike,
    Sway,
    Venom,
    Tail,
    Constrict,
    Spiral,
}

impl Attack {
    /// The order to fall through when a move is refused: the next entry is the
    /// cheapest alternative, and `Rear` is the last resort.
    #[inline]
    fn fallback(self) -> Option<Attack> {
        match self {
            Attack::Strike => Some(Attack::Venom),
            Attack::Sway => Some(Attack::Strike),
            Attack::Venom => Some(Attack::Tail),
            Attack::Tail => Some(Attack::Constrict),
            Attack::Constrict => Some(Attack::Venom),
            Attack::Spiral => Some(Attack::Strike),
        }
    }
}

/// End of `Rear`: pick the next beat and start it.
///
/// The opening is scripted — a strike, then the constriction, then the tail —
/// so the rat meets the three moves whose counterplay (step off the line, then
/// bite the hood; leave through the gap or dash the coil; take the floor in
/// front of the head) is the whole fight before anything else is on the table.
/// Afterwards exactly one draw from the boss's own stream decides the beat, so
/// the draw count per decision is constant and seed-reproducible.
fn next_attack(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let decisions = boss.scratch[S_DECISIONS];
    boss.scratch[S_DECISIONS] = decisions + 1.0;
    boss.scratch[S_ROT] = ((boss.scratch[S_ROT] as u32 + 1) % 6) as f32;

    let want = if decisions < 0.5 {
        Attack::Strike
    } else if decisions < 1.5 {
        Attack::Constrict
    } else if decisions < 2.5 {
        Attack::Tail
    } else {
        let roll = ctx.rng.random::<f32>();
        match boss.phase.clamp(1, 3) {
            1 => {
                if roll < 0.42 {
                    Attack::Strike
                } else if roll < 0.72 {
                    Attack::Venom
                } else if roll < 0.90 {
                    Attack::Tail
                } else {
                    Attack::Constrict
                }
            }
            2 => {
                if roll < 0.32 {
                    Attack::Strike
                } else if roll < 0.52 {
                    Attack::Sway
                } else if roll < 0.70 {
                    Attack::Venom
                } else if roll < 0.88 {
                    Attack::Tail
                } else {
                    Attack::Constrict
                }
            }
            _ => {
                if roll < 0.24 {
                    Attack::Strike
                } else if roll < 0.40 {
                    Attack::Sway
                } else if roll < 0.56 {
                    Attack::Venom
                } else if roll < 0.72 {
                    Attack::Tail
                } else if roll < 0.88 {
                    Attack::Constrict
                } else {
                    Attack::Spiral
                }
            }
        }
    };

    let mut pick = Some(want);
    let mut depth = 0;
    while let Some(attack) = pick {
        if depth >= 6 {
            break;
        }
        depth += 1;
        if commit(ctx, boss, attack) {
            return;
        }
        pick = attack.fallback();
    }
    // Nothing fit where it stands: re-seek through the rearing beat.
    enter_rear(boss);
}

/// Try to start one attack. Returns `true` when the state was entered.
fn commit(ctx: &mut BossContext<'_>, boss: &mut Boss, attack: Attack) -> bool {
    match attack {
        Attack::Strike => commit_strike(ctx, boss),
        Attack::Sway => commit_sway(ctx, boss),
        Attack::Venom => commit_venom(ctx, boss),
        Attack::Tail => commit_tail(ctx, boss),
        Attack::Constrict => commit_constrict(ctx, boss),
        Attack::Spiral => commit_spiral(ctx, boss),
    }
}

/// Start the hood flare if the rat is in reach, visible, and has somewhere to
/// step.
fn commit_strike(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(p) = strike_point(ctx, boss) else {
        return false;
    };
    let dir = (p - boss.pos).normalized();
    if dir.len_sq() <= 0.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::STRIKE_ESCAPE) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    boss.target = p;
    boss.facing = dir;
    boss.pounce_origin = boss.pos;
    boss.scratch[S_CHAIN] = if boss.phase >= 2 { 1.0 } else { 0.0 };
    enter(boss, CobraState::HoodTell, art::HOOD_TELL[slot(boss)]);
    true
}

/// Start the sway if the rat is inside strike reach and the head can see it.
fn commit_sway(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if boss.phase < 2 {
        return false;
    }
    let to = ctx.player_pos - boss.pos;
    if to.len() > art::STRIKE_REACH + 24.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    let dir = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        boss.facing
    };
    boss.facing = dir;
    boss.target = ctx.player_pos;
    boss.pounce_origin = boss.pos;
    boss.scratch[S_CHAIN] = if boss.phase >= 2 { 1.0 } else { 0.0 };
    enter(boss, CobraState::SwayTell, art::SWAY_TELL[slot(boss)]);
    true
}

/// Start the spit if all three landing circles are on floor the rat can see.
fn commit_venom(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    if to.len() > art::STRIKE_REACH + 64.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    let dir = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        boss.facing
    };
    for k in 0..art::VENOM_MARKS {
        let at = venom_mark(ctx.player_pos, dir, k);
        if !ctx
            .level
            .tiles
            .circle_clear(at, art::VENOM_RADIUS * 0.6, SizeClass::Small)
        {
            return false;
        }
    }
    boss.facing = dir;
    boss.target = ctx.player_pos;
    boss.pounce_origin = boss.pos;
    enter(boss, CobraState::VenomTell, art::VENOM_TELL[slot(boss)]);
    true
}

/// Start the whip if the rat is inside its reach and can step out of the wedge.
fn commit_tail(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    if to.len() > art::TAIL_OUTER + 30.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    let dir = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        boss.facing
    };
    // The whip's wedge is 150 degrees, so a step *sideways* leaves it: a rat
    // with neither side open is being herded into a corner it cannot read.
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::TAIL_ESCAPE) {
        return false;
    }
    boss.facing = dir;
    boss.pounce_origin = boss.pos;
    boss.scratch[S_SIDE] = rat_side(ctx, boss);
    enter(boss, CobraState::TailTell, art::TAIL_TELL[slot(boss)]);
    true
}

/// Start the constriction if the rat is inside the loop's radius and the whole
/// ring — plus the pocket it closes to — is clear floor.
fn commit_constrict(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist > art::COIL_START_R + 40.0 || dist < art::COIL_END_R {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    let anchor = ctx.player_pos;
    if !ctx
        .level
        .tiles
        .circle_clear(anchor, art::COIL_END_R, SizeClass::Small)
    {
        return false;
    }
    let samples = 36;
    for i in 0..samples {
        let a = i as f32 / samples as f32 * std::f32::consts::TAU;
        let p = anchor + Vec2::from_angle(a) * art::COIL_START_R;
        if !ctx.level.tiles.circle_clear(
            p,
            art::COIL_BODY_HALF + 1.0,
            SizeClass::Small,
        ) {
            return false;
        }
    }
    boss.target = anchor;
    // The opening starts on the far side of the loop from the head: the exit is
    // never behind the animal's own jaw.
    boss.scratch[S_GAP] = (boss.pos - anchor).angle() + std::f32::consts::PI;
    enter(boss, CobraState::ConstrictTell, art::COIL_TELL[slot(boss)]);
    true
}

/// Start the spiral if this is the last phase, the rat is inside the arena, and
/// every turn of the lane is on the encounter floor.
fn commit_spiral(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if boss.phase < 3 {
        return false;
    }
    let centre = ctx.arena.center_world();
    boss.pounce_origin = centre;
    let view = boss.cobra_view();
    let r_max = art::spiral_radius(&view);
    if r_max <= art::SPIRAL_R0 {
        return false;
    }
    if ctx.player_pos.dist(centre) > r_max * art::SPIRAL_INSIDE {
        return false;
    }
    let radii = [art::SPIRAL_R0, (art::SPIRAL_R0 + r_max) * 0.5, r_max];
    for k in 0..12 {
        let a = k as f32 / 12.0 * std::f32::consts::TAU;
        for r in radii {
            let p = centre + Vec2::from_angle(a) * r;
            if !ctx
                .level
                .tiles
                .circle_clear(p, art::COIL_BODY_HALF + 1.0, SizeClass::Small)
            {
                return false;
            }
        }
    }
    boss.target = ctx.player_pos;
    enter(boss, CobraState::SpiralCoil, art::SPIRAL_TELL);
    true
}

/// The point the fangs would land on: `STRIKE_MIN..=STRIKE_REACH` down the line
/// to the rat, stopped by the first solid the fangs would meet.
fn strike_point(ctx: &BossContext<'_>, boss: &Boss) -> Option<Vec2> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist < art::STRIKE_MIN * 0.5 {
        return None;
    }
    let dir = to * (1.0 / dist.max(1e-3));
    let want = dist.min(art::STRIKE_REACH);
    if want < art::STRIKE_MIN {
        return None;
    }
    let mut reach = want;
    if let Some(t) = ctx
        .level
        .tiles
        .segment_hit(boss.pos, boss.pos + dir * want, 2.0)
    {
        reach = reach.min(want * t - 3.0);
    }
    if reach < art::STRIKE_MIN * 0.5 {
        return None;
    }
    Some(boss.pos + dir * reach)
}

/// Freeze the strike line the rat was shown and queue it as a hazard. Returns
/// `false` when the line cannot be committed where the animal stands.
fn commit_strike_line(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(p) = strike_point(ctx, boss) else {
        return false;
    };
    let dir = (p - boss.pos).normalized();
    if dir.len_sq() <= 0.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::STRIKE_ESCAPE) {
        return false;
    }
    boss.flags |= F_LOCK;
    boss.target = p;
    boss.facing = dir;
    boss.pounce_origin = boss.pos;
    // The fangs going into a wall costs the animal a longer, more punishable
    // beat — the same read as the Megarat's teeth in a cabinet.
    let ahead = boss.pos + dir * (art::FANG_WALL_PROBE + art::BODY_RADIUS);
    boss.scratch[S_WALLED] = if ctx.footprint_fits(ahead, boss.kind.body_radius()) {
        0.0
    } else {
        1.0
    };
    let fangs = art::fangs_pos(&boss.cobra_view());
    let warn = (boss.duration - boss.elapsed).max(0.02);
    let _ = boss.push_hazard(
        BossHazard::new(
            HazardShape::Corridor {
                from: fangs,
                to: p,
                half_width: art::STRIKE_HALF_WIDTH,
            },
            warn,
            art::STRIKE_ACTIVE,
            art::STRIKE_DAMAGE,
        )
        .with_knockback(art::STRIKE_KNOCKBACK),
    );
    true
}

/// The three venom marks, relative to the frozen point and the frozen facing.
#[inline]
pub fn venom_mark(target: Vec2, facing: Vec2, k: usize) -> Vec2 {
    let p = if facing.len_sq() > 1e-6 {
        facing.normalized().perp()
    } else {
        Vec2::new(0.0, 1.0)
    };
    match k {
        0 => target,
        1 => target + p * art::VENOM_SPREAD,
        _ => target - p * art::VENOM_SPREAD,
    }
}

/// Queue the three landing circles: one splash each, the puddle that stays, and
/// the drip in it. All of it is the boss's own hazards, so all of it stops with
/// the boss.
fn push_venom(boss: &mut Boss) {
    let warn = (boss.duration - boss.elapsed).max(0.02);
    let facing = boss.facing;
    let target = boss.target;
    for k in 0..art::VENOM_MARKS {
        let at = venom_mark(target, facing, k);
        let splash = BossHazard::new(
            HazardShape::Circle {
                center: at,
                radius: art::VENOM_RADIUS,
            },
            warn,
            0.14,
            art::VENOM_SPLASH,
        )
        .with_knockback(40.0);
        let _ = boss.push_hazard(splash);
        // The puddle: no damage of its own (so it spends no i-frames), it
        // charges the rat a walk-speed tax for as long as it is foul.
        let puddle = BossHazard::new(
            HazardShape::Circle {
                center: at,
                radius: art::VENOM_RADIUS,
            },
            warn,
            art::VENOM_POOL_LIFE,
            0.0,
        )
        .with_slow(art::VENOM_SLOW);
        let _ = boss.push_hazard(puddle);
        let drip = BossHazard::new(
            HazardShape::Circle {
                center: at,
                radius: art::VENOM_RADIUS * 0.8,
            },
            warn + art::VENOM_DRIP_DELAY,
            0.14,
            art::VENOM_DRIP,
        );
        let _ = boss.push_hazard(drip);
    }
}

/// Is there a clear step off a line with direction `dir` at `at`?
fn escape_side_clear(ctx: &BossContext<'_>, at: Vec2, dir: Vec2, step: f32) -> bool {
    let perp = dir.perp();
    [1.0f32, -1.0].iter().any(|s| {
        let p = at + perp * (*s * step);
        let (cx, cy) = collision::cell_of(p);
        ctx.los_clear(at, p, ctx.player_radius)
            && ctx.level.tiles.passable(cx, cy, SizeClass::Small)
    })
}

// ============================================================================
// The coil (a moving wall, outside the EnemyIndex window)
// ============================================================================

/// The King Cobra's laid body is a **wall**: a walking rat is pushed out of the
/// nearest body point, a dashing one passes over it.
///
/// Why a system and not the controller: a controller gets a `&Level`, a
/// `&mut StdRng` and nothing else — no player, no `Commands` — and the coil is
/// not a tile edit (that would move the map pin). It runs right after
/// `boss::boss_update_system`, so the loop laid this tick separates this tick,
/// and it touches nothing at all on a floor with no cobra on it (or a cobra
/// that is not coiled), which is every other fight in the game.
pub fn coil_block_system(
    level: Res<Level>,
    mut player: ResMut<Player>,
    bosses: Query<&Boss>,
) {
    if player.dash_active > 0.0 || !player.alive() {
        return;
    }
    let mut pts = [Vec2::ZERO; art::COIL_MAX];
    for boss in bosses.iter() {
        if boss.dead || boss.kind != BossKind::Cobra {
            continue;
        }
        let view = boss.cobra_view();
        let n = art::blocking_chain(&view, &mut pts);
        if n < 2 {
            continue;
        }
        let min = art::COIL_BODY_HALF + PLAYER_RADIUS;
        // The **nearest** body point decides, and the push is along that
        // point's own normal (the chain's tangent turned a quarter), not along
        // the vector from the point to the rat: a radial push would slide a rat
        // standing *on* the line all the way round the loop, one point at a
        // time. A perpendicular shove puts it off the wall in one pass, which
        // is what a wall is for.
        let mut best = f32::MAX;
        let mut at = 0usize;
        for (i, q) in pts.iter().take(n).enumerate() {
            let d = q.dist_sq(player.pos);
            if d < best {
                best = d;
                at = i;
            }
        }
        let q = pts[at];
        if best >= min * min {
            continue;
        }
        let prev = pts[at.saturating_sub(1)];
        let next = pts[(at + 1).min(n - 1)];
        let tangent = (next - prev).normalized();
        let normal = if tangent.len_sq() > 1e-6 {
            tangent.perp()
        } else {
            art::unit_facing(&view).perp()
        };
        // Which side of the body is the rat on? Exactly on it counts as +1,
        // which is deterministic and matches the art's light coming from the
        // upper left.
        let side = if (player.pos - q).dot(normal) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let want = q + normal * (side * min);
        if level
            .tiles
            .circle_clear(want, PLAYER_RADIUS, SizeClass::Small)
        {
            player.pos = player.clip_giant_displacement(player.pos, want, &level.tiles);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    //! Narrow tests for the King Cobra's fight.
    //!
    //! `cargo test boss::cobra` runs these with the framework's suite. Every
    //! attack test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, contact, the coil's
    //! separation pass and the death hand-off are the real ones; the
    //! suppression and selection tests call the same `commit_*` functions the
    //! controller calls, with the same `BossContext` the system builds.

    use super::*;
    use crate::game::art_boss_cobra as art;
    use crate::game::collision::{TileKind, TILE_F};
    use crate::game::level::TileRect;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    // ---- harness ---------------------------------------------------------

    /// A Lab seed whose pool pick is the Cobra (the pool is seed-driven, so a
    /// test that wants *this* fight asks for a seed that fields it).
    fn cobra_seed(mut seed: u64) -> u64 {
        for _ in 0..8192 {
            if BossKind::for_theme_seeded(Theme::Lab, seed) == BossKind::Cobra {
                return seed;
            }
            seed = seed.wrapping_add(1);
        }
        // Should never happen: the pool is a splitmix64 hash, so both entries
        // appear within a handful of seeds. Fall back rather than fail a whole
        // suite on a hash accident.
        seed
    }

    fn lab_world(seed: u64) -> World {
        let mut w = World::with_tuning(
            Theme::Lab, cobra_seed(seed), crate::tuning::RunTuning::with_base_boss_hazards(BossKind::Cobra),
        );
        // Campaign slot 2 is the Lab, i.e. the mower's floor.
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

    /// Spawn the level's boss through the real spawn system.
    fn spawn_cobra(w: &mut World) -> Boss {
        let boss = spawn_cobra_real(w);
        // The unit tests stage their moves on bare floor: the generated arena's
        // own furniture (benches, pillars) would decide the lines otherwise.
        let arena = w.level.fight_rect_around(w.player.pos);
        w.level
            .tiles
            .fill_cells(arena.x + 1, arena.y + 1, arena.w - 2, arena.h - 2, TileKind::Open);
        w.level.tiles.rebuild_masks();
        boss
    }

    /// The same spawn on the arena exactly as the generator built it.
    fn spawn_cobra_real(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the boss");
        assert_eq!(boss.kind, BossKind::Cobra, "the seed fields the cobra");
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

    /// One tick of the coil's separation pass.
    fn tick_coil(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(coil_block_system)
            .expect("the coil system must run");
        w.end_tick();
    }

    fn set_state(w: &mut World, state: CobraState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::Cobra(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
            b.hazards.clear();
            b.damage_mult = art::incoming_damage_mult(state);
        });
    }

    fn state_of(w: &mut World) -> CobraState {
        cobrastate(&boss_of(w).expect("boss alive"))
    }

    /// The boss arena's centre.
    fn arena_centre(w: &World) -> Vec2 {
        w.level.fight_rect_around(w.player.pos).center_world()
    }

    fn arena_rect(w: &World) -> TileRect {
        w.level.fight_rect_around(w.player.pos)
    }

    /// Put the boss at `pos` and the rat at `rat`, both usually in the arena.
    fn place(w: &mut World, pos: Vec2, rat: Vec2) {
        with_boss(w, |b| {
            b.pos = pos;
            b.target = rat;
            b.pounce_origin = pos;
            b.facing = (rat - pos).normalized();
        });
        w.player.pos = rat;
        w.player.vel = Vec2::ZERO;
    }

    /// Tick until the state changes or `limit` seconds pass.
    fn drive_until_change(w: &mut World, limit: f32) -> CobraState {
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
    ) -> BossContext<'a> {
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena: arena_rect(w),
            level: &w.level,
            flow: &w.flow,
            rng,
        }
    }

    /// The awkward `BossContext` needs its own rng; a test-only stream is fine
    /// (the boss's real one is seeded from the run).
    fn fresh_rng() -> rand::rngs::StdRng {
        use rand::SeedableRng;
        rand::rngs::StdRng::seed_from_u64(0xC0B4A)
    }

    // ---- registration and the pool ---------------------------------------

    #[test]
    fn the_lab_pool_fields_both_specimens_by_seed() {
        let pool = BossKind::pool_for_theme(Theme::Lab);
        assert_eq!(pool.len(), 2, "the Lab shares with the cobra");
        assert_eq!(pool[0], BossKind::Mower, "the floor's original boss is first");
        assert!(pool.contains(&BossKind::Cobra));
        assert_eq!(BossKind::for_theme(Theme::Lab), BossKind::Mower);
        let mut seen_mower = false;
        let mut seen_cobra = false;
        for seed in 0..64u64 {
            let a = BossKind::for_theme_seeded(Theme::Lab, seed);
            assert_eq!(a, BossKind::for_theme_seeded(Theme::Lab, seed));
            assert!(pool.contains(&a));
            seen_mower |= a == BossKind::Mower;
            seen_cobra |= a == BossKind::Cobra;
        }
        assert!(seen_mower && seen_cobra, "both specimens must be reachable");
        // The other pooled floors keep their own pools.
        assert_eq!(
            BossKind::pool_for_theme(Theme::Sewer),
            &[BossKind::Cat, BossKind::HorseshoeCrab]
        );
        assert_eq!(
            BossKind::pool_for_theme(Theme::Kitchen),
            &[BossKind::Broom, BossKind::MegaRat]
        );
        for theme in Theme::all() {
            if BossKind::pool_for_theme(theme).len() > 1 {
                continue;
            }
            for seed in 0..16u64 {
                assert_eq!(
                    BossKind::for_theme_seeded(theme, seed),
                    BossKind::for_theme(theme),
                    "{theme:?} must be untouched by the pool"
                );
            }
        }
    }

    #[test]
    fn the_real_schedule_spawns_the_king_cobra_in_the_lab_arena() {
        let mut w = lab_world(7);
        w.elapsed = crate::game::director::BOSS_TIME;
        w.update(DT, Vec2::ZERO, false);
        let boss = boss_of(&mut w).expect("the schedule spawned the boss");
        assert_eq!(boss.kind, BossKind::Cobra);
        let arena = arena_rect(&w);
        assert!(arena.contains_world(boss.pos), "it spawns in its arena");
        let expect = BossKind::Cobra.hp_pool(w.level_index());
        assert!(
            (boss.max_hp - expect).abs() < 1.0,
            "pool {} vs {expect}",
            boss.max_hp
        );
        // The HUD accessors answer for it, and the plate names it.
        let (hp, max_hp) = w.boss_hp().expect("HUD accessor");
        assert!((hp - boss.hp).abs() < 1e-3 && (max_hp - boss.max_hp).abs() < 1e-3);
        assert!(w.boss_offset().is_some());
        let (name, tag) = crate::render::hud::boss_plate(&w);
        assert_eq!(name, crate::lore::COBRA_NAME);
        assert_eq!(tag, crate::lore::COBRA_CLASS);
        for _ in 0..240 {
            w.update(DT, Vec2::ZERO, false);
        }
        assert!(boss_of(&mut w).is_some(), "the fight is still on");
    }

    #[test]
    fn phases_follow_the_art() {
        assert_eq!(BossKind::Cobra.phase_for(1.0), 1);
        assert_eq!(BossKind::Cobra.phase_for(art::PHASE2_HP_FRAC), 2);
        assert_eq!(BossKind::Cobra.phase_for(0.5), 2);
        assert_eq!(BossKind::Cobra.phase_for(art::PHASE3_HP_FRAC), 3);
        assert_eq!(BossKind::Cobra.phase_for(0.0), 3);
        assert_eq!(
            BossKind::Cobra.initial_state(),
            BossState::Cobra(CobraState::Rear)
        );
        assert_eq!(BossKind::Cobra.contact_damage(), art::CONTACT_DAMAGE);
        assert_eq!(BossKind::Cobra.death_state(), BossState::Cobra(CobraState::Death));
        // The three strike circles are grounded, and the first is the head (or
        // the exposed hood).
        let mut w = lab_world(11);
        spawn_cobra(&mut w);
        let boss = boss_of(&mut w).expect("alive");
        let shapes = boss.hit_shapes();
        assert_eq!(shapes.iter().count(), 3);
        assert!(shapes.iter().all(|s| s.grounded()));
        assert_eq!(boss.target_points().len(), 3);
    }

    // ---- the hood flare and the strike -----------------------------------

    #[test]
    fn the_strike_lands_after_its_tell_and_only_once() {
        let mut w = lab_world(11);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(
            &mut w,
            centre - Vec2::new(60.0, 0.0),
            centre + Vec2::new(30.0, 0.0),
        );
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp0 = w.player.hp;
        set_state(&mut w, CobraState::HoodTell, art::HOOD_TELL[0]);

        // The tell: nothing hurts before the line is live.
        let mut t = 0.0;
        while t < art::HOOD_TELL[0] * art::HOOD_LOCK_PROGRESS {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0, "the strike line hurt before it was live");
        }
        // The committed line is on the rat, and the app sees a telegraph.
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.target - w.player.pos).len() < 8.0,
            "the bite point should be on the rat ({:?} vs {:?})",
            boss.target,
            w.player.pos
        );
        let fangs = art::fangs_pos(&boss.cobra_view());
        let line = boss
            .hazards
            .iter()
            .find(|h| h.damage >= art::STRIKE_DAMAGE)
            .expect("the marked line is a hazard");
        assert_eq!(line.damage, art::STRIKE_DAMAGE);
        assert!(
            line.shape.contains(w.player.pos, PLAYER_RADIUS),
            "the rat is standing on the marked line"
        );
        assert!(
            (line.shape.center() - (fangs + boss.target) * 0.5).len() < 6.0,
            "the line runs from the fangs to the bite point"
        );

        // Exactly one hit, the art's number, and then the fangs are in the
        // floor.
        let mut hits = 0;
        for _ in 0..(2.0 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!(
                    (before - w.player.hp - art::STRIKE_DAMAGE).abs() < 0.6,
                    "the strike hit for {} not {}",
                    before - w.player.hp,
                    art::STRIKE_DAMAGE
                );
            }
            if state_of(&mut w) == CobraState::FangsStuck {
                break;
            }
        }
        assert_eq!(hits, 1, "a strike is a single hit");
        assert_eq!(state_of(&mut w), CobraState::FangsStuck);
    }

    #[test]
    fn a_dodged_strike_sticks_the_fangs_and_opens_the_hood() {
        let mut w = lab_world(12);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(60.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, CobraState::HoodTell, art::HOOD_TELL[0]);
        // Commit the line, then step out of it (north).
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").flags & F_LOCK == 0 && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        w.player.pos = centre + Vec2::new(60.0, 44.0);
        let hp = w.player.hp;
        let mut t = 0.0;
        while state_of(&mut w) != CobraState::FangsStuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), CobraState::FangsStuck);
        assert_eq!(w.player.hp, hp, "the rat stepped off the line");
        let boss = boss_of(&mut w).expect("alive");
        // The punish window: the art's multiplier, no body contact, and the
        // hood's spectacle mark as the fight's one weak point.
        assert!((boss.damage_mult - art::STUCK_DAMAGE_MULT).abs() < 1e-6);
        assert!(boss.contact_damage <= 0.0, "a prone head cannot bump");
        assert!((boss.duration - art::STUCK_DURATION[0]).abs() < 1e-3);
        let view = boss.cobra_view();
        let (hood, radius) = art::weak_point(&view).expect("the hood is exposed");
        assert!(radius >= art::HOOD_WEAK_RADIUS - 1e-3);
        assert!(boss.hit_shapes().circle_hits_grounded(hood, 2.0));
        let points = boss.target_points();
        assert!((points.first().expect("a target point") - hood).len() < 0.5);
        // The window lasts its beat, then it is back to work.
        let mut t = 0.0;
        while state_of(&mut w) == CobraState::FangsStuck && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert!((t - art::STUCK_DURATION[0]).abs() < 0.12, "stuck {t}");
        assert_eq!(state_of(&mut w), CobraState::Rear);
    }

    #[test]
    fn a_strike_into_a_wall_stays_stuck_longer() {
        let mut w = lab_world(13);
        spawn_cobra(&mut w);
        let arena = arena_rect(&w);
        let centre = arena_centre(&w);
        // A solid column east of the boss: the fangs go into it.
        let col = arena.x + arena.w - 4;
        w.level.tiles.fill_cells(
            col,
            arena.y + 2,
            1,
            arena.h - 4,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        let row = centre.y;
        let face_x = (col as f32) * TILE_F;
        // The rat has its back to the wall and the head a strike's length in
        // front of it: the fangs go through the mark and into the column. (A
        // rat *behind* the wall is out of sight and the strike is refused.)
        place(
            &mut w,
            Vec2::new(face_x - 30.0, row),
            Vec2::new(face_x - 7.0, row),
        );
        set_state(&mut w, CobraState::HoodTell, art::HOOD_TELL[0]);
        let mut t = 0.0;
        while state_of(&mut w) != CobraState::FangsStuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), CobraState::FangsStuck);
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.duration - art::STUCK_WALLED).abs() < 1e-3,
            "walled window {} should be {}",
            boss.duration,
            art::STUCK_WALLED
        );
    }

    #[test]
    fn phase_two_strikes_twice() {
        let mut w = lab_world(14);
        spawn_cobra(&mut w);
        with_boss(&mut w, |b| {
            b.phase = 2;
            b.scratch[S_CHAIN] = 1.0;
        });
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(60.0, 0.0));
        set_state(&mut w, CobraState::FangsStuck, 0.12);
        with_boss(&mut w, |b| b.scratch[S_CHAIN] = 1.0);
        assert_eq!(drive_until_change(&mut w, 1.0), CobraState::ReStrikeTell);
        // The second tell is shorter, and it lands in a second strike.
        let boss = boss_of(&mut w).expect("alive");
        assert!((boss.duration - art::RESTRIKE_TELL[1]).abs() < 1e-3);
        let next = drive_until_change(&mut w, 1.5);
        assert_eq!(next, CobraState::Strike, "the second strike comes down");
        // ...and its own fangs stick, after which the pair is spent.
        let next = drive_until_change(&mut w, 1.0);
        assert_eq!(next, CobraState::FangsStuck);
        let next = drive_until_change(&mut w, 2.0);
        assert_eq!(next, CobraState::Rear, "a pair is two strikes, not three");
    }

    #[test]
    fn the_sway_locks_late_and_then_strikes() {
        let mut w = lab_world(15);
        spawn_cobra(&mut w);
        with_boss(&mut w, |b| b.phase = 2);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(56.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, CobraState::SwayTell, art::SWAY_TELL[1]);
        // Before the late lock the aim is still swinging: no hazard exists.
        let mut t = 0.0;
        // (Two ticks short of the lock itself: that tick commits the line.)
        while t < art::SWAY_TELL[1] * art::SWAY_LOCK_PROGRESS - 2.0 * DT {
            tick(&mut w);
            t += DT;
            let boss = boss_of(&mut w).expect("alive");
            assert!(
                !boss.hazards.iter().any(|h| h.damage >= art::STRIKE_DAMAGE),
                "the sway must not commit a line before the last beat"
            );
        }
        let hp = w.player.hp;
        let mut t = 0.0;
        let mut hits = 0;
        while t < 2.0 && state_of(&mut w) != CobraState::FangsStuck {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
            }
            t += DT;
        }
        assert_eq!(hits, 1, "the sway's strike lands once (hp {hp} -> {})", w.player.hp);
        assert_eq!(state_of(&mut w), CobraState::FangsStuck);
    }

    // ---- venom ------------------------------------------------------------

    #[test]
    fn the_venom_leaves_a_slowing_puddle_and_a_drip() {
        let mut w = lab_world(16);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(50.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        w.player.iframes = 0.0;
        set_state(&mut w, CobraState::VenomTell, art::VENOM_TELL[0]);
        let mut t = 0.0;
        while t < art::VENOM_TELL[0] * art::VENOM_LOCK_PROGRESS + DT {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(
            boss.hazards.len(),
            art::VENOM_MARKS * 3,
            "three circles, three effects each"
        );
        let puddles = boss.hazards.iter().filter(|h| h.slow > 0.0).count();
        let splashes = boss
            .hazards
            .iter()
            .filter(|h| (h.damage - art::VENOM_SPLASH).abs() < 1e-3)
            .count();
        let drips = boss
            .hazards
            .iter()
            .filter(|h| (h.damage - art::VENOM_DRIP).abs() < 1e-3)
            .count();
        assert_eq!(puddles, art::VENOM_MARKS, "every mark leaves a puddle");
        assert_eq!(splashes, art::VENOM_MARKS);
        assert_eq!(drips, art::VENOM_MARKS);
        assert!((boss.hazards.iter().find(|h| h.slow > 0.0).unwrap().slow - art::VENOM_SLOW).abs() < 1e-6);
        assert!(boss
            .hazards
            .iter()
            .filter(|h| (h.damage - art::VENOM_DRIP).abs() < 1e-3)
            .all(|h| h.warn >= art::VENOM_DRIP_DELAY));
        // The rat stands in the first mark: one splash, then the drip, and the
        // puddle is a walk-speed tax it can wade out of.
        let hp = w.player.hp;
        let mut hits = 0;
        for _ in 0..(3.0 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
            }
            w.player.pos = centre + Vec2::new(50.0, 0.0);
            w.player.iframes = (w.player.iframes - DT).max(0.0);
            // Hold it between attacks: this test counts one volley's hits, not
            // whatever it would pick next against a rat that never moves.
            with_boss(&mut w, |b| {
                if cobrastate(b) == CobraState::Rear {
                    b.elapsed = 0.0;
                }
            });
        }
        assert!(hits >= 1, "the splash lands (hp {hp} -> {})", w.player.hp);
        assert!(hits <= 2, "the puddle bites once and drips once: {hits}");
        // A puddle of that age is drawn from the boss's own hazards.
        assert!(art::pool_count(&boss_of(&mut w).expect("alive").cobra_view()) >= 1);
    }

    // ---- the tail whip ----------------------------------------------------

    #[test]
    fn the_tail_whip_spares_the_floor_under_the_jaw() {
        let mut w = lab_world(17);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(34.0, 0.0));
        set_state(&mut w, CobraState::TailTell, art::TAIL_TELL[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.5 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        let whip = boss.hazards.last().expect("the arc is a hazard");
        assert_eq!(whip.damage, art::TAIL_DAMAGE);
        // The geometry: the whip takes the floor the head is looking at, and
        // the jaw's own dead zone is safe.
        let ahead = boss.pos + boss.facing * 30.0;
        let under = boss.pos + boss.facing * (art::TAIL_INNER * 0.6);
        let behind = boss.pos - boss.facing * 30.0;
        assert!(whip.shape.contains(ahead, PLAYER_RADIUS), "the arc takes the front");
        assert!(
            !whip.shape.contains(under, PLAYER_RADIUS),
            "inside TAIL_INNER is the safe pocket"
        );
        assert!(!whip.shape.contains(behind, PLAYER_RADIUS), "and so is behind it");
        // A rat standing in it takes one hit of the art's number.
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        w.player.pos = ahead;
        w.player.vel = Vec2::ZERO;
        let hp = w.player.hp;
        let mut hits = 0;
        for _ in 0..(1.0 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            w.player.pos = ahead;
            w.player.vel = Vec2::ZERO;
            w.player.iframes = (w.player.iframes - DT).max(0.0);
            if w.player.hp < before - 1e-3 {
                hits += 1;
            }
            if state_of(&mut w) == CobraState::TailRecover {
                break;
            }
        }
        assert_eq!(hits, 1, "the whip is one pass (hp {hp} -> {})", w.player.hp);
        assert_eq!(state_of(&mut w), CobraState::TailRecover);
        let _ = centre;
    }

    // ---- the constriction -------------------------------------------------

    #[test]
    fn the_constriction_marks_one_gap_and_closes_on_the_clock() {
        let mut w = lab_world(18);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre - Vec2::new(30.0, 0.0), centre);
        set_state(&mut w, CobraState::ConstrictTell, art::COIL_TELL[0]);
        // The loop is on the floor from the first frame, and it has one gap.
        tick(&mut w);
        let view = boss_of(&mut w).expect("alive").cobra_view();
        let mut pts = [Vec2::ZERO; art::COIL_MAX];
        let n = art::blocking_chain(&view, &mut pts);
        assert!(n > 8, "the body is laid as a wall");
        let _r = art::coil_radius(&view);
        let gap = art::coil_gap_angle(&view);
        let half = art::coil_gap_half(&view);
        for q in pts.iter().take(n) {
            let a = (*q - view.target).angle();
            let mut rel = (a - gap).abs();
            if rel > std::f32::consts::PI {
                rel = std::f32::consts::TAU - rel;
            }
            assert!(rel >= half - 0.02, "the gap is the only opening");
        }
        // The closing clock is queued as the rat is shown the ring.
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.5 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        let clock = boss
            .hazards
            .iter()
            .find(|h| (h.damage - art::CONSTRICT_DAMAGE).abs() < 1e-3)
            .expect("the squeeze is a hazard");
        let pocket = art::COIL_END_R - art::COIL_BODY_HALF;
        match clock.shape {
            HazardShape::Circle { center, radius } => {
                assert!((center - boss.target).len() < 1e-3, "it closes on the anchor");
                assert!((radius - pocket).abs() < 1e-3);
            }
            other => panic!("the squeeze should be a circle, got {other:?}"),
        }
        assert!(clock.warn >= art::COIL_TIGHTEN[0] * 0.5);
        // The clock tightens: the radius only shrinks, and the doorway stays
        // wide enough to walk through at every radius.
        let mut last = art::coil_radius(&view);
        let mut t = 0.0;
        let span = art::COIL_TELL[0] + art::COIL_TIGHTEN[0] + 0.3;
        while t < span {
            tick(&mut w);
            t += DT;
            let view = boss_of(&mut w).expect("alive").cobra_view();
            let r = art::coil_radius(&view);
            if r > 0.0 {
                assert!(r <= last + 1e-3, "the loop must not loosen off the clock");
                let half = art::coil_gap_half(&view);
                assert!(
                    2.0 * r * crate::detmath::sin(half) >= art::COIL_GAP_ARC * 0.85,
                    "the doorway must stay walkable at r={r}"
                );
                last = r;
            }
            if state_of(&mut w) == CobraState::Loosen {
                break;
            }
        }
        assert_eq!(state_of(&mut w), CobraState::Loosen);
    }

    #[test]
    fn a_rat_outside_the_pocket_is_not_squeezed() {
        let mut w = lab_world(19);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        // The rat is inside the loop but outside the pocket it closes to, on
        // the far side of the doorway: the squeeze misses it.
        place(&mut w, centre - Vec2::new(30.0, 0.0), centre);
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, CobraState::ConstrictTell, art::COIL_TELL[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.5 {
            tick(&mut w);
            t += DT;
        }
        let anchor = boss_of(&mut w).expect("alive").target;
        // Stand in the doorway, COIL_END_R outside the anchor.
        let view = boss_of(&mut w).expect("alive").cobra_view();
        let gap = art::coil_gap_angle(&view);
        let out = anchor + Vec2::from_angle(gap) * (art::COIL_END_R + 6.0);
        let hp = w.player.hp;
        for _ in 0..(art::COIL_TIGHTEN[0] / DT) as usize {
            w.player.pos = out;
            w.player.vel = Vec2::ZERO;
            tick(&mut w);
            if state_of(&mut w) == CobraState::Loosen {
                break;
            }
        }
        assert_eq!(w.player.hp, hp, "the doorway is a real way out");
        // ...and the pocket itself is a hit.
        let mut w = lab_world(19);
        spawn_cobra(&mut w);
        place(&mut w, centre - Vec2::new(30.0, 0.0), centre);
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, CobraState::ConstrictTell, art::COIL_TELL[0]);
        let hp = w.player.hp;
        let mut hits = 0;
        for _ in 0..((art::COIL_TELL[0] + art::COIL_TIGHTEN[0]) / DT + 10.0) as usize {
            let before = w.player.hp;
            w.player.pos = centre;
            w.player.vel = Vec2::ZERO;
            tick(&mut w);
            w.player.iframes = (w.player.iframes - DT).max(0.0);
            if w.player.hp < before - 1e-3 {
                hits += 1;
            }
        }
        assert!(hits >= 1, "the loop squeezes a rat that stays inside (hp {hp})");
    }

    #[test]
    fn the_coil_blocks_a_walking_rat_and_a_dash_passes_over() {
        let mut w = lab_world(20);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre);
        set_state(&mut w, CobraState::Constrict, art::COIL_TIGHTEN[0]);
        // Let the loop tighten a little, then stand ON it, away from the gap.
        for _ in 0..20 {
            tick(&mut w);
        }
        let view = boss_of(&mut w).expect("alive").cobra_view();
        let anchor = view.target;
        let r = art::coil_radius(&view);
        let on_ring = anchor + Vec2::from_angle(art::coil_gap_angle(&view) + std::f32::consts::PI) * r;
        w.player.pos = on_ring;
        w.player.dash_active = 0.0;
        tick_coil(&mut w);
        let pushed = w.player.pos;
        assert!(
            (pushed.dist(anchor) - r).abs() >= art::COIL_BODY_HALF,
            "a walking rat is put off the body ({:?} at r={r})",
            pushed.dist(anchor)
        );
        // A dashing rat passes over it.
        w.player.pos = on_ring;
        w.player.dash_active = 0.2;
        tick_coil(&mut w);
        assert!(
            w.player.pos.dist(on_ring) < 0.01,
            "a dash goes over the coil"
        );
        w.player.dash_active = 0.0;
    }

    // ---- the spiral -------------------------------------------------------

    #[test]
    fn the_third_phase_lays_a_lane_and_ends_spent() {
        let mut w = lab_world(21);
        spawn_cobra(&mut w);
        with_boss(&mut w, |b| b.phase = 3);
        let centre = arena_centre(&w);
        place(&mut w, centre - Vec2::new(40.0, 0.0), centre);
        let mut rng = fresh_rng();
        let mut boss = boss_of(&mut w).expect("alive");
        let committed = {
            let mut ctx = boss_context(&w, &mut rng);
            commit_spiral(&mut ctx, &mut boss)
        };
        with_boss(&mut w, |b| *b = boss.clone());
        assert!(committed, "the lane fits the arena and the rat is inside it");
        assert_eq!(state_of(&mut w), CobraState::SpiralCoil);
        // The head comes down the lane, then the coils drop and it is spent.
        assert_eq!(drive_until_change(&mut w, 2.0), CobraState::SpiralChase);
        // Once the tell has drawn it on, the laid lane blocks.
        let view = boss_of(&mut w).expect("alive").cobra_view();
        let mut pts = [Vec2::ZERO; art::COIL_MAX];
        assert!(art::spiral_chain(&view, &mut pts) > 8);
        assert!(art::blocking_chain(&view, &mut pts) > 8);
        let before = boss_of(&mut w).expect("alive").pos;
        for _ in 0..30 {
            tick(&mut w);
        }
        let after = boss_of(&mut w).expect("alive");
        assert!(
            after.pos.dist(centre) < before.dist(centre) + 4.0,
            "the head chases the rat it can see"
        );
        assert_eq!(drive_until_change(&mut w, 4.0), CobraState::SpiralEnd);
        let boss = boss_of(&mut w).expect("alive");
        assert!((boss.damage_mult - art::SPENT_DAMAGE_MULT).abs() < 1e-6);
        assert!(boss.contact_damage <= 0.0);
        assert!(art::weak_point(&boss.cobra_view()).is_some());
        assert_eq!(drive_until_change(&mut w, 1.5), CobraState::Rear);
    }

    // ---- the death path ---------------------------------------------------

    #[test]
    fn the_coils_stop_with_the_boss() {
        let mut w = lab_world(22);
        spawn_cobra(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre - Vec2::new(30.0, 0.0), centre);
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, CobraState::ConstrictTell, art::COIL_TELL[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.5 {
            tick(&mut w);
            t += DT;
        }
        assert!(!boss_of(&mut w).expect("alive").hazards.is_empty());
        // Kill it mid-fight: the death beat clears every hazard it owned.
        with_boss(&mut w, |b| b.hp = 0.0);
        let hp = w.player.hp;
        tick(&mut w);
        let boss = boss_of(&mut w).expect("the death beat is still playing");
        assert!(boss.dead);
        assert!(
            boss.hazards.is_empty(),
            "its telegraphs stop with it ({} left)",
            boss.hazards.len()
        );
        // (A few ticks over: the beat ends on the tick after its clock.)
        for _ in 0..(art::DEATH_DURATION / DT) as usize + 12 {
            w.player.pos = centre;
            tick(&mut w);
            if boss_of(&mut w).is_none() {
                break;
            }
        }
        assert!(boss_of(&mut w).is_none(), "the death beat ended");
        assert_eq!(w.player.hp, hp, "a dead cobra hit the rat");
        assert!(
            art::pool_count(&Boss::new(
                BossKind::Cobra,
                centre,
                100.0,
                arena_rect(&w),
                centre,
                false
            )
            .cobra_view())
                == 0,
            "a fresh boss has no puddles"
        );
    }

}
