//! Flatbed boss pool — **THE REDTAIL** (the hawk that has been following the
//! truck).
//!
//! The art module [`crate::game::art_boss_hawk`] is the gameplay spec: the state
//! enum, every per-state duration, damage number, hit-circle helper
//! (`body_hit_circles`, `weak_point`), the mark geometry (`impact_circle`,
//! `dart_wedge`, `grab_reach`, `rake_lane`) and the overhead rule
//! (`is_overhead`) are binding. This file drives the transitions, picks the marks
//! and lanes, and queues the art's telegraphed attacks as shared
//! [`BossHazard`]s.
//!
//! ## The fight the level asked for
//!
//! The flatbed's own boss, the Magpie, is a thief: it pecks, it strafes, it
//! steals the cargo and blows the slipstream back at you. This is the predator
//! that hunts from above, and its three beats are the owner's:
//!
//! 1. **It arrives as a shadow.** [`HawkState::Circle`] is the bird *high
//!    overhead*: it has no hit shapes at all (the plate says
//!    [`art::OVERHEAD_TAG`]), only a shadow that circles the rat on the planks,
//!    lagging a half-step behind. That is the neutral beat — the rat is fighting
//!    the horde and reading the floor.
//! 2. **It dive bombs.** [`HawkState::Dive`] stops the shadow, snaps the talon
//!    reticle onto the deck where the rat was, screams, and falls. The impact is
//!    one [`art::DIVE_DAMAGE`] hit inside [`art::DIVE_RADIUS`] plus a
//!    damage-free shockwave ring. **A miss buries the talons**
//!    ([`HawkState::TalonBury`]): no contact, ×[`art::TALON_BURY_DAMAGE_MULT`]
//!    incoming, the back of the neck exposed as the first hit circle. A dive
//!    that connects wheels straight back up.
//! 3. **It hovers.** [`HawkState::Stoop`] brings it down into
//!    [`HawkState::Hover`]: fully hittable, wings beating out damage-free
//!    downdraft rings, and three ways to reach down — the feather-dart wedge
//!    ([`HawkState::DartWindup`]), the marked grab ([`HawkState::GrabWindup`])
//!    and the strafing talon rake ([`HawkState::RakeWindup`]). A wounded bird
//!    hovers lower, longer and more often.
//!
//! The feint (phase 2+) is a *second* reticle: the shadow jinks and the mark
//! flashes [`art::FEINT_OFFSET`] px off the rat's shoulder before the real lock
//! lands, so a rat that dodges the first print steps into the second.
//!
//! ## Suppression (every attack, every time)
//!
//! No move is committed when the rat could not read or leave it, or when the
//! flatbed's own holes are in the way. Refusals fall through to a cheaper move
//! and finally back to `Circle`/`Hover`:
//!
//! * a **dive** needs the mark on open floor, a clear line from the shadow, a
//!   clear step out of the reticle, **and no hole within
//!   [`art::DIVE_PIT_CLEARANCE`]** — the bird will not slam the rat toward a
//!   broken plank (its knockback is radial, so the check walks the ring);
//! * **darts** need the rat inside [`art::DART_REACH`] with a clear line and a
//!   clear step out of the wedge;
//! * the **grab** needs the rat inside [`art::GRAB_REACH`] with a clear step off
//!   the reach — a rat that could not have left is never grabbed;
//! * the **rake** needs a lane at least [`art::RAKE_MIN_LEN`] long that the
//!   bird's body fits along, with [`art::RAKE_SAFE_SIDE`] px of clear deck on
//!   one side of the rat.
//!
//! ## Determinism
//!
//! One boss-rng draw per `Circle` entry (which way the shadow circles) and one
//! per `commit_dive` (which shoulder a feint jinks to). Every attack choice is a
//! fixed rotation in `Boss::scratch`, and the hazard ring is the boss's own
//! pre-reserved `Vec`. Nothing allocates per tick.
//!
//! ## Spawns
//!
//! The controller takes no `Commands`: it moves the boss, queues hazards and
//! cues. No `Enemy` entity is spawned or despawned inside the `EnemyIndex`
//! window by this fight.

use super::*;
use crate::game::art_boss_hawk as art;
use crate::game::art_boss_hawk::HawkState;

// ---- scratch layout -------------------------------------------------------

/// The overhead rotation counter (`0..4`, persists across states).
const S_ROT: usize = 0;
/// The hover rotation counter (`0..3`).
const S_HROT: usize = 1;
/// `1.0` while the bird is in its low mode (it resumes it after a phase pose).
const S_MODE: usize = 2;
/// Hover attacks spent in the current low mode.
const S_HOVER_N: usize = 3;
/// The feint's decoy point (world px), so the art can draw the print it jinked
/// to.
pub(crate) const S_DECOY_X: usize = 4;
pub(crate) const S_DECOY_Y: usize = 5;

// ---- flags layout ---------------------------------------------------------

/// The committed mark has queued its live hazard(s) (one per tell).
const F_LOCK: u32 = 1 << 0;
/// This dive is a feint (the decoy is armed).
const F_FEINT: u32 = 1 << 1;
/// The feint's decoy print has been shown.
const F_DECOY: u32 = 1 << 2;
/// The dive's impact has landed (the hand-over to bury/wheel).
const F_IMPACT: u32 = 1 << 3;
/// The death beat has taken its one shot (its own bit).
const F_DEATH: u32 = 1 << 4;
/// Downdraft ring `k` has been beaten: bits 8..8+[`art::DOWNDRAFT_MAX`].
const F_BEAT0: u32 = 1 << 8;

/// The step the rat needs to leave a line or a lane (px).
const ESCAPE_STEP: f32 = 24.0;

/// Free-flight margin from the level's edges (px): the shadow stays on the map.
const FLY_MARGIN: f32 = 14.0;

// ============================================================================
// Controller
// ============================================================================

/// The Redtail's controller (zero-sized; the state lives on the [`Boss`]).
pub struct HawkController;

impl BossController for HawkController {
    fn kind(&self) -> BossKind {
        BossKind::Hawk
    }

    fn tick(&self, ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        let before = hawk_state(boss);
        // The bird follows the rat out of the hold on the large-class flow
        // field (it flies, so it crosses cargo; the shadow still has to find
        // him).
        boss.hunting = !ctx.arena.contains_world(ctx.player_pos);

        // A phase threshold is a set piece: the harmless alight.
        // Whatever it was winding up is cancelled with the move it belonged to
        // (the Mangle's and the Cobra's rule): a wedge, a reach or a rake lane
        // must not land out of a pose that is no longer attacking.
        if out.phase_changed && !matches!(boss.state, BossState::Hawk(HawkState::Death)) {
            boss.hazards.clear();
            enter(boss, HawkState::Alight, art::ALIGHT_DURATION);
        }

        match boss.state {
            BossState::Hawk(HawkState::Circle) => tick_circle(ctx, boss, out),
            BossState::Hawk(HawkState::Dive) => tick_dive(ctx, boss, out),
            BossState::Hawk(HawkState::TalonBury) => tick_talon_bury(ctx, boss),
            BossState::Hawk(HawkState::Wheel) => tick_wheel(ctx, boss),
            BossState::Hawk(HawkState::Stoop) => tick_stoop(ctx, boss),
            BossState::Hawk(HawkState::Hover) => tick_hover(ctx, boss, out),
            BossState::Hawk(HawkState::DartWindup) => tick_dart_windup(ctx, boss),
            BossState::Hawk(HawkState::DartFan) => tick_dart_fan(ctx, boss),
            BossState::Hawk(HawkState::GrabWindup) => tick_grab_windup(ctx, boss),
            BossState::Hawk(HawkState::Grab) => tick_grab(ctx, boss),
            BossState::Hawk(HawkState::RakeWindup) => tick_rake_windup(ctx, boss),
            BossState::Hawk(HawkState::RakePass) => tick_rake_pass(ctx, boss),
            BossState::Hawk(HawkState::Alight) => tick_alight(ctx, boss),
            BossState::Hawk(HawkState::Climb) => tick_climb(boss),
            BossState::Hawk(HawkState::Death) => {}
            BossState::Cat(_)
            | BossState::Broom(_)
            | BossState::Mower(_)
            | BossState::Mangle(_)
            | BossState::NightEdition(_)
            | BossState::Magpie(_)
            | BossState::HorseshoeCrab(_)
            | BossState::MegaRat(_)
            | BossState::Cobra(_)
            | BossState::Lantern(_) => {}
        }

        // The art owns the multiplier and the contact rule; apply them after the
        // transitions so a punish window is live on the tick it opens.
        let now = hawk_state(boss);
        boss.damage_mult = art::incoming_damage_mult(now);
        if !art::has_body_contact(now) {
            boss.no_contact();
        }

        // One cue per state entry, on the tick of the transition.
        let after = hawk_state(boss);
        if after != before {
            if let Some(cue) = state_cue(after) {
                out.cue(cue);
            }
        }
    }

    /// The death beat: it comes down out of the sky for the last time.
    ///
    /// Its telegraphs stop with it: the first tick of the beat clears every
    /// hazard the living bird had queued — a locked reticle, a live wedge, a
    /// marked reach, a rake lane, every downdraft ring — so nothing can land on
    /// the rat after the fight is already won. Nothing new is dealt here.
    fn death_tick(&self, _ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
        if boss.flags & F_DEATH == 0 {
            boss.flags |= F_DEATH;
            boss.hazards.clear();
            out.cue(BossCue::HawkDeath);
        }
    }
}

/// The sound/effect cue for entering a state (within-state beats — the impact,
/// the rings, the darts — are cued where they happen).
#[inline]
fn state_cue(state: HawkState) -> Option<BossCue> {
    match state {
        HawkState::Circle => Some(BossCue::HawkCall),
        HawkState::TalonBury => Some(BossCue::HawkStuck),
        HawkState::Wheel | HawkState::Climb => Some(BossCue::HawkWings),
        HawkState::Stoop => Some(BossCue::HawkStoop),
        HawkState::Hover => Some(BossCue::HawkHover),
        HawkState::DartWindup => Some(BossCue::HawkDartWindup),
        HawkState::DartFan => Some(BossCue::HawkDarts),
        HawkState::GrabWindup => Some(BossCue::HawkGrabWindup),
        HawkState::Grab => Some(BossCue::HawkGrab),
        HawkState::RakeWindup => Some(BossCue::HawkRakeWindup),
        HawkState::RakePass => Some(BossCue::HawkRake),
        HawkState::Alight => Some(BossCue::HawkRuffle),
        _ => None,
    }
}

// ============================================================================
// States
// ============================================================================

/// **Overhead: the shadow circles.** The bird is out of reach; the shadow is
/// the fight's only read, and it holds a ring around the rat a half-step behind
/// him.
fn tick_circle(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    let sign = if boss.swipe_side < 0.0 { -1.0 } else { 1.0 };
    let bearing = if dist > 1e-3 { to * (1.0 / dist) } else { Vec2::ZERO };
    let dir = if dist < art::CIRCLE_BAND[0] {
        // Too close: drift outward across the rat's shoulder.
        (bearing * -0.75 + bearing.perp() * (sign * 0.65)).normalized()
    } else if dist > art::CIRCLE_BAND[1] {
        // A bird does not fly straight at a rat: it leads the run, then closes.
        let lead = ctx.player_vel * 0.35;
        let want = (ctx.player_pos + lead - boss.pos).normalized();
        if ctx.los_clear(boss.pos, ctx.player_pos, 1.0) {
            want
        } else {
            ctx.seek(boss)
        }
    } else {
        // In the band: hold it by sliding around.
        (bearing.perp() * (sign * 0.85) + bearing * 0.18).normalized()
    };
    let speed = art::CIRCLE_SPEED[slot]
        * if boss.hunting {
            art::HUNT_SPEED_MULT
        } else {
            1.0
        };
    fly(ctx, boss, dir, speed);
    if dir.len_sq() > 1e-6 {
        boss.facing = dir;
    }
    boss.target = ctx.player_pos;
    boss.pounce_origin = boss.pos;
    if boss.progress() >= 1.0 {
        next_overhead(ctx, boss, out);
    }
}

/// **The dive.** The shadow stops at the lock, the reticle snaps onto the deck,
/// the bird screams and falls on the mark.
fn tick_dive(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let p = boss.progress();
    if p < art::DIVE_LOCK_PROGRESS {
        // The shadow is still reading the floor. A feint jinks once: the decoy
        // print flashes off the rat's shoulder, so a rat that leaves on the
        // first print leaves for the wrong place.
        if boss.flags & F_FEINT != 0 && p >= art::DIVE_FEINT_PROGRESS && boss.flags & F_DECOY == 0 {
            boss.flags |= F_DECOY;
            let perp = (ctx.player_pos - boss.pos).normalized().perp();
            let decoy = ctx.player_pos
                + perp * (boss.swipe_side.signum() * art::FEINT_OFFSET)
                + Vec2::new(0.0, art::FEINT_OFFSET * 0.25);
            boss.scratch[S_DECOY_X] = decoy.x;
            boss.scratch[S_DECOY_Y] = decoy.y;
            out.cue(BossCue::HawkFeint);
        }
        if boss.flags & F_DECOY == 0 {
            // Still reading: the mark trails the rat.
            boss.target = ctx.player_pos;
        }
        let to = boss.target - boss.pos;
        if to.len() > 4.0 {
            fly(ctx, boss, to, art::CIRCLE_SPEED[phase_slot(boss)] * 0.8);
        }
        if to.len_sq() > 1e-6 {
            boss.facing = to;
        }
        return;
    }
    if boss.flags & F_LOCK == 0 {
        // The real lock: the shadow has stopped and the deck point is frozen.
        boss.flags |= F_LOCK;
        boss.target = ctx.player_pos;
        boss.pounce_origin = boss.pos;
        let to = boss.target - boss.pos;
        if to.len_sq() > 1e-6 {
            boss.facing = to;
        }
        out.cue(BossCue::HawkLock);
        let left = (art::DIVE_IMPACT_PROGRESS - art::DIVE_LOCK_PROGRESS).max(0.05);
        let warn = left * boss.duration;
        // One frame of impact inside the talon spread.
        let impact = BossHazard::new(
            HazardShape::Circle {
                center: boss.target,
                radius: art::DIVE_RADIUS,
            },
            warn,
            art::DIVE_ACTIVE,
            art::DIVE_DAMAGE,
        )
        .with_knockback(art::DIVE_KNOCKBACK);
        let _ = boss.push_hazard(impact);
        // The shockwave ring: push-only, so it costs no i-frames and the next
        // strike can still land.
        let shock = BossHazard::new(
            HazardShape::Annulus {
                center: boss.target,
                inner: art::DIVE_RADIUS,
                outer: art::DIVE_RING_OUTER,
            },
            warn,
            art::DIVE_RING_ACTIVE,
            0.0,
        )
        .with_knockback(art::DIVE_RING_PUSH);
        let _ = boss.push_hazard(shock);
    }
    // The stoop: the bird falls from where the shadow stopped onto the mark.
    let t = ((p - art::DIVE_LOCK_PROGRESS)
        / (art::DIVE_IMPACT_PROGRESS - art::DIVE_LOCK_PROGRESS).max(0.01))
    .clamp(0.0, 1.0);
    let eased = t * t;
    boss.pos = boss.pounce_origin + (boss.target - boss.pounce_origin) * eased;
    if p >= art::DIVE_IMPACT_PROGRESS && boss.flags & F_IMPACT == 0 {
        boss.flags |= F_IMPACT;
        boss.pos = boss.target;
        out.cue(BossCue::HawkImpact);
        // Did the talons find anything? The hazard is resolved right after this
        // tick, so the test is the committed shape itself.
        let caught = boss.hazards.iter().any(|h| {
            h.damage > 0.0 && h.shape.contains(ctx.player_pos, ctx.player_radius)
        });
        if caught {
            enter(boss, HawkState::Wheel, art::WHEEL_DURATION);
        } else {
            let slot = phase_slot(boss);
            enter(boss, HawkState::TalonBury, art::TALON_BURY_DURATION[slot]);
        }
    }
}

/// **The talons are in the planks.** The punish window a dodged dive opens.
fn tick_talon_bury(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Nose down, nailed to its own strike: it does not move and it does not
    // bump (the art's contact rule owns that).
    if boss.progress() >= 1.0 {
        enter(boss, HawkState::Wheel, art::WHEEL_DURATION);
    }
}

/// **Wheel.** A dive that connected: it bounces and climbs back out of reach.
fn tick_wheel(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Drift back toward the rat while it climbs, so the next shadow is already
    // in the fight.
    if boss.progress() >= 1.0 {
        enter_circle(ctx, boss);
    }
}

/// **The stoop.** Down out of the sky to the low hover: hittable and watching.
fn tick_stoop(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // It has to be over floor the rat can reach: a hover over a cargo stack is
    // a hover nothing can hit.
    if !ctx.footprint_fits(boss.pos, art::BODY_RADIUS) {
        boss.pos = ctx.nearest_open(boss.pos);
    }
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    let dir = if dist > art::HOVER_BAND[1] {
        ctx.seek(boss)
    } else if dist < art::HOVER_BAND[0] {
        -to.normalized()
    } else {
        to.normalized().perp() * boss.swipe_side.signum()
    };
    move_boss(ctx, boss, dir, art::STOOP_SPEED);
    if to.len_sq() > 1e-6 {
        boss.facing = to;
    }
    boss.target = ctx.player_pos;
    boss.pounce_origin = boss.pos;
    if boss.progress() >= 1.0 {
        enter_hover(ctx, boss);
    }
}

/// **The hover.** Wings beating two body-lengths over the planks: hittable,
/// watching, and beating downdraft rings out over the deck.
fn tick_hover(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    let sign = if boss.swipe_side < 0.0 { -1.0 } else { 1.0 };
    let bearing = if dist > 1e-3 { to * (1.0 / dist) } else { Vec2::ZERO };
    let dir = if dist < art::HOVER_BAND[0] {
        (bearing * -0.8 + bearing.perp() * (sign * 0.5)).normalized()
    } else if dist > art::HOVER_BAND[1] {
        ctx.seek(boss)
    } else {
        (bearing.perp() * (sign * 0.9) + bearing * 0.12).normalized()
    };
    move_boss(ctx, boss, dir, art::HOVER_SPEED[slot]);
    if to.len_sq() > 1e-6 {
        boss.facing = to;
    }
    boss.target = ctx.player_pos;
    boss.pounce_origin = boss.pos;

    // The downdraft: one push-only ring per beat of the wings.
    let elapsed = boss.progress() * boss.duration;
    for k in 0..art::DOWNDRAFT_MAX {
        if boss.flags & (F_BEAT0 << k) != 0 {
            continue;
        }
        if !art::downdraft_due(elapsed, boss.duration, k) {
            break;
        }
        boss.flags |= F_BEAT0 << k;
        out.cue(BossCue::HawkDowndraft);
        let ring = BossHazard::new(art::downdraft_shape(boss.pos), 0.0, art::DOWNDRAFT_ACTIVE, 0.0)
            .with_knockback(art::DOWNDRAFT_PUSH);
        let _ = boss.push_hazard(ring);
    }

    if boss.progress() >= 1.0 {
        // Enough of the low fight: climb back out of reach, or reach down one
        // more time.
        if boss.scratch[S_HOVER_N] >= f32::from(art::HOVER_LIMIT[slot] as u16) {
            boss.scratch[S_HOVER_N] = 0.0;
            enter(boss, HawkState::Climb, art::CLIMB_DURATION);
            return;
        }
        next_hover(ctx, boss, out);
    }
}

/// **Feather darts.** The wedge is frozen at the lock and fills once.
fn tick_dart_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.flags & F_LOCK == 0 && boss.progress() >= art::DART_LOCK_PROGRESS {
        // Commit: the wedge is the rat's mark from a moment ago.
        let Some((origin, target)) = pick_fan(ctx, boss) else {
            // The rat left the reach or the sight line: the throw is refused
            // and the bird repositions instead of throwing at nothing.
            enter_hover(ctx, boss);
            return;
        };
        boss.flags |= F_LOCK;
        boss.pounce_origin = origin;
        boss.target = target;
        let (_, reach, facing, half) = art::dart_wedge(&view_of(boss));
        let left = (1.0 - art::DART_LOCK_PROGRESS).max(0.05);
        let warn = left * boss.duration + art::DART_GAP;
        let wedge = BossHazard::new(
            HazardShape::Arc {
                center: origin,
                inner: art::DART_INNER,
                outer: reach,
                facing,
                half_angle: half,
            },
            warn,
            art::DART_ACTIVE,
            art::DART_DAMAGE,
        )
        .with_knockback(70.0);
        let _ = boss.push_hazard(wedge);
    }
    if boss.progress() >= 1.0 {
        if boss.flags & F_LOCK == 0 {
            enter_hover(ctx, boss);
            return;
        }
        enter(boss, HawkState::DartFan, art::DART_FAN_DURATION);
    }
}

/// **The darts cross.** The wedge is live (the hazard owns the hit); the state
/// is the artwork's beat and the hand-over.
fn tick_dart_fan(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // It does not fly during the fan: the wedge the rat read is the wedge the
    // darts take.
    if boss.progress() >= 1.0 {
        enter_hover(ctx, boss);
    }
}

/// **The reach is marked.** A talon's reach out to the rat, frozen at the lock.
fn tick_grab_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.flags & F_LOCK == 0 && boss.progress() >= art::GRAB_LOCK_PROGRESS {
        let Some((talons, far)) = pick_grab(ctx, boss) else {
            enter_hover(ctx, boss);
            return;
        };
        boss.flags |= F_LOCK;
        boss.pounce_origin = talons;
        boss.target = far;
        let to = far - talons;
        if to.len_sq() > 1e-6 {
            boss.facing = to;
        }
        let left = (1.0 - art::GRAB_LOCK_PROGRESS).max(0.05);
        let warn = left * boss.duration;
        // The carry: a damage-free corridor that drags the grabbed rat toward
        // the talons for the first half of `Grab`...
        let carry = BossHazard::new(
            HazardShape::Corridor {
                from: talons,
                to: far,
                half_width: art::GRAB_HALF_WIDTH,
            },
            warn,
            art::GRAB_CARRY_SECS,
            0.0,
        )
        .with_drag((talons - far).normalized() * art::GRAB_CARRY_DRAG);
        let _ = boss.push_hazard(carry);
        // ...and the drop: the one damage event, along the reach the tell
        // showed, once the carry is done. The bird holds still for the whole
        // grab, so the reach (and therefore the drop) is where the rat read it.
        // A rat that stepped or dashed off the marks is outside it.
        let drop = BossHazard::new(
            HazardShape::Corridor {
                from: talons,
                to: far,
                half_width: art::GRAB_HALF_WIDTH,
            },
            warn + art::GRAB_CARRY_SECS,
            art::GRAB_ACTIVE,
            art::GRAB_DAMAGE,
        )
        .with_knockback(64.0);
        let _ = boss.push_hazard(drop);
    }
    if boss.progress() >= 1.0 {
        if boss.flags & F_LOCK == 0 {
            enter_hover(ctx, boss);
            return;
        }
        enter(boss, HawkState::Grab, art::GRAB_DURATION);
    }
}

/// **The grab.** The talons close; the bird holds still while the carry runs.
fn tick_grab(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // It does not drift during a grab: the drop point has to be the one the rat
    // read. (A grabbed rat is dragged onto the talons by the hazard's own drag.)
    let _ = ctx;
    if boss.progress() >= 1.0 {
        enter_hover(ctx, boss);
    }
}

/// **The rake's lane is painted** down the bed at the rat's row.
fn tick_rake_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Hover onto the lane's line and its near end, then hold. Once the lane is
    // locked the bird settles onto the *committed* start and keeps the
    // committed heading: re-planning here would chase the rat's new row (the
    // live lane stays where it was painted) and could flip the facing, which
    // made `RakePass` read itself as already past its end on the first tick.
    if boss.flags & F_LOCK != 0 {
        let to = boss.pounce_origin - boss.pos;
        if to.len() > 1.0 {
            move_boss(ctx, boss, to, 96.0);
        }
    } else if let Some((start, _end, dir)) = lane_plan(ctx, boss) {
        let to = start - boss.pos;
        if to.len() > 1.0 {
            move_boss(ctx, boss, to, 96.0);
        }
        boss.facing = Vec2::new(dir.signum(), 0.0);
    }
    if boss.flags & F_LOCK == 0 && boss.progress() >= art::RAKE_LOCK_PROGRESS {
        let Some((start, end, dir)) = lane_plan(ctx, boss) else {
            enter_hover(ctx, boss);
            return;
        };
        boss.flags |= F_LOCK;
        boss.pounce_origin = start;
        boss.target = end;
        boss.facing = Vec2::new(dir.signum(), 0.0);
        let left = (1.0 - art::RAKE_LOCK_PROGRESS).max(0.05);
        let warn = left * boss.duration;
        let len = (end - start).len();
        let pass = (len / art::RAKE_SPEED[phase_slot(boss)]).max(0.12);
        let lane = BossHazard::new(
            HazardShape::Corridor {
                from: start,
                to: end,
                half_width: art::RAKE_HALF_WIDTH,
            },
            warn,
            pass,
            art::RAKE_DAMAGE,
        )
        .with_knockback(96.0);
        let _ = boss.push_hazard(lane);
    }
    if boss.progress() >= 1.0 {
        if boss.flags & F_LOCK == 0 {
            enter_hover(ctx, boss);
            return;
        }
        // The pass's own clock is its flight time down the lane.
        let len = boss.pos.dist(boss.target);
        let secs = (len / art::RAKE_SPEED[phase_slot(boss)]).max(0.12);
        enter(boss, HawkState::RakePass, secs);
    }
}

/// **The rake.** A low strafing flight down the lane, talons trailing.
fn tick_rake_pass(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let dir = boss.facing;
    let moved = move_boss(ctx, boss, dir, art::RAKE_SPEED[phase_slot(boss)]);
    let past = (boss.pos.x - boss.target.x) * dir.x >= 0.0;
    if !moved || past || boss.progress() >= 1.0 {
        enter_hover(ctx, boss);
    }
}

/// **Alight.** The phase threshold's harmless pose on the planks.
fn tick_alight(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        // Back to whichever mode it was fighting in.
        if boss.scratch[S_MODE] > 0.5 {
            enter_hover(ctx, boss);
        } else {
            enter_circle(ctx, boss);
        }
    }
}

/// **Climb.** Back up out of reach.
fn tick_climb(boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        boss.scratch[S_MODE] = 0.0;
        boss.scratch[S_HOVER_N] = 0.0;
        enter(boss, HawkState::Circle, art::CIRCLE_DURATION[phase_slot(boss)]);
        // A fresh circling direction each time it takes up the sky again.
        boss.swipe_side = if boss.swipe_side < 0.0 { 1.0 } else { -1.0 };
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
fn hawk_state(boss: &Boss) -> HawkState {
    match boss.state {
        BossState::Hawk(s) => s,
        _ => HawkState::Circle,
    }
}

/// The boss's own view, for the art's shared geometry helpers.
#[inline]
fn view_of(boss: &Boss) -> art::HawkView {
    boss.hawk_view()
}

/// Enter an art state, clearing the per-state latch and the beat bits.
#[inline]
fn enter(boss: &mut Boss, state: HawkState, duration: f32) {
    boss.set_state(BossState::Hawk(state), duration);
    boss.flags = 0;
}

/// Take up the sky: the shadow starts circling, on a fresh side.
fn enter_circle(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let roll = ctx.rng.random::<f32>();
    enter(boss, HawkState::Circle, art::CIRCLE_DURATION[slot]);
    // One draw per entry: which way the shadow circles.
    boss.swipe_side = if roll < 0.5 { -1.0 } else { 1.0 };
    boss.scratch[S_MODE] = 0.0;
    boss.pounce_origin = boss.pos;
}

/// Come down into the low fight.
fn enter_hover(ctx: &BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    enter(boss, HawkState::Hover, art::HOVER_DURATION[slot]);
    boss.scratch[S_MODE] = 1.0;
    boss.pounce_origin = boss.pos;
    boss.target = ctx.player_pos;
}

/// Free flight: the shadow crosses cargo and holes (it is in the air), but
/// never leaves the map.
fn fly(ctx: &BossContext<'_>, boss: &mut Boss, dir: Vec2, speed: f32) {
    let step = dir.normalized() * (speed * boss.slow_mult() * ctx.dt);
    if step.len_sq() <= 0.0 {
        return;
    }
    let world = ctx.level.arena;
    let max = Vec2::new(world.x - FLY_MARGIN, world.y - FLY_MARGIN);
    if max.x <= FLY_MARGIN || max.y <= FLY_MARGIN {
        return;
    }
    boss.pos = (boss.pos + step).clamp_box(Vec2::splat(FLY_MARGIN), max);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attack {
    Dive,
    Feint,
    Stoop,
    Darts,
    Grab,
    Rake,
}

/// Choose the next move from the air (no rng: a fixed rotation per phase).
fn next_overhead(ctx: &mut BossContext<'_>, boss: &mut Boss, _out: &mut BossOutput) {
    let rot = boss.scratch[S_ROT] as u32;
    boss.scratch[S_ROT] = ((rot + 1) % 4) as f32;
    let phase = boss.phase.clamp(1, 3);
    let want = match rot % 4 {
        // Phase 2 brings the feint in; phase 3 opens on one.
        0 => Attack::Dive,
        1 if phase >= 2 => Attack::Feint,
        2 => Attack::Stoop,
        3 if phase >= 3 => Attack::Feint,
        _ => Attack::Dive,
    };
    let mut pick = want;
    for _ in 0..3 {
        let ok = match pick {
            Attack::Dive => commit_dive(ctx, boss, false),
            Attack::Feint => commit_dive(ctx, boss, true),
            Attack::Stoop => {
                // The descent needs no mark: it is the safe fallback, and the
                // stoop itself repositions onto reachable floor.
                enter(boss, HawkState::Stoop, art::STOOP_DURATION);
                boss.pounce_origin = boss.pos;
                boss.target = ctx.player_pos;
                true
            }
            // Overhead moves only; the hover ones are refused here.
            _ => false,
        };
        if ok {
            return;
        }
        pick = match pick {
            Attack::Feint => Attack::Dive,
            Attack::Dive => Attack::Stoop,
            _ => Attack::Stoop,
        };
    }
    // Nowhere fair to strike from up here: circle again (the shadow re-seeks
    // and the next beat retries with the rat somewhere else).
    enter_circle(ctx, boss);
}

/// Choose the next move from the hover.
fn next_hover(ctx: &mut BossContext<'_>, boss: &mut Boss, _out: &mut BossOutput) {
    let rot = boss.scratch[S_HROT] as u32;
    boss.scratch[S_HROT] = ((rot + 1) % 3) as f32;
    boss.scratch[S_HOVER_N] += 1.0;
    let phase = boss.phase.clamp(1, 3);
    let want = match phase {
        1 => Attack::Darts,
        2 => {
            if rot.is_multiple_of(2) {
                Attack::Darts
            } else {
                Attack::Grab
            }
        }
        _ => match rot % 3 {
            0 => Attack::Rake,
            1 => Attack::Grab,
            _ => Attack::Darts,
        },
    };
    let mut pick = want;
    for _ in 0..3 {
        let ok = match pick {
            Attack::Darts => commit_darts(ctx, boss),
            Attack::Grab => commit_grab(ctx, boss),
            Attack::Rake => commit_rake(ctx, boss),
            _ => false,
        };
        if ok {
            return;
        }
        pick = match pick {
            Attack::Rake => Attack::Grab,
            Attack::Grab => Attack::Darts,
            _ => Attack::Darts,
        };
    }
    // Nothing it can reach from here: climb back out of reach.
    boss.scratch[S_HOVER_N] = 0.0;
    enter(boss, HawkState::Climb, art::CLIMB_DURATION);
}

/// Start a dive on the rat's current position, if the mark is fair.
fn commit_dive(ctx: &mut BossContext<'_>, boss: &mut Boss, feint: bool) -> bool {
    let slot = phase_slot(boss);
    let at = ctx.player_pos;
    // The shadow must be able to see the rat it is aiming at — but only when
    // the shadow is over open floor. The bird *flies over the cargo* (that is
    // the whole point of a hunting hawk), so a shadow sitting on a crate is not
    // a reason to refuse the dive; a bulkhead between the shadow and the rat
    // on open deck is.
    if ctx.footprint_fits(boss.pos, art::BODY_RADIUS) && !ctx.los_clear(boss.pos, at, 1.0) {
        return false;
    }
    if !ctx.footprint_fits(at, art::DIVE_RADIUS) {
        return false;
    }
    // The flatbed's own rule: never slam a rat toward a broken plank.
    if pit_near(ctx, at, art::DIVE_PIT_CLEARANCE) {
        return false;
    }
    if !escape_from_circle(ctx, at, art::DIVE_RADIUS + art::DIVE_ESCAPE_STEP) {
        return false;
    }
    let roll = ctx.rng.random::<f32>();
    let to = at - boss.pos;
    enter(boss, HawkState::Dive, art::DIVE_DURATION[slot]);
    boss.target = at;
    boss.pounce_origin = boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    if feint {
        boss.flags |= F_FEINT;
    }
    boss.swipe_side = if roll < 0.5 { -1.0 } else { 1.0 };
    // The decoy print: off the rat's shoulder (which side the jink takes is the
    // draw above).
    let perp = if to.len_sq() > 1e-6 {
        to.normalized().perp()
    } else {
        Vec2::new(0.0, 1.0)
    };
    let decoy = at + perp * (boss.swipe_side * art::FEINT_OFFSET);
    boss.scratch[S_DECOY_X] = decoy.x;
    boss.scratch[S_DECOY_Y] = decoy.y;
    true
}

/// Start a dart fan: a wedge out of the bill through the rat.
fn commit_darts(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some((origin, target)) = pick_fan(ctx, boss) else {
        return false;
    };
    let slot = phase_slot(boss);
    let to = target - origin;
    enter(boss, HawkState::DartWindup, art::DART_WINDUP[slot]);
    boss.pounce_origin = origin;
    boss.target = target;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    true
}

/// The fan's bill origin and centre landing point, or `None` when the throw is
/// unfair (out of reach, blocked, or with no step out of the wedge).
fn pick_fan(ctx: &BossContext<'_>, boss: &Boss) -> Option<(Vec2, Vec2)> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if !(art::DART_INNER + 8.0..=art::DART_REACH).contains(&dist) {
        return None;
    }
    let dir = to * (1.0 / dist);
    let origin = boss.pos + dir * art::BEAK_LENGTH;
    if !ctx.los_clear(origin, ctx.player_pos, 1.0) {
        return None;
    }
    if !escape_side_clear(ctx, ctx.player_pos, dir) {
        return None;
    }
    Some((origin, ctx.player_pos))
}

/// Start a grab: the talons' reach out to the rat.
fn commit_grab(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some((talons, far)) = pick_grab(ctx, boss) else {
        return false;
    };
    let slot = phase_slot(boss);
    let to = far - talons;
    enter(boss, HawkState::GrabWindup, art::GRAB_WINDUP[slot]);
    boss.pounce_origin = talons;
    boss.target = far;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    true
}

/// The talon origin and far end of a grab, or `None` when it would be unfair.
fn pick_grab(ctx: &BossContext<'_>, boss: &Boss) -> Option<(Vec2, Vec2)> {
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    if !(art::GRAB_MIN..=art::GRAB_REACH).contains(&dist) {
        return None;
    }
    let dir = to * (1.0 / dist);
    let talons = boss.pos + dir * 5.0;
    let far = ctx.player_pos;
    if !ctx.los_clear(talons, far, 1.0) {
        return None;
    }
    // A rat that could not have left is never grabbed: one clear step off the
    // reach, sideways or back, is the whole fairness rule.
    let perp = dir.perp();
    let can_leave = [1.0f32, -1.0].iter().any(|s| {
        ctx.los_clear(
            far,
            far + perp * (*s * art::GRAB_ESCAPE_STEP),
            ctx.player_radius,
        )
    }) || ctx.los_clear(far, far + dir * art::GRAB_ESCAPE_STEP, ctx.player_radius);
    if !can_leave {
        return None;
    }
    Some((talons, far))
}

/// Start the talon rake down a lane through the rat's row.
fn commit_rake(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some((start, end, dir)) = lane_plan(ctx, boss) else {
        return false;
    };
    let slot = phase_slot(boss);
    enter(boss, HawkState::RakeWindup, art::RAKE_WINDUP[slot]);
    boss.pounce_origin = start;
    boss.target = end;
    boss.facing = Vec2::new(dir.signum(), 0.0);
    true
}

/// A lane down the deck at the rat's row: `(start, end, dir)`.
///
/// The runway is the fight's own floor: the boss room when the rat is inside it
/// (the flatbed's hold is a clear, furnished stage and its own generator
/// guarantees the open lanes), otherwise a fixed corridor through the rat — a
/// bird rakes where the rat is, not the whole map.
fn lane_plan(ctx: &BossContext<'_>, boss: &Boss) -> Option<(Vec2, Vec2, f32)> {
    let (inner_min, inner_max, lane_y) = if ctx.arena.contains_world(ctx.player_pos) {
        let lo = ctx.arena.x as f32 * collision::TILE_F + art::RAKE_END_MARGIN;
        let hi = ctx.arena.right() as f32 * collision::TILE_F - art::RAKE_END_MARGIN;
        let y = ctx.player_pos.y.clamp(
            ctx.arena.y as f32 * collision::TILE_F + art::RAKE_END_MARGIN,
            (ctx.arena.bottom() as f32 * collision::TILE_F - art::RAKE_END_MARGIN)
                .max(ctx.arena.y as f32 * collision::TILE_F + art::RAKE_END_MARGIN),
        );
        (lo, hi, y)
    } else {
        // Out on the deck: a corridor through the rat, long enough to be a
        // flight and short enough to stay on the planks he is standing on.
        let half = art::RAKE_MIN_LEN + 26.0;
        (ctx.player_pos.x - half, ctx.player_pos.x + half, ctx.player_pos.y)
    };
    if inner_max - inner_min < art::RAKE_MIN_LEN {
        return None;
    }
    // Fly toward whichever side of the deck the rat is on; ambiguous means the
    // long way round.
    let dx = ctx.player_pos.x - boss.pos.x;
    let dir = if dx.abs() > 24.0 {
        dx.signum()
    } else if boss.pos.x > (inner_min + inner_max) * 0.5 {
        -1.0
    } else {
        1.0
    };
    let end_x = if dir > 0.0 { inner_max } else { inner_min };
    let start_x = boss.pos.x.clamp(inner_min, inner_max);
    let start = Vec2::new(start_x, lane_y);
    let mut end = Vec2::new(end_x, lane_y);
    // Batch 12: the fight rectangle is recentred on the rat, so it can hold real
    // walls and props where the old arena's generator guaranteed a clear stage.
    // The lane is clipped at the first solid rather than flown through it.
    if let Some(t) = ctx
        .level
        .tiles
        .segment_hit(start, end, art::BODY_RADIUS)
    {
        end = start + (end - start) * (t * 0.98).clamp(0.0, 1.0);
    }
    if (end.x - start.x).abs() < art::RAKE_MIN_LEN {
        return None;
    }
    // The whole flight has to fit the bird's body…
    let steps = ((end - start).len() / 8.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let p = start + (end - start) * (i as f32 / steps as f32);
        if !ctx.footprint_fits(p, art::BODY_RADIUS) {
            return None;
        }
    }
    // …and the rat has to be able to step off it.
    let lane_dir = Vec2::new(dir, 0.0);
    let on_lane = Vec2::new(ctx.player_pos.x, lane_y);
    if !escape_side_clear(ctx, on_lane, lane_dir) {
        return None;
    }
    Some((start, end, dir))
}

/// The flatbed's rule: is there a hole in the deck within `clearance` of `at`?
fn pit_near(ctx: &BossContext<'_>, at: Vec2, clearance: f32) -> bool {
    let deck = &ctx.level.trailer;
    if deck.is_empty() {
        return false;
    }
    if deck.pit_at(at).is_some() {
        return true;
    }
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::TAU / 8.0;
        if deck.pit_at(at + Vec2::from_angle(a) * clearance).is_some() {
            return true;
        }
    }
    false
}

/// Is there a clear step out of a circle of `radius` centred `at`?
fn escape_from_circle(ctx: &BossContext<'_>, at: Vec2, radius: f32) -> bool {
    for k in 0..8 {
        let d = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0);
        let p = at + d * radius;
        if ctx.footprint_fits(p, ctx.player_radius) && ctx.los_clear(at, p, ctx.player_radius) {
            return true;
        }
    }
    false
}

/// Is there a clear step off a line with direction `dir` at `at`?
fn escape_side_clear(ctx: &BossContext<'_>, at: Vec2, dir: Vec2) -> bool {
    let perp = dir.perp();
    [1.0f32, -1.0].iter().any(|s| {
        let p = at + perp * (*s * ESCAPE_STEP);
        ctx.los_clear(at, p, ctx.player_radius)
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    //! Narrow tests for the Redtail fight.
    //!
    //! `cargo test boss::hawk` runs these with the framework's suite. Every
    //! attack test drives the shared boss system through `begin_tick` →
    //! `run_system_cached` → `end_tick`, so hazards, contact, the airborne rule
    //! and the death hand-off are the real ones; the suppression tests call the
    //! same `commit_*`/`pick_*` functions the controller calls, with the same
    //! `BossContext` the system builds.

    use super::*;
    use crate::game::art_boss_hawk as art;
    use crate::game::bullet::{Bullet, Faction};
    use crate::game::campaign;
    use crate::game::ecs::systems::projectiles;
    use crate::game::collision::TILE_F;
    use crate::game::level::{Pit, PitKind, Theme};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    /// The `n`th Trailer seed whose pool draw is the hawk. Searched, never
    /// guessed: the pool mix is the framework's, and a test that hard-coded a
    /// seed would silently start fighting the magpie.
    fn hawk_seed(n: usize) -> u64 {
        (0..4096u64)
            .filter(|s| BossKind::pool_pick(Theme::Trailer, *s) == BossKind::Hawk)
            .nth(n)
            .expect("the flatbed's pool must be able to field the hawk")
    }

    fn hawk_world(n: usize) -> World {
        let seed = hawk_seed(n);
        let mut w = World::new(Theme::Trailer, seed);
        assert_eq!(
            BossKind::pool_pick(Theme::Trailer, w.level.seed),
            BossKind::Hawk,
            "this test needs a Trailer seed that fields the hawk"
        );
        w.level_index = campaign::LEVEL_COUNT - 1;
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

    /// Spawn the flatbed's boss through the real spawn system.
    fn spawn_hawk(w: &mut World) -> Boss {
        w.ecs.resource_mut::<BossDue>().due = true;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_spawn_system)
            .expect("boss spawn system must run");
        w.end_tick();
        let boss = boss_of(w).expect("BossDue spawned the hawk");
        assert_eq!(boss.kind, BossKind::Hawk);
        boss
    }

    /// One tick of the shared boss system, plus the rat's i-frame decay that the
    /// player system would otherwise do.
    fn tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(boss_update_system)
            .expect("boss update system must run");
        w.end_tick();
        w.player.iframes = (w.player.iframes - DT).max(0.0);
    }

    /// The damage system alone (player bullets + pools + traps into the boss).
    fn tick_boss_damage(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(projectiles::bullets_system)
            .expect("bullets system");
        w.ecs
            .run_system_cached(boss_damage_system)
            .expect("boss damage system");
        w.end_tick();
    }

    fn set_state(w: &mut World, state: HawkState, duration: f32) {
        with_boss(w, |b| {
            b.set_state(BossState::Hawk(state), duration);
            b.flags = 0;
            b.hurt = 0.0;
        });
    }

    fn state_of(w: &mut World) -> HawkState {
        hawk_state(&boss_of(w).expect("boss alive"))
    }

    /// The arena's centre: the cab hold, which is flat open deck.
    fn arena_centre(w: &World) -> Vec2 {
        w.level.fight_rect_around(w.player.pos).center_world()
    }

    /// Put the bird at `pos` and the rat at `rat`.
    fn place(w: &mut World, pos: Vec2, rat: Vec2) {
        with_boss(w, |b| {
            b.pos = pos;
            b.target = rat;
            b.facing = (rat - pos).normalized();
            b.pounce_origin = pos;
        });
        w.clear_enemies();
        w.player.pos = rat;
        w.player.vel = Vec2::ZERO;
        w.player.iframes = 0.0;
        w.player.loadout.stats.armor = 0.0;
        w.player.hp = w.player.max_hp();
    }

    /// Apply a boss a `commit_*` function prepared (state, marks, latches).
    fn apply_commit(w: &mut World, c: &Boss) {
        with_boss(w, |b| {
            b.pos = c.pos;
            b.target = c.target;
            b.pounce_origin = c.pounce_origin;
            b.facing = c.facing;
            b.set_state(c.state, c.duration);
            b.flags = c.flags;
        });
    }

    fn boss_context<'a>(w: &'a World, rng: &'a mut rand::rngs::StdRng) -> BossContext<'a> {
        let arena = w.level.fight_rect_around(w.player.pos);
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
            arena,
            level: &w.level,
            flow: &w.flow,
            rng,
        }
    }

    /// Tick until the state changes or `limit` seconds pass; returns the first
    /// state the bird entered.
    fn drive_until_change(w: &mut World, limit: f32) -> HawkState {
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

    // ---- registration & pool ---------------------------------------------

    #[test]
    fn the_flatbed_pools_the_magpie_and_the_redtail() {
        let pool = BossKind::pool_for_theme(Theme::Trailer);
        assert_eq!(pool[0], BossKind::Magpie, "the floor's original boss is first");
        assert!(pool.contains(&BossKind::Hawk));
        assert_eq!(pool.len(), 2);
        // The flatbed's *pinned* boss is still the bird that was there first.
        assert_eq!(BossKind::for_theme(Theme::Trailer), BossKind::Magpie);
        assert_eq!(trailer_boss(), BossKind::Magpie);
        // The draw is deterministic and reaches both entries.
        let mut saw_hawk = false;
        let mut saw_magpie = false;
        for seed in 0..64u64 {
            let a = BossKind::pool_pick(Theme::Trailer, seed);
            assert_eq!(a, BossKind::pool_pick(Theme::Trailer, seed));
            saw_hawk |= a == BossKind::Hawk;
            saw_magpie |= a == BossKind::Magpie;
        }
        assert!(saw_hawk && saw_magpie, "the flatbed's draw must reach both birds");
    }

    #[test]
    fn the_spawn_system_fields_the_hawk_for_a_hawk_seed() {
        let mut w = hawk_world(0);
        let boss = spawn_hawk(&mut w);
        assert!(boss.hp > 0.0);
        let expect = art::BOSS_BASE_HP
            * BossKind::Hawk.hp_scale()
            * crate::game::director::boss_hp_mult_for(
                campaign::LEVEL_COUNT - 1,
                crate::game::director::BOSS_TIME,
            );
        assert!((boss.max_hp - expect).abs() < 1.0, "pool {} vs {expect}", boss.max_hp);
        assert_eq!(boss.kind.body_radius(), art::BODY_RADIUS);
        assert_eq!(boss.kind.contact_damage(), art::CONTACT_DAMAGE);
        assert_eq!(boss.kind.death_duration(), art::DEATH_DURATION);
        assert_eq!(boss.kind.cull_margin(), art::CULL_MARGIN);
        // It starts high: the first thing the rat sees is a shadow.
        assert_eq!(boss.state, BossState::Hawk(HawkState::Circle));
        assert!(art::is_overhead(hawk_state(&boss)));
        assert!(boss.hit_shapes().iter().next().is_none());
    }

    #[test]
    fn the_name_plate_names_the_redtail_and_says_overhead() {
        assert_eq!(
            crate::render::hud::boss_codename_kind(BossKind::Hawk),
            (crate::lore::HAWK_NAME, crate::lore::HAWK_CLASS)
        );
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let (name, tag) = crate::render::hud::boss_plate(&w);
        assert_eq!(name, crate::lore::HAWK_NAME);
        assert_eq!(tag, art::OVERHEAD_TAG, "an out-of-reach bird must say so");
        assert!(w.boss_overhead());
        // Once it is down, the plate goes back to the classification tag.
        with_boss(&mut w, |b| {
            b.set_state(BossState::Hawk(HawkState::Hover), art::HOVER_DURATION[0])
        });
        let (name, tag) = crate::render::hud::boss_plate(&w);
        assert_eq!(name, crate::lore::HAWK_NAME);
        assert_eq!(tag, crate::lore::HAWK_CLASS);
        assert!(!w.boss_overhead());
    }

    #[test]
    fn the_phase_thresholds_are_the_arts() {
        assert_eq!(BossKind::Hawk.phase_for(0.99), 1);
        assert_eq!(BossKind::Hawk.phase_for(art::PHASE2_HP_FRAC), 2);
        assert_eq!(BossKind::Hawk.phase_for(0.40), 2);
        assert_eq!(BossKind::Hawk.phase_for(art::PHASE3_HP_FRAC), 3);
        assert_eq!(BossKind::Hawk.phase_for(0.10), 3);
    }

    // ---- the shadow ------------------------------------------------------

    /// Overhead the bird has no hit shapes, so a shot passes through it; the
    /// moment it is down, the same shot lands.
    #[test]
    fn the_overhead_bird_cannot_be_hit_and_the_low_bird_can() {
        let mut w = hawk_world(1);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(40.0, 0.0));
        set_state(&mut w, HawkState::Circle, art::CIRCLE_DURATION[0]);
        let hp0 = boss_of(&mut w).unwrap().hp;
        // Four player darts crossing the body.
        w.clear_bullets();
        for (i, off) in [-8.0f32, -3.0, 2.0, 7.0].iter().enumerate() {
            let mut b = Bullet::new(
                c - Vec2::new(30.0, 0.0) + Vec2::new(0.0, *off),
                Vec2::new(520.0, 0.0),
                40.0,
                Faction::Player,
                (250, 250, 250),
            );
            b.weapon = Some(WeaponKind::Gnaw);
            w.ecs.spawn((b, SpawnId(40 + i as u64)));
        }
        for _ in 0..6 {
            tick_boss_damage(&mut w);
        }
        let hp1 = boss_of(&mut w).unwrap().hp;
        assert!(
            (hp1 - hp0).abs() < 0.01,
            "the shadow must not be hittable ({hp0} -> {hp1})"
        );

        // Down on the deck the same darts land.
        with_boss(&mut w, |b| {
            b.set_state(BossState::Hawk(HawkState::Hover), art::HOVER_DURATION[0]);
            b.pos = c;
        });
        w.clear_bullets();
        for (i, off) in [-8.0f32, -3.0, 2.0, 7.0].iter().enumerate() {
            let mut b = Bullet::new(
                c - Vec2::new(30.0, 0.0) + Vec2::new(0.0, *off),
                Vec2::new(520.0, 0.0),
                40.0,
                Faction::Player,
                (250, 250, 250),
            );
            b.weapon = Some(WeaponKind::Gnaw);
            w.ecs.spawn((b, SpawnId(60 + i as u64)));
        }
        for _ in 0..6 {
            tick_boss_damage(&mut w);
        }
        let hp2 = boss_of(&mut w).unwrap().hp;
        assert!(hp2 < hp1 - 1.0, "a low bird must take the shot ({hp1} -> {hp2})");
    }

    // ---- the dive --------------------------------------------------------

    #[test]
    fn the_dive_lands_after_its_tell_and_only_once() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let rat = c + Vec2::new(30.0, 0.0);
        place(&mut w, c, rat);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::Dive, art::DIVE_DURATION[0]);

        // The tell: nothing lands before the impact. The mark is on the rat
        // until the lock freezes it.
        // (Two ticks of margin: the hazard's own clock starts on the lock tick.)
        let until = art::DIVE_DURATION[0] * art::DIVE_IMPACT_PROGRESS - 2.0 * DT;
        let mut t = 0.0;
        while t < until {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0, "the reticle hurt before the impact");
        }
        let boss = boss_of(&mut w).unwrap();
        assert!(boss.flags & F_LOCK != 0, "the dive never locked");
        assert!(
            (boss.target - rat).len() < 6.0,
            "the reticle should be on the rat ({:?} vs {rat:?})",
            boss.target
        );
        assert!(boss.hit_shapes().iter().next().is_none(), "a dive is still overhead");

        // The rat stands on the mark (a dummy): one hit, the art's number.
        let mut hits = 0;
        for _ in 0..(0.6 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!(
                    (before - w.player.hp - art::DIVE_DAMAGE).abs() < 0.6,
                    "the impact hit for {} not {}",
                    before - w.player.hp,
                    art::DIVE_DAMAGE
                );
            }
            if state_of(&mut w) != HawkState::Dive {
                break;
            }
        }
        assert_eq!(hits, 1, "the impact is a single hit");
        // A dive that connects wheels straight back up.
        assert_eq!(state_of(&mut w), HawkState::Wheel);
    }

    #[test]
    fn a_dodged_dive_buries_the_talons_and_opens_the_nape() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let rat = c + Vec2::new(30.0, 0.0);
        place(&mut w, c, rat);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::Dive, art::DIVE_DURATION[0]);
        // Wait for the lock, then step out of the print (south).
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.flags & F_LOCK == 0).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        w.player.pos = rat + Vec2::new(0.0, art::DIVE_RADIUS + 16.0);
        let mut guard = 0;
        while state_of(&mut w) == HawkState::Dive && guard < 200 {
            guard += 1;
            tick(&mut w);
        }
        assert_eq!(state_of(&mut w), HawkState::TalonBury);
        assert_eq!(w.player.hp, hp0, "the rat stepped off the reticle");
        let boss = boss_of(&mut w).unwrap();
        assert!((boss.damage_mult - art::TALON_BURY_DAMAGE_MULT).abs() < 1e-6);
        assert!(boss.contact_damage <= 0.0, "a buried bird cannot bump");
        assert!((boss.duration - art::TALON_BURY_DURATION[0]).abs() < 1e-3);
        let view = view_of(&boss);
        let (nape, _) = art::weak_point(&view).expect("the nape is exposed");
        assert!(
            boss.hit_shapes().circle_hits_grounded(nape, 1.0),
            "the nape must be the exposed weak point"
        );
        // It lasts exactly its beat, then the bird climbs back out.
        let mut t = 0.0;
        while state_of(&mut w) == HawkState::TalonBury && t < 3.0 {
            tick(&mut w);
            t += DT;
        }
        assert!((t - art::TALON_BURY_DURATION[0]).abs() < 0.10, "bury {t}");
        assert_eq!(state_of(&mut w), HawkState::Wheel);
    }

    #[test]
    fn a_feint_flashes_a_decoy_print_before_the_real_lock() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(40.0, 0.0));
        with_boss(&mut w, |b| {
            b.set_state(BossState::Hawk(HawkState::Dive), art::DIVE_DURATION[0]);
            b.flags = F_FEINT;
            b.swipe_side = 1.0;
        });
        let mut guard = 0;
        while guard < 400 && state_of(&mut w) == HawkState::Dive {
            tick(&mut w);
            guard += 1;
            let boss = boss_of(&mut w).unwrap();
            if boss.flags & F_DECOY != 0 {
                let decoy = Vec2::new(boss.scratch[S_DECOY_X], boss.scratch[S_DECOY_Y]);
                assert!(
                    decoy.dist(w.player.pos) > art::FEINT_OFFSET * 0.5,
                    "the decoy is not the rat's own feet"
                );
                break;
            }
        }
        let boss = boss_of(&mut w).unwrap();
        assert!(boss.flags & F_DECOY != 0, "the feint never jinked");
    }

    // ---- the hover -------------------------------------------------------

    #[test]
    fn the_hover_beats_downdraft_rings_that_push_without_damage() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        // The rat inside the downdraft ring but clear of the body: the ring
        // shoves it outward, the bird does not touch it.
        place(&mut w, c, c + Vec2::new(26.0, 0.0));
        set_state(&mut w, HawkState::Hover, art::HOVER_DURATION[0]);
        let hp0 = w.player.hp;
        let start = w.player.pos;
        let mut pushed = 0.0f32;
        let mut last = start;
        for _ in 0..(art::HOVER_DURATION[0] / DT) as usize {
            tick(&mut w);
            // The downdraft is push-only: no damage, no i-frames.
            assert_eq!(w.player.hp, hp0, "the downdraft must not damage");
            pushed += last.dist(w.player.pos);
            last = w.player.pos;
            w.player.iframes = 0.0;
        }
        assert!(pushed > 3.0, "the rings must shove the rat, moved {pushed:.1} px");
        // The rings are escapable: an outward shove, never a drag toward it.
        assert!(w.player.pos.dist(c) >= start.dist(c) - 1.0);
    }

    #[test]
    fn the_dart_wedge_lands_once_after_its_tell() {
        let mut w = hawk_world(1);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c - Vec2::new(60.0, 0.0), c);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::DartWindup, art::DART_WINDUP[0]);
        // The tell: nothing lands during the windup.
        let until = art::DART_WINDUP[0] * art::DART_LOCK_PROGRESS;
        let mut t = 0.0;
        while t < until - DT {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0, "the darts flew before their tell");
        }
        // Across the lock itself: the wedge is queued on that tick.
        for _ in 0..3 {
            tick(&mut w);
        }
        assert_eq!(w.player.hp, hp0, "the darts flew before their tell");
        assert!(!boss_of(&mut w).unwrap().hazards.is_empty(), "the wedge was never queued");
        // Then one hit inside the wedge.
        let mut hits = 0;
        for _ in 0..(1.4 / DT) as usize {
            let before = w.player.hp;
            tick(&mut w);
            if w.player.hp < before - 1e-3 {
                hits += 1;
                assert!(
                    (before - w.player.hp - art::DART_DAMAGE).abs() < 0.6,
                    "a dart hit for {} not {}",
                    before - w.player.hp,
                    art::DART_DAMAGE
                );
            }
        }
        assert_eq!(hits, 1, "the fan is a single hit");
    }

    #[test]
    fn a_grabbed_rat_is_carried_and_dropped_for_damage() {
        let mut w = hawk_world(2);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let bird = c - Vec2::new(38.0, 0.0);
        place(&mut w, bird, c);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::GrabWindup, art::GRAB_WINDUP[0]);
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.flags & F_LOCK == 0).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        let d0 = w.player.pos.dist(boss_of(&mut w).unwrap().pos);
        let mut biggest = 0.0f32;
        let mut last = w.player.hp;
        let mut t = 0.0;
        while t < art::GRAB_DURATION + art::GRAB_CARRY_SECS + 0.3 {
            tick(&mut w);
            if w.player.hp < last {
                biggest = biggest.max(last - w.player.hp);
                last = w.player.hp;
            }
            t += DT;
        }
        assert!(w.player.hp < hp0, "a held rat must be dropped for damage");
        assert!(
            (biggest - art::GRAB_DAMAGE).abs() < 0.6,
            "the drop is the grab's own number, got {biggest}"
        );
        // It was carried *toward* the bird, not away from it.
        let boss = boss_of(&mut w).unwrap();
        assert!(
            w.player.pos.dist(boss.pos) < d0,
            "the rat was not carried in ({d0:.1} -> {:.1})",
            w.player.pos.dist(boss.pos)
        );
    }

    #[test]
    fn a_rat_that_leaves_the_marked_reach_is_not_grabbed() {
        let mut w = hawk_world(2);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let bird = c - Vec2::new(38.0, 0.0);
        place(&mut w, bird, c);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::GrabWindup, art::GRAB_WINDUP[0]);
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.flags & F_LOCK == 0).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        // Step off the reach (south) and stay there.
        let escaped = w.player.pos + Vec2::new(0.0, 34.0);
        for _ in 0..(1.2 / DT) as usize {
            tick(&mut w);
            w.player.pos = escaped;
        }
        assert_eq!(w.player.hp, hp0, "a rat that left the reach was still hurt");
    }

    #[test]
    fn a_locked_rake_keeps_its_lane_when_the_rat_crosses_behind() {
        // After the lock the windup used to keep re-planning against the rat:
        // a rat that crossed to the bird's other side flipped the facing, and
        // `RakePass` then read itself as already past its end and quit on its
        // first tick, leaving the painted lane live with no bird in it.
        let mut w = hawk_world(3);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c - Vec2::new(30.0, 0.0), c + Vec2::new(30.0, 12.0));
        set_state(&mut w, HawkState::RakeWindup, art::RAKE_WINDUP[0]);
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.flags & F_LOCK == 0).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        let locked = boss_of(&mut w).unwrap();
        assert!(locked.flags & F_LOCK != 0, "the rake never locked");
        assert!(locked.facing.x > 0.0, "the lane runs east, toward the rat");
        // The rat crosses behind the bird, well off the painted lane.
        w.player.pos = c + Vec2::new(-90.0, 44.0);
        guard = 0;
        while state_of(&mut w) == HawkState::RakeWindup && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        assert_eq!(state_of(&mut w), HawkState::RakePass);
        let pass = boss_of(&mut w).unwrap();
        assert!(pass.facing.x > 0.0, "the committed heading flipped");
        assert!(
            (pass.target - locked.target).len() < 0.5,
            "the committed lane end moved after the lock"
        );
        let x0 = pass.pos.x;
        for _ in 0..4 {
            tick(&mut w);
        }
        assert_eq!(
            state_of(&mut w),
            HawkState::RakePass,
            "the pass quit on its first ticks"
        );
        assert!(boss_of(&mut w).unwrap().pos.x > x0 + 2.0, "the bird flies its lane");
    }

    #[test]
    fn the_talon_rake_crosses_the_rat_s_row_and_hits_once() {
        let mut w = hawk_world(3);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let rat = c + Vec2::new(30.0, 12.0);
        place(&mut w, c - Vec2::new(30.0, 0.0), rat);
        let hp0 = w.player.hp;
        set_state(&mut w, HawkState::RakeWindup, art::RAKE_WINDUP[0]);
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.flags & F_LOCK == 0).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        let boss = boss_of(&mut w).unwrap();
        let (start, end) = art::rake_lane(&view_of(&boss));
        assert!(
            (start.y - rat.y).abs() < 20.0 && (end.y - rat.y).abs() < 20.0,
            "the lane {start:?}..{end:?} misses the rat's row {rat:?}"
        );
        let mut hits = 0;
        let last = w.player.hp;
        for _ in 0..(3.0 / DT) as usize {
            tick(&mut w);
            if w.player.hp < last {
                hits += 1;
                assert!(
                    (last - w.player.hp - art::RAKE_DAMAGE).abs() < 0.6,
                    "the rake hit for {} not {}",
                    last - w.player.hp,
                    art::RAKE_DAMAGE
                );
                // The lane has fired; standing under the pass afterwards is
                // body contact, not the rake.
                break;
            }
            if matches!(state_of(&mut w), HawkState::Hover | HawkState::Climb) {
                break;
            }
        }
        assert_eq!(hits, 1, "a rake is a single hit");
        let _ = hp0;
    }

    // ---- phase change & death -------------------------------------------

    #[test]
    fn a_phase_threshold_is_a_harmless_alight() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c - Vec2::new(70.0, 0.0), c);
        with_boss(&mut w, |b| b.hp = b.max_hp * (art::PHASE2_HP_FRAC - 0.01));
        let hp0 = w.player.hp;
        tick(&mut w);
        let boss = boss_of(&mut w).unwrap();
        assert_eq!(boss.phase, 2);
        assert_eq!(hawk_state(&boss), HawkState::Alight);
        assert!((boss.duration - art::ALIGHT_DURATION).abs() < 1e-3);
        assert!(boss.contact_damage <= 0.0, "the alight is harmless");
        assert!(boss.hazards.is_empty(), "the alight queues nothing");
        let mut t = 0.0;
        while state_of(&mut w) == HawkState::Alight && t < 3.0 {
            tick(&mut w);
            t += DT;
            assert_eq!(w.player.hp, hp0);
        }
        assert!((t - art::ALIGHT_DURATION).abs() < 0.08, "alight {t}");
        // It resumes whichever mode it was fighting in (it was overhead).
        assert_eq!(state_of(&mut w), HawkState::Circle);
    }

    #[test]
    fn the_birds_hazards_stop_when_it_dies() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        place(&mut w, c, c + Vec2::new(40.0, 0.0));
        // A live reticle on the deck…
        set_state(&mut w, HawkState::Dive, art::DIVE_DURATION[0]);
        let mut guard = 0;
        while boss_of(&mut w).map(|b| b.hazards.is_empty()).unwrap_or(false) && guard < 300 {
            guard += 1;
            tick(&mut w);
        }
        assert!(!boss_of(&mut w).unwrap().hazards.is_empty());
        // …and the bird killed on it.
        with_boss(&mut w, |b| b.hp = 0.0);
        let hp = w.player.hp;
        tick(&mut w);
        let boss = boss_of(&mut w).expect("the death beat is still playing");
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

    // ---- suppression -----------------------------------------------------

    #[test]
    fn a_dive_at_a_mark_beside_a_hole_is_refused() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let rat = c + Vec2::new(20.0, 0.0);
        place(&mut w, c, rat);
        let mut rng = rand::rngs::StdRng::seed_from_u64(4);
        let mut boss = boss_of(&mut w).unwrap();
        {
            let mut ctx = boss_context(&w, &mut rng);
            assert!(commit_dive(&mut ctx, &mut boss, false), "an open mark is fair");
        }
        // Now put a broken plank under the knockback ring of the next mark.
        set_state(&mut w, HawkState::Circle, art::CIRCLE_DURATION[0]);
        let cell = (
            ((rat.x + art::DIVE_PIT_CLEARANCE - 4.0) / TILE_F).floor() as i32,
            (rat.y / TILE_F).floor() as i32,
        );
        w.level.trailer.pits.push(Pit::new(
            crate::game::level::TileRect::new(cell.0, cell.1, 1, 1),
            PitKind::BrokenPlank,
        ));
        let mut boss = boss_of(&mut w).unwrap();
        let mut ctx = boss_context(&w, &mut rng);
        assert!(
            !commit_dive(&mut ctx, &mut boss, false),
            "the bird must not slam the rat toward a hole"
        );
        // With the hole gone the same mark is a dive again.
        w.level.trailer.pits.clear();
        let mut ctx = boss_context(&w, &mut rng);
        assert!(commit_dive(&mut ctx, &mut boss, false));
        assert_eq!(hawk_state(&boss), HawkState::Dive);
    }

    #[test]
    fn a_grab_with_no_way_out_is_refused() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let c = arena_centre(&w);
        let arena = w.level.fight_rect_around(w.player.pos);
        // A one-cell slot: walls north and south of the rat, so there is no
        // step off the reach.
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + (arena.h / 2) - 1,
            arena.w - 4,
            1,
            crate::game::collision::TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.fill_cells(
            arena.x + 2,
            arena.y + (arena.h / 2) + 1,
            arena.w - 4,
            1,
            crate::game::collision::TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        let row = Vec2::new(
            c.x,
            (arena.y + (arena.h / 2)) as f32 * TILE_F + 8.0,
        );
        // ...and the slot is a dead end right behind him, so backing off the
        // reach is not a step either.
        w.level.tiles.fill_cells(
            ((row.x + 12.0) / TILE_F).floor() as i32 + 1,
            arena.y + (arena.h / 2),
            1,
            1,
            crate::game::collision::TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        place(&mut w, row - Vec2::new(34.0, 0.0), row + Vec2::new(4.0, 0.0));
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let mut boss = boss_of(&mut w).unwrap();
        let mut ctx = boss_context(&w, &mut rng);
        assert!(
            !commit_grab(&mut ctx, &mut boss),
            "a reach the rat cannot leave is not a fair grab"
        );
        // A refused grab leaves the bird in the low fight, not in the tell.
        assert_ne!(hawk_state(&boss), HawkState::GrabWindup);
    }

    // ---- the hunt --------------------------------------------------------

    #[test]
    fn the_bird_hunts_a_rat_that_leaves_the_hold() {
        let mut w = hawk_world(0);
        spawn_hawk(&mut w);
        let centre = arena_centre(&w);
        // The rat out on the deck, west of the cab mouth.
        let deck = w.level.trailer.deck;
        let rat = Vec2::new(
            (deck.x as f32 + 6.0) * TILE_F,
            (deck.y as f32 + deck.h as f32 * 0.5) * TILE_F,
        );
        place(&mut w, centre, rat);
        // The fight rectangle follows a rat that walks far away (batch 12),
        // so the hunt is the band just outside it: a small rectangle two cells
        // from the rat.
        {
            let (rx, ry) = crate::game::collision::cell_of(rat);
            let near = crate::game::level::TileRect::new(rx + 2, ry - 3, 6, 6);
            with_boss(&mut w, |b| b.arena = near);
        }
        w.clear_enemies();
        let d0 = boss_of(&mut w).unwrap().pos.dist(rat);
        for _ in 0..180 {
            w.begin_tick(DT, Vec2::ZERO, false);
            let _ = w
                .ecs
                .run_system_cached(crate::game::ecs::systems::enemy_move::flow_refresh_system);
            let _ = w.ecs.run_system_cached(boss_update_system);
            w.end_tick();
        }
        let boss = boss_of(&mut w).unwrap();
        assert!(boss.hunting, "the bird should be hunting");
        let d1 = boss.pos.dist(rat);
        assert!(
            d1 < d0 - 40.0,
            "the shadow must close on a rat outside the hold ({d0:.0} -> {d1:.0})"
        );
    }

    // ---- determinism -----------------------------------------------------

    #[test]
    fn twin_fights_are_identical() {
        fn run() -> (u64, Vec<(u64, i32, i32)>) {
            let mut w = hawk_world(1);
            spawn_hawk(&mut w);
            w.player.loadout = crate::autoplay::CarryPreset::Typical.loadout();
            let max = w.player.max_hp();
            w.player.hp = max;
            w.player.pos = arena_centre(&w);
            w.clear_enemies();
            let mut trace = Vec::new();
            let mut t = 0.0;
            while t < 24.0 {
                // A fixed scripted orbit with a dash every 2 s.
                let a = t * 1.7;
                let mv = Vec2::from_angle(a);
                let dash = (t * 4.0) as i32 % 8 == 0;
                w.update(DT, mv, dash);
                w.sfx = Vec::new();
                if (t * 60.0) as i32 % 20 == 0 {
                    let boss = boss_of(&mut w).expect("boss alive");
                    trace.push((
                        boss.hp.to_bits() as u64,
                        (boss.pos.x * 16.0) as i32,
                        (boss.pos.y * 16.0) as i32,
                    ));
                }
                t += DT;
            }
            let boss = boss_of(&mut w).expect("boss alive");
            (boss.hp.to_bits() as u64, trace)
        }
        let a = run();
        let b = run();
        assert_eq!(a.0, b.0, "the same fight must leave the same HP");
        assert_eq!(a.1, b.1, "the same fight must walk the same path");
    }

    // ---- the headless fight ---------------------------------------------

    /// The bot fights the bird to the death on the real deck, over five seeds,
    /// draining the sfx queue every tick the way the App does.
    #[test]
    fn a_bot_kills_the_hawk_on_the_real_deck() {
        use crate::autoplay::CarryPreset;
        use crate::game::bot::{Bot, Skill};

        let mut lengths = Vec::new();
        for n in 0..5 {
            let mut w = hawk_world(n);
            spawn_hawk(&mut w);
            // A near-complete build, the way a survivor arrives at 15:00.
            w.player.loadout = CarryPreset::Typical.loadout();
            let max = w.player.max_hp();
            w.player.hp = max;
            w.player.pos = arena_centre(&w);
            w.clear_enemies();
            let seed = w.level.seed;

            let mut bot = Bot::new(Skill::Expert, seed);
            let mut min_frac = 1.0f32;
            let mut t = 0.0f32;
            // Generous: the bird is only reachable while it is low, so a
            // cautious bot spends part of the fight waiting for a dive to
            // miss. The balance pass owns the pool and therefore this bound.
            let limit = 900.0;
            while t < limit && !w.finished() {
                let input = bot.decide(&w, DT);
                w.update(DT, input.move_dir, input.dash);
                w.sfx = Vec::new();
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
                "seed {seed}: the hawk was still alive after {t:.1} s"
            );
            assert!(boss_of(&mut w).is_none(), "seed {seed}: the death beat did not finish");
            lengths.push((t, min_frac));
        }
        let total: f32 = lengths.iter().map(|(s, _)| *s).sum();
        let worst = lengths.iter().map(|(_, h)| *h).fold(1.0f32, f32::min);
        println!(
            "hawk fights: {} seeds, mean {:.1} s, lowest rat hp {:.0}% ({:?})",
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
