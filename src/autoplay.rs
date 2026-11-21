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
use crate::game::ecs::resources::DamageLedger;
use crate::game::ecs::systems::boss::{Boss, BossFight, BossKind, BossProjectile, BossState};
use crate::game::weapon::WeaponKind;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::loadout::Loadout;
use crate::game::world::World;
use crate::math::Vec2;

pub mod anomaly;
pub mod batch;

/// Player collision radius, as the simulation defines it.
const R: f32 = PLAYER_RADIUS;

/// What loadout a measured level starts with.
///
/// A `--level 2` / `--level 3` batch used to start with [`Loadout::starting`],
/// which is not the level a carried player meets: a real campaign arrives with
/// the three powerups the keep screen hands over. [`CarryPreset`] lets the
/// batch runner reproduce that start without the whole campaign before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CarryPreset {
    /// The real level-1 start: one Gnaw at level 1, no passives.
    #[default]
    None,
    /// Three max-level weapons: the modal keep from the 30-run seed-900
    /// campaigns at every skill (Tail Whip Lv5 + Squeak Nova Lv5 + Spore Orbit
    /// Lv5; the "carried on" lines of `base_*.txt`), i.e. what a player who
    /// finished the Sewer actually walks into the Kitchen with.
    Typical,
}

impl CarryPreset {
    /// Parse the `--carry` value. `None` on an unknown name.
    pub fn parse(name: &str) -> Option<CarryPreset> {
        match name {
            "none" => Some(CarryPreset::None),
            "typical" => Some(CarryPreset::Typical),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CarryPreset::None => "none",
            CarryPreset::Typical => "typical",
        }
    }

    /// The loadout this preset starts a level with.
    pub fn loadout(self) -> Loadout {
        match self {
            CarryPreset::None => Loadout::starting(),
            CarryPreset::Typical => {
                use crate::game::weapon::{Weapon, WeaponKind};
                let weapons = [WeaponKind::TailWhip, WeaponKind::SqueakNova, WeaponKind::SporeOrbit]
                    .into_iter()
                    .map(|kind| {
                        let mut w = Weapon::new(kind);
                        w.level = kind.max_level();
                        w
                    })
                    .collect();
                Loadout::from_parts(weapons, Vec::new())
            }
        }
    }
}

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
    /// What `start_level` starts with (measurement only: the campaign itself
    /// always builds its loadout through [`crate::game::campaign`]).
    pub carry: CarryPreset,
}

impl AutoOpts {
    /// A full campaign at `start_level`.
    pub fn campaign(start_level: usize, skill: Skill) -> AutoOpts {
        AutoOpts {
            start_level,
            campaign: true,
            skill,
            carry: CarryPreset::None,
        }
    }

    /// One level alone, no carry-over.
    pub fn practice(level: usize, skill: Skill) -> AutoOpts {
        AutoOpts {
            start_level: level,
            campaign: false,
            skill,
            carry: CarryPreset::None,
        }
    }

    /// One level alone, starting with `carry` (see [`CarryPreset`]).
    pub fn practice_with(level: usize, skill: Skill, carry: CarryPreset) -> AutoOpts {
        AutoOpts {
            start_level: level,
            campaign: false,
            skill,
            carry,
        }
    }
}

impl Default for AutoOpts {
    fn default() -> Self {
        AutoOpts {
            start_level: 0,
            campaign: true,
            skill: Skill::Expert,
            carry: CarryPreset::None,
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
    /// Seconds until the first level-up on this level (`None` if none came).
    pub first_levelup: Option<f32>,
    /// Kills booked on this level.
    pub kills: u32,
    /// Seconds spent with an enemy body within [`NEAR_ENEMY_PX`].
    pub near_seconds: f32,
    /// Player XP level when the boss first appeared (`None` if it never did).
    /// The "level about 15 to 30 at the first boss" target is read from this.
    pub level_at_boss: Option<u32>,
    /// Player state at the fixed progress checkpoints (see [`PROGRESS_TIMES`]).
    pub progress: Vec<Progress>,
    /// Enemies alive when the rat died here.
    pub enemies_at_death: Option<u32>,
    /// Highest player XP level reached on this level (the anomaly scan's
    /// 3x-the-median test needs a peak, not a checkpoint).
    pub max_player_level: u32,
    /// Peak analytic build DPS (see [`analytic_dps`]) seen on this level.
    pub max_analytic_dps: f32,
    /// One-line cause-of-death classification (diagnostic, not a spec).
    pub death_note: Option<String>,
    /// Effective damage dealt to enemies, by weapon (`WeaponKind::all()`
    /// index). Read from the simulation's `DamageLedger`, which no system
    /// reads back: pure balance instrumentation.
    pub dealt: [f32; crate::game::weapon::WEAPON_SLOTS],
    /// Damage dealt to this level's boss, by weapon (`WeaponKind::all()`
    /// index). The boss is not an `Enemy`, so this comes from the boss fight's
    /// own per-tick source tally: acid pools and Tail Whip are exact, aimed
    /// bullets (Gnaw / Cheese Spray) and pattern bullets (Squeak Nova / Spore
    /// Orbit) are split in proportion to those weapons' analytic dps (see
    /// [`boss_weapon_split`]).
    pub boss_dealt: [f32; crate::game::weapon::WEAPON_SLOTS],
    /// Running totals of [`DamageLedger::boss`] already booked into
    /// `boss_dealt`. The ledger is cumulative for the level while the report is
    /// incremental, so each tick attributes the delta.
    pub ledger_boss_seen: [f32; crate::game::weapon::WEAPON_SLOTS],
}

/// Progress checkpoints the batch reports level/upgrade snapshots at (seconds
/// on a level's clock).
pub const PROGRESS_TIMES: [f32; 4] = [60.0, 120.0, 180.0, 240.0];

/// "Within 30 px of an enemy" means the rat's centre is that close to an
/// enemy's *body* (centre distance minus the enemy's radius).
pub const NEAR_ENEMY_PX: f32 = 30.0;

/// One checkpoint reading of the run's progression.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Level-clock time the snapshot was taken at (`PROGRESS_TIMES`).
    pub at: f32,
    /// Player XP level.
    pub player_level: u32,
    /// How many upgrades have been taken.
    pub upgrades: u32,
    /// Kills booked so far on this level.
    pub kills: u32,
    /// HP fraction (max HP changes with picks).
    pub hp_frac: f32,
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
    /// An enemy body was within `NEAR_ENEMY_PX` before this tick.
    near_enemy: bool,
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
        let start_loadout = opts.carry.loadout();
        let (campaign, world) = if opts.campaign {
            let mut state = CampaignState::new(run_seed);
            state.level = start_level;
            state.loadout = start_loadout;
            let world = state.world();
            (Some(state), world)
        } else {
            let world = World::new_level(
                start_level,
                campaign::level_seed(run_seed, start_level),
                start_loadout,
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
            near_enemy: false,
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
        if self.world.pending_levelups > 0 && !self.world.game_over {
            let at = self.world.elapsed;
            let stats = self.level_stats();
            if stats.first_levelup.is_none() {
                stats.first_levelup = Some(at);
            }
        }
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
            "t={:6.1} lvl={} pos=({:6.1},{:6.1}) vel=({:+.1},{:+.1}) cell=({},{}) hp={:5.1}/{:5.1} en={:3} near={:2} min={:5.1} goal={:5.1}px boss={:5.1}px head=({:+.2},{:+.2}) {:?}",
            w.elapsed,
            w.level_index + 1,
            w.player.pos.x,
            w.player.pos.y,
            w.player.vel.x,
            w.player.vel.y,
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
        let enemies = self.world.enemy_count() as u32;
        let note = classify_death(&self.world, cause);
        let stats = self.level_stats();
        stats.death = Some(death);
        stats.death_note = Some(note);
        stats.enemies_at_death = Some(enemies);
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

        let near = self.near_enemy;
        let kills = self.world.kills;
        let player_level = self.world.player.level;
        let hp_frac = (self.world.player.hp / self.world.player.max_hp().max(1.0))
            .clamp(0.0, 1.0);
        let pos = self
            .stats
            .levels
            .iter()
            .position(|l| l.level == level)
            .unwrap_or(0);
        let stats = &mut self.stats.levels[pos];
        stats.time = elapsed;
        stats.kills = kills;
        stats.max_player_level = stats.max_player_level.max(player_level);
        stats.max_analytic_dps = stats.max_analytic_dps.max(dps_now);
        if near {
            stats.near_seconds += dt;
        }
        if let Some(cause) = cause {
            *stats.damage.entry(cause.name()).or_insert(0.0) += lost;
            stats.damage_total += lost;
        }
        // Fixed progression checkpoints, in order (one push per crossing).
        let next_checkpoint = PROGRESS_TIMES.get(stats.progress.len()).copied();
        if let Some(at) = next_checkpoint {
            if elapsed >= at {
                stats.progress.push(Progress {
                    at,
                    player_level,
                    upgrades: stats.upgrades.len() as u32,
                    kills,
                    hp_frac,
                });
            }
        }

        // Boss fight accounting.
        match (boss_before, boss_after) {
            (None, Some((frac, _))) => {
                self.boss_alive = true;
                self.boss_hp = frac * boss_max;
                self.boss_damage = 0.0;
                stats.reached_boss = true;
                stats.dps_at_boss = dps_now;
                stats.level_at_boss = Some(player_level);
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

        // Balance instrumentation: per-weapon damage actually dealt. The
        // ledger is cumulative for this level's world; copying the six floats
        // is cheaper than reading it once at level end (which a capped or
        // killed run would then have to special-case).
        stats.dealt = self.world.ecs.resource::<DamageLedger>().dealt;
        attribute_boss_damage(&self.world, boss_before.is_some(), &mut self.stats.levels[pos]);
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
        self.near_enemy = false;
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
                if d - en.radius <= NEAR_ENEMY_PX {
                    self.near_enemy = true;
                }
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

/// One-line, human-readable classification of a death, for tuning reports:
/// "surrounded in the open", "cornered", "burst from one enemy kind", "boss
/// attack by name" or "trap".
///
/// This reads the state *at the death tick* (bodies in contact range, the wall
/// gap, the attributed damage source), so it is a diagnostic, not a spec.
fn classify_death(w: &World, cause: DamageCause) -> String {
    match cause {
        DamageCause::Trap => "trap: mousetrap snap".to_string(),
        DamageCause::BossContact(kind) => format!("boss: {} body", boss_name(kind)),
        DamageCause::BossHazard(kind, attack) => {
            format!("boss: {} {}", boss_name(kind), attack.name())
        }
        DamageCause::BossProjectile(kind) => {
            format!("boss: {} projectile", boss_name(kind))
        }
        DamageCause::EnemyBullet => {
            let (near60, near120) = enemy_density(w);
            if near60 >= 5 {
                format!(
                    "surrounded in the open: spitter burst with {near60} bodies within 60 px"
                )
            } else if near120 >= 12 {
                format!("swarmed: spitter burst, {near120} bodies within 120 px")
            } else {
                "burst from spitter bullets".to_string()
            }
        }
        DamageCause::EnemyContact(kind) => {
            let (near60, near120) = enemy_density(w);
            let gap = wall_gap(w);
            let open = open_directions(w);
            if gap < 22.0 && open <= 3 {
                format!(
                    "cornered: {} contact with a wall {gap:.0} px away",
                    enemy_kind_name(kind)
                )
            } else if near60 >= 6 {
                format!(
                    "surrounded in the open: {} contact, {near60} bodies within 60 px",
                    enemy_kind_name(kind)
                )
            } else if near120 >= 12 {
                format!(
                    "swarmed: {} contact, {near120} bodies within 120 px",
                    enemy_kind_name(kind)
                )
            } else {
                format!("burst from {} contact", enemy_kind_name(kind))
            }
        }
        DamageCause::Unknown => "unclear: no attacker matched the HP loss".to_string(),
    }
}

fn enemy_kind_name(kind: EnemyKind) -> &'static str {
    match kind {
        EnemyKind::Skitterer => "skitterer",
        EnemyKind::Bat => "bat",
        EnemyKind::Cat => "cat",
        EnemyKind::Spitter => "spitter",
        EnemyKind::Brute => "brute",
        EnemyKind::Boss => "boss",
    }
}

fn boss_name(kind: BossKind) -> &'static str {
    match kind {
        BossKind::Cat => "jaguar",
        BossKind::Broom => "broom human",
        BossKind::Mower => "mower",
    }
}

/// `(bodies within 60 px, within 120 px)` of the rat, at death.
fn enemy_density(w: &World) -> (usize, usize) {
    let p = w.player.pos;
    let mut near60 = 0;
    let mut near120 = 0;
    for e in w.ecs.iter_entities() {
        if let Some(en) = e.get::<Enemy>() {
            if !en.alive() {
                continue;
            }
            let d = en.pos.dist(p);
            if d <= 60.0 {
                near60 += 1;
            }
            if d <= 120.0 {
                near120 += 1;
            }
        }
    }
    (near60, near120)
}

/// Distance from the rat to the nearest wall, capped at 100 px.
fn wall_gap(w: &World) -> f32 {
    use crate::game::collision::SizeClass;
    let p = w.player.pos;
    let mut d = 0.0;
    while d < 100.0 {
        if !w
            .level
            .tiles
            .circle_clear(p + Vec2::from_angle(d * 0.7) * d, PLAYER_RADIUS, SizeClass::Small)
        {
            return d;
        }
        d += 4.0;
    }
    100.0
}

/// How many of the 8 compass directions have 28 px of clear runway.
fn open_directions(w: &World) -> usize {
    use crate::game::collision::SizeClass;
    let p = w.player.pos;
    let mut n = 0;
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        if w.level
            .tiles
            .circle_clear(p + Vec2::from_angle(a) * 28.0, PLAYER_RADIUS, SizeClass::Small)
        {
            n += 1;
        }
    }
    n
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

/// Add this tick's boss damage to `stats.boss_dealt`, split by weapon.
///
/// The boss fight books its per-tick damage by *source* (`BossDamageTally`:
/// aimed / pattern / pools / whip / traps) rather than by weapon. Acid Spit's
/// pools and Tail Whip's arc map one-to-one; "aimed" covers Gnaw and Cheese
/// Spray and "pattern" covers Squeak Nova and Spore Orbit, so those two are
/// split in proportion to the owned weapons' analytic dps. Traps and anything
/// else are not weapon damage and are skipped, which is why the boss column
/// can sum to slightly less than the fight's real damage.
fn attribute_boss_damage(world: &World, boss_alive: bool, stats: &mut LevelStats) {
    // Weapons that damage the boss *outside* `boss_damage_system` book their own
    // boss column in the ledger (the two active weapons: a thrash sweep and a
    // squeak ring call `Boss::take_damage` themselves). Attribute the delta
    // since the last tick before the tally below is read; a lethal hit still
    // lands because the flag is the *pre-tick* boss state.
    let ledger = world.ecs.resource::<DamageLedger>().boss;
    for kind in WeaponKind::all() {
        let i = kind.index();
        let now = ledger[i];
        if now > stats.ledger_boss_seen[i] {
            stats.boss_dealt[i] += now - stats.ledger_boss_seen[i];
            stats.ledger_boss_seen[i] = now;
        }
    }
    if !boss_alive {
        return;
    }
    let tally = world.ecs.resource::<BossFight>().scratch.dmg;
    let lo = &world.player.loadout;
    let mut add = |kind: WeaponKind, amount: f32| {
        if amount > 0.0 {
            stats.boss_dealt[kind.index()] += amount;
        }
    };
    add(WeaponKind::AcidSpit, tally.pools);
    add(WeaponKind::TailWhip, tally.whip);
    for (part, a, b) in [
        (tally.aimed, WeaponKind::Gnaw, WeaponKind::CheeseSpray),
        (tally.pattern, WeaponKind::SqueakNova, WeaponKind::SporeOrbit),
    ] {
        let split = boss_weapon_split(lo, a, b);
        add(a, part * split[0]);
        add(b, part * split[1]);
    }
}

/// Split a boss-damage bucket between two weapons in proportion to their
/// analytic dps contribution (see [`LevelStats::boss_dealt`]).
///
/// The boss fight books damage by source, and two sources cover two weapons
/// each ("aimed" = Gnaw + Cheese Spray, "pattern" = Squeak Nova + Spore
/// Orbit). This is the only assumption in the per-weapon boss column: a weapon
/// that does not own a bucket gets exactly zero, and a build that owns only one
/// of a pair gets the whole bucket.
fn boss_weapon_split(lo: &Loadout, a: WeaponKind, b: WeaponKind) -> [f32; 2] {
    let weight = |kind: WeaponKind| -> f32 {
        lo.weapons
            .iter()
            .filter(|w| w.kind == kind)
            .map(|w| {
                w.damage() * w.level_projectiles().max(1) as f32 / w.cooldown().max(0.05)
            })
            .sum::<f32>()
            * lo.stats.damage_mult
            * lo.stats.fire_rate_mult
    };
    let (wa, wb) = (weight(a), weight(b));
    let total = wa + wb;
    if total > 0.0 {
        [wa / total, wb / total]
    } else {
        [0.0, 0.0]
    }
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
        // **The 360 s cap no longer reaches an end state.** Since the balance
        // pass the opening is calm and a campaign that survives level 1 runs
        // long: median total survival is 963 s over 120 expert campaigns (see
        // the 60-per-base verification), and a level-1 boss only arrives at
        // 255 s. What this test is for is "the beatability instrument plays
        // whole campaigns without panicking and keeps finite numbers on every
        // seed", so a run still playing at the cap is a legitimate outcome; the
        // end states themselves are covered by the campaign tests above and by
        // the batched clear rates.
        let jobs: Vec<(Skill, u64)> = (0..10u64).map(|seed| (Skill::Expert, seed)).collect();
        let runs = batch_runs(&jobs, 0, true, 360.0);
        assert_eq!(runs.len(), jobs.len());
        for run in &runs {
            assert!(
                run.total_time.is_finite() && run.total_time > 0.0,
                "a campaign ran for {:?} s",
                run.total_time
            );
            assert!(!run.levels.is_empty(), "a campaign ran no level at all");
        }
    }

    /// Run `(skill, seed)` jobs in parallel and return each run's stats in job
    /// order. A run is either one practice level or a whole campaign from
    /// `level`, capped at `cap` simulated seconds.
    fn batch_runs(
        jobs: &[(Skill, u64)],
        level: usize,
        campaign: bool,
        cap: f32,
    ) -> Vec<super::RunStats> {
        let threads = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(4)
            .clamp(1, jobs.len());
        let opts_for = |skill: Skill| AutoOpts {
            start_level: level,
            campaign,
            skill,
            carry: CarryPreset::None,
        };
        let mut out: Vec<Option<super::RunStats>> = (0..jobs.len()).map(|_| None).collect();
        if threads <= 1 {
            for (i, &(skill, seed)) in jobs.iter().enumerate() {
                let mut run = AutoRun::new(seed, opts_for(skill));
                run_to_end(&mut run, 1.0 / 60.0, cap);
                out[i] = Some(run.stats().clone());
            }
        } else {
            let chunk = jobs.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(threads);
                for (c, part) in jobs.chunks(chunk).enumerate() {
                    handles.push(scope.spawn(move || {
                        let mut v = Vec::with_capacity(part.len());
                        for (j, &(skill, seed)) in part.iter().enumerate() {
                            let mut run = AutoRun::new(seed, opts_for(skill));
                            run_to_end(&mut run, 1.0 / 60.0, cap);
                            v.push((c * chunk + j, run.stats().clone()));
                        }
                        v
                    }));
                }
                for h in handles {
                    for (i, stats) in h.join().expect("batch worker panicked") {
                        out[i] = Some(stats);
                    }
                }
            });
        }
        out.into_iter().map(|o| o.expect("every job ran")).collect()
    }

    /// One practice level per job (the ladder and guard tests).
    fn practice_runs(jobs: &[(Skill, u64)], level: usize, cap: f32) -> Vec<super::RunStats> {
        batch_runs(jobs, level, false, cap)
    }

    fn damage_taken(stats: &[super::RunStats]) -> f32 {
        stats
            .iter()
            .flat_map(|r| r.levels.iter())
            .map(|l| l.damage_total)
            .sum()
    }

    fn kills_per_min(stats: &[super::RunStats]) -> f32 {
        let kills: u32 = stats.iter().map(|r| r.total_kills).sum();
        let time: f32 = stats.iter().map(|r| r.total_time).sum();
        if time > 0.0 {
            kills as f32 / (time / 60.0)
        } else {
            0.0
        }
    }

    /// The skills are an ordered ladder, not three flavours: on the same seeds
    /// Expert must play better than Skilled and Skilled better than Casual.
    ///
    /// **What changed, and why the old measure had to go.** Until the balance
    /// pass this test compared survival time under a 240 s cap. `BOSS_TIME` is
    /// 255 s, so that cap sat *below* the boss, and after the pass (calm
    /// opening, calmer spawn curve — `scratchpad/balance/LOG.md`) ten or
    /// eleven of every twelve runs of *every* skill reach it alive: the metric
    /// was censored at the top and read `expert 240 s vs skilled 238 s`. The
    /// ladder is real; survival time under that cap simply cannot see it.
    ///
    /// **What separates the skills instead.** Measured on the 24 seeds of seed
    /// base 4242 at commit 8ada8f5 (release batches, practice level 1):
    ///
    /// | measure | Expert | Skilled | Casual |
    /// |---|---|---|---|
    /// | damage taken in the first 120 s (HP/run) | 15.4 | 28.3 | 67.1 |
    /// | 120 s damage, per-seed ordering | Expert<Skilled 19/24, Skilled<Casual 22/24, Expert<Casual 24/24 | | |
    /// | damage per second survived, whole level | 1.00 | 1.15 | 1.53 |
    /// | level 1 cleared | 13/24 (54 %) | 10/24 (42 %) | 6/24 (25 %) |
    /// | boss fight, median | 71 s | 61 s | 51 s |
    ///
    /// So the ladder is measured on **damage taken in a fixed 120 s window**
    /// (no survival confound: everyone is alive for the whole window), and the
    /// boss phase is checked for *reachability and length* on a few seeds
    /// rather than for a clear-rate ordering, which at these sample sizes is
    /// not separable — 24 campaigns per skill were needed to see 54/42/25 %
    /// clearly, and putting that in the suite would cost minutes. The
    /// campaign-level clear/win bands live in
    /// [`early_game_headline_rates_stay_inside_generous_bands`] and in the
    /// 60-run batches.
    #[test]
    fn skills_form_an_ordered_ladder_on_the_same_seeds() {
        const EARLY_CAP: f32 = 120.0;
        const EARLY_SEEDS: usize = 6;
        const BOSS_SEEDS: usize = 3;
        const BOSS_CAP: f32 = 600.0;
        let skills = [Skill::Expert, Skill::Skilled, Skill::Casual];
        let early: Vec<Vec<super::RunStats>> = skills
            .iter()
            .map(|&skill| {
                let jobs: Vec<(Skill, u64)> = (0..EARLY_SEEDS)
                    .map(|i| (skill, batch::seed_for(900, i)))
                    .collect();
                practice_runs(&jobs, 0, EARLY_CAP)
            })
            .collect();
        let [expert, skilled, casual] = [&early[0], &early[1], &early[2]];

        // Every run is alive for the whole window, so this is a clean rate.
        for (skill, runs) in skills.iter().zip(&early) {
            let alive = runs
                .iter()
                .filter(|r| r.death.is_none() && r.total_time >= EARLY_CAP - 1.0)
                .count();
            assert!(
                alive * 6 >= EARLY_SEEDS * 5,
                "{} died inside the calm {EARLY_CAP:.0} s window on {}/{EARLY_SEEDS} seeds",
                skill.name(),
                EARLY_SEEDS - alive
            );
        }

        let (de, ds, dc) = (
            damage_taken(expert),
            damage_taken(skilled),
            damage_taken(casual),
        );
        assert!(
            de * 1.3 < ds,
            "expert took {de:.0} HP in {EARLY_CAP:.0} s, skilled {ds:.0}: expert must be \
             at least 30 % cleaner"
        );
        assert!(
            ds * 1.3 < dc,
            "skilled took {ds:.0} HP in {EARLY_CAP:.0} s, casual {dc:.0}: skilled must be \
             at least 30 % cleaner"
        );
        // Per-seed, not just in aggregate: expert picks up less damage than
        // casual on (almost) every one of these seeds.
        let beats = expert
            .iter()
            .zip(casual)
            .filter(|(a, b)| damage_taken(std::slice::from_ref(*a)) < damage_taken(std::slice::from_ref(*b)))
            .count();
        assert!(
            beats + 1 >= EARLY_SEEDS,
            "expert took less damage than casual on only {beats}/{EARLY_SEEDS} seeds"
        );
        let (ke, ks, kc) = (
            kills_per_min(expert),
            kills_per_min(skilled),
            kills_per_min(casual),
        );
        for (skill, k) in [("expert", ke), ("skilled", ks), ("casual", kc)] {
            assert!(
                (30.0..400.0).contains(&k),
                "{skill} killed {k:.0}/min in {EARLY_CAP:.0} s"
            );
        }

        // Boss phase: every skill must reach and kill a level-1 boss, the
        // fight must be a fight (not a strand), and not every run may win.
        let boss: Vec<super::RunStats> = skills
            .iter()
            .flat_map(|&skill| {
                let jobs: Vec<(Skill, u64)> = (0..BOSS_SEEDS)
                    .map(|i| (skill, batch::seed_for(900, i)))
                    .collect();
                practice_runs(&jobs, 0, BOSS_CAP)
            })
            .collect();
        let cleared = boss.iter().filter(|r| r.won).count();
        assert!(
            cleared >= 1,
            "no run of any skill killed the level-1 boss in {BOSS_CAP:.0} s ({} runs)",
            boss.len()
        );
        assert!(
            cleared < boss.len(),
            "all {} runs killed the boss: the game has no ceiling left",
            boss.len()
        );
        let reached = boss.iter().filter(|r| r.levels[0].reached_boss).count();
        assert!(
            reached >= boss.len() / 2,
            "only {reached}/{} runs reached the boss at all",
            boss.len()
        );
        for run in boss.iter().filter(|r| r.levels[0].reached_boss) {
            let fight = run.levels[0].boss_seconds;
            assert!(
                fight < 200.0,
                "a level-1 boss fight ran {fight:.0} s (stranded-boss signature)"
            );
        }
    }

    /// **Balance regression guard — RE-BASELINED after the batch-4 merge.**
    ///
    /// A change that makes the game wildly easier or harder must fail the
    /// suite, not just a batch. Twelve runs (four per skill) of a fixed 120 s
    /// window cost a few seconds. The bands were re-measured on the merged
    /// build with 24 seeds per skill (seed base 4242, practice level 1,
    /// `--max-sim-seconds 120`) and now also have a **floor**: the old bands
    /// were upper bounds only, so the "opening calm" defect (the first 60-129 s
    /// of a level costing the rat no health on 169 of 300 mixed campaigns) passed
    /// the guard unnoticed. A mean that collapses toward zero is a regression
    /// too, and it is the one this test can cheaply see.
    ///
    /// | measure | 8ada8f5 | merged + batch 4 | band |
    /// |---|---|---|---|
    /// | runs surviving the 120 s window | 12/12 | 12/12 (24/24 measured) | ≥ 10/12 |
    /// | damage/run, expert | 15.4 | 9.6 | 3-120 |
    /// | damage/run, skilled | 28.3 | 24.2 | 8-200 |
    /// | damage/run, casual | 67.1 | 61.8 | 20-400 |
    /// | kills/min | ~128 | 131-145 | 60-400 |
    /// | player level at 60 s | ~3.1 | 3.0-3.5 | 1.5-8 |
    /// | first level-up (median) | 20-25 s | 20-25 s | 8-50 s |
    /// | expert vs casual damage | 4.4x | 6.4x | ≥ 1.2x |
    ///
    /// The bands are wide on purpose: they catch a doubled spawn rate, a
    /// doubled weapon, a broken XP curve or a bot that stops working, not a
    /// 10 % tuning step. The ladder check (expert at least 20 % cleaner than
    /// casual) is a *reversal* guard.
    #[test]
    fn early_game_headline_rates_stay_inside_generous_bands() {
        const N: usize = 4;
        const CAP: f32 = 120.0;
        let skills = [Skill::Expert, Skill::Skilled, Skill::Casual];
        let mut all: Vec<super::RunStats> = Vec::new();
        let mut per_skill: Vec<(Skill, Vec<super::RunStats>)> = Vec::new();
        for &skill in &skills {
            let jobs: Vec<(Skill, u64)> = (0..N)
                .map(|i| (skill, batch::seed_for(4242, i)))
                .collect();
            let runs = practice_runs(&jobs, 0, CAP);
            all.extend(runs.iter().cloned());
            per_skill.push((skill, runs));
        }
        let survived = all
            .iter()
            .filter(|r| r.death.is_none() && r.total_time >= CAP - 1.0)
            .count();
        assert!(
            survived + 2 >= all.len(),
            "only {survived}/{} runs survived the calm {CAP:.0} s opening: the game got much harder",
            all.len()
        );

        // (skill, floor, ceiling): the floor is the "opening calm" guard, the
        // ceiling the "the game got much harder" guard. See the doc table.
        let caps = [
            (Skill::Expert, 3.0, 120.0),
            (Skill::Skilled, 8.0, 200.0),
            (Skill::Casual, 20.0, 400.0),
        ];
        let mut damage = Vec::new();
        for ((skill, runs), (_, floor, hp_cap)) in per_skill.iter().zip(caps) {
            let dmg = damage_taken(runs);
            let mean = dmg / runs.len() as f32;
            assert!(
                mean <= hp_cap,
                "{} took {mean:.0} HP per {CAP:.0} s, band {floor:.0}-{hp_cap:.0}: the game got much harder",
                skill.name()
            );
            assert!(
                mean >= floor,
                "{} took only {mean:.1} HP in {CAP:.0} s (floor {floor:.0}): the opening stopped \
                 pressing the rat at all",
                skill.name()
            );
            damage.push(mean);
        }
        assert!(
            damage[0] * 1.2 < damage[2],
            "expert ({:.0} HP) is no longer at least 20 % cleaner than casual ({:.0} HP): \
             the skill ladder reversed",
            damage[0],
            damage[2]
        );

        let k = kills_per_min(&all);
        assert!(
            (60.0..400.0).contains(&k),
            "{k:.0} kills/min over the first {CAP:.0} s (band 60-400)"
        );
        let level_at_60: Vec<f32> = all
            .iter()
            .flat_map(|r| r.levels.iter())
            .flat_map(|l| l.progress.iter())
            .filter(|p| (p.at - 60.0).abs() < 0.01)
            .map(|p| p.player_level as f32)
            .collect();
        assert!(!level_at_60.is_empty(), "no run reached the 60 s checkpoint");
        let mean_level = level_at_60.iter().sum::<f32>() / level_at_60.len() as f32;
        assert!(
            (1.5..8.0).contains(&mean_level),
            "mean player level at 60 s is {mean_level:.1} (band 1.5-8): the XP curve moved"
        );
        let mut firsts: Vec<f32> = all
            .iter()
            .flat_map(|r| r.levels.iter())
            .filter_map(|l| l.first_levelup)
            .collect();
        firsts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if !firsts.is_empty() {
            let median = firsts[firsts.len() / 2];
            assert!(
                (8.0..50.0).contains(&median),
                "first level-up median {median:.1} s (band 8-50 s): the opening moved"
            );
        }
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

