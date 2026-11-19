//! Simulation constants, lifted verbatim from the legacy `world.rs` so every
//! Phase 1 system and the ported tests share one definition.

/// Player collision radius (world pixels).
pub const PLAYER_RADIUS: f32 = 3.0;
/// Max distance the player moves per collision sub-step; must stay well under
/// `PLAYER_RADIUS` so a fast tick cannot tunnel through a thin wall.
pub const PLAYER_SUBSTEP: f32 = 2.0;
/// Hard cap on live particles (`Particles::new(PARTICLE_CAP)`).
pub const PARTICLE_CAP: usize = 6000;
/// Hard cap on live enemies; the director reserves one slot for the boss.
pub const ENEMY_CAP: usize = 600;
/// Broadphase grid cell size.
pub const GRID_CELL: f32 = 16.0;
