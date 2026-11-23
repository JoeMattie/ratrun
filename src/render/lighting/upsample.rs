//! W1 — wall-aware bilinear upsampling of the light grid, the tone curve and
//! coloured bounce light (contract §4, techniques 04 + the bilinear
//! foundation). Every function here runs only while the effects are on.
//!
//! STUB: each body is a pass-through, so the picture is unchanged.

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::{Grid, LightingMode, AMBIENT_SCALE};
use super::fx::{Canvas, FxFrame};
use crate::game::level::Theme;

/// The `mul` clamp `Sample::light` uses while the effects are on (fx off
/// keeps 350). Raise it for the soft clip (contract: 350 -> about 405).
pub(super) const MUL_CEILING: i32 = 350;

/// The share of the theme tint every floor's ambient is, while the effects are
/// on (fx off keeps [`AMBIENT_SCALE`]). Called once per frame from
/// `Grid::prepare`.
pub(super) fn ambient_scale(theme: Theme, mode: LightingMode) -> f32 {
    AMBIENT_SCALE
}

/// Grid stage, after every splat and before the vignette: add the coloured
/// bounce term into `grid.samples[].mul` (a low-frequency spread of the lit
/// energy, `fx::strength::BOUNCE`).
pub(super) fn bounce(grid: &mut Grid, frame: &FxFrame<'_>) {}

/// Shade the pixels with the grid (replaces `Grid::composite` while the
/// effects are on): bilinear taps that never cross a wall boundary, the tone
/// curve, the tint mix, the shine floor and — below truecolor — the palette
/// dither, exactly like the old path.
pub(super) fn composite(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {
    grid.composite_px(canvas.px, canvas.w, canvas.h, frame.depth);
}
