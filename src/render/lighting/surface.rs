//! W3 — bump-mapped floors and walls from the albedo's own luminance
//! gradient (contract §4, technique 08). Writes the per-pixel normals the
//! specular and reflection stages read.
//!
//! STUB: no-op (normals stay flat, the albedo is untouched).

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::Grid;
use super::fx::{Canvas, FxFrame, Normal};

/// Pre-composite pixel stage over the **unlit** albedo: write a tangent
/// normal per pixel into `normals` (`canvas.w * canvas.h`, zeroed this frame)
/// and modulate the albedo by `N·L` against the covering sample's dominant
/// light (`fx::strength::BUMP`). Never touches CREATURE or Emissive pixels.
pub(super) fn apply(grid: &Grid, canvas: &mut Canvas<'_>, normals: &mut [Normal], frame: &FxFrame<'_>) {}
