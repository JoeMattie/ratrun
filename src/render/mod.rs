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
