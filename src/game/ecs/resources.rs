//! Singleton resources and the facade round-trip helpers.
//!
//! ## Facade round-trip (the mechanism the whole port uses)
//!
//! `game::world::World` keeps the legacy public fields as **real fields**
//! (`world.player`, `world.level`, `world.elapsed`, ...) so `app.rs`,
//! `render/hud.rs` and the app tests compile unchanged, and code between ticks
//! (tests setting `world.player.hp`, the App opening level-up cards) keeps
//! working.
//!
//! `World::update` calls `World::begin_tick`, which *moves* those values into
//! this Bevy world as resources via [`swap_in`]; it then runs the schedule and
//! calls `World::end_tick`, which moves the resources back via [`swap_out`].
//! Internal scratch state that has no public facade field (`SpatialGrid`,
//! `NeighborScratch`, `PickupScratch`, `NextSpawnId`, `NukeTick`) lives in the
//! Bevy world permanently.
//!
//! Entity components are *not* imported/exported: the Bevy world owns them for
//! the whole run, and nothing is copied out per tick. `World`'s HUD accessors
//! read the Bevy world directly (`&self`, via `World::iter_entities`) and
//! `World::enemies()`/`bullets()`/... clone a snapshot only when a test or the
//! determinism harness asks for a whole list.

use bevy_ecs::bundle::Bundle;
use bevy_ecs::entity::Entity;
use bevy_ecs::component::Component;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use rand::rngs::StdRng;

use crate::audio::Sfx;
use crate::game::gem::PickupKind;
use crate::math::Vec2;

use super::components::SpawnId;

// ---- Per-tick input ------------------------------------------------------

/// The three arguments of the legacy `World::update(dt, move_dir, dash)`.
///
/// Inserted by `World::begin_tick` before the schedule runs, so every system
/// sees the same tick inputs.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct TickInput {
    pub dt: f32,
    pub move_dir: Vec2,
    pub dash: bool,
    /// The Space/click press, read by the **active weapons**
    /// (`weapons::active_weapons_system`): it releases every fully charged
    /// active weapon. It is a separate field from `dash` because the two jobs
    /// only share a key on a terminal that cannot report a lone Ctrl press
    /// (there Space dashes *and* fires, so `World::begin_tick` mirrors `dash`
    /// into it); on a Kitty-protocol terminal Ctrl dashes and Space is only
    /// this field (`World::begin_tick_with`). A test can fire an active weapon
    /// while the dash is charging either way.
    pub active: bool,
}

/// Viewport size as of the last `World::draw` (spawn ring radius depends on
/// it). Copied in by `begin_tick`; never mutated by the sim.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Viewport(pub Vec2);

// ---- Rng / sfx -----------------------------------------------------------

/// `StdRng` is a foreign type, so it needs a newtype to become a resource.
///
/// Determinism: every system that draws random numbers takes `ResMut<RngRes>`,
/// and the schedule is a single chained (single-threaded) sequence, so the
/// draw order is exactly the legacy call order.
#[derive(Resource)]
pub struct RngRes(pub StdRng);

/// `World::sfx` facade queue. Systems push directly; the App drains it after
/// `update` (de-duplicating per frame), exactly like the legacy `Vec<Sfx>`.
#[derive(Resource, Default)]
pub struct SfxQueue(pub Vec<Sfx>);

// ---- Run counters / shake ------------------------------------------------

/// The legacy `elapsed`, `kills`, `score`, `pending_levelups`, `game_over`,
/// `won` fields, gathered into one resource for the round-trip.
///
/// Campaign structure (see [`crate::game::campaign`]): a level is complete when
/// its boss dies, which sets [`RunCounters::level_complete`]. Surviving
/// `director::RUN_SECONDS` no longer ends anything — the clock only tops out the
/// difficulty curve, and the boss fight runs past it.
#[derive(Resource, Default)]
pub struct RunCounters {
    pub elapsed: f32,
    pub kills: u32,
    pub score: u32,
    pub gold: u32,
    pub pending_chests: u32,
    pub pending_shop: bool,
    pub pending_levelups: u32,
    pub game_over: bool,
    pub won: bool,
    /// The level's boss is dead ⇒ the level is complete.
    ///
    /// Raised by the boss framework when the living boss's death is reaped
    /// (`loot::reap_dead` sees `EnemyKind::Boss`, or the boss system's own
    /// level-complete hook), read by `loot::outcome_system` to set `won` on the
    /// final level, and round-tripped to `World::level_complete`.
    pub level_complete: bool,
}

/// Which campaign level is being simulated: 0 Sewer, 1 Kitchen, 2 Lab.
///
/// Inserted by `World::begin_tick` from `World::level_index`, so every system
/// (the director's per-level difficulty scaling, the boss framework) can read
/// it without touching the facade. The theme and the final-level flag are
/// derived from it by [`crate::game::campaign::level_theme`] /
/// [`crate::game::campaign::is_final_level`].
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LevelIndex(pub usize);

/// Which row of the director's per-level difficulty tables this run plays on.
///
/// Equal to [`LevelIndex`] in a campaign, where the rat arrives with a carried
/// build. A practice run starts a later sector on the bare starting loadout, so
/// it plays that sector's map and boss on the first level's curve (`0`): the
/// later rows assume a carried build and open on a late-run horde.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct DifficultyLevel(pub usize);

impl LevelIndex {
    /// Is this the campaign's last level (its completion wins the run)?
    #[inline]
    pub fn is_final(self) -> bool {
        crate::game::campaign::is_final_level(self.0)
    }

    /// How many levels of the campaign come after this one.
    #[inline]
    pub fn levels_remaining(self) -> usize {
        crate::game::campaign::levels_remaining(self.0)
    }
}

/// The legacy `shake` / `shake_ofs` fields.
#[derive(Resource, Default)]
pub struct Shake {
    pub amount: f32,
    pub ofs: Vec2,
}

// ---- Balance instrumentation (lives in the ECS world permanently) --------

/// Cumulative damage the player's weapons actually dealt, split by weapon
/// (`WeaponKind::all()` index).
///
/// This is *instrumentation*, not simulation: nothing reads it inside the
/// schedule, so it cannot change behaviour or rng draws, and it is reset when a
/// new [`crate::game::world::World`] is built (one per campaign level). The
/// batch harness (`autoplay`) reads it to report each weapon's share of total
/// damage — the number that used to be inferred from "Tail Whip is about half
/// of boss damage" anecdotes.
///
/// Damage is booked at the sites that hurt an enemy, as *effective* damage
/// (`hp_before - hp_after`, floored at zero), so overkill on a nearly-dead
/// skitterer does not inflate a weapon's share. The boss is not an `Enemy` and
/// applies damage in its own system, so boss damage is attributed by the
/// harness from the boss fight's per-tick tally instead (`autoplay`).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct DamageLedger {
    /// Effective damage dealt to every enemy, by weapon index.
    pub dealt: [f32; 8],
    /// Effective damage dealt to **the boss**, by weapon index.
    ///
    /// Only weapons that damage the boss outside `boss_damage_system` need to
    /// book here (the two active weapons do: a thrash sweep and a squeak ring
    /// call `Boss::take_damage` directly). The boss fight's own source tally
    /// cannot carry them — `boss_damage_system` resets that tally after this
    /// one runs — so the batch harness reads this column instead. Also pure
    /// instrumentation: nothing in the schedule reads it back.
    pub boss: [f32; 8],
}

impl DamageLedger {
    /// Book `amount` effective damage from `kind` (no-op for non-positive
    /// amounts, which keeps a floor-clamped hit from subtracting).
    #[inline]
    pub fn add(&mut self, kind: crate::game::weapon::WeaponKind, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.dealt[kind.index()] += amount;
    }

    /// Book `amount` effective boss damage from `kind` (see [`DamageLedger::boss`]).
    #[inline]
    pub fn add_boss(&mut self, kind: crate::game::weapon::WeaponKind, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.boss[kind.index()] += amount;
    }
}

// ---- Internal scratch (lives in the ECS world permanently) ---------------

/// Monotonic spawn-id source. Entities are not respawned between ticks, so
/// spawn ids are stable for a whole run.
#[derive(Resource, Default)]
pub struct NextSpawnId(pub u64);

impl NextSpawnId {
    /// Hand out the next id and advance the counter.
    #[inline]
    pub fn take(&mut self) -> SpawnId {
        let id = self.0;
        self.0 += 1;
        SpawnId(id)
    }
}

// ---- Active (player-triggered) weapons -----------------------------------

/// Live state of the two **active** weapons (`WeaponKind::TailThrash`,
/// `WeaponKind::SonicSqueak`).
///
/// An active weapon's `Weapon::timer` is its charge meter (the same field every
/// auto-firing weapon uses as a cooldown), so nothing here decides *when* a
/// weapon may fire — only what the already-spent trigger is doing this tick.
/// One burst of each kind can be in flight at a time; a re-trigger (only
/// possible after a full charge) restarts it.
///
/// Permanent scratch: inserted by `World::new` and reset with the run, never
/// round-tripped (no facade field holds it between ticks; `World::draw` reads
/// the thrash arc through `World::thrash_visual` while it sits idle in the ECS
/// world, and the HUD reads each weapon's charge off the loadout instead). The
/// fields are a pure function of the trigger tick and `dt`, so a replay with
/// the same inputs draws the same rng and lands the same damage.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct ActiveFx {
    /// Seconds left of the tail-thrash burst (0 = idle).
    pub thrash_left: f32,
    /// Duration of the current burst (art progress; 0 when idle).
    pub thrash_total: f32,
    /// Seconds elapsed in the current burst (drives the sweep schedule).
    pub thrash_age: f32,
    /// The drawn sweep angle in radians (world space).
    pub thrash_angle: f32,
    /// Reach (px) and per-sweep damage of the current burst.
    pub thrash_reach: f32,
    pub thrash_dmg: f32,
    /// Sweeps already resolved in this burst.
    pub thrash_done: u32,
    /// Seconds left of the squeak ring's expansion (0 = idle).
    pub squeak_left: f32,
    /// Ring radius last tick and now; the damage band is `prev..now`, which is
    /// what makes each enemy get hit exactly once per ring.
    pub squeak_prev_r: f32,
    pub squeak_r: f32,
    /// Final radius and per-enemy damage of the current ring.
    pub squeak_max_r: f32,
    pub squeak_dmg: f32,
    /// Ring origin (the rat's position on the trigger tick).
    pub squeak_pos: Vec2,
}

impl ActiveFx {
    /// Is a tail-thrash burst playing right now?
    #[inline]
    pub fn thrash_active(&self) -> bool {
        self.thrash_left > 0.0
    }

    /// Is a squeak ring still expanding?
    #[inline]
    pub fn squeak_active(&self) -> bool {
        self.squeak_left > 0.0
    }

    /// Burst progress in `0..=1` (1 when idle), for the art's fade.
    #[inline]
    pub fn thrash_progress(&self) -> f32 {
        if self.thrash_total <= 0.0 {
            1.0
        } else {
            (self.thrash_age / self.thrash_total).clamp(0.0, 1.0)
        }
    }
}

/// Shared neighbour scratch buffer (legacy `World::neigh`).
#[derive(Resource, Default)]
pub struct NeighborScratch(pub Vec<u32>);

/// Reused pickup-collection scratch (legacy `World::pickup_scratch`).
#[derive(Resource, Default)]
pub struct PickupScratch(pub Vec<PickupKind>);

/// Set by `loot::pickups_system` when a Nuke lands this tick; consumed by
/// `loot::nuke_reap_system`. Reset to `false` by `World::begin_tick`.
#[derive(Resource, Default)]
pub struct NukeTick(pub bool);

/// Derived broadphase data shared between the grid build, separation and the
/// bullet-vs-enemy pass (legacy recomputed `max_radius` inline).
#[derive(Resource, Default)]
pub struct GridScratch {
    /// Largest enemy radius at grid-build time. Separation reach is `2 * max`.
    pub max_radius: f32,
}

/// Maps the index stored in `SpatialGrid` buckets to its entity.
///
/// The grid is filled in `SpawnId`-sorted order, so `EnemyIndex.0[j]` is the
/// entity the legacy code would have addressed as `self.enemies[j]`.
///
/// ## Validity window (read this before using it instead of sorting)
///
/// `EnemyIndex` is **only** valid while no enemy has been spawned or despawned
/// since it was built, because bucket values and the `Vec` indices are the
/// legacy `self.enemies` positions:
///
/// | built by | valid for |
/// | --- | --- |
/// | `enemy_move::grid_build_system` (schedule 6) | `separation_system`, `wall_resolve_system`, `grid_refresh_system` |
/// | `enemy_move::grid_refresh_system` (schedule 9) | `weapons::fire_weapons_system`, `projectiles::*`, `combat::*` — i.e. everything up to `loot::reap_enemies_system` |
///
/// **Stale** (never read it there):
/// * in `enemy_steering_system` / `enemy_fire_system`, where
///   `spawn::director_system` may already have added enemies this tick (those
///   systems keep their own `Local` ordered buffer);
/// * after `loot::reap_enemies_system` / `loot::nuke_reap_system` (or any other
///   despawn, e.g. `World::clear_enemies`): entries can point at dead entities,
///   so `Query::get` fails — the legacy equivalent would index `self.enemies`
///   out of range.
///
/// The next tick's `grid_build_system` rebuilds it from scratch; nothing has to
/// repair it in between. Do not append to it yourself: call the grid build.
#[derive(Resource, Default)]
pub struct EnemyIndex(pub Vec<Entity>);

/// Debug-build guard for the [`EnemyIndex`] validity window.
///
/// Every system that reads `EnemyIndex.0[j]` (a `SpatialGrid` bucket index) or
/// walks `EnemyIndex` instead of sorting the enemy query calls this first with
/// the live `(SpawnId, Entity)` list. `cargo test` builds with
/// `debug_assertions`, so any tick where an enemy was spawned or despawned
/// between a build point and a reader trips here; release builds compile the
/// function body away (`#[cfg(not(debug_assertions))]` callers skip it too).
///
/// The comparison is against a freshly sorted list, not just a length check:
/// a despawn + spawn in the same window keeps the length identical while
/// invalidating every slot, which is exactly the hole this is meant to catch.
#[cfg(debug_assertions)]
pub fn debug_assert_enemy_index(index: &[Entity], spawn_ids: impl Iterator<Item = (u64, Entity)>) {
    let mut fresh: Vec<(u64, Entity)> = spawn_ids.collect();
    fresh.sort_unstable_by_key(|&(id, _)| id);
    assert_eq!(
        index.len(),
        fresh.len(),
        "stale EnemyIndex: {} entries but {} live enemies",
        index.len(),
        fresh.len()
    );
    for (slot, &(id, entity)) in fresh.iter().enumerate() {
        assert_eq!(
            index[slot], entity,
            "stale EnemyIndex at slot {slot} (SpawnId {id}): cached {:?}, live {:?}",
            index[slot], entity
        );
    }
}

// ---- Merge resources (neon combat-feedback team) -------------------------
//
// These mirror the fields the other team added to the legacy `World`. They
// are round-tripped by `World::begin_tick` /
// `World::end_tick` exactly like `Player`/`Particles`/... so the facade fields
// stay authoritative between ticks and systems see the same values.
//
// `DamageNumbers` itself carries `#[derive(Resource)]` in
// `game::particle::damage_number` (a one-line derive, no behaviour change), so
// systems take `ResMut<DamageNumbers>` directly.

/// Monotonic enemy identity source (legacy `World::next_enemy_id`).
///
/// Starts at `1` and wraps skipping `0` (0 means "anonymous" to
/// `DamageNumbers`). The director and the facade spawn helpers both draw from
/// this so every live enemy has a stable `Enemy::id`.
#[derive(Resource)]
pub struct NextEnemyId(pub u32);

impl Default for NextEnemyId {
    fn default() -> Self {
        Self(1)
    }
}

impl NextEnemyId {
    /// Hand out the next id and advance the counter (legacy
    /// `e.id = self.next_enemy_id; self.next_enemy_id = self.next_enemy_id.wrapping_add(1).max(1);`).
    #[inline]
    pub fn take(&mut self) -> u32 {
        let id = self.0;
        self.0 = self.0.wrapping_add(1).max(1);
        id
    }
}

/// Pickups collected during the most recent tick (legacy
/// `World::last_pickups`), read by the HUD telemetry.
#[derive(Resource, Default)]
pub struct LastPickups(pub Vec<PickupKind>);

// ---- Boss hand-off -------------------------------------------------------

/// "The boss is due": the spawn director's hand-off to the boss framework.
///
/// `director::raise_boss_due` raises it exactly once, at
/// `director::BOSS_TIME`; the boss framework (P7) consumes it with
/// [`BossDue::take`] and spawns the level's themed boss. It lives in the ECS
/// world permanently (like `NukeTick`), **not** in the facade round-trip, so a
/// boss system anywhere in the schedule reads the same flag.
///
/// The pre-boss warning is unchanged: the HUD derives it from
/// `Director::boss_spawned`, which the director also sets at the boss time, so
/// the 25 s countdown still runs from `BOSS_TIME - 25 s` to `BOSS_TIME`.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct BossDue {
    /// The director has reached the boss time and wants a boss.
    pub due: bool,
    /// The boss framework has taken the signal (it has spawned, or is
    /// spawning, the boss). A second `take` is a no-op.
    pub claimed: bool,
    /// Campaign level the boss belongs to: 0 Sewer (jaguar), 1 Kitchen (broom),
    /// 2 Lab (mower) — see `crate::game::campaign::level_theme`.
    pub level: usize,
    /// Run clock when the signal was raised (`director::BOSS_TIME`).
    pub at: f32,
}

impl BossDue {
    /// Consume the signal: `true` on the first call after the director raised
    /// it, then `false` forever (per level).
    #[inline]
    pub fn take(&mut self) -> bool {
        if self.due && !self.claimed {
            self.claimed = true;
            true
        } else {
            false
        }
    }

    /// Raised and not yet claimed.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.due && !self.claimed
    }
}

// ---- Round-trip helpers --------------------------------------------------

/// Move a facade field into the ECS world, using the pre-built spare value to
/// fill the hole in the facade.
pub fn swap_in<T: Resource>(ecs: &mut World, facade: &mut T, spare: &mut Option<T>) {
    let placeholder = spare
        .take()
        .expect("facade spare must be initialised in World::new");
    let value = std::mem::replace(facade, placeholder);
    ecs.insert_resource(value);
}

/// Move a resource back out of the ECS world into the facade field, leaving
/// the previous facade value in the spare slot for the next tick.
pub fn swap_out<T: Resource>(ecs: &mut World, facade: &mut T, spare: &mut Option<T>) {
    let value = ecs
        .remove_resource::<T>()
        .expect("facade resource must have been inserted by begin_tick");
    *spare = Some(std::mem::replace(facade, value));
}

/// Clone every component of type `T` into a `Vec`, in stable `SpawnId` order.
///
/// Only [`super::super::world::World::enemies`] and its siblings (test and
/// determinism-harness helpers) call this: the facade no longer rebuilds
/// per-tick entity caches (nothing outside `src/game` read them), so this
/// allocates once per call instead of once per tick.
pub fn collect_sorted<T: Component + Clone>(ecs: &mut World) -> Vec<T> {
    let mut query = ecs.query::<(&SpawnId, &T)>();
    let mut pairs: Vec<(u64, T)> = query
        .iter(ecs)
        .map(|(id, component)| (id.0, component.clone()))
        .collect();
    pairs.sort_unstable_by_key(|(id, _)| *id);
    pairs.into_iter().map(|(_, component)| component).collect()
}

/// Spawn a bundle with the next [`SpawnId`] attached, returning the entity.
pub fn spawn_with_id<B: Bundle>(ecs: &mut World, bundle: B) -> Entity {
    let id = ecs.resource_mut::<NextSpawnId>().take();
    ecs.spawn((bundle, id)).id()
}

/// The guard is `debug_assertions`-only, so these tests only exist in a debug
/// build (which is what `cargo test` uses).
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    /// The guard itself must fail on a stale index, or `cargo test` would be
    /// proving nothing. Two entities whose cached order is reversed, and a
    /// despawn+spawn pair that keeps the length but changes the contents.
    #[test]
    #[should_panic(expected = "stale EnemyIndex at slot 0")]
    fn enemy_index_guard_rejects_a_same_length_reordering() {
        let mut ecs = World::new();
        let a = ecs.spawn(()).id();
        let b = ecs.spawn(()).id();
        debug_assert_enemy_index(&[b, a], [(1u64, a), (2u64, b)].into_iter());
    }

    #[test]
    #[should_panic(expected = "stale EnemyIndex: 2 entries but 1 live enemies")]
    fn enemy_index_guard_rejects_a_length_mismatch() {
        let mut ecs = World::new();
        let a = ecs.spawn(()).id();
        debug_assert_enemy_index(&[a, a], [(1u64, a)].into_iter());
    }
}
