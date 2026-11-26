//! Deterministic ricochet and chain swing.
//!
//! Plumb Bob uses a cosine swing over each 150° pass. The weight leaves each
//! extreme at rest and peaks in speed on the aim line. Hit sweeps and art share
//! the wall-stopped endpoint. Out and back share a hit set to preserve damage.

use std::collections::BTreeSet;
use std::f32::consts::PI;

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;

use crate::audio::Sfx;
use crate::detmath::DetF32;
use crate::game::collision::segment_circle_hit;
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::resources::{
    BeadHitClock, Cvars, DamageLedger, EnemyIndex, NextSpawnId, SfxQueue, TickInput,
};
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles, Shockwave};
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

use super::boss::Boss;
use super::evolutions::{damage_boss, enemy_order, hit_body, nearest_on_segment};
use super::projectiles::los_clear;
use super::weapons::cast_dir;

const BEAD: WeaponKind = WeaponKind::GlassBead;
const BEAD_RADIUS: f32 = 2.0;
const BEAD_SPEED: f32 = 120.0;
const BEAD_LEG: f32 = 80.0;
const BEAD_LIFE: f32 = 1.8;
const BEAD_REPEAT: f32 = 0.2;
const BEAD_DELAY: f32 = 0.12;
const LAST_BOUNCE_RADIUS: f32 = 16.0;
const WALL_GAP: f32 = 0.01;

const BOB: WeaponKind = WeaponKind::PlumbBob;
const BOB_RADIUS: f32 = 2.5;
const BOB_HALF: f32 = 75.0 * PI / 180.0;
pub(crate) const BOB_PASS: f32 = 0.55;
const SHED_REACH: f32 = 48.0;
pub(crate) const SHED_PASS: f32 = 0.30;

#[derive(Component)]
pub(crate) struct GlassBead {
    pub pos: Vec2,
    pub vel: Vec2,
    pub bounces: u8,
    pub delay: f32,
    age: f32,
    leg: f32,
    last_tile: Option<(i32, i32)>,
    damage: f32,
    evolved: bool,
    leg_hits: BTreeSet<u64>,
}

#[derive(Component)]
pub(crate) struct PlumbBob {
    pub pivot: Vec2,
    pub pos: Vec2,
    pub evolved: bool,
    facing: f32,
    reach: f32,
    age: f64,
    damage: f32,
    hits: BTreeSet<u64>,
}

impl PlumbBob {
    pub(crate) fn age(&self) -> f32 {
        self.age as f32
    }

    pub(crate) fn turn_end(&self) -> f32 {
        BOB_PASS + if self.evolved { 2.0 * SHED_PASS } else { 0.0 }
    }

    /// Normalized time over both passes, held at 1 while Shed Bob rolls.
    pub(crate) fn swing_phase(&self, age: f32) -> f32 {
        if age <= BOB_PASS {
            age / BOB_PASS
        } else {
            1.0 + ((age - self.turn_end()) / BOB_PASS).clamp(0.0, 1.0)
        }
    }

    pub(crate) fn angle_at(&self, age: f32) -> f32 {
        self.facing - BOB_HALF * (PI * self.swing_phase(age)).dcos()
    }

    pub(crate) fn offsets_at(&self, age: f32) -> (Vec2, Vec2) {
        let dir = Vec2::from_angle(self.angle_at(age));
        let roll = if self.evolved {
            (1.0 - ((age - BOB_PASS) / SHED_PASS - 1.0).abs()).max(0.0)
        } else {
            0.0
        };
        (dir * self.reach, dir.perp() * (roll * SHED_REACH))
    }

    pub(crate) fn endpoint_at(&self, level: &Level, age: f32) -> Vec2 {
        bob_endpoint(level, self, self.pivot, age)
    }
}

pub(super) fn fire_beads(
    commands: &mut Commands,
    next_id: &mut NextSpawnId,
    pos: Vec2,
    dir: Vec2,
    damage: f32,
    count: i32,
    evolved: bool,
) {
    for i in 0..count {
        commands.spawn((
            GlassBead {
                pos,
                vel: dir * BEAD_SPEED,
                bounces: 0,
                delay: i as f32 * BEAD_DELAY,
                age: 0.0,
                leg: 0.0,
                last_tile: None,
                damage,
                evolved,
                leg_hits: BTreeSet::new(),
            },
            next_id.take(),
        ));
    }
}

#[derive(SystemParam)]
pub(crate) struct Wave5Hits<'w, 's> {
    level: Res<'w, Level>,
    index: Res<'w, EnemyIndex>,
    enemies: Query<'w, 's, (Entity, &'static SpawnId, &'static mut Enemy)>,
    bosses: Query<'w, 's, (&'static SpawnId, &'static mut Boss)>,
    order: Local<'s, Vec<Entity>>,
    ledger: ResMut<'w, DamageLedger>,
    numbers: ResMut<'w, DamageNumbers>,
    particles: ResMut<'w, Particles>,
}

impl Wave5Hits<'_, '_> {
    fn sweep(
        &mut self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        damage: f32,
        kind: WeaponKind,
        mut can_hit: impl FnMut(u64) -> bool,
    ) {
        let order = enemy_order(&self.index, &self.enemies, &mut self.order);
        for &entity in order {
            let Ok((_, id, mut enemy)) = self.enemies.get_mut(entity) else { continue };
            if !enemy.targetable()
                || !segment_circle_hit(from, to, radius, enemy.pos, enemy.radius)
                || !los_clear(&self.level, nearest_on_segment(from, to, enemy.pos), enemy.pos, 0.0)
                || !can_hit(id.0)
            {
                continue;
            }
            let killed = hit_body(
                &mut enemy, damage, from, kind, &mut self.ledger, &mut self.numbers,
                DamageSource::Normal,
            );
            vfx::emit_weapon_contact(&mut self.particles, enemy.pos, to - from, kind, killed);
        }
        for (id, mut boss) in self.bosses.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
            if !boss.dead && boss.hp > 0.0 && boss.invuln <= 0.0 && boss.hit_shapes().iter().any(|shape| {
                let centre = shape.center();
                shape.grounded()
                    && shape.overlaps_segment(from, to, radius)
                    && los_clear(&self.level, nearest_on_segment(from, to, centre), centre, 0.0)
            }) && can_hit(id.0)
                && damage_boss(
                    &mut boss, damage, kind, DamageSource::Normal, &mut self.ledger, &mut self.numbers,
                )
            {
                vfx::emit_weapon_contact(
                    &mut self.particles, boss.pos, to - from, kind, boss.hp <= 0.0,
                );
            }
        }
    }
}

pub(crate) fn glass_bead_system(
    mut commands: Commands,
    input: Res<TickInput>,
    player: Res<Player>,
    mut beads: Query<(Entity, &SpawnId, &mut GlassBead)>,
    mut clock: ResMut<BeadHitClock>,
    mut sfx: ResMut<SfxQueue>,
    mut hits: Wave5Hits,
) {
    clock.0.retain(|_, time| {
        *time -= input.dt;
        *time > 1e-6
    });
    for (entity, _, mut bead) in beads.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
        let mut dt = input.dt;
        if bead.delay > 0.0 {
            if player.buffs.weapons_suspended() {
                commands.entity(entity).despawn();
                continue;
            }
            let wait = bead.delay.min(dt);
            bead.delay -= wait;
            dt -= wait;
            if bead.delay > 0.0 {
                continue;
            }
            bead.pos = player.pos;
            sfx.0.push(Sfx::Shoot);
        }
        dt = dt.min(BEAD_LIFE - bead.age);
        bead.age += dt;
        if advance_bead(&mut bead, dt * BEAD_SPEED, &mut clock, &mut hits) || bead.age >= BEAD_LIFE {
            commands.entity(entity).despawn();
        }
    }
}

/// Returns true when a fourth bounce consumes the bead. Each segment ends at a
/// wall or at the leg's 80 px limit. A turn without a fresh tile spends two
/// bounces, so the same grout line cannot keep restarting an overlap.
fn advance_bead(
    bead: &mut GlassBead,
    mut remaining: f32,
    clock: &mut BeadHitClock,
    hits: &mut Wave5Hits,
) -> bool {
    while remaining > 1e-5 {
        let from = bead.pos;
        let leg_left = BEAD_LEG - bead.leg;
        let distance = remaining.min(leg_left);
        let to = from + bead.vel * (distance / BEAD_SPEED);
        let wall = super::projectiles::wall_hit_face(&hits.level, from, to, BEAD_RADIUS);
        let travel = distance * wall.map_or(1.0, |(t, _, _)| t);
        let impact = from + bead.vel * (travel / BEAD_SPEED);
        hits.sweep(from, impact, BEAD_RADIUS, bead.damage, BEAD, |key| {
            if bead.leg_hits.contains(&key) || clock.0.contains_key(&key) {
                return false;
            }
            bead.leg_hits.insert(key);
            clock.0.insert(key, BEAD_REPEAT);
            true
        });
        bead.pos = impact;
        bead.leg += travel;
        remaining -= travel;

        let (normal, tile) = match wall {
            Some((_, normal, tile)) => (normal, Some(tile)),
            None if distance >= leg_left => (-bead.vel.normalized(), None),
            None => break,
        };
        let fresh = tile.is_some() && tile != bead.last_tile;
        bead.bounces += if fresh { 1 } else { 2 };
        bead.last_tile = tile.or(bead.last_tile);
        bead.pos += normal * WALL_GAP;
        vfx::emit_weapon_contact(&mut hits.particles, bead.pos, normal, BEAD, bead.bounces >= 4);
        if bead.bounces >= 4 {
            if bead.evolved {
                hits.sweep(bead.pos, bead.pos, LAST_BOUNCE_RADIUS, bead.damage * 0.5, BEAD, |_| true);
                hits.particles.push_shockwave(Shockwave {
                    pos: bead.pos,
                    start_r: 2.0,
                    end_r: LAST_BOUNCE_RADIUS,
                    life: 0.22,
                    max_life: 0.22,
                    color: BEAD.color(),
                    core: (236, 254, 255),
                    thick: 1.0,
                    additive: true,
                    arc_dir: None,
                    arc_half_angle: 0.0,
                });
            }
            return true;
        }
        bead.vel -= normal * (2.0 * bead.vel.dot(normal));
        bead.leg = 0.0;
        bead.leg_hits.clear();
    }
    false
}

#[allow(clippy::too_many_arguments)] // Independent ECS queries and resources.
pub(crate) fn plumb_bob_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    cvars: Res<Cvars>,
    mut next_id: ResMut<NextSpawnId>,
    mut bobs: Query<(Entity, &SpawnId, &mut PlumbBob)>,
    mut sfx: ResMut<SfxQueue>,
    mut hits: Wave5Hits,
) {
    if player.buffs.weapons_suspended() {
        for (entity, _, _) in bobs.iter_mut() {
            commands.entity(entity).despawn();
        }
        return;
    }
    if !bobs.is_empty() {
        for (entity, _, mut bob) in bobs.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
            if advance_bob(&mut bob, player.pos, input.dt, &mut hits) {
                commands.entity(entity).despawn();
            }
        }
        return;
    }

    let damage_mult = player.damage_mult() * cvars.damage_mult.max(0.0)
        * player.loadout.weapon_damage_mult(BOB);
    let cooldown_mult = player.loadout.weapon_cooldown_mult(BOB);
    let fire_rate = player.loadout.stats.fire_rate_mult;
    let pos = player.pos;
    let face = player.face;
    let Some(weapon) = player.loadout.weapons.iter_mut().find(|w| w.kind == BOB) else { return };
    weapon.timer -= input.dt;
    if weapon.timer > 0.0 {
        return;
    }
    let mut bob = PlumbBob {
        pivot: pos,
        pos,
        evolved: weapon.evolved,
        facing: cast_dir(pos, face, input.aim).angle(),
        reach: if weapon.level >= 4 { 42.0 } else { 34.0 },
        age: 0.0,
        damage: weapon.damage() * damage_mult,
        hits: BTreeSet::new(),
    };
    bob.pos = bob_endpoint(&hits.level, &bob, pos, 0.0);
    weapon.timer = (weapon.cooldown() * cooldown_mult / fire_rate).max(0.05);
    commands.spawn((bob, next_id.take()));
    sfx.0.push(Sfx::ShootWhip);
}

fn bob_endpoint(level: &Level, bob: &PlumbBob, pivot: Vec2, age: f32) -> Vec2 {
    let (swing, roll) = bob.offsets_at(age);
    let swing_tip = bob_clear_step(level, pivot, pivot + swing);
    if roll != Vec2::ZERO {
        bob_clear_step(level, swing_tip, swing_tip + roll)
    } else {
        swing_tip
    }
}

fn bob_clear_step(level: &Level, from: Vec2, to: Vec2) -> Vec2 {
    let t = super::projectiles::wall_hit(level, from, to, BOB_RADIUS)
        .map_or(1.0, |t| (t - 0.001).max(0.0));
    from + (to - from) * t
}

fn advance_bob(bob: &mut PlumbBob, pivot: Vec2, dt: f32, hits: &mut Wave5Hits) -> bool {
    let turn_end = bob.turn_end();
    let end = f64::from(turn_end + BOB_PASS);
    let start = bob.age;
    // Accumulate frame time in f64, then snap only at the cast's final
    // boundary to absorb rounding in f32 inputs like BOB_PASS / 33.
    let target = start + f64::from(dt);
    let target = if target + 1e-7 >= end { end } else { target };
    let previous_pivot = bob.pivot;
    while bob.age < target {
        // Sample short chords, including the exact reversal of the swing or roll.
        let boundary = [BOB_PASS, BOB_PASS + SHED_PASS, turn_end].into_iter()
            .map(f64::from)
            .filter(|&t| t > bob.age)
            .fold(end, f64::min);
        bob.age = (bob.age + f64::from(1.0_f32 / 60.0)).min(boundary).min(target);
        let tick_progress = ((bob.age - start) / f64::from(dt)).min(1.0) as f32;
        bob.pivot = previous_pivot + (pivot - previous_pivot) * tick_progress;
        let next = bob_endpoint(&hits.level, bob, bob.pivot, bob.age as f32);
        let clear = bob_clear_step(&hits.level, bob.pos, next);
        hits.sweep(bob.pos, clear, BOB_RADIUS, bob.damage, BOB, |key| bob.hits.insert(key));
        bob.pos = next;
    }
    bob.age >= end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{SizeClass, TileKind, TileMap};
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::weapon::Weapon;
    use crate::game::world::World;
    use rand::RngExt;

    const ORIGIN: Vec2 = Vec2 { x: 256.0, y: 264.0 };

    fn arena(kind: WeaponKind, level: u8, evolved: bool) -> World {
        let mut world = World::new(Theme::Sewer, 84);
        world.clear_enemies();
        world.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, Theme::Sewer, 0);
        world.player.pos = ORIGIN;
        world.player.face = Vec2::new(1.0, 0.0);
        world.player.loadout.weapons = vec![Weapon { kind, level, evolved, timer: 0.0 }];
        world
    }

    fn fire(world: &mut World) {
        world.begin_tick(0.0, Vec2::ZERO, false);
        world.ecs.run_system_cached(fire_weapons_system).unwrap();
        world.end_tick();
    }

    fn step_bead(world: &mut World, dt: f32) {
        world.begin_tick(dt, Vec2::ZERO, false);
        world.ecs.run_system_cached(glass_bead_system).unwrap();
        world.end_tick();
    }

    fn step_bob(world: &mut World, dt: f32) {
        world.begin_tick(dt, Vec2::ZERO, false);
        world.ecs.run_system_cached(plumb_bob_system).unwrap();
        world.end_tick();
    }

    #[test]
    fn bead_reflects_and_last_bounce_bursts_on_the_fourth_wall() {
        let mut world = arena(BEAD, 1, true);
        let wall = TileKind::Wall(WallKind::Solid);
        world.level.tiles.fill_cells(14, 14, 1, 5, wall);
        world.level.tiles.fill_cells(17, 14, 1, 5, wall);
        world.level.tiles.rebuild_masks();
        let body = world.spawn_enemy(EnemyKind::Skitterer, Vec2::new(248.0, 274.0), 10.0);
        let hp = world.ecs.get::<Enemy>(body).unwrap().hp;
        let mut reference = arena(BEAD, 1, true);
        reference.spawn_enemy(EnemyKind::Skitterer, Vec2::new(248.0, 274.0), 10.0);

        fire(&mut world);
        step_bead(&mut world, 0.15);
        let mut query = world.ecs.query::<&GlassBead>();
        let bead = query.iter(&world.ecs).next().unwrap();
        assert_eq!(bead.bounces, 1);
        assert_eq!(bead.vel, Vec2::new(-BEAD_SPEED, 0.0));
        assert!((bead.pos.x - 266.0).abs() < 0.03);
        step_bead(&mut world, 0.50);
        assert_eq!(query.iter(&world.ecs).next().unwrap().bounces, 3);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp);
        step_bead(&mut world, 0.20);
        assert_eq!(query.iter(&world.ecs).len(), 0);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 4.0);
        assert_eq!(world.rng.random::<u64>(), reference.rng.random::<u64>());
    }

    #[test]
    fn bead_rank_three_delays_the_partner_and_shares_the_repeat_window() {
        let mut world = arena(BEAD, 3, false);
        let body = world.spawn_enemy(EnemyKind::Skitterer, ORIGIN + Vec2::new(7.0, 0.0), 10.0);
        let hp = world.ecs.get::<Enemy>(body).unwrap().hp;
        fire(&mut world);
        step_bead(&mut world, 0.10);
        let mut query = world.ecs.query::<&GlassBead>();
        assert_eq!(query.iter(&world.ecs).filter(|b| b.delay > 0.0).count(), 1);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 12.0);

        step_bead(&mut world, 0.05);
        assert!(query.iter(&world.ecs).all(|b| b.delay <= 0.0));
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 12.0);
        step_bead(&mut world, 0.05);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 12.0);
    }

    #[test]
    fn bead_without_a_wall_spends_two_bounces_at_the_leg_limit() {
        let mut world = arena(BEAD, 1, false);
        fire(&mut world);
        step_bead(&mut world, BEAD_LEG / BEAD_SPEED + 0.01);
        let mut query = world.ecs.query::<&GlassBead>();
        let bead = query.iter(&world.ecs).next().unwrap();
        assert_eq!(bead.bounces, 2);
        assert_eq!(bead.vel.x, -BEAD_SPEED);
        step_bead(&mut world, BEAD_LEG / BEAD_SPEED);
        assert_eq!(query.iter(&world.ecs).len(), 0);
    }

    #[test]
    fn white_coat_stops_attached_wave5_attacks_and_leaves_launched_beads() {
        let mut bob = arena(BOB, 1, false);
        step_bob(&mut bob, 0.0);
        let recovery = bob.player.loadout.weapons[0].timer;
        bob.player.buffs.white_coat = 5.0;
        step_bob(&mut bob, 0.2);
        let mut bobs = bob.ecs.query::<&PlumbBob>();
        assert_eq!(bobs.iter(&bob.ecs).len(), 0);
        assert_eq!(bob.player.loadout.weapons[0].timer, recovery);

        let mut bead = arena(BEAD, 3, false);
        fire(&mut bead);
        step_bead(&mut bead, 0.05);
        bead.player.buffs.white_coat = 5.0;
        step_bead(&mut bead, 0.10);
        let mut beads = bead.ecs.query::<&GlassBead>();
        let mut live = beads.iter(&bead.ecs);
        assert!(live.next().is_some_and(|shot| shot.pos.dist(ORIGIN) > 16.0));
        assert!(live.next().is_none());
    }

    #[test]
    fn bob_arc_starts_at_rest_and_moves_fastest_through_the_middle() {
        let mut world = arena(BOB, 1, false);
        step_bob(&mut world, 0.0);
        let mut query = world.ecs.query::<&PlumbBob>();
        let bob = query.single(&world.ecs).unwrap();

        let arc = |a, b| (bob.angle_at(b * BOB_PASS) - bob.angle_at(a * BOB_PASS)).abs();
        let middle = arc(0.45, 0.55);
        assert!(middle > arc(0.25, 0.35));
        assert!(middle > arc(0.0, 0.1) * 5.0);
        assert!(middle > arc(0.9, 1.0) * 5.0);
        assert!((arc(0.0, 0.1) - arc(0.9, 1.0)).abs() < 1e-6);
        assert_eq!(bob.angle_at(0.0), -BOB_HALF);
        assert_eq!(bob.angle_at(BOB_PASS), BOB_HALF);
        assert_eq!(bob.angle_at(2.0 * BOB_PASS), -BOB_HALF);
    }

    #[test]
    fn bob_reaches_each_extreme_on_time_across_tick_splits() {
        for steps in [1, 3, 11, 33, 61] {
            let mut world = arena(BOB, 1, false);
            let dt = BOB_PASS / steps as f32;
            let end = ORIGIN + Vec2::from_angle(BOB_HALF) * 34.0;
            let start = ORIGIN + Vec2::from_angle(-BOB_HALF) * 34.0;
            step_bob(&mut world, 0.0);
            let mut query = world.ecs.query::<&PlumbBob>();
            assert_eq!(query.single(&world.ecs).unwrap().pos, start);
            for _ in 0..steps {
                step_bob(&mut world, dt);
            }
            let bob = query.single(&world.ecs).unwrap();
            assert!((bob.age() - BOB_PASS).abs() < 1e-7);
            assert_eq!(bob.pos, end);
            for _ in 1..steps {
                step_bob(&mut world, dt);
            }
            assert!(query.single(&world.ecs).is_ok());
            step_bob(&mut world, dt);
            assert!(query.iter(&world.ecs).next().is_none());
        }
    }

    #[test]
    fn only_the_bob_hits_and_the_cooldown_follows_both_passes() {
        let mut world = arena(BOB, 1, false);
        let chain = world.spawn_enemy(EnemyKind::Skitterer, ORIGIN + Vec2::new(17.0, 0.0), 10.0);
        let tip = world.spawn_enemy(EnemyKind::Skitterer, ORIGIN + Vec2::new(34.0, 0.0), 10.0);
        let hp = world.ecs.get::<Enemy>(tip).unwrap().hp;
        step_bob(&mut world, 0.0);
        step_bob(&mut world, BOB_PASS);
        assert_eq!(world.ecs.get::<Enemy>(chain).unwrap().hp, hp);
        assert_eq!(world.ecs.get::<Enemy>(tip).unwrap().hp, hp - 11.0);
        step_bob(&mut world, BOB_PASS);
        let mut query = world.ecs.query::<&PlumbBob>();
        assert_eq!(query.iter(&world.ecs).len(), 0);
        assert_eq!(world.ecs.get::<Enemy>(tip).unwrap().hp, hp - 11.0);
        assert_eq!(world.player.loadout.weapons[0].timer, 0.85);
    }

    #[test]
    fn bob_chain_bows_under_acceleration_and_draws_a_still_frame_identically() {
        use crate::game::art_wave5::bob_chain;
        use crate::render::framebuffer::PixelBuffer;
        let mut world = arena(BOB, 1, false);
        step_bob(&mut world, 0.0);
        step_bob(&mut world, BOB_PASS / 6.0);
        let mut query = world.ecs.query::<&PlumbBob>();
        let slack = |world: &World, bob: &PlumbBob| {
            let nodes = bob_chain(bob, world.player.face);
            assert_eq!(nodes[nodes.len() - 1], bob.pos);
            assert_ne!(nodes[0], bob.pivot);
            let length: f32 = nodes.windows(2).map(|pair| pair[0].dist(pair[1])).sum();
            length - nodes[0].dist(bob.pos)
        };
        assert!(slack(&world, query.single(&world.ecs).unwrap()) > 0.5);
        step_bob(&mut world, BOB_PASS / 3.0);
        assert!(slack(&world, query.single(&world.ecs).unwrap()) < 0.05);
        for zoom in [1.0, 2.0] {
            world.zoom_view = zoom;
            let mut first = PixelBuffer::new(96, 92);
            let mut second = PixelBuffer::new(96, 92);
            world.draw(&mut first);
            world.draw(&mut second);
            assert_eq!(first.pixels(), second.pixels());
        }
    }

    /// Optional headless capture of both passes at normal and widest zoom.
    #[test]
    fn capture_bob_swing() {
        use crate::render::framebuffer::PixelBuffer;
        let Ok(dir) = std::env::var("RATRUN_BOB_CAPTURE_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut world = World::new(Theme::Sewer, 84);
        world.configure_testbed();
        world.clear_enemies();
        let hall = world.level.largest_hall().unwrap().rect;
        world.player.pos = hall.cells()
            .map(|(x, y)| TileMap::tile_rect(x, y).center())
            .filter(|&pos| world.level.tiles.circle_clear(pos, 48.0, SizeClass::Small))
            .min_by(|a, b| a.dist_sq(hall.center_world()).total_cmp(&b.dist_sq(hall.center_world())))
            .expect("clear space for the full arc");
        world.player.face = Vec2::new(1.0, 0.0);
        world.player.loadout.weapons = vec![Weapon { kind: BOB, level: 5, evolved: false, timer: 0.0 }];
        world.cam_focus = Some(world.player.pos + Vec2::new(12.0, 0.0));
        step_bob(&mut world, 0.0);
        let mut query = world.ecs.query::<&PlumbBob>();
        let mut frame = PixelBuffer::new(100, 96);
        let mut last = 0.0;
        let phases = [0.0, 0.12, 0.30, 0.50, 0.78, 1.0, 1.12, 1.30, 1.50, 1.78, 2.0];
        for (i, phase) in phases.into_iter().enumerate() {
            let age = (phase * BOB_PASS).min(2.0 * BOB_PASS - 0.001);
            world.elapsed = age;
            step_bob(&mut world, age - last);
            last = age;
            let bob = query.single(&world.ecs).unwrap();
            assert!((bob.pos.dist(bob.pivot) - 42.0).abs() < 1e-3, "capture arc clips a wall");
            for zoom in [1, 2] {
                world.zoom_view = zoom as f32;
                world.draw(&mut frame);
                let rgb: Vec<_> = frame.pixels().iter().flat_map(|&(r, g, b)| [r, g, b]).collect();
                image::save_buffer(
                    dir.join(format!("plumb-bob-{zoom}x-{i:02}.png")),
                    &rgb,
                    frame.w as u32,
                    frame.h as u32,
                    image::ColorType::Rgb8,
                ).unwrap();
            }
        }
    }

    #[test]
    fn shed_bob_rolls_off_the_tangent_and_reels_back() {
        let mut world = arena(BOB, 4, true);
        let end_dir = Vec2::from_angle(BOB_HALF);
        let tip = ORIGIN + end_dir * 42.0;
        let roll_end = tip + end_dir.perp() * SHED_REACH;
        let body = world.spawn_enemy(EnemyKind::Skitterer, roll_end, 10.0);
        let hp = world.ecs.get::<Enemy>(body).unwrap().hp;
        step_bob(&mut world, 0.0);
        step_bob(&mut world, BOB_PASS + SHED_PASS);
        let mut query = world.ecs.query::<&PlumbBob>();
        assert!(query.iter(&world.ecs).next().unwrap().pos.dist(roll_end) < 1e-3);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 19.0);
        step_bob(&mut world, SHED_PASS);
        assert!(query.iter(&world.ecs).next().unwrap().pos.dist(tip) < 1e-3);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp - 19.0);
    }
}
