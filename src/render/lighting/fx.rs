//! Shared types of the batch-13 lighting effects (the contract is
//! `.review/handoff/lighting/CONTRACT.md`).
//!
//! The light grid keeps, beside each [`super::engine::Sample`], a
//! [`LightInfo`]: the dominant light's direction, colour and contribution, the
//! summed specular-eligible energy, and the air (scatter) and fog channels. The
//! pipeline stages (`upsample`, `air`, `surface`, `specular`, `softshadow`,
//! `motes`; `overhead` runs in the scene draw) read it through [`FxFrame`] and
//! write pixels through [`Canvas`]. Every stage runs only while the effects
//! switch is on ([`enabled_from_env`], the console's `fx on|off`); with it off
//! the lighting pass is byte-identical to the batch-12 one.
//!
//! Render-only: nothing here reads the simulation's rng or writes `World`.

#![allow(dead_code)] // Read by the stage files as they are filled in.

use super::engine::{Grid, LightingMode};
use super::shadow::Surface;
use super::{Light, LightShape, ShadowSoftness};
use crate::game::level::{Level, LightFixtureKind, Theme};
use crate::math::Vec2;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;

/// `LightInfo::dir` fixed point: 1.0 = 4096 (Q12).
pub(super) const DIR_ONE: i16 = 4096;
/// The strongest lights kept per frame in [`FxFrame::lights`].
pub(super) const MAX_FX_LIGHTS: usize = 16;

/// One tunable per technique. `1.0` is the concept plates' **bold** strength
/// (`relight_core.py`, `strength = 1.0`); the owner chose restrained (0.5)
/// everywhere and a quarter of bold for bloom/halation. Foundation-owned: a
/// worker reads these and keeps its own internal constants in its own file.
pub(super) mod strength {
    /// Lower ambient and the soft clip of bright cores (`upsample`).
    pub const TONE: f32 = 0.5;
    /// Coloured bounce light (`upsample::bounce`).
    pub const BOUNCE: f32 = 0.5;
    /// Volumetric shafts from fixtures (`air`).
    pub const SHAFTS: f32 = 1.0;
    /// Depth fog / distance fade (`air`).
    pub const FOG: f32 = 0.5;
    /// Bloom and halation (`air`): a quarter of bold.
    pub const BLOOM: f32 = 0.30;
    /// Bump-mapped floors and walls (`surface`).
    pub const BUMP: f32 = 1.0;
    /// Per-material specular (`specular`).
    pub const SPECULAR: f32 = 0.75;
    /// Directional rim light on creatures (`specular`).
    pub const RIM: f32 = 0.5;
    /// Soft penumbra (`softshadow`).
    pub const SOFT_SHADOW: f32 = 0.5;
    /// Long casts from low fixtures (`softshadow`).
    pub const LONG_SHADOW: f32 = 0.7;
    /// Wet-floor reflections (`softshadow::reflections`).
    pub const REFLECTION: f32 = 1.0;
    /// Particulate in beams (`motes`).
    pub const MOTES: f32 = 1.0;
    /// Caustics (`motes`).
    pub const CAUSTICS: f32 = 0.5;
    /// Standing water (`water`): glints, lamp mirrors and body mirrors. The
    /// master of every constant at the top of `water.rs`; 0 hands the water
    /// back to `specular` and `softshadow::reflections` exactly as before.
    pub const WATER: f32 = 1.0;
    /// The overhead light's ground shadow: darkening at the shadow's core
    /// (absolute, 0.30 = 30 %; the owner asked for "somewhat", 30–40 %).
    pub const OVERHEAD_DARKEN: f32 = 0.42;
}

/// Are the lighting effects on by default? `RATRUN_FX=0` (or
/// `off`/`false`/`no`) pins them off for a session; the console's read-only
/// `fx on|off` is the in-run A/B.
pub fn enabled_from_env() -> bool {
    match std::env::var("RATRUN_FX") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => true,
    }
}

/// Capture aid, test builds only: `RATRUN_FX_SKIP=overhead,body,spec,...`
/// leaves the named stages out of a dump, so one effect can be judged against
/// the same frame without it. Always `false` in the game.
#[cfg(test)]
pub(crate) fn skip(stage: &str) -> bool {
    std::env::var("RATRUN_FX_SKIP").is_ok_and(|v| v.split(',').any(|s| s.trim() == stage))
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn skip(_stage: &str) -> bool {
    false
}

/// Where a splatted light came from (shafts, cages and bloom key on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LightTag {
    /// A placed level fixture.
    Fixture(LightFixtureKind),
    /// The small bright wash at a sconce's mount (`fixtures::sconce_wash`).
    SconceWash,
    /// A per-frame light: the rat's aura (priority 3), halos, flashes, boss
    /// glows. `priority` is the shaped list's priority.
    Moving { priority: u8 },
    /// A directional light (the Flatbed sun).
    Sun,
    /// A stationary per-frame source that contributes to surface lighting.
    Ground,
}

/// Per-sample record beside [`super::engine::Sample`]. Reset every fx frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LightInfo {
    /// Unit vector from the surface toward the dominant light, Q12
    /// ([`DIR_ONE`]); `z` from the light's height. `[0, 0, 0]` = no light.
    pub dir: [i16; 3],
    /// The dominant light's colour.
    pub dom_rgb: Rgb,
    /// The dominant light's contribution, in `Q8.8` peak-channel units (the
    /// scale of a `mul` increment). Top-1, never a vector sum.
    pub dom: u16,
    /// Sum of every additive contribution (specular-eligible energy), `Q8.8`.
    pub spec: u16,
    /// Additive air scatter (shafts, bloom), `Q8.8` per channel. `air.rs`.
    pub air: [u16; 3],
    /// Fog amount 0..=255. `air.rs`.
    pub fog: u8,
    /// Peak contribution lost to cast shadows at this tap, in Q8.8
    /// multiplier units. The composite uses its gradient to keep shadow
    /// edges at pixel resolution.
    pub shadow: u8,
}

/// Fold one contribution into a sample's record: `toward` is the 2D vector
/// from the sample to the light (world px), `z` its height above the floor
/// (or the sun's rise per pixel), `contribution` in `Q8.8` peak-channel units.
#[inline]
pub(super) fn note_light(info: &mut LightInfo, light: &Light, toward: Vec2, z: f32, contribution: f32) {
    if contribution < 1.0 {
        return;
    }
    let c = contribution.min(65_535.0) as u16;
    info.spec = info.spec.saturating_add(c);
    if c > info.dom {
        let len = (toward.len_sq() + z * z).sqrt().max(1e-3);
        let k = f32::from(DIR_ONE) / len;
        info.dir = [(toward.x * k) as i16, (toward.y * k) as i16, (z * k) as i16];
        info.dom = c;
        info.dom_rgb = light.color;
    }
}

/// One of the frame's strongest lights, in world pixels.
#[derive(Clone, Copy, Debug)]
pub(super) struct FxLight {
    pub pos: Vec2,
    pub height: f32,
    pub color: Rgb,
    /// Peak contribution at weight 1 in `Q8.8` (as `Curve::gain`), flicker
    /// included.
    pub gain: f32,
    /// `Light::extent()` (infinite for the sun).
    pub extent: f32,
    pub tag: LightTag,
    /// `gain` without the flicker term (stable across frames).
    pub steady: f32,
}

impl FxLight {
    pub(super) fn of(light: &Light, tag: LightTag, flicker: f32) -> Self {
        let peak = f32::from(light.color.0.max(light.color.1).max(light.color.2));
        let steady = peak * (light.energy.abs() + light.scatter.abs());
        Self {
            pos: light.pos,
            height: light.height,
            color: light.color,
            gain: steady * flicker,
            extent: light.extent(),
            tag,
            steady,
        }
    }

    /// The ranking key of [`FxFrame::lights`].
    pub(super) fn importance(&self) -> f32 {
        let reach = if self.extent.is_finite() { self.extent } else { 4096.0 };
        self.gain * reach
    }
}

/// Keep the [`MAX_FX_LIGHTS`] most important lights (replacing the weakest).
pub(super) fn push_fx_light(list: &mut Vec<FxLight>, light: FxLight) {
    if list.len() < MAX_FX_LIGHTS {
        list.push(light);
        return;
    }
    let mut weakest = 0;
    for (i, l) in list.iter().enumerate() {
        if l.importance() < list[weakest].importance() {
            weakest = i;
        }
    }
    if list[weakest].importance() < light.importance() {
        list[weakest] = light;
    }
}

/// Sort descending by importance, ties by position (deterministic).
pub(super) fn sort_fx_lights(list: &mut [FxLight]) {
    list.sort_by(|a, b| {
        b.importance()
            .total_cmp(&a.importance())
            .then_with(|| a.pos.x.total_cmp(&b.pos.x))
            .then_with(|| a.pos.y.total_cmp(&b.pos.y))
    });
}

/// What `air::splat_sample` sees for one touched sample of one light.
pub(super) struct SampleHit<'a> {
    pub theme: Theme,
    pub light: &'a Light,
    pub tag: LightTag,
    /// Sample point minus light position, world px.
    pub delta: Vec2,
    /// The sample's world position.
    pub point: Vec2,
    /// The falloff parts (`Curve::parts`; the tail already quantised).
    pub core: f32,
    pub tail: f32,
    /// Shadow visibility 0..=1 (after `softshadow::visibility`).
    pub visibility: f32,
    pub surface: Surface,
    pub mode: LightingMode,
}

/// What `softshadow::visibility` sees.
pub(super) struct ShadowQuery<'a> {
    pub light: &'a Light,
    pub tag: LightTag,
    pub delta: Vec2,
    pub surface: Surface,
    /// The softness the old path would use (`None` below Full).
    pub softness: ShadowSoftness,
    pub mode: LightingMode,
}

/// A per-pixel tangent normal, Q0.7 (`[0, 0]` = flat, facing up).
pub(super) type Normal = [i8; 2];
pub(super) const NORMAL_ONE: i8 = 127;

/// The world pixels and their material bytes for one pixel stage.
pub(super) struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a mut [Rgb],
    /// `w * h` material bytes, or empty when materials are off (every pixel
    /// is then `MaterialKind::Default`).
    pub mat: &'a [u8],
}

impl Canvas<'_> {
    #[inline]
    pub fn mat_at(&self, i: usize) -> u8 {
        self.mat.get(i).copied().unwrap_or(0)
    }
}

/// Everything a stage may read about the frame.
pub(super) struct FxFrame<'a> {
    pub mode: LightingMode,
    pub depth: ColorDepth,
    pub theme: Theme,
    pub level: &'a Level,
    /// World position of pixel (0, 0), rounded the way the grid snaps it.
    pub cam: Vec2,
    #[cfg(test)]
    pub elapsed: f32,
    /// `floor(elapsed * 8)`: the only clock an animated term may read.
    pub step8: u32,
    /// The rat's world position.
    pub rat: Vec2,
    /// The frame's strongest lights, most important first.
    pub lights: &'a [FxLight],
    /// The theme's ambient tint (`ThemeAmbient::mul`, unscaled, `Q8.8`).
    pub ambient: [u16; 3],
}

/// `floor(elapsed * 8)`, the 8 Hz step every animated effect is held on.
#[inline]
pub(super) fn step8(elapsed: f32) -> u32 {
    (elapsed.max(0.0) * 8.0).floor() as u32
}

/// The flicker gain a light carries this frame (for [`FxLight::of`]).
#[inline]
pub(super) fn flicker_of(light: &Light, elapsed: f32) -> f32 {
    light.flicker.map_or(1.0, |f| f.gain(elapsed))
}

/// Is this a directional light (no position, infinite reach)?
#[inline]
pub(super) fn is_directional(light: &Light) -> bool {
    matches!(light.shape, LightShape::Directional { .. })
}

impl Grid {
    /// Index of the sample covering pixel `(px, py)` of the frame.
    #[inline]
    pub(super) fn sample_index(&self, px: usize, py: usize) -> usize {
        ((py + self.offset.1) / self.stride) * self.w + (px + self.offset.0) / self.stride
    }

    /// The top-left pixel of sample `(sx, sy)`'s block (may be negative for
    /// the first row/column). The sample point is the block's centre.
    #[inline]
    pub(super) fn sample_origin(&self, sx: usize, sy: usize) -> (i32, i32) {
        (
            (sx * self.stride) as i32 - self.offset.0 as i32,
            (sy * self.stride) as i32 - self.offset.1 as i32,
        )
    }
}

/// Test helpers for the stage files (see the contract's §6).
#[cfg(test)]
pub(super) mod testkit {
    use super::super::engine::Sample;
    use super::super::{LightMap, LightScene};
    use super::*;
    use crate::game::world::World;
    use crate::render::framebuffer::PixelBuffer;

    /// An empty level of `theme` with the rat at (700, 700), smoke off.
    pub fn fx_world(theme: Theme) -> World {
        let mut w = World::new(theme, 80);
        w.level = Level::empty(theme);
        w.player.pos = Vec2::new(700.0, 700.0);
        w.smoke_enabled = false;
        w
    }

    /// A lighting scene over `w` with the effects **on**.
    pub fn fx_scene(w: &World, mode: LightingMode, depth: ColorDepth, cam: Vec2) -> LightScene<'_> {
        LightScene {
            ecs: &w.ecs,
            level: &w.level,
            player: &w.player,
            particles: &w.particles,
            thrash: None,
            squeak: None,
            cam,
            elapsed: w.elapsed,
            blackout: w.blackout(),
            mode,
            depth,
            fx: true,
        }
    }

    /// The same input lit twice, `(fx off, fx on)`: a `w × h` buffer cleared
    /// to a mid floor colour, `paint` drawn over it (materials on for the fx
    /// run at Full/Hard, so `set_pen`/`stamp_mat` in `paint` tag pixels), the
    /// `lights` pushed at priority 3, then the lighting pass.
    pub fn lit(
        w: &World,
        lights: &[Light],
        mode: LightingMode,
        size: (usize, usize),
        cam: Vec2,
        paint: impl Fn(&mut PixelBuffer),
    ) -> (PixelBuffer, PixelBuffer) {
        let run = |fx: bool| {
            let mut pb = PixelBuffer::new(size.0, size.1);
            pb.set_materials(fx && matches!(mode, LightingMode::Full | LightingMode::Hard));
            pb.set_pen(0);
            pb.clear((85, 71, 78));
            paint(&mut pb);
            pb.set_pen(0);
            let mut map = LightMap::new();
            for &light in lights {
                map.push_shaped(light, 3);
            }
            let mut scene = fx_scene(w, mode, ColorDepth::Truecolor, cam);
            scene.fx = fx;
            map.apply_scene(&mut pb, &scene);
            pb
        };
        (run(false), run(true))
    }

    /// A hand-built grid: `w × h` samples of `stride`, every sample at `mul`
    /// with no add/tint, floor surfaces, zeroed `info`, offset (0, 0).
    pub fn flat_grid(w: usize, h: usize, stride: usize, mul: [u16; 3]) -> Grid {
        Grid {
            w,
            h,
            stride,
            start: Vec2::new(stride as f32 / 2.0, stride as f32 / 2.0),
            offset: (0, 0),
            samples: vec![
                Sample {
                    mul,
                    ..Sample::default()
                };
                w * h
            ],
            surfaces: vec![
                Surface {
                    layer: super::super::LayerMask::FLOOR.0,
                    ..Surface::default()
                };
                w * h
            ],
            info: vec![LightInfo::default(); w * h],
            hard_shadows: Default::default(),
        }
    }

    /// A frame over `level` with no lights, cam at the origin, elapsed 0.
    pub fn frame(level: &Level, mode: LightingMode) -> FxFrame<'_> {
        FxFrame {
            mode,
            depth: ColorDepth::Truecolor,
            theme: level.theme,
            level,
            cam: Vec2::ZERO,
            elapsed: 0.0,
            step8: 0,
            rat: Vec2::new(64.0, 48.0),
            lights: &[],
            ambient: super::super::theme_ambient(level.theme).mul,
        }
    }

    /// A canvas over `pb` (materials as `pb` has them).
    pub fn canvas(pb: &mut PixelBuffer) -> Canvas<'_> {
        let (w, h) = (pb.w, pb.h);
        let (px, mat) = pb.split_mat();
        Canvas { w, h, px, mat }
    }
}
