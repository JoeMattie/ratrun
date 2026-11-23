//! Narrow tests for the boss framework (P7).
//!
//! Every test drives one system through `begin_tick` → `run_system_cached` →
//! `end_tick`, the same harness the other `systems/*.rs` files use. The art
//! files are the gameplay spec, so the numbers asserted here are the art's
//! constants, not copies.

use super::*;
use crate::game::art_traps::MousetrapView;
use crate::game::collision::TileMap;
use crate::game::ecs::systems::projectiles;
use crate::game::level::{MAP_COLS, MAP_ROWS, Theme};
use crate::game::world::World;

const DT: f32 = 1.0 / 60.0;

fn boss_of(w: &mut World) -> Option<Boss> {
    let mut query = w.ecs.query::<&Boss>();
    query.iter(&w.ecs).next().cloned()
}

/// The boss this floor's seed actually fields (the wave-3 pool seam): the
/// framework tests assert against the spawn system's own choice rather than
/// against a floor's original boss, so a pooled floor is covered by the same
/// tests.
fn floor_boss(w: &World) -> BossKind {
    BossKind::pool_pick(w.level.theme, w.level.seed)
}

fn spawn_boss(w: &mut World) {
    w.ecs.resource_mut::<BossDue>().due = true;
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(boss_spawn_system)
        .expect("boss spawn system must run");
    w.end_tick();
}

fn update_boss(w: &mut World, dt: f32) {
    w.begin_tick(dt, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(boss_update_system)
        .expect("boss update system must run");
    w.end_tick();
}

// ---- spawn ---------------------------------------------------------------

#[test]
fn boss_spawns_out_of_sight_when_due() {
    let mut w = World::new(Theme::Sewer, 7);
    let arena = w.level.fight_rect_around(w.player.pos);
    spawn_boss(&mut w);

    let boss = boss_of(&mut w).expect("BossDue spawned the boss");
    assert_eq!(boss.kind, floor_boss(&w));
    assert!(arena.contains_world(boss.pos), "boss must spawn in its fight rectangle");
    assert!(boss.hp > 0.0);
    assert!(boss.max_hp > 0.0);
    assert_eq!(boss.phase, 1);
    // The boss's private rng stream is seeded from the run seed.
    assert!(!w.ecs.resource::<BossFight>().level_complete);
}

/// **Batch 12.** The boss used to arrive in a dedicated arena. It now arrives
/// where the rat is: out of sight (outside the *widest* view, the same
/// rectangle the horde's spawns are measured against) and in a fight rectangle
/// of the floor's standard size that contains the rat and is centred on it.
#[test]
fn every_boss_arrives_out_of_sight_around_the_rat() {
    use crate::game::ecs::systems::spawn::widest_view;
    for (i, kind) in BossKind::all().iter().copied().enumerate() {
        // A different seed per kind so no pool, map or rng state is shared.
        let mut w = World::new(Theme::playable()[i % Theme::playable().len()], 100 + i as u64);
        // A known viewport: the App's own default, so the widest view is the
        // same rectangle a real run measures.
        w.viewport = crate::math::Vec2::new(120.0, 74.0);
        let rat = w.player.pos;
        w.boss_override = Some(kind);
        spawn_boss(&mut w);

        let boss = boss_of(&mut w).expect("BossDue spawned the boss");
        assert_eq!(boss.kind, kind);
        let half = widest_view(w.viewport) * 0.5;
        let d = boss.pos - rat;
        assert!(
            d.x.abs() > half.x - 1.0 || d.y.abs() > half.y - 1.0,
            "{kind:?}: arrived inside the widest view ({d:?} vs {:?})",
            half
        );
        let arena = boss.arena;
        assert!(arena.contains_world(rat), "{kind:?}: the rectangle misses the rat");
        assert!(arena.contains_world(boss.pos), "{kind:?}: the boss is outside its rectangle");
        // Centred on the rat, then clamped inside the map (the Flatbed's
        // tailgate is near the map's west edge).
        assert_eq!(
            arena,
            w.level.fight_rect_around(rat),
            "{kind:?}: the rectangle is not the one built around the rat"
        );
    }
}

#[test]
fn boss_does_not_spawn_without_the_due_signal() {
    let mut w = World::new(Theme::Sewer, 8);
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(boss_spawn_system)
        .expect("boss spawn system must run");
    w.end_tick();
    assert!(boss_of(&mut w).is_none());
}

#[test]
fn boss_kind_follows_the_campaign_level() {
    assert_eq!(BossKind::for_level(0), BossKind::Cat);
    assert_eq!(BossKind::for_level(1), BossKind::Broom);
    assert_eq!(BossKind::for_level(2), BossKind::Mower);
    assert_eq!(BossKind::for_theme(Theme::Sewer), BossKind::Cat);
    assert_eq!(BossKind::for_theme(Theme::Kitchen), BossKind::Broom);
    assert_eq!(BossKind::for_theme(Theme::Lab), BossKind::Mower);
}

// ---- hit shapes ----------------------------------------------------------

#[test]
fn cat_hit_shapes_are_the_art_body_circles() {
    let arena = TileRect::new(0, 0, 400, 400);
    let boss = Boss::new(
        BossKind::Cat,
        Vec2::new(200.0, 200.0),
        art_boss_cat::BOSS_BASE_HP,
        arena,
        Vec2::new(200.0, 200.0),
        false,
    );
    let shapes = boss.hit_shapes();
    assert_eq!(shapes.iter().count(), 3);
    // A bullet crossing the body center must hit; a bullet 60 px away must not.
    assert!(shapes.segment_hits_grounded(
        boss.pos - Vec2::new(2.0, 0.0),
        boss.pos + Vec2::new(2.0, 0.0),
        1.0
    ));
    assert!(!shapes.segment_hits_grounded(
        boss.pos + Vec2::new(60.0, 0.0),
        boss.pos + Vec2::new(64.0, 0.0),
        1.0
    ));
}

#[test]
fn lifted_cat_pounce_is_not_hit_by_ground_weapons() {
    let arena = TileRect::new(0, 0, 400, 400);
    let mut boss = Boss::new(
        BossKind::Cat,
        Vec2::new(200.0, 200.0),
        art_boss_cat::BOSS_BASE_HP,
        arena,
        Vec2::new(200.0, 200.0),
        false,
    );
    boss.set_state(BossState::Cat(BossCatState::Pounce), 0.6);
    boss.elapsed = 0.3;
    // The cat's hit circles are all grounded in the shared shape set; the
    // airborne window is carried by `Boss::is_airborne` so the contact/damage
    // systems can skip it.
    assert!(boss.is_airborne());
}

// ---- hazards -------------------------------------------------------------

#[test]
fn hazard_warns_then_activates_then_expires() {
    let mut hazard = BossHazard::new(
        HazardShape::Circle {
            center: Vec2::ZERO,
            radius: 10.0,
        },
        1.0,
        0.5,
        12.0,
    );
    assert!(!hazard.is_active());
    assert!(hazard.shape.contains(Vec2::new(5.0, 0.0), 0.0));
    assert!(!hazard.shape.contains(Vec2::new(20.0, 0.0), 0.0));
    hazard.elapsed = 1.0;
    assert!(hazard.is_active());
    hazard.elapsed = 1.5;
    assert!(!hazard.is_active());
    assert!(hazard.is_expired());
}

#[test]
fn hazard_shapes_cover_their_footprints() {
    let ring = HazardShape::Annulus {
        center: Vec2::ZERO,
        inner: 10.0,
        outer: 20.0,
    };
    assert!(!ring.contains(Vec2::ZERO, 0.0));
    assert!(ring.contains(Vec2::new(15.0, 0.0), 0.0));

    let arc = HazardShape::Arc {
        center: Vec2::ZERO,
        inner: 0.0,
        outer: 20.0,
        facing: Vec2::new(1.0, 0.0),
        half_angle: 0.5,
    };
    assert!(arc.contains(Vec2::new(10.0, 0.0), 0.0));
    assert!(!arc.contains(Vec2::new(-10.0, 0.0), 0.0));

    let corridor = HazardShape::Corridor {
        from: Vec2::new(-10.0, 0.0),
        to: Vec2::new(10.0, 0.0),
        half_width: 3.0,
    };
    assert!(corridor.contains(Vec2::new(0.0, 2.0), 0.0));
    assert!(!corridor.contains(Vec2::new(0.0, 5.0), 0.0));
}

// ---- phases --------------------------------------------------------------

#[test]
fn phase_thresholds_follow_the_art() {
    assert_eq!(BossKind::Cat.phase_for(0.9), 1);
    assert_eq!(BossKind::Cat.phase_for(0.4), 2);
    assert_eq!(BossKind::Broom.phase_for(0.9), 1);
    assert_eq!(BossKind::Broom.phase_for(0.4), 2);
    assert_eq!(BossKind::Mower.phase_for(0.9), 1);
    assert_eq!(BossKind::Mower.phase_for(0.5), 2);
    assert_eq!(BossKind::Mower.phase_for(0.2), 3);
}

// ---- death ---------------------------------------------------------------

#[test]
fn boss_death_plays_its_beat_then_completes_the_level_once() {
    let mut w = World::new(Theme::Sewer, 9);
    spawn_boss(&mut w);
    {
        let mut query = w.ecs.query::<&mut Boss>();
        for mut boss in query.iter_mut(&mut w.ecs) {
            boss.hp = 0.0;
        }
    }
    update_boss(&mut w, DT);
    // The lethal tick only *starts* the art's death state: the level is not
    // complete yet, so the App cannot cut the death animation short.
    let boss = boss_of(&mut w).expect("death state plays before despawn");
    assert!(boss.dead);
    assert!(boss.death_announced);
    assert_eq!(boss.state, floor_boss(&w).death_state());
    assert!(
        !w.level_complete,
        "the level completes after the death state, not on the lethal tick"
    );
    assert!(!w.ecs.resource::<BossFight>().level_complete);
    // §6: the boss's 30 gold is credited to the wallet at the clear, not
    // dropped as a coin at its body (which a ranged kill outside magnet range
    // could lose entirely). Nothing is on the floor yet.
    let mut gold = w.ecs.query::<&crate::game::gem::Gold>();
    assert_eq!(gold.iter(&w.ecs).count(), 0, "the boss drops no coin now");

    // Every tick of the beat keeps the boss (and never re-raises anything).
    // (The floor's boss, not "the cat": the Sewer pool fields the Jaguar or
    // the Horseshoe by seed, and their death beats differ in length.)
    let ticks = (floor_boss(&w).death_duration() / DT).ceil() as usize + 2;
    let mut completions = 0usize;
    let mut was_complete = false;
    for _ in 0..ticks {
        update_boss(&mut w, DT);
        if w.level_complete && !was_complete {
            completions += 1;
            was_complete = true;
        }
    }
    assert_eq!(completions, 1, "the signal must rise exactly once");
    assert!(w.ecs.resource::<BossFight>().level_complete);
    assert!(
        w.gold >= crate::game::shop::BOSS_GOLD,
        "the boss's gold must be in the wallet at the clear ({})",
        w.gold
    );
    assert!(
        boss_of(&mut w).is_none(),
        "the death state despawns the boss when it ends"
    );

    // And it stays risen (and the wallet credit is not paid twice).
    let banked = w.gold;
    update_boss(&mut w, DT);
    assert!(w.level_complete);
    assert_eq!(completions, 1);
    assert_eq!(w.gold, banked, "the boss's gold is credited once");
}

/// §6 for **every boss of every floor** (both pool mates of the Sewer and the
/// Kitchen included), on a campaign floor and on a practice floor
/// (`difficulty_override = Some(0)`): the clear credits exactly
/// [`crate::game::shop::BOSS_GOLD`] to the wallet, once, and leaves no coin on
/// the floor. The credit lives in the shared death path of
/// `boss_update_system`, so a boss added later inherits it; this pins that.
#[test]
fn every_boss_credits_its_gold_exactly_once_at_the_clear() {
    // Every pool mate of every floor: a seed is searched per boss, so a pooled
    // floor (Sewer, Kitchen) is cleared with each of its bosses.
    let mut cleared = 0usize;
    for (theme, want) in Theme::all()
        .into_iter()
        .flat_map(|t| BossKind::pool_for_theme(t).iter().map(move |k| (t, *k)))
    {
        let seed = (0..256u64)
            .find(|s| BossKind::pool_pick(theme, *s) == want)
            .expect("every pool mate is reachable by seed");
        cleared += 1;
        for practice in [false, true] {
            let mut w = World::new(theme, seed);
            if practice {
                w.difficulty_override = Some(0);
            }
            spawn_boss(&mut w);
            let kind = boss_of(&mut w).expect("the floor's boss").kind;
            assert_eq!(kind, floor_boss(&w));
            assert_eq!(kind, want);
            let before = w.gold;
            {
                let mut query = w.ecs.query::<&mut Boss>();
                for mut boss in query.iter_mut(&mut w.ecs) {
                    boss.hp = 0.0;
                }
            }
            let ticks = (kind.death_duration() / DT).ceil() as usize + 120;
            for _ in 0..ticks {
                update_boss(&mut w, DT);
            }
            assert!(w.level_complete, "{kind:?} practice={practice}: never cleared");
            assert!(boss_of(&mut w).is_none(), "{kind:?}: the death beat ended");
            assert_eq!(
                w.gold - before,
                crate::game::shop::BOSS_GOLD,
                "{kind:?} practice={practice}: the clear pays 30 gold exactly once"
            );
            let mut coins = w.ecs.query::<&crate::game::gem::Gold>();
            assert_eq!(coins.iter(&w.ecs).count(), 0, "{kind:?}: no boss coin");
        }
    }
    // The five `Theme::all()` floors field eight bosses between them (the
    // Magpie's flatbed is not a menu floor): the Cat and the Horseshoe, the
    // Sweeper and SUBJECT R-01, the Mower and the King Cobra, the Mangle, the
    // Night Edition.
    assert_eq!(cleared, 8, "every pool mate of every floor was cleared");
}

/// The boss's death breaks the horde, and that despawn has to happen **after**
/// the last `EnemyIndex` reader.
///
/// `enemy_move::grid_refresh_system` publishes the `SpawnId`-ordered entity
/// index that the combat systems walk, and
/// `resources::debug_assert_enemy_index` (debug builds only) verifies on every
/// read that the cache still matches the live enemy set. `boss_update_system`
/// runs inside that window, so the horde break cannot despawn enemies there —
/// it raises [`BossFight::horde_break`] and `loot::reap_enemies_system`, after
/// `contact_system`, does the removal. This test drives the whole real tick
/// schedule (director, horde, boss systems, combat, loot) for each of the three
/// campaign bosses with a 60-strong horde on the field, kills the boss, and
/// ticks on: any despawn in the index window would panic here in a debug
/// build (`cargo test`) and be a silent stale read in release.
#[test]
fn boss_death_breaks_the_horde_without_invalidating_the_enemy_index() {
    for (theme, level) in [(Theme::Sewer, 0usize), (Theme::Kitchen, 1), (Theme::Lab, 2)] {
        let mut w = World::new(theme, 31);
        w.level_index = level;
        spawn_boss(&mut w);
        let boss = boss_of(&mut w).expect("BossDue spawned the boss");
        assert_eq!(boss.kind, floor_boss(&w));
        // 60 enemies on the field, spread over the arena so a few of them
        // stand in the boss's death hazards too.
        let centre = boss.arena.center_world();
        for i in 0..60 {
            let angle = i as f32 * 0.41;
            let dist = 24.0 + (i % 9) as f32 * 7.0;
            let pos = centre + Vec2::from_angle(angle) * dist;
            w.spawn_enemy(EnemyKind::Skitterer, pos, 1.0);
        }
        assert!(
            w.enemy_count() >= 50,
            "the horde has to be there for the break to be a test: {}",
            w.enemy_count()
        );
        let kills_before = w.kills;
        let score_before = w.score;

        // Lethal hit, then the real schedule: the death tick starts the dance
        // and breaks the horde; the next 120 ticks (2 s at 60 Hz) keep every
        // system — and every `EnemyIndex` reader — running.
        {
            let mut query = w.ecs.query::<&mut Boss>();
            for mut boss in query.iter_mut(&mut w.ecs) {
                boss.hp = 0.0;
            }
        }
        w.update(DT, Vec2::ZERO, false);
        assert_eq!(
            w.enemy_count(),
            0,
            "the horde breaks on the tick the boss dies ({theme:?})"
        );
        // The break is not a reap: no kills, no score, no drops.
        assert_eq!(w.kills, kills_before, "the fleeing horde must not count");
        assert_eq!(w.score, score_before, "the fleeing horde must not score");
        for _ in 0..120 {
            w.update(DT, Vec2::ZERO, false);
        }
    }
}

// ---- cues ----------------------------------------------------------------

#[test]
fn every_cue_maps_to_its_sound_and_at_least_one_particle() {
    use crate::game::particle::Particles;

    // The cue channel is driven directly (a controller only queues the enum),
    // so the mapping is tested on the same function the system drains with.
    let all = [
        BossCue::CatCrouch,
        BossCue::CatPounce,
        BossCue::CatLand,
        BossCue::CatSkid,
        BossCue::CatSwipe,
        BossCue::CatRoar,
        BossCue::CatFangs,
        BossCue::CatHurt,
        BossCue::CatDeath,
        BossCue::BroomStomp,
        BossCue::BroomSweep,
        BossCue::BroomDustpan,
        BossCue::BroomSlam,
        BossCue::BroomHurtHop,
        BossCue::BroomHop,
        BossCue::BroomDefeat,
        BossCue::MowerScan,
        BossCue::MowerLockOn,
        BossCue::MowerWindup,
        BossCue::MowerCharge,
        BossCue::MowerWallImpact,
        BossCue::MowerFlip,
        BossCue::MowerOverheat,
        BossCue::MowerDock,
        BossCue::MowerDeath,
    ];
    for cue in all {
        // A real particle buffer: `Particles::default()` has a zero cap, so
        // every `budget_count` would (correctly) return nothing.
        let mut ps = Particles::new(crate::game::ecs::constants::PARTICLE_CAP);
        let mut rng = StdRng::seed_from_u64(7);
        let mut sfx = SfxQueue::default();
        let mut boss = Boss::new(
            BossKind::Cat,
            Vec2::new(300.0, 300.0),
            1000.0,
            TileRect::new(0, 0, 64, 64),
            Vec2::new(280.0, 300.0),
            false,
        );
        boss.target = Vec2::new(320.0, 300.0);
        boss.facing = Vec2::new(1.0, 0.0);
        let before = ps.len();
        emit_cues(&[cue], &boss, (120, 20, 30), &mut ps, &mut rng, &mut sfx);
        assert_eq!(sfx.0, vec![cue.sfx()], "{cue:?} played the wrong sound");
        assert!(ps.len() > before, "{cue:?} emitted no particles");
    }
}

// ---- push-only hazards ---------------------------------------------------

#[test]
fn a_zero_damage_hazard_pushes_without_hurting() {
    let mut w = World::new(Theme::Sewer, 22);
    let mut boss = Boss::new(
        BossKind::Cat,
        Vec2::new(300.0, 300.0),
        1000.0,
        TileRect::new(0, 0, 64, 64),
        Vec2::new(280.0, 300.0),
        false,
    );
    boss.push_hazard(
        BossHazard::new(
            HazardShape::Annulus {
                center: Vec2::new(300.0, 300.0),
                inner: 26.0,
                outer: 44.0,
            },
            0.0,
            0.5,
            0.0,
        )
        .with_knockback(85.0),
    );
    w.ecs.spawn((boss, SpawnId(0)));
    w.player.pos = Vec2::new(300.0 + 35.0, 300.0);
    let hp = w.player.hp;
    let start = w.player.pos;
    for _ in 0..6 {
        update_boss(&mut w, DT);
    }
    assert!((w.player.hp - hp).abs() < 1e-6, "a push-only ring dealt damage");
    assert!(
        w.player.pos.dist(start) > 2.0,
        "the ring must shove the rat outward"
    );
    assert!(w.player.iframes <= 0.0, "a shove must not spend i-frames");
}

// ---- grounded shapes -----------------------------------------------------

#[test]
fn the_lifted_foot_neither_contacts_nor_is_targeted() {
    let arena = TileRect::new(0, 0, 200, 200);
    let mut boss = Boss::new(
        BossKind::Broom,
        Vec2::new(1600.0, 1280.0),
        1600.0,
        arena,
        Vec2::new(1600.0, 1280.0),
        false,
    );
    boss.facing = Vec2::new(1.0, 0.0);
    boss.set_state(BossState::Broom(BroomBossState::StompWalk { left_foot: true }), 1.15);
    boss.elapsed = 0.2 * 1.15;
    let feet = art_boss_broom::foot_hitboxes(&boss.broom_view());
    let lifted = feet.iter().find(|f| !f.grounded).expect("one slipper is aloft");
    let planted = feet.iter().find(|f| f.grounded).expect("one slipper is planted");
    // Auto-aim never targets the lifted foot.
    let targets: Vec<Vec2> = boss.target_points().iter().collect();
    assert!(
        targets.iter().all(|p| p.dist(lifted.pos) > 1e-3),
        "a lifted foot attracted auto-aim"
    );
    assert!(
        targets.iter().any(|p| p.dist(planted.pos) < 1e-3),
        "the planted foot must stay targetable"
    );
    // Body contact goes through grounded shapes only.
    let shapes = boss.hit_shapes();
    assert!(!shapes.circle_hits_grounded(lifted.pos, 1.0));
    assert!(shapes.circle_hits_grounded(planted.pos, 1.0));
}

// ---- movement ------------------------------------------------------------

#[test]
fn boss_never_ends_a_tick_inside_a_wall() {
    let mut w = World::new(Theme::Sewer, 10);
    spawn_boss(&mut w);
    let radius = BossKind::Cat.body_radius();
    for _ in 0..120 {
        update_boss(&mut w, DT);
        let boss = boss_of(&mut w).expect("boss alive");
        assert!(
            w.level
                .tiles
                .circle_clear(boss.pos, radius, SizeClass::Large),
            "large body left the passable cells at {:?}",
            boss.pos
        );
    }
}

// ---- determinism ---------------------------------------------------------

#[test]
fn twin_runs_with_a_boss_match() {
    let run = |seed: u64| {
        let mut w = World::new(Theme::Kitchen, seed);
        spawn_boss(&mut w);
        for _ in 0..120 {
            update_boss(&mut w, DT);
        }
        let boss = boss_of(&mut w).expect("boss alive");
        (boss.pos, boss.hp, boss.state, boss.phase, w.level_complete)
    };
    let a = run(4242);
    let b = run(4242);
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
    assert_eq!(a.2, b.2);
    assert_eq!(a.3, b.3);
    assert_eq!(a.4, b.4);
}

// ---- the siege: a rat a large body cannot follow --------------------------
//
// A level cannot end until the boss dies, and the level design has pockets no
// large body can enter (a maze's one-way door, the Sewer's escape rat-hole, a
// 1-cell squeeze). The verifier's soft-lock (seed 17986931858700579504: Expert,
// boss alive 1244 s, `Cat(Stalk)` at full HP while the rejoin kept putting it
// 130 px away behind the same wall) is that shape. These two tests pin the
// framework's answer: the rejoin never teleports the boss somewhere it cannot
// walk to the rat, and a walled-off boss tears a body-sized hole and keeps
// coming.

/// A rat parked in a 2 x 2-cell pocket with a solid wall ring around it, on an
/// otherwise empty map. No spot on any [`REJOIN_RADII`] ring is both standable
/// and walkable to the rat, which is the situation the siege exists for.
fn walled_pocket_world() -> World {
    let mut w = World::new(Theme::Sewer, 11);
    open_arena(&mut w);
    w.clear_enemies();
    // Ring: cells 30..36 x 8..14; interior 31..35 x 9..13 (64 px, so a boss
    // that gets in can stand and fight).
    for y in 8..14 {
        for x in 30..36 {
            let interior = (31..35).contains(&x) && (9..13).contains(&y);
            if !interior {
                w.level
                    .tiles
                    .set(
                        x,
                        y,
                        crate::game::collision::TileKind::Wall(
                            crate::game::art_walls::WallKind::Solid,
                        ),
                    );
            }
        }
    }
    w.level.tiles.rebuild_masks();
    w.flow = crate::game::nav::FlowField::new(&w.level.tiles);
    w.player.pos = Vec2::new(33.0 * 16.0, 11.0 * 16.0);
    // A big pool so the test measures the hole, not a rat death.
    w.player.loadout.stats.max_hp = 10_000.0;
    w.player.hp = 10_000.0;
    w
}

#[test]
fn a_rejoin_never_teleports_the_boss_into_a_wall() {
    let w = walled_pocket_world();
    let radius = BossKind::Cat.body_radius();
    if let Some(spot) = rejoin_spot(&w.level, w.player.pos, radius) {
        assert!(
            w.level.tiles.circle_clear(spot, radius, SizeClass::Large),
            "a rejoin spot must be body-clear ({spot:?})"
        );
        // ...and it is only accepted as a *walkable* ring spot when it is one:
        // the fallback (a rat no large body can reach) is handled by the siege,
        // whose own test follows.
        assert!(
            walkable_line(&w.level, spot, w.player.pos, radius, SizeClass::Large)
                || spot.dist(w.player.pos) < 220.0,
            "a far rejoin spot must be walkable to the rat ({spot:?})"
        );
    }
}

/// The rejoin waits for the neutral beat: a boss that is owed a relocation but
/// is in the middle of a set piece stays where its attack is, and is walked
/// back on the first neutral tick.
#[test]
fn a_rejoin_waits_for_the_neutral_beat() {
    let mut w = World::new(Theme::Sewer, 41);
    open_arena(&mut w);
    let far = w.player.pos + Vec2::new(600.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, far, 5000.0);
    {
        let mut query = w.ecs.query::<&mut Boss>();
        let mut boss = query.iter_mut(&mut w.ecs).next().expect("boss alive");
        boss.set_state(BossState::Cat(BossCatState::Roar), 5.0);
        boss.lost = RELOCATE_AFTER_SECS;
    }
    update_boss(&mut w, DT);
    let boss = boss_of(&mut w).expect("boss alive");
    assert!(
        boss.pos.dist(w.player.pos) > REJOIN_DIST,
        "a roaring cat was relocated mid-roar to {:?}",
        boss.pos
    );
    assert!(boss.lost >= RELOCATE_AFTER_SECS, "the timer keeps running");
    {
        let mut query = w.ecs.query::<&mut Boss>();
        let mut boss = query.iter_mut(&mut w.ecs).next().expect("boss alive");
        boss.set_state(BossState::Cat(BossCatState::Stalk), 5.0);
    }
    update_boss(&mut w, DT);
    let boss = boss_of(&mut w).expect("boss alive");
    assert!(
        boss.pos.dist(w.player.pos) <= REJOIN_DIST,
        "the neutral cat was not walked back ({:?})",
        boss.pos
    );
}

#[test]
fn a_walled_off_boss_smashes_through_and_reaches_the_rat() {
    let mut w = walled_pocket_world();
    let start = Vec2::new(21.0 * 16.0, 11.0 * 16.0);
    spawn_boss_at(&mut w, BossKind::Cat, start, 400_000.0);

    // The real schedule refreshes the large-class flow field toward the rat
    // every tick; the boss's steering needs it (an empty field is not the
    // game).
    let mut broke = false;
    for _ in 0..(90.0 / DT) as usize {
        w.begin_tick(DT, Vec2::ZERO, false);
        let _ = w.ecs.run_system_cached(
            crate::game::ecs::systems::enemy_move::flow_refresh_system,
        );
        let _ = w.ecs.run_system_cached(boss_update_system);
        w.end_tick();
        // Any ring wall in the rat's row opening is the boss's hole (which
        // side it comes from depends on where the flow put it).
        if (30..36).any(|x| w.level.tiles.get(x, 11).is_open()) {
            broke = true;
        }
    }
    assert!(broke, "the boss never broke into the pocket");
    let boss = boss_of(&mut w).expect("boss alive");
    assert!(
        boss.pos.dist(w.player.pos) < 90.0,
        "the boss must walk in through the hole it made (boss {:?}, rat {:?})",
        boss.pos,
        w.player.pos
    );
}

// ---- the real schedule ----------------------------------------------------

#[test]
fn real_schedule_spawns_the_boss_at_boss_time() {
    let mut w = World::new(Theme::Sewer, 12);
    // The director raises `BossDue` when the run clock reaches `BOSS_TIME`;
    // jump the clock and let the real tick schedule run the hand-off.
    w.elapsed = crate::game::director::BOSS_TIME;
    w.update(DT, Vec2::ZERO, false);

    assert!(w.boss_time_reached());
    let boss = boss_of(&mut w).expect("the real schedule spawned the boss");
    assert_eq!(boss.kind, BossKind::pool_pick(Theme::Sewer, w.level.seed));
    let (hp, max_hp) = w.boss_hp().expect("HUD accessor reports the boss");
    assert!((hp - boss.hp).abs() < 1e-3);
    assert!((max_hp - boss.max_hp).abs() < 1e-3);
    assert!(w.boss_offset().is_some());
    assert!(w.boss_marker().is_some());

    // Keep the fight running for a second of real ticks: hazards, contact and
    // the controller must all resolve without panicking.
    for _ in 0..60 {
        w.update(DT, Vec2::ZERO, false);
    }
    assert!(boss_of(&mut w).is_some());
}

// ---- the mower's last gasp -----------------------------------------------

#[test]
fn the_mower_death_blast_lands_inside_the_art_radius() {
    use crate::game::level::Theme as BossTheme;

    // The Lab pools the Mower with the King Cobra: ask for a seed that fields
    // the mower, because the blast is the mower's own art.
    let seed = (0..8192u64)
        .find(|s| BossKind::for_theme_seeded(BossTheme::Lab, *s) == BossKind::Mower)
        .expect("some Lab seed fields the mower");
    let mut w = World::new(BossTheme::Lab, seed);
    w.level_index = 2;
    w.ecs.resource_mut::<BossDue>().due = true;
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(boss_spawn_system)
        .expect("spawn");
    w.end_tick();
    let boss = boss_of(&mut w).expect("the mower spawned");
    // The blast is the art's `18` inside `DEATH_BLAST_RADIUS` (`28 px`), and
    // only fires once the death state has had its `0.42` of telegraph.
    // Stand the rat so the blast covers it after the telegraph.
    w.player.pos = boss.pos + Vec2::new(6.0, 0.0);
    {
        let mut query = w.ecs.query::<&mut Boss>();
        for mut b in query.iter_mut(&mut w.ecs) {
            b.hp = 0.0;
        }
    }
    // Let the meltdown reach the detonation window.
    let detonate_at = (BossKind::Mower.death_duration() * 0.42 / DT).ceil() as usize;
    let mut drops = Vec::new();
    for i in 0..detonate_at + 60 {
        let hp = w.player.hp;
        // Re-park the rat inside the blast every tick: the knockback shoves it.
        if i + 6 > detonate_at {
            w.player.pos = boss.pos + Vec2::new(6.0, 0.0);
            w.player.iframes = 0.0;
        }
        update_boss(&mut w, DT);
        if w.player.hp < hp - 1e-3 {
            drops.push(hp - w.player.hp);
        }
    }
    assert!(
        drops.iter().any(|d| (d - 18.0).abs() < 1e-3),
        "the death blast must deal the art's 18 damage: {drops:?}"
    );
}

// ---- the landing ring is visible with the cat off screen -----------------

#[test]
fn the_landing_ring_is_drawn_with_the_cat_off_screen() {
    use crate::game::level::{Level, Theme as BossTheme};
    use crate::game::world::World as GameWorld;
    use crate::render::framebuffer::PixelBuffer;

    const VIEW: (usize, usize) = (160, 168);

    let render = |with_boss: bool| {
        let mut w = GameWorld::new(BossTheme::Sewer, 31);
        w.level = Level::empty(BossTheme::Sewer);
        w.player.pos = Vec2::new(1600.0, 1200.0);
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        if !with_boss {
            w.draw(&mut pb);
            return (pb, w.cam);
        }
        // Draw once to learn the camera, then park the cat just off screen with
        // its landing ring (a 96 px leap away) inside the view.
        w.draw(&mut pb);
        let cam = w.cam;
        let pos = Vec2::new(cam.x - 100.0, cam.y + 84.0);
        let target = pos + Vec2::new(art_boss_cat::POUNCE_MAX_DIST, 0.0);
        let mut boss = Boss::new(
            BossKind::Cat,
            pos,
            art_boss_cat::BOSS_BASE_HP,
            TileRect::new(0, 0, w.level.tiles.cols(), w.level.tiles.rows()),
            pos,
            false,
        );
        boss.facing = Vec2::new(1.0, 0.0);
        boss.set_state(BossState::Cat(BossCatState::Crouch), art_boss_cat::CROUCH_DURATION_P1);
        boss.elapsed = 0.4 * boss.duration;
        boss.target = target;
        w.ecs.spawn((boss, SpawnId(0)));
        let mut pb = PixelBuffer::new(VIEW.0, VIEW.1);
        w.draw(&mut pb);
        (pb, w.cam)
    };

    let (blank, _) = render(false);
    let (with_ring, cam) = render(true);
    // The cat body itself is outside the view (the 85 px margin test fails).
    let cat_screen = Vec2::new(cam.x - 100.0, cam.y + 84.0) - cam;
    assert!(
        cat_screen.x + BossKind::Cat.cull_margin() < 0.0,
        "test setup: the cat body must be off screen ({cat_screen:?})"
    );
    // ...yet the ring at the target is drawn.
    let target = cat_screen + Vec2::new(art_boss_cat::POUNCE_MAX_DIST, 0.0);
    let (tx, ty) = (target.x as i32, target.y as i32);
    let mut diff = 0;
    for y in (ty - 24).max(0)..(ty + 24).min(VIEW.1 as i32) {
        for x in (tx - 24).max(0)..(tx + 24).min(VIEW.0 as i32) {
            if blank.pixel_at(x as usize, y as usize) != with_ring.pixel_at(x as usize, y as usize) {
                diff += 1;
            }
        }
    }
    assert!(
        diff > 30,
        "the landing ring must be drawn with the cat off screen (only {diff} px differ)"
    );
}

// ---- player damage actually reaches the boss ------------------------------
//
// `boss_damage_system` is the single place player damage lands on the boss:
// bullets (aimed and pattern), acid pools, Tail Whip and mousetraps. Each test
// drives the real system(s) through the tick harness and reads the boss's HP,
// the same shape as the per-system tests in the other `systems/*` files.

/// Flat map (the generated walls would decide every sweep), no horde, no
/// leftover projectiles.
fn open_arena(w: &mut World) {
    w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    w.clear_enemies();
    w.clear_bullets();
    w.clear_pools();
}

/// The fight rectangle the boss's own arrival builds (batch 12 removed the
/// arena): the nominal fight size centred on the rat.
fn arena_rect(w: &World) -> TileRect {
    w.level.fight_rect_around(w.player.pos)
}

/// A live boss of `kind` at `pos` with a real `max_hp`.
fn spawn_boss_at(w: &mut World, kind: BossKind, pos: Vec2, hp: f32) {
    let arena = arena_rect(w);
    let boss = Boss::new(kind, pos, hp, arena, pos, false);
    w.ecs.spawn((boss, SpawnId(0)));
}

/// One tick of the projectile pass then the boss damage pass (the schedule's
/// order: bullets move, then `boss_damage_system` resolves the sweep).
fn tick_bullets_and_damage(w: &mut World) {
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(projectiles::bullets_system)
        .expect("bullets system");
    w.ecs
        .run_system_cached(boss_damage_system)
        .expect("boss damage system");
    w.end_tick();
}

/// One tick of the boss damage pass alone (pools, whip, traps).
fn tick_boss_damage(w: &mut World) {
    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(boss_damage_system)
        .expect("boss damage system");
    w.end_tick();
}

/// A player dart already inside the body, moving right at 480 px/s (8 px/tick).
fn dart_at(from: Vec2, damage: f32, weapon: WeaponKind) -> crate::game::bullet::Bullet {
    let mut b = crate::game::bullet::Bullet::new(
        from,
        Vec2::new(480.0, 0.0),
        damage,
        Faction::Player,
        (250, 250, 250),
    );
    b.weapon = Some(weapon);
    b
}

/// A player bullet is swept against the boss's grounded hit shapes, damages it
/// through `boss_damage_system`, and is consumed by the hit.
#[test]
fn player_bullets_damage_the_boss_and_are_consumed() {
    let mut w = World::new(Theme::Sewer, 31);
    open_arena(&mut w);
    let ppos = w.player.pos;
    let boss_pos = ppos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    // A fast dart: one tick crosses the whole chest circle, so only the swept
    // segment (not the endpoint) can register the hit.
    let start = boss_pos - Vec2::new(9.0, 0.0);
    let bullet = w.spawn_bullet(dart_at(start, 12.0, WeaponKind::Gnaw));
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_bullets_and_damage(&mut w);

    let hp_after = w.boss_hp().expect("boss alive").0;
    assert!(
        (hp_before - hp_after - 12.0).abs() < 1e-3,
        "the dart dealt {} damage, expected 12",
        hp_before - hp_after
    );
    let b = w.ecs.get::<crate::game::bullet::Bullet>(bullet).unwrap();
    assert!(
        b.life <= 0.0,
        "a non-piercing shot must be consumed by the boss"
    );
    // ...and the next projectile pass reaps it.
    tick_bullets_and_damage(&mut w);
    assert!(w.bullets().is_empty(), "the spent dart survived another pass");
    assert!(
        (w.boss_hp().expect("boss alive").0 - hp_after).abs() < 1e-3,
        "a spent dart hit the boss twice"
    );
}

/// Pierce behaves exactly as it does on enemies: one hit spent, the shot keeps
/// flying and cannot re-hit inside its re-hit cooldown.
#[test]
fn a_piercing_shot_spends_one_hit_on_the_boss() {
    let mut w = World::new(Theme::Sewer, 32);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    let mut b = dart_at(boss_pos - Vec2::new(9.0, 0.0), 9.0, WeaponKind::Gnaw);
    b.pierce = 2;
    let bullet = w.spawn_bullet(b);
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_bullets_and_damage(&mut w);

    let hp_after = w.boss_hp().expect("boss alive").0;
    assert!((hp_before - hp_after - 9.0).abs() < 1e-3);
    let b = w.ecs.get::<crate::game::bullet::Bullet>(bullet).unwrap();
    assert_eq!(b.pierce, 1, "a pierce charge was not spent");
    assert!(b.hit_cd > 0.0, "the re-hit cooldown was not set");
    assert!(b.life > 0.0, "a piercing shot must survive the boss");
}

/// A grounded boss is hit by the pattern weapons too: Squeak Nova's ring and
/// Spore Orbit's spore.
#[test]
fn squeak_nova_and_spore_orbit_damage_the_boss() {
    // Squeak Nova: a radial dart crossing the body.
    let mut w = World::new(Theme::Sewer, 33);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    w.spawn_bullet(dart_at(boss_pos - Vec2::new(9.0, 0.0), 7.0, WeaponKind::SqueakNova));
    let hp_before = w.boss_hp().expect("boss alive").0;
    tick_bullets_and_damage(&mut w);
    assert!(
        (hp_before - w.boss_hp().expect("boss alive").0 - 7.0).abs() < 1e-3,
        "Squeak Nova did not damage the boss"
    );
    // The tally books it as pattern damage, not as an aimed hit.
    let dmg = w.ecs.resource::<BossFight>().scratch.dmg;
    assert!((dmg.pattern - 7.0).abs() < 1e-3, "pattern tally {dmg:?}");
    assert!(dmg.aimed.abs() < 1e-6, "a radial dart counted as aimed");

    // Spore Orbit: the spore sits on its ring, 24 px from the rat, and the body
    // is 30 px out, so the two overlap on the ring.
    let mut w = World::new(Theme::Sewer, 34);
    open_arena(&mut w);
    let ppos = w.player.pos;
    spawn_boss_at(&mut w, BossKind::Cat, ppos + Vec2::new(30.0, 0.0), 5000.0);
    let mut b = crate::game::bullet::Bullet::new(
        ppos + Vec2::new(24.0, 0.0),
        Vec2::ZERO,
        5.0,
        Faction::Player,
        (250, 250, 250),
    );
    b.pierce = 999;
    b.radius = 3.0;
    b.weapon = Some(WeaponKind::SporeOrbit);
    b.orbit = Some(crate::game::bullet::Orbit {
        angle: 0.0,
        radius: 24.0,
        speed: 0.0,
    });
    w.spawn_bullet(b);
    let hp_before = w.boss_hp().expect("boss alive").0;
    tick_bullets_and_damage(&mut w);
    assert!(
        (hp_before - w.boss_hp().expect("boss alive").0 - 5.0).abs() < 1e-3,
        "a spore on its ring did not damage the boss"
    );
}

/// Tail Whip's arc reaches the boss at the same radius it reaches enemies.
#[test]
fn tail_whip_damages_the_boss() {
    use crate::game::weapon::Weapon;

    let mut w = World::new(Theme::Sewer, 35);
    open_arena(&mut w);
    // Level 8: `fire_one`'s aura is 26 + 3 * 8 = 50 px, so a body at 45 px is
    // inside the whip (and outside the old 30 px melee reach).
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::TailWhip,
        level: 8,
        timer: 0.0,
        evolved: false,
    }];
    let ppos = w.player.pos;
    spawn_boss_at(&mut w, BossKind::Cat, ppos + Vec2::new(45.0, 0.0), 5000.0);
    let expected = w.player.loadout.weapons[0].damage();
    let hp_before = w.boss_hp().expect("boss alive").0;

    w.begin_tick(DT, Vec2::ZERO, false);
    w.ecs
        .run_system_cached(super::super::weapons::fire_weapons_system)
        .expect("weapons system");
    w.ecs
        .run_system_cached(boss_damage_system)
        .expect("boss damage system");
    w.end_tick();

    let dealt = hp_before - w.boss_hp().expect("boss alive").0;
    assert!(
        (dealt - expected).abs() < 1e-3,
        "Tail Whip dealt {dealt}, expected {expected}"
    );
}

/// A landed acid pool ticks damage into the boss while it stands in it.
#[test]
fn acid_pools_damage_the_boss() {
    let mut w = World::new(Theme::Sewer, 36);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    w.spawn_pool(Pool {
        pos: boss_pos,
        radius: 12.0,
        dps: 240.0,
        life: 3.0,
        color: (140, 220, 70),
    });
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_boss_damage(&mut w);

    let dealt = hp_before - w.boss_hp().expect("boss alive").0;
    assert!(
        (dealt - 240.0 * DT).abs() < 1e-3,
        "the pool dealt {dealt} in one tick, expected {}",
        240.0 * DT
    );
}

/// A snapping mousetrap under the boss damages it once per snap, not once per
/// tick of the snap.
#[test]
fn mousetraps_damage_the_boss_once_per_snap() {
    let mut w = World::new(Theme::Sewer, 37);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    w.level.objects.mousetraps.push(MousetrapView {
        pos: boss_pos,
        facing: Vec2::new(1.0, 0.0),
        state: MousetrapState::Snapping,
        // Inside the art's `MOUSETRAP_HIT_PROGRESS_MIN..=MAX` window.
        progress: 0.5,
        anim: 0.0,
        hit_flash: 0.0,
        ..MousetrapView::new(boss_pos, Vec2::new(1.0, 0.0))
    });
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_boss_damage(&mut w);
    let first = hp_before - w.boss_hp().expect("boss alive").0;
    assert!(
        (first - MOUSETRAP_DAMAGE_ENEMY).abs() < 1e-3,
        "the snap dealt {first}, expected {MOUSETRAP_DAMAGE_ENEMY}"
    );
    tick_boss_damage(&mut w);
    let second = hp_before - w.boss_hp().expect("boss alive").0;
    assert!(
        (second - first).abs() < 1e-3,
        "the same snap hit the boss twice ({first} then {second})"
    );
}

/// The leaping cat has no grounded shape, so a dart sweeps underneath it: no
/// damage, and the shot is not consumed by a body it cannot touch.
#[test]
fn a_leaping_cat_takes_no_ground_damage() {
    let mut w = World::new(Theme::Sewer, 38);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    {
        let mut query = w.ecs.query::<&mut Boss>();
        let mut boss = query.iter_mut(&mut w.ecs).next().expect("boss alive");
        boss.set_state(BossState::Cat(BossCatState::Pounce), 0.70);
        boss.elapsed = 0.35;
        assert!(boss.is_airborne());
    }
    let bullet = w.spawn_bullet(dart_at(boss_pos - Vec2::new(9.0, 0.0), 12.0, WeaponKind::Gnaw));
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_bullets_and_damage(&mut w);

    assert!(
        (w.boss_hp().expect("boss alive").0 - hp_before).abs() < 1e-3,
        "a leaping cat took ground damage"
    );
    assert!(
        w.ecs.get::<crate::game::bullet::Bullet>(bullet).is_some(),
        "the shot was consumed by a body it passed under"
    );
}

/// A mousetrap is on the floor: like a dart or a pool it cannot bite a jaguar
/// in mid-leap (the trap pass used to test every hit shape, grounded or not,
/// so a snap under the arc of a pounce cost the cat a full trap hit).
#[test]
fn a_leaping_cat_is_not_bitten_by_a_mousetrap() {
    let mut w = World::new(Theme::Sewer, 39);
    open_arena(&mut w);
    let boss_pos = w.player.pos + Vec2::new(40.0, 0.0);
    spawn_boss_at(&mut w, BossKind::Cat, boss_pos, 5000.0);
    {
        let mut query = w.ecs.query::<&mut Boss>();
        let mut boss = query.iter_mut(&mut w.ecs).next().expect("boss alive");
        boss.set_state(BossState::Cat(BossCatState::Pounce), 0.70);
        boss.elapsed = 0.35;
        assert!(boss.is_airborne());
    }
    w.level.objects.mousetraps.push(MousetrapView {
        pos: boss_pos,
        facing: Vec2::new(1.0, 0.0),
        state: MousetrapState::Snapping,
        progress: 0.5,
        anim: 0.0,
        hit_flash: 0.0,
        ..MousetrapView::new(boss_pos, Vec2::new(1.0, 0.0))
    });
    let hp_before = w.boss_hp().expect("boss alive").0;

    tick_boss_damage(&mut w);

    assert!(
        (w.boss_hp().expect("boss alive").0 - hp_before).abs() < 1e-3,
        "a floor trap bit a cat that was in the air"
    );
}

/// Same seed, same run, with a boss on the field and the weapons firing at it:
/// the targeting, the damage pass and the boss's own rng must all be
/// reproducible.
#[test]
fn twin_runs_with_a_boss_and_weapons_match() {
    let run = |kind: BossKind, seed: u64| -> Vec<u32> {
        let mut w = World::new(Theme::Sewer, seed);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.clear_enemies();
        w.clear_bullets();
        let arena = arena_rect(&w);
        let centre = arena.center_world();
        let mut boss = Boss::new(kind, centre, 20_000.0, arena, centre, false);
        boss.id = 1;
        w.ecs.spawn((boss, SpawnId(0)));
        // In weapons range, so the aimed families have to choose between the
        // boss points and the horde the director keeps spawning.
        w.player.pos = centre + Vec2::new(40.0, 0.0);
        let mut trace = Vec::with_capacity(240 * 10);
        for k in 0..240 {
            let dir = Vec2::new((k as f32 * 0.13).sin(), (k as f32 * 0.09).cos());
            w.update(DT, dir, k % 70 == 0);
            let (hp, _) = w.boss_hp().expect("boss alive");
            let boss = boss_of(&mut w).expect("boss alive");
            trace.push(hp.to_bits());
            trace.push(boss.pos.x.to_bits());
            trace.push(boss.pos.y.to_bits());
            // `duration` and `flags` are state-local, so they pin the state
            // machine without needing a `State -> u32` mapping.
            trace.push(boss.duration.to_bits());
            trace.push(boss.flags);
            trace.push((boss.phase as u32) << 8 | (boss.elapsed.to_bits() & 0xff));
            trace.push(w.player.hp.to_bits());
            trace.push(w.enemy_count() as u32);
            trace.push(w.player.level);
            trace.push(w.kills);
        }
        trace
    };
    for kind in [BossKind::Cat, BossKind::Broom, BossKind::Mower] {
        assert_eq!(run(kind, 4242), run(kind, 4242), "{kind:?} twin run diverged");
    }
}

// ---- campaign DPS measurement (balance; gated, slow) ---------------------
//
// `RATRUN_BOSS_DPS=1 cargo test boss::tests::measure_campaign_boss_dps -- --nocapture`
//
// Plays the campaign with the App's level-up behaviour (auto-pick the first
// card) and a flee script, then measures the real damage the weapons deal to
// the boss-sized target at the boss time on each level. The output feeds the
// per-boss HP multipliers ([`BossKind::hp_scale`]).

#[test]
fn measure_campaign_boss_dps() {
    use crate::game::campaign;
    use crate::game::loadout::Loadout;

    if std::env::var("RATRUN_BOSS_DPS").is_err() {
        return;
    }
    const SEEDS: [u64; 4] = [4242, 7, 99, 1234];
    // A warm-up while the weapons' target pick settles onto the boss and the
    // rat finds its range, then a full minute of measurement.
    const WARMUP_SECS: f32 = 15.0;
    const MEASURE_SECS: f32 = 60.0;

    /// Flee the local crowd, with a deterministic wander bias (the harness's
    /// `Evade` script), and dash on a fixed cadence.
    fn script(w: &mut World, tick: u64) -> (Vec2, bool) {
        let t = tick as f32;
        let phase = t * 0.013;
        let bias = Vec2::new(phase.sin(), (phase * 0.7).cos());
        let p = w.player.pos;
        let enemies = w.enemies();
        let (mut fx, mut fy) = (0.0f32, 0.0f32);
        let mut nearest = f32::MAX;
        for e in &enemies {
            let dx = p.x - e.pos.x;
            let dy = p.y - e.pos.y;
            let d2 = dx * dx + dy * dy;
            nearest = nearest.min(d2);
            if d2 < 12100.0 {
                let d = d2.sqrt();
                let wgt = 1.4 / (d + 6.0);
                fx += dx * wgt;
                fy += dy * wgt;
            }
        }
        let nearest = nearest.sqrt();
        let len = (fx * fx + fy * fy).sqrt();
        let flee = if len > 1e-4 {
            Vec2::new(fx / len, fy / len)
        } else {
            bias
        };
        // Kite around the arena centre on top of the flee vector: running a
        // wide circle is how a survivor keeps the horde behind it.
        let centre = w.level.arena * 0.5;
        let outward = p - centre;
        let tangent = if outward.len_sq() > 1.0 {
            outward.normalized().perp()
        } else {
            bias
        };
        let dir = Vec2::new(
            flee.x * 0.65 + tangent.x * 0.30 + bias.x * 0.05,
            flee.y * 0.65 + tangent.y * 0.30 + bias.y * 0.05,
        );
        (dir, nearest < 26.0)
    }

    fn resolve_levelups(w: &mut World) {
        let mut guard = 0;
        while w.pending_levelups > 0 && !w.finished() && guard < 64 {
            guard += 1;
            let choices = w.player.loadout.generate_choices(&mut w.rng);
            let Some(up) = choices.into_iter().next() else {
                break;
            };
            let heal = w.player.loadout.apply(&up);
            w.player.heal(heal);
            w.pending_levelups = w.pending_levelups.saturating_sub(1);
        }
    }

    let mut totals = [0.0f32; 3];
    let mut samples: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for seed in SEEDS {
        let mut carried = Loadout::starting();
        for (level, total) in totals.iter_mut().enumerate() {
            let mut w = World::new_level(level, campaign::level_seed(seed, level), carried.clone());
            let mut tick = 0u64;
            let mut deaths = 0usize;
            // Build phase: play to the boss time exactly as the App would. The
            // rat is topped up when hurt (the measurement is about the weapons'
            // damage, not about surviving the horde), while the script keeps
            // kiting so the fire patterns and targeting stay realistic.
            while w.elapsed < crate::game::director::BOSS_TIME && !w.finished() {
                let (dir, dash) = script(&mut w, tick);
                w.update(DT, dir, dash);
                resolve_levelups(&mut w);
                if w.player.hp < 55.0 {
                    deaths += 1;
                    let max = w.player.max_hp();
                    w.player.heal(max);
                }
                tick += 1;
            }
            let _ = deaths;
            assert!(
                !w.finished(),
                "seed {seed} level {level}: the scripted rat died at {:.1} s",
                w.elapsed
            );
            assert!(
                w.boss_hp().is_some(),
                "the boss spawned at the boss time"
            );
            // Make it unkillable so the fixed window measures a damage *rate*
            // rather than a time-to-kill, and hold it at `hp_frac == 1` so
            // every seed measures the same phase (phase 2 would speed the boss
            // up and change how much of the window it spends in range). The
            // per-controller `damage_mult` is left alone: the mower's overheat
            // window is part of what a real build gets to shoot into.
            // `hp = 1e6` with the **real** `max_hp`: `hp_frac` clamps to 1 so
            // the boss stays in phase 1, every absolute-damage threshold in the
            // controllers (the cat's burst, the dodge hits) still reads the
            // true per-tick number, and the f32 ulp at 1e6 is 0.06 so the
            // measured loss is not quantised (at 1e9 the ulp is 64 and a 20
            // damage tick rounds away — the first version of this measurement
            // read 4x low). The broom's relative poise stagger needs a burst of
            // 18% of the pool in one beat; no build reaches that with a
            // 100k pool, so it is not distorted by this either.
            {
                let mut query = w.ecs.query::<&mut Boss>();
                for mut b in query.iter_mut(&mut w.ecs) {
                    b.hp = 1.0e6;
                    b.phase = 1;
                    b.invuln = 0.0;
                    b.damage_mult = 1.0;
                }
            }
            // The fight starts where the boss arrived (batch 12 removed the
            // arena, so there is no walk to measure): put the rat beside it.
            if let Some(pos) = w.boss_marker() {
                if let Some(open) = w
                    .level
                    .tiles
                    .nearest_open_center(pos, crate::game::collision::SizeClass::Small, 16)
                {
                    w.player.pos = open;
                }
            }
            // The real boss is the target: `weapons::nearest_visible` takes
            // `Boss::target_points` as ordinary bodies in the aim queue, so
            // aimed bullets land on its grounded hit shapes and
            // `boss_damage_system` books their damage (and every incidental
            // source) per source. Nothing has to be pinned to a punch-bag: the
            // boss is both the thing that gets shot and the thing measured.
            let mut tally = BossDamageTally::default();
            let measure_ticks = ((WARMUP_SECS + MEASURE_SECS) / DT) as usize;
            for k in 0..measure_ticks {
                let before = 1.0e6 - w.boss_hp().map(|(h, _)| h).unwrap_or(1.0e6);
                if k % 600 == 0 {
                    println!(
                        "   t={:.0}s boss_hp_lost={:.0} dist={:?} elapsed={:.1} finished={} enemies={} aimed={:.0} incident={:.0}",
                        k as f32 * DT,
                        before,
                        w.boss_marker().map(|b| b.dist(w.player.pos)),
                        w.elapsed,
                        w.finished(),
                        w.enemy_count(),
                        tally.aimed,
                        tally.incidental()
                    );
                }
                // The boss fight: close to fighting range and circle it, so the
                // weapons actually fire at the boss-sized target (with the
                // horde still in the way, exactly as in a real fight).
                let (flee, dash) = script(&mut w, tick);
                let dir = if let Some(boss_pos) = w.boss_marker() {
                    let to = boss_pos - w.player.pos;
                    let dist = to.len();
                    if dist > 120.0 {
                        // Long trek across the level: follow the bearing.
                        to.normalized()
                    } else if dist > 34.0 {
                        (to.normalized() * 0.85 + flee * 0.15).normalized()
                    } else {
                        (-to.normalized() * 0.6 + to.normalized().perp() * 0.4)
                            .normalized()
                    }
                } else {
                    flee
                };
                w.update(DT, dir, dash);
                resolve_levelups(&mut w);
                tick += 1;
                // This tick's attributed damage, straight from the boss damage
                // system (reset per tick; it never feeds the simulation).
                let dmg = w.ecs.resource::<BossFight>().scratch.dmg;
                if k as f32 * DT >= WARMUP_SECS {
                    tally.add(dmg);
                }
                if w.player.hp < 55.0 {
                    let max = w.player.max_hp();
                    w.player.heal(max);
                }
                if w.finished() {
                    break;
                }
            }
            let dps = tally.total() / MEASURE_SECS;
            let d = |v: f32| v / MEASURE_SECS;
            samples[level].push(dps);
            *total += dps;
            let (weapons, passives) = (
                w.player.loadout.weapons.len(),
                w.player.loadout.passives.len(),
            );
            println!(
                "seed {seed} level {level}: dps {dps:.1} (aimed {:.1}, pattern {:.1}, pools {:.1}, whip {:.1}, traps {:.1}; incidental {:.1})  (weapons {weapons}, passives {passives}, lvl {})",
                d(tally.aimed),
                d(tally.pattern),
                d(tally.pools),
                d(tally.whip),
                d(tally.traps),
                d(tally.incidental()),
                w.player.level
            );
            // The campaign carry: the player's first three keepable upgrades.
            let keeps = campaign::keepables(&w.player.loadout);
            let chosen: Vec<_> = keeps.into_iter().take(campaign::KEEP_COUNT).collect();
            carried = campaign::next_loadout(&w.player.loadout, &chosen)
                .expect("a keep-three selection is valid");
        }
    }
    println!(
        "mean dps: level 0 {:.1}, level 1 {:.1}, level 2 {:.1}",
        totals[0] / SEEDS.len() as f32,
        totals[1] / SEEDS.len() as f32,
        totals[2] / SEEDS.len() as f32
    );
    // Median (the balance target) and the spread (how badly a seed can swing),
    // plus the `hp_scale` each target fight length implies.
    for (level, sample) in samples.iter().enumerate() {
        let mut v = sample.clone();
        v.sort_by(f32::total_cmp);
        let median = (v[1] + v[2]) * 0.5;
        let kind = BossKind::for_level(level);
        let pool = kind.base_hp() * director::boss_hp_mult_for(level, director::BOSS_TIME);
        println!(
            "level {level}: median dps {median:.1}  spread {:.1}..{:.1}  pool {pool:.0} hp  \
             scale@60s {:.2}@75s {:.2}@90s {:.2}  current {:.2} -> {:.0}s",
            v[0],
            v[v.len() - 1],
            median * 60.0 / pool,
            median * 75.0 / pool,
            median * 90.0 / pool,
            kind.hp_scale(),
            pool * kind.hp_scale() / median
        );
        if v[0] > 0.0 {
            println!(
                "   at that scale: slowest seed {:.0}s, fastest seed {:.0}s",
                pool * kind.hp_scale() / v[0],
                pool * kind.hp_scale() / v[v.len() - 1]
            );
        }
    }
}

// ============================================================================
// The control budget and the pursuit rule (batch 13)
//
// The owner's report: "with certain weapons, apex jaguar is never able to
// attack". Two mechanisms did it, both diagnosed in `cat.rs`/`mod.rs`:
//
// * the cat's `Hurt` beat was re-tripped by any 55-damage tick, and a max-rank
//   Spiked Tail (44 lash + 18 burst on one tick, every 0.85 s), one Tail Thrash
//   sweep (52 at rank 5, three a second) and one Laser sweep (5 x 10 in a tick)
//   all clear it — so every `SwipeWindup` (0.55 s) and every `Crouch` (0.95 s)
//   was cancelled before it fired;
// * the cat stalks at 24 px/s while the rat runs 58-114 px/s and lashes from
//   80-120 px, so a rat holding 100 px was outside the pounce band (45..95) for
//   the whole fight.
//
// These tests are the lock: every boss starts and fires attacks under every
// weapon script, and the stagger a boss can be given never exceeds the budget.
// ============================================================================

use crate::game::nav::NavBudget;
use crate::game::weapon::Weapon;

/// The rat's character level in these fights (the level a campaign reaches at
/// the boss): `Player::damage_mult` is 1.36 there.
const RAT_LEVEL: u32 = 10;
/// The rat's speed cap while kiting (a build with MoveSpeed stacks).
const RAT_SPEED: f32 = 110.0;

/// One weapon, replayed the way the weapon systems land it on a boss.
///
/// The real weapon systems need a loadout, an `ActiveFx` and a rat that presses
/// a key; what they actually do to a boss is `Boss::take_damage` once per hit at
/// the weapon's own table number, plus the Sonic Squeak's `slow_for` and the few
/// pixels of `Boss::shove` the squeak and the thrash ask for. This replays that
/// with the real tables, so the cadence in the test is the cadence in the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    /// Sonic Squeak at rank 5, pressed on cooldown.
    Squeak,
    /// Tail Thrash at rank 5, on cooldown: three sweeps a third of a second
    /// apart, then its charge.
    Thrash,
    /// Spiked Tail at rank 5, kited at 100 px: the lash plus the burst it lands
    /// on the same tick.
    Lash,
    /// Laser Eyes at rank 5, kited at 70 px: one sweep's single hit per cycle.
    Laser,
    /// Tail Whip at rank 5 behind Counterweight x6, at hugging range.
    Whip6,
}

impl Script {
    fn all() -> [Script; 5] {
        [
            Script::Squeak,
            Script::Thrash,
            Script::Lash,
            Script::Laser,
            Script::Whip6,
        ]
    }

    /// The radius the rat holds this weapon's range at (px from the boss).
    fn ring(self) -> f32 {
        match self {
            Script::Squeak => 60.0,
            Script::Thrash => 50.0,
            // The lash reaches 120 px: this is the kite that beat the jaguar.
            Script::Lash => 100.0,
            // The beam reaches 54-86 px.
            Script::Laser => 70.0,
            // A whip hit needs the rat inside 42 px.
            Script::Whip6 => 34.0,
        }
    }

    /// Every hit this weapon lands in `secs`, as
    /// `(time, damage, shove px, seconds of movement slow)`.
    fn events(self, secs: f32) -> Vec<(f32, f32, f32, f32)> {
        let kind = match self {
            Script::Squeak => WeaponKind::SonicSqueak,
            Script::Thrash => WeaponKind::TailThrash,
            Script::Lash => WeaponKind::SpikedTail,
            Script::Laser => WeaponKind::LaserEyes,
            Script::Whip6 => WeaponKind::TailWhip,
        };
        let mut weapon = Weapon::new(kind);
        weapon.level = kind.max_level();
        let mult = 1.0 + 0.04 * (RAT_LEVEL.saturating_sub(1)) as f32;
        let mut out = Vec::new();
        match self {
            Script::Squeak => {
                // One cone a press: the damage, then the slow (applied by the
                // caller for the first 0.8 s of each press).
                let mut t = 0.0;
                while t < secs {
                    out.push((
                        t,
                        weapon.damage() * mult,
                        0.0,
                        crate::game::weapon::SQUEAK_BOSS_SLOW_SECS,
                    ));
                    t += weapon.cooldown();
                }
            }
            Script::Thrash => {
                // Three sweeps inside `THRASH_BURST`, then the charge time.
                let sweep = crate::game::weapon::THRASH_BURST
                    / crate::game::weapon::THRASH_SWEEPS as f32;
                let mut burst = 0.5;
                while burst < secs {
                    for k in 0..crate::game::weapon::THRASH_SWEEPS {
                        let t = burst + k as f32 * sweep;
                        if t < secs {
                            out.push((t, weapon.damage() * mult, 0.0, 0.0));
                        }
                    }
                    burst += weapon.cooldown();
                }
            }
            Script::Lash => {
                // The lash and the burst it lands on the same tick, as
                // `weapon_spiketail::land` books them.
                let hit = weapon.damage() * (1.0 + crate::game::weapon::LASH_BURST_FRAC) * mult;
                let mut t = 0.0;
                while t < secs {
                    out.push((t, hit, 0.0, 0.0));
                    t += weapon.cooldown();
                }
            }
            Script::Laser => {
                // One sweep cuts a body once for one old pulse's damage.
                let hit = weapon.laser_hit_damage() * mult;
                let mut t = 0.0;
                while t < secs {
                    out.push((t, hit, 0.0, 0.0));
                    t += weapon.cooldown();
                }
            }
            Script::Whip6 => {
                // Counterweight x6 is `Stats::shove_px` 12 px a whip hit.
                let mut t = 0.0;
                while t < secs {
                    out.push((t, weapon.damage() * mult, 12.0, 0.0));
                    t += weapon.cooldown();
                }
            }
        }
        out
    }
}

/// The floor a kind is fought on (the controllers read `level.theme`).
fn theme_for_boss(kind: BossKind) -> Theme {
    match kind {
        BossKind::Cat | BossKind::HorseshoeCrab => Theme::Sewer,
        BossKind::Broom | BossKind::MegaRat => Theme::Kitchen,
        BossKind::Mower | BossKind::Cobra => Theme::Lab,
        BossKind::Mangle => Theme::Laundry,
        BossKind::NightEdition => Theme::Printworks,
        BossKind::Magpie | BossKind::Hawk => Theme::Trailer,
    }
}

/// What the scripted fight measured.
#[derive(Default, Clone, Copy, Debug)]
struct Tally {
    /// Attack beats entered (a committed state after a non-committed one).
    started: u32,
    /// Attack beats that fired — a hazard went active or a projectile flew.
    fired: u32,
    /// Seconds the boss spent in its own stagger pose.
    controlled: f32,
}

/// One scripted fight: an open floor, one boss, and a rat that kites.
struct ScriptedFight {
    w: World,
}

impl ScriptedFight {
    fn new(kind: BossKind, seed: u64) -> Self {
        let mut w = World::new(theme_for_boss(kind), seed);
        open_arena(&mut w);
        w.flow = crate::game::nav::FlowField::new(&w.level.tiles);
        // The rat cannot die: the point of the test is the boss's beats, and a
        // rat death would end the fight early.
        w.player.loadout.stats.max_hp = 1.0e6;
        w.player.hp = 1.0e6;
        w.player.level = RAT_LEVEL;
        let room = Vec2::new(800.0, 800.0);
        w.player.pos = room + Vec2::new(100.0, 0.0);
        // A pool nothing in 30 s can empty, so the fight stays in phase 1.
        spawn_boss_at(&mut w, kind, room, 1.0e6);
        Self { w }
    }

    fn boss_pos(&mut self) -> Vec2 {
        boss_of(&mut self.w).expect("the boss survives").pos
    }

    /// Move the rat one tick: orbit the boss and hold `ring` px from it, at (at
    /// most) [`RAT_SPEED`]. A real kiter, not a teleport: the boss can close
    /// when it is faster, and the rat can run back out when it is not.
    fn kite(&mut self, ring: f32, bearing: f32) {
        let boss = self.boss_pos();
        let offset = self.w.player.pos - boss;
        let want = if offset.len_sq() > 1e-6 {
            offset.normalized() * ring
        } else {
            Vec2::new(ring, 0.0)
        };
        let tangent = Vec2::from_angle(bearing) * RAT_SPEED * 0.35;
        let mut vel = tangent + (want - offset) * 4.0;
        let speed = vel.len();
        if speed > RAT_SPEED {
            vel = vel * (RAT_SPEED / speed);
        }
        self.w.player.pos += vel * DT;
        self.w.player.vel = vel;
        self.w.player.hp = self.w.player.loadout.stats.max_hp;
    }

    /// Run `secs` of fight with `scripts` firing, and measure the boss's beats.
    fn run(&mut self, secs: f32, scripts: &[Script]) -> Tally {
        let mut events: Vec<(f32, f32, f32, f32)> = Vec::new();
        for script in scripts {
            events.extend(script.events(secs));
        }
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        let rings: Vec<f32> = scripts.iter().map(|s| s.ring()).collect();
        let ring = rings.iter().copied().fold(0.0f32, f32::max);

        let mut tally = Tally::default();
        let mut cursor = 0usize;
        let ticks = (secs / DT).round() as usize;
        let mut prev_committed = false;
        let mut beat_fired = false;
        for i in 0..ticks {
            let t = i as f32 * DT;
            self.kite(ring, t * 0.4);
            // The large-class flow field is the schedule's own refresh; in an
            // open room it is only the fallback path, so a coarse cadence is
            // plenty and keeps the test quick.
            if i % 20 == 0 {
                let goal = self.w.player.pos;
                self.w.flow.update(&self.w.level.tiles, goal, NavBudget::FULL);
            }
            while cursor < events.len() && events[cursor].0 <= t {
                let (_, dmg, shove, slow) = events[cursor];
                let dir = self.boss_pos() - self.w.player.pos;
                // The one path every weapon uses on a boss.
                {
                    let level = &self.w.level;
                    let mut query = self.w.ecs.query::<&mut Boss>();
                    if let Some(mut boss) = query.iter_mut(&mut self.w.ecs).next() {
                        let amount = dmg * boss.damage_mult;
                        boss.take_damage(amount);
                        if slow > 0.0 {
                            boss.slow_for(crate::game::weapon::SQUEAK_BOSS_SLOW, slow);
                        }
                        if shove > 0.0 {
                            let _ = boss.shove(level, dir, shove);
                        }
                    }
                }
                cursor += 1;
            }
            update_boss(&mut self.w, DT);

            let boss = boss_of(&mut self.w).expect("the boss survives 30 s");
            let committed = boss.committed();
            if boss.is_stagger_state() {
                tally.controlled += DT;
            }
            let mut firing = boss.hazards.iter().any(|h| h.is_active());
            if !firing {
                let mut query = self.w.ecs.query::<&BossProjectile>();
                firing = query.iter(&self.w.ecs).next().is_some();
            }
            if committed && !prev_committed {
                tally.started += 1;
                beat_fired = false;
            }
            if committed && firing {
                beat_fired = true;
            }
            if !committed && prev_committed && beat_fired {
                tally.fired += 1;
            }
            prev_committed = committed;
        }
        tally
    }
}

/// The jaguar, under every weapon script, in a 30 s fight.
#[test]
fn the_jaguar_attacks_under_every_weapon_script() {
    for (i, script) in Script::all().iter().enumerate() {
        let mut fight = ScriptedFight::new(BossKind::Cat, 700 + i as u64);
        let tally = fight.run(30.0, &[ *script ]);
        assert!(
            tally.started >= 3,
            "the jaguar started only {} attacks under {script:?} in 30 s",
            tally.started
        );
        assert!(
            tally.fired >= 3,
            "the jaguar completed only {} attacks under {script:?} in 30 s",
            tally.fired
        );
        // The whole report in one number: a stun-locked cat is controlled for
        // most of the fight.
        assert!(
            tally.controlled <= CONTROL_BUDGET_SECS * (30.0 / CONTROL_WINDOW_SECS + 1.0) + DT,
            "a {script:?} build held the jaguar under control for {} s of 30",
            tally.controlled
        );
    }
}

/// Three more kinds (a melee walker, a charger and the Lab's striker) under the
/// same scripts: the budget is shared machinery, not a cat special case.
#[test]
fn other_bosses_attack_under_every_weapon_script() {
    for (k, kind) in [BossKind::Broom, BossKind::Mower, BossKind::Cobra]
        .iter()
        .copied()
        .enumerate()
    {
        for (i, script) in Script::all().iter().enumerate() {
            let mut fight = ScriptedFight::new(kind, 900 + (k * 16 + i) as u64);
            let tally = fight.run(30.0, &[*script]);
            assert!(
                tally.started >= 3,
                "{kind:?} started only {} attacks under {script:?} in 30 s",
                tally.started
            );
            assert!(
                tally.fired >= 3,
                "{kind:?} completed only {} attacks under {script:?} in 30 s",
                tally.fired
            );
            assert!(
                tally.controlled <= CONTROL_BUDGET_SECS * (30.0 / CONTROL_WINDOW_SECS + 1.0) + DT,
                "{kind:?} was held under control for {} s of 30 by {script:?}",
                tally.controlled
            );
        }
    }
}

/// **All ten kinds**, with the two weapons that were locking the cat firing
/// together: a boss may be staggered, never taken out of the fight.
#[test]
fn every_boss_starts_and_fires_attacks_under_the_heaviest_build() {
    for (i, kind) in BossKind::all().iter().copied().enumerate() {
        let mut fight = ScriptedFight::new(kind, 1300 + i as u64);
        let tally = fight.run(30.0, &[Script::Lash, Script::Thrash, Script::Laser]);
        assert!(
            tally.started >= 3,
            "{kind:?} started only {} attacks under the heaviest build in 30 s",
            tally.started
        );
        assert!(
            tally.fired >= 3,
            "{kind:?} completed only {} attacks under the heaviest build in 30 s",
            tally.fired
        );
        assert!(
            tally.controlled <= CONTROL_BUDGET_SECS * (30.0 / CONTROL_WINDOW_SECS + 1.0) + DT,
            "{kind:?} was held under control for {} s of 30",
            tally.controlled
        );
    }
}

/// The budget itself, on the API every controller uses.
#[test]
fn grant_control_caps_the_window_and_refuses_committed_beats() {
    let arena = TileRect::new(0, 0, 25, 19);
    let level = crate::game::level::Level::empty(Theme::Sewer);
    let mut boss = Boss::new(BossKind::Cat, Vec2::new(90.0, 74.0), 900.0, arena, Vec2::ZERO, false);

    // Neutral: the art's `HURT_DURATION` (0.45 s) is clipped to the budget.
    let granted = boss.grant_control(art_boss_cat::HURT_DURATION);
    assert!(
        (granted - CONTROL_BUDGET_SECS).abs() < 1e-5,
        "a fresh window pays the budget ({granted})"
    );
    // ...and the same window pays nothing more.
    assert_eq!(boss.grant_control(0.45), 0.0, "the window is spent");

    // A committed beat is never granted, even with a fresh window.
    boss.control_window = 0.0;
    boss.control_used = 0.0;
    boss.set_state(
        BossState::Cat(BossCatState::SwipeWindup),
        art_boss_cat::SWIPE_WINDUP_P1,
    );
    assert!(boss.committed(), "a wind-up is a committed beat");
    assert_eq!(boss.grant_control(0.45), 0.0, "a wind-up is never staggered");
    assert_eq!(
        boss.shove(&level, Vec2::new(1.0, 0.0), 40.0),
        0.0,
        "a wind-up is never shoved"
    );

    // A neutral boss takes at most `SHOVE_PER_HIT_PX` a hit and
    // `SHOVE_PER_SEC_PX` a second.
    boss.set_state(
        BossState::Cat(BossCatState::Stalk),
        BossKind::Cat.initial_duration(),
    );
    assert!(!boss.committed(), "Stalk is the neutral beat");
    let first = boss.shove(&level, Vec2::new(1.0, 0.0), 40.0);
    assert!((first - SHOVE_PER_HIT_PX).abs() < 1e-5, "one hit is 4 px ({first})");
    let mut moved = first;
    for _ in 0..8 {
        moved += boss.shove(&level, Vec2::new(1.0, 0.0), 40.0);
    }
    assert!(
        moved <= SHOVE_PER_SEC_PX + 1e-5,
        "a second of shoves is at most 12 px ({moved})"
    );

    // The stagger pose cannot be re-entered or extended.
    boss.control_window = 0.0;
    boss.control_used = 0.0;
    boss.set_state(BossState::Cat(BossCatState::Hurt), 0.2);
    assert_eq!(
        boss.grant_control(0.3),
        0.0,
        "a stagger is not re-tripped while it plays"
    );
}

/// The pursuit rule: a rat that holds whip range is closed down, so the jaguar
/// gets its band back instead of stalking forever at 24 px/s.
#[test]
fn a_kited_jaguar_closes_the_gap_instead_of_idling() {
    let mut fight = ScriptedFight::new(BossKind::Cat, 4242);
    // No weapons at all: this is the pure pursue rule.
    // 150 px is outside the pounce band (45..95) and outside `engage_range`.
    // The rat starts at 100 px and runs out to its 150 px ring within a
    // second; the closing pounce waits `OUT_OF_REACH_SECS`, so "closed" is
    // measured against the ring it holds, once it is holding it.
    const RING: f32 = 150.0;
    let mut closest = f32::INFINITY;
    for i in 0..(12.0 / DT) as usize {
        fight.kite(RING, i as f32 * DT * 0.3);
        update_boss(&mut fight.w, DT);
        if i as f32 * DT > 1.5 {
            closest = closest.min(fight.boss_pos().dist(fight.w.player.pos));
        }
    }
    assert!(
        closest < RING - 30.0,
        "the jaguar never closed (ring {RING} px, closest {closest})"
    );
}
