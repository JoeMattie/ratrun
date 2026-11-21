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
use crate::game::gem::{Gem, Pickup};
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

/// A dead Wick Moth's burning fuse (`WAVE3_DESIGN.md` §9).
///
/// The moth's body is reaped with every other corpse, so the fuse is its own
/// entity: [`crate::game::ecs::systems::enemy_wave3`] spawns one on the tick it
/// sees a dead moth, burns `fuse` down, and then bursts for 9 to the rat and 20
/// to every enemy inside 9 px. A moth killed by a burst leaves its own fuse, so
/// the chain only ends when the moths do.
#[derive(Component, Clone, Debug)]
pub struct Wick {
    pub pos: Vec2,
    /// Seconds left before the burst.
    pub fuse: f32,
}

impl Wick {
    /// A fresh, fully-fused wick at `pos`.
    pub fn new(pos: Vec2) -> Self {
        Wick {
            pos,
            fuse: crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS,
        }
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
