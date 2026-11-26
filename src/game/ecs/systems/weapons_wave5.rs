//! Deterministic ricochet, chain swing, and stillness pulse.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;

use crate::audio::Sfx;
use crate::game::art_wave5::HOLD_RADIUS;
use crate::game::collision::{segment_circle_hit, SizeClass, SpatialGrid};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::GRID_CELL;
use crate::game::ecs::resources::{
    BeadHitClock, Cvars, DamageLedger, EnemyIndex, HoldPhase, HoldPointFx, NextSpawnId, SfxQueue,
    TickInput,
};
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles, Shockwave};
use crate::game::player::Player;
use crate::game::weapon::{WeaponKind, THRASH_BOSS_KNOCKBACK};
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
const BOB_HALF: f32 = 75.0 * std::f32::consts::PI / 180.0;
const BOB_PASS: f32 = 0.55;
const SHED_REACH: f32 = 48.0;
const SHED_PASS: f32 = 0.30;

const HOLD: WeaponKind = WeaponKind::HoldPoint;
const HOLD_SPEED: f32 = 10.0;
const HOLD_CHARGE: f32 = 0.45;
const HOLD_CRACK: f32 = 0.6;
const HOLD_SHOVE: f32 = 50.0;
const HOLD_FLASH: f32 = 0.30;
const CLAMP_LIFE: f32 = 1.4;

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
    age: f32,
    damage: f32,
    hits: BTreeSet<u64>,
}

impl HoldPointFx {
    pub(crate) fn visual(&self) -> Option<(f32, f32, bool)> {
        match self.phase {
            HoldPhase::Ready => None,
            HoldPhase::Charging(t) => Some(((t / HOLD_CHARGE).clamp(0.0, 1.0), -1.0, false)),
            HoldPhase::Cracked(_) => Some((0.0, -1.0, true)),
            HoldPhase::Spent { age, evolved, .. } => {
                let duration = if evolved { CLAMP_LIFE } else { HOLD_FLASH };
                let fired = (age / HOLD_FLASH).clamp(f32::EPSILON, 1.0);
                (age < duration).then_some((0.0, fired, false))
            }
        }
    }

    pub(crate) fn gait_at(&self, player: &Player, level: &Level, pos: Vec2) -> f32 {
        if matches!(self.phase, HoldPhase::Spent { age, evolved: true, .. } if age < CLAMP_LIFE)
            && !player.buffs.is_giant()
            && self.centre.dist_sq(pos) <= HOLD_RADIUS * HOLD_RADIUS
            && los_clear(level, self.centre, pos, 0.0)
        {
            0.45
        } else {
            1.0
        }
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

    let damage_mult = player.damage_mult() * cvars.damage_mult.max(0.0);
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
    weapon.timer = (weapon.cooldown() / fire_rate).max(0.05);
    commands.spawn((bob, next_id.take()));
    sfx.0.push(Sfx::ShootWhip);
}

fn bob_endpoint(level: &Level, bob: &PlumbBob, pivot: Vec2, age: f32) -> Vec2 {
    let turn_end = BOB_PASS + if bob.evolved { 2.0 * SHED_PASS } else { 0.0 };
    let swing = if age <= BOB_PASS {
        age / BOB_PASS
    } else {
        (1.0 - (age - turn_end).max(0.0) / BOB_PASS).max(0.0)
    };
    let dir = Vec2::from_angle(bob.facing + (2.0 * swing - 1.0) * BOB_HALF);
    let swing_tip = bob_clear_step(level, pivot, pivot + dir * bob.reach);
    if bob.evolved && age > BOB_PASS && age < turn_end {
        let roll = (1.0 - ((age - BOB_PASS) / SHED_PASS - 1.0).abs()).max(0.0);
        bob_clear_step(level, swing_tip, swing_tip + dir.perp() * (roll * SHED_REACH))
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
    let turn_end = BOB_PASS + if bob.evolved { 2.0 * SHED_PASS } else { 0.0 };
    let end = turn_end + BOB_PASS;
    let start = bob.age;
    let target = (start + dt).min(end);
    let previous_pivot = bob.pivot;
    while bob.age < target {
        // Sample short chords, including the exact reversal of the swing or roll.
        let boundary = [BOB_PASS, BOB_PASS + SHED_PASS, turn_end, end].into_iter()
            .filter(|&t| t > bob.age)
            .fold(end, f32::min);
        bob.age = (bob.age + 1.0 / 60.0).min(boundary).min(target);
        bob.pivot = previous_pivot + (pivot - previous_pivot) * ((bob.age - start) / dt);
        let next = bob_endpoint(&hits.level, bob, bob.pivot, bob.age);
        let clear = bob_clear_step(&hits.level, bob.pos, next);
        hits.sweep(bob.pos, clear, BOB_RADIUS, bob.damage, BOB, |key| bob.hits.insert(key));
        bob.pos = next;
    }
    bob.age >= end
}

pub(crate) fn hold_point_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    cvars: Res<Cvars>,
    mut fx: ResMut<HoldPointFx>,
    mut grid: ResMut<SpatialGrid>,
    mut sfx: ResMut<SfxQueue>,
    mut hits: Wave5Hits,
) {
    let damage_mult = player.damage_mult() * cvars.damage_mult.max(0.0);
    let shove = player.loadout.stats.shove(HOLD_SHOVE);
    let centre = player.pos;
    let giant = player.buffs.is_giant();
    let suspended = player.buffs.weapons_suspended();
    let step = HOLD_SPEED * input.dt;
    let moving = player.vel.len_sq() >= HOLD_SPEED * HOLD_SPEED
        || (input.dt > 0.0 && centre.dist_sq(input.previous_pos) >= step * step);
    let Some(weapon) = player.loadout.weapons.iter_mut().find(|w| w.kind == HOLD) else {
        fx.phase = HoldPhase::Ready;
        return;
    };
    match &mut fx.phase {
        HoldPhase::Spent { age, .. } => {
            *age += input.dt;
            if !suspended {
                weapon.timer = (weapon.timer - input.dt).max(0.0);
            }
            if weapon.timer <= 1e-6 {
                fx.phase = HoldPhase::Ready;
            }
            return;
        }
        HoldPhase::Cracked(remaining) => {
            *remaining -= input.dt;
            if !suspended {
                weapon.timer = (weapon.timer - input.dt).max(0.0);
            }
            if *remaining <= 1e-6 || giant {
                fx.phase = HoldPhase::Ready;
            }
            return;
        }
        _ => {}
    }
    if suspended {
        fx.phase = HoldPhase::Ready;
        return;
    }
    if weapon.timer > 1e-6 {
        weapon.timer = (weapon.timer - input.dt).max(0.0);
        return;
    }
    if giant {
        weapon.timer = 0.0;
        fx.phase = HoldPhase::Ready;
        return;
    }
    if moving {
        if matches!(fx.phase, HoldPhase::Charging(t) if t > 0.0) {
            fx.phase = HoldPhase::Cracked(HOLD_CRACK);
            weapon.timer = HOLD_CRACK;
            sfx.0.push(Sfx::TrapSnap);
        } else {
            weapon.timer = 0.0;
            fx.phase = HoldPhase::Ready;
        }
        return;
    }
    let previous = match fx.phase {
        HoldPhase::Charging(t) => t,
        _ => 0.0,
    };
    fx.centre = centre;
    let charge = previous + input.dt;
    if charge + 1e-6 < HOLD_CHARGE {
        weapon.timer = 0.0;
        fx.phase = HoldPhase::Charging(charge);
        return;
    }

    weapon.timer = weapon.cooldown();
    fx.phase = HoldPhase::Spent { age: 0.0, evolved: weapon.evolved };
    sfx.0.push(Sfx::ShootNova);
    hold_pulse(centre, weapon.damage() * damage_mult, shove, &mut grid, &mut hits);
}

fn hold_pulse(
    centre: Vec2, damage: f32, shove: f32, grid: &mut SpatialGrid, hits: &mut Wave5Hits,
) {
    let order = enemy_order(&hits.index, &hits.enemies, &mut hits.order);
    let mut moved = false;
    for &entity in order {
        let Ok((_, _, mut enemy)) = hits.enemies.get_mut(entity) else { continue };
        let pos = enemy.pos;
        if !enemy.targetable() || !segment_circle_hit(centre, centre, HOLD_RADIUS, pos, enemy.radius)
            || !los_clear(&hits.level, centre, pos, 0.0)
        {
            continue;
        }
        let killed = hit_body(
            &mut enemy, damage, centre, HOLD, &mut hits.ledger, &mut hits.numbers, DamageSource::Normal,
        );
        vfx::emit_weapon_contact(&mut hits.particles, pos, pos - centre, HOLD, killed);
        if killed || enemy.crab_pot_held {
            continue;
        }
        let distance = shove * if enemy.kind.heavy() { THRASH_BOSS_KNOCKBACK } else { 1.0 };
        let class = SizeClass::of_enemy(enemy.kind);
        let dir = (pos - centre).normalized();
        // Test the whole shove so a body cannot skip a wall or a closed gate.
        let steps = (distance / 2.0).ceil() as usize;
        for i in 1..=steps {
            let to = pos + dir * (distance * i as f32 / steps as f32);
            if to != enemy.pos && hits.level.tiles.circle_clear(to, enemy.radius, class) {
                enemy.pos = to;
                moved = true;
            } else {
                break;
            }
        }
    }
    for (_, mut boss) in hits.bosses.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
        if boss.dead || !boss.hit_shapes().iter().any(|shape| {
            shape.grounded() && shape.overlaps_circle(centre, HOLD_RADIUS)
                && los_clear(&hits.level, centre, shape.center(), 0.0)
        }) || !damage_boss(
            &mut boss, damage * 0.5, HOLD, DamageSource::Normal, &mut hits.ledger, &mut hits.numbers,
        ) {
            continue;
        }
        let away = boss.pos - centre;
        if boss.hp > 0.0 {
            boss.shove(&hits.level, away, 4.0);
        }
        vfx::emit_weapon_contact(&mut hits.particles, boss.pos, away, HOLD, boss.hp <= 0.0);
    }
    if moved && hits.index.0.len() == order.len() {
        // EnemyIndex still names the same bodies. Update only their buckets.
        grid.reset(hits.level.arena, GRID_CELL);
        grid.fill(order.iter().filter_map(|&entity| hits.enemies.get(entity).ok().map(|(_, _, e)| e.pos)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::resources::spawn_with_id;
    use crate::game::ecs::systems::boss::BossKind;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, TileRect, MAP_COLS, MAP_ROWS};
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

    fn step_hold(world: &mut World, dt: f32) {
        world.begin_tick(dt, Vec2::ZERO, false);
        world.ecs.run_system_cached(hold_point_system).unwrap();
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

        let mut hold = arena(HOLD, 1, false);
        step_hold(&mut hold, 0.2);
        hold.player.buffs.white_coat = 5.0;
        step_hold(&mut hold, 0.4);
        assert!(hold.ecs.resource::<HoldPointFx>().visual().is_none());

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

    #[test]
    fn hold_point_cracks_then_fires_after_a_full_still_beat() {
        let mut world = arena(HOLD, 1, false);
        let pos = ORIGIN + Vec2::new(12.0, 0.0);
        let body = world.spawn_enemy(EnemyKind::Skitterer, pos, 10.0);
        let hp = world.ecs.get::<Enemy>(body).unwrap().hp;
        step_hold(&mut world, 0.20);
        world.player.vel = Vec2::new(10.0, 0.0);
        step_hold(&mut world, 0.01);
        let fx = world.ecs.resource::<HoldPointFx>();
        assert!(matches!(fx.phase, HoldPhase::Cracked(t) if t == HOLD_CRACK));
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp);

        world.player.vel = Vec2::ZERO;
        step_hold(&mut world, HOLD_CRACK);
        step_hold(&mut world, 0.44);
        assert_eq!(world.ecs.get::<Enemy>(body).unwrap().hp, hp);
        step_hold(&mut world, 0.01);
        let enemy = world.ecs.get::<Enemy>(body).unwrap();
        assert_eq!(enemy.hp, hp - 26.0);
        assert_eq!(enemy.pos, pos + Vec2::new(HOLD_SHOVE, 0.0));
        assert_eq!(world.player.loadout.weapons[0].timer, 2.2);
    }

    #[test]
    fn hold_point_shove_cannot_cross_a_wall() {
        let mut world = arena(HOLD, 1, false);
        let wall = TileKind::Wall(WallKind::Solid);
        world.level.tiles.fill_cells(18, 15, 1, 4, wall);
        world.level.tiles.rebuild_masks();
        let pos = ORIGIN + Vec2::new(20.0, 0.0);
        let body = world.spawn_enemy(EnemyKind::Skitterer, pos, 10.0);
        step_hold(&mut world, HOLD_CHARGE);
        let body = world.ecs.get::<Enemy>(body).unwrap();
        assert!(body.pos.x > pos.x);
        assert!(body.pos.x + body.radius <= 288.0);
    }

    #[test]
    fn clamp_slows_the_circle_and_preserves_the_boss_shove_cap() {
        let mut world = arena(HOLD, 1, true);
        let pos = ORIGIN + Vec2::new(20.0, 0.0);
        let boss = Boss::new(BossKind::Cat, pos, 500.0, TileRect::new(0, 0, 32, 32), pos, false);
        let boss = spawn_with_id(&mut world.ecs, boss);
        step_hold(&mut world, HOLD_CHARGE);
        let b = world.ecs.get::<Boss>(boss).unwrap();
        assert_eq!(b.hp, 500.0 - 13.0);
        assert_eq!(b.pos, pos + Vec2::new(4.0, 0.0));
        let gait = |world: &World, pos| {
            world.ecs.resource::<HoldPointFx>().gait_at(&world.player, &world.level, pos)
        };
        assert_eq!(gait(&world, ORIGIN), 0.45);
        assert_eq!(gait(&world, ORIGIN + Vec2::new(29.0, 0.0)), 1.0);
        step_hold(&mut world, CLAMP_LIFE);
        assert_eq!(gait(&world, ORIGIN), 1.0);

        step_hold(&mut world, 2.2);
        world.player.buffs.giant = 8.0;
        step_hold(&mut world, HOLD_CHARGE);
        assert_eq!(world.ecs.get::<Boss>(boss).unwrap().hp, 500.0 - 13.0);
        assert!(world.ecs.resource::<HoldPointFx>().visual().is_none());
    }
}
