//! (b) Spawn director, spawn ring, elite + boss spawns, enemy cap.
//!
//! Reproduce **verbatim**:
//! - `World::run_director`.
//! - `World::spawn_point`.
//! - `World::spawn_point_ok`.
//! - `Enemy::spawn` (`src/game/enemy.rs`) is the only constructor; it draws
//!   `fire_cd`, `wander` and `anim` from the rng in that order.
//!
//! Resources: `Res<TickInput>` (dt), `Res<RunCounters>` (elapsed),
//! `Res<Viewport>` (spawn ring radius), `Res<Level>`, `Res<Player>`,
//! `ResMut<Director>`, `ResMut<RngRes>`, `ResMut<NextSpawnId>`,
//! `ResMut<Shake>` + `ResMut<SfxQueue>` (boss shake/sfx), `Query<&Enemy>` for
//! the live count, `Commands` to spawn.
//!
//! Ordering: after `player_system` + `flow_refresh_system`, before enemy
//! steering (the legacy `update()` order). `.chain()` inserts an
//! `ApplyDeferred` after this system, so enemies spawned here steer on the same
//! tick, exactly like the legacy `self.enemies.push` inside `run_director`.
//!
//! **Enemy cap:** `ENEMY_CAP` is 600 and one slot is reserved until
//! `director.boss_spawned`. Because `Commands` defers spawns until the end of
//! the system, count the query **and** add your own running count for each
//! queued spawn; simply re-querying `enemies.len()` inside the pulse loop would
//! over-spawn.
//!
//! Every spawned enemy is `commands.spawn((enemy, next_id.take()))` so it gets
//! a monotonic `SpawnId` (see `components::SpawnId`). Spawn order inside a
//! pulse is the legacy loop order: one `pick_kind` draw, one `spawn_point`
//! (which draws from the same rng), then `Enemy::spawn`.
//!
//! ## Port notes
//!
//! The only structural change is the cap bookkeeping above: legacy asked
//! `self.enemies.len()` after each push (the `Vec` grew immediately), the ECS
//! port asks the query once and adds `queued`. Rng draws, their order and the
//! spawn order are unchanged — `spawn_point` draws its ring angle before
//! `Enemy::spawn` draws its three fields for every single spawn, and the elite
//! and boss branches keep consuming the same draws. `shake = 14.0` is an
//! assignment (not a `max`) exactly like the legacy code.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::rngs::StdRng;
use rand::RngExt;

use crate::audio::Sfx;
use crate::game::director::{self, Director};
use crate::game::ecs::constants::ENEMY_CAP;
use crate::game::ecs::resources::{
    NextEnemyId, NextSpawnId, RngRes, RunCounters, Shake, SfxQueue, TickInput, Viewport,
};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::particle::{vfx, Particles};
use crate::game::player::Player;
use crate::math::Vec2;

/// Legacy `World::run_director` .
pub fn director_system(
    mut commands: Commands,
    input: Res<TickInput>,
    counters: Res<RunCounters>,
    viewport: Res<Viewport>,
    level: Res<Level>,
    player: Res<Player>,
    mut director: ResMut<Director>,
    mut rng: ResMut<RngRes>,
    mut next_id: ResMut<NextSpawnId>,
    mut next_enemy_id: ResMut<NextEnemyId>,
    mut particles: ResMut<Particles>,
    mut shake: ResMut<Shake>,
    mut sfx: ResMut<SfxQueue>,
    enemies: Query<&Enemy>,
) {
    let dt = input.dt;
    let elapsed = counters.elapsed;
    // Keep one slot reserved for the mandatory boss until it has spawned, so
    // the live count stays `<= ENEMY_CAP` for the whole run.
    let cap = if director.boss_spawned {
        ENEMY_CAP
    } else {
        ENEMY_CAP - 1
    };
    // Legacy `self.enemies.len()` is the *live* length, which grows with every
    // push inside the pulse. `Commands` defers those pushes, so mirror the
    // growth locally: live count + spawns queued by this system.
    //
    // The count is only needed on ticks that actually spawn, so it is sampled
    // lazily inside each branch (no per-tick query work when the timer is
    // still running down). `enemies` cannot see this system's own deferred
    // spawns, so re-sampling in the elite branch yields the same value the
    // pulse branch saw.
    let mut queued = 0usize;

    director.spawn_timer -= dt;
    if director.spawn_timer <= 0.0 {
        director.spawn_timer = director::spawn_interval(elapsed);
        let count = director::pulse_count(elapsed);
        let mult = director::hp_mult(elapsed);
        let live = enemies.iter().count();
        for _ in 0..count {
            if live + queued >= cap {
                break;
            }
            let kind = director::pick_kind(elapsed, &mut rng.0);
            let pos = spawn_point(&mut rng.0, &level, &player, viewport.0);
            let mut enemy = Enemy::spawn(kind, pos, mult, &mut rng.0);
            // Neon merge: legacy `run_director` ids every director-spawned
            // enemy from the monotonic `next_enemy_id` counter (0 is reserved
            // for "anonymous enemy" in `DamageNumbers`). `SpawnId` and the
            // enemy id are two independent orderings; never share them.
            enemy.id = next_enemy_id.take();
            commands.spawn((enemy, next_id.take()));
            queued += 1;
        }
    }

    // Periodic elite brute. Skip it instead of taking the boss's slot.
    director.elite_timer -= dt;
    if director.elite_timer <= 0.0 && elapsed > director::ELITE_START {
        director.elite_timer = director::ELITE_INTERVAL;
        if enemies.iter().count() + queued < cap {
            let pos = spawn_point(&mut rng.0, &level, &player, viewport.0);
            let mult = director::hp_mult(elapsed) * 1.6;
            let mut enemy = Enemy::spawn(EnemyKind::Brute, pos, mult, &mut rng.0);
            enemy.id = next_enemy_id.take();
            // Director elites are flagged so the art and death VFX can tell
            // them apart from late-game Brutes with overlapping scaled HP.
            enemy.elite = true;
            commands.spawn((enemy, next_id.take()));
        }
    }

    // Boss.
    if !director.boss_spawned && elapsed >= director::BOSS_TIME {
        director.boss_spawned = true;
        let pos = spawn_point(&mut rng.0, &level, &player, viewport.0);
        let mut enemy = Enemy::spawn(EnemyKind::Boss, pos, 1.0, &mut rng.0);
        enemy.id = next_enemy_id.take();
        commands.spawn((enemy, next_id.take()));
        // Neon merge: arrival shockwave + cross burst, drawn before the shake
        // and SFX exactly like `run_director` in the merged reference.
        let accent = level.palette.accent;
        vfx::emit_boss_spawn(&mut particles, &mut rng.0, pos, accent);
        shake.amount = 14.0;
        sfx.0.push(Sfx::Boss);
    }
}

/// `World::spawn_point` . Also used by the facade's
/// `World::spawn_point` test hook.
///
/// A point on a ring around the player, just beyond view. Rejection samples so
/// the point stays inside the arena margin, far enough from the player, and
/// clear of level walls; clamping is only a last resort.
pub fn spawn_point(rng: &mut StdRng, level: &Level, player: &Player, viewport: Vec2) -> Vec2 {
    const ATTEMPTS: i32 = 12;
    const MARGIN: f32 = 8.0;
    const MIN_DIST: f32 = 90.0;
    const WALL_CLEAR: f32 = 4.0;

    let r = (viewport.len() * 0.5 + 16.0).max(120.0);
    let base = rng.random_range(0.0..std::f32::consts::TAU);
    let lo = Vec2::splat(MARGIN);
    let hi = level.arena - Vec2::splat(MARGIN);
    for k in 0..ATTEMPTS {
        let a = base + k as f32 / ATTEMPTS as f32 * std::f32::consts::TAU;
        let p = player.pos + Vec2::from_angle(a) * r;
        if spawn_point_ok(p, lo, hi, MIN_DIST, WALL_CLEAR, level, player.pos) {
            return p;
        }
    }
    // Mirror of the first angle, as a cheap second sweep.
    let p = player.pos + Vec2::from_angle(-base) * r;
    if spawn_point_ok(p, lo, hi, MIN_DIST, WALL_CLEAR, level, player.pos) {
        return p;
    }
    // Last resort: clamp the original ring point into the arena.
    (player.pos + Vec2::from_angle(base) * r)
        .clamp_box(Vec2::splat(6.0), level.arena - Vec2::splat(6.0))
}

/// `World::spawn_point_ok` .
pub fn spawn_point_ok(
    p: Vec2,
    lo: Vec2,
    hi: Vec2,
    min_dist: f32,
    wall_clear: f32,
    level: &Level,
    player_pos: Vec2,
) -> bool {
    if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
        return false;
    }
    if p.dist_sq(player_pos) < min_dist * min_dist {
        return false;
    }
    for w in &level.walls {
        let cx = p.x.clamp(w.x, w.right());
        let cy = p.y.clamp(w.y, w.bottom());
        if (p.x - cx) * (p.x - cx) + (p.y - cy) * (p.y - cy) < wall_clear * wall_clear {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use super::{director_system, spawn_point};
    use crate::audio::Sfx;
    use crate::game::director::{self, Director};
    use crate::game::ecs::constants::{ENEMY_CAP, PARTICLE_CAP};
    use crate::game::enemy::{Enemy, EnemyKind};
    use crate::game::level::{Level, Rectf, Theme};
    use crate::game::particle::{vfx, Particles};
    use crate::game::player::Player;
    use crate::game::weapon::WeaponKind;
    use crate::game::world::World;
    use crate::math::Vec2;

    /// Drive exactly one director tick at a chosen run clock, exactly the way
    /// the real schedule does (`World::update` sets `elapsed` before
    /// `begin_tick`).
    fn director_tick(w: &mut World, dt: f32, elapsed: f32) {
        w.elapsed = elapsed;
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(director_system)
            .expect("director system must run");
        w.end_tick();
    }

    /// Legacy `spawn_point_stays_in_bounds_clear_of_walls_and_player`: the ring
    /// stays inside the 8 px margin, at least 90 px from the player and 4 px
    /// clear of a wall.
    #[test]
    fn spawn_point_respects_margins_distance_and_walls() {
        let mut level = Level::new(Theme::Sewer);
        let centre = level.arena * 0.5;
        let player = Player::new(centre, WeaponKind::Gnaw);
        level.walls = vec![Rectf::new(centre.x + 110.0, centre.y - 20.0, 40.0, 40.0)];
        // The facade default viewport, so the ring radius is the legacy 120 px.
        let viewport = Vec2::new(160.0, 80.0);
        let mut rng = StdRng::seed_from_u64(13);
        let wall = level.walls[0];
        for _ in 0..300 {
            let p = spawn_point(&mut rng, &level, &player, viewport);
            assert!(
                p.x >= 8.0 - 1e-3 && p.y >= 8.0 - 1e-3,
                "spawn {p:?} past min margin"
            );
            assert!(
                p.x <= level.arena.x - 8.0 + 1e-3 && p.y <= level.arena.y - 8.0 + 1e-3,
                "spawn {p:?} past max margin"
            );
            assert!(
                p.dist(centre) >= 90.0 - 1e-3,
                "spawn {p:?} too close to player"
            );
            let cx = p.x.clamp(wall.x, wall.right());
            let cy = p.y.clamp(wall.y, wall.bottom());
            let d = ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt();
            assert!(d >= 4.0 - 1e-3, "spawn {p:?} overlaps a wall");
        }
        // The viewport argument drives the radius: `len() * 0.5 + 16`, floor
        // 120 px . A cleared level accepts the whole ring, so
        // the distance is exact.
        level.walls.clear();
        let small = spawn_point(&mut rng, &level, &player, Vec2::splat(1.0));
        assert!(
            (small.dist(centre) - 120.0).abs() < 1e-3,
            "ring radius floored wrong: {small:?}"
        );
        let wide = spawn_point(&mut rng, &level, &player, Vec2::new(600.0, 400.0));
        let r = (wide - centre).len();
        assert!(
            (r - (Vec2::new(600.0, 400.0).len() * 0.5 + 16.0)).abs() < 1e-3,
            "ring radius does not follow the viewport: {r}"
        );
    }

    /// A ring that cannot find a legal point (arena smaller than the ring) is
    /// clamped to the 6 px last resort, never left outside the arena.
    #[test]
    fn spawn_point_clamps_when_the_ring_cannot_fit() {
        let level = Level::new(Theme::Sewer);
        let player = Player::new(level.arena * 0.5, WeaponKind::Gnaw);
        let mut rng = StdRng::seed_from_u64(3);
        // A 2000 px viewport pushes the ring far outside the arena.
        for _ in 0..50 {
            let p = spawn_point(&mut rng, &level, &player, Vec2::splat(2000.0));
            assert!(
                p.x >= 6.0 && p.y >= 6.0 && p.x <= level.arena.x - 6.0 && p.y <= level.arena.y - 6.0,
                "clamped spawn {p:?} left the arena"
            );
        }
    }

    /// `World::spawn_point` (the facade hook used by the ported legacy test) is
    /// the same free function with the facade rng/level/player/viewport.
    #[test]
    fn facade_spawn_point_matches_the_system_helper() {
        let mut w = World::new(Theme::Sewer, 11);
        let mut rng = StdRng::seed_from_u64(11);
        for _ in 0..20 {
            let expected = spawn_point(&mut rng, &w.level, &w.player, w.viewport);
            let actual = w.spawn_point();
            assert_eq!(expected, actual);
        }
    }

    /// One pulse that would overflow the cap must stop at the cap *within* the
    /// tick: `Commands` are deferred, so the system has to count its own queued
    /// spawns instead of re-reading the enemy count.
    #[test]
    fn a_pulse_never_overfills_the_cap_between_deferred_spawns() {
        let mut w = World::new(Theme::Sewer, 5);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 4) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        assert_eq!(w.enemies().len(), ENEMY_CAP - 4);
        // Force a pulse this tick; at 100 s it asks for 6 enemies but only 3
        // slots exist (one is reserved for the boss).
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 0.1, 100.0);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1);
        assert!(!w.director.boss_spawned);
    }

    /// The reserved slot holds for the whole run and is only spent by the boss.
    #[test]
    fn director_reserves_one_slot_for_the_boss_until_it_spawns() {
        let mut w = World::new(Theme::Sewer, 17);
        let dt = 0.1;
        let mut elapsed = 0.0;
        loop {
            elapsed += dt;
            assert!(elapsed < director::BOSS_TIME);
            director_tick(&mut w, dt, elapsed);
            assert!(
                w.enemies().len() < ENEMY_CAP,
                "cap exceeded at {elapsed:.1}s: {}",
                w.enemies().len()
            );
            assert!(!w.director.boss_spawned);
            if w.enemies().len() == ENEMY_CAP - 1 {
                break;
            }
        }
        // The reserved slot is still free: the next tick past BOSS_TIME fills
        // it, and the cap still holds.
        director_tick(&mut w, dt, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        assert_eq!(w.enemies().len(), ENEMY_CAP);
        assert!(w.enemies().iter().any(|e| e.kind == EnemyKind::Boss));
    }

    /// Boss spawn *assigns* `shake = 14.0` (the legacy code, not a `max`) and
    /// pushes `Sfx::Boss`.
    #[test]
    fn boss_spawn_shakes_the_screen_and_plays_the_boss_sfx() {
        let mut w = World::new(Theme::Sewer, 19);
        w.shake = 40.0;
        w.sfx.clear();
        director_tick(&mut w, 0.1, director::BOSS_TIME);
        assert!(w.director.boss_spawned);
        assert_eq!(w.shake, 14.0);
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::Boss).count(), 1);
    }

    /// The boss spawns exactly once.
    #[test]
    fn boss_spawns_once() {
        let mut w = World::new(Theme::Sewer, 23);
        director_tick(&mut w, 0.1, director::BOSS_TIME + 3.0);
        let first = w
            .enemies()
            .iter()
            .filter(|e| e.kind == EnemyKind::Boss)
            .count();
        assert_eq!(first, 1);
        director_tick(&mut w, 0.1, director::BOSS_TIME + 300.0);
        let second = w
            .enemies()
            .iter()
            .filter(|e| e.kind == EnemyKind::Boss)
            .count();
        assert_eq!(second, 1, "the boss respawned");
    }

    /// Elites are gated on `elapsed > ELITE_START` (strictly greater), spawn a
    /// Brute at 1.6× the current hp mult and reset the timer to the interval.
    #[test]
    fn elites_start_after_the_gate_and_scale_their_hp() {
        let mut w = World::new(Theme::Sewer, 29);
        w.director.elite_timer = 0.0;
        // Exactly at the gate: no elite (legacy `elapsed > ELITE_START`).
        director_tick(&mut w, 0.0, director::ELITE_START);
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::Brute));

        director_tick(&mut w, 0.1, director::ELITE_START + 0.1);
        let enemies = w.enemies();
        let brute = enemies
            .iter()
            .find(|e| e.kind == EnemyKind::Brute)
            .expect("an elite brute must spawn once the gate opens");
        assert_eq!(w.director.elite_timer, director::ELITE_INTERVAL);
        let expected =
            EnemyKind::Brute.base_hp() * director::hp_mult(director::ELITE_START + 0.1) * 1.6;
        assert!((brute.max_hp - expected).abs() < 1e-3, "hp = {}", brute.hp);
    }

    /// The elite branch never takes the boss's reserved slot.
    #[test]
    fn elites_are_skipped_when_only_the_boss_slot_is_left() {
        let mut w = World::new(Theme::Sewer, 31);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 1) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        w.director.elite_timer = 0.0;
        director_tick(&mut w, 0.1, director::ELITE_START + 1.0);
        assert_eq!(w.enemies().len(), ENEMY_CAP - 1, "elite took the boss slot");
        assert!(!w.enemies().iter().any(|e| e.kind == EnemyKind::Brute));
    }

    /// A tick that both pulses *and* crosses the boss gate still respects the
    /// cap: the pulse is blocked, the boss still gets its reserved slot.
    #[test]
    fn a_late_tick_blocked_at_the_cap_still_spawns_the_boss() {
        let mut w = World::new(Theme::Sewer, 43);
        let centre = w.level.arena * 0.5;
        for _ in 0..(ENEMY_CAP - 1) {
            w.spawn_enemy(EnemyKind::Skitterer, centre, 1.0);
        }
        w.director.spawn_timer = 0.0;
        director_tick(&mut w, 500.0, director::BOSS_TIME);
        assert_eq!(w.enemies().len(), ENEMY_CAP);
        assert!(w.director.boss_spawned);
    }

    /// The neon-era legacy body of `run_director`, kept as an independent
    /// reference so the rng
    /// draw order can be compared tick by tick — including the enemy-id
    /// counter and the boss-arrival VFX, both of which consume rng or touch
    /// state in the same order the system must.
    #[allow(clippy::too_many_arguments)]
    fn legacy_director_tick(
        dt: f32,
        elapsed: f32,
        director: &mut Director,
        rng: &mut StdRng,
        level: &Level,
        player: &Player,
        viewport: Vec2,
        particles: &mut Particles,
        next_enemy_id: &mut u32,
        out: &mut Vec<Enemy>,
    ) {
        let cap = if director.boss_spawned {
            ENEMY_CAP
        } else {
            ENEMY_CAP - 1
        };

        director.spawn_timer -= dt;
        if director.spawn_timer <= 0.0 {
            director.spawn_timer = director::spawn_interval(elapsed);
            let count = director::pulse_count(elapsed);
            let mult = director::hp_mult(elapsed);
            for _ in 0..count {
                if out.len() >= cap {
                    break;
                }
                let kind = director::pick_kind(elapsed, rng);
                let pos = spawn_point(rng, level, player, viewport);
                let mut e = Enemy::spawn(kind, pos, mult, rng);
                e.id = *next_enemy_id;
                *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
                out.push(e);
            }
        }

        director.elite_timer -= dt;
        if director.elite_timer <= 0.0 && elapsed > director::ELITE_START {
            director.elite_timer = director::ELITE_INTERVAL;
            if out.len() < cap {
                let pos = spawn_point(rng, level, player, viewport);
                let mult = director::hp_mult(elapsed) * 1.6;
                let mut e = Enemy::spawn(EnemyKind::Brute, pos, mult, rng);
                e.id = *next_enemy_id;
                e.elite = true;
                *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
                out.push(e);
            }
        }

        if !director.boss_spawned && elapsed >= director::BOSS_TIME {
            director.boss_spawned = true;
            let pos = spawn_point(rng, level, player, viewport);
            let mut e = Enemy::spawn(EnemyKind::Boss, pos, 1.0, rng);
            e.id = *next_enemy_id;
            *next_enemy_id = next_enemy_id.wrapping_add(1).max(1);
            out.push(e);
            let accent = level.palette.accent;
            vfx::emit_boss_spawn(particles, rng, pos, accent);
        }
    }

    /// Everything `spawn_point` and `Enemy::spawn` derive from the rng, so any
    /// divergence in the draw order shows up.
    fn fingerprint(e: &Enemy) -> (EnemyKind, bool, Vec<u32>) {
        let fields = [
            e.pos.x, e.pos.y, e.hp, e.max_hp, e.radius, e.damage, e.fire_cd, e.wander, e.anim,
        ];
        (
            e.kind,
            e.elite,
            fields.iter().map(|f| f.to_bits()).collect(),
        )
    }

    /// The system is a line-by-line port: the same seed, clock, level and
    /// viewport must produce the same enemies in the same order as the legacy
    /// body, which only holds if every rng draw (count, range and order)
    /// matches.
    #[test]
    fn director_matches_the_legacy_draw_order() {
        let seed = 41;
        let mut w = World::new(Theme::Sewer, seed);

        // The facade player/level are `Level::new`/`Player::new` fresh and no
        // other system moves them, so an independently built pair is identical.
        let level = Level::new(Theme::Sewer);
        let player = Player::new(level.arena * 0.5, WeaponKind::Gnaw);
        assert_eq!(w.player.pos, player.pos);
        assert_eq!(w.level.arena, level.arena);
        let viewport = w.viewport;

        let mut ref_rng = StdRng::seed_from_u64(seed);
        let mut ref_director = Director::default();
        let mut ref_particles = Particles::new(PARTICLE_CAP);
        let mut ref_next_enemy_id: u32 = 1;
        let mut reference: Vec<Enemy> = Vec::new();

        let dt = 0.5;
        let mut elapsed = 0.0;
        let mut compared = 0usize;
        for _ in 0..620 {
            elapsed += dt;
            legacy_director_tick(
                dt,
                elapsed,
                &mut ref_director,
                &mut ref_rng,
                &level,
                &player,
                viewport,
                &mut ref_particles,
                &mut ref_next_enemy_id,
                &mut reference,
            );
            director_tick(&mut w, dt, elapsed);

            let port = w.enemies();
            assert_eq!(port.len(), reference.len(), "count at {elapsed:.2}s");
            for (i, expected) in reference.iter().enumerate().skip(compared) {
                assert_eq!(
                    fingerprint(&port[i]),
                    fingerprint(expected),
                    "enemy {i} differs at {elapsed:.2}s"
                );
            }
            compared = reference.len();
            assert_eq!(w.director.spawn_timer, ref_director.spawn_timer);
            assert_eq!(w.director.elite_timer, ref_director.elite_timer);
            assert_eq!(w.director.boss_spawned, ref_director.boss_spawned);
        }
        assert_eq!(compared, ENEMY_CAP, "the run must exercise the cap");
        assert!(w.director.boss_spawned, "the run must reach the boss");
        // In a director-only run the port's SpawnId-derived enemy ids land on
        // exactly the legacy values (1, 2, 3, ...), so the ids above are a real
        // equivalence check, not just uniqueness.
        assert_eq!(
            w.enemies().first().map(|e| e.id),
            Some(1),
            "first director enemy must get id 1"
        );
    }

    // ---- Neon-merge hooks (`run_director`) -------------------------------

    /// Hook: `e.id = next_enemy_id` on every director spawn. Ids must be
    /// non-zero (0 means "anonymous enemy" to `DamageNumbers`), unique, and
    /// increase in spawn order, so per-enemy damage numbers aggregate instead
    /// of merging strangers.
    #[test]
    fn director_assigns_unique_nonzero_enemy_ids_in_spawn_order() {
        let mut w = World::new(Theme::Sewer, 47);
        let dt = 0.25;
        let mut elapsed = 0.0;
        for _ in 0..240 {
            elapsed += dt;
            director_tick(&mut w, dt, elapsed);
        }
        assert!(w.enemies().len() > 20, "the run should have spawned a horde");

        let ids: Vec<u32> = w.enemies().iter().map(|e| e.id).collect();
        assert!(ids.iter().all(|id| *id != 0), "an enemy got id 0");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "enemy ids are not unique");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ids must increase in spawn order: {:?}",
            &ids[..ids.len().min(8)]
        );
    }

    /// Hook: `e.elite = true` on the director's periodic elite Brute only —
    /// ordinary late-game Brutes stay unflagged so the art/VFX can tell them
    /// apart.
    #[test]
    fn only_director_elites_are_flagged_elite() {
        let mut w = World::new(Theme::Sewer, 53);
        // A late pulse with the elite gate closed: nothing may be elite.
        w.director.spawn_timer = 0.0;
        w.director.elite_timer = director::ELITE_INTERVAL;
        director_tick(&mut w, 0.1, 200.0);
        assert!(!w.enemies().is_empty(), "the pulse should have spawned");
        assert!(
            w.enemies().iter().all(|e| !e.elite),
            "a non-elite spawn was flagged elite"
        );

        // Now the elite branch fires: exactly one flagged Brute.
        w.director.elite_timer = 0.0;
        director_tick(&mut w, 0.1, 200.1);
        let enemies = w.enemies();
        let elites: Vec<&Enemy> = enemies.iter().filter(|e| e.elite).collect();
        assert_eq!(elites.len(), 1, "exactly one elite per elite spawn");
        assert_eq!(elites[0].kind, EnemyKind::Brute);
        assert_ne!(elites[0].id, 0, "the elite still needs a runtime id");
    }

    /// Hook: `vfx::emit_boss_spawn(...)` right after the boss is pushed and
    /// before the shake/SFX, so the arrival shockwave and cross burst exist on
    /// the boss tick.
    #[test]
    fn boss_arrival_emits_the_spawn_vfx() {
        let mut w = World::new(Theme::Sewer, 59);
        assert_eq!(w.particles.shockwaves.len(), 0);
        let particles_before = w.particles.len();

        director_tick(&mut w, 0.1, director::BOSS_TIME);

        assert!(w.director.boss_spawned);
        assert_eq!(
            w.particles.shockwaves.len(),
            1,
            "boss arrival should push its shockwave"
        );
        assert!(
            w.particles.len() > particles_before,
            "boss arrival should emit particles"
        );
        // Still the legacy shake + SFX.
        assert_eq!(w.shake, 14.0);
        assert_eq!(w.sfx.iter().filter(|s| **s == Sfx::Boss).count(), 1);
    }

    /// The boss VFX draws from the same rng the spawn used, in the same place:
    /// the tick after the boss spawn must continue the identical draw sequence.
    #[test]
    fn boss_vfx_draws_do_not_disturb_the_following_tick() {
        let seed = 61;
        let mut w = World::new(Theme::Sewer, seed);

        let level = Level::new(Theme::Sewer);
        let player = Player::new(level.arena * 0.5, WeaponKind::Gnaw);
        let viewport = w.viewport;
        let mut ref_rng = StdRng::seed_from_u64(seed);
        let mut ref_director = Director::default();
        let mut ref_particles = Particles::new(PARTICLE_CAP);
        let mut ref_next_enemy_id: u32 = 1;
        let mut reference: Vec<Enemy> = Vec::new();

        let dt = 0.5;
        let mut elapsed = 0.0;
        for _ in 0..560 {
            elapsed += dt;
            legacy_director_tick(
                dt,
                elapsed,
                &mut ref_director,
                &mut ref_rng,
                &level,
                &player,
                viewport,
                &mut ref_particles,
                &mut ref_next_enemy_id,
                &mut reference,
            );
            director_tick(&mut w, dt, elapsed);
        }
        // Past BOSS_TIME: the boss VFX consumed rng inside the spawn tick, so
        // the next pulse's enemies only match if the draw order is unchanged.
        assert!(w.director.boss_spawned);
        assert_eq!(w.enemies().len(), reference.len());
        assert_eq!(
            w.enemies()
                .iter()
                .map(|e| e.fire_cd.to_bits())
                .collect::<Vec<_>>(),
            reference
                .iter()
                .map(|e| e.fire_cd.to_bits())
                .collect::<Vec<_>>(),
            "the boss VFX rng draws disturbed the following spawns"
        );
        assert_eq!(w.particles.shockwaves.len(), 1);
        assert_eq!(ref_particles.shockwaves.len(), 1);
    }
}
