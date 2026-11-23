//! W4 — per-material specular (Blinn-Phong LUTs, the slime double lobe,
//! steel/glass/grease/ink/water) and the directional rim light on creatures
//! (contract §4, techniques 07 + 09), plus the material **tagging tables**
//! the scene draw stamps with (`World::draw_scene` sets these pens around
//! each sprite's draw call).
//!
//! STUB: every creature is untextured Fur, nothing else is tagged, and the
//! pixel stage is a no-op.

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::{Grid, LightingMode};
use super::fx::{Canvas, FxFrame, Normal};
use super::material::{pen, MaterialKind};
use crate::game::ecs::systems::boss::BossKind;
use crate::game::ecs::systems::miniboss::MinibossKind;
use crate::game::enemy::EnemyKind;
use crate::game::level::PropKind;

/// The pen for an ordinary enemy or (with `miniboss`) a named miniboss body.
pub(crate) fn enemy_pen(kind: EnemyKind, miniboss: Option<MinibossKind>) -> u8 {
    pen(MaterialKind::Fur, true, 0)
}

/// The pen for a boss body (both the ground and the airborne pass).
pub(crate) fn boss_pen(kind: BossKind) -> u8 {
    pen(MaterialKind::Fur, true, 0)
}

/// The pen for the rat's body (tail and whip included).
pub(crate) fn rat_pen() -> u8 {
    pen(MaterialKind::Fur, true, 0)
}

/// The pen for a prop (consoles and tanks: Glass; pipes: Metal; …).
pub(crate) fn prop_pen(kind: PropKind) -> u8 {
    0
}

/// The pen for an acid pool (Acid Spit's ground pool).
pub(crate) fn pool_pen() -> u8 {
    0
}

/// Keep the old 1 px enemy rim glints this frame? False where [`apply`]'s
/// directional rim replaces them (Full/Hard).
pub(super) fn keep_glints(mode: LightingMode) -> bool {
    true
}

/// Post-composite pixel stage: specular highlights by material and the rim
/// light on CREATURE perimeter pixels (`fx::strength::SPECULAR`, `RIM`).
pub(super) fn apply(grid: &Grid, canvas: &mut Canvas<'_>, normals: &[Normal], frame: &FxFrame<'_>) {}
