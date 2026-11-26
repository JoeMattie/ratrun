//! Standing water that reads as WATER (the owner: "specular glints in the
//! puddles and reflections please"). A lit-space pixel stage over the pixels
//! tagged `MaterialKind::Water` (the puddles and their ripples, drawn under
//! `specular::pool_pen`), run after `softshadow::reflections` and before the
//! air. In order:
//!
//! * **C. Body mirrors** ([`body_mirrors`], Full only): every water pixel
//!   looks straight up the screen for the nearest creature pixel within
//!   [`MIRROR_REACH`]; at distance `d` it shows the creature pixel mirrored
//!   about that body's foot line, `(x + wobble, y − 2d + 1)`, darkened, tinted
//!   toward the water and faded with `d`. So the rat and the horde stand on an
//!   upside-down, wobbling, dim copy of themselves. One pass, one column array
//!   on the stack; nothing is written where no body is in reach.
//! * **B. Lamp mirrors** ([`lamp_mirrors`], Full and Hard): each visible
//!   fixture (never the rat's aura or a halo) is mirrored as a bright soft
//!   ellipse [`MIRROR_K`] × its height below it, its core pulled
//!   [`MIRROR_WHITE`] toward white, its rows shifted ±1 px by the ripple
//!   field, clipped to water. It replaces `softshadow`'s wet-brick streak on
//!   water pixels (the streak still runs on brick, tile, metal and ink).
//! * **A. Glints** ([`glints`], Full and Hard): a sparse sparkle. A water
//!   pixel is a glint *candidate* one time in `256 / GLINT_DENSITY` (a hash of
//!   its world position and the 3 Hz step), and only a candidate is lit: a fake
//!   surface normal from a two-octave ripple field (value noise on
//!   [`OCT1_CELL`] px cells plus per-pixel grain, tilted further on a live
//!   ripple ring), a narrow Blinn lobe (`cos^GLINT_POWER`) against the
//!   sample's dominant light and — on Full — the rat's own aura evaluated
//!   directly, so the water sparkles round a wading rat far from any lamp.
//!   On Full the view vector leans toward the view centre ([`GLINT_VIEW_K`]),
//!   so the pattern slides as the camera moves. Each glint is 1 px, now and
//!   then a 2 px dash, pulled toward white and as bright as the light the
//!   water receives. It replaces `specular`'s per-sample sheen on water,
//!   which painted a puddle as one flat 4 px wash.
//!
//! Tiers: Full runs A (two lobes, view term), B and C; Hard runs B and A with
//! the dominant lobe only; Coarse and Compact nothing, and nothing at all at
//! 16 colours. With the effects off this file never runs, and with
//! `fx::strength::WATER` at 0 [`owns_water`] is false, `specular` and
//! `softshadow` light the water exactly as before, and [`apply`] writes
//! nothing.
//!
//! Clocks: only the 3 Hz step ([`GLINT_HZ`]) is read, so a still scene
//! repaints on the step and only the glints that moved. Render-only: no rng,
//! no simulation state; the ripples are read from the cosmetic `PuddleSteps`.

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{self, strength, Canvas, FxFrame, LightTag, DIR_ONE};
use super::material::{self, MaterialKind};
use crate::game::art_puddle;
use crate::game::ecs::resources::{PuddleSteps, RIPPLE_SLOTS};
use crate::math::Vec2;
use crate::render::palette::{mix, scale, Rgb};
use crate::term::ColorDepth;

// ============================================================================
// Tunables (every strength is one constant; `fx::strength::WATER` scales all)
// ============================================================================

// ---- A. glints -------------------------------------------------------------

/// The held step of the ripple field and the glint pattern, Hz. Raising it
/// makes the sparkle livelier and repaints more often.
pub(super) const GLINT_HZ: f32 = 3.0;
/// Glint candidates per 256 water pixels. The hard ceiling on how many pixels
/// can sparkle at once (with the dashes: `× (1 + DASH_CHANCE / 16)`, 9.3 %);
/// the lobe then keeps only the facets that face the light. Raising it makes
/// the water busier.
pub(super) const GLINT_DENSITY: u32 = 20;
/// Candidates per 256 pixels on a live ripple ring (a ring sparkles harder).
pub(super) const RIPPLE_DENSITY: u32 = 96;
/// Half-width of a ripple ring's glint band, px.
pub(super) const RIPPLE_BAND: f32 = 1.5;
/// Extra outward tilt of the normal on a ripple ring, Q0.7 (127 = 90°).
pub(super) const RIPPLE_TILT: i32 = 40;
/// The ripple field's broad octave: value-noise cell size (px) and tilt (Q0.7).
/// A bigger tilt spreads the glints over more of the puddle (further from the
/// mirror direction of the light).
pub(super) const OCT1_CELL: i32 = 8;
pub(super) const OCT1_TILT: i32 = 40;
/// The fine octave: per-pixel grain tilt, Q0.7.
pub(super) const OCT2_TILT: i32 = 22;
/// The highlight's exponent: higher is a narrower lobe (fewer, harder glints).
pub(super) const GLINT_POWER: i32 = 48;
/// Lobe values under this are no glint at all (the rest is rescaled to 0..1),
/// so a glint is a crisp point, never a faint smear. Raising it thins them.
pub(super) const GLINT_MIN_LOBE: f32 = 0.30;
/// Received light (`Q8.8`, `LightInfo::dom`) under which water never glints,
/// and at which a glint reaches full brightness.
pub(super) const GLINT_MIN_LIGHT: f32 = 20.0;
pub(super) const GLINT_FULL_LIGHT: f32 = 160.0;
/// A full glint's lift toward white, in levels of 256. The brightness knob.
pub(super) const GLINT_PEAK: f32 = 230.0;
/// How far a glint's colour is pulled toward white, /256 (`specular`'s
/// `WET_WHITE` is 160 for slime; water's glint is the lamp's image).
pub(super) const GLINT_WHITE: i32 = 190;
/// Of 16 candidates, how many glint as a 2 px horizontal dash, and the second
/// pixel's share of the first's brightness.
pub(super) const DASH_CHANCE: u32 = 3;
pub(super) const DASH_SHARE: f32 = 0.6;
/// View dependence (Full): `(view centre − pixel) × K` leans the view vector,
/// so the glints slide as the camera moves. 0 pins them to the world.
pub(super) const GLINT_VIEW_K: f32 = 0.0035;
/// The view direction (`specular`'s; slightly off axis).
const VIEW: [f32; 3] = [0.0, 0.24, 0.97];
/// The rat's aura as a glint light (Full): its height above the water (px),
/// its peak energy (`Q8.8`) and its reach (the aura's own radius, 66 px,
/// 58 on the Lab: `sources.rs`). Raising the energy brightens the sparkle
/// round a wading rat.
pub(super) const RAT_AURA_HEIGHT: f32 = 12.0;
pub(super) const RAT_GLINT_ENERGY: f32 = 150.0;
pub(super) const RAT_GLINT_RADIUS: f32 = 66.0;
pub(super) const RAT_GLINT_RADIUS_LAB: f32 = 58.0;

// ---- B. lamp mirrors -------------------------------------------------------

/// The mirror image sits this many fixture heights below the lamp on screen.
pub(super) const MIRROR_K: f32 = 1.0;
/// The image's half-width (px): a base plus a share of the fixture's height.
pub(super) const MIRROR_RX: f32 = 2.5;
pub(super) const MIRROR_RX_PER_H: f32 = 0.08;
/// Height over width of the image (a reflection is a vertical column).
pub(super) const MIRROR_ASPECT: f32 = 2.2;
/// The core's pull toward white (0.6 = 60 %).
pub(super) const MIRROR_WHITE: f32 = 0.6;
/// The image's opacity at its core at full lamp strength. With 0.9 it is the
/// brightest thing in the puddle.
pub(super) const MIRROR_PEAK: f32 = 0.9;
/// A fixture dimmer than this (steady `Q8.8` peak) is not mirrored; at the
/// second it is mirrored at full strength.
pub(super) const MIRROR_MIN_GAIN: f32 = 48.0;
pub(super) const MIRROR_FULL_GAIN: f32 = 256.0;
/// Received light (`LightInfo::spec`) at which the image is ungated: a
/// puddle in a wall's shadow does not show the lamp.
pub(super) const MIRROR_GATE_SPEC: f32 = 24.0;
/// The most fixtures mirrored a frame.
pub(super) const MAX_MIRRORS: usize = 5;
/// How far a row of the image shifts (± px) with the ripple field.
pub(super) const MIRROR_WOBBLE_PX: i32 = 1;

// ---- C. body mirrors -------------------------------------------------------

/// How far (px) a water pixel looks up the screen for a body. Also where the
/// image has faded to nothing.
pub(super) const MIRROR_REACH: usize = 10;
/// The body image's opacity next to the body (d = 1).
pub(super) const BODY_MIRROR: f32 = 0.32;
/// The body image is this much darker than the body (0.30 = 30 %).
pub(super) const BODY_DARKEN: f32 = 0.30;
/// ... and tinted this far toward the water under it.
pub(super) const BODY_TINT: f32 = 0.15;
/// How far (± px) a row of the body image shifts with the ripple field.
pub(super) const BODY_WOBBLE_PX: i32 = 1;
/// Columns the body pass handles (a stack array); wider buffers skip the rest.
const MAX_COLS: usize = 2048;

// Hash salts: one stream per use, so the patterns do not line up.
const GATE_SALT: u32 = 0x6a09_e667;
const NX_SALT: u32 = 0xbb67_ae85;
const NY_SALT: u32 = 0x3c6e_f372;
const MIRROR_SALT: u32 = 0xa54f_f53a;
const BODY_SALT: u32 = 0x510e_527f;

/// Is the water stage switched on at all? Read by the art: the dark-mirror
/// base (`art_puddle::GLOSS_DEEPEN`) is drawn only while something lights it.
pub(crate) fn gloss_enabled() -> bool {
    strength::WATER > 0.0
}

/// Does this stage own the water this frame? When it does, `specular` leaves
/// water pixels to the glints and `softshadow::reflections` to the mirrors.
/// `RATRUN_FX_SKIP=water` (test builds) hands the water back to them, which
/// is the A/B capture.
pub(super) fn owns_water(mode: LightingMode) -> bool {
    gloss_enabled() && matches!(mode, LightingMode::Full | LightingMode::Hard) && !fx::skip("water")
}

/// A live ripple ring, in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WaterRipple {
    pub centre: Vec2,
    pub rx: f32,
    pub ry: f32,
}

impl WaterRipple {
    pub(super) const NONE: Self = Self {
        centre: Vec2::ZERO,
        rx: 0.0,
        ry: 0.0,
    };
}

/// The most ripples a frame can hold (`PuddleSteps::ripples`).
pub(super) const MAX_RIPPLES: usize = RIPPLE_SLOTS;

/// The rat's live ripples from the cosmetic `PuddleSteps`, in canvas pixels
/// (`cam` is the frame's rounded camera); returns how many were written.
pub(super) fn gather_ripples(
    ecs: &bevy_ecs::world::World,
    cam: Vec2,
    out: &mut [WaterRipple; MAX_RIPPLES],
) -> usize {
    let Some(steps) = ecs.get_resource::<PuddleSteps>() else {
        return 0;
    };
    let mut n = 0;
    for r in steps.ripples.iter().filter(|r| r.live) {
        if n >= out.len() {
            break;
        }
        if let Some((rx, ry)) = art_puddle::ripple_radii(r.age, r.big) {
            out[n] = WaterRipple {
                centre: r.pos - cam,
                rx,
                ry,
            };
            n += 1;
        }
    }
    n
}

/// The stage (`fx::strength::WATER`).
pub(super) fn apply(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, ripples: &[WaterRipple]) {
    run(grid, canvas, frame, ripples, strength::WATER);
}

/// [`apply`] at strength `k`, so a test can prove 0 is a byte no-op.
fn run(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, ripples: &[WaterRipple], k: f32) {
    let full = match frame.mode {
        LightingMode::Full => true,
        LightingMode::Hard => false,
        _ => return,
    };
    if k <= 0.0 || frame.depth == ColorDepth::Ansi16 {
        return;
    }
    if canvas.w == 0 || canvas.h == 0 || canvas.mat.len() != canvas.w * canvas.h {
        return;
    }
    let z = step3(frame.elapsed);
    if full {
        body_mirrors(canvas, frame, k, z);
    }
    lamp_mirrors(grid, canvas, frame, k, z);
    glints(grid, canvas, frame, ripples, k, full, z);
}

// ============================================================================
// Shared pieces
// ============================================================================

#[inline]
fn hash3(x: i32, y: i32, z: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b9)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ z.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^= n >> 16;
    n
}

/// `floor(elapsed * GLINT_HZ)`: the one clock this stage reads.
#[inline]
fn step3(elapsed: f32) -> u32 {
    (elapsed.max(0.0) * GLINT_HZ).floor() as u32
}

/// A world row's shift (−1, 0 or +1) on step `z`: the wobble of a reflection.
#[inline]
fn wobble(wy: i32, z: u32, salt: u32) -> i32 {
    (hash3(0x5157, wy, z ^ salt) % 3) as i32 - 1
}

#[inline]
fn is_water(m: u8) -> bool {
    !material::is_creature(m) && material::kind(m) == MaterialKind::Water
}

/// Is grid sample `si` open floor (not a wall face or a prop top)?
#[inline]
fn floor_sample(grid: &Grid, si: usize) -> bool {
    grid.surfaces
        .get(si)
        .is_some_and(|s| s.height <= 0.0 && s.layer & LayerMask::FLOOR.0 != 0)
}

/// Is the canvas point `(x, y)` on a wall sample? (the rat lobe's wall guard)
fn wall_at(grid: &Grid, w: usize, h: usize, x: f32, y: f32) -> bool {
    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
        return false;
    }
    let si = grid.sample_index(x as usize, y as usize);
    grid.surfaces
        .get(si)
        .is_some_and(|s| s.layer & LayerMask::WALLS.0 != 0)
}

#[inline]
fn pull_white(c: Rgb, amount: i32) -> Rgb {
    let m = |v: u8| (i32::from(v) + (((255 - i32::from(v)) * amount) >> 8)) as u8;
    (m(c.0), m(c.1), m(c.2))
}

/// Additive light toward white (`specular::lift`).
#[inline]
fn lift(pixel: u8, add: i32) -> u8 {
    let add = add.clamp(0, 255);
    (i32::from(pixel) + (((255 - i32::from(pixel)) * add) >> 8)) as u8
}

/// Move `p` toward `c` by `a`, brightening only (an image of a lamp never
/// darkens the water it lands on).
#[inline]
fn toward(p: Rgb, c: Rgb, a: f32) -> Rgb {
    let ch = |p: u8, c: u8| {
        if c > p {
            (f32::from(p) + (f32::from(c) - f32::from(p)) * a).min(255.0) as u8
        } else {
            p
        }
    };
    (ch(p.0, c.0), ch(p.1, c.1), ch(p.2, c.2))
}

// ============================================================================
// C. Body mirrors
// ============================================================================

fn body_mirrors(canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, k: f32, z: u32) {
    let stride = canvas.w;
    let (w, h) = (canvas.w.min(MAX_COLS), canvas.h);
    // The row of the nearest creature pixel above, per column.
    let mut last = [i32::MIN / 2; MAX_COLS];
    let cam_y = frame.cam.y as i32;
    let reach = MIRROR_REACH as i32;
    let alpha0 = (BODY_MIRROR * k).min(1.0);
    for y in 0..h {
        let yi = y as i32;
        let shift = wobble(cam_y + yi, z, BODY_SALT) * BODY_WOBBLE_PX;
        let row = y * stride;
        for x in 0..w {
            let i = row + x;
            let m = canvas.mat[i];
            if material::is_creature(m) {
                last[x] = yi;
                continue;
            }
            if material::kind(m) != MaterialKind::Water {
                continue;
            }
            let d = yi - last[x];
            if d < 1 || d >= reach {
                continue;
            }
            let sy = yi - 2 * d + 1;
            let sx = x as i32 + shift;
            if sy < 0 || sx < 0 || sx >= w as i32 {
                continue;
            }
            let si = sy as usize * stride + sx as usize;
            if !material::is_creature(canvas.mat[si]) {
                continue;
            }
            let here = canvas.px[i];
            let image = mix(scale(canvas.px[si], 1.0 - BODY_DARKEN), here, BODY_TINT);
            let fade = (reach - d) as f32 / (reach - 1) as f32;
            canvas.px[i] = mix(here, image, alpha0 * fade);
        }
    }
}

// ============================================================================
// B. Lamp mirrors
// ============================================================================

fn lamp_mirrors(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, k: f32, z: u32) {
    let (w, h) = (canvas.w as i32, canvas.h as i32);
    let cam_y = frame.cam.y as i32;
    let mut done = 0;
    for l in frame.lights {
        if done >= MAX_MIRRORS {
            break;
        }
        // Lamps only (as `softshadow::reflections`): the aura and the halos
        // have no lamp to mirror. The steady gain, so a flicker does not
        // repaint the image every frame.
        if !matches!(l.tag, LightTag::Fixture(_)) || !l.extent.is_finite() || l.steady < MIRROR_MIN_GAIN {
            continue;
        }
        let s = l.pos - frame.cam;
        let height = l.height.clamp(2.0, 48.0);
        let (cx, cy) = (s.x, s.y + MIRROR_K * height);
        let rx = MIRROR_RX + height * MIRROR_RX_PER_H;
        let ry = rx * MIRROR_ASPECT;
        let pad = MIRROR_WOBBLE_PX as f32 + 1.0;
        let x0 = ((cx - rx - pad).floor() as i32).max(0);
        let x1 = ((cx + rx + pad).ceil() as i32 + 1).min(w);
        let y0 = ((cy - ry - 1.0).floor() as i32).max(0);
        let y1 = ((cy + ry + 1.0).ceil() as i32 + 1).min(h);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        done += 1;
        let level = (l.steady / MIRROR_FULL_GAIN).min(1.0);
        let peak = MIRROR_PEAK * level * k;
        if peak <= 0.0 {
            continue;
        }
        let salt = hash3(l.pos.x as i32, l.pos.y as i32, MIRROR_SALT);
        for py in y0..y1 {
            let dy = (py as f32 + 0.5 - cy) / ry;
            if dy.abs() >= 1.0 {
                continue;
            }
            let shift = (wobble(cam_y + py, z, salt) * MIRROR_WOBBLE_PX) as f32;
            for px in x0..x1 {
                let i = py as usize * canvas.w + px as usize;
                if !is_water(canvas.mat[i]) {
                    continue;
                }
                let dx = (px as f32 + 0.5 - cx - shift) / rx;
                let r2 = dx * dx + dy * dy;
                if r2 >= 1.0 {
                    continue;
                }
                let si = grid.sample_index(px as usize, py as usize);
                let Some(info) = grid.info.get(si) else {
                    continue;
                };
                if !floor_sample(grid, si) {
                    continue;
                }
                let gate = (f32::from(info.spec) / MIRROR_GATE_SPEC).min(1.0);
                if gate <= 0.0 {
                    continue;
                }
                let wgt = (1.0 - r2) * (1.0 - r2);
                let colour = mix(l.color, (255, 255, 255), MIRROR_WHITE * wgt);
                let alpha = (peak * wgt * gate).min(1.0);
                canvas.px[i] = toward(canvas.px[i], colour, alpha);
            }
        }
    }
}

// ============================================================================
// A. Glints
// ============================================================================

/// Value noise on [`OCT1_CELL`] cells, −128..=127.
fn value_noise(wx: i32, wy: i32, z: u32) -> i32 {
    let c = OCT1_CELL;
    let (cx, cy) = (wx.div_euclid(c), wy.div_euclid(c));
    let (fx, fy) = (wx.rem_euclid(c), wy.rem_euclid(c));
    let corner = |x: i32, y: i32| (hash3(x, y, z) & 255) as i32 - 128;
    let (a, b) = (corner(cx, cy), corner(cx + 1, cy));
    let (d, e) = (corner(cx, cy + 1), corner(cx + 1, cy + 1));
    (a * (c - fx) * (c - fy) + b * fx * (c - fy) + d * (c - fx) * fy + e * fx * fy) / (c * c)
}

/// The outward tilt (Q0.7) of a ripple ring under canvas point `p`, if any.
fn ring_tilt(ripples: &[WaterRipple], p: Vec2) -> Option<[i32; 2]> {
    for r in ripples {
        if r.rx <= 0.0 || r.ry <= 0.0 {
            continue;
        }
        let d = p - r.centre;
        let (ex, ey) = (d.x / r.rx, d.y / r.ry);
        let q = (ex * ex + ey * ey).sqrt();
        if ((q - 1.0) * (r.rx + r.ry) * 0.5).abs() > RIPPLE_BAND {
            continue;
        }
        let len = d.len().max(0.001);
        return Some([
            (d.x / len * RIPPLE_TILT as f32) as i32,
            (d.y / len * RIPPLE_TILT as f32) as i32,
        ]);
    }
    None
}

/// One light as a glint lobe: unit direction toward it, energy 0..1, colour.
struct Lobe {
    dir: [f32; 3],
    energy: f32,
    rgb: Rgb,
}

#[inline]
fn light_ramp(e: f32) -> f32 {
    if e < GLINT_MIN_LIGHT {
        0.0
    } else {
        ((e - GLINT_MIN_LIGHT) / (GLINT_FULL_LIGHT - GLINT_MIN_LIGHT)).clamp(0.0, 1.0)
    }
}

/// The rescaled narrow lobe (0..1) of normal `n` for light `l` and view `v`.
#[inline]
fn lobe(n: [f32; 3], l: [f32; 3], v: [f32; 3]) -> f32 {
    let hv = [l[0] + v[0], l[1] + v[1], l[2] + v[2]];
    let len = (hv[0] * hv[0] + hv[1] * hv[1] + hv[2] * hv[2]).sqrt();
    if len <= 0.001 || len.is_nan() {
        return 0.0;
    }
    let cos = ((n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]) / len).max(0.0);
    let s = cos.powi(GLINT_POWER);
    if s <= GLINT_MIN_LOBE {
        0.0
    } else {
        (s - GLINT_MIN_LOBE) / (1.0 - GLINT_MIN_LOBE)
    }
}

/// The frame-wide inputs of the glint pass.
struct GlintCtx<'a> {
    grid: &'a Grid,
    ripples: &'a [WaterRipple],
    full: bool,
    z: u32,
    w: usize,
    h: usize,
    cam: (i32, i32),
    centre: (f32, f32),
    rat: Vec2,
    rat_r: f32,
    rat_rgb: Rgb,
    peak: f32,
}

/// One pixel's glint: the per-channel lift and whether it runs on as a dash.
fn glint_at(ctx: &GlintCtx<'_>, x: usize, y: usize) -> Option<([i32; 3], bool)> {
    let (wx, wy) = (ctx.cam.0 + x as i32, ctx.cam.1 + y as i32);
    let hsh = hash3(wx, wy, ctx.z ^ GATE_SALT);
    let gate = hsh & 255;
    if gate >= GLINT_DENSITY.max(RIPPLE_DENSITY) {
        return None;
    }
    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
    let ring = if ctx.ripples.is_empty() {
        None
    } else {
        ring_tilt(ctx.ripples, p)
    };
    if gate >= GLINT_DENSITY && ring.is_none() {
        return None;
    }
    let si = ctx.grid.sample_index(x, y);
    if !floor_sample(ctx.grid, si) {
        return None;
    }
    let info = ctx.grid.info.get(si).copied().unwrap_or_default();
    // The lights this facet can catch.
    let mut lobes: [Option<Lobe>; 2] = [None, None];
    if info.dir != [0, 0, 0] {
        let energy = light_ramp(f32::from(info.dom));
        if energy > 0.0 {
            let s = 1.0 / f32::from(DIR_ONE);
            lobes[0] = Some(Lobe {
                dir: [
                    f32::from(info.dir[0]) * s,
                    f32::from(info.dir[1]) * s,
                    f32::from(info.dir[2]) * s,
                ],
                energy,
                rgb: pull_white(info.dom_rgb, GLINT_WHITE),
            });
        }
    }
    if ctx.full {
        let d = ctx.rat - p;
        let d2 = d.len_sq();
        let r2 = ctx.rat_r * ctx.rat_r;
        if d2 < r2 {
            let fall = 1.0 - d2 / r2;
            let energy = light_ramp(RAT_GLINT_ENERGY * fall * fall);
            let mid = (ctx.rat + p) * 0.5;
            if energy > 0.0 && !wall_at(ctx.grid, ctx.w, ctx.h, mid.x, mid.y) {
                let len = (d2 + RAT_AURA_HEIGHT * RAT_AURA_HEIGHT).sqrt().max(0.001);
                lobes[1] = Some(Lobe {
                    dir: [d.x / len, d.y / len, RAT_AURA_HEIGHT / len],
                    energy,
                    rgb: ctx.rat_rgb,
                });
            }
        }
    }
    if lobes[0].is_none() && lobes[1].is_none() {
        return None;
    }
    // The facet: two octaves of ripple plus the ring's tilt.
    let tilt = ring.unwrap_or([0, 0]);
    let nx = value_noise(wx, wy, ctx.z ^ NX_SALT) * OCT1_TILT / 128
        + (((hsh >> 16) & 255) as i32 - 128) * OCT2_TILT / 128
        + tilt[0];
    let ny = value_noise(wx, wy, ctx.z ^ NY_SALT) * OCT1_TILT / 128
        + (((hsh >> 24) & 255) as i32 - 128) * OCT2_TILT / 128
        + tilt[1];
    let (nx, ny) = (nx.clamp(-110, 110) as f32 / 127.0, ny.clamp(-110, 110) as f32 / 127.0);
    let n = [nx, ny, (1.0 - nx * nx - ny * ny).max(0.0).sqrt()];
    // The view vector, leaning toward the view centre on Full.
    let v = if ctx.full {
        let vx = VIEW[0] + (ctx.centre.0 - p.x) * GLINT_VIEW_K;
        let vy = VIEW[1] + (ctx.centre.1 - p.y) * GLINT_VIEW_K;
        let len = (vx * vx + vy * vy + VIEW[2] * VIEW[2]).sqrt();
        [vx / len, vy / len, VIEW[2] / len]
    } else {
        VIEW
    };
    let mut best = 0.0f32;
    let mut rgb = (255, 255, 255);
    for l in lobes.iter().flatten() {
        let a = lobe(n, l.dir, v) * l.energy;
        if a > best {
            best = a;
            rgb = l.rgb;
        }
    }
    let amount = best * ctx.peak;
    if amount < 1.0 {
        return None;
    }
    let add = [
        (amount * f32::from(rgb.0) / 255.0) as i32,
        (amount * f32::from(rgb.1) / 255.0) as i32,
        (amount * f32::from(rgb.2) / 255.0) as i32,
    ];
    let dash = ((hsh >> 8) & 15) < DASH_CHANCE;
    Some((add, dash))
}

#[allow(clippy::too_many_arguments)]
fn glints(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    ripples: &[WaterRipple],
    k: f32,
    full: bool,
    z: u32,
) {
    let (w, h) = (canvas.w, canvas.h);
    let low_colour = frame.depth != ColorDepth::Truecolor;
    let rat_r = if frame.theme == crate::game::level::Theme::Lab {
        RAT_GLINT_RADIUS_LAB
    } else {
        RAT_GLINT_RADIUS
    };
    let ctx = GlintCtx {
        grid,
        ripples,
        full,
        z,
        w,
        h,
        cam: (frame.cam.x as i32, frame.cam.y as i32),
        centre: (w as f32 * 0.5, h as f32 * 0.5),
        rat: frame.rat - frame.cam,
        rat_r,
        rat_rgb: pull_white(super::theme_ambient(frame.theme).rat_tint, GLINT_WHITE),
        peak: GLINT_PEAK * k,
    };
    for y in 0..h {
        let row = y * w;
        // A dash from the pixel to the left.
        let mut carry: Option<[i32; 3]> = None;
        for x in 0..w {
            let i = row + x;
            let left = carry.take();
            if !is_water(canvas.mat[i]) {
                continue;
            }
            let mut add = [0i32; 3];
            if let Some((own, dash)) = glint_at(&ctx, x, y) {
                add = own;
                if dash {
                    carry = Some(own.map(|a| (a as f32 * DASH_SHARE) as i32));
                }
            }
            if let Some(l) = left {
                for (a, b) in add.iter_mut().zip(l) {
                    *a = (*a).max(b);
                }
            }
            let mut any = false;
            for a in add.iter_mut() {
                if low_colour {
                    *a &= !7;
                }
                any |= *a > 0;
            }
            if any {
                let p = canvas.px[i];
                canvas.px[i] = (lift(p.0, add[0]), lift(p.1, add[1]), lift(p.2, add[2]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::{Grid, LightingMode};
    use super::super::fx::{testkit, FxLight, LightTag, DIR_ONE};
    use super::super::material::{self, MaterialKind};
    use super::{run, WaterRipple, GLINT_HZ, MIRROR_REACH};
    use crate::game::level::{Level, LightFixtureKind, Theme};
    use crate::math::Vec2;
    use crate::render::framebuffer::PixelBuffer;
    use crate::render::palette::Rgb;

    const STONE: Rgb = (60, 66, 70);
    const WATER: Rgb = (30, 50, 58);
    const SIZE: (usize, usize) = (64, 48);

    /// A stone buffer with materials on.
    fn buffer() -> PixelBuffer {
        let mut pb = PixelBuffer::new(SIZE.0, SIZE.1);
        pb.set_materials(true);
        pb.set_pen(material::pen(MaterialKind::Stone, false, 0));
        pb.clear(STONE);
        pb.set_pen(0);
        pb
    }

    fn paint(pb: &mut PixelBuffer, rect: (i32, i32, i32, i32), kind: MaterialKind, creature: bool, c: Rgb) {
        let prev = pb.set_pen(material::pen(kind, creature, 2));
        pb.rect_fill(rect.0, rect.1, rect.2, rect.3, c);
        pb.set_pen(prev);
    }

    /// Every sample lit from straight overhead at `dom` (`Q8.8`).
    fn grid(dom: u16) -> Grid {
        let mut g = testkit::flat_grid(SIZE.0 / 4, SIZE.1 / 4, 4, [256; 3]);
        for info in g.info.iter_mut() {
            if dom > 0 {
                info.dir = [0, 0, DIR_ONE];
                info.dom = dom;
                info.dom_rgb = (255, 236, 200);
            }
            info.spec = dom;
        }
        g
    }

    /// Run the stage over `pb` with the rat far away unless `rat` says.
    #[allow(clippy::too_many_arguments)]
    fn stage(
        pb: &mut PixelBuffer,
        g: &Grid,
        mode: LightingMode,
        elapsed: f32,
        cam: Vec2,
        rat: Option<Vec2>,
        lights: &[FxLight],
        k: f32,
    ) {
        let level = Level::empty(Theme::Sewer);
        let mut frame = testkit::frame(&level, mode);
        frame.elapsed = elapsed;
        frame.cam = cam;
        frame.rat = rat.map_or(Vec2::new(-5000.0, -5000.0), |r| r + cam);
        frame.lights = lights;
        let mut canvas = testkit::canvas(pb);
        run(g, &mut canvas, &frame, &[] as &[WaterRipple], k);
    }

    fn changed(a: &PixelBuffer, b: &PixelBuffer) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..a.h {
            for x in 0..a.w {
                if a.pixel_at(x, y) != b.pixel_at(x, y) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// Water on the right two thirds, stone on the left.
    fn puddle() -> PixelBuffer {
        let mut pb = buffer();
        paint(&mut pb, (16, 0, 48, 48), MaterialKind::Water, false, WATER);
        pb
    }

    #[test]
    fn no_light_no_glint() {
        let before = puddle();
        let mut after = puddle();
        stage(&mut after, &grid(0), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
        assert!(changed(&before, &after).is_empty());
        // A whisper of light is still none.
        let mut dim = puddle();
        stage(&mut dim, &grid(10), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
        assert!(changed(&before, &dim).is_empty());
    }

    #[test]
    fn a_lit_puddle_glints_sparsely_and_only_on_water() {
        let before = puddle();
        let mut after = puddle();
        stage(&mut after, &grid(220), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
        let moved = changed(&before, &after);
        let water_px = 48 * 48;
        assert!(!moved.is_empty(), "a lit puddle sparkles");
        assert!(moved.len() * 10 <= water_px, "sparse: {} of {water_px}", moved.len());
        for (x, y) in &moved {
            assert!(*x >= 16, "glint on stone at {x},{y}");
            let (a, b) = (before.pixel_at(*x, *y), after.pixel_at(*x, *y));
            assert!(b.0 >= a.0 && b.1 >= a.1 && b.2 >= a.2, "a glint only brightens");
        }
    }

    #[test]
    fn the_pattern_holds_within_a_step_and_moves_across_one() {
        let run_at = |elapsed: f32, cam: Vec2| {
            let mut pb = puddle();
            stage(&mut pb, &grid(220), LightingMode::Full, elapsed, cam, None, &[], 1.0);
            pb
        };
        let step = 1.0 / GLINT_HZ;
        let a = run_at(3.0 * step + 0.01, Vec2::ZERO);
        let b = run_at(3.0 * step + 0.2, Vec2::ZERO);
        assert_eq!(a.pixels(), b.pixels(), "held within a step");
        let c = run_at(4.0 * step + 0.01, Vec2::ZERO);
        assert_ne!(a.pixels(), c.pixels(), "a new step moves the glints");
        // Hard has no view term and no rat lobe: also still within a step.
        let mut h1 = puddle();
        let mut h2 = puddle();
        stage(&mut h1, &grid(220), LightingMode::Hard, 1.01, Vec2::ZERO, None, &[], 1.0);
        stage(&mut h2, &grid(220), LightingMode::Hard, 1.2, Vec2::ZERO, None, &[], 1.0);
        assert_eq!(h1.pixels(), h2.pixels());
    }

    #[test]
    fn the_rat_aura_sparkles_the_water_on_full_only() {
        let before = puddle();
        let mut full = puddle();
        let rat = Some(Vec2::new(40.0, 24.0));
        stage(&mut full, &grid(0), LightingMode::Full, 1.0, Vec2::ZERO, rat, &[], 1.0);
        assert!(!changed(&before, &full).is_empty(), "the rat's own light glints");
        let mut hard = puddle();
        stage(&mut hard, &grid(0), LightingMode::Hard, 1.0, Vec2::ZERO, rat, &[], 1.0);
        assert!(changed(&before, &hard).is_empty(), "Hard: the dominant lobe only");
    }

    /// A body (blue top half, red bottom half) at x 10..20, y 4..14, over a
    /// water strip x 6..26, y 14..34; stone everywhere else.
    fn wading(with_body: bool) -> PixelBuffer {
        let mut pb = buffer();
        paint(&mut pb, (6, 14, 20, 20), MaterialKind::Water, false, WATER);
        if with_body {
            paint(&mut pb, (10, 4, 10, 5), MaterialKind::Fur, true, (0, 0, 255));
            paint(&mut pb, (10, 9, 10, 5), MaterialKind::Fur, true, (255, 0, 0));
        }
        pb
    }

    #[test]
    fn a_body_is_mirrored_upside_down_under_itself() {
        let before = wading(true);
        let mut after = wading(true);
        stage(&mut after, &grid(0), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
        // The foot line is under row 13: d = 1 (y 14) mirrors row 13 (red),
        // d = 6 (y 19) mirrors row 8 (blue).
        let near = after.pixel_at(15, 14);
        assert!(near.0 > WATER.0 + 20, "red foot under the body: {near:?}");
        let deeper = after.pixel_at(15, 19);
        assert!(deeper.2 > WATER.2 + 10, "blue head further down: {deeper:?}");
        assert!(deeper.0 <= WATER.0, "and no red there: {deeper:?}");
        // Fades out: nothing at or past the reach.
        let past = 14 + MIRROR_REACH - 1;
        for y in past..34 {
            for x in 6..26 {
                assert_eq!(after.pixel_at(x, y), WATER, "faded out at {x},{y}");
            }
        }
        // Only water pixels were written.
        for (x, y) in changed(&before, &after) {
            assert!((6..26).contains(&x) && (14..34).contains(&y), "wrote off the water at {x},{y}");
        }
        // No body, no image.
        let plain = wading(false);
        let mut alone = wading(false);
        stage(&mut alone, &grid(0), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
        assert!(changed(&plain, &alone).is_empty(), "no smear without a body");
        // Hard leaves bodies unmirrored.
        let mut hard = wading(true);
        stage(&mut hard, &grid(0), LightingMode::Hard, 1.0, Vec2::ZERO, None, &[], 1.0);
        assert!(changed(&before, &hard).is_empty());
    }

    fn lamp(pos: Vec2) -> FxLight {
        FxLight {
            pos,
            height: 16.0,
            color: (255, 210, 150),
            gain: 300.0,
            extent: 120.0,
            tag: LightTag::Fixture(LightFixtureKind::Pendant),
            steady: 300.0,
        }
    }

    #[test]
    fn a_lamp_is_mirrored_below_itself_and_only_in_water() {
        // Received light but no dominant direction: the mirror alone.
        let mut g = grid(0);
        for info in g.info.iter_mut() {
            info.spec = 200;
        }
        let lights = [lamp(Vec2::new(40.0, 6.0))];
        let before = puddle();
        let mut after = puddle();
        stage(&mut after, &g, LightingMode::Hard, 1.0, Vec2::ZERO, None, &lights, 1.0);
        let moved = changed(&before, &after);
        assert!(moved.len() > 10, "an image of the lamp ({} px)", moved.len());
        let mut brightest = (0u32, 0usize, 0usize);
        for &(x, y) in &moved {
            assert!(x >= 16, "the image stays in the water");
            let p = after.pixel_at(x, y);
            let l = u32::from(p.0) + u32::from(p.1) + u32::from(p.2);
            if l > brightest.0 {
                brightest = (l, x, y);
            }
        }
        // Centred under the lamp, MIRROR_K × 16 px below it.
        assert!((brightest.1 as i32 - 40).abs() <= 2, "{brightest:?}");
        assert!((brightest.2 as i32 - 22).abs() <= 3, "{brightest:?}");
        // A halo is not a lamp.
        let mut halo = lamp(Vec2::new(40.0, 6.0));
        halo.tag = LightTag::Moving { priority: 3 };
        let mut none = puddle();
        stage(&mut none, &g, LightingMode::Full, 1.0, Vec2::ZERO, None, &[halo], 1.0);
        assert!(changed(&before, &none).is_empty());
    }

    #[test]
    fn strength_zero_and_the_low_tiers_write_nothing() {
        let lights = [lamp(Vec2::new(40.0, 6.0))];
        let before = wading(true);
        for (mode, k) in [
            (LightingMode::Full, 0.0),
            (LightingMode::Coarse, 1.0),
            (LightingMode::Compact, 1.0),
        ] {
            let mut pb = wading(true);
            stage(&mut pb, &grid(220), mode, 1.0, Vec2::ZERO, Some(Vec2::new(15.0, 20.0)), &lights, k);
            assert_eq!(before.pixels(), pb.pixels(), "{mode:?} at {k}");
        }
    }
}
