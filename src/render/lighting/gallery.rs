//! Capture harnesses for the lighting pass (test builds only).
//!
//! `capture_lighting_gallery` and `capture_ambience_gallery` stage scenes and
//! write PPM frames per lighting tier and colour depth for a human to compare;
//! they assert nothing about the picture. Both are no-ops unless
//! `RATRUN_LIGHTING_CAPTURES=<dir>` is set. `terminal_rgb` gives
//! the colour a cell lands on after the fold to a terminal depth.

use super::*;
use crate::game::art_boss_cat::BossCatState;
use crate::game::art_boss_mower::MowerState;
use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::components::Pool;
use crate::game::ecs::systems::boss::BossState;
use crate::game::enemy::EnemyKind;
use crate::game::level::PropKind;
use crate::game::{
    art_walls::{WallAxis, WallKind},
    collision::TileKind,
    world::World,
};
use crate::render::framebuffer::PixelBuffer;

pub(super) fn terminal_rgb(c: Rgb, depth: ColorDepth) -> Rgb {
    use ratatui::style::Color::*;
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match crate::term::map_rgb_with(depth, c) {
        Rgb(r, g, b) => (r, g, b),
        Black => (0, 0, 0),
        Red => (128, 0, 0),
        Green => (0, 128, 0),
        Yellow => (128, 128, 0),
        Blue => (0, 0, 128),
        Magenta => (128, 0, 128),
        Cyan => (0, 128, 128),
        Gray => (192, 192, 192),
        DarkGray => (128, 128, 128),
        LightRed => (255, 0, 0),
        LightGreen => (0, 255, 0),
        LightYellow => (255, 255, 0),
        LightBlue => (0, 0, 255),
        LightMagenta => (255, 0, 255),
        LightCyan => (0, 255, 255),
        White => (255, 255, 255),
        Indexed(i @ 16..=231) => {
            let n = usize::from(i - 16);
            (CUBE[n / 36], CUBE[(n / 6) % 6], CUBE[n % 6])
        }
        Indexed(i @ 232..=255) => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
        _ => c,
    }
}

fn draw_matrix(dir: &std::path::Path, name: &str, world: &mut World) {
    let mut pb = PixelBuffer::new(120, 80);
    for (zoom, scale) in [(1.0, "1x"), (2.0, "2x")] {
        world.set_zoom_instant(zoom);
        for (depth, palette) in [
            ("rgb", ColorDepth::Truecolor),
            ("256", ColorDepth::Ansi256),
            ("16", ColorDepth::Ansi16),
        ] {
            world.lighting = LightingMode::default_for_depth(palette);
            world.lighting_depth = palette;
            world.draw(&mut pb);
            for pixel in pb.pixels_mut() {
                *pixel = terminal_rgb(*pixel, palette);
            }
            write_ppm(&dir.join(format!("{name}_{scale}_{depth}.ppm")), &pb);
        }
    }
}

fn write_ppm(path: &std::path::Path, pb: &PixelBuffer) {
    let mut data = Vec::with_capacity(32 + pb.w * pb.h * 3);
    use std::io::Write;
    writeln!(data, "P6\n{} {}\n255", pb.w, pb.h).unwrap();
    for &(r, g, b) in pb.pixels() {
        data.push(r);
        data.push(g);
        data.push(b);
    }
    std::fs::write(path, data).unwrap();
}

/// Gated behind `RATRUN_LIGHTING_CAPTURES=<dir>`: renders the visual review
/// gallery (busy frame on Sewer/Kitchen/Lab at Off/Soft/Full, boss fights,
/// nuke, 3-frame explosion smoke progression, low-HP vignette).
#[test]
fn capture_lighting_gallery() {
    let Some(dir) = std::env::var_os("RATRUN_LIGHTING_CAPTURES") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();

    use crate::game::bullet::Orbit;
    use crate::game::ecs::components::SpawnId;
    use crate::game::ecs::systems::boss::{Boss, BossKind};
    use crate::game::gem::{Gem, Pickup, PickupKind};
    use crate::game::level::{Prop, Rectf, TileRect};
    use crate::game::particle::vfx;
    use crate::game::weapon::WeaponKind;
    use rand::SeedableRng;

    let (vw, vh) = (160usize, 120usize);

    // Helper to build a rich, busy scene for a given theme with real props,
    // a doorway wall, enemies (including bats in dark corners), bullets,
    // orbiting spores, acid pool, pickup, gems, and live smoke.
    let build_busy = |theme: Theme| -> World {
        let mut w = World::new(theme, 17);
        let center = w.player.pos;
        // Place a partial wall with a doorway just east of the player so wall
        // occlusion and doorway light spill are clearly visible!
        let (pcx, pcy) = crate::game::collision::cell_of(center);
        w.level
            .tiles
            .fill_cells(pcx + 2, pcy - 3, 1, 7, TileKind::Wall(WallKind::Solid));
        w.level.tiles.set(
            pcx + 2,
            pcy,
            TileKind::Wall(WallKind::Doorway {
                axis: WallAxis::Vertical,
            }),
        );
        w.level.tiles.rebuild_masks();

        // Place theme-appropriate illuminated props inside view.
        #[allow(unreachable_patterns)]
        let prop_kinds = match theme {
            Theme::Sewer => [PropKind::Grate, PropKind::Valve],
            Theme::Kitchen => [PropKind::Counter, PropKind::Table],
            Theme::Lab => [PropKind::Console, PropKind::Tank],
            _ => [PropKind::Grate, PropKind::Valve],
        };
        let (pw0, ph0) = prop_kinds[0].size();
        let (pw1, ph1) = prop_kinds[1].size();
        w.level.props.push(Prop {
            kind: prop_kinds[0],
            rect: Rectf::new(center.x - 58.0, center.y - 36.0, pw0, ph0),
        });
        w.level.props.push(Prop {
            kind: prop_kinds[1],
            rect: Rectf::new(center.x - 52.0, center.y + 18.0, pw1, ph1),
        });

        let mut rng = rand::rngs::StdRng::seed_from_u64(99);
        // Enemies including bats in the dark corners behind the wall.
        for (kind, ofs) in [
            (EnemyKind::Skitterer, Vec2::new(-20.0, -14.0)),
            (EnemyKind::Skitterer, Vec2::new(-28.0, 12.0)),
            (EnemyKind::Cat, Vec2::new(14.0, -24.0)),
            (EnemyKind::Spitter, Vec2::new(-10.0, 26.0)),
            (EnemyKind::Brute, Vec2::new(18.0, 22.0)),
            (EnemyKind::Bat, Vec2::new(56.0, -26.0)),
            (EnemyKind::Bat, Vec2::new(62.0, 24.0)),
            (EnemyKind::Bat, Vec2::new(-64.0, -2.0)),
            (EnemyKind::Skitterer, Vec2::new(52.0, 2.0)),
        ] {
            w.spawn_enemy_with_rng(kind, center + ofs, 2.0, &mut rng);
        }
        // Acid pool, pickup, gems, player bullets, orbiting spores, enemy bullets.
        w.spawn_pool(Pool {
            pos: center + Vec2::new(-22.0, 20.0),
            radius: 12.0,
            dps: 6.0,
            life: 2.4,
            color: (150, 255, 90),
        });
        w.spawn_pickup(Pickup {
            pos: center + Vec2::new(12.0, -34.0),
            kind: PickupKind::Heal,
            bob: 1.1,
        });
        for ofs in [
            Vec2::new(-12.0, -22.0),
            Vec2::new(8.0, 16.0),
            Vec2::new(-38.0, 6.0),
        ] {
            w.spawn_gem(Gem::new(center + ofs, 25));
        }
        for (ang, wkind, col) in [
            (
                0.2f32,
                WeaponKind::CheeseSpray,
                WeaponKind::CheeseSpray.color(),
            ),
            (-0.4, WeaponKind::SqueakNova, WeaponKind::SqueakNova.color()),
            (2.4, WeaponKind::Gnaw, WeaponKind::Gnaw.color()),
        ] {
            let d = Vec2::from_angle(ang);
            let mut b = Bullet::new(center + d * 14.0, d * 90.0, 6.0, Faction::Player, col);
            b.weapon = Some(wkind);
            b.size = 2;
            w.spawn_bullet(b);
        }
        // Orbiting spores.
        for k in 0..3 {
            let a = k as f32 * 2.09;
            let mut b = Bullet::new(
                center + Vec2::from_angle(a) * 18.0,
                Vec2::ZERO,
                5.0,
                Faction::Player,
                WeaponKind::SporeOrbit.color(),
            );
            b.orbit = Some(Orbit {
                angle: a,
                radius: 18.0,
                speed: 3.0,
            });
            b.size = 2;
            w.spawn_bullet(b);
        }
        // Hostile bullets in both lit and shadowed areas (must stay self-lit!).
        for ofs in [
            Vec2::new(54.0, -14.0),
            Vec2::new(-16.0, 32.0),
            Vec2::new(48.0, 18.0),
        ] {
            let mut b = Bullet::new(
                center + ofs,
                Vec2::new(-40.0, 0.0),
                6.0,
                Faction::Enemy,
                (255, 75, 110),
            );
            b.size = 2;
            w.spawn_bullet(b);
        }
        // Pre-warm smoke emitters with a few ticks of continuous emission.
        w.smoke_mut().emit_dash_dust(
            center + Vec2::new(-6.0, 4.0),
            Vec2::new(1.0, 0.0),
            theme,
            1.0,
        );
        w.elapsed = 12.0;
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        for step in 0..10 {
            w.elapsed = 12.0 + step as f32 * 0.08;
            w.draw(&mut pb);
        }
        w
    };

    // Shared lighting gallery scene, at each playable scale and quality.
    for theme in Theme::playable() {
        let mut w = build_busy(theme);
        draw_matrix(dir, &format!("{theme:?}"), &mut w);

        let mut signature = World::new(theme, 17);
        if let Some(emitter) = signature.level.objects.emitters.first() {
            signature.player.pos = emitter.pos + Vec2::new(-12.0, 13.0);
            signature.elapsed = 12.24;
            draw_matrix(dir, &format!("{theme:?}_signature"), &mut signature);
        }
    }

    // 2. Boss fights: Lab Mower (headlamp + lidar scan + exhaust) & Sewer Cat (pounce tell).
    {
        let mut w = World::new(Theme::Lab, 33);
        let c = w.level.boss_room().unwrap().center();
        w.player.pos = c;
        let dock = c + Vec2::new(48.0, -28.0);
        w.level.boss.dock = Some(dock);
        let mut boss = Boss::new(
            BossKind::Mower,
            c + Vec2::new(-38.0, -8.0),
            1600.0,
            TileRect::new(90, 70, 20, 20),
            dock,
            false,
        );
        boss.facing = Vec2::new(1.0, 0.15).normalized();
        boss.state = BossState::Mower(MowerState::Scan);
        boss.duration = 1.1;
        boss.elapsed = 0.88; // locked amber-red lidar!
        boss.target = c + Vec2::new(8.0, 2.0);
        boss.phase = 2;
        w.ecs.spawn((boss, SpawnId(9001)));
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        for step in 0..8 {
            w.elapsed = 30.0 + step as f32 * 0.08;
            w.draw(&mut pb);
        }
        write_ppm(&dir.join("boss_mower_full.ppm"), &pb);
        w.lighting = LightingMode::Off;
        w.smoke_enabled = false;
        w.draw(&mut pb);
        write_ppm(&dir.join("boss_mower_off.ppm"), &pb);
        draw_matrix(dir, "arena_phase_2", &mut w);
    }
    {
        let mut w = World::new(Theme::Sewer, 44);
        let c = w.player.pos;
        let mut boss = Boss::new(
            BossKind::Cat,
            c + Vec2::new(-34.0, -14.0),
            2200.0,
            TileRect::new(90, 70, 20, 20),
            c,
            false,
        );
        boss.facing = Vec2::new(1.0, 0.35).normalized();
        boss.state = BossState::Cat(BossCatState::Crouch);
        boss.duration = 0.95;
        boss.elapsed = 0.78; // locked pounce warning ring on player!
        boss.target = c + Vec2::new(6.0, 4.0);
        boss.phase = 2;
        w.ecs.spawn((boss, SpawnId(9002)));
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        for step in 0..8 {
            w.elapsed = 40.0 + step as f32 * 0.08;
            w.draw(&mut pb);
        }
        write_ppm(&dir.join("boss_cat_full.ppm"), &pb);
    }

    // 3. Nuke capture (Off vs Full).
    {
        let mut w = build_busy(Theme::Sewer);
        let c = w.player.pos;
        vfx::emit_nuke(&mut w.particles, &mut w.rng, c);
        w.particles.update(0.06);
        w.elapsed += 0.06;
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        w.draw(&mut pb);
        write_ppm(&dir.join("nuke_full.ppm"), &pb);
        w.lighting = LightingMode::Off;
        w.smoke_enabled = false;
        w.draw(&mut pb);
        write_ppm(&dir.join("nuke_off.ppm"), &pb);
    }

    // 4. Explosion smoke progression over 3 frames (lit by fireball -> rising -> fading).
    {
        let mut w = World::new(Theme::Sewer, 55);
        let c = w.player.pos;
        let blast = c + Vec2::new(18.0, 2.0);
        vfx::emit_enemy_death(
            &mut w.particles,
            &mut w.rng,
            blast,
            6.5,
            EnemyKind::Brute,
            w.level.palette.blood,
        );
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        w.elapsed = 10.0;
        w.draw(&mut pb);
        write_ppm(&dir.join("smoke_exp_f1.ppm"), &pb);

        for _ in 0..14 {
            w.particles.update(1.0 / 60.0);
            w.elapsed += 1.0 / 60.0;
            w.draw(&mut pb);
        }
        write_ppm(&dir.join("smoke_exp_f2.ppm"), &pb);

        for _ in 0..24 {
            w.particles.update(1.0 / 60.0);
            w.elapsed += 1.0 / 60.0;
            w.draw(&mut pb);
        }
        write_ppm(&dir.join("smoke_exp_f3.ppm"), &pb);
    }

    // 5. Low-HP vignette capture.
    {
        let mut w = build_busy(Theme::Sewer);
        w.player.hp = w.player.max_hp() * 0.16;
        let mut pb = PixelBuffer::new(vw, vh);
        w.lighting = LightingMode::Full;
        w.smoke_enabled = true;
        w.draw(&mut pb);
        write_ppm(&dir.join("low_hp_full.ppm"), &pb);
    }
}

#[test]
fn capture_ambience_gallery() {
    use crate::game::collision::cell_of;
    use crate::game::level::{
        AmbientEmitter, EmitterKind, LightZone, LightZoneKind, Prop, Rectf, TileRect,
    };
    let Some(dir) = std::env::var_os("RATRUN_LIGHTING_CAPTURES") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let mut world = World::new(Theme::Sewer, 17);
    let center = world.level.start;
    let (cx, cy) = cell_of(center);
    world.level.objects.lights.clear();
    world.level.objects.light_zones.push(LightZone {
        rect: TileRect::new(cx - 10, cy - 8, 20, 16),
        kind: LightZoneKind::Blackout,
    });
    world.level.props.push(Prop {
        kind: PropKind::Crate,
        rect: Rectf::new(center.x + 4.0, center.y - 24.0, 16.0, 16.0),
    });
    world.level.objects.emitters.clear();
    let at = center + Vec2::new(-25.0, -4.0);
    let mut pb = PixelBuffer::new(90, 62);
    for kind in [
        EmitterKind::Fire,
        EmitterKind::Steam,
        EmitterKind::Drip,
        EmitterKind::Cable,
        EmitterKind::Dust,
        EmitterKind::Fog,
        EmitterKind::Exhaust,
        EmitterKind::Pilot,
        EmitterKind::Fumes,
        EmitterKind::Insects,
    ] {
        world.level.objects.emitters.clear();
        world.level.objects.emitters.push(AmbientEmitter {
            pos: at,
            kind,
            seed: 93,
            rate: 1.0,
            intensity: 1.0,
        });
        world.lighting_depth = ColorDepth::Truecolor;
        world.lighting = LightingMode::Full;
        world.set_zoom_instant(1.0);
        for i in 0..3 {
            world.elapsed = 12.0 + i as f32 * 0.31;
            world.draw(&mut pb);
            write_ppm(&dir.join(format!("emitter_{kind:?}_{i}.ppm")), &pb);
        }
        if kind == EmitterKind::Fire {
            draw_matrix(dir, "fire_shadow", &mut world);
        }
    }
    world.level.objects.emitters.clear();
    world.spawn_bullet(Bullet::new(
        center + Vec2::new(13.0, 0.0),
        Vec2::new(50.0, 0.0),
        2.0,
        Faction::Player,
        (247, 149, 68),
    ));
    draw_matrix(dir, "blackout_rat_and_flash", &mut world);
}
