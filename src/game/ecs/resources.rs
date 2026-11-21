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
    /// Which shop hole opened the screen (`LevelObjects::shop_holes` index),
    /// so the App can open that hole's own shelf. `0` when none is open.
    pub pending_shop_hole: usize,
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
    pub dealt: [f32; crate::game::weapon::WEAPON_SLOTS],
    /// Effective damage dealt to **the boss**, by weapon index.
    ///
    /// Only weapons that damage the boss outside `boss_damage_system` need to
    /// book here (the two active weapons do: a thrash sweep and a squeak ring
    /// call `Boss::take_damage` directly). The boss fight's own source tally
    /// cannot carry them — `boss_damage_system` resets that tally after this
    /// one runs — so the batch harness reads this column instead. Also pure
    /// instrumentation: nothing in the schedule reads it back.
    pub boss: [f32; crate::game::weapon::WEAPON_SLOTS],
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

    // ---- Evolutions (WAVE3_DESIGN.md §3) --------------------------------
    //
    // The two active weapons' evolutions need cross-tick state that the firing
    // systems write and the evolution systems read, so it rides along here
    // (this resource is already permanent scratch). Nothing below is touched
    // unless the weapon is evolved.

    /// The burst's weapon was evolved when it was spent (**Flywheel**).
    pub thrash_evolved: bool,
    /// The ring's weapon was evolved when it was spent (**Roll Call**).
    pub squeak_evolved: bool,
    /// **Flywheel**: hostile pellets caught by the current thrash burst, with
    /// the direction each was travelling when the arc took it.
    pub caught: u8,
    pub caught_dirs: [Vec2; FLYWHEEL_CAP],
    /// **Roll Call**: bodies the ring stunned or slowed, their `SpawnId`s (the
    /// handle can go stale, the id identifies it), and the answer timer.
    pub tags: [Option<Entity>; ROLL_CALL_TAG_CAP],
    pub tag_ids: [Option<u64>; ROLL_CALL_TAG_CAP],
    pub tag_n: u8,
    /// Seconds until the tags answer (0 = nothing pending).
    pub answer_left: f32,
    /// Half the squeak's per-hit damage, fixed when the tags were collected.
    pub answer_damage: f32,
}

/// Bodies one **Roll Call** squeak may tag (and answer with) per ring.
pub const ROLL_CALL_TAG_CAP: usize = 12;
/// Hostile pellets one **Flywheel** thrash burst may hold (two per sweep, three
/// sweeps, plus slack).
pub const FLYWHEEL_CAP: usize = 6;

impl ActiveFx {
    /// Forget everything tied to one squeak ring.
    pub fn clear_roll_call(&mut self) {
        self.tags = [None; ROLL_CALL_TAG_CAP];
        self.tag_ids = [None; ROLL_CALL_TAG_CAP];
        self.tag_n = 0;
        self.answer_left = 0.0;
        self.answer_damage = 0.0;
    }

    /// Record one Roll Call tag. Returns false when the ring already tagged
    /// that body (or the table is full).
    pub fn tag(&mut self, entity: Entity, id: u64) -> bool {
        if self.tag_ids[..self.tag_n as usize].contains(&Some(id)) {
            return false;
        }
        if (self.tag_n as usize) >= ROLL_CALL_TAG_CAP {
            return false;
        }
        self.tags[self.tag_n as usize] = Some(entity);
        self.tag_ids[self.tag_n as usize] = Some(id);
        self.tag_n += 1;
        true
    }

    /// Throw every caught Flywheel pellet away (burst over).
    pub fn clear_caught(&mut self) {
        self.caught = 0;
        self.caught_dirs = [Vec2::ZERO; FLYWHEEL_CAP];
    }

    /// Catch one pellet's direction; returns false when the burst is holding
    /// its maximum.
    pub fn catch_pellet(&mut self, dir: Vec2) -> bool {
        if (self.caught as usize) >= FLYWHEEL_CAP {
            return false;
        }
        self.caught_dirs[self.caught as usize] = dir;
        self.caught += 1;
        true
    }
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

// ---- Wave 3 weapons A ----------------------------------------------------

/// The Hantavirus mark currently on the boss, if any.
///
/// The boss is not an `Enemy`, so it cannot carry the `components::Infection`
/// component the horde uses; this resource is that mark's home instead. One
/// mark at a time, no jumps (design §4: "A boss can take ticks"), cleared when
/// its last tick lands or the boss dies. Permanent scratch: inserted by
/// `World::new`, never round-tripped (no facade field holds it between ticks).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct BossVirus(pub Option<crate::game::ecs::components::Infection>);

/// Shared neighbour scratch buffer (legacy `World::neigh`).
#[derive(Resource, Default)]
pub struct NeighborScratch(pub Vec<u32>);

/// Reused pickup-collection scratch (legacy `World::pickup_scratch`).
///
/// Each entry is the kind collected **and** the pickup's floor position: the
/// wave-3 Wind-up Mouse leaves its brass key where the mouse was lying, so the
/// collection pass has to carry the spot as well as the kind.
#[derive(Resource, Default)]
pub struct PickupScratch(pub Vec<(PickupKind, Vec2)>);

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

// ---- Wave 3 weapons (Laser Eyes, Super Dash) and their passives ---------
//
// All four live in the ECS world permanently (inserted by `World::new`, like
// `ActiveFx`), because none of them is a facade field: they are per-tick state
// whose readers are systems, `World::draw` (the laser art) and tests.

/// Live state of one **Laser Eyes** burst.
///
/// Laser Eyes is an ordinary auto-firing weapon (not an active one: it has no
/// charge gauge and Space does not spend it) whose shot is a *burst* of pulses
/// over the following ticks. `weapons_wave3b::laser_eyes_system` charges the
/// weapon's `Weapon::timer` and owns the burst; `fire_weapons_system` skips the
/// kind entirely (`WeaponKind::fires_outside_fire_one`).
///
/// The facing is **locked at the trigger tick** (design §4: "Lock last facing
/// at burst start"), so a rat that turns mid-burst does not drag the sweep with
/// it — the pulses fan out around the direction it was looking when it fired.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct LaserFx {
    /// Seconds left in the current burst (0 = idle).
    pub left: f32,
    /// Seconds elapsed since the trigger tick (drives the pulse schedule).
    pub age: f32,
    /// Pulses already resolved in this burst.
    pub fired: u32,
    /// Pulses this burst will make at the owning weapon's level.
    pub pulses: u32,
    /// The facing locked at the trigger tick (unit vector).
    pub facing: Vec2,
    /// The beam's origin (the rat's eye line at the trigger tick).
    pub origin: Vec2,
    /// Nominal beam length in px before the wall stop (Lens Cap applied).
    pub reach: f32,
    /// Per-pulse damage at the owning weapon's level (Sharp Teeth applied).
    pub dmg: f32,
    /// World angle of the pulse resolved most recently (art).
    pub angle: f32,
    /// Beam length the last pulse actually reached after the wall stop (art).
    pub beam: f32,
    /// Seconds since the last pulse was resolved (drives the blink: the bar is
    /// lit for the first part of the interval and dark for the rest).
    pub since_pulse: f32,
    /// Full length of the burst in seconds (`0` when idle), set at trigger.
    pub total: f32,
    /// Was the burst in flight fired by the evolved (**Carbon Copy**) weapon?
    /// Only such a burst replays the kept sweep and records itself.
    pub evolved: bool,
    /// The last **completed** sweep, kept for **Carbon Copy** (design §3: "keep
    /// one copy"). `None` until the evolved weapon has finished its first
    /// burst.
    pub copy: Option<LaserCopy>,
    /// Seconds left of the afterimage replay (0 = not replaying).
    pub copy_left: f32,
    /// Seconds elapsed since the afterimage started replaying.
    pub copy_age: f32,
    /// Afterimage pulses already resolved.
    pub copy_fired: u32,
}

/// One recorded Laser Eyes sweep: the exact geometry a **Carbon Copy**
/// afterimage replays on the next sweep (design §3: "Record the facing and
/// origin of a completed sweep").
#[derive(Clone, Copy, Debug)]
pub struct LaserCopy {
    /// Where the recorded burst fired from.
    pub origin: Vec2,
    /// The facing it locked (unit vector).
    pub facing: Vec2,
    /// Pulses it made, so the replay is the same fan.
    pub pulses: u32,
    /// Beam length it used (already Lens Cap-stretched).
    pub reach: f32,
    /// Per-pulse damage it dealt; the afterimage deals half.
    pub dmg: f32,
}

impl LaserFx {
    /// Is a burst in flight right now?
    #[inline]
    pub fn active(&self) -> bool {
        self.left > 0.0
    }

    /// Is a **Carbon Copy** afterimage replaying right now?
    #[inline]
    pub fn copy_active(&self) -> bool {
        self.copy_left > 0.0
    }

    /// **Carbon Copy**: the sweep this burst just finished, ready to be kept as
    /// the next burst's afterimage (`LaserFx::copy`).
    #[inline]
    pub fn copy_of_this_sweep(&self) -> Option<LaserCopy> {
        if self.total <= 0.0 {
            return None;
        }
        Some(LaserCopy {
            origin: self.origin,
            facing: self.facing,
            pulses: self.pulses,
            reach: self.reach,
            dmg: self.dmg,
        })
    }

    /// Burst progress in `0..=1`, for the art's fade-out.
    #[inline]
    pub fn progress(&self) -> f32 {
        if self.total <= 0.0 {
            1.0
        } else {
            (self.age / self.total).clamp(0.0, 1.0)
        }
    }
}

/// How many bodies one dash can remember as already hit. A dash crosses a
/// handful of bodies; the fixed history only exists so "once per target per
/// dash" cannot allocate and cannot loop.
pub const DASH_HIT_CAP: usize = 48;

/// **Ripcord**: seconds the dash's actual route stays taut after the dash, and
/// the fixed number of travelled legs it can remember (a 0.16 s dash at 60 fps
/// is ~10 legs; the cap bounds a large-`dt` or click-dash route).
pub const RIPCORD_SECS: f32 = 0.6;
pub const RIPCORD_SEG_CAP: usize = 24;

/// Dash legs one tick can record. The dash moves in sub-steps shorter than the
/// player radius (`player::PLAYER_SUBSTEP`), so a normal tick is one or two
/// legs and this cap only bounds a huge `dt`; past it the last leg is extended
/// to the tick's end so the recorded path still reaches the rat's real position.
pub const DASH_TICK_SEG_CAP: usize = 8;

/// The dash's **actual** travelled segment(s), written by
/// `systems::player::player_system`'s dash hook and consumed by
/// `weapons_wave3b::super_dash_system`.
///
/// This is the whole of the coupling between the dash and the Super Dash
/// weapon: the dash owner records what really happened (a successful dash
/// started here; this tick moved the rat from `a` to `b` while the dash was
/// live), so the weapon never has to guess. A blocked or cooling press never
/// reaches the hook, and the tick after `dash_active` has run out records no
/// segment, so neither buys a hit.
#[derive(Resource)]
pub struct DashTrack {
    /// `Some((from, to))` while this tick's movement happened during a dash
    /// **and actually moved the rat**. A dash pressed straight into a wall
    /// resolves to its starting point and records nothing, so a degenerate
    /// (zero-length) sweep can never damage a body standing next to the rat.
    pub segment: Option<(Vec2, Vec2)>,
    /// This tick's dash movement as the sub-steps that really happened, in
    /// travel order (`seg_len` of them). One chord cannot cut a corner the
    /// rat's collision-resolved path took around it, so the dash hit tests
    /// each real leg.
    pub segs: [(Vec2, Vec2); DASH_TICK_SEG_CAP],
    pub seg_len: usize,
    /// How many dash legs have been travelled in the current dash (the weapon
    /// uses `== 1` for its one sound per dash).
    pub legs: u32,
    /// Has the boss already taken this dash's hit? (The boss is not an
    /// `Enemy`, so it needs its own flag rather than a `SpawnId` entry.)
    pub boss_hit: bool,
    /// `SpawnId`s already hit by the current dash.
    hits: [u64; DASH_HIT_CAP],
    hit_len: usize,
    /// **Ripcord**'s taut route: the legs this dash actually travelled, kept
    /// for [`RIPCORD_SECS`] after the dash and then snapped.
    route: [(Vec2, Vec2); RIPCORD_SEG_CAP],
    route_len: usize,
    /// Seconds left before the route snaps (0 = no taut route).
    pub route_life: f32,
    /// Where the dash left from: what the cord slides bodies toward.
    route_start: Vec2,
    /// `SpawnId`s the cord has already hit (one dash hit each).
    route_hits: [u64; DASH_HIT_CAP],
    route_hit_len: usize,
    /// Has the cord already hurt the boss?
    pub route_boss_hit: bool,
}

impl Default for DashTrack {
    fn default() -> Self {
        Self {
            segment: None,
            segs: [(Vec2::ZERO, Vec2::ZERO); DASH_TICK_SEG_CAP],
            seg_len: 0,
            legs: 0,
            boss_hit: false,
            hits: [0; DASH_HIT_CAP],
            hit_len: 0,
            route: [(Vec2::ZERO, Vec2::ZERO); RIPCORD_SEG_CAP],
            route_len: 0,
            route_life: 0.0,
            route_start: Vec2::ZERO,
            route_hits: [0; DASH_HIT_CAP],
            route_hit_len: 0,
            route_boss_hit: false,
        }
    }
}

impl DashTrack {
    /// A **successful** dash just began: forget the previous dash's hits, and
    /// snap any cord still hanging from the previous dash.
    #[inline]
    pub fn begin(&mut self) {
        self.hit_len = 0;
        self.legs = 0;
        self.boss_hit = false;
        self.segment = None;
        self.seg_len = 0;
        self.clear_route();
    }

    /// Record this tick's movement. `live` is the dash flag the dash owner
    /// computed *before* it decayed `player.dash_active`, so the last dash tick
    /// still counts; `frame_start` is where the rat stood at the top of the
    /// tick.
    ///
    /// A dash that moved **nothing** (pressed flush into a wall, so the
    /// sub-step resolution put the rat back where it started) records no
    /// segment at all: it is a refused dash in every way that matters, and the
    /// zero-length "sweep" it used to record was a valid circle overlap on the
    /// rat's own position.
    #[inline]
    pub fn record(&mut self, frame_start: Vec2, to: Vec2, live: bool) {
        if live && frame_start.dist_sq(to) > 0.000_1 {
            self.segment = Some((frame_start, to));
            self.legs = self.legs.saturating_add(1);
        } else {
            // Not dashing (or the dash moved nothing): this tick has no legs,
            // and the sub-steps pushed during the move are dropped with it.
            self.segment = None;
            self.seg_len = 0;
        }
    }

    /// Record one collision-resolved sub-step of this tick's dash movement.
    /// Zero-length legs are dropped (they can hit nothing anyway).
    #[inline]
    pub fn push_leg(&mut self, from: Vec2, to: Vec2) {
        if from.dist_sq(to) <= 0.000_1 {
            return;
        }
        if self.seg_len < DASH_TICK_SEG_CAP {
            self.segs[self.seg_len] = (from, to);
            self.seg_len += 1;
        } else {
            // Past the cap the path keeps its first legs and the last one is
            // stretched to the rat's real position (the cap is only reachable
            // with an enormous `dt`).
            self.segs[DASH_TICK_SEG_CAP - 1].1 = to;
        }
    }

    /// This tick's recorded dash legs, in travel order.
    #[inline]
    pub fn legs_now(&self) -> &[(Vec2, Vec2)] {
        &self.segs[..self.seg_len]
    }

    /// Has the current dash already hit this body?
    #[inline]
    pub fn already_hit(&self, id: u64) -> bool {
        self.hits[..self.hit_len].contains(&id)
    }

    /// Remember a hit. Past [`DASH_HIT_CAP`] the body is simply not recorded
    /// (deterministic: the walk is in `SpawnId` order, so the same bodies win
    /// on every replay).
    #[inline]
    pub fn mark_hit(&mut self, id: u64) {
        if self.hit_len < DASH_HIT_CAP && !self.already_hit(id) {
            self.hits[self.hit_len] = id;
            self.hit_len += 1;
        }
    }

    // ---- Ripcord: the taut route -----------------------------------------

    /// Add this dash leg to the taut route and keep the cord alive for the
    /// design's [`RIPCORD_SECS`] after the last leg. Past
    /// [`RIPCORD_SEG_CAP`] legs the route keeps the legs it has (deterministic:
    /// the cap is reached in travel order).
    #[inline]
    pub fn extend_route(&mut self, from: Vec2, to: Vec2) {
        if self.route_len == 0 {
            self.route_start = from;
        }
        if self.route_len < RIPCORD_SEG_CAP {
            self.route[self.route_len] = (from, to);
            self.route_len += 1;
        }
        self.route_life = RIPCORD_SECS;
    }

    /// The travelled legs, in travel order.
    #[inline]
    pub fn route(&self) -> &[(Vec2, Vec2)] {
        &self.route[..self.route_len]
    }

    /// One leg, copied (so a caller can take `&mut self` again afterwards).
    #[inline]
    pub fn route_segment(&self, i: usize) -> (Vec2, Vec2) {
        self.route[i]
    }

    /// Legs in the route.
    #[inline]
    pub fn route_len(&self) -> usize {
        self.route_len
    }

    /// Where the dash left from (the cord's slide target).
    #[inline]
    pub fn route_start(&self) -> Vec2 {
        self.route_start
    }

    /// Has the cord already hit this body?
    #[inline]
    pub fn route_already_hit(&self, id: u64) -> bool {
        self.route_hits[..self.route_hit_len].contains(&id)
    }

    /// Remember a cord hit (one dash hit per body per cord, like the dash's
    /// own history).
    #[inline]
    pub fn mark_route_hit(&mut self, id: u64) {
        if self.route_hit_len < DASH_HIT_CAP && !self.route_already_hit(id) {
            self.route_hits[self.route_hit_len] = id;
            self.route_hit_len += 1;
        }
    }

    /// Drop the route: the cord has snapped.
    #[inline]
    pub fn clear_route(&mut self) {
        self.route_len = 0;
        self.route_life = 0.0;
        self.route_hit_len = 0;
        self.route_boss_hit = false;
    }
}

// ---- Per-run evolution bookkeeping --------------------------------------

/// Evolution state that must survive between ticks but reset with the run.
///
/// It lives in the ECS world (not in a system `Local`) on purpose: the schedule
/// and any `run_system_cached` copy of a system are *different instances* with
/// their own locals, so a `Local` counter made the same cast sequence depend on
/// which path fired it — and survived a world reset. A resource inserted by
/// `World::new_level*` starts at zero with the run, so a replayed level always
/// counts the same way.
#[derive(Resource, Default, Debug)]
pub struct EvoRuntime {
    /// Spore Orbit casts of the **current weapon instance**: **Fairy Ring**
    /// leaves a fixed ring on every second one (design §3: "every second orbit
    /// cycle").
    pub fairy_casts: u8,
    /// Which Spore Orbit instance `fairy_casts` belongs to (kind + level +
    /// evolved as one key). A recast from a *different* instance — the shop's
    /// Molt, or dropping and re-acquiring the weapon — restarts the parity at
    /// zero, so a new orbit never inherits half of the old one's cycle.
    pub fairy_key: u32,
}

impl EvoRuntime {
    /// Count one Spore Orbit cast for the instance identified by `key` and
    /// return the new count. A key change starts the parity again.
    #[inline]
    pub fn count_fairy_cast(&mut self, key: u32) -> u8 {
        if self.fairy_key != key {
            self.fairy_key = key;
            self.fairy_casts = 0;
        }
        self.fairy_casts = self.fairy_casts.wrapping_add(1);
        self.fairy_casts
    }
}

/// Capacity of the Counterweight melee-nudge log ("once per target per 0.4 s").
pub const MELEE_NUDGE_CAP: usize = 24;

/// Per-target cooldown for **Counterweight**'s extra melee shove.
///
/// The passive gives +2 px of outward shove per stack to ordinary melee hits
/// that have no native knockback (Tail Whip), at most once per body per 0.4 s.
/// `weapons::fire_one` asks [`MeleeNudge::ready`] before nudging and then
/// [`MeleeNudge::mark`]s the body; `weapons_wave3b::melee_nudge_system` decays
/// the timers (and does nothing at all when the passive is not owned).
#[derive(Resource, Default)]
pub struct MeleeNudge {
    entries: [(u64, f32); MELEE_NUDGE_CAP],
    len: usize,
}

impl MeleeNudge {
    /// Decay every cooldown by `dt`, dropping spent entries (order-preserving,
    /// so the array cannot drift between replays).
    pub fn tick(&mut self, dt: f32) {
        let mut write = 0;
        for read in 0..self.len {
            let (id, left) = self.entries[read];
            let left = left - dt;
            if left > 0.0 {
                self.entries[write] = (id, left);
                write += 1;
            }
        }
        self.len = write;
    }

    /// May this body be nudged this tick?
    #[inline]
    pub fn ready(&self, id: u64) -> bool {
        !self.entries[..self.len].iter().any(|&(h, _)| h == id)
    }

    /// Start this body's cooldown. A full log forgets the body instead (it was
    /// nudged, it just cannot be tracked — never a wrong hit, only a rarer one).
    #[inline]
    pub fn mark(&mut self, id: u64, cooldown: f32) {
        if self.len < MELEE_NUDGE_CAP && self.ready(id) {
            self.entries[self.len] = (id, cooldown);
            self.len += 1;
        }
    }

    /// Live entries (tests and diagnostics).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
}

/// **Second Scent**'s fraction bank: gem XP is an integer, so `+10% per stack`
/// has to remember the remainder between collections ("bank fractional XP
/// across collections").
///
/// Permanent scratch: a new `World` per campaign level starts empty, which is
/// the campaign's "reset XP and player level" rule.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct XpBank(pub f32);

impl XpBank {
    /// Scale `xp` by `mult` and return the whole XP now payable, keeping the
    /// fraction for the next collection.
    pub fn grant(&mut self, xp: u32, mult: f32) -> u32 {
        let total = xp as f32 * mult + self.0;
        // A hair of tolerance keeps 0.6 + 0.6 from arriving as 1.1999999 and
        // paying 1.
        let whole = (total + 1e-4).floor().max(0.0);
        self.0 = (total - whole).max(0.0);
        whole as u32
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
