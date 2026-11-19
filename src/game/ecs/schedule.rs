//! The single-threaded, explicitly ordered tick schedule.
//!
//! `bevy_ecs` is built with `default-features = false, features = ["std"]`, so
//! `multi_threaded` is off and the schedule runs on `SingleThreadedExecutor`.
//! Every system is chained, so the execution order is exactly the legacy
//! `World::update` order and no query iteration may choose an order that
//! affects the sim (components carry `SpawnId`; see [`super::components`]).
//!
//! Legacy `update()` order map:
//!
//! | # | system | legacy step |
//! |---|--------|-------------|
//! | 1 | `player::player_system` | `update_player` |
//! | 2 | `fog::reveal_system` | fog of war (minimap; new, writes only `Fog`) |
//! | 3 | `enemy_move::flow_refresh_system` | flow refresh |
//! | 4 | `spawn::director_system` | `run_director` |
//! | 5 | `enemy_move::enemy_steering_system` | `update_enemies` steer |
//! | 6 | `enemy_move::enemy_fire_system` | `update_enemies` fire |
//! | 7 | `enemy_move::grid_build_system` | first grid build + `max_radius` |
//! | 8 | `enemy_move::separation_system` | `separate_enemies` |
//! | 9 | `enemy_move::wall_resolve_system` | `resolve_enemy_walls` |
//! | 10 | `enemy_move::grid_refresh_system` | second grid build |
//! | 11 | `weapons::fire_weapons_system` | `fire_weapons` |
//! | 12 | `projectiles::bullets_system` | `update_bullets` |
//! | 13 | `projectiles::pools_system` | `update_pools` |
//! | 14 | `combat::player_bullet_hits_system` | `resolve_player_bullets` |
//! | 15 | `combat::enemy_bullet_hits_system` | `resolve_enemy_hits` |
//! | 16 | `combat::contact_system` | `resolve_contact` |
//! | 17 | `loot::reap_enemies_system` | `reap_enemies` |
//! | 18 | `loot::gems_system` | `update_gems` |
//! | 19 | `loot::pickups_system` | `update_pickups` |
//! | 20 | `loot::nuke_reap_system` | the nested re-reap after a Nuke |
//! | 21 | `fx::particles_system` | `particles.update(dt)` |
//! | 22 | `damage_numbers_system` | `damage_numbers.update(dt)` (neon merge) |
//! | 23 | `fx::shake_system` | shake decay + jitter |
//! | 24 | `loot::outcome_system` | the win/loss block |
//!
//! `fog::reveal_system` is the only system with no legacy step: it reads the
//! player position and the tile grid, writes the `Fog` resource and nothing
//! else (no rng draw, no entity, no simulated value), so the golden traces are
//! identical with it enabled or disabled.
//!
//! The chain is split into two tuples because `IntoScheduleConfigs` is
//! implemented for tuples up to 20 elements. Each tuple is `.chain()`ed, so
//! Bevy inserts `ApplyDeferred` between consecutive systems whose predecessor
//! uses `Commands`. The second group is explicitly `.after(contact_system)`;
//! it starts with reaping, so the reaped→gem ordering is itself chained and
//! deferred spawns are visible to `gems_system`.
//!
//! `elapsed += dt` is applied by `World::update` before
//! `begin_tick`; the per-tick `NukeTick` reset happens in `World::begin_tick`,
//!
//! The chain is split into two tuples because `IntoScheduleConfigs` is
//! implemented for tuples up to 20 elements. Each tuple is `.chain()`ed, so
//! Bevy inserts `ApplyDeferred` between consecutive systems whose predecessor
//! uses `Commands`. The second group is explicitly `.after(contact_system)`;
//! it starts with reaping, so the reaped→gem ordering is itself chained and
//! deferred spawns are visible to `gems_system`.
//!
//! `settle_schedule` is the legacy `finished()` early-return path (lines
//! 118-122): only particles advance, nothing else in the sim runs.

use bevy_ecs::prelude::*;

use super::resources::TickInput;
use super::systems::{combat, enemy_move, fog, fx, loot, player, projectiles, spawn, weapons};

/// Legacy `self.damage_numbers.update(dt)` (neon merge, `World::update`).
///
/// Runs immediately after `fx::particles_system`, exactly where the legacy
/// `update` advanced the floating numbers next to the particles. It lives here
/// (rather than in `fx.rs`) because the schedule is the file that owns the
/// tick order; the body is a one-line delegation.
fn damage_numbers_system(
    input: Res<TickInput>,
    mut numbers: ResMut<crate::game::particle::damage_number::DamageNumbers>,
) {
    numbers.update(input.dt);
}

/// Build the full per-tick schedule (order documented above).
pub fn build_tick_schedule() -> Schedule {
    let mut schedule = Schedule::default();

    // 1-15: everything up to and including contact damage.
    schedule.add_systems(
        (
            player::player_system,
            fog::reveal_system,
            enemy_move::flow_refresh_system,
            spawn::director_system,
            enemy_move::enemy_steering_system,
            enemy_move::enemy_fire_system,
            enemy_move::grid_build_system,
            enemy_move::separation_system,
            enemy_move::wall_resolve_system,
            enemy_move::grid_refresh_system,
            weapons::fire_weapons_system,
            projectiles::bullets_system,
            projectiles::pools_system,
            combat::player_bullet_hits_system,
            combat::enemy_bullet_hits_system,
            combat::contact_system,
        )
            .chain(),
    );

    // 16-22: reaping through the outcome. Ordered after contact; internally
    // chained so the Nuke re-reap and every deferred drop stay in order.
    schedule.add_systems(
        (
            loot::reap_enemies_system,
            loot::gems_system,
            loot::pickups_system,
            loot::nuke_reap_system,
            fx::particles_system,
            damage_numbers_system,
            fx::shake_system,
            loot::outcome_system,
        )
            .chain()
            .after(combat::contact_system),
    );

    schedule
}

/// Legacy `finished()` early-return: `self.particles.update(dt)` plus the
/// damage-number settle the neon merge added next to it.
pub fn build_settle_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems((fx::particles_system, damage_numbers_system).chain());
    schedule
}
