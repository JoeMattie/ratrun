//! (d) Weapon firing: `fire_weapons`, `fire_one` for every weapon, and
//! nearest-enemy targeting.
//!
//! Ported from the legacy `World` (then replayed against the merged neon
//! branch):
//! - `nearest_enemy` — plain pick shared with `World::nearest_enemy`.
//! - `fire_weapons` (cooldown, `fire_rate_mult`, `damage_mult`, dud-shot retry
//!   rule).
//! - `fire_one` — all six weapons (Gnaw, Cheese Spray, Squeak Nova, Spore
//!   Orbit, Tail Whip, Acid Spit), including Tail Whip's direct `Enemy::hurt`
//!   and Spore Orbit's "carry the angle, drop the old ring" rule.
//!
//! Neon-branch hooks re-applied here (`fire_one`):
//! - `b.weapon = Some(kind)` on every player projectile.
//! - `vfx::emit_weapon_fire(particles, rng, kind, ppos, aim, radius)` replacing
//!   the old inline muzzle/pulse particles (same call sites, same arguments).
//! - Tail Whip: `e.alive()` guard, `vfx::emit_tail_whip_hit(...)` and
//!   `damage_numbers.record(..., DamageSource::TailWhip)` per clipped enemy.
//! - Weapon-specific `Sfx` variants when a shot fires.
//!
//! Resources: `Res<TickInput>` (dt), `ResMut<Player>`, `Res<Level>`,
//! `ResMut<RngRes>`, `ResMut<Particles>`, `ResMut<SfxQueue>`,
//! `Commands` + `ResMut<NextSpawnId>`, `ResMut<DamageNumbers>`,
//! `Res<EnemyIndex>` (stable order) and two `Local` scratch buffers.
//!
//! ## Performance
//!
//! No per-tick heap allocation and no repeated sorting:
//! - The enemy iteration order is taken from `EnemyIndex`, which
//!   `enemy_move::grid_refresh_system` rebuilds in `SpawnId` order immediately
//!   before this system; no enemy is spawned or despawned in between, so it is
//!   complete and current. Narrow single-system tests that never ran the grid
//!   are detected by a length check and fall back to sorting a reused `Local`
//!   scratch **once** for the tick (never once per `nearest_enemy` call).
//! - `nearest_enemy` walks that order and reads through `Query::get`, so it
//!   allocates nothing; it is called at most once per firing weapon.
//! - Tail Whip reuses the same order instead of sorting the enemy query.
//! - Spore Orbit's orbit-bullet scan is a single unsorted pass that keeps the
//!   lowest-`SpawnId` angle and collects entities into a reused `Local` scratch
//!   instead of sorting the bullet query twice.
//!
//! **Stable order:** enemy iteration is `SpawnId` order (exactly legacy `Vec`
//! order before any `swap_remove` shuffle), so nearest-enemy ties and Tail
//! Whip's per-enemy rng draws are deterministic. Dead enemies are skipped by
//! the targeting pick; `total_cmp` keeps NaN distances last exactly as legacy.

// `fire_one` mirrors the legacy signature (weapon stats + the whole sim as
// arguments) and the loadout loop needs Vec indices, exactly like legacy.
#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::bullet::{Bullet, Faction, Lob, Orbit};
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::resources::{EnemyIndex, NextSpawnId, RngRes, SfxQueue, TickInput};
#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;

pub fn fire_weapons_system(
    mut commands: Commands,
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut next_id: ResMut<NextSpawnId>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    bullets: Query<(Entity, &SpawnId, &Bullet)>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut fallback_order: Local<Vec<Entity>>,
    mut orbit_scratch: Local<Vec<Entity>>,
) {
    let dt = input.dt;
    // Legacy: snapshot the player state and the two multipliers.
    let ppos = player.pos;
    let pface = player.face;
    let fire_rate = player.loadout.stats.fire_rate_mult;
    let dmg_mult = player.loadout.stats.damage_mult;
    let n = player.loadout.weapons.len();

    // One stable enemy order for the whole tick. `EnemyIndex` is rebuilt by
    // `enemy_move::grid_refresh_system` right before this system and nothing
    // spawns or despawns an enemy in between, so it is the `SpawnId`-ordered
    // enemy list. Validate its length so the narrow single-system harness
    // (which never runs the grid) still targets correctly: sort a reused
    // scratch once, not once per lookup.
    // `ExactSizeIterator::len` is O(1) (no per-tick allocation or scan).
    let enemy_count = enemies.iter().len();
    let order: &[Entity] = if index.0.len() == enemy_count {
        // Debug-only guard: the cached order must still be the live
        // `SpawnId` order before it decides targeting (see `enemy_move`).
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        fallback_order.clear();
        fallback_order.extend(enemies.iter().map(|(entity, _, _)| entity));
        fallback_order.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        fallback_order.as_slice()
    };

    for i in 0..n {
        let (kind, dmg, cd, projs) = {
            let w = &player.loadout.weapons[i];
            (
                w.kind,
                w.damage() * dmg_mult,
                (w.cooldown() / fire_rate).max(0.05),
                w.level_projectiles(),
            )
        };
        player.loadout.weapons[i].timer -= dt;
        if player.loadout.weapons[i].timer > 0.0 {
            continue;
        }
        let fired = fire_one(
            &mut commands,
            kind,
            dmg,
            cd,
            projs,
            ppos,
            pface,
            &level,
            &mut rng,
            &mut particles,
            &mut damage_numbers,
            &mut sfx,
            &mut next_id,
            order,
            &mut enemies,
            &bullets,
            &mut orbit_scratch,
        );
        if fired {
            player.loadout.weapons[i].timer += cd;
        } else {
            // Nothing spawned (e.g. Gnaw with no live target): retry next
            // tick instead of serving a full cooldown for a dud shot.
            player.loadout.weapons[i].timer = 0.0;
        }
    }
}

/// Plain, allocation-free nearest-enemy pick.
///
/// Shared by [`fire_weapons_system`] and the `World::nearest_enemy` facade
/// (worker h calls it with `(Entity, SpawnId, pos, alive)` tuples pulled from
/// a `(Entity, &SpawnId, &Enemy)` query). Dead candidates are skipped; the
/// winner is the smallest `dist_sq` under `total_cmp` (NaN sorts last), and an
/// exact tie keeps the lowest [`SpawnId`] — the deterministic form of legacy's
/// "first minimum in `Vec` order". The result does not depend on the caller's
/// iteration order.
pub fn nearest_of<I>(candidates: I, from: Vec2) -> Option<(Entity, Vec2)>
where
    I: IntoIterator<Item = (Entity, SpawnId, Vec2, bool)>,
{
    let mut best: Option<(Entity, Vec2, f32, SpawnId)> = None;
    for (entity, id, pos, alive) in candidates {
        if !alive {
            continue;
        }
        let dist = pos.dist_sq(from);
        best = match best {
            None => Some((entity, pos, dist, id)),
            Some((best_entity, best_pos, best_dist, best_id)) => {
                match dist.total_cmp(&best_dist) {
                    Ordering::Less => Some((entity, pos, dist, id)),
                    Ordering::Equal if id.0 < best_id.0 => Some((entity, pos, dist, id)),
                    _ => Some((best_entity, best_pos, best_dist, best_id)),
                }
            }
        };
    }
    best.map(|(entity, pos, _, _)| (entity, pos))
}

/// `World::nearest_enemy` over the tick's stable enemy order.
fn nearest_enemy(
    order: &[Entity],
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    from: Vec2,
) -> Option<(Entity, Vec2)> {
    nearest_of(
        order.iter().filter_map(|&entity| {
            enemies
                .get(entity)
                .ok()
                .map(|(e, id, enemy)| (e, *id, enemy.pos, enemy.alive()))
        }),
        from,
    )
}

/// Legacy `World::fire_one`, with the neon-branch VFX/damage-number hooks.
/// Returns whether a shot was produced (Gnaw with no live target fires
/// nothing).
#[allow(clippy::too_many_arguments)]
fn fire_one(
    commands: &mut Commands,
    kind: WeaponKind,
    dmg: f32,
    cd: f32,
    projs: i32,
    ppos: Vec2,
    pface: Vec2,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    sfx: &mut SfxQueue,
    next_id: &mut NextSpawnId,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bullets: &Query<(Entity, &SpawnId, &Bullet)>,
    orbit_scratch: &mut Vec<Entity>,
) -> bool {
    let color = kind.color();
    let mut fired = false;
    match kind {
        WeaponKind::Gnaw => {
            if let Some((_, epos)) = nearest_enemy(order, enemies, ppos) {
                let dir = (epos - ppos).normalized();
                let mut b = Bullet::new(ppos, dir * 120.0, dmg, Faction::Player, color);
                b.size = 1;
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
                vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, dir, 0.0);
                fired = true;
            }
        }
        WeaponKind::CheeseSpray => {
            let aim = nearest_enemy(order, enemies, ppos)
                .map(|(_, p)| (p - ppos).normalized())
                .unwrap_or(pface);
            let base = aim.angle();
            for k in 0..projs {
                let spread = (k as f32 - (projs - 1) as f32 / 2.0) * 0.18;
                let v = Vec2::from_angle(base + spread) * 110.0;
                let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                b.life = 1.4;
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, aim, 0.0);
            fired = true;
        }
        WeaponKind::SqueakNova => {
            for k in 0..projs {
                let a = k as f32 / projs as f32 * std::f32::consts::TAU;
                let v = Vec2::from_angle(a) * 95.0;
                let mut b = Bullet::new(ppos, v, dmg, Faction::Player, color);
                b.life = 1.3;
                b.weapon = Some(kind);
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, 0.0);
            fired = true;
        }
        WeaponKind::SporeOrbit => {
            let radius = 24.0;
            // Carry the ring's current angle forward, then drop the old ring
            // before spawning its replacement so two never stack. One pass:
            // remember the lowest-`SpawnId` orbit angle (legacy `find_map`
            // over stable `SpawnId` order) and collect the old ring for
            // deferred despawn without sorting the bullet query.
            let mut carry: Option<(u64, f32)> = None;
            orbit_scratch.clear();
            for (entity, id, b) in bullets.iter() {
                if let Some(o) = b.orbit {
                    orbit_scratch.push(entity);
                    if carry.is_none_or(|(best, _)| id.0 < best) {
                        carry = Some((id.0, o.angle));
                    }
                }
            }
            let base = carry.map(|(_, angle)| angle).unwrap_or(0.0);
            for &entity in orbit_scratch.iter() {
                commands.entity(entity).despawn();
            }
            for k in 0..projs {
                let angle = base + k as f32 / projs as f32 * std::f32::consts::TAU;
                let mut b = Bullet::new(ppos, Vec2::ZERO, dmg, Faction::Player, color);
                b.life = cd;
                b.pierce = 999;
                b.size = 2;
                b.radius = 3.0;
                b.weapon = Some(kind);
                b.orbit = Some(Orbit {
                    angle,
                    radius,
                    speed: 3.0,
                });
                commands.spawn((b, next_id.take()));
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, radius);
            fired = true;
        }
        WeaponKind::TailWhip => {
            let radius = 26.0 + projs as f32 * 3.0;
            for &entity in order {
                let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
                    continue;
                };
                if e.alive() && e.pos.dist(ppos) <= radius + e.radius {
                    e.hurt(dmg);
                    let killed = !e.alive();
                    let pos = e.pos;
                    let enemy_radius = e.radius;
                    let enemy_id = e.id;
                    vfx::emit_tail_whip_hit(particles, &mut rng.0, pos, pos - ppos, killed);
                    damage_numbers.record(
                        enemy_id,
                        pos,
                        enemy_radius,
                        dmg,
                        killed,
                        DamageSource::TailWhip,
                    );
                }
            }
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, pface, radius);
            fired = true;
        }
        WeaponKind::AcidSpit => {
            // Lob a projectile: it does no damage in flight and only becomes a
            // pool once it lands (target, wall, or timeout).
            let target = nearest_enemy(order, enemies, ppos)
                .map(|(_, p)| p)
                .unwrap_or(ppos + pface * 50.0)
                .clamp_box(Vec2::splat(4.0), level.arena - Vec2::splat(4.0));
            let to = target - ppos;
            let speed = 110.0;
            let mut b = Bullet::new(ppos, to.normalized() * speed, dmg, Faction::Player, color);
            b.size = 2;
            b.radius = 3.0;
            b.life = to.len() / speed + 0.5;
            b.weapon = Some(kind);
            b.lob = Some(Lob {
                target,
                radius: 12.0 + projs as f32 * 1.5,
                dps: dmg * 4.0,
                life: 2.6,
                color: (140, 220, 70),
            });
            commands.spawn((b, next_id.take()));
            vfx::emit_weapon_fire(particles, &mut rng.0, kind, ppos, to, 0.0);
            fired = true;
        }
    }
    // Only announce a shot when the weapon actually produced something
    // (Gnaw with no target in range fires nothing).
    if fired {
        let sound = match kind {
            WeaponKind::Gnaw => Sfx::Shoot,
            WeaponKind::CheeseSpray => Sfx::ShootSpray,
            WeaponKind::SqueakNova => Sfx::ShootNova,
            WeaponKind::SporeOrbit => Sfx::ShootOrbit,
            WeaponKind::TailWhip => Sfx::ShootWhip,
            WeaponKind::AcidSpit => Sfx::ShootAcid,
        };
        sfx.0.push(sound);
    }
    fired
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::{fire_weapons_system, nearest_of};
    use crate::audio::Sfx;
    use crate::game::bullet::Bullet;
    use crate::game::ecs::components::SpawnId;
    use crate::game::particle::damage_number::DamageNumbers;
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::Theme;
    use crate::game::particle::Particles;
    use crate::game::player::Player;
    use crate::game::weapon::{Weapon, WeaponKind};
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Every kind that spawns a projectile (Tail Whip is an aura).
    const PROJECTILE_WEAPONS: [WeaponKind; 5] = [
        WeaponKind::Gnaw,
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
        WeaponKind::SporeOrbit,
        WeaponKind::AcidSpit,
    ];

    /// Replace the loadout with a single level-1 weapon whose cooldown is
    /// ready, which is what the legacy tests get by calling `World::fire_one`
    /// directly with `cd = 1.0`.
    fn arm(w: &mut World, kind: WeaponKind) {
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level: 1,
            timer: 0.0,
        }];
    }

    fn run_weapons(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
    }

    /// Fire `kind` once at a fixed near target so targeting weapons always
    /// shoot. Returns the world after the tick.
    fn fire_kind(kind: WeaponKind) -> World {
        let mut w = World::new(Theme::Sewer, 1234);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        arm(&mut w, kind);
        let target = w.player.pos + Vec2::new(10.0, 0.0);
        w.spawn_enemy(EnemyKind::Cat, target, 1.0);
        run_weapons(&mut w, 1.0 / 60.0);
        w
    }

    /// Orbit angles in stable `SpawnId` order (legacy `Vec<Bullet>` order).
    fn orbit_angles(ecs: &mut bevy_ecs::world::World) -> Vec<f32> {
        let mut query = ecs.query::<(&SpawnId, &Bullet)>();
        query
            .iter(ecs)
            .sort_by_key::<&SpawnId, _>(|id| id.0)
            .filter_map(|(_, b)| b.orbit.map(|o| o.angle))
            .collect()
    }

    /// Legacy `spore_orbit_replaces_its_ring_and_keeps_its_angle`: a second
    /// volley drops the old ring instead of stacking, and the first new orbit
    /// continues from the old ring's first angle.
    #[test]
    fn spore_orbit_replaces_its_ring_and_keeps_its_angle() {
        let mut w = World::new(Theme::Sewer, 4);
        w.clear_enemies();
        w.clear_bullets();
        arm(&mut w, WeaponKind::SporeOrbit);
        // Level 2 grants 2 + level = 4 orbit spores, the legacy test's
        // `projs = 4`, without bypassing the loadout's projectile count.
        w.player.loadout.weapons[0].level = 2;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        assert_eq!(orbit_angles(&mut w.ecs).len(), 4);

        // Legacy test: advance every orbit angle by 0.7 before re-firing.
        {
            let mut query = w.ecs.query::<&mut Bullet>();
            for mut b in query.iter_mut(&mut w.ecs) {
                if let Some(o) = b.orbit.as_mut() {
                    o.angle += 0.7;
                }
            }
        }
        let carried = orbit_angles(&mut w.ecs)[0];

        // Ready the weapon again, exactly like resetting `weapons[i].timer`.
        w.ecs
            .resource_mut::<Player>()
            .loadout
            .weapons[0]
            .timer = 0.0;
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        let angles = orbit_angles(&mut w.ecs);
        w.end_tick();

        assert_eq!(angles.len(), 4, "two spore rings stacked");
        assert!(
            (angles[0] - carried).abs() < 1e-4,
            "ring angle snapped instead of carrying forward"
        );
    }

    /// Legacy `untargeted_gnaw_does_not_emit_a_shoot_sfx`.
    #[test]
    fn untargeted_gnaw_does_not_emit_a_shoot_sfx() {
        let mut w = World::new(Theme::Sewer, 2);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(w.bullets().is_empty());
        assert!(!w.sfx.contains(&Sfx::Shoot));
    }

    /// Legacy `untargeted_gnaw_retries_next_tick`: a dud shot leaves the timer
    /// at exactly 0 so the weapon retries instead of serving a cooldown.
    #[test]
    fn untargeted_gnaw_retries_next_tick() {
        let mut w = World::new(Theme::Sewer, 61);
        w.clear_enemies();
        w.clear_bullets();
        w.player.loadout.weapons[0].timer = 0.0;

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(w.bullets().is_empty(), "Gnaw fired with no target");
        assert_eq!(
            w.player.loadout.weapons[0].timer, 0.0,
            "a dud shot should leave the timer ready for next tick"
        );
    }

    /// Legacy `nearest_enemy_skips_dead_and_nan_positions`, exercised through
    /// the firing system: the dart must aim at the live enemy, not the dead one
    /// sitting on the player or the NaN one.
    #[test]
    fn nearest_enemy_skips_dead_and_nan_positions() {
        let mut w = World::new(Theme::Sewer, 29);
        w.clear_enemies();
        w.clear_bullets();
        let mut rng = StdRng::seed_from_u64(3);
        let from = w.player.pos;

        let dead = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
        w.ecs.get_mut::<Enemy>(dead).unwrap().hp = 0.0;
        let nan = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
        w.ecs.get_mut::<Enemy>(nan).unwrap().pos = Vec2::new(f32::NAN, f32::NAN);
        let alive_pos = from + Vec2::new(10.0, 0.0);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, alive_pos, 1.0, &mut rng);
        arm(&mut w, WeaponKind::Gnaw);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(w.bullets().len(), 1, "should pick exactly the live enemy");
        let v = w.bullets()[0].vel;
        assert!(
            (v.x - 120.0).abs() < 1e-3 && v.y.abs() < 1e-3,
            "aimed at {:?} instead of the live enemy",
            v
        );
    }

    /// The plain pick shared with `World::nearest_enemy`: ties keep the lowest
    /// `SpawnId` regardless of iteration order, dead entries are skipped and a
    /// NaN distance never wins over a finite one.
    #[test]
    fn nearest_of_is_order_independent_and_stable() {
        let mut w = World::new(Theme::Sewer, 7);
        w.clear_enemies();
        let a = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(10.0, 0.0), 1.0);
        let b = w.spawn_enemy(EnemyKind::Skitterer, Vec2::new(-10.0, 0.0), 1.0);
        // Equal distances: the lower SpawnId wins even when it comes second.
        let picked = nearest_of(
            vec![
                (b, SpawnId(9), Vec2::new(-10.0, 0.0), true),
                (a, SpawnId(4), Vec2::new(10.0, 0.0), true),
            ],
            Vec2::ZERO,
        )
        .expect("a live enemy");
        assert_eq!(picked.0, a);

        let dead = w.spawn_enemy(EnemyKind::Skitterer, Vec2::ZERO, 1.0);
        let nan = w.spawn_enemy(EnemyKind::Skitterer, Vec2::ZERO, 1.0);
        let picked = nearest_of(
            vec![
                (dead, SpawnId(1), Vec2::ZERO, false),
                (nan, SpawnId(2), Vec2::new(f32::NAN, f32::NAN), true),
                (a, SpawnId(3), Vec2::new(4.0, 0.0), true),
            ],
            Vec2::ZERO,
        )
        .expect("a live enemy");
        assert_eq!(picked.0, a);
    }

    /// Legacy `acid_spit_launches_a_projectile_not_an_instant_pool`.
    #[test]
    fn acid_spit_launches_a_projectile_not_an_instant_pool() {
        let mut w = World::new(Theme::Sewer, 73);
        w.clear_enemies();
        w.clear_bullets();
        w.clear_pools();
        arm(&mut w, WeaponKind::AcidSpit);

        run_weapons(&mut w, 1.0 / 60.0);

        assert_eq!(w.bullets().len(), 1);
        assert!(w.bullets()[0].lob.is_some(), "acid shot should be lobbed");
        assert!(w.pools().is_empty(), "acid pool appeared instantly");
    }

    /// Multi-projectile weapons still emit exactly one `Sfx::Shoot`.
    #[test]
    fn one_shoot_sfx_per_successful_shot() {
        let mut w = World::new(Theme::Sewer, 2);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        arm(&mut w, WeaponKind::CheeseSpray);

        run_weapons(&mut w, 1.0 / 60.0);

        // Cheese Spray level 1: 2 + level projectiles.
        assert_eq!(w.bullets().len(), 3);
        assert_eq!(
            w.sfx.iter().filter(|s| **s == Sfx::ShootSpray).count(),
            1,
            "one specialized shot sfx per volley"
        );
    }

    /// Tail Whip damages in place (through `Enemy::hurt`) only inside the aura.
    #[test]
    fn tail_whip_hurts_enemies_inside_the_radius_only() {
        let mut w = World::new(Theme::Sewer, 5);
        w.clear_enemies();
        w.clear_bullets();
        w.sfx.clear();
        let ppos = w.player.pos;
        let near = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(10.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(80.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        let near_hp = w.ecs.get::<Enemy>(near).unwrap().hp;
        let far_hp = w.ecs.get::<Enemy>(far).unwrap().hp;
        assert!(near_hp < EnemyKind::Cat.base_hp(), "near enemy unhurt");
        assert_eq!(far_hp, EnemyKind::Cat.base_hp(), "far enemy hurt");
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::ShootWhip).count(), 1);
        assert!(w.bullets().is_empty(), "Tail Whip must not spawn bullets");
    }

    /// Neon hook: every player projectile is tagged with the weapon that fired
    /// it (used by `resolve_player_bullets` for spark/damage-number styling).
    #[test]
    fn every_projectile_is_tagged_with_its_weapon() {
        for kind in PROJECTILE_WEAPONS {
            let mut w = fire_kind(kind);
            let bullets = w.bullets();
            assert!(!bullets.is_empty(), "{kind:?} fired nothing");
            for b in &bullets {
                assert_eq!(b.weapon, Some(kind), "{kind:?} left an untagged bullet");
            }
        }
    }

    /// Neon hook: each weapon emits its own `Sfx` variant (legacy
    /// `Sfx::Shoot` only for Gnaw).
    #[test]
    fn weapons_emit_their_specialized_sfx() {
        let cases = [
            (WeaponKind::Gnaw, Sfx::Shoot),
            (WeaponKind::CheeseSpray, Sfx::ShootSpray),
            (WeaponKind::SqueakNova, Sfx::ShootNova),
            (WeaponKind::SporeOrbit, Sfx::ShootOrbit),
            (WeaponKind::TailWhip, Sfx::ShootWhip),
            (WeaponKind::AcidSpit, Sfx::ShootAcid),
        ];
        for (kind, expected) in cases {
            let w = fire_kind(kind);
            assert_eq!(
                w.sfx.iter().filter(|s| **s == expected).count(),
                1,
                "{kind:?} should emit exactly one {expected:?}, got {:?}",
                w.sfx
            );
        }
    }

    /// Neon hook: `vfx::emit_weapon_fire` runs for every weapon — particles for
    /// most, a shockwave ring for Squeak Nova / Tail Whip.
    #[test]
    fn weapon_fire_emits_muzzle_vfx() {
        let gnaw = fire_kind(WeaponKind::Gnaw);
        assert!(
            !gnaw.particles.is_empty(),
            "Gnaw muzzle particles missing"
        );

        let nova = fire_kind(WeaponKind::SqueakNova);
        assert!(
            !nova.particles.shockwaves.is_empty(),
            "Squeak Nova shockwave missing"
        );

        let whip = fire_kind(WeaponKind::TailWhip);
        assert!(
            !whip.particles.shockwaves.is_empty(),
            "Tail Whip pulse ring missing"
        );
    }

    /// Neon hook: Tail Whip records a floating damage number per clipped
    /// enemy and emits its hit VFX.
    #[test]
    fn tail_whip_records_damage_numbers_and_hit_vfx() {
        let mut w = World::new(Theme::Sewer, 4242);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        let enemy = w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(10.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);
        let hp_before = w.ecs.get::<Enemy>(enemy).unwrap().hp;

        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        let particles_before = w.ecs.resource::<Particles>().len();
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        let hp_after = w.ecs.get::<Enemy>(enemy).unwrap().hp;
        let (numbers_len, first_number) = {
            let numbers = w.ecs.resource::<DamageNumbers>();
            (
                numbers.len(),
                numbers.items.first().map(|n| (n.total_dmg, n.is_kill)),
            )
        };
        let particles_after = w.ecs.resource::<Particles>().len();
        let shockwaves = w.ecs.resource::<Particles>().shockwaves.len();
        w.end_tick();

        assert!(hp_after < hp_before, "Tail Whip did not hurt the enemy");
        assert_eq!(numbers_len, 1, "one damage number per clipped enemy");
        let (total_dmg, is_kill) = first_number.expect("damage number recorded");
        assert!(
            (total_dmg - (hp_before - hp_after)).abs() < 1e-4,
            "number {total_dmg} != dealt {}",
            hp_before - hp_after
        );
        assert!(!is_kill);
        assert!(
            particles_after > particles_before,
            "tail-whip hit VFX missing"
        );
        assert_eq!(shockwaves, 1, "tail-whip pulse ring missing");
    }

    /// An enemy outside the aura records no damage number.
    #[test]
    fn tail_whip_records_nothing_for_far_enemies() {
        let mut w = World::new(Theme::Sewer, 1717);
        w.clear_enemies();
        w.clear_bullets();
        let ppos = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, ppos + Vec2::new(80.0, 0.0), 1.0);
        arm(&mut w, WeaponKind::TailWhip);

        run_weapons(&mut w, 1.0 / 60.0);

        assert!(
            w.damage_numbers.is_empty(),
            "far enemy should not pop a damage number"
        );
    }
}
