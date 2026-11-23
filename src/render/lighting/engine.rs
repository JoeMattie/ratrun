//! The light map itself: light descriptions, accumulation, shadows and the
//! final shade over the world pixels.
//!
//! * [`LightingMode`] — the detail tiers (`LIVE = [Full, Hard, Coarse,
//!   Compact]`). `default_for_depth` picks the starting tier from the
//!   terminal's colour depth, and the cost-driven quality controller drops a
//!   tier when a frame runs past its budget.
//! * [`Light`], [`LightShape`], [`Falloff`], [`Blend`], [`Flicker`],
//!   [`LayerMask`], [`ShadowSoftness`], [`ShadowColor`] — what a light is.
//!   Radii are world pixels, cone half-angles radians; receiver and occluder
//!   masks are independent.
//!
//! ## Small lights and the darker base level (batch 8)
//!
//! A small source — a projectile, orbiter or enemy halo, a gem, gold, a pickup
//! or chest, an ember or pilot light, an emissive prop, a fixture smaller than
//! a room lamp — uses [`Falloff::Small`]: the smooth core it always had plus a
//! faint, long tail out to [`TAIL_REACH`] times its stated radius. The tail is
//! `TAIL_SHARE * (1 - u²)²` with `u = d / reach`, so it is exactly zero at the
//! end of the reach (no edge, no ring) and is skippable: `TAIL_SHARE` is a
//! share of the core, so 60 % of the reach already carries under 8 % of it.
//! Large lights — the sun, the rat's own aura and a boss presence — keep their
//! size and their own curves. (The Arena fixture family is dormant since batch
//! 12 removed the boss arena; no map places one.)
//!
//! The tail is not free: a moving halo is what makes the terminal repaint
//! cells, and a tail that creeps with every flicker would repaint a whole room
//! for a light that is barely there. Two rules keep the cost down. The tail is
//! quantised into flat steps of at most one output level ([`MIN_GAIN`] in
//! `Q8.8` units, so four coarse levels for a typical halo), and it is added
//! *after* the flicker, so a breathing flame animates its core only. And any
//! contribution below that same level is exactly zero: it could not change a
//! pixel, and painting it would repaint cells for nothing.
//!
//! The ambient is the same story from the other side. Every floor's and zone's
//! base level is [`AMBIENT_SCALE`] of its theme tint (never below
//! [`AMBIENT_FLOOR`]), so the room is a dark floor with pools of light on it,
//! not a lit room. Tints and the zone ordering are untouched.
//!
//! Ordinary enemy bodies are drawn before this pass. [`LightMap::mark_glint`]
//! keeps one pixel of each body lit in the body's own colour, so a silhouette
//! still reads outside any pool.
//! * [`LightMap`] — owns its scratch buffers, the occluder set and the cached
//!   angular visibility profiles, so a steady frame allocates nothing.
//!
//! Render-only: nothing here reads or writes simulation state, and a change of
//! tier must never change a simulated number.

use super::fx::{self, Canvas, FxFrame, FxLight, LightInfo, LightTag, Normal, SampleHit, ShadowQuery};
use super::shadow::{Body, Geometry, ShadowProfile, Surface};
use super::softshadow::SoftProfile;
use super::{air, motes, softshadow, specular, surface, upsample};
use super::{fixtures, LightScene, ThemeAmbient};
use crate::game::ecs::systems::boss::Boss;
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::level::{LightFixtureKind, LightZoneKind, Theme};
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingMode {
    Full,
    Hard,
    Coarse,
    Compact,
    #[cfg(test)]
    Off,
}

impl LightingMode {
    pub const LIVE: [Self; 4] = [Self::Full, Self::Hard, Self::Coarse, Self::Compact];

    pub const fn default_for_depth(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::Truecolor => Self::Full,
            ColorDepth::Ansi256 => Self::Hard,
            ColorDepth::Ansi16 => Self::Coarse,
        }
    }

    const fn rank(self) -> usize {
        self as usize
    }

    const fn stride(self) -> usize {
        match self {
            Self::Full | Self::Hard => 4,
            _ => 8,
        }
    }

    const fn budget(self) -> (usize, usize) {
        // (dynamic light count, shadowed dynamic light count)
        match self {
            Self::Full | Self::Hard | Self::Coarse => (28, 3),
            _ => (14, 1),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LightShape {
    Radial {
        radius: f32,
    },
    Cone {
        radius: f32,
        dir: Vec2,
        half_angle: f32,
    },
    Beam {
        dir: Vec2,
        length: f32,
        half_width: f32,
    },
    Directional {
        dir: Vec2,
    },
}

impl LightShape {
    pub(super) fn extent(self) -> f32 {
        match self {
            Self::Radial { radius } | Self::Cone { radius, .. } => radius,
            Self::Beam {
                length, half_width, ..
            } => length + half_width,
            Self::Directional { .. } => f32::INFINITY,
        }
    }

    fn weight(self, delta: Vec2, cone_limit: f32) -> f32 {
        match self {
            Self::Radial { radius } => 1.0 - delta.len_sq() / (radius * radius),
            Self::Cone { radius, dir, .. } => {
                let distance = delta.len();
                let edge = if distance < 0.5 {
                    1.0
                } else {
                    (delta.dot(dir) / distance - cone_limit) / 0.12
                };
                (1.0 - distance * distance / (radius * radius)) * edge.clamp(0.0, 1.0)
            }
            Self::Beam {
                dir,
                length,
                half_width,
            } => {
                let along = delta.dot(dir);
                if !(0.0..length).contains(&along) {
                    return 0.0;
                }
                let across = delta.dot(dir.perp());
                (1.0 - across * across / (half_width * half_width)).max(0.0)
                    * ((length - along) / (half_width * 2.0)).min(1.0)
            }
            Self::Directional { .. } => 1.0,
        }
    }
}

#[allow(dead_code)] // Subtract and Mix are content authoring knobs.
#[derive(Clone, Copy, Debug)]
pub enum Blend {
    Add,
    Subtract,
    Mix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    Smooth,
    Linear,
    Hard,
    /// A small pool: the smooth core unchanged, plus a faint long tail
    /// ([`TAIL_SHARE`]) out to [`TAIL_REACH`] times the stated radius and
    /// exactly zero at the end of it. The tail is painted in flat steps of at
    /// most one output level and is not animated by the flicker.
    Small,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowSoftness {
    None,
    Soft,
    Softer,
}

/// RGB tint and amount of light intercepted by a blocker.
#[derive(Clone, Copy, Debug)]
pub struct ShadowColor {
    pub rgb: Rgb,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerMask(pub u8);

impl LayerMask {
    pub const FLOOR: Self = Self(1);
    pub const WALLS: Self = Self(2);
    pub const PROPS: Self = Self(4);
    pub const BODIES: Self = Self(8);
    pub const ALL: Self = Self(15);
}

#[derive(Clone, Copy, Debug)]
pub struct Flicker {
    pub amount: f32,
    pub speed: f32,
    pub phase: f32,
}

impl Flicker {
    pub fn gain(self, seconds: f32) -> f32 {
        let t = seconds * self.speed + self.phase;
        1.0 - self.amount * (0.5 + 0.30 * t.sin() + 0.20 * (t * 2.31 + 1.0).sin())
    }
}

/// World-pixel light. Cone and beam directions are unit vectors. Directional
/// lights travel along `dir` and use `height` as rise per world pixel.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub pos: Vec2,
    pub shape: LightShape,
    pub color: Rgb,
    pub energy: f32,
    pub scatter: f32,
    pub blend: Blend,
    pub height: f32,
    pub falloff: Falloff,
    pub casts_shadow: bool,
    pub shadow_color: ShadowColor,
    pub shadow_softness: ShadowSoftness,
    pub flicker: Option<Flicker>,
    /// Surfaces affected by this light.
    pub layers: LayerMask,
    /// Shapes that can block this light.
    pub occluders: LayerMask,
}

impl Light {
    pub fn radial(pos: Vec2, radius: f32, color: Rgb, energy: f32) -> Self {
        Self {
            pos,
            shape: LightShape::Radial { radius },
            color,
            energy,
            scatter: 0.11,
            blend: Blend::Add,
            height: 12.0,
            falloff: Falloff::Smooth,
            casts_shadow: true,
            shadow_color: ShadowColor {
                rgb: (0, 0, 0),
                opacity: 1.0,
            },
            shadow_softness: ShadowSoftness::Soft,
            flicker: None,
            layers: LayerMask::ALL,
            occluders: LayerMask::ALL,
        }
    }

    /// How far this light reaches in world pixels: its shape's own extent, and
    /// for a small pool the whole length of its soft tail.
    pub(super) fn extent(&self) -> f32 {
        let extent = self.shape.extent();
        if self.falloff == Falloff::Small {
            extent * TAIL_REACH
        } else {
            extent
        }
    }
}

/// The falloff of one light, with the per-sample reciprocals hoisted out of
/// the pixel loop.
///
/// The core is the light's own shape weight (for a radial light
/// `1 - d²/r²`, squared for [`Falloff::Smooth`]). A [`Falloff::Small`] light
/// adds a raw tail `TAIL_SHARE * (1 - u²)²` — `u = d / reach` — which
/// [`Curve::quantise`] steps in coarse levels and which is exactly zero from
/// the end of the reach. The parts are returned separately so the caller can
/// flicker the core and leave the tail steady.
#[derive(Clone, Copy)]
pub(super) struct Curve {
    light: Light,
    inv_core: f32,
    inv_reach: f32,
    small: bool,
    /// The light's peak contribution at weight 1, in `Q8.8` multiplier units:
    /// the brightest channel times the energy and scatter it can add.
    gain: f32,
}

impl Curve {
    pub(super) fn new(light: &Light) -> Self {
        let core = light.shape.extent().max(0.001);
        let reach = light.extent().max(0.001);
        let peak = f32::from(light.color.0.max(light.color.1).max(light.color.2));
        Self {
            light: *light,
            inv_core: 1.0 / (core * core),
            inv_reach: 1.0 / (reach * reach),
            small: light.falloff == Falloff::Small
                && matches!(light.shape, LightShape::Radial { .. }),
            gain: (peak * (light.energy.abs() + light.scatter.abs())).max(1e-3),
        }
    }

    /// The light's peak contribution at weight 1, in `Q8.8` units.
    pub(super) fn gain(&self) -> f32 {
        self.gain
    }

    /// How much a falloff weight must change to move one channel of one pixel
    /// by a whole `Q8.8` level at this light's brightness. This is also the
    /// smallest contribution worth painting.
    pub(super) fn step(&self) -> f32 {
        MIN_GAIN / self.gain
    }

    /// The raw `(core, tail)` at squared distance `d2` from the light. The
    /// shape, not the terminal: [`Curve::quantise`] is what a pixel sees.
    pub(super) fn parts(&self, delta: Vec2, d2: f32, cone_limit: f32) -> (f32, f32) {
        let w = if matches!(self.light.shape, LightShape::Radial { .. }) {
            1.0 - d2 * self.inv_core
        } else {
            self.light.shape.weight(delta, cone_limit)
        };
        if self.small {
            let core = if w > 0.0 { w * w } else { 0.0 };
            let q = d2 * self.inv_reach;
            if q >= 1.0 {
                return (core, 0.0);
            }
            let a = 1.0 - q;
            return (core, a * a * TAIL_SHARE);
        }
        if w <= 0.0 {
            return (0.0, 0.0);
        }
        let core = match self.light.falloff {
            Falloff::Linear => w,
            Falloff::Hard => (w * 1.6).min(1.0),
            Falloff::Smooth | Falloff::Small => w * w,
        };
        (core, 0.0)
    }

    /// The flat steps the far tail is painted in: never more than one output
    /// level each, so a still frame's changed cells do not grow and the end of
    /// the pool cannot ring.
    pub(super) fn quantise(&self, tail: f32) -> f32 {
        let step = self.step();
        if tail < step {
            return 0.0;
        }
        (tail / step).floor() * step
    }
}

// Existing combat art supplies small screen-space pools using this adapter.
#[derive(Clone, Copy)]
pub(super) struct PointLight {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub color: Rgb,
    pub intensity: u16,
    pub scatter: u16,
    pub priority: u8,
    pub occlude: bool,
    pub hard: bool,
}

const CANDIDATES: usize = 256;
const MAX_MARKS: usize = 12_288;
const MAX_BODIES: usize = 8;
/// Shaped lights kept per frame. A full list evicts its lowest priority.
const MAX_SHAPED: usize = 64;
/// Consecutive over-budget frames before the controller drops a tier.
const SLOW_FRAMES: u8 = 8;
/// Consecutive comfortably cheap frames before it climbs back one tier. The
/// count doubles per relapse (see `QualityController::backoff`).
const FAST_FRAMES: u16 = 180;
/// A tier regained and lost again within this many frames is a relapse.
const PROBATION_FRAMES: u16 = 600;
const MAX_BACKOFF: u8 = 4;
/// Share of the gap the ambient cap closes per frame (about 0.4 s at 60 FPS).
const CAP_EASE: f32 = 0.12;

/// Share of a small light's core emission that carries into its soft tail.
/// The tail is `TAIL_SHARE * (1 - u²)²` for `u = distance / reach`, so 60 % of
/// the reach already holds under 8 % of the core.
pub(super) const TAIL_SHARE: f32 = 0.19;
/// How far a small light reaches, as a multiple of its stated radius.
pub(super) const TAIL_REACH: f32 = 1.7;
/// The largest stated radius a _point_ light may have and still count as a
/// small pool. Above it the source is a room-scale light (the rat's aura, a
/// boss presence, a nuke) and keeps its own size.
const SMALL_LIGHT_MAX: f32 = 52.0;
/// A contribution smaller than this, in `Q8.8` multiplier units, is exactly
/// zero, and a small light's tail is painted in flat steps of this size. It is
/// the one knob between a long soft tail and terminal output: every band edge
/// of a *moving* halo repaints cells. Measured with `--prof-bench --mode app
/// --still` (Sewer, 5:00): 2.0 cost +26 % / +32 % bytes a frame at 120x40 /
/// 200x60 against the build before the tail existed, 4.0 +20 % / +24 %, 6.0
/// +14 % / +17 %, 7.0 +13 % / +15 %. At 7.0 a dim halo (gain under ~40) keeps
/// its old size and only the brighter small lights (lamps, chests, emissive
/// props) throw a visible skirt.
pub(super) const MIN_GAIN: f32 = 7.0;
/// The base level of every floor and zone, as a share of the theme's tint.
/// Batch 8 lowered it by 40 %: the room is a dark floor with pools of light on
/// it, so the pools, the auras and the enemy glints read as light.
pub(super) const AMBIENT_SCALE: f32 = 0.60;
/// The darkest a floor can be, in `Q8.8` (`5.5 %`): no zone and no theme is
/// ever pure black, so shapes still read by their own tint.
pub(super) const AMBIENT_FLOOR: u16 = 14;

/// Ambient gain of a light zone; `None` is unzoned floor.
pub(super) fn zone_gain(zone: Option<LightZoneKind>) -> f32 {
    match zone {
        Some(LightZoneKind::Start) => 0.95,
        Some(LightZoneKind::Room) => 0.80,
        Some(LightZoneKind::Corridor) => 0.56,
        // Unreachable since batch 12 (no map has an Arena zone); the arm stays
        // because the enum variant is matched exhaustively here.
        Some(LightZoneKind::Arena) => 0.64,
        Some(LightZoneKind::Blackout) => 0.26,
        Some(LightZoneKind::Maze) => 0.25,
        None => 0.70,
    }
}

/// The ambient multiplier of one sample, in `Q8.8`: the theme's tint times the
/// zone's gain, lowered by [`AMBIENT_SCALE`] and never below [`AMBIENT_FLOOR`].
#[inline]
pub(super) fn ambient_level(theme_mul: u16, gain: f32) -> u16 {
    ambient_level_scaled(theme_mul, gain, AMBIENT_SCALE)
}

/// [`ambient_level`] with the scale given (the effects' tone curve picks its
/// own, `upsample::ambient_scale`).
#[inline]
pub(super) fn ambient_level_scaled(theme_mul: u16, gain: f32, scale: f32) -> u16 {
    let level = (f32::from(theme_mul) * gain * scale) as u16;
    level.max(AMBIENT_FLOOR)
}

/// The `mul` clamp of [`Sample::light`] with the effects off.
const MUL_CLAMP: i32 = 350;

/// What one lighting pass runs with, handed to the grid's methods.
#[derive(Clone, Copy)]
pub(super) struct Pass {
    pub mode: LightingMode,
    pub elapsed: f32,
    pub theme: Theme,
    /// The batch-13 effects are on (`LightScene::fx`).
    pub fx: bool,
    /// The ambient share of the theme tint ([`AMBIENT_SCALE`] with fx off).
    pub ambient_scale: f32,
}

pub(super) struct Candidates {
    points: Vec<PointLight>,
    weakest: usize,
}

impl Candidates {
    pub(super) fn push(&mut self, point: PointLight) {
        if self.points.len() < CANDIDATES {
            if self.points.is_empty() || point.priority < self.points[self.weakest].priority {
                self.weakest = self.points.len();
            }
            self.points.push(point);
            return;
        }
        if point.priority > self.points[self.weakest].priority {
            self.points[self.weakest] = point;
            self.weakest = self
                .points
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| p.priority)
                .map_or(0, |(i, _)| i);
        }
    }
}

#[derive(Clone, Copy)]
struct Mark {
    index: usize,
    source: Rgb,
    readable: bool,
    /// `Some(tint)`: keep the drawn pixel but lift it to at least `tint`. This
    /// is the enemy rim — a self-lit pixel that the darkness cannot swallow.
    glint: Option<Rgb>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Sample {
    /// Multiplier per channel, `Q8.8`.
    pub(super) mul: [u16; 3],
    /// Additive light toward white per channel, `Q8.8`.
    pub(super) add: [i16; 3],
    /// Mix tint (`tint[3]` is the amount).
    pub(super) tint: [u8; 4],
}

/// The light grid: one [`Sample`] per `stride × stride` block of world pixels,
/// snapped to world space so the pools do not crawl as the camera pans.
pub(super) struct Grid {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) stride: usize,
    /// World position of sample (0, 0) (a block centre).
    pub(super) start: Vec2,
    /// Pixel (0, 0)'s offset inside its block.
    pub(super) offset: (usize, usize),
    pub(super) samples: Vec<Sample>,
    pub(super) surfaces: Vec<Surface>,
    /// Batch-13 light record per sample (`fx.rs`); empty while fx is off.
    pub(super) info: Vec<LightInfo>,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            w: 0,
            h: 0,
            stride: 4,
            start: Vec2::ZERO,
            offset: (0, 0),
            samples: Vec::new(),
            surfaces: Vec::new(),
            info: Vec::new(),
        }
    }
}

/// Cost-driven tier switching. `penalty` tiers are added to the base mode.
///
/// Dropping is quick (`SLOW_FRAMES` over budget in a row) and climbing is slow
/// (`FAST_FRAMES` well under budget in a row). A cheap frame at the lower tier
/// does not prove the tier above fits, so a climb that is lost again inside
/// `PROBATION_FRAMES` doubles the next climb's wait: a scene that sits between
/// two tiers settles on the cheaper one instead of flapping every few seconds.
/// A climb that survives its probation earns one step of that wait back.
#[derive(Default)]
struct QualityController {
    penalty: usize,
    slow: u8,
    fast: u16,
    /// Frames left in which losing the regained tier counts as a relapse.
    probation: u16,
    /// Left shift on `FAST_FRAMES`, `0..=MAX_BACKOFF`.
    backoff: u8,
}

pub struct LightMap {
    pub(super) lights: Candidates,
    shaped: Vec<(Light, u8)>,
    marks: Vec<Mark>,
    /// Screen-space enemy rim glints for this frame, filled by the gather and
    /// stamped as marks once the sprites are final (`sources.rs`).
    pub(super) rims: Vec<(i32, i32, Rgb)>,
    grid: Grid,
    geometry: Geometry,
    static_shadows: Vec<ShadowProfile>,
    moving_shadows: [ShadowProfile; 3],
    bodies: Vec<Body>,
    controller: QualityController,
    /// Reused coverage scratch for the ambient smoke plumes (`ambience.rs`).
    pub(super) plume: Vec<u8>,
    /// Eased ambient gain of the zone the rat stands in. Nothing on screen is
    /// lit above it, so a room he has not walked into never outshines the
    /// passage he is in; it brightens as he enters. `None` until he has stood
    /// in a zone (an unzoned level is not capped at all).
    ambient_cap: Option<f32>,
    /// Batch 13: the frame's strongest lights (`FxFrame::lights`).
    fx_lights: Vec<FxLight>,
    /// Batch 13: one tangent normal per world pixel (`surface.rs` writes).
    normals: Vec<Normal>,
    /// Batch 13: `softshadow.rs`'s cache beside each static shadow profile
    /// and each moving shadow slot.
    static_soft: Vec<SoftProfile>,
    moving_soft: [SoftProfile; 3],
    /// Whether the last pass ran the effects (widens the cost budget).
    fx_active: bool,
}

impl Default for LightMap {
    fn default() -> Self {
        Self::new()
    }
}

impl LightMap {
    pub fn new() -> Self {
        Self {
            lights: Candidates {
                points: Vec::with_capacity(CANDIDATES),
                weakest: 0,
            },
            shaped: Vec::with_capacity(MAX_SHAPED),
            marks: Vec::with_capacity(MAX_MARKS),
            rims: Vec::new(),
            grid: Grid::default(),
            geometry: Geometry::default(),
            static_shadows: Vec::new(),
            moving_shadows: [ShadowProfile::default(); 3],
            bodies: Vec::with_capacity(MAX_BODIES),
            controller: QualityController::default(),
            plume: Vec::new(),
            ambient_cap: None,
            fx_lights: Vec::with_capacity(fx::MAX_FX_LIGHTS),
            normals: Vec::new(),
            static_soft: Vec::new(),
            moving_soft: Default::default(),
            fx_active: false,
        }
    }

    pub fn quality(&self, base: LightingMode) -> LightingMode {
        #[cfg(test)]
        if base == LightingMode::Off {
            return base;
        }
        LightingMode::LIVE[(base.rank() + self.controller.penalty).min(3)]
    }

    pub(crate) fn prof_reset_controller(&mut self) {
        self.controller = QualityController::default();
    }

    pub fn observe_cost(&mut self, mode: LightingMode, elapsed: Duration, pixels: usize) {
        if crate::prof::controller_frozen() {
            return;
        }
        let mut budget = 160.0 + pixels as f64 * 0.035;
        if self.fx_active {
            budget += pixels as f64 * fx::FX_BUDGET_PER_PX;
        }
        let micros = elapsed.as_secs_f64() * 1e6;
        let state = &mut self.controller;
        if state.probation != 0 {
            state.probation -= 1;
            if state.probation == 0 {
                // The regained tier held: forgive one relapse.
                state.backoff = state.backoff.saturating_sub(1);
            }
        }
        if micros > budget && mode.rank() < LightingMode::Compact.rank() {
            state.fast = 0;
            state.slow += 1;
            if state.slow >= SLOW_FRAMES {
                state.slow = 0;
                // Never bank more tiers than exist, or a later, cheaper base
                // mode would have to pay them all back before recovering.
                state.penalty = (state.penalty + 1).min(LightingMode::LIVE.len() - 1);
                if state.probation != 0 {
                    state.probation = 0;
                    state.backoff = (state.backoff + 1).min(MAX_BACKOFF);
                }
            }
        } else {
            state.slow = 0;
            if micros < budget * 0.56 && state.penalty != 0 {
                state.fast += 1;
                if state.fast >= FAST_FRAMES << state.backoff {
                    state.fast = 0;
                    state.penalty -= 1;
                    state.probation = PROBATION_FRAMES;
                }
            } else {
                state.fast = 0;
            }
        }
    }

    pub fn begin_frame(&mut self) {
        self.shaped.clear();
        self.marks.clear();
        self.rims.clear();
    }

    /// Submit a light for this frame. The list holds `MAX_SHAPED`; once full,
    /// a light replaces the lowest-priority entry it outranks (so a screen of
    /// priority-1 halos cannot crowd out the rat's aura) and is dropped
    /// otherwise.
    pub fn push_shaped(&mut self, light: Light, priority: u8) {
        push_shaped(&mut self.shaped, light, priority);
    }

    /// A cheap coloured halo from a short-lived source, in world pixels.
    pub fn emit(&mut self, pos: Vec2, color: Rgb, radius: f32, strength: f32) {
        let mut light = Light::radial(pos, radius, color, strength);
        light.scatter = strength * 0.32;
        light.casts_shadow = false;
        // A halo is the archetypal small light: bright core, long faint tail.
        light.falloff = Falloff::Small;
        self.push_shaped(light, 1);
    }

    /// Preserve the sprite pixels just painted. A later sprite covering the
    /// marked colour replaces it, as it would elsewhere in the draw order.
    pub fn mark_emissive_rect(&mut self, pb: &PixelBuffer, x: i32, y: i32, w: i32, h: i32) {
        for y in y.max(0)..(y + h).min(pb.h as i32) {
            for x in x.max(0)..(x + w).min(pb.w as i32) {
                self.mark(pb, x, y, false);
            }
        }
    }

    pub fn mark_readable(&mut self, pb: &PixelBuffer, x: i32, y: i32) {
        self.mark(pb, x, y, true);
    }

    /// Keep one pixel of a body lit in `tint` whatever the ambient and the
    /// lights do. Ordinary enemies are drawn *before* the lighting pass, so
    /// without this a body in an unlit room is indistinguishable from the
    /// floor; a rim glint leaves the silhouette and its own colour readable.
    pub fn mark_glint(&mut self, pb: &PixelBuffer, x: i32, y: i32, tint: Rgb) {
        if x >= 0 && y >= 0 && x < pb.w as i32 && y < pb.h as i32 && self.marks.len() < MAX_MARKS {
            let index = y as usize * pb.w + x as usize;
            self.marks.push(Mark {
                index,
                source: pb.pixels()[index],
                readable: false,
                glint: Some(tint),
            });
        }
    }

    pub fn mark_emissive_highlights(&mut self, pb: &PixelBuffer, center: (i32, i32), radius: i32) {
        let (x, y) = center;
        for y in (y - radius).max(0)..=(y + radius).min(pb.h as i32 - 1) {
            for x in (x - radius).max(0)..=(x + radius).min(pb.w as i32 - 1) {
                let (r, g, b) = pb.pixel_at(x as usize, y as usize);
                if r.max(g).max(b) >= 232 {
                    self.mark(pb, x, y, false);
                }
            }
        }
    }

    fn mark(&mut self, pb: &PixelBuffer, x: i32, y: i32, readable: bool) {
        if x >= 0 && y >= 0 && x < pb.w as i32 && y < pb.h as i32 && self.marks.len() < MAX_MARKS {
            let index = y as usize * pb.w + x as usize;
            self.marks.push(Mark {
                index,
                source: pb.pixels()[index],
                readable,
                glint: None,
            });
        }
    }

    pub fn apply_scene(&mut self, pb: &mut PixelBuffer, scene: &LightScene<'_>) {
        #[cfg(test)]
        if scene.mode == LightingMode::Off {
            return;
        }
        if pb.w == 0 || pb.h == 0 {
            return;
        }
        // The batch-13 effects (`fx.rs`, the stage files). With `fx` off
        // every line below is the batch-12 pass exactly.
        let fx = scene.fx;
        self.fx_active = fx;
        self.fx_lights.clear();
        {
            let _p = crate::prof::scope(crate::prof::Stage::LSync);
            self.geometry.sync(scene.level);
        }
        let theme = scene.level.theme;
        let pass = Pass {
            mode: scene.mode,
            elapsed: scene.elapsed,
            theme,
            fx,
            ambient_scale: if fx {
                upsample::ambient_scale(theme, scene.mode)
            } else {
                AMBIENT_SCALE
            },
        };
        let profile = super::theme_ambient(theme);
        // The rat's own zone bounds the ambient everywhere and the reach of
        // lamps hung in brighter zones. Eased per frame (render-only state), and
        // held while he crosses unzoned cells so a doorway does not pulse.
        let cap = match (
            self.geometry.surface(scene.player.pos).zone,
            self.ambient_cap,
        ) {
            (Some(zone), Some(cap)) => {
                let cap = cap + (zone_gain(Some(zone)) - cap) * CAP_EASE;
                self.ambient_cap = Some(cap);
                cap
            }
            (Some(zone), None) => {
                let cap = zone_gain(Some(zone));
                self.ambient_cap = Some(cap);
                cap
            }
            (None, cap) => cap.unwrap_or(1.0),
        };
        {
            let _p = crate::prof::scope(crate::prof::Stage::LPrepare);
            self.grid
                .prepare(pb, scene.cam, &self.geometry, profile, cap, pass);
        }
        crate::prof::count(crate::prof::Stage::CSamples, self.grid.samples.len() as u64);
        self.lights.points.clear();
        self.lights.weakest = 0;
        let phase = {
            let _p = crate::prof::scope(crate::prof::Stage::LGather);
            self.gather_lights(pb.w, pb.h, scene)
        };
        // Enemy rim glints: taken now, while the body pixels are final and the
        // lighting has not yet covered them. (The effects' directional rim,
        // `specular.rs`, may replace them.)
        if !fx || specular::keep_glints(scene.mode) {
            for i in 0..self.rims.len() {
                let (x, y, tint) = self.rims[i];
                self.mark_glint(pb, x, y, tint);
            }
        }
        crate::prof::count(crate::prof::Stage::CPoints, self.lights.points.len() as u64);
        let _pstatic = crate::prof::scope(crate::prof::Stage::LStaticLights);
        self.static_shadows
            .resize(scene.level.objects.lights.len(), ShadowProfile::default());
        self.static_soft
            .resize(scene.level.objects.lights.len(), SoftProfile::default());
        for (i, fixture) in scene.level.objects.lights.iter().enumerate() {
            let mut light = fixtures::light(*fixture, theme, profile, phase, scene);
            if !self.grid.overlaps(light) || light.energy <= 0.001 {
                continue;
            }
            if fixture.kind != LightFixtureKind::Sun {
                // A lamp hung in a brighter zone than the rat's is turned down
                // by the same ratio as that zone's ambient.
                let own = zone_gain(self.geometry.surface(fixture.pos).zone);
                if own > cap {
                    light.energy *= cap / own;
                    light.scatter *= cap / own;
                }
            }
            crate::prof::count(crate::prof::Stage::CStaticLit, 1);
            if fixture.kind == LightFixtureKind::Sun {
                let _p = crate::prof::scope(crate::prof::Stage::LSun);
                self.sun(pb, scene, light);
                continue;
            }
            let tag = LightTag::Fixture(fixture.kind);
            if fx {
                fx::push_fx_light(
                    &mut self.fx_lights,
                    FxLight::of(&light, tag, fx::flicker_of(&light, scene.elapsed)),
                );
            }
            let shadow = &mut self.static_shadows[i];
            let rebaked = {
                let _p = crate::prof::scope(crate::prof::Stage::LStaticBake);
                let rebaked = shadow.bake(&self.geometry, light);
                if rebaked {
                    crate::prof::count(crate::prof::Stage::CStaticBakeMiss, 1);
                }
                rebaked
            };
            if fx {
                softshadow::bake(&mut self.static_soft[i], &self.geometry, &light, tag, rebaked);
            }
            let shadow: &ShadowProfile = shadow;
            let soft = Some(&self.static_soft[i]);
            let _p = crate::prof::scope(crate::prof::Stage::LStaticSplat);
            self.grid.splat(light, Some(shadow), soft, tag, pass);
            // A wall lamp also washes the face it hangs on and the floor at its
            // foot (batch 12): a second, very small and very bright source at
            // the mount, sharing the fan's cached bake so it cannot leak
            // through its own wall. One more splat per sconce in view, with no
            // second bake.
            if fixture.kind == LightFixtureKind::Sconce {
                let wash = fixtures::sconce_wash(&light, fixture.seed, scene.elapsed);
                if self.grid.overlaps(wash) {
                    self.grid
                        .splat(wash, Some(shadow), soft, LightTag::SconceWash, pass);
                }
            }
        }
        drop(_pstatic);
        let _prank = crate::prof::scope(crate::prof::Stage::LRank);
        let (cap, shadows) = scene.mode.budget();
        let points = &mut self.lights.points;
        let rat = scene.player.pos - scene.cam;
        let rank = |a: &PointLight, b: &PointLight| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| {
                    Vec2::new(a.x, a.y)
                        .dist_sq(rat)
                        .total_cmp(&Vec2::new(b.x, b.y).dist_sq(rat))
                })
                .then_with(|| a.x.total_cmp(&b.x))
                .then_with(|| a.y.total_cmp(&b.y))
        };
        if points.len() > cap {
            points.select_nth_unstable_by(cap, rank);
            points.truncate(cap);
        }
        points.sort_unstable_by(rank);
        for &p in points.iter() {
            let mut light = Light::radial(
                scene.cam + Vec2::new(p.x, p.y),
                p.radius,
                p.color,
                p.intensity as f32 / 256.0,
            );
            light.scatter = p.scatter as f32 / 256.0;
            light.casts_shadow = p.occlude && p.radius >= 26.0;
            light.falloff = if p.hard {
                Falloff::Hard
            } else if p.radius <= SMALL_LIGHT_MAX {
                // A halo, a gem, an emissive prop or a fixture smaller than a
                // room lamp: the core it always had, and a long faint tail.
                Falloff::Small
            } else {
                Falloff::Smooth
            };
            push_shaped(&mut self.shaped, light, p.priority);
        }
        // Prefer the rat, then the brighter lights and foreground flashes.
        self.shaped.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                a.0.pos
                    .dist_sq(scene.player.pos)
                    .total_cmp(&b.0.pos.dist_sq(scene.player.pos))
            })
        });
        drop(_prank);
        crate::prof::count(crate::prof::Stage::CShaped, self.shaped.len() as u64);
        let mut shadow_index = 0;
        for &(mut light, priority) in self.shaped.iter().take(cap) {
            if !self.grid.overlaps(light) {
                continue;
            }
            crate::prof::count(crate::prof::Stage::CShapedLit, 1);
            let directional = matches!(light.shape, LightShape::Directional { .. });
            let tag = if directional {
                LightTag::Sun
            } else {
                LightTag::Moving { priority }
            };
            if fx {
                fx::push_fx_light(
                    &mut self.fx_lights,
                    FxLight::of(&light, tag, fx::flicker_of(&light, scene.elapsed)),
                );
            }
            if directional {
                self.grid.sun(light, &self.geometry, &[], scene.mode, fx);
                continue;
            }
            let (shadow, soft) = if light.casts_shadow && shadow_index < shadows {
                let profile = &mut self.moving_shadows[shadow_index];
                // Stabilise the visibility origin at pixel scale as the rat moves.
                light.pos = Vec2::new(light.pos.x.round(), light.pos.y.round());
                let rebaked = {
                    let _p = crate::prof::scope(crate::prof::Stage::LMovingBake);
                    let rebaked = profile.bake(&self.geometry, light);
                    if rebaked {
                        crate::prof::count(crate::prof::Stage::CMovingBakeMiss, 1);
                    }
                    rebaked
                };
                if fx {
                    softshadow::bake(
                        &mut self.moving_soft[shadow_index],
                        &self.geometry,
                        &light,
                        tag,
                        rebaked,
                    );
                }
                let soft = &self.moving_soft[shadow_index];
                shadow_index += 1;
                (Some(&*profile), Some(soft))
            } else {
                (None, None)
            };
            let _p = crate::prof::scope(crate::prof::Stage::LMovingSplat);
            self.grid.splat(light, shadow, soft, tag, pass);
        }
        fx::sort_fx_lights(&mut self.fx_lights);
        let frame = FxFrame {
            mode: scene.mode,
            depth: scene.depth,
            theme,
            level: scene.level,
            cam: Vec2::new(scene.cam.x.round(), scene.cam.y.round()),
            elapsed: scene.elapsed,
            step8: fx::step8(scene.elapsed),
            rat: scene.player.pos,
            lights: &self.fx_lights,
            ambient: profile.mul,
        };
        if fx {
            let _p = crate::prof::scope(crate::prof::Stage::LComposite);
            upsample::bounce(&mut self.grid, &frame);
            air::grid_pass(&mut self.grid, &frame);
        }
        let hp = (scene.player.hp / scene.player.max_hp().max(1.0)).clamp(0.0, 1.0);
        if hp < 0.35 {
            let _p = crate::prof::scope(crate::prof::Stage::LVignette);
            self.grid.vignette(hp, scene.elapsed);
        }
        // A mark outlives its pixel when a later sprite painted over it, and
        // (defensively) when the buffer it was taken from is not this one.
        crate::prof::count(crate::prof::Stage::CMarks, self.marks.len() as u64);
        let _pm = crate::prof::scope(crate::prof::Stage::LMarks);
        self.marks
            .retain(|m| pb.pixels().get(m.index) == Some(&m.source));
        drop(_pm);
        {
            let _p = crate::prof::scope(crate::prof::Stage::LComposite);
            if fx {
                // The effects' pixel stages, in the contract's order: the
                // albedo pass, the shade, then the lit-space passes.
                let (w, h) = (pb.w, pb.h);
                self.normals.clear();
                self.normals.resize(w * h, [0, 0]);
                let (px, mat) = pb.split_mat();
                let mut canvas = Canvas { w, h, px, mat };
                surface::apply(&self.grid, &mut canvas, &mut self.normals, &frame);
                upsample::composite(&self.grid, &mut canvas, &frame);
                specular::apply(&self.grid, &mut canvas, &self.normals, &frame);
                softshadow::reflections(&self.grid, &mut canvas, &self.normals, &frame);
                air::composite(&self.grid, &mut canvas, &frame);
                motes::draw(&self.grid, &mut canvas, &frame);
            } else {
                self.grid.composite(pb, scene.depth);
            }
        }
        let _pm = crate::prof::scope(crate::prof::Stage::LMarks);
        for mark in &self.marks {
            let pixel = &mut pb.pixels_mut()[mark.index];
            if let Some(tint) = mark.glint {
                // A rim glint only ever lifts the pixel: it never dims a body
                // that a light is already shining on.
                *pixel = (
                    pixel.0.max(tint.0),
                    pixel.1.max(tint.1),
                    pixel.2.max(tint.2),
                );
            } else if mark.readable {
                // A tiny silhouette highlight reads at the edge of darkness.
                // Lift dark coats only; fully lit actors keep their own colour.
                let minimum: u8 = if scene.depth == ColorDepth::Ansi16 {
                    118
                } else {
                    78
                };
                let lift = minimum.saturating_sub(super::luminance(*pixel));
                *pixel = (
                    pixel.0.saturating_add(lift),
                    pixel.1.saturating_add(lift),
                    pixel.2.saturating_add(lift),
                );
            } else {
                *pixel = (
                    pixel.0.max(mark.source.0),
                    pixel.1.max(mark.source.1),
                    pixel.2.max(mark.source.2),
                );
            }
        }
    }

    fn sun(&mut self, pb: &PixelBuffer, scene: &LightScene<'_>, light: Light) {
        self.bodies.clear();
        let add_body = |bodies: &mut Vec<Body>, body: Body| {
            let p = body.pos - scene.cam;
            if p.x > -40.0
                && p.y > -40.0
                && p.x < pb.w as f32 + 40.0
                && p.y < pb.h as f32 + 40.0
                && bodies.len() < MAX_BODIES
            {
                bodies.push(body);
            }
        };
        add_body(
            &mut self.bodies,
            Body {
                pos: scene.player.pos,
                radius: 4.5,
                height: 8.0,
            },
        );
        if scene.mode != LightingMode::Compact {
            for entity in scene.ecs.iter_entities() {
                if let Some(boss) = entity.get::<Boss>() {
                    // A bird overhead or a cat mid-pounce draws its own
                    // detached shadow; a ground shadow at its feet would be a
                    // second one. A wreck casts nothing new either.
                    let magpie_up = matches!(
                        boss.state,
                        crate::game::ecs::systems::boss::BossState::Magpie(st) if st.flying()
                    );
                    if boss.is_airborne() || magpie_up || boss.dead {
                        continue;
                    }
                    add_body(
                        &mut self.bodies,
                        Body {
                            pos: boss.pos,
                            radius: 13.0,
                            height: 23.0,
                        },
                    );
                } else if let Some(e) = entity
                    .get::<Enemy>()
                    .filter(|e| e.kind.archetype() == EnemyKind::Brute && e.alive())
                {
                    add_body(
                        &mut self.bodies,
                        Body {
                            pos: e.pos,
                            radius: e.radius,
                            height: 13.0,
                        },
                    );
                }
            }
        }
        // Sliding load stands above the deck and follows the art/physics clock.
        for slide in &scene.level.trailer.slides {
            let pos = slide.rest
                + slide.dir * crate::game::level::TrailerDeck::slide_offset(slide, scene.elapsed);
            add_body(
                &mut self.bodies,
                Body {
                    pos,
                    radius: 9.0,
                    height: 17.0,
                },
            );
        }
        if scene.fx {
            fx::push_fx_light(&mut self.fx_lights, FxLight::of(&light, LightTag::Sun, 1.0));
        }
        self.grid
            .sun(light, &self.geometry, &self.bodies, scene.mode, scene.fx);
    }
}

/// The body of [`LightMap::push_shaped`], on the list alone so it can run
/// while another field of the map is borrowed.
fn push_shaped(shaped: &mut Vec<(Light, u8)>, light: Light, priority: u8) {
    if shaped.len() < MAX_SHAPED {
        shaped.push((light, priority));
        return;
    }
    // First of the lowest priority: deterministic, and only on overflow.
    let mut weakest = 0;
    for (i, entry) in shaped.iter().enumerate() {
        if entry.1 < shaped[weakest].1 {
            weakest = i;
        }
    }
    if shaped[weakest].1 < priority {
        shaped[weakest] = (light, priority);
    }
}

impl Grid {
    fn prepare(
        &mut self,
        pb: &PixelBuffer,
        cam: Vec2,
        geometry: &Geometry,
        profile: ThemeAmbient,
        cap: f32,
        pass: Pass,
    ) {
        self.stride = pass.mode.stride();
        let s = self.stride as i32;
        let pos = (cam.x.round() as i32, cam.y.round() as i32);
        self.offset = (pos.0.rem_euclid(s) as usize, pos.1.rem_euclid(s) as usize);
        self.start = Vec2::new(
            (pos.0.div_euclid(s) * s + s / 2) as f32,
            (pos.1.div_euclid(s) * s + s / 2) as f32,
        );
        self.w = (pb.w + self.offset.0).div_ceil(self.stride);
        self.h = (pb.h + self.offset.1).div_ceil(self.stride);
        // Reserve for the finest tier and any alignment within a snapped cell
        // on viewport growth, rather than growing when the camera first pans.
        let max_len = (pb.w + 3).div_ceil(4) * (pb.h + 3).div_ceil(4);
        if self.samples.capacity() < max_len {
            self.samples.reserve_exact(max_len - self.samples.len());
            self.surfaces.reserve_exact(max_len - self.surfaces.len());
        }
        self.samples.resize(self.w * self.h, Sample::default());
        self.surfaces.resize(self.w * self.h, Surface::default());
        self.info.clear();
        if pass.fx {
            if self.info.capacity() < max_len {
                self.info.reserve_exact(max_len);
            }
            self.info.resize(self.w * self.h, LightInfo::default());
        }
        let scale = pass.ambient_scale;
        for i in 0..self.samples.len() {
            let p = self.point(i % self.w, i / self.w);
            let surface = geometry.surface(p);
            let gain = zone_gain(surface.zone).min(cap);
            self.samples[i] = Sample {
                mul: profile.mul.map(|c| ambient_level_scaled(c, gain, scale)),
                ..Sample::default()
            };
            self.surfaces[i] = surface;
        }
    }

    fn point(&self, x: usize, y: usize) -> Vec2 {
        self.start + Vec2::new((x * self.stride) as f32, (y * self.stride) as f32)
    }

    fn overlaps(&self, light: Light) -> bool {
        let r = light.extent();
        let end = self.point(self.w, self.h);
        light.pos.x + r >= self.start.x
            && light.pos.x - r <= end.x
            && light.pos.y + r >= self.start.y
            && light.pos.y - r <= end.y
    }

    /// Add one light to every sample it reaches. With the effects on it also
    /// folds the light into each sample's [`LightInfo`] (the dominant
    /// direction, colour and the specular energy), offers each sample to
    /// `air::splat_sample` and takes its visibility from
    /// `softshadow::visibility`.
    fn splat(
        &mut self,
        light: Light,
        shadow: Option<&ShadowProfile>,
        soft: Option<&SoftProfile>,
        tag: LightTag,
        pass: Pass,
    ) {
        let (mode, elapsed, theme) = (pass.mode, pass.elapsed, pass.theme);
        let radius = light.extent();
        let centre = light.pos - self.start;
        let spacing = self.stride as f32;
        let lo = |p: f32| ((p - radius) / spacing).ceil().max(0.0) as usize;
        let hi = |p: f32, size: usize| {
            (((p + radius) / spacing).floor() as i32 + 1).clamp(0, size as i32) as usize
        };
        let softness = if mode == LightingMode::Full {
            light.shadow_softness
        } else {
            ShadowSoftness::None
        };
        let flicker = light.flicker.map_or(1.0, |f| f.gain(elapsed));
        let cone_limit = match light.shape {
            LightShape::Cone { half_angle, .. } => half_angle.cos(),
            _ => 0.0,
        };
        let curve = Curve::new(&light);
        // A light too dim to move any channel by a whole level lights nothing.
        let min_amount = curve.step();
        if min_amount > 1.0 {
            return;
        }
        let fx_on = pass.fx;
        let air_on = fx_on && air::wants(&light, tag, mode);
        let floor_glint = !fx_on || softshadow::keep_floor_glint(mode);
        let ceiling = if fx_on {
            upsample::MUL_CEILING
        } else {
            MUL_CLAMP
        };
        let peak = f32::from(light.color.0.max(light.color.1).max(light.color.2));
        let mut touched = 0u64;
        for y in lo(centre.y)..hi(centre.y, self.h) {
            for x in lo(centre.x)..hi(centre.x, self.w) {
                let delta = self.point(x, y) - light.pos;
                let (core, tail) = curve.parts(delta, delta.len_sq(), cone_limit);
                let tail = curve.quantise(tail);
                if core <= 0.0 && tail <= 0.0 {
                    continue;
                }
                let i = y * self.w + x;
                let surface = self.surfaces[i];
                if surface.layer & light.layers.0 == 0 {
                    continue;
                }
                let visibility = match shadow {
                    None => 1.0,
                    Some(s) if fx_on => softshadow::visibility(
                        s,
                        soft,
                        &ShadowQuery {
                            light: &light,
                            tag,
                            delta,
                            surface,
                            softness,
                            mode,
                        },
                    ),
                    Some(s) => s.visibility(delta, surface, softness),
                };
                if air_on {
                    air::splat_sample(
                        &mut self.info[i],
                        &SampleHit {
                            light: &light,
                            tag,
                            delta,
                            point: light.pos + delta,
                            core,
                            tail,
                            visibility,
                            surface,
                            mode,
                        },
                    );
                }
                let face = if surface.height > 0.0 {
                    facing_gain(surface, -delta)
                } else {
                    1.0
                };
                let glint = if floor_glint
                    && shadow.is_some()
                    && surface.height == 0.0
                    && matches!(theme, Theme::Sewer | Theme::Lab)
                {
                    let streak =
                        (1.0 - delta.x.abs() / 26.0 - (delta.y - 11.0).abs() / 36.0).max(0.0);
                    // A narrow reflection runs out along the wet/metal floor.
                    streak * streak * 0.085
                } else {
                    0.0
                };
                // The flicker animates the core only: a faint, far tail that
                // breathed with it would repaint cells for no visible reason.
                let amount = (core * face + glint) * flicker + tail * face;
                if amount.abs() < min_amount {
                    continue;
                }
                touched += 1;
                self.samples[i].light(light, amount, visibility, ceiling);
                if fx_on && matches!(light.blend, Blend::Add) {
                    let shadowed = (1.0 - visibility) * light.shadow_color.opacity;
                    fx::note_light(
                        &mut self.info[i],
                        &light,
                        -delta,
                        light.height,
                        amount * (1.0 - shadowed) * light.energy * peak,
                    );
                }
            }
        }
        crate::prof::count(crate::prof::Stage::CSplatSamples, touched);
    }

    fn sun(
        &mut self,
        light: Light,
        geometry: &Geometry,
        bodies: &[Body],
        mode: LightingMode,
        fx_on: bool,
    ) {
        let LightShape::Directional { dir } = light.shape else {
            return;
        };
        let ray = -dir.normalized();
        let ceiling = if fx_on {
            upsample::MUL_CEILING
        } else {
            MUL_CLAMP
        };
        let peak = f32::from(light.color.0.max(light.color.1).max(light.color.2));
        for i in 0..self.samples.len() {
            let surface = self.surfaces[i];
            if surface.zone == Some(LightZoneKind::Maze) || surface.layer & light.layers.0 == 0 {
                continue;
            }
            let p = self.point(i % self.w, i / self.w);
            let mut vis = if light.casts_shadow {
                geometry.sun_visibility(p, surface, light, ray, bodies)
            } else {
                1.0
            };
            if light.casts_shadow
                && mode == LightingMode::Full
                && light.shadow_softness != ShadowSoftness::None
            {
                let side = ray.perp()
                    * if light.shadow_softness == ShadowSoftness::Softer {
                        3.0
                    } else {
                        1.75
                    };
                vis = (vis * 2.0
                    + geometry.sun_visibility(p - side, surface, light, ray, &[])
                    + geometry.sun_visibility(p + side, surface, light, ray, &[]))
                    * 0.25;
            }
            let facing = facing_gain(surface, ray);
            self.samples[i].light(light, facing, vis, ceiling);
            if fx_on && matches!(light.blend, Blend::Add) {
                let shadowed = (1.0 - vis) * light.shadow_color.opacity;
                fx::note_light(
                    &mut self.info[i],
                    &light,
                    ray,
                    light.height,
                    facing * (1.0 - shadowed) * light.energy * peak,
                );
            }
        }
    }

    fn vignette(&mut self, hp: f32, elapsed: f32) {
        let urgency = (1.0 - hp / 0.35) * (0.68 + 0.32 * (elapsed * 5.5).sin().abs());
        let rx = (self.w as f32 * 0.5).max(1.0);
        let ry = (self.h as f32 * 0.5).max(1.0);
        for (i, sample) in self.samples.iter_mut().enumerate() {
            let nx = (i % self.w) as f32 / rx - 1.0;
            let ny = (i / self.w) as f32 / ry - 1.0;
            let edge = ((nx * nx + ny * ny - 0.55) * 0.70).clamp(0.0, 1.0);
            let factor = urgency * edge;
            sample.add[0] += (factor * 38.0) as i16;
            sample.mul[1] = sample.mul[1].saturating_sub((factor * 32.0) as u16);
            sample.mul[2] = sample.mul[2].saturating_sub((factor * 32.0) as u16);
        }
    }

    /// Apply the light grid to the pixels.
    ///
    /// The hot loop of the lighting pass: every world pixel, every frame. The
    /// per-sample work (which of the shine, additive and tint terms can matter
    /// at all) is decided once per run of pixels under a sample, and the walk
    /// along a row steps the sample index instead of dividing for it. The
    /// arithmetic per pixel is unchanged — [`Sample::shade_exact`] is the
    /// reference form and the fast paths are that form with terms that are
    /// provably no-ops removed (pinned by `composite_fast_paths_match_the_reference`).
    fn composite(&self, pb: &mut PixelBuffer, depth: ColorDepth) {
        let (w, h) = (pb.w, pb.h);
        self.composite_px(pb.pixels_mut(), w, h, depth);
    }

    /// [`Grid::composite`] over a bare `w × h` pixel slice (the effects'
    /// `upsample::composite` stub calls this).
    pub(super) fn composite_px(&self, pixels: &mut [Rgb], w: usize, h: usize, depth: ColorDepth) {
        if w == 0 || h == 0 || self.stride == 0 {
            return;
        }
        let stride = self.stride;
        let low_colour = depth != ColorDepth::Truecolor;
        // The first run of a row ends at the next sample boundary; every later
        // run is a whole stride.
        let first = (stride - self.offset.0 % stride).min(w);
        let first_sample = self.offset.0 / stride;
        for (y, row) in pixels.chunks_exact_mut(w).take(h).enumerate() {
            let by = y + self.offset.1;
            let samples = &self.samples[(by / stride) * self.w + first_sample..];
            if low_colour {
                // Palette terminals dither after the exact shade.
                let (mut x, mut end) = (0, first);
                for sample in samples {
                    for (dx, pixel) in row[x..end].iter_mut().enumerate() {
                        let lit = sample.shade_exact(*pixel);
                        *pixel = palette_dither(lit, depth, x + dx + self.offset.0, by);
                    }
                    if end == w {
                        break;
                    }
                    x = end;
                    end = (end + stride).min(w);
                }
            } else if stride == 1 {
                for (pixel, sample) in row.iter_mut().zip(samples) {
                    sample.shade_run(std::slice::from_mut(pixel));
                }
            } else {
                let (mut x, mut end) = (0, first);
                for sample in samples {
                    sample.shade_run(&mut row[x..end]);
                    if end == w {
                        break;
                    }
                    x = end;
                    end = (end + stride).min(w);
                }
            }
        }
    }
}

/// The most the emission term can raise a multiplier to: a channel peak of
/// 255 is `(255 - 218) * 5`.
pub(super) const MAX_SHINE: u16 = (255 - SHINE_FLOOR as u16) * 5;
/// Channel peaks above this carry a little emission of their own.
pub(super) const SHINE_FLOOR: u8 = 218;

impl Sample {
    /// The reference shade of one pixel under this sample: multiply (never
    /// below the pixel's own emission), add toward white, then mix the tint.
    #[inline(always)]
    pub(super) fn shade_exact(&self, (r, g, b): Rgb) -> Rgb {
        // Transient highlights from older sprite art still carry a
        // little emission. Marked cores are restored explicitly.
        let peak = r.max(g).max(b);
        let shine = i32::from(peak.saturating_sub(SHINE_FLOOR)) * 5;
        let channel = |c: u8, n: usize| {
            let mul = i32::from(self.mul[n]).max(shine);
            let base = ((i32::from(c) * mul) >> 8).min(255);
            let lit = (base + ((i32::from(self.add[n]) * (255 - base)) >> 8)).clamp(0, 255);
            if self.tint[3] == 0 {
                lit as u8
            } else {
                (((lit * (256 - i32::from(self.tint[3]))) >> 8) + i32::from(self.tint[n])).min(255)
                    as u8
            }
        };
        (channel(r, 0), channel(g, 1), channel(b, 2))
    }

    /// Shade a run of truecolor pixels that all sit under this sample.
    ///
    /// Bit-identical to [`Sample::shade_exact`] per pixel; what is decided
    /// once for the run is which terms can be skipped:
    ///
    /// * no tint (`tint[3] == 0`) — the tint mix is skipped;
    /// * every multiplier at or above [`MAX_SHINE`] — the emission term can
    ///   never win the `max`, so the channel peak is not even computed;
    /// * no additive light (`add == 0`) — the add is `+ 0`, and the result of
    ///   the multiply is already in `0..=255`;
    /// * the identity sample (`mul == 256`, `add == 0`) — nothing to do.
    #[inline(always)]
    pub(super) fn shade_run(&self, run: &mut [Rgb]) {
        if self.tint[3] != 0 {
            for pixel in run {
                *pixel = self.shade_exact(*pixel);
            }
            return;
        }
        let mul = [
            i32::from(self.mul[0]),
            i32::from(self.mul[1]),
            i32::from(self.mul[2]),
        ];
        let add = [
            i32::from(self.add[0]),
            i32::from(self.add[1]),
            i32::from(self.add[2]),
        ];
        let no_add = self.add == [0, 0, 0];
        let no_shine = self.mul.iter().all(|m| *m >= MAX_SHINE);
        if no_add && self.mul == [256, 256, 256] {
            return;
        }
        // One channel with the multipliers already chosen.
        #[inline(always)]
        fn scale(c: u8, mul: i32) -> i32 {
            ((i32::from(c) * mul) >> 8).min(255)
        }
        #[inline(always)]
        fn lift(base: i32, add: i32) -> u8 {
            (base + ((add * (255 - base)) >> 8)).clamp(0, 255) as u8
        }
        match (no_shine, no_add) {
            (true, true) => {
                for (r, g, b) in run {
                    *r = scale(*r, mul[0]) as u8;
                    *g = scale(*g, mul[1]) as u8;
                    *b = scale(*b, mul[2]) as u8;
                }
            }
            (true, false) => {
                for (r, g, b) in run {
                    *r = lift(scale(*r, mul[0]), add[0]);
                    *g = lift(scale(*g, mul[1]), add[1]);
                    *b = lift(scale(*b, mul[2]), add[2]);
                }
            }
            (false, _) => {
                for (r, g, b) in run {
                    let peak = (*r).max(*g).max(*b);
                    let m = if peak > SHINE_FLOOR {
                        let shine = i32::from(peak - SHINE_FLOOR) * 5;
                        [mul[0].max(shine), mul[1].max(shine), mul[2].max(shine)]
                    } else {
                        mul
                    };
                    *r = lift(scale(*r, m[0]), add[0]);
                    *g = lift(scale(*g, m[1]), add[1]);
                    *b = lift(scale(*b, m[2]), add[2]);
                }
            }
        }
    }
}

impl Sample {
    /// `ceiling` is the `mul` clamp ([`MUL_CLAMP`] with the effects off).
    fn light(&mut self, light: Light, weight: f32, visibility: f32, ceiling: i32) {
        let shadow = (1.0 - visibility) * light.shadow_color.opacity;
        let strength = weight * (1.0 - shadow);
        let color = [light.color.0, light.color.1, light.color.2];
        let shade = [
            light.shadow_color.rgb.0,
            light.shadow_color.rgb.1,
            light.shadow_color.rgb.2,
        ];
        if matches!(light.blend, Blend::Mix) {
            let mix = (strength * light.energy).clamp(0.0, 1.0);
            for (n, tint) in self.tint.iter_mut().enumerate() {
                let target = color.get(n).copied().unwrap_or(255);
                *tint = (f32::from(*tint) * (1.0 - mix) + f32::from(target) * mix) as u8;
            }
            return;
        }
        if strength == 0.0 && shade == [0, 0, 0] {
            return;
        }
        let signed = if matches!(light.blend, Blend::Subtract) {
            -strength
        } else {
            strength
        };
        let gain = light.energy * signed;
        let scatter = light.scatter * signed;
        let tint = shadow * weight * 0.20;
        for (n, (color, shade)) in color.into_iter().zip(shade).enumerate() {
            let gain = (f32::from(color) * gain) as i32;
            let scatter = (f32::from(color) * scatter + f32::from(shade) * tint) as i32;
            self.mul[n] = (i32::from(self.mul[n]) + gain).clamp(0, ceiling) as u16;
            self.add[n] = (i32::from(self.add[n]) + scatter).clamp(-128, 142) as i16;
        }
    }
}

fn facing_gain(surface: Surface, toward: Vec2) -> f32 {
    if surface.height == 0.0 || surface.facing == Vec2::ZERO {
        return 1.0;
    }
    let denom = toward.x.abs().max(toward.y.abs()).max(0.01);
    let facing = (surface.facing.dot(toward) / denom).clamp(-1.0, 1.0);
    0.75 + 0.26 * facing
}

/// Ordered luminance dither before the terminal's palette mapper. Its phase
/// moves with world pixels, so the light pools do not crawl as the camera pans.
pub(super) fn palette_dither(color: Rgb, depth: ColorDepth, x: usize, y: usize) -> Rgb {
    const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    // With only 16 colours, group the dither into 2×2 world pixels and
    // restrain its range so the plank and wall art remains legible at 2×.
    let (x, y, span) = if depth == ColorDepth::Ansi16 {
        (x / 2, y / 2, 36)
    } else {
        (x, y, 28)
    };
    let phase = BAYER[y & 3][x & 3] * 2 - 15;
    let offset = phase * span / 32;
    let adjust = |c| (i16::from(c) + offset).clamp(0, 255) as u8;
    (adjust(color.0), adjust(color.1), adjust(color.2))
}

#[cfg(test)]
mod composite_tests {
    use super::*;

    /// A small deterministic generator (the tests must not depend on `rand`'s
    /// stream).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next() % n
        }
    }

    /// Samples that hit every fast path and every boundary of one.
    fn samples(rng: &mut Lcg, n: usize) -> Vec<Sample> {
        let muls: [u16; 12] = [0, 1, 60, 184, 185, 186, 255, 256, 257, 300, 512, 900];
        let adds: [i16; 9] = [0, 0, 0, 1, -1, 38, -120, 200, 256];
        (0..n)
            .map(|i| {
                let pick_mul = |rng: &mut Lcg| match rng.below(3) {
                    0 => muls[rng.below(12) as usize],
                    1 => 256,
                    _ => rng.below(700) as u16,
                };
                let mul = if i % 7 == 0 {
                    [256, 256, 256]
                } else {
                    [pick_mul(rng), pick_mul(rng), pick_mul(rng)]
                };
                let add = if rng.below(2) == 0 {
                    [0, 0, 0]
                } else {
                    [
                        adds[rng.below(9) as usize],
                        adds[rng.below(9) as usize],
                        adds[rng.below(9) as usize],
                    ]
                };
                let tint = if rng.below(4) == 0 {
                    [
                        rng.below(256) as u8,
                        rng.below(256) as u8,
                        rng.below(256) as u8,
                        rng.below(256) as u8,
                    ]
                } else {
                    [rng.below(256) as u8, 0, 0, 0]
                };
                Sample { mul, add, tint }
            })
            .collect()
    }

    fn pixel(rng: &mut Lcg) -> Rgb {
        let edge: [u8; 8] = [0, 1, 217, 218, 219, 254, 255, 128];
        let c = |rng: &mut Lcg| {
            if rng.below(3) == 0 {
                edge[rng.below(8) as usize]
            } else {
                rng.below(256) as u8
            }
        };
        (c(rng), c(rng), c(rng))
    }

    /// The composite exactly as it was before the fast paths: one closure,
    /// every term evaluated for every pixel.
    fn reference(grid: &Grid, pb: &mut PixelBuffer, depth: ColorDepth) {
        let (w, h) = (pb.w, pb.h);
        let pixels = pb.pixels_mut();
        let low_colour = depth != ColorDepth::Truecolor;
        for y in 0..h {
            let by = y + grid.offset.1;
            let base = (by / grid.stride) * grid.w;
            let mut x = 0;
            while x < w {
                let bx = x + grid.offset.0;
                let end = (x + grid.stride - bx % grid.stride).min(w);
                let sample = grid.samples[base + bx / grid.stride];
                for (dx, pixel) in pixels[y * w + x..y * w + end].iter_mut().enumerate() {
                    let (r, g, b) = *pixel;
                    let peak = r.max(g).max(b);
                    let shine = i32::from(peak.saturating_sub(218)) * 5;
                    let channel = |c: u8, n: usize| {
                        let mul = i32::from(sample.mul[n]).max(shine);
                        let base = ((i32::from(c) * mul) >> 8).min(255);
                        let lit =
                            (base + ((i32::from(sample.add[n]) * (255 - base)) >> 8)).clamp(0, 255);
                        if sample.tint[3] == 0 {
                            lit as u8
                        } else {
                            (((lit * (256 - i32::from(sample.tint[3]))) >> 8)
                                + i32::from(sample.tint[n]))
                            .min(255) as u8
                        }
                    };
                    let lit = (channel(r, 0), channel(g, 1), channel(b, 2));
                    *pixel = if low_colour {
                        palette_dither(lit, depth, bx + dx, by)
                    } else {
                        lit
                    };
                }
                x = end;
            }
        }
    }

    #[test]
    fn the_shine_ceiling_is_what_the_fast_path_assumes() {
        assert_eq!(MAX_SHINE, 185);
        assert_eq!(i32::from(255u8.saturating_sub(SHINE_FLOOR)) * 5, i32::from(MAX_SHINE));
    }

    #[test]
    fn shade_run_matches_the_reference_for_every_channel_value() {
        // Every sample class against every channel value (and every peak).
        let mut rng = Lcg(7);
        for sample in samples(&mut rng, 400) {
            for c in 0..=255u8 {
                for other in [0u8, 100, 218, 219, 255] {
                    for px in [(c, other, other), (other, c, other), (other, other, c), (c, c, c)] {
                        let mut run = [px];
                        sample.shade_run(&mut run);
                        assert_eq!(
                            run[0],
                            sample.shade_exact(px),
                            "pixel {px:?} under mul {:?} add {:?} tint {:?}",
                            sample.mul,
                            sample.add,
                            sample.tint
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn composite_fast_paths_match_the_reference() {
        let mut rng = Lcg(42);
        for stride in [1usize, 2, 3, 4, 8] {
            for (pw, ph) in [(1usize, 1usize), (7, 5), (64, 33), (121, 80)] {
                for offset in [(0usize, 0usize), (1, 0), (stride - 1, stride - 1), (5, 9)] {
                    let gw = (pw + offset.0) / stride + 2;
                    let gh = (ph + offset.1) / stride + 2;
                    let grid = Grid {
                        w: gw,
                        h: gh,
                        stride,
                        start: Vec2::ZERO,
                        offset,
                        samples: samples(&mut rng, gw * gh),
                        surfaces: Vec::new(),
                        info: Vec::new(),
                    };
                    for depth in [ColorDepth::Truecolor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
                        let mut fast = PixelBuffer::new(pw, ph);
                        for p in fast.pixels_mut() {
                            *p = pixel(&mut rng);
                        }
                        let mut slow = PixelBuffer::new(pw, ph);
                        slow.pixels_mut().copy_from_slice(fast.pixels());
                        grid.composite(&mut fast, depth);
                        reference(&grid, &mut slow, depth);
                        assert!(
                            fast.pixels() == slow.pixels(),
                            "stride {stride} size {pw}x{ph} offset {offset:?} depth {depth:?}"
                        );
                    }
                }
            }
        }
    }
}
