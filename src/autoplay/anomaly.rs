//! Automatic anomaly scan for self-playing batches (`--bot-batch … --analyze`).
//!
//! The worst balance defect of the first tuning pass (three stranded bosses,
//! `LOG.md` step 5) was found by hand, by tracing one tournament and reading
//! the fight clock. This module is the automated version of that hunt: it
//! watches every tick of every analysed run and reports the *silences* that
//! should be impossible — nothing hurting the player for a minute, nothing the
//! player fires landing for half a minute, a boss that takes no damage or does
//! not move, a rat frozen against geometry, an enemy that survives at arm's
//! length untouched, a map the rat never leaves.
//!
//! ## What it records, and how
//!
//! Detection is deliberately geometric rather than model-based: every class is
//! a window of time the simulation should not produce, so a new class can be
//! added without knowing which subsystem is at fault. The scan is split in two
//! cadences so a 200-campaign run stays cheap:
//!
//! * **Every tick** (O(1)): the player's HP, whose drop is the "something
//!   hurt the rat" signal.
//! * **Every sample** (`SAMPLE_SECS` = 0.1 s, one O(entities) sweep): enemy
//!   count / total HP / near bodies, the boss's HP and position, the
//!   unreachable-enemy table, and the stuck/roam geometry (so the stuck
//!   window is exactly `STUCK_SECS`, not 50 ticks).
//!
//! Damage dealt is read as a *decrease in the enemies' summed HP*: spawning
//! raises it, and the only other way it falls is damage (a kill included; a
//! horde break despawn is the one false positive, which can only hide a real
//! silence, never invent one).
//!
//! ## What it does not do
//!
//! It never writes to the simulation and never touches the bot, so an analysed
//! batch is the same batch as an unanalysed one (only slower). Finding an
//! anomaly is a routing hint, not a diagnosis: the `owner` string on each
//! class says which subsystem to read first.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::game::ecs::systems::boss::Boss;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::world::World;
use crate::math::Vec2;

// ---- Thresholds ---------------------------------------------------------
//
// Every one of these is "obviously wrong" rather than "suspicious": a run may
// be calm, and a bot may kite a pack for a while. These are the windows that
// only a defect (or a plainly broken bot) produces.

/// Nothing damaged the player for longer than this (s).
pub const NO_PLAYER_DAMAGE_SECS: f32 = 60.0;
/// The player dealt no damage for longer than this while bodies were alive (s).
pub const NO_DAMAGE_DEALT_SECS: f32 = 30.0;
/// A live boss took no damage for longer than this (s).
pub const BOSS_NO_DAMAGE_SECS: f32 = 20.0;
/// A live boss did not move for longer than this (s), or never moved at all.
pub const BOSS_STILL_SECS: f32 = 20.0;
/// The rat's position stayed inside this spread for `STUCK_SECS` (px,
/// maximum deviation from the window's mean) while bodies were alive.
pub const STUCK_SPREAD_PX: f32 = 8.0;
/// The window the stuck test looks at (s).
pub const STUCK_SECS: f32 = 5.0;
/// An enemy this close to the rat (px) that survived this long untouched is
/// reported as possibly unreachable. 60 px is inside every weapon's range and
/// inside the distance at which a body starts pushing, so a live body here for
/// 20 s without losing HP is either behind geometry or not being hit by the
/// auto-aim at all.
pub const UNREACHABLE_PX: f32 = 60.0;
/// …for this long (s).
pub const UNREACHABLE_SECS: f32 = 20.0;
/// …and only while damage is still flowing (the player dealt damage within
/// this many seconds), so a rat that is simply not firing is not blamed on the
/// enemy.
pub const UNREACHABLE_FIRE_WINDOW: f32 = 5.0;
/// The rat's roam over a level shorter than this (px, longer axis) means it
/// never left the start area.
pub const START_AREA_PX: f32 = 80.0;
/// A level that lasts this many times the batch median is flagged.
pub const LEVEL_MEDIAN_FACTOR: f32 = 2.0;
/// A player level or DPS this many times the batch median is flagged.
pub const OUTLIER_FACTOR: f32 = 3.0;
/// Entities are sampled this often (s).
pub const SAMPLE_SECS: f32 = 0.1;
/// Cap on anomalies recorded per level (a broken level must not flood a report).
const MAX_PER_LEVEL: usize = 24;
/// Cap on unreachable-enemy reports per level.
const MAX_UNREACHABLE_PER_LEVEL: usize = 2;

/// One anomaly class. Serialised `snake_case` so the JSON names are stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    /// Nothing hurt the player for `NO_PLAYER_DAMAGE_SECS`.
    NoPlayerDamage,
    /// The player dealt no damage for `NO_DAMAGE_DEALT_SECS` with bodies alive.
    NoDamageDealt,
    /// A live boss took no damage for `BOSS_NO_DAMAGE_SECS` (the stranded-boss
    /// signature).
    BossNoDamage,
    /// A live boss did not move for `BOSS_STILL_SECS`, or never moved at all.
    BossStill,
    /// The rat's position hardly changed for `STUCK_SECS` with bodies alive.
    Stuck,
    /// An enemy within `UNREACHABLE_PX` survived `UNREACHABLE_SECS` untouched.
    EnemyUnreachable,
    /// The rat's roam over the level stayed inside `START_AREA_PX`.
    NeverLeftStart,
    /// Batch-level: the level lasted `LEVEL_MEDIAN_FACTOR` x the batch median.
    LongLevel,
    /// Batch-level: peak player level over `OUTLIER_FACTOR` x the batch median.
    LevelOutlier,
    /// Batch-level: peak build DPS over `OUTLIER_FACTOR` x the batch median.
    DpsOutlier,
}

impl AnomalyKind {
    /// Stable machine name (the JSON value).
    pub fn name(self) -> &'static str {
        match self {
            AnomalyKind::NoPlayerDamage => "no_player_damage",
            AnomalyKind::NoDamageDealt => "no_damage_dealt",
            AnomalyKind::BossNoDamage => "boss_no_damage",
            AnomalyKind::BossStill => "boss_still",
            AnomalyKind::Stuck => "stuck",
            AnomalyKind::EnemyUnreachable => "enemy_unreachable",
            AnomalyKind::NeverLeftStart => "never_left_start",
            AnomalyKind::LongLevel => "long_level",
            AnomalyKind::LevelOutlier => "level_outlier",
            AnomalyKind::DpsOutlier => "dps_outlier",
        }
    }

    /// The subsystem to read first when this class fires (a routing hint, not a
    /// diagnosis).
    pub fn owner(self) -> &'static str {
        match self {
            AnomalyKind::NoPlayerDamage => {
                "director.rs / systems/spawn.rs (no pressure) or bot.rs (kiting)"
            }
            AnomalyKind::NoDamageDealt => "systems/weapons.rs aiming / systems/combat.rs hit test",
            AnomalyKind::BossNoDamage => "systems/boss/mod.rs (rejoin rule, state machine)",
            AnomalyKind::BossStill => "systems/boss/mod.rs (state machine)",
            AnomalyKind::Stuck => "bot.rs steering / game/collision.rs wall resolve",
            AnomalyKind::EnemyUnreachable => "game/nav.rs flow field / systems/spawn.rs ring",
            AnomalyKind::NeverLeftStart => "bot.rs waypoint plan / game/levelgen",
            AnomalyKind::LongLevel => "systems/boss/mod.rs (fight length) / level.rs",
            AnomalyKind::LevelOutlier => "game/player.rs XP curve / game/loadout.rs",
            AnomalyKind::DpsOutlier => "game/weapon.rs / game/loadout.rs",
        }
    }

    /// Every class, in report order.
    pub fn all() -> [AnomalyKind; 10] {
        [
            AnomalyKind::NoPlayerDamage,
            AnomalyKind::NoDamageDealt,
            AnomalyKind::BossNoDamage,
            AnomalyKind::BossStill,
            AnomalyKind::Stuck,
            AnomalyKind::EnemyUnreachable,
            AnomalyKind::NeverLeftStart,
            AnomalyKind::LongLevel,
            AnomalyKind::LevelOutlier,
            AnomalyKind::DpsOutlier,
        ]
    }
}

/// One detection: what, where, when, and how bad.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    /// The run's seed (so the run can be replayed on its own).
    pub seed: u64,
    /// 0-based campaign level.
    pub level: usize,
    /// Level-clock second the offending window opened (or the level length,
    /// for the batch-level classes).
    pub at: f32,
    /// Length of the offending window (s); 0 for point-in-time classes.
    pub seconds: f32,
    /// Human-readable detail.
    pub detail: String,
}

/// Per-level scratch for one run. Cheap: one O(1) update per tick, one
/// O(entities) sweep per `SAMPLE_SECS`.
pub struct AnomalyScan {
    seed: u64,
    /// Level-clock seconds observed on the current level.
    t: f32,
    /// Seconds since the last entity sample.
    since_sample: f32,
    level: Option<usize>,
    out: Vec<Anomaly>,

    // No HP lost / no damage dealt windows.
    /// Last observed player HP (a drop is a hit).
    last_hp: f32,
    hurt_gap: f32,
    hurt_start: f32,
    hurt_worst: (f32, f32),
    dealt_gap: f32,
    dealt_start: f32,
    dealt_worst: (f32, f32),
    dealt_bodies: usize,
    prev_enemy_hp: f32,
    prev_kills: u32,
    /// Level clock of the last tick the player dealt damage.
    last_dealt_at: f32,

    // Boss window.
    boss_alive: bool,
    boss_hp: f32,
    boss_pos: Vec2,
    boss_fight: f32,
    boss_idle: f32,
    boss_idle_start: f32,
    boss_idle_worst: (f32, f32),
    boss_still: f32,
    boss_still_start: f32,
    boss_still_worst: (f32, f32),
    boss_moved: f32,
    /// Displacement accumulated in the current 1 s "did it move" window.
    boss_move_acc: f32,
    boss_move_clock: f32,

    // Stuck window: the last STUCK_SECS of positions.
    pos_ring: Vec<Vec2>,
    pos_next: usize,
    stuck_run: f32,
    stuck_start: f32,
    stuck_worst: (f32, f32, f32),

    // Roam.
    roam_min: Vec2,
    roam_max: Vec2,
    roam_origin: Vec2,

    // Enemies close to the rat and not yet damaged, by `Enemy::id`.
    close: HashMap<u32, CloseEnemy>,
    unreachable_reports: usize,
    /// Monotonic sample counter (the close-enemy liveness stamp).
    sample_id: u64,
}

#[derive(Clone, Copy)]
struct CloseEnemy {
    /// HP when it was last seen to take damage.
    hp: f32,
    /// Level clock when it was last seen taking damage / coming into range.
    since: f32,
    /// The sample stamp of its last sighting (cheap "still in range" test).
    seen: u64,
}

impl AnomalyScan {
    pub fn new(seed: u64) -> AnomalyScan {
        AnomalyScan {
            seed,
            t: 0.0,
            since_sample: SAMPLE_SECS,
            level: None,
            out: Vec::new(),
            last_hp: f32::INFINITY,
            hurt_gap: 0.0,
            hurt_start: 0.0,
            hurt_worst: (0.0, 0.0),
            dealt_gap: 0.0,
            dealt_start: 0.0,
            dealt_worst: (0.0, 0.0),
            dealt_bodies: 0,
            prev_enemy_hp: 0.0,
            prev_kills: 0,
            last_dealt_at: -1.0e9,
            boss_alive: false,
            boss_hp: 0.0,
            boss_pos: Vec2::ZERO,
            boss_fight: 0.0,
            boss_idle: 0.0,
            boss_idle_start: 0.0,
            boss_idle_worst: (0.0, 0.0),
            boss_still: 0.0,
            boss_still_start: 0.0,
            boss_still_worst: (0.0, 0.0),
            boss_moved: 0.0,
            boss_move_acc: 0.0,
            boss_move_clock: 0.0,
            pos_ring: Vec::with_capacity(STUCK_SAMPLES),
            pos_next: 0,
            stuck_run: 0.0,
            stuck_start: 0.0,
            stuck_worst: (0.0, 0.0, 0.0),
            roam_min: Vec2::ZERO,
            roam_max: Vec2::ZERO,
            roam_origin: Vec2::ZERO,
            close: HashMap::new(),
            unreachable_reports: 0,
            sample_id: 0,
        }
    }

    /// Observe one simulated tick. `dt` is the step the run took.
    pub fn tick(&mut self, w: &World, dt: f32) {
        self.start_level_if_needed(w);
        self.t += dt;

        // Every tick: the player's HP (a hit is the one O(1) signal).
        self.track_hp(w.player.hp, dt);

        // Every sample: the O(entities) sweep and the geometry windows. The
        // stuck ring is sampled at this cadence too, so its window is exactly
        // `STUCK_SECS` and not 50 ticks.
        self.since_sample += dt;
        if self.since_sample + 1e-6 >= SAMPLE_SECS {
            let step = self.since_sample;
            self.since_sample = 0.0;
            self.track_position(w.player.pos);
            self.sample(w, step);
        }
    }

    /// The run ended (or the batch cap cut it): close every open window.
    /// Returns the anomalies of every level the run touched.
    pub fn finish(mut self) -> Vec<Anomaly> {
        self.flush_level();
        self.out
    }

    // ---- Internals -------------------------------------------------------

    fn start_level_if_needed(&mut self, w: &World) {
        let level = w.level_index;
        if self.level == Some(level) {
            return;
        }
        self.flush_level();
        self.level = Some(level);
        self.t = w.elapsed;
        self.hurt_gap = 0.0;
        self.hurt_start = w.elapsed;
        self.hurt_worst = (0.0, 0.0);
        self.dealt_gap = 0.0;
        self.dealt_start = w.elapsed;
        self.dealt_worst = (0.0, 0.0);
        self.dealt_bodies = 0;
        self.prev_enemy_hp = 0.0;
        self.prev_kills = w.kills;
        self.last_dealt_at = -1.0e9;
        self.last_hp = w.player.hp;
        self.boss_alive = false;
        self.boss_fight = 0.0;
        self.boss_idle = 0.0;
        self.boss_idle_worst = (0.0, 0.0);
        self.boss_still = 0.0;
        self.boss_still_worst = (0.0, 0.0);
        self.boss_moved = 0.0;
        self.boss_move_acc = 0.0;
        self.boss_move_clock = 0.0;
        self.pos_ring.clear();
        self.pos_next = 0;
        self.stuck_run = 0.0;
        self.stuck_worst = (0.0, 0.0, 0.0);
        self.roam_origin = w.player.pos;
        self.roam_min = w.player.pos;
        self.roam_max = w.player.pos;
        self.close.clear();
        self.unreachable_reports = 0;
    }

    fn push(&mut self, kind: AnomalyKind, at: f32, seconds: f32, detail: String) {
        let level_anomalies = self
            .out
            .iter()
            .filter(|a| Some(a.level) == self.level)
            .count();
        if level_anomalies >= MAX_PER_LEVEL {
            return;
        }
        self.out.push(Anomaly {
            kind,
            seed: self.seed,
            level: self.level.unwrap_or(0),
            at,
            seconds,
            detail,
        });
    }

    fn track_hp(&mut self, hp: f32, dt: f32) {
        // A hit is the only thing that lowers HP fast (regen is 0.5-3 HP/s, so
        // a single tick cannot hide one).
        let hurt = hp < self.last_hp - 1e-3;
        if hurt {
            let len = self.t - self.hurt_start;
            if len > self.hurt_worst.1 {
                self.hurt_worst = (self.hurt_start, len);
            }
            self.hurt_start = self.t;
            self.hurt_gap = 0.0;
        } else {
            self.hurt_gap += dt;
        }
        self.last_hp = hp;
    }

    fn track_position(&mut self, p: Vec2) {
        if p.x < self.roam_min.x {
            self.roam_min.x = p.x;
        }
        if p.y < self.roam_min.y {
            self.roam_min.y = p.y;
        }
        if p.x > self.roam_max.x {
            self.roam_max.x = p.x;
        }
        if p.y > self.roam_max.y {
            self.roam_max.y = p.y;
        }
        if self.pos_ring.len() < STUCK_SAMPLES {
            self.pos_ring.push(p);
        } else {
            self.pos_ring[self.pos_next] = p;
            self.pos_next = (self.pos_next + 1) % STUCK_SAMPLES;
        }
    }

    /// The O(entities) half of the scan.
    fn sample(&mut self, w: &World, step: f32) {
        let p = w.player.pos;
        let mut bodies = 0usize;
        let mut hp_sum = 0.0f32;
        let mut boss: Option<(f32, f32, Vec2, bool)> = None; // hp, max, pos, moved
        // Only the enemies still close to the rat stay in the table.
        self.sample_id += 1;
        let stamp = self.sample_id;
        for entity in w.ecs.iter_entities() {
            if let Some(e) = entity.get::<Enemy>() {
                if !e.alive() {
                    continue;
                }
                bodies += 1;
                hp_sum += e.hp;
                if e.kind == EnemyKind::Boss {
                    boss = boss.or(Some((e.hp, e.max_hp, e.pos, true)));
                }
                if e.pos.dist(p) <= UNREACHABLE_PX {
                    // The borrow of the table ends before the report is filed.
                    let mut report: Option<(f32, String)> = None;
                    match self.close.get_mut(&e.id) {
                        Some(entry) => {
                            entry.seen = stamp;
                            if e.hp < entry.hp - 1e-3 {
                                // It is taking damage: restart its clock.
                                entry.hp = e.hp;
                                entry.since = self.t;
                            } else if self.t - entry.since >= UNREACHABLE_SECS
                                && self.t - self.last_dealt_at <= UNREACHABLE_FIRE_WINDOW
                                && self.unreachable_reports < MAX_UNREACHABLE_PER_LEVEL
                            {
                                let seconds = self.t - entry.since;
                                let kind = format!("{:?}", e.kind);
                                report = Some((
                                    entry.since,
                                    format!(
                                        "{kind} alive {seconds:.0} s within {UNREACHABLE_PX:.0} px, hp {:.1}/{:.1} untouched while {bodies} bodies alive",
                                        e.hp, e.max_hp
                                    ),
                                ));
                                // Do not report it again for this enemy.
                                entry.since = f32::INFINITY;
                                entry.hp = e.hp;
                            }
                        }
                        None => {
                            self.close.insert(
                                e.id,
                                CloseEnemy {
                                    hp: e.hp,
                                    since: self.t,
                                    seen: stamp,
                                },
                            );
                        }
                    }
                    if let Some((since, detail)) = report {
                        self.unreachable_reports += 1;
                        self.push(
                            AnomalyKind::EnemyUnreachable,
                            since,
                            self.t - since,
                            detail,
                        );
                    }
                }
            } else if let Some(b) = entity.get::<Boss>() {
                if !b.dead {
                    boss = Some((b.hp, b.max_hp, b.pos, true));
                }
            }
        }
        self.close.retain(|_, e| e.seen == stamp);

        // ---- Damage dealt (an HP-sum decrease with bodies alive) ----------
        let dealt = bodies > 0
            && (hp_sum < self.prev_enemy_hp - 1e-3 || w.kills != self.prev_kills);
        if dealt {
            self.last_dealt_at = self.t;
            let len = self.t - self.dealt_start;
            if len > self.dealt_worst.1 {
                self.dealt_worst = (self.dealt_start, len);
            }
            self.dealt_start = self.t;
            self.dealt_gap = 0.0;
        } else if bodies > 0 {
            if self.dealt_gap <= 0.0 {
                self.dealt_bodies = bodies;
            }
            self.dealt_gap += step;
        } else {
            // No bodies: nothing could be dealt, so the window is not running.
            self.dealt_start = self.t;
            self.dealt_gap = 0.0;
        }
        self.prev_enemy_hp = hp_sum;
        self.prev_kills = w.kills;

        // ---- Boss windows -------------------------------------------------
        match boss {
            Some((hp, _max, pos, _)) => {
                if !self.boss_alive {
                    self.boss_alive = true;
                    self.boss_hp = hp;
                    self.boss_pos = pos;
                    self.boss_fight = 0.0;
                    self.boss_idle = 0.0;
                    self.boss_idle_start = self.t;
                    self.boss_still = 0.0;
                    self.boss_still_start = self.t;
                    self.boss_moved = 0.0;
                    self.boss_move_acc = 0.0;
                    self.boss_move_clock = 0.0;
                } else {
                    self.boss_fight += step;
                    if hp < self.boss_hp - 1e-3 {
                        let len = self.t - self.boss_idle_start;
                        if len > self.boss_idle_worst.1 {
                            self.boss_idle_worst = (self.boss_idle_start, len);
                        }
                        self.boss_idle_start = self.t;
                        self.boss_idle = 0.0;
                    } else {
                        self.boss_idle += step;
                    }
                    let moved = pos.dist(self.boss_pos);
                    self.boss_moved += moved;
                    // "Did not move" is judged on whole seconds: a slow walk
                    // (< 8 px/s) is still a boss that is not repositioning,
                    // but 2 px per sample would call a 15 px/s stalker frozen.
                    self.boss_move_acc += moved;
                    self.boss_move_clock += step;
                    if self.boss_move_clock >= 1.0 {
                        if self.boss_move_acc < 8.0 {
                            if self.boss_still <= 0.0 {
                                self.boss_still_start = self.t - 1.0;
                            }
                            self.boss_still += 1.0;
                        } else {
                            let len = self.t - self.boss_still_start;
                            if len > self.boss_still_worst.1 {
                                self.boss_still_worst = (self.boss_still_start, len);
                            }
                            self.boss_still_start = self.t;
                            self.boss_still = 0.0;
                        }
                        self.boss_move_acc = 0.0;
                        self.boss_move_clock = 0.0;
                    }
                    self.boss_hp = hp;
                    self.boss_pos = pos;
                }
            }
            None => {
                if self.boss_alive {
                    // The fight ended: close both windows.
                    let idle = self.t - self.boss_idle_start;
                    if idle > self.boss_idle_worst.1 {
                        self.boss_idle_worst = (self.boss_idle_start, idle);
                    }
                    let still = self.t - self.boss_still_start;
                    if still > self.boss_still_worst.1 {
                        self.boss_still_worst = (self.boss_still_start, still);
                    }
                    // "Never moved" is its own finding, even inside the window.
                    if self.boss_moved < 2.0 && self.boss_fight >= BOSS_STILL_SECS {
                        self.push(
                            AnomalyKind::BossStill,
                            self.t - self.boss_fight,
                            self.boss_fight,
                            format!(
                                "boss did not move at all over a {:.0} s fight",
                                self.boss_fight
                            ),
                        );
                    }
                    self.boss_alive = false;
                }
            }
        }

        // ---- Stuck window -------------------------------------------------
        // Only while a body is within `UNREACHABLE_PX`: standing still in a
        // calm opening (nothing to chase yet) is not a defect, and the calm
        // opening is 60-110 s long at this balance (`LOG.md`).
        if self.pos_ring.len() >= STUCK_SAMPLES && !self.close.is_empty() {
            let n = self.pos_ring.len() as f32;
            let mut sum = Vec2::ZERO;
            for q in &self.pos_ring {
                sum += *q;
            }
            let mean = sum * (1.0 / n);
            let mut spread = 0.0f32;
            for q in &self.pos_ring {
                spread = spread.max(q.dist(mean));
            }
            if spread <= STUCK_SPREAD_PX {
                if self.stuck_run <= 0.0 {
                    self.stuck_start = self.t - STUCK_SECS;
                }
                self.stuck_run += step;
                if self.stuck_run > self.stuck_worst.1 {
                    self.stuck_worst = (self.stuck_start, self.stuck_run, spread);
                }
            } else {
                self.stuck_run = 0.0;
            }
        } else {
            self.stuck_run = 0.0;
        }
    }

    /// Close the current level's open windows and file its anomalies.
    fn flush_level(&mut self) {
        let Some(_level) = self.level else { return };
        let t = self.t;

        let hurt = (self.hurt_start, t - self.hurt_start);
        if hurt.1 > self.hurt_worst.1 {
            self.hurt_worst = hurt;
        }
        if self.hurt_worst.1 >= NO_PLAYER_DAMAGE_SECS {
            let (at, seconds) = self.hurt_worst;
            let opening = if at <= 0.5 {
                "opening calm: "
            } else {
                ""
            };
            self.push(
                AnomalyKind::NoPlayerDamage,
                at,
                seconds,
                format!("{opening}{seconds:.0} s with no HP lost (level ran {t:.0} s)"),
            );
        }

        let dealt = (self.dealt_start, t - self.dealt_start);
        if dealt.1 > self.dealt_worst.1 {
            self.dealt_worst = dealt;
        }
        if self.dealt_worst.1 >= NO_DAMAGE_DEALT_SECS {
            let (at, seconds) = self.dealt_worst;
            self.push(
                AnomalyKind::NoDamageDealt,
                at,
                seconds,
                format!(
                    "{seconds:.0} s with no damage dealt while {} bodies were alive",
                    self.dealt_bodies
                ),
            );
        }

        if self.boss_alive {
            let idle = (self.boss_idle_start, t - self.boss_idle_start);
            if idle.1 > self.boss_idle_worst.1 {
                self.boss_idle_worst = idle;
            }
            let still = (self.boss_still_start, t - self.boss_still_start);
            if still.1 > self.boss_still_worst.1 {
                self.boss_still_worst = still;
            }
        }
        if self.boss_idle_worst.1 >= BOSS_NO_DAMAGE_SECS {
            let (at, seconds) = self.boss_idle_worst;
            self.push(
                AnomalyKind::BossNoDamage,
                at,
                seconds,
                format!(
                    "boss took no damage for {seconds:.0} s (fight so far: {:.0} s)",
                    self.boss_fight
                ),
            );
        }
        if self.boss_still_worst.1 >= BOSS_STILL_SECS {
            let (at, seconds) = self.boss_still_worst;
            self.push(
                AnomalyKind::BossStill,
                at,
                seconds,
                format!("boss did not move for {seconds:.0} s"),
            );
        }

        if self.stuck_worst.1 >= STUCK_SECS {
            let (at, seconds, spread) = self.stuck_worst;
            self.push(
                AnomalyKind::Stuck,
                at,
                seconds,
                format!(
                    "rat held inside a {spread:.0} px spread for {seconds:.0} s with bodies alive"
                ),
            );
        }

        let extent = (self.roam_max.x - self.roam_min.x).max(self.roam_max.y - self.roam_min.y);
        if t >= 20.0 && extent < START_AREA_PX {
            self.push(
                AnomalyKind::NeverLeftStart,
                t,
                t,
                format!(
                    "roamed only {extent:.0} px in {t:.0} s from ({:.0},{:.0})",
                    self.roam_origin.x, self.roam_origin.y
                ),
            );
        }

        self.level = None;
    }
}

/// Samples in the stuck window (`STUCK_SECS` at the sample cadence).
const STUCK_SAMPLES: usize = (STUCK_SECS / SAMPLE_SECS) as usize;

/// Fold every run's anomalies into one class list (counts + example seeds).
pub fn summarise(anomalies: &[Anomaly]) -> Vec<AnomalyAgg> {
    let mut by_kind: BTreeMap<AnomalyKind, Vec<&Anomaly>> = BTreeMap::new();
    for a in anomalies {
        by_kind.entry(a.kind).or_default().push(a);
    }
    let mut out: Vec<AnomalyAgg> = AnomalyKind::all()
        .into_iter()
        .filter_map(|kind| {
            let list = by_kind.get(&kind)?;
            let mut seeds: Vec<u64> = list.iter().map(|a| a.seed).collect();
            seeds.sort_unstable();
            seeds.dedup();
            let mut examples: Vec<Anomaly> = list.iter().take(5).map(|a| (*a).clone()).collect();
            // Worst first inside a class: longest window, then earliest.
            examples.sort_by(|a, b| {
                b.seconds
                    .partial_cmp(&a.seconds)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.seed.cmp(&b.seed))
            });
            examples.truncate(5);
            Some(AnomalyAgg {
                kind,
                owner: kind.owner().to_string(),
                count: list.len(),
                runs: seeds.len(),
                seeds: seeds.iter().take(8).copied().collect(),
                examples,
            })
        })
        .collect();
    out.sort_by(|a, b| b.runs.cmp(&a.runs).then(b.count.cmp(&a.count)));
    out
}

/// One class in the batch report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnomalyAgg {
    pub kind: AnomalyKind,
    /// The subsystem to read first.
    pub owner: String,
    /// Total anomalies of this kind.
    pub count: usize,
    /// Distinct runs affected.
    pub runs: usize,
    /// The first few affected seeds.
    pub seeds: Vec<u64>,
    /// Worst examples (longest window first).
    pub examples: Vec<Anomaly>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::campaign;
    use crate::game::enemy::EnemyKind;
    use crate::game::level::Theme;
    use crate::game::loadout::Loadout;

    fn world() -> World {
        let mut w = World::new_level(0, campaign::level_seed(5, 0), Loadout::starting());
        w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
        w.player.pos = w.level.arena * 0.5;
        w
    }

    /// A quiet world with no bodies and no boss files nothing.
    #[test]
    fn a_still_world_has_no_anomalies() {
        let w = world();
        let mut scan = AnomalyScan::new(1);
        for _ in 0..600 {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        // The rat never moved, but with no bodies alive "stuck" is not a fault
        // and 10 s is under every silence threshold.
        assert!(found.is_empty(), "unexpected anomalies: {found:?}");
    }

    /// A rat frozen in a crowd for longer than `STUCK_SECS` is reported, with
    /// the window's spread and the seed.
    #[test]
    fn a_frozen_rat_in_a_crowd_is_reported() {
        let mut w = world();
        w.spawn_enemy(EnemyKind::Skitterer, w.player.pos + Vec2::new(20.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(77);
        for _ in 0..600 {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::Stuck),
            "expected a stuck report, got {found:?}"
        );
        assert!(found.iter().all(|a| a.seed == 77));
    }

    /// A boss that never loses HP and never moves is reported twice: no damage
    /// and no movement (the stranded-boss signature).
    #[test]
    fn a_stranded_boss_is_reported() {
        let mut w = world();
        w.spawn_enemy(EnemyKind::Boss, w.player.pos + Vec2::new(300.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(9);
        for _ in 0..(60 * 40) {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::BossNoDamage),
            "expected a no-damage boss report, got {found:?}"
        );
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::BossStill),
            "expected a still boss report, got {found:?}"
        );
    }

    /// Long silences on both sides are reported: nothing hurting the rat and
    /// nothing the rat fired landing.
    #[test]
    fn long_silences_are_reported() {
        let mut w = world();
        w.spawn_enemy(EnemyKind::Brute, w.player.pos + Vec2::new(40.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(11);
        for _ in 0..(60 * 70) {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::NoPlayerDamage),
            "expected a no-player-damage report, got {found:?}"
        );
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::NoDamageDealt),
            "expected a no-damage-dealt report, got {found:?}"
        );
    }

    /// An enemy that stays in weapon range untouched while damage flows to
    /// the rest is reported; one that is out of range is not.
    #[test]
    fn an_unreachable_enemy_is_reported_only_inside_weapon_range() {
        let mut w = world();
        let near = w.spawn_enemy(EnemyKind::Skitterer, w.player.pos + Vec2::new(30.0, 0.0), 1.0);
        let far = w.spawn_enemy(EnemyKind::Skitterer, w.player.pos + Vec2::new(150.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(5);
        // Long enough for the close enemy's clock to pass the threshold.
        for _ in 0..(60 * 21) {
            scan.tick(&w, 1.0 / 60.0);
        }
        // Damage "flows": the close body is removed, so the summed enemy HP
        // falls (the scan's damage-dealt signal).
        w.despawn_entity(near);
        for _ in 0..30 {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            !found.iter().any(|a| a.kind == AnomalyKind::EnemyUnreachable),
            "the removed body cannot be reported and the far one is out of range: {found:?}"
        );

        // Now the same long-lived body but the rat is firing *at it*: the far
        // body from the first half is replaced by a close one that survives.
        let mut w = world();
        let _near = w.spawn_enemy(EnemyKind::Skitterer, w.player.pos + Vec2::new(30.0, 0.0), 1.0);
        let spare = w.spawn_enemy(EnemyKind::Skitterer, w.player.pos + Vec2::new(40.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(6);
        for _ in 0..(60 * 21) {
            scan.tick(&w, 1.0 / 60.0);
        }
        w.despawn_entity(spare);
        for _ in 0..60 {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::EnemyUnreachable),
            "a close body that survived 21 s untouched must be reported: {found:?}"
        );
        let _ = far;
    }

    /// Every class names an owner file, and the summary groups by class, lists
    /// the seed and puts the worst example first.
    #[test]
    fn summary_groups_by_class_and_names_the_seed() {
        let a = Anomaly {
            kind: AnomalyKind::BossNoDamage,
            seed: 42,
            level: 1,
            at: 300.0,
            seconds: 500.0,
            detail: "boss took no damage for 500 s".into(),
        };
        let b = Anomaly {
            kind: AnomalyKind::BossNoDamage,
            seed: 7,
            level: 1,
            at: 300.0,
            seconds: 90.0,
            detail: "boss took no damage for 90 s".into(),
        };
        let agg = summarise(&[a, b]);
        assert_eq!(agg.len(), 1);
        assert_eq!(agg[0].kind, AnomalyKind::BossNoDamage);
        assert_eq!(agg[0].runs, 2);
        assert_eq!(agg[0].count, 2);
        assert!(agg[0].owner.contains("boss"));
        assert_eq!(agg[0].examples[0].seconds, 500.0, "worst example first");
        for kind in AnomalyKind::all() {
            assert!(!kind.owner().is_empty());
            assert!(!kind.name().is_empty());
        }
    }

    /// A cramped roam is reported; one that laps the arena is not.
    #[test]
    fn never_left_start_needs_a_cramped_roam() {
        let mut w = world();
        let mut scan = AnomalyScan::new(3);
        for _ in 0..(60 * 40) {
            w.player.pos += Vec2::new(0.0, 0.5); // 1 200 px of roam
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            !found.iter().any(|a| a.kind == AnomalyKind::NeverLeftStart),
            "a lapping rat must not be flagged: {found:?}"
        );

        let mut w = world();
        w.spawn_enemy(EnemyKind::Brute, w.player.pos + Vec2::new(40.0, 0.0), 1.0);
        let mut scan = AnomalyScan::new(4);
        for _ in 0..(60 * 40) {
            scan.tick(&w, 1.0 / 60.0);
        }
        let found = scan.finish();
        assert!(
            found.iter().any(|a| a.kind == AnomalyKind::NeverLeftStart),
            "a cinematic rat must be flagged: {found:?}"
        );
    }
}
