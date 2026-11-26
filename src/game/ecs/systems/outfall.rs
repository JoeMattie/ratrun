//! Outfall motion runs after the rat has moved and before puddles, fog and
//! the director. The second pass sees final enemy positions after the last
//! spatial-cache reader and before any loot is created.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::{cell_of, SizeClass, TileMap};
use crate::game::director::{self, Director};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{
    BossDue, DashTrack, DebugFlags, DifficultyLevel, NextEnemyId, NextSpawnId, RunCounters, SfxQueue,
    TickInput,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Gold, Pickup};
use crate::game::hazards::{
    clamp_outfall_displacement, lure_landing_clear, outfall_sand_clear, outfall_surge,
    outfall_surge_enabled, LurePhase, LURE_ARM_RADIUS, LURE_WARN_SECS, SURGE_ACTIVE_SECS,
    SURGE_DISTANCE, SURGE_PERIOD,
};
use crate::game::level::{Level, Theme};
use crate::game::player::Player;
use crate::math::Vec2;

use super::boss::{Boss, BossKind};
use super::hazards::is_ground;
use super::trailer::{
    FALL_ENEMY_FRACTION, FALL_ENEMY_STUN, FALL_PLAYER_DAMAGE, FALL_PLAYER_IFRAMES,
};

const RECOVERY_CELLS: i32 = 7;

fn active_slice(previous_elapsed: f32, elapsed: f32) -> f32 {
    let end = f64::from(elapsed);
    let start = f64::from(previous_elapsed).max(0.0);
    let period = f64::from(SURGE_PERIOD);
    let duration = f64::from(SURGE_ACTIVE_SECS);
    // Cumulative active time starts at cycle one. Subtracting two samples
    // accounts for both partial ticks, including a tick ending after the surge.
    let active_until = |time: f64| {
        let cycle = (time / period).floor();
        (cycle - 1.0).max(0.0) * duration
            + if cycle >= 1.0 { (time - cycle * period).min(duration) } else { 0.0 }
    };
    (active_until(end) - active_until(start)).max(0.0) as f32
}

#[allow(clippy::too_many_arguments)]
pub fn outfall_motion_system(
    mut commands: Commands,
    mut level: ResMut<Level>,
    counters: Res<RunCounters>,
    input: Res<TickInput>,
    difficulty: Res<DifficultyLevel>,
    arrival: (Res<Director>, Res<BossDue>),
    debug: Res<DebugFlags>,
    dash: Res<DashTrack>,
    mut player: ResMut<Player>,
    mut enemies: Query<(Entity, &mut Enemy)>,
    mut gems: Query<&mut Gem>,
    bosses: Query<&Boss>,
    mut sfx: ResMut<SfxQueue>,
    ids: (ResMut<NextSpawnId>, ResMut<NextEnemyId>),
    mut origins: ResMut<OutfallOrigins>,
) {
    if level.theme != Theme::Outfall {
        return;
    }
    let elapsed = counters.elapsed;
    let previous_elapsed = origins.last_elapsed
        .filter(|&last| last + input.dt == elapsed)
        .unwrap_or(elapsed - input.dt);
    origins.last_elapsed = Some(elapsed);
    let (director, due) = arrival;
    let enabled = outfall_surge_enabled(
        director.boss_spawned
            || due.is_pending()
            || due.claimed
            || bosses.iter().any(|boss| boss.kind == BossKind::Lantern),
        counters.testbed,
        elapsed,
    );
    let shove = if enabled {
        outfall_surge(level.seed, elapsed).dir
            * (SURGE_DISTANCE / SURGE_ACTIVE_SECS * active_slice(previous_elapsed, elapsed))
    } else {
        Vec2::ZERO
    };

    if player.alive() {
        let (radius, class) = player.movement_body(&level.tiles);
        if level.pit_at(player.pos).is_some() {
            if let Some(safe) = recover(&level, player.pos, Some(input.previous_pos), radius, class) {
                let damaged = player.take_damage(FALL_PLAYER_DAMAGE);
                player.pos = safe;
                player.vel = Vec2::ZERO;
                player.iframes = player.iframes.max(FALL_PLAYER_IFRAMES);
                if damaged {
                    sfx.0.push(Sfx::Hurt);
                }
            }
        } else if player.dash_active <= 0.0 && dash.segment.is_none() {
            let step = if player.buffs.is_giant() { shove * 0.5 } else { shove };
            player.pos = clamp_outfall_displacement(&level, player.pos, step, radius, class);
        }
    }

    origins.positions.clear();
    let mut fell = false;
    for (entity, mut enemy) in &mut enemies {
        let in_drop = level.pit_at(enemy.pos).is_some();
        if in_drop {
            fell |= recover_enemy(&level, &mut enemy, None);
        } else if !debug.freeze && drifts(&enemy) {
            let radius = enemy.radius;
            let class = SizeClass::of_enemy(enemy.kind);
            enemy.pos = clamp_outfall_displacement(&level, enemy.pos, shove, radius, class);
        }
        origins.positions.insert(entity, enemy.pos);
    }

    for mut gem in &mut gems {
        if level.pit_at(gem.pos).is_some() {
            if recover_loot(&level, &mut gem.pos) {
                gem.vel = Vec2::ZERO;
            }
        } else if !gem.magnetized {
            gem.pos = clamp_outfall_displacement(&level, gem.pos, shove, 0.0, SizeClass::Small);
        }
    }
    if fell {
        sfx.0.push(Sfx::TrapTrigger);
    }

    // Move the vector out briefly so release can check the same Level used
    // by motion and by generator validation. This keeps its allocation.
    let mut traps = std::mem::take(&mut level.objects.lure_traps);
    let (mut next_spawn, mut next_enemy) = ids;
    for trap in &mut traps {
        match trap.phase {
            LurePhase::Ready => {
                if enabled && player.alive() && trap.pos.dist_sq(player.pos) <= LURE_ARM_RADIUS * LURE_ARM_RADIUS {
                    trap.phase = LurePhase::Warning;
                    trap.warning_left = LURE_WARN_SECS;
                    sfx.0.push(Sfx::TrapTrigger);
                }
            }
            LurePhase::Warning => {
                trap.warning_left = (trap.warning_left - input.dt).max(0.0);
                if trap.warning_left > 0.0 {
                    continue;
                }
                trap.phase = LurePhase::Spent;
                let kind = EnemyKind::Conger;
                let clear_rat = PLAYER_RADIUS + kind.radius();
                if let Some(at) = trap.landings.iter().copied().find(|&at| {
                    lure_landing_clear(&level, trap.pos, at)
                        && at.dist_sq(player.pos) > clear_rat * clear_rat
                }) {
                    let hp = director::horde_hp_mult_for(difficulty.0, elapsed);
                    let mods = director::enemy_mods_for(difficulty.0, elapsed);
                    let mut enemy = Enemy::spawn_with_phases(kind, at, hp, 1.5, 0.0, 0.0);
                    enemy.damage = kind.damage() * mods.damage;
                    enemy.speed_scale = mods.speed;
                    enemy.id = next_enemy.take();
                    enemy.facing = (player.pos - at).normalized();
                    commands.spawn((enemy, next_spawn.take()));
                } else {
                    trap.failed = true;
                }
            }
            LurePhase::Spent => trap.spent_age = (trap.spent_age + input.dt).min(1.0),
        }
    }
    level.objects.lure_traps = traps;
}

fn drifts(enemy: &Enemy) -> bool {
    enemy.alive() && !enemy.named && !enemy.elite && !enemy.kind.heavy()
        && enemy.kind != EnemyKind::Grouper && !enemy.position_locked
}

#[derive(Resource, Default)]
pub struct OutfallOrigins {
    positions: HashMap<Entity, Vec2>,
    last_elapsed: Option<f32>,
}

pub fn outfall_falls_system(
    level: Res<Level>,
    mut enemies: Query<(Entity, &mut Enemy)>,
    mut gems: Query<&mut Gem>,
    mut gold: Query<&mut Gold>,
    mut pickups: Query<&mut Pickup>,
    mut sfx: ResMut<SfxQueue>,
    origins: Res<OutfallOrigins>,
) {
    if level.theme != Theme::Outfall {
        return;
    }
    let mut fell = false;
    for (entity, mut enemy) in &mut enemies {
        if level.pit_at(enemy.pos).is_some() {
            fell |= recover_enemy(&level, &mut enemy, origins.positions.get(&entity).copied());
        }
    }
    for mut gem in &mut gems {
        if recover_loot(&level, &mut gem.pos) {
            gem.vel = Vec2::ZERO;
        }
    }
    for mut coin in &mut gold {
        if recover_loot(&level, &mut coin.pos) {
            coin.vel = Vec2::ZERO;
        }
    }
    for mut pickup in &mut pickups {
        recover_loot(&level, &mut pickup.pos);
    }
    if fell {
        sfx.0.push(Sfx::TrapTrigger);
    }
}

fn recover_enemy(level: &Level, enemy: &mut Enemy, previous: Option<Vec2>) -> bool {
    let alive = enemy.alive();
    if alive && (enemy.named || !is_ground(enemy.kind)) {
        return false;
    }
    let class = SizeClass::of_enemy(enemy.kind);
    let Some(safe) = recover(level, enemy.pos, previous, enemy.radius, class)
        .or_else(|| (!alive).then(|| recover(level, enemy.pos, previous, 0.0, SizeClass::Small)).flatten())
    else {
        return false;
    };
    // Reap reads position after this pass, so a swimming body that died over
    // water has its drops paid directly onto sand.
    enemy.pos = safe;
    enemy.vel = Vec2::ZERO;
    if alive {
        enemy.hp -= enemy.max_hp * FALL_ENEMY_FRACTION;
        enemy.note_damage();
        enemy.stun = enemy.stun.max(FALL_ENEMY_STUN);
        enemy.contact_cd = enemy.contact_cd.max(FALL_ENEMY_STUN);
    }
    alive
}

fn recover_loot(level: &Level, pos: &mut Vec2) -> bool {
    if level.pit_at(*pos).is_some() {
        if let Some(safe) = recover(level, *pos, None, 0.0, SizeClass::Small) {
            *pos = safe;
            return true;
        }
    }
    false
}

/// A centred legal landing on the entry side wins over the opposite bank.
/// Within that side, closest centre then row/column order decides.
fn recover(
    level: &Level,
    from: Vec2,
    previous: Option<Vec2>,
    radius: f32,
    class: SizeClass,
) -> Option<Vec2> {
    let (cx, cy) = cell_of(from);
    let side = previous
        .filter(|&p| outfall_sand_clear(level, p, radius, class))
        .map_or(Vec2::ZERO, |p| p - from);
    let mut best = None;
    for y in cy - RECOVERY_CELLS..=cy + RECOVERY_CELLS {
        for x in cx - RECOVERY_CELLS..=cx + RECOVERY_CELLS {
            let at = TileMap::tile_rect(x, y).center();
            let delta = at - from;
            let against_entry = delta.dot(side) < 0.0;
            let score = (against_entry, delta.len_sq());
            if best.is_none_or(|(s, _)| score < s)
                && outfall_sand_clear(level, at, radius, class)
                && level.tiles.segment_hit(from, at, 0.0).is_none()
            {
                best = Some((score, at));
            }
        }
    }
    best.map(|(_, at)| at)
}

#[cfg(test)]
mod tests {
    use rand::RngExt;

    use super::*;
    use crate::game::hazards::{LureTrap, SurgePhase};
    use crate::game::level::{LightFixture, LightFixtureKind, Pit, PitKind, TileRect};
    use crate::game::world::World;

    fn patch() -> World {
        let mut world = World::new(Theme::Sewer, 41);
        world.level = Level::empty(Theme::Outfall);
        world.player.pos = Vec2::new(300.0, 300.0);
        world.ecs.init_resource::<OutfallOrigins>();
        world
    }

    fn motion(world: &mut World, elapsed: f32, dt: f32) {
        world.elapsed = elapsed;
        world.begin_tick(dt, Vec2::ZERO, false);
        world.ecs.run_system_cached(outfall_motion_system).unwrap();
        world.end_tick();
    }

    fn pit() -> Pit {
        Pit::new(TileRect::new(10, 10, 1, 3), PitKind::TidalDrop)
    }

    #[test]
    fn surge_uses_one_compass_and_the_whole_active_slice() {
        let first = outfall_surge(23, 8.0);
        assert_eq!(first.phase, SurgePhase::Warning);
        assert_eq!(outfall_surge(23, 9.0).phase, SurgePhase::Active);
        assert_eq!(outfall_surge(23, 9.6).phase, SurgePhase::Quiet);
        assert_eq!(outfall_surge(23, 17.0), first);
        let slice = active_slice(8.98, 9.03) + active_slice(9.03, 9.58) + active_slice(9.58, 9.64);
        assert!((slice * SURGE_DISTANCE / SURGE_ACTIVE_SECS - 20.0).abs() < 1e-5);
    }

    #[test]
    fn displacement_stops_before_the_swept_pit_and_wall() {
        use crate::game::art_walls::WallKind;
        use crate::game::collision::TileKind;

        let mut level = Level::empty(Theme::Outfall);
        let from = Vec2::new(141.0, 168.0);
        let step = Vec2::new(60.0, 0.0);
        level.objects.tidal_pits.push(pit());
        for radius in [0.0, 3.0, 12.0] {
            let end = clamp_outfall_displacement(&level, from, step, radius, SizeClass::Small);
            assert!((end.x - (159.99 - radius)).abs() < 0.02, "{end:?}");
        }
        level.objects.tidal_pits.clear();
        level.tiles.set(10, 10, TileKind::Wall(WallKind::Solid));
        let end = clamp_outfall_displacement(&level, from, step, 3.0, SizeClass::Small);
        assert!(end.x > from.x && end.x < 157.0, "{end:?}");
    }

    #[test]
    fn director_spawn_rejects_sand_centres_that_overlap_a_pit() {
        use super::super::spawn::spawn_point_ok;

        let mut level = Level::empty(Theme::Outfall);
        level.objects.tidal_pits.push(pit());
        let centre = Vec2::new(158.0, 168.0);
        assert!(level.pit_at(centre).is_none());
        assert!(!spawn_point_ok(
            centre, Vec2::ZERO, level.arena, 0.0, 4.0, &level, level.start,
        ));
        assert!(spawn_point_ok(
            centre - Vec2::new(6.0, 0.0), Vec2::ZERO, level.arena,
            0.0, 4.0, &level, level.start,
        ));
    }

    #[test]
    fn surge_exempts_dashes_heavies_named_and_elites_and_halves_giant() {
        let mut world = patch();
        let from = world.player.pos;
        world.player.buffs.giant = 20.0;
        let bodies = [
            (EnemyKind::Conger, false, false),
            (EnemyKind::Conger, true, false),
            (EnemyKind::Conger, false, true),
            (EnemyKind::Grouper, false, false),
        ].map(|(kind, named, elite)| {
            let entity = world.spawn_enemy(kind, from, 1.0);
            let mut enemy = world.ecs.get_mut::<Enemy>(entity).unwrap();
            enemy.named = named;
            enemy.elite = elite;
            entity
        });
        motion(&mut world, 9.61, 0.65);
        assert!((world.player.pos.dist(from) - 10.0).abs() < 0.001);
        let [ordinary, other @ ..] = bodies;
        assert!((world.ecs.get::<Enemy>(ordinary).unwrap().pos.dist(from) - 20.0).abs() < 0.001);
        for entity in other {
            assert_eq!(world.ecs.get::<Enemy>(entity).unwrap().pos, from);
        }
        world.player.dash_active = 0.1;
        let before = world.player.pos;
        motion(&mut world, 18.61, 0.65);
        assert_eq!(world.player.pos, before);
    }

    #[test]
    fn falls_return_to_entry_and_only_ground_bodies_take_damage() {
        use crate::game::gem::PickupKind;

        let mut world = patch();
        world.level.objects.tidal_pits.push(pit());
        let sand = Vec2::new(152.0, 168.0);
        let water = Vec2::new(164.0, 168.0);
        world.player.pos = sand;
        let hp = world.player.hp;
        let entities = [EnemyKind::Conger, EnemyKind::Lampfish, EnemyKind::GlassShrimp]
            .map(|kind| world.spawn_enemy(kind, sand, 1.0));
        let mut named = Enemy::spawn_with_phases(EnemyKind::Conger, sand, 1.0, 0.0, 0.0, 0.0);
        named.named = true;
        let named = world.ecs.spawn(named).id();
        let gem = world.ecs.spawn(Gem::new(water, 1)).id();
        let coin = world.ecs.spawn(Gold::new(water, 1)).id();
        let pickup = world.ecs.spawn(Pickup { pos: water, kind: PickupKind::Heal, bob: 0.0 }).id();

        world.begin_tick(0.05, Vec2::ZERO, false);
        world.ecs.resource_mut::<Player>().pos = water;
        world.ecs.run_system_cached(outfall_motion_system).unwrap();
        for entity in entities.into_iter().chain([named]) {
            world.ecs.get_mut::<Enemy>(entity).unwrap().pos = water;
        }
        world.ecs.run_system_cached(outfall_falls_system).unwrap();
        world.end_tick();

        assert_eq!(world.player.pos, sand);
        assert_eq!(world.player.hp, hp - FALL_PLAYER_DAMAGE);
        assert!(world.player.iframes >= FALL_PLAYER_IFRAMES);
        let ground = world.ecs.get::<Enemy>(entities[0]).unwrap();
        assert_eq!(ground.pos, sand);
        assert_eq!(ground.hp, ground.max_hp - ground.max_hp * FALL_ENEMY_FRACTION);
        assert_eq!(ground.stun, FALL_ENEMY_STUN);
        for entity in [entities[1], entities[2], named] {
            let e = world.ecs.get::<Enemy>(entity).unwrap();
            assert_eq!(e.pos, water);
            assert_eq!(e.hp, e.max_hp);
        }
        assert_eq!(world.ecs.get::<Gem>(gem).unwrap().pos, sand);
        assert_eq!(world.ecs.get::<Gold>(coin).unwrap().pos, sand);
        assert_eq!(world.ecs.get::<Pickup>(pickup).unwrap().pos, sand);
    }

    #[test]
    fn armed_lures_stay_single_use_and_draw_no_run_rng() {
        let mut world = patch();
        let pos = world.player.pos;
        let at = pos + Vec2::new(24.0, 0.0);
        world.level.objects.lure_traps.push(LureTrap::new(pos, vec![at]));
        let mut expected_rng = patch().rng;
        motion(&mut world, 1.0, 0.01);
        assert_eq!(world.level.objects.lure_traps[0].phase, LurePhase::Warning);
        // An already armed trap still resolves after the arrival gate.
        world.director.boss_spawned = true;
        world.level.objects.lure_traps.push(LureTrap::new(pos, vec![at]));
        motion(&mut world, 1.8, 0.8);
        assert_eq!(world.level.objects.lure_traps[0].phase, LurePhase::Spent);
        assert_eq!(world.level.objects.lure_traps[1].phase, LurePhase::Ready);
        assert_eq!(world.enemy_count(), 1);
        let enemy = world.enemies().remove(0);
        assert_eq!(enemy.kind, EnemyKind::Conger);
        assert_eq!(enemy.pos, at);
        motion(&mut world, 2.6, 0.8);
        assert_eq!(world.enemy_count(), 1);
        assert_eq!(world.rng.random::<u64>(), expected_rng.random::<u64>());
    }

    #[test]
    fn blocked_lure_spends_with_failed_pulse() {
        let mut world = patch();
        let pos = world.player.pos;
        let at = pos + Vec2::new(24.0, 0.0);
        world.level.objects.lure_traps.push(LureTrap::new(pos, vec![at]));
        motion(&mut world, 1.0, 0.01);
        world.player.pos = at;
        motion(&mut world, 1.8, 0.8);
        let trap = &world.level.objects.lure_traps[0];
        assert_eq!(trap.phase, LurePhase::Spent);
        assert!(trap.failed);
        assert!(!trap.is_lit());
        assert_eq!(world.enemy_count(), 0);
    }

    #[test]
    fn sconce_queries_share_spent_state_and_fixture_order() {
        let mut level = Level::empty(Theme::Outfall);
        let center = Vec2::new(100.0, 100.0);
        for (dx, seed) in [(12.0, 8), (-24.0, 1), (24.0, 3)] {
            level.objects.lights.push(LightFixture {
                pos: center + Vec2::new(dx, 0.0),
                kind: LightFixtureKind::Sconce,
                radius: 110.0,
                seed,
            });
        }
        assert_eq!(level.nearest_live_sconce(center, 24.0).unwrap().0, 1);
        level.objects.outfall_elite_snuff = Some(1);
        assert_eq!(level.nearest_live_sconce(center, 24.0).unwrap().0, 2);
        level.objects.outfall_boss_snuff = Some(2);
        assert!(level.nearest_live_sconce(center, 24.0).is_none());
        level.objects.outfall_elite_snuff = None;
        assert_eq!(level.nearest_live_sconce(center, 24.0).unwrap().0, 1);
    }
}
