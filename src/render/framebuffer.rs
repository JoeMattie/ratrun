//! Half-block pixel framebuffer.
//!
//! Each terminal cell renders two stacked pixels using the `▀` (upper half
//! block) glyph: the foreground color paints the top pixel, the background
//! color paints the bottom pixel. The pixel grid is therefore
//! `width = cols` by `height = rows * 2`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

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

    #[inline]
    fn get(&self, x: usize, y: usize) -> Rgb {
        if x < self.w && y < self.h {
            self.px[y * self.w + x]
        } else {
            (0, 0, 0)
        }
    }

    /// Blit the pixel grid into a ratatui buffer region using `▀` cells.
    pub fn render_to(&self, area: Rect, buf: &mut Buffer) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        for cy in 0..area.height {
            let top_y = cy as usize * 2;
            let bot_y = top_y + 1;
            for cx in 0..area.width {
                let xx = cx as usize;
                let top = self.get(xx, top_y);
                let bot = self.get(xx, bot_y);
                if let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) {
                    cell.set_symbol("▀");
                    cell.set_fg(Color::Rgb(top.0, top.1, top.2));
                    cell.set_bg(Color::Rgb(bot.0, bot.1, bot.2));
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
