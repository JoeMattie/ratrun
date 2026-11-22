//! The Flatbed's own enemy rules (`WAVE4_ROSTERS.md`, "Flatbed trailer").
//!
//! | kind | build | rule here |
//! | --- | --- | --- |
//! | Gravel Tick | V(Skitterer) | drifts [`TICK_DRIFT`] px/s toward the tailgate while it chases ([`floor_flatbed_system`]) |
//! | Toll Gull | V(Spitter) | its shot aims [`GULL_LEAD_SECS`] ahead along the rat's velocity ([`shot_target`], called from `enemy_move::enemy_fire_system`) |
//! | Loose Bungee | V(Turnkey) | the crossbar's one hit also pulls the rat [`BUNGEE_PULL`] px toward the hook ([`bungee_pull`], called from `enemy_wave3::turnkey_tick`) |
//! | Loose Can | N | rocks, marks a rebounding roll, rolls it, lies still ([`CanRoll`], [`floor_flatbed_system`]) |
//! | June Bug, Tar Rat | R | none: they are the Bat and the Cat |
//!
//! ## The Loose Can
//!
//! Within [`CAN_TRIGGER_RANGE`] it rocks for [`CAN_ROCK_SECS`] with its whole
//! path outlined: [`CAN_ROLL_LENGTH`] px long and `2 ×` [`CAN_LANE_HALF_WIDTH`]
//! wide, with **one specular bounce** off the first solid face. It then rolls
//! the path at [`CAN_ROLL_SPEED`], stops at the second obstacle (or the path's
//! end, or the lip of a pit), lies still for [`CAN_REST_SECS`] and waits
//! [`CAN_COOLDOWN`] before the next roll. One hit per roll (`CanRoll::spent`).
//!
//! The path is planned once, when the rock starts ([`plan_can_path`]), and
//! stored on the body ([`store_can_path`]): the outline the hostile layer draws
//! (`art_roster_flatbed::draw_loose_can_tell`), the roll and the hit all read
//! that one [`CanPath`].
//!
//! The can **keeps the Murder Hornet as its archetype** (size class, colliders,
//! culling, loot and the steering hold all still key on it) and layers its own
//! state machine on top: `enemy_wave4::wave4_system` skips a kind that
//! [`owns_state_machine`], and this system drives it instead. Its phases are
//! numbered away from the hornet's ([`CAN_PHASE_ROCK`]…) so no hornet reader
//! (the dive-lane tell, the grounded damage bonus, the bot's lane) mistakes a
//! can for a hornet.
//!
//! ## The Wind-up Mouse and the pits
//!
//! A lured can chases and aims its roll at the key (`lure_target`, the rule
//! waves 3 and 4 share); the roll still only hurts the rat. The Loose Bungee
//! needs nothing extra: `turnkey_tick` already lays its bar across the lured
//! target, and the pull only ever moves the rat.
//!
//! Pits: the path planner ends before any hole, the roll re-checks every tick
//! and stops on the last solid plank, the walk refuses a step onto a hole, and
//! [`bungee_pull`] sweeps the rat in 1 px steps and stops before a solid or a
//! hole. A can shoved over a hole by something else falls like any ground
//! body: `systems::trailer` exempts the hornet's archetype but names the can
//! as the exception, and the stun a fall deals cancels a tell or roll in
//! progress (`can_tick`). The Flatbed's elite is the existing Brute with its plated crest:
//! nothing new here.
//!
//! Walls: the roll follows a path planned for the can's own body
//! (`circle_clear`), so it cannot pass a wall, and the can only starts a tell
//! with a clear swept line to its target (`projectiles::los_clear`).
//!
//! ## Determinism
//!
//! Bodies are walked in `SpawnId` order. The system draws rng in exactly one
//! place: `vfx::emit_player_hurt` when a roll lands its hit, the same draw the
//! hornet's jab makes. No draw happens for a drift, a tell, a path or a miss,
//! and a floor with no Gravel Tick and no Loose Can returns before the sort.
//! [`shot_target`] and [`bungee_pull`] draw nothing.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::collision::{self, SizeClass};
use crate::game::ecs::components::SpawnId;
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::Level;
use crate::game::nav::FlowField;
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

// ---- Gravel Tick -----------------------------------------------------------

/// How fast a Gravel Tick slides toward the tailgate while it chases (px/s).
pub const TICK_DRIFT: f32 = 5.0;
/// The tailgate's direction on the deck: the same `-x` the slipstream blows
/// (`systems::trailer`).
pub const TAILGATE_DIR: Vec2 = Vec2 { x: -1.0, y: 0.0 };

// ---- Toll Gull -------------------------------------------------------------

/// How far ahead along the rat's velocity a Toll Gull aims (seconds).
pub const GULL_LEAD_SECS: f32 = 0.35;
/// The gull's beak opens this long before the shot leaves (the tell pose).
pub const GULL_BEAK_OPEN_SECS: f32 = 0.35;

// ---- Loose Bungee ----------------------------------------------------------

/// How far the bungee's crossbar pulls the rat toward the hook (px).
pub const BUNGEE_PULL: f32 = 6.0;

// ---- Loose Can -------------------------------------------------------------

/// The can starts rocking when the rat is this close (px).
pub const CAN_TRIGGER_RANGE: f32 = 32.0;
/// The rock: the tell, with the whole path outlined.
pub const CAN_ROCK_SECS: f32 = 0.8;
/// Length of the marked roll, bounce included (px).
pub const CAN_ROLL_LENGTH: f32 = 48.0;
/// Half the roll lane's width (the lane is 6 px wide).
pub const CAN_LANE_HALF_WIDTH: f32 = 3.0;
/// Roll speed along the path (px/s).
pub const CAN_ROLL_SPEED: f32 = 60.0;
/// How long the can lies still after a roll.
pub const CAN_REST_SECS: f32 = 1.0;
/// Seconds between the end of one rest and the next rock.
pub const CAN_COOLDOWN: f32 = 4.0;
/// A path shorter than this is not worth a tell; the can looks again shortly.
pub const CAN_MIN_ROLL: f32 = 10.0;
/// How soon a can that found no room looks again.
pub const CAN_RETRY_SECS: f32 = 0.5;
/// Step of the path planner's march (px).
const CAN_PLAN_STEP: f32 = 1.0;

/// Trundling toward the rat.
pub const CAN_PHASE_WALK: u8 = 0;
/// Rocking: the tell.
pub const CAN_PHASE_ROCK: u8 = 10;
/// Rolling the marked path.
pub const CAN_PHASE_ROLL: u8 = 11;
/// Lying still after the roll.
pub const CAN_PHASE_REST: u8 = 12;

/// Does this kind run its state machine in this module rather than in its
/// archetype's? (`enemy_wave4::wave4_system` skips it when true.)
#[inline]
pub fn owns_state_machine(kind: EnemyKind) -> bool {
    kind == EnemyKind::LooseCan
}

/// The Loose Can's marked roll: a start, the bounce (if any) and the stop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanPath {
    /// `[start, bounce or end, end]`. Without a bounce the last two are equal.
    pub points: [Vec2; 3],
    /// Did the path reflect off a solid face?
    pub bounced: bool,
}

impl CanPath {
    /// A path with no bounce.
    pub fn straight(from: Vec2, to: Vec2) -> CanPath {
        CanPath {
            points: [from, to, to],
            bounced: false,
        }
    }

    /// How many straight legs the path has (1, or 2 with a bounce).
    #[inline]
    pub fn legs(&self) -> usize {
        if self.bounced {
            2
        } else {
            1
        }
    }

    /// Leg `i`'s two ends.
    #[inline]
    pub fn leg(&self, i: usize) -> (Vec2, Vec2) {
        let i = i.min(1);
        (self.points[i], self.points[i + 1])
    }

    /// Where the roll stops.
    #[inline]
    pub fn end(&self) -> Vec2 {
        self.points[2]
    }

    /// Total length along both legs (px).
    pub fn len(&self) -> f32 {
        self.points[0].dist(self.points[1]) + self.points[1].dist(self.points[2])
    }

    /// A path of no length (never rolled).
    pub fn is_empty(&self) -> bool {
        self.len() < 1e-3
    }

    /// The point `d` px along the path and the unit heading there.
    pub fn point_at(&self, d: f32) -> (Vec2, Vec2) {
        let first = self.points[0].dist(self.points[1]);
        let (a, b, along) = if d <= first || !self.bounced {
            (self.points[0], self.points[1], d.min(first))
        } else {
            (self.points[1], self.points[2], d - first)
        };
        let span = a.dist(b);
        if span < 1e-4 {
            return (b, Vec2::new(1.0, 0.0));
        }
        let dir = (b - a) * (1.0 / span);
        (a + dir * along.clamp(0.0, span), dir)
    }
}

/// What the planner's probe found at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Deck the can may roll over.
    Open,
    /// A solid face: the first one reflects the path, the second ends it.
    Solid,
    /// A hole in the bed: the path always ends before it.
    Pit,
}

/// Plan a roll of `total` px from `from` along unit `dir`: march in 1 px
/// steps, reflect once off the first solid face (the axis that is blocked
/// flips; a corner flips both, concave or convex, so a clipped convex corner
/// sends the can straight back), and stop at the second obstacle or before
/// any pit. A pit never counts as a reflecting face. Pure: the same inputs always give the same path, and nothing is drawn.
pub fn plan_path(probe: impl Fn(Vec2) -> Probe, from: Vec2, dir: Vec2, total: f32) -> CanPath {
    let mut pos = from;
    let mut dir = dir;
    let mut left = total;
    let mut path = CanPath::straight(from, from);
    while left > 1e-3 {
        let step = left.min(CAN_PLAN_STEP);
        let next = pos + dir * step;
        match probe(next) {
            Probe::Open => {
                pos = next;
                left -= step;
            }
            Probe::Pit => break,
            Probe::Solid => {
                if path.bounced {
                    break;
                }
                // Which face? Try each axis of the step alone.
                // Only a solid face reflects; a hole beside the impact is
                // not a wall (the march itself stops at a pit).
                let x_blocked = probe(Vec2::new(next.x, pos.y)) == Probe::Solid;
                let y_blocked = probe(Vec2::new(pos.x, next.y)) == Probe::Solid;
                dir = match (x_blocked, y_blocked) {
                    (true, false) => Vec2::new(-dir.x, dir.y),
                    (false, true) => Vec2::new(dir.x, -dir.y),
                    _ => Vec2::new(-dir.x, -dir.y),
                };
                path.bounced = true;
                path.points[1] = pos;
            }
        }
    }
    if !path.bounced {
        path.points[1] = pos;
    }
    path.points[2] = pos;
    path
}

/// [`plan_path`] against the real deck: walls from the tile grid for a body of
/// `radius`, holes from the trailer, and the arena's edge as a wall.
pub fn plan_can_path(level: &Level, from: Vec2, dir: Vec2, radius: f32, class: SizeClass) -> CanPath {
    plan_path(
        |p| {
            if level.trailer.pit_at(p).is_some() {
                Probe::Pit
            } else if p.x < radius
                || p.y < radius
                || p.x > level.arena.x - radius
                || p.y > level.arena.y - radius
                || !level.tiles.circle_clear(p, radius, class)
            {
                Probe::Solid
            } else {
                Probe::Open
            }
        },
        from,
        dir,
        CAN_ROLL_LENGTH,
    )
}

/// Mirror a path onto the body for the `&Enemy` readers (the hostile layer,
/// the bot): `w4.trail[0..3]` holds the three points. `trail_len` stays 0, so
/// no snake reader ever walks them.
pub fn store_can_path(e: &mut Enemy, path: &CanPath) {
    e.w4.trail[0] = path.points[0];
    e.w4.trail[1] = path.points[1];
    e.w4.trail[2] = path.points[2];
}

/// The path a rocking or rolling can has committed to, read back from the body
/// ([`store_can_path`]). `None` while it walks or rests.
pub fn can_path(e: &Enemy) -> Option<CanPath> {
    if e.w4.phase != CAN_PHASE_ROCK && e.w4.phase != CAN_PHASE_ROLL {
        return None;
    }
    let points = [e.w4.trail[0], e.w4.trail[1], e.w4.trail[2]];
    Some(CanPath {
        points,
        bounced: points[1].dist_sq(points[2]) > 0.01,
    })
}

/// Does a roll from `prev` to `now` (one tick of the can's lane) touch the rat?
#[inline]
pub fn can_roll_hits(prev: Vec2, now: Vec2, rat: Vec2) -> bool {
    collision::segment_circle_hit(prev, now, CAN_LANE_HALF_WIDTH, rat, PLAYER_RADIUS)
}

/// A Loose Can's own state (the authoritative copy; the phase, the tell clock
/// and the path are mirrored onto `Enemy::w4` for the renderer).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CanRoll {
    /// One of the `CAN_PHASE_*` constants.
    pub phase: u8,
    /// Seconds left in the rock or the rest.
    pub t: f32,
    /// Seconds until it may rock again.
    pub cd: f32,
    /// The committed path.
    pub path: CanPath,
    /// Distance rolled along [`CanRoll::path`] (px).
    pub traveled: f32,
    /// The roll has landed its one hit.
    pub spent: bool,
}

// ---- Toll Gull: the leading shot -------------------------------------------

/// Where a ranged body at `from` aims.
///
/// Every kind but the Toll Gull gets `ppos` back untouched (bit-for-bit the
/// old aim). The gull aims [`GULL_LEAD_SECS`] ahead along the rat's current
/// velocity; if that point is on top of the gull the plain aim is used, so the
/// shot always has a direction.
#[inline]
pub fn shot_target(kind: EnemyKind, from: Vec2, ppos: Vec2, pvel: Vec2) -> Vec2 {
    if kind != EnemyKind::TollGull {
        return ppos;
    }
    let lead = ppos + pvel * GULL_LEAD_SECS;
    if lead.dist_sq(from) < 1.0 {
        ppos
    } else {
        lead
    }
}

// ---- Loose Bungee: the pull ------------------------------------------------

/// Sweep a point from `from` toward `toward` by at most `max` px in 1 px
/// steps, stopping at the last point `open` accepts and never passing
/// `toward`. Returns where it ends up.
pub fn sweep_pull(from: Vec2, toward: Vec2, max: f32, open: impl Fn(Vec2) -> bool) -> Vec2 {
    let gap = from.dist(toward);
    if gap < 1e-3 {
        return from;
    }
    let dir = (toward - from) * (1.0 / gap);
    let reach = max.min(gap);
    let mut at = from;
    let mut done = 0.0;
    while done < reach - 1e-4 {
        let step = (reach - done).min(1.0);
        let next = at + dir * step;
        if !open(next) {
            break;
        }
        at = next;
        done += step;
    }
    at
}

/// The Loose Bungee's rule: the crossbar's hit drags the rat [`BUNGEE_PULL`]
/// px toward the hook (the body), swept against solids and stopped before a
/// pit. Any other kind: nothing. Returns how far the rat moved.
///
/// Called from `enemy_wave3::turnkey_tick` on the tick the bar lands its one
/// hit, so it happens once per bar. Draws no rng.
pub fn bungee_pull(kind: EnemyKind, hook: Vec2, level: &Level, player: &mut Player) -> f32 {
    if kind != EnemyKind::LooseBungee {
        return 0.0;
    }
    // The rat's own class (`player_system` resolves it as `Small`).
    let class = SizeClass::Small;
    let from = player.pos;
    let to = sweep_pull(from, hook, BUNGEE_PULL, |p| {
        level.trailer.pit_at(p).is_none() && level.tiles.circle_clear(p, PLAYER_RADIUS, class)
    });
    player.pos = to;
    from.dist(to)
}

// ---- Gravel Tick: the drift ------------------------------------------------

/// Where a Gravel Tick at `pos` is after `dt` of drift, or `pos` if the deck
/// there is not walkable (a wall, a crate, a hole or the edge).
#[inline]
pub fn tick_drift(level: &Level, pos: Vec2, radius: f32, class: SizeClass, dt: f32) -> Vec2 {
    let to = pos + TAILGATE_DIR * (TICK_DRIFT * dt);
    // The tick's own body, not just its centre's cell: a jamb or a door leaf
    // sits inside a "passable" cell.
    if level.trailer.pit_at(to).is_none() && level.tiles.circle_clear(to, radius, class) {
        to
    } else {
        pos
    }
}

// ---- The system ------------------------------------------------------------

/// The Flatbed's per-tick rules: the Gravel Tick's drift and the Loose Can's
/// state machine. Chain it right after `enemy_wave4::wave4_system`.
pub fn floor_flatbed_system(
    mut commands: Commands,
    input: Res<TickInput>,
    level: Res<Level>,
    flow: Res<FlowField>,
    mut player: ResMut<Player>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut sfx: ResMut<SfxQueue>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy, Option<&mut CanRoll>)>,
    glue: Query<&crate::game::ecs::systems::traps::TrapSlow>,
    ink: Query<&crate::game::ecs::systems::hazards::InkSlow>,
    decoys: Query<(&SpawnId, &crate::game::ecs::systems::powerups::Decoy)>,
    mut order: Local<Vec<(u64, Entity)>>,
) {
    let dt = input.dt;
    let level: &Level = &level;
    let flow: &FlowField = &flow;
    // Cheap gate: none of this floor's rule-carrying bodies alive, nothing to
    // do. No sort, no rng: every other floor's tick is untouched.
    if dt <= 0.0
        || !enemies
            .iter()
            .any(|(_, _, e, _)| e.kind == EnemyKind::GravelTick || e.kind == EnemyKind::LooseCan)
    {
        return;
    }
    order.clear();
    order.extend(
        enemies
            .iter()
            .filter(|(_, _, e, _)| e.kind == EnemyKind::GravelTick || e.kind == EnemyKind::LooseCan)
            .map(|(entity, id, _, _)| (id.0, entity)),
    );
    order.sort_unstable_by_key(|&(id, _)| id);
    // The live Wind-up Mouse key (highest `SpawnId` wins): the rule the
    // steering lure and waves 3 and 4 share.
    let decoy = decoys
        .iter()
        .filter(|(_, d)| d.live())
        .max_by_key(|(id, _)| id.0)
        .map(|(_, d)| d.pos);

    for &(_, entity) in order.iter() {
        let Ok((_, _, mut e, state)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        match e.kind {
            EnemyKind::GravelTick => {
                // A stunned tick is not chasing, so it does not drift.
                if e.stun <= 0.0 {
                    e.pos = tick_drift(level, e.pos, e.radius, SizeClass::of_enemy(e.kind), dt);
                }
            }
            EnemyKind::LooseCan => {
                let speed_mult =
                    glue.get(entity).map_or(1.0, |s| s.0) * ink.get(entity).map_or(1.0, |s| s.0);
                let target = lure_target(&*e, player.pos, decoy, level);
                match state {
                    Some(mut state) => can_tick(
                        &mut *e,
                        &mut *state,
                        target,
                        dt,
                        level,
                        flow,
                        speed_mult,
                        &mut *player,
                        &mut *particles,
                        &mut *rng,
                        &mut *sfx,
                    ),
                    None => {
                        // First tick: run it on a fresh state and attach that
                        // state for the ticks after (the insert is deferred).
                        let mut fresh = CanRoll {
                            cd: CAN_COOLDOWN * 0.5,
                            ..CanRoll::default()
                        };
                        can_tick(
                            &mut *e,
                            &mut fresh,
                            target,
                            dt,
                            level,
                            flow,
                            speed_mult,
                            &mut *player,
                            &mut *particles,
                            &mut *rng,
                            &mut *sfx,
                        );
                        commands.entity(entity).insert(fresh);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Whom a Loose Can chases and aims at: the Wind-up Mouse's key when it lures
/// this body (in range, in line of sight), else the rat. The same rule as
/// `enemy_wave4`'s and `floor_lab`'s `lure_target`; draws no rng.
fn lure_target(e: &Enemy, ppos: Vec2, decoy: Option<Vec2>, level: &Level) -> Vec2 {
    let Some(key) = decoy else {
        return ppos;
    };
    if !crate::game::ecs::systems::powerups::decoy_lures_body(e) {
        return ppos;
    }
    if e.pos.dist_sq(key) > crate::game::ecs::systems::powerups::WINDUP_MOUSE_RADIUS.powi(2) {
        return ppos;
    }
    if !crate::game::ecs::systems::projectiles::los_clear(level, e.pos, key, e.radius) {
        return ppos;
    }
    key
}

/// One tick of one Loose Can.
fn can_tick(
    e: &mut Enemy,
    state: &mut CanRoll,
    // What it chases and aims at: the rat, or the key that lured it. The roll
    // still only ever hurts the rat.
    target: Vec2,
    dt: f32,
    level: &Level,
    flow: &FlowField,
    speed_mult: f32,
    player: &mut Player,
    particles: &mut Particles,
    rng: &mut RngRes,
    sfx: &mut SfxQueue,
) {
    let class = SizeClass::of_enemy(e.kind);
    // A stun mid-tell or mid-roll (a fall through the deck, the Sonic Squeak)
    // ends the attack: the can lies where it was put instead of snapping back
    // onto its path next tick.
    if e.stun > 0.0 && (state.phase == CAN_PHASE_ROCK || state.phase == CAN_PHASE_ROLL) {
        state.phase = CAN_PHASE_REST;
        state.t = CAN_REST_SECS;
        e.vel = Vec2::ZERO;
    }
    match state.phase {
        CAN_PHASE_ROCK => {
            e.vel = Vec2::ZERO;
            e.w4.warn_age += dt;
            state.t -= dt;
            if state.t <= 0.0 {
                state.phase = CAN_PHASE_ROLL;
                state.traveled = 0.0;
                state.spent = false;
            }
        }
        CAN_PHASE_ROLL => {
            let prev = e.pos;
            state.traveled = (state.traveled + CAN_ROLL_SPEED * dt).min(state.path.len());
            let (at, dir) = state.path.point_at(state.traveled);
            // Belt and braces over the planned path: the can never ends a tick
            // inside a tile solid, and a block ends the roll. (Sliding cargo is
            // not in the tile grid, so this does not see it.)
            let resolved = level.tiles.resolve_circle(at, e.radius, class, None);
            // A hole that opened on the path (or a shove off it) stops the
            // roll on the last solid plank: the can never rolls over a pit.
            let over_pit = level.trailer.pit_at(resolved).is_some();
            let blocked = over_pit || resolved.dist_sq(at) > 0.25;
            e.pos = if over_pit { prev } else { resolved };
            e.vel = dir * CAN_ROLL_SPEED;
            e.facing = dir;
            e.anim += dt * 20.0;
            // One hit per roll, through the rat's i-frames like every other
            // hit: a roll that crosses an invulnerable rat stays unspent.
            if !state.spent && can_roll_hits(prev, e.pos, player.pos) && player.take_damage(e.damage) {
                state.spent = true;
                vfx::emit_player_hurt(particles, &mut rng.0, player.pos);
                sfx.0.push(Sfx::Hurt);
            }
            if blocked || state.traveled >= state.path.len() - 1e-3 {
                state.phase = CAN_PHASE_REST;
                state.t = CAN_REST_SECS;
            }
        }
        CAN_PHASE_REST => {
            e.vel = Vec2::ZERO;
            state.t -= dt;
            if state.t <= 0.0 {
                state.phase = CAN_PHASE_WALK;
                state.cd = CAN_COOLDOWN;
                state.spent = false;
            }
        }
        _ => {
            state.cd = (state.cd - dt).max(0.0);
            if e.stun > 0.0 {
                // Stunned: no step and no tell (the cooldown still runs).
                e.vel = Vec2::ZERO;
            } else {
                let lured = target.dist_sq(player.pos) > 1e-6;
                let to = target - e.pos;
                let dist = to.len();
                let direct = if dist > 1e-3 {
                    to * (1.0 / dist)
                } else {
                    e.facing
                };
                // No tell at a target behind a wall or a shut door leaf: the
                // can must see what it rolls at.
                if state.cd <= 0.0
                    && dist <= CAN_TRIGGER_RANGE
                    && crate::game::ecs::systems::projectiles::los_clear(
                        level,
                        e.pos,
                        target,
                        e.radius,
                    )
                {
                    let path = plan_can_path(level, e.pos, direct, e.radius, class);
                    if path.len() < CAN_MIN_ROLL {
                        // Nose against a wall or a hole: no room to roll.
                        state.cd = CAN_RETRY_SECS;
                    } else {
                        state.path = path;
                        state.phase = CAN_PHASE_ROCK;
                        state.t = CAN_ROCK_SECS;
                        state.spent = false;
                        e.w4.warn_age = 0.0;
                        e.w4.dir = direct;
                        e.facing = direct;
                        e.vel = Vec2::ZERO;
                        store_can_path(e, &path);
                        sfx.0.push(Sfx::Warn);
                    }
                } else {
                    // Trundle after the rat on the flow field (straight at it
                    // where the field has no answer).
                    // The field leads to the rat, so a lured can walks
                    // straight at the key instead.
                    let field = if lured { Vec2::ZERO } else { flow.dir_at(class, e.pos) };
                    let dir = if field.len_sq() > 1e-6 {
                        field.normalized()
                    } else {
                        direct
                    };
                    let speed = e.kind.speed() * e.speed_scale * e.slow_mult * speed_mult;
                    let before = e.pos;
                    e.pos += dir * (speed * dt);
                    e.vel = dir * speed;
                    e.anim += dt * 12.0;
                    e.facing += (dir - e.facing) * (8.0 * dt).min(1.0);
                    e.pos = level.tiles.resolve_circle(e.pos, e.radius, class, None);
                    e.pos = e
                        .pos
                        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
                    // A ground body: it never ends a step over a hole,
                    // whatever pushed it there (the step itself, the wall
                    // resolve or the arena clamp). Checked last for that
                    // reason.
                    if level.trailer.pit_at(e.pos).is_some() {
                        e.pos = before;
                    }
                }
            }
        }
    }
    // The renderer and the bot read the phase off the body.
    e.w4.phase = state.phase;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn run(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_flatbed_system).unwrap();
        w.end_tick();
    }

    // ---- the path planner (pure) -----------------------------------------

    #[test]
    fn an_open_deck_gives_one_straight_48_px_leg() {
        let path = plan_path(|_| Probe::Open, Vec2::new(10.0, 10.0), Vec2::new(1.0, 0.0), CAN_ROLL_LENGTH);
        assert!(!path.bounced);
        assert_eq!(path.legs(), 1);
        assert!(approx(path.len(), 48.0), "{}", path.len());
        assert!(approx(path.end().x, 58.0) && approx(path.end().y, 10.0));
    }

    #[test]
    fn the_first_solid_face_reflects_the_path_once() {
        // A wall face at x = 30: everything at or beyond it is solid.
        let probe = |p: Vec2| if p.x >= 30.0 { Probe::Solid } else { Probe::Open };
        let dir = Vec2::new(0.8, 0.6);
        let path = plan_path(probe, Vec2::new(10.0, 10.0), dir, CAN_ROLL_LENGTH);
        assert!(path.bounced);
        assert_eq!(path.legs(), 2);
        // Specular: x flips, y keeps going.
        let (a, b) = path.leg(1);
        assert!(b.x < a.x, "the rebound must leave the wall: {a:?} -> {b:?}");
        assert!(b.y > a.y, "the rebound keeps its along-wall heading");
        assert!(path.points[1].x < 30.0);
        assert!(approx(path.len(), 48.0), "a bounce costs no length: {}", path.len());
    }

    #[test]
    fn the_second_obstacle_ends_the_roll() {
        // A 14 px corridor between two faces: bounce off one, stop at the other.
        let probe = |p: Vec2| if p.x >= 24.0 || p.x <= 10.0 { Probe::Solid } else { Probe::Open };
        let path = plan_path(probe, Vec2::new(17.0, 10.0), Vec2::new(1.0, 0.0), CAN_ROLL_LENGTH);
        assert!(path.bounced);
        assert!(path.len() < 48.0, "it must stop at the second face: {}", path.len());
        assert!(path.end().x > 10.0 && path.end().x < 24.0);
    }

    #[test]
    fn a_pit_always_ends_the_path_and_never_reflects_it() {
        let probe = |p: Vec2| if p.x >= 30.0 { Probe::Pit } else { Probe::Open };
        let path = plan_path(probe, Vec2::new(10.0, 10.0), Vec2::new(1.0, 0.0), CAN_ROLL_LENGTH);
        assert!(!path.bounced);
        assert!(path.end().x < 30.0);
        assert!(approx(path.len(), 19.0) || path.len() < 20.0);
    }

    #[test]
    fn point_at_walks_both_legs_and_reports_the_heading() {
        let path = CanPath {
            points: [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
            bounced: true,
        };
        let (p, d) = path.point_at(4.0);
        assert!(approx(p.x, 4.0) && approx(p.y, 0.0) && approx(d.x, 1.0));
        let (p, d) = path.point_at(15.0);
        assert!(approx(p.x, 10.0) && approx(p.y, 5.0) && approx(d.y, 1.0));
        let (p, _) = path.point_at(99.0);
        assert!(approx(p.x, 10.0) && approx(p.y, 10.0), "clamped to the end");
    }

    #[test]
    fn the_stored_path_reads_back_only_while_warned() {
        let mut w = World::new(Theme::Trailer, 3);
        let at = w.player.pos;
        let entity = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        let mut e = w.ecs.get_mut::<Enemy>(entity).unwrap();
        let path = CanPath {
            points: [Vec2::new(1.0, 2.0), Vec2::new(9.0, 2.0), Vec2::new(9.0, 8.0)],
            bounced: true,
        };
        store_can_path(&mut e, &path);
        assert!(can_path(&e).is_none(), "a walking can has no footprint");
        e.w4.phase = CAN_PHASE_ROCK;
        assert_eq!(can_path(&e), Some(path));
        assert_eq!(e.w4.trail_len, 0, "no snake reader may walk a can's path");
    }

    // ---- the gull's aim (pure) --------------------------------------------

    #[test]
    fn only_the_toll_gull_leads_its_shot() {
        let from = Vec2::new(0.0, 0.0);
        let ppos = Vec2::new(100.0, 0.0);
        let pvel = Vec2::new(0.0, 80.0);
        assert_eq!(shot_target(EnemyKind::Spitter, from, ppos, pvel), ppos);
        assert_eq!(shot_target(EnemyKind::PaperWasp, from, ppos, pvel), ppos);
        let lead = shot_target(EnemyKind::TollGull, from, ppos, pvel);
        assert!(approx(lead.x, 100.0) && approx(lead.y, 80.0 * GULL_LEAD_SECS));
        // A standing rat is aimed at directly.
        assert_eq!(shot_target(EnemyKind::TollGull, from, ppos, Vec2::ZERO), ppos);
        // A lead point on top of the gull falls back to the plain aim.
        let onto = shot_target(EnemyKind::TollGull, from, ppos, Vec2::new(-100.0 / GULL_LEAD_SECS, 0.0));
        assert_eq!(onto, ppos);
    }

    // ---- the bungee's pull -------------------------------------------------

    #[test]
    fn the_pull_is_six_px_toward_the_hook_and_never_past_it() {
        let to = sweep_pull(Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), BUNGEE_PULL, |_| true);
        assert!(approx(to.x, 6.0) && approx(to.y, 0.0));
        let near = sweep_pull(Vec2::new(0.0, 0.0), Vec2::new(2.5, 0.0), BUNGEE_PULL, |_| true);
        assert!(approx(near.x, 2.5), "the rat stops at the hook: {near:?}");
        let same = sweep_pull(Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0), BUNGEE_PULL, |_| true);
        assert_eq!(same, Vec2::new(3.0, 3.0));
    }

    #[test]
    fn the_pull_stops_at_a_solid_or_a_pit() {
        let to = sweep_pull(Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), BUNGEE_PULL, |p| p.x < 3.5);
        assert!(approx(to.x, 3.0), "swept, not teleported: {to:?}");
        let held = sweep_pull(Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), BUNGEE_PULL, |_| false);
        assert_eq!(held, Vec2::new(0.0, 0.0));
    }

    #[test]
    fn only_the_loose_bungee_pulls() {
        let mut w = World::new(Theme::Trailer, 4);
        let start = w.player.pos;
        let hook = start + Vec2::new(30.0, 0.0);
        assert_eq!(bungee_pull(EnemyKind::Turnkey, hook, &w.level, &mut w.player), 0.0);
        assert_eq!(w.player.pos, start, "the Turnkey's bar is unchanged");
        let moved = bungee_pull(EnemyKind::LooseBungee, hook, &w.level, &mut w.player);
        assert!(moved <= BUNGEE_PULL + 1e-3);
        assert!(w.player.pos.dist(start) <= BUNGEE_PULL + 1e-3);
        assert!(w.player.pos.x >= start.x, "pulled toward the hook, never away");
        assert!(w.level.trailer.pit_at(w.player.pos).is_none(), "never into a hole");
    }

    // ---- the system --------------------------------------------------------

    #[test]
    fn a_gravel_tick_drifts_toward_the_tailgate_and_a_skitterer_does_not() {
        let mut w = World::new(Theme::Trailer, 5);
        let at = w.player.pos + Vec2::new(40.0, 0.0);
        let tick = w.spawn_enemy(EnemyKind::GravelTick, at, 1.0);
        let skit = w.spawn_enemy(EnemyKind::Skitterer, at, 1.0);
        for _ in 0..60 {
            run(&mut w);
        }
        let t = w.ecs.get::<Enemy>(tick).unwrap().pos;
        let s = w.ecs.get::<Enemy>(skit).unwrap().pos;
        assert_eq!(s, at, "the Skitterer is untouched by this system");
        assert!(approx(t.y, at.y));
        assert!(
            t.x < at.x && at.x - t.x <= TICK_DRIFT + 1e-2,
            "one second of drift is at most {TICK_DRIFT} px tailward: {at:?} -> {t:?}"
        );
    }

    #[test]
    fn a_stunned_gravel_tick_does_not_drift() {
        let mut w = World::new(Theme::Trailer, 5);
        let at = w.player.pos + Vec2::new(40.0, 0.0);
        let tick = w.spawn_enemy(EnemyKind::GravelTick, at, 1.0);
        w.ecs.get_mut::<Enemy>(tick).unwrap().stun = 5.0;
        for _ in 0..30 {
            run(&mut w);
        }
        assert_eq!(w.ecs.get::<Enemy>(tick).unwrap().pos, at);
    }

    #[test]
    fn no_flatbed_body_means_no_rng_and_no_state() {
        let mut w = World::new(Theme::Sewer, 9);
        let at = w.player.pos + Vec2::new(40.0, 0.0);
        let e = w.spawn_enemy(EnemyKind::MurderHornet, at, 1.0);
        // The twin world takes the same spawn and never runs the system.
        let mut twin = World::new(Theme::Sewer, 9);
        twin.spawn_enemy(EnemyKind::MurderHornet, at, 1.0);
        for _ in 0..20 {
            run(&mut w);
        }
        use rand::RngExt;
        assert_eq!(w.rng.random::<u64>(), twin.rng.random::<u64>(), "the system drew rng");
        assert!(w.ecs.get::<CanRoll>(e).is_none(), "a hornet is not a can");
        assert_eq!(w.ecs.get::<Enemy>(e).unwrap().pos, at);
    }

    /// Drive a can next to the rat through one whole beat and record the
    /// phases it passes through.
    #[test]
    fn a_can_rocks_rolls_rests_and_cools_down() {
        let mut w = World::new(Theme::Trailer, 6);
        w.player.iframes = 0.0;
        let at = w.player.pos + Vec2::new(20.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        // Skip the opening half cooldown.
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).expect("state attached on the first tick").cd = 0.0;
        let hp0 = w.player.hp;

        run(&mut w);
        let e = w.ecs.get::<Enemy>(can).unwrap().clone();
        assert_eq!(e.w4.phase, CAN_PHASE_ROCK, "within 32 px it rocks");
        let path = can_path(&e).expect("the footprint exists for the whole tell");
        assert!(path.len() >= CAN_MIN_ROLL && path.len() <= CAN_ROLL_LENGTH + 1e-2);
        let rock_pos = e.pos;

        // The rock holds still and deals nothing for 0.8 s.
        let rock_ticks = (CAN_ROCK_SECS / DT) as usize - 2;
        for _ in 0..rock_ticks {
            run(&mut w);
        }
        let e = w.ecs.get::<Enemy>(can).unwrap().clone();
        assert_eq!(e.w4.phase, CAN_PHASE_ROCK);
        assert_eq!(e.pos, rock_pos, "a rocking can does not travel");
        assert_eq!(w.player.hp, hp0, "the tell never hurts");

        // The roll: along the marked path, one hit at most.
        let mut rolled = false;
        let mut rested = false;
        for _ in 0..((CAN_ROLL_LENGTH / CAN_ROLL_SPEED + CAN_REST_SECS + 0.5) / DT) as usize {
            run(&mut w);
            let e = w.ecs.get::<Enemy>(can).unwrap();
            match e.w4.phase {
                CAN_PHASE_ROLL => rolled = true,
                CAN_PHASE_REST => rested = true,
                _ => {}
            }
        }
        assert!(rolled && rested, "rolled={rolled} rested={rested}");
        let e = w.ecs.get::<Enemy>(can).unwrap().clone();
        assert_eq!(e.w4.phase, CAN_PHASE_WALK, "back to walking after the rest");
        assert!(can_path(&e).is_none());
        let state = *w.ecs.get::<CanRoll>(can).unwrap();
        assert!(state.cd > CAN_COOLDOWN - 1.0, "the 4 s cooldown is running: {}", state.cd);
        let lost = hp0 - w.player.hp;
        assert!(
            lost <= e.damage + 1e-3,
            "one hit per roll at most: lost {lost}, a hit is {}",
            e.damage
        );
        assert!(
            e.pos.dist(path.end()) < 4.0,
            "the can stops where the outline ended: {:?} vs {:?}",
            e.pos,
            path.end()
        );
    }

    #[test]
    fn a_far_can_never_starts_a_tell() {
        let mut w = World::new(Theme::Trailer, 6);
        let at = w.player.pos + Vec2::new(CAN_TRIGGER_RANGE + 60.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).unwrap().cd = 0.0;
        run(&mut w);
        assert_eq!(w.ecs.get::<Enemy>(can).unwrap().w4.phase, CAN_PHASE_WALK);
    }

    #[test]
    fn the_roll_hit_is_the_lane_the_outline_draws() {
        let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0));
        let reach = CAN_LANE_HALF_WIDTH + PLAYER_RADIUS;
        assert!(can_roll_hits(a, b, Vec2::new(5.0, reach - 0.2)));
        assert!(!can_roll_hits(a, b, Vec2::new(5.0, reach + 0.6)));
        assert!(owns_state_machine(EnemyKind::LooseCan));
        assert!(!owns_state_machine(EnemyKind::MurderHornet));
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    fn run(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(floor_flatbed_system).unwrap();
        w.end_tick();
    }

    #[test]
    fn a_lured_can_aims_its_roll_at_the_key() {
        let mut w = World::new(Theme::Trailer, 6);
        let rat = w.player.pos;
        // The can sits between the rat (20 px left) and the key (20 px right).
        let at = rat + Vec2::new(20.0, 0.0);
        let key = at + Vec2::new(20.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        crate::game::ecs::systems::powerups::spawn_decoy(&mut w, key);
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).unwrap().cd = 0.0;
        run(&mut w);
        let e = w.ecs.get::<Enemy>(can).unwrap().clone();
        if e.w4.phase != CAN_PHASE_ROCK {
            // Cargo or a hole left no room on this seed: nothing to assert.
            return;
        }
        if crate::game::ecs::systems::powerups::decoy_lures_body(&e) {
            assert!(e.w4.dir.x > 0.0, "a lured can rolls at the key: {:?}", e.w4.dir);
        } else {
            assert!(e.w4.dir.x < 0.0, "an unlured can rolls at the rat: {:?}", e.w4.dir);
        }
    }

    #[test]
    fn a_roll_never_ends_a_tick_over_a_pit() {
        let mut w = World::new(Theme::Trailer, 6);
        let at = w.player.pos + Vec2::new(20.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).unwrap().cd = 0.0;
        for _ in 0..240 {
            run(&mut w);
            let e = w.ecs.get::<Enemy>(can).unwrap();
            assert!(
                w.level.trailer.pit_at(e.pos).is_none(),
                "the can is over a hole at {:?}",
                e.pos
            );
        }
    }

    #[test]
    fn the_planned_path_stays_off_every_pit_on_the_real_deck() {
        let w = World::new(Theme::Trailer, 6);
        let from = w.player.pos;
        for k in 0..16 {
            let dir = Vec2::from_angle(k as f32 / 16.0 * std::f32::consts::TAU);
            let path = plan_can_path(&w.level, from, dir, 2.6, SizeClass::Small);
            assert!(path.len() <= CAN_ROLL_LENGTH + 1e-2);
            for d in 0..=(path.len() as usize) {
                let (p, _) = path.point_at(d as f32);
                assert!(w.level.trailer.pit_at(p).is_none(), "path crosses a hole at {p:?}");
            }
        }
    }
}

#[cfg(test)]
mod pit_tests {
    use super::*;
    use crate::game::level::{Pit, PitKind, Theme, TileRect};
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    /// A flat 20 × 8 bed with no holes, wind or cargo (the trailer tests' deck).
    fn bed_world() -> World {
        let mut w = World::new(Theme::Trailer, 3);
        let deck = w.level.trailer.deck;
        w.level
            .tiles
            .fill_cells(deck.x, deck.y, deck.w, deck.h, crate::game::collision::TileKind::Open);
        w.level.trailer.pits.clear();
        w.level.trailer.wind.clear();
        w.level.trailer.slides.clear();
        w
    }

    /// The can is a ground body: put over a hole it falls like a Skitterer,
    /// while the Murder Hornet it borrows its archetype from still flies.
    #[test]
    fn a_can_over_a_hole_falls_and_a_hornet_does_not() {
        let mut w = bed_world();
        let deck = w.level.trailer.deck;
        let hole = TileRect::new(deck.x + 8, deck.y + 4, 1, 1);
        w.level.trailer.pits.push(Pit::new(hole, PitKind::BrokenPlank));
        let at = hole.center_world();
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        let hornet = w.spawn_enemy(EnemyKind::MurderHornet, at, 1.0);
        let can_hp = w.ecs.get::<Enemy>(can).unwrap().hp;
        let hornet_hp = w.ecs.get::<Enemy>(hornet).unwrap().hp;

        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(crate::game::ecs::systems::trailer::trailer_system)
            .unwrap();
        w.end_tick();

        let c = w.ecs.get::<Enemy>(can).expect("the can survives one fall");
        assert!(c.hp < can_hp, "the fall did not hurt the can");
        assert!(c.stun > 0.0, "the fallen can was not stunned");
        assert!(w.level.trailer.pit_at(c.pos).is_none(), "the can was left in the hole");
        let h = w.ecs.get::<Enemy>(hornet).unwrap();
        assert_eq!(h.hp, hornet_hp, "the hornet still flies over holes");
        assert_eq!(h.pos, at);
    }

    /// The stun a fall deals ends a roll in progress, so the can stays on the
    /// plank it was put back on.
    #[test]
    fn a_stun_cancels_a_tell_or_a_roll() {
        let mut w = bed_world();
        let at = w.level.trailer.deck.center_world();
        w.player.pos = at + Vec2::new(-20.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        let run = |w: &mut World| {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(floor_flatbed_system).unwrap();
            w.end_tick();
        };
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).unwrap().cd = 0.0;
        run(&mut w);
        assert_eq!(w.ecs.get::<Enemy>(can).unwrap().w4.phase, CAN_PHASE_ROCK);
        w.ecs.get_mut::<Enemy>(can).unwrap().stun = 0.6;
        let held = w.ecs.get::<Enemy>(can).unwrap().pos;
        run(&mut w);
        let e = w.ecs.get::<Enemy>(can).unwrap();
        assert_eq!(e.w4.phase, CAN_PHASE_REST, "a stunned can drops its attack");
        assert_eq!(e.pos, held);
        assert!(can_path(e).is_none(), "and its footprint goes with it");
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::game::level::Theme;
    use crate::game::world::World;

    const DT: f32 = 1.0 / 60.0;

    /// G7: a hole beside the impact point is not a second face. The can hits a
    /// vertical wall at x = 30 moving down-right; just below the impact row is
    /// a pit. The bounce must flip x only, not reverse the path.
    #[test]
    fn a_pit_beside_the_impact_does_not_turn_a_bounce_into_a_reversal() {
        let probe = |p: Vec2| {
            if p.x >= 30.0 {
                Probe::Solid
            } else if p.y >= 40.0 {
                Probe::Pit
            } else {
                Probe::Open
            }
        };
        // Start so the wall is met right at the pit's lip (y just under 40).
        let dir = Vec2::new(0.8, 0.6);
        let from = Vec2::new(30.0 - 0.8 * 10.0, 39.6 - 0.6 * 10.0);
        let path = plan_path(probe, from, dir, CAN_ROLL_LENGTH);
        assert!(path.bounced, "the wall reflects the path");
        let (a, b) = path.leg(1);
        if a.dist(b) > 0.5 {
            assert!(b.x < a.x, "x flips off the vertical face");
            assert!(b.y >= a.y, "y does not reverse: the pit is not a wall ({a:?} -> {b:?})");
        }
        // And the path still ends before the hole.
        assert!(path.end().y < 40.0);
    }

    /// A concave corner still flips both axes.
    #[test]
    fn a_concave_corner_sends_the_can_straight_back() {
        let probe = |p: Vec2| {
            if p.x >= 30.0 || p.y >= 30.0 {
                Probe::Solid
            } else {
                Probe::Open
            }
        };
        let d = std::f32::consts::FRAC_1_SQRT_2;
        let path = plan_path(probe, Vec2::new(20.0, 20.0), Vec2::new(d, d), CAN_ROLL_LENGTH);
        assert!(path.bounced);
        let (a, b) = path.leg(1);
        assert!(b.x < a.x && b.y < a.y, "{a:?} -> {b:?}");
    }

    /// G8: the drift keeps the tick's whole body clear, and never over a hole.
    #[test]
    fn the_drift_never_puts_the_ticks_body_in_a_wall_or_a_hole() {
        let w = World::new(Theme::Trailer, 5);
        let radius = EnemyKind::GravelTick.radius();
        let class = SizeClass::of_enemy(EnemyKind::GravelTick);
        let mut pos = w.player.pos;
        for _ in 0..(60 * 120) {
            let to = tick_drift(&w.level, pos, radius, class, DT);
            if to != pos {
                assert!(w.level.tiles.circle_clear(to, radius, class), "body in a wall at {to:?}");
                assert!(w.level.trailer.pit_at(to).is_none(), "over a hole at {to:?}");
                assert!(to.x < pos.x, "the drift is tailward");
            }
            pos = to;
        }
    }

    /// G6: across a long walk on the real deck the can never ends a tick over a
    /// hole, including the ticks where the wall resolve moved it.
    #[test]
    fn a_walking_can_never_ends_a_tick_over_a_hole() {
        let mut w = World::new(Theme::Trailer, 11);
        let at = w.player.pos + Vec2::new(90.0, 30.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        let start_clear = w.level.trailer.pit_at(at).is_none();
        for _ in 0..600 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(floor_flatbed_system).unwrap();
            w.end_tick();
            let e = w.ecs.get::<Enemy>(can).unwrap();
            if start_clear {
                assert!(w.level.trailer.pit_at(e.pos).is_none(), "over a hole at {:?}", e.pos);
            }
        }
    }

    /// The can does not start a tell at a rat it cannot see.
    #[test]
    fn no_tell_without_a_clear_line_to_the_target() {
        let mut w = World::new(Theme::Trailer, 6);
        let at = w.player.pos + Vec2::new(20.0, 0.0);
        let can = w.spawn_enemy(EnemyKind::LooseCan, at, 1.0);
        let run = |w: &mut World| {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(floor_flatbed_system).unwrap();
            w.end_tick();
        };
        run(&mut w);
        w.ecs.get_mut::<CanRoll>(can).unwrap().cd = 0.0;
        run(&mut w);
        let e = w.ecs.get::<Enemy>(can).unwrap().clone();
        let seen = crate::game::ecs::systems::projectiles::los_clear(
            &w.level,
            at,
            w.player.pos,
            e.radius,
        );
        if !seen {
            assert_eq!(e.w4.phase, CAN_PHASE_WALK, "a blind can must not tell");
        }
    }
}
