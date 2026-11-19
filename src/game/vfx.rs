//! High-level combat feedback VFX emitters.
//!
//! Keeps `world.rs` clean and surgical: every weapon hit, enemy death, dash,
//! pickup, acid splash, explosion, and level-up delegates here, and every
//! emitter degrades gracefully via `Particles::budget_count` under heavy load.

use rand::{Rng, RngExt};

use crate::game::enemy::EnemyKind;
use crate::game::gem::PickupKind;
use crate::game::particle::{Particle, Particles, Ramp, Shape, Shockwave, Stain};
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
        scorch: false,
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
                scorch: false,
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
                scorch: false,
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
                scorch: false,
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
                scorch: false,
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
                scorch: false,
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
                scorch: true,
            });
            ps.push_stain(Stain {
                pos: pos + Vec2::new(2.0, -1.5),
                radius: 7.0,
                color: stain_col,
                life: 11.0,
                max_life: 11.0,
                seed: rng.random::<u16>(),
                scorch: false,
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
        scorch: true,
    });
}
