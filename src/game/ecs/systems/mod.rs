//! One file per Phase 1 assignment. Each system stub documents the exact
//! legacy functions and line ranges it must reproduce, the resources and
//! components it uses, and the ordering constraints it relies on.
//!
//! See `PORT_PLAN.md` for the ownership table and the stable-ordering rule
//! (`SpawnId`).

pub mod boss;
pub mod combat;
pub mod enemy_move;
pub mod evolutions;
pub mod enemy_wave3;
pub mod fog;
pub mod fx;
pub mod gates;
pub mod loot;
pub mod player;
pub mod powerups;
pub mod projectiles;
pub mod shop;
pub mod spawn;
pub mod trailer;
pub mod traps;
pub mod weapons;
// Wave 3's self-driving weapons (Laser Eyes, Super Dash) and Counterweight's
// melee-nudge clock. Split out of `weapons` so the auto-fire file stays about
// the weapons it fires.
pub mod weapons_wave3b;
pub mod weapons_wave3a;
