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

use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;

use super::{run_to_end, AutoOpts, AutoRun, DeathInfo, LevelStats};
use crate::game::bot::Skill;
use crate::game::campaign;

/// The simulation step the batch uses. Fixed, so a run is reproducible.
pub const DT: f32 = 1.0 / 60.0;
/// Default cap on simulated time per run (seconds).
pub const DEFAULT_MAX_SIM_SECONDS: f32 = 1500.0;

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
    /// Print a per-5-simulated-second trace of the first run (diagnostics).
    pub trace: bool,
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
            trace: false,
        }
    }
}

/// One run's outcome (what the aggregation sees).
#[derive(Clone, Debug, Serialize)]
pub struct RunOutcome {
    pub seed: u64,
    pub won: bool,
    pub death: Option<DeathOutcome>,
    pub total_time: f32,
    pub total_score: u32,
    pub total_kills: u32,
    pub levels: Vec<LevelOutcome>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DeathOutcome {
    pub level: usize,
    pub at: f32,
    pub cause: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct LevelOutcome {
    pub level: usize,
    pub time: f32,
    pub cleared: bool,
    pub reached_boss: bool,
    pub boss_seconds: f32,
    pub boss_defeated: bool,
    pub damage_total: f32,
    pub damage: BTreeMap<&'static str, f32>,
    pub dps_at_boss: f32,
    pub boss_dps: f32,
    pub upgrades: Vec<String>,
    pub kept: Vec<String>,
    pub death: Option<DeathOutcome>,
}

impl LevelOutcome {
    fn from_stats(s: &LevelStats) -> LevelOutcome {
        LevelOutcome {
            level: s.level,
            time: s.time,
            cleared: s.boss_defeated,
            reached_boss: s.reached_boss,
            boss_seconds: s.boss_seconds,
            boss_defeated: s.boss_defeated,
            damage_total: s.damage_total,
            damage: s.damage.clone(),
            dps_at_boss: s.dps_at_boss,
            boss_dps: s.boss_dps,
            upgrades: s.upgrades.clone(),
            kept: s.kept.clone(),
            death: s.death.map(death_outcome),
        }
    }
}

fn death_outcome(d: DeathInfo) -> DeathOutcome {
    DeathOutcome {
        level: d.level,
        at: d.at,
        cause: d.cause.name(),
    }
}

/// Everything the report prints.
#[derive(Clone, Debug, Serialize)]
pub struct BatchReport {
    pub skill: &'static str,
    pub practice: bool,
    pub start_level: usize,
    pub base_seed: u64,
    pub runs: usize,
    pub campaign_wins: usize,
    pub win_rate: f32,
    pub sim_seconds: f64,
    pub wall_seconds: f64,
    pub threads: usize,
    pub sim_per_wall_second_per_thread: f64,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    pub seed: u64,
    pub won: bool,
    /// Levels cleared (0-based count) — a campaign win clears 3.
    pub levels_cleared: usize,
    pub total_time: f32,
    pub death: Option<DeathOutcome>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LevelAgg {
    pub level: usize,
    pub attempted: usize,
    pub cleared: usize,
    pub deaths: usize,
    pub reached_boss: usize,
    pub boss_kills: usize,
    pub clear_rate: f32,
    /// Death clock on this level: min / p25 / median / p75 / max.
    pub death_time: Percentiles,
    /// Time on the level (all attempted runs).
    pub time: Percentiles,
    /// Boss fight length in seconds, for the runs that reached a boss.
    pub boss_seconds: Percentiles,
    /// Build DPS when the boss appeared.
    pub dps_at_boss: f32,
    /// Measured DPS into the boss.
    pub boss_dps: f32,
    pub damage: Vec<DamageAgg>,
    pub upgrades: Vec<(String, usize)>,
    pub kept: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct DamageAgg {
    pub source: &'static str,
    /// Summed over all runs.
    pub total: f32,
    /// Mean per run that took damage from it.
    pub mean: f32,
    /// Mean per attempted run (so it reads as "damage per run").
    pub per_run: f32,
    pub hits: usize,
}

// ---- Running -------------------------------------------------------------

/// Play one run to its end (or the time cap).
pub fn run_one(seed: u64, opts: AutoOpts, max_sim_seconds: f32) -> RunOutcome {
    let mut run = AutoRun::new(seed, opts);
    run_to_end(&mut run, DT, max_sim_seconds);
    let _ = &run;
    let stats = run.stats();
    RunOutcome {
        seed,
        won: stats.won,
        death: stats.death.map(death_outcome),
        total_time: stats.total_time,
        total_score: stats.total_score,
        total_kills: stats.total_kills,
        levels: stats.levels.iter().map(LevelOutcome::from_stats).collect(),
    }
}

/// Run a whole batch (sequentially). `run_batch_threaded` is the fast entry.
pub fn run_batch(opts: &BatchOpts) -> BatchReport {
    let started = Instant::now();
    let mut outcomes = Vec::with_capacity(opts.runs);
    for i in 0..opts.runs {
        outcomes.push(run_one(seed_for(opts.seed, i), auto_opts(opts), opts.max_sim_seconds));
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
            *slot = Some(run_one(
                seed_for(opts.seed, i),
                auto_opts(opts),
                opts.max_sim_seconds,
            ));
        }
    } else {
        let auto = auto_opts(opts);
        let max_sim = opts.max_sim_seconds;
        let base = opts.seed;
        let runs = opts.runs;
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(threads);
            for t in 0..threads {
                handles.push(scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut i = t;
                    while i < runs {
                        out.push((i, run_one(seed_for(base, i), auto, max_sim)));
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
        .map(|(i, o)| o.unwrap_or_else(|| run_one(seed_for(opts.seed, i), auto_opts(opts), 0.0)))
        .collect();
    aggregate(opts, outcomes, wall)
}

/// Single-threaded batch that traces the first run (diagnostics only; the
/// aggregate is still the same report).
pub fn run_batch_traced(opts: &BatchOpts) -> BatchReport {
    let started = Instant::now();
    let mut outcomes = Vec::with_capacity(opts.runs);
    for i in 0..opts.runs {
        let seed = seed_for(opts.seed, i);
        let mut run = AutoRun::new(seed, auto_opts(opts));
        let mut next_trace = 0.0f32;
        let mut simulated = 0.0f32;
        while !run.finished() && simulated < opts.max_sim_seconds {
            run.step(DT);
            simulated += DT;
            if i == 0 && run.world().elapsed >= next_trace {
                eprintln!("{}", run.trace_line());
                next_trace += 2.0;
            }
        }
        let stats = run.stats();
        outcomes.push(RunOutcome {
            seed,
            won: stats.won,
            death: stats.death.map(death_outcome),
            total_time: stats.total_time,
            total_score: stats.total_score,
            total_kills: stats.total_kills,
            levels: stats.levels.iter().map(LevelOutcome::from_stats).collect(),
        });
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
    }
}

/// Fold per-run outcomes into the report. `outcomes` is in run order.
fn aggregate(opts: &BatchOpts, outcomes: Vec<RunOutcome>, wall_seconds: f64) -> BatchReport {
    let runs = outcomes.len();
    let mut levels: Vec<LevelAgg> = Vec::new();
    let mut damage: BTreeMap<&'static str, (f32, usize)> = BTreeMap::new();
    let mut deaths: Vec<DeathOutcome> = Vec::new();
    let mut builds: BTreeMap<String, usize> = BTreeMap::new();
    let mut campaign_wins = 0;
    let mut sim_seconds = 0.0f64;
    let mut run_summaries: Vec<RunSummary> = Vec::with_capacity(outcomes.len());

    for outcome in &outcomes {
        if outcome.won {
            campaign_wins += 1;
        }
        sim_seconds += outcome.total_time as f64;
        if let Some(d) = outcome.death {
            deaths.push(d);
        }
        run_summaries.push(RunSummary {
            seed: outcome.seed,
            won: outcome.won,
            levels_cleared: outcome.levels.iter().filter(|l| l.cleared).count(),
            total_time: outcome.total_time,
            death: outcome.death,
        });
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
                        death_time: Percentiles::default(),
                        time: Percentiles::default(),
                        boss_seconds: Percentiles::default(),
                        dps_at_boss: 0.0,
                        boss_dps: 0.0,
                        damage: Vec::new(),
                        upgrades: Vec::new(),
                        kept: Vec::new(),
                    });
                    levels.last_mut().expect("just pushed")
                }
            };
            // Scratch accumulators live in side vectors to keep the aggregate
            // struct simple; see the recompute below.
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
            for (src, amount) in &level.damage {
                let entry = damage.entry(src).or_insert((0.0, 0));
                entry.0 += *amount;
                entry.1 += 1;
            }
            for up in &level.upgrades {
                *builds.entry(up.clone()).or_insert(0) += 1;
            }
            if let Some(death) = level.death {
                let _ = death;
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
        let mut dmg: BTreeMap<&'static str, (f32, usize)> = BTreeMap::new();
        let mut upgrades: BTreeMap<String, usize> = BTreeMap::new();
        let mut kept: BTreeMap<String, usize> = BTreeMap::new();
        for outcome in &outcomes {
            for level in outcome.levels.iter().filter(|l| l.level == agg.level) {
                times.push(level.time);
                if let Some(d) = level.death {
                    death_times.push(d.at);
                }
                if level.reached_boss {
                    boss_seconds.push(level.boss_seconds);
                    dps.push(level.dps_at_boss);
                    boss_dps.push(level.boss_dps);
                }
                for (src, amount) in &level.damage {
                    let e = dmg.entry(src).or_insert((0.0, 0));
                    e.0 += *amount;
                    e.1 += 1;
                }
                for u in &level.upgrades {
                    *upgrades.entry(u.clone()).or_insert(0) += 1;
                }
                for k in &level.kept {
                    *kept.entry(k.clone()).or_insert(0) += 1;
                }
            }
        }
        agg.clear_rate = if agg.attempted == 0 {
            0.0
        } else {
            agg.cleared as f32 / agg.attempted as f32
        };
        agg.death_time = Percentiles::of(death_times);
        agg.time = Percentiles::of(times);
        agg.boss_seconds = Percentiles::of(boss_seconds);
        agg.dps_at_boss = mean(&dps);
        agg.boss_dps = mean(&boss_dps);
        agg.damage = damage_vec(dmg, agg.attempted);
        agg.upgrades = top(upgrades, 12);
        agg.kept = top(kept, 12);
    }
    levels.sort_by_key(|l| l.level);
    deaths.sort_by(|a, b| {
        a.level
            .cmp(&b.level)
            .then(a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal))
    });

    let threads = opts.threads.max(1);
    BatchReport {
        skill: opts.skill.name(),
        practice: opts.practice,
        start_level: opts.start_level,
        base_seed: opts.seed,
        runs,
        campaign_wins,
        win_rate: if runs == 0 {
            0.0
        } else {
            campaign_wins as f32 / runs as f32
        },
        sim_seconds,
        wall_seconds,
        threads,
        sim_per_wall_second_per_thread: if wall_seconds > 0.0 {
            sim_seconds / wall_seconds / threads as f64
        } else {
            0.0
        },
        levels,
        damage: damage_vec(damage, runs),
        deaths,
        run_summaries,
        builds: top(builds, 16),
    }
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn damage_vec(map: BTreeMap<&'static str, (f32, usize)>, runs: usize) -> Vec<DamageAgg> {
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
            .then(a.source.cmp(b.source))
    });
    out
}

fn top(map: BTreeMap<String, usize>, n: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

// ---- CLI -----------------------------------------------------------------

/// Handle `--bot-batch …`. Returns `None` when the flag is absent (so the real
/// game starts normally), or the process exit code.
pub fn maybe_run_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
[--skill expert|casual] [--max-sim-seconds <T>] [--threads <K>] [--json]";

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
                if !(1..=campaign::LEVEL_COUNT).contains(&level) {
                    return Err(format!("level must be 1-{}", campaign::LEVEL_COUNT));
                }
                opts.start_level = level - 1;
            }
            "--practice" => opts.practice = true,
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

/// The human-readable table the owner reads.
pub fn render_table(r: &BatchReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Rat Run bot batch — {} runs, skill {}, {}, start level {}, seed {}\n",
        r.runs,
        r.skill,
        if r.practice {
            "single level (practice)"
        } else {
            "full campaign"
        },
        r.start_level + 1,
        r.base_seed
    ));
    s.push_str(&format!(
        "campaign wins: {}/{} ({})   sim time: {:.0}s in {:.1}s wall on {} thread(s) = {:.1}x sim-seconds per wall-second per thread\n",
        r.campaign_wins,
        r.runs,
        pct(r.win_rate),
        r.sim_seconds,
        r.wall_seconds,
        r.threads,
        r.sim_per_wall_second_per_thread
    ));
    s.push('\n');
    for l in &r.levels {
        s.push_str(&format!(
            "Level {} — {}/{} cleared ({}), {} deaths, boss reached {} / killed {}\n",
            l.level + 1,
            l.cleared,
            l.attempted,
            pct(l.clear_rate),
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
        if !l.damage.is_empty() {
            s.push_str("  damage taken:\n");
            s.push_str(&damage_lines(&l.damage, "    "));
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
    s.push_str("Damage taken by source (all levels, per run):\n");
    s.push_str(&damage_lines(&r.damage, "  "));
    if !r.deaths.is_empty() {
        s.push_str("\nDeaths (level, clock, cause):\n");
        for d in r.deaths.iter().take(40) {
            s.push_str(&format!(
                "  L{} at {:<6} {}\n",
                d.level + 1,
                secs(d.at),
                d.cause
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
            trace: false,
        }
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
