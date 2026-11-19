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

pub struct Director {
    pub spawn_timer: f32,
    pub boss_spawned: bool,
    pub elite_timer: f32,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            spawn_timer: 0.0,
            boss_spawned: false,
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

/// Seconds between spawn pulses, shrinking over time (floor 0.28s).
pub fn spawn_interval(elapsed: f32) -> f32 {
    (1.4 - elapsed / 220.0).max(0.28)
}

/// Enemies emitted per pulse, growing over time.
pub fn pulse_count(elapsed: f32) -> usize {
    (2.0 + elapsed / 22.0) as usize
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
        assert!(spawn_interval(100000.0) >= 0.28 - 1e-6);
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
