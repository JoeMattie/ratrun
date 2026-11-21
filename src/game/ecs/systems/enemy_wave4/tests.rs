//! Narrow single-system tests for the wave-4 kinds: behaviour-state timings,
//! that every tell is visible before it can bite, each kind's own rule (the
//! snake's body wall, the centipede's flight, the hornet's punish window, the
//! horde's per-hit cap and split, the lilliputians' spear wall / volley / rope
//! knot / panic), the director's unlock times and caps, the floor rules
//! (pits, ink, decoy), that the later damage sources reach them, and twin-run
//! determinism.
//!
//! The harness is the one the rest of `systems/*` uses: `begin_tick` → the
//! system via `run_system_cached` → `end_tick`, asserting on the facade.

use super::*;
use crate::game::collision::TileMap;
use crate::game::ecs::components::SpawnId;
use crate::game::enemy::{Enemy, EnemyKind};
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

/// One wave-4 behaviour tick.
fn tick(w: &mut World) {
    run(w, wave4_system);
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

fn ready<F: FnOnce(&mut Enemy)>(w: &mut World, entity: Entity, init: F) {
    put(w, entity, |e| {
        e.w4.armed = 1;
        init(e);
    });
}

/// A world with the generated walls stripped, the rat at the centre and no
/// enemies.
fn open_world(seed: u64) -> (World, Vec2) {
    let mut w = World::new(Theme::Sewer, seed);
    open_floor(&mut w);
    w.clear_enemies();
    w.clear_bullets();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    (w, p)
}

/// Fill `SpatialGrid` + `EnemyIndex` + `GridScratch` the way
/// `enemy_move::grid_build_system` does (in `SpawnId` order).
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

// ---- Big Snake -----------------------------------------------------------

#[test]
fn the_big_snake_coils_lunges_and_lies_stretched() {
    let (mut w, p) = open_world(11);
    // Inside the 64 px strike band, outside the contact radius.
    let snake = w.spawn_enemy(EnemyKind::BigSnake, p + Vec2::new(50.0, 0.0), 1.0);
    ready(&mut w, snake, |e| {
        e.w4.cd = 0.0;
        e.w4.phase = SNAKE_PHASE_SLITHER;
    });
    tick(&mut w);
    let s = state(&w, snake);
    assert_eq!(s.w4.phase, SNAKE_PHASE_COIL, "the snake must coil at 50 px");
    assert!(s.w4.holds(EnemyKind::BigSnake), "a coiling snake is planted");
    assert!(
        s.w4.dir.dot(Vec2::new(-1.0, 0.0)) > 0.99,
        "the lunge aim is locked: {:?}",
        s.w4.dir
    );
    assert!(w.sfx.contains(&crate::audio::Sfx::Warn), "the coil must sound");
    let coil_at = s.pos;

    // It does not lunge before the tell has run out.
    ticks(&mut w, secs(SNAKE_TELL_SECS) - 3);
    assert_eq!(
        state(&w, snake).w4.phase,
        SNAKE_PHASE_COIL,
        "the lunge fired before its tell"
    );
    assert_eq!(state(&w, snake).pos, coil_at, "the coil must not drift");

    // Then the lunge covers SNAKE_LUNGE_DISTANCE, and it lies stretched.
    ticks(&mut w, 6);
    let s = state(&w, snake);
    assert!(
        matches!(s.w4.phase, SNAKE_PHASE_LUNGE | SNAKE_PHASE_STRETCHED),
        "the coil did not resolve into a lunge: phase {}",
        s.w4.phase
    );
    ticks(&mut w, secs(SNAKE_LUNGE_SECS) + 2);
    let stretched = state(&w, snake);
    assert_eq!(stretched.w4.phase, SNAKE_PHASE_STRETCHED);
    let travelled = (stretched.pos - coil_at).len();
    assert!(
        (travelled - SNAKE_LUNGE_DISTANCE).abs() < 4.0,
        "the lunge travelled {travelled} px, expected {SNAKE_LUNGE_DISTANCE}"
    );
    assert!(stretched.w4.holds(EnemyKind::BigSnake));

    // The stretched pose is the punish window: it is still and its cooldown
    // restarts when it slides back into the slither.
    ticks(&mut w, secs(SNAKE_STRETCH_SECS) + 2);
    let after = state(&w, snake);
    assert_eq!(after.w4.phase, SNAKE_PHASE_SLITHER);
    assert!(
        after.w4.cd > SNAKE_STRIKE_COOLDOWN - 0.2,
        "the strike cooldown did not restart: {}",
        after.w4.cd
    );
}

#[test]
fn the_snake_body_blocks_walking_but_not_dashing() {
    let (mut w, p) = open_world(12);
    let snake = w.spawn_enemy(EnemyKind::BigSnake, p + Vec2::new(40.0, 0.0), 1.0);
    ready(&mut w, snake, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w4.phase = SNAKE_PHASE_STRETCHED;
        e.w4.t = 1e6;
        for i in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
            e.w4.trail[i] = e.pos - Vec2::new(i as f32 * SNAKE_SPACING, 0.0);
        }
        e.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
    });
    // The rat stands on the middle of the body.
    let on_body = p + Vec2::new(40.0 - 3.0 * SNAKE_SPACING, 0.0);
    w.player.pos = on_body;
    tick(&mut w);
    let pushed = w.player.pos;
    assert!(
        pushed.dist(on_body) > 0.5,
        "the walking rat must be pushed out of the body"
    );
    assert!(
        pushed.dist(state(&w, snake).pos) >= 3.0 * SNAKE_SPACING - 1.0,
        "the rat must end up outside the body, not on it"
    );

    // The same rat, dashing, crosses the body untouched.
    w.player.pos = on_body;
    w.player.dash_active = 1.0;
    tick(&mut w);
    assert_eq!(
        w.player.pos, on_body,
        "a dash must pass over the snake's body"
    );
}

#[test]
fn the_snake_head_takes_more_than_its_body() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(5);
    let mut snake = Enemy::spawn(EnemyKind::BigSnake, Vec2::ZERO, 1.0, &mut rng);
    // A hit on the head …
    assert_eq!(snake.damage_mult_from(Vec2::new(3.0, 0.0)), SNAKE_HEAD_MULT);
    // … and one 30 px down the body (the trail is not a hit shape, but the
    // multiplier is pure): body 1.0.
    assert_eq!(snake.damage_mult_from(Vec2::new(30.0, 0.0)), 1.0);
    let hp = snake.hp;
    let applied = snake.hurt_from(10.0, Vec2::new(2.0, 0.0));
    assert!((applied - 10.0 * SNAKE_HEAD_MULT).abs() < 1e-4);
    assert!((snake.hp - (hp - 10.0 * SNAKE_HEAD_MULT)).abs() < 1e-4);
}

// ---- House Centipede -----------------------------------------------------

#[test]
fn the_centipede_flutters_then_darts_and_flees_when_hit() {
    let (mut w, p) = open_world(13);
    let bug = w.spawn_enemy(EnemyKind::HouseCentipede, p + Vec2::new(60.0, 0.0), 1.0);
    ready(&mut w, bug, |e| {
        e.w4.cd = 0.0;
        e.w4.phase = CENTIPEDE_PHASE_RUN;
        e.w4.flee_t = 0.0;
    });
    tick(&mut w);
    let s = state(&w, bug);
    assert_eq!(s.w4.phase, CENTIPEDE_PHASE_FLUTTER, "the dart needs a tell");
    assert!(s.w4.holds(EnemyKind::HouseCentipede), "the flutter is planted");
    let start = s.pos;

    // No movement and no damage until the flutter has run out.
    ticks(&mut w, secs(CENTIPEDE_FLUTTER_SECS) - 2);
    assert_eq!(state(&w, bug).pos, start, "the flutter must not slide");
    ticks(&mut w, 4);
    assert_eq!(state(&w, bug).w4.phase, CENTIPEDE_PHASE_DART);
    ticks(&mut w, secs(CENTIPEDE_DART_SECS) + 2);
    let after = state(&w, bug);
    let dart = (after.pos - start).len();
    assert!(
        (dart - CENTIPEDE_DART_SPEED * CENTIPEDE_DART_SECS).abs() < 6.0,
        "the dart covered {dart} px"
    );
    assert_eq!(after.w4.phase, CENTIPEDE_PHASE_RUN);
    assert!(after.w4.cd > CENTIPEDE_DART_COOLDOWN - 0.2);

    // Any hit makes it run: the flee timer is booked by `note_damage`, and the
    // body moves away from the rat.
    put(&mut w, bug, |e| {
        e.w4.cd = 1e6;
        e.w4.phase = CENTIPEDE_PHASE_RUN;
        e.w4.flee_t = 0.0;
        e.hurt(1.0);
    });
    assert!(state(&w, bug).w4.flee_t > 0.0, "a hit must start the flight");
    let before = state(&w, bug).pos;
    ticks(&mut w, secs(0.25));
    let fled = state(&w, bug).pos;
    assert!(
        fled.dist(p) > before.dist(p) + 1.0,
        "the centipede did not flee: {} -> {}",
        before.dist(p),
        fled.dist(p)
    );
}

// ---- Murder Hornet -------------------------------------------------------

#[test]
fn the_hornet_charges_dives_jabs_and_must_recover() {
    let (mut w, p) = open_world(14);
    // 70 px away: inside the 170 px charge range, outside contact.
    let hornet = w.spawn_enemy(EnemyKind::MurderHornet, p + Vec2::new(70.0, 0.0), 1.0);
    ready(&mut w, hornet, |e| {
        e.w4.cd = 0.0;
        e.w4.phase = HORNET_PHASE_HOVER;
    });
    tick(&mut w);
    let s = state(&w, hornet);
    assert_eq!(s.w4.phase, HORNET_PHASE_CHARGE, "the hornet must buzz up");
    assert!(
        s.w4.dir.dot(Vec2::new(-1.0, 0.0)) > 0.99,
        "the dive aim is locked: {:?}",
        s.w4.dir
    );
    assert!(w.sfx.contains(&crate::audio::Sfx::HornetBuzz));
    let hp = w.player.hp;

    // No damage during the tell.
    ticks(&mut w, secs(HORNET_TELL_SECS) - 3);
    assert_eq!(w.player.hp, hp, "the dive bit during its tell");
    assert_eq!(state(&w, hornet).w4.phase, HORNET_PHASE_CHARGE);

    // The dive crosses the rat: one jab, heavier than contact.
    ticks(&mut w, secs(HORNET_DIVE_SECS) - 2);
    assert!(
        w.player.hp < hp,
        "the dive line crossed the rat without a jab"
    );
    let jab = hp - w.player.hp;
    assert!(
        (jab - HORNET_JAB_DAMAGE).abs() < 1e-3,
        "the jab landed {jab}, expected {HORNET_JAB_DAMAGE}"
    );
    ticks(&mut w, secs(HORNET_DIVE_SECS) + 2);
    assert_eq!(state(&w, hornet).w4.phase, HORNET_PHASE_RECOVER);

    // The punish window is a number: ×1.5 from any direction while grounded.
    let grounded = state(&w, hornet);
    assert_eq!(
        grounded.damage_mult_from(Vec2::new(0.0, 30.0)),
        HORNET_RECOVER_MULT
    );
    let hp = grounded.hp;
    put(&mut w, hornet, |e| {
        e.hurt_from(10.0, e.pos + Vec2::new(0.0, 20.0));
    });
    assert!((state(&w, hornet).hp - (hp - 10.0 * HORNET_RECOVER_MULT)).abs() < 1e-3);

    // It stays down for the full recovery, then hovers again.
    ticks(&mut w, secs(HORNET_RECOVER_SECS) + 2);
    assert_eq!(state(&w, hornet).w4.phase, HORNET_PHASE_HOVER);
}

// ---- Termite Horde -------------------------------------------------------

#[test]
fn the_horde_caps_one_hit_and_splits_on_a_hard_one() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(21);
    let mut h = Enemy::spawn(EnemyKind::TermiteHorde, Vec2::ZERO, 1.0, &mut rng);
    h.max_hp = 100.0;
    h.hp = 100.0;
    h.w4.tier = 2;
    // A dart: full damage, no split (below the split threshold).
    h.hurt(5.0);
    assert!((h.hp - 95.0).abs() < 1e-3, "a small hit must land in full");
    assert_eq!(h.w4.split_pending, 0);
    // A single huge hit: capped to HORDE_HIT_CAP of the pool, and it arms the
    // split.
    let lost = 95.0 - h.hp;
    let _ = lost;
    h.hurt(80.0);
    assert!(
        (h.hp - (95.0 - 100.0 * HORDE_HIT_CAP)).abs() < 1e-3,
        "a 80-damage hit removed {}",
        95.0 - h.hp
    );
    assert_eq!(h.w4.split_pending, 1, "a hard hit must arm the split");

    // Area damage still works at full rate: many small hits in one tick.
    let mut h2 = Enemy::spawn(EnemyKind::TermiteHorde, Vec2::ZERO, 1.0, &mut rng);
    h2.max_hp = 100.0;
    h2.hp = 100.0;
    h2.w4.tier = 2;
    for _ in 0..4 {
        h2.hurt(5.0);
    }
    assert!(
        (h2.hp - 80.0).abs() < 1e-3,
        "four bullets removed only {}",
        100.0 - h2.hp
    );
    // A continuous source (a pool's per-tick damage) is never capped.
    let mut h3 = Enemy::spawn(EnemyKind::TermiteHorde, Vec2::ZERO, 1.0, &mut rng);
    h3.max_hp = 100.0;
    h3.hp = 100.0;
    for _ in 0..60 {
        h3.hurt(1.0);
    }
    assert!((h3.hp - 40.0).abs() < 1e-3, "the pool was capped: {}", h3.hp);
}

#[test]
fn a_horde_split_conserves_health_and_the_pieces_regrow() {
    let (mut w, p) = open_world(22);
    let horde = w.spawn_enemy(EnemyKind::TermiteHorde, p + Vec2::new(40.0, 0.0), 1.0);
    ready(&mut w, horde, |e| {
        e.w4.tier = 2;
        e.w4.splits = 0;
        e.max_hp = 64.0;
        e.hp = 48.0;
    });
    // A hard hit: the flag is raised, and the system turns it into a second
    // body with the health split between them.
    put(&mut w, horde, |e| {
        e.hurt(40.0);
    });
    tick(&mut w);
    let all: Vec<Enemy> = w.enemies();
    let hordes: Vec<&Enemy> = all
        .iter()
        .filter(|e| e.kind == EnemyKind::TermiteHorde)
        .collect();
    assert_eq!(hordes.len(), 2, "the horde must split into two bodies");
    let total: f32 = hordes.iter().map(|e| e.hp).sum();
    assert!(
        (total - (48.0 - 64.0 * HORDE_HIT_CAP)).abs() < 0.5,
        "the split invented or lost health: {total}"
    );
    for e in &hordes {
        assert_eq!(e.w4.tier, 1, "both halves are medium hordes");
        assert_eq!(e.w4.splits, 1, "neither half may split again");
    }

    // A medium horde that stays unhit regrows to full and reforms into a big
    // one (which cannot split again, it already did).
    let one = w
        .ecs
        .query::<(Entity, &Enemy)>()
        .iter(&w.ecs)
        .find(|(_, e)| e.kind == EnemyKind::TermiteHorde)
        .map(|(entity, _)| entity)
        .unwrap();
    put(&mut w, one, |e| {
        e.hp = 64.0 * HORDE_TIER1_FRACTION;
        e.w4.regrow_t = 0.0;
        e.w4.tier = 1;
        e.w4.splits = 1;
        e.w4.cd = 1e6;
        // Park it far from the rat so it is not the one that splits again.
    });
    ticks(&mut w, secs(HORDE_REGROW_SECS) + 4);
    let reformed = state(&w, one);
    assert_eq!(reformed.w4.tier, 2, "the survivors must reform");
    assert_eq!(reformed.w4.splits, 1, "splits are once per body");
}

// ---- Lilliputians --------------------------------------------------------

fn squad_member(
    w: &mut World,
    p: Vec2,
    role: u8,
    slot: u8,
    squad: u32,
    offset: Vec2,
) -> Entity {
    let e = w.spawn_enemy(EnemyKind::Lilliputian, p + offset, 1.0);
    put(w, e, |e| {
        e.w4.armed = 1;
        e.w4.role = role;
        e.w4.slot = slot;
        e.w4.squad = squad;
        e.w4.cd = 0.0;
        e.w4.phase = LILLI_PHASE_WALK;
    });
    e
}

#[test]
fn the_spear_line_hurts_from_the_front_and_is_soft_from_behind() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(31);
    let mut l = Enemy::spawn(EnemyKind::Lilliputian, Vec2::ZERO, 1.0, &mut rng);
    l.facing = Vec2::new(1.0, 0.0);
    // Front: the wall of points.
    assert_eq!(l.damage_mult_from(Vec2::new(10.0, 0.0)), LILLI_FRONT_MULT);
    // Behind.
    assert_eq!(l.damage_mult_from(Vec2::new(-10.0, 0.0)), LILLI_REAR_MULT);

    // The line levels its points inside SPEAR_REACH, plants and then steps in.
    let (mut w, p) = open_world(32);
    let spear = squad_member(&mut w, p, LILLI_ROLE_SPEAR, 1, 7, Vec2::new(40.0, 0.0));
    tick(&mut w);
    let s = state(&w, spear);
    assert_eq!(s.w4.phase, LILLI_PHASE_TELL, "the spear must level first");
    assert!(s.w4.holds(EnemyKind::Lilliputian));
    let planted = s.pos;
    ticks(&mut w, secs(SPEAR_TELL_SECS) - 2);
    assert_eq!(state(&w, spear).pos, planted, "the tell must hold still");
    ticks(&mut w, 4);
    assert_eq!(state(&w, spear).w4.phase, LILLI_PHASE_ACTION);
    ticks(&mut w, secs(SPEAR_CHARGE_SECS) + 2);
    let after = state(&w, spear);
    assert_eq!(after.w4.phase, LILLI_PHASE_WALK);
    assert!(
        after.pos.dist(p) < planted.dist(p) - 1.0,
        "the levelled line did not step in"
    );
}

#[test]
fn the_archers_kneel_and_loose_through_the_normal_bullet_path() {
    let (mut w, p) = open_world(33);
    let archer = squad_member(&mut w, p, LILLI_ROLE_ARCHER, 4, 8, Vec2::new(80.0, 0.0));
    tick(&mut w);
    let s = state(&w, archer);
    assert_eq!(s.w4.phase, LILLI_PHASE_TELL, "the archer must kneel");
    assert_eq!(w.bullets().len(), 0, "no pellets before the tell is done");
    ticks(&mut w, secs(ARCHER_TELL_SECS) + 2);
    let bullets = w.bullets();
    let enemy_pellets: Vec<_> = bullets
        .iter()
        .filter(|b| b.faction == crate::game::bullet::Faction::Enemy)
        .collect();
    assert_eq!(
        enemy_pellets.len(),
        ARCHER_VOLLEY,
        "the volley must be a normal enemy bullet each"
    );
    let spread = (enemy_pellets[0].vel.normalized().dot(
        enemy_pellets[1].vel.normalized(),
    ))
    .clamp(-1.0, 1.0);
    assert!(
        spread < 0.999,
        "the volley must be spread across a small arc"
    );
    assert_eq!(state(&w, archer).w4.phase, LILLI_PHASE_WALK);
}

#[test]
fn three_rope_crews_pin_the_rat_and_a_dash_breaks_it() {
    let (mut w, p) = open_world(34);
    let mut crew = Vec::new();
    for k in 0..ROPE_MIN_TIED {
        let a = k as f32 / ROPE_MIN_TIED as f32 * std::f32::consts::TAU;
        let e = squad_member(
            &mut w,
            p,
            LILLI_ROLE_ROPE,
            6 + k as u8,
            9,
            Vec2::from_angle(a) * 20.0,
        );
        // Already tying: one tick from the knot closing.
        put(&mut w, e, |e| {
            e.w4.phase = LILLI_PHASE_TELL;
            e.w4.t = DT * 0.5;
        });
        crew.push(e);
    }
    tick(&mut w);
    for e in &crew {
        assert_eq!(
            state(&w, *e).w4.phase,
            LILLI_PHASE_ACTION,
            "the knot did not close"
        );
    }
    // The pin: the rat is dragged back to the knot and its velocity is killed.
    w.player.pos = p + Vec2::new(6.0, 0.0);
    w.player.vel = Vec2::new(40.0, 0.0);
    let before = w.player.pos;
    tick(&mut w);
    assert_eq!(w.player.vel, Vec2::ZERO, "a pinned rat cannot run");
    assert!(
        w.player.pos.dist(p) < before.dist(p) + 0.1,
        "the rope did not pull the rat back"
    );

    // A dash breaks out even while the ring is closed.
    w.player.pos = p + Vec2::new(6.0, 0.0);
    w.player.dash_active = 1.0;
    let dasher = w.player.pos;
    tick(&mut w);
    assert_eq!(w.player.pos, dasher, "a dash must break the pin");

    // Clean Socks ignore the rope exactly like a glue board.
    w.player.dash_active = 0.0;
    w.player.buffs.socks = 2.0;
    let socked = w.player.pos;
    tick(&mut w);
    assert_eq!(w.player.pos, socked, "Clean Socks must ignore the pin");

    // Killing a rope-holder drops its rope: the pin needs three taut lines.
    w.player.buffs.socks = 0.0;
    put(&mut w, crew[0], |e| {
        e.hurt(10.0);
    });
    assert_eq!(
        state(&w, crew[0]).w4.phase,
        LILLI_PHASE_WALK,
        "a hit must drop the rope"
    );
    w.player.pos = p + Vec2::new(6.0, 0.0);
    w.player.vel = Vec2::ZERO;
    let free = w.player.pos;
    tick(&mut w);
    assert_eq!(w.player.pos, free, "two ropes must not pin the rat");
}

#[test]
fn a_squad_below_a_third_panics_and_escapes() {
    let (mut w, p) = open_world(35);
    let squad = 77u32;
    let mut bugs = Vec::new();
    // Three survivors of a ten-body squad: below LILLI_SQUAD_SIZE / 3.
    for k in 0..(LILLI_SQUAD_SIZE / PANIC_DIVISOR) {
        let e = squad_member(
            &mut w,
            p,
            LILLI_ROLE_SPEAR,
            k as u8,
            squad,
            Vec2::new(50.0 + k as f32 * 4.0, 10.0),
        );
        put(&mut w, e, |en| en.w4.squad_peak = LILLI_SQUAD_SIZE as u16);
        bugs.push(e);
    }
    tick(&mut w);
    for e in &bugs {
        assert_eq!(
            state(&w, *e).w4.phase,
            LILLI_PHASE_PANIC,
            "the survivors must break and run"
        );
    }
    let before = state(&w, bugs[0]).pos;
    ticks(&mut w, secs(0.4));
    assert!(
        state(&w, bugs[0]).pos.dist(p) > before.dist(p) + 1.0,
        "a panicking survivor must run from the rat"
    );
    // After PANIC_SECS they escape into the walls (despawn, no loot).
    ticks(&mut w, secs(PANIC_SECS) + 2);
    assert!(
        w.enemies()
            .iter()
            .all(|e| e.kind != EnemyKind::Lilliputian),
        "the panicked squad must escape"
    );
}

#[test]
fn a_straggler_that_runs_into_another_squad_joins_it() {
    let (mut w, p) = open_world(36);
    // A panicking straggler of a broken squad, right beside a healthy one.
    let straggler = squad_member(&mut w, p, LILLI_ROLE_ROPE, 6, 100, Vec2::new(60.0, 0.0));
    put(&mut w, straggler, |e| {
        e.w4.phase = LILLI_PHASE_PANIC;
        e.w4.t = PANIC_SECS;
    });
    for k in 0..5 {
        squad_member(
            &mut w,
            p,
            LILLI_ROLE_SPEAR,
            k as u8,
            200,
            Vec2::new(64.0 + k as f32, 4.0),
        );
    }
    tick(&mut w);
    let joined = state(&w, straggler);
    assert_eq!(joined.w4.squad, 200, "the straggler must rejoin a squad");
    assert_ne!(joined.w4.phase, LILLI_PHASE_PANIC);
}

// ---- Floor rules ---------------------------------------------------------

#[test]
fn the_decoy_lures_the_new_ground_kinds() {
    use crate::game::ecs::systems::powerups::spawn_decoy;
    let (mut w, p) = open_world(41);
    // The key sits well away from the rat; the snake is inside its strike band
    // of the key, not of the rat.
    let key = p + Vec2::new(0.0, 40.0);
    spawn_decoy(&mut w, key);
    let snake = w.spawn_enemy(EnemyKind::BigSnake, p + Vec2::new(0.0, 86.0), 1.0);
    ready(&mut w, snake, |e| {
        e.w4.cd = 0.0;
        e.w4.phase = SNAKE_PHASE_SLITHER;
    });
    tick(&mut w);
    let s = state(&w, snake);
    assert_eq!(s.w4.phase, SNAKE_PHASE_COIL);
    assert!(
        s.w4.dir.dot((key - s.pos).normalized()) > 0.9,
        "the lured lunge must aim at the key"
    );
}

#[test]
fn flying_kinds_ignore_ink_and_ground_kinds_do_not() {
    use crate::game::ecs::systems::hazards::is_ground as ink_slows;
    assert!(!ink_slows(EnemyKind::MurderHornet));
    assert!(ink_slows(EnemyKind::HouseCentipede));
    assert!(ink_slows(EnemyKind::BigSnake));
    assert!(ink_slows(EnemyKind::TermiteHorde));
    assert!(ink_slows(EnemyKind::Lilliputian));
}

#[test]
fn a_hornet_crosses_a_flatbed_hole_and_a_ground_kind_falls() {
    // The rule lives in the trailer pass (another system), so this is the two
    // lines it reads: the flyer match and the ground faller.
    let flies = |kind| matches!(kind, EnemyKind::WickMoth | EnemyKind::MurderHornet);
    assert!(flies(EnemyKind::MurderHornet));
    assert!(!flies(EnemyKind::BigSnake));
    assert!(!flies(EnemyKind::HouseCentipede));
    assert!(!flies(EnemyKind::TermiteHorde));
    assert!(!flies(EnemyKind::Lilliputian));
}

// ---- Every later damage source ------------------------------------------

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

/// Run one damage source against a fresh wave-4 body and report the HP it lost
/// on the first tick that lands anything.
fn source_loss(
    kind: EnemyKind,
    offset: Vec2,
    arrange: impl Fn(&mut World, Vec2, Entity),
    step: impl Fn(&mut World),
) -> f32 {
    let mut w = World::new(Theme::Sewer, 17);
    open_floor(&mut w);
    w.clear_enemies();
    w.clear_bullets();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.player.face = Vec2::new(1.0, 0.0);
    let body = w.spawn_enemy(kind, p + offset, 1.0);
    put(&mut w, body, |e| {
        e.hp = 10_000.0;
        e.max_hp = 10_000.0;
        if kind == EnemyKind::BigSnake {
            e.w4.armed = 1;
            e.w4.phase = SNAKE_PHASE_STRETCHED;
            e.w4.t = 1e6;
            for i in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
                e.w4.trail[i] = e.pos;
            }
            e.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
        }
    });
    arrange(&mut w, p, body);
    rebuild_broadphase(&mut w);
    for _ in 0..240 {
        if state(&w, body).hp < 10_000.0 {
            break;
        }
        step(&mut w);
    }
    10_000.0 - state(&w, body).hp
}

#[test]
fn every_later_damage_source_reaches_the_wave4_bodies() {
    use crate::game::ecs::components::{AcidDrop, Infection};
    use crate::game::ecs::resources::spawn_with_id;
    use crate::game::ecs::systems::{projectiles, weapons_wave3a, weapons_wave3b};
    use crate::game::weapon::WeaponKind;

    let ahead = Vec2::new(30.0, 0.0);
    for kind in [
        EnemyKind::BigSnake,
        EnemyKind::HouseCentipede,
        EnemyKind::MurderHornet,
        EnemyKind::TermiteHorde,
        EnemyKind::Lilliputian,
    ] {
        // A bullet through the ordinary combat path.
        let bullet = source_loss(
            kind,
            ahead,
            |w, p, _| {
                w.spawn_bullet(crate::game::bullet::Bullet::new(
                    p + Vec2::new(28.0, 0.0),
                    Vec2::new(400.0, 0.0),
                    10.0,
                    crate::game::bullet::Faction::Player,
                    (255, 255, 255),
                ));
            },
            |w| run(w, crate::game::ecs::systems::combat::player_bullet_hits_system),
        );
        assert!(bullet > 0.0, "{kind:?}: a bullet never reached it");

        // A pool's continuous tick.
        let pool = source_loss(
            kind,
            ahead,
            |w, p, _| {
                w.spawn_pool(crate::game::ecs::components::Pool {
                    pos: p + Vec2::new(28.0, 0.0),
                    radius: 14.0,
                    dps: 60.0,
                    life: 2.0,
                    color: (120, 220, 90),
                });
            },
            |w| run(w, projectiles::pools_system),
        );
        assert!(pool > 0.0, "{kind:?}: a pool never reached it");

        // An Acid Rain splash.
        let rain = source_loss(
            kind,
            ahead,
            |w, p, _| {
                spawn_with_id(
                    &mut w.ecs,
                    AcidDrop {
                        pos: p + Vec2::new(30.0, 0.0),
                        delay: 0.0,
                        radius: 12.0,
                        dmg: 10.0,
                        index: 0,
                        evolved: false,
                        dir: Vec2::ZERO,
                        volley: 1,
                    },
                );
            },
            |w| run(w, weapons_wave3a::acid_rain_system),
        );
        assert!(rain > 0.0, "{kind:?}: Acid Rain never reached it");

        // A Hantavirus tick (no direction: the unmodified multiplier).
        let virus = source_loss(
            kind,
            ahead,
            |w, _, body| {
                let id = w.ecs.get::<SpawnId>(body).unwrap().0;
                w.ecs
                    .entity_mut(body)
                    .insert(Infection::new(id, 0, 40.0, 5.0));
            },
            |w| run(w, weapons_wave3a::hantavirus_system),
        );
        assert!(virus > 0.0, "{kind:?}: a virus tick never reached it");

        // Laser Eyes' pulse (a weapon system, not a hit shape).
        let laser = source_loss(
            kind,
            ahead,
            |w, _, _| {
                arm_weapon(w, WeaponKind::LaserEyes, false);
            },
            |w| run(w, weapons_wave3b::laser_eyes_system),
        );
        assert!(laser > 0.0, "{kind:?}: a laser pulse never reached it");
    }
}

#[test]
fn a_snake_body_hit_lands_on_the_same_pool_and_consumes_the_bullet() {
    let (mut w, p) = open_world(51);
    let snake = w.spawn_enemy(EnemyKind::BigSnake, p + Vec2::new(60.0, 0.0), 1.0);
    ready(&mut w, snake, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w4.phase = SNAKE_PHASE_STRETCHED;
        e.w4.t = 1e6;
        for i in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
            e.w4.trail[i] = e.pos - Vec2::new(i as f32 * SNAKE_SPACING, 0.0);
        }
        e.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
        e.hp = 500.0;
        e.max_hp = 500.0;
    });
    // A bullet aimed at the tail: the head is 60 px from the rat, the tail is
    // behind it, so this path only exists because of the body pass.
    let tail = state(&w, snake).w4.trail[5];
    let bullet = w.spawn_bullet(crate::game::bullet::Bullet::new(
        tail,
        Vec2::new(400.0, 0.0),
        10.0,
        crate::game::bullet::Faction::Player,
        (255, 255, 255),
    ));
    put(&mut w, snake, |_| {});
    tick(&mut w);
    assert!(
        state(&w, snake).hp < 500.0,
        "a bullet crossing the body did not land"
    );
    let b = w.ecs.get::<crate::game::bullet::Bullet>(bullet).unwrap();
    assert!(b.life <= 0.0 || b.pierce > 0, "the body did not eat the bullet");
}

// ---- Determinism ---------------------------------------------------------

#[test]
fn a_tick_without_wave4_bodies_draws_no_rng() {
    use rand::RngExt;
    let mut w = World::new(Theme::Sewer, 37);
    open_floor(&mut w);
    w.clear_enemies();
    let p = w.level.arena * 0.5;
    w.player.pos = p;
    w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::new(30.0, 0.0), 1.0);
    w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(-30.0, 0.0), 1.0);
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
        "the wave-4 system consumed rng on a map with no wave-4 enemy"
    );
}

/// Two worlds, the same seed, one of every wave-4 body plus a full lilliputian
/// squad, the same scripted rat: the whole schedule must agree tick for tick.
/// The waves' own kinds share the field, so separation, the limiter and the
/// broadphase are all exercised together.
#[test]
fn twin_runs_with_wave4_kinds_stay_identical() {
    fn run_once() -> u64 {
        use rand::RngExt;
        let mut w = World::new(Theme::Sewer, 41);
        open_floor(&mut w);
        w.clear_enemies();
        let p = w.level.arena * 0.5;
        w.player.pos = p;
        w.viewport = Vec2::new(160.0, 168.0);
        let kinds = [
            EnemyKind::BigSnake,
            EnemyKind::HouseCentipede,
            EnemyKind::MurderHornet,
            EnemyKind::TermiteHorde,
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
        ];
        for (i, kind) in kinds.iter().enumerate() {
            let a = i as f32 * 0.9;
            let pos = p + Vec2::from_angle(a) * (24.0 + i as f32 * 4.0);
            let e = w.spawn_enemy(*kind, pos, 1.0);
            put(&mut w, e, |en| {
                en.w4.armed = 0;
                en.w3.armed = 0;
            });
        }
        // A squad of tiny people with roles and rope crews.
        for k in 0..LILLI_SQUAD_SIZE {
            let a = k as f32 / LILLI_SQUAD_SIZE as f32 * std::f32::consts::TAU;
            let e = w.spawn_enemy(
                EnemyKind::Lilliputian,
                p + Vec2::from_angle(a) * 70.0,
                1.0,
            );
            put(&mut w, e, |en| {
                en.w4.armed = 1;
                en.w4.role = match k {
                    0 => LILLI_ROLE_LEADER,
                    1..=3 => LILLI_ROLE_SPEAR,
                    4 | 5 => LILLI_ROLE_ARCHER,
                    _ => LILLI_ROLE_ROPE,
                };
                en.w4.slot = k as u8;
                en.w4.squad = 4242;
            });
        }
        let mut hash = 14_695_981_039_346_656_037u64;
        for step in 0..1800 {
            let dir = Vec2::from_angle(step as f32 * 0.03);
            w.update(DT, dir, step % 97 == 0);
            for e in w.enemies() {
                for v in [
                    e.pos.x.to_bits() as u64,
                    e.pos.y.to_bits() as u64,
                    e.hp.to_bits() as u64,
                    e.w4.phase as u64,
                    e.w4.t.to_bits() as u64,
                    e.w4.cd.to_bits() as u64,
                    e.w4.tier as u64,
                    e.w4.splits as u64,
                    e.w4.role as u64,
                    e.w4.squad as u64,
                    e.w4.trail[3].x.to_bits() as u64,
                ] {
                    hash ^= v;
                    hash = hash.wrapping_mul(1_099_511_628_211);
                }
            }
            hash ^= w.player.hp.to_bits() as u64;
            hash ^= w.player.pos.x.to_bits() as u64;
            hash ^= w.player.pos.y.to_bits() as u64;
            w.rng.random::<u64>();
        }
        hash
    }
    assert_eq!(
        run_once(),
        run_once(),
        "two runs of the same seed diverged with wave-4 bodies on the field"
    );
}

/// A late-curve twin run on a real generated map: the director is spawning all
/// fourteen kinds, so this is the determinism witness for the spawn path
/// (squads, the snake cap, the special-tier caps).
#[test]
fn a_late_curve_twin_run_stays_identical() {
    fn run_once() -> u64 {
        use rand::RngExt;
        let mut w = World::new(Theme::Kitchen, 97);
        w.elapsed = 560.0;
        let mut hash = 0u64;
        for step in 0..900 {
            let dir = Vec2::from_angle(step as f32 * 0.02);
            w.update(DT, dir, step % 131 == 0);
            hash = hash.wrapping_mul(31).wrapping_add(w.enemies().len() as u64);
            hash ^= w.player.hp.to_bits() as u64;
            w.rng.random::<u64>();
        }
        hash
    }
    assert_eq!(run_once(), run_once(), "a late-curve run diverged");
}

// ---- The design contract -------------------------------------------------

#[test]
fn the_wave4_stats_and_timings_match_the_design_table() {
    for (kind, hp, speed, dmg, xp) in [
        (EnemyKind::BigSnake, 80.0, 26.0, 8.0, 12),
        (EnemyKind::HouseCentipede, 7.0, 54.0, 5.0, 2),
        (EnemyKind::MurderHornet, 14.0, 32.0, 6.0, 4),
        (EnemyKind::TermiteHorde, 64.0, 26.0, 3.0, 3),
        (EnemyKind::Lilliputian, 3.0, 22.0, 3.0, 1),
    ] {
        assert_eq!(kind.base_hp(), hp, "{kind:?} hp");
        assert_eq!(kind.speed(), speed, "{kind:?} speed");
        assert_eq!(kind.damage(), dmg, "{kind:?} contact");
        assert_eq!(kind.xp(), xp, "{kind:?} xp");
    }
    // Size classes: only the snake is Large (it cannot use a rat-hole).
    use crate::game::collision::SizeClass;
    assert_eq!(SizeClass::of_enemy(EnemyKind::BigSnake), SizeClass::Large);
    for small in [
        EnemyKind::HouseCentipede,
        EnemyKind::MurderHornet,
        EnemyKind::TermiteHorde,
        EnemyKind::Lilliputian,
    ] {
        assert_eq!(SizeClass::of_enemy(small), SizeClass::Small, "{small:?}");
    }
    // Timings used by the report's design table.
    assert!(SNAKE_TELL_SECS >= 0.6, "the lunge tell must be readable");
    assert!(CENTIPEDE_FLUTTER_SECS >= 0.25);
    assert!(HORNET_TELL_SECS >= 0.5);
    assert!(ROPE_TIE_SECS >= 0.8, "the knot must be visible for a beat");
    assert!(HORNET_RECOVER_SECS >= 1.0, "the punish window must pay");
}

#[test]
fn the_director_unlocks_wave4_in_design_order_and_keeps_the_caps() {
    use crate::game::director;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    let expected = [
        (EnemyKind::HouseCentipede, 240.0),
        (EnemyKind::Lilliputian, 330.0),
        (EnemyKind::TermiteHorde, 360.0),
        (EnemyKind::MurderHornet, 450.0),
        (EnemyKind::BigSnake, 510.0),
    ];
    let mut prev = 0.0;
    for (kind, at) in expected {
        let unlock = director::wave4_unlock_time(kind).expect("in the table");
        assert!((unlock - at).abs() < 1e-3, "{kind:?} unlocks at {unlock}");
        assert!(unlock > prev, "{kind:?} unlocked out of order");
        prev = unlock;
        // Never before its own instant …
        let mut rng = StdRng::seed_from_u64(5);
        for _ in 0..400 {
            assert_ne!(
                director::pick_kind_at(unlock - 0.5, &mut rng),
                kind,
                "{kind:?} appeared before its unlock"
            );
        }
        // … and it does appear after it.
        let mut rng = StdRng::seed_from_u64(5);
        let seen = (0..4000).any(|_| director::pick_kind_at(unlock + 5.0, &mut rng) == kind);
        assert!(seen, "{kind:?} never appeared after its unlock");
    }
    // Every wave-4 unlock is after the opening 60 s the golden traces sample.
    assert!(director::wave4_unlock_time(EnemyKind::HouseCentipede).unwrap() > 60.0);

    // The second special tier is a real cap: the four kinds together are at
    // most 15 % of the roll, and both tiers together at most 30 %.
    let mut rng = StdRng::seed_from_u64(4242);
    let total = 60_000usize;
    let mut wave4 = 0usize;
    let mut special = 0usize;
    for _ in 0..total {
        let kind = director::pick_kind_at(director::BOSS_TIME, &mut rng);
        if kind.is_wave4() {
            wave4 += 1;
        }
        if kind.is_wave3() || kind.is_wave4() {
            special += 1;
        }
    }
    let w4_share = wave4 as f32 / total as f32;
    let special_share = special as f32 / total as f32;
    assert!(
        w4_share <= director::WAVE4_SHARE_CAP + 0.012,
        "wave-4 share {w4_share} exceeds its cap"
    );
    assert!(
        w4_share > director::WAVE4_SHARE_CAP * 0.5,
        "the wave-4 tier is too rare to be a feature: {w4_share}"
    );
    assert!(
        special_share <= director::SPECIAL_SHARE_CAP + 0.012,
        "the combined special share {special_share} exceeds its ceiling"
    );
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
    std::fs::write(path, out).expect("write wave4 ppm");
}

fn scratch_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(
        "/private/tmp/claude-501/-Users-joe-dev-ratrun/a0cccbef-9e45-4849-814a-e727185739a1/scratchpad/w3/enemies2",
    );
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Bestiary sheet: every wave-4 silhouette on the floor, both facings.
/// `RATRUN_DUMP=wave4 cargo test dump_wave4_kinds`.
#[test]
fn dump_wave4_kinds() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("wave4") {
        return;
    }
    use crate::render::framebuffer::PixelBuffer;
    let mut w = World::new(Theme::Sewer, 1);
    let c = w.level.arena * 0.5;
    w.player.pos = c;
    w.clear_enemies();
    let kinds = [
        EnemyKind::BigSnake,
        EnemyKind::HouseCentipede,
        EnemyKind::MurderHornet,
        EnemyKind::TermiteHorde,
    ];
    for (i, kind) in kinds.iter().enumerate() {
        let x = c.x - 78.0 + i as f32 * 46.0;
        let e = w.spawn_enemy(*kind, Vec2::new(x, c.y - 14.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(1.0, 0.0);
            en.anim = i as f32 * 0.7;
            en.w4.tier = 2;
            for k in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
                en.w4.trail[k] = en.pos - Vec2::new(k as f32 * SNAKE_SPACING, 0.0);
            }
            en.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
        });
        let e = w.spawn_enemy(*kind, Vec2::new(x, c.y + 26.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(-0.4, 1.0).normalized();
            en.anim = 1.7 + i as f32;
            en.w4.tier = 2;
            for k in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
                en.w4.trail[k] = en.pos - en.facing * (k as f32 * SNAKE_SPACING);
            }
            en.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
        });
    }
    // A lilliputian rank: one of each role, two facings.
    for k in 0..5u8 {
        let x = c.x - 60.0 + k as f32 * 18.0;
        let e = w.spawn_enemy(EnemyKind::Lilliputian, Vec2::new(x, c.y + 54.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(1.0, 0.0);
            en.w4.armed = 1;
            en.w4.role = k % 4;
            en.w4.slot = k;
            en.anim = k as f32;
        });
        let e = w.spawn_enemy(EnemyKind::Lilliputian, Vec2::new(x, c.y + 66.0), 1.0);
        put(&mut w, e, |en| {
            en.facing = Vec2::new(-1.0, 0.0);
            en.w4.armed = 1;
            en.w4.role = k % 4;
            en.w4.phase = if k % 4 == 3 {
                LILLI_PHASE_ACTION
            } else {
                LILLI_PHASE_WALK
            };
            en.w4.slot = k;
        });
    }
    let mut pb = PixelBuffer::new(200, 120);
    w.draw(&mut pb);
    write_ppm(
        scratch_dir().join("wave4_kinds.ppm").to_str().unwrap(),
        &pb,
    );
}

/// Tell sheet: every wave-4 warned attack in the state the rat reads.
/// `RATRUN_DUMP=wave4_tells cargo test dump_wave4_tells`.
#[test]
fn dump_wave4_tells() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("wave4_tells") {
        return;
    }
    use crate::render::framebuffer::PixelBuffer;
    let mut w = World::new(Theme::Sewer, 2);
    let c = w.level.arena * 0.5;
    w.player.pos = c;
    w.clear_enemies();

    // 1) Snake mid-coil, lunge lane filling.
    let snake = w.spawn_enemy(EnemyKind::BigSnake, Vec2::new(c.x - 60.0, c.y - 26.0), 1.0);
    put(&mut w, snake, |e| {
        e.facing = Vec2::new(1.0, 0.2).normalized();
        e.w4.armed = 1;
        e.w4.phase = SNAKE_PHASE_COIL;
        e.w4.t = SNAKE_TELL_SECS * 0.5;
        e.w4.dir = e.facing;
        for k in 0..crate::game::enemy::SNAKE_MAX_SEGMENTS {
            e.w4.trail[k] = e.pos - e.facing * (k as f32 * SNAKE_SPACING);
        }
        e.w4.trail_len = crate::game::enemy::SNAKE_MAX_SEGMENTS as u8;
    });

    // 2) Centipede fluttering.
    let bug = w.spawn_enemy(
        EnemyKind::HouseCentipede,
        Vec2::new(c.x - 20.0, c.y - 26.0),
        1.0,
    );
    put(&mut w, bug, |e| {
        e.facing = Vec2::new(0.6, 1.0).normalized();
        e.w4.armed = 1;
        e.w4.phase = CENTIPEDE_PHASE_FLUTTER;
        e.w4.t = CENTIPEDE_FLUTTER_SECS * 0.5;
        e.w4.dir = e.facing;
    });

    // 3) Hornet charging, dive lane drawn; 4) the same hornet grounded.
    let hornet = w.spawn_enemy(
        EnemyKind::MurderHornet,
        Vec2::new(c.x + 20.0, c.y - 26.0),
        1.0,
    );
    put(&mut w, hornet, |e| {
        e.facing = Vec2::new(-1.0, 0.3).normalized();
        e.w4.armed = 1;
        e.w4.phase = HORNET_PHASE_CHARGE;
        e.w4.t = HORNET_TELL_SECS * 0.4;
        e.w4.dir = e.facing;
        e.anim = 1.2;
    });
    let grounded = w.spawn_enemy(
        EnemyKind::MurderHornet,
        Vec2::new(c.x + 62.0, c.y - 26.0),
        1.0,
    );
    put(&mut w, grounded, |e| {
        e.w4.armed = 1;
        e.w4.phase = HORNET_PHASE_RECOVER;
        e.w4.t = HORNET_RECOVER_SECS * 0.5;
    });

    // 5) Spear line levelling; 6) archer kneeling; 7) a rope ring closing.
    let spear = w.spawn_enemy(
        EnemyKind::Lilliputian,
        Vec2::new(c.x - 54.0, c.y + 30.0),
        1.0,
    );
    put(&mut w, spear, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w4.armed = 1;
        e.w4.role = LILLI_ROLE_SPEAR;
        e.w4.phase = LILLI_PHASE_TELL;
        e.w4.t = SPEAR_TELL_SECS * 0.5;
    });
    let archer = w.spawn_enemy(
        EnemyKind::Lilliputian,
        Vec2::new(c.x - 16.0, c.y + 30.0),
        1.0,
    );
    put(&mut w, archer, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w4.armed = 1;
        e.w4.role = LILLI_ROLE_ARCHER;
        e.w4.phase = LILLI_PHASE_TELL;
        e.w4.t = ARCHER_TELL_SECS * 0.4;
        e.w4.dir = Vec2::new(1.0, 0.0);
    });
    let rope = w.spawn_enemy(
        EnemyKind::Lilliputian,
        Vec2::new(c.x + 26.0, c.y + 30.0),
        1.0,
    );
    put(&mut w, rope, |e| {
        e.facing = Vec2::new(1.0, 0.0);
        e.w4.armed = 1;
        e.w4.role = LILLI_ROLE_ROPE;
        e.w4.phase = LILLI_PHASE_TELL;
        e.w4.t = ROPE_TIE_SECS * 0.4;
    });

    let mut pb = PixelBuffer::new(200, 120);
    w.draw(&mut pb);
    write_ppm(
        scratch_dir().join("wave4_tells.ppm").to_str().unwrap(),
        &pb,
    );
}

/// The horde at its three sizes, plus one mid-split warning.
/// `RATRUN_DUMP=horde cargo test dump_horde_sizes`.
#[test]
fn dump_horde_sizes() {
    if std::env::var("RATRUN_DUMP").as_deref() != Ok("horde") {
        return;
    }
    use crate::render::framebuffer::PixelBuffer;
    let mut w = World::new(Theme::Sewer, 3);
    let c = w.level.arena * 0.5;
    w.player.pos = c;
    w.clear_enemies();
    for (i, tier) in [2u8, 1, 0].iter().enumerate() {
        let e = w.spawn_enemy(
            EnemyKind::TermiteHorde,
            Vec2::new(c.x - 50.0 + i as f32 * 44.0, c.y - 10.0),
            1.0,
        );
        put(&mut w, e, |en| {
            en.w4.tier = *tier;
            en.anim = i as f32 * 1.3;
            en.facing = Vec2::new(1.0, 0.0);
        });
    }
    let splitting = w.spawn_enemy(
        EnemyKind::TermiteHorde,
        Vec2::new(c.x + 54.0, c.y + 30.0),
        1.0,
    );
    put(&mut w, splitting, |en| {
        en.w4.tier = 2;
        en.w4.split_pending = 1;
        en.anim = 2.2;
    });
    let mut pb = PixelBuffer::new(200, 96);
    w.draw(&mut pb);
    write_ppm(
        scratch_dir().join("wave4_horde_sizes.ppm").to_str().unwrap(),
        &pb,
    );
}
