//! Narrow single-system tests for the five wave-3 enemies (`WAVE3_DESIGN.md`
//! §9): behaviour-state timings, the damage numbers, that every tell is visible
//! before it bites, the two-at-once limiter, the turnkey's exit rule and
//! twin-run determinism.
//!
//! The harness is the one the rest of `systems/*` uses: `begin_tick` → the
//! system via `run_system_cached` → `end_tick`, asserting on the facade.

use super::*;
use crate::game::collision::TileMap;
use crate::game::ecs::components::{SpawnId, Wick};
use crate::game::ecs::systems::enemy_wave3 as w3;
use crate::game::enemy::EnemyKind;
use crate::game::level::{Theme, MAP_COLS, MAP_ROWS};
use crate::game::world::World;
use crate::math::Vec2;

const DT: f32 = 1.0 / 60.0;

/// Strip the generated layout's walls: these tests measure one system, not the
/// map.
fn open_floor(w: &mut World) {
    w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
}

fn run<M>(w: &mut World, system: impl IntoSystem<(), (), M> + 'static) {
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs.run_system_cached(system).unwrap();
    w.end_tick();
}

/// One wave-3 behaviour tick.
fn tick(w: &mut World) {
    run(w, w3::wave3_system);
}

fn ticks(w: &mut World, n: usize) {
    for _ in 0..n {
        tick(w);
    }
}

/// Seconds → ticks (rounded up, so "run the tell out" always passes it).
fn secs(s: f32) -> usize {
    (s / DT).ceil() as usize
}

fn put<F: FnOnce(&mut Enemy)>(w: &mut World, entity: Entity, f: F) {
    let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
    f(&mut e);
}

fn state(w: &World, entity: Entity) -> Enemy {
    w.ecs.get::<Enemy>(entity).unwrap().clone()
}

fn wick_count(w: &mut World) -> usize {
    let mut q = w.ecs.query::<(Entity, &Wick)>();
    q.iter(&w.ecs).count()
}

/// Fill `SpatialGrid` + `EnemyIndex` + `GridScratch` the way
/// `enemy_move::grid_build_system` does (in `SpawnId` order), without depending
/// on that (separately owned) system. Call outside a tick.
fn rebuild_broadphase(w: &mut World) {
    use crate::game::collision::SpatialGrid;
    use crate::game::ecs::constants::GRID_CELL;
    use crate::game::ecs::resources::{EnemyIndex, GridScratch};
    let arena = w.level.arena;
    let mut q = w.ecs.query::<(Entity, &SpawnId, &Enemy)>();
    let mut items: Vec<(u64, Entity, Vec2, f32)> = q
        .iter(&w.ecs)
        .map(|(entity, id, e)| (id.0, entity, e.pos, e.radius))
        .collect();
    items.sort_by_key(|item| item.0);
    {
        let mut grid = w.ecs.resource_mut::<SpatialGrid>();
        grid.reset(arena, GRID_CELL);
        grid.fill(items.iter().map(|item| item.2));
    }
    let mut index = w.ecs.resource_mut::<EnemyIndex>();
    index.0 = items.iter().map(|item| item.1).collect();
    w.ecs.resource_mut::<GridScratch>().max_radius =
        items.iter().map(|item| item.3).fold(0.0f32, f32::max);
}

/// A body ready to fire its special this very tick.
fn ready<F: FnOnce(&mut Enemy)>(w: &mut World, entity: Entity, init: F) {
    put(w, entity, |e| {
        e.w3.armed = 1;
        init(e);
    });
}

// ---- Pillbug -------------------------------------------------------------

/// At 24 px the pillbug curls for 1.4 s and holds still; it does not curl
/// again until its cooldown is spent, so it walks between shells.
#[test]
fn the_pillbug_curls_at_its_range_and_holds_its_ground() {
    let mut w = World::new(Theme::Sewer, 5);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    // Just inside the 24 px trigger.
    let bug = w.spawn_enemy(EnemyKind::Pillbug, p + Vec2::new(20.0, 0.0), 1.0);
    ready(&mut w, bug, |e| e.w3.curl_cd = 0.0);

    tick(&mut w);
    let s = state(&w, bug);
    assert!(s.w3.curl_t > 0.0, "the pillbug did not curl at 20 px");
    assert!(s.w3.holds(), "a curled pillbug must hold its position");
    assert_eq!(s.vel, Vec2::ZERO, "a curled pillbug must not move");
    assert!(w.sfx.contains(&crate::audio::Sfx::Warn),
        "the curl must play its tell");

    // The curl lasts PILLBUG_CURL_SECS; then the cooldown starts.
    ticks(&mut w, secs(PILLBUG_CURL_SECS) + 2);
    let s = state(&w, bug);
    assert_eq!(s.w3.curl_t, 0.0, "the curl overran its 1.4 s");
    assert!(
        s.w3.curl_cd > PILLBUG_CURL_COOLDOWN - 0.2,
        "the cooldown did not start: {}",
        s.w3.curl_cd
    );

    // Still next to the rat, but the cooldown keeps it walking.
    ticks(&mut w, secs(1.0));
    assert_eq!(
        state(&w, bug).w3.curl_t,
        0.0,
        "the pillbug re-curled inside its cooldown"
    );
    // Once the cooldown is spent it curls again.
    ticks(&mut w, secs(PILLBUG_CURL_COOLDOWN) + 2);
    assert!(
        state(&w, bug).w3.curl_t > 0.0,
        "the pillbug never curled again after its cooldown"
    );
}

/// The shell: the frontal 120° arc quarters incoming damage, everything else
/// doubles it, and only while curled.
#[test]
fn the_shell_quarters_the_front_and_doubles_the_rear() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(3);
    let mut bug = Enemy::spawn(EnemyKind::Pillbug, Vec2::ZERO, 1.0, &mut rng);
    bug.facing = Vec2::new(1.0, 0.0);

    // Walking: no armour at all.
    assert_eq!(bug.damage_mult_from(Vec2::new(10.0, 0.0)), 1.0);
    assert_eq!(bug.damage_mult_from(Vec2::new(-10.0, 0.0)), 1.0);

    bug.w3.curl_t = PILLBUG_CURL_SECS;
    // Dead ahead and 45° off it: inside the 120° cone.
    assert_eq!(
        bug.damage_mult_from(Vec2::new(10.0, 0.0)),
        PILLBUG_FRONT_MULT
    );
    let diag = Vec2::new(6.0, 6.0);
    assert_eq!(bug.damage_mult_from(diag), PILLBUG_FRONT_MULT);
    // The shoulder and the rear take double.
    assert_eq!(bug.damage_mult_from(Vec2::new(0.0, 10.0)), PILLBUG_REAR_MULT);
    assert_eq!(
        bug.damage_mult_from(Vec2::new(-10.0, 0.0)),
        PILLBUG_REAR_MULT
    );

    // And `hurt_from` books exactly what it reports.
    let hp = bug.hp;
    let applied = bug.hurt_from(8.0, Vec2::new(10.0, 0.0));
    assert!((applied - 2.0).abs() < 1e-4, "front hit applied {applied}");
    assert!((hp - bug.hp - 2.0).abs() < 1e-4, "front hit took {}", hp - bug.hp);
    let applied = bug.hurt_from(8.0, Vec2::new(-10.0, 0.0));
    assert!((applied - 16.0).abs() < 1e-4, "rear hit applied {applied}");
}

/// Every damage source routes through `hurt_from`/`damage_mult_from`, so a
/// bullet fired into the shell is quartered and the same bullet from behind is
/// doubled. (The pool, melee, active, trap, hazard and nuke sites all call the
/// same two helpers; this is the end-to-end witness.)
#[test]
fn a_bullet_obeys_the_shell() {
    use crate::game::bullet::{Bullet, Faction};
    use crate::game::ecs::systems::combat;

    for (label, shell_side, expect) in [
        ("frontal", 1.0f32, 10.0 * PILLBUG_FRONT_MULT),
        ("rear", -1.0, 10.0 * PILLBUG_REAR_MULT),
    ] {
        let mut w = World::new(Theme::Sewer, 17);
        open_floor(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        let bug = w.spawn_enemy(EnemyKind::Pillbug, p + Vec2::new(40.0, 0.0), 1.0);
        put(&mut w, bug, |e| {
            e.facing = Vec2::new(1.0, 0.0);
            e.w3.curl_t = PILLBUG_CURL_SECS;
            e.hp = 100.0;
            e.max_hp = 100.0;
        });
        // A 10-damage dart arriving from the shell's side (`+x`) or from
        // behind (`-x`); the pillbug faces `+x`.
        let hit = state(&w, bug).pos + Vec2::new(shell_side * 6.0, 0.0);
        w.spawn_bullet(Bullet::new(
            hit,
            Vec2::new(-shell_side * 200.0, 0.0),
            10.0,
            Faction::Player,
            (255, 255, 200),
        ));
        rebuild_broadphase(&mut w);
        run(&mut w, combat::player_bullet_hits_system);

        let e = state(&w, bug);
        let lost = 100.0 - e.hp;
        assert!(
            (lost - expect).abs() < 0.01,
            "{label} hit took {lost}, expected {expect}"
        );
        // ... and the floating number shows what landed, not the nominal hit.
        assert_eq!(
            w.damage_numbers.items[0].total_dmg, lost,
            "{label}: damage number disagrees with the HP lost"
        );
    }
}

// ---- Bell Flea -----------------------------------------------------------

/// The crosses are dotted out for FLEA_TELL_SECS before the first impact, the
/// first impact lands at the locked point, and the rat takes nothing during the
/// tell.
#[test]
fn the_flea_telegraphs_both_crosses_before_they_bite() {
    let mut w = World::new(Theme::Sewer, 7);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.player.hp = 100.0;
    // 20 px away: the locked point is the rat's own spot (reach is clamped to
    // the distance), so the first cross lands on the rat.
    let flea = w.spawn_enemy(EnemyKind::BellFlea, p + Vec2::new(20.0, 0.0), 1.0);
    ready(&mut w, flea, |e| e.w3.flea_cd = 0.0);

    tick(&mut w);
    let s = state(&w, flea);
    assert_eq!(s.w3.flea_phase, 1, "the flea did not wind up");
    assert!(
        s.w3.hop_to.dist(p) < 1.0,
        "the locked point should be the rat's cell: {:?}",
        s.w3.hop_to
    );
    assert_eq!(s.pos, p + Vec2::new(20.0, 0.0), "it moved during the tell");

    // The whole tell: no damage, the body still at home.
    ticks(&mut w, secs(FLEA_TELL_SECS) - 2);
    assert_eq!(w.player.hp, 100.0, "the cross bit before its tell ended");
    assert_eq!(state(&w, flea).w3.flea_phase, 1);

    // The landing tick: 7 to the rat, the body on the locked point.
    ticks(&mut w, 3);
    assert_eq!(
        w.player.hp,
        100.0 - FLEA_CROSS_DAMAGE,
        "the first landing should take {FLEA_CROSS_DAMAGE}"
    );
    let s = state(&w, flea);
    assert_eq!(s.w3.flea_phase, 2);
    assert!(s.pos.dist(p) < 1.5, "it did not land on the cross");
}

/// The second jump goes back to where it started, springs back on its own
/// timer, and the whole cycle waits out FLEA_JUMP_PERIOD.
#[test]
fn the_flea_springs_home_and_waits_out_its_period() {
    let mut w = World::new(Theme::Sewer, 9);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.player.hp = 100.0;
    let home = p + Vec2::new(20.0, 0.0);
    let flea = w.spawn_enemy(EnemyKind::BellFlea, home, 1.0);
    ready(&mut w, flea, |e| e.w3.flea_cd = 0.0);

    // Tell + landing.
    ticks(&mut w, secs(FLEA_TELL_SECS) + 2);
    assert_eq!(state(&w, flea).w3.flea_phase, 2);
    assert_eq!(w.player.hp, 100.0 - FLEA_CROSS_DAMAGE);

    // The spring back: the second cross bites at home, so a rat that stays
    // where the flea started takes it too (the design's "leave the second
    // empty").
    w.player.pos = home;
    w.player.iframes = 0.0;
    ticks(&mut w, secs(FLEA_GAP_SECS) + 2);
    let s = state(&w, flea);
    assert_eq!(s.w3.flea_phase, 0, "the flea did not reset its cycle");
    assert!(
        s.pos.dist(home) < 1.5,
        "the flea did not spring back home: {:?}",
        s.pos
    );
    assert_eq!(
        w.player.hp,
        100.0 - 2.0 * FLEA_CROSS_DAMAGE,
        "the second cross did not bite"
    );
    assert!(
        s.w3.flea_cd > FLEA_JUMP_PERIOD - 0.2,
        "the jump period did not start: {}",
        s.w3.flea_cd
    );
}

/// A hop whose landing spot is inside a wall is refused (and costs no rng), so
/// the flea can never teleport into masonry or onto a jamb.
#[test]
fn the_flea_never_hops_into_a_wall() {
    let mut w = World::new(Theme::Sewer, 11);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    let flea = w.spawn_enemy(EnemyKind::BellFlea, p + Vec2::new(20.0, 0.0), 1.0);
    ready(&mut w, flea, |e| e.w3.flea_cd = 0.0);
    // Wall the rat's own cell off: the 20 px hop lands in solid rock.
    let (cx, cy) = crate::game::collision::cell_of(p);
    w.level.tiles.set(cx, cy, crate::game::collision::TileKind::Wall(
        crate::game::art_walls::WallKind::Solid,
    ));

    tick(&mut w);
    let s = state(&w, flea);
    assert_eq!(s.w3.flea_phase, 0, "the flea hopped into a wall");
    assert!(s.w3.flea_cd > 0.0, "the refused attempt should retry");
}

// ---- Wick Moth -----------------------------------------------------------

/// A dead moth leaves a wick that flashes for a second and then bursts for 9 to
/// the rat and 20 to enemies — and a moth killed by that burst lights its own
/// full fuse.
#[test]
fn a_wick_burst_hurts_both_sides_and_chains_its_own_fuse() {
    let mut w = World::new(Theme::Sewer, 13);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.player.hp = 100.0;
    let what = p + Vec2::new(6.0, 0.0);
    let first = w.spawn_enemy(EnemyKind::WickMoth, what, 1.0);
    // A second moth close enough to die inside the burst (a moth has 12 hp and
    // the burst deals 20), and a skitterer to prove the friendly fire.
    let second = w.spawn_enemy(EnemyKind::WickMoth, what + Vec2::new(4.0, 0.0), 1.0);
    let bug = w.spawn_enemy(EnemyKind::Skitterer, what + Vec2::new(-3.0, 0.0), 1.0);
    assert_eq!(wick_count(&mut w), 0);

    // Kill the first moth: the corpse arms its own fuse, nothing else yet.
    put(&mut w, first, |e| {
        e.hurt(99.0);
    });
    tick(&mut w);
    assert_eq!(wick_count(&mut w), 1, "a dead moth must leave a wick");
    assert_eq!(w.player.hp, 100.0, "the wick must flash before it bursts");

    // The fuse burns for a second, then the burst resolves both ways.
    ticks(&mut w, secs(WICK_FUSE_SECS) + 2);
    assert_eq!(
        w.player.hp,
        100.0 - WICK_RAT_DAMAGE,
        "the burst should take {WICK_RAT_DAMAGE} off the rat"
    );
    let bug_hp = state(&w, bug).hp;
    assert!(
        (bug_hp - (EnemyKind::Skitterer.base_hp() - WICK_ENEMY_DAMAGE)).abs() < 1e-3,
        "the burst should take {WICK_ENEMY_DAMAGE} off a nearby enemy, left {bug_hp}"
    );
    // The moth it killed lit its own full fuse: one wick remains, at full
    // length (the burst is the second moth's, not a chain of the first).
    let mut q = w.ecs.query::<&Wick>();
    let fuses: Vec<f32> = q.iter(&w.ecs).map(|k| k.fuse).collect();
    assert_eq!(fuses.len(), 1, "expected the second moth's own wick");
    assert!(
        fuses[0] > WICK_FUSE_SECS - 2.0 * DT,
        "the chained wick did not get a full fuse: {}",
        fuses[0]
    );
    assert!(!state(&w, second).alive());

    // And that second wick bursts in turn.
    w.player.iframes = 0.0;
    ticks(&mut w, secs(WICK_FUSE_SECS) + 1);
    assert_eq!(wick_count(&mut w), 0, "the chained wick never burst");
    assert_eq!(w.player.hp, 100.0 - 2.0 * WICK_RAT_DAMAGE);
}

/// A moth killed by a pool (a damage-over-time source that never calls `hurt`)
/// still lights its fuse, so no weapon can dodge the burst.
#[test]
fn a_moth_killed_by_any_source_still_lights_its_fuse() {
    use crate::game::ecs::components::Pool;
    let mut w = World::new(Theme::Sewer, 15);
    open_floor(&mut w);
    w.clear_enemies();
    w.clear_pools();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    let moth = w.spawn_enemy(EnemyKind::WickMoth, p + Vec2::new(60.0, 0.0), 1.0);
    w.spawn_pool(Pool {
        pos: p + Vec2::new(60.0, 0.0),
        radius: 20.0,
        dps: 400.0,
        life: 2.0,
        color: (140, 220, 70),
    });
    // `pools_system` walks the cached `EnemyIndex` (that is its ordering
    // contract), so a narrow test has to provide it.
    rebuild_broadphase(&mut w);
    for _ in 0..3 {
        run(&mut w, crate::game::ecs::systems::projectiles::pools_system);
    }
    assert!(!state(&w, moth).alive(), "the pool should have killed it");
    tick(&mut w);
    assert_eq!(wick_count(&mut w), 1, "the pool kill must still leave a wick");
}

// ---- Porter --------------------------------------------------------------

/// The cube goes up for 0.9 s, then the heal lands; a hit on the porter in the
/// meantime cancels it and costs it a retry, not its whole cycle.
#[test]
fn the_porter_heals_after_the_raise_and_flinches_when_hit() {
    let mut w = World::new(Theme::Sewer, 19);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    let porter = w.spawn_enemy(EnemyKind::Porter, p + Vec2::new(40.0, 0.0), 1.0);
    let ally = w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(50.0, 0.0), 1.0);
    put(&mut w, ally, |e| e.hp = 2.0);
    ready(&mut w, porter, |e| e.w3.heal_cd = 0.0);

    tick(&mut w);
    let s = state(&w, porter);
    assert!(s.w3.raise_t > 0.0, "the porter did not raise the cube");
    assert!(s.w3.holds(), "a raising porter holds position");
    assert_eq!(state(&w, ally).hp, 2.0, "the heal landed before the tell");

    ticks(&mut w, secs(PORTER_RAISE_SECS) + 2);
    assert_eq!(
        state(&w, ally).hp,
        EnemyKind::Skitterer.base_hp(),
        "the heal should top the ally up to its max"
    );
    let s = state(&w, porter);
    assert_eq!(s.w3.raise_t, 0.0);
    assert!(
        s.w3.heal_cd > PORTER_PERIOD - 0.2,
        "the porter's period did not start"
    );

    // Interrupt: a hit during the raise cancels it and starts a short flinch.
    put(&mut w, ally, |e| e.hp = 2.0);
    ready(&mut w, porter, |e| e.w3.heal_cd = 0.0);
    tick(&mut w);
    assert!(state(&w, porter).w3.raise_t > 0.0);
    put(&mut w, porter, |e| {
        e.hurt(1.0);
    });
    let s = state(&w, porter);
    assert_eq!(s.w3.raise_t, 0.0, "the hit did not interrupt the raise");
    assert!(
        (s.w3.heal_cd - PORTER_INTERRUPT_COOLDOWN).abs() < 1e-3,
        "the interrupt should set the flinch window, got {}",
        s.w3.heal_cd
    );
    ticks(&mut w, secs(PORTER_INTERRUPT_COOLDOWN * 0.8));
    assert_eq!(
        state(&w, ally).hp,
        2.0,
        "the interrupted treatment healed anyway"
    );
}

// ---- Turnkey -------------------------------------------------------------

/// The dotted bar shows for 0.8 s, then bites the rat once, then lifts.
#[test]
fn the_turnkey_telegraphs_its_bar_and_bites_once() {
    let mut w = World::new(Theme::Sewer, 23);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.player.hp = 100.0;
    let key = w.spawn_enemy(EnemyKind::Turnkey, p + Vec2::new(40.0, 0.0), 1.0);
    ready(&mut w, key, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w3.bar_cd = 0.0;
    });

    tick(&mut w);
    let s = state(&w, key);
    assert!(s.w3.bar_telling(), "the bar has to be dotted first");
    let centre = s.w3.bar_pos;
    // Stand on the bar's line, well after its tell started.
    w.player.pos = centre + Vec2::new(0.0, 6.0);
    ticks(&mut w, secs(CROSSBAR_TELL_SECS) - 2);
    assert_eq!(w.player.hp, 100.0, "the bar bit during its tell");
    assert!(state(&w, key).w3.bar_telling());

    // Live: the crossing bites for 8, exactly once per bar.
    ticks(&mut w, 3);
    assert_eq!(
        w.player.hp,
        100.0 - CROSSBAR_DAMAGE,
        "the live bar should bite"
    );
    w.player.iframes = 0.0;
    ticks(&mut w, secs(CROSSBAR_LIVE_SECS) - 6);
    assert_eq!(
        w.player.hp,
        100.0 - CROSSBAR_DAMAGE,
        "the bar bit the same rat twice"
    );
    // And it lifts.
    ticks(&mut w, 8);
    let s = state(&w, key);
    assert_eq!(s.w3.bar_t, 0.0, "the bar never lifted");
    assert!(
        s.w3.bar_cd > TURNKEY_PERIOD - 0.3,
        "the bar period did not start: {}",
        s.w3.bar_cd
    );
}

/// A bar that would fill an exit is refused: a 22 px bar in a 16 px corridor
/// leaves no rat-sized gap, so the turnkey waits for open floor instead.
#[test]
fn the_turnkey_refuses_a_bar_that_would_fill_an_exit() {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::TileKind;

    let mut w = World::new(Theme::Sewer, 29);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    // A one-cell-wide corridor running east-west: solid rock either side of the
    // rat's own row.
    let (cx, cy) = crate::game::collision::cell_of(p);
    for dy in [-1, 1] {
        for dx in -4..=4 {
            w.level.tiles.set(
                cx + dx,
                cy + dy,
                TileKind::Wall(WallKind::Solid),
            );
        }
    }
    let key = w.spawn_enemy(EnemyKind::Turnkey, p, 1.0);
    ready(&mut w, key, |e| {
        // Facing along the corridor: the bar lies across it.
        e.facing = Vec2::new(1.0, 0.0);
        e.w3.bar_cd = 0.0;
    });
    assert!(!crossbar_placement_ok(&w.level, p, Vec2::new(0.0, 1.0)));

    tick(&mut w);
    let s = state(&w, key);
    assert_eq!(s.w3.bar_t, 0.0, "the turnkey filled the exit");
    assert!(s.w3.bar_cd > 0.0, "a refused bar should retry later");

    // The same turnkey on open floor lays it happily.
    open_floor(&mut w);
    assert!(crossbar_placement_ok(
        &w.level,
        p,
        Vec2::new(0.0, 1.0)
    ));
    put(&mut w, key, |e| e.w3.bar_cd = 0.0);
    tick(&mut w);
    assert!(
        state(&w, key).w3.bar_telling(),
        "open floor should allow the bar"
    );
}

// ---- The two-at-once limiter ---------------------------------------------

/// Three pillbugs standing shoulder to shoulder may only start two shells at
/// once; the third waits its turn without losing its chance.
#[test]
fn at_most_two_nearby_warned_attacks_start_together() {
    let mut w = World::new(Theme::Sewer, 31);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    let ring = p + Vec2::new(18.0, 0.0);
    let a = w.spawn_enemy(EnemyKind::Pillbug, ring + Vec2::new(0.0, -6.0), 1.0);
    let b = w.spawn_enemy(EnemyKind::Pillbug, ring, 1.0);
    let c = w.spawn_enemy(EnemyKind::Pillbug, ring + Vec2::new(0.0, 6.0), 1.0);
    for e in [a, b, c] {
        ready(&mut w, e, |en| en.w3.curl_cd = 0.0);
    }

    tick(&mut w);
    let curled = [a, b, c]
        .iter()
        .filter(|e| state(&w, **e).w3.curl_t > 0.0)
        .count();
    assert_eq!(curled, WARN_MAX_NEARBY, "the limiter let {curled} start");

    // The body waiting its turn is not protected: it still takes damage, and
    // it keeps its spent cooldown so it retries the instant a slot frees.
    let waiting = [a, b, c]
        .into_iter()
        .find(|e| state(&w, *e).w3.curl_t <= 0.0)
        .expect("one bug must be waiting");
    let hp = state(&w, waiting).hp;
    put(&mut w, waiting, |e| {
        e.hurt(4.0);
    });
    assert!(state(&w, waiting).hp < hp, "a waiting body must still bleed");
    assert_eq!(state(&w, waiting).w3.curl_cd, 0.0, "the retry was lost");

    // When the shells drop, the waiting bug gets its turn.
    ticks(&mut w, secs(PILLBUG_CURL_SECS) + 2);
    let curled = [a, b, c]
        .iter()
        .filter(|e| state(&w, **e).w3.curl_t > 0.0)
        .count();
    assert!(curled > 0, "nobody curled after the first shells dropped");
    assert!(curled <= WARN_MAX_NEARBY);
}

// ---- Determinism and the rng contract ------------------------------------

/// A run with no wave-3 body on the map draws no rng at all, so the golden
/// traces' openings cannot move because of this system.
#[test]
fn a_tick_without_wave3_bodies_draws_no_rng() {
    use rand::RngExt;
    let mut w = World::new(Theme::Sewer, 37);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 0.0), 1.0);
    w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(-30.0, 0.0), 1.0);
    // The rng cannot be cloned, so compare the *stream*: a world built the same
    // way (same seed, same two spawns, no ticks) is the untouched reference.
    let mut reference = World::new(Theme::Sewer, 37);
    open_floor(&mut reference);
    reference.clear_enemies();
    reference.player.pos = p;
    reference.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 0.0), 1.0);
    reference.spawn_enemy(EnemyKind::Cat, p + Vec2::new(-30.0, 0.0), 1.0);
    ticks(&mut w, 30);
    assert_eq!(
        w.rng.random::<u64>(),
        reference.rng.random::<u64>(),
        "the wave-3 system consumed rng on a map with no wave-3 enemy"
    );
}

/// Two worlds, the same seed, the same wave-3 bodies and the same scripted rat:
/// the full schedule must agree tick for tick. This is the twin-run witness for
/// the new state machines (the golden harness covers the pre-port reference).
#[test]
fn twin_runs_with_wave3_kinds_stay_identical() {
    fn run_once() -> u64 {
        use rand::RngExt;
        let mut w = World::new(Theme::Sewer, 41);
        open_floor(&mut w);
        w.clear_enemies();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        w.viewport = Vec2::new(160.0, 168.0);
        let kinds = [
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
            EnemyKind::WickMoth,
            EnemyKind::Porter,
            EnemyKind::Turnkey,
            EnemyKind::Skitterer,
        ];
        for (i, kind) in kinds.iter().enumerate() {
            let a = i as f32 * 0.9;
            let pos = p + Vec2::from_angle(a) * (18.0 + i as f32 * 3.0);
            let e = w.spawn_enemy(*kind, pos, 1.0);
            put(&mut w, e, |en| {
                en.w3.armed = 0;
                if *kind == EnemyKind::WickMoth && i == 2 {
                    en.hp = 1.0;
                }
            });
        }
        let mut hash = 14_695_981_039_346_656_037u64;
        for step in 0..420 {
            let dir = Vec2::from_angle(step as f32 * 0.03);
            w.update(DT, dir, false);
            for e in w.enemies() {
                for v in [
                    e.pos.x.to_bits() as u64,
                    e.pos.y.to_bits() as u64,
                    e.hp.to_bits() as u64,
                    e.w3.curl_t.to_bits() as u64,
                    e.w3.flea_phase as u64,
                    e.w3.flea_t.to_bits() as u64,
                    e.w3.raise_t.to_bits() as u64,
                    e.w3.bar_t.to_bits() as u64,
                    e.w3.wick_t.to_bits() as u64,
                ] {
                    hash ^= v;
                    hash = hash.wrapping_mul(1_099_511_628_211);
                }
            }
            hash ^= w.player.hp.to_bits() as u64;
            w.rng.random::<u64>();
        }
        hash
    }
    assert_eq!(
        run_once(),
        run_once(),
        "two runs of the same seed diverged with wave-3 bodies on the field"
    );
}

// ---- The design contract, in one place -----------------------------------

/// The numbers in `WAVE3_DESIGN.md` §9, plus the gem XP each kind pays.
#[test]
fn the_wave3_stats_and_timings_match_the_design() {
    // Base HP / speed / contact damage / gem XP, per §9.
    for (kind, hp, speed, dmg, xp) in [
        (EnemyKind::Pillbug, 18.0, 23.0, 5.0, 3),
        (EnemyKind::BellFlea, 9.0, 32.0, 4.0, 2),
        (EnemyKind::WickMoth, 12.0, 28.0, 4.0, 3),
        (EnemyKind::Porter, 26.0, 20.0, 6.0, 5),
        (EnemyKind::Turnkey, 22.0, 24.0, 6.0, 4),
    ] {
        assert_eq!(kind.base_hp(), hp, "{kind:?} hp");
        assert_eq!(kind.speed(), speed, "{kind:?} speed");
        assert_eq!(kind.damage(), dmg, "{kind:?} contact damage");
        assert_eq!(kind.xp(), xp, "{kind:?} gem xp");
        assert!(!kind.ranged(), "{kind:?} is not a shooter");
    }

    // Behaviour timings, straight out of §9.
    assert_eq!(PILLBUG_CURL_SECS, 1.4);
    assert_eq!(FLEA_JUMP_PERIOD, 4.0);
    assert!((FLEA_TELL_SECS + FLEA_GAP_SECS - 0.65).abs() < 1e-6);
    assert_eq!(FLEA_CROSS_DAMAGE, 7.0);
    assert_eq!(FLEA_CROSS_RADIUS, 6.0);
    assert_eq!(FLEA_HOP_REACH, 26.0);
    assert_eq!(WICK_FUSE_SECS, 1.0);
    assert_eq!(WICK_BURST_RADIUS, 9.0);
    assert_eq!(WICK_RAT_DAMAGE, 9.0);
    assert_eq!(WICK_ENEMY_DAMAGE, 20.0);
    assert_eq!(PORTER_RAISE_SECS, 0.9);
    assert_eq!(PORTER_HEAL, 8.0);
    assert_eq!(PORTER_PERIOD, 5.0);
    assert_eq!(PORTER_RANGE, 24.0);
    assert_eq!(TURNKEY_PERIOD, 6.0);
    assert_eq!(CROSSBAR_LEN, 22.0);
    assert_eq!(CROSSBAR_TELL_SECS, 0.8);
    assert_eq!(CROSSBAR_LIVE_SECS, 2.0);
    assert_eq!(CROSSBAR_DAMAGE, 8.0);
    assert_eq!(PILLBUG_TRIGGER_RANGE, 24.0);

    // Size classes: the two small kinds follow the rat through a rat-hole, the
    // three big ones have to take the long way round.
    use crate::game::collision::SizeClass;
    assert_eq!(SizeClass::of_enemy(EnemyKind::BellFlea), SizeClass::Small);
    assert_eq!(SizeClass::of_enemy(EnemyKind::WickMoth), SizeClass::Small);
    assert_eq!(SizeClass::of_enemy(EnemyKind::Pillbug), SizeClass::Large);
    assert_eq!(SizeClass::of_enemy(EnemyKind::Porter), SizeClass::Large);
    assert_eq!(SizeClass::of_enemy(EnemyKind::Turnkey), SizeClass::Large);
}

/// The round-trip contract: a wave-3 body's state travels through
/// `begin_tick`/`end_tick` with the body (it is a field of `Enemy`, which is a
/// component), so a test can force a state between ticks and the next tick sees
/// it.
#[test]
fn the_wave3_state_round_trips_between_ticks() {
    let mut w = World::new(Theme::Sewer, 43);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    let bug = w.spawn_enemy(EnemyKind::Pillbug, p + Vec2::new(60.0, 0.0), 1.0);
    put(&mut w, bug, |e| {
        e.w3.curl_t = PILLBUG_CURL_SECS;
        e.w3.armed = 1;
    });
    tick(&mut w);
    assert!(state(&w, bug).w3.curl_t > 0.0, "the state did not survive");
    assert!(state(&w, bug).w3.curl_t < PILLBUG_CURL_SECS);
}

// ---- Screenshot tools ----------------------------------------------------

fn write_ppm(path: &str, pb: &crate::render::framebuffer::PixelBuffer) {
    let mut out = format!("P6\n{} {}\n255\n", pb.w, pb.h).into_bytes();
    for y in 0..pb.h {
        for x in 0..pb.w {
            let c = pb.pixel_at(x, y);
            out.push(c.0);
            out.push(c.1);
            out.push(c.2);
        }
    }
    std::fs::write(path, out).expect("write wave3 ppm");
}

/// Bestiary sheet: the five wave-3 silhouettes on the floor.
/// `RATRUN_DUMP=wave3 cargo test dump_wave3_kinds`.
#[test]
fn dump_wave3_kinds() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("wave3") {
        return;
    }
    use crate::render::framebuffer::PixelBuffer;
    let mut w = World::new(Theme::Sewer, 1);
    let c = w.level.arena * 0.5;
    w.player.pos = c;
    w.clear_enemies();
    let kinds = [
        EnemyKind::Pillbug,
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Porter,
        EnemyKind::Turnkey,
    ];
    for (i, kind) in kinds.iter().enumerate() {
        let x = c.x - 64.0 + i as f32 * 32.0;
        let e = w.spawn_enemy(*kind, Vec2::new(x, c.y - 12.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(1.0, 0.0);
            en.anim = i as f32 * 0.7;
        });
        // A second row facing the rat, mid-animation.
        let e = w.spawn_enemy(*kind, Vec2::new(x, c.y + 30.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(-0.4, 1.0).normalized();
            en.anim = 1.7 + i as f32;
        });
    }
    let mut pb = PixelBuffer::new(200, 96);
    w.draw(&mut pb);
    write_ppm("/tmp/ratrun_wave3_kinds.ppm", &pb);
}

/// Tell sheet: every warned attack, in the state the rat has to read.
/// `RATRUN_DUMP=wave3_tells cargo test dump_wave3_tells`.
#[test]
fn dump_wave3_tells() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("wave3_tells") {
        return;
    }
    use crate::render::framebuffer::PixelBuffer;
    let mut w = World::new(Theme::Sewer, 2);
    let c = w.level.arena * 0.5;
    w.player.pos = c;
    w.clear_enemies();

    // 1) Pillbug, curled (shell forward).
    let bug = w.spawn_enemy(EnemyKind::Pillbug, Vec2::new(c.x - 78.0, c.y - 18.0), 1.0);
    put(&mut w, bug, |e| {
        e.facing = Vec2::new(0.3, 1.0).normalized();
        e.w3.curl_t = PILLBUG_CURL_SECS * 0.5;
        e.w3.armed = 1;
    });

    // 2) Bell Flea mid-tell: both landing crosses dotted, the first under the
    //    rat's nose.
    let flea = w.spawn_enemy(EnemyKind::BellFlea, Vec2::new(c.x - 26.0, c.y - 20.0), 1.0);
    put(&mut w, flea, |e| {
        e.facing = Vec2::new(1.0, 0.2).normalized();
        e.w3.armed = 1;
        e.w3.flea_phase = 1;
        e.w3.flea_t = FLEA_TELL_SECS * 0.6;
        e.w3.home = e.pos;
        e.w3.hop_to = e.pos + Vec2::new(26.0, 4.0);
    });

    // 3) Porter raising the cube.
    let porter = w.spawn_enemy(EnemyKind::Porter, Vec2::new(c.x + 18.0, c.y - 18.0), 1.0);
    put(&mut w, porter, |e| {
        e.facing = Vec2::new(-1.0, 0.0);
        e.w3.armed = 1;
        e.w3.raise_t = PORTER_RAISE_SECS * 0.5;
    });

    // 4) Turnkey with a dotted bar, 5) the same drone with the bar live.
    let key = w.spawn_enemy(EnemyKind::Turnkey, Vec2::new(c.x + 62.0, c.y - 20.0), 1.0);
    put(&mut w, key, |e| {
        e.facing = Vec2::new(0.0, 1.0);
        e.w3.armed = 1;
        e.w3.bar_pos = e.pos;
        e.w3.bar_dir = Vec2::new(0.0, 1.0);
        e.w3.bar_t = CROSSBAR_LIVE_SECS + CROSSBAR_TELL_SECS * 0.5;
    });

    // 6) A burning wick (the frame before its burst).
    crate::game::ecs::resources::spawn_with_id(
        &mut w.ecs,
        Wick {
            pos: Vec2::new(c.x - 52.0, c.y + 30.0),
            fuse: WICK_FUSE_SECS * 0.25,
        },
    );
    // 7) A flea that has landed on its first cross (solid) with the spring back
    //    still dotted.
    let landed = w.spawn_enemy(EnemyKind::BellFlea, Vec2::new(c.x + 2.0, c.y + 30.0), 1.0);
    put(&mut w, landed, |e| {
        e.w3.armed = 1;
        e.w3.flea_phase = 2;
        e.w3.flea_t = FLEA_GAP_SECS * 0.5;
        e.w3.hop_to = e.pos;
        e.w3.home = e.pos + Vec2::new(-26.0, 0.0);
    });
    // 8) A turnkey with its bar live.
    let live = w.spawn_enemy(EnemyKind::Turnkey, Vec2::new(c.x + 58.0, c.y + 30.0), 1.0);
    put(&mut w, live, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w3.armed = 1;
        e.w3.bar_pos = e.pos;
        e.w3.bar_dir = Vec2::new(0.0, 1.0);
        e.w3.bar_t = CROSSBAR_LIVE_SECS * 0.6;
    });

    let mut pb = PixelBuffer::new(200, 96);
    w.draw(&mut pb);
    write_ppm("/tmp/ratrun_wave3_tells.ppm", &pb);
}

// ---- Integration: the shell vs every damage source added since ------------

/// One curled Pillbug per facing, hit by the same source: the loss with the
/// shell toward the hit must be `FRONT / REAR` of the loss with the shell away
/// from it (or equal, for a source with no direction).
///
/// `arrange` builds the source (the bug exists already), `step` advances it one
/// tick; the measurement stops on the first tick the bug loses HP, so a
/// multi-phase source is judged on the phase the case set up.
fn shell_case(
    label: &str,
    bug_offset: Vec2,
    toward_hit: Vec2,
    directional: bool,
    arrange: impl Fn(&mut World, Vec2, Entity),
    step: impl Fn(&mut World),
) {
    let mut lost = [0.0f32; 2];
    for (i, sign) in [1.0f32, -1.0].into_iter().enumerate() {
        let mut w = World::new(Theme::Sewer, 17);
        open_floor(&mut w);
        w.clear_enemies();
        w.clear_bullets();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        w.player.face = Vec2::new(1.0, 0.0);
        let bug = w.spawn_enemy(EnemyKind::Pillbug, p + bug_offset, 1.0);
        ready(&mut w, bug, |e| {
            e.facing = toward_hit * sign;
            e.w3.curl_t = 1.0e6;
            e.hp = 10_000.0;
            e.max_hp = 10_000.0;
        });
        arrange(&mut w, p, bug);
        rebuild_broadphase(&mut w);
        for _ in 0..240 {
            if state(&w, bug).hp < 10_000.0 {
                break;
            }
            step(&mut w);
        }
        lost[i] = 10_000.0 - state(&w, bug).hp;
    }
    assert!(lost[0] > 0.0 && lost[1] > 0.0, "{label}: never hit ({lost:?})");
    let expect = if directional {
        PILLBUG_REAR_MULT / PILLBUG_FRONT_MULT
    } else {
        1.0
    };
    let ratio = lost[1] / lost[0];
    assert!(
        (ratio / expect - 1.0).abs() < 0.02,
        "{label}: shell-away / shell-toward = {ratio} ({lost:?}), expected {expect}"
    );
}

/// A one-weapon loadout, ready to fire.
fn arm_weapon(w: &mut World, kind: crate::game::weapon::WeaponKind, evolved: bool) {
    use crate::game::loadout::Loadout;
    use crate::game::weapon::Weapon;
    let mut weapon = Weapon::new(kind);
    if evolved {
        weapon.level = kind.max_level();
        weapon.evolved = true;
    }
    weapon.timer = 0.0;
    w.player.loadout = Loadout::from_parts(vec![weapon], Vec::new());
}

/// Directional armour reaches every damage source the other wave-3 branches
/// added: Acid Rain, Laser Eyes, Super Dash and the evolutions' own hit paths
/// (Milk Teeth, Fondue string, Fairy Ring, Drain Cleaner front and runoff,
/// Roll Call, Ratchet swing) and the last four evolutions (a Downspout tread
/// and its runoff from the gutter's axis, a Carbon Copy beam from its recorded
/// origin, a Ripcord snap along the cord toward the departure point). Echo
/// Chamber spokes and Flywheel pellets are `Bullet`s, so
/// `a_bullet_obeys_the_shell` is their witness. A Hantavirus tick has no
/// direction and takes the unmodified multiplier, and so does the tick of a
/// chain a Dead Letter packet resumed.
#[test]
fn the_shell_reaches_every_later_damage_source() {
    use crate::game::ecs::components::{
        AcidDrop, CheeseString, CorrosiveChannel, DeadLetterPacket, FairyRing, Infection,
    };
    use crate::game::ecs::resources::{spawn_with_id, ActiveFx, DashTrack, LaserCopy, LaserFx};
    use crate::game::ecs::systems::{evolutions as evo, weapons, weapons_wave3a, weapons_wave3b};
    use crate::game::weapon::WeaponKind;

    let west = Vec2::new(-1.0, 0.0);
    let ahead = Vec2::new(20.0, 0.0);

    shell_case(
        "acid rain splash",
        ahead,
        west,
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                AcidDrop {
                    pos: p + Vec2::new(15.0, 0.0),
                    delay: 0.0,
                    radius: 12.0,
                    dmg: 10.0,
                    index: 0,
                    dir: Vec2::ZERO,
                    evolved: false,
                    volley: 1,
                },
            );
        },
        |w| run(w, weapons_wave3a::acid_rain_system),
    );
    shell_case(
        "hantavirus tick",
        ahead,
        west,
        false,
        |w, _, bug| {
            let id = w.ecs.get::<SpawnId>(bug).unwrap().0;
            w.ecs.entity_mut(bug).insert(Infection::new(id, 0, 40.0, 5.0));
        },
        |w| run(w, weapons_wave3a::hantavirus_system),
    );
    shell_case(
        "laser eyes pulse",
        ahead,
        west,
        true,
        |w, _, _| arm_weapon(w, WeaponKind::LaserEyes, false),
        |w| run(w, weapons_wave3b::laser_eyes_system),
    );
    shell_case(
        "super dash",
        Vec2::new(10.0, 0.0),
        west,
        true,
        |w, _, _| {
            arm_weapon(w, WeaponKind::SuperDash, false);
            w.player.vel = Vec2::new(58.0, 0.0);
            w.player.dash_cd = 0.0;
        },
        |w| {
            let first = w.player.dash_active <= 0.0;
            w.begin_tick(DT, Vec2::new(1.0, 0.0), first);
            w.ecs
                .run_system_cached(crate::game::ecs::systems::player::player_system)
                .unwrap();
            w.ecs.run_system_cached(weapons_wave3b::super_dash_system).unwrap();
            w.end_tick();
        },
    );
    shell_case(
        "milk teeth dart",
        Vec2::new(24.0, 0.0),
        west,
        true,
        |w, _, _| {
            arm_weapon(w, WeaponKind::Gnaw, true);
            run(w, weapons::fire_weapons_system);
        },
        |w| run(w, evo::milk_teeth_system),
    );
    shell_case(
        "fondue string",
        Vec2::new(30.0, 0.0),
        west,
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                CheeseString::new(
                    p + Vec2::new(27.0, -14.0),
                    p + Vec2::new(27.0, 14.0),
                    evo::STRING_SECS,
                    10.0,
                ),
            );
        },
        |w| run(w, evo::fondue_string_system),
    );
    shell_case(
        "fairy ring",
        Vec2::new(evo::FAIRY_RADIUS + 2.0, 0.0),
        west,
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                FairyRing::new(p, evo::FAIRY_RADIUS, evo::FAIRY_SECS, 5, 10.0),
            );
        },
        |w| run(w, evo::fairy_ring_system),
    );
    shell_case(
        "drain cleaner front",
        ahead,
        west,
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                CorrosiveChannel::new(p, Vec2::new(1.0, 0.0), evo::CHANNEL_LEN, 30.0, 10.0),
            );
        },
        |w| run(w, evo::drain_cleaner_system),
    );
    shell_case(
        "drain cleaner runoff",
        Vec2::new(20.0, 3.0),
        Vec2::new(0.0, -1.0),
        true,
        |w, p, _| {
            let mut ch =
                CorrosiveChannel::new(p, Vec2::new(1.0, 0.0), evo::CHANNEL_LEN, 30.0, 10.0);
            // Already settled: only the runoff ticks.
            ch.age = evo::CHANNEL_ADVANCE;
            spawn_with_id(&mut w.ecs, ch);
        },
        |w| run(w, evo::drain_cleaner_system),
    );
    shell_case(
        "roll call answer",
        Vec2::new(66.0, 0.0),
        west,
        true,
        |w, p, _| {
            let src = w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(60.0, 0.0), 400.0);
            let id = w.ecs.get::<SpawnId>(src).unwrap().0;
            let mut fx = w.ecs.resource_mut::<ActiveFx>();
            fx.clear_roll_call();
            assert!(fx.tag(src, id));
            fx.answer_left = evo::ROLL_CALL_DELAY;
            fx.answer_damage = 4.0;
        },
        |w| run(w, evo::roll_call_system),
    );
    // ---- The last four evolutions --------------------------------------
    // Downspout: a tread arrives from the gutter's axis (the march line
    // through the drop), not from the splash centre. The bug sits 3 px south
    // of an eastward gutter, so the hit arrives from the north.
    shell_case(
        "downspout tread",
        Vec2::new(20.0, 3.0),
        Vec2::new(0.0, -1.0),
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                AcidDrop {
                    pos: p + Vec2::new(15.0, 0.0),
                    delay: 0.0,
                    radius: 12.0,
                    dmg: 10.0,
                    index: 0,
                    dir: Vec2::new(1.0, 0.0),
                    evolved: true,
                    // Not the last of its volley: no runoff yet, tread only.
                    volley: 3,
                },
            );
        },
        |w| run(w, weapons_wave3a::acid_rain_system),
    );
    // Downspout: the burning runoff, from the nearest point of the gutter.
    shell_case(
        "downspout runoff",
        Vec2::new(20.0, 3.0),
        Vec2::new(0.0, -1.0),
        true,
        |w, p, _| {
            spawn_with_id(
                &mut w.ecs,
                CorrosiveChannel::gutter(p, Vec2::new(1.0, 0.0), evo::CHANNEL_LEN, 30.0, 1.0),
            );
        },
        |w| run(w, evo::drain_cleaner_system),
    );
    // Carbon Copy: the afterimage fires from its *recorded* origin, not from
    // the rat. The bug is north of the rat (outside the live fan, which faces
    // east) and the copy looks south at it from further north.
    shell_case(
        "carbon copy beam",
        Vec2::new(0.0, 30.0),
        Vec2::new(0.0, 1.0),
        true,
        |w, p, _| {
            arm_weapon(w, WeaponKind::LaserEyes, true);
            w.ecs.resource_mut::<LaserFx>().copy = Some(LaserCopy {
                origin: p + Vec2::new(0.0, 60.0),
                facing: Vec2::new(0.0, -1.0),
                pulses: 3,
                reach: 40.0,
                dmg: 10.0,
            });
        },
        |w| run(w, weapons_wave3b::laser_eyes_system),
    );
    // Ripcord: the snap travels along the cord toward the departure point
    // (west of the bug), so it strikes the bug from the east.
    shell_case(
        "ripcord snap",
        Vec2::new(20.0, 0.0),
        Vec2::new(1.0, 0.0),
        true,
        |w, p, _| {
            arm_weapon(w, WeaponKind::SuperDash, true);
            let mut track = w.ecs.resource_mut::<DashTrack>();
            track.segment = None;
            track.extend_route(p + Vec2::new(-12.0, 0.0), p + Vec2::new(44.0, 0.0));
        },
        |w| run(w, evo::ripcord_system),
    );
    // Dead Letter: the packet's chain resumes on the bug and ticks it like any
    // Hantavirus mark: no direction.
    shell_case(
        "dead letter tick",
        ahead,
        west,
        false,
        |w, p, _| {
            // A chain sealed by some other (dead) host, one jump left.
            let chain = Infection::new(u64::MAX - 7, 1, 40.0, 5.0);
            spawn_with_id(
                &mut w.ecs,
                DeadLetterPacket {
                    pos: p + ahead + Vec2::new(2.0, 0.0),
                    life: evo::DEAD_LETTER_SECS,
                    total: evo::DEAD_LETTER_SECS,
                    chain,
                },
            );
        },
        |w| {
            run(w, evo::dead_letter_system);
            run(w, weapons_wave3a::hantavirus_system);
        },
    );
    shell_case(
        "ratchet swing",
        Vec2::new(0.0, 20.0),
        Vec2::new(0.0, -1.0),
        true,
        |w, p, _| {
            arm_weapon(w, WeaponKind::TailWhip, true);
            w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(16.0, 0.0), 400.0);
        },
        |w| run(w, weapons::fire_weapons_system),
    );
}

/// A Wick Moth killed by an evolved weapon still lights its fuse: the full
/// schedule runs the wave-3 behaviour tick after the evolutions and before the
/// reap. (The Drain Cleaner's runoff is the one evolution path that does not
/// go through `hurt`; it books the death through `note_damage`.)
#[test]
fn a_moth_killed_by_an_evolution_leaves_its_wick() {
    use crate::game::ecs::components::{CorrosiveChannel, FairyRing};
    use crate::game::ecs::resources::spawn_with_id;
    use crate::game::ecs::systems::evolutions as evo;

    for runoff in [false, true] {
        let mut w = World::new(Theme::Sewer, 23);
        open_floor(&mut w);
        w.clear_enemies();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        let at = p + Vec2::new(120.0, 0.0);
        let moth = w.spawn_enemy(EnemyKind::WickMoth, at, 1.0);
        put(&mut w, moth, |e| e.hp = 0.01);
        if runoff {
            let mut ch = CorrosiveChannel::new(
                at - Vec2::new(10.0, 0.0),
                Vec2::new(1.0, 0.0),
                evo::CHANNEL_LEN,
                30.0,
                10.0,
            );
            ch.age = evo::CHANNEL_ADVANCE;
            spawn_with_id(&mut w.ecs, ch);
        } else {
            spawn_with_id(
                &mut w.ecs,
                FairyRing::new(
                    at - Vec2::new(evo::FAIRY_RADIUS, 0.0),
                    evo::FAIRY_RADIUS,
                    evo::FAIRY_SECS,
                    5,
                    10.0,
                ),
            );
        }
        w.update(DT, Vec2::ZERO, false);
        assert!(w.ecs.get::<Enemy>(moth).is_none(), "runoff={runoff}: the moth survived");
        assert_eq!(wick_count(&mut w), 1, "runoff={runoff}: no wick was lit");
    }
}

/// The Wind-up Mouse lures the five new kinds like any ordinary body, and
/// leaves their elites on the rat.
#[test]
fn the_decoy_lures_ordinary_wave3_bodies_only() {
    use crate::game::ecs::systems::powerups::decoy_lures;
    for kind in [
        EnemyKind::Pillbug,
        EnemyKind::BellFlea,
        EnemyKind::WickMoth,
        EnemyKind::Porter,
        EnemyKind::Turnkey,
    ] {
        assert!(decoy_lures(false, kind), "{kind:?} ignores the decoy");
        assert!(!decoy_lures(true, kind), "an elite {kind:?} chases the decoy");
    }
}
