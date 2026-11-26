//! Lava Breath's fixed fan, ballistic flight and molten floor residue.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::{cell_of, TILE_F};
use crate::game::ecs::components::{LavaGlob, LavaPool, SpawnId};
use crate::game::ecs::resources::{DamageLedger, EnemyIndex, NextSpawnId, RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::loadout::Stats;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::weapon::{
    WeaponKind, CALDERA_LIFE, CALDERA_RADIUS, LAVA_APEX, LAVA_DPS, LAVA_FAN_DEGREES, LAVA_FLIGHT,
    LAVA_LANDINGS, LAVA_MOUTH_OFFSET, LAVA_POOL_LIFE, LAVA_POOL_RADIUS, LAVA_POOL_TICK,
    LAVA_SPLASH_DAMAGE, LAVA_SPLASH_RADIUS,
};
use crate::math::Vec2;

use super::boss::Boss;
use super::evolutions::{damage_boss, enemy_order, hit_body, nearest_on_segment, BOSS_EFFECT_KEY};
use super::hazards::is_ground;
use super::projectiles::los_clear;

const KIND: WeaponKind = WeaponKind::LavaBreath;
const SOURCE: DamageSource = DamageSource::Lava;
const FLIER_HEIGHT: f32 = 8.0;

/// Launch stats travel with each glob, even if the weapon changes in flight.
#[derive(Component)]
pub(crate) struct LavaFlight {
    pool_life: f32,
    dps: f32,
    evolved: bool,
    fliers_hit: BTreeSet<u64>,
}

/// One per volley. Rendering reads the firing direction and the cycle that
/// set the weapon's timer, including any fire-rate modifier at launch.
#[derive(Component)]
pub(crate) struct LavaMuzzle {
    pub dir: Vec2,
    pub cycle: f32,
}

/// Pool-local contact history, keyed by SpawnId (or the boss effect key).
#[derive(Component, Default)]
pub(crate) struct LavaContacts {
    evolved: bool,
    bodies: BTreeMap<u64, (f32, f32)>,
}

impl LavaContacts {
    fn damage(&mut self, key: u64, age: f32, dt: f32, dps: f32) -> f32 {
        let (last_seen, exposure) = self.bodies.entry(key).or_default();
        *last_seen = age;
        *exposure += dt;
        let ticks = (*exposure / LAVA_POOL_TICK).floor();
        let spent = ticks * LAVA_POOL_TICK;
        *exposure -= spent;
        dps * spent
    }
}

/// Only check the destination: the glob can fly across an intervening wall.
/// If it would land in masonry, search backward along its throw line.
fn landing_floor(level: &Level, origin: Vec2, target: Vec2) -> Vec2 {
    let (mut x, mut y) = cell_of(target);
    if level.tiles.get(x, y).is_open() {
        return target;
    }
    let ray = origin - target;
    let axis = |at: f32, cell: i32, delta: f32| {
        if delta == 0.0 {
            return (0, f32::INFINITY, f32::INFINITY);
        }
        let step = if delta > 0.0 { 1 } else { -1 };
        let edge = (cell + i32::from(step > 0)) as f32 * TILE_F;
        (step, (edge - at) / delta, TILE_F / delta.abs())
    };
    let (step_x, mut next_x, stride_x) = axis(target.x, x, ray.x);
    let (step_y, mut next_y, stride_y) = axis(target.y, y, ray.y);
    let inset = 0.5 / ray.len();
    loop {
        let enter = next_x.min(next_y);
        if enter >= 1.0 {
            return origin;
        }
        if next_x == enter {
            x += step_x;
            next_x += stride_x;
        }
        if next_y == enter {
            y += step_y;
            next_y += stride_y;
        }
        if !level.tiles.get(x, y).is_open() {
            continue;
        }
        let exit = next_x.min(next_y).min(1.0);
        // Stay on the throw line and step just inside the first floor cell.
        let point = target + ray * (enter + ((exit - enter) * 0.5).min(inset));
        if level.tiles.get_at(point).is_open() {
            return point;
        }
    }
}

fn spawn_pool(commands: &mut Commands, next_id: &mut NextSpawnId, pool: LavaPool, evolved: bool) {
    commands.spawn((
        next_id.take(),
        pool,
        LavaContacts { evolved, ..Default::default() },
    ));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fire_lava(
    commands: &mut Commands,
    next_id: &mut NextSpawnId,
    level: &Level,
    input: &TickInput,
    stats: &Stats,
    pos: Vec2,
    face: Vec2,
    rank: u8,
    damage: f32,
    cycle: f32,
    evolved: bool,
) {
    let rank = usize::from(rank.clamp(1, 5) - 1);
    let aim = input.aim.map(|at| at - pos).filter(|dir| dir.len_sq() > 1e-6);
    let dir = aim.unwrap_or(face).normalized();
    let from = pos + dir * LAVA_MOUTH_OFFSET;
    let dps = LAVA_DPS[rank] * (damage / LAVA_SPLASH_DAMAGE[rank]);
    let landings = LAVA_LANDINGS[rank];
    for (slot, &distance) in landings.iter().enumerate() {
        let angle = (slot as f32 / (landings.len() - 1) as f32 - 0.5) * LAVA_FAN_DEGREES.to_radians();
        let to = landing_floor(level, from, from + dir.rotate(angle) * distance);
        let to = if level.tiles.get_at(to).is_open() { to } else { pos };
        let id = next_id.take();
        let mut entity = commands.spawn((
            id,
            LavaGlob {
                ground: from,
                from,
                to,
                t: 0.0,
                flight: LAVA_FLIGHT,
                apex: LAVA_APEX,
                damage,
                seed: id.0 as u32,
            },
            LavaFlight {
                pool_life: stats.zone_life(LAVA_POOL_LIFE),
                dps,
                evolved,
                fliers_hit: BTreeSet::new(),
            },
        ));
        if slot == 0 {
            entity.insert(LavaMuzzle { dir, cycle });
        }
    }
    if evolved {
        let seed = next_id.0 as u32;
        spawn_pool(
            commands,
            next_id,
            LavaPool {
                pos: landing_floor(level, pos, input.previous_pos),
                radius: CALDERA_RADIUS,
                age: 0.0,
                life: stats.zone_life(CALDERA_LIFE),
                dps,
                seed,
            },
            true,
        );
    }
}

/// First time on descent when the glob is low enough to splash a flier.
fn low_descent(glob: &LavaGlob) -> f32 {
    glob.flight * 0.5 * (1.0 + (1.0 - FLIER_HEIGHT / glob.apex).max(0.0).sqrt())
}

/// Runs immediately after Acid Rain while EnemyIndex still holds the ordered
/// live entity list. Ground bodies are hit at landing. A flier can also be
/// clipped on the last, low part of the descent, once per glob.
#[allow(clippy::too_many_arguments)] // Independent ECS queries and resources.
pub(crate) fn lava_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut globs: Query<(Entity, &SpawnId, &mut LavaGlob, &mut LavaFlight)>,
    mut pools: Query<(Entity, &SpawnId, &mut LavaPool, &mut LavaContacts)>,
    mut order_scratch: Local<Vec<Entity>>,
    mut pool_order: Local<Vec<(u64, Entity)>>,
    mut next_id: ResMut<NextSpawnId>,
    mut ledger: ResMut<DamageLedger>,
    mut numbers: ResMut<DamageNumbers>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    mut sfx: ResMut<SfxQueue>,
) {
    if globs.is_empty() && pools.is_empty() {
        return;
    }
    let dt = input.dt;
    let order = enemy_order(&index, &enemies, &mut order_scratch);
    let mut landed = false;
    for (entity, _, mut glob, mut flight) in globs.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
        let start = glob.t;
        glob.t = (glob.t + dt).min(glob.flight);
        glob.ground = glob.from + (glob.to - glob.from) * (glob.t / glob.flight);
        let touching_down = glob.t >= glob.flight;
        let descent = low_descent(&glob);
        if glob.t <= descent {
            continue;
        }
        let low_start = glob.from + (glob.to - glob.from) * (start.max(descent) / glob.flight);
        for &host in order {
            let Ok((_, id, mut enemy)) = enemies.get_mut(host) else { continue };
            if !enemy.alive() {
                continue;
            }
            let ground = is_ground(enemy.kind);
            let centre = if ground {
                if !touching_down {
                    continue;
                }
                glob.to
            } else {
                if flight.fliers_hit.contains(&id.0) {
                    continue;
                }
                nearest_on_segment(low_start, glob.ground, enemy.pos)
            };
            if enemy.pos.dist_sq(centre) > LAVA_SPLASH_RADIUS * LAVA_SPLASH_RADIUS
                || !los_clear(&level, centre, enemy.pos, 0.0)
            {
                continue;
            }
            if !ground {
                flight.fliers_hit.insert(id.0);
            }
            let killed = hit_body(&mut enemy, glob.damage, centre, KIND, &mut ledger, &mut numbers, SOURCE);
            vfx::emit_weapon_hit(
                &mut particles, &mut rng.0, enemy.pos, glob.to - glob.from,
                Some(KIND), enemy.kind, killed,
            );
        }
        if touching_down {
            for mut boss in bosses.iter_mut() {
                if boss_in_lava(&boss, &level, glob.to, LAVA_SPLASH_RADIUS)
                    && damage_boss(&mut boss, glob.damage, KIND, SOURCE, &mut ledger, &mut numbers)
                {
                    vfx::emit_weapon_hit(
                        &mut particles, &mut rng.0, boss.pos, glob.to - glob.from,
                        Some(KIND), EnemyKind::Brute, boss.hp <= 0.0,
                    );
                }
            }
            spawn_pool(
                &mut commands,
                &mut next_id,
                LavaPool {
                    pos: glob.to,
                    radius: LAVA_POOL_RADIUS,
                    age: 0.0,
                    life: flight.pool_life,
                    dps: flight.dps,
                    seed: glob.seed,
                },
                flight.evolved,
            );
            commands.entity(entity).despawn();
            landed = true;
        }
    }
    if landed {
        sfx.0.push(Sfx::SteamHiss);
    }

    pool_order.clear();
    pool_order.extend(pools.iter().map(|(entity, id, _, _)| (id.0, entity)));
    pool_order.sort_unstable_by_key(|&(id, _)| id);
    // New landing pools enter the query next tick via Commands. Merge before
    // burning so a pair never applies two ticks on the frame it joins.
    if pools.iter().any(|(_, _, _, contacts)| contacts.evolved) {
        merge_pools(&level, &pool_order, &mut pools);
    }
    for &(_, entity) in pool_order.iter() {
        let Ok((_, _, mut pool, mut contacts)) = pools.get_mut(entity) else { continue };
        let burn_dt = dt.min((pool.life - pool.age).max(0.0));
        pool.age += burn_dt;
        let pool = *pool;
        if burn_dt > 0.0 {
            for &host in order {
                let Ok((_, id, mut enemy)) = enemies.get_mut(host) else { continue };
                if !enemy.alive() || !is_ground(enemy.kind)
                    || pool.pos.dist_sq(enemy.pos) > pool.radius * pool.radius
                    || !los_clear(&level, pool.pos, enemy.pos, 0.0)
                {
                    continue;
                }
                let amount = contacts.damage(id.0, pool.age, burn_dt, pool.dps);
                if amount > 0.0 {
                    hit_body(&mut enemy, amount, pool.pos, KIND, &mut ledger, &mut numbers, SOURCE);
                }
            }
            for mut boss in bosses.iter_mut() {
                if boss_in_lava(&boss, &level, pool.pos, pool.radius) {
                    let key = BOSS_EFFECT_KEY | u64::from(boss.id);
                    let amount = contacts.damage(key, pool.age, burn_dt, pool.dps);
                    if amount > 0.0 {
                        damage_boss(&mut boss, amount, KIND, SOURCE, &mut ledger, &mut numbers);
                    }
                }
            }
            contacts.bodies.retain(|_, (last_seen, _)| *last_seen == pool.age);
        }
        if pool.age >= pool.life {
            commands.entity(entity).despawn();
        }
    }
}

fn boss_in_lava(boss: &Boss, level: &Level, pos: Vec2, radius: f32) -> bool {
    !boss.dead && boss.hit_shapes().circle_hits_grounded(pos, radius)
        && los_clear(level, pos, boss.pos, 0.0)
}

fn merge_pools(
    level: &Level,
    order: &[(u64, Entity)],
    pools: &mut Query<(Entity, &SpawnId, &mut LavaPool, &mut LavaContacts)>,
) {
    for (i, &(_, older)) in order.iter().enumerate() {
        let mut next = i + 1;
        while next < order.len() {
            let newer = order[next].1;
            next += 1;
            let Ok([(_, _, mut a, mut a_contacts), (_, _, mut b, mut b_contacts)]) =
                pools.get_many_mut([older, newer]) else { continue };
            let reach = a.radius + b.radius;
            if !(a_contacts.evolved || b_contacts.evolved)
                || a.age >= a.life || b.age >= b.life
                || a.pos.dist_sq(b.pos) > reach * reach
                || !los_clear(level, a.pos, b.pos, 0.0)
            {
                continue;
            }
            // Retain the older SpawnId and the larger pool's footprint.
            // Extend the remaining lifetime without resetting contact ticks.
            a.life += b.life - b.age;
            a.dps = a.dps.max(b.dps);
            if b.radius > a.radius {
                a.pos = b.pos;
                a.radius = b.radius;
            }
            a_contacts.evolved = true;
            for (key, (_, exposure)) in std::mem::take(&mut b_contacts.bodies) {
                let entry = a_contacts.bodies.entry(key).or_default();
                entry.1 = entry.1.max(exposure);
            }
            b.age = b.life;
            // A bigger pool, or a newly evolved one, may reach a pool we
            // already skipped. Each restart consumes one pool.
            next = i + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{cell_of, TileKind, TileMap};
    use crate::game::ecs::resources::spawn_with_id;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::level::{Theme, TileRect, MAP_COLS, MAP_ROWS};
    use crate::game::loadout::{Loadout, PassiveKind};
    use crate::game::weapon::Weapon;
    use crate::game::world::World;
    use rand::RngExt;

    fn arena(rank: u8) -> World {
        let mut world = World::new(Theme::Sewer, 81);
        world.clear_enemies();
        world.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, Theme::Sewer, 0);
        world.level.tiles.rebuild_masks();
        world.player.pos = Vec2::new(256.0, 264.0);
        world.player.face = Vec2::new(1.0, 0.0);
        world.player.loadout.weapons = vec![Weapon { kind: KIND, level: rank, timer: 0.0, evolved: false }];
        world
    }

    fn fire(world: &mut World, aim: Option<Vec2>) {
        world.begin_tick_with_aim(0.0, Vec2::ZERO, false, false, aim);
        world.ecs.run_system_cached(fire_weapons_system).unwrap();
        world.end_tick();
    }

    fn advance(world: &mut World, dt: f32) {
        world.begin_tick(dt, Vec2::ZERO, false);
        world.ecs.run_system_cached(lava_system).unwrap();
        world.end_tick();
    }

    fn globs(world: &mut World) -> Vec<LavaGlob> {
        let mut query = world.ecs.query::<(&SpawnId, &LavaGlob)>();
        query.iter(&world.ecs).sort_by_key::<&SpawnId, _>(|id| id.0)
            .map(|(_, glob)| *glob).collect()
    }

    fn pools(world: &mut World) -> Vec<LavaPool> {
        let mut query = world.ecs.query::<&LavaPool>();
        query.iter(&world.ecs).copied().collect()
    }

    fn isolate_glob(world: &mut World) -> LavaGlob {
        let mut query = world.ecs.query::<(Entity, &SpawnId, &LavaGlob)>();
        let mut spawned: Vec<_> = query.iter(&world.ecs)
            .sort_by_key::<&SpawnId, _>(|id| id.0)
            .map(|(entity, _, glob)| (entity, *glob)).collect();
        let (_, centre) = spawned.remove(1);
        for (side, _) in spawned {
            world.ecs.despawn(side);
        }
        centre
    }

    fn hp(world: &World, entity: Entity) -> f32 {
        world.ecs.get::<Enemy>(entity).unwrap().hp
    }

    #[test]
    fn volleys_match_the_fixed_symmetric_fan_and_aim() {
        for rank in 1..=5 {
            let mut world = arena(rank);
            let dir = if rank == 5 { Vec2::new(0.0, -1.0) } else { world.player.face };
            let aim = (rank == 5).then_some(world.player.pos + dir * 60.0);
            fire(&mut world, aim);
            let globs = globs(&mut world);
            let distances = LAVA_LANDINGS[usize::from(rank - 1)];
            assert_eq!(globs.len(), usize::from(rank) + 2);
            let mouth = world.player.pos + dir * LAVA_MOUTH_OFFSET;
            for (i, (glob, distance)) in globs.iter().zip(distances).enumerate() {
                let diameter = LAVA_FAN_DEGREES.to_radians();
                let angle = diameter * (i as f32 / (globs.len() - 1) as f32 - 0.5);
                assert_eq!(glob.from, mouth);
                let reach = glob.to - glob.from;
                assert!((reach.len() - *distance).abs() < 1e-3);
                assert!(reach.normalized().dist(dir.rotate(angle)) < 1e-4);
                let opposite = globs[globs.len() - 1 - i].to - mouth;
                assert!((reach.dot(dir.perp()) + opposite.dot(dir.perp())).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn ballistic_glob_lands_once_and_splashes_inside_nine_pixels() {
        let mut world = arena(1);
        fire(&mut world, None);
        let glob = isolate_glob(&mut world);
        let near = world.spawn_enemy(EnemyKind::Brute, glob.to + Vec2::new(0.0, 8.0), 10.0);
        let far = world.spawn_enemy(EnemyKind::Brute, glob.to + Vec2::new(0.0, 12.0), 10.0);
        let (near_hp, far_hp) = (hp(&world, near), hp(&world, far));
        advance(&mut world, LAVA_FLIGHT * 0.5);
        let live = globs(&mut world)[0];
        assert!((live.height() - LAVA_APEX).abs() < 1e-4);
        assert!(live.ground.dist((glob.from + glob.to) * 0.5) < 1e-4);
        assert!(pools(&mut world).is_empty());
        assert_eq!(hp(&world, near), near_hp);
        advance(&mut world, LAVA_FLIGHT * 0.5);
        assert!(globs(&mut world).is_empty());
        assert_eq!(hp(&world, near), near_hp - LAVA_SPLASH_DAMAGE[0]);
        assert_eq!(hp(&world, far), far_hp);
        let pools = pools(&mut world);
        assert_eq!(pools.len(), 1);
        let pool = pools[0];
        assert_eq!(pool.pos, glob.to);
        assert_eq!(pool.radius, LAVA_POOL_RADIUS);
        assert_eq!(pool.dps, LAVA_DPS[0]);
        assert_eq!(world.ecs.resource::<DamageLedger>().dealt[KIND.index()], LAVA_SPLASH_DAMAGE[0]);
    }

    #[test]
    fn fliers_take_one_low_splash_and_never_pool_damage() {
        let mut world = arena(1);
        fire(&mut world, None);
        let glob = isolate_glob(&mut world);
        let flyer = world.spawn_enemy(EnemyKind::Bat, glob.to, 10.0);
        let base_hp = hp(&world, flyer);
        advance(&mut world, LAVA_FLIGHT * 0.8);
        assert_eq!(hp(&world, flyer), base_hp);
        advance(&mut world, LAVA_FLIGHT * 0.15);
        assert_eq!(hp(&world, flyer), base_hp - LAVA_SPLASH_DAMAGE[0]);
        advance(&mut world, LAVA_FLIGHT * 0.1);
        advance(&mut world, 1.0);
        assert_eq!(hp(&world, flyer), base_hp - LAVA_SPLASH_DAMAGE[0]);
    }

    #[test]
    fn a_pool_ticks_each_body_on_its_own_clock_and_expires() {
        let mut world = arena(1);
        let pos = world.player.pos;
        let resident = world.spawn_enemy(EnemyKind::Brute, pos, 10.0);
        let arrival = world.spawn_enemy(EnemyKind::Brute, pos + Vec2::new(8.0, 0.0), 10.0);
        let base_hp = hp(&world, resident);
        spawn_with_id(&mut world.ecs, (
            LavaPool { pos, radius: LAVA_POOL_RADIUS, age: 0.0, life: LAVA_POOL_LIFE, dps: LAVA_DPS[0], seed: 0 },
            LavaContacts::default(),
        ));
        advance(&mut world, LAVA_POOL_TICK * 0.5);
        assert_eq!(hp(&world, resident), base_hp);
        world.ecs.get_mut::<Enemy>(arrival).unwrap().pos = pos;
        advance(&mut world, LAVA_POOL_TICK * 0.5);
        assert_eq!(hp(&world, arrival), base_hp);
        assert_eq!(hp(&world, resident), base_hp - LAVA_DPS[0] * LAVA_POOL_TICK);
        advance(&mut world, LAVA_POOL_LIFE);
        assert!(pools(&mut world).is_empty());
        assert_eq!(hp(&world, resident), base_hp - LAVA_DPS[0] * LAVA_POOL_LIFE);
        assert_eq!(hp(&world, arrival), base_hp - LAVA_DPS[0] * (LAVA_POOL_LIFE - LAVA_POOL_TICK));
        advance(&mut world, LAVA_POOL_TICK);
        assert_eq!(hp(&world, resident), base_hp - LAVA_DPS[0] * LAVA_POOL_LIFE);
    }

    #[test]
    fn a_wall_landing_pulls_back_and_open_floor_across_it_is_still_reachable() {
        let mut world = arena(1);
        let from = world.player.pos + world.player.face * LAVA_MOUTH_OFFSET;
        let intended = from + world.player.face * 26.0;
        let (cx, cy) = cell_of(intended);
        world.level.tiles.fill_cells(cx, cy, 1, 1, TileKind::Wall(WallKind::Solid));
        world.level.tiles.rebuild_masks();
        fire(&mut world, None);
        let glob = isolate_glob(&mut world);
        assert!(world.level.tiles.get_at(glob.to).is_open());
        assert_eq!(glob.to.y, intended.y);
        assert!(glob.to.x > from.x && glob.to.x < intended.x);
        let beyond = intended + Vec2::new(16.0, 0.0);
        assert_eq!(landing_floor(&world.level, from, beyond), beyond);
        advance(&mut world, LAVA_FLIGHT);
        assert_eq!(pools(&mut world)[0].pos, glob.to);
    }

    #[test]
    fn a_grounded_boss_takes_the_splash_and_pool_ticks() {
        let mut world = arena(1);
        fire(&mut world, None);
        let glob = isolate_glob(&mut world);
        let boss = Boss::new(
            BossKind::Cat, glob.to, 500.0, TileRect::new(0, 0, 32, 32), glob.to, false,
        );
        let boss = spawn_with_id(&mut world.ecs, boss);
        advance(&mut world, LAVA_FLIGHT);
        let splashed = world.ecs.get::<Boss>(boss).unwrap().hp;
        assert!(splashed < 500.0);
        advance(&mut world, LAVA_POOL_TICK);
        let burnt = world.ecs.get::<Boss>(boss).unwrap().hp;
        assert!(burnt < splashed);
        let credited = world.ecs.resource::<DamageLedger>().boss[KIND.index()];
        assert!((credited - (500.0 - burnt)).abs() < 1e-3);
    }

    #[test]
    fn caldera_drops_a_long_fuse_pool_at_the_previous_position_and_merges_life() {
        let mut world = arena(5);
        let mut weapon = world.player.loadout.weapons[0].clone();
        weapon.evolved = true;
        world.player.loadout = Loadout::from_parts(vec![weapon], vec![(PassiveKind::LongFuse, 1)]);
        let previous = world.player.pos;
        world.begin_tick(0.0, Vec2::ZERO, false);
        world.ecs.resource_mut::<crate::game::player::Player>().pos += Vec2::new(3.0, 0.0);
        world.ecs.run_system_cached(fire_weapons_system).unwrap();
        world.end_tick();
        let big = pools(&mut world)[0];
        assert_eq!(big.pos, previous);
        assert_eq!(big.radius, CALDERA_RADIUS);
        assert_eq!(big.life, world.player.loadout.stats.zone_life(CALDERA_LIFE));
        assert_eq!(globs(&mut world).len(), 7);

        let other_life = 1.0;
        spawn_with_id(&mut world.ecs, (
            LavaPool { pos: previous + Vec2::new(10.0, 0.0), radius: LAVA_POOL_RADIUS,
                       age: 0.5, life: other_life, dps: big.dps, seed: 99 },
            LavaContacts { evolved: true, ..Default::default() },
        ));
        advance(&mut world, 0.0);
        let remaining = pools(&mut world);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].life, big.life + other_life - 0.5);
    }

    #[test]
    fn firing_splashing_and_burning_draw_no_gameplay_rng() {
        let mut active = arena(1);
        let mut control = arena(1);
        for world in [&mut active, &mut control] {
            let near = world.player.pos + Vec2::new(31.0, 0.0);
            world.spawn_enemy(EnemyKind::Brute, near, 10.0);
        }
        fire(&mut active, None);
        advance(&mut active, LAVA_FLIGHT);
        advance(&mut active, LAVA_POOL_LIFE);
        assert!(active.ecs.resource::<DamageLedger>().dealt[KIND.index()] > LAVA_SPLASH_DAMAGE[0]);
        let fingerprint = |rng: &mut rand::rngs::StdRng| [rng.random::<u64>(), rng.random(), rng.random()];
        assert_eq!(fingerprint(&mut active.rng), fingerprint(&mut control.rng));
    }
}
