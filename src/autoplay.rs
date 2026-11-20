//! Self-playing runs: the bot, the campaign driver, and the beatability report.
//!
//! [`AutoRun`] owns one seeded run (a whole three-level campaign, or one level
//! in practice mode), steps it at full speed with no terminal and **no audio**
//! — it never creates an `AudioEngine` and drains/discards the world's `sfx`
//! queue every tick — and keeps the statistics the batch report needs.
//!
//! ```ignore
//! let mut run = AutoRun::new(7, AutoOpts::campaign(Skill::Expert));
//! while !run.finished() {
//!     run.step(1.0 / 60.0);
//! }
//! println!("{:?}", run.status());
//! ```
//!
//! ## What `step` does
//!
//! 1. ask the [`Bot`](crate::game::bot::Bot) for this tick's `move_dir`/`dash`
//!    (movement, dash, boss counters — see the bot module docs);
//! 2. snapshot what could hurt the rat this tick (for damage attribution);
//! 3. `world.update(dt, dir, dash)` — one simulation tick;
//! 4. drain and discard `world.sfx`;
//! 5. bank the tick's damage, boss-fight time and boss damage, then resolve
//!    anything that needs a decision: pending level-ups (1-of-3 pick), a dead
//!    boss (keep-three pick and the campaign's next `World`), death, victory.
//!
//! Nothing in here blocks and nothing allocates beyond the per-level vectors,
//! so 16 of these can be stepped from 16 threads.
//!
//! ## Determinism
//!
//! A run is a pure function of `(run_seed, AutoOpts)`: the bot has its own rng
//! stream seeded from the run seed, upgrade generation uses a second stream
//! seeded from the run seed in `AutoRun`, and the maps come from
//! `campaign::level_seed`. The simulation rng is only ever moved by the
//! simulation itself.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use rand::rngs::StdRng;
use rand::RngExt;
use rand::SeedableRng;

use crate::game::art_traps::{MousetrapState, MOUSETRAP_DAMAGE_PLAYER, MOUSETRAP_TRIGGER_OFFSET_U, MOUSETRAP_TRIGGER_RADIUS};
use crate::game::bot::{Bot, Skill};
use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::{self, CampaignState, Keepable};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::art_boss_broom::BroomBossState;
use crate::game::art_boss_cat::BossCatState;
use crate::game::art_boss_mower::MowerState;
use crate::game::ecs::systems::boss::{Boss, BossKind, BossProjectile, BossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::loadout::Loadout;
use crate::game::world::World;
use crate::math::Vec2;

pub mod batch;

/// Player collision radius, as the simulation defines it.
const R: f32 = PLAYER_RADIUS;

/// How a run is configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoOpts {
    /// Campaign level to start on (0 Sewer, 1 Kitchen, 2 Lab).
    pub start_level: usize,
    /// `true`: play the whole three-level campaign from `start_level`.
    /// `false`: play `start_level` alone (practice) and stop after its boss.
    pub campaign: bool,
    /// How well the bot plays.
    pub skill: Skill,
}

impl AutoOpts {
    /// A full campaign at `start_level`.
    pub fn campaign(start_level: usize, skill: Skill) -> AutoOpts {
        AutoOpts {
            start_level,
            campaign: true,
            skill,
        }
    }

    /// One level alone, no carry-over.
    pub fn practice(level: usize, skill: Skill) -> AutoOpts {
        AutoOpts {
            start_level: level,
            campaign: false,
            skill,
        }
    }
}

impl Default for AutoOpts {
    fn default() -> Self {
        AutoOpts {
            start_level: 0,
            campaign: true,
            skill: Skill::Expert,
        }
    }
}

/// What the run is doing right now — cheap enough to poll every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AutoStatus {
    Playing {
        /// 0-based campaign level.
        level: usize,
        /// Seconds on this level's clock.
        elapsed: f32,
        hp: f32,
        max_hp: f32,
        /// Player XP level (1 = start).
        player_level: u32,
        kills: u32,
        score: u32,
        /// `(hp fraction, fight phase)` while a boss is alive.
        boss: Option<(f32, u8)>,
    },
    Died {
        level: usize,
        at: f32,
        cause: DamageCause,
    },
    Won {
        total_time: f32,
        score: u32,
    },
}

/// What hurt the rat.
///
/// The simulation does not record a damage source for the player (it only
/// applies `Player::take_damage`), so the runner attributes each tick's HP
/// loss to the most likely attacker it snapshotted **before** the tick: exact
/// damage match first, then proximity. That is a diagnostic, not a spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageCause {
    /// Body contact with a regular enemy of this kind.
    EnemyContact(EnemyKind),
    /// A regular enemy's bullet (spitters).
    EnemyBullet,
    /// Body contact with a boss.
    BossContact(BossKind),
    /// A telegraphed boss hazard (stomp, sweep, pounce, charge corridor, …).
    BossHazard(BossKind, BossAttack),
    /// A boss projectile (blade disc, fang, dustpan shard).
    BossProjectile(BossKind),
    /// A floor trap (the mousetrap snap; glue only slows).
    Trap,
    /// Nothing matched the HP loss (should be rare).
    Unknown,
}

/// The boss attack a hazard telegraph belongs to, named by the boss state the
/// controller was in when it was pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossAttack {
    Pounce,
    Swipe,
    Roar,
    Stomp,
    Sweep,
    Jab,
    Dustpan,
    PhaseSlam,
    Charge,
    Ricochet,
    Blade,
    Other,
}

impl BossAttack {
    pub fn name(self) -> &'static str {
        match self {
            BossAttack::Pounce => "pounce",
            BossAttack::Swipe => "swipe",
            BossAttack::Roar => "roar",
            BossAttack::Stomp => "stomp",
            BossAttack::Sweep => "sweep",
            BossAttack::Jab => "broom jab",
            BossAttack::Dustpan => "dustpan",
            BossAttack::PhaseSlam => "phase slam",
            BossAttack::Charge => "charge",
            BossAttack::Ricochet => "ricochet",
            BossAttack::Blade => "blade",
            BossAttack::Other => "other",
        }
    }
}

impl DamageCause {
    /// Stable display/aggregation key.
    pub fn name(self) -> &'static str {
        match self {
            DamageCause::EnemyContact(EnemyKind::Skitterer) => "skitterer contact",
            DamageCause::EnemyContact(EnemyKind::Bat) => "bat contact",
            DamageCause::EnemyContact(EnemyKind::Cat) => "cat contact",
            DamageCause::EnemyContact(EnemyKind::Spitter) => "spitter contact",
            DamageCause::EnemyContact(EnemyKind::Brute) => "brute contact",
            DamageCause::EnemyContact(EnemyKind::Boss) => "boss contact",
            DamageCause::EnemyBullet => "spitter bullets",
            DamageCause::BossContact(BossKind::Cat) => "jaguar body",
            DamageCause::BossContact(BossKind::Broom) => "broom human body",
            DamageCause::BossContact(BossKind::Mower) => "mower body",
            DamageCause::BossHazard(BossKind::Cat, a) => match a {
                BossAttack::Pounce => "jaguar pounce",
                BossAttack::Swipe => "jaguar swipe",
                BossAttack::Roar => "jaguar roar",
                BossAttack::Other => "jaguar hazard",
                _ => "jaguar hazard",
            },
            DamageCause::BossHazard(BossKind::Broom, a) => match a {
                BossAttack::Stomp => "broom stomp",
                BossAttack::Sweep => "broom sweep",
                BossAttack::Jab => "broom jab",
                BossAttack::Dustpan => "dustpan",
                _ => "broom hazard",
            },
            DamageCause::BossHazard(BossKind::Mower, a) => match a {
                BossAttack::Charge => "mower charge",
                BossAttack::Ricochet => "mower ricochet",
                BossAttack::Blade => "mower blades",
                _ => "mower hazard",
            },
            DamageCause::BossProjectile(BossKind::Cat) => "sonic fang",
            DamageCause::BossProjectile(BossKind::Broom) => "dustpan shard",
            DamageCause::BossProjectile(BossKind::Mower) => "blade disc",
            DamageCause::Trap => "mousetrap",
            DamageCause::Unknown => "unknown",
        }
    }
}

/// One campaign level's numbers.
#[derive(Clone, Debug, Default)]
pub struct LevelStats {
    /// 0-based campaign level.
    pub level: usize,
    /// Seconds survived on this level.
    pub time: f32,
    /// Did the bot reach the boss phase at all?
    pub reached_boss: bool,
    /// Seconds spent with a boss alive.
    pub boss_seconds: f32,
    /// Did the boss die (level cleared)?
    pub boss_defeated: bool,
    /// Damage taken, by attributed source.
    pub damage: BTreeMap<&'static str, f32>,
    /// Total damage taken on this level (attributed or not).
    pub damage_total: f32,
    /// Damage per second the **build** was worth when the boss appeared
    /// (analytic from the loadout, so it is comparable across runs).
    pub dps_at_boss: f32,
    /// Damage per second actually booked into the boss while it lived.
    pub boss_dps: f32,
    /// Level-up picks, in order (`"Gnaw Lv2"`, `"Sharp Teeth"`, …).
    pub upgrades: Vec<String>,
    /// What was carried into the next level (manifest titles).
    pub kept: Vec<String>,
    /// How the rat died here, if it did.
    pub death: Option<DeathInfo>,
}

impl LevelStats {
    fn new(level: usize) -> LevelStats {
        LevelStats {
            level,
            ..LevelStats::default()
        }
    }
}

/// Where and to what a run ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathInfo {
    pub level: usize,
    pub at: f32,
    pub cause: DamageCause,
}

/// Everything a run measured.
#[derive(Clone, Debug, Default)]
pub struct RunStats {
    pub levels: Vec<LevelStats>,
    /// Wall-independent total simulated seconds (all levels).
    pub total_time: f32,
    pub total_kills: u32,
    pub total_score: u32,
    pub death: Option<DeathInfo>,
    /// The run beat the whole campaign (or cleared its practice level).
    pub won: bool,
}

impl RunStats {
    pub fn level(&self, index: usize) -> Option<&LevelStats> {
        self.levels.iter().find(|l| l.level == index)
    }
}

/// One self-playing run. `Send`, so batches can step runs on worker threads.
pub struct AutoRun {
    opts: AutoOpts,
    campaign: Option<CampaignState>,
    world: World,
    bot: Bot,
    /// Upgrade-choice rng (the bot's own decisions stream is inside `Bot`).
    rng: StdRng,
    stats: RunStats,
    /// Threat attribution scratch for the current tick.
    threats: Vec<(DamageCause, f32, f32)>,
    /// Boss fight bookkeeping for the current level.
    boss_alive: bool,
    boss_hp: f32,
    boss_damage: f32,
    /// Totals banked from finished levels.
    banked_time: f32,
    banked_kills: u32,
    banked_score: u32,
    died: bool,
    won: bool,
    ticks: u64,
}

impl AutoRun {
    /// A fresh, seeded run.
    pub fn new(run_seed: u64, opts: AutoOpts) -> AutoRun {
        let start_level = opts.start_level.min(campaign::LEVEL_COUNT - 1);
        let (campaign, world) = if opts.campaign {
            let mut state = CampaignState::new(run_seed);
            state.level = start_level;
            let world = state.world();
            (Some(state), world)
        } else {
            let world = World::new_level(
                start_level,
                campaign::level_seed(run_seed, start_level),
                Loadout::starting(),
            );
            (None, world)
        };
        let mut stats = RunStats::default();
        stats.levels.push(LevelStats::new(start_level));
        let mut bot = Bot::new(opts.skill, run_seed);
        bot.on_new_level(start_level);
        let mut rng = StdRng::seed_from_u64(run_seed ^ 0x000A_0700_u64);
        // Keep the value stream from starting on a degenerate value.
        let _ = rng.random::<f32>();
        AutoRun {
            opts,
            campaign,
            world,
            bot,
            rng,
            stats,
            threats: Vec::with_capacity(32),
            boss_alive: false,
            boss_hp: 0.0,
            boss_damage: 0.0,
            banked_time: 0.0,
            banked_kills: 0,
            banked_score: 0,
            died: false,
            won: false,
            ticks: 0,
        }
    }

    pub fn opts(&self) -> AutoOpts {
        self.opts
    }

    /// One simulation tick plus every bot decision it implies.
    pub fn step(&mut self, dt: f32) {
        if self.finished() {
            return;
        }
        self.ticks += 1;
        let input = self.bot.decide(&self.world, dt);
        self.snapshot_threats();
        let hp_before = self.world.player.hp;
        let regen = self.world.player.loadout.stats.regen * dt;
        let boss_before = self.boss_state();
        self.world.update(dt, input.move_dir, input.dash);
        // No audio in a self-playing run: the queue is drained and discarded.
        self.world.sfx.clear();
        self.account_tick(dt, hp_before, regen, boss_before);

        // Decisions the real App would have made between ticks.
        while self.world.pending_levelups > 0 && !self.world.game_over {
            self.world.pending_levelups -= 1;
            self.take_upgrade();
        }
        if self.world.game_over {
            self.finish_death();
            return;
        }
        if self.world.level_complete() {
            self.finish_level();
        }
    }

    /// The world to draw.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The world to draw into (`World::draw` needs `&mut` for its camera).
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn status(&self) -> AutoStatus {
        if self.died {
            if let Some(d) = self.stats.death {
                return AutoStatus::Died {
                    level: d.level,
                    at: d.at,
                    cause: d.cause,
                };
            }
        }
        if self.won {
            return AutoStatus::Won {
                total_time: self.stats.total_time,
                score: self.stats.total_score,
            };
        }
        let w = &self.world;
        let max_hp = w.player.max_hp().max(1.0);
        AutoStatus::Playing {
            level: w.level_index,
            elapsed: w.elapsed,
            hp: w.player.hp,
            max_hp,
            player_level: w.player.level,
            kills: w.kills,
            score: w.score,
            boss: self.boss_state(),
        }
    }

    /// Dead or victorious — no more simulation.
    pub fn finished(&self) -> bool {
        self.died || self.won
    }

    pub fn stats(&self) -> &RunStats {
        &self.stats
    }

    /// One human-readable line of policy/game state (`--bot-batch --trace`).
    pub fn trace_line(&self) -> String {
        let w = &self.world;
        let (goal, heading) = self.bot.debug_goal();
        let status = self.status();
        let mut near = 0usize;
        let mut closest = f32::INFINITY;
        for e in w.ecs.iter_entities() {
            if let Some(en) = e.get::<Enemy>() {
                if !en.alive() {
                    continue;
                }
                let d = en.pos.dist(w.player.pos);
                closest = closest.min(d);
                if d < 60.0 {
                    near += 1;
                }
            }
        }
        let cell = crate::game::collision::cell_of(w.player.pos);
        format!(
            "t={:6.1} lvl={} pos=({:6.1},{:6.1}) cell=({},{}) hp={:5.1}/{:5.1} en={:3} near={:2} min={:5.1} goal={:5.1}px boss={:5.1}px head=({:+.2},{:+.2}) {:?}",
            w.elapsed,
            w.level_index + 1,
            w.player.pos.x,
            w.player.pos.y,
            cell.0,
            cell.1,
            w.player.hp,
            w.player.max_hp(),
            w.enemy_count(),
            near,
            closest,
            w.player.pos.dist(goal),
            w.boss_offset().map(|o| o.len()).unwrap_or(f32::INFINITY),
            heading.x,
            heading.y,
            status
        )
    }

    /// Ticks simulated so far (diagnostics / speed reporting).
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    // ---- Internals -------------------------------------------------------

    /// `(hp fraction, phase)` of the living boss.
    fn boss_state(&self) -> Option<(f32, u8)> {
        self.world
            .boss_hp()
            .map(|(hp, max)| ((hp / max.max(1.0)).clamp(0.0, 1.0), 0))
            .map(|(frac, _)| (frac, self.world.boss_phase().unwrap_or(1)))
    }

    fn level_stats(&mut self) -> &mut LevelStats {
        let index = self.world.level_index;
        if !self.stats.levels.iter().any(|l| l.level == index) {
            self.stats.levels.push(LevelStats::new(index));
        }
        let pos = self
            .stats
            .levels
            .iter()
            .position(|l| l.level == index)
            .unwrap_or(0);
        &mut self.stats.levels[pos]
    }

    fn take_upgrade(&mut self) {
        let choices = self.world.player.loadout.generate_choices(&mut self.rng);
        if choices.is_empty() {
            return;
        }
        let pick = self.bot.pick_upgrade(&self.world, &choices);
        let pick = pick.min(choices.len() - 1);
        let title = self.world.player.loadout.describe(&choices[pick]).0;
        let heal = self.world.player.loadout.apply(&choices[pick]);
        self.world.player.heal(heal);
        self.level_stats().upgrades.push(title);
    }

    /// The boss died: carry three powerups on, or finish the run.
    fn finish_level(&mut self) {
        // Completing a level *is* killing its boss; record it even if the
        // boss's death tick and the completion flag landed on different ticks.
        let dealt = self.boss_damage;
        {
            let stats = self.level_stats();
            stats.reached_boss = true;
            stats.boss_defeated = true;
            if stats.boss_seconds > 0.0 && stats.boss_dps <= 0.0 {
                stats.boss_dps = dealt / stats.boss_seconds;
            }
        }
        let final_level = self.world.is_final_level();
        let practice = self.campaign.is_none();
        let mut level_finished = true;
        if !practice && !final_level {
            let items = self.world.keepables();
            let picks = self.bot.pick_keep(&self.world, &items);
            let chosen: Vec<Keepable> = picks.iter().map(|&i| items[i]).collect();
            let kept: Vec<String> = chosen.iter().map(|k| k.title()).collect();
            let carried = self.campaign.as_mut().and_then(|c| {
                c.complete_level(&self.world, &chosen)
                    .ok()
                    .map(|_| c.loadout.clone())
            });
            match carried {
                Some(loadout) => {
                    let stats = self.level_stats();
                    stats.kept = kept;
                    let next = self.campaign.as_ref().map(|c| c.level).unwrap_or(0);
                    self.banked_time += self.world.elapsed;
                    self.banked_kills += self.world.kills;
                    self.banked_score += self.world.score;
                    let seed = self.campaign.as_ref().map(|c| c.level_seed()).unwrap_or(0);
                    let theme = self.campaign.as_ref().map(|c| c.theme());
                    let totals = self
                        .campaign
                        .as_ref()
                        .map(|c| c.totals)
                        .unwrap_or_default();
                    let mut world = World::new_level(next, seed, loadout);
                    if let Some(theme) = theme {
                        world.level.theme = theme;
                    }
                    world.carried = totals;
                    self.world = world;
                    self.stats.levels.push(LevelStats::new(next));
                    self.bot.on_new_level(next);
                    self.reset_boss_book();
                    level_finished = false;
                }
                None => {
                    // Could not build the next level: treat as a completed run.
                    level_finished = true;
                }
            }
        }
        if level_finished {
            self.banked_time += self.world.elapsed;
            self.banked_kills += self.world.kills;
            self.banked_score += self.world.score;
            self.stats.total_time = self.banked_time;
            self.stats.total_kills = self.banked_kills;
            self.stats.total_score = self.banked_score;
            self.stats.won = true;
            self.won = true;
        }
    }

    fn finish_death(&mut self) {
        let level = self.world.level_index;
        let at = self.world.elapsed;
        let cause = self
            .threats
            .iter()
            .min_by(|a, b| {
                a.2.partial_cmp(&b.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|t| t.0)
            .unwrap_or(DamageCause::Unknown);
        let death = DeathInfo { level, at, cause };
        let stats = self.level_stats();
        stats.death = Some(death);
        stats.time = at;
        let time = self.banked_time + self.world.elapsed;
        self.stats.total_kills = self.banked_kills + self.world.kills;
        self.stats.total_score = self.banked_score + self.world.score;
        self.stats.total_time = time;
        self.stats.death = Some(death);
        self.died = true;
    }

    fn reset_boss_book(&mut self) {
        self.boss_alive = false;
        self.boss_hp = 0.0;
        self.boss_damage = 0.0;
    }

    /// Book the tick's damage, boss time and boss damage.
    fn account_tick(
        &mut self,
        dt: f32,
        hp_before: f32,
        regen: f32,
        boss_before: Option<(f32, u8)>,
    ) {
        let hp_after = self.world.player.hp;
        let lost = hp_before + regen - hp_after;
        let cause = if lost > 0.05 {
            Some(self.attribute(lost))
        } else {
            None
        };
        let level = self.world.level_index;
        let elapsed = self.world.elapsed;
        let boss_after = self.boss_state();
        let boss_max = self.world.boss_hp().map(|(_, max)| max).unwrap_or(1.0);
        let level_complete = self.world.level_complete();
        let dps_now = analytic_dps(&self.world.player.loadout);

        // Keep the run totals live, so a run stopped by the batch's time cap
        // still reports how long it actually lasted.
        self.stats.total_time = self.banked_time + elapsed;
        self.stats.total_kills = self.banked_kills + self.world.kills;
        self.stats.total_score = self.banked_score + self.world.score;

        let pos = self
            .stats
            .levels
            .iter()
            .position(|l| l.level == level)
            .unwrap_or(0);
        let stats = &mut self.stats.levels[pos];
        stats.time = elapsed;
        if let Some(cause) = cause {
            *stats.damage.entry(cause.name()).or_insert(0.0) += lost;
            stats.damage_total += lost;
        }

        // Boss fight accounting.
        match (boss_before, boss_after) {
            (None, Some((frac, _))) => {
                self.boss_alive = true;
                self.boss_hp = frac * boss_max;
                self.boss_damage = 0.0;
                stats.reached_boss = true;
                stats.dps_at_boss = dps_now;
            }
            (Some((before, _)), Some((frac, _))) => {
                let dealt = (before - frac) * boss_max;
                if dealt > 0.0 {
                    self.boss_damage += dealt;
                }
                stats.boss_seconds += dt;
                if stats.boss_seconds > 0.0 {
                    stats.boss_dps = self.boss_damage / stats.boss_seconds;
                }
            }
            (Some(_), None) => {
                // The boss died (or despawned) this tick.
                if level_complete {
                    stats.boss_defeated = true;
                    stats.boss_seconds += dt;
                    if stats.boss_seconds > 0.0 {
                        stats.boss_dps = self.boss_damage / stats.boss_seconds;
                    }
                }
                self.boss_alive = false;
            }
            (None, None) => {}
        }
    }

    /// Best guess at what dealt `lost` HP this tick: exact damage match first,
    /// then proximity (the snapshot is pre-tick, so it can only hold things
    /// that were in a position to hit).
    fn attribute(&self, lost: f32) -> DamageCause {
        // Contact damage from several enemies is summed into one
        // `take_damage`, so the HP loss is often larger than any single
        // candidate: prefer the biggest contributor that does not overshoot
        // the loss, and only fall back to the closest exact match.
        let mut best: Option<(DamageCause, f32)> = None;
        for &(cause, expected, dist) in &self.threats {
            let key = if expected <= lost + 0.01 {
                -expected + dist * 0.02
            } else {
                (expected - lost) * 6.0 + dist * 0.02
            };
            if best.map(|(_, k)| key < k) != Some(false) {
                best = Some((cause, key));
            }
        }
        best.map(|(c, _)| c).unwrap_or(DamageCause::Unknown)
    }

    /// Snapshot everything that could hurt the rat this tick.
    fn snapshot_threats(&mut self) {
        self.threats.clear();
        let w = &self.world;
        let p = w.player.pos;
        let armor = w.player.loadout.stats.armor;
        // Fallback cause when nothing was in contact range: the nearest enemy
        // is still the likeliest author of an unexplained HP drop.
        let mut nearest: Option<(EnemyKind, f32)> = None;
        for e in w.ecs.iter_entities() {
            if let Some(en) = e.get::<Enemy>() {
                if !en.alive() {
                    continue;
                }
                let d = en.pos.dist(p);
                if nearest.map(|(_, nd)| d < nd) != Some(false) {
                    nearest = Some((en.kind, d));
                }
                if d < en.radius + R + 4.0 {
                    self.threats.push((
                        DamageCause::EnemyContact(en.kind),
                        (en.damage - armor).max(1.0),
                        d,
                    ));
                }
            } else if let Some(b) = e.get::<Bullet>() {
                if b.faction != Faction::Enemy {
                    continue;
                }
                // Swept, so a bullet that lands this tick still matches.
                let d = segment_point_dist(b.prev_pos, b.pos, p);
                if d < b.radius + R + 2.0 {
                    self.threats.push((
                        DamageCause::EnemyBullet,
                        (b.damage - armor).max(1.0),
                        d,
                    ));
                }
            } else if let Some(boss) = e.get::<Boss>() {
                if boss.dead {
                    continue;
                }
                let d = boss.pos.dist(p);
                if d < boss.kind.body_radius() + R + 3.0 && boss.contact_damage > 0.0 {
                    self.threats.push((
                        DamageCause::BossContact(boss.kind),
                        (boss.contact_damage - armor).max(1.0),
                        d,
                    ));
                }
                let attack = boss_attack(&boss.state);
                for h in &boss.hazards {
                    if !h.hit_player || h.is_expired() {
                        continue;
                    }
                    let warn_left = h.warn - h.elapsed;
                    // Active now, or warning out inside this tick's dt.
                    let landing = h.is_active() || (warn_left > 0.0 && warn_left < 0.05);
                    if landing && h.shape.contains(p, R) {
                        self.threats.push((
                            DamageCause::BossHazard(boss.kind, attack),
                            (h.damage - armor).max(1.0),
                            h.shape.center().dist(p),
                        ));
                    }
                }
            } else if let Some(bp) = e.get::<BossProjectile>() {
                let d = segment_point_dist(bp.prev, bp.pos, p);
                if d < bp.radius + R + 2.0 {
                    // The projectile knows its kind; the boss that fired it is
                    // the campaign level's boss.
                    let kind = BossKind::for_level(w.level_index);
                    self.threats.push((
                        DamageCause::BossProjectile(kind),
                        (bp.damage - armor).max(1.0),
                        d,
                    ));
                }
            }
        }
        if self.threats.is_empty() {
            if let Some((kind, d)) = nearest {
                self.threats.push((DamageCause::EnemyContact(kind), 1.0, d));
            }
        }
        for trap in &w.level.objects.mousetraps {
            if !matches!(
                trap.state,
                MousetrapState::Triggered | MousetrapState::Snapping
            ) {
                continue;
            }
            let trigger = trap.pos + trap.facing * MOUSETRAP_TRIGGER_OFFSET_U;
            let d = trigger.dist(p);
            if d < MOUSETRAP_TRIGGER_RADIUS + R + 4.0 {
                self.threats.push((
                    DamageCause::Trap,
                    (MOUSETRAP_DAMAGE_PLAYER - armor).max(1.0),
                    d,
                ));
            }
        }
    }
}

/// Analytic single-target-ish DPS of a build: every weapon's per-shot damage
/// times its projectile count over its cooldown, with the passive multipliers.
/// Comparable across runs because it ignores aiming and misses.
pub fn analytic_dps(lo: &Loadout) -> f32 {
    let mut dps = 0.0;
    for weapon in &lo.weapons {
        let cooldown = weapon.cooldown().max(0.05);
        let shots = weapon.level_projectiles().max(1) as f32;
        dps += weapon.damage() * shots / cooldown;
    }
    dps * lo.stats.damage_mult * lo.stats.fire_rate_mult
}

/// The attack a boss state's hazards belong to.
fn boss_attack(state: &BossState) -> BossAttack {
    match *state {
        BossState::Cat(s) => match s {
            BossCatState::Crouch | BossCatState::Pounce => BossAttack::Pounce,
            BossCatState::Swipe | BossCatState::SwipeWindup => BossAttack::Swipe,
            BossCatState::Roar => BossAttack::Roar,
            _ => BossAttack::Other,
        },
        BossState::Broom(s) => match s {
            BroomBossState::StompWalk { .. } => BossAttack::Stomp,
            BroomBossState::BroomSweep { .. } => BossAttack::Sweep,
            BroomBossState::BroomJab => BossAttack::Jab,
            BroomBossState::DustpanDrop => BossAttack::Dustpan,
            BroomBossState::PhaseChange => BossAttack::PhaseSlam,
            _ => BossAttack::Other,
        },
        BossState::Mower(s) => match s {
            MowerState::Charge | MowerState::ChargeWindup => BossAttack::Charge,
            MowerState::Ricochet => BossAttack::Ricochet,
            MowerState::FlingBlades => BossAttack::Blade,
            _ => BossAttack::Other,
        },
    }
}

/// Distance from `p` to the segment `a..b`.
fn segment_point_dist(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let ab = b - a;
    let denom = ab.len_sq();
    let t = if denom <= 1e-6 {
        0.0
    } else {
        ((p - a).dot(ab) / denom).clamp(0.0, 1.0)
    };
    (a + ab * t).dist(p)
}

/// A `Send` assert, so a batch can step runs on worker threads.
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<AutoRun>();
}

/// Silence the unused-entity import on non-test builds.
#[allow(dead_code)]
fn _entity_marker(_: Entity) {}

/// Convenience for tests and the batch: step `run` to its end or until
/// `max_seconds` of simulated time, whichever comes first.
pub fn run_to_end(run: &mut AutoRun, dt: f32, max_seconds: f32) -> f32 {
    let mut simulated = 0.0;
    while !run.finished() && simulated < max_seconds {
        run.step(dt);
        simulated += dt;
    }
    simulated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::loadout::Upgrade;

    #[test]
    fn a_run_steps_without_audio_and_without_sfx() {
        let mut run = AutoRun::new(3, AutoOpts::practice(0, Skill::Expert));
        for _ in 0..600 {
            run.step(1.0 / 60.0);
            assert!(run.world().sfx.is_empty(), "autoplay must discard sfx");
            assert!(!matches!(run.status(), AutoStatus::Playing { .. }) || !run.finished());
        }
    }

    #[test]
    fn identical_seeds_produce_identical_runs() {
        let mut a = AutoRun::new(21, AutoOpts::campaign(0, Skill::Expert));
        let mut b = AutoRun::new(21, AutoOpts::campaign(0, Skill::Expert));
        for _ in 0..900 {
            a.step(1.0 / 60.0);
            b.step(1.0 / 60.0);
            assert_eq!(a.status(), b.status());
            assert_eq!(a.stats().levels.len(), b.stats().levels.len());
        }
        assert_eq!(a.stats().total_score, b.stats().total_score);
    }

    /// The campaign's between-levels path (keep three → next `World`) and the
    /// final-level victory, driven without waiting for a real boss kill.
    #[test]
    fn a_campaign_advances_through_three_levels_and_wins() {
        let mut run = AutoRun::new(31, AutoOpts::campaign(0, Skill::Expert));
        for _ in 0..120 {
            run.step(1.0 / 60.0);
        }
        assert_eq!(run.world().level_index, 0);

        // Level 1 boss dies: the runner must keep three and build level 2.
        run.world_mut().mark_level_complete();
        run.step(1.0 / 60.0);
        assert_eq!(run.world().level_index, 1, "campaign advanced to Kitchen");
        assert_eq!(run.world().level.theme, campaign::level_theme(1));
        assert!(!run.finished());
        // The carried pack is at most three entries, never empty of weapons.
        assert!(!run.world().player.loadout.weapons.is_empty());
        let carried = run.world().player.loadout.weapons.len()
            + run.world().player.loadout.passives.len();
        assert!((1..=campaign::KEEP_COUNT).contains(&carried));

        run.world_mut().mark_level_complete();
        run.step(1.0 / 60.0);
        assert_eq!(run.world().level_index, 2, "campaign advanced to Lab");
        assert!(!run.finished());

        run.world_mut().mark_level_complete();
        run.step(1.0 / 60.0);
        assert!(run.finished());
        assert!(matches!(run.status(), AutoStatus::Won { .. }));
        assert!(run.stats().won);
        assert_eq!(run.stats().levels.len(), 3);
    }

    /// The owner's beatability instrument has to run whole expert campaigns to
    /// an end state without panicking on any seed.
    #[test]
    fn ten_expert_campaigns_reach_an_end_state() {
        for seed in 0..10u64 {
            let mut run = AutoRun::new(seed, AutoOpts::campaign(0, Skill::Expert));
            // 6 simulated minutes is well past the boss's 255 s arrival; most
            // runs die before it, which is also an end state.
            let simulated = run_to_end(&mut run, 1.0 / 60.0, 360.0);
            assert!(run.finished(), "seed {seed} did not finish in {simulated}s");
            assert!(
                matches!(
                    run.status(),
                    AutoStatus::Died { .. } | AutoStatus::Won { .. }
                ),
                "seed {seed} ended in {:?}",
                run.status()
            );
            assert!(run.stats().total_time.is_finite());
        }
    }

    /// Casual has to be measurably worse than expert on the same seeds: that
    /// gap is what the coordinator reads as "an expert wins N %, a casual M %".
    ///
    /// The seeds are the ones the beatability report uses (`batch::seed_for`),
    /// so this test and the report describe the same games.
    #[test]
    fn casual_does_worse_than_expert_on_the_same_seeds() {
        let seeds: Vec<u64> = (0..8).map(|i| batch::seed_for(900, i)).collect();
        let score = |skill: Skill| -> (u32, f32) {
            let mut clears = 0u32;
            let mut time = 0.0f32;
            for &seed in &seeds {
                let mut run = AutoRun::new(seed, AutoOpts::practice(0, skill));
                // Below `director::BOSS_TIME` (255 s): this test measures
                // survival, and staying under the boss keeps it independent of
                // the shared-tree boss-death bug (see the escalation on
                // `boss_update_system` invalidating `EnemyIndex`).
                let simulated = run_to_end(&mut run, 1.0 / 60.0, 240.0);
                time += simulated;
                if run.stats().won {
                    clears += 1;
                }
            }
            (clears, time / seeds.len() as f32)
        };
        let (expert_clears, expert_time) = score(Skill::Expert);
        let (casual_clears, casual_time) = score(Skill::Casual);
        assert!(
            expert_time > casual_time * 1.15,
            "expert survived {expert_time:.0}s on average, casual {casual_time:.0}s"
        );
        assert!(
            expert_clears >= casual_clears,
            "expert cleared {expert_clears}, casual {casual_clears}"
        );
    }

    #[test]
    fn analytic_dps_grows_with_the_build() {
        let base = Loadout::starting();
        let mut upgraded = Loadout::starting();
        upgraded.apply(&Upgrade::LevelWeapon(0));
        upgraded.apply(&Upgrade::Passive(crate::game::loadout::PassiveKind::Damage));
        assert!(analytic_dps(&upgraded) > analytic_dps(&base));
    }
}
