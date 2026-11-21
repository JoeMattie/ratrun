//! Coarse ECS components for the first port.
//!
//! Contract: each legacy entity struct stays **one component as a whole** —
//! `Enemy`, `Bullet`, `Gem`, `Pickup`, `Pool` — plus a [`SpawnId`] ordering
//! component. Do not split them into Position/Health/etc.; the art functions
//! (`art_enemy::draw_enemy`, `art_rat::draw_rat`, ...) take `&Enemy` and
//! friends, and Phase 1 is a behaviour-preserving port.
//!
//! The leaf modules (`enemy.rs`, `bullet.rs`, `gem.rs`) are frozen except for
//! derives, so the `Component` impls for those types are manual impls here
//! instead of touching the leaf files. `Pool` moving out of `world.rs` is
//! re-exported from the facade for source compatibility.

use bevy_ecs::component::{Component, Mutable, StorageType};

use crate::game::bullet::Bullet;
use crate::game::enemy::Enemy;
use crate::game::gem::{Chest, Gem, Pickup};
use crate::math::Vec2;
use crate::render::palette::Rgb;

pub use bevy_ecs::entity::Entity;

/// Monotonically increasing spawn order.
///
/// **Every** system that today depends on `Vec` index order (nearest-enemy
/// ties, separation nudges, per-enemy rng draws, reaping/drop order, bullet
/// spawn order, draw order) must iterate its query sorted by this component:
///
/// ```ignore
/// for (id, mut e) in enemies
///     .iter_mut()
///     .sort_by_key::<&SpawnId, _>(|id| id.0)
/// { /* legacy Vec order */ }
/// ```
///
/// `SpawnId` is assigned from [`super::resources::NextSpawnId`] at spawn time,
/// so it is stable across ticks (entities are NOT respawned between ticks).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpawnId(pub u64);

/// Damaging ground pool left by an Acid Spit lob.
///
/// Moved here from `world.rs` (it was never an entity outside the world
/// module). The damage + decay + despawning all happen in one pass in
/// `systems::projectiles::pools_system` so the same-tick ordering matches the
/// legacy `update_pools`.
#[derive(Component, Clone, Debug)]
pub struct Pool {
    pub pos: Vec2,
    pub radius: f32,
    pub dps: f32,
    pub life: f32,
    pub color: Rgb,
}

// ---- Wave 3 weapons A: Acid Rain drops and Hantavirus marks -------------

/// One locked Acid Rain drop: a floor position a volley marked, with the
/// splash that lands there when its countdown runs out.
///
/// Spawned by `weapons::fire_one` (the whole volley at once, in drop order) at
/// the moment the weapon fires, so the mark ring is on the floor for the
/// entire wind-up and the drops are visibly staggered. `systems::
/// weapons_wave3a::acid_rain_system` counts `delay` down, applies the splash
/// once and despawns the entity. It never leaves a lingering pool (design §4:
/// "No lingering base pool").
#[derive(Component, Clone, Copy, Debug)]
pub struct AcidDrop {
    /// Where the drop lands (world px, clamped into the arena at fire time).
    pub pos: Vec2,
    /// Seconds until it lands; the art reads this for the falling strokes.
    pub delay: f32,
    /// Splash radius and per-splash damage, snapshotted when the volley fired.
    pub radius: f32,
    pub dmg: f32,
    /// Position in the volley (0-based), for deterministic art variety.
    pub index: u8,
}

/// Longest Hantavirus chain the visit history can record. The design's largest
/// chain is `1 + VIRUS_JUMPS[4]` = 5 bodies; the array is fixed-size so a chain
/// never allocates, with slack for future tuning.
pub const MAX_CHAIN: usize = 8;

/// An active Hantavirus mark on one body (an [`crate::game::enemy::Enemy`] or
/// the boss — a boss is not an `Enemy`, so its mark lives in
/// `resources::BossVirus` instead of on an entity).
///
/// Three ticks 0.35 s apart, then the remaining jumps pass to one fresh
/// neighbour. The whole chain shares one visit history (copied on every jump),
/// so a chain visits each body at most once and stops when it is isolated.
#[derive(Component, Clone, Copy, Debug)]
pub struct Infection {
    /// Ticks left (3 → 2 → 1 → 0). The pips drawn over the body show this.
    pub ticks_left: u8,
    /// Seconds until the next tick.
    pub timer: f32,
    /// Jumps the chain may still make from this host.
    pub jumps_left: u8,
    /// Reach of the next jump and the damage per tick, snapshotted from the
    /// weapon level when the chain was started.
    pub reach: f32,
    pub dmg: f32,
    /// `SpawnId`s this chain has already visited, in visit order.
    pub visits: [u64; MAX_CHAIN],
    pub visited: u8,
}

impl Infection {
    /// A fresh mark on the body with `id` (three ticks, `jumps` transfers
    /// still available).
    pub fn new(id: u64, jumps: u8, reach: f32, dmg: f32) -> Self {
        let mut visits = [0u64; MAX_CHAIN];
        visits[0] = id;
        Self {
            ticks_left: crate::game::weapon::VIRUS_TICKS,
            timer: crate::game::weapon::VIRUS_TICK,
            jumps_left: jumps,
            reach,
            dmg,
            visits,
            visited: 1,
        }
    }

    /// Is this mark still ticking? (A spent mark no longer blocks a new tag.)
    #[inline]
    pub fn active(&self) -> bool {
        self.ticks_left > 0
    }

    /// Has this chain already visited the body with `id`?
    #[inline]
    pub fn has_visited(&self, id: u64) -> bool {
        self.visits[..self.visited as usize].contains(&id)
    }

    /// The next generation of this mark: a fresh three-tick infection on a new
    /// host, one jump shorter and with `id` appended to the shared history.
    /// `None` when the fixed history is full (which caps a chain, it never
    /// allocates).
    #[inline]
    pub fn jumped(&self, id: u64) -> Option<Self> {
        if self.jumps_left == 0 || self.visited as usize >= MAX_CHAIN {
            return None;
        }
        let mut next = *self;
        next.visits[next.visited as usize] = id;
        next.visited += 1;
        next.ticks_left = crate::game::weapon::VIRUS_TICKS;
        next.timer = crate::game::weapon::VIRUS_TICK;
        next.jumps_left -= 1;
        Some(next)
    }
}

// ---- Component impls for the frozen leaf entity structs ------------------

impl Component for Enemy {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Bullet {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Gem {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

impl Component for Pickup {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;
}

/// Queue with `commands.queue(ChestSpawn { pos })` to drop a chest from a system.
pub struct ChestSpawn {
    pub pos: Vec2,
}

impl ChestSpawn {
    pub fn spawn(self, ecs: &mut bevy_ecs::world::World) -> Entity {
        super::resources::spawn_with_id(ecs, Chest { pos: self.pos })
    }
}

impl bevy_ecs::system::Command for ChestSpawn {
    type Out = ();

    fn apply(self, ecs: &mut bevy_ecs::world::World) {
        self.spawn(ecs);
    }
}
