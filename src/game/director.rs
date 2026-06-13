//! Time-based spawn director. Replaces fixed waves with a difficulty curve:
//! spawn rate, enemy HP, and the enemy mix all escalate with the run clock.

use rand::Rng;

use super::enemy::EnemyKind;

pub const RUN_SECONDS: f32 = 300.0; // survive 5 minutes to win
pub const BOSS_TIME: f32 = RUN_SECONDS - 45.0;

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
            elite_timer: 25.0,
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

/// Weighted pick of an enemy kind appropriate to the current time.
pub fn pick_kind(elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    // (kind, weight) — heavier kinds unlock as the run progresses.
    let mut table: Vec<(EnemyKind, f32)> = vec![(EnemyKind::Skitterer, 6.0)];
    if elapsed > 20.0 {
        table.push((EnemyKind::Bat, 4.0));
    }
    if elapsed > 45.0 {
        table.push((EnemyKind::Cat, 3.0));
    }
    if elapsed > 75.0 {
        table.push((EnemyKind::Spitter, 2.0));
    }
    if elapsed > 120.0 {
        table.push((EnemyKind::Brute, 1.5));
    }
    let total: f32 = table.iter().map(|(_, w)| w).sum();
    let mut roll = rng.gen_range(0.0..total);
    for (k, w) in &table {
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
    fn difficulty_escalates() {
        assert!(hp_mult(120.0) > hp_mult(0.0));
        assert!(spawn_interval(200.0) < spawn_interval(0.0));
        assert!(pulse_count(200.0) > pulse_count(0.0));
    }

    #[test]
    fn spawn_interval_has_floor() {
        assert!(spawn_interval(100000.0) >= 0.28 - 1e-6);
    }
}
