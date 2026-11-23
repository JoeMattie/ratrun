//! World-resolution, render-only lighting. Hostile art is composited above it.
//! LightMap owns its scratch, occluders and cached angular visibility profiles.
//!
//! Add moving lights with Light::radial, configure the shape and shadow knobs,
//! and submit with LightMap::push_shaped. For small halos use LightMap::emit at
//! a world position and mark_emissive_rect after painting the sprite core.
//! Radii are in world pixels, cone half-angles in radians. Use the receiver
//! and occluder masks independently when a surface should only cast a shadow.
//! The theme fixture palette, housing, and arena phase hooks are in fixtures.rs.
//! To add an emitter, give its kind a recipe in ambience.rs and place it by
//! room role in game/levelgen/lights.rs. Prop height and 4×4 occluder outlines
//! are in shadow.rs.
//!
//! Fog and the minimap use their existing sight and maze rules. Lighting only
//! shades the playfield, so a distant lamp cannot reveal a route.

mod air;
mod ambience;
mod engine;
mod fixtures;
mod fx;
mod material;
mod motes;
mod overhead;
mod shadow;
mod softshadow;
mod sources;
mod specular;
mod surface;
mod upsample;

pub(crate) use ambience::draw_ambience;
pub(crate) use fixtures::draw_fixtures;
// Batch-13 effects (`.review/handoff/lighting/CONTRACT.md`).
pub(crate) use fx::enabled_from_env as fx_enabled_from_env;
pub(crate) use overhead::{draw_deferred as draw_overhead_shadows, OverheadScene};
pub(crate) use specular::{boss_pen, enemy_pen, giant_rat_pen, pool_pen, prop_pen, rat_pen};
#[cfg(test)]
mod gallery;
#[cfg(test)]
mod tests;

#[allow(unused_imports)] // Artist-facing blend modes.
pub use engine::Blend;
#[cfg(test)]
use engine::{ambient_level, zone_gain, Curve, AMBIENT_FLOOR, MIN_GAIN};
use engine::{PointLight, TAIL_REACH};
pub use engine::{
    Falloff, Flicker, LayerMask, Light, LightMap, LightShape, LightingMode, ShadowColor,
    ShadowSoftness,
};

use crate::game::level::{Level, Theme};
use crate::game::particle::Particles;
use crate::game::player::Player;
use crate::math::Vec2;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;
use bevy_ecs::world::World as EcsWorld;

pub struct LightScene<'a> {
    pub ecs: &'a EcsWorld,
    pub level: &'a Level,
    pub player: &'a Player,
    pub particles: &'a Particles,
    pub thrash: Option<(f32, f32, f32)>,
    pub squeak: Option<(Vec2, f32)>,
    pub cam: Vec2,
    pub elapsed: f32,
    pub mode: LightingMode,
    pub depth: ColorDepth,
    /// The batch-13 lighting effects (`fx.rs`): off, the pass is the
    /// batch-12 one byte for byte.
    pub fx: bool,
}

/// Per-theme lighting character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeAmbient {
    /// Base multiplicative ambient light in `Q8.8` (`256 = 1.0×`).
    pub mul: [u16; 3],
    /// Whether fixed level lights use crisp/hard pool falloff (Lab).
    pub hard_pools: bool,
    /// Soft personal aura tint around the player rat.
    pub rat_tint: Rgb,
}

/// Ambient tint & lighting character for `theme`: one row per playable floor
/// (the match is exhaustive, so a new theme cannot ship without one).
#[inline]
pub fn theme_ambient(theme: Theme) -> ThemeAmbient {
    match theme {
        // Dim, damp green-blue subterranean shadow; warm lantern rat contrasts nicely.
        Theme::Sewer => ThemeAmbient {
            mul: [112, 139, 148],
            hard_pools: false,
            rat_tint: (255, 240, 206),
        },
        // Warm amber kitchen lighting with cozy shaded corners.
        Theme::Kitchen => ThemeAmbient {
            mul: [161, 130, 102],
            hard_pools: false,
            rat_tint: (255, 244, 222),
        },
        // Cold clinical shadows with sharp, bright pools of light.
        Theme::Lab => ThemeAmbient {
            mul: [103, 132, 170],
            hard_pools: true,
            rat_tint: (224, 244, 255),
        },
        // Cool sky bounce remains under the warm directional sun.
        Theme::Trailer => ThemeAmbient {
            mul: [107, 117, 141],
            hard_pools: false,
            rat_tint: (255, 248, 230),
        },
        // Cool indigo under a haze of steam: the blue channel leads and the
        // shadows stay soft (wet air scatters the light), with a pale
        // porcelain glow on the rat.
        Theme::Laundry => ThemeAmbient {
            mul: [102, 116, 158],
            hard_pools: false,
            rat_tint: (232, 244, 246),
        },
        // Flat newsprint grey with a registration-cyan lean: neutral shadows,
        // crisp pools under the press lamps, a cool cyan-white rat.
        Theme::Printworks => ThemeAmbient {
            mul: [115, 126, 140],
            hard_pools: true,
            rat_tint: (220, 246, 248),
        },
    }
}

/// Fast pixel luminance approximation (0..=255) for readability assertions.
#[inline]
pub fn luminance(c: Rgb) -> u8 {
    ((c.0 as u32 * 77 + c.1 as u32 * 150 + c.2 as u32 * 29) >> 8) as u8
}
