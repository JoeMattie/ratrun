//! (f) Player bullets vs enemies, enemy bullets vs player, contact damage,
//! i-frames.
//!
//! Reproduce **verbatim**:
//! - `World::resolve_player_bullets`
//!   (swept segment, grid broadphase, pierce/hit cooldown, one `Sfx::Hit` per
//!   tick).
//! - Lines 744-776: `World::resolve_enemy_hits` (swept segment, damage through
//!   i-frames, consume a bullet only when the hit lands).
//! - Lines 777-822: `World::resolve_contact` (skip all when `iframes > 0`,
//!   max-damage aggregation, contact cooldown, knockback + immediate wall
//!   re-resolution).
//!
//! Resources/components:
//! - `player_bullet_hits_system`: `Res<SpatialGrid>`, `Res<GridScratch>`
//!   (`max_radius`), `Res<EnemyIndex>`, `ResMut<NeighborScratch>`,
//!   `Query<&mut Enemy>`, `Query<(Entity, &SpawnId, &mut Bullet)>`,
//!   `ResMut<RngRes>`, `ResMut<Particles>`, `ResMut<SfxQueue>`, `Commands`
//!   (despawn consumed bullets).
//! - `enemy_bullet_hits_system`: `ResMut<Player>`, the same bullet query,
//!   `ResMut<RngRes>`, `ResMut<Particles>`, `ResMut<SfxQueue>`,
//!   `ResMut<Shake>`, `Commands`.
//! - `contact_system`: `Res<Level>` (walls for knockback re-resolution),
//!   `ResMut<Player>`, `Query<(&SpawnId, &mut Enemy)>`, `ResMut<RngRes>`,
//!   `ResMut<Particles>`, `ResMut<SfxQueue>`, `ResMut<Shake>`.
//!
//! Ordering: after `pools_system`, in order player-bullets → enemy-bullets →
//! contact , then reaping (`loot.rs`).
//!
//! **Broadphase mapping:** `SpatialGrid` buckets store indices filled in
//! `SpawnId` order; `EnemyIndex.0[j]` is the entity for index `j`. Use
//! `enemies.get_mut(EnemyIndex.0[j])` to resolve. Reuse
//! `NeighborScratch.0` for `neighbors_within` (never allocate per tick).
//!
//! **Swept oracles, not endpoint tests:** use
//! `collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, ...)` for the
//! enemy query and the player. `b.lob.is_some()` bullets are skipped by the
//! player-bullet pass.
//!
//! **i-frames:** `Player::take_damage` returns false while i-framed; the enemy
//! bullet is only consumed on a landed hit. Contact skips the whole enemy loop
//! when `iframes > 0.0`, so no cooldown is spent and no knockback applied.
//! A dead enemy (`!e.alive()`) must not deal contact damage or knockback.
//!
//! **Stable order:** bullets iterated in `SpawnId` order (one reused-scratch
//! sort per system per tick); enemies for contact walked in the tick's
//! `SpawnId`-ordered `EnemyIndex` (legacy Vec order). The grid neighbour
//! `break` on first hit and the "one hit sfx per tick" latch must be
//! preserved.
//!
//! ## Deviations forced by the port
//!
//! - **Consumed bullets are not despawned here.** Legacy sets `b.life = 0.0`
//!   and lets the *next* tick's `update_bullets` retain (`life <= 0.0`) drop
//!   the bullet, so the facade `world.bullets` snapshot at the end of this
//!   tick still holds a dead bullet (the ported tests read `bullets[0].life`).
//!   `projectiles::bullets_system` performs the actual despawn one tick later,
//!   which is byte-for-byte the legacy lifetime; no system after combat reads
//!   bullets, so nothing observable changes. Neither bullet system therefore
//!   needs `Commands` (both dropped it in the perf pass: fewer system params
//!   and one less no-op `ApplyDeferred` in the chain).
//! - **Grid indices go through `EnemyIndex`.** Legacy indexed `self.enemies[j]`;
//!   `EnemyIndex.0[j]` is that same slot in `SpawnId` order (filled by
//!   `enemy_move::grid_build_system`), and a stale/despawned entity is skipped
//!   where legacy could not address one.
//! - **Sorts.** Every entity iteration below is `SpawnId`-sorted so bullet
//!   order, the `break` on the first grid hit and the contact-damage order
//!   match the legacy `Vec` order (contract determinism rule).
//!
//! ## Combat feedback hooks (the other team's `world.rs` changes, re-applied)
//!
//! Ported 1:1 from the merged reference:
//!
//! - `resolve_player_bullets`: on a landed hit,
//!   `let killed = !e.alive();` then `vfx::emit_weapon_hit(particles, rng,
//!   b.pos, b.vel, b.weapon, e.kind, killed)` replaces the old inline Spark
//!   burst, and `DamageNumbers::record(e.id, e.pos, e.radius, b.damage, killed,
//!   DamageSource::from_weapon(b.weapon))` pushes this enemy's floating damage
//!   number. Both run *before* the pierce / `life = 0.0` consume, exactly as in
//!   the legacy function (same rng call order).
//! - `resolve_enemy_hits` (neon 841-845): the player-hurt burst is replaced by
//!   `vfx::emit_player_hurt(particles, rng, p)`; `shake.max(4.0)` and
//!   `Sfx::Hurt` are unchanged.
//! - `resolve_contact` (neon 878-882): same `vfx::emit_player_hurt` swap with
//!   `shake.max(5.0)`; the i-frame early return, max-damage aggregation,
//!   `contact_cd` and knockback + wall re-resolution are untouched.
//!
//! `DamageNumbers` is a singleton resource round-tripped by the facade (owned
//! by worker (h), `World::damage_numbers`); the other two systems need no new
//! state.
//!
//! ## Per-tick cost (Phase 2 perf pass)
//!
//! - No per-tick heap allocation: both bullet systems sort into a reused
//!   [`Local`] scratch `Vec` (`order_bullets`) instead of bevy's
//!   `QueryIter::sort_by_key`, which allocates an entity list on every call.
//!   One sort per system per tick, never one per bullet.
//! - `contact_system` does not sort at all: `enemy_move::grid_refresh_system`
//!   rebuilds `EnemyIndex` from the final post-separation positions and nothing
//!   spawns or despawns an enemy between there and contact (weapons/tail whip,
//!   bullets and pools only damage), so it is already this tick's
//!   `SpawnId`-ordered enemy list — exactly the legacy `Vec` order.
//! - The two unused `Commands` parameters were dropped (neither system
//!   despawns; see above), which also removes a no-op `ApplyDeferred` from the
//!   chained schedule at this point.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::{self, SpatialGrid};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{EnemyIndex, GridScratch, NeighborScratch, RngRes, Shake, SfxQueue};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

/// Collect every bullet entity with its `SpawnId` into `order` (a per-system
/// scratch buffer reused across ticks) and sort it, so iteration matches the
/// legacy `Vec<Bullet>` order without allocating once the buffer is warm.
///
/// `SpawnId`s are unique, so the unstable sort is exact (stable vs unstable is
/// indistinguishable); pdqsort is O(n) on the already-ordered common case.
fn order_bullets(
    bullets: &Query<(Entity, &SpawnId, &mut Bullet)>,
    order: &mut Vec<(u64, Entity)>,
) {
    order.clear();
    order.extend(bullets.iter().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|&(id, _)| id);
}

/// Legacy `World::resolve_player_bullets`  plus the neon
/// combat-feedback hooks .
pub fn player_bullet_hits_system(
    grid: Res<SpatialGrid>,
    grid_scratch: Res<GridScratch>,
    index: Res<EnemyIndex>,
    mut neigh: ResMut<NeighborScratch>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bullets: Query<(Entity, &SpawnId, &mut Bullet)>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let max_enemy_radius = grid_scratch.max_radius;
    // Debug-only guard: the grid buckets index `EnemyIndex`, so it must still
    // be the live `SpawnId` order here (see `enemy_move` module docs).
    #[cfg(debug_assertions)]
    debug_assert_enemy_index(
        &index.0,
        enemies.iter().map(|(entity, id, _)| (id.0, entity)),
    );
    // Legacy takes `self.neigh` out so the borrows stay disjoint; here the
    // scratch is its own resource, so the vec can be borrowed directly.
    let neigh = &mut neigh.0;
    let mut hit_any = false;

    // Legacy `for b in self.bullets.iter_mut()`: bullets live in spawn order.
    order_bullets(&bullets, &mut order);
    for &(_id, entity) in order.iter() {
        let Ok((_, _, mut b)) = bullets.get_mut(entity) else {
            continue;
        };
        if b.faction != Faction::Player || b.hit_cd > 0.0 || b.lob.is_some() {
            continue;
        }
        // Cover the whole swept path (not just the endpoint) plus the largest
        // enemy radius, so fast bullets cannot skip a target or miss an
        // overlap across a cell boundary.
        let mid = (b.prev_pos + b.pos) * 0.5;
        let half_sweep = (b.pos - b.prev_pos).len() * 0.5;
        grid.neighbors_within(mid, b.radius + max_enemy_radius + half_sweep, neigh);
        for &j in neigh.iter() {
            let Some(entity) = index.0.get(j as usize).copied() else {
                continue;
            };
            let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            if collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, e.pos, e.radius) {
                e.hurt(b.damage);
                let killed = !e.alive();
                hit_any = true;
                // Weapon-specific impact sparks (`b.weapon` was tagged by
                // `weapons::fire_one`) + this enemy's floating damage number.
                vfx::emit_weapon_hit(
                    &mut particles,
                    &mut rng.0,
                    b.pos,
                    b.vel,
                    b.weapon,
                    e.kind,
                    killed,
                );
                damage_numbers.record(
                    e.id,
                    e.pos,
                    e.radius,
                    b.damage,
                    killed,
                    DamageSource::from_weapon(b.weapon),
                );
                if b.pierce > 0 {
                    b.pierce -= 1;
                    b.hit_cd = 0.12;
                } else {
                    b.life = 0.0;
                }
                break;
            }
        }
    }
    if hit_any {
        sfx.0.push(Sfx::Hit);
    }
}

/// Legacy `World::resolve_enemy_hits`  plus `vfx::emit_player_hurt`
/// .
pub fn enemy_bullet_hits_system(
    mut player: ResMut<Player>,
    mut bullets: Query<(Entity, &SpawnId, &mut Bullet)>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let ppos = player.pos;
    let mut hit = false;
    order_bullets(&bullets, &mut order);
    for &(_id, entity) in order.iter() {
        let Ok((_, _, mut b)) = bullets.get_mut(entity) else {
            continue;
        };
        if b.faction != Faction::Enemy {
            continue;
        }
        if collision::segment_circle_hit(b.prev_pos, b.pos, b.radius, ppos, PLAYER_RADIUS)
            && player.take_damage(b.damage)
        {
            hit = true;
            // Consume the bullet only when the hit landed; an i-framed
            // player must not delete projectiles for free.
            b.life = 0.0;
        }
    }
    if hit {
        let p = player.pos;
        vfx::emit_player_hurt(&mut particles, &mut rng.0, p);
        shake.amount = shake.amount.max(4.0);
        sfx.0.push(Sfx::Hurt);
    }
}

/// Contact damage for one enemy (legacy `resolve_contact` loop body).
///
/// Pulled out of [`contact_system`] so the two iteration sources (the tick's
/// `EnemyIndex` and the narrow-harness fallback) share exactly one body.
fn contact_one(
    entity: Entity,
    ppos: Vec2,
    level: &Level,
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    hit_dmg: &mut f32,
) {
    let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
        return;
    };
    // An enemy killed earlier this tick (by a bullet or a pool) must
    // not still land contact damage or knockback.
    if !e.alive() {
        return;
    }
    if e.contact_cd <= 0.0 && collision::circles_overlap(ppos, PLAYER_RADIUS, e.pos, e.radius) {
        *hit_dmg = hit_dmg.max(e.damage);
        e.contact_cd = 0.6;
        // Knockback. (Split into two statements: `Mut<Enemy>` cannot
        // read `e.pos` while the same place is mutably borrowed.)
        let knockback = (e.pos - ppos).normalized() * 4.0;
        e.pos += knockback;
        // Wall resolution already ran this tick, so re-resolve the
        // knocked-back enemy to avoid a frame inside a prop.
        for w in &level.walls {
            e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
        }
    }
}

/// Legacy `World::resolve_contact`  plus `vfx::emit_player_hurt`
/// .
pub fn contact_system(
    level: Res<Level>,
    mut player: ResMut<Player>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let ppos = player.pos;
    // `take_damage` returns false while i-frames are up. Skip the loop
    // entirely in that case so enemies do not spend their contact cooldown
    // (or get knocked back) for a hit that cannot land.
    if player.iframes > 0.0 {
        return;
    }
    let mut hit_dmg = 0.0f32;
    // `EnemyIndex` was rebuilt by `enemy_move::grid_refresh_system` from the
    // post-separation, post-wall positions and nothing has spawned or
    // despawned an enemy since (weapons, bullets and pools only damage), so it
    // is this tick's authoritative `SpawnId`-ordered enemy list: the legacy
    // `Vec` order with no sort and no allocation. Narrow single-system tests
    // run this system without the grid build, so when the index does not cover
    // every live enemy we fall back to one allocation-free sort into the
    // per-system scratch buffer; in the real schedule the lengths always match
    // and the sort never runs.
    if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        for &entity in index.0.iter() {
            contact_one(entity, ppos, &level, &mut enemies, &mut hit_dmg);
        }
    } else {
        order.clear();
        order.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
        order.sort_unstable_by_key(|&(id, _)| id);
        for &(_, entity) in order.iter() {
            contact_one(entity, ppos, &level, &mut enemies, &mut hit_dmg);
        }
    }
    if hit_dmg > 0.0 && player.take_damage(hit_dmg) {
        let p = player.pos;
        vfx::emit_player_hurt(&mut particles, &mut rng.0, p);
        shake.amount = shake.amount.max(5.0);
        sfx.0.push(Sfx::Hurt);
    }
}

#[cfg(test)]
mod tests {
    //! Narrow single-system tests (PORT_PLAN §2): `begin_tick` → one system via
    //! `run_system_cached` → `end_tick`. They cover the legacy tests that
    //! exercise only combat .

    use super::*;
    use crate::game::collision::SpatialGrid;
    use crate::game::ecs::constants::GRID_CELL;
    use crate::game::ecs::resources::{EnemyIndex, GridScratch};
    use crate::game::ecs::systems::combat;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Rectf, Theme};
    use crate::game::particle::Ramp;
    use crate::game::world::World;
    use crate::math::Vec2;

    const DT: f32 = 1.0 / 60.0;

    /// Fill `SpatialGrid` + `EnemyIndex` + `GridScratch` the way
    /// `enemy_move::grid_build_system` does (in `SpawnId` order), without
    /// depending on that (separately owned) system. Call outside a tick.
    fn rebuild_broadphase(w: &mut World) {
        let arena = w.level.arena;
        let mut query = w.ecs.query::<(Entity, &SpawnId, &Enemy)>();
        let mut items: Vec<(u64, Entity, Vec2, f32)> = query
            .iter(&w.ecs)
            .map(|(entity, id, e)| (id.0, entity, e.pos, e.radius))
            .collect();
        items.sort_by_key(|item| item.0);

        {
            let mut grid = w.ecs.resource_mut::<SpatialGrid>();
            grid.reset(arena, GRID_CELL);
            grid.fill(items.iter().map(|item| item.2));
        }

        let mut index = w.ecs.resource_mut::<EnemyIndex>();
        index.0 = items.iter().map(|item| item.1).collect();
        let max_radius = items.iter().map(|item| item.3).fold(0.0f32, f32::max);
        w.ecs.resource_mut::<GridScratch>().max_radius = max_radius;
    }

    fn run<M>(w: &mut World, system: impl IntoSystem<(), (), M> + 'static) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(system).unwrap();
        w.end_tick();
    }

    // Legacy `iframed_player_keeps_enemy_bullets` .
    #[test]
    fn iframed_player_keeps_enemy_bullets() {
        let mut w = World::new(Theme::Sewer, 67);
        w.clear_bullets();
        w.player.pos = w.level.arena * 0.5;
        let pos = w.player.pos;
        w.spawn_bullet(Bullet::new(pos, Vec2::ZERO, 5.0, Faction::Enemy, (255, 0, 0)));
        w.player.iframes = 0.5;
        let hp = w.player.hp;

        run(&mut w, combat::enemy_bullet_hits_system);
        assert_eq!(w.player.hp, hp, "i-frames should block the damage");
        assert!(w.bullets()[0].life > 0.0, "i-frames must not delete the bullet");

        w.player.iframes = 0.0;
        run(&mut w, combat::enemy_bullet_hits_system);
        assert!(w.player.hp < hp, "a vulnerable player should take the hit");
        assert_eq!(
            w.bullets()[0].life, 0.0,
            "a landed hit should consume the bullet"
        );
    }

    // Legacy `iframed_player_does_not_spend_contact_cooldowns` .
    #[test]
    fn iframed_player_does_not_spend_contact_cooldowns() {
        let mut w = World::new(Theme::Sewer, 71);
        w.level.walls.clear();
        w.player.pos = w.level.arena * 0.5;
        w.clear_enemies();
        let p = w.player.pos;
        let entity = w.spawn_enemy(EnemyKind::Cat, p, 1.0);
        w.ecs.get_mut::<Enemy>(entity).unwrap().contact_cd = 0.0;
        w.player.iframes = 0.5;
        let hp = w.player.hp;
        let pos = w.enemies()[0].pos;
        // `contact_system` reads the tick's `EnemyIndex` instead of sorting.
        rebuild_broadphase(&mut w);

        run(&mut w, combat::contact_system);
        assert_eq!(w.player.hp, hp);
        assert_eq!(w.enemies()[0].contact_cd, 0.0, "i-frames spent the cooldown");
        assert_eq!(w.enemies()[0].pos, pos, "i-frames applied knockback");

        w.player.iframes = 0.0;
        run(&mut w, combat::contact_system);
        assert!(w.player.hp < hp, "vulnerable contact should damage");
        assert!(
            w.enemies()[0].contact_cd > 0.0,
            "a landed hit should set cooldown"
        );
    }

    // Legacy `dead_enemies_deal_no_contact_damage` .
    #[test]
    fn dead_enemies_deal_no_contact_damage() {
        let mut w = World::new(Theme::Sewer, 12);
        w.clear_enemies();
        let p = w.player.pos;
        let entity = w.spawn_enemy(EnemyKind::Cat, p, 1.0);
        {
            let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
            e.hp = -1.0;
            e.contact_cd = 0.0;
        }
        let hp = w.player.hp;
        rebuild_broadphase(&mut w);

        run(&mut w, combat::contact_system);
        assert_eq!(w.player.hp, hp, "a dead enemy still dealt contact damage");
        assert_eq!(w.enemies()[0].contact_cd, 0.0);
    }

    // Legacy `contact_knockback_stays_out_of_walls` .
    #[test]
    fn contact_knockback_stays_out_of_walls() {
        let mut w = World::new(Theme::Sewer, 43);
        w.level.walls = vec![Rectf::new(100.0, 90.0, 10.0, 20.0)];
        w.player.pos = Vec2::new(94.0, 100.0);
        w.clear_enemies();
        let entity = w.spawn_enemy(EnemyKind::Cat, Vec2::new(96.5, 100.0), 1.0);
        w.ecs.get_mut::<Enemy>(entity).unwrap().contact_cd = 0.0;
        rebuild_broadphase(&mut w);

        run(&mut w, combat::contact_system);
        let wall = w.level.walls[0];
        assert!(
            !wall.contains(w.enemies()[0].pos),
            "knockback left the enemy inside a wall at {:?}",
            w.enemies()[0].pos
        );
    }

    // Legacy `fast_enemy_bullet_cannot_skip_the_player` ,
    // with the movement pass applied directly so this test does not depend on
    // `projectiles::bullets_system`.
    #[test]
    fn fast_enemy_bullet_cannot_skip_the_player() {
        let mut w = World::new(Theme::Sewer, 91);
        w.clear_bullets();
        w.player.pos = Vec2::new(100.0, 100.0);
        w.player.iframes = 0.0;
        let hp = w.player.hp;
        let mut b = Bullet::new(
            Vec2::new(88.0, 100.0),
            Vec2::new(400.0, 0.0),
            5.0,
            Faction::Enemy,
            (255, 0, 0),
        );
        b.radius = 2.0;
        let entity = w.spawn_bullet(b);
        // The tick's movement: 400 px/s * 0.05 s = 20 px, ending 8 px past the
        // player, outside the 5 px combined hit radius.
        w.ecs.get_mut::<Bullet>(entity).unwrap().pos = Vec2::new(108.0, 100.0);
        assert!(!collision::circles_overlap(
            w.bullets()[0].pos,
            2.0,
            w.player.pos,
            PLAYER_RADIUS
        ));

        run(&mut w, combat::enemy_bullet_hits_system);
        assert!(
            w.player.hp < hp,
            "fast enemy bullet passed through the player"
        );
        assert_eq!(w.bullets()[0].life, 0.0);
    }

    // Legacy `fast_bullet_through_a_skitterer_registers_a_hit` ,
    // with the movement + grid passes applied directly so this test does not
    // depend on `projectiles::bullets_system` / `grid_build_system`.
    #[test]
    fn fast_bullet_through_a_skitterer_registers_a_hit() {
        let mut w = World::new(Theme::Sewer, 89);
        w.clear_enemies();
        w.clear_bullets();
        let entity = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(86.0, 100.0), 1.0);
        let hp = w.enemies()[0].hp;
        assert!(hp > 0.0);
        let mut b = Bullet::new(
            Vec2::new(80.0, 100.0),
            Vec2::new(240.0, 0.0),
            3.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 2.0;
        let bullet = w.spawn_bullet(b);
        // 240 px/s * 0.05 s = 12 px, i.e. 6 px past the enemy center.
        w.ecs.get_mut::<Bullet>(bullet).unwrap().pos = Vec2::new(92.0, 100.0);
        assert!(!collision::circles_overlap(
            w.bullets()[0].pos,
            2.0,
            w.enemies()[0].pos,
            w.enemies()[0].radius
        ));
        rebuild_broadphase(&mut w);
        assert_eq!(w.ecs.resource::<EnemyIndex>().0.len(), 1);
        assert_eq!(w.ecs.resource::<EnemyIndex>().0[0], entity);

        run(&mut w, combat::player_bullet_hits_system);
        assert!(
            w.enemies()[0].hp < hp,
            "fast bullet passed through the enemy"
        );
        assert_eq!(w.bullets()[0].life, 0.0);
        assert_eq!(w.sfx.len(), 1, "one hit sfx per tick");
        assert!(matches!(w.sfx[0], Sfx::Hit));
    }

    /// A piercing bullet survives a hit with the legacy re-hit cooldown, and a
    /// player bullet with an active `hit_cd` is skipped entirely.
    #[test]
    fn pierce_spends_one_hit_and_sets_the_rehit_cooldown() {
        let mut w = World::new(Theme::Sewer, 93);
        w.clear_enemies();
        w.clear_bullets();
        let entity = w.spawn_enemy(EnemyKind::Cat, Vec2::new(86.0, 100.0), 1.0);
        let hp = w.enemies()[0].hp;
        let mut b = Bullet::new(
            Vec2::new(80.0, 100.0),
            Vec2::ZERO,
            3.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 2.0;
        b.pos = Vec2::new(92.0, 100.0);
        b.pierce = 2;
        w.spawn_bullet(b);
        rebuild_broadphase(&mut w);
        assert!(w.enemies()[0].hp == hp);

        run(&mut w, combat::player_bullet_hits_system);
        assert!(w.enemies()[0].hp < hp);
        assert_eq!(w.bullets()[0].pierce, 1);
        assert!((w.bullets()[0].hit_cd - 0.12).abs() < 1e-6);
        assert!(w.bullets()[0].life > 0.0, "a piercing bullet is not consumed");

        // Second call with the bullet still cooling down: no new hit, no sfx.
        // (The facade queue is only drained by the App, so clear it first.)
        let hp = w.enemies()[0].hp;
        w.sfx.clear();
        run(&mut w, combat::player_bullet_hits_system);
        assert_eq!(w.enemies()[0].hp, hp, "hit_cd should block the re-hit");
        assert!(w.sfx.is_empty(), "no hit sfx when nothing was hit");
        let _ = entity;
    }

    /// Contact damage is the max over overlapping enemies, and every touching
    /// enemy spends its cooldown (legacy `hit_dmg = hit_dmg.max(e.damage)`).
    #[test]
    fn contact_uses_max_damage_and_cools_every_touching_enemy() {
        let mut w = World::new(Theme::Sewer, 95);
        w.level.walls.clear();
        w.clear_enemies();
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Skitterer, p, 1.0);
        w.spawn_enemy(EnemyKind::Brute, p, 1.0);
        let armor = w.player.loadout.stats.armor;
        let hp = w.player.hp;
        let brute_damage = w.enemies()[1].damage;
        assert!(brute_damage > w.enemies()[0].damage);
        rebuild_broadphase(&mut w);

        run(&mut w, combat::contact_system);
        for e in &w.enemies() {
            assert_eq!(e.contact_cd, 0.6, "every touching enemy spends its cooldown");
        }
        let expected = (brute_damage - armor).max(1.0);
        assert!(
            (hp - w.player.hp - expected).abs() < 1e-4,
            "contact should apply the largest damage, hp {} -> {}",
            hp,
            w.player.hp
        );
    }

    // ---- Combat-feedback hooks (the other team's `world.rs` changes) ------

    /// A player bullet that lands pushes one floating damage number carrying
    /// the enemy's runtime id, the bullet's damage and a weapon-derived colour
    /// source, and the impact spark is the weapon-specific one (neon
    /// `resolve_player_bullets`): a SqueakNova hit must emit Nova sparks, not
    /// the Gnaw Spark cone.
    #[test]
    fn player_bullet_hit_pushes_a_damage_number_and_weapon_sparks() {
        let mut w = World::new(Theme::Sewer, 101);
        w.clear_enemies();
        w.clear_bullets();
        let entity = w.spawn_enemy(EnemyKind::Cat, Vec2::new(86.0, 100.0), 1.0);
        let hp = w.enemies()[0].hp;
        {
            let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
            e.id = 77;
        }
        let mut b = Bullet::new(
            Vec2::new(80.0, 100.0),
            Vec2::new(240.0, 0.0),
            3.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 2.0;
        b.pos = Vec2::new(92.0, 100.0);
        b.weapon = Some(crate::game::weapon::WeaponKind::SqueakNova);
        w.spawn_bullet(b);
        rebuild_broadphase(&mut w);

        run(&mut w, combat::player_bullet_hits_system);

        assert!(w.enemies()[0].hp < hp, "the bullet must land");
        assert_eq!(
            w.damage_numbers.len(),
            1,
            "a landed player bullet must push exactly one damage number"
        );
        let dn = w.damage_numbers.items[0];
        assert_eq!(dn.enemy_id, 77, "the number carries the enemy id");
        assert!(
            (dn.total_dmg - 3.0).abs() < 1e-6,
            "the number carries the bullet damage, got {}",
            dn.total_dmg
        );
        assert!(!dn.is_kill, "the cat survived, so this is not a kill");
        assert_eq!(
            dn.source,
            DamageSource::Normal,
            "SqueakNova maps to the normal damage source"
        );
        assert!(
            w.particles.items.iter().any(|p| matches!(p.ramp, Ramp::Nova)),
            "the impact spark must be the SqueakNova one"
        );
        assert!(
            !w.particles.items.iter().any(|p| matches!(p.ramp, Ramp::Spark)),
            "a SqueakNova hit must not fall back to the Gnaw spark cone"
        );
    }

    /// The `killed` argument is computed after `e.hurt` and passed to both the
    /// impact spark and the damage number, so a lethal hit is flagged as a kill
    /// and reports `DamageSource::from_weapon` for an Acid Spit bullet.
    #[test]
    fn killed_hit_marks_the_damage_number_as_a_kill_with_its_weapon_source() {
        let mut w = World::new(Theme::Sewer, 102);
        w.clear_enemies();
        w.clear_bullets();
        let entity = w.spawn_enemy(EnemyKind::Cat, Vec2::new(86.0, 100.0), 1.0);
        {
            let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
            e.id = 9;
            e.hp = 1.0;
        }
        let mut b = Bullet::new(
            Vec2::new(80.0, 100.0),
            Vec2::new(240.0, 0.0),
            3.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.radius = 2.0;
        b.pos = Vec2::new(92.0, 100.0);
        b.weapon = Some(crate::game::weapon::WeaponKind::AcidSpit);
        w.spawn_bullet(b);
        rebuild_broadphase(&mut w);

        run(&mut w, combat::player_bullet_hits_system);

        assert!(!w.enemies()[0].alive(), "the hit must be lethal");
        assert_eq!(w.damage_numbers.len(), 1);
        let dn = w.damage_numbers.items[0];
        assert_eq!(dn.enemy_id, 9);
        assert!(dn.is_kill, "a lethal hit must flag the number as a kill");
        assert_eq!(dn.source, DamageSource::Acid);
    }

    /// Being hit by an enemy bullet emits the neon hurt VFX (a red shockwave
    /// ring, which the old inline burst never pushed) plus `Sfx::Hurt` and the
    /// shake floor.
    #[test]
    fn enemy_bullet_hurt_emits_the_hurt_shockwave() {
        let mut w = World::new(Theme::Sewer, 103);
        w.clear_bullets();
        w.player.pos = w.level.arena * 0.5;
        w.player.iframes = 0.0;
        let pos = w.player.pos;
        w.spawn_bullet(Bullet::new(pos, Vec2::ZERO, 5.0, Faction::Enemy, (255, 0, 0)));
        let hp = w.player.hp;

        run(&mut w, combat::enemy_bullet_hits_system);

        assert!(w.player.hp < hp);
        assert_eq!(
            w.particles.shockwaves.len(),
            1,
            "the player-hurt VFX must emit its shockwave ring"
        );
        assert_eq!(w.sfx.len(), 1);
        assert!(matches!(w.sfx[0], Sfx::Hurt));
        assert!(w.shake >= 4.0, "bullet damage must raise the shake floor");
    }

    /// Contact damage takes the same hurt-VFX path (with the 5.0 shake floor).
    #[test]
    fn contact_hurt_emits_the_hurt_shockwave() {
        let mut w = World::new(Theme::Sewer, 104);
        w.level.walls.clear();
        w.clear_enemies();
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p, 1.0);
        let hp = w.player.hp;
        rebuild_broadphase(&mut w);

        run(&mut w, combat::contact_system);

        assert!(w.player.hp < hp);
        assert_eq!(
            w.particles.shockwaves.len(),
            1,
            "contact damage must emit the player-hurt shockwave"
        );
        assert_eq!(w.sfx.len(), 1);
        assert!(matches!(w.sfx[0], Sfx::Hurt));
        assert!(w.shake >= 5.0, "contact must raise the shake floor");
    }

    /// Bullet order must stay `SpawnId` order even when the archetype's row
    /// order no longer matches it, because the legacy loop's order decides
    /// which enemy bullet lands through the i-frame window.
    #[test]
    fn bullet_order_follows_spawn_id_not_table_order() {
        let mut w = World::new(Theme::Sewer, 105);
        w.clear_bullets();
        w.player.pos = Vec2::new(100.0, 100.0);
        w.player.iframes = 0.0;
        let p = w.player.pos;
        // SpawnId 0 sits far away and can never hit; 1..=3 all overlap.
        let far = w.spawn_bullet(Bullet::new(
            Vec2::new(10.0, 10.0),
            Vec2::ZERO,
            5.0,
            Faction::Enemy,
            (255, 0, 0),
        ));
        for _ in 0..3 {
            w.spawn_bullet(Bullet::new(p, Vec2::ZERO, 5.0, Faction::Enemy, (255, 0, 0)));
        }
        // Despawning the first bullet moves the last row into its slot, so the
        // query's archetype order (D, B, C) differs from spawn order (B, C, D).
        w.despawn_entity(far);
        let hp = w.player.hp;

        run(&mut w, combat::enemy_bullet_hits_system);

        assert!(w.player.hp < hp, "one bullet must land");
        let dead: Vec<bool> = w.bullets().iter().map(|b| b.life <= 0.0).collect();
        assert_eq!(
            dead,
            vec![true, false, false],
            "i-frames must consume only the lowest-SpawnId bullet"
        );
    }
}

