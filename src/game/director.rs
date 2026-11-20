//! Time-based spawn director. Replaces fixed waves with a difficulty curve:
//! spawn rate, enemy HP, and the enemy mix all escalate with the run clock.
//!
//! ## Campaign levels
//!
//! The three maps are a campaign in order (Sewer → Kitchen → Lab — see
//! [`crate::game::campaign`]) and the director takes the campaign level index
//! every system tick (`resources::LevelIndex`). The level shifts the whole
//! difficulty curve with [`LEVEL_CLOCK_OFFSET`]: level 2 opens where level 1's
//! curve would be at 20 s and level 3 where it would be at 45 s. Every curve
//! function reads [`curve_clock`], so enemy hp, the spawn rate, the kind
//! unlocks and the elite gate all move together and "a harder level" is by
//! construction "further along level 0's curve". That is the baseline for a
//! player who walks into level 2 or 3 carrying three upgrades of their choice.
//!
//! ## The boss
//!
//! The director no longer spawns a boss from the spawn ring. At [`BOSS_TIME`]
//! it raises the `BossDue` signal ([`raise_boss_due`]) and the boss framework
//! spawns the themed boss in its arena. The 25 s warning before [`BOSS_TIME`]
//! is unchanged (`render::overlay::BOSS_WARNING_SECS`). The run clock keeps
//! running, but [`curve_clock`] freezes at [`BOSS_TIME`], so the horde keeps
//! arriving at the level's late-curve rate for as long as the boss fight lasts:
//! the level ends when the boss or the rat dies, not when the clock does.

use rand::{Rng, RngExt};

use super::enemy::EnemyKind;
use crate::game::ecs::resources::BossDue;

/// The leg of the clock a *level* is paced against. Since the campaign change
/// this is the build-up, not a win condition: survival no longer ends a level,
/// the boss fight does (see the module docs). Kept as the pacing anchor the HUD
/// counts down from.
pub const RUN_SECONDS: f32 = 300.0;
pub const BOSS_TIME: f32 = RUN_SECONDS - 45.0;
/// Elites (buffed brutes) only start spawning once the *curve* clock passes
/// this — the world's elite branch gates on the same instant.
pub const ELITE_START: f32 = 60.0;
/// Gap between elites once they start, and the value the timer is reset to.
pub const ELITE_INTERVAL: f32 = 30.0;

/// How far along level 0's curve each campaign level opens, in seconds.
///
/// The player keeps three upgrades between levels, so a level-2 player is
/// expected to handle a mid-run horde from the first second. Level 1 opens at
/// level 0's 20 s mark (hp ×1.44, 4 enemies a pulse) and level 2 at level 0's
/// 45 s mark (hp ×2.0, 6 enemies a pulse). Tune here and nowhere else: every
/// curve function is a function of [`curve_clock`], so one table moves hp, the
/// spawn curve, the kind unlocks and the elite gate together.
pub const LEVEL_CLOCK_OFFSET: [f32; 3] = [0.0, 20.0, 45.0];

/// Seconds the rat has to hold one chamber before the director stages a flank.
///
/// Design change 6: a room is a good place to kite, but not a place to stand.
/// After this long in the same chamber (and only when the chamber has two real
/// approaches) the next pulse arrives as two off-screen groups through those
/// approaches instead of on the ring. Leaving the chamber resets the timer.
pub const FLANK_DWELL_SECS: f32 = 8.0;

/// `level` as an index into [`LEVEL_CLOCK_OFFSET`] (out-of-range clamps).
#[inline]
fn level_slot(level: usize) -> usize {
    level.min(LEVEL_CLOCK_OFFSET.len() - 1)
}

/// The difficulty clock, in seconds: the real run clock, frozen at
/// [`BOSS_TIME`], shifted by the level's opening offset.
///
/// Freezing at the boss time is what makes "the horde keeps spawning at the
/// level's late-curve rate until the level ends" true without the rate running
/// away: a ten-minute boss fight carries the same pressure from minute five on.
#[inline]
pub fn curve_clock(level: usize, elapsed: f32) -> f32 {
    elapsed.min(BOSS_TIME) + LEVEL_CLOCK_OFFSET[level_slot(level)]
}

#[derive(bevy_ecs::resource::Resource)]
pub struct Director {
    pub spawn_timer: f32,
    /// The boss phase has been handed to the boss framework (it used to mean
    /// "the generic boss has spawned"). Set exactly once, at [`BOSS_TIME`]; the
    /// HUD reads it for the pre-boss warning and the boss-phase labels.
    pub boss_spawned: bool,
    /// Counts down to the next elite. Starts spent: the first elite is gated on
    /// the *curve* clock reaching [`ELITE_START`] (so a later campaign level,
    /// whose curve opens further along, meets its first elite sooner), and
    /// after that the timer is the real-time [`ELITE_INTERVAL`].
    pub elite_timer: f32,
    /// How many spawn pulses have fired this run. Drives the room-wave cadence
    /// in `systems/spawn.rs` (every [`ROOM_WAVE_EVERY`]-th pulse brings its
    /// first enemy in through a doorway instead of the ring) and is exposed for
    /// the HUD/debug readouts.
    pub pulses: u32,
    /// Seconds the rat has held the same chamber (flank staging).
    pub chamber_dwell: f32,
    /// The chamber the dwell is measured in (`Level::rooms` index, -1 = none).
    pub chamber_room: i32,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            spawn_timer: 0.0,
            boss_spawned: false,
            // Spent: the first elite fires the moment the curve gate opens.
            // (It used to be seeded with ELITE_START so the countdown landed on
            // the gate; with the gate itself on the curve clock that seeding
            // would be one level's worth of wrong.)
            elite_timer: 0.0,
            pulses: 0,
            chamber_dwell: 0.0,
            chamber_room: -1,
        }
    }
}

impl Director {
    /// Advance the chamber-dwell timer for this tick. `room` is the player's
    /// chamber index (`None` in a corridor or on open floor), and leaving the
    /// chamber resets the clock — the flank is the reward for *staying*.
    pub fn note_chamber(&mut self, room: Option<usize>, dt: f32) {
        let room = room.map(|i| i as i32).unwrap_or(-1);
        if room != self.chamber_room {
            self.chamber_room = room;
            self.chamber_dwell = 0.0;
        }
        if room >= 0 {
            self.chamber_dwell += dt;
        }
    }

    /// Has the rat held this chamber long enough to be flanked?
    #[inline]
    pub fn flank_ready(&self) -> bool {
        self.chamber_dwell >= FLANK_DWELL_SECS
    }

    /// Spend the flank the timer earned: resets the dwell so the next one is
    /// another full dwell away. Call this on the pulse that attempts a flank,
    /// whether or not the chamber offered two approaches.
    pub fn take_flank(&mut self) -> bool {
        if !self.flank_ready() {
            return false;
        }
        self.chamber_dwell = 0.0;
        true
    }
}

/// Hand the boss fight over to the boss framework (worker P7).
///
/// Called by `systems/spawn.rs::director_system` once the run clock reaches
/// [`BOSS_TIME`]; raises `BossDue` exactly once and returns `true` on that
/// tick. The boss framework can either call this itself or replace the call
/// site — either way the director stops touching the boss from here on (it no
/// longer spawns one, shakes the screen or plays `Sfx::Boss`; those belong to
/// the boss's arrival).
pub fn raise_boss_due(director: &mut Director, due: &mut BossDue, elapsed: f32, level: usize) -> bool {
    if director.boss_spawned || elapsed < BOSS_TIME {
        return false;
    }
    director.boss_spawned = true;
    due.due = true;
    due.claimed = false;
    due.level = level;
    due.at = elapsed;
    true
}

/// How tough enemies are right now (HP multiplier) on the curve clock.
#[inline]
pub fn hp_mult_at(curve: f32) -> f32 {
    1.0 + curve / 45.0
}

/// [`hp_mult_at`] for a campaign level and the real run clock.
#[inline]
pub fn hp_mult_for(level: usize, elapsed: f32) -> f32 {
    hp_mult_at(curve_clock(level, elapsed))
}

/// The director's actual hp curve at the real run clock (level 0): the curve
/// clock, so it freezes at [`BOSS_TIME`] like the running system.
#[inline]
pub fn hp_mult(elapsed: f32) -> f32 {
    hp_mult_at(curve_clock(0, elapsed))
}

/// Seconds between spawn pulses at curve time `curve`, shrinking over time
/// (floor 0.20 s).
///
/// The floor came down from 0.28 s and the pulse grew (see [`pulse_count_at`])
/// for the ten-times-larger map: the horde has to fill 3200 × 2560 px instead
/// of 1020 × 800, so the late game needs a noticeably higher spawn rate to keep
/// the screen busy while the player kites across it.
pub fn spawn_interval_at(curve: f32) -> f32 {
    (1.4 - curve / 220.0).max(0.20)
}

/// [`spawn_interval_at`] for a campaign level and the real run clock.
#[inline]
pub fn spawn_interval_for(level: usize, elapsed: f32) -> f32 {
    spawn_interval_at(curve_clock(level, elapsed))
}

/// The director's actual pulse interval at the real run clock (level 0), i.e.
/// frozen at [`BOSS_TIME`]. The 0.20 s floor lives in [`spawn_interval_at`].
#[inline]
pub fn spawn_interval(elapsed: f32) -> f32 {
    spawn_interval_at(curve_clock(0, elapsed))
}

/// Enemies emitted per pulse at curve time `curve`, growing over time.
///
/// The starting pulse grew from 2 to 3 (three enemies on a 3 200 px map are
/// barely a blip) and the ramp steepened from `t/16` to `t/12`: with the ring
/// finally wall-clear and reachable, every spawn now actually reaches the rat,
/// so `3 + t/16` was measured to peak at only 35 live enemies in the scripted
/// crowd case; `3 + t/12` clears the "peak ≥ 40 before 60 s" acceptance test
/// while the late game stays clamped at [`crate::game::ecs::constants::ENEMY_CAP`]
/// from about 160 s on.
pub fn pulse_count_at(curve: f32) -> usize {
    (3.0 + curve / 12.0) as usize
}

/// [`pulse_count_at`] for a campaign level and the real run clock.
#[inline]
pub fn pulse_count_for(level: usize, elapsed: f32) -> usize {
    pulse_count_at(curve_clock(level, elapsed))
}

/// The director's actual pulse size at the real run clock (level 0), i.e.
/// frozen at [`BOSS_TIME`].
#[inline]
pub fn pulse_count(elapsed: f32) -> usize {
    pulse_count_at(curve_clock(0, elapsed))
}

/// Spawn rate in enemies per second at curve time `curve` (tests, tuning).
#[inline]
pub fn spawn_rate_at(curve: f32) -> f32 {
    pulse_count_at(curve) as f32 / spawn_interval_at(curve).max(1e-3)
}

/// [`spawn_rate_at`] for a campaign level and the real run clock.
pub fn spawn_rate_for(level: usize, elapsed: f32) -> f32 {
    spawn_rate_at(curve_clock(level, elapsed))
}

/// The director's actual spawn rate at the real run clock (level 0), i.e.
/// frozen at [`BOSS_TIME`].
pub fn spawn_rate(elapsed: f32) -> f32 {
    spawn_rate_at(curve_clock(0, elapsed))
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

/// Weighted pick of an enemy kind appropriate to curve time `curve`.
pub fn pick_kind_at(curve: f32, rng: &mut impl Rng) -> EnemyKind {
    // One slot per possible kind, on the stack: this runs on every spawn roll
    // and must not allocate.
    let mut table = [(EnemyKind::Skitterer, 0.0f32); 1 + UNLOCKS.len()];
    table[0] = (EnemyKind::Skitterer, 6.0);
    let mut n = 1;
    for &(unlock, kind, weight) in UNLOCKS.iter() {
        if curve > unlock {
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

/// [`pick_kind_at`] for a campaign level and the real run clock.
#[inline]
pub fn pick_kind_for(level: usize, elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    pick_kind_at(curve_clock(level, elapsed), rng)
}

/// Weighted pick of an enemy kind appropriate to the current time (level 0),
/// i.e. frozen at [`BOSS_TIME`] like the running system.
pub fn pick_kind(elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    pick_kind_at(curve_clock(0, elapsed), rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::campaign::LEVEL_COUNT;

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
        // The floor lives in the raw curve; the running curve freezes at the
        // boss time and never gets there.
        assert!((spawn_interval_at(1_000.0) - 0.20).abs() < 1e-6);
        assert_eq!(spawn_interval(1_000.0), spawn_interval(BOSS_TIME));
        // Monotone on the way there: no dip in rate, no dip in pulse size.
        let mut prev = 0.0f32;
        for t in [0.0f32, 30.0, 60.0, 120.0, 180.0, 240.0, 255.0, 300.0] {
            let rate = spawn_rate(t);
            assert!(rate >= prev - 1e-4, "spawn rate dipped at {t}s");
            prev = rate;
        }
        // Late: ~24 enemies every 0.24 s at the frozen boss-time curve, far
        // past the cap (the cap itself is `ENEMY_CAP`; this only proves the
        // curve wants to overflow it).
        assert!(spawn_rate(300.0) > 60.0, "late rate {}", spawn_rate(300.0));
        // And the early game is not a wall of bugs: the first seconds stay slow.
        assert!(spawn_rate(0.0) < 3.0, "early rate {}", spawn_rate(0.0));
    }

    /// The first elite lands on the curve gate on every campaign level, and
    /// then on the fixed real-time interval.
    #[test]
    fn the_first_elite_lands_on_the_curve_gate() {
        // The timer starts spent: the gate alone decides the first elite.
        let d = Director::default();
        assert_eq!(d.elite_timer, 0.0);

        let dt = 1.0 / 60.0;
        for (level, offset) in LEVEL_CLOCK_OFFSET.iter().enumerate().take(LEVEL_COUNT) {
            let offset = *offset;
            let mut d = Director::default();
            let mut elapsed = 0.0;
            let mut first: Option<f32> = None;
            let mut second: Option<f32> = None;
            while elapsed < 200.0 {
                elapsed += dt;
                d.elite_timer -= dt;
                if d.elite_timer <= 0.0 && curve_clock(level, elapsed) > ELITE_START {
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
            // The gate opens when the curve clock reaches ELITE_START.
            let expected = (ELITE_START - offset).max(0.0);
            assert!(
                (first - expected).abs() <= 2.0 * dt,
                "level {level}: first elite at {first}, gate at {expected}"
            );
            assert!(
                (second - first - ELITE_INTERVAL).abs() <= 2.0 * dt,
                "level {level}: elite cadence {} is not {ELITE_INTERVAL}",
                second - first
            );
        }
    }

    /// Level 2 and 3 are harder starts than level 1 at *every* instant: more
    /// hp, a bigger pulse, a shorter gap and a later point on the unlock table.
    /// This is the monotonicity the campaign scaling has to keep.
    #[test]
    fn campaign_levels_scale_the_curve_monotonically() {
        for t in [0.0f32, 30.0, 60.0, 120.0, 200.0, 255.0, 400.0] {
            for level in 1..LEVEL_COUNT {
                assert!(
                    curve_clock(level, t) > curve_clock(level - 1, t),
                    "the curve clock did not advance from level {} at {t}s",
                    level - 1
                );
                assert!(
                    hp_mult_for(level, t) > hp_mult_for(level - 1, t) + 1e-4,
                    "hp did not scale from level {} at {t}s",
                    level - 1
                );
                assert!(
                    pulse_count_for(level, t) >= pulse_count_for(level - 1, t),
                    "the pulse shrank from level {} at {t}s",
                    level - 1
                );
                assert!(
                    spawn_interval_for(level, t) <= spawn_interval_for(level - 1, t) + 1e-6,
                    "the interval grew from level {} at {t}s",
                    level - 1
                );
                assert!(
                    spawn_rate_for(level, t) >= spawn_rate_for(level - 1, t) - 1e-4,
                    "the spawn rate dipped from level {} at {t}s",
                    level - 1
                );
            }
        }
        // The opening really is a different game: level 3 starts at the hp and
        // the pulse count level 1 reaches ~45 s in.
        assert!((hp_mult_for(2, 0.0) - hp_mult_for(0, 45.0)).abs() < 1e-4);
        assert_eq!(pulse_count_for(2, 0.0), pulse_count_for(0, 45.0));
        // And out-of-range indices clamp instead of panicking.
        for level in LEVEL_COUNT..LEVEL_COUNT + 4 {
            assert_eq!(curve_clock(level, 10.0), curve_clock(LEVEL_COUNT - 1, 10.0));
        }
    }

    /// The elite cadence scales too: the first brute arrives as soon as the
    /// level's curve passes [`ELITE_START`], i.e. earlier on every later map,
    /// and the interval between elites is unchanged (curve pressure, not a new
    /// timer).
    #[test]
    fn elite_cadence_scales_with_the_level() {
        let dt = 1.0 / 60.0;
        let mut previous = f32::INFINITY;
        for level in 0..LEVEL_COUNT {
            let mut d = Director::default();
            let mut elapsed = 0.0;
            let mut first = None;
            while elapsed < 200.0 {
                elapsed += dt;
                d.elite_timer -= dt;
                if d.elite_timer <= 0.0 && curve_clock(level, elapsed) > ELITE_START {
                    d.elite_timer = ELITE_INTERVAL;
                    first.get_or_insert(elapsed);
                }
            }
            let first = first.expect("an elite should arrive on every level");
            assert!(
                first < previous,
                "level {level}'s first elite ({first:.1}s) is not earlier than the previous level's ({previous:.1}s)"
            );
            previous = first;
        }
    }

    /// The curve freezes at the boss time (the horde keeps its late-curve rate
    /// until the level ends instead of running away for the length of a boss
    /// fight), and it never goes backwards.
    #[test]
    fn the_curve_freezes_at_the_boss_time() {
        for level in 0..LEVEL_COUNT {
            assert_eq!(
                curve_clock(level, BOSS_TIME),
                curve_clock(level, BOSS_TIME + 600.0)
            );
            assert_eq!(
                spawn_rate_for(level, 400.0),
                spawn_rate_for(level, BOSS_TIME)
            );
            assert_eq!(hp_mult_for(level, 400.0), hp_mult_for(level, BOSS_TIME));
        }
        let mut prev = f32::NEG_INFINITY;
        for t in [0.0f32, 1.0, 60.0, 254.0, BOSS_TIME, 300.0, 1000.0] {
            let c = curve_clock(1, t);
            assert!(c >= prev, "the curve clock went backwards at {t}s");
            prev = c;
        }
    }

    /// The boss is handed over exactly once, at [`BOSS_TIME`], with the level
    /// the boss framework needs to pick the right boss.
    #[test]
    fn the_boss_is_due_once_at_the_boss_time() {
        let mut d = Director::default();
        let mut due = BossDue::default();
        assert!(!raise_boss_due(&mut d, &mut due, BOSS_TIME - 0.1, 2));
        assert!(!due.due && !d.boss_spawned);

        assert!(raise_boss_due(&mut d, &mut due, BOSS_TIME, 2));
        assert!(d.boss_spawned);
        assert!(due.due && !due.claimed);
        assert_eq!(due.level, 2);
        assert_eq!(due.at, BOSS_TIME);
        // P7 consumes it once.
        assert!(due.take());
        assert!(!due.take());

        // Never again, however long the fight lasts.
        assert!(!raise_boss_due(&mut d, &mut due, BOSS_TIME + 300.0, 2));
        assert!(!due.is_pending());
    }

    /// The flank timer only runs while the rat holds one chamber, and spends
    /// itself in one attempt.
    #[test]
    fn the_flank_needs_one_chamber_and_eight_seconds() {
        let mut d = Director::default();
        let dt = 1.0 / 60.0;
        for _ in 0..(FLANK_DWELL_SECS * 60.0 - 1.0).round() as usize {
            d.note_chamber(Some(3), dt);
        }
        assert!(!d.flank_ready(), "flanked too early: {}", d.chamber_dwell);
        d.note_chamber(Some(3), dt);
        assert!(d.flank_ready());
        assert!(d.take_flank(), "a ready flank must fire");
        assert!(!d.flank_ready(), "the flank did not spend the timer");
        assert!(!d.take_flank());

        // Leaving the chamber resets it, and a corridor (no chamber) never
        // charges it.
        for _ in 0..600 {
            d.note_chamber(Some(9), dt);
        }
        assert!(d.flank_ready());
        d.note_chamber(Some(4), dt);
        assert!(!d.flank_ready(), "changing chamber kept the old dwell");
        for _ in 0..600 {
            d.note_chamber(None, dt);
        }
        assert!(!d.flank_ready(), "a corridor charged the dwell");
    }
}
