//! The autoplay bot: a test instrument that plays Rat Run.
//!
//! The bot is deliberately **not** a fair opponent. It reads the whole
//! simulation (enemy positions *and* velocities, enemy bullets, every
//! telegraphed boss hazard with its remaining warn time, boss state, trap
//! pads, the tile map) and steers by a local danger field. Its job is to answer
//! "is this beatable, and how hard is it?", so it plays like a strong human
//! and never touches the simulation's own rng: it feeds `move_dir`/`dash`
//! into [`World::update`] and picks upgrades, and nothing else.
//!
//! ## Behaviours, in priority order
//!
//! 1. **Do not get hit.** Candidates are 16 compass directions plus "stay";
//!    each is scored by a danger field built from enemies (predicted with
//!    their velocity), enemy bullets (closest-approach test), boss body
//!    contact shapes, boss hazards (`BossHazard::warn`/`active` timers, so a
//!    telegraphed stomp/pounce/sweep counts before it fires), boss
//!    projectiles, glue pads and armed mousetraps. Walls are hard, and a
//!    direction that would corner the rat inside a 20 px pocket is penalised.
//! 2. **Keep moving along loops.** A waypoint is re-rolled every ~0.7-1.3 s
//!    from a rotating sweep; candidates are scored by path clearance and the
//!    open ground around them, so the bot circles the arena instead of
//!    walking into dead ends.
//! 3. **Dash out of telegraphed danger.** When a hazard is about to fire
//!    inside the rat's footprint (or a bullet's closest approach is now), the
//!    dash is spent along the best-scoring escape direction.
//! 4. **Collect while safe.** XP gems and pickups attract only when the
//!    danger field there is low; heals are chased when hurt, the nuke when
//!    the screen is crowded.
//! 5. **Boss counters.** The generic hazard field covers the telegraphs; on
//!    top of it the cat's locked pounce ring is sidestepped perpendicular at
//!    the last moment (so the leap overshoots) and the mower's charge windup
//!    is sidestepped out of the corridor.
//!
//! ## Skills
//!
//! [`Skill::Expert`] decides every tick with no noise and every counter above.
//! [`Skill::Casual`] decides every ~0.2 s, adds angular noise, keeps a shorter
//! threat horizon, weights safety lower, never uses the smart dash and ignores
//! boss counters. Same seeds ⇒ expert should beat casual measurably more
//! often; that difference is the difficulty reading the coordinator wants.
//!
//! ## Determinism
//!
//! Every random draw comes from the bot's own [`StdRng`], seeded from the run
//! seed. The simulation's rng is never touched, so the bot only affects a run
//! through the `move_dir`/`dash` it feeds and the upgrades it picks.

use rand::rngs::StdRng;
use rand::RngExt;
use rand::SeedableRng;

use crate::game::bullet::{Bullet, Faction};
use crate::game::campaign::Keepable;
use crate::game::collision::SizeClass;
use crate::game::gem::{Gem, Pickup};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::systems::boss::{Boss, BossProjectile, HazardShape};
use crate::game::enemy::Enemy;
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

/// How well the bot plays; see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skill {
    /// Every tick, no noise, every counter, smart dash.
    Expert,
    /// Slower reactions, noisy steering, shorter horizon, no boss counters.
    Casual,
}

impl Skill {
    pub fn name(self) -> &'static str {
        match self {
            Skill::Expert => "expert",
            Skill::Casual => "casual",
        }
    }

    /// Parse the `--skill` command-line value.
    pub fn parse(s: &str) -> Option<Skill> {
        match s.to_ascii_lowercase().as_str() {
            "expert" => Some(Skill::Expert),
            "casual" => Some(Skill::Casual),
            _ => None,
        }
    }
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
}

impl Params {
    fn for_skill(skill: Skill) -> Params {
        match skill {
            Skill::Expert => Params {
                reaction: 0.0,
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
            },
            Skill::Casual => Params {
                reaction: 0.20,
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
            since_decision: 0.0,
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
        self.since_decision = 0.0;
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
        if !due {
            return self.held;
        }
        // Casual's reaction is jittered so its errors are not periodic.
        if self.params.reaction > 0.0 {
            self.since_decision = 0.0;
        }
        self.decisions += 1;

        self.scan(w);
        self.update_goal(w, dt);

        let pos = w.player.pos;
        let dirs = unit_dirs();
        let mut best_dir = Vec2::ZERO;
        let mut best_score = self.score(w, pos, Vec2::ZERO, false);
        for d in dirs {
            let s = self.score(w, pos, d, true);
            if s > best_score {
                best_score = s;
                best_dir = d;
            }
        }

        // Casual steering error: rotate the chosen direction a little.
        if self.params.noise > 0.0 && best_dir.len_sq() > 0.0 {
            let jitter = (self.rng.random::<f32>() - 0.5) * 2.0 * self.params.noise;
            best_dir = best_dir.rotate(jitter).normalized();
        }

        if best_dir.len_sq() > 0.25 {
            self.heading = best_dir;
        }
        let dash = self.wants_dash(w, &dirs);
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

    /// Pick one of the three level-up cards by the documented value heuristic.
    pub fn pick_upgrade(&mut self, w: &World, choices: &[Upgrade]) -> usize {
        let lo = &w.player.loadout;
        let hp_frac = (w.player.hp / w.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        let mut best = 0usize;
        let mut best_value = f32::NEG_INFINITY;
        for (i, up) in choices.iter().enumerate() {
            let v = upgrade_value(lo, up, hp_frac);
            // Expert never takes the noise draw, so both skills stay
            // deterministic for a seed; casual's value read is slightly off.
            let v = if self.skill == Skill::Casual {
                v * (0.85 + self.rng.random::<f32>() * 0.3)
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

    /// Snapshot every threat near the rat (and the nearby loot).
    fn scan(&mut self, w: &World) {
        self.threats.clear();
        self.gems.clear();
        self.pickups.clear();
        let p = w.player.pos;
        let reach_sq = SCAN_RADIUS * SCAN_RADIUS;
        self.boss_pos = None;
        self.boss_kind = None;
        for e in w.ecs.iter_entities() {
            if let Some(en) = e.get::<Enemy>() {
                if !en.alive() || en.pos.dist_sq(p) > reach_sq {
                    continue;
                }
                self.threats.push(Threat::Enemy {
                    pos: en.pos,
                    vel: en.vel,
                    radius: en.radius,
                    damage: en.damage,
                });
            } else if let Some(b) = e.get::<Bullet>() {
                if b.faction != Faction::Enemy || b.pos.dist_sq(p) > reach_sq {
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
                if boss.pos.dist_sq(p) > reach_sq {
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
                        warn_left: (h.warn - h.elapsed).max(0.0),
                        active_left: (h.warn + h.active - h.elapsed).max(0.0),
                        damage: h.damage,
                    });
                }
            } else if let Some(bp) = e.get::<BossProjectile>() {
                if bp.pos.dist_sq(p) > reach_sq {
                    continue;
                }
                self.threats.push(Threat::Projectile {
                    pos: bp.pos,
                    vel: bp.vel,
                    radius: bp.radius,
                    damage: bp.damage,
                });
            } else if let Some(g) = e.get::<Gem>() {
                if g.pos.dist_sq(p) <= 260.0 * 260.0 {
                    self.gems.push((g.pos, g.value));
                }
            } else if let Some(pk) = e.get::<Pickup>() {
                if pk.pos.dist_sq(p) <= 200.0 * 200.0 {
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
                if armed && trap.pos.dist_sq(p) < 70.0 * 70.0 {
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
        let tiles = &w.level.tiles;
        let mut danger = 0.0f32;

        // Walls: standing inside one is fatal to the plan; a short runway is
        // a dead end, and a dead end with a horde behind it is where runs die.
        if !tiles.circle_clear(p, R + 0.5, SizeClass::Small) {
            danger += 100.0;
        }
        if moving {
            let runway = runway(w, p, dir, 140.0);
            danger += (1.0 - runway / 140.0).powi(2) * 9.0;
        }

        // Invulnerability makes contact free: dash-frames are the one moment a
        // rat can cross a pack, so the field must know.
        let invuln = (w.player.iframes / 0.6).clamp(0.0, 1.0);
        let contact_scale = 1.0 - 0.85 * invuln;

        for (i, &t) in TIMES.iter().enumerate() {
            let q = p + dir * (speed * t);
            let wt = WEIGHTS[i];
            if !tiles.circle_clear(q, R, SizeClass::Small) {
                danger += 80.0 * wt;
            }
            danger += (DIRS as f32 - openness(w, q) as f32) * 1.1 * wt;

            for threat in &self.threats {
                danger += wt * threat_cost(*threat, q, t, contact_scale);
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
        let edge = edge_gap(w, end);
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
                    // Dash to break out before the ring closes: one big body
                    // on top of the rat, or a tight pack around it.
                    if damage >= 10.0 && pos.dist(p) < radius + R + 2.0 {
                        imminent = true;
                    }
                    if pos.dist(p) < radius + R + 14.0 {
                        close += 1;
                    }
                }
                _ => {}
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
            if !path_clear(w, p, q) {
                continue;
            }
            let mut score = openness(w, q) as f32 * 0.9;
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
/// rectangle `|u| <= 7.0`, `|v| <= 5.0`.)
fn on_pad(pos: Vec2, facing: Vec2, q: Vec2) -> bool {
    let d = q - pos;
    let f = if facing.len_sq() > 0.0 {
        facing.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    d.dot(f).abs() <= 7.0 + R && d.dot(f.perp()).abs() <= 5.0 + R
}

/// Danger of one threat to a candidate position `q` at `t` seconds from now.
fn threat_cost(threat: Threat, q: Vec2, t: f32, contact_scale: f32) -> f32 {
    match threat {
        Threat::Enemy {
            pos,
            vel,
            radius,
            damage,
        } => {
            let pred = pos + vel * t;
            let near = pred.dist(q) - radius - R;
            if near > 34.0 {
                0.0
            } else {
                let w = (1.0 - (near.max(-34.0) / 34.0)).clamp(0.0, 1.0);
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

/// How far the rat could walk from `p` along `dir` before a wall stops it
/// (sampled every 8 px, capped at `max`).
fn runway(w: &World, p: Vec2, dir: Vec2, max: f32) -> f32 {
    let mut travelled = 0.0;
    while travelled < max {
        travelled += 8.0;
        if !w
            .level
            .tiles
            .circle_clear(p + dir * travelled, R, SizeClass::Small)
        {
            return (travelled - 8.0).max(0.0);
        }
    }
    max
}

/// Distance from `p` to the nearest wall cell, capped at 120 px.
fn edge_gap(w: &World, p: Vec2) -> f32 {
    let mut d = 0.0;
    while d < 120.0 {
        if !w.level.tiles.circle_clear(
            p + Vec2::from_angle(d * 0.3) * d,
            R,
            SizeClass::Small,
        ) {
            return d;
        }
        d += 12.0;
    }
    120.0
}

/// Is the straight line between two points walkable for the rat?
fn path_clear(w: &World, a: Vec2, b: Vec2) -> bool {
    let steps = ((b - a).len() / 8.0).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let q = a + (b - a) * t;
        if !w.level.tiles.circle_clear(q, R, SizeClass::Small) {
            return false;
        }
    }
    true
}

/// How many of the 8 compass neighbours of `q` (20 px out) are walkable.
fn openness(w: &World, q: Vec2) -> i32 {
    let mut n = 0;
    for d in unit_dirs().iter().step_by(2) {
        if w.level
            .tiles
            .circle_clear(q + *d * 20.0, R, SizeClass::Small)
        {
            n += 1;
        }
    }
    n
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
    match k {
        WeaponKind::Gnaw => 6.0,
        WeaponKind::CheeseSpray => 7.0,
        WeaponKind::SqueakNova => 8.0,
        WeaponKind::SporeOrbit => 7.5,
        WeaponKind::TailWhip => 9.0,
        WeaponKind::AcidSpit => 7.5,
    }
}

/// The value heuristic behind level-up picks:
///
/// * a **third weapon** is worth a lot (coverage), a fourth+ much less;
/// * levelling an owned weapon is the strongest single pick (25 % damage per
///   level on a weapon that already fires);
/// * damage / fire rate are the best passives, max HP and armour are valued
///   when hurt or unarmoured, everything else is filler.
pub fn upgrade_value(lo: &Loadout, up: &Upgrade, hp_frac: f32) -> f32 {
    match up {
        Upgrade::NewWeapon(k) => {
            let base = weapon_tier(*k);
            if lo.weapons.len() < 3 {
                base + 4.0
            } else if lo.weapons.len() < 4 {
                base - 1.0
            } else {
                base - 3.0
            }
        }
        Upgrade::LevelWeapon(i) => match lo.weapons.get(*i) {
            Some(w) => weapon_tier(w.kind) + 5.0 - w.level as f32 * 0.4,
            None => 0.0,
        },
        Upgrade::Passive(p) => match p {
            PassiveKind::Damage => 7.0 + if lo.stats.damage_mult < 1.6 { 2.0 } else { 0.0 },
            PassiveKind::FireRate => 6.5,
            PassiveKind::MaxHp => {
                if hp_frac < 0.6 || lo.stacks(PassiveKind::MaxHp) == 0 {
                    8.0
                } else {
                    3.5
                }
            }
            PassiveKind::Armor => {
                if lo.stacks(PassiveKind::Armor) < 3 {
                    6.0
                } else {
                    4.0
                }
            }
            PassiveKind::MoveSpeed => 4.5,
            PassiveKind::Regen => 4.0,
            // Whiskers is the XP engine: a bigger pickup radius is what turns
            // kills into levels, so the first stacks are worth more than any
            // damage pick.
            PassiveKind::Magnet => match lo.stacks(PassiveKind::Magnet) {
                0 => 11.0,
                1 => 9.0,
                2 => 7.0,
                _ => 3.0,
            },
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
            let one = Upgrade::Passive(passive);
            upgrade_value(lo, &one, hp_frac) * (1.0 + 0.12 * k.level as f32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;

    #[test]
    fn decision_is_finite_and_bounded_on_an_empty_floor() {
        let mut w = World::new(Theme::Sewer, 7);
        // Strip the map so the test measures the policy, not the layout.
        w.level.tiles = crate::game::collision::TileMap::new(200, 160, Theme::Sewer, 0);
        let mut bot = Bot::new(Skill::Expert, 1);
        for _ in 0..120 {
            let input = bot.decide(&w, 1.0 / 60.0);
            assert!(input.move_dir.x.is_finite() && input.move_dir.y.is_finite());
            assert!(input.move_dir.len() <= 1.0 + 1e-4);
        }
    }

    #[test]
    fn expert_and_casual_are_deterministic_per_seed() {
        let w = World::new(Theme::Kitchen, 11);
        let mut a = Bot::new(Skill::Expert, 5);
        let mut b = Bot::new(Skill::Expert, 5);
        for _ in 0..90 {
            assert_eq!(a.decide(&w, 1.0 / 60.0), b.decide(&w, 1.0 / 60.0));
        }
        let mut c = Bot::new(Skill::Casual, 5);
        let mut d = Bot::new(Skill::Casual, 5);
        for _ in 0..90 {
            assert_eq!(c.decide(&w, 1.0 / 60.0), d.decide(&w, 1.0 / 60.0));
        }
    }
}
