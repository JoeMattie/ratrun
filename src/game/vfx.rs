//! High-level combat feedback VFX emitters.
//!
//! Keeps `world.rs` clean and surgical: every weapon hit, enemy death, dash,
//! pickup, acid splash, explosion, and level-up delegates here, and every
//! emitter degrades gracefully via `Particles::budget_count` under heavy load.

use rand::{Rng, RngExt};

use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::particle::{Particle, Particles, Ramp, Shape, Shockwave, Stain, StainKind};
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;
use crate::render::palette::{self, Rgb};

// ---- Player movement & hurt ------------------------------------------------

/// Crisp kick-off puff + pale speed streaks when the rat triggers a dash.
pub fn emit_dash_start(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, face: Vec2) {
    let back = if face.len_sq() > 0.01 {
        -face.normalized()
    } else {
        Vec2::new(-1.0, 0.0)
    };
    let streaks = ps.budget_count(8, false);
    ps.cone_shaped(
        rng,
        pos + back * 2.0,
        back,
        streaks,
        92.0,
        0.55,
        Ramp::Dash,
        0.26,
        true,
        Shape::Streak,
        6.5,
    );
    let dust = ps.budget_count(7, false);
    ps.cone_shaped(
        rng,
        pos,
        back,
        dust,
        42.0,
        0.95,
        Ramp::Smoke,
        0.34,
        false,
        Shape::Dot,
        4.5,
    );
    // Crisp translucent snap arc behind the rat's takeoff spot.
    ps.push_shockwave(Shockwave {
        pos: pos + back * 1.5,
        start_r: 2.0,
        end_r: 10.5,
        life: 0.22,
        max_life: 0.22,
        color: (155, 218, 255),
        core: (250, 252, 255),
        thick: 1.5,
        additive: true,
        arc_dir: Some(back),
        arc_half_angle: 1.18,
    });
}

/// Continuous sleek motion trail emitted while `player.dash_active > 0`.
pub fn emit_dash_trail(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, vel: Vec2, dt: f32) {
    let dir = if vel.len_sq() > 1.0 {
        -vel.normalized()
    } else {
        Vec2::new(-1.0, 0.0)
    };
    let perp = dir.perp();
    // Rate-independent ~1..2 streaks per 60Hz tick.
    let expected = (dt * 105.0).clamp(0.4, 3.0);
    let count = ps.budget_count(expected.round() as usize, false);
    for _ in 0..count {
        let lateral = rng.random_range(-2.4..2.4);
        let speed = rng.random_range(38.0..76.0);
        let life = rng.random_range(0.14..0.24);
        // Occasional warm tail-pink accent mixed into the ghost-cyan wake.
        let ramp = if rng.random_range(0..5) == 0 {
            Ramp::Tail
        } else {
            Ramp::Dash
        };
        ps.push(Particle {
            pos: pos + perp * lateral,
            vel: dir * speed + perp * rng.random_range(-8.0..8.0),
            life,
            max_life: life,
            drag: 7.5,
            gravity: 0.0,
            ramp,
            additive: true,
            shape: Shape::Streak,
        });
    }
}

/// Sharp crimson burst + brief impact ring when the player rat takes damage.
pub fn emit_player_hurt(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 2.0,
        end_r: 13.0,
        life: 0.22,
        max_life: 0.22,
        color: (255, 70, 82),
        core: (255, 220, 225),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let streaks = ps.budget_count(10, true);
    ps.burst_shaped(
        rng,
        pos,
        streaks,
        68.0,
        1.5,
        Ramp::Custom((255, 110, 110), (140, 22, 30)),
        0.30,
        true,
        Shape::Streak,
        5.0,
    );
    let drops = ps.budget_count(6, true);
    ps.burst_shaped(
        rng,
        pos,
        drops,
        36.0,
        1.5,
        Ramp::Custom((255, 80, 95), (90, 18, 24)),
        0.42,
        false,
        Shape::Dot,
        3.5,
    );
}

// ---- Weapons: muzzle / cast / flight / hit ---------------------------------

/// Distinct muzzle flash / cast visual per weapon.
pub fn emit_weapon_fire(
    ps: &mut Particles,
    rng: &mut impl Rng,
    kind: WeaponKind,
    pos: Vec2,
    aim: Vec2,
    radius: f32,
) {
    let dir = if aim.len_sq() > 0.01 {
        aim.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    match kind {
        WeaponKind::Gnaw => {
            let n = ps.budget_count(3, false);
            ps.cone_shaped(
                rng,
                pos + dir * 2.5,
                dir,
                n,
                68.0,
                0.30,
                Ramp::Spark,
                0.14,
                true,
                Shape::Streak,
                8.0,
            );
        }
        WeaponKind::CheeseSpray => {
            let n = ps.budget_count(6, false);
            ps.cone_shaped(
                rng,
                pos + dir * 2.5,
                dir,
                n,
                72.0,
                0.55,
                Ramp::Fire,
                0.22,
                true,
                Shape::Dot,
                5.5,
            );
        }
        WeaponKind::SqueakNova => {
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 2.0,
                end_r: 18.0,
                life: 0.24,
                max_life: 0.24,
                color: (185, 135, 255),
                core: (255, 245, 255),
                thick: 1.5,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            let n = ps.budget_count(10, false);
            ps.burst_shaped(
                rng,
                pos,
                n,
                78.0,
                1.0,
                Ramp::Nova,
                0.28,
                true,
                Shape::Streak,
                5.5,
            );
        }
        WeaponKind::SporeOrbit => {
            let n = ps.budget_count(6, false);
            ps.burst_shaped(
                rng,
                pos,
                n,
                28.0,
                3.0,
                Ramp::Spore,
                0.32,
                true,
                Shape::Ember,
                4.0,
            );
        }
        WeaponKind::TailWhip => {
            // Bold hot-pink & white sweeping shockwave ring around the rat.
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 5.0,
                end_r: radius,
                life: 0.24,
                max_life: 0.24,
                color: (255, 130, 192),
                core: (255, 245, 252),
                thick: 2.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            let sparks = ps.budget_count(16, false);
            for k in 0..sparks {
                let a = k as f32 / sparks.max(1) as f32 * std::f32::consts::TAU
                    + rng.random_range(-0.12..0.12);
                let tangent = Vec2::from_angle(a + std::f32::consts::FRAC_PI_2);
                let out = Vec2::from_angle(a);
                let life = rng.random_range(0.18..0.30);
                ps.push(Particle {
                    pos: pos + out * (radius * 0.88),
                    vel: tangent * rng.random_range(35.0..68.0) + out * 16.0,
                    life,
                    max_life: life,
                    drag: 6.0,
                    gravity: 0.0,
                    ramp: Ramp::Tail,
                    additive: true,
                    shape: Shape::Streak,
                });
            }
        }
        WeaponKind::AcidSpit => {
            let n = ps.budget_count(4, false);
            ps.cone_shaped(
                rng,
                pos + dir * 2.0,
                dir,
                n,
                46.0,
                0.45,
                Ramp::Acid,
                0.24,
                false,
                Shape::Dot,
                5.0,
            );
        }
    }
}

/// Distinct weapon hit sparks on projectile impact.
pub fn emit_weapon_hit(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    bullet_vel: Vec2,
    weapon: Option<WeaponKind>,
    _enemy_kind: EnemyKind,
    killed: bool,
) {
    let reflect = if bullet_vel.len_sq() > 1.0 {
        -bullet_vel.normalized()
    } else {
        Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU))
    };
    let bonus = if killed { 2 } else { 0 };

    match weapon {
        Some(WeaponKind::Gnaw) | None => {
            // Sharp ivory-gold kinetic streaks + crisp hot impact glint.
            ps.push(Particle {
                pos,
                vel: Vec2::ZERO,
                life: 0.11,
                max_life: 0.11,
                drag: 0.0,
                gravity: 0.0,
                ramp: Ramp::Spark,
                additive: true,
                shape: Shape::Blob,
            });
            let n = ps.budget_count(5 + bonus, false);
            ps.cone_shaped(
                rng,
                pos,
                reflect,
                n,
                92.0,
                1.05,
                Ramp::Spark,
                0.22,
                true,
                Shape::Streak,
                5.5,
            );
        }
        Some(WeaponKind::CheeseSpray) => {
            // Sizzling golden-orange cheddar splatter droplets + hot orange sparks.
            let n = ps.budget_count(6 + bonus, false);
            ps.cone_shaped(
                rng,
                pos,
                reflect,
                n,
                72.0,
                1.35,
                Ramp::Fire,
                0.26,
                true,
                Shape::Dot,
                4.2,
            );
            let globs = ps.budget_count(2, false);
            ps.cone_shaped(
                rng,
                pos,
                reflect,
                globs,
                46.0,
                1.0,
                Ramp::Gold,
                0.22,
                true,
                Shape::Blob,
                5.0,
            );
        }
        Some(WeaponKind::SqueakNova) => {
            // Electric violet star-cross pop + sharp radial streaks.
            ps.push(Particle {
                pos,
                vel: Vec2::ZERO,
                life: 0.22,
                max_life: 0.22,
                drag: 0.0,
                gravity: 0.0,
                ramp: Ramp::Nova,
                additive: true,
                shape: Shape::Cross,
            });
            let n = ps.budget_count(5 + bonus, false);
            ps.burst_shaped(
                rng,
                pos,
                n,
                82.0,
                0.8,
                Ramp::Nova,
                0.24,
                true,
                Shape::Streak,
                5.2,
            );
        }
        Some(WeaponKind::SporeOrbit) => {
            // Drifting bioluminescent mint spore puff + subtle cross glint on kill.
            if killed {
                ps.push(Particle {
                    pos,
                    vel: Vec2::ZERO,
                    life: 0.20,
                    max_life: 0.20,
                    drag: 0.0,
                    gravity: 0.0,
                    ramp: Ramp::Spore,
                    additive: true,
                    shape: Shape::Cross,
                });
            }
            let n = ps.budget_count(4 + bonus / 2, false);
            ps.burst_shaped(
                rng,
                pos,
                n,
                46.0,
                1.5,
                Ramp::Spore,
                0.34,
                true,
                Shape::Ember,
                4.2,
            );
        }
        Some(WeaponKind::TailWhip) => {
            emit_tail_whip_hit(ps, rng, pos, reflect, killed);
        }
        Some(WeaponKind::AcidSpit) => {
            let globs = ps.budget_count(3, false);
            ps.burst_shaped(
                rng,
                pos,
                globs,
                46.0,
                1.0,
                Ramp::Acid,
                0.30,
                false,
                Shape::Blob,
                4.2,
            );
            let spray = ps.budget_count(5, false);
            ps.burst_shaped(
                rng,
                pos,
                spray,
                72.0,
                1.2,
                Ramp::Acid,
                0.28,
                true,
                Shape::Dot,
                4.8,
            );
        }
    }
}

/// Crisp rat-tail pink slash flash on enemies clipped by `TailWhip`.
pub fn emit_tail_whip_hit(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    out_dir: Vec2,
    killed: bool,
) {
    ps.push(Particle {
        pos,
        vel: Vec2::ZERO,
        life: 0.16,
        max_life: 0.16,
        drag: 0.0,
        gravity: 0.0,
        ramp: Ramp::Tail,
        additive: true,
        shape: Shape::Cross,
    });
    let n = ps.budget_count(if killed { 7 } else { 5 }, false);
    let dir = if out_dir.len_sq() > 0.01 {
        out_dir.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    ps.cone_shaped(
        rng,
        pos,
        dir,
        n,
        88.0,
        0.95,
        Ramp::Tail,
        0.24,
        true,
        Shape::Streak,
        5.2,
    );
}

/// Subtle mint shimmer trailing behind orbiting spores.
pub fn emit_orbit_trail(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, dt: f32) {
    // Emit roughly 12 spores/sec per orb so it glows without flooding the ring.
    if rng.random::<f32>() < (dt * 14.0).min(0.45) && ps.budget_count(1, false) > 0 {
        let life = rng.random_range(0.18..0.30);
        ps.push(Particle {
            pos: pos + Vec2::new(rng.random_range(-0.8..0.8), rng.random_range(-0.8..0.8)),
            vel: Vec2::new(rng.random_range(-8.0..8.0), rng.random_range(-8.0..8.0)),
            life,
            max_life: life,
            drag: 4.5,
            gravity: 0.0,
            ramp: Ramp::Spore,
            additive: true,
            shape: Shape::Dot,
        });
    }
}

/// Wet lime drip trailing behind a lobbed AcidSpit projectile in flight.
pub fn emit_acid_drip(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, dt: f32) {
    if rng.random::<f32>() < (dt * 28.0).min(0.7) && ps.budget_count(1, false) > 0 {
        let life = rng.random_range(0.16..0.28);
        ps.push(Particle {
            pos,
            vel: Vec2::new(rng.random_range(-10.0..10.0), rng.random_range(-6.0..12.0)),
            life,
            max_life: life,
            drag: 3.5,
            gravity: 22.0,
            ramp: Ramp::Acid,
            additive: false,
            shape: Shape::Dot,
        });
    }
}

/// Juicy acid splash when a lobbed AcidSpit lands and forms a pool.
pub fn emit_acid_splash(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, radius: f32) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 2.0,
        end_r: radius * 1.12,
        life: 0.28,
        max_life: 0.28,
        color: (145, 240, 72),
        core: (230, 255, 165),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    // Chunky lime globs with light gravity arc.
    let globs = ps.budget_count(7, false);
    for _ in 0..globs {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(20.0..48.0);
        let life = rng.random_range(0.28..0.46);
        ps.push(Particle {
            pos: pos + Vec2::from_angle(a) * rng.random_range(0.0..2.5),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -14.0),
            life,
            max_life: life,
            drag: 3.8,
            gravity: 48.0,
            ramp: Ramp::Acid,
            additive: false,
            shape: Shape::Blob,
        });
    }
    // Fine bright sizzle mist.
    let mist = ps.budget_count(10, false);
    ps.burst_shaped(
        rng,
        pos,
        mist,
        42.0,
        2.5,
        Ramp::Acid,
        0.38,
        true,
        Shape::Dot,
        4.5,
    );
    // Subtle dark-lime etched stain on the floor beneath the splash.
    ps.push_stain(Stain {
        pos,
        radius: (radius * 0.45).clamp(3.0, 6.5),
        color: (52, 98, 28),
        life: 5.5,
        max_life: 5.5,
        seed: rng.random::<u16>(),
        kind: StainKind::Splat,
    });
}

/// Occasional rising sizzle bubbles from an active acid pool.
pub fn emit_pool_bubbles(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    radius: f32,
    life: f32,
    dt: f32,
) {
    let rate = if life > 0.5 { 8.0 } else { 3.5 };
    if rng.random::<f32>() < (dt * rate).min(0.35) && ps.budget_count(1, false) > 0 {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let d = rng.random_range(0.0..radius * 0.85);
        let blife = rng.random_range(0.22..0.40);
        ps.push(Particle {
            pos: pos + Vec2::from_angle(a) * d,
            vel: Vec2::new(rng.random_range(-4.0..4.0), rng.random_range(-16.0..-7.0)),
            life: blife,
            max_life: blife,
            drag: 2.0,
            gravity: 0.0,
            ramp: Ramp::Acid,
            additive: true,
            shape: Shape::Dot,
        });
    }
}

// ---- Enemy death bursts & floor stains -------------------------------------

/// Enemy-kind-specific death burst + wet floor ichor stain.
pub fn emit_enemy_death(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    radius: f32,
    kind: EnemyKind,
    theme_blood: Rgb,
) {
    // Blend the map's signature ichor with the enemy's own color so deaths
    // feel both map-coherent and enemy-specific.
    let enemy_col = kind.color();
    let ichor = palette::mix(theme_blood, enemy_col, 0.35);
    let stain_col = palette::scale(ichor, 0.56);

    match kind {
        EnemyKind::Skitterer => {
            // Crisp chitin flecks + small ichor pop.
            let drops = ps.budget_count(7, false);
            ps.burst_shaped(
                rng,
                pos,
                drops,
                52.0,
                radius * 0.6,
                Ramp::Blood(ichor),
                0.42,
                false,
                Shape::Dot,
                4.0,
            );
            let chips = ps.budget_count(4, false);
            ps.burst_shaped(
                rng,
                pos,
                chips,
                64.0,
                1.0,
                Ramp::Custom(enemy_col, palette::scale(enemy_col, 0.3)),
                0.30,
                false,
                Shape::Streak,
                6.0,
            );
            ps.push_stain(Stain {
                pos,
                radius: 2.1,
                color: stain_col,
                life: 5.5,
                max_life: 5.5,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
        EnemyKind::Bat => {
            // Wispy violet wing-flutter sparks + airy mist.
            let wisps = ps.budget_count(7, false);
            ps.burst_shaped(
                rng,
                pos,
                wisps,
                44.0,
                radius * 0.8,
                Ramp::Custom((195, 165, 245), (58, 38, 92)),
                0.45,
                true,
                Shape::Ember,
                5.5,
            );
            let drops = ps.budget_count(4, false);
            ps.burst_shaped(
                rng,
                pos,
                drops,
                38.0,
                1.0,
                Ramp::Blood(ichor),
                0.35,
                false,
                Shape::Dot,
                4.5,
            );
            ps.push_stain(Stain {
                pos,
                radius: 1.7,
                color: stain_col,
                life: 4.5,
                max_life: 4.5,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
        EnemyKind::Cat => {
            // Armored beetle: crunchy shell-crack sparks + dark carapace shards + ichor.
            let sparks = ps.budget_count(6, false);
            ps.burst_shaped(
                rng,
                pos,
                sparks,
                74.0,
                1.5,
                Ramp::Spark,
                0.24,
                true,
                Shape::Streak,
                6.5,
            );
            let shell = ps.budget_count(5, false);
            ps.burst_shaped(
                rng,
                pos,
                shell,
                56.0,
                radius * 0.6,
                Ramp::Custom((145, 150, 168), (38, 40, 50)),
                0.45,
                false,
                Shape::Blob,
                4.5,
            );
            let blood = ps.budget_count(9, false);
            ps.burst_shaped(
                rng,
                pos,
                blood,
                54.0,
                radius * 0.7,
                Ramp::Blood(ichor),
                0.52,
                false,
                Shape::Dot,
                3.5,
            );
            ps.push_stain(Stain {
                pos,
                radius: 3.0,
                color: stain_col,
                life: 6.5,
                max_life: 6.5,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
        EnemyKind::Spitter => {
            // Toxic wasp rupture: sizzling lime blobs + bright acid mist.
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 1.5,
                end_r: 10.5,
                life: 0.22,
                max_life: 0.22,
                color: (145, 240, 75),
                core: (235, 255, 170),
                thick: 1.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            let blobs = ps.budget_count(6, false);
            ps.burst_shaped(
                rng,
                pos,
                blobs,
                52.0,
                radius * 0.6,
                Ramp::Acid,
                0.48,
                false,
                Shape::Blob,
                3.8,
            );
            let mist = ps.budget_count(8, false);
            ps.burst_shaped(
                rng,
                pos,
                mist,
                62.0,
                1.5,
                Ramp::Acid,
                0.36,
                true,
                Shape::Ember,
                4.8,
            );
            ps.push_stain(Stain {
                pos,
                radius: 3.3,
                color: (58, 118, 34),
                life: 7.0,
                max_life: 7.0,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
        EnemyKind::Brute => {
            // Heavy rhino beetle explosion: shockwave + heavy ichor + shell chunks + sparks!
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 3.0,
                end_r: 22.0,
                life: 0.30,
                max_life: 0.30,
                color: (245, 120, 70),
                core: (255, 245, 210),
                thick: 2.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            let streaks = ps.budget_count(14, true);
            ps.burst_shaped(
                rng,
                pos,
                streaks,
                96.0,
                radius * 0.4,
                Ramp::Fire,
                0.34,
                true,
                Shape::Streak,
                5.0,
            );
            let blobs = ps.budget_count(12, true);
            ps.burst_shaped(
                rng,
                pos,
                blobs,
                70.0,
                radius * 0.7,
                Ramp::Blood(ichor),
                0.62,
                false,
                Shape::Blob,
                3.5,
            );
            let smoke = ps.budget_count(8, true);
            ps.burst_shaped(
                rng,
                pos,
                smoke,
                36.0,
                radius * 0.5,
                Ramp::Smoke,
                0.55,
                false,
                Shape::Blob,
                3.2,
            );
            ps.push_stain(Stain {
                pos,
                radius: 5.2,
                color: stain_col,
                life: 8.5,
                max_life: 8.5,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
        EnemyKind::Boss => {
            // Climactic Boss death: triple shockwaves, blazing debris, golden stars, big stain!
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 4.0,
                end_r: 46.0,
                life: 0.36,
                max_life: 0.36,
                color: (255, 95, 165),
                core: (255, 255, 255),
                thick: 2.2,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 2.0,
                end_r: 68.0,
                life: 0.62,
                max_life: 0.62,
                color: (255, 200, 82),
                core: (255, 240, 200),
                thick: 2.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            ps.push_shockwave(Shockwave {
                pos,
                start_r: 1.0,
                end_r: 28.0,
                life: 0.24,
                max_life: 0.24,
                color: (255, 255, 240),
                core: (255, 255, 255),
                thick: 2.0,
                additive: true,
                arc_dir: None,
                arc_half_angle: 0.0,
            });
            let streaks = ps.budget_count(42, true);
            ps.burst_shaped(
                rng,
                pos,
                streaks,
                140.0,
                radius * 0.4,
                Ramp::Fire,
                0.55,
                true,
                Shape::Streak,
                3.6,
            );
            let stars = ps.budget_count(22, true);
            ps.burst_shaped(
                rng,
                pos,
                stars,
                105.0,
                radius * 0.5,
                Ramp::Gold,
                0.78,
                true,
                Shape::Cross,
                3.0,
            );
            let gore = ps.budget_count(36, true);
            ps.burst_shaped(
                rng,
                pos,
                gore,
                92.0,
                radius * 0.8,
                Ramp::Blood(ichor),
                0.85,
                false,
                Shape::Blob,
                2.8,
            );
            let embers = ps.budget_count(32, true);
            ps.burst_shaped(
                rng,
                pos,
                embers,
                64.0,
                radius * 0.7,
                Ramp::Custom((255, 110, 175), (90, 20, 50)),
                1.15,
                true,
                Shape::Ember,
                2.0,
            );
            ps.push_stain(Stain {
                pos,
                radius: 8.5,
                color: (18, 12, 16),
                life: 12.0,
                max_life: 12.0,
                seed: rng.random::<u16>(),
                kind: StainKind::Scorch,
            });
            ps.push_stain(Stain {
                pos: pos + Vec2::new(2.0, -1.5),
                radius: 7.0,
                color: stain_col,
                life: 11.0,
                max_life: 11.0,
                seed: rng.random::<u16>(),
                kind: StainKind::Splat,
            });
        }
    }
}

/// Extra gilded burst when a crowned Elite Brute goes down.
pub fn emit_elite_death_accent(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 3.0,
        end_r: 28.0,
        life: 0.38,
        max_life: 0.38,
        color: (255, 212, 88),
        core: (255, 252, 230),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let stars = ps.budget_count(8, true);
    ps.burst_shaped(
        rng,
        pos,
        stars,
        74.0,
        2.5,
        Ramp::Gold,
        0.50,
        true,
        Shape::Cross,
        3.5,
    );
}

/// Ominous entrance ring when the Hornet Queen boss arrives.
pub fn emit_boss_spawn(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, accent: Rgb) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 4.0,
        end_r: 36.0,
        life: 0.55,
        max_life: 0.55,
        color: (255, 70, 140),
        core: (255, 220, 240),
        thick: 2.0,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let n = ps.budget_count(18, true);
    ps.burst_shaped(
        rng,
        pos,
        n,
        70.0,
        6.0,
        Ramp::Custom((255, 95, 155), accent),
        0.55,
        true,
        Shape::Cross,
        3.5,
    );
}

// ---- Gems, Level-Up & Pickups ----------------------------------------------

/// Subtle glint trail left behind magnetized XP gems zipping toward the rat.
pub fn emit_gem_trail(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, color: Rgb, dt: f32) {
    if rng.random::<f32>() < (dt * 12.0).min(0.35) && ps.budget_count(1, false) > 0 {
        let life = rng.random_range(0.14..0.22);
        ps.push(Particle {
            pos,
            vel: Vec2::new(rng.random_range(-6.0..6.0), rng.random_range(-6.0..6.0)),
            life,
            max_life: life,
            drag: 5.0,
            gravity: 0.0,
            ramp: Ramp::Custom(color, palette::scale(color, 0.3)),
            additive: true,
            shape: Shape::Dot,
        });
    }
}

/// Crisp sparkle pop on XP gem collection (gold stars for big gems, cyan for normal).
pub fn emit_gem_collect(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, gained: u32) {
    let big = gained >= 10;
    let ramp = if big { Ramp::Gold } else { Ramp::Gem };
    let crosses = ps.budget_count(if big { 4 } else { 2 }, false);
    for _ in 0..crosses {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(16.0..38.0);
        let life = rng.random_range(0.20..0.34);
        ps.push(Particle {
            pos: pos + Vec2::from_angle(a) * rng.random_range(1.0..3.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -8.0),
            life,
            max_life: life,
            drag: 5.0,
            gravity: 0.0,
            ramp,
            additive: true,
            shape: Shape::Cross,
        });
    }
    let dots = ps.budget_count(if big { 6 } else { 3 }, false);
    ps.burst_shaped(
        rng,
        pos,
        dots,
        42.0,
        1.5,
        ramp,
        0.24,
        true,
        Shape::Dot,
        4.5,
    );
}

/// Triumphant level-up burst: double gold + tail-pink ring & ascending star fountain!
pub fn emit_level_up(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 3.0,
        end_r: 32.0,
        life: 0.48,
        max_life: 0.48,
        color: (255, 222, 102),
        core: (255, 255, 245),
        thick: 2.0,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 2.0,
        end_r: 20.0,
        life: 0.34,
        max_life: 0.34,
        color: (255, 162, 198),
        core: (255, 248, 252),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let stars = ps.budget_count(16, true);
    for k in 0..stars {
        let a = k as f32 / stars.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.15..0.15);
        let sp = rng.random_range(28.0..72.0);
        let life = rng.random_range(0.45..0.75);
        let ramp = if k % 3 == 0 { Ramp::Tail } else { Ramp::Gold };
        ps.push(Particle {
            pos: pos + Vec2::from_angle(a) * rng.random_range(1.5..4.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -14.0),
            life,
            max_life: life,
            drag: 3.2,
            gravity: -8.0,
            ramp,
            additive: true,
            shape: Shape::Cross,
        });
    }
    let sparks = ps.budget_count(18, true);
    ps.burst_shaped(
        rng,
        pos,
        sparks,
        84.0,
        2.0,
        Ramp::Gold,
        0.48,
        true,
        Shape::Streak,
        4.2,
    );
}

/// Gentle ambient sparkle around pickups waiting on the floor.
pub fn emit_pickup_ambient(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    kind: PickupKind,
    dt: f32,
) {
    if rng.random::<f32>() < (dt * 4.5).min(0.25) && ps.budget_count(1, false) > 0 {
        let ramp = match kind {
            PickupKind::Heal => Ramp::Heal,
            PickupKind::Magnet => Ramp::Magnet,
            PickupKind::Nuke => Ramp::Fire,
        };
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let d = rng.random_range(2.5..5.5);
        let life = rng.random_range(0.30..0.48);
        ps.push(Particle {
            pos: pos + Vec2::from_angle(a) * d,
            vel: Vec2::new(rng.random_range(-4.0..4.0), rng.random_range(-16.0..-7.0)),
            life,
            max_life: life,
            drag: 2.0,
            gravity: 0.0,
            ramp,
            additive: true,
            shape: Shape::Dot,
        });
    }
}

/// Soothing emerald ring + rising `+` sparkles on Heal pickup.
pub fn emit_pickup_heal(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 2.0,
        end_r: 20.0,
        life: 0.36,
        max_life: 0.36,
        color: (92, 255, 130),
        core: (235, 255, 240),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let crosses = ps.budget_count(10, true);
    for _ in 0..crosses {
        let life = rng.random_range(0.38..0.62);
        ps.push(Particle {
            pos: pos + Vec2::new(rng.random_range(-5.0..5.0), rng.random_range(-3.0..4.0)),
            vel: Vec2::new(rng.random_range(-14.0..14.0), rng.random_range(-36.0..-16.0)),
            life,
            max_life: life,
            drag: 2.6,
            gravity: 0.0,
            ramp: Ramp::Heal,
            additive: true,
            shape: Shape::Cross,
        });
    }
}

/// Sweeping cyan-blue magnetic pulse on Magnet pickup.
pub fn emit_pickup_magnet(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 3.0,
        end_r: 42.0,
        life: 0.42,
        max_life: 0.42,
        color: (110, 185, 255),
        core: (235, 248, 255),
        thick: 2.0,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let n = ps.budget_count(14, true);
    ps.burst_shaped(
        rng,
        pos,
        n,
        82.0,
        2.0,
        Ramp::Magnet,
        0.38,
        true,
        Shape::Streak,
        4.5,
    );
}

/// Screen-clearing Nuke shockwave + fiery blast + scorched floor decal!
pub fn emit_nuke(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    // Primary blinding white-gold shockwave sweeping across the viewport.
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 4.0,
        end_r: 115.0,
        life: 0.62,
        max_life: 0.62,
        color: (255, 155, 52),
        core: (255, 255, 245),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    // Secondary hot orange-red inner shockwave.
    ps.push_shockwave(Shockwave {
        pos,
        start_r: 2.0,
        end_r: 62.0,
        life: 0.40,
        max_life: 0.40,
        color: (255, 92, 42),
        core: (255, 235, 150),
        thick: 2.0,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    // Blazing radial shock-front streaks riding outward with the wave.
    let streaks = ps.budget_count(52, true);
    ps.burst_shaped(
        rng,
        pos,
        streaks,
        215.0,
        3.0,
        Ramp::Fire,
        0.56,
        true,
        Shape::Streak,
        2.5,
    );
    // Fiery blast blobs + lingering drifting embers.
    let blobs = ps.budget_count(24, true);
    ps.burst_shaped(
        rng,
        pos,
        blobs,
        95.0,
        3.5,
        Ramp::Fire,
        0.65,
        true,
        Shape::Blob,
        3.5,
    );
    let embers = ps.budget_count(36, true);
    ps.burst_shaped(
        rng,
        pos,
        embers,
        120.0,
        4.0,
        Ramp::Gold,
        1.05,
        true,
        Shape::Ember,
        2.2,
    );
    // Dark blast scorch mark on the floor beneath ground zero.
    ps.push_stain(Stain {
        pos,
        radius: 11.0,
        color: (14, 10, 10),
        life: 12.0,
        max_life: 12.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Scorch,
    });
}

// ---- Wave 1 Helpers: defensive math ----------------------------------------

#[inline]
fn safe_dir(v: Vec2, fallback: Vec2) -> Vec2 {
    if v.x.is_finite() && v.y.is_finite() && v.len_sq() > 1e-4 {
        v.normalized()
    } else {
        fallback
    }
}

#[inline]
fn safe_pos(v: Vec2) -> Vec2 {
    if v.x.is_finite() && v.y.is_finite() {
        v
    } else {
        Vec2::ZERO
    }
}

#[inline]
fn safe_clamp(x: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        default
    }
}

#[inline]
fn safe_dt(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 {
        dt.min(0.25)
    } else {
        0.0
    }
}

// ---- Traps & Interactive Level Objects (art_traps.rs) ----------------------

/// Viscous amber squelch splash, wet floor stain, and flying glue-strand flicks
/// when a victim first sticks to a glue board (`StickyTrapState::Idle -> Stuck`).
///
/// Complements `art_traps::draw_sticky_trap` (which draws the 5 taut holding
/// strands continuously while `Stuck`).
pub fn emit_glue_stick(
    ps: &mut Particles,
    rng: &mut impl Rng,
    trap_pos: Vec2,
    victim_pos: Vec2,
    victim_radius: f32,
) {
    let tpos = safe_pos(trap_pos);
    let vpos = safe_pos(victim_pos);
    let vr = safe_clamp(victim_radius, 2.0, 12.0, 3.0);
    let to_v = safe_dir(vpos - tpos, Vec2::new(1.0, 0.0));
    let side = to_v.perp();

    // Soft non-additive honey-amber squelch ripple around the stuck feet.
    ps.push_shockwave(Shockwave {
        pos: vpos,
        start_r: (vr * 0.7).max(2.0),
        end_r: vr + 8.5,
        life: 0.28,
        max_life: 0.28,
        color: (224, 162, 48),
        core: (255, 250, 214),
        thick: 1.8,
        additive: false,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // Chunky viscous amber glue globs popping visibly past the cardboard tray rim (squelch!).
    let globs = ps.budget_count(10, false);
    for _ in 0..globs {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(38.0..72.0);
        let life = rng.random_range(0.32..0.52);
        ps.push(Particle {
            pos: vpos + Vec2::from_angle(a) * rng.random_range(2.5..vr + 3.5),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -18.0),
            life,
            max_life: life,
            drag: 3.4,
            gravity: 50.0,
            ramp: Ramp::Custom((255, 232, 134), (176, 112, 22)),
            additive: false,
            shape: Shape::Blob,
        });
    }

    // Glossy ivory-amber strand flicks spraying outward from the pad rim.
    let strands = ps.budget_count(8, false);
    for k in 0..strands {
        let flank = if k % 2 == 0 { 1.0 } else { -1.0 };
        let anchor = tpos + side * (flank * rng.random_range(2.0..5.0));
        let vel_dir = safe_dir(
            (vpos + side * (flank * (vr + 4.5))) - anchor,
            side * flank,
        );
        let life = rng.random_range(0.24..0.40);
        ps.push(Particle {
            pos: anchor,
            vel: vel_dir * rng.random_range(44.0..76.0) + Vec2::new(0.0, -10.0),
            life,
            max_life: life,
            drag: 4.8,
            gravity: 26.0,
            ramp: Ramp::Custom((255, 252, 226), (222, 166, 54)),
            additive: false,
            shape: Shape::Streak,
        });
    }

    // Fine adhesive mist droplets.
    let dots = ps.budget_count(8, false);
    ps.burst_shaped(
        rng,
        vpos,
        dots,
        62.0,
        vr * 0.8,
        Ramp::Custom((255, 248, 210), (192, 134, 34)),
        0.34,
        false,
        Shape::Dot,
        4.2,
    );

    // Glossy wet amber glue smudge on the floor around the victim's landing spot.
    ps.push_stain(Stain {
        pos: vpos,
        radius: (vr * 1.15).clamp(3.2, 6.8),
        color: (192, 134, 32),
        life: 8.5,
        max_life: 8.5,
        seed: rng.random::<u16>(),
        kind: StainKind::Splat,
    });
}

/// Snapping elastic glue strands, flying taffy droplets, and a crisp release pop
/// when a victim tears free of a glue board (`StickyTrapState::Stuck -> Used`).
pub fn emit_glue_tear_free(
    ps: &mut Particles,
    rng: &mut impl Rng,
    trap_pos: Vec2,
    victim_pos: Vec2,
    pull_dir: Vec2,
    victim_radius: f32,
) {
    let tpos = safe_pos(trap_pos);
    let vpos = safe_pos(victim_pos);
    let vr = safe_clamp(victim_radius, 2.0, 12.0, 3.0);
    let pull = safe_dir(pull_dir, safe_dir(vpos - tpos, Vec2::new(1.0, 0.0)));
    let back = -pull;
    let mid = (tpos + vpos) * 0.5;

    // Crisp directional release snap arc along pull direction.
    ps.push_shockwave(Shockwave {
        pos: vpos,
        start_r: (vr * 0.7).max(2.0),
        end_r: vr + 8.5,
        life: 0.22,
        max_life: 0.22,
        color: (246, 214, 118),
        core: (255, 254, 238),
        thick: 1.6,
        additive: true,
        arc_dir: Some(pull),
        arc_half_angle: 1.12,
    });

    // Bright snap glint where the main strand parts.
    if ps.budget_count(1, false) > 0 {
        ps.push(Particle {
            pos: mid,
            vel: Vec2::ZERO,
            life: 0.16,
            max_life: 0.16,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Custom((255, 254, 236), (236, 186, 78)),
            additive: true,
            shape: Shape::Cross,
        });
    }

    // Recoiling elastic taffy filaments snapping back onto the board.
    let snap_back = ps.budget_count(8, false);
    ps.cone_shaped(
        rng,
        mid,
        back,
        snap_back,
        88.0,
        0.65,
        Ramp::Custom((255, 252, 226), (212, 156, 48)),
        0.26,
        false,
        Shape::Streak,
        5.5,
    );

    // Trailing sticky threads whipping forward off the freed victim.
    let snap_fwd = ps.budget_count(8, false);
    ps.cone_shaped(
        rng,
        vpos,
        pull,
        snap_fwd,
        84.0,
        0.82,
        Ramp::Custom((255, 252, 220), (220, 166, 56)),
        0.28,
        false,
        Shape::Streak,
        4.8,
    );

    // Arcing torn glue blobs & droplets with gravity.
    let beads = ps.budget_count(8, false);
    for k in 0..beads {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(24.0..58.0);
        let life = rng.random_range(0.28..0.46);
        ps.push(Particle {
            pos: mid + Vec2::from_angle(a) * rng.random_range(0.0..3.0),
            vel: Vec2::from_angle(a) * sp + pull * 18.0 + Vec2::new(0.0, -16.0),
            life,
            max_life: life,
            drag: 3.4,
            gravity: 50.0,
            ramp: Ramp::Custom((252, 234, 162), (176, 118, 28)),
            additive: false,
            shape: if k < 3 { Shape::Blob } else { Shape::Dot },
        });
    }

    // Small torn adhesive residue smudge near the exit point.
    ps.push_stain(Stain {
        pos: vpos + pull * 1.5,
        radius: 2.2,
        color: (166, 120, 38),
        life: 6.5,
        max_life: 6.5,
        seed: rng.random::<u16>(),
        kind: StainKind::Splat,
    });
}

/// Sharp percussive snap flash, flying pine wood splinters, smashed cheese crumbs,
/// and lateral floor dust kick when a mousetrap striker bar slams shut.
///
/// Aligns with `art_traps::MOUSETRAP_SIZE` (`18×11 px`, strike zone at `+facing * 5.5`).
pub fn emit_mousetrap_snap(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, facing: Vec2) {
    let base = safe_pos(pos);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let perp = dir.perp();
    // Striker U-bar slams down onto the +facing bait end (`u ≈ +5.5..7.2`).
    let impact = base + dir * 5.5;

    // 1. Sharp metallic snap shockwave & blinding bar-impact cross glint.
    ps.push_shockwave(Shockwave {
        pos: impact,
        start_r: 2.0,
        end_r: 14.0,
        life: 0.17,
        max_life: 0.17,
        color: (255, 206, 108),
        core: (255, 255, 248),
        thick: 1.6,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    if ps.budget_count(1, true) > 0 {
        ps.push(Particle {
            pos: impact,
            vel: Vec2::ZERO,
            life: 0.14,
            max_life: 0.14,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Spark,
            additive: true,
            shape: Shape::Cross,
        });
    }

    // 2. Crisp steel-on-brass impact sparks shooting off the bar.
    let sparks = ps.budget_count(8, false);
    ps.burst_shaped(
        rng,
        impact,
        sparks,
        96.0,
        1.8,
        Ramp::Spark,
        0.18,
        true,
        Shape::Streak,
        6.5,
    );

    // 3. Pine baseboard wood splinters arcing cleanly outward with gravity.
    let splinters = ps.budget_count(9, false);
    for _ in 0..splinters {
        let a = dir.angle() + rng.random_range(-2.2..2.2);
        let sp = rng.random_range(52.0..96.0);
        let life = rng.random_range(0.28..0.46);
        ps.push(Particle {
            pos: impact + perp * rng.random_range(-4.0..4.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -20.0),
            life,
            max_life: life,
            drag: 3.0,
            gravity: 60.0,
            ramp: Ramp::Custom((244, 198, 126), (116, 70, 30)),
            additive: false,
            shape: Shape::Streak,
        });
    }

    // 4. Smashed golden cheese bait crumbs hopping clear.
    let crumbs = ps.budget_count(5, false);
    for _ in 0..crumbs {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(28.0..60.0);
        let life = rng.random_range(0.32..0.50);
        ps.push(Particle {
            pos: impact + Vec2::new(rng.random_range(-1.5..1.5), rng.random_range(-1.5..1.5)),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -18.0),
            life,
            max_life: life,
            drag: 2.8,
            gravity: 56.0,
            ramp: Ramp::Custom((255, 224, 92), (186, 128, 28)),
            additive: false,
            shape: Shape::Dot,
        });
    }

    // 5. Lateral & forward floor dust kick puffed out from under the slamming wooden board.
    let dust = ps.budget_count(10, false);
    for k in 0..dust {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let u_ofs = rng.random_range(-2.0..7.5);
        let v_ofs = side * rng.random_range(4.2..6.5);
        let life = rng.random_range(0.36..0.56);
        ps.push(Particle {
            pos: base + dir * u_ofs + perp * v_ofs,
            vel: perp * (side * rng.random_range(34.0..60.0))
                + dir * rng.random_range(8.0..24.0),
            life,
            max_life: life,
            drag: 4.5,
            gravity: 0.0,
            ramp: Ramp::Custom((228, 218, 196), (124, 114, 98)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 6. Subtle floor impact scuff under the strike bar.
    ps.push_stain(Stain {
        pos: impact,
        radius: 3.4,
        color: (58, 48, 38),
        life: 8.0,
        max_life: 8.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Dent,
    });
}

/// Crisp dust line along the `18 px` doorway threshold, directional exit-side
/// shockwave, and brass latch sparks when a one-way maze gate slams shut
/// (`OneWayDoorState::Closing` latch beat -> `Closed`).
///
/// Aligns with `art_traps::ONE_WAY_DOOR_PASSAGE_HALF_V` (`9.0 px`).
pub fn emit_one_way_gate_slam(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    facing: Vec2,
    accent: Rgb,
) {
    let base = safe_pos(pos);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let perp = dir.perp();
    // Closed V-flaps meet at the brass clasp around `u = +2.4`.
    let latch = base + dir * 2.4;

    // 1. Directional mechanical latch shockwave projecting out the +facing exit side.
    ps.push_shockwave(Shockwave {
        pos: latch,
        start_r: 2.5,
        end_r: 15.5,
        life: 0.24,
        max_life: 0.24,
        color: palette::mix(accent, (186, 224, 238), 0.45),
        core: (248, 252, 255),
        thick: 1.8,
        additive: true,
        arc_dir: Some(dir),
        arc_half_angle: 1.28,
    });

    // 2. Twin crisp dust ribbons kicked cleanly off BOTH sides of the 18 px threshold (`v in [-9.0, +9.0]`).
    let puffs = ps.budget_count(14, false);
    for k in 0..puffs {
        let frac = if puffs > 1 {
            k as f32 / (puffs - 1) as f32 * 2.0 - 1.0
        } else {
            0.0
        };
        let v = frac * 9.0 + rng.random_range(-0.35..0.35);
        let side_u = if k % 2 == 0 { 1.0 } else { -1.0 };
        let sill_u = if side_u > 0.0 { 4.2 } else { -3.8 };
        let life = rng.random_range(0.40..0.62);
        ps.push(Particle {
            pos: base + dir * sill_u + perp * v,
            vel: dir * (side_u * rng.random_range(34.0..58.0))
                + perp * rng.random_range(-3.0..3.0),
            life,
            max_life: life,
            drag: 4.4,
            gravity: 0.0,
            ramp: Ramp::Custom((238, 234, 222), (128, 126, 122)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // Fine threshold grit dots kicked along the sill.
    let grit = ps.budget_count(8, false);
    for _ in 0..grit {
        let v = rng.random_range(-8.5..8.5);
        let side_u = if rng.random::<bool>() { 1.0 } else { -1.0 };
        let life = rng.random_range(0.22..0.38);
        ps.push(Particle {
            pos: base + dir * 1.4 + perp * v,
            vel: dir * (side_u * rng.random_range(22.0..48.0))
                + perp * rng.random_range(-8.0..8.0),
            life,
            max_life: life,
            drag: 5.2,
            gravity: 0.0,
            ramp: Ramp::Custom((192, 188, 178), (82, 80, 76)),
            additive: false,
            shape: Shape::Dot,
        });
    }

    // 3. Brass-steel clasp glint and tiny metallic latch sparks at center (`v = 0`).
    if ps.budget_count(1, false) > 0 {
        ps.push(Particle {
            pos: latch,
            vel: Vec2::ZERO,
            life: 0.16,
            max_life: 0.16,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Spark,
            additive: true,
            shape: Shape::Cross,
        });
    }
    let sparks = ps.budget_count(5, false);
    ps.cone_shaped(
        rng,
        latch,
        dir,
        sparks,
        66.0,
        1.15,
        Ramp::Spark,
        0.18,
        true,
        Shape::Streak,
        7.0,
    );
}

/// Paired masonry-dust puffs blowing out of BOTH mouths of a wall rat-hole
/// (`-facing * 7.0` and `+facing * 7.0`) when the rat or a small creature
/// squeezes through (`RatHoleState::Squeezing`).
///
/// Aligns with `art_traps::RAT_HOLE_HALF_U` (`7.0`) and `RAT_HOLE_GAP_HALF_V` (`4.0`).
pub fn emit_rat_hole_squeeze(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    facing: Vec2,
    travel_dir: Vec2,
) {
    let base = safe_pos(pos);
    let mut axis = safe_dir(facing, Vec2::new(1.0, 0.0));
    // Orient `axis` so `+axis` is the exit mouth matching `travel_dir` when given.
    if travel_dir.x.is_finite()
        && travel_dir.y.is_finite()
        && travel_dir.len_sq() > 1e-4
        && travel_dir.dot(axis) < 0.0
    {
        axis = -axis;
    }
    let perp = axis.perp();

    // Entry mouth (-axis * 7.4) and exit mouth (+axis * 7.4) both get distinct puffs.
    let mouths: [(f32, usize, f32); 2] = [(-1.0, 5, 36.0), (1.0, 6, 48.0)];
    for &(sign, desired_puffs, base_speed) in &mouths {
        let mouth = base + axis * (sign * 7.4);
        let out = axis * sign;

        let puffs = ps.budget_count(desired_puffs, false);
        for k in 0..puffs {
            let spread_v = if puffs > 1 {
                (k as f32 / (puffs - 1) as f32 * 2.0 - 1.0) * 3.2
            } else {
                0.0
            };
            let life = rng.random_range(0.34..0.54);
            ps.push(Particle {
                pos: mouth + perp * (spread_v + rng.random_range(-0.5..0.5)),
                vel: out * rng.random_range(base_speed * 0.65..base_speed * 1.2)
                    + perp * (spread_v * 4.8 + rng.random_range(-6.0..6.0)),
                life,
                max_life: life,
                drag: 4.8,
                gravity: 0.0,
                ramp: Ramp::Custom((234, 222, 198), (124, 114, 96)),
                additive: false,
                shape: Shape::Puff,
            });
        }

        // Tiny gnawed plaster grit chips popping clear of each arch lip.
        let chips = ps.budget_count(4, false);
        ps.cone_shaped(
            rng,
            mouth,
            out,
            chips,
            base_speed * 1.15,
            0.78,
            Ramp::Custom((226, 214, 188), (124, 112, 92)),
            0.30,
            false,
            Shape::Dot,
            5.0,
        );
    }

    // Subtle speed whisk on the exit side showing the rat popping out cleanly.
    let exit_mouth = base + axis * 7.6;
    let whisks = ps.budget_count(3, false);
    ps.cone_shaped(
        rng,
        exit_mouth,
        axis,
        whisks,
        58.0,
        0.52,
        Ramp::Dash,
        0.18,
        true,
        Shape::Streak,
        7.5,
    );
}

/// Celebratory treasure-cache opening burst (antique gold coin fountain +
/// theme-`accent` crystal shards + lingering golden embers), intentionally
/// distinct from `emit_level_up` (which uses gold + rat-tail pink rings and
/// upward-floating stars).
///
/// Aligns with `art_traps::MAZE_REWARD_PEDESTAL_RADIUS` (`8.5 px`). Call once
/// on `MazeRewardState::Idle -> Opening`.
pub fn emit_maze_cache_open(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2, accent: Rgb) {
    let base = safe_pos(pos);
    let accent_hi = palette::mix(accent, (255, 255, 245), 0.45);

    // 1. Single broad antique-gold outer shockwave starting at the pedestal rim
    //    (complements the inner twin halo already drawn by `draw_maze_reward_ground`).
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 8.5,
        end_r: 40.0,
        life: 0.56,
        max_life: 0.56,
        color: (248, 188, 52),
        core: (255, 250, 222),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Tumbling bullion / gold coin fountain arcing wide out past the pedestal rim with gravity!
    let coins = ps.budget_count(16, true);
    for k in 0..coins {
        let a = k as f32 / coins.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.18..0.18);
        let horiz = rng.random_range(46.0..96.0);
        let up = rng.random_range(48.0..88.0);
        let life = rng.random_range(0.60..0.92);
        ps.push(Particle {
            pos: base + Vec2::from_angle(a) * rng.random_range(5.0..9.0),
            vel: Vec2::new(a.cos() * horiz, a.sin() * horiz * 0.45 - up),
            life,
            max_life: life,
            drag: 1.5,
            gravity: 96.0,
            ramp: Ramp::Gold,
            additive: true,
            shape: Shape::Blob,
        });
    }

    // 3. Map-accent crystal conduit rays bursting from the octagonal pedestal rim.
    let rays = ps.budget_count(14, true);
    for k in 0..rays {
        let a = k as f32 / rays.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.10..0.10);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.36..0.54);
        let ramp = if k % 3 == 0 {
            Ramp::Gold
        } else {
            Ramp::Custom(accent_hi, palette::scale(accent, 0.45))
        };
        ps.push(Particle {
            pos: base + out * 7.5,
            vel: out * rng.random_range(68.0..118.0),
            life,
            max_life: life,
            drag: 3.6,
            gravity: 0.0,
            ramp,
            additive: true,
            shape: Shape::Streak,
        });
    }

    // 4. Four cardinal brass-gold star glints right on the octagonal dais corners.
    let stars = ps.budget_count(4, true);
    for k in 0..stars {
        let a = k as f32 * std::f32::consts::FRAC_PI_2;
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.45..0.68);
        ps.push(Particle {
            pos: base + out * 8.5,
            vel: out * 14.0 + Vec2::new(0.0, -10.0),
            life,
            max_life: life,
            drag: 3.0,
            gravity: 0.0,
            ramp: Ramp::Gold,
            additive: true,
            shape: Shape::Cross,
        });
    }

    // 5. Lingering warm treasure dust embers hovering gently over the open reliquary.
    let embers = ps.budget_count(16, true);
    for _ in 0..embers {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let r = rng.random_range(3.0..12.0);
        let life = rng.random_range(0.75..1.25);
        ps.push(Particle {
            pos: base + Vec2::from_angle(a) * r,
            vel: Vec2::new(rng.random_range(-12.0..12.0), rng.random_range(-26.0..-8.0)),
            life,
            max_life: life,
            drag: 2.0,
            gravity: 0.0,
            ramp: Ramp::Gold,
            additive: true,
            shape: Shape::Ember,
        });
    }
}

// ---- Jungle Cat Boss / Apex Jaguar (art_boss_cat.rs) -----------------------

/// Crushing jaguar pounce landing: outward-rolling dust ring, gravel chips,
/// persistent cracked-floor stain, and Phase 2 fiery shockwave.
///
/// Pass `pos = view.target`, `radius = art_boss_cat::POUNCE_LAND_RADIUS` (`22.0`),
/// and `phase2 = hp_frac <= PHASE2_HP_FRAC` when `Pounce` crosses `POUNCE_LAND_PROGRESS` (`0.80`).
pub fn emit_boss_cat_pounce_land(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    radius: f32,
    phase2: bool,
) {
    let base = safe_pos(pos);
    let r = safe_clamp(radius, 8.0, 48.0, 22.0);

    // 1. Cracked-floor impact stain sized to spider-web visibly outside the jaguar's body.
    ps.push_stain(Stain {
        pos: base,
        radius: (r * 0.80).clamp(11.0, 19.0),
        color: (68, 60, 52),
        life: 14.0,
        max_life: 14.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Crack,
    });

    // 2. Ground shockwave (sandstone dust ring in P1; scorching molten ring to 38 px in P2).
    if phase2 {
        ps.push_shockwave(Shockwave {
            pos: base,
            start_r: r * 0.65,
            end_r: (r * 1.78).max(39.0),
            life: 0.42,
            max_life: 0.42,
            color: (255, 86, 42),
            core: (255, 240, 178),
            thick: 2.2,
            additive: true,
            arc_dir: None,
            arc_half_angle: 0.0,
        });
    } else {
        ps.push_shockwave(Shockwave {
            pos: base,
            start_r: r * 0.52,
            end_r: r * 1.24,
            life: 0.36,
            max_life: 0.36,
            color: (224, 206, 172),
            core: (250, 244, 224),
            thick: 2.0,
            additive: false,
            arc_dir: None,
            arc_half_angle: 0.0,
        });
    }

    // 3. Outward-rolling ring of heavy landing dust puffs starting around the landing perimeter
    //    (`0.72..0.96 * r`) so the billowing dust ring clearly surrounds the jaguar's torso.
    let puffs = ps.budget_count(if phase2 { 22 } else { 18 }, true);
    for k in 0..puffs {
        let a = k as f32 / puffs.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.12..0.12);
        let out = Vec2::from_angle(a);
        let start_d = r * rng.random_range(0.72..0.96);
        let sp = rng.random_range(44.0..76.0) * (r / 22.0).clamp(0.7, 1.4);
        let life = rng.random_range(0.46..0.72);
        ps.push(Particle {
            pos: base + out * start_d,
            vel: out * sp,
            life,
            max_life: life,
            drag: 4.0,
            gravity: 0.0,
            ramp: Ramp::Custom((232, 216, 186), (112, 102, 86)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 4. Heavy cracked-stone chunks hopping out past the landing rim with gravity.
    let chips = ps.budget_count(12, true);
    for _ in 0..chips {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(42.0..86.0);
        let life = rng.random_range(0.36..0.56);
        ps.push(Particle {
            pos: base + Vec2::from_angle(a) * rng.random_range(r * 0.55..r * 0.90),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -24.0),
            life,
            max_life: life,
            drag: 2.5,
            gravity: 76.0,
            ramp: Ramp::Custom((184, 172, 152), (68, 62, 54)),
            additive: false,
            shape: Shape::Blob,
        });
    }

    // 5. Phase 2 molten amber-crimson sparks radiating from the slam.
    if phase2 {
        let sparks = ps.budget_count(16, true);
        ps.burst_shaped(
            rng,
            base,
            sparks,
            118.0,
            r * 0.35,
            Ramp::Fire,
            0.34,
            true,
            Shape::Streak,
            4.2,
        );
    }
}

/// Kinetic claw-slash sparks and directional cutting shockwave arc for the
/// jaguar's `BossCatState::Swipe`.
///
/// Complements (does not duplicate) the 3 solid crescent ribbons painted by
/// `art_boss_cat::draw_swipe_claw_arcs`. Pass `origin = art_boss_cat::swipe_origin(&view)`.
pub fn emit_boss_cat_swipe(
    ps: &mut Particles,
    rng: &mut impl Rng,
    origin: Vec2,
    facing: Vec2,
    swipe_side: f32,
    phase2: bool,
) {
    let base = safe_pos(origin);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let side = if swipe_side.is_finite() && swipe_side < 0.0 {
        -1.0_f32
    } else {
        1.0_f32
    };
    let base_ang = dir.angle();

    // 1. Razor-edged directional cutting shockwave arc matching SWIPE_RADIUS (30) & SWIPE_HALF_ANGLE (1.134).
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 20.0,
        end_r: 35.0,
        life: 0.24,
        max_life: 0.24,
        color: if phase2 { (255, 86, 44) } else { (255, 206, 98) },
        core: (255, 254, 242),
        thick: 2.0,
        additive: true,
        arc_dir: Some(dir),
        arc_half_angle: 1.134,
    });

    // 2. Tangential razor sparks peeling off the 3 exact claw ribbon radii (17.5, 23.8, 29.8).
    let radii = [17.5_f32, 23.8, 29.8];
    let per_arc = ps.budget_count(5, false);
    let ramp = if phase2 {
        Ramp::Fire
    } else {
        Ramp::Custom((255, 252, 234), (234, 134, 46))
    };
    for &rad in &radii {
        for k in 0..per_arc {
            let frac = if per_arc > 1 {
                k as f32 / (per_arc - 1) as f32
            } else {
                0.5
            };
            // Sweep runs from +side * 1.134 toward -side * 1.134.
            let rel_a = side * 1.10 * (1.0 - 2.0 * frac) + rng.random_range(-0.08..0.08);
            let ang = base_ang + rel_a;
            let radial = Vec2::from_angle(ang);
            // Tangent points in sweep travel direction (`-side`).
            let tangent = Vec2::from_angle(ang - side * std::f32::consts::FRAC_PI_2);
            let life = rng.random_range(0.18..0.30);
            ps.push(Particle {
                pos: base + radial * rad,
                vel: tangent * rng.random_range(72.0..118.0) + radial * rng.random_range(16.0..34.0),
                life,
                max_life: life,
                drag: 5.0,
                gravity: 0.0,
                ramp,
                additive: true,
                shape: Shape::Streak,
            });
        }
    }

    // 3. Low floor dust flicks swept up at the outer edge of the slash.
    let dust = ps.budget_count(5, false);
    for _ in 0..dust {
        let ang = base_ang + rng.random_range(-1.05..1.05);
        let radial = Vec2::from_angle(ang);
        let tangent = Vec2::from_angle(ang - side * std::f32::consts::FRAC_PI_2);
        let life = rng.random_range(0.28..0.44);
        ps.push(Particle {
            pos: base + radial * rng.random_range(20.0..30.0),
            vel: tangent * 28.0 + radial * 18.0,
            life,
            max_life: life,
            drag: 4.8,
            gravity: 0.0,
            ramp: Ramp::Custom((204, 188, 158), (96, 86, 72)),
            additive: false,
            shape: Shape::Puff,
        });
    }
}

/// Propagating acoustic pressure shockwaves, outward air-blast streaks, and
/// floor dust gusts for `BossCatState::Roar`.
///
/// Pass `origin = art_boss_cat::roar_origin(&view)` (`pos + facing * 24.0`).
/// Call when `Roar` crosses `ROAR_BLAST_START` (`0.35`), and optionally again
/// at `progress = 0.65` in Phase 2.
pub fn emit_boss_cat_roar(ps: &mut Particles, rng: &mut impl Rng, origin: Vec2, phase2: bool) {
    let base = safe_pos(origin);

    // 1. Concentric propagating acoustic shockwaves out to ROAR_INNER_RADIUS (26) & ROAR_OUTER_RADIUS (44).
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 6.0,
        end_r: 44.0,
        life: 0.44,
        max_life: 0.44,
        color: if phase2 { (255, 86, 48) } else { (244, 180, 72) },
        core: (255, 252, 232),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 4.0,
        end_r: 26.0,
        life: 0.26,
        max_life: 0.26,
        color: if phase2 { (255, 168, 68) } else { (255, 230, 146) },
        core: (255, 255, 246),
        thick: 1.5,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Radial acoustic pressure wind streaks blasting outward from the jaws.
    let gusts = ps.budget_count(14, true);
    let gust_ramp = if phase2 {
        Ramp::Fire
    } else {
        Ramp::Custom((255, 246, 206), (222, 142, 46))
    };
    for k in 0..gusts {
        let a = k as f32 / gusts.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.12..0.12);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.24..0.38);
        ps.push(Particle {
            pos: base + out * rng.random_range(6.0..12.0),
            vel: out * rng.random_range(92.0..140.0),
            life,
            max_life: life,
            drag: 3.4,
            gravity: 0.0,
            ramp: gust_ramp,
            additive: true,
            shape: Shape::Streak,
        });
    }

    // 3. Floor dust blown radially outward past the jaguar's shoulders by the roar shock.
    let dust = ps.budget_count(12, false);
    for k in 0..dust {
        let a = k as f32 / dust.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.16..0.16);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.38..0.60);
        ps.push(Particle {
            pos: base + out * rng.random_range(18.0..26.0),
            vel: out * rng.random_range(44.0..76.0),
            life,
            max_life: life,
            drag: 3.8,
            gravity: 0.0,
            ramp: Ramp::Custom((226, 212, 184), (108, 98, 84)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 4. Phase 2 sonic-fang ember accents along the 8 projectile lanes.
    if phase2 {
        let fangs = ps.budget_count(8, true);
        for k in 0..fangs {
            let a = k as f32 / 8.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_4 * 0.5;
            let out = Vec2::from_angle(a);
            let life = rng.random_range(0.36..0.52);
            ps.push(Particle {
                pos: base + out * 11.0,
                vel: out * 76.0,
                life,
                max_life: life,
                drag: 2.2,
                gravity: 0.0,
                ramp: Ramp::Fire,
                additive: true,
                shape: Shape::Ember,
            });
        }
    }
}

/// Braking claw-skid dust plumes, floor-scrape sparks, and subtle claw scuff
/// stains when the jaguar overshoots a missed pounce and slides to a halt.
///
/// Call each tick while skidding (rate-independent via `dt`; pass `dt ≈ 0.12..0.16`
/// for a single discrete skid burst).
pub fn emit_boss_cat_skid(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    move_dir: Vec2,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 {
        return;
    }
    let base = safe_pos(pos);
    let fwd = safe_dir(move_dir, Vec2::new(1.0, 0.0));
    let perp = fwd.perp();
    let back = -fwd;

    // Rate-independent dust count (~68 puffs/sec across both planted fore/hind paw tracks
    // positioned cleanly outside the flanks so the braking V-wake reads clearly!).
    let expected_puffs = (dt * 68.0).clamp(1.0, 10.0);
    let puffs = ps.budget_count(expected_puffs.round() as usize, false);
    for k in 0..puffs {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let paw = base + fwd * rng.random_range(-12.0..12.0) + perp * (side * rng.random_range(8.5..11.2));
        let life = rng.random_range(0.38..0.64);
        ps.push(Particle {
            pos: paw,
            vel: back * rng.random_range(10.0..26.0) + perp * (side * rng.random_range(18.0..38.0)),
            life,
            max_life: life,
            drag: 4.2,
            gravity: 0.0,
            ramp: Ramp::Custom((234, 220, 190), (118, 108, 92)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // Claw-scrape sparks shooting backward from the digging claws outside the shoulders.
    let expected_sparks = (dt * 52.0).clamp(1.0, 8.0);
    let sparks = ps.budget_count(expected_sparks.round() as usize, false);
    for k in 0..sparks {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let claw = base + fwd * rng.random_range(2.0..14.0) + perp * (side * rng.random_range(8.0..10.5));
        let life = rng.random_range(0.18..0.28);
        ps.push(Particle {
            pos: claw,
            vel: back * rng.random_range(64.0..112.0) + perp * (side * rng.random_range(10.0..26.0)),
            life,
            max_life: life,
            drag: 5.0,
            gravity: 0.0,
            ramp: Ramp::Spark,
            additive: true,
            shape: Shape::Streak,
        });
    }

    // Occasional twin claw-gouge skid floor cracks (~10/sec).
    if rng.random::<f32>() < (dt * 10.0).min(0.90) {
        let side = if rng.random::<bool>() { 1.0 } else { -1.0 };
        ps.push_stain(Stain {
            pos: base + perp * (side * 8.5),
            radius: 3.0,
            color: (68, 60, 50),
            life: 8.5,
            max_life: 8.5,
            seed: rng.random::<u16>(),
            kind: StainKind::Crack,
        });
    }
}

/// Climactic apex-predator defeat burst for the Jungle Cat Boss: golden-ochre
/// & crimson shockwaves, elongated 51 px body collapse dust, rosette embers,
/// cracked impact stain, and rich ichor splatter.
pub fn emit_boss_cat_death(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    facing: Vec2,
    theme_blood: Rgb,
) {
    let base = safe_pos(pos);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let ichor = palette::mix(theme_blood, (198, 54, 38), 0.45);
    let stain_col = palette::scale(ichor, 0.56);

    // 1. Twin apex golden-ochre & crimson shockwaves.
    ps.push_shockwave(Shockwave {
        pos: base + dir * 4.0,
        start_r: 5.0,
        end_r: 54.0,
        life: 0.56,
        max_life: 0.56,
        color: (242, 168, 52),
        core: (255, 250, 224),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos: base + dir * 4.0,
        start_r: 3.0,
        end_r: 32.0,
        life: 0.34,
        max_life: 0.34,
        color: (238, 66, 44),
        core: (255, 232, 186),
        thick: 1.8,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Elongated collapse dust clouds framing haunch (-13.5), chest (+5.5), and head (+20.0).
    let centers = [base - dir * 13.5, base + dir * 5.5, base + dir * 20.0];
    let puffs_per = ps.budget_count(6, true);
    for &c in &centers {
        for _ in 0..puffs_per {
            let a = rng.random_range(0.0..std::f32::consts::TAU);
            let sp = rng.random_range(34.0..68.0);
            let life = rng.random_range(0.56..0.90);
            ps.push(Particle {
                pos: c + Vec2::from_angle(a) * rng.random_range(7.0..14.0),
                vel: Vec2::from_angle(a) * sp,
                life,
                max_life: life,
                drag: 3.2,
                gravity: 0.0,
                ramp: Ramp::Custom((228, 208, 174), (104, 92, 76)),
                additive: false,
                shape: Shape::Puff,
            });
        }
    }

    // 3. Blazing amber-crimson fury streaks + golden victory stars + ichor blobs.
    let streaks = ps.budget_count(32, true);
    ps.burst_shaped(
        rng,
        base + dir * 4.0,
        streaks,
        135.0,
        10.0,
        Ramp::Fire,
        0.50,
        true,
        Shape::Streak,
        3.6,
    );
    let stars = ps.budget_count(16, true);
    ps.burst_shaped(
        rng,
        base + dir * 4.0,
        stars,
        94.0,
        12.0,
        Ramp::Gold,
        0.74,
        true,
        Shape::Cross,
        3.0,
    );
    let gore = ps.budget_count(24, true);
    ps.burst_shaped(
        rng,
        base + dir * 2.0,
        gore,
        82.0,
        12.0,
        Ramp::Blood(ichor),
        0.78,
        false,
        Shape::Blob,
        2.8,
    );

    // 4. Rising rosette-gold embers across the slumped length.
    let embers = ps.budget_count(24, true);
    for _ in 0..embers {
        let u = rng.random_range(-18.0..22.0);
        let v = rng.random_range(-9.0..9.0);
        let life = rng.random_range(0.85..1.35);
        ps.push(Particle {
            pos: base + dir * u + dir.perp() * v,
            vel: Vec2::new(rng.random_range(-14.0..14.0), rng.random_range(-34.0..-12.0)),
            life,
            max_life: life,
            drag: 1.8,
            gravity: 0.0,
            ramp: Ramp::Gold,
            additive: true,
            shape: Shape::Ember,
        });
    }

    // 5. Heavy cracked floor impact under chest + twin ichor stains.
    ps.push_stain(Stain {
        pos: base + dir * 5.5,
        radius: 10.0,
        color: (44, 36, 32),
        life: 14.0,
        max_life: 14.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Crack,
    });
    ps.push_stain(Stain {
        pos: base - dir * 10.0,
        radius: 6.5,
        color: stain_col,
        life: 12.0,
        max_life: 12.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Splat,
    });
}

// ---- Broom Human Boss / The Sweeper (art_boss_broom.rs) --------------------

/// Heavy slipper stomp impact: shockwave and outward-rolling kitchen dust ring
/// scaled directly by `stomp_radius` (`STOMP_SHOCKWAVE_RADIUS = 16.0` for
/// `StompWalk`, or `PHASE_SLAM_RADIUS = 26.0` for `PhaseChange`), plus floor
/// grit and a subtle cracked floor-tremor stain.
pub fn emit_boss_broom_stomp(
    ps: &mut Particles,
    rng: &mut impl Rng,
    foot_pos: Vec2,
    stomp_radius: f32,
    enraged: bool,
) {
    let base = safe_pos(foot_pos);
    let r = safe_clamp(stomp_radius, 6.0, 48.0, 16.0);
    let scale = (r / 16.0).clamp(0.65, 2.0);

    // 1. Impact shockwave sized directly to the art's stomp radius.
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: (r * 0.42).max(3.5),
        end_r: r * 1.25,
        life: 0.30 + 0.08 * scale,
        max_life: 0.30 + 0.08 * scale,
        color: if enraged { (255, 92, 52) } else { (236, 224, 198) },
        core: if enraged { (255, 242, 184) } else { (255, 252, 242) },
        thick: if r >= 22.0 { 2.2 } else { 2.0 },
        additive: enraged,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Billowing outward dust ring scaled by `stomp_radius` (framed cleanly outside the slipper).
    let desired_puffs = (r * 0.95).round().clamp(12.0, 24.0) as usize;
    let puffs = ps.budget_count(desired_puffs, true);
    for k in 0..puffs {
        let a = k as f32 / puffs.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.14..0.14);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.42..0.66) * scale.sqrt();
        ps.push(Particle {
            pos: base + out * (r * rng.random_range(0.60..0.88)),
            vel: out * rng.random_range(42.0..74.0) * scale,
            life,
            max_life: life,
            drag: 3.8,
            gravity: 0.0,
            ramp: Ramp::Custom((238, 230, 210), (126, 118, 106)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 3. Hopping kitchen floor grit & crumbs jarred loose by the stomp.
    let crumbs = ps.budget_count((11.0 * scale).round() as usize, false);
    for _ in 0..crumbs {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(32.0..68.0) * scale;
        let life = rng.random_range(0.32..0.52);
        ps.push(Particle {
            pos: base + Vec2::from_angle(a) * rng.random_range(2.5..r * 0.65),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -22.0 * scale),
            life,
            max_life: life,
            drag: 2.8,
            gravity: 70.0,
            ramp: Ramp::Custom((228, 186, 112), (116, 90, 58)),
            additive: false,
            shape: Shape::Dot,
        });
    }

    // 4. Anger sparks on enraged stomps / PhaseChange tantrum slam.
    if enraged {
        let sparks = ps.budget_count((10.0 * scale).round() as usize, true);
        ps.burst_shaped(
            rng,
            base,
            sparks,
            92.0 * scale,
            r * 0.30,
            Ramp::Fire,
            0.28,
            true,
            Shape::Streak,
            4.8,
        );
    }

    // 5. Heavy heel impact tremor cracks on the kitchen tile.
    ps.push_stain(Stain {
        pos: base,
        radius: (r * 0.42).clamp(4.0, 11.5),
        color: (62, 54, 48),
        life: 9.0,
        max_life: 9.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Crack,
    });
}

/// Travelling bow-wave of billowing kitchen dust, tumbling bread/floor crumbs,
/// and flying corn-straw flecks along the sweeping broom bristles
/// (`r ∈ [12.0, 38.0]` at `sweep_angle`).
///
/// Call each tick during active `BroomBossState::BroomSweep` (`progress 0.46..0.72`),
/// passing `sweep_angle = art_boss_broom::broom_sweep_angle(&view)`.
pub fn emit_boss_broom_sweep(
    ps: &mut Particles,
    rng: &mut impl Rng,
    boss_pos: Vec2,
    sweep_angle: f32,
    clockwise: bool,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 || !sweep_angle.is_finite() {
        return;
    }
    let base = safe_pos(boss_pos);
    let sign = if clockwise { 1.0_f32 } else { -1.0_f32 };
    let radial = Vec2::from_angle(sweep_angle);
    let tangent = Vec2::from_angle(sweep_angle + sign * std::f32::consts::FRAC_PI_2);

    // 1. Travelling wave of billowing dust puffs along the 12..38 px bristle band.
    let expected_puffs = (dt * 92.0).clamp(1.0, 12.0);
    let puffs = ps.budget_count(expected_puffs.round() as usize, false);
    for _ in 0..puffs {
        let r = rng.random_range(13.0..37.5);
        let sp = 48.0 + r * 1.25;
        let life = rng.random_range(0.34..0.56);
        ps.push(Particle {
            pos: base + radial * r + tangent * rng.random_range(-1.5..3.5),
            vel: tangent * sp + radial * rng.random_range(10.0..26.0),
            life,
            max_life: life,
            drag: 4.0,
            gravity: 0.0,
            ramp: Ramp::Custom((234, 226, 206), (126, 118, 104)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 2. Tumbling kitchen crumbs & floor debris swept ahead of the bristles.
    let expected_crumbs = (dt * 115.0).clamp(1.0, 14.0);
    let crumbs = ps.budget_count(expected_crumbs.round() as usize, false);
    for k in 0..crumbs {
        let r = rng.random_range(12.5..38.0);
        let sp = 64.0 + r * 1.45;
        let life = rng.random_range(0.32..0.52);
        let ramp = match k % 3 {
            0 => Ramp::Custom((238, 190, 104), (138, 96, 44)),  // toasted bread crumb
            1 => Ramp::Custom((248, 242, 226), (156, 150, 136)), // pale salt/flour fleck
            _ => Ramp::Custom((184, 160, 126), (86, 72, 56)),  // pantry grit
        };
        ps.push(Particle {
            pos: base + radial * r + tangent * rng.random_range(0.5..4.5),
            vel: tangent * sp + radial * rng.random_range(16.0..38.0) + Vec2::new(0.0, -12.0),
            life,
            max_life: life,
            drag: 3.0,
            gravity: 34.0,
            ramp,
            additive: false,
            shape: if k % 3 == 0 { Shape::Blob } else { Shape::Dot },
        });
    }

    // 3. Corn-straw bristle flecks snapping off the outer tips.
    let expected_straw = (dt * 42.0).clamp(1.0, 6.0);
    let straws = ps.budget_count(expected_straw.round() as usize, false);
    for _ in 0..straws {
        let r = rng.random_range(24.0..38.0);
        let life = rng.random_range(0.24..0.40);
        ps.push(Particle {
            pos: base + radial * r,
            vel: tangent * rng.random_range(76.0..118.0) + radial * rng.random_range(18.0..36.0),
            life,
            max_life: life,
            drag: 3.8,
            gravity: 22.0,
            ramp: Ramp::Custom((255, 226, 124), (164, 114, 42)),
            additive: false,
            shape: Shape::Streak,
        });
    }
}

/// Galvanized tin dustpan crash: metallic clatter shockwave, 6-lane radial shard
/// sparks (aligned with `art_boss_broom::DUSTPAN_SHARD_COUNT = 6`), spilled
/// dustpan dust & crumbs, and a floor impact dent stain.
///
/// Pass `impact_radius = art_boss_broom::DUSTPAN_IMPACT_RADIUS` (`15.0`) when
/// `DustpanDrop` reaches `progress >= 0.48`.
pub fn emit_boss_broom_dustpan_drop(
    ps: &mut Particles,
    rng: &mut impl Rng,
    target_pos: Vec2,
    impact_radius: f32,
) {
    let base = safe_pos(target_pos);
    let r = safe_clamp(impact_radius, 6.0, 36.0, 15.0);

    // 1. Crisp galvanized-tin clatter shockwave.
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 3.0,
        end_r: r * 1.18,
        life: 0.28,
        max_life: 0.28,
        color: (158, 184, 208),
        core: (242, 248, 255),
        thick: 1.9,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. 6-lane metallic clatter sparks aligned to the exact shard angles (`k * TAU / 6 + 0.25`).
    let lanes = ps.budget_count(12, true);
    for k in 0..lanes {
        let a = (k % 6) as f32 * (std::f32::consts::TAU / 6.0)
            + 0.25
            + rng.random_range(-0.10..0.10);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.24..0.38);
        ps.push(Particle {
            pos: base + out * rng.random_range(4.0..7.5),
            vel: out * rng.random_range(92.0..146.0),
            life,
            max_life: life,
            drag: 4.2,
            gravity: 0.0,
            ramp: if k < 6 {
                Ramp::Custom((244, 250, 255), (126, 150, 174))
            } else {
                Ramp::Spark
            },
            additive: true,
            shape: Shape::Streak,
        });
    }

    // 3. Spilled dustpan dust cloud + rattling floor crumbs.
    let puffs = ps.budget_count(10, false);
    for k in 0..puffs {
        let a = k as f32 / puffs.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.2..0.2);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.38..0.60);
        ps.push(Particle {
            pos: base + out * rng.random_range(2.0..r * 0.45),
            vel: out * rng.random_range(24.0..48.0),
            life,
            max_life: life,
            drag: 4.0,
            gravity: 0.0,
            ramp: Ramp::Custom((210, 204, 192), (108, 102, 96)),
            additive: false,
            shape: Shape::Puff,
        });
    }
    let crumbs = ps.budget_count(10, false);
    for _ in 0..crumbs {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(26.0..60.0);
        let life = rng.random_range(0.32..0.52);
        ps.push(Particle {
            pos: base + Vec2::from_angle(a) * rng.random_range(1.0..4.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -24.0),
            life,
            max_life: life,
            drag: 2.8,
            gravity: 66.0,
            ramp: Ramp::Custom((218, 182, 114), (108, 86, 56)),
            additive: false,
            shape: Shape::Dot,
        });
    }

    // 4. Metallic pan dent scuff on the kitchen floor.
    ps.push_stain(Stain {
        pos: base,
        radius: (r * 0.38).clamp(4.0, 8.0),
        color: (68, 76, 86),
        life: 10.0,
        max_life: 10.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Dent,
    });
}

/// Comedic stubbed-toe burst + hopping slipper landing dust puff for
/// `BroomBossState::Hurt`.
///
/// Complements `art_boss_broom::draw_hurt_stars` (which draws tight orbiting
/// stars on the lifted toe) with wider screen-readable comic pain stars and a
/// soft floor dust puff under the hopping foot.
pub fn emit_boss_broom_hurt_hop(
    ps: &mut Particles,
    rng: &mut impl Rng,
    hop_foot_pos: Vec2,
    hurt_foot_pos: Vec2,
) {
    let hop = safe_pos(hop_foot_pos);
    let hurt = safe_pos(hurt_foot_pos);

    // 1. Comic stubbed-toe pain stars & impact ticks bursting around the hurt toe.
    let stars = ps.budget_count(6, false);
    for k in 0..stars {
        let a = k as f32 / stars.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.18..0.18);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.36..0.58);
        let ramp = if k % 2 == 0 {
            Ramp::Gold
        } else {
            Ramp::Custom((255, 142, 100), (196, 52, 46))
        };
        ps.push(Particle {
            pos: hurt + out * rng.random_range(4.0..7.5) + Vec2::new(0.0, -5.0),
            vel: out * rng.random_range(34.0..64.0) + Vec2::new(0.0, -20.0),
            life,
            max_life: life,
            drag: 3.0,
            gravity: 16.0,
            ramp,
            additive: true,
            shape: Shape::Cross,
        });
    }

    // 2. Soft hop-landing dust ring & floor puffs under the hopping slipper.
    ps.push_shockwave(Shockwave {
        pos: hop,
        start_r: 3.0,
        end_r: 12.5,
        life: 0.26,
        max_life: 0.26,
        color: (228, 218, 198),
        core: (250, 246, 234),
        thick: 1.6,
        additive: false,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    let puffs = ps.budget_count(8, false);
    for k in 0..puffs {
        let a = k as f32 / puffs.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.2..0.2);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.32..0.50);
        ps.push(Particle {
            pos: hop + out * rng.random_range(3.5..6.5),
            vel: out * rng.random_range(24.0..44.0),
            life,
            max_life: life,
            drag: 4.4,
            gravity: 0.0,
            ramp: Ramp::Custom((230, 222, 204), (124, 118, 106)),
            additive: false,
            shape: Shape::Puff,
        });
    }
}

/// Indignant broom-drop & playful victory burst when **The Sweeper** gives up
/// (`BroomBossState::Defeat`): corn-straw burst and kitchen dust cloud where
/// the broom slams flat, plus celebratory golden-ivory shockwaves & stars.
pub fn emit_boss_broom_defeat(
    ps: &mut Particles,
    rng: &mut impl Rng,
    boss_pos: Vec2,
    broom_pos: Vec2,
) {
    let boss = safe_pos(boss_pos);
    let broom = safe_pos(broom_pos);

    // 1. Celebratory warm golden-ivory & slipper-oxblood shockwaves around the boss.
    ps.push_shockwave(Shockwave {
        pos: boss,
        start_r: 4.0,
        end_r: 46.0,
        life: 0.54,
        max_life: 0.54,
        color: (250, 212, 108),
        core: (255, 254, 238),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos: broom,
        start_r: 3.0,
        end_r: 22.0,
        life: 0.34,
        max_life: 0.34,
        color: (218, 204, 176),
        core: (250, 244, 224),
        thick: 1.6,
        additive: false,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Dropped broom straw flecks & billowing dust cloud at `broom_pos`.
    let puffs = ps.budget_count(14, true);
    for k in 0..puffs {
        let a = k as f32 / puffs.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.2..0.2);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.45..0.78);
        ps.push(Particle {
            pos: broom + out * rng.random_range(2.0..7.5),
            vel: out * rng.random_range(24.0..54.0),
            life,
            max_life: life,
            drag: 3.6,
            gravity: 0.0,
            ramp: Ramp::Custom((220, 212, 194), (116, 110, 98)),
            additive: false,
            shape: Shape::Puff,
        });
    }
    let straws = ps.budget_count(18, true);
    for _ in 0..straws {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let sp = rng.random_range(34.0..84.0);
        let life = rng.random_range(0.38..0.64);
        ps.push(Particle {
            pos: broom + Vec2::from_angle(a) * rng.random_range(1.0..5.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -22.0),
            life,
            max_life: life,
            drag: 2.8,
            gravity: 52.0,
            ramp: Ramp::Custom((250, 218, 118), (150, 102, 34)),
            additive: false,
            shape: Shape::Streak,
        });
    }

    // 3. Playful ascending golden victory stars & warm motes around the retreating human.
    let stars = ps.budget_count(18, true);
    for k in 0..stars {
        let a = k as f32 / stars.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.18..0.18);
        let sp = rng.random_range(32.0..82.0);
        let life = rng.random_range(0.55..0.92);
        ps.push(Particle {
            pos: boss + Vec2::from_angle(a) * rng.random_range(3.0..10.0),
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -18.0),
            life,
            max_life: life,
            drag: 2.6,
            gravity: -6.0,
            ramp: if k % 3 == 0 {
                Ramp::Custom((244, 152, 116), (156, 62, 46))
            } else {
                Ramp::Gold
            },
            additive: true,
            shape: Shape::Cross,
        });
    }
    let embers = ps.budget_count(14, true);
    ps.burst_shaped(
        rng,
        boss,
        embers,
        58.0,
        10.0,
        Ramp::Gold,
        0.95,
        true,
        Shape::Ember,
        2.2,
    );

    // 4. Dusty broom-drop scuff left on the floor under the abandoned broom.
    ps.push_stain(Stain {
        pos: broom,
        radius: 6.0,
        color: (116, 104, 84),
        life: 11.0,
        max_life: 11.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Splat,
    });
}

// ---- Robot Lawnmower Boss / TURF-REAPER MK.IV (art_boss_mower.rs) ----------

/// Continuous charge/ricochet trail kicking out either vibrant chopped grass
/// clippings (`scorched == false`) or roaring orange-gold blade-friction sparks
/// (`scorched == true`) from the side chute and rear drive wheels.
///
/// Call each tick during `MowerState::Charge` and `MowerState::Ricochet`
/// (rate-independent via `dt`; aligns with `BLADE_DECK_RADIUS = 12.0`).
pub fn emit_boss_mower_charge_trail(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    facing: Vec2,
    scorched: bool,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 {
        return;
    }
    let base = safe_pos(pos);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let perp = dir.perp();
    let back = -dir;

    if scorched {
        // Hot metallic orange-gold sparks spraying off rear treads & scraping blade skirt!
        let expected_sparks = (dt * 95.0).clamp(1.0, 10.0);
        let sparks = ps.budget_count(expected_sparks.round() as usize, false);
        for k in 0..sparks {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let origin = base + back * rng.random_range(6.0..12.5) + perp * (side * rng.random_range(6.5..10.5));
            let life = rng.random_range(0.16..0.30);
            ps.push(Particle {
                pos: origin,
                vel: back * rng.random_range(58.0..108.0) + perp * (side * rng.random_range(12.0..34.0)),
                life,
                max_life: life,
                drag: 4.5,
                gravity: 0.0,
                ramp: if k % 3 == 0 { Ramp::Spark } else { Ramp::Fire },
                additive: true,
                shape: Shape::Streak,
            });
        }
        // Dark scorch-ash smoke wisps trailing behind.
        let expected_smoke = (dt * 30.0).clamp(0.4, 4.0);
        let smoke = ps.budget_count(expected_smoke.round() as usize, false);
        for _ in 0..smoke {
            let life = rng.random_range(0.28..0.46);
            ps.push(Particle {
                pos: base + back * 12.0 + perp * rng.random_range(-7.0..7.0),
                vel: back * rng.random_range(18.0..34.0) + Vec2::new(0.0, -10.0),
                life,
                max_life: life,
                drag: 3.6,
                gravity: 0.0,
                ramp: Ramp::Smoke,
                additive: false,
                shape: Shape::Puff,
            });
        }
    } else {
        // Vibrant chopped grass-clipping blades spraying out the side chute & rear deck!
        let expected_clips = (dt * 125.0).clamp(1.0, 14.0);
        let clips = ps.budget_count(expected_clips.round() as usize, false);
        for k in 0..clips {
            let side = if k % 4 == 0 { -1.0 } else { 1.0 }; // heavier discharge out +perp chute
            let origin = base + back * rng.random_range(0.0..10.0) + perp * (side * rng.random_range(9.0..12.0));
            let life = rng.random_range(0.26..0.46);
            let ramp = if k % 2 == 0 {
                Ramp::Custom((194, 255, 118), (64, 138, 36))
            } else {
                Ramp::Custom((104, 212, 66), (40, 94, 26))
            };
            ps.push(Particle {
                pos: origin,
                vel: perp * (side * rng.random_range(46.0..88.0))
                    + back * rng.random_range(26.0..62.0)
                    + Vec2::new(0.0, -12.0),
                life,
                max_life: life,
                drag: 3.2,
                gravity: 42.0,
                ramp,
                additive: false,
                shape: if k % 3 == 0 { Shape::Dot } else { Shape::Streak },
            });
        }
        // Subtle green-dust tire wake behind the rear treads.
        let expected_dust = (dt * 24.0).clamp(0.4, 3.0);
        let dust = ps.budget_count(expected_dust.round() as usize, false);
        for k in 0..dust {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let life = rng.random_range(0.28..0.44);
            ps.push(Particle {
                pos: base + back * 11.5 + perp * (side * 8.8),
                vel: back * rng.random_range(14.0..28.0) + perp * (side * 8.0),
                life,
                max_life: life,
                drag: 4.4,
                gravity: 0.0,
                ramp: Ramp::Custom((168, 184, 152), (82, 92, 74)),
                additive: false,
                shape: Shape::Puff,
            });
        }
    }
}

/// High-energy wall collision burst when the mower slams into a wall or steel
/// bumper during `Charge` or `Ricochet`: welding & cyan electrical sparks,
/// directional rebound shockwave, orange/steel paint chips, and a persistent
/// concave dent floor stain.
///
/// Pass `impact_pos = pos + facing * CHASSIS_HALF_LEN`, `wall_normal` pointing
/// away from the wall into the arena, and `heavy_flip = true` if the slam flips
/// the mower into `MowerState::Flipped`.
pub fn emit_boss_mower_wall_impact(
    ps: &mut Particles,
    rng: &mut impl Rng,
    impact_pos: Vec2,
    wall_normal: Vec2,
    heavy_flip: bool,
) {
    let base = safe_pos(impact_pos);
    let normal = safe_dir(wall_normal, Vec2::new(1.0, 0.0));
    let tangent = normal.perp();

    // 1. Concave impact dent stain right at the wall contact point.
    ps.push_stain(Stain {
        pos: base,
        radius: if heavy_flip { 7.6 } else { 5.6 },
        color: (54, 62, 72),
        life: 15.0,
        max_life: 15.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Dent,
    });

    // 2. Directional impact shockwave opening wide into the arena along `wall_normal`.
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 4.5,
        end_r: if heavy_flip { 28.0 } else { 20.0 },
        life: if heavy_flip { 0.30 } else { 0.22 },
        max_life: if heavy_flip { 0.30 } else { 0.22 },
        color: if heavy_flip { (255, 112, 34) } else { (255, 206, 92) },
        core: (255, 254, 240),
        thick: if heavy_flip { 2.2 } else { 1.8 },
        additive: true,
        arc_dir: Some(normal),
        arc_half_angle: 1.48,
    });

    // 3. Blazing white-orange welding sparks fanning wide along BOTH wall-tangent flanks
    //    (±tangent + normal) so sparks visibly explode out around the mower bumper!
    let hot_sparks = ps.budget_count(if heavy_flip { 24 } else { 14 }, true);
    let max_sp = if heavy_flip { 190.0 } else { 145.0 };
    for k in 0..hot_sparks {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let fan_dir = safe_dir(
            tangent * (side * rng.random_range(0.65..1.35)) + normal * rng.random_range(0.25..0.95),
            normal,
        );
        let life = rng.random_range(0.24..0.38);
        ps.push(Particle {
            pos: base + tangent * (side * rng.random_range(1.5..8.5)) + normal * 1.5,
            vel: fan_dir * rng.random_range(max_sp * 0.55..max_sp),
            life,
            max_life: life,
            drag: 3.4,
            gravity: 0.0,
            ramp: if k % 3 == 0 { Ramp::Fire } else { Ramp::Spark },
            additive: true,
            shape: Shape::Streak,
        });
    }

    // 4. Crackling cyan electrical short-circuit sparks snapping wide along the wall face.
    let elec = ps.budget_count(if heavy_flip { 14 } else { 8 }, true);
    for k in 0..elec {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let vel_dir = safe_dir(tangent * side * 1.4 + normal * rng.random_range(0.10..0.60), normal);
        let life = rng.random_range(0.22..0.38);
        ps.push(Particle {
            pos: base + tangent * (side * rng.random_range(3.0..10.5)),
            vel: vel_dir * rng.random_range(108.0..176.0),
            life,
            max_life: life,
            drag: 4.0,
            gravity: 0.0,
            ramp: Ramp::Custom((240, 255, 255), (104, 212, 248)),
            additive: true,
            shape: if k % 4 == 0 { Shape::Cross } else { Shape::Streak },
        });
    }

    // 5. Safety-orange ABS cowl chips & gunmetal flakes popping off with gravity.
    let chips = ps.budget_count(if heavy_flip { 7 } else { 4 }, false);
    for k in 0..chips {
        let a = normal.angle() + rng.random_range(-1.1..1.1);
        let sp = rng.random_range(34.0..76.0);
        let life = rng.random_range(0.32..0.52);
        let ramp = if k % 2 == 0 {
            Ramp::Custom((255, 148, 52), (140, 48, 16))
        } else {
            Ramp::Custom((164, 180, 198), (52, 60, 72))
        };
        ps.push(Particle {
            pos: base + normal * 1.5,
            vel: Vec2::from_angle(a) * sp + Vec2::new(0.0, -20.0),
            life,
            max_life: life,
            drag: 2.6,
            gravity: 72.0,
            ramp,
            additive: false,
            shape: Shape::Blob,
        });
    }

    // 6. Heavy flip impact dust plume & stunned cross glint.
    if heavy_flip {
        let puffs = ps.budget_count(8, false);
        for k in 0..puffs {
            let frac = (k as f32 / puffs.max(1) as f32) * 2.0 - 1.0;
            let life = rng.random_range(0.38..0.60);
            ps.push(Particle {
                pos: base + tangent * (frac * 7.0) + normal * 2.0,
                vel: normal * rng.random_range(26.0..52.0) + tangent * (frac * 24.0),
                life,
                max_life: life,
                drag: 4.2,
                gravity: 0.0,
                ramp: Ramp::Smoke,
                additive: false,
                shape: Shape::Puff,
            });
        }
        if ps.budget_count(1, true) > 0 {
            ps.push(Particle {
                pos: base + normal * 2.0,
                vel: Vec2::ZERO,
                life: 0.16,
                max_life: 0.16,
                drag: 0.0,
                gravity: 0.0,
                ramp: Ramp::Spark,
                additive: true,
                shape: Shape::Cross,
            });
        }
    }
}

/// Persistent drifting dark oily smoke plumes and sputtering PCB/battery sparks
/// rising from the belly-up mower during `MowerState::Flipped` (`2.0×` punish window).
///
/// Call each tick while `Flipped` (rate-independent via `dt`).
pub fn emit_boss_mower_flipped_smoke(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 {
        return;
    }
    let base = safe_pos(pos);

    // 1. Rising warm-soot oily engine smoke plumes drifting wide above the overturned deck
    //    (visibly grungy warm-ash charcoal so it reads clearly against dark Lab tiles while
    //     staying unmistakably darker & smokier than Overheat's bright white-cyan steam).
    let expected_smoke = (dt * 44.0).clamp(0.65, 7.0);
    let smoke = if expected_smoke < 1.0 {
        usize::from(rng.random::<f32>() < expected_smoke && ps.budget_count(1, false) > 0)
    } else {
        ps.budget_count(expected_smoke.round() as usize, false)
    };
    for _ in 0..smoke {
        let life = rng.random_range(0.75..1.20);
        ps.push(Particle {
            pos: base + Vec2::new(rng.random_range(-10.0..10.0), rng.random_range(-8.0..3.0)),
            vel: Vec2::new(rng.random_range(-18.0..18.0), rng.random_range(-44.0..-24.0)),
            life,
            max_life: life,
            drag: 1.4,
            gravity: -4.0,
            ramp: Ramp::Custom((170, 152, 136), (76, 70, 68)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 2. Lively exposed-belly cyan/PCB-green short-circuit sputter sparks & arc pops.
    let expected_sparks = (dt * 36.0).clamp(0.5, 5.0);
    let sparks = if expected_sparks < 1.0 {
        usize::from(rng.random::<f32>() < expected_sparks && ps.budget_count(1, false) > 0)
    } else {
        ps.budget_count(expected_sparks.round() as usize, false)
    };
    for k in 0..sparks {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let life = rng.random_range(0.20..0.36);
        let pcb = k % 2 == 0;
        let glint = k % 3 == 0;
        ps.push(Particle {
            pos: base + Vec2::new(rng.random_range(-8.0..8.0), rng.random_range(-6.5..6.5)),
            vel: Vec2::from_angle(a) * rng.random_range(52.0..96.0),
            life,
            max_life: life,
            drag: 4.4,
            gravity: 0.0,
            ramp: if pcb {
                Ramp::Custom((190, 255, 212), (46, 168, 92))
            } else {
                Ramp::Custom((242, 255, 255), (108, 218, 252))
            },
            additive: true,
            shape: if glint { Shape::Cross } else { Shape::Streak },
        });
    }
}

/// Pressurized white-cyan steam clouds hissing from the mower's rear cooling
/// louvers (`pos - facing * 8.6`) during `MowerState::Overheat` (`1.5×` punish window),
/// visibly distinct from the darker oily smoke of `Flipped`.
///
/// Call each tick while `Overheat` (rate-independent via `dt`).
pub fn emit_boss_mower_overheat_steam(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    facing: Vec2,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 {
        return;
    }
    let base = safe_pos(pos);
    let dir = safe_dir(facing, Vec2::new(1.0, 0.0));
    let perp = dir.perp();
    // Matches `art_boss_mower::draw_overheat_smoke` rear vent anchor at `u = -8.6`.
    let vent = base - dir * 8.6;

    // 1. Twin pressurized bright white-cyan steam jets hissing far backward & upward!
    let expected_steam = (dt * 48.0).clamp(0.65, 8.0);
    let steam = if expected_steam < 1.0 {
        usize::from(rng.random::<f32>() < expected_steam && ps.budget_count(1, false) > 0)
    } else {
        ps.budget_count(expected_steam.round() as usize, false)
    };
    for k in 0..steam {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let life = rng.random_range(0.60..1.00);
        ps.push(Particle {
            pos: vent + perp * (side * rng.random_range(2.0..5.2)),
            vel: (-dir) * rng.random_range(26.0..50.0)
                + perp * (side * rng.random_range(12.0..28.0))
                + Vec2::new(0.0, rng.random_range(-48.0..-26.0)),
            life,
            max_life: life,
            drag: 1.7,
            gravity: -8.0,
            ramp: Ramp::Custom((252, 255, 255), (176, 206, 228)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 2. Glowing orange radiator heat-shimmer embers venting from the overheated motor.
    if rng.random::<f32>() < (dt * 16.0).min(0.85) && ps.budget_count(1, false) > 0 {
        let life = rng.random_range(0.34..0.58);
        ps.push(Particle {
            pos: vent + perp * rng.random_range(-4.0..4.0),
            vel: (-dir) * rng.random_range(16.0..34.0) + Vec2::new(0.0, rng.random_range(-34.0..-16.0)),
            life,
            max_life: life,
            drag: 1.8,
            gravity: 0.0,
            ramp: Ramp::Fire,
            additive: true,
            shape: Shape::Ember,
        });
    }
}

/// Crisp steel-cyan spinning saw-tooth glint trail behind a flung mower blade
/// disc projectile (`art_boss_mower::BLADE_DISC_RADIUS = 3.5`).
///
/// Call each tick per active blade-disc projectile (rate-independent & budget-throttled).
pub fn emit_boss_mower_blade_trail(
    ps: &mut Particles,
    rng: &mut impl Rng,
    pos: Vec2,
    vel: Vec2,
    dt: f32,
) {
    let dt = safe_dt(dt);
    if dt <= 0.0 {
        return;
    }
    let base = safe_pos(pos);
    let back = -safe_dir(vel, Vec2::new(1.0, 0.0));
    let perp = back.perp();

    // ~46 glints/sec per disc so flying saw discs leave sleek, unmistakable metallic wakes.
    let expected = (dt * 46.0).min(4.0);
    let count = if expected < 1.0 {
        usize::from(rng.random::<f32>() < expected && ps.budget_count(1, false) > 0)
    } else {
        ps.budget_count(expected.round() as usize, false)
    };
    for k in 0..count {
        let side = if (k % 2 == 0) ^ rng.random::<bool>() { 1.0 } else { -1.0 };
        let life = rng.random_range(0.20..0.34);
        let glint_streak = rng.random_range(0..5) != 0;
        ps.push(Particle {
            pos: base + perp * (side * rng.random_range(1.6..3.5)) + back * 1.2,
            vel: back * rng.random_range(56.0..96.0) + perp * (side * rng.random_range(12.0..26.0)),
            life,
            max_life: life,
            drag: 3.8,
            gravity: 0.0,
            ramp: Ramp::Custom((244, 254, 255), (122, 204, 238)),
            additive: true,
            shape: if glint_streak { Shape::Streak } else { Shape::Dot },
        });
    }
}

/// Massive mechanical capacitor meltdown explosion when the Robot Lawnmower Boss
/// detonates (`MowerState::Dying` crossing `progress >= 0.44`): triple shockwaves
/// matching `DEATH_BLAST_RADIUS = 28.0` and beyond, safety-orange fireballs,
/// electric cyan arcs, billowing soot clouds, and a charred cracked crater stain.
pub fn emit_boss_mower_death(ps: &mut Particles, rng: &mut impl Rng, pos: Vec2) {
    let base = safe_pos(pos);

    // 1. Triple shockwaves: capacitor flash, main DEATH_BLAST_RADIUS wave, outer concussion wave.
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 4.0,
        end_r: 32.0,
        life: 0.34,
        max_life: 0.34,
        color: (255, 112, 34),
        core: (255, 255, 238),
        thick: 2.2,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 6.0,
        end_r: 58.0,
        life: 0.60,
        max_life: 0.60,
        color: (255, 198, 72),
        core: (255, 248, 212),
        thick: 2.0,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });
    ps.push_shockwave(Shockwave {
        pos: base,
        start_r: 2.0,
        end_r: 20.0,
        life: 0.22,
        max_life: 0.22,
        color: (140, 236, 255),
        core: (250, 255, 255),
        thick: 1.6,
        additive: true,
        arc_dir: None,
        arc_half_angle: 0.0,
    });

    // 2. Blazing safety-orange detonation streaks + crackling cyan capacitor arcs.
    let fire_streaks = ps.budget_count(32, true);
    ps.burst_shaped(
        rng,
        base,
        fire_streaks,
        148.0,
        5.0,
        Ramp::Fire,
        0.52,
        true,
        Shape::Streak,
        3.4,
    );
    let elec_streaks = ps.budget_count(16, true);
    ps.burst_shaped(
        rng,
        base,
        elec_streaks,
        122.0,
        6.0,
        Ramp::Custom((240, 254, 255), (96, 214, 252)),
        0.38,
        true,
        Shape::Streak,
        4.2,
    );

    // 3. Incandescent orange fireball blobs + electric star flashes.
    let fireballs = ps.budget_count(18, true);
    ps.burst_shaped(
        rng,
        base,
        fireballs,
        78.0,
        6.0,
        Ramp::Fire,
        0.62,
        true,
        Shape::Blob,
        3.2,
    );
    let stars = ps.budget_count(12, true);
    ps.burst_shaped(
        rng,
        base,
        stars,
        96.0,
        6.0,
        Ramp::Spark,
        0.56,
        true,
        Shape::Cross,
        3.4,
    );

    // 4. Heavy billowing soot explosion smoke ring framing the detonation.
    let soot = ps.budget_count(18, true);
    for k in 0..soot {
        let a = k as f32 / soot.max(1) as f32 * std::f32::consts::TAU
            + rng.random_range(-0.16..0.16);
        let out = Vec2::from_angle(a);
        let life = rng.random_range(0.70..1.15);
        ps.push(Particle {
            pos: base + out * rng.random_range(10.0..18.0),
            vel: out * rng.random_range(34.0..68.0) + Vec2::new(0.0, -14.0),
            life,
            max_life: life,
            drag: 2.6,
            gravity: -4.0,
            ramp: Ramp::Custom((140, 128, 122), (48, 44, 48)),
            additive: false,
            shape: Shape::Puff,
        });
    }

    // 5. Lingering hot motor-coil embers drifting above the crater.
    let embers = ps.budget_count(24, true);
    ps.burst_shaped(
        rng,
        base,
        embers,
        84.0,
        8.0,
        Ramp::Gold,
        1.15,
        true,
        Shape::Ember,
        2.0,
    );

    // 6. Layered blackened scorch crater + outer floor fracture web.
    ps.push_stain(Stain {
        pos: base,
        radius: 14.0,
        color: (42, 40, 44),
        life: 16.0,
        max_life: 16.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Crack,
    });
    ps.push_stain(Stain {
        pos: base,
        radius: 10.5,
        color: (14, 12, 12),
        life: 16.0,
        max_life: 16.0,
        seed: rng.random::<u16>(),
        kind: StainKind::Scorch,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::particle::{SHOCKWAVE_CAP, STAIN_CAP};
    use crate::render::framebuffer::PixelBuffer;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Invoke every Wave 1 VFX helper with a given `Particles` buffer and parameters.
    fn call_all_new_emitters(ps: &mut Particles, rng: &mut StdRng, pos: Vec2, dir: Vec2, size: f32, dt: f32) {
        let accent: Rgb = (120, 232, 180);
        let blood: Rgb = (180, 48, 42);
        emit_glue_stick(ps, rng, pos, pos + dir * 3.0, size);
        emit_glue_tear_free(ps, rng, pos, pos + dir * 4.0, dir, size);
        emit_mousetrap_snap(ps, rng, pos, dir);
        emit_one_way_gate_slam(ps, rng, pos, dir, accent);
        emit_rat_hole_squeeze(ps, rng, pos, dir, dir);
        emit_maze_cache_open(ps, rng, pos, accent);

        emit_boss_cat_pounce_land(ps, rng, pos, size, false);
        emit_boss_cat_pounce_land(ps, rng, pos, size, true);
        emit_boss_cat_swipe(ps, rng, pos, dir, 1.0, false);
        emit_boss_cat_swipe(ps, rng, pos, dir, -1.0, true);
        emit_boss_cat_roar(ps, rng, pos, false);
        emit_boss_cat_roar(ps, rng, pos, true);
        emit_boss_cat_skid(ps, rng, pos, dir, dt);
        emit_boss_cat_death(ps, rng, pos, dir, blood);

        emit_boss_broom_stomp(ps, rng, pos, size, false);
        emit_boss_broom_stomp(ps, rng, pos, size, true);
        emit_boss_broom_sweep(ps, rng, pos, 0.5, true, dt);
        emit_boss_broom_sweep(ps, rng, pos, -0.8, false, dt);
        emit_boss_broom_dustpan_drop(ps, rng, pos, size);
        emit_boss_broom_hurt_hop(ps, rng, pos - Vec2::new(10.0, 0.0), pos + Vec2::new(10.0, 0.0));
        emit_boss_broom_defeat(ps, rng, pos, pos + dir * 16.0);

        emit_boss_mower_charge_trail(ps, rng, pos, dir, false, dt);
        emit_boss_mower_charge_trail(ps, rng, pos, dir, true, dt);
        emit_boss_mower_wall_impact(ps, rng, pos, dir, false);
        emit_boss_mower_wall_impact(ps, rng, pos, dir, true);
        emit_boss_mower_flipped_smoke(ps, rng, pos, dt);
        emit_boss_mower_overheat_steam(ps, rng, pos, dir, dt);
        emit_boss_mower_blade_trail(ps, rng, pos, dir * 72.0, dt);
        emit_boss_mower_death(ps, rng, pos);
    }

    #[test]
    fn wave1_vfx_helpers_respect_caps_and_produce_output() {
        let mut rng = StdRng::seed_from_u64(777);

        // 1. Tight cap must never be exceeded even when firing every emitter repeatedly.
        let cap = 40;
        let mut ps = Particles::new(cap);
        for _ in 0..4 {
            call_all_new_emitters(&mut ps, &mut rng, Vec2::new(50.0, 50.0), Vec2::new(1.0, 0.0), 18.0, 0.05);
            assert!(ps.len() <= cap, "particle cap {} exceeded: {}", cap, ps.len());
            assert!(ps.shockwaves.len() <= SHOCKWAVE_CAP);
            assert!(ps.stains.len() <= STAIN_CAP);
        }
        assert!(!ps.is_empty());
        assert!(!ps.shockwaves.is_empty());
        assert!(!ps.stains.is_empty());

        // 2. Render and update must succeed cleanly without panics.
        let mut pb = PixelBuffer::new(120, 120);
        pb.clear((18, 22, 20));
        ps.draw_stains(&mut pb, Vec2::ZERO);
        ps.update(0.05);
        ps.draw(&mut pb, Vec2::ZERO);
    }

    #[test]
    fn wave1_vfx_helpers_never_panic_on_degenerate_inputs_or_zero_cap() {
        let mut rng = StdRng::seed_from_u64(999);

        // Zero-cap Particles::default() drops everything cleanly.
        let mut zero_ps = Particles::default();
        call_all_new_emitters(&mut zero_ps, &mut rng, Vec2::ZERO, Vec2::ZERO, 0.0, 0.016);
        assert_eq!(zero_ps.len(), 0);
        assert!(zero_ps.shockwaves.is_empty());
        assert!(zero_ps.stains.is_empty());

        // Degenerate zero, negative, huge, and NaN arguments must never panic.
        let mut ps = Particles::new(256);
        let degenerate_cases = [
            (Vec2::ZERO, Vec2::ZERO, 0.0_f32, 0.0_f32),
            (Vec2::new(10.0, 10.0), Vec2::ZERO, -50.0, -1.0),
            (Vec2::new(f32::NAN, f32::INFINITY), Vec2::new(f32::NAN, 0.0), f32::NAN, f32::NAN),
            (Vec2::new(20.0, 20.0), Vec2::new(0.0, -1.0), 5000.0, 100.0),
        ];
        for (pos, dir, size, dt) in degenerate_cases {
            call_all_new_emitters(&mut ps, &mut rng, pos, dir, size, dt);
            assert!(ps.len() <= 256);
            assert!(ps.shockwaves.len() <= SHOCKWAVE_CAP);
            assert!(ps.stains.len() <= STAIN_CAP);
        }

        // Sweep with NaN angle must also be a clean no-op.
        emit_boss_broom_sweep(&mut ps, &mut rng, Vec2::ZERO, f32::NAN, true, 0.016);
        emit_boss_cat_swipe(&mut ps, &mut rng, Vec2::ZERO, Vec2::ZERO, f32::NAN, false);

        // Drawing after degenerate inputs onto both normal and 0x0 buffers must never panic.
        let mut pb = PixelBuffer::new(64, 64);
        ps.draw_stains(&mut pb, Vec2::ZERO);
        ps.update(0.05);
        ps.draw(&mut pb, Vec2::ZERO);

        let mut tiny = PixelBuffer::new(0, 0);
        ps.draw_stains(&mut tiny, Vec2::ZERO);
        ps.draw(&mut tiny, Vec2::ZERO);
    }

    #[test]
    fn each_wave1_helper_individually_emits_and_respects_small_cap() {
        let accent: Rgb = (110, 235, 195);
        let blood: Rgb = (176, 46, 44);
        let c = Vec2::new(40.0, 40.0);
        let right = Vec2::new(1.0, 0.0);

        type EmitBox = Box<dyn Fn(&mut Particles, &mut StdRng)>;
        let cases: Vec<(&str, EmitBox)> = vec![
            ("glue_stick", Box::new(move |ps, rng| emit_glue_stick(ps, rng, c, c + Vec2::new(3.0, 1.0), 3.0))),
            ("glue_tear_free", Box::new(move |ps, rng| emit_glue_tear_free(ps, rng, c, c + Vec2::new(6.0, 0.0), right, 3.0))),
            ("mousetrap_snap", Box::new(move |ps, rng| emit_mousetrap_snap(ps, rng, c, right))),
            ("one_way_gate_slam", Box::new(move |ps, rng| emit_one_way_gate_slam(ps, rng, c, right, accent))),
            ("rat_hole_squeeze", Box::new(move |ps, rng| emit_rat_hole_squeeze(ps, rng, c, right, right))),
            ("maze_cache_open", Box::new(move |ps, rng| emit_maze_cache_open(ps, rng, c, accent))),
            ("cat_pounce_land_p1", Box::new(move |ps, rng| emit_boss_cat_pounce_land(ps, rng, c, 22.0, false))),
            ("cat_pounce_land_p2", Box::new(move |ps, rng| emit_boss_cat_pounce_land(ps, rng, c, 22.0, true))),
            ("cat_swipe", Box::new(move |ps, rng| emit_boss_cat_swipe(ps, rng, c, right, 1.0, false))),
            ("cat_roar", Box::new(move |ps, rng| emit_boss_cat_roar(ps, rng, c, true))),
            ("cat_skid", Box::new(move |ps, rng| emit_boss_cat_skid(ps, rng, c, right, 0.12))),
            ("cat_death", Box::new(move |ps, rng| emit_boss_cat_death(ps, rng, c, right, blood))),
            ("broom_stomp", Box::new(move |ps, rng| emit_boss_broom_stomp(ps, rng, c, 16.0, false))),
            ("broom_sweep", Box::new(move |ps, rng| emit_boss_broom_sweep(ps, rng, c, 0.3, true, 0.05))),
            ("broom_dustpan_drop", Box::new(move |ps, rng| emit_boss_broom_dustpan_drop(ps, rng, c, 15.0))),
            ("broom_hurt_hop", Box::new(move |ps, rng| emit_boss_broom_hurt_hop(ps, rng, c - Vec2::new(10.0, 0.0), c + Vec2::new(10.0, 0.0)))),
            ("broom_defeat", Box::new(move |ps, rng| emit_boss_broom_defeat(ps, rng, c, c + Vec2::new(14.0, 4.0)))),
            ("mower_charge_grass", Box::new(move |ps, rng| emit_boss_mower_charge_trail(ps, rng, c, right, false, 0.05))),
            ("mower_charge_sparks", Box::new(move |ps, rng| emit_boss_mower_charge_trail(ps, rng, c, right, true, 0.05))),
            ("mower_wall_impact", Box::new(move |ps, rng| emit_boss_mower_wall_impact(ps, rng, c, right, true))),
            ("mower_flipped_smoke", Box::new(move |ps, rng| emit_boss_mower_flipped_smoke(ps, rng, c, 0.12))),
            ("mower_overheat_steam", Box::new(move |ps, rng| emit_boss_mower_overheat_steam(ps, rng, c, right, 0.12))),
            ("mower_blade_trail", Box::new(move |ps, rng| emit_boss_mower_blade_trail(ps, rng, c, right * 72.0, 0.12))),
            ("mower_death", Box::new(move |ps, rng| emit_boss_mower_death(ps, rng, c))),
        ];

        for (name, emit) in &cases {
            let mut rng = StdRng::seed_from_u64(42);
            let mut ps = Particles::new(12);
            emit(&mut ps, &mut rng);
            assert!(
                !ps.is_empty() || !ps.shockwaves.is_empty(),
                "{name} emitted nothing"
            );
            assert!(
                ps.len() <= 12,
                "{name} exceeded cap 12: {}",
                ps.len()
            );
        }
    }

    fn write_ppm(path: &str, pb: &PixelBuffer) {
        let mut out = Vec::with_capacity(pb.w * pb.h * 3 + 32);
        out.extend_from_slice(format!("P6\n{} {}\n255\n", pb.w, pb.h).as_bytes());
        for y in 0..pb.h {
            for x in 0..pb.w {
                let (r, g, b) = pb.pixel_at(x, y);
                out.push(r);
                out.push(g);
                out.push(b);
            }
        }
        std::fs::write(path, out).expect("write vfx ppm");
    }

    fn fill_checker_floor(pb: &mut PixelBuffer, x0: usize, y0: usize, w: usize, h: usize, a: Rgb, b: Rgb) {
        for y in y0..(y0 + h).min(pb.h) {
            for x in x0..(x0 + w).min(pb.w) {
                let tx = (x - x0) / 16;
                let ty = (y - y0) / 16;
                let seam = (x - x0).is_multiple_of(16) || (y - y0).is_multiple_of(16);
                let base = if (tx + ty).is_multiple_of(2) { a } else { b };
                let col = if seam { palette::scale(base, 0.78) } else { base };
                pb.plot(x as i32, y as i32, col);
            }
        }
        // Subtle 1px cell border.
        let border = (44, 50, 58);
        for x in x0..(x0 + w).min(pb.w) {
            pb.plot(x as i32, y0 as i32, border);
        }
        for y in y0..(y0 + h).min(pb.h) {
            pb.plot(x0 as i32, y as i32, border);
        }
    }

    fn blit_cell(dst: &mut PixelBuffer, src: &PixelBuffer, dx: usize, dy: usize) {
        for y in 0..src.h {
            for x in 0..src.w {
                dst.plot((dx + x) as i32, (dy + y) as i32, src.pixel_at(x, y));
            }
        }
    }

    /// RATRUN_DUMP-gated visual showcase sheet for all 22 Wave 1 VFX emitters
    /// rendered in context over themed floors + trap/boss art!
    ///
    /// Usage: `RATRUN_DUMP=1 cargo test dump_vfx_wave1_sheet`
    /// Writes `/tmp/ratrun_vfx_traps_cat.ppm` and `/tmp/ratrun_vfx_broom_mower.ppm`.
    #[test]
    fn dump_vfx_wave1_sheet() {
        let Ok(val) = std::env::var("RATRUN_DUMP") else {
            return;
        };
        if val.is_empty() {
            return;
        }

        use crate::game::art_boss_broom::{self, BroomBossState, BroomBossView};
        use crate::game::art_boss_cat::{self, BossCatState, BossCatView};
        use crate::game::art_boss_mower::{self, MowerState, MowerView};
        use crate::game::art_traps::{
            self, MazeRewardState, MazeRewardView, MousetrapState, MousetrapView,
            OneWayDoorState, OneWayDoorView, RatHoleState, RatHoleView, StickyTrapState,
            StickyTrapView,
        };

        let cw = 120usize;
        let ch = 96usize;
        let cols = 4usize;
        let rows = 3usize;
        let center = Vec2::new(cw as f32 * 0.5, ch as f32 * 0.52);

        let sewer_a: Rgb = (20, 26, 28);
        let sewer_b: Rgb = (24, 31, 33);
        let kitchen_a: Rgb = (34, 30, 28);
        let kitchen_b: Rgb = (42, 37, 34);
        let lab_a: Rgb = (24, 30, 38);
        let lab_b: Rgb = (30, 38, 46);
        let sewer_accent: Rgb = (96, 224, 152);
        let kitchen_accent: Rgb = (246, 178, 86);
        let lab_accent: Rgb = (96, 224, 255);

        // ---- Sheet 1: Traps (0..5), Level-Up Ref (6), Jaguar Cat Boss (7..11) ----
        let mut sheet1 = PixelBuffer::new(cw * cols, ch * rows);
        let mut rng = StdRng::seed_from_u64(2026);

        let mut render_cell = |idx: usize,
                           sheet: &mut PixelBuffer,
                           fa: Rgb,
                           fb: Rgb,
                           draw_fn: &mut dyn FnMut(&mut PixelBuffer, &mut Particles, &mut StdRng)| {
            let mut cell = PixelBuffer::new(cw, ch);
            fill_checker_floor(&mut cell, 0, 0, cw, ch, fa, fb);
            let mut ps = Particles::new(600);
            draw_fn(&mut cell, &mut ps, &mut rng);
            let col = idx % cols;
            let row = idx / cols;
            blit_cell(sheet, &cell, col * cw, row * ch);
        };

        // [0] Glue Stick
        render_cell(0, &mut sheet1, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let mut tv = StickyTrapView::new(center, Vec2::new(1.0, 0.0));
            tv.state = StickyTrapState::Stuck;
            tv.progress = 0.25;
            tv.victim_pos = center + Vec2::new(4.0, -1.0);
            emit_glue_stick(ps, rng, tv.pos, tv.victim_pos, 3.0);
            ps.update(0.06);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_sticky_trap_ground(pb, &tv, Vec2::ZERO, kitchen_accent);
            art_traps::draw_sticky_trap(pb, &tv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [1] Glue Tear Free
        render_cell(1, &mut sheet1, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let mut tv = StickyTrapView::new(center - Vec2::new(6.0, 0.0), Vec2::new(1.0, 0.0));
            tv.state = StickyTrapState::Used;
            let vpos = center + Vec2::new(10.0, -1.0);
            emit_glue_tear_free(ps, rng, tv.pos, vpos, Vec2::new(1.0, 0.0), 3.0);
            ps.update(0.05);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_sticky_trap_ground(pb, &tv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [2] Mousetrap Snap
        render_cell(2, &mut sheet1, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let mut mv = MousetrapView::new(center - Vec2::new(4.0, 0.0), Vec2::new(1.0, 0.0));
            mv.state = MousetrapState::Snapping;
            mv.progress = 0.50;
            emit_mousetrap_snap(ps, rng, mv.pos, mv.facing);
            ps.update(0.05);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_mousetrap_ground(pb, &mv, Vec2::ZERO, kitchen_accent);
            art_traps::draw_mousetrap(pb, &mv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [3] One-Way Gate Slam
        render_cell(3, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut dv = OneWayDoorView::new(center, Vec2::new(1.0, 0.0));
            dv.state = OneWayDoorState::Closed;
            emit_one_way_gate_slam(ps, rng, dv.pos, dv.facing, sewer_accent);
            ps.update(0.06);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_one_way_door_ground(pb, &dv, Vec2::ZERO, sewer_accent);
            art_traps::draw_one_way_door(pb, &dv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [4] Rat-Hole Squeeze
        render_cell(4, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut rv = RatHoleView::new(center, Vec2::new(1.0, 0.0));
            rv.state = RatHoleState::Squeezing;
            rv.progress = 0.45;
            emit_rat_hole_squeeze(ps, rng, rv.pos, rv.facing, Vec2::new(1.0, 0.0));
            ps.update(0.06);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_rat_hole_ground(pb, &rv, Vec2::ZERO, sewer_accent);
            art_traps::draw_rat_hole(pb, &rv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [5] Maze Cache Open (distinct antique gold + accent coin fountain!)
        render_cell(5, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut cv = MazeRewardView::new(center, Vec2::new(0.0, 1.0));
            cv.state = MazeRewardState::Opening;
            cv.progress = 0.30;
            emit_maze_cache_open(ps, rng, cv.pos, sewer_accent);
            ps.update(0.10);
            ps.draw_stains(pb, Vec2::ZERO);
            art_traps::draw_maze_reward_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_traps::draw_maze_reward(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [6] Reference Level-Up (gold + tail-pink rings & floating stars — side-by-side comparison)
        render_cell(6, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            emit_level_up(ps, rng, center);
            ps.update(0.10);
            ps.draw_stains(pb, Vec2::ZERO);
            ps.draw(pb, Vec2::ZERO);
        });

        // [7] Jaguar Pounce Land (Phase 2 dust ring + cracked floor stain + shockwave)
        render_cell(7, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut cv = BossCatView::new(center, Vec2::new(1.0, 0.0), center);
            cv.state = BossCatState::Pounce;
            cv.progress = 0.88;
            cv.hp_frac = 0.40;
            emit_boss_cat_pounce_land(ps, rng, center, art_boss_cat::POUNCE_LAND_RADIUS, true);
            ps.update(0.08);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_cat::draw_boss_cat_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_boss_cat::draw_boss_cat(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [8] Jaguar Claw Swipe Arcs
        render_cell(8, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut cv = BossCatView::new(center - Vec2::new(12.0, 0.0), Vec2::new(1.0, 0.0), center + Vec2::new(30.0, 0.0));
            cv.state = BossCatState::Swipe;
            cv.progress = 0.25;
            let origin = art_boss_cat::swipe_origin(&cv);
            emit_boss_cat_swipe(ps, rng, origin, cv.facing, 1.0, false);
            ps.update(0.05);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_cat::draw_boss_cat_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_boss_cat::draw_boss_cat(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [9] Jaguar Roar Rings
        render_cell(9, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut cv = BossCatView::new(center - Vec2::new(16.0, 0.0), Vec2::new(1.0, 0.0), center + Vec2::new(30.0, 0.0));
            cv.state = BossCatState::Roar;
            cv.progress = 0.50;
            cv.hp_frac = 0.42;
            let origin = art_boss_cat::roar_origin(&cv);
            emit_boss_cat_roar(ps, rng, origin, true);
            ps.update(0.10);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_cat::draw_boss_cat_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_boss_cat::draw_boss_cat(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [10] Jaguar Overshoot Skid (multi-frame sliding brake trail!)
        render_cell(10, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            for step in 0..8 {
                let p = center + Vec2::new(-22.0 + step as f32 * 4.5, 0.0);
                emit_boss_cat_skid(ps, rng, p, Vec2::new(1.0, 0.0), 0.04);
                ps.update(0.04);
            }
            let cv = BossCatView::new(center + Vec2::new(14.0, 0.0), Vec2::new(1.0, 0.0), center + Vec2::new(40.0, 0.0));
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_cat::draw_boss_cat_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_boss_cat::draw_boss_cat(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [11] Jaguar Death
        render_cell(11, &mut sheet1, sewer_a, sewer_b, &mut |pb, ps, rng| {
            let mut cv = BossCatView::new(center, Vec2::new(1.0, 0.0), center);
            cv.state = BossCatState::Death;
            cv.progress = 0.45;
            emit_boss_cat_death(ps, rng, center, Vec2::new(1.0, 0.0), (168, 44, 48));
            ps.update(0.10);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_cat::draw_boss_cat_ground(pb, &cv, Vec2::ZERO, sewer_accent);
            art_boss_cat::draw_boss_cat(pb, &cv, Vec2::ZERO, sewer_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        write_ppm("/tmp/ratrun_vfx_traps_cat.ppm", &sheet1);

        // ---- Sheet 2: Broom Boss (0..4) + Mower Boss (5..11) ----
        let mut sheet2 = PixelBuffer::new(cw * cols, ch * rows);

        // [0] Broom Stomp (r = 16.0)
        render_cell(0, &mut sheet2, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let bv = BroomBossView {
                pos: center,
                facing: Vec2::new(0.0, 1.0),
                state: BroomBossState::StompWalk { left_foot: true },
                progress: 0.72,
                anim: 1.0,
                hit_flash: 0.0,
                hp_frac: 0.75,
                target: center + Vec2::new(-10.5, 4.0),
                enraged: false,
            };
            let feet = art_boss_broom::foot_hitboxes(&bv);
            emit_boss_broom_stomp(ps, rng, feet[0].pos, art_boss_broom::STOMP_SHOCKWAVE_RADIUS, false);
            ps.update(0.07);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_broom::draw_ground(pb, &bv, Vec2::ZERO, kitchen_accent);
            art_boss_broom::draw(pb, &bv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [1] Broom Sweep (travelling dust + crumb wave across arc!)
        render_cell(1, &mut sheet2, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let mut bv = BroomBossView {
                pos: center - Vec2::new(0.0, 8.0),
                facing: Vec2::new(0.0, 1.0),
                state: BroomBossState::BroomSweep { clockwise: true },
                progress: 0.48,
                anim: 1.0,
                hit_flash: 0.0,
                hp_frac: 0.75,
                target: center,
                enraged: false,
            };
            for step in 0..7 {
                bv.progress = 0.48 + step as f32 * 0.032;
                let ang = art_boss_broom::broom_sweep_angle(&bv);
                emit_boss_broom_sweep(ps, rng, bv.pos, ang, true, 0.035);
                ps.update(0.035);
            }
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_broom::draw_ground(pb, &bv, Vec2::ZERO, kitchen_accent);
            art_boss_broom::draw(pb, &bv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [2] Broom Dustpan Drop
        render_cell(2, &mut sheet2, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let bv = BroomBossView {
                pos: center - Vec2::new(26.0, 0.0),
                facing: Vec2::new(1.0, 0.0),
                state: BroomBossState::DustpanDrop,
                progress: 0.54,
                anim: 1.0,
                hit_flash: 0.0,
                hp_frac: 0.65,
                target: center + Vec2::new(12.0, 0.0),
                enraged: false,
            };
            emit_boss_broom_dustpan_drop(ps, rng, bv.target, art_boss_broom::DUSTPAN_IMPACT_RADIUS);
            ps.update(0.07);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_broom::draw_ground(pb, &bv, Vec2::ZERO, kitchen_accent);
            art_boss_broom::draw(pb, &bv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [3] Broom Hurt Hop
        render_cell(3, &mut sheet2, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let bv = BroomBossView {
                pos: center,
                facing: Vec2::new(0.0, 1.0),
                state: BroomBossState::Hurt { left_foot_hit: true },
                progress: 0.35,
                anim: 1.4,
                hit_flash: 0.4,
                hp_frac: 0.60,
                target: center,
                enraged: false,
            };
            let feet = art_boss_broom::foot_hitboxes(&bv);
            emit_boss_broom_hurt_hop(ps, rng, feet[1].pos, feet[0].pos);
            ps.update(0.06);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_broom::draw_ground(pb, &bv, Vec2::ZERO, kitchen_accent);
            art_boss_broom::draw(pb, &bv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [4] Broom Defeat
        render_cell(4, &mut sheet2, kitchen_a, kitchen_b, &mut |pb, ps, rng| {
            let bv = BroomBossView {
                pos: center - Vec2::new(4.0, 0.0),
                facing: Vec2::new(0.0, 1.0),
                state: BroomBossState::Defeat,
                progress: 0.24,
                anim: 2.0,
                hit_flash: 0.0,
                hp_frac: 0.0,
                target: center,
                enraged: false,
            };
            let bpos = art_boss_broom::broom_head_pos(&bv);
            emit_boss_broom_defeat(ps, rng, bv.pos, bpos);
            ps.update(0.10);
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_broom::draw_ground(pb, &bv, Vec2::ZERO, kitchen_accent);
            art_boss_broom::draw(pb, &bv, Vec2::ZERO, kitchen_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [5] Mower Charge Trail (Grass Clippings)
        render_cell(5, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            let start = center - Vec2::new(34.0, 0.0);
            for step in 0..8 {
                let p = start + Vec2::new(step as f32 * 6.0, 0.0);
                emit_boss_mower_charge_trail(ps, rng, p, Vec2::new(1.0, 0.0), false, 0.035);
                ps.update(0.035);
            }
            let mut mv = MowerView::patrol(start + Vec2::new(48.0, 0.0), Vec2::new(1.0, 0.0), 1.0);
            mv.state = MowerState::Charge;
            mv.trail_start = start;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [6] Mower Charge Trail (Scorched Sparks)
        render_cell(6, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            let start = center - Vec2::new(34.0, 0.0);
            for step in 0..8 {
                let p = start + Vec2::new(step as f32 * 6.0, 0.0);
                emit_boss_mower_charge_trail(ps, rng, p, Vec2::new(1.0, 0.0), true, 0.035);
                ps.update(0.035);
            }
            let mut mv = MowerView::patrol(start + Vec2::new(48.0, 0.0), Vec2::new(1.0, 0.0), 1.0);
            mv.state = MowerState::Ricochet;
            mv.bounce_count = 2;
            mv.hp_frac = 0.45;
            mv.trail_start = start;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [7] Mower Wall Impact (Sparks + Dent Stain)
        render_cell(7, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            let wall_x = center.x + 24.0;
            // Draw a simple vertical wall bar on the right to frame the impact.
            for y in 10..(ch - 10) {
                for x in (wall_x as usize)..(wall_x as usize + 10).min(cw) {
                    pb.plot(x as i32, y as i32, (62, 72, 86));
                }
            }
            let imp = Vec2::new(wall_x - 2.0, center.y);
            emit_boss_mower_wall_impact(ps, rng, imp, Vec2::new(-1.0, 0.0), true);
            ps.update(0.08);
            let mut mv = MowerView::patrol(imp - Vec2::new(19.0, 0.0), Vec2::new(1.0, 0.0), 1.0);
            mv.state = MowerState::Flipped;
            mv.progress = 0.16;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [8] Mower Flipped Smoke
        render_cell(8, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            for _ in 0..14 {
                emit_boss_mower_flipped_smoke(ps, rng, center + Vec2::new(0.0, 10.0), 0.05);
                ps.update(0.05);
            }
            let mut mv = MowerView::patrol(center + Vec2::new(0.0, 10.0), Vec2::new(1.0, 0.0), 2.0);
            mv.state = MowerState::Flipped;
            mv.progress = 0.45;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [9] Mower Overheat Steam
        render_cell(9, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            let mpos = center + Vec2::new(6.0, 10.0);
            for _ in 0..14 {
                emit_boss_mower_overheat_steam(ps, rng, mpos, Vec2::new(1.0, 0.0), 0.05);
                ps.update(0.05);
            }
            let mut mv = MowerView::patrol(mpos, Vec2::new(1.0, 0.0), 2.0);
            mv.state = MowerState::Overheat;
            mv.progress = 0.45;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        // [10] Mower Blade-Disc Trails (3 flying discs!)
        render_cell(10, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            let lanes = [
                (center + Vec2::new(-32.0, -18.0), Vec2::new(72.0, 14.0)),
                (center + Vec2::new(-34.0, 2.0), Vec2::new(78.0, -4.0)),
                (center + Vec2::new(-30.0, 20.0), Vec2::new(68.0, -16.0)),
            ];
            let mut finals = [Vec2::ZERO; 3];
            for step in 0..12 {
                for (i, &(p0, vel)) in lanes.iter().enumerate() {
                    let p = p0 + vel * (step as f32 * 0.055);
                    finals[i] = p;
                    emit_boss_mower_blade_trail(ps, rng, p, vel, 0.055);
                }
                ps.update(0.055);
            }
            ps.draw_stains(pb, Vec2::ZERO);
            ps.draw(pb, Vec2::ZERO);
            for &p in &finals {
                art_boss_mower::draw_blade_disc(pb, p, 1.4, Vec2::ZERO, lab_accent);
            }
        });

        // [11] Mower Death Explosion
        render_cell(11, &mut sheet2, lab_a, lab_b, &mut |pb, ps, rng| {
            emit_boss_mower_death(ps, rng, center);
            ps.update(0.10);
            let mut mv = MowerView::patrol(center, Vec2::new(1.0, 0.0), 1.5);
            mv.state = MowerState::Dying;
            mv.progress = 0.56;
            mv.hp_frac = 0.0;
            ps.draw_stains(pb, Vec2::ZERO);
            art_boss_mower::draw_mower_ground(pb, &mv, Vec2::ZERO, lab_accent);
            art_boss_mower::draw_mower(pb, &mv, Vec2::ZERO, lab_accent);
            ps.draw(pb, Vec2::ZERO);
        });

        write_ppm("/tmp/ratrun_vfx_broom_mower.ppm", &sheet2);

        // ---- Sheet 3: Unobstructed Floor Stains Strip (6 cells: Glue, Trap Dent, Cat Crack, Broom Crack, Mower Dent, Mower Crater) ----
        let scw = 72usize;
        let sch = 72usize;
        let sc = Vec2::new(scw as f32 * 0.5, sch as f32 * 0.5);
        let mut sheet3 = PixelBuffer::new(scw * 6, sch);
        type StainCase = (Rgb, Rgb, Box<dyn Fn(&mut Particles, &mut StdRng)>);
        let stain_cases: [StainCase; 6] = [
            (kitchen_a, kitchen_b, Box::new(move |ps, rng| emit_glue_stick(ps, rng, sc, sc, 4.0))),
            (kitchen_a, kitchen_b, Box::new(move |ps, rng| emit_mousetrap_snap(ps, rng, sc - Vec2::new(5.5, 0.0), Vec2::new(1.0, 0.0)))),
            (sewer_a, sewer_b, Box::new(move |ps, rng| emit_boss_cat_pounce_land(ps, rng, sc, art_boss_cat::POUNCE_LAND_RADIUS, false))),
            (kitchen_a, kitchen_b, Box::new(move |ps, rng| emit_boss_broom_stomp(ps, rng, sc, art_boss_broom::PHASE_SLAM_RADIUS, false))),
            (lab_a, lab_b, Box::new(move |ps, rng| emit_boss_mower_wall_impact(ps, rng, sc, Vec2::new(-1.0, 0.0), true))),
            (lab_a, lab_b, Box::new(move |ps, rng| emit_boss_mower_death(ps, rng, sc))),
        ];
        for (idx, (fa, fb, emit)) in stain_cases.iter().enumerate() {
            let mut cell = PixelBuffer::new(scw, sch);
            fill_checker_floor(&mut cell, 0, 0, scw, sch, *fa, *fb);
            let mut ps = Particles::new(200);
            emit(&mut ps, &mut rng);
            ps.draw_stains(&mut cell, Vec2::ZERO);
            blit_cell(&mut sheet3, &cell, idx * scw, 0);
        }
        write_ppm("/tmp/ratrun_vfx_stains.ppm", &sheet3);
    }
}
