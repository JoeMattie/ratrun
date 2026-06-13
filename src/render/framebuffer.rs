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
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.plot(x, y, c);
            }
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
