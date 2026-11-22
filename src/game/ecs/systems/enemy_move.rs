//! (c) Flow-field refresh, enemy steering + shooting, grid build, separation
//! and wall resolution.
//!
//! Ported verbatim from the legacy `World`:
//! - the flow-field refresh call (`flow.update(&level.tiles, player.pos, ..)`);
//! - `World::update_enemies`: per-enemy steering, arena clamp, ranged-enemy
//!   fire (`fire_cd`, boss ring of 10, spitter aimed shot);
//! - the two grid rebuilds + `max_radius`;
//! - `World::separate_enemies` and `World::resolve_enemy_walls`.
//!
//! Resources: `ResMut<FlowField>`, `Res<Level>`, `Res<Player>`,
//! `Res<RunCounters>` (elapsed speed multiplier), `ResMut<SpatialGrid>`,
//! `ResMut<EnemyIndex>`, `ResMut<GridScratch>` (max radius), `ResMut<RngRes>`
//! (not used by these legacy functions but a neighbour resource), `Commands` +
//! `ResMut<NextSpawnId>` (enemy bullets), `Res<NeighborScratch>`.
//!
//! Components: `Enemy` + `SpawnId`; bullets are `Bullet` + `SpawnId`.
//!
//! ## Stable ordering (critical)
//!
//! Legacy indexes enemies by `Vec` position. The ECS port replaces that with
//! `SpawnId` order: `grid_build_system` iterates the enemy query sorted by
//! `SpawnId`, fills the grid in that order **and** stores the entities in
//! `EnemyIndex` in the same order, so a bucket index `j` means
//! `EnemyIndex.0[j]` (exactly legacy `self.enemies[j]`).
//!
//! Separation keeps the **in-place sequential** semantics of the legacy
//! `separate_enemies`: enemy 0's push is visible to enemy 1's neighbour read.
//! It snapshots `(pos, radius)` into a reused `Local` array in `EnemyIndex`
//! order and updates that array in place, so later indices see earlier pushes
//! (legacy `self.enemies[j].pos`); final positions are written back to the
//! components in the same order.
//!
//! ## `EnemyIndex` validity window (this module owns it)
//!
//! `grid_build_system` (schedule point 6) fills [`EnemyIndex`] in `SpawnId`
//! order, and `grid_refresh_system` (schedule point 9) refills it identically
//! from the post-separation/post-wall positions. The grid bucket values are
//! indices into that vector, i.e. legacy `self.enemies[j]`, so any system may
//! use `EnemyIndex.0[j]` instead of re-sorting by `SpawnId` **exactly while no
//! enemy is spawned or despawned**. In the current schedule that is:
//!
//! | after | valid until | usable by |
//! | --- | --- | --- |
//! | `grid_build_system` | `grid_refresh_system` | `separation_system`, `wall_resolve_system`, `grid_refresh_system` |
//! | `grid_refresh_system` | `loot::reap_enemies_system` despawns | `weapons::fire_weapons_system`, `projectiles::*`, `combat::*` (all before the reap) |
//!
//! It is **stale** at these points, so never read it there:
//!
//! - inside `enemy_steering_system` / `enemy_fire_system`: the index still
//!   describes the *previous* tick (or is empty on the first tick) because
//!   `spawn::director_system` may have added enemies since it was built; those
//!   two systems keep their own `Local` ordered buffer instead;
//! - after `loot::reap_enemies_system`, `loot::nuke_reap_system` and any other
//!   despawn site (`World::clear_enemies`, the facade spawn helpers): the
//!   index can hold dead entities, so `Query::get` would fail (it would index
//!   legacy `self.enemies` out of range just the same).
//!
//! Next tick's `grid_build_system` rebuilds it from scratch, so no system has
//! to repair it.
//!
//! ## Neon-merge hooks (re-applied from the merged reference)
//!
//! **None apply here.** The merge diff touched nothing between the director
//! and the weapon fire, and `update_enemies`, `separate_enemies` and
//! `resolve_enemy_walls` in the merged reference are byte-identical to the
//! pre-merge legacy (verified with `diff`): enemy shooting gained no `Sfx`/vfx
//! call, separation and wall ejection gained nothing. The one contract that does touch this file
//! is `Bullet::weapon`: the other team tags only *player* bullets in
//! `fire_one`, so enemy bullets must keep `Bullet::new`'s `None` (asserted by
//! `enemy_bullets_are_not_tagged_with_a_weapon`).
//!
//! ## Split of `update_enemies`
//!
//! The legacy loop runs steer → arena clamp → ranged fire for each enemy.
//! The schedule splits it into `enemy_steering_system` (steer + clamp) then
//! `enemy_fire_system` (cooldown, range check, bullet spawn). Nothing in the
//! fire half is read by the steering half and both walk enemies in `SpawnId`
//! order, so two passes over all enemies are equivalent to one: per-enemy the
//! sequence is unchanged and bullet spawn order (what must match legacy) is
//! preserved. `enemy_fire_system` therefore needs `Res<TickInput>` for `dt`
//! (a parameter the Phase 0 stub was missing); reported to the integrator.

use bevy_ecs::prelude::*;

use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{SizeClass, SpatialGrid};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::GRID_CELL;
use crate::game::ecs::resources::{
    DebugFlags, EnemyIndex, GridScratch, NeighborScratch, NextSpawnId, RunCounters, TickInput,
};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::nav::{FlowField, NavBudget};
use crate::game::player::Player;
use crate::math::Vec2;

/// Scratch entry for the two grid builds: `(SpawnId, Entity, pos, radius)`.
///
/// Kept in a per-system `Local<Vec<_>>` and reused every tick: the port was
/// measured ~45% slower than the `Vec`-based legacy sim, and one allocation
/// per sorted query pass per tick was a measurable part of it. Sorting is
/// `sort_unstable_by_key` on a `SpawnId` that is unique, so it is equivalent
/// to the stable order legacy got from its `Vec`.
type GridEntry = (u64, Entity, Vec2, f32);

/// Legacy `World::update`: refresh the shared flow field toward the player.
///
/// Cheap by construction: a recompute only starts when the player crosses into
/// a new cell, and even then it is spread over frames by [`NavBudget`] (the
/// previous field stays readable meanwhile).
pub fn flow_refresh_system(mut flow: ResMut<FlowField>, level: Res<Level>, player: Res<Player>) {
    flow.update(&level.tiles, player.pos, NavBudget::default());
}

/// Radius of an enemy projectile, used for the line-of-sight check below.
///
/// Every enemy shot is a `Bullet::new` projectile (`enemy_fire_system` never
/// overrides `radius`), so this must stay in step with `Bullet::new`'s radius;
/// `spitter_holds_fire_until_the_line_to_the_rat_is_clear` asserts the two agree.
const ENEMY_SHOT_RADIUS: f32 = 2.0;

/// How hard a ground body is pushed away from a hole on the flatbed, relative
/// to the one-unit steering/flow direction it would otherwise take. 1.0 means
/// the hole wins whenever the route and the hole disagree, which is what makes
/// the horde walk around a broken plank instead of into it.
const PIT_AVOID_PUSH: f32 = 1.0;

/// Legacy `World::update_enemies`: steering + arena clamp, in
/// `SpawnId` order.
///
/// The ordered entity list is built in a reused `Local` buffer: bevy's
/// `QueryIter::sort_by_key` allocates a fresh entity list on every call, which
/// this system (plus the fire pass) paid once per tick.
///
/// (Eight parameters because the trap glue is a separate query — see
/// `traps::TrapSlow` — not a fourth component of the enemy query.)
#[allow(clippy::too_many_arguments)]
pub fn enemy_steering_system(
    input: Res<TickInput>,
    counters: Res<RunCounters>,
    player: Res<Player>,
    flow: Res<FlowField>,
    level: Res<Level>,
    mut order: Local<Vec<(u64, Entity)>>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    slow: Query<&crate::game::ecs::systems::traps::TrapSlow>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    ink: Query<&crate::game::ecs::systems::hazards::InkSlow>,
    // A named miniboss owns its own position during a signature (tell and
    // active window): see `miniboss::Miniboss::holds`.
    minibosses: Query<&crate::game::ecs::systems::miniboss::Miniboss>,
    // Console `freeze on`: the horde holds still. Default false = normal.
    debug: Res<DebugFlags>,
) {
    let dt = input.dt;
    if debug.freeze {
        // Freeze: bodies keep their positions and their timers tick down so
        // the horde resumes exactly where it left off when unfrozen.
        for (_, _, mut e) in enemies.iter_mut() {
            e.contact_cd = (e.contact_cd - dt).max(0.0);
            if e.flash > 0.0 {
                e.flash -= dt;
            }
        }
        return;
    }
    let ppos = player.pos;
    // The horde's own speed ramp: ×1.0 → ×1.75 over the first five minutes,
    // then frozen at that ceiling to the boss (see
    // `director::horde_speed_mult`; the legacy `1.0 + elapsed / 400.0` reached
    // ×3.25 by the boss on a 900 s level), times the per-stage
    // `EnemyMods::speed` already baked into every body at spawn — so stage
    // three's own top is ×1.925.
    let speed_mult = crate::game::director::horde_speed_mult(counters.elapsed);
    // The Wind-up Mouse's key, if one is spinning (at most one at a time). The
    // lure is a straight line at the key, not a second flow field: it only
    // reaches `WINDUP_MOUSE_RADIUS` (48 px, three tiles), it is refused when a
    // wall crosses that line, and it costs one cheap sector query per body
    // inside the radius. Recomputing a field per enemy would cost far more
    // than the trick is worth, and a key that pulls the whole map would make
    // the maze partitions meaningless.
    // The highest `SpawnId` wins if a second key ever exists (only one can by
    // construction), so the query's storage order never decides the lure.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    // Legacy indexed `self.enemies[i]`; here that is `SpawnId` order. Unique
    // ids, so an unstable sort is the same order.
    order.clear();
    order.extend(enemies.iter_mut().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|(id, _)| *id);

    for &(_, entity) in order.iter() {
        let (_, _, mut e) = enemies
            .get_mut(entity)
            .expect("ordered enemy must still be alive");
        // One field per size class: a rat-sized enemy may take a rat-hole, a
        // Cat or a Brute must route around it.
        // A miniboss mid-tell/mid-signature stands still: its state machine
        // owns the body's position, and it is held here (not by a `stun`) so a
        // Sonic Squeak ring still slows it exactly like it slows an elite.
        if minibosses.get(entity).is_ok_and(|mb| mb.holds()) {
            e.vel = e.vel * (1.0 - (12.0 * dt).min(1.0));
            e.contact_cd = (e.contact_cd - dt).max(0.0);
            if e.flash > 0.0 {
                e.flash -= dt;
            }
            continue;
        }
        let mut flow_dir = flow.dir_at(SizeClass::of_enemy(e.kind), e.pos);
        // `traps::TrapSlow` is the per-enemy movement multiplier a sticky board
        // inserts while it holds this enemy (absent = unaffected).
        let glue = slow.get(entity).map_or(1.0, |s| s.0);
        // ---- Wind-up Mouse lure (`systems::powerups`) --------------------
        // An ordinary body inside the key's radius that can *walk* to it (no
        // wall across the line) chases the key instead of the rat; elites and
        // the boss keep their target, so the key is a crowd tool and not a
        // boss exploit. The line test is the mover's own radius, the same
        // question `los_clear` answers for shots and pounces.
        let mut target = ppos;
        if let Some(key) = decoy {
            if crate::game::ecs::systems::powerups::decoy_lures_body(&e)
                && e.pos.dist_sq(key)
                    <= crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS.powi(2)
                && crate::game::ecs::systems::projectiles::los_clear(
                    &level, e.pos, key, e.radius,
                )
            {
                // Spitters keep a stand-off from whatever they chase, so hand
                // steering a point well past the key along the same line: the
                // body then walks up to the key instead of backing away from it.
                target = match e.kind.archetype() {
                    EnemyKind::Spitter => key + (key - e.pos).normalized() * 130.0,
                    _ => key,
                };
                let dir = (key - e.pos).normalized();
                if dir.len_sq() > 0.0 {
                    flow_dir = dir;
                }
            }
        }
        // `hazards::InkSlow` is the Printworks' ink pad: the same idea for a
        // permanent floor stain, kept as its own component so a glue board and
        // a pad can slow the same body without either system clobbering the
        // other's bookkeeping.
        // The flatbed's holes are lethal to a pursuer (35 % of its max HP), so
        // a ground body treats them like a wall: a local push straight away
        // from the nearest hole it is about to step into. Every other theme
        // skips this (the deck is empty) and the check is one small scan over
        // the trailer's three-to-five pits.
        if !level.trailer.is_empty() {
            if let Some(away) = level.trailer.pit_avoid(e.pos, e.radius) {
                let push = flow_dir + away * PIT_AVOID_PUSH;
                flow_dir = if push.len_sq() > 1e-6 { push.normalized() } else { away };
                target = e.pos + flow_dir * 32.0;
            }
        }
        let ink = ink.get(entity).map_or(1.0, |s| s.0);
        // Laundry (`floor_laundry`): a Wool Pill's stall and a planted Peg
        // Snapper stand still; exactly 1.0 for every other body.
        let ink = ink * crate::game::ecs::systems::floor_laundry::gait_mult(&e);
        // Kitchen (`floor_kitchen`): a Crumb Ant holds its heading between
        // course corrections; every other kind gets `flow_dir` back untouched.
        if e.kind == EnemyKind::CrumbAnt {
            flow_dir = crate::game::ecs::systems::floor_kitchen::crumb_ant_course(
                &mut e, target, flow_dir, dt,
            );
        }
        // Lab (`floor_lab`): a Spinner Lid's weave flips handedness; a no-op
        // for every other kind.
        let lid_wander = e.wander;
        // Sewer: the Pipe Eel eases its velocity at half the rate (`floor_sewer`).
        // `None` for every other kind, so nothing else changes.
        let eel_before = crate::game::ecs::systems::floor_sewer::pipe_eel_before(&e);
        e.steer(target, flow_dir, dt, speed_mult * glue * ink);
        crate::game::ecs::systems::floor_sewer::pipe_eel_after(&mut e, eel_before, dt);
        crate::game::ecs::systems::floor_lab::lid_weave(&mut e, lid_wander, dt);
        // Keep enemies inside the arena.
        e.pos = e
            .pos
            .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
    }
}

/// Legacy `World::update_enemies`: ranged enemies shoot.
///
/// Each enemy's shot depends only on that enemy, the player and the level, so
/// the loop walks the query in whatever order the storage yields; the only
/// order-sensitive output, the order bullets are *spawned* in (`SpawnId` order,
/// and the boss's 10 spokes in `k` order), is restored by buffering the shots
/// in a reused `Local` list and sorting it stably by `SpawnId` before spawning.
/// Enemy bullets keep `Bullet::new`'s `weapon: None` (only player bullets are
/// tagged in `World::fire_one` by the neon merge).
///
/// Since bullets stop on walls, a shot at a rat behind cover is a wasted shot:
/// the fire gate asks `projectiles::los_clear` for a clear line at the bullet's
/// own radius ([`ENEMY_SHOT_RADIUS`]) and holds the shot when cover blocks it.
/// The cooldown keeps running on the held path (it is never re-armed), so the
/// shot leaves on the first tick the line opens — no extra rng draw, and the
/// `SpawnId` order of the shots that *do* leave is unchanged.
pub fn enemy_fire_system(
    input: Res<TickInput>,
    player: Res<Player>,
    level: Res<Level>,
    mut commands: Commands,
    mut pending: Local<Vec<(u64, Bullet)>>,
    mut enemies: Query<(&SpawnId, &mut Enemy)>,
    mut next_id: ResMut<NextSpawnId>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let color = level.palette.accent;
    pending.clear();
    for (id, mut e) in enemies.iter_mut() {
        // A stunned body (the Sonic Squeak ring) cannot squeeze the trigger.
        if !e.kind.ranged() || e.stun > 0.0 {
            continue;
        }
        e.fire_cd -= dt;
        let dist = e.pos.dist(ppos);
        if e.fire_cd <= 0.0 && dist < 170.0 {
            // Hold the shot when cover blocks the line to the rat: a shot that
            // would die on the first wall is wasted, and the cooldown keeps
            // ticking (it stays <= 0), so the shot leaves the instant the line
            // opens. No rng draw happens on either path.
            if !crate::game::ecs::systems::projectiles::los_clear(
                &level,
                e.pos,
                ppos,
                ENEMY_SHOT_RADIUS,
            ) {
                continue;
            }
            let is_boss = e.kind == EnemyKind::Boss;
            e.fire_cd = if is_boss { 1.2 } else { 2.2 };
            let from = e.pos;
            let dmg = e.damage * 0.6;
            if is_boss {
                for k in 0..10 {
                    let a = k as f32 / 10.0 * std::f32::consts::TAU;
                    let v = Vec2::from_angle(a) * 60.0;
                    let mut b = Bullet::new(from, v, dmg, Faction::Enemy, (255, 120, 160));
                    b.life = 5.0;
                    b.size = 2;
                    pending.push((id.0, b));
                }
            } else if e.kind == EnemyKind::PepperMill {
                // Kitchen (`floor_kitchen`): the mill's shot leaves as two rays
                // at ±10°, both under this body's `SpawnId`, left ray first.
                for b in crate::game::ecs::systems::floor_kitchen::pepper_mill_shots(
                    from, ppos, dmg, color,
                ) {
                    pending.push((id.0, b));
                }
            } else {
                // Flatbed (`floor_flatbed`): the Toll Gull leads the rat by
                // 0.35 s; every other kind gets `ppos` back untouched.
                let aim = crate::game::ecs::systems::floor_flatbed::shot_target(
                    e.kind, from, ppos, player.vel,
                );
                let v = (aim - from).normalized() * 75.0;
                let mut b = Bullet::new(from, v, dmg, Faction::Enemy, color);
                b.life = 4.0;
                // Laundry: the Starch Bulb's jet leaves slow (no-op otherwise).
                crate::game::ecs::systems::floor_laundry::shape_shot(e.kind, &mut b);
                pending.push((id.0, b));
            }
        }
    }
    // Stable: the boss's spokes share one `SpawnId` and must keep `k` order.
    pending.sort_by_key(|(id, _)| *id);
    for (_, bullet) in pending.drain(..) {
        commands.spawn((bullet, next_id.take()));
    }
}

/// First grid build: before separation. Also records `GridScratch::max_radius`
/// (`enemies.iter().map(|e| e.radius).fold(0.0, f32::max)`).
///
/// Legacy. Positions are inserted in `SpawnId` order, so the
/// `u32` indices stored in the buckets address `EnemyIndex.0` (legacy
/// `self.enemies` index) directly.
pub fn grid_build_system(
    level: Res<Level>,
    enemies: Query<(Entity, &SpawnId, &Enemy)>,
    mut cache: Local<Vec<GridEntry>>,
    mut grid: ResMut<SpatialGrid>,
    mut index: ResMut<EnemyIndex>,
    mut scratch: ResMut<GridScratch>,
) {
    // One query pass into the reused buffer, then one in-place sort; the grid,
    // `EnemyIndex` and `max_radius` all come from that buffer.
    cache.clear();
    cache.extend(
        enemies
            .iter()
            .map(|(entity, id, e)| (id.0, entity, e.pos, e.radius)),
    );
    cache.sort_unstable_by_key(|entry| entry.0);

    grid.reset(level.arena, GRID_CELL);
    grid.fill(cache.iter().map(|entry| entry.2));

    index.0.clear();
    index.0.extend(cache.iter().map(|entry| entry.1));
    scratch.max_radius = cache.iter().map(|entry| entry.3).fold(0.0, f32::max);
}

/// Legacy `World::separate_enemies` .
///
/// The widest possible overlapping pair spans two largest radii
/// (`reach = max_radius * 2.0`). Enemy `i` is read, its neighbours' current
/// positions are read (so earlier pushes are already visible) and the clamped
/// nudge is written back, exactly like the legacy in-place loop.
pub fn separation_system(
    grid: Res<SpatialGrid>,
    index: Res<EnemyIndex>,
    scratch: Res<GridScratch>,
    mut neigh: ResMut<NeighborScratch>,
    mut state: Local<Vec<(Vec2, f32)>>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
) {
    // Debug builds (i.e. every `cargo test`): prove the cached order is still
    // the live `SpawnId` order before indexing the grid with it.
    #[cfg(debug_assertions)]
    debug_assert_enemy_index(
        &index.0,
        enemies.iter().map(|(entity, id, _)| (id.0, entity)),
    );
    // Snapshot `(pos, radius)` once into the reused buffer, in `EnemyIndex`
    // (= legacy `self.enemies`) order: the legacy loop then only touches that
    // flat array, so the tick pays two entity lookups per enemy instead of one
    // per neighbour in the 5x5 bucket query.
    state.clear();
    for &entity in index.0.iter() {
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        state.push((e.pos, e.radius));
    }

    let reach = scratch.max_radius * 2.0;
    for i in 0..state.len() {
        let (pi, ri) = state[i];
        grid.neighbors_within(pi, reach, &mut neigh.0);
        let mut push = Vec2::ZERO;
        for &j in &neigh.0 {
            let j = j as usize;
            if j == i {
                continue;
            }
            let (pj, rj) = state[j];
            let sum = ri + rj;
            let d2 = pi.dist_sq(pj);
            if d2 < sum * sum {
                if d2 > 1e-6 {
                    let d = d2.sqrt();
                    push += (pi - pj) * ((sum - d) / d) * 0.5;
                } else {
                    // Perfectly stacked: nudge apart deterministically by
                    // index order so they never stay glued together.
                    let dir = if i < j { -1.0 } else { 1.0 };
                    push += Vec2::new(dir * sum * 0.5, 0.0);
                }
            }
        }
        // Legacy wrote `self.enemies[i].pos += push`, so a later neighbour read
        // must see this push: update the array in place, not a copy.
        state[i].0 += push.clamp_len(6.0);
    }

    // Write the final positions back in the same order (no sort, no clone).
    for (i, &entity) in index.0.iter().enumerate() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        e.pos = state[i].0;
    }
}

/// Legacy `World::resolve_enemy_walls` : push enemies out of
/// interior walls (final position constraint, after steering + separation) so
/// they path around obstacles instead of through.
///
/// Wall ejection is per-enemy independent, and the `EnemyIndex` built by
/// `grid_build_system` earlier in this same tick is still valid here (nothing
/// spawns or despawns between the two), so it iterates that instead of sorting
/// the query again.
pub fn wall_resolve_system(
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
) {
    #[cfg(debug_assertions)]
    debug_assert_enemy_index(
        &index.0,
        enemies.iter().map(|(entity, id, _)| (id.0, entity)),
    );
    let arena = level.arena;
    for &entity in index.0.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        let r = e.radius;
        let class = SizeClass::of_enemy(e.kind);
        // One tile-grid query per enemy: the cells it touches, never the
        // level's walls. `None` = no one-way-door permission, so a gate is
        // solid to the horde from both sides forever.
        e.pos = level.tiles.resolve_circle(e.pos, r, class, None);
        e.pos = e.pos.clamp_box(Vec2::splat(2.0), arena - Vec2::splat(2.0));
    }
}

/// Second grid build: after separation + wall ejection, before weapon fire, so
/// bullet queries use the enemies' final positions. Keeps `EnemyIndex` as is.
///
/// Legacy only reset + refill the grid: `max_radius` is *not*
/// recomputed there (no enemy changes radius between the two builds, and no
/// enemy is spawned or reaped in between), so `GridScratch::max_radius` keeps
/// the value from [`grid_build_system`].
///
/// `EnemyIndex` is still the authoritative `SpawnId` order here — nothing has
/// spawned or despawned since `grid_build_system` built it — so the grid is
/// refilled straight from it (only the positions are re-read, because
/// separation and wall ejection moved the enemies) instead of sorting the
/// query a second time.
pub fn grid_refresh_system(
    level: Res<Level>,
    enemies: Query<(Entity, &SpawnId, &Enemy)>,
    index: Res<EnemyIndex>,
    mut grid: ResMut<SpatialGrid>,
    mut _scratch: ResMut<GridScratch>,
) {
    #[cfg(debug_assertions)]
    debug_assert_enemy_index(
        &index.0,
        enemies.iter().map(|(entity, id, _)| (id.0, entity)),
    );
    grid.reset(level.arena, GRID_CELL);
    grid.fill(index.0.iter().filter_map(|&entity| enemies.get(entity).ok().map(|(_, _, e)| e.pos)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{circle_rect_overlap, TileKind, TileMap, TILE_F};
    use crate::game::level::{Rectf, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;

    /// Positions are f32 arithmetic; compare with a small tolerance.
    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn approx_v(a: Vec2, b: Vec2) -> bool {
        approx(a.x, b.x) && approx(a.y, b.y)
    }

    /// Narrow harness: one system, no schedule.
    fn run_one<O: 'static, M>(w: &mut World, system: impl IntoSystem<(), O, M> + 'static) -> O {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        let out = w.ecs.run_system_cached(system).unwrap();
        w.end_tick();
        out
    }

    /// Narrow harness for a whole sub-sequence of systems inside one tick.
    fn tick(w: &mut World, run: impl FnOnce(&mut World)) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        run(w);
        w.end_tick();
    }

    #[test]
    fn flow_refresh_follows_the_player() {
        let mut w = World::new(Theme::Sewer, 3);
        let center = w.level.arena * 0.5;
        // `World::new` builds the field toward the arena center, so the center
        // cell is the goal cell and has no direction.
        assert_eq!(w.flow.dir_at(SizeClass::Small, center).len_sq(), 0.0);

        // Move the player two cells to the right; the field must retarget.
        // The refresh is staggered across frames by `NavBudget`, so give it a
        // handful of ticks — the previous field stays usable in between.
        w.player.pos = Vec2::new(center.x + 200.0, center.y);
        for _ in 0..16 {
            run_one(&mut w, flow_refresh_system);
            if w.flow.dir_at(SizeClass::Small, center).len_sq() > 0.0 {
                break;
            }
        }

        assert!(
            w.flow.dir_at(SizeClass::Small, center).len_sq() > 0.0,
            "flow field did not follow the player"
        );
    }

    #[test]
    fn enemy_steering_chases_the_player_and_stays_in_the_arena() {
        let mut w = World::new(Theme::Sewer, 5);
        let center = w.level.arena * 0.5;
        // Start outside the arena: steering must still clamp it back inside.
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(-40.0, center.y), 1.0);

        run_one(&mut w, enemy_steering_system);

        let e = &w.enemies()[0];
        assert!(approx(e.pos.x, 2.0), "enemy not clamped to the arena: {:?}", e.pos);
        assert!(approx(e.pos.y, center.y), "enemy drifted off its row: {:?}", e.pos);
        assert!(e.vel.x > 0.0, "enemy did not steer toward the player: {:?}", e.vel);
    }

    #[test]
    fn spitter_fires_an_aimed_shot_and_resets_its_cooldown() {
        let mut w = World::new(Theme::Sewer, 7);
        let center = w.level.arena * 0.5;
        let from = Vec2::new(center.x + 100.0, center.y);
        let e = w.spawn_enemy(EnemyKind::Spitter, from, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().fire_cd = 0.0;
        let accent = w.level.palette.accent;

        run_one(&mut w, enemy_fire_system);

        // Neon-merge hook check: enemy shots are NOT player bullets, so they
        // keep `Bullet::new`'s `weapon: None` (only `World::fire_one` tags
        // player bullets) and enemy fire emits no `Sfx` of its own.
        assert!(w.bullets()[0].weapon.is_none(), "enemy bullet was weapon-tagged");
        assert!(w.sfx.is_empty(), "enemy fire emitted sfx: {:?}", w.sfx);

        assert_eq!(w.bullets().len(), 1, "spitter fired {:?} bullets", w.bullets().len());
        let b = &w.bullets()[0];
        assert!(matches!(b.faction, Faction::Enemy));
        assert!(approx_v(b.pos, from));
        assert!(approx_v(b.vel, (center - from).normalized() * 75.0), "{:?}", b.vel);
        assert!(approx(b.damage, EnemyKind::Spitter.damage() * 0.6));
        assert_eq!(b.life, 4.0);
        assert_eq!(b.size, 1);
        assert_eq!(b.color, accent);
        assert!(approx(w.enemies()[0].fire_cd, 2.2), "{:?}", w.enemies()[0].fire_cd);
    }

    #[test]
    fn boss_fires_a_ten_way_ring() {
        let mut w = World::new(Theme::Lab, 8);
        let center = w.level.arena * 0.5;
        let from = Vec2::new(center.x + 60.0, center.y);
        let e = w.spawn_enemy(EnemyKind::Boss, from, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().fire_cd = 0.0;

        run_one(&mut w, enemy_fire_system);

        assert_eq!(w.bullets().len(), 10, "boss ring should be 10 bullets");
        for (k, b) in w.bullets().iter().enumerate() {
            let a = k as f32 / 10.0 * std::f32::consts::TAU;
            assert!(approx_v(b.pos, from));
            assert!(approx_v(b.vel, Vec2::from_angle(a) * 60.0), "spoke {k}");
            assert_eq!(b.life, 5.0);
            assert_eq!(b.size, 2);
            assert_eq!(b.color, (255, 120, 160));
            assert!(approx(b.damage, EnemyKind::Boss.damage() * 0.6));
        }
        assert!(approx(w.enemies()[0].fire_cd, 1.2), "{:?}", w.enemies()[0].fire_cd);
    }

    #[test]
    fn ranged_enemies_respect_cooldown_range_and_spawn_id_order() {
        let mut w = World::new(Theme::Sewer, 9);
        let center = w.level.arena * 0.5;
        let near = w.spawn_enemy(EnemyKind::Spitter, Vec2::new(center.x + 100.0, center.y), 1.0);
        let also_near = w.spawn_enemy(EnemyKind::Spitter, Vec2::new(center.x - 100.0, center.y), 1.0);
        let far = w.spawn_enemy(EnemyKind::Spitter, Vec2::new(center.x + 300.0, center.y), 1.0);
        let melee = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(center.x, center.y + 40.0), 1.0);
        w.ecs.get_mut::<Enemy>(near).unwrap().fire_cd = 0.0;
        w.ecs.get_mut::<Enemy>(also_near).unwrap().fire_cd = 0.0;
        w.ecs.get_mut::<Enemy>(far).unwrap().fire_cd = 0.0;
        w.ecs.get_mut::<Enemy>(melee).unwrap().fire_cd = 1.5;

        run_one(&mut w, enemy_fire_system);

        // Bullets are spawned in `SpawnId` order: the nearer spitter first.
        assert_eq!(w.bullets().len(), 2, "out-of-range or melee enemy fired");
        assert!(approx_v(w.bullets()[0].pos, Vec2::new(center.x + 100.0, center.y)));
        assert!(approx_v(w.bullets()[1].pos, Vec2::new(center.x - 100.0, center.y)));
        // Out of range: the cooldown is still spent (legacy keeps decrementing)
        // and no bullet is spawned.
        assert!(w.enemies()[2].fire_cd < 0.0, "{:?}", w.enemies()[2].fire_cd);
        // Melee enemies have no fire cooldown at all.
        assert!(approx(w.enemies()[3].fire_cd, 1.5));
    }

    /// Bullets stop on walls, so a spitter must not fire through cover. The
    /// shot is held while the line is blocked *and the cooldown keeps ticking*,
    /// so it leaves on the first tick the line opens. No rng draw happens on
    /// the held path (this system has no rng at all), and the shot order of the
    /// shots that do leave is still `SpawnId` order.
    #[test]
    fn spitter_holds_fire_until_the_line_to_the_rat_is_clear() {
        use crate::game::ecs::systems::projectiles::los_clear;

        // The LOS radius must be the radius the bullet it gates actually has.
        let shot = Bullet::new(Vec2::ZERO, Vec2::ZERO, 0.0, Faction::Enemy, (0, 0, 0));
        assert!(approx(shot.radius, ENEMY_SHOT_RADIUS));

        let mut w = World::new(Theme::Sewer, 601);
        // An empty map: the only wall on the line is the one painted below.
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, Theme::Sewer, 0);
        let center = w.level.arena * 0.5;
        let from = Vec2::new(center.x + 100.0, center.y);
        let e = w.spawn_enemy(EnemyKind::Spitter, from, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().fire_cd = 0.0;

        // One solid cell on the line between spitter and player.
        let (cx, cy) = crate::game::collision::cell_of((from + center) * 0.5);
        w.level.tiles.set(cx, cy, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        assert!(
            !los_clear(&w.level, from, center, ENEMY_SHOT_RADIUS),
            "the case under test must start with cover on the line"
        );

        run_one(&mut w, enemy_fire_system);
        assert!(w.bullets().is_empty(), "the spitter fired through cover");
        assert!(
            w.enemies()[0].fire_cd < 0.0,
            "the held shot must keep the cooldown ticking: {:?}",
            w.enemies()[0].fire_cd
        );

        // The line opens: the very next tick releases the held shot.
        w.level.tiles.set(cx, cy, TileKind::Open);
        w.level.tiles.rebuild_masks();
        run_one(&mut w, enemy_fire_system);
        assert_eq!(
            w.bullets().len(),
            1,
            "the held shot did not leave when the line opened"
        );
        assert!(approx_v(w.bullets()[0].pos, from));
        assert!(approx(w.enemies()[0].fire_cd, 2.2), "{:?}", w.enemies()[0].fire_cd);
    }

    #[test]
    fn grid_build_orders_the_index_by_spawn_id_and_records_max_radius() {
        let mut w = World::new(Theme::Sewer, 11);
        w.clear_enemies();
        let cat = w.spawn_enemy(EnemyKind::Cat, Vec2::new(100.0, 100.0), 1.0);
        let skitterer = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(200.0, 200.0), 1.0);
        let boss = w.spawn_enemy(EnemyKind::Boss, Vec2::new(300.0, 300.0), 1.0);

        run_one(&mut w, grid_build_system);

        let index = w.ecs.resource::<EnemyIndex>();
        assert_eq!(index.0, vec![cat, skitterer, boss]);
        assert!(approx(
            w.ecs.resource::<GridScratch>().max_radius,
            EnemyKind::Boss.radius()
        ));
    }

    /// Legacy `exactly_stacked_enemies_are_nudged_apart`, plus the exact
    /// positions the in-place sequential algorithm produces (earlier pushes
    /// visible to later neighbour reads).
    #[test]
    fn exactly_stacked_enemies_are_nudged_apart() {
        let mut w = World::new(Theme::Sewer, 41);
        w.clear_enemies();
        let p = w.level.arena * 0.5;
        w.spawn_enemy(EnemyKind::Cat, p, 1.0);
        w.spawn_enemy(EnemyKind::Cat, p, 1.0);

        run_one(&mut w, grid_build_system);
        // `run_one` ends the tick; the grid/index survive (internal scratch).
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(separation_system).unwrap();
        w.end_tick();

        assert!(
            w.enemies()[0].pos.dist(w.enemies()[1].pos) > 0.1,
            "stacked enemies stayed glued together"
        );
        // i = 0 (stacked, i < j) is pushed -sum/2; i = 1 then sees the moved
        // enemy 0 and is pushed +2.0.
        assert!(approx_v(w.enemies()[0].pos, Vec2::new(p.x - 4.0, p.y)), "{:?}", w.enemies()[0].pos);
        assert!(approx_v(w.enemies()[1].pos, Vec2::new(p.x + 2.0, p.y)), "{:?}", w.enemies()[1].pos);
    }

    /// The separation broadphase reach is `2 * max_radius`, so a fat enemy in
    /// one grid cell must still see an overlapping neighbour two cells away.
    #[test]
    fn separation_reaches_across_grid_cells() {
        let mut w = World::new(Theme::Lab, 19);
        w.clear_enemies();
        // 15.9 (cell 0) and 32.5 (cell 2) are 16.6 px apart; a radius-14 boss
        // and a radius-4 cat overlap (sum 18) but span two cells.
        w.spawn_enemy(EnemyKind::Boss, Vec2::new(15.9, 100.0), 1.0);
        w.spawn_enemy(EnemyKind::Cat, Vec2::new(32.5, 100.0), 1.0);

        run_one(&mut w, grid_build_system);
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(separation_system).unwrap();
        w.end_tick();

        let d = w.enemies()[0].pos.dist(w.enemies()[1].pos);
        assert!(d > 17.0, "wide separation query missed the cross-cell neighbour: {d}");
    }

    /// Neon-merge hook check for this file: the other team tags only *player*
    /// bullets (`World::fire_one` sets `Bullet::weapon = Some(kind)`), so enemy
    /// shots must keep `Bullet::new`'s `None`, and enemy fire emits no `Sfx`.
    #[test]
    fn enemy_bullets_are_not_tagged_with_a_weapon() {
        let mut w = World::new(Theme::Sewer, 23);
        let center = w.level.arena * 0.5;
        let spitter = w.spawn_enemy(EnemyKind::Spitter, Vec2::new(center.x + 100.0, center.y), 1.0);
        let boss = w.spawn_enemy(EnemyKind::Boss, Vec2::new(center.x - 60.0, center.y), 1.0);
        w.ecs.get_mut::<Enemy>(spitter).unwrap().fire_cd = 0.0;
        w.ecs.get_mut::<Enemy>(boss).unwrap().fire_cd = 0.0;

        run_one(&mut w, enemy_fire_system);

        assert_eq!(w.bullets().len(), 11, "spitter shot + boss ring");
        for (i, b) in w.bullets().iter().enumerate() {
            assert!(
                b.weapon.is_none(),
                "enemy bullet {i} was tagged with {:?}",
                b.weapon
            );
        }
        assert!(w.sfx.is_empty(), "enemy fire emitted sfx: {:?}", w.sfx);
    }

    /// Does any wall collider overlap a circle of `r` at `p` for `class`?
    fn overlaps_a_wall(w: &World, p: Vec2, r: f32, class: SizeClass) -> bool {
        let mut hit = false;
        let m = r + TILE_F;
        w.level.tiles.for_each_collider_in(
            Rectf::new(p.x - m, p.y - m, 2.0 * m, 2.0 * m),
            class,
            None,
            |rect| {
                if circle_rect_overlap(p, r, &rect) {
                    hit = true;
                }
            },
        );
        hit
    }

    /// Legacy `enemies_are_pushed_out_of_walls`.
    ///
    /// `wall_resolve_system` walks `EnemyIndex` (no re-sort), exactly as in the
    /// schedule where `grid_build_system` ran earlier in the same tick.
    ///
    /// The map is painted here rather than sampled from the generator: the
    /// first interior `Solid` cell of a real map is deep inside a wall block,
    /// and a body buried that deep has nowhere to be ejected *to* (the tile
    /// grid's search window is three cells and says so). An isolated 2 × 2
    /// block with open floor all around is the case the system actually
    /// guarantees.
    #[test]
    fn enemies_are_pushed_out_of_walls() {
        let mut w = World::new(Theme::Sewer, 5);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, Theme::Sewer, 0);
        w.level
            .tiles
            .fill_cells(60, 60, 2, 2, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        let block = Rectf::new(60.0 * TILE_F, 60.0 * TILE_F, 2.0 * TILE_F, 2.0 * TILE_F);
        // Spawn an enemy buried in the middle of the block.
        w.spawn_enemy(EnemyKind::Cat, block.center(), 1.0);
        let class = SizeClass::of_enemy(EnemyKind::Cat);
        assert!(
            overlaps_a_wall(&w, block.center(), EnemyKind::Cat.radius(), class),
            "the case under test starts buried"
        );

        tick(&mut w, |w| {
            w.ecs.run_system_cached(grid_build_system).unwrap();
            w.ecs.run_system_cached(wall_resolve_system).unwrap();
        });

        let c = w.enemies()[0].pos;
        assert!(
            !overlaps_a_wall(&w, c, EnemyKind::Cat.radius(), class),
            "wall resolution left the body overlapping a wall: {c:?}"
        );
    }

    /// The reused grid buffer must be cleared, not appended to, on the next
    /// call (a stale entry would multiply `EnemyIndex`).
    #[test]
    fn grid_build_reuses_its_scratch_without_duplicating() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        let cat = w.spawn_enemy(EnemyKind::Cat, Vec2::new(100.0, 100.0), 1.0);
        let boss = w.spawn_enemy(EnemyKind::Boss, Vec2::new(300.0, 300.0), 1.0);

        tick(&mut w, |w| {
            for _ in 0..2 {
                w.ecs.run_system_cached(grid_build_system).unwrap();
            }
        });

        assert_eq!(w.ecs.resource::<EnemyIndex>().0, vec![cat, boss]);
        assert!(approx(
            w.ecs.resource::<GridScratch>().max_radius,
            EnemyKind::Boss.radius()
        ));
    }

    /// The buffered-shot list must be drained every tick and still spawn the
    /// boss's ring in spoke order.
    #[test]
    fn fire_scratch_does_not_leak_between_ticks() {
        let mut w = World::new(Theme::Lab, 17);
        let center = w.level.arena * 0.5;
        let from = Vec2::new(center.x + 60.0, center.y);
        let boss = w.spawn_enemy(EnemyKind::Boss, from, 1.0);

        w.ecs.get_mut::<Enemy>(boss).unwrap().fire_cd = 0.0;
        run_one(&mut w, enemy_fire_system);
        assert_eq!(w.bullets().len(), 10);

        w.ecs.get_mut::<Enemy>(boss).unwrap().fire_cd = 0.0;
        run_one(&mut w, enemy_fire_system);
        assert_eq!(w.bullets().len(), 20, "stale shots from the scratch buffer");
        // Spokes stay in `k` order after the stable `SpawnId` sort.
        for k in 0..10 {
            let a = k as f32 / 10.0 * std::f32::consts::TAU;
            assert!(approx_v(w.bullets()[10 + k].vel, Vec2::from_angle(a) * 60.0));
        }
    }

    /// Separation snapshots positions from the components each call (the
    /// reused array must not carry the previous tick's pushes into this one).
    #[test]
    fn separation_rebuilds_its_scratch_each_tick() {
        let mut w = World::new(Theme::Sewer, 43);
        w.clear_enemies();
        let p = w.level.arena * 0.5;
        w.spawn_enemy(EnemyKind::Cat, p, 1.0);
        w.spawn_enemy(EnemyKind::Cat, p, 1.0);
        let separate = |w: &mut World| {
            w.ecs.run_system_cached(grid_build_system).unwrap();
            w.ecs.run_system_cached(separation_system).unwrap();
        };

        tick(&mut w, separate);
        // Tick 1: 0 -> p - 4 (stacked, i < j), then 1 sees the moved 0.
        assert!(approx_v(w.enemies()[0].pos, Vec2::new(p.x - 4.0, p.y)));
        assert!(approx_v(w.enemies()[1].pos, Vec2::new(p.x + 2.0, p.y)));

        tick(&mut w, separate);
        // Tick 2 starts from those positions (6 and 7 px apart, sum of radii 8):
        // 0 is pushed a further -1.0, 1 a further +0.5.
        assert!(approx_v(w.enemies()[0].pos, Vec2::new(p.x - 5.0, p.y)), "{:?}", w.enemies()[0].pos);
        assert!(approx_v(w.enemies()[1].pos, Vec2::new(p.x + 2.5, p.y)), "{:?}", w.enemies()[1].pos);
    }
}
