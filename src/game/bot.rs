//! The autoplay bot: a test instrument that plays Rat Run.
//!
//! The bot is deliberately **not** a fair opponent: it reads the simulation
//! (enemy positions and velocities, enemy bullets, every telegraphed boss
//! hazard with its remaining warn time, boss state, trap pads, the tile map)
//! and plays it like a strong human. It feeds `move_dir`/`dash` into [`World::update`] and picks upgrades,
//! and claims chest rewards through the same path as the App.
//!
//! ## The planning model
//!
//! Two layers, because the two failure modes are different:
//!
//! * **The plan (macro, 2.4–3.2 s).** A waypoint 160–200 px away is chosen from
//!   a rotating sweep through *open* floor: candidates are scored by local
//!   walkable area, distance from the crowd centroid, continuation of the
//!   current loop and enemy density around the target. The waypoint is held
//!   for its whole window — that is the committed heading; it is re-rolled
//!   when it is reached or its timer expires. (`update_goal` is the only place
//!   a new leg is chosen, so the rat laps the open rooms instead of jittering
//!   in place.)
//! * **Steering (micro).** Every tick the bot scores 16 compass headings (plus
//!   "stay") on a danger field. Expert additionally **commits its movement
//!   heading for 0.08 s** (five ticks) and re-plans on that cadence, while
//!   scanning and dashing stay live every tick. The 0.08 s hold is the
//!   heading-commitment A/B: it stops two near-equal directions oscillating,
//!   but freezing the *dash* with it cost 20 s of median survival, so the
//!   sensors and the burst never wait for the cadence. The human skills are
//!   gated 0.25 s / 0.45 s end to end — that delay is their handicap.
//!
//! **Planning horizon:** three samples at 0.12 / 0.27 / 0.48 s (weighted
//! 0.50/0.32/0.18). At the base 58 px/s that is 7–28 px of rat travel and
//! 3.6–20 px of a Cat's, i.e. everything that can reach contact range inside a
//! reaction plus one wind-up; a longer lookahead changed no decision (the
//! danger field only fires inside ~34 px) but cost CPU. The *escape* probe is
//! longer: mass at 85 / 165 / 250 px out (weights 1.0/0.55/0.3) decides which
//! arc is thinnest when the rat is inside a pack, and the waypoint leg
//! (160–200 px) is ~3–4 s of travel — the horizon at which the spawn ring and
//! the next room matter.
//!
//! ## Behaviours, in priority order
//!
//! 1. **Do not get hit.** Candidates are scored by a danger field built from
//!    enemies (predicted with their velocity: `Enemy::steer` smooths toward
//!    `EnemyKind::speed`, so the sample already carries the kind's chase),
//!    enemy bullets (closest-approach test), boss body shapes, telegraphed
//!    hazards, boss projectiles, glue pads and armed mousetraps. Walls are
//!    hard and a short runway is penalised. **Fast chasers:** the Cat is 38
//!    px/s and 12 contact damage against a 58 px/s rat; it catches the rat only
//!    by cutting across a turn, and a hit is capped at one body's damage per
//!    0.6 s, so the rules are: never let a Cat (or Brute, `damage >= 10`) sit
//!    on the rat (dash), and treat a pack of three bodies within 14 px (~0.4 s
//!    at a Cat's chase) as the encirclement trigger. The kinds' speeds
//!    (Skitterer 30, Bat 42, Cat 38, Spitter 22, Brute 16) also widen a fast
//!    body's danger trigger a little (`0.9 x speed`, so the bat is read at
//!    38 px and the crawlers at 34).
//! 2. **Kite in wide loops through open floor.** The waypoint sweep prefers
//!    local walkable area and headings that continue the lap; a leg that
//!    reverses is penalised (`-4 x` the reversal), which is what stops the rat
//!    turning back into the pack it just outran.
//! 3. **Dash with intent.** The dash is spent off a body on the rat, off a
//!    projectile whose closest approach lands within 0.22 s, off a hazard
//!    firing inside the footprint, out of an encirclement, or off the Cat's
//!    locked pounce ring — never on cooldown for its own sake.
//! 4. **Collect XP aggressively.** XP is the whole progression: a gem's pull
//!    is up to 10 points against ~6 for safety, guarded only by enemies sitting
//!    on it. The measured A/B is blunt — an expert that avoids the pack kills
//!    fewer bodies, levels less and dies *earlier*; gem greed is what starts
//!    the snowball.
//! 5. **Early survival picks.** Armour, max HP and the first Whiskers stack
//!    outrank raw damage while the build has fewer than 8 upgrades; a flat
//!    armour point cuts 98 % of the damage sources in this game (contact).
//! 6. **Boss counters.** The generic hazard field covers the telegraphs; on
//!    top of it the Cat's locked pounce ring is sidestepped at the last moment
//!    and the mower's charge corridor is left early.
//!
//! ## Skills
//!
//! An ordered ladder from a strong instrument to a beginner:
//!
//! | | Expert | Skilled | Casual |
//! |---|---|---|---|
//! | reactions | 0.08 s heading hold, live sensors/dash | 0.25 s | 0.45 s |
//! | horizon | 0.48 s | 0.22 s | 0.14 s |
//! | vision | whole simulation | screen + explored minimap | screen only |
//! | hazard timer read | exact | quantised to 0.2 s | quantised to 0.4 s |
//! | steering noise | 0 | 0.12 rad | 0.42 rad |
//! | dash | deliberate | deliberate | panic only |
//! | boss counters | yes | yes (art telegraphs) | no |
//! | building | optimal | good (8 % pick noise) | sloppy (35 %) |
//!
//! `Skilled` is a competent human: it only knows what is on screen plus the
//! cells the minimap has already explored (unexplored cells are assumed open,
//! so it bumps into unseen walls), and it does not read internal timers beyond
//! what the art telegraphs. `Casual` is a new player: late, noisy, short
//! sight, no deliberate dash, shaky read of the upgrade cards.
//!
//! ## Determinism
//!
//! Every random draw comes from the bot's own [`StdRng`], seeded from the run
//! seed, in a fixed order and only where the skill uses randomness (steering
//! noise, pick jitter, panic dashes). Chest rewards are rolled by the World
//! when claimed, using the same stream as the App.

use rand::rngs::StdRng;
use rand::RngExt;
use rand::SeedableRng;

use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::Keepable;
use crate::game::collision::{cell_of, SizeClass};
use crate::game::gem::{Gem, Pickup};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::systems::boss::{Boss, BossProjectile, HazardShape};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::PickupKind;
use crate::game::loadout::{Loadout, PassiveKind, Upgrade};
use crate::game::weapon::WeaponKind;
use crate::game::world::World;
use crate::math::Vec2;

/// Player collision radius where the simulation defines it as a fixed count.
/// Where the simulation uses `PLAYER_RADIUS`, the bot does too.
const R: f32 = PLAYER_RADIUS;
/// How far from the rat a threat must be before it stops being scanned.
const SCAN_RADIUS: f32 = 230.0;
/// Number of compass directions the bot scores each decision.
const DIRS: usize = 16;
/// Skin gap (px) inside which a live body forbids a "stand still" decision:
/// six px past contact. The freeze this breaks happens with bodies already on
/// the rat, so the trigger is deliberately narrow: a wider one (40 px) was
/// measured *worse* — pushing the rat to move whenever a body was within 40 px
/// cost wins in a paired 200-campaign batch, because a brief standstill inside
/// a pack is part of how the bot kites.
const FOE_ON_RAT_PX: f32 = 6.0;

/// How well the bot plays; see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skill {
    /// The measuring instrument: 0.08 s heading commitment, no noise, every
    /// counter, full information.
    Expert,
    /// A competent human: 0.25 s reactions, screen + explored minimap only.
    Skilled,
    /// A new player: late, noisy, short sight, no deliberate dash.
    Casual,
}

impl Skill {
    pub fn name(self) -> &'static str {
        match self {
            Skill::Expert => "expert",
            Skill::Skilled => "skilled",
            Skill::Casual => "casual",
        }
    }

    /// Ladder position, high to low (used by the ordering tests).
    pub fn tier(self) -> u8 {
        match self {
            Skill::Expert => 3,
            Skill::Skilled => 2,
            Skill::Casual => 1,
        }
    }

    /// Parse the `--skill` command-line value.
    pub fn parse(s: &str) -> Option<Skill> {
        match s.to_ascii_lowercase().as_str() {
            "expert" => Some(Skill::Expert),
            "skilled" => Some(Skill::Skilled),
            "casual" => Some(Skill::Casual),
            _ => None,
        }
    }
}

/// How much of the world a skill may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vision {
    /// Enemy positions and timers anywhere within `SCAN_RADIUS`, and the whole
    /// tile map.
    Full,
    /// On-screen bodies only (a rectangle centred on the rat), and only the
    /// map cells the minimap has explored.
    Screen,
}

/// One tick of input the bot wants to feed the simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotInput {
    pub move_dir: Vec2,
    pub dash: bool,
}

impl BotInput {
    pub const IDLE: BotInput = BotInput {
        move_dir: Vec2::ZERO,
        dash: false,
    };
}

/// The skill's reaction model.
#[derive(Clone, Copy, Debug)]
struct Params {
    /// Seconds between decisions (`0` = every tick).
    reaction: f32,
    /// Angular noise added to the chosen direction (radians).
    noise: f32,
    /// Multiplier on the danger field.
    safety: f32,
    /// Seconds of enemy-motion prediction.
    horizon: f32,
    /// Sidestep the cat's locked pounce ring at the last moment.
    boss_counters: bool,
    /// Spend the dash out of telegraphed hazards.
    smart_dash: bool,
    /// Treat glue pads and armed mousetraps as obstacles.
    trap_aware: bool,
    /// How much XP-gem greed competes with safety.
    greed: f32,
    /// Weight of the soft crowd gradient (enemies further than contact range).
    crowd: f32,
    /// Weight of "keep going the way you were going".
    momentum: f32,
    /// Graze the horde: creep at a controlled speed so kills (and the XP
    /// gems they drop) happen within pickup range instead of 60 px behind.
    standoff: bool,
    /// Keep scanning and dashing every tick while the heading is committed.
    instant_dash: bool,
    /// What the skill may read.
    vision: Vision,
    /// Quantise hazard warn timers to this bucket (0 = exact).
    hazard_quantum: f32,
    /// Relative error applied to upgrade-card values.
    pick_noise: f32,
}

impl Params {
    fn for_skill(skill: Skill) -> Params {
        match skill {
            Skill::Expert => Params {
                // 0.08 s (five ticks) is the shortest commitment that stops
                // per-tick oscillation. Dash and scanning stay live every tick
                // (`instant_dash`): freezing those with the heading cost 20 s of
                // median survival in the A/B, because a bullet-hell dash cannot
                // wait out a reaction interval.
                reaction: 0.08,
                noise: 0.0,
                safety: 1.0,
                horizon: 0.30,
                boss_counters: true,
                smart_dash: true,
                trap_aware: true,
                greed: 10.0,
                crowd: 2.2,
                momentum: 1.6,
                standoff: false,
                instant_dash: true,
                vision: Vision::Full,
                hazard_quantum: 0.0,
                pick_noise: 0.0,
            },
            Skill::Skilled => Params {
                reaction: 0.25,
                noise: 0.12,
                safety: 0.88,
                horizon: 0.22,
                boss_counters: true,
                smart_dash: true,
                trap_aware: true,
                greed: 5.0,
                crowd: 1.5,
                momentum: 1.4,
                standoff: false,
                instant_dash: false,
                vision: Vision::Screen,
                hazard_quantum: 0.2,
                pick_noise: 0.08,
            },
            Skill::Casual => Params {
                reaction: 0.45,
                noise: 0.42,
                safety: 0.62,
                horizon: 0.14,
                boss_counters: false,
                smart_dash: false,
                trap_aware: false,
                greed: 1.0,
                crowd: 0.8,
                momentum: 1.0,
                standoff: false,
                instant_dash: false,
                vision: Vision::Screen,
                hazard_quantum: 0.4,
                pick_noise: 0.35,
            },
        }
    }
}

/// A hazard / body the danger field reacts to, snapshotted once per decision.
#[derive(Clone, Copy, Debug)]
enum Threat {
    Enemy {
        pos: Vec2,
        vel: Vec2,
        radius: f32,
        damage: f32,
        kind: EnemyKind,
    },
    Bullet {
        pos: Vec2,
        vel: Vec2,
        radius: f32,
        damage: f32,
    },
    /// The boss's body, through its art hit shapes.
    BossBody {
        pos: Vec2,
        radius: f32,
        damage: f32,
    },
    /// A telegraphed boss hazard.
    Hazard {
        shape: HazardShape,
        warn_left: f32,
        active_left: f32,
        damage: f32,
    },
    /// A boss projectile (blade disc, fang, shard).
    Projectile {
        pos: Vec2,
        vel: Vec2,
        radius: f32,
        damage: f32,
    },
    StickyPad {
        pos: Vec2,
        facing: Vec2,
    },
    Mousetrap {
        pos: Vec2,
        facing: Vec2,
        armed: bool,
    },
}

/// The bot: policy state plus its own rng stream.
pub struct Bot {
    skill: Skill,
    params: Params,
    rng: StdRng,
    /// Seconds since the last decision (Casual holds its input in between).
    since_decision: f32,
    held: BotInput,
    /// Direction of the loop the bot is running (radians, rotates).
    wander: f32,
    /// Which way the loop turns (`+1` / `-1`); stable so runs are cycles.
    orbit_sign: f32,
    /// Current waypoint.
    goal: Vec2,
    goal_timer: f32,
    /// How far the current waypoint is (the loop's legs get longer when the
    /// bot is kiting a crowd and shorter when it is collecting).
    goal_radius: f32,
    /// Last chosen heading: kiting wants continuity, not jitter.
    heading: Vec2,
    /// Reused threat / gem scratch so a decision allocates nothing.
    threats: Vec<Threat>,
    gems: Vec<(Vec2, u32)>,
    pickups: Vec<(Vec2, PickupKind)>,
    /// Level the policy was reset for (keeps goal state per level).
    level: usize,
    /// Boss seen this level, for the counter rules.
    boss_pos: Option<Vec2>,
    boss_kind: Option<crate::game::ecs::systems::boss::BossKind>,
    decisions: u64,
}

impl Bot {
    /// A bot with its own predictable rng stream (never the simulation's).
    pub fn new(skill: Skill, seed: u64) -> Bot {
        Bot {
            skill,
            params: Params::for_skill(skill),
            rng: StdRng::seed_from_u64(seed ^ 0xB07_u64),
            since_decision: f32::INFINITY,
            held: BotInput::IDLE,
            wander: 0.0,
            orbit_sign: 1.0,
            goal: Vec2::ZERO,
            goal_timer: 0.0,
            goal_radius: 200.0,
            heading: Vec2::ZERO,
            threats: Vec::with_capacity(256),
            gems: Vec::with_capacity(64),
            pickups: Vec::with_capacity(8),
            level: 0,
            boss_pos: None,
            boss_kind: None,
            decisions: 0,
        }
    }

    pub fn skill(&self) -> Skill {
        self.skill
    }

    /// `(goal, heading)` — diagnostics for `--bot-batch --trace`.
    pub fn debug_goal(&self) -> (Vec2, Vec2) {
        (self.goal, self.heading)
    }

    /// Forget per-level state when the campaign advances to the next map.
    pub fn on_new_level(&mut self, level: usize) {
        self.level = level;
        self.goal = Vec2::ZERO;
        self.goal_timer = 0.0;
        self.heading = Vec2::ZERO;
        self.wander = self.rng.random::<f32>() * std::f32::consts::TAU;
        self.orbit_sign = if self.rng.random::<bool>() { 1.0 } else { -1.0 };
        self.held = BotInput::IDLE;
        self.since_decision = f32::INFINITY;
        self.boss_pos = None;
        self.boss_kind = None;
    }

    /// One decision: the movement/dash input for this tick.
    ///
    /// Called every tick by `AutoRun::step`; the bot itself decides whether it
    /// actually re-plans (`Params::reaction`).
    pub fn decide(&mut self, w: &World, dt: f32) -> BotInput {
        self.since_decision += dt;
        let due = self.params.reaction <= 0.0 || self.since_decision >= self.params.reaction;
        // Expert keeps its sensors and its dash live every tick: only the
        // movement *heading* is committed for the reaction interval. A human
        // skill's reaction delay gates everything (that is what the delay is).
        let instant = !due && self.params.instant_dash;
        if instant {
            self.scan(w);
            let dirs = unit_dirs();
            // One press is both the dash and the active-weapon release, so the
            // bot presses when either wants it.
            let dash = self.wants_dash(w, &dirs) || self.wants_active(w);
            let input = BotInput {
                move_dir: self.held.move_dir,
                dash,
            };
            self.held = input;
            return input;
        }
        if !due {
            return self.held;
        }
        let since = self.since_decision;
        self.since_decision = 0.0;
        self.decisions += 1;

        self.scan(w);
        self.update_goal(w, since);

        let pos = w.player.pos;
        let dirs = unit_dirs();
        let mut best_dir = Vec2::ZERO;
        let mut best_score = self.score(w, pos, Vec2::ZERO, false);
        let mut best_move = dirs[0];
        let mut best_move_score = f32::NEG_INFINITY;
        for d in dirs {
            let s = self.score(w, pos, d, true);
            if s > best_move_score {
                best_move_score = s;
                best_move = d;
            }
            if s > best_score {
                best_score = s;
                best_dir = d;
            }
        }
        // Standing still is never the answer with a body on the rat. The
        // danger field scores "stay" without the runway and heading penalties,
        // so inside a dense pack every heading can look worse than not moving
        // and the rat freezes while the horde chews through it (measured: 11 s
        // motionless at HP 115 -> 65 with 100+ bodies alive, run seed
        // 13414622563986662246; `autoplay::anomaly::AnomalyKind::Stuck`). Take
        // the best *heading* instead: moving into the thinnest arc is the only
        // way out, and this only fires while a body is on the rat.
        if best_dir.len_sq() <= 0.01 && self.body_on_rat(w) {
            best_dir = best_move;
        }

        // Casual steering error: rotate the chosen direction a little.
        if self.params.noise > 0.0 && best_dir.len_sq() > 0.0 {
            let jitter = (self.rng.random::<f32>() - 0.5) * 2.0 * self.params.noise;
            best_dir = best_dir.rotate(jitter).normalized();
        }

        if best_dir.len_sq() > 0.25 {
            self.heading = best_dir;
        }
        let dash = self.wants_dash(w, &dirs) || self.wants_active(w);
        // Grazing throttle: full speed only to break away, a creep when the
        // pack is close, so the weapons keep killing *next to* the rat and the
        // gems they drop land inside the magnet radius.
        let throttle = if self.params.standoff && !dash {
            self.graze_throttle(w)
        } else {
            1.0
        };
        let input = BotInput {
            move_dir: best_dir * throttle,
            dash,
        };
        // Never hand the simulation a NaN or an over-long throttle vector.
        let input = BotInput {
            move_dir: sanitize(input.move_dir),
            dash: input.dash,
        };
        self.held = input;
        input
    }

    /// How fast to move right now: 1.0 to break away, less to graze.
    ///
    /// `gap` is the distance from the rat's skin to the nearest enemy's skin.
    /// Close enough to be in weapon range but not in contact is the sweet spot
    /// (kills happen there, and a gem dropped there is inside the 22 px
    /// magnet), so the bot creeps in that band and only sprints to escape.
    fn graze_throttle(&self, w: &World) -> f32 {
        let p = w.player.pos;
        let mut gap = f32::INFINITY;
        for t in &self.threats {
            if let Threat::Enemy { pos, radius, .. } = *t {
                gap = gap.min(pos.dist(p) - radius - R);
            }
            // Anything that can land *this* tick means sprint, not graze.
            if let Threat::Bullet { pos, vel, radius, .. }
            | Threat::Projectile { pos, vel, radius, .. } = *t
            {
                let rel = pos - p;
                let denom = vel.len_sq();
                if denom > 1.0 {
                    let t_hit = (rel.dot(vel) / denom).clamp(0.0, 0.25);
                    if (rel - vel * t_hit).len() < radius + R + 1.5 && t_hit < 0.2 {
                        return 1.0;
                    }
                }
            }
            if let Threat::Hazard {
                shape,
                warn_left,
                active_left,
                ..
            } = *t
            {
                if active_left > 0.0 && shape.contains(p, R) && warn_left < 0.25 {
                    return 1.0;
                }
            }
        }
        if gap < 14.0 {
            1.0
        } else if gap < 26.0 {
            0.8
        } else if gap < 46.0 {
            0.5
        } else {
            0.35
        }
    }

    /// Acknowledge every queued chest immediately during autoplay.
    pub fn claim_chests(w: &mut World) {
        if !w.finished() {
            while w.claim_chest().is_some() {}
        }
    }

    /// Pick one of the three level-up cards by the documented value heuristic.
    pub fn pick_upgrade(&mut self, w: &World, choices: &[Upgrade]) -> usize {
        let lo = &w.player.loadout;
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        let mut best = 0usize;
        let mut best_value = f32::NEG_INFINITY;
        for (i, up) in choices.iter().enumerate() {
            let v = upgrade_value(lo, up, hp_frac);
            // The human skills' read of the cards is imperfect; Expert never
            // takes the draw, so every skill stays deterministic per seed.
            let v = if self.params.pick_noise > 0.0 {
                v * (1.0
                    + (self.rng.random::<f32>() - 0.5) * 2.0 * self.params.pick_noise)
            } else {
                v
            };
            if v > best_value {
                best_value = v;
                best = i;
            }
        }
        best
    }

    /// Pick up to `KEEP_COUNT` entries to carry into the next level, by the
    /// same value heuristic (always at least one weapon).
    pub fn pick_keep(&mut self, w: &World, items: &[Keepable]) -> Vec<usize> {
        let lo = &w.player.loadout;
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        let mut scored: Vec<(f32, usize)> = items
            .iter()
            .enumerate()
            .map(|(i, k)| (keep_value(lo, k, hp_frac), i))
            .collect();
        // Stable: highest value first, then the manifest's own order.
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        let mut chosen: Vec<usize> = scored
            .iter()
            .take(crate::game::campaign::KEEP_COUNT)
            .map(|&(_, i)| i)
            .collect();
        // A pack with no weapon would fall back to the starting weapon, so the
        // bot always takes its best weapon instead.
        if !chosen.iter().any(|&i| items[i].is_weapon()) {
            if let Some((_, i)) = scored.iter().find(|&&(_, i)| items[i].is_weapon()) {
                if let Some(slot) = chosen.last_mut() {
                    *slot = *i;
                }
            }
        }
        chosen.sort_unstable();
        chosen
    }

    // ---- Decision internals ---------------------------------------------

    /// Is `pos` within what this skill may read?
    fn sees(&self, w: &World, p: Vec2, q: Vec2) -> bool {
        match self.params.vision {
            Vision::Full => q.dist_sq(p) <= SCAN_RADIUS * SCAN_RADIUS,
            Vision::Screen => {
                let half = w.viewport * 0.5;
                (q.x - p.x).abs() <= half.x + 10.0 && (q.y - p.y).abs() <= half.y + 10.0
            }
        }
    }

    /// Is a rat-sized circle free of walls, as far as this skill knows? A
    /// screen-only skill trusts the minimap: explored walls block, unexplored
    /// cells are assumed open (it discovers a wall by bumping into it).
    fn clear(&self, w: &World, q: Vec2, r: f32) -> bool {
        match self.params.vision {
            Vision::Full => w.level.tiles.circle_clear(q, r, SizeClass::Small),
            Vision::Screen => explored_clear(w, q, r),
        }
    }

    /// Round a hazard's warn timer to the skill's readability.
    fn quantise_warn(&self, warn_left: f32) -> f32 {
        let q = self.params.hazard_quantum;
        if q <= 0.0 || warn_left <= 0.0 {
            warn_left
        } else {
            (warn_left / q).round() * q
        }
    }

    /// Walkable runway along `dir`, sampled every 8 px (capped at `max`).
    fn runway(&self, w: &World, p: Vec2, dir: Vec2, max: f32) -> f32 {
        let mut travelled = 0.0;
        while travelled < max {
            travelled += 8.0;
            if !self.clear(w, p + dir * travelled, R) {
                return (travelled - 8.0).max(0.0);
            }
        }
        max
    }

    /// Distance from `p` to the nearest wall, capped at 120 px.
    fn edge_gap(&self, w: &World, p: Vec2) -> f32 {
        let mut d = 0.0;
        while d < 120.0 {
            if !self.clear(w, p + Vec2::from_angle(d * 0.3) * d, R) {
                return d;
            }
            d += 12.0;
        }
        120.0
    }

    /// Is the straight line between two points walkable, as far as the skill
    /// knows the map?
    fn path_clear(&self, w: &World, a: Vec2, b: Vec2) -> bool {
        let steps = ((b - a).len() / 8.0).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            if !self.clear(w, a + (b - a) * t, R) {
                return false;
            }
        }
        true
    }

    /// How many of the 8 compass neighbours of `q` (20 px out) are walkable.
    fn openness(&self, w: &World, q: Vec2) -> i32 {
        let mut n = 0;
        for d in unit_dirs().iter().step_by(2) {
            if self.clear(w, q + *d * 20.0, R) {
                n += 1;
            }
        }
        n
    }

    /// Snapshot every threat near the rat (and the nearby loot).
    fn scan(&mut self, w: &World) {
        self.threats.clear();
        self.gems.clear();
        self.pickups.clear();
        let p = w.player.pos;
        self.boss_pos = None;
        self.boss_kind = None;
        for e in w.ecs.iter_entities() {
            if let Some(en) = e.get::<Enemy>() {
                if !en.alive() || !self.sees(w, p, en.pos) {
                    continue;
                }
                self.threats.push(Threat::Enemy {
                    pos: en.pos,
                    vel: en.vel,
                    radius: en.radius,
                    damage: en.damage,
                    kind: en.kind,
                });
            } else if let Some(b) = e.get::<Bullet>() {
                if b.faction != Faction::Enemy || !self.sees(w, p, b.pos) {
                    continue;
                }
                self.threats.push(Threat::Bullet {
                    pos: b.pos,
                    vel: b.vel,
                    radius: b.radius,
                    damage: b.damage,
                });
            } else if let Some(boss) = e.get::<Boss>() {
                if boss.dead {
                    continue;
                }
                // The bearing to the boss matters even from across the map
                // (the arena is where the level ends), so it is recorded
                // before the neighbourhood filter.
                self.boss_pos = Some(boss.pos);
                self.boss_kind = Some(boss.kind);
                if !self.sees(w, p, boss.pos) {
                    continue;
                }
                self.threats.push(Threat::BossBody {
                    pos: boss.pos,
                    radius: boss.kind.body_radius(),
                    damage: boss.contact_damage.max(boss.kind.contact_damage()),
                });
                for h in &boss.hazards {
                    if h.is_expired() || !h.hit_player {
                        continue;
                    }
                    self.threats.push(Threat::Hazard {
                        shape: h.shape,
                        warn_left: self.quantise_warn((h.warn - h.elapsed).max(0.0)),
                        active_left: (h.warn + h.active - h.elapsed).max(0.0),
                        damage: h.damage,
                    });
                }
            } else if let Some(bp) = e.get::<BossProjectile>() {
                if !self.sees(w, p, bp.pos) {
                    continue;
                }
                self.threats.push(Threat::Projectile {
                    pos: bp.pos,
                    vel: bp.vel,
                    radius: bp.radius,
                    damage: bp.damage,
                });
            } else if let Some(g) = e.get::<Gem>() {
                if g.pos.dist_sq(p) <= 260.0 * 260.0 && self.sees(w, p, g.pos) {
                    self.gems.push((g.pos, g.value));
                }
            } else if let Some(pk) = e.get::<Pickup>() {
                if pk.pos.dist_sq(p) <= 200.0 * 200.0 && self.sees(w, p, pk.pos) {
                    self.pickups.push((pk.pos, pk.kind));
                }
            }
        }
        if self.params.trap_aware {
            for trap in &w.level.objects.sticky_traps {
                if matches!(
                    trap.state,
                    crate::game::art_traps::StickyTrapState::Idle
                ) && trap.pos.dist_sq(p) < 90.0 * 90.0
                    && self.sees(w, p, trap.pos)
                {
                    self.threats.push(Threat::StickyPad {
                        pos: trap.pos,
                        facing: trap.facing,
                    });
                }
            }
            for trap in &w.level.objects.mousetraps {
                let armed = matches!(
                    trap.state,
                    crate::game::art_traps::MousetrapState::Armed
                        | crate::game::art_traps::MousetrapState::Triggered
                        | crate::game::art_traps::MousetrapState::Snapping
                );
                if armed && trap.pos.dist_sq(p) < 70.0 * 70.0 && self.sees(w, p, trap.pos) {
                    self.threats.push(Threat::Mousetrap {
                        pos: trap.pos,
                        facing: trap.facing,
                        armed,
                    });
                }
            }
        }
    }

    /// Score moving along `dir` from `p`: a short forward lookahead that
    /// predicts where the rat and every threat would be at three sample
    /// times, so "walk away from a chaser" and "walk into a chaser" are
    /// different numbers instead of the same distance.
    fn score(&self, w: &World, p: Vec2, dir: Vec2, moving: bool) -> f32 {
        let speed = w.player.loadout.stats.move_speed;
        // Seconds ahead, and how much each sample counts.
        const TIMES: [f32; 3] = [0.12, 0.27, 0.48];
        const WEIGHTS: [f32; 3] = [0.5, 0.32, 0.18];
        let mut danger = 0.0f32;

        // Walls: standing inside one is fatal to the plan; a short runway is
        // a dead end, and a dead end with a horde behind it is where runs die.
        if !self.clear(w, p, R + 0.5) {
            danger += 100.0;
        }
        if moving {
            let runway = self.runway(w, p, dir, 140.0);
            danger += (1.0 - runway / 140.0).powi(2) * 9.0;
        }

        // Invulnerability makes contact free: dash-frames are the one moment a
        // rat can cross a pack, so the field must know.
        let invuln = (w.player.iframes / 0.6).clamp(0.0, 1.0);
        let contact_scale = 1.0 - 0.85 * invuln;

        for (i, &t) in TIMES.iter().enumerate() {
            let q = p + dir * (speed * t);
            let wt = WEIGHTS[i];
            if !self.clear(w, q, R) {
                danger += 80.0 * wt;
            }
            danger += (DIRS as f32 - self.openness(w, q) as f32) * 1.1 * wt;

            for threat in &self.threats {
                danger += wt * threat_cost(p, *threat, q, t, contact_scale);
            }
        }

        // Escape field: the short lookahead only sees contact range, but being
        // surrounded is decided 100-200 px out. When the rat is inside a pack
        // every nearby direction looks equally bad, so the way out is the
        // direction with the thinnest mass further along it.
        let end = p + dir * (speed * 0.48);
        let mut thin = 0.0f32;
        for (dist, weight) in [(85.0f32, 1.0f32), (165.0, 0.55), (250.0, 0.3)] {
            let probe = p + dir * dist;
            for t in &self.threats {
                if let Threat::Enemy { pos, radius, .. } = *t {
                    let d = pos.dist(probe) - radius;
                    if d < 65.0 {
                        thin += weight * (1.0 - (d.max(0.0) / 65.0)).powi(2);
                    }
                }
            }
        }
        danger += thin * 0.8;

        // Drifting into the arena border funnels a kite into a corner.
        let edge = self.edge_gap(w, end);
        if edge < 90.0 {
            danger += (1.0 - edge / 90.0) * 3.0;
        }

        let mut bonus = 0.0f32;
        // In a boss fight the boss is the objective: the level (and the run)
        // ends when it dies, so it outranks loot and the loop.
        let fighting = self.boss_pos.is_some();
        let momentum = if fighting {
            self.params.momentum * 0.4
        } else {
            self.params.momentum
        };
        // Kiting continuity: do not oscillate between two equally good dirs.
        if self.heading.len_sq() > 0.25 {
            bonus += momentum * dir.dot(self.heading);
        }
        // Loop / waypoint attraction.
        let to_goal = self.goal - p;
        if to_goal.len_sq() > 4.0 {
            let g = if fighting { 0.5 } else { 1.4 };
            bonus += g * dir.dot(to_goal.normalized());
        }
        // Loot attraction. XP is the whole progression, so gems pull hard —
        // but not into a pack that is sitting on them.
        //
        // **Tested and rejected (batch 4, concern "Expert's gem greed").** The
        // later-levels worker read Expert's 42 % campaign win as the bot's
        // greed (10 vs Skilled's 5) walking it into the pack, so the pull was
        // scaled by the crowd around the *rat* (`1/(1 + near*0.45)`): 200
        // paired Expert campaigns on bases 1234 and 7777 went **48 % → 43 %**
        // win, level-2 clear 77 → 71 %, so the greed was not the cause and the
        // change was reverted. The death notes say why: Expert's later-level
        // deaths are "surrounded in the open: brute contact, 20-40 bodies
        // within 60 px" — it is *already* in the pack when it dies, and a rat
        // surrounded at contact range has no good heading for greed to spoil.
        // See the batch4 report for the table.
        if self.params.greed > 0.0 {
            let mut best = 0.0f32;
            for &(gpos, value) in &self.gems {
                let d = gpos.dist(p);
                if d < 260.0 {
                    let w = (value as f32).clamp(1.0, 5.0) / 5.0;
                    let guard = 1.0 / (1.0 + self.enemies_near(gpos, 45.0) as f32 * 0.5);
                    let greed = if fighting {
                        self.params.greed * 0.2
                    } else {
                        self.params.greed
                    };
                    let v = greed * w * (1.0 - d / 260.0) * guard;
                    let toward = (gpos - p).normalized();
                    best = best.max(v * dir.dot(toward));
                }
            }
            bonus += best;
        }
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        for &(ppos, kind) in &self.pickups {
            let d = ppos.dist(p);
            if d > 190.0 {
                continue;
            }
            let want = match kind {
                PickupKind::Heal => {
                    if hp_frac < 0.8 {
                        9.0
                    } else {
                        1.0
                    }
                }
                PickupKind::Nuke => {
                    if w.enemy_count() > 30 {
                        7.0
                    } else {
                        2.0
                    }
                }
                PickupKind::Magnet => 4.0,
            };
            bonus += want * (1.0 - d / 190.0) * dir.dot((ppos - p).normalized());
        }
        // Boss: close enough for the weapons, not inside the body. The pull is
        // strongest from far away (the arena has to be reached) and fades to a
        // slow orbit once the rat is in the fight.
        if let Some(bpos) = self.boss_pos {
            let d = bpos.dist(p);
            let toward = (bpos - p).normalized();
            bonus += if d > 50.0 {
                let pull = 3.0 + (d / 200.0).min(4.0);
                pull * dir.dot(toward)
            } else if d > 24.0 {
                1.2 * dir.dot(toward)
            } else {
                -1.5
            };
        }

        -danger * self.params.safety + bonus
    }

    /// Is a live body inside `FOE_ON_RAT_PX` of the rat's skin right now?
    /// (`self.threats` is this decision's snapshot.)
    fn body_on_rat(&self, w: &World) -> bool {
        let p = w.player.pos;
        self.threats.iter().any(|t| match *t {
            Threat::Enemy { pos, radius, .. } => pos.dist(p) < radius + R + FOE_ON_RAT_PX,
            _ => false,
        })
    }

    /// Should the dash be spent this tick, and if so, in the best direction?
    ///
    /// The chosen escape direction is already `self.held.move_dir`'s role: the
    /// caller scores directions independently, so this only decides *whether*
    /// to dash; the movement picker has already chosen the safest heading.
    fn wants_dash(&mut self, w: &World, dirs: &[Vec2; DIRS]) -> bool {
        if w.player.dash_cd > 0.0 {
            return false;
        }
        if !self.params.smart_dash {
            // Casual: a panic dash now and then when badly hurt.
            return w.player.hp < w.player.max_hp() * 0.35
                && self.rng.random::<f32>() < 0.05;
        }
        let p = w.player.pos;
        let mut imminent = false;
        let mut close = 0u32;
        for t in &self.threats {
            match *t {
                Threat::Hazard {
                    shape,
                    warn_left,
                    active_left,
                    ..
                } => {
                    if active_left > 0.0
                        && shape.contains(p, R + 1.0)
                        && (warn_left <= 0.20 || warn_left > 0.0)
                    {
                        imminent = true;
                    }
                }
                Threat::Bullet { pos, vel, radius, .. }
                | Threat::Projectile { pos, vel, radius, .. } => {
                    let rel = pos - p;
                    let rv = vel;
                    let denom = rv.len_sq();
                    if denom > 1.0 {
                        let t_hit = (rel.dot(rv) / denom).clamp(0.0, 0.30);
                        let miss = (rel - rv * t_hit).len();
                        if t_hit < 0.22 && miss < radius + R + 2.0 {
                            imminent = true;
                        }
                    }
                }
                Threat::Enemy {
                    pos,
                    radius,
                    damage,
                    ..
                } => {
                    // Dash to break out before the ring closes: one body on
                    // the rat (Cat/Brute are the `damage >= 10` kinds), or a
                    // tight pack around it.
                    if damage >= 10.0 && pos.dist(p) < radius + R + 2.0 {
                        imminent = true;
                    }
                    // 14 px is ~0.4 s at a Cat's 38 px/s chase, the fastest
                    // melee body; the count and the `damage >= 10` contact
                    // test above are what make a Cat on the rat spend the dash.
                    if pos.dist(p) < radius + R + 14.0 {
                        close += 1;
                    }
                }
                _ => {}
            }
        }
        // A Cat already in chase range: spend the burst to open the gap.
        for t in &self.threats {
            if let Threat::Enemy {
                pos,
                vel,
                kind: EnemyKind::Cat,
                radius,
                ..
            } = *t
            {
                let to_rat = p - pos;
                if to_rat.len() < radius + R + 16.0
                    && vel.dot(to_rat) > 0.5 * EnemyKind::Cat.speed()
                {
                    return true;
                }
            }
        }

        // The cat's locked pounce ring: sidestep at the last moment so the
        // jaguar overshoots instead of landing on the rat.
        if self.params.boss_counters {
            if let Some(BossCounter::DodgeRing { center, radius }) = self.boss_counter(w) {
                let away = (p - center).normalized();
                if center.dist(p) < radius + R + 6.0 {
                    let _ = away;
                    imminent = true;
                }
            }
        }
        if close >= 3 {
            imminent = true;
        }
        // One body on top of the rat with the pack behind it is already too
        // late to walk out: spend the dash.
        if let Some(gap) = self.nearest_gap(w) {
            if gap < 9.0 {
                imminent = true;
            }
        }
        if !imminent {
            return false;
        }
        // Is there an escape direction that is actually safe? If every
        // direction is bad, dashing is still fine (i-frames), but prefer one
        // that clears the immediate footprint.
        let best = dirs
            .iter()
            .map(|d| self.score(w, p, *d, true) - d.dot(Vec2::ZERO) * 0.0)
            .fold(f32::NEG_INFINITY, f32::max);
        best > f32::NEG_INFINITY
    }

    /// Should the Space press be spent on a charged **active** weapon?
    ///
    /// The same press dashes, and `wants_dash` already covers the escapes, so
    /// this only asks the other half of the question: is there a reason to
    /// *thrash* right now? Two reasons:
    ///
    /// * **A crowd in reach.** Three or more bodies inside the longest charged
    ///   weapon's reach is what the area weapons are for; waiting for more only
    ///   wastes the charge.
    /// * **Cornered.** Under half health with a heavy body (`damage >= 9`) on
    ///   the rat, a charged panic button is worth spending even on one body,
    ///   because the stun/knockback is the escape.
    ///
    /// Pure read of the bot's own threat snapshot, so it draws no rng and
    /// cannot change a run except through the `dash` input it returns.
    fn wants_active(&self, w: &World) -> bool {
        // Keep the dash for escapes: the Space press dashes too, so while the
        // dash is ready it belongs to `wants_dash`, and every press made here is
        // free in dash terms (the dash is already charging).
        //
        // A bot that does **not** spend its dash deliberately (Casual: human
        // reaction, no `smart_dash`, a 5 % panic dash under 35 % HP) has nothing
        // to save it for, so it mashes Space like a human would — its press
        // spends the panic dash, and the i-frames that come with it are the
        // point. Without this, six of Casual's hundred campaigns picked an
        // active weapon it could never fire (measured: level-1 clear 47 % → 23 %,
        // the picks crowding out Armour and Fat Reserves).
        if w.player.dash_cd <= 0.0 && self.params.smart_dash {
            return false;
        }
        let p = w.player.pos;
        let mut reach = 0.0f32;
        for weapon in &w.player.loadout.weapons {
            if weapon.kind.is_active() && weapon.timer <= 0.0 {
                reach = reach.max(weapon.active_reach());
            }
        }
        if reach <= 0.0 {
            return false;
        }
        let mut in_reach = 0u32;
        let mut heavy_on_rat = false;
        for t in &self.threats {
            if let Threat::Enemy {
                pos,
                radius,
                damage,
                ..
            } = *t
            {
                if pos.dist(p) > reach + radius {
                    continue;
                }
                in_reach += 1;
                if damage >= 9.0 && pos.dist(p) < radius + R + 10.0 {
                    heavy_on_rat = true;
                }
            }
        }
        in_reach >= 3 || (heavy_on_rat && w.player.hp < w.player.max_hp() * 0.5)
    }

    /// Distance from the rat's skin to the nearest enemy's skin.
    fn nearest_gap(&self, w: &World) -> Option<f32> {
        let p = w.player.pos;
        self.threats
            .iter()
            .filter_map(|t| match *t {
                Threat::Enemy { pos, radius, .. } => Some(pos.dist(p) - radius - R),
                _ => None,
            })
            .fold(None, |acc: Option<f32>, g| {
                Some(acc.map(|a| a.min(g)).unwrap_or(g))
            })
    }

    /// Boss-specific counter: the one rule the generic hazard field cannot
    /// express is "wait for the lock, then leave the ring sideways".
    fn boss_counter(&self, w: &World) -> Option<BossCounter> {
        use crate::game::art_boss_cat::{BossCatState, CROUCH_LOCK_PROGRESS, POUNCE_LAND_RADIUS};
        use crate::game::ecs::systems::boss::BossState;
        for e in w.ecs.iter_entities() {
            let Some(b) = e.get::<Boss>() else {
                continue;
            };
            if b.dead {
                continue;
            }
            if let BossState::Cat(state) = b.state {
                if state == BossCatState::Crouch
                    && b.progress() >= CROUCH_LOCK_PROGRESS
                    && b.target.dist(w.player.pos) < POUNCE_LAND_RADIUS + R + 4.0
                {
                    return Some(BossCounter::DodgeRing {
                        center: b.target,
                        radius: POUNCE_LAND_RADIUS,
                    });
                }
            }
        }
        None
    }

    /// Re-roll the waypoint when it is reached or its timer expires.
    fn update_goal(&mut self, w: &World, dt: f32) {
        let p = w.player.pos;
        self.goal_timer -= dt;
        let reached = self.goal.dist_sq(p) < 40.0 * 40.0;
        if self.goal_timer > 0.0 && !reached {
            return;
        }
        self.goal_timer = 2.4 + self.rng.random::<f32>() * 0.8;

        // In a boss fight, orbit the arena around the boss instead of roaming.
        if let Some(bpos) = self.boss_pos {
            let ang = (p - bpos).angle() + self.orbit_sign * 1.1;
            // Close enough for the short-range weapons (Tail Whip, Squeak
            // Nova, acid pools) to be on the boss, out to the body's reach.
            let ring = 32.0;
            self.goal =
                (bpos + Vec2::from_angle(ang) * ring).clamp_box(Vec2::splat(20.0), w.level.arena - Vec2::splat(20.0));
            return;
        }

        // Otherwise run a long leg toward open ground away from the crowd.
        // Waypoints are far enough that the rat keeps a sustained heading
        // (kiting laps), not a 110 px circle that walks straight back into
        // the horde.
        let crowd = self.crowd_centroid(p);
        let radius = self.goal_radius;
        let mut best = p;
        let mut best_score = f32::NEG_INFINITY;
        for i in 0..16 {
            let a = self.wander + i as f32 * std::f32::consts::TAU / 16.0;
            let q = (p + Vec2::from_angle(a) * radius)
                .clamp_box(Vec2::splat(20.0), w.level.arena - Vec2::splat(20.0));
            if !self.path_clear(w, p, q) {
                continue;
            }
            let mut score = self.openness(w, q) as f32 * 0.9;
            // Head away from the crowd centroid.
            if let Some(c) = crowd {
                let away = (q - c).normalized();
                score += 2.5 * away.dot((q - p).normalized());
            }
            // Keep the loop's own heading: a leg that continues the last one
            // is a lap; a leg that reverses is a trap.
            if self.heading.len_sq() > 0.25 {
                score += 2.0 * self.heading.dot((q - p).normalized());
            }
            // Prefer ground away from the nearest enemies.
            let mut nearest = f32::INFINITY;
            for t in &self.threats {
                if let Threat::Enemy { pos, .. } = *t {
                    nearest = nearest.min(pos.dist(q));
                }
            }
            score += (nearest / 120.0).min(1.4);
            if score > best_score {
                best_score = score;
                best = q;
            }
        }
        self.goal = best;
        self.goal_radius = if crowd.is_some() {
            // Kiting a pack: a long enough leg that the rat does not turn
            // back into the horde it is outrunning.
            200.0
        } else {
            160.0
        };
        self.wander = (self.wander + 0.55 * self.orbit_sign)
            .rem_euclid(std::f32::consts::TAU);
    }

    /// How many enemies sit within `reach` of `point`.
    fn enemies_near(&self, point: Vec2, reach: f32) -> usize {
        self.threats
            .iter()
            .filter(|t| matches!(t, Threat::Enemy { pos, .. } if pos.dist(point) < reach))
            .count()
    }

    /// The mean position of the enemies closing in, if there are enough of
    /// them to be called a crowd.
    fn crowd_centroid(&self, p: Vec2) -> Option<Vec2> {
        let mut sum = Vec2::ZERO;
        let mut n = 0u32;
        for t in &self.threats {
            if let Threat::Enemy { pos, .. } = *t {
                if pos.dist(p) < 150.0 {
                    sum += pos;
                    n += 1;
                }
            }
        }
        if n >= 4 {
            Some(sum * (1.0 / n as f32))
        } else {
            None
        }
    }
}

/// What the boss counter wants the rat to do right now.
enum BossCounter {
    /// Leave the marked ring before it fires.
    DodgeRing { center: Vec2, radius: f32 },
}

fn unit_dirs() -> [Vec2; DIRS] {
    let mut out = [Vec2::ZERO; DIRS];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = Vec2::from_angle(i as f32 * std::f32::consts::TAU / DIRS as f32);
    }
    out
}

/// Closest-approach danger of one moving projectile to a candidate point.
fn bullet_danger(pos: Vec2, vel: Vec2, radius: f32, damage: f32, q: Vec2, horizon: f32) -> f32 {
    let rel = pos - q;
    let denom = vel.len_sq();
    let reach = radius + R + 2.5;
    if denom <= 1.0 {
        // Static hazard (orbit/spore style): plain distance.
        let d = rel.len();
        return if d < reach {
            (damage / 6.0).clamp(1.0, 4.0) * 10.0 * (1.0 - d / reach)
        } else {
            0.0
        };
    }
    let t = (rel.dot(vel) / denom).clamp(0.0, horizon + 0.1);
    let closest = rel - vel * t;
    let miss = closest.len();
    if t > horizon + 0.1 || miss > reach {
        0.0
    } else {
        let w = 1.0 - miss / reach;
        (damage / 6.0).clamp(1.0, 4.0) * 10.0 * w * w
    }
}

/// Is `q` on the glue pad of the board at `pos`/`facing`? (Art's oriented
/// rectangle `STICKY_TRAP_PAD_HALF`.)
fn on_pad(pos: Vec2, facing: Vec2, q: Vec2) -> bool {
    let d = q - pos;
    let f = if facing.len_sq() > 0.0 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    let pad = crate::game::art_traps::STICKY_TRAP_PAD_HALF;
    d.dot(f).abs() <= pad.x + R && d.dot(f.perp()).abs() <= pad.y + R
}

/// Danger of one threat to a candidate position `q` at `t` seconds from now.
fn threat_cost(_p: Vec2, threat: Threat, q: Vec2, t: f32, contact_scale: f32) -> f32 {
    match threat {
        Threat::Enemy {
            pos,
            vel,
            radius,
            damage,
            kind,
        } => {
            // Each kind chases at `EnemyKind::speed` px/s (Skitterer 30, Bat
            // 42, Cat 38, Spitter 22, Brute 16) and the velocity sample
            // already carries that: `Enemy::steer` smooths toward
            // `kind.speed() * speed_mult`. Predicting with it is measurably
            // better than a kind-speed chase model (the A/B lost 60 s of
            // median survival), so the kind only decides the weight below.
            let _ = kind;
            let pred = pos + vel * t;
            // Faster kinds are read a moment earlier: 0.9 x their own speed
            // (34 px for the 30-38 px/s crawlers and cats, 38 px for the
            // 42 px/s bat).
            let reach = 34.0f32.max(kind.speed() * 0.9);
            let near = pred.dist(q) - radius - R;
            if near > reach {
                0.0
            } else {
                let w = (1.0 - (near.max(-reach) / reach)).clamp(0.0, 1.0);
                w * w * (damage / 6.0).clamp(0.6, 3.0) * 6.0 * contact_scale
            }
        }
        Threat::Bullet {
            pos,
            vel,
            radius,
            damage,
        }
        | Threat::Projectile {
            pos,
            vel,
            radius,
            damage,
        } => {
            let pred = pos + vel * t;
            let reach = radius + R + 3.0;
            let d = pred.dist(q);
            if d > reach {
                0.0
            } else {
                (damage / 6.0).clamp(1.0, 4.0) * 12.0 * (1.0 - d / reach)
            }
        }
        Threat::BossBody {
            pos,
            radius,
            damage,
        } => {
            let near = pos.dist(q) - radius - R;
            if near > 26.0 {
                0.0
            } else {
                let w = (1.0 - (near.max(-26.0) / 26.0)).clamp(0.0, 1.0);
                w * (damage / 10.0).clamp(1.0, 3.0) * 9.0 * contact_scale
            }
        }
        Threat::Hazard {
            shape,
            warn_left,
            active_left,
            damage,
        } => {
            if active_left <= 0.0 || !shape.contains(q, R) {
                0.0
            } else {
                // Only worth avoiding if it fires inside the lookahead.
                if warn_left > t + 0.12 {
                    0.0
                } else {
                    (damage / 10.0).clamp(0.8, 4.0) * 16.0
                }
            }
        }
        Threat::StickyPad { pos, facing } => {
            if on_pad(pos, facing, q) {
                3.0
            } else {
                0.0
            }
        }
        Threat::Mousetrap { pos, facing, armed } => {
            let trigger = pos + facing * 3.5;
            if armed && trigger.dist(q) < 4.5 + R {
                4.0
            } else {
                0.0
            }
        }
    }
}

/// A screen-only skill's map knowledge: explored walls block; unexplored cells
/// are assumed open (the bot finds them the way a player does).
fn explored_clear(w: &World, q: Vec2, r: f32) -> bool {
    let (x0, y0) = cell_of(q - Vec2::splat(r));
    let (x1, y1) = cell_of(q + Vec2::splat(r));
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            if w.explored(cx, cy) && !w.level.tiles.get(cx, cy).passable(SizeClass::Small) {
                return false;
            }
        }
    }
    true
}

/// Clamp the throttle vector into the simulation's contract: finite, and no
/// longer than a unit.
fn sanitize(v: Vec2) -> Vec2 {
    if !v.x.is_finite() || !v.y.is_finite() {
        return Vec2::ZERO;
    }
    v.clamp_len(1.0)
}

// ---- Upgrade valuation ---------------------------------------------------

/// A weapon's raw value in the upgrade heuristic (AoE and coverage above
/// single-target).
fn weapon_tier(k: WeaponKind) -> f32 {
    // b4-upgrades retuned the weapons for the measured damage band (Tail Whip
    // 10 dmg, Squeak Nova 6.5, Spore Orbit 9 with a wider spore, Gnaw a second
    // dart from level 4, Acid Spit's pool at 3.5x). The tiers follow: Spore
    // Orbit is no longer the weakest pick, Squeak Nova no longer the best, and
    // Gnaw's late double dart makes keeping it a real single-target option.
    match k {
        WeaponKind::Gnaw => 6.5,
        WeaponKind::CheeseSpray => 7.0,
        WeaponKind::SqueakNova => 8.0,
        WeaponKind::SporeOrbit => 8.0,
        WeaponKind::TailWhip => 9.0,
        WeaponKind::AcidSpit => 8.0,
        // The active weapons are a panic button (area + crowd control) rather
        // than a damage stream, so they rank above the passives early and lose
        // to a maxed weapon later: `upgrade_value` adds +6 per level pick.
        WeaponKind::TailThrash => 9.5,
        WeaponKind::SonicSqueak => 9.0,
    }
}

/// The value heuristic behind level-up picks, ordered for **early survival**:
///
/// * **Armour and max HP first.** 98 % of all damage is enemy contact and a
///   hit is capped at one enemy's damage per 0.6 s, so a flat armour cut is
///   worth more than any damage multiplier in the first two minutes.
///   Whiskers (magnet) is close behind: it is the XP engine, and the run is
///   decided by whether the snowball starts before the horde closes.
/// * a **third weapon** is worth a lot (coverage); levelling an owned weapon
///   is the strongest single offensive pick; a fourth+ weapon is filler.
/// * damage / fire rate overtake the defensive picks only once the build has
///   a base (late game), and move speed is a modest utility pick.
pub fn upgrade_value(lo: &Loadout, up: &Upgrade, hp_frac: f32) -> f32 {
    let progression: u32 = lo
        .weapons
        .iter()
        .map(|w| w.level.saturating_sub(1) as u32)
        .sum::<u32>()
        + lo.passives.iter().map(|(_, n)| *n).sum::<u32>()
        + lo.weapons.len().saturating_sub(1) as u32;
    let early = progression < 8;
    match up {
        Upgrade::NewWeapon(k) => {
            let base = weapon_tier(*k);
            if lo.weapons.len() < 3 {
                base + 5.0
            } else if lo.weapons.len() < 4 {
                base - 1.5
            } else {
                base - 3.5
            }
        }
        Upgrade::LevelWeapon(i) => match lo.weapons.get(*i) {
            Some(w) => {
                let base = weapon_tier(w.kind) + 6.0 - w.level as f32 * 0.4;
                if early { base + 1.0 } else { base }
            }
            None => 0.0,
        },
        Upgrade::Passive(p) => match p {
            PassiveKind::Armor => match lo.stacks(PassiveKind::Armor) {
                0 => 10.0,
                1 | 2 => 8.5,
                3 | 4 => 6.0,
                _ => 4.0,
            },
            PassiveKind::MaxHp => {
                if lo.stacks(PassiveKind::MaxHp) < 2 || hp_frac < 0.7 {
                    8.5
                } else {
                    4.5
                }
            }
            PassiveKind::Magnet => match lo.stacks(PassiveKind::Magnet) {
                0 => 10.0,
                1 => 8.0,
                2 => 6.0,
                _ => 3.0,
            },
            PassiveKind::Regen => {
                if lo.stacks(PassiveKind::Regen) < 2 { 6.0 } else { 3.5 }
            }
            PassiveKind::Damage => {
                if early { 6.0 } else { 7.5 }
            }
            PassiveKind::FireRate => {
                if early { 5.5 } else { 7.0 }
            }
            PassiveKind::MoveSpeed => 4.5,
        },
    }
}

/// The keep-three value of one manifest entry (same scale as
/// [`upgrade_value`]).
fn keep_value(lo: &Loadout, k: &Keepable, hp_frac: f32) -> f32 {
    match k.kind {
        crate::game::campaign::KeepKind::Weapon(kind) => {
            weapon_tier(kind) * (1.0 + 0.18 * k.level as f32)
        }
        crate::game::campaign::KeepKind::Passive(passive) => {
            // A kept passive carries **every** stack (`Keepable.level`), so
            // its keep value has to grow faster than one more stack would:
            // 0.12 per stack made a 5-stack Armor/Fat Reserves card clearly
            // worse than any level-5 weapon, which is not the choice the keep
            // screen is supposed to pose. 0.25 per stack keeps a maxed passive
            // comparable to a kept level-4 weapon.
            let one = Upgrade::Passive(passive);
            upgrade_value(lo, &one, hp_frac) * (1.0 + 0.25 * k.level as f32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;

    fn seeded_bot(skill: Skill, seed: u64) -> Bot {
        let mut bot = Bot::new(skill, seed);
        bot.on_new_level(0);
        bot
    }

    #[test]
    fn decision_is_finite_and_bounded_on_an_empty_floor() {
        for skill in [Skill::Expert, Skill::Skilled, Skill::Casual] {
            let mut w = World::new(Theme::Sewer, 7);
            // Strip the map so the test measures the policy, not the layout.
            w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
            let mut bot = seeded_bot(skill, 1);
            for _ in 0..240 {
                let input = bot.decide(&w, 1.0 / 60.0);
                assert!(
                    input.move_dir.x.is_finite() && input.move_dir.y.is_finite(),
                    "{skill:?} produced a non-finite direction"
                );
                assert!(
                    input.move_dir.len() <= 1.0 + 1e-4,
                    "{skill:?} over-throttled"
                );
            }
        }
    }

    /// The bot spends a charged active weapon on a crowd — and keeps its dash
    /// for an escape, because the Space press is shared.
    #[test]
    fn the_bot_fires_a_charged_active_on_a_crowd_without_wasting_its_dash() {
        use crate::game::weapon::{Weapon, WeaponKind};
        use crate::game::enemy::EnemyKind;

        let crowd = |active: bool, enemies: usize| -> World {
            let mut w = World::new(Theme::Sewer, 31);
            w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
            w.clear_enemies();
            let p = w.player.pos;
            // Skitterers 30 px out: in the thrash's reach, but not `close`
            // enough (19.5 px) or heavy enough to make `wants_dash` fire.
            for i in 0..enemies {
                let a = i as f32 * 1.4;
                w.spawn_enemy(EnemyKind::Skitterer, p + Vec2::from_angle(a) * 30.0, 1.0);
            }
            let mut weapon = Weapon::new(WeaponKind::TailThrash);
            weapon.timer = if active { 0.0 } else { 5.0 };
            w.player.loadout.weapons.push(weapon);
            w
        };

        // Charged, four bodies in reach, dash charging: press.
        let mut w = crowd(true, 4);
        w.player.dash_cd = 1.0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        assert!(
            bot.decide(&w, 1.0 / 60.0).dash,
            "the bot should spend a charged active on a crowd"
        );

        // Charged but the dash is ready: the press belongs to the escape.
        let mut w = crowd(true, 4);
        w.player.dash_cd = 0.0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        assert!(
            !bot.decide(&w, 1.0 / 60.0).dash,
            "the bot must not burn a ready dash on a calm crowd"
        );

        // Charging, or only one body: nothing to press.
        let mut w = crowd(false, 4);
        w.player.dash_cd = 1.0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        assert!(!bot.decide(&w, 1.0 / 60.0).dash, "uncharged weapon fired");

        let mut w = crowd(true, 1);
        w.player.dash_cd = 1.0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        assert!(
            !bot.decide(&w, 1.0 / 60.0).dash,
            "one body is not worth a charge"
        );

        // Cornered: hurt, one heavy body on the rat → spend the panic button.
        let mut w = crowd(true, 0);
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Cat, p + Vec2::new(6.0, 0.0), 1.0);
        w.player.dash_cd = 1.0;
        w.player.hp = w.player.max_hp() * 0.3;
        let mut bot = seeded_bot(Skill::Expert, 3);
        assert!(
            bot.decide(&w, 1.0 / 60.0).dash,
            "a cornered bot should spend a charged active"
        );
    }

    #[test]
    fn every_skill_is_deterministic_per_seed() {
        let w = World::new(Theme::Kitchen, 11);
        for skill in [Skill::Expert, Skill::Skilled, Skill::Casual] {
            let mut a = seeded_bot(skill, 5);
            let mut b = seeded_bot(skill, 5);
            for tick in 0..120 {
                assert_eq!(
                    a.decide(&w, 1.0 / 60.0),
                    b.decide(&w, 1.0 / 60.0),
                    "{skill:?} diverged on tick {tick}"
                );
            }
        }
    }

    /// The ladder is a property of the parameter table, not a happy accident of
    /// the map: each rung is strictly slower, noisier, blinder and
    /// shorter-sighted than the one above it.
    #[test]
    fn skills_form_an_ordered_ladder() {
        let expert = Params::for_skill(Skill::Expert);
        let skilled = Params::for_skill(Skill::Skilled);
        let casual = Params::for_skill(Skill::Casual);
        assert!(Skill::Expert.tier() > Skill::Skilled.tier());
        assert!(Skill::Skilled.tier() > Skill::Casual.tier());
        assert!(expert.reaction < skilled.reaction);
        assert!(skilled.reaction < casual.reaction);
        assert!(expert.noise <= skilled.noise);
        assert!(skilled.noise < casual.noise);
        assert!(expert.safety > skilled.safety);
        assert!(skilled.safety > casual.safety);
        assert!(expert.horizon > skilled.horizon);
        assert!(skilled.horizon > casual.horizon);
        assert_eq!(expert.vision, Vision::Full);
        assert_eq!(skilled.vision, Vision::Screen);
        assert_eq!(casual.vision, Vision::Screen);
        assert!(expert.greed > skilled.greed);
        assert!(skilled.greed > casual.greed);
        assert!(expert.boss_counters && !casual.boss_counters);
        assert!(expert.smart_dash && !casual.smart_dash);
        assert!(expert.pick_noise < casual.pick_noise);
        assert_eq!(Skill::parse("skilled"), Some(Skill::Skilled));
    }

    /// A screen-only skill cannot see a body that is off screen, even when the
    /// expert can (the batch viewport is 160x80 px).
    #[test]
    fn vision_limits_what_the_lower_skills_scan() {
        let mut w = World::new(Theme::Sewer, 3);
        w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
        w.player.pos = w.level.arena * 0.5;
        w.spawn_enemy(EnemyKind::Cat, w.player.pos + Vec2::new(150.0, 0.0), 1.0);

        let mut expert = seeded_bot(Skill::Expert, 1);
        expert.scan(&w);
        assert_eq!(expert.threats.len(), 1, "expert sees the whole simulation");

        let mut skilled = seeded_bot(Skill::Skilled, 1);
        skilled.scan(&w);
        assert!(
            skilled.threats.is_empty(),
            "skilled only sees on screen (viewport is 160x80)"
        );
    }

    /// Early survival first: armour, max HP and the XP magnet must outrank raw
    /// damage on the first cards.
    #[test]
    fn early_picks_favour_survival() {
        let lo = Loadout::starting();
        let armor = upgrade_value(&lo, &Upgrade::Passive(PassiveKind::Armor), 1.0);
        let hp = upgrade_value(&lo, &Upgrade::Passive(PassiveKind::MaxHp), 1.0);
        let damage = upgrade_value(&lo, &Upgrade::Passive(PassiveKind::Damage), 1.0);
        let magnet = upgrade_value(&lo, &Upgrade::Passive(PassiveKind::Magnet), 1.0);
        assert!(armor > damage, "armour {armor} must beat damage {damage}");
        assert!(hp > damage, "max HP {hp} must beat damage {damage}");
        assert!(magnet > damage, "magnet {magnet} must beat damage {damage}");
    }
}
