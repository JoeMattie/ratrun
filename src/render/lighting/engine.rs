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
//! * [`LightMap`] — owns its scratch buffers, the occluder set and the cached
//!   angular visibility profiles, so a steady frame allocates nothing.
//!
//! Render-only: nothing here reads or writes simulation state, and a change of
//! tier must never change a simulated number.

use super::shadow::{Body, Geometry, ShadowProfile, Surface};
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

#[derive(Clone, Copy, Debug)]
pub enum Falloff {
    Smooth,
    Linear,
    Hard,
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

/// Ambient gain of a light zone; `None` is unzoned floor.
fn zone_gain(zone: Option<LightZoneKind>) -> f32 {
    match zone {
        Some(LightZoneKind::Start) => 0.95,
        Some(LightZoneKind::Room) => 0.80,
        Some(LightZoneKind::Corridor) => 0.56,
        Some(LightZoneKind::Arena) => 0.64,
        Some(LightZoneKind::Blackout) => 0.26,
        Some(LightZoneKind::Maze) => 0.25,
        None => 0.70,
    }
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
}

#[derive(Clone, Copy, Default)]
struct Sample {
    mul: [u16; 3],
    add: [i16; 3],
    tint: [u8; 4],
}

struct Grid {
    w: usize,
    h: usize,
    stride: usize,
    start: Vec2,
    offset: (usize, usize),
    samples: Vec<Sample>,
    surfaces: Vec<Surface>,
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
            grid: Grid::default(),
            geometry: Geometry::default(),
            static_shadows: Vec::new(),
            moving_shadows: [ShadowProfile::default(); 3],
            bodies: Vec::with_capacity(MAX_BODIES),
            controller: QualityController::default(),
            plume: Vec::new(),
            ambient_cap: None,
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
        let budget = 160.0 + pixels as f64 * 0.035;
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
        {
            let _p = crate::prof::scope(crate::prof::Stage::LSync);
            self.geometry.sync(scene.level);
        }
        let theme = scene.level.theme;
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
                .prepare(pb, scene.cam, scene.mode, &self.geometry, profile, cap);
        }
        crate::prof::count(crate::prof::Stage::CSamples, self.grid.samples.len() as u64);
        self.lights.points.clear();
        self.lights.weakest = 0;
        let phase = {
            let _p = crate::prof::scope(crate::prof::Stage::LGather);
            self.gather_lights(pb.w, pb.h, scene)
        };
        crate::prof::count(crate::prof::Stage::CPoints, self.lights.points.len() as u64);
        let _pstatic = crate::prof::scope(crate::prof::Stage::LStaticLights);
        self.static_shadows
            .resize(scene.level.objects.lights.len(), ShadowProfile::default());
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
            let shadow = &mut self.static_shadows[i];
            {
                let _p = crate::prof::scope(crate::prof::Stage::LStaticBake);
                if shadow.bake(&self.geometry, light) {
                    crate::prof::count(crate::prof::Stage::CStaticBakeMiss, 1);
                }
            }
            let _p = crate::prof::scope(crate::prof::Stage::LStaticSplat);
            self.grid
                .splat(light, Some(shadow), scene.mode, scene.elapsed, theme);
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
            } else {
                Falloff::Smooth
            };
            push_shaped(&mut self.shaped, light, p.priority);
        }
        // Prefer the rat, then active arena lights and foreground flashes.
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
        for &(mut light, _) in self.shaped.iter().take(cap) {
            if !self.grid.overlaps(light) {
                continue;
            }
            crate::prof::count(crate::prof::Stage::CShapedLit, 1);
            if matches!(light.shape, LightShape::Directional { .. }) {
                self.grid.sun(light, &self.geometry, &[], scene.mode);
                continue;
            }
            let shadow = if light.casts_shadow && shadow_index < shadows {
                let profile = &mut self.moving_shadows[shadow_index];
                // Stabilise the visibility origin at pixel scale as the rat moves.
                light.pos = Vec2::new(light.pos.x.round(), light.pos.y.round());
                {
                    let _p = crate::prof::scope(crate::prof::Stage::LMovingBake);
                    if profile.bake(&self.geometry, light) {
                        crate::prof::count(crate::prof::Stage::CMovingBakeMiss, 1);
                    }
                }
                shadow_index += 1;
                Some(&*profile)
            } else {
                None
            };
            let _p = crate::prof::scope(crate::prof::Stage::LMovingSplat);
            self.grid
                .splat(light, shadow, scene.mode, scene.elapsed, theme);
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
            self.grid.composite(pb, scene.depth);
        }
        let _pm = crate::prof::scope(crate::prof::Stage::LMarks);
        for mark in &self.marks {
            let pixel = &mut pb.pixels_mut()[mark.index];
            if mark.readable {
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
        self.grid
            .sun(light, &self.geometry, &self.bodies, scene.mode);
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
        mode: LightingMode,
        geometry: &Geometry,
        profile: ThemeAmbient,
        cap: f32,
    ) {
        self.stride = mode.stride();
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
        for i in 0..self.samples.len() {
            let p = self.point(i % self.w, i / self.w);
            let surface = geometry.surface(p);
            let gain = zone_gain(surface.zone).min(cap);
            self.samples[i] = Sample {
                mul: profile.mul.map(|c| (c as f32 * gain) as u16),
                ..Sample::default()
            };
            self.surfaces[i] = surface;
        }
    }

    fn point(&self, x: usize, y: usize) -> Vec2 {
        self.start + Vec2::new((x * self.stride) as f32, (y * self.stride) as f32)
    }

    fn overlaps(&self, light: Light) -> bool {
        let r = light.shape.extent();
        let end = self.point(self.w, self.h);
        light.pos.x + r >= self.start.x
            && light.pos.x - r <= end.x
            && light.pos.y + r >= self.start.y
            && light.pos.y - r <= end.y
    }

    fn splat(
        &mut self,
        light: Light,
        shadow: Option<&ShadowProfile>,
        mode: LightingMode,
        elapsed: f32,
        theme: Theme,
    ) {
        let radius = light.shape.extent();
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
        let mut touched = 0u64;
        for y in lo(centre.y)..hi(centre.y, self.h) {
            for x in lo(centre.x)..hi(centre.x, self.w) {
                let delta = self.point(x, y) - light.pos;
                let w = light.shape.weight(delta, cone_limit);
                if w <= 0.0 {
                    continue;
                }
                let i = y * self.w + x;
                let surface = self.surfaces[i];
                if surface.layer & light.layers.0 == 0 {
                    continue;
                }
                let visibility = shadow.map_or(1.0, |s| s.visibility(delta, surface, softness));
                let face = if surface.height > 0.0 {
                    facing_gain(surface, -delta)
                } else {
                    1.0
                };
                let fall = match light.falloff {
                    Falloff::Smooth => w * w,
                    Falloff::Linear => w,
                    Falloff::Hard => (w * 1.6).min(1.0),
                };
                let glint = if shadow.is_some()
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
                touched += 1;
                self.samples[i].light(light, (fall * face + glint) * flicker, visibility);
            }
        }
        crate::prof::count(crate::prof::Stage::CSplatSamples, touched);
    }

    fn sun(&mut self, light: Light, geometry: &Geometry, bodies: &[Body], mode: LightingMode) {
        let LightShape::Directional { dir } = light.shape else {
            return;
        };
        let ray = -dir.normalized();
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
            self.samples[i].light(light, facing_gain(surface, ray), vis);
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

    fn composite(&self, pb: &mut PixelBuffer, depth: ColorDepth) {
        let (w, h) = (pb.w, pb.h);
        let pixels = pb.pixels_mut();
        let low_colour = depth != ColorDepth::Truecolor;
        for y in 0..h {
            let by = y + self.offset.1;
            let base = (by / self.stride) * self.w;
            let mut x = 0;
            while x < w {
                let bx = x + self.offset.0;
                let end = (x + self.stride - bx % self.stride).min(w);
                let sample = self.samples[base + bx / self.stride];
                for (dx, pixel) in pixels[y * w + x..y * w + end].iter_mut().enumerate() {
                    let (r, g, b) = *pixel;
                    // Transient highlights from older sprite art still carry a
                    // little emission. Marked cores are restored explicitly.
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
}

impl Sample {
    fn light(&mut self, light: Light, weight: f32, visibility: f32) {
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
            self.mul[n] = (i32::from(self.mul[n]) + gain).clamp(0, 350) as u16;
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
fn palette_dither(color: Rgb, depth: ColorDepth, x: usize, y: usize) -> Rgb {
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
