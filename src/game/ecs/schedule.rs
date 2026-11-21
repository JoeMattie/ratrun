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
//! | -1 | `powerups::powerups_system` | wave-3 pocket-find timers (new) |
//! | 1 | `player::player_system` | `update_player` |
//! | 2 | `fog::reveal_system` | fog of war (minimap; new, writes only `Fog`) |
//! | 3 | `enemy_move::flow_refresh_system` | flow refresh |
//! | 4 | `spawn::director_system` | `run_director` |
//! | 4b | `boss::boss_spawn_system` | boss hand-off (reads `BossDue`) |
//! | 5 | `enemy_move::enemy_steering_system` | `update_enemies` steer |
//! | 6 | `enemy_move::enemy_fire_system` | `update_enemies` fire |
//! | 7 | `enemy_move::grid_build_system` | first grid build + `max_radius` |
//! | 8 | `enemy_move::separation_system` | `separate_enemies` |
//! | 9 | `enemy_move::wall_resolve_system` | `resolve_enemy_walls` |
//! | 10 | `enemy_move::grid_refresh_system` | second grid build |
//! | 11 | `weapons::fire_weapons_system` | `fire_weapons` |
//! | 11b | `weapons::active_weapons_system` | the two Space-triggered weapons (new) |
//! | 11c | `weapons_wave3b::{laser_eyes,super_dash,melee_nudge}_system` | wave 3's self-driving weapons (new) |
//! | 11d | `weapons_wave3a::acid_rain_system` | Acid Rain's locked drops landing (wave 3) |
//! | 11e | `weapons_wave3a::hantavirus_system` | Hantavirus ticks + third-tick jumps (wave 3) |
//! | 12 | `projectiles::bullets_system` | `update_bullets` |
//! | 13 | `projectiles::pools_system` | `update_pools` |
//! | 13b | `boss::boss_damage_system` | player damage → boss hit shapes |
//! | 13c | `boss::boss_update_system` | boss state machine + hazards |
//! | 14 | `combat::player_bullet_hits_system` | `resolve_player_bullets` |
//! | 15 | `combat::enemy_bullet_hits_system` | `resolve_enemy_hits` |
//! | 16 | `combat::contact_system` | `resolve_contact` |
//! | 16b | `weapons_wave3a::hantavirus_death_system` | death-triggered jumps, before the reap |
//! | 17 | `loot::reap_enemies_system` | `reap_enemies` |
//! | 18 | `loot::gems_system` | `update_gems` |
//! | 18b | `loot::treasure_system` | gold and chests |
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
//! `gates::passage_system`, `gates::cache_system` and `gates::door_system`
//! (the interactive-object runtime: rat-hole states and centring, the maze
//! cache heist, the one-way door leaves and the exit slam) are new in the same
//! sense. They are chained after `fog::reveal_system` and before
//! `enemy_move::flow_refresh_system` so they read the rat's post-move position
//! and write only `Level::objects`, `player.iframes`/`pending_levelups`,
//! `Fog::maze_claimed` and the `GateRuntime` scratch.
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
use super::systems::{
    boss, combat, enemy_move, fog, fx, gates, loot, player, powerups, projectiles, spawn, traps,
    weapons, weapons_wave3a, weapons_wave3b,
};

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

    // 0: floor traps. First in the tick, because the sticky board's slow is
    // applied to this tick's `TickInput` throttle (before `player_system`
    // derives its movement from it) and to the enemies' `TrapSlow` component
    // (which `enemy_steering_system` multiplies in); damage dealt here also
    // lands before `loot::reap_enemies_system`, so trap kills take the normal
    // loot path. Trap state lives in `level.objects` (see `traps.rs`).
    schedule.add_systems(traps::traps_system.before(player::player_system));

    // -1: the wave-3 pocket finds' timers. Before the traps (Clean Socks must
    // already be aged when a glue board looks for a victim), before the player
    // (a Pocket Watch has to cover the whole tick) and before everything that
    // reads a buff. A run without one of these pickups returns immediately.
    schedule.add_systems(powerups::powerups_system.before(traps::traps_system));

    // 1-15: everything up to and including contact damage.
    schedule.add_systems(
        (
            player::player_system,
            fog::reveal_system,
            enemy_move::flow_refresh_system,
            spawn::director_system,
            boss::boss_spawn_system,
            enemy_move::enemy_steering_system,
            enemy_move::enemy_fire_system,
            enemy_move::grid_build_system,
            enemy_move::separation_system,
            enemy_move::wall_resolve_system,
            enemy_move::grid_refresh_system,
            weapons::fire_weapons_system,
            // 11b: the player-triggered weapons. Right after the auto-fire pass
            // and before the projectile sweep, so it sees the same cached
            // `EnemyIndex` order (nothing spawned or reaped since
            // `grid_refresh`), its bullet deletions are applied before
            // `bullets_system` moves them, and a sweep's damage lands before
            // `player_bullet_hits`/`reap_enemies` in the same tick.
            weapons::active_weapons_system,
        )
            .chain(),
    );

    // 11d-16: wave 3's Acid Rain and Hantavirus, then the projectile sweep
    // through contact. A third tuple only because `IntoScheduleConfigs` stops
    // at 20 entries; it is explicitly chained after `active_weapons_system`, so
    // the order is still the single chain documented above. The drops' splash
    // and the virus ticks land before `player_bullet_hits`/`reap_enemies`, and
    // both read the cached `EnemyIndex` order, which nothing has invalidated
    // since `grid_refresh`.
    schedule.add_systems(
        (
            weapons_wave3a::acid_rain_system,
            weapons_wave3a::hantavirus_system,
            projectiles::bullets_system,
            projectiles::pools_system,
            boss::boss_damage_system,
            boss::boss_update_system,
            combat::player_bullet_hits_system,
            combat::enemy_bullet_hits_system,
            combat::contact_system,
        )
            .chain()
            .after(weapons::active_weapons_system),
    );

    // 11c: wave 3's self-driving weapons (Laser Eyes' burst, Super Dash's dash
    // hit) and Counterweight's nudge clock. Chained right after the active
    // weapons and before `bullets_system`: the cached `EnemyIndex` order is
    // still valid there (nothing spawned or reaped since `grid_refresh`), the
    // damage lands before `player_bullet_hits`/`reap_enemies` in the same tick,
    // and both systems read this tick's player position. Explicitly before
    // 11d (`acid_rain_system`, the head of the next tuple) so the order of the
    // wave-3 weapon passes is fixed: 3b's three systems, then rain, then virus.
    schedule.add_systems(
        (
            weapons_wave3b::laser_eyes_system,
            weapons_wave3b::super_dash_system,
            weapons_wave3b::melee_nudge_system,
        )
            .chain()
            .after(weapons::active_weapons_system)
            .before(weapons_wave3a::acid_rain_system),
    );

    // The interactive-object runtime (gates.rs): rat-holes + centring, the
    // maze-cache heist, then the one-way doors and the exit slam. Chained
    // between the fog reveal and the flow refresh so all three see this tick's
    // player position and nothing downstream sees a stale cell.
    schedule.add_systems(
        (
            gates::passage_system,
            gates::cache_system,
            gates::door_system,
        )
            .chain()
            .after(fog::reveal_system)
            .before(enemy_move::flow_refresh_system),
    );

    // 16-22: reaping through the outcome. Ordered after contact; internally
    // chained so the Nuke re-reap and every deferred drop stay in order.
    schedule.add_systems(
        (
            // 16b: a Hantavirus host that died this tick (any source) passes
            // its remaining jumps on while the corpse and the `EnemyIndex`
            // order still exist — i.e. after every damage pass and before the
            // reap despawns it.
            weapons_wave3a::hantavirus_death_system,
            loot::reap_enemies_system,
            loot::gems_system,
            loot::treasure_system,
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
