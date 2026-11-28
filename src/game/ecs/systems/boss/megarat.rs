//! Kitchen's second boss — **SUBJECT R-01**, the Megarat.
//!
//! The art module [`crate::game::art_boss_megarat`] is the gameplay spec: its
//! state enum, every per-state duration, damage number, hit-shape helper and
//! the view struct are binding. This file drives the transitions, picks the
//! bite points and lanes, and queues the art's telegraphed attacks as shared
//! [`BossHazard`]s.
//!
//! ## The fight
//!
//! R-01 is the rat's dark mirror: the facility's first success, four hundred
//! pounds of it, collared and tagged. It fights with the player's own kit —
//! Gnaw, Tail Thrash, Sonic Squeak, the dash — every move bigger, slower and
//! telegraphed, and every one of them hands the rat a window.
//!
//! | state | duration | telegraph → active | damage |
//! |-------|----------|--------------------|--------|
//! | `Groom` | 1.10 / 0.95 / 0.80 s | — | contact 8 |
//! | `GnawWindup` | 0.70 / 0.62 / 0.55 s | teeth line freezes at 0.62 | — |
//! | `GnawLunge` | 0.26 s | line live 0.16 s | 22 |
//! | `GnawStuck` | 1.10 / 1.00 / 0.90 s (1.55 walled) | — | **belly ×1.35** |
//! | `ThrashWindup` | 0.85 / 0.75 / 0.66 s | the huge arc, split in two | — |
//! | `ThrashPass1` | 0.40 s | the rat's own half | 18 |
//! | `ThrashGap` | 0.34 s | **nothing is live** | — |
//! | `ThrashPass2` | 0.40 s | the other half | 18 |
//! | `ThrashRecover` | 0.90 / 0.80 / 0.72 s | — | **tail base ×1.25** |
//! | `SqueakWindup` | 0.80 / 0.72 / 0.62 s | the ring + the safe wedge | — |
//! | `SqueakShout` | 0.22 s | the band 26..44 px | 16 |
//! | `DashWindup` | 0.62 / 0.55 / 0.48 s | the lane, freezes at 0.58 | — |
//! | `DashPass` | lane length / 430-510 px/s | the lane | 20 |
//! | `DashSkid` | 0.32 s | — | contact 8 |
//! | `Call` | 1.20 s | **harmless pose change**, summons 3 rats | none |
//! | `Death` | 2.60 s | — | none |
//!
//! ## Suppression (every attack, every time)
//!
//! The owner's grammar: no move is committed when the rat could not read or
//! leave it, or when R-01 could not execute it where it stands.
//!
//! * the **gnaw** needs line of sight on the rat, a bite between
//!   [`art::GNAW_MIN`] and [`art::GNAW_REACH`], and a clear step out of the
//!   line on at least one side;
//! * the **thrash** needs a clear step out of the arc on one side (the rat
//!   cannot be herded against a wall by a 156 degree sweep);
//! * the **squeak** needs the safe wedge behind the boss to be open floor — it
//!   will not scream a rat into a corner it cannot read — and the rat in range;
//! * the **dash** needs its cooldown spent, a lane at least
//!   [`art::DASH_MIN_LEN`] long that the whole body fits down, and a clear step
//!   off the lane.
//!
//! A refused move falls through the rotation to a cheaper one and finally back
//! to `Groom`, which re-seeks; a phase change, a hunt or a dead rat never leaves
//! it standing on an unreadable move.
//!
//! ## Chaining (phase 2+)
//!
//! From the second phase every grooming is followed by **two** moves: an attack
//! that ends with the chain pending goes straight into the next one instead of
//! grooming. The second move always finishes into a `Groom`, so the rat always
//! gets the beat it needs to reposition (and to punish a recovery).
//!
//! ## Determinism and allocation
//!
//! One `f32` draw per open-ended attack decision from `ctx.rng` (the boss's own
//! stream; the opening three decisions are scripted so the fight teaches its
//! own counterplay first), and none from anywhere else. The summon rides on the
//! `Boss` component (`call_count`/`call_at`/`call_kind`) and is drained by
//! [`boss_summon_system`] at the head of the next tick, outside the
//! `EnemyIndex` window. Nothing in this file allocates: hazards go into the
//! boss's pre-reserved ring and the line/lane probes walk the stack.
//!
//! ## Per-controller memory
//!
//! The art view reads `pos`/`facing`/`state`/`progress`/`anim`/`hit_flash`/
//! `hp_frac`/`target`/`pounce_origin`/`phase` plus two scratch slots, so the
//! rest lives in [`Boss::flags`] and [`Boss::scratch`]:
//!
//! * `flags` — bit 0 `LOCK` ("the committed line/lane/arc is queued"), bit 1
//!   `CUE`, bit 2 `SUMMON` (the call has already called), bit 3 `DEATH` (the
//!   death beat has cleared its hazards).
//! * `scratch[0]` the attack-rotation index, `scratch[1]` the dash cooldown,
//!   `scratch[2]` the chain budget, `scratch[3]` the decision counter (the
//!   scripted opening), `scratch[4]` `1.0` when the last lunge was walled,
//!   `scratch[5]` the thrash's first-pass side.

use super::*;
use crate::game::art_boss_megarat as art;
use crate::game::art_boss_megarat::MegaRatState;

/// The Megarat's controller (zero-sized; the state lives on the [`Boss`]).
pub struct MegaRatController;

// ============================================================================
// Scratch and flag layout (see the module docs)
// ============================================================================

/// The attack rotation index.
const S_ROT: usize = 0;
/// Dash cooldown, seconds remaining.
pub(crate) const S_DASH_CD: usize = 1;
/// Chain budget: `1.0` = the next attack is the second of a pair.
const S_CHAIN: usize = 2;
/// Attack decisions taken (the scripted opening).
const S_DECISIONS: usize = 3;
/// `1.0` when the last lunge ended against a wall or a cabinet.
pub(crate) const S_WALLED: usize = 4;
/// `+1`/`-1`: which half of the thrash arc is swept first.
pub(crate) const S_SIDE: usize = 5;

/// The committed line / arc / lane has been queued.
const F_LOCK: u32 = 1 << 0;
/// This state has already played its one cue.
const F_CUE: u32 = 1 << 1;
/// The `Call` has already called the rats.
const F_SUMMON: u32 = 1 << 2;
/// The death beat has cleared the living boss's telegraphs.
const F_DEATH: u32 = 1 << 3;

/// Progress through `ThrashWindup` where the arc and its two passes commit.
const THRASH_LOCK_PROGRESS: f32 = 0.55;

/// Progress through `SqueakWindup` where the ring commits.
const SQUEAK_LOCK_PROGRESS: f32 = 0.55;

/// How far past its own body the squeak reaches before a rat is "out of range".
const SQUEAK_RANGE_SLACK: f32 = 44.0;

// ============================================================================
// Controller
// ============================================================================

impl BossController for MegaRatController {
    fn kind(&self) -> BossKind {
        BossKind::MegaRat
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = megastate(boss);
        // It follows the rat out of the arena on reachable floor (the flow
        // field routes it; `BossContext::seek` prefers the direct bearing).
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);
        boss.scratch[S_DASH_CD] = (boss.scratch[S_DASH_CD] - ctx.dt).max(0.0);

        // A phase threshold is the art's set piece: the harmless call. Anything
        // it was winding up is cancelled with the move it belonged to, so a
        // telegraph never fires out of a pose the rat can no longer see.
        if out.phase_changed && megastate(boss) != MegaRatState::Death {
            boss.hazards.clear();
            boss.scratch[S_CHAIN] = 0.0;
            enter(boss, MegaRatState::Call, art::CALL_DURATION);
        }

        match megastate(boss) {
            MegaRatState::Groom => tick_groom(ctx, boss),
            MegaRatState::GnawWindup => tick_gnaw_windup(ctx, boss),
            MegaRatState::GnawLunge => tick_gnaw_lunge(ctx, boss),
            MegaRatState::GnawStuck => tick_stuck(ctx, boss),
            MegaRatState::ThrashWindup => tick_thrash_windup(ctx, boss),
            MegaRatState::ThrashPass1
            | MegaRatState::ThrashGap
            | MegaRatState::ThrashPass2 => tick_thrash_passes(ctx, boss),
            MegaRatState::ThrashRecover => tick_thrash_recover(ctx, boss),
            MegaRatState::SqueakWindup => tick_squeak_windup(ctx, boss),
            MegaRatState::SqueakShout => tick_squeak_shout(ctx, boss),
            MegaRatState::DashWindup => tick_dash_windup(ctx, boss),
            MegaRatState::DashPass => tick_dash_pass(ctx, boss),
            MegaRatState::DashSkid => tick_dash_skid(ctx, boss),
            MegaRatState::Call => tick_call(ctx, boss, out),
            // The death beat is the framework's, and `death_tick` owns its cue.
            MegaRatState::Death => {}
        }

        // The art owns the multiplier and the contact rule; apply them after
        // the transitions so a punish window is live on the tick it opens.
        let now = megastate(boss);
        boss.damage_mult = art::incoming_damage_mult(now);
        if !art::has_body_contact(now) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        let after = megastate(boss);
        if after != before {
            if let Some(cue) = state_cue(after) {
                out.cue(cue);
            }
        }
    }

    /// The death beat: it goes over, the collar rings on the tile.
    ///
    /// Its telegraphs stop with it: the first tick of the beat clears every
    /// hazard the living animal had queued, so a teeth line or a live arc
    /// cannot land on a rat that has already won. Nothing new is dealt here.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH == 0 {
            boss.flags |= F_DEATH;
            boss.hazards.clear();
            out.cue(BossCue::MegaRatDeath);
        }
    }
}

/// The sound/effect cue for entering a state (within-state beats — the summon
/// and the arc's passes — are cued where they happen).
#[inline]
fn state_cue(state: MegaRatState) -> Option<BossCue> {
    match state {
        MegaRatState::Groom => Some(BossCue::MegaRatChitter),
        MegaRatState::GnawWindup => Some(BossCue::MegaRatGnaw),
        MegaRatState::GnawStuck => Some(BossCue::MegaRatStuck),
        MegaRatState::ThrashWindup => Some(BossCue::MegaRatThrash),
        MegaRatState::ThrashPass1 | MegaRatState::ThrashPass2 => {
            Some(BossCue::MegaRatThrashSweep)
        }
        MegaRatState::SqueakShout => Some(BossCue::MegaRatSqueak),
        MegaRatState::DashPass => Some(BossCue::MegaRatDash),
        MegaRatState::Call => Some(BossCue::MegaRatCall),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

/// `Groom`: it walks the rat down to its band, cleaning its own ear and
/// sniffing the air between steps — the neutral beat that gives it a
/// personality instead of an idle.
fn tick_groom(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    let dir = if dist < art::GROOM_BAND[0] {
        -boss.facing
    } else if dist > art::GROOM_BAND[1] {
        ctx.seek(boss)
    } else {
        // Hold the band by pacing around the rat: a caged animal never stands
        // still, and a circling boss keeps the arc's safe strip moving.
        boss.facing.perp() * pace_sign(boss)
    };
    let speed = art::GROOM_SPEED[slot(boss)]
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

fn tick_gnaw_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The line follows the rat until the art's lock, then freezes so it can be
    // stepped out of (the cat's landing-ring rule).
    if boss.progress() < art::GNAW_LOCK_PROGRESS {
        if let Some(strike) = gnaw_strike(ctx, boss) {
            boss.target = strike;
            let to = strike - boss.pos;
            if to.len_sq() > 1e-6 {
                boss.facing = to.normalized();
            }
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        // Commit the strike the rat was shown a moment ago.
        let Some(strike) = gnaw_strike(ctx, boss) else {
            // The rat left the line's reach (walked behind a range or a
            // cabinet): the lunge is refused and R-01 repositions rather than
            // biting at nothing.
            enter_groom(boss);
            return;
        };
        boss.flags |= F_LOCK;
        boss.target = strike;
        let to = strike - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        boss.pounce_origin = boss.pos;
        // Cornered rule: a cabinet or a wall in front of the muzzle means the
        // teeth are going into it, and pulling them back out takes longer.
        let ahead = boss.pos + boss.facing * (art::GNAW_WALL_PROBE + art::BODY_RADIUS);
        let walled = !ctx.footprint_fits(ahead, boss.kind.body_radius());
        boss.scratch[S_WALLED] = if walled { 1.0 } else { 0.0 };

        let muzzle = art::muzzle(&boss.megarat_view());
        let warn = (boss.duration - boss.elapsed).max(0.02);
        let line = BossHazard::new(
            HazardShape::Corridor {
                from: muzzle,
                to: strike,
                half_width: art::GNAW_ACTIVE_HALF_WIDTH,
            },
            warn,
            art::GNAW_ACTIVE,
            art::GNAW_DAMAGE,
        )
        .with_knockback(120.0);
        let _ = boss.push_hazard(line);
    }
    if boss.progress() >= 1.0 {
        enter(boss, MegaRatState::GnawLunge, art::GNAW_LUNGE);
    }
}

fn tick_gnaw_lunge(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Drive the body down the frozen line to the bite point.
    let want = boss.target - boss.facing * (art::HEAD_OFFSET + art::HEAD_RADIUS);
    let to = want - boss.pos;
    let mut blocked = false;
    if to.len() > 2.0 {
        blocked = !move_boss(ctx, boss, to, art::GNAW_LUNGE_SPEED);
    }
    if boss.progress() >= 1.0 || blocked {
        if blocked {
            boss.scratch[S_WALLED] = 1.0;
        }
        let walled = boss.scratch[S_WALLED] > 0.5;
        let duration = if walled {
            art::GNAW_STUCK_WALLED
        } else {
            art::GNAW_STUCK[slot(boss)]
        };
        enter(boss, MegaRatState::GnawStuck, duration);
    }
}

fn tick_stuck(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The teeth are in the floor: no movement, no contact, belly out. (The art
    // rule applies both.)
    let _ = ctx;
    if boss.progress() >= 1.0 {
        after_attack(ctx, boss);
    }
}

fn tick_thrash_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < THRASH_LOCK_PROGRESS {
        // It plants and aims: the facing (and so the arc) tracks the rat until
        // the arc commits.
        let to = ctx.player_pos - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        boss.scratch[S_SIDE] = rat_side(ctx, boss);
        return;
    }
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        // The rat's own half goes first, so standing where you are is wrong and
        // the gap is the answer.
        let side = rat_side(ctx, boss);
        boss.scratch[S_SIDE] = side;
        let facing = boss.facing;
        let half = art::THRASH_HALF_ARC * 0.5;
        let warn = (boss.duration - boss.elapsed).max(0.02);
        // Pass 1 sweeps the rat's half; pass 2 the other, after the gap.
        for (k, side_sign) in [1.0f32, -1.0].into_iter().enumerate() {
            let from_lock = warn
                + if k == 0 {
                    0.0
                } else {
                    art::THRASH_PASS + art::THRASH_GAP
                };
            // `side` is the rat's own half: pass 1 sweeps where the rat is,
            // pass 2 the other half, so the answer is to be in the other half
            // when the tail comes, and to cross back in the gap.
            let dir = facing.rotate(side_sign * side * half);
            let _ = boss.push_hazard(
                BossHazard::new(
                    HazardShape::Arc {
                        center: boss.pos,
                        inner: art::THRASH_INNER,
                        outer: art::THRASH_OUTER,
                        facing: dir,
                        half_angle: half,
                    },
                    from_lock,
                    art::THRASH_PASS,
                    art::THRASH_DAMAGE,
                )
                .with_knockback(art::THRASH_KNOCKBACK)
                .with_enemies(),
            );
        }
    }
    if boss.progress() >= 1.0 {
        enter(boss, MegaRatState::ThrashPass1, art::THRASH_PASS);
    }
}

fn tick_thrash_passes(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let _ = ctx;
    if boss.progress() < 1.0 {
        return;
    }
    match megastate(boss) {
        MegaRatState::ThrashPass1 => enter(boss, MegaRatState::ThrashGap, art::THRASH_GAP),
        MegaRatState::ThrashGap => enter(boss, MegaRatState::ThrashPass2, art::THRASH_PASS),
        _ => enter(
            boss,
            MegaRatState::ThrashRecover,
            art::THRASH_RECOVER[slot(boss)],
        ),
    }
}

fn tick_thrash_recover(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The tail drags; it is off balance and cannot chase.
    if boss.progress() >= 1.0 {
        after_attack(ctx, boss);
    }
}

fn tick_squeak_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < SQUEAK_LOCK_PROGRESS {
        let to = ctx.player_pos - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to.normalized();
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        let warn = (boss.duration - boss.elapsed).max(0.02);
        // The ring is everything except one wedge behind it. `PI - wedge` is
        // exactly "all of the circle but the wedge".
        let ring = BossHazard::new(
            HazardShape::Arc {
                center: boss.pos,
                inner: art::SQUEAK_INNER,
                outer: art::SQUEAK_OUTER,
                facing: boss.facing,
                half_angle: std::f32::consts::PI - art::SQUEAK_WEDGE,
            },
            warn,
            art::SQUEAK_ACTIVE,
            art::SQUEAK_DAMAGE,
        )
        .with_knockback(art::SQUEAK_KNOCKBACK);
        let _ = boss.push_hazard(ring);
    }
    if boss.progress() >= 1.0 {
        enter(boss, MegaRatState::SqueakShout, art::SQUEAK_ACTIVE);
    }
}

fn tick_squeak_shout(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        after_attack(ctx, boss);
    }
}

fn tick_dash_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() < art::DASH_LOCK_PROGRESS {
        // It hovers onto the lane's line with the rat, so the lane it paints
        // is the lane it runs.
        if let Some((_, end)) = dash_plan(ctx, boss) {
            boss.target = end;
            let to = end - boss.pos;
            if to.len_sq() > 1e-6 {
                boss.facing = to.normalized();
            }
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        boss.flags |= F_LOCK;
        let Some((start, end)) = dash_plan(ctx, boss) else {
            enter_groom(boss);
            return;
        };
        boss.pounce_origin = start;
        boss.target = end;
        let dir = (end - start).normalized();
        if dir.len_sq() > 1e-6 {
            boss.facing = dir;
        }
        let len = start.dist(end);
        let pass = (len / art::DASH_SPEED[slot(boss)]).max(0.12);
        let warn = (boss.duration - boss.elapsed).max(0.02);
        let lane = BossHazard::new(
            HazardShape::Corridor {
                from: start,
                to: end,
                half_width: art::DASH_LANE_HALF,
            },
            warn,
            pass,
            art::DASH_DAMAGE,
        )
        .with_knockback(art::DASH_KNOCKBACK);
        let _ = boss.push_hazard(lane);
    }
    if boss.progress() >= 1.0 {
        let len = boss.pounce_origin.dist(boss.target);
        let pass = (len / art::DASH_SPEED[slot(boss)]).max(0.12);
        enter(boss, MegaRatState::DashPass, pass);
    }
}

fn tick_dash_pass(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let dir = (boss.target - boss.pounce_origin).normalized();
    let speed = art::DASH_SPEED[slot(boss)];
    let mut blocked = false;
    if dir.len_sq() > 1e-6 {
        blocked = !move_boss(ctx, boss, dir, speed);
    }
    let past = (boss.pos - boss.pounce_origin).dot(dir) >= (boss.target - boss.pounce_origin).dot(dir);
    if blocked || past || boss.progress() >= 1.0 {
        enter(boss, MegaRatState::DashSkid, art::DASH_SKID);
        // The gauge restarts here: the rat can read the cooldown off the boss.
        boss.scratch[S_DASH_CD] = art::DASH_COOLDOWN;
    }
}

fn tick_dash_skid(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        after_attack(ctx, boss);
    }
}

fn tick_call(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    // The harmless pose change. At the call's beat the walls answer: the
    // request rides on the boss and `boss_summon_system` spawns it at the head
    // of the next tick, outside the `EnemyIndex` window.
    let _ = ctx;
    if boss.progress() >= art::CALL_SUMMON_PROGRESS && boss.flags & F_SUMMON == 0 {
        boss.flags |= F_SUMMON;
        boss.call_count = art::CALL_RATS;
        boss.call_at = boss.pos;
        boss.call_kind = EnemyKind::Skitterer;
        out.cue(BossCue::MegaRatCall);
    }
    if boss.progress() >= 1.0 {
        enter_groom(boss);
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
fn megastate(boss: &Boss) -> MegaRatState {
    match boss.state {
        BossState::MegaRat(s) => s,
        _ => MegaRatState::Groom,
    }
}

/// `+1`/`-1`, alternating per attack decision, for the grooming pace.
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
    let cross = boss.facing.perp().dot(to);
    if cross >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Enter a state, clearing the per-state latches (but not the death bit).
#[inline]
fn enter(boss: &mut Boss, state: MegaRatState, duration: f32) {
    boss.set_state(BossState::MegaRat(state), duration);
    boss.flags &= !(F_LOCK | F_CUE | F_SUMMON);
    boss.damage_mult = art::incoming_damage_mult(state);
}

/// The grooming beat.
#[inline]
fn enter_groom(boss: &mut Boss) {
    enter(boss, MegaRatState::Groom, art::GROOM_DURATION[slot(boss)]);
}

/// An attack has finished: either take the second move of the chain (phase 2+)
/// or groom (and load the chain for the next one).
fn after_attack(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.phase >= 2 && boss.scratch[S_CHAIN] > 0.5 {
        boss.scratch[S_CHAIN] = 0.0;
        next_attack(ctx, boss);
    } else {
        boss.scratch[S_CHAIN] = if boss.phase >= 2 { 1.0 } else { 0.0 };
        enter_groom(boss);
    }
}

// ============================================================================
// Attack selection
// ============================================================================

/// Which attack a decision picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attack {
    Gnaw,
    Thrash,
    Squeak,
    Dash,
}

impl Attack {
    /// The order to fall through when a move is refused: the next entry is the
    /// cheapest alternative, and `Groom` is the last resort.
    #[inline]
    fn fallback(self) -> Option<Attack> {
        match self {
            Attack::Gnaw => Some(Attack::Dash),
            Attack::Dash => Some(Attack::Thrash),
            Attack::Thrash => Some(Attack::Squeak),
            Attack::Squeak => None,
        }
    }
}

/// End of `Groom` (or of the first move of a chain): pick the next beat and
/// start it.
///
/// The opening is scripted — two gnaws and then a thrash — so the rat meets the
/// two attacks whose counterplay (step off the line, then bite the belly; step
/// into the swept half during the gap) is the whole fight before anything else
/// is on the table. Afterwards exactly one draw from the boss's own stream
/// decides the beat, so the draw count per decision is constant and
/// seed-reproducible.
fn next_attack(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let decisions = boss.scratch[S_DECISIONS];
    boss.scratch[S_DECISIONS] = decisions + 1.0;
    boss.scratch[S_ROT] = ((boss.scratch[S_ROT] as u32 + 1) % 4) as f32;

    let want = if decisions < 1.5 {
        Attack::Gnaw
    } else if decisions < 2.5 {
        Attack::Thrash
    } else {
        let roll = ctx.rng.random::<f32>();
        match boss.phase.clamp(1, 3) {
            1 => {
                if roll < 0.45 {
                    Attack::Gnaw
                } else if roll < 0.78 {
                    Attack::Thrash
                } else {
                    Attack::Squeak
                }
            }
            2 => {
                if roll < 0.40 {
                    Attack::Gnaw
                } else if roll < 0.70 {
                    Attack::Thrash
                } else if roll < 0.88 {
                    Attack::Dash
                } else {
                    Attack::Squeak
                }
            }
            _ => {
                if roll < 0.34 {
                    Attack::Gnaw
                } else if roll < 0.62 {
                    Attack::Thrash
                } else if roll < 0.84 {
                    Attack::Dash
                } else {
                    Attack::Squeak
                }
            }
        }
    };

    let mut pick = Some(want);
    while let Some(attack) = pick {
        if commit(ctx, boss, attack) {
            return;
        }
        pick = attack.fallback();
    }
    // Nothing fit where it stands: re-seek through the grooming beat.
    enter_groom(boss);
}

/// Try to start one attack. Returns `true` when the state was entered.
fn commit(ctx: &mut BossContext<'_>, boss: &mut Boss, attack: Attack) -> bool {
    match attack {
        Attack::Gnaw => commit_gnaw(ctx, boss),
        Attack::Thrash => commit_thrash(ctx, boss),
        Attack::Squeak => commit_squeak(ctx, boss),
        Attack::Dash => commit_dash(ctx, boss),
    }
}

/// Start a lunge if the rat is visible, in reach and has somewhere to step.
fn commit_gnaw(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(strike) = gnaw_strike(ctx, boss) else {
        return false;
    };
    let dir = (strike - boss.pos).normalized();
    // The rat has to be able to see the line, and to step off it.
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::GNAW_ESCAPE) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    boss.target = strike;
    if dir.len_sq() > 1e-6 {
        boss.facing = dir;
    }
    boss.pounce_origin = boss.pos;
    enter(boss, MegaRatState::GnawWindup, art::GNAW_WINDUP[slot(boss)]);
    true
}

/// The point the incisors would bite: `GNAW_MIN..=GNAW_REACH` down the line to
/// the rat, stopped by the first solid the muzzle would meet (that is the
/// cabinet the teeth stick in).
fn gnaw_strike(ctx: &BossContext<'_>, boss: &Boss) -> Option<Vec2> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist < art::GNAW_MIN * 0.5 {
        return None;
    }
    let dir = to * (1.0 / dist);
    let want = dist.min(art::GNAW_REACH);
    if want < art::GNAW_MIN {
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
    if reach < art::GNAW_MIN * 0.5 {
        return None;
    }
    Some(boss.pos + dir * reach)
}

/// Is there a clear step off a line with direction `dir` at `at`?
fn escape_side_clear(ctx: &BossContext<'_>, at: Vec2, dir: Vec2, step: f32) -> bool {
    let perp = dir.perp();
    [1.0f32, -1.0].iter().any(|s| {
        let p = at + perp * (*s * step);
        ctx.los_clear(at, p, ctx.player_radius)
            && ctx
                .level
                .tiles
                .passable(crate::game::collision::cell_of(p).0, crate::game::collision::cell_of(p).1, SizeClass::Small)
    })
}

/// Start a thrash if the rat has a way out of the arc.
fn commit_thrash(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    // The rat must be able to leave the arc: a step off the facing's line is
    // the perpendicular answer, and a rat pinned in a corner has none.
    let dir = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        boss.facing
    };
    if to.len() > art::THRASH_OUTER + 60.0 {
        return false;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::THRASH_ESCAPE) {
        return false;
    }
    boss.facing = dir;
    boss.scratch[S_SIDE] = rat_side(ctx, boss);
    enter(boss, MegaRatState::ThrashWindup, art::THRASH_WINDUP[slot(boss)]);
    true
}

/// Start a squeak if the safe wedge behind it is open floor.
fn commit_squeak(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let to = ctx.player_pos - boss.pos;
    // A shout with the rat a screen away is a wasted beat; it falls through to
    // something that closes.
    if to.len() > art::SQUEAK_OUTER + SQUEAK_RANGE_SLACK {
        return false;
    }
    let dir = if to.len_sq() > 1e-6 {
        to.normalized()
    } else {
        boss.facing
    };
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    boss.facing = dir;
    // Never scream a rat into a corner it cannot read: the wedge behind the
    // boss is the only safe floor the ring leaves, so it has to exist.
    let mid = (art::SQUEAK_INNER + art::SQUEAK_OUTER) * 0.5;
    let wedge_at = boss.pos - boss.facing * mid;
    if !ctx.footprint_fits(wedge_at, art::SQUEAK_WEDGE_CLEAR) {
        return false;
    }
    if !ctx.footprint_fits(boss.pos, boss.kind.body_radius()) {
        return false;
    }
    enter(boss, MegaRatState::SqueakWindup, art::SQUEAK_WINDUP[slot(boss)]);
    true
}

/// Start a dash if the gauge is up and a lane fits through the rat.
fn commit_dash(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if boss.scratch[S_DASH_CD] > 0.0 {
        return false;
    }
    if !ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
        return false;
    }
    let Some((start, end)) = dash_plan(ctx, boss) else {
        return false;
    };
    let dir = (end - start).normalized();
    if !escape_side_clear(ctx, ctx.player_pos, dir, art::DASH_ESCAPE) {
        return false;
    }
    boss.pounce_origin = start;
    boss.target = end;
    boss.facing = dir;
    enter(boss, MegaRatState::DashWindup, art::DASH_WINDUP[slot(boss)]);
    true
}

/// Choose the dash lane: a straight run from the body through the rat (plus the
/// overshoot) that the whole body fits down. Returns `(start, end)`.
fn dash_plan(ctx: &BossContext<'_>, boss: &Boss) -> Option<(Vec2, Vec2)> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if dist < 1.0 {
        return None;
    }
    let dir = to * (1.0 / dist);
    let len = (dist + art::DASH_OVERSHOOT)
        .clamp(art::DASH_MIN_LEN, art::DASH_REACH);
    let start = boss.pos;
    let mut len = len;
    // Keep the lane's far end on the encounter floor while both are inside it.
    // The lane is *shortened along its own heading*: clamping the end point
    // per axis (the old rule) bent a diagonal lane away from the rat it was
    // aimed through, so near an arena edge the tell could be painted beside
    // the rat instead of over it.
    if ctx.arena.contains_world(start) && ctx.arena.contains_world(ctx.player_pos) {
        let margin = art::BODY_RADIUS + 4.0;
        let min = Vec2::new(
            ctx.arena.x as f32 * collision::TILE_F + margin,
            ctx.arena.y as f32 * collision::TILE_F + margin,
        );
        let max = Vec2::new(
            ctx.arena.right() as f32 * collision::TILE_F - margin,
            ctx.arena.bottom() as f32 * collision::TILE_F - margin,
        );
        if dir.x > 1e-4 {
            len = len.min((max.x - start.x) / dir.x);
        } else if dir.x < -1e-4 {
            len = len.min((min.x - start.x) / dir.x);
        }
        if dir.y > 1e-4 {
            len = len.min((max.y - start.y) / dir.y);
        } else if dir.y < -1e-4 {
            len = len.min((min.y - start.y) / dir.y);
        }
    }
    // (A body standing inside the margin can make `len` negative: no lane.)
    if len < art::DASH_MIN_LEN {
        return None;
    }
    let end = start + dir * len;
    // The whole flight has to fit the body, or the lane lies.
    let steps = (start.dist(end) / 8.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let p = start + (end - start) * (i as f32 / steps as f32);
        if !ctx.footprint_fits(p, boss.kind.body_radius()) {
            return None;
        }
    }
    Some((start, end))
}

// ============================================================================
// The summon (the head of the next tick, outside the `EnemyIndex` window)
// ============================================================================

/// Spawn the ordinary bodies a boss's `Call` asked for.
///
/// It runs right after [`boss_spawn_system`] and before
/// `enemy_move::grid_build_system`: at the head of the tick, **outside** the
/// `SpawnId`-ordered `EnemyIndex` window that `resources::debug_assert_enemy_index`
/// polices. That is the only place a boss may add an `Enemy` entity — a
/// controller cannot (`BossController::tick` has no `Commands`), and the
/// update/hazard slots are inside the window.
///
/// A boss with no pending call (`Boss::call_count == 0`) draws no rng, spawns
/// nothing and costs one branch per boss per tick, so a fight that never calls
/// is byte-identical to one without this system.
#[allow(clippy::too_many_arguments)]
pub fn boss_summon_system(
    mut commands: Commands,
    level: Res<Level>,
    mut next_spawn_id: ResMut<NextSpawnId>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut fight: ResMut<BossFight>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut bosses: Query<&mut Boss>,
) {
    // The called rats roll their spawn variance (and their arrival vfx) on the
    // boss's own stream, like every other draw a fight makes: a `Call` must not
    // move the run's main rng, or the horde after the phase change would depend
    // on when the rat crossed the threshold.
    for mut boss in bosses.iter_mut() {
        let count = boss.call_count;
        if count == 0 {
            continue;
        }
        boss.call_count = 0;
        if boss.dead {
            continue;
        }
        let centre = boss.call_at;
        let kind = boss.call_kind;
        let mut placed = 0u8;
        for k in 0..count {
            let angle = std::f32::consts::TAU * k as f32 / count.max(1) as f32
                + boss.facing.angle();
            let ring = art::CALL_RING + 8.0 * (k % 2) as f32;
            let want = centre + Vec2::from_angle(angle) * ring;
            let pos = if level.tiles.circle_clear(want, 5.0, SizeClass::Small) {
                want
            } else {
                level
                    .tiles
                    .nearest_open_center(want, SizeClass::Small, 6)
                    .filter(|p| level.tiles.circle_clear(*p, 5.0, SizeClass::Small))
                    .unwrap_or(want)
            };
            if !level.tiles.circle_clear(pos, 5.0, SizeClass::Small) {
                continue;
            }
            let mut enemy = Enemy::spawn_with_mods(
                kind,
                pos,
                BOSS_CALL_HP_MULT,
                crate::game::enemy::EnemyMods::NONE,
                &mut fight.rng.0,
            );
            enemy.id = next_enemy_id.take();
            commands.spawn((enemy, next_spawn_id.take()));
            placed += 1;
            vfx::emit_boss_spawn(
                &mut particles,
                &mut fight.rng.0,
                pos,
                level.palette.accent,
            );
        }
        if placed > 0 {
            sfx.0.push(Sfx::Warn);
        }
    }
}

/// How tough the called rats are relative to the horde's own curve: they are
/// the room's residents, not the run's scaling bodies (the boss's `Call` is a
/// pressure beat, not a second horde).
const BOSS_CALL_HP_MULT: f32 = 1.0;

#[cfg(test)]
mod tests {
    //! Narrow tests for SUBJECT R-01's fight.
    //!
    //! `cargo test boss::megarat` runs these with the framework's suite. Every
    //! attack test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, contact and the death
    //! hand-off are the real ones; the suppression tests call the same
    //! `commit_*` functions the controller calls, with the same `BossContext`
    //! the system builds.

    use super::*;
    use crate::game::art_boss_megarat as art;
    use crate::game::collision::{TileKind, TILE_F};
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    // ---- harness ---------------------------------------------------------

    /// A Kitchen seed whose pool pick is the Megarat (the pool is seed-driven,
    /// so a test that wants *this* fight asks for a seed that fields it).
    fn megaseed(mut seed: u64) -> u64 {
        for _ in 0..4096 {
            if BossKind::for_theme_seeded(Theme::Kitchen, seed) == BossKind::MegaRat {
                return seed;
            }
            seed = seed.wrapping_add(1);
        }
        panic!("no Kitchen seed in range fields the Megarat");
    }

    fn kitchen_world(seed: u64) -> World {
        World::with_tuning(
            Theme::Kitchen, megaseed(seed), crate::tuning::RunTuning::with_base_boss_hazards(BossKind::MegaRat),
        )
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
    fn spawn_megarat(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the boss");
        assert_eq!(
            boss.kind,
            BossKind::MegaRat,
            "the seed should field the Megarat"
        );
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

    fn set_state(w: &mut World, state: MegaRatState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::MegaRat(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
            b.hazards.clear();
        });
    }

    fn state_of(w: &mut World) -> MegaRatState {
        megastate(&boss_of(w).expect("boss alive"))
    }

    /// The boss arena's centre: the Kitchen's encounter floor.
    fn arena_centre(w: &World) -> Vec2 {
        w.level.fight_rect_around(w.player.pos).center_world()
    }

    /// Put the boss at `pos` and the rat at `rat` (both usually in the arena).
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

    /// Tick until the state changes or `limit` seconds pass; returns the state
    /// entered.
    fn drive_until_change(w: &mut World, limit: f32) -> MegaRatState {
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
        _arena: crate::game::level::TileRect,
    ) -> BossContext<'a> {
        let boss_arena = w.level.fight_rect_around(w.player.pos);
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena: boss_arena,
            view: w.viewport,
            level: &w.level,
            flow: &w.flow,
            rng,
        }
    }

    // ---- registration and the pool ---------------------------------------

    #[test]
    fn the_kitchen_pool_fields_both_wardens_by_seed() {
        let pool = BossKind::pool_for_theme(Theme::Kitchen);
        assert_eq!(pool.len(), 2, "the Kitchen shares with the Megarat");
        assert_eq!(pool[0], BossKind::Broom, "the floor's original boss is first");
        assert!(pool.contains(&BossKind::MegaRat));
        // `for_theme` stays the floor's canonical name (the first entry), so
        // every pre-pool caller and test keeps its answer.
        assert_eq!(BossKind::for_theme(Theme::Kitchen), BossKind::Broom);
        // The pick is deterministic and actually reaches both wardens.
        let mut seen_broom = false;
        let mut seen_megarat = false;
        for seed in 0..64u64 {
            let a = BossKind::for_theme_seeded(Theme::Kitchen, seed);
            let b = BossKind::for_theme_seeded(Theme::Kitchen, seed);
            assert_eq!(a, b, "seed {seed} must be deterministic");
            assert!(pool.contains(&a));
            seen_broom |= a == BossKind::Broom;
            seen_megarat |= a == BossKind::MegaRat;
        }
        assert!(seen_broom && seen_megarat, "both wardens must be reachable");
        // The Sewer is the other pooled floor (the Jaguar and the Horseshoe),
        // and its draw stays inside its own pool.
        let sewer = BossKind::pool_for_theme(Theme::Sewer);
        assert_eq!(sewer, &[BossKind::Cat, BossKind::HorseshoeCrab]);
        for seed in 0..64u64 {
            assert!(sewer.contains(&BossKind::for_theme_seeded(Theme::Sewer, seed)));
        }
        // Every single-entry floor cannot move at all.
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
    fn the_real_schedule_spawns_r01_in_the_kitchen_arena() {
        let mut w = kitchen_world(7);
        w.elapsed = crate::game::director::BOSS_TIME;
        w.update(DT, Vec2::ZERO, false);
        let boss = boss_of(&mut w).expect("the schedule spawned the boss");
        assert_eq!(boss.kind, BossKind::MegaRat);
        let arena = w.level.fight_rect_around(w.player.pos);
        assert!(arena.contains_world(boss.pos), "it spawns in its arena");
        // The pool is the framework's: art pool × scale × the director curve.
        let expect = BossKind::MegaRat.hp_pool(w.level_index());
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
        assert_eq!(name, crate::lore::MEGARAT_NAME);
        assert_eq!(tag, crate::lore::MEGARAT_CLASS);
        // Keep the fight running for a few seconds of real ticks.
        for _ in 0..240 {
            w.update(DT, Vec2::ZERO, false);
        }
        assert!(boss_of(&mut w).is_some());
    }

    #[test]
    fn phases_follow_the_art() {
        assert_eq!(BossKind::MegaRat.phase_for(1.0), 1);
        assert_eq!(BossKind::MegaRat.phase_for(art::PHASE2_HP_FRAC), 2);
        assert_eq!(BossKind::MegaRat.phase_for(0.5), 2);
        assert_eq!(BossKind::MegaRat.phase_for(art::PHASE3_HP_FRAC), 3);
        assert_eq!(BossKind::MegaRat.phase_for(0.0), 3);
        assert_eq!(
            BossKind::MegaRat.initial_state(),
            BossState::MegaRat(MegaRatState::Groom)
        );
        assert_eq!(BossKind::MegaRat.contact_damage(), 8.0);
    }

    // ---- the gnaw lunge ---------------------------------------------------

    #[test]
    fn the_gnaw_lands_after_its_tell_and_only_once() {
        let mut w = kitchen_world(11);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(
            &mut w,
            centre - Vec2::new(50.0, 0.0),
            centre + Vec2::new(30.0, 0.0),
        );
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp0 = w.player.hp;
        set_state(&mut w, MegaRatState::GnawWindup, art::GNAW_WINDUP[0]);

        // The tell: nothing hurts before the line is live.
        let mut t = 0.0;
        while t < art::GNAW_WINDUP[0] * art::GNAW_LOCK_PROGRESS {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0, "the teeth line hurt before it was live");
        }
        // The committed bite is on the rat, and the app sees a telegraph.
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.target - w.player.pos).len() < 8.0,
            "the bite point should be on the rat ({:?} vs {:?})",
            boss.target,
            w.player.pos
        );

        // Exactly one hit, the art's number.
        let mut hits = 0;
        for _ in 0..(1.6 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if state_of(&mut w) == MegaRatState::GnawLunge {
                // The corridor is the lunge's damage: a body bump would spend
                // the rat's i-frames and mask the bite.
                assert!(
                    boss_of(&mut w).expect("alive").contact_damage <= 0.0,
                    "the lunge deals no body contact"
                );
            }
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!(
                    (before - w.player.hp - art::GNAW_DAMAGE).abs() < 0.6,
                    "gnaw hit for {} not {}",
                    before - w.player.hp,
                    art::GNAW_DAMAGE
                );
            }
            if state_of(&mut w) == MegaRatState::GnawStuck {
                break;
            }
        }
        assert_eq!(hits, 1, "a lunge is a single hit");
        assert_eq!(state_of(&mut w), MegaRatState::GnawStuck);
    }

    #[test]
    fn a_dodged_gnaw_opens_the_belly() {
        let mut w = kitchen_world(12);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(60.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        set_state(&mut w, MegaRatState::GnawWindup, art::GNAW_WINDUP[0]);
        // Commit the line, then step out of it (north).
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").flags & F_LOCK == 0 && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        w.player.pos = centre + Vec2::new(60.0, 40.0);
        let hp = w.player.hp;
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::GnawStuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MegaRatState::GnawStuck);
        assert_eq!(w.player.hp, hp, "the rat stepped off the line");
        let boss = boss_of(&mut w).expect("alive");
        // The punish window: the art's multiplier, no body contact, and the
        // belly exposed where a rat that walked in can reach it.
        assert!((boss.damage_mult - art::GNAW_STUCK_DAMAGE_MULT).abs() < 1e-6);
        assert!(boss.contact_damage <= 0.0, "a reared rat cannot bump");
        assert!((boss.duration - art::GNAW_STUCK[0]).abs() < 1e-3);
        let view = boss.megarat_view();
        let (belly, radius) = art::weak_point(&view).expect("the belly is exposed");
        assert!(radius >= art::BELLY_RADIUS - 1e-3);
        assert!(boss.hit_shapes().circle_hits_grounded(belly, 2.0));
        // ...and it is the first auto-aim target point, so the rat's weapons
        // reach for it.
        let points = boss.target_points();
        assert!((points.first().expect("a target point") - belly).len() < 0.5);
        // The window lasts its beat, then it is back to work.
        let mut t = 0.0;
        while state_of(&mut w) == MegaRatState::GnawStuck && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert!((t - art::GNAW_STUCK[0]).abs() < 0.10, "stuck {t}");
        assert_eq!(state_of(&mut w), MegaRatState::Groom);
    }

    #[test]
    fn a_lunge_into_a_wall_stays_stuck_longer() {
        let mut w = kitchen_world(13);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        let arena = w.level.fight_rect_around(w.player.pos);
        // A solid column east of the boss: the teeth go into it.
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
        place(
            &mut w,
            Vec2::new(face_x - 18.0, row),
            Vec2::new(face_x - 90.0, row),
        );
        set_state(&mut w, MegaRatState::GnawWindup, art::GNAW_WINDUP[0]);
        // Aim it at the wall by putting the rat beyond it.
        w.player.pos = Vec2::new(face_x + 30.0, row);
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::GnawStuck && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MegaRatState::GnawStuck);
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.duration - art::GNAW_STUCK_WALLED).abs() < 1e-3,
            "walled window {} should be {}",
            boss.duration,
            art::GNAW_STUCK_WALLED
        );
    }

    // ---- the tail thrash --------------------------------------------------

    #[test]
    fn the_thrash_sweeps_two_halves_with_a_real_gap() {
        let mut w = kitchen_world(14);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        // The rat north of the boss: its side is +1 (the arc's left half).
        place(&mut w, centre, centre + Vec2::new(46.0, 0.0));
        w.player.pos = centre + Vec2::new(46.0, 0.0) + Vec2::new(0.0, 0.0);
        set_state(&mut w, MegaRatState::ThrashWindup, art::THRASH_WINDUP[0]);
        let side = {
            let boss = boss_of(&mut w).expect("alive");
            boss.scratch[S_SIDE]
        };
        let _ = side;
        // Windup: two arcs queued, neither live yet.
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.5 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(boss.hazards.len(), 2, "one hazard per pass");
        for h in &boss.hazards {
            assert_eq!(h.damage, art::THRASH_DAMAGE);
            match h.shape {
                HazardShape::Arc {
                    inner, outer, half_angle, ..
                } => {
                    assert!((inner - art::THRASH_INNER).abs() < 1e-3);
                    assert!((outer - art::THRASH_OUTER).abs() < 1e-3);
                    assert!((half_angle - art::THRASH_HALF_ARC * 0.5).abs() < 1e-3);
                }
                other => panic!("a pass must be an arc, got {other:?}"),
            }
        }
        // Pass 1 is the **rat's own half** (the bug this pins: squaring the
        // side selector cancelled it and both passes landed on the same side).
        let facing = boss_of(&mut w).expect("alive").facing.normalized();
        let half = art::THRASH_HALF_ARC * 0.5;
        let want_first = facing.rotate(half);
        let want_second = facing.rotate(-half);
        for (h, want) in boss_of(&mut w)
            .expect("alive")
            .hazards
            .iter()
            .zip([want_first, want_second])
        {
            match h.shape {
                HazardShape::Arc { facing: dir, .. } => assert!(
                    (dir - want).len() < 1e-3,
                    "pass aim {dir:?} should be {want:?} (the rat is on the + side)"
                ),
                other => panic!("a pass must be an arc, got {other:?}"),
            }
        }

        // Pass 1 is live first, pass 2 only after the gap.
        let (warn0, warn1) = {
            let b = boss_of(&mut w).expect("alive");
            (b.hazards[0].warn, b.hazards[1].warn)
        };
        assert!(
            warn1 >= warn0 + art::THRASH_PASS + art::THRASH_GAP - 1e-3,
            "pass 2 must wait for pass 1 plus the gap ({warn0} -> {warn1})"
        );

        // Run the whole beat and watch the live windows: pass 1, nothing, pass 2.
        // A hazard and the state clock are advanced by the same `dt` from
        // different anchors, so a window edge can land one tick either side of
        // the state change; the assertions allow that one tick and no more.
        let mut saw_pass1 = false;
        let mut saw_gap = false;
        let mut saw_pass2 = false;
        let mut t = 0.0;
        while t < 4.0 && state_of(&mut w) != MegaRatState::ThrashRecover {
            tick(&mut w);
            t += DT;
            let b = boss_of(&mut w).expect("alive");
            // Which pass is each live hazard? By its aim, not its slot: the
            // framework drops an expired hazard, so pass 2 moves into slot 0
            // the tick pass 1 runs out.
            let live: Vec<usize> = b
                .hazards
                .iter()
                .filter(|h| h.is_active())
                .map(|h| match h.shape {
                    HazardShape::Arc { facing: dir, .. } => {
                        if (dir - want_first).len() < 1e-3 {
                            0
                        } else {
                            assert!((dir - want_second).len() < 1e-3, "a stray arc: {dir:?}");
                            1
                        }
                    }
                    other => panic!("a pass must be an arc, got {other:?}"),
                })
                .collect();
            match megastate(&b) {
                MegaRatState::ThrashPass1 => {
                    // Pass 2 can never be live while pass 1 is.
                    assert!(!live.contains(&1), "pass 2 fired early: {live:?}");
                    saw_pass1 |= live.contains(&0);
                }
                MegaRatState::ThrashGap => {
                    // The middle of the gap has nothing live at all: that is
                    // the window the rat dashes through.
                    if b.progress() > 0.25 && b.progress() < 0.75 {
                        assert!(live.is_empty(), "the gap is a gap: {live:?}");
                        saw_gap = true;
                    }
                }
                MegaRatState::ThrashPass2 => {
                    assert!(!live.contains(&0), "pass 1 is still live: {live:?}");
                    saw_pass2 |= live.contains(&1);
                }
                _ => {}
            }
        }
        assert!(
            saw_pass1 && saw_gap && saw_pass2,
            "each pass must fire once and the gap must be empty ({saw_pass1} {saw_gap} {saw_pass2})"
        );
        assert_eq!(state_of(&mut w), MegaRatState::ThrashRecover);
        // The recovery exposes the tail base behind it.
        let boss = boss_of(&mut w).expect("alive");
        assert!((boss.damage_mult - art::THRASH_RECOVER_MULT).abs() < 1e-6);
        let view = boss.megarat_view();
        let (base, _) = art::weak_point(&view).expect("the tail base is exposed");
        assert!(
            (base - (view.pos - view.facing.normalized() * art::TAIL_BASE_OFFSET)).len() < 0.5,
            "the weak point is the tail base, behind it"
        );
    }

    // ---- the sonic squeak -------------------------------------------------

    #[test]
    fn the_squeak_ring_leaves_the_wedge_behind_safe() {
        let mut w = kitchen_world(15);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        // The rat inside the live band, in front of the boss.
        place(
            &mut w,
            centre,
            centre + Vec2::new((art::SQUEAK_INNER + art::SQUEAK_OUTER) * 0.5, 0.0),
        );
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp = w.player.hp;
        set_state(&mut w, MegaRatState::SqueakWindup, art::SQUEAK_WINDUP[0]);
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::SqueakShout && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MegaRatState::SqueakShout);
        let drop = hp - w.player.hp;
        assert!(
            (drop - art::SQUEAK_DAMAGE).abs() < 0.6,
            "the ring should shout for {} not {drop}",
            art::SQUEAK_DAMAGE
        );
        // ...and a rat that dives *behind* it once the ring has committed is
        // untouched: the wedge behind the boss is the safe floor.
        let mut w = kitchen_world(16);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(
            &mut w,
            centre,
            centre + Vec2::new((art::SQUEAK_INNER + art::SQUEAK_OUTER) * 0.5, 0.0),
        );
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp = w.player.hp;
        set_state(&mut w, MegaRatState::SqueakWindup, art::SQUEAK_WINDUP[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").flags & F_LOCK == 0 && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        // The ring has committed and the boss has stopped aiming: the rat runs
        // through the (safe) inner radius into the wedge behind it.
        let boss = boss_of(&mut w).expect("alive");
        w.player.pos = boss.pos - boss.facing * ((art::SQUEAK_INNER + art::SQUEAK_OUTER) * 0.5);
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::Groom && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(w.player.hp, hp, "the wedge behind it is the safe floor");

        // Standing on its feet (inside the inner radius) is safe too: the shout
        // leaves the boss's own body, exactly like the rat's weapon.
        let mut w = kitchen_world(24);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(40.0, 0.0));
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
        let hp = w.player.hp;
        set_state(&mut w, MegaRatState::SqueakWindup, art::SQUEAK_WINDUP[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").flags & F_LOCK == 0 && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        // On its own feet: inside the inner radius, so the shout goes over the
        // rat's head. (The body still bumps: this is contact, not the ring.)
        w.player.pos = boss.pos + boss.facing * 12.0;
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::Groom && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        let lost = hp - w.player.hp;
        assert!(
            lost <= art::CONTACT_DAMAGE + 0.6,
            "inside the inner radius the shout passes over (lost {lost}, ring is {})",
            art::SQUEAK_DAMAGE
        );
    }

    // ---- the dash ---------------------------------------------------------

    #[test]
    fn the_dash_reads_its_cooldown_like_the_hud_gauge() {
        let mut w = kitchen_world(17);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(70.0, 0.0));
        // A dash the bot could start: the gauge is up.
        {
            let mut boss = boss_of(&mut w).expect("alive");
            let mut rng = rand::rngs::StdRng::seed_from_u64(1);
            let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
            assert!(commit_dash(&mut ctx, &mut boss), "a lane through the rat fits");
        }
        set_state(&mut w, MegaRatState::DashWindup, art::DASH_WINDUP[0]);
        // Run the dash out: it must end in the skid with the gauge spent.
        let mut t = 0.0;
        while state_of(&mut w) != MegaRatState::DashSkid && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert_eq!(state_of(&mut w), MegaRatState::DashSkid);
        let boss = boss_of(&mut w).expect("alive");
        assert!(
            (boss.scratch[S_DASH_CD] - art::DASH_COOLDOWN).abs() < 1e-3,
            "the skid restarts the cooldown"
        );
        assert!(boss.megarat_view().dash_cd > 0.9, "the gauge reads spent");
        // A cooling boss cannot dash again, however good the lane.
        {
            let mut boss = boss_of(&mut w).expect("alive");
            let mut rng = rand::rngs::StdRng::seed_from_u64(2);
            let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
            assert!(
                !commit_dash(&mut ctx, &mut boss),
                "the cooldown is a real cooldown"
            );
        }
        // ...and the gauge refills on its own clock.
        let mut t = 0.0;
        while t < art::DASH_COOLDOWN + 0.2 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        assert!(boss.scratch[S_DASH_CD] <= 0.0, "the gauge refilled");
    }

    // ---- the phase change and the call ------------------------------------

    #[test]
    fn a_phase_threshold_is_a_harmless_call_that_summons_rats() {
        let mut w = kitchen_world(18);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(50.0, 0.0));
        w.clear_enemies();
        let max = boss_of(&mut w).expect("alive").max_hp;
        // A live telegraph from the move it was in the middle of.
        set_state(&mut w, MegaRatState::GnawWindup, art::GNAW_WINDUP[0]);
        with_boss(&mut w, |b| b.hp = max * (art::PHASE2_HP_FRAC - 0.01));
        tick(&mut w);
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(boss.phase, 2);
        assert_eq!(megastate(&boss), MegaRatState::Call);
        assert!((boss.duration - art::CALL_DURATION).abs() < 1e-3);
        assert!(boss.contact_damage <= 0.0, "the call is harmless");
        assert!(
            boss.hazards.is_empty(),
            "a set piece cancels the telegraph it interrupted"
        );
        assert_eq!(boss.call_count, 0, "the rats come at the beat, not before");

        // At the beat the walls answer: the request rides on the boss...
        let mut t = 0.0;
        let mut concurrent = 0;
        while state_of(&mut w) == MegaRatState::Call && t < 2.5 {
            tick(&mut w);
            t += DT;
            concurrent = concurrent.max(boss_of(&mut w).expect("alive").call_count);
        }
        assert_eq!(concurrent, art::CALL_RATS, "the call sends its rats");
        // ...and the summon system spawns them at the head of the next tick.
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(super::boss_summon_system)
            .expect("summon system");
        w.end_tick();
        assert!(
            w.enemy_count() >= art::CALL_RATS as usize,
            "the answer must actually arrive: {} bodies",
            w.enemy_count()
        );
        assert_eq!(
            boss_of(&mut w).expect("alive").call_count,
            0,
            "the request is consumed once"
        );
        // A second summon tick with nothing pending is a no-op (and draws no
        // rng, so a fight that never calls is byte-identical).
        let before = w.enemy_count();
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(super::boss_summon_system)
            .expect("summon system");
        w.end_tick();
        assert_eq!(w.enemy_count(), before, "a consumed call spawns nothing");
    }

    #[test]
    fn phase_two_chains_two_moves_before_it_grooms_again() {
        let mut w = kitchen_world(19);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(54.0, 0.0));
        let max = boss_of(&mut w).expect("alive").max_hp;
        with_boss(&mut w, |b| {
            b.hp = max * 0.5;
            b.phase = 2;
            b.scratch[S_CHAIN] = 1.0;
            b.set_state(BossState::MegaRat(MegaRatState::GnawStuck), 0.12);
            b.flags = 0;
        });
        // The stuck beat has 0.12 s to run; the chain fires when it ends.
        let mut t = 0.0;
        while state_of(&mut w) == MegaRatState::GnawStuck && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        let after = state_of(&mut w);
        assert_ne!(after, MegaRatState::Groom, "phase 2 chains: {after:?}");
        assert!(
            art::is_telegraph(after)
                || after == MegaRatState::ThrashPass1
                || after == MegaRatState::SqueakShout
                || after == MegaRatState::DashPass
                || after == MegaRatState::DashWindup
                || after == MegaRatState::GnawLunge,
            "the second move of the chain: {after:?}"
        );
        assert!(
            boss_of(&mut w).expect("alive").scratch[S_CHAIN] < 0.5,
            "the chain budget is spent"
        );
    }

    // ---- death ------------------------------------------------------------

    #[test]
    fn the_hazards_stop_when_it_dies() {
        let mut w = kitchen_world(20);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(60.0, 0.0));
        set_state(&mut w, MegaRatState::GnawWindup, art::GNAW_WINDUP[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        assert!(!boss_of(&mut w).expect("alive").hazards.is_empty());
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
        assert!(boss_of(&mut w).is_none(), "the death beat reaps it");
        assert!(w.level_complete());
        assert!(w.gold >= crate::game::shop::BOSS_GOLD, "the clear pays 30 gold");
    }

    // ---- the hunt ---------------------------------------------------------

    #[test]
    fn it_runs_down_a_rat_that_leaves_the_arena() {
        let mut w = kitchen_world(21);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        let arena = w.level.fight_rect_around(w.player.pos);
        // The rat out west of the arena, on real Kitchen floor (snapped onto a
        // cell the small class can stand in).
        let west = Vec2::new(
            (arena.x as f32 - 12.0) * TILE_F,
            (arena.y as f32 + arena.h as f32 * 0.5) * TILE_F,
        );
        let rat = w
            .level
            .tiles
            .nearest_open_center(west, crate::game::collision::SizeClass::Small, 12)
            .expect("open floor west of the arena");
        assert!(!arena.contains_world(rat), "the rat must be out of the arena");
        place(&mut w, centre, rat);
        w.clear_enemies();
        let d0 = boss_of(&mut w).expect("alive").pos.dist(rat);
        for _ in 0..240 {
            w.begin_tick(DT, Vec2::ZERO, false);
            let _ = w
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::flow_refresh_system);
            let _ = w.ecs.run_system_cached(boss_update_system);
            w.end_tick();
        }
        let boss = boss_of(&mut w).expect("alive");
        assert!(boss.hunting, "it should be hunting");
        let d1 = boss.pos.dist(rat);
        assert!(
            d1 < d0 - 40.0,
            "it must close on a rat outside the arena ({d0:.0} -> {d1:.0})"
        );
    }

    // ---- suppression ------------------------------------------------------

    #[test]
    fn a_gnaw_the_rat_cannot_leave_is_refused() {
        let mut w = kitchen_world(22);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        let arena = w.level.fight_rect_around(w.player.pos);
        // A corridor one tile tall: the rat has nowhere to step off the line.
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + 2,
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
        let mut boss = boss_of(&mut w).expect("alive");
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(
            !commit_gnaw(&mut ctx, &mut boss),
            "a line with no escape is not a fair attack"
        );
        assert_eq!(megastate(&boss), MegaRatState::Groom);
    }

    #[test]
    fn a_squeak_whose_safe_wedge_is_wall_is_refused() {
        let mut w = kitchen_world(23);
        spawn_megarat(&mut w);
        let centre = arena_centre(&w);
        let arena = w.level.fight_rect_around(w.player.pos);
        // Wall the boss in from behind: the only safe floor the ring leaves is
        // a solid cabinet.
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + 2,
            3,
            arena.h - 4,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        let wall_x = (arena.x + 5) as f32 * TILE_F;
        place(
            &mut w,
            Vec2::new(wall_x + 20.0, centre.y),
            Vec2::new(wall_x + 60.0, centre.y),
        );
        let mut boss = boss_of(&mut w).expect("alive");
        let mut rng = rand::rngs::StdRng::seed_from_u64(4);
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(
            !commit_squeak(&mut ctx, &mut boss),
            "never scream a rat into a corner it cannot read"
        );
    }

}
