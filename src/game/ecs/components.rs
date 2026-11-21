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
