//! Headless batch runner: `ratrun --bot-batch <N> …`.
//!
//! Plays N seeded campaigns (or single practice levels) to completion at full
//! speed, spread over worker threads, with no terminal, no rendering and no
//! audio, then prints a report on beatability: win/clear rates, where runs
//! die, boss fight lengths and outcomes, damage taken by source, the build's
//! DPS at the boss, and how fast the batch ran.
//!
//! ```sh
//! cargo run --release -- --bot-batch 30 --seed 1000 --skill expert
//! cargo run --release -- --bot-batch 30 --skill casual --json > casual.json
//! cargo run --release -- --bot-batch 20 --level 3 --practice
//! ```
//!
//! ## Determinism
//!
//! Run `i` uses seed `base + i`, each run is independent, and the aggregate is
//! built in run order (never in completion order), so two identical batches
//! produce an identical report whatever `--threads` is. `tests::identical_batches_match`
//! is the regression.
//!
//! ## Reading a rate: Wilson intervals and sample size
//!
//! Every headline rate carries a **Wilson 95 % interval**
//! ([`wilson95`]), and the text report prints the sample size a tuning step
//! actually needs: resolving a 5-point difference between two rates near 50 %
//! takes ~1 500 runs per arm (95 % confidence, 80 % power, [`runs_to_resolve`]).
//! A 30-run batch has a ±14-18 point interval, so a 5-point move in it is
//! noise unless it moves the same direction on the same seeds in a paired
//! comparison.
//!
//! ## Finding defects: `--analyze` and `--cycle-skills`
//!
//! `--analyze` runs [`super::anomaly::AnomalyScan`] alongside every run and
//! lists each class it found with example seeds and the subsystem to read
//! first (silent player, silent weapons, a boss that takes no damage or never
//! moves, a rat stuck against geometry, an untouched enemy in weapon range, a
//! map the rat never leaves, levels/DPS/levels far over the batch median). The
//! scan only reads the world, so an analysed run is the same run
//! (`tests::the_anomaly_scan_does_not_change_a_run`).
//!
//! `--cycle-skills` rotates `Expert, Skilled, Casual` per run, so one 200-run
//! batch samples the whole ladder: the right shape for a defect hunt (the
//! headline rates per skill still come from `--skill` batches).

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::anomaly::{self, Anomaly, AnomalyAgg, AnomalyKind, AnomalyScan};
use super::{
    run_to_end, AutoOpts, AutoRun, CarryPreset, DeathInfo, LevelStats, Progress, PROGRESS_TIMES,
};
use crate::game::bot::Skill;
use crate::game::campaign;
use crate::game::weapon::WeaponKind;

/// The simulation step the batch uses. Fixed, so a run is reproducible.
pub const DT: f32 = 1.0 / 60.0;
/// Default cap on simulated time per run (seconds).
///
/// A campaign is **three fifteen-minute levels** (3 × 900 s) plus three boss
/// fights, so the old 1500 s cap cut every full campaign off in the middle of
/// its second level: the batch could never report a campaign clear at all. 3600
/// leaves room for a long boss beat on every floor.
pub const DEFAULT_MAX_SIM_SECONDS: f32 = 3600.0;

/// Parsed command line.
#[derive(Clone, Debug)]
pub struct BatchOpts {
    pub runs: usize,
    pub seed: u64,
    pub start_level: usize,
    pub skill: Skill,
    pub max_sim_seconds: f32,
    pub threads: usize,
    pub json: bool,
    /// `false` = full campaign from `start_level`, `true` = that level alone.
    pub practice: bool,
    /// What a measured level starts with. `None` is the real level-1 default;
    /// `Typical` is the modal three-powerup keep of real campaigns, so a
    /// `--level 2`/`--level 3` batch measures the level a carried player meets.
    pub carry: CarryPreset,
    /// Print a per-5-simulated-second trace of the first run (diagnostics).
    pub trace: bool,
    /// Run the anomaly scan (see [`super::anomaly`]) and report its classes.
    /// Costs roughly 10 % of a batch and never changes a run.
    pub analyze: bool,
    /// Rotate the skill per run (`Expert, Skilled, Casual, Expert, …`), so one
    /// batch samples the whole ladder. The headline rates are then over mixed
    /// skills; per-skill runs still use `--skill`.
    pub cycle_skills: bool,
    /// Treat `--seed` as a *run seed*, not a batch base: run `i` uses
    /// `seed + i`. This is how an anomaly report's `seed` is replayed
    /// (`--bot-batch 1 --seed-exact <seed> --trace`).
    pub exact_seed: bool,
    /// Play `--level` on a specific floor (`--route`): "laundry" and
    /// "printworks" reach the two side routes the batch could not otherwise
    /// measure at all (`--level 2` is always the Kitchen, `--level 3` the Lab).
    pub route: Option<crate::game::level::Theme>,
}

impl Default for BatchOpts {
    fn default() -> Self {
        BatchOpts {
            runs: 10,
            seed: 1,
            start_level: 0,
            skill: Skill::Expert,
            max_sim_seconds: DEFAULT_MAX_SIM_SECONDS,
            threads: 1,
            json: false,
            practice: false,
            carry: CarryPreset::None,
            trace: false,
            analyze: false,
            cycle_skills: false,
            exact_seed: false,
            route: None,
        }
    }
}

/// The skills `--cycle-skills` rotates through, in ladder order.
pub const SKILL_CYCLE: [Skill; 3] = [Skill::Expert, Skill::Skilled, Skill::Casual];

/// One run's outcome (what the aggregation sees).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunOutcome {
    pub seed: u64,
    /// The skill this run played (`--cycle-skills` mixes them).
    #[serde(default)]
    pub skill: String,
    pub won: bool,
    pub death: Option<DeathOutcome>,
    pub total_time: f32,
    pub total_score: u32,
    pub total_kills: u32,
    /// Kills per simulated minute over the whole run.
    pub kills_per_min: f32,
    /// Fraction of simulated time within `NEAR_ENEMY_PX` of an enemy body.
    pub near_frac: f32,
    pub levels: Vec<LevelOutcome>,
    /// Anomalies the scan found (`--analyze`; empty otherwise).
    #[serde(default)]
    pub anomalies: Vec<Anomaly>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeathOutcome {
    pub level: usize,
    pub at: f32,
    pub cause: String,
    /// One-line classification ("surrounded in the open", "cornered", …).
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelOutcome {
    pub level: usize,
    pub time: f32,
    pub cleared: bool,
    pub reached_boss: bool,
    pub boss_seconds: f32,
    pub boss_defeated: bool,
    pub damage_total: f32,
    pub damage: BTreeMap<String, f32>,
    pub dps_at_boss: f32,
    /// Player XP level when the boss first appeared.
    #[serde(default)]
    pub level_at_boss: Option<u32>,
    pub boss_dps: f32,
    pub upgrades: Vec<String>,
    pub kept: Vec<String>,
    pub death: Option<DeathOutcome>,
    /// Seconds until the first level-up (`None` when none came).
    pub first_levelup: Option<f32>,
    /// Kills on this level.
    pub kills: u32,
    /// Fraction of the level's time within `NEAR_ENEMY_PX` of a body.
    pub near_frac: f32,
    /// Player level / upgrades / kills at 60, 120, 180, 240 s.
    pub progress: Vec<ProgressOutcome>,
    /// Enemies alive at death.
    pub enemies_at_death: Option<u32>,
    /// Effective damage dealt to enemies by each weapon (`WeaponKind::all()`
    /// order), for the per-weapon balance table.
    #[serde(default)]
    pub dealt: BTreeMap<String, f32>,
    /// Damage dealt to this level's boss by each weapon (see
    /// [`LevelStats::boss_dealt`]).
    #[serde(default)]
    pub boss_dealt: BTreeMap<String, f32>,
    /// Highest player level reached on this level.
    #[serde(default)]
    pub max_player_level: u32,
    /// Peak analytic build DPS on this level.
    #[serde(default)]
    pub max_analytic_dps: f32,
}

/// A progression checkpoint as it appears in a report.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ProgressOutcome {
    pub at: f32,
    pub player_level: u32,
    pub upgrades: u32,
    pub kills: u32,
    pub hp_frac: f32,
}

impl LevelOutcome {
    fn from_stats(s: &LevelStats) -> LevelOutcome {
        let note = s.death_note.clone();
        LevelOutcome {
            level: s.level,
            time: s.time,
            cleared: s.boss_defeated,
            reached_boss: s.reached_boss,
            boss_seconds: s.boss_seconds,
            boss_defeated: s.boss_defeated,
            damage_total: s.damage_total,
            damage: s
                .damage
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            dps_at_boss: s.dps_at_boss,
            level_at_boss: s.level_at_boss,
            boss_dps: s.boss_dps,
            upgrades: s.upgrades.clone(),
            kept: s.kept.clone(),
            death: s.death.map(|d| death_outcome(d, note)),
            first_levelup: s.first_levelup,
            kills: s.kills,
            near_frac: if s.time > 0.0 {
                (s.near_seconds / s.time).clamp(0.0, 1.0)
            } else {
                0.0
            },
            progress: s.progress.iter().map(progress_outcome).collect(),
            enemies_at_death: s.enemies_at_death,
            dealt: weapon_damage_map(&s.dealt),
            boss_dealt: weapon_damage_map(&s.boss_dealt),
            max_player_level: s.max_player_level,
            max_analytic_dps: s.max_analytic_dps,
        }
    }
}

/// `[f32; WEAPON_SLOTS]` in `WeaponKind::all()` order → a serialisable name map.
fn weapon_damage_map(by_kind: &[f32; crate::game::weapon::WEAPON_SLOTS]) -> BTreeMap<String, f32> {
    WeaponKind::all()
        .iter()
        .map(|k| (k.name().to_string(), by_kind[k.index()]))
        .collect()
}

fn progress_outcome(p: &Progress) -> ProgressOutcome {
    ProgressOutcome {
        at: p.at,
        player_level: p.player_level,
        upgrades: p.upgrades,
        kills: p.kills,
        hp_frac: p.hp_frac,
    }
}

fn death_outcome(d: DeathInfo, note: Option<String>) -> DeathOutcome {
    DeathOutcome {
        level: d.level,
        at: d.at,
        cause: d.cause.name().to_string(),
        note,
    }
}

/// Everything the report prints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchReport {
    pub skill: String,
    pub practice: bool,
    pub start_level: usize,
    /// What the measured level started with (`CarryPreset::name`).
    #[serde(default)]
    pub carry: String,
    pub base_seed: u64,
    pub runs: usize,
    pub campaign_wins: usize,
    pub win_rate: f32,
    /// Wilson 95 % interval of `win_rate`, as `[low, high]`.
    #[serde(default)]
    pub win_rate_ci: [f32; 2],
    /// Was the anomaly scan on for this batch?
    #[serde(default)]
    pub analysis: bool,
    /// Anomaly classes found (see [`super::anomaly`]); empty when off.
    #[serde(default)]
    pub anomalies: Vec<AnomalyAgg>,
    pub sim_seconds: f64,
    pub wall_seconds: f64,
    pub threads: usize,
    pub sim_per_wall_second_per_thread: f64,
    /// Survival time per run (all levels), as a distribution.
    pub survival: Percentiles,
    /// Kills per simulated minute, averaged over runs.
    pub kills_per_min: f32,
    /// Fraction of time within `NEAR_ENEMY_PX` of an enemy body, all levels.
    pub near_frac: f32,
    /// Seconds to the first level-up, over every level attempted.
    pub first_levelup: Percentiles,
    /// Enemies alive at death, over every death.
    pub enemies_at_death: Percentiles,
    /// Player level / upgrades / kills at 60, 120, 180 and 240 s (all levels).
    pub progress: Vec<ProgressAgg>,
    /// Cause-of-death classifications, most common first.
    pub death_notes: Vec<(String, usize)>,
    pub levels: Vec<LevelAgg>,
    /// All runs' damage by source, summed and averaged.
    pub damage: Vec<DamageAgg>,
    /// Every death, sorted by level then time.
    pub deaths: Vec<DeathOutcome>,
    /// One line per run: seed, outcome, how long it lasted. This is how a
    /// specific seed (for instance a campaign win) is found again.
    pub run_summaries: Vec<RunSummary>,
    /// Final builds of the runs that reached the last boss, most common first.
    pub builds: Vec<(String, usize)>,
    /// Per-weapon damage share, pick rate and win correlation (see
    /// [`WeaponAgg`]). This is the table that answers "is every weapon a
    /// reasonable pick", which used to be guesswork.
    #[serde(default)]
    pub weapons: Vec<WeaponAgg>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub seed: u64,
    #[serde(default)]
    pub skill: String,
    pub won: bool,
    /// Levels cleared (0-based count) — a campaign win clears 3.
    pub levels_cleared: usize,
    pub total_time: f32,
    pub kills_per_min: f32,
    pub near_frac: f32,
    /// Total damage taken on the run (all levels).
    #[serde(default)]
    pub damage_taken: f32,
    /// Seconds with a boss alive (all levels).
    #[serde(default)]
    pub boss_seconds: f32,
    /// Highest player level reached in the run.
    #[serde(default)]
    pub max_player_level: u32,
    pub death: Option<DeathOutcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelAgg {
    pub level: usize,
    pub attempted: usize,
    pub cleared: usize,
    pub deaths: usize,
    pub reached_boss: usize,
    pub boss_kills: usize,
    pub clear_rate: f32,
    /// Wilson 95 % interval of `clear_rate`, as `[low, high]`.
    #[serde(default)]
    pub clear_rate_ci: [f32; 2],
    /// Death clock on this level: min / p25 / median / p75 / max.
    pub death_time: Percentiles,
    /// Time on the level (all attempted runs).
    pub time: Percentiles,
    /// Boss fight length in seconds, for the runs that reached a boss.
    pub boss_seconds: Percentiles,
    /// Build DPS when the boss appeared.
    pub dps_at_boss: f32,
    /// Player XP level when the boss appeared (min / p25 / median / p75 / max).
    #[serde(default)]
    pub level_at_boss: Percentiles,
    /// Measured DPS into the boss.
    pub boss_dps: f32,
    /// Seconds to the first level-up on this level.
    pub first_levelup: Percentiles,
    /// Kills per simulated minute on this level.
    pub kills_per_min: f32,
    /// Fraction of this level's time within `NEAR_ENEMY_PX` of a body.
    pub near_frac: f32,
    /// Enemies alive at death on this level.
    pub enemies_at_death: Percentiles,
    /// Player level / upgrades / kills at the fixed checkpoints.
    pub progress: Vec<ProgressAgg>,
    /// Cause-of-death classifications, most common first.
    pub death_notes: Vec<(String, usize)>,
    pub damage: Vec<DamageAgg>,
    pub upgrades: Vec<(String, usize)>,
    pub kept: Vec<(String, usize)>,
    /// Per-weapon damage share / pick rate / win correlation on this level.
    #[serde(default)]
    pub weapons: Vec<WeaponAgg>,
}

/// A progression checkpoint aggregated over runs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ProgressAgg {
    pub at: f32,
    pub runs: usize,
    pub mean_player_level: f32,
    pub mean_upgrades: f32,
    pub mean_kills: f32,
    pub mean_hp_frac: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Percentiles {
    pub n: usize,
    pub min: f32,
    pub p25: f32,
    pub median: f32,
    pub p75: f32,
    pub max: f32,
    pub mean: f32,
}

impl Percentiles {
    fn of(mut v: Vec<f32>) -> Percentiles {
        if v.is_empty() {
            return Percentiles::default();
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        let at = |q: f32| v[((n as f32 - 1.0) * q).round() as usize];
        Percentiles {
            n,
            min: v[0],
            p25: at(0.25),
            median: at(0.5),
            p75: at(0.75),
            max: v[n - 1],
            mean: v.iter().sum::<f32>() / n as f32,
        }
    }
}

/// Wilson score interval for a proportion, 95 % (z = 1.96).
///
/// `[low, high]` in 0..1, clamped. With `n == 0` it is the uninformative
/// `[0, 1]`. A Wilson interval is the right one at these sample sizes: the
/// normal approximation collapses to a zero-width interval at 0 % and 100 %,
/// which is exactly where a clear rate lives after a small batch.
pub fn wilson95(successes: usize, n: usize) -> [f32; 2] {
    wilson_ci(successes, n, 1.96)
}

/// Wilson score interval for a proportion at an arbitrary `z`.
pub fn wilson_ci(successes: usize, n: usize, z: f32) -> [f32; 2] {
    if n == 0 {
        return [0.0, 1.0];
    }
    let p = successes as f32 / n as f32;
    let n = n as f32;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = (z / denom) * ((p * (1.0 - p) / n + z2 / (4.0 * n * n)).max(0.0)).sqrt();
    [(centre - half).clamp(0.0, 1.0), (centre + half).clamp(0.0, 1.0)]
}

/// Runs per arm needed for a 95 % interval half-width of `half_width` around a
/// rate near `p` (the worst case is `p = 0.5`).
pub fn runs_for_half_width(p: f32, half_width: f32) -> usize {
    let p = p.clamp(0.001, 0.999);
    ((1.96 * 1.96) * p * (1.0 - p) / (half_width * half_width)).ceil() as usize
}

/// Runs per arm needed to resolve a difference of `margin` (absolute, e.g.
/// 0.05) between two independent rates near `p`, at 95 % confidence and 80 %
/// power (the standard two-proportion sample-size formula).
///
/// This is the number that matters when judging tuning: a batch of 30 cannot
/// tell a 5-point move from noise, whatever its direction looks like.
pub fn runs_to_resolve(p: f32, margin: f32) -> usize {
    let p = p.clamp(0.001, 0.999);
    let (z_a, z_b) = (1.96f32, 0.8416f32);
    let n = 2.0 * p * (1.0 - p) * (z_a + z_b).powi(2) / (margin * margin);
    n.ceil() as usize
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DamageAgg {
    pub source: String,
    /// Summed over all runs.
    pub total: f32,
    /// Mean per run that took damage from it.
    pub mean: f32,
    /// Mean per attempted run (so it reads as "damage per run").
    pub per_run: f32,
    pub hits: usize,
}

/// One weapon's balance row: how much damage it dealt, how often it was taken,
/// and how the runs that took it did.
///
/// Overall (`BatchReport::weapons`) `taken` counts **runs** in which the weapon
/// was picked at least once (a starting Gnaw that was never levelled does not
/// count) and `win_rate_with` is the campaign win rate of those runs, so
/// `win_rate_with` vs `win_rate_without` is the correlation to read. Per level
/// (`LevelAgg::weapons`) the same fields count **levels** and use the level's
/// own clear rate instead of the campaign's.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WeaponAgg {
    pub weapon: String,
    /// Effective damage dealt to enemies, summed over the batch.
    pub dealt: f32,
    /// `dealt / every weapon's dealt` (0..1).
    pub share: f32,
    /// Damage dealt to bosses, summed over the batch.
    pub boss_dealt: f32,
    /// `boss_dealt / every weapon's boss_dealt` (0..1).
    pub boss_share: f32,
    /// Level-up picks of this weapon (across every level and run).
    pub picks: usize,
    /// Runs (overall) / levels (per level) that took it at least once.
    pub taken: usize,
    /// `taken / runs` (overall) or `taken / attempted` (per level).
    pub pick_rate: f32,
    /// Of the runs/levels that took it, how many won the campaign (overall) or
    /// cleared the level (per level).
    pub wins_with: usize,
    pub win_rate_with: f32,
    /// The same rate for the runs/levels that did **not** take it.
    pub win_rate_without: f32,
}

impl WeaponAgg {
    /// Does a level-up card title ("Gnaw Lv2", "Tail Whip (NEW)") name `kind`?
    fn title_is(title: &str, kind: WeaponKind) -> bool {
        title.starts_with(kind.name())
    }
}

// ---- Running -------------------------------------------------------------

/// Play one run to its end (or the time cap).
pub fn run_one(seed: u64, opts: AutoOpts, max_sim_seconds: f32) -> RunOutcome {
    let mut run = AutoRun::new(seed, opts);
    run_to_end(&mut run, DT, max_sim_seconds);
    outcome_of(seed, run.stats())
}

/// Play one run with the anomaly scan watching every tick.
///
/// The scan only reads the world, so the run is identical to [`run_one`]; the
/// step loop is spelled out here because the scan needs a tick after each one.
pub fn run_one_analyzed(seed: u64, opts: AutoOpts, max_sim_seconds: f32) -> RunOutcome {
    let mut run = AutoRun::new(seed, opts);
    let mut scan = AnomalyScan::new(seed);
    let mut simulated = 0.0f32;
    while !run.finished() && simulated < max_sim_seconds {
        run.step(DT);
        simulated += DT;
        scan.tick(run.world(), DT);
    }
    let anomalies = scan.finish();
    let mut outcome = outcome_of(seed, run.stats());
    outcome.anomalies = anomalies;
    outcome
}

/// The seed run `index` of this batch plays. `--seed-exact` takes the number
/// literally (so a reported run seed can be replayed); otherwise the base is a
/// batch label and every run is hashed off it ([`seed_for`]).
fn run_seed(opts: &BatchOpts, index: usize) -> u64 {
    if opts.exact_seed {
        opts.seed.wrapping_add(index as u64)
    } else {
        seed_for(opts.seed, index)
    }
}

/// The skill a run plays: `--cycle-skills` rotates the ladder, `--skill` is
/// fixed, and both record the skill on the outcome.
fn skill_for(opts: &BatchOpts, index: usize) -> Skill {
    if opts.cycle_skills {
        SKILL_CYCLE[index % SKILL_CYCLE.len()]
    } else {
        opts.skill
    }
}

/// One batch row: the run and the outcome, with the skill recorded.
fn run_row(opts: &BatchOpts, index: usize) -> RunOutcome {
    let seed = run_seed(opts, index);
    let skill = skill_for(opts, index);
    let auto = AutoOpts {
        skill,
        ..auto_opts(opts)
    };
    let mut outcome = if opts.analyze {
        run_one_analyzed(seed, auto, opts.max_sim_seconds)
    } else {
        run_one(seed, auto, opts.max_sim_seconds)
    };
    outcome.skill = skill.name().to_string();
    outcome
}

/// One run's statistics as a report row (shared by every batch entry point).
fn outcome_of(seed: u64, stats: &super::RunStats) -> RunOutcome {
    let levels: Vec<LevelOutcome> = stats.levels.iter().map(LevelOutcome::from_stats).collect();
    let kills_per_min = if stats.total_time > 0.0 {
        stats.total_kills as f32 / (stats.total_time / 60.0)
    } else {
        0.0
    };
    let near_seconds: f32 = stats.levels.iter().map(|l| l.near_seconds).sum();
    let level_time: f32 = stats.levels.iter().map(|l| l.time).sum();
    let death = stats.death.map(|d| {
        let note = stats
            .levels
            .iter()
            .find(|l| l.level == d.level)
            .and_then(|l| l.death_note.clone());
        death_outcome(d, note)
    });
    RunOutcome {
        seed,
        skill: String::new(),
        won: stats.won,
        death,
        total_time: stats.total_time,
        total_score: stats.total_score,
        total_kills: stats.total_kills,
        kills_per_min,
        near_frac: if level_time > 0.0 {
            (near_seconds / level_time).clamp(0.0, 1.0)
        } else {
            0.0
        },
        levels,
        anomalies: Vec::new(),
    }
}

/// Run a whole batch (sequentially). `run_batch_threaded` is the fast entry.
pub fn run_batch(opts: &BatchOpts) -> BatchReport {
    let started = Instant::now();
    let mut outcomes = Vec::with_capacity(opts.runs);
    for i in 0..opts.runs {
        outcomes.push(run_row(opts, i));
    }
    let wall = started.elapsed().as_secs_f64();
    aggregate(opts, outcomes, wall)
}

/// Run a batch across `opts.threads` worker threads.
pub fn run_batch_threaded(opts: &BatchOpts) -> BatchReport {
    let started = Instant::now();
    let threads = opts.threads.max(1).min(opts.runs.max(1));
    let mut outcomes: Vec<Option<RunOutcome>> = (0..opts.runs).map(|_| None).collect();
    if threads <= 1 {
        for (i, slot) in outcomes.iter_mut().enumerate() {
            *slot = Some(run_row(opts, i));
        }
    } else {
        let opts = opts.clone();
        let runs = opts.runs;
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(threads);
            for t in 0..threads {
                let opts = opts.clone();
                handles.push(scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut i = t;
                    while i < runs {
                        out.push((i, run_row(&opts, i)));
                        i += threads;
                    }
                    out
                }));
            }
            for h in handles {
                for (i, outcome) in h.join().expect("batch worker panicked") {
                    outcomes[i] = Some(outcome);
                }
            }
        });
    }
    let wall = started.elapsed().as_secs_f64();
    // Run order, never completion order: the aggregate is deterministic.
    let outcomes: Vec<RunOutcome> = outcomes
        .into_iter()
        .enumerate()
        .map(|(i, o)| o.unwrap_or_else(|| run_row(opts, i)))
        .collect();
    aggregate(opts, outcomes, wall)
}

/// Single-threaded batch that traces the first run (diagnostics only; the
/// aggregate is still the same report).
pub fn run_batch_traced(opts: &BatchOpts) -> BatchReport {
    let started = Instant::now();
    let mut outcomes = Vec::with_capacity(opts.runs);
    for i in 0..opts.runs {
        let seed = run_seed(opts, i);
        let skill = skill_for(opts, i);
        let mut run = AutoRun::new(
            seed,
            AutoOpts {
                skill,
                ..auto_opts(opts)
            },
        );
        let mut next_trace = 0.0f32;
        let mut simulated = 0.0f32;
        while !run.finished() && simulated < opts.max_sim_seconds {
            run.step(DT);
            simulated += DT;
            if i == 0 && run.world().elapsed >= next_trace {
                eprintln!("{}", run.trace_line());
                next_trace += 0.5;
            }
        }
        let mut outcome = outcome_of(seed, run.stats());
        outcome.skill = skill.name().to_string();
        outcomes.push(outcome);
    }
    let wall = started.elapsed().as_secs_f64();
    aggregate(opts, outcomes, wall)
}

/// The seed run `index` of a batch uses. Public so tests and the report can
/// name the exact run behind a number.
pub fn seed_for(base: u64, index: usize) -> u64 {
    // Splitmix64 finaliser, so neighbouring runs get unrelated maps.
    let mut h = base
        .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

fn auto_opts(opts: &BatchOpts) -> AutoOpts {
    AutoOpts {
        start_level: opts.start_level,
        campaign: !opts.practice,
        skill: opts.skill,
        carry: opts.carry,
        route: opts.route,
    }
}

/// Fold per-run outcomes into the report. `outcomes` is in run order.
fn aggregate(opts: &BatchOpts, outcomes: Vec<RunOutcome>, wall_seconds: f64) -> BatchReport {
    let runs = outcomes.len();
    let mut levels: Vec<LevelAgg> = Vec::new();
    let mut damage: BTreeMap<String, (f32, usize)> = BTreeMap::new();
    let mut deaths: Vec<DeathOutcome> = Vec::new();
    let mut builds: BTreeMap<String, usize> = BTreeMap::new();
    let mut campaign_wins = 0;
    let mut sim_seconds = 0.0f64;
    let mut run_summaries: Vec<RunSummary> = Vec::with_capacity(outcomes.len());
    // Overall (all levels) accumulators for the new tuning columns.
    let mut firsts: Vec<f32> = Vec::new();
    let mut enemies_at_death: Vec<f32> = Vec::new();
    let mut notes: BTreeMap<String, usize> = BTreeMap::new();
    let mut overall_progress: Vec<Vec<ProgressOutcome>> = vec![Vec::new(); PROGRESS_TIMES.len()];
    let mut near_weighted = 0.0f32;
    let mut near_total_time = 0.0f32;
    // Per-weapon balance columns (all levels), see `WeaponAgg`.
    let mut w_dealt: BTreeMap<String, f32> = BTreeMap::new();
    let mut w_boss: BTreeMap<String, f32> = BTreeMap::new();
    let mut w_picks: BTreeMap<String, usize> = BTreeMap::new();
    let mut w_taken: BTreeMap<String, usize> = BTreeMap::new();
    let mut w_won: BTreeMap<String, usize> = BTreeMap::new();

    for outcome in &outcomes {
        if outcome.won {
            campaign_wins += 1;
        }
        sim_seconds += outcome.total_time as f64;
        near_weighted += outcome.near_frac * outcome.total_time;
        near_total_time += outcome.total_time;
        if let Some(d) = &outcome.death {
            deaths.push(d.clone());
            if let Some(n) = &d.note {
                *notes.entry(n.clone()).or_insert(0) += 1;
            }
        }
        run_summaries.push(RunSummary {
            seed: outcome.seed,
            skill: outcome.skill.clone(),
            won: outcome.won,
            levels_cleared: outcome.levels.iter().filter(|l| l.cleared).count(),
            total_time: outcome.total_time,
            kills_per_min: outcome.kills_per_min,
            near_frac: outcome.near_frac,
            damage_taken: outcome.levels.iter().map(|l| l.damage_total).sum(),
            boss_seconds: outcome.levels.iter().map(|l| l.boss_seconds).sum(),
            max_player_level: outcome
                .levels
                .iter()
                .map(|l| l.max_player_level)
                .max()
                .unwrap_or(0),
            death: outcome.death.clone(),
        });
        // A weapon "was taken" by a run once, however many levels levelled it.
        let mut taken_in_run = [false; crate::game::weapon::WEAPON_SLOTS];
        for level in &outcome.levels {
            let agg = match levels.iter_mut().find(|l| l.level == level.level) {
                Some(a) => a,
                None => {
                    levels.push(LevelAgg {
                        level: level.level,
                        attempted: 0,
                        cleared: 0,
                        deaths: 0,
                        reached_boss: 0,
                        boss_kills: 0,
                        clear_rate: 0.0,
                        clear_rate_ci: [0.0, 1.0],
                        death_time: Percentiles::default(),
                        time: Percentiles::default(),
                        boss_seconds: Percentiles::default(),
                        dps_at_boss: 0.0,
                        level_at_boss: Percentiles::default(),
                        boss_dps: 0.0,
                        first_levelup: Percentiles::default(),
                        kills_per_min: 0.0,
                        near_frac: 0.0,
                        enemies_at_death: Percentiles::default(),
                        progress: Vec::new(),
                        death_notes: Vec::new(),
                        damage: Vec::new(),
                        upgrades: Vec::new(),
                        kept: Vec::new(),
                        weapons: Vec::new(),
                    });
                    levels.last_mut().expect("just pushed")
                }
            };
            agg.attempted += 1;
            if level.cleared || level.boss_defeated {
                agg.cleared += 1;
            }
            if level.death.is_some() {
                agg.deaths += 1;
            }
            if level.reached_boss {
                agg.reached_boss += 1;
            }
            if level.boss_defeated {
                agg.boss_kills += 1;
            }
            if let Some(f) = level.first_levelup {
                firsts.push(f);
            }
            if let Some(n) = level.enemies_at_death {
                enemies_at_death.push(n as f32);
            }
            collect_progress(&mut overall_progress, &level.progress);
            for (src, amount) in &level.damage {
                let entry = damage.entry(src.clone()).or_insert((0.0, 0));
                entry.0 += *amount;
                entry.1 += 1;
            }
            for up in &level.upgrades {
                *builds.entry(up.clone()).or_insert(0) += 1;
            }
            for (w, v) in &level.dealt {
                *w_dealt.entry(w.clone()).or_insert(0.0) += *v;
            }
            for (w, v) in &level.boss_dealt {
                *w_boss.entry(w.clone()).or_insert(0.0) += *v;
            }
            for kind in WeaponKind::all() {
                for u in &level.upgrades {
                    if WeaponAgg::title_is(u, kind) {
                        *w_picks.entry(kind.name().to_string()).or_insert(0) += 1;
                        if !taken_in_run[kind.index()] {
                            taken_in_run[kind.index()] = true;
                            *w_taken.entry(kind.name().to_string()).or_insert(0) += 1;
                            if outcome.won {
                                *w_won.entry(kind.name().to_string()).or_insert(0) += 1;
                            }
                        }
                        break;
                    }
                }
            }
        }
        // The last level reached carries this run's final kept set.
        if let Some(last) = outcome.levels.last() {
            for k in &last.kept {
                *builds.entry(format!("kept {k}")).or_insert(0) += 1;
            }
        }
    }

    // Per-level distributions, recomputed in run order.
    for agg in levels.iter_mut() {
        let mut times = Vec::new();
        let mut death_times = Vec::new();
        let mut boss_seconds = Vec::new();
        let mut dps = Vec::new();
        let mut boss_dps = Vec::new();
        let mut levels_at_boss: Vec<f32> = Vec::new();
        let mut dmg: BTreeMap<String, (f32, usize)> = BTreeMap::new();
        let mut upgrades: BTreeMap<String, usize> = BTreeMap::new();
        let mut kept: BTreeMap<String, usize> = BTreeMap::new();
        let mut firsts: Vec<f32> = Vec::new();
        let mut enemies: Vec<f32> = Vec::new();
        let mut notes: BTreeMap<String, usize> = BTreeMap::new();
        let mut progress: Vec<Vec<ProgressOutcome>> =
            vec![Vec::new(); PROGRESS_TIMES.len()];
        let mut kill_rates: Vec<f32> = Vec::new();
        let mut near_weighted = 0.0f32;
        let mut near_time = 0.0f32;
        // Per-weapon balance columns for this level (see `WeaponAgg`).
        let mut w_dealt: BTreeMap<String, f32> = BTreeMap::new();
        let mut w_boss: BTreeMap<String, f32> = BTreeMap::new();
        let mut w_picks: BTreeMap<String, usize> = BTreeMap::new();
        let mut w_taken: BTreeMap<String, usize> = BTreeMap::new();
        let mut w_cleared: BTreeMap<String, usize> = BTreeMap::new();
        for outcome in &outcomes {
            for level in outcome.levels.iter().filter(|l| l.level == agg.level) {
                times.push(level.time);
                if level.time > 0.0 {
                    kill_rates.push(level.kills as f32 / (level.time / 60.0));
                }
                near_weighted += level.near_frac * level.time;
                near_time += level.time;
                if let Some(d) = &level.death {
                    death_times.push(d.at);
                    if let Some(n) = &d.note {
                        *notes.entry(n.clone()).or_insert(0) += 1;
                    }
                }
                if let Some(f) = level.first_levelup {
                    firsts.push(f);
                }
                if let Some(n) = level.enemies_at_death {
                    enemies.push(n as f32);
                }
                collect_progress(&mut progress, &level.progress);
                if level.reached_boss {
                    boss_seconds.push(level.boss_seconds);
                    dps.push(level.dps_at_boss);
                    boss_dps.push(level.boss_dps);
                    if let Some(lv) = level.level_at_boss {
                        levels_at_boss.push(lv as f32);
                    }
                }
                for (src, amount) in &level.damage {
                    let e = dmg.entry(src.clone()).or_insert((0.0, 0));
                    e.0 += *amount;
                    e.1 += 1;
                }
                for u in &level.upgrades {
                    *upgrades.entry(u.clone()).or_insert(0) += 1;
                }
                for k in &level.kept {
                    *kept.entry(k.clone()).or_insert(0) += 1;
                }
                for (w, v) in &level.dealt {
                    *w_dealt.entry(w.clone()).or_insert(0.0) += *v;
                }
                for (w, v) in &level.boss_dealt {
                    *w_boss.entry(w.clone()).or_insert(0.0) += *v;
                }
                let cleared = level.cleared || level.boss_defeated;
                for kind in WeaponKind::all() {
                    let took = level
                        .upgrades
                        .iter()
                        .any(|u| WeaponAgg::title_is(u, kind));
                    if took {
                        *w_taken.entry(kind.name().to_string()).or_insert(0) += 1;
                        if cleared {
                            *w_cleared.entry(kind.name().to_string()).or_insert(0) += 1;
                        }
                    }
                }
                for u in &level.upgrades {
                    for kind in WeaponKind::all() {
                        if WeaponAgg::title_is(u, kind) {
                            *w_picks.entry(kind.name().to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        agg.clear_rate = if agg.attempted == 0 {
            0.0
        } else {
            agg.cleared as f32 / agg.attempted as f32
        };
        agg.clear_rate_ci = wilson95(agg.cleared, agg.attempted);
        agg.death_time = Percentiles::of(death_times);
        agg.time = Percentiles::of(times);
        agg.boss_seconds = Percentiles::of(boss_seconds);
        agg.dps_at_boss = mean(&dps);
        agg.level_at_boss = Percentiles::of(levels_at_boss);
        agg.boss_dps = mean(&boss_dps);
        agg.first_levelup = Percentiles::of(firsts);
        agg.kills_per_min = mean(&kill_rates);
        agg.near_frac = if near_time > 0.0 {
            (near_weighted / near_time).clamp(0.0, 1.0)
        } else {
            0.0
        };
        agg.enemies_at_death = Percentiles::of(enemies);
        agg.progress = progress_aggs(progress);
        agg.death_notes = top(notes, 8);
        agg.damage = damage_vec(dmg, agg.attempted);
        agg.upgrades = top(upgrades, 12);
        agg.kept = top(kept, 12);
        agg.weapons = weapon_aggs(
            &w_dealt,
            &w_boss,
            &w_picks,
            &w_taken,
            &w_cleared,
            agg.attempted,
            agg.cleared,
        );
    }
    // ---- Anomalies ------------------------------------------------------
    //
    // The per-tick classes come from the scan; the four batch-level classes
    // need every run, so they are computed here (the "2x the median level" and
    // "3x the median level/DPS" tests).
    let mut found: Vec<Anomaly> = if opts.analyze {
        outcomes
            .iter()
            .flat_map(|o| o.anomalies.iter().cloned())
            .collect()
    } else {
        Vec::new()
    };
    if opts.analyze {
        for agg in &levels {
            // `(seed, level)` pairs, so every batch-level find names the run it
            // came from and can be replayed with `--seed-exact`.
            let runs: Vec<(u64, &LevelOutcome)> = outcomes
                .iter()
                .flat_map(|o| o.levels.iter().map(move |l| (o.seed, l)))
                .filter(|(_, l)| l.level == agg.level)
                .collect();
            let median_time = agg.time.median;
            let mut levels_seen: Vec<f32> =
                runs.iter().map(|(_, l)| l.max_player_level as f32).collect();
            levels_seen.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median_level = median_of(&levels_seen);
            let mut dps_seen: Vec<f32> = runs
                .iter()
                .map(|(_, l)| l.max_analytic_dps)
                .filter(|d| *d > 0.0)
                .collect();
            dps_seen.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median_dps = median_of(&dps_seen);
            for (seed, run) in runs {
                if median_time > 0.0
                    && run.time > median_time * anomaly::LEVEL_MEDIAN_FACTOR
                    && run.death.is_some()
                {
                    found.push(Anomaly {
                        kind: AnomalyKind::LongLevel,
                        seed,
                        level: run.level,
                        at: run.time,
                        seconds: run.time,
                        detail: format!(
                            "level lasted {:.0} s, {:.1}x the batch median {:.0} s",
                            run.time,
                            run.time / median_time,
                            median_time
                        ),
                    });
                }
                if median_level > 0.0
                    && run.max_player_level as f32 > median_level * anomaly::OUTLIER_FACTOR
                {
                    found.push(Anomaly {
                        kind: AnomalyKind::LevelOutlier,
                        seed,
                        level: run.level,
                        at: run.time,
                        seconds: run.max_player_level as f32,
                        detail: format!(
                            "player level {} is {:.1}x the batch median {:.0}",
                            run.max_player_level,
                            run.max_player_level as f32 / median_level,
                            median_level
                        ),
                    });
                }
                if median_dps > 0.0
                    && run.max_analytic_dps > median_dps * anomaly::OUTLIER_FACTOR
                {
                    found.push(Anomaly {
                        kind: AnomalyKind::DpsOutlier,
                        seed,
                        level: run.level,
                        at: run.time,
                        seconds: run.max_analytic_dps,
                        detail: format!(
                            "peak build dps {:.0} is {:.1}x the batch median {:.0}",
                            run.max_analytic_dps,
                            run.max_analytic_dps / median_dps,
                            median_dps
                        ),
                    });
                }
            }
        }
    }
    let anomalies = anomaly::summarise(&found);

    levels.sort_by_key(|l| l.level);
    deaths.sort_by(|a, b| {
        a.level
            .cmp(&b.level)
            .then(a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal))
    });

    let survival: Vec<f32> = outcomes.iter().map(|o| o.total_time).collect();
    let per_run_kills: Vec<f32> = outcomes.iter().map(|o| o.kills_per_min).collect();
    let near_frac = if near_total_time > 0.0 {
        (near_weighted / near_total_time).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let threads = opts.threads.max(1);
    BatchReport {
        skill: if opts.cycle_skills {
            "mixed".to_string()
        } else {
            opts.skill.name().to_string()
        },
        practice: opts.practice,
        start_level: opts.start_level,
        carry: opts.carry.name().to_string(),
        base_seed: opts.seed,
        runs,
        campaign_wins,
        win_rate: if runs == 0 {
            0.0
        } else {
            campaign_wins as f32 / runs as f32
        },
        win_rate_ci: wilson95(campaign_wins, runs),
        analysis: opts.analyze,
        anomalies,
        sim_seconds,
        wall_seconds,
        threads,
        sim_per_wall_second_per_thread: if wall_seconds > 0.0 {
            sim_seconds / wall_seconds / threads as f64
        } else {
            0.0
        },
        survival: Percentiles::of(survival),
        // Mean of the runs' own rates: a single 20-minute snowball must not
        // decide the batch's "kills per minute".
        kills_per_min: mean(&per_run_kills),
        near_frac,
        first_levelup: Percentiles::of(firsts),
        enemies_at_death: Percentiles::of(enemies_at_death),
        progress: progress_aggs(overall_progress),
        death_notes: top(notes, 8),
        levels,
        damage: damage_vec(damage, runs),
        deaths,
        run_summaries,
        builds: top(builds, 16),
        weapons: weapon_aggs(
            &w_dealt,
            &w_boss,
            &w_picks,
            &w_taken,
            &w_won,
            runs,
            campaign_wins,
        ),
    }
}

/// Build one [`WeaponAgg`] row per weapon from the per-weapon accumulators.
///
/// `taken`/`wins_with` are counted in the caller's unit (runs overall, levels
/// per level); `attempts` and `wins` are that unit's totals, so
/// `win_rate_without` compares "everyone who did not take it".
#[allow(clippy::too_many_arguments)]
fn weapon_aggs(
    dealt: &BTreeMap<String, f32>,
    boss: &BTreeMap<String, f32>,
    picks: &BTreeMap<String, usize>,
    taken: &BTreeMap<String, usize>,
    wins_with: &BTreeMap<String, usize>,
    attempts: usize,
    wins: usize,
) -> Vec<WeaponAgg> {
    let total_dealt: f32 = dealt.values().sum();
    let total_boss: f32 = boss.values().sum();
    let mut out: Vec<WeaponAgg> = WeaponKind::all()
        .iter()
        .map(|kind| {
            let name = kind.name();
            let d = dealt.get(name).copied().unwrap_or(0.0);
            let b = boss.get(name).copied().unwrap_or(0.0);
            let taken_n = taken.get(name).copied().unwrap_or(0);
            let wins_n = wins_with.get(name).copied().unwrap_or(0);
            let not_taken = attempts.saturating_sub(taken_n);
            let wins_without = wins.saturating_sub(wins_n);
            WeaponAgg {
                weapon: name.to_string(),
                dealt: d,
                share: if total_dealt > 0.0 {
                    d / total_dealt
                } else {
                    0.0
                },
                boss_dealt: b,
                boss_share: if total_boss > 0.0 {
                    b / total_boss
                } else {
                    0.0
                },
                picks: picks.get(name).copied().unwrap_or(0),
                taken: taken_n,
                pick_rate: if attempts > 0 {
                    taken_n as f32 / attempts as f32
                } else {
                    0.0
                },
                wins_with: wins_n,
                win_rate_with: if taken_n > 0 {
                    wins_n as f32 / taken_n as f32
                } else {
                    0.0
                },
                win_rate_without: if not_taken > 0 {
                    wins_without as f32 / not_taken as f32
                } else {
                    0.0
                },
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.dealt
            .partial_cmp(&a.dealt)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.weapon.cmp(&b.weapon))
    });
    out
}

/// File a checkpoint reading into its bucket (`PROGRESS_TIMES` index).
fn collect_progress(into: &mut [Vec<ProgressOutcome>], progress: &[ProgressOutcome]) {
    for p in progress {
        if let Some(i) = PROGRESS_TIMES
            .iter()
            .position(|t| (*t - p.at).abs() < 0.01)
        {
            into[i].push(*p);
        }
    }
}

/// Mean the checkpoint buckets into report rows.
fn progress_aggs(buckets: Vec<Vec<ProgressOutcome>>) -> Vec<ProgressAgg> {
    buckets
        .into_iter()
        .enumerate()
        .filter_map(|(i, v)| {
            if v.is_empty() {
                return None;
            }
            let n = v.len() as f32;
            Some(ProgressAgg {
                at: PROGRESS_TIMES[i],
                runs: v.len(),
                mean_player_level: v.iter().map(|p| p.player_level as f32).sum::<f32>() / n,
                mean_upgrades: v.iter().map(|p| p.upgrades as f32).sum::<f32>() / n,
                mean_kills: v.iter().map(|p| p.kills as f32).sum::<f32>() / n,
                mean_hp_frac: v.iter().map(|p| p.hp_frac).sum::<f32>() / n,
            })
        })
        .collect()
}

fn median_of(sorted: &[f32]) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[sorted.len() / 2]
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn damage_vec(map: BTreeMap<String, (f32, usize)>, runs: usize) -> Vec<DamageAgg> {
    let mut out: Vec<DamageAgg> = map
        .into_iter()
        .map(|(source, (total, hits))| DamageAgg {
            source,
            total,
            mean: if hits == 0 { 0.0 } else { total / hits as f32 },
            per_run: if runs == 0 { 0.0 } else { total / runs as f32 },
            hits,
        })
        .collect();
    out.sort_by(|a, b| {
        b.total
            .partial_cmp(&a.total)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source.cmp(&b.source))
    });
    out
}

fn top(map: BTreeMap<String, usize>, n: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

// ---- Comparing two reports ----------------------------------------------

/// Two saved batch reports, diffed metric by metric (before/after at a glance).
#[derive(Clone, Debug, Serialize)]
pub struct CompareReport {
    pub a: ReportHeader,
    pub b: ReportHeader,
    pub rows: Vec<CompareRow>,
    pub levels: Vec<CompareLevel>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReportHeader {
    pub path: String,
    pub skill: String,
    pub runs: usize,
    pub base_seed: u64,
    pub practice: bool,
    pub start_level: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompareLevel {
    pub level: usize,
    pub rows: Vec<CompareRow>,
}

/// One metric, both sides and the delta (`b - a`).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CompareRow {
    pub metric: &'static str,
    pub a: f32,
    pub b: f32,
    pub delta: f32,
    pub unit: Unit,
}

/// How a comparison value is printed (and what a delta means).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Unit {
    Seconds,
    Percent,
    CountPerMin,
    Count,
    Level,
    Damage,
}

/// Read a report written by `--bot-batch --json`.
pub fn load_report(path: &str) -> Result<BatchReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path} is not a batch report: {e}"))
}

/// Diff two reports. Rows are in a fixed order, so two comparisons of the same
/// files print identically.
pub fn compare_reports(
    a: &BatchReport,
    b: &BatchReport,
    a_path: &str,
    b_path: &str,
) -> CompareReport {
    let mut rows = vec![
        row(
            "campaign wins",
            a.campaign_wins as f32,
            b.campaign_wins as f32,
            Unit::Count,
        ),
        row("win rate", a.win_rate * 100.0, b.win_rate * 100.0, Unit::Percent),
        row("mean survival", a.survival.mean, b.survival.mean, Unit::Seconds),
        row(
            "median survival",
            a.survival.median,
            b.survival.median,
            Unit::Seconds,
        ),
        row("deaths", a.deaths.len() as f32, b.deaths.len() as f32, Unit::Count),
        row("kills/min", a.kills_per_min, b.kills_per_min, Unit::CountPerMin),
        row(
            "near-enemy time",
            a.near_frac * 100.0,
            b.near_frac * 100.0,
            Unit::Percent,
        ),
        row(
            "first level-up (median)",
            a.first_levelup.median,
            b.first_levelup.median,
            Unit::Seconds,
        ),
        row(
            "enemies at death (median)",
            a.enemies_at_death.median,
            b.enemies_at_death.median,
            Unit::Count,
        ),
        row(
            "damage per run",
            a.damage.iter().map(|d| d.per_run).sum::<f32>(),
            b.damage.iter().map(|d| d.per_run).sum::<f32>(),
            Unit::Damage,
        ),
    ];
    // Damage by source: the union of both sides, so a new source shows up as a
    // row rather than vanishing.
    let mut sources: Vec<String> = a
        .damage
        .iter()
        .chain(b.damage.iter())
        .map(|d| d.source.clone())
        .collect();
    sources.sort();
    sources.dedup();
    for source in sources {
        let av = a
            .damage
            .iter()
            .find(|d| d.source == source)
            .map(|d| d.per_run)
            .unwrap_or(0.0);
        let bv = b
            .damage
            .iter()
            .find(|d| d.source == source)
            .map(|d| d.per_run)
            .unwrap_or(0.0);
        let metric: &'static str = Box::leak(format!("damage/run: {source}").into_boxed_str());
        rows.push(row(metric, av, bv, Unit::Damage));
    }
    // Progression checkpoints: the same fixed times on both sides.
    for at in super::PROGRESS_TIMES {
        let ap = a.progress.iter().find(|p| (p.at - at).abs() < 0.01);
        let bp = b.progress.iter().find(|p| (p.at - at).abs() < 0.01);
        let metric: &'static str = Box::leak(format!("player level @{at:.0}s").into_boxed_str());
        rows.push(row(
            metric,
            ap.map(|p| p.mean_player_level).unwrap_or(0.0),
            bp.map(|p| p.mean_player_level).unwrap_or(0.0),
            Unit::Level,
        ));
        let metric: &'static str = Box::leak(format!("upgrades @{at:.0}s").into_boxed_str());
        rows.push(row(
            metric,
            ap.map(|p| p.mean_upgrades).unwrap_or(0.0),
            bp.map(|p| p.mean_upgrades).unwrap_or(0.0),
            Unit::Count,
        ));
    }

    // Per-level rows for the levels either side attempted.
    let mut level_ids: Vec<usize> = a.levels.iter().chain(b.levels.iter()).map(|l| l.level).collect();
    level_ids.sort_unstable();
    level_ids.dedup();
    let mut levels = Vec::new();
    for id in level_ids {
        let al = a.levels.iter().find(|l| l.level == id);
        let bl = b.levels.iter().find(|l| l.level == id);
        let get = |l: Option<&LevelAgg>, f: &dyn Fn(&LevelAgg) -> f32| l.map(f).unwrap_or(0.0);
        levels.push(CompareLevel {
            level: id,
            rows: vec![
                row(
                    "attempted",
                    get(al, &|l| l.attempted as f32),
                    get(bl, &|l| l.attempted as f32),
                    Unit::Count,
                ),
                row(
                    "cleared",
                    get(al, &|l| l.cleared as f32),
                    get(bl, &|l| l.cleared as f32),
                    Unit::Count,
                ),
                row(
                    "clear rate",
                    get(al, &|l| l.clear_rate * 100.0),
                    get(bl, &|l| l.clear_rate * 100.0),
                    Unit::Percent,
                ),
                row(
                    "death clock (median)",
                    get(al, &|l| l.death_time.median),
                    get(bl, &|l| l.death_time.median),
                    Unit::Seconds,
                ),
                row(
                    "boss reached",
                    get(al, &|l| l.reached_boss as f32),
                    get(bl, &|l| l.reached_boss as f32),
                    Unit::Count,
                ),
                row(
                    "kills/min",
                    get(al, &|l| l.kills_per_min),
                    get(bl, &|l| l.kills_per_min),
                    Unit::CountPerMin,
                ),
                row(
                    "near-enemy time",
                    get(al, &|l| l.near_frac * 100.0),
                    get(bl, &|l| l.near_frac * 100.0),
                    Unit::Percent,
                ),
                row(
                    "first level-up (median)",
                    get(al, &|l| l.first_levelup.median),
                    get(bl, &|l| l.first_levelup.median),
                    Unit::Seconds,
                ),
                row(
                    "enemies at death (median)",
                    get(al, &|l| l.enemies_at_death.median),
                    get(bl, &|l| l.enemies_at_death.median),
                    Unit::Count,
                ),
            ],
        });
    }

    CompareReport {
        a: header(a, a_path),
        b: header(b, b_path),
        rows,
        levels,
    }
}

fn header(r: &BatchReport, path: &str) -> ReportHeader {
    ReportHeader {
        path: path.to_string(),
        skill: r.skill.clone(),
        runs: r.runs,
        base_seed: r.base_seed,
        practice: r.practice,
        start_level: r.start_level,
    }
}

fn row(metric: &'static str, a: f32, b: f32, unit: Unit) -> CompareRow {
    CompareRow {
        metric,
        a,
        b,
        delta: b - a,
        unit,
    }
}

/// A comparison value in its own unit.
fn fmt_metric(v: f32, unit: Unit) -> String {
    match unit {
        Unit::Seconds => secs(v),
        Unit::Percent => format!("{v:.0}%"),
        Unit::CountPerMin | Unit::Count | Unit::Damage => format!("{v:.0}"),
        Unit::Level => format!("{v:.2}"),
    }
}

/// A delta in its own unit (percentage rows report percentage points).
fn fmt_delta(v: f32, unit: Unit) -> String {
    match unit {
        Unit::Seconds => {
            if v.abs() >= 60.0 {
                format!("{:+}s", v.round() as i64)
            } else {
                format!("{v:+.1}s")
            }
        }
        Unit::Percent => format!("{v:+.0}pp"),
        Unit::CountPerMin | Unit::Count | Unit::Damage => format!("{v:+.0}"),
        Unit::Level => format!("{v:+.2}"),
    }
}

/// The human-readable before/after table.
pub fn render_comparison(c: &CompareReport) -> String {
    let mut s = String::new();
    s.push_str("Rat Run bot comparison (delta = B - A)\n");
    for h in [&c.a, &c.b] {
        s.push_str(&format!(
            "  {}: {} runs, skill {}, {}, seed {}\n",
            if std::ptr::eq(h, &c.a) { "A" } else { "B" },
            h.runs,
            h.skill,
            if h.practice {
                "single level (practice)"
            } else {
                "full campaign"
            },
            h.base_seed
        ));
        s.push_str(&format!("     {}\n", h.path));
    }
    s.push('\n');
    let mut table = String::new();
    table.push_str(&format!(
        "{:<34} {:>10} {:>10} {:>10}\n",
        "metric", "A", "B", "delta"
    ));
    let mut rows = |rows: &[CompareRow], title: &str| {
        table.push_str(&format!("{title}\n"));
        for r in rows {
            table.push_str(&format!(
                "{:<34} {:>10} {:>10} {:>10}\n",
                r.metric,
                fmt_metric(r.a, r.unit),
                fmt_metric(r.b, r.unit),
                fmt_delta(r.delta, r.unit)
            ));
        }
    };
    rows(&c.rows, "overall");
    for l in &c.levels {
        rows(&l.rows, &format!("level {}", l.level + 1));
    }
    s.push_str(&table);
    s
}

/// `ratrun --bot-compare a.json b.json [--json]`.
fn run_compare_cli(args: &[String], pos: usize) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let files: Vec<&str> = args[pos + 1..]
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();
    if files.len() != 2 {
        eprintln!("ratrun --bot-compare: needs exactly two batch JSON files");
        eprintln!("{USAGE}");
        return 2;
    }
    let a = match load_report(files[0]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ratrun --bot-compare: {e}");
            return 2;
        }
    };
    let b = match load_report(files[1]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ratrun --bot-compare: {e}");
            return 2;
        }
    };
    let cmp = compare_reports(&a, &b, files[0], files[1]);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&cmp).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print!("{}", render_comparison(&cmp));
    }
    0
}

// ---- CLI -----------------------------------------------------------------

/// Handle `--bot-batch …`. Returns `None` when the flag is absent (so the real
/// game starts normally), or the process exit code.
pub fn maybe_run_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--bot-compare a.json b.json` diff two saved reports.
    if let Some(pos) = args.iter().position(|a| a == "--bot-compare") {
        return Some(run_compare_cli(&args, pos));
    }
    if !args.iter().any(|a| a == "--bot-batch") {
        return None;
    }
    let opts = match parse_args(&args) {
        Ok(Some(opts)) => opts,
        Ok(None) => return None,
        Err(msg) => {
            eprintln!("ratrun --bot-batch: {msg}");
            eprintln!("{USAGE}");
            return Some(2);
        }
    };
    let report = if opts.trace {
        run_batch_traced(&opts)
    } else {
        run_batch_threaded(&opts)
    };
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print!("{}", render_table(&report));
    }
    Some(0)
}

const USAGE: &str = "\
usage: ratrun --bot-batch <N> [--seed <S>] [--level <1-3>] [--practice] \
[--carry none|typical] [--route sewer|kitchen|lab|laundry|printworks] \
[--skill expert|skilled|casual] [--cycle-skills] \
[--seed-exact] [--max-sim-seconds <T>] [--threads <K>] [--analyze] [--json] \
[--trace] \
       ratrun --bot-compare <a.json> <b.json> [--json]

  --carry          start a measured level with a real carried loadout
                   (none = the level-1 default, typical = the modal keep-three)
  --analyze        also run the anomaly scan (see autoplay::anomaly) and list
                   every class it finds with example seeds (costs ~10 %)
  --cycle-skills   rotate Expert, Skilled, Casual per run (mixed-skill batch)
  --route          play --level on this floor instead of the original route's
                   (laundry / printworks are the two side routes)
  --seed-exact     read --seed as a run seed (replay an anomaly's seed)";

/// Parse a `--route` argument: the five campaign floors (and the flatbed).
pub fn parse_route(s: &str) -> Result<crate::game::level::Theme, String> {
    use crate::game::level::Theme;
    match s.to_ascii_lowercase().as_str() {
        "sewer" => Ok(Theme::Sewer),
        "kitchen" => Ok(Theme::Kitchen),
        "lab" => Ok(Theme::Lab),
        "laundry" => Ok(Theme::Laundry),
        "printworks" | "print" => Ok(Theme::Printworks),
        "flatbed" | "trailer" | "getaway" => Ok(Theme::Trailer),
        other => Err(format!("unknown route {other:?}")),
    }
}

/// Parse the batch flags. `Ok(None)` means "the flag was not for us".
pub fn parse_args(args: &[String]) -> Result<Option<BatchOpts>, String> {
    let mut opts = BatchOpts::default();
    let mut i = 0;
    let mut saw = false;
    while i < args.len() {
        let arg = args[i].as_str();
        let mut value = || -> Result<&str, String> {
            i += 1;
            args.get(i)
                .map(|s| s.as_str())
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg {
            "--bot-batch" => {
                saw = true;
                opts.runs = value()?.parse::<usize>().map_err(|_| "N must be a number")?;
            }
            "--seed" => {
                opts.seed = value()?.parse::<u64>().map_err(|_| "seed must be a number")?;
            }
            "--level" => {
                let level = value()?.parse::<usize>().map_err(|_| "level must be 1-3")?;
                if !(1..=campaign::SECTOR_COUNT).contains(&level) {
                    return Err(format!("level must be 1-{}", campaign::SECTOR_COUNT));
                }
                opts.start_level = level - 1;
            }
            "--practice" => opts.practice = true,
            "--carry" => {
                let s = value()?;
                opts.carry =
                    CarryPreset::parse(s).ok_or_else(|| format!("unknown carry {s:?}"))?;
            }
            "--skill" => {
                let s = value()?;
                opts.skill = Skill::parse(s).ok_or_else(|| format!("unknown skill {s:?}"))?;
            }
            "--max-sim-seconds" => {
                opts.max_sim_seconds = value()?
                    .parse::<f32>()
                    .map_err(|_| "max-sim-seconds must be a number")?;
                if !opts.max_sim_seconds.is_finite() || opts.max_sim_seconds <= 0.0 {
                    return Err("max-sim-seconds must be a positive number".into());
                }
            }
            "--threads" => {
                opts.threads = value()?
                    .parse::<usize>()
                    .map_err(|_| "threads must be a number")?;
                if opts.threads == 0 {
                    return Err("threads must be at least 1".into());
                }
            }
            "--json" => opts.json = true,
            "--trace" => opts.trace = true,
            "--analyze" => opts.analyze = true,
            "--cycle-skills" => opts.cycle_skills = true,
            "--seed-exact" => opts.exact_seed = true,
            "--route" => {
                let s = value()?;
                opts.route = Some(parse_route(s)?);
            }
            other => {
                if saw {
                    return Err(format!("unknown flag {other:?}"));
                }
                // Not our flag (another mode owns the command line); leave it.
                return Ok(None);
            }
        }
        i += 1;
    }
    if opts.runs == 0 {
        return Err("N must be at least 1".into());
    }
    Ok(Some(opts))
}

// ---- Text report ---------------------------------------------------------

fn pct(x: f32) -> String {
    format!("{:.0}%", x * 100.0)
}

/// A Wilson interval as "low-high" percent.
fn ci(ci: [f32; 2]) -> String {
    format!("{:.0}-{:.0}%", ci[0] * 100.0, ci[1] * 100.0)
}

/// The anomaly-scan section of the text report.
fn anomaly_section(classes: &[AnomalyAgg], runs: usize) -> String {
    let mut s = String::new();
    if classes.is_empty() {
        s.push_str(&format!(
            "\nAnomaly scan: no class fired over {runs} analysed runs.\n"
        ));
        return s;
    }
    let total: usize = classes.iter().map(|c| c.count).collect::<Vec<_>>().iter().sum();
    s.push_str(&format!(
        "\nAnomaly scan — {} class(es), {total} finds over {runs} analysed runs:\n",
        classes.len()
    ));
    for c in classes {
        let seeds: Vec<String> = c.seeds.iter().take(4).map(|s| s.to_string()).collect();
        s.push_str(&format!(
            "  {:<18} {:>3} run(s) / {:>3} find(s)   seeds {}\n",
            c.kind.name(),
            c.runs,
            c.count,
            seeds.join(", ")
        ));
        s.push_str(&format!("      owner: {}\n", c.owner));
        for e in c.examples.iter().take(2) {
            s.push_str(&format!(
                "      e.g. seed {} L{} @{:.0}s: {}\n",
                e.seed,
                e.level + 1,
                e.at,
                e.detail
            ));
        }
    }
    s
}

fn secs(x: f32) -> String {
    if x >= 100.0 {
        format!("{x:.0}s")
    } else if x > 0.0 {
        format!("{x:.1}s")
    } else {
        "-".to_string()
    }
}

fn dist(p: &Percentiles) -> String {
    if p.n == 0 {
        "-".to_string()
    } else {
        format!(
            "{} / {} / {} / {} / {} (n={})",
            secs(p.min),
            secs(p.p25),
            secs(p.median),
            secs(p.p75),
            secs(p.max),
            p.n
        )
    }
}

fn damage_lines(damage: &[DamageAgg], indent: &str) -> String {
    let mut out = String::new();
    if damage.is_empty() {
        out.push_str(&format!("{indent}(no damage taken)\n"));
        return out;
    }
    let total: f32 = damage.iter().map(|d| d.total).sum();
    for d in damage.iter().take(12) {
        let share = if total > 0.0 { d.total / total } else { 0.0 };
        out.push_str(&format!(
            "{indent}{:<20} {:>8.0} total  {:>6.1}/run  {:>5}  ({:.0}%)\n",
            d.source,
            d.total,
            d.per_run,
            pct(share),
            share * 100.0
        ));
    }
    out
}

/// Per-weapon balance rows: damage share, boss share, pick rate and the win
/// correlation (runs/levels that took it versus runs/levels that did not).
fn weapon_lines(weapons: &[WeaponAgg], indent: &str, per_level: bool) -> String {
    let mut out = String::new();
    if weapons.iter().all(|w| w.dealt <= 0.0 && w.picks == 0) {
        return out;
    }
    let unit = if per_level { "cleared" } else { "won" };
    for w in weapons {
        out.push_str(&format!(
            "{indent}{:<14} dmg {:>8.0} {:>5}   boss {:>8.0} {:>5}   picked {:>4}   {unit} with {:>4} vs without {:>4}\n",
            w.weapon,
            w.dealt,
            pct(w.share),
            w.boss_dealt,
            pct(w.boss_share),
            pct(w.pick_rate),
            pct(w.win_rate_with),
            pct(w.win_rate_without),
        ));
    }
    out
}

/// A count distribution ("n / median / max", no seconds).
fn dist_count(p: &Percentiles) -> String {
    if p.n == 0 {
        "-".to_string()
    } else {
        format!(
            "{:.0} / {:.0} / {:.0} / {:.0} / {:.0} (n={})",
            p.min, p.p25, p.median, p.p75, p.max, p.n
        )
    }
}

/// One compact line of progression checkpoints.
fn progress_line(p: &[ProgressAgg]) -> String {
    if p.is_empty() {
        return "-".to_string();
    }
    p.iter()
        .map(|g| {
            format!(
                "@{:.0}s lv{:.1}/{} upgrades/{} kills",
                g.at, g.mean_player_level, g.mean_upgrades, g.mean_kills
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One compact line of death classifications.
fn notes_line(notes: &[(String, usize)]) -> String {
    notes
        .iter()
        .take(6)
        .map(|(n, c)| format!("{n} x{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The human-readable table the owner reads.
pub fn render_table(r: &BatchReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Rat Run bot batch — {} runs, skill {}, {}, start level {}, carry {}, seed {}\n",
        r.runs,
        r.skill,
        if r.practice {
            "single level (practice)"
        } else {
            "full campaign"
        },
        r.start_level + 1,
        r.carry,
        r.base_seed
    ));
    s.push_str(&format!(
        "campaign wins: {}/{} ({}, 95% CI {})   sim time: {:.0}s in {:.1}s wall on {} thread(s) = {:.1}x sim-seconds per wall-second per thread\n",
        r.campaign_wins,
        r.runs,
        pct(r.win_rate),
        ci(r.win_rate_ci),
        r.sim_seconds,
        r.wall_seconds,
        r.threads,
        r.sim_per_wall_second_per_thread
    ));
    s.push_str(&format!(
        "survival per run: {}   kills/min {:.0}   within {} px of a body {:.0}% of the time\n",
        dist(&r.survival),
        r.kills_per_min,
        super::NEAR_ENEMY_PX as u32,
        r.near_frac * 100.0
    ));
    s.push_str(&format!(
        "first level-up: {}   enemies alive at death: {}\n",
        dist(&r.first_levelup),
        dist_count(&r.enemies_at_death)
    ));
    s.push_str(&format!("progress: {}\n", progress_line(&r.progress)));
    // How much of a change is noise at this batch size. A 5-point move is the
    // unit the owner talks in; at n=30 almost none of it is resolvable.
    s.push_str(&format!(
        "sample size: a 95% interval half-width of 5 points needs ~{} runs at p=0.5; \
resolving a 5-point difference between two arms needs ~{} runs per arm (95% conf, 80% power)\n",
        runs_for_half_width(0.5, 0.05),
        runs_to_resolve(0.5, 0.05)
    ));
    if !r.death_notes.is_empty() {
        s.push_str(&format!("death causes: {}\n", notes_line(&r.death_notes)));
    }
    s.push('\n');
    for l in &r.levels {
        s.push_str(&format!(
            "Level {} — {}/{} cleared ({}, 95% CI {}), {} deaths, boss reached {} / killed {}\n",
            l.level + 1,
            l.cleared,
            l.attempted,
            pct(l.clear_rate),
            ci(l.clear_rate_ci),
            l.deaths,
            l.reached_boss,
            l.boss_kills
        ));
        s.push_str(&format!("  time on level:   {}\n", dist(&l.time)));
        s.push_str(&format!("  death clock:     {}\n", dist(&l.death_time)));
        s.push_str(&format!(
            "  boss fight:      {}   build dps at boss {:.0}  measured into boss {:.0}\n",
            dist(&l.boss_seconds),
            l.dps_at_boss,
            l.boss_dps
        ));
        s.push_str(&format!(
            "  player level at boss: {}\n",
            dist_count(&l.level_at_boss)
        ));
        s.push_str(&format!(
            "  kills/min:       {:.0}   first level-up: {}   near-enemy time: {:.0}%   enemies at death: {}\n",
            l.kills_per_min,
            dist(&l.first_levelup),
            l.near_frac * 100.0,
            dist_count(&l.enemies_at_death)
        ));
        if !l.progress.is_empty() {
            s.push_str(&format!("  progress:        {}\n", progress_line(&l.progress)));
        }
        if !l.death_notes.is_empty() {
            s.push_str(&format!("  death causes:    {}\n", notes_line(&l.death_notes)));
        }
        if !l.damage.is_empty() {
            s.push_str("  damage taken:\n");
            s.push_str(&damage_lines(&l.damage, "    "));
        }
        let wl = weapon_lines(&l.weapons, "    ", true);
        if !wl.is_empty() {
            s.push_str("  weapons (share / boss share / pick rate / clear rate with vs without):\n");
            s.push_str(&wl);
        }
        if !l.upgrades.is_empty() {
            let list: Vec<String> = l
                .upgrades
                .iter()
                .take(6)
                .map(|(u, n)| format!("{u} x{n}"))
                .collect();
            s.push_str(&format!("  top picks:       {}\n", list.join(", ")));
        }
        if !l.kept.is_empty() {
            let list: Vec<String> = l
                .kept
                .iter()
                .take(6)
                .map(|(u, n)| format!("{u} x{n}"))
                .collect();
            s.push_str(&format!("  carried on:      {}\n", list.join(", ")));
        }
        s.push('\n');
    }
    if !r.weapons.is_empty() {
        s.push_str("Weapons (all levels: damage share / boss share / pick rate / campaign win rate with vs without):\n");
        s.push_str(&weapon_lines(&r.weapons, "  ", false));
        s.push('\n');
    }
    s.push_str("Damage taken by source (all levels, per run):\n");
    s.push_str(&damage_lines(&r.damage, "  "));
    if !r.deaths.is_empty() {
        s.push_str("\nDeaths (level, clock, cause):\n");
        for d in r.deaths.iter().take(40) {
            s.push_str(&format!(
                "  L{} at {:<6} {}{}\n",
                d.level + 1,
                secs(d.at),
                d.cause,
                d.note
                    .as_ref()
                    .map(|n| format!("  — {n}"))
                    .unwrap_or_default()
            ));
        }
        if r.deaths.len() > 40 {
            s.push_str(&format!("  … {} more\n", r.deaths.len() - 40));
        }
    }
    if !r.builds.is_empty() {
        let list: Vec<String> = r
            .builds
            .iter()
            .take(10)
            .map(|(b, n)| format!("{b} x{n}"))
            .collect();
        s.push_str(&format!("\nCommon picks/carries: {}\n", list.join(", ")));
    }
    if r.analysis {
        s.push_str(&anomaly_section(&r.anomalies, r.runs));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(skill: Skill, runs: usize, seed: u64) -> BatchOpts {
        BatchOpts {
            runs,
            seed,
            start_level: 0,
            skill,
            max_sim_seconds: 40.0,
            threads: 2,
            json: false,
            practice: true,
            carry: CarryPreset::None,
            trace: false,
            analyze: false,
            cycle_skills: false,
            exact_seed: false,
            route: None,
        }
    }

    /// `--route` reaches the two side routes: the batch could only ever ask
    /// for the original route's floors (`--level 2` was always the Kitchen), so
    /// the Laundry and Printworks were unmeasurable.
    #[test]
    fn the_route_flag_plays_the_side_route_floor() {
        use crate::game::level::Theme;
        // Parsing.
        let args: Vec<String> = ["--bot-batch", "1", "--level", "2", "--practice", "--route", "laundry"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).expect("parses").expect("our flag");
        assert_eq!(opts.route, Some(Theme::Laundry));
        assert_eq!(parse_route("PRINTWORKS"), Ok(Theme::Printworks));
        assert!(parse_route("kitchenette").is_err());

        // The world the run builds is that floor.
        let mut practice = tiny(Skill::Expert, 1, 3);
        practice.start_level = 1;
        practice.route = Some(Theme::Laundry);
        let run = AutoRun::new(7, auto_opts(&practice));
        assert_eq!(
            run.world().level.theme,
            Theme::Laundry,
            "--route must build the side route's map"
        );
        practice.start_level = 2;
        practice.route = Some(Theme::Printworks);
        let run = AutoRun::new(7, auto_opts(&practice));
        assert_eq!(run.world().level.theme, Theme::Printworks);
    }

    #[test]
    fn identical_batches_give_identical_reports() {
        let a = run_batch_threaded(&tiny(Skill::Expert, 3, 77));
        let b = run_batch_threaded(&tiny(Skill::Expert, 3, 77));
        // Ignore the wall clock (the only intentionally varying field).
        let ja = serde_json::to_string(&a).expect("serialise");
        let jb = serde_json::to_string(&b).expect("serialise");
        let strip = |s: &str| {
            let mut v: serde_json::Value = serde_json::from_str(s).expect("json");
            if let Some(o) = v.as_object_mut() {
                o.remove("wall_seconds");
                o.remove("sim_per_wall_second_per_thread");
            }
            serde_json::to_string(&v).expect("json")
        };
        assert_eq!(strip(&ja), strip(&jb));
    }

    #[test]
    fn thread_count_does_not_change_the_report() {
        let mut a = tiny(Skill::Expert, 2, 9);
        let mut b = tiny(Skill::Expert, 2, 9);
        a.threads = 1;
        b.threads = 2;
        let ra = run_batch_threaded(&a);
        let rb = run_batch_threaded(&b);
        assert_eq!(ra.win_rate, rb.win_rate);
        assert_eq!(ra.levels.len(), rb.levels.len());
        for (la, lb) in ra.levels.iter().zip(&rb.levels) {
            assert_eq!(la.attempted, lb.attempted);
            assert_eq!(la.deaths, lb.deaths);
            assert_eq!(la.boss_kills, lb.boss_kills);
            assert!((la.boss_seconds.mean - lb.boss_seconds.mean).abs() < 1e-4);
        }
    }

    #[test]
    fn cli_parsing_reads_every_flag() {
        let args: Vec<String> = "--bot-batch 5 --seed 42 --level 3 --skill casual --max-sim-seconds 120 --threads 4 --json --practice"
            .split(' ')
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).expect("ok").expect("parsed");
        assert_eq!(opts.runs, 5);
        assert_eq!(opts.seed, 42);
        assert_eq!(opts.start_level, 2);
        assert_eq!(opts.skill, Skill::Casual);
        assert_eq!(opts.max_sim_seconds, 120.0);
        assert_eq!(opts.threads, 4);
        assert!(opts.json && opts.practice);
        assert!(!opts.analyze && !opts.cycle_skills);
        assert_eq!(opts.carry, CarryPreset::None);

        // `--carry` picks the measurement start; `--bot-compare` reads the
        // report back, so the field has to survive the round trip too.
        let args: Vec<String> = "--bot-batch 2 --level 2 --practice --carry typical"
            .split(' ')
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).expect("ok").expect("parsed");
        assert_eq!(opts.carry, CarryPreset::Typical);
        assert_eq!(opts.start_level, 1);
        assert!(opts.practice);
        // The preset is a real loadout, not a level-1 start.
        let lo = CarryPreset::Typical.loadout();
        assert_eq!(lo.weapons.len(), 3);
        assert!(lo.weapons.iter().all(|w| w.level == w.kind.max_level()));

        let bad: Vec<String> = "--bot-batch 2 --carry nope"
            .split(' ')
            .map(|s| s.to_string())
            .collect();
        assert!(parse_args(&bad).is_err());

        let args: Vec<String> = "--bot-batch 3 --analyze --cycle-skills"
            .split(' ')
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).expect("ok").expect("parsed");
        assert!(opts.analyze && opts.cycle_skills);
        assert_eq!(opts.skill, Skill::Expert, "--skill is the fixed default");
        let args: Vec<String> = "--bot-batch 2 --seed 123 --seed-exact"
            .split(' ')
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).expect("ok").expect("parsed");
        assert!(opts.exact_seed);
        assert_eq!(run_seed(&opts, 0), 123);
        assert_eq!(run_seed(&opts, 1), 124);
    }

    /// The Wilson interval and the sample-size helper agree with the textbook
    /// numbers (and stay inside 0..1 at the corners).
    #[test]
    fn wilson_intervals_and_sample_size_are_sane() {
        let [lo, hi] = wilson95(5, 10);
        assert!((lo - 0.236).abs() < 0.01, "low {lo}");
        assert!((hi - 0.764).abs() < 0.01, "high {hi}");
        let [lo, hi] = wilson95(0, 10);
        assert_eq!(lo, 0.0);
        assert!(hi > 0.2 && hi < 0.4, "0/10 high {hi}");
        let [lo, hi] = wilson95(10, 10);
        assert_eq!(hi, 1.0);
        assert!(lo < 0.8, "10/10 low {lo}");
        assert_eq!(wilson95(0, 0), [0.0, 1.0]);
        // 5 points at p = 0.5 is the classic two-proportion case.
        assert!((1400..1700).contains(&runs_to_resolve(0.5, 0.05)));
        assert!(runs_to_resolve(0.9, 0.05) < runs_to_resolve(0.5, 0.05));
        // A single-arm half-width of 5 points at p = 0.5 is 384.
        assert_eq!(runs_for_half_width(0.5, 0.05), 385);
    }

    /// The scan is read-only: an analysed run must be the same run.
    #[test]
    fn the_anomaly_scan_does_not_change_a_run() {
        let opts = tiny(Skill::Expert, 2, 21);
        for i in 0..2 {
            let seed = seed_for(opts.seed, i);
            let plain = run_one(seed, AutoOpts::practice(0, Skill::Expert), 40.0);
            let scanned = run_one_analyzed(seed, AutoOpts::practice(0, Skill::Expert), 40.0);
            let mut a = serde_json::to_value(&plain).expect("json");
            let mut b = serde_json::to_value(&scanned).expect("json");
            if let Some(o) = a.as_object_mut() {
                o.remove("anomalies");
            }
            if let Some(o) = b.as_object_mut() {
                o.remove("anomalies");
            }
            assert_eq!(a, b, "the scan changed run {i}");
        }
    }

    #[test]
    fn cli_without_our_flag_is_not_handled() {
        let args = vec!["--tiled".to_string()];
        assert!(parse_args(&args).expect("ok").is_none());
    }

    #[test]
    fn percentiles_are_sane() {
        let p = Percentiles::of(vec![5.0, 1.0, 3.0]);
        assert_eq!(p.n, 3);
        assert_eq!(p.min, 1.0);
        assert_eq!(p.median, 3.0);
        assert_eq!(p.max, 5.0);
        assert_eq!(p.mean, 3.0);
    }

    /// The tuning columns exist, survive a JSON round trip (the comparison
    /// mode reads them back) and the progression checkpoints fire.
    #[test]
    fn report_has_the_tuning_columns() {
        let mut opts = tiny(Skill::Expert, 2, 11);
        opts.max_sim_seconds = 80.0;
        let r = run_batch(&opts);
        let v = serde_json::to_value(&r).expect("json");
        for key in [
            "survival",
            "kills_per_min",
            "near_frac",
            "first_levelup",
            "enemies_at_death",
            "progress",
            "death_notes",
            "win_rate_ci",
            "analysis",
            "anomalies",
        ] {
            assert!(!v[key].is_null(), "report is missing {key}");
        }
        assert!(
            !r.levels[0].progress.is_empty(),
            "80 simulated seconds must cross the 60 s checkpoint"
        );
        // Round trip: `--bot-compare` reads exactly this shape.
        let back: BatchReport = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back.runs, r.runs);
        assert_eq!(back.levels.len(), r.levels.len());
        assert_eq!(back.skill, r.skill);
    }

    /// Comparing a report with itself must show zero everywhere.
    #[test]
    fn comparison_of_a_report_with_itself_is_all_zero() {
        let r = run_batch(&tiny(Skill::Skilled, 2, 5));
        let c = compare_reports(&r, &r, "a.json", "b.json");
        for row in &c.rows {
            assert_eq!(row.delta, 0.0, "{} did not diff to zero", row.metric);
        }
        for level in &c.levels {
            for row in &level.rows {
                assert_eq!(row.delta, 0.0, "L{} {}", level.level + 1, row.metric);
            }
        }
        let text = render_comparison(&c);
        assert!(text.contains("delta"));
        assert!(text.contains("overall"));
        assert!(text.contains("level 1"));
    }

    #[test]
    fn json_has_the_report_columns() {
        let r = run_batch(&tiny(Skill::Casual, 2, 3));
        let text = serde_json::to_string(&r).expect("serialise");
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["runs"], serde_json::json!(2));
        assert_eq!(v["skill"], serde_json::json!("casual"));
        assert!(v["levels"].is_array());
        assert!(v["damage"].is_array());
        assert!(v["deaths"].is_array());
    }
}
