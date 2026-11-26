//! Wave 4: four ordinary enemies that each ask a question the other ten do not
//! (`WAVE3_DESIGN.md` §9 is the wave-3 grammar this follows; the wave-4 table
//! lives in this module's docs and in the task report).
//!
//! | Kind | Question it asks | Tell | Resolves into |
//! |---|---|---|---|
//! | Big Snake | "the room is split — go round, or dash over" | the head rears and a [`SNAKE_WARN_LINE`] line is drawn along the locked aim for [`SNAKE_TELL_SECS`] | a [`SNAKE_LUNGE_DISTANCE`] lunge, then [`SNAKE_STRETCH_SECS`] lying stretched and still (the punish window) |
//! | House Centipede | "it bolts at a wall the moment you touch it" | the legs flutter in place for [`CENTIPEDE_FLUTTER_SECS`] with a dotted dash line | a [`CENTIPEDE_DART_SPEED`] straight burst; any hit makes it flee for [`CENTIPEDE_FLEE_SECS`] |
//! | Murder Hornet | "it is only safe to punish after a miss" | a rising buzz for [`HORNET_CHARGE_SECS`], then a locked dive line for [`HORNET_TELL_SECS`] | a straight dive for a [`HORNET_JAB_DAMAGE`] jab, then [`HORNET_RECOVER_SECS`] grounded at ×[`HORNET_RECOVER_MULT`] damage taken |
//! | Termite Horde | "single-target darts only chip it; bring area damage" | none (it is a slow flood, not an attack) | a per-hit damage cap ([`HORDE_HIT_CAP`]), a split on a hard enough hit and slow regrowth |
//!
//! ## Why one system, placed after the wave-3 pass
//!
//! The schedule puts this directly after `enemy_wave3::wave3_system` and before
//! `trailer::trailer_system`:
//!
//! * Every weapon damage pass has already run (bullets, pools, melee, actives,
//!   traps, boss hazards, the evolutions and wave 3's own weapons), so any kill
//!   this system causes — a horde split, a lunge, a jab — lands before the
//!   reap one row later and takes the normal loot path.
//! * Every tell it starts is visible for at least a full tick before it can
//!   bite: the coil line, the flutter line and the dive line are all drawn from
//!   `w4.phase`/`w4.t` in the post-lighting hostile layer, and the state can
//!   only advance one phase per tick.
//! * It **does** spawn and despawn enemies (a horde split), which invalidates
//!   [`crate::game::ecs::resources::EnemyIndex`] — legal here only because
//!   nothing after `combat::contact_system` reads that cache (see the
//!   determinism rules in `CLAUDE.md`). The flatbed pass and the Hantavirus
//!   death jump that follow both walk the query, not the cache.
//!
//! ## Determinism
//!
//! Every pass walks the bodies in `SpawnId` order (one reused scratch sort, and
//! none at all on a tick with no wave-4 body on the map), nothing allocates
//! after the scratch buffers have grown, and **no behaviour draws rng**: the
//! lunge aim, the flutter line, the dive line, the split point and the horde's
//! regrowth are pure functions of the tick state. `Enemy::spawn` is only called
//! for the second half of a horde split, which can only happen once a horde is
//! already on the floor, so a run without wave-4 kinds draws exactly the rng it
//! drew before. The one exception is visual: the `vfx` emitters draw rng like
//! every other emitter in the game, and only when their kind is on the floor.
//!
//! ## The warned-attack limiter
//!
//! `WAVE3_DESIGN.md` §9: "at most two nearby ordinary enemies may start a
//! warned attack together". A wave-4 body wants to start its tell and the
//! ledger already holds two nearby (wave-3 or wave-4) tells, so the start is
//! deferred — its cooldown stays spent and it retries next tick while it keeps
//! moving and taking damage. The ledger reads wave 3's live state too, so a
//! wave-4 body waits behind a curled Pillbug or a live crossbar. The reverse
//! is not true (`wave3_system` runs first and cannot see this tick's wave-4
//! tells), which is the one asymmetry, documented and bounded: a wave-3 body
//! may start a second tell beside a wave-4 one on the same tick.
//!
//! **Lilliputians are exempt on purpose.** A squad's whole point is coordinated
//! pressure — ten tiny people levelling points, kneeling and throwing ropes in
//! the same second — and every one of those attacks is individually small (a
//! 1.8-damage pellet, a 3-damage contact, a hold the rat can dash out of). The
//! limiter would turn a squad into a queue of two. Their tells still *read* the
//! ledger (they wait when two other kinds are already warning nearby), they just
//! do not add to it.

#![allow(clippy::too_many_arguments)]
// The body chains are fixed arrays walked with an index: `trail[i]` is written
// while `trail[i - 1]` is read, which no iterator borrows allow.
#![allow(clippy::needless_range_loop)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{self, SizeClass};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{NextSpawnId, RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind, SNAKE_MAX_SEGMENTS};
use crate::game::level::Level;
use crate::game::nav::FlowField;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

// ---- The warned-attack limiter -------------------------------------------

/// "Nearby" for the two-at-once warned-attack rule (wave 3's radius, so both
/// systems count the same neighbourhood).
pub const WARN_NEARBY_RADIUS: f32 =
    crate::game::ecs::systems::enemy_wave3::WARN_NEARBY_RADIUS;
/// At most this many warned attacks may be running near one another.
pub const WARN_MAX_NEARBY: usize = crate::game::ecs::systems::enemy_wave3::WARN_MAX_NEARBY;

// ---- Big Snake -----------------------------------------------------------

/// Distance between two body points (the head included), in world px.
pub const SNAKE_SPACING: f32 = 6.0;
/// Radius of one body point: the moving wall the rat cannot walk through.
pub const SNAKE_BODY_RADIUS: f32 = 4.5;
/// The head starts its coil inside this range.
pub const SNAKE_STRIKE_RANGE: f32 = 64.0;
/// How long the rears and the dotted lunge line warn before the lunge.
pub const SNAKE_TELL_SECS: f32 = 0.80;
/// The lunge itself lasts this long …
pub const SNAKE_LUNGE_SECS: f32 = 0.22;
/// … and covers this many px (about 180 px/s, a fast commit).
pub const SNAKE_LUNGE_DISTANCE: f32 = 40.0;
/// How long it lies stretched and still afterwards: the punish window.
pub const SNAKE_STRETCH_SECS: f32 = 1.10;
/// Seconds between two strikes (measured from the end of the stretched pose).
pub const SNAKE_STRIKE_COOLDOWN: f32 = 3.2;
/// Cross-sway of the slither, as a fraction of the forward heading.
pub const SNAKE_SLITHER_SWAY: f32 = 0.55;
/// The dotted line drawn along the locked lunge aim.
pub const SNAKE_WARN_LINE: f32 = 44.0;
/// A hit this close to the head lands at [`SNAKE_HEAD_MULT`]; the body is 1.0.
pub const SNAKE_HEAD_SWEET_SPOT: f32 = 10.0;
/// The head's damage multiplier (`Enemy::damage_mult_from`).
pub const SNAKE_HEAD_MULT: f32 = 1.6;
/// Live snakes allowed at once (`spawn::director_system` substitutes fodder).
pub const SNAKE_LIVE_CAP: usize = 2;

/// Snake phases (`W4::phase`).
pub const SNAKE_PHASE_SLITHER: u8 = 0;
pub const SNAKE_PHASE_COIL: u8 = 1;
pub const SNAKE_PHASE_LUNGE: u8 = 2;
pub const SNAKE_PHASE_STRETCHED: u8 = 3;

// ---- House Centipede -----------------------------------------------------

/// The leg flutter before a dart (immobile, dotted line on the floor).
pub const CENTIPEDE_FLUTTER_SECS: f32 = 0.30;
/// How long the dart runs.
pub const CENTIPEDE_DART_SECS: f32 = 0.34;
/// Dart speed: about 54 px in one burst.
pub const CENTIPEDE_DART_SPEED: f32 = 160.0;
/// It only darts at a rat inside this band (closer is a contact fight, farther
/// is a wall run).
pub const CENTIPEDE_DART_MIN: f32 = 24.0;
pub const CENTIPEDE_DART_MAX: f32 = 120.0;
/// Seconds between darts.
pub const CENTIPEDE_DART_COOLDOWN: f32 = 2.4;
/// How long it flees after any hit.
pub const CENTIPEDE_FLEE_SECS: f32 = 0.90;
/// Flee speed multiplier (it is already the fastest body in the game).
pub const CENTIPEDE_FLEE_MULT: f32 = 1.35;
/// How far out it looks for a wall to hug.
pub const CENTIPEDE_WALL_PROBE: f32 = 9.0;

/// Centipede phases (`W4::phase`).
pub const CENTIPEDE_PHASE_RUN: u8 = 0;
pub const CENTIPEDE_PHASE_FLUTTER: u8 = 1;
pub const CENTIPEDE_PHASE_DART: u8 = 2;

// ---- Murder Hornet -------------------------------------------------------

/// Hover stand-off: it keeps this far from the rat while it charges.
pub const HORNET_HOVER_MIN: f32 = 56.0;
pub const HORNET_HOVER_MAX: f32 = 104.0;
/// The rising buzz before a dive can start.
pub const HORNET_CHARGE_SECS: f32 = 1.6;
/// The locked dive line's warning.
pub const HORNET_TELL_SECS: f32 = 0.55;
/// The dive itself.
pub const HORNET_DIVE_SECS: f32 = 0.45;
/// Dive speed: about 85 px.
pub const HORNET_DIVE_SPEED: f32 = 190.0;
/// The grounded recovery after any dive (hit or miss).
pub const HORNET_RECOVER_SECS: f32 = 1.40;
/// The jab's damage (heavier than the 6 an ordinary contact lands).
pub const HORNET_JAB_DAMAGE: f32 = 10.0;
/// Damage taken while grounded: the punish window, in numbers.
pub const HORNET_RECOVER_MULT: f32 = 1.5;
/// It only starts a charge with the rat this close.
pub const HORNET_BUZZ_RANGE: f32 = 170.0;

/// Hornet phases (`W4::phase`).
pub const HORNET_PHASE_HOVER: u8 = 0;
pub const HORNET_PHASE_CHARGE: u8 = 1;
pub const HORNET_PHASE_DIVE: u8 = 2;
pub const HORNET_PHASE_RECOVER: u8 = 3;

// ---- Termite Horde -------------------------------------------------------

/// One hit can take at most this fraction of the horde's pool: a single dart
/// chips it, an area volley (rain drops, nova spokes, thrash sweeps) or a
/// continuous source (a pool's per-tick damage) still works at full rate.
pub const HORDE_HIT_CAP: f32 = 0.12;
/// A single hit that would have taken this fraction splits the horde.
pub const HORDE_SPLIT_FRACTION: f32 = 0.30;
/// Only a large horde (tier 2) splits.
pub const HORDE_SPLIT_TIER: u8 = 2;
/// Pool size of a medium horde, as a fraction of the large one.
pub const HORDE_TIER1_FRACTION: f32 = 0.56;
/// Pool size of a small horde, as a fraction of the large one.
pub const HORDE_TIER0_FRACTION: f32 = 0.31;
/// A tier-1 horde that stays unhit this long and is back at full size reforms
/// into a large one (the seething mass gathers itself again).
pub const HORDE_REGROW_SECS: f32 = 6.0;
/// Hit points a second it regrows while unhit.
pub const HORDE_REGROW_RATE: f32 = 3.0;
/// How close to a glue board it has to be for the chewing crumbs.
pub const HORDE_CHEW_RADIUS: f32 = 14.0;
/// Seconds between two mouthfuls of crumbs.
pub const HORDE_CHEW_PERIOD: f32 = 0.45;
/// A mass of this radius is drawn for the horde (the *collision* radius stays
/// rat-sized so it can flow through a rat-hole).
pub const HORDE_DRAW_RADIUS: f32 = 7.0;

// ---- Lilliputians --------------------------------------------------------

/// Bodies in one squad: one leader, three spears, two archers and four rope
/// carriers. Ten is the smallest group that can actually form a spear line,
/// loose a volley and throw a rope knot at the same time.
pub const LILLI_SQUAD_SIZE: usize = 10;
/// Squads allowed on the floor at once (two, i.e. this many bodies).
pub const LILLI_LIVE_CAP: usize = 2 * LILLI_SQUAD_SIZE;

/// Roles (`W4::role`).
pub const LILLI_ROLE_LEADER: u8 = 0;
pub const LILLI_ROLE_SPEAR: u8 = 1;
pub const LILLI_ROLE_ARCHER: u8 = 2;
pub const LILLI_ROLE_ROPE: u8 = 3;

/// Phases (`W4::phase`), per role (the numbers are shared because only one
/// role's machine runs on a body).
pub const LILLI_PHASE_WALK: u8 = 0;
/// Spear: levelling the points (the tell). Archer: kneeling to loose. Rope:
/// planting and tying.
pub const LILLI_PHASE_TELL: u8 = 1;
/// Spear: the levelled charge. Rope: the knot is closed (the pin).
pub const LILLI_PHASE_ACTION: u8 = 2;
/// Everyone: the squad broke — run for the walls.
pub const LILLI_PHASE_PANIC: u8 = 3;

/// The spear wall takes this from the front …
pub const LILLI_FRONT_MULT: f32 = 0.6;
/// … and this from behind.
pub const LILLI_REAR_MULT: f32 = 1.6;

/// A spear levels its points inside this range (the front is visible).
pub const SPEAR_REACH: f32 = 44.0;
/// How long the points glint before the line steps in.
pub const SPEAR_TELL_SECS: f32 = 0.35;
/// The levelled charge lasts this long …
pub const SPEAR_CHARGE_SECS: f32 = 1.4;
/// … at this multiplier of the walk speed.
pub const SPEAR_CHARGE_MULT: f32 = 1.5;

/// An archer holds this band.
pub const ARCHER_MIN: f32 = 56.0;
pub const ARCHER_MAX: f32 = 120.0;
/// Kneel (the tell) before loosing.
pub const ARCHER_TELL_SECS: f32 = 0.70;
/// Seconds between volleys.
pub const ARCHER_PERIOD: f32 = 2.8;
/// Pellets per volley (each a normal enemy bullet).
pub const ARCHER_VOLLEY: usize = 2;
/// Half the volley's arc, in radians.
pub const ARCHER_SPREAD: f32 = 0.16;
/// Pellet speed.
pub const ARCHER_BULLET_SPEED: f32 = 70.0;
/// Pellet damage, as a fraction of the body's contact damage (a tiny arrow).
pub const ARCHER_BULLET_MULT: f32 = 0.6;

/// A rope carrier starts tying at this range.
pub const ROPE_REACH: f32 = 30.0;
/// How long the rope takes to tighten (the visible tell).
pub const ROPE_TIE_SECS: f32 = 0.90;
/// Rope carriers that must have a taut line on the rat before it is pinned.
pub const ROPE_MIN_TIED: usize = 3;
/// How long the pin holds once the knot closes (a sticky board's class).
pub const ROPE_HOLD_SECS: f32 = 1.10;
/// Fraction of the way back to the knot the pinned rat is dragged per tick.
pub const ROPE_DRAG: f32 = 0.30;
/// A squad at or below `LILLI_SQUAD_SIZE / PANIC_DIVISOR` bodies panics.
pub const PANIC_DIVISOR: usize = 3;
/// How long a panicking survivor runs before it escapes into the walls.
pub const PANIC_SECS: f32 = 4.0;
/// Panic runs at this multiplier.
pub const PANIC_MULT: f32 = 1.45;
/// A panicking straggler this close to another squad joins it.
pub const LILLI_REGROUP_RADIUS: f32 = 40.0;

/// A horde's pool cap at its current tier.
#[inline]
pub fn horde_tier_cap(max_hp: f32, tier: u8) -> f32 {
    max_hp
        * match tier {
            2 => 1.0,
            1 => HORDE_TIER1_FRACTION,
            _ => HORDE_TIER0_FRACTION,
        }
}

// ---- Scratch -------------------------------------------------------------

/// One half of a split, spawned after the walk so the mutable enemy borrow is
/// single.
#[derive(Clone, Copy, Debug)]
pub struct PendingSplit {
    pub pos: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    /// The parent's damage-number identity: both halves share one id, so the
    /// floating numbers of a split read as one horde.
    pub id: u32,
}

/// The wave-4 system's reused scratch (the same pattern as wave 3's).
#[derive(Default)]
pub struct Wave4Scratch {
    /// Every live enemy, in `SpawnId` order.
    pub order: Vec<(u64, Entity)>,
    /// The warned tells already running this tick (position + `SpawnId`).
    pub warns: Vec<(Vec2, u64)>,
    /// Player bullets, in `SpawnId` order (the snake body pass).
    pub bullets: Vec<(u64, Entity)>,
    /// The second halves of this tick's horde splits.
    pub splits: Vec<PendingSplit>,
    /// Live bodies per lilliputian squad (`squad id`, `count`, anchor), so the
    /// panic rule and the regroup hand-off are one scan instead of a per-body
    /// search.
    pub squads: Vec<(u32, u16, Vec2)>,
}

/// Is a warned wave-4 tell running on this body right now?
#[inline]
pub fn warn_active(e: &Enemy) -> bool {
    if !e.kind.is_wave4() {
        return false;
    }
    match e.kind.archetype() {
        EnemyKind::BigSnake => e.w4.phase == SNAKE_PHASE_COIL,
        EnemyKind::HouseCentipede => e.w4.phase == CENTIPEDE_PHASE_FLUTTER,
        EnemyKind::MurderHornet => e.w4.phase == HORNET_PHASE_CHARGE,
        _ => false,
    }
}

/// Is a *wave-3* warned tell running on this body (read only, so the limiter
/// counts both batches)?
#[inline]
fn wave3_warn_active(e: &Enemy) -> bool {
    match e.kind.archetype() {
        EnemyKind::Pillbug => e.w3.curl_t > 0.0,
        EnemyKind::BellFlea => e.w3.flea_phase != 0,
        EnemyKind::Porter => e.w3.raise_t > 0.0,
        EnemyKind::Turnkey => e.w3.bar_telling(),
        _ => false,
    }
}

/// How many warned tells are already running within [`WARN_NEARBY_RADIUS`].
#[inline]
fn warn_pressure(warns: &[(Vec2, u64)], at: Vec2) -> usize {
    let r2 = WARN_NEARBY_RADIUS * WARN_NEARBY_RADIUS;
    let mut n = 0;
    for &(pos, _) in warns {
        if pos.dist_sq(at) <= r2 {
            n += 1;
        }
    }
    n
}

/// The full wave-4 behaviour tick. See the module docs for placement and
/// determinism.
pub fn wave4_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    flow: Res<FlowField>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut next_id: ResMut<NextSpawnId>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bullets: Query<(Entity, &SpawnId, &mut Bullet)>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    // The floor slows the steering pass would have applied. A wave-4 body owns
    // its movement (see `W4::holds`), so it has to read them itself: a glued
    // centipede still crawls and an inked lilliputian still walks slowly.
    glue: Query<&crate::game::ecs::systems::traps::TrapSlow>,
    ink: Query<&crate::game::ecs::systems::hazards::InkSlow>,
    mut scratch: Local<Wave4Scratch>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let scratch = &mut *scratch;
    // The live key (highest `SpawnId` wins), the same rule the steering lure
    // and wave 3 use, so the walk and the tell agree.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    // Cheap gate: no wave-4 body on the map, nothing to do — not one sort, not
    // one rng draw. This is what keeps an early run identical.
    if !enemies.iter().any(|(_, _, e)| e.kind.is_wave4()) {
        return;
    }

    scratch.order.clear();
    scratch.order.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
    scratch.order.sort_unstable_by_key(|&(id, _)| id);

    // ---- Pass 1: the tells already running (wave 3 and wave 4) -----------
    scratch.warns.clear();
    scratch.squads.clear();
    for &(_, entity) in scratch.order.iter() {
        if let Ok((_, _, e)) = enemies.get(entity) {
            if e.alive() && e.kind.archetype() == EnemyKind::Lilliputian {
                // A flag, not `match find(...)`: the borrow of the table must
                // end before the push (the classic get-mut-then-insert case).
                let mut found = false;
                for slot in scratch.squads.iter_mut() {
                    if slot.0 == e.w4.squad {
                        slot.1 = slot.1.saturating_add(1);
                        found = true;
                        break;
                    }
                }
                if !found {
                    scratch.squads.push((e.w4.squad, 1, e.pos));
                }
            }
        }
    }
    for &(self_id, entity) in scratch.order.iter() {
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        let started_earlier = (e.kind.is_wave4() && warn_active(e) && e.w4.warn_age > 0.0)
            || (wave3_warn_active(e) && e.w3.warn_age > 0.0);
        if started_earlier {
            scratch.warns.push((e.pos, self_id));
        }
    }

    // ---- Pass 2: the state machines, in `SpawnId` order ------------------
    let beam = super::powerups::HeadlampBeam::from_player(&level, &player, player.face);
    scratch.splits.clear();
    for &(_, entity) in scratch.order.iter() {
        let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.kind.is_wave4() || !e.alive() {
            continue;
        }
        // Flatbed (`floor_flatbed`): the Loose Can runs its own state machine.
        if crate::game::ecs::systems::floor_flatbed::owns_state_machine(e.kind) {
            continue;
        }
        if e.w4.armed == 0 {
            arm(&mut e);
        }
        let self_id = id.0;
        let target = lure_target(&e, ppos, decoy, &level);
        // Glue and ink, the same product `enemy_steering_system` feeds to
        // `Enemy::steer`.
        let speed_mult = glue.get(entity).map_or(1.0, |s| s.0) * ink.get(entity).map_or(1.0, |s| s.0);
        let shy = beam.map_or(Vec2::ZERO, |beam| beam.shy(&level, &e, self_id));
        // The state machines are the archetype's (a registered kind runs its
        // base body's warned attack until its floor's own batch).
        match e.kind.archetype() {
            EnemyKind::BigSnake => snake_tick(
                &mut e,
                target,
                dt,
                &level,
                &flow,
                &scratch.warns,
                speed_mult,
                shy,
                &mut player,
                &mut particles,
                &mut rng,
                &mut sfx,
            ),
            EnemyKind::HouseCentipede => centipede_tick(
                &mut e,
                target,
                dt,
                &level,
                &flow,
                &scratch.warns,
                speed_mult,
                shy,
                &mut particles,
                &mut rng,
                &mut sfx,
            ),
            EnemyKind::MurderHornet => hornet_tick(
                &mut e,
                target,
                dt,
                &level,
                &scratch.warns,
                speed_mult,
                shy,
                &mut player,
                &mut particles,
                &mut rng,
                &mut sfx,
            ),
            EnemyKind::Lilliputian => lilliputian_tick(
                &mut e,
                entity,
                target,
                dt,
                &level,
                &flow,
                &scratch.warns,
                &scratch.squads,
                speed_mult,
                shy,
                &mut player,
                &mut particles,
                &mut rng,
                &mut sfx,
                &mut commands,
                &mut next_id,
            ),
            EnemyKind::TermiteHorde => horde_tick(
                &mut e,
                dt,
                &level,
                &mut particles,
                &mut rng,
                &mut scratch.splits,
            ),
            _ => {}
        }
        // The tell ledger: a tell that started this tick joins now (so bodies
        // later in `SpawnId` order see the slot taken), then ages for the next
        // tick's pass 1.
        if warn_active(&e) {
            if e.w4.warn_age <= 0.0 {
                scratch.warns.push((e.pos, self_id));
            }
            e.w4.warn_age += dt;
        } else {
            e.w4.warn_age = 0.0;
        }
    }

    // ---- Pass 2b: the rope knot ------------------------------------------
    //
    // A pin is not stored anywhere: it exists exactly while at least
    // `ROPE_MIN_TIED` rope carriers have a taut line on the rat, so killing a
    // rope-holder or dashing out of the ring ends it with no extra state — and
    // Clean Socks or a dash break it outright, exactly like a glue board.
    {
        let mut tied = 0usize;
        let mut knot = Vec2::ZERO;
        for &(_, entity) in scratch.order.iter() {
            let Ok((_, _, e)) = enemies.get(entity) else {
                continue;
            };
            if !e.alive() || e.kind.archetype() != EnemyKind::Lilliputian {
                continue;
            }
            if e.w4.role == LILLI_ROLE_ROPE
                && e.w4.phase == LILLI_PHASE_ACTION
                && e.pos.dist_sq(player.pos)
                    <= (ROPE_REACH + PLAYER_RADIUS + player.buffs.giant_hit_padding()) * (ROPE_REACH + PLAYER_RADIUS + player.buffs.giant_hit_padding())
            {
                tied += 1;
                knot += e.pos;
            }
        }
        let dash_free = player.dash_active > 0.0;
        let socked = player.buffs.socks > 0.0;
        if tied >= ROPE_MIN_TIED && !dash_free && !socked && player.alive() {
            knot = knot * (1.0 / tied as f32);
            player.vel = Vec2::ZERO;
            let drag = (knot - player.pos) * ROPE_DRAG;
            let from = player.pos;
            player.pos = if player.buffs.is_giant() {
                player.clip_giant_displacement(from, from + drag, &level.tiles)
            } else {
                level.tiles.resolve_circle(from + drag, PLAYER_RADIUS, SizeClass::Small, None)
            };
            sfx.0.push(Sfx::TrapGlueGrab);
        }
    }

    // ---- Pass 3: the snake body takes bullet hits ------------------------
    //
    // The head is an ordinary enemy circle (`combat::player_bullet_hits_system`
    // already resolved it); the body points are not, or a snake would be
    // unshootable once it turns. A bullet that reaches a body point lands on
    // the same HP pool, at `Enemy::damage_mult_from`'s rule: ×1 on the body,
    // ×1.6 on the head, and a bullet that already hit the head is skipped
    // (`life == 0`).
    body_bullet_pass(
        &mut enemies,
        &mut bullets,
        &scratch.order,
        &mut scratch.bullets,
        &mut particles,
        &mut numbers,
        &mut rng,
        &mut sfx,
    );

    // ---- Pass 4: spawn the second halves of this tick's splits -----------
    for split in scratch.splits.iter() {
        let mut child = Enemy::spawn(EnemyKind::TermiteHorde, split.pos, 1.0, &mut rng.0);
        child.id = split.id;
        child.hp = split.hp;
        child.max_hp = split.max_hp;
        child.w4.armed = 1;
        child.w4.tier = 1;
        child.w4.splits = 1;
        commands.spawn((child, next_id.take()));
    }
}

/// The point a body's warned attack is aimed at: the Wind-up Mouse's key while
/// the key is luring it, the rat otherwise (wave 3's rule, shared verbatim).
#[inline]
fn lure_target(e: &Enemy, ppos: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return ppos;
    };
    if !crate::game::ecs::systems::powerups::decoy_lures_body(e) {
        return ppos;
    }
    let decoy_r = crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS;
    if e.pos.dist_sq(key) > decoy_r * decoy_r {
        return ppos;
    }
    if !crate::game::ecs::systems::projectiles::los_clear(level, e.pos, key, e.radius) {
        return ppos;
    }
    key
}

/// Seed a body's timers the first time it is simulated (the facade and tests
/// spawn bodies without touching the wave-4 fields).
fn arm(e: &mut Enemy) {
    e.w4.armed = 1;
    match e.kind.archetype() {
        EnemyKind::BigSnake => {
            e.w4.phase = SNAKE_PHASE_SLITHER;
            e.w4.cd = SNAKE_STRIKE_COOLDOWN * 0.5;
            let f = if e.facing.len_sq() > 1e-6 {
                e.facing.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            };
            for i in 0..SNAKE_MAX_SEGMENTS {
                e.w4.trail[i] = e.pos - f * (i as f32 * SNAKE_SPACING);
            }
            e.w4.trail_len = SNAKE_MAX_SEGMENTS as u8;
        }
        EnemyKind::HouseCentipede => {
            e.w4.phase = CENTIPEDE_PHASE_RUN;
            e.w4.cd = CENTIPEDE_DART_COOLDOWN * 0.5;
        }
        EnemyKind::MurderHornet => {
            e.w4.phase = HORNET_PHASE_HOVER;
            e.w4.cd = HORNET_CHARGE_SECS * 0.5;
        }
        EnemyKind::TermiteHorde => {
            e.w4.tier = 2;
            e.w4.regrow_t = 0.0;
        }
        EnemyKind::Lilliputian => {
            e.w4.phase = LILLI_PHASE_WALK;
            e.w4.cd = match e.w4.role {
                LILLI_ROLE_ARCHER => ARCHER_PERIOD * 0.5,
                LILLI_ROLE_ROPE => 0.0,
                _ => 0.0,
            };
        }
        _ => {}
    }
}

/// Advance the snake's body points behind the head: every point is pulled to
/// exactly [`SNAKE_SPACING`] behind the one in front of it, so the body never
/// stretches and never folds over itself.
fn snake_follow(e: &mut Enemy) {
    let n = e.w4.trail_len as usize;
    if n == 0 {
        return;
    }
    e.w4.trail[0] = e.pos;
    let f = if e.facing.len_sq() > 1e-6 {
        e.facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    for i in 1..n {
        let prev = e.w4.trail[i - 1];
        let d = e.w4.trail[i] - prev;
        let dir = if d.len_sq() > 1e-6 {
            d.normalized()
        } else {
            -f
        };
        e.w4.trail[i] = prev + dir * SNAKE_SPACING;
    }
}

fn snake_tick(
    e: &mut Enemy,
    target: Vec2,
    dt: f32,
    level: &Level,
    flow: &FlowField,
    warns: &[(Vec2, u64)],
    speed_mult: f32,
    shy: Vec2,
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    e.w4.cd = (e.w4.cd - dt).max(0.0);
    match e.w4.phase {
        SNAKE_PHASE_COIL => {
            // Rearing: immobile, aim locked, the dotted line on the floor.
            e.w4.t -= dt;
            e.vel = Vec2::ZERO;
            if e.w4.t <= 0.0 {
                e.w4.phase = SNAKE_PHASE_LUNGE;
                e.w4.t = SNAKE_LUNGE_SECS;
                e.w4.traveled = 0.0;
                sfx.0.push(Sfx::SnakeStrike);
            }
        }
        SNAKE_PHASE_LUNGE => {
            let step = (SNAKE_LUNGE_DISTANCE / SNAKE_LUNGE_SECS * dt).min(SNAKE_LUNGE_DISTANCE);
            e.pos += e.w4.dir * step;
            e.w4.traveled += step;
            e.facing = e.w4.dir;
            e.vel = e.w4.dir * (SNAKE_LUNGE_DISTANCE / SNAKE_LUNGE_SECS);
            // A wall stops the lunge early: it still ends stretched.
            let class = SizeClass::of_enemy(e.kind);
            let resolved = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            let blocked = resolved.dist_sq(e.pos) > 0.25;
            e.pos = resolved;
            e.w4.t -= dt;
            if blocked || e.w4.t <= 0.0 || e.w4.traveled >= SNAKE_LUNGE_DISTANCE {
                e.w4.phase = SNAKE_PHASE_STRETCHED;
                e.w4.t = SNAKE_STRETCH_SECS;
                sfx.0.push(Sfx::Warn);
            }
        }
        SNAKE_PHASE_STRETCHED => {
            // The punish window: lying still, head on the floor.
            e.w4.t -= dt;
            e.vel = Vec2::ZERO;
            if e.w4.t <= 0.0 {
                e.w4.phase = SNAKE_PHASE_SLITHER;
                e.w4.cd = SNAKE_STRIKE_COOLDOWN;
            }
        }
        _ => {
            // Slither: route with the flow field, then cross-sway so the body
            // lays S-curves instead of a straight line.
            let flow_dir = flow.dir_at(SizeClass::of_enemy(e.kind), e.pos);
            let to = target - e.pos;
            let base = if flow_dir.len_sq() > 0.001 {
                flow_dir
            } else if to.len_sq() > 1e-6 {
                to.normalized()
            } else {
                e.facing
            };
            e.anim += dt * 3.0;
            let sway = base.perp() * e.anim.dsin() * SNAKE_SLITHER_SWAY;
            let dir = deflected((base + sway).normalized(), shy);
            e.facing += (dir - e.facing) * (5.0 * dt).min(1.0);
            let speed = e.speed() * e.speed_scale * e.slow_mult * speed_mult;
            e.pos += dir * speed * dt;
            e.vel = dir * speed;
            let class = SizeClass::of_enemy(e.kind);
            e.pos = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            e.pos = e
                .pos
                .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));

            // Coil when the rat is inside the strike band, the cooldown is
            // spent and the limiter has room.
            if e.w4.cd <= 0.0
                && e.pos.dist(target) <= SNAKE_STRIKE_RANGE
                && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY
            {
                let to = target - e.pos;
                if to.len_sq() > 1e-6 {
                    e.w4.dir = to.normalized();
                    e.facing = e.w4.dir;
                } else if e.facing.len_sq() <= 1e-6 {
                    e.w4.dir = Vec2::new(1.0, 0.0);
                } else {
                    e.w4.dir = e.facing.normalized();
                }
                e.w4.phase = SNAKE_PHASE_COIL;
                e.w4.t = SNAKE_TELL_SECS;
                e.w4.warn_age = 0.0;
                e.vel = Vec2::ZERO;
                vfx::emit_wave4_tell(particles, &mut rng.0, e.pos, e.w4.dir, e.kind);
                sfx.0.push(Sfx::Warn);
            }
        }
    }
    snake_follow(e);

    // The body is a moving wall: a walking rat is pushed out of it, a dashing
    // one passes over. No damage — the lunge and the head's contact do that.
    if player.dash_active <= 0.0 {
        // The push happens perpendicular to the body when the rat is exactly
        // on a body point, so one pass always gets it clear instead of sliding
        // it along the chain.
        let f = if e.facing.len_sq() > 1e-6 {
            e.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let perp = Vec2::new(-f.y, f.x);
        for i in 1..e.w4.trail_len as usize {
            let seg = e.w4.trail[i];
            let d = player.pos - seg;
            let min = SNAKE_BODY_RADIUS + PLAYER_RADIUS;
            let dist = d.len();
            if dist >= min {
                continue;
            }
            let n = if dist > 1e-3 {
                d * (1.0 / dist)
            } else {
                perp
            };
            player.pos = player.clip_giant_displacement(player.pos, seg + n * min, &level.tiles);
        }
    }
}

fn centipede_tick(
    e: &mut Enemy,
    target: Vec2,
    dt: f32,
    level: &Level,
    flow: &FlowField,
    warns: &[(Vec2, u64)],
    speed_mult: f32,
    shy: Vec2,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    e.w4.flee_t = (e.w4.flee_t - dt).max(0.0);
    e.w4.cd = (e.w4.cd - dt).max(0.0);
    let class = SizeClass::of_enemy(e.kind);
    let speed = e.speed() * e.speed_scale * e.slow_mult * speed_mult;
    match e.w4.phase {
        CENTIPEDE_PHASE_FLUTTER => {
            e.w4.t -= dt;
            e.vel = Vec2::ZERO;
            e.anim += dt * 26.0;
            if e.w4.t <= 0.0 {
                e.w4.phase = CENTIPEDE_PHASE_DART;
                e.w4.t = CENTIPEDE_DART_SECS;
                sfx.0.push(Sfx::CentipedeDart);
            }
        }
        CENTIPEDE_PHASE_DART => {
            e.pos += e.w4.dir * (CENTIPEDE_DART_SPEED * dt);
            e.vel = e.w4.dir * CENTIPEDE_DART_SPEED;
            e.anim += dt * 20.0;
            e.facing = e.w4.dir;
            let resolved = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            let blocked = resolved.dist_sq(e.pos) > 0.25;
            e.pos = resolved;
            e.w4.t -= dt;
            if blocked || e.w4.t <= 0.0 {
                e.w4.phase = CENTIPEDE_PHASE_RUN;
                e.w4.cd = CENTIPEDE_DART_COOLDOWN;
            }
        }
        _ => {
            // Run: follow the flow, hugging a wall when one is beside it.
            let flow_dir = flow.dir_at(class, e.pos);
            let mut dir = if flow_dir.len_sq() > 0.001 {
                flow_dir
            } else if (target - e.pos).len_sq() > 1e-6 {
                (target - e.pos).normalized()
            } else {
                e.facing
            };
            if e.w4.flee_t > 0.0 {
                // Flee: away from the rat, and toward a wall if one is near.
                let away = if (e.pos - target).len_sq() > 1e-6 {
                    (e.pos - target).normalized()
                } else {
                    -e.facing
                };
                dir = wall_blend(level, e.pos, away, class);
                e.pos += dir * (speed * CENTIPEDE_FLEE_MULT * dt);
                e.vel = dir * (speed * CENTIPEDE_FLEE_MULT);
            } else {
                // The wall hug only applies close to a wall, so open floor is a
                // straight crossing.
                dir = deflected(wall_blend(level, e.pos, dir, class), shy);
                e.anim += dt * (6.0 + speed * 0.5);
                e.pos += dir * speed * dt;
                e.vel = dir * speed;
            }
            e.facing += (dir - e.facing) * (10.0 * dt).min(1.0);
            e.pos = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            e.pos = e
                .pos
                .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));

            // Dart across open floor at the rat: a straight burst with a
            // flutter first.
            let dist = e.pos.dist(target);
            if e.w4.flee_t <= 0.0
                && e.w4.cd <= 0.0
                && (CENTIPEDE_DART_MIN..=CENTIPEDE_DART_MAX).contains(&dist)
                && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY
                && crate::game::ecs::systems::projectiles::los_clear(
                    level, e.pos, target, e.radius,
                )
            {
                e.w4.dir = if dist > 1e-3 {
                    (target - e.pos) * (1.0 / dist)
                } else {
                    e.facing.normalized()
                };
                e.w4.phase = CENTIPEDE_PHASE_FLUTTER;
                e.w4.t = CENTIPEDE_FLUTTER_SECS;
                e.w4.warn_age = 0.0;
                e.vel = Vec2::ZERO;
                e.facing = e.w4.dir;
                vfx::emit_wave4_tell(
                    particles,
                    &mut rng.0,
                    e.pos,
                    e.w4.dir,
                    e.kind,
                );
                sfx.0.push(Sfx::Warn);
            }
        }
    }
}

/// Blend `dir` toward the tangent of any wall the body is running beside, so a
/// centipede visibly scuttles along a wall face instead of away from it.
fn wall_blend(level: &Level, pos: Vec2, dir: Vec2, class: SizeClass) -> Vec2 {
    let mut best: Option<Vec2> = None;
    let mut best_dot = 0.25f32;
    for probe in [
        Vec2::new(1.0, 0.0),
        Vec2::new(-1.0, 0.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(0.0, -1.0),
    ] {
        let p = pos + probe * CENTIPEDE_WALL_PROBE;
        let (cx, cy) = collision::cell_of(p);
        if level.tiles.passable(cx, cy, class) {
            continue;
        }
        // The wall's tangent: run along it in whichever direction is closer to
        // where the body was already going.
        let tangent = probe.perp();
        let along = if tangent.dot(dir) >= 0.0 {
            tangent
        } else {
            -tangent
        };
        let d = along.dot(dir);
        if d > best_dot {
            best_dot = d;
            best = Some(along);
        }
    }
    let Some(tangent) = best else {
        return dir;
    };
    (dir * 0.45 + tangent * 0.55).normalized()
}

fn hornet_tick(
    e: &mut Enemy,
    target: Vec2,
    dt: f32,
    level: &Level,
    warns: &[(Vec2, u64)],
    speed_mult: f32,
    shy: Vec2,
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    let class = SizeClass::of_enemy(e.kind);
    match e.w4.phase {
        HORNET_PHASE_CHARGE => {
            e.w4.t -= dt;
            e.vel = Vec2::ZERO;
            e.anim += dt * 30.0;
            if e.w4.t <= 0.0 {
                e.w4.phase = HORNET_PHASE_DIVE;
                e.w4.t = HORNET_DIVE_SECS;
                e.w4.spent = 0;
                sfx.0.push(Sfx::HornetBuzz);
            }
        }
        HORNET_PHASE_DIVE => {
            e.pos += e.w4.dir * (HORNET_DIVE_SPEED * dt);
            e.vel = e.w4.dir * HORNET_DIVE_SPEED;
            e.facing = e.w4.dir;
            e.anim += dt * 34.0;
            let resolved = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            let blocked = resolved.dist_sq(e.pos) > 0.25;
            e.pos = resolved;
            // The jab: one heavier hit, through the player's i-frames like
            // every other hit in the game.
            if e.w4.spent == 0
                && collision::circles_overlap(player.pos, player.buffs.hurt_radius(), e.pos, e.radius)
                && player.take_damage(HORNET_JAB_DAMAGE)
            {
                e.w4.spent = 1;
                vfx::emit_player_hurt(particles, &mut rng.0, player.pos);
                sfx.0.push(Sfx::Hurt);
            }
            e.w4.t -= dt;
            if blocked || e.w4.t <= 0.0 {
                e.w4.phase = HORNET_PHASE_RECOVER;
                e.w4.t = HORNET_RECOVER_SECS;
                e.w4.spent = 0;
            }
        }
        HORNET_PHASE_RECOVER => {
            // Grounded and still: the punish window (×HORNET_RECOVER_MULT).
            e.w4.t -= dt;
            e.vel = Vec2::ZERO;
            if e.w4.t <= 0.0 {
                e.w4.phase = HORNET_PHASE_HOVER;
                e.w4.cd = HORNET_CHARGE_SECS;
            }
        }
        _ => {
            // Hover at range, orbiting; the buzz rises with the charge clock.
            let to = target - e.pos;
            let dist = to.len();
            let direct = if dist > 1e-3 {
                to * (1.0 / dist)
            } else {
                e.facing
            };
            let orbit = direct.perp() * 0.55;
            let radial = if dist > HORNET_HOVER_MAX {
                direct
            } else if dist < HORNET_HOVER_MIN {
                -direct
            } else {
                Vec2::ZERO
            };
            let dir = deflected((radial + orbit).normalized(), shy);
            let speed = e.speed() * e.speed_scale * e.slow_mult * speed_mult;
            e.pos += dir * speed * dt;
            e.vel = dir * speed;
            e.anim += dt * 18.0;
            if dir.len_sq() > 1e-6 {
                e.facing += (direct - e.facing) * (6.0 * dt).min(1.0);
            }
            e.pos = level.tiles.resolve_circle(e.pos, e.radius, class, None);
            e.pos = e
                .pos
                .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));

            e.w4.cd = (e.w4.cd - dt).max(0.0);
            if e.w4.cd <= 0.0
                && dist <= HORNET_BUZZ_RANGE
                && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY
            {
                e.w4.dir = if dist > 1e-3 {
                    to * (1.0 / dist)
                } else {
                    e.facing.normalized()
                };
                e.w4.phase = HORNET_PHASE_CHARGE;
                e.w4.t = HORNET_TELL_SECS;
                e.w4.warn_age = 0.0;
                e.vel = Vec2::ZERO;
                e.facing = e.w4.dir;
                vfx::emit_wave4_tell(
                    particles,
                    &mut rng.0,
                    e.pos,
                    e.w4.dir,
                    e.kind,
                );
                sfx.0.push(Sfx::Warn);
                sfx.0.push(Sfx::HornetBuzz);
            }
        }
    }
}

/// One tiny person: a spear, a bow, a rope or the little flag.
///
/// The squad's three questions are answered by the three roles — a spear wall
/// that hurts from the front (and only the front), archers whose pellets travel
/// the ordinary enemy-bullet path, and a rope crew that has to hold a taut line
/// on the rat for [`ROPE_TIE_SECS`] before the knot closes. The panic rule
/// (below a third of the squad) is the squad's own, and a straggler that runs
/// into another squad joins it instead of vanishing.
fn lilliputian_tick(
    e: &mut Enemy,
    entity: Entity,
    target: Vec2,
    dt: f32,
    level: &Level,
    flow: &FlowField,
    warns: &[(Vec2, u64)],
    squads: &[(u32, u16, Vec2)],
    speed_mult: f32,
    shy: Vec2,
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
    commands: &mut Commands,
    next_id: &mut NextSpawnId,
) {
    let speed = e.speed() * e.speed_scale * e.slow_mult * speed_mult;
    let dist = e.pos.dist(target);

    // ---- Panic -----------------------------------------------------------
    if e.w4.phase == LILLI_PHASE_PANIC {
        e.w4.t -= dt;
        if e.w4.t <= 0.0 {
            // Escaped into the walls (no loot: it was never killed).
            commands.entity(entity).despawn();
            return;
        }
        // A straggler that runs into another squad joins it and stops running.
        if let Some(&(other, _n, _anchor)) = squads
            .iter()
            .find(|(id, n, at)| {
                *id != e.w4.squad
                    && *n as usize > LILLI_SQUAD_SIZE / PANIC_DIVISOR
                    && at.dist_sq(e.pos) <= LILLI_REGROUP_RADIUS * LILLI_REGROUP_RADIUS
            })
        {
            e.w4.squad = other;
            e.w4.phase = LILLI_PHASE_WALK;
            e.w4.cd = 0.0;
            return;
        }
        let away = if e.pos.dist_sq(target) > 1e-6 {
            (e.pos - target).normalized()
        } else {
            -e.facing.normalized()
        };
        step_body(e, away, speed * PANIC_MULT, dt, level, Vec2::ZERO);
        return;
    }
    // Below a third of its *own* peak: the squad has broken.
    if let Some(&(_, n, _)) = squads.iter().find(|(id, _, _)| *id == e.w4.squad) {
        if n > e.w4.squad_peak {
            e.w4.squad_peak = n;
        }
        if (n as usize) <= LILLI_SQUAD_SIZE / PANIC_DIVISOR
            && (e.w4.squad_peak as usize) > LILLI_SQUAD_SIZE / PANIC_DIVISOR
        {
            e.w4.phase = LILLI_PHASE_PANIC;
            e.w4.t = PANIC_SECS;
            e.w4.warn_age = 0.0;
        }
    }
    if e.w4.phase == LILLI_PHASE_PANIC {
        return;
    }

    e.w4.cd = (e.w4.cd - dt).max(0.0);
    match e.w4.role {
        // ---- Spear and the flag ------------------------------------------
        LILLI_ROLE_LEADER | LILLI_ROLE_SPEAR => match e.w4.phase {
            LILLI_PHASE_TELL => {
                // Levelled points, planted feet: the wall's tell.
                e.w4.t -= dt;
                e.vel = Vec2::ZERO;
                let to = target - e.pos;
                if to.len_sq() > 1e-6 {
                    e.facing += (to.normalized() - e.facing) * (10.0 * dt).min(1.0);
                }
                if e.w4.t <= 0.0 {
                    e.w4.phase = LILLI_PHASE_ACTION;
                    e.w4.t = SPEAR_CHARGE_SECS;
                    e.w4.cd = SPEAR_CHARGE_SECS + 1.2;
                }
            }
            LILLI_PHASE_ACTION => {
                // The line steps in behind the points.
                e.w4.t -= dt;
                let dir = chase_dir(e, target, flow);
                step_body(e, dir, speed * SPEAR_CHARGE_MULT, dt, level, Vec2::ZERO);
                if e.w4.t <= 0.0 {
                    e.w4.phase = LILLI_PHASE_WALK;
                }
            }
            _ => {
                // Walk in a loose line: the slot decides which side of the
                // leader this one takes, so ten of them read as a rank.
                let dir = chase_dir(e, target, flow);
                let side = (e.w4.slot as f32 - (LILLI_SQUAD_SIZE as f32 * 0.5)) * 0.10;
                let formed = (dir + dir.perp() * side).normalized();
                step_body(e, formed, speed, dt, level, shy);
                if dist <= SPEAR_REACH
                    && e.w4.cd <= 0.0
                    && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY
                {
                    e.w4.phase = LILLI_PHASE_TELL;
                    e.w4.t = SPEAR_TELL_SECS;
                    e.w4.warn_age = 0.0;
                    vfx::emit_wave4_tell(
                        particles,
                        &mut rng.0,
                        e.pos,
                        e.facing,
                        e.kind,
                    );
                    sfx.0.push(Sfx::Warn);
                }
            }
        },
        // ---- Archer ------------------------------------------------------
        LILLI_ROLE_ARCHER => match e.w4.phase {
            LILLI_PHASE_TELL => {
                // Kneel, the arc marked: aim locked at the start.
                e.w4.t -= dt;
                e.vel = Vec2::ZERO;
                if e.w4.t <= 0.0 {
                    let from = e.pos;
                    let dmg = e.damage * ARCHER_BULLET_MULT;
                    let base = e.w4.dir;
                    for k in 0..ARCHER_VOLLEY {
                        let spread = if ARCHER_VOLLEY <= 1 {
                            0.0
                        } else {
                            (k as f32 / (ARCHER_VOLLEY - 1) as f32 - 0.5) * 2.0 * ARCHER_SPREAD
                        };
                        let dir = base.rotate(spread);
                        let v = dir * ARCHER_BULLET_SPEED;
                        let mut b = Bullet::new(
                            from,
                            v,
                            dmg,
                            Faction::Enemy,
                            crate::render::palette::scale(EnemyKind::Lilliputian.color(), 1.2),
                        );
                        b.life = 4.0;
                        b.size = 1;
                        commands.spawn((b, next_id.take()));
                    }
                    sfx.0.push(Sfx::Shoot);
                    e.w4.phase = LILLI_PHASE_WALK;
                    e.w4.cd = ARCHER_PERIOD;
                }
            }
            _ => {
                // Hold the band: back off inside it, close up outside it,
                // strafe otherwise.
                let to = target - e.pos;
                let direct = if dist > 1e-3 {
                    to * (1.0 / dist)
                } else {
                    e.facing
                };
                let dir = if dist > ARCHER_MAX {
                    direct
                } else if dist < ARCHER_MIN {
                    -direct
                } else {
                    direct.perp() * 0.5
                };
                step_body(e, dir, speed, dt, level, shy);
                if e.w4.cd <= 0.0
                    && (ARCHER_MIN..=ARCHER_MAX).contains(&dist)
                    && crate::game::ecs::systems::projectiles::los_clear(
                        level, e.pos, target, e.radius,
                    )
                    && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY
                {
                    e.w4.dir = if dist > 1e-3 { direct } else { e.facing };
                    e.w4.phase = LILLI_PHASE_TELL;
                    e.w4.t = ARCHER_TELL_SECS;
                    e.w4.warn_age = 0.0;
                    vfx::emit_wave4_tell(
                        particles,
                        &mut rng.0,
                        e.pos,
                        e.w4.dir,
                        e.kind,
                    );
                    sfx.0.push(Sfx::Warn);
                }
            }
        },
        // ---- Rope crew ----------------------------------------------------
        _ => match e.w4.phase {
            LILLI_PHASE_TELL => {
                // Planting the rope. If the rat steps out of reach the line
                // goes slack and the crew has to run it down again.
                e.w4.t -= dt;
                e.vel = Vec2::ZERO;
                let to = target - e.pos;
                if to.len_sq() > 1e-6 {
                    e.facing += (to.normalized() - e.facing) * (10.0 * dt).min(1.0);
                }
                if dist > ROPE_REACH + 8.0 {
                    e.w4.phase = LILLI_PHASE_WALK;
                    e.w4.t = 0.0;
                } else if e.w4.t <= 0.0 {
                    e.w4.phase = LILLI_PHASE_ACTION;
                    e.w4.t = ROPE_HOLD_SECS;
                }
            }
            LILLI_PHASE_ACTION => {
                // The knot is closed; the pin pass reads this phase.
                e.w4.t -= dt;
                e.vel = Vec2::ZERO;
                if e.w4.t <= 0.0 {
                    e.w4.phase = LILLI_PHASE_WALK;
                }
            }
            _ => {
                let dir = chase_dir(e, target, flow);
                step_body(e, dir, speed, dt, level, shy);
                if dist <= ROPE_REACH && warn_pressure(warns, e.pos) < WARN_MAX_NEARBY {
                    e.w4.phase = LILLI_PHASE_TELL;
                    e.w4.t = ROPE_TIE_SECS;
                    e.w4.warn_age = 0.0;
                    vfx::emit_wave4_tell(
                        particles,
                        &mut rng.0,
                        e.pos,
                        e.facing,
                        e.kind,
                    );
                    sfx.0.push(Sfx::Warn);
                }
            }
        },
    }
    let _ = player;
}

/// The direction a squad member walks: the flow field (so they come round
/// walls like every other body), or the straight line when the field is empty.
#[inline]
fn chase_dir(e: &Enemy, target: Vec2, flow: &FlowField) -> Vec2 {
    let f = flow.dir_at(SizeClass::of_enemy(e.kind), e.pos);
    if f.len_sq() > 0.001 {
        f
    } else if (target - e.pos).len_sq() > 1e-6 {
        (target - e.pos).normalized()
    } else {
        e.facing
    }
}

fn deflected(dir: Vec2, shy: Vec2) -> Vec2 {
    if shy == Vec2::ZERO { dir } else { dir + shy }
}

/// Move one body along `dir` for `speed * dt`, resolving walls and the arena.
#[inline]
fn step_body(e: &mut Enemy, dir: Vec2, speed: f32, dt: f32, level: &Level, shy: Vec2) {
    if dir.len_sq() <= 1e-6 {
        return;
    }
    let d = deflected(dir.normalized(), shy);
    e.pos += d * speed * dt;
    e.vel = d * speed;
    e.anim += dt * (8.0 + speed * 0.35);
    e.facing += (d - e.facing) * (8.0 * dt).min(1.0);
    let class = SizeClass::of_enemy(e.kind);
    e.pos = level.tiles.resolve_circle(e.pos, e.radius, class, None);
    e.pos = e
        .pos
        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
}

fn horde_tick(
    e: &mut Enemy,
    dt: f32,
    level: &Level,
    particles: &mut Particles,
    rng: &mut RngRes,
    splits: &mut Vec<PendingSplit>,
) {
    // Regrowth: a tier-1 horde that stays unhit swells back into a large one.
    e.w4.regrow_t += dt;
    let cap = horde_tier_cap(e.max_hp, e.w4.tier);
    if e.hp < cap {
        e.hp = (e.hp + HORDE_REGROW_RATE * dt).min(cap);
    }
    if e.w4.tier == 1 && e.w4.regrow_t >= HORDE_REGROW_SECS && e.hp >= cap - 0.01 {
        e.w4.tier = 2;
        e.w4.regrow_t = 0.0;
        vfx::emit_horde_reform(particles, &mut rng.0, e.pos);
    }

    // Splitting: a hard hit set the flag in `Enemy::apply_damage`. Total health
    // is conserved (each half gets half of what was left), so splitting is a
    // change of shape, not free healing.
    if e.w4.split_pending != 0 {
        e.w4.split_pending = 0;
        if e.w4.tier >= HORDE_SPLIT_TIER && e.w4.splits == 0 && e.hp > 0.0 {
            let half = e.hp * 0.5;
            e.hp = half;
            e.w4.tier = 1;
            e.w4.splits = 1;
            e.w4.regrow_t = 0.0;
            let class = SizeClass::of_enemy(e.kind);
            let f = if e.facing.len_sq() > 1e-6 {
                e.facing.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            };
            let side = f.perp() * (HORDE_DRAW_RADIUS * 0.8);
            let mut pos = e.pos + side;
            if !level.tiles.circle_clear(pos, e.radius, class) {
                pos = e.pos - side;
            }
            if level.tiles.circle_clear(pos, e.radius, class) {
                splits.push(PendingSplit {
                    pos,
                    hp: half,
                    max_hp: e.max_hp,
                    id: e.id,
                });
            }
            vfx::emit_horde_split(particles, &mut rng.0, e.pos);
        }
    }

    // The chew flourish: on a glue board it eats its way through (render-only —
    // the trap's own hold still applies, which is what keeps the trap a trap).
    e.w4.chew_t -= dt;
    if e.w4.chew_t <= 0.0 && on_a_glue_board(level, e.pos) {
        e.w4.chew_t = HORDE_CHEW_PERIOD;
        vfx::emit_horde_chew(particles, &mut rng.0, e.pos);
    }
}

/// Is `pos` on (or beside) a glue board's pad? The bite rule is the trap's own;
/// this is only for the crumbs.
fn on_a_glue_board(level: &Level, pos: Vec2) -> bool {
    level
        .objects
        .sticky_traps
        .iter()
        .any(|t| t.pos.dist_sq(pos) <= HORDE_CHEW_RADIUS * HORDE_CHEW_RADIUS)
}

/// The snake body points are not enemy circles the broadphase knows about, so a
/// bullet that would have crossed the tail has to be resolved here. The head is
/// skipped: `combat::player_bullet_hits_system` already owns it.
fn body_bullet_pass(
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bullets: &mut Query<(Entity, &SpawnId, &mut Bullet)>,
    order: &[(u64, Entity)],
    bullet_order: &mut Vec<(u64, Entity)>,
    particles: &mut Particles,
    numbers: &mut DamageNumbers,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    // Cheap gate: no snake, no bullet sweep.
    let has_snake = order.iter().any(|&(_, entity)| {
        enemies
            .get(entity)
            .is_ok_and(|(_, _, e)| e.kind.archetype() == EnemyKind::BigSnake && e.alive())
    });
    if !has_snake {
        return;
    }
    bullet_order.clear();
    bullet_order.extend(bullets.iter().map(|(entity, id, _)| (id.0, entity)));
    bullet_order.sort_unstable_by_key(|&(id, _)| id);
    if bullet_order.is_empty() {
        return;
    }
    let mut hit_any = false;
    for &(self_id, entity) in order.iter() {
        let _ = self_id;
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if e.kind.archetype() != EnemyKind::BigSnake || !e.alive() {
            continue;
        }
        for j in 0..bullet_order.len() {
            if !e.alive() {
                break;
            }
            let bullet_entity = bullet_order[j].1;
            let Ok((_, _, mut b)) = bullets.get_mut(bullet_entity) else {
                continue;
            };
            if b.faction != Faction::Player || b.life <= 0.0 || b.hit_cd > 0.0 || b.lob.is_some() {
                continue;
            }
            // Nearest body point along the sweep (index 0 is the head, which
            // combat owns; start at 1).
            let mut hit: Option<(usize, Vec2)> = None;
            for k in 1..e.w4.trail_len as usize {
                let seg = e.w4.trail[k];
                if collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, seg, SNAKE_BODY_RADIUS)
                {
                    hit = Some((k, seg));
                    break;
                }
            }
            let Some((_, seg)) = hit else {
                continue;
            };
            let applied = e.hurt_from(b.damage, seg);
            let killed = !e.alive();
            vfx::emit_weapon_hit(
                particles,
                &mut rng.0,
                b.pos,
                b.vel,
                b.weapon,
                e.kind,
                killed,
            );
            numbers.record(
                e.id,
                seg,
                SNAKE_BODY_RADIUS,
                applied,
                killed,
                DamageSource::from_weapon(b.weapon),
            );
            hit_any = true;
            if b.pierce > 0 {
                b.pierce -= 1;
                b.hit_cd = 0.12;
            } else {
                b.life = 0.0;
            }
        }
    }
    if hit_any {
        sfx.0.push(Sfx::Hit);
    }
}

#[cfg(test)]
mod tests;
