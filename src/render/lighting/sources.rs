//! Render-only lights attached to actors, weapons and changing level props.

use crate::game::art_boss_broom::BroomBossState;
use crate::game::art_boss_cat::BossCatState;
use crate::game::art_boss_mangle::MangleState;
use crate::game::art_boss_mower::MowerState;
use crate::game::art_boss_night_edition::NightEditionState;
use crate::game::art_traps::MazeRewardState;
use crate::game::bullet::{Bullet, Faction};
use crate::game::ecs::components::{
    AcidDrop, CheeseString, CorrosiveChannel, DeadLetterPacket, EchoSpoke, FairyRing, Infection,
    Dropping, LavaGlob, LavaPool, MilkTooth, Pool, SpawnId,
};
use crate::game::ecs::resources::{ActiveFx, DashTrack, LaserFx};
use crate::game::ecs::systems::boss::{Boss, BossProjectile, BossProjectileKind, BossState};
use crate::game::ecs::systems::miniboss::{Miniboss, MinibossKind, MinibossState};
use crate::game::ecs::systems::powerups::Decoy;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Chest, Gem, Gold, Pickup};
use crate::game::level::PropKind;
use crate::game::weapon::{DROPPING_ARM, DROPPING_LIFE, DROPPING_LIGHT_ENERGY, DROPPING_LIGHT_RADIUS};
use crate::math::Vec2;

use super::*;

/// Enemy rim glints kept per frame: one pixel per visible body. Past this the
/// crowd is past what the eye separates anyway.
const MAX_RIMS: usize = 512;
const MAX_DROPPING_LIGHTS: usize = 12;
const DROPPING_DRY_SECS: f32 = 2.0;
// The green channel can reach the lighting ceiling when pools overlap. Keep
// the other channels low so overlapping spills retain their colour.
const GLOW_RGB: Rgb = (0, 255, 0);
const DROPPING_GLOW_REACH: f32 = 2.75;
const DROPPING_GLOW_GAIN: f32 = 10.0;

fn dropping_light(drop: &Dropping) -> Option<Light> {
    let left = DROPPING_LIFE - drop.age;
    if left <= 0.0 {
        return None;
    }
    let rank = usize::from(drop.level.clamp(1, 5) - 1);
    let arm = if drop.age < DROPPING_ARM { 0.5 } else { 1.0 };
    let fade = (left / DROPPING_DRY_SECS).min(1.0);
    let mut light = Light::radial(
        drop.pos,
        DROPPING_LIGHT_RADIUS[rank] * DROPPING_GLOW_REACH,
        GLOW_RGB,
        DROPPING_LIGHT_ENERGY[rank] * DROPPING_GLOW_GAIN * arm * fade,
    );
    light.falloff = Falloff::Small;
    light.height = 2.0;
    light.scatter = 0.0;
    light.casts_shadow = false;
    Some(light)
}

const LASER_LIGHT_SPACING: f32 = 12.0;
const LASER_LIGHT_RADIUS: f32 = 14.0;
const MAX_LASER_LIGHTS: usize = 16;

/// A stopped beam ray, shared by the light sampler and the water mirror.
#[derive(Clone, Copy)]
pub(super) struct LaserSegment {
    pub start: Vec2,
    pub end: Vec2,
    pub brightness: f32,
}

impl LaserSegment {
    fn new(start: Vec2, angle: f32, len: f32, brightness: f32) -> Option<Self> {
        (len > 0.0 && brightness > 0.02).then(|| Self {
            start,
            end: start + Vec2::from_angle(angle) * len,
            brightness,
        })
    }

    pub(super) fn lights(self) -> impl Iterator<Item = Light> {
        let ray = self.end - self.start;
        let count = ((ray.len() / LASER_LIGHT_SPACING).ceil() as usize + 1)
            .clamp(2, MAX_LASER_LIGHTS);
        (0..count).map(move |i| {
            let pos = self.start + ray * (i as f32 / (count - 1) as f32);
            let mut light = Light::radial(
                pos,
                LASER_LIGHT_RADIUS,
                crate::game::particle::vfx::LASER_GLOW,
                0.56 * self.brightness,
            );
            light.falloff = Falloff::Small;
            light.casts_shadow = false;
            light
        })
    }
}

pub(super) fn laser_segments(ecs: &bevy_ecs::world::World) -> [Option<LaserSegment>; 4] {
    let mut rays = [None; 4];
    if let Some(laser) = ecs.get_resource::<LaserFx>() {
        if laser.active() {
            let gain = laser.brightness();
            rays[0] = LaserSegment::new(laser.origin, laser.angle, laser.beam, gain);
            rays[1] = LaserSegment::new(laser.origin, laser.angle_b, laser.beam_b, gain);
        }
        if laser.copy_active() {
            if let Some(copy) = laser.copy {
                let gain = laser.copy_brightness();
                rays[2] = LaserSegment::new(
                    copy.origin, laser.copy_prev_angle, laser.copy_beams[0], gain,
                );
                rays[3] = LaserSegment::new(
                    copy.origin, laser.copy_prev_angle_b, laser.copy_beams[1], gain,
                );
            }
        }
    }
    rays
}

#[cfg(test)]
#[test]
fn laser_line_lights_cover_both_ends_with_bounded_samples() {
    let start = Vec2::new(10.0, 20.0);
    for (distance, expected_count) in [(3.0, 2), (36.0, 4), (180.0, 16), (320.0, 16)] {
        let end = start + Vec2::new(distance, 0.0);
        let ray = LaserSegment { start, end, brightness: 1.0 };
        let lights: Vec<_> = ray.lights().collect();
        assert_eq!(lights.len(), expected_count);
        assert_eq!(lights.first().unwrap().pos, start);
        assert_eq!(lights.last().unwrap().pos, end);
        for light in lights {
            assert!(matches!(light.shape, LightShape::Radial { radius } if radius == 14.0));
            assert!(light.falloff == Falloff::Small && !light.casts_shadow);
        }
    }
}

#[cfg(test)]
#[test]
fn laser_art_and_lights_use_the_moved_hit_segments() {
    use crate::game::art_walls::WallKind;
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::systems::weapons_wave3b::laser_eyes_system;
    use crate::game::level::{MAP_COLS, MAP_ROWS};
    use crate::game::weapon::{Weapon, WeaponKind, LASER_EYE_OFFSET, LASER_HALF_WIDTH};
    use crate::game::world::World;

    let mut w = World::new(Theme::Sewer, 31);
    w.clear_enemies();
    w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
    // The lower beam will meet the wall at x = 1088. The upper beam sees open
    // floor all the way to the view edge.
    w.level.tiles.fill_cells(68, 63, 1, 2, TileKind::Wall(WallKind::Solid));
    w.level.tiles.rebuild_masks();
    w.viewport = Vec2::new(64.0, 50.0);
    w.player.pos = Vec2::new(1000.0, 1000.0);
    w.player.face = Vec2::new(1.0, 0.0);
    w.player.loadout.weapons = vec![Weapon {
        kind: WeaponKind::LaserEyes,
        level: 1,
        timer: 0.0,
        evolved: false,
    }];

    let dt = 1.0 / 60.0;
    let moved = w.player.pos + Vec2::new(30.0, 0.0);
    let eye = moved + w.player.face * LASER_EYE_OFFSET;
    let half = w.player.loadout.weapons[0].laser_half_arc();
    let secs = w.player.loadout.weapons[0].laser_sweep_secs();
    let dir = Vec2::from_angle(half - half * (6.0 * dt / secs));
    // Beyond the old view edge, on the new lower beam and before the wall.
    let target = w.spawn_enemy(EnemyKind::Cat, eye + dir * 48.0, 1.0);
    let hp = w.ecs.get::<Enemy>(target).unwrap().hp;
    let tick = |w: &mut World| {
        w.begin_tick(dt, Vec2::ZERO, false);
        w.ecs.run_system_cached(laser_eyes_system).unwrap();
        w.end_tick();
    };
    for _ in 0..5 {
        tick(&mut w);
    }
    assert_eq!(w.ecs.get::<Enemy>(target).unwrap().hp, hp);

    w.player.pos = moved;
    tick(&mut w);
    let damage = hp - w.ecs.get::<Enemy>(target).unwrap().hp;
    assert!((damage - 6.0 * (28.0f32 / 48.0).powi(2)).abs() < 1e-3);

    let art = w.laser_visual().unwrap();
    assert_eq!(art.origin, eye);
    let rays = laser_segments(&w.ecs);
    for (ray, angle, len, stop_x) in [
        (rays[0], art.angle, art.len, moved.x + w.viewport.x),
        (rays[1], art.angle_b, art.len_b, 1088.0 - LASER_HALF_WIDTH),
    ] {
        let ray = ray.unwrap();
        assert_eq!(ray.start, eye);
        assert_eq!(ray.end, art.origin + Vec2::from_angle(angle) * len);
        assert!((ray.end.x - stop_x).abs() < 1e-3);
        let mut lights = ray.lights();
        assert_eq!(lights.next().unwrap().pos, ray.start);
        assert_eq!(lights.last().unwrap().pos, ray.end);
    }
    tick(&mut w);
    assert_eq!(hp - w.ecs.get::<Enemy>(target).unwrap().hp, damage);
}

const MAX_LAVA_LIGHTS: usize = 24;

impl LightMap {
    /// Gather all lights from world state; returns the active boss phase if any.
    pub(super) fn gather_lights(
        &mut self,
        pb_w: usize,
        pb_h: usize,
        scene: &LightScene<'_>,
    ) -> Option<u8> {
        let vw = pb_w as f32;
        let vh = pb_h as f32;
        let cam = scene.cam;
        let elapsed = scene.elapsed;
        let theme_prof = theme_ambient(scene.level.theme);

        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            // A small light throws its faint tail out to `TAIL_REACH` times its
            // stated radius; cull by the whole reach, not just the core.
            let r = r * TAIL_REACH;
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };

        // 1. Player rat soft personal aura (always present, priority 3).
        let px = scene.player.pos.x - cam.x;
        let py = scene.player.pos.y - cam.y;
        let rat_radius = match scene.level.theme {
            Theme::Lab => 58.0,
            Theme::Outfall => 54.0,
            _ => 66.0,
        };
        // With the effects on the pool is a smooth gradient, and a continuous
        // 3 % breath repainted all of it every frame: hold it in 4 Hz steps
        // (as the sconces do). Effects off keeps the batch-12 breath exactly.
        let breath_t = if scene.fx {
            (elapsed * 4.0).floor() * 0.25
        } else {
            elapsed
        };
        let rat_flicker = 1.0 + 0.03 * (breath_t * 4.3).sin();
        if scene.level.theme == Theme::Outfall {
            let energy = (184.0 * rat_flicker) as u16 as f32 / 256.0;
            let mut light =
                Light::radial(scene.player.pos, rat_radius, theme_prof.rat_tint, energy);
            light.scatter = 20.0 / 256.0;
            light.casts_shadow = true;
            light.falloff = Falloff::Smooth;
            // Restrained steady underwater beam scatter around the rat's
            // diving headlamp so drifting marine snow reads even in open halls
            // far from sconces, without breathing with the 4 Hz aura step.
            self.push_lantern_lure(light, (184.0 + 20.0) / 256.0, 0.28);
        } else {
            self.lights.push(PointLight {
                x: px,
                y: py,
                radius: rat_radius,
                color: theme_prof.rat_tint,
                intensity: ((match scene.level.theme {
                    Theme::Trailer => 62.0,
                    _ => 196.0,
                }) * rat_flicker) as u16,
                scatter: match scene.level.theme {
                    Theme::Trailer => 7,
                    _ => 22,
                },
                priority: 3,
                occlude: true,
                hard: false,
            });
        }

        let dash_scale = scene.player.buffs.draw_scale();
        // Dash cyan-white burst light.
        if scene.player.dash_active > 0.0 {
            let k = (scene.player.dash_active / crate::game::ecs::systems::player::DASH_SECS)
                .clamp(0.2, 1.0);
            self.lights.push(PointLight {
                x: px,
                y: py,
                radius: 36.0 * dash_scale,
                color: (165, 224, 255),
                intensity: (170.0 * k) as u16,
                scatter: (42.0 * k) as u16,
                priority: 3,
                occlude: false,
                hard: false,
            });
        }

        // Player hurt flash.
        if scene.player.hurt_flash > 0.0 {
            let k = (scene.player.hurt_flash / 0.25).clamp(0.0, 1.0);
            self.lights.push(PointLight {
                x: px,
                y: py,
                radius: 42.0,
                color: (255, 76, 86),
                intensity: (180.0 * k) as u16,
                scatter: (48.0 * k) as u16,
                priority: 3,
                occlude: false,
                hard: false,
            });
        }

        // 2. Active weapons: Tail Thrash arc & Sonic Squeak ring.
        if let Some((angle, reach, fade)) = scene.thrash {
            let dir = Vec2::from_angle(angle);
            let tip = scene.player.pos + dir * (reach * 0.62);
            let sx = tip.x - cam.x;
            let sy = tip.y - cam.y;
            let r = (reach * 0.95 + 16.0).clamp(24.0, 72.0);
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 178, 92),
                    intensity: (230.0 * fade) as u16,
                    scatter: (58.0 * fade) as u16,
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }
        if let Some((origin, ring_r)) = scene.squeak {
            let sx = origin.x - cam.x;
            let sy = origin.y - cam.y;
            let r = (ring_r + 22.0).clamp(24.0, 110.0);
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (140, 240, 255),
                    intensity: 205,
                    scatter: 50,
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }

        // 2b. Muzzle glows. The narrow bars are submitted separately so
        // the dynamic-light budget cannot clip their far ends.
        if let Some(laser) = scene.ecs.get_resource::<LaserFx>() {
            if laser.active() {
                let muzzle = laser.origin - cam;
                if in_range(muzzle.x, muzzle.y, 18.0) {
                    self.lights.push(PointLight {
                        x: muzzle.x,
                        y: muzzle.y,
                        radius: 18.0,
                        color: crate::game::particle::vfx::LASER_GLOW,
                        intensity: 108,
                        scatter: 16,
                        priority: 3,
                        occlude: false,
                        hard: false,
                    });
                }
            }
            if laser.copy_active() {
                if let Some(copy) = laser.copy {
                    let eye = copy.origin - cam;
                    if in_range(eye.x, eye.y, 28.0) {
                        let pulse = 0.80 + 0.20 * (elapsed * 18.0).sin().abs();
                        self.lights.push(PointLight {
                            x: eye.x,
                            y: eye.y,
                            radius: 28.0,
                            color: (255, 192, 58),
                            intensity: (190.0 * pulse) as u16,
                            scatter: (48.0 * pulse) as u16,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
        }

        // 2c. Roll Call active tags: electric-cyan glow on tagged hosts.
        if let Some(afx) = scene.ecs.get_resource::<ActiveFx>() {
            if afx.answer_left > 0.0 && afx.tag_n > 0 {
                let prog = 1.0
                    - (afx.answer_left / crate::game::ecs::systems::evolutions::ROLL_CALL_DELAY)
                        .clamp(0.0, 1.0);
                for ent in afx.tags.iter().take(afx.tag_n as usize).flatten().copied() {
                    let Some(e) = scene.ecs.get::<Enemy>(ent) else {
                        continue;
                    };
                    if !e.alive() {
                        continue;
                    }
                    let sx = e.pos.x - cam.x;
                    let sy = e.pos.y - cam.y;
                    if in_range(sx, sy, 20.0) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: 18.0,
                            color: (108, 238, 255),
                            intensity: (110.0 + 70.0 * prog) as u16,
                            scatter: (28.0 + 22.0 * prog) as u16,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
        }

        // 2d. Ripcord taut cord: cobalt-blue glow along the dash route + crisp
        //     ice-white highlights at the two white knots.
        if let Some(track) = scene.ecs.get_resource::<DashTrack>() {
            let route = track.route();
            if track.route_life > 0.0 && !route.is_empty() {
                let k =
                    (track.route_life / crate::game::ecs::resources::RIPCORD_SECS).clamp(0.25, 1.0);
                let start = track.route_start();
                let far = route[route.len() - 1].1;
                let mid_seg = route[route.len() / 2];
                let mid = (mid_seg.0 + mid_seg.1) * 0.5;
                for (p, col, r, int, sc) in [
                    (start, (148, 212, 255), 22.0, 155.0, 32.0),
                    (mid, (44, 134, 255), 28.0, 178.0, 38.0),
                    (far, (148, 212, 255), 22.0, 155.0, 32.0),
                ] {
                    let r = r * dash_scale;
                    let sx = p.x - cam.x;
                    let sy = p.y - cam.y;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: col,
                            intensity: (int * k) as u16,
                            scatter: (sc * k) as u16,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
        }

        // 3. Shockwaves (explosions, nukes, brute/boss deaths, TailWhip, SqueakNova, acid splashes).
        for sw in &scene.particles.shockwaves {
            let age = 1.0 - (sw.life / sw.max_life.max(1e-4)).clamp(0.0, 1.0);
            let fade = (1.0 - age) * (1.0 - age * 0.55);
            if fade <= 0.04 {
                continue;
            }
            let sx = sw.pos.x - cam.x;
            let sy = sw.pos.y - cam.y;
            let r = (sw.end_r * 1.30 + 18.0).clamp(20.0, 145.0);
            if !in_range(sx, sy, r) {
                continue;
            }
            let is_nuke = sw.end_r >= 75.0;
            let is_big = sw.end_r >= 20.0;
            // Keep nuke flash warm golden-orange so it illuminates dramatically without washing out.
            let col = if is_nuke {
                blend_rgb((255, 198, 92), sw.color, age * 0.65)
            } else {
                blend_rgb(sw.core, sw.color, age)
            };
            self.lights.push(PointLight {
                x: sx,
                y: sy,
                radius: r,
                color: col,
                intensity: ((if is_nuke {
                    235.0
                } else if is_big {
                    205.0
                } else {
                    155.0
                }) * fade) as u16,
                scatter: ((if is_nuke {
                    92.0
                } else if is_big {
                    64.0
                } else {
                    36.0
                }) * fade) as u16,
                priority: if is_nuke { 3 } else { 2 },
                occlude: is_big && !is_nuke,
                hard: false,
            });
        }

        // 4. Fixed level lights from props, dock pad, and unopened maze rewards.
        self.gather_fixed_lights(scene, vw, vh, theme_prof.hard_pools);

        // 5. ECS entities: pools, pickups, gems, bullets, enemies (bats/boss), bosses, boss projectiles.
        let mut boss_phase: Option<u8> = None;
        let mut dropping_lights = [None; MAX_DROPPING_LIGHTS];
        let mut lava = Vec::new();
        for entity in scene.ecs.iter_entities() {
            if let Some(bob) = entity.get::<crate::game::ecs::systems::weapons_wave5::PlumbBob>() {
                let radius = if bob.evolved { 18.0 } else { 12.0 };
                let s = bob.pos - cam;
                if in_range(s.x, s.y, radius) {
                    let mut light = Light::radial(bob.pos, radius, (255, 212, 110), 0.52);
                    light.height = 5.0;
                    light.falloff = Falloff::Small;
                    light.casts_shadow = false;
                    self.push_shaped(light, 1);
                }
            }
            if let Some(id) = entity.get::<SpawnId>() {
                let source = if let Some(glob) = entity.get::<LavaGlob>() {
                    let height = glob.height();
                    Some((glob.ground - Vec2::new(0.0, height), 18.0, height, false))
                } else {
                    entity.get::<LavaPool>().and_then(|pool| {
                        let radius = 14.0 * (pool.life - pool.age).clamp(0.0, 1.0);
                        (radius > 0.0).then_some((pool.pos, radius, 2.0, true))
                    })
                };
                if let Some((pos, radius, height, ground_glow)) = source {
                    let s = pos - cam;
                    if in_range(s.x, s.y, radius) {
                        let fade = if ground_glow { radius / 14.0 } else { 1.0 };
                        let mut light = Light::radial(pos, radius, (255, 140, 40), 0.66 * fade);
                        light.height = height;
                        light.scatter = 0.16 * fade;
                        light.falloff = Falloff::Small;
                        light.casts_shadow = false;
                        lava.push((ground_glow, id.0, light));
                    }
                }
            }
            // Acid pools (glowing toxic green).
            if let Some(p) = entity.get::<Pool>() {
                let sx = p.pos.x - cam.x;
                let sy = p.pos.y - cam.y;
                let r = p.radius * 2.1 + 12.0;
                if in_range(sx, sy, r) {
                    let k = (p.life / 2.6).clamp(0.25, 1.0);
                    let pulse = 0.90 + 0.10 * (elapsed * 4.5 + p.pos.x * 0.13).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: p.color,
                        intensity: (175.0 * k * pulse) as u16,
                        scatter: (44.0 * k * pulse) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Pickups (pulsing colored halos).
            if let Some(p) = entity.get::<Pickup>() {
                let sx = p.pos.x - cam.x;
                let sy = p.pos.y - cam.y;
                let pulse = 0.72 + 0.28 * (p.bob.sin() * 0.5 + 0.5);
                let r = 28.0 + pulse * 8.0;
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: p.kind.color(),
                        intensity: (185.0 * pulse) as u16,
                        scatter: (46.0 * pulse) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Gems (small glints; big gold gems glow slightly brighter).
            if let Some(g) = entity.get::<Gem>() {
                let sx = g.pos.x - cam.x;
                let sy = g.pos.y - cam.y;
                let big = g.value >= 25;
                let r = if big { 18.0 } else { 11.0 };
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: g.color,
                        intensity: if big { 130 } else { 80 },
                        scatter: if big { 26 } else { 14 },
                        priority: 0,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Bullets (hostile shots self-lit & high priority; player shots & spores colored).
            if let Some(b) = entity.get::<Bullet>() {
                let sx = b.pos.x - cam.x;
                let sy = b.pos.y - cam.y;
                if b.faction == Faction::Enemy {
                    let r = 22.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: brighten_rgb(b.color, 40),
                            intensity: 200,
                            scatter: 48,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if b.orbit.is_some() {
                    let r = 22.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: (125, 255, 180),
                            intensity: 155,
                            scatter: 34,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else {
                    // Muzzle flash boost when freshly spawned near the player.
                    let near_muzzle = b.pos.dist_sq(scene.player.pos) < 160.0;
                    let r = if near_muzzle {
                        24.0
                    } else if b.lob.is_some() {
                        20.0
                    } else {
                        15.0
                    };
                    if in_range(sx, sy, r) {
                        let col = b.weapon.map(|w| w.color()).unwrap_or(b.color);
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: col,
                            intensity: if near_muzzle { 170 } else { 115 },
                            scatter: if near_muzzle { 38 } else { 22 },
                            priority: if near_muzzle { 2 } else { 1 },
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }

            // Boss-owned projectiles (high-priority warning glows).
            if let Some(bp) = entity.get::<BossProjectile>() {
                let sx = bp.pos.x - cam.x;
                let sy = bp.pos.y - cam.y;
                let r = 26.0;
                if in_range(sx, sy, r) {
                    #[allow(unreachable_patterns)]
                    let col = match bp.kind {
                        BossProjectileKind::BladeDisc => (255, 145, 70),
                        BossProjectileKind::GrassShrapnel => (170, 255, 110),
                        BossProjectileKind::SonicFang => (255, 115, 185),
                        BossProjectileKind::DustpanShard => (255, 210, 120),
                        BossProjectileKind::RolledTowel => (252, 128, 108),
                        _ => scene.level.palette.accent,
                    };
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: col,
                        intensity: 215,
                        scatter: 54,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Legacy boss glow and hit-flash glint.
            if let Some(e) = entity.get::<Enemy>() {
                if !e.alive() {
                    continue;
                }
                let sx = e.pos.x - cam.x;
                let sy = e.pos.y - cam.y;
                self.gather_floor_tells(e, cam, vw, vh, elapsed);
                self.gather_rim(e, sx, sy, vw, vh);
                if e.kind == EnemyKind::Boss {
                    boss_phase = Some(boss_phase.unwrap_or(1));
                    let r = 136.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: (255, 90, 150),
                            intensity: 210,
                            scatter: 52,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if e.kind.archetype() == EnemyKind::WickMoth {
                    // Lit lantern tail of the Wick Moth: small flickering ember glow.
                    let face = if e.facing.len_sq() > 0.01 {
                        e.facing.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let tail = e.pos - face * (e.radius * 1.25);
                    let tx = tail.x - cam.x;
                    let ty = tail.y - cam.y;
                    let r = 20.0;
                    if in_range(tx, ty, r) {
                        let flick = 0.84 + 0.16 * (elapsed * 11.0 + e.anim * 2.0).sin();
                        self.lights.push(PointLight {
                            x: tx,
                            y: ty,
                            radius: r,
                            // A kind that borrows the Moth's fuse (the Ink
                            // Blot) burns in its own colour.
                            color: if e.kind == EnemyKind::WickMoth {
                                (255, 174, 78)
                            } else {
                                emissive_rgb(e.kind.color())
                            },
                            intensity: (145.0 * flick) as u16,
                            scatter: (30.0 * flick) as u16,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if e.kind.archetype() == EnemyKind::Turnkey
                    && e.w3.bar_t > 0.0
                    && !e.w3.bar_spent
                {
                    // Turnkey crossbar: subtle warning pulse while telling, crisp brass glow live.
                    let bx = e.w3.bar_pos.x - cam.x;
                    let by = e.w3.bar_pos.y - cam.y;
                    let telling = e.w3.bar_telling();
                    let r = if telling { 20.0 } else { 26.0 };
                    if in_range(bx, by, r) {
                        let pulse = 0.78 + 0.22 * (elapsed * 9.0 + e.anim).sin();
                        let base = if telling { 120.0 } else { 185.0 };
                        let brass = if telling {
                            (248, 196, 92)
                        } else {
                            (255, 224, 118)
                        };
                        self.lights.push(PointLight {
                            x: bx,
                            y: by,
                            radius: r,
                            // The Loose Bungee's bar reads in its own colour.
                            color: if e.kind == EnemyKind::Turnkey {
                                brass
                            } else {
                                blend_rgb(brass, emissive_rgb(e.kind.color()), 0.6)
                            },
                            intensity: (base * pulse) as u16,
                            scatter: ((if telling { 24.0 } else { 42.0 }) * pulse) as u16,
                            priority: if telling { 1 } else { 2 },
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if e.kind.archetype() == EnemyKind::Porter && e.w3.raise_t > 0.0 {
                    // Porter raising its sugar cube: soft mint-white healing beacon.
                    let r = 22.0;
                    if in_range(sx, sy - 4.0, r) {
                        let pulse = 0.75 + 0.25 * (elapsed * 8.0 + e.anim).sin();
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy - 4.0,
                            radius: r,
                            color: (186, 255, 214),
                            intensity: (148.0 * pulse) as u16,
                            scatter: (32.0 * pulse) as u16,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                } else if e.w3.curl_t > 0.0 || e.w3.flea_phase != 0 {
                    // A committed curl or a marked hop (Pillbug, Bell Flea and
                    // every kind on those bodies): a small pulse in the kind's
                    // own colour, so the commit reads in a dark room.
                    let r = 18.0;
                    if in_range(sx, sy, r) {
                        let pulse = 0.78 + 0.22 * (elapsed * 9.0 + e.anim).sin();
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: emissive_rgb(e.kind.color()),
                            intensity: (120.0 * pulse) as u16,
                            scatter: (24.0 * pulse) as u16,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }

            // Dead Wick Moth's burning fuse: flickering orange->white blast warning light.
            if let Some(wick) = entity.get::<crate::game::ecs::components::Wick>() {
                let wx = wick.pos.x - cam.x;
                let wy = wick.pos.y - cam.y;
                let total = crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS;
                let t = if total > 0.0 {
                    (1.0 - wick.fuse / total).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let r = 22.0 + 14.0 * t;
                if in_range(wx, wy, r) {
                    let strobe = 0.72 + 0.28 * (elapsed * (14.0 + 18.0 * t)).sin();
                    let col = blend_rgb((255, 148, 52), (255, 246, 206), t * 0.75);
                    self.lights.push(PointLight {
                        x: wx,
                        y: wy,
                        radius: r,
                        color: col,
                        intensity: ((168.0 + 62.0 * t) * strobe) as u16,
                        scatter: ((38.0 + 24.0 * t) * strobe) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Bosses (headlamps, lidar cones, attack tells, phase auras, hazards).
            if let Some(boss) = entity.get::<Boss>() {
                if !boss.dead {
                    boss_phase = Some(boss.phase.max(1));
                }
                self.gather_boss_lights(boss, cam, vw, vh, elapsed);
            }

            // Named minibosses: character-tinted presence & state glow on the
            // body plus self-lit telegraph light so arrival, wind-up, strike
            // and exposed recovery all read clearly in dark rooms.
            if let Some(mb) = entity.get::<Miniboss>() {
                if let Some(e) = entity.get::<Enemy>() {
                    if e.alive() {
                        let ex = e.pos.x - cam.x;
                        let ey = e.pos.y - cam.y;
                        let kind_col: Rgb = match mb.kind {
                            MinibossKind::Corkscrew => (255, 202, 96),
                            MinibossKind::SisterSnip => (208, 224, 255),
                            MinibossKind::Bobbin => (255, 118, 168),
                            MinibossKind::BellDiver => (114, 242, 218),
                            MinibossKind::Foreman => (255, 134, 86),
                            MinibossKind::MilkCarton => (238, 248, 196),
                            // The eighteen wave-four appointments
                            // (`WAVE4_ROSTERS.md`), each on its own body
                            // colour so a floor's four read apart in a dark
                            // room (`art_miniboss`'s palettes).
                            MinibossKind::ValveWidow => (214, 158, 58),
                            MinibossKind::Mudlark => (140, 112, 72),
                            MinibossKind::PastryWheel => (196, 208, 222),
                            MinibossKind::SaltHen => (240, 232, 216),
                            MinibossKind::PotBrush => (208, 178, 104),
                            MinibossKind::DosePump => (196, 228, 236),
                            MinibossKind::PetriMother => (214, 130, 180),
                            MinibossKind::CageCounter => (216, 166, 68),
                            MinibossKind::RotorTechnician => (110, 152, 184),
                            MinibossKind::SudsBailiff => (186, 226, 234),
                            MinibossKind::WringerBoy => (172, 194, 216),
                            MinibossKind::QuoinKey => (206, 216, 230),
                            MinibossKind::CarbonTwin => (132, 144, 166),
                            MinibossKind::LatePlate => (152, 132, 206),
                            MinibossKind::TarpaulinPup => (128, 144, 88),
                            MinibossKind::AxleNut => (168, 180, 198),
                            MinibossKind::ChainDog => (206, 136, 56),
                            MinibossKind::RoadAtlas => (122, 142, 164),
                            MinibossKind::Gasket => (126, 214, 220),
                            MinibossKind::Swell => (96, 206, 246),
                            MinibossKind::Shade => (124, 152, 208),
                            MinibossKind::FalseBulb => (92, 244, 234),
                        };
                        let pulse = 0.84 + 0.16 * (elapsed * 7.0).sin();
                        let entrance =
                            !mb.tell_cued && mb.origin == Vec2::ZERO && mb.timer < 1.15;
                        let (b_col, b_r, b_int, b_scat) = match mb.state {
                            // The wind-up glows in the kind's own colour;
                            // the strike is red and the opening mint for all.
                            MinibossState::Tell => (
                                emissive_rgb(kind_col),
                                36.0,
                                (195.0 * pulse) as u16,
                                (46.0 * pulse) as u16,
                            ),
                            MinibossState::Active => ((255, 102, 74), 42.0, 235, 60),
                            MinibossState::Recover => (
                                (136, 255, 196),
                                34.0,
                                (190.0 * pulse) as u16,
                                (48.0 * pulse) as u16,
                            ),
                            _ if entrance => (kind_col, 38.0, 210, 54),
                            _ => (kind_col, 30.0, 150, 30),
                        };
                        if in_range(ex, ey, b_r) {
                            self.lights.push(PointLight {
                                x: ex,
                                y: ey,
                                radius: b_r,
                                color: b_col,
                                intensity: b_int,
                                scatter: b_scat,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                        if let Some(shape) = mb.telegraph() {
                            let c = shape.center();
                            let sx = c.x - cam.x;
                            let sy = c.y - cam.y;
                            let active = mb.state == MinibossState::Active;
                            let r = if active { 48.0 } else { 38.0 };
                            if in_range(sx, sy, r) {
                                self.lights.push(PointLight {
                                    x: sx,
                                    y: sy,
                                    radius: r,
                                    color: if active {
                                        (255, 92, 66)
                                    } else {
                                        emissive_rgb(kind_col)
                                    },
                                    intensity: if active { 260 } else { 205 },
                                    scatter: if active { 68 } else { 44 },
                                    priority: 3,
                                    occlude: false,
                                    hard: false,
                                });
                            }
                        }
                    }
                }
            }

            // ---- Wave 3 lights: chests, gold, decoy, acid rain, virus, evolutions ----

            // Miniboss chest: warm golden beacon that draws the player from across the room.
            if let Some(ch) = entity.get::<Chest>() {
                let sx = ch.pos.x - cam.x;
                let sy = ch.pos.y - cam.y;
                let r = 44.0;
                if in_range(sx, sy, r) {
                    let pulse = 0.84 + 0.16 * (elapsed * 3.4).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (255, 214, 78),
                        intensity: (210.0 * pulse) as u16,
                        scatter: (54.0 * pulse) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Gold coins: small warm golden glint.
            if let Some(gold) = entity.get::<Gold>() {
                let sx = gold.pos.x - cam.x;
                let sy = gold.pos.y - cam.y;
                let big = gold.value >= 5;
                let r = if big { 17.0 } else { 13.0 };
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (255, 206, 68),
                        intensity: if big { 132 } else { 94 },
                        scatter: if big { 28 } else { 18 },
                        priority: 0,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Wind-up Mouse floor decoy: warm pulsing brass lure beacon.
            if let Some(decoy) = entity.get::<Decoy>() {
                let sx = decoy.pos.x - cam.x;
                let sy = decoy.pos.y - cam.y;
                let r = 36.0;
                if in_range(sx, sy, r) {
                    let pulse = 0.78 + 0.22 * (elapsed * 5.2).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (244, 196, 86),
                        intensity: (185.0 * pulse) as u16,
                        scatter: (44.0 * pulse) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Acid Rain drops: caustic green ground glow intensifying during fall.
            if let Some(drop) = entity.get::<AcidDrop>() {
                let sx = drop.pos.x - cam.x;
                let sy = drop.pos.y - cam.y;
                let fall = ((crate::game::weapon::RAIN_FALL - drop.delay)
                    / crate::game::weapon::RAIN_FALL)
                    .clamp(0.0, 1.0);
                let r = drop.radius * 1.9 + 10.0;
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (168, 255, 62),
                        intensity: (110.0 + 105.0 * fall) as u16,
                        scatter: (26.0 + 36.0 * fall) as u16,
                        priority: if fall > 0.5 { 2 } else { 1 },
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Hantavirus droppings: steady green floor light.
            if let Some(drop) = entity.get::<Dropping>() {
                if let Some((light, id)) = dropping_light(drop).zip(entity.get::<SpawnId>()) {
                    let pos = light.pos - cam;
                    if in_range(pos.x, pos.y, light.shape.extent()) {
                        // Keep the newest visible lights in a fixed-size list.
                        let slot = dropping_lights.partition_point(|entry| {
                            entry.is_some_and(|(prior, _)| prior > id.0)
                        });
                        if slot < MAX_DROPPING_LIGHTS {
                            dropping_lights[slot..].rotate_right(1);
                            dropping_lights[slot] = Some((id.0, light));
                        }
                    }
                }
            }

            // Hantavirus infected hosts: sickly plum-magenta glow.
            if let (Some(e), Some(inf)) = (entity.get::<Enemy>(), entity.get::<Infection>()) {
                if e.alive() && inf.active() {
                    let sx = e.pos.x - cam.x;
                    let sy = e.pos.y - cam.y;
                    let r = 20.0;
                    if in_range(sx, sy, r) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color: (224, 92, 206),
                            intensity: 135,
                            scatter: 34,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }

            // Milk Teeth dart: warm ivory glint.
            if let Some(dart) = entity.get::<MilkTooth>() {
                let sx = dart.pos.x - cam.x;
                let sy = dart.pos.y - cam.y;
                let r = 18.0;
                if in_range(sx, sy, r) {
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (255, 244, 206),
                        intensity: 145,
                        scatter: 32,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Fondue Fork string: warm melted-cheddar glow at midpoint.
            if let Some(line) = entity.get::<CheeseString>() {
                let mid = (line.a + line.b) * 0.5;
                let sx = mid.x - cam.x;
                let sy = mid.y - cam.y;
                let r = (line.a.dist(line.b) * 0.65 + 14.0).clamp(18.0, 42.0);
                if in_range(sx, sy, r) {
                    let k = (line.life / line.total.max(1e-4)).clamp(0.2, 1.0);
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (255, 212, 74),
                        intensity: (155.0 * k) as u16,
                        scatter: (38.0 * k) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Fairy Ring: bioluminescent mint aura around the mushroom circle.
            if let Some(ring) = entity.get::<FairyRing>() {
                let sx = ring.pos.x - cam.x;
                let sy = ring.pos.y - cam.y;
                let r = ring.radius + 16.0;
                if in_range(sx, sy, r) {
                    let k = (ring.life / ring.total.max(1e-4)).clamp(0.25, 1.0);
                    let pulse = 0.88 + 0.12 * (elapsed * 4.0).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (114, 255, 182),
                        intensity: (168.0 * k * pulse) as u16,
                        scatter: (40.0 * k * pulse) as u16,
                        priority: 1,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Drain Cleaner channel / Downspout stepped runoff: caustic green glow.
            if let Some(ch) = entity.get::<CorrosiveChannel>() {
                if ch.runoff {
                    let total = ch.settle.max(0.05);
                    let k = ((total - ch.age) / 0.35).clamp(0.25, 1.0);
                    let end = ch.start + ch.dir * ch.length;
                    let mid = ch.start + ch.dir * (ch.length * 0.5);
                    for p in [ch.start, mid, end] {
                        let sx = p.x - cam.x;
                        let sy = p.y - cam.y;
                        if in_range(sx, sy, 28.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 26.0,
                                color: (176, 255, 92),
                                intensity: (175.0 * k) as u16,
                                scatter: (46.0 * k) as u16,
                                priority: 2,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                } else {
                    let total = crate::game::ecs::systems::evolutions::CHANNEL_ADVANCE
                        + crate::game::ecs::systems::evolutions::CHANNEL_SETTLE;
                    let k = ((total - ch.age) / 0.5).clamp(0.2, 1.0);
                    let open_len = ch.length
                        * (ch.age / crate::game::ecs::systems::evolutions::CHANNEL_ADVANCE)
                            .clamp(0.0, 1.0);
                    let front = ch.start + ch.dir * open_len;
                    let mid = ch.start + ch.dir * (open_len * 0.5);
                    for p in [mid, front] {
                        let sx = p.x - cam.x;
                        let sy = p.y - cam.y;
                        if in_range(sx, sy, 26.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 26.0,
                                color: (166, 255, 72),
                                intensity: (165.0 * k) as u16,
                                scatter: (44.0 * k) as u16,
                                priority: 1,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                }
            }

            // Dead Letter sealed envelope: pulsing vivid magenta wax-seal beacon.
            if let Some(pkt) = entity.get::<DeadLetterPacket>() {
                let sx = pkt.pos.x - cam.x;
                let sy = pkt.pos.y - cam.y;
                let r = 26.0;
                if in_range(sx, sy, r) {
                    let lf = (pkt.life / pkt.total.max(1e-4)).clamp(0.0, 1.0);
                    let pulse = 0.78 + 0.22 * (elapsed * (6.0 + 10.0 * (1.0 - lf))).sin().abs();
                    let k = (0.45 + 0.55 * lf) * pulse;
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (242, 88, 210),
                        intensity: (188.0 * k) as u16,
                        scatter: (48.0 * k) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }

            // Echo Chamber reflected inward spoke: crisp lavender-white glint.
            if let (Some(b), Some(spoke)) = (entity.get::<Bullet>(), entity.get::<EchoSpoke>()) {
                if spoke.reflected {
                    let sx = b.pos.x - cam.x;
                    let sy = b.pos.y - cam.y;
                    if in_range(sx, sy, 18.0) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: 18.0,
                            color: (238, 224, 255),
                            intensity: 155,
                            scatter: 38,
                            priority: 1,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
        }

        for (_, light) in dropping_lights.into_iter().flatten() {
            self.push_ground(light, 2);
        }
        // Keep airborne lights first, then the most recently spawned pools.
        // This cap is independent of the lighting tier's overall budget.
        lava.sort_unstable_by_key(|&(pool, id, _)| (pool, std::cmp::Reverse(id)));
        for (pool, _, light) in lava.into_iter().take(MAX_LAVA_LIGHTS) {
            if pool {
                self.push_ground(light, 1);
            } else {
                self.push_shaped(light, 1);
            }
        }
        boss_phase
    }

    /// A constant one-pixel rim glint on a living body, in the kind's own
    /// colour. Ordinary enemies are drawn before the lighting pass, so a body
    /// standing outside every pool would otherwise be part of the dark floor.
    /// The glint is a mark, so it survives the darkness at every colour depth
    /// and costs one pixel per enemy.
    fn gather_rim(&mut self, e: &Enemy, sx: f32, sy: f32, vw: f32, vh: f32) {
        if self.rims.len() >= MAX_RIMS {
            return;
        }
        let (x, y) = (
            sx.round() as i32,
            (sy - e.radius * 0.6).round() as i32,
        );
        if x < 0 || y < 0 || x >= vw as i32 || y >= vh as i32 {
            return;
        }
        self.rims.push((x, y, rim_glint(e.kind.color())));
    }

    /// Light for the floor kinds' own warned attacks, read from the state each
    /// `floor_*` system mirrors onto the body: the Loose Can's lane, the Sump
    /// Leech's tongue, the Walking Syringe's ray, the Egg Timer's ring, the Peg
    /// Snapper's jaws and the Staple Crawler's feet. A warning pulses in the
    /// kind's own colour; the live strike is the shared hostile red.
    fn gather_floor_tells(&mut self, e: &Enemy, cam: Vec2, vw: f32, vh: f32, elapsed: f32) {
        use crate::game::ecs::systems::{
            floor_flatbed, floor_kitchen, floor_lab, floor_laundry, floor_outfall,
            floor_printworks, floor_sewer,
        };
        const LIVE: Rgb = (255, 92, 66);
        let own = emissive_rgb(e.kind.color());
        let pulse = 0.80 + 0.20 * (elapsed * 9.0 + e.anim).sin();
        let mut pool = |at: Vec2, radius: f32, live: bool, strength: f32| {
            let (sx, sy) = (at.x - cam.x, at.y - cam.y);
            if sx + radius >= 0.0 && sy + radius >= 0.0 && sx - radius < vw && sy - radius < vh {
                let (color, intensity, scatter) = if live {
                    (LIVE, 215.0 * strength, 50.0 * strength)
                } else {
                    (own, 150.0 * strength * pulse, 30.0 * strength * pulse)
                };
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius,
                    color,
                    intensity: intensity as u16,
                    scatter: scatter as u16,
                    priority: 2,
                    occlude: false,
                    hard: false,
                });
            }
        };
        match e.kind {
            EnemyKind::LooseCan => {
                if let Some(path) = floor_flatbed::can_path(e) {
                    let live = e.w4.phase == floor_flatbed::CAN_PHASE_ROLL;
                    pool((path.points[0] + path.points[1]) * 0.5, 24.0, live, 1.0);
                    if path.bounced {
                        pool((path.points[1] + path.points[2]) * 0.5, 24.0, live, 1.0);
                    }
                }
            }
            EnemyKind::SumpLeech => {
                if let Some((lip, tip, phase, progress)) = floor_sewer::leech_tongue(e) {
                    let live = phase == floor_sewer::LEECH_PHASE_HOLD;
                    pool((lip + tip) * 0.5, 20.0, live, 0.6 + 0.4 * progress);
                }
            }
            EnemyKind::WalkingSyringe => {
                if let Some(ray) = floor_lab::syringe_ray(e) {
                    pool((ray.from + ray.to) * 0.5, 20.0, ray.live, 0.6 + 0.4 * ray.progress);
                }
            }
            EnemyKind::EggTimer => {
                if let Some((centre, radius)) = floor_kitchen::egg_ring_of(e) {
                    pool(centre, (radius + 12.0).min(90.0), true, 0.8);
                } else if let Some((centre, progress)) = floor_kitchen::egg_tell_of(e) {
                    pool(centre, 20.0 + 8.0 * progress, false, 0.6 + 0.4 * progress);
                }
            }
            EnemyKind::PegSnapper => {
                if let Some(view) = floor_laundry::jaw_view(e) {
                    let live = view.phase == floor_laundry::PEG_PHASE_CLOSE;
                    for jaw in view.jaws() {
                        pool(jaw, 16.0, live, 1.0);
                    }
                }
            }
            EnemyKind::StapleCrawler
                if floor_printworks::staple_warned(e) => {
                    let live = e.w4.phase == floor_printworks::STAPLE_PHASE_STRIKE;
                    for foot in [e.w4.trail[0], e.w4.trail[1]] {
                        pool(foot, 15.0, live, 1.0);
                    }
                }
            EnemyKind::Lampfish => {
                if let Some(bulb) = floor_outfall::lampfish_bulb(e) {
                    let sx = bulb.x - cam.x;
                    let sy = bulb.y - cam.y;
                    let attacking = floor_outfall::outfall_enemy_tell(e).is_some();
                    let r = if attacking { 28.0 } else { 26.0 };
                    let reach = r * TAIL_REACH;
                    if sx + reach >= 0.0
                        && sy + reach >= 0.0
                        && sx - reach < vw
                        && sy - reach < vh
                    {
                        let (color, intensity, scatter) = if attacking {
                            // 3 Hz held-step attack flicker; moving small halo leaves air steady.
                            let held = (elapsed * 3.0).floor() / 3.0;
                            let flick = 0.72 + 0.28 * (held * 19.0 + e.anim).sin().abs();
                            ((148, 252, 255), (220.0 * flick) as u16, (52.0 * flick) as u16)
                        } else {
                            ((84, 238, 236), 168, 36)
                        };
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius: r,
                            color,
                            intensity,
                            scatter,
                            priority: 2,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
            EnemyKind::Skater if floor_outfall::outfall_enemy_tell(e).is_some() => {
                pool(e.pos, 18.0, false, 0.75);
            }
            _ => {}
        }
    }

    /// Fixed lights derived from existing themed props, Lab charging dock, and
    /// unopened maze reward pedestals.
    fn gather_fixed_lights(&mut self, scene: &LightScene<'_>, vw: f32, vh: f32, hard_pools: bool) {
        let cam = scene.cam;
        let elapsed = scene.elapsed;
        // Small prop lights reach past their stated radius (see `Falloff::Small`).
        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            let r = r * TAIL_REACH;
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };

        for prop in &scene.level.props {
            let c = prop.rect.center();
            let sx = c.x - cam.x;
            let sy = c.y - cam.y;
            let h = pos_hash(prop.rect.x as i32, prop.rect.y as i32);

            #[allow(unreachable_patterns)]
            let spec: Option<(f32, Rgb, u16, u16, bool)> = match prop.kind {
                // Lab computer screens: crisp cyan pools of light with subtle CRT flicker.
                PropKind::Console => {
                    let flick = 0.94 + 0.06 * (elapsed * 7.0 + (h & 31) as f32).sin();
                    Some((
                        48.0,
                        (96, 220, 255),
                        (142.0 * flick) as u16,
                        (38.0 * flick) as u16,
                        true,
                    ))
                }
                // Lab bio-specimen tanks: bioluminescent emerald-cyan pools.
                PropKind::Tank => {
                    let pulse = 0.90 + 0.10 * (elapsed * 2.4 + (h & 63) as f32).sin();
                    Some((
                        44.0,
                        (84, 240, 192),
                        (136.0 * pulse) as u16,
                        (36.0 * pulse) as u16,
                        true,
                    ))
                }
                // Sewer grates: eerie sub-floor teal vent glow.
                PropKind::Grate => {
                    let pulse = 0.88 + 0.12 * (elapsed * 1.9 + (h & 31) as f32).sin();
                    Some((
                        42.0,
                        (84, 215, 185),
                        (132.0 * pulse) as u16,
                        (30.0 * pulse) as u16,
                        false,
                    ))
                }
                // Sewer valves: warm industrial gauge lamp.
                PropKind::Valve => {
                    let pulse = 0.92 + 0.08 * (elapsed * 3.1 + (h & 31) as f32).sin();
                    Some((
                        34.0,
                        (235, 175, 96),
                        (124.0 * pulse) as u16,
                        (28.0 * pulse) as u16,
                        false,
                    ))
                }
                // Kitchen counters: warm overhead range/hood light (keeps stainless steel crisp).
                PropKind::Counter => Some((52.0, (255, 202, 126), 118, 28, true)),
                // Kitchen tables: warm diner lamp on ~2/3 of tables for pools of light & shade.
                PropKind::Table if !h.is_multiple_of(3) => {
                    Some((44.0, (255, 192, 114), 108, 24, true))
                }
                // Kitchen barrels: occasional warm lantern glint.
                PropKind::Barrel if h & 1 == 0 => Some((30.0, (245, 174, 84), 96, 20, false)),
                // Barriers: small pulsing amber hazard beacon.
                PropKind::Barrier => {
                    let blink = 0.65 + 0.35 * (elapsed * 3.8 + (h & 15) as f32).sin().abs();
                    Some((
                        28.0,
                        (255, 182, 68),
                        (118.0 * blink) as u16,
                        (28.0 * blink) as u16,
                        false,
                    ))
                }
                // Laundry washer/dryer banks emit separate cool (washer) and
                // warm (dryer) bay glows below so each half matches its unit.
                PropKind::Washer => None,
                // Printworks press islands: warm illuminated Press Clock pool
                // over the press bed (twin cyan registration lamp added below).
                // `occlude: false` so the press's own solid tiles never slice
                // through the clock's warm halo.
                PropKind::PressIsland => {
                    let pulse = 0.94 + 0.06 * (elapsed * 1.6 + (h & 31) as f32).sin();
                    Some((
                        50.0,
                        (252, 224, 162),
                        (142.0 * pulse) as u16,
                        (36.0 * pulse) as u16,
                        false,
                    ))
                }
                // Printworks display type blocks: cool cyan registration-table lamp
                // on alternating blocks so press halls have crisp pools of light.
                PropKind::TypeBlock if (h & 1) == 0 => {
                    let pulse = 0.92 + 0.08 * (elapsed * 2.1 + (h & 31) as f32).sin();
                    Some((
                        36.0,
                        (88, 204, 212),
                        (112.0 * pulse) as u16,
                        (28.0 * pulse) as u16,
                        false,
                    ))
                }
                _ => None,
            };

            if let Some((radius, color, intensity, scatter, occlude)) = spec {
                let (lx, ly) = match prop.kind {
                    // Center press-clock light over the upper press-bed dial.
                    PropKind::PressIsland => (sx - 2.0, sy - 6.0),
                    _ => (sx, sy),
                };
                if in_range(lx, ly, radius) {
                    self.lights.push(PointLight {
                        x: lx,
                        y: ly,
                        radius,
                        color,
                        intensity,
                        scatter,
                        priority: 1,
                        occlude,
                        hard: hard_pools,
                    });
                }
            }

            // Laundry washer/dryer banks: soft cool aqua glow from the washer
            // porthole and soft warm amber glow from the dryer porthole.
            if prop.kind == PropKind::Washer {
                let px = prop.rect.x.round() as i32;
                let py = prop.rect.y.round() as i32;
                let units = crate::game::art_props::laundry_bank_units(px, py);
                let ly = sy + 6.0;
                let r = 38.0;
                for (bay, &unit) in units.iter().enumerate() {
                    let lx = prop.rect.x + prop.rect.w * if bay == 0 { 0.25 } else { 0.75 } - cam.x;
                    if !in_range(lx, ly, r) {
                        continue;
                    }
                    let phase = crate::game::art_props::laundry_unit_phase(px, py, bay);
                    let (color, intensity, scatter) = match unit {
                        crate::game::art_props::LaundryUnitKind::Washer => {
                            let tumble = 0.88 + 0.12 * (elapsed * 3.0 + phase).sin();
                            ((132, 214, 232), (118.0 * tumble) as u16, (30.0 * tumble) as u16)
                        }
                        crate::game::art_props::LaundryUnitKind::Dryer => {
                            let hum = 0.92 + 0.08 * (elapsed * 5.2 + phase).sin();
                            ((248, 178, 104), (122.0 * hum) as u16, (32.0 * hum) as u16)
                        }
                    };
                    self.lights.push(PointLight {
                        x: lx,
                        y: ly,
                        radius: r,
                        color,
                        intensity,
                        scatter,
                        priority: 1,
                        occlude: false,
                        hard: hard_pools,
                    });
                }
            }

            // Secondary registration-cyan lamp on the front-right control rail
            // of every Printworks PressIsland.
            if prop.kind == PropKind::PressIsland {
                let rx = prop.rect.x + prop.rect.w - 9.0 - cam.x;
                let ry = prop.rect.y + prop.rect.h - 6.0 - cam.y;
                let r = 34.0;
                if in_range(rx, ry, r) {
                    let blink = 0.84 + 0.16 * (elapsed * 3.4 + (h & 15) as f32).sin();
                    self.lights.push(PointLight {
                        x: rx,
                        y: ry,
                        radius: r,
                        color: (84, 218, 224),
                        intensity: (132.0 * blink) as u16,
                        scatter: (34.0 * blink) as u16,
                        priority: 1,
                        occlude: false,
                        hard: true,
                    });
                }
            }
        }

        // Laundry steam grates: boiler under-vent glow while resting, pale
        // steam catching light as the three warning puffs rise, and a bright
        // luminous column glow during the active scalding plume.
        for grate in &scene.level.objects.steam_grates {
            let sx = grate.pos.x - cam.x;
            let sy = grate.pos.y - cam.y;
            if grate.plume() {
                let r = 48.0;
                let ly = sy - 10.0;
                if in_range(sx, ly, r) {
                    let flicker = 0.88 + 0.12 * (elapsed * 14.0 + grate.pos.x * 0.1).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: ly,
                        radius: r,
                        color: (226, 246, 255),
                        intensity: (215.0 * flicker) as u16,
                        scatter: (62.0 * flicker) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            } else if grate.warning() {
                let t = grate.phase.progress(grate.timer);
                let r = 28.0 + 12.0 * t;
                let ly = sy - 4.0 - 6.0 * t;
                if in_range(sx, ly, r) {
                    let pulse = 0.78 + 0.22 * (t * std::f32::consts::TAU * 3.0).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: ly,
                        radius: r,
                        color: (238, 230, 206),
                        intensity: ((136.0 + 48.0 * t) * pulse) as u16,
                        scatter: ((32.0 + 18.0 * t) * pulse) as u16,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            } else {
                let r = 20.0;
                if in_range(sx, sy, r) {
                    let breath = 0.85 + 0.15 * (elapsed * 1.8 + grate.pos.x * 0.09).sin();
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: r,
                        color: (186, 152, 98),
                        intensity: (82.0 * breath) as u16,
                        scatter: (18.0 * breath) as u16,
                        priority: 0,
                        occlude: false,
                        hard: false,
                    });
                }
            }
        }

        // Printworks ink pads: subtle wet registration-cyan & plum catchlight
        // so sticky ink pools glint visibly before the rat steps in them.
        for pad in &scene.level.objects.ink_pads {
            let sx = pad.pos.x - cam.x;
            let sy = pad.pos.y - cam.y;
            let r = (pad.half.x.max(pad.half.y) + 16.0).clamp(22.0, 36.0);
            if in_range(sx, sy, r) {
                let sheen = 0.86 + 0.14 * (elapsed * 1.4 + (pad.seed & 31) as f32).sin();
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (116, 146, 178),
                    intensity: (96.0 * sheen) as u16,
                    scatter: (22.0 * sheen) as u16,
                    priority: 0,
                    occlude: false,
                    hard: false,
                });
            }
        }

        // Outfall lure traps: cyan ground bulb while Ready, 3 Hz held-step
        // warning pulse while armed, completely dark once Spent. Submitted via
        // `push_ground` so stationary bulbs drive local relief and caustics
        // without shadow bakes or fixture mirrors.
        for trap in &scene.level.objects.lure_traps {
            use crate::game::hazards::LurePhase;
            let (radius, color, energy, scatter, height) = match &trap.phase {
                LurePhase::Ready => (40.0, (78, 242, 232), 0.72, 0.16, 7.0),
                LurePhase::Warning => {
                    let held = (elapsed * 3.0).floor() / 3.0;
                    let flick = 0.80 + 0.20 * (held * 18.0 + trap.pos.x * 0.17).sin().abs();
                    (46.0, (128, 252, 255), 0.96 * flick, 0.24 * flick, 8.0)
                }
                LurePhase::Spent => continue,
            };
            let sx = trap.pos.x - cam.x;
            let sy = trap.pos.y - cam.y;
            if in_range(sx, sy, radius) {
                let mut light = Light::radial(trap.pos, radius, color, energy);
                light.scatter = scatter;
                light.height = height;
                light.falloff = Falloff::Small;
                light.casts_shadow = false;
                self.push_ground(light, 2);
            }
        }

        // Lab boss charging dock pad glow.
        if let Some(dock) = scene.level.boss.dock {
            let sx = dock.x - cam.x;
            let sy = dock.y - cam.y;
            let r = 46.0;
            if in_range(sx, sy, r) {
                let pulse = 0.82 + 0.18 * (elapsed * 3.2).sin();
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (100, 242, 215),
                    intensity: (195.0 * pulse) as u16,
                    scatter: (44.0 * pulse) as u16,
                    priority: 1,
                    occlude: false,
                    hard: hard_pools,
                });
            }
        }

        // A mouse-hole shop: a small steady candle-warm pool on its threshold.
        for hole in &scene.level.objects.shop_holes {
            let at = hole.face();
            let (sx, sy) = (at.x - cam.x, at.y - cam.y);
            let r = 36.0;
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 196, 120),
                    intensity: 130,
                    scatter: 28,
                    priority: 1,
                    occlude: false,
                    hard: false,
                });
            }
        }

        // Unopened maze reward pedestals emit a warm golden treasure glint.
        for reward in &scene.level.objects.rewards {
            if reward.state == MazeRewardState::Opened {
                continue;
            }
            let sx = reward.pos.x - cam.x;
            let sy = reward.pos.y - cam.y;
            let r = 44.0;
            if in_range(sx, sy, r) {
                let pulse = 0.85 + 0.15 * (elapsed * 3.6).sin();
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 215, 92),
                    intensity: (190.0 * pulse) as u16,
                    scatter: (42.0 * pulse) as u16,
                    priority: 1,
                    occlude: false,
                    hard: false,
                });
            }
        }
    }

    /// Lights for living/dying bosses: mower headlamp & lidar, cat eyes & pounce
    /// tell, broom slam/sweep tells, The Lantern's real/decoy lures, and all
    /// active `BossHazard` warnings.
    fn gather_boss_lights(&mut self, boss: &Boss, cam: Vec2, vw: f32, vh: f32, elapsed: f32) {
        // The Lantern owns its darkness: skip the generic 168 px presence aura
        // so Arrival, DarkTell and Death stay abyssal around the cyan lure.
        if let Some(view) = boss.lantern_view() {
            self.gather_lantern_lights(&view, boss, cam, vw, vh, elapsed);
            return;
        }

        // A boss's own small tells and halos reach past their stated radius.
        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            let r = r * TAIL_REACH;
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };
        let bsx = boss.pos.x - cam.x;
        let bsy = boss.pos.y - cam.y;
        let face = if boss.facing.len_sq() > 0.01 {
            boss.facing.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let progress = if boss.duration > 0.001 {
            (boss.elapsed / boss.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let tint = match boss.phase {
            0 | 1 => (238, 113, 73),
            2 => (252, 75, 118),
            _ => (202, 75, 255),
        };
        if in_range(bsx, bsy, 168.0) {
            self.lights.push(PointLight {
                x: bsx,
                y: bsy,
                radius: 168.0,
                color: tint,
                intensity: 85,
                scatter: 23,
                priority: 3,
                occlude: true,
                hard: false,
            });
        }

        #[allow(unreachable_patterns)]
        match boss.state {
            BossState::Mower(st) => {
                // Mower headlamp / lidar beam projecting ahead of the chassis.
                let (lamp_col, lamp_r, lamp_int, lamp_sc) = match st {
                    MowerState::Patrol => ((155, 232, 255), 54.0, 210u16, 48u16),
                    MowerState::Scan => {
                        if progress >= 0.70 {
                            ((255, 105, 58), 64.0, 255, 68)
                        } else {
                            ((92, 255, 148), 62.0, 225, 56)
                        }
                    }
                    MowerState::ChargeWindup | MowerState::Charge | MowerState::Ricochet => {
                        ((255, 96, 48), 66.0, 265, 72)
                    }
                    MowerState::FlingBlades => ((255, 168, 72), 54.0, 230, 58),
                    MowerState::Overheat | MowerState::Flipped => {
                        let flick = 0.72 + 0.28 * (elapsed * 16.0).sin().abs();
                        (
                            (255, 152, 58),
                            46.0,
                            (215.0 * flick) as u16,
                            (58.0 * flick) as u16,
                        )
                    }
                    MowerState::Docking => ((105, 245, 230), 58.0, 235, 60),
                    MowerState::Dying => ((255, 215, 120), 92.0, 320, 105),
                };

                // Chassis core glow.
                if in_range(bsx, bsy, 46.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 44.0,
                        color: lamp_col,
                        intensity: lamp_int / 2 + 70,
                        scatter: lamp_sc / 2 + 12,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }

                // A single beam from the nose, with the tile and prop outline
                // visible even as the mower sweeps its aim.
                if !matches!(st, MowerState::Flipped | MowerState::Dying) {
                    let p = boss.pos + face * 11.0;
                    let radius = lamp_r * 1.65;
                    if in_range(p.x - cam.x, p.y - cam.y, radius) {
                        let mut beam =
                            Light::radial(p, radius, lamp_col, f32::from(lamp_int) / 256.0);
                        beam.shape = LightShape::Cone {
                            radius,
                            dir: face,
                            half_angle: if st == MowerState::Scan { 0.43 } else { 0.67 },
                        };
                        beam.scatter = f32::from(lamp_sc) / 256.0;
                        beam.falloff = Falloff::Linear;
                        self.push_shaped(beam, 3);
                    }
                }

                // Target lock / charge rail beacon.
                if matches!(st, MowerState::Scan | MowerState::ChargeWindup) {
                    let tsx = boss.target.x - cam.x;
                    let tsy = boss.target.y - cam.y;
                    if in_range(tsx, tsy, 40.0) {
                        self.lights.push(PointLight {
                            x: tsx,
                            y: tsy,
                            radius: 38.0,
                            color: (255, 92, 62),
                            intensity: 230,
                            scatter: 60,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
            BossState::Cat(st) => {
                // Predator amber-gold eyes & presence glow.
                let head = boss.pos + face * 10.0;
                let hsx = head.x - cam.x;
                let hsy = head.y - cam.y;
                if in_range(hsx, hsy, 42.0) {
                    let col = if boss.phase >= 2 {
                        (255, 130, 62)
                    } else {
                        (255, 205, 88)
                    };
                    self.lights.push(PointLight {
                        x: hsx,
                        y: hsy,
                        radius: 40.0,
                        color: col,
                        intensity: 195,
                        scatter: 42,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }

                // Pounce landing telegraph or swipe tell.
                match st {
                    BossCatState::Crouch | BossCatState::Pounce => {
                        let tsx = boss.target.x - cam.x;
                        let tsy = boss.target.y - cam.y;
                        let locked = st == BossCatState::Pounce || progress >= 0.75;
                        let r = if locked { 44.0 } else { 36.0 };
                        if in_range(tsx, tsy, r) {
                            self.lights.push(PointLight {
                                x: tsx,
                                y: tsy,
                                radius: r,
                                color: if locked {
                                    (255, 82, 62)
                                } else {
                                    (255, 186, 72)
                                },
                                intensity: if locked { 255 } else { 205 },
                                scatter: if locked { 66 } else { 44 },
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BossCatState::SwipeWindup | BossCatState::Swipe => {
                        let p = boss.pos + face * 20.0;
                        let sx = p.x - cam.x;
                        let sy = p.y - cam.y;
                        if in_range(sx, sy, 46.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 44.0,
                                color: (255, 140, 70),
                                intensity: 235,
                                scatter: 58,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BossCatState::Roar | BossCatState::Death if in_range(bsx, bsy, 80.0) => {
                        self.lights.push(PointLight {
                            x: bsx,
                            y: bsy,
                            radius: 76.0,
                            color: (255, 165, 88),
                            intensity: 280,
                            scatter: 82,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                    _ => {}
                }
            }
            BossState::Broom(st) => {
                // Warm overhead spotlight following the Sweeper.
                if in_range(bsx, bsy, 54.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 52.0,
                        color: (255, 214, 150),
                        intensity: 175,
                        scatter: 34,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                match st {
                    BroomBossState::StompWalk { .. }
                    | BroomBossState::DustpanDrop
                    | BroomBossState::BroomJab
                    | BroomBossState::BroomSweep { .. } => {
                        let tsx = boss.target.x - cam.x;
                        let tsy = boss.target.y - cam.y;
                        if in_range(tsx, tsy, 42.0) {
                            self.lights.push(PointLight {
                                x: tsx,
                                y: tsy,
                                radius: 40.0,
                                color: (255, 118, 72),
                                intensity: 230,
                                scatter: 56,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    BroomBossState::PhaseChange | BroomBossState::Defeat
                        if in_range(bsx, bsy, 76.0) =>
                    {
                        self.lights.push(PointLight {
                            x: bsx,
                            y: bsy,
                            radius: 74.0,
                            color: (255, 195, 105),
                            intensity: 270,
                            scatter: 78,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                    _ => {}
                }
            }
            BossState::Mangle(st) => {
                // The wringer sits under a steamy laundry-bay lamp; phase 3
                // shifts warmer/hotter as the torn belt and cracked housing glow.
                if in_range(bsx, bsy, 54.0) {
                    let body_col = if boss.phase >= 3 {
                        (242, 186, 152)
                    } else if boss.phase >= 2 {
                        (222, 224, 210)
                    } else {
                        (204, 232, 228)
                    };
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 50.0,
                        color: body_col,
                        intensity: 196,
                        scatter: 42,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                match st {
                    MangleState::Intake { .. } => {
                        // The nip pulls light with it: amber warning ramping to a red-hot bite throat.
                        let view = boss.mangle_view();
                        let nip = crate::game::art_boss_mangle::nip_pos(&view);
                        let sx = nip.x - cam.x;
                        let sy = nip.y - cam.y;
                        let biting = progress * boss.duration >= 1.1;
                        let pulse = 0.85 + 0.15 * (elapsed * 14.0).sin().abs();
                        if in_range(sx, sy, 50.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: if biting { 48.0 } else { 44.0 },
                                color: if biting {
                                    (255, 88, 56)
                                } else {
                                    (255, 198, 78)
                                },
                                intensity: ((if biting { 268.0 } else { 240.0 }) * pulse) as u16,
                                scatter: if biting { 68 } else { 56 },
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    MangleState::WringOut => {
                        let sx = boss.target.x - cam.x;
                        let sy = boss.target.y - cam.y;
                        if in_range(sx, sy, 46.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 44.0,
                                color: (255, 184, 96),
                                intensity: 235,
                                scatter: 56,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    MangleState::Jam | MangleState::FinalJam => {
                        // The exposed crank glows bright pulsing gold: the punish
                        // window reads unmistakably in a dim laundry bay.
                        let view = boss.mangle_view();
                        let crank = crate::game::art_boss_mangle::crank_pos(&view);
                        let sx = crank.x - cam.x;
                        let sy = crank.y - cam.y;
                        let pulse = 0.86 + 0.14 * (elapsed * 11.0).sin().abs();
                        if in_range(sx, sy, 46.0) {
                            self.lights.push(PointLight {
                                x: sx,
                                y: sy,
                                radius: 44.0,
                                color: (255, 224, 114),
                                intensity: (272.0 * pulse) as u16,
                                scatter: 70,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                    MangleState::PhaseShift | MangleState::Death if in_range(bsx, bsy, 80.0) => {
                        self.lights.push(PointLight {
                            x: bsx,
                            y: bsy,
                            radius: 76.0,
                            color: (255, 196, 112),
                            intensity: 280,
                            scatter: 82,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                    _ => {}
                }
            }
            BossState::NightEdition(st) => {
                // Cold compositor's desk lamp over the cast-iron press, warming
                // slightly as Phase 3 pushes the rollers into smoky overload.
                if in_range(bsx, bsy, 54.0) {
                    let body_col = if boss.phase >= 3 {
                        (236, 206, 186)
                    } else {
                        (212, 226, 242)
                    };
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 50.0,
                        color: body_col,
                        intensity: 196,
                        scatter: 42,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                let mut tell = |p: Vec2, radius: f32, color: (u8, u8, u8), intensity: u16| {
                    let sx = p.x - cam.x;
                    let sy = p.y - cam.y;
                    if in_range(sx, sy, radius + 2.0) {
                        self.lights.push(PointLight {
                            x: sx,
                            y: sy,
                            radius,
                            color,
                            intensity,
                            scatter: 58,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                };
                let warn = if progress >= 0.7 {
                    (255, 92, 62)
                } else {
                    (255, 198, 78)
                };
                match st {
                    NightEditionState::PageTell | NightEditionState::PageStamp => {
                        let (ink, _) = crate::game::art_boss_night_edition::inked_rect(
                            boss.target,
                            boss.swipe_side,
                        );
                        let color = if matches!(st, NightEditionState::PageStamp) {
                            (255, 88, 60)
                        } else {
                            warn
                        };
                        tell(ink, 44.0, color, 248);
                        // Subtle cool-cyan safe-margin lift on the opposite side of the target
                        // so the blank margin reads cleanly in the dark press hall.
                        let safe = boss.target + (boss.target - ink) * 1.15;
                        tell(safe, 28.0, (124, 224, 242), 145);
                    }
                    NightEditionState::MisprintTell | NightEditionState::MisprintStamp => {
                        let rects = crate::game::art_boss_night_edition::misprint_rects(
                            boss.target,
                            boss.swipe_side,
                        );
                        let color = if matches!(st, NightEditionState::MisprintStamp) {
                            (255, 88, 60)
                        } else {
                            warn
                        };
                        for (ink, _) in rects {
                            tell(ink, 42.0, color, 240);
                        }
                    }
                    NightEditionState::CarriageTell => {
                        // Gleaming brass bell flash at the dotted rule's near end.
                        tell(boss.pounce_origin, 42.0, (255, 218, 96), 255);
                    }
                    NightEditionState::CarriageSweep => {
                        tell(boss.pos, 48.0, (255, 96, 62), 260);
                    }
                    NightEditionState::BedLowered | NightEditionState::Ribbon => {
                        // Pulsing bright gold-white beacon on the exposed bed or silk ribbon.
                        let view = boss.night_edition_view();
                        if let Some(weak) = crate::game::art_boss_night_edition::weak_point(&view) {
                            let pulse = 0.86 + 0.14 * (elapsed * 12.0).sin().abs();
                            tell(weak, 44.0, (255, 228, 122), (274.0 * pulse) as u16);
                        }
                    }
                    NightEditionState::PhaseTurn | NightEditionState::Wreck => {
                        tell(boss.pos, 76.0, (255, 202, 122), 280);
                    }
                    NightEditionState::Roll => {}
                }
            }
            BossState::HorseshoeCrab(st) => {
                // The crab sits under a cold drain-light (its shell is wet
                // stone); the punish windows glow copper-blue, and the gills
                // when they are out are the brightest thing on the floor.
                use crate::game::art_boss_horseshoe_crab::CrabState;
                if in_range(bsx, bsy, 58.0) {
                    let (col, r, int, sc) = match st {
                        CrabState::Wedged | CrabState::Flipped => {
                            ((126, 196, 255), 62.0, 250, 62)
                        }
                        CrabState::TelsonTell | CrabState::BulldozeTell | CrabState::BurrowTell => {
                            ((255, 198, 116), 58.0, 225, 54)
                        }
                        CrabState::Death => ((104, 156, 246), 64.0, 230, 58),
                        _ => ((198, 214, 226), 50.0, 180, 42),
                    };
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: r,
                        color: col,
                        intensity: int,
                        scatter: sc,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                // The exposed gills / the marked burrow stay lit through the
                // dark, so the weak point can never hide in the ambient.
                let view = boss.crab_view();
                let at = match st {
                    CrabState::BurrowTell | CrabState::Burrow | CrabState::Emerge => view.target,
                    _ => crate::game::art_boss_horseshoe_crab::gill_pos(&view),
                };
                let sx = at.x - cam.x;
                let sy = at.y - cam.y;
                if in_range(sx, sy, 40.0) {
                    let live = matches!(
                        st,
                        CrabState::Wedged | CrabState::Flipped | CrabState::BurrowTell
                    );
                    self.lights.push(PointLight {
                        x: sx,
                        y: sy,
                        radius: 38.0,
                        color: if live { (255, 232, 138) } else { (150, 206, 236) },
                        intensity: if live { 245 } else { 165 },
                        scatter: 44,
                        priority: 3,
                        occlude: false,
                        hard: false,
                    });
                }
            }
            BossState::MegaRat(st) => {
                use crate::game::art_boss_megarat::MegaRatState;
                // SUBJECT R-01: hot kitchen fat and cold steel. The key light
                // tracks its states; the exposed weak point in the two
                // recoveries gets its own bright beacon, because that is the
                // one thing the rat has to see across the room.
                if in_range(bsx, bsy, 74.0) {
                    let (col, r, int, sc) = match st {
                        MegaRatState::GnawStuck => ((255, 226, 150), 70.0, 255, 64),
                        MegaRatState::ThrashRecover => ((255, 210, 132), 68.0, 250, 62),
                        MegaRatState::GnawWindup
                        | MegaRatState::GnawLunge
                        | MegaRatState::DashWindup
                        | MegaRatState::DashPass => ((255, 148, 92), 62.0, 236, 54),
                        MegaRatState::ThrashWindup
                        | MegaRatState::ThrashPass1
                        | MegaRatState::ThrashGap
                        | MegaRatState::ThrashPass2 => ((255, 176, 108), 64.0, 238, 56),
                        MegaRatState::SqueakWindup | MegaRatState::SqueakShout => {
                            ((178, 234, 255), 70.0, 244, 62)
                        }
                        MegaRatState::Call => ((255, 236, 176), 66.0, 236, 58),
                        MegaRatState::Death => ((216, 200, 196), 64.0, 220, 56),
                        _ => ((214, 190, 160), 58.0, 204, 48),
                    };
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: r,
                        color: col,
                        intensity: int,
                        scatter: sc,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                if matches!(st, MegaRatState::GnawStuck | MegaRatState::ThrashRecover) {
                    let view = boss.megarat_view();
                    if let Some((weak, _)) = crate::game::art_boss_megarat::weak_point(&view) {
                        let wx = weak.x - cam.x;
                        let wy = weak.y - cam.y;
                        if in_range(wx, wy, 48.0) {
                            let pulse = 0.86 + 0.14 * (elapsed * 12.0).sin().abs();
                            self.lights.push(PointLight {
                                x: wx,
                                y: wy,
                                radius: 46.0,
                                color: (255, 232, 138),
                                intensity: (276.0 * pulse) as u16,
                                scatter: 66,
                                priority: 3,
                                occlude: false,
                                hard: false,
                            });
                        }
                    }
                }
            }
            BossState::Magpie(st) => {
                use crate::game::art_boss_magpie::MagpieState;
                // Iridescent corvid key-light so the black-and-white plumage
                // and peacock wing sheen pop sharply over the wooden flatbed.
                if in_range(bsx, bsy, 68.0) {
                    let (col, r, int, sc) = match st {
                        MagpieState::Stuck => ((255, 218, 134), 64.0, 255, 62),
                        MagpieState::Peck | MagpieState::PeckWindup => {
                            ((255, 162, 94), 58.0, 226, 50)
                        }
                        MagpieState::Gust | MagpieState::GustWindup => {
                            ((178, 222, 255), 72.0, 242, 66)
                        }
                        MagpieState::StrafePass => ((196, 228, 255), 62.0, 228, 56),
                        MagpieState::Death => ((206, 216, 236), 64.0, 224, 58),
                        _ if boss.phase >= 3 => ((174, 186, 255), 56.0, 204, 46),
                        _ => ((156, 206, 236), 56.0, 198, 44),
                    };
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: r,
                        color: col,
                        intensity: int,
                        scatter: sc,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                // Marked strike point / buried-head punish window stays vividly lit.
                if matches!(
                    st,
                    MagpieState::PeckWindup
                        | MagpieState::Peck
                        | MagpieState::Stuck
                        | MagpieState::StrafeWindup
                        | MagpieState::StrafePass
                        | MagpieState::SnatchDrop
                ) {
                    let tsx = boss.target.x - cam.x;
                    let tsy = boss.target.y - cam.y;
                    if in_range(tsx, tsy, 46.0) {
                        let pulse = 0.86 + 0.14 * (elapsed * 12.0).sin().abs();
                        let (r, col, int, sc) = match st {
                            MagpieState::Stuck => {
                                (44.0, (255, 228, 118), (272.0 * pulse) as u16, 66)
                            }
                            MagpieState::Peck => (42.0, (255, 108, 72), 255, 58),
                            MagpieState::SnatchDrop => (40.0, (255, 148, 78), 232, 52),
                            _ => (36.0, (255, 188, 92), 205, 48),
                        };
                        self.lights.push(PointLight {
                            x: tsx,
                            y: tsy,
                            radius: r,
                            color: col,
                            intensity: int,
                            scatter: sc,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
            BossState::Hawk(st) => {
                use crate::game::art_boss_hawk::HawkState;
                // While the bird is high the only thing on the deck is its
                // shadow, so the key light sits *there*: a tawny pool that
                // reads the bird's position through the flatbed's ambient.
                let (col, r, int, sc) = match st {
                    HawkState::Dive => {
                        if progress >= 0.55 {
                            ((255, 96, 62), 62.0, 262, 66)
                        } else {
                            ((255, 176, 96), 58.0, 220, 52)
                        }
                    }
                    HawkState::TalonBury => ((255, 214, 128), 56.0, 244, 58),
                    HawkState::Hover
                    | HawkState::DartFan
                    | HawkState::Grab
                    | HawkState::RakePass => ((255, 168, 92), 54.0, 226, 52),
                    HawkState::Death => ((206, 174, 150), 62.0, 220, 56),
                    _ => ((186, 142, 96), 52.0, 186, 40),
                };
                if in_range(bsx, bsy, r) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: r,
                        color: col,
                        intensity: int,
                        scatter: sc,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
                // The locked reticle and the buried nape are the two marks the
                // fight is read on: they stay lit through the dark.
                let weak = match st {
                    HawkState::Dive if progress >= 0.55 => Some((boss.target, (255, 108, 72), 46.0)),
                    HawkState::TalonBury => Some((
                        crate::game::art_boss_hawk::nape_pos(&boss.hawk_view()),
                        (255, 232, 138),
                        42.0,
                    )),
                    _ => None,
                };
                if let Some((at, colour, radius)) = weak {
                    let wx = at.x - cam.x;
                    let wy = at.y - cam.y;
                    if in_range(wx, wy, radius + 4.0) {
                        let pulse = 0.86 + 0.14 * (elapsed * 12.0).sin().abs();
                        self.lights.push(PointLight {
                            x: wx,
                            y: wy,
                            radius,
                            color: colour,
                            intensity: (268.0 * pulse) as u16,
                            scatter: 62,
                            priority: 3,
                            occlude: false,
                            hard: false,
                        });
                    }
                }
            }
            BossState::Cobra(st) => {
                // A cold-lab specimen: the head carries a sickly green lamp
                // (the venom), and the hood's spectacle mark lights up when it
                // is exposed.
                let (col, r, inten, scat) = match st {
                    crate::game::art_boss_cobra::CobraState::HoodTell
                    | crate::game::art_boss_cobra::CobraState::Strike
                    | crate::game::art_boss_cobra::CobraState::ReStrikeTell => {
                        ((255, 196, 92), 62.0, 235u16, 54u16)
                    }
                    crate::game::art_boss_cobra::CobraState::FangsStuck
                    | crate::game::art_boss_cobra::CobraState::SpiralEnd => {
                        let pulse = 0.78 + 0.22 * (elapsed * 9.0).sin().abs();
                        ((198, 255, 132), 46.0, (215.0 * pulse) as u16, (52.0 * pulse) as u16)
                    }
                    crate::game::art_boss_cobra::CobraState::VenomTell
                    | crate::game::art_boss_cobra::CobraState::VenomSpit => {
                        ((126, 226, 104), 56.0, 225, 50)
                    }
                    crate::game::art_boss_cobra::CobraState::Constrict
                    | crate::game::art_boss_cobra::CobraState::ConstrictTell => {
                        ((148, 236, 118), 60.0, 200, 44)
                    }
                    _ => ((132, 208, 118), 50.0, 178, 40),
                };
                if in_range(bsx, bsy, r) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: r,
                        color: col,
                        intensity: inten,
                        scatter: scat,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }
            _ => {
                // Fallback aura for future wave-3 bosses.
                if in_range(bsx, bsy, 58.0) {
                    self.lights.push(PointLight {
                        x: bsx,
                        y: bsy,
                        radius: 56.0,
                        color: (255, 145, 95),
                        intensity: 210,
                        scatter: 48,
                        priority: 2,
                        occlude: false,
                        hard: false,
                    });
                }
            }
        }

        // Every active boss hazard gets a high-priority self-lit warning pool.
        for hazard in &boss.hazards {
            if hazard.is_expired() {
                continue;
            }
            let c = hazard.shape.center();
            let sx = c.x - cam.x;
            let sy = c.y - cam.y;
            let active = hazard.is_active();
            let r = if active { 46.0 } else { 36.0 };
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: if active {
                        (255, 92, 66)
                    } else {
                        (255, 195, 76)
                    },
                    intensity: if active { 260 } else { 205 },
                    scatter: if active { 68 } else { 44 },
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }
    }

    /// The Lantern (Outfall boss): at most one dominant real lure (`Falloff::Smooth`,
    /// steady bioluminescent air channel independent of its 3 Hz tell flicker, no
    /// per-frame shadow bake) plus at most one cheap steady non-dominant decoy bulb.
    fn gather_lantern_lights(
        &mut self,
        view: &crate::game::art_boss_lantern::LanternView,
        boss: &Boss,
        cam: Vec2,
        vw: f32,
        vh: f32,
        elapsed: f32,
    ) {
        const LURE_CYAN: Rgb = (66, 255, 214);
        let in_range = |sx: f32, sy: f32, r: f32| -> bool {
            let r = r * TAIL_REACH;
            sx + r >= 0.0 && sy + r >= 0.0 && sx - r < vw && sy - r < vh
        };

        if let Some(bulb) = view.real_bulb.as_ref() {
            if bulb.radius > 0.5 && bulb.gain > 0.001 {
                let sx = bulb.pos.x - cam.x;
                let sy = bulb.pos.y - cam.y;
                if in_range(sx, sy, bulb.radius) {
                    let steady_energy = (1.35 * bulb.gain).clamp(0.0, 2.4);
                    let steady_scatter = (0.30 * bulb.gain).clamp(0.0, 0.45);
                    // 3 Hz held-step surface modulation during feint tells (gameplay
                    // cue); air scatter and `FxLight::steady` remain strictly steady.
                    let flick = if bulb.flicker > 0.0 {
                        let held = (elapsed * 3.0).floor() / 3.0;
                        0.76 + 0.24
                            * (held * std::f32::consts::TAU * 1.5 + bulb.flicker)
                                .sin()
                                .abs()
                    } else {
                        1.0
                    };
                    let mut light =
                        Light::radial(bulb.pos, bulb.radius, LURE_CYAN, steady_energy * flick);
                    light.scatter = steady_scatter * flick;
                    light.height = if bulb.planted { 10.0 } else { 12.0 };
                    light.falloff = Falloff::Smooth;
                    light.casts_shadow = false;
                    let lure_air = bulb.gain.clamp(0.0, 1.35);
                    self.push_lantern_lure(light, steady_energy + steady_scatter, lure_air);
                }
            }
        }

        if let Some(decoy) = view.decoy.as_ref() {
            if decoy.radius > 0.5 && decoy.gain > 0.001 {
                let sx = decoy.pos.x - cam.x;
                let sy = decoy.pos.y - cam.y;
                if in_range(sx, sy, decoy.radius) {
                    let mut light = Light::radial(
                        decoy.pos,
                        decoy.radius,
                        LURE_CYAN,
                        (0.72 * decoy.gain).clamp(0.0, 1.4),
                    );
                    light.scatter = (0.16 * decoy.gain).clamp(0.0, 0.30);
                    light.height = 10.0;
                    light.falloff = Falloff::Small;
                    light.casts_shadow = false;
                    self.push_shaped(light, 2);
                }
            }
        }

        // Only live strikes emit a restrained coral flash; dark tells rely on
        // the self-lit hostile layer outlines so DarkTell stays truly dark.
        for hazard in &boss.hazards {
            if hazard.is_expired() || !hazard.is_active() {
                continue;
            }
            let c = hazard.shape.center();
            let sx = c.x - cam.x;
            let sy = c.y - cam.y;
            let r = 38.0;
            if in_range(sx, sy, r) {
                self.lights.push(PointLight {
                    x: sx,
                    y: sy,
                    radius: r,
                    color: (255, 92, 66),
                    intensity: 210,
                    scatter: 52,
                    priority: 3,
                    occlude: false,
                    hard: false,
                });
            }
        }
    }
}

/// A body colour as a light colour: same hue, scaled so its strongest channel
/// is full. Dull coats (mud, tarpaulin, ink) still throw a readable glow.
fn emissive_rgb(c: Rgb) -> Rgb {
    let peak = u32::from(c.0.max(c.1).max(c.2)).max(1);
    let lift = |v: u8| (u32::from(v) * 255 / peak) as u8;
    (lift(c.0), lift(c.1), lift(c.2))
}

/// Share of full brightness an enemy rim glint carries: a hint of the body's
/// own colour, never a lamp.
const RIM_GAIN: f32 = 0.12;
/// The rim in the body's own colour, as a mark pixel. The mark pass keeps it
/// only if nothing repainted over that pixel, and then lifts the pixel rather
/// than dimming it.
fn rim_glint(c: Rgb) -> Rgb {
    let (r, g, b) = emissive_rgb(c);
    (
        (f32::from(r) * RIM_GAIN) as u8,
        (f32::from(g) * RIM_GAIN) as u8,
        (f32::from(b) * RIM_GAIN) as u8,
    )
}

#[inline]
fn blend_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

#[inline]
fn brighten_rgb(c: Rgb, boost: u8) -> Rgb {
    (
        c.0.saturating_add(boost),
        c.1.saturating_add(boost),
        c.2.saturating_add(boost),
    )
}

#[inline]
fn pos_hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^ (h >> 15)
}
