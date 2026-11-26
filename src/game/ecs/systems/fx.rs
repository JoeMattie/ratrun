//! (g, second file) Particles update and screen shake.
//!
//! Faithful port of:
//! - `self.particles.update(dt)` at the end of the tick, and the `finished()`
//!   early-return path, where **only** particles advance (this system is the
//!   whole `settle_schedule`);
//! - screen-shake decay + jitter offset.
//!
//! Neon merge: the other team also advance `World::damage_numbers` right after
//! `self.particles.update(dt)` (in the normal tick *and* in the `finished()`
//! freeze path). That one-line delegation lives in
//! [`crate::game::ecs::schedule`] as `damage_numbers_system`, chained directly
//! after `particles_system` in **both** schedules, so the tick position is
//! exactly the legacy one; see the `settle_tick_...` test below.
//!
//! SFX has no system by design: every system pushes into [`SfxQueue`] at the
//! same points the legacy code pushed into `self.sfx`; `World::end_tick` moves
//! the queue back into the facade where the App drains it.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;
use rand::RngExt;

use crate::game::ecs::resources::{RngRes, Shake, TickInput};
use crate::game::particle::Particles;
use crate::math::Vec2;

/// Legacy `particles.update(dt)`.
pub fn particles_system(input: Res<TickInput>, mut particles: ResMut<Particles>) {
    particles.update(input.dt);
}

/// Legacy shake decay and jitter, with a damped rebound for impact pulses.
///
/// Determinism: the two `random_range(-1.0..1.0)` draws happen **only** when
/// `shake > 0.05`, in the legacy x-then-y order, and the decay happens after
/// them (both coordinates use the pre-decay amplitude). Drawing speculatively
/// would diverge the global rng stream.
pub fn shake_system(
    input: Res<TickInput>,
    mut shake: ResMut<Shake>,
    mut rng: ResMut<RngRes>,
    mut rebound: Local<Vec2>,
) {
    let dt = input.dt;
    if shake.amount > 0.05 {
        let amount = shake.amount;
        shake.ofs = Vec2::new(
            rng.0.random_range(-1.0..1.0) * amount,
            rng.0.random_range(-1.0..1.0) * amount,
        );
        shake.amount = (shake.amount - dt * 30.0).max(0.0);
    } else {
        shake.ofs = Vec2::ZERO;
    }
    let impact = shake.impact;
    *rebound = if impact != Vec2::ZERO {
        impact
    } else {
        -*rebound * (1.0 - dt * 26.0).max(0.0)
    };
    if rebound.len_sq() > 0.01 {
        shake.ofs += *rebound;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::particle::{Particle, Ramp, Shape};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn particle(life: f32) -> Particle {
        Particle {
            pos: Vec2::ZERO,
            vel: Vec2::new(10.0, 0.0),
            life,
            max_life: life,
            drag: 0.0,
            gravity: 0.0,
            ramp: Ramp::Fire,
            additive: false,
            shape: Shape::Dot,
        }
    }

    #[test]
    fn particles_advance_by_dt_and_expire() {
        let mut w = World::new(Theme::Sewer, 11);
        w.particles.push(particle(0.5));
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(particles_system)
            .expect("particles run");
        w.end_tick();
        assert_eq!(w.particles.len(), 1);
        let p = &w.particles.items[0];
        assert!((p.life - (0.5 - DT)).abs() < 1e-6, "life did not decay");
        assert!((p.pos.x - 10.0 * DT).abs() < 1e-4, "particle did not move");

        // A long tick ages the particle out.
        w.begin_tick(1.0, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(particles_system)
            .expect("particles run");
        w.end_tick();
        assert!(w.particles.is_empty(), "expired particle survived");
    }

    #[test]
    fn calm_shake_draws_no_rng_and_zeroes_the_offset() {
        let mut jittered = World::new(Theme::Sewer, 13);
        jittered.shake = 0.0;
        jittered.shake_ofs = Vec2::new(3.0, 4.0);

        jittered.begin_tick(DT, Vec2::ZERO, false);
        jittered
            .ecs
            .run_system_cached(shake_system)
            .expect("shake runs");
        jittered.end_tick();

        assert_eq!(jittered.shake_ofs, Vec2::ZERO);
        assert_eq!(jittered.shake, 0.0);

        // A same-seed world that never ran the system sits at the same rng
        // position, so equal next draws prove no speculative draws happened.
        let mut untouched = World::new(Theme::Sewer, 13);
        untouched.begin_tick(DT, Vec2::ZERO, false);
        untouched.end_tick();
        assert_eq!(
            jittered.rng.random::<u64>(),
            untouched.rng.random::<u64>(),
            "calm shake consumed rng draws"
        );
    }

    #[test]
    fn shake_jitters_within_amplitude_and_decays() {
        let mut w = World::new(Theme::Sewer, 17);
        w.shake = 10.0;
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(shake_system).expect("shake runs");
        w.end_tick();

        // Both coordinates are `random_range(-1..1) * 10`, so each is within
        // +-10 (the legacy x-then-y draw order).
        assert!(w.shake_ofs.x.abs() <= 10.0);
        assert!(w.shake_ofs.y.abs() <= 10.0);
        assert!((w.shake - (10.0 - DT * 30.0)).abs() < 1e-5, "no decay");
    }

    #[test]
    fn impact_shake_rebounds_without_consuming_simulation_rng() {
        let mut w = World::new(Theme::Kitchen, 17);
        let mut quiet = World::new(Theme::Kitchen, 17);
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.resource_mut::<Shake>().impact = Vec2::new(1.6, 2.8);
        w.ecs.run_system_cached(shake_system).expect("shake runs");
        w.end_tick();
        assert_eq!(w.shake_ofs, Vec2::new(1.6, 2.8));

        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(shake_system).expect("shake runs");
        w.end_tick();
        assert!((-2.8..0.0).contains(&w.shake_ofs.y));
        assert_eq!(w.rng.random::<u64>(), quiet.rng.random::<u64>());
    }

    #[test]
    fn shake_clamps_at_zero_on_the_frame_it_runs_out() {
        let calm_after = |run_second_tick: bool| {
            let mut w = World::new(Theme::Sewer, 19);
            // Above the 0.05 threshold: it jitters, then decays past zero.
            w.shake = 0.06;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(shake_system).expect("shake runs");
            w.end_tick();
            assert_eq!(w.shake, 0.0, "shake went negative");
            assert_ne!(w.shake_ofs, Vec2::ZERO, "the jitter still happened");
            if run_second_tick {
                w.begin_tick(DT, Vec2::ZERO, false);
                w.ecs.run_system_cached(shake_system).expect("shake runs");
                w.end_tick();
                assert_eq!(w.shake_ofs, Vec2::ZERO, "calm tick kept an offset");
            }
            w.rng.random::<u64>()
        };
        // The second (calm) tick must not consume rng draws: its next value
        // must match the same-seed world that stopped after the jitter tick.
        assert_eq!(calm_after(true), calm_after(false));
    }

    #[test]
    fn settle_tick_advances_particles_and_damage_numbers() {
        // Legacy `finished()` branch: particles **and** the neon damage
        // numbers keep settling while the sim is frozen. The damage-number
        // update is chained after `particles_system` in the settle schedule
        // (it lives in `schedule.rs`, the file that owns tick order).
        use crate::game::particle::damage_number::DamageSource;

        let mut w = World::new(Theme::Sewer, 29);
        w.won = true; // finished() → settle path
        w.particles.push(particle(0.5));
        w.damage_numbers
            .record(0, Vec2::ZERO, 2.0, 12.0, false, DamageSource::Normal);
        let life_before = w.damage_numbers.items[0].life;

        w.update(0.1, Vec2::ZERO, false);

        assert_eq!(w.particles.len(), 1, "particles must keep settling");
        assert_eq!(w.damage_numbers.len(), 1, "the number must keep floating");
        assert!(
            w.damage_numbers.items[0].life < life_before,
            "the frozen tick did not advance the damage numbers"
        );
        assert!(
            (w.particles.items[0].life - 0.4).abs() < 1e-5,
            "the frozen tick did not advance the particles"
        );
    }

    #[test]
    fn same_seed_gives_the_same_jitter() {
        let jitter = |seed: u64| {
            let mut w = World::new(Theme::Sewer, seed);
            w.shake = 8.0;
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(shake_system).expect("shake runs");
            w.end_tick();
            w.shake_ofs
        };
        assert_eq!(jitter(23), jitter(23));
    }
}
