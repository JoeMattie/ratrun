//! The **Spiked Tail Whip** (wave 3, `w3-spiketail`).
//!
//! An ordinary auto-firing level-up weapon with five levels, and the third tail
//! weapon in the game — deliberately *not* the other two:
//!
//! - **Tail Whip** is a radial aura pulse around the rat (everything close).
//! - **Tail Thrash** is an active, charged burst of full sweeps (Space).
//! - **Spiked Tail** is a single-target lash: on its cooldown the tail
//!   bristles, then cracks straight out at one targeted body, lands a heavy
//!   hit with a small burst at the impact point, and recoils.
//!
//! ## The beat
//!
//! One [`Lash`] entity per shot, spawned by `weapons::fire_one`, carries the
//! whole shot (the numbers are snapshotted at fire time, so a level-up card
//! opened mid-lash cannot change a hit that was already paid for). The sim
//! runs on the same clock the art does — [`LASH_WINDUP`](crate::game::weapon::
//! LASH_WINDUP) → the crack → [`LASH_RECOIL_AT`](crate::game::weapon::
//! LASH_RECOIL_AT) → [`LASH_TOTAL`](crate::game::weapon::LASH_TOTAL):
//!
//! 1. **Wind-up.** The tail thickens and sprouts spikes (drawn on the real
//!    tail chain in `art_rat`), and the impact point follows its body.
//! 2. **Crack.** The primary body takes the full lash damage
//!    ([`Enemy::hurt_from`] with the direction *from the rat*, so the
//!    Pillbug's shell still applies), a short stagger and a shove through
//!    `Stats::shove` (Counterweight). Everything *else* within the burst
//!    radius takes [`LASH_BURST_FRAC`](crate::game::weapon::LASH_BURST_FRAC)
//!    of it, arriving from the impact point. From level 4 a couple of spike
//!    shards break off and fly on as ordinary friendly bullets.
//! 3. **Recoil.** An evolved **Barbed Wire** lash drags its catch home,
//!    bursting every body it crosses. Heavy bodies are not dragged.
//!
//! The boss is not an `Enemy`: it is picked through its grounded hit points
//! (`BossTargetable`), takes the full lash through its hit shapes, and can
//! take the burst once on top (the design's rule for the big bodies).
//!
//! ## Determinism
//!
//! - The enemy walk uses the cached [`EnemyIndex`] order (valid at this slot:
//!   nothing has spawned or despawned since `grid_refresh`), with the usual
//!   length-checked `Local` fallback for the narrow single-system tests.
//! - Lash entities are walked in `SpawnId` order; the burst walks the
//!   `SpawnId`-ordered enemy list, so two identical runs land the same hits in
//!   the same order and draw their sparks in the same order.
//! - Firing draws **no** rng at all (the bristle is the tail art, not
//!   particles), so a run that never owns the weapon is byte-for-byte
//!   unchanged. A lash with no target never spawns and draws nothing.
//! - The Barbed Wire drag moves a body, so the spatial grid is re-filed from
//!   the still-valid `EnemyIndex` before the projectile and contact passes read
//!   it (the same one-fill rule as Counterweight's nudge in `weapons.rs`).

use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{SizeClass, SpatialGrid};
use crate::game::ecs::components::{Entity, Lash, LashTarget, SpawnId};
use crate::game::ecs::resources::{
    DamageLedger, EnemyIndex, NextSpawnId, RngRes, Shake, TickInput,
};
use crate::game::ecs::systems::boss::{Boss, BossTargetable};
use crate::game::ecs::systems::projectiles::los_clear;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::ecs::constants::GRID_CELL;
use crate::game::weapon::{
    WeaponKind, LASH_BURST_FRAC, LASH_DRAG_GAP, LASH_HEAVY_SCALE, LASH_RECOIL_AT,
    LASH_SHARD_FRAC, LASH_SHAKE, LASH_STAGGER, LASH_TOTAL, LASH_WINDUP,
};
use crate::math::Vec2;

#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;

/// Radius the lash's line-of-sight tests use (the honest body radius of a
/// melee swing — `weapons.rs` uses the same number for its melee effects).
const LASH_LOS_RADIUS: f32 = 1.0;
/// Radius a splash/line test uses (a floor effect, not a projectile).
const FIELD_LOS_RADIUS: f32 = 1.0;
/// Candidate cap for one pick, matching `weapons::TARGET_LOS_CANDIDATES`: the
/// pick is nearest-first, so a screen of covered bodies costs a fixed number of
/// tile queries.
const CANDIDATES: usize = 12;
/// The boss is huge; a locked impact point still counts as "on it" inside this
/// many px of a grounded hit shape (the shape moves under the point for the
/// 0.12 s of wind-up).
const BOSS_LOCK_SLOP: f32 = 10.0;
/// Speed (px/s) and life (s) of a broken-off spike shard: about 63 px of
/// travel, a shard's throw.
const SHARD_SPEED: f32 = 150.0;
const SHARD_LIFE: f32 = 0.42;
/// Half-angle between two shards of the fan (radians).
const SHARD_FAN: f32 = 0.22;

/// The per-tick reusable scratch: the fallback `SpawnId`-ordered enemy list and
/// the lash entities in `SpawnId` order. Both are `Local`s' worth of memory
/// reused for the life of the schedule, so the system allocates nothing.
#[derive(Default)]
pub struct LashScratch {
    order: Vec<Entity>,
    lashes: Vec<(u64, Entity)>,
}

// ---- Target picking ------------------------------------------------------

/// Where a lash is aimed: the impact point plus what the full hit lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LashPick {
    pub pos: Vec2,
    pub target: LashTarget,
}

#[derive(Clone, Copy)]
struct Candidate {
    /// The pick key: distance from the choosing point (the rat, or the aim
    /// point when an aim exists).
    dist_sq: f32,
    /// Enemy bodies first, boss hit points second (the rule every aiming
    /// weapon shares).
    rank: u8,
    /// `SpawnId` for an enemy, the point index for a boss point.
    id: u64,
    pos: Vec2,
    target: LashTarget,
}

const EMPTY_CANDIDATE: Candidate = Candidate {
    dist_sq: f32::MAX,
    rank: u8::MAX,
    id: u64::MAX,
    pos: Vec2::ZERO,
    target: LashTarget::Boss,
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

/// **The one place the lash's target is chosen.**
///
/// The nearest eligible body in reach, ties by `SpawnId`, enemies ahead of the
/// boss's hit points; the walk then spends one line-of-sight query per
/// candidate, nearest first, and returns the first body the tail can actually
/// reach (walls stop a lash: it is a tail, not a bullet).
///
/// `aim` is **the hook the mouse/gamepad aim worker plugs into**. `None` (the
/// keyboard/auto-fire case) picks the body nearest the *rat*; `Some(point)`
/// picks the eligible body nearest that world point instead, so an aimed
/// lash goes where the player is pointing rather than behind them. Everything
/// else — the reach, the tie order, the wall stop — is shared, so the aim
/// branch is one argument at the call site in [`fire_spiked_tail`].
pub(crate) fn pick_lash_target(
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    from: Vec2,
    reach: f32,
    aim: Option<Vec2>,
) -> Option<LashPick> {
    let reach_sq = reach * reach;
    // With an aim, "nearest" is measured from the aim point; without, from the
    // rat's rump.
    let key_from = aim.unwrap_or(from);
    let mut cand = [EMPTY_CANDIDATE; CANDIDATES];
    let mut n = 0;
    for &entity in order {
        let Ok((_, id, e)) = enemies.get(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // The reach is always the tail's: an aim cannot stretch the weapon.
        let dist_sq = e.pos.dist_sq(from);
        if dist_sq.is_nan() || dist_sq > reach_sq {
            continue;
        }
        push_candidate(
            &mut cand,
            &mut n,
            Candidate {
                dist_sq: e.pos.dist_sq(key_from),
                rank: 0,
                id: id.0,
                pos: e.pos,
                target: LashTarget::Enemy(entity),
            },
        );
    }
    for (index, point) in boss_points.iter().enumerate() {
        let dist_sq = point.dist_sq(from);
        if dist_sq.is_nan() || dist_sq > reach_sq {
            continue;
        }
        push_candidate(
            &mut cand,
            &mut n,
            Candidate {
                dist_sq: point.dist_sq(key_from),
                rank: 1,
                id: index as u64,
                pos: point,
                target: LashTarget::Boss,
            },
        );
    }
    cand[..n]
        .iter()
        .find(|c| los_clear(level, from, c.pos, LASH_LOS_RADIUS))
        .map(|c| LashPick {
            pos: c.pos,
            target: c.target,
        })
}

// ---- The shot ------------------------------------------------------------

/// The Spiked Tail's shot: pick one body in reach with a clear line and spawn
/// the lash that will crack at it.
///
/// Returns `false` when nothing eligible is in reach behind cover, which makes
/// `fire_weapons_system` retry next tick instead of serving a full cooldown
/// for a whip that cracked at nothing. Draws no rng and spawns nothing else:
/// the wind-up's whole tell is the drawn tail (`art_rat::draw_tail` with a
/// bristle), so a lash that never lands costs the rng stream nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_spiked_tail(
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    boss_points: &BossTargetable,
    ppos: Vec2,
    reach: f32,
    radius: f32,
    dmg: f32,
    shove: f32,
    shards: u8,
    evolved: bool,
    next_id: &mut NextSpawnId,
    aim: Option<Vec2>,
) -> bool {
    // The aim hook: `None` is auto-fire (nearest the rat); an aimed scheme
    // hands its reticle point through and the lash takes the eligible body
    // nearest it.
    let Some(pick) = pick_lash_target(level, order, enemies, boss_points, ppos, reach, aim) else {
        return false;
    };
    commands.spawn((
        Lash {
            origin: ppos,
            target: pick.pos,
            primary: match pick.target {
                LashTarget::Enemy(entity) => Some(LashTarget::Enemy(entity)),
                LashTarget::Boss => None,
            },
            boss: pick.target == LashTarget::Boss,
            reach,
            radius,
            dmg,
            shove,
            shards,
            evolved,
            timer: 0.0,
            struck: false,
            dragged: false,
        },
        next_id.take(),
    ));
    true
}

// ---- The crack -----------------------------------------------------------

/// Land one lash's crack: the primary hit, the boss, the small burst, the
/// shards and the feedback.
///
/// It never moves a body (only the scattered spike shards, which are new
/// entities), so unlike [`drag`] it cannot invalidate the spatial grid.
#[allow(clippy::too_many_arguments)]
fn crack(
    lash: &Lash,
    commands: &mut Commands,
    level: &Level,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    next_id: &mut NextSpawnId,
    shake: &mut Shake,
) {
    let kind = WeaponKind::SpikedTail;
    let burst = lash.dmg * LASH_BURST_FRAC;
    let from = lash.origin;
    let point = lash.target;
    let mut killed_any = false;

    // 1. The primary body: the full lash, a stagger and a shove.
    if let Some(LashTarget::Enemy(primary)) = lash.primary {
        if let Ok((_, _, mut e)) = enemies.get_mut(primary) {
            // It has to still be where the tail is pointing (the point follows
            // the body, so this only fails for a body that died) and still
            // behind a clear line.
            let reachable = e.alive()
                && e.pos.dist(point) <= e.radius + 8.0
                && los_clear(level, from, e.pos, LASH_LOS_RADIUS);
            if reachable {
                let hp_before = e.hp.max(0.0);
                // Directional armour: the lash arrives from the rat.
                let applied = e.hurt_from(lash.dmg, from);
                let killed = !e.alive();
                killed_any |= killed;
                ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
                let pos = e.pos;
                let radius = e.radius;
                let id = e.id;
                // Heavy bodies (an elite, a named miniboss, a Brute, a curled
                // Pillbug) keep their footing: a shorter shove, no stagger.
                let heavy = e.kind.heavy() || e.elite || e.named;
                let scale = if heavy { LASH_HEAVY_SCALE } else { 1.0 };
                if !heavy {
                    e.stun = e.stun.max(LASH_STAGGER);
                }
                let out = (pos - from).normalized();
                e.vel = out * lash.shove * scale;
                damage_numbers.record(id, pos, radius, applied, killed, DamageSource::Lash);
            }
        }
    }

    // 2. The boss, when the lash was aimed at one of its hit shapes: the full
    //    lash damage there. (It can take the burst once on top, below.)
    if lash.boss {
        for mut boss in bosses.iter_mut() {
            if boss.dead {
                continue;
            }
            let shapes = boss.hit_shapes();
            if shapes.iter().next().is_none()
                || !shapes.circle_hits_grounded(point, BOSS_LOCK_SLOP)
            {
                continue;
            }
            if level
                .tiles
                .segment_hit(from, boss.pos, LASH_LOS_RADIUS)
                .is_some()
            {
                continue;
            }
            let amount = lash.dmg * boss.damage_mult;
            if !boss.take_damage(amount) {
                continue;
            }
            killed_any |= boss.hp <= 0.0;
            ledger.add_boss(kind, amount);
            damage_numbers.record(
                boss.id,
                boss.pos,
                boss.kind.body_radius(),
                amount,
                boss.hp <= 0.0,
                DamageSource::Lash,
            );
        }
    }

    // 3. The burst at the impact point. It never double-hits the primary body
    //    (it took the whole lash instead), and it is floor geometry: masonry
    //    stops it.
    let primary_entity = match lash.primary {
        Some(LashTarget::Enemy(entity)) => Some(entity),
        _ => None,
    };
    for &entity in order {
        if Some(entity) == primary_entity {
            continue;
        }
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || e.pos.dist(point) > lash.radius + e.radius {
            continue;
        }
        if !los_clear(level, point, e.pos, FIELD_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // The burst arrives from the impact point, not from the rat.
        let applied = e.hurt_from(burst, point);
        let killed = !e.alive();
        killed_any |= killed;
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let radius = e.radius;
        let id = e.id;
        damage_numbers.record(id, pos, radius, applied, killed, DamageSource::Lash);
    }
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        if !boss.hit_shapes().circle_hits_grounded(point, lash.radius) {
            continue;
        }
        if level
            .tiles
            .segment_hit(point, boss.pos, FIELD_LOS_RADIUS)
            .is_some()
        {
            continue;
        }
        let amount = burst * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        killed_any |= boss.hp <= 0.0;
        ledger.add_boss(kind, amount);
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            boss.hp <= 0.0,
            DamageSource::Lash,
        );
    }

    // 4. Spikes break off and fly on (level 4+): ordinary friendly bullets in a
    //    narrow fan, so every existing projectile rule (walls, shells, the
    //    damage ledger column) applies to them.
    let dir = (point - from).normalized();
    if lash.shards > 0 {
        let base = dir.angle();
        let n = lash.shards as i32;
        for k in 0..n {
            let spread = if n > 1 {
                (k as f32 - (n - 1) as f32 / 2.0) * SHARD_FAN
            } else {
                0.0
            };
            let mut b = Bullet::new(
                point,
                Vec2::from_angle(base + spread) * SHARD_SPEED,
                lash.dmg * LASH_SHARD_FRAC,
                Faction::Player,
                kind.color(),
            );
            b.size = 1;
            b.radius = 1.5;
            b.life = SHARD_LIFE;
            b.weapon = Some(kind);
            commands.spawn((b, next_id.take()));
        }
    }

    // 5. Feedback: the spiked impact star with its burst ring, the shards' own
    //    glints, a small screen shake. All from the impact point.
    vfx::emit_lash_impact(
        particles,
        &mut rng.0,
        point,
        dir,
        lash.radius,
        killed_any,
    );
    if lash.shards > 0 {
        vfx::emit_lash_shards(particles, &mut rng.0, point, dir, lash.shards as usize);
    }
    shake.amount = shake.amount.max(LASH_SHAKE);
}

/// **Barbed Wire** (design: the lash hooks, the recoil reels the catch in).
///
/// The caught body is dragged from the impact point to the rat's feet; each
/// body it is dragged *through* takes one burst, exactly as if the lash had
/// cracked on it. Heavy bodies (elites, named minibosses, `EnemyKind::heavy()`)
/// are not dragged at all — the hook slips off and only the lash hit stands.
///
/// Returns whether a body moved (the caller re-files the spatial grid).
#[allow(clippy::too_many_arguments)]
fn drag(
    lash: &Lash,
    level: &Level,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
) -> bool {
    let Some(LashTarget::Enemy(primary)) = lash.primary else {
        return false;
    };
    // Everything read off the caught body before the walk below (which needs
    // the query mutably).
    let Some((from, radius, heavy)) = enemies.get(primary).ok().and_then(|(_, _, e)| {
        e.alive()
            .then(|| (e.pos, e.radius, e.kind.heavy() || e.elite || e.named))
    }) else {
        return false;
    };
    // The hook slips off anything heavy: only the lash hit stands.
    if heavy {
        return false;
    }
    let gap = LASH_DRAG_GAP + radius;
    let dir = (from - lash.origin).normalized();
    let to = if from.dist(lash.origin) <= gap {
        // Already at the rat's feet: nothing to reel in.
        from
    } else {
        lash.origin + dir * gap
    };

    // Every body the drag crosses takes the burst once, in `SpawnId` order.
    let burst = lash.dmg * LASH_BURST_FRAC;
    let mut crossed = 0u32;
    for &entity in order {
        if entity == primary {
            continue;
        }
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // Distance from the dragged body's centre to the line it is dragged
        // along: a body whose disc the catch sweeps through is crossed.
        let closest = closest_on_segment(from, to, e.pos);
        if e.pos.dist(closest) > lash.radius + e.radius {
            continue;
        }
        if !los_clear(level, from, e.pos, FIELD_LOS_RADIUS) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        let applied = e.hurt_from(burst, from);
        let killed = !e.alive();
        ledger.add(WeaponKind::SpikedTail, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let enemy_radius = e.radius;
        let id = e.id;
        damage_numbers.record(
            id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Lash,
        );
        crossed += 1;
    }

    let moved = if to != from {
        if let Ok((_, _, mut e)) = enemies.get_mut(primary) {
            e.pos = level
                .tiles
                .resolve_circle(to, e.radius, SizeClass::Small, None);
            e.vel = Vec2::ZERO;
            e.stun = e.stun.max(LASH_STAGGER);
            true
        } else {
            false
        }
    } else {
        false
    };
    vfx::emit_lash_drag(particles, &mut rng.0, from, to, crossed);
    moved
}

/// Closest point to `p` on the segment `a..b` (`a` for a degenerate segment).
#[inline]
fn closest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.len_sq();
    if len_sq <= 1e-6 {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

// ---- The tick ------------------------------------------------------------

/// Advance every lash in flight: follow the body, crack, drag, expire.
///
/// Chained right after the auto-fire pass (slot 11b2, `schedule.rs`), so the
/// cached [`EnemyIndex`] is the live `SpawnId` order here and the crack's
/// damage lands before `projectiles::bullets_system`, `combat` and the reap —
/// a spike shard spawned by a crack flies on the same tick and a lash kill is
/// looted on it.
#[allow(clippy::too_many_arguments)]
pub fn spiked_tail_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    player: Res<Player>,
    mut grid: ResMut<SpatialGrid>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut next_id: ResMut<NextSpawnId>,
    mut shake: ResMut<Shake>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut lashes: Query<(Entity, &SpawnId, &mut Lash)>,
    // Reused buffers: the narrow-harness fallback enemy order (the schedule's
    // cached `EnemyIndex` is used whenever it is the live list) and this tick's
    // lash entities in `SpawnId` order.
    mut scratch: Local<LashScratch>,
) {
    // No lash in flight: nothing to advance, and (importantly) no rng draw.
    if lashes.is_empty() {
        return;
    }
    let dt = input.dt;

    // Both scratch buffers at once: one `DerefMut` through the `Local`, then
    // two disjoint fields, so the enemy order can stay borrowed while the lash
    // list is filled (a field split through `Deref` would not be disjoint).
    let LashScratch {
        order: order_buf,
        lashes: lash_buf,
    } = &mut *scratch;

    // The tick's stable enemy order, exactly as `weapons.rs` derives it.
    let cached = index.0.len() == enemies.iter().len();
    if !cached {
        order_buf.clear();
        order_buf.extend(enemies.iter().map(|(entity, _, _)| entity));
        order_buf.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
    }
    let order: &[Entity] = if cached {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        order_buf.as_slice()
    };

    // `sort_by_key` on a mutable query allocates an entity list per call, so
    // the lashes are gathered into the reused buffer and sorted as
    // `(SpawnId, Entity)` — the repo's idiom (`enemy_move`, `combat`).
    lash_buf.clear();
    for (entity, id, _) in lashes.iter() {
        lash_buf.push((id.0, entity));
    }
    lash_buf.sort_unstable_by_key(|&(id, _)| id);

    let mut moved = false;
    for &(_, entity) in lash_buf.iter() {
        let Ok((_, _, mut lash)) = lashes.get_mut(entity) else {
            continue;
        };
        lash.timer += dt;
        // The tail's root follows the rat: the whip stays attached to the rump
        // it is really on, whatever the rat did during the wind-up.
        lash.origin = player.pos;
        // The impact point follows its body while the body lives (a lash that
        // whiffs a walking rat-sized target reads as broken). A boss point is
        // followed through the boss's own body.
        if let Some(LashTarget::Enemy(primary)) = lash.primary {
            if let Ok((_, _, e)) = enemies.get(primary) {
                if e.alive() {
                    lash.target = e.pos;
                }
            }
        } else if lash.boss {
            for boss in bosses.iter() {
                if !boss.dead {
                    lash.target = boss.pos;
                    break;
                }
            }
        }

        if !lash.struck && lash.timer >= LASH_WINDUP {
            lash.struck = true;
            crack(
                &lash,
                &mut commands,
                &level,
                order,
                &mut enemies,
                &mut bosses,
                &mut rng,
                &mut particles,
                &mut damage_numbers,
                &mut ledger,
                &mut next_id,
                &mut shake,
            );
        }
        // Independent of the branch above so a huge `dt` still resolves both
        // beats in one tick (the schedule is fixed-step in practice, but a
        // narrow test drives whatever `dt` it likes).
        if lash.struck && lash.evolved && !lash.dragged && lash.timer >= LASH_RECOIL_AT {
            lash.dragged = true;
            moved |= drag(
                &lash,
                &level,
                order,
                &mut enemies,
                &mut rng,
                &mut particles,
                &mut damage_numbers,
                &mut ledger,
            );
        }
        if lash.timer >= LASH_TOTAL {
            commands.entity(entity).despawn();
        }
    }

    // A drag moved a body. `EnemyIndex` is unchanged (nothing spawned or
    // despawned), so re-filling the buckets from it is one pass and every
    // later reader this tick sees the body where it actually is.
    if moved {
        let arena = level.arena;
        grid.reset(arena, GRID_CELL);
        grid.fill(
            index
                .0
                .iter()
                .filter_map(|&entity| enemies.get(entity).ok().map(|(_, _, e)| e.pos)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::resources::{DamageLedger, EnemyIndex};
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS, TileRect};
    use crate::game::loadout::{PassiveKind, Upgrade};
    use crate::game::weapon::{Weapon, LASH_CYCLE, LASH_DAMAGE, LASH_RADIUS, LASH_REACH};
    use crate::game::world::World;
    use rand::SeedableRng;

    const DT: f32 = 1.0 / 60.0;

    /// Wipe the generated layout's walls so a test measures the weapon, not
    /// the map.
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.tiles.rebuild_masks();
    }

    /// A fresh, empty arena with one Spiked Tail armed and ready, the rat
    /// facing +x.
    fn arena(seed: u64, level: u8) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        open_map(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.loadout.weapons = vec![Weapon {
            kind: WeaponKind::SpikedTail,
            level,
            timer: 0.0,
            evolved: false,
        }];
        w
    }

    /// The fixture `weapons.rs` uses for its line-of-sight tests: an open map,
    /// the rat pressed up against (6 px from) a 3-cell solid column whose near
    /// face is `x = 176`. Deliberately close, so "inside the lash's reach but
    /// behind the wall" is a real configuration at level 1.
    fn wall_between(w: &mut World) -> f32 {
        open_map(w);
        w.player.pos = Vec2::new(170.0, 168.0);
        w.player.face = Vec2::new(1.0, 0.0);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let (cx, cy) = (11, 10);
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// Fire once on a zero-length tick, so the observed cooldown is exactly the
    /// design's cycle. Returns whether a lash was spawned.
    fn run_fire(w: &mut World) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        w.player.loadout.weapons[0].timer > before
    }

    /// [`run_fire`] with a reticle: the lash takes the eligible body nearest
    /// `aim` instead of the one nearest the rat.
    fn run_fire_aimed(w: &mut World, aim: Vec2) -> bool {
        let before = w.player.loadout.weapons[0].timer;
        w.begin_tick_with_aim(0.0, Vec2::ZERO, false, false, Some(aim));
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        w.player.loadout.weapons[0].timer > before
    }

    fn run_lash(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(spiked_tail_system).unwrap();
        w.end_tick();
    }

    /// Every live lash, in `SpawnId` order.
    fn lashes(w: &mut World) -> Vec<Lash> {
        let mut q = w.ecs.query::<(&SpawnId, &Lash)>();
        let mut v: Vec<(u64, Lash)> = q.iter(&w.ecs).map(|(id, l)| (id.0, *l)).collect();
        v.sort_unstable_by_key(|&(id, _)| id);
        v.into_iter().map(|(_, l)| l).collect()
    }

    /// Fire, then run the wind-up out so the crack lands (but not the recoil's
    /// full span, so the lash entity is still there to read).
    fn fire_and_crack(w: &mut World) -> Lash {
        assert!(run_fire(w), "the lash must have a target");
        // One tick past the wind-up lands the crack.
        let ticks = (LASH_WINDUP / DT).floor() as usize + 1;
        for _ in 0..ticks {
            run_lash(w, DT);
        }
        lashes(w).first().copied().expect("the lash is still in flight")
    }

    fn hp(w: &World, entity: Entity) -> f32 {
        w.ecs.get::<Enemy>(entity).unwrap().hp
    }

    fn dealt(w: &World) -> f32 {
        w.ecs.resource::<DamageLedger>().dealt[WeaponKind::SpikedTail.index()]
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

    fn stack(w: &mut World, passive: PassiveKind) {
        for _ in 0..passive.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(passive));
        }
    }

    // ---- Targeting ------------------------------------------------------

    /// One lash at a time, aimed at the nearest body in reach, and the whole
    /// table of level numbers is what the shot actually uses.
    #[test]
    fn the_lash_targets_the_nearest_body_in_reach() {
        let mut w = arena(1, 1);
        let p = w.player.pos;
        let far = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 6.0), 400.0);
        assert!(run_fire(&mut w));
        let l = lashes(&mut w);
        assert_eq!(l.len(), 1, "one lash per shot");
        assert_eq!(l[0].target, p + Vec2::new(20.0, 6.0), "the nearest body");
        assert_eq!(l[0].primary, Some(LashTarget::Enemy(near)));
        assert_eq!(l[0].reach, LASH_REACH[0]);
        assert_eq!(l[0].radius, LASH_RADIUS[0]);
        assert_eq!(l[0].dmg, LASH_DAMAGE[0]);
        assert_eq!(l[0].shards, 0, "no shards below level 4");
        assert!(!l[0].struck, "the crack waits for the wind-up");
        assert_ne!(near, far);
    }

    /// Ties on distance break by `SpawnId` — the lowest id wins — whatever
    /// order the bodies were queried in.
    #[test]
    fn distance_ties_break_by_spawn_id() {
        for flip in [false, true] {
            let mut w = arena(2, 1);
            let p = w.player.pos;
            let a = if flip {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 18.0), 400.0)
            } else {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 400.0)
            };
            let _b = if flip {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(18.0, 0.0), 400.0)
            } else {
                w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(0.0, 18.0), 400.0)
            };
            assert!(run_fire(&mut w));
            let l = lashes(&mut w);
            assert_eq!(
                l[0].primary,
                Some(LashTarget::Enemy(a)),
                "the lowest SpawnId wins an exact tie (flip = {flip})"
            );
        }
    }

    /// A body past the reach is not a target at all: the weapon stays ready
    /// instead of serving a cooldown for a whip that would crack at nothing.
    #[test]
    fn out_of_reach_fires_nothing_and_keeps_the_weapon_ready() {
        for level in 1..=5u8 {
            let mut w = arena(3 + level as u64, level);
            let p = w.player.pos;
            let reach = LASH_REACH[(level - 1) as usize];
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(reach + 14.0, 0.0), 400.0);
            assert!(!run_fire(&mut w), "L{level}: out of reach must not fire");
            assert!(lashes(&mut w).is_empty());
            assert_eq!(w.player.loadout.weapons[0].timer, 0.0, "retry next tick");
        }
    }

    /// Reach grows with level: the same body is out of reach at L1 and in
    /// reach at L5 strictly by the table.
    #[test]
    fn reach_grows_with_level() {
        for level in 1..=5u8 {
            let mut w = arena(10 + level as u64, level);
            let p = w.player.pos;
            let reach = LASH_REACH[(level - 1) as usize];
            w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(reach - 1.0, 0.0), 400.0);
            assert!(run_fire(&mut w), "L{level}: a body inside the reach is hit");
            assert_eq!(lashes(&mut w)[0].reach, reach);
        }
    }

    /// A wall between the rat and the body stops the lash: no target, no shot,
    /// no damage. With a clear second line (the body beside the column) the
    /// weapon fires at *that* body instead.
    #[test]
    fn a_wall_blocks_the_lash() {
        let mut w = arena(20, 1);
        let face = wall_between(&mut w);
        let p = w.player.pos;
        // Straight through the column: 30 px away (well inside L1's 44 px
        // reach), but not visible.
        let hidden = w.spawn_enemy(EnemyKind::Cat, Vec2::new(face + 24.0, p.y), 400.0);
        let before = hp(&w, hidden);
        assert!(!run_fire(&mut w), "a walled-off body is not a target");
        assert!(lashes(&mut w).is_empty(), "no lash was thrown at a wall");
        assert_eq!(hp(&w, hidden), before);
        // The same body stepping out from behind the column is reachable.
        let open = Vec2::new(p.x, p.y + 24.0);
        w.ecs.get_mut::<Enemy>(hidden).unwrap().pos = open;
        assert!(run_fire(&mut w), "a clear line fires");
    }

    // ---- The crack ------------------------------------------------------

    /// The primary takes the full lash (per the table for its level), the rest
    /// of the burst takes 40 % of it, the primary is never double-hit, and the
    /// whole thing is booked to the Spiked Tail column.
    #[test]
    fn the_crack_hits_the_primary_once_and_the_burst_for_the_rest() {
        for level in 1..=5u8 {
            let mut w = arena(30 + level as u64, level);
            let p = w.player.pos;
            let dmg = LASH_DAMAGE[(level - 1) as usize];
            let radius = LASH_RADIUS[(level - 1) as usize];
            let primary = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
            // Inside the burst radius of the impact point.
            let splash = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 6.0), 400.0);
            // Well outside it (the gap clears the target's own body radius).
            let far = w.spawn_enemy(
                EnemyKind::Brute,
                p + Vec2::new(20.0, radius + 24.0),
                400.0,
            );
            let before = [hp(&w, primary), hp(&w, splash), hp(&w, far)];
            let _ = fire_and_crack(&mut w);
            let burst = dmg * LASH_BURST_FRAC;
            let lost = |w: &World, i: usize| before[i] - hp(w, [primary, splash, far][i]);
            assert!(
                (lost(&w, 0) - dmg).abs() < 1e-3,
                "L{level}: the primary takes the whole lash, once ({})",
                lost(&w, 0)
            );
            assert!(
                (lost(&w, 1) - burst).abs() < 1e-3,
                "L{level}: the burst lands once at 40 % ({})",
                lost(&w, 1)
            );
            assert_eq!(lost(&w, 2), 0.0, "L{level}: outside the burst");
            let want = (dmg + burst) as f64;
            assert!(
                (dealt(&w) as f64 - want).abs() < 0.05,
                "L{level}: ledger {:.2} != {want:.2}",
                dealt(&w)
            );
        }
    }

    /// Every lash has its own cycle from the table: the weapon fires again
    /// only after `LASH_CYCLE[level - 1]`.
    #[test]
    fn the_lash_fires_on_its_design_cycle() {
        for level in 1..=5u8 {
            let mut w = arena(40 + level as u64, level);
            w.spawn_enemy(
                EnemyKind::Brute,
                w.player.pos + Vec2::new(20.0, 0.0),
                4000.0,
            );
            assert!(run_fire(&mut w));
            let cycle = LASH_CYCLE[(level - 1) as usize];
            let timer = w.player.loadout.weapons[0].timer;
            assert!(
                (timer - cycle).abs() < 1e-4,
                "L{level}: a shot serves {timer:.3} s, the table says {cycle}"
            );
        }
    }

    /// The crack staggers and shoves the primary, through the same shove hook
    /// (Counterweight) the other weapons read; heavy bodies are nudged less
    /// and are not staggered.
    #[test]
    fn the_crack_shoves_and_staggers_the_primary() {
        let mut w = arena(50, 1);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        assert!(hp(&w, body) > 0.0, "the body survives the crack under test");
        let e = w.ecs.get::<Enemy>(body).unwrap();
        assert!(e.stun > 0.0, "a staggered body");
        assert!(e.vel.x > 0.0, "shoved away from the rat: {:?}", e.vel);
        assert!(e.vel.y.abs() < 1e-3);

        // Counterweight multiplies the native shove.
        let mut w = arena(51, 1);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let bare = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        let mut w = arena(51, 1);
        let p = w.player.pos;
        stack(&mut w, PassiveKind::Counterweight);
        let body = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let heavy_handed = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        assert!(
            heavy_handed > bare * 1.1,
            "Counterweight must shove harder: {bare} -> {heavy_handed}"
        );

        // A Brute is heavy: no stagger, a token shove.
        let mut w = arena(52, 1);
        let p = w.player.pos;
        let brute = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let _ = fire_and_crack(&mut w);
        let e = w.ecs.get::<Enemy>(brute).unwrap();
        assert_eq!(e.stun, 0.0, "a Brute keeps its feet");
        assert!(e.vel.len() < 40.0, "and barely moves: {:?}", e.vel);
    }

    /// Level 4 and 5 break spike shards off the impact: two and three ordinary
    /// friendly bullets in a narrow fan, tagged with the weapon so the ledger
    /// and the impact art credit it.
    #[test]
    fn spike_shards_break_off_from_level_four() {
        let mut shipped = [0usize; 6];
        for level in 1..=5u8 {
            let mut w = arena(60 + level as u64, level);
            w.spawn_enemy(
                EnemyKind::Brute,
                w.player.pos + Vec2::new(20.0, 0.0),
                4000.0,
            );
            let _ = fire_and_crack(&mut w);
            let mut q = w.ecs.query::<&Bullet>();
            let shards: Vec<Bullet> = q
                .iter(&w.ecs)
                .filter(|b| b.weapon == Some(WeaponKind::SpikedTail))
                .cloned()
                .collect();
            shipped[level as usize] = shards.len();
            assert_eq!(shards.len(), crate::game::weapon::LASH_SHARDS[level as usize - 1] as usize);
            for b in &shards {
                assert_eq!(b.faction as u8, Faction::Player as u8);
                assert!((b.damage - LASH_DAMAGE[level as usize - 1] * LASH_SHARD_FRAC).abs() < 1e-4);
            }
            if shards.len() > 1 {
                // A narrow fan: they are not all on one line.
                assert!(
                    shards.iter().any(|b| b.vel.y != shards[0].vel.y),
                    "the shards fan out"
                );
            }
        }
        assert_eq!(shipped[1..], [0, 0, 0, 2, 3], "shards from level 4 only");
    }

    // ---- Passives -------------------------------------------------------

    /// Lens Cap stretches the reach (a melee weapon), Long Fuse widens the
    /// burst (a splash), Sharp Teeth lifts the hit and Frenzy shortens the
    /// cycle — each through its own named hook.
    #[test]
    fn every_passive_hook_reaches_the_lash() {
        // Lens Cap: the recorded reach is the stretched one.
        let mut w = arena(70, 1);
        stack(&mut w, PassiveKind::LensCap);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let want = w.player.loadout.stats.melee_reach(LASH_REACH[0]);
        assert!(want > LASH_REACH[0]);
        assert_eq!(lashes(&mut w)[0].reach, want);

        // Long Fuse: the recorded burst radius is the stretched one.
        let mut w = arena(71, 1);
        stack(&mut w, PassiveKind::LongFuse);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let want = w.player.loadout.stats.splash_radius(LASH_RADIUS[0]);
        assert!(want > LASH_RADIUS[0]);
        assert_eq!(lashes(&mut w)[0].radius, want);

        // Sharp Teeth: more damage actually lands.
        let mut bare = arena(72, 1);
        let p = bare.player.pos;
        let target = bare.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let start = hp(&bare, target);
        let _ = fire_and_crack(&mut bare);
        let plain = start - hp(&bare, target);
        let mut sharp = arena(72, 1);
        stack(&mut sharp, PassiveKind::Damage);
        let p = sharp.player.pos;
        let target = sharp.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let start = hp(&sharp, target);
        let _ = fire_and_crack(&mut sharp);
        let filed = start - hp(&sharp, target);
        assert!(filed > plain * 1.2, "Sharp Teeth: {plain} -> {filed}");

        // Frenzy: the same cooldown serves a shorter cycle.
        let mut w = arena(73, 1);
        stack(&mut w, PassiveKind::FireRate);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let cycled = w.player.loadout.weapons[0].timer;
        assert!(cycled < LASH_CYCLE[0], "Frenzy must shorten the cycle");
    }

    // ---- Bosses ---------------------------------------------------------

    /// The lash targets a boss hit shape and lands the full hit there; the
    /// burst can hit the boss once more on top.
    #[test]
    fn the_boss_takes_the_lash_and_one_burst() {
        for kind in [BossKind::Broom, BossKind::Mower] {
            let mut w = arena(80, 5);
            let boss = boss_at(&mut w, kind, Vec2::new(22.0, 0.0));
            let lash = fire_and_crack(&mut w);
            assert!(lash.boss, "{kind:?}: the pick was a boss hit point");
            assert_eq!(
                lash.primary, None,
                "{kind:?}: a boss has no Enemy entity to follow"
            );
            let lost = 5000.0 - w.ecs.get::<Boss>(boss).unwrap().hp;
            let want = LASH_DAMAGE[4] + LASH_DAMAGE[4] * LASH_BURST_FRAC;
            assert!(
                (lost - want).abs() < 1e-2,
                "{kind:?}: the boss takes the lash and one burst: {lost} != {want}"
            );
            let booked = w.ecs.resource::<DamageLedger>().boss[WeaponKind::SpikedTail.index()];
            assert!((booked - lost).abs() < 1e-2, "{kind:?}: ledger {booked}");
        }
    }

    /// A miniboss is an ordinary `Enemy` body: it is lashed like one (and,
    /// being heavy, is not dragged by the evolved weapon).
    #[test]
    fn a_miniboss_body_takes_the_lash_but_is_never_dragged() {
        use crate::game::ecs::systems::miniboss::MinibossKind;
        let mut w = arena(81, 5);
        let p = w.player.pos;
        let mb = w.spawn_miniboss(MinibossKind::Corkscrew, 0, p + Vec2::new(20.0, 0.0));
        w.player.loadout.weapons[0].evolved = true;
        let before = w.ecs.get::<Enemy>(mb).unwrap().hp;
        let _ = fire_and_crack(&mut w);
        for _ in 0..60 {
            run_lash(&mut w, DT);
        }
        let e = w.ecs.get::<Enemy>(mb).unwrap();
        assert!(e.hp < before, "the miniboss takes the lash");
        assert_eq!(
            e.pos,
            p + Vec2::new(20.0, 0.0),
            "a named miniboss is never dragged"
        );
    }

    // ---- The evolution --------------------------------------------------

    /// **Barbed Wire**: the recoil drags the catch home, each body it crosses
    /// takes the burst once, and heavy bodies are not dragged at all.
    #[test]
    fn barbed_wire_drags_the_catch_through_everything_on_the_way() {
        let mut w = arena(90, 5);
        let p = w.player.pos;
        let caught = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
        // Two bodies on the line between the rat and the catch, so the drag
        // sweeps both discs — far enough from the impact that the crack's own
        // burst cannot reach them (this measures the drag, not the splash).
        let crossed_a = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(26.0, 1.0), 400.0);
        let crossed_b = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(9.0, -1.0), 400.0);
        // Off the line: never crossed.
        let aside = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 30.0), 400.0);
        let start = [
            hp(&w, caught),
            hp(&w, crossed_a),
            hp(&w, crossed_b),
            hp(&w, aside),
        ];
        w.player.loadout.weapons[0].evolved = true;
        // Auto-fire would take the nearest body (`crossed_b`); the far catch
        // with bodies in between is an aimed lash, pointed at the catch.
        assert!(run_fire_aimed(&mut w, p + Vec2::new(50.0, 0.0)));
        let l = lashes(&mut w);
        assert!(l[0].evolved);
        assert_eq!(l[0].primary, Some(LashTarget::Enemy(caught)));
        let burst = LASH_DAMAGE[4] * LASH_BURST_FRAC;

        // Run past the wind-up and the recoil's start.
        let ticks = (LASH_RECOIL_AT / DT).floor() as usize + 2;
        for _ in 0..ticks {
            run_lash(&mut w, DT);
        }
        let dragged = w.ecs.get::<Enemy>(caught).unwrap().pos;
        assert!(
            dragged.dist(p) < 16.0,
            "the catch is reeled in to the rat's feet: {dragged:?}"
        );
        for (i, crossed) in [crossed_a, crossed_b].into_iter().enumerate() {
            let lost = start[1 + i] - hp(&w, crossed);
            assert!(
                (lost - burst).abs() < 1e-3,
                "a crossed body takes the burst once ({lost} != {burst})"
            );
        }
        assert_eq!(hp(&w, aside), start[3], "a body off the line is untouched");
        // The catch itself keeps the lash hit and is not burst again by the drag.
        let caught_lost = start[0] - hp(&w, caught);
        assert!(
            (caught_lost - LASH_DAMAGE[4]).abs() < 1e-3,
            "the catch takes the lash, not the drag's bursts: {caught_lost}"
        );
    }

    /// Heavy bodies are bookkeeping-only: the hook slips off, the lash hit
    /// stands, and nothing is moved.
    #[test]
    fn barbed_wire_never_drags_a_heavy_body() {
        let mut w = arena(91, 5);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        let brute = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(50.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        for _ in 0..60 {
            run_lash(&mut w, DT);
        }
        assert_eq!(
            w.ecs.get::<Enemy>(brute).unwrap().pos,
            p + Vec2::new(50.0, 0.0),
            "a Brute is not a fish on a line"
        );
    }

    /// The evolution changes behaviour, not numbers: same cycle, same damage
    /// per hit, same reach.
    #[test]
    fn barbed_wire_keeps_the_timing_family() {
        let mut w = arena(92, 3);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        let l = lashes(&mut w)[0];
        assert_eq!(l.dmg, LASH_DAMAGE[2]);
        assert_eq!(l.reach, LASH_REACH[2]);
        assert_eq!(l.radius, LASH_RADIUS[2]);
        assert!((w.player.loadout.weapons[0].timer - LASH_CYCLE[2]).abs() < 1e-4);
    }

    // ---- Ownership / determinism ---------------------------------------

    /// Without the weapon nothing happens at all: no lash ever appears, no
    /// damage is booked, and a run without it is the same run.
    #[test]
    fn nothing_changes_when_the_weapon_is_not_owned() {
        let mut a = World::new(Theme::Sewer, 100);
        open_map(&mut a);
        a.clear_enemies();
        a.player.loadout.weapons.clear();
        let mut b = World::new(Theme::Sewer, 100);
        open_map(&mut b);
        b.clear_enemies();
        b.player.loadout.weapons.clear();
        let p = a.player.pos;
        a.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        b.spawn_enemy(EnemyKind::Brute, p + Vec2::new(40.0, 0.0), 400.0);
        for _ in 0..90 {
            a.update(DT, Vec2::ZERO, false);
            b.update(DT, Vec2::ZERO, false);
        }
        assert!(lashes(&mut a).is_empty(), "an unowned lash never spawns");
        assert_eq!(dealt(&a), 0.0, "and never deals a point of damage");
        // The same seed plays out identically with and without the weapon.
        assert_eq!(a.player.pos, b.player.pos);
        assert_eq!(a.kills, b.kills);
        assert_eq!(a.enemies().len(), b.enemies().len());
    }

    /// Twin runs: the same seed and the same inputs land the same hits, in the
    /// same order, on every level and both evolution states.
    #[test]
    fn twin_runs_are_deterministic() {
        fn trace(level: u8, evolved: bool) -> (u64, f32) {
            let mut w = arena(123, level);
            w.player.loadout.weapons[0].evolved = evolved;
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            let p = w.player.pos;
            for i in 0..8 {
                let a = i as f32 * 0.8;
                w.spawn_enemy_with_rng(
                    EnemyKind::Brute,
                    p + Vec2::from_angle(a) * (18.0 + i as f32 * 5.0),
                    400.0,
                    &mut rand::rngs::StdRng::seed_from_u64(i as u64),
                );
            }
            for tick in 0..240u32 {
                let dir = match (tick / 30) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(DT, dir, false);
                if tick % 12 == 0 {
                    for v in [
                        w.player.pos.x.to_bits() as u64,
                        w.player.pos.y.to_bits() as u64,
                        w.kills as u64,
                    ] {
                        h ^= v;
                        h = h.wrapping_mul(0x100_0000_01b3);
                    }
                    for e in w.enemies() {
                        for v in [e.pos.x.to_bits() as u64, e.pos.y.to_bits() as u64, e.hp.to_bits() as u64] {
                            h ^= v;
                            h = h.wrapping_mul(0x100_0000_01b3);
                        }
                    }
                }
            }
            (h, dealt(&w))
        }
        for level in [1u8, 3, 5] {
            for evolved in [false, true] {
                let a = trace(level, evolved);
                let b = trace(level, evolved);
                assert_eq!(a, b, "L{level} evolved={evolved}: twin runs diverged");
                assert!(a.1 > 0.0, "L{level} evolved={evolved}: nothing was hit");
            }
        }
    }

    /// The lash never spawns two entities for one shot, and the wind-up's
    /// crack is exactly once however the ticks are cut.
    #[test]
    fn one_crack_per_lash() {
        let mut w = arena(130, 1);
        let p = w.player.pos;
        let target = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(20.0, 0.0), 400.0);
        let before = hp(&w, target);
        assert!(run_fire(&mut w));
        for _ in 0..90 {
            run_lash(&mut w, DT);
        }
        assert!((before - hp(&w, target) - LASH_DAMAGE[0]).abs() < 1e-3);
        assert!(lashes(&mut w).is_empty(), "the lash expired");
    }

    /// The index the grid and the burst both use stays the live `SpawnId`
    /// order: a dragged body is re-filed so a bullet still finds it.
    #[test]
    fn a_dragged_body_is_re_filed_in_the_grid() {
        let mut w = arena(140, 5);
        let p = w.player.pos;
        w.player.loadout.weapons[0].evolved = true;
        let caught = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 400.0);
        assert!(run_fire(&mut w));
        // The single-system harness has no schedule in front of it: build the
        // index and the grid the way the tick does before the lash runs.
        w.begin_tick(0.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::enemy_move::grid_build_system)
            .unwrap();
        w.end_tick();
        let ticks = (LASH_RECOIL_AT / DT).floor() as usize + 2;
        for _ in 0..ticks {
            run_lash(&mut w, DT);
        }
        let pos = w.ecs.get::<Enemy>(caught).unwrap().pos;
        assert!(pos.dist(p) < 14.0);
        // The grid lists the body under its *new* cell.
        let grid = w.ecs.resource::<SpatialGrid>();
        let index = w.ecs.resource::<EnemyIndex>();
        let want = index.0.iter().position(|&e| e == caught).unwrap() as u32;
        let mut out: Vec<u32> = Vec::new();
        grid.neighbors_within(pos, 1.0, &mut out);
        assert!(
            out.contains(&want),
            "the dragged body is filed at its new position"
        );
    }
}
