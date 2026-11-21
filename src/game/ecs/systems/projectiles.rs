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
//! Resources: `Res<TickInput>` (dt), `Res<Level>` (arena, tile grid),
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
//!
//! ## Shots respect walls
//!
//! Every free-flying player **and** enemy bullet is swept against the tile grid
//! each tick ([`wall_hit`]): it stops at the first wall along
//! `prev_pos -> pos`, emits that weapon's impact burst ([`emit_wall_impact`])
//! and is consumed. Because the sweep *truncates* `pos` rather than nudging
//! it, the shot's whole visible path stays on the near side of the wall, and
//! the hit systems that run after this one (`combat.rs`) resolve an enemy hit
//! that happened earlier along the same sweep — a hit before the wall counts,
//! a hit after it does not exist.
//!
//! - **Consumption is legacy-shaped.** A wall-stopped bullet gets `life = 0.0`
//!   (the legacy "spent" marker) but survives the remainder of *this* tick so
//!   `combat` can still resolve its hit; the next tick's pass despawns it,
//!   exactly like a bullet consumed by an enemy hit.
//! - **Openings are openings.** Rat-holes, doorways and maze gates are art
//!   openings in the grid: only their piers/jambs collide, so a 2 px shot
//!   slips through an 8 px rat-hole and a 14 px doorway. A **closed** one-way
//!   gate is a wall — `TileMap::segment_hit` deliberately ignores the leaf
//!   (the player swings it) so [`wall_hit`] adds the art spec's rule
//!   (`open_progress() < 0.5` blocks); see [`GATE_BLOCKS_BELOW_OPEN`].
//! - **Orbit bullets are exempt** (see `weapons.rs`): the ring is 24 px from
//!   the rat and a two-cell corridor is only 16 px half-width, so
//!   destroy-on-contact would empty the weapon in exactly the corridors it has
//!   to stay useful in. They already ignore the arena edge for the same reason.
//! - **Lobs** keep their own landing rule (target / wall / timeout) and use the
//!   same [`wall_hit`], so acid also lands on a closed gate.
//!
//! ## Line of sight for other systems
//!
//! [`los_clear`] is the supported question: *"is there a clear line from A to B
//! for a body or a shot of radius `r`?"* — used by `weapons.rs` targeting and
//! the Spore Orbit damage test, and available to the boss systems (a boss or a
//! boss attack may not lock or reach through a solid). It is a tile query over
//! the cells the segment crosses, so it is cheap but not free: callers that
//! sweep many candidates must cap them (`weapons.rs` caps at
//! `TARGET_LOS_CANDIDATES`).

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::game::bullet::Bullet;
use bevy_ecs::entity::Entity;

use crate::game::ecs::components::{Pool, SpawnId};
use crate::game::ecs::resources::{EnemyIndex, NextSpawnId, RngRes, TickInput};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::ecs::resources::DamageLedger;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

/// Reused `(SpawnId, Entity)` ordering scratch: filled, sorted in place and
/// consumed once per tick so the hot path allocates nothing after warm-up.
type OrderScratch = Vec<(u64, Entity)>;

// ---- Line of sight / shot sweep (shared with `weapons.rs`, `combat.rs`) ----

/// A one-way gate leaf stops blocking (for bodies *and* shots) once it is at
/// least half swung open — the rule in `art_walls` next to
/// `one_way_gate_rect`.
pub const GATE_BLOCKS_BELOW_OPEN: f32 = 0.5;

/// Earliest wall contact along `from -> to` for a circle of `radius`, as a
/// `0..=1` parameter along the segment.
///
/// The tile grid (`TileMap::segment_hit`) owns the geometry: solid cells, prop
/// footprints and maze partitions block, while rat-holes and doorways only
/// collide on their piers/jambs, so their openings stay shootable. On top of
/// that this adds the one thing the tile sweep leaves out on purpose: a
/// one-way gate leaf blocks everything while it is shut
/// ([`GATE_BLOCKS_BELOW_OPEN`]) instead of opening for whoever pushes through.
///
/// Cost: one cell `AABB` query over the segment, plus `0..=2` gate rects (the
/// level's door list is tiny, and no cell is looked up per door).
pub fn wall_hit(level: &Level, from: Vec2, to: Vec2, radius: f32) -> Option<f32> {
    let mut best = level.tiles.segment_hit(from, to, radius);
    for door in &level.objects.one_way_doors {
        if door.open_progress() >= GATE_BLOCKS_BELOW_OPEN {
            continue;
        }
        let Some(dir) = level.tiles.one_way_dir(door.cell.0, door.cell.1) else {
            continue;
        };
        let gate = crate::game::art_walls::one_way_gate_rect(door.cell.0, door.cell.1, dir);
        if let Some(t) = crate::game::collision::segment_rect_hit(from, to, radius, &gate) {
            best = Some(best.map_or(t, |b: f32| b.min(t)));
        }
    }
    best
}

/// Is the straight line `from -> to` clear for a body or a shot of `radius`?
///
/// The supported line-of-sight question for gameplay code that is not a
/// projectile: auto-aim's "can the rat actually see this threat" test, Spore
/// Orbit's "is this enemy behind cover" test, and the boss systems (a pounce
/// landing, a scan lock, a sweep target). `radius` is the mover's own radius, so
/// a 14 px boss is refused a passage a 2 px shot slips through.
pub fn los_clear(level: &Level, from: Vec2, to: Vec2, radius: f32) -> bool {
    wall_hit(level, from, to, radius).is_none()
}

/// Impact feedback for a shot that stopped on a wall.
///
/// Reuses `vfx::emit_weapon_hit`, which styles the burst by weapon and throws it
/// back along `-vel`; its `enemy_kind` argument is unused by every weapon arm
/// (see its signature), so a wall impact gets the weapon's own spark cone
/// without a new art hook. No `Sfx`: the hit cue belongs to flesh, and a
/// corridor would otherwise machine-gun the sound every tick.
fn emit_wall_impact(
    particles: &mut Particles,
    rng: &mut rand::rngs::StdRng,
    pos: Vec2,
    vel: Vec2,
    weapon: Option<crate::game::weapon::WeaponKind>,
) {
    vfx::emit_weapon_hit(
        particles,
        rng,
        pos,
        vel,
        weapon,
        crate::game::enemy::EnemyKind::Skitterer,
        false,
    );
}

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
        // Already spent (consumed by a hit or by a wall last tick): it still
        // takes its last step this tick, but it must not spray a second
        // impact burst on the way out.
        let spent = b.life <= 0.0;
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

        // Shots respect walls: stop a free-flying bullet at the first wall
        // along this tick's sweep and consume it. Clamping `pos` (instead of
        // only killing the bullet) is what keeps the rest of the tick honest:
        // the hit systems run after this one and resolve hits on the truncated
        // segment, so a hit *before* the wall counts and a hit behind it cannot
        // happen. Orbit bullets are exempt (they must survive a two-cell
        // corridor) and lobs have their own landing rule below.
        let mut hit_wall = false;
        if b.orbit.is_none() && b.lob.is_none() {
            let step = b.pos - b.prev_pos;
            if let Some(t) = wall_hit(&level, b.prev_pos, b.pos, b.radius) {
                b.pos = b.prev_pos + step * t;
                hit_wall = true;
                if !spent {
                    let vel = b.vel;
                    emit_wall_impact(&mut particles, &mut rng.0, b.pos, vel, b.weapon);
                }
            }
        }

        // Lobbed shots land on the first wall along this tick's sweep,
        // when they reach the target, or when they time out.
        if let Some(lob) = b.lob {
            let step = b.pos - b.prev_pos;
            let step_len = step.len();
            // One swept tile query for the whole step: the cells the segment
            // crosses, earliest hit `t`, so a fast lob cannot skip the near
            // face of a 16 px wall.
            let mut land: Option<Vec2> = wall_hit(&level, b.prev_pos, b.pos, b.radius)
                .map(|t| b.prev_pos + step * t);
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
        // that left the arena. A bullet stopped by a wall this tick is spent
        // (`life = 0.0`, the legacy marker) but lives until the next pass so
        // `combat` can still resolve the part of the sweep that did connect.
        // A bullet that was *already* spent dies now, even if the wall clamp
        // still matches it sitting on the wall face (otherwise it would
        // re-arm the "survive one more tick" rule forever).
        let wall_stopped_now = hit_wall && !spent;
        if wall_stopped_now {
            b.life = 0.0;
        }
        let keep = if wall_stopped_now {
            true
        } else if b.life <= 0.0 {
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
    mut ledger: ResMut<DamageLedger>,
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
                let hp_before = e.hp.max(0.0);
                // Directional armour (the Pillbug's shell): the acid is
                // arriving from the pool's own centre. The softer flash below
                // is the pool's own, so this books the hit's side effects
                // without `hurt`'s tint.
                let mult = e.damage_mult_from(p.pos);
                e.hp -= tick_dmg * mult;
                e.note_damage();
                // Balance instrumentation only: pools are exclusively Acid
                // Spit's landed lobs, so the weapon is known here.
                ledger.add(
                    crate::game::weapon::WeaponKind::AcidSpit,
                    (hp_before - e.hp.max(0.0)).max(0.0),
                );
                e.flash = e.flash.max(0.05);
                damage_numbers.record(
                    e.id,
                    e.pos,
                    e.radius,
                    tick_dmg * mult,
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
    use crate::game::art_traps::OneWayDoorState;
    use crate::game::art_walls::{WallAxis, WallKind};
    use crate::game::bullet::{Faction, Lob, Orbit};
    use crate::game::art_walls::Cardinal;
    use crate::game::collision::{self, TileKind, TileMap};
    use crate::game::ecs::systems::{combat, enemy_move};
    use crate::game::level::{OneWayDoor, Theme, MAP_COLS, MAP_ROWS};
    use crate::game::world::World;

    /// Strip the generated layout's walls so the test measures the bullet pass
    /// instead of the map (what `Level::walls.clear()` used to do).
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

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
        open_map(&mut w);
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
        open_map(&mut w);
        let ppos = w.player.pos;
        let face = Vec2::new(1.0, 0.0);
        // A one-cell (`Solid`) wall column 20..36 px east of the rat.
        let (cx, cy) = collision::cell_of(ppos);
        w.level
            .tiles
            .fill_cells(cx + 1, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        let wall = TileMap::tile_rect(cx + 1, cy);
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
        open_map(&mut w);
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
        open_map(&mut w);
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
        open_map(&mut w);
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

    /// Per-tick cost of the bullet movement pass with 200 free-flying
    /// bullets, plus the tile sweep this change adds, measured in the same
    /// process (a loaded machine hits both numbers). The movement pass runs on
    /// an open map so the population survives all 200 bullets; the sweep is
    /// measured as 200 one-tick segments through a *generated* map, which is
    /// the same work each bullet now does per tick.
    #[test]
    fn bullets_cost_per_tick_with_200_bullets() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        open_map(&mut w);
        let arena = w.level.arena;
        let mut probes: Vec<(Vec2, Vec2, f32)> = Vec::with_capacity(200);
        for i in 0..200 {
            let a = i as f32 * 0.31;
            let r = 40.0 + (i % 40) as f32 * 6.0;
            let pos = (arena * 0.5 + Vec2::from_angle(a) * r).clamp_box(Vec2::splat(8.0), arena);
            let vel = Vec2::from_angle(a + 1.0) * 60.0;
            let mut b = Bullet::new(pos, vel, 1.0, Faction::Player, (255, 255, 255));
            b.life = 1e6; // never expires during the measurement
            let radius = b.radius;
            w.spawn_bullet(b);
            probes.push((pos, pos + vel * (1.0 / 60.0), radius));
        }

        let ticks = 120u32;
        let t0 = std::time::Instant::now();
        for _ in 0..ticks {
            tick_bullets(&mut w, 1.0 / 60.0);
        }
        let per_tick = t0.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);
        assert_eq!(w.bullets().len(), 200, "benchmark bullets died");

        // The sweep itself, through the walls of a real generated map.
        let walled = World::new(Theme::Sewer, 14);
        let t1 = std::time::Instant::now();
        for _ in 0..ticks {
            for (from, to, radius) in &probes {
                let _ = std::hint::black_box(wall_hit(&walled.level, *from, *to, *radius));
            }
        }
        let sweep = t1.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);

        println!(
            "bullets_system @200 bullets: movement {per_tick:.1} us/tick, \
             wall sweep for the same 200 {sweep:.1} us/tick"
        );
        assert!(per_tick + sweep < 2000.0, "out of budget");
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

    // ---- Shots respect walls ------------------------------------------------

    /// A 3-cell solid column whose near face is the line `x = 16 * cx`.
    fn wall_column(w: &mut World, cx: i32, cy: i32) -> f32 {
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// One bullet travelling east along the cell-10 centre line.
    fn eastbound(
        w: &mut World,
        x: f32,
        faction: Faction,
        speed: f32,
        pierce: i32,
    ) -> bevy_ecs::entity::Entity {
        let mut b = Bullet::new(
            Vec2::new(x, 168.0),
            Vec2::new(speed, 0.0),
            1.0,
            faction,
            (255, 255, 255),
        );
        b.life = 10.0;
        b.pierce = pierce;
        w.spawn_bullet(b)
    }

    /// The sweep truncates the shot at the wall: it never appears past the near
    /// face, and it is consumed (legacy `life = 0.0` marker, despawned by the
    /// next pass).
    #[test]
    fn player_bullet_stops_at_the_first_wall() {
        let mut w = World::new(Theme::Sewer, 601);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        let face = wall_column(&mut w, 12, 10);
        eastbound(&mut w, 150.0, Faction::Player, 200.0, 0);

        let mut furthest = f32::MIN;
        for _ in 0..40 {
            tick_bullets(&mut w, 1.0 / 60.0);
            if let Some(b) = w.bullets().first() {
                furthest = furthest.max(b.pos.x);
            }
        }

        assert!(
            w.bullets().is_empty(),
            "a wall-stopped bullet should be despawned"
        );
        assert!(
            furthest <= face - 2.0 + 1e-3,
            "bullet reached x={furthest}, past the wall face at {face}"
        );
    }

    /// Enemy shots respect walls the same way (the same sweep, no faction
    /// special case).
    #[test]
    fn enemy_bullet_stops_at_the_first_wall() {
        let mut w = World::new(Theme::Sewer, 602);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        let face = wall_column(&mut w, 12, 10);
        eastbound(&mut w, 150.0, Faction::Enemy, 200.0, 0);

        let mut furthest = f32::MIN;
        for _ in 0..40 {
            tick_bullets(&mut w, 1.0 / 60.0);
            if let Some(b) = w.bullets().first() {
                furthest = furthest.max(b.pos.x);
            }
        }

        assert!(w.bullets().is_empty(), "enemy bullet should be consumed");
        assert!(
            furthest <= face - 2.0 + 1e-3,
            "enemy bullet got through the wall to x={furthest}"
        );
    }

    /// A rat-hole is an opening, not a wall: only its two piers collide, so a
    /// 2 px shot slips through the 8 px gap.
    #[test]
    fn shot_passes_through_a_rat_hole() {
        let mut w = World::new(Theme::Sewer, 603);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        w.level.tiles.set(
            12,
            10,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        w.level.tiles.rebuild_masks();
        eastbound(&mut w, 150.0, Faction::Player, 200.0, 0);

        for _ in 0..20 {
            tick_bullets(&mut w, 1.0 / 60.0);
        }

        let b = w
            .bullets()
            .into_iter()
            .next()
            .expect("shot was stopped by a rat-hole");
        assert!(
            b.pos.x > 208.0,
            "bullet only reached x={} — the rat-hole gap did not open",
            b.pos.x
        );
    }

    /// A shut one-way gate is a wall for shots; the same gate at half swing is
    /// an opening (the art spec's `open_progress() < 0.5` rule).
    #[test]
    fn closed_one_way_gate_stops_shots_and_an_open_one_does_not() {
        let mut w = World::new(Theme::Sewer, 604);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        let cell = (12, 10);
        w.level.tiles.set(
            cell.0,
            cell.1,
            TileKind::Wall(WallKind::OneWayDoor {
                dir: Cardinal::East,
            }),
        );
        w.level.tiles.rebuild_masks();
        w.level
            .objects
            .one_way_doors
            .push(OneWayDoor::new(cell, Vec2::new(1.0, 0.0)));

        // Shut: the bullet dies on the leaf.
        eastbound(&mut w, 150.0, Faction::Player, 200.0, 0);
        let mut furthest = f32::MIN;
        for _ in 0..20 {
            tick_bullets(&mut w, 1.0 / 60.0);
            if let Some(b) = w.bullets().first() {
                furthest = furthest.max(b.pos.x);
            }
        }
        assert!(w.bullets().is_empty(), "gate did not consume the shot");
        assert!(
            furthest < 198.0,
            "shot passed a closed gate to x={furthest}"
        );

        // Half-open (progress 1.0 in `Opening`): the same shot goes through.
        w.level.objects.one_way_doors[0].view.state = OneWayDoorState::Opening;
        w.level.objects.one_way_doors[0].view.progress = 1.0;
        eastbound(&mut w, 150.0, Faction::Player, 200.0, 0);
        for _ in 0..20 {
            tick_bullets(&mut w, 1.0 / 60.0);
        }
        let b = w
            .bullets()
            .into_iter()
            .next()
            .expect("an open gate stopped a shot");
        assert!(b.pos.x > 208.0, "shot only reached x={}", b.pos.x);
    }

    /// Orbit bullets are exempt from the sweep: a 24 px ring cannot survive a
    /// 16 px half-width corridor otherwise, and the weapon would silently
    /// empty itself there.
    #[test]
    fn orbit_bullets_pass_through_walls() {
        let mut w = World::new(Theme::Sewer, 605);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        w.player.pos = Vec2::new(170.0, 168.0);
        wall_column(&mut w, 11, 10);
        let mut b = Bullet::new(
            w.player.pos,
            Vec2::ZERO,
            1.0,
            Faction::Player,
            (255, 255, 255),
        );
        b.life = 100.0;
        b.orbit = Some(Orbit {
            angle: 0.0,
            radius: 24.0,
            speed: 1.0,
        });
        w.spawn_bullet(b);

        for _ in 0..10 {
            tick_bullets(&mut w, 1.0 / 60.0);
        }

        let b = w.bullets().into_iter().next().expect("orbit bullet destroyed");
        assert!(
            b.pos.x > 176.0,
            "orbit bullet stopped at the corridor wall at x={}",
            b.pos.x
        );
    }

    /// One chained tick of the three systems that decide a shot's fate
    /// (`grid_build` for the enemy index, then movement/wall sweep, then hits).
    fn tick_shots(w: &mut World, schedule: &mut Schedule, ticks: u32) {
        for _ in 0..ticks {
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            schedule.run(&mut w.ecs);
            w.end_tick();
        }
    }

    fn shot_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                enemy_move::grid_build_system,
                bullets_system,
                combat::player_bullet_hits_system,
            )
                .chain(),
        );
        schedule
    }

    /// The ordering guarantee: a hit that happens *earlier* along the sweep
    /// than the wall still counts. The bullet is clamped at the wall in the
    /// same tick it crosses the cat, so an implementation that despawned the
    /// shot inside the movement pass would silently lose this hit.
    #[test]
    fn a_hit_before_the_wall_still_counts() {
        use crate::game::enemy::EnemyKind;

        let mut w = World::new(Theme::Sewer, 606);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        wall_column(&mut w, 12, 10);
        // Circle 182..190, i.e. on the same tick's sweep as the wall face at
        // 190 for a 2000 px/s shot starting at 160.
        let front = w.spawn_enemy(EnemyKind::Cat, Vec2::new(186.0, 168.0), 1.0);
        let front_hp = w.ecs.get::<Enemy>(front).unwrap().hp;
        eastbound(&mut w, 160.0, Faction::Player, 2000.0, 1);
        let mut schedule = shot_schedule();

        tick_shots(&mut w, &mut schedule, 10);

        assert!(
            w.ecs.get::<Enemy>(front).unwrap().hp < front_hp,
            "the hit earlier along the sweep was lost when the wall stopped the shot"
        );
    }

    /// A body on the far side of a wall is never reached: the shot is consumed
    /// at the near face, long before it could cross. (Unclamped, the 200 px/s
    /// shot would reach the cat at x=216 in about 19 ticks — well inside its
    /// 10 s life.)
    #[test]
    fn a_body_behind_the_wall_is_never_hit() {
        use crate::game::enemy::EnemyKind;

        let mut w = World::new(Theme::Sewer, 608);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        wall_column(&mut w, 12, 10);
        let behind = w.spawn_enemy(EnemyKind::Cat, Vec2::new(220.0, 168.0), 1.0);
        let behind_hp = w.ecs.get::<Enemy>(behind).unwrap().hp;
        eastbound(&mut w, 150.0, Faction::Player, 200.0, 0);
        let mut schedule = shot_schedule();

        tick_shots(&mut w, &mut schedule, 60);

        assert_eq!(
            w.ecs.get::<Enemy>(behind).unwrap().hp,
            behind_hp,
            "a body behind the wall took damage from a shot that should have stopped"
        );
        assert!(w.bullets().is_empty(), "the shot should be gone by now");
    }

    /// `los_clear` is the shared "can this get from A to B" question, and its
    /// radius argument is the mover's own: the rat-hole gap passes a shot and
    /// refuses a Brute-width body.
    #[test]
    fn los_clear_covers_shots_and_bodies() {
        let mut w = World::new(Theme::Sewer, 607);
        open_map(&mut w);
        let a = Vec2::new(150.0, 168.0);
        let b = Vec2::new(220.0, 168.0);
        assert!(los_clear(&w.level, a, b, 2.0), "open floor must be clear");

        w.level.tiles.set(
            12,
            10,
            TileKind::Wall(WallKind::RatHole {
                axis: WallAxis::Vertical,
            }),
        );
        w.level.tiles.rebuild_masks();
        assert!(
            los_clear(&w.level, a, b, 2.0),
            "a shot must see through the rat-hole gap"
        );
        assert!(
            !los_clear(&w.level, a, b, 6.5),
            "a Brute-width body must not see through an 8 px gap"
        );
        assert!(
            wall_hit(&w.level, a, b, 6.5).is_some(),
            "wall_hit must report the near face"
        );
    }
}

