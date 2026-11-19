//! Time-based spawn director. Replaces fixed waves with a difficulty curve:
//! spawn rate, enemy HP, and the enemy mix all escalate with the run clock.

use rand::{Rng, RngExt};

use super::enemy::EnemyKind;

pub const RUN_SECONDS: f32 = 300.0; // survive 5 minutes to win
pub const BOSS_TIME: f32 = RUN_SECONDS - 45.0;
/// Elites (buffed brutes) only start spawning after this many seconds — the
/// world's elite branch gates on the same instant.
pub const ELITE_START: f32 = 60.0;
/// Gap between elites once they start, and the value the timer is reset to.
pub const ELITE_INTERVAL: f32 = 30.0;

#[derive(bevy_ecs::resource::Resource)]
pub struct Director {
    pub spawn_timer: f32,
    pub boss_spawned: bool,
    pub elite_timer: f32,
    /// How many spawn pulses have fired this run. Drives the room-wave cadence
    /// in `systems/spawn.rs` (every [`ROOM_WAVE_EVERY`]-th pulse brings its
    /// first enemy in through a doorway instead of the ring) and is exposed for
    /// the HUD/debug readouts.
    pub pulses: u32,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            spawn_timer: 0.0,
            boss_spawned: false,
            pulses: 0,
            // Count down to the first elite, which is gated on ELITE_START. A
            // shorter seed (it used to be 25 s) just ran far negative and the
            // first brute popped the moment the gate opened.
            elite_timer: ELITE_START,
        }
    }
}

/// How tough enemies are right now (HP multiplier), grows with time.
pub fn hp_mult(elapsed: f32) -> f32 {
    1.0 + elapsed / 45.0
}

/// Seconds between spawn pulses, shrinking over time (floor 0.20s).
///
/// The floor came down from 0.28 s and the pulse grew (see [`pulse_count`]) for
/// the ten-times-larger map: the horde has to fill 3200 × 2560 px instead of
/// 1020 × 800, so the late game needs a noticeably higher spawn rate to keep
/// the screen busy while the player kites across it.
pub fn spawn_interval(elapsed: f32) -> f32 {
    (1.4 - elapsed / 220.0).max(0.20)
}

/// Enemies emitted per pulse, growing over time.
///
/// The starting pulse grew from 2 to 3 (three enemies on a 3 200 px map are
/// barely a blip) and the ramp steepened from `t/16` to `t/12`: with the ring
/// finally wall-clear and reachable, every spawn now actually reaches the rat,
/// so `3 + t/16` was measured to peak at only 35 live enemies in the scripted
/// crowd case; `3 + t/12` clears the "peak ≥ 40 before 60 s" acceptance test
/// while the late game stays clamped at [`crate::game::ecs::constants::ENEMY_CAP`]
/// from about 160 s on.
pub fn pulse_count(elapsed: f32) -> usize {
    (3.0 + elapsed / 12.0) as usize
}

/// Late-run spawn rate in enemies per second (tests, tuning screens).
pub fn spawn_rate(elapsed: f32) -> f32 {
    pulse_count(elapsed) as f32 / spawn_interval(elapsed).max(1e-3)
}

/// Kinds that unlock over time: (unlock time, kind, weight). Kept in table
/// order so the weighted roll keeps its historical result for a given rng
/// state.
const UNLOCKS: [(f32, EnemyKind, f32); 4] = [
    (20.0, EnemyKind::Bat, 4.0),
    (45.0, EnemyKind::Cat, 3.0),
    (75.0, EnemyKind::Spitter, 2.0),
    (120.0, EnemyKind::Brute, 1.5),
];

/// Weighted pick of an enemy kind appropriate to the current time.
pub fn pick_kind(elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    // One slot per possible kind, on the stack: this runs on every spawn roll
    // and must not allocate.
    let mut table = [(EnemyKind::Skitterer, 0.0f32); 1 + UNLOCKS.len()];
    table[0] = (EnemyKind::Skitterer, 6.0);
    let mut n = 1;
    for &(unlock, kind, weight) in UNLOCKS.iter() {
        if elapsed > unlock {
            table[n] = (kind, weight);
            n += 1;
        }
    }
    let live = &table[..n];
    let total: f32 = live.iter().map(|(_, w)| w).sum();
    let mut roll = rng.random_range(0.0..total);
    for (k, w) in live {
        if roll < *w {
            return *k;
        }
        roll -= w;
    }
    EnemyKind::Skitterer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_kind_is_deterministic_and_time_gated() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        // Same seed + clock ⇒ same picks. The roll consumes exactly one rng
        // value and walks the table in the original order (skitterer first,
        // then bat/cat/spitter/brute as they unlock), so the stack rewrite
        // cannot change which kind a given rng state produces.
        let mut a = StdRng::seed_from_u64(42);
        let mut b = StdRng::seed_from_u64(42);
        for elapsed in [0.0f32, 30.0, 60.0, 100.0, 200.0] {
            for _ in 0..50 {
                assert_eq!(
                    pick_kind(elapsed, &mut a),
                    pick_kind(elapsed, &mut b),
                    "pick_kind must be a pure function of rng + clock"
                );
            }
        }

        // Before any unlock only skitterers exist.
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..200 {
            assert_eq!(pick_kind(0.0, &mut rng), EnemyKind::Skitterer);
        }
        // Late in the run every kind is in play.
        let mut seen: Vec<EnemyKind> = Vec::new();
        let mut rng = StdRng::seed_from_u64(9);
        for _ in 0..2000 {
            let k = pick_kind(200.0, &mut rng);
            if !seen.contains(&k) {
                seen.push(k);
            }
        }
        assert!(seen.contains(&EnemyKind::Bat) && seen.contains(&EnemyKind::Brute));
        assert!(seen.contains(&EnemyKind::Skitterer));
    }

    #[test]
    fn difficulty_escalates() {
        assert!(hp_mult(120.0) > hp_mult(0.0));
        assert!(spawn_interval(200.0) < spawn_interval(0.0));
        assert!(pulse_count(200.0) > pulse_count(0.0));
    }

    #[test]
    fn spawn_interval_has_floor() {
        assert!(spawn_interval(100000.0) >= 0.20 - 1e-6);
    }

    /// The retuned curve, as the content contract specifies it: at least three
    /// enemies per pulse from the start, a 0.20 s floor, and a rate that keeps
    /// climbing (so the ten-times-larger map still fills up) up to a rate that
    /// overshoots the 600-enemy cap well before the clock runs out.
    #[test]
    fn the_curve_fills_a_map_ten_times_larger() {
        assert_eq!(pulse_count(0.0), 3);
        assert!((spawn_interval(1_000.0) - 0.20).abs() < 1e-6);
        // Monotone on the way there: no dip in rate, no dip in pulse size.
        let mut prev = 0.0f32;
        for t in [0.0f32, 30.0, 60.0, 120.0, 180.0, 240.0, 255.0, 300.0] {
            let rate = spawn_rate(t);
            assert!(rate >= prev - 1e-4, "spawn rate dipped at {t}s");
            prev = rate;
        }
        // 300 s: ~21 enemies every 0.2 s, far past the cap (it is clamped by
        // `ENEMY_CAP`, this only proves the curve wants to overflow it).
        assert!(spawn_rate(300.0) > 60.0, "late rate {}", spawn_rate(300.0));
        // And the early game is not a wall of bugs: the first seconds stay slow.
        assert!(spawn_rate(0.0) < 3.0, "early rate {}", spawn_rate(0.0));
    }

    #[test]
    fn elite_timer_counts_down_to_the_unlock_time() {
        // The elite branch fires when `elite_timer <= 0.0 && elapsed >
        // ELITE_START`; seeding the timer with ELITE_START keeps the first
        // brute at the unlock instant instead of leaving the timer tens of
        // seconds negative before the gate opens.
        let mut d = Director::default();
        assert_eq!(d.elite_timer, ELITE_START);

        let dt = 1.0 / 60.0;
        let mut elapsed = 0.0;
        let mut first: Option<f32> = None;
        let mut second: Option<f32> = None;
        while elapsed < ELITE_START + 2.0 * ELITE_INTERVAL + 1.0 {
            elapsed += dt;
            d.elite_timer -= dt;
            if d.elite_timer <= 0.0 && elapsed > ELITE_START {
                d.elite_timer = ELITE_INTERVAL;
                if first.is_none() {
                    first = Some(elapsed);
                } else if second.is_none() {
                    second = Some(elapsed);
                }
            }
        }
        let first = first.expect("an elite should spawn");
        let second = second.expect("elites should keep coming");
        assert!(first <= ELITE_START + 2.0 * dt, "first elite late: {first}");
        assert!(
            (second - first - ELITE_INTERVAL).abs() <= 2.0 * dt,
            "elite cadence {} is not {ELITE_INTERVAL}",
            second - first
        );
    }
}
