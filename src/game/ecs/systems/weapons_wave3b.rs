//! Wave 3's self-driving weapons: **Laser Eyes**, **Super Dash**, and the
//! bookkeeping **Counterweight**'s melee nudge needs.
//!
//! `weapons::fire_weapons_system` skips both kinds
//! (`WeaponKind::fires_outside_fire_one`), because neither is a "spawn a
//! projectile now" weapon:
//!
//! - **Laser Eyes** is an ordinary auto-firing weapon whose shot is a *burst*:
//!   it charges its own `Weapon::timer` here, and on the trigger tick it locks
//!   the rat's facing and resolves `Weapon::laser_pulses()` pulses spread
//!   evenly across `-35°..+35°`, `LASER_PULSE_INTERVAL` apart. Each pulse is a
//!   ray that pierces every exposed body it reaches and stops at the first
//!   wall.
//! - **Super Dash** is a weapon whose *fire* is the dash. `player_system`
//!   records the dash's real travelled segments into [`DashTrack`]; this system
//!   damages every body those segments crossed, once per body per dash. A
//!   blocked or cooling dash records no segment, so it buys no hit, and the
//!   dash's own motion and immunity are untouched (nothing here writes
//!   `Player`).
//!
//! Schedule slot 11b (right after `weapons::active_weapons_system`): the cached
//! [`EnemyIndex`] is the live `SpawnId` order there (nothing has spawned or
//! been reaped since `grid_refresh`), the damage lands before
//! `player_bullet_hits`/`reap_enemies` in the same tick, and both systems see
//! this tick's player position.
//!
//! Determinism: every walk is `SpawnId` order (or the cached `EnemyIndex`, which
//! *is* that order), every rng draw happens inside a `vfx` emitter called only
//! when something actually happened, and a run that never owns either weapon
//! draws no rng and changes no value — the golden traces do not move.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::segment_circle_hit;
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::resources::{
    DamageLedger, DashTrack, EnemyIndex, LaserFx, MeleeNudge, RngRes, SfxQueue, Shake, TickInput,
};
use crate::game::ecs::systems::boss::Boss;
use crate::game::enemy::Enemy;
use crate::game::level::Level;
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::game::weapon::{
    WeaponKind, LASER_HALF_WIDTH, LASER_PULSE_INTERVAL, LASER_SWEEP_HALF, LASER_TAIL,
};
use crate::math::Vec2;

#[cfg(debug_assertions)]
use crate::game::ecs::resources::debug_assert_enemy_index;

/// How often Counterweight may nudge the same body (design §5).
pub const MELEE_NUDGE_COOLDOWN: f32 = 0.4;

/// The tick's stable `SpawnId` enemy order.
///
/// Prefers the cached [`EnemyIndex`] (identical order, no sort) and falls back
/// to sorting a reused `Local` buffer once for the tick, which is what narrow
/// single-system tests need (they never run the grid). Same rule as
/// `fire_weapons_system`; the returned slice borrows only `index` or
/// `fallback`, never the query, so a caller can still take `&mut` on it.
fn stable_order<'a>(
    index: &'a EnemyIndex,
    enemies: &Query<(Entity, &SpawnId, &mut Enemy)>,
    fallback: &'a mut Vec<Entity>,
) -> &'a [Entity] {
    if index.0.len() == enemies.iter().len() {
        #[cfg(debug_assertions)]
        debug_assert_enemy_index(
            &index.0,
            enemies.iter().map(|(entity, id, _)| (id.0, entity)),
        );
        &index.0
    } else {
        fallback.clear();
        fallback.extend(enemies.iter().map(|(entity, _, _)| entity));
        fallback.sort_by_key(|&entity| {
            enemies
                .get(entity)
                .map(|(_, id, _)| id.0)
                .unwrap_or(u64::MAX)
        });
        fallback.as_slice()
    }
}

/// Does the bar `origin -> origin + dir * len`, `half_w` px either side, reach
/// the circle at `center` with radius `r`?
///
/// The beam is a bar, not a capsule: the body's near edge has to be in front of
/// the beam's end (which is where a wall stopped it), and its perpendicular
/// distance from the ray has to be inside `half_w + r`. Bodies behind the
/// muzzle fail the same projection, so a rat standing in a crowd never shoots
/// backwards.
#[inline]
pub fn ray_hits_circle(origin: Vec2, dir: Vec2, len: f32, half_w: f32, center: Vec2, r: f32) -> bool {
    let rel = center - origin;
    let along = rel.dot(dir);
    if along + r < 0.0 || along - r > len {
        return false;
    }
    let lateral = (rel - dir * along).len();
    lateral <= half_w + r
}

/// Angle offset (radians) of pulse `index` in a burst of `pulses`, spread
/// evenly across `-35°..+35°` of the locked facing. A single pulse sits dead
/// ahead.
#[inline]
pub fn pulse_angle_offset(index: u32, pulses: u32) -> f32 {
    if pulses <= 1 {
        return 0.0;
    }
    let t = index as f32 / (pulses - 1) as f32;
    -LASER_SWEEP_HALF + 2.0 * LASER_SWEEP_HALF * t
}

/// Resolve one Laser Eyes pulse: pierce every exposed body along the ray, then
/// the boss's grounded hit shapes. Returns `(hits, beam length after the wall
/// stop)`.
///
/// `ghost` is a **Carbon Copy** afterimage pulse: same geometry and the same
/// ledger column, half the recorded damage (the caller passes it), amber
/// dashed art instead of the live beam, and no sound — the copy is a negative,
/// and two rasps per pulse would read as one louder weapon.
#[allow(clippy::too_many_arguments)]
fn laser_pulse(
    origin: Vec2,
    dir: Vec2,
    nominal: f32,
    dmg: f32,
    ghost: bool,
    level: &Level,
    rng: &mut RngRes,
    particles: &mut Particles,
    order: &[Entity],
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    bosses: &mut Query<&mut Boss>,
    damage_numbers: &mut DamageNumbers,
    ledger: &mut DamageLedger,
    sfx: &mut SfxQueue,
) -> (u32, f32) {
    let kind = WeaponKind::LaserEyes;
    // Walls stop the beam: everything past the first face is shadowed.
    let end = origin + dir * nominal;
    let stop = level
        .tiles
        .segment_hit(origin, end, LASER_HALF_WIDTH)
        .map(|t| t.clamp(0.0, 1.0))
        .unwrap_or(1.0);
    let len = nominal * stop;

    let mut hits = 0u32;
    for &entity in order {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        if !ray_hits_circle(origin, dir, len, LASER_HALF_WIDTH, e.pos, e.radius) {
            continue;
        }
        let hp_before = e.hp.max(0.0);
        // Directional armour: the beam arrives from the rat's eyes.
        let applied = e.hurt_from(dmg, origin);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        if ghost {
            vfx::emit_carbon_copy_hit(particles, &mut rng.0, pos, pos - origin, killed);
        } else {
            vfx::emit_laser_hit(particles, &mut rng.0, pos, pos - origin, killed);
        }
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Normal,
        );
        hits += 1;
    }

    // The boss is a body too: its grounded hit shapes are pierced like
    // everything else (a lifted foot or a mid-pounce jaguar is not).
    for mut boss in bosses.iter_mut() {
        if boss.dead {
            continue;
        }
        let shapes = boss.hit_shapes();
        if shapes.iter().next().is_none() {
            continue;
        }
        if !shapes.segment_hits_grounded(origin, origin + dir * len.max(0.5), LASER_HALF_WIDTH) {
            continue;
        }
        let amount = dmg * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        if ghost {
            vfx::emit_carbon_copy_hit(particles, &mut rng.0, boss.pos, boss.pos - origin, killed);
        } else {
            vfx::emit_laser_hit(particles, &mut rng.0, boss.pos, boss.pos - origin, killed);
        }
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            killed,
            DamageSource::Normal,
        );
        hits += 1;
    }

    if ghost {
        vfx::emit_carbon_copy_pulse(particles, &mut rng.0, origin, dir.angle(), len, hits);
    } else {
        vfx::emit_laser_pulse(particles, &mut rng.0, origin, dir.angle(), len, hits);
    }
    if !ghost {
        // One zap per pulse: a burst is a short rattle of fire, not one shot.
        sfx.0.push(Sfx::LaserEyes);
    }
    (hits, len)
}

/// Laser Eyes: charge the weapon, lock the facing, and resolve the burst's
/// pulses on schedule.
///
/// The burst is the weapon's shot, so it fires on its own cycle without needing
/// a target (like Squeak Nova, unlike Gnaw): a sweep down a corridor is worth
/// firing before the horde is on screen. `player.face` is read **once**, at the
/// trigger tick — turning afterwards does not drag the sweep, which is what
/// makes the fan read as a fan.
#[allow(clippy::too_many_arguments)]
pub fn laser_eyes_system(
    input: Res<TickInput>,
    mut player: ResMut<Player>,
    level: Res<Level>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut shake: ResMut<Shake>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut fx: ResMut<LaserFx>,
    mut fallback: Local<Vec<Entity>>,
) {
    let dt = input.dt;
    let (dmg_mult, fire_rate, reach_mult) = {
        let s = player.loadout.stats;
        (s.damage_mult, s.fire_rate_mult.max(0.1), s.reach_mult)
    };

    // 1. Charge (and trigger). Only one burst is in flight at a time, and a
    // burst is always shorter than its cycle, so a retrigger cannot overlap.
    let mut trigger: Option<(u32, f32, f32, bool)> = None; // (pulses, reach, dmg, evolved)
    for w in player.loadout.weapons.iter_mut() {
        if w.kind != WeaponKind::LaserEyes {
            continue;
        }
        w.timer -= dt;
        if w.timer > 0.0 || fx.active() {
            continue;
        }
        w.timer += (w.cooldown() / fire_rate).max(0.05);
        // The beam's length is a ranged reach: Lens Cap stretches it.
        trigger = Some((
            w.laser_pulses().max(1),
            reach_mult * w.laser_reach(),
            w.damage() * dmg_mult,
            w.evolved,
        ));
    }
    if let Some((pulses, reach, dmg, evolved)) = trigger {
        let facing = if player.face.len_sq() > 1e-4 {
            player.face.normalized()
        } else {
            Vec2::new(0.0, -1.0)
        };
        let origin = player.pos;
        let total = (pulses - 1) as f32 * LASER_PULSE_INTERVAL + LASER_TAIL;
        // Carbon Copy: the sweep recorded at the end of the *previous* burst
        // replays now, backwards, from its own origin and facing. One copy is
        // kept, and a copy is never itself recorded (only a real sweep is).
        let recorded = if evolved { fx.copy } else { None };
        let copy_total = match recorded {
            Some(copy) => (copy.pulses - 1) as f32 * LASER_PULSE_INTERVAL + LASER_TAIL,
            None => 0.0,
        };
        *fx = LaserFx {
            left: total,
            total,
            age: 0.0,
            fired: 0,
            pulses,
            facing,
            origin,
            reach,
            dmg,
            angle: facing.angle() + pulse_angle_offset(0, pulses),
            beam: 0.0,
            since_pulse: 0.0,
            evolved,
            copy: recorded,
            copy_left: copy_total,
            copy_age: 0.0,
            copy_fired: 0,
        };
        shake.amount = shake.amount.max(0.9);
        vfx::emit_laser_muzzle(&mut particles, &mut rng.0, origin, facing);
        if let Some(copy) = recorded {
            // The negative's camera: an amber eye pair left where the recorded
            // sweep fired from.
            vfx::emit_carbon_copy_afterimage(&mut particles, &mut rng.0, copy.origin, copy.facing, copy.pulses);
        }
    }

    // 2. Resolve this tick's pulses. `age` decides how many are owed; at 60 fps
    // a tick never owes two, but a large test dt can.
    if !fx.active() {
        return;
    }
    fx.age += dt;
    fx.since_pulse += dt;
    let due = ((fx.age / LASER_PULSE_INTERVAL) as u32 + 1).min(fx.pulses);
    if due > fx.fired {
        let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
        while fx.fired < due {
            let offset = pulse_angle_offset(fx.fired, fx.pulses);
            let dir = Vec2::from_angle(fx.facing.angle() + offset);
            let (_, beam) = laser_pulse(
                fx.origin,
                dir,
                fx.reach,
                fx.dmg,
                false,
                &level,
                &mut rng,
                &mut particles,
                order,
                &mut enemies,
                &mut bosses,
                &mut damage_numbers,
                &mut ledger,
                &mut sfx,
            );
            fx.angle = dir.angle();
            fx.beam = beam;
            fx.fired += 1;
            fx.since_pulse = 0.0;
        }
    }

    // 3. The Carbon Copy afterimage on the same cadence, in **reverse**: its
    // first pulse is the recorded sweep's last angle, so the two fans cross.
    if fx.copy_active() {
        let copy = fx.copy;
        if let Some(copy) = copy {
            fx.copy_age += dt;
            let copy_due = ((fx.copy_age / LASER_PULSE_INTERVAL) as u32 + 1).min(copy.pulses);
            if copy_due > fx.copy_fired {
                let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
                while fx.copy_fired < copy_due {
                    // Backwards: the recorded sweep ran pulse 0 → n-1, so the
                    // copy runs n-1 → 0.
                    let slot = copy.pulses - 1 - fx.copy_fired;
                    let offset = pulse_angle_offset(slot, copy.pulses);
                    let dir = Vec2::from_angle(copy.facing.angle() + offset);
                    laser_pulse(
                        copy.origin,
                        dir,
                        copy.reach,
                        copy.dmg * crate::game::ecs::systems::evolutions::CARBON_COPY_DAMAGE,
                        true,
                        &level,
                        &mut rng,
                        &mut particles,
                        order,
                        &mut enemies,
                        &mut bosses,
                        &mut damage_numbers,
                        &mut ledger,
                        &mut sfx,
                    );
                    fx.copy_fired += 1;
                }
                vfx::emit_carbon_copy_afterimage(
                    &mut particles,
                    &mut rng.0,
                    copy.origin,
                    copy.facing,
                    copy.pulses,
                );
            }
            fx.copy_left = (fx.copy_left - dt).max(0.0);
        }
    }

    fx.left = (fx.left - dt).max(0.0);
    if fx.left <= 0.0 {
        // Done. A completed sweep is what the next burst's afterimage replays
        // (Carbon Copy keeps exactly one); everything else starts clean.
        // `fx.evolved` is the burst's own flag (the trigger tick is long gone
        // by the time it completes).
        let recorded = if fx.evolved { fx.copy_of_this_sweep() } else { None };
        *fx = LaserFx {
            copy: recorded,
            ..LaserFx::default()
        };
    }
}

/// Super Dash: damage every body this dash's travelled segment crosses, once
/// per body per dash.
///
/// `player_system` writes the segment (`DashTrack`); this system only reads it,
/// so a press that never became a dash cannot reach this code at all.
#[allow(clippy::too_many_arguments)]
pub fn super_dash_system(
    player: Res<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    index: Res<EnemyIndex>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut bosses: Query<&mut Boss>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut ledger: ResMut<DamageLedger>,
    mut track: ResMut<DashTrack>,
    mut fallback: Local<Vec<Entity>>,
) {
    let Some((from, to)) = track.segment else {
        return;
    };
    // The weapon's own numbers: no weapon, no hit — and no rng, no sfx, so a
    // run without Super Dash is byte-identical to one before this feature.
    let Some(w) = player
        .loadout
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::SuperDash)
    else {
        return;
    };
    let radius = w.dash_radius();
    let dmg = w.damage() * player.loadout.stats.damage_mult;
    let kind = WeaponKind::SuperDash;
    // The legs the rat actually travelled this tick (collision-resolved
    // sub-steps). A chord across them could cut a corner a wall put in the
    // path; `record` guarantees at least one leg whenever `segment` is set, so
    // a dash that moved nothing (pressed flush into a wall) hits nothing.
    // Copied out of the resource (the array is `Copy`), so the hit walk below
    // can still mark hits and extend the Ripcord route.
    let legs = track.segs;
    let leg_n = track.seg_len;
    let on_legs = |pos: Vec2, other: f32| {
        (0..leg_n).any(|i| {
            let (a, b) = legs[i];
            segment_circle_hit(a, b, radius, pos, other)
        })
    };

    // One sound for the whole dash, on its first leg.
    if track.legs == 1 {
        sfx.0.push(Sfx::SuperDash);
    }
    // Ripcord: the real travelled legs are kept for 0.6 s after the dash, so
    // the evolution damages the route the rat actually took (a dash stopped by
    // a wall records its short leg, exactly like the dash hit does).
    if w.evolved {
        for &(a, b) in &legs[..leg_n] {
            track.extend_route(a, b);
        }
    }
    vfx::emit_super_dash_comb(&mut particles, &mut rng.0, from, to, radius);

    let order: &[Entity] = stable_order(&index, &enemies, &mut fallback);
    for &entity in order {
        let Ok((_, id, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() || track.already_hit(id.0) {
            continue;
        }
        if !on_legs(e.pos, e.radius) {
            continue;
        }
        track.mark_hit(id.0);
        let hp_before = e.hp.max(0.0);
        // Directional armour: the hit arrives along the dash's travel, i.e.
        // from the point one leg behind the body (a zero-length leg has no
        // direction and takes the unmodified multiplier).
        let arrives_from = e.pos - (to - from);
        let applied = e.hurt_from(dmg, arrives_from);
        let killed = !e.alive();
        ledger.add(kind, (hp_before - e.hp.max(0.0)).max(0.0));
        let pos = e.pos;
        let enemy_radius = e.radius;
        let enemy_id = e.id;
        // The dash drags what it crosses along with it: the ride is the point.
        let out = (pos - from).normalized();
        e.vel += out * 40.0;
        vfx::emit_super_dash_hit(&mut particles, &mut rng.0, pos, pos - from, killed);
        damage_numbers.record(
            enemy_id,
            pos,
            enemy_radius,
            applied,
            killed,
            DamageSource::Normal,
        );
    }

    // Bosses take the damage only (the slide is Ripcord's, in the evolution
    // table).
    for mut boss in bosses.iter_mut() {
        if boss.dead || track.boss_hit {
            continue;
        }
        let shapes = boss.hit_shapes();
        if shapes.iter().next().is_none() {
            continue;
        }
        if !shapes.segment_hits_grounded(from, to, radius) {
            continue;
        }
        track.boss_hit = true;
        let amount = dmg * boss.damage_mult;
        if !boss.take_damage(amount) {
            continue;
        }
        ledger.add_boss(kind, amount);
        let killed = boss.hp <= 0.0;
        vfx::emit_super_dash_hit(&mut particles, &mut rng.0, boss.pos, boss.pos - from, killed);
        damage_numbers.record(
            boss.id,
            boss.pos,
            boss.kind.body_radius(),
            amount,
            killed,
            DamageSource::Normal,
        );
    }
}

/// Decay Counterweight's per-body nudge cooldowns (nothing else).
///
/// `weapons::fire_one` asks the log before nudging a melee hit; this system
/// keeps the timers honest. With no Counterweight stacks the log is always
/// empty, so the whole pass is a no-op.
pub fn melee_nudge_system(input: Res<TickInput>, mut nudge: ResMut<MeleeNudge>) {
    if nudge.len() > 0 {
        nudge.tick(input.dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ecs::systems::weapons::fire_weapons_system;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::art_walls::WallKind;
    use crate::game::weapon::Weapon;
    use crate::game::world::World;

    /// Wipe the generated layout's walls so a test measures the beam, not the
    /// map.
    fn open_map(w: &mut World) {
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    }

    /// A 3-cell solid column whose left face is the line `x = 16 * cx`.
    fn wall_column(w: &mut World, cx: i32, cy: i32) -> f32 {
        w.level
            .tiles
            .fill_cells(cx, cy - 1, 1, 3, TileKind::Wall(WallKind::Solid));
        w.level.tiles.rebuild_masks();
        (cx * 16) as f32
    }

    /// Replace the loadout with one ready weapon at `level`.
    fn arm(w: &mut World, kind: WeaponKind, level: u8) {
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level,
            timer: 0.0,
            evolved: false,
        }];
    }

    fn run_laser(w: &mut World, dt: f32) {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(laser_eyes_system).unwrap();
        w.end_tick();
    }

    fn run_super_dash(w: &mut World) {
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
    }

    fn enemy_hp(w: &World, e: Entity) -> f32 {
        w.ecs.get::<Enemy>(e).map(|e| e.hp).unwrap_or(f32::NAN)
    }

    /// The sweep's angles: evenly spread from -35° to +35° of the locked
    /// facing, dead ahead for a single pulse.
    #[test]
    fn pulse_angles_span_the_design_sweep() {
        let three: Vec<f32> = (0..3).map(|i| pulse_angle_offset(i, 3)).collect();
        assert!((three[0] + LASER_SWEEP_HALF).abs() < 1e-6, "first pulse -35°");
        assert!(three[1].abs() < 1e-6, "middle pulse dead ahead");
        assert!((three[2] - LASER_SWEEP_HALF).abs() < 1e-6, "last pulse +35°");
        // Even spacing for the 4- and 5-pulse levels too.
        for pulses in [4u32, 5] {
            let step = 2.0 * LASER_SWEEP_HALF / (pulses - 1) as f32;
            let mut prev = pulse_angle_offset(0, pulses);
            assert!((prev + LASER_SWEEP_HALF).abs() < 1e-6);
            for i in 1..pulses {
                let a = pulse_angle_offset(i, pulses);
                assert!((a - prev - step).abs() < 1e-5, "uneven spacing at {i}");
                prev = a;
            }
            assert!((prev - LASER_SWEEP_HALF).abs() < 1e-6);
        }
        assert_eq!(pulse_angle_offset(0, 1), 0.0);
    }

    /// Per level: the number of pulses, their spacing in time, the beam's
    /// length and the damage each pulse deals.
    #[test]
    fn laser_numbers_per_level() {
        for (level, (pulses, reach, dmg)) in [
            (1u8, (3u32, 34.0f32, 6.0f32)),
            (2, (3, 40.0, 7.0)),
            (3, (4, 40.0, 8.0)),
            (4, (4, 48.0, 9.0)),
            (5, (5, 54.0, 10.0)),
        ] {
            let mut w = World::new(Theme::Sewer, 11);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::LaserEyes, level);
            // The burst fires on the trigger tick, then one pulse per 0.10 s.
            run_laser(&mut w, 1.0 / 60.0);
            let fx = *w.ecs.resource::<LaserFx>();
            assert!(fx.active(), "L{level} burst did not start");
            assert_eq!(fx.pulses, pulses, "L{level} pulse count");
            assert_eq!(fx.reach, reach, "L{level} beam length");
            assert_eq!(fx.dmg, dmg, "L{level} damage per pulse");
            assert_eq!(fx.fired, 1, "the trigger tick resolves the first pulse");
            // …and the burst resolves exactly `pulses` of them.
            let mut ticks = 1;
            while w.ecs.resource::<LaserFx>().active() && ticks < 200 {
                run_laser(&mut w, 1.0 / 60.0);
                ticks += 1;
            }
            assert!(!w.ecs.resource::<LaserFx>().active(), "burst never ended");
            // `0.10 s` per gap at 60 fps is 6-7 ticks per pulse, plus the tail.
            let expected = (pulses - 1) as f32 * LASER_PULSE_INTERVAL + LASER_TAIL;
            let ran = ticks as f32 / 60.0;
            assert!(
                (ran - expected).abs() < 0.12,
                "L{level}: burst ran {ran:.3}s, expected about {expected:.3}s"
            );
        }
    }

    /// The facing is locked at the trigger tick: turning the rat mid-burst
    /// must not drag the sweep.
    #[test]
    fn laser_locks_the_facing_at_burst_start() {
        let mut w = World::new(Theme::Sewer, 12);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        run_laser(&mut w, 1.0 / 60.0);
        let first = *w.ecs.resource::<LaserFx>();
        assert!((first.facing.x - 1.0).abs() < 1e-5 && first.facing.y.abs() < 1e-5);
        w.player.face = Vec2::new(0.0, 1.0);
        run_laser(&mut w, 1.0 / 60.0);
        let after = *w.ecs.resource::<LaserFx>();
        assert!(
            (after.facing.x - 1.0).abs() < 1e-5,
            "the burst followed the rat's turn"
        );
    }

    /// Every exposed body on the ray takes the pulse once, and the beam pierces
    /// the whole line rather than stopping at the first body.
    #[test]
    fn laser_pierces_every_exposed_body_once() {
        let mut w = World::new(Theme::Sewer, 13);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // Three bodies dead ahead inside the 34 px beam, plus two well off the
        // sweep (one beside, one behind the rat).
        let near = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let mid = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(18.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(26.0, 0.0), 1.0);
        let beside = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(10.0, 30.0), 1.0);
        let behind = w.spawn_enemy(EnemyKind::Cat, p - Vec2::new(18.0, 0.0), 1.0);
        let full = [near, mid, far, beside, behind].map(|e| enemy_hp(&w, e));

        for _ in 0..40 {
            if w.ecs.resource::<LaserFx>().fired > 0
                && !w.ecs.resource::<LaserFx>().active()
            {
                break;
            }
            run_laser(&mut w, 1.0 / 60.0);
        }
        // Three pulses, 6 damage each. The big brute 10 px out is wide enough
        // for all three rays (-35°, 0°, +35°); the two cats further down the
        // line are only on the dead-ahead ray.
        assert_eq!(enemy_hp(&w, near), full[0] - 18.0, "near body missed pulses");
        assert_eq!(
            enemy_hp(&w, mid),
            full[1] - 6.0,
            "the beam stopped at the near body"
        );
        assert_eq!(enemy_hp(&w, far), full[2] - 6.0, "the far body was shadowed");
        assert_eq!(enemy_hp(&w, beside), full[3], "a body beside the sweep was hit");
        assert_eq!(enemy_hp(&w, behind), full[4], "a body behind the rat was hit");
    }

    /// A wall stops the beam: nothing past the first face is damaged.
    #[test]
    fn laser_stops_at_a_wall() {
        let mut w = World::new(Theme::Sewer, 14);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 5); // 54 px reach
        w.player.face = Vec2::new(1.0, 0.0);
        let p = w.player.pos;
        // A solid column 16 px ahead of the rat, and a body behind it.
        let cx = (p.x / 16.0) as i32 + 1;
        let cy = (p.y / 16.0) as i32;
        let face = wall_column(&mut w, cx, cy);
        let behind_wall = w.spawn_enemy(EnemyKind::Brute, Vec2::new(face + 14.0, p.y), 1.0);
        let in_front = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(8.0, 0.0), 1.0);
        let behind_hp = enemy_hp(&w, behind_wall);
        let front_hp = enemy_hp(&w, in_front);
        for _ in 0..60 {
            if !w.ecs.resource::<LaserFx>().active() && w.ecs.resource::<LaserFx>().fired > 0 {
                break;
            }
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert!(enemy_hp(&w, in_front) < front_hp, "the near body was shielded");
        assert_eq!(
            enemy_hp(&w, behind_wall),
            behind_hp,
            "the beam reached through the wall"
        );
    }

    /// The weapon never fires through the auto-fire pass: it charges itself and
    /// spawns no bullets at all.
    #[test]
    fn laser_is_not_fired_by_the_auto_fire_pass() {
        let mut w = World::new(Theme::Sewer, 15);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        assert!(w.bullets().is_empty(), "the laser spawned a projectile");
        assert!(w.ecs.resource::<LaserFx>().total == 0.0, "fired outside its system");
    }

    /// Nothing happens when the weapon is not owned: no beam, no state.
    #[test]
    fn laser_does_nothing_when_not_owned() {
        let mut w = World::new(Theme::Sewer, 16);
        w.clear_enemies();
        arm(&mut w, WeaponKind::Gnaw, 1);
        let e = w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 1.0);
        let hp = enemy_hp(&w, e);
        for _ in 0..30 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        assert_eq!(enemy_hp(&w, e), hp);
        assert!(!w.ecs.resource::<LaserFx>().active());
        assert!(w.laser_visual().is_none());
    }

    /// The visual blinks: lit hard just after a pulse, dark before the next.
    #[test]
    fn laser_beacon_blinks_between_pulses() {
        let mut w = World::new(Theme::Sewer, 17);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::LaserEyes, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        run_laser(&mut w, 1.0 / 60.0);
        let on = w.laser_visual().expect("a beam on the pulse tick");
        assert!(on.brightness > 0.9, "the bar should be lit: {on:?}");
        assert!(on.muzzle > 0.0);
        // 0.07 s later (4-5 ticks at 60 fps) it is inside the same interval.
        for _ in 0..4 {
            run_laser(&mut w, 1.0 / 60.0);
        }
        let off = w.laser_visual().expect("the burst is still running");
        assert!(
            off.brightness < 0.5,
            "the bar should have gone dark: {off:?}"
        );
    }

    /// Super Dash: only the travelled segment damages, once per body per dash.
    #[test]
    fn super_dash_hits_only_what_the_dash_crossed() {
        let mut w = World::new(Theme::Sewer, 21);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 0.0;
        let p = w.player.pos;
        let crossed = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 1.0);
        let aside = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 40.0), 1.0);
        let hp_crossed = enemy_hp(&w, crossed);
        let hp_aside = enemy_hp(&w, aside);

        // First dash tick: the press and the movement happen together.
        w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::player::player_system)
            .unwrap();
        w.ecs.run_system_cached(super_dash_system).unwrap();
        w.end_tick();
        assert!(w.player.dash_active > 0.0, "the dash did not start");
        let after_first = enemy_hp(&w, crossed);
        assert!(
            after_first < hp_crossed,
            "the crossed body took no dash damage"
        );
        assert_eq!(enemy_hp(&w, aside), hp_aside, "an un-crossed body was hit");

        // The rest of the dash: the crossed body never takes a second hit.
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
        }
        assert_eq!(
            enemy_hp(&w, crossed),
            after_first,
            "the same body was hit twice by one dash"
        );
        assert!(w.player.dash_active == 0.0, "the dash should be over");
    }

    /// A dash on cooldown (a blocked press) buys no hit, and neither does the
    /// ordinary movement that follows it.
    #[test]
    fn super_dash_buys_nothing_on_a_failed_dash() {
        let mut w = World::new(Theme::Sewer, 22);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 5);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 5.0; // cooling: the press must do nothing
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(4.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
        }
        assert_eq!(w.player.dash_active, 0.0, "the dash should not have started");
        assert_eq!(enemy_hp(&w, body), hp, "a cooling dash dealt damage");
    }

    /// Clean Socks ready the dash; they are not a dash. Collecting them with
    /// Super Dash owned, then walking through a body without pressing anything,
    /// deals nothing and opens no dash segment.
    #[test]
    fn super_dash_does_not_fire_when_clean_socks_ready_the_dash() {
        use crate::game::ecs::resources::DashTrack;
        use crate::game::ecs::systems::loot;
        use crate::game::gem::{Pickup, PickupKind};
        let mut w = World::new(Theme::Sewer, 24);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SuperDash, 5);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        w.player.dash_cd = 1.5;
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(4.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        w.spawn_pickup(Pickup {
            pos: p,
            kind: PickupKind::CleanSocks,
            bob: 0.0,
        });
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(loot::pickups_system).unwrap();
        w.end_tick();
        assert_eq!(w.player.dash_cd, 0.0, "setup: the socks must ready the dash");
        for _ in 0..12 {
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
            assert!(
                w.ecs.resource::<DashTrack>().segment.is_none(),
                "readying the dash opened a dash segment"
            );
        }
        assert_eq!(w.player.dash_active, 0.0, "the socks must not start a dash");
        assert_eq!(enemy_hp(&w, body), hp, "Clean Socks bought a Super Dash hit");
    }

    /// No Super Dash owned, no damage — even through a body.
    #[test]
    fn super_dash_does_nothing_when_not_owned() {
        let mut w = World::new(Theme::Sewer, 23);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::Gnaw, 1);
        w.player.face = Vec2::new(1.0, 0.0);
        w.player.vel = Vec2::new(58.0, 0.0);
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(8.0, 0.0), 1.0);
        let hp = enemy_hp(&w, body);
        let mut ran = false;
        for _ in 0..10 {
            w.player.dash_cd = 0.0;
            w.begin_tick(1.0 / 60.0, Vec2::new(1.0, 0.0), true);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(super_dash_system).unwrap();
            w.end_tick();
            ran |= w.player.dash_active > 0.0;
        }
        assert!(ran, "the dash itself must still work");
        assert_eq!(enemy_hp(&w, body), hp, "an unowned Super Dash dealt damage");
    }

    /// Two runs with the same seed and the same inputs land the same numbers:
    /// the burst and the dash draw rng only through their emitters, in order.
    #[test]
    fn twin_runs_of_the_new_weapons_match() {
        let play = |seed: u64| {
            let mut w = World::new(Theme::Sewer, seed);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            w.player.loadout.weapons = vec![
                Weapon {
                    kind: WeaponKind::LaserEyes,
                    level: 3,
                    timer: 0.0,
                    evolved: false,
                },
                Weapon {
                    kind: WeaponKind::SuperDash,
                    level: 3,
                    timer: 0.0,
                    evolved: false,
                },
            ];
            let p = w.player.pos;
            let mut enemies = Vec::new();
            for i in 0..12 {
                let a = i as f32 * 0.5;
                let at = p + Vec2::from_angle(a) * (8.0 + i as f32 * 3.0);
                enemies.push(w.spawn_enemy(EnemyKind::Cat, at, 4.0));
            }
            for tick in 0..240 {
                let move_dir = if tick < 30 {
                    Vec2::new(1.0, 0.0)
                } else {
                    Vec2::new(0.0, 1.0)
                };
                let dash = tick % 40 == 0;
                w.player.dash_cd = w.player.dash_cd.min(0.0);
                w.begin_tick(1.0 / 60.0, move_dir, dash);
                w.ecs
                    .run_system_cached(crate::game::ecs::systems::player::player_system)
                    .unwrap();
                w.ecs.run_system_cached(laser_eyes_system).unwrap();
                w.ecs.run_system_cached(super_dash_system).unwrap();
                w.end_tick();
            }
            let hps: Vec<u32> = enemies
                .iter()
                .map(|&e| (enemy_hp(&w, e) * 100.0) as u32)
                .collect();
            (hps, w.player.pos.x.to_bits(), w.player.pos.y.to_bits())
        };
        assert_eq!(play(900), play(900));
    }

    /// Every wave-3 passive that claims a weapon actually reaches it: the
    /// audit the design asks for (all eight pre-existing weapons plus the two
    /// new ones).
    #[test]
    fn passive_hooks_reach_every_weapon_they_claim() {
        use crate::game::loadout::{PassiveKind, Upgrade};
        use crate::game::ecs::systems::weapons::active_weapons_system;

        // Lens Cap x4 (+32% ranged / +16% melee): a Gnaw dart's life, a Tail
        // Whip's aura radius, a Spore Orbit ring's radius, and the laser's
        // beam all grow by the hook's multiplier; without the passive they are
        // the plain numbers.
        for lens in [false, true] {
            let mut w = World::new(Theme::Sewer, 41);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            arm(&mut w, WeaponKind::Gnaw, 1);
            if lens {
                for _ in 0..PassiveKind::LensCap.max_stacks() {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LensCap));
                }
            }
            let reach = w.player.loadout.stats.ranged_reach(2.5);
            w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 9.0);
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            let b = w.bullets();
            assert_eq!(b.len(), 1);
            assert!((b[0].life - reach).abs() < 1e-5, "Gnaw reach: {}", b[0].life);
            let asleep = World::new(Theme::Sewer, 41);
            assert_eq!(
                lens,
                reach > asleep.player.loadout.stats.ranged_reach(2.5),
                "Lens Cap must change the reach"
            );
        }

        // Tail Whip's radius (melee reach) and Counterweight's shove.
        for (counter, lens) in [(false, false), (true, true)] {
            let mut w = World::new(Theme::Sewer, 42);
            w.clear_enemies();
            open_map(&mut w);
            arm(&mut w, WeaponKind::TailWhip, 1);
            if counter {
                for _ in 0..PassiveKind::Counterweight.max_stacks() {
                    w.player
                        .loadout
                        .apply(&Upgrade::Passive(PassiveKind::Counterweight));
                }
            }
            for _ in 0..PassiveKind::LensCap.max_stacks() {
                if lens {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LensCap));
                }
            }
            let p = w.player.pos;
            let plain_radius = 26.0 + 3.0;
            // A body just outside the plain aura but inside the stretched one.
            // Just past the plain aura (`plain_radius + cat radius`), inside the
            // Lens Cap one.
            let edge = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(plain_radius + 4.5, 0.0), 9.0);
            let inside = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(plain_radius - 2.0, 0.0), 9.0);
            let edge_hp = enemy_hp(&w, edge);
            let inside_hp = enemy_hp(&w, inside);
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            assert!(enemy_hp(&w, inside) < inside_hp, "the plain aura missed");
            if lens {
                assert!(
                    enemy_hp(&w, edge) < edge_hp,
                    "Lens Cap did not widen the whip"
                );
            } else {
                assert_eq!(enemy_hp(&w, edge), edge_hp, "the plain aura reached too far");
            }
        }

        // Long Fuse x4: the acid pool's splash radius and life.
        for fuse in [false, true] {
            let mut w = World::new(Theme::Sewer, 43);
            w.clear_enemies();
            w.clear_bullets();
            open_map(&mut w);
            arm(&mut w, WeaponKind::AcidSpit, 1);
            if fuse {
                for _ in 0..PassiveKind::LongFuse.max_stacks() {
                    w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
                }
            }
            let lob_radius = w.player.loadout.stats.splash_radius(12.0 + 1.5);
            let pool_life = w.player.loadout.stats.zone_life(2.6);
            w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(10.0, 0.0), 9.0);
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.end_tick();
            let b = w.bullets();
            assert_eq!(b.len(), 1);
            let lob = b[0].lob.expect("acid lob");
            assert!((lob.radius - lob_radius).abs() < 1e-4, "pool radius");
            assert!((lob.life - pool_life).abs() < 1e-4, "pool life");
            if fuse {
                assert!((lob_radius - 13.5 * 1.16).abs() < 1e-4);
                assert!((pool_life - 2.6 * 1.48).abs() < 1e-4);
            }
        }

        // Long Fuse on an orbiter (Spore Orbit's ring lifetime), and
        // Counterweight on the Sonic Squeak's ring shove.
        let mut w = World::new(Theme::Sewer, 44);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SporeOrbit, 1);
        for _ in 0..PassiveKind::LongFuse.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
        }
        let expect_life = w.player.loadout.stats.zone_life(WeaponKind::SporeOrbit.base_cooldown());
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        w.ecs.run_system_cached(fire_weapons_system).unwrap();
        w.end_tick();
        for b in w.bullets() {
            assert!((b.life - expect_life).abs() < 1e-4, "orbiter life {}", b.life);
        }

        // Sonic Squeak: Long Fuse widens the ring, Counterweight pushes harder.
        let mut w = World::new(Theme::Sewer, 45);
        w.clear_enemies();
        open_map(&mut w);
        arm(&mut w, WeaponKind::SonicSqueak, 1);
        for _ in 0..PassiveKind::LongFuse.max_stacks() {
            w.player.loadout.apply(&Upgrade::Passive(PassiveKind::LongFuse));
        }
        for _ in 0..PassiveKind::Counterweight.max_stacks() {
            w.player
                .loadout
                .apply(&Upgrade::Passive(PassiveKind::Counterweight));
        }
        w.player.loadout.weapons[0].timer = 0.0;
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(20.0, 0.0), 40.0);
        let ring = w.player.loadout.stats.splash_radius(90.0);
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
        w.ecs.run_system_cached(active_weapons_system).unwrap();
        w.end_tick();
        assert!((w.active_fx().squeak_max_r - ring).abs() < 1e-4, "ring reach");
        for _ in 0..40 {
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(active_weapons_system).unwrap();
            w.end_tick();
        }
        let vel = w.ecs.get::<Enemy>(body).unwrap().vel.len();
        assert!(vel > 1.0, "the ring did not shove the body");
    }

    /// Second Scent: gem XP is scaled and the fraction is banked across
    /// collections, so four stacks are worth exactly +40% over a run (a `+10%`
    /// that truncated every 1-XP gem would be worth nothing at all).
    #[test]
    fn second_scent_scales_gem_xp_and_banks_the_fraction() {
        use crate::game::ecs::resources::XpBank;
        use crate::game::gem::Gem;
        use crate::game::loadout::{PassiveKind, Upgrade};

        // The bank itself: 1 XP at x1.4 pays 1, 1, 2, 1, 2 (the fraction
        // carries: 0.4, 0.8, then 1.2 pays the extra point and banks 0.2, …),
        // so five 1-XP gems pay 7 and nothing is lost to truncation.
        let mut bank = XpBank::default();
        let paid: Vec<u32> = (0..5).map(|_| bank.grant(1, 1.4)).collect();
        assert_eq!(paid, vec![1, 1, 2, 1, 2], "the bank must not lose XP");
        assert_eq!(paid.iter().sum::<u32>(), 7, "five 1-XP gems at +40%");
        // Identity: no stacks, no change, no bank.
        let mut bank = XpBank::default();
        assert_eq!(bank.grant(3, 1.0), 3);
        assert_eq!(bank.0, 0.0);

        // …and through the real gem pass.
        let collect = |stacks: u32| {
            let mut w = World::new(Theme::Sewer, 51);
            w.clear_enemies();
            open_map(&mut w);
            // No level-ups in the window: this measures XP collected, not the
            // XP curve.
            w.player.xp_to_next = 10_000;
            for _ in 0..stacks {
                w.player
                    .loadout
                    .apply(&Upgrade::Passive(PassiveKind::SecondScent));
            }
            // Ten 1-XP gems, one per tick, all on the rat.
            for _ in 0..10 {
                let p = w.player.pos;
                w.spawn_gem(Gem::new(p, 1));
                w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
                w.ecs
                    .run_system_cached(crate::game::ecs::systems::loot::gems_system)
                    .unwrap();
                w.end_tick();
            }
            w.player.xp
        };
        assert_eq!(collect(0), 10);
        assert_eq!(collect(4), 14, "+40% of ten 1-XP gems");
    }

    /// The melee-nudge log holds the 0.4 s rule: a body is ready again only
    /// after the cooldown has run.
    #[test]
    fn melee_nudge_cooldowns_expire() {
        let mut nudge = MeleeNudge::default();
        assert!(nudge.ready(7));
        nudge.mark(7, MELEE_NUDGE_COOLDOWN);
        assert!(!nudge.ready(7), "the body should be on cooldown");
        assert!(nudge.ready(8), "another body is unaffected");
        nudge.tick(0.39);
        assert!(!nudge.ready(7));
        nudge.tick(0.02);
        assert!(nudge.ready(7), "the cooldown did not expire");
        assert_eq!(nudge.len(), 0);
    }

    /// A body is nudgeable once per 0.4 s from the Tail Whip, whatever the
    /// cooldown maths says — and not at all without the passive.
    #[test]
    fn counterweight_nudges_a_melee_target_once_per_cooldown() {
        let mut w = World::new(Theme::Sewer, 31);
        w.clear_enemies();
        w.clear_bullets();
        open_map(&mut w);
        arm(&mut w, WeaponKind::TailWhip, 1);
        w.player
            .loadout
            .apply(&crate::game::loadout::Upgrade::Passive(
                crate::game::loadout::PassiveKind::Counterweight,
            ));
        let p = w.player.pos;
        let body = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(10.0, 0.0), 20.0);
        let start = w.ecs.get::<Enemy>(body).unwrap().pos;
        // Fire the whip twice inside one 0.4 s window: only the first nudges.
        for _ in 0..2 {
            w.player.loadout.weapons[0].timer = 0.0;
            w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
            w.ecs.run_system_cached(fire_weapons_system).unwrap();
            w.ecs.run_system_cached(melee_nudge_system).unwrap();
            w.end_tick();
        }
        let nudged = w.ecs.get::<Enemy>(body).unwrap().pos;
        let pushed = (nudged - start).len();
        assert!(pushed > 1.5, "the body was not nudged: {pushed}");
        assert!(pushed < 4.0, "the nudge stacked twice: {pushed}");
        assert_eq!(w.ecs.resource::<MeleeNudge>().len(), 1);
    }
}
