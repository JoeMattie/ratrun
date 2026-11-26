//! W2 — the air channel: volumetric light shafts, depth fog and bloom /
//! halation (contract §4, techniques 01, 02, 03). Writes `LightInfo::air` and
//! `LightInfo::fog`, then composites them over the lit surface.
//!
//! Three terms, one record, one pixel stage:
//!
//! * **Shafts** ([`splat_sample`], inside the engine's splat loop). A lamp with
//!   a real throw (`Sconce`, `Pendant`, `Passage`, `Arena`) adds
//!   `core * visibility` of its own colour into the sample's air. The weight is
//!   the light's *shape* only — never its energy, which a sconce flickers on a
//!   3 Hz held step — so the beam is cut by the cached visibility profile (walls
//!   and props) and never breathes. The Sewer's caged bulkhead adds the wire
//!   cage's angular bars on the `Full` tier.
//! * **Bloom** ([`grid_pass`]). A deliberately faint dual-Gaussian halo (one
//!   tight, one wide lobe) around each visible fixture, at
//!   `strength::BLOOM` (a quarter of the bold plates: the owner found the
//!   concept's glow over the top). The amplitude is keyed on the light's colour
//!   and tag, not on its flickered gain: a breathing lamp's core flickers, its
//!   halo does not.
//! * **Fog** ([`grid_pass`]). A per-sample amount growing with the distance from
//!   the rat, suppressed where the sample is brightly lit, lerped toward a
//!   colour derived from the theme's tint ([`composite`]).
//!
//! [`composite`] screen-blends the interpolated air over the picture after the
//! surface stages and then lerps toward the fog colour. Air is bilinearly
//! interpolated (and never across a wall face) so the 4×4 sample grid has no
//! block edges; below truecolor both terms are snapped to flat steps, because
//! this stage runs after the palette dither.
//!
//! Render-only: nothing here reads the clock for air (an animated term would
//! repaint a still screen), the rng, or `World`.

use super::engine::{Grid, LightingMode, MIN_GAIN};
use super::fx::{strength, Canvas, FxFrame, FxLight, LightInfo, LightTag, SampleHit};
use super::{LayerMask, Light};
use crate::game::level::{LightFixtureKind, Theme};
use crate::math::Vec2;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;

/// Additive levels of the light's colour a shaft reaches at
/// `core * visibility = 1` and the *bold* (1.0) strength. `strength::SHAFTS`
/// halves it, so a room lamp hazes its own beam at about 9 levels at the lens
/// and fades with the cone — a beam read in the air, not a white wash.
const SHAFT_LEVELS: f32 = 44.0;
/// A light shorter than this does not throw a visible shaft (a glowing pebble
/// is bloom's job, not a god ray).
const SHAFT_MIN_REACH: f32 = 24.0;
/// Additive levels at the centre of a hot fixture's halo at the *bold* (1.0)
/// strength. `strength::BLOOM` is a quarter, so the default peak is 8 levels:
/// the owner asked for less glow than the concept plates.
const BLOOM_LEVELS: f32 = 32.0;
/// A fixture dimmer than this (Q8.8 peak units, flicker included) does not
/// bloom. Live lamps sit at 190–690, a spent sconce at ~7 and a dying bulb's
/// dark half at ~5, so the gate never straddles a breathing lamp's range.
const BLOOM_MIN_GAIN: f32 = 160.0;
/// A halo's radius as a share of the light's own reach, clamped: a sconce's
/// 110 px fan blooms over ~33 px, a small pendant over its minimum.
const BLOOM_REACH_SHARE: f32 = 0.30;
const BLOOM_MIN_REACH: f32 = 12.0;
const BLOOM_MAX_REACH: f32 = 44.0;
/// The tight lobe's and the wide lobe's Gaussian sigma, as shares of the halo
/// radius.
const BLOOM_TIGHT_SIGMA: f32 = 0.22;
const BLOOM_WIDE_SIGMA: f32 = 0.45;
/// Share of the halo's amplitude in the tight lobe (the rest is the wide one).
const BLOOM_TIGHT_SHARE: f32 = 0.62;
/// Where both lobes have decayed below a third of a level: the halo is cut
/// (2.8 sigma of the wide lobe).
const BLOOM_CUT_SIGMAS: f32 = 2.8;

/// The fog's reach as a share of the view's long side (fog ramps out across the
/// screen, not across the map: at a 120×80 view the far corner is at full fog).
const FOG_REACH_SHARE: f32 = 0.55;
/// No fog at all within this many world px of the rat.
const FOG_NEAR: f32 = 44.0;
/// The share of a sample's colour the fog lerps toward at full amount, before
/// `strength::FOG`.
const FOG_MAX: f32 = 0.72;
/// The `mul` peak at which a sample is "brightly lit" and the fog fully
/// suppressed (pools stay clear; dark corners haze).
const FOG_BIAS_PEAK: f32 = 320.0;
/// How close to `FOG_BIAS_PEAK` a bright sample pushes the fog (62 %).
const FOG_SUPPRESS: f32 = 0.62;
/// The fog colour as a share of the theme's ambient tint (`frame.ambient`).
const FOG_TINT: f32 = 0.58;
/// The fog amount is painted in flat steps of this many levels (out of 255).
const FOG_STEP: u8 = 8;

/// The smallest additive unit written to `air`, Q8.8 (`MIN_GAIN` rounded up to
/// one more than a whole level's share): nothing fainter is painted, so a
/// beam's far tail cannot creep over a cell edge as the view pans.
const AIR_STEP: u16 = 8;
/// The most any channel of `air` holds, Q8.8 (40 levels): shafts plus bloom
/// haze a bright room, they never wash it white.
const AIR_MAX: u16 = 64 * 256;
/// Below truecolor the interpolated air is snapped to this many levels and the
/// fog to a multiple of this many parts in 255, so a faint gradient stays a few
/// flat cells instead of repainting a band every frame.
const AIR_STEP_LOW: i32 = 4;
const FOG_STEP_LOW: i32 = 16;
/// The same idea at truecolor, much finer: output steps of the interpolated
/// air (levels) and fog (of 255), so a shallow wash keeps the encoder's runs.
const AIR_STEP_OUT: i32 = 2;
const FOG_STEP_OUT: i32 = 8;

// The one theme whose drawn sconce wears a wire cage is the Sewer
// (`fixtures::housing`: the caged amber bulkhead); `caged_sconce` keys on it.
/// The cage's bars run round the lamp every 30° (12 bars a turn, so about 5
/// cross a 150° fan).
const CAGE_BARS: f32 = 6.0;
/// How much of the beam a bar takes out.
const CAGE_DIM: f32 = 0.55;
/// `|sin(angle · CAGE_BARS)|` below this is inside a bar.
const CAGE_BAR_WIDTH: f32 = 0.30;

/// `exp(-x / 2)` for `x = d² / sigma²` sampled every eighth (`x = i / 8`), in
/// Q0.10. [`gauss`] indexes it by `d² · 8 / sigma²`, so a two-lobe halo costs
/// two lookups and no `powf`/`exp`.
const GAUSS: [u16; 64] = [
    1024, 962, 904, 849, 797, 749, 704, 661, 621, 583, 548, 515, 484, 454, 427, 401, 377, 354, 332,
    312, 293, 276, 259, 243, 228, 215, 202, 189, 178, 167, 157, 148, 139, 130, 122, 115, 108, 101,
    95, 89, 84, 79, 74, 70, 65, 61, 58, 54, 51, 48, 45, 42, 40, 37, 35, 33, 31, 29, 27, 26, 24, 23,
    21, 20,
];

/// `GAUSS` at squared distance `d2` for `k = 8 / sigma²` (Q0.10, 0 past 8σ).
#[inline]
fn gauss(d2: f32, k: f32) -> i32 {
    let i = (d2 * k) as usize;
    if i < GAUSS.len() {
        i32::from(GAUSS[i])
    } else {
        0
    }
}

/// Once per splat: does this light put anything into the air? (Shafts from
/// sconces, pendants, passage lamps and arena lamps; never on Coarse/Compact,
/// never from a dying bulb — its shaft would sputter with it.)
pub(super) fn wants(light: &Light, tag: LightTag, mode: LightingMode) -> bool {
    if !matches!(mode, LightingMode::Full | LightingMode::Hard) {
        return false;
    }
    // The sun is directional (`extent` infinite) and splatted by `Grid::sun`,
    // which never offers its samples to this stage: nothing to do.
    if !light.extent().is_finite() || light.extent() < SHAFT_MIN_REACH {
        return false;
    }
    // A spent wall lamp's sputter (energy 0.42; a live one is >= 0.60 even
    // zone-capped) must not flash a beam.
    if tag == LightTag::Fixture(LightFixtureKind::Sconce) && light.energy < 0.5 {
        return false;
    }
    matches!(
        tag,
        LightTag::Fixture(
            LightFixtureKind::Sconce
                | LightFixtureKind::Pendant
                | LightFixtureKind::Passage
                | LightFixtureKind::Arena
        )
    )
}

/// Per touched sample of a light that [`wants`] air: add its shaft scatter
/// into `info.air` (`fx::strength::SHAFTS`). Never flickered: `core` is the
/// light's shape weight, not its (possibly flickering) energy.
pub(super) fn splat_sample(info: &mut LightInfo, hit: &SampleHit<'_>) {
    if hit.core <= 0.0 || hit.visibility <= 0.0 {
        return;
    }
    // `core²`: the scatter gathers toward the lens, so the beam reads as a
    // volume leaving the lamp and the outer pool keeps its floor contrast (a
    // haze as wide as the pool only looked like a brighter pool).
    let mut weight = hit.core * hit.core * hit.visibility * strength::SHAFTS * SHAFT_LEVELS;
    if hit.mode == LightingMode::Full {
        if caged_sconce(hit) {
            weight *= cage_bars(hit.delta);
        }
        weight *= rays(hit);
    }
    if weight <= 0.0 {
        return;
    }
    // Q8.8: one added level per 1/256th, the colour's share per channel.
    let per_level = weight * (256.0 / 255.0);
    let color = [hit.light.color.0, hit.light.color.1, hit.light.color.2];
    for (n, &c) in color.iter().enumerate() {
        let add = (per_level * f32::from(c)) as u16;
        let add = add / AIR_STEP * AIR_STEP;
        if add < MIN_GAIN as u16 {
            continue;
        }
        info.air[n] = info.air[n].saturating_add(add).min(AIR_MAX);
    }
}

/// Unoccluded shaft gain per RGB unit on Hard, which has no angular ray bands.
pub(super) fn hard_shaft_weight(core: f32) -> f32 {
    core * core * strength::SHAFTS * SHAFT_LEVELS * (256.0 / 255.0)
}

/// How deep the rays cut a wall lamp's fan and a hanging lamp's disc.
const RAY_DEPTH_SCONCE: f32 = 0.60;
const RAY_DEPTH_HANGING: f32 = 0.35;
/// Inside this many px of the lens the beam is whole (no pinwheel at the lamp).
const RAY_NEAR: f32 = 7.0;

/// Irregular rays: the beam's scatter banded by angle round the lamp, two
/// incommensurate harmonics with a phase from the lamp's own position, so
/// every lamp has its own streaks and none of them ever moves (no clock).
fn rays(hit: &SampleHit<'_>) -> f32 {
    let d2 = hit.delta.len_sq();
    if d2 < RAY_NEAR * RAY_NEAR {
        return 1.0;
    }
    let depth = match hit.tag {
        LightTag::Fixture(LightFixtureKind::Sconce) => RAY_DEPTH_SCONCE,
        _ => RAY_DEPTH_HANGING,
    };
    let seed = hit.light.pos.x * 0.37 + hit.light.pos.y * 0.61;
    let a = hit.delta.y.atan2(hit.delta.x);
    let v = 0.5 + 0.28 * (a * 7.0 + seed).sin() + 0.22 * (a * 13.0 - seed * 1.7).sin();
    // Ease in over the first few px past the lens.
    let ease = ((d2.sqrt() - RAY_NEAR) / 8.0).min(1.0);
    1.0 - depth * ease * (1.0 - v.clamp(0.0, 1.0))
}

/// Is this the caged bulkhead (the Sewer's sconce)?
fn caged_sconce(hit: &SampleHit<'_>) -> bool {
    hit.theme == Theme::Sewer && hit.tag == LightTag::Fixture(LightFixtureKind::Sconce)
}

/// The wire cage's bars: dark spokes across the fan, keyed on the sample's
/// angle round the lamp. Still (no clock): the cage does not move.
fn cage_bars(delta: Vec2) -> f32 {
    if delta.len_sq() < 1.0 {
        return 1.0;
    }
    let s = (delta.y.atan2(delta.x) * CAGE_BARS).sin();
    if s.abs() < CAGE_BAR_WIDTH {
        1.0 - CAGE_DIM
    } else {
        1.0
    }
}

/// Grid stage after every splat: analytic bloom per `frame.lights` entry
/// (`fx::strength::BLOOM`) into `info.air`, and the fog amount into
/// `info.fog` (`fx::strength::FOG`).
pub(super) fn grid_pass(grid: &mut Grid, frame: &FxFrame<'_>) {
    if grid.w == 0 || grid.h == 0 || grid.info.len() < grid.w * grid.h {
        return;
    }
    if !matches!(frame.mode, LightingMode::Full | LightingMode::Hard) {
        return;
    }
    bloom(grid, frame);
    fog(grid, frame);
}

/// One light's halo, with the per-frame trigonometry hoisted out.
struct Bloom {
    centre: Vec2,
    /// `8 / sigma²` of the tight lobe, the [`gauss`] index scale.
    k_tight: f32,
    /// `8 / sigma²` of the wide lobe.
    k_wide: f32,
    /// Where the halo is cut (world px, and its square).
    cut: f32,
    cut2: f32,
    /// Centre amplitude per channel, Q8.8, split between the two lobes.
    tight: [i32; 3],
    wide: [i32; 3],
}

impl Bloom {
    fn of(light: &FxLight) -> Option<Self> {
        if !blooms(light.tag) || light.steady < BLOOM_MIN_GAIN || !light.extent.is_finite() {
            return None;
        }
        let reach = (light.extent * BLOOM_REACH_SHARE).clamp(BLOOM_MIN_REACH, BLOOM_MAX_REACH);
        let sigma_tight = (reach * BLOOM_TIGHT_SIGMA).max(1.0);
        let sigma_wide = (reach * BLOOM_WIDE_SIGMA).max(sigma_tight + 1.0);
        let cut = sigma_wide * BLOOM_CUT_SIGMAS;
        let peak = light.color.0.max(light.color.1).max(light.color.2);
        let channels = [light.color.0, light.color.1, light.color.2];
        let mut tight = [0i32; 3];
        let mut wide = [0i32; 3];
        for (n, &c) in channels.iter().enumerate() {
            let amp = bloom_amplitude(peak, c, strength::BLOOM);
            tight[n] = (amp as f32 * BLOOM_TIGHT_SHARE) as i32;
            wide[n] = (amp as f32 * (1.0 - BLOOM_TIGHT_SHARE)) as i32;
        }
        Some(Self {
            centre: light.pos,
            k_tight: 8.0 / (sigma_tight * sigma_tight),
            k_wide: 8.0 / (sigma_wide * sigma_wide),
            cut,
            cut2: cut * cut,
            tight,
            wide,
        })
    }
}

/// Which tags carry a halo? Placed fixtures (never the directional sun, never
/// the dying bulb whose flash would make the halo sputter) and the sconce's
/// own wall wash. Per-frame emitters (halos, the rat's aura) are left to their
/// own light pools: the owner asked for less glow, not for a glow round
/// everything.
fn blooms(tag: LightTag) -> bool {
    matches!(
        tag,
        LightTag::Fixture(
            LightFixtureKind::Sconce
                | LightFixtureKind::Pendant
                | LightFixtureKind::Passage
                | LightFixtureKind::Arena
        ) | LightTag::SconceWash
    )
}

/// The additive amplitude (Q8.8) of a halo at its centre for a light of
/// `peak`-channel `peak` whose channel is `channel`, at `strength`
/// (`strength::BLOOM` in the game; 0 draws nothing).
fn bloom_amplitude(peak: u8, channel: u8, strength: f32) -> i32 {
    let levels = BLOOM_LEVELS * strength * (f32::from(peak) / 255.0) * (f32::from(channel) / 255.0);
    (levels * 256.0) as i32
}

fn bloom(grid: &mut Grid, frame: &FxFrame<'_>) {
    let stride = grid.stride.max(1) as f32;
    let start = grid.start;
    let (gw, gh) = (grid.w as i32, grid.h as i32);
    for light in frame.lights {
        let Some(b) = Bloom::of(light) else { continue };
        // The sample block the halo can touch, in sample indices.
        let lo_x = (((b.centre.x - b.cut - start.x) / stride).floor().max(0.0) as i32).max(0);
        let hi_x = (((b.centre.x + b.cut - start.x) / stride).ceil() as i32 + 1).clamp(0, gw);
        let lo_y = (((b.centre.y - b.cut - start.y) / stride).floor().max(0.0) as i32).max(0);
        let hi_y = (((b.centre.y + b.cut - start.y) / stride).ceil() as i32 + 1).clamp(0, gh);
        for sy in lo_y..hi_y {
            for sx in lo_x..hi_x {
                let p = start + Vec2::new(sx as f32 * stride, sy as f32 * stride);
                let d2 = p.dist_sq(b.centre);
                if d2 > b.cut2 {
                    continue;
                }
                let g_t = gauss(d2, b.k_tight);
                let g_w = gauss(d2, b.k_wide);
                if g_t == 0 && g_w == 0 {
                    continue;
                }
                let i = (sy * gw + sx) as usize;
                // A halo only where the lamp itself reached (its own colour
                // dominates): a sample behind a wall never took this light.
                if grid.info[i].dom == 0 || grid.info[i].dom_rgb != light.color {
                    continue;
                }
                let info = &mut grid.info[i];
                for (n, (t, w)) in b.tight.iter().zip(&b.wide).enumerate() {
                    let add = (t * g_t + w * g_w) >> 10;
                    if add < MIN_GAIN as i32 {
                        continue;
                    }
                    info.air[n] = info.air[n]
                        .saturating_add(add.min(i32::from(u16::MAX)) as u16)
                        .min(AIR_MAX);
                }
            }
        }
    }
}

/// Underwater depth-haze parameters for The Outfall: closer onset, deeper
/// marine blue-cyan colour and slightly denser distance fade than dry floors.
const FOG_NEAR_OUTFALL: f32 = 30.0;
const FOG_MAX_OUTFALL: f32 = 0.84;
const FOG_COLOR_OUTFALL: Rgb = (6, 36, 62);

fn fog(grid: &mut Grid, frame: &FxFrame<'_>) {
    let stride = grid.stride.max(1) as f32;
    let start = grid.start;
    let reach = ((grid.w.max(grid.h)) as f32 * stride * FOG_REACH_SHARE).max(1.0);
    let rat = frame.rat;
    let (fog_near, fog_max) = if frame.theme == Theme::Outfall {
        (FOG_NEAR_OUTFALL, FOG_MAX_OUTFALL)
    } else {
        (FOG_NEAR, FOG_MAX)
    };
    for sy in 0..grid.h {
        for sx in 0..grid.w {
            let p = start + Vec2::new(sx as f32 * stride, sy as f32 * stride);
            let d = (p - rat).len();
            let ramp = ((d - fog_near) / reach).clamp(0.0, 1.0);
            if ramp <= 0.0 {
                continue;
            }
            let mul = grid.samples[sy * grid.w + sx].mul;
            let peak = f32::from(mul[0].max(mul[1]).max(mul[2]));
            let bias = 1.0 - FOG_SUPPRESS * (peak / FOG_BIAS_PEAK).min(1.0);
            let amount = ramp * bias * fog_max * strength::FOG;
            if amount <= 0.0 {
                continue;
            }
            let levels = ((amount * 255.0) as u32 / u32::from(FOG_STEP)) * u32::from(FOG_STEP);
            grid.info[sy * grid.w + sx].fog = levels.min(255) as u8;
        }
    }
}

/// The fog colour: the theme's own tint at [`FOG_TINT`], so the distance fades
/// into the room's colour rather than a generic grey (deep ocean blue-cyan on
/// The Outfall).
fn fog_color(frame: &FxFrame<'_>) -> Rgb {
    if frame.theme == Theme::Outfall {
        return FOG_COLOR_OUTFALL;
    }
    let channel = |v: u16| (f32::from(v) * FOG_TINT) as u8;
    (
        channel(frame.ambient[0]),
        channel(frame.ambient[1]),
        channel(frame.ambient[2]),
    )
}

/// One channel of the pixel stage: the air screen blend, then the fog lerp.
#[inline]
fn blend_channel(c: i32, air: i32, fog: i32, fog_col: i32) -> u8 {
    let mut v = c;
    if air > 0 {
        // Screen: 255 - (255 - v)(255 - air) / 255.
        v += air - (v * air) / 255;
    }
    if fog > 0 {
        v += (fog_col - v) * fog / 255;
    }
    v.clamp(0, 255) as u8
}

/// Pixel stage after the surface stages: screen-blend `air`, then lerp toward
/// the theme's fog colour by `fog`. The air is bilinearly interpolated over the
/// four neighbouring samples (a tap across a wall face carries no air), so the
/// 4×4 grid leaves no block edges; with `air` and `fog` both zero the picture
/// is left bit-identical.
pub(super) fn composite(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {
    let (w, h) = (canvas.w, canvas.h);
    // Coarse and below: shafts, bloom and fog are off, so the grid carries no
    // air and the picture is untouched (the contract's tier table).
    if !matches!(frame.mode, LightingMode::Full | LightingMode::Hard) {
        return;
    }
    if w == 0 || h == 0 || grid.w == 0 || grid.h == 0 || grid.stride == 0 {
        return;
    }
    if grid.info.len() < grid.w * grid.h || grid.surfaces.len() < grid.w * grid.h {
        return;
    }
    if canvas.px.len() < w * h {
        return;
    }
    if grid.hard_shadows.shafts_active {
        composite_pixels::<true>(grid, canvas, frame);
    } else {
        composite_pixels::<false>(grid, canvas, frame);
    }
}

fn composite_pixels<const SHAFT_EDGES: bool>(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {
    let (w, h) = (canvas.w, canvas.h);
    let stride = grid.stride as i32;
    let step = 65_536 / stride;
    let inv = 65_536.0 / stride as f32;
    let sx0 = ((frame.cam.x - grid.start.x) * inv) as i32;
    let fog_col = fog_color(frame);
    let low = frame.depth != ColorDepth::Truecolor;
    let walls = LayerMask::WALLS.0;
    let (gw, gh) = (grid.w as i32, grid.h as i32);
    for y in 0..h {
        let sy = ((frame.cam.y + y as f32 - grid.start.y) * inv) as i32;
        let ty = (sy & 0xFFFF) >> 8;
        let y0 = (sy >> 16).clamp(0, gh - 1) as usize;
        let y1 = ((sy >> 16) + 1).clamp(0, gh - 1) as usize;
        for x in 0..w {
            let s = sx0 + x as i32 * step;
            let tx = (s & 0xFFFF) >> 8;
            let x0 = (s >> 16).clamp(0, gw - 1) as usize;
            let x1 = ((s >> 16) + 1).clamp(0, gw - 1) as usize;
            let correction = if SHAFT_EDGES {
                grid.hard_shadows.shaft(s >> 16, sy >> 16, x, y)
            } else {
                None
            };
            // Two cells share a horizontal weight, as the shade does.
            let tx = if super::upsample::PAIR_X && correction.is_none() {
                tx & !(2 * (256 / stride) - 1)
            } else {
                tx
            };
            // Bilinear weights, Q16; they sum to 65536 before the wall mask.
            let w00 = (256 - tx) * (256 - ty);
            let w10 = tx * (256 - ty);
            let w01 = (256 - tx) * ty;
            let w11 = tx * ty;
            let (i00, i10) = (y0 * grid.w + x0, y0 * grid.w + x1);
            let (i01, i11) = (y1 * grid.w + x0, y1 * grid.w + x1);
            // Air never bleeds across a wall face: a tap whose surface layer is
            // not the pixel's own wall-ness gets no weight.
            let own = grid.surfaces[grid.sample_index(x, y)].layer & walls;
            let mask = |i: usize, weight: i32| {
                if grid.surfaces[i].layer & walls == own {
                    weight
                } else {
                    0
                }
            };
            let (m00, m10) = (mask(i00, w00), mask(i10, w10));
            let (m01, m11) = (mask(i01, w01), mask(i11, w11));
            let air = |n: usize| -> i32 {
                let sum = i32::from(grid.info[i00].air[n]) * m00
                    + i32::from(grid.info[i10].air[n]) * m10
                    + i32::from(grid.info[i01].air[n]) * m01
                    + i32::from(grid.info[i11].air[n]) * m11;
                // Q16 weights over Q8.8 air: 24 bits down to output levels.
                let levels = if let Some(correction) = correction {
                    (i64::from(sum) + (i64::from(correction[n]) << 16) + (1 << 23)) >> 24
                } else {
                    i64::from((sum + (1 << 23)) >> 24)
                };
                (levels as i32).clamp(0, 255)
            };
            let mut air = [air(0), air(1), air(2)];
            let fog = (i32::from(grid.info[i00].fog) * w00
                + i32::from(grid.info[i10].fog) * w10
                + i32::from(grid.info[i01].fog) * w01
                + i32::from(grid.info[i11].fog) * w11)
                >> 16;
            if low {
                // After the palette dither: flat steps, not a smooth wash.
                air = air.map(|a| a / AIR_STEP_LOW * AIR_STEP_LOW);
            }
            let fog = if low {
                fog / FOG_STEP_LOW * FOG_STEP_LOW
            } else {
                // Truecolor: two-level air steps and 8/255 fog steps keep runs.
                air = air.map(|a| a / AIR_STEP_OUT * AIR_STEP_OUT);
                fog / FOG_STEP_OUT * FOG_STEP_OUT
            };
            if air == [0, 0, 0] && fog == 0 {
                continue;
            }
            let px = &mut canvas.px[y * w + x];
            let (r, g, b) = (i32::from(px.0), i32::from(px.1), i32::from(px.2));
            *px = (
                blend_channel(r, air[0], fog, i32::from(fog_col.0)),
                blend_channel(g, air[1], fog, i32::from(fog_col.1)),
                blend_channel(b, air[2], fog, i32::from(fog_col.2)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    /// The Sewer sconce's colour (`fixtures::sconce_color`).
    const CAGED_SCONCE_TINT: Rgb = (255, 190, 96);
    use super::super::engine::Curve;
    use super::super::fx::testkit;
    use super::super::shadow::Surface;
    use super::super::LightShape;
    use super::*;
    use crate::game::level::{Level, LightFixture, Theme};
    use crate::render::framebuffer::PixelBuffer;

    /// A wall lamp in the Sewer's colours, facing south (down the screen).
    fn sconce_light() -> Light {
        let mut light = Light::radial(Vec2::new(60.0, 30.0), 90.0, CAGED_SCONCE_TINT, 2.35);
        light.shape = LightShape::Cone {
            radius: 90.0,
            dir: Vec2::new(0.0, 1.0),
            half_angle: 1.31,
        };
        light.height = 9.0;
        light.scatter = 0.34;
        light
    }

    fn hit<'a>(light: &'a Light, delta: Vec2, core: f32, visibility: f32) -> SampleHit<'a> {
        SampleHit {
            theme: Theme::Sewer,
            light,
            tag: LightTag::Fixture(LightFixtureKind::Sconce),
            delta,
            point: light.pos + delta,
            core,
            tail: 0.0,
            visibility,
            surface: Surface::default(),
            mode: LightingMode::Full,
        }
    }

    #[test]
    fn shafts_are_wanted_for_lamps_on_full_and_hard_only() {
        let light = sconce_light();
        for mode in [LightingMode::Full, LightingMode::Hard] {
            assert!(wants(&light, LightTag::Fixture(LightFixtureKind::Sconce), mode));
            assert!(wants(&light, LightTag::Fixture(LightFixtureKind::Pendant), mode));
        }
        for mode in [LightingMode::Coarse, LightingMode::Compact] {
            assert!(!wants(&light, LightTag::Fixture(LightFixtureKind::Sconce), mode));
        }
        // A dying bulb's beam would sputter with it; the wash is not a lamp.
        assert!(!wants(&light, LightTag::Fixture(LightFixtureKind::Broken), LightingMode::Full));
        assert!(!wants(&light, LightTag::SconceWash, LightingMode::Full));
        assert!(!wants(&light, LightTag::Sun, LightingMode::Full));
        assert!(!wants(&light, LightTag::Moving { priority: 3 }, LightingMode::Full));
    }

    #[test]
    fn a_lit_sconce_adds_air_along_its_cone_and_none_in_an_occluder_shadow() {
        let light = sconce_light();
        // Ahead of the lens, in full view: warm air, no white-out.
        let mut lit = LightInfo::default();
        splat_sample(&mut lit, &hit(&light, Vec2::new(0.0, 30.0), 0.7, 1.0));
        assert!(lit.air[0] > 0, "a lit lamp must haze its beam: {:?}", lit.air);
        assert!(lit.air[0] > lit.air[2], "the shaft keeps the lamp's warm tint");
        assert!(lit.air[0] <= AIR_MAX);
        // The same sample behind the wall (visibility 0): not one unit.
        let mut shadowed = LightInfo::default();
        splat_sample(&mut shadowed, &hit(&light, Vec2::new(0.0, 30.0), 0.7, 0.0));
        assert_eq!(shadowed.air, [0, 0, 0]);
        // Behind the lens the engine's own cone weight is exactly zero.
        let curve = Curve::new(&light);
        let limit = 1.31f32.cos();
        let (ahead, _) = curve.parts(Vec2::new(0.0, 30.0), 900.0, limit);
        let (behind, _) = curve.parts(Vec2::new(0.0, -30.0), 900.0, limit);
        assert!(ahead > 0.0 && behind <= 0.0);
        let mut back = LightInfo::default();
        splat_sample(&mut back, &hit(&light, Vec2::new(0.0, -30.0), behind, 1.0));
        assert_eq!(back.air, [0, 0, 0]);
    }

    #[test]
    fn rays_band_the_beam_by_angle_and_never_darken_past_their_depth() {
        let light = sconce_light();
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for step in 0..360 {
            let a = (step as f32).to_radians();
            let delta = Vec2::new(a.cos(), a.sin()) * 40.0;
            let h = hit(&light, delta, 1.0, 1.0);
            let r = rays(&h);
            assert_eq!(r, rays(&h), "no clock: the same sample, the same ray");
            lo = lo.min(r);
            hi = hi.max(r);
        }
        assert!(hi > 0.95, "somewhere the beam is whole ({hi})");
        assert!(lo < 0.7, "somewhere a ray is cut ({lo})");
        assert!(lo >= 1.0 - RAY_DEPTH_SCONCE - 1e-3, "never past the depth ({lo})");
        // At the lens the beam is whole.
        assert_eq!(rays(&hit(&light, Vec2::new(3.0, 2.0), 1.0, 1.0)), 1.0);
    }

    #[test]
    fn a_flickering_lamp_does_not_breathe_the_air() {
        let level = Level::empty(Theme::Sewer);
        // The lamp's flicker lives in its energy and in `FxLight::gain`; the
        // shaft reads the shape and the halo reads the colour class, so the
        // air record must not move with it.
        let steady = FxLight {
            pos: Vec2::new(60.0, 30.0),
            height: 9.0,
            color: CAGED_SCONCE_TINT,
            gain: 686.0,
            extent: 110.0,
            tag: LightTag::Fixture(LightFixtureKind::Sconce),
            steady: 686.0,
        };
        let mut dim = steady;
        dim.gain = 665.0; // a 3 % held-step dip in the sconce's core
        let (a, b) = ([steady], [dim]);
        let mut frame_a = testkit::frame(&level, LightingMode::Full);
        frame_a.lights = &a;
        frame_a.elapsed = 0.02;
        frame_a.step8 = 0;
        let mut frame_b = testkit::frame(&level, LightingMode::Full);
        frame_b.lights = &b;
        frame_b.elapsed = 0.30;
        frame_b.step8 = 2;
        let mut grid_a = testkit::flat_grid(24, 16, 4, [90, 110, 118]);
        let mut grid_b = testkit::flat_grid(24, 16, 4, [90, 110, 118]);
        // Seed the shaft term the splat stage would have written, then bloom.
        let light = sconce_light();
        for i in 0..grid_a.info.len() {
            splat_sample(&mut grid_a.info[i], &hit(&light, Vec2::new(0.0, 20.0), 0.6, 1.0));
            splat_sample(&mut grid_b.info[i], &hit(&light, Vec2::new(0.0, 20.0), 0.6, 1.0));
        }
        assert_eq!(grid_a.info, grid_b.info);
        grid_pass(&mut grid_a, &frame_a);
        grid_pass(&mut grid_b, &frame_b);
        assert_eq!(grid_a.info, grid_b.info, "air must not flicker");
        // And the splat itself ignores the flickered energy.
        let mut bright = sconce_light();
        let mut faint = sconce_light();
        faint.energy = 2.28;
        faint.scatter = 0.32;
        let (mut ia, mut ib) = (LightInfo::default(), LightInfo::default());
        splat_sample(&mut ia, &hit(&bright, Vec2::new(0.0, 30.0), 0.7, 1.0));
        splat_sample(&mut ib, &hit(&faint, Vec2::new(0.0, 30.0), 0.7, 1.0));
        assert_eq!(ia.air, ib.air);
        bright.energy = 1.0;
        bright.scatter = 0.1;
        ia = LightInfo::default();
        splat_sample(&mut ia, &hit(&bright, Vec2::new(0.0, 30.0), 0.7, 1.0));
        assert_eq!(ia.air, ib.air);
    }

    #[test]
    fn fog_grows_with_distance_and_is_zero_at_the_rat() {
        let level = Level::empty(Theme::Sewer);
        let probe = testkit::flat_grid(40, 3, 4, [0, 0, 0]);
        let mut frame = testkit::frame(&level, LightingMode::Full);
        frame.rat = probe.start; // sample (0, 0) sits under the rat
        let mut grid = probe;
        grid_pass(&mut grid, &frame);
        let row: Vec<u8> = (0..grid.w).map(|x| grid.info[x].fog).collect();
        assert_eq!(row[0], 0, "no fog at the rat");
        assert!(row.iter().any(|&v| v > 0), "fog must arrive with distance: {row:?}");
        for pair in row.windows(2) {
            assert!(pair[1] >= pair[0], "fog must not fall off: {row:?}");
        }
        // A brightly lit sample is fogged less than a dark one at the same place.
        let mut bright = testkit::flat_grid(40, 3, 4, [400, 400, 400]);
        grid_pass(&mut bright, &frame);
        let far = grid.w - 1;
        assert!(bright.info[far].fog < grid.info[far].fog);
    }

    #[test]
    fn bloom_is_zero_at_strength_zero_and_never_whites_out() {
        assert_eq!(bloom_amplitude(255, 255, 0.0), 0);
        assert!(bloom_amplitude(255, 255, strength::BLOOM) > 0);
        let level = Level::empty(Theme::Sewer);
        let lights: Vec<FxLight> = (0..8)
            .map(|i| FxLight {
                pos: Vec2::new(30.0 + i as f32 * 3.0, 30.0),
                height: 9.0,
                color: (255, 255, 255),
                gain: 686.0,
                extent: 110.0,
                tag: LightTag::Fixture(LightFixtureKind::Sconce),
                steady: 686.0,
            })
            .collect();
        let mut frame = testkit::frame(&level, LightingMode::Full);
        frame.lights = &lights;
        let mut grid = testkit::flat_grid(24, 20, 4, [80, 90, 100]);
        // A halo only lands where its own lamp dominates the sample.
        for info in grid.info.iter_mut() {
            info.dom = 400;
            info.dom_rgb = (255, 255, 255);
        }
        grid_pass(&mut grid, &frame);
        let peak = grid.info.iter().flat_map(|i| i.air).max().unwrap_or(0);
        assert!(peak > 0, "bright fixtures must bloom at all");
        assert!(
            grid.info.iter().all(|i| i.air.iter().all(|&v| v <= AIR_MAX)),
            "bloom must stay under the air ceiling"
        );
        // The picture is deepened, not blown out: with a mid floor the composited
        // pixel keeps its hue and never reaches white.
        let mut pb = PixelBuffer::new(96, 80);
        pb.set_pen(0);
        pb.clear((85, 71, 78));
        {
            let mut canvas = testkit::canvas(&mut pb);
            composite(&grid, &mut canvas, &frame);
        }
        assert!(pb.pixels().iter().all(|p| p.0.max(p.1).max(p.2) < 255));
    }

    #[test]
    fn composite_with_no_air_leaves_the_picture_bit_identical() {
        let level = Level::empty(Theme::Sewer);
        let frame = testkit::frame(&level, LightingMode::Full);
        // Every sample's air and fog at their defaults: nothing to blend.
        let grid = testkit::flat_grid(30, 20, 4, [90, 110, 118]);
        let mut pb = PixelBuffer::new(37, 23);
        for (i, p) in pb.pixels_mut().iter_mut().enumerate() {
            *p = ((i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 17 % 256) as u8);
        }
        let before = pb.pixels().to_vec();
        {
            let mut canvas = testkit::canvas(&mut pb);
            composite(&grid, &mut canvas, &frame);
        }
        assert_eq!(pb.pixels(), before.as_slice());
    }

    #[test]
    fn air_screen_blends_the_lit_picture_up_and_never_whites_it_out() {
        let level = Level::empty(Theme::Sewer);
        let mut frame = testkit::frame(&level, LightingMode::Full);
        frame.rat = Vec2::new(2.0, 2.0); // sample (0, 0) sits under the rat: no fog there
        let mut grid = testkit::flat_grid(30, 20, 4, [90, 110, 118]);
        // A shaft in the first sample only; the rest of the grid is clear air.
        splat_sample(
            &mut grid.info[0],
            &hit(&sconce_light(), Vec2::new(0.0, 20.0), 0.7, 1.0),
        );
        // The fog amounts (no lights in the frame, so no bloom).
        grid_pass(&mut grid, &frame);
        let mut pb = PixelBuffer::new(120, 80);
        pb.set_pen(0);
        pb.clear((85, 71, 78));
        let before = pb.pixels().to_vec();
        {
            let mut canvas = testkit::canvas(&mut pb);
            composite(&grid, &mut canvas, &frame);
        }
        // The pixel under sample (0, 0) is sample 0's own centre: the warm air
        // lifts it, and the picture is not left alone anywhere in the beam.
        let probe = 2 * 120 + 2;
        assert!(pb.pixels()[probe].0 > before[probe].0, "air must light the picture");
        assert!(pb.pixels()[probe].0 < 255);
        // Far from the rat the sample carries fog: the floor is toned toward the
        // theme's own haze (the Sewer's tint, dimmer than the lit floor).
        let far = 60 * 120 + 100;
        assert!(grid.info[(60 / 4) * grid.w + 100 / 4].fog > 0);
        assert!(pb.pixels()[far].0 < before[far].0, "fog must tone the far field");
    }

    #[test]
    fn a_placed_sconce_reaches_the_air_stage_through_the_pipeline() {
        // A smoke test of the real path (`apply_scene` -> `fixtures` ->
        // `wants`/`splat_sample` -> `grid_pass` -> `composite`), kept weak on
        // purpose: the other stages are filled in by other workers, so only the
        // direction of the air is asserted above, at the sample level.
        let mut world = testkit::fx_world(Theme::Sewer);
        world.player.pos = Vec2::new(40.0, 60.0);
        world.level.objects.lights.push(LightFixture {
            pos: Vec2::new(70.0, 30.0),
            kind: LightFixtureKind::Sconce,
            radius: 90.0,
            seed: 3,
        });
        let paint = |_pb: &mut PixelBuffer| {};
        let (off, full) =
            testkit::lit(&world, &[], LightingMode::Full, (120, 80), Vec2::ZERO, paint);
        assert!(off.pixels() != full.pixels());
        // Below Hard `wants` is false, so the beam is not even offered to us.
        assert!(!wants(
            &sconce_light(),
            LightTag::Fixture(LightFixtureKind::Sconce),
            LightingMode::Coarse
        ));
    }
}
