//! The `bevy_ecs` port of Rat Run's simulation.
//!
//! Layout:
//! - [`components`]: coarse entity components (`Enemy`, `Bullet`, `Gem`,
//!   `Pickup`, `Pool`) plus the [`components::SpawnId`] ordering component.
//! - [`resources`]: singleton resources + facade round-trip helpers.
//! - [`schedule`]: the one explicitly chained, single-threaded tick schedule.
//! - [`systems`]: one file per concern (player, spawn, enemies, weapons,
//!   projectiles, combat, loot, fx).
//! - `tests`: the ported behavioural suite (every legacy `world.rs` test).
//!
//! The public facade (`game::world::World`) owns a [`bevy_ecs::world::World`]
//! and the schedules, and keeps the legacy public fields as real fields. See
//! `CLAUDE.md` ("The facade round-trip") for the mechanism.

pub mod components;
pub mod constants;
pub mod resources;
pub mod schedule;
pub mod systems;

#[cfg(test)]
mod tests;

