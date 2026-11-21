//! Half-block pixel framebuffer.
//!
//! Each terminal cell renders two stacked pixels using the `▀` (upper half
//! block) glyph: the foreground color paints the top pixel, the background
//! color paints the bottom pixel. The pixel grid is therefore
//! `width = cols` by `height = rows * 2`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::palette::Rgb;

pub struct PixelBuffer {
    pub w: usize,
    pub h: usize,
    px: Vec<Rgb>,
}

impl PixelBuffer {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![(0, 0, 0); w * h],
        }
    }

    /// Resize in place. The app calls this every frame with the game viewport
    /// size, so only reallocate when the dimensions actually change.
    pub fn resize(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h {
            return;
        }
        self.w = w;
        self.h = h;
        // `clear` + `resize` keeps the existing allocation when growing.
        self.px.clear();
        self.px.resize(w * h, (0, 0, 0));
    }

    /// Resample this buffer into `dst` (the zoomed render → the screen grid).
    ///
    /// The source covers exactly the same world rectangle the destination will
    /// show, so each destination pixel owns the source box
    /// `[x·sx, (x+1)·sx) × [y·sy, (y+1)·sy)`:
    ///
    /// * **zoomed out** (`self.w > dst.w`) the box spans several source
    ///   pixels and exactly one of them is written, unblended: the one that
    ///   stands out most from the box when the box holds a feature, else its
    ///   first pixel. Edges stay crisp and thin things (bullets, the rat's
    ///   tail, wall edges) survive at full contrast instead of blurring;
    /// * **zoomed in** (`self.w < dst.w`) the box is a single source pixel,
    ///   which is copied — nearest neighbour, so pixels stay crisp;
    /// * **1:1** this is an exact copy.
    ///
    /// Integer arithmetic throughout, so a 1:1 frame is bit-identical to the
    /// source and nothing depends on float rounding.
    pub fn resample_to(&self, dst: &mut PixelBuffer) {
        if dst.w == 0 || dst.h == 0 {
            return;
        }
        if self.w == 0 || self.h == 0 {
            dst.clear((0, 0, 0));
            return;
        }
        for y in 0..dst.h {
            let y0 = y * self.h / dst.h;
            let y1 = ((y + 1) * self.h).div_ceil(dst.h).max(y0 + 1).min(self.h);
            let row = y * dst.w;
            for x in 0..dst.w {
                let x0 = x * self.w / dst.w;
                let x1 = ((x + 1) * self.w).div_ceil(dst.w).max(x0 + 1).min(self.w);
                if x1 - x0 == 1 && y1 - y0 == 1 {
                    // Nearest neighbour (zoomed in): keep the pixel exact.
                    dst.px[row + x] = self.px[y0 * self.w + x0];
                    continue;
                }
                // Zoomed out: several source pixels land on one screen pixel.
                // Averaging them (a box filter) keeps thin things from
                // vanishing but smears every edge into a blur, which is the
                // opposite of what pixel art wants. Instead pick ONE source
                // pixel, unblended:
                //  * if the box holds a feature (a pixel that stands out from
                //    the box's mean by more than `SALIENT`), take the pixel
                //    that stands out most, so a one-pixel line, bullet or
                //    outline survives at full contrast wherever it falls in
                //    the box (no flicker as the camera scrolls);
                //  * otherwise the box is flat or mere texture grain: take its
                //    first pixel, which keeps soft gradients (lighting, smoke)
                //    from turning into noise.
                const SALIENT: u32 = 3 * 28 * 28;
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for sy in y0..y1 {
                    let srow = sy * self.w;
                    for sx in x0..x1 {
                        let c = self.px[srow + sx];
                        r += u32::from(c.0);
                        g += u32::from(c.1);
                        b += u32::from(c.2);
                    }
                }
                let n = ((x1 - x0) * (y1 - y0)) as u32;
                let mean = ((r / n) as i32, (g / n) as i32, (b / n) as i32);
                let mut best = self.px[y0 * self.w + x0];
                let mut best_d = SALIENT;
                for sy in y0..y1 {
                    let srow = sy * self.w;
                    for sx in x0..x1 {
                        let c = self.px[srow + sx];
                        let (dr, dg, db) = (
                            i32::from(c.0) - mean.0,
                            i32::from(c.1) - mean.1,
                            i32::from(c.2) - mean.2,
                        );
                        let d = (dr * dr + dg * dg + db * db) as u32;
                        if d > best_d {
                            best_d = d;
                            best = c;
                        }
                    }
                }
                dst.px[row + x] = best;
            }
        }
    }

    pub fn clear(&mut self, c: Rgb) {
        for p in self.px.iter_mut() {
            *p = c;
        }
    }

    #[inline]
    pub fn plot(&mut self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.w || y >= self.h {
            return;
        }
        self.px[y * self.w + x] = c;
    }

    /// Additive plot — useful for glowing particles / overlapping light.
    #[inline]
    pub fn plot_add(&mut self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.w || y >= self.h {
            return;
        }
        let i = y * self.w + x;
        let cur = self.px[i];
        self.px[i] = super::palette::add(cur, c);
    }

    pub fn filled_circle(&mut self, cx: i32, cy: i32, r: i32, c: Rgb) {
        if r <= 0 {
            self.plot(cx, cy, c);
            return;
        }
        let r2 = r * r;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r2 {
                    self.plot(cx + dx, cy + dy, c);
                }
            }
        }
    }

    /// Outline ring (for auras / shockwaves).
    pub fn ring(&mut self, cx: i32, cy: i32, r: i32, c: Rgb) {
        if r <= 0 {
            return;
        }
        let steps = (r * 8).max(8);
        for i in 0..steps {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            let x = cx + (a.cos() * r as f32).round() as i32;
            let y = cy + (a.sin() * r as f32).round() as i32;
            self.plot_add(x, y, c);
        }
    }

    pub fn rect_fill(&mut self, x0: i32, y0: i32, w: i32, h: i32, c: Rgb) {
        if w <= 0 || h <= 0 || self.w == 0 || self.h == 0 {
            return;
        }
        // Clip the rectangle to the buffer once instead of bounds-checking
        // every pixel inside `plot` (bulk fills are per-cell hot paths).
        let (bw, bh) = (self.w as i32, self.h as i32);
        let xa = x0.max(0).min(bw);
        let xb = x0.saturating_add(w).max(0).min(bw);
        let ya = y0.max(0).min(bh);
        let yb = y0.saturating_add(h).max(0).min(bh);
        if xa >= xb || ya >= yb {
            return;
        }
        for y in ya..yb {
            let row = y as usize * self.w;
            self.px[row + xa as usize..row + xb as usize].fill(c);
        }
    }

    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.plot(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Backing-store length (tests only: proves `resize` tracks the size).
    #[cfg(test)]
    pub(crate) fn px_len_for_test(&self) -> usize {
        self.px.len()
    }

    /// Backing-store address (tests only: proves a buffer was reused instead of
    /// reallocated).
    #[cfg(test)]
    pub(crate) fn px_ptr_for_test(&self) -> *const Rgb {
        self.px.as_ptr()
    }

    /// Read a pixel (for screenshot tooling).
    pub fn pixel_at(&self, x: usize, y: usize) -> Rgb {
        self.get(x, y)
    }

    /// Direct read-only access to the backing RGB pixel slice (row-major, `w * h`).
    #[inline]
    pub fn pixels(&self) -> &[Rgb] {
        &self.px
    }

    /// Direct mutable access to the backing RGB pixel slice (row-major, `w * h`).
    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [Rgb] {
        &mut self.px
    }

    #[inline]
    fn get(&self, x: usize, y: usize) -> Rgb {
        if x < self.w && y < self.h {
            self.px[y * self.w + x]
        } else {
            (0, 0, 0)
        }
    }

    /// Blit the pixel grid into a ratatui buffer region using `▀` cells.
    ///
    /// This is the hot path: a 160 × 168-pixel viewport is 13 440 cells, so
    /// the colour fold for a terminal without 24-bit colour happens here, per
    /// pixel, through [`palette::to_term`]'s cached lookup. The capability is
    /// read once per frame, not once per cell.
    pub fn render_to(&self, area: Rect, buf: &mut Buffer) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        let depth = crate::term::color_depth();
        for cy in 0..area.height {
            let top_y = cy as usize * 2;
            let bot_y = top_y + 1;
            for cx in 0..area.width {
                let xx = cx as usize;
                let top = self.get(xx, top_y);
                let bot = self.get(xx, bot_y);
                if let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) {
                    cell.set_symbol("▀");
                    cell.set_fg(super::palette::to_term(top, depth));
                    cell.set_bg(super::palette::to_term(bot, depth));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_fill_clips_to_bounds() {
        let mut pb = PixelBuffer::new(4, 4);
        // Straddles the top-left corner: only the in-bounds part is painted.
        pb.rect_fill(-2, -1, 3, 3, (9, 9, 9));
        assert_eq!(pb.pixel_at(0, 0), (9, 9, 9));
        assert_eq!(pb.pixel_at(1, 2), (0, 0, 0));
        assert_eq!(pb.pixel_at(2, 2), (0, 0, 0));

        // Straddles the bottom-right corner too, and overflows by far more
        // than the buffer size.
        pb.rect_fill(2, 2, 900, 900, (7, 7, 7));
        assert_eq!(pb.pixel_at(3, 3), (7, 7, 7));
        assert_eq!(pb.pixel_at(1, 1), (0, 0, 0));

        // Fully outside → no-op, no panic.
        pb.rect_fill(-50, -50, 10, 10, (1, 2, 3));
        pb.rect_fill(90, 90, 10, 10, (1, 2, 3));
        assert_eq!(pb.pixel_at(0, 0), (9, 9, 9));
    }

    #[test]
    fn resample_is_exact_at_one_to_one() {
        let mut src = PixelBuffer::new(4, 3);
        for y in 0..3 {
            for x in 0..4 {
                src.plot(x, y, ((x * 10) as u8, (y * 10) as u8, 7));
            }
        }
        let mut dst = PixelBuffer::new(4, 3);
        src.resample_to(&mut dst);
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(dst.pixel_at(x, y), src.pixel_at(x, y));
            }
        }
    }

    #[test]
    fn resample_keeps_one_crisp_pixel_when_zooming_out() {
        // Zooming out never blends: a single bright bullet in a 2 × 2 box
        // survives at full contrast wherever it sits in the box, a solid block
        // stays solid, and low-contrast grain falls back to the first pixel.
        for (bx, by) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let mut src = PixelBuffer::new(6, 2);
            src.plot(bx, by, (255, 255, 255));
            for (x, y) in [(2, 0), (3, 0), (2, 1), (3, 1)] {
                src.plot(x, y, (255, 0, 0));
            }
            src.plot(4, 0, (40, 40, 40));
            src.plot(5, 0, (44, 42, 40));
            src.plot(4, 1, (38, 40, 44));
            src.plot(5, 1, (42, 38, 40));
            let mut dst = PixelBuffer::new(3, 1);
            src.resample_to(&mut dst);
            assert_eq!(dst.pixel_at(0, 0), (255, 255, 255), "the lit pixel at ({bx},{by}) is kept whole");
            assert_eq!(dst.pixel_at(1, 0), (255, 0, 0), "a solid block stays solid");
            assert_eq!(dst.pixel_at(2, 0), (40, 40, 40), "flat grain takes the first pixel, unblended");
        }
    }

    #[test]
    fn resample_samples_nearest_when_zooming_in() {
        // Zooming in maps several destination pixels onto one source pixel and
        // copies it, so pixels stay crisp (no blur across the block edge).
        let mut src = PixelBuffer::new(2, 1);
        src.plot(0, 0, (200, 0, 0));
        src.plot(1, 0, (0, 200, 0));
        let mut dst = PixelBuffer::new(4, 2);
        src.resample_to(&mut dst);
        assert_eq!(dst.pixel_at(0, 0), (200, 0, 0));
        assert_eq!(dst.pixel_at(1, 0), (200, 0, 0));
        assert_eq!(dst.pixel_at(2, 0), (0, 200, 0));
        assert_eq!(dst.pixel_at(3, 1), (0, 200, 0));
    }

    #[test]
    fn resample_survives_degenerate_sizes() {
        let mut dst = PixelBuffer::new(0, 0);
        PixelBuffer::new(3, 3).resample_to(&mut dst);
        assert_eq!((dst.w, dst.h), (0, 0));
        let mut dst = PixelBuffer::new(4, 2);
        dst.clear((9, 9, 9));
        PixelBuffer::new(0, 0).resample_to(&mut dst);
        assert_eq!(dst.pixel_at(0, 0), (0, 0, 0), "an empty source blanks the frame");
        // Both directions, tiny → huge and huge → tiny, never panics.
        for (sw, sh, dw, dh) in [(1usize, 1usize, 5usize, 3usize), (9, 7, 1, 1), (1, 1, 1, 1)] {
            let mut src = PixelBuffer::new(sw, sh);
            src.plot(0, 0, (255, 128, 0));
            let mut dst = PixelBuffer::new(dw, dh);
            src.resample_to(&mut dst);
            assert_eq!((dst.w, dst.h), (dw, dh));
        }
    }

    #[test]
    fn resize_keeps_dimensions_and_len() {
        let mut pb = PixelBuffer::new(3, 2);
        assert_eq!(pb.px_len_for_test(), 6);
        pb.resize(3, 2); // unchanged: no work
        pb.resize(5, 4);
        assert_eq!((pb.w, pb.h), (5, 4));
        assert_eq!(pb.px_len_for_test(), 20);
        pb.resize(1, 1);
        assert_eq!(pb.px_len_for_test(), 1);
        assert_eq!(pb.pixel_at(0, 0), (0, 0, 0));
    }
}
