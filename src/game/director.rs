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
//! | 6–9 | 5→7 | 115 | 1→2 % |
//! | 9–12 | 7→9 | 150 | 2→3 % |
//! | 12–15 | 9→11 | 185 | 3→4 % |
//! | 15+ | 1 (cleanup) | 35 | 0 |
//!
//! ## Campaign levels
//!
//! A campaign is **four** levels on a branching route (Sewer → Kitchen or
//! Laundry → Lab or Printworks → the Flatbed ride, see
//! [`crate::game::campaign`]) and the director takes the campaign level index
//! every system tick (`resources::LevelIndex`). The owner found the later
//! levels far too hard on the old 300 s curves, so the wave-3 re-base is
//! deliberately gentle: **no clock offset** (every stage opens with the same
//! grace) and only +5 % / +10 % ordinary hp after 3:00 ([`LEVEL_HP_EXTRA`]).
//! The campaign level now only scales *numbers* (hp, damage, speed, elite
//! cadence): the **kinds** come from the floor's own roster
//! ([`crate::game::roster`]), looked up by [`Theme`](crate::game::level::Theme),
//! so a practice run ([`crate::game::world::World::difficulty_override`]` =
//! Some(0)`) still meets the Flatbed's cast with the Sewer's scalars.
//!
//! ## The boss
//!
//! The director no longer spawns a boss from the spawn ring. At [`BOSS_TIME`]
//! it raises the `BossDue` signal ([`raise_boss_due`]) and the boss framework
//! spawns the themed boss in its arena. The 25 s warning before [`BOSS_TIME`]
//! is unchanged (`render::overlay::BOSS_WARNING_SECS`). Once the boss is on the
//! floor the ordinary horde falls to **cleanup pace** (1 body/s, the floor's own
//! A/B fodder, live budget 35) — the roster's 15+ row — while the curve clock
//! freezes so the boss's own pool cannot run away.
//!
//! ## Kind selection
//!
//! The director no longer owns a kind table. [`crate::game::roster`] holds one
//! static row per floor, and every spawn source (ring pulses, room waves,
//! flanking groups and fallbacks) asks it the same question with the same clock
//! and the same live caps, so a floor can never spawn a kind it does not own and
//! a per-floor content pass only ever edits one roster row.
//!
//! Optional pressure beats move ordinary pulse supply from each recovery into
//! its preceding surge. The clock still gates the same roster, and every spawn
//! uses the usual placement and kind draws. Live caps and appointment grace
//! still apply, so the balance is on offered bodies, before those constraints.
//! Sewer opening grace reduces contact damage while keeping the early XP feed.
//! The default switch is off and preserves the original spawn timer and traces.

use rand::Rng;

use crate::detmath::DetF32;
use super::enemy::EnemyKind;
use crate::game::ecs::resources::BossDue;
use crate::game::level::Theme;
use crate::game::roster::{self, SlotCaps};

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
pub const LEVEL_CLOCK_OFFSET: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

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
///
/// **Batch seven** (measured, 96 campaigns): the design's +5 % / +10 % left
/// the Skilled bot at 16/16 wins and the Casual one at 9/16. The rows are now
/// +20 % / +45 % / +70 %, stage one included (its damage and speed stay at
/// base: see [`enemy_mods_for`]). **Batch twelve** adds the ride's row, one
/// step further along the same ramp: +90 % by its boss.
pub const LEVEL_HP_EXTRA: [f32; 4] = [1.2, 1.45, 1.7, 1.9];
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
/// elites and a different kind mix (the mix itself is the floor's roster — see
/// [`crate::game::roster`]). These are the base intervals the running system
/// uses when the curve's elite chance is zero; [`elite_interval_at`] follows
/// the §1 elite-chance column instead.
///
/// Batch twelve adds the ride's row (16 s) so the fourth level's cadence keeps
/// stepping down the same series.
pub const LEVEL_ELITE_INTERVAL: [f32; 4] = [ELITE_INTERVAL, 27.0, 20.0, 16.0];

// The old per-level kind weighting (`LEVEL_MIX` / `KIND_ORDER`) lived here.
// Kind selection is per floor now: see [`crate::game::roster`]. The campaign
// level still scales numbers only (hp, damage, speed, elite cadence), which is
// what keeps a practice run (level 0 scalars) on any floor faithful.

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
    /// Ordinary bodies carrying the approach speed boost: `(SpawnId, the speed
    /// scale they spawned with)`, sorted by id. See
    /// [`crate::game::ecs::systems::spawn::APPROACH_BOOST_GROUND`]: the spawn
    /// ring sits outside the *widest* view now, so a walking body is boosted
    /// until it comes into the unzoomed view and then put back on exactly the
    /// scale it spawned with. Never an rng draw, never a spawn, never a position.
    pub approach: Vec<(u64, f32)>,
    pub(crate) pressure: PressureState,
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
            approach: Vec::new(),
            pressure: PressureState::default(),
        }
    }
}

/// Per-tick modifiers shared with the contact, loot, and HUD passes.
pub(crate) struct PressureState {
    rates: Option<[(f32, f32); 2]>,
    warned: [bool; 2],
    pub(crate) warning: bool,
    pub(crate) cadence: f32,
    pub(crate) budget: f32,
    pub(crate) contact: f32,
    pub(crate) heal: f32,
}

impl Default for PressureState {
    fn default() -> Self {
        Self {
            rates: None,
            warned: [false; 2],
            warning: false,
            cadence: 1.0,
            budget: 1.0,
            contact: 1.0,
            heal: 0.0,
        }
    }
}

impl PressureState {
    pub(crate) fn tick(
        &mut self,
        tuning: &crate::tuning::Tuning,
        sector: usize,
        theme: Theme,
        elapsed: f32,
        boss_phase: bool,
    ) {
        let director = &tuning.director;
        if !director.pressure_beats {
            return;
        }
        self.warning = false;
        self.cadence = 1.0;
        self.budget = 1.0;
        self.contact = 1.0;
        self.heal = 0.0;
        if boss_phase || elapsed >= director.boss_time {
            return;
        }
        if theme == Theme::Sewer && elapsed < director.pressure_sewer_opening_secs {
            self.contact = 1.0 - director.pressure_sewer_opening_cut;
        }
        let scale = tuning.pressure_scale_for(sector);
        if scale == 0.0 {
            return;
        }
        for i in 0..2 {
            let start = director.pressure_starts[i];
            let surge_end = start + director.pressure_lengths[i];
            let recovery_end = surge_end + director.pressure_recovery_lengths[i];
            if elapsed >= start - director.pressure_warning_lead && elapsed < start && !self.warned[i] {
                self.warned[i] = true;
                self.warning = true;
            }
            if elapsed >= start && elapsed < recovery_end {
                let rates = self.rates.get_or_insert_with(|| director.pressure_rates(sector, scale));
                if elapsed < surge_end {
                    self.cadence = rates[i].0;
                    self.budget = 1.0 + director.pressure_budget_bonus * scale;
                    self.contact *= 1.0 + director.pressure_contact_bonus * scale;
                } else {
                    self.cadence = rates[i].1;
                    self.budget = (1.0 - director.pressure_recovery_budget_cut * scale).max(0.0);
                    self.heal = (director.pressure_recovery_heal * scale).min(1.0);
                }
                break;
            }
        }
    }

    pub(crate) fn live_budget(&self, base: usize) -> usize {
        if self.budget == 1.0 {
            base
        } else {
            (base as f32 * self.budget).round() as usize
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
    raise_boss_due_at(director, due, elapsed, level, BOSS_TIME)
}

pub fn raise_boss_due_at(
    director: &mut Director, due: &mut BossDue, elapsed: f32, level: usize, boss_time: f32,
) -> bool {
    if director.boss_spawned || elapsed < boss_time {
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
    hp_mult_for(level, elapsed) * BOSS_POOL_SCALE * BOSS_BUILD_LEVEL[level_slot(level)]
}

/// How tough enemies are right now (HP multiplier) on the curve clock.
///
/// §1: "Keep ordinary HP scaling shallow (×1 at start to ×2 at 15:00)." The
/// curve is linear over the fifteen minutes, so a skitterer that took one hit
/// at 0:00 takes two at the boss. Later campaign stages add their own
/// [`LEVEL_HP_EXTRA`] on top after the grace window.
///
/// On top of that sits [`level_bonus_hp_at`]: the rat's character level now
/// adds weapon damage, and the pools grow by the bonus a typical rat carries
/// at that minute, so a body still takes the hits it took before. And on top
/// of that, [`build_hp_at`]: batch twelve's builds hold a quarter of the cards,
/// and the pools are cut to match.
#[inline]
pub fn hp_mult_at(curve: f32) -> f32 {
    (1.0 + curve / RUN_SECONDS) * level_bonus_hp_at(curve) * build_hp_at(curve)
}

/// The weapon-damage bonus a typical rat's character level is worth on the
/// curve clock (`player::level_damage_mult` at the measured mean level: 5 at
/// 1:00, 13 at 2:00, 20 at 3:00, 35 at 6:00, 45 at 9:00, 52 at 12:00, 56 at
/// the boss — 48 mixed-skill campaigns, batch eight).
///
/// **Batch twelve moves no knot here.** The character level now takes four
/// times the XP (`player::xp_for_level`) and `LEVEL_DAMAGE_PER_LEVEL` went
/// 0.01 → 0.04, so the same rat reads level 2 / 4 / 5.75 / 9.5 / 12 / 13.75 /
/// 14.75 at those minutes (the mapping is `(old + 3) / 4`) and
/// `1 + 0.04 × (level − 1)` is `1 + 0.01 × (old − 1)` to the digit:
/// ×1.04 / ×1.12 / ×1.19 / ×1.34 / ×1.44 / ×1.51 / ×1.55. The knots below are
/// therefore unchanged, and so is every pool sized against them
/// (`miniboss::appointment_hp` included).
///
/// Every pool that is sized against the rat's damage is multiplied by it (the
/// horde and the boss through [`hp_mult_at`], the named minibosses through
/// their own table), so the per-level damage reads as the rat getting
/// stronger without shortening a fight that was tuned by length.
pub(crate) const LEVEL_BONUS_HP: [(f32, f32); 8] = [
    (0.0, 1.0),
    (60.0, 1.04),
    (120.0, 1.12),
    (180.0, 1.19),
    (360.0, 1.34),
    (540.0, 1.44),
    (720.0, 1.51),
    (900.0, 1.55),
];

/// [`LEVEL_BONUS_HP`] at a curve time.
#[inline]
pub fn level_bonus_hp_at(curve: f32) -> f32 {
    lerp_table(&LEVEL_BONUS_HP, curve)
}

/// How much smaller a batch-twelve build's damage is than the build every
/// pool was sized against, on the curve clock.
///
/// Level-ups come a quarter as often (about 13 cards by the boss instead of
/// 55), so a rat reaches 6:00 with three weapons at low rank and two passive
/// stacks where it used to have a full maxed rack, and the starting Gnaw is a
/// bite, not a homing dart. The per-level damage bonus is unchanged by
/// construction ([`LEVEL_BONUS_HP`]); this table is the rest of the gap. It
/// scales every pool that is sized against the rat's damage: the horde and
/// the boss through [`hp_mult_at`], and `miniboss::appointment_hp` carries
/// its own table of the same correction.
///
/// Measured over four 48-campaign mixed-skill batches (seed 1000): with these
/// knots the kill counts on levels two to four match batch eight's to the
/// minute (320 / 980 / 1970 / 3260 at 3 / 6 / 9 / 12 min) and the character
/// level reads 5.5 / 9.0 / 11.8 / 13.2 there. Level one lags (178 kills at
/// 3:00 against 332): from 5:00 on its ordinary bodies are already one or two
/// bites each, so a lower pool no longer buys anything there; what is left is
/// the bite's reach and the missing defensive stacks, which are not hp.
pub(crate) const BUILD_HP: [(f32, f32); 7] = [
    (0.0, 0.60),
    (60.0, 0.45),
    (180.0, 0.30),
    (300.0, 0.20),
    (480.0, 0.16),
    (660.0, 0.20),
    (900.0, 0.28),
];

/// The carried build's share of that gap, per campaign level: a rat arrives
/// on levels two to four with its kept upgrades already ranked, so the pools
/// there are larger than level one's (which starts from the bare Gnaw). A
/// practice run plays level one's row (`DifficultyLevel`). Rising, so the
/// Flatbed finale is the hardest level.
pub const BUILD_HP_LEVEL: [f32; 4] = [0.8, 2.2, 2.4, 2.6];

/// The boss pool's own correction per campaign level, on top of
/// [`BUILD_HP_LEVEL`]: measured median fights 66 / 78 / 72 / 66 s against the
/// 50-80 s target.
pub const BOSS_BUILD_LEVEL: [f32; 4] = [0.75, 0.78, 0.57, 0.68];

/// [`BUILD_HP`] at a curve time.
#[inline]
pub fn build_hp_at(curve: f32) -> f32 {
    lerp_table(&BUILD_HP, curve)
}

/// [`hp_mult_at`] for a campaign level and the real run clock.
#[inline]
pub fn hp_mult_for(level: usize, elapsed: f32) -> f32 {
    hp_mult_at(curve_clock(level, elapsed)) * BUILD_HP_LEVEL[level_slot(level)]
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
pub(crate) const BODIES_PER_SECOND: [(f32, f32); 7] = [
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
pub(crate) const LIVE_BUDGET: [(f32, usize); 7] = [
    (0.0, 18),
    (60.0, 42),
    (180.0, 72),
    (360.0, 115),
    (540.0, 150),
    (720.0, 185),
    (900.0, 185),
];

/// Elite chance per ordinary spawn by curve clock (the §1 column), as a
/// fraction. Linear within a row.
pub(crate) const ELITE_CHANCE: [(f32, f32); 7] = [
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
    let base = base_spawn_interval_at(curve);
    if curve < OPENING_SECS {
        base * OPENING_INTERVAL_MULT
    } else {
        base
    }
}

pub(crate) fn base_spawn_interval_at(curve: f32) -> f32 {
    (1.6 - curve / 900.0).max(0.35)
}

/// The opening's quicker pulse: for the first [`OPENING_SECS`] of the curve the
/// pulse interval is multiplied by [`OPENING_INTERVAL_MULT`].
///
/// Spawns land beyond the widest zoom's view now, so the first bodies die (and
/// drop their XP) further from the rat than they used to and the first card
/// came about three seconds later. Ten bodies instead of seven in the first
/// twelve seconds give those seconds back. A pulse in this window is still one
/// body (`rate × interval` stays under 1.5), so nothing but the cadence moves.
pub const OPENING_SECS: f32 = 12.0;
/// See [`OPENING_SECS`].
pub const OPENING_INTERVAL_MULT: f32 = 0.7;

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

/// When a kind first joins a floor's roll, on the curve clock: its earliest
/// slot unlock over every floor that names it ([`crate::game::roster`]). `None`
/// for a kind no floor fields (the level boss).
///
/// The HUD's escalation callouts read this, so "bats inbound" still lands on the
/// second the first bat can spawn (0:45 in the Sewer, and every other floor's
/// flying fodder slot is the same 0:45).
#[inline]
pub fn base_unlock_time(kind: EnemyKind) -> Option<f32> {
    roster::first_unlock(kind)
}

// ---- The kinds come from the floor's roster -------------------------------
//
// `WAVE4_ROSTERS.md`: eight ordinary kinds and four named appointments per
// floor, looked up by `Theme` in `crate::game::roster`. The director used to
// own a global unlock table, a per-campaign-level mix and two special-kind
// tiers here; all of that is one floor table now, so a per-floor content pass
// never touches this file.

/// Converts a design (fifteen-minute) time into the current build's clock.
///
/// The roster's unlock times and the featured rotation are written on the
/// design's own clock, and [`curve_clock`] is that clock while `RUN_SECONDS` is
/// the design's fifteen minutes. Kept because it is the one expression that has
/// to stay right as the timeline grows.
pub const WAVE3_TIME_SCALE: f32 = RUN_SECONDS / DESIGN_RUN_SECONDS;

/// The run length the design's own times are written for (fifteen minutes).
pub const DESIGN_RUN_SECONDS: f32 = 900.0;

/// How many big snakes may be alive at once. Kept as the public name for the
/// number; the roster enforces it through [`roster::kind_cap`], which reads
/// `enemy_wave4::SNAKE_LIVE_CAP` itself.
pub const WAVE4_SNAKE_LIVE_CAP: usize =
    crate::game::ecs::systems::enemy_wave4::SNAKE_LIVE_CAP;

/// How many lilliputian bodies may be alive at once (two squads). A roll that
/// would start a third squad is substituted with fodder instead.
pub const WAVE4_LILLI_LIVE_CAP: usize =
    crate::game::ecs::systems::enemy_wave4::LILLI_LIVE_CAP;

/// The floor's weighted roll at curve time `curve`, with the live caps the
/// director counted this pulse. This is the whole kind-selection surface.
#[inline]
pub fn pick_kind_for_theme(
    theme: Theme,
    curve: f32,
    caps: &SlotCaps,
    rng: &mut impl Rng,
) -> EnemyKind {
    roster::pick_kind(theme, curve, caps, rng)
}

/// The Sewer's roll at curve time `curve` with nothing capped: the historical
/// entry point for tests and tuning (the Sewer is the reference floor).
pub fn pick_kind_at(curve: f32, rng: &mut impl Rng) -> EnemyKind {
    roster::pick_kind(Theme::Sewer, curve, &SlotCaps::none(), rng)
}

/// [`pick_kind_at`] for a campaign level and the real run clock.
pub fn pick_kind_for(level: usize, elapsed: f32, rng: &mut impl Rng) -> EnemyKind {
    pick_kind_at(curve_clock(level, elapsed), rng)
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
    // The ceiling matters as much as the floor: the timer is reset once, at
    // spawn time, with the interval of *that* instant. The first elite lands
    // around 3:20, where the chance is ~0.1 % and the raw interval ~280 s, so
    // without a cap the floor saw no second elite until ~8:00 (the chance
    // column implies about ten in that gap) and then one every 6-10 s.
    (1.0 / (chance * bodies_per_second_at(curve).max(0.1))).clamp(6.0, ELITE_INTERVAL_MAX)
}

/// The longest the elite timer may be reset to once elites are live (see
/// [`elite_interval_at`]): a rising chance is re-read at least this often.
pub const ELITE_INTERVAL_MAX: f32 = 45.0;

/// The horde's damage multiplier at the top of a level's ramp (its boss time).
///
/// **Wave-3 re-base.** The old 300 s curves reached ×4.1/×4.3, which the owner
/// measured as near-lethal on a 15-minute level. The shallow ramp keeps the
/// later stages readable: contact damage is the base kind's, plus a small
/// per-stage ramp. The horde's bite lives in [`LEVEL_ELITE_INTERVAL`] and
/// the floor's roster (more visible heavies, a real ranged line) instead.
///
/// **Batch seven**: ×1.12 / ×1.20 did not register on a build with a few
/// armour stacks (a flat point off every hit). Stages two and three reach
/// ×1.6 / ×1.9 at the boss; stage one stays at base, as the design asks.
/// **Batch twelve** adds the ride: ×2.1, one step further up the same ramp.
pub(crate) const LEVEL_DAMAGE_TOP: [f32; 4] = [1.0, 1.6, 1.9, 2.1];

/// The horde's speed multiplier at the top of a level's ramp. The Lab's swarm
/// stays the quickest of the facility floors, but only just: a faster horde
/// walks into melee-range weapons on the merged build, so the measured cliff
/// lives well above this. The ride's row (×1.2) is the fourth stage's step.
pub(crate) const LEVEL_SPEED_TOP: [f32; 4] = [1.0, 1.08, 1.15, 1.2];

/// Shape of the ramp: `t^exp` over the level, so a lower exponent bites sooner.
const LEVEL_RAMP_EXP: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// How a campaign level's horde differs from the base kinds right now.
///
/// Both multipliers ramp linearly from 1.0 at the level's own start to
/// [`LEVEL_DAMAGE_TOP`]/[`LEVEL_SPEED_TOP`] at its [`BOSS_TIME`]. Level 1 is
/// exactly `EnemyMods::NONE`.
pub fn enemy_mods_for(level: usize, elapsed: f32) -> crate::game::enemy::EnemyMods {
    use crate::game::enemy::EnemyMods;
    let slot = level_slot(level);
    let t = (elapsed / BOSS_TIME).clamp(0.0, 1.0).dpowf(LEVEL_RAMP_EXP[slot]);
    EnemyMods {
        damage: 1.0 + (LEVEL_DAMAGE_TOP[slot] - 1.0) * t,
        speed: 1.0 + (LEVEL_SPEED_TOP[slot] - 1.0) * t,
    }
}

impl crate::tuning::Tuning {
    fn fresh_sector_slot(&self, level: usize) -> Option<usize> {
        level_slot(level).checked_sub(1).filter(|_| !self.campaign.carry)
    }

    pub(crate) fn pressure_scale_for(&self, level: usize) -> f32 {
        match self.fresh_sector_slot(level) {
            Some(i) => self.director.fresh_pressure_sector_scale[i],
            None => self.director.pressure_sector_scale[level_slot(level)],
        }
    }

    pub fn horde_hp_mult_for(&self, level: usize, elapsed: f32) -> f32 {
        let slot = level_slot(level);
        let d = &self.director;
        let (build, extra) = match self.fresh_sector_slot(level) {
            Some(i) => (d.fresh_build_hp_level[i], d.fresh_level_hp_extra[i]),
            None => (d.build_hp_level[slot], d.level_hp_extra[slot]),
        };
        d.horde_hp_mult_for(level, elapsed, build, extra)
    }

    pub fn boss_hp_mult_for(&self, level: usize) -> f32 {
        let slot = level_slot(level);
        let d = &self.director;
        let (build, boss) = match self.fresh_sector_slot(level) {
            Some(i) => (d.fresh_build_hp_level[i], self.bosses.fresh_build_level[i]),
            None => (d.build_hp_level[slot], self.bosses.build_level[slot]),
        };
        d.hp_mult_for(level, d.boss_time, build) * BOSS_POOL_SCALE * boss
    }
}

impl crate::tuning::Director {
    pub fn curve_clock(&self, level: usize, elapsed: f32) -> f32 {
        elapsed.min(self.boss_time) * self.curve_speed + LEVEL_CLOCK_OFFSET[level_slot(level)]
    }

    pub fn horde_speed_mult(&self, elapsed: f32) -> f32 {
        elapsed.min(self.horde_speed_ramp_end)
            / (self.horde_speed_ramp_end / (self.horde_speed_top - 1.0)) + 1.0
    }

    fn hp_mult_for(&self, level: usize, elapsed: f32, build_mult: f32) -> f32 {
        let curve = self.curve_clock(level, elapsed);
        (1.0 + curve / RUN_SECONDS)
            * lerp_table(&self.level_bonus_hp, curve)
            * lerp_table(&self.build_hp, curve)
            * build_mult
    }

    fn horde_hp_mult_for(&self, level: usize, elapsed: f32, build_mult: f32, top: f32) -> f32 {
        let grace = self.level_hp_extra_after;
        let span = self.boss_time - grace;
        let progress = if span > 0.0 {
            ((elapsed.min(self.boss_time) - grace) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.hp_mult_for(level, elapsed, build_mult) * (1.0 + (top.max(1.0) - 1.0) * progress)
    }

    pub fn enemy_mods_for(&self, level: usize, elapsed: f32) -> crate::game::enemy::EnemyMods {
        let slot = level_slot(level);
        let t = (elapsed / self.boss_time).clamp(0.0, 1.0).dpowf(LEVEL_RAMP_EXP[slot]);
        crate::game::enemy::EnemyMods {
            damage: 1.0 + (self.level_damage_top[slot] - 1.0) * t,
            speed: 1.0 + (self.level_speed_top[slot] - 1.0) * t,
        }
    }

    fn sector(&self, level: usize) -> Option<&crate::tuning::DirectorSector> {
        self.sectors.get(crate::tuning::SECTOR_KEYS[level_slot(level)])
    }

    fn body_rate_at(&self, level: usize, curve: f32) -> f32 {
        let knots = self.sector(level)
            .and_then(|row| row.bodies_per_second.as_ref())
            .unwrap_or(&self.bodies_per_second);
        lerp_table(knots, curve)
    }

    fn opening_interval(&self, base: f32, curve: f32) -> f32 {
        if curve < self.opening_secs { base * self.opening_interval_mult } else { base }
    }

    pub fn horde_interval(&self, level: usize, elapsed: f32, boss_phase: bool) -> f32 {
        if boss_phase {
            self.cleanup_interval
        } else {
            let curve = self.curve_clock(level, elapsed);
            let base = self.sector(level)
                .and_then(|row| row.spawn_interval.as_ref())
                .map_or_else(|| base_spawn_interval_at(curve), |knots| lerp_table(knots, curve));
            self.opening_interval(base, curve) * self.spawn_interval_mult
        }
    }

    pub fn horde_pulse(&self, level: usize, elapsed: f32, boss_phase: bool) -> usize {
        if boss_phase {
            self.cleanup_pulse
        } else {
            let curve = self.curve_clock(level, elapsed);
            // Cadence changes leave the pulse size alone, like spawn_interval_mult.
            let quantum = self.opening_interval(base_spawn_interval_at(curve), curve);
            let n = (self.body_rate_at(level, curve) * quantum).round();
            (n as usize).max(1)
        }
    }

    pub fn horde_live_budget(&self, level: usize, elapsed: f32, boss_phase: bool) -> usize {
        if boss_phase {
            return self.cleanup_live_budget;
        }
        let curve = self.curve_clock(level, elapsed);
        let steps = self.sector(level)
            .and_then(|row| row.live_budget.as_ref())
            .unwrap_or(&self.live_budget);
        let mut budget = steps[0].1;
        for &(time, count) in steps {
            if curve >= time {
                budget = count;
            }
        }
        budget
    }

    /// Integrate the nominal offer over each window using the same pulse size
    /// and interval as the spawn system. Fixed midpoint samples bound the work
    /// for arbitrary tuning tables. Recovery always keeps at least 20% of its
    /// usual cadence, so short recoveries bound the possible surge.
    fn pressure_rates(&self, sector: usize, scale: f32) -> [(f32, f32); 2] {
        const STEPS: usize = 128;
        let offered = |start: f32, duration: f32| {
            let step = duration / STEPS as f32;
            let mut total = 0.0;
            for i in 0..STEPS {
                let t = start + (i as f32 + 0.5) * step;
                total += self.horde_pulse(sector, t, false) as f32 / self.horde_interval(sector, t, false);
            }
            total * step
        };
        std::array::from_fn(|i| {
            let start = self.pressure_starts[i];
            let surge_len = self.pressure_lengths[i];
            let surge = offered(start, surge_len);
            let recovery = offered(start + surge_len, self.pressure_recovery_lengths[i]);
            let moved = (surge * self.pressure_rate_bonus * scale).min(recovery * 0.8);
            (1.0 + moved / surge, 1.0 - moved / recovery)
        })
    }

    pub fn elite_start_for(&self, level: usize) -> f32 {
        self.sector(level).and_then(|row| row.elite_start).unwrap_or(self.elite_start)
    }

    pub fn elite_interval_at(&self, level: usize, curve: f32) -> f32 {
        let row = self.sector(level);
        let knots = row.and_then(|row| row.elite_chance.as_ref()).unwrap_or(&self.elite_chance);
        let chance = lerp_table(knots, curve);
        let interval = if chance <= 0.0 {
            60.0
        } else {
            (1.0 / (chance * self.body_rate_at(level, curve).max(0.1)))
                .clamp(6.0, ELITE_INTERVAL_MAX)
        };
        interval * row.and_then(|row| row.elite_interval_mult).unwrap_or(self.elite_interval_mult)
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

    #[test]
    fn director_without_sector_overrides_matches_the_shared_curves() {
        let mut tuning = crate::tuning::Director::default();
        tuning.sectors.clear();
        for level in 0..4 {
            for elapsed in [0.0, 0.1, 11.99, 12.0, 180.0, 255.5, 600.0, BOSS_TIME, BOSS_TIME + 5.0] {
                for boss_phase in [false, true] {
                    assert_eq!(tuning.horde_interval(level, elapsed, boss_phase).to_bits(),
                        horde_interval(elapsed, boss_phase).to_bits());
                    assert_eq!(tuning.horde_pulse(level, elapsed, boss_phase),
                        horde_pulse(elapsed, boss_phase));
                    assert_eq!(tuning.horde_live_budget(level, elapsed, boss_phase),
                        horde_live_budget(elapsed, boss_phase));
                }
                let curve = curve_clock(level, elapsed);
                assert_eq!(tuning.elite_start_for(level), ELITE_START);
                assert_eq!(tuning.elite_interval_at(level, curve).to_bits(),
                    elite_interval_at(curve).to_bits());
            }
        }
    }

    #[test]
    fn tuning_overrides_only_the_selected_director_sector() {
        let tuned = crate::tuning::Tuning::parse(r#"
            [director.sectors.1]
            live_budget = [[0, 5], [240, 12], [270, 15], [400, 20], [500, 25], [600, 30], [900, 40]]
            bodies_per_second = [[0, 8.0], [240, 8.0], [270, 8.0], [400, 8.0], [500, 8.0], [600, 8.0], [900, 8.0]]
            spawn_interval = [[0, 1.6], [240, 1.6], [270, 2.6], [400, 2.6], [500, 2.6], [600, 2.6], [900, 2.6]]
            elite_start = 240.0
            elite_chance = [[0, 0.25], [240, 0.25], [270, 0.25], [400, 0.25], [500, 0.25], [600, 0.25], [900, 0.25]]
            elite_interval_mult = 2.0
        "#).unwrap().director;
        let at = 255.0;
        assert!((tuned.horde_interval(0, at, false) - 2.1).abs() < 1e-6);
        assert_eq!(tuned.horde_pulse(0, at, false), 11);
        assert_eq!(tuned.horde_live_budget(0, at, false), 12);
        assert_eq!(tuned.horde_live_budget(0, 270.0, false), 15);
        assert_eq!(tuned.elite_start_for(0), 240.0);
        assert_eq!(tuned.elite_interval_at(0, at), 12.0);
        assert_eq!(tuned.horde_interval(0, at, true), CLEANUP_INTERVAL);
        assert_eq!(tuned.horde_pulse(0, at, true), CLEANUP_PULSE);
        assert_eq!(tuned.horde_live_budget(0, at, true), CLEANUP_LIVE_BUDGET);

        let shipped = crate::tuning::Director::default();
        for level in 1..4 {
            assert_eq!(tuned.horde_interval(level, at, false), shipped.horde_interval(level, at, false));
            assert_eq!(tuned.horde_pulse(level, at, false), shipped.horde_pulse(level, at, false));
            assert_eq!(tuned.horde_live_budget(level, at, false), shipped.horde_live_budget(level, at, false));
            assert_eq!(tuned.elite_start_for(level), shipped.elite_start_for(level));
            assert_eq!(tuned.elite_interval_at(level, at), shipped.elite_interval_at(level, at));
        }
    }

    /// The elite timer is reset once per elite, so a huge interval read while
    /// the chance is still tiny must not silence elites for minutes.
    #[test]
    fn the_elite_interval_is_bounded_once_elites_are_live() {
        let mut t = ELITE_START + 1.0;
        while t <= BOSS_TIME {
            let gap = elite_interval_at(t);
            assert!(
                (6.0..=ELITE_INTERVAL_MAX).contains(&gap),
                "elite interval {gap} s at {t} s is outside 6..={ELITE_INTERVAL_MAX}"
            );
            t += 1.0;
        }
        // Late in the floor the design's rate still wins over the ceiling.
        assert!(elite_interval_at(BOSS_TIME) < 10.0);
    }
    use crate::game::campaign::LEVEL_COUNT;

    #[test]
    fn pick_kind_is_deterministic_and_time_gated() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        // Same seed + clock ⇒ same picks. The roll consumes exactly one rng
        // value and walks the floor's eight slots in the design's stable order
        // (A, B, C, I, R, II, H, III), so a locked or capped slot cannot shift
        // the stream.
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
        // The Sewer's heavy is the Sluice Toad now: a Brute by archetype.
        assert!(seen.contains(&EnemyKind::Bat));
        assert!(seen.iter().any(|k| k.archetype() == EnemyKind::Brute));
        assert!(seen.contains(&EnemyKind::Skitterer));
    }

    #[test]
    fn difficulty_escalates() {
        // The design's own curve rises; the pool a body actually gets also
        // carries the batch-twelve build correction (`BUILD_HP`), which falls
        // through the opening minutes.
        assert!(hp_mult(120.0) / build_hp_at(120.0) > hp_mult(0.0) / build_hp_at(0.0));
        // Measured from the end of the opening's quick pulse (`OPENING_SECS`):
        // the first twelve seconds are deliberately faster than what follows.
        assert!(spawn_interval(200.0) < spawn_interval(OPENING_SECS));
        assert!(pulse_count(200.0) > pulse_count(0.0));
    }

    /// The opening's quick pulse: one body at a time, about ten of them in the
    /// first twelve seconds instead of seven, and the curve's own interval the
    /// moment the window closes.
    #[test]
    fn the_opening_pulse_is_quicker_and_still_one_body() {
        let mut t = 0.0f32;
        let mut bodies = 0;
        while t < OPENING_SECS {
            assert_eq!(pulse_count(t), 1, "an opening pulse at {t}s is not one body");
            bodies += 1;
            t += spawn_interval(t);
        }
        assert!((10..=11).contains(&bodies), "{bodies} bodies in the opening window");
        let raw = 1.6 - OPENING_SECS / 900.0;
        assert!((spawn_interval(OPENING_SECS) - raw).abs() < 1e-6);
        assert!((spawn_interval(0.0) - 1.6 * OPENING_INTERVAL_MULT).abs() < 1e-6);
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
        // (Against the curve's own opening pace, right after the first quick
        // seconds of `OPENING_SECS`.)
        assert!(
            spawn_rate(300.0) > 5.0 * spawn_rate(OPENING_SECS),
            "late rate {} is not an order above the opening {}",
            spawn_rate(300.0),
            spawn_rate(OPENING_SECS)
        );
        // And the early game is not a wall of bugs: the first seconds stay slow.
        assert!(spawn_rate(0.0) < 3.0, "early rate {}", spawn_rate(0.0));
    }

    /// The §1 table's boundary values, re-expressed over fifteen minutes.
    #[test]
    fn the_curve_boundaries_match_the_design_table() {
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
        for (t, v) in [(0.0, 18), (60.0, 42), (180.0, 72), (360.0, 115), (540.0, 150), (720.0, 185)] {
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
        // (Times the batch-twelve build correction, `BUILD_HP`: 0.60 at the
        // start, 0.28 at the boss.)
        assert!((hp_mult_at(0.0) - build_hp_at(0.0)).abs() < 1e-6);
        assert!((build_hp_at(0.0) - 0.60).abs() < 1e-6);
        assert!((build_hp_at(RUN_SECONDS) - 0.28).abs() < 1e-6);
        // ×2 by the boss on the design's curve, times the level bonus a
        // typical rat carries there (×1.55 at level 14.75).
        assert!((level_bonus_hp_at(0.0) - 1.0).abs() < 1e-6);
        assert!((level_bonus_hp_at(RUN_SECONDS) - 1.55).abs() < 1e-6);
        assert!((hp_mult_at(RUN_SECONDS) - 2.0 * 1.55 * build_hp_at(RUN_SECONDS)).abs() < 1e-5);
        let mut last = 0.0;
        for t in 0..=RUN_SECONDS as u32 {
            let b = level_bonus_hp_at(t as f32);
            assert!(b >= last, "the level bonus falls at {t}s");
            last = b;
        }
        // The base kinds' unlock times are the roster's slot unlocks now
        // (`WAVE4_ROSTERS.md` "Reading a room"): B 0:45, C 1:15, R 3:35,
        // H 6:35. The old global table had exactly these numbers, so the HUD's
        // escalation callouts did not move.
        assert_eq!(base_unlock_time(EnemyKind::Skitterer), Some(0.0));
        assert_eq!(base_unlock_time(EnemyKind::Bat), Some(45.0));
        assert_eq!(base_unlock_time(EnemyKind::Cat), Some(75.0));
        assert_eq!(base_unlock_time(EnemyKind::Spitter), Some(215.0));
        assert_eq!(base_unlock_time(EnemyKind::Brute), Some(395.0));
        assert_eq!(base_unlock_time(EnemyKind::Boss), None);
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
        // (Each level also carries its own build scale, `BUILD_HP_LEVEL`.)
        assert!(
            (hp_mult_for(2, 0.0) / BUILD_HP_LEVEL[2]
                - hp_mult_for(0, LEVEL_CLOCK_OFFSET[2]) / BUILD_HP_LEVEL[0])
                .abs()
                < 1e-4
        );
        assert_eq!(
            pulse_count_for(2, 0.0),
            pulse_count_for(0, LEVEL_CLOCK_OFFSET[2])
        );
        // And out-of-range indices clamp instead of panicking.
        for level in LEVEL_COUNT..LEVEL_COUNT + 4 {
            assert_eq!(curve_clock(level, 10.0), curve_clock(LEVEL_COUNT - 1, 10.0));
        }
    }

    /// The campaign's **fourth** level — the Flatbed ride — has its own row in
    /// every per-level table, and each row continues its own progression by one
    /// step (hp 1.9, damage 2.1, speed 1.2, elites every 16 s).
    ///
    /// It must be strictly harder than stage three at every instant past the
    /// hp grace window, and it must not move a single number for levels 0–2
    /// (the determinism goldens are 60 s floors on those levels).
    #[test]
    fn the_fourth_level_is_the_ride_and_harder_than_stage_three() {
        use crate::game::campaign::{getaway_theme, theme_sector, LEVEL_COUNT};
        assert_eq!(LEVEL_COUNT, 4);
        assert_eq!(theme_sector(getaway_theme()), 3);
        // One row per campaign level in every table.
        assert_eq!(LEVEL_HP_EXTRA.len(), LEVEL_COUNT);
        assert_eq!(LEVEL_DAMAGE_TOP.len(), LEVEL_COUNT);
        assert_eq!(LEVEL_SPEED_TOP.len(), LEVEL_COUNT);
        assert_eq!(LEVEL_ELITE_INTERVAL.len(), LEVEL_COUNT);
        assert_eq!(LEVEL_RAMP_EXP.len(), LEVEL_COUNT);
        assert_eq!(LEVEL_CLOCK_OFFSET.len(), LEVEL_COUNT);
        // The rows themselves: each table keeps its own progression going.
        assert_eq!(LEVEL_HP_EXTRA[3], 1.9);
        assert_eq!(LEVEL_DAMAGE_TOP[3], 2.1);
        assert_eq!(LEVEL_SPEED_TOP[3], 1.2);
        const { assert!(LEVEL_ELITE_INTERVAL[3] < LEVEL_ELITE_INTERVAL[2]) };
        assert_eq!(LEVEL_CLOCK_OFFSET[3], 0.0, "the ride opens with the grace");
        // Strictly harder than the Printworks/Lab at every real clock second
        // past the grace, and never softer in damage or speed at any instant.
        for t in [0.0f32, 30.0, 180.0, 181.0, 480.0, BOSS_TIME, BOSS_TIME + 60.0] {
            if t > LEVEL_HP_EXTRA_AFTER {
                assert!(
                    horde_hp_mult_for(3, t) > horde_hp_mult_for(2, t),
                    "the ride's horde hp is not above stage three at {t}s"
                );
            }
            let ride = enemy_mods_for(3, t);
            let stage3 = enemy_mods_for(2, t);
            assert!(ride.damage >= stage3.damage && ride.speed >= stage3.speed);
            assert!(ride.damage >= 1.0 && ride.speed >= 1.0);
        }
        // The boss pool follows the ride's clock, never the horde's extra.
        assert_eq!(
            boss_hp_mult_for(3, BOSS_TIME),
            hp_mult_for(3, BOSS_TIME) * BOSS_POOL_SCALE * BOSS_BUILD_LEVEL[3]
        );
        assert_eq!(level_hp_extra(3, BOSS_TIME), LEVEL_HP_EXTRA[3]);
        assert_eq!(elite_interval_for(3), LEVEL_ELITE_INTERVAL[3]);
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

    /// Level 0's damage, speed and elite cadence are the base game, bit for
    /// bit (the design: "speed and contact damage at base on stage one"). Its
    /// ordinary hp is the base curve through the opening, then carries the
    /// batch-seven horde extra from 3:00 ([`LEVEL_HP_EXTRA`], +20 % by the
    /// boss), which the boss pool does not. The *kinds* are the floor's
    /// roster, which is a separate axis (see `each_floor_fields_its_own_cast`).
    #[test]
    fn level_one_is_the_base_game() {
        use crate::game::enemy::EnemyMods;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        for t in [0.0f32, 30.0, 120.0, 200.0, BOSS_TIME, 999.0] {
            assert_eq!(enemy_mods_for(0, t), EnemyMods::NONE);
            let extra = level_hp_extra(0, t);
            assert_eq!(horde_hp_mult_for(0, t), hp_mult_for(0, t) * extra);
            if t <= LEVEL_HP_EXTRA_AFTER {
                assert_eq!(extra, 1.0, "the opening is the base curve at {t}s");
            } else {
                assert!(extra > 1.0 && extra <= LEVEL_HP_EXTRA[0], "{extra} at {t}s");
            }
            // The boss pool never carries the horde's extra.
            assert_eq!(
                boss_hp_mult_for(0, t),
                hp_mult_for(0, t) * BOSS_POOL_SCALE * BOSS_BUILD_LEVEL[0]
            );
        }
        assert_eq!(level_hp_extra(0, BOSS_TIME), LEVEL_HP_EXTRA[0]);
        assert_eq!(elite_interval_for(0), ELITE_INTERVAL);
        // The level-0 roll is the Sewer's roster roll, with or without a
        // campaign level in front of it (`pick_kind_for` only shifts the
        // clock).
        let mut a = StdRng::seed_from_u64(17);
        let mut b = StdRng::seed_from_u64(17);
        for t in [0.0f32, 60.0, 150.0, 250.0] {
            for _ in 0..200 {
                assert_eq!(pick_kind_for(0, t, &mut a), pick_kind(t, &mut b));
                assert_eq!(
                    pick_kind_for(0, t, &mut a),
                    pick_kind_at(curve_clock(0, t), &mut b)
                );
            }
        }
        // A practice run (`difficulty_override = Some(0)`) is level 0's scalars
        // on whichever floor it is played on. The level index never selects a
        // kind any more, so there is nothing for the override to disagree with.
        assert_eq!(enemy_mods_for(0, 42.0), EnemyMods::NONE);
        assert_eq!(
            hp_mult_for(0, 42.0),
            hp_mult_at(curve_clock(0, 42.0)) * BUILD_HP_LEVEL[0]
        );
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
                (pool - hp_mult_for(level, BOSS_TIME) * BOSS_POOL_SCALE * BOSS_BUILD_LEVEL[level_slot(level)])
                    .abs()
                    < 1e-4,
                "level {level}: boss pool {pool} is not the curve times the scale"
            );
            assert!(
                (horde - hp_mult_for(level, BOSS_TIME) * level_hp_extra_at(BOSS_TIME, level)).abs()
                    < 1e-4,
                "level {level}: horde hp {horde} is not the curve times its extra"
            );
        }
    }

    /// Each floor fields its own cast: the roster says which eight kinds a
    /// floor can spawn at all, and the director's picker never returns anything
    /// outside that set.
    ///
    /// This replaces `later_levels_field_a_different_mix`, which pinned the old
    /// global `LEVEL_MIX` weighting (Kitchen = more Spitters, Lab = more Bats).
    /// The mix is now per floor by construction; the numbers it used to weigh
    /// are the roster's pool shares, which are the same on every floor.
    #[test]
    fn each_floor_fields_its_own_cast() {
        use crate::game::level::Theme;
        use crate::game::roster;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        // Late enough that every slot on every floor is unlocked, and before
        // 15:00 (where the roster deliberately fields only A/B again).
        let curve = curve_clock(2, 700.0);
        for theme in Theme::playable() {
            let floor = roster::floor(theme);
            let allowed: Vec<EnemyKind> = floor.slots.iter().map(|s| s.kind).collect();
            let mut rng = StdRng::seed_from_u64(99);
            let mut seen: Vec<EnemyKind> = Vec::new();
            for _ in 0..4_000 {
                let k = pick_kind_for_theme(theme, curve, &roster::SlotCaps::none(), &mut rng);
                assert!(
                    allowed.contains(&k),
                    "{theme:?} spawned {k:?}, which is not on its roster"
                );
                if !seen.contains(&k) {
                    seen.push(k);
                }
            }
            assert_eq!(
                seen.len(),
                allowed.len(),
                "{theme:?} only fielded {seen:?}"
            );
            // Every one of the eight is in the draw, so a floor's own content
            // cannot be silently missing from play.
            let w = roster::masses(curve, &roster::SlotCaps::none());
            for slot in roster::Slot::ALL {
                assert!(
                    w[slot.index()] > 0.0,
                    "{theme:?}: {} has no share late in the run",
                    slot.label()
                );
            }
        }
        // The flats really are different casts, not one list relabelled.
        assert_ne!(
            roster::appointments(Theme::Sewer),
            roster::appointments(Theme::Kitchen)
        );
        assert_ne!(roster::kind(Theme::Sewer, roster::Slot::III), roster::kind(Theme::Lab, roster::Slot::III));
    }

    /// §1 over *simulated time*, on every floor: the special pool (I+II+III)
    /// never takes more than a fifth of the roll, the fodder pool keeps its
    /// minimum, and from 15:00 only the floor's own A/B spawn.
    ///
    /// This replaces `the_wave3_kinds_are_capped_at_a_fifth_of_spawns`, which
    /// measured the old global wave-3 tier. The numbers now come from the
    /// roster's clock table (`WAVE4_ROSTERS.md` "Director and campaign"), and
    /// the check runs through the real picker, draw for draw.
    #[test]
    fn the_special_pool_never_exceeds_a_fifth_on_any_floor() {
        use crate::game::level::Theme;
        use crate::game::roster;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        // One sample per design minute, the whole fifteen-minute build-up.
        for theme in Theme::playable() {
            let floor = roster::floor(theme);
            let specials: Vec<EnemyKind> = roster::Pool::Special
                .slots()
                .iter()
                .map(|s| floor.kind(*s))
                .collect();
            // 15:30 (minute 15) is the cleanup row, past the boss time.
            for minute in 0..16u64 {
                let clock = (minute as f32 + 0.5) * 60.0;
                let mut rng = StdRng::seed_from_u64(4242 + minute);
                let total = 12_000usize;
                let mut special = 0usize;
                let mut fodder = 0usize;
                for _ in 0..total {
                    let k = pick_kind_for_theme(theme, clock, &roster::SlotCaps::none(), &mut rng);
                    if specials.contains(&k) {
                        special += 1;
                    }
                    if k == floor.kind(roster::Slot::A) || k == floor.kind(roster::Slot::B) {
                        // A floor's fodder kinds are the only ones at 15:00+;
                        // earlier this is a lower bound, not an equality.
                        fodder += 1;
                    }
                }
                // Sampling tolerances: the exact masses are pinned in the
                // roster's own tests, so these only have to prove the *picker*
                // lands on them (a fixed seed, so a failure is reproducible).
                let special_share = special as f32 / total as f32;
                assert!(
                    special_share <= roster::SPECIAL_CEILING + 0.015,
                    "{theme:?} at {clock}s: the special pool took {special_share}"
                );
                // The ceiling is reached, not just never breached: with three
                // specials on the table from 9:45 the pool is a real presence.
                if (600.0..BOSS_TIME).contains(&clock) {
                    assert!(
                        special_share > roster::SPECIAL_CEILING * 0.5,
                        "{theme:?} at {clock}s: the special pool is too rare ({special_share})"
                    );
                }
                let fodder_share = fodder as f32 / total as f32;
                assert!(
                    fodder_share >= roster::fodder_minimum(clock) - 0.05,
                    "{theme:?} at {clock}s: fodder fell to {fodder_share}"
                );
                if clock >= BOSS_TIME {
                    // 15:00: "new spawns use only local A/B at cleanup pace".
                    assert!(
                        (fodder_share - 1.0).abs() < 1e-6,
                        "{theme:?}: the cleanup roll is not pure fodder ({fodder_share})"
                    );
                }
            }
        }
    }

    /// The 7:15 → 8:15 rehearsals and the 9:45 rotation reach the draw: the
    /// featured slot really is about twice as likely as its siblings, and the
    /// pool's own share does not grow.
    ///
    /// This replaces `the_featured_wave3_kind_rotates_and_doubles`.
    #[test]
    fn the_featured_slot_rehearses_and_then_rotates() {
        use crate::game::level::Theme;
        use crate::game::roster;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        // 8:15 to 9:45 doubles I; 9:45 starts the I → II → III rotation.
        assert_eq!(roster::featured(494.0), Some(roster::Slot::II));
        assert_eq!(roster::featured(495.0), Some(roster::Slot::I));
        assert_eq!(roster::featured(585.0), Some(roster::Slot::I));
        assert_eq!(roster::featured(615.0), Some(roster::Slot::II));
        assert_eq!(roster::featured(645.0), Some(roster::Slot::III));
        let theme = Theme::Sewer;
        let floor = roster::floor(theme);
        let share = |clock: f32, kind: EnemyKind| {
            let mut rng = StdRng::seed_from_u64(99);
            let total = 40_000usize;
            let hits = (0..total)
                .filter(|_| {
                    pick_kind_for_theme(theme, clock, &roster::SlotCaps::none(), &mut rng) == kind
                })
                .count();
            hits as f32 / total as f32
        };
        // During I's rehearsal, I outdraws II and III without breaking the cap.
        let i = floor.kind(roster::Slot::I);
        let ii = floor.kind(roster::Slot::II);
        let rehearsed = share(550.0, i);
        let plain = share(550.0, ii);
        assert!(
            rehearsed > plain * 1.6,
            "the featured slot did not double: {rehearsed} vs {plain}"
        );
        let iii = floor.kind(roster::Slot::III);
        let special = rehearsed + plain + share(550.0, iii);
        assert!(
            special <= roster::SPECIAL_CEILING + 0.012,
            "the featured window broke the ceiling: {special}"
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

    #[test]
    fn pressure_changes_only_the_surge_and_recovery_windows() {
        let tuning = crate::tuning::Tuning::parse("[director]\npressure_beats = true").unwrap();
        let director = &tuning.director;
        let mut pressure = PressureState::default();
        let start = director.pressure_starts[0];
        let end = start + director.pressure_lengths[0];
        let recovery_end = end + director.pressure_recovery_lengths[0];
        pressure.tick(&tuning, 1, Theme::Kitchen, start - 0.01, false);
        assert_eq!(pressure.live_budget(72), 72);
        assert_eq!(pressure.cadence, 1.0);
        pressure.tick(&tuning, 1, Theme::Kitchen, start, false);
        assert!(pressure.live_budget(72) > 72 && pressure.cadence > 1.0);
        assert!(pressure.contact > 1.0 && pressure.heal == 0.0);
        pressure.tick(&tuning, 1, Theme::Kitchen, end, false);
        assert!(pressure.live_budget(72) < 72 && pressure.cadence < 1.0);
        assert!(pressure.heal > 0.0 && pressure.contact == 1.0);
        for at in [recovery_end, director.boss_time] {
            pressure.tick(&tuning, 1, Theme::Kitchen, at, false);
            assert_eq!(pressure.live_budget(72), 72);
            assert_eq!(pressure.cadence, 1.0);
            assert_eq!(pressure.heal, 0.0);
        }
        pressure.tick(&tuning, 0, Theme::Sewer, 0.0, false);
        assert!(pressure.contact < 1.0);
        assert_eq!(pressure.cadence, 1.0, "preserve the first cards' XP feed");
        pressure.tick(&tuning, 0, Theme::Sewer, director.pressure_sewer_opening_secs, false);
        assert_eq!(pressure.contact, 1.0);
        pressure.tick(&tuning, 0, Theme::Sewer, start, true);
        assert_eq!(pressure.cadence, 1.0, "a manually opened boss fight keeps cleanup pace");
        assert_eq!(pressure.contact, 1.0);
        assert_eq!(pressure.live_budget(72), 72);
    }

    #[test]
    fn pressure_conserves_the_scheduled_body_supply() {
        let tuning = crate::tuning::Tuning::parse(
            "[director]\npressure_beats = true\npressure_rate_bonus = 0.85\n\
            [director.sectors.2]\nspawn_interval = [[0, 1.5], [60, 1.4], [180, 1.3], \
            [360, 1.2], [540, 1.0], [720, 0.9], [900, 0.8]]",
        ).unwrap();
        let director = &tuning.director;
        for sector in 0..4 {
            let mut pressure = PressureState::default();
            let mut baseline = 0;
            let mut tuned = 0;
            let mut baseline_timer = 0.0;
            let mut tuned_timer = 0.0;
            let dt = 1.0 / 60.0;
            for frame in 1..=54_000 {
                let elapsed = frame as f32 * dt;
                pressure.tick(&tuning, sector, Theme::Kitchen, elapsed, false);
                let interval = director.horde_interval(sector, elapsed, false);
                let pulse = director.horde_pulse(sector, elapsed, false);
                baseline_timer -= dt;
                if baseline_timer <= 0.0 {
                    baseline += pulse;
                    baseline_timer = interval;
                }
                tuned_timer -= dt * pressure.cadence;
                if tuned_timer <= 0.0 {
                    tuned += pulse;
                    tuned_timer = interval;
                }
            }
            assert!(baseline.abs_diff(tuned) as f32 <= baseline as f32 * 0.01,
                "sector {sector}: offered {tuned} bodies against a baseline of {baseline}");
        }
    }
}
