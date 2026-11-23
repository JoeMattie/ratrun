//! W7 — particulate in beams (dust, steam, marine snow) and caustics
//! (contract §4, techniques 10 + 11). Analytic from `(level.seed, step8)`,
//! masked by the air channel, held on 8 Hz steps and sparse.
//!
//! STUB: no caustics anywhere, no motes.

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::Grid;
use super::fx::{Canvas, FxFrame};
use crate::game::level::{LightZoneKind, Theme};

/// Does this theme/zone show caustics? Off everywhere by default.
pub(super) fn caustics_here(theme: Theme, zone: Option<LightZoneKind>) -> bool {
    false
}

/// Last pixel stage: motes inside lit air, caustics where flagged
/// (`fx::strength::MOTES`, `CAUSTICS`).
pub(super) fn draw(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {}
