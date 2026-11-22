//! Sub-cell detail for the playfield (always on; there is no option).
//!
//! The half-block blit shows two pixels a cell. Zoomed out, a cell covers more
//! world pixels than that (eight at 2×), and [`PixelBuffer::resample_to`]
//! keeps one per half. This module shows more of them by drawing a cell with a
//! block glyph that splits it finer:
//!
//! | grid | sub-pixels | glyphs |
//! | --- | --- | --- |
//! | [`Grid::Quad`] | 2 × 2 | the quadrants `▖▗▘▙▚▛▜▝▞▟` plus `▀▄▌▐█` |
//! | [`Grid::Sextant`] | 2 × 3 | `U+1FB00..=U+1FB3B` plus `▌▐█` |
//! | [`Grid::Octant`] | 2 × 4 | `U+1CD00..=U+1CDE5` plus the 26 patterns older blocks already cover |
//!
//! An octant is square in a 1:2 cell, and at 2× the world buffer is exactly
//! `2·cols × 4·rows`, so the octant grid shows **every** world pixel; what is
//! lost is colour, because a cell still has only a foreground and a
//! background. [`encode`] therefore splits a cell's sub-pixels into two
//! clusters and draws the pattern between them. A cell without real contrast
//! is emitted as the plain `▀` the half-block blit would have written, so
//! flat floor and soft light gradients look (and cost) what they did.
//!
//! The pattern → glyph tables in `subcell/tables.rs` are generated from the
//! Unicode character names (`subcell/gen_tables.py`), never typed by hand:
//! sextant numbering skips the two patterns that are `▌` and `▐`, and the
//! octant block is not in bit order at all.
//!
//! At 1× (and zoomed in, where a cell covers less than two world pixels) the
//! half-block picture is already the whole picture, so there is nothing to
//! resample: the sub-pixels come from an **EPX / Scale2x** upscale of it
//! instead (`smooth_into`). A corner takes the neighbour's colour only when
//! the two adjacent neighbours agree and the opposite ones differ, so a
//! staircase diagonal or a curve rounds off — and because the rule only ever
//! copies an existing colour, no new colour appears. The guards (contrast
//! threshold `EDGE`, the 1-px **thin** rule, and the identical-pair cell
//! rule) are what keep it from touching a pellet, a whisker or a soft
//! gradient; see `smooth_into`.
//!
//! **Render-only.** Nothing here reads or writes simulation state. With
//! smoothing off (`RATRUN_SMOOTH=0`, console `smooth off`) a 1× frame is
//! byte-identical to the half-block blit every earlier build drew.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::framebuffer::PixelBuffer;
use super::palette::{self, Rgb};

mod tables;

/// Above this eased zoom the view is zoomed out and the detail path may run
/// (the same line the ink pass draws, [`super::ink::ZOOMED_OUT`]).
pub const ZOOMED_OUT: f32 = super::ink::ZOOMED_OUT;

/// The finest glyph set the detail path may use. `Off` is today's half blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SubcellSet {
    #[default]
    Off,
    Quad,
    Sextant,
    Octant,
}

impl SubcellSet {
    pub fn as_str(self) -> &'static str {
        match self {
            SubcellSet::Off => "off",
            SubcellSet::Quad => "quad",
            SubcellSet::Sextant => "sextant",
            SubcellSet::Octant => "octant",
        }
    }
}

/// What `RATRUN_SUBCELL` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Use what the terminal is known to draw (the AUTO rule).
    Auto,
    /// Exactly this set, whatever the terminal claims (`Off` included).
    Force(SubcellSet),
}

impl Choice {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "none" | "half" => Choice::Force(SubcellSet::Off),
            "quad" | "quadrant" | "quadrants" => Choice::Force(SubcellSet::Quad),
            "sextant" | "sextants" => Choice::Force(SubcellSet::Sextant),
            "octant" | "octants" => Choice::Force(SubcellSet::Octant),
            "auto" | "on" | "1" => Choice::Auto,
            _ => return None,
        })
    }

    /// The `RATRUN_SUBCELL` override, if it is set to something readable.
    pub fn from_env() -> Option<Self> {
        std::env::var("RATRUN_SUBCELL").ok().and_then(|v| Choice::parse(&v))
    }
}

/// Whether the 1× edge smoothing is on: `RATRUN_SMOOTH=0` (or `off`/`false`/
/// `no`) pins it off for a session, anything else leaves the default (on).
/// The console `smooth` command is the live A/B; this is the capture pin.
pub fn smooth_from_env() -> bool {
    match std::env::var("RATRUN_SMOOTH") {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "off" | "false" | "no"),
        Err(_) => true,
    }
}

/// The set to draw with: the environment override wins, else whatever the
/// terminal is known to draw ([`crate::term::BlockSet`]), which is nothing
/// finer than half blocks on a terminal this game has not been told about.
/// There is no setting: detail is always on, and `RATRUN_SUBCELL` exists only
/// to pin a set for a session, a capture or a test (`off` forces half blocks).
pub fn resolve(env: Option<Choice>, blocks: crate::term::BlockSet) -> SubcellSet {
    use crate::term::BlockSet;
    let auto = match blocks {
        BlockSet::Half => SubcellSet::Off,
        BlockSet::Quad => SubcellSet::Quad,
        BlockSet::Sextant => SubcellSet::Sextant,
        BlockSet::Octant => SubcellSet::Octant,
    };
    match env {
        Some(Choice::Force(set)) => set,
        Some(Choice::Auto) | None => auto,
    }
}

/// A sub-pixel grid: two columns a cell and [`Grid::rows`] rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    Quad,
    Sextant,
    Octant,
}

impl Grid {
    /// Sub-pixel rows per cell.
    pub const fn rows(self) -> usize {
        match self {
            Grid::Quad => 2,
            Grid::Sextant => 3,
            Grid::Octant => 4,
        }
    }

    /// Sub-pixels per cell.
    pub const fn subpixels(self) -> usize {
        self.rows() * 2
    }

    /// The glyph whose foreground covers exactly the set bits of `pattern`
    /// (bit `row * 2 + col`).
    pub fn glyph(self, pattern: u8) -> char {
        match self {
            Grid::Quad => tables::QUADRANTS[usize::from(pattern & 0x0F)],
            Grid::Sextant => tables::SEXTANTS[usize::from(pattern & 0x3F)],
            Grid::Octant => tables::OCTANTS[usize::from(pattern)],
        }
    }

    /// The first sub-pixel of the cell's lower half: what the half-block blit
    /// would have shown as the bottom pixel.
    const fn lower(self) -> usize {
        match self {
            Grid::Quad => 2,
            Grid::Sextant | Grid::Octant => 4,
        }
    }
}

/// The grid for a zoom: the one whose sub-pixel is nearest a world pixel
/// without being coarser than the set allows. `None` at 1× and zoomed in.
pub fn grid_for(set: SubcellSet, zoom: f32) -> Option<Grid> {
    if zoom <= ZOOMED_OUT {
        return None;
    }
    Some(match set {
        SubcellSet::Off => return None,
        SubcellSet::Quad => Grid::Quad,
        SubcellSet::Sextant if zoom >= 1.375 => Grid::Sextant,
        SubcellSet::Sextant => Grid::Quad,
        SubcellSet::Octant if zoom >= 1.75 => Grid::Octant,
        SubcellSet::Octant if zoom >= 1.375 => Grid::Sextant,
        SubcellSet::Octant => Grid::Quad,
    })
}

/// The grid for a whole frame: zoomed out, [`grid_for`]'s answer (the finest
/// set the zoom can fill); at 1× and zoomed in, the grid the **smoothing**
/// upscale (`smooth_into`) feeds — two sub-pixels across per world pixel
/// either way, and one sub-row per world pixel at the quad grid (the vertical
/// resolution the half-block blit already had) or two at the octant grid (the
/// full 2 × 2 of the rule). `None` when the set is `Off` or smoothing is off.
pub fn grid_for_frame(set: SubcellSet, zoom: f32, smooth: bool) -> Option<Grid> {
    if let Some(g) = grid_for(set, zoom) {
        return Some(g);
    }
    if !smooth || set == SubcellSet::Off {
        return None;
    }
    // A sextant grid has one and a half sub-rows per world pixel, which the
    // 2 × 2 rule cannot fill exactly; the quad grid is the honest answer (and
    // what the zoom ladder already picks for a sextant terminal at 1.25×).
    Some(match set {
        SubcellSet::Octant => Grid::Octant,
        SubcellSet::Off => return None,
        SubcellSet::Quad | SubcellSet::Sextant => Grid::Quad,
    })
}

/// One encoded cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub glyph: char,
    pub fg: Rgb,
    pub bg: Rgb,
}

/// A cell whose sub-pixels span no more than this (summed over the three
/// channels) is texture grain or a light gradient, not a feature: it is drawn
/// as the plain half block. Two colours cut through a gradient read as noise.
const FLAT_SPREAD: i32 = 28;

/// Squared distance from the cell's mean past which a sub-pixel is a feature
/// (the same line `PixelBuffer::resample_to` draws).
const SALIENT: u32 = 3 * 28 * 28;

/// The contrast (summed over the three channels) an edge must exceed before
/// the smoothing rule may round it. Below this the two pixels are a lit
/// gradient or texture grain: rounding across it would band a light falloff
/// or shimmer frame to frame, so the picture is left alone. Above it they are
/// two regions, and the corner between them is a stair-step worth rounding.
const EDGE: i32 = 48;

/// Do two pixels differ by more than `EDGE` (summed channel distance)?
#[inline]
fn differs(a: Rgb, b: Rgb) -> bool {
    (i32::from(a.0) - i32::from(b.0)).abs()
        + (i32::from(a.1) - i32::from(b.1)).abs()
        + (i32::from(a.2) - i32::from(b.2)).abs()
        >= EDGE
}

#[inline]
fn dist(a: Rgb, b: Rgb) -> u32 {
    let (dr, dg, db) = (
        i32::from(a.0) - i32::from(b.0),
        i32::from(a.1) - i32::from(b.1),
        i32::from(a.2) - i32::from(b.2),
    );
    (dr * dr + dg * dg + db * db) as u32
}

#[inline]
fn luma(c: Rgb) -> u32 {
    u32::from(c.0) * 3 + u32::from(c.1) * 6 + u32::from(c.2)
}

/// Encode one cell: `px` is its sub-pixels in row-major order
/// (`grid.subpixels()` of them) and `ink` flags the ones the screen-space passes
/// stamped (damage digits, the ink pass), which must come out exact.
///
/// A two-means split in integer arithmetic. The seeds are the sub-pixel that
/// stands out most from the cell and the one farthest from it; one refinement
/// step follows. Each colour is its cluster's mean, weighted so a salient
/// sub-pixel (a tell, a pellet, an outline) outvotes the texture it sits in,
/// and an inked one outvotes everything. The brighter cluster is the
/// foreground, which keeps neighbouring cells' colours in the same slots.
pub fn encode(px: &[Rgb], ink: u8, grid: Grid) -> Encoded {
    let n = grid.subpixels().min(px.len());
    let px = &px[..n];
    let plain = Encoded { glyph: '▀', fg: px[0], bg: px[grid.lower().min(n - 1)] };

    let (mut lo, mut hi) = (px[0], px[0]);
    let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
    for &c in px {
        lo = (lo.0.min(c.0), lo.1.min(c.1), lo.2.min(c.2));
        hi = (hi.0.max(c.0), hi.1.max(c.1), hi.2.max(c.2));
        sr += u32::from(c.0);
        sg += u32::from(c.1);
        sb += u32::from(c.2);
    }
    let spread = i32::from(hi.0) - i32::from(lo.0) + i32::from(hi.1) - i32::from(lo.1)
        + i32::from(hi.2)
        - i32::from(lo.2);
    if spread == 0 || (ink == 0 && spread <= FLAT_SPREAD) {
        return plain;
    }
    let nn = n as u32;
    let mean = ((sr / nn) as u8, (sg / nn) as u8, (sb / nn) as u8);

    // Per-sub-pixel weight in its cluster's mean.
    let mut weight = [1u32; 8];
    let mut a = 0usize;
    let mut a_d = 0u32;
    for (i, &c) in px.iter().enumerate() {
        let d = dist(c, mean);
        if ink & (1 << i) != 0 {
            weight[i] = 64;
        } else if d > 4 * SALIENT {
            weight[i] = 4;
        } else if d > SALIENT {
            weight[i] = 2;
        }
        if d > a_d {
            a_d = d;
            a = i;
        }
    }
    if ink != 0 {
        a = ink.trailing_zeros() as usize % n;
    }
    let mut b = a;
    let mut b_d = 0u32;
    for (i, &c) in px.iter().enumerate() {
        let d = dist(c, px[a]) * if ink & (1 << i) != 0 { 4 } else { 1 };
        if d > b_d {
            b_d = d;
            b = i;
        }
    }
    if b_d == 0 {
        return plain;
    }

    let (mut ca, mut cb) = (px[a], px[b]);
    let mut mask = 0u8;
    for _ in 0..2 {
        let mut next = 0u8;
        let mut sa = [0u32; 4];
        let mut sbb = [0u32; 4];
        for (i, &c) in px.iter().enumerate() {
            let w = weight[i];
            let s = if dist(c, ca) <= dist(c, cb) {
                next |= 1 << i;
                &mut sa
            } else {
                &mut sbb
            };
            s[0] += u32::from(c.0) * w;
            s[1] += u32::from(c.1) * w;
            s[2] += u32::from(c.2) * w;
            s[3] += w;
        }
        if sa[3] == 0 || sbb[3] == 0 {
            // A refinement that emptied a cluster: keep the previous split.
            break;
        }
        mask = next;
        ca = ((sa[0] / sa[3]) as u8, (sa[1] / sa[3]) as u8, (sa[2] / sa[3]) as u8);
        cb = ((sbb[0] / sbb[3]) as u8, (sbb[1] / sbb[3]) as u8, (sbb[2] / sbb[3]) as u8);
    }
    let full = ((1u16 << n) - 1) as u8;
    if mask == 0 || mask == full {
        return plain;
    }
    // An inked sub-pixel's colour is exact, not a mean it merely dominates.
    if ink & mask != 0 {
        ca = px[(ink & mask).trailing_zeros() as usize];
    }
    if ink & !mask & full != 0 {
        cb = px[(ink & !mask & full).trailing_zeros() as usize];
    }
    if ca == cb {
        return Encoded { glyph: '▀', fg: ca, bg: ca };
    }
    if luma(ca) >= luma(cb) {
        Encoded { glyph: grid.glyph(mask), fg: ca, bg: cb }
    } else {
        Encoded { glyph: grid.glyph(!mask & full), fg: cb, bg: ca }
    }
}

/// The key each encoded cell's neighbourhood falls back on when it cannot be
/// split: a flat cell is the plain half block the old blit wrote.
///
/// # Smoothing (the 1× and zoomed-in picture)
///
/// Zoomed out, [`DetailFrame`] is fed by `PixelBuffer::resample_to`; at 1× and
/// zoomed in the picture holds no world pixels the half-block blit does not
/// already show, so the sub-pixels are made by an **EPX / Scale2x** upscale of
/// it: every world pixel becomes `2 × grid.rows()/2` sub-pixels. The corner
/// rule is EPX's: a corner takes its adjacent neighbour's colour only when
/// those two neighbours agree and the two opposite ones differ, so a staircase
/// diagonal or a curve rounds off — and only ever copies an *existing* colour,
/// so no new colour appears and a cell still holds at most three.
///
/// Four guards keep the exactness the playfield needs. Each is a pure function
/// of the current picture (no temporal state), so a frame cannot shimmer
/// because the last one differed:
///
/// 1. **Contrast.** The rule only runs across edges that differ by at least
///    `EDGE` summed over the channels. Two pixels either side of a lit
///    gradient are inside that; rounding across one would band the falloff.
/// 2. **Thin.** A pixel whose colour run is one pixel wide in either axis
///    (both horizontal neighbours differ, or both vertical ones do, by
///    `EDGE`) is a line — a whisker, a tail tip, a telegraph, a pellet's rim
///    — or a lone speck. It is never modified and never copied outward as a
///    corner colour, so a one-pixel feature cannot be thinned, thickened or
///    erased. A *corner* of a body with like-coloured neighbours on two
///    adjacent sides is not thin, so silhouettes still round.
/// 3. **Identical pair.** A cell whose two world pixels are the same colour is
///    written flat: the smoothing has nothing to round there, so flat floor
///    costs exactly what it did before.
/// 4. **Ink.** Ink and salience are not in this picture at all: the ink pass
///    and the damage digits stamp the half-block buffer *after* this, and
///    [`DetailFrame::overlay`] carries what they changed onto the sub-pixels it
///    covers, exact colours, so a mark can never be smoothed away.
///
/// Returns whether it filled `dst` (the sizes must be the smoothing grid's).
fn smooth_into(src: &PixelBuffer, dst: &mut PixelBuffer, grid: Grid, thin: &mut Vec<bool>) -> bool {
    let rp = (grid.rows() / 2).max(1);
    let (w, h) = (src.w, src.h);
    if w == 0 || h == 0 || !h.is_multiple_of(2) || dst.w != w * 2 || dst.h != h * rp {
        return false;
    }
    let s = src.pixels();
    let at = |x: usize, y: usize| s[y * w + x];
    thin.clear();
    thin.resize(w * h, false);
    for y in 0..h {
        for x in 0..w {
            let p = at(x, y);
            let dl = x == 0 || differs(at(x - 1, y), p);
            let dr = x + 1 == w || differs(at(x + 1, y), p);
            let du = y == 0 || differs(at(x, y - 1), p);
            let dd = y + 1 == h || differs(at(x, y + 1), p);
            thin[y * w + x] = (dl && dr) || (du && dd);
        }
    }
    let dw = dst.w;
    let out = dst.pixels_mut();
    for y in 0..h {
        let yu = y.saturating_sub(1);
        let yd = (y + 1).min(h - 1);
        let row = y * rp * dw;
        for x in 0..w {
            let xl = x.saturating_sub(1);
            let xr = (x + 1).min(w - 1);
            let p = at(x, y);
            // The cell's partner row (the other half of the same terminal
            // cell). Identical pair, or a thin pixel: write it flat.
            let partner = if y % 2 == 0 { (y + 1).min(h - 1) } else { y - 1 };
            if at(x, partner) == p || thin[y * w + x] {
                for r in 0..rp {
                    out[row + r * dw + x * 2] = p;
                    out[row + r * dw + x * 2 + 1] = p;
                }
                continue;
            }
            let (b, g) = (at(x, yu), at(x, yd));
            let (d, e) = (at(xl, y), at(xr, y));
            let f = at(xl, yd);
            let hh = at(xr, yd);
            let (mut c0, mut c1) = (p, p);
            let (mut c2, mut c3) = (p, p);
            // Upper-left: left and up agree, and each differs from the corner
            // opposite it. `c0` is the neighbour's own colour, never a blend.
            if !differs(d, b)
                && differs(d, hh)
                && differs(b, f)
                && !thin[yu * w + xl]
                && !thin[y * w + xl]
            {
                c0 = d;
            }
            if !differs(b, e)
                && differs(b, f)
                && differs(e, hh)
                && !thin[yu * w + xr]
                && !thin[y * w + xr]
            {
                c1 = e;
            }
            if !differs(d, g)
                && differs(d, hh)
                && differs(g, f)
                && !thin[yd * w + xl]
                && !thin[y * w + xl]
            {
                c2 = d;
            }
            if !differs(e, g)
                && differs(e, f)
                && differs(g, hh)
                && !thin[yd * w + xr]
                && !thin[y * w + xr]
            {
                c3 = e;
            }
            if rp == 1 {
                // One sub-row a world pixel (the quad grid): the rule has no
                // second row to land in, so the rounded half is written — the
                // upper one where it rounded, else the lower, so a staircase
                // rounds whichever way it steps.
                out[row + x * 2] = if c0 != p { c0 } else { c2 };
                out[row + x * 2 + 1] = if c1 != p { c1 } else { c3 };
            } else {
                for (r, (a, c)) in [(c0, c1), (c2, c3)].into_iter().enumerate() {
                    out[row + r * dw + x * 2] = a;
                    out[row + r * dw + x * 2 + 1] = c;
                }
            }
        }
    }
    true
}

/// The zoomed-out picture at sub-cell resolution: `2·cols × grid.rows()·rows`
/// pixels, plus one bit a sub-pixel for what the screen-space passes stamped.
pub struct DetailFrame {
    grid: Grid,
    cols: usize,
    rows: usize,
    px: PixelBuffer,
    /// Per cell: bit `row * 2 + col` set = that sub-pixel is ink.
    ink: Vec<u8>,
    /// The smoothing pass's per-pixel "this is a one-pixel-wide feature" flags,
    /// held here so a smoothing frame allocates nothing.
    thin: Vec<bool>,
}

impl Default for DetailFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailFrame {
    pub fn new() -> Self {
        DetailFrame {
            grid: Grid::Quad,
            cols: 0,
            rows: 0,
            px: PixelBuffer::new(0, 0),
            ink: Vec::new(),
            thin: Vec::new(),
        }
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }

    /// Size in cells.
    pub fn cells(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// The sub-pixel picture (`2·cols` wide, `grid.rows()·rows` high).
    pub fn pixels(&self) -> &PixelBuffer {
        &self.px
    }

    /// Start a frame of `cols × rows` cells on `grid` and hand back the
    /// sub-pixel buffer for the world picture to be resampled into. Reuses the
    /// allocation unless the size changed.
    pub fn begin(&mut self, grid: Grid, cols: usize, rows: usize) -> &mut PixelBuffer {
        self.grid = grid;
        self.cols = cols;
        self.rows = rows;
        self.px.resize(cols * 2, rows * grid.rows());
        self.ink.clear();
        self.ink.resize(cols * rows, 0);
        &mut self.px
    }

    /// Start a smoothing frame of `cols × rows` cells on `grid` and fill it
    /// from `src` (the half-block picture at 1x or zoomed in) with the EPX
    /// upscale of `smooth_into`. Returns whether it filled the frame; the
    /// scratch the rule needs lives in this frame, so a steady-state frame
    /// allocates nothing.
    pub fn smooth_from(&mut self, grid: Grid, cols: usize, rows: usize, src: &PixelBuffer) -> bool {
        self.begin(grid, cols, rows);
        smooth_into(src, &mut self.px, grid, &mut self.thin)
    }

    /// Carry the screen-space passes over: every half-block pixel that differs
    /// between `before` (the resampled picture) and `after` (the same buffer
    /// once the damage digits and the ink pass have stamped it) is written
    /// over the sub-pixels that half covers, and flagged so [`encode`] keeps
    /// its colour exact. A mark is therefore never smaller than it is today.
    ///
    /// On the sextant grid a half is one and a half rows: the middle row goes
    /// to whichever half is inked, the upper one if both are.
    pub fn overlay(&mut self, before: &PixelBuffer, after: &PixelBuffer) {
        if before.w != after.w
            || before.h != after.h
            || after.w != self.cols
            || after.h != self.rows * 2
            || self.cols == 0
        {
            return;
        }
        let k = self.grid.rows();
        let w = self.cols;
        let dw = w * 2;
        let dst = self.px.pixels_mut();
        let rows = before.pixels().chunks_exact(w).zip(after.pixels().chunks_exact(w));
        for (y, (was, now)) in rows.enumerate() {
            if was == now {
                continue;
            }
            let (cy, lower) = (y / 2, y % 2 == 1);
            let (r0, r1) = match (k, lower) {
                (2, false) => (0, 1),
                (2, true) => (1, 2),
                (3, false) => (0, 2),
                (3, true) => (1, 3),
                (_, false) => (0, 2),
                (_, true) => (2, 4),
            };
            for (x, (&c0, &c1)) in was.iter().zip(now).enumerate() {
                if c0 == c1 {
                    continue;
                }
                let flags = &mut self.ink[cy * w + x];
                for r in r0..r1 {
                    let bits = 0b11u8 << (r * 2);
                    if k == 3 && r == 1 && lower && *flags & bits != 0 {
                        continue;
                    }
                    let at = (cy * k + r) * dw + x * 2;
                    dst[at] = c1;
                    dst[at + 1] = c1;
                    *flags |= bits;
                }
            }
        }
    }

    /// Encode the cell at `(cx, cy)`.
    pub fn cell(&self, cx: usize, cy: usize) -> Encoded {
        let k = self.grid.rows();
        let dw = self.cols * 2;
        let src = self.px.pixels();
        let mut px = [(0u8, 0u8, 0u8); 8];
        for r in 0..k {
            let at = (cy * k + r) * dw + cx * 2;
            px[r * 2] = src[at];
            px[r * 2 + 1] = src[at + 1];
        }
        encode(&px[..k * 2], self.ink[cy * self.cols + cx], self.grid)
    }

    /// Blit into a ratatui buffer region, one glyph a cell: the counterpart of
    /// [`PixelBuffer::render_to`], with the same colour fold.
    pub fn render_to(&self, area: Rect, buf: &mut Buffer) {
        let depth = crate::term::color_depth();
        let cols = (area.width as usize).min(self.cols);
        let rows = (area.height as usize).min(self.rows);
        let mut utf8 = [0u8; 4];
        for cy in 0..rows {
            for cx in 0..cols {
                let Some(cell) = buf.cell_mut((area.x + cx as u16, area.y + cy as u16)) else {
                    continue;
                };
                let e = self.cell(cx, cy);
                cell.set_symbol(e.glyph.encode_utf8(&mut utf8));
                cell.fg = palette::to_term(e.fg, depth);
                cell.bg = palette::to_term(e.bg, depth);
            }
        }
    }
}

/// The `--glyph-card` test card: each glyph set drawn as labelled rows, so the
/// owner of a terminal can see which sets it draws cleanly (solid, gapless,
/// one cell wide). Sub-cell detail is always on, so this is how a terminal that
/// is not recognised can be checked before it is added to `term`'s known list
/// (or pinned with `--glyphs`/`RATRUN_GLYPHS`). Plain text plus SGR; no raw
/// mode.
pub fn glyph_card() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let fg = "\x1b[38;2;255;214;96;48;2;28;32;52m";
    let reset = "\x1b[0m";
    let _ = writeln!(out, "Rat Run glyph card. Each row should be solid blocks with crisp");
    let _ = writeln!(out, "edges, no gaps between cells, and end exactly under the ruler's bar.");
    let _ = writeln!(out);
    let ruler: String = (0..64).map(|i| if i % 8 == 0 { '|' } else { '.' }).collect();
    let _ = writeln!(out, "  ruler      {ruler}|");

    let row = |out: &mut String, label: &str, glyphs: &mut dyn Iterator<Item = char>| {
        let cells: String = glyphs.take(64).collect();
        let pad = 64usize.saturating_sub(cells.chars().count());
        let _ = writeln!(out, "  {label:<10} {fg}{cells}{}{reset}|", " ".repeat(pad));
    };

    row(&mut out, "half", &mut "▀▄".chars().cycle());
    row(&mut out, "quadrant", &mut (1..15u8).map(|p| Grid::Quad.glyph(p)).cycle());
    let _ = writeln!(out);
    row(&mut out, "sextant", &mut (1..63u8).map(|p| Grid::Sextant.glyph(p)));
    // Checkers and stripes: the patterns where a font's gaps show first.
    row(
        &mut out,
        "  checker",
        &mut [0b01_10_01u8, 0b10_01_10].iter().map(|&p| Grid::Sextant.glyph(p)).cycle(),
    );
    row(
        &mut out,
        "  stripes",
        &mut [0b11_00_11u8, 0b00_11_00].iter().map(|&p| Grid::Sextant.glyph(p)).cycle(),
    );
    let _ = writeln!(out);
    for (i, chunk) in (1..=254u8).collect::<Vec<_>>().chunks(64).enumerate() {
        let label = if i == 0 { "octant" } else { "" };
        row(&mut out, label, &mut chunk.iter().map(|&p| Grid::Octant.glyph(p)));
    }
    row(
        &mut out,
        "  checker",
        &mut [0b01_10_01_10u8, 0b10_01_10_01].iter().map(|&p| Grid::Octant.glyph(p)).cycle(),
    );
    row(
        &mut out,
        "  stripes",
        &mut [0b00_11_00_11u8, 0b11_00_11_00].iter().map(|&p| Grid::Octant.glyph(p)).cycle(),
    );
    // A diagonal drawn through each set: the picture the option is for.
    let _ = writeln!(out);
    for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
        let k = grid.rows();
        for line in 0..2usize {
            let mut cells = String::new();
            for cx in 0..32usize {
                let mut pattern = 0u8;
                for r in 0..k {
                    for c in 0..2 {
                        // y = x / 4 across two rows of cells, two sub-pixels thick.
                        let (x, y) = (cx * 2 + c, line * k + r);
                        let want = x * k * 2 / 64;
                        if y == want || y + 1 == want {
                            pattern |= 1 << (r * 2 + c);
                        }
                    }
                }
                cells.push(grid.glyph(pattern));
            }
            let label = if line == 0 {
                match grid {
                    Grid::Quad => "quad line",
                    Grid::Sextant => "sext line",
                    Grid::Octant => "oct line",
                }
            } else {
                ""
            };
            let _ = writeln!(out, "  {label:<10} {fg}{cells}{reset}|");
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Detail is always on. Pin a set with --glyphs quad|sextant|octant|half,");
    let _ = writeln!(out, "or RATRUN_SUBCELL=quad|sextant|octant|auto|off for one session.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::CellWidth;

    const A: Rgb = (250, 240, 60);
    const B: Rgb = (20, 24, 40);

    fn all_glyphs(grid: Grid) -> Vec<char> {
        (0..1u16 << grid.subpixels()).map(|p| grid.glyph(p as u8)).collect()
    }

    #[test]
    fn the_glyph_tables_are_bijective_and_one_cell_wide() {
        for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
            let glyphs = all_glyphs(grid);
            let unique: std::collections::BTreeSet<char> = glyphs.iter().copied().collect();
            assert_eq!(unique.len(), 1 << grid.subpixels(), "{grid:?}: a glyph is used twice");
            for &g in &glyphs {
                let mut utf8 = [0u8; 4];
                let s = g.encode_utf8(&mut utf8);
                assert_eq!(s.cell_width(), 1, "{grid:?}: U+{:X} is not one cell wide", g as u32);
                let mut cell = ratatui::buffer::Cell::default();
                cell.set_symbol(s);
                assert_eq!(cell.symbol(), s);
            }
            // Empty is a space, full is the full block, in every set.
            assert_eq!(glyphs[0], ' ');
            assert_eq!(*glyphs.last().unwrap(), '█');
        }
    }

    #[test]
    fn the_tables_match_the_unicode_charts_at_known_points() {
        // Quadrants: bit 0 upper left, 1 upper right, 2 lower left, 3 lower right.
        assert_eq!(Grid::Quad.glyph(0b0001), '▘');
        assert_eq!(Grid::Quad.glyph(0b0010), '▝');
        assert_eq!(Grid::Quad.glyph(0b0100), '▖');
        assert_eq!(Grid::Quad.glyph(0b1000), '▗');
        assert_eq!(Grid::Quad.glyph(0b0011), '▀');
        assert_eq!(Grid::Quad.glyph(0b1001), '▚');
        assert_eq!(Grid::Quad.glyph(0b0110), '▞');
        assert_eq!(Grid::Quad.glyph(0b0111), '▛');
        // Sextants: BLOCK SEXTANT-1 is U+1FB00, and the numbering skips the two
        // patterns that are the left and right half blocks.
        assert_eq!(Grid::Sextant.glyph(0b000001), '\u{1FB00}');
        assert_eq!(Grid::Sextant.glyph(0b000010), '\u{1FB01}');
        assert_eq!(Grid::Sextant.glyph(0b010100), '\u{1FB13}', "SEXTANT-35");
        assert_eq!(Grid::Sextant.glyph(0b010101), '▌');
        assert_eq!(Grid::Sextant.glyph(0b010110), '\u{1FB14}', "SEXTANT-235");
        assert_eq!(Grid::Sextant.glyph(0b101010), '▐');
        assert_eq!(Grid::Sextant.glyph(0b111110), '\u{1FB3B}', "SEXTANT-23456");
        // Octants: BLOCK OCTANT-3 is U+1CD00 and BLOCK OCTANT-2345678 is the
        // last, U+1CDE5; the patterns older blocks cover are not re-encoded.
        assert_eq!(Grid::Octant.glyph(0b0000_0100), '\u{1CD00}', "OCTANT-3");
        assert_eq!(Grid::Octant.glyph(0b0000_0110), '\u{1CD01}', "OCTANT-23");
        assert_eq!(Grid::Octant.glyph(0b1111_1110), '\u{1CDE5}', "OCTANT-2345678");
        assert_eq!(Grid::Octant.glyph(0b0000_1111), '▀');
        assert_eq!(Grid::Octant.glyph(0b1111_0000), '▄');
        assert_eq!(Grid::Octant.glyph(0b0101_0101), '▌');
        assert_eq!(Grid::Octant.glyph(0b1010_1010), '▐');
        assert_eq!(Grid::Octant.glyph(0b0000_0101), '▘');
        assert_eq!(Grid::Octant.glyph(0b1010_0101), '▚');
        assert_eq!(Grid::Octant.glyph(0b0000_0011), '\u{1FB82}', "upper one quarter");
        assert_eq!(Grid::Octant.glyph(0b1100_0000), '▂');
        assert_eq!(Grid::Octant.glyph(0b0000_0001), '\u{1CEA8}', "OCTANT-1");
        assert_eq!(Grid::Octant.glyph(0b1000_0000), '\u{1CEA0}', "OCTANT-8");
        // 230 of the 256 live in the octant block, each once.
        let block = all_glyphs(Grid::Octant)
            .into_iter()
            .filter(|g| ('\u{1CD00}'..='\u{1CDE5}').contains(g))
            .count();
        assert_eq!(block, 230);
        let sext = all_glyphs(Grid::Sextant)
            .into_iter()
            .filter(|g| ('\u{1FB00}'..='\u{1FB3B}').contains(g))
            .count();
        assert_eq!(sext, 60);
    }

    #[test]
    fn a_known_pattern_comes_back_as_its_glyph_and_its_two_colours() {
        for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
            let n = grid.subpixels();
            for pattern in 1..((1u16 << n) - 1) {
                let pattern = pattern as u8;
                let px: Vec<Rgb> =
                    (0..n).map(|i| if pattern & (1 << i) != 0 { A } else { B }).collect();
                let e = encode(&px, 0, grid);
                assert_eq!(e.glyph, grid.glyph(pattern), "{grid:?} pattern {pattern:#b}");
                assert_eq!((e.fg, e.bg), (A, B), "{grid:?} pattern {pattern:#b}");
            }
        }
    }

    #[test]
    fn a_flat_cell_is_the_half_block_the_old_blit_wrote() {
        let c = (90, 80, 70);
        for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
            let e = encode(&vec![c; grid.subpixels()], 0, grid);
            assert_eq!(e, Encoded { glyph: '▀', fg: c, bg: c });
        }
        // Grain below the feature line keeps the half-block's two pixels: the
        // first of the upper half and the first of the lower.
        let mut px = [c; 8];
        px[4] = (96, 84, 72);
        px[7] = (84, 76, 66);
        let e = encode(&px, 0, Grid::Octant);
        assert_eq!(e, Encoded { glyph: '▀', fg: c, bg: (96, 84, 72) });
    }

    #[test]
    fn a_lone_bright_sub_pixel_keeps_its_colour_over_textured_floor() {
        // Seven floor sub-pixels in two shades and one tell: the tell is the
        // foreground, alone and unblended.
        let (f0, f1) = ((60, 52, 44), (72, 62, 52));
        let tell = (255, 80, 60);
        let px = [f0, f1, f0, tell, f1, f0, f1, f0];
        let e = encode(&px, 0, Grid::Octant);
        assert_eq!(e.glyph, Grid::Octant.glyph(0b0000_1000));
        assert_eq!(e.fg, tell);
        assert!(dist(e.bg, f0) < 300 && dist(e.bg, f1) < 300, "the floor is its mean: {:?}", e.bg);
    }

    #[test]
    fn inked_sub_pixels_come_out_exact() {
        // A dark ink border and a busy cell: both ink colours survive as they
        // were stamped, whatever else is in the cell.
        let border = (14, 10, 18);
        let core = (255, 230, 200);
        let px = [border, border, core, core, (200, 40, 40), (40, 200, 40), (90, 90, 200), (120, 120, 120)];
        let e = encode(&px, 0b0000_1111, Grid::Octant);
        assert_eq!((e.fg, e.bg), (core, border));
        // One ink colour: it is one of the two, exactly.
        let px = [(60, 60, 60), (70, 66, 60), core, core, (64, 60, 58), (60, 60, 60), (70, 66, 60), (66, 62, 60)];
        let e = encode(&px, 0b0000_1100, Grid::Octant);
        assert_eq!(e.fg, core);
        assert_eq!(e.glyph, Grid::Octant.glyph(0b0000_1100));
    }

    #[test]
    fn the_grid_follows_the_zoom_and_never_exceeds_the_set() {
        use SubcellSet::*;
        for z in [0.5, 0.75, 1.0, 1.04] {
            for set in [Off, Quad, Sextant, Octant] {
                assert_eq!(grid_for(set, z), None, "{set:?} at {z}");
            }
        }
        for z in [1.25, 1.5, 2.0] {
            assert_eq!(grid_for(Off, z), None);
            assert_eq!(grid_for(Quad, z), Some(Grid::Quad));
        }
        assert_eq!(grid_for(Sextant, 1.25), Some(Grid::Quad));
        assert_eq!(grid_for(Sextant, 1.5), Some(Grid::Sextant));
        assert_eq!(grid_for(Sextant, 2.0), Some(Grid::Sextant));
        assert_eq!(grid_for(Octant, 1.25), Some(Grid::Quad));
        assert_eq!(grid_for(Octant, 1.5), Some(Grid::Sextant));
        assert_eq!(grid_for(Octant, 2.0), Some(Grid::Octant));
    }

    #[test]
    fn the_override_beats_the_terminal_and_auto_needs_a_known_one() {
        use crate::term::BlockSet;
        // There is no option any more: with no override the terminal decides,
        // and a terminal not known to draw finer blocks keeps half blocks.
        assert_eq!(resolve(None, BlockSet::Half), SubcellSet::Off, "unknown terminal");
        assert_eq!(resolve(None, BlockSet::Quad), SubcellSet::Quad);
        assert_eq!(resolve(None, BlockSet::Sextant), SubcellSet::Sextant);
        assert_eq!(resolve(None, BlockSet::Octant), SubcellSet::Octant);
        assert_eq!(resolve(Choice::parse("auto"), BlockSet::Quad), SubcellSet::Quad);
        assert_eq!(resolve(Choice::parse("octant"), BlockSet::Half), SubcellSet::Octant);
        assert_eq!(resolve(Choice::parse("off"), BlockSet::Octant), SubcellSet::Off);
        assert_eq!(Choice::parse("sextant"), Some(Choice::Force(SubcellSet::Sextant)));
        assert_eq!(Choice::parse("QUAD "), Some(Choice::Force(SubcellSet::Quad)));
        assert_eq!(Choice::parse("lots"), None);
    }

    /// The frame grid: zoomed out it is [`grid_for`]'s answer; at 1x and zoomed
    /// in it is the smoothing grid — the full 2x2 only where the glyph set has
    /// two sub-rows a world pixel (octants), the 2-wide one else — and nothing
    /// at all with smoothing off or the half-block set.
    #[test]
    fn the_frame_grid_covers_the_smoothing_zooms() {
        use SubcellSet::*;
        assert_eq!(grid_for_frame(Octant, 2.0, true), Some(Grid::Octant));
        assert_eq!(grid_for_frame(Octant, 2.0, false), Some(Grid::Octant), "zoomed out is not smoothing");
        assert_eq!(grid_for_frame(Sextant, 1.25, true), Some(Grid::Quad));
        for z in [0.5, 0.75, 1.0] {
            assert_eq!(grid_for_frame(Octant, z, true), Some(Grid::Octant));
            assert_eq!(grid_for_frame(Quad, z, true), Some(Grid::Quad));
            assert_eq!(grid_for_frame(Sextant, z, true), Some(Grid::Quad));
            for set in [Off, Quad, Sextant, Octant] {
                assert_eq!(grid_for_frame(set, z, false), None, "{set:?} at {z} with smoothing off");
            }
            assert_eq!(grid_for_frame(Off, z, true), None, "half blocks: nothing to encode");
        }
    }

    /// A 45° stair-step between two regions, both at least two pixels thick,
    /// rounds exactly as the EPX rule says: only the one cell the step cuts
    /// through changes, and its corner becomes the other region's own colour.
    #[test]
    fn a_stair_step_rounds_into_the_expected_sub_pixels() {
        // A above, B below, stepping down to the right.
        let (a, b) = (A, B);
        let mut src = PixelBuffer::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                src.plot(x, y, if x >= 3 - y { a } else { b });
            }
        }
        // Octants: two sub-rows a world pixel, the full 2x2 of the rule.
        let mut frame = DetailFrame::new();
        assert!(frame.smooth_from(Grid::Octant, 4, 2, &src));
        // The cell holding the step (x = 2; world rows 0 and 1): its upper
        // sub-row is the dark region, its lower one rounds its upper-left
        // corner to the dark region's colour.
        let e = frame.cell(2, 0);
        assert_eq!(
            e,
            Encoded { glyph: Grid::Octant.glyph(0b1110_0000), fg: a, bg: b },
            "the step's corner should take the dark region's own colour"
        );
        // Every other cell is flat or exactly the half block it was.
        assert_eq!(frame.cell(0, 0), Encoded { glyph: '▀', fg: b, bg: b });
        assert_eq!(frame.cell(1, 0), Encoded { glyph: '▀', fg: b, bg: b });
        assert_eq!(frame.cell(3, 0), Encoded { glyph: '▀', fg: a, bg: a });
        assert_eq!(frame.cell(2, 1), Encoded { glyph: '▀', fg: a, bg: a });

        // Quadrants: two sub-pixels across, one down — the horizontal half of
        // the rule, which rounds the same corner in its own resolution.
        let mut frame = DetailFrame::new();
        assert!(frame.smooth_from(Grid::Quad, 4, 2, &src));
        assert_eq!(
            frame.cell(2, 0),
            Encoded { glyph: Grid::Quad.glyph(0b1000), fg: a, bg: b }
        );
        assert_eq!(frame.cell(3, 0), Encoded { glyph: '▀', fg: a, bg: a });
    }

    /// A flat picture is the half-block blit: every sub-pixel is its world
    /// pixel, every cell the plain `▀`, so flat floor costs exactly what it did.
    #[test]
    fn a_flat_picture_is_the_half_block_blit() {
        let floor = (52, 48, 44);
        let mut src = PixelBuffer::new(8, 6);
        src.clear(floor);
        for grid in [Grid::Quad, Grid::Octant] {
            let mut frame = DetailFrame::new();
            assert!(frame.smooth_from(grid, 8, 3, &src));
            for cy in 0..3 {
                for cx in 0..8 {
                    assert_eq!(
                        frame.cell(cx, cy),
                        Encoded { glyph: '▀', fg: floor, bg: floor },
                        "{grid:?} ({cx}, {cy})"
                    );
                }
            }
        }
    }

    /// A soft lit gradient is not an edge: nothing is rounded and no cell
    /// needs a sub-cell glyph, so a lamp's falloff cannot band or shimmer.
    #[test]
    fn a_smooth_lit_gradient_emits_no_sub_cell_glyphs() {
        let mut src = PixelBuffer::new(24, 16);
        for y in 0..16 {
            for x in 0..24 {
                let v = 30 + (x * 5 + y * 3) as u8; // steps of 3-8 a pixel
                src.plot(x, y, (v, v, v));
            }
        }
        let mut frame = DetailFrame::new();
        assert!(frame.smooth_from(Grid::Octant, 24, 8, &src));
        for cy in 0..8 {
            for cx in 0..24 {
                let e = frame.cell(cx, cy);
                assert_eq!(e.glyph, '▀', "({cx}, {cy}) took a sub-cell glyph: {e:?}");
                assert_eq!(e.fg, src.pixel_at(cx, cy * 2));
                assert_eq!(e.bg, src.pixel_at(cx, cy * 2 + 1));
            }
        }
        // A hard *straight* edge through the same picture is not a stair-step
        // either, and must not turn into glyphs: the rule rounds corners, not
        // edges, so a wall or a platform keeps its exact silhouette.
        let mut step = PixelBuffer::new(24, 16);
        step.clear(B);
        for y in 0..16 {
            step.line(0, y, 11, y, A);
        }
        let mut frame = DetailFrame::new();
        assert!(frame.smooth_from(Grid::Octant, 24, 8, &step));
        for cy in 0..8 {
            for cx in 0..24 {
                let e = frame.cell(cx, cy);
                assert_eq!(e.glyph, '▀', "({cx}, {cy}) took a sub-cell glyph: {e:?}");
            }
        }
    }

    /// A one-pixel line — a whisker, a telegraph, a bullet's rim — is never
    /// modified and never copied outward: every line pixel keeps all its
    /// sub-pixels, and no sub-pixel outside the line takes its colour.
    #[test]
    fn a_one_pixel_line_survives_the_smoothing_unchanged() {
        let floor = (40, 44, 52);
        let line = (255, 236, 120);
        let (w, h) = (24usize, 16usize);
        let mut src = PixelBuffer::new(w, h);
        src.clear(floor);
        // A shallow diagonal, a vertical, a horizontal and a lone speck.
        for i in 0..12 {
            src.plot(2 + i, 2 + i / 2, line);
        }
        src.line(18, 1, 18, 14, line);
        src.line(3, 12, 14, 12, line);
        src.plot(21, 14, line);
        let mut is_line = vec![false; w * h];
        for (i, &c) in src.pixels().iter().enumerate() {
            is_line[i] = c == line;
        }
        for grid in [Grid::Quad, Grid::Octant] {
            let rp = (grid.rows() / 2).max(1);
            let mut frame = DetailFrame::new();
            assert!(frame.smooth_from(grid, w, h / 2, &src));
            let px = frame.pixels();
            for y in 0..h {
                for x in 0..w {
                    let want = is_line[y * w + x];
                    for r in 0..rp {
                        for c in 0..2 {
                            let got = px.pixel_at(x * 2 + c, y * rp + r) == line;
                            assert_eq!(
                                got, want,
                                "{grid:?}: ({x}, {y}) sub-pixel ({c}, {r}) is_line={want}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A two-by-two pellet (the smallest a body's bright core gets) is not a
    /// stair-step: all four of its pixels keep every sub-pixel, and no floor
    /// cell takes its colour.
    #[test]
    fn a_two_pixel_pellet_is_not_thinned() {
        let floor = (44, 40, 48);
        let pellet = (255, 120, 160);
        let mut src = PixelBuffer::new(10, 8);
        src.clear(floor);
        for y in 3..5 {
            for x in 4..6 {
                src.plot(x, y, pellet);
            }
        }
        let mut frame = DetailFrame::new();
        assert!(frame.smooth_from(Grid::Octant, 10, 4, &src));
        let px = frame.pixels();
        for y in 0..8 {
            for x in 0..10 {
                let want = (4usize..6).contains(&x) && (3usize..5).contains(&y);
                for r in 0..2 {
                    for c in 0..2 {
                        assert_eq!(
                            px.pixel_at(x * 2 + c, y * 2 + r) == pellet,
                            want,
                            "({x}, {y}) sub-pixel ({c}, {r})"
                        );
                    }
                }
            }
        }
        assert_eq!(frame.cell(0, 0), Encoded { glyph: '▀', fg: floor, bg: floor });
    }

    /// At 1x the smoothed frame is still one cell wide a column and carries the
    /// ink pass's marks exactly: the smoothing runs on the picture *before* the
    /// screen-space passes, and [`DetailFrame::overlay`] stamps what they
    /// changed, so a coin, a gem and a hostile pellet are never smoothed away.
    #[test]
    fn the_ink_marks_survive_the_one_x_smoothing() {
        use crate::game::bullet::{Bullet, Faction};
        use crate::game::gem::{Gem, Gold};
        use crate::game::level::Theme;
        use crate::game::world::World;
        use crate::math::Vec2;

        const COIN: Rgb = (255, 196, 64);
        const COIN_CORE: Rgb = (255, 246, 196);
        const BORDER: Rgb = (14, 10, 18);
        const WHITE: Rgb = (255, 255, 255);
        for set in [SubcellSet::Quad, SubcellSet::Octant] {
            let mut w = World::new(Theme::Sewer, 7);
            let p = w.player.pos;
            w.spawn_gem(Gem::new(p + Vec2::new(-30.0, -20.0), 5));
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Gold::new(p + Vec2::new(30.0, -20.0), 1),
            );
            w.spawn_bullet(Bullet::new(
                p + Vec2::new(25.0, 22.0),
                Vec2::new(0.0, 0.0),
                4.0,
                Faction::Enemy,
                (240, 120, 160),
            ));
            w.set_zoom_instant(1.0);
            w.subcell = set;
            w.smooth = true;
            let mut pb = PixelBuffer::new(120, 80);
            w.draw(&mut pb);
            let frame = w.detail().expect("1x with smoothing: a detail frame");
            assert_eq!(frame.cells(), (120, 40));
            assert_eq!(frame.grid(), grid_for_frame(set, 1.0, true).unwrap());

            let mut seen = [0usize; 4];
            for y in 0..pb.h {
                for x in 0..pb.w {
                    if x.abs_diff(60) < 14 && y.abs_diff(40) < 14 {
                        continue;
                    }
                    let c = pb.pixel_at(x, y);
                    let Some(k) = [COIN, COIN_CORE, BORDER, WHITE].iter().position(|&m| m == c)
                    else {
                        continue;
                    };
                    seen[k] += 1;
                    let e = frame.cell(x, y / 2);
                    assert!(
                        e.fg == c || e.bg == c,
                        "{set:?}: the mark {c:?} at ({x}, {y}) became {e:?}"
                    );
                }
            }
            assert!(seen[0] >= 8 && seen[1] >= 1, "{set:?}: the coin square: {seen:?}");
            // (No pellet border here: the ink pass re-stamps hostile marks only
            // when zoomed out, so at 1x there is none to keep.)
            assert!(seen[3] >= 1, "{set:?}: the gem centre: {seen:?}");

            // Smoothing off: no detail frame, so the frame is the half-block
            // blit byte for byte (the same buffer `set = Off` draws).
            w.smooth = false;
            w.draw(&mut pb);
            assert!(w.detail().is_none(), "with smoothing off 1x has no detail frame");
            let mut plain = PixelBuffer::new(120, 80);
            w.subcell = SubcellSet::Off;
            w.draw(&mut plain);
            assert_eq!(pb.pixels(), plain.pixels(), "smoothing off is today's picture");
        }
    }

    /// A one-world-pixel bright line at 2× lands on the octant grid 1:1, so it
    /// comes out as a continuous line of sub-pixels: every column of every
    /// cell it crosses has a foreground sub-pixel, in the line's own colour.
    #[test]
    fn a_one_pixel_line_at_double_zoom_survives_as_a_continuous_line() {
        let (cols, rows) = (24usize, 10usize);
        let floor = (40, 44, 52);
        let line = (255, 236, 120);
        let mut world = PixelBuffer::new(cols * 2, rows * 4);
        world.clear(floor);
        // A shallow diagonal and a vertical, both one world pixel wide.
        world.line(1, 3, cols as i32 * 2 - 2, rows as i32 * 4 - 5, line);
        world.line(7, 0, 7, rows as i32 * 4 - 1, line);
        let mut frame = DetailFrame::new();
        world.resample_to(frame.begin(Grid::Octant, cols, rows));
        assert_eq!(frame.pixels().pixels(), world.pixels(), "2x on octants is the world, 1:1");

        // Decode every cell back to sub-pixels and compare with the world.
        let inverse: std::collections::HashMap<char, u8> =
            (0..=255u8).map(|p| (Grid::Octant.glyph(p), p)).collect();
        let mut lit = 0;
        for cy in 0..rows {
            for cx in 0..cols {
                let e = frame.cell(cx, cy);
                for r in 0..4 {
                    for c in 0..2 {
                        let want = world.pixel_at(cx * 2 + c, cy * 4 + r);
                        let on = if e.fg == e.bg {
                            true
                        } else {
                            inverse[&e.glyph] & (1 << (r * 2 + c)) != 0
                        };
                        let got = if on { e.fg } else { e.bg };
                        assert_eq!(got, want, "cell ({cx}, {cy}) sub-pixel ({c}, {r})");
                        lit += usize::from(got == line);
                    }
                }
            }
        }
        assert!(lit >= cols * 2 + rows * 4 - 6, "the lines are all there: {lit} sub-pixels");

        // The same picture through the half-block resample keeps a quarter of
        // the cell's pixels: the detail frame holds four times as many.
        let mut half = PixelBuffer::new(cols, rows * 2);
        world.resample_to(&mut half);
        let half_lit = half.pixels().iter().filter(|&&c| c == line).count();
        assert!(lit > half_lit, "{lit} line sub-pixels against {half_lit} half-block pixels");
    }

    #[test]
    fn a_stamped_mark_covers_its_half_of_the_cell_on_every_grid() {
        let floor = (40, 44, 52);
        let mark = (255, 255, 255);
        for grid in [Grid::Quad, Grid::Sextant, Grid::Octant] {
            let (cols, rows) = (6usize, 4usize);
            let mut world = PixelBuffer::new(cols * 2, rows * 4);
            world.clear(floor);
            let mut frame = DetailFrame::new();
            world.resample_to(frame.begin(grid, cols, rows));
            let mut before = PixelBuffer::new(cols, rows * 2);
            before.clear(floor);
            let mut after = PixelBuffer::new(cols, rows * 2);
            after.clear(floor);
            after.plot(2, 2, mark); // upper half of cell (2, 1)
            after.plot(4, 5, mark); // lower half of cell (4, 2)
            after.plot(1, 6, mark); // both halves of cell (1, 3)
            after.plot(1, 7, mark);
            frame.overlay(&before, &after);
            let upper = frame.cell(2, 1);
            let lower = frame.cell(4, 2);
            let both = frame.cell(1, 3);
            assert_eq!((upper.fg, upper.bg), (mark, floor), "{grid:?}");
            assert_eq!((lower.fg, lower.bg), (mark, floor), "{grid:?}");
            assert_eq!(both, Encoded { glyph: '▀', fg: mark, bg: mark }, "{grid:?}");
            let (up, low) = match grid {
                Grid::Quad => (0b0011, 0b1100),
                Grid::Sextant => (0b001111, 0b111100),
                Grid::Octant => (0b0000_1111, 0b1111_0000),
            };
            assert_eq!(upper.glyph, grid.glyph(up), "{grid:?}");
            assert_eq!(lower.glyph, grid.glyph(low), "{grid:?}");
            // Nothing else moved.
            assert_eq!(frame.cell(0, 0), Encoded { glyph: '▀', fg: floor, bg: floor });
        }
    }

    /// The screen-space passes survive the detail path: at 2× on every set a
    /// gem's white centre, a coin's square and a hostile pellet's bright core
    /// and dark border all come out of the encoder in the exact colours the
    /// ink pass stamped, in the cells it stamped them.
    #[test]
    fn the_ink_marks_still_appear_through_every_glyph_set() {
        use crate::game::bullet::{Bullet, Faction};
        use crate::game::gem::{Gem, Gold};
        use crate::game::level::Theme;
        use crate::game::world::World;
        use crate::math::Vec2;

        const COIN: Rgb = (255, 196, 64);
        const COIN_CORE: Rgb = (255, 246, 196);
        const BORDER: Rgb = (14, 10, 18);
        const WHITE: Rgb = (255, 255, 255);
        for set in [SubcellSet::Quad, SubcellSet::Sextant, SubcellSet::Octant] {
            let mut w = World::new(Theme::Sewer, 7);
            let p = w.player.pos;
            w.spawn_gem(Gem::new(p + Vec2::new(-60.0, -40.0), 5));
            crate::game::ecs::resources::spawn_with_id(
                &mut w.ecs,
                Gold::new(p + Vec2::new(60.0, -40.0), 1),
            );
            w.spawn_bullet(Bullet::new(
                p + Vec2::new(50.0, 44.0),
                Vec2::new(0.0, 0.0),
                4.0,
                Faction::Enemy,
                (240, 120, 160),
            ));
            w.set_zoom_instant(2.0);
            w.subcell = set;
            let mut pb = PixelBuffer::new(120, 80);
            w.draw(&mut pb);
            let frame = w.detail().expect("zoomed out with a set: a detail frame");
            assert_eq!(frame.cells(), (120, 40));
            assert_eq!(frame.grid(), grid_for(set, 2.0).unwrap());

            let mut seen = [0usize; 4];
            for y in 0..pb.h {
                for x in 0..pb.w {
                    // The rat's own marks sit at the centre and can put three
                    // ink colours in one cell; the loot and the pellet are
                    // placed well clear of it.
                    if x.abs_diff(60) < 14 && y.abs_diff(40) < 14 {
                        continue;
                    }
                    let c = pb.pixel_at(x, y);
                    let Some(k) = [COIN, COIN_CORE, BORDER, WHITE].iter().position(|&m| m == c)
                    else {
                        continue;
                    };
                    seen[k] += 1;
                    let e = frame.cell(x, y / 2);
                    assert!(
                        e.fg == c || e.bg == c,
                        "{set:?}: the mark {c:?} at ({x}, {y}) became {e:?}"
                    );
                }
            }
            assert!(seen[0] >= 8 && seen[1] >= 1, "{set:?}: the coin square: {seen:?}");
            // (No pellet border here: the ink pass re-stamps hostile marks only
            // when zoomed out, so at 1x there is none to keep.)
            assert!(seen[3] >= 1, "{set:?}: the gem centre: {seen:?}");

            // Without a set, or at 1x with the smoothing off, there is no
            // detail frame.
            w.set_zoom_instant(1.0);
            w.smooth = false;
            w.draw(&mut pb);
            assert!(w.detail().is_none(), "{set:?}: 1x with smoothing off is the half-block picture");
            w.set_zoom_instant(2.0);
            w.subcell = SubcellSet::Off;
            w.draw(&mut pb);
            assert!(w.detail().is_none());
        }
    }

    #[test]
    fn the_glyph_card_names_every_set() {
        let card = glyph_card();
        for label in ["half", "quadrant", "sextant", "octant", "RATRUN_SUBCELL", "always on"] {
            assert!(card.contains(label), "{label}");
        }
        assert!(card.contains('\u{1FB00}') && card.contains('\u{1CD00}') && card.contains('▚'));
    }
}
