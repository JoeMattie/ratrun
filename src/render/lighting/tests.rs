//! Assertions for the lighting pass: shapes, blends and layer masks; distinct
//! wall, prop and doorway shadows; cached fixture shadows refreshing when a
//! door opens; emissive cores; colour-depth defaults and palette dither; an
//! enemy staying readable at the edge of a blackout on every theme; and that
//! degenerate viewports or a change of tier never change the simulation.
//!
//! Also here: the fixture/emitter placement sweep over generated maps and the
//! timing measurements (placement cost, a 600-enemy frame), which print
//! numbers and are not correctness gates.

use super::*;
use crate::game::art_traps::OneWayDoorState;
use crate::game::art_walls::{Cardinal, WallAxis, WallKind};
use crate::game::bullet::{Bullet, Faction};
use crate::game::collision::TileKind;
use crate::game::ecs::components::Pool;
use crate::game::enemy::EnemyKind;
use crate::game::level::{
    AmbientEmitter, EmitterKind, LightFixture, LightFixtureKind, LightZone, LightZoneKind,
    OneWayDoor, Prop, PropKind, Rectf, RoomKind, TileRect,
};
use crate::game::world::World;
use crate::render::framebuffer::PixelBuffer;
use rand::RngExt;
use std::time::Instant;

const WALL: TileKind = TileKind::Wall(WallKind::Solid);

fn test_scene(world: &World, mode: LightingMode, depth: ColorDepth, cam: Vec2) -> LightScene<'_> {
    LightScene {
        ecs: &world.ecs,
        level: &world.level,
        player: &world.player,
        particles: &world.particles,
        thrash: None,
        squeak: None,
        cam,
        elapsed: world.elapsed,
        blackout: world.blackout(),
        mode,
        depth,
        fx: false,
    }
}

fn test_world(theme: Theme) -> World {
    let mut w = World::new(theme, 80);
    w.level = Level::empty(theme);
    w.player.pos = Vec2::new(700.0, 700.0);
    w.smoke_enabled = false;
    w
}

fn frame(w: &World, lights: &[Light], mode: LightingMode) -> PixelBuffer {
    let mut pb = PixelBuffer::new(128, 96);
    pb.clear((85, 71, 78));
    let mut map = LightMap::new();
    for &light in lights {
        map.push_shaped(light, 3);
    }
    map.apply_scene(
        &mut pb,
        &test_scene(w, mode, ColorDepth::Truecolor, Vec2::ZERO),
    );
    pb
}

fn lum(pb: &PixelBuffer, x: usize, y: usize) -> u8 {
    luminance(pb.pixel_at(x, y))
}

#[test]
fn shapes_blends_layers_and_ambient_zones() {
    let mut w = test_world(Theme::Sewer);
    w.level.objects.light_zones.push(LightZone {
        rect: TileRect::new(4, 0, 4, 6),
        kind: LightZoneKind::Blackout,
    });
    let idle = frame(&w, &[], LightingMode::Full);
    // The ambient is 40 % darker than it was (`AMBIENT_SCALE`), so the gap
    // between an open floor and a blackout scaled with it (12 -> 7).
    assert!(lum(&idle, 24, 42) >= lum(&idle, 90, 42) + 7);
    let p = Vec2::new(48.0, 42.0);
    let mut light = Light::radial(p, 63.0, (255, 199, 105), 0.95);
    let radial = frame(&w, &[light], LightingMode::Full);
    assert!(lum(&radial, 66, 42) > lum(&idle, 66, 42) + 30);
    assert!(lum(&radial, 66, 42) > lum(&radial, 103, 42));
    light.shape = LightShape::Cone {
        radius: 63.0,
        dir: Vec2::new(1.0, 0.0),
        half_angle: 0.61,
    };
    let cone = frame(&w, &[light], LightingMode::Full);
    assert!(lum(&cone, 70, 42) > lum(&idle, 70, 42) + 30);
    assert_eq!(lum(&cone, 20, 42), lum(&idle, 20, 42));
    assert_eq!(lum(&cone, 66, 69), lum(&idle, 66, 69));
    light.shape = LightShape::Beam {
        dir: Vec2::new(1.0, 0.0),
        length: 53.0,
        half_width: 10.0,
    };
    let beam = frame(&w, &[light], LightingMode::Full);
    assert!(lum(&beam, 80, 42) > lum(&idle, 80, 42) + 25);
    assert_eq!(lum(&beam, 80, 58), lum(&idle, 80, 58));
    light.blend = Blend::Subtract;
    assert!(lum(&frame(&w, &[light], LightingMode::Full), 60, 42) < lum(&idle, 60, 42));
    light.blend = Blend::Mix;
    let mix = frame(&w, &[light], LightingMode::Full).pixel_at(70, 42);
    assert!(mix.0 > 180 && mix.1 > 140 && mix.2 < 130, "{mix:?}");
    light.layers = LayerMask::PROPS;
    assert_eq!(
        frame(&w, &[light], LightingMode::Full).pixels(),
        idle.pixels()
    );
}

#[test]
fn walls_props_and_doorways_cast_distinct_shadows() {
    let mut w = test_world(Theme::Sewer);
    w.level.tiles.fill_cells(4, 0, 1, 6, WALL);
    w.level.tiles.set(
        4,
        2,
        TileKind::Wall(WallKind::Doorway {
            axis: WallAxis::Vertical,
        }),
    );
    w.level.props.push(Prop {
        rect: Rectf::new(48.0, 64.0, 16.0, 16.0),
        kind: PropKind::Crate,
    });
    let mut light = Light::radial(Vec2::new(38.0, 41.0), 110.0, (255, 235, 199), 0.98);
    light.shadow_softness = ShadowSoftness::None;
    let lit = frame(&w, &[light], LightingMode::Full);
    assert!(lum(&lit, 92, 42) > lum(&lit, 92, 18) + 35);
    assert!(lum(&lit, 66, 18) > lum(&lit, 78, 18) + 10);
    let floor = frame(&w, &[], LightingMode::Full);
    assert_eq!(lum(&floor, 92, 18), lum(&lit, 92, 18));
    assert!(lum(&lit, 66, 86) < lum(&lit, 42, 70));
    light.pos = Vec2::new(27.0, 54.0);
    light.shadow_softness = ShadowSoftness::None;
    let crate_shadow = frame(&w, &[light], LightingMode::Full);
    w.level.props[0].kind = PropKind::Grate;
    light.height = 17.0;
    let low_grate = frame(&w, &[light], LightingMode::Full);
    assert!(lum(&crate_shadow, 60, 74) + 15 < lum(&low_grate, 60, 74));

    // Measure a single penumbra at an isolated wall edge.
    w.level.tiles.fill_cells(4, 0, 1, 3, WALL);
    w.level.tiles.fill_cells(4, 3, 1, 3, TileKind::Open);
    light.pos = Vec2::new(38.0, 41.0);
    light.occluders = LayerMask::WALLS;
    light.shadow_softness = ShadowSoftness::None;
    let hard = frame(&w, &[light], LightingMode::Full);
    light.shadow_softness = ShadowSoftness::Softer;
    let soft = frame(&w, &[light], LightingMode::Full);
    let changed = (20..90)
        .filter(|&y| lum(&hard, 95, y).abs_diff(lum(&soft, 95, y)) >= 3)
        .count();
    assert!(
        (4..=28).contains(&changed),
        "soft edge affected {changed} pixels"
    );
}

#[test]
fn opening_a_door_refreshes_a_cached_fixture_shadow() {
    let mut world = test_world(Theme::Sewer);
    world.level.tiles.fill_cells(4, 0, 1, 6, WALL);
    world.level.tiles.set(
        4,
        2,
        TileKind::Wall(WallKind::OneWayDoor {
            dir: Cardinal::East,
        }),
    );
    world
        .level
        .objects
        .one_way_doors
        .push(OneWayDoor::new((4, 2), Vec2::new(1.0, 0.0)));
    world.level.objects.lights.push(LightFixture {
        pos: Vec2::new(44.0, 40.0),
        kind: LightFixtureKind::Passage,
        radius: 100.0,
        seed: 16,
    });
    let mut map = LightMap::new();
    let mut pb = PixelBuffer::new(128, 96);
    let measure = |map: &mut LightMap, pb: &mut PixelBuffer, world: &World| {
        pb.clear((79, 79, 79));
        map.apply_scene(
            pb,
            &test_scene(world, LightingMode::Hard, ColorDepth::Truecolor, Vec2::ZERO),
        );
        lum(pb, 90, 40)
    };
    let closed = measure(&mut map, &mut pb, &world);
    let gate = &mut world.level.objects.one_way_doors[0].view;
    gate.state = OneWayDoorState::Opening;
    gate.progress = 1.0;
    let open = measure(&mut map, &mut pb, &world);
    assert!(open >= closed + 18, "closed {closed}, opened {open}");
}

#[test]
fn directional_shadows_end_and_raised_caps_face_the_light() {
    let mut w = test_world(Theme::Trailer);
    w.level.props.push(Prop {
        rect: Rectf::new(48.0, 32.0, 16.0, 16.0),
        kind: PropKind::Crate,
    });
    let mut sun = Light::radial(Vec2::ZERO, 1.0, (255, 218, 145), 0.73);
    sun.shape = LightShape::Directional {
        dir: Vec2::new(1.0, 0.0),
    };
    sun.height = 0.9;
    sun.shadow_softness = ShadowSoftness::None;
    let day = frame(&w, &[sun], LightingMode::Full);
    assert!(lum(&day, 46, 39) > lum(&day, 70, 39) + 20);
    assert!(lum(&day, 100, 39) > lum(&day, 70, 39) + 20);
    assert!(lum(&day, 50, 39) > lum(&day, 62, 39) + 10);
}

#[test]
fn emissive_core_is_preserved_and_lights_nearby_pixels() {
    let w = test_world(Theme::Sewer);
    let mut map = LightMap::new();
    let mut pb = PixelBuffer::new(128, 96);
    pb.clear((35, 45, 50));
    let core = (211, 40, 25);
    pb.plot(40, 40, core);
    map.mark_emissive_rect(&pb, 40, 40, 1, 1);
    map.emit(Vec2::new(40.0, 40.0), core, 28.0, 0.62);
    map.apply_scene(
        &mut pb,
        &test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO),
    );
    assert!(pb.pixel_at(40, 40).0 >= core.0);
    assert!(pb.pixel_at(48, 40).0 > pb.pixel_at(71, 40).0 + 16);
}

#[test]
fn lighting_defaults_follow_colour_depth_and_palette_dither_keeps_multiple_tones() {
    assert_eq!(
        LightingMode::default_for_depth(ColorDepth::Truecolor),
        LightingMode::Full
    );
    assert_eq!(
        LightingMode::default_for_depth(ColorDepth::Ansi256),
        LightingMode::Hard
    );
    assert_eq!(
        LightingMode::default_for_depth(ColorDepth::Ansi16),
        LightingMode::Coarse
    );
    let mut map = LightMap::new();
    let w = test_world(Theme::Lab);
    for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256] {
        let mut pb = PixelBuffer::new(96, 80);
        pb.clear((104, 102, 107));
        map.begin_frame();
        map.emit(Vec2::new(48.0, 40.0), (218, 232, 255), 42.0, 1.10);
        let mode = LightingMode::default_for_depth(depth);
        map.apply_scene(&mut pb, &test_scene(&w, mode, depth, Vec2::ZERO));
        let variants: std::collections::HashSet<_> = pb
            .pixels()
            .iter()
            .map(|&color| crate::term::map_rgb_with(depth, color))
            .collect();
        assert!(variants.len() >= 3, "{depth:?}: {variants:?}");
    }
}

#[test]
fn every_theme_reads_an_enemy_at_the_edge_of_a_blackout() {
    for theme in Theme::playable() {
        let mut w = test_world(theme);
        w.player.pos = Vec2::new(400.0, 400.0);
        w.level.objects.light_zones.push(LightZone {
            rect: TileRect::new(22, 22, 9, 6),
            kind: LightZoneKind::Maze,
        });
        for depth in [ColorDepth::Truecolor, ColorDepth::Ansi16] {
            let mut pb = PixelBuffer::new(120, 80);
            w.lighting = LightingMode::default_for_depth(depth);
            w.lighting_depth = depth;
            w.clear_enemies();
            w.draw(&mut pb);
            let floor = pb.pixels().to_vec();
            w.spawn_enemy(
                EnemyKind::Skitterer,
                w.player.pos + Vec2::new(56.0, 2.0),
                1.0,
            );
            w.draw(&mut pb);
            let margin = (38..46)
                .flat_map(|y| (110..120).map(move |x| y * 120 + x))
                .map(|i| {
                    let lum = |c| luminance(super::gallery::terminal_rgb(c, depth));
                    lum(pb.pixels()[i]).saturating_sub(lum(floor[i]))
                })
                .max()
                .unwrap();
            assert!(margin >= 35, "{theme:?} {depth:?}: only {margin} contrast");
        }
    }
}

#[test]
fn blackout_keeps_the_rat_aura_at_its_normal_strength() {
    let mut w = test_world(Theme::Lab);
    w.player.pos = Vec2::new(48.0, 48.0);
    w.elapsed = 458.0;
    for fx in [false, true] {
        let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
        scene.fx = fx;
        let draw = |scene: &LightScene<'_>| {
            let mut pb = PixelBuffer::new(128, 96);
            pb.clear((85, 71, 78));
            LightMap::new().apply_scene(&mut pb, scene);
            pb
        };
        assert_eq!(scene.blackout, 1.0);
        let dark = draw(&scene);
        scene.blackout = 0.0;
        let normal = draw(&scene);
        assert_eq!(dark.pixels(), normal.pixels());
        assert!(lum(&dark, 52, 48) > lum(&dark, 124, 48) + 20);
    }
}

#[test]
fn drawing_a_blackout_does_not_change_simulation_or_fog() {
    let mut drawn = World::new(Theme::Sewer, 827);
    let mut headless = World::new(Theme::Sewer, 827);
    let viewport = Vec2::new(120.0, 80.0);
    drawn.viewport = viewport;
    headless.viewport = viewport;
    let mut pb = PixelBuffer::new(120, 80);
    for elapsed in [299.95, 300.55, 308.55, 309.55] {
        drawn.elapsed = elapsed;
        headless.elapsed = elapsed;
        for _ in 0..8 {
            drawn.update(1.0 / 60.0, Vec2::ZERO, false);
            headless.update(1.0 / 60.0, Vec2::ZERO, false);
            drawn.draw(&mut pb);
            assert_eq!(drawn.elapsed, headless.elapsed);
            assert_eq!(drawn.player.pos, headless.player.pos);
            assert_eq!(drawn.player.hp, headless.player.hp);
            assert_eq!(drawn.fog.seen_count(), headless.fog.seen_count());
            assert_eq!(drawn.sfx, headless.sfx);
            let enemies = |world: &mut World| {
                world
                    .enemies()
                    .iter()
                    .map(|e| (e.id, e.pos, e.hp))
                    .collect::<Vec<_>>()
            };
            assert_eq!(enemies(&mut drawn), enemies(&mut headless));
        }
    }
    assert_eq!(drawn.rng.random::<u64>(), headless.rng.random::<u64>());
}

#[test]
fn degenerate_viewports_and_quality_do_not_change_simulation() {
    let mut reference = World::new(Theme::Sewer, 827);
    reference.viewport = Vec2::new(120.0, 80.0);
    let mut worlds = LightingMode::LIVE.map(|mode| {
        let mut world = World::new(Theme::Sewer, 827);
        world.viewport = reference.viewport;
        world.lighting = mode;
        world.level.objects.emitters.push(AmbientEmitter {
            kind: EmitterKind::Fire,
            pos: world.player.pos + Vec2::new(17.0, 0.0),
            seed: 21,
            rate: 1.0,
            intensity: 1.0,
        });
        world
    });
    let mut pb = PixelBuffer::new(120, 80);
    for i in 0..120 {
        let dir = Vec2::from_angle((i / 20) as f32 * 1.57);
        let dash = i == 23;
        reference.update(1.0 / 60.0, dir, dash);
        for w in &mut worlds {
            w.update(1.0 / 60.0, dir, dash);
            w.draw(&mut pb);
            assert_eq!(w.player.pos, reference.player.pos);
            assert_eq!(w.player.hp, reference.player.hp);
            assert_eq!(w.kills, reference.kills);
            assert_eq!(w.fog.seen_count(), reference.fog.seen_count());
            assert_eq!(w.enemy_count(), reference.enemy_count());
        }
    }
    let sample = reference.rng.random::<u64>();
    for w in &mut worlds {
        assert_eq!(w.rng.random::<u64>(), sample);
        assert_eq!(
            w.enemies()
                .iter()
                .map(|e| (e.id, e.pos, e.hp))
                .collect::<Vec<_>>(),
            reference
                .enemies()
                .iter()
                .map(|e| (e.id, e.pos, e.hp))
                .collect::<Vec<_>>()
        );
        for (width, height) in [(0, 0), (1, 1), (5, 3)] {
            pb.resize(width, height);
            w.draw(&mut pb);
        }
    }
}

#[test]
fn fixtures_and_emitters_are_seeded_sparse_and_walkable_over_200_seeds() {
    for theme in Theme::playable() {
        let mut signature = false;
        for seed in 0..200 {
            let level = Level::generate(theme, seed);
            let twin = Level::generate(theme, seed);
            assert_eq!(level.objects.lights, twin.objects.lights);
            assert_eq!(level.objects.light_zones, twin.objects.light_zones);
            assert_eq!(level.objects.emitters, twin.objects.emitters);
            let ctx = (theme, seed);
            for fixture in &level.objects.lights {
                if fixture.kind != LightFixtureKind::Sun {
                    assert_eq!(level.tiles.get_at(fixture.pos), TileKind::Open, "{ctx:?}");
                    assert!(level.pit_at(fixture.pos).is_none(), "{ctx:?}");
                }
            }
            if theme == Theme::Trailer {
                assert_eq!(
                    level
                        .objects
                        .lights
                        .iter()
                        .filter(|f| f.kind == LightFixtureKind::Sun)
                        .count(),
                    1,
                    "{ctx:?}"
                );
            } else {
                assert_eq!(
                    level.objects.light_zones.len(),
                    level.rooms.len(),
                    "{ctx:?}"
                );
                if theme == Theme::Outfall {
                    // Outfall has four working entry sconces and sparse mounts
                    // thereafter (no default hanging lamps in every room).
                    let sconces = level
                        .objects
                        .lights
                        .iter()
                        .filter(|f| f.kind == LightFixtureKind::Sconce)
                        .count();
                    assert!(sconces >= 4, "{ctx:?}: only {sconces} sconces");
                } else {
                    for room in level.rooms.iter().filter(|r| r.kind != RoomKind::Maze) {
                        // A corridor's bounding rect can be entirely superseded
                        // by a later structure, in which case it has no open pad.
                        if room.kind == RoomKind::Corridor
                            && !room.rect.cells().any(|(x, y)| {
                                level.tiles.get(x, y) == TileKind::Open
                                    && !level.mazes.iter().any(|m| m.bounds.contains(x, y))
                            })
                        {
                            continue;
                        }
                        assert!(
                            level
                                .objects
                                .lights
                                .iter()
                                .any(|f| room.rect.contains_world(f.pos)),
                            "{ctx:?}: {room:?}"
                        );
                    }
                }
                assert_eq!(
                    level
                        .objects
                        .lights
                        .iter()
                        .filter(|f| f.kind == LightFixtureKind::Arena)
                        .count(),
                    0,
                    "{ctx:?}: no floor has arena lamps any more"
                );
            }
            let emitters = &level.objects.emitters;
            signature |= !emitters.is_empty();
            assert!(emitters.len() <= 4 + level.mazes.len(), "{ctx:?}");
            for e in emitters {
                assert_eq!(level.tiles.get_at(e.pos), TileKind::Open, "{ctx:?}: {e:?}");
                // The warren can be near the start behind stone. Its pilot
                // emits embers only, with no smoke in the first sightline.
                if e.kind != EmitterKind::Pilot
                    || !level.mazes.iter().any(|m| m.bounds.contains_world(e.pos))
                {
                    assert!(e.pos.dist(level.start) >= 190.0, "{ctx:?}: {e:?}");
                }
                assert!(level.pit_at(e.pos).is_none(), "{ctx:?}: {e:?}");
                assert!(
                    !level
                        .rooms
                        .iter()
                        .any(|r| r.kind == RoomKind::Boss && r.rect.contains_world(e.pos)),
                    "{ctx:?}: {e:?}"
                );
                assert!(e.rate > 0.0 && e.intensity > 0.0);
            }
        }
        assert!(signature, "{theme:?} has no signature place");
    }
}

#[test]
fn lighting_placement_time_on_generated_maps() {
    for theme in Theme::playable() {
        let mut samples: Vec<_> = (0..24)
            .map(|seed| {
                crate::game::levelgen::generate_once_timed(theme, seed, 0)
                    .1
                    .lights
            })
            .collect();
        samples.sort_unstable();
        eprintln!(
            "{theme:?}: fixture and emitter placement {} us median, {} us p95",
            samples[12], samples[22]
        );
    }
}

/// A scheduling pause on the shared machine is visible in the wall times.
/// Use the thread's CPU time for the bandwidth regression.
fn cpu_micros() -> Option<u128> {
    #[cfg(unix)]
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: clock_gettime writes one timespec to the supplied pointer.
        let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
        (result == 0).then_some(time.tv_sec as u128 * 1_000_000 + time.tv_nsec as u128 / 1000)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn sample_times<const N: usize>(mut frame: impl FnMut(usize)) -> ([u128; N], [u128; N]) {
    let mut cpu = [0; N];
    let mut wall = [0; N];
    for i in 0..N {
        let started = Instant::now();
        let cpu_started = cpu_micros();
        frame(i);
        cpu[i] = cpu_started
            .zip(cpu_micros())
            .map_or(0, |(a, b)| b.saturating_sub(a));
        wall[i] = started.elapsed().as_micros();
    }
    cpu.sort_unstable();
    wall.sort_unstable();
    (cpu, wall)
}

#[test]
fn lighting_timings_with_600_enemies_and_ambient_particles() {
    for theme in [Theme::Lab, Theme::Trailer] {
        let mut world = World::new(theme, 42);
        if theme == Theme::Lab {
            world.player.pos = world.level.largest_hall().unwrap().center();
        }
        let center = world.player.pos;
        for i in 0..600 {
            let a = i as f32 * 0.37;
            let r = 12.0 + (i % 8) as f32 * 11.0;
            let kind = [
                EnemyKind::Skitterer,
                EnemyKind::Bat,
                EnemyKind::Spitter,
                EnemyKind::Cat,
                EnemyKind::Brute,
            ][i % 5];
            world.spawn_enemy(kind, center + Vec2::from_angle(a) * r, 1.0);
        }
        for i in 0..40 {
            let a = i as f32 * 0.15;
            world.spawn_bullet(Bullet::new(
                center + Vec2::from_angle(a) * 28.0,
                Vec2::from_angle(a) * 80.0,
                5.0,
                if i % 3 == 0 {
                    Faction::Enemy
                } else {
                    Faction::Player
                },
                (255, 180, 90),
            ));
        }
        for i in 0..8 {
            world.spawn_pool(Pool {
                pos: center + Vec2::from_angle(i as f32 * 0.78) * 35.0,
                radius: 11.0,
                dps: 6.0,
                life: 2.0,
                color: (150, 255, 90),
            });
        }
        for (i, kind) in [EmitterKind::Fire, EmitterKind::Steam, EmitterKind::Fog]
            .into_iter()
            .enumerate()
        {
            world.level.objects.emitters.push(AmbientEmitter {
                kind,
                pos: center + Vec2::new((i as f32 - 1.0) * 34.0, 12.0),
                seed: 197 + i as u32,
                rate: 1.0,
                intensity: 1.0,
            });
        }
        let mut map = LightMap::new();
        for (zoom, width, height) in [(1.0, 120, 80), (2.0, 240, 160)] {
            for mode in LightingMode::LIVE {
                let depth = match mode {
                    LightingMode::Full => ColorDepth::Truecolor,
                    LightingMode::Hard => ColorDepth::Ansi256,
                    LightingMode::Coarse | LightingMode::Compact => ColorDepth::Ansi16,
                    LightingMode::Off => unreachable!(),
                };
                let mut pb = PixelBuffer::new(width, height);
                let cam = center - Vec2::new(width as f32 * 0.5, height as f32 * 0.5);
                let do_pass = |map: &mut LightMap, pb: &mut PixelBuffer, time: f32| {
                    map.begin_frame();
                    let mut scene = test_scene(&world, mode, depth, cam);
                    scene.elapsed = time;
                    draw_ambience(
                        map,
                        pb,
                        &scene.level.objects.emitters,
                        cam,
                        time,
                        scene.player.pos,
                    );
                    map.apply_scene(pb, &scene);
                };
                let (cold_cpu, cold_wall) = sample_times::<1>(|_| do_pass(&mut map, &mut pb, 12.0));
                let (light_cpu, light_wall) = sample_times::<48>(|i| {
                    do_pass(&mut map, &mut pb, 12.0 + i as f32 / 60.0);
                });
                let mut screen = PixelBuffer::new(120, 80);
                world.lighting = mode;
                world.lighting_depth = depth;
                world.set_zoom_instant(zoom);
                world.draw(&mut screen);
                let (frame_cpu, frame_wall) = sample_times::<24>(|i| {
                    world.elapsed = 12.0 + i as f32 / 60.0;
                    world.draw(&mut screen);
                });
                eprintln!("{theme:?} {zoom:.0}x {mode:?}: lighting + emitters median {}/{} us CPU/wall, cold {}/{} us, frame median {}/{} us and wall p95 {} us",
                    light_cpu[24], light_wall[24], cold_cpu[0], cold_wall[0],
                    frame_cpu[12], frame_wall[12], frame_wall[22]);
                assert!(frame_cpu[12] < 16_000);
            }
        }
    }
}

#[test]
fn a_flood_of_halos_cannot_evict_the_rats_aura() {
    // The rat's aura joins the shaped list inside `apply_scene`, after every
    // `emit` of the frame. A full list used to drop it silently.
    let w = test_world(Theme::Sewer);
    let cam = w.player.pos - Vec2::new(64.0, 48.0);
    let shot = |halos: usize| {
        let mut map = LightMap::new();
        let mut pb = PixelBuffer::new(128, 96);
        pb.clear((104, 102, 107));
        map.begin_frame();
        for _ in 0..halos {
            // Far off-screen: the halos themselves light nothing here.
            map.emit(Vec2::new(-4000.0, -4000.0), (255, 90, 60), 20.0, 0.6);
        }
        map.apply_scene(
            &mut pb,
            &test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, cam),
        );
        pb
    };
    let calm = shot(0);
    let flood = shot(200);
    for (x, y) in [(64, 48), (72, 48), (64, 58)] {
        assert_eq!(lum(&flood, x, y), lum(&calm, x, y), "at {x},{y}");
    }
}

#[test]
fn lighting_tier_stays_fixed_across_frames_and_prop_pools_survive_bullet_volleys() {
    let mut w = test_world(Theme::Lab);
    w.player.pos = Vec2::new(200.0, 200.0);
    w.level.props.push(Prop {
        kind: PropKind::Console,
        rect: Rectf::new(256.0, 192.0, 16.0, 16.0),
    });
    let mut pb = PixelBuffer::new(160, 100);
    for mode in LightingMode::LIVE {
        w.lighting = mode;
        for i in 0..16 {
            w.elapsed = i as f32 * 0.25;
            w.draw(&mut pb);
            assert_eq!(w.lighting_tier(), mode);
        }
    }

    // A stationary room prop pool (`Console`) must not pop off when a volley of
    // player projectiles spawns around the rat and overflows the dynamic cap.
    let cam = Vec2::new(140.0, 150.0);
    let render = |w: &World| {
        let mut pb = PixelBuffer::new(160, 100);
        pb.clear((72, 76, 82));
        let mut map = LightMap::new();
        map.begin_frame();
        map.apply_scene(
            &mut pb,
            &test_scene(w, LightingMode::Full, ColorDepth::Truecolor, cam),
        );
        pb
    };
    let calm = render(&w);
    for i in 0..40 {
        let a = i as f32 * 0.15;
        w.spawn_bullet(Bullet::new(
            w.player.pos + Vec2::from_angle(a) * 8.0,
            Vec2::from_angle(a) * 90.0,
            4.0,
            Faction::Player,
            (255, 210, 80),
        ));
    }
    let volley = render(&w);
    // Probe on the far side of the Console (x = 132 in screen px -> world x = 272,
    // 72 px from the rat and out of reach of the muzzle bullets).
    assert!(
        lum(&calm, 132, 50) > 45,
        "console pool must light the floor: {}",
        lum(&calm, 132, 50)
    );
    assert_eq!(
        lum(&volley, 132, 50),
        lum(&calm, 132, 50),
        "player projectile volley must not evict a stationary room prop light"
    );
}

#[test]
fn a_prop_lapping_onto_a_wall_does_not_open_a_light_leak() {
    let mut w = test_world(Theme::Sewer);
    w.level.tiles.fill_cells(4, 0, 1, 6, WALL);
    let mut light = Light::radial(Vec2::new(38.0, 41.0), 110.0, (255, 235, 199), 0.98);
    light.shadow_softness = ShadowSoftness::None;
    let sealed = frame(&w, &[light], LightingMode::Full);
    // A coil with no occluder of its own, half on the floor and half on the
    // wall cell straight between the light and the probe.
    w.level.props.push(Prop {
        rect: Rectf::new(56.0, 32.0, 16.0, 16.0),
        kind: PropKind::ChainCoil,
    });
    let lapped = frame(&w, &[light], LightingMode::Full);
    assert_eq!(lum(&lapped, 92, 42), lum(&sealed, 92, 42));
}

#[test]
fn a_room_seen_from_a_passage_is_no_brighter_than_the_passage() {
    // The view is a lamp-lit room; the rat stands far off-screen at (700, 700).
    let mut w = test_world(Theme::Kitchen);
    w.level.objects.light_zones.push(LightZone {
        rect: TileRect::new(0, 0, 8, 6),
        kind: LightZoneKind::Room,
    });
    w.level.objects.lights.push(LightFixture {
        pos: Vec2::new(64.0, 48.0),
        kind: LightFixtureKind::Pendant,
        radius: 60.0,
        seed: 1,
    });
    // Unzoned ground under the rat: nothing is capped.
    let open = frame(&w, &[], LightingMode::Full);
    // The rat in a passage: the room's ambient and its lamp both come down.
    w.level.objects.light_zones.push(LightZone {
        rect: TileRect::new(40, 40, 8, 8),
        kind: LightZoneKind::Corridor,
    });
    let from_passage = frame(&w, &[], LightingMode::Full);
    assert!(
        // 8 before `AMBIENT_SCALE` 0.60 took two fifths off both levels.
        lum(&from_passage, 8, 8) + 4 < lum(&open, 8, 8),
        "ambient: {} vs {}",
        lum(&from_passage, 8, 8),
        lum(&open, 8, 8)
    );
    assert!(
        lum(&from_passage, 70, 52) + 8 < lum(&open, 70, 52),
        "lamp: {} vs {}",
        lum(&from_passage, 70, 52),
        lum(&open, 70, 52)
    );
    // The rat in a room of his own: an equal room is shown at full value.
    w.level.objects.light_zones[1].kind = LightZoneKind::Room;
    let from_room = frame(&w, &[], LightingMode::Full);
    assert_eq!(lum(&from_room, 8, 8), lum(&open, 8, 8));
    assert_eq!(lum(&from_room, 70, 52), lum(&open, 70, 52));
}

// ---------------------------------------------------------------------------
// Batch 8: the small-light tail and the darker base level
// ---------------------------------------------------------------------------

/// `Curve` samples for a small light at `d` px: the raw shape and what the
/// pixel actually sees (core plus the quantised tail).
fn small_light_samples(d: f32) -> (f32, f32) {
    let mut light = Light::radial(Vec2::ZERO, 24.0, (255, 208, 148), 0.9);
    light.falloff = Falloff::Small;
    let curve = Curve::new(&light);
    let (core, tail) = curve.parts(Vec2::new(d, 0.0), d * d, 0.0);
    (core + tail, core + curve.quantise(tail))
}

#[test]
fn a_small_light_reaches_farther_and_fades_softer() {
    let mut light = Light::radial(Vec2::ZERO, 24.0, (255, 208, 148), 0.9);
    light.falloff = Falloff::Small;
    let curve = Curve::new(&light);
    let old_radius = 24.0;
    let reach = light.extent();
    assert!((reach - old_radius * 1.7).abs() < 0.01, "reach {reach}");

    // Monotone all the way out, both the physical shape and the painted one.
    let (mut raw_before, mut painted_before) = small_light_samples(0.0);
    let mut d = 0.0;
    while d < reach + 8.0 {
        d += 0.5;
        let (raw, painted) = small_light_samples(d);
        assert!(raw <= raw_before + 1e-6, "raw rises at {d} px");
        assert!(painted <= painted_before + 1e-6, "painted rises at {d} px");
        raw_before = raw;
        painted_before = painted;
    }

    // Exactly zero from the end of the reach: no edge, no ring.
    assert_eq!(small_light_samples(reach).0, 0.0);
    assert_eq!(small_light_samples(reach + 0.5).0, 0.0);
    // Still lit at 1.5x the old radius, and under 8 % of the core by 60 % of
    // the reach.
    let at_old_edge = small_light_samples(old_radius * 1.5).0;
    assert!(at_old_edge > 0.0, "dark at 1.5x the old radius");
    assert!(
        small_light_samples(reach * 0.6).0 < 0.08,
        "60 % of the reach still holds {}",
        small_light_samples(reach * 0.6).0
    );

    // Past the old radius the pool is the quantised tail, and a step there is
    // at most one output level of this light's own brightness.
    let one_level = curve.step();
    assert!((one_level * curve.gain() - MIN_GAIN).abs() < 1e-3, "step");
    let mut d = old_radius;
    while d < reach {
        let (_, near) = small_light_samples(d);
        let (_, far) = small_light_samples(d + 0.5);
        assert!(near - far <= one_level + 1e-6, "a step at {d} px");
        d += 0.5;
    }
}

#[test]
fn every_floor_and_zone_is_darker_by_two_fifths_and_never_black() {
    let zones = [
        None,
        Some(LightZoneKind::Start),
        Some(LightZoneKind::Room),
        Some(LightZoneKind::Corridor),
        Some(LightZoneKind::Arena),
        Some(LightZoneKind::Blackout),
        Some(LightZoneKind::Maze),
    ];
    for theme in Theme::playable() {
        let profile = theme_ambient(theme);
        for zone in zones {
            let gain = zone_gain(zone);
            for (channel, &mul) in profile.mul.iter().enumerate() {
                let before = f32::from(mul) * gain;
                let now = ambient_level(mul, gain);
                assert!(
                    now >= AMBIENT_FLOOR,
                    "{theme:?} {zone:?} ch{channel} is black"
                );
                assert!(
                    f32::from(now) < before,
                    "{theme:?} {zone:?} ch{channel}: {now} vs {before}"
                );
                if now > AMBIENT_FLOOR {
                    let ratio = f32::from(now) / before;
                    assert!(
                        (0.55..=0.65).contains(&ratio),
                        "{theme:?} {zone:?} ch{channel}: ratio {ratio}"
                    );
                }
            }
        }
    }
    // The tint survives the lowering: the plane keeps its own hue.
    let sewer = theme_ambient(Theme::Sewer);
    assert!(sewer.mul[2] > sewer.mul[0], "Sewer should stay green-blue");
    assert!(theme_ambient(Theme::Kitchen).mul[0] > theme_ambient(Theme::Kitchen).mul[2]);
}

#[test]
fn the_far_tail_is_frozen_while_the_core_breathes() {
    // A moving halo is what repaints cells, so a *still* frame must not pay for
    // a flickering light's faint skirt: the tail is quantised and the flicker
    // animates the core only.
    let w = test_world(Theme::Sewer);
    let mut light = Light::radial(Vec2::new(64.0, 48.0), 26.0, (255, 176, 88), 1.0);
    light.falloff = Falloff::Small;
    light.casts_shadow = false;
    light.scatter = 0.24;
    let flicker = Flicker {
        amount: 0.40,
        speed: 8.9,
        phase: 0.0,
    };
    light.flicker = Some(flicker);
    // The two times are the flicker's own minimum and maximum.
    let (low, high) = (0.176, 0.529);
    assert!(
        flicker.gain(high) > flicker.gain(low) + 0.2,
        "{} vs {}",
        flicker.gain(low),
        flicker.gain(high)
    );
    let shot = |elapsed: f32| {
        let mut pb = PixelBuffer::new(128, 96);
        pb.clear((32, 40, 38));
        let mut map = LightMap::new();
        map.begin_frame();
        map.push_shaped(light, 3);
        let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
        scene.elapsed = elapsed;
        map.apply_scene(&mut pb, &scene);
        pb
    };
    let dim = shot(low);
    let bright = shot(high);
    let mut changed_in_core = 0;
    for y in 0..96usize {
        for x in 0..128usize {
            let d = (Vec2::new(x as f32, y as f32) - light.pos).len();
            if d > light.shape.extent() {
                assert_eq!(
                    dim.pixel_at(x, y),
                    bright.pixel_at(x, y),
                    "the tail moved at {x},{y} ({d} px out)"
                );
            } else if d < light.shape.extent() * 0.5
                && dim.pixel_at(x, y) != bright.pixel_at(x, y)
            {
                changed_in_core += 1;
            }
        }
    }
    assert!(changed_in_core > 20, "the core never animated");
}

#[test]
fn an_enemy_rim_glint_stays_lit_in_the_dark() {
    // The mark itself: a body pixel drawn before the lighting pass keeps a
    // pixel of its own colour however dark the ambient is.
    let w = test_world(Theme::Sewer);
    // `rim_glint` of a warm coat. Asserted at full colour: at 16 and 256
    // colours a glint this faint is inside the palette's first step, and the
    // silhouette there is carried by the readable mark the enemy draw pass
    // takes (asserted end to end below).
    let tint = (30, 13, 9);
    let mut pb = PixelBuffer::new(64, 48);
    pb.clear((14, 22, 20));
    pb.plot(32, 24, (52, 44, 40));
    let mut map = LightMap::new();
    map.mark_glint(&pb, 32, 24, tint);
    map.apply_scene(
        &mut pb,
        &test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO),
    );
    let lit = pb.pixel_at(32, 24);
    assert!(
        lit.0 >= tint.0 && lit.1 >= tint.1 && lit.2 >= tint.2,
        "{lit:?}"
    );
    assert!(
        luminance(lit) > luminance(pb.pixel_at(40, 24)) + 8,
        "{lit:?} vs the floor"
    );

    // And end to end: every floor's darkest zone still shows a live body.
    for theme in Theme::playable() {
        let mut w = test_world(theme);
        w.player.pos = Vec2::new(400.0, 400.0);
        w.level.objects.light_zones.push(LightZone {
            rect: TileRect::new(20, 20, 16, 12),
            kind: LightZoneKind::Maze,
        });
        let mut pb = PixelBuffer::new(120, 80);
        for depth in [ColorDepth::Truecolor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            w.lighting = LightingMode::default_for_depth(depth);
            w.lighting_depth = depth;
            w.clear_enemies();
            w.draw(&mut pb);
            let floor = pb.pixels().to_vec();
            w.spawn_enemy(
                EnemyKind::Skitterer,
                w.player.pos + Vec2::new(56.0, 2.0),
                1.0,
            );
            w.draw(&mut pb);
            let margin = (36..48)
                .flat_map(|y| (112..120).map(move |x| y * 120 + x))
                .map(|i| {
                    let lum = |c| luminance(super::gallery::terminal_rgb(c, depth));
                    lum(pb.pixels()[i]).saturating_sub(lum(floor[i]))
                })
                .max()
                .unwrap_or(0);
            assert!(margin >= 20, "{theme:?} {depth:?}: only {margin} contrast");
        }
    }
}

#[test]
fn a_lit_wall_stays_a_step_above_the_floor_at_every_colour_depth() {
    // The darker base level must not fold a wall face and a floor tile into the
    // same terminal colour where a lamp reaches them, or the maze's shape is
    // lost. (Outside a pool both are the near-black ambient, as before.) At 16
    // colours the palette is coarse enough that a mid-tone wall face and a lit
    // floor can share the one grey; the wall's own edge highlight is what a
    // 16-colour terminal reads, so that is the pixel probed there.
    let light = Light::radial(Vec2::new(64.0, 48.0), 90.0, (255, 244, 224), 1.35);
    for theme in Theme::playable() {
        let pal = crate::game::level::Palette::for_theme(theme);
        let w = test_world(theme);
        for depth in [ColorDepth::Truecolor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            let mut pb = PixelBuffer::new(128, 96);
            for y in 0..96usize {
                for x in 0..128usize {
                    pb.plot(x as i32, y as i32, if x < 64 { pal.bg } else { pal.wall });
                }
            }
            pb.plot(70, 48, pal.wall_edge);
            let mut map = LightMap::new();
            map.begin_frame();
            map.push_shaped(light, 3);
            map.apply_scene(
                &mut pb,
                &test_scene(&w, LightingMode::default_for_depth(depth), depth, Vec2::ZERO),
            );
            let floor = crate::term::map_rgb_with(depth, pb.pixel_at(58, 48));
            let edge = crate::term::map_rgb_with(depth, pb.pixel_at(70, 48));
            assert_ne!(floor, edge, "{theme:?} {depth:?}: the wall edge vanished");
            if depth != ColorDepth::Ansi16 {
                let wall = crate::term::map_rgb_with(depth, pb.pixel_at(72, 48));
                assert_ne!(floor, wall, "{theme:?} {depth:?}: the wall face vanished");
            }
        }
    }
}

#[test]
fn outfall_lure_traps_light_ready_and_warning_and_go_dark_when_spent() {
    use crate::game::hazards::LurePhase;

    let generated = Level::generate(Theme::Outfall, 7);
    assert!(
        !generated.objects.lure_traps.is_empty(),
        "generated Outfall map must carry lure traps"
    );
    let base_trap = generated.objects.lure_traps[0].clone();

    let render = |phase: LurePhase| {
        let mut w = test_world(Theme::Outfall);
        let mut trap = base_trap.clone();
        trap.pos = Vec2::new(64.0, 48.0);
        trap.phase = phase;
        w.level.objects.lure_traps.push(trap);
        let mut pb = PixelBuffer::new(128, 96);
        pb.clear((42, 56, 62));
        let mut map = LightMap::new();
        let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
        scene.fx = true;
        map.apply_scene(&mut pb, &scene);
        pb
    };

    let mut bare_w = test_world(Theme::Outfall);
    bare_w.level.objects.lure_traps.clear();
    let mut bare_pb = PixelBuffer::new(128, 96);
    bare_pb.clear((42, 56, 62));
    let mut bare_map = LightMap::new();
    let mut bare_scene = test_scene(&bare_w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
    bare_scene.fx = true;
    bare_map.apply_scene(&mut bare_pb, &bare_scene);

    let ready = render(LurePhase::Ready);
    let warning = render(LurePhase::Warning);
    let spent = render(LurePhase::Spent);

    assert!(
        lum(&ready, 64, 48) > lum(&bare_pb, 64, 48) + 20,
        "ready lure trap lights the seabed"
    );
    assert!(
        lum(&warning, 64, 48) >= lum(&ready, 64, 48),
        "warning lure trap is at least as bright as ready"
    );
    assert_eq!(
        spent.pixels(),
        bare_pb.pixels(),
        "spent lure trap emits no light"
    );
}

#[test]
fn outfall_lantern_real_lure_has_steady_air_and_decoy_stays_cheap() {
    let w = test_world(Theme::Outfall);
    let run_lure = |energy: f32, steady_energy: f32, lure_air: f32| {
        let mut pb = PixelBuffer::new(128, 96);
        pb.clear((42, 56, 62));
        let mut map = LightMap::new();
        let mut light = Light::radial(Vec2::new(64.0, 48.0), 40.0, (66, 255, 214), energy);
        light.scatter = 0.28;
        light.falloff = Falloff::Smooth;
        light.casts_shadow = false;
        map.push_lantern_lure(light, steady_energy, lure_air);
        let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
        scene.fx = true;
        map.apply_scene(&mut pb, &scene);
        (pb, map)
    };

    // Two different 3 Hz surface flicker levels with the same steady channel:
    // air scatter must stay identical while surface pixels reflect the flicker.
    let (dim_pb, dim_map) = run_lure(1.05, 1.35 + 0.28, 1.0);
    let (bright_pb, bright_map) = run_lure(1.35, 1.35 + 0.28, 1.0);
    assert!(lum(&bright_pb, 64, 48) >= lum(&dim_pb, 64, 48));
    assert_ne!(dim_pb.pixels(), bright_pb.pixels());

    // A cheap non-dominant decoy (Falloff::Small, Moving) lights pixels
    // without dominating `LightInfo` or adding air shafts.
    let mut decoy_pb = PixelBuffer::new(128, 96);
    decoy_pb.clear((42, 56, 62));
    let mut decoy_map = LightMap::new();
    let mut decoy = Light::radial(Vec2::new(64.0, 48.0), 22.0, (66, 255, 214), 0.72);
    decoy.scatter = 0.16;
    decoy.falloff = Falloff::Small;
    decoy.casts_shadow = false;
    decoy_map.push_shaped(decoy, 2);
    let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
    scene.fx = true;
    decoy_map.apply_scene(&mut decoy_pb, &scene);
    assert!(lum(&decoy_pb, 64, 48) > 20, "decoy still visibly glows");
    drop((dim_map, bright_map));
}

#[test]
fn outfall_lit_floor_is_bluer_than_sewer_across_all_tiers_and_depths() {
    let albedo: Rgb = (85, 78, 72);
    let light = Light::radial(Vec2::new(64.0, 48.0), 80.0, (240, 232, 220), 1.1);
    let render = |theme: Theme, mode: LightingMode, depth: ColorDepth, fx: bool| {
        let w = test_world(theme);
        let mut pb = PixelBuffer::new(128, 96);
        pb.set_materials(fx && matches!(mode, LightingMode::Full | LightingMode::Hard));
        pb.set_pen(0);
        pb.clear(albedo);
        let mut map = LightMap::new();
        map.push_shaped(light, 3);
        let mut scene = test_scene(&w, mode, depth, Vec2::ZERO);
        scene.fx = fx;
        map.apply_scene(&mut pb, &scene);
        pb
    };
    let blue_bias = |pb: &PixelBuffer, depth: ColorDepth| -> i64 {
        let mut sum = 0i64;
        for y in 24..72 {
            for x in 32..96 {
                let c = super::gallery::terminal_rgb(pb.pixel_at(x, y), depth);
                sum += i64::from(c.2) - i64::from(c.0);
            }
        }
        sum
    };

    for fx in [true, false] {
        for mode in LightingMode::LIVE {
            let sewer = render(Theme::Sewer, mode, ColorDepth::Truecolor, fx);
            let outfall = render(Theme::Outfall, mode, ColorDepth::Truecolor, fx);
            let s_bias = blue_bias(&sewer, ColorDepth::Truecolor);
            let o_bias = blue_bias(&outfall, ColorDepth::Truecolor);
            assert!(
                o_bias > s_bias + 40_000,
                "Outfall ({o_bias}) must be much bluer than Sewer ({s_bias}) at {mode:?} fx={fx}"
            );
        }
        // Survives 256-colour folding: Outfall stays visibly blue-shifted after palette mapping.
        let sewer_256 = render(Theme::Sewer, LightingMode::Hard, ColorDepth::Ansi256, fx);
        let outfall_256 = render(Theme::Outfall, LightingMode::Hard, ColorDepth::Ansi256, fx);
        let s256 = blue_bias(&sewer_256, ColorDepth::Ansi256);
        let o256 = blue_bias(&outfall_256, ColorDepth::Ansi256);
        assert!(
            o256 > s256 + 20_000,
            "Outfall must survive 256-colour folding bluer than Sewer: {o256} vs {s256} (fx={fx})"
        );
    }

    // Lighting OFF applies the cheap flat underwater grade on Outfall and leaves Sewer untouched.
    let sewer_off = render(Theme::Sewer, LightingMode::Off, ColorDepth::Truecolor, false);
    let outfall_off = render(Theme::Outfall, LightingMode::Off, ColorDepth::Truecolor, false);
    assert_eq!(sewer_off.pixel_at(64, 48), albedo);
    let o_px = outfall_off.pixel_at(64, 48);
    assert!(
        o_px.2 > o_px.0 + 40,
        "Outfall lighting-OFF flat grade must shift blue: {o_px:?}"
    );
}

#[test]
fn outfall_caustics_vary_over_space_and_time_and_are_absent_on_other_floors() {
    let light = Light::radial(Vec2::new(64.0, 48.0), 84.0, (180, 230, 240), 1.15);
    let render = |theme: Theme, elapsed: f32, fx: bool| {
        let mut w = test_world(theme);
        w.elapsed = elapsed;
        w.level.objects.light_zones.push(LightZone {
            rect: TileRect::new(0, 0, 8, 6),
            kind: LightZoneKind::Room,
        });
        let mut pb = PixelBuffer::new(128, 96);
        pb.set_materials(fx);
        pb.set_pen(0);
        pb.clear((64, 72, 74));
        let mut map = LightMap::new();
        map.push_shaped(light, 3);
        let mut scene = test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO);
        scene.fx = fx;
        map.apply_scene(&mut pb, &scene);
        pb
    };

    for fx in [true, false] {
        // 2.000 s and 2.040 s are inside the same 8 Hz bucket: continuous
        // Outfall caustics must still advance smoothly without stepping.
        let a = render(Theme::Outfall, 2.000, fx);
        let b = render(Theme::Outfall, 2.040, fx);
        assert_ne!(
            a.pixels(),
            b.pixels(),
            "Outfall caustics must animate continuously over elapsed (fx={fx})"
        );
        // Spatial variation across the lit floor (bright net vs troughs).
        let a = &a;
        let unique: std::collections::HashSet<Rgb> = (32..96)
            .flat_map(|x| (24..72).map(move |y| a.pixel_at(x, y)))
            .collect();
        assert!(
            unique.len() >= 4,
            "Outfall caustics must vary spatially across lit floor (fx={fx}): {}",
            unique.len()
        );
    }

    // Every non-Outfall floor has no caustics (no continuous drift inside an 8 Hz step).
    for theme in Theme::playable() {
        if theme == Theme::Outfall {
            continue;
        }
        for fx in [true, false] {
            let a = render(theme, 2.000, fx);
            let b = render(theme, 2.040, fx);
            assert_eq!(
                a.pixels(),
                b.pixels(),
                "{theme:?} must stay free of Outfall caustics (fx={fx})"
            );
        }
    }
}

#[test]
fn non_outfall_themes_keep_batch12_fx_off_and_off_mode_identity() {
    for theme in Theme::playable() {
        if theme == Theme::Outfall {
            continue;
        }
        let w = test_world(theme);
        let albedo: Rgb = (85, 71, 78);
        // LightingMode::Off must leave every non-Outfall pixel untouched.
        let mut off_pb = PixelBuffer::new(64, 48);
        off_pb.clear(albedo);
        let mut off_map = LightMap::new();
        off_map.apply_scene(
            &mut off_pb,
            &test_scene(&w, LightingMode::Off, ColorDepth::Truecolor, Vec2::ZERO),
        );
        assert!(
            off_pb.pixels().iter().all(|&p| p == albedo),
            "{theme:?} LightingMode::Off must remain byte-identical"
        );

        // With fx = false and no lights, every floor pixel matches the batch-12
        // ambient multiplier exactly (no Outfall grade or caustic shift).
        let mut pb = PixelBuffer::new(64, 48);
        pb.clear(albedo);
        let mut map = LightMap::new();
        map.apply_scene(
            &mut pb,
            &test_scene(&w, LightingMode::Full, ColorDepth::Truecolor, Vec2::ZERO),
        );
        let profile = theme_ambient(theme);
        let gain = zone_gain(None);
        let expected_mul = profile.mul.map(|c| ambient_level(c, gain));
        let expected = (
            ((i32::from(albedo.0) * i32::from(expected_mul[0])) >> 8) as u8,
            ((i32::from(albedo.1) * i32::from(expected_mul[1])) >> 8) as u8,
            ((i32::from(albedo.2) * i32::from(expected_mul[2])) >> 8) as u8,
        );
        assert_eq!(
            pb.pixel_at(32, 24),
            expected,
            "{theme:?} fx=false must match batch-12 composite exactly"
        );
    }
}

#[test]
fn outfall_frozen_frames_are_bit_identical_and_hostile_layer_stays_untinted() {
    for fx in [true, false] {
        let mut w = World::new(Theme::Outfall, 77);
        w.elapsed = 12.345;
        w.lighting = LightingMode::Full;
        w.lighting_fx = fx;
        w.smoke_enabled = false;
        let mut first = PixelBuffer::new(120, 80);
        let mut second = PixelBuffer::new(120, 80);
        // Prime any first-frame eased cap, then verify frozen frames match bit-for-bit.
        w.draw(&mut first);
        w.draw(&mut first);
        w.draw(&mut second);
        assert_eq!(
            first.pixels(),
            second.pixels(),
            "frozen Outfall frame must be bit-identical (fx={fx})"
        );
    }

    // Hostile layer (enemy pellet) is drawn after lighting and must stay untinted.
    let mut w = World::new(Theme::Outfall, 77);
    w.elapsed = 4.0;
    w.lighting = LightingMode::Full;
    w.lighting_fx = true;
    w.smoke_enabled = false;
    let pellet_pos = w.player.pos + Vec2::new(24.0, 0.0);
    w.spawn_bullet(Bullet::new(
        pellet_pos,
        Vec2::ZERO,
        4.0,
        Faction::Enemy,
        (255, 60, 40),
    ));
    let mut pb = PixelBuffer::new(120, 80);
    w.draw(&mut pb);
    let sx = (pellet_pos.x - w.cam.x).round() as usize;
    let sy = (pellet_pos.y - w.cam.y).round() as usize;
    let mut max_red = 0u8;
    for dy in sy.saturating_sub(2)..=(sy + 2).min(pb.h - 1) {
        for dx in sx.saturating_sub(2)..=(sx + 2).min(pb.w - 1) {
            let c = pb.pixel_at(dx, dy);
            if c.0 > c.2.saturating_add(40) {
                max_red = max_red.max(c.0);
            }
        }
    }
    assert!(
        max_red >= 240,
        "hostile pellet on Outfall must remain bright untinted red (got max_red={max_red})"
    );
}
