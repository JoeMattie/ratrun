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
use crate::game::weapon::WeaponKind;
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

// ---- Evolution entities (WAVE3_DESIGN.md §3) -----------------------------
//
// One component per evolved weapon that needs state between ticks. They are
// owned by `systems::evolutions` (except the two markers, which
// `projectiles::bullets_system` reads at the moment the evolved shot lands or
// hits a wall). Every history below is a fixed-size array: nothing here
// allocates per tick, and every walk over them is `SpawnId`-ordered.

/// Bodies a single Milk Teeth return thread may bite (fixed history).
pub const MILK_BITE_CAP: usize = 16;
/// Bodies one Fondue Fork string may cut (fixed history).
pub const STRING_CUT_CAP: usize = 24;
/// Bodies one Fairy Ring may track for its "one hit per target per second".
pub const FAIRY_TRACK_CAP: usize = 24;
/// Bodies one Drain Cleaner channel may damage on its advancing front.
pub const CHANNEL_HIT_CAP: usize = 24;

/// Marker on a Squeak Nova spoke fired by an **Echo Chamber** evolution: its
/// first wall contact reflects the spoke along its incoming line instead of
/// consuming it (one reflection per spoke).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct EchoSpoke {
    pub reflected: bool,
}

/// Marker on an evolved projectile whose landing behaviour differs from its
/// base weapon (Acid Spit → **Drain Cleaner**'s directed channel).
#[derive(Component, Clone, Copy, Debug)]
pub struct EvolvedShot {
    pub kind: WeaponKind,
    /// Firing direction (unit vector); the corrosive channel follows it.
    pub dir: Vec2,
}

/// Phase of a lodged **Milk Teeth** dart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MilkPhase {
    /// Flying out toward the aim point, biting the first body it crosses.
    Out,
    /// Lodged: hanging still for [`crate::game::ecs::systems::evolutions::MILK_LODGE_SECS`].
    Lodged,
    /// Pulling back to the rat's current position, biting each crossed body.
    Back,
}

/// A **Milk Teeth** dart: out, lodge 1 s, then home back to the rat.
#[derive(Component, Clone, Debug)]
pub struct MilkTooth {
    pub pos: Vec2,
    pub vel: Vec2,
    pub phase: MilkPhase,
    /// Seconds in the current phase.
    pub age: f32,
    /// Per-hit damage (the weapon's L5 hit after stats).
    pub damage: f32,
    /// Bodies bitten on the way back, in bite order.
    pub bitten: [Option<u64>; MILK_BITE_CAP],
    pub bitten_n: u8,
}

impl MilkTooth {
    pub fn new(pos: Vec2, vel: Vec2, damage: f32) -> Self {
        Self {
            pos,
            vel,
            phase: MilkPhase::Out,
            age: 0.0,
            damage,
            bitten: [None; MILK_BITE_CAP],
            bitten_n: 0,
        }
    }

    #[inline]
    pub fn has_bitten(&self, id: u64) -> bool {
        self.bitten[..self.bitten_n as usize].contains(&Some(id))
    }

    /// Record a bite; returns false when the history is full (the bite still
    /// lands — the cap bounds the memory, not the damage).
    pub fn remember_bite(&mut self, id: u64) -> bool {
        if self.has_bitten(id) {
            return false;
        }
        if (self.bitten_n as usize) < MILK_BITE_CAP {
            self.bitten[self.bitten_n as usize] = Some(id);
            self.bitten_n += 1;
        }
        true
    }
}

/// A **Fondue Fork** cheese string between two planted tines: cuts each crosser
/// once while it lasts.
#[derive(Component, Clone, Debug)]
pub struct CheeseString {
    pub a: Vec2,
    pub b: Vec2,
    pub life: f32,
    pub total: f32,
    pub damage: f32,
    pub cut: [Option<u64>; STRING_CUT_CAP],
    pub cut_n: u8,
}

impl CheeseString {
    pub fn new(a: Vec2, b: Vec2, life: f32, damage: f32) -> Self {
        Self { a, b, life, total: life, damage, cut: [None; STRING_CUT_CAP], cut_n: 0 }
    }

    pub fn has_cut(&self, id: u64) -> bool {
        self.cut[..self.cut_n as usize].contains(&Some(id))
    }

    pub fn remember_cut(&mut self, id: u64) {
        if (self.cut_n as usize) < STRING_CUT_CAP {
            self.cut[self.cut_n as usize] = Some(id);
            self.cut_n += 1;
        }
    }
}

/// A **Fairy Ring**: satellites left fixed around the casting point. Crossing
/// the band costs one hit per body per second.
#[derive(Component, Clone, Debug)]
pub struct FairyRing {
    pub pos: Vec2,
    pub radius: f32,
    pub life: f32,
    pub total: f32,
    pub spin: f32,
    pub satellites: u8,
    pub damage: f32,
    /// Per-body hit cooldowns (SpawnId, seconds left), a fixed history.
    pub cooled: [Option<(u64, f32)>; FAIRY_TRACK_CAP],
}

impl FairyRing {
    pub fn new(pos: Vec2, radius: f32, life: f32, satellites: u8, damage: f32) -> Self {
        Self {
            pos,
            radius,
            life,
            total: life,
            spin: 0.0,
            satellites,
            damage,
            cooled: [None; FAIRY_TRACK_CAP],
        }
    }

    /// Seconds of cooldown left for `id` (0 when it may be hit again).
    pub fn cooldown(&self, id: u64) -> f32 {
        for entry in self.cooled.iter().flatten() {
            if entry.0 == id {
                return entry.1;
            }
        }
        0.0
    }

    /// Start (or restart) `id`'s cooldown. When the table is full the entry
    /// with the least time left is recycled, so a long ring keeps hurting the
    /// bodies that stay in it.
    pub fn cool(&mut self, id: u64, secs: f32) {
        let mut free: Option<usize> = None;
        let mut weakest: Option<(usize, f32)> = None;
        for (i, slot) in self.cooled.iter().enumerate() {
            match slot {
                Some((held, left)) if *held == id => {
                    self.cooled[i] = Some((id, secs.max(*left)));
                    return;
                }
                Some((_, left)) => {
                    if weakest.is_none_or(|(_, best)| *left < best) {
                        weakest = Some((i, *left));
                    }
                }
                None => {
                    free = Some(i);
                    break;
                }
            }
        }
        let slot = free.or(weakest.map(|(i, _)| i));
        if let Some(i) = slot {
            self.cooled[i] = Some((id, secs));
        }
    }

    pub fn tick_cooldowns(&mut self, dt: f32) {
        for slot in self.cooled.iter_mut() {
            if let Some((id, left)) = *slot {
                let left = left - dt;
                *slot = if left <= 0.0 { None } else { Some((id, left)) };
            }
        }
    }
}

/// A **Drain Cleaner** channel: the splash runs `CHANNEL_LEN` px along the
/// firing direction over 0.6 s, then the settled stripe ticks for 2 s.
#[derive(Component, Clone, Debug)]
pub struct CorrosiveChannel {
    pub start: Vec2,
    /// Unit firing direction.
    pub dir: Vec2,
    /// Length still open to walk down (truncated at the first wall).
    pub length: f32,
    /// Seconds since the splash landed.
    pub age: f32,
    /// Pool damage per second for the settled phase (base pool dps).
    pub dps: f32,
    /// Per-body damage of the advancing front (one hit each).
    pub front_damage: f32,
    pub hit: [Option<u64>; CHANNEL_HIT_CAP],
    pub hit_n: u8,
}

impl CorrosiveChannel {
    pub fn new(start: Vec2, dir: Vec2, length: f32, dps: f32, front_damage: f32) -> Self {
        Self {
            start,
            dir,
            length,
            age: 0.0,
            dps,
            front_damage,
            hit: [None; CHANNEL_HIT_CAP],
            hit_n: 0,
        }
    }

    pub fn has_hit(&self, id: u64) -> bool {
        self.hit[..self.hit_n as usize].contains(&Some(id))
    }

    pub fn remember_hit(&mut self, id: u64) {
        if (self.hit_n as usize) < CHANNEL_HIT_CAP {
            self.hit[self.hit_n as usize] = Some(id);
            self.hit_n += 1;
        }
    }

    /// Far end of the stripe, clamped to the wall it stopped at.
    pub fn end(&self) -> Vec2 {
        self.start + self.dir * self.length
    }

    /// The stripe centre (for the settled tick test).
    pub fn centre(&self) -> Vec2 {
        self.start + self.dir * (self.length * 0.5)
    }
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
