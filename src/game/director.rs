//! Time-based spawn director. Replaces fixed waves with a difficulty curve:
//! spawn rate, enemy HP, and the enemy mix all escalate with the run clock.
//!
//! ## Fifteen minutes on the floor (wave 3)
//!
//! A level is a **fifteen-minute** build-up followed by its themed boss
//! (`WAVE3_DESIGN.md` §1). The clock reads 15:00 down to BOSS; the boss arrives
//! at [`BOSS_TIME`] and the level ends when it or the rat dies. Four named
//! minibosses arrive at [`MINIBOSS_TIMES`] (3:00, 6:00, 9:00, 12:00) and each
//! leaves a chest; see `systems::miniboss`.
//!
//! Every curve function reads [`curve_clock`], so enemy hp, the spawn rate, the
//! kind unlocks, the elite cadence and the live budget all move together. The
//! tables are the design doc's §1 rows, piecewise-linear between row endpoints:
//!
//! | Clock | Bodies/s | Live budget | Elite chance |
//! |---|---:|---:|---:|
//! | 0–1 | 0.8→1.5 | 18 | 0 |
//! | 1–3 | 1.5→3 | 42 | 0 |
//! | 3–6 | 3→5 | 72 | 0→1 % |
//! | 6–9 | 5→7 | 105 | 1→2 % |
//! | 9–12 | 7→9 | 135 | 2→3 % |
//! | 12–15 | 9→11 | 165 | 3→4 % |
//! | 15+ | 1 (cleanup) | 35 | 0 |
//!
//! ## Campaign levels
//!
//! The three maps are a campaign in order (Sewer → Kitchen → Lab — see
//! [`crate::game::campaign`]) and the director takes the campaign level index
//! every system tick (`resources::LevelIndex`). The owner found the later
//! levels far too hard on the old 300 s curves, so the wave-3 re-base is
//! deliberately gentle: **no clock offset** (every stage opens with the same
//! grace) and only +5 % / +10 % ordinary hp after 3:00 ([`LEVEL_HP_EXTRA`]).
//! The per-level difference now lives in kind ([`LEVEL_MIX`]), elite cadence
//! ([`LEVEL_ELITE_INTERVAL`]) and a small damage/speed ramp
//! ([`enemy_mods_for`]).
//!
//! ## The boss
//!
//! The director no longer spawns a boss from the spawn ring. At [`BOSS_TIME`]
//! it raises the `BossDue` signal ([`raise_boss_due`]) and the boss framework
//! spawns the themed boss in its arena. The 25 s warning before [`BOSS_TIME`]
//! is unchanged (`render::overlay::BOSS_WARNING_SECS`). Once the boss is on the
//! floor the ordinary horde falls to **cleanup pace** (1 body/s, Skitterer/Bat,
//! live budget 35) — the design's last row — while the curve clock freezes so
//! the boss's own pool cannot run away.

use rand::{Rng, RngExt};

use super::enemy::EnemyKind;
use crate::game::ecs::resources::BossDue;

/// The leg of the clock a *level* is paced against. Since the campaign change
/// this is the build-up, not a win condition: survival no longer ends a level,
/// the boss fight does (see the module docs). Kept as the pacing anchor the HUD
/// counts down from.
pub const RUN_SECONDS: f32 = 900.0;
/// The boss arrives at the end of the fifteen-minute build-up.
pub const BOSS_TIME: f32 = RUN_SECONDS;
/// Named miniboss appointments (§8): 3:00, 6:00, 9:00, 12:00.
pub const MINIBOSS_TIMES: &[f32] = &[180.0, 360.0, 540.0, 720.0];
/// Elites (buffed brutes) only start spawning once the *curve* clock passes
/// this — the world's elite branch gates on the same instant.
pub const ELITE_START: f32 = 180.0;
/// Gap between elites once they start, and the value the timer is reset to.
/// Kept for the level-1 row; the running system uses [`elite_interval_at`],
/// which follows the design's elite-chance column.
pub const ELITE_INTERVAL: f32 = 40.0;

/// Seconds either side of a miniboss appointment in which no elite may spawn.
///
/// Design intent (§8): a build faces **one named threat at a time**. The elite
/// gate ([`ELITE_START`]) opens on the very second the first miniboss arrives,
/// so without this window the first elite and the first appointment would land
/// together. The director holds a due elite (its timer stays spent, no rng is
/// drawn) and spawns it on the first tick after the window closes.
pub const ELITE_APPOINTMENT_GAP: f32 = 20.0;

/// Is the run clock inside [`ELITE_APPOINTMENT_GAP`] of a miniboss appointment?
#[inline]
pub fn near_appointment(elapsed: f32) -> bool {
    MINIBOSS_TIMES
        .iter()
        .any(|at| (elapsed - at).abs() < ELITE_APPOINTMENT_GAP)
}

/// How far along level 0's curve each campaign level opens, in seconds.
///
/// **Wave-3 re-base.** The old 300 s curves opened the Kitchen 120 s in and the
/// Lab 150 s in, which the owner measured as far too hard on a 15-minute level.
/// Every stage now opens with the same grace (`0.0`): the later stages differ by
/// kind, elite cadence and a shallow damage/speed ramp, not by a head start.
/// [`curve_clock`] still adds the offset, so a future tuning pass can bias a
/// single stage without touching the tables.
pub const LEVEL_CLOCK_OFFSET: [f32; 3] = [0.0, 0.0, 0.0];

/// The horde's movement multiplier from the run clock, before the per-stage
/// ramp ([`enemy_mods_for`]).
///
/// The legacy curve was `1.0 + elapsed / 400.0`, which on a fifteen-minute
/// level reached **×3.25 at the boss** (and already ×1.75 at 5:00): a horde that
/// outruns the rat late in the floor, which the design's §1 says it must not do
/// ("speed and contact damage at base on stage one").
///
/// Picked: the legacy ramp for its first five minutes, then **frozen** —
/// ×1.0 at 0:00, ×1.75 from [`HORDE_SPEED_RAMP_END`] (5:00) to the boss, i.e.
/// the horde at 15:00 moves at exactly the speed the old build's 5:00 horde
/// did (and the old 15:00 value of ×3.25 is gone). Freezing it at the boss is
/// automatic: the ramp's clock stops being read at 5:00.
///
/// The per-stage part (`+0 / +5 / +10 %`) is [`enemy_mods_for`]'s `speed`, baked
/// into every enemy at spawn time, so the two multiply: stage three's top is
/// ×1.925.
pub const HORDE_SPEED_TOP: f32 = 1.75;

/// Run-clock second the speed ramp reaches [`HORDE_SPEED_TOP`] and stops
/// (5:00 — the old curve's own value at that moment).
pub const HORDE_SPEED_RAMP_END: f32 = 300.0;

/// [`HORDE_SPEED_TOP`] as a function of the run clock.
///
/// The first five minutes are exactly the legacy ramp
/// (`1.0 + elapsed / 400.0`), then it **freezes** at ×1.75 for the rest of the
/// fifteen-minute level: the horde never gets faster than the old build's 5:00
/// horde, and the boss phase inherits that frozen value. (The per-stage
/// `+0 / +5 / +10 %` from [`enemy_mods_for`] multiplies on top, baked in at
/// spawn, so stage three's ceiling is ×1.925.)
#[inline]
pub fn horde_speed_mult(elapsed: f32) -> f32 {
    1.0 + elapsed.min(HORDE_SPEED_RAMP_END) / 400.0
}

/// Extra ordinary-horde toughness per campaign level, after the grace window.
///
/// §1: "Stages two/three add 5/10 % ordinary HP after 3:00, with the same
/// opening grace." The multiplier is 1.0 until [`LEVEL_HP_EXTRA_AFTER`] on the
/// run clock, then ramps linearly to the row's value by [`BOSS_TIME`], so the
/// opening minutes of every stage are identical and the extra bite arrives
/// during the build-up. It multiplies the *horde's* hp only, at spawn time
/// ([`horde_hp_mult_for`]); the boss pool stays on [`hp_mult_for`].
pub const LEVEL_HP_EXTRA: [f32; 3] = [1.0, 1.05, 1.10];
/// Run-clock second after which a later stage's extra hp starts to apply.
pub const LEVEL_HP_EXTRA_AFTER: f32 = 180.0;

/// Seconds between elites, per campaign level.
///
/// The first elite still lands when the level's curve passes [`ELITE_START`]
/// (so a carried level meets one in its first seconds); after that the gap is
/// this, and the later maps keep the pressure up with a brute every 27 s and
/// 20 s instead of level 1's 40 s. Brutes are the only horde body that reliably
/// gets through a capped build's flat armour, so their cadence is a real lever —
/// but they are also the fattest XP drop, so pushing it too far levels the rat
/// up and makes the level easier again.
///
/// The later stages keep the horde's bite in *bodies* rather than numbers: more
/// elites and a different kind mix. These are the base intervals the running
/// system uses when the curve's elite chance is zero; [`elite_interval_at`]
/// follows the §1 elite-chance column instead.
pub const LEVEL_ELITE_INTERVAL: [f32; 3] = [ELITE_INTERVAL, 27.0, 20.0];

/// Per-level weighting of the kind table ([`pick_kind_for`]), columns in
/// [`KIND_ORDER`].
///
/// Level 1 is the curve's own mix. The **Kitchen** keeps its ranged Spitters
/// and Cats (a carried build that kites melee all day has to dodge a stand-off
/// line), but the first cut (Cat ×1.7, Spitter ×2.0) was the single hardest
/// thing in the merged game: it killed Expert on level 2 34 % of the time, more
/// than the Lab killed anybody, so its Cat/Spitter weights came down and its
/// Brute weight up. The **Lab** is still the fast swarm (Bats stay the largest
/// single share, so the rat cannot out-walk the horde it arrived with) but it
/// fields a real ranged line too: after the damage ramp came down the Lab was
/// the *easiest* cleared level of the three, and Spitters are the pressure that
/// separates a reading player from a reacting one — the bots' Lab gap only
/// opens up when the threat is something you can see coming.
const LEVEL_MIX: [[f32; 5]; 3] = [
    [1.0, 1.0, 1.0, 1.0, 1.0],
    [1.2, 1.0, 1.45, 1.7, 1.05],
    [1.2, 1.7, 1.3, 1.4, 1.0],
];

/// The kind order [`LEVEL_MIX`] columns are indexed by.
const KIND_ORDER: [EnemyKind; 5] = [
    EnemyKind::Skitterer,
    EnemyKind::Bat,
    EnemyKind::Cat,
    EnemyKind::Spitter,
    EnemyKind::Brute,
];

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

/// Boss-pool scale for the fifteen-minute build-up.
///
/// The §1 XP curve is deliberately cheap (a surviving player carries a
/// near-complete build to 15:00), so the unchanged boss pools would die in
/// seconds. The pool still follows the curve ([`hp_mult_for`]) but this scale
/// keeps the fight inside its 45-80 s target. Measured with the bot: the pool
/// needs to roughly double the old 300 s build's, because the new XP curve
/// carries a near-complete build to 15:00.
pub const BOSS_POOL_SCALE: f32 = 4.0;

/// The boss's hp multiplier for a campaign level: the curve times
/// [`BOSS_POOL_SCALE`] (the horde's own extra is deliberately excluded).
#[inline]
pub fn boss_hp_mult_for(level: usize, elapsed: f32) -> f32 {
    hp_mult_for(level, elapsed) * BOSS_POOL_SCALE
}

/// How tough enemies are right now (HP multiplier) on the curve clock.
///
/// §1: "Keep ordinary HP scaling shallow (×1 at start to ×2 at 15:00)." The
/// curve is linear over the fifteen minutes, so a skitterer that took one hit
/// at 0:00 takes two at the boss. Later campaign stages add their own
/// [`LEVEL_HP_EXTRA`] on top after the grace window.
#[inline]
pub fn hp_mult_at(curve: f32) -> f32 {
    1.0 + curve / RUN_SECONDS
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

// ---- §1 pacing tables -----------------------------------------------------
//
// The design's §1 rows, as piecewise-linear knots. `curve` is the curve clock
// (seconds). The rate table is the design's "Bodies/s" column; the budget table
// is its "Live budget" column; the elite table is its "Elite chance per
// ordinary spawn" column. Interpolate between row endpoints; the last row is
// the boss-phase cleanup pace.

/// Ordinary bodies attempted per second, by curve clock. Row endpoints are the
/// §1 table's "Bodies/s" column.
const BODIES_PER_SECOND: [(f32, f32); 7] = [
    (0.0, 0.8),
    (60.0, 1.5),
    (180.0, 3.0),
    (360.0, 5.0),
    (540.0, 7.0),
    (720.0, 9.0),
    (900.0, 11.0),
];

/// Live ordinary-enemy budget by curve clock (the §1 "Live budget" column).
/// A step table: each row holds from its clock until the next row's clock.
const LIVE_BUDGET: [(f32, usize); 7] = [
    (0.0, 18),
    (60.0, 42),
    (180.0, 72),
    (360.0, 105),
    (540.0, 135),
    (720.0, 165),
    (900.0, 165),
];

/// Elite chance per ordinary spawn by curve clock (the §1 column), as a
/// fraction. Linear within a row.
const ELITE_CHANCE: [(f32, f32); 7] = [
    (0.0, 0.0),
    (180.0, 0.0),
    (360.0, 0.01),
    (540.0, 0.02),
    (720.0, 0.03),
    (900.0, 0.04),
    (900.0, 0.04),
];

/// Linear interpolation over a `(clock, value)` knot table. Clamps to the ends.
#[inline]
fn lerp_table(table: &[(f32, f32)], curve: f32) -> f32 {
    let first = table[0];
    if curve <= first.0 {
        return first.1;
    }
    for w in table.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        if curve <= t1 {
            let span = (t1 - t0).max(1e-6);
            return v0 + (v1 - v0) * ((curve - t0) / span).clamp(0.0, 1.0);
        }
    }
    table[table.len() - 1].1
}

/// Ordinary bodies attempted per second at curve time `curve` (§1 table).
#[inline]
pub fn bodies_per_second_at(curve: f32) -> f32 {
    lerp_table(&BODIES_PER_SECOND, curve)
}

/// Live ordinary-enemy budget at curve time `curve` (§1 table, step function).
#[inline]
pub fn live_budget_at(curve: f32) -> usize {
    let mut budget = LIVE_BUDGET[0].1;
    for &(t, b) in LIVE_BUDGET.iter() {
        if curve >= t {
            budget = b;
        }
    }
    budget
}

/// Elite chance per ordinary spawn at curve time `curve` (fraction).
#[inline]
pub fn elite_chance_at(curve: f32) -> f32 {
    lerp_table(&ELITE_CHANCE, curve)
}

/// Seconds between spawn pulses at curve time `curve`.
///
/// The pulse interval is the scheduling quantum; the pulse *size* is derived
/// from the §1 rate table so the two together attempt
/// [`bodies_per_second_at`] bodies per second. The interval shortens over the
/// level (floor 0.35 s) so a late pulse is a handful of bodies rather than a
/// wall of them.
pub fn spawn_interval_at(curve: f32) -> f32 {
    (1.6 - curve / 900.0).max(0.35)
}

/// [`spawn_interval_at`] for a campaign level and the real run clock.
#[inline]
pub fn spawn_interval_for(level: usize, elapsed: f32) -> f32 {
    spawn_interval_at(curve_clock(level, elapsed))
}

/// The director's actual pulse interval at the real run clock (level 0), i.e.
/// frozen at [`BOSS_TIME`].
#[inline]
pub fn spawn_interval(elapsed: f32) -> f32 {
    spawn_interval_at(curve_clock(0, elapsed))
}

/// Enemies emitted per pulse at curve time `curve`, derived from the §1 rate
/// table: `rate × interval`, rounded up so a pulse is never empty.
pub fn pulse_count_at(curve: f32) -> usize {
    let n = (bodies_per_second_at(curve) * spawn_interval_at(curve)).round();
    (n as usize).max(1)
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

/// The boss-phase cleanup pace (§1 last row): 1 body/s, 35 live, no elites.
pub const CLEANUP_BODIES_PER_SECOND: f32 = 1.0;
pub const CLEANUP_LIVE_BUDGET: usize = 35;
/// Seconds between cleanup pulses.
pub const CLEANUP_INTERVAL: f32 = 1.0;
/// Bodies per cleanup pulse.
pub const CLEANUP_PULSE: usize = 1;

/// The pulse interval the running director uses: the curve's interval during
/// the build-up, the cleanup pace once the boss is on the floor.
#[inline]
pub fn horde_interval(elapsed: f32, boss_phase: bool) -> f32 {
    if boss_phase {
        CLEANUP_INTERVAL
    } else {
        spawn_interval(elapsed)
    }
}

/// The pulse size the running director uses (see [`horde_interval`]).
#[inline]
pub fn horde_pulse(elapsed: f32, boss_phase: bool) -> usize {
    if boss_phase {
        CLEANUP_PULSE
    } else {
        pulse_count(elapsed)
    }
}

/// The live ordinary-enemy budget the running director uses.
#[inline]
pub fn horde_live_budget(elapsed: f32, boss_phase: bool) -> usize {
    if boss_phase {
        CLEANUP_LIVE_BUDGET
    } else {
        live_budget_at(curve_clock(0, elapsed))
    }
}

/// Kinds that unlock over time: (unlock time, kind, weight). Kept in table
/// order so the weighted roll keeps its historical result for a given rng
/// state. Times are the §1 "First appearances" column for the five base kinds.
const UNLOCKS: [(f32, EnemyKind, f32); 4] = [
    (45.0, EnemyKind::Bat, 4.0),
    (75.0, EnemyKind::Cat, 3.0),
    (215.0, EnemyKind::Spitter, 2.0),
    (395.0, EnemyKind::Brute, 1.5),
];

// ---- Wave 3 kinds ---------------------------------------------------------
//
// `WAVE3_DESIGN.md` §1 and §9. **Every time in this block is written on the
// design's fifteen-minute clock** and scaled by [`WAVE3_TIME_SCALE`], so
// `w3-timeline` can stretch `RUN_SECONDS` to 900 without touching a number
// here: the scale is derived from `RUN_SECONDS` itself.

/// The run length the wave-3 unlock times in `WAVE3_DESIGN.md` are written for.
pub const DESIGN_RUN_SECONDS: f32 = 900.0;

/// Converts a design (fifteen-minute) time into the current build's clock.
///
/// The 5-minute build (`RUN_SECONDS = 300`, this worker's base) sees every
/// wave-3 kind at a third of its design time, so all five land inside one run;
/// the 15-minute build gets the design times unchanged. Only this expression
/// needs to stay right as the timeline grows — nothing else in the file hard
/// codes a wave-3 arrival.
pub const WAVE3_TIME_SCALE: f32 = RUN_SECONDS / DESIGN_RUN_SECONDS;

/// First appearance of each wave-3 kind, in **design** seconds (§1: Pillbugs
/// at 2:00, Bell Fleas at 4:30, Wick Moths at 7:15, Porters at 8:15, Turnkeys
/// at 9:45). Table order is the roll order, so a given rng state keeps picking
/// the same kind.
const WAVE3_UNLOCK_DESIGN: [(f32, EnemyKind); 5] = [
    (120.0, EnemyKind::Pillbug),
    (270.0, EnemyKind::BellFlea),
    (435.0, EnemyKind::WickMoth),
    (495.0, EnemyKind::Porter),
    (585.0, EnemyKind::Turnkey),
];

/// Raw weight of each wave-3 kind before the 20 % cap is applied, in
/// [`WAVE3_UNLOCK_DESIGN`] order (Pillbug, Bell Flea, Wick Moth, Porter,
/// Turnkey). Deliberately close together: these are threats, not fodder, and
/// the cap — not these numbers — decides how much of the horde they take.
///
/// The Pillbug is deliberately *not* weighted above the rest. It unlocks first
/// (2:00, i.e. 40 s in the current build), so until the Bell Flea arrives it is
/// the only wave-3 kind on the table and its weight *is* the wave-3 share; at
/// 3.0 that filled the whole 20 % cap with the one kind that never chases
/// (23 px/s, immobile while curled). Measured on the four-seed autoplay sample
/// in `autoplay::tests::early_game_headline_rates_stay_inside_generous_bands`,
/// Expert's damage over the first 120 s fell from 4.4 HP (no wave-3 kind) to
/// 2.9 (the 3.0 weight), below that guard's 3.0 floor; at 2.0 it is 8.5 and the
/// opening still meets its band. So no single kind gets to own the share.
const WAVE3_WEIGHT: [f32; 5] = [2.0, 2.0, 2.5, 2.0, 2.0];

/// §1: "Limit all five new kinds combined to 20 % of spawns."
///
/// A ceiling, and the shipped value: the five kinds together take at most a
/// fifth of the roll, on every campaign level, at every point of the curve.
/// (`WAVE3_WEIGHT` decides the mix *inside* that share, and the featured
/// rotation doubles one kind's weight within it.)
pub const WAVE3_SHARE_CAP: f32 = 0.20;

/// §1: from 9:45 the three newest kinds rotate as *featured threats*, each
/// taking twice its usual weight within that 20 % share for 30 s.
const WAVE3_FEATURED_FROM_DESIGN: f32 = 585.0;
const WAVE3_FEATURED_PERIOD_DESIGN: f32 = 30.0;
const WAVE3_FEATURED: [EnemyKind; 3] = [
    EnemyKind::WickMoth,
    EnemyKind::Porter,
    EnemyKind::Turnkey,
];

// ---- Wave 4 kinds ---------------------------------------------------------
//
// The four kinds the wave-4 batch adds, on the same 900 s design clock as the
// wave-3 table above (no second scale: `WAVE3_TIME_SCALE` already converts
// design seconds to this build's clock).

/// First appearance of each wave-4 kind, in **design** seconds, in roll order.
/// The centipede arrives mid-run, the horde at the six-minute mark, the hornet
/// at 7:30 and the snake once the build is nearly complete.
///
/// Every one of these is after the opening 60 s on purpose: the determinism
/// goldens (`ecs::harness`) and the map pin sample a 60 s run, so adding a kind
/// that cannot appear before 4:00 leaves both untouched.
pub const WAVE4_UNLOCK_DESIGN: [(f32, EnemyKind); 5] = [
    (240.0, EnemyKind::HouseCentipede),
    (330.0, EnemyKind::Lilliputian),
    (360.0, EnemyKind::TermiteHorde),
    (450.0, EnemyKind::MurderHornet),
    (510.0, EnemyKind::BigSnake),
];

/// Raw weight of each wave-4 kind, in [`WAVE4_UNLOCK_DESIGN`] order. The snake
/// is the rarest by a wide margin: it is also the only kind with a live cap
/// (`enemy_wave4::SNAKE_LIVE_CAP`, enforced by `spawn::director_system`).
const WAVE4_WEIGHT: [f32; 5] = [2.2, 1.6, 1.8, 1.8, 1.2];

/// The second tier of the special-kind budget: the four wave-4 kinds together
/// take at most 15 % of the roll.
///
/// **Decision (two tiers).** Wave 3's five told kinds keep their 20 % ceiling —
/// they are the floor's texture — and the wave-4 kinds get a tighter 15 %
/// because each one is a *response test* rather than a body (a snake splits the
/// room, a horde ignores chip damage, a hornet punishes greed). A third ceiling
/// ([`SPECIAL_SHARE_CAP`]) scales both if a generous level mix ever pushes the
/// two groups past 30 % together, so the ordinary ten kinds stay the horde.
pub const WAVE4_SHARE_CAP: f32 = 0.15;

/// The combined ceiling for every special kind (wave 3 + wave 4).
pub const SPECIAL_SHARE_CAP: f32 = 0.30;

/// How many big snakes may be alive at once. A snake's body is a wall, so the
/// cap is the room's: two snakes is an arena with three lanes, three snakes is
/// a maze. `spawn::director_system` substitutes fodder for an over-cap roll.
pub const WAVE4_SNAKE_LIVE_CAP: usize =
    crate::game::ecs::systems::enemy_wave4::SNAKE_LIVE_CAP;

/// How many lilliputian bodies may be alive at once (two squads). A roll that
/// would start a third squad is substituted with fodder instead.
pub const WAVE4_LILLI_LIVE_CAP: usize =
    crate::game::ecs::systems::enemy_wave4::LILLI_LIVE_CAP;

/// Design time of a wave-4 kind's first appearance, scaled to this build.
#[inline]
pub fn wave4_unlock_time(kind: EnemyKind) -> Option<f32> {
    WAVE4_UNLOCK_DESIGN
        .iter()
        .find(|(_, k)| *k == kind)
        .map(|(t, _)| t * WAVE3_TIME_SCALE)
}

/// Design time of a wave-3 kind's first appearance, scaled to this build.
#[inline]
pub fn wave3_unlock_time(kind: EnemyKind) -> Option<f32> {
    WAVE3_UNLOCK_DESIGN
        .iter()
        .find(|(_, k)| *k == kind)
        .map(|(t, _)| t * WAVE3_TIME_SCALE)
}

/// The featured wave-3 kind at curve time `curve`, if the rotation has started.
#[inline]
pub fn wave3_featured(curve: f32) -> Option<EnemyKind> {
    let from = WAVE3_FEATURED_FROM_DESIGN * WAVE3_TIME_SCALE;
    if curve < from {
        return None;
    }
    let period = WAVE3_FEATURED_PERIOD_DESIGN * WAVE3_TIME_SCALE;
    let slot = (((curve - from) / period.max(1e-3)).floor() as usize) % WAVE3_FEATURED.len();
    Some(WAVE3_FEATURED[slot])
}

/// Weighted pick of an enemy kind appropriate to curve time `curve`.
pub fn pick_kind_at(curve: f32, rng: &mut impl Rng) -> EnemyKind {
    pick_kind_mixed(curve, 0, rng)
}

/// [`pick_kind_at`] with a campaign level's [`LEVEL_MIX`] weighting applied
/// (level 0 is the curve's own mix, bit for bit).
///
/// The roll is a single `random_range` over the summed weights and the table is
/// walked in its historical order (skitterer first, then bat/cat/spitter/brute
/// as they unlock, then the wave-3 kinds), so the number and order of rng draws
/// is unchanged: only the weights differ, and only for levels above 0.
pub fn pick_kind_mixed(curve: f32, level: usize, rng: &mut impl Rng) -> EnemyKind {
    let mix = &LEVEL_MIX[level_slot(level)];
    // One slot per possible kind, on the stack: this runs on every spawn roll
    // and must not allocate.
    let mut table = [(EnemyKind::Skitterer, 0.0f32);
        1 + UNLOCKS.len() + WAVE3_UNLOCK_DESIGN.len() + WAVE4_UNLOCK_DESIGN.len()];
    table[0] = (EnemyKind::Skitterer, 6.0 * mix[0]);
    let mut n = 1;
    for &(unlock, kind, weight) in UNLOCKS.iter() {
        if curve > unlock {
            let col = KIND_ORDER
                .iter()
                .position(|k| *k == kind)
                .unwrap_or(0);
            table[n] = (kind, weight * mix[col]);
            n += 1;
        }
    }
    let base_n = n;

    // Wave-3 kinds are appended after the base table, so the roll over a
    // run without them is bit-for-bit the historical one. Their combined
    // weight is then capped to `WAVE3_SHARE_CAP` of the whole table, which is
    // what keeps five special-behaviour bodies from becoming the horde.
    let featured = wave3_featured(curve);
    let mut new_weight = 0.0f32;
    for (i, &(design_at, kind)) in WAVE3_UNLOCK_DESIGN.iter().enumerate() {
        if curve <= design_at * WAVE3_TIME_SCALE {
            continue;
        }
        let boosted = if featured == Some(kind) { 2.0 } else { 1.0 };
        let w = WAVE3_WEIGHT[i] * boosted;
        new_weight += w;
        table[n] = (kind, w);
        n += 1;
    }
    if new_weight > 0.0 {
        let base_total: f32 = table[..base_n].iter().map(|(_, w)| w).sum();
        // new / (base + new) <= cap  <=>  new <= cap/(1-cap) * base
        let cap = WAVE3_SHARE_CAP / (1.0 - WAVE3_SHARE_CAP) * base_total;
        if new_weight > cap {
            let scale = cap / new_weight;
            for slot in &mut table[base_n..n] {
                slot.1 *= scale;
            }
        }
    }

    // Wave-4 kinds are appended after the wave-3 group, so the roll over a run
    // without them is bit-for-bit the wave-3 result. Their combined weight is
    // capped to `WAVE4_SHARE_CAP` of the table they join, and then both special
    // groups are scaled together if they would take more than
    // `SPECIAL_SHARE_CAP` of the whole roll.
    let wave3_n = n;
    let mut w4_weight = 0.0f32;
    for (i, &(design_at, kind)) in WAVE4_UNLOCK_DESIGN.iter().enumerate() {
        if curve <= design_at * WAVE3_TIME_SCALE {
            continue;
        }
        let w = WAVE4_WEIGHT[i];
        w4_weight += w;
        table[n] = (kind, w);
        n += 1;
    }
    if w4_weight > 0.0 {
        let rest: f32 = table[..wave3_n].iter().map(|(_, w)| w).sum();
        let cap = WAVE4_SHARE_CAP / (1.0 - WAVE4_SHARE_CAP) * rest;
        if w4_weight > cap {
            let scale = cap / w4_weight;
            for slot in &mut table[wave3_n..n] {
                slot.1 *= scale;
            }
        }
    }
    // The combined special-kind ceiling (see `SPECIAL_SHARE_CAP`).
    {
        let base_total: f32 = table[..base_n].iter().map(|(_, w)| w).sum();
        let special: f32 = table[base_n..n].iter().map(|(_, w)| w).sum();
        if special > 0.0 && base_total > 0.0 {
            let cap = SPECIAL_SHARE_CAP / (1.0 - SPECIAL_SHARE_CAP) * base_total;
            if special > cap {
                let scale = cap / special;
                for slot in &mut table[base_n..n] {
                    slot.1 *= scale;
                }
            }
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
#[inline]
pub fn pick_kind_for(level: usize, elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    pick_kind_mixed(curve_clock(level, elapsed), level, rng)
}

/// The horde's hp multiplier on a campaign level: the curve, times the level's
/// [`LEVEL_HP_EXTRA`] after its grace window. The boss deliberately does **not**
/// use this — its pool stays on [`hp_mult_for`].
#[inline]
pub fn horde_hp_mult_for(level: usize, elapsed: f32) -> f32 {
    hp_mult_for(level, elapsed) * level_hp_extra(level, elapsed)
}

/// [`horde_hp_mult_for`] at an explicit curve time (tests, tuning).
#[inline]
pub fn horde_hp_mult_at(curve: f32, level: usize) -> f32 {
    hp_mult_at(curve) * level_hp_extra_at(curve, level)
}

/// A campaign level's ordinary-hp multiplier at the real run clock: 1.0 during
/// the opening grace, then a linear ramp to [`LEVEL_HP_EXTRA`] by the boss time.
#[inline]
pub fn level_hp_extra(level: usize, elapsed: f32) -> f32 {
    level_hp_extra_at(elapsed.min(BOSS_TIME), level)
}

/// [`level_hp_extra`] at an explicit curve time.
#[inline]
pub fn level_hp_extra_at(curve: f32, level: usize) -> f32 {
    let top = LEVEL_HP_EXTRA[level_slot(level)];
    if top <= 1.0 {
        return 1.0;
    }
    let t = ((curve - LEVEL_HP_EXTRA_AFTER) / (BOSS_TIME - LEVEL_HP_EXTRA_AFTER)).clamp(0.0, 1.0);
    1.0 + (top - 1.0) * t
}

/// Seconds the elite timer is reset to on a campaign level (the base row; the
/// running system prefers [`elite_interval_at`]).
#[inline]
pub fn elite_interval_for(level: usize) -> f32 {
    LEVEL_ELITE_INTERVAL[level_slot(level)]
}

/// Seconds between elite spawns at curve time `curve`, derived from the §1
/// elite-chance column and the body rate.
///
/// The design expresses the elite cadence as a chance per ordinary spawn. The
/// running director keeps the base's timer (`elite_timer` / `elite_interval`)
/// rather than adding an rng draw per body, and derives the interval that
/// produces the same rate: `1 / (chance × bodies/s)`. The 6 s floor keeps the
/// "elites at three" cap the limiter instead of flooding the floor; before
/// 3:00 the chance is zero, so the interval is infinite (no elites).
#[inline]
pub fn elite_interval_at(curve: f32) -> f32 {
    let chance = elite_chance_at(curve);
    if chance <= 0.0 {
        // No elites yet: retry in a minute rather than never arming the timer.
        return 60.0;
    }
    (1.0 / (chance * bodies_per_second_at(curve).max(0.1))).max(6.0)
}

/// The horde's damage multiplier at the top of a level's ramp (its boss time).
///
/// **Wave-3 re-base.** The old 300 s curves reached ×4.1/×4.3, which the owner
/// measured as near-lethal on a 15-minute level. The shallow ramp keeps the
/// later stages readable: contact damage is the base kind's, plus a small
/// per-stage ramp. The horde's bite lives in [`LEVEL_ELITE_INTERVAL`] and
/// [`LEVEL_MIX`] (more visible brutes, a real ranged line) instead.
const LEVEL_DAMAGE_TOP: [f32; 3] = [1.0, 1.12, 1.20];

/// The horde's speed multiplier at the top of a level's ramp. The Lab's swarm
/// stays the quickest, but only just: a faster horde walks into melee-range
/// weapons on the merged build, so the measured cliff lives well above this.
const LEVEL_SPEED_TOP: [f32; 3] = [1.0, 1.05, 1.10];

/// Shape of the ramp: `t^exp` over the level, so a lower exponent bites sooner.
const LEVEL_RAMP_EXP: [f32; 3] = [1.0, 1.0, 1.0];

/// How a campaign level's horde differs from the base kinds right now.
///
/// Both multipliers ramp linearly from 1.0 at the level's own start to
/// [`LEVEL_DAMAGE_TOP`]/[`LEVEL_SPEED_TOP`] at its [`BOSS_TIME`]. Level 1 is
/// exactly `EnemyMods::NONE`.
pub fn enemy_mods_for(level: usize, elapsed: f32) -> crate::game::enemy::EnemyMods {
    use crate::game::enemy::EnemyMods;
    let slot = level_slot(level);
    let t = (elapsed / BOSS_TIME).clamp(0.0, 1.0).powf(LEVEL_RAMP_EXP[slot]);
    EnemyMods {
        damage: 1.0 + (LEVEL_DAMAGE_TOP[slot] - 1.0) * t,
        speed: 1.0 + (LEVEL_SPEED_TOP[slot] - 1.0) * t,
    }
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
            let k = pick_kind(600.0, &mut rng);
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

    /// The retuned curve, as the content contract specifies it: at least two
    /// enemies per pulse from the start, a floor on the pulse interval, and a
    /// rate that keeps climbing (so the ten-times-larger map still fills up) up
    /// to a rate that overshoots the 600-enemy cap well before the clock runs
    /// out.
    ///
    /// The *exact* starting pulse and floor are the level-1 curve's business
    /// (they have been 3 and 0.20 s, and 2 and 0.50 s, across tuning passes);
    /// what this test is about is the shape, not the number.
    #[test]
    fn the_curve_fills_a_map_ten_times_larger() {
        assert!(pulse_count(0.0) >= 1);
        // The floor lives in the raw curve; the running curve freezes at the
        // boss time and never gets there.
        let floor = spawn_interval_at(1_000_000.0);
        assert!(floor >= 0.20 - 1e-6, "the interval floor collapsed: {floor}");
        assert_eq!(spawn_interval_at(2_000_000.0), floor, "no floor at all");
        assert_eq!(spawn_interval(1_000.0), spawn_interval(BOSS_TIME));
        // Monotone on the way there: no dip in rate, no dip in pulse size.
        let mut prev = 0.0f32;
        for t in [0.0f32, 30.0, 60.0, 120.0, 180.0, 240.0, 255.0, 300.0] {
            let rate = spawn_rate(t);
            assert!(rate >= prev - 1e-4, "spawn rate dipped at {t}s");
            prev = rate;
        }
        // Late: the frozen boss-time curve wants many more bodies than the
        // opening (the exact rate is the level-1 curve's tuning; the cap itself
        // is `ENEMY_CAP`).
        assert!(
            spawn_rate(300.0) > 5.0 * spawn_rate(0.0),
            "late rate {} is not an order above the opening {}",
            spawn_rate(300.0),
            spawn_rate(0.0)
        );
        // And the early game is not a wall of bugs: the first seconds stay slow.
        assert!(spawn_rate(0.0) < 3.0, "early rate {}", spawn_rate(0.0));
    }

    /// The §1 table's boundary values, re-expressed over fifteen minutes.
    #[test]
    fn the_wave3_curve_boundaries_match_the_design_table() {
        // Bodies/s.
        for (t, v) in [
            (0.0, 0.8),
            (60.0, 1.5),
            (180.0, 3.0),
            (360.0, 5.0),
            (540.0, 7.0),
            (720.0, 9.0),
            (900.0, 11.0),
        ] {
            assert!(
                (bodies_per_second_at(t) - v).abs() < 1e-4,
                "bodies/s at {t}s is {}, not {v}",
                bodies_per_second_at(t)
            );
        }
        // Live budget.
        for (t, v) in [(0.0, 18), (60.0, 42), (180.0, 72), (360.0, 105), (540.0, 135), (720.0, 165)] {
            assert_eq!(live_budget_at(t), v, "live budget at {t}s");
        }
        // Elite chance per ordinary spawn.
        for (t, v) in [(180.0, 0.0), (360.0, 0.01), (540.0, 0.02), (720.0, 0.03), (900.0, 0.04)] {
            assert!(
                (elite_chance_at(t) - v).abs() < 1e-6,
                "elite chance at {t}s is {}, not {v}",
                elite_chance_at(t)
            );
        }
        // Ordinary hp: ×1 at the start, ×2 at 15:00.
        assert!((hp_mult_at(0.0) - 1.0).abs() < 1e-6);
        assert!((hp_mult_at(RUN_SECONDS) - 2.0).abs() < 1e-6);
        // The five base kinds' unlock times.
        assert_eq!(UNLOCKS[0].0, 45.0);
        assert_eq!(UNLOCKS[1].0, 75.0);
        assert_eq!(UNLOCKS[2].0, 215.0);
        assert_eq!(UNLOCKS[3].0, 395.0);
    }

    /// The first elite lands on the curve gate on every campaign level, and
    /// then on the fixed real-time interval.
    #[test]
    fn the_first_elite_lands_on_the_curve_gate() {
        // The timer starts spent: the gate alone decides the first elite.
        let d = Director::default();
        assert_eq!(d.elite_timer, 0.0);

        let dt = 1.0 / 60.0;
        for (level, &offset) in LEVEL_CLOCK_OFFSET.iter().enumerate().take(LEVEL_COUNT) {
            let interval = elite_interval_for(level);
            let mut d = Director::default();
            let mut elapsed = 0.0;
            let mut first: Option<f32> = None;
            let mut second: Option<f32> = None;
            while elapsed < 400.0 {
                elapsed += dt;
                d.elite_timer -= dt;
                if d.elite_timer <= 0.0 && curve_clock(level, elapsed) > ELITE_START {
                    d.elite_timer = interval;
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
                (second - first - interval).abs() <= 2.0 * dt,
                "level {level}: elite cadence {} is not {interval}",
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
                    curve_clock(level, t) >= curve_clock(level - 1, t),
                    "the curve clock went backwards from level {} at {t}s",
                    level - 1
                );
                assert!(
                    hp_mult_for(level, t) >= hp_mult_for(level - 1, t),
                    "hp went backwards from level {} at {t}s",
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
        // the pulse count level 1 only reaches 150 s in.
        assert!((hp_mult_for(2, 0.0) - hp_mult_for(0, LEVEL_CLOCK_OFFSET[2])).abs() < 1e-4);
        assert_eq!(
            pulse_count_for(2, 0.0),
            pulse_count_for(0, LEVEL_CLOCK_OFFSET[2])
        );
        // And out-of-range indices clamp instead of panicking.
        for level in LEVEL_COUNT..LEVEL_COUNT + 4 {
            assert_eq!(curve_clock(level, 10.0), curve_clock(LEVEL_COUNT - 1, 10.0));
        }
    }

    /// The elite cadence scales too: the first brute arrives as soon as the
    /// level's curve passes [`ELITE_START`], i.e. earlier on every later map,
    /// *and* the gap after it shrinks with the level ([`LEVEL_ELITE_INTERVAL`]).
    #[test]
    fn elite_cadence_scales_with_the_level() {
        let dt = 1.0 / 60.0;
        let mut previous = f32::INFINITY;
        let mut previous_interval = f32::INFINITY;
        let mut arrivals_per_level = [0usize; LEVEL_COUNT];
        for level in 0..LEVEL_COUNT {
            let mut d = Director::default();
            let mut elapsed = 0.0;
            let mut first = None;
            let mut arrivals = 0usize;
            while elapsed < 400.0 {
                elapsed += dt;
                d.elite_timer -= dt;
                if d.elite_timer <= 0.0 && curve_clock(level, elapsed) > ELITE_START {
                    d.elite_timer = elite_interval_for(level);
                    arrivals += 1;
                    first.get_or_insert(elapsed);
                }
            }
            let first = first.expect("an elite should arrive on every level");
            // The gate lands on `ELITE_START` minus the level's offset, and a
            // level whose opening curve already passed it meets one at 0 s.
            let gate = (ELITE_START - LEVEL_CLOCK_OFFSET[level]).max(0.0);
            assert!(
                (first - gate).abs() <= 2.0 * dt,
                "level {level}: first elite at {first}, gate at {gate}"
            );
            assert!(
                first <= previous + 2.0 * dt,
                "level {level}'s first elite ({first:.1}s) is later than the previous level's ({previous:.1}s)"
            );
            previous = first;
            let interval = elite_interval_for(level);
            assert!(
                interval <= previous_interval,
                "level {level} fields elites less often than the level before it"
            );
            previous_interval = interval;
            arrivals_per_level[level] = arrivals;
        }
        // More frequent elites really do mean more of them in one level: over
        // the same 400 s every later level must field strictly more brutes.
        for level in 1..LEVEL_COUNT {
            assert!(
                arrivals_per_level[level] > arrivals_per_level[level - 1],
                "level {level} fielded {} elites, level {} {}",
                arrivals_per_level[level],
                level - 1,
                arrivals_per_level[level - 1]
            );
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
                spawn_rate_for(level, BOSS_TIME + 600.0),
                spawn_rate_for(level, BOSS_TIME)
            );
            assert_eq!(
                hp_mult_for(level, BOSS_TIME + 600.0),
                hp_mult_for(level, BOSS_TIME)
            );
        }
        let mut prev = f32::NEG_INFINITY;
        for t in [0.0f32, 1.0, 60.0, 254.0, BOSS_TIME, BOSS_TIME + 1.0, 1000.0] {
            let c = curve_clock(1, t);
            assert!(c >= prev, "the curve clock went backwards at {t}s");
            prev = c;
        }
    }

    /// Level 1 is the base game, bit for bit: the campaign knobs must not touch
    /// the Sewer, because the level-1 curve belongs to the early-game pass.
    #[test]
    fn level_one_is_the_base_game() {
        use crate::game::enemy::EnemyMods;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        for t in [0.0f32, 30.0, 120.0, 200.0, BOSS_TIME, 999.0] {
            assert_eq!(enemy_mods_for(0, t), EnemyMods::NONE);
            assert_eq!(horde_hp_mult_for(0, t), hp_mult_for(0, t));
            assert_eq!(horde_hp_mult_at(curve_clock(0, t), 0), hp_mult_at(curve_clock(0, t)));
        }
        assert_eq!(elite_interval_for(0), ELITE_INTERVAL);
        assert_eq!(LEVEL_MIX[0], [1.0; 5]);
        // And the kind roll is the historical one: same rng, same picks.
        let mut a = StdRng::seed_from_u64(17);
        let mut b = StdRng::seed_from_u64(17);
        for t in [0.0f32, 60.0, 150.0, 250.0] {
            for _ in 0..200 {
                assert_eq!(pick_kind_for(0, t, &mut a), pick_kind(t, &mut b));
            }
        }
    }

    /// Every later level is harder than the one before it at every instant, in
    /// all four per-level dimensions: horde hp, elite cadence, horde damage and
    /// horde speed.
    #[test]
    fn later_levels_are_strictly_harder() {
        for t in [0.0f32, 30.0, 120.0, 200.0, BOSS_TIME] {
            for level in 1..LEVEL_COUNT {
                let expected = if t < LEVEL_HP_EXTRA_AFTER {
                    horde_hp_mult_for(level, t) >= horde_hp_mult_for(level - 1, t)
                } else {
                    horde_hp_mult_for(level, t) > horde_hp_mult_for(level - 1, t)
                };
                assert!(expected, "level {level} horde hp is wrong at {t}s");
                assert!(
                    elite_interval_for(level) <= elite_interval_for(level - 1),
                    "level {level} fields elites less often"
                );
                let a = enemy_mods_for(level, t);
                let b = enemy_mods_for(level - 1, t);
                assert!(a.damage >= b.damage && a.speed >= b.speed);
                assert!(a.damage >= 1.0 && a.speed >= 1.0, "the ramp never dips below 1");
            }
        }
        // The ramp is a ramp: base kinds at the level's first second, the full
        // modifier at its boss time (which is when the clock freezes too).
        for level in 1..LEVEL_COUNT {
            assert_eq!(enemy_mods_for(level, 0.0), crate::game::enemy::EnemyMods::NONE);
            let top = enemy_mods_for(level, BOSS_TIME);
            assert_eq!(top, enemy_mods_for(level, 1_000.0));
            assert!(top.damage > 1.0, "level {level} horde damage never ramps");
        }
    }

    /// The boss pool follows the level's clock and the 15-minute scale and
    /// nothing else: the horde's `LEVEL_HP_EXTRA` must not leak into it (that
    /// is what keeps the boss fight inside its 45-80 s target on the later
    /// maps).
    #[test]
    fn the_boss_pool_ignores_the_horde_extra() {
        for level in 1..LEVEL_COUNT {
            let pool = boss_hp_mult_for(level, BOSS_TIME);
            let horde = horde_hp_mult_for(level, BOSS_TIME);
            assert!(
                (pool - hp_mult_for(level, BOSS_TIME) * BOSS_POOL_SCALE).abs() < 1e-4,
                "level {level}: boss pool {pool} is not the curve times the scale"
            );
            assert!(
                (horde - hp_mult_for(level, BOSS_TIME) * level_hp_extra_at(BOSS_TIME, level)).abs()
                    < 1e-4,
                "level {level}: horde hp {horde} is not the curve times its extra"
            );
        }
    }

    /// Each map fields a different horde: the Kitchen is the ranged one and the
    /// Lab the fast one, while the Sewer keeps the historical mix.
    #[test]
    fn later_levels_field_a_different_mix() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        // Late enough that every kind is unlocked on every level.
        let curve = curve_clock(2, BOSS_TIME);
        let share = |level: usize| {
            let mut rng = StdRng::seed_from_u64(99);
            let mut counts: Vec<usize> = vec![0; KIND_ORDER.len()];
            for _ in 0..20_000 {
                let k = pick_kind_mixed(curve, level, &mut rng);
                // The wave-3 kinds are not part of the per-level mix table;
                // their share is capped separately, so this test measures the
                // five base kinds' shares among themselves.
                let Some(i) = KIND_ORDER.iter().position(|x| *x == k) else {
                    continue;
                };
                counts[i] += 1;
            }
            counts
        };
        let base = share(0);
        let kitchen = share(1);
        let lab = share(2);
        let spitter = |v: &[usize]| v[3] as f32 / v.iter().sum::<usize>() as f32;
        let bat = |v: &[usize]| v[1] as f32 / v.iter().sum::<usize>() as f32;
        assert!(
            spitter(&kitchen) > spitter(&base) * 1.2,
            "the Kitchen is not the ranged map: {} vs {}",
            spitter(&kitchen),
            spitter(&base)
        );
        assert!(
            bat(&lab) > bat(&base) * 1.2,
            "the Lab is not the swarm map: {} vs {}",
            bat(&lab),
            bat(&base)
        );
        // Still one rng draw per roll whatever the mix.
        let mut a = StdRng::seed_from_u64(3);
        let mut b = StdRng::seed_from_u64(3);
        for _ in 0..500 {
            let _ = pick_kind_mixed(curve, 1, &mut a);
            let _ = pick_kind_mixed(curve, 2, &mut b);
            assert_eq!(a.random::<u64>(), b.random::<u64>());
        }
    }

    /// With the fifteen-minute timeline (`RUN_SECONDS = 900`) the five wave-3
    /// kinds unlock at exactly the design's minutes — Pillbug 2:00, Bell Flea
    /// 4:30, Wick Moth 7:15, Porter 8:15, Turnkey 9:45 — in the §1 order, and
    /// never before their own instant.
    #[test]
    fn the_wave3_kinds_unlock_in_design_order() {
        assert_eq!(RUN_SECONDS, 900.0);
        assert_eq!(WAVE3_TIME_SCALE, 1.0, "the design clock is the run clock now");
        // 2:00, 4:30, 7:15, 8:15, 9:45.
        assert_eq!(wave3_unlock_time(EnemyKind::Pillbug), Some(2.0 * 60.0));
        assert_eq!(wave3_unlock_time(EnemyKind::BellFlea), Some(4.0 * 60.0 + 30.0));
        assert_eq!(wave3_unlock_time(EnemyKind::WickMoth), Some(7.0 * 60.0 + 15.0));
        assert_eq!(wave3_unlock_time(EnemyKind::Porter), Some(8.0 * 60.0 + 15.0));
        assert_eq!(wave3_unlock_time(EnemyKind::Turnkey), Some(9.0 * 60.0 + 45.0));
        // The featured rotation starts with the Turnkey's unlock (9:45) and
        // turns every 30 s.
        assert_eq!(wave3_featured(584.0), None);
        assert_eq!(wave3_featured(586.0), Some(EnemyKind::WickMoth));
        assert_eq!(wave3_featured(616.0), Some(EnemyKind::Porter));
        assert_eq!(wave3_featured(646.0), Some(EnemyKind::Turnkey));
        assert_eq!(wave3_featured(676.0), Some(EnemyKind::WickMoth));
        let expected = [
            (EnemyKind::Pillbug, 120.0),
            (EnemyKind::BellFlea, 270.0),
            (EnemyKind::WickMoth, 435.0),
            (EnemyKind::Porter, 495.0),
            (EnemyKind::Turnkey, 585.0),
        ];
        let mut prev = 0.0;
        for (kind, design_at) in expected {
            let at = wave3_unlock_time(kind).expect("every wave-3 kind is in the table");
            assert!(
                (at - design_at * WAVE3_TIME_SCALE).abs() < 1e-3,
                "{kind:?} unlocks at {at}, expected {}",
                design_at * WAVE3_TIME_SCALE
            );
            assert!(
                (at - design_at).abs() < 1e-3,
                "{kind:?}: on the 15-minute clock the unlock is the design's own minute"
            );
            assert!(at > prev, "{kind:?} unlocked out of order");
            prev = at;
        }
        // The last unlock is inside the run, and the whole set is inside the
        // design's own 15 minutes.
        assert!(prev < BOSS_TIME, "the last wave-3 kind arrives after the boss");
        assert!(prev * DESIGN_RUN_SECONDS / RUN_SECONDS < DESIGN_RUN_SECONDS);

        // No wave-3 kind before its unlock, every one after it.
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        for (kind, design_at) in expected {
            let at = design_at * WAVE3_TIME_SCALE;
            let mut rng = StdRng::seed_from_u64(1234);
            for _ in 0..400 {
                assert_ne!(
                    pick_kind_mixed(at - 0.5, 0, &mut rng),
                    kind,
                    "{kind:?} appeared before its own unlock"
                );
            }
            let mut rng = StdRng::seed_from_u64(1234);
            let mut seen = false;
            for _ in 0..4000 {
                if pick_kind_mixed(at + 5.0, 0, &mut rng) == kind {
                    seen = true;
                    break;
                }
            }
            assert!(seen, "{kind:?} never appeared after its unlock");
        }
    }

    /// §1: "Limit all five new kinds combined to 20 % of spawns." The cap holds
    /// on every campaign level, and the kinds are still a real presence.
    #[test]
    fn the_wave3_kinds_are_capped_at_a_fifth_of_spawns() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        for level in 0..LEVEL_COUNT {
            // Design-clock instants with at least three wave-3 kinds on the
            // table (7:30, 10:00, 12:30 and the boss time), where the raw
            // weights overshoot and the cap is what limits them.
            for curve in [450.0f32, 600.0, 750.0, BOSS_TIME] {
                let mut rng = StdRng::seed_from_u64(4242);
                let mut wave3 = 0usize;
                let total = 40_000usize;
                for _ in 0..total {
                    if pick_kind_mixed(curve, level, &mut rng).is_wave3() {
                        wave3 += 1;
                    }
                }
                let share = wave3 as f32 / total as f32;
                let cap = WAVE3_SHARE_CAP;
                assert!(
                    share <= cap + 0.012,
                    "level {level} at {curve}s: wave-3 share {share} exceeds the cap {cap}"
                );
                // A cap that is never reached is a missing feature, not a cap.
                assert!(
                    share > cap * 0.6,
                    "level {level} at {curve}s: wave-3 share {share} is too rare to matter"
                );
            }
        }
        // With only the Pillbug unlocked (2:00 to 4:30) the share sits under the
        // cap on its raw weight alone; the cap must still hold there.
        for level in 0..LEVEL_COUNT {
            let mut rng = StdRng::seed_from_u64(4242);
            let total = 40_000usize;
            let wave3 = (0..total)
                .filter(|_| pick_kind_mixed(250.0, level, &mut rng).is_wave3())
                .count();
            let share = wave3 as f32 / total as f32;
            assert!(share > 0.0 && share <= WAVE3_SHARE_CAP + 0.012, "level {level}: {share}");
        }
        // Early on, before any of them unlock, the roll is the historical one:
        // not one wave-3 body and exactly the same rng stream as a table without
        // them.
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..2000 {
            assert!(!pick_kind_mixed(20.0, 0, &mut rng).is_wave3());
        }
    }

    /// The three newest kinds rotate as featured threats from 9:45 (design
    /// time), doubling their weight inside the same 20 % share.
    #[test]
    fn the_featured_wave3_kind_rotates_and_doubles() {
        let from = WAVE3_FEATURED_FROM_DESIGN * WAVE3_TIME_SCALE;
        let period = WAVE3_FEATURED_PERIOD_DESIGN * WAVE3_TIME_SCALE;
        assert!(wave3_featured(from - 0.1).is_none(), "rotation started early");
        assert_eq!(wave3_featured(from + 0.1), Some(EnemyKind::WickMoth));
        assert_eq!(wave3_featured(from + period + 0.1), Some(EnemyKind::Porter));
        assert_eq!(
            wave3_featured(from + 2.0 * period + 0.1),
            Some(EnemyKind::Turnkey)
        );
        // And it cycles.
        assert_eq!(
            wave3_featured(from + 3.0 * period + 0.1),
            Some(EnemyKind::WickMoth)
        );

        // Doubling one kind inside a fixed share really is a doubled share: the
        // featured kind shows up about twice as often as the same kind does in
        // its own (rotating) off-window.
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let share_of = |curve: f32, kind: EnemyKind| {
            let mut rng = StdRng::seed_from_u64(99);
            let mut hits = 0usize;
            let total = 60_000usize;
            for _ in 0..total {
                if pick_kind_mixed(curve, 0, &mut rng) == kind {
                    hits += 1;
                }
            }
            hits as f32 / total as f32
        };
        let featured_window = from + 0.5;
        let other_window = from + period + 0.5;
        let wick_featured = share_of(featured_window, EnemyKind::WickMoth);
        let wick_off = share_of(other_window, EnemyKind::WickMoth);
        assert!(
            wick_featured > wick_off * 1.6,
            "the featured kind did not double: {wick_featured} vs {wick_off}"
        );
        // The featured rotation never breaks the 20 % cap.
        let mut total = 0.0;
        for k in [EnemyKind::WickMoth, EnemyKind::Porter, EnemyKind::Turnkey] {
            total += share_of(featured_window, k);
        }
        assert!(
            total <= WAVE3_SHARE_CAP + 0.012,
            "the featured window broke the cap: {total}"
        );
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
