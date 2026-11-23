//! W5 — the global overhead light: one light high above the map's centre, so
//! every body casts a soft ground shadow displaced away from the centre by
//! `(pos - centre) * height / H` (contract §4; owner decision in
//! `.review/handoff/reports-b12/lighting-decisions.md`). Drawn into the floor
//! before the walls, props and bodies, so they cover it and walls clip it.
//!
//! STUB: draws nothing.

#![allow(unused_variables, dead_code)] // Stub: remove when filled in.

use super::engine::LightingMode;
use crate::game::level::Level;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use bevy_ecs::world::World as EcsWorld;

/// What the overhead pass reads. Built by `World::draw_scene` each frame the
/// effects are on (and the plate is not flat-lit).
pub(crate) struct OverheadScene<'a> {
    pub ecs: &'a EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    /// World position of pixel (0, 0).
    pub cam: Vec2,
    pub elapsed: f32,
    /// The tier this frame's lighting runs at.
    pub mode: LightingMode,
    /// A Library plate with no rat: cast no rat shadow.
    pub hide_rat: bool,
}

/// Darken the floor under every body (`fx::strength::OVERHEAD_DARKEN`).
pub(crate) fn draw(pb: &mut PixelBuffer, scene: &OverheadScene<'_>) {}
