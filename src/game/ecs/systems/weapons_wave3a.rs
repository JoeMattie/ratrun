//! Wave 3 weapons, part A: **Acid Rain** and **Hantavirus**
//! (`WAVE3_DESIGN.md` §4).
//!
//! Both are ordinary auto-firing level-up weapons: their `Weapon::timer` is a
//! plain cooldown, `weapons::fire_weapons_system` decides when they fire, and
//! nothing here draws rng unless the weapon is owned *and* produces an effect.
//! Their damage, cycle, count and reach live in the tables at the top of
//! `weapon.rs` (the design fixes every number, so the generic
//! `1 + 0.25 * (level - 1)` curve is not used for them).
//!
//! ## Acid Rain
//!
//! A volley marks up to `RAIN_DROPS[level]` separate foes within
//! [`RAIN_RANGE`](crate::game::weapon::RAIN_RANGE) px, nearest first (ties by `SpawnId`, boss hit points behind
//! enemies — the ordering every aiming weapon shares). Each mark becomes an
//! [`AcidDrop`] entity: a floor position locked at fire time (clamped into the
//! arena), a splash radius and one damage number. The drops drizzle in after
//! [`RAIN_WINDUP`] seconds, [`RAIN_STAGGER`] apart; `acid_rain_system` lands
//! each one exactly once, and in the last [`RAIN_FALL`](crate::game::weapon::
//! RAIN_FALL) seconds before impact the art draws three descending lime
//! strokes over the mark. Spare drops (fewer foes than drops) are spent
//! round-robin over the marked spots, so a lone target eats the whole volley —
//! the design's "if only one exists, spend the spare drops at its marked spot"
//! is the one-mark case of that rule. There is no lingering pool: the splash is
//! one hit, then the entity is gone.
//!
//! ## Hantavirus
//!
//! Tags one unmarked body within [`VIRUS_RANGE`](crate::game::weapon::VIRUS_RANGE) px (an [`Infection`]): three
//! damage ticks [`VIRUS_TICK`] apart, and on the third tick or on death the
//! remaining jumps pass to **one fresh neighbour** within reach. A chain shares
//! one fixed-size visit history, so it visits each body once, skips bodies that
//! carry an active mark, never jumps through a wall, and ends when it is
//! isolated (or when the history is full). The boss cannot carry the component
//! (it is not an `Enemy`), so its mark lives in [`BossVirus`]: a boss takes
//! ticks and never transfers.
//!
//! Death transfer runs in its own system chained **after**
//! `combat::contact_system` and **before** `loot::reap_enemies_system`, so it
//! catches every way a host can die this tick (traps, any weapon, contact, boss
//! hazards) while the corpse and the [`EnemyIndex`] order still exist. The only
//! deaths it cannot see are the Nuke's, which are resolved *after* the reap
//! (`loot::nuke_reap_system`) — and a Nuke levels the whole neighbourhood, so
//! there is nothing for a chain to jump to.
//!
//! ## Determinism
//!
//! - Enemy walks use the cached [`EnemyIndex`] order (rebuilt by
//!   `enemy_move::grid_refresh_system` before the weapon pass), with the same
//!   length-checked `Local` fallback the other weapon systems use for narrow
//!   single-system tests. Every reader calls [`debug_assert_enemy_index`] in
//!   debug builds.
//! - Drops land in `SpawnId` (volley) order; transfers are resolved in enemy
//!   order and keep the nearest neighbour, so a replay lands the same hits in
//!   the same order.
//! - Nothing allocates per tick: the fallback order, the due list, the jump
//!   list and the claimed list are reused `Local`s, and the candidate scratch
//!   is a fixed-size stack array.

#![allow(clippy::too_many_arguments)]

use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use crate::game::ecs::components::{AcidDrop, Entity, Infection, SpawnId};
use crate::game::ecs::resources::{
    BossVirus, DamageLedger, EnemyIndex, NextSpawnId, RngRes, TickInput,
};
use crate::game::ecs::systems::boss::{Boss, BossTargetable};
use crate::game::ecs::systems::evolutions;
use crate::game::ecs::systems::projectiles::los_clear;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::weapon::{
    WeaponKind, RAIN_STAGGER, RAIN_WINDUP, VIRUS_TICK,
};
use crate::math::Vec2;

#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;

/// Radius the splash and transfer line tests use (the friendly-geometry radius
/// `weapons.rs` uses for melee effects).
const FIELD_LOS_RADIUS: f32 = 1.0;
/// The most drops one volley can have (`RAIN_DROPS[4]`). A stack array keeps
/// the volley allocation-free.
const MAX_DROPS: usize = 4;
/// Candidate cap for one pick, matching `weapons::TARGET_LOS_CANDIDATES`: the
/// pick is nearest-first, so a screen of covered bodies costs a fixed number of
/// tile queries.
const CANDIDATES: usize = 12;
/// Visit id recorded for the boss in a chain's history (a boss has no
/// `SpawnId`; it never transfers anyway).
const BOSS_VISIT: u64 = u64::MAX;

// ---- Target picking ------------------------------------------------------

/// Which body a pick chose to hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    /// A regular enemy entity (its `SpawnId` is the tie-break).
    Enemy(Entity),
    /// One of the boss's grounded hit-shape points.
    Boss,
}

#[derive(Clone, Copy)]
struct Candidate {
    dist_sq: f32,
    /// Regular enemies first, boss points second (the tie rule every aiming
    /// weapon shares).
    rank: u8,
    /// `SpawnId` for an enemy, the point index for a boss point.
    id: u64,
    pos: Vec2,
    picked: Picked,
}

const EMPTY_CANDIDATE: Candidate = Candidate {
    dist_sq: f32::MAX,
    rank: u8::MAX,
    id: u64::MAX,
    pos: Vec2::ZERO,
    picked: Picked::Enemy(Entity::PLACEHOLDER),
};

/// Nearest-first ordering: distance, then enemy before boss point, then lower
/// `SpawnId` / point index.
#[inline]
fn precedes(a: &Candidate, b: &Candidate) -> bool {
    match a.dist_sq.total_cmp(&b.dist_sq) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match a.rank.cmp(&b.rank) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => a.id < b.id,
        },
    }
}

/// Insert `c` into the fixed-size nearest-first scratch (bodies past the cap
/// are simply not candidates this shot).
#[inline]
fn push_candidate(out: &mut [Candidate; CANDIDATES], n: &mut usize, c: Candidate) {
    let at = out[..*n].partition_point(|o| precedes(o, &c));
    if at >= CANDIDATES {
        return;
    }
    let last = (*n).min(CANDIDATES - 1);
    let mut i = last;
    while i > at {
        out[i] = out[i - 1];
        i -= 1;
    }
    out[at] = c;
    *n = last + 1;
}

/// Fill `out` with the nearest in-range bodies (enemies in `SpawnId` order,
/// then the boss's grounded points), nearest first. No line test yet: the
/// caller walks the list and spends one tile query per candidate until it has
/// what it needs, exactly like `weapons::nearest_visible`.
///
/// `rank_from` is the point the ordering is measured from: the rat for
/// auto-aim, and the **reticle** when the player aims (the wave 3 controls:
/// "marks prefer the enemies nearest the aimed point"), so an aimed Acid Rain
/// volley lands where the player is pointing. The reach test always measures
/// from the rat, because that is where the weapon fires from.
fn gather(
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    from: Vec2,
    rank_from: Vec2,
    range: f32,
    out: &mut [Candidate; CANDIDATES],
) -> usize {
    let range_sq = range * range;
    let mut n = 0;
    for &entity in order {
        let Ok((_, id, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        if e.pos.dist_sq(from) > range_sq {
            continue;
        }
        let dist_sq = e.pos.dist_sq(rank_from);
        if dist_sq.is_nan() {
            continue;
        }
        push_candidate(
            out,
            &mut n,
            Candidate {
                dist_sq,
                rank: 0,
                id: id.0,
                pos: e.pos,
                picked: Picked::Enemy(entity),
            },
        );
    }
    for (index, point) in boss_points.iter().enumerate() {
        if point.dist_sq(from) > range_sq {
            continue;
        }
        let dist_sq = point.dist_sq(rank_from);
        if dist_sq.is_nan() {
            continue;
        }
        push_candidate(
            out,
            &mut n,
            Candidate {
                dist_sq,
                rank: 1,
                id: index as u64,
                pos: point,
                picked: Picked::Boss,
            },
        );
    }
    n
}

// ---- Acid Rain: the shot -------------------------------------------------

/// Acid Rain's shot: lock the mark positions and spawn one [`AcidDrop`] per
/// drop, all in volley order (so `SpawnId` order *is* drop order).
///
/// Returns `false` when nothing is in range or every candidate is behind cover,
/// which makes `fire_weapons_system` retry next tick instead of serving a full
/// cooldown for a volley that marked nothing.
///
/// `evolved` is **Downspout** (design §3): the volley no longer marks one foe
/// per drop. It marks the single nearest visible foe and walks the rest of the
/// drops back along the marking line, [`evolutions::DOWNSPOUT_STEP`] px apart,
/// so the splashes march toward the rat as they land. Every drop keeps its own
/// splash (and its own landing in `acid_rain_system`, which leaves the runoff).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_acid_rain(
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    ppos: Vec2,
    range: f32,
    dmg: f32,
    drops: u8,
    radius: f32,
    evolved: bool,
    particles: &mut Particles,
    rng: &mut RngRes,
    next_id: &mut NextSpawnId,
    // Wave 3 controls: the reticle in world pixels; marks are ranked from it
    // (and the reach still measured from the rat).
    aim: Option<Vec2>,
) -> bool {
    let mut cand = [EMPTY_CANDIDATE; CANDIDATES];
    let n = gather(
        order,
        enemies,
        boss_points,
        ppos,
        aim.unwrap_or(ppos),
        range,
        &mut cand,
    );

    let count = (drops as usize).min(MAX_DROPS);
    // Downspout is one mark walking home; the base volley marks one foe per
    // drop.
    let want = if evolved { 1 } else { count };
    let mut marks = [Vec2::ZERO; MAX_DROPS];
    let mut marked = 0usize;
    for c in cand[..n].iter() {
        if marked >= want {
            break;
        }
        // A foe behind masonry is not marked: friendly attack geometry stops
        // at solid cover, exactly like every other weapon's.
        if !los_clear(level, ppos, c.pos, 2.0) {
            continue;
        }
        marks[marked] = c.pos.clamp_box(Vec2::splat(4.0), level.arena - Vec2::splat(4.0));
        marked += 1;
    }
    if marked == 0 {
        return false;
    }

    let far = marks[0];
    let to_far = far - ppos;
    let dist = to_far.len();
    // The gutter's march line: from the rat out to the marked body.
    let gutter_dir = if dist > 0.001 {
        to_far * (1.0 / dist)
    } else {
        Vec2::new(1.0, 0.0)
    };

    for i in 0..count {
        let at = if evolved {
            // Step back toward the rat, but never past the rat's own feet: a
            // drop that lands under the rat would be wasted splash.
            let back = (evolutions::DOWNSPOUT_STEP * i as f32)
                .min((dist - radius * 0.5).max(0.0));
            (far - gutter_dir * back).clamp_box(Vec2::splat(4.0), level.arena - Vec2::splat(4.0))
        } else {
            // Spare drops go round-robin over the marked spots: a lone target
            // eats the whole volley (design §4), a pair splits the extras
            // evenly.
            marks[i % marked]
        };
        commands.spawn((
            AcidDrop {
                pos: at,
                delay: RAIN_WINDUP + i as f32 * RAIN_STAGGER,
                radius,
                dmg,
                index: i as u8,
                evolved,
                dir: if evolved { gutter_dir } else { Vec2::ZERO },
                volley: count as u8,
            },
            next_id.take(),
        ));
    }
    if evolved {
        // The whole staircase is drawn at fire time: the drops walk it, so the
        // player can read where each one will land before the first hits.
        vfx::emit_downspout_staircase(
            particles,
            &mut rng.0,
            ppos,
            far,
            count,
            evolutions::DOWNSPOUT_STEP,
        );
    }
    let aim = far - ppos;
    vfx::emit_acid_rain_fire(particles, &mut rng.0, ppos, aim);
    true
}

// ---- Hantavirus: the shot ------------------------------------------------

/// Hantavirus's shot: tag the nearest reachable, **unmarked** body within
/// [`VIRUS_RANGE`](crate::game::weapon::VIRUS_RANGE) (stretched by Lens Cap).
///
/// An enemy gets an [`Infection`] component (deferred, so the tick system sees
/// it on the next tick); the boss gets [`BossVirus`] instead, with no jumps.
/// Bodies already carrying an active mark are skipped, so the weapon never
/// restarts a chain that is mid-flight. Returns `false` when nothing can be
/// tagged, so the weapon retries next tick.
/// `evolved` is **Dead Letter** (design §3): a chain tagged by the evolved
/// weapon may seal its last jumps into a packet when its host dies isolated
/// (`Infection::can_seal`; the seal itself happens in
/// `hantavirus_death_system`). Nothing else about the shot changes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_hantavirus(
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    infected: &Query<&Infection>,
    boss_points: &BossTargetable,
    boss_virus: &mut BossVirus,
    ppos: Vec2,
    range: f32,
    dmg: f32,
    jumps: u8,
    reach: f32,
    evolved: bool,
    particles: &mut Particles,
    rng: &mut RngRes,
    // Wave 3 controls: tag the eligible body nearest the reticle, still within
    // the weapon's reach from the rat.
    aim: Option<Vec2>,
) -> bool {
    let mut cand = [EMPTY_CANDIDATE; CANDIDATES];
    let n = gather(
        order,
        enemies,
        boss_points,
        ppos,
        aim.unwrap_or(ppos),
        range,
        &mut cand,
    );
    let mut hit = None;
    let boss_marked = boss_virus.0.is_some_and(|i| i.active());
    for c in cand[..n].iter() {
        if let Picked::Enemy(entity) = c.picked {
            if infected.get(entity).is_ok_and(|i| i.active()) {
                continue;
            }
        } else if boss_marked {
            // The boss is mid-chain too: a second tag would restart its pips.
            continue;
        }
        if los_clear(level, ppos, c.pos, 2.0) {
            hit = Some(*c);
            break;
        }
    }
    let Some(target) = hit else {
        return false;
    };
    match target.picked {
        Picked::Enemy(entity) => {
            let id = enemies.get(entity).map(|(_, sid, _)| sid.0).unwrap_or(0);
            let mut mark = Infection::new(id, jumps, reach, dmg);
            // Dead Letter: only a chain tagged by the evolved weapon can seal.
            mark.can_seal = evolved;
            commands.entity(entity).insert(mark);
        }
        Picked::Boss => {
            *boss_virus = BossVirus(Some(Infection::new(BOSS_VISIT, 0, reach, dmg)));
        }
    }
    vfx::emit_virus_tag(particles, &mut rng.0, target.pos);
    true
}

// ---- Acid Rain: landing --------------------------------------------------

/// Land every Acid Rain drop whose countdown ran out this tick.
///
/// Runs right after `weapons::fire_weapons_system` (schedule 11c): the cached
/// [`EnemyIndex`] is the live `SpawnId` order there, and splash damage lands
/// before `combat::player_bullet_hits` / `loot::reap_enemies_system`, so a
/// splash kill is looted on the same tick.
pub fn acid_rain_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut drops: Query<(Entity, &SpawnId, &mut AcidDrop)>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut next_id: ResMut<NextSpawnId>,
    mut order_scratch: Local<Vec<Entity>>,
    mut due: Local<Vec<(u64, Entity)>>,
) {
    // No volley in flight: nothing to advance (and, importantly, no rng draw).
    if drops.is_empty() {
        return;
    }
    let dt = input.dt;
    due.clear();
    for (entity, id, mut d) in drops.iter_mut() {
        d.delay -= dt;
        if d.delay <= 0.0 {
            due.push((id.0, entity));
        }
    }
    if due.is_empty() {
        return;
    }
    // Land in spawn (volley) order: overlapping splashes and their rng draws
    // must not depend on archetype iteration order.
    due.sort_unstable_by_key(|&(id, _)| id);

    // The tick's stable enemy order, exactly as `weapons.rs` derives it.
    let order: &[Entity] = if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        order_scratch.clear();
        order_scratch.extend(enemies.iter().map(|(entity, _, _)| entity));
        order_scratch.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        order_scratch.as_slice()
    };

    for &(_, entity) in due.iter() {
        let Ok((_, _, d)) = drops.get(entity) else {
            continue;
        };
        let (pos, radius, dmg) = (d.pos, d.radius, d.dmg);
        let gutter = d.evolved;
        let gutter_dir = d.dir;
        let last_of_volley = d.index + 1 >= d.volley;
        commands.entity(entity).despawn();

        // Every enemy inside the splash takes one hit. Walls stop it: a splash
        // is floor geometry, not artillery through a partition.
        for &host in order {
            let Ok((_, _, mut e)) = enemies.get_mut(host) else {
                continue;
            };
            if !e.alive() || e.pos.dist(pos) > radius + e.radius {
                continue;
            }
            if !los_clear(&level, pos, e.pos, FIELD_LOS_RADIUS) {
                continue;
            }
            let hp_before = e.hp.max(0.0);
            // Directional armour: the splash arrives from its own centre. A
            // Downspout tread arrives from the gutter's axis instead (the
            // nearest point of the march line through this drop), exactly as
            // the gutter's runoff does in `evolutions::drain_cleaner_system`.
            let from = if gutter && gutter_dir.len_sq() > 1e-6 {
                let axis = gutter_dir.normalized();
                pos + axis * (e.pos - pos).dot(axis)
            } else {
                pos
            };
            let applied = e.hurt_from(dmg, from);
            let killed = !e.alive();
            ledger.add(WeaponKind::AcidRain, (hp_before - e.hp.max(0.0)).max(0.0));
            let enemy_pos = e.pos;
            let enemy_radius = e.radius;
            let enemy_id = e.id;
            damage_numbers.record(
                enemy_id,
                enemy_pos,
                enemy_radius,
                applied,
                killed,
                DamageSource::Rain,
            );
        }

        // The boss takes the splash through its grounded damageable shapes.
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none() || !shapes.circle_hits_grounded(pos, radius) {
                continue;
            }
            if level
                .tiles
                .segment_hit(pos, boss.pos, FIELD_LOS_RADIUS)
                .is_some()
            {
                continue;
            }
            let amount = dmg * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            ledger.add_boss(WeaponKind::AcidRain, amount);
            let killed = boss.hp <= 0.0;
            damage_numbers.record(
                boss.id,
                boss.pos,
                boss.kind.body_radius(),
                amount,
                killed,
                DamageSource::Rain,
            );
        }

        if gutter {
            // Downspout: the splash still splashes (it is acid), plus one tread
            // of the staircase flashing where it landed.
            vfx::emit_acid_splash(&mut particles, &mut rng.0, pos, radius * 0.7);
            vfx::emit_downspout_step(&mut particles, &mut rng.0, pos, radius);
        } else {
            vfx::emit_acid_splash(&mut particles, &mut rng.0, pos, radius);
        }
        if gutter && last_of_volley {
            // The gutter has marched all the way home: the stripe it walked
            // keeps burning for `RUNOFF_SECS` (Long Fuse stretches it), from
            // the near end back out toward the marked body. The runoff follows
            // the pooled wall rule: masonry stops it where it was laid.
            let span = (evolutions::DOWNSPOUT_STEP * (d.volley.saturating_sub(1)) as f32)
                .max(evolutions::RUNOFF_MIN_LEN);
            let open = evolutions::channel_open_len(&level, pos, gutter_dir, span);
            commands.spawn((
                crate::game::ecs::components::CorrosiveChannel::gutter(
                    pos,
                    gutter_dir,
                    open,
                    dmg * evolutions::RUNOFF_DPS,
                    evolutions::RUNOFF_SECS,
                ),
                next_id.take(),
            ));
            vfx::emit_downspout_runoff(
                &mut particles,
                &mut rng.0,
                pos,
                gutter_dir,
                open,
                1.0,
                input.dt,
            );
        }
    }
}

// ---- Hantavirus: ticks and transfers ------------------------------------

/// One pending transfer: the body it leaves and the chain generation to plant
/// on a fresh neighbour. `host` is only needed by the death system, which has
/// to spend the dying body's jumps.
pub struct Jump {
    host: Entity,
    from: Vec2,
    chain: Infection,
}

/// Pick the nearest eligible neighbour for a jump, in stable enemy order.
///
/// Eligibility: alive, not already visited by the chain, no active mark, not
/// already claimed by another chain this tick, within the chain's reach, with a
/// clear line. Ties break by `SpawnId`. `peek` is the caller's read-only view of
/// one enemy: `(spawn_id, pos, alive, has_an_active_mark, line_clear)`.
fn jump_target(
    order: &[Entity],
    from: Vec2,
    chain: &Infection,
    pending: &[(u64, Entity)],
    mut peek: impl FnMut(Entity) -> Option<(u64, Vec2, bool, bool, bool)>,
) -> Option<(u64, Entity)> {
    let mut best: Option<(f32, u64, Entity)> = None;
    for &host in order {
        let Some((id, pos, alive, marked, clear)) = peek(host) else {
            continue;
        };
        if !alive || marked || chain.has_visited(id) {
            continue;
        }
        if pending.iter().any(|&(_, p)| p == host) {
            continue;
        }
        let dist = pos.dist(from);
        if dist > chain.reach || !clear {
            continue;
        }
        let wins = match best {
            None => true,
            Some((d, best_id, _)) => match dist.total_cmp(&d) {
                Ordering::Less => true,
                Ordering::Equal => id < best_id,
                Ordering::Greater => false,
            },
        };
        if wins {
            best = Some((dist, id, host));
        }
    }
    best.map(|(_, id, entity)| (id, entity))
}

/// Tick every Hantavirus mark and pass finished chains on to a fresh neighbour.
///
/// Runs after [`acid_rain_system`] and before the projectile sweep, so tick
/// damage is reaped and looted on the tick it lands. The boss mark
/// ([`BossVirus`]) ticks here too; it never transfers.
pub fn hantavirus_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut Infection>)>,
    mut bosses: Query<&mut Boss>,
    mut boss_virus: ResMut<BossVirus>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut order_scratch: Local<Vec<Entity>>,
    mut jumps: Local<Vec<Jump>>,
    mut pending: Local<Vec<(u64, Entity)>>,
) {
    if enemies.is_empty() && boss_virus.0.is_none() {
        return; // no mark anywhere: no work, no rng
    }
    let dt = input.dt;
    jumps.clear();
    pending.clear();

    // The tick's stable enemy order (cached index, or the narrow-test
    // fallback). Taken once, before any mark mutates.
    let order: &[Entity] = if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _, _)| (id.0, entity)),
        );
        &index.0
    } else {
        order_scratch.clear();
        order_scratch.extend(enemies.iter().map(|(entity, _, _, _)| entity));
        order_scratch.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        order_scratch.as_slice()
    };

    // 1. Tick every living host. A corpse is left to the death-transfer system
    // (it runs after this tick's contact damage has been resolved).
    for &host in order {
        let Ok((_, _, mut e, inf)) = enemies.get_mut(host) else {
            continue;
        };
        let Some(mut inf) = inf else {
            continue;
        };
        if !inf.active() || !e.alive() {
            continue;
        }
        inf.timer -= dt;
        while inf.timer <= 0.0 && inf.ticks_left > 0 {
            inf.timer += VIRUS_TICK;
            inf.ticks_left -= 1;
            let hp_before = e.hp.max(0.0);
            // A contagion tick comes from inside the host: it has no
            // direction, so directional armour (a curled Pillbug's shell)
            // neither blunts nor doubles it. Plain `hurt` on purpose.
            e.hurt(inf.dmg);
            let killed = !e.alive();
            ledger.add(
                WeaponKind::Hantavirus,
                (hp_before - e.hp.max(0.0)).max(0.0),
            );
            let pos = e.pos;
            let enemy_radius = e.radius;
            let enemy_id = e.id;
            damage_numbers.record(
                enemy_id,
                pos,
                enemy_radius,
                inf.dmg,
                killed,
                DamageSource::Virus,
            );
            vfx::emit_virus_tick(&mut particles, &mut rng.0, pos, killed);
            // Third tick: the chain leaves for a fresh body (whether or not
            // this tick killed the host — "on third tick or death"). Spending
            // `jumps_left` here is what keeps the death system from
            // transferring the same chain a second time.
            if inf.ticks_left == 0 {
                if inf.jumps_left > 0 {
                    jumps.push(Jump {
                        host,
                        from: pos,
                        chain: *inf,
                    });
                }
                inf.jumps_left = 0;
            }
        }
    }

    // 2. Tick the boss's mark (a boss takes ticks, never transfers).
    if let Some(mut inf) = boss_virus.0 {
        let mut alive = false;
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                break;
            }
            alive = true;
            inf.timer -= dt;
            while inf.timer <= 0.0 && inf.ticks_left > 0 {
                inf.timer += VIRUS_TICK;
                inf.ticks_left -= 1;
                let amount = inf.dmg * boss.damage_mult;
                if boss.take_damage(amount) {
                    ledger.add_boss(WeaponKind::Hantavirus, amount);
                    let killed = boss.hp <= 0.0;
                    damage_numbers.record(
                        boss.id,
                        boss.pos,
                        boss.kind.body_radius(),
                        amount,
                        killed,
                        DamageSource::Virus,
                    );
                    vfx::emit_virus_tick(&mut particles, &mut rng.0, boss.pos, killed);
                }
            }
        }
        boss_virus.0 = if alive && inf.ticks_left > 0 {
            Some(inf)
        } else {
            None
        };
    }

    // 3. Resolve the transfers, in the order the chains finished (host
    // `SpawnId` order), nearest fresh neighbour first.
    for jump in jumps.iter() {
        let best = jump_target(
            order,
            jump.from,
            &jump.chain,
            &pending,
            |entity| {
                let (_, sid, e, inf) = enemies.get(entity).ok()?;
                Some((
                    sid.0,
                    e.pos,
                    e.alive(),
                    inf.is_some_and(|i| i.active()),
                    los_clear(&level, jump.from, e.pos, FIELD_LOS_RADIUS),
                ))
            },
        );
        let Some((id, target)) = best else {
            continue; // isolated: the chain ends here
        };
        let Some(next) = jump.chain.jumped(id) else {
            continue; // fixed history full: the chain ends rather than allocate
        };
        commands.entity(target).insert(next);
        pending.push((id, target));
        let to = enemies
            .get(target)
            .map(|(_, _, e, _)| e.pos)
            .unwrap_or(jump.from);
        vfx::emit_virus_jump(&mut particles, &mut rng.0, jump.from, to);
    }
}

/// Death-triggered Hantavirus transfer.
///
/// Chained after `combat::contact_system` and before
/// `loot::reap_enemies_system`, so it sees every host that died this tick — to a
/// trap, any weapon, contact or a boss hazard — while the corpse (and its
/// position, and the live [`EnemyIndex`] order) still exists. A host with jumps
/// left plants its chain on one fresh neighbour; a host with none is simply
/// forgotten with the body.
pub fn hantavirus_death_system(
    mut commands: Commands,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut next_id: ResMut<NextSpawnId>,
    mut enemies: Query<(Entity, &SpawnId, &Enemy, Option<&mut Infection>)>,
    mut order_scratch: Local<Vec<Entity>>,
    mut jumps: Local<Vec<Jump>>,
    mut pending: Local<Vec<(u64, Entity)>>,
) {
    // Cheap early out (no rng draws when no chain is on a corpse).
    let mut any = false;
    for (_, _, e, inf) in enemies.iter() {
        if !e.alive() && inf.is_some_and(|i| i.active() && i.jumps_left > 0) {
            any = true;
            break;
        }
    }
    if !any {
        return;
    }
    jumps.clear();
    pending.clear();

    let order: &[Entity] = if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _, _)| (id.0, entity)),
        );
        &index.0
    } else {
        order_scratch.clear();
        order_scratch.extend(enemies.iter().map(|(entity, _, _, _)| entity));
        order_scratch.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        order_scratch.as_slice()
    };

    // 1. Collect the corpses that still owe a jump, in `SpawnId` order.
    for &host in order {
        let Ok((_, _, e, inf)) = enemies.get(host) else {
            continue;
        };
        let Some(inf) = inf else {
            continue;
        };
        if e.alive() || !inf.active() || inf.jumps_left == 0 {
            continue;
        }
        jumps.push(Jump {
            host,
            from: e.pos,
            chain: *inf,
        });
    }
    // 2. Spend each dying host's jumps so it cannot transfer twice if the reap
    // is deferred past this tick (the boss's horde break, a frozen sim).
    for jump in jumps.iter() {
        if let Ok((_, _, _, Some(mut inf))) = enemies.get_mut(jump.host) {
            inf.jumps_left = 0;
            inf.ticks_left = 0;
        }
    }
    // 3. Plant the chains on fresh neighbours.
    for jump in jumps.iter() {
        let best = jump_target(order, jump.from, &jump.chain, &pending, |entity| {
            let (_, sid, e, inf) = enemies.get(entity).ok()?;
            Some((
                sid.0,
                e.pos,
                e.alive(),
                inf.is_some_and(|i| i.active()),
                los_clear(&level, jump.from, e.pos, FIELD_LOS_RADIUS),
            ))
        });
        let Some((id, target)) = best else {
            // Isolated. A **Dead Letter** chain does not die with its host: it
            // folds its remaining jumps into a sealed packet and waits
            // (`evolutions::dead_letter_system`). One packet per chain — the
            // flag travels with the reopened chain, so a packet that comes back
            // and dies isolated again simply ends.
            if jump.chain.can_seal && !jump.chain.sealed {
                let mut chain = jump.chain;
                chain.sealed = true;
                commands.spawn((
                    crate::game::ecs::components::DeadLetterPacket {
                        pos: jump.from,
                        life: evolutions::DEAD_LETTER_SECS,
                        total: evolutions::DEAD_LETTER_SECS,
                        chain,
                    },
                    next_id.take(),
                ));
                vfx::emit_dead_letter_seal(&mut particles, &mut rng.0, jump.from);
            }
            continue;
        };
        let Some(next) = jump.chain.jumped(id) else {
            continue;
        };
        commands.entity(target).insert(next);
        pending.push((id, target));
        let to = enemies
            .get(target)
            .map(|(_, _, e, _)| e.pos)
            .unwrap_or(jump.from);
        vfx::emit_virus_jump(&mut particles, &mut rng.0, jump.from, to);
    }
}

#[cfg(test)]
mod tests {
    use super::{acid_rain_system, hantavirus_death_system, hantavirus_system};
    use crate::audio::Sfx;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::components::{AcidDrop, Entity, Infection, SpawnId};
    use crate::game::ecs::resources::{BossVirus, DamageLedger};
    use crate::game::ecs::systems::boss::{Boss, BossKind};
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS, TileRect};
    use crate::game::loadout::Upgrade;
    use crate::game::weapon::{
        Weapon, WeaponKind, RAIN_CYCLE, RAIN_DAMAGE, RAIN_DROPS, VIRUS_CYCLE, VIRUS_DAMAGE,
        VIRUS_JUMPS, VIRUS_REACH,
    };
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Wipe the generated layout's walls so a test measures the weapon, not the
    /// map.
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.tiles.rebuild_masks();
    }

    /// A fresh, empty arena with the given weapon armed and ready, and the rat
    /// facing +x.
    fn arena(seed: u64, kind: WeaponKind, level: u8) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level,
            timer: 0.0,
            evolved: false,
        }];
        w
    }

    /// A 3-cell solid column whose left face is the line `x = 16 * cx`, placed
    /// three cells right of the rat. Returns the face's world x.
    fn wall_between(w: &mut World) -> f32 {
        let cx = (w.player.pos.x / 16.0) as i32 + 3;
        let cy = (w.player.pos.y / 16.0) as i32;
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// Fire once on a zero-length tick, so the observed cooldown is exactly the
    /// design's cycle (a real tick subtracts `dt` first, which is the same
    /// number the live loop adds back).
    fn run_fire(w: &mut World) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        // A dud leaves the timer at 0 (ready for the next tick); a shot serves
        // the full cycle, so the timer only ever grows on a real shot.
        w.player.loadout.weapons[0].timer > before
    }

    fn run_rain(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(acid_rain_system).unwrap();
        w.end_tick();
    }

    fn run_virus(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(hantavirus_system).unwrap();
        w.end_tick();
    }

    fn run_virus_death(w: &mut World) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(hantavirus_death_system)
            .unwrap();
        w.end_tick();
    }

    /// Every live drop, in `SpawnId` (volley) order.
    fn drops(w: &mut World) -> Vec<AcidDrop> {
        let mut q = w.ecs.query::<(&SpawnId, &AcidDrop)>();
        let mut v: Vec<(u64, AcidDrop)> = q.iter(&w.ecs).map(|(id, d)| (id.0, *d)).collect();
        v.sort_unstable_by_key(|&(id, _)| id);
        v.into_iter().map(|(_, d)| d).collect()
    }

    /// Every infected body, in `SpawnId` order: `(entity, spawn id, mark)`.
    fn infections(w: &mut World) -> Vec<(Entity, u64, Infection)> {
        let mut q = w.ecs.query::<(Entity, &SpawnId, &Infection, &Enemy)>();
        let mut v: Vec<(u64, Entity, Infection)> = q
            .iter(&w.ecs)
            .map(|(e, id, inf, _)| (id.0, e, *inf))
            .collect();
        v.sort_unstable_by_key(|&(id, _, _)| id);
        v.into_iter().map(|(id, e, inf)| (e, id, inf)).collect()
    }

    fn hp(w: &World, entity: Entity) -> f32 {
        w.ecs.get::<Enemy>(entity).unwrap().hp
    }

    fn mark_of(w: &World, entity: Entity) -> Option<Infection> {
        w.ecs.get::<Infection>(entity).copied()
    }

    fn boss_at(w: &mut World, kind: BossKind, offset: Vec2) -> Entity {
        let arena = w
            .level
            .boss_room()
            .map(|r| r.rect)
            .unwrap_or_else(|| TileRect::new(0, 0, 10, 10));
        let at = w.player.pos + offset;
        let boss = Boss::new(kind, at, 5000.0, arena, at, false);
        w.ecs.spawn((boss, SpawnId(999))).id()
    }

    /// Run the rain system until every drop has landed (or the cap trips).
    fn land_all(w: &mut World) -> Vec<AcidDrop> {
        let mut seen = drops(w);
        for _ in 0..240 {
            run_rain(w, 1.0 / 60.0);
            if drops(w).is_empty() {
                return seen;
            }
            seen = drops(w);
        }
        panic!("drops never landed");
    }

    // ---- Acid Rain ------------------------------------------------------

    /// Nearest foes first, one drop per mark, staggered after the wind-up: the
    /// design's "mark separate foes within 72 px, nearest first … lock floor
    /// positions, drizzle after 0.35 s at 0.12 s intervals".
    #[test]
    fn rain_marks_the_nearest_foes_one_staggered_drop_each() {
        let mut w = arena(11, WeaponKind::AcidRain, 1);
        let p = w.player.pos;
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(60.0, 0.0), 1.0);
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let mid = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(-30.0, 0.0), 1.0);
        let far_pos = w.ecs.get::<Enemy>(far).unwrap().pos;

        assert!(run_fire(&mut w), "a volley with three marks must fire");
        let d = drops(&mut w);
        assert_eq!(d.len(), RAIN_DROPS[0] as usize);
        assert_eq!(d[0].pos, p + Vec2::new(10.0, 0.0), "nearest mark first");
        assert_eq!(d[1].pos, p + Vec2::new(-30.0, 0.0), "second nearest next");
        assert_eq!(d[0].delay, 0.35);
        assert!((d[1].delay - 0.47).abs() < 1e-5, "0.12 s stagger");
        assert_eq!(d[0].radius, 7.0);
        assert_eq!(d[0].dmg, RAIN_DAMAGE[0]);
        assert!(
            d.iter().all(|x| x.pos != far_pos),
            "a third foe is left unmarked: the volley only has two drops"
        );
        assert_ne!(near, mid);
    }

    /// Fewer foes than drops: the spare drops stack on the one mark (the design
    /// says so explicitly for the lone-target case).
    #[test]
    fn rain_spends_spare_drops_on_the_only_mark() {
        let mut w = arena(12, WeaponKind::AcidRain, 5);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(12.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        let d = drops(&mut w);
        assert_eq!(d.len(), RAIN_DROPS[4] as usize);
        for (i, drop) in d.iter().enumerate() {
            assert_eq!(drop.pos, p + Vec2::new(12.0, 0.0));
            assert!((drop.delay - (0.35 + 0.12 * i as f32)).abs() < 1e-5);
            assert_eq!(drop.radius, 10.0);
            assert_eq!(drop.dmg, RAIN_DAMAGE[4]);
        }
    }

    /// An exact distance tie goes to the lower `SpawnId` (the whole wave's tie
    /// rule), not to query order.
    #[test]
    fn rain_breaks_distance_ties_by_spawn_id() {
        let mut w = arena(13, WeaponKind::AcidRain, 1);
        let p = w.player.pos;
        // Two bodies exactly 20 px away, one above and one below.
        let first = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 20.0), 1.0);
        let second = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, -20.0), 1.0);
        let first_pos = w.ecs.get::<Enemy>(first).unwrap().pos;
        let second_pos = w.ecs.get::<Enemy>(second).unwrap().pos;
        assert!(run_fire(&mut w));
        let d = drops(&mut w);
        assert_eq!(d[0].pos, first_pos, "the earlier body wins the tie");
        assert_eq!(d[1].pos, second_pos);
    }

    /// Per-level splash damage, one hit per landing drop.
    #[test]
    fn rain_splash_damage_matches_the_design_table() {
        for level in 1..=5u8 {
            let mut w = arena(20 + level as u64, WeaponKind::AcidRain, level);
            let p = w.player.pos;
            let target = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
            let start = hp(&w, target);
            assert!(run_fire(&mut w));
            // One drop at a time: let only the first land.
            let first_landing = drops(&mut w)[0].delay;
            let mut t = 0.0;
            while t < first_landing + 0.02 {
                run_rain(&mut w, 1.0 / 60.0);
                t += 1.0 / 60.0;
            }
            let after_first = hp(&w, target);
            assert!(
                (start - after_first - RAIN_DAMAGE[level as usize - 1]).abs() < 1e-4,
                "L{level}: expected {} damage, got {}",
                RAIN_DAMAGE[level as usize - 1],
                start - after_first
            );
        }
    }

    /// The volley keeps its design cycle: one shot, then nothing until the
    /// cooldown elapses.
    #[test]
    fn rain_fires_on_its_design_cycle() {
        for level in 1..=5u8 {
            let mut w = arena(30 + level as u64, WeaponKind::AcidRain, level);
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
            assert!(run_fire(&mut w));
            let cd = w.player.loadout.weapons[0].timer;
            assert!(
                (cd - RAIN_CYCLE[level as usize - 1]).abs() < 1e-5,
                "L{level} cycle {} != {}",
                cd,
                RAIN_CYCLE[level as usize - 1]
            );
            assert_eq!(drops(&mut w).len(), RAIN_DROPS[level as usize - 1] as usize);
            // Half a cycle later, still nothing new.
            w.player.loadout.weapons[0].timer = cd * 0.5;
            assert!(!run_fire(&mut w), "must not fire mid-cooldown");
            assert_eq!(drops(&mut w).len(), RAIN_DROPS[level as usize - 1] as usize);
        }
    }

    /// A foe behind masonry is not marked: the volley is a dud and the weapon
    /// stays ready instead of serving a cooldown for nothing.
    #[test]
    fn rain_does_not_mark_through_a_wall() {
        let mut w = arena(41, WeaponKind::AcidRain, 1);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, Vec2::new(face + 20.0, p.y), 1.0);
        assert!(!run_fire(&mut w), "a covered foe is not a mark");
        assert!(drops(&mut w).is_empty());
        assert_eq!(w.player.loadout.weapons[0].timer, 0.0);
    }

    /// The splash is floor geometry: a body on the far side of a partition
    /// within the radius is spared, the one on the mark's side is not.
    #[test]
    fn rain_splash_does_not_cross_a_wall() {
        let mut w = arena(42, WeaponKind::AcidRain, 1);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        // 6 px in front / 3 px behind the face: the bodies are 9 px apart, well
        // inside the 7 px splash + 6.5 px body test, and both ends of the line
        // clear the wall's own 2 px targeting radius.
        let near = w.spawn_enemy(EnemyKind::Brute, Vec2::new(face - 6.0, p.y), 1.0);
        let behind = w.spawn_enemy(EnemyKind::Brute, Vec2::new(face + 3.0, p.y), 1.0);
        let near_hp = hp(&w, near);
        let behind_hp = hp(&w, behind);
        assert!(run_fire(&mut w));
        let d = drops(&mut w);
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.pos == Vec2::new(face - 6.0, p.y)));
        land_all(&mut w);
        assert!(hp(&w, near) < near_hp, "the marked body takes the splash");
        assert_eq!(
            hp(&w, behind),
            behind_hp,
            "the wall must stop the splash (the bodies are only 5 px apart)"
        );
    }

    /// A marked boss takes the splash through its damageable hit shapes, and
    /// the ledger books it in Acid Rain's column.
    #[test]
    fn rain_damages_the_boss_and_books_it() {
        let mut w = arena(43, WeaponKind::AcidRain, 2);
        let boss_entity = boss_at(&mut w, BossKind::Cat, Vec2::new(12.0, 0.0));
        assert!(run_fire(&mut w), "the boss is a markable body");
        land_all(&mut w);
        let boss = w.ecs.get::<Boss>(boss_entity).unwrap();
        assert!(boss.hp < boss.max_hp, "the boss took no rain damage");
        let dealt = w.ecs.resource::<DamageLedger>().boss[WeaponKind::AcidRain.index()];
        assert!(dealt > 0.0, "boss damage must be booked to Acid Rain");
        assert!(
            (dealt - (boss.max_hp - boss.hp)).abs() < 1e-3,
            "booked {dealt} but the boss lost {}",
            boss.max_hp - boss.hp
        );
    }

    // ---- Hantavirus -----------------------------------------------------

    /// The tag lands on the nearest body, starts at three pips and carries the
    /// level's jumps and reach.
    #[test]
    fn hantavirus_tags_the_nearest_body_at_its_level() {
        for level in 1..=5u8 {
            let mut w = arena(50 + level as u64, WeaponKind::Hantavirus, level);
            let p = w.player.pos;
            let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(50.0, 0.0), 1.0);
            assert!(run_fire(&mut w), "L{level} must tag something");
            let got = infections(&mut w);
            assert_eq!(got.len(), 1, "L{level}: exactly one body is tagged");
            assert_eq!(got[0].0, near);
            let inf = got[0].2;
            assert_eq!(inf.ticks_left, 3);
            assert_eq!(inf.jumps_left, VIRUS_JUMPS[level as usize - 1]);
            assert_eq!(inf.reach, VIRUS_REACH[level as usize - 1]);
            assert_eq!(inf.dmg, VIRUS_DAMAGE[level as usize - 1]);
            assert!(inf.has_visited(got[0].1), "the first host is in its history");
            // …and the cycle is the design's.
            let cd = w.player.loadout.weapons[0].timer;
            assert!((cd - VIRUS_CYCLE[level as usize - 1]).abs() < 1e-5);
        }
    }

    /// Three ticks, 0.35 s apart, each worth the level's per-tick damage.
    #[test]
    fn hantavirus_ticks_three_times_for_the_table_damage() {
        for level in 1..=5u8 {
            let mut w = arena(60 + level as u64, WeaponKind::Hantavirus, level);
            let p = w.player.pos;
            let host = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
            let start = hp(&w, host);
            assert!(run_fire(&mut w));
            let dmg = VIRUS_DAMAGE[level as usize - 1];
            // 1.2 s of ticks: three hits 0.35 s apart, nothing more.
            let mut t = 0.0;
            while t < 1.2 {
                run_virus(&mut w, 1.0 / 60.0);
                t += 1.0 / 60.0;
            }
            let lost = start - hp(&w, host);
            assert!(
                (lost - 3.0 * dmg).abs() < 1e-4,
                "L{level}: expected {} over three ticks, got {lost}",
                3.0 * dmg
            );
            let inf = mark_of(&w, host).expect("the mark is spent, not removed");
            assert_eq!(inf.ticks_left, 0);
            assert!(infections(&mut w).len() == 1, "isolated: no jump");
        }
    }

    /// The chain walks a pack, visits each body once, and stops when every
    /// neighbour is already in its history.
    #[test]
    fn hantavirus_chain_visits_each_body_once() {
        let mut w = arena(71, WeaponKind::Hantavirus, 5);
        let p = w.player.pos;
        let a = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 1.0);
        let c = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(26.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        assert_eq!(infections(&mut w).len(), 1);
        let mut t = 0.0;
        while t < 4.0 {
            run_virus(&mut w, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        let dmg = VIRUS_DAMAGE[4];
        assert!((hp(&w, a) - (60.0 - 3.0 * dmg)).abs() < 1e-4, "a's ticks");
        assert!((hp(&w, b) - (60.0 - 3.0 * dmg)).abs() < 1e-4, "b's ticks");
        assert!((hp(&w, c) - (60.0 - 3.0 * dmg)).abs() < 1e-4, "c's ticks");
        // Four jumps at L5, three bodies: the chain ended on `c` with jumps
        // left, because every neighbour was already in its history.
        let end = mark_of(&w, c).unwrap();
        assert_eq!(end.ticks_left, 0);
        assert!(infections(&mut w).len() == 3, "no body was tagged twice");
        assert_eq!(end.visited, 3);
    }

    /// A body that already carries an active mark is skipped: another chain
    /// never steals it, and the weapon will not restart a running chain.
    #[test]
    fn hantavirus_skips_bodies_with_an_active_mark() {
        let mut w = arena(72, WeaponKind::Hantavirus, 5);
        let p = w.player.pos;
        let a = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 1.0);
        let c = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(26.0, 0.0), 1.0);
        // `c` belongs to a foreign chain that is not ticking yet.
        let mut foreign = Infection::new(999, 4, 22.0, 7.0);
        foreign.timer = 999.0;
        w.ecs.entity_mut(c).insert(foreign);
        assert!(run_fire(&mut w));
        assert_eq!(infections(&mut w).len(), 2);
        // The weapon tagged `a` (the nearest unmarked body).
        assert!(mark_of(&w, a).is_some_and(|i| i.active()));

        let mut t = 0.0;
        while t < 4.0 {
            run_virus(&mut w, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        // a → b, then b's jump finds only visited `a` and marked `c`.
        let c_mark = mark_of(&w, c).unwrap();
        assert_eq!(c_mark.ticks_left, 3, "the foreign mark must be untouched");
        assert_eq!(c_mark.jumps_left, 4);
        assert_eq!(hp(&w, c), 60.0, "no chain damage reached the marked body");
        assert!((hp(&w, b) - (60.0 - 3.0 * VIRUS_DAMAGE[4])).abs() < 1e-4);
    }

    /// Firing again while a chain is running is a dud: the nearest body is
    /// marked, so the weapon waits (and keeps its pips) instead of resetting
    /// the tick counter.
    #[test]
    fn hantavirus_does_not_retag_a_running_chain() {
        let mut w = arena(73, WeaponKind::Hantavirus, 1);
        let p = w.player.pos;
        let host = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        run_virus(&mut w, 0.35);
        let after_one = mark_of(&w, host).unwrap();
        assert_eq!(after_one.ticks_left, 2);
        // Ready again, but the only body is marked.
        w.player.loadout.weapons[0].timer = 0.0;
        assert!(!run_fire(&mut w), "a marked body is not re-tagged");
        assert_eq!(mark_of(&w, host).unwrap().ticks_left, 2);
    }

    /// The boss takes ticks and never transfers.
    #[test]
    fn hantavirus_ticks_the_boss_without_transferring() {
        let mut w = arena(74, WeaponKind::Hantavirus, 3);
        let boss_entity = boss_at(&mut w, BossKind::Cat, Vec2::new(10.0, 0.0));
        assert!(run_fire(&mut w), "the boss is taggable");
        let marked = w.ecs.resource::<BossVirus>().0;
        assert!(marked.is_some_and(|i| i.ticks_left == 3 && i.jumps_left == 0));
        assert!(infections(&mut w).is_empty(), "the boss is not an enemy");
        let before = w.ecs.get::<Boss>(boss_entity).unwrap().hp;
        let mult = w.ecs.get::<Boss>(boss_entity).unwrap().damage_mult;
        let dmg = VIRUS_DAMAGE[2];
        let mut t = 0.0;
        while t < 1.2 {
            run_virus(&mut w, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        let boss = w.ecs.get::<Boss>(boss_entity).unwrap();
        assert!((before - boss.hp - 3.0 * dmg * mult).abs() < 1e-3);
        assert!(w.ecs.resource::<BossVirus>().0.is_none(), "the mark clears");
        let dealt = w.ecs.resource::<DamageLedger>().boss[WeaponKind::Hantavirus.index()];
        assert!((dealt - 3.0 * dmg * mult).abs() < 1e-3);
    }

    /// A host that dies before its third tick passes the chain to the nearest
    /// fresh neighbour — whatever killed it, because every damage source leaves
    /// `hp <= 0` for the pre-reap system to see.
    #[test]
    fn hantavirus_death_transfers_the_chain() {
        let mut w = arena(75, WeaponKind::Hantavirus, 3);
        let p = w.player.pos;
        let a = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(16.0, 0.0), 1.0);
        let c = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        run_virus(&mut w, 0.35); // one tick: two pips left
        assert_eq!(mark_of(&w, a).unwrap().ticks_left, 2);
        // Any source can do this: acid, a dart, a trap, a boss hazard.
        w.ecs.get_mut::<Enemy>(a).unwrap().hp = 0.0;

        run_virus_death(&mut w);

        let moved = mark_of(&w, b).expect("the chain must jump to the nearest fresh body");
        assert_eq!(moved.ticks_left, 3, "a jump restarts the three ticks");
        assert_eq!(moved.jumps_left, VIRUS_JUMPS[2] - 1, "one jump spent");
        assert!(moved.has_visited(999 - 1) || moved.visited == 2);
        assert!(mark_of(&w, c).is_none(), "the far body is not the neighbour");
        let spent = mark_of(&w, a).unwrap();
        assert!(!spent.active() && spent.jumps_left == 0, "the corpse is spent");
    }

    /// A host with no jumps left, or no fresh neighbour, simply ends: no mark
    /// moves and nothing panics.
    #[test]
    fn hantavirus_death_ends_when_the_chain_is_isolated_or_spent() {
        // Isolated: nobody within reach.
        let mut w = arena(76, WeaponKind::Hantavirus, 1);
        let p = w.player.pos;
        let host = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(50.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        w.ecs.get_mut::<Enemy>(host).unwrap().hp = 0.0;
        run_virus_death(&mut w);
        assert_eq!(infections(&mut w).len(), 1, "only the spent corpse carries it");
        assert!(!infections(&mut w)[0].2.active());

        // Spent: the third tick already used the jumps.
        let mut w = arena(77, WeaponKind::Hantavirus, 1);
        let p = w.player.pos;
        let a = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(16.0, 0.0), 1.0);
        assert!(run_fire(&mut w));
        let mut t = 0.0;
        while t < 1.1 {
            run_virus(&mut w, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        // `a` finished its ticks and already jumped to `b`, so `a` is spent: a
        // death now must not start a second chain.
        w.ecs.get_mut::<Enemy>(a).unwrap().hp = 0.0;
        let b_mark = mark_of(&w, b).unwrap();
        run_virus_death(&mut w);
        assert_eq!(
            mark_of(&w, b).unwrap().ticks_left,
            b_mark.ticks_left,
            "no second tag on the neighbour"
        );
        assert_eq!(infections(&mut w).len(), 2);
    }

    // ---- Ownership / determinism ---------------------------------------

    /// Without the two weapons nothing spawns, nothing is marked and no wave-3
    /// sound is queued: an unowned weapon cannot change a run.
    #[test]
    fn nothing_happens_when_the_weapons_are_not_owned() {
        let mut w = World::new(Theme::Sewer, 909);
        assert!(!w.player.loadout.has(WeaponKind::AcidRain));
        assert!(!w.player.loadout.has(WeaponKind::Hantavirus));
        let mut saw_wave3_sfx = false;
        for tick in 0..180u32 {
            let mv = match (tick / 20) % 4 {
                0 => Vec2::new(1.0, 0.0),
                1 => Vec2::new(0.0, 1.0),
                2 => Vec2::new(-1.0, 0.0),
                _ => Vec2::new(0.0, -1.0),
            };
            w.update(1.0 / 60.0, mv, false);
            saw_wave3_sfx |= w
                .sfx
                .iter()
                .any(|s| matches!(s, Sfx::ShootRain | Sfx::Infect));
        }
        assert!(!saw_wave3_sfx, "an unowned weapon made a sound");
        assert!(drops(&mut w).is_empty(), "an unowned weapon marked the floor");
        let mut q = w.ecs.query::<&Infection>();
        assert_eq!(q.iter(&w.ecs).count(), 0, "an unowned weapon tagged a body");
    }

    /// Same seed, same inputs, same run — with each new weapon owned and at L3,
    /// over a full live schedule (director spawning, walls, loot).
    #[test]
    fn twin_runs_are_deterministic_with_each_wave3_weapon() {
        /// `(enemies as (id, x, y, hp) in 1/64 px, kills, marked bodies, this
        /// weapon's booked damage)` — a state trace small enough to compare
        /// between twin runs.
        type Trace = (Vec<(u32, i64, i64, i64)>, u32, usize, i64);

        fn trace(seed: u64, kind: WeaponKind) -> Trace {
            let mut w = World::new(Theme::Sewer, seed);
            if !w.player.loadout.has(kind) {
                w.player.loadout.apply(&Upgrade::NewWeapon(kind));
            }
            for weapon in w.player.loadout.weapons.iter_mut() {
                weapon.level = 3;
                weapon.timer = 0.0;
            }
            for tick in 0..600u32 {
                let a = tick as f32 * 0.01;
                let mv = Vec2::from_angle(a);
                w.update(1.0 / 60.0, mv, false);
            }
            let enemies = w
                .enemies()
                .into_iter()
                .map(|e| {
                    (
                        e.id,
                        (e.pos.x * 64.0).round() as i64,
                        (e.pos.y * 64.0).round() as i64,
                        (e.hp * 64.0).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            let marked = infections(&mut w).len();
            let dealt = w.ecs.resource::<DamageLedger>().dealt[kind.index()];
            (enemies, w.kills, marked, (dealt * 64.0).round() as i64)
        }
        for kind in [WeaponKind::AcidRain, WeaponKind::Hantavirus] {
            let a = trace(4242, kind);
            let b = trace(4242, kind);
            assert_eq!(a.0, b.0, "{kind:?}: enemies diverged between twin runs");
            assert_eq!(a.1, b.1, "{kind:?}: kills diverged");
            assert_eq!(a.2, b.2, "{kind:?}: the set of marked bodies diverged");
            assert_eq!(a.3, b.3, "{kind:?}: booked damage diverged");
            // The trace comparison is worth nothing if the weapon never fired.
            assert!(a.3 > 0, "{kind:?} dealt no damage in a live 10 s run");
        }
    }

    // ---- weapons-b passive hooks on the two wave-3a weapons --------------

    fn stack(w: &mut World, passive: crate::game::loadout::PassiveKind) {
        for _ in 0..passive.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(passive));
        }
    }

    /// Long Fuse's "+4% splash/radial radius" widens every Acid Rain splash.
    #[test]
    fn long_fuse_widens_the_rain_splash() {
        use crate::game::loadout::PassiveKind;
        for fuse in [false, true] {
            let mut w = arena(71, WeaponKind::AcidRain, 5);
            if fuse {
                stack(&mut w, PassiveKind::LongFuse);
            }
            let want = w.player.loadout.stats.splash_radius(10.0);
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(12.0, 0.0), 1.0);
            assert!(run_fire(&mut w));
            for drop in drops(&mut w) {
                assert!((drop.radius - want).abs() < 1e-5);
            }
            assert_eq!(want > 10.0, fuse, "only Long Fuse widens the splash");
        }
    }

    /// Lens Cap's "+8% reach of ranged weapons" stretches Acid Rain's 72 px
    /// marking range: a foe at 80 px is out of reach bare and marked with it.
    #[test]
    fn lens_cap_stretches_the_rain_marking_reach() {
        use crate::game::loadout::PassiveKind;
        for lens in [false, true] {
            let mut w = arena(72, WeaponKind::AcidRain, 1);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
            }
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(80.0, 0.0), 1.0);
            assert_eq!(run_fire(&mut w), lens, "lens={lens}");
        }
    }

    /// Lens Cap stretches Hantavirus's 56 px tag reach and the jump reach the
    /// chain carries: a foe at 65 px is only taggable with the passive, and its
    /// mark's reach is the stretched table value.
    #[test]
    fn lens_cap_stretches_the_virus_tag_and_jump_reach() {
        use crate::game::loadout::PassiveKind;
        for lens in [false, true] {
            let mut w = arena(73, WeaponKind::Hantavirus, 1);
            if lens {
                stack(&mut w, PassiveKind::LensCap);
            }
            let want = w.player.loadout.stats.ranged_reach(VIRUS_REACH[0]);
            let p = w.player.pos;
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(65.0, 0.0), 1.0);
            assert_eq!(run_fire(&mut w), lens, "lens={lens}");
            if lens {
                let got = infections(&mut w);
                assert_eq!(got.len(), 1);
                assert!((got[0].2.reach - want).abs() < 1e-5);
                assert!(want > VIRUS_REACH[0]);
            }
        }
    }
}
