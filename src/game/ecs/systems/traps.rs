//! (h) Floor traps: sticky glue boards and baited mousetraps.
//!
//! The traps are generated into [`LevelObjects`](crate::game::level::LevelObjects)
//! (`level.objects.sticky_traps` / `.mousetraps`) and drawn by `art_traps.rs`
//! from those very view structs. Those view structs carry the *gameplay* state
//! (`state` / `progress` / `victim_pos` / `victim_radius` / `hit_flash`), so
//! this system is the only writer of them: the renderer just overwrites `anim`
//! with the run clock. Everything the art's per-state doc comments promise
//! (sizes, timings, hit windows) is read from `art_traps`' own constants, so
//! the drawn glue pool and the simulated pad cannot drift apart.
//!
//! ```
//! StickyTrapState:  Idle --(victim steps on the pad)--> Stuck --(hold spent,
//!                   victim freed, or the rat dashes out)--> Used
//! MousetrapState:   Armed --(any ground entity in the trigger circle)-->
//!                   Triggered (0.28 s) --> Snapping (0.20 s) --> Sprung
//! ```
//!
//! ## Ordering
//!
//! `traps_system` runs **first in the tick**, before `player::player_system`:
//!
//! - the rat's glue slow is applied by shortening this tick's
//!   [`TickInput`] throttle vector *and* its current velocity, so the
//!   unmodified `player_system` derives 20 % of its normal speed from it (see
//!   [`apply_player_slow`]); nothing in that system had to change;
//! - a glued ground enemy carries the [`TrapSlow`] component, which
//!   `enemy_move::enemy_steering_system` multiplies into the per-enemy speed
//!   (the one-line hook in that system);
//! - damage dealt here lands before `loot::reap_enemies_system` (schedule
//!   point 17), so a trap kill drops its gem, scores, and queues its `Sfx`
//!   exactly like a weapon kill — the normal loot path.
//!
//! Running first also means every position read is the entity's position at
//! the *start* of the tick, which is what makes the trigger tests
//! frame-order independent.
//!
//! ## Determinism
//!
//! Nothing here iterates an unordered query to decide something: the enemy
//! scans pick the **lowest [`SpawnId`]** candidate among the overlapping ones
//! (so the query's storage order cannot choose a victim), and a mousetrap's
//! multi-hit window collects its targets into a reused `Local` buffer and
//! sorts *those* by `SpawnId` before applying damage, so the rng draw order of
//! the impact vfx is the same as everywhere else in the sim.
//!
//! Rng is drawn **only** where a trap crosses a state edge and the matching
//! [`vfx`] burst fires — the four glue edges (grab / tear-free, for the rat and
//! for the horde), the mousetrap's snap beat and the players' hurt / hit bursts
//! inside `snap_window`. Those draws live at fixed points in a fixed walk
//! (`live_sticky` then `live_mice`, both ascending indices), so twin runs draw
//! the same numbers in the same order.
//!
//! ## Sound + effects (one variant per edge, never a stand-in)
//!
//! | edge | `Sfx` | vfx |
//! |---|---|---|
//! | a victim sticks to a board | `TrapGlueGrab` | `emit_glue_stick` |
//! | the glue releases (hold spent / dash) | `TrapGlueRelease` | `emit_glue_tear_free` |
//! | the bail pedal is touched | `TrapTrigger` | — (the art dips the pedal) |
//! | the striker bar lands | `TrapSnap` | `emit_mousetrap_snap` |
//!
//! ## No per-tick allocation
//!
//! The scratch (`hits`, `fresh`, `spent_*`) are `Local` buffers reused for the
//! whole run; the runtime side table ([`TrapRuntime`]) resizes only when the
//! level's object lists change, and every loop is a plain index walk.
//!
//! ## Designer overrides (review "Boss and trap direction")
//!
//! - the rat is held `0.9 s` ([`STICKY_HOLD_PLAYER_SECS`]) instead of the art's
//!   1.6 s;
//! - ground Skitterers / Cats / Spitters `2.5 s`, Brutes `1.5 s`, all at the
//!   art's `20 %` ([`STICKY_TRAP_SLOW_MULT`]); Bats fly over;
//! - bosses keep the art's `0.8 s` at `40 %` ([`STICKY_HOLD_BOSS_SECS`]);
//! - the mousetrap keeps the art's `0.28 s` bait warning, `0.20 s` snap,
//!   `20` damage to the rat and `80` to ground enemies.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_traps::{
    local_box_contains, MousetrapState, MousetrapView, StickyTrapState, MOUSETRAP_DAMAGE_ENEMY,
    MOUSETRAP_DAMAGE_PLAYER, MOUSETRAP_HIT_PROGRESS_MAX, MOUSETRAP_HIT_PROGRESS_MIN,
    MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_U_MIN, MOUSETRAP_SNAP_SECS,
    MOUSETRAP_TRIGGER_OFFSET_U, MOUSETRAP_TRIGGER_RADIUS, MOUSETRAP_TRIGGER_SECS,
    STICKY_TRAP_BLOCKS_MOVEMENT, STICKY_TRAP_HOLD_BRUTE_SECS, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS,
    STICKY_TRAP_PAD_HALF, STICKY_TRAP_SLOW_MULT,
};
use crate::game::ecs::components::{Entity, SpawnId};
use crate::game::ecs::constants::PLAYER_RADIUS;
use crate::game::ecs::resources::{RngRes, SfxQueue, TickInput};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{Level, Theme};
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;

// Runtime checks: the art's pads are walkable hazards, and the two traps this
// file drives are the ones whose states it writes.
const _: () = assert!(!STICKY_TRAP_BLOCKS_MOVEMENT);

/// How long the rat is held by a glue board.
///
/// The art's `STICKY_TRAP_HOLD_PLAYER_SECS` is 1.6 s; the design review kept
/// the shorter 0.9 s "with dash on cooldown" as the tested value.
pub const STICKY_HOLD_PLAYER_SECS: f32 = 0.9;
/// How long a boss is held (art `StickyTrapState::Stuck`).
pub const STICKY_HOLD_BOSS_SECS: f32 = 0.8;
/// Boss movement multiplier while glued (art `StickyTrapState::Stuck`).
pub const STICKY_SLOW_BOSS: f32 = 0.4;

/// Squared "could this point still be on the pad?" prefilter radius.
///
/// The pad's half-diagonal is `sqrt(7² + 5²) ≈ 8.6 px`; anything farther from
/// the board centre than this cannot be inside the oriented rectangle, so the
/// cheap distance test rejects almost every enemy before the exact one.
const PAD_REACH_SQ: f32 = 81.0;

/// Per-enemy movement multiplier applied by a trap (absent = `1.0`).
///
/// Only glued ground enemies carry this component. It is inserted when a
/// sticky board catches the enemy and removed when the glue releases, and
/// `enemy_move::enemy_steering_system` multiplies it into `speed_mult`, which
/// is what gives the art's exact `20 %` (or a boss's `40 %`) regardless of
/// frame rate.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct TrapSlow(pub f32);

/// Who a glue board is currently holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Victim {
    Player,
    Enemy(Entity),
}

/// Runtime side table for one sticky board.
#[derive(Clone, Copy, Debug, Default)]
struct StickySlot {
    victim: Option<Victim>,
    /// Seconds of hold left.
    remaining: f32,
    /// Total hold this victim gets, for the art's `progress`.
    hold: f32,
}

/// Runtime side table for one mousetrap.
#[derive(Clone, Debug, Default)]
struct MouseSlot {
    /// Ground enemies already damaged by this snap (each target at most once).
    struck: Vec<Entity>,
    /// The rat was already damaged by this snap.
    player_struck: bool,
}

/// ECS side table for the floor traps, indexed exactly like `LevelObjects`.
///
/// The *visible* state lives in the art view structs (that is what the
/// renderer reads); this only keeps what the views cannot hold: who is glued
/// and for how long, and which targets a snap has already hit.
#[derive(Resource, Default)]
pub struct TrapRuntime {
    /// Level identity the lists were built for (`seed`, `theme`).
    seed: u64,
    theme: Option<Theme>,
    sticky: Vec<StickySlot>,
    mice: Vec<MouseSlot>,
    /// Ascending indices of the boards that can still do something.
    live_sticky: Vec<usize>,
    /// Ascending indices of the traps that can still do something.
    live_mice: Vec<usize>,
}

impl TrapRuntime {
    /// Rebuild the side table if the level (or its object lists) changed.
    ///
    /// Runs every tick but returns immediately in the steady state: the lists
    /// only change on a new level, or when a test edits `level.objects`
    /// directly (the narrow harness does exactly that).
    fn sync(&mut self, level: &Level) {
        let objects = &level.objects;
        if self.seed == level.seed
            && self.theme == Some(level.theme)
            && self.sticky.len() == objects.sticky_traps.len()
            && self.mice.len() == objects.mousetraps.len()
        {
            return;
        }
        self.seed = level.seed;
        self.theme = Some(level.theme);
        self.sticky.clear();
        self.sticky
            .resize_with(objects.sticky_traps.len(), StickySlot::default);
        self.live_sticky.clear();
        self.live_sticky.extend(
            objects
                .sticky_traps
                .iter()
                .enumerate()
                .filter(|(_, v)| is_live_sticky(v.state))
                .map(|(i, _)| i),
        );
        self.mice.clear();
        self.mice
            .resize_with(objects.mousetraps.len(), MouseSlot::default);
        self.live_mice.clear();
        self.live_mice.extend(
            objects
                .mousetraps
                .iter()
                .enumerate()
                .filter(|(_, v)| is_live_mouse(v.state))
                .map(|(i, _)| i),
        );
    }
}

/// A board can still catch a victim or is holding one.
#[inline]
fn is_live_sticky(state: StickyTrapState) -> bool {
    matches!(state, StickyTrapState::Idle | StickyTrapState::Stuck)
}

/// A mousetrap can still be triggered, or is in the middle of a snap.
#[inline]
fn is_live_mouse(state: MousetrapState) -> bool {
    matches!(
        state,
        MousetrapState::Armed | MousetrapState::Triggered | MousetrapState::Snapping
    )
}

/// Ground enemies can be glued and can trip a mousetrap; Bats fly over both.
#[inline]
fn is_ground(kind: EnemyKind) -> bool {
    !matches!(kind, EnemyKind::Bat)
}

/// `(hold seconds, movement multiplier)` for a ground enemy caught by glue.
#[inline]
fn enemy_hold(kind: EnemyKind) -> (f32, f32) {
    match kind {
        // Art `Stuck`: bosses take the shorter 0.8 s at a milder 40 % slow.
        EnemyKind::Boss => (STICKY_HOLD_BOSS_SECS, STICKY_SLOW_BOSS),
        // Art `Stuck`: Brutes are heavy, so they tear free sooner.
        EnemyKind::Brute => (STICKY_TRAP_HOLD_BRUTE_SECS, STICKY_TRAP_SLOW_MULT),
        // Skitterer, Cat, Spitter (and anything else ground-bound).
        _ => (STICKY_TRAP_HOLD_SMALL_ENEMY_SECS, STICKY_TRAP_SLOW_MULT),
    }
}

/// Is `p` on the glue pad of the board at `pos`/`facing`? (Art `Idle`:
/// oriented rectangle `|u| <= 7.0`, `|v| <= 5.0`.)
#[inline]
fn on_pad(pos: Vec2, facing: Vec2, p: Vec2) -> bool {
    local_box_contains(
        pos,
        facing,
        p,
        -STICKY_TRAP_PAD_HALF.x,
        STICKY_TRAP_PAD_HALF.x,
        STICKY_TRAP_PAD_HALF.y,
    )
}

/// Is `p` inside the bait plate's trigger circle? (Art `Armed`: radius `4.5`
/// around `pos + facing * 3.5`.)
#[inline]
fn on_bait(pos: Vec2, facing: Vec2, p: Vec2) -> bool {
    let centre = pos + facing.normalized() * MOUSETRAP_TRIGGER_OFFSET_U;
    p.dist_sq(centre) <= MOUSETRAP_TRIGGER_RADIUS * MOUSETRAP_TRIGGER_RADIUS
}

/// Is `p` inside the striker bar's kill rectangle? (Art `Snapping`:
/// `u in [-1.0, 9.5]`, `|v| <= 6.0`.)
#[inline]
fn in_kill_rect(pos: Vec2, facing: Vec2, p: Vec2) -> bool {
    local_box_contains(
        pos,
        facing,
        p,
        MOUSETRAP_KILL_U_MIN,
        MOUSETRAP_KILL_U_MAX,
        MOUSETRAP_KILL_HALF_V,
    )
}

/// The lowest-`SpawnId` ground enemy whose centre is on a board's pad.
///
/// Choosing the minimum id instead of the first hit keeps the query's storage
/// order out of the decision (determinism rule), without sorting the whole
/// horde once per board.
fn pick_enemy_on_pad(
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    slow: &Query<&TrapSlow>,
    pos: Vec2,
    facing: Vec2,
) -> Option<(u64, Entity)> {
    let mut best: Option<(u64, Entity)> = None;
    for (entity, id, e) in enemies.iter() {
        if !e.alive() || !is_ground(e.kind) || slow.contains(entity) {
            continue;
        }
        // Cheap circle reject first (the pad's half-diagonal), then the exact
        // oriented rectangle the art draws.
        if e.pos.dist_sq(pos) > PAD_REACH_SQ {
            continue;
        }
        if !on_pad(pos, facing, e.pos) {
            continue;
        }
        if best.is_none_or(|(best_id, _)| id.0 < best_id) {
            best = Some((id.0, entity));
        }
    }
    best
}

/// Apply one mousetrap snap's active window: the rat (through i-frames) and
/// every ground enemy under the bar, each target at most once.
///
/// Enemy targets are damaged in `SpawnId` order because the impact vfx draws
/// from the rng, so the draw order is part of the spec.
fn snap_window(
    pos: Vec2,
    facing: Vec2,
    player: &mut Player,
    player_alive: bool,
    player_struck: &mut bool,
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    struck: &mut Vec<Entity>,
    hits: &mut Vec<(u64, Entity)>,
    rng: &mut rand::rngs::StdRng,
    particles: &mut Particles,
    damage_numbers: &mut DamageNumbers,
    sfx: &mut Vec<Sfx>,
) {
    if player_alive && !*player_struck {
        let p = player.pos;
        if in_kill_rect(pos, facing, p) && player.take_damage(MOUSETRAP_DAMAGE_PLAYER) {
            *player_struck = true;
            vfx::emit_player_hurt(particles, rng, p);
            sfx.push(Sfx::Hurt);
        }
    }

    hits.clear();
    for (entity, id, e) in enemies.iter() {
        if !e.alive() || !is_ground(e.kind) || struck.contains(&entity) {
            continue;
        }
        if in_kill_rect(pos, facing, e.pos) {
            hits.push((id.0, entity));
        }
    }
    hits.sort_unstable_by_key(|&(id, _)| id);
    for &(_, entity) in hits.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        e.hurt(MOUSETRAP_DAMAGE_ENEMY);
        let killed = !e.alive();
        // Kills go through `loot::reap_enemies_system` later this tick: gem,
        // score, `Sfx::Kill`/`Explosion`, death burst — the normal loot path.
        vfx::emit_weapon_hit(particles, rng, e.pos, Vec2::ZERO, None, e.kind, killed);
        damage_numbers.record(
            e.id,
            e.pos,
            e.radius,
            MOUSETRAP_DAMAGE_ENEMY,
            killed,
            DamageSource::Normal,
        );
        struck.push(entity);
    }
}

/// The rat's glue slow, without touching `systems/player.rs`.
///
/// `player_system` derives its target velocity from `TickInput::move_dir`
/// (`throttle * stats.move_speed`) and lerps the current velocity toward it, so
/// there are two things to do before it runs:
///
/// 1. shorten the throttle vector: the target becomes `mult` of the rat's own
///    speed;
/// 2. clip the current velocity to that same ceiling, so a rat that walks into
///    the glue is *immediately* at `mult` speed instead of skidding through it
///    for a fifth of a second.
///
/// Both recurrences are then exactly `mult`-scaled versions of the free walk
/// (`v' = (1-k)v + k*mult*speed`), so the glued rat covers exactly `mult` of
/// the ground a free rat does — the art's 20 %, frame-rate independent.
#[inline]
fn apply_player_slow(input: &mut TickInput, player: &mut Player, mult: f32) {
    input.move_dir = input.move_dir * mult;
    let cap = player.loadout.stats.move_speed * mult;
    player.vel = player.vel.clamp_len(cap);
}

/// Advance every sticky board and mousetrap by one tick.
pub fn traps_system(
    mut input: ResMut<TickInput>,
    mut player: ResMut<Player>,
    mut level: ResMut<Level>,
    mut traps: ResMut<TrapRuntime>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    slow: Query<&TrapSlow>,
    mut commands: Commands,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut hits: Local<Vec<(u64, Entity)>>,
    mut spent_sticky: Local<Vec<usize>>,
    mut spent_mice: Local<Vec<usize>>,
) {
    let dt = input.dt;
    traps.sync(&level);

    // Art `hit_flash`: a short 1 -> 0 flash when the bar strikes. Decayed for
    // every trap — including sprung ones, which drop out of the live list on
    // the tick they snap — so a flash can never stick at half brightness. It
    // runs here, before the early-out below, for exactly that reason.
    if dt > 0.0 {
        for view in level.objects.mousetraps.iter_mut() {
            if view.hit_flash > 0.0 {
                view.hit_flash = (view.hit_flash - dt * 4.0).max(0.0);
            }
        }
    }

    // Defensive prune: a swapped level (or a test editing `level.objects`) can
    // leave a live index pointing at a spent trap. Cheap (`live` is tiny).
    traps
        .live_sticky
        .retain(|&i| is_live_sticky(level.objects.sticky_traps[i].state));
    traps
        .live_mice
        .retain(|&i| is_live_mouse(level.objects.mousetraps[i].state));
    if traps.live_sticky.is_empty() && traps.live_mice.is_empty() {
        return;
    }

    // The rat's own state at the start of the tick (nothing has moved yet).
    let player_pos = player.pos;
    let player_alive = player.alive();
    let player_dashing = player.dash_active > 0.0;
    // Exactly `player_system`'s dash condition: a dash that will actually
    // happen this tick breaks the rat out of the glue (and a dash in progress
    // crosses the pad without sticking, per the art's `Idle` counter).
    let player_dash_now = input.dash && player.dash_cd <= 0.0;
    // Wave 3 Clean Socks (`systems::powerups`): while they are on, the glue pad
    // has nothing to grip. Treated exactly like a dash here — a socked rat is
    // released from a board it is already on and crosses every other pad
    // without sticking — because the design has one rule ("ignore
    // sticky-board drag") and no per-board exceptions.
    let player_socked = player.buffs.socks > 0.0;

    spent_sticky.clear();
    spent_mice.clear();

    // ---- Sticky boards ---------------------------------------------------
    //
    // One pass: advance the board that holds a victim, or catch one if the
    // board is free. Boards are walked in index order, so the board a victim
    // ends up on is the lowest-index one it overlaps.
    let mut player_glue = 1.0f32;
    let mut player_glued = false;
    for k in 0..traps.live_sticky.len() {
        let i = traps.live_sticky[k];
        let slot = traps.sticky[i];
        let view = level.objects.sticky_traps[i];
        match slot.victim {
            Some(Victim::Player) => {
                let remaining = slot.remaining - dt;
                if remaining <= 0.0
                    || player_dash_now
                    || player_dashing
                    || player_socked
                    || !player_alive
                {
                    // The tear follows whatever the rat is doing: its own
                    // momentum, or straight off the board when it is standing
                    // still (a spent hold).
                    let pull = if player.vel.len_sq() > 1e-4 {
                        player.vel
                    } else {
                        player_pos - view.pos
                    };
                    vfx::emit_glue_tear_free(
                        &mut particles,
                        &mut rng.0,
                        view.pos,
                        player_pos,
                        pull,
                        PLAYER_RADIUS,
                    );
                    let v = &mut level.objects.sticky_traps[i];
                    v.state = StickyTrapState::Used;
                    v.progress = 1.0;
                    v.victim_pos = player_pos;
                    v.victim_radius = PLAYER_RADIUS;
                    traps.sticky[i] = StickySlot::default();
                    spent_sticky.push(i);
                    sfx.0.push(Sfx::TrapGlueRelease);
                } else {
                    traps.sticky[i].remaining = remaining;
                    let v = &mut level.objects.sticky_traps[i];
                    v.state = StickyTrapState::Stuck;
                    // `progress` is the art's strand-stretch / snap driver.
                    v.progress = (1.0 - remaining / slot.hold.max(1e-3)).clamp(0.0, 1.0);
                    v.victim_pos = player_pos;
                    v.victim_radius = PLAYER_RADIUS;
                    player_glued = true;
                    player_glue = STICKY_TRAP_SLOW_MULT;
                }
            }
            Some(Victim::Enemy(entity)) => {
                let remaining = slot.remaining - dt;
                let live = match enemies.get_mut(entity) {
                    Ok((_, _, e)) if e.alive() => Some((e.pos, e.radius, e.vel)),
                    _ => None,
                };
                match live {
                    Some((pos, radius, _)) if remaining > 0.0 => {
                        traps.sticky[i].remaining = remaining;
                        let v = &mut level.objects.sticky_traps[i];
                        v.state = StickyTrapState::Stuck;
                        v.progress = (1.0 - remaining / slot.hold.max(1e-3)).clamp(0.0, 1.0);
                        v.victim_pos = pos;
                        v.victim_radius = radius;
                    }
                    _ => {
                        // Hold spent, victim killed or despawned: the glue is
                        // torn through.
                        if let Some((pos, radius, vel)) = live {
                            let pull = if vel.len_sq() > 1e-4 {
                                vel
                            } else {
                                pos - view.pos
                            };
                            vfx::emit_glue_tear_free(
                                &mut particles,
                                &mut rng.0,
                                view.pos,
                                pos,
                                pull,
                                radius,
                            );
                            let v = &mut level.objects.sticky_traps[i];
                            v.victim_pos = pos;
                            v.victim_radius = radius;
                        }
                        let v = &mut level.objects.sticky_traps[i];
                        v.state = StickyTrapState::Used;
                        v.progress = 1.0;
                        traps.sticky[i] = StickySlot::default();
                        commands.entity(entity).remove::<TrapSlow>();
                        spent_sticky.push(i);
                        sfx.0.push(Sfx::TrapGlueRelease);
                    }
                }
            }
            None => {
                // Free board: the rat first (it has priority over the horde it
                // is baiting), then the lowest-`SpawnId` ground enemy on the pad.
                if !player_glued
                    && player_alive
                    && !player_dashing
                    && !player_dash_now
                    && !player_socked
                    && on_pad(view.pos, view.facing, player_pos)
                {
                    player_glued = true;
                    player_glue = STICKY_TRAP_SLOW_MULT;
                    traps.sticky[i] = StickySlot {
                        victim: Some(Victim::Player),
                        remaining: STICKY_HOLD_PLAYER_SECS,
                        hold: STICKY_HOLD_PLAYER_SECS,
                    };
                    vfx::emit_glue_stick(
                        &mut particles,
                        &mut rng.0,
                        view.pos,
                        player_pos,
                        PLAYER_RADIUS,
                    );
                    let v = &mut level.objects.sticky_traps[i];
                    v.state = StickyTrapState::Stuck;
                    v.progress = 0.0;
                    v.victim_pos = player_pos;
                    v.victim_radius = PLAYER_RADIUS;
                    sfx.0.push(Sfx::TrapGlueGrab);
                } else {
                    if let Some((_, entity)) =
                        pick_enemy_on_pad(&mut enemies, &slow, view.pos, view.facing)
                    {
                        if let Ok((_, _, e)) = enemies.get_mut(entity) {
                            let (pos, radius, kind) = (e.pos, e.radius, e.kind);
                            let (hold, mult) = enemy_hold(kind);
                            // The component is the per-entity slow the steering
                            // system multiplies in (`enemy_move` hook).
                            commands.entity(entity).insert(TrapSlow(mult));
                            traps.sticky[i] = StickySlot {
                                victim: Some(Victim::Enemy(entity)),
                                remaining: hold,
                                hold,
                            };
                            vfx::emit_glue_stick(
                                &mut particles,
                                &mut rng.0,
                                view.pos,
                                pos,
                                radius,
                            );
                            let v = &mut level.objects.sticky_traps[i];
                            v.state = StickyTrapState::Stuck;
                            v.progress = 0.0;
                            v.victim_pos = pos;
                            v.victim_radius = radius;
                            sfx.0.push(Sfx::TrapGlueGrab);
                        }
                    }
                }
            }
        }
    }

    // ---- Mousetraps ------------------------------------------------------
    for k in 0..traps.live_mice.len() {
        let i = traps.live_mice[k];
        let mut view: MousetrapView = level.objects.mousetraps[i];
        match view.state {
            MousetrapState::Armed => {
                // Art `Armed`: *any* ground entity entering the bait circle —
                // the rat or a member of the horde it is baiting — starts the
                // warning. Bats fly over, dead entities have no weight.
                let mut tripped = player_alive && on_bait(view.pos, view.facing, player_pos);
                if !tripped {
                    tripped = enemies.iter().any(|(_, _, e)| {
                        e.alive() && is_ground(e.kind) && on_bait(view.pos, view.facing, e.pos)
                    });
                }
                if tripped {
                    view.state = MousetrapState::Triggered;
                    view.progress = 0.0;
                    traps.mice[i].struck.clear();
                    traps.mice[i].player_struck = false;
                    // Art `Triggered`: the bait pedal dips first (the quiet
                    // click), then the bar lands with the crack at the snap
                    // beat below.
                    sfx.0.push(Sfx::TrapTrigger);
                }
            }
            MousetrapState::Triggered => {
                // Art `Triggered`: 0.28 s of shudder, then the bar releases.
                view.progress += dt / MOUSETRAP_TRIGGER_SECS;
                if view.progress >= 1.0 {
                    view.state = MousetrapState::Snapping;
                    view.progress = 0.0;
                }
            }
            MousetrapState::Snapping => {
                // Art `Snapping`: 0.20 s swing, damaging between `progress`
                // 0.40 and 0.65. Tested as a *crossing* (previous progress on
                // one side, new progress on the other) so a long frame can
                // never skip straight past the hit window.
                let before = view.progress;
                view.progress = (before + dt / MOUSETRAP_SNAP_SECS).min(1.0);
                if view.progress >= MOUSETRAP_HIT_PROGRESS_MIN && before <= MOUSETRAP_HIT_PROGRESS_MAX
                {
                    let pos = view.pos;
                    let facing = view.facing;
                    // The bar slams whether or not anything is under it: one
                    // crack and one flash per snap, not one per window tick.
                    if before < MOUSETRAP_HIT_PROGRESS_MIN {
                        // One crack, one wood-splinter burst, one flash per
                        // snap — not one per window tick.
                        vfx::emit_mousetrap_snap(&mut particles, &mut rng.0, pos, facing);
                        sfx.0.push(Sfx::TrapSnap);
                        view.hit_flash = 1.0;
                    }
                    let mouse = &mut traps.mice[i];
                    snap_window(
                        pos,
                        facing,
                        &mut player,
                        player_alive,
                        &mut mouse.player_struck,
                        &mut enemies,
                        &mut mouse.struck,
                        &mut hits,
                        &mut rng.0,
                        &mut particles,
                        &mut damage_numbers,
                        &mut sfx.0,
                    );
                }
                if view.progress >= 1.0 {
                    view.state = MousetrapState::Sprung;
                    view.progress = 1.0;
                    spent_mice.push(i);
                }
            }
            MousetrapState::Sprung => spent_mice.push(i),
        }
        level.objects.mousetraps[i] = view;
    }

    // ---- Write the rat's glue back into its movement ----------------------
    if player_glued {
        apply_player_slow(&mut input, &mut player, player_glue);
    }

    // Retire the traps that just became worthless (used board / sprung trap).
    for &i in spent_sticky.iter() {
        traps.live_sticky.retain(|&j| j != i);
    }
    for &i in spent_mice.iter() {
        traps.live_mice.retain(|&j| j != i);
    }
    spent_sticky.clear();
    spent_mice.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_traps::StickyTrapView;
    use crate::game::collision::TileMap;
    use crate::game::ecs::systems::{enemy_move, loot, player};
    use crate::game::level::{MAP_COLS, MAP_ROWS};
    use crate::game::world::World;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const DT: f32 = 1.0 / 60.0;
    const FACING: Vec2 = Vec2 { x: 1.0, y: 0.0 };

    /// A world with no walls and no generated traps: the trap under test is the
    /// only thing in the arena, so a failure is the trap's and not the map's.
    fn open_arena(seed: u64) -> World {
        let mut w = World::new(Theme::Sewer, seed);
        w.level.tiles = TileMap::new(MAP_COLS, MAP_ROWS, w.level.theme, 0);
        w.level.objects.sticky_traps.clear();
        w.level.objects.mousetraps.clear();
        w.player.pos = w.level.arena * 0.5;
        w
    }

    fn centre(w: &World) -> Vec2 {
        w.level.arena * 0.5
    }

    /// Narrow harness: `traps_system` alone (it runs first in the real tick).
    fn traps_tick(w: &mut World, move_dir: Vec2, dash: bool) {
        w.begin_tick(DT, move_dir, dash);
        w.ecs
            .run_system_cached(traps_system)
            .expect("traps system must run");
        w.end_tick();
    }

    /// Narrow harness for the real ordering of the two systems the glue slow
    /// touches: `traps_system` then `player_system`.
    fn move_tick(w: &mut World, move_dir: Vec2) {
        w.begin_tick(DT, move_dir, false);
        w.ecs
            .run_system_cached(traps_system)
            .expect("traps system must run");
        w.ecs
            .run_system_cached(player::player_system)
            .expect("player system must run");
        w.end_tick();
    }

    fn board_state(w: &World, i: usize) -> StickyTrapState {
        w.level.objects.sticky_traps[i].state
    }

    fn mouse_state(w: &World, i: usize) -> MousetrapState {
        w.level.objects.mousetraps[i].state
    }

    fn add_board(w: &mut World, pos: Vec2) {
        w.level
            .objects
            .sticky_traps
            .push(StickyTrapView::new(pos, FACING));
    }

    fn add_mousetrap(w: &mut World, pos: Vec2) {
        w.level
            .objects
            .mousetraps
            .push(MousetrapView::new(pos, FACING));
    }

    fn add_enemy(w: &mut World, kind: EnemyKind, pos: Vec2, hp_mult: f32) -> Entity {
        let mut rng = StdRng::seed_from_u64(7);
        w.spawn_enemy_with_rng(kind, pos, hp_mult, &mut rng)
    }

    fn enemy_of(w: &mut World, entity: Entity) -> Enemy {
        let mut q = w.ecs.query::<&Enemy>();
        q.get(&w.ecs, entity).expect("enemy must exist").clone()
    }

    // ---- Sticky boards ---------------------------------------------------

    /// The rat is held for the designer's 0.9 s and the board is then spent;
    /// `progress` is written into the view (the art's strand stretch) and ends
    /// near 1.0 before the state flips to `Used`.
    #[test]
    fn glue_holds_the_rat_for_0_9s_then_spends_the_board() {
        let mut w = open_arena(101);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;

        // Tick 1 catches the rat.
        traps_tick(&mut w, FACING, false);
        {
            let v = w.level.objects.sticky_traps[0];
            assert_eq!(v.state, StickyTrapState::Stuck);
            assert!(v.progress.abs() < 1e-6, "a fresh catch starts at progress 0");
            assert_eq!(v.victim_radius, PLAYER_RADIUS);
            assert!((v.victim_pos.dist(w.player.pos)) < 1e-4);
        }

        // Hold exactly the designer's 0.9 s (the state machine steps in whole
        // frames, so allow a frame either side of the nominal tick count).
        let mut held = 0usize;
        let mut last_progress = 0.0f32;
        for _ in 0..1_000 {
            if board_state(&w, 0) != StickyTrapState::Stuck {
                break;
            }
            last_progress = w.level.objects.sticky_traps[0].progress;
            traps_tick(&mut w, FACING, false);
            held += 1;
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        let held_secs = held as f32 * DT;
        assert!(
            (held_secs - STICKY_HOLD_PLAYER_SECS).abs() <= 2.0 * DT,
            "the rat was held {held_secs} s, expected {STICKY_HOLD_PLAYER_SECS} s"
        );
        assert!(
            (0.9..1.0).contains(&last_progress),
            "strand progress just before the release = {last_progress}"
        );
        // The art keeps drawing the spent board where it was.
        assert!((w.level.objects.sticky_traps[0].pos - c).len() < 1e-4);
    }

    /// The rat walks at exactly 20 % of its free speed while glued (the art's
    /// `STICKY_TRAP_SLOW_MULT`), measured against an identical trap-free walk.
    #[test]
    fn glued_rat_walks_at_twenty_percent_speed() {
        let hold_ticks = (STICKY_HOLD_PLAYER_SECS / DT).round() as usize;

        let walk = |glued: bool| {
            let mut w = open_arena(103);
            let c = centre(&w);
            if glued {
                add_board(&mut w, c);
            }
            w.player.pos = c;
            let x0 = w.player.pos.x;
            for _ in 0..hold_ticks {
                move_tick(&mut w, FACING);
            }
            if glued {
                assert_eq!(board_state(&w, 0), StickyTrapState::Stuck, "still glued");
            }
            w.player.pos.x - x0
        };

        let glued = walk(true);
        let free = walk(false);
        assert!(free > 40.0, "the free walk covered only {free} px");
        let ratio = glued / free;
        assert!(
            (0.19..0.21).contains(&ratio),
            "glued walk {glued} px vs free {free} px = {ratio:.3}"
        );
    }

    /// A dash breaks the rat out immediately (the art's `Stuck` counter), and a
    /// dash that is already in progress crosses the pad without sticking.
    #[test]
    fn dash_breaks_glue_and_a_dash_crosses_cleanly() {
        let mut w = open_arena(107);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.dash_cd = 0.0;

        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);

        // Press dash: the board releases instead of holding.
        traps_tick(&mut w, FACING, true);
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert!((w.level.objects.sticky_traps[0].progress - 1.0).abs() < 1e-6);

        // Dash on cooldown does *not* break the glue (it is not a dash).
        let mut w = open_arena(109);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.dash_cd = 1.0;
        traps_tick(&mut w, FACING, false);
        traps_tick(&mut w, FACING, true);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);

        // A rat mid-dash crosses an idle board without sticking to it.
        let mut w = open_arena(113);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.dash_active = 0.1;
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
    }

    /// One victim per board: the rat has it and a second body on the same pad
    /// is not caught, and a second board does not catch an already-glued rat.
    #[test]
    fn a_board_holds_one_victim_only() {
        let mut w = open_arena(127);
        let c = centre(&w);
        add_board(&mut w, c);
        // Two more boards overlapping the first: only the lowest index may win.
        add_board(&mut w, c + Vec2::new(2.0, 0.0));
        w.player.pos = c;

        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        assert_eq!(
            board_state(&w, 1),
            StickyTrapState::Idle,
            "the second board caught the same rat"
        );

        // Same for the horde: one Skitterer per board.
        let mut w = open_arena(131);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(200.0, 0.0);
        let a = add_enemy(&mut w, EnemyKind::Skitterer, c, 1.0);
        let b = add_enemy(&mut w, EnemyKind::Skitterer, c + Vec2::new(2.0, 0.0), 1.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        // Exactly one of the two carries the slow component.
        let has = |w: &mut World, e: Entity| {
            let mut q = w.ecs.query::<&TrapSlow>();
            q.get(&w.ecs, e).is_ok()
        };
        assert!(
            has(&mut w, a) ^ has(&mut w, b),
            "both skitterers or neither were glued"
        );
    }

    /// Bats fly over both traps (art `Idle` / `Armed`: flying entities pass).
    #[test]
    fn flying_bats_pass_over_traps() {
        let mut w = open_arena(137);
        let c = centre(&w);
        add_board(&mut w, c);
        add_mousetrap(&mut w, c + Vec2::new(60.0, 0.0));
        w.player.pos = c + Vec2::new(200.0, 200.0);
        add_enemy(&mut w, EnemyKind::Bat, c, 1.0);
        add_enemy(
            &mut w,
            EnemyKind::Bat,
            c + Vec2::new(60.0 + MOUSETRAP_TRIGGER_OFFSET_U, 0.0),
            1.0,
        );

        for _ in 0..5 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Armed);
    }

    /// Hold durations by victim class (designer's numbers), and the `TrapSlow`
    /// multiplier the enemy steering system reads.
    #[test]
    fn glue_holds_each_enemy_class_for_its_own_duration() {
        // (kind, hold seconds art/designer value)
        for (kind, hold) in [
            (EnemyKind::Skitterer, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS),
            (EnemyKind::Cat, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS),
            (EnemyKind::Spitter, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS),
            (EnemyKind::Brute, STICKY_TRAP_HOLD_BRUTE_SECS),
            (EnemyKind::Boss, STICKY_HOLD_BOSS_SECS),
        ] {
            let mut w = open_arena(139);
            let c = centre(&w);
            add_board(&mut w, c);
            w.player.pos = c + Vec2::new(300.0, 0.0);
            let e = add_enemy(&mut w, kind, c, 20.0);

            traps_tick(&mut w, Vec2::ZERO, false);
            assert_eq!(
                board_state(&w, 0),
                StickyTrapState::Stuck,
                "{kind:?} was not caught"
            );
            let expected_mult = if kind == EnemyKind::Boss {
                STICKY_SLOW_BOSS
            } else {
                STICKY_TRAP_SLOW_MULT
            };
            {
                let mut q = w.ecs.query::<&TrapSlow>();
                let slow = q.get(&w.ecs, e).expect("caught enemy carries TrapSlow");
                assert!((slow.0 - expected_mult).abs() < 1e-6);
            }

            let mut held = 0usize;
            for _ in 0..10_000 {
                if board_state(&w, 0) != StickyTrapState::Stuck {
                    break;
                }
                traps_tick(&mut w, Vec2::ZERO, false);
                held += 1;
            }
            assert_eq!(board_state(&w, 0), StickyTrapState::Used, "{kind:?}");
            let held_secs = held as f32 * DT;
            assert!(
                (held_secs - hold).abs() <= 2.0 * DT,
                "{kind:?} was held {held_secs} s, expected {hold} s"
            );
            // The slow is taken off with the glue.
            let mut q = w.ecs.query::<&TrapSlow>();
            assert!(q.get(&w.ecs, e).is_err(), "{kind:?} kept its slow");
        }
    }

    /// A glued enemy really moves at 20 % of an identical free enemy's speed
    /// through the real `traps → enemy_steering` ordering.
    #[test]
    fn glued_enemy_moves_at_twenty_percent_speed() {
        let steer_tick = |glued: bool| {
            let mut w = open_arena(149);
            let c = centre(&w);
            w.player.pos = c + Vec2::new(400.0, 0.0);
            if glued {
                // The enemy is placed on the pad and the rat far away, so the
                // steering pulls it off the pad in a straight line.
                add_board(&mut w, c);
            }
            add_enemy(&mut w, EnemyKind::Skitterer, c, 1.0);
            let e = w.enemies().first().map(|e| e.id).expect("one enemy");
            let start_pos = w.enemies()[0].pos;
            for _ in 0..30 {
                w.begin_tick(DT, Vec2::ZERO, false);
                w.ecs
                    .run_system_cached(traps_system)
                    .expect("traps system must run");
                w.ecs
                    .run_system_cached(enemy_move::enemy_steering_system)
                    .expect("steering must run");
                w.end_tick();
            }
            let end = w.enemies().into_iter().find(|x| x.id == e).expect("alive");
            end.pos.dist(start_pos)
        };

        let glued = steer_tick(true);
        let free = steer_tick(false);
        assert!(free > 8.0, "the free enemy only moved {free} px");
        let ratio = glued / free;
        assert!(
            (0.18..0.23).contains(&ratio),
            "glued {glued} px vs free {free} px = {ratio:.3}"
        );
    }

    // ---- Mousetraps ------------------------------------------------------

    /// Full state timeline: `Armed` → 0.28 s `Triggered` (no damage) → 0.20 s
    /// `Snapping` → `Sprung`, with the art's `hit_flash` at the impact.
    #[test]
    fn mousetrap_warns_then_snaps_then_is_sprung() {
        let mut w = open_arena(151);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        let bait = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        let e = add_enemy(&mut w, EnemyKind::Brute, bait, 5.0); // 300 hp

        let mut ticks = 0usize;
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert!(w.level.objects.mousetraps[0].progress.abs() < 1e-6);
        for _ in 0..10_000 {
            assert!(
                enemy_of(&mut w, e).hp == 300.0,
                "the warning damaged the brute"
            );
            if mouse_state(&w, 0) != MousetrapState::Triggered {
                break;
            }
            traps_tick(&mut w, Vec2::ZERO, false);
            ticks += 1;
        }
        // `ticks` advancing ticks elapse between the trigger and the snap.
        let warn = ticks as f32 * DT;
        assert!(
            (warn - MOUSETRAP_TRIGGER_SECS).abs() <= DT * 1.01,
            "warning lasted {warn} s, expected {}",
            MOUSETRAP_TRIGGER_SECS
        );
        assert_eq!(mouse_state(&w, 0), MousetrapState::Snapping);

        ticks = 0;
        for _ in 0..10_000 {
            if mouse_state(&w, 0) != MousetrapState::Snapping {
                break;
            }
            traps_tick(&mut w, Vec2::ZERO, false);
            ticks += 1;
        }
        let snap = ticks as f32 * DT;
        assert!(
            (snap - MOUSETRAP_SNAP_SECS).abs() <= DT * 1.01,
            "snap lasted {snap} s, expected {}",
            MOUSETRAP_SNAP_SECS
        );
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert!((w.level.objects.mousetraps[0].progress - 1.0).abs() < 1e-6);
        assert!(
            w.level.objects.mousetraps[0].hit_flash > 0.0,
            "the impact flash had already faded"
        );
        // 80 damage, once.
        assert_eq!(enemy_of(&mut w, e).hp, 300.0 - MOUSETRAP_DAMAGE_ENEMY);

        // Sprung is permanent and harmless: nothing more happens, and the
        // impact flash fades out on its own.
        let hp = enemy_of(&mut w, e).hp;
        for _ in 0..30 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert_eq!(enemy_of(&mut w, e).hp, hp);
        assert!(w.level.objects.mousetraps[0].hit_flash.abs() < 1e-6);
    }

    /// Three enemies under the bar are each hit exactly once, one outside the
    /// oriented rectangle is untouched.
    #[test]
    fn a_mousetrap_hits_three_enemies_exactly_once_each() {
        let mut w = open_arena(157);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        let perp = FACING.perp();
        // Inside the art's kill rect (`u in [-1, 9.5]`, `|v| <= 6`).
        let inside = [
            c + FACING * 1.0,
            c + FACING * 4.0 + perp * 3.0,
            c + FACING * 8.0 - perp * 4.0,
        ];
        for p in inside.iter() {
            // Skitterers would die to one 80-damage hit, so use Brutes with an
            // inflated health pool: only a *second* hit can be detected on them.
            add_enemy(&mut w, EnemyKind::Brute, *p, 4.0); // 240 hp
        }
        let outside = [
            c - FACING * 4.0,              // behind the cocked bar
            c + FACING * 4.0 + perp * 8.0, // beside the bar
        ];
        for p in outside.iter() {
            add_enemy(&mut w, EnemyKind::Brute, *p, 4.0);
        }
        // Spawn order is `SpawnId` order: the first three are the inside ones.
        let ids: Vec<u32> = w.enemies().iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 5);

        // Fire the trap.
        w.level.objects.mousetraps[0].state = MousetrapState::Triggered;
        for _ in 0..(MOUSETRAP_TRIGGER_SECS / DT).ceil() as usize + 2 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        let mut rounds = 0;
        while mouse_state(&w, 0) == MousetrapState::Snapping && rounds < 100 {
            traps_tick(&mut w, Vec2::ZERO, false);
            rounds += 1;
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);

        // Each inside brute lost exactly one hit's worth of hp (not two).
        for e in w.enemies() {
            let start = 240.0;
            if ids[..3].contains(&e.id) {
                assert_eq!(
                    e.hp,
                    start - MOUSETRAP_DAMAGE_ENEMY,
                    "inside brute hit more than once: hp = {}",
                    e.hp
                );
            } else {
                assert_eq!(e.hp, start, "outside brute was hit: hp = {}", e.hp);
            }
        }
    }

    /// Kills by a mousetrap take the normal loot path: the reap later in the
    /// same tick scores, counts the kill and drops the gem.
    #[test]
    fn mousetrap_kills_go_through_the_normal_loot_path() {
        let mut w = open_arena(163);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        for off in [1.0, 3.0, 6.0] {
            add_enemy(&mut w, EnemyKind::Skitterer, c + FACING * off, 1.0); // 6 hp
        }
        // Start the snap directly (the trigger phase is covered above).
        w.level.objects.mousetraps[0].state = MousetrapState::Triggered;

        let mut killed_tick = false;
        for _ in 0..200 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs
                .run_system_cached(traps_system)
                .expect("traps system must run");
            w.ecs
                .run_system_cached(loot::reap_enemies_system)
                .expect("reap must run");
            w.end_tick();
            if w.kills > 0 {
                killed_tick = true;
                break;
            }
        }
        assert!(killed_tick, "the mousetrap never killed the skitterers");
        assert_eq!(w.kills, 3, "all three skitterers died");
        assert!(
            (w.score as f32) >= 3.0 * 1.0 * 10.0,
            "score not credited: {}",
            w.score
        );
        assert!(!w.gems().is_empty(), "no loot dropped");
        assert!(w.sfx.contains(&Sfx::Kill), "no kill sfx queued");
        assert!(w.enemies().is_empty(), "corpses were not reaped");
    }

    /// The rat takes the art's 20 damage (through armour/i-frames) once per
    /// snap, and a dash's i-frames dodge it.
    #[test]
    fn a_mousetrap_damages_the_rat_twenty_once_but_not_through_iframes() {
        let snap = |w: &mut World| {
            w.level.objects.mousetraps[0].state = MousetrapState::Triggered;
            for _ in 0..200 {
                traps_tick(w, Vec2::ZERO, false);
                if mouse_state(w, 0) == MousetrapState::Sprung {
                    break;
                }
            }
            assert_eq!(mouse_state(w, 0), MousetrapState::Sprung);
        };

        // The base pool moved 100 → 130 HP in the early-game balance pass
        // (`Stats::default`), so the check is against the world's own max HP:
        // what this protects is "exactly one hit's worth, once", not the 100.
        let mut w = open_arena(167);
        let full = w.player.max_hp();
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + FACING * 4.0; // standing on the kill plate
        snap(&mut w);
        assert!(
            (w.player.hp - (full - MOUSETRAP_DAMAGE_PLAYER)).abs() < 1e-3,
            "hp = {}",
            w.player.hp
        );

        // A rat already inside i-frames is untouched (the art's "dash i-frames
        // or exit the kill zone" counter).
        let mut w = open_arena(173);
        let full = w.player.max_hp();
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + FACING * 4.0;
        w.player.iframes = 5.0;
        snap(&mut w);
        assert!((w.player.hp - full).abs() < 1e-3, "hp = {}", w.player.hp);

        // Standing off the rectangle is safe.
        let mut w = open_arena(179);
        let full = w.player.max_hp();
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + FACING * 4.0 + FACING.perp() * 9.0;
        snap(&mut w);
        assert!((w.player.hp - full).abs() < 1e-3, "hp = {}", w.player.hp);
    }

    /// The rat can bait the horde through: a trap the rat springs hits the
    /// enemies that followed it into the rectangle.
    #[test]
    fn the_rat_can_bait_the_horde_into_a_trap() {
        let mut w = open_arena(181);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        // The rat stands *behind* the trap, the bait sits on the plate.
        w.player.pos = c - FACING * 9.0;
        add_enemy(&mut w, EnemyKind::Skitterer, c + FACING * 0.5, 1.0);
        add_enemy(&mut w, EnemyKind::Skitterer, c + FACING * 5.0, 1.0);

        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(
            mouse_state(&w, 0),
            MousetrapState::Triggered,
            "an enemy on the bait must trip the trap"
        );
        for _ in 0..200 {
            traps_tick(&mut w, Vec2::ZERO, false);
            if mouse_state(&w, 0) == MousetrapState::Sprung {
                break;
            }
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert!(
            w.enemies().iter().all(|e| !e.alive()),
            "80 damage must kill a 6 hp skitterer"
        );
        // The rat, well behind the cocked bar, was not hit. (The base pool is
        // 130 HP since the early-game balance pass; compare to the world's max.)
        let full = w.player.max_hp();
        assert!((w.player.hp - full).abs() < 1e-3, "hp = {}", w.player.hp);
    }

    // ---- Sound + effects -------------------------------------------------

    /// How many times `s` is in the world's sfx queue (one tick's worth).
    fn sfx_count(w: &World, s: Sfx) -> usize {
        w.sfx.iter().filter(|x| **x == s).count()
    }

    /// The glue board speaks and bursts only on its two edges: the grab and the
    /// tear-free — never a menu `UiMove` stand-in, never on the hold.
    #[test]
    fn glue_grab_and_release_use_their_own_sound_and_burst() {
        let mut w = open_arena(191);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;

        // Grab.
        w.sfx.clear();
        let p0 = w.particles.len();
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        assert_eq!(sfx_count(&w, Sfx::TrapGlueGrab), 1, "one grab sound");
        assert_eq!(sfx_count(&w, Sfx::UiMove), 0, "a stand-in leaked");
        assert!(w.particles.len() > p0, "the grab must burst on the board");

        // The hold itself is silent.
        w.sfx.clear();
        traps_tick(&mut w, FACING, false);
        assert!(w.sfx.is_empty(), "the hold is silent, got {:?}", w.sfx);

        // Release: exactly one tear-free, on the tick the board is spent.
        w.sfx.clear();
        let mut releases = 0usize;
        let mut burst = 0usize;
        for _ in 0..200 {
            let before = w.particles.len();
            traps_tick(&mut w, FACING, false);
            releases += sfx_count(&w, Sfx::TrapGlueRelease);
            burst += usize::from(w.particles.len() > before);
            assert_eq!(sfx_count(&w, Sfx::UiMove), 0, "a stand-in leaked");
            w.sfx.clear();
            if board_state(&w, 0) == StickyTrapState::Used {
                break;
            }
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert_eq!(releases, 1, "exactly one release sound");
        assert_eq!(burst, 1, "exactly one release burst");
    }

    /// A dash tears the glue with the release edge (not with a grab).
    #[test]
    fn a_dash_break_plays_the_tear_free_release() {
        let mut w = open_arena(193);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.dash_cd = 0.0;
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);

        w.sfx.clear();
        traps_tick(&mut w, FACING, true);
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert_eq!(sfx_count(&w, Sfx::TrapGlueRelease), 1);
        assert_eq!(sfx_count(&w, Sfx::TrapGlueGrab), 0);
        assert_eq!(sfx_count(&w, Sfx::UiMove), 0);
    }

    /// A glued enemy gets the grab on sticking and the tear-free on release.
    #[test]
    fn a_glued_enemy_hears_its_own_grab_and_release() {
        let mut w = open_arena(197);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        add_enemy(&mut w, EnemyKind::Skitterer, c, 1.0);

        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        assert_eq!(sfx_count(&w, Sfx::TrapGlueGrab), 1);

        w.sfx.clear();
        let mut releases = 0usize;
        for _ in 0..400 {
            traps_tick(&mut w, Vec2::ZERO, false);
            releases += sfx_count(&w, Sfx::TrapGlueRelease);
            w.sfx.clear();
            if board_state(&w, 0) == StickyTrapState::Used {
                break;
            }
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert_eq!(releases, 1);
    }

    /// The mousetrap: one quiet `TrapTrigger` click when the bait is touched,
    /// then one `TrapSnap` crack (and one splinter burst) at the bar's impact —
    /// never a `Hit` stand-in, never per window tick.
    #[test]
    fn the_mousetrap_clicks_once_then_cracks_once() {
        let mut w = open_arena(199);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        add_enemy(
            &mut w,
            EnemyKind::Brute,
            c + FACING * MOUSETRAP_TRIGGER_OFFSET_U,
            5.0,
        );

        // Trigger: the pedal dips (quiet click), no damage yet.
        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert_eq!(sfx_count(&w, Sfx::TrapTrigger), 1);
        assert_eq!(sfx_count(&w, Sfx::UiMove), 0);

        // The 0.28 s warning is silent.
        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert!(w.sfx.is_empty(), "the warning is silent, got {:?}", w.sfx);

        // The snap: one crack, one burst.
        w.sfx.clear();
        let mut snaps = 0usize;
        let mut bursts = 0usize;
        for _ in 0..300 {
            let before = w.particles.len();
            traps_tick(&mut w, Vec2::ZERO, false);
            snaps += sfx_count(&w, Sfx::TrapSnap);
            assert_eq!(sfx_count(&w, Sfx::Hit), 0, "a stand-in leaked");
            bursts += usize::from(w.particles.len() > before);
            w.sfx.clear();
            if mouse_state(&w, 0) == MousetrapState::Sprung {
                break;
            }
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert_eq!(snaps, 1, "exactly one crack per snap");
        // The burst happens on the impact tick (the brute is damaged there, so
        // the hit vfx fires on the same tick — count the snap's own beat).
        assert!(bursts >= 1, "the snap must splinter");
    }

    // ---- Determinism -----------------------------------------------------

    /// Twin runs: same seed, same scripted input, identical trap state and
    /// identical positions. (The golden trace is regenerated by the integrator;
    /// this only proves the trap system adds no nondeterminism of its own.)
    #[test]
    fn twin_runs_reproduce_the_same_trap_state() {
        let run = |seed: u64| {
            let mut w = World::new(Theme::Kitchen, seed);
            for i in 0..900 {
                // Wander the map so some of the generated traps get stepped on.
                let dir = match (i / 90) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(DT, dir, false);
            }
            let sticky: Vec<(u8, f32)> = w
                .level
                .objects
                .sticky_traps
                .iter()
                .map(|v| (v.state as u8, v.progress))
                .collect();
            let mice: Vec<(u8, f32)> = w
                .level
                .objects
                .mousetraps
                .iter()
                .map(|v| (v.state as u8, v.progress))
                .collect();
            (
                w.player.pos.x,
                w.player.pos.y,
                w.kills,
                w.elapsed,
                sticky,
                mice,
            )
        };

        let a = run(4242);
        let b = run(4242);
        assert_eq!(a.0.to_bits(), b.0.to_bits(), "player x differs");
        assert_eq!(a.1.to_bits(), b.1.to_bits(), "player y differs");
        assert_eq!(a.2, b.2, "kills differ");
        assert_eq!(a.3.to_bits(), b.3.to_bits(), "elapsed differs");
        assert_eq!(a.4, b.4, "sticky boards differ between twin runs");
        assert_eq!(a.5, b.5, "mousetraps differ between twin runs");
        // And the run really exercised traps (the generator places some).
        assert!(
            !a.4.is_empty() || !a.5.is_empty(),
            "no traps on the map, test proves nothing"
        );
    }
}
