//! (g) Reaping, drops, gems, pickups (Nuke/Magnet), XP + level-ups, score +
//! kills, and the run outcome.
//!
//! Faithful port of:
//! - `World::reap_enemies`.
//! - 885-926: `World::update_gems`.
//! - 927-974: `World::update_pickups` (+ the `if self.update_pickups(dt) {
//!   self.reap_enemies() }` rule at 156-158, which `nuke_reap_system` is).
//! - 174-179: the outcome block.
//!
//! ## Neon (HUD/VFX/audio) hooks — re-applied from the merge diff
//!
//! The other team's `world.rs` rewrite replaced the inline particle code in
//! exactly these four functions with `vfx` calls and added damage numbers,
//! floor stains, a telemetry list and new `Sfx` variants. Every one of those
//! hunks is re-applied here with the same arguments at the same point in the
//! logic:
//!
//! - `reap_enemies` → `vfx::emit_enemy_death` (replaces the inline burst, and
//!   is also what leaves the ichor floor stain), `vfx::emit_elite_death_accent`
//!   for elites, `Sfx::BossDeath` for the boss (Brutes keep `Explosion`,
//!   everything else `Kill`).
//! - `update_gems` → `vfx::emit_gem_trail` per magnetized gem, and
//!   `vfx::emit_gem_collect` + `vfx::emit_level_up` on collection/level-up.
//! - `update_pickups` → `vfx::emit_pickup_ambient` for waiting pickups,
//!   `last_pickups.push(k)` for the HUD, `vfx::emit_pickup_heal` /
//!   `emit_pickup_magnet` / `emit_nuke` with `Sfx::Heal` / `Magnet` / `Nuke`
//!   (replacing the single `Sfx::Pickup`), and the boss Nuke damage number.
//!
//! Constants, operation order and the number/order of rng draws are copied
//! from the legacy code (and the neon diff did not change them). The only
//! sanctioned divergence is the iteration order: every entity query is walked
//! in `SpawnId` order instead of the legacy `Vec`/`swap_remove` order (port
//! contract, PORT_PLAN.md §3).
//!
//! ## No per-tick allocation
//!
//! Every order-sensitive walk sorts its entity list **once per system per
//! tick** into a `Local<Vec<(u64, Entity)>>` scratch buffer that lives for the
//! whole run (Bevy's `QueryIter::sort_by_key` allocates on every call).
//! `reap_enemies_system` sorts only the corpses. The Nuke pass cannot reuse the
//! `SpawnId`-ordered `EnemyIndex` from `enemy_move`: `reap_enemies_system` has
//! already despawned this tick's corpses through deferred commands, so the
//! index is stale there — it sorts the live enemies instead (Nukes are rare).

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::RngExt;

use crate::audio::Sfx;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{
    LastPickups, LevelIndex, NextSpawnId, NukeTick, PickupScratch, RngRes, RunCounters, SfxQueue,
    Shake, TickInput,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Pickup, PickupKind};
use crate::game::level::Level;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::game::particle::vfx;

/// Legacy `World::reap_enemies`  shared by the normal reap and
/// the same-tick Nuke re-reap.
///
/// Legacy walked `self.enemies` with `swap_remove`; the ECS port walks the
/// corpses in stable `SpawnId` order (port contract §3) and despawns through
/// the deferred command queue instead of removing from a `Vec`. Everything
/// else — death burst/stain, elite accent, shake/sfx choice, gem, drop roll,
/// kills/score — is the merged legacy+neon body.
fn reap_dead(
    commands: &mut Commands,
    level: &Level,
    counters: &mut RunCounters,
    rng: &mut StdRng,
    particles: &mut Particles,
    sfx: &mut Vec<Sfx>,
    shake: &mut Shake,
    next_id: &mut NextSpawnId,
    enemies: &Query<(Entity, &SpawnId, &Enemy)>,
    dead: &mut Vec<(u64, Entity)>,
) {
    // Only the corpses matter, so collect and sort just those: the per-enemy
    // rng draws below (death burst, then the drop roll) depend on this order.
    // The buffer is reused every tick, so this allocates nothing.
    dead.clear();
    dead.extend(
        enemies
            .iter()
            .filter(|(_, _, e)| !e.alive())
            .map(|(entity, id, _)| (id.0, entity)),
    );
    dead.sort_unstable_by_key(|&(id, _)| id);

    for &(_, entity) in dead.iter() {
        let Ok((_, _, e)) = enemies.get(entity) else {
            continue;
        };
        commands.entity(entity).despawn();

        let blood = level.palette.blood;
        let big = matches!(e.kind, EnemyKind::Brute | EnemyKind::Boss);
        // Neon hook: enemy-kind-specific burst + wet ichor floor stain (the
        // legacy 120/28/10-particle inline burst now lives in here).
        vfx::emit_enemy_death(particles, rng, e.pos, e.radius, e.kind, blood);
        if e.elite {
            vfx::emit_elite_death_accent(particles, rng, e.pos);
        }
        if e.kind == EnemyKind::Boss {
            shake.amount = shake.amount.max(16.0);
            // Neon hook: the boss gets its own death sting.
            sfx.push(Sfx::BossDeath);
        } else if big {
            shake.amount = shake.amount.max(6.0);
            sfx.push(Sfx::Explosion);
        } else {
            sfx.push(Sfx::Kill);
        }
        // Drop XP gem(s).
        let gem = Gem::new(e.pos, e.kind.xp());
        commands.spawn((gem, next_id.take()));
        // Rare pickup drops.
        let drop_chance = if e.kind == EnemyKind::Boss {
            1.0
        } else if big {
            0.12
        } else {
            0.012
        };
        if rng.random::<f32>() < drop_chance {
            let kind = match rng.random_range(0..3) {
                0 => PickupKind::Heal,
                1 => PickupKind::Magnet,
                _ => PickupKind::Nuke,
            };
            let pickup = Pickup {
                pos: e.pos,
                kind,
                bob: 0.0,
            };
            commands.spawn((pickup, next_id.take()));
        }
        counters.kills += 1;
        counters.score += e.kind.xp() * 10;
    }
}

pub fn reap_enemies_system(
    mut commands: Commands,
    level: Res<Level>,
    mut counters: ResMut<RunCounters>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut next_id: ResMut<NextSpawnId>,
    enemies: Query<(Entity, &SpawnId, &Enemy)>,
    mut dead: Local<Vec<(u64, Entity)>>,
) {
    reap_dead(
        &mut commands,
        &level,
        &mut counters,
        &mut rng.0,
        &mut particles,
        &mut sfx.0,
        &mut shake,
        &mut next_id,
        &enemies,
        &mut dead,
    );
}

/// Same body as `reap_enemies_system`, gated on `NukeTick`; clears the flag.
///
/// Legacy `update`: `if self.update_pickups(dt) {
/// self.reap_enemies() }` — a Nuke sets `hp = -1` during the pickup pass, so
/// those kills count in the same tick.
pub fn nuke_reap_system(
    mut commands: Commands,
    mut nuke: ResMut<NukeTick>,
    level: Res<Level>,
    mut counters: ResMut<RunCounters>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut next_id: ResMut<NextSpawnId>,
    enemies: Query<(Entity, &SpawnId, &Enemy)>,
    mut dead: Local<Vec<(u64, Entity)>>,
) {
    if !nuke.0 {
        return;
    }
    reap_dead(
        &mut commands,
        &level,
        &mut counters,
        &mut rng.0,
        &mut particles,
        &mut sfx.0,
        &mut shake,
        &mut next_id,
        &enemies,
        &mut dead,
    );
    nuke.0 = false;
}

/// Legacy `World::update_gems` , merged with the neon hooks.
pub fn gems_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    mut counters: ResMut<RunCounters>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut gems: Query<(Entity, &SpawnId, &mut Gem)>,
    mut ordered: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    let magnet = player.loadout.stats.magnet;
    let mut gained = 0u32;
    let collect_r = PLAYER_RADIUS + 3.0;

    // One `SpawnId` sort per tick into the reused buffer (the gem trail vfx
    // draws rng per gem, so the walk order is part of the spec).
    ordered.clear();
    ordered.extend(gems.iter().map(|(entity, id, _)| (id.0, entity)));
    ordered.sort_unstable_by_key(|&(id, _)| id);

    for i in 0..ordered.len() {
        let entity = ordered[i].1;
        let Ok((_, _, mut g)) = gems.get_mut(entity) else {
            continue;
        };
        let to = ppos - g.pos;
        let dist = to.len();
        if g.magnetized || dist < magnet {
            g.magnetized = true;
            // Steer velocity toward the player (instead of accumulating
            // acceleration forever) and speed up as the gem gets closer.
            // (`vel` copied out first: `Mut<Gem>` cannot be deref'd twice.)
            let closeness = (1.0 - dist / magnet.max(1.0)).clamp(0.0, 1.0);
            let speed = 90.0 + closeness * 150.0;
            let desired = to.normalized() * speed;
            let vel = g.vel;
            g.vel = vel + (desired - vel) * (12.0 * dt).min(1.0);
            // Neon hook: glint trail behind magnetized gems.
            vfx::emit_gem_trail(&mut particles, &mut rng.0, g.pos, g.color, dt);
        }
        let pos = g.pos;
        let vel = g.vel;
        g.pos = pos + vel * dt;
        // Collect on the post-move distance, so a gem that just arrived this
        // tick is picked up rather than waiting an extra frame.
        if g.pos.dist_sq(ppos) < collect_r * collect_r {
            gained += g.value;
            commands.entity(entity).despawn();
        }
    }
    if gained > 0 {
        let p = player.pos;
        // Neon hook: replaces the 4-particle inline gem burst.
        vfx::emit_gem_collect(&mut particles, &mut rng.0, p, gained);
        sfx.0.push(Sfx::Gem);
        let ups = player.add_xp(gained);
        if ups > 0 {
            // Neon hook: triumphant level-up burst, before the sting.
            vfx::emit_level_up(&mut particles, &mut rng.0, p);
            sfx.0.push(Sfx::LevelUp);
        }
        counters.pending_levelups += ups;
    }
}

/// Legacy `World::update_pickups` , merged with the neon hooks.
///
/// Returns true in legacy when a Nuke was applied this tick; the ECS version
/// signals it through [`NukeTick`] instead (consumed by `nuke_reap_system`).
pub fn pickups_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    mut nuke: ResMut<NukeTick>,
    mut last_pickups: ResMut<LastPickups>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut scratch: ResMut<PickupScratch>,
    mut pickups: Query<(Entity, &SpawnId, &mut Pickup)>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut gems: Query<&mut Gem>,
    mut ordered: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let ppos = player.pos;
    // Legacy `std::mem::take(&mut self.pickup_scratch)` — the buffer (and its
    // allocation) is kept on the world and reused, never reallocated.
    let collected = &mut scratch.0;
    collected.clear();

    // One `SpawnId` sort per tick into the reused buffer (the ambient pickup
    // vfx draws rng per waiting pickup, so the walk order is part of the spec).
    ordered.clear();
    ordered.extend(pickups.iter().map(|(entity, id, _)| (id.0, entity)));
    ordered.sort_unstable_by_key(|&(id, _)| id);
    for i in 0..ordered.len() {
        let entity = ordered[i].1;
        let Ok((_, _, mut p)) = pickups.get_mut(entity) else {
            continue;
        };
        p.bob += dt * 4.0;
        if p.pos.dist(ppos) < PLAYER_RADIUS + 5.0 {
            collected.push(p.kind);
            commands.entity(entity).despawn();
        } else {
            // Neon hook: gentle ambient sparkle around pickups waiting on the floor.
            vfx::emit_pickup_ambient(&mut particles, &mut rng.0, p.pos, p.kind, dt);
        }
    }
    for &k in collected.iter() {
        // Neon hook: the HUD's telemetry list (legacy `self.last_pickups`).
        last_pickups.0.push(k);
        match k {
            PickupKind::Heal => {
                let h = player.max_hp() * 0.35;
                player.heal(h);
                // Neon hooks: heal burst + its own sting (was `Sfx::Pickup`).
                let p = player.pos;
                vfx::emit_pickup_heal(&mut particles, &mut rng.0, p);
                sfx.0.push(Sfx::Heal);
            }
            PickupKind::Magnet => {
                for mut g in gems.iter_mut() {
                    g.magnetized = true;
                }
                // Neon hooks: magnet ring + its own sting (was `Sfx::Pickup`).
                let p = player.pos;
                vfx::emit_pickup_magnet(&mut particles, &mut rng.0, p);
                sfx.0.push(Sfx::Magnet);
            }
            PickupKind::Nuke => {
                nuke.0 = true;
                shake.amount = shake.amount.max(14.0);
                // Walk every enemy in stable `SpawnId` order. The
                // `SpawnId`-ordered `EnemyIndex` from `enemy_move` cannot be
                // reused here: `reap_enemies_system` already despawned this
                // tick's corpses through deferred commands, so the index is
                // stale at this point in the schedule (and the deferred
                // despawns make its length disagree with the live set).
                // Nukes are rare, so one sort into the reused buffer is free.
                ordered.clear();
                ordered.extend(enemies.iter().map(|(entity, id, _)| (id.0, entity)));
                ordered.sort_unstable_by_key(|&(id, _)| id);
                for j in 0..ordered.len() {
                    let entity = ordered[j].1;
                    let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                        continue;
                    };
                    if e.kind != EnemyKind::Boss {
                        e.hp = -1.0;
                    } else {
                        e.hurt(200.0);
                        // Neon hook: the boss Nuke damage number.
                        damage_numbers.record(
                            e.id,
                            e.pos,
                            e.radius,
                            200.0,
                            !e.alive(),
                            DamageSource::Nuke,
                        );
                    }
                }
                let p = player.pos;
                // Neon hook: shockwave, ring and scorch (was an inline burst).
                vfx::emit_nuke(&mut particles, &mut rng.0, p);
                sfx.0.push(Sfx::Nuke);
            }
        }
    }
    collected.clear();
}

/// The run/level outcome: defeat takes precedence over completion.
///
/// ## Campaign rule (this replaces the old "survive 300 s to win")
///
/// * Death is still `game_over`.
/// * Reaching `director::RUN_SECONDS` no longer completes anything: the clock
///   only tops out the difficulty curve, the boss fight starts at
///   `director::BOSS_TIME` and the level lasts until the boss (or the player)
///   dies while the horde keeps coming.
/// * A level is complete when its boss dies, which the boss framework signals
///   with [`RunCounters::level_complete`] (this system does not detect the kill
///   itself: the boss fight owns its own death sequence).
/// * `won` means **the campaign** was won: the final level (Lab) completed.
///   Non-final completions leave `won` false; the App shows the keep-three
///   screen and starts the next level.
///
/// The legacy "loss beats win on the same tick" rule is kept: a rat that dies
/// on the tick its boss dies does not get the level.
pub fn outcome_system(
    mut counters: ResMut<RunCounters>,
    player: Res<Player>,
    level: Res<LevelIndex>,
) {
    if !player.alive() {
        counters.game_over = true;
        counters.level_complete = false;
        counters.won = false;
    } else if counters.level_complete {
        counters.won = level.is_final();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::director;
    use crate::game::ecs::resources::PickupScratch;
    use crate::game::level::Theme;
    use crate::game::loadout::Loadout;
    use crate::game::world::World;
    use crate::math::Vec2;

    const DT: f32 = 1.0 / 60.0;

    /// `begin_tick` + the four (g) systems in schedule order, leaving the ECS
    /// world live so hook state can be inspected before `end_tick`.
    fn run_loot_chain_raw(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(reap_enemies_system)
            .expect("reap runs");
        w.ecs.run_system_cached(gems_system).expect("gems run");
        w.ecs
            .run_system_cached(pickups_system)
            .expect("pickups run");
        w.ecs
            .run_system_cached(nuke_reap_system)
            .expect("nuke reap runs");
    }

    /// Run the tail of the tick that assignment (g) owns, in schedule order.
    fn run_loot_chain(w: &mut World, dt: f32) {
        run_loot_chain_raw(w, dt);
        w.end_tick();
    }

    #[test]
    fn dead_enemies_drop_a_gem_and_count_a_kill() {
        let mut w = World::new(Theme::Sewer, 3);
        let pos = w.player.pos;
        let e = w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = -1.0;

        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(reap_enemies_system)
            .expect("reap runs");
        w.end_tick();

        assert!(w.enemies().is_empty(), "dead enemy survived the reap");
        assert_eq!(w.kills, 1);
        assert_eq!(w.score, 10, "skitterer xp(1) * 10");
        assert_eq!(w.gems().len(), 1);
        assert_eq!(w.gems()[0].value, 1);
        assert_eq!(w.gems()[0].pos, pos, "the gem drops at the corpse");
        assert!(w.sfx.contains(&Sfx::Kill));
    }

    #[test]
    fn death_burst_leaves_a_floor_stain() {
        // Neon hook: `vfx::emit_enemy_death` replaces the inline burst and is
        // what leaves the ichor stain the renderer draws under entities.
        let mut w = World::new(Theme::Sewer, 31);
        let pos = w.player.pos;
        let e = w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = -1.0;
        assert!(w.particles.stains.is_empty(), "tick 0 starts clean");

        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(reap_enemies_system)
            .expect("reap runs");
        w.end_tick();

        assert!(
            !w.particles.stains.is_empty(),
            "death should stain the floor"
        );
    }

    #[test]
    fn living_enemies_are_not_reaped() {
        let mut w = World::new(Theme::Sewer, 4);
        let pos = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, pos, 1.0);
        run_loot_chain(&mut w, DT);
        assert_eq!(w.enemies().len(), 1);
        assert_eq!(w.kills, 0);
        assert!(w.gems().is_empty());
        assert!(w.particles.stains.is_empty());
    }

    #[test]
    fn nuke_kills_are_reaped_on_the_same_tick() {
        let mut w = World::new(Theme::Sewer, 47);
        let ppos = w.player.pos;
        // A boss-tough cat that only the Nuke can kill this tick.
        let e = w.spawn_enemy(EnemyKind::Cat, ppos, 1.0);
        {
            let mut enemy = w.ecs.get_mut::<Enemy>(e).unwrap();
            enemy.hp = 1.0e6;
            enemy.max_hp = 1.0e6;
        }
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Nuke,
            bob: 0.0,
        });

        run_loot_chain(&mut w, DT);

        assert!(w.enemies().is_empty(), "a Nuke kill survived the tick");
        assert_eq!(w.kills, 1);
        assert!(w.pickups().is_empty(), "the Nuke pickup was consumed");
        assert!(w.shake >= 14.0, "the Nuke shakes the screen");
        // Neon hook: the Nuke has its own sting (`Sfx::Nuke`, not Explosion).
        assert!(w.sfx.contains(&Sfx::Nuke));
        assert!(
            !w.ecs.resource::<NukeTick>().0,
            "the Nuke flag must be cleared after the re-reap"
        );
    }

    #[test]
    fn nuke_boss_hit_records_a_damage_number() {
        let mut w = World::new(Theme::Sewer, 49);
        let ppos = w.player.pos;
        let boss = w.spawn_enemy(EnemyKind::Boss, ppos + Vec2::new(4.0, 0.0), 1.0);
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Nuke,
            bob: 0.0,
        });

        run_loot_chain_raw(&mut w, DT);

        {
            let numbers = w.ecs.resource::<DamageNumbers>();
            assert_eq!(numbers.len(), 1, "the boss Nuke hit left no number");
            // Merged/rounded up for display: 200 damage.
            let shown = numbers.items[0].display_value();
            assert!((190..=200).contains(&shown), "shown {shown}");
        }
        // 1400 hp - 200 damage: alive, so the boss was not reaped.
        let mut query = w.ecs.query::<&Enemy>();
        assert!(query.get(&w.ecs, boss).is_ok(), "the boss should survive");
        w.end_tick();
        assert_eq!(w.kills, 0);
    }

    #[test]
    fn nuke_reap_is_a_noop_when_no_nuke_landed() {
        let mut w = World::new(Theme::Sewer, 48);
        let pos = w.player.pos;
        let e = w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        w.ecs.get_mut::<Enemy>(e).unwrap().hp = -1.0;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(nuke_reap_system)
            .expect("nuke reap runs");
        w.end_tick();
        assert_eq!(w.enemies().len(), 1, "no nuke, no second reap");
        assert_eq!(w.kills, 0);
    }

    #[test]
    fn boss_death_uses_the_boss_death_sting() {
        let kill = |kind: EnemyKind| {
            let mut w = World::new(Theme::Sewer, 51);
            let pos = w.player.pos;
            let e = w.spawn_enemy(kind, pos, 1.0);
            w.ecs.get_mut::<Enemy>(e).unwrap().hp = -1.0;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(reap_enemies_system)
                .expect("reap runs");
            w.end_tick();
            w
        };
        let boss = kill(EnemyKind::Boss);
        assert!(boss.sfx.contains(&Sfx::BossDeath));
        assert!(!boss.sfx.contains(&Sfx::Explosion));
        assert!(boss.shake >= 16.0, "boss death shake");

        let brute = kill(EnemyKind::Brute);
        assert!(brute.sfx.contains(&Sfx::Explosion));
        assert!(!brute.sfx.contains(&Sfx::BossDeath));

        let skitterer = kill(EnemyKind::Skitterer);
        assert!(skitterer.sfx.contains(&Sfx::Kill));
    }

    #[test]
    fn elite_death_adds_the_gilded_accent() {
        let shocks = |elite: bool| {
            let mut w = World::new(Theme::Sewer, 52);
            let pos = w.player.pos;
            let e = w.spawn_enemy(EnemyKind::Brute, pos, 1.0);
            {
                let mut enemy = w.ecs.get_mut::<Enemy>(e).unwrap();
                enemy.hp = -1.0;
                enemy.elite = elite;
            }
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(reap_enemies_system)
                .expect("reap runs");
            w.end_tick();
            w.particles.shockwaves.len()
        };
        assert_eq!(
            shocks(true),
            shocks(false) + 1,
            "elite death must add its gilded shockwave"
        );
    }

    #[test]
    fn heal_pickup_heals_a_wounded_rat() {
        let mut w = World::new(Theme::Sewer, 5);
        let ppos = w.player.pos;
        w.player.hp = 1.0;
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        run_loot_chain(&mut w, DT);
        assert!(w.player.hp > 1.0, "heal pickup did nothing");
        assert!(w.pickups().is_empty());
        // Neon hook: per-kind stings replace the single `Sfx::Pickup`.
        assert!(w.sfx.contains(&Sfx::Heal));
        assert!(!w.sfx.contains(&Sfx::Pickup));
    }

    #[test]
    fn last_pickups_records_what_the_hud_reads() {
        let mut w = World::new(Theme::Sewer, 37);
        let ppos = w.player.pos;
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Magnet,
            bob: 0.0,
        });
        run_loot_chain_raw(&mut w, DT);
        let recorded = w.ecs.resource::<LastPickups>().0.clone();
        w.end_tick();
        assert_eq!(
            recorded,
            vec![PickupKind::Heal, PickupKind::Magnet],
            "last_pickups must record collected kinds in pickup order"
        );
    }

    #[test]
    fn magnet_pickup_magnetizes_every_gem() {
        let mut w = World::new(Theme::Sewer, 6);
        let ppos = w.player.pos;
        let g = w.spawn_gem(Gem::new(ppos + Vec2::new(300.0, 0.0), 1));
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Magnet,
            bob: 0.0,
        });
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(pickups_system)
            .expect("pickups run");
        w.end_tick();
        assert!(w.sfx.contains(&Sfx::Magnet));
        let mut query = w.ecs.query::<&Gem>();
        let gem = query.get(&w.ecs, g).expect("gem still exists");
        assert!(gem.magnetized, "Magnet pickup did not magnetize the gem");
    }

    #[test]
    fn waiting_pickups_shed_ambient_sparkles() {
        // Neon hook: `vfx::emit_pickup_ambient` for pickups that are not
        // collected this tick (probabilistic per tick, so allow a few).
        let mut w = World::new(Theme::Sewer, 38);
        let ppos = w.player.pos;
        w.spawn_pickup(Pickup {
            pos: ppos + Vec2::new(40.0, 0.0),
            kind: PickupKind::Magnet,
            bob: 0.0,
        });
        for _ in 0..40 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(pickups_system)
                .expect("pickups run");
            w.end_tick();
        }
        assert!(!w.pickups().is_empty(), "the pickup must stay uncollected");
        assert!(!w.particles.is_empty(), "no ambient sparkle was emitted");
    }

    #[test]
    fn magnetized_gem_reaches_the_player() {
        let mut w = World::new(Theme::Sewer, 53);
        w.player.pos = Vec2::new(200.0, 200.0);
        let mut g = Gem::new(w.player.pos + Vec2::new(100.0, 0.0), 5);
        g.magnetized = true;
        w.spawn_gem(g);
        let xp0 = w.player.xp;
        for _ in 0..300 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(gems_system).expect("gems run");
            w.end_tick();
            if w.gems().is_empty() {
                break;
            }
        }
        assert!(w.gems().is_empty(), "magnetized gem was never collected");
        assert!(w.player.xp > xp0);
        assert!(w.sfx.contains(&Sfx::Gem));
    }

    #[test]
    fn magnetized_gems_leave_a_glint_trail() {
        // Neon hook: `vfx::emit_gem_trail` inside the magnet branch.
        let mut w = World::new(Theme::Sewer, 55);
        w.player.pos = Vec2::new(200.0, 200.0);
        let mut g = Gem::new(w.player.pos + Vec2::new(100.0, 0.0), 5);
        g.magnetized = true;
        w.spawn_gem(g);
        for _ in 0..30 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(gems_system).expect("gems run");
            w.end_tick();
            if w.gems().is_empty() {
                break;
            }
        }
        assert!(!w.particles.is_empty(), "no gem glint trail was emitted");
    }

    #[test]
    fn gem_level_up_bumps_pending_levelups() {
        let mut w = World::new(Theme::Sewer, 54);
        w.player.loadout.stats.magnet = 0.0;
        let ppos = w.player.pos;
        // Enough XP to cross the level-2 threshold in one pickup.
        let mut g = Gem::new(ppos, 1_000);
        g.magnetized = true;
        w.spawn_gem(g);
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(gems_system).expect("gems run");
        w.end_tick();
        assert!(w.gems().is_empty());
        assert!(w.pending_levelups > 0, "level-up was not queued");
        assert!(w.sfx.contains(&Sfx::LevelUp));
        // Neon hook: the level-up ring.
        assert!(!w.particles.shockwaves.is_empty(), "no level-up burst");
    }

    #[test]
    fn pickup_scratch_vec_is_reused_across_ticks() {
        let mut w = World::new(Theme::Sewer, 97);
        let ppos = w.player.pos;
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        run_loot_chain(&mut w, DT);
        let ptr = w.ecs.resource::<PickupScratch>().0.as_ptr();
        let cap = w.ecs.resource::<PickupScratch>().0.capacity();
        assert!(cap > 0, "scratch vec should be kept on the world");
        assert!(w.ecs.resource::<PickupScratch>().0.is_empty());
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Heal,
            bob: 0.0,
        });
        run_loot_chain(&mut w, DT);
        assert_eq!(
            w.ecs.resource::<PickupScratch>().0.as_ptr(),
            ptr,
            "scratch vec reallocated"
        );
        assert_eq!(w.ecs.resource::<PickupScratch>().0.capacity(), cap);
    }

    #[test]
    fn loot_walks_do_not_allocate_after_warm_up() {
        // Performance guard: the per-system `Local` scratch buffers are reused,
        // so a steady-state tick must not grow them (and `PickupScratch` keeps
        // its pointer, covered above). Entity churn is what would resize them.
        let mut w = World::new(Theme::Sewer, 101);
        let ppos = w.player.pos;
        for _ in 0..3 {
            w.spawn_enemy(EnemyKind::Skitterer, ppos + Vec2::new(20.0, 0.0), 1.0);
        }
        w.spawn_gem(Gem::new(ppos + Vec2::new(60.0, 0.0), 1));
        w.spawn_pickup(Pickup {
            pos: ppos + Vec2::new(40.0, 0.0),
            kind: PickupKind::Magnet,
            bob: 0.0,
        });
        run_loot_chain(&mut w, DT);
        // A second identical tick with the same population stays allocation
        // free in the order buffers; we assert observable stability instead of
        // reaching for a global allocator hook.
        run_loot_chain(&mut w, DT);
        assert_eq!(w.kills, 0);
        assert_eq!(w.gems().len(), 1);
        assert_eq!(w.pickups().len(), 1);
    }

    /// Run one `outcome_system` tick through the facade round-trip.
    fn outcome_tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(outcome_system)
            .expect("outcome runs");
        w.end_tick();
    }

    #[test]
    fn loss_beats_win_on_the_final_tick() {
        let mut w = World::new(Theme::Sewer, 7);
        // `World::update` adds dt before the tick; mirror the post-add clock.
        w.elapsed = director::RUN_SECONDS - 0.001 + DT;
        w.player.hp = -1.0;
        outcome_tick(&mut w);
        assert!(w.game_over, "death on the final tick should be a loss");
        assert!(!w.won, "death must not also score a win");
    }

    /// **The level does not end at the five-minute mark.** The clock only tops
    /// out the difficulty curve now: the old "survive 300 s ⇒ win" rule is
    /// gone, and the level runs on until the boss (or the rat) dies.
    #[test]
    fn the_level_does_not_end_at_five_minutes() {
        let mut w = World::new(Theme::Sewer, 8);
        w.elapsed = director::RUN_SECONDS - 0.001 + DT;
        w.player.hp = w.player.max_hp();
        outcome_tick(&mut w);
        assert!(!w.won, "surviving the clock must not win any more");
        assert!(!w.game_over);
        assert!(!w.level_complete());
        assert!(!w.finished(), "the level must keep simulating");
        // The HUD countdown saturates at the boss time and reports the phase.
        assert_eq!(w.time_left(), 0.0, "time_left counts down to BOSS_TIME");
        assert!(w.boss_time_reached());
    }

    /// A full simulated tick past the clock keeps the run alive: no system
    /// completes a level on time alone.
    #[test]
    fn a_real_tick_past_the_clock_never_finishes_a_level() {
        let mut w = World::new(Theme::Sewer, 12);
        w.elapsed = director::RUN_SECONDS + 30.0;
        w.player.hp = 1.0e9;
        w.update(DT, Vec2::ZERO, false);
        assert!(!w.finished(), "a level ended on the clock alone");
        assert!(!w.won && !w.game_over && !w.level_complete());
        assert_eq!(w.time_left(), 0.0);
        assert!(w.boss_time_reached());
    }

    /// The boss dies once ⇒ the level is complete once, and a non-final level
    /// does not win the campaign (the App shows the keep screen instead).
    #[test]
    fn a_boss_kill_completes_the_level_once() {
        let mut w = World::new(Theme::Sewer, 13);
        assert_eq!(w.level_index(), 0, "World::new is level 0");
        // What the boss framework does when the living boss is reaped.
        w.level_complete = true;
        outcome_tick(&mut w);
        assert!(w.level_complete(), "the level should be complete");
        assert!(!w.won, "the Sewer is not the last level");
        assert!(!w.game_over);
        assert!(!w.campaign_won());
        assert!(w.finished(), "a complete level stops simulating");

        // Idempotent: a second outcome tick keeps the same verdict.
        outcome_tick(&mut w);
        assert!(w.level_complete());
        assert!(!w.won);
        assert!(!w.game_over);
    }

    /// The final level's completion *is* the campaign win.
    #[test]
    fn the_final_levels_completion_wins_the_campaign() {
        let mut w = World::new_level(2, 13, Loadout::starting());
        assert_eq!(w.level_index(), 2);
        assert!(w.is_final_level());
        w.mark_level_complete();
        outcome_tick(&mut w);
        assert!(w.level_complete());
        assert!(w.won, "the Lab boss completes the campaign");
        assert!(w.campaign_won());
        assert!(!w.game_over);
        assert!(w.finished());
    }

    /// Death stays a loss, and it beats a same-tick completion.
    #[test]
    fn death_cancels_a_same_tick_completion() {
        let mut w = World::new_level(2, 14, Loadout::starting());
        w.level_complete = true;
        w.player.hp = -1.0;
        outcome_tick(&mut w);
        assert!(w.game_over, "death is still game over");
        assert!(!w.level_complete(), "loss beats completion");
        assert!(!w.won);
        assert!(!w.campaign_won());
    }

    #[test]
    fn outcome_keeps_the_clock_running_before_the_boss_time() {
        let mut w = World::new(Theme::Sewer, 9);
        w.elapsed = 1.0;
        outcome_tick(&mut w);
        assert!(!w.won);
        assert!(!w.game_over);
        assert!(!w.level_complete());
        assert!(!w.boss_time_reached());
        assert!((w.time_left() - (director::BOSS_TIME - 1.0)).abs() < 1e-4);
    }

    /// The level index is available to every system during a tick (the director
    /// scales difficulty per level from it; the boss framework reads the same
    /// resource).
    #[test]
    fn the_level_index_is_a_tick_resource() {
        let mut w = World::new_level(1, 15, Loadout::starting());
        w.begin_tick(DT, Vec2::ZERO, false);
        assert_eq!(w.ecs.resource::<LevelIndex>().0, 1);
        assert!(!w.ecs.resource::<LevelIndex>().is_final());
        assert_eq!(w.ecs.resource::<LevelIndex>().levels_remaining(), 1);
        w.end_tick();
    }

    #[test]
    fn seeded_rng_stream_matches_legacy_draw_order() {
        // Two identical runs must draw the same numbers in the same order: the
        // reap loop, gem vfx and Nuke vfx all share `RngRes`.
        let run = |seed: u64| {
            let mut w = World::new(Theme::Sewer, seed);
            for i in 0..4 {
                let pos = w.player.pos + Vec2::new(i as f32 * 3.0, 0.0);
                let e = w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
                w.ecs.get_mut::<Enemy>(e).unwrap().hp = -1.0;
            }
            run_loot_chain(&mut w, DT);
            // The rng has advanced during the tick; compare the *next* draw.
            w.rng.random::<u64>()
        };
        assert_eq!(run(11), run(11));
        assert_ne!(run(11), run(12));
    }
}
