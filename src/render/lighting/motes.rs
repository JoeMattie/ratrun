//! W7 — particulate in beams (dust, steam, marine snow) and caustics
//! (contract §4, techniques 10 + 11).
//!
//! Both terms are **analytic**: every position and phase is a pure function of
//! the level seed and `frame.step8` (8 Hz; 4 Hz on Hard), reconstructed the way
//! [`super::ambience`] rebuilds its emitters. There is no particle pool, no
//! rng, no allocation and no wall clock, so a still frame inside one 8 Hz step
//! is bit-identical, and only about one mote per two moves a cell per step.
//!
//! * Motes exist **only inside lit air**: the air channel
//!   (`fx::LightInfo::air`, written by `air.rs`, W2) is the beam mask, so dust
//!   appears in the shafts and nowhere else and can never cross a wall (W2's
//!   scatter already carries visibility).
//! * Caustics are water light on a floor: off for every shipped theme
//!   ([`caustics_here`] returns false for all six). The under-ocean floor flips
//!   one arm of that function and the whole tested path lights up; the tests
//!   below drive it through a thread-local override.

use std::f32::consts::TAU;

use super::engine::{Grid, LayerMask, LightingMode};
use super::fx::{strength, Canvas, FxFrame};
use super::material::{self, MaterialKind};
use crate::detmath::{cos, sin};
use crate::game::level::{LightZoneKind, Theme};
use crate::render::palette::Rgb;

// ---------------------------------------------------------------------------
// Motes
// ---------------------------------------------------------------------------

/// World pixels per mote at Full: a 120×80 view gets 60, a 240×160 one 80.
const MOTE_AREA_PER_MOTE: usize = 240;
/// The contract's band: 30–80 sparse 1 px motes per view.
const MOTE_COUNT_MIN: usize = 30;
const MOTE_COUNT_MAX: usize = 80;
/// Steam wisps per view (Kitchen, Laundry): 2–4 low-alpha puffs.
const STEAM_PUFFS: usize = 4;
/// A mote is only drawn where the air channel reaches this share of the
/// view's brightest air (1/8, at least one Q8.8 level). W2's absolute scale is
/// its own; the ratio is the invariant — dust lives in the body of a beam, not
/// only at its core, and nowhere in the dark.
const AIR_GATE_SHIFT: u32 = 2;
/// Other homes a mote in the dark may try (see `particulate`).
const MOTE_RETRIES: usize = 2;
/// A mote's lift in output levels at the beam gate / at the brightest air,
/// before `fx::strength::MOTES` (1.0): 30..=80 levels.
const MOTE_LIFT_MIN: i32 = 30;
const MOTE_LIFT_SPAN: i32 = 50;
/// Beam brightness buckets: flat steps, never a gradient creeping one level.
const MOTE_REL_STEP: i32 = 85;

// ---------------------------------------------------------------------------
// Caustics
// ---------------------------------------------------------------------------

/// The quarter wave of the sine LUT: `round(256 · sin(kπ/128))`, Q8.8. Written
/// out rather than built from `f64::sin` so the pattern is identical on every
/// platform (no libm, no init, no allocation).
const SIN_QUARTER: [i16; 65] = [
    0, 6, 13, 19, 25, 31, 38, 44, 50, 56, 62, 68, 74, 80, 86, 92, 98, 104, 109, 115, 121, 126,
    132, 137, 142, 147, 152, 157, 162, 167, 172, 177, 181, 185, 190, 194, 198, 202, 206, 209, 213,
    216, 220, 223, 226, 229, 231, 234, 237, 239, 241, 243, 245, 247, 248, 250, 251, 252, 253, 254,
    255, 255, 256, 256, 256,
];

/// Integer wave numbers of the two interfering fronts, in 256ths of a cycle
/// per world pixel: wavelengths ≈ 21 px and 21 px along (1, 0.5) and
/// (-0.6, 1). Both are integers, so the pattern is periodic in world space
/// (64 px in x) and rides the floor as the camera pans.
const CAUSTIC_AX: i32 = 12;
const CAUSTIC_AY: i32 = 6;
const CAUSTIC_BX: i32 = -8;
const CAUSTIC_BY: i32 = 14;
/// Phase advance per 8 Hz step, in 256ths of a cycle (≈ 0.63 cycles a second),
/// kept for the legacy stepped test override.
const CAUSTIC_PHASE_STEP: u32 = 20;
/// Mean downward drift speed of Outfall marine snow, in world pixels per second
/// (individual flakes stagger from 5.0..=7.0 px/s with sub-pixel start phases
/// so only a few flakes step 1 px on any 60 Hz frame).
const MARINE_SNOW_BASE_PX_S: f32 = 5.0;
const MARINE_SNOW_STEP_PX_S: f32 = 0.5;
const MARINE_SNOW_SWAY_PX_S: f32 = 1.5;
/// The brightest a caustic lifts a floor pixel, in output levels before
/// `fx::strength::CAUSTICS` (0.5), and the flat step it is quantised to (the
/// edges of a slack-water pattern must not repaint cells one level at a time).
const CAUSTIC_GAIN: i32 = 30;
const CAUSTIC_LEVEL_STEP: i32 = 3;
/// The cool cast a beam's own colour is pulled toward: water light reads cold
/// even under a warm lamp.
const CAUSTIC_COOL: Rgb = (120, 200, 255);
/// Cool pale cyan-blue cast for Outfall marine snow.
const MARINE_SNOW_COOL: Rgb = (176, 226, 248);

// ---------------------------------------------------------------------------
// Outfall ambient drifting motes (always-on, all lighting tiers)
// ---------------------------------------------------------------------------

/// World pixels per ambient Outfall mote at `Full`: a 120×80 view floats 56
/// motes (inside the 40–70 band).
const AMBIENT_AREA_PER_MOTE: usize = 170;
const AMBIENT_COUNT_MIN: usize = 40;
const AMBIENT_COUNT_MAX: usize = 70;
/// Reduced tier counts so lower lighting tiers stay light on terminal churn.
const AMBIENT_HARD_NUM: usize = 7;
const AMBIENT_HARD_DEN: usize = 10;
const AMBIENT_HARD_MIN: usize = 24;
const AMBIENT_COARSE_DIV: usize = 2;
const AMBIENT_COARSE_MIN: usize = 18;
const AMBIENT_COMPACT_NUM: usize = 2;
const AMBIENT_COMPACT_DEN: usize = 5;
const AMBIENT_COMPACT_MIN: usize = 14;
/// Virtual world-space seeding domain before wrapping into the view.
const AMBIENT_DOMAIN_PX: f32 = 1024.0;
/// Base random-direction drift speed (px/s) and gentle downward settle (px/s).
const AMBIENT_DRIFT_MIN_PX_S: f32 = 1.6;
const AMBIENT_DRIFT_SPAN_PX_S: f32 = 1.8;
const AMBIENT_SETTLE_MIN_PX_S: f32 = 0.7;
const AMBIENT_SETTLE_SPAN_PX_S: f32 = 0.7;
/// Per-mote two-octave smooth wandering wave velocity budgets (px/s), combining
/// with base drift + settle for ~3–8 px/s organic omnidirectional wander.
const AMBIENT_WAVE1_SPEED_MIN: f32 = 2.0;
const AMBIENT_WAVE1_SPEED_SPAN: f32 = 1.5;
const AMBIENT_WAVE2_SPEED_MIN: f32 = 0.9;
const AMBIENT_WAVE2_SPEED_SPAN: f32 = 0.9;
/// Brightness lifts (output RGB levels): faint self-lit floor in dim water,
/// boosted when a mote floats through a lamp or headlamp beam.
const AMBIENT_DIM_LIFT_MIN: i32 = 20;
const AMBIENT_DIM_LIFT_SPAN: i32 = 14;
const AMBIENT_LIT_BONUS: i32 = 44;
/// Companion pixel brightness for 2 px motes (% of core lift).
const AMBIENT_SECOND_PX_PCT: i32 = 55;
/// Pale cool sea-mist colour for ambient drifting motes in dim water.
const AMBIENT_MOTE_COOL: Rgb = (196, 234, 244);

/// The per-stage strength, Q8. The game always uses [`Gains::live`]; the tests
/// zero it to prove that strength 0 draws nothing.
#[derive(Clone, Copy)]
struct Gains {
    motes: i32,
    caustics: i32,
}

impl Gains {
    fn live() -> Self {
        Self {
            motes: (strength::MOTES * 256.0) as i32,
            caustics: (strength::CAUSTICS * 256.0) as i32,
        }
    }

    #[cfg(test)]
    const ZERO: Self = Self {
        motes: 0,
        caustics: 0,
    };
}

/// What floats in this theme's air.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavour {
    /// Fine grit in a wet shaft (the Sewer, the Printworks, road dust).
    Dust,
    /// Rising wisps off warm floors (the Kitchen, the Laundry).
    Steam,
    /// Sparse cold motes (the Lab).
    Cold,
    /// Sparse, cool, downward-drifting marine snow (The Outfall).
    MarineSnow,
}

const fn flavour(theme: Theme) -> Flavour {
    match theme {
        Theme::Kitchen | Theme::Laundry => Flavour::Steam,
        Theme::Lab => Flavour::Cold,
        Theme::Outfall => Flavour::MarineSnow,
        Theme::Sewer | Theme::Printworks | Theme::Trailer => Flavour::Dust,
    }
}

/// Does this theme/zone show caustics? On for The Outfall outside Blackout
/// zones; dry everywhere else.
pub(super) fn caustics_here(theme: Theme, zone: Option<LightZoneKind>) -> bool {
    let water = match theme {
        Theme::Outfall => true,
        Theme::Sewer
        | Theme::Kitchen
        | Theme::Lab
        | Theme::Laundry
        | Theme::Printworks
        | Theme::Trailer => false,
    };
    #[cfg(test)]
    let water = water || caustics_override();
    // A blacked-out zone has its lamps dead: no water light there either.
    water && !matches!(zone, Some(LightZoneKind::Blackout))
}

// Test-only switch: pretend the level asked for caustics (the ocean arm).
#[cfg(test)]
thread_local! {
    static CAUSTICS_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_caustics_override(on: bool) {
    CAUSTICS_OVERRIDE.with(|c| c.set(on));
}

#[cfg(test)]
fn caustics_override() -> bool {
    CAUSTICS_OVERRIDE.with(|c| c.get())
}

/// Last pixel stage: motes inside lit air, caustics where flagged
/// (`fx::strength::MOTES`, `CAUSTICS`). `elapsed` drives continuous smooth
/// motion for Outfall caustics and marine snow; other floors keep their stepped
/// `frame.step8` clock.
pub(super) fn draw(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>, elapsed: f32) {
    draw_with(grid, canvas, frame, elapsed, Gains::live());
}

fn draw_with(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    elapsed: f32,
    gains: Gains,
) {
    // §5: Full and Hard run motes/caustics; Coarse/Compact skip them.
    // Non-Outfall floors keep 8 Hz at Full / 4 Hz at Hard; Outfall uses
    // continuous `elapsed` for smooth water light and falling marine snow.
    let clock = match frame.mode {
        LightingMode::Full => frame.step8,
        LightingMode::Hard => frame.step8 / 2,
        _ => return,
    };
    if canvas.w == 0 || canvas.h == 0 {
        return;
    }
    caustics(grid, canvas, frame, clock, elapsed, gains);
    if gains.motes > 0 {
        particulate(grid, canvas, frame, clock, elapsed, gains);
    }
}

fn particulate(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    clock: u32,
    elapsed: f32,
    gains: Gains,
) {
    let Some(beam) = Beam::measure(grid) else {
        return;
    };
    let flavour = flavour(frame.theme);
    let count = mote_count(canvas.w, canvas.h, frame.mode, flavour);
    let seed = (frame.level.seed as u32) ^ (frame.level.seed >> 32) as u32;
    let field = (canvas.w * canvas.h) as u32;
    let cold = flavour == Flavour::Cold;
    let cam = (frame.cam.x as i64, frame.cam.y as i64);
    let t = elapsed.max(0.0);
    for i in 0..count {
        // Ambience's hashing, cloned: the mote's whole life comes from these
        // bits, so two runs (or two frames at the same time) agree exactly.
        let h0 = hash(seed ^ (i as u32).wrapping_mul(0x9e37_79b9));
        let h1 = hash(h0 ^ 0x5bf0_3635);
        let cell = (h0 % field) as i32;
        let (base_x, base_y) = (cell % canvas.w as i32, cell / canvas.w as i32);
        let (dx, dy) = match flavour {
            // Marine snow drifts smoothly downward at ~6 px/s (5.0..=7.0 px/s)
            // from continuous `elapsed`, with hash-staggered sub-pixel phases
            // so flakes step 1 px smoothly across frames instead of jumping on
            // an 8 Hz boundary.
            Flavour::MarineSnow => {
                let sway_dir = match h1 % 5 {
                    0 => -1i64,
                    1 => 1i64,
                    _ => 0i64,
                };
                let speed_y =
                    MARINE_SNOW_BASE_PX_S + ((h1 >> 16) % 5) as f32 * MARINE_SNOW_STEP_PX_S;
                let phase_y = ((h1 >> 8) & 255) as f32 / 256.0;
                let phase_x = ((h0 >> 8) & 255) as f32 / 256.0;
                let dy = (t * speed_y + phase_y).floor() as i64;
                let dx = sway_dir * (t * MARINE_SNOW_SWAY_PX_S + phase_x).floor() as i64;
                (dx, dy)
            }
            _ => {
                let (vx, mut vy) = ((h1 % 3) as i32 - 1, ((h1 / 3) % 3) as i32 - 1);
                if flavour == Flavour::Steam {
                    // A wisp rises whether or not its hash wanted to drift.
                    vy = -1;
                } else if vx == 0 && vy == 0 {
                    vy = 1;
                }
                // Each mote steps on every other 8 Hz step, so about half the field
                // moves a pixel at a time: 30 moves ≈ 60 changed cells, the budget.
                let moves = (i64::from(clock) + 1 + i64::from(i as u32 & 1)) / 2;
                (i64::from(vx) * moves, i64::from(vy) * moves)
            }
        };
        // Anchored to the world (tiled at the view size), so the dust/snow rides
        // the floor under a moving camera instead of sticking to the screen.
        let x = (i64::from(base_x) + dx - cam.0).rem_euclid(canvas.w as i64) as i32;
        let y = (i64::from(base_y) + dy - cam.1).rem_euclid(canvas.h as i64) as i32;
        // A mote in the dark tries its other homes (fixed per mote, so it
        // is the same mote every step): most of the field then floats where
        // there is a beam to show it, and the count stays the budget.
        let mut found = beam.at(grid, x as usize, y as usize).map(|hit| (x, y, hit));
        let mut hr = h1;
        for _ in 0..MOTE_RETRIES {
            if found.is_some() {
                break;
            }
            hr = hash(hr ^ 0x2545_f491);
            let cell = (hr % field) as i32;
            let rx = (i64::from(cell % canvas.w as i32) + dx - cam.0)
                .rem_euclid(canvas.w as i64) as i32;
            let ry = (i64::from(cell / canvas.w as i32) + dy - cam.1)
                .rem_euclid(canvas.h as i64) as i32;
            found = beam.at(grid, rx as usize, ry as usize).map(|hit| (rx, ry, hit));
        }
        let Some((x, y, (rel, tint))) = found else {
            continue;
        };
        let lift = (lift_q8(rel, cold) * gains.motes) >> 8;
        if lift <= 0 {
            continue;
        }
        let tint = if flavour == Flavour::MarineSnow {
            (
                ((u16::from(tint.0) + u16::from(MARINE_SNOW_COOL.0)) / 2) as u8,
                ((u16::from(tint.1) + u16::from(MARINE_SNOW_COOL.1)) / 2) as u8,
                ((u16::from(tint.2) + u16::from(MARINE_SNOW_COOL.2)) / 2) as u8,
            )
        } else {
            tint
        };
        if flavour == Flavour::Steam {
            // A soft wisp: a core with two dimmer companions leaning whichever
            // way this mote's hash points.
            let side = if h1 & 1 == 0 { 1 } else { -1 };
            stamp(canvas, x, y, lift, tint);
            stamp(canvas, x + side, y - 1, lift * 2 / 3, tint);
            stamp(canvas, x, y + 1, lift / 2, tint);
        } else {
            stamp(canvas, x, y, lift, tint);
        }
    }
}

/// The lit-air mask of one frame: the view's brightest air and the gate a mote
/// must reach.
struct Beam {
    peak: u16,
    gate: u16,
}

impl Beam {
    /// `None` when this view has no lit air at all (every mote is then in the
    /// dark, and the whole pass is free).
    fn measure(grid: &Grid) -> Option<Self> {
        let mut peak = 0u16;
        for info in grid.info.iter().take(grid.w * grid.h) {
            peak = peak.max(info.air[0]).max(info.air[1]).max(info.air[2]);
        }
        if peak == 0 {
            return None;
        }
        Some(Self {
            peak,
            gate: (peak >> AIR_GATE_SHIFT).max(1),
        })
    }

    /// The beam's brightness bucket (0..=255, flat-stepped) and its colour at
    /// one pixel, or `None` where the air is too thin for a mote.
    fn at(&self, grid: &Grid, px: usize, py: usize) -> Option<(i32, Rgb)> {
        let info = grid.info.get(grid.sample_index(px, py))?;
        let air = info.air[0].max(info.air[1]).max(info.air[2]);
        if air < self.gate {
            return None;
        }
        let rel = ((u32::from(air) * 255) / u32::from(self.peak)).min(255) as i32;
        let tint = if info.dom == 0 {
            (210, 214, 230)
        } else {
            info.dom_rgb
        };
        Some(((rel / MOTE_REL_STEP) * MOTE_REL_STEP, tint))
    }
}

/// A mote's lift in output levels before the strength scale, Q8.
#[inline]
fn lift_q8(rel: i32, cold: bool) -> i32 {
    let base = MOTE_LIFT_MIN + rel * MOTE_LIFT_SPAN / 255;
    if cold {
        base * 3 / 4
    } else {
        base
    }
}

/// Add `amount` levels toward `tint` (at least half white, so a cold beam's
/// mote is still a spark and not a blue hole).
#[inline]
fn stamp(canvas: &mut Canvas<'_>, x: i32, y: i32, amount: i32, tint: Rgb) {
    if amount <= 0 || x < 0 || y < 0 || x as usize >= canvas.w || y as usize >= canvas.h {
        return;
    }
    let i = y as usize * canvas.w + x as usize;
    if material::is_creature(canvas.mat_at(i)) {
        return;
    }
    let pixel = &mut canvas.px[i];
    *pixel = (
        pixel.0.saturating_add(channel(amount, tint.0)),
        pixel.1.saturating_add(channel(amount, tint.1)),
        pixel.2.saturating_add(channel(amount, tint.2)),
    );
}

/// One channel's share of an additive amount: half the lift is white, the rest
/// carries the beam's colour.
#[inline]
fn channel(amount: i32, tint: u8) -> u8 {
    (((amount * (128 + i32::from(tint) / 2)) >> 8).clamp(0, 255)) as u8
}

/// [`SIN_QUARTER`] looked up over the full turn, Q8.8 (`-256..=256`).
#[inline]
pub(super) fn sin_q8(index: u8) -> i32 {
    let i = usize::from(index);
    let (i, sign) = if i < 128 { (i, 1) } else { (i - 128, -1) };
    let v = if i <= 64 {
        SIN_QUARTER[i]
    } else {
        SIN_QUARTER[128 - i]
    };
    sign * i32::from(v)
}

/// The two-wave interference of technique 10 at a world pixel, Q8.8
/// (`0..=256`): the product of two `1 - |sin|` fronts, squared, so the crests
/// are thin and the troughs broad (the plates' caustic cells).
#[inline]
fn caustics_q8(wx: i32, wy: i32, phase: i32) -> i32 {
    let i1 = (wx * CAUSTIC_AX + wy * CAUSTIC_AY + phase) & 255;
    let i2 = (wx * CAUSTIC_BX + wy * CAUSTIC_BY - phase) & 255;
    let f1 = 256 - sin_q8(i1 as u8).abs();
    let f2 = 256 - sin_q8(i2 as u8).abs();
    let fil = (f1 * f2) >> 8;
    (fil * fil) >> 8
}

fn caustics(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    clock: u32,
    elapsed: f32,
    gains: Gains,
) {
    if gains.caustics <= 0 {
        return;
    }
    if frame.theme == Theme::Outfall {
        super::underwater::apply_fx_caustics(grid, canvas, frame, elapsed, gains.caustics);
        return;
    }
    // A view-wide flag (or any flagged zone in this level) gates the pass; the
    // per-sample `Surface::zone` then decides pixel by pixel.
    let whole_view = caustics_here(frame.theme, None);
    if !whole_view
        && !frame
            .level
            .objects
            .light_zones
            .iter()
            .any(|z| caustics_here(frame.theme, Some(z.kind)))
    {
        return;
    }
    let phase = ((u64::from(clock) * u64::from(CAUSTIC_PHASE_STEP)) & 255) as i32;
    let cam = (frame.cam.x.round() as i32, frame.cam.y.round() as i32);
    let (w, h) = (canvas.w, canvas.h);
    for py in 0..h {
        let wy = cam.1 + py as i32;
        for px in 0..w {
            let wx = cam.0 + px as i32;
            let i = grid.sample_index(px, py);
            let Some(surface) = grid.surfaces.get(i) else {
                continue;
            };
            // Water light lands on the floor, never on a wall face.
            if surface.layer & LayerMask::FLOOR.0 == 0 {
                continue;
            }
            let Some(info) = grid.info.get(i) else {
                continue;
            };
            // Caustics need the light that makes them: a dark floor is dry.
            if info.dom == 0 {
                continue;
            }
            if !caustics_here(frame.theme, surface.zone) {
                continue;
            }
            let index = py * w + px;
            let byte = canvas.mat_at(index);
            let emissive = material::kind(byte) == material::MaterialKind::Emissive;
            if material::is_creature(byte) || emissive {
                continue;
            }
            let dom_att = i32::from(info.dom).min(128);
            let level = (caustics_q8(wx, wy, phase) * CAUSTIC_GAIN) >> 8;
            let level = (((level * gains.caustics) >> 8) * dom_att) >> 7;
            let level = level / CAUSTIC_LEVEL_STEP * CAUSTIC_LEVEL_STEP;
            if level <= 0 {
                continue;
            }
            let tint = (
                ((u16::from(info.dom_rgb.0) + u16::from(CAUSTIC_COOL.0)) / 2) as u8,
                ((u16::from(info.dom_rgb.1) + u16::from(CAUSTIC_COOL.1)) / 2) as u8,
                ((u16::from(info.dom_rgb.2) + u16::from(CAUSTIC_COOL.2)) / 2) as u8,
            );
            let pixel = &mut canvas.px[index];
            *pixel = (
                pixel.0.saturating_add(channel(level, tint.0)),
                pixel.1.saturating_add(channel(level, tint.1)),
                pixel.2.saturating_add(channel(level, tint.2)),
            );
        }
    }
}

/// How many elements a view floats: the contract's 30–80 motes (half on Hard),
/// fewer cold ones in the Lab, and 2–4 steam wisps in the warm floors.
fn mote_count(w: usize, h: usize, mode: LightingMode, flavour: Flavour) -> usize {
    let base = (w * h / MOTE_AREA_PER_MOTE).clamp(MOTE_COUNT_MIN, MOTE_COUNT_MAX);
    match flavour {
        Flavour::Steam => STEAM_PUFFS,
        Flavour::Cold => (base * 3 / 5).max(MOTE_COUNT_MIN),
        Flavour::MarineSnow => match mode {
            LightingMode::Hard => (base * 3 / 10).max(MOTE_COUNT_MIN / 2),
            _ => (base * 3 / 5).max(MOTE_COUNT_MIN),
        },
        Flavour::Dust => match mode {
            LightingMode::Hard => (base / 2).max(MOTE_COUNT_MIN / 2),
            _ => base,
        },
    }
}

/// `ambience.rs`'s integer hash, cloned: the effects stage must not depend on
/// the simulation's rng and must not advance a pool.
fn hash(mut seed: u32) -> u32 {
    seed ^= seed >> 16;
    seed = seed.wrapping_mul(0x7feb_352d);
    seed ^ (seed >> 15)
}

/// How many always-on ambient drifting motes The Outfall floats at this view
/// size and lighting tier (`40..=70` at `Full`, reduced on `Hard`/`Coarse`/
/// `Compact`).
pub(super) fn ambient_mote_count(w: usize, h: usize, mode: LightingMode) -> usize {
    if w == 0 || h == 0 {
        return 0;
    }
    let base = (w * h / AMBIENT_AREA_PER_MOTE).clamp(AMBIENT_COUNT_MIN, AMBIENT_COUNT_MAX);
    match mode {
        LightingMode::Full => base,
        LightingMode::Hard => (base * AMBIENT_HARD_NUM / AMBIENT_HARD_DEN)
            .max(AMBIENT_HARD_MIN)
            .min(base),
        LightingMode::Coarse => (base / AMBIENT_COARSE_DIV)
            .max(AMBIENT_COARSE_MIN)
            .min(base),
        LightingMode::Compact => (base * AMBIENT_COMPACT_NUM / AMBIENT_COMPACT_DEN)
            .max(AMBIENT_COMPACT_MIN)
            .min(base),
        LightingMode::Off => 0,
    }
}

/// Analytic world-space position `(wx, wy)` of one ambient Outfall mote from
/// `(seed, elapsed, slot)` plus its secondary hash words `(h1, h2)` for size
/// and brightness variation. Continuous in `elapsed`, wandering in all
/// directions at ~3–8 px/s with a slight downward settle.
#[inline]
fn ambient_mote_state(seed: u32, elapsed: f32, slot: usize) -> (f32, f32, u32, u32) {
    let h0 = hash(seed ^ (slot as u32).wrapping_mul(0x9e37_79b9) ^ 0x4f55_5446);
    let h1 = hash(h0 ^ 0x5bf0_3635);
    let h2 = hash(h1 ^ 0x2545_f491);
    let t = elapsed.max(0.0);

    let x0 = (h0 & 0xffff) as f32 * (AMBIENT_DOMAIN_PX / 65_536.0);
    let y0 = ((h0 >> 16) & 0xffff) as f32 * (AMBIENT_DOMAIN_PX / 65_536.0);

    let angle = (h1 & 0x3ff) as f32 * (TAU / 1024.0);
    let drift_spd =
        AMBIENT_DRIFT_MIN_PX_S + (((h1 >> 10) & 255) as f32 / 255.0) * AMBIENT_DRIFT_SPAN_PX_S;
    let settle =
        AMBIENT_SETTLE_MIN_PX_S + (((h1 >> 18) & 63) as f32 / 63.0) * AMBIENT_SETTLE_SPAN_PX_S;
    let vx = cos(angle) * drift_spd;
    let vy = sin(angle) * drift_spd + settle;

    // Two incommensurate smooth sine waves per axis act as a band-limited
    // smooth noise field per mote with strictly bounded drift velocity.
    let w1x = 0.34 + ((h1 >> 24) & 31) as f32 * (0.24 / 31.0);
    let w2x = 0.78 + (h2 & 31) as f32 * (0.42 / 31.0);
    let w1y = 0.31 + ((h2 >> 5) & 31) as f32 * (0.26 / 31.0);
    let w2y = 0.73 + ((h2 >> 10) & 31) as f32 * (0.45 / 31.0);

    let p1x = ((h2 >> 15) & 31) as f32 * (TAU / 32.0);
    let p2x = ((h2 >> 20) & 31) as f32 * (TAU / 32.0);
    let p1y = ((h2 >> 25) & 31) as f32 * (TAU / 32.0);
    let p2y = ((h0 >> 12) & 31) as f32 * (TAU / 32.0);

    let v1x = AMBIENT_WAVE1_SPEED_MIN + (((h0 >> 4) & 15) as f32 / 15.0) * AMBIENT_WAVE1_SPEED_SPAN;
    let v2x = AMBIENT_WAVE2_SPEED_MIN + (((h0 >> 8) & 15) as f32 / 15.0) * AMBIENT_WAVE2_SPEED_SPAN;
    let v1y = AMBIENT_WAVE1_SPEED_MIN + (((h1 >> 4) & 15) as f32 / 15.0) * AMBIENT_WAVE1_SPEED_SPAN;
    let v2y = AMBIENT_WAVE2_SPEED_MIN + (((h1 >> 8) & 15) as f32 / 15.0) * AMBIENT_WAVE2_SPEED_SPAN;

    let wx = x0
        + vx * t
        + (v1x / w1x) * sin((t * w1x + p1x).rem_euclid(TAU))
        + (v2x / w2x) * sin((t * w2x + p2x).rem_euclid(TAU));
    let wy = y0
        + vy * t
        + (v1y / w1y) * sin((t * w1y + p1y).rem_euclid(TAU))
        + (v2y / w2y) * sin((t * w2y + p2y).rem_euclid(TAU));
    (wx, wy, h1, h2)
}

/// Analytic world-space coordinates `(wx, wy)` of one ambient Outfall mote.
#[cfg(test)]
fn ambient_mote_world_pos(seed: u32, elapsed: f32, slot: usize) -> (f32, f32) {
    let (wx, wy, _, _) = ambient_mote_state(seed, elapsed, slot);
    (wx, wy)
}

/// Always-on ambient drifting particulate layer for The Outfall.
///
/// Analytic from `(level.seed, elapsed, slot)` with zero allocation and zero
/// rng; anchored to world space for proper camera parallax; visible faintly in
/// dim water and brighter inside lamps and the headlamp beam. Runs at every
/// live lighting tier (`Full`, `Hard`, `Coarse`, `Compact`) and is a no-op on
/// every other floor.
pub(super) fn draw_outfall_ambient(
    grid: &Grid,
    canvas: &mut Canvas<'_>,
    frame: &FxFrame<'_>,
    elapsed: f32,
) {
    if frame.theme != Theme::Outfall || canvas.w == 0 || canvas.h == 0 {
        return;
    }
    let count = ambient_mote_count(canvas.w, canvas.h, frame.mode);
    if count == 0 {
        return;
    }
    let seed = (frame.level.seed as u32) ^ (frame.level.seed >> 32) as u32;
    let cam_x = frame.cam.x.round() as i64;
    let cam_y = frame.cam.y.round() as i64;
    let w_i = canvas.w as i64;
    let h_i = canvas.h as i64;
    let grid_ok = grid.stride > 0 && grid.w > 0 && grid.h > 0;
    let allow_2px = matches!(frame.mode, LightingMode::Full | LightingMode::Hard);

    for slot in 0..count {
        let (wx, wy, h1, h2) = ambient_mote_state(seed, elapsed, slot);
        let sx = (wx.floor() as i64 - cam_x).rem_euclid(w_i) as i32;
        let sy = (wy.floor() as i64 - cam_y).rem_euclid(h_i) as i32;

        let mut lit = 0i32;
        let mut dom_rgb: Option<Rgb> = None;
        if grid_ok {
            let si = grid.sample_index(sx as usize, sy as usize);
            if grid
                .surfaces
                .get(si)
                .is_some_and(|surface| surface.layer & LayerMask::FLOOR.0 == 0)
            {
                continue;
            }
            if let Some(sample) = grid.samples.get(si) {
                let mul_peak = i32::from(sample.mul[0].max(sample.mul[1]).max(sample.mul[2]));
                lit = ((mul_peak - 112).clamp(0, 220) * 256) / 220;
            }
            if let Some(info) = grid.info.get(si) {
                let dom_lit = (i32::from(info.dom).min(360) * 256) / 360;
                let air_peak = i32::from(info.air[0].max(info.air[1]).max(info.air[2]));
                let air_lit = (air_peak.min(900) * 256) / 900;
                lit = lit.max(dom_lit).max(air_lit);
                if info.dom > 0 {
                    dom_rgb = Some(info.dom_rgb);
                }
            }
        }

        let dim_lift =
            AMBIENT_DIM_LIFT_MIN + (i32::from((h1 >> 29) as u8) * AMBIENT_DIM_LIFT_SPAN) / 7;
        let lift = dim_lift + (lit * AMBIENT_LIT_BONUS) / 256;
        let tint = match dom_rgb {
            Some(rgb) if lit > 40 => (
                ((u16::from(AMBIENT_MOTE_COOL.0) * 2 + u16::from(rgb.0)) / 3) as u8,
                ((u16::from(AMBIENT_MOTE_COOL.1) * 2 + u16::from(rgb.1)) / 3) as u8,
                ((u16::from(AMBIENT_MOTE_COOL.2) * 2 + u16::from(rgb.2)) / 3) as u8,
            ),
            _ => AMBIENT_MOTE_COOL,
        };

        stamp_ambient(canvas, sx, sy, lift, tint);
        if allow_2px && (h2 & 3) == 0 {
            let (dx, dy) = if (h2 & 4) == 0 { (1, 0) } else { (0, 1) };
            let nx = sx + dx;
            let ny = sy + dy;
            if nx >= 0 && ny >= 0 && (nx as usize) < canvas.w && (ny as usize) < canvas.h {
                let on_floor = !grid_ok
                    || grid
                        .surfaces
                        .get(grid.sample_index(nx as usize, ny as usize))
                        .is_none_or(|s| s.layer & LayerMask::FLOOR.0 != 0);
                if on_floor {
                    stamp_ambient(canvas, nx, ny, lift * AMBIENT_SECOND_PX_PCT / 100, tint);
                }
            }
        }
    }
}

/// Additive speck stamp for ambient Outfall motes: skips creatures and emissive
/// pixels so bodies and lamps stay crisp.
#[inline]
fn stamp_ambient(canvas: &mut Canvas<'_>, x: i32, y: i32, amount: i32, tint: Rgb) {
    if amount <= 0 || x < 0 || y < 0 || x as usize >= canvas.w || y as usize >= canvas.h {
        return;
    }
    let i = y as usize * canvas.w + x as usize;
    let mat = canvas.mat_at(i);
    if material::is_creature(mat) || material::kind(mat) == MaterialKind::Emissive {
        return;
    }
    let pixel = &mut canvas.px[i];
    *pixel = (
        pixel.0.saturating_add(channel(amount, tint.0)),
        pixel.1.saturating_add(channel(amount, tint.1)),
        pixel.2.saturating_add(channel(amount, tint.2)),
    );
}

#[cfg(test)]
mod tests {
    use super::super::engine::LightingMode;
    use super::super::fx::{step8, testkit, LightInfo};
    use super::{
        ambient_mote_count, ambient_mote_world_pos, caustics_here, caustics_q8,
        draw_outfall_ambient, draw_with, mote_count, set_caustics_override, Flavour, FxFrame,
        Gains, Grid, LayerMask, LightZoneKind, Rgb, Theme, AMBIENT_COUNT_MAX, AMBIENT_COUNT_MIN,
        MOTE_COUNT_MAX, MOTE_COUNT_MIN,
    };
    use crate::game::level::Level;
    use crate::math::Vec2;
    use crate::render::framebuffer::PixelBuffer;
    use crate::term::ColorDepth;

    fn frame<'a>(level: &'a Level, mode: LightingMode, elapsed: f32) -> FxFrame<'a> {
        FxFrame {
            mode,
            depth: ColorDepth::Truecolor,
            theme: level.theme,
            level,
            cam: Vec2::ZERO,
            elapsed,
            step8: step8(elapsed),
            rat: Vec2::new(32.0, 24.0),
            lights: &[],
            ambient: [256, 256, 256],
        }
    }

    fn level(theme: Theme) -> Level {
        Level::empty(theme)
    }

    /// A lit floor buffer of `w × h` and its (still black) canvas.
    fn run(grid: &Grid, frame: &FxFrame<'_>, gains: Gains, w: usize, h: usize) -> Vec<Rgb> {
        let mut pb = PixelBuffer::new(w, h);
        pb.set_pen(0);
        pb.clear((85, 71, 78));
        let mut canvas = testkit::canvas(&mut pb);
        draw_with(grid, &mut canvas, frame, frame.elapsed, gains);
        pb.pixels().to_vec()
    }

    fn changed(grid: &Grid, frame: &FxFrame<'_>, w: usize, h: usize) -> Vec<usize> {
        let before = vec![(85, 71, 78); w * h];
        let after = run(grid, frame, Gains::live(), w, h);
        (0..w * h).filter(|&i| after[i] != before[i]).collect()
    }

    #[test]
    fn motes_draw_only_inside_lit_air() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (60, 40);
        let f = frame(&lvl, LightingMode::Full, 1.0);
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        // The beam covers the left half of the view; the right half is dark.
        for y in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    grid.info[y * w + x].air = [900, 720, 480];
                    grid.info[y * w + x].dom = 512;
                    grid.info[y * w + x].dom_rgb = (255, 210, 140);
                }
            }
        }
        let hit = changed(&grid, &f, w, h);
        assert!(!hit.is_empty(), "motes must appear in a lit beam");
        for i in hit {
            assert!(
                i % w < w / 2,
                "a mote outside the lit air at pixel {}",
                i % w
            );
        }
        // And nothing at all in a view with no light in the air.
        let dark = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        assert!(changed(&dark, &f, w, h).is_empty(), "no air, no motes");
    }

    #[test]
    fn a_still_view_is_frozen_between_eight_hz_steps_and_moves_on_the_next() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (120, 80);
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            *info = LightInfo {
                air: [1200, 1000, 800],
                dom: 700,
                dom_rgb: (240, 220, 190),
                ..LightInfo::default()
            };
        }
        // 1.000 s and 1.100 s are the same step (8); 1.125 s is the next one.
        let a = frame(&lvl, LightingMode::Full, 1.000);
        let b = frame(&lvl, LightingMode::Full, 1.100);
        let c = frame(&lvl, LightingMode::Full, 1.125);
        assert_eq!(a.step8, b.step8);
        assert_ne!(a.step8, c.step8);
        let first = run(&grid, &a, Gains::live(), w, h);
        assert_eq!(first, run(&grid, &b, Gains::live(), w, h), "held on the step");
        assert_ne!(first, run(&grid, &c, Gains::live(), w, h), "moved on the next");
    }

    #[test]
    fn a_step_moves_about_one_mote_per_cell_of_budget() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (120, 80);
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            info.air = [1500, 1500, 1500];
            info.dom = 800;
            info.dom_rgb = (230, 230, 230);
        }
        let a = run(&grid, &frame(&lvl, LightingMode::Full, 2.0), Gains::live(), w, h);
        let b = run(&grid, &frame(&lvl, LightingMode::Full, 2.125), Gains::live(), w, h);
        let moved = (0..w * h).filter(|&i| a[i] != b[i]).count();
        assert!(moved > 0, "the step must move something");
        assert!(
            moved <= 64,
            "{moved} changed cells in one step, over the 60-cell budget"
        );
        // Nothing above the strength band either: a mote is a spark, not a lamp.
        for pixel in &b {
            let d = i32::from(pixel.0) - i32::from(85u8);
            assert!(d <= 80, "a mote lifted a pixel by {d} levels");
        }
    }

    #[test]
    fn mote_counts_stay_in_the_contract_band() {
        for (w, h) in [(40, 30), (60, 40), (120, 80), (200, 120), (240, 160)] {
            let full = mote_count(w, h, LightingMode::Full, Flavour::Dust);
            assert!(
                (MOTE_COUNT_MIN..=MOTE_COUNT_MAX).contains(&full),
                "{w}x{h} floats {full} motes"
            );
            let hard = mote_count(w, h, LightingMode::Hard, Flavour::Dust);
            assert!(hard <= full && hard >= 12, "Hard is sparser, not empty");
            let cold = mote_count(w, h, LightingMode::Full, Flavour::Cold);
            assert!((MOTE_COUNT_MIN..=MOTE_COUNT_MAX).contains(&cold));
            let steam = mote_count(w, h, LightingMode::Full, Flavour::Steam);
            assert!((2..=4).contains(&steam), "2-4 steam wisps, got {steam}");
        }
    }

    #[test]
    fn every_non_outfall_theme_is_dry_and_outfall_respects_blackout() {
        let zones = [
            None,
            Some(LightZoneKind::Corridor),
            Some(LightZoneKind::Room),
            Some(LightZoneKind::Start),
            Some(LightZoneKind::Arena),
            Some(LightZoneKind::Blackout),
            Some(LightZoneKind::Maze),
        ];
        for theme in Theme::playable() {
            for zone in zones {
                let expected = theme == Theme::Outfall && zone != Some(LightZoneKind::Blackout);
                assert_eq!(
                    caustics_here(theme, zone),
                    expected,
                    "{} caustics in {zone:?}",
                    theme.name()
                );
            }
        }
    }

    #[test]
    fn outfall_caustics_skip_blackout_and_emissives_and_are_fainter_on_creatures() {
        let lvl = level(Theme::Outfall);
        let (w, h) = (64, 32);
        let mut grid = caustic_grid(w, h);
        // Right half is a Blackout zone: no water light there even on Outfall.
        for y in 0..h {
            for x in (w / 2)..w {
                grid.surfaces[y * w + x].zone = Some(LightZoneKind::Blackout);
            }
        }
        let mut pb = PixelBuffer::new(w, h);
        pb.set_materials(true);
        pb.set_pen(0);
        pb.clear((85, 71, 78));
        // Stamp a creature patch and an emissive pixel in the lit left half.
        let creature_pen = super::material::pen(super::material::MaterialKind::Slime, true, 3);
        let emissive_pen = super::material::pen(super::material::MaterialKind::Emissive, false, 0);
        pb.set_pen(creature_pen);
        for y in 4..28i32 {
            for x in 2..16i32 {
                pb.plot(x, y, (85, 71, 78));
            }
        }
        pb.set_pen(emissive_pen);
        pb.plot(18, 10, (85, 71, 78));
        pb.set_pen(0);

        let f = frame(&lvl, LightingMode::Full, 3.0);
        let mut canvas = testkit::canvas(&mut pb);
        draw_with(&grid, &mut canvas, &f, f.elapsed, Gains::live());

        assert_eq!(pb.pixel_at(18, 10), (85, 71, 78), "emissive excluded");
        let mut max_creature_lift = 0u8;
        let mut max_floor_lift = 0u8;
        for y in 0..h {
            for x in 0..w {
                let c = pb.pixel_at(x, y);
                let changed = c != (85, 71, 78);
                if x >= w / 2 {
                    assert!(!changed, "blackout zone must stay dry at ({x}, {y})");
                } else if (4..28).contains(&y) && (2..16).contains(&x) {
                    max_creature_lift = max_creature_lift.max(c.2.saturating_sub(78));
                } else if !(x == 18 && y == 10) {
                    max_floor_lift = max_floor_lift.max(c.2.saturating_sub(78));
                }
            }
        }
        assert!(max_floor_lift >= 25, "lit non-blackout Outfall floor gets bright caustics ({max_floor_lift})");
        assert!(
            max_creature_lift > 0 && max_creature_lift < max_floor_lift,
            "creatures take fainter caustics than floor: creature={max_creature_lift} floor={max_floor_lift}"
        );
    }

    #[test]
    fn outfall_marine_snow_is_sparse_cool_and_gated_to_lit_air() {
        let lvl = level(Theme::Outfall);
        let (w, h) = (120, 80);
        let full = mote_count(w, h, LightingMode::Full, Flavour::MarineSnow);
        let hard = mote_count(w, h, LightingMode::Hard, Flavour::MarineSnow);
        assert!((MOTE_COUNT_MIN..=MOTE_COUNT_MAX).contains(&full));
        assert!(hard < full && hard >= MOTE_COUNT_MIN / 2);

        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        // Air on left half only; zero dom so caustics are off and we measure snow alone.
        for y in 0..h {
            for x in 0..w / 2 {
                grid.info[y * w + x].air = [1200, 1200, 1200];
            }
        }
        // Continuous motion: moves smoothly inside a single 8 Hz window (1.00 -> 1.05 s)
        // rather than holding on 8 Hz steps, while staying well within per-frame churn.
        let f0 = frame(&lvl, LightingMode::Full, 1.000);
        let f1 = frame(&lvl, LightingMode::Full, 1.050);
        assert_eq!(f0.step8, f1.step8, "same 8 Hz bucket");
        let p0 = run(&grid, &f0, Gains::live(), w, h);
        let p1 = run(&grid, &f1, Gains::live(), w, h);
        assert_ne!(p0, p1, "marine snow advances smoothly from continuous time");
        let f_next = frame(&lvl, LightingMode::Full, 1.000 + 1.0 / 60.0);
        let p_next = run(&grid, &f_next, Gains::live(), w, h);
        let step_px = (0..w * h).filter(|&i| p0[i] != p_next[i]).count();
        assert!(
            step_px <= 24,
            "staggered continuous snow moved {step_px} pixels in one 60 Hz frame"
        );
        let hit: Vec<usize> = (0..w * h).filter(|&i| p0[i] != (85, 71, 78)).collect();
        assert!(!hit.is_empty(), "marine snow appears in lit air");
        for i in hit {
            assert!(i % w < w / 2, "snow must stay inside lit air");
            let c = p0[i];
            assert!(c.2 - 78 >= c.0 - 85, "marine snow adds more blue than red: {c:?}");
        }
    }

    #[test]
    fn outfall_caustics_drift_continuously_without_eight_hz_holds() {
        let lvl = level(Theme::Outfall);
        let (w, h) = (64, 32);
        let grid = caustic_grid(w, h);
        let a = frame(&lvl, LightingMode::Full, 2.000);
        let b = frame(&lvl, LightingMode::Full, 2.040);
        assert_eq!(a.step8, b.step8, "both times fall inside the same 8 Hz bucket");
        let pa = run(&grid, &a, Gains::live(), w, h);
        let pb = run(&grid, &b, Gains::live(), w, h);
        assert_ne!(
            pa, pb,
            "Outfall caustics advance continuously instead of freezing on 8 Hz steps"
        );
    }

    fn caustic_grid(w: usize, h: usize) -> Grid {
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            info.dom = 600;
            info.dom_rgb = (200, 230, 255);
        }
        grid
    }

    #[test]
    fn caustics_light_a_flagged_floor_and_nothing_else() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (64, 32);
        let mut grid = caustic_grid(w, h);
        // The top rows are a wall face: a wall takes no water light.
        for y in 0..8 {
            for x in 0..w {
                grid.surfaces[y * w + x].layer = LayerMask::WALLS.0;
            }
        }
        let f = frame(&lvl, LightingMode::Full, 3.0);
        // Dry: the arm is off, so the pass is a no-op.
        assert!(changed(&grid, &f, w, h).is_empty(), "no water light here yet");

        set_caustics_override(true);
        let hit = changed(&grid, &f, w, h);
        set_caustics_override(false);
        assert!(!hit.is_empty(), "the ocean arm must light the floor");
        for i in &hit {
            assert!(*i / w >= 8, "caustics on a wall at row {}", *i / w);
        }
        // Bounded: at restrained strength a crest lifts a pixel by a few levels,
        // never past the free-step band.
        let after = {
            set_caustics_override(true);
            let out = run(&grid, &f, Gains::live(), w, h);
            set_caustics_override(false);
            out
        };
        for (index, pixel) in after.iter().enumerate() {
            let now = [pixel.0, pixel.1, pixel.2];
            for (channel, before) in [85u8, 71, 78].into_iter().enumerate() {
                assert!(now[channel] >= before, "a caustic dimmed a pixel at {index}");
                assert!(
                    now[channel] - before <= 16,
                    "caustic crest too bright: {} at {index}",
                    now[channel]
                );
            }
        }
        // Darkness is dry even where the floor is flagged.
        let mut dark = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for s in dark.surfaces.iter_mut() {
            s.layer = LayerMask::FLOOR.0;
        }
        set_caustics_override(true);
        let dark_hit = changed(&dark, &f, w, h);
        set_caustics_override(false);
        assert!(dark_hit.is_empty(), "caustics cannot light a dark floor");
    }

    #[test]
    fn caustics_tile_across_the_floor_and_step_at_eight_hz() {
        // Period of both fronts in x: 64 px (both wave numbers divide it).
        for phase in [0i32, 20, 44, 128, 255] {
            for wy in [-37i32, 0, 11, 91] {
                for wx in [0i32, 7, 33, -45] {
                    assert_eq!(
                        caustics_q8(wx, wy, phase),
                        caustics_q8(wx + 64, wy, phase),
                        "seam at ({wx}, {wy}) phase {phase}"
                    );
                }
                // Smooth enough to read as water: measured over every phase
                // and position the steepest cell-to-cell step is 173 in x and
                // 175 in y, so neither axis has a cliff.
                let mut last = caustics_q8(-100, wy, phase);
                for wx in -99..100 {
                    let now = caustics_q8(wx, wy, phase);
                    assert!((now - last).abs() <= 176, "a caustic cliff at {wx}");
                    last = now;
                }
                let mut last = caustics_q8(3, -100, phase);
                for wy in -99..100 {
                    let now = caustics_q8(3, wy, phase);
                    assert!((now - last).abs() <= 180, "a caustic cliff at {wy}");
                    last = now;
                }
            }
        }

        let lvl = level(Theme::Sewer);
        let (w, h) = (64, 32);
        let grid = caustic_grid(w, h);
        let mut f = frame(&lvl, LightingMode::Full, 4.0);
        f.cam = Vec2::new(37.0, 21.0);
        set_caustics_override(true);
        let here = run(&grid, &f, Gains::live(), w, h);
        let mut same = frame(&lvl, LightingMode::Full, 4.05);
        same.cam = Vec2::new(37.0, 21.0);
        let held = run(&grid, &same, Gains::live(), w, h);
        let mut next = frame(&lvl, LightingMode::Full, 4.125);
        next.cam = Vec2::new(37.0, 21.0);
        let stepped = run(&grid, &next, Gains::live(), w, h);
        set_caustics_override(false);
        assert_eq!(here, held, "held inside the 8 Hz step");
        assert_ne!(here, stepped, "the pattern steps at 8 Hz");

        // Panning by a whole period in x shows the same water.
        let mut panned = frame(&lvl, LightingMode::Full, 4.0);
        panned.cam = Vec2::new(37.0 + 64.0, 21.0);
        set_caustics_override(true);
        let shifted = run(&grid, &panned, Gains::live(), w, h);
        set_caustics_override(false);
        for (index, (a, b)) in here.iter().zip(shifted.iter()).enumerate() {
            assert_eq!(a, b, "the pattern tiles at {index}");
        }
    }

    #[test]
    fn strength_zero_draws_nothing() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (120, 80);
        let mut airy = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in airy.info.iter_mut() {
            info.air = [1500, 1200, 900];
            info.dom = 600;
            info.dom_rgb = (240, 220, 190);
        }
        let mut watery = caustic_grid(w, h);
        for s in watery.surfaces.iter_mut() {
            s.layer = LayerMask::FLOOR.0;
        }
        let f = frame(&lvl, LightingMode::Full, 5.0);
        set_caustics_override(true);
        let quiet = run(&airy, &f, Gains::ZERO, w, h);
        assert!(quiet.iter().all(|&p| p == (85, 71, 78)));
        let dry = run(&watery, &f, Gains::ZERO, w, h);
        set_caustics_override(false);
        assert!(dry.iter().all(|&p| p == (85, 71, 78)));
        // And the same frame at full strength does draw, so the zero case is
        // not vacuous.
        assert!(!changed(&airy, &f, w, h).is_empty());
    }

    #[test]
    fn coarse_and_compact_tiers_draw_nothing() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (60, 40);
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            info.air = [1500, 1200, 900];
            info.dom = 600;
            info.dom_rgb = (240, 220, 190);
        }
        set_caustics_override(true);
        for mode in [LightingMode::Coarse, LightingMode::Compact] {
            let f = frame(&lvl, mode, 6.0);
            assert!(changed(&grid, &f, w, h).is_empty(), "{mode:?} motes are off");
        }
        set_caustics_override(false);
    }

    #[test]
    fn hard_steps_at_four_hz() {
        let lvl = level(Theme::Sewer);
        let (w, h) = (60, 40);
        let mut grid = testkit::flat_grid(w, h, 1, [256, 256, 256]);
        for info in grid.info.iter_mut() {
            info.air = [1500, 1500, 1500];
            info.dom = 800;
            info.dom_rgb = (230, 230, 230);
        }
        // 0.000 s → step 0, 0.125 s → step 1: one clock at 4 Hz, so identical.
        let a = run(&grid, &frame(&lvl, LightingMode::Hard, 0.0), Gains::live(), w, h);
        let b = run(&grid, &frame(&lvl, LightingMode::Hard, 0.125), Gains::live(), w, h);
        let c = run(&grid, &frame(&lvl, LightingMode::Hard, 0.25), Gains::live(), w, h);
        assert_eq!(a, b, "Hard holds two 8 Hz steps per step");
        assert_ne!(a, c, "and moves at 4 Hz");
    }

    #[test]
    fn outfall_ambient_motes_positions_are_continuous_and_wander_with_settle() {
        let seed = 0x1234_5678u32;
        let dt = 1.0 / 60.0;
        let mut speed_sum = 0.0f32;
        let mut vy_sum = 0.0f32;
        let mut up_count = 0usize;
        let mut left_count = 0usize;
        let mut right_count = 0usize;
        let slots = 60;
        for slot in 0..slots {
            let (x0, y0) = ambient_mote_world_pos(seed, 2.5, slot);
            let (x1, y1) = ambient_mote_world_pos(seed, 2.5 + dt, slot);
            let dx = x1 - x0;
            let dy = y1 - y0;
            let dist = (dx * dx + dy * dy).sqrt();
            // Continuous: a 1/60 s step must move a fraction of a pixel, never jump.
            assert!(
                dist < 0.25,
                "mote {slot} jumped {dist} px in {dt} s (must be continuous)"
            );
            let speed = dist / dt;
            assert!(
                speed <= 12.0,
                "mote {slot} speed {speed} px/s exceeded gentle drift budget"
            );
            speed_sum += speed;
            vy_sum += dy / dt;
            if dy < 0.0 {
                up_count += 1;
            }
            if dx < 0.0 {
                left_count += 1;
            }
            if dx > 0.0 {
                right_count += 1;
            }
        }
        let mean_speed = speed_sum / slots as f32;
        let mean_vy = vy_sum / slots as f32;
        assert!(
            (2.5..=8.5).contains(&mean_speed),
            "mean ambient mote speed {mean_speed} px/s outside ~3–8 px/s band"
        );
        assert!(
            mean_vy > 0.2 && mean_vy < 3.0,
            "expected slight downward settle, got mean vy={mean_vy}"
        );
        assert!(
            up_count > 0 && left_count > 10 && right_count > 10,
            "motes must wander in all directions (up={up_count}, left={left_count}, right={right_count})"
        );
    }

    #[test]
    fn outfall_ambient_motes_count_is_within_budget_across_all_tiers() {
        let full = ambient_mote_count(120, 80, LightingMode::Full);
        let hard = ambient_mote_count(120, 80, LightingMode::Hard);
        let coarse = ambient_mote_count(120, 80, LightingMode::Coarse);
        let compact = ambient_mote_count(120, 80, LightingMode::Compact);
        assert!(
            (AMBIENT_COUNT_MIN..=AMBIENT_COUNT_MAX).contains(&full),
            "Full 120×80 count {full} outside 40..=70"
        );
        assert!(hard > 0 && hard < full, "Hard ({hard}) must be reduced from Full ({full})");
        assert!(coarse > 0 && coarse < hard, "Coarse ({coarse}) must be reduced from Hard ({hard})");
        assert!(
            compact > 0 && compact <= coarse,
            "Compact ({compact}) must run at reduced count"
        );
        assert_eq!(ambient_mote_count(120, 80, LightingMode::Off), 0);

        for (w, h) in [(60, 40), (120, 80), (240, 160)] {
            let n = ambient_mote_count(w, h, LightingMode::Full);
            assert!(
                (AMBIENT_COUNT_MIN..=AMBIENT_COUNT_MAX).contains(&n),
                "{w}x{h} Full ambient count {n} out of budget"
            );
        }
    }

    #[test]
    fn outfall_ambient_motes_show_in_dim_and_brighter_in_lit_and_parallax_and_outfall_only() {
        let (w, h) = (120, 80);
        let base: Rgb = (24, 42, 52);

        // Dim grid (ambient only) vs bright grid (lit): both show motes, lit is brighter.
        let dim_grid = testkit::flat_grid(w, h, 1, [100, 110, 118]);
        let mut lit_grid = testkit::flat_grid(w, h, 1, [320, 320, 320]);
        for info in lit_grid.info.iter_mut() {
            info.dom = 400;
            info.dom_rgb = (240, 220, 180);
        }

        let draw_ambient = |grid: &Grid, theme: Theme, mode: LightingMode, cam: Vec2, t: f32| {
            let l = level(theme);
            let mut f = frame(&l, mode, t);
            f.cam = cam;
            let mut pb = PixelBuffer::new(w, h);
            pb.clear(base);
            let mut canvas = testkit::canvas(&mut pb);
            draw_outfall_ambient(grid, &mut canvas, &f, t);
            pb.pixels().to_vec()
        };

        // Visible at every live tier including Coarse and Compact even in dim water.
        for mode in LightingMode::LIVE {
            let px = draw_ambient(&dim_grid, Theme::Outfall, mode, Vec2::ZERO, 1.5);
            let painted = px.iter().filter(|&&c| c != base).count();
            assert!(
                painted >= 14,
                "{mode:?}: expected visible ambient motes in dim water, got {painted} px"
            );
        }

        // Brighter where light is.
        let dim_px = draw_ambient(&dim_grid, Theme::Outfall, LightingMode::Full, Vec2::ZERO, 1.5);
        let lit_px = draw_ambient(&lit_grid, Theme::Outfall, LightingMode::Full, Vec2::ZERO, 1.5);
        let max_lift = |px: &[Rgb]| -> u8 {
            px.iter()
                .map(|c| c.2.saturating_sub(base.2))
                .max()
                .unwrap_or(0)
        };
        assert!(
            max_lift(&lit_px) > max_lift(&dim_px) + 15,
            "lit motes ({}) must be brighter than dim motes ({})",
            max_lift(&lit_px),
            max_lift(&dim_px)
        );

        // World-space anchoring: panning cam by (17, 11) shifts every mote by (-17, -11) mod (w, h).
        let panned = draw_ambient(
            &dim_grid,
            Theme::Outfall,
            LightingMode::Coarse,
            Vec2::new(17.0, 11.0),
            1.5,
        );
        let origin = draw_ambient(&dim_grid, Theme::Outfall, LightingMode::Coarse, Vec2::ZERO, 1.5);
        for y in 0..h {
            for x in 0..w {
                let ox = (x + 17) % w;
                let oy = (y + 11) % h;
                assert_eq!(
                    panned[y * w + x],
                    origin[oy * w + ox],
                    "world-anchored mote mismatch at ({x},{y})"
                );
            }
        }

        // Outfall only: every other theme draws nothing.
        for theme in Theme::playable() {
            if theme == Theme::Outfall {
                continue;
            }
            let px = draw_ambient(&lit_grid, theme, LightingMode::Full, Vec2::ZERO, 1.5);
            assert!(
                px.iter().all(|&c| c == base),
                "{theme:?} must not draw Outfall ambient motes"
            );
        }
    }
}
