//! ECS facade tests.
//!
//! Port of **every** test from the legacy `world.rs` suite. Assertions are kept
//! identical in intent; only the setup/inspection mechanics changed:
//!
//! - Entities are spawned through the facade helpers (`World::spawn_enemy`,
//!   `spawn_bullet`, `spawn_gem`, `spawn_pickup`, `spawn_pool`) instead of
//!   pushing to the legacy `Vec` fields.
//! - Single-system tests use the narrow harness (`CLAUDE.md`, "Testing strategy"):
//!   `world.begin_tick(dt, dir, dash)`,
//!   `world.ecs.run_system_cached(systems::..::system)`, `world.end_tick()`.
//!   Between `begin_tick` and `end_tick` the facade singleton fields are
//!   placeholders, so those tests read/write `world.ecs.resource::<T>()`.
//! - `apply_pending` is the same loop over `world.pending_levelups` using
//!   `world.player.loadout.generate_choices(&mut world.rng)`.
//!
//! The legacy `fire_one`/`update_bullets`/`update_gems`/... private helpers are
//! not part of the facade, so the tests that called them drive the equivalent
//! system through the narrow harness with the same inputs (weapon kind/level,
//! dt, seeded rng). `nearest_enemy` is exercised through the facade helper of
//! the same name (it returns the entity instead of the legacy `Vec` index).

use bevy_ecs::entity::Entity;
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::audio::Sfx;
use crate::game::art_walls::WallKind;
use crate::game::bullet::{Bullet, Faction, Orbit};
use crate::game::collision;
use crate::game::collision::{SizeClass, TileKind, TileMap};
use crate::game::director;
use crate::game::ecs::components::Pool;
use crate::game::ecs::constants::{ENEMY_CAP, PARTICLE_CAP, PLAYER_RADIUS};
use crate::game::ecs::resources::{GridScratch, PickupScratch};
use crate::game::ecs::systems::{combat, enemy_move, loot, player, projectiles, weapons};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, Pickup, PickupKind};
use crate::game::level::{Rectf, Theme, MAP_COLS, MAP_ROWS};
use crate::game::particle::damage_number::DamageSource;
use crate::game::particle::vfx;
use crate::game::player::Player;
use crate::game::weapon::{Weapon, WeaponKind};
use crate::game::world::{on_screen, World};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;

/// Run one system through the narrow harness (deferred `Commands` are applied
/// by `run_system_cached`, exactly like a chained schedule step).
fn run<O: 'static, M, S>(w: &mut World, system: S) -> O
where
    S: bevy_ecs::system::IntoSystem<(), O, M> + 'static,
{
    w.ecs.run_system_cached(system).expect("system runs")
}

/// Mutate one spawned enemy component straight in the ECS world (queries see
/// it immediately; there is no facade cache to refresh).
fn edit_enemy(w: &mut World, entity: Entity, f: impl FnOnce(&mut Enemy)) {
    let mut enemy = w.ecs.get_mut::<Enemy>(entity).expect("enemy exists");
    f(&mut enemy);
}

/// Strip the generated layout's walls so the test measures one system instead
/// of the map (what `Level::walls.clear()` used to do before the tile grid
/// owned collision).
fn open_map(w: &mut World) {
    w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
}

/// Paint a solid cell (`WallKind::Solid`) and rebuild the neighbour masks.
fn paint_solid(w: &mut World, cx: i32, cy: i32, cw: i32, ch: i32) {
    w.level
        .tiles
        .fill_cells(cx, cy, cw, ch, TileKind::Wall(WallKind::Solid));
    w.level.tiles.rebuild_masks();
}

/// Does any wall collider overlap a circle of `r` at `p` for `class`?
fn touches_a_wall(w: &World, p: Vec2, r: f32, class: SizeClass) -> bool {
    let mut hit = false;
    let m = r + collision::TILE_F;
    w.level.tiles.for_each_collider_in(
        Rectf::new(p.x - m, p.y - m, 2.0 * m, 2.0 * m),
        class,
        None,
        |rect| {
            if collision::circle_rect_overlap(p, r, &rect) {
                hit = true;
            }
        },
    );
    hit
}

fn apply_pending(w: &mut World) {
    while w.pending_levelups > 0 {
        let choices = w.player.loadout.generate_choices(&mut w.rng);
        if let Some(up) = choices.first().cloned() {
            let heal = w.player.loadout.apply(&up);
            w.player.heal(heal);
        }
        w.pending_levelups -= 1;
    }
}

/// Drive a simulation on every theme, drawing every few frames at several
/// viewport sizes, to catch panics in the update + render paths.
///
/// One theme keeps the 100 s span that reaches the mid-run spawn curve and the
/// elite branch; the other five get 20 s each, because a per-theme update or
/// render panic shows up in the first few hundred ticks, and the crowd caps
/// are asserted at the top of the curve by
/// `enemy_cap_holds_at_the_top_of_the_curve` too.
/// (It used to run 6,000 ticks on all six themes: 36,000 ticks and 857
/// `World::draw` calls of a 240x120 buffer.)
#[test]
fn headless_run_does_not_panic() {
    for (i, theme) in Theme::all().into_iter().enumerate() {
        let mut w = World::new(theme, 12345);
        let dt = 1.0 / 60.0;
        let sizes = [(120usize, 60usize), (1, 1), (240, 120)];
        let ticks: u32 = if i == 0 { 6_000 } else { 1_200 };
        for tick in 0..ticks {
            let dir = Vec2::from_angle(tick as f32 * 0.05);
            let dash = tick % 90 == 0;
            w.update(dt, dir, dash);
            apply_pending(&mut w);
            if tick % 7 == 0 {
                let (sw, sh) = sizes[(tick as usize / 7) % sizes.len()];
                let mut pb = PixelBuffer::new(sw, sh);
                w.draw(&mut pb);
            }
            if w.finished() {
                break;
            }
        }
        // Invariants.
        assert!(w.enemies().len() <= ENEMY_CAP);
        assert!(w.particles.len() <= PARTICLE_CAP);
    }
}

/// The `EnemyIndex` validity proof under a real crowd.
///
/// `weapons`, `combat::player_bullet_hits_system`, `combat::contact_system`
/// and `projectiles::pools_system` read the cached `SpawnId`-ordered
/// `EnemyIndex` (as do `separation`, `wall_resolve` and `grid_refresh` in
/// `enemy_move`). Each of them calls `resources::debug_assert_enemy_index`
/// first, which compares the cache against a freshly sorted `SpawnId` list and
/// fails the test if any slot disagrees. That guard is **opt-in**
/// (`RATRUN_CHECK_ENEMY_INDEX=1`) because it re-sorts the whole enemy list per
/// call, so this test turns it on for its own duration.
///
/// It used to drive a full fifteen minutes (54,100 ticks) of the Sewer to
/// reach the boss window. The window is broken by a spawn/despawn race, which
/// shows up in the first seconds of a live crowd, and the boss window's own
/// spawn and reap paths are covered by `systems::spawn`'s tests — so this
/// seeds a full crowd, ticks ten seconds through it, jumps the clock into the
/// boss window with `w.elapsed` and ticks two more seconds there. The rat is
/// kept alive so no tick ends early.
/// Debug builds only: the guard itself (`debug_assert_enemy_index`) is
/// `debug_assertions`-gated.
#[cfg(debug_assertions)]
#[test]
fn enemy_index_is_fresh_at_every_reader() {
    use crate::game::ecs::resources::force_enemy_index_check;

    force_enemy_index_check(true);
    let mut w = World::new(Theme::Sewer, 4242);
    let dt = 1.0 / 60.0;
    let centre = w.level.arena * 0.5;
    w.player.pos = centre;
    let mut rng = StdRng::seed_from_u64(99);
    for i in 0..120u32 {
        let a = i as f32 * std::f32::consts::TAU / 120.0;
        let p = centre + Vec2::from_angle(a) * (40.0 + (i % 7) as f32 * 18.0);
        w.spawn_enemy_with_rng(EnemyKind::Skitterer, p, 1.0, &mut rng);
    }
    let mut peak = 0usize;
    let mut run = |w: &mut World, ticks: u32| {
        for tick in 0..ticks {
            let dir = Vec2::from_angle(tick as f32 * 0.05);
            w.update(dt, dir, tick % 157 == 0);
            w.player.hp = w.player.max_hp();
            apply_pending(w);
            peak = peak.max(w.enemies().len());
        }
    };
    // Ten seconds of a live crowd: spawning, killing, reaping, the grid swap
    // and every reader's check.
    run(&mut w, 600);
    // Then the boss hand-off, reached by moving the clock rather than ticking
    // through fourteen more minutes.
    w.elapsed = director::BOSS_TIME - 1.0;
    run(&mut w, 180);
    force_enemy_index_check(false);
    assert!(
        w.elapsed >= director::BOSS_TIME,
        "the clock jump never reached the boss time"
    );
    assert!(peak > 10, "crowd never grew (peak {peak}): guard untested");
}

/// **One twin-run sweep for the whole simulation.**
///
/// Same seed, same scripted input, twice ⇒ the same run, down to the bit. This
/// replaces the hand-written twin-run test that every system and every boss
/// used to carry: each one re-ran a `World` twice to prove order-independence,
/// so the suite paid for the same property twenty-odd times. Here it is proven
/// once per floor and once per difficulty window that actually reorders
/// entities (the miniboss appointment and the boss hand-off).
///
/// Rows are cheap on purpose — at most 20 s of game time each, 23,400 ticks
/// for the whole table, where the deleted per-theme full-length twin ran
/// 649,440. Add a row (a name and a setup closure) rather than a new test when
/// a new system needs its own twin proof.
#[test]
fn the_same_seed_plays_the_same_run() {
    /// Fold everything a divergence could move: the clock and the score, the
    /// rat, and every body's kind, hp and position (in `SpawnId` order).
    /// `enemies()`/`bullets()` collect the cached order, so they need `&mut`.
    fn fingerprint(w: &mut World) -> (u64, u64) {
        let enemies = w.enemies();
        let bullets = w.bullets();
        let particles = w.particles.len();
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        {
            let p = &w.player;
            for v in [
                w.elapsed.to_bits() as u64,
                w.kills as u64,
                w.score as u64,
                w.gold as u64,
                p.hp.to_bits() as u64,
                p.xp as u64,
                p.level as u64,
                p.pos.x.to_bits() as u64,
                p.pos.y.to_bits() as u64,
                enemies.len() as u64,
                bullets.len() as u64,
                particles as u64,
            ] {
                h ^= v;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        for e in &enemies {
            for v in [
                e.kind as u32 as u64,
                e.hp.to_bits() as u64,
                e.pos.x.to_bits() as u64,
                e.pos.y.to_bits() as u64,
            ] {
                h ^= v;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        for b in &bullets {
            for v in [
                b.pos.x.to_bits() as u64,
                b.pos.y.to_bits() as u64,
                b.vel.x.to_bits() as u64,
                b.vel.y.to_bits() as u64,
            ] {
                h ^= v;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        (w.elapsed as u64, h)
    }

    /// Play `ticks` of one world with the scripted circle input every
    /// full-length test in this file used, keeping the rat alive.
    fn play(w: &mut World, ticks: u32) {
        for tick in 0..ticks {
            let dir = Vec2::from_angle(tick as f32 * 0.017);
            w.update(1.0 / 60.0, dir, tick.is_multiple_of(97));
            w.player.hp = w.player.max_hp();
            apply_pending(w);
            if w.finished() {
                break;
            }
        }
    }

    // One row per floor, 20 s of play each.
    for theme in Theme::playable() {
        let mut a = World::new(theme, 12345);
        let mut b = World::new(theme, 12345);
        play(&mut a, 1_200);
        play(&mut b, 1_200);
        assert_eq!(
            fingerprint(&mut a),
            fingerprint(&mut b),
            "{theme:?}: the same seed diverged over 20 s"
        );
    }

    // The miniboss appointment (3:00) and the boss hand-off (15:00): both
    // spawn and reorder bodies mid-run, which the opening never does.
    for jump in [director::MINIBOSS_TIMES[0], director::BOSS_TIME] {
        let mut a = World::new(Theme::Sewer, 777);
        let mut b = World::new(Theme::Sewer, 777);
        a.elapsed = jump - 5.0;
        b.elapsed = jump - 5.0;
        play(&mut a, 1_200);
        play(&mut b, 1_200);
        assert_eq!(
            fingerprint(&mut a),
            fingerprint(&mut b),
            "the Sewer diverged across the {jump:.0} s window"
        );
    }

    // Firing weapons at a dense crowd: the draw order of every weapon system,
    // in one row instead of one per weapon.
    let mut a = World::new(Theme::Kitchen, 5);
    let mut b = World::new(Theme::Kitchen, 5);
    for w in [&mut a, &mut b] {
        w.player.loadout.weapons.clear();
        for kind in [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::AcidSpit,
            WeaponKind::AcidRain,
            WeaponKind::Hantavirus,
        ] {
            w.player.loadout.weapons.push(Weapon {
                kind,
                level: 3,
                timer: 0.0,
                evolved: false,
            });
        }
        for i in 0..8u32 {
            let ang = i as f32 * std::f32::consts::TAU / 8.0;
            let p = w.player.pos + Vec2::from_angle(ang) * (28.0 + i as f32 * 3.0);
            w.spawn_enemy(EnemyKind::Skitterer, p, 40.0);
        }
    }
    play(&mut a, 600);
    play(&mut b, 600);
    assert_eq!(
        fingerprint(&mut a),
        fingerprint(&mut b),
        "the same seed and the same weapons diverged"
    );
}

/// Manual screenshot tool: line up one of each insect + a glowing pickup
/// in a zoomable frame. `RATRUN_DUMP=bugs cargo test dump_bestiary`.
#[test]
fn dump_bestiary() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("bugs") {
        return;
    }
    let mut w = World::new(Theme::Sewer, 1);
    let cx = w.level.arena.x * 0.5;
    let cy = w.level.arena.y * 0.5;
    w.player.pos = Vec2::new(cx, cy + 42.0);
    let mut rng = StdRng::seed_from_u64(3);
    let kinds = [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
        EnemyKind::Boss,
    ];
    for (i, k) in kinds.iter().enumerate() {
        let e = w.spawn_enemy_with_rng(
            *k,
            Vec2::new(cx - 78.0 + i as f32 * 30.0, cy),
            1.0,
            &mut rng,
        );
        edit_enemy(&mut w, e, |enemy| {
            enemy.facing = Vec2::new(0.25, 1.0).normalized();
            enemy.anim = i as f32 * 0.9;
        });
    }
    w.spawn_pickup(Pickup {
        pos: Vec2::new(cx - 30.0, cy + 22.0),
        kind: PickupKind::Heal,
        bob: 1.2,
    });
    w.spawn_pickup(Pickup {
        pos: Vec2::new(cx + 30.0, cy + 22.0),
        kind: PickupKind::Nuke,
        bob: 0.3,
    });
    let mut pb = PixelBuffer::new(200, 120);
    w.draw(&mut pb);
    // Write PPM (top fg + bottom bg per cell handled by caller's converter;
    // here we dump the raw pixel grid directly).
    let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
    for y in 0..pb.h {
        for x in 0..pb.w {
            let c = pb.pixel_at(x, y);
            out.push(c.0);
            out.push(c.1);
            out.push(c.2);
        }
    }
    std::fs::write("/tmp/ratrun_bugs.ppm", out).unwrap();
}

/// The map here is painted, not sampled from the generator: the first interior
/// `Solid` cell of a real map is deep inside a wall block, where a body has
/// nowhere to be ejected *to* (the tile grid's search window is three cells and
/// documents that as best-effort). An isolated 2 × 2 block surrounded by open
/// floor is the case wall resolution actually guarantees.
#[test]
fn enemies_are_pushed_out_of_walls() {
    let mut w = World::new(Theme::Sewer, 5);
    open_map(&mut w);
    paint_solid(&mut w, 60, 60, 2, 2);
    let block = Rectf::new(
        60.0 * collision::TILE_F,
        60.0 * collision::TILE_F,
        2.0 * collision::TILE_F,
        2.0 * collision::TILE_F,
    );
    let class = SizeClass::of_enemy(EnemyKind::Cat);
    let mut rng = StdRng::seed_from_u64(9);
    // Spawn an enemy buried in the middle of the block.
    w.spawn_enemy_with_rng(EnemyKind::Cat, block.center(), 1.0, &mut rng);
    assert!(
        touches_a_wall(&w, block.center(), EnemyKind::Cat.radius(), class),
        "the case under test starts buried"
    );
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    let c = w.enemies()[0].pos;
    assert!(
        !touches_a_wall(&w, c, EnemyKind::Cat.radius(), class),
        "wall resolution left the body overlapping a wall: {c:?}"
    );
}

/// Bodies that stop moving while they are nowhere near the rat are a routing
/// bug, not a lull: the flow field is steering them into a wall face, `steer`
/// pushes them into it and wall resolution pushes them straight back out, so
/// the position is bit-for-bit unchanged every tick (measured before the fix
/// below: 9 of 261 bodies in a 90 s run, plus bodies that dithered on the spot
/// when their coarse target sat under them).
///
/// This drives the **real schedule** — director, steering, separation, wall
/// resolution, the lot — with a rat that wanders, samples every body every tick
/// and counts the ones that stayed put for two seconds while more than 40 px
/// from the rat (a body pressed against the rat is contact-blocked, which is
/// legitimate). Each body is counted once, however long it stays stuck.
#[test]
fn no_enemy_is_pinned_against_a_wall() {
    use crate::game::ecs::components::SpawnId;

    const DT: f32 = 1.0 / 60.0;
    /// 2 s at 60 Hz: how long a body has to be motionless to count as pinned.
    const STILL_TICKS: u32 = 120;
    const NEAR_PLAYER: f32 = 40.0;
    const TICKS: u32 = 1800;

    let mut total_pinned = 0;
    let mut total_bodies = 0;
    for theme in Theme::all() {
        for seed in [3u64, 11, 29] {
            let mut w = World::new(theme, seed);
            // (SpawnId, last position that moved, tick it last moved, counted).
            let mut track: Vec<(u64, Vec2, u32, bool)> = Vec::new();
            let mut pinned_here = 0;
            for tick in 0..TICKS {
                // A rat wandering in a slow circle: the flow field retargets
                // the way it does in a real run and the horde has to route.
                let dir = Vec2::from_angle(tick as f32 * 0.03);
                w.update(DT, dir, false);
                apply_pending(&mut w);
                w.player.hp = w.player.max_hp();
                let ppos = w.player.pos;
                let current: Vec<(u64, Vec2)> = {
                    let mut q = w.ecs.query::<(Entity, &SpawnId, &Enemy)>();
                    // An opening Pillbug stays planted by design until the rat approaches.
                    q.iter(&w.ecs)
                        .filter(|(_, _, e)| !e.w3.opening_curl)
                        .map(|(_, id, e)| (id.0, e.pos))
                        .collect()
                };
                for (id, pos) in &current {
                    match track.iter_mut().find(|(i, _, _, _)| i == id) {
                        Some(slot) if slot.1.dist(*pos) > 0.05 => {
                            slot.1 = *pos;
                            slot.2 = tick;
                        }
                        Some(slot) => {
                            if !slot.3
                                && tick.saturating_sub(slot.2) >= STILL_TICKS
                                && pos.dist(ppos) > NEAR_PLAYER
                            {
                                slot.3 = true;
                                pinned_here += 1;
                            }
                        }
                        None => track.push((*id, *pos, tick, false)),
                    }
                }
                track.retain(|(id, _, _, _)| current.iter().any(|(i, _)| i == id));
            }
            total_bodies += track.len();
            total_pinned += pinned_here;
            eprintln!(
                "{theme:?} seed {seed}: {pinned_here} pinned of {} bodies",
                track.len()
            );
        }
    }
    eprintln!("pinned: {total_pinned} of {total_bodies} bodies");
    assert_eq!(
        total_pinned, 0,
        "{total_pinned} of {total_bodies} enemies stood still for 2 s away from the rat"
    );
}

/// After a long run, no enemy should ever sit inside a wall.
#[test]
fn run_keeps_enemies_out_of_walls() {
    for theme in Theme::all() {
        let mut w = World::new(theme, 21);
        for tick in 0..1800u32 {
            let dir = Vec2::from_angle(tick as f32 * 0.05);
            w.update(1.0 / 60.0, dir, false);
            apply_pending(&mut w);
        }
        for e in &w.enemies() {
            assert!(
                !touches_a_wall(&w, e.pos, e.radius, SizeClass::of_enemy(e.kind)),
                "{:?} enemy overlapping a wall at {:?}",
                theme,
                e.pos
            );
        }
    }
}

/// Screenshot tool for the themed props. `RATRUN_DUMP=props cargo test dump_props`.
#[test]
fn dump_props() {
    let theme = match std::env::var("RATRUN_DUMP").as_deref() {
        Ok("props") => Theme::Kitchen,
        Ok("props_sewer") => Theme::Sewer,
        Ok("props_lab") => Theme::Lab,
        _ => return,
    };
    let mut w = World::new(theme, 1);
    w.player.pos = w.level.arena * 0.5;
    // Full-arena overview.
    let mut pb = PixelBuffer::new(w.level.arena.x as usize, w.level.arena.y as usize);
    w.draw(&mut pb);
    let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
    for y in 0..pb.h {
        for x in 0..pb.w {
            let c = pb.pixel_at(x, y);
            out.push(c.0);
            out.push(c.1);
            out.push(c.2);
        }
    }
    std::fs::write("/tmp/ratrun_props.ppm", out).unwrap();
}

/// Exercise the boss + every pickup/pool/gem draw branch in one frame.
#[test]
fn draw_all_entity_kinds() {
    let mut w = World::new(Theme::Lab, 99);
    let c = w.level.arena * 0.5;
    let mut rng = StdRng::seed_from_u64(1);
    w.spawn_enemy_with_rng(EnemyKind::Boss, c + Vec2::new(30.0, 0.0), 1.0, &mut rng);
    for k in [
        EnemyKind::Skitterer,
        EnemyKind::Bat,
        EnemyKind::Cat,
        EnemyKind::Spitter,
        EnemyKind::Brute,
    ] {
        w.spawn_enemy_with_rng(k, c + Vec2::from_angle(0.3) * 20.0, 1.0, &mut rng);
    }
    w.spawn_gem(Gem::new(c, 1));
    w.spawn_gem(Gem::new(c, 30));
    for kind in [PickupKind::Heal, PickupKind::Magnet, PickupKind::Nuke] {
        w.spawn_pickup(Pickup {
            pos: c,
            kind,
            bob: 0.5,
        });
    }
    w.spawn_pool(Pool {
        pos: c,
        radius: 14.0,
        dps: 5.0,
        life: 2.0,
        color: (140, 220, 70),
    });
    let mut b = Bullet::new(c, Vec2::new(10.0, 0.0), 3.0, Faction::Enemy, (255, 0, 0));
    b.size = 2;
    w.spawn_bullet(b);

    let mut pb = PixelBuffer::new(100, 50);
    w.draw(&mut pb);
}

// ---- Review-fix regression tests -----------------------------------

#[test]
fn dead_enemies_deal_no_contact_damage() {
    let mut w = World::new(Theme::Sewer, 12);
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(1);
    let ppos = w.player.pos;
    let e = w.spawn_enemy_with_rng(EnemyKind::Cat, ppos, 1.0, &mut rng);
    edit_enemy(&mut w, e, |enemy| {
        enemy.hp = -1.0;
        enemy.contact_cd = 0.0;
    });
    let hp = w.player.hp;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, combat::contact_system);
    w.end_tick();
    assert_eq!(w.player.hp, hp, "a dead enemy still dealt contact damage");
    assert_eq!(w.enemies()[0].contact_cd, 0.0);
}

#[test]
fn loss_beats_win_on_the_final_tick() {
    let mut w = World::new(Theme::Sewer, 7);
    w.elapsed = director::RUN_SECONDS - 0.001;
    w.player.hp = -1.0;
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    assert!(w.game_over, "death on the final tick should be a loss");
    assert!(!w.won, "death must not also score a win");
}

/// Campaign rule (P8): the clock no longer wins a level. Surviving past
/// `director::RUN_SECONDS` only means the boss fight is on — the level ends
/// when the boss or the rat dies.
#[test]
fn the_level_does_not_end_at_the_run_clock() {
    let mut w = World::new(Theme::Sewer, 8);
    w.elapsed = director::RUN_SECONDS - 0.001;
    w.player.hp = w.player.max_hp();
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    assert!(!w.won, "surviving the full clock must not win the level");
    assert!(!w.game_over);
    assert!(!w.level_complete());
    assert!(!w.finished(), "the level keeps simulating past the clock");
    assert_eq!(w.time_left(), 0.0, "the countdown saturates at BOSS_TIME");
    assert!(w.boss_time_reached());
}

#[test]
fn nuke_kills_are_reaped_on_the_same_tick() {
    let mut w = World::new(Theme::Sewer, 47);
    w.director.spawn_timer = 1.0e9;
    w.director.elite_timer = 1.0e9;
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(6);
    let ppos = w.player.pos;
    let e = w.spawn_enemy_with_rng(EnemyKind::Cat, ppos, 1.0, &mut rng);
    edit_enemy(&mut w, e, |enemy| {
        enemy.hp = 1.0e6;
        enemy.max_hp = 1.0e6;
    });
    w.spawn_pickup(Pickup {
        pos: ppos,
        kind: PickupKind::Nuke,
        bob: 0.0,
    });
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    assert!(w.enemies().is_empty(), "a Nuke kill survived the tick");
    assert_eq!(w.kills, 1);
}

#[test]
fn spawn_point_stays_in_bounds_clear_of_walls_and_player() {
    let mut w = World::new(Theme::Sewer, 13);
    let center = w.level.arena * 0.5;
    w.player.pos = center;
    open_map(&mut w);
    // A 2 x 2-cell solid block out on the 120 px ring, east of the rat.
    let (ccx, ccy) = collision::cell_of(center);
    paint_solid(&mut w, ccx + 7, ccy - 2, 2, 2);
    let block = {
        let r = TileMap::tile_rect(ccx + 7, ccy - 2);
        Rectf::new(r.x, r.y, 2.0 * collision::TILE_F, 2.0 * collision::TILE_F)
    };
    for _ in 0..300 {
        let p = w.spawn_point();
        assert!(
            p.x >= 8.0 - 1e-3 && p.y >= 8.0 - 1e-3,
            "spawn {p:?} past min margin"
        );
        assert!(
            p.x <= w.level.arena.x - 8.0 + 1e-3 && p.y <= w.level.arena.y - 8.0 + 1e-3,
            "spawn {p:?} past max margin"
        );
        assert!(
            p.dist(center) >= 90.0 - 1e-3,
            "spawn {p:?} too close to player"
        );
        assert!(
            !collision::circle_rect_overlap(p, 4.0 - 1e-3, &block),
            "spawn {p:?} overlaps a wall"
        );
    }
}

/// The crowd cap holds at the busiest point of the curve.
///
/// It used to tick all 920 s of a level at 10 Hz (9,200 ticks) to walk into the
/// late budget. The cap can only be pressed once the curve is near its top, so
/// this jumps the clock to just before the boss hand-off with `w.elapsed` and
/// plays that window instead: the late budget, the miniboss chest and the boss
/// reserved slot, in 25 s.
#[test]
fn enemy_cap_holds_at_the_top_of_the_curve() {
    let mut w = World::new(Theme::Sewer, 17);
    let dt = 0.1;
    w.elapsed = director::BOSS_TIME - 25.0;
    let mut ticks = 0;
    while !w.finished() && ticks < 250 {
        w.update(dt, Vec2::ZERO, false);
        apply_pending(&mut w);
        // Keep the rat alive so the cap is exercised into the boss fight.
        w.player.hp = w.player.max_hp();
        let live = w.enemies().len();
        assert!(
            live <= ENEMY_CAP,
            "cap exceeded at {:.1}s: {}",
            w.elapsed,
            live
        );
        ticks += 1;
    }
    assert!(w.director.boss_spawned, "boss never spawned");
    assert!(
        w.elapsed >= director::BOSS_TIME,
        "the run ended before the boss arrived"
    );
}

#[test]
fn bullet_query_reaches_large_enemies_across_cells() {
    let mut w = World::new(Theme::Lab, 19);
    w.clear_enemies();
    w.clear_bullets();
    let mut rng = StdRng::seed_from_u64(2);
    // 15.9 (cell 0) and 32.5 (cell 2) are 16.6 px apart, which overlaps a
    // radius-3 bullet with a radius-14 boss but spans two cells.
    w.spawn_enemy_with_rng(EnemyKind::Boss, Vec2::new(32.5, 100.0), 1.0, &mut rng);
    let boss_hp = w.enemies()[0].hp;
    let mut b = Bullet::new(
        Vec2::new(15.9, 100.0),
        Vec2::ZERO,
        7.0,
        Faction::Player,
        (255, 255, 255),
    );
    b.radius = 3.0;
    w.spawn_bullet(b);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    // The legacy test built the grid by hand and passed `max_enemy_radius`
    // 14.0; `grid_build_system` derives exactly that from the boss radius.
    run(&mut w, enemy_move::grid_build_system);
    run(&mut w, combat::player_bullet_hits_system);
    w.end_tick();
    assert!(
        w.enemies()[0].hp < boss_hp,
        "wide bullet query missed the boss across cells"
    );
}

#[test]
fn fast_player_cannot_tunnel_a_thin_wall() {
    let mut w = World::new(Theme::Sewer, 23);
    open_map(&mut w);
    // A one-cell `WallKind::Maze` column: an 8 px spine at world x 516..524.
    w.level
        .tiles
        .fill_cells(32, 18, 1, 14, TileKind::Wall(WallKind::Maze));
    w.level.tiles.rebuild_masks();
    w.player.pos = Vec2::new(500.0, 400.0);
    w.player.vel = Vec2::new(900.0, 0.0);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, player::player_system);
    w.end_tick();
    assert!(
        w.player.pos.x <= 516.0 - PLAYER_RADIUS + 0.01,
        "player tunnelled to {:?}",
        w.player.pos
    );
}

#[test]
fn spore_orbit_replaces_its_ring_and_keeps_its_angle() {
    let mut w = World::new(Theme::Sewer, 4);
    w.clear_enemies();
    w.clear_bullets();
    // Legacy `fire_one(SporeOrbit, 5.0, 1.0, 4, ..)`: level 2 gives 4 spores.
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::SporeOrbit,
        level: 2,
        timer: 0.0,
        evolved: false,
    }];
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    {
        let mut query = w.ecs.query::<&mut Bullet>();
        for mut b in query.iter_mut(&mut w.ecs) {
            if let Some(o) = b.orbit.as_mut() {
                o.angle += 0.7;
            }
        }
    }
    assert_eq!(w.bullets().len(), 4);
    let carried = w.bullets()[0].orbit.unwrap().angle;
    w.ecs.resource_mut::<Player>().loadout.weapons[0].timer = 0.0;
    run(&mut w, weapons::fire_weapons_system);
    w.end_tick();
    let orbits: Vec<_> = w.bullets().iter().filter_map(|b| b.orbit).collect();
    assert_eq!(orbits.len(), 4, "two spore rings stacked");
    assert!(
        (orbits[0].angle - carried).abs() < 1e-4,
        "ring angle snapped instead of carrying forward"
    );
}

#[test]
fn untargeted_gnaw_does_not_emit_a_shoot_sfx() {
    let mut w = World::new(Theme::Sewer, 2);
    w.clear_enemies();
    w.clear_bullets();
    w.sfx.clear();
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::Gnaw,
        level: 1,
        timer: 0.0,
        evolved: false,
    }];
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    w.end_tick();
    assert!(w.bullets().is_empty());
    assert!(!w.sfx.contains(&Sfx::Shoot));
}

#[test]
fn nearest_enemy_skips_dead_and_nan_positions() {
    let mut w = World::new(Theme::Sewer, 29);
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(3);
    let from = w.player.pos;
    let dead = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
    edit_enemy(&mut w, dead, |enemy| enemy.hp = 0.0);
    let nan = w.spawn_enemy_with_rng(EnemyKind::Skitterer, from, 1.0, &mut rng);
    edit_enemy(&mut w, nan, |enemy| enemy.pos = Vec2::new(f32::NAN, f32::NAN));
    let alive_pos = from + Vec2::new(10.0, 0.0);
    let alive = w.spawn_enemy_with_rng(EnemyKind::Skitterer, alive_pos, 1.0, &mut rng);
    let (picked, p) = w.nearest_enemy(from).expect("should pick the live enemy");
    // Legacy asserted the `Vec` index 2; the facade returns the entity.
    assert_eq!(picked, alive);
    assert_eq!(p, alive_pos);
}

#[test]
fn on_screen_culls_offscreen_entities() {
    let vp = Vec2::new(100.0, 50.0);
    assert!(on_screen((10, 10), 2.0, vp));
    assert!(
        on_screen((-2, 10), 4.0, vp),
        "an overhanging edge is visible"
    );
    assert!(!on_screen((-10, 10), 2.0, vp));
    assert!(!on_screen((120, 10), 2.0, vp));
    assert!(!on_screen((10, 60), 3.0, vp));
}

#[test]
fn orbit_bullets_survive_the_edge_cull() {
    let mut w = World::new(Theme::Sewer, 31);
    w.player.pos = Vec2::new(50.0, 50.0);
    w.clear_bullets();
    let ppos = w.player.pos;
    let arena_x = w.level.arena.x;
    let mut orbit = Bullet::new(ppos, Vec2::ZERO, 1.0, Faction::Player, (255, 255, 255));
    orbit.orbit = Some(Orbit {
        angle: 0.0,
        radius: arena_x + 20.0,
        speed: 0.0,
    });
    w.spawn_bullet(orbit);
    let free = Bullet::new(
        Vec2::new(arena_x + 50.0, 50.0),
        Vec2::ZERO,
        1.0,
        Faction::Player,
        (255, 255, 255),
    );
    w.spawn_bullet(free);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, projectiles::bullets_system);
    w.end_tick();
    assert_eq!(w.bullets().len(), 1, "free bullet should be culled");
    assert!(
        w.bullets()[0].orbit.is_some(),
        "orbit bullet was culled at the edge"
    );
}

#[test]
fn dash_without_direction_uses_facing() {
    let mut w = World::new(Theme::Sewer, 37);
    open_map(&mut w);
    w.player.pos = w.level.arena * 0.5;
    w.player.face = Vec2::new(1.0, 0.0);
    w.player.dash_cd = 0.0;
    let x0 = w.player.pos.x;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, true);
    run(&mut w, player::player_system);
    w.end_tick();
    assert!(w.player.pos.x > x0 + 0.1, "dash with no input did not move");
    assert!(w.player.dash_cd > 0.0);
}

#[test]
fn magnetized_gem_reaches_the_player() {
    let mut w = World::new(Theme::Sewer, 53);
    w.clear_gems();
    w.player.pos = Vec2::new(200.0, 200.0);
    let ppos = w.player.pos;
    let mut g = Gem::new(ppos + Vec2::new(100.0, 0.0), 3);
    g.magnetized = true;
    w.spawn_gem(g);
    let xp0 = w.player.xp;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    for _ in 0..300 {
        run(&mut w, loot::gems_system);
        if w.gems().is_empty() {
            break;
        }
    }
    w.end_tick();
    assert!(w.gems().is_empty(), "magnetized gem was never collected");
    assert!(w.player.xp > xp0);
}

#[test]
fn exactly_stacked_enemies_are_nudged_apart() {
    let mut w = World::new(Theme::Sewer, 41);
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(4);
    let p = w.level.arena * 0.5;
    w.spawn_enemy_with_rng(EnemyKind::Cat, p, 1.0, &mut rng);
    w.spawn_enemy_with_rng(EnemyKind::Cat, p, 1.0, &mut rng);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, enemy_move::grid_build_system);
    // Legacy called `separate_enemies(&grid, 6.5)`; the system derives its
    // reach as `2 * max_radius`, so feed it the same 6.5 reach.
    w.ecs.resource_mut::<GridScratch>().max_radius = 3.25;
    run(&mut w, enemy_move::separation_system);
    w.end_tick();
    assert!(
        w.enemies()[0].pos.dist(w.enemies()[1].pos) > 0.1,
        "stacked enemies stayed glued together"
    );
}

#[test]
fn contact_knockback_stays_out_of_walls() {
    let mut w = World::new(Theme::Sewer, 43);
    open_map(&mut w);
    // One solid cell (96..112, 96..112) with the cat just left of it.
    paint_solid(&mut w, 6, 6, 1, 1);
    let wall = TileMap::tile_rect(6, 6);
    w.player.pos = Vec2::new(94.0, 100.0);
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(5);
    let e = w.spawn_enemy_with_rng(EnemyKind::Cat, Vec2::new(96.5, 100.0), 1.0, &mut rng);
    edit_enemy(&mut w, e, |enemy| enemy.contact_cd = 0.0);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, combat::contact_system);
    w.end_tick();
    let pos = w.enemies()[0].pos;
    assert!(
        !collision::circle_rect_overlap(pos, w.enemies()[0].radius, &wall),
        "knockback left the enemy inside a wall at {pos:?}"
    );
}

#[test]
fn untargeted_gnaw_retries_next_tick() {
    let mut w = World::new(Theme::Sewer, 61);
    w.clear_enemies();
    w.clear_bullets();
    w.player.loadout.weapons[0].timer = 0.0;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    w.end_tick();
    assert!(w.bullets().is_empty(), "Gnaw fired with no target");
    assert_eq!(
        w.player.loadout.weapons[0].timer, 0.0,
        "a dud shot should leave the timer ready for next tick"
    );
}

#[test]
fn iframed_player_keeps_enemy_bullets() {
    let mut w = World::new(Theme::Sewer, 67);
    w.clear_bullets();
    w.player.pos = w.level.arena * 0.5;
    let ppos = w.player.pos;
    w.spawn_bullet(Bullet::new(
        ppos,
        Vec2::ZERO,
        5.0,
        Faction::Enemy,
        (255, 0, 0),
    ));
    w.player.iframes = 0.5;
    let hp = w.player.hp;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, combat::enemy_bullet_hits_system);
    assert_eq!(
        w.ecs.resource::<Player>().hp,
        hp,
        "i-frames should block the damage"
    );
    assert!(w.bullets()[0].life > 0.0, "i-frames must not delete the bullet");

    w.ecs.resource_mut::<Player>().iframes = 0.0;
    run(&mut w, combat::enemy_bullet_hits_system);
    w.end_tick();
    assert!(w.player.hp < hp, "a vulnerable player should take the hit");
    assert_eq!(
        w.bullets()[0].life, 0.0,
        "a landed hit should consume the bullet"
    );
}

#[test]
fn iframed_player_does_not_spend_contact_cooldowns() {
    let mut w = World::new(Theme::Sewer, 71);
    open_map(&mut w);
    w.player.pos = w.level.arena * 0.5;
    w.clear_enemies();
    let mut rng = StdRng::seed_from_u64(7);
    let ppos = w.player.pos;
    let e = w.spawn_enemy_with_rng(EnemyKind::Cat, ppos, 1.0, &mut rng);
    edit_enemy(&mut w, e, |enemy| enemy.contact_cd = 0.0);
    w.player.iframes = 0.5;
    let hp = w.player.hp;
    let pos = w.enemies()[0].pos;
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, combat::contact_system);
    assert_eq!(w.ecs.resource::<Player>().hp, hp);
    assert_eq!(w.enemies()[0].contact_cd, 0.0, "i-frames spent the cooldown");
    assert_eq!(w.enemies()[0].pos, pos, "i-frames applied knockback");

    w.ecs.resource_mut::<Player>().iframes = 0.0;
    run(&mut w, combat::contact_system);
    w.end_tick();
    assert!(w.player.hp < hp, "vulnerable contact should damage");
    assert!(
        w.enemies()[0].contact_cd > 0.0,
        "a landed hit should set cooldown"
    );
}

// ---- Round 3 review fixes -------------------------------------------

#[test]
fn fast_bullet_through_a_skitterer_registers_a_hit() {
    let mut w = World::new(Theme::Sewer, 89);
    // Fixed world coordinates (below) are inside a `Solid` cell on some
    // generated maps, and a shot now stops on the first wall it meets, so the
    // swept-hit fixture needs open floor.
    open_map(&mut w);
    w.clear_enemies();
    w.clear_bullets();
    let mut rng = StdRng::seed_from_u64(8);
    w.spawn_enemy_with_rng(EnemyKind::Skitterer, Vec2::new(86.0, 100.0), 1.0, &mut rng);
    let hp = w.enemies()[0].hp;
    let mut b = Bullet::new(
        Vec2::new(80.0, 100.0),
        Vec2::new(240.0, 0.0),
        3.0,
        Faction::Player,
        (255, 255, 255),
    );
    b.radius = 2.0;
    w.spawn_bullet(b);
    w.begin_tick(0.05, Vec2::ZERO, false);
    run(&mut w, projectiles::bullets_system);
    // Endpoint-only collision misses: the bullet ends 6 px past center,
    // outside the 4.5 px combined hit radius.
    assert!(!collision::circles_overlap(
        w.bullets()[0].pos,
        2.0,
        w.enemies()[0].pos,
        2.5
    ));
    run(&mut w, enemy_move::grid_build_system);
    run(&mut w, combat::player_bullet_hits_system);
    w.end_tick();
    assert!(w.enemies()[0].hp < hp, "fast bullet passed through the enemy");
    assert_eq!(w.bullets()[0].life, 0.0);
}

#[test]
fn fast_enemy_bullet_cannot_skip_the_player() {
    let mut w = World::new(Theme::Sewer, 91);
    // See `fast_bullet_through_a_skitterer_registers_a_hit`: the fixture uses
    // fixed coordinates, so it needs open floor now that shots stop on walls.
    open_map(&mut w);
    w.clear_bullets();
    w.player.pos = Vec2::new(100.0, 100.0);
    w.player.iframes = 0.0;
    let hp = w.player.hp;
    let mut b = Bullet::new(
        Vec2::new(88.0, 100.0),
        Vec2::new(400.0, 0.0),
        5.0,
        Faction::Enemy,
        (255, 0, 0),
    );
    b.radius = 2.0;
    w.spawn_bullet(b);
    w.begin_tick(0.05, Vec2::ZERO, false);
    run(&mut w, projectiles::bullets_system);
    let ppos = w.ecs.resource::<Player>().pos;
    assert!(!collision::circles_overlap(
        w.bullets()[0].pos,
        2.0,
        ppos,
        PLAYER_RADIUS
    ));
    run(&mut w, combat::enemy_bullet_hits_system);
    w.end_tick();
    assert!(
        w.player.hp < hp,
        "fast enemy bullet passed through the player"
    );
    assert_eq!(w.bullets()[0].life, 0.0);
}

#[test]
fn pickup_scratch_vec_is_reused_across_ticks() {
    let mut w = World::new(Theme::Sewer, 97);
    w.clear_pickups();
    let ppos = w.player.pos;
    w.spawn_pickup(Pickup {
        pos: ppos,
        kind: PickupKind::Heal,
        bob: 0.0,
    });
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, loot::pickups_system);
    w.end_tick();
    let ptr = w.ecs.resource::<PickupScratch>().0.as_ptr();
    let cap = w.ecs.resource::<PickupScratch>().0.capacity();
    assert!(cap > 0, "scratch vec should be kept on the world");
    assert!(w.ecs.resource::<PickupScratch>().0.is_empty());
    w.spawn_pickup(Pickup {
        pos: ppos,
        kind: PickupKind::Heal,
        bob: 0.0,
    });
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, loot::pickups_system);
    w.end_tick();
    assert_eq!(
        w.ecs.resource::<PickupScratch>().0.as_ptr(),
        ptr,
        "scratch vec reallocated"
    );
    assert_eq!(w.ecs.resource::<PickupScratch>().0.capacity(), cap);
}

#[test]
fn acid_spit_launches_a_projectile_not_an_instant_pool() {
    let mut w = World::new(Theme::Sewer, 73);
    w.clear_enemies();
    w.clear_bullets();
    w.clear_pools();
    // Legacy `fire_one(AcidSpit, 10.0, 1.0, 1, ..)`: level 1 gives 1 lob.
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::AcidSpit,
        level: 1,
        timer: 0.0,
        evolved: false,
    }];
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    w.end_tick();
    assert_eq!(w.bullets().len(), 1);
    assert!(w.bullets()[0].lob.is_some(), "acid shot should be lobbed");
    assert!(w.pools().is_empty(), "acid pool appeared instantly");
}

#[test]
fn acid_spit_pool_lands_at_its_target_after_travel() {
    let mut w = World::new(Theme::Sewer, 79);
    w.clear_enemies();
    w.clear_bullets();
    w.clear_pools();
    open_map(&mut w);
    let ppos = w.player.pos;
    let face = Vec2::new(1.0, 0.0);
    w.player.face = face;
    let target = ppos + face * 50.0;
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::AcidSpit,
        level: 1,
        timer: 0.0,
        evolved: false,
    }];
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    let mut ticks = 0;
    while w.pools().is_empty() && ticks < 120 {
        run(&mut w, projectiles::bullets_system);
        ticks += 1;
    }
    w.end_tick();
    assert_eq!(w.pools().len(), 1, "acid pool never landed");
    assert!(
        w.pools()[0].pos.dist(target) < 1.0,
        "pool landed at {:?}, target {target:?}",
        w.pools()[0].pos
    );
    assert!(w.bullets().is_empty(), "landed projectile should be removed");
}

#[test]
fn acid_spit_pool_lands_on_a_wall_not_behind_it() {
    let mut w = World::new(Theme::Sewer, 83);
    w.clear_enemies();
    w.clear_bullets();
    w.clear_pools();
    let ppos = w.player.pos;
    let face = Vec2::new(1.0, 0.0);
    w.player.face = face;
    // A one-cell (`Solid`) wall column 20..36 px east of the rat.
    let (cx, cy) = collision::cell_of(ppos);
    paint_solid(&mut w, cx + 1, cy - 1, 1, 3);
    let wall = TileMap::tile_rect(cx + 1, cy);
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::AcidSpit,
        level: 1,
        timer: 0.0,
        evolved: false,
    }];
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, weapons::fire_weapons_system);
    let mut ticks = 0;
    while w.pools().is_empty() && ticks < 120 {
        run(&mut w, projectiles::bullets_system);
        ticks += 1;
    }
    w.end_tick();
    assert_eq!(w.pools().len(), 1, "acid pool never landed");
    assert!(
        w.pools()[0].pos.x <= wall.x + 0.01,
        "pool landed past the wall at {:?}",
        w.pools()[0].pos
    );
    assert!(w.pools()[0].pos.x > ppos.x + 10.0, "pool landed short");
}

// ---- Neon combat-feedback merge tests -------------------------------------
// Ported from the other team's `world.rs` additions (audited hunk by hunk when
// the simulation was ported). Entity setup goes through the facade
// spawn helpers; single-system steps use the narrow harness.

#[test]
fn world_accessors_report_boss_enemies_and_pickups() {
    let mut w = World::new(Theme::Sewer, 101);
    w.clear_enemies();
    assert_eq!(w.enemy_count(), 0);
    assert_eq!(w.boss_hp(), None);
    assert_eq!(w.boss_offset(), None);
    assert!(w.last_pickups().is_empty());

    let mut rng = StdRng::seed_from_u64(11);
    let bpos = w.player.pos + Vec2::new(40.0, -15.0);
    let boss = w.spawn_enemy_with_rng(EnemyKind::Boss, bpos, 1.0, &mut rng);
    edit_enemy(&mut w, boss, |enemy| enemy.hp = 900.0);
    assert_eq!(w.enemy_count(), 1);
    let (hp, max_hp) = w.boss_hp().expect("living boss should report hp");
    assert!((hp - 900.0).abs() < 1e-3);
    assert!((max_hp - 1400.0).abs() < 1e-3);
    let ofs = w.boss_offset().expect("living boss should report offset");
    assert!((ofs.x - 40.0).abs() < 1e-3 && (ofs.y + 15.0).abs() < 1e-3);

    let ppos = w.player.pos;
    w.spawn_pickup(Pickup {
        pos: ppos,
        kind: PickupKind::Heal,
        bob: 0.0,
    });
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    assert_eq!(w.last_pickups(), &[PickupKind::Heal]);
    assert!(w.sfx.contains(&Sfx::Heal));
    w.update(1.0 / 60.0, Vec2::ZERO, false);
    assert!(w.last_pickups().is_empty());
}

#[test]
fn enemy_hits_spawn_damage_numbers_and_deaths_stain_floor() {
    let mut w = World::new(Theme::Kitchen, 109);
    w.clear_enemies();
    w.clear_bullets();
    let mut rng = StdRng::seed_from_u64(12);
    let epos = w.player.pos + Vec2::new(16.0, 0.0);
    let e = w.spawn_enemy_with_rng(EnemyKind::Brute, epos, 1.0, &mut rng);
    edit_enemy(&mut w, e, |enemy| enemy.id = 77);

    let mut b = Bullet::new(
        epos - Vec2::new(2.0, 0.0),
        Vec2::new(120.0, 0.0),
        12.0,
        Faction::Player,
        WeaponKind::Gnaw.color(),
    );
    b.weapon = Some(WeaponKind::Gnaw);
    w.spawn_bullet(b);

    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, enemy_move::grid_build_system);
    run(&mut w, combat::player_bullet_hits_system);
    w.end_tick();
    assert_eq!(w.damage_numbers.len(), 1);
    assert_eq!(w.damage_numbers.items[0].display_value(), 12);

    // Killing the brute leaves a floor stain and shockwave.
    edit_enemy(&mut w, e, |enemy| enemy.hp = -1.0);
    w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
    run(&mut w, loot::reap_enemies_system);
    w.end_tick();
    assert!(
        !w.particles.stains.is_empty(),
        "death should stain the floor"
    );
    assert!(
        !w.particles.shockwaves.is_empty(),
        "brute death should emit a shockwave"
    );
}

#[test]
fn weapons_emit_specialized_sfx() {
    let mut w = World::new(Theme::Lab, 113);
    let mut rng = StdRng::seed_from_u64(14);
    let ppos = w.player.pos;
    let cases = [
        (WeaponKind::Gnaw, Sfx::Shoot),
        (WeaponKind::CheeseSpray, Sfx::ShootSpray),
        (WeaponKind::SqueakNova, Sfx::ShootNova),
        (WeaponKind::SporeOrbit, Sfx::ShootOrbit),
        (WeaponKind::TailWhip, Sfx::ShootWhip),
        (WeaponKind::AcidSpit, Sfx::ShootAcid),
    ];
    for (kind, expected) in cases {
        w.clear_enemies();
        w.clear_bullets();
        w.spawn_enemy_with_rng(
            EnemyKind::Skitterer,
            ppos + Vec2::new(20.0, 0.0),
            1.0,
            &mut rng,
        );
        w.sfx.clear();
        w.player.loadout.weapons = vec![Weapon {
            kind,
            level: 1,
            timer: 0.0,
            evolved: false,
        }];
        w.begin_tick(1.0 / 60.0, Vec2::ZERO, false);
        run(&mut w, weapons::fire_weapons_system);
        w.end_tick();
        assert!(
            w.sfx.contains(&expected),
            "{kind:?} did not emit {expected:?} (got {:?})",
            w.sfx
        );
    }
}

/// Visual showcase dumper for combat feedback iteration:
/// `RATRUN_DUMP=vfx_sheet|vfx_combat|vfx_nuke|vfx_kitchen|vfx_lab cargo test dump_vfx_showcase`
#[test]
fn dump_vfx_showcase() {
    let mode = match std::env::var("RATRUN_DUMP") {
        Ok(m) if m.starts_with("vfx_") => m,
        _ => return,
    };
    let save_ppm = |pb: &PixelBuffer, path: &str| {
        let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
        for y in 0..pb.h {
            for x in 0..pb.w {
                let c = pb.pixel_at(x, y);
                out.push(c.0);
                out.push(c.1);
                out.push(c.2);
            }
        }
        std::fs::write(path, out).unwrap();
    };

    if mode == "vfx_sheet" {
        let mut w = World::new(Theme::Sewer, 7);
        open_map(&mut w);
        w.level.props.clear();
        let c = w.level.arena * 0.5;
        // Camera centers on `w.player.pos`, so place the dashing rat at `c`
        // and arrange all 3 showcase rows symmetrically inside 220x120.
        w.player.pos = c;
        w.player.face = Vec2::new(1.0, -0.1).normalized();
        w.player.dash_active = 0.12;
        let mut rng = StdRng::seed_from_u64(42);

        // Row 1 (top, y = c.y - 34): Enemies + damage numbers (normal, kill, medium, acid, big kill, boss nuke).
        let samples: &[(EnemyKind, f32, bool, DamageSource)] = &[
            (EnemyKind::Skitterer, 5.0, false, DamageSource::Normal),
            (EnemyKind::Bat, 7.0, true, DamageSource::Normal),
            (EnemyKind::Cat, 14.0, false, DamageSource::Normal),
            (EnemyKind::Spitter, 18.0, false, DamageSource::Acid),
            (EnemyKind::Brute, 34.0, true, DamageSource::TailWhip),
            (EnemyKind::Boss, 200.0, false, DamageSource::Nuke),
        ];
        for (i, &(kind, dmg, kill, src)) in samples.iter().enumerate() {
            let p = Vec2::new(c.x - 86.0 + i as f32 * 32.0, c.y - 30.0);
            let e = w.spawn_enemy_with_rng(kind, p, 1.0, &mut rng);
            edit_enemy(&mut w, e, |enemy| {
                enemy.id = (i + 1) as u32;
                enemy.facing = Vec2::new(0.2, 1.0).normalized();
                if i == 0 || i == 4 {
                    enemy.flash = 0.1;
                }
            });
            let id = e_id(&w, e);
            let radius = w.enemies()[i].radius;
            w.damage_numbers.record(id, p, radius, dmg, kill, src);
        }

        // Row 2 (middle, y = c.y): Left/right weapon hit sparks + center dashing rat + right acid splash.
        let weapon_xs = [-88.0, -64.0, -40.0, 32.0, 56.0, 82.0];
        for (i, &wk) in WeaponKind::all().iter().enumerate() {
            let p = Vec2::new(c.x + weapon_xs[i], c.y + 2.0);
            vfx::emit_weapon_hit(
                &mut w.particles,
                &mut rng,
                p,
                Vec2::new(100.0, 0.0),
                Some(wk),
                EnemyKind::Cat,
                i % 2 == 1,
            );
        }
        vfx::emit_dash_start(
            &mut w.particles,
            &mut rng,
            c - Vec2::new(5.0, 0.0),
            w.player.face,
            1.0,
        );
        for k in 0..5 {
            vfx::emit_dash_trail(
                &mut w.particles,
                &mut rng,
                c - Vec2::new(k as f32 * 3.2, 0.0),
                w.player.face * 130.0,
                1.0 / 60.0,
                1.0,
            );
        }

        // Row 3 (bottom, y = c.y + 34): Acid splash, level-up burst, brute death + stain, heal & magnet.
        vfx::emit_acid_splash(&mut w.particles, &mut rng, c + Vec2::new(-78.0, 34.0), 12.0);
        vfx::emit_level_up(&mut w.particles, &mut rng, c + Vec2::new(-28.0, 34.0));
        vfx::emit_enemy_death(
            &mut w.particles,
            &mut rng,
            c + Vec2::new(26.0, 34.0),
            6.5,
            EnemyKind::Brute,
            w.level.palette.blood,
        );
        vfx::emit_pickup_heal(&mut w.particles, &mut rng, c + Vec2::new(78.0, 34.0));

        // Advance 3 frames so shockwaves, streaks, and numbers are crisp mid-pop.
        for _ in 0..3 {
            w.particles.update(1.0 / 60.0);
            w.damage_numbers.update(1.0 / 60.0);
        }
        let mut pb = PixelBuffer::new(220, 120);
        w.draw(&mut pb);
        save_ppm(&pb, "/tmp/ratrun_vfx.ppm");
        return;
    }

    if mode == "vfx_boss_death" {
        let mut w = World::new(Theme::Lab, 99);
        open_map(&mut w);
        w.level.props.clear();
        let c = w.level.arena * 0.5;
        w.player.pos = c + Vec2::new(-44.0, 12.0);
        w.player.face = Vec2::new(1.0, -0.2).normalized();
        let mut rng = StdRng::seed_from_u64(77);
        // Boss death centerpiece on right + Magnet pickup & Gem sparkles on left + Player hurt.
        let bpos = c + Vec2::new(14.0, -6.0);
        vfx::emit_enemy_death(
            &mut w.particles,
            &mut rng,
            bpos,
            14.0,
            EnemyKind::Boss,
            w.level.palette.blood,
        );
        w.damage_numbers
            .record(500, bpos, 14.0, 200.0, true, DamageSource::Nuke);
        vfx::emit_pickup_magnet(
            &mut w.particles,
            &mut rng,
            w.player.pos + Vec2::new(-32.0, -24.0),
        );
        vfx::emit_gem_collect(
            &mut w.particles,
            &mut rng,
            w.player.pos + Vec2::new(-10.0, -24.0),
            25,
        );
        vfx::emit_player_hurt(&mut w.particles, &mut rng, w.player.pos);
        // Enemy death gallery along the bottom: Skitterer, Bat, Cat, Spitter, Elite Brute.
        let kinds = [
            (EnemyKind::Skitterer, false),
            (EnemyKind::Bat, false),
            (EnemyKind::Cat, false),
            (EnemyKind::Spitter, false),
            (EnemyKind::Brute, true),
        ];
        for (i, &(k, elite)) in kinds.iter().enumerate() {
            let p = w.player.pos + Vec2::new(-64.0 + i as f32 * 34.0, 34.0);
            vfx::emit_enemy_death(
                &mut w.particles,
                &mut rng,
                p,
                k.radius(),
                k,
                w.level.palette.blood,
            );
            if elite {
                vfx::emit_elite_death_accent(&mut w.particles, &mut rng, p);
            }
        }
        for _ in 0..6 {
            w.particles.update(1.0 / 60.0);
            w.damage_numbers.update(1.0 / 60.0);
        }
        let mut pb = PixelBuffer::new(220, 120);
        w.draw(&mut pb);
        save_ppm(&pb, "/tmp/ratrun_vfx.ppm");
        return;
    }

    let theme = match mode.as_str() {
        "vfx_kitchen" => Theme::Kitchen,
        "vfx_lab" => Theme::Lab,
        _ => Theme::Sewer,
    };
    let mut w = World::new(theme, 2025);
    // Equip a rich mid-game build so every weapon + pool + orbit + whip is active.
    for wk in [
        WeaponKind::CheeseSpray,
        WeaponKind::SqueakNova,
        WeaponKind::SporeOrbit,
        WeaponKind::TailWhip,
        WeaponKind::AcidSpit,
    ] {
        let _ = w
            .player
            .loadout
            .apply(&crate::game::loadout::Upgrade::NewWeapon(wk));
    }
    w.elapsed = 125.0; // brutes + spitters + dense horde active
    let dt = 1.0 / 60.0;
    for tick in 0..420u32 {
        let dir = Vec2::from_angle(tick as f32 * 0.035);
        let dash = tick % 110 == 40;
        w.update(dt, dir, dash);
        w.player.hp = w.player.max_hp();
        apply_pending(&mut w);
    }
    if mode == "vfx_nuke" {
        let bpos = w.player.pos + Vec2::new(36.0, -14.0);
        let boss = w.spawn_enemy(EnemyKind::Boss, bpos, 1.0);
        edit_enemy(&mut w, boss, |enemy| enemy.id = 9999);
        let ppos = w.player.pos;
        w.spawn_pickup(Pickup {
            pos: ppos,
            kind: PickupKind::Nuke,
            bob: 0.0,
        });
        for _ in 0..7 {
            w.update(dt, Vec2::new(1.0, 0.0), false);
        }
    }
    let mut pb = PixelBuffer::new(200, 110);
    w.draw(&mut pb);
    save_ppm(&pb, "/tmp/ratrun_vfx.ppm");
}

/// Read the runtime `Enemy::id` of a spawned entity (test helper).
fn e_id(w: &World, entity: Entity) -> u32 {
    w.ecs.get::<Enemy>(entity).map(|e| e.id).unwrap_or(0)
}



// ---- Wave 3 controls: aiming is input ------------------------------------

/// A seeded run that **aims** must replay exactly.
///
/// Aim is input like `move_dir`/`dash`, so the same seed and the same scripted
/// aim trace must produce the same run tick for tick. The digests cover the
/// rat's position/facing, every body's position and hp (in `SpawnId` order),
/// the projectile count and the score — everything the aim touches.
#[test]
fn the_same_seed_and_aim_replays_the_same_fight() {
    use std::hash::{Hash, Hasher};

    fn digest(seed: u64) -> u64 {
        use crate::game::weapon::WeaponKind;
        let mut w = World::new(Theme::Kitchen, seed);
        w.player.pos = Vec2::new(100.0, 100.0);
        w.clear_enemies();
        w.player.loadout.weapons.clear();
        for kind in [
            WeaponKind::Gnaw,
            WeaponKind::CheeseSpray,
            WeaponKind::AcidSpit,
            WeaponKind::AcidRain,
            WeaponKind::Hantavirus,
        ] {
            w.player.loadout.weapons.push(crate::game::weapon::Weapon {
                kind,
                level: 3,
                timer: 0.0,
                evolved: false,
            });
        }
        // A ring of bodies so aiming actually changes which one is picked.
        for i in 0..8u32 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let p = w.player.pos + Vec2::from_angle(a) * (28.0 + i as f32 * 3.0);
            w.spawn_enemy(EnemyKind::Skitterer, p, 40.0);
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for tick in 0..420u32 {
            // A scripted reticle: a slow sweep across the ring, so a different
            // aim picks a different body on a different tick.
            let a = tick as f32 * 0.031;
            let aim = w.player.pos + Vec2::from_angle(a) * 70.0;
            let move_dir = if tick % 90 < 45 {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::new(0.0, 1.0)
            };
            w.update_with_aim(1.0 / 60.0, move_dir, false, false, Some(aim));
            w.player.pos = w.player.pos.clamp_box(
                Vec2::splat(24.0),
                w.level.arena - Vec2::splat(24.0),
            );
            if w.pending_levelups > 0 {
                let c = w.player.loadout.generate_choices(&mut w.rng);
                if let Some(u) = c.first() {
                    w.player.loadout.apply(u);
                }
                w.pending_levelups = 0;
            }
            w.player.pos.x.to_bits().hash(&mut hasher);
            w.player.pos.y.to_bits().hash(&mut hasher);
            w.player.face.x.to_bits().hash(&mut hasher);
            w.player.face.y.to_bits().hash(&mut hasher);
            for e in w.enemies() {
                e.pos.x.to_bits().hash(&mut hasher);
                e.pos.y.to_bits().hash(&mut hasher);
                e.hp.to_bits().hash(&mut hasher);
            }
            w.bullets().len().hash(&mut hasher);
            w.score.hash(&mut hasher);
            w.kills.hash(&mut hasher);
        }
        hasher.finish()
    }

    for seed in [5u64, 77, 90210] {
        assert_eq!(
            digest(seed),
            digest(seed),
            "the same seed and the same scripted aim diverged (seed {seed})"
        );
    }
}

/// Aim changes the fight (or the twin-run would be vacuous): the same seed with
/// two different aim scripts must not produce the same trace.
#[test]
fn a_different_aim_changes_the_run() {
    fn trace(aim_step: f32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut w = World::new(Theme::Kitchen, 5);
        w.clear_enemies();
        w.player.loadout.weapons.clear();
        w.player.loadout.weapons.push(crate::game::weapon::Weapon {
            kind: crate::game::weapon::WeaponKind::Gnaw,
            level: 5,
            timer: 0.0,
            evolved: false,
        });
        for i in 0..8u32 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let p = w.player.pos + Vec2::from_angle(a) * 40.0;
            w.spawn_enemy(EnemyKind::Skitterer, p, 60.0);
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for tick in 0..180u32 {
            let a = tick as f32 * aim_step;
            let aim = w.player.pos + Vec2::from_angle(a) * 60.0;
            w.update_with_aim(1.0 / 60.0, Vec2::ZERO, false, false, Some(aim));
            for e in w.enemies() {
                e.hp.to_bits().hash(&mut hasher);
            }
        }
        hasher.finish()
    }
    assert_ne!(trace(0.01), trace(1.4), "aim did not change the fight");
}

/// `World::update`/`update_with` keep passing `None`, so the auto-aim path is
/// byte-identical to the pre-wave-3 sim (the scripted traces depend on it).
#[test]
fn no_aim_is_the_auto_aim_path() {
    let mut a = World::new(Theme::Sewer, 3);
    let mut b = World::new(Theme::Sewer, 3);
    for _ in 0..120 {
        a.update(1.0 / 60.0, Vec2::new(1.0, 0.0), false);
        b.update_with_aim(1.0 / 60.0, Vec2::new(1.0, 0.0), false, false, None);
    }
    assert_eq!(a.player.pos, b.player.pos);
    assert_eq!(a.player.face, b.player.face);
    assert_eq!(a.score, b.score);
    assert_eq!(a.enemies().len(), b.enemies().len());
}
