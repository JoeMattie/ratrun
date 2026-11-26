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
//!   upside-down, dim copy of themselves. One pass, one column array
//!   on the stack; nothing is written where no body is in reach.
//! * **B. Lamp mirrors** ([`lamp_mirrors`], Full and Hard): each visible
//!   fixture (never the rat's aura or a halo) is mirrored as a bright soft
//!   ellipse [`MIRROR_K`] × its height below it, its core pulled
//!   [`MIRROR_WHITE`] toward white, its rows shifted ±1 px by a world-row
//!   hash, clipped to water. It replaces `softshadow`'s wet-brick streak on
//!   water pixels (the streak still runs on brick, tile, metal and ink).
//! * **A. Glints** ([`glints`], Full and Hard): a sparse sparkle. A water
//!   pixel is a glint *candidate* if its world-position hash passes
//!   [`GLINT_DENSITY`], and only a candidate is lit: a fake
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
//! Full and Hard also mirror Laser Eyes and Carbon Copy over the waterline
//! and add glints along the reflected rays.
//!
//! Tiers: Full runs A (two lobes, view term), B and C; Hard runs B and A with
//! the dominant lobe only; Coarse and Compact nothing, and nothing at all at
//! 16 colours. With the effects off this file never runs, and with
//! `fx::strength::WATER` at 0 [`owns_water`] is false, `specular` and
//! `softshadow` light the water exactly as before, and [`apply`] writes
//! nothing.
//!
//! The surface pattern and row offsets are fixed in world space. Live rings
//! from the cosmetic `PuddleSteps` bend the reflections and tilt the glints
//! where they pass. This stage reads no clock and draws no rng.

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{self, strength, Canvas, FxFrame, LightTag, DIR_ONE};
use super::material::{self, MaterialKind};
use super::sources::LaserSegment;
use crate::game::art_puddle;
use crate::game::ecs::resources::{PuddleSteps, RIPPLE_SLOTS};
use crate::math::Vec2;
use crate::render::palette::{mix, scale, Rgb};
use crate::term::ColorDepth;

// ============================================================================
// Tunables (every strength is one constant; `fx::strength::WATER` scales all)
// ============================================================================

// ---- A. glints -------------------------------------------------------------

/// Glint candidates per 256 water pixels. The hard ceiling on how many pixels
/// can sparkle at rest, before the lobe and the dashes. Raising it makes the
/// water busier.
pub(super) const GLINT_DENSITY: u32 = 40;
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
pub(super) const OCT1_TILT: i32 = 60;
/// The fine octave: per-pixel grain tilt, Q0.7.
pub(super) const OCT2_TILT: i32 = 22;
/// The highlight's exponent: higher is a narrower lobe (fewer, harder glints).
pub(super) const GLINT_POWER: i32 = 28;
/// Lobe values under this are no glint at all (the rest is rescaled to 0..1),
/// so a glint is a crisp point, never a faint smear. Raising it thins them.
pub(super) const GLINT_MIN_LOBE: f32 = 0.12;
/// Received light (`Q8.8`, `LightInfo::dom`) under which water never glints,
/// and at which a glint reaches full brightness.
pub(super) const GLINT_MIN_LIGHT: f32 = 8.0;
pub(super) const GLINT_FULL_LIGHT: f32 = 90.0;
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
pub(super) const RAT_GLINT_ENERGY: f32 = 260.0;
pub(super) const RAT_GLINT_RADIUS: f32 = 66.0;
pub(super) const RAT_GLINT_RADIUS_LAB: f32 = 58.0;

// ---- B. lamp mirrors -------------------------------------------------------

/// The mirror image sits this many fixture heights below the lamp on screen.
pub(super) const MIRROR_K: f32 = 1.0;
/// The image's half-width (px): a base plus a share of the fixture's height.
pub(super) const MIRROR_RX: f32 = 3.5;
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
/// How far a row of the lamp image shifts (± px) at rest.
pub(super) const MIRROR_WOBBLE_PX: i32 = 1;
/// Maximum extra horizontal displacement (px) from live rings.
const RIPPLE_SHIFT_PX: f32 = 2.0;

// ---- C. body mirrors -------------------------------------------------------

/// How far (px) a water pixel looks up the screen for a body. Also where the
/// image has faded to nothing.
pub(super) const MIRROR_REACH: usize = 14;
/// The body image's opacity next to the body (d = 1).
pub(super) const BODY_MIRROR: f32 = 0.58;
/// The body image is this much darker than the body (0.30 = 30 %).
pub(super) const BODY_DARKEN: f32 = 0.15;
/// ... and tinted this far toward the water under it.
pub(super) const BODY_TINT: f32 = 0.15;
/// How far (± px) a row of the body image shifts at rest.
pub(super) const BODY_WOBBLE_PX: i32 = 1;
/// Columns the body pass handles (a stack array); wider buffers skip the rest.
const MAX_COLS: usize = 2048;

/// A beam up to 10 px above a waterline can be seen in the water.
const LASER_MIRROR_REACH: i32 = 10;
const LASER_MIRROR: f32 = 0.85;

// Hash salts: one stream per use, so the patterns do not line up.
const GATE_SALT: u32 = 0x6a09_e667;
const NX_SALT: u32 = 0xbb67_ae85;
const NY_SALT: u32 = 0x3c6e_f372;
const MIRROR_SALT: u32 = 0xa54f_f53a;
const BODY_SALT: u32 = 0x510e_527f;
const LASER_SALT: u32 = 0x1f83_d9ab;

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
    strength: f32,
}

impl WaterRipple {
    pub(super) const NONE: Self = Self {
        centre: Vec2::ZERO,
        rx: 0.0,
        ry: 0.0,
        strength: 0.0,
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
                rx: rx * r.scale,
                ry: ry * r.scale,
                strength: 1.0 - r.age / art_puddle::RIPPLE_LIFE,
            };
            n += 1;
        }
    }
    n
}

/// The stage (`fx::strength::WATER`).
pub(super) fn apply(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    ripples: &[WaterRipple],
    lasers: &[Option<LaserSegment>],
) {
    run(grid, canvas, frame, ripples, lasers, strength::WATER);
}

/// [`apply`] at strength `k`, so a test can prove 0 is a byte no-op.
fn run(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    ripples: &[WaterRipple],
    lasers: &[Option<LaserSegment>],
    k: f32,
) {
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
    if full {
        body_mirrors(canvas, frame, ripples, k);
    }
    lamp_mirrors(grid, canvas, frame, ripples, k);
    glints(grid, canvas, frame, ripples, k, full);
    for &laser in lasers.iter().flatten() {
        laser_mirror(canvas, frame.cam, laser, ripples, k);
    }
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

/// A reflection's fixed shift (−1, 0 or +1) for a world row.
#[inline]
fn wobble(wy: i32, salt: u32) -> i32 {
    (hash3(0x5157, wy, salt) % 3) as i32 - 1
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

fn laser_mirror(canvas: &mut Canvas<'_>, cam: Vec2, laser: LaserSegment, ripples: &[WaterRipple], k: f32) {
    let start = laser.start - cam;
    let end = laser.end - cam;
    let (x0, y0) = (start.x.round() as i32, start.y.round() as i32);
    let (dx, dy) = (end.x.round() as i32 - x0, end.y.round() as i32 - y0);
    let steps = dx.abs().max(dy.abs());
    let inv_steps = 1.0 / steps.max(1) as f32;
    let (w, h) = (canvas.w as i32, canvas.h as i32);
    let color = pull_white(crate::game::particle::vfx::LASER_GLOW, 192);
    let peak = (LASER_MIRROR * laser.brightness * k).min(1.0);
    for step in 0..=steps {
        let t = step as f32 * inv_steps;
        let sx = x0 + (dx as f32 * t).round() as i32;
        let sy = y0 + (dy as f32 * t).round() as i32;
        if !(0..w).contains(&sx) {
            continue;
        }
        let waterline = (sy.max(0)..=(sy + LASER_MIRROR_REACH).min(h - 1))
            .find(|&y| is_water(canvas.mat[y as usize * canvas.w + sx as usize]));
        let Some(waterline) = waterline else {
            continue;
        };
        let d = waterline - sy;
        // The surface is half a pixel above its first wet row. At a crossing,
        // keep the reflection just below the direct ray.
        let y = if d == 0 { sy + 1 } else { waterline + d - 1 };
        let world_y = y + cam.y as i32;
        let wet = Vec2::new(sx as f32, y as f32) + cam;
        let x = sx + wobble(world_y, LASER_SALT) + ripple_shift(ripples, wet).round() as i32;
        if !(0..w).contains(&x) || !(0..h).contains(&y) {
            continue;
        }
        let i = y as usize * canvas.w + x as usize;
        if !is_water(canvas.mat[i]) {
            continue;
        }
        let fade = 1.0 - 0.025 * d as f32;
        let glint = hash3(x + cam.x as i32, world_y, LASER_SALT) & 7 == 0;
        let (tint, gain) = if glint {
            ((255, 255, 255), (peak * fade + 0.12 * k).min(1.0))
        } else {
            (color, peak * fade)
        };
        canvas.px[i] = toward(canvas.px[i], tint, gain);
    }
}

// ============================================================================
// C. Body mirrors
// ============================================================================

fn body_mirrors(canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, ripples: &[WaterRipple], k: f32) {
    let stride = canvas.w;
    let (w, h) = (canvas.w.min(MAX_COLS), canvas.h);
    // The row of the nearest creature pixel above, per column.
    let mut last = [i32::MIN / 2; MAX_COLS];
    let cam_y = frame.cam.y as i32;
    let reach = MIRROR_REACH as i32;
    let alpha0 = (BODY_MIRROR * k).min(1.0);
    for y in 0..h {
        let yi = y as i32;
        let shift = wobble(cam_y + yi, BODY_SALT) * BODY_WOBBLE_PX;
        let row = y * stride;
        #[allow(clippy::needless_range_loop)]
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
            if sy < 0 {
                continue;
            }
            let here = canvas.px[i];
            let sample = |sx: i32| {
                if sx >= 0 && sx < w as i32 {
                    let si = sy as usize * stride + sx as usize;
                    if material::is_creature(canvas.mat[si]) {
                        return mix(scale(canvas.px[si], 1.0 - BODY_DARKEN), here, BODY_TINT);
                    }
                }
                here
            };
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let dx = ripple_shift(ripples, p);
            let whole = dx.floor();
            let sx = x as i32 + shift + whole as i32;
            let frac = dx - whole;
            let mut image = sample(sx);
            if frac > 0.0 {
                image = mix(image, sample(sx + 1), frac);
            }
            let fade = (reach - d) as f32 / (reach - 1) as f32;
            canvas.px[i] = mix(here, image, alpha0 * fade);
        }
    }
}

// ============================================================================
// B. Lamp mirrors
// ============================================================================

fn lamp_mirrors(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    ripples: &[WaterRipple],
    k: f32,
) {
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
        let pad = MIRROR_WOBBLE_PX as f32 + RIPPLE_SHIFT_PX + 1.0;
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
            let shift = (wobble(cam_y + py, salt) * MIRROR_WOBBLE_PX) as f32;
            for px in x0..x1 {
                let i = py as usize * canvas.w + px as usize;
                if !is_water(canvas.mat[i]) {
                    continue;
                }
                let p = Vec2::new(px as f32 + 0.5, py as f32 + 0.5);
                let dx = (p.x - cx - shift - ripple_shift(ripples, p)) / rx;
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
fn value_noise(wx: i32, wy: i32, salt: u32) -> i32 {
    let c = OCT1_CELL;
    let (cx, cy) = (wx.div_euclid(c), wy.div_euclid(c));
    let (fx, fy) = (wx.rem_euclid(c), wy.rem_euclid(c));
    let corner = |x: i32, y: i32| (hash3(x, y, salt) & 255) as i32 - 128;
    let (a, b) = (corner(cx, cy), corner(cx + 1, cy));
    let (d, e) = (corner(cx, cy + 1), corner(cx + 1, cy + 1));
    (a * (c - fx) * (c - fy) + b * fx * (c - fy) + d * (c - fx) * fy + e * fx * fy) / (c * c)
}

/// Outward tilt and peak strength from the live rings under canvas point `p`.
fn ring_wave(ripples: &[WaterRipple], p: Vec2) -> (Vec2, f32) {
    if ripples.is_empty() {
        return (Vec2::ZERO, 0.0);
    }
    let mut tilt = Vec2::ZERO;
    let mut strength = 0.0f32;
    for r in ripples {
        if r.rx <= 0.0 || r.ry <= 0.0 {
            continue;
        }
        let d = p - r.centre;
        let (ex, ey) = (d.x / r.rx, d.y / r.ry);
        let q = (ex * ex + ey * ey).sqrt();
        let distance = ((q - 1.0) * (r.rx + r.ry) * 0.5).abs();
        if distance >= RIPPLE_BAND {
            continue;
        }
        let t = 1.0 - distance / RIPPLE_BAND;
        let weight = t * t * (3.0 - 2.0 * t) * r.strength;
        tilt += d * (weight / d.len().max(0.001));
        strength = strength.max(weight);
    }
    (tilt.clamp_box(Vec2::splat(-1.0), Vec2::splat(1.0)), strength)
}

#[inline]
fn ripple_shift(ripples: &[WaterRipple], p: Vec2) -> f32 {
    ring_wave(ripples, p).0.x * RIPPLE_SHIFT_PX
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
    let hsh = hash3(wx, wy, GATE_SALT);
    let gate = hsh & 255;
    if gate >= GLINT_DENSITY.max(RIPPLE_DENSITY) {
        return None;
    }
    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
    let (tilt, ring_strength) = ring_wave(ctx.ripples, p);
    let extra = gate >= GLINT_DENSITY;
    if extra && ring_strength == 0.0 {
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
    let nx = value_noise(wx, wy, NX_SALT) * OCT1_TILT / 128
        + (((hsh >> 16) & 255) as i32 - 128) * OCT2_TILT / 128;
    let ny = value_noise(wx, wy, NY_SALT) * OCT1_TILT / 128
        + (((hsh >> 24) & 255) as i32 - 128) * OCT2_TILT / 128;
    let nx = (nx as f32 + tilt.x * RIPPLE_TILT as f32).clamp(-110.0, 110.0) / 127.0;
    let ny = (ny as f32 + tilt.y * RIPPLE_TILT as f32).clamp(-110.0, 110.0) / 127.0;
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
    let amount = best * ctx.peak * if extra { ring_strength } else { 1.0 };
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

fn glints(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    ripples: &[WaterRipple],
    k: f32,
    full: bool,
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
    use super::{body_mirrors, lamp_mirrors, run, LaserSegment, WaterRipple, MIRROR_REACH};
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
        frame.step8 = super::super::fx::step8(elapsed);
        frame.cam = cam;
        frame.rat = rat.map_or(Vec2::new(-5000.0, -5000.0), |r| r + cam);
        frame.lights = lights;
        let mut canvas = testkit::canvas(pb);
        run(g, &mut canvas, &frame, &[], &[], k);
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
        stage(&mut dim, &grid(6), LightingMode::Full, 1.0, Vec2::ZERO, None, &[], 1.0);
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
        // Under a strong lamp about one water pixel in seven sparkles (the owner
        // asked for glints he can see); never more than one in five.
        assert!(moved.len() * 5 <= water_px, "sparse: {} of {water_px}", moved.len());
        for (x, y) in &moved {
            assert!(*x >= 16, "glint on stone at {x},{y}");
            let (a, b) = (before.pixel_at(*x, *y), after.pixel_at(*x, *y));
            assert!(b.0 >= a.0 && b.1 >= a.1 && b.2 >= a.2, "a glint only brightens");
        }
    }

    #[test]
    fn still_water_is_byte_identical_across_elapsed_times() {
        let g = grid(220);
        let lights = [lamp(Vec2::new(22.0, 6.0))];
        for mode in [LightingMode::Full, LightingMode::Hard] {
            let draw = |elapsed| {
                let mut pb = wading(true);
                stage(&mut pb, &g, mode, elapsed, Vec2::ZERO, None, &lights, 1.0);
                pb
            };
            let first = draw(0.01);
            assert_ne!(wading(true).pixels(), first.pixels());
            assert_eq!(first.pixels(), draw(1.37).pixels(), "{mode:?}");
        }
    }

    #[test]
    fn glints_slide_with_a_one_pixel_camera_move() {
        let g = grid(220);
        let draw = |cam| {
            let mut pb = puddle();
            stage(&mut pb, &g, LightingMode::Full, 0.0, cam, None, &[], 1.0);
            pb
        };
        let first = draw(Vec2::ZERO);
        let next = draw(Vec2::new(1.0, 0.0));
        let mut glints = 0;
        let mut shared = 0;
        let mut adjusted = 0;
        for y in 0..SIZE.1 {
            // Compare the same world pixels past the clipped shore and its dashes.
            for x in 18..SIZE.0 {
                let a = first.pixel_at(x, y);
                let b = next.pixel_at(x - 1, y);
                glints += usize::from(a != WATER || b != WATER);
                if a != WATER && b != WATER {
                    shared += 1;
                    adjusted += usize::from(a != b);
                    assert!(a.0.abs_diff(b.0) <= 16, "small brightness change at {x},{y}");
                }
            }
        }
        assert!(shared * 2 > glints, "most glints persist in world space");
        assert!(adjusted > 0, "the view term still adjusts their brightness");
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
    fn a_live_ripple_distorts_lamp_mirror_rows_as_it_expands() {
        let g = grid(220);
        let level = Level::empty(Theme::Sewer);
        let lights = [lamp(Vec2::new(40.0, 6.0))];
        let mut frame = testkit::frame(&level, LightingMode::Hard);
        frame.lights = &lights;
        let draw = |ripples: &[WaterRipple]| {
            let mut pb = puddle();
            lamp_mirrors(&g, &mut testkit::canvas(&mut pb), &frame, ripples, 1.0);
            pb
        };
        let first = draw(&[]);
        let ripple = WaterRipple {
            centre: Vec2::new(34.0, 22.5),
            rx: 6.5,
            ry: 3.25,
            strength: 0.55,
        };
        let passing = draw(&[ripple]);
        let wider = draw(&[WaterRipple {
            rx: 6.75,
            ry: 3.375,
            ..ripple
        }]);
        let moved = changed(&first, &passing);
        assert!(moved.iter().any(|&(_, y)| y == 22));
        assert!(moved.iter().all(|&(_, y)| (18..=26).contains(&y)));
        assert!(!changed(&passing, &wider).is_empty());
    }

    #[test]
    fn a_live_ripple_distorts_a_body_mirror() {
        let level = Level::empty(Theme::Sewer);
        let frame = testkit::frame(&level, LightingMode::Full);
        let draw = |ripples: &[WaterRipple]| {
            let mut pb = wading(true);
            for x in (10..20).step_by(2) {
                paint(&mut pb, (x, 13, 1, 1), MaterialKind::Fur, true, (0, 255, 0));
            }
            body_mirrors(&mut testkit::canvas(&mut pb), &frame, ripples, 1.0);
            pb
        };
        let first = draw(&[]);
        let passing = draw(&[WaterRipple {
            centre: Vec2::new(8.0, 14.5),
            rx: 7.0,
            ry: 3.5,
            strength: 0.55,
        }]);
        let moved = changed(&first, &passing);
        assert!(moved.iter().any(|&(_, y)| y == 14));
        assert!(moved.iter().all(|&(_, y)| (14..=19).contains(&y)));
    }

    #[test]
    fn laser_mirrors_are_bright_and_follow_the_waterline() {
        let before = wading(false);
        let lasers = [
            // Five pixels above the water in rows 14..34. Only columns 6..26
            // are wet, and a wobble shifts the image by at most one column.
            Some(LaserSegment {
                start: Vec2::new(2.0, 9.0),
                end: Vec2::new(30.0, 9.0),
                brightness: 1.0,
            }),
            // More than 10 px above the puddle. It should have no image.
            Some(LaserSegment {
                start: Vec2::new(10.0, 2.0),
                end: Vec2::new(26.0, 2.0),
                brightness: 1.0,
            }),
        ];
        let level = Level::empty(Theme::Sewer);
        for mode in [LightingMode::Full, LightingMode::Hard] {
            let mut after = wading(false);
            let mut frame = testkit::frame(&level, mode);
            frame.rat = Vec2::new(-5000.0, -5000.0);
            {
                let mut canvas = testkit::canvas(&mut after);
                run(&grid(0), &mut canvas, &frame, &[], &lasers, 1.0);
            }
            let pixels = changed(&before, &after);
            assert!(pixels.len() >= 18, "most of the wet row should reflect");
            for (x, y) in pixels {
                assert!((6..26).contains(&x) && y == 18, "wrote off the mirror at {x},{y}");
                assert!(after.pixel_at(x, y).0 >= 190, "dim reflection at {x},{y}");
            }
        }
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
