//! W2 — the air channel: volumetric light shafts, depth fog and bloom /
//! halation (contract §4, techniques 01, 02, 03). Writes `LightInfo::air` and
//! `LightInfo::fog`, then composites them over the lit surface.
//!
//! STUB: no light wants air, the passes are no-ops.

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::{Grid, LightingMode};
use super::fx::{Canvas, FxFrame, LightInfo, LightTag, SampleHit};
use super::Light;

/// Once per splat: does this light put anything into the air? (Shafts from
/// sconces, pendants, passage lamps; never on Coarse/Compact.)
pub(super) fn wants(light: &Light, tag: LightTag, mode: LightingMode) -> bool {
    false
}

/// Per touched sample of a light that [`wants`] air: add its shaft scatter
/// into `info.air` (`fx::strength::SHAFTS`). Never flickered.
pub(super) fn splat_sample(info: &mut LightInfo, hit: &SampleHit<'_>) {}

/// Grid stage after every splat: analytic bloom per `frame.lights` entry
/// (`fx::strength::BLOOM`) into `info.air`, and the fog amount into
/// `info.fog` (`fx::strength::FOG`).
pub(super) fn grid_pass(grid: &mut Grid, frame: &FxFrame<'_>) {}

/// Pixel stage after the surface stages: screen-blend `air`, then lerp toward
/// the theme's fog colour by `fog`.
pub(super) fn composite(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {}
