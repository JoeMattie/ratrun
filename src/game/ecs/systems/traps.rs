//! (h) Floor traps: sticky glue boards, baited mousetraps and Outfall crab pots.
//!
//! Crab pots replace both legacy traps on the Outfall. Their warning precedes
//! a single deterministic catch in SpawnId order. The pot's view carries the
//! rat's hold across the player and Outfall motion passes. A held enemy stays
//! targetable while movement and contact honour its cage flag.
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
//! StickyTrapState:  Idle --(a body on the pad, room and glue left)--> Stuck
//!                   --(that body's hold spent / killed / the rat dashes out)-->
//!                   back to Idle  ...  --(the lifetime budget is gone AND the
//!                   last victim has left)--> Used
//! MousetrapState:   Armed --(the rat touches the plate, or any ground entity
//!                   enters the trigger circle)-->
//!                   Triggered (0.18 s, ordinary only) --> Snapping (0.20 s)
//!                   --> Sprung   (a hair trigger skips `Triggered`: its bar
//!                   resolves its damage on the touch tick)
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
//!   1.6 s, and costs `2` glue units instead of the whole board;
//! - ground Skitterers / Cats / Spitters `2.5 s`, Brutes `1.5 s`, all at the
//!   art's `20 %` ([`STICKY_TRAP_SLOW_MULT`]); Bats fly over;
//! - bosses and named minibosses keep the art's `0.8 s` at `40 %`
//!   ([`STICKY_HOLD_BOSS_SECS`]) as a *crossing* slow: they never take a glue
//!   unit and are never held;
//! - the mousetrap keeps the art's `0.18 s` bait warning and `0.20 s` swing for
//!   an ordinary snap, deals a share of the rat's pool
//!   ([`MOUSETRAP_DAMAGE_PLAYER_SHARE`] / [`MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE`],
//!   half its flat armour ignored) and flattens ground enemies
//!   ([`MOUSETRAP_DAMAGE_ENEMY`] / [`MOUSETRAP_DAMAGE_ENEMY_HAIR`]).
//!
//! ## A glue board is a crowd tool (wave 3, "sticky traps should accumulate a
//! bunch of enemies before being not sticky")
//!
//! One board now holds **several** victims at once and wears out over a
//! lifetime, instead of catching one body and being spent:
//!
//! - **Capacity.** Bodies take glue *units* by size ([`enemy_units`]): a
//!   Skitterer/Bell Flea 1, a Cat/Spitter/Pillbug/Porter/Turnkey 2, a Brute or
//!   elite 4, the rat 2. A board holds [`STICKY_TRAP_CAPACITY`] (8) units at
//!   once. Each victim takes its own fixed slot ([`STICKY_TRAP_SLOTS`] = 8, a
//!   plain array — no per-tick allocation, no per-board `Vec`).
//! - **Wear.** Each catch also spends its units from a lifetime budget
//!   ([`STICKY_TRAP_BUDGET`] = 20 units), so a fresh board pins roughly a dozen
//!   to twenty small bodies over its life. Spent units are gone even after the
//!   victim tears free; the board only becomes `Used` when the budget is gone
//!   **and** its last victim has left.
//! - **Hold.** A victim is pinned to its own pad slot for its class hold
//!   (`enemy_hold`), then tears free (the existing release feedback) and its
//!   capacity is freed. A victim killed on the board frees its units at once:
//!   the corpse never blocks the pad. A board **remembers a body it has just
//!   released** ([`StickySlot::freed`]) and will not catch it again until it
//!   has walked off the pad once — a time-only grace let a body jammed on the
//!   pad by the crowd be re-caught 1.2 s later and spend the budget twice.
//! - **Overflow.** A body too big for the room left (or arriving once the
//!   budget is gone) is only *slowed* while it crosses, never held: the horde
//!   behind a full board keeps moving. Stuck bodies do not block pathing
//!   either — they are ordinary enemies, and enemy movement never blocks on
//!   other enemies.
//! - **The rat.** Unchanged in feel (same hold, a dash still breaks out, Clean
//!   Socks still ignore the glue) but it costs `2` units like anything else
//!   instead of using the board up. Baiting a pack onto a board and killing it
//!   there is the point: stuck bodies are ordinary enemies, so weapons, pools,
//!   Hantavirus, Wick bursts and contact all reach them normally.

#![allow(clippy::too_many_arguments)]

use bevy_ecs::prelude::*;

use crate::audio::Sfx;
use crate::game::art_crabpot::{CrabPotState, CrabPotView, DOOR_DEPTH, POT_HALF};
use crate::game::art_traps::{
    local_box_contains, MousetrapState, MousetrapView, StickyTrapState, StickyVictimKind,
    StickyVictimView, TrapBait, MOUSETRAP_ARMOUR_IGNORED, MOUSETRAP_BAIT_TAKE_RADIUS,
    MOUSETRAP_DAMAGE_ENEMY, MOUSETRAP_DAMAGE_ENEMY_HAIR, MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE,
    MOUSETRAP_DAMAGE_PLAYER_SHARE, MOUSETRAP_HIT_PROGRESS_MAX, MOUSETRAP_HIT_PROGRESS_MIN,
    MOUSETRAP_KILL_HALF_V, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_U_MIN, MOUSETRAP_SNAP_SECS,
    MOUSETRAP_TRIGGER_OFFSET_U, MOUSETRAP_TRIGGER_RADIUS, MOUSETRAP_TRIGGER_SECS,
    STICKY_TRAP_BLOCKS_MOVEMENT, STICKY_TRAP_BUDGET, STICKY_TRAP_CAPACITY,
    STICKY_TRAP_HOLD_BRUTE_SECS, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS, STICKY_TRAP_PAD_HALF,
    STICKY_TRAP_RAT_UNITS, STICKY_TRAP_RELEASE_GRACE_SECS, STICKY_TRAP_SLOTS,
    STICKY_TRAP_SLOW_MULT,
};
use crate::game::ecs::components::{ChestSpawn, Entity, SpawnId};
use crate::game::director::Director;
use crate::game::ecs::constants::{ENEMY_CAP, PLAYER_RADIUS};
use crate::game::ecs::resources::{
    BossDue, BossOverride, Cvars, DebugFlags, DifficultyLevel, NextEnemyId, NextSpawnId, RngRes,
    RunCounters, SfxQueue, TickInput,
};
use crate::game::ecs::systems::{boss::Boss, crab, miniboss::MinibossSchedule};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::Pickup;
use crate::game::level::{Level, Theme};
use crate::game::particle::damage_number::{DamageNumbers, DamageSource};
use crate::game::particle::vfx;
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::palette::Rgb;

// Runtime checks: the art's pads are walkable hazards, and the two traps this
// file drives are the ones whose states it writes.
const _: () = assert!(!STICKY_TRAP_BLOCKS_MOVEMENT);
// The victim array has to be able to hold a full load of the smallest bodies.
const _: () = assert!(STICKY_TRAP_SLOTS as u8 >= STICKY_TRAP_CAPACITY);

/// How long the rat is held by a glue board.
///
/// The art's `STICKY_TRAP_HOLD_PLAYER_SECS` is 1.6 s; the design review kept
/// the shorter 0.9 s "with dash on cooldown" as the tested value.
pub const STICKY_HOLD_PLAYER_SECS: f32 = 0.9;
/// How long a boss is held (art `StickyTrapState::Stuck`).
pub const STICKY_HOLD_BOSS_SECS: f32 = 0.8;
/// Boss movement multiplier while glued (art `StickyTrapState::Stuck`).
pub const STICKY_SLOW_BOSS: f32 = 0.4;

/// The rat's glue colour on the pad (the rat art's warm grey-brown).
const RAT_GLUE_COLOR: Rgb = (176, 148, 126);

/// Squared "could this point still be on the pad?" prefilter radius.
///
/// Derived directly from [`STICKY_TRAP_PAD_HALF`] (`sqrt(15² + 11²) ≈ 18.6 px`,
/// padded by 1 px per axis for float margin); anything farther from the board
/// centre than this cannot be inside the oriented rectangle, so the cheap
/// distance test rejects almost every enemy before the exact one.
const PAD_REACH_SQ: f32 = (STICKY_TRAP_PAD_HALF.x + 1.0) * (STICKY_TRAP_PAD_HALF.x + 1.0)
    + (STICKY_TRAP_PAD_HALF.y + 1.0) * (STICKY_TRAP_PAD_HALF.y + 1.0);

/// Per-enemy movement multiplier applied by a trap (absent = `1.0`).
///
/// Only glued ground enemies carry this component. It is inserted when a
/// sticky board catches the enemy (and, at the same multiplier, while an
/// overflow body crosses a full pad), and removed when the glue releases;
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

/// One of a board's fixed victim slots.
#[derive(Clone, Copy, Debug)]
struct StuckVictim {
    who: Victim,
    /// Glue units it occupies (0 in the post-release grace).
    units: u8,
    /// Seconds of hold left (or of the rat's post-release grace when `held` is
    /// `false`; a freed body is remembered in [`StickySlot::freed`] instead).
    remaining: f32,
    /// Total hold this victim got, for the art's `progress`.
    hold: f32,
    /// `true` while it is held; `false` only for the rat's short grace after a
    /// dash / sock / spent hold.
    held: bool,
}

/// A boss / named miniboss being slowed (never held) by a board.
#[derive(Clone, Copy, Debug)]
struct BossSlow {
    entity: Entity,
    remaining: f32,
}

/// How many released bodies a board remembers (one per victim slot: a board can
/// only free what it could hold).
const FREED_MEMORY: usize = STICKY_TRAP_SLOTS;

/// Runtime side table for one sticky board.
///
/// Fixed-size: the victim slots are an array, never a `Vec`, so a busy board
/// allocates nothing while it holds its crowd.
#[derive(Clone, Copy, Debug)]
struct StickySlot {
    slots: [Option<StuckVictim>; STICKY_TRAP_SLOTS],
    /// Glue units currently occupied.
    load: u8,
    /// Glue units left in the board's lifetime budget.
    budget_left: u8,
    /// The boss-like body this board is slowing (takes no capacity).
    boss: Option<BossSlow>,
    /// Bodies this board has just let go, newest last. Each is immune to *this*
    /// board until it has walked off the pad once (the documented rule), so a
    /// body jammed on the pad by the crowd is never re-caught and never spends
    /// the same glue twice. Fixed array: no per-tick allocation.
    freed: [Option<Entity>; FREED_MEMORY],
}

impl Default for StickySlot {
    fn default() -> Self {
        Self {
            slots: [None; STICKY_TRAP_SLOTS],
            load: 0,
            // A fresh board carries the whole lifetime budget.
            budget_left: STICKY_TRAP_BUDGET,
            boss: None,
            freed: [None; FREED_MEMORY],
        }
    }
}

impl StickySlot {
    /// Is this entity already in one of this board's slots (held or cooling)?
    #[inline]
    fn touches(&self, entity: Entity) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|v| matches!(v.who, Victim::Enemy(e) if e == entity))
    }

    /// The lowest free victim slot, if any.
    #[inline]
    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }

    /// Has this board just let this body go, with the body still on the pad?
    /// A freed body is not caught (and not slowed) again until it walks off.
    #[inline]
    fn is_freed(&self, entity: Entity) -> bool {
        self.freed.iter().flatten().any(|&e| e == entity)
    }

    /// Remember a body this board has just released.
    ///
    /// Entries are appended in a fixed array; a full memory drops the oldest
    /// (the body that has had the longest chance to walk off the pad).
    fn remember_freed(&mut self, entity: Entity) {
        if self.is_freed(entity) {
            return;
        }
        if let Some(empty) = self.freed.iter_mut().find(|s| s.is_none()) {
            *empty = Some(entity);
            return;
        }
        self.freed.copy_within(1.., 0);
        self.freed[FREED_MEMORY - 1] = Some(entity);
    }
}

/// Glue **units** a ground body takes on a board (the owner's size classes).
///
/// Small bodies (Skitterer, Bell Flea, a Bat that had to walk) are 1; the
/// medium bodies (Cat, Spitter, Pillbug, Porter, Turnkey) 2; a Brute — and an
/// elite Brute — 4. Boss-like bodies are handled outside the capacity (see
/// [`is_boss_like`]) and report 0.
#[inline]
pub fn enemy_units(kind: EnemyKind) -> u8 {
    // A registered kind takes the glue its base body takes (the floor's heavy
    // fills a pad like a Brute).
    match kind.archetype() {
        EnemyKind::Boss => 0,
        // A snake is a long heavy body; a termite horde is a mass (it fills
        // the pad like a Brute does).
        EnemyKind::Brute | EnemyKind::BigSnake | EnemyKind::TermiteHorde => 4,
        EnemyKind::Cat
        | EnemyKind::Spitter
        | EnemyKind::Pillbug
        | EnemyKind::Porter
        | EnemyKind::Turnkey => 2,
        EnemyKind::Skitterer
        | EnemyKind::Bat
        | EnemyKind::BellFlea
        | EnemyKind::WickMoth
        | EnemyKind::HouseCentipede
        | EnemyKind::MurderHornet
        | EnemyKind::Lilliputian => 1,
        EnemyKind::Skater | EnemyKind::Lampfish => 1,
        EnemyKind::FrySchool => 2,
        _ => 1,
    }
}

/// Bodies a glue board never catches, whatever the room: the level boss and a
/// named miniboss are slowed while crossing (the pre-wave-3 rule) and take no
/// capacity.
#[inline]
fn is_boss_like(e: &Enemy) -> bool {
    e.kind == EnemyKind::Boss || e.named
}

/// The art's body class for a caught kind (drives the drawn mound size).
#[inline]
fn victim_kind(kind: EnemyKind) -> StickyVictimKind {
    match enemy_units(kind) {
        1 => StickyVictimKind::Small,
        2 => StickyVictimKind::Medium,
        _ => StickyVictimKind::Big,
    }
}

/// Fixed pad-slot anchors, in pad-normalised `[-1, 1]` units, in slot order.
///
/// Deterministic from the slot index (a victim never moves mid-hold), spread so
/// a full board reads as a crowd rather than a stack.
const SLOT_UV: [(f32, f32); STICKY_TRAP_SLOTS] = [
    (-0.42, -0.36),
    (0.42, 0.36),
    (0.40, -0.38),
    (-0.40, 0.38),
    (-0.02, 0.02),
    (0.06, 0.74),
    (-0.08, -0.74),
    (0.76, -0.02),
];

/// World position of slot `slot` for a victim of collision `radius`.
#[inline]
fn sticky_slot_pos(pos: Vec2, facing: Vec2, slot: usize, radius: f32) -> Vec2 {
    let f = safe_dir(facing);
    let p = f.perp();
    let inset = Vec2::new(
        (STICKY_TRAP_PAD_HALF.x - radius).max(1.5),
        (STICKY_TRAP_PAD_HALF.y - radius).max(1.5),
    );
    let (su, sv) = SLOT_UV[slot.min(STICKY_TRAP_SLOTS - 1)];
    pos + f * (su * inset.x) + p * (sv * inset.y)
}

#[inline]
fn safe_dir(v: Vec2) -> Vec2 {
    if v.len_sq() > 1e-6 {
        v.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    }
}

/// Runtime side table for one mousetrap.
#[derive(Clone, Debug, Default)]
struct MouseSlot {
    /// Ground enemies already damaged by this snap (each target at most once).
    struck: Vec<Entity>,
    /// The rat was already damaged by this snap.
    player_struck: bool,
    /// This snap's damage is already resolved (a hair trigger resolves it on
    /// the touch tick, before the swing's hit window would run).
    hit_done: bool,
}

/// A body standing on a pad, snapshotted for the catch walk.
#[derive(Clone, Copy, Debug)]
pub struct PadCandidate {
    id: u64,
    entity: Entity,
    kind: EnemyKind,
    named: bool,
    radius: f32,
    pos: Vec2,
}

/// All of the system's scratch, in one reused `Local` (the function-argument
/// budget is 16, and every buffer here must be allocated once per run).
#[derive(Default)]
pub struct Scratch {
    /// Mousetrap snap targets, sorted by `SpawnId` before damage.
    hits: Vec<(u64, Entity)>,
    /// Boards / traps that became worthless this tick.
    spent_sticky: Vec<usize>,
    spent_mice: Vec<usize>,
    /// Bodies on this tick's pad, sorted by `SpawnId`.
    cands: Vec<PadCandidate>,
    /// Every already-held body across the live boards this tick.
    held: Vec<Entity>,
    /// Bodies given a crossing slow this tick (see `TrapRuntime::cross_prev`).
    cross_now: Vec<Entity>,
    empty_pots: Vec<(Vec2, Vec2)>,
}

/// ECS side table for the floor traps, indexed exactly like `LevelObjects`.
///
/// The *visible* state lives in the art view structs (that is what the
/// renderer reads); this only keeps what the views cannot hold: who is glued
/// and for how long, how much lifetime glue a board has left, and which targets
/// a snap has already hit, and the occupants and clock of each crab pot.
#[derive(Resource, Default)]
pub struct TrapRuntime {
    /// Level identity the lists were built for (`seed`, `theme`).
    seed: u64,
    theme: Option<Theme>,
    sticky: Vec<StickySlot>,
    mice: Vec<MouseSlot>,
    /// Ascending indices of the boards that can still do something.
    live_sticky: Vec<usize>,
    /// Ascending indices of the traps that can still do something (a sprung
    /// trap keeps a live entry while an untaken bait lies on it).
    live_mice: Vec<usize>,
    /// Bodies that carried a *crossing* slow at the end of the last tick, so a
    /// body that walks off a full pad has the glue taken back off it.
    cross_prev: Vec<Entity>,
    pots: Vec<PotSlot>,
}

impl TrapRuntime {
    /// Rebuild the side table if the level (or its object lists) changed.
    ///
    /// Runs every tick but returns immediately in the steady state: the lists
    /// only change on a new level, or when a test edits `level.objects`
    /// directly (the narrow harness does exactly that).
    fn sync(&mut self, level: &Level, enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>) {
        let objects = &level.objects;
        if self.seed == level.seed
            && self.theme == Some(level.theme)
            && self.sticky.len() == objects.sticky_traps.len()
            && self.mice.len() == objects.mousetraps.len()
            && self.pots.len() == objects.crab_pots.len()
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
                .filter(|(_, v)| is_live_mouse(v))
                .map(|(i, _)| i),
        );
        self.cross_prev.clear();
        for entity in self.pots.iter().flat_map(|slot| slot.held.iter().flatten()) {
            if let Ok((_, _, mut enemy)) = enemies.get_mut(*entity) {
                enemy.crab_pot_held = false;
            }
        }
        self.pots.clear();
        self.pots.resize_with(objects.crab_pots.len(), PotSlot::default);
    }
}

pub(crate) const POT_WARNING_SECS: f64 = 0.35;
pub(crate) const POT_RAT_HOLD_SECS: f64 = 0.8;
pub(crate) const POT_DASH_ESCAPE_SECS: f64 = 0.25;
pub(crate) const POT_HORDE_HOLD_SECS: f64 = 3.0;
pub(crate) const POT_OPEN_SECS: f64 = 20.0;
pub(crate) const POT_RAT_DAMAGE: f32 = 6.0;
pub(crate) const POT_HORDE_DAMAGE_SHARE: f32 = 0.25;
pub(crate) const POT_CAPACITY: usize = 4;

#[derive(Default)]
struct PotSlot {
    elapsed: f64,
    held: [Option<Entity>; POT_CAPACITY],
    caught_any: bool,
}

impl PotSlot {
    fn enter(&mut self, pot: &mut CrabPotView, state: CrabPotState) {
        self.elapsed = 0.0;
        pot.progress = 0.0;
        pot.state = state;
    }
}

pub fn set_crab_pot(pos: Vec2, facing: Vec2) -> CrabPotView {
    CrabPotView {
        pos,
        facing,
        state: CrabPotState::Set,
        progress: 0.0,
        caught_rat: false,
        caught: 0,
        anim: 0.0,
    }
}

pub fn in_crab_pot(centre: Vec2, point: Vec2) -> bool {
    let d = point - centre;
    d.x.abs() <= POT_HALF && d.y.abs() <= POT_HALF
}

fn pot_can_catch(e: &Enemy) -> bool {
    let limit = EnemyKind::Brute.radius();
    e.targetable() && !e.crab_pot_held && !e.elite && !e.named && e.kind != EnemyKind::Boss
        && e.radius.max(e.kind.radius()) < limit
}

impl TrapRuntime {
    pub(super) fn pot_dash_ready(&self, index: usize) -> bool {
        self.pots.get(index).is_some_and(|slot| slot.elapsed >= POT_DASH_ESCAPE_SECS)
    }

    /// Called only when the player system actually starts the escape dash.
    pub(super) fn dash_out_of_pot(&mut self, index: usize, pot: &mut CrabPotView) {
        pot.caught_rat = false;
        if pot.caught == 0 {
            self.pots[index].enter(pot, CrabPotState::Open);
        }
    }
}

fn prune_pot_bodies(
    slot: &mut PotSlot,
    pot: &mut CrabPotView,
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    release: bool,
) {
    for held in &mut slot.held {
        if let Some(entity) = *held {
            if let Ok((_, _, mut enemy)) = enemies.get_mut(entity) {
                enemy.vel = Vec2::ZERO;
                if enemy.alive() && !release {
                    continue;
                }
                enemy.crab_pot_held = false;
            }
            *held = None;
        }
    }
    pot.caught = slot.held.iter().flatten().count() as u8;
}

fn tick_crab_pots(
    views: &mut [CrabPotView],
    slots: &mut [PotSlot],
    dt: f32,
    player: &mut Player,
    enemies: &mut Query<(Entity, &SpawnId, &mut Enemy)>,
    hits: &mut Vec<(u64, Entity)>,
    damage_numbers: &mut DamageNumbers,
    sfx: &mut SfxQueue,
    empty_pots: &mut Vec<(Vec2, Vec2)>,
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    use CrabPotState::*;

    let mut rat_held = views.iter().any(|pot| pot.caught_rat);
    for (pot, slot) in views.iter_mut().zip(slots) {
        pot.anim += dt;
        slot.elapsed += f64::from(dt);
        let rat_inside = player.alive() && in_crab_pot(pot.pos, player.pos);
        if pot.state != Open && rat_inside && player.buffs.is_giant() {
            rat_held &= !pot.caught_rat;
            pot.caught_rat = false;
            prune_pot_bodies(slot, pot, enemies, true);
            slot.enter(pot, Open);
            sfx.0.push(Sfx::TrapSnap);
            continue;
        }
        match pot.state {
            Set => {
                if (rat_inside && !rat_held) || enemies.iter().any(|(_, _, e)| {
                    in_crab_pot(pot.pos, e.pos) && pot_can_catch(e)
                }) {
                    slot.enter(pot, Closing);
                    sfx.0.push(Sfx::TrapTrigger);
                }
            }
            Closing => {
                pot.progress = (slot.elapsed / POT_WARNING_SECS).min(1.0) as f32;
                if slot.elapsed < POT_WARNING_SECS {
                    continue;
                }
                slot.enter(pot, Shut);
                slot.caught_any = false;
                sfx.0.push(Sfx::TrapSnap);
                if rat_inside && !rat_held {
                    player.vel = Vec2::ZERO;
                    player.dash_active = 0.0;
                    if player.take_damage(POT_RAT_DAMAGE * dmg_scale) {
                        sfx.0.push(Sfx::Hurt);
                    }
                    pot.caught_rat = player.alive();
                    rat_held = pot.caught_rat;
                    slot.caught_any = true;
                }

                hits.clear();
                hits.extend(enemies.iter().filter_map(|(entity, id, e)| {
                    (in_crab_pot(pot.pos, e.pos) && pot_can_catch(e)).then_some((id.0, entity))
                }));
                hits.sort_unstable_by_key(|&(id, _)| id);
                for &(_, entity) in hits.iter().take(POT_CAPACITY) {
                    let Ok((_, _, mut enemy)) = enemies.get_mut(entity) else { continue };
                    slot.caught_any = true;
                    // The door takes a fixed share of the body, including
                    // through directional armour. The ordinary reap still pays loot.
                    let damage = enemy.max_hp * POT_HORDE_DAMAGE_SHARE;
                    enemy.hp -= damage;
                    enemy.flash = 0.12;
                    enemy.note_damage();
                    let killed = !enemy.alive();
                    damage_numbers.record(
                        enemy.id, enemy.pos, enemy.radius, damage, killed, DamageSource::Normal,
                    );
                    if !killed {
                        enemy.crab_pot_held = true;
                        enemy.vel = Vec2::ZERO;
                        if enemy.kind == EnemyKind::ShoreCrab {
                            crab::cancel_attack(&mut enemy);
                        } else if matches!(enemy.kind, EnemyKind::Skater | EnemyKind::Lampfish) {
                            super::floor_outfall::cancel_attack(&mut enemy);
                        }
                        slot.held[pot.caught as usize] = Some(entity);
                        pot.caught += 1;
                    }
                }
                if pot.caught > 0 {
                    sfx.0.push(Sfx::CrabRattle);
                }
            }
            Shut => {
                if pot.caught_rat && (!player.alive() || slot.elapsed >= POT_RAT_HOLD_SECS) {
                    pot.caught_rat = false;
                    rat_held = false;
                }
                prune_pot_bodies(slot, pot, enemies, slot.elapsed >= POT_HORDE_HOLD_SECS);
                if !pot.caught_rat && pot.caught == 0 {
                    if !slot.caught_any {
                        empty_pots.push((pot.pos, pot.facing));
                    }
                    slot.enter(pot, Open);
                } else {
                    let duration = if pot.caught > 0 { POT_HORDE_HOLD_SECS } else { POT_RAT_HOLD_SECS };
                    pot.progress = (slot.elapsed / duration).min(1.0) as f32;
                }
            }
            Open => {
                pot.progress = (slot.elapsed / POT_OPEN_SECS).min(1.0) as f32;
                if slot.elapsed >= POT_OPEN_SECS {
                    slot.enter(pot, Set);
                }
            }
        }
    }
}

/// A board can still catch a victim or is holding one.
#[inline]
fn is_live_sticky(state: StickyTrapState) -> bool {
    matches!(state, StickyTrapState::Idle | StickyTrapState::Stuck)
}

/// A mousetrap can still be triggered, is in the middle of a snap, or holds an
/// untaken bait on its sprung plate.
#[inline]
fn is_live_mouse(view: &MousetrapView) -> bool {
    view.live() || (!view.bait_taken && view.bait.is_good())
}

/// Ground enemies can be glued and can trip a mousetrap; Bats fly over both.
#[inline]
fn is_ground(kind: EnemyKind) -> bool {
    // The Wick Moth flies (a teardrop with a lit tail) exactly like the Bat, so
    // glue and mousetraps do not hold it either. A registered kind flies or
    // walks exactly as its base body does.
    !kind.outfall_swims()
        && !matches!(
            kind.archetype(),
            EnemyKind::Bat | EnemyKind::WickMoth | EnemyKind::MurderHornet
        )
}

/// `(hold seconds, movement multiplier)` for a ground enemy caught by glue.
#[inline]
fn enemy_hold(kind: EnemyKind) -> (f32, f32) {
    match kind.archetype() {
        // Art `Stuck`: bosses take the shorter 0.8 s at a milder 40 % slow.
        EnemyKind::Boss => (STICKY_HOLD_BOSS_SECS, STICKY_SLOW_BOSS),
        // Art `Stuck`: Brutes are heavy, so they tear free sooner.
        EnemyKind::Brute => (STICKY_TRAP_HOLD_BRUTE_SECS, STICKY_TRAP_SLOW_MULT),
        // Skitterer, Cat, Spitter (and anything else ground-bound).
        _ => (STICKY_TRAP_HOLD_SMALL_ENEMY_SECS, STICKY_TRAP_SLOW_MULT),
    }
}

/// Is `p` on the glue pad of the board at `pos`/`facing`? (Art `Idle`:
/// oriented rectangle `|u| <= STICKY_TRAP_PAD_HALF.x`, `|v| <= STICKY_TRAP_PAD_HALF.y`.)
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

/// Is `p` inside the bait plate's trigger circle? (Art `Armed`: radius
/// `MOUSETRAP_TRIGGER_RADIUS` around `pos + facing * MOUSETRAP_TRIGGER_OFFSET_U`.)
#[inline]
fn on_bait(pos: Vec2, facing: Vec2, p: Vec2) -> bool {
    let centre = pos + safe_dir(facing) * MOUSETRAP_TRIGGER_OFFSET_U;
    p.dist_sq(centre) <= MOUSETRAP_TRIGGER_RADIUS * MOUSETRAP_TRIGGER_RADIUS
}

/// Is `p` close enough to the plate to take the bait? (A hair more generous
/// than the trigger circle, so touching the plate always collects.)
#[inline]
fn near_bait(pos: Vec2, facing: Vec2, p: Vec2) -> bool {
    let centre = pos + safe_dir(facing) * MOUSETRAP_TRIGGER_OFFSET_U;
    p.dist_sq(centre) <= MOUSETRAP_BAIT_TAKE_RADIUS * MOUSETRAP_BAIT_TAKE_RADIUS
}

/// Is `p` inside the striker bar's kill rectangle? (Art `Snapping`:
/// `u in [-4.0, 18.0]`, `|v| <= 10.0`.)
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

/// Does the expanded part of the giant reach an oriented trap rectangle?
fn giant_reaches_box(
    player: &Player,
    pos: Vec2,
    facing: Vec2,
    u_min: f32,
    u_max: f32,
    half_v: f32,
) -> bool {
    let padding = player.buffs.point_hit_radius();
    if padding == 0.0 {
        return false;
    }
    let f = safe_dir(facing);
    let d = player.pos - pos;
    let u = d.dot(f);
    let v = d.dot(f.perp());
    let outside = Vec2::new(u - u.clamp(u_min, u_max), v - v.clamp(-half_v, half_v));
    outside.len_sq() <= padding * padding
}

/// The damage a snap deals to the rat.
///
/// The bar's bite is a share of the rat's own pool
/// ([`MOUSETRAP_DAMAGE_PLAYER_SHARE`] / [`MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE`]),
/// so it keeps meaning something late in a run; **half** its flat armour is
/// ignored (a spring bar does not care about a thick hide), which is why the
/// armour is added back here: `Player::take_damage` subtracts the whole armour
/// from whatever it is handed.
///
/// Never fatal from full health: the shares are 0.35 and 0.65, and a share can
/// only be reduced by armour, so the worst case leaves the rat standing.
#[inline]
fn snap_player_damage(player: &Player, hair: bool) -> f32 {
    let share = if hair {
        MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE
    } else {
        MOUSETRAP_DAMAGE_PLAYER_SHARE
    };
    player.max_hp() * share + player.loadout.stats.armor * MOUSETRAP_ARMOUR_IGNORED
}

/// Resolve a mousetrap snap's active window: the rat (through i-frames) and
/// every ground enemy under the bar, each target at most once.
///
/// Enemy targets are damaged in `SpawnId` order because the impact vfx draws
/// from the rng, so the draw order is part of the spec.
#[allow(clippy::too_many_arguments)]
fn snap_window(
    pos: Vec2,
    facing: Vec2,
    hair: bool,
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
    // `[enemies] sector_damage_mult` for this floor.
    dmg_scale: f32,
) {
    if player_alive && !*player_struck {
        let p = player.pos;
        if in_kill_rect(pos, facing, p)
            || giant_reaches_box(
                player, pos, facing, MOUSETRAP_KILL_U_MIN, MOUSETRAP_KILL_U_MAX, MOUSETRAP_KILL_HALF_V,
            )
        {
            let dmg = snap_player_damage(player, hair) * dmg_scale;
            if player.take_damage(dmg) {
                *player_struck = true;
                vfx::emit_player_hurt(particles, rng, p);
                sfx.push(Sfx::Hurt);
            }
        }
    }

    hits.clear();
    for (entity, id, e) in enemies.iter() {
        if !e.targetable() || !is_ground(e.kind) || struck.contains(&entity) {
            continue;
        }
        if in_kill_rect(pos, facing, e.pos) {
            hits.push((id.0, entity));
        }
    }
    hits.sort_unstable_by_key(|&(id, _)| id);
    let bite = if hair {
        MOUSETRAP_DAMAGE_ENEMY_HAIR
    } else {
        MOUSETRAP_DAMAGE_ENEMY
    };
    for &(_, entity) in hits.iter() {
        let Ok((_, _, mut e)) = enemies.get_mut(entity) else {
            continue;
        };
        if !e.alive() {
            continue;
        }
        // Directional armour (the Pillbug's shell) applies to a trap bite too.
        let applied = e.hurt_from(bite, pos);
        let killed = !e.alive();
        // Kills go through `loot::reap_enemies_system` later this tick: gem,
        // score, `Sfx::Kill`/`Explosion`, death burst — the normal loot path.
        vfx::emit_weapon_hit(particles, rng, e.pos, Vec2::ZERO, None, e.kind, killed);
        damage_numbers.record(
            e.id,
            e.pos,
            e.radius,
            applied,
            killed,
            DamageSource::Normal,
        );
        struck.push(entity);
    }
}

/// Hand the bait to the rat.
///
/// A pickup is spawned on the rat (the pickup system — later this tick —
/// collects it and applies the effect through the one normal path, HUD callout
/// and sfx included); a chest goes through `ChestSpawn`, so it raises
/// `pending_chests` and opens the ordinary chest card. Cheese is nothing.
fn collect_bait(
    bait: TrapBait,
    at: Vec2,
    commands: &mut Commands,
    next_id: &mut NextSpawnId,
) {
    match bait {
        TrapBait::Cheese => {}
        TrapBait::Pickup(kind) => {
            // No `LastPickups` push here: `loot::pickups_system` books the find
            // when the rat collects it, so a second push from the trap made the
            // HUD call it out twice and the App count one Heal as two.
            commands.spawn((
                Pickup {
                    pos: at,
                    kind,
                    bob: 0.0,
                },
                next_id.take(),
            ));
        }
        TrapBait::Chest => {
            commands.queue(ChestSpawn { pos: at });
        }
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

/// Advance the floor traps by one tick.
#[allow(clippy::type_complexity)]
pub fn traps_system(
    mut input: ResMut<TickInput>,
    mut player: ResMut<Player>,
    mut level: ResMut<Level>,
    mut traps: ResMut<TrapRuntime>,
    mut enemies: Query<(Entity, &SpawnId, &mut Enemy)>,
    mut commands: Commands,
    tuning: Res<crate::tuning::RunTuning>,
    mut rng: ResMut<RngRes>,
    mut particles: ResMut<Particles>,
    mut damage_numbers: ResMut<DamageNumbers>,
    mut sfx: ResMut<SfxQueue>,
    mut next_id: ResMut<NextSpawnId>,
    mut scratch: Local<Scratch>,
    spawn: (
        Res<RunCounters>,
        Res<DifficultyLevel>,
        Res<DebugFlags>,
        Res<Cvars>,
        Res<Director>,
        Res<BossDue>,
        Res<BossOverride>,
        Res<MinibossSchedule>,
        // The sector row for the floor's hit softening: this system is at
        // bevy's 16-parameter limit, so it rides in the spawn bundle.
        Res<crate::game::ecs::resources::LevelIndex>,
    ),
    mut next_enemy_id: ResMut<NextEnemyId>,
    bosses: Query<&Boss>,
) {
    let dt = input.dt;
    // `[enemies] sector_damage_mult`: the crab pot's door and the mousetrap's
    // bar carry it, as they did when the softening sat on the rat.
    let dmg_scale = crate::game::director::sector_damage_mult(&tuning, spawn.8.0);
    traps.sync(&level, &mut enemies);
    if level.theme == Theme::Outfall && !level.objects.crab_pots.is_empty() {
        let scratch = &mut *scratch;
        tick_crab_pots(
            &mut level.objects.crab_pots, &mut traps.pots, dt, &mut player, &mut enemies,
            &mut scratch.hits, &mut damage_numbers, &mut sfx, &mut scratch.empty_pots,
            dmg_scale,
        );
        if !scratch.empty_pots.is_empty() {
            let (counters, difficulty, debug, cvars, director, due, forced, appointments, ..) = spawn;
            let elapsed = counters.elapsed;
            let blocked = debug.peace
                || appointments.suppress(elapsed)
                || (!counters.testbed && appointments.next_appointment(&tuning.minibosses.times).is_some_and(|at| elapsed >= at))
                || director.boss_spawned
                || due.due
                || due.claimed
                || forced.0.is_some()
                || !bosses.is_empty()
                || enemies.iter().any(|(_, _, enemy)| enemy.named && enemy.alive());
            let cap = if cvars.enemy_cap == 0 { ENEMY_CAP } else { cvars.enemy_cap };
            let mut room = cap.saturating_sub(1 + enemies.iter().count());
            let mut live = enemies.iter().filter(|(_, _, enemy)| {
                enemy.kind == EnemyKind::ShoreCrab && enemy.alive()
            }).count();
            for (pos, facing) in scratch.empty_pots.drain(..) {
                if blocked || room == 0 || elapsed >= tuning.director.boss_time {
                    break;
                }
                let facing = safe_dir(facing);
                let mouth = POT_HALF + DOOR_DEPTH;
                if let Some(mut enemy) = [mouth, mouth + 4.0, mouth + 8.0].into_iter().find_map(|offset| {
                    crab::release_pot_crab_with(
                        &level, pos + facing * offset, player.pos, difficulty.0, elapsed, live, blocked, &tuning,
                    )
                }) {
                    enemy.id = next_enemy_id.take();
                    commands.spawn((enemy, next_id.take()));
                    live += 1;
                    room -= 1;
                }
            }
        }
    }

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
        .retain(|&i| is_live_mouse(&level.objects.mousetraps[i]));
    // (Not while a crossing slow is still owed its removal: the board that
    // gave it may have been the last live trap on the floor.)
    if traps.live_sticky.is_empty() && traps.live_mice.is_empty() && traps.cross_prev.is_empty() {
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
    // sticky-board drag") and no per-board exceptions. Socks do nothing to a
    // mousetrap: the bar does not care about laundry.
    //
    // Batch 13: the Growth Serum's giant is the same case — a five-times rat
    // is not held by a sticky board either ("sticky boards do not hold the
    // giant"). Mousetraps still snap, at the giant's half damage.
    let player_socked = player.buffs.socks > 0.0 || player.buffs.is_giant();

    scratch.spent_sticky.clear();
    scratch.spent_mice.clear();

    // ---- Sticky boards ---------------------------------------------------
    //
    // One pass per live board: advance the crowd it already holds, then catch
    // whatever new bodies fit. Boards are walked in index order, so the board
    // a victim ends up on is the lowest-index one it overlaps.
    let mut player_glue = 1.0f32;
    let mut player_glued = false;
    scratch.held.clear();
    scratch.cross_now.clear();
    for k in 0..traps.live_sticky.len() {
        let i = traps.live_sticky[k];
        let mut slot = traps.sticky[i];
        let mut view = level.objects.sticky_traps[i];
        let (pos, facing) = (view.pos, view.facing);
        // Rebuilt from the live slots every tick; the sim is the only writer.
        view.victims = [None; STICKY_TRAP_SLOTS];

        // A body the board has let go stays immune until it has walked off the
        // pad once: drop the memory of anything that is dead, despawned or has
        // left. (This is the whole of the re-catch rule — the old timer let a
        // jammed body be caught again 1.2 s after release.)
        for entry in slot.freed.iter_mut() {
            let Some(entity) = *entry else {
                continue;
            };
            let still_on_pad = match enemies.get(entity) {
                Ok((_, _, e)) if e.alive() => {
                    e.pos.dist_sq(pos) <= PAD_REACH_SQ && on_pad(pos, facing, e.pos)
                }
                _ => false,
            };
            if !still_on_pad {
                *entry = None;
            }
        }

        // ---- 1. Advance the victims this board already holds ----------------
        for s in 0..STICKY_TRAP_SLOTS {
            let Some(v) = slot.slots[s] else {
                continue;
            };
            if !v.held {
                // The rat's own short grace after a dash, the socks or a spent
                // hold: it must not be re-stuck the instant it stops. Freed
                // *bodies* live in `freed` instead (they are immune until they
                // walk off the pad, however long that takes).
                let remaining = v.remaining - dt;
                let keep = matches!(v.who, Victim::Player) && remaining > 0.0;
                slot.slots[s] = keep.then_some(StuckVictim { remaining, ..v });
                continue;
            }
            match v.who {
                Victim::Player => {
                    let remaining = v.remaining - dt;
                    let release = remaining <= 0.0
                        || player_dash_now
                        || player_dashing
                        || player_socked
                        || !player_alive;
                    if !release {
                        slot.slots[s] = Some(StuckVictim { remaining, ..v });
                        player_glued = true;
                        player_glue = STICKY_TRAP_SLOW_MULT;
                        // `progress` is the art's strand-stretch / snap driver.
                        view.victims[s] = Some(StickyVictimView {
                            pos: player_pos,
                            radius: PLAYER_RADIUS,
                            kind: StickyVictimKind::Rat,
                            color: RAT_GLUE_COLOR,
                            progress: (1.0 - remaining / v.hold.max(1e-3)).clamp(0.0, 1.0),
                        });
                    } else {
                        // The tear follows whatever the rat is doing: its own
                        // momentum, or straight off the board when it is
                        // standing still (a spent hold, a dash, the socks).
                        let pull = if player.vel.len_sq() > 1e-4 {
                            player.vel
                        } else {
                            player_pos - pos
                        };
                        vfx::emit_glue_tear_free(
                            &mut particles,
                            &mut rng.0,
                            pos,
                            player_pos,
                            pull,
                            PLAYER_RADIUS,
                        );
                        view.add_tear(player_pos);
                        sfx.0.push(Sfx::TrapGlueRelease);
                        slot.load = slot.load.saturating_sub(v.units);
                        slot.slots[s] = Some(StuckVictim {
                            who: v.who,
                            units: 0,
                            remaining: STICKY_TRAP_RELEASE_GRACE_SECS,
                            hold: v.hold,
                            held: false,
                        });
                    }
                }
                Victim::Enemy(entity) => {
                    let remaining = v.remaining - dt;
                    let live = match enemies.get_mut(entity) {
                        Ok((_, _, e)) if e.alive() => {
                            Some((e.pos, e.radius, e.vel, e.kind))
                        }
                        _ => None,
                    };
                    match live {
                        Some((_, radius, _, kind)) if remaining > 0.0 => {
                            // Hold: pin the body to its own pad slot (a board
                            // holds a crowd, it does not let it creep off) and
                            // keep the glue slow on it, so the weapon pass still
                            // sees exactly the body it always did.
                            let anchor = sticky_slot_pos(pos, facing, s, radius);
                            if let Ok((_, _, mut e)) = enemies.get_mut(entity) {
                                e.pos = anchor;
                                e.vel = Vec2::ZERO;
                            }
                            slot.slots[s] = Some(StuckVictim { remaining, ..v });
                            scratch.held.push(entity);
                            view.victims[s] = Some(StickyVictimView {
                                pos: anchor,
                                radius,
                                kind: victim_kind(kind),
                                color: kind.color(),
                                progress: (1.0 - remaining / v.hold.max(1e-3)).clamp(0.0, 1.0),
                            });
                        }
                        _ => {
                            // Hold spent, victim killed or despawned: the glue is
                            // torn through (or, for a corpse, simply let go — the
                            // kill already has its own feedback).
                            if let Some((body_pos, radius, vel, _)) = live {
                                let pull = if vel.len_sq() > 1e-4 {
                                    vel
                                } else {
                                    body_pos - pos
                                };
                                vfx::emit_glue_tear_free(
                                    &mut particles,
                                    &mut rng.0,
                                    pos,
                                    body_pos,
                                    pull,
                                    radius,
                                );
                                view.add_tear(body_pos);
                                sfx.0.push(Sfx::TrapGlueRelease);
                            }
                            if enemies.get(entity).is_ok() {
                                commands.entity(entity).remove::<TrapSlow>();
                            }
                            slot.load = slot.load.saturating_sub(v.units);
                            // Freed for good: the slot goes back to the pool at
                            // once, and the board remembers the body so it is
                            // not caught again until it has walked off the pad.
                            slot.slots[s] = None;
                            if live.is_some() {
                                if let Victim::Enemy(entity) = v.who {
                                    slot.remember_freed(entity);
                                }
                            }
                        }
                    }
                }
            }
        }

        // ---- 2. Catch what this board can still hold -----------------------
        //
        // Every ground body whose centre is on the pad, in `SpawnId` order (the
        // query's storage order never decides; the buffer is reused).
        scratch.cands.clear();
        for (entity, id, e) in enemies.iter() {
            if !e.targetable() || !is_ground(e.kind) {
                continue;
            }
            // Cheap circle reject first (the pad's half-diagonal), then the
            // exact oriented rectangle the art draws.
            if e.pos.dist_sq(pos) > PAD_REACH_SQ {
                continue;
            }
            if !on_pad(pos, facing, e.pos) {
                continue;
            }
            scratch.cands.push(PadCandidate {
                id: id.0,
                entity,
                kind: e.kind,
                named: e.named,
                radius: e.radius,
                pos: e.pos,
            });
        }
        scratch.cands.sort_unstable_by_key(|c| c.id);
        let mut boss_here: Option<Entity> = None;
        for idx in 0..scratch.cands.len() {
            let c = scratch.cands[idx];
            if c.kind == EnemyKind::Boss || c.named {
                // Boss-like: a crossing slow, never a slot and never glue units.
                // One per board at a time (only one exists on the floor).
                boss_here = Some(c.entity);
                continue;
            }
            if scratch.held.contains(&c.entity)
                || slot.touches(c.entity)
                || slot.is_freed(c.entity)
            {
                // Held by this board, cooling in a slot, or a body this board
                // has just let go and which has not walked off the pad yet.
                continue;
            }
            let units = enemy_units(c.kind);
            let room = STICKY_TRAP_CAPACITY.saturating_sub(slot.load);
            if units > room || units > slot.budget_left || slot.free_slot().is_none() {
                // Too big for the room left, or the glue is gone: it only gets
                // the crossing slow, so the horde behind it keeps moving.
                commands.entity(c.entity).insert(TrapSlow(STICKY_TRAP_SLOW_MULT));
                scratch.cross_now.push(c.entity);
                continue;
            }
            let Some(fs) = slot.free_slot() else {
                continue;
            };
            let (hold, mult) = enemy_hold(c.kind);
            // The component is the per-entity slow the steering system
            // multiplies in (`enemy_move` hook); the body is pinned to its slot
            // every tick it is held.
            commands.entity(c.entity).insert(TrapSlow(mult));
            slot.slots[fs] = Some(StuckVictim {
                who: Victim::Enemy(c.entity),
                units,
                remaining: hold,
                hold,
                held: true,
            });
            slot.load = slot.load.saturating_add(units);
            slot.budget_left = slot.budget_left.saturating_sub(units);
            scratch.held.push(c.entity);
            let anchor = sticky_slot_pos(pos, facing, fs, c.radius);
            if let Ok((_, _, mut e)) = enemies.get_mut(c.entity) {
                e.pos = anchor;
                e.vel = Vec2::ZERO;
                vfx::emit_glue_stick(&mut particles, &mut rng.0, pos, anchor, c.radius);
            }
            view.victims[fs] = Some(StickyVictimView {
                pos: anchor,
                radius: c.radius,
                kind: victim_kind(c.kind),
                color: c.kind.color(),
                progress: 0.0,
            });
            sfx.0.push(Sfx::TrapGlueGrab);
        }

        // The rat: it has priority over the horde it is baiting, and a board
        // with a live victim of its own never catches a second rat (there is
        // only one). A rat already held by another board is skipped.
        if !player_glued
            && player_alive
            && !player_dashing
            && !player_dash_now
            && !player_socked
            && !slot.touches_player()
            && (on_pad(pos, facing, player_pos)
                || giant_reaches_box(
                    &player, pos, facing, -STICKY_TRAP_PAD_HALF.x, STICKY_TRAP_PAD_HALF.x,
                    STICKY_TRAP_PAD_HALF.y,
                ))
            && STICKY_TRAP_RAT_UNITS <= STICKY_TRAP_CAPACITY.saturating_sub(slot.load)
            && STICKY_TRAP_RAT_UNITS <= slot.budget_left
        {
            if let Some(fs) = slot.free_slot() {
                slot.slots[fs] = Some(StuckVictim {
                    who: Victim::Player,
                    units: STICKY_TRAP_RAT_UNITS,
                    remaining: STICKY_HOLD_PLAYER_SECS,
                    hold: STICKY_HOLD_PLAYER_SECS,
                    held: true,
                });
                slot.load = slot.load.saturating_add(STICKY_TRAP_RAT_UNITS);
                slot.budget_left = slot.budget_left.saturating_sub(STICKY_TRAP_RAT_UNITS);
                player_glued = true;
                player_glue = STICKY_TRAP_SLOW_MULT;
                vfx::emit_glue_stick(
                    &mut particles,
                    &mut rng.0,
                    pos,
                    player_pos,
                    PLAYER_RADIUS,
                );
                view.victims[fs] = Some(StickyVictimView {
                    pos: player_pos,
                    radius: PLAYER_RADIUS,
                    kind: StickyVictimKind::Rat,
                    color: RAT_GLUE_COLOR,
                    progress: 0.0,
                });
                sfx.0.push(Sfx::TrapGlueGrab);
            }
        }

        // Boss-like bodies: the pre-wave-3 `0.8 s @ 40 %` hold, as a crossing
        // slow with no capacity and no budget.
        match boss_here {
            Some(entity) => {
                commands.entity(entity).insert(TrapSlow(STICKY_SLOW_BOSS));
                slot.boss = Some(BossSlow {
                    entity,
                    remaining: STICKY_HOLD_BOSS_SECS,
                });
            }
            None => {
                if let Some(mut b) = slot.boss {
                    b.remaining -= dt;
                    if b.remaining <= 0.0 || enemies.get(b.entity).is_err() {
                        if enemies.get(b.entity).is_ok() {
                            commands.entity(b.entity).remove::<TrapSlow>();
                        }
                        slot.boss = None;
                    } else {
                        slot.boss = Some(b);
                    }
                }
            }
        }

        // ---- 3. The public state, the load and the wear ---------------------
        view.load = slot.load;
        view.worn = 1.0 - (slot.budget_left as f32 / STICKY_TRAP_BUDGET as f32);
        // Primary victim: the most-progressed one (ties go to the lowest slot,
        // so the choice is fixed by construction).
        let mut primary: Option<StickyVictimView> = None;
        for victim in view.victims.iter().flatten() {
            if primary.is_none_or(|p| victim.progress > p.progress) {
                primary = Some(*victim);
            }
        }
        match primary {
            Some(v) => {
                view.state = StickyTrapState::Stuck;
                view.progress = v.progress;
                view.victim_pos = v.pos;
                view.victim_radius = v.radius;
            }
            None => {
                view.progress = 0.0;
                view.victim_pos = pos;
                view.victim_radius = 3.0;
                if slot.budget_left == 0 {
                    // The budget is gone and the last victim has left: spent.
                    view.state = StickyTrapState::Used;
                    view.progress = 1.0;
                    scratch.spent_sticky.push(i);
                    // A used board leaves the live list, so nothing would
                    // ever run its boss timer down: lift the crossing slow
                    // now or the boss keeps its 40 % for the rest of the fight.
                    if let Some(b) = slot.boss.take() {
                        if enemies.get(b.entity).is_ok() {
                            commands.entity(b.entity).remove::<TrapSlow>();
                        }
                    }
                } else {
                    view.state = StickyTrapState::Idle;
                }
            }
        }
        traps.sticky[i] = slot;
        level.objects.sticky_traps[i] = view;
    }

    // A crossing slow lasts exactly as long as the body is on the pad: take it
    // back off anything that was slowed last tick and is not slowed now (or
    // held, which carries its own slow).
    for &e in traps.cross_prev.iter() {
        if !scratch.cross_now.contains(&e)
            && !scratch.held.contains(&e)
            && enemies.get(e).is_ok()
        {
            commands.entity(e).remove::<TrapSlow>();
        }
    }
    traps.cross_prev.clear();
    traps.cross_prev.extend(scratch.cross_now.iter().copied());

    // ---- Mousetraps ------------------------------------------------------
    for k in 0..traps.live_mice.len() {
        let i = traps.live_mice[k];
        let mut view: MousetrapView = level.objects.mousetraps[i];
        let hair = view.kind.hair();
        match view.state {
            MousetrapState::Armed => {
                // Art `Armed`: the plate is the bait. Taking it is what springs
                // the trap: the reward is collected the instant the rat's centre
                // touches the plate, and the bar lands after the kind's
                // reaction time (zero for a hair trigger).
                let centre = view.pos + safe_dir(view.facing) * MOUSETRAP_TRIGGER_OFFSET_U;
                let reach = MOUSETRAP_TRIGGER_RADIUS + player.buffs.point_hit_radius();
                let on_plate = player_alive && player_pos.dist_sq(centre) <= reach * reach;
                if on_plate && !view.bait_taken && near_bait(view.pos, view.facing, player_pos) {
                    view.bait_taken = true;
                    collect_bait(
                        view.bait,
                        player_pos,
                        &mut commands,
                        &mut next_id,
                    );
                }
                // *Any* ground body — the rat or a member of the horde it is
                // baiting — starts it. Bats fly over, dead bodies have no
                // weight.
                let mut tripped = on_plate;
                if !tripped {
                    tripped = enemies.iter().any(|(_, _, e)| {
                        e.alive() && is_ground(e.kind) && on_bait(view.pos, view.facing, e.pos)
                    });
                }
                if tripped {
                    traps.mice[i].struck.clear();
                    traps.mice[i].player_struck = false;
                    traps.mice[i].hit_done = false;
                    if hair {
                        // Hair trigger: the bar lands on the touch tick. Only a
                        // dash that is *already* under way (its i-frames) or a
                        // body outside the kill rect survives; the swing below
                        // is just the eye catching up.
                        view.state = MousetrapState::Snapping;
                        view.progress = MOUSETRAP_HIT_PROGRESS_MAX;
                        let mouse = &mut traps.mice[i];
                        snap_window(
                            view.pos,
                            view.facing,
                            true,
                            &mut player,
                            player_alive,
                            &mut mouse.player_struck,
                            &mut enemies,
                            &mut mouse.struck,
                            &mut scratch.hits,
                            &mut rng.0,
                            &mut particles,
                            &mut damage_numbers,
                            &mut sfx.0,
                            dmg_scale,
                        );
                        mouse.hit_done = true;
                        // One crack, one splinter burst, one flash, on the tick
                        // it happens.
                        vfx::emit_mousetrap_snap(&mut particles, &mut rng.0, view.pos, view.facing);
                        sfx.0.push(Sfx::TrapSnap);
                        view.hit_flash = 1.0;
                    } else {
                        // Art `Triggered`: the bait pedal dips first (the quiet
                        // click), then the bar lands with the crack at the snap
                        // beat below.
                        view.state = MousetrapState::Triggered;
                        view.progress = 0.0;
                        sfx.0.push(Sfx::TrapTrigger);
                    }
                }
            }
            MousetrapState::Triggered => {
                // Art `Triggered`: `MOUSETRAP_TRIGGER_SECS` of shudder, then the
                // bar releases.
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
                // never skip straight past the hit window — and a hair trigger,
                // which resolved its damage on the touch tick, never hits twice.
                let before = view.progress;
                view.progress = (before + dt / MOUSETRAP_SNAP_SECS).min(1.0);
                if !traps.mice[i].hit_done
                    && view.progress >= MOUSETRAP_HIT_PROGRESS_MIN
                    && before <= MOUSETRAP_HIT_PROGRESS_MAX
                {
                    let pos = view.pos;
                    let facing = view.facing;
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
                        hair,
                        &mut player,
                        player_alive,
                        &mut mouse.player_struck,
                        &mut enemies,
                        &mut mouse.struck,
                        &mut scratch.hits,
                        &mut rng.0,
                        &mut particles,
                        &mut damage_numbers,
                        &mut sfx.0,
                        dmg_scale,
                    );
                    traps.mice[i].hit_done = true;
                }
                if view.progress >= 1.0 {
                    view.state = MousetrapState::Sprung;
                    view.progress = 1.0;
                }
            }
            MousetrapState::Sprung => {
                // Inert, but an untaken bait still lies on the plate, free to
                // take: touching it collects it (and nothing else happens).
                if !view.bait_taken
                    && player_alive
                    && near_bait(view.pos, view.facing, player_pos)
                {
                    view.bait_taken = true;
                    collect_bait(
                        view.bait,
                        player_pos,
                        &mut commands,
                        &mut next_id,
                    );
                }
            }
        }
        level.objects.mousetraps[i] = view;
    }

    // ---- Write the rat's glue back into its movement ----------------------
    if player_glued {
        apply_player_slow(&mut input, &mut player, player_glue);
    }

    // Retire the traps that just became worthless (used board / sprung trap
    // with no bait left to take).
    for &i in scratch.spent_sticky.iter() {
        traps.live_sticky.retain(|&j| j != i);
    }
    traps
        .live_mice
        .retain(|&j| is_live_mouse(&level.objects.mousetraps[j]));
    scratch.spent_sticky.clear();
    scratch.spent_mice.clear();
}

impl StickySlot {
    /// Does this board already hold the rat?
    #[inline]
    fn touches_player(&self) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|v| v.who == Victim::Player)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::art_traps::{
        MousetrapKind, StickyTrapView, TrapBait, STICKY_TRAP_HOLD_SMALL_ENEMY_SECS,
        STICKY_TRAP_HOLD_BRUTE_SECS,
    };
    use crate::game::collision::{TileKind, TileMap};
    use crate::game::ecs::systems::{enemy_move, loot, player};
    use crate::game::art_walls::WallKind;
    use crate::game::gem::PickupKind;
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
        // The masks are what collision, line of sight and the flow field read:
        // a replaced map without them would still see the generated walls.
        w.level.tiles.rebuild_masks();
        w.level.nav = crate::game::nav::NavGrid::from_tiles(
            &w.level.tiles,
            crate::game::collision::SizeClass::Large,
        );
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

    /// `traps_system` plus the pickup pass later in the real schedule, so a
    /// collected bait's *effect* lands (the bait is a normal floor pickup).
    fn traps_and_pickups_tick(w: &mut World) {
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs
            .run_system_cached(traps_system)
            .expect("traps system must run");
        w.ecs
            .run_system_cached(loot::pickups_system)
            .expect("pickup system must run");
        w.ecs
            .run_system_cached(loot::treasure_system)
            .expect("treasure system must run");
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

    fn board(w: &World, i: usize) -> StickyTrapView {
        w.level.objects.sticky_traps[i]
    }

    /// The runtime side table of one board (capacity / budget bookkeeping).
    fn slot(w: &mut World, i: usize) -> StickySlot {
        let t = w.ecs.resource::<TrapRuntime>();
        t.sticky[i]
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

    fn add_mousetrap_spec(w: &mut World, pos: Vec2, kind: MousetrapKind, bait: TrapBait) {
        w.level
            .objects
            .mousetraps
            .push(MousetrapView::baited(pos, FACING, kind, bait));
    }

    fn add_enemy(w: &mut World, kind: EnemyKind, pos: Vec2, hp_mult: f32) -> Entity {
        let mut rng = StdRng::seed_from_u64(7);
        w.spawn_enemy_with_rng(kind, pos, hp_mult, &mut rng)
    }

    fn enemy_of(w: &mut World, entity: Entity) -> Enemy {
        let mut q = w.ecs.query::<&Enemy>();
        q.get(&w.ecs, entity).expect("enemy must exist").clone()
    }

    fn has_slow(w: &mut World, e: Entity) -> bool {
        let mut q = w.ecs.query::<&TrapSlow>();
        q.get(&w.ecs, e).is_ok()
    }

    fn pot_world() -> (World, Vec2) {
        let mut w = World::with_level(
            0, Level::empty(Theme::Outfall), 163,
            crate::game::loadout::Loadout::starting(), Default::default(),
        );
        let c = w.player.pos;
        w.peace = true;
        w.player.loadout.weapons.clear();
        w.level.objects.crab_pots.push(set_crab_pot(c, FACING));
        (w, c)
    }

    fn pot(w: &World) -> CrabPotView {
        w.level.objects.crab_pots[0]
    }

    fn pot_ticks(w: &mut World, ticks: usize) {
        for _ in 0..ticks {
            traps_tick(w, Vec2::ZERO, false);
        }
    }

    #[test]
    fn crab_pot_warns_then_snaps_empty_opens_and_resets_without_rng() {
        use rand::RngExt;
        use CrabPotState::*;
        let (mut w, c) = pot_world();
        w.rng = StdRng::seed_from_u64(23);
        let mut expected_rng = StdRng::seed_from_u64(23);
        assert_eq!(pot(&w).state, Set);
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, Closing);
        assert_eq!(pot(&w).progress, 0.0);
        assert_eq!(sfx_count(&w, Sfx::TrapTrigger), 1);
        w.player.pos = c + FACING * (POT_HALF + 0.5);
        pot_ticks(&mut w, 20);
        assert_eq!(pot(&w).state, Closing);
        assert!(pot(&w).progress > 0.9);
        w.sfx.clear();
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, Shut);
        assert_eq!(sfx_count(&w, Sfx::TrapSnap), 1);
        assert!(!pot(&w).caught_rat);
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, Open);
        pot_ticks(&mut w, 1199);
        assert_eq!(pot(&w).state, Open);
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, Set);
        assert_eq!(w.rng.random::<u64>(), expected_rng.random::<u64>());
    }

    #[test]
    fn empty_crab_pot_releases_one_crab_at_its_mouth() {
        let (mut w, c) = pot_world();
        w.peace = false;
        pot_ticks(&mut w, 1);
        w.player.pos = c - FACING * 25.0;
        pot_ticks(&mut w, 21);
        assert_eq!(pot(&w).state, CrabPotState::Shut);
        assert!(w.enemies().is_empty());
        pot_ticks(&mut w, 2);
        assert_eq!(pot(&w).state, CrabPotState::Open);
        let bodies = w.enemies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].kind, EnemyKind::ShoreCrab);
        assert_eq!(bodies[0].pos, c + FACING * (POT_HALF + DOOR_DEPTH));
    }

    #[test]
    fn crab_pot_holds_the_rat_and_an_actual_dash_breaks_free_after_a_quarter_second() {
        let (mut w, c) = pot_world();
        let hp = w.player.hp;
        pot_ticks(&mut w, 22);
        assert_eq!(w.player.hp, hp - POT_RAT_DAMAGE);
        assert!(pot(&w).caught_rat);
        w.update(DT, FACING, true);
        assert_eq!(w.player.pos, c);
        assert_eq!(w.player.dash_cd, 0.0, "the early press cannot spend a dash");
        for _ in 0..13 {
            w.update(DT, FACING, false);
        }
        assert_eq!(w.player.pos, c);
        w.update(DT, FACING, true);
        assert!(!pot(&w).caught_rat);
        assert_eq!(pot(&w).state, CrabPotState::Open);
        assert!(w.player.pos.x > c.x);
    }

    #[test]
    fn crab_pot_respects_damage_protection_and_holds_a_shrunken_rat_for_eight_tenths() {
        use crate::game::player::Buffs;
        for (buffs, iframes, expected_damage, expected_shield) in [
            (Buffs { tiny: 12.0, ..Buffs::default() }, 0.0, POT_RAT_DAMAGE, 0.0),
            (Buffs { shield: 12.0, ..Buffs::default() }, 0.0, 0.0, 0.0),
            (Buffs { white_coat: 5.0, shield: 12.0, ..Buffs::default() }, 0.0, 0.0, 12.0),
            (Buffs { shield: 12.0, ..Buffs::default() }, 1.0, 0.0, 12.0),
        ] {
            let (mut w, _) = pot_world();
            w.player.buffs = buffs;
            w.player.iframes = iframes;
            let hp = w.player.hp;
            pot_ticks(&mut w, 22);
            assert_eq!(w.player.hp, hp - expected_damage);
            assert_eq!(w.player.buffs.shield, expected_shield);
            assert!(pot(&w).caught_rat);
            pot_ticks(&mut w, 47);
            assert!(pot(&w).caught_rat);
            pot_ticks(&mut w, 1);
            assert_eq!(pot(&w).state, CrabPotState::Open);
            assert!(!pot(&w).caught_rat);
        }
    }

    #[test]
    fn crab_pot_holds_only_four_bodies_in_spawn_order_and_they_cannot_bite() {
        let (mut w, c) = pot_world();
        w.player.pos = c + FACING * 100.0;
        let bodies: [_; 5] = std::array::from_fn(|_| add_enemy(&mut w, EnemyKind::SandHopper, c, 1.0));
        pot_ticks(&mut w, 22);
        assert_eq!(pot(&w).caught, 4);
        for (i, entity) in bodies.into_iter().enumerate() {
            let e = w.ecs.get::<Enemy>(entity).unwrap();
            assert_eq!(e.crab_pot_held, i < 4);
            assert_eq!(e.hp, if i < 4 { e.max_hp - e.max_hp * POT_HORDE_DAMAGE_SHARE } else { e.max_hp });
        }
        // The fifth body is free to bite, so put it beyond contact range.
        w.ecs.get_mut::<Enemy>(bodies[4]).unwrap().pos = w.player.pos;
        w.player.pos = c;
        let hp = w.player.hp;
        w.update(DT, Vec2::ZERO, false);
        assert_eq!(w.player.hp, hp);
        for entity in bodies.into_iter().take(4) {
            assert_eq!(w.ecs.get::<Enemy>(entity).unwrap().pos, c);
        }
        pot_ticks(&mut w, 178);
        assert_eq!(pot(&w).caught, 4);
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, CrabPotState::Open);
        for entity in bodies {
            assert!(!w.ecs.get::<Enemy>(entity).unwrap().crab_pot_held);
        }
    }

    #[test]
    fn crab_pot_excludes_large_elite_named_and_boss_bodies_and_the_giant_crushes_it() {
        let (mut w, c) = pot_world();
        w.player.pos = c + FACING * 100.0;
        let elite = add_enemy(&mut w, EnemyKind::SandHopper, c, 1.0);
        w.ecs.get_mut::<Enemy>(elite).unwrap().elite = true;
        let named = add_enemy(&mut w, EnemyKind::SandHopper, c, 1.0);
        w.ecs.get_mut::<Enemy>(named).unwrap().named = true;
        let bodies = [
            elite, named, add_enemy(&mut w, EnemyKind::Brute, c, 1.0),
            add_enemy(&mut w, EnemyKind::FrySchool, c, 1.0),
            add_enemy(&mut w, EnemyKind::Boss, c, 1.0),
        ];
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, CrabPotState::Set);
        w.player.pos = c;
        pot_ticks(&mut w, 22);
        assert_eq!(pot(&w).caught, 0);
        assert!(pot(&w).caught_rat);
        for entity in bodies {
            let e = w.ecs.get::<Enemy>(entity).unwrap();
            assert_eq!(e.hp, e.max_hp);
            assert!(!e.crab_pot_held);
        }
        let hp = w.player.hp;
        w.player.buffs.giant = 20.0;
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, CrabPotState::Open);
        assert!(!pot(&w).caught_rat);
        assert_eq!(w.player.hp, hp);
        // A giant approaching a fresh pot skips the warning too.
        w.level.objects.crab_pots[0] = set_crab_pot(c, FACING);
        pot_ticks(&mut w, 1);
        assert_eq!(pot(&w).state, CrabPotState::Open);
        assert!(!pot(&w).caught_rat);
    }

    #[test]
    fn crab_pot_kill_goes_through_the_normal_loot_path() {
        use rand::RngExt;
        let (mut w, c) = pot_world();
        w.player.pos = c + FACING * 100.0;
        let weak = add_enemy(&mut w, EnemyKind::SandHopper, c, 1.0);
        w.ecs.get_mut::<Enemy>(weak).unwrap().hp = 0.5;
        w.rng = StdRng::seed_from_u64(23);
        let mut expected_rng = StdRng::seed_from_u64(23);
        pot_ticks(&mut w, 22);
        assert_eq!(w.rng.random::<u64>(), expected_rng.random::<u64>());
        assert!(!w.ecs.get::<Enemy>(weak).unwrap().alive());
        w.begin_tick(DT, Vec2::ZERO, false);
        w.ecs.run_system_cached(loot::reap_enemies_system).unwrap();
        w.end_tick();
        assert_eq!(w.kills, 1);
        assert!(!w.gems().is_empty());
        assert!(w.ecs.get::<Enemy>(weak).is_none());
    }

    /// How many bodies of `kind` are currently held on board `i`.
    fn held_on(w: &mut World, i: usize, kind: EnemyKind) -> usize {
        let who: Vec<Entity> = slot(w, i)
            .slots
            .iter()
            .flatten()
            .filter(|v| v.held)
            .filter_map(|v| match v.who {
                Victim::Enemy(e) => Some(e),
                Victim::Player => None,
            })
            .collect();
        let mut q = w.ecs.query::<&Enemy>();
        who.iter()
            .filter(|e| q.get(&w.ecs, **e).is_ok_and(|en| en.kind == kind))
            .count()
    }

    // ---- Sticky boards: the rat -------------------------------------------

    /// The rat is held for the designer's 0.9 s, costs 2 glue units, and the
    /// board is **not** spent: it goes back to a (worn) fresh board, ready for
    /// the next victim.
    #[test]
    fn glue_holds_the_rat_for_0_9s_and_then_releases_it() {
        let mut w = open_arena(101);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;

        // Tick 1 catches the rat.
        traps_tick(&mut w, FACING, false);
        {
            let v = board(&w, 0);
            assert_eq!(v.state, StickyTrapState::Stuck);
            assert!(v.progress.abs() < 1e-6, "a fresh catch starts at progress 0");
            assert_eq!(v.victim_radius, PLAYER_RADIUS);
            assert!((v.victim_pos.dist(w.player.pos)) < 1e-4);
            assert_eq!(v.load, STICKY_TRAP_RAT_UNITS, "the rat costs 2 units");
        }
        assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET - 2);

        // Hold exactly the designer's 0.9 s (the state machine steps in whole
        // frames, so allow a frame either side of the nominal tick count).
        let mut held = 0usize;
        let mut last_progress = 0.0f32;
        for _ in 0..1_000 {
            if board_state(&w, 0) != StickyTrapState::Stuck {
                break;
            }
            last_progress = board(&w, 0).progress;
            traps_tick(&mut w, FACING, false);
            held += 1;
        }
        assert_eq!(
            board_state(&w, 0),
            StickyTrapState::Idle,
            "a board with glue left is fresh again after a release"
        );
        let held_secs = held as f32 * DT;
        assert!(
            (held_secs - STICKY_HOLD_PLAYER_SECS).abs() <= 2.0 * DT,
            "the rat was held {held_secs} s, expected {STICKY_HOLD_PLAYER_SECS} s"
        );
        assert!(
            (0.9..1.0).contains(&last_progress),
            "strand progress just before the release = {last_progress}"
        );
        let v = board(&w, 0);
        assert_eq!(v.load, 0, "the rat's units came back");
        assert!(v.worn > 0.0, "the spent units are still spent");
        assert_eq!(v.tear_n, 1, "a torn scar marks where it pulled free");
        // The art keeps drawing the board where it was.
        assert!((v.pos - c).len() < 1e-4);
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

    /// A dash breaks the rat out immediately (the art's `Stuck` counter), a
    /// dash in progress crosses the pad without sticking, and Clean Socks
    /// ignore the glue entirely. None of them spends a board.
    #[test]
    fn dash_and_clean_socks_break_glue_without_spending_the_board() {
        let mut w = open_arena(107);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.dash_cd = 0.0;

        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);

        // Press dash: the board releases instead of holding.
        traps_tick(&mut w, FACING, true);
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
        assert_eq!(board(&w, 0).load, 0);
        assert!(
            slot(&mut w, 0).budget_left < STICKY_TRAP_BUDGET,
            "the catch still wore the board"
        );

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
        assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET);

        // Clean Socks: an armed socked rat walks over the pad untouched...
        let mut w = open_arena(127);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        w.player.buffs.socks = 3.0;
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
        // ...and is let go of a board it is already stuck to.
        let mut w = open_arena(131);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        w.player.buffs.socks = 3.0;
        traps_tick(&mut w, FACING, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
    }

    // ---- Sticky boards: capacity by size class -----------------------------

    /// The owner's size classes: small bodies 1 unit, Cat/Spitter/Pillbug/
    /// Porter/Turnkey 2, Brute (and an elite Brute) 4, the rat 2, boss-likes 0.
    #[test]
    fn capacity_is_taken_by_size_class() {
        assert_eq!(enemy_units(EnemyKind::Skitterer), 1);
        assert_eq!(enemy_units(EnemyKind::BellFlea), 1);
        assert_eq!(enemy_units(EnemyKind::Bat), 1, "a ground bat would be small");
        assert_eq!(enemy_units(EnemyKind::Cat), 2);
        assert_eq!(enemy_units(EnemyKind::Spitter), 2);
        assert_eq!(enemy_units(EnemyKind::Pillbug), 2);
        assert_eq!(enemy_units(EnemyKind::Porter), 2);
        assert_eq!(enemy_units(EnemyKind::Turnkey), 2);
        assert_eq!(enemy_units(EnemyKind::Brute), 4);
        assert_eq!(enemy_units(EnemyKind::Boss), 0, "never held, never capacity");
        assert_eq!(STICKY_TRAP_RAT_UNITS, 2);
        assert_eq!(STICKY_TRAP_CAPACITY, 8);
        assert_eq!(STICKY_TRAP_BUDGET, 20);
        assert!(
            STICKY_TRAP_SLOTS as u8 >= STICKY_TRAP_CAPACITY,
            "the slot array must hold a full load of 1-unit bodies"
        );
    }

    #[test]
    fn outfall_glue_uses_body_weight_and_exempts_swimmers() {
        assert_eq!(enemy_units(EnemyKind::SandHopper), 1);
        assert_eq!(enemy_units(EnemyKind::Skater), 1);
        assert_eq!(enemy_units(EnemyKind::Conger), 2);
        assert_eq!(enemy_units(EnemyKind::PistolShrimp), 2);
        assert_eq!(enemy_units(EnemyKind::FrySchool), 2);
        assert_eq!(enemy_units(EnemyKind::Grouper), 4);
        assert!(is_ground(EnemyKind::FrySchool));
        assert!(!is_ground(EnemyKind::GlassShrimp));
        assert!(!is_ground(EnemyKind::Lampfish));
    }

    /// A mixed crowd fills a board exactly: 4 + 2 + 2 units fit, the next small
    /// body overflows with only the crossing slow, and killing one frees its
    /// units for the body waiting on the pad.
    #[test]
    fn a_mixed_crowd_fills_a_board_and_overflows_it() {
        let mut w = open_arena(211);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        // 4 + 2 + 2 = 8 units: exactly full.
        let brute = add_enemy(&mut w, EnemyKind::Brute, c + Vec2::new(-4.0, 0.0), 20.0);
        let cat = add_enemy(&mut w, EnemyKind::Cat, c + Vec2::new(2.0, -3.0), 20.0);
        let spitter = add_enemy(&mut w, EnemyKind::Spitter, c + Vec2::new(3.0, 3.0), 20.0);
        // Waiting just **off** the pad, then stepping on it once there is room.
        let waiting = add_enemy(&mut w, EnemyKind::Skitterer, c + Vec2::new(22.0, 0.0), 20.0);

        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board(&w, 0).load, STICKY_TRAP_CAPACITY);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Brute), 1);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Cat), 1);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Spitter), 1);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 0);
        assert!(!has_slow(&mut w, waiting), "an overflow body is only slowed while on the pad");
        let budget_after_fill = slot(&mut w, 0).budget_left;
        assert_eq!(budget_after_fill, STICKY_TRAP_BUDGET - STICKY_TRAP_CAPACITY);

        // Now a small body walks onto the full pad: it is slowed, not held.
        if let Ok(mut e) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, waiting) {
            e.pos = c + Vec2::new(4.0, 7.0);
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert!(has_slow(&mut w, waiting), "the overflow body crosses slowly");
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 0);
        assert_eq!(board(&w, 0).state, StickyTrapState::Stuck);

        // The brute is killed on the board: its 4 units come free at once, and
        // the body still standing on the pad is caught.
        if let Ok(mut e) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, brute) {
            e.hp = -1.0;
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(
            board(&w, 0).load,
            5,
            "2 + 2 held and the freed room taken by the waiting body"
        );
        assert_eq!(
            held_on(&mut w, 0, EnemyKind::Skitterer),
            1,
            "the waiting body walked into the freed room"
        );
        let _ = (cat, spitter);
    }

    /// The documented rule is "cannot be re-caught until it has walked off": a
    /// body jammed on the pad by the crowd is not caught again, however long it
    /// stays (the old time-only grace re-caught it 1.2 s after release and spent
    /// the board's budget on it a second time). Once it leaves the pad its
    /// immunity ends and stepping back on is a fresh catch.
    #[test]
    fn a_freed_body_is_not_caught_again_until_it_has_walked_off() {
        let mut w = open_arena(251);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        // A small crowd on the pad. Nothing moves a body in this narrow
        // harness, which is exactly the jam the rule is about.
        let bodies: Vec<Entity> = [-5.0f32, 0.0, 5.0]
            .iter()
            .map(|dx| add_enemy(&mut w, EnemyKind::Skitterer, c + Vec2::new(*dx, 2.0), 40.0))
            .collect();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 3);
        assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET - 3);

        // Long past the hold *and* well past the old grace, all three are free
        // and still standing on the pad: none is caught again and the budget
        // does not move.
        let ticks = ((STICKY_TRAP_HOLD_SMALL_ENEMY_SECS + 3.0 * STICKY_TRAP_RELEASE_GRACE_SECS) / DT)
            as usize
            + 4;
        for _ in 0..ticks {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 0);
        assert_eq!(
            slot(&mut w, 0).budget_left,
            STICKY_TRAP_BUDGET - 3,
            "a body jammed on the pad spent the budget a second time"
        );
        assert!(
            !has_slow(&mut w, bodies[0]),
            "a freed body on the pad is immune, not slowed"
        );

        // The crowd breaks up. One body walks off the pad (that is what clears
        // the board's memory of it), then steps back on: a fresh catch.
        if let Ok(mut e) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, bodies[1]) {
            e.pos = c + Vec2::new(0.0, 60.0);
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        if let Ok(mut e) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, bodies[1]) {
            e.pos = c + Vec2::new(1.0, 1.0);
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(
            held_on(&mut w, 0, EnemyKind::Skitterer),
            1,
            "walking off the pad must clear the immunity"
        );
        assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET - 4);
    }

    /// Each victim keeps its own class hold and tears free on its own clock,
    /// and a big body too large for the room left is never caught.
    #[test]
    fn victims_hold_and_release_on_their_own_clock() {
        let mut w = open_arena(223);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        // A small body and a Brute: different holds (2.5 s / 1.5 s).
        add_enemy(&mut w, EnemyKind::Skitterer, c + Vec2::new(-6.0, -4.0), 20.0);
        let brute = add_enemy(&mut w, EnemyKind::Brute, c + Vec2::new(4.0, 4.0), 20.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board(&w, 0).load, 5);
        assert_eq!(board(&w, 0).state, StickyTrapState::Stuck);

        // After 1.5 s the Brute has torn free, the Skitterer is still held.
        for _ in 0..(STICKY_TRAP_HOLD_BRUTE_SECS / DT).ceil() as usize + 3 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(held_on(&mut w, 0, EnemyKind::Brute), 0, "the Brute is free");
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 1);
        assert!(!has_slow(&mut w, brute), "the released Brute lost its slow");
        assert_eq!(board(&w, 0).load, 1);
        // The freed Brute walks off the pad: that is what clears the board's
        // memory of it. (One left standing on the pad is *not* caught again —
        // see `a_freed_body_is_not_caught_again_until_it_has_walked_off`.)
        if let Ok(mut e) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, brute) {
            e.pos = c + Vec2::new(0.0, 80.0);
        }

        // The Skitterer follows on its own (longer) clock: the rest of its
        // 2.5 s, measured from the same catch. (Not a second full hold: it is
        // still standing on the pad, and the release grace would have run out.)
        let rest = STICKY_TRAP_HOLD_SMALL_ENEMY_SECS - STICKY_TRAP_HOLD_BRUTE_SECS;
        assert!(rest > 0.0 && rest < STICKY_TRAP_RELEASE_GRACE_SECS);
        for _ in 0..(rest / DT).ceil() as usize + 2 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(board(&w, 0).load, 0);
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);

        // Overflow by size: 3 units of room, a 4-unit Brute is only slowed.
        let mut w = open_arena(227);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        for off in [-6.0f32, -2.0, 2.0] {
            add_enemy(&mut w, EnemyKind::Cat, c + Vec2::new(off, 6.0), 20.0);
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board(&w, 0).load, 6, "three Cats");
        let big = add_enemy(&mut w, EnemyKind::Brute, c + Vec2::new(0.0, -6.0), 20.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Brute), 0, "2 units of room, a Brute needs 4");
        assert!(has_slow(&mut w, big), "it crosses slowly instead");
        assert_eq!(board(&w, 0).load, 6);
    }

    /// A board wears out by total glue spent: it catches a dozen small bodies
    /// over its life, then becomes `Used` only after its last victim leaves.
    #[test]
    fn the_budget_wears_the_board_out_and_used_waits_for_the_last_victim() {
        let mut w = open_arena(229);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);

        // Five waves of four Skitterers: 20 units, the whole budget.
        let mut caught = 0usize;
        for wave in 0..6 {
            for k in 0..4 {
                let a = (wave * 4 + k) as f32 * 1.4;
                let at = c + Vec2::new(crate::detmath::cos(a) * 6.0, crate::detmath::sin(a) * 5.0);
                add_enemy(&mut w, EnemyKind::Skitterer, at, 40.0);
            }
            traps_tick(&mut w, Vec2::ZERO, false);
            caught += held_on(&mut w, 0, EnemyKind::Skitterer);
            assert!(
                budget_floor_is_monotone(&mut w),
                "the budget went back up"
            );
            // Let the whole wave tear free before the next one arrives.
            for _ in 0..(STICKY_TRAP_HOLD_SMALL_ENEMY_SECS / DT).ceil() as usize + 2 {
                traps_tick(&mut w, Vec2::ZERO, false);
            }
        }
        assert!(caught >= 12, "only {caught} bodies were caught");
        assert_eq!(slot(&mut w, 0).budget_left, 0, "the budget is spent");
        assert_eq!(
            board_state(&w, 0),
            StickyTrapState::Used,
            "with no victims left, a spent board is Used"
        );
        assert!((board(&w, 0).worn - 1.0).abs() < 1e-6);

        // A board whose budget runs out while it still holds a body stays
        // `Stuck` (it is still doing its job), then turns Used on the release.
        let mut w = open_arena(233);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        traps_tick(&mut w, Vec2::ZERO, false); // build the runtime table
        {
            // Force a nearly spent board through the runtime table.
            let mut t = w.ecs.resource_mut::<TrapRuntime>();
            t.sticky[0].budget_left = 1;
        }
        let e = add_enemy(&mut w, EnemyKind::Skitterer, c, 40.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(slot(&mut w, 0).budget_left, 0);
        assert_eq!(board_state(&w, 0), StickyTrapState::Stuck);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 1);
        // Another body on the pad gets no glue at all.
        let overflow = add_enemy(&mut w, EnemyKind::Skitterer, c + Vec2::new(0.0, 6.0), 40.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 1, "no glue to catch with");
        assert!(has_slow(&mut w, overflow), "it only crosses slowly");
        // The held body is killed: its corpse frees the pad and the board is done.
        if let Ok(mut en) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, e) {
            en.hp = -1.0;
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
    }

    /// The budget never rises: catch after catch only ever spends it.
    fn budget_floor_is_monotone(w: &mut World) -> bool {
        let left = slot(w, 0).budget_left;
        left <= STICKY_TRAP_BUDGET
    }

    /// The rat costs 2 units like anything else and does not use the board up.
    #[test]
    fn the_rat_costs_two_units_not_the_whole_board() {
        let mut w = open_arena(239);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c;
        traps_tick(&mut w, FACING, false);
        assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET - STICKY_TRAP_RAT_UNITS);
        // Nine more rat catches fit in the lifetime budget (2 units each).
        let mut rat_catches = 1;
        for _ in 0..40 {
            // Walk off the pad, let the grace expire, walk back on.
            w.player.pos = c + Vec2::new(60.0, 0.0);
            for _ in 0..((STICKY_TRAP_RELEASE_GRACE_SECS + 0.2) / DT) as usize {
                traps_tick(&mut w, Vec2::ZERO, false);
            }
            w.player.pos = c;
            traps_tick(&mut w, Vec2::ZERO, false);
            if board_state(&w, 0) == StickyTrapState::Stuck {
                rat_catches += 1;
                for _ in 0..((STICKY_HOLD_PLAYER_SECS + 0.2) / DT) as usize {
                    traps_tick(&mut w, Vec2::ZERO, false);
                }
            }
            if slot(&mut w, 0).budget_left == 0 {
                break;
            }
        }
        assert!(
            rat_catches >= 9,
            "a board only caught the rat {rat_catches} times over its life"
        );
        assert_eq!(slot(&mut w, 0).budget_left, 0);
    }

    /// Bosses and named minibosses are slowed while they cross a board (the
    /// old 0.8 s @ 40 % rule) and never held: no slot, no glue unit, no budget.
    #[test]
    fn boss_likes_are_slowed_but_never_stuck() {
        for named in [false, true] {
            let mut w = open_arena(241);
            let c = centre(&w);
            add_board(&mut w, c);
            w.player.pos = c + Vec2::new(400.0, 0.0);
            let e = if named {
                w.spawn_miniboss(
                    crate::game::ecs::systems::miniboss::MinibossKind::Corkscrew,
                    0,
                    c,
                )
            } else {
                add_enemy(&mut w, EnemyKind::Boss, c, 40.0)
            };
            traps_tick(&mut w, Vec2::ZERO, false);
            assert_eq!(board(&w, 0).load, 0, "a boss takes no capacity");
            assert_eq!(slot(&mut w, 0).budget_left, STICKY_TRAP_BUDGET);
            assert_eq!(board(&w, 0).state, StickyTrapState::Idle, "never Stuck");
            let mut q = w.ecs.query::<&TrapSlow>();
            let slow = q
                .get(&w.ecs, e)
                .expect("a boss on the pad is slowed")
                .0;
            assert!((slow - STICKY_SLOW_BOSS).abs() < 1e-6);
            // Standing in the glue keeps it slowed (the glue is still there)...
            for _ in 0..((STICKY_HOLD_BOSS_SECS + 0.2) / DT) as usize {
                traps_tick(&mut w, Vec2::ZERO, false);
            }
            let mut q = w.ecs.query::<&TrapSlow>();
            assert!(
                q.get(&w.ecs, e).is_ok(),
                "a boss standing in the glue lost the slow"
            );
            // ...and it shrugs the slow off within the hold once it steps clear.
            if let Ok(mut en) = w.ecs.query::<&mut Enemy>().get_mut(&mut w.ecs, e) {
                en.pos = c + Vec2::new(0.0, 60.0);
            }
            for _ in 0..((STICKY_HOLD_BOSS_SECS + 0.2) / DT) as usize {
                traps_tick(&mut w, Vec2::ZERO, false);
            }
            let mut q = w.ecs.query::<&TrapSlow>();
            assert!(q.get(&w.ecs, e).is_err(), "the boss kept the glue");
        }
    }

    /// Stuck bodies are ordinary enemies at ordinary positions: an area effect
    /// (a Nuke) kills them like any other body, and a weapon's dart reaches
    /// them where they are pinned.
    #[test]
    fn weapons_and_area_effects_reach_stuck_bodies() {
        // Nuke: every stuck body dies through the normal area-effect path.
        let mut w = open_arena(257);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(10.0, 0.0);
        // Three 2-unit bodies (6 of the board's 8 units, with the rat's own 2
        // beside them): three 4-unit Brutes would not all fit on one board.
        for off in [-5.0f32, 0.0, 5.0] {
            add_enemy(&mut w, EnemyKind::Cat, c + Vec2::new(0.0, off), 20.0);
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Cat), 3);
        w.spawn_pickup(Pickup {
            pos: w.player.pos,
            kind: PickupKind::Nuke,
            bob: 0.0,
        });
        for _ in 0..4 {
            w.update(DT, Vec2::ZERO, false);
        }
        assert!(
            w.enemies().iter().all(|e| !e.alive()),
            "the Nuke spared the glued crowd"
        );

        // Weapons: the rat stands beside a board holding one small body; over a
        // couple of seconds the dart has to land on it (a stuck body is just an
        // enemy at a fixed position).
        let mut w = open_arena(259);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(24.0, 0.0);
        let e = add_enemy(&mut w, EnemyKind::Skitterer, c, 1.0); // 6 hp
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 1);
        for _ in 0..240 {
            w.update(DT, Vec2::ZERO, false);
            if w.kills > 0 {
                break;
            }
        }
        assert!(w.kills > 0, "no weapon ever reached the stuck body");
        let mut q = w.ecs.query::<&Enemy>();
        assert!(q.get(&w.ecs, e).is_err(), "the stuck body survived");
    }

    /// A stuck crowd is a slow, not a wall: the horde behind a loaded board
    /// funnels down a three-cell corridor and gets past it to the rat.
    #[test]
    fn a_stuck_crowd_does_not_block_a_three_cell_corridor() {
        let mut w = World::new(Theme::Sewer, 263);
        w.level.objects.sticky_traps.clear();
        w.level.objects.mousetraps.clear();
        let mut tiles = TileMap::new(MAP_COLS, MAP_ROWS, Theme::Sewer, 0);
        let cy = MAP_ROWS / 2;
        let (x0, x1) = (20, MAP_COLS - 20);
        tiles.fill_cells(0, 0, MAP_COLS, MAP_ROWS, TileKind::Wall(WallKind::Solid));
        tiles.fill_cells(x0, cy - 1, x1 - x0, 3, TileKind::Open);
        tiles.rebuild_masks();
        w.level.tiles = tiles;
        w.level.nav = crate::game::nav::NavGrid::from_tiles(
            &w.level.tiles,
            crate::game::collision::SizeClass::Large,
        );
        // The flow field keeps its own masks of the grid it was built for: a
        // replaced map needs a new one, or the horde routes round the
        // generated level's walls instead of down this corridor.
        w.flow = crate::game::nav::FlowField::new(&w.level.tiles);

        let rat = crate::game::collision::TileMap::tile_rect(x1 - 4, cy).center();
        let board_at = crate::game::collision::TileMap::tile_rect(x0 + 14, cy).center();
        w.player.pos = rat;
        add_board(&mut w, board_at);
        // Fill the board with four Skitterers (pinned there).
        for off in [-2.0f32, 2.0, -1.0, 1.0] {
            add_enemy(&mut w, EnemyKind::Skitterer, board_at + Vec2::new(0.0, off), 40.0);
        }
        // The horde behind them, still in the corridor.
        let mut followers = Vec::new();
        for k in 0..6 {
            let at = crate::game::collision::TileMap::tile_rect(x0 + 2 + k % 2, cy + (k % 3) - 1)
                .center();
            followers.push(add_enemy(&mut w, EnemyKind::Skitterer, at, 40.0));
        }
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(held_on(&mut w, 0, EnemyKind::Skitterer), 4);

        let start: Vec<f32> = followers.iter().map(|e| enemy_of(&mut w, *e).pos.x).collect();
        // Twenty seconds: long enough for every follower to reach the board,
        // be caught or slowed in its turn (the board has budget for all ten
        // bodies) and walk out the far side. Getting *onto* the pad is not
        // getting past it.
        for _ in 0..1200 {
            w.begin_tick(DT, Vec2::ZERO, false);
            w.ecs.run_system_cached(traps_system).unwrap();
            w.ecs.run_system_cached(enemy_move::flow_refresh_system).unwrap();
            w.ecs
                .run_system_cached(enemy_move::enemy_steering_system)
                .unwrap();
            // The real movement chain, not steering alone: separation is the
            // only thing that could make a pinned crowd push the horde back,
            // and the wall resolve is what makes the corridor a corridor.
            w.ecs.run_system_cached(enemy_move::grid_build_system).unwrap();
            w.ecs.run_system_cached(enemy_move::separation_system).unwrap();
            w.ecs.run_system_cached(enemy_move::wall_resolve_system).unwrap();
            w.end_tick();
        }
        let rat_x = rat.x;
        let mut passed = 0usize;
        for (e, s) in followers.iter().zip(start) {
            if let Ok(en) = w.ecs.query::<&Enemy>().get(&w.ecs, *e) {
                // Out the far side of the pad and on the way to the rat: the
                // board and its crowd are not a wall.
                let _ = s;
                if en.pos.x > board_at.x + STICKY_TRAP_PAD_HALF.x + 6.0 && en.pos.x < rat_x + 16.0 {
                    passed += 1;
                }
            } else {
                // Killed or gone is also "not blocked".
                passed += 1;
            }
        }
        assert!(
            passed >= 5,
            "only {passed} of {} bodies got past the stuck crowd",
            followers.len()
        );
    }

    /// One catch per body, one body per slot, whichever board is overlapped.
    #[test]
    fn overlapping_boards_never_share_a_body() {
        let mut w = open_arena(269);
        let c = centre(&w);
        add_board(&mut w, c);
        add_board(&mut w, c + Vec2::new(2.0, 0.0));
        w.player.pos = c + Vec2::new(400.0, 0.0);
        let a = add_enemy(&mut w, EnemyKind::Skitterer, c, 40.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        let second = slot(&mut w, 1).slots.iter().flatten().count();
        assert_eq!(
            second, 0,
            "the second board caught a body that was already held"
        );
        assert!(has_slow(&mut w, a));
    }

    /// The catch radius and the four cardinal facings still hold on the pad
    /// edges and corners.
    #[test]
    fn sticky_pad_catches_at_edges_in_all_four_facings() {
        let px = STICKY_TRAP_PAD_HALF.x;
        let py = STICKY_TRAP_PAD_HALF.y;
        let facings = [
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, -1.0),
        ];

        for facing in facings {
            let f = facing.normalized();
            let p = f.perp();
            let inside_offsets = [
                f * (px - 0.25),
                -f * (px - 0.25),
                p * (py - 0.25),
                -p * (py - 0.25),
                f * (px - 0.25) + p * (py - 0.25),
                -f * (px - 0.25) - p * (py - 0.25),
            ];
            for off in inside_offsets {
                let mut w = open_arena(301);
                let c = centre(&w);
                w.level
                    .objects
                    .sticky_traps
                    .push(StickyTrapView::new(c, facing));
                w.player.pos = c + off;
                traps_tick(&mut w, Vec2::ZERO, false);
                assert_eq!(
                    board_state(&w, 0),
                    StickyTrapState::Stuck,
                    "player should stick at offset {off:?} for facing {facing:?}"
                );

                let mut w = open_arena(302);
                let c = centre(&w);
                w.level
                    .objects
                    .sticky_traps
                    .push(StickyTrapView::new(c, facing));
                w.player.pos = c + Vec2::new(300.0, 0.0);
                add_enemy(&mut w, EnemyKind::Skitterer, c + off, 1.0);
                traps_tick(&mut w, Vec2::ZERO, false);
                assert_eq!(
                    board_state(&w, 0),
                    StickyTrapState::Stuck,
                    "enemy should stick at offset {off:?} for facing {facing:?}"
                );
            }

            let outside_offsets = [
                f * (px + 0.25),
                -f * (px + 0.25),
                p * (py + 0.25),
                -p * (py + 0.25),
            ];
            for off in outside_offsets {
                let mut w = open_arena(303);
                let c = centre(&w);
                w.level
                    .objects
                    .sticky_traps
                    .push(StickyTrapView::new(c, facing));
                w.player.pos = c + off;
                traps_tick(&mut w, Vec2::ZERO, false);
                assert_eq!(
                    board_state(&w, 0),
                    StickyTrapState::Idle,
                    "player should NOT stick outside pad at {off:?} for facing {facing:?}"
                );

                let mut w = open_arena(304);
                let c = centre(&w);
                w.level
                    .objects
                    .sticky_traps
                    .push(StickyTrapView::new(c, facing));
                w.player.pos = c + Vec2::new(300.0, 0.0);
                add_enemy(&mut w, EnemyKind::Skitterer, c + off, 1.0);
                traps_tick(&mut w, Vec2::ZERO, false);
                assert_eq!(
                    board_state(&w, 0),
                    StickyTrapState::Idle,
                    "enemy should NOT stick outside pad at {off:?} for facing {facing:?}"
                );
            }
        }
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
        // The Wick Moth flies too.
        add_enemy(&mut w, EnemyKind::WickMoth, c + Vec2::new(1.0, 1.0), 1.0);
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

    // ---- Mousetraps ------------------------------------------------------

    /// Full state timeline of an ordinary trap: `Armed` → 0.28 s `Triggered`
    /// (no damage) → 0.20 s `Snapping` → `Sprung`, with the art's `hit_flash`
    /// at the impact.
    #[test]
    fn mousetrap_warns_then_snaps_then_is_sprung() {
        let mut w = open_arena(151);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        let bait = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        let e = add_enemy(&mut w, EnemyKind::Brute, bait, 20.0); // 1200 hp

        let mut ticks = 0usize;
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert!(w.level.objects.mousetraps[0].progress.abs() < 1e-6);
        let hp = enemy_of(&mut w, e).hp;
        for _ in 0..10_000 {
            assert!(
                enemy_of(&mut w, e).hp == hp,
                "the warning damaged the brute"
            );
            if mouse_state(&w, 0) != MousetrapState::Triggered {
                break;
            }
            traps_tick(&mut w, Vec2::ZERO, false);
            ticks += 1;
        }
        let warn = ticks as f32 * DT;
        assert!(
            (warn - MOUSETRAP_TRIGGER_SECS).abs() <= DT * 1.01,
            "warning lasted {warn} s, expected {MOUSETRAP_TRIGGER_SECS}"
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
            "snap lasted {snap} s, expected {MOUSETRAP_SNAP_SECS}"
        );
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert!((w.level.objects.mousetraps[0].progress - 1.0).abs() < 1e-6);
        assert!(
            w.level.objects.mousetraps[0].hit_flash > 0.0,
            "the impact flash had already faded"
        );
        assert_eq!(enemy_of(&mut w, e).hp, hp - MOUSETRAP_DAMAGE_ENEMY);

        // Sprung is permanent and harmless.
        let hp = enemy_of(&mut w, e).hp;
        for _ in 0..30 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert_eq!(enemy_of(&mut w, e).hp, hp);
        assert!(w.level.objects.mousetraps[0].hit_flash.abs() < 1e-6);
    }

    /// Three enemies under the bigger bar are each hit exactly once; bodies
    /// outside the `u in [-4, 18], |v| <= 10` rect are untouched.
    #[test]
    fn a_mousetrap_hits_three_enemies_exactly_once_each() {
        let mut w = open_arena(157);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        let perp = FACING.perp();
        let inside = [
            c + FACING * 2.0,
            c + FACING * 8.0 + perp * 6.0,
            c + FACING * 15.0 - perp * 8.0,
        ];
        for p in inside.iter() {
            add_enemy(&mut w, EnemyKind::Brute, *p, 20.0);
        }
        let outside = [
            c - FACING * 8.0,               // behind the cocked bar
            c + FACING * 8.0 + perp * 14.0, // beside the bar
            c + FACING * 24.0,              // past the far rim
        ];
        for p in outside.iter() {
            add_enemy(&mut w, EnemyKind::Brute, *p, 20.0);
        }
        let ids: Vec<u32> = w.enemies().iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 6);

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

        for e in w.enemies() {
            if ids[..3].contains(&e.id) {
                assert_eq!(
                    e.hp,
                    1200.0 - MOUSETRAP_DAMAGE_ENEMY,
                    "inside brute hit more than once: hp = {}",
                    e.hp
                );
            } else {
                assert_eq!(e.hp, 1200.0, "outside brute was hit: hp = {}", e.hp);
            }
        }
    }

    /// An ordinary snap: a walking rat is hit (hard), a dashing one is already
    /// out of the bar, and the damage is a share of the pool.
    #[test]
    fn an_ordinary_snap_hits_a_walking_rat_and_a_dash_escapes() {
        // Walking straight through the plate, from behind the bar.
        let mut w = open_arena(271);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        let start = c - FACING * 16.0;
        w.player.pos = start;
        w.player.face = FACING;
        let full = w.player.max_hp();
        let mut hp_after = full;
        for _ in 0..120 {
            w.begin_tick(DT, FACING, false);
            w.ecs.run_system_cached(traps_system).unwrap();
            w.ecs.run_system_cached(player::player_system).unwrap();
            w.end_tick();
            if w.player.hp < full {
                hp_after = w.player.hp;
                break;
            }
            if w.player.pos.x > c.x + 40.0 {
                break;
            }
        }
        let share = (full - hp_after) / full;
        assert!(
            (share - MOUSETRAP_DAMAGE_PLAYER_SHARE).abs() < 0.05,
            "a walking rat took {share:.2} of its pool, expected ~{MOUSETRAP_DAMAGE_PLAYER_SHARE}"
        );

        // Dashing through: the same run with a dash pressed at the plate.
        let mut w = open_arena(277);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c - FACING * 16.0;
        w.player.face = FACING;
        w.player.dash_cd = 0.0;
        let full = w.player.max_hp();
        for _ in 0..90 {
            let dash = w.player.dash_cd <= 0.0 && w.player.pos.x > c.x - 20.0;
            w.begin_tick(DT, FACING, dash);
            w.ecs.run_system_cached(traps_system).unwrap();
            w.ecs.run_system_cached(player::player_system).unwrap();
            w.end_tick();
            if w.player.pos.x > c.x + 40.0 {
                break;
            }
        }
        assert!(
            (w.player.hp - full).abs() < 1e-3,
            "a dashing rat was hit for {} hp",
            full - w.player.hp
        );
    }

    /// A hair trigger lands on the touch tick: only a dash that is *already*
    /// running (its i-frames) survives, and it does far more damage.
    #[test]
    fn a_hair_trigger_snaps_on_the_touch_tick() {
        // No dash: the rat takes the hair-trigger share the instant it touches.
        let mut w = open_arena(281);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Cheese);
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        let full = w.player.max_hp();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Snapping);
        let lost = (full - w.player.hp) / full;
        assert!(
            (lost - MOUSETRAP_DAMAGE_PLAYER_HAIR_SHARE).abs() < 0.05,
            "the hair trigger took {lost:.2} of the pool"
        );
        assert!(w.player.hp > 0.0, "no trap may one-shot a full rat");

        // Pushing dash on the same tick is not enough (the bar is already down).
        let mut w = open_arena(283);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Cheese);
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        w.player.dash_cd = 0.0;
        w.begin_tick(DT, FACING, true);
        w.ecs.run_system_cached(traps_system).unwrap();
        w.ecs.run_system_cached(player::player_system).unwrap();
        w.end_tick();
        assert!(
            w.player.hp < w.player.max_hp(),
            "a dash that only started this tick dodged a hair trigger"
        );

        // A dash already under way (its i-frames) gets through untouched.
        let mut w = open_arena(293);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Cheese);
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        w.player.iframes = 0.18;
        let full = w.player.max_hp();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert!((w.player.hp - full).abs() < 1e-3, "dash i-frames must dodge it");
        // It still snaps, and only once.
        assert_eq!(mouse_state(&w, 0), MousetrapState::Snapping);
        for _ in 0..40 {
            traps_tick(&mut w, Vec2::ZERO, false);
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert!((w.player.hp - full).abs() < 1e-3, "the bar hit twice");
    }

    /// Neither kind can take a full-health rat in one hit, whatever its armour,
    /// and armour only partly blunts the bar.
    #[test]
    fn snap_damage_respects_armour_and_never_one_shots() {
        for hair in [false, true] {
            for armor in [0.0f32, 2.0, 6.0, 20.0] {
                let mut w = open_arena(307);
                let c = centre(&w);
                let kind = if hair {
                    MousetrapKind::HairTrigger
                } else {
                    MousetrapKind::Ordinary
                };
                add_mousetrap_spec(&mut w, c, kind, TrapBait::Cheese);
                w.player.loadout.stats.armor = armor;
                w.player.hp = w.player.max_hp();
                w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
                let full = w.player.max_hp();
                for _ in 0..40 {
                    traps_tick(&mut w, Vec2::ZERO, false);
                    if w.player.hp < full {
                        break;
                    }
                }
                let lost = full - w.player.hp;
                assert!(lost > 0.0, "armour {armor} blocked the whole bar");
                assert!(
                    lost < full,
                    "a {kind:?} snap one-shot a full rat at armour {armor}"
                );
                // Armour blunts only part of it: the loss is above the share
                // minus half the armour, and below the bare share.
                let bare = full * if hair { 0.65 } else { 0.35 };
                assert!(
                    lost >= bare - armor * 0.5 - 1.0,
                    "armour ate too much: lost {lost}, bare {bare}, armour {armor}"
                );
                assert!(lost <= bare + 1.0, "armour made the bar worse");
            }
        }
    }

    /// A Bottle Cap eats a snap like any other hit (one charge, one bite).
    #[test]
    fn a_bottle_cap_shield_blocks_a_snap() {
        let mut w = open_arena(311);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Cheese);
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        w.player.buffs.shield = 12.0;
        let full = w.player.max_hp();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert!((w.player.hp - full).abs() < 1e-3, "the cap did not eat the bar");
        assert_eq!(w.player.buffs.shield, 0.0, "the cap was not spent");
    }

    /// Taking the bait is what springs the trap: the reward is collected on the
    /// touch and lands through the normal pickup path.
    #[test]
    fn the_bait_is_collected_on_touch_and_the_trap_springs() {
        let mut w = open_arena(313);
        let c = centre(&w);
        add_mousetrap_spec(
            &mut w,
            c,
            MousetrapKind::Ordinary,
            TrapBait::Pickup(PickupKind::PocketWatch),
        );
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        traps_and_pickups_tick(&mut w);
        assert!(
            w.player.buffs.slow_bullets > 0.0,
            "the pocket watch never reached the rat"
        );
        assert!(w.level.objects.mousetraps[0].bait_taken);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert!(w.last_pickups().contains(&PickupKind::PocketWatch));
    }

    /// A chest on the plate opens the ordinary chest flow.
    #[test]
    fn a_chest_bait_raises_the_chest_count() {
        let mut w = open_arena(317);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Chest);
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        traps_and_pickups_tick(&mut w);
        assert_eq!(w.pending_chests, 1, "the chest was not collected");
        assert!(w.level.objects.mousetraps[0].bait_taken);
    }

    /// An enemy sets the trap off and the bait is left lying on the sprung
    /// plate for the rat to take afterwards.
    #[test]
    fn an_enemy_trigger_leaves_the_bait_for_the_rat() {
        let mut w = open_arena(319);
        let c = centre(&w);
        add_mousetrap_spec(
            &mut w,
            c,
            MousetrapKind::Ordinary,
            TrapBait::Pickup(PickupKind::Magnet),
        );
        w.player.pos = c + Vec2::new(300.0, 0.0);
        add_enemy(&mut w, EnemyKind::Skitterer, c + FACING * MOUSETRAP_TRIGGER_OFFSET_U, 1.0);
        for _ in 0..200 {
            traps_tick(&mut w, Vec2::ZERO, false);
            if mouse_state(&w, 0) == MousetrapState::Sprung {
                break;
            }
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert!(
            !w.level.objects.mousetraps[0].bait_taken,
            "the horde ate the bait"
        );
        // The rat walks onto the sprung plate and takes it.
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        traps_and_pickups_tick(&mut w);
        assert!(w.level.objects.mousetraps[0].bait_taken);
        // The reward lands through the normal pickup path (the HUD's telemetry
        // list is what the App drains, so this is the same route a floor find takes).
        assert!(w.last_pickups().contains(&PickupKind::Magnet));
    }

    /// A sprung trap with nothing left to take drops out of the live list (the
    /// sim stops touching it), and a set one stays in it.
    #[test]
    fn a_sprung_trap_goes_inert() {
        let mut w = open_arena(323);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        w.level.objects.mousetraps[0].state = MousetrapState::Sprung;
        traps_tick(&mut w, Vec2::ZERO, false);
        let t = w.ecs.resource::<TrapRuntime>();
        assert!(t.live_mice.is_empty(), "a sprung trap stayed live");
    }

    /// Kills by a mousetrap take the normal loot path: the reap later in the
    /// same tick scores, counts the kill and drops the gem.
    #[test]
    fn mousetrap_kills_go_through_the_normal_loot_path() {
        let mut w = open_arena(163);
        let c = centre(&w);
        add_mousetrap(&mut w, c);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        for off in [1.0, 5.0, 9.0] {
            add_enemy(&mut w, EnemyKind::Skitterer, c + FACING * off, 1.0); // 6 hp
        }
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
        assert!(!w.gems().is_empty(), "no loot dropped");
        assert!(w.sfx.contains(&Sfx::Kill), "no kill sfx queued");
        assert!(w.enemies().is_empty(), "corpses were not reaped");
    }

    // ---- Sound + effects -------------------------------------------------

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

        // Release: exactly one tear-free, on the tick the rat is let go.
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
            if board_state(&w, 0) == StickyTrapState::Idle {
                break;
            }
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
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
        add_enemy(&mut w, EnemyKind::Skitterer, c, 40.0);

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
            if board_state(&w, 0) == StickyTrapState::Idle {
                break;
            }
        }
        assert_eq!(board_state(&w, 0), StickyTrapState::Idle);
        assert_eq!(releases, 1);
    }

    /// The mousetrap: one quiet `TrapTrigger` click when the bait is touched,
    /// then one `TrapSnap` crack at the bar's impact — never a `Hit` stand-in,
    /// never per window tick. A hair trigger cracks at once, with no click.
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
            20.0,
        );

        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert_eq!(sfx_count(&w, Sfx::TrapTrigger), 1);
        assert_eq!(sfx_count(&w, Sfx::UiMove), 0);

        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(mouse_state(&w, 0), MousetrapState::Triggered);
        assert!(w.sfx.is_empty(), "the warning is silent, got {:?}", w.sfx);

        w.sfx.clear();
        let mut snaps = 0usize;
        for _ in 0..300 {
            traps_tick(&mut w, Vec2::ZERO, false);
            snaps += sfx_count(&w, Sfx::TrapSnap);
            assert_eq!(sfx_count(&w, Sfx::Hit), 0, "a stand-in leaked");
            w.sfx.clear();
            if mouse_state(&w, 0) == MousetrapState::Sprung {
                break;
            }
        }
        assert_eq!(mouse_state(&w, 0), MousetrapState::Sprung);
        assert_eq!(snaps, 1, "exactly one crack per snap");

        // Hair trigger: no click, one crack, on the touch tick.
        let mut w = open_arena(203);
        let c = centre(&w);
        add_mousetrap_spec(&mut w, c, MousetrapKind::HairTrigger, TrapBait::Cheese);
        w.player.pos = c + Vec2::new(300.0, 0.0);
        add_enemy(
            &mut w,
            EnemyKind::Brute,
            c + FACING * MOUSETRAP_TRIGGER_OFFSET_U,
            20.0,
        );
        w.sfx.clear();
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(sfx_count(&w, Sfx::TrapTrigger), 0, "a hair trigger ticks, not clicks");
        assert_eq!(sfx_count(&w, Sfx::TrapSnap), 1);
    }

    // ---- Determinism -----------------------------------------------------

    /// Twin runs: same seed, same scripted input, identical trap state, load,
    /// wear and positions.
    #[test]
    fn twin_runs_reproduce_the_same_trap_state() {
        let run = |seed: u64| {
            let mut w = World::new(Theme::Kitchen, seed);
            for i in 0..900 {
                let dir = match (i / 90) % 4 {
                    0 => Vec2::new(1.0, 0.0),
                    1 => Vec2::new(0.0, 1.0),
                    2 => Vec2::new(-1.0, 0.0),
                    _ => Vec2::new(0.0, -1.0),
                };
                w.update(DT, dir, false);
            }
            let sticky: Vec<(u8, f32, u8, u32, u8)> = w
                .level
                .objects
                .sticky_traps
                .iter()
                .map(|v| (v.state as u8, v.worn, v.load, v.progress.to_bits(), v.tear_n))
                .collect();
            let mice: Vec<(u8, f32, bool)> = w
                .level
                .objects
                .mousetraps
                .iter()
                .map(|v| (v.state as u8, v.progress, v.bait_taken))
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
        assert!(
            !a.4.is_empty() || !a.5.is_empty(),
            "no traps on the map, test proves nothing"
        );
    }

    /// A bait is an ordinary floor find: the pickup pass books it for the HUD
    /// and the App's counters, once. (The trap used to book it as well, so one
    /// Heal on a plate was called out twice and counted as two.)
    #[test]
    fn a_collected_bait_is_booked_exactly_once() {
        let mut w = open_arena(331);
        let c = centre(&w);
        add_mousetrap_spec(
            &mut w,
            c,
            MousetrapKind::Ordinary,
            TrapBait::Pickup(PickupKind::Heal),
        );
        w.player.pos = c + FACING * MOUSETRAP_TRIGGER_OFFSET_U;
        traps_and_pickups_tick(&mut w);
        assert!(w.level.objects.mousetraps[0].bait_taken);
        let booked = w
            .last_pickups()
            .iter()
            .filter(|&&k| k == PickupKind::Heal)
            .count();
        assert_eq!(booked, 1, "one bait, one callout");
    }

    /// The floor's last live board runs out of glue with a body crossing it:
    /// the crossing slow must still come off the tick after, although no trap
    /// is live any more (the early-out used to strand it for the whole floor).
    #[test]
    fn a_crossing_slow_is_lifted_after_the_last_board_is_used_up() {
        let mut w = open_arena(337);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        // One empty tick builds the side table, then the glue is spent.
        traps_tick(&mut w, Vec2::ZERO, false);
        w.ecs.resource_mut::<TrapRuntime>().sticky[0].budget_left = 0;
        let e = add_enemy(&mut w, EnemyKind::Skitterer, c, 1.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert!(has_slow(&mut w, e), "a body on a spent pad is still slowed that tick");
        traps_tick(&mut w, Vec2::ZERO, false);
        assert!(!has_slow(&mut w, e), "the crossing slow outlived the board");
    }

    /// A board that is used up while a boss carries its crossing slow lifts
    /// that slow itself: a used board leaves the live list, so its boss timer
    /// would never run down.
    #[test]
    fn a_board_used_up_under_a_boss_lifts_the_boss_slow() {
        let mut w = open_arena(347);
        let c = centre(&w);
        add_board(&mut w, c);
        w.player.pos = c + Vec2::new(400.0, 0.0);
        let boss = add_enemy(&mut w, EnemyKind::Boss, c, 40.0);
        traps_tick(&mut w, Vec2::ZERO, false);
        assert!(has_slow(&mut w, boss));
        w.ecs.resource_mut::<TrapRuntime>().sticky[0].budget_left = 0;
        traps_tick(&mut w, Vec2::ZERO, false);
        assert_eq!(board_state(&w, 0), StickyTrapState::Used);
        assert!(!has_slow(&mut w, boss), "the boss kept the glue of a dead board");
    }
}
