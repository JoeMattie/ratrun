//! The 1× edge treatment [`Smooth::Xbr`](super::Smooth::Xbr): an xBR 2×
//! upscale of the half-block picture, reduced to one glyph and two colours a
//! cell.
//!
//! The rules are FFmpeg's `xbr=2` filter (`libavfilter/vf_xbr.c`, `FILT2`,
//! "level 2"): each corner of a pixel compares the cost of the two diagonals
//! through it over a radius-two neighbourhood (summed YUV distances, the
//! crossing edge weighted by four), and where the diagonal that enters the
//! pixel wins, the corner leans toward the nearer of its two axial
//! neighbours — by a half on a plain diagonal, by 3/4 with a 1/4 spill into the
//! next quadrant on a 2:1 slope, by 7/8 where both slopes meet. The four
//! corners are taken in the reference's order (lower right, upper right, upper
//! left, lower left), because a later one blends what an earlier one wrote.
//!
//! What is kept differently from the reference, and why:
//!
//! * **Weights, not colours.** A sub-pixel is kept as its share of each axial
//!   neighbour (out of `BASE`) instead of a blended RGB value, because a
//!   terminal cell has two colours and no blend. The cell's glyph is then the
//!   pattern nearest that coverage, and its two colours are **exact source
//!   colours**: a smoothed cell never shows a colour the picture did not hold.
//! * **Contrast.** The reference tests `PE != PH` and friends for exact
//!   equality, which a lit picture never satisfies; they are the `EDGE` test
//!   the other 1× passes use (`differs`), so a light gradient or texture grain
//!   is never an edge and costs nothing.
//! * **Stairs only.** A corner leans only where the pixel's colour carries on
//!   along the diagonal on at least one side (the reference's `eq(PE, PG) ||
//!   eq(PE, PC)` made a requirement), so a rectangle keeps its corners and an
//!   axis-aligned picture is the half-block blit, byte for byte.
//! * **Two colours.** A neighbour of a third colour lends nothing to a cell.
//!
//! A terminal cell is two source pixels stacked, so its xBR block is 2 wide and
//! 4 tall. That block is fitted on each grid the terminal draws — quadrants
//! (rows 0+1, 2+3), sextants (the prototype's band mapping: a 12-row mesh on
//! which a block row is 3 rows and a sextant row 4, so the sextant rows weigh
//! the block rows 3:1, 2:2 and 1:3) and octants (1:1) — by majority in every
//! sub-cell, and the grid with the least error wins; an exact tie goes to the
//! coarser grid unless that would leave the cell as it was. (A quadrant-only
//! terminal has one sub-row a pixel, so each shows whichever of the pixel's two
//! block rows leaned further.) A sub-cell that is
//! exactly half and half goes to the brighter colour, a rule that does not
//! depend on which of the two a cell happens to call its first, so two
//! neighbouring cells always agree on where a shared edge runs.
//!
//! **Marks are frozen** (`View::feature`, the prototype's thresholds): a cell
//! holding a bright or saturated isolated sample, or a pixel of a one-pixel
//! line (axis-aligned or 45°), is emitted untouched, and such a pixel never
//! lends its colour either, so a pellet, a telegraph or a whisker can neither
//! lose a corner nor grow one. The ink pass and the damage digits are not in
//! this picture at all; `DetailFrame::overlay` carries them exact.
//!
//! A cell no rule touched is the plain `▀` with its own two colours, so the
//! frame is byte-identical to the half-block blit wherever nothing was
//! smoothed. Integer arithmetic only, no state between frames, and the scratch
//! (YUV and edge flags a pixel) is the caller's, so a frame allocates nothing.
//!
//! `xbr_sim.py` beside this file is the same pass in Python, for looking at a
//! rule change on a captured frame.

use super::{differs, edge_flags, luma, Encoded, Grid, Pair, E_D, E_R};
use crate::render::framebuffer::PixelBuffer;
use crate::render::palette::Rgb;

/// The reference's `eq`: two pixels are alike under this summed YUV distance.
const EQ: u32 = 155;

/// A sub-pixel's weights sum to this. Three blends of at most 7/8 divide it
/// exactly.
const BASE: u32 = 512;

/// A sample is **bright** over this peak channel, or **saturated** over
/// `SAT_PEAK` with a channel range over `SAT_RANGE` (the prototype's guard).
const BRIGHT_PEAK: i32 = 172;
const SAT_PEAK: i32 = 96;
const SAT_RANGE: i32 = 80;
/// Such a sample is isolated when at least three axial neighbours differ from
/// it by more than this in some channel.
const ISOLATED: i32 = 30;
/// A pixel is on a one-pixel line when both neighbours along an axis or a
/// diagonal are within `ALIGNED` of it (largest channel difference) and both
/// perpendicular neighbours are more than `ACROSS` away.
const ALIGNED: i32 = 16;
const ACROSS: i32 = 36;

/// Axial offsets by direction index: right, down, left, up.
const DIRS: [(isize, isize); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// For each of the reference's four `FILT2` calls: the direction its `PF` and
/// `PH` roles point in, and the sub-pixels (`row * 2 + col`) its `N1`, `N2` and
/// `N3` name.
const F_DIR: [usize; 4] = [0, 3, 2, 1];
const H_DIR: [usize; 4] = [1, 0, 3, 2];
const N1: [usize; 4] = [1, 0, 2, 3];
const N2: [usize; 4] = [2, 3, 1, 0];
const N3: [usize; 4] = [3, 1, 0, 2];

/// The 2 × 4 block rows each sub-cell row of a grid covers, as weights on the
/// 12-row mesh (a block row is three mesh rows).
const QUAD_BANDS: [[u32; 4]; 2] = [[3, 3, 0, 0], [0, 0, 3, 3]];
const SEXTANT_BANDS: [[u32; 4]; 3] = [[3, 1, 0, 0], [0, 2, 2, 0], [0, 0, 1, 3]];
const OCTANT_BANDS: [[u32; 4]; 4] = [[3, 0, 0, 0], [0, 3, 0, 0], [0, 0, 3, 0], [0, 0, 0, 3]];

/// The reference's RGB → YUV table entry, computed (it tabulates all 2²⁴).
#[inline]
fn yuv(c: Rgb) -> Rgb {
    let (r, g, b) = (i32::from(c.0), i32::from(c.1), i32::from(c.2));
    let (rg, bg) = (r - g, b - g);
    let u = (-169 * rg + 500 * bg) / 1000 + 128;
    let v = (500 * rg - 81 * bg) / 1000 + 128;
    let start = (-bg).max(-rg).max(0);
    let y = (299 * rg + 1000 * start + 114 * bg) / 1000 + (g - start);
    (y.clamp(0, 255) as u8, u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

/// The largest channel difference between two colours.
#[inline]
fn chan(a: Rgb, b: Rgb) -> i32 {
    (i32::from(a.0) - i32::from(b.0))
        .abs()
        .max((i32::from(a.1) - i32::from(b.1)).abs())
        .max((i32::from(a.2) - i32::from(b.2)).abs())
}

/// Is `a` the brighter of two colours? A total order, so it does not depend on
/// which of the two a cell met first.
#[inline]
fn brighter(a: Rgb, b: Rgb) -> bool {
    (luma(a), a) > (luma(b), b)
}

/// Rotate a role offset into the `k`-th `FILT2` call's frame.
#[inline]
fn rot(k: usize, dx: isize, dy: isize) -> (isize, isize) {
    match k {
        0 => (dx, dy),
        1 => (dy, -dx),
        2 => (-dx, -dy),
        _ => (-dy, dx),
    }
}

/// The picture with what the pass precomputes a pixel.
pub(super) struct View<'a> {
    s: &'a [Rgb],
    yuv: &'a [Rgb],
    flags: &'a [u8],
    w: usize,
    h: usize,
}

impl<'a> View<'a> {
    /// A view for the mark tests alone ([`View::bright_isolated`]): no YUV, no
    /// edge flags.
    pub(super) fn marks(src: &'a PixelBuffer) -> Self {
        View { s: src.pixels(), yuv: &[], flags: &[], w: src.w, h: src.h }
    }

    /// The pixel index at `(x + dx, y + dy)`, clamped to the picture.
    #[inline]
    fn at(&self, x: usize, y: usize, dx: isize, dy: isize) -> usize {
        let nx = (x as isize + dx).clamp(0, self.w as isize - 1) as usize;
        let ny = (y as isize + dy).clamp(0, self.h as isize - 1) as usize;
        ny * self.w + nx
    }

    /// Does `(x, y)` differ (by `EDGE`) from its axial neighbour `dir`?
    #[inline]
    fn edge(&self, x: usize, y: usize, dir: usize) -> bool {
        let i = y * self.w + x;
        match dir {
            0 => x + 1 < self.w && self.flags[i] & E_R != 0,
            1 => y + 1 < self.h && self.flags[i] & E_D != 0,
            2 => x > 0 && self.flags[i - 1] & E_R != 0,
            _ => y > 0 && self.flags[i - self.w] & E_D != 0,
        }
    }

    /// Can any corner of `(x, y)` lean? Only one whose two sides are edges.
    #[inline]
    fn active(&self, x: usize, y: usize) -> bool {
        let e = [self.edge(x, y, 0), self.edge(x, y, 1), self.edge(x, y, 2), self.edge(x, y, 3)];
        (e[0] && e[1]) || (e[1] && e[2]) || (e[2] && e[3]) || (e[3] && e[0])
    }

    /// A bright or saturated sample that stands alone: a pellet, a spark, a
    /// telegraph dot.
    pub(super) fn bright_isolated(&self, x: usize, y: usize) -> bool {
        let c = self.s[y * self.w + x];
        let hi = i32::from(c.0.max(c.1).max(c.2));
        let lo = i32::from(c.0.min(c.1).min(c.2));
        if !(hi > BRIGHT_PEAK || (hi > SAT_PEAK && hi - lo > SAT_RANGE)) {
            return false;
        }
        let far = DIRS
            .iter()
            .filter(|&&(dx, dy)| chan(c, self.s[self.at(x, y, dx, dy)]) > ISOLATED)
            .count();
        far >= 3
    }

    /// A gameplay mark by the picture's own evidence: [`View::bright_isolated`],
    /// or a pixel of a one-pixel line along an axis or a 45° diagonal.
    pub(super) fn feature(&self, x: usize, y: usize) -> bool {
        if self.bright_isolated(x, y) {
            return true;
        }
        let c = self.s[y * self.w + x];
        let d = |dx: isize, dy: isize| chan(c, self.s[self.at(x, y, dx, dy)]);
        [(1isize, 0isize), (0, 1), (1, 1), (1, -1)].iter().any(|&(dx, dy)| {
            d(dx, dy) < ALIGNED && d(-dx, -dy) < ALIGNED && d(-dy, dx) > ACROSS && d(dy, -dx) > ACROSS
        })
    }

    #[inline]
    fn df(&self, a: usize, b: usize) -> u32 {
        let (p, q) = (self.yuv[a], self.yuv[b]);
        (i32::from(p.0) - i32::from(q.0)).unsigned_abs()
            + (i32::from(p.1) - i32::from(q.1)).unsigned_abs()
            + (i32::from(p.2) - i32::from(q.2)).unsigned_abs()
    }

    #[inline]
    fn eq(&self, a: usize, b: usize) -> bool {
        self.df(a, b) < EQ
    }

    /// `xbr=2` for one pixel: `w[sub][dir]` is the share (of `BASE`) sub-pixel
    /// `sub` (`row * 2 + col`) takes from the axial neighbour `dir`; the rest
    /// is the pixel's own colour. All zero where nothing leaned.
    fn pixel(&self, x: usize, y: usize) -> [[u32; 4]; 4] {
        let mut w = [[0u32; 4]; 4];
        let e = y * self.w + x;
        for k in 0..4 {
            let (fd, hd) = (F_DIR[k], H_DIR[k]);
            // The reference's `PE != PH && PE != PF`, as contrast.
            if !(self.edge(x, y, fd) && self.edge(x, y, hd)) {
                continue;
            }
            let n = |dx: isize, dy: isize| {
                let (rx, ry) = rot(k, dx, dy);
                self.at(x, y, rx, ry)
            };
            let (f, h) = (n(1, 0), n(0, 1));
            let (i, g, c) = (n(1, 1), n(-1, 1), n(1, -1));
            let (d, b) = (n(-1, 0), n(0, -1));
            let (f4, i4, h5, i5) = (n(2, 0), n(2, 1), n(0, 2), n(1, 2));
            let ee = self.df(e, c) + self.df(e, g) + self.df(i, h5) + self.df(i, f4) + (self.df(h, f) << 2);
            let ii = self.df(h, d) + self.df(h, i5) + self.df(f, i4) + self.df(f, b) + (self.df(e, i) << 2);
            if ee > ii {
                continue;
            }
            // A stair, not a corner: the pixel's colour carries on along the
            // diagonal on at least one side. (A rectangle keeps its corners.)
            if differs(self.s[e], self.s[c]) && differs(self.s[e], self.s[g]) {
                continue;
            }
            let take_f = self.df(e, f) <= self.df(e, h);
            let (donor, other, dir) = if take_f { (f, h, fd) } else { (h, f, hd) };
            // A protected mark never lends its colour, and neither does the
            // other side of the corner when it is the same colour as a mark.
            if self.feature(donor % self.w, donor / self.w)
                || (chan(self.s[donor], self.s[other]) < ALIGNED
                    && self.feature(other % self.w, other / self.w))
            {
                continue;
            }
            let (n1, n2, n3) = (N1[k], N2[k], N3[k]);
            let blend = |w: &mut [[u32; 4]; 4], sub: usize, num: u32, den: u32| {
                for q in w[sub].iter_mut() {
                    *q -= *q * num / den;
                }
                w[sub][dir] += BASE * num / den;
            };
            let extended = ee < ii
                && ((!self.eq(f, b) && !self.eq(h, d))
                    || (self.eq(e, i) && !self.eq(f, i4) && !self.eq(h, i5))
                    || self.eq(e, g)
                    || self.eq(e, c));
            if !extended {
                blend(&mut w, n3, 1, 2);
                continue;
            }
            let (ke, ki) = (self.df(f, g), self.df(h, c));
            let s = self.s;
            let left = (ke << 1) <= ki && differs(s[e], s[g]) && differs(s[d], s[g]);
            let up = ke >= (ki << 1) && differs(s[e], s[c]) && differs(s[b], s[c]);
            match (left, up) {
                (true, true) => {
                    blend(&mut w, n3, 7, 8);
                    blend(&mut w, n2, 1, 4);
                    w[n1] = w[n2];
                }
                (true, false) => {
                    blend(&mut w, n3, 3, 4);
                    blend(&mut w, n2, 1, 4);
                }
                (false, true) => {
                    blend(&mut w, n3, 3, 4);
                    blend(&mut w, n1, 1, 4);
                }
                (false, false) => blend(&mut w, n3, 1, 2),
            }
        }
        w
    }
}

/// The pattern of one grid nearest the block's coverage, and how far it is.
/// `cov[row * 2 + col]` is the second colour's share (of `BASE`) of each of the
/// block's eight sub-pixels.
fn reduce(cov: &[u32; 8], bands: &[[u32; 4]], first_wins_ties: bool) -> (u8, u32) {
    let (mut pattern, mut err) = (0u8, 0u32);
    for (r, band) in bands.iter().enumerate() {
        let total = band.iter().sum::<u32>() * BASE;
        for col in 0..2 {
            let c: u32 = (0..4).map(|j| band[j] * cov[j * 2 + col]).sum();
            if c * 2 > total || (c * 2 == total && !first_wins_ties) {
                pattern |= 1 << (r * 2 + col);
                err += total - c;
            } else {
                err += c;
            }
        }
    }
    (pattern, err)
}

/// One cell of the frame: the plain half block unless a corner leaned.
fn cell(v: &View, cx: usize, cy: usize, grid: Grid) -> Encoded {
    let (ty, by) = (cy * 2, cy * 2 + 1);
    let (top, bottom) = (v.s[ty * v.w + cx], v.s[by * v.w + cx]);
    let plain = Encoded { glyph: '▀', fg: top, bg: bottom };
    if !(v.active(cx, ty) || v.active(cx, by)) {
        return plain;
    }
    if v.feature(cx, ty) || v.feature(cx, by) {
        return plain;
    }
    let halves = [v.pixel(cx, ty), v.pixel(cx, by)];
    if halves == [[[0u32; 4]; 4]; 2] {
        return plain;
    }
    let mut pair = Pair { x: top, y: None };
    let Some(bottom_second) = pair.class(bottom) else { return plain };
    let mut cov = [0u32; 8];
    for (half, (w, own, y)) in [(&halves[0], false, ty), (&halves[1], bottom_second, by)]
        .into_iter()
        .enumerate()
    {
        for (sub, shares) in w.iter().enumerate() {
            // Only a neighbour that lends something is classified, in a fixed
            // order; one of a third colour lends nothing.
            let (mut second, mut rest) = (0u32, BASE);
            for (&share, &(dx, dy)) in shares.iter().zip(DIRS.iter()) {
                if share == 0 {
                    continue;
                }
                let Some(is_second) = pair.class(v.s[v.at(cx, y, dx, dy)]) else { continue };
                rest -= share;
                if is_second {
                    second += share;
                }
            }
            if own {
                second += rest;
            }
            cov[half * 4 + sub] = second;
        }
    }
    let Some(y) = pair.y else { return plain };
    let x = pair.x;
    let first_wins = brighter(x, y);
    let original: u8 = if bottom_second { 0b1100 } else { 0 };
    if grid == Grid::Quad {
        // One sub-row a pixel: it shows whichever of the pixel's two block
        // rows leaned further (an average of the two would never reach half).
        let mut p = 0u8;
        for (half, own) in [(0usize, false), (1, bottom_second)] {
            for col in 0..2 {
                let (a, b) = (cov[half * 4 + col], cov[half * 4 + 2 + col]);
                let c = if own { a.min(b) } else { a.max(b) };
                if c * 2 > BASE || (c * 2 == BASE && !first_wins) {
                    p |= 1 << (half * 2 + col);
                }
            }
        }
        return emit(p, original, Grid::Quad, x, y).unwrap_or(plain);
    }
    let (mut best_grid, (mut best, mut best_err)) =
        (Grid::Quad, reduce(&cov, &QUAD_BANDS, first_wins));
    let finer: &[Grid] = match grid {
        Grid::Quad => &[],
        Grid::Sextant => &[Grid::Sextant],
        Grid::Octant => &[Grid::Sextant, Grid::Octant],
    };
    for &g in finer {
        let (p, err) = match g {
            Grid::Sextant => reduce(&cov, &SEXTANT_BANDS, first_wins),
            _ => reduce(&cov, &OCTANT_BANDS, first_wins),
        };
        // Strictly nearer, or as near where the quadrants would change nothing.
        if err < best_err || (err == best_err && best_grid == Grid::Quad && best == original) {
            (best_grid, best, best_err) = (g, p, err);
        }
    }
    emit(best, original, best_grid, x, y).unwrap_or(plain)
}

/// The cell for `pattern` on `grid` (bit set = the second colour `y`), or
/// `None` when it is the quadrant pattern the cell already showed.
fn emit(pattern: u8, original: u8, grid: Grid, x: Rgb, y: Rgb) -> Option<Encoded> {
    if grid == Grid::Quad && pattern == original {
        return None;
    }
    let full = ((1u16 << grid.subpixels()) - 1) as u8;
    if pattern == 0 || pattern == full {
        let c = if pattern == 0 { x } else { y };
        return Some(Encoded { glyph: '▀', fg: c, bg: c });
    }
    // The brighter colour is the foreground, as in `encode`.
    Some(if luma(y) >= luma(x) {
        Encoded { glyph: grid.glyph(pattern), fg: y, bg: x }
    } else {
        Encoded { glyph: grid.glyph(!pattern & full), fg: x, bg: y }
    })
}

/// The whole pass: `src` is the half-block picture (`cols × 2·rows`), `grid`
/// the finest grid the terminal draws, and `out` gets one cell a terminal cell
/// — every one of them, the untouched ones as the plain half block. `yuv` and
/// `flags` are scratch the caller keeps between frames.
pub(super) fn xbr(
    src: &PixelBuffer,
    cols: usize,
    rows: usize,
    grid: Grid,
    yuv_buf: &mut Vec<Rgb>,
    flags: &mut Vec<u8>,
    out: &mut Vec<Encoded>,
) -> bool {
    out.clear();
    if cols == 0 || rows == 0 || src.w != cols || src.h != rows * 2 {
        return false;
    }
    edge_flags(src, flags);
    yuv_buf.clear();
    yuv_buf.extend(src.pixels().iter().map(|&c| yuv(c)));
    let v = View { s: src.pixels(), yuv: yuv_buf.as_slice(), flags: flags.as_slice(), w: src.w, h: src.h };
    out.reserve(cols * rows);
    for cy in 0..rows {
        for cx in 0..cols {
            out.push(cell(&v, cx, cy, grid));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rgb = (250, 240, 60);
    const B: Rgb = (20, 24, 40);

    fn run(src: &PixelBuffer, grid: Grid) -> Vec<Encoded> {
        let (mut yuv_buf, mut flags, mut out) = (Vec::new(), Vec::new(), Vec::new());
        assert!(xbr(src, src.w, src.h / 2, grid, &mut yuv_buf, &mut flags, &mut out));
        assert_eq!(out.len(), src.w * (src.h / 2));
        out
    }

    /// The frame `Smooth::Off` draws: one `▀` a cell, its two pixels' colours.
    fn off(src: &PixelBuffer) -> Vec<Encoded> {
        let mut out = Vec::new();
        for cy in 0..src.h / 2 {
            for cx in 0..src.w {
                out.push(Encoded {
                    glyph: '▀',
                    fg: src.pixel_at(cx, cy * 2),
                    bg: src.pixel_at(cx, cy * 2 + 1),
                });
            }
        }
        out
    }

    fn stair(run_len: usize) -> PixelBuffer {
        let mut src = PixelBuffer::new(32, 24);
        for y in 0..24 {
            for x in 0..32 {
                src.plot(x as i32, y as i32, if y >= 6 + x / run_len { B } else { A });
            }
        }
        src
    }

    #[test]
    fn the_yuv_matches_the_reference_at_known_points() {
        assert_eq!(yuv((0, 0, 0)), (0, 128, 128));
        assert_eq!(yuv((255, 255, 255)), (255, 128, 128));
        // Pure red: y = 299 * 255 / 1000, u = -169 * 255 / 1000 + 128, v = 500 * 255 / 1000 + 128.
        assert_eq!(yuv((255, 0, 0)), (76, 85, 255));
        assert_eq!(yuv((0, 0, 255)), (29, 255, 108));
    }

    #[test]
    fn a_flat_field_and_axis_aligned_rectangles_are_the_half_block_blit() {
        let mut flat = PixelBuffer::new(16, 12);
        flat.clear(B);
        let mut rects = PixelBuffer::new(32, 24);
        rects.clear(B);
        rects.rect_fill(3, 3, 9, 6, A);
        rects.rect_fill(16, 5, 4, 11, A);
        rects.rect_fill(22, 2, 8, 2, (200, 60, 60));
        // A lit gradient: every pixel differs from its neighbours, none by an edge.
        let mut ramp = PixelBuffer::new(32, 24);
        for y in 0..24 {
            for x in 0..32 {
                ramp.plot(x, y, ((40 + x * 3) as u8, (44 + y * 2) as u8, (52 + x + y) as u8));
            }
        }
        for (name, src) in [("flat", &flat), ("rects", &rects), ("ramp", &ramp)] {
            for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
                assert!(run(src, grid) == off(src), "{name} on {grid:?} is not the plain blit");
            }
        }
    }

    #[test]
    fn a_stair_is_smoothed_with_only_its_two_colours() {
        for (run_len, name) in [(1, "45 degree"), (2, "2:1")] {
            let src = stair(run_len);
            for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
                let cells = run(&src, grid);
                let plain = off(&src);
                let shaped = cells.iter().filter(|e| e.glyph != '▀').count();
                assert!(shaped >= 4, "{name} on {grid:?}: only {shaped} cells changed shape");
                for (i, e) in cells.iter().enumerate() {
                    assert!(
                        (e.fg == A || e.fg == B) && (e.bg == A || e.bg == B),
                        "{name} on {grid:?}: cell {i} shows a colour the picture did not hold: {e:?}"
                    );
                    // A cell away from the stair is untouched, byte for byte.
                    if plain[i].fg == plain[i].bg {
                        let (cx, cy) = (i % 32, i / 32);
                        let near = (0..24).any(|y| {
                            (0..32).any(|x| {
                                x.abs_diff(cx) <= 1
                                    && y / 2 == cy
                                    && src.pixel_at(x, y) != plain[i].fg
                            })
                        });
                        assert!(near || *e == plain[i], "{name} on {grid:?}: far cell {i} changed");
                    }
                }
            }
        }
    }

    #[test]
    fn a_pellet_ring_and_a_one_pixel_line_survive_exactly() {
        let floor = (40, 44, 52);
        let hot = (255, 120, 160);
        let mut src = PixelBuffer::new(40, 32);
        src.clear(floor);
        // A ring of single-pixel pellets.
        for &(dx, dy) in &[(0, -6), (4, -4), (6, 0), (4, 4), (0, 6), (-4, 4), (-6, 0), (-4, -4)] {
            src.plot(12 + dx, 14 + dy, hot);
        }
        // One-pixel lines: a 45° one, a 2:1 stair line, a level one, an upright one.
        for i in 0..10 {
            src.plot(24 + i, 4 + i, (255, 236, 120));
            src.plot(22 + i, 18 + i / 2, (255, 236, 120));
        }
        src.line(2, 28, 16, 28, (230, 230, 240));
        src.line(37, 2, 37, 26, (230, 230, 240));
        for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
            assert!(run(&src, grid) == off(&src), "{grid:?}: a mark was smoothed");
        }
    }

    #[test]
    fn a_mark_does_not_lend_its_colour_to_a_corner_beside_it() {
        // A stair whose step carries a lone bright pixel: the stair's own
        // corners may lean, but no cell may gain the pellet's colour.
        let hot = (255, 60, 200);
        let mut src = stair(1);
        src.plot(10, 12, hot);
        let cells = run(&src, Grid::Octant);
        let plain = off(&src);
        for (i, e) in cells.iter().enumerate() {
            let had = plain[i].fg == hot || plain[i].bg == hot;
            let has = e.fg == hot || e.bg == hot;
            assert_eq!(had, has, "cell {i}: {e:?}");
        }
    }

    #[test]
    fn the_same_picture_always_yields_the_same_cells() {
        let (one, two) = (stair(1), stair(2));
        let fresh = run(&one, Grid::Sextant);
        let (mut yuv_buf, mut flags, mut out) = (Vec::new(), Vec::new(), Vec::new());
        for src in [&one, &two, &one, &one] {
            assert!(xbr(src, 32, 12, Grid::Sextant, &mut yuv_buf, &mut flags, &mut out));
        }
        assert!(out == fresh, "the pass remembered an earlier picture");
        // A picture of the wrong size is refused and leaves no stale answer.
        assert!(!xbr(&one, 30, 12, Grid::Sextant, &mut yuv_buf, &mut flags, &mut out));
        assert!(out.is_empty());
    }

    #[test]
    fn the_sub_cell_fit_is_a_majority_with_ties_to_the_brighter_colour() {
        // Untouched top / bottom: the quadrants hold it exactly, the sextants
        // split the middle row.
        let half = [0, 0, 0, 0, BASE, BASE, BASE, BASE];
        assert_eq!(reduce(&half, &QUAD_BANDS, true), (0b1100, 0));
        assert_eq!(reduce(&half, &SEXTANT_BANDS, true), (0b11_00_00, 4 * BASE));
        assert_eq!(reduce(&half, &SEXTANT_BANDS, false), (0b11_11_00, 4 * BASE));
        assert_eq!(reduce(&half, &OCTANT_BANDS, true), (0b1111_0000, 0));
        // A half-leaned corner on the octant grid goes to the brighter colour.
        let corner = [0, 0, 0, BASE / 2, BASE, BASE, BASE, BASE];
        assert_eq!(reduce(&corner, &OCTANT_BANDS, true).0, 0b1111_0000);
        assert_eq!(reduce(&corner, &OCTANT_BANDS, false).0, 0b1111_1000);
    }
}
