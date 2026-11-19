//! (e) Bullet movement with swept positions, orbit bullets, Acid Spit lobs and
//! pools.
//!
//! Reproduce **verbatim** (the legacy bodies, then the neon combat-feedback
//! merge):
//! - `World::update_bullets` (`prev_pos`, orbit angle, lob
//!   landing on wall/target/timeout, pool spawn, life/edge culling) plus the
//!   merge hooks: `vfx::emit_orbit_trail`, `vfx::emit_acid_splash` (replacing
//!   the old inline landing burst) and the in-flight `vfx::emit_acid_drip`.
//! - `World::update_pools` (life decay, enemy damage, flash,
//!   despawn) plus `vfx::emit_pool_bubbles`, the `!e.alive()` guard and
//!   `DamageNumbers::record(..., DamageSource::Acid)` per damaged enemy.
//!
//! Resources: `Res<TickInput>` (dt), `Res<Level>` (arena, walls),
//! `Res<Player>` (orbit centre), `ResMut<RngRes>`, `ResMut<Particles>`,
//! `Commands` + `ResMut<NextSpawnId>` (pools), `Res<EnemyIndex>` (stable enemy
//! walk in `pools_system`) and `ResMut<DamageNumbers>`.
//!
//! Components: bullets `(Entity, &SpawnId, &mut Bullet)`, pools
//! `(Entity, &SpawnId, &mut Pool)`, enemies `&mut Enemy`.
//!
//! Ordering: `bullets_system` after `fire_weapons_system`, then `pools_system`
//! , before the collision systems. `.chain()` applies
//! the deferred pool spawns before `pools_system`, so a pool that lands this
//! tick is decayed and deals damage on this tick — exactly like legacy, where
//! `update_bullets` pushed onto `self.pools` and `update_pools` iterated the
//! whole Vec immediately afterwards.
//!
//! **Pools are decayed, damage enemies and are despawned in one pass** (this is
//! why the contract's "pool damage" bullet lives here rather than in
//! `combat.rs`: splitting decay/despawn from damage would lose a tick of damage
//! for a pool that expires this tick). `pools_system` sets
//! `enemy.flash = enemy.flash.max(0.05)` and subtracts `p.dps * dt` from hp
//! directly (no `hurt()`).
//!
//! **Stable order, no per-tick allocation.** Legacy walks bullets/pools in Vec
//! order, which is spawn order, and those walks draw rng (`emit_orbit_trail`,
//! `emit_acid_drip`, `emit_acid_splash`, `emit_pool_bubbles`) — so the order is
//! part of the spec. Instead of Bevy's allocating `QueryIter::sort_by_key`, each
//! system fills a reused `Local` scratch buffer of `(SpawnId, Entity)` once per
//! tick, sorts it in place (ids are unique, so the unstable sort is
//! deterministic) and looks each entity up mutably. Enemy order comes from
//! `EnemyIndex`, which `enemy_move::grid_build_system` fills in `SpawnId` order
//! earlier in the tick (`grid_refresh_system` only re-reads it to rebuild the
//! grid). No enemy is spawned or despawned between that build and this system,
//! so the index is exact and costs no sort at all.
//!
//! **Orbit bullets** sit outside the arena edge on purpose; only free-flying
//! projectiles are edge-culled. Lobbed bullets (`lob.is_some()`) do no
//! in-flight damage (enforced in `combat.rs`).
//!
//! **Bullets at `life <= 0` are always despawned** (legacy `retain`); the
//! collision systems rely on a spent bullet being gone.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::game::bullet::Bullet;
use bevy_ecs::entity::Entity;

use crate::game::collision;
use crate::game::ecs::components::{Pool, SpawnId};
use crate::game::ecs::resources::{EnemyIndex, NextSpawnId, RngRes, TickInput};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

/// Reused `(SpawnId, Entity)` ordering scratch: filled, sorted in place and
/// consumed once per tick so the hot path allocates nothing after warm-up.
type OrderScratch = Vec<(u64, Entity)>;

pub fn bullets_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    player: Res<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut next_id: ResMut<NextSpawnId>,
    mut order: Local<OrderScratch>,
    mut bullets: Query<(Entity, &SpawnId, &mut Bullet)>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let arena = level.arena;

    // `SpawnId` order == legacy `Vec<Bullet>` order (spawn order), and the
    // per-bullet rng draws below depend on it.
    order.clear();
    order.extend(bullets.iter().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|(id, _)| *id);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut b)) = bullets.get_mut(entity) else {
            continue;
        };

        b.prev_pos = b.pos;
        if let Some(o) = b.orbit.as_mut() {
            o.angle += o.speed * dt;
            // Copy the orbit fields out so the `Mut<Bullet>` borrow of `o`
            // ends before `b.pos` is written (legacy borrowed disjoint
            // fields of a plain `&mut Bullet`).
            let angle = o.angle;
            let radius = o.radius;
            b.pos = ppos + Vec2::from_angle(angle) * radius;
            vfx::emit_orbit_trail(&mut particles, &mut rng.0, b.pos, dt);
        } else {
            // Copy `vel` out first: `Mut<Bullet>` has to be re-borrowed for
            // the `pos` write and the `vel` read would otherwise alias.
            let vel = b.vel;
            b.pos += vel * dt;
        }
        b.life -= dt;
        b.hit_cd = (b.hit_cd - dt).max(0.0);

        // Lobbed shots land on the first wall along this tick's sweep,
        // when they reach the target, or when they time out.
        if let Some(lob) = b.lob {
            let step = b.pos - b.prev_pos;
            let step_len = step.len();
            let mut land: Option<Vec2> = None;
            let mut best_t = f32::INFINITY;
            for w in &level.walls {
                if let Some(t) = collision::segment_rect_hit(b.prev_pos, b.pos, b.radius, w) {
                    if t < best_t {
                        best_t = t;
                        land = Some(b.prev_pos + step * t);
                    }
                }
            }
            if land.is_none() && step_len >= (lob.target - b.prev_pos).len() - 0.01 {
                land = Some(lob.target);
            }
            if land.is_none() && b.life <= 0.0 {
                land = Some(b.pos);
            }
            if let Some(p) = land {
                commands.spawn((
                    Pool {
                        pos: p,
                        radius: lob.radius,
                        dps: lob.dps,
                        life: lob.life,
                        color: lob.color,
                    },
                    next_id.take(),
                ));
                vfx::emit_acid_splash(&mut particles, &mut rng.0, p, lob.radius);
                b.life = 0.0;
            } else {
                vfx::emit_acid_drip(&mut particles, &mut rng.0, b.pos, dt);
            }
        }

        // Legacy `retain`: drop expired bullets, and cull free-flying ones
        // that left the arena.
        let keep = if b.life <= 0.0 {
            false
        } else if b.orbit.is_some() {
            // Orbit bullets deliberately sit outside the arena edge when the
            // player hugs a wall; only cull free-flying projectiles.
            true
        } else {
            b.pos.x > -12.0
                && b.pos.y > -12.0
                && b.pos.x < arena.x + 12.0
                && b.pos.y < arena.y + 12.0
        };
        if !keep {
            commands.entity(entity).despawn();
        }
    }
}

pub fn pools_system(
    input: Res<TickInput>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut order: Local<OrderScratch>,
    mut pools: Query<(Entity, &SpawnId, &mut Pool)>,
    mut commands: Commands,
) {
    let dt = input.dt;
    // Debug-only guard for the cached enemy order (see `enemy_move` module
    // docs): the pool damage walk below reuses it instead of sorting.
    #[cfg(debug_assertions)]
    debug_assert_enemy_index(
        &index.0,
        enemies.iter().map(|(entity, id, _)| (id.0, entity)),
    );

    // `SpawnId` order == legacy `Vec<Pool>` order; `emit_pool_bubbles` draws
    // rng per pool, so the walk order is part of the behaviour.
    order.clear();
    order.extend(pools.iter().map(|(entity, id, _)| (id.0, entity)));
    order.sort_unstable_by_key(|(id, _)| *id);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut p)) = pools.get_mut(entity) else {
            continue;
        };

        // Same pass as legacy: decay, bubbles, damage, then drop expired
        // pools. A pool that expires this tick still deals this tick's damage.
        p.life -= dt;
        vfx::emit_pool_bubbles(&mut particles, &mut rng.0, p.pos, p.radius, p.life, dt);
        let tick_dmg = p.dps * dt;
        // Legacy walks `self.enemies` (Vec order). `EnemyIndex` is that exact
        // order: `enemy_move::grid_build_system` filled it in `SpawnId` order
        // earlier in this tick and no enemy has been spawned or despawned since
        // (`spawn::director_system` runs before it; reaping runs later).
        for &enemy in &index.0 {
            let Ok((_, _, mut e)) = enemies.get_mut(enemy) else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            if e.pos.dist(p.pos) <= p.radius + e.radius {
                e.hp -= tick_dmg;
                e.flash = e.flash.max(0.05);
                damage_numbers.record(
                    e.id,
                    e.pos,
                    e.radius,
                    tick_dmg,
                    !e.alive(),
                    DamageSource::Acid,
                );
            }
        }
        if p.life <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::bullet::{Faction, Lob, Orbit};
    use crate::game::ecs::systems::enemy_move;
    use crate::game::level::{Rectf, Theme};
    use crate::game::world::World;

    /// Legacy `acid_spit_pool_lands_at_its_target_after_travel` setup: the
    /// bullet weapons.rs builds for a level-1 Acid Spit aimed at `target`.
    fn acid_bullet(ppos: Vec2, target: Vec2, arena: Vec2) -> Bullet {
        let target = target.clamp_box(Vec2::splat(4.0), arena - Vec2::splat(4.0));
        let to = target - ppos;
        let speed = 110.0;
        let mut b = Bullet::new(
            ppos,
            to.normalized() * speed,
            10.0,
            Faction::Player,
            (140, 220, 70),
        );
        b.size = 2;
        b.radius = 3.0;
        b.life = to.len() / speed + 0.5;
        b.lob = Some(Lob {
            target,
            radius: 12.0 + 1.5,
            dps: 10.0 * 4.0,
            life: 2.6,
            color: (140, 220, 70),
        });
        b
    }

    /// One `bullets_system` tick through the narrow harness.
    fn tick_bullets(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(bullets_system).unwrap();
        w.end_tick();
    }

    /// One `pools_system` tick. `EnemyIndex` is the `SpawnId`-ordered enemy
    /// list that `enemy_move::grid_build_system` produces earlier in the same
    /// tick, so the test runs that first (it only reads enemies and writes the
    /// grid/index/scratch).
    fn tick_pools(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(enemy_move::grid_build_system).unwrap();
        w.ecs.run_system_cached(pools_system).unwrap();
        w.end_tick();
    }

    #[test]
    fn orbit_bullets_survive_the_edge_cull() {
        let mut w = World::new(Theme::Sewer, 31);
        w.player.pos = Vec2::new(50.0, 50.0);
        w.clear_bullets();
        let mut orbit = Bullet::new(
            w.player.pos,
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        orbit.life = 100.0;
        orbit.orbit = Some(Orbit {
            angle: 0.0,
            radius: w.level.arena.x + 20.0,
            speed: 0.0,
        });
        w.spawn_bullet(orbit);
        let free = Bullet::new(
            Vec2::new(w.level.arena.x + 50.0, 50.0),
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        w.spawn_bullet(free);

        tick_bullets(&mut w, 1.0 / 60.0);

        assert_eq!(w.bullets().len(), 1, "free bullet should be culled");
        assert!(
            w.bullets()[0].orbit.is_some(),
            "orbit bullet was culled at the edge"
        );
    }

    /// Merge hook: `update_bullets` emits a spore shimmer behind orbit bullets
    /// (`vfx::emit_orbit_trail`), which the old code never did.
    #[test]
    fn orbit_bullets_emit_a_trail() {
        let mut w = World::new(Theme::Sewer, 32);
        w.clear_bullets();
        let mut orbit = Bullet::new(
            w.player.pos,
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        orbit.life = 100.0;
        orbit.orbit = Some(Orbit {
            angle: 0.0,
            radius: 20.0,
            speed: 1.0,
        });
        w.spawn_bullet(orbit);

        let before = w.particles.len();
        let mut emitted = false;
        for _ in 0..300 {
            tick_bullets(&mut w, 1.0 / 60.0);
            if w.particles.len() > before {
                emitted = true;
                break;
            }
        }
        assert!(emitted, "orbit trail never emitted a particle");
    }

    #[test]
    fn acid_spit_pool_lands_at_its_target_after_travel() {
        let mut w = World::new(Theme::Sewer, 79);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.level.walls.clear();
        let ppos = w.player.pos;
        let face = Vec2::new(1.0, 0.0);
        let target = ppos + face * 50.0;
        w.spawn_bullet(acid_bullet(ppos, target, w.level.arena));

        let dt = 1.0 / 60.0;
        let mut ticks = 0;
        while w.pools().is_empty() && ticks < 120 {
            tick_bullets(&mut w, dt);
            ticks += 1;
        }
        assert_eq!(w.pools().len(), 1, "acid pool never landed");
        assert!(
            w.pools()[0].pos.dist(target) < 1.0,
            "pool landed at {:?}, target {target:?}",
            w.pools()[0].pos
        );
        assert!(w.bullets().is_empty(), "landed projectile should be removed");
    }

    #[test]
    fn acid_spit_pool_lands_on_a_wall_not_behind_it() {
        let mut w = World::new(Theme::Sewer, 83);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        let ppos = w.player.pos;
        let face = Vec2::new(1.0, 0.0);
        let wall = Rectf::new(ppos.x + 20.0, ppos.y - 20.0, 4.0, 40.0);
        w.level.walls = vec![wall];
        let target = ppos + face * 50.0;
        w.spawn_bullet(acid_bullet(ppos, target, w.level.arena));

        let dt = 1.0 / 60.0;
        let mut ticks = 0;
        while w.pools().is_empty() && ticks < 120 {
            tick_bullets(&mut w, dt);
            ticks += 1;
        }
        assert_eq!(w.pools().len(), 1, "acid pool never landed");
        assert!(
            w.pools()[0].pos.x <= wall.x + 0.01,
            "pool landed past the wall at {:?}",
            w.pools()[0].pos
        );
        assert!(w.pools()[0].pos.x > ppos.x + 10.0, "pool landed short");
    }

    /// Merge hook: a landing lob calls `vfx::emit_acid_splash`, which pushes a
    /// shockwave ring and a floor stain. Distinct from `emit_acid_drip`, which
    /// is the in-flight path.
    #[test]
    fn acid_splash_emits_a_ring_and_a_floor_stain() {
        let mut w = World::new(Theme::Sewer, 84);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.level.walls.clear();
        let ppos = w.player.pos;
        let target = ppos + Vec2::new(6.0, 0.0);
        w.spawn_bullet(acid_bullet(ppos, target, w.level.arena));

        let stains0 = w.particles.stains.len();
        let waves0 = w.particles.shockwaves.len();
        let mut ticks = 0;
        while w.pools().is_empty() && ticks < 120 {
            tick_bullets(&mut w, 1.0 / 60.0);
            ticks += 1;
        }

        assert_eq!(w.pools().len(), 1, "lob should have landed this tick");
        assert!(
            w.particles.shockwaves.len() > waves0,
            "acid splash emitted no shockwave"
        );
        assert!(
            w.particles.stains.len() > stains0,
            "acid splash left no floor stain"
        );
    }

    /// Merge hook: a lob still in flight calls `vfx::emit_acid_drip`.
    #[test]
    fn acid_spit_drips_while_in_flight() {
        let mut w = World::new(Theme::Sewer, 85);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.level.walls.clear();
        let ppos = w.player.pos;
        // Far target: the lob stays airborne for a while.
        let target = ppos + Vec2::new(120.0, 0.0);
        w.spawn_bullet(acid_bullet(ppos, target, w.level.arena));

        let before = w.particles.len();
        let mut emitted = false;
        for _ in 0..120 {
            tick_bullets(&mut w, 1.0 / 60.0);
            if w.pools().is_empty() && w.particles.len() > before {
                emitted = true;
                break;
            }
        }
        assert!(emitted, "lobbed acid emitted no in-flight drip");
    }

    /// The chained pair must make a pool that lands this tick deal this
    /// tick's damage (legacy pushed onto `self.pools` and `update_pools` ran
    /// immediately afterwards). Bevy only does that if `ApplyDeferred` runs
    /// between `bullets_system` and `pools_system`, which is what the real
    /// schedule relies on.
    #[test]
    fn landed_pool_damages_on_the_landing_tick() {
        use crate::game::enemy::EnemyKind;
        use rand::SeedableRng;

        let mut w = World::new(Theme::Sewer, 47);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        w.level.walls.clear();
        let ppos = w.player.pos;
        let mut rng = rand::rngs::StdRng::seed_from_u64(9);
        w.spawn_enemy_with_rng(
            EnemyKind::Skitterer,
            ppos + Vec2::new(1.0, 0.0),
            100.0,
            &mut rng,
        );
        let hp0 = w.enemies()[0].hp;

        let mut b = Bullet::new(
            ppos,
            Vec2::new(110.0, 0.0),
            10.0,
            Faction::Player,
            (140, 220, 70),
        );
        b.radius = 3.0;
        b.lob = Some(Lob {
            target: ppos + Vec2::new(1.0, 0.0),
            radius: 20.0,
            dps: 1000.0,
            life: 2.6,
            color: (140, 220, 70),
        });
        w.spawn_bullet(b);

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                enemy_move::grid_build_system,
                bullets_system,
                pools_system,
            )
                .chain(),
        );
        let dt = 1.0 / 60.0;
        w.begin_tick(dt, Vec2::ZERO, false);
        schedule.run(&mut w.ecs);
        w.end_tick();

        assert_eq!(w.bullets().len(), 0, "lobbed bullet should have landed");
        assert_eq!(w.pools().len(), 1, "landing should spawn a pool");
        assert!(
            w.enemies()[0].hp <= hp0 - 1000.0 * dt * 0.5,
            "pool that landed this tick dealt no damage until the next tick"
        );
    }

    /// Legacy `update_pools`: a pool damages every overlapping enemy for
    /// `dps * dt`, sets the flash floor, decays and is removed at life <= 0 —
    /// all in the same pass.
    #[test]
    fn pool_damages_then_expires_in_the_same_pass() {
        use crate::game::enemy::EnemyKind;
        use rand::SeedableRng;

        let mut w = World::new(Theme::Sewer, 41);
        w.clear_enemies();
        w.clear_pools();
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, Vec2::new(200.0, 200.0), 1.0, &mut rng);
        let hp0 = w.enemies()[0].hp;
        w.spawn_pool(Pool {
            pos: Vec2::new(200.0, 200.0),
            radius: 20.0,
            dps: 8.0,
            life: 0.01,
            color: (140, 220, 70),
        });

        let dt = 1.0 / 60.0;
        tick_pools(&mut w, dt);

        assert_eq!(w.pools().len(), 0, "expired pool should be despawned");
        assert!(
            w.enemies()[0].hp <= hp0 - 8.0 * dt + 1e-4,
            "expiring pool dealt no damage on its last tick"
        );
        assert!(w.enemies()[0].flash >= 0.05);
    }

    /// Merge hook: pool ticks record one aggregated `DamageSource::Acid`
    /// popup per enemy (the other team's per-enemy merging), and the record
    /// carries the tick damage and its kill flag.
    #[test]
    fn pool_damage_records_acid_damage_numbers() {
        use crate::game::enemy::EnemyKind;
        use rand::SeedableRng;

        let mut w = World::new(Theme::Sewer, 43);
        w.clear_enemies();
        w.clear_pools();
        let mut rng = rand::rngs::StdRng::seed_from_u64(6);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, Vec2::new(200.0, 200.0), 50.0, &mut rng);
        let id = w.enemies()[0].id;
        assert_ne!(id, 0, "enemies need a stable id for damage aggregation");
        w.spawn_pool(Pool {
            pos: Vec2::new(200.0, 200.0),
            radius: 20.0,
            dps: 8.0,
            life: 2.0,
            color: (140, 220, 70),
        });

        let dt = 1.0 / 60.0;
        for _ in 0..5 {
            tick_pools(&mut w, dt);
        }

        // `DamageNumbers` round-trips through the facade, and its merge window
        // is keyed on `since_hit` (only advanced by `DamageNumbers::update`,
        // which the narrow harness does not run), so five pool ticks on the
        // same enemy id aggregate into one popup.
        assert_eq!(
            w.damage_numbers.len(),
            1,
            "pool ticks should merge into one popup"
        );
        let entry = &w.damage_numbers.items[0];
        assert_eq!(entry.enemy_id, id);
        assert_eq!(entry.source, DamageSource::Acid);
        assert!(
            (entry.total_dmg - 5.0 * 8.0 * dt).abs() < 0.05,
            "aggregated pool damage was {}, expected {}",
            entry.total_dmg,
            5.0 * 8.0 * dt
        );
        assert!(!entry.is_kill);
    }

    /// Merge hooks: `emit_pool_bubbles` runs per pool per tick, and dead
    /// enemies are skipped entirely (no damage, no damage number).
    #[test]
    fn dead_enemies_take_no_pool_damage_or_numbers() {
        use crate::game::enemy::EnemyKind;
        use rand::SeedableRng;

        let mut w = World::new(Theme::Sewer, 44);
        w.clear_enemies();
        w.clear_pools();
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, Vec2::new(200.0, 200.0), 1.0, &mut rng);
        // Kill it before the pool pass (legacy leaves dead enemies in the Vec
        // until reaping later in the tick).
        {
            let mut q = w.ecs.query::<&mut Enemy>();
            for mut e in q.iter_mut(&mut w.ecs) {
                e.hp = -1.0;
            }
        }
        w.spawn_pool(Pool {
            pos: Vec2::new(200.0, 200.0),
            radius: 20.0,
            dps: 8.0,
            life: 2.0,
            color: (140, 220, 70),
        });

        let before = w.particles.len();
        let mut bubbled = false;
        for _ in 0..240 {
            tick_pools(&mut w, 1.0 / 60.0);
            if w.particles.len() > before {
                bubbled = true;
            }
            assert_eq!(
                w.damage_numbers.len(),
                0,
                "dead enemies should record no damage numbers"
            );
            if w.pools().is_empty() {
                break;
            }
        }
        assert!(bubbled, "pool bubbles never emitted a particle");
    }
}
