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
//! | horizon | 0.48 s | 0.48 s | 0.19 s |
//! | crowd read (escape field) | 0.8 | 0.8 | 0.38 |
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
use crate::game::ecs::systems::boss::{Boss, BossProjectile, BossState, HazardShape};
use crate::game::ecs::systems::miniboss::{Miniboss, MinibossShape, MinibossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::PickupKind;
use crate::game::loadout::{ChestReward, Loadout, PassiveKind, Upgrade};
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

/// How far ahead a charged Super Dash is worth holding for, in px (the full
/// charge travels about twice the ordinary dash's distance).
const SUPER_DASH_LANE_PX: f32 = 96.0;
/// Half-width of that lane, in px: the dash's swept radius plus a body's own
/// radius has to overlap it for the body to be a reason to charge.
const SUPER_DASH_LANE_HALF_W: f32 = 10.0;

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
    /// How hard to charge the dash, `0.0` (a tap: today's dash) to `1.0`.
    /// Only non-zero when the bot owns Super Dash *and* the lane ahead is
    /// worth holding for (see `Bot::charge_for_lane`); it is fed through
    /// `World::update_with_charge`, so the plain `update` path is unchanged.
    pub charge: f32,
}

impl BotInput {
    pub const IDLE: BotInput = BotInput {
        move_dir: Vec2::ZERO,
        dash: false,
        charge: 0.0,
    };
}

/// The lookahead every decision used before the horizon was a real
/// parameter: `score`'s last sample, in seconds. Expert and Skilled keep it,
/// so wiring the parameter changed nothing for them (`x * (0.48 / 0.48)` is
/// exact).
const HORIZON_FULL: f32 = 0.48;
/// The escape field's weight, likewise the value every skill used.
const CROWD_FULL: f32 = 0.8;
/// Casual's lookahead: a little over a quarter of a second, about 15 px of
/// travel at the base speed. Tuned with the batch (see the balance log): the
/// table's old 0.14 s label left it blind to anything past contact range.
const CASUAL_HORIZON: f32 = 0.19;
/// Casual's read of the crowd further out: it notices the thin side of a pack
/// less than a practised player does.
const CASUAL_CROWD: f32 = 0.38;

/// The skill's reaction model.
#[derive(Clone, Copy, Debug)]
struct Params {
    /// Seconds between decisions (`0` = every tick).
    reaction: f32,
    /// Angular noise added to the chosen direction (radians).
    noise: f32,
    /// Multiplier on the danger field.
    safety: f32,
    /// How far ahead `score` looks, in seconds: the last of its three
    /// samples (the other two keep their ratio, see [`HORIZON_FULL`]). Expert
    /// and Skilled read the full 0.48 s; Casual's is short, which is the way a
    /// new player is worse: it sees what is about to touch it and little of
    /// what it is walking into, so it leaves a warned shape late.
    horizon: f32,
    /// Sidestep the cat's locked pounce ring at the last moment.
    boss_counters: bool,
    /// Spend the dash out of telegraphed hazards.
    smart_dash: bool,
    /// Treat glue pads and armed mousetraps as obstacles.
    trap_aware: bool,
    /// How much XP-gem greed competes with safety.
    greed: f32,
    /// Weight of the soft crowd gradient: the escape field in `score` (mass
    /// 85-250 px along a heading, which decides the way out of a pack).
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
                horizon: HORIZON_FULL,
                boss_counters: true,
                smart_dash: true,
                trap_aware: true,
                greed: 10.0,
                crowd: CROWD_FULL,
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
                horizon: HORIZON_FULL,
                boss_counters: true,
                smart_dash: true,
                trap_aware: true,
                greed: 5.0,
                crowd: CROWD_FULL,
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
                horizon: CASUAL_HORIZON,
                boss_counters: false,
                smart_dash: false,
                trap_aware: false,
                greed: 1.0,
                crowd: CASUAL_CROWD,
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
        /// Growth Serum (batch 13): while the rat is giant this body is one it
        /// **crushes** rather than one it must avoid — set at scan time from
        /// `powerups::giant_crushes` and the live buff, so `threat_cost` only
        /// has to read a flag. Never true when the rat is not giant.
        stomp_free: bool,
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
    /// A named miniboss's signature telegraph / active shape.
    MinibossHazard {
        shape: MinibossShape,
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
    /// A **live** glue board (a spent one is plain floor). Wave 3: a board is a
    /// crowd tool, so the threat carries the load: a board the rat steps on
    /// holds it, but a leg that drags the chase over one is an asset.
    StickyPad {
        pos: Vec2,
        facing: Vec2,
        /// Glue units currently held (0 on a fresh board).
        load: u8,
        /// Beacons of the horde already pinned to it.
        stuck: u8,
    },
    /// A steam grate: a 1 s warn, then a 0.7 s one-tile plume for 8.
    Steam {
        pos: Vec2,
        warn_left: f32,
        active_left: f32,
    },
    /// An ink pad: no damage, but the slow makes every escape worse, so a dry
    /// route of equal length wins.
    Ink {
        pos: Vec2,
        half: Vec2,
    },
    /// A set (or mid-snap) snap trap, or a sprung one still holding a bait.
    Mousetrap {
        pos: Vec2,
        facing: Vec2,
        /// The bar is still up and the plate untouched: stepping on the plate
        /// (or a horde body doing it) starts the snap.
        armed: bool,
        /// The plate has been touched and the bar has not landed yet
        /// (`Triggered`, or `Snapping` before the hit window closes): the kill
        /// rect bites within ~0.26 s, whoever set it off.
        committed: bool,
        /// Hair trigger: no reaction window at all, never entered on purpose.
        hair: bool,
        /// A good bait still sits on the plate.
        bait_good: bool,
    },
    /// A wave-3 floor hazard: a Bell Flea's landing cross or a dead Wick
    /// Moth's burst are circles (`dir` is `ZERO`); a Turnkey's crossbar is the
    /// line `pos ± dir * radius`.
    WaveHazard {
        pos: Vec2,
        radius: f32,
        dir: Vec2,
        warn_left: f32,
        active_left: f32,
        damage: f32,
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
    /// Seconds before the rat may set off to another mouse-hole shop (a visit
    /// should not turn into a loop between two holes).
    shop_cd: f32,
    /// The current leg is a walk to a mouse-hole shop (the goal pull is
    /// stronger and the gem pull weaker, or greed keeps the rat off the shelf).
    shop_leg: bool,
    /// Seconds spent on shop legs since the last visit: a hole the straight
    /// line cannot reach is given up on instead of being rubbed against.
    shop_chase: f32,
    /// The threshold the shop leg is walking to.
    shop_face: Vec2,
    /// The route there: a flow field toward `shop_face`, built the first time
    /// a floor's shop is worth a walk (the horde's own field routes to the
    /// rat, so the bot keeps one of its own). `None` until then.
    shop_flow: Option<Box<crate::game::nav::FlowField>>,
    /// Boss seen this level, for the counter rules.
    boss_pos: Option<Vec2>,
    /// Living named miniboss (wave 3), for the fight objective.
    miniboss_pos: Option<Vec2>,
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
            shop_cd: 0.0,
            shop_leg: false,
            shop_chase: 0.0,
            shop_face: Vec2::ZERO,
            shop_flow: None,
            boss_pos: None,
            miniboss_pos: None,
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
        // A new floor has new holes: the last floor's visit does not count.
        self.shop_cd = 0.0;
        self.shop_leg = false;
        self.shop_chase = 0.0;
        // The field is built for one floor's tiles.
        self.shop_flow = None;
        self.boss_pos = None;
        self.miniboss_pos = None;
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
            let charge = if dash {
                self.charge_for_lane(w, self.held.move_dir)
            } else {
                0.0
            };
            let input = BotInput {
                move_dir: self.held.move_dir,
                dash,
                charge,
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
        // Escape first (that is what the dash is for); otherwise, with the dash
        // charged, take a good bait off an ordinary trap by dashing *through*
        // its plate — the i-frames carry the rat over and out of the bar.
        let escape = self.wants_dash(w, &dirs) || self.wants_active(w);
        let (best_dir, dash) = if escape {
            (best_dir, true)
        } else if let Some(dir) = self.bait_dash(w) {
            (dir, true)
        } else {
            (best_dir, false)
        };
        // Grazing throttle: full speed only to break away, a creep when the
        // pack is close, so the weapons keep killing *next to* the rat and the
        // gems they drop land inside the magnet radius.
        let throttle = if self.params.standoff && !dash {
            self.graze_throttle(w)
        } else {
            1.0
        };
        // A charged Super Dash is spent only when the lane is worth it (see
        // `charge_for_lane`); every other dash is the ordinary tap.
        let charge = if dash {
            self.charge_for_lane(w, best_dir)
        } else {
            0.0
        };
        let input = BotInput {
            move_dir: best_dir * throttle,
            dash,
            charge,
        };
        // Never hand the simulation a NaN or an over-long throttle vector.
        let input = BotInput {
            move_dir: sanitize(input.move_dir),
            dash: input.dash,
            charge: input.charge,
        };
        self.held = input;
        input
    }

    /// How hard to charge a Super Dash down `dir`: full when the lane ahead is a
    /// corridor of bodies (three or more) or the boss's body sits on it, and no
    /// charge otherwise (so the dash is today's ordinary tap).
    ///
    /// A charged dash is up to twice as long, wider, up to 2.5x the damage, and
    /// immune for the whole travel — but it costs the whole hold, so it is only
    /// worth it when the route is a line of bodies rather than an escape.
    fn charge_for_lane(&self, w: &World, dir: Vec2) -> f32 {
        if dir.len_sq() <= 0.25 || !w.player.loadout.has(WeaponKind::SuperDash) {
            return 0.0;
        }
        let dir = dir.normalized();
        let p = w.player.pos;
        let mut bodies = 0u32;
        let mut on_boss = false;
        for t in &self.threats {
            let (pos, radius, is_boss) = match *t {
                Threat::Enemy { pos, radius, .. } => (pos, radius, false),
                Threat::BossBody { pos, radius, .. } => (pos, radius, true),
                _ => continue,
            };
            let rel = pos - p;
            let along = rel.dot(dir);
            // Inside the super dash's reach, ahead of the rat, and close enough
            // to the lane that the swept radius would touch it.
            if along > 0.0
                && along < SUPER_DASH_LANE_PX + radius
                && (rel - dir * along).len() < radius + SUPER_DASH_LANE_HALF_W
            {
                bodies += 1;
                if is_boss {
                    on_boss = true;
                }
            }
        }
        if on_boss || bodies >= 3 {
            1.0
        } else {
            0.0
        }
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
            // The bot takes every chest at once, and applies the fallback
            // chest's heal exactly like `App::resume_or_open_reward` does (a
            // treatment run should not leave HP on the table).
            while let Some(reward) = w.claim_chest() {
                if let ChestReward::Gold(_) = reward {
                    let heal = crate::game::loadout::CHEST_HEAL_FRAC * w.player.max_hp();
                    w.player.heal(heal);
                }
            }
        }
    }

    /// Pick one of the three level-up cards by the documented value heuristic.
    pub fn pick_upgrade(&mut self, w: &World, choices: &[Upgrade]) -> usize {
        // Expert never takes the card-read draw: the noiseless valuation is the
        // shared one (`best_upgrade`), so the auto-pick toggle and the bot take
        // the same card from the same state.
        if self.params.pick_noise <= 0.0 {
            return best_upgrade(w, choices);
        }
        let lo = &w.player.loadout;
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        let mut best = 0usize;
        let mut best_value = f32::NEG_INFINITY;
        for (i, up) in choices.iter().enumerate() {
            let v = upgrade_value(lo, up, hp_frac);
            // The human skills' read of the cards is imperfect; Expert never
            // takes the draw, so every skill stays deterministic per seed.
            let v = v * (1.0 + (self.rng.random::<f32>() - 0.5) * 2.0 * self.params.pick_noise);
            if v > best_value {
                best_value = v;
                best = i;
            }
        }
        best
    }

    /// Visit the open mouse-hole shop: buy what is worth buying, then leave.
    ///
    /// The bot never stalls on the shelf — every branch ends in
    /// [`World::leave_shop`] — and it buys in a fixed order, so a seeded run
    /// stays deterministic. What it wants, in order:
    ///
    /// 1. a **Stitch** when it is under two thirds health (the heal that gets it
    ///    through the next fight);
    /// 2. the first affordable splice on the shelf while a socket is free,
    ///    skipping a max-HP bargain when the rat is hurt and skipping a
    ///    Blandboard Belly style healing cut when it has no healing to cut.
    ///
    /// Returns the number of purchases (tests read it).
    pub fn shop_visit(&mut self, w: &mut World) -> u32 {
        if !w.pending_shop || w.finished() {
            return 0;
        }
        let hole = w.pending_shop_hole;
        w.open_shop(hole);
        let mut bought = 0u32;
        for _ in 0..4 {
            let Some(item) = self.shop_choice(w, hole) else {
                break;
            };
            match crate::game::shop::buy(w, hole, item) {
                crate::game::shop::Purchase::Bought { .. } => bought += 1,
                crate::game::shop::Purchase::Denied(_) => break,
            }
        }
        w.leave_shop();
        self.shop_leg = false;
        self.shop_chase = 0.0;
        // A visit that bought nothing (the shelf held only trades the bot
        // refuses right now) must not be repeated every few seconds: the hole
        // still looks worth a walk to `World::shop_goal_point`.
        self.shop_cd = if bought == 0 {
            SHOP_DRY_COOLDOWN_SECS
        } else {
            SHOP_COOLDOWN_SECS
        };
        bought
    }

    /// The one thing the bot would buy next at `hole`, if anything.
    fn shop_choice(&self, w: &World, hole: usize) -> Option<crate::game::shop::ShopItem> {
        use crate::game::shop::{self, ShopItem};
        let stock = w.shop.get(hole)?;
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        if hp_frac < 0.66 && shop::deny_reason(w, hole, ShopItem::Stitch).is_none() {
            return Some(ShopItem::Stitch);
        }
        if w.player.loadout.splices.len() >= shop::MAX_SPLICES {
            return None;
        }
        for slot in 0..shop::STOCK_SIZE {
            let Some(kind) = stock.slot(slot) else {
                continue;
            };
            if shop::deny_reason(w, hole, ShopItem::Splice(slot)).is_some() {
                continue;
            }
            // A max-HP bargain on a hurt rat, or a healing cut with nothing to
            // heal, is a bad trade: skip it and look at the next hook.
            let cuts_hp = kind == shop::SpliceKind::GlassIncisors
                || kind == shop::SpliceKind::LongFingers;
            if cuts_hp && hp_frac < 0.6 {
                continue;
            }
            return Some(ShopItem::Splice(slot));
        }
        None
    }

    /// Pick up to `campaign::keep_limit` entries (3, plus the nest's carry
    /// track) to carry into the next level, by the same value heuristic
    /// (always at least one weapon).
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
            // The nest's carry track raises the limit; a `--meta max` batch
            // must use the slots it paid for, like the keep screen does.
            .take(crate::game::campaign::keep_limit(lo))
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
            Vision::Full => {
                w.level.tiles.circle_clear(q, r, SizeClass::Small) && pit_clear(w, q, r)
            }
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
        self.miniboss_pos = None;
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
                    // Growth Serum: an ordinary body under a giant rat is not a
                    // hazard, it is a stomp. Elites and named bodies keep their
                    // full weight (they hit back at half damage).
                    stomp_free: w.player.buffs.is_giant()
                        && crate::game::ecs::systems::powerups::giant_crushes(en),
                });
                // Wave 3: the warned attacks that are already on the floor.
                // The body is a threat of its own above; these are the shapes
                // its tell has committed to, so the bot can leave them now
                // instead of walking into them when they bite.
                // A kind whose warned attack lives in a `floor_*` module keeps
                // its state in the same `w3`/`w4` scratch its archetype would
                // use, with a different meaning: read it through that module's
                // own view and never through the archetype's.
                if !self.scan_floor_enemy(w, en, p) {
                    self.scan_wave3_enemy(w, en, p);
                    self.scan_wave4_enemy(w, en, p);
                }
                if let Some(mb) = e.get::<Miniboss>() {
                    if en.alive() && !mb.chest_dropped {
                        self.miniboss_pos = Some(en.pos);
                        if let Some(shape) = mb.telegraph() {
                            // The telegraph is a danger for its whole tell:
                            // treat it as immediately lethal so the bot leaves
                            // the shape before the 1 s tell expires.
                            let active_left = match mb.state {
                                MinibossState::Tell => 1.0,
                                MinibossState::Active => {
                                    (mb.kind.active_duration() - mb.timer).max(0.0) + 0.1
                                }
                                _ => 0.0,
                            };
                            // Expert and Skilled read the shape the moment
                            // it is drawn (`warn_left` 0). Casual reads it the
                            // way it reads a boss telegraph: through its own
                            // coarse timer and short lookahead, so it starts
                            // to leave late and is sometimes still inside.
                            let warn_left = if self.skill == Skill::Casual
                                && mb.state == MinibossState::Tell
                            {
                                self.quantise_warn(
                                    (crate::game::ecs::systems::miniboss::TELL_TIME - mb.timer)
                                        .max(0.0),
                                )
                            } else {
                                0.0
                            };
                            if active_left > 0.0 {
                                self.threats.push(Threat::MinibossHazard {
                                    shape,
                                    warn_left,
                                    active_left,
                                    damage: mb.signature_damage,
                                });
                            }
                        }
                    }
                }
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
                // (the fight is where the level ends, and since batch 12 it
                // starts wherever the rat is), so it is recorded before the
                // neighbourhood filter.
                self.boss_pos = Some(boss.pos);
                self.boss_kind = Some(boss.kind);
                if !self.sees(w, p, boss.pos) {
                    continue;
                }
                // A boss that is out of reach (the hawk overhead) is only a
                // shadow: dodging it would be dodging nothing, and the bot must
                // spend that movement reading the deck marks instead.
                if !boss.overhead() {
                    self.threats.push(Threat::BossBody {
                        pos: boss.pos,
                        radius: boss.kind.body_radius(),
                        damage: boss.contact_damage.max(boss.kind.contact_damage()),
                    });
                }
                for h in &boss.hazards {
                    // A damage-free hazard is a *shove*, not a hit: the
                    // Magpie's wing-beat gusts are push-only, and treating one
                    // as lethal made the bot break its orbit and step into the
                    // gust's own line to get away from it. The pushing still
                    // moves the rat; there is nothing to dodge.
                    if h.is_expired() || !h.hit_player || h.damage <= 0.0 {
                        continue;
                    }
                    self.threats.push(Threat::Hazard {
                        shape: h.shape,
                        warn_left: self.quantise_warn((h.warn - h.elapsed).max(0.0)),
                        active_left: (h.warn + h.active - h.elapsed).max(0.0),
                        damage: h.damage,
                    });
                }
                // The Mangle's intake lane is a damage-free conveyor: the
                // generic hazard field cannot score it (0 damage), so the rat
                // would stand in it and be fed into the bite. Give the lane the
                // whole block's worth of danger until the nip has fired, so the
                // bot steps out or across during the tell.
                if let BossState::Mangle(mstate) = boss.state {
                    if matches!(
                        mstate,
                        crate::game::art_boss_mangle::MangleState::Intake { .. }
                    ) {
                        let view = boss.mangle_view();
                        let lane = crate::game::art_boss_mangle::intake_lane(&view);
                        let (tell, pull) = crate::game::art_boss_mangle::intake_windows(boss.phase);
                        let to_bite = (tell + pull - boss.elapsed).max(0.0);
                        self.threats.push(Threat::Hazard {
                            shape: HazardShape::Rect {
                                center: lane.center,
                                facing: lane.facing,
                                half: lane.half,
                                half_perp: lane.half_perp,
                            },
                            warn_left: self.quantise_warn(to_bite),
                            active_left: to_bite + crate::game::art_boss_mangle::BITE_WINDOW,
                            damage: crate::game::art_boss_mangle::BITE_DAMAGE,
                        });
                    }
                }
            } else if let Some(wick) = e.get::<crate::game::ecs::components::Wick>() {
                // A dead moth's fuse: the burst lands when the fuse runs out,
                // so it reads exactly like a hazard with `fuse` left to warn.
                if self.sees(w, p, wick.pos) {
                    self.threats.push(Threat::WaveHazard {
                        pos: wick.pos,
                        radius: crate::game::ecs::systems::enemy_wave3::WICK_BURST_RADIUS,
                        dir: Vec2::ZERO,
                        warn_left: wick.fuse,
                        active_left: wick.fuse + 0.10,
                        damage: crate::game::ecs::systems::enemy_wave3::WICK_RAT_DAMAGE,
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
                // A live board catches the rat whatever it is carrying (a
                // spent/Used one is harmless floor and drops out of the model).
                let live = matches!(
                    trap.state,
                    crate::game::art_traps::StickyTrapState::Idle
                        | crate::game::art_traps::StickyTrapState::Stuck
                );
                if live && trap.pos.dist_sq(p) < 90.0 * 90.0 && self.sees(w, p, trap.pos) {
                    self.threats.push(Threat::StickyPad {
                        pos: trap.pos,
                        facing: trap.facing,
                        load: trap.load,
                        stuck: trap.victims.iter().flatten().count() as u8,
                    });
                }
            }
            for trap in &w.level.objects.mousetraps {
                use crate::game::art_traps::{MousetrapState, MOUSETRAP_HIT_PROGRESS_MAX};
                let armed = trap.state == MousetrapState::Armed;
                // Touched, bar not landed yet: the rect is about to bite. A
                // `Snapping` trap past its hit window is only the bar settling.
                let committed = trap.state == MousetrapState::Triggered
                    || (trap.state == MousetrapState::Snapping
                        && trap.progress <= MOUSETRAP_HIT_PROGRESS_MAX);
                // A sprung trap is inert, but its bait is still free to take.
                let keeps_bait = !trap.bait_taken && trap.bait.is_good();
                if (armed || committed || keeps_bait)
                    && trap.pos.dist_sq(p) < 90.0 * 90.0
                    && self.sees(w, p, trap.pos)
                {
                    self.threats.push(Threat::Mousetrap {
                        pos: trap.pos,
                        facing: trap.facing,
                        armed,
                        committed,
                        hair: trap.kind.hair(),
                        bait_good: keeps_bait,
                    });
                }
            }
            // The Laundry's steam grates and the Printworks' ink pads. Both
            // lists are empty on every other floor.
            for grate in &w.level.objects.steam_grates {
                use crate::game::hazards::{
                    SteamPhase, STEAM_PLUME_SECS, STEAM_REST_SECS, STEAM_WARN_SECS,
                };
                if grate.pos.dist_sq(p) >= 90.0 * 90.0 || !self.sees(w, p, grate.pos) {
                    continue;
                }
                let (warn_left, active_left) = match grate.phase {
                    SteamPhase::Resting => (
                        (STEAM_REST_SECS - grate.timer).max(0.0) + STEAM_WARN_SECS,
                        0.0,
                    ),
                    SteamPhase::Warning => ((STEAM_WARN_SECS - grate.timer).max(0.0), 0.0),
                    // A plume that has already scalded the rat cannot do it
                    // again (once per occupant per plume).
                    SteamPhase::Plume if grate.player_struck => (STEAM_REST_SECS, 0.0),
                    SteamPhase::Plume => (0.0, (STEAM_PLUME_SECS - grate.timer).max(0.0)),
                };
                self.threats.push(Threat::Steam {
                    pos: grate.pos,
                    warn_left: self.quantise_warn(warn_left),
                    active_left,
                });
            }
            for pad in &w.level.objects.ink_pads {
                // The pad is a rectangle of whole cells, so the range test is
                // against its nearest edge, not its centre.
                let dx = ((p.x - pad.pos.x).abs() - pad.half.x).max(0.0);
                let dy = ((p.y - pad.pos.y).abs() - pad.half.y).max(0.0);
                if dx * dx + dy * dy < 90.0 * 90.0 && self.sees(w, p, pad.pos) {
                    self.threats.push(Threat::Ink {
                        pos: pad.pos,
                        half: pad.half,
                    });
                }
            }
        }
    }

    /// Push the warned floor shape of a kind owned by a `floor_*` module (batch
    /// 6): the Sump Leech's tongue, the Egg Timer's ring, the Walking Syringe's
    /// needle ray and the Peg Snapper's sliding jaws. Each is read through the
    /// module's own `&Enemy` view (the one geometry its hit test and its art
    /// use), never through the raw scratch fields.
    ///
    /// Returns `true` when the kind is one of these, whether or not a shape is
    /// on the floor right now, so the caller skips the archetype scans (their
    /// reading of the same scratch would be a phantom hazard).
    fn scan_floor_enemy(&mut self, w: &World, en: &Enemy, p: Vec2) -> bool {
        use crate::game::ecs::systems::{floor_kitchen, floor_lab, floor_laundry, floor_sewer};
        match en.kind {
            EnemyKind::SumpLeech => {
                if let Some((lip, tip, phase, _)) = floor_sewer::leech_tongue(en) {
                    let mid = (lip + tip) * 0.5;
                    if self.sees(w, p, mid) {
                        // `w4.t` is the seconds left in the phase. The tongue
                        // only slows, so it is costed as the lightest hazard.
                        let (warn_left, active_left) = if phase == floor_sewer::LEECH_PHASE_WARN {
                            (en.w4.t.max(0.0), en.w4.t.max(0.0) + floor_sewer::LEECH_HOLD_SECS)
                        } else {
                            (0.0, en.w4.t.max(0.0) + 0.05)
                        };
                        self.threats.push(Threat::WaveHazard {
                            pos: mid,
                            radius: lip.dist(tip) * 0.5,
                            dir: (tip - lip).normalized(),
                            warn_left: self.quantise_warn(warn_left),
                            active_left,
                            damage: 2.0,
                        });
                    }
                }
                true
            }
            EnemyKind::EggTimer => {
                // The ring grows from the body out to `EGG_RING_END`; the
                // whole disc it sweeps is the place to be out of. (Standing
                // inside the band once it has passed is safe, but the bot
                // does not plan a hop through a 5 px band.)
                let reach = floor_kitchen::EGG_RING_END + floor_kitchen::EGG_RING_WIDTH * 0.5;
                let timing = if let Some((_, progress)) = floor_kitchen::egg_tell_of(en) {
                    let warn = (1.0 - progress) * floor_kitchen::EGG_WIND_SECS;
                    Some((warn, warn + floor_kitchen::EGG_RING_SECS))
                } else if floor_kitchen::egg_ring_of(en).is_some() {
                    Some((0.0, en.w4.t.max(0.0) + 0.05))
                } else {
                    None
                };
                if let Some((warn_left, active_left)) = timing {
                    if self.sees(w, p, en.pos) {
                        self.threats.push(Threat::WaveHazard {
                            pos: en.pos,
                            radius: reach,
                            dir: Vec2::ZERO,
                            warn_left: self.quantise_warn(warn_left),
                            active_left,
                            damage: floor_kitchen::EGG_RING_DAMAGE,
                        });
                    }
                }
                true
            }
            EnemyKind::WalkingSyringe => {
                if let Some(ray) = floor_lab::syringe_ray(en) {
                    let mid = (ray.from + ray.to) * 0.5;
                    if self.sees(w, p, mid) {
                        let warn_left = if ray.live {
                            0.0
                        } else {
                            (1.0 - ray.progress) * floor_lab::SYRINGE_TELL_SECS
                        };
                        self.threats.push(Threat::WaveHazard {
                            pos: mid,
                            radius: ray.from.dist(ray.to) * 0.5,
                            dir: (ray.to - ray.from).normalized(),
                            warn_left: self.quantise_warn(warn_left),
                            active_left: warn_left + floor_lab::SYRINGE_THRUST_SECS,
                            damage: floor_lab::SYRINGE_DAMAGE,
                        });
                    }
                }
                true
            }
            EnemyKind::PegSnapper => {
                if let Some(view) = floor_laundry::jaw_view(en) {
                    // `view.t` is the seconds *into* the phase. The two bars
                    // slide in from `PEG_JAW_GAP / 2` either side of the
                    // centre; the disc that holds their whole travel is the
                    // shape to leave during the tell.
                    let timing = match view.phase {
                        floor_laundry::PEG_PHASE_TELL => {
                            let warn = (floor_laundry::PEG_TELL_SECS - view.t).max(0.0);
                            Some((warn, warn + floor_laundry::PEG_CLOSE_SECS))
                        }
                        floor_laundry::PEG_PHASE_CLOSE => Some((
                            0.0,
                            (floor_laundry::PEG_CLOSE_SECS - view.t).max(0.0) + 0.05,
                        )),
                        // Shut: the bars are at rest and harmless.
                        _ => None,
                    };
                    if let Some((warn_left, active_left)) = timing {
                        if self.sees(w, p, view.centre) {
                            self.threats.push(Threat::WaveHazard {
                                pos: view.centre,
                                radius: (floor_laundry::PEG_JAW_GAP
                                    + floor_laundry::PEG_JAW_THICK)
                                    * 0.5
                                    + 1.0,
                                dir: Vec2::ZERO,
                                warn_left: self.quantise_warn(warn_left),
                                active_left,
                                damage: floor_laundry::PEG_JAW_DAMAGE,
                            });
                        }
                    }
                }
                true
            }
            EnemyKind::StapleCrawler => {
                use crate::game::ecs::systems::floor_printworks as pw;
                if pw::staple_warned(en) {
                    // The two planted feet are mirrored in `w4.trail[0..2]`;
                    // `w4.t` is the seconds left in the tell or the strike.
                    let left = en.w4.t.max(0.0);
                    let (warn_left, active_left) = if en.w4.phase == pw::STAPLE_PHASE_TELL {
                        (left, left + pw::STAPLE_STRIKE_SECS)
                    } else {
                        (0.0, left + 0.05)
                    };
                    for foot in [en.w4.trail[0], en.w4.trail[1]] {
                        if !self.sees(w, p, foot) {
                            continue;
                        }
                        self.threats.push(Threat::WaveHazard {
                            pos: foot,
                            // The strike's own reach (`staple_hits`): the
                            // spot between the feet is the safe one.
                            radius: pw::STAPLE_FOOT_RADIUS - pw::STAPLE_GRAZE,
                            dir: Vec2::ZERO,
                            warn_left: self.quantise_warn(warn_left),
                            active_left,
                            damage: pw::STAPLE_FOOT_DAMAGE,
                        });
                    }
                }
                true
            }
            EnemyKind::LooseCan => {
                use crate::game::ecs::systems::floor_flatbed as fb;
                if let Some(path) = fb::can_path(en) {
                    // The marked lane, bounce included. The rock clock is not
                    // mirrored on the body, so the whole lane counts as live
                    // from the first rock: the bot leaves it during the tell.
                    let active_left =
                        fb::CAN_ROCK_SECS + fb::CAN_ROLL_LENGTH / fb::CAN_ROLL_SPEED;
                    for i in 0..path.legs() {
                        let (a, b) = path.leg(i);
                        let span = a.dist(b);
                        if span < 1.0 {
                            continue;
                        }
                        let dir = (b - a) * (1.0 / span);
                        let mid = (a + b) * 0.5;
                        if !self.sees(w, p, mid) {
                            continue;
                        }
                        // `WaveHazard` lines are 1 px thick; two of them 2 px
                        // either side of the centre line cover the 3 px lane.
                        for side in [-2.0f32, 2.0] {
                            self.threats.push(Threat::WaveHazard {
                                pos: mid + dir.perp() * side,
                                radius: span * 0.5,
                                dir,
                                warn_left: 0.0,
                                active_left,
                                damage: en.damage,
                            });
                        }
                    }
                }
                true
            }
            // The guest (Sewer): the peck lane, from the body out to
            // `art_duck::PECK_REACH` along the aim the bird locked (or is
            // turning between pecks). One lane, read exactly as the hostile
            // layer draws it (`drake::peck_lane`), so the bot steps off the
            // shape the player sees.
            EnemyKind::Drake => {
                use crate::game::ecs::systems::drake;
                if en.w4.phase == drake::PHASE_TELL || en.w4.phase == drake::PHASE_PECK {
                    let (a, b) = drake::peck_lane(en.pos, en.w4.dir);
                    let mid = (a + b) * 0.5;
                    if self.sees(w, p, mid) {
                        let (warn_left, active_left) = if en.w4.phase == drake::PHASE_TELL {
                            let warn = en.w4.t.max(0.0);
                            (warn, warn + drake::PECK_SECS * drake::PECK_COUNT as f32)
                        } else {
                            (0.0, en.w4.t.max(0.0) + 0.05)
                        };
                        self.threats.push(Threat::WaveHazard {
                            pos: mid,
                            radius: a.dist(b) * 0.5,
                            dir: en.w4.dir,
                            warn_left: self.quantise_warn(warn_left),
                            active_left,
                            damage: drake::PECK_DAMAGE,
                        });
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Push the floor shapes a wave-3 body's tell has already committed to.
    fn scan_wave3_enemy(&mut self, w: &World, en: &Enemy, p: Vec2) {
        use crate::game::ecs::systems::enemy_wave3 as w3;
        // The hazard a body has committed to is its archetype's shape (a
        // registered kind posts the same threat its base body does).
        match en.kind.archetype() {
            EnemyKind::BellFlea => {
                if en.w3.flea_phase == 0 {
                    return;
                }
                // Phase 1: both crosses are pending. Phase 2: the first has
                // landed and the spring back home is pending.
                let landed = en.w3.flea_phase == 2;
                let cross = |pos: Vec2, warn: f32| Threat::WaveHazard {
                    pos,
                    radius: w3::FLEA_CROSS_RADIUS,
                    dir: Vec2::ZERO,
                    warn_left: warn,
                    active_left: warn + 0.20,
                    damage: w3::FLEA_CROSS_DAMAGE,
                };
                if !landed && self.sees(w, p, en.w3.hop_to) {
                    self.threats.push(cross(en.w3.hop_to, en.w3.flea_t));
                }
                if self.sees(w, p, en.w3.home) {
                    let warn = if landed { en.w3.flea_t } else { en.w3.flea_t + w3::FLEA_GAP_SECS };
                    self.threats.push(cross(en.w3.home, warn));
                }
            }
            EnemyKind::Turnkey => {
                if en.w3.bar_t <= 0.0 || !self.sees(w, p, en.w3.bar_pos) {
                    return;
                }
                let warn_left = if en.w3.bar_telling() {
                    en.w3.bar_t - w3::CROSSBAR_LIVE_SECS
                } else if en.w3.bar_spent {
                    return;
                } else {
                    0.0
                };
                self.threats.push(Threat::WaveHazard {
                    pos: en.w3.bar_pos,
                    radius: w3::CROSSBAR_LEN * 0.5,
                    dir: en.w3.bar_dir,
                    warn_left,
                    active_left: en.w3.bar_t,
                    damage: w3::CROSSBAR_DAMAGE,
                });
            }
            _ => {}
        }
    }

    /// Push the floor shapes a wave-4 body has committed to: the snake's lunge
    /// lane *and* its no-walk-body, the centipede's dart, the hornet's dive,
    /// the spear line's reach, the archer's marked arc and a tightening rope.
    #[allow(clippy::needless_range_loop)]
    fn scan_wave4_enemy(&mut self, w: &World, en: &Enemy, p: Vec2) {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        match en.kind.archetype() {
            EnemyKind::BigSnake => {
                // The body is a moving wall: a chain of small soft circles so
                // the bot walks round it instead of being pushed along it.
                for k in 1..en.w4.trail_len as usize {
                    let seg = en.w4.trail[k];
                    if !self.sees(w, p, seg) {
                        continue;
                    }
                    self.threats.push(Threat::WaveHazard {
                        pos: seg,
                        radius: w4::SNAKE_BODY_RADIUS,
                        dir: Vec2::ZERO,
                        warn_left: 0.0,
                        active_left: 0.20,
                        damage: 1.0,
                    });
                }
                // The lunge lane: a 40 px line down the locked aim.
                let live = en.w4.phase == w4::SNAKE_PHASE_COIL
                    || en.w4.phase == w4::SNAKE_PHASE_LUNGE;
                if live {
                    let half = w4::SNAKE_LUNGE_DISTANCE * 0.5;
                    let centre = en.pos + en.w4.dir * half;
                    let warn_left = if en.w4.phase == w4::SNAKE_PHASE_COIL {
                        en.w4.t
                    } else {
                        0.0
                    };
                    self.threats.push(Threat::WaveHazard {
                        pos: centre,
                        radius: half,
                        dir: en.w4.dir,
                        warn_left,
                        active_left: warn_left + 0.30,
                        damage: en.damage,
                    });
                }
            }
            EnemyKind::HouseCentipede => {
                if en.w4.phase != w4::CENTIPEDE_PHASE_FLUTTER {
                    return;
                }
                let half = w4::CENTIPEDE_DART_SPEED * w4::CENTIPEDE_DART_SECS * 0.5;
                self.threats.push(Threat::WaveHazard {
                    pos: en.pos + en.w4.dir * half,
                    radius: half,
                    dir: en.w4.dir,
                    warn_left: en.w4.t,
                    active_left: en.w4.t + 0.25,
                    damage: en.damage,
                });
            }
            EnemyKind::MurderHornet => {
                let live = en.w4.phase == w4::HORNET_PHASE_CHARGE
                    || en.w4.phase == w4::HORNET_PHASE_DIVE;
                if !live {
                    return;
                }
                let half = w4::HORNET_DIVE_SPEED * w4::HORNET_DIVE_SECS * 0.5;
                let warn_left = if en.w4.phase == w4::HORNET_PHASE_CHARGE {
                    en.w4.t
                } else {
                    0.0
                };
                self.threats.push(Threat::WaveHazard {
                    pos: en.pos + en.w4.dir * half,
                    radius: half,
                    dir: en.w4.dir,
                    warn_left,
                    active_left: warn_left + 0.35,
                    damage: w4::HORNET_JAB_DAMAGE,
                });
            }
            EnemyKind::Lilliputian => match (en.w4.role, en.w4.phase) {
                (w4::LILLI_ROLE_SPEAR, w4::LILLI_PHASE_TELL)
                | (w4::LILLI_ROLE_LEADER, w4::LILLI_PHASE_TELL) => {
                    let half = w4::SPEAR_REACH * 0.5;
                    self.threats.push(Threat::WaveHazard {
                        pos: en.pos + en.facing * half,
                        radius: half,
                        dir: en.facing,
                        warn_left: en.w4.t,
                        active_left: en.w4.t + 0.20,
                        damage: en.damage,
                    });
                }
                (w4::LILLI_ROLE_ARCHER, w4::LILLI_PHASE_TELL) => {
                    // The marked arc, sampled as three small circles along the
                    // aim so the bot steps off the line before the pellets.
                    let reach = w4::ARCHER_BULLET_SPEED * 0.8;
                    for k in [-1.0f32, 0.0, 1.0] {
                        let dir = en.w4.dir.rotate(k * w4::ARCHER_SPREAD);
                        let at = en.pos + dir * reach;
                        if !self.sees(w, p, at) {
                            continue;
                        }
                        self.threats.push(Threat::WaveHazard {
                            pos: at,
                            radius: 7.0,
                            dir: Vec2::ZERO,
                            warn_left: en.w4.t,
                            active_left: en.w4.t + 0.25,
                            damage: en.damage,
                        });
                    }
                }
                (w4::LILLI_ROLE_ROPE, w4::LILLI_PHASE_TELL)
                | (w4::LILLI_ROLE_ROPE, w4::LILLI_PHASE_ACTION) => {
                    self.threats.push(Threat::WaveHazard {
                        pos: en.pos,
                        radius: w4::ROPE_REACH,
                        dir: Vec2::ZERO,
                        warn_left: if en.w4.phase == w4::LILLI_PHASE_TELL {
                            en.w4.t
                        } else {
                            0.0
                        },
                        active_left: en.w4.t + 0.25,
                        damage: 2.0,
                    });
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Score moving along `dir` from `p`: a short forward lookahead that
    /// predicts where the rat and every threat would be at three sample
    /// times, so "walk away from a chaser" and "walk into a chaser" are
    /// different numbers instead of the same distance.
    fn score(&self, w: &World, p: Vec2, dir: Vec2, moving: bool) -> f32 {
        let speed = w.player.loadout.stats.move_speed;
        // Seconds ahead, and how much each sample counts.
        // The three samples keep their spacing and scale with the skill's
        // horizon (×1 exactly for Expert and Skilled).
        const TIMES: [f32; 3] = [0.12, 0.27, 0.48];
        const WEIGHTS: [f32; 3] = [0.5, 0.32, 0.18];
        let reach = self.params.horizon / HORIZON_FULL;
        let mut danger = 0.0f32;

        // Walls: standing inside one is fatal to the plan; a short runway is
        // a dead end, and a dead end with a horde behind it is where runs die.
        if !self.clear(w, p, R + 0.5) {
            danger += 100.0;
        }
        // The last steps of a shop leg end *on* a wall face: there the wall
        // terms below (a short runway, a sample inside the wall, a closed-in
        // cell) are the destination, not a hazard. Without this the rat stood
        // 15-30 px from the shelf until it gave the errand up.
        let door = self.shop_leg && p.dist(self.shop_face) < SHOP_DOOR_RADIUS;
        if moving && !door {
            let runway = self.runway(w, p, dir, 140.0);
            let tight = 1.0 - runway / 140.0;
            danger += tight * tight * 9.0;
        }

        // Invulnerability makes contact free: dash-frames are the one moment a
        // rat can cross a pack, so the field must know.
        let invuln = (w.player.iframes / 0.6).clamp(0.0, 1.0);
        let contact_scale = 1.0 - 0.85 * invuln;

        for (i, &t) in TIMES.iter().enumerate() {
            let t = t * reach;
            let q = p + dir * (speed * t);
            let wt = WEIGHTS[i];
            let at_door = door && q.dist(self.shop_face) < SHOP_DOOR_RADIUS * 0.5;
            if !at_door {
                if !self.clear(w, q, R) {
                    danger += 80.0 * wt;
                }
                danger += (DIRS as f32 - self.openness(w, q) as f32) * 1.1 * wt;
            }

            for threat in &self.threats {
                danger += wt * threat_cost(p, *threat, q, t, contact_scale);
            }
        }

        // Escape field: the short lookahead only sees contact range, but being
        // surrounded is decided 100-200 px out. When the rat is inside a pack
        // every nearby direction looks equally bad, so the way out is the
        // direction with the thinnest mass further along it.
        let end = p + dir * (speed * self.params.horizon);
        let mut thin = 0.0f32;
        for (dist, weight) in [(85.0f32, 1.0f32), (165.0, 0.55), (250.0, 0.3)] {
            let probe = p + dir * dist;
            for t in &self.threats {
                if let Threat::Enemy { pos, radius, .. } = *t {
                    let d = pos.dist(probe) - radius;
                    if d < 65.0 {
                        let close = 1.0 - (d.max(0.0) / 65.0);
                        thin += weight * close * close;
                    }
                }
            }
        }
        danger += thin * self.params.crowd;

        // Drifting into the map border funnels a kite into a corner.
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
            let g = if fighting {
                0.5
            } else if self.shop_leg {
                // The threshold is in a wall face, and the field's wall terms
                // (a short runway, a closed-in cell) push the rat off it: the
                // last steps need a pull that beats them, or the rat stands
                // 20 px from the shelf until it gives up.
                if p.dist(self.shop_face) < SHOP_DOOR_RADIUS {
                    SHOP_DOOR_PULL
                } else {
                    SHOP_LEG_PULL
                }
            } else {
                1.4
            };
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
                    } else if self.shop_leg {
                        // On the way to the shelf: gems on the route, not a
                        // detour for every one in sight.
                        self.params.greed.min(SHOP_LEG_PULL * 0.5)
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
                // Wave 3 pocket finds (WAVE3_DESIGN §10). The bot values them
                // by what they buy a surviving rat: a free hit absorbed is
                // worth most when the rat is not already full, the key splits
                // a pack, the watch buys four seconds of standing still to
                // shoot, and the socks are a dash refill with a bonus.
                PickupKind::BottleCap => {
                    if hp_frac < 0.85 {
                        6.0
                    } else {
                        3.5
                    }
                }
                PickupKind::WindupMouse => {
                    if w.enemy_count() > 8 {
                        6.5
                    } else {
                        3.0
                    }
                }
                PickupKind::PocketWatch => {
                    if w.enemy_count() > 12 {
                        5.5
                    } else {
                        3.5
                    }
                }
                PickupKind::CleanSocks => 3.5,
                // Growth Serum (batch 13): twenty seconds in which the horde
                // cannot touch the rat and dies under it. The best find on the
                // floor when there is a crowd, still worth a detour without one.
                PickupKind::GrowthSerum => {
                    if w.enemy_count() > 8 {
                        9.0
                    } else {
                        5.0
                    }
                }
            };
            bonus += want * (1.0 - d / 190.0) * dir.dot((ppos - p).normalized());
        }
        // Boss: close enough for the weapons, not inside the body. The pull is
        // strongest from far away (the boss arrives out of sight, a screen
        // off) and fades to a slow orbit once the rat is in the fight. There is
        // no arena to walk to any more: the fight starts where the rat stands.
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
        // A named miniboss is a fight objective too (wave 3): keep the weapons
        // in range, but stay out of its body and signatures (the hazard cost
        // already does the dodging). The pull is lighter than the boss's.
        if self.boss_pos.is_none() {
            if let Some(mpos) = self.miniboss_pos {
                let d = mpos.dist(p);
                let toward = (mpos - p).normalized();
                bonus += if d > 80.0 {
                    1.6 * dir.dot(toward)
                } else if d > 28.0 {
                    0.8 * dir.dot(toward)
                } else {
                    -1.0
                };
            }
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
                    // Spend the dash only when the hazard is about to fire (or
                    // is live). The old test was `warn_left <= 0.20 ||
                    // warn_left > 0.0`, which is always true: the dash went
                    // the moment the rat stood in a tell, however long it had
                    // left, and was on cooldown when the hazard landed. 0.30 s
                    // covers Skilled's 0.25 s reaction gap and its 0.2 s
                    // timer quantum; the walk out of a longer tell is the
                    // danger field's job.
                    if active_left > 0.0 && shape.contains(p, R + 1.0) && warn_left <= 0.30 {
                        imminent = true;
                    }
                }
                Threat::Mousetrap {
                    pos,
                    facing,
                    committed: true,
                    ..
                } => {
                    // A snap already coming down with the rat in its rect:
                    // the dash's i-frames are the only sure way out.
                    use crate::game::art_traps::{
                        MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_U_MIN,
                    };
                    if crate::game::art_traps::local_box_contains(
                        pos,
                        facing,
                        p,
                        MOUSETRAP_KILL_U_MIN,
                        MOUSETRAP_KILL_U_MAX,
                        MOUSETRAP_KILL_HALF_V,
                    ) {
                        imminent = true;
                    }
                }
                Threat::MinibossHazard { shape, active_left, .. } => {
                    if active_left > 0.0 && shape.contains(p, R + 1.5) {
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
        // A Cat already in chase range: spend the burst to open the gap. Keyed
        // on the archetype, so the registered Cat-bodied chasers (Pipe Eel,
        // Glass Runner, Type Slug, Tar Rat) get the same rule at their own
        // speed.
        for t in &self.threats {
            if let Threat::Enemy {
                pos,
                vel,
                kind,
                radius,
                ..
            } = *t
            {
                if kind.archetype() != EnemyKind::Cat {
                    continue;
                }
                let to_rat = p - pos;
                if to_rat.len() < radius + R + 16.0 && vel.dot(to_rat) > 0.5 * kind.speed() {
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
        // Batch 12: the Sonic Squeak is a forward cone now, so only what stands
        // in front of the rat is worth the charge. A charged Tail Thrash covers
        // the whole disc, so it makes the cone irrelevant.
        let mut cone: Option<f32> = None;
        let mut round = false;
        for weapon in &w.player.loadout.weapons {
            if weapon.kind.is_active() && weapon.timer <= 0.0 {
                match weapon.kind {
                    crate::game::weapon::WeaponKind::SonicSqueak => {
                        cone = Some(crate::game::weapon::SQUEAK_CONE_HALF);
                    }
                    crate::game::weapon::WeaponKind::TailThrash => round = true,
                    _ => {}
                }
                reach = reach.max(weapon.active_reach());
            }
        }
        if reach <= 0.0 {
            return false;
        }
        // The cone's axis is the rat's facing (the bot never aims with a
        // reticle).
        let cone_half = cone.filter(|_| !round);
        let facing = if w.player.face.len_sq() > 1e-4 {
            w.player.face.normalized()
        } else {
            Vec2::new(0.0, -1.0)
        };
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
                if let Some(half) = cone_half {
                    if !crate::game::ecs::systems::weapons::cone_hits_circle(
                        p, facing, reach, half, pos, radius,
                    ) {
                        continue;
                    }
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

    /// Where a shop leg heads next from `p`: a stride along the route to the
    /// threshold, or the threshold itself for the last steps.
    fn shop_waypoint(&self, p: Vec2) -> Vec2 {
        let face = self.shop_face;
        if p.dist(face) < 40.0 {
            return face;
        }
        let dir = self
            .shop_flow
            .as_ref()
            .map_or(Vec2::ZERO, |f| f.dir_at(SizeClass::Small, p));
        if dir.len_sq() < 0.25 {
            face
        } else {
            p + dir * 64.0
        }
    }

    /// Re-roll the waypoint when it is reached or its timer expires.
    fn update_goal(&mut self, w: &World, dt: f32) {
        let p = w.player.pos;
        self.goal_timer -= dt;
        // The shop cooldown is wall-clock: it runs on every decision. (It used
        // to be decremented by one decision's `dt` only on the ticks a leg was
        // re-rolled, and not at all near a boss or miniboss, so the 9 s
        // cooldown really lasted minutes.)
        self.shop_cd = (self.shop_cd - dt.max(0.0)).max(0.0);
        if self.shop_leg {
            self.shop_chase += dt.max(0.0);
            // Follow the route, not the straight line: the next waypoint is a
            // stride along the flow field (the threshold itself once it is
            // close or the field has no opinion, i.e. inside its own room).
            self.goal = self.shop_waypoint(p);
        }
        // A shop leg ends on the threshold itself (`pending_shop`), not 40 px
        // short of it, so only its timer re-plans it.
        let reached = !self.shop_leg && self.goal.dist_sq(p) < 40.0 * 40.0;
        if self.goal_timer > 0.0 && !reached {
            return;
        }
        self.goal_timer = 2.4 + self.rng.random::<f32>() * 0.8;
        self.shop_leg = false;

        // In a boss fight, orbit the boss instead of roaming.
        if let Some(bpos) = self.boss_pos {
            let ang = (p - bpos).angle() + self.orbit_sign * 1.1;
            // Close enough for the short-range weapons (Tail Whip, Squeak
            // Nova, acid pools) to be on the boss, out to the body's reach.
            // Punish time is closer: a jammed Mangle is harmless, so the rat
            // walks in onto the exposed crank instead of orbiting wide.
            let jamming = w.ecs.iter_entities().any(|e| {
                e.get::<Boss>().is_some_and(|b| {
                    matches!(
                        b.state,
                        BossState::Mangle(
                            crate::game::art_boss_mangle::MangleState::Jam
                                | crate::game::art_boss_mangle::MangleState::FinalJam
                        )
                        // A reared-up Megarat and a dragging tail are the same
                        // read: stand on the exposed weak point, not orbit it.
                            | BossState::MegaRat(
                                crate::game::art_boss_megarat::MegaRatState::GnawStuck
                                    | crate::game::art_boss_megarat::MegaRatState::ThrashRecover
                            )
                        // The cobra's two windows read the same way: the fangs
                        // are in the floor, or the head is spent at the end of
                        // its own lane — walk onto the hood, do not orbit it.
                            | BossState::Cobra(
                                crate::game::art_boss_cobra::CobraState::FangsStuck
                                    | crate::game::art_boss_cobra::CobraState::SpiralEnd
                            )
                    )
                })
            });
            let ring = if jamming { 20.0 } else { 32.0 };
            self.goal =
                (bpos + Vec2::from_angle(ang) * ring).clamp_box(Vec2::splat(20.0), w.level.arena - Vec2::splat(20.0));
            return;
        }
        // A living miniboss gets the same slow orbit, a little wider so the
        // 1 s tell is always readable and the signature shapes can be left.
        if let Some(mpos) = self.miniboss_pos {
            let ang = (p - mpos).angle() + self.orbit_sign * 1.1;
            let ring = 38.0;
            self.goal = (mpos + Vec2::from_angle(ang) * ring)
                .clamp_box(Vec2::splat(20.0), w.level.arena - Vec2::splat(20.0));
            return;
        }

        // A mouse-hole shop the purse can actually use pulls the leg: walking
        // past a shelf the build could buy from is leaving gold on the floor.
        // Never during a boss fight (the orbit above owns that) and never twice
        // in a row off the same visit.
        if self.shop_cd <= 0.0 && self.miniboss_pos.is_none() {
            if self.shop_chase > SHOP_CHASE_LIMIT_SECS {
                // The straight line does not get there (a wall, a pack in the
                // doorway): stop trying for a while.
                self.shop_cd = SHOP_DRY_COOLDOWN_SECS;
                self.shop_chase = 0.0;
            } else if let Some(face) = w.shop_goal_point() {
                if p.dist(face) < SHOP_SEEK_RADIUS {
                    let tiles = &w.level.tiles;
                    let flow = self
                        .shop_flow
                        .get_or_insert_with(|| Box::new(crate::game::nav::FlowField::new(tiles)));
                    // The same goal cell again costs nothing.
                    flow.update(tiles, face, crate::game::nav::NavBudget::FULL);
                    self.shop_face = face;
                    self.goal = self.shop_waypoint(p);
                    self.goal_timer = 1.4;
                    self.shop_leg = true;
                    return;
                }
            }
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
            // Under pressure, a leg that crosses a trap is worth choosing: a
            // live glue board pins the pack behind the rat, and an ordinary
            // snap trap's bar flattens a Brute when one is in the chase.
            if crowd.is_some() {
                score += 1.6 * self.trap_crossing_bonus(w, p, q);
            }
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

    /// How strongly a leg from `p` to `q` crosses a trap worth kiting the horde
    /// over: a live glue board (pins the pack) or an **ordinary** snap trap when
    /// a heavy body is in the chase (its bar kills a Brute). A hair-trigger trap
    /// is never an asset, and a spent trap is nothing.
    fn trap_crossing_bonus(&self, w: &World, p: Vec2, q: Vec2) -> f32 {
        use crate::game::art_traps::{
            MousetrapState, StickyTrapState, MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX,
            MOUSETRAP_KILL_U_MIN,
        };
        let heavy_chasing = self.threats.iter().any(|t| {
            matches!(
                t,
                Threat::Enemy { pos, kind, .. }
                    if matches!(kind.archetype(), EnemyKind::Brute | EnemyKind::Boss)
                        && pos.dist(p) < 90.0
            )
        });
        let mut best = 0.0f32;
        for k in 1..=5 {
            let s = p + (q - p) * (k as f32 / 5.0);
            for trap in &w.level.objects.sticky_traps {
                if matches!(trap.state, StickyTrapState::Idle | StickyTrapState::Stuck)
                    && on_pad(trap.pos, trap.facing, s)
                {
                    best = best.max(1.0);
                }
            }
            if heavy_chasing {
                for trap in &w.level.objects.mousetraps {
                    if trap.state != MousetrapState::Armed || trap.kind.hair() {
                        continue;
                    }
                    if crate::game::art_traps::local_box_contains(
                        trap.pos,
                        trap.facing,
                        s,
                        MOUSETRAP_KILL_U_MIN,
                        MOUSETRAP_KILL_U_MAX,
                        MOUSETRAP_KILL_HALF_V,
                    ) {
                        best = best.max(0.8);
                    }
                }
            }
        }
        best
    }

    /// The direction to spend a charged dash through an **ordinary** trap's
    /// plate for a good bait, if one is a dash away and the way is not suicidal.
    ///
    /// Never a hair trigger: only a dash that is *already* under way survives
    /// one, and the bot does not plan those (it avoids their rect entirely in
    /// the danger field).
    fn bait_dash(&self, w: &World) -> Option<Vec2> {
        use crate::game::art_traps::MOUSETRAP_TRIGGER_OFFSET_U;
        if w.player.dash_cd > 0.0 {
            return None;
        }
        let p = w.player.pos;
        let mut best: Option<(f32, Vec2)> = None;
        for t in &self.threats {
            let Threat::Mousetrap {
                pos,
                facing,
                armed,
                hair,
                bait_good,
                ..
            } = *t
            else {
                continue;
            };
            // Only a trap that is still set: one already coming down is a
            // hazard to leave, and a sprung one's bait is a free walk-up.
            if !armed || hair || !bait_good {
                continue;
            }
            let f = if facing.len_sq() > 0.0 {
                facing.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            };
            let plate = pos + f * MOUSETRAP_TRIGGER_OFFSET_U;
            let to = plate - p;
            let d = to.len();
            if !(6.0..=34.0).contains(&d) {
                continue;
            }
            let dir = to.normalized();
            // Do not spend the escape dash into a wall of bodies. (This used
            // to read `score(..) > 10.0`, which is backwards — a high score is
            // a *good* heading — and never fired either way, because the
            // openness term alone keeps every score below zero.)
            if self.enemies_near(plate, 30.0) >= 2 || !self.path_clear(w, p, plate) {
                continue;
            }
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, dir));
            }
        }
        best.map(|(_, dir)| dir)
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

/// How far the bot will walk to a mouse-hole shop the purse can use.
const SHOP_SEEK_RADIUS: f32 = 1400.0;
/// The goal pull of a shop leg. An ordinary waypoint pulls 1.4, which Expert's
/// gem greed (10) and Skilled's (5) always beat: the better the bot, the less
/// it shopped (0.08 / 0.36 / 0.83 purchases a floor, measured).
const SHOP_LEG_PULL: f32 = 5.0;
/// Inside this distance of the threshold the pull is [`SHOP_DOOR_PULL`].
const SHOP_DOOR_RADIUS: f32 = 72.0;
/// The pull of the last steps onto the threshold (the runway and openness
/// terms it has to beat sum to about 12 against a wall).
const SHOP_DOOR_PULL: f32 = 16.0;
/// Seconds of shop legs without reaching the threshold before the bot gives
/// that errand up for [`SHOP_DRY_COOLDOWN_SECS`].
const SHOP_CHASE_LIMIT_SECS: f32 = 40.0;
/// Seconds after a visit before the next shop leg may be planned.
const SHOP_COOLDOWN_SECS: f32 = 9.0;
/// The same, after a visit that bought nothing.
const SHOP_DRY_COOLDOWN_SECS: f32 = 45.0;

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

/// Growth Serum (batch 13): how much of its ordinary-body contact danger the
/// bot still feels while the rat is giant. The bodies cannot hurt the giant and
/// die under it, so the only reason to weigh them at all is that walking over
/// one is a stomp and not a movement mistake.
const STOMP_FREE_CONTACT: f32 = 0.2;

/// Danger of one threat to a candidate position `q` at `t` seconds from now.
fn threat_cost(_p: Vec2, threat: Threat, q: Vec2, t: f32, contact_scale: f32) -> f32 {
    match threat {
        Threat::Enemy {
            pos,
            vel,
            radius,
            damage,
            kind,
            stomp_free,
        } => {
            // Each kind chases at `EnemyKind::speed` px/s (Skitterer 30, Bat
            // 42, Cat 38, Spitter 22, Brute 16) and the velocity sample
            // already carries that: `Enemy::steer` smooths toward
            // `kind.speed() * speed_mult`. Predicting with it is measurably
            // better than a kind-speed chase model (the A/B lost 60 s of
            // median survival), so the kind only decides the weight below.
            let _ = kind;
            // Growth Serum: a body the giant crushes is almost free to walk
            // into (`STOMP_FREE_CONTACT`). This is the one place the giant
            // changes the bot's reading of the floor, and it applies to
            // ordinary bodies only (see the flag's doc).
            let stomp_scale = if stomp_free { STOMP_FREE_CONTACT } else { 1.0 };
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
                w * w * (damage / 6.0).clamp(0.6, 3.0) * 6.0 * contact_scale * stomp_scale
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
        Threat::MinibossHazard {
            shape,
            warn_left,
            active_left,
            damage,
        } => {
            // `warn_left` is 0 for every skill but Casual (see the scan), so
            // the lookahead gate below only ever delays Casual's read.
            if active_left <= 0.0 || !shape.contains(q, R) || warn_left > t + 0.12 {
                0.0
            } else {
                (damage / 10.0).clamp(0.8, 4.0) * 16.0
            }
        }
        Threat::StickyPad {
            pos,
            facing,
            load,
            stuck,
        } => {
            if !on_pad(pos, facing, q) {
                return 0.0;
            }
            // Standing on a live board is a 0.9 s hold for the rat; crossing it
            // is fine. The cost is the same either way per sample, but a board
            // the horde is already stuck to is worth *less* as a hazard to the
            // rat's eye — nothing else will be caught on it — and a crowded
            // board is where the weapons want the rat to be, so the penalty is
            // nudged down with the load (`kite across the glue`).
            let crowded = (stuck as f32 * 0.25 + load as f32 * 0.05).min(1.0);
            3.0 - 1.0 * crowded
        }
        Threat::Steam {
            pos,
            warn_left,
            active_left,
        } => {
            // The plume covers one 16 px cell; a rat in the cell takes 8. Read
            // it like a hazard: leave the cell before the puffs stop, and never
            // walk in while the plume is up.
            let near = (q.x - pos.x).abs() <= crate::game::hazards::STEAM_HALF + R
                && (q.y - pos.y).abs() <= crate::game::hazards::STEAM_HALF + R;
            if !near {
                0.0
            } else if active_left > 0.0 {
                14.0
            } else if warn_left <= t + 0.12 {
                12.0
            } else {
                // A grate that is still resting is a mild "don't stand here".
                1.5
            }
        }
        Threat::Ink { pos, half } => {
            let on = (q.x - pos.x).abs() <= half.x + R && (q.y - pos.y).abs() <= half.y + R;
            if on {
                2.5
            } else {
                0.0
            }
        }
        Threat::Mousetrap {
            pos,
            facing,
            armed,
            committed,
            hair,
            ..
        } => {
            use crate::game::art_traps::{
                MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_U_MIN,
                MOUSETRAP_TRIGGER_OFFSET_U, MOUSETRAP_TRIGGER_RADIUS,
            };
            let f = if facing.len_sq() > 0.0 {
                facing.normalized()
            } else {
                Vec2::new(1.0, 0.0)
            };
            let d = q - pos;
            let (u, v) = (d.dot(f), d.dot(f.perp()));
            let in_kill = (MOUSETRAP_KILL_U_MIN - R..=MOUSETRAP_KILL_U_MAX + R).contains(&u)
                && v.abs() <= MOUSETRAP_KILL_HALF_V + R;
            let on_plate = (q - pos - f * MOUSETRAP_TRIGGER_OFFSET_U).len()
                < MOUSETRAP_TRIGGER_RADIUS + R;
            if committed {
                // The plate has been touched (by the rat or by the horde): the
                // bar lands inside ~0.26 s for a third of the rat's health, so
                // the rect is a firing hazard, not a nuisance.
                if in_kill {
                    40.0
                } else {
                    0.0
                }
            } else if armed && hair && in_kill {
                // A hair-trigger trap lands on the touch tick, so its rect is
                // never worth entering (a dash that is *already* running is
                // the only way through, and the bot does not plan those).
                40.0
            } else if armed && on_plate {
                // Touching the plate springs it. An ordinary trap gives 0.26 s
                // from the touch to the bite, and the plate sits 10 px inside
                // the rect on every side: at the base 58 px/s a walking rat
                // does **not** get out in time, so the plate of an ordinary
                // trap is as much a no-go as a hair trigger's (the old 6-point
                // cost lost to any gem and is why the Skilled bot ate snaps).
                // `bait_dash` is the one deliberate way across.
                30.0
            } else if armed && in_kill {
                // The rest of an ordinary trap's rect only bites if something
                // steps on the plate while the rat is inside: a mild cost, so
                // a leg can still drag a Brute across it.
                10.0
            } else {
                0.0
            }
        }
        Threat::WaveHazard {
            pos,
            radius,
            dir,
            warn_left,
            active_left,
            damage,
        } => {
            if active_left <= 0.0 {
                return 0.0;
            }
            let inside = if dir.len_sq() > 0.5 {
                // A crossbar: the line `pos ± dir * radius`, 1 px thick.
                let half = dir * radius;
                crate::game::collision::segment_circle_hit(pos - half, pos + half, 1.0, q, R)
            } else {
                pos.dist(q) <= radius + R
            };
            if !inside {
                0.0
            } else if warn_left > t + 0.12 {
                // It does not fire inside the lookahead: leave it for later.
                0.0
            } else {
                // A floor hazard that is about to bite is worth more than a
                // body's contact: it cannot be walked around mid-swing.
                (damage / 8.0).clamp(0.8, 4.0) * 15.0 * (1.0 - 0.5 * contact_scale)
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
    pit_clear(w, q, r)
}

/// The flatbed's holes are open floor to the tile grid, so the bot has to read
/// the deck's hazard list itself: it never plans a waypoint, a runway or a dash
/// through a broken plank or a missing rail section. Every other theme has an
/// empty list, so this is one length check there.
fn pit_clear(w: &World, q: Vec2, r: f32) -> bool {
    if w.level.trailer.pits.is_empty() {
        return true;
    }
    let m = r + 2.0;
    !w.level.trailer.pits.iter().any(|pit| {
        let rect = pit.rect.to_world();
        q.x + m > rect.x && q.x - m < rect.right() && q.y + m > rect.y && q.y - m < rect.bottom()
    })
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
        // Wave 3: the laser is a damage stream that closes on the aim line from
        // both edges of its arc (and from rank four, crosses and keeps going),
        // with the longest reach on the rack (batch 13: ×1.6, 54–86 px): a good
        // pick once a second weapon covers the sides; Super Dash is escape
        // damage that also thins a chase, so it ranks with the panic buttons.
        WeaponKind::LaserEyes => 9.0,
        WeaponKind::SuperDash => 8.8,
        // Wave 3: Acid Rain is area denial that needs no aiming (a horde
        // weapon, like Tail Whip); Hantavirus needs a pack around the host to
        // pay off, so it ranks below the direct damage weapons.
        WeaponKind::AcidRain => 8.5,
        WeaponKind::Hantavirus => 7.5,
        // The Spiked Tail is the build's heavy single target with a little
        // area on top: better than the spraying weapons into a Brute or a
        // miniboss, worse than a real crowd clearer.
        WeaponKind::SpikedTail => 8.6,
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
                let base = if early { base + 1.0 } else { base };
                // The pick that maxes a weapon whose partner is already owned
                // makes the next chest an evolution: worth chasing.
                if completes_evolution(lo, w) {
                    base + EVOLUTION_VALUE
                } else {
                    base
                }
            }
            None => 0.0,
        },
        // A partner for an owned L4/L5 weapon is an evolution on the next
        // chest: worth as much as a defensive staple.
        Upgrade::Passive(p) => {
            let base = passive_value(lo, *p, hp_frac, early);
            if lo.priority_partner() == Some(*p) {
                base + EVOLUTION_VALUE
            } else {
                base
            }
        }
        // Fallback consumables: a bandage is worth what the HP is worth.
        Upgrade::Bandage => {
            if hp_frac < 0.5 {
                7.5
            } else if hp_frac < 0.8 {
                5.0
            } else {
                1.5
            }
        }
        Upgrade::Purse => 3.0,
    }
}

/// The card the deterministic valuation takes: the highest [`upgrade_value`],
/// ties to the earliest slot. This is the choice the auto-pick toggle makes
/// without opening the card screen, and what [`Bot::pick_upgrade`] returns when
/// its skill takes no card-read noise — so a manual pick and an auto pick from
/// the same state land on the same card. It draws no rng and reads only the
/// loadout + health, so the same state always gives the same card.
///
/// The cards are already filtered by [`Loadout::generate_choices_with_hp`]
/// (slot caps, evolution partners, the bandage hidden at full health), so this
/// only ranks legal cards.
pub fn best_upgrade(w: &World, choices: &[Upgrade]) -> usize {
    let lo = &w.player.loadout;
    let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
    let mut best = 0usize;
    let mut best_value = f32::NEG_INFINITY;
    for (i, up) in choices.iter().enumerate() {
        let v = upgrade_value(lo, up, hp_frac);
        if v > best_value {
            best_value = v;
            best = i;
        }
    }
    best
}

/// How much the bot values the pick that completes an evolution (a weapon
/// reaching L5 with its partner owned, or the partner itself arriving).
const EVOLUTION_VALUE: f32 = 4.0;

/// Would this level-up make `weapon` evolvable (L5 + partner owned)?
fn completes_evolution(lo: &Loadout, weapon: &crate::game::weapon::Weapon) -> bool {
    if weapon.evolved || weapon.level + 1 < weapon.kind.max_level() {
        return false;
    }
    weapon
        .kind
        .evolution()
        .is_some_and(|e| lo.stacks(e.passive) > 0)
}

/// The passive half of [`upgrade_value`] (split out so the evolution-partner
/// bonus is one place).
fn passive_value(lo: &Loadout, p: PassiveKind, hp_frac: f32, early: bool) -> f32 {
    match p {
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
        // Wave 3. The bot has no opinion about the new passives beyond
        // "take one early and their ranks later": Spring Heel is escape
        // (like armour), the reach/zone ones are build multipliers, and
        // Second Scent is a slow XP trickle.
        PassiveKind::SpringHeel => {
            if early { 7.0 } else { 5.0 }
        }
        PassiveKind::Counterweight => 5.5,
        PassiveKind::LongFuse => 5.5,
        PassiveKind::SecondScent => 5.0,
        PassiveKind::LensCap => if early { 4.5 } else { 6.0 },
    }
}

/// The keep-three value of one manifest entry (same scale as
/// [`upgrade_value`]).
fn keep_value(lo: &Loadout, k: &Keepable, hp_frac: f32) -> f32 {
    match k.kind {
        crate::game::campaign::KeepKind::Weapon(kind) => {
            let evolved = lo
                .weapons
                .iter()
                .find(|w| w.kind == kind)
                .is_some_and(|w| w.evolved);
            let base = weapon_tier(kind) * (1.0 + 0.18 * k.level as f32);
            // An earned evolution keeps working without its partner (design
            // §1 campaign rule), so it is the strongest thing on the manifest.
            if evolved {
                base + EVOLUTION_VALUE
            } else {
                base
            }
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

    /// Gold in the purse and a shelf within reach: the bot's waypoint moves to
    /// the hole's face (it walks to the shop instead of kiting past it).
    #[test]
    fn the_bot_walks_to_a_shop_it_can_afford() {
        let mut w = World::new(Theme::Sewer, 11);
        w.gold = 200;
        let hole = w.level.objects.shop_holes[0];
        let face = hole.face();
        // A stride off the face: the threshold has not fired, but the hole is
        // in reach of the shop seek.
        let side = Vec2::new(-hole.facing.y, hole.facing.x);
        w.player.pos = face + hole.facing * 120.0 + side * 40.0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        let _ = bot.decide(&w, 1.0 / 60.0);
        let (goal, _) = bot.debug_goal();
        // The leg is to that threshold; the waypoint is a stride along the
        // route there (the flow field's), so it is nearer the face than the
        // rat is, or the face itself for the last steps.
        assert!(bot.shop_leg, "a shelf it can afford starts a shop leg");
        assert!(bot.shop_face.dist(face) < 1.5, "{:?} vs {face:?}", bot.shop_face);
        assert!(
            goal.dist(face) < w.player.pos.dist(face),
            "the waypoint leads toward the hole's face ({goal:?} vs {face:?})"
        );
        w.player.pos = face + hole.facing * 20.0;
        let mut close = seeded_bot(Skill::Expert, 3);
        let _ = close.decide(&w, 1.0 / 60.0);
        let (goal, _) = close.debug_goal();
        assert!(
            goal.dist(face) < 1.5,
            "the last steps aim at the face itself ({goal:?} vs {face:?})"
        );
        w.player.pos = face + hole.facing * 120.0 + side * 40.0;

        // No gold: no shop leg, the rat goes back to kiting open floor.
        w.gold = 0;
        let mut bot = seeded_bot(Skill::Expert, 3);
        let _ = bot.decide(&w, 1.0 / 60.0);
        let (goal, _) = bot.debug_goal();
        assert!(goal.dist(face) > 1.5, "a broke rat does not detour");
    }

    /// The bot buys at the open shelf and leaves it: it never stalls, and it
    /// comes back off the cooldown rather than pacing between two holes.
    #[test]
    fn the_bot_buys_at_the_shelf_and_leaves_it() {
        let mut w = World::new(Theme::Sewer, 12);
        w.gold = 300;
        w.player.hp = w.player.max_hp() * 0.4;
        // The threshold system's contract: the flag plus the hole index.
        w.pending_shop = true;
        w.pending_shop_hole = 0;
        let mut bot = seeded_bot(Skill::Expert, 4);
        let bought = bot.shop_visit(&mut w);
        assert!(bought > 0, "a rich, hurt rat buys something");
        assert!(!w.pending_shop, "the bot always leaves the shelf");
        assert_eq!(
            w.player.loadout.splices.len(),
            bought.min(crate::game::shop::MAX_SPLICES as u32) as usize,
            "it spliced something in"
        );
        // A second visit inside the cooldown does not plan another leg.
        let hole = w.level.objects.shop_holes[0];
        w.player.pos = hole.face() + hole.facing * 60.0;
        let _ = bot.decide(&w, 1.0 / 60.0);
        let (goal, _) = bot.debug_goal();
        assert!(
            goal.dist(hole.face()) > 1.5,
            "the shop cooldown keeps the rat off the shelf"
        );
    }

    /// Sockets empty on every floor: the bot's "both sockets full, nothing to
    /// buy" rule reads the live loadout, and a campaign level starts with no
    /// splices (`campaign::next_loadout` drops them), so it splices again on the
    /// next floor instead of remembering a full rat.
    #[test]
    fn the_bot_splices_again_on_the_next_floor() {
        use crate::game::campaign::CampaignState;
        let mut camp = CampaignState::new(77);
        let mut w = camp.world();
        w.gold = 600;
        w.pending_shop = true;
        w.pending_shop_hole = 0;
        let mut bot = seeded_bot(Skill::Expert, 6);
        let bought = bot.shop_visit(&mut w);
        assert_eq!(bought, 2, "a rich rat fills both sockets on floor one");
        assert_eq!(w.player.loadout.splices.len(), 2);

        // Clear the floor and carry the build the way the App does.
        w.mark_level_complete();
        camp.complete_level(&w, &[]).expect("an empty pack is legal");
        let mut next = camp.world();
        assert!(
            next.player.loadout.splices.is_empty(),
            "a splice rode into the next floor, so the bot would skip the shelf"
        );
        next.gold = 600;
        next.pending_shop = true;
        next.pending_shop_hole = 0;
        let again = bot.shop_visit(&mut next);
        assert!(
            again > 0,
            "the bot thought the sockets were still full on the new floor"
        );
        assert_eq!(next.player.loadout.splices.len(), again as usize);
    }

    /// A headless run on each wave 3 floor: the bot survives the opening, keeps
    /// moving (never frozen for a long window), and its path crosses the floor.
    ///
    /// The hazards are the point: a bot that walks into every steam plume or
    /// stands on ink loses its escape speed and gets caught. `Skill::Skilled`
    /// (a good human) has to clear the first two minutes on both floors.
    #[test]
    fn the_bot_plays_both_new_floors_without_getting_stuck() {
        for theme in [Theme::Laundry, Theme::Printworks] {
            for seed in [3u64, 11] {
                let mut w = World::new(theme, seed);
                let mut bot = seeded_bot(Skill::Skilled, seed);
                let dt = 1.0 / 30.0;
                let mut last = w.player.pos;
                let mut still = 0.0f32;
                let mut worst_still = 0.0f32;
                let mut travelled = 0.0f32;
                let mut steps = 0u32;
                while w.elapsed < 60.0 && w.player.alive() {
                    let input = bot.decide(&w, dt);
                    w.update_with_charge(dt, input.move_dir, input.dash, input.dash, None, input.charge);
                    // Spend level-ups like the app does (first card), so the
                    // build keeps up with the director.
                    if w.pending_levelups > 0 {
                        w.pending_levelups = 0;
                        let choices = w.player.loadout.generate_choices(&mut w.rng);
                        if let Some(choice) = choices.first() {
                            let choice = choice.clone();
                            w.player.loadout.apply(&choice);
                        }
                    }
                    let moved = w.player.pos.dist(last);
                    travelled += moved;
                    last = w.player.pos;
                    still = if moved < 0.4 { still + dt } else { 0.0 };
                    worst_still = worst_still.max(still);
                    steps += 1;
                }
                assert!(steps > 60, "{theme:?} seed {seed}: the run never ran");
                assert!(
                    worst_still < 6.0,
                    "{theme:?} seed {seed}: the bot stood still for {worst_still:.1} s"
                );
                assert!(
                    travelled > 900.0,
                    "{theme:?} seed {seed}: the bot only travelled {travelled:.0} px"
                );
                assert!(
                    w.elapsed >= 40.0,
                    "{theme:?} seed {seed}: died at {:.1} s on a new floor",
                    w.elapsed
                );
            }
        }
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
        // Expert and Skilled share the full lookahead (and the full read of
        // the crowd); only Casual is short-sighted.
        assert_eq!(expert.horizon, HORIZON_FULL);
        assert_eq!(skilled.horizon, HORIZON_FULL);
        assert!(skilled.horizon > casual.horizon);
        assert_eq!(expert.crowd, CROWD_FULL);
        assert_eq!(skilled.crowd, CROWD_FULL);
        assert!(skilled.crowd > casual.crowd);
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

    /// The flatbed: the bot crosses the whole deck and kills the boss without
    /// ever standing over a broken plank or a missing rail section.
    ///
    /// The clock is wound forward to just before the boss so the test measures
    /// the floor and the fight, not four minutes of build-up; `pit_clear` is
    /// what keeps every waypoint, runway and dash off the holes.
    #[test]
    fn the_bot_never_stands_on_a_hole_and_clears_the_flatbed() {
        use crate::game::weapon::WeaponKind;
        // A seed whose flatbed pool draws **the magpie**: this test (and its
        // 600 s fight clock) measures the magpie's pool, and the flatbed now
        // pools two birds. The Redtail's own fight has its own test
        // (`boss::hawk::tests::a_bot_kills_the_hawk_on_the_real_deck`).
        let mut w = World::new(Theme::Trailer, 5);
        {
            let lo = &mut w.player.loadout;
            for kind in [
                WeaponKind::CheeseSpray,
                WeaponKind::SqueakNova,
                WeaponKind::SporeOrbit,
                WeaponKind::TailWhip,
            ] {
                if !lo.has(kind) {
                    lo.apply(&Upgrade::NewWeapon(kind));
                }
            }
            for _ in 0..8 {
                for i in 1..lo.weapons.len() {
                    lo.apply(&Upgrade::LevelWeapon(i));
                }
            }
            for passive in [
                PassiveKind::MaxHp,
                PassiveKind::Armor,
                PassiveKind::Damage,
                PassiveKind::Regen,
            ] {
                for _ in 0..4 {
                    lo.apply(&Upgrade::Passive(passive));
                }
            }
        }
        w.player.hp = w.player.max_hp();
        w.elapsed = crate::game::director::BOSS_TIME - 6.0;
        let mut bot = seeded_bot(Skill::Expert, 9);
        let dt = 1.0 / 60.0;
        let mut ticks = 0u32;
        // The Magpie's pool is still on its pre-balance scale (a fight runs
        // 150-380 s depending on the seed's loot): the wave 3 balance pass
        // brings the fight, and this limit, back down.
        let limit = 60 * 600;
        while ticks < limit && !w.level_complete && w.player.alive() {
            let input = bot.decide(&w, dt);
            w.update_with_charge(dt, input.move_dir, input.dash, input.dash, None, input.charge);
            assert!(
                w.level.trailer.pit_at(w.player.pos).is_none(),
                "the bot stepped into a hole at t={:.1}",
                w.elapsed
            );
            ticks += 1;
        }
        assert!(
            w.player.alive(),
            "the bot died on the flatbed after {ticks} ticks"
        );
        assert!(
            w.level_complete,
            "the bot did not clear the flatbed's boss in {ticks} ticks ({:.0} s)",
            ticks as f32 * dt
        );
    }

    /// The bot chases all twelve evolutions, the four wave-3 rows included: the
    /// partner of an owned L5 weapon is worth more than the same passive
    /// without a weapon waiting for it, and maxing a weapon whose partner is
    /// already owned is worth more than the last rank of one whose partner is
    /// not.
    #[test]
    fn the_bot_values_every_evolution_partner_including_the_wave3_rows() {
        use crate::game::weapon::{Weapon, WeaponKind};
        for kind in WeaponKind::all() {
            let evo = kind
                .evolution()
                .unwrap_or_else(|| panic!("{kind:?} has no evolution row"));
            // An L5 weapon with no partner: the partner is the evolution pick.
            let mut weapon = Weapon::new(kind);
            weapon.level = kind.max_level();
            let bare = Loadout::from_parts(vec![weapon.clone()], Vec::new());
            let with_partner = upgrade_value(&bare, &Upgrade::Passive(evo.passive), 1.0);
            let without_partner = upgrade_value(&Loadout::starting(), &Upgrade::Passive(evo.passive), 1.0);
            assert!(
                with_partner > without_partner,
                "{kind:?}: the partner of an owned L5 weapon ({with_partner}) must                  outrank the same passive alone ({without_partner})"
            );
            // …and the L4 → L5 rank that completes it outranks the same rank
            // without the partner owned.
            let mut l4 = Weapon::new(kind);
            l4.level = kind.max_level() - 1;
            let completing = Loadout::from_parts(vec![l4.clone()], vec![(evo.passive, 1)]);
            let plain = Loadout::from_parts(vec![l4], Vec::new());
            let a = upgrade_value(&completing, &Upgrade::LevelWeapon(0), 1.0);
            let b = upgrade_value(&plain, &Upgrade::LevelWeapon(0), 1.0);
            assert!(
                a > b,
                "{kind:?}: completing an evolution ({a}) must outrank a bare rank ({b})"
            );
            assert_eq!(completing.priority_partner(), None, "{kind:?}: already paired");
        }
    }

    /// The bot spends a charged Super Dash only when the lane ahead is a
    /// corridor of bodies (or the boss's body sits on it), and taps otherwise.
    #[test]
    fn the_bot_charges_a_super_dash_only_into_a_lane_of_bodies() {
        use crate::math::Vec2;

        let mut w = World::new(Theme::Sewer, 41);
        w.clear_enemies();
        let mut bot = seeded_bot(Skill::Expert, 41);
        bot.scan(&w);
        let ahead = Vec2::new(1.0, 0.0);
        // No weapon: never a charge, whatever the lane looks like.
        let p = w.player.pos;
        for i in 0..3 {
            w.spawn_enemy(
                crate::game::enemy::EnemyKind::Brute,
                p + Vec2::new(14.0 + 6.0 * i as f32, 0.5),
                4.0,
            );
        }
        bot.scan(&w);
        assert_eq!(
            bot.charge_for_lane(&w, ahead),
            0.0,
            "a charge without the weapon would be a wasted dash"
        );

        // With Super Dash owned and three bodies in the lane: a full charge.
        w.player
            .loadout
            .apply(&Upgrade::NewWeapon(WeaponKind::SuperDash));
        assert_eq!(
            bot.charge_for_lane(&w, ahead),
            1.0,
            "three bodies in the lane must be worth a charged dash"
        );

        // Behind the rat, or off to the side: only a tap.
        w.clear_enemies();
        for i in 0..3 {
            w.spawn_enemy(
                crate::game::enemy::EnemyKind::Brute,
                p - Vec2::new(14.0 + 6.0 * i as f32, 0.0),
                4.0,
            );
        }
        bot.scan(&w);
        assert_eq!(
            bot.charge_for_lane(&w, ahead),
            0.0,
            "a lane behind the rat is not a lane"
        );

        // The boss's body on the lane is worth it on its own.
        w.clear_enemies();
        bot.threats.clear();
        bot.threats.push(Threat::BossBody {
            pos: p + Vec2::new(40.0, 0.0),
            radius: 14.0,
            damage: 20.0,
        });
        assert_eq!(
            bot.charge_for_lane(&w, ahead),
            1.0,
            "the boss on the lane must be worth a charged dash"
        );
        // …but not from behind.
        bot.threats.clear();
        bot.threats.push(Threat::BossBody {
            pos: p - Vec2::new(40.0, 0.0),
            radius: 14.0,
            damage: 20.0,
        });
        assert_eq!(
            bot.charge_for_lane(&w, ahead),
            0.0,
            "a boss behind the rat is not a lane"
        );
    }

    /// Batch 6: the plate of an **ordinary** trap is as much a no-go as a hair
    /// trigger's (a walking rat cannot leave the rect in the 0.26 s between
    /// the touch and the bite), a bar that is already coming down makes the
    /// whole rect a hazard, and a bar that has landed is nothing.
    #[test]
    fn an_ordinary_trap_plate_is_a_no_go_and_a_committed_bar_is_a_hazard() {
        use crate::game::art_traps::MOUSETRAP_TRIGGER_OFFSET_U;
        let pos = Vec2::new(400.0, 400.0);
        let facing = Vec2::new(1.0, 0.0);
        let plate = pos + facing * MOUSETRAP_TRIGGER_OFFSET_U;
        // Inside the kill rect, clear of the plate circle.
        let corner = pos + Vec2::new(-2.0, 9.0);
        let far = pos + Vec2::new(60.0, 0.0);
        let trap = |armed: bool, committed: bool, hair: bool| Threat::Mousetrap {
            pos,
            facing,
            armed,
            committed,
            hair,
            bait_good: false,
        };
        let cost = |t: Threat, q: Vec2| threat_cost(Vec2::ZERO, t, q, 0.12, 1.0);

        let ordinary = trap(true, false, false);
        assert!(cost(ordinary, plate) >= 30.0, "the plate must outweigh any loot pull");
        assert!(cost(ordinary, plate) >= cost(trap(true, false, true), plate) * 0.75);
        let rect = cost(ordinary, corner);
        assert!(rect > 0.0 && rect < cost(ordinary, plate), "the rest of the rect is mild");
        assert_eq!(cost(ordinary, far), 0.0);

        let coming_down = trap(false, true, false);
        assert!(cost(coming_down, corner) >= 40.0, "a committed bar bites the whole rect");
        assert_eq!(cost(coming_down, far), 0.0);

        // Landed (or sprung with a bait left): free floor.
        let spent = trap(false, false, false);
        assert_eq!(cost(spent, plate), 0.0);
        assert_eq!(cost(spent, corner), 0.0);
    }

    /// Batch 6: a snap already coming down with the rat in its rect spends the
    /// dash (the i-frames are the only sure way out for a 0.25 s reaction).
    #[test]
    fn a_committed_snap_under_the_rat_spends_the_dash() {
        let mut w = World::new(Theme::Sewer, 17);
        w.clear_enemies();
        w.player.dash_cd = 0.0;
        let p = w.player.pos;
        for skill in [Skill::Expert, Skill::Skilled] {
            let mut bot = seeded_bot(skill, 17);
            bot.threats.clear();
            bot.threats.push(Threat::Mousetrap {
                pos: p - Vec2::new(4.0, 0.0),
                facing: Vec2::new(1.0, 0.0),
                armed: false,
                committed: true,
                hair: false,
                bait_good: false,
            });
            assert!(bot.wants_dash(&w, &unit_dirs()), "{skill:?} stood under the bar");
            // The same trap still merely armed is not a reason to dash.
            bot.threats.clear();
            bot.threats.push(Threat::Mousetrap {
                pos: p - Vec2::new(4.0, 0.0),
                facing: Vec2::new(1.0, 0.0),
                armed: true,
                committed: false,
                hair: false,
                bait_good: false,
            });
            assert!(!bot.wants_dash(&w, &unit_dirs()), "{skill:?} dashed off an armed trap");
        }
    }

    /// Batch 6: the scan used to never build `Threat::Steam` / `Threat::Ink`,
    /// so the Laundry's plumes were invisible to every skill.
    #[test]
    fn the_scan_reads_steam_grates() {
        use crate::game::hazards::{SteamGrate, STEAM_REST_SECS};
        let mut w = World::new(Theme::Sewer, 9);
        w.clear_enemies();
        let p = w.player.pos;
        // Half a second into the 1 s tell, two cells from the rat.
        let cell = cell_of(p + Vec2::new(32.0, 0.0));
        w.level
            .objects
            .steam_grates
            .push(SteamGrate::new(cell, STEAM_REST_SECS + 0.5));

        let mut expert = seeded_bot(Skill::Expert, 9);
        expert.scan(&w);
        let warn = expert.threats.iter().find_map(|t| match *t {
            Threat::Steam {
                warn_left,
                active_left,
                ..
            } => Some((warn_left, active_left)),
            _ => None,
        });
        let (warn_left, active_left) = warn.expect("the expert never saw the grate");
        assert!((warn_left - 0.5).abs() < 1e-3, "warn_left = {warn_left}");
        assert_eq!(active_left, 0.0);

        // Casual is not trap-aware: it keeps walking into them.
        let mut casual = seeded_bot(Skill::Casual, 9);
        casual.scan(&w);
        assert!(!casual.threats.iter().any(|t| matches!(t, Threat::Steam { .. })));
    }

    /// Casual's short horizon is real: the same bot, given the full lookahead,
    /// penalises walking at a body ahead of it much more. (The parameter used
    /// to be a label that `score` never read.)
    #[test]
    fn casual_looks_a_shorter_way_ahead() {
        let mut w = World::new(Theme::Sewer, 5);
        w.clear_enemies();
        let p = w.player.pos;
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(52.0, 0.0), 1.0);
        let toward = Vec2::new(1.0, 0.0);
        let danger = |horizon: f32| -> f32 {
            let mut bot = seeded_bot(Skill::Casual, 3);
            bot.params.horizon = horizon;
            bot.scan(&w);
            bot.goal = p;
            // Less is worse: the heading at the body against the heading away.
            bot.score(&w, p, -toward, true) - bot.score(&w, p, toward, true)
        };
        let short = danger(CASUAL_HORIZON);
        let full = danger(HORIZON_FULL);
        assert!(
            full > short + 0.5,
            "the full lookahead reads the body ahead ({full:.2}) and Casual's barely does ({short:.2})"
        );
        // Expert and Skilled are untouched by the wiring: the scale is exactly 1.
        assert_eq!(Params::for_skill(Skill::Skilled).horizon / HORIZON_FULL, 1.0);
        assert_eq!(Params::for_skill(Skill::Expert).horizon / HORIZON_FULL, 1.0);
    }

    /// Batch 6: the shop cooldown is wall-clock. It used to tick only on the
    /// decisions that re-rolled a leg, by one decision's `dt`.
    #[test]
    fn the_shop_cooldown_runs_on_every_decision() {
        let w = World::new(Theme::Sewer, 5);
        let mut bot = seeded_bot(Skill::Expert, 5);
        // A leg that is neither reached nor expired: no re-roll this call.
        bot.goal = w.player.pos + Vec2::new(150.0, 0.0);
        bot.goal_timer = 100.0;
        bot.shop_cd = SHOP_COOLDOWN_SECS;
        bot.update_goal(&w, 4.0);
        assert!(
            (bot.shop_cd - (SHOP_COOLDOWN_SECS - 4.0)).abs() < 1e-4,
            "shop_cd = {}",
            bot.shop_cd
        );
        bot.on_new_level(1);
        assert_eq!(bot.shop_cd, 0.0, "a new floor's holes are not on cooldown");
    }

    /// Batch 6: Casual is worse *on purpose*, in the ways a new player is —
    /// it re-plans late (it holds one input for its whole 0.45 s reaction) and
    /// it does not dash off a body the way the other two do (its only dash is
    /// the low-health panic) — not through a side effect.
    #[test]
    fn casual_is_slower_and_dashes_later_by_design() {
        let mut w = World::new(Theme::Sewer, 11);
        w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
        w.clear_enemies();
        w.player.pos = w.level.arena * 0.5;
        w.player.dash_cd = 0.0;
        let p = w.player.pos;
        // A heavy body on the rat at full health: the deliberate dash.
        w.spawn_enemy(EnemyKind::Brute, p + Vec2::new(8.0, 0.0), 1.0);
        for skill in [Skill::Expert, Skill::Skilled] {
            let mut bot = seeded_bot(skill, 11);
            bot.scan(&w);
            assert!(bot.wants_dash(&w, &unit_dirs()), "{skill:?} keeps a Brute on it");
        }
        let mut casual = seeded_bot(Skill::Casual, 11);
        casual.scan(&w);
        assert!(
            !casual.wants_dash(&w, &unit_dirs()),
            "Casual has no deliberate dash at full health"
        );

        // Reaction: one decision, then the same input for the rest of the
        // window (20 ticks = 0.33 s < 0.45 s); Expert re-plans every 0.08 s.
        let dt = 1.0 / 60.0;
        let first = casual.decide(&w, dt);
        let made = casual.decisions;
        for _ in 0..20 {
            assert_eq!(casual.decide(&w, dt), first, "Casual changed its mind early");
        }
        assert_eq!(casual.decisions, made);
        let mut expert = seeded_bot(Skill::Expert, 11);
        for _ in 0..21 {
            let _ = expert.decide(&w, dt);
        }
        assert!(expert.decisions >= 3, "Expert re-plans inside the same window");
    }

    // ---- Batch 6: the `floor_*` warned attacks ---------------------------

    /// One body of `kind` 20 px east of the rat, posed by `pose(body, rat)`,
    /// scanned by an Expert: `(rat, body, the hazards it posted)`.
    fn floor_scan(kind: EnemyKind, pose: impl FnOnce(&mut Enemy, Vec2)) -> (Vec2, Enemy, Vec<Threat>) {
        let mut w = World::new(Theme::Sewer, 3);
        let p = w.player.pos;
        let mut en = Enemy::spawn(kind, p + Vec2::new(20.0, 0.0), 1.0, &mut w.rng);
        pose(&mut en, p);
        let mut bot = seeded_bot(Skill::Expert, 3);
        bot.threats.clear();
        assert!(
            bot.scan_floor_enemy(&w, &en, p),
            "{kind:?} is a floor-owned kind: its archetype's scan must be skipped"
        );
        (p, en, bot.threats.clone())
    }

    /// The cost of standing at `q` when every posted hazard has fired.
    fn floor_cost(threats: &[Threat], q: Vec2) -> f32 {
        threats
            .iter()
            .map(|t| threat_cost(Vec2::ZERO, *t, q, 2.0, 1.0))
            .sum()
    }

    #[test]
    fn a_plain_kind_is_not_a_floor_owned_kind() {
        let mut w = World::new(Theme::Sewer, 3);
        let p = w.player.pos;
        let en = Enemy::spawn(EnemyKind::Skitterer, p + Vec2::new(20.0, 0.0), 1.0, &mut w.rng);
        let mut bot = seeded_bot(Skill::Expert, 3);
        bot.threats.clear();
        assert!(!bot.scan_floor_enemy(&w, &en, p));
        assert!(bot.threats.is_empty());
    }

    #[test]
    fn the_bot_reads_the_sump_leech_tongue() {
        use crate::game::ecs::systems::floor_sewer as fs;
        let (_, en, threats) = floor_scan(EnemyKind::SumpLeech, |e, _| {
            e.w4.phase = fs::LEECH_PHASE_WARN;
            e.w4.t = 0.3;
            e.w4.dir = Vec2::new(-1.0, 0.0);
        });
        assert_eq!(threats.len(), 1);
        let (lip, tip) = fs::tongue_segment(en.pos, en.radius, en.w4.dir);
        assert!(floor_cost(&threats, (lip + tip) * 0.5) > 0.0, "on the tongue");
        assert_eq!(floor_cost(&threats, en.pos + Vec2::new(0.0, 30.0)), 0.0, "beside it");
        match threats[0] {
            Threat::WaveHazard { warn_left, .. } => assert!((warn_left - 0.3).abs() < 1e-3),
            other => panic!("not a floor hazard: {other:?}"),
        }
    }

    #[test]
    fn the_bot_reads_the_drakes_peck_lane() {
        use crate::game::ecs::systems::drake;
        // A tell locked straight at the rat (the rat 20 px to the east).
        let (_, en, threats) = floor_scan(EnemyKind::Drake, |e, _| {
            e.w4.phase = drake::PHASE_TELL;
            e.w4.t = 0.3;
            e.w4.dir = Vec2::new(1.0, 0.0);
        });
        assert_eq!(threats.len(), 1);
        let (a, b) = drake::peck_lane(en.pos, en.w4.dir);
        assert!(floor_cost(&threats, (a + b) * 0.5) > 0.0, "on the lane");
        assert!(floor_cost(&threats, b) > 0.0, "at the bill's reach");
        assert_eq!(
            floor_cost(&threats, en.pos + Vec2::new(0.0, 30.0)),
            0.0,
            "beside it"
        );
        match threats[0] {
            Threat::WaveHazard {
                warn_left,
                active_left,
                ..
            } => {
                assert!((warn_left - 0.3).abs() < 1e-3);
                assert!(active_left > warn_left, "the volley outlives the tell");
            }
            other => panic!("not a floor hazard: {other:?}"),
        }
        // Waddling: nothing on the floor yet.
        let (_, _, quiet) = floor_scan(EnemyKind::Drake, |e, _| {
            e.w4.phase = drake::PHASE_WADDLE;
        });
        assert!(quiet.is_empty());
    }

    #[test]
    fn the_bot_reads_the_egg_timer_ring() {
        use crate::game::ecs::systems::floor_kitchen as fk;
        let (_, en, threats) = floor_scan(EnemyKind::EggTimer, |e, _| {
            e.w4.phase = fk::EGG_PHASE_WIND;
            e.w4.t = 0.4;
        });
        assert_eq!(threats.len(), 1);
        // Anywhere the ring will pass is hot; past its last radius is not.
        assert!(floor_cost(&threats, en.pos + Vec2::new(fk::EGG_RING_END - 2.0, 0.0)) > 0.0);
        assert!(floor_cost(&threats, en.pos + Vec2::new(0.0, 10.0)) > 0.0);
        assert_eq!(floor_cost(&threats, en.pos + Vec2::new(fk::EGG_RING_END + 12.0, 0.0)), 0.0);
        // Asleep: nothing on the floor.
        let (_, _, asleep) = floor_scan(EnemyKind::EggTimer, |e, _| {
            e.w4.phase = fk::EGG_PHASE_SLEEP;
        });
        assert!(asleep.is_empty());
    }

    #[test]
    fn the_bot_reads_the_walking_syringe_ray() {
        use crate::game::ecs::systems::floor_lab as fl;
        let (_, en, threats) = floor_scan(EnemyKind::WalkingSyringe, |e, _| {
            fl::pose_for_test(e, fl::SYRINGE_TELL, Vec2::new(-1.0, 0.0));
        });
        assert_eq!(threats.len(), 1);
        let (from, to) = fl::ray_from(en.pos, en.w4.dir);
        assert!(floor_cost(&threats, (from + to) * 0.5) > 0.0, "on the ray");
        assert_eq!(floor_cost(&threats, (from + to) * 0.5 + Vec2::new(0.0, 12.0)), 0.0);
        // Retracting: the needle is spent.
        let (_, _, spent) = floor_scan(EnemyKind::WalkingSyringe, |e, _| {
            fl::pose_for_test(e, fl::SYRINGE_RETRACT, Vec2::new(-1.0, 0.0));
        });
        assert!(spent.is_empty());
    }

    #[test]
    fn the_bot_reads_the_peg_snapper_jaws() {
        use crate::game::ecs::systems::floor_laundry as fl;
        let pose = |phase: u8| {
            move |e: &mut Enemy, rat: Vec2| {
                // The `W3` mirror `jaw_view` reads: phase and seconds into it.
                e.w3.home = Vec2::new(phase as f32, 0.2);
                e.w3.bar_pos = rat;
                e.w3.bar_dir = Vec2::new(1.0, 0.0);
            }
        };
        let (p, _, threats) = floor_scan(EnemyKind::PegSnapper, pose(fl::PEG_PHASE_TELL));
        assert_eq!(threats.len(), 1);
        assert!(floor_cost(&threats, p) > 0.0, "between the jaws");
        assert!(floor_cost(&threats, p + Vec2::new(0.0, fl::PEG_JAW_GAP * 0.5)) > 0.0);
        assert_eq!(floor_cost(&threats, p + Vec2::new(0.0, 30.0)), 0.0, "well outside");
        let (_, _, shut) = floor_scan(EnemyKind::PegSnapper, pose(fl::PEG_PHASE_SHUT));
        assert!(shut.is_empty(), "shut jaws are harmless");
    }

    #[test]
    fn the_bot_reads_the_staple_crawler_feet() {
        use crate::game::ecs::systems::floor_printworks as fp;
        let (p, en, threats) = floor_scan(EnemyKind::StapleCrawler, |e, rat| {
            let feet = fp::staple_feet(e.pos, rat);
            e.w4.phase = fp::STAPLE_PHASE_TELL;
            e.w4.t = 0.3;
            e.w4.trail[0] = feet[0];
            e.w4.trail[1] = feet[1];
        });
        assert_eq!(threats.len(), 2, "one hazard per foot");
        let feet = fp::staple_feet(en.pos, p);
        assert!(floor_cost(&threats, feet[0]) > 0.0);
        assert!(floor_cost(&threats, feet[1]) > 0.0);
        // The rat's own spot, between the feet, is the safe place to stand.
        assert_eq!(floor_cost(&threats, p), 0.0);
    }

    #[test]
    fn the_bot_reads_the_loose_can_lane_and_its_bounce() {
        use crate::game::ecs::systems::floor_flatbed as fb;
        let (p, _, threats) = floor_scan(EnemyKind::LooseCan, |e, rat| {
            let path = fb::CanPath {
                points: [e.pos, rat - Vec2::new(20.0, 0.0), rat - Vec2::new(20.0, -30.0)],
                bounced: true,
            };
            e.w4.phase = fb::CAN_PHASE_ROCK;
            fb::store_can_path(e, &path);
        });
        assert_eq!(threats.len(), 4, "two legs, two lines each");
        assert!(floor_cost(&threats, p) > 0.0, "on the first leg");
        assert!(floor_cost(&threats, p + Vec2::new(-20.0, 20.0)) > 0.0, "on the bounce");
        assert_eq!(floor_cost(&threats, p + Vec2::new(0.0, -25.0)), 0.0, "off the lane");
    }
}
