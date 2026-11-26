//! The Outfall's guest. One ordinary placement brings a small group, separate
//! from the eight roster slots and their live budget.

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_crab::{CrabState, CrabView, CLAW_HALF, CLAW_REACH, CRAB_RADIUS};
use crate::game::collision::SizeClass;
use crate::game::director::{self, Director};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::{ENEMY_CAP, PLAYER_RADIUS};
use crate::game::ecs::resources::{
    BossDue, CrabSpawner, Cvars, DebugFlags, DifficultyLevel, NextEnemyId, NextSpawnId, RngRes,
    RunCounters, SfxQueue, TickInput, Viewport,
};
use crate::game::enemy::{Enemy, EnemyKind, EnemyMods};
use crate::game::hazards::{clamp_outfall_displacement, outfall_sand_clear};
use crate::game::level::{Level, Theme};
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

use super::boss::{Boss, HazardShape};
use super::miniboss::MinibossSchedule;
use super::projectiles::los_clear;
use super::spawn::{off_screen, spawn_point_ok, spawn_point_reachable, SpawnScratch};

pub const FROM_CLOCK: f32 = 60.0;
const LATE_CLOCK: f32 = 7.0 * 60.0;
const GROUP_GAP: f32 = 25.0;
const PACK_SPACING: f32 = 2.0 * CRAB_RADIUS + 2.0;
const KIND: EnemyKind = EnemyKind::ShoreCrab;

const ENGAGE_RANGE: f32 = CLAW_REACH + 6.0;
const TELL_LOCK: f32 = 0.20;
pub const TELL_SECS: f32 = 0.40;
pub const PINCH_SECS: f32 = 0.15;
pub const RECOVER_SECS: f32 = 0.80;
const PINCH_DAMAGE: f32 = 10.0;
const RECOVER_DAMAGE_MULT: f32 = 1.5;

// This guest uses none of the wave-four rules. Its phase, remaining time and
// strike latch fit in that body's scratch and travel with every &Enemy view.
const TELL: u8 = 1;
const PINCH: u8 = 2;
const RECOVER: u8 = 3;

fn live_cap(curve: f32) -> usize {
    if curve >= LATE_CLOCK { 9 } else { 6 }
}

fn group_room(curve: f32, elapsed: f32, last_change: Option<f32>, live: usize) -> usize {
    if curve < FROM_CLOCK || last_change.is_some_and(|last| elapsed - last < GROUP_GAP) {
        return 0;
    }
    live_cap(curve).saturating_sub(live).min(3)
}

fn fixed_crab(at: Vec2, rat: Vec2, hp_mult: f32, mods: EnemyMods, anim: f32) -> Enemy {
    let mut crab = Enemy::spawn_with_phases(KIND, at, hp_mult, 1.0, 0.0, anim);
    crab.speed_scale = mods.speed;
    crab.damage = KIND.damage() * mods.damage;
    crab.facing = (rat - at).normalized();
    crab
}

/// Builds an ordinary crab for an empty pot to release. Pass a candidate on
/// the door's approach side. The caller supplies the current crab count
/// including queued releases, the same-tick boss/miniboss/suppress gate, and
/// reserves room under the hard entity cap before assigning both spawn IDs
/// and queuing the body. A refused candidate spends no IDs or RNG.
#[allow(clippy::too_many_arguments)]
pub fn release_pot_crab(
    level: &Level,
    at: Vec2,
    rat: Vec2,
    difficulty: usize,
    elapsed: f32,
    live_crabs: usize,
    blocked: bool,
) -> Option<Enemy> {
    let clear_rat = PLAYER_RADIUS + CRAB_RADIUS;
    if level.theme != Theme::Outfall
        || blocked
        || elapsed >= director::BOSS_TIME
        || live_crabs >= live_cap(director::curve_clock(difficulty, elapsed))
        || at.dist_sq(rat) <= clear_rat * clear_rat
        || !outfall_sand_clear(level, at, CRAB_RADIUS, SizeClass::of_enemy(KIND))
    {
        return None;
    }
    Some(fixed_crab(
        at,
        rat,
        director::horde_hp_mult_for(difficulty, elapsed),
        director::enemy_mods_for(difficulty, elapsed),
        0.0,
    ))
}

/// Companions must fit beside the leader. Preserve the ladder's visibility
/// rung, reject isolated pockets, and skip the whole group if no pair fits.
fn group_positions(
    level: &Level,
    leader: Vec2,
    rat: Vec2,
    viewport: Vec2,
    room: usize,
) -> ([Vec2; 3], usize) {
    let mut points = [leader; 3];
    let mut count = 1;
    let wide = off_screen(leader, rat, viewport);
    let normal = off_screen(leader, rat, viewport * 0.5);
    for dir in [
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(-1.0, 0.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, -1.0),
    ] {
        let at = leader + dir * PACK_SPACING;
        if (wide && !off_screen(at, rat, viewport))
            || (normal && !off_screen(at, rat, viewport * 0.5))
            || !spawn_point_ok(
                at, Vec2::splat(CRAB_RADIUS), level.arena - Vec2::splat(CRAB_RADIUS),
                CRAB_RADIUS + PLAYER_RADIUS, CRAB_RADIUS, level, rat,
            )
            || points[..count].iter().any(|&p| p.dist_sq(at) < PACK_SPACING * PACK_SPACING)
            || clamp_outfall_displacement(
                level, leader, at - leader, CRAB_RADIUS, SizeClass::of_enemy(KIND),
            ).dist_sq(at) > 1e-4
        {
            continue;
        }
        points[count] = at;
        count += 1;
        if count == room {
            break;
        }
    }
    (points, count)
}

/// Runs after the appointments and boss hand-off, before the first grid.
#[allow(clippy::too_many_arguments)]
pub fn spawn_crabs_system(
    mut commands: Commands,
    level: Res<Level>,
    player: Res<Player>,
    viewport: Res<Viewport>,
    counters: Res<RunCounters>,
    difficulty: Res<DifficultyLevel>,
    spawn_rules: (
        Res<DebugFlags>,
        Res<Cvars>,
        Res<Director>,
        Res<BossDue>,
        Res<MinibossSchedule>,
    ),
    enemies: Query<(Entity, &Enemy)>,
    bosses: Query<&Boss>,
    mut spawner: ResMut<CrabSpawner>,
    mut scratch: Local<SpawnScratch>,
    mut rng: ResMut<RngRes>,
    ids: (ResMut<NextSpawnId>, ResMut<NextEnemyId>),
) {
    if level.theme != Theme::Outfall {
        return;
    }
    let elapsed = counters.elapsed;
    // Compare identities so a release from a pot earlier this tick cannot
    // hide the loss of a crab that was alive on the previous tick.
    if spawner.live.iter().any(|&entity| {
        !enemies.get(entity).is_ok_and(|(_, e)| e.kind == KIND && e.alive())
    }) {
        spawner.last_change = Some(elapsed);
    }
    spawner.live.clear();
    spawner.live.extend(
        enemies.iter().filter(|(_, e)| e.kind == KIND && e.alive()).map(|(id, _)| id),
    );

    let curve = director::curve_clock(difficulty.0, elapsed);
    let room = group_room(curve, elapsed, spawner.last_change, spawner.live.len());
    let (debug, cvars, director, due, appointments) = spawn_rules;
    if room < 2 {
        return;
    }
    if debug.peace
        || appointments.suppress(elapsed)
        || elapsed >= director::BOSS_TIME
        || director.boss_spawned
        || due.due
        || due.claimed
        || !bosses.is_empty()
        || enemies.iter().any(|(_, e)| e.named && e.alive())
    {
        return;
    }
    let hard_cap = if cvars.enemy_cap == 0 {
        ENEMY_CAP
    } else {
        cvars.enemy_cap
    };
    let hard_room = hard_cap.saturating_sub(1 + enemies.iter().count());
    let room = room.min(hard_room);
    if room < 2 {
        return;
    }
    let Some(leader) = spawn_point_reachable(&mut rng.0, &level, &player, viewport.0, &mut scratch)
    else {
        return;
    };
    if !outfall_sand_clear(&level, leader, CRAB_RADIUS, SizeClass::of_enemy(KIND)) {
        return;
    }
    let (points, count) = group_positions(&level, leader, player.pos, viewport.0, room);
    if count < 2 {
        return;
    }
    let hp_mult = director::horde_hp_mult_for(difficulty.0, elapsed);
    let mods = director::enemy_mods_for(difficulty.0, elapsed);
    let (mut next_spawn, mut next_enemy) = ids;
    for (i, &at) in points[..count].iter().enumerate() {
        let mut crab = if i == 0 {
            let mut leader = Enemy::spawn_with_mods(KIND, at, hp_mult, mods, &mut rng.0);
            leader.facing = (player.pos - at).normalized();
            leader
        } else {
            fixed_crab(at, player.pos, hp_mult, mods, i as f32 * 2.0)
        };
        crab.id = next_enemy.take();
        spawner.live.push(commands.spawn((crab, next_spawn.take())).id());
    }
    spawner.last_change = Some(elapsed);
}

/// Apply the crab's sideways gait in the ordinary steering slot.
/// The flow direction breaks the tie when the rat is exactly on the facing
/// axis, and supplies a route around walls when the claws face across one.
pub(crate) fn scuttle(e: &mut Enemy, level: &Level, target: Vec2, flow: Vec2, dt: f32, speed: f32) {
    let to = target - e.pos;
    let distance = to.len();
    let direct = to.normalized();
    if direct == Vec2::ZERO {
        e.vel = Vec2::ZERO;
        return;
    }
    let tracked = (e.facing + (direct - e.facing) * (dt * 10.0).min(1.0)).normalized();
    e.facing = if tracked.dot(direct) >= 0.65 { tracked } else { direct };
    if can_engage(e, target, level) {
        e.vel = Vec2::ZERO;
        return;
    }

    let side = e.facing.perp();
    let lateral_error = direct.dot(side);
    let across = if lateral_error.abs() > 0.015 {
        lateral_error
    } else {
        flow.dot(side)
    };
    let side = if across.abs() > 0.05 {
        side * across.signum()
    } else if e.vel.dot(side) < 0.0 {
        side * -1.0
    } else {
        side
    };
    let forward = e.facing * (distance / 70.0).clamp(0.35, 0.75);
    let start = e.pos;
    let class = SizeClass::of_enemy(KIND);
    for dir in [side + forward, forward - side, flow, direct] {
        let velocity = dir.normalized() * speed;
        let step = velocity * dt;
        let end = if level.theme == Theme::Outfall {
            clamp_outfall_displacement(level, start, step, e.radius, class)
        } else {
            level.tiles.resolve_circle(start + step, e.radius, class, None)
        };
        if step.len_sq() > 0.0 && end.dist_sq(start + step) <= step.len_sq() * 0.01 {
            e.vel = velocity;
            e.pos = end;
            e.anim += dt * (4.0 + velocity.len() * 0.5);
            return;
        }
    }
    e.vel = Vec2::ZERO;
}

fn can_engage(e: &Enemy, target: Vec2, level: &Level) -> bool {
    e.pos.dist_sq(target) <= ENGAGE_RANGE * ENGAGE_RANGE && los_clear(level, e.pos, target, 0.0)
}

fn face(e: &mut Enemy, target: Vec2) {
    let facing = (target - e.pos).normalized();
    if facing != Vec2::ZERO {
        e.facing = facing;
    }
}

fn claw_box(e: &Enemy) -> HazardShape {
    let half = CLAW_REACH * 0.5;
    HazardShape::Rect {
        center: e.pos + e.facing * half,
        facing: e.facing,
        half,
        half_perp: CLAW_HALF,
    }
}

fn pinch_damage(e: &Enemy) -> f32 {
    PINCH_DAMAGE * e.damage / KIND.damage()
}

/// The exact telegraphed box, seconds until activation/expiry, and damage.
pub fn claw_warning(e: &Enemy) -> Option<(HazardShape, f32, f32, f32)> {
    if e.kind != KIND || !e.targetable() {
        return None;
    }
    let (warn, end) = match e.w4.phase {
        TELL => (e.w4.t, e.w4.t + PINCH_SECS),
        PINCH => (0.0, e.w4.t),
        _ => return None,
    };
    Some((claw_box(e), warn, end, pinch_damage(e)))
}

/// Predict a pending claw strike for the batch damage attribution.
pub fn pending_pinch(e: &Enemy, at: Vec2, soon: f32, slack: f32) -> Option<f32> {
    let (shape, warn, _, damage) = claw_warning(e)?;
    (e.w4.role == 0 && warn <= soon && shape.contains(at, slack)).then_some(damage)
}

pub fn incoming_damage_mult(e: &Enemy) -> f32 {
    if e.w4.phase == RECOVER {
        RECOVER_DAMAGE_MULT
    } else {
        1.0
    }
}

/// Pose and geometry inputs shared by the body, crowd LOD, and hostile pass.
pub fn crab_view(e: &Enemy) -> CrabView {
    let (state, duration) = match e.w4.phase {
        TELL => (CrabState::Tell, TELL_SECS),
        PINCH => (CrabState::Pinch, PINCH_SECS),
        RECOVER => (CrabState::Recover, RECOVER_SECS),
        _ => (CrabState::Scuttle, 1.0),
    };
    CrabView {
        pos: e.pos,
        facing: e.facing,
        state,
        progress: if state == CrabState::Scuttle {
            0.0
        } else {
            (1.0 - e.w4.t / duration).clamp(0.0, 1.0)
        },
        anim: e.anim,
        hurt: (e.flash / 0.12).clamp(0.0, 1.0),
        elite: e.elite,
    }
}

/// Drop a crab's committed attack when another mechanic pins or stuns it.
pub(super) fn cancel_attack(e: &mut Enemy) {
    if e.w4.phase != 0 {
        e.w4.phase = 0;
        e.w4.t = 0.0;
        e.w4.role = 0;
        e.position_locked = false;
    }
}

fn tick_attack(
    e: &mut Enemy, player: &mut Player, level: &Level, target: Vec2, dt: f32,
    sfx: &mut SfxQueue,
) -> bool {
    if e.crab_pot_held || e.stun > 0.0 {
        cancel_attack(e);
        return false;
    }
    if e.w4.phase == 0 {
        if !e.position_locked && player.alive() && can_engage(e, target, level) {
            face(e, target);
            e.vel = Vec2::ZERO;
            e.position_locked = true;
            e.w4.phase = TELL;
            e.w4.t = TELL_SECS;
            e.w4.role = 0;
        }
        return false;
    }
    if e.w4.phase == TELL && e.w4.t > TELL_SECS - TELL_LOCK + 1e-6 {
        face(e, target);
    }

    e.w4.t -= dt;
    if e.w4.phase == TELL && e.w4.t <= 1e-6 {
        e.w4.t += PINCH_SECS;
        e.w4.phase = PINCH;
        sfx.0.push(Sfx::CrabShell);
    }
    let mut landed = false;
    if e.w4.phase == PINCH {
        if e.w4.role == 0
            && player.alive()
            && claw_box(e).contains(player.pos, player.buffs.point_hit_radius())
        {
            e.w4.role = 1;
            landed = player.take_damage(pinch_damage(e));
            if landed {
                sfx.0.push(Sfx::Hurt);
            }
        }
        if e.w4.t <= 1e-6 {
            e.w4.t += RECOVER_SECS;
            e.w4.phase = RECOVER;
        }
    }
    if e.w4.phase == RECOVER && e.w4.t <= 1e-6 {
        cancel_attack(e);
    }
    landed
}

/// Advance the attack state after the damage/contact passes, like the Drake.
#[allow(clippy::too_many_arguments)]
pub fn crab_system(
    input: Res<TickInput>,
    level: Res<Level>,
    debug: Res<DebugFlags>,
    mut player: ResMut<Player>,
    mut sfx: ResMut<SfxQueue>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<RngRes>,
    decoys: Query<(&SpawnId, &super::powerups::Decoy)>,
    mut enemies: Query<(&SpawnId, &mut Enemy)>,
) {
    if debug.freeze || !enemies.iter().any(|(_, e)| e.kind == KIND) {
        return;
    }
    let rat = player.pos;
    let decoy = decoys.iter().filter(|(_, d)| d.live()).max_by_key(|(id, _)| id.0);
    for (_, mut e) in enemies.iter_mut().sort_by_key::<&SpawnId, _>(|id| id.0) {
        if e.kind != KIND || !e.targetable() {
            continue;
        }
        let target = decoy
            .filter(|(_, d)| super::powerups::decoy_lures_body(&e)
                && e.pos.dist_sq(d.pos) <= super::powerups::WINDUP_MOUSE_RADIUS.powi(2)
                && los_clear(&level, e.pos, d.pos, e.radius))
            .map_or(rat, |(_, d)| d.pos);
        if tick_attack(&mut e, &mut player, &level, target, input.dt, &mut sfx) {
            vfx::emit_player_hurt(&mut particles, &mut rng.0, rat);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision::TileMap;
    use crate::game::level::{MAP_COLS, MAP_ROWS};
    use crate::game::world::World;
    use rand::{rngs::StdRng, RngExt, SeedableRng};

    const DT: f32 = 1.0 / 60.0;
    const EAST: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    const NORTH: Vec2 = Vec2 { x: 0.0, y: -1.0 };

    fn open_world(theme: Theme) -> World {
        let mut w = World::new(theme, 41);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, theme, 0);
        w.level.mazes.clear();
        w.level.objects.tidal_pits.clear();
        w.player.pos = Vec2::new(400.0, 400.0);
        w.viewport = Vec2::new(120.0, 80.0);
        w
    }

    fn spawn_tick(w: &mut World, elapsed: f32) {
        w.elapsed = elapsed;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(spawn_crabs_system).unwrap();
        w.end_tick();
    }

    fn fight_tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(crab_system).unwrap();
        w.end_tick();
    }

    fn crab_count(w: &mut World) -> usize {
        w.enemies().iter().filter(|e| e.kind == KIND && e.alive()).count()
    }

    #[test]
    fn groups_obey_the_first_minute_gap_and_both_live_caps() {
        let mut w = open_world(Theme::Outfall);
        for (time, count) in [
            (59.99, 0), (60.0, 3), (84.99, 3), (85.0, 6), (419.99, 6), (420.0, 9),
            (445.0, 9),
        ] {
            spawn_tick(&mut w, time);
            assert_eq!(crab_count(&mut w), count, "at {time}");
        }
        assert_eq!(group_room(60.0, 60.0, None, 4), 2);
        assert_eq!(group_room(60.0, 60.0, None, 5), 1, "insufficient room for a group");
    }

    #[test]
    fn a_death_restarts_the_gap_even_when_replaced_by_a_pot_crab() {
        let mut w = open_world(Theme::Outfall);
        let pos = w.player.pos + EAST * 30.0;
        let lost = w.spawn_enemy(KIND, pos, 1.0);
        spawn_tick(&mut w, 59.0);
        w.ecs.despawn(lost);
        w.spawn_enemy(KIND, pos, 1.0);
        spawn_tick(&mut w, 60.0);
        assert_eq!(crab_count(&mut w), 1);
        spawn_tick(&mut w, 84.99);
        assert_eq!(crab_count(&mut w), 1);
        spawn_tick(&mut w, 85.0);
        assert_eq!(crab_count(&mut w), 4);
    }

    #[test]
    fn no_group_at_boss_time_or_beside_a_miniboss_and_no_draw_when_blocked() {
        use super::super::miniboss::MinibossKind;
        let mut w = open_world(Theme::Outfall);
        let pos = w.player.pos + EAST * 50.0;
        let named = w.spawn_miniboss(MinibossKind::Gasket, 0, pos);
        w.rng = StdRng::seed_from_u64(23);
        let mut expected = StdRng::seed_from_u64(23);
        spawn_tick(&mut w, 181.0);
        assert_eq!(crab_count(&mut w), 0);
        w.ecs.despawn(named);
        w.ecs.resource_mut::<BossDue>().due = true;
        spawn_tick(&mut w, 190.0);
        assert_eq!(crab_count(&mut w), 0);
        w.ecs.resource_mut::<BossDue>().due = false;
        spawn_tick(&mut w, director::BOSS_TIME);
        assert_eq!(crab_count(&mut w), 0);
        assert_eq!(w.rng.random::<u64>(), expected.random::<u64>());
    }

    #[test]
    fn one_group_draws_only_one_ordinary_placement_and_initialisation() {
        let mut w = open_world(Theme::Outfall);
        w.rng = StdRng::seed_from_u64(23);
        let mut expected = StdRng::seed_from_u64(23);
        let center = spawn_point_reachable(
            &mut expected, &w.level, &w.player, w.viewport, &mut SpawnScratch::default(),
        ).unwrap();
        let _ = Enemy::spawn(KIND, center, 1.0, &mut expected);

        spawn_tick(&mut w, FROM_CLOCK);
        let crabs = w.enemies();
        assert_eq!(crabs.len(), 3);
        assert_eq!(crabs[0].pos, center);
        assert_eq!(w.rng.random::<u64>(), expected.random::<u64>());
        for (i, e) in crabs.iter().enumerate() {
            assert!(off_screen(e.pos, w.player.pos, w.viewport));
            assert!(outfall_sand_clear(&w.level, e.pos, e.radius, SizeClass::of_enemy(KIND)));
            assert!(crabs[..i].iter().all(|other| other.pos.dist(e.pos) >= PACK_SPACING));
        }
    }

    #[test]
    fn the_pot_release_checks_ground_contact_and_the_guest_cap() {
        let w = open_world(Theme::Outfall);
        let rat = w.player.pos;
        let at = rat + EAST * 20.0;
        let body = release_pot_crab(&w.level, at, rat, 0, 70.0, 5, false).unwrap();
        assert_eq!((body.kind, body.pos, body.elite), (KIND, at, false));
        assert!(release_pot_crab(&w.level, at, rat, 0, 70.0, 6, false).is_none());
        assert!(release_pot_crab(&w.level, at, rat, 0, 70.0, 0, true).is_none());
        assert!(release_pot_crab(&w.level, rat, rat, 0, 70.0, 0, false).is_none());
        assert!(release_pot_crab(&w.level, at, rat, 0, director::BOSS_TIME, 0, false).is_none());
    }

    #[test]
    fn scuttle_closes_sideways_while_its_facing_tracks_the_rat() {
        let w = open_world(Theme::Outfall);
        let start = w.player.pos - EAST * 65.0;
        let mut body = Enemy::spawn_fixed(KIND, start, 1.0);
        body.steer_crab(&w.level, w.player.pos, NORTH, DT, 1.0);
        assert_eq!(body.facing, EAST);
        assert!((body.vel.len() - 46.0).abs() < 1e-4);
        assert!(body.vel.dot(NORTH) > body.vel.dot(EAST));
        assert!(body.pos.dist(w.player.pos) < start.dist(w.player.pos));
    }

    #[test]
    fn tell_locks_halfway_pinch_hits_once_and_recovery_is_exposed() {
        let mut w = open_world(Theme::Outfall);
        let anchor = w.player.pos - EAST * 13.0;
        let crab = w.spawn_enemy(KIND, anchor, 1.0);
        let hp = w.player.hp;
        fight_tick(&mut w);
        assert_eq!(crab_view(w.ecs.get::<Enemy>(crab).unwrap()).state, CrabState::Tell);

        w.player.pos = anchor + NORTH * 13.0;
        for _ in 0..12 {
            fight_tick(&mut w);
        }
        assert_eq!(w.ecs.get::<Enemy>(crab).unwrap().facing, NORTH);
        // Both endpoints of the box count. Leave its side after the lock,
        // then re-enter during Pinch to exercise the whole active window.
        w.player.pos = anchor + NORTH * 13.0 + EAST * (CLAW_HALF + 1.0);
        for _ in 0..11 {
            fight_tick(&mut w);
        }
        assert_eq!(crab_view(w.ecs.get::<Enemy>(crab).unwrap()).state, CrabState::Tell);
        fight_tick(&mut w);
        let body = w.ecs.get::<Enemy>(crab).unwrap();
        assert_eq!(body.pos, anchor);
        assert_eq!(body.facing, NORTH);
        assert_eq!(crab_view(body).state, CrabState::Pinch);
        let box_shape = claw_box(body);
        assert!(box_shape.contains(anchor, 0.0));
        assert!(box_shape.contains(anchor + NORTH * CLAW_REACH + EAST * CLAW_HALF, 0.0));
        assert!(!box_shape.contains(anchor - NORTH, 0.0));
        assert!(!box_shape.contains(anchor + NORTH * (CLAW_REACH + 1.0), 0.0));
        assert_eq!(w.player.hp, hp);

        w.player.pos = anchor + NORTH * 13.0;
        fight_tick(&mut w);
        assert_eq!(w.player.hp, hp - PINCH_DAMAGE);
        for _ in 0..8 {
            w.player.iframes = 0.0;
            fight_tick(&mut w);
        }
        assert_eq!(w.player.hp, hp - PINCH_DAMAGE);
        let body = w.ecs.get::<Enemy>(crab).unwrap().clone();
        assert_eq!(crab_view(&body).state, CrabState::Recover);
        assert!(body.position_locked);
        assert_eq!(body.damage_mult_from(anchor), 1.5);
        let mut hit = body;
        hit.hurt(4.0);
        assert_eq!(hit.hp, hit.max_hp - 6.0);
        for _ in 0..47 {
            fight_tick(&mut w);
        }
        assert!(w.ecs.get::<Enemy>(crab).unwrap().position_locked);
        fight_tick(&mut w);
        let body = w.ecs.get::<Enemy>(crab).unwrap();
        assert_eq!(crab_view(body).state, CrabState::Scuttle);
        assert!(!body.position_locked);
    }

    #[test]
    fn a_shore_crab_can_pinch_the_edge_of_a_giant() {
        use crate::game::ecs::systems::powerups::GROWTH_SECS;

        for giant in [false, true] {
            let mut w = open_world(Theme::Outfall);
            let start = w.player.pos;
            let crab = w.spawn_enemy(KIND, start, 1.0);
            {
                let mut body = w.ecs.get_mut::<Enemy>(crab).unwrap();
                body.facing = NORTH;
                body.w4.phase = PINCH;
                body.w4.t = PINCH_SECS;
            }
            w.player.pos = start + NORTH * (CLAW_REACH + 10.0);
            if giant {
                w.player.buffs.giant = GROWTH_SECS - 1.0;
            }
            let hp = w.player.hp;
            fight_tick(&mut w);
            assert_eq!(w.player.hp < hp, giant);
        }
    }

    #[test]
    fn the_reference_floors_cannot_spawn_the_outfall_guest() {
        for theme in [Theme::Sewer, Theme::Kitchen, Theme::Lab] {
            let mut w = open_world(theme);
            spawn_tick(&mut w, FROM_CLOCK);
            spawn_tick(&mut w, LATE_CLOCK);
            assert_eq!(crab_count(&mut w), 0);
            assert_eq!(w.ecs.resource::<CrabSpawner>().last_change, None);
        }
    }

    #[test]
    fn the_crab_is_a_ground_guest_with_fodder_glue_weight() {
        assert_eq!(KIND.archetype(), EnemyKind::Skitterer);
        assert_eq!(SizeClass::of_enemy(KIND), SizeClass::Large);
        assert_eq!(super::super::traps::enemy_units(KIND), 1);
        assert!(!KIND.is_registration());
        assert!(!KIND.heavy());
        assert!(!KIND.ranged());
        for theme in Theme::playable() {
            assert!(crate::game::roster::floor(theme).slot_of(KIND).is_none());
        }
    }
}
