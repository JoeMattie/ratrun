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
//! 1. **`Soar`** is the neutral beat and it is **flown**: the bird circles and
//!    banks over the deck around the rat's live position, holding
//!    [`art_boss_magpie::SOAR_RING`] px at [`art_boss_magpie::SOAR_SPEED`], over
//!    cargo and holes, riding the slipstream tailward and flapping back upwind
//!    for it. It is airborne: nothing grounded reaches it, and it does not touch
//!    the rat. Now and then a beat ends in a low wing-over (`Swoop`) past the
//!    rat. It only comes down to work: every attack begins with `Land`, a short
//!    visible descent onto a perch spot in the pecking band.
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
//! A refused move falls through to the cheaper one (`Soar` picks again), and a
//! peck or gust that dies at the bottom of its descent takes off again and
//! keeps flying, so the bird never idles on a move it cannot finish.
//!
//! ## The flight (movement the framework does not own)
//!
//! `Soar`, `Swoop` and `Land` move with [`fly`], **not** [`move_boss`]: a bird
//! crosses cargo stacks and holes in the planks, so the flight is steered
//! straight at a moving target point and clamped only to the map's own edge.
//! Nothing is ever placed: every tick's step is bounded by
//! [`art_boss_magpie::SOAR_SPEED`] (or [`art_boss_magpie::SWOOP_SPEED`]) times
//! `dt`, so the bird is only ever where its own wings carried it.
//!
//! The shared pursuit rule and the siege both step aside for it
//! (`boss::pursue`, `boss_update_system`): the flight is always closing, and a
//! bird is never walled off from the rat it can fly over.
//!
//! ## Determinism
//!
//! Every attack is chosen from a fixed rotation in [`Boss::scratch`] — the
//! rotation, not the rng, decides what comes next. The **flight path** draws
//! from the boss's own [`BossRng`](super::BossRng) stream only (the orbit's
//! ring radius and the side it circles to, once per beat), so the run's rng
//! order is untouched, exactly as the other controllers' own draws are. Nothing
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
/// The ring radius `Soar` is holding this beat (drawn on entry).
const S_FLY_R: usize = 4;
/// Beats flown since the last fly-by: every third beat ends in a `Swoop`.
const S_FLY_BEATS: usize = 5;

// ---- flags layout ---------------------------------------------------------

/// The committed line/lane/crate has been queued.
const F_LOCK: u32 = 1 << 0;
/// The crate has burst (one cue per drop).
const F_CRATE: u32 = 1 << 1;
/// The death beat has taken its one shot (its own bit: `F_LOCK` is the living
/// bird's commit latch and may well be set when it dies).
const F_DEATH: u32 = 1 << 2;
/// This descent (`Land`) ends in a peck tell.
const F_PECK_NEXT: u32 = 1 << 3;
/// This descent (`Land`) ends in the gust.
const F_GUST_NEXT: u32 = 1 << 4;
/// This `Takeoff` was begun to fly a committed lane.
const F_LANE_NEXT: u32 = 1 << 5;
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

/// How far inside the map's own edge a flying bird is kept (px).
const FLY_MARGIN: f32 = 14.0;

/// Where a peck's descent comes down, in px from the rat: the middle of the
/// pecking band first, then nearer, then the band's outer edge.
const PERCH_SPOTS: [f32; 3] = [46.0, 34.0, 58.0];

/// How many bearings the descent tries around the rat (starts on the bird's
/// own side, so the landing is always the short way down).
const PERCH_SPOKES: u32 = 12;

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
        // Whatever it was winding up is cancelled with the move it belonged to
        // (the Mangle's and the Cobra's rule): a beak line, a lane or a crate
        // must not land out of a pose that is no longer attacking.
        if out.phase_changed && !matches!(boss.state, BossState::Magpie(MagpieState::Death)) {
            boss.hazards.clear();
            enter(boss, MagpieState::Ruffle, art_boss_magpie::RUFFLE_DURATION);
            boss.scratch[S_ROT] = 0.0;
        }

        match boss.state {
            BossState::Magpie(MagpieState::Soar) => tick_soar(ctx, boss, out),
            BossState::Magpie(MagpieState::Swoop) => tick_swoop(ctx, boss),
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
            | BossState::NightEdition(_)
            | BossState::HorseshoeCrab(_)
            | BossState::MegaRat(_)
            | BossState::Hawk(_)
            | BossState::Cobra(_)
            | BossState::Lantern(_) => {}
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
        // The flight is the bird's resting state: no cue every beat (it would
        // be one cry every 1.3 s). The wing-over gets the cry instead.
        MagpieState::Soar => None,
        MagpieState::Swoop => Some(BossCue::MagpieCry),
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

/// **The neutral beat, flown.**
///
/// The bird works a ring around the rat's **live** position: the next point on
/// that ring a tick of flight ahead (a carrot), the slipstream dragging it
/// tailward so it always ends up crabbing back upwind, and the whole thing
/// skimmed over cargo and holes because a bird does not care where the planks
/// are. Nothing is remembered about where it started, and nothing steers home:
/// the only position in the steering is `ctx.player_pos`.
fn tick_soar(ctx: &mut BossContext<'_>, boss: &mut Boss, out: &mut BossOutput) {
    let slot = phase_slot(boss);
    let r = boss.scratch[S_FLY_R]
        .clamp(art_boss_magpie::SOAR_RING[0], art_boss_magpie::SOAR_RING[1]);
    let sign = if boss.swipe_side < 0.0 { -1.0 } else { 1.0 };
    let speed = art_boss_magpie::SOAR_SPEED[slot];
    let to = ctx.player_pos - boss.pos;
    let dist = to.len();
    let bearing = if dist > 1e-3 { to * (1.0 / dist) } else { boss.facing };

    // The carrot: the ring point one tick of arc ahead. Chasing a point that is
    // always exactly `r` from the rat is what keeps the ring around the rat
    // instead of a circle around the spot the bird spawned at.
    let arc = if r > 1.0 {
        (speed * ctx.dt / r).clamp(0.0, 0.6)
    } else {
        0.0
    };
    let aim = if dist > r + art_boss_magpie::SOAR_CHASE_BAND {
        // Out of the ring: come straight at the rat, at full flight speed,
        // until the ring has it again (the arrival from across the deck).
        ctx.player_pos
    } else {
        // `bearing` points from the bird to the rat, so the ring point on the
        // bird's own side is the rat *minus* it (turned one step of arc). Added,
        // the carrot sat on the far side and the bird flew through the rat and
        // hovered on top of it.
        ctx.player_pos - bearing.rotate(arc * sign) * r
    };
    // The wind, as a velocity: the slipstream drags everything tailward, so the
    // bird's ground track trails the carrot and it flies a little upwind for it.
    let want = (aim - boss.pos) * (1.0 / ctx.dt.max(1e-4))
        - Vec2::new(art_boss_magpie::SOAR_DRIFT, 0.0);
    // Stay in the fight. The rectangle is rebuilt around the rat, so a ring
    // flown at a rat inside it is inside it; this is the edge case (the bird is
    // outside, or a step of the arc would leave). It turns back at the rat,
    // never at a remembered spot: the rat is what the rectangle is centred on.
    let mut dir = want;
    if ctx.arena.contains_world(ctx.player_pos)
        && (!ctx.arena.contains_world(boss.pos)
            || !ctx.arena.contains_world(boss.pos + want.normalized() * 10.0))
    {
        dir = to;
    }
    fly(ctx, boss, dir, speed);
    if dir.len_sq() > 1e-6 {
        boss.facing = dir.normalized();
    }
    boss.target = ctx.player_pos;
    if boss.progress() >= 1.0 {
        // Every third beat ends in a wing-over instead of a turn.
        let beats = boss.scratch[S_FLY_BEATS] + 1.0;
        if beats >= 3.0 {
            boss.scratch[S_FLY_BEATS] = 0.0;
            enter_swoop(ctx, boss);
        } else {
            boss.scratch[S_FLY_BEATS] = beats;
            next_attack(ctx, boss, out);
        }
    }
}

/// **The wing-over.** A straight, fast pass on a line that crosses safely
/// abeam of the rat, then back up into the ring.
///
/// Nothing is queued and nothing is dealt: it is a fly-by, not an attack. The
/// pass is aimed at a point [`art_boss_magpie::SWOOP_PASS_OFFSET`] px off the
/// rat's shoulder, which keeps the whole chord further from the rat than the
/// art's own offset (the pass reads without ever being a hit the rat cannot
/// explain).
fn tick_swoop(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let to = boss.target - boss.pos;
    if to.len() > 1.0 {
        fly(ctx, boss, to, art_boss_magpie::SWOOP_SPEED[slot]);
        boss.facing = to.normalized();
    }
    boss.pounce_origin = boss.pos;
    if boss.progress() >= 1.0 {
        enter_soar(ctx, boss);
    }
}

/// **Free flight.** A bird crosses cargo stacks and holes in the planks; it
/// cannot leave the map. The step is bounded by `speed * dt` (through
/// [`Boss::slow_mult`] like every other move), so the bird is only ever where
/// its own wings carried it.
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

/// Fly up into the neutral ring: draw this beat's radius and the side it will
/// circle to, from the boss's own stream (never the run's).
fn enter_soar(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let slot = phase_slot(boss);
    let lo = art_boss_magpie::SOAR_RING[0];
    let hi = art_boss_magpie::SOAR_RING[1];
    boss.scratch[S_FLY_R] = lo + (hi - lo) * ctx.rng.random::<f32>();
    boss.swipe_side = if ctx.rng.random::<f32>() < 0.5 { -1.0 } else { 1.0 };
    boss.pounce_origin = boss.pos;
    boss.target = ctx.player_pos;
    enter(boss, MagpieState::Soar, art_boss_magpie::SOAR_DURATION[slot]);
}

/// Plan and start the low pass.
fn enter_swoop(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    let sign = if boss.swipe_side < 0.0 { -1.0 } else { 1.0 };
    let to = ctx.player_pos - boss.pos;
    let bearing = if to.len_sq() > 1e-3 { to.normalized() } else { boss.facing };
    // Abeam of the rat, a run further along the orbit's own direction.
    let abeam = bearing.perp() * (sign * art_boss_magpie::SWOOP_PASS_OFFSET);
    let ahead = bearing.perp() * (sign * art_boss_magpie::SWOOP_RUN);
    boss.pounce_origin = boss.pos;
    boss.target = ctx.player_pos + abeam + ahead;
    boss.facing = (boss.target - boss.pos).normalized();
    enter(boss, MagpieState::Swoop, art_boss_magpie::SWOOP_DURATION);
}

/// Where a peck's descent comes down: a spot in the pecking band around the
/// rat, on a bearing close to the bird's own side of it, where the bird's body
/// fits and the line to the rat is clear.
fn perch_spot(ctx: &BossContext<'_>, boss: &Boss) -> Option<Vec2> {
    let to = ctx.player_pos - boss.pos;
    let base = if to.len_sq() > 1e-3 { to.normalized() } else { boss.facing };
    let mut best: Option<(f32, Vec2)> = None;
    for dist in PERCH_SPOTS {
        for k in 0..PERCH_SPOKES {
            // 0, +1, -1, +2, -2 ... spokes: the short way down first.
            let step = k.div_ceil(2) as f32 * (std::f32::consts::TAU / PERCH_SPOKES as f32);
            let off = if k % 2 == 1 { step } else { -step };
            let spot = ctx.player_pos - base.rotate(off) * dist;
            if !ctx.footprint_fits(spot, art_boss_magpie::BODY_RADIUS) {
                continue;
            }
            if !ctx.los_clear(spot, ctx.player_pos, 1.0) {
                continue;
            }
            let travel = boss.pos.dist(spot);
            if best.is_none_or(|(d, _)| travel < d) {
                best = Some((travel, spot));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|(_, spot)| spot)
}

/// Start the descent that ends in a peck, or refuse it (and keep flying).
fn start_peck_descent(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let Some(spot) = perch_spot(ctx, boss) else {
        return false;
    };
    descend_to(boss, spot, F_PECK_NEXT);
    true
}

/// Start the descent that ends in the gust. The gust is a push field, not a
/// line, so the bird only needs somewhere on the deck to brace itself — it
/// comes down where it is. The descent's own end re-checks the rule
/// ([`commit_gust`]).
fn start_gust_descent(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if !gust_allowed(ctx, boss) {
        return false;
    }
    let spot = boss.pos;
    descend_to(boss, spot, F_GUST_NEXT);
    true
}

/// Fly down onto `spot` over a beat that is the distance over the bird's own
/// flight speed (held inside [`art_boss_magpie::DESCENT_MIN`]..
/// [`art_boss_magpie::DESCENT_MAX`]), so the descent is visible, short, and
/// never faster than the bird can fly.
fn descend_to(boss: &mut Boss, spot: Vec2, intent: u32) {
    let slot = phase_slot(boss);
    let dist = boss.pos.dist(spot);
    let secs = (dist / art_boss_magpie::SOAR_SPEED[slot].max(1.0)).clamp(
        art_boss_magpie::DESCENT_MIN,
        art_boss_magpie::DESCENT_MAX,
    );
    boss.pounce_origin = boss.pos;
    boss.target = spot;
    let to = spot - boss.pos;
    if to.len_sq() > 1e-6 {
        boss.facing = to.normalized();
    }
    enter(boss, MagpieState::Land, secs);
    boss.flags |= intent;
}

fn tick_ruffle(_ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // The phase-change pose: no contact, no attack, free damage (the art rule
    // `has_body_contact` owns the contact side).
    if boss.progress() >= 1.0 {
        // Back to work, and back into the air: the flare happens on the deck,
        // so the bird climbs out of it.
        enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
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
            // refused and the bird climbs back out instead of stabbing at
            // nothing.
            enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
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
        // The punish window is over: the bill comes out and the bird climbs back
        // into the ring. It does not hop away from where it landed.
        enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
    }
}

fn tick_takeoff(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    if boss.progress() >= 1.0 {
        if boss.flags & F_LANE_NEXT == 0 {
            // A climb-out (out of a peck, or out of the ruffle): back into the
            // air, where the neutral beat is flown.
            enter_soar(ctx, boss);
            return;
        }
        // The lane is planned on entry so the art paints it on the first frame
        // of the windup; `queue_lane` re-plans with the same inputs at the lock.
        let slot = phase_slot(boss);
        plan_lane(ctx, boss);
        enter(boss, MagpieState::StrafeWindup, art_boss_magpie::STRAFE_WINDUP[slot]);
    }
}

fn tick_strafe_windup(ctx: &mut BossContext<'_>, boss: &mut Boss) {
    // Hover onto the lane's line (it passes through the rat), then hold. The
    // bird is airborne, so it crosses a cargo stack to reach its own lane.
    let lane_y = boss.scratch[S_LANE_Y];
    let dy = lane_y - boss.pos.y;
    if dy.abs() > 1.0 {
        fly(ctx, boss, Vec2::new(0.0, dy), HOVER_SPEED);
    }
    let dir = boss.scratch[S_LANE_DIR];
    boss.facing = Vec2::new(dir.signum(), 0.0);
    boss.target = Vec2::new(boss.target.x, lane_y);
    if boss.flags & F_LOCK == 0 && boss.progress() >= 0.55 {
        // The lane lost its fit between the takeoff and the lock (the rat
        // walked behind cargo, or into a spot with no step off the lane): the
        // pass is refused. Flying it anyway would be a pass with no live lane
        // and no tell, so the bird repositions instead.
        if !queue_lane(ctx, boss) {
            // Already in the air: give the turn back to the ring.
            enter_soar(ctx, boss);
            return;
        }
        boss.flags |= F_LOCK;
    }
    if boss.progress() >= 1.0 {
        // Defensive: a windup that somehow ends unlocked never flies.
        if boss.flags & F_LOCK == 0 {
            enter_soar(ctx, boss);
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
    // Flown, not walked: the lane is deck the bird's body fits along, and it is
    // flown at the art's pass speed either way.
    fly(ctx, boss, dir, art_boss_magpie::STRAFE_SPEED[slot]);
    let past = (boss.pos.x - boss.target.x) * dir.x >= 0.0;
    if past || boss.progress() >= 1.0 {
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

/// **The descent.** A flying bird comes down onto a spot in its own pecking
/// band (or, after a wing-over, settles where it stopped), then the beat it
/// descended for begins: the unchanged peck tell, the gust, or back up into the
/// ring.
fn tick_land(ctx: &mut BossContext<'_>, boss: &mut Boss) {
let intent = boss.flags & (F_PECK_NEXT | F_GUST_NEXT);
if intent != 0 {
// Glide the rest of the way at the bird's own flight speed. The beat was
// sized to the distance, so this arrives as the beat ends.
let slot = phase_slot(boss);
let to = boss.target - boss.pos;
if to.len() > 1.0 {
fly(ctx, boss, to, art_boss_magpie::SOAR_SPEED[slot]);
}
}
    if boss.progress() < 1.0 {
        return;
    }
    if boss.flags & F_PECK_NEXT != 0 {
        // The peck's own fairness rule decides at the bottom of the descent; a
        // refused line means the bird climbs straight back out.
        if !commit_peck(ctx, boss) {
            enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
        }
        return;
    }
    if boss.flags & F_GUST_NEXT != 0 {
        // The deck moved under it on the way down (the rat walked a hole into
        // the gust's path, or the bird came down on cargo): refused, and it
        // climbs back out.
        if !commit_gust(ctx, boss) {
            enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
        }
        return;
    }
    // A settle after the wing-over's crate: climb back out.
    enter(boss, MagpieState::Takeoff, art_boss_magpie::TAKEOFF_DURATION);
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
    // Fall through whatever the floor refuses, then keep flying.
    //
    // A peck and a gust need the deck underfoot, so they are started with a
    // **descent** (`Land`) and the tell begins at the bottom of it; a strafe is
    // flown from the air, and its `Takeoff` is the climb into the lane.
    let mut pick = want;
    for _ in 0..3 {
        let ok = match pick {
            Attack::Peck => start_peck_descent(ctx, boss),
            Attack::Strafe => commit_strafe(ctx, boss),
            Attack::Gust => start_gust_descent(ctx, boss),
        };
        if ok {
            return;
        }
        pick = match pick {
            Attack::Peck => {
                // The peck is the cheapest and the last resort of the three; a
                // refused peck means the rat is not reachable from the air —
                // keep the ring and try again on the next beat.
                enter_soar(ctx, boss);
                return;
            }
            Attack::Strafe => Attack::Peck,
            Attack::Gust => Attack::Peck,
        };
    }
    enter_soar(ctx, boss);
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
    // `enter` clears the latch word, so the takeoff learns its job after it.
    boss.flags |= F_LANE_NEXT;
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

/// Queue the pass's live corridor once the windup has shown it. Returns
/// `false` (and queues nothing) when no fair lane exists any more.
fn queue_lane(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    let slot = phase_slot(boss);
    let Some((start, end, dir)) = lane_plan(ctx, boss) else {
        return false;
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
    true
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

/// May the bird beat its wings down here? It needs deck under it to brace on,
/// and it will **never** blow the rat down a hole it cannot read (the pulse is
/// refused when the plank behind the rat is already a hole).
fn gust_allowed(ctx: &BossContext<'_>, boss: &Boss) -> bool {
    gust_pit_ahead(ctx).is_none()
        && ctx.footprint_fits(boss.pos, art_boss_magpie::BODY_RADIUS)
}

/// Start a gust, from the deck (the flight comes down first: see
/// [`start_gust_descent`]).
fn commit_gust(ctx: &mut BossContext<'_>, boss: &mut Boss) -> bool {
    if !gust_allowed(ctx, boss) {
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

    /// A flatbed seed that draws **the magpie**.
    ///
    /// The trailer's boss pool holds two birds now (`BossKind::pool_for_theme`:
    /// the magpie and the Redtail), and `boss_spawn_system` picks by the level
    /// seed, so a suite pinned to arbitrary seeds would spend half its tests
    /// fighting the hawk's file. Seeds that already draw the magpie keep their
    /// layout; the rest are mapped onto one of the magpie seeds, keeping the
    /// spread the seeds were chosen for.
    fn magpie_seed(seed: u64) -> u64 {
        if BossKind::pool_pick(Theme::Trailer, seed) == BossKind::Magpie {
            return seed;
        }
        (0..4096u64)
            .filter(|s| BossKind::pool_pick(Theme::Trailer, *s) == BossKind::Magpie)
            .nth((seed % 32) as usize)
            .unwrap_or(seed)
    }

    fn trailer_world(seed: u64) -> World {
        let mut w = World::with_tuning(
            Theme::Trailer, magpie_seed(seed), crate::tuning::RunTuning::with_base_boss_hazards(BossKind::Magpie),
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
        w.level.fight_rect_around(w.player.pos).center_world()
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
        let _ = arena;
        let boss_arena = w.level.fight_rect_around(w.player.pos);
        BossContext {
            dt: DT,
            time: 0.0,
            player_pos: w.player.pos,
            player_vel: Vec2::ZERO,
            player_radius: PLAYER_RADIUS,
            player_hurt_radius: PLAYER_RADIUS,
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
        let expect = BossKind::Magpie.hp_pool(w.level_index());
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
        let arena = w.level.fight_rect_around(w.player.pos);
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
            if matches!(state_of(&mut w), MagpieState::Takeoff | MagpieState::Soar) {
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
        // The bill comes out and the bird climbs back into the ring: it does
        // not hop away from where it landed.
        assert_eq!(state_of(&mut w), MagpieState::Takeoff);
    }

    #[test]
    fn a_cornered_bird_stays_stuck_longer() {
        let mut w = trailer_world(13);
        spawn_magpie(&mut w);
        // A cargo column right behind the bird: it can lunge but it cannot get
        // any purchase to pull its bill back out.
        // Built where the other fixtures stand (the fight rectangle's
        // centre): a column two cells west of it, seven cells tall.
        let centre = arena_centre(&w);
        let (cx, cy) = crate::game::collision::cell_of(centre);
        w.level.tiles.fill_cells(
            cx - 2,
            cy - 3,
            1,
            7,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        let row = cy as f32 * TILE_F + TILE_F * 0.5;
        // 24 px east of the cargo face: the back-probe reaches into it.
        let wall_x = (cx - 1) as f32 * TILE_F + 24.0;
        place(
            &mut w,
            Vec2::new(wall_x, row),
            Vec2::new(wall_x + 44.0, row),
        );
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
        let arena = w.level.fight_rect_around(w.player.pos);
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
        let before = magpie_state(&boss);
        let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
        assert!(
            !commit_peck(&mut ctx, &mut boss),
            "a line the rat cannot leave is not a fair attack"
        );
        assert_eq!(magpie_state(&boss), before, "a refused peck changes nothing");
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
        // The flare happens on the deck, so the bird climbs out of it.
        assert_eq!(state_of(&mut w), MagpieState::Takeoff);
    }

    /// A phase threshold cancels the move it interrupts: a beak line that was
    /// already on the planks must not strike out of the harmless ruffle.
    #[test]
    fn a_ruffle_cancels_the_telegraph_it_interrupts() {
        let mut w = trailer_world(19);
        spawn_magpie(&mut w);
        let centre = arena_centre(&w);
        place(&mut w, centre, centre + Vec2::new(40.0, 0.0));
        set_state(&mut w, MagpieState::PeckWindup, art::PECK_WINDUP[0]);
        let mut t = 0.0;
        while boss_of(&mut w).expect("alive").hazards.is_empty() && t < 1.0 {
            tick(&mut w);
            t += DT;
        }
        assert!(!boss_of(&mut w).expect("alive").hazards.is_empty());
        // Cross the first threshold with the line still warning.
        with_boss(&mut w, |b| b.hp = b.max_hp * (art::PHASE2_HP_FRAC - 0.01));
        tick(&mut w);
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(magpie_state(&boss), MagpieState::Ruffle);
        assert!(
            boss.hazards.is_empty(),
            "the interrupted peck's line is still on the planks"
        );
    }

    /// A lane that stops being fair between the takeoff and the lock is
    /// refused: the bird goes back to its ring. It used to latch the lock
    /// anyway and fly the pass with no live lane and no tell.
    #[test]
    fn a_lane_that_loses_its_fit_is_never_flown() {
        let mut w = trailer_world(21);
        spawn_magpie(&mut w);
        let arena = w.level.fight_rect_around(w.player.pos);
        let row = arena_centre(&w).y;
        // 40 px short of the lane's east end with the rat further east still:
        // the only run left is shorter than `STRAFE_MIN_LEN`, so `lane_plan`
        // has nothing to offer at the lock.
        let east_end = arena.right() as f32 * TILE_F - art::STRAFE_END_MARGIN;
        let bird = Vec2::new(east_end - art::STRAFE_MIN_LEN * 0.5, row);
        place(&mut w, bird, bird + Vec2::new(30.0, 0.0));
        set_state(&mut w, MagpieState::StrafeWindup, art::STRAFE_WINDUP[0]);
        with_boss(&mut w, |b| {
            b.scratch[S_LANE_Y] = row;
            b.scratch[S_LANE_DIR] = 1.0;
        });
        let mut t = 0.0;
        while state_of(&mut w) == MagpieState::StrafeWindup && t < 2.0 {
            tick(&mut w);
            t += DT;
        }
        let boss = boss_of(&mut w).expect("alive");
        assert_eq!(
            magpie_state(&boss),
            MagpieState::Soar,
            "a refused lane must fall back to the ring, not a pass"
        );
        assert!(boss.hazards.is_empty(), "no lane was queued");
        assert!(
            t < art::STRAFE_WINDUP[0] - 0.05,
            "the refusal happens at the lock, not at the end of the windup ({t})"
        );
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
        // The fight rectangle follows a rat that walks far away (batch 12),
        // so the hunt is the band just outside it: a small rectangle two cells
        // from the rat.
        {
            let (rx, ry) = crate::game::collision::cell_of(rat);
            let near = crate::game::level::TileRect::new(rx + 2, ry - 3, 6, 6);
            with_boss(&mut w, |b| b.arena = near);
        }
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

    // ---- the flight -------------------------------------------------------

    /// The bird's own fastest travel in one tick, with the tolerance the
    /// steering's arithmetic needs. A placement (a pop-in, a rejoin teleport)
    /// would be one to two orders of magnitude larger.
    fn max_step() -> f32 {
        art::PECK_LUNGE_SPEED
            .max(art::SWOOP_SPEED[2])
            .max(art::STRAFE_SPEED[2])
            * DT
            * 1.06
    }

    /// **The owner's regression.** The bird used to be tethered to the deck it
    /// arrived on: it lunged and hopped back, and a rat that walked away left it
    /// behind ("it never moves from where it spawned except to leap forward and
    /// attack and then go back"). The ring is built on the rat's **live**
    /// position every tick, so the bird walks the deck with it — and nothing it
    /// does moves it further in one tick than its own wings can.
    #[test]
    fn the_flight_follows_a_rat_that_walks_the_deck() {
        let mut w = trailer_world(37);
        spawn_magpie(&mut w);
        w.clear_enemies();
        w.player.loadout.stats.max_hp = 1.0e6;
        w.player.hp = 1.0e6;
        let centre = arena_centre(&w);
        // Down the bed from the tailgate: 300 px of the deck, straight through
        // whatever cargo the seed stacked on it.
        let start = centre - Vec2::new(150.0, 0.0);
        // The bird starts 180 px off, across the bed: the fight rectangle is
        // clamped to the map's west edge here, so 180 px *behind* the rat is
        // off the map (and the flight's own map clamp would be the first step).
        place(&mut w, start - Vec2::new(0.0, 180.0), start);
        let mut prev = boss_of(&mut w).expect("alive").pos;
        let step = 300.0 / (10.0 / DT);
        let mut flown = 0.0f32;
        let mut worst = 0.0f32;
        for i in 0..(10.0 / DT) as usize {
            // The rat walks 300 px east over the 10 s. It is a scripted dummy:
            // it holds its line whatever the fight does to it, so the test
            // measures the bird and not the knockback.
            w.player.pos = start + Vec2::new(step * (i + 1) as f32, 0.0);
            tick(&mut w);
            let boss = boss_of(&mut w).expect("alive");
            let moved = boss.pos.dist(prev);
            assert!(
                moved <= max_step(),
                "tick {i}: the bird travelled {moved:.2} px in {DT} s"
            );
            flown += moved;
            prev = boss.pos;
            if (i + 1) as f32 * DT >= 3.0 {
                worst = worst.max(boss.pos.dist(w.player.pos));
            }
        }
        assert!(
            worst <= 120.0,
            "the bird fell {worst:.0} px behind a rat walking the deck"
        );
        assert!(flown >= 300.0, "the bird only flew {flown:.0} px in 10 s");
    }

    /// **The neutral beat is flight.** Over 20 s the bird covers real ground,
    /// and outside its attacks it holds the ring around the rat: `SOAR_RING`
    /// (56-84 px) with the chase band's own ceiling.
    ///
    /// The sample starts a moment into the beat (progress `>= 0.5`): after an
    /// attack the bird is where that attack left it — the peck's lunge parks it
    /// about 25 px off the rat's shoulder — and it flies back out to the ring
    /// from there.
    #[test]
    fn the_neutral_beat_is_flown() {
        let mut w = trailer_world(38);
        spawn_magpie(&mut w);
        w.clear_enemies();
        w.player.loadout.stats.max_hp = 1.0e6;
        w.player.hp = 1.0e6;
        let centre = arena_centre(&w);
        place(&mut w, centre - Vec2::new(100.0, 0.0), centre);
        let mut prev = boss_of(&mut w).expect("alive").pos;
        let mut flown = 0.0f32;
        let mut nearest = f32::MAX;
        let mut furthest = 0.0f32;
        for _ in 0..(20.0 / DT) as usize {
            tick(&mut w);
            let boss = boss_of(&mut w).expect("alive");
            let moved = boss.pos.dist(prev);
            assert!(moved <= max_step(), "the bird travelled {moved:.2} px in a tick");
            flown += moved;
            prev = boss.pos;
            if matches!(state_of(&mut w), MagpieState::Soar | MagpieState::Swoop)
                && boss.progress() >= 0.5
            {
                let d = boss.pos.dist(w.player.pos);
                nearest = nearest.min(d);
                furthest = furthest.max(d);
            }
        }
        assert!(flown >= 400.0, "the bird only covered {flown:.0} px in 20 s");
        assert!(
            furthest <= 140.0,
            "the ring drifted out to {furthest:.0} px from the rat"
        );
        assert!(
            nearest >= 40.0,
            "the ring closed to {nearest:.0} px from the rat"
        );
    }

    /// **A bird crosses cargo and holes.** The neutral beat used to walk: a
    /// cargo stack between it and the rat was a wall, and a hole in the planks a
    /// detour — which is how a moving rat left the boss behind. The flight is
    /// straight over both.
    #[test]
    fn the_flight_crosses_cargo_and_a_hole() {
        let mut w = trailer_world(39);
        spawn_magpie(&mut w);
        w.clear_enemies();
        w.player.loadout.stats.max_hp = 1.0e6;
        w.player.hp = 1.0e6;
        let centre = arena_centre(&w);
        let rat = centre;
        let (cx, cy) = crate::game::collision::cell_of(centre);
        // A cargo column three cells west of the rat, and a hole in the planks
        // in front of it: neither is anything to a bird in the air.
        w.level.tiles.fill_cells(
            cx - 3,
            cy - 4,
            1,
            9,
            TileKind::Wall(crate::game::art_walls::WallKind::Solid),
        );
        w.level.tiles.rebuild_masks();
        w.level.trailer.pits.push(Pit::new(
            crate::game::level::TileRect::new(cx - 6, cy, 2, 1),
            PitKind::BrokenPlank,
        ));
        place(&mut w, centre - Vec2::new(200.0, 0.0), rat);
        let d0 = boss_of(&mut w).expect("alive").pos.dist(rat);
        assert!(d0 > 190.0, "the fixture starts the bird across the cargo ({d0})");
        let mut d1 = d0;
        for _ in 0..(4.0 / DT) as usize {
            tick(&mut w);
            d1 = boss_of(&mut w).expect("alive").pos.dist(rat);
            if d1 <= art::SOAR_RING[1] + art::SOAR_CHASE_BAND {
                break;
            }
        }
        assert!(
            d1 <= art::SOAR_RING[1] + art::SOAR_CHASE_BAND,
            "the bird never crossed the cargo and the hole ({d0:.0} -> {d1:.0})"
        );
    }

    /// **The wing-over is a fly-by, not an attack.** It queues nothing, deals
    /// nothing, and passes the rat no closer than [`art::SWOOP_PASS_OFFSET`] px
    /// abeam, then climbs back into the ring.
    #[test]
    fn a_wing_over_passes_the_rat_and_deals_nothing() {
        let mut w = trailer_world(40);
        spawn_magpie(&mut w);
        w.clear_enemies();
        let centre = arena_centre(&w);
        place(&mut w, centre - Vec2::new(70.0, 0.0), centre);
        // Plan the pass with the controller's own planner.
        let (target, facing) = {
            let mut boss = boss_of(&mut w).expect("alive");
            boss.swipe_side = 1.0;
            let mut rng = rand::rngs::StdRng::seed_from_u64(9);
            let mut ctx = boss_context(&w, &mut rng, crate::game::level::TileRect::new(0, 0, 8, 8));
            enter_swoop(&mut ctx, &mut boss);
            assert_eq!(magpie_state(&boss), MagpieState::Swoop);
            (boss.target, boss.facing)
        };
        with_boss(&mut w, |b| {
            b.target = target;
            b.facing = facing;
        });
        set_state(&mut w, MagpieState::Swoop, art::SWOOP_DURATION);
        let mut nearest = f32::MAX;
        let mut queued = false;
        let mut t = 0.0f32;
        while state_of(&mut w) == MagpieState::Swoop && t < art::SWOOP_DURATION + 0.5 {
            tick(&mut w);
            t += DT;
            let boss = boss_of(&mut w).expect("alive");
            queued |= !boss.hazards.is_empty();
            nearest = nearest.min(boss.pos.dist(w.player.pos));
        }
        assert!((t - art::SWOOP_DURATION).abs() < 0.08, "swoop beat {t}");
        assert!(!queued, "a wing-over must queue no hazard");
        assert!(
            nearest >= 40.0,
            "the pass came within {nearest:.0} px of the rat"
        );
        assert_eq!(state_of(&mut w), MagpieState::Soar);
    }

    /// **Flatbed, eight seeds, both birds, the scheduled arrival.** The
    /// director's `BOSS_TIME` call spawns the boss out of sight; from there it
    /// must close to its own engagement range inside 8 s, with every step
    /// bounded by its own speed (the Magpie's; the Redtail's dive is a drawn
    /// interpolation of its own art and is not this fix's to change), and its
    /// spawn must be on the deck, on the horde's connected floor, and inside the
    /// 34 x 28 fight rectangle.
    #[test]
    fn every_flatbed_bird_arrives_on_the_deck_and_reaches_the_rat() {
        use crate::game::ecs::systems::spawn::SpawnScratch;
        for seed in [3u64, 8, 17, 24, 41, 77, 128, 260] {
            for kind in [BossKind::Magpie, BossKind::Hawk] {
                let mut w = World::with_tuning(
                    Theme::Trailer,
                    seed,
                    crate::tuning::RunTuning::with_base_boss_hazards(kind),
                );
                w.level_index = campaign::LEVEL_COUNT - 1;
                // The App's own default viewport, so the arrival's ring is the
                // one a real run measures.
                w.viewport = Vec2::new(120.0, 74.0);
                w.player.loadout.stats.max_hp = 1.0e6;
                w.player.hp = 1.0e6;
                w.boss_override = Some(kind);
                w.elapsed = crate::game::director::BOSS_TIME;
                w.update(DT, Vec2::ZERO, false);
                let boss = boss_of(&mut w).unwrap_or_else(|| {
                    panic!("{kind:?} seed {seed}: the schedule spawned no boss")
                });
                assert_eq!(boss.kind, kind, "seed {seed}: the override was ignored");
                // On the deck, on the horde's floor, inside the rectangle.
                let (cx, cy) = crate::game::collision::cell_of(boss.pos);
                assert!(
                    w.level.trailer.deck.contains(cx, cy),
                    "{kind:?} seed {seed}: arrival off the deck at {cx},{cy}"
                );
                let mut scratch = SpawnScratch::default();
                scratch.sync(&w.level);
                assert!(
                    scratch.is_reachable(&w.level, boss.pos),
                    "{kind:?} seed {seed}: arrival off the horde's floor"
                );
                assert!(
                    boss.arena.contains_world(boss.pos),
                    "{kind:?} seed {seed}: arrival outside the fight rectangle"
                );
                // Eight seconds of the boss system (the rat stands still and is
                // invulnerable here: the fix is the boss's own closure, and the
                // whole schedule at `BOSS_TIME` would spend the window spawning
                // a horde for each of the sixteen fights).
                let mut prev = boss.pos;
                let mut reached = false;
                for _ in 0..(8.0 / DT) as usize {
                    tick(&mut w);
                    let Some(b) = boss_of(&mut w) else { break };
                    if kind == BossKind::Magpie {
                        let moved = b.pos.dist(prev);
                        assert!(
                            moved <= max_step(),
                            "{kind:?} seed {seed}: travelled {moved:.2} px in one tick"
                        );
                    }
                    prev = b.pos;
                    if b.pos.dist(w.player.pos) <= kind.engage_range() {
                        reached = true;
                    }
                }
                assert!(
                    reached,
                    "{kind:?} seed {seed}: never reached engagement range in 8 s"
                );
            }
        }
    }
}
