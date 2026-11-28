//! Everything drawn: the half-block pixel buffer the world is painted into, the
//! render-only passes over it, and the ratatui screens around it.
//!
//! * `framebuffer` — `PixelBuffer`, the `width × (height*2)` RGB grid blitted
//!   one `▀` per cell, and `resample_to` (the zoom resample).
//! * `zoom`, `lighting`, `ink`, `hazard_outline` — render-only passes: the zoom
//!   ladder, the 2D light map applied before the resample, and the screen-space
//!   re-stamp of gameplay-critical marks after it. None may touch the
//!   simulation.
//! * `subcell` — sub-cell detail (always on): the zoomed-out picture drawn with quadrant,
//!   sextant or octant glyphs instead of half blocks. Render-only.
//! * `foreground` — the parallax overhead layer: pipes, lines and hanging things
//!   drawn after the lighting pass and under the hostile layer, sliding faster
//!   than the floor. Render-only, capped at 8 % of the view's pixels a frame.
//! * `palette` — colours and `to_term()`, the fold to the terminal's depth.
//! * `hud`, `overlay`, `minimap`, `minimap_source`, `cursor` — what sits over
//!   the playfield during a run.
//! * `menu`, `options`, `controls`, `campaign`, `shop`, `nest`, `library`,
//!   `console` — one module per screen. Each screen's `layout()` is the single
//!   source of truth for its pointer hit rectangles.

pub mod campaign;
pub mod controls;
pub mod console;
pub mod cursor;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod framebuffer;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod hazard_outline;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod hud;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod ink;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod library;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod menu;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod overlay;
pub mod shop;
pub mod subcell;
pub mod palette;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod minimap;
pub mod nest;
pub mod minimap_source;
pub mod options;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod lighting;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod zoom;
pub mod foreground;
pub mod glyphfx;
pub mod graphics;
pub mod testbed;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod outfall_current;
#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod touch_art;
pub mod dropping_fade;

/// Package version stamped in the top-left corner of every screen.
pub const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dim, non-competing ink for [`BUILD_VERSION`]: readable over both
/// [`palette::PLATE`] and the menu backdrop without rivalling HUD labels.
pub const BUILD_VERSION_FG: ratatui::style::Color = ratatui::style::Color::Rgb(118, 130, 134);

/// Where a `panel` caption starts on its top border.
///
/// When `panel` touches the top row of `frame_area` near its left edge, room is
/// reserved after `┌` for `" <BUILD_VERSION> "` plus one `─` border cell so the
/// caption shifts right instead of overlapping the build number.
pub fn border_caption_x(
    frame_area: ratatui::layout::Rect,
    panel: ratatui::layout::Rect,
) -> u16 {
    let r = panel.intersection(frame_area);
    let vw = BUILD_VERSION.len() as u16;
    let reserve = vw + 2;
    if r.y == frame_area.y
        && r.x <= frame_area.x.saturating_add(reserve + 1)
        && r.width >= reserve + 8
        && r.height >= 2
    {
        r.x + 2 + reserve
    } else {
        r.x.saturating_add(2)
    }
}

/// Stamp [`BUILD_VERSION`] at `(area.x, area.y)` over a backdrop or wall tile.
pub fn stamp_build_version_corner(
    buf: &mut ratatui::buffer::Buffer,
    area: ratatui::layout::Rect,
    bg: ratatui::style::Color,
) {
    let r = area.intersection(buf.area);
    let vw = BUILD_VERSION.len() as u16;
    if r.width < vw || r.height == 0 {
        return;
    }
    buf.set_stringn(
        r.x,
        r.y,
        BUILD_VERSION,
        vw as usize,
        ratatui::style::Style::default().fg(BUILD_VERSION_FG).bg(bg),
    );
}

/// Stamp `" <BUILD_VERSION> "` on `panel`'s top border when it sits at the
/// top-left of `buf.area`, clearing any partial underlying stamp to the left of
/// `panel.x`, and return the column where `panel`'s own caption should begin.
pub fn stamp_build_version_on_border(
    buf: &mut ratatui::buffer::Buffer,
    panel: ratatui::layout::Rect,
) -> u16 {
    let frame_area = buf.area;
    let r = panel.intersection(frame_area);
    let cap_x = border_caption_x(frame_area, panel);
    if cap_x > r.x.saturating_add(2) {
        for x in frame_area.x..r.x {
            let cell = &mut buf[(x, r.y)];
            let bg = cell.bg;
            cell.set_char(' ')
                .set_style(ratatui::style::Style::default().fg(palette::PLATE).bg(bg));
        }
        let vw = BUILD_VERSION.len() as u16;
        let style = ratatui::style::Style::default()
            .fg(BUILD_VERSION_FG)
            .bg(palette::PLATE);
        buf[(r.x + 1, r.y)].set_char(' ').set_style(style);
        buf.set_stringn(r.x + 2, r.y, BUILD_VERSION, vw as usize, style);
        buf[(r.x + 2 + vw, r.y)].set_char(' ').set_style(style);
    }
    cap_x
}
