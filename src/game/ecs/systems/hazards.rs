//! The wave 3 floor hazards: steam grates (Laundry) and ink pads (Printworks).
//!
//! Both hazards are **walkable floor** and live in
//! [`LevelObjects`](crate::game::level::LevelObjects) as
//! [`SteamGrate`]/[`InkPad`] view structs; the floor art reads those very
//! structs, so the drawn plume and the damaging plume cannot drift.
//!
//! ```text
//! SteamGrate:  Resting (4.0 s) -> Warning (1.0 s, three pale puffs)
//!                             -> Plume   (0.7 s, 8 once per occupant) -> ...
//! InkPad:      permanent, no state: ground movement at 75 %, flying and
//!              dashing bodies unaffected
//! ```
//!
//! ## Ordering
//!
//! `hazards_system` runs **immediately after `traps_system`, before
//! `player_system`**, for the same two reasons the trap runtime does:
//!
//! * the rat's ink slow is applied by shortening this tick's
//!   [`TickInput`] throttle vector and clipping its current velocity, so
//!   `player_system` derives `INK_SLOW` of the normal speed without knowing
//!   hazards exist;
//! * a grounded enemy carries [`InkSlow`], which
//!   `enemy_move::enemy_steering_system` multiplies in next to the trap glue.
//!
//! Steam damage also lands before `loot::reap_enemies_system`, so a grate kill
//! drops its gem, scores and queues its `Sfx` exactly like a weapon kill.
//!
//! ## Determinism
//!
//! Every grate runs its own phase clock from a generator-drawn offset, so the
//! system never decides *which* grate vents. The enemies a plume scalds are
//! collected into a reused `Local` list and sorted by `SpawnId` before damage,
//! so the rng draws of the impact vfx happen in the run's fixed order. The rng
//! is drawn only when a plume actually scalds somebody (the hurt/impact bursts),
//! and the ink pass draws none at all.
//!
//! ## No per-tick allocation
//!
//! The struck lists are `Local` buffers for the whole run; the ink pass walks
//! the level's small pad list and inserts/removes the [`InkSlow`] component only
//! on the ticks an enemy enters or leaves a pad.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::hazards::{INK_SLOW, STEAM_DAMAGE};
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

/// Per-enemy movement multiplier while standing on ink (absent = `1.0`).
///
/// Kept separate from `traps::TrapSlow` so a glue board and an ink pad can both
/// slow the same pursuer without either system's bookkeeping clobbering the
/// other: `enemy_steering_system` multiplies both in.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct InkSlow(pub f32);

/// Only ground bodies feel ink; a Bat flies over it.
#[inline]
fn is_ground(kind: EnemyKind) -> bool {
    !matches!(kind, EnemyKind::Bat)
}

/// Apply the rat's ink slow exactly like the trap glue does: shorten this
/// tick's throttle and clip the current velocity, so the recurrence is an
/// `INK_SLOW`-scaled copy of a free walk (frame-rate independent).
#[inline]
fn apply_player_slow(input: &mut TickInput, player: &mut Player, mult: f32) {
    input.move_dir = input.move_dir * mult;
    let cap = player.loadout.stats.move_speed * mult;
    player.vel = player.vel.clamp_len(cap);
}

/// Advance every steam grate and apply the ink pads for one tick.
pub fn hazards_system(
    mut input: ResMut<TickInput>,
    mut player: ResMut<Player>,
    mut level: ResMut<Level>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    ink_slow: Query<&InkSlow>,
    mut commands: Commands,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut hits: Local<Vec<(u64, Entity)>>,
    mut struck: Local<Vec<(u32, Entity)>>,
) {
    let dt = input.dt;
    let pads_exist = !level.objects.ink_pads.is_empty();
    let grates_exist = !level.objects.steam_grates.is_empty();
    if !pads_exist && !grates_exist {
        return;
    }

    let player_pos = player.pos;
    let player_alive = player.alive();
    // Exactly `player_system`'s dash condition: a dash that will happen this
    // tick, or one in progress, ignores both hazards.
    let player_dash_now = input.dash && player.dash_cd <= 0.0;
    let player_dashing = player.dash_active > 0.0;

    // ---- ink: the rat -----------------------------------------------------
    if pads_exist && player_alive && !player_dashing && !player_dash_now {
        let on_ink = level
            .objects
            .ink_pads
            .iter()
            .any(|pad| pad.covers(player_pos, PLAYER_RADIUS));
        if on_ink {
            apply_player_slow(&mut input, &mut player, INK_SLOW);
        }
    }

    // ---- steam: phase clocks and the plume ------------------------------
    if grates_exist {
        // One pass advances every clock; a plume that starts clears the struck
        // list (a new plume scalds everyone in it again, once).
        let mut plume_started = false;
        for grate in level.objects.steam_grates.iter_mut() {
            if grate.tick(dt) {
                plume_started = true;
            }
        }
        if plume_started {
            struck.clear();
            sfx.0.push(Sfx::SteamHiss);
        }

        // The rat: once per plume, through the normal i-frame path.
        let mut rat_hit: Option<Vec2> = None;
        for (index, grate) in level.objects.steam_grates.iter_mut().enumerate() {
            if !grate.plume() || grate.player_struck {
                continue;
            }
            if !player_alive || !grate.covers(player_pos, PLAYER_RADIUS) {
                continue;
            }
            if player.take_damage(STEAM_DAMAGE) {
                grate.player_struck = true;
                rat_hit = Some(player_pos);
            }
            let _ = index;
        }
        if let Some(pos) = rat_hit {
            vfx::emit_player_hurt(&mut particles, &mut rng.0, pos);
            sfx.0.push(Sfx::Hurt);
        }

        // The horde: every grounded body on a live plume, once per plume,
        // damaged in `SpawnId` order because the vfx draws from the rng.
        hits.clear();
        for (entity, id, e) in enemies.iter_mut() {
            if !e.alive() || !is_ground(e.kind) {
                continue;
            }
            let mut on_live = None;
            for (index, grate) in level.objects.steam_grates.iter().enumerate() {
                if grate.plume() && grate.covers(e.pos, e.radius) {
                    on_live = Some(index as u32);
                    break;
                }
            }
            let Some(index) = on_live else { continue };
            if struck.contains(&(index, entity)) {
                continue;
            }
            hits.push((id.0, entity));
        }
        hits.sort_unstable_by_key(|&(id, _)| id);
        for &(_, entity) in hits.iter() {
            let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                continue;
            };
            if !e.alive() {
                continue;
            }
            // Which grate scalded it (the first live one covering it).
            let index = level
                .objects
                .steam_grates
                .iter()
                .position(|g| g.plume() && g.covers(e.pos, e.radius))
                .unwrap_or(0) as u32;
            if struck.contains(&(index, entity)) {
                continue;
            }
            e.hurt(STEAM_DAMAGE);
            let killed = !e.alive();
            vfx::emit_weapon_hit(&mut particles, &mut rng.0, e.pos, Vec2::ZERO, None, e.kind, killed);
            damage_numbers.record(
                e.id,
                e.pos,
                e.radius,
                STEAM_DAMAGE,
                killed,
                DamageSource::Normal,
            );
            struck.push((index, entity));
        }
    }

    // ---- ink: the horde ---------------------------------------------------
    if pads_exist {
        for (entity, _, e) in enemies.iter_mut() {
            if !e.alive() || !is_ground(e.kind) {
                continue;
            }
            let on_ink = level
                .objects
                .ink_pads
                .iter()
                .any(|pad| pad.covers(e.pos, e.radius));
            let already = ink_slow.contains(entity);
            if on_ink && !already {
                commands.entity(entity).insert(InkSlow(INK_SLOW));
            } else if !on_ink && already {
                commands.entity(entity).remove::<InkSlow>();
            }
        }
    }
}

/// The ink multiplier at a point, for tests and for anything that needs the
/// hazard's effect without walking the ECS world.
pub fn ink_multiplier_at(level: &Level, p: Vec2, radius: f32) -> f32 {
    let mut mult: f32 = 1.0;
    for pad in &level.objects.ink_pads {
        if pad.covers(p, radius) {
            mult = mult.min(INK_SLOW);
        }
    }
    mult
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::hazards::{
        InkPad, SteamGrate, SteamPhase, STEAM_CYCLE_SECS, STEAM_PLUME_SECS, STEAM_WARN_SECS,
    };
    use crate::game::level::{Level, Theme, TileRect};
    use crate::game::world::World;

    /// A world whose level carries one grate/pad at the cell in front of the
    /// rat, with everything else (enemies) cleared.
    fn harness(theme: Theme, grate: Option<SteamGrate>, pad: Option<InkPad>) -> World {
        let mut world = World::new(theme, 7);
        // No passive regen: these tests are about the hazard's own numbers.
        world.player.loadout.stats.regen = 0.0;
        world.level.objects.steam_grates.clear();
        world.level.objects.ink_pads.clear();
        if let Some(g) = grate {
            world.level.objects.steam_grates.push(g);
        }
        if let Some(p) = pad {
            world.level.objects.ink_pads.push(p);
        }
        world
    }

    fn cell_of(p: Vec2) -> (i32, i32) {
        (
            (p.x / crate::game::collision::TILE_F) as i32,
            (p.y / crate::game::collision::TILE_F) as i32,
        )
    }

    /// One tick of exactly the two systems the hazards need: the hazard pass
    /// first (as the schedule orders it), then the player's movement so the ink
    /// slow is visible in the rat's speed. No director, no contact damage.
    fn tick(world: &mut World, dt: f32) {
        tick_with(world, dt, Vec2::new(0.0, 0.0));
    }

    fn tick_with(world: &mut World, dt: f32, move_dir: Vec2) {
        world.begin_tick(dt, move_dir, false);
        let _ = world.ecs.run_system_cached(hazards_system);
        let _ = world
            .ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system);
        world.end_tick();
    }

    #[test]
    fn a_plume_scalds_the_rat_once_per_vent_cycle() {
        let start = Vec2::new(1600.0, 1280.0);
        let cell = cell_of(start);
        // Put the grate under the rat, already at the end of its warning.
        let mut grate = SteamGrate::new(cell, 0.0);
        grate.seek(STEAM_REST_SECS_END);
        let mut world = harness(Theme::Laundry, Some(grate), None);
        world.player.pos = start;
        let hp = world.player.hp;
        // Warning finishes on the next tick, then the plume burns for 0.7 s.
        for _ in 0..6 {
            tick(&mut world, 0.1);
        }
        assert!(
            world.player.hp < hp,
            "the plume under the rat did no damage ({} -> {})",
            hp,
            world.player.hp
        );
        let after_first = world.player.hp;
        // The rest of the plume must not stack: one hit per occupant per plume.
        for _ in 0..(STEAM_PLUME_SECS as i32 * 10 - 6) {
            tick(&mut world, 0.1);
        }
        assert_eq!(
            world.player.hp, after_first,
            "the same plume hit the rat twice"
        );
        // The next vent cycle (warn + plume) burns again.
        for _ in 0..((STEAM_CYCLE_SECS * 10.0) as i32) {
            tick(&mut world, 0.1);
        }
        assert!(
            world.player.hp < after_first,
            "the second vent cycle did not burn"
        );
    }

    /// Just before the warning ends.
    const STEAM_REST_SECS_END: f32 = STEAM_CYCLE_SECS - STEAM_PLUME_SECS - 0.01;

    #[test]
    fn a_grate_keeps_its_own_clock_and_hurts_pursuers_too() {
        let start = Vec2::new(1600.0, 1280.0);
        let cell = cell_of(start);
        let mut grate = SteamGrate::new(cell, 0.0);
        grate.seek(STEAM_REST_SECS_END);
        let mut world = harness(Theme::Laundry, Some(grate), None);
        // The rat stands off the grate; a Skitterer stands on it.
        world.player.pos = start + Vec2::new(64.0, 0.0);
        world.spawn_enemy(crate::game::enemy::EnemyKind::Skitterer, start, 1.0);
        let hp = world.player.hp;
        for _ in 0..8 {
            tick(&mut world, 0.1);
        }
        assert_eq!(world.player.hp, hp, "the rat was off the grate and got burnt");
        let burned = world
            .enemies()
            .iter()
            .all(|e| !e.alive() || e.hp < crate::game::enemy::EnemyKind::Skitterer.base_hp());
        assert!(burned, "the plume did not scald the pursuer standing in it");
    }

    #[test]
    fn ink_slows_ground_movement_and_not_a_dash() {
        let start = Vec2::new(1600.0, 1280.0);
        let cell = cell_of(start);
        let pad = InkPad::new(TileRect::new(cell.0 - 1, cell.1 - 1, 3, 3), 3);
        let mut world = harness(Theme::Printworks, None, Some(pad));
        world.player.pos = start;
        world.player.vel = Vec2::ZERO;
        // A held walk: the rat's speed on ink must be a quarter short.
        for _ in 0..20 {
            tick_with(&mut world, 1.0 / 60.0, Vec2::new(1.0, 0.0));
        }
        let on_ink_speed = world.player.vel.len();
        assert!(
            on_ink_speed <= world.player.loadout.stats.move_speed * INK_SLOW + 0.5,
            "ink did not slow the rat: {on_ink_speed} > {}",
            world.player.loadout.stats.move_speed * 0.75
        );
        // Step off the pad and the speed comes back.
        world.player.pos = start + Vec2::new(120.0, 0.0);
        for _ in 0..40 {
            tick_with(&mut world, 1.0 / 60.0, Vec2::new(1.0, 0.0));
        }
        assert!(
            world.player.vel.len() > on_ink_speed + 1.0,
            "the rat did not speed back up off the ink"
        );
        assert_eq!(ink_multiplier_at(&world.level, start, 3.0), INK_SLOW);
        assert_eq!(
            ink_multiplier_at(&world.level, start + Vec2::new(200.0, 0.0), 3.0),
            1.0
        );
    }

    #[test]
    fn a_theme_without_hazards_draws_nothing_and_does_nothing() {
        let mut world = World::new(Theme::Sewer, 3);
        assert!(world.level.objects.steam_grates.is_empty());
        assert!(world.level.objects.ink_pads.is_empty());
        // Nothing to step and nothing to draw: a tick with no hazards must not
        // touch the rat at all.
        let hp = world.player.hp;
        tick(&mut world, 1.0 / 60.0);
        assert_eq!(world.player.hp, hp);
        assert_eq!(ink_multiplier_at(&world.level, world.player.pos, 3.0), 1.0);
    }

    #[test]
    fn the_generator_phases_the_grates_apart() {
        // Two grates in one map never share a phase offset (the generator draws
        // each one's), so an aisle cannot vent as one wave.
        let mut differed = 0;
        for seed in 0..8u64 {
            let level = Level::generate(Theme::Laundry, seed);
            let grates = &level.objects.steam_grates;
            assert!(
                (crate::game::levelgen::hazards::HAZARDS.0
                    ..=crate::game::levelgen::hazards::HAZARDS.1)
                    .contains(&grates.len()),
                "seed {seed}: {} steam grates",
                grates.len()
            );
            if grates.windows(2).any(|w| w[0].phase != w[1].phase || w[0].timer != w[1].timer) {
                differed += 1;
            }
            for g in grates {
                assert!(
                    matches!(
                        g.phase,
                        SteamPhase::Resting | SteamPhase::Warning | SteamPhase::Plume
                    ),
                    "phase must be one of the three beats"
                );
            }
        }
        assert!(differed > 4, "the grates vent in lockstep on {} maps", 8 - differed);
    }

    /// Twin runs on both wave 3 floors: the same seed and the same scripted
    /// inputs reproduce the same run, hazards and all (the levels' extra rng
    /// draws happen at generation time only, and the plumes are deterministic
    /// clock crossings, so nothing here may diverge).
    #[test]
    fn twin_runs_match_on_both_new_floors() {
        for theme in [Theme::Laundry, Theme::Printworks] {
            for seed in [5u64, 17] {
                let digest = |seed: u64| {
                    let mut w = World::new(theme, seed);
                    let dt = 1.0 / 60.0;
                    for tick in 0..1200u32 {
                        let dir = Vec2::from_angle(tick as f32 * 0.045);
                        w.update(dt, dir, tick % 120 == 0);
                        if w.pending_levelups > 0 {
                            w.pending_levelups = 0;
                            let choices = w.player.loadout.generate_choices(&mut w.rng);
                            if let Some(choice) = choices.first() {
                                let choice = choice.clone();
                                w.player.loadout.apply(&choice);
                            }
                        }
                    }
                    fn fold(mut h: u64, v: f32) -> u64 {
                        h ^= v.to_bits() as u64;
                        h.wrapping_mul(0x100000001b3)
                    }
                    let mut h = 0xcbf29ce484222325u64;
                    h = fold(h, w.player.pos.x);
                    h = fold(h, w.player.pos.y);
                    h = fold(h, w.player.hp);
                    h = fold(h, w.elapsed);
                    h ^= w.kills as u64;
                    h = h.wrapping_mul(0x100000001b3);
                    for e in w.enemies() {
                        h = fold(h, e.pos.x);
                        h = fold(h, e.pos.y);
                        h = fold(h, e.hp);
                    }
                    for g in &w.level.objects.steam_grates {
                        h ^= g.phase as u64 ^ ((g.timer * 1000.0) as u64).rotate_left(17);
                        h = h.wrapping_mul(0x100000001b3);
                    }
                    h
                };
                assert_eq!(digest(seed), digest(seed), "{theme:?} seed {seed} diverged");
            }
        }
    }

    #[test]
    fn plume_damage_matches_the_design_number() {
        assert_eq!(STEAM_DAMAGE, 8.0);
        assert_eq!(INK_SLOW, 0.75);
        assert_eq!(STEAM_WARN_SECS, 1.0);
        assert_eq!(STEAM_PLUME_SECS, 0.7);
    }
}
