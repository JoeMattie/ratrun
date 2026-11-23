//! W6 — soft penumbra and long shadows from low fixtures (an extended cache
//! beside the cached `ShadowProfile`), and wet-floor reflections (contract
//! §4, techniques 05 + 06).
//!
//! STUB: visibility is the old one, the cache is empty, no reflections.

#![allow(unused_variables)] // Stub: remove when filled in.

use super::engine::{Grid, LightingMode};
use super::fx::{Canvas, FxFrame, LightTag, Normal, ShadowQuery};
use super::shadow::{Geometry, ShadowProfile};
use super::Light;

/// Your per-light cache beside each `ShadowProfile` (one per static fixture,
/// one per moving shadow slot): blocker distance / height per ray, …
#[derive(Clone, Default)]
pub(super) struct SoftProfile {}

/// Called every frame after the light's `ShadowProfile::bake`; `rebaked` is
/// true when that profile was rebuilt (a static fixture's is cached).
pub(super) fn bake(soft: &mut SoftProfile, geometry: &Geometry, light: &Light, tag: LightTag, rebaked: bool) {}

/// The shadow visibility of one sample (replaces
/// `ShadowProfile::visibility` while the effects are on).
pub(super) fn visibility(profile: &ShadowProfile, soft: Option<&SoftProfile>, q: &ShadowQuery<'_>) -> f32 {
    profile.visibility(q.delta, q.surface, q.softness)
}

/// Keep the old hard-coded Sewer/Lab floor streak in `Grid::splat`? False
/// once [`reflections`] draws the real ones.
pub(super) fn keep_floor_glint(mode: LightingMode) -> bool {
    true
}

/// Post-composite pixel stage: lamp streaks on reflective pixels
/// (`fx::strength::REFLECTION`).
pub(super) fn reflections(grid: &Grid, canvas: &mut Canvas<'_>, normals: &[Normal], frame: &FxFrame<'_>) {}
