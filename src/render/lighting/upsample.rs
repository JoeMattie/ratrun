//! W1 — wall-aware bilinear upsampling of the light grid, the tone curve and
//! coloured bounce light (contract §4, techniques 04 + the bilinear
//! foundation). Every function here runs only while the effects are on.
//!
//! * [`composite`] — every pixel takes its light from the four samples around
//!   it (Q8 fixed-point weights), so a pool's edge and a shadow's edge are a
//!   ramp instead of 4 × 4 stairs. A tap whose wall class differs from the
//!   pixel's own (covering) sample gets weight 0 and the rest are
//!   renormalised: corridor light never bleeds across a wall face. A cell
//!   whose four taps are equal takes the old per-run fast path, so flat light
//!   is bit-identical to the old composite. Compact stays flat (old blocks).
//! * The tone curve — `mul` may now reach [`MUL_CEILING`]; [`tone`] is a soft
//!   shoulder above [`KNEE`], applied to a sample's **peak** channel and the
//!   other two scaled by the same factor, so a bright core keeps its hue
//!   instead of all three channels clipping to flat white.
//! * [`ambient_scale`] — the darker base level (Full/Hard), per theme.
//! * [`bounce`] — a wall-aware low-frequency blur of the lit-minus-ambient
//!   energy, added back into `mul` at `strength::BOUNCE`: the floor in a
//!   lamp's shadow picks up the lamp's colour.
//!
//! Integer arithmetic in the per-pixel loop; scratch lives in thread-locals
//! that only grow (no allocation in a steady frame). Render-only and
//! stateless across frames, so a still scene draws the same pixels.

#![allow(clippy::needless_range_loop)] // Parallel per-channel arrays.

use std::cell::RefCell;

use super::engine::{
    ambient_level_scaled, palette_dither, zone_gain, Grid, LightingMode, Sample, AMBIENT_SCALE,
};
use super::fx::{strength, Canvas, FxFrame};
use super::LayerMask;
use crate::game::level::Theme;
use crate::render::palette::Rgb;
use crate::term::ColorDepth;

/// The `mul` clamp `Sample::light` uses while the effects are on (fx off
/// keeps 350): headroom so overlapping lights do not hit a hard wall; the
/// tone curve below brings the peak back down softly.
pub(super) const MUL_CEILING: i32 = 460;

/// How far apart two taps' multipliers (`Q8`) and adds may be and still be
/// shaded as flat blocks, and whether horizontally adjacent cell pairs share
/// one interpolation weight. Both are byte budget, chosen by measurement
/// (Sewer, 120x40, bot play at 5:00, bytes a frame; effects off is 55.3 k):
///
/// | near | pair | bytes |
/// | --- | --- | --- |
/// | exact | no | 101.7 k |
/// | exact | yes | 79.9 k |
/// | 12 | yes | 77.8 k |
/// | 20 | yes | 71.7 k |
///
/// A smooth gradient over flat art gives every cell its own colour, so every
/// cell needs its own SGR; pairs and near-flat blocks give the encoder runs
/// back where the difference is not visible.
const NEAR_MUL: i32 = 20;
const NEAR_ADD: i32 = 5;
pub(super) const PAIR_X: bool = true;
/// Cell pairs share a weight only where the two taps' multipliers are within
/// this (`Q8.8`) of each other: a pair then steps by at most half of it (about
/// 5 % of the albedo), which does not read as a column.
const PAIR_STEEP: i32 = 28;

/// Below this peak multiplier (`Q8.8`, 220 ≈ 0.86×) the tone curve is the
/// identity, so ambient, shadow and ordinary pool light are untouched.
pub(super) const KNEE: i32 = 220;
/// Shoulder width of the soft clip: above the knee the curve is
/// `KNEE + d·S / (d + S)` (`d` = excess), asymptotic to `KNEE + S`. With 320
/// the ceiling (460) maps to 357, about the old hard clamp (350).
const SHOULDER: i32 = 320;
/// How much of the shoulder is applied, Q8: `strength::TONE` 0.5 is the
/// owner's restrained target and the full curve (bold would be steeper, so
/// the target is the curve itself; clamped to 256).
const TONE_MIX: i32 = {
    let k = (strength::TONE * 512.0) as i32;
    if k > 256 {
        256
    } else {
        k
    }
};

/// Ambient share of the theme tint on Full/Hard (fx on), per theme. All in
/// 0.40–0.50 (the old 0.60 × about 0.75): darker floors so pools read, each
/// floor keeping enough of its own tint. Coarse/Compact keep [`AMBIENT_SCALE`].
const AMBIENT_SEWER: f32 = 0.45;
const AMBIENT_KITCHEN: f32 = 0.47;
const AMBIENT_LAB: f32 = 0.44;
const AMBIENT_LAUNDRY: f32 = 0.47;
const AMBIENT_PRINTWORKS: f32 = 0.45;
const AMBIENT_FLATBED: f32 = 0.48;
const AMBIENT_OUTFALL: f32 = 0.41;

/// Bounce blur radius in samples (a tent of weights 4,3,2,1 per axis, sum 16,
/// so each pass is a shift). 3 samples is 12 px at stride 4, 24 px at 8: wide
/// against the 4 px block, low-frequency against a pool.
const BOUNCE_RADIUS: usize = 3;
const BOUNCE_WEIGHTS: [i32; BOUNCE_RADIUS + 1] = [4, 3, 2, 1];
/// Bold share of the blurred energy added back (Q8 after `strength::BOUNCE`).
const BOUNCE_BOLD: f32 = 0.34;
const BOUNCE_GAIN: i32 = (BOUNCE_BOLD * strength::BOUNCE * 256.0) as i32;
/// A bounce increment below this many `Q8.8` levels is exactly zero, so the
/// far tail of the blur never creeps cells (contract §1: flat steps).
const BOUNCE_MIN: i32 = 3;

/// The share of the theme tint every floor's ambient is, while the effects are
/// on (fx off keeps [`AMBIENT_SCALE`]). Called once per frame from
/// `Grid::prepare`.
pub(super) fn ambient_scale(theme: Theme, mode: LightingMode) -> f32 {
    if !matches!(mode, LightingMode::Full | LightingMode::Hard) {
        return AMBIENT_SCALE;
    }
    match theme {
        Theme::Sewer => AMBIENT_SEWER,
        Theme::Kitchen => AMBIENT_KITCHEN,
        Theme::Lab => AMBIENT_LAB,
        Theme::Laundry => AMBIENT_LAUNDRY,
        Theme::Printworks => AMBIENT_PRINTWORKS,
        Theme::Trailer => AMBIENT_FLATBED,
        Theme::Outfall => AMBIENT_OUTFALL,
    }
}

/// The tone curve on a peak multiplier: identity up to [`KNEE`], then the
/// soft shoulder, never above [`MUL_CEILING`] and never above its input.
/// Monotonic (non-decreasing) over the whole `u16` range.
#[inline]
pub(super) fn tone(m: u16) -> u16 {
    let m = i32::from(m);
    if m <= KNEE {
        return m as u16;
    }
    let d = m - KNEE;
    let curve = KNEE + d * SHOULDER / (d + SHOULDER);
    let out = m + (((curve - m) * TONE_MIX) >> 8);
    out.clamp(0, MUL_CEILING) as u16
}

/// One sample through the tone curve: the peak channel is toned and the
/// other two scaled by the same factor (hue kept), `add`/`tint` untouched.
#[inline]
fn tone_sample(s: &Sample) -> Sample {
    let peak = s.mul[0].max(s.mul[1]).max(s.mul[2]);
    if i32::from(peak) <= KNEE {
        return *s;
    }
    let target = u32::from(tone(peak));
    let p = u32::from(peak);
    let mut out = *s;
    for n in 0..3 {
        out.mul[n] = ((u32::from(s.mul[n]) * target + p / 2) / p) as u16;
    }
    out
}

#[inline]
fn same(a: &Sample, b: &Sample) -> bool {
    a.mul == b.mul && a.add == b.add && a.tint == b.tint
}

/// Taps this close shade as flat blocks ([`NEAR_MUL`], [`NEAR_ADD`]): the
/// step between two such blocks is under about five output levels on the
/// dark floors, and a gradient that shallow costs a new colour on every cell
/// of a flat run for nothing the eye can see.
#[inline]
fn near(a: &Sample, b: &Sample) -> bool {
    a.tint == b.tint
        && (0..3).all(|n| {
            (i32::from(a.mul[n]) - i32::from(b.mul[n])).abs() <= NEAR_MUL
                && (i32::from(a.add[n]) - i32::from(b.add[n])).abs() <= NEAR_ADD
        })
}

/// The wall class of a sample (0 floor/props/bodies, 1 wall).
#[inline]
fn class(grid: &Grid, i: usize) -> usize {
    usize::from(grid.surfaces.get(i).is_some_and(|s| s.layer & LayerMask::WALLS.0 != 0))
}

/// Ten interpolated channels: `mul[0..3]`, `add[0..3]`, `tint[0..4]`.
const CH: usize = 10;

#[inline]
fn channels(s: &Sample) -> [i32; CH] {
    [
        i32::from(s.mul[0]),
        i32::from(s.mul[1]),
        i32::from(s.mul[2]),
        i32::from(s.add[0]),
        i32::from(s.add[1]),
        i32::from(s.add[2]),
        i32::from(s.tint[0]),
        i32::from(s.tint[1]),
        i32::from(s.tint[2]),
        i32::from(s.tint[3]),
    ]
}

/// One sample column's vertical blend for one wall class: `v` is the
/// weighted sum (weights Q8), `w` the summed weight (0 = no tap of that
/// class in this column).
#[derive(Clone, Copy, Default)]
struct Col {
    v: [i32; CH],
    w: i32,
}

/// Per-row scratch: two `Col`s (one per class) per sample column, plus
/// whether the column's two taps are equal and of one class.
#[derive(Default)]
struct Scratch {
    toned: Vec<Sample>,
    cols: Vec<Col>,
    flat: Vec<bool>,
}

/// The bounce term's two scratch planes (energy, blur).
type BouncePlanes = (Vec<[i32; 3]>, Vec<[i32; 3]>);

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
    static BOUNCE: RefCell<BouncePlanes> =
        const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Grid stage, after every splat and before the vignette: add the coloured
/// bounce term into `grid.samples[].mul` (a low-frequency spread of the lit
/// energy, `fx::strength::BOUNCE`).
///
/// Energy is `mul` above the sample's own ambient level (what the splats
/// added). It is blurred by a separable tent ([`BOUNCE_RADIUS`]) that walks
/// outward from each target and stops at the first wall sample; walls
/// neither give nor take bounce, so it can never pass through one. The
/// result, at [`BOUNCE_GAIN`], is added to `mul` (clamped to the ceiling);
/// increments under [`BOUNCE_MIN`] are dropped.
pub(super) fn bounce(grid: &mut Grid, frame: &FxFrame<'_>) {
    let (w, h) = (grid.w, grid.h);
    let n = w * h;
    if n == 0 || grid.samples.len() < n || grid.surfaces.len() < n || BOUNCE_GAIN <= 0 {
        return;
    }
    let scale = ambient_scale(frame.theme, frame.mode);
    BOUNCE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let (energy, blur) = &mut *cell;
        energy.clear();
        energy.resize(n, [0; 3]);
        blur.clear();
        blur.resize(n, [0; 3]);
        let mut any = false;
        for i in 0..n {
            let surface = grid.surfaces[i];
            if surface.layer & LayerMask::WALLS.0 != 0 {
                continue;
            }
            let gain = zone_gain(surface.zone);
            let s = grid.samples[i];
            let mut e = [0i32; 3];
            for c in 0..3 {
                let base = i32::from(ambient_level_scaled(frame.ambient[c], gain, scale));
                e[c] = (i32::from(s.mul[c]) - base).max(0);
                any |= e[c] != 0;
            }
            energy[i] = e;
        }
        if !any {
            return;
        }
        let wall = |i: usize| grid.surfaces[i].layer & LayerMask::WALLS.0 != 0;
        // Horizontal pass: energy -> blur.
        for y in 0..h {
            let row = y * w;
            for x in 0..w {
                let i = row + x;
                if wall(i) {
                    continue;
                }
                let mut acc = [0i32; 3];
                for c in 0..3 {
                    acc[c] = energy[i][c] * BOUNCE_WEIGHTS[0];
                }
                for d in 1..=BOUNCE_RADIUS {
                    if x < d || wall(i - d) {
                        break;
                    }
                    for c in 0..3 {
                        acc[c] += energy[i - d][c] * BOUNCE_WEIGHTS[d];
                    }
                }
                for d in 1..=BOUNCE_RADIUS {
                    if x + d >= w || wall(i + d) {
                        break;
                    }
                    for c in 0..3 {
                        acc[c] += energy[i + d][c] * BOUNCE_WEIGHTS[d];
                    }
                }
                blur[i] = [acc[0] >> 4, acc[1] >> 4, acc[2] >> 4];
            }
        }
        // Vertical pass: blur -> mul increment (energy is free again; reuse
        // it as the vertical result so the pass reads only finished rows).
        for x in 0..w {
            for y in 0..h {
                let i = y * w + x;
                if wall(i) {
                    energy[i] = [0; 3];
                    continue;
                }
                let mut acc = [0i32; 3];
                for c in 0..3 {
                    acc[c] = blur[i][c] * BOUNCE_WEIGHTS[0];
                }
                for d in 1..=BOUNCE_RADIUS {
                    if y < d || wall(i - d * w) {
                        break;
                    }
                    for c in 0..3 {
                        acc[c] += blur[i - d * w][c] * BOUNCE_WEIGHTS[d];
                    }
                }
                for d in 1..=BOUNCE_RADIUS {
                    if y + d >= h || wall(i + d * w) {
                        break;
                    }
                    for c in 0..3 {
                        acc[c] += blur[i + d * w][c] * BOUNCE_WEIGHTS[d];
                    }
                }
                energy[i] = [acc[0] >> 4, acc[1] >> 4, acc[2] >> 4];
            }
        }
        for i in 0..n {
            let b = energy[i];
            let s = &mut grid.samples[i];
            for c in 0..3 {
                let add = (b[c] * BOUNCE_GAIN) >> 8;
                if add >= BOUNCE_MIN {
                    s.mul[c] = (i32::from(s.mul[c]) + add).min(MUL_CEILING) as u16;
                }
            }
        }
    });
}

/// Shade the pixels with the grid (replaces `Grid::composite` while the
/// effects are on): bilinear taps that never cross a wall boundary, the tone
/// curve, the tint mix, the shine floor and — below truecolor — the palette
/// dither, exactly like the old path.
pub(super) fn composite(grid: &Grid, canvas: &mut Canvas<'_>, frame: &FxFrame<'_>) {
    let (w, h) = (canvas.w, canvas.h);
    let n = grid.w * grid.h;
    if w == 0 || h == 0 || grid.stride == 0 || n == 0 || grid.samples.len() < n {
        return;
    }
    SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        let scratch = &mut *scratch;
        scratch.toned.clear();
        scratch.toned.extend(grid.samples[..n].iter().map(tone_sample));
        if frame.mode == LightingMode::Compact {
            flat(grid, &scratch.toned, canvas.px, w, h, frame.depth);
        } else {
            bilinear(grid, scratch, canvas.px, w, h, frame.depth);
        }
    });
}

/// The old blocky composite over the toned samples (Compact).
fn flat(grid: &Grid, toned: &[Sample], pixels: &mut [Rgb], w: usize, h: usize, depth: ColorDepth) {
    let stride = grid.stride;
    let low_colour = depth != ColorDepth::Truecolor;
    let (ox, oy) = grid.offset;
    for (y, row) in pixels.chunks_exact_mut(w).take(h).enumerate() {
        let by = y + oy;
        let base = (by / stride).min(grid.h - 1) * grid.w;
        let mut x = 0;
        while x < w {
            let bx = x + ox;
            let end = (x + stride - bx % stride).min(w);
            let sample = &toned[base + (bx / stride).min(grid.w - 1)];
            if low_colour {
                for (dx, pixel) in row[x..end].iter_mut().enumerate() {
                    *pixel = palette_dither(sample.shade_exact(*pixel), depth, bx + dx, by);
                }
            } else {
                sample.shade_run(&mut row[x..end]);
            }
            x = end;
        }
    }
}

/// Q8 position of block coordinate `b` between the sample centres around it:
/// `(s0, weight of s0 + 1)`. Sample `s`'s centre is block coordinate
/// `s * stride + stride / 2` (`Grid::point`).
#[inline]
fn locate(b: usize, stride: usize) -> (i32, i32) {
    let s = stride as i32;
    let t = b as i32 - s / 2;
    let s0 = t.div_euclid(s);
    let frac = t - s0 * s;
    (s0, frac * 256 / s)
}

fn bilinear(
    grid: &Grid,
    scratch: &mut Scratch,
    pixels: &mut [Rgb],
    w: usize,
    h: usize,
    depth: ColorDepth,
) {
    let Scratch { toned, cols, flat } = scratch;
    let (gw, gh) = (grid.w, grid.h);
    let stride = grid.stride;
    let s = stride as i32;
    let half = s / 2;
    let (ox, oy) = grid.offset;
    let low_colour = depth != ColorDepth::Truecolor;
    let clamp_x = |i: i32| i.clamp(0, gw as i32 - 1) as usize;
    let clamp_y = |i: i32| i.clamp(0, gh as i32 - 1) as usize;
    cols.clear();
    cols.resize(gw * 2, Col::default());
    flat.clear();
    flat.resize(gw, false);
    // Rows that share their vertical taps and weight share the column blend.
    let mut cached: Option<(usize, usize, i32)> = None;
    for (y, row) in pixels.chunks_exact_mut(w).take(h).enumerate() {
        let by = y + oy;
        let (s0y, wy) = locate(by, stride);
        let (top, bot) = (clamp_y(s0y), clamp_y(s0y + 1));
        let own_row = (by / stride).min(gh - 1);
        if cached != Some((top, bot, wy)) {
            cached = Some((top, bot, wy));
            let (wt, wb) = (256 - wy, wy);
            for sx in 0..gw {
                let (it, ib) = (top * gw + sx, bot * gw + sx);
                let (ct, cb) = (class(grid, it), class(grid, ib));
                let (vt, vb) = (channels(&toned[it]), channels(&toned[ib]));
                let mut pair = [Col::default(); 2];
                for (k, col) in pair.iter_mut().enumerate() {
                    if ct == k && wt != 0 {
                        col.w += wt;
                        for j in 0..CH {
                            col.v[j] += vt[j] * wt;
                        }
                    }
                    if cb == k && wb != 0 {
                        col.w += wb;
                        for j in 0..CH {
                            col.v[j] += vb[j] * wb;
                        }
                    }
                }
                cols[sx * 2] = pair[0];
                cols[sx * 2 + 1] = pair[1];
                flat[sx] = ct == cb && near(&toned[it], &toned[ib]);
            }
        }
        let mut x = 0;
        while x < w {
            let bx = x + ox;
            let (s0x, _) = locate(bx, stride);
            // Pixels up to the next sample centre share these four taps.
            let end = (((s0x + 1) * s + half) as usize - ox).min(w);
            let (l, r) = (clamp_x(s0x), clamp_x(s0x + 1));
            let (tl, tr) = (top * gw + l, top * gw + r);
            let uniform = flat[l]
                && flat[r]
                && class(grid, tl) == class(grid, tr)
                && near(&toned[tl], &toned[tr]);
            if uniform {
                // All four taps equal (or as good as): each pixel takes its own
                // sample, which for equal taps is the old composite exactly.
                let split = (((s0x + 1) * s).max(0) as usize).saturating_sub(ox).clamp(x, end);
                for (lo, hi) in [(x, split), (split, end)] {
                    if lo == hi {
                        continue;
                    }
                    let sample = &toned[own_row * gw + ((lo + ox) / stride).min(gw - 1)];
                    if low_colour {
                        for (dx, pixel) in row[lo..hi].iter_mut().enumerate() {
                            *pixel =
                                palette_dither(sample.shade_exact(*pixel), depth, lo + ox + dx, by);
                        }
                    } else {
                        sample.shade_run(&mut row[lo..hi]);
                    }
                }
                x = end;
                continue;
            }
            // Without tint on any tap the tint channels stay zero: blend six.
            let (bl, br) = (bot * gw + l, bot * gw + r);
            let nch = if (toned[tl].tint[3] | toned[tr].tint[3] | toned[bl].tint[3] | toned[br].tint[3]) == 0 {
                6
            } else {
                CH
            };
            // Within the segment the horizontal weight steps by exactly
            // `256 / stride` a pixel (the stride is a power of two) and the
            // pixel's own sample changes once, at the block boundary.
            let (_, wx0) = locate(bx, stride);
            let dw = 256 / s;
            let split = (((s0x + 1) * s).max(0) as usize).saturating_sub(ox).clamp(x, end);
            for (lo, hi) in [(x, split), (split, end)] {
                if lo == hi {
                    continue;
                }
                let own = own_row * gw + ((lo + ox) / stride).min(gw - 1);
                let k = class(grid, own);
                let (cl, cr) = (&cols[l * 2 + k], &cols[r * 2 + k]);
                // Where the light changes fast across this sample (the rim of
                // the rat's aura, a pool's edge) paired cells showed as 2-cell
                // columns: there every cell takes its own weight.
                let pair = PAIR_X && {
                    let (wl, wr) = (cl.w.max(1), cr.w.max(1));
                    (0..3).all(|j| (cl.v[j] / wl - cr.v[j] / wr).abs() <= PAIR_STEEP)
                };
                for px in lo..hi {
                    let pbx = px + ox;
                    let wx = wx0 + (px - x) as i32 * dw;
                    // Two cells share a horizontal weight (see `PAIR_X`).
                    let wx = if pair { wx & !(2 * dw - 1) } else { wx };
                    let (hl, hr) = (256 - wx, wx);
                    let ws = hl * cl.w + hr * cr.w;
                    let sample = if ws <= 0 {
                        toned[own]
                    } else {
                        let mut v = [0i32; CH];
                        if ws == 1 << 16 {
                            for j in 0..nch {
                                v[j] = (hl * cl.v[j] + hr * cr.v[j] + (1 << 15)) >> 16;
                            }
                        } else {
                            let d = i64::from(ws);
                            for j in 0..nch {
                                let sum = i64::from(hl) * i64::from(cl.v[j])
                                    + i64::from(hr) * i64::from(cr.v[j]);
                                v[j] = (sum * 2 + d).div_euclid(2 * d) as i32;
                            }
                        }
                        Sample {
                            mul: [
                                v[0].clamp(0, MUL_CEILING) as u16,
                                v[1].clamp(0, MUL_CEILING) as u16,
                                v[2].clamp(0, MUL_CEILING) as u16,
                            ],
                            add: [
                                // An interpolated value lies between its taps
                                // (the vignette may lift one past 142): only
                                // the i16 range needs guarding.
                                v[3].clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                                v[4].clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                                v[5].clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                            ],
                            tint: [
                                v[6].clamp(0, 255) as u8,
                                v[7].clamp(0, 255) as u8,
                                v[8].clamp(0, 255) as u8,
                                v[9].clamp(0, 255) as u8,
                            ],
                        }
                    };
                    let pixel = &mut row[px];
                    let lit = sample.shade_exact(*pixel);
                    *pixel = if low_colour {
                        palette_dither(lit, depth, pbx, by)
                    } else {
                        lit
                    };
                }
            }
            x = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fx::testkit;
    use super::*;
    use crate::game::level::Level;

    const THEMES: [Theme; 7] = [
        Theme::Sewer,
        Theme::Kitchen,
        Theme::Lab,
        Theme::Laundry,
        Theme::Printworks,
        Theme::Trailer,
        Theme::Outfall,
    ];
    const MODES: [LightingMode; 4] = [
        LightingMode::Full,
        LightingMode::Hard,
        LightingMode::Coarse,
        LightingMode::Compact,
    ];

    fn run(grid: &Grid, px: &mut [Rgb], w: usize, h: usize, mode: LightingMode, level: &Level) {
        let mut frame = testkit::frame(level, mode);
        frame.depth = ColorDepth::Truecolor;
        let mut canvas = Canvas { w, h, px, mat: &[] };
        composite(grid, &mut canvas, &frame);
    }

    fn lum(c: Rgb) -> i32 {
        i32::from(c.0) + i32::from(c.1) + i32::from(c.2)
    }

    #[test]
    fn tone_is_monotonic_and_bounded() {
        let mut prev = 0;
        for m in 0..=u16::MAX {
            let t = tone(m);
            assert!(t >= prev, "tone({m}) = {t} < {prev}");
            assert!(i32::from(t) <= MUL_CEILING);
            assert!(t <= m);
            if i32::from(m) <= KNEE {
                assert_eq!(t, m);
            }
            prev = t;
        }
        // The shoulder does something above the knee.
        assert!(tone(MUL_CEILING as u16) < MUL_CEILING as u16);
        assert!(i32::from(tone(MUL_CEILING as u16)) > KNEE + 60);
    }

    #[test]
    fn toned_sample_keeps_its_hue() {
        let s = Sample {
            mul: [440, 220, 110],
            ..Sample::default()
        };
        let t = tone_sample(&s);
        assert!(t.mul[0] < 440);
        // Ratios kept within rounding.
        assert!((i32::from(t.mul[0]) - 2 * i32::from(t.mul[1])).abs() <= 2);
        assert!((i32::from(t.mul[1]) - 2 * i32::from(t.mul[2])).abs() <= 2);
    }

    #[test]
    fn ambient_scale_is_in_bounds_for_every_theme_and_tier() {
        for theme in THEMES {
            for mode in MODES {
                let s = ambient_scale(theme, mode);
                if matches!(mode, LightingMode::Full | LightingMode::Hard) {
                    assert!((0.40..=0.50).contains(&s), "{theme:?} {mode:?} {s}");
                } else {
                    assert_eq!(s, AMBIENT_SCALE);
                }
            }
        }
    }

    #[test]
    fn equal_taps_match_the_old_composite_exactly() {
        let level = Level::empty(Theme::Sewer);
        for stride in [4usize, 8] {
            for offset in [(0usize, 0usize), (1, 3), (stride - 1, stride - 1)] {
                let (w, h) = (37usize, 21usize);
                let gw = (w + offset.0).div_ceil(stride);
                let gh = (h + offset.1).div_ceil(stride);
                let mut grid = testkit::flat_grid(gw, gh, stride, [180, 150, 120]);
                grid.offset = offset;
                for s in &mut grid.samples {
                    s.add = [12, -5, 30];
                    s.tint = [40, 20, 10, 30];
                }
                let src: Vec<Rgb> = (0..w * h)
                    .map(|i| ((i * 7 % 256) as u8, (i * 13 % 256) as u8, (i * 29 % 256) as u8))
                    .collect();
                let mut old = src.clone();
                grid.composite_px(&mut old, w, h, ColorDepth::Truecolor);
                for mode in [LightingMode::Full, LightingMode::Compact] {
                    let mut new = src.clone();
                    run(&grid, &mut new, w, h, mode, &level);
                    assert_eq!(old, new, "stride {stride} offset {offset:?} {mode:?}");
                }
            }
        }
    }

    #[test]
    fn bilinear_is_continuous_across_block_boundaries() {
        let level = Level::empty(Theme::Lab);
        let (gw, gh, stride) = (12usize, 8usize, 4usize);
        let (w, h) = (gw * stride, gh * stride);
        let mut grid = testkit::flat_grid(gw, gh, stride, [40, 40, 40]);
        // A bright 2×2 pool in the middle.
        for (x, y) in [(5, 3), (6, 3), (5, 4), (6, 4)] {
            grid.samples[y * gw + x].mul = [200, 200, 200];
        }
        let mut px = vec![(200u8, 200u8, 200u8); w * h];
        let mut old = px.clone();
        run(&grid, &mut px, w, h, LightingMode::Full, &level);
        grid.composite_px(&mut old, w, h, ColorDepth::Truecolor);
        let max_step = |img: &[Rgb]| {
            let mut m = 0;
            for y in 0..h {
                for x in 1..w {
                    m = m.max((lum(img[y * w + x]) - lum(img[y * w + x - 1])).abs());
                }
            }
            m
        };
        let (new_step, old_step) = (max_step(&px), max_step(&old));
        // Old: one hard 4 px stair (~94 per channel); new: a ramp. Across a
        // row it runs in cell pairs (`PAIR_X`), so the largest step is about
        // half the old stair (a quarter without the pairing); down a column
        // it is a quarter.
        assert!(old_step > 200, "old step {old_step}");
        assert!(new_step * 10 < old_step * 6, "new step {new_step} vs old {old_step}");
    }

    #[test]
    fn a_wall_between_samples_blocks_the_blend() {
        let level = Level::empty(Theme::Sewer);
        let (gw, gh, stride) = (10usize, 4usize, 4usize);
        let (w, h) = (gw * stride, gh * stride);
        let mut grid = testkit::flat_grid(gw, gh, stride, [30, 30, 30]);
        // Column 5 is a wall; the floor left of it is brightly lit.
        for y in 0..gh {
            grid.surfaces[y * gw + 5].layer = LayerMask::WALLS.0;
            for x in 0..5 {
                grid.samples[y * gw + x].mul = [300, 300, 300];
            }
        }
        let mut px = vec![(128u8, 128u8, 128u8); w * h];
        run(&grid, &mut px, w, h, LightingMode::Full, &level);
        // Every pixel under the wall column and right of it stays at the dark
        // level: (128 * 30) >> 8 = 15.
        for y in 0..h {
            for x in 5 * stride..w {
                assert_eq!(px[y * w + x], (15, 15, 15), "pixel ({x}, {y})");
            }
        }
        // And the lit floor still ramps toward the dark floor only on its side:
        // the last lit pixel is still lit (no blend with the wall).
        let last = tone(300);
        let lit = ((128 * i32::from(last)) >> 8).min(255) as u8;
        assert_eq!(px[w + 5 * stride - 1], (lit, lit, lit));
    }

    #[test]
    fn bounce_colours_a_shadowed_neighbour_and_leaves_an_empty_scene() {
        let level = Level::empty(Theme::Sewer);
        let frame = testkit::frame(&level, LightingMode::Full);
        let scale = ambient_scale(Theme::Sewer, LightingMode::Full);
        let base: [u16; 3] = std::array::from_fn(|c| {
            ambient_level_scaled(frame.ambient[c], zone_gain(None), scale)
        });
        let (gw, gh) = (20usize, 8usize);
        let warm = [base[0] + 200, base[1] + 120, base[2] + 20];
        // Empty: ambient only, nothing changes.
        let mut empty = testkit::flat_grid(gw, gh, 4, base);
        bounce(&mut empty, &frame);
        assert!(empty.samples.iter().all(|s| s.mul == base));
        // A warm lit band (columns 3..=7); the shadowed floor two samples past
        // its edge gains warm light (red more than blue).
        let mut grid = testkit::flat_grid(gw, gh, 4, base);
        for y in 0..gh {
            for x in 3..=7 {
                grid.samples[y * gw + x].mul = warm;
            }
        }
        bounce(&mut grid, &frame);
        let near = grid.samples[4 * gw + 9].mul;
        assert!(near[0] > base[0], "{near:?} vs {base:?}");
        assert!(i32::from(near[0]) - i32::from(base[0]) > i32::from(near[2]) - i32::from(base[2]));
        // Far outside the radius: untouched.
        assert_eq!(grid.samples[4 * gw + 15].mul, base);
        // A wall column in between stops it, and the wall takes nothing.
        let mut walled = testkit::flat_grid(gw, gh, 4, base);
        for y in 0..gh {
            walled.surfaces[y * gw + 8].layer = LayerMask::WALLS.0;
            for x in 3..=7 {
                walled.samples[y * gw + x].mul = warm;
            }
        }
        bounce(&mut walled, &frame);
        assert_eq!(walled.samples[4 * gw + 9].mul, base);
        assert_eq!(walled.samples[4 * gw + 8].mul, base);
    }

    #[test]
    fn still_scene_composites_identically_twice() {
        let level = Level::empty(Theme::Kitchen);
        let (gw, gh, stride) = (9usize, 7usize, 4usize);
        let mut grid = testkit::flat_grid(gw, gh, stride, [60, 50, 40]);
        grid.samples[3 * gw + 4].mul = [420, 300, 200];
        let (w, h) = (gw * stride - 2, gh * stride - 3);
        let src = vec![(150u8, 120u8, 90u8); w * h];
        let (mut a, mut b) = (src.clone(), src);
        run(&grid, &mut a, w, h, LightingMode::Full, &level);
        run(&grid, &mut b, w, h, LightingMode::Full, &level);
        assert_eq!(a, b);
    }
}
