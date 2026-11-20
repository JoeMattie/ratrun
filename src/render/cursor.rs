//! Cheese mouse cursor — a tiny half-block Swiss cheese wedge drawn on top of
//! every normal-game screen while the player is using the mouse.
//!
//! # Geometry & Hot Spot
//!
//! The cursor occupies a `4 × 2` terminal-cell bounding box (`4 × 4` square
//! half-block pixels using `▀` and `▄`), with the resting Swiss cheese wedge
//! sitting cleanly in the top `4 × 3` half-block pixels (`4:3` landscape
//! wedge — `1.5` cells tall, leaving the bottom half of row 1 transparent so
//! text beneath peeks through). Its **hot spot** is `(0, 0)` — the top-left
//! pixel of the reported mouse cell `(col, row)`, which is the sharp lit tip
//! of the cheese wedge. Because the wedge expands right and slightly down from
//! that tip:
//!
//! - On a 1-row button or menu label, the tip points cleanly at the target cell
//!   without obscuring text above or to the left, and leaves the bottom half of
//!   the row below uncovered.
//! - On terminals that do not implement OSC 22 (`Apple Terminal`, `WezTerm`),
//!   the right half and lower face of the wedge extend beside and below the OS
//!   arrow pointer rather than disappearing behind it.
//! - On terminals that support OSC 22 (`Ghostty`, `Kitty`, `iTerm2`), the real
//!   pointer switches to a thin `crosshair` centered on the tip while the cheese
//!   is visible.
//!
//! # Zero-Allocation Contract
//!
//! [`draw`] and [`draw_with_depth`] operate purely on stack arrays and write at
//! most 8 cells directly into ratatui's [`Buffer`]. No heap allocation happens
//! per frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::render::palette::{self, Rgb, CHEESE_RGB};
use crate::term::ColorDepth;

/// Cursor bounding box in terminal cells (width × height).
/// The resting wedge uses `4 × 3` half-block pixels (`1.5` cells tall); the
/// fourth half-row (`py = 3`) is used only for the pressed-down `Held` squash.
pub const WIDTH: u16 = 4;
pub const HEIGHT: u16 = 2;

/// Half-block pixel height (`HEIGHT * 2`).
const PIXEL_W: usize = WIDTH as usize;
const PIXEL_H: usize = (HEIGHT as usize) * 2;

/// Hot spot offset from the reported mouse cell `(col, row)`: the top-left tip
/// pixel of the cheese wedge sits directly on `(col, row)`.
pub const HOTSPOT: (u16, u16) = (0, 0);

/// Seconds of keyboard-only activity before the cursor starts fading.
pub const FADE_START_SECS: f32 = 0.65;
/// Seconds of keyboard-only activity before the cursor hides completely.
pub const FADE_END_SECS: f32 = 1.05;
/// Duration of the click nibble + sparkle pulse.
pub const CLICK_PULSE_SECS: f32 = 0.22;

// ---- Cheese Palette (matching `WeaponKind::CheeseSpray` & `art_traps.rs`) ---

/// Sunlit top-left wedge tip — warm creamy yellow (`LightYellow` in 16-colour).
const CHEESE_TIP: Rgb = (255, 242, 114);
/// Lit upper ridge of the wedge (`LightYellow` in 16-colour).
const CHEESE_HI: Rgb = (255, 226, 94);
/// Main cut cheese face — exact `WeaponKind::CheeseSpray` colour (`palette::CHEESE_RGB`).
const CHEESE_BODY: Rgb = CHEESE_RGB;
/// Slightly warmer lower-cut cheese face for top-left lighting depth (`LightYellow` in 16-colour).
const CHEESE_BODY_LO: Rgb = (244, 194, 58);
/// Aged golden-ochre rind along the right back face (`Yellow` in 16-colour).
const CHEESE_RIND: Rgb = (202, 132, 22);
/// Shaded lower-right rind corner (`Yellow` in 16-colour so the back edge stays solid).
const CHEESE_RIND_DARK: Rgb = (148, 88, 14);
/// Deep amber Swiss cheese hole (`Yellow` in 16-colour, enclosed by `LightYellow`).
const CHEESE_HOLE: Rgb = (120, 68, 12);
/// Dark umber pressed-down rim used on the `Held` squash (`Black` in 16-colour).
const CHEESE_RIM: Rgb = (58, 32, 8);
/// Bright butter-white click sparkle / crumb (`White` in 16-colour).
const CHEESE_SPARKLE: Rgb = (255, 252, 224);

/// Hover brighten target — warm sunlit gold whose low blue channel keeps every
/// lit pixel `LightYellow` (never `Gray`/`White`) even in 16-colour mode.
const HOVER_GLEAM: Rgb = (255, 250, 120);

/// Player preference for the cheese cursor, persisted beside high scores and
/// minimap size. Defaults to [`CursorSetting::On`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorSetting {
    #[default]
    On,
    Off,
}

impl CursorSetting {
    #[inline]
    pub const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }

    #[inline]
    pub const fn toggle(self) -> Self {
        match self {
            Self::On => Self::Off,
            Self::Off => Self::On,
        }
    }

    /// Canonical lowercase string persisted in `scores.json`.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }

    /// Parse a persisted setting string (case-insensitive). Returns `None` on
    /// unrecognized values so callers fall back cleanly to [`Self::On`].
    pub fn parse_setting(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "1" | "yes" | "enabled" => Some(Self::On),
            "off" | "false" | "0" | "no" | "none" | "disabled" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Parse a `RATRUN_CURSOR` environment override value.
///
/// Recognises `off|0|false|no|none|disabled` → `Some(false)` and
/// `on|1|true|yes|enabled` → `Some(true)`. Anything empty or unrecognised
/// returns `None` (deferring to the persisted setting).
pub fn parse_env_override(raw: &str) -> Option<bool> {
    CursorSetting::parse_setting(raw).map(CursorSetting::is_on)
}

/// Read `RATRUN_CURSOR` from the process environment once at app startup.
pub fn env_override() -> Option<bool> {
    std::env::var("RATRUN_CURSOR")
        .ok()
        .as_deref()
        .and_then(parse_env_override)
}

/// Per-frame visibility and animation state owned by `App`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CursorState {
    /// Whether any mouse movement or click has arrived in this session.
    seen_mouse: bool,
    /// True once keyboard activity has occurred without subsequent mouse input
    /// (or when movement keys reclaimed control during play).
    kb_hiding: bool,
    /// Elapsed seconds in `kb_hiding` without mouse activity.
    hide_timer: f32,
    /// Remaining seconds of the click nibble/sparkle pulse.
    click_timer: f32,
    /// Continuous phase clock for the subtle bait-bob during mouse steering.
    bob_time: f32,
}

/// Per-frame input signals fed into [`CursorState::update`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CursorInput {
    /// Mouse moved, dragged, pressed, or released this frame.
    pub mouse_active: bool,
    /// A mouse button was pressed this frame.
    pub mouse_clicked: bool,
    /// Any non-toggle keyboard press occurred this frame.
    pub kb_pressed: bool,
    /// A movement key is currently held.
    pub kb_moving: bool,
    /// `InputState::prefer_mouse` (false once movement keys reclaim steering).
    pub prefer_mouse: bool,
    /// True on `Screen::Playing`.
    pub playing: bool,
}

impl CursorState {
    /// Advance cursor visibility and micro-animation state by one frame.
    pub fn update(&mut self, dt: f32, inp: CursorInput) {
        let dt = dt.max(0.0);
        if inp.mouse_active {
            self.seen_mouse = true;
            self.kb_hiding = false;
            self.hide_timer = 0.0;
        }
        if inp.mouse_clicked {
            self.click_timer = CLICK_PULSE_SECS;
        } else if self.click_timer > 0.0 {
            self.click_timer = (self.click_timer - dt).max(0.0);
        }

        if !inp.mouse_active {
            if inp.playing {
                // During play, Space (dash) / Tab (map) / M (mute) while still
                // steering by mouse must not hide the cheese; only reclaiming
                // steering via movement keys (prefer_mouse == false) hides it.
                if !inp.prefer_mouse || inp.kb_moving {
                    self.kb_hiding = true;
                }
            } else if inp.kb_pressed || inp.kb_moving || !inp.prefer_mouse {
                self.kb_hiding = true;
            }
        }

        if self.kb_hiding {
            self.hide_timer = (self.hide_timer + dt).min(FADE_END_SECS + 1.0);
        } else {
            self.hide_timer = 0.0;
        }

        if self.seen_mouse && !self.kb_hiding && inp.playing && inp.prefer_mouse {
            self.bob_time = (self.bob_time + dt) % 60.0;
        } else {
            self.bob_time = 0.0;
        }
    }

    /// Current opacity in `0.0..=1.0`. `0.0` means completely hidden.
    #[inline]
    pub fn alpha(&self) -> f32 {
        if !self.seen_mouse {
            return 0.0;
        }
        if !self.kb_hiding || self.hide_timer <= FADE_START_SECS {
            return 1.0;
        }
        if self.hide_timer >= FADE_END_SECS {
            return 0.0;
        }
        let span = (FADE_END_SECS - FADE_START_SECS).max(0.001);
        (1.0 - (self.hide_timer - FADE_START_SECS) / span).clamp(0.0, 1.0)
    }

    /// True when the cursor would currently paint at least one pixel.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.alpha() > 0.02
    }

    /// Normalised click pulse strength (`1.0` on press → `0.0` when done).
    #[inline]
    pub fn click_pulse(&self) -> f32 {
        (self.click_timer / CLICK_PULSE_SECS).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn bob_time(&self) -> f32 {
        self.bob_time
    }
}

/// Snapshot of everything needed to render the cheese cursor on one frame.
#[derive(Clone, Copy, Debug)]
pub struct CursorView {
    /// Reported mouse cell `(col, row)` — also the hot spot.
    pub pos: (u16, u16),
    /// True when hovering over a clickable UI target (excludes full-field Dash).
    pub hover: bool,
    /// True while a mouse button is held down.
    pub held: bool,
    /// Normalised click burst (`0.0..=1.0`).
    pub click_pulse: f32,
    /// Continuous phase clock for subtle bait-bob during mouse-steered play.
    pub bob_time: f32,
    /// Opacity (`0.0..=1.0`, fades out during keyboard-only inactivity).
    pub alpha: f32,
    /// True while steering the rat with the mouse in `Playing`.
    pub playing_steer: bool,
}

/// Build the `4 × 4` half-block pixel grid for the current cursor state.
///
/// Layout coordinates: `grid[py][px]` where `px ∈ 0..4` (columns) and
/// `py ∈ 0..4` (half-block rows; cell row 0 = `py 0,1`, cell row 1 = `py 2,3`).
/// Hot spot is `grid[0][0]`.
///
/// The resting wedge occupies a `4 × 3` landscape silhouette (`py = 0..=2`):
/// sunlit top-left tip at `(0, 0)`, a single 4-side-enclosed Swiss hole at
/// `(1, 1)` so it reads unmistakably even in 16-colour mode, warm cheddar
/// shading across the cut face, and a solid 2-pixel back rind along `x = 3`.
fn build_sprite(view: CursorView) -> [[Option<Rgb>; PIXEL_W]; PIXEL_H] {
    let gleam = |c: Rgb| -> Rgb {
        if view.hover {
            palette::mix(c, HOVER_GLEAM, 0.24)
        } else {
            c
        }
    };

    let tip = gleam(CHEESE_TIP);
    let hi = gleam(CHEESE_HI);
    let body = gleam(CHEESE_BODY);
    let body_lo = gleam(CHEESE_BODY_LO);
    let rind = if view.hover {
        palette::mix(CHEESE_RIND, CHEESE_BODY, 0.18)
    } else {
        CHEESE_RIND
    };
    let rind_dark = CHEESE_RIND_DARK;
    let hole = CHEESE_HOLE;
    let rim = CHEESE_RIM;

    // Subtle bait-bob while the rat is chasing the cheese during play: gentle
    // 1.3 Hz sunlit sheen across the top ridge & cut face, keeping the wedge
    // silhouette and hot-spot tip (0,0) rock-steady.
    let bob_up = view.playing_steer && ((view.bob_time * 1.3).fract() < 0.45);

    let mut grid: [[Option<Rgb>; PIXEL_W]; PIXEL_H] = if view.held {
        // Pressed / squished wedge: compresses down 1 half-pixel onto a dark
        // rim shadow while keeping a smooth diagonal slope from tip (0, 0).
        [
            [Some(tip), Some(hi), None, None],
            [Some(body), Some(hole), Some(hi), None],
            [Some(body_lo), Some(body), Some(body_lo), Some(rind)],
            [Some(rim), Some(rim), Some(rim), Some(rind_dark)],
        ]
    } else if view.hover {
        // Hovering over a clickable UI target: brighter gleaming cheese with a
        // crisp mouse-nibble out of the upper-right slope (2, 0), sharpening
        // the triangular wedge while leaving the back rind solid.
        [
            [Some(tip), Some(hi), None, None],
            [Some(hi), Some(hole), Some(body), Some(rind)],
            [Some(body), Some(body_lo), Some(body_lo), Some(rind_dark)],
            [None, None, None, None],
        ]
    } else if bob_up {
        // Gentle play-mode bob: sunlit gleam glides along the ridge & face.
        [
            [Some(tip), Some(tip), Some(hi), None],
            [Some(tip), Some(hole), Some(hi), Some(rind)],
            [Some(hi), Some(body), Some(body), Some(rind)],
            [None, None, None, None],
        ]
    } else {
        // Default 4×3 landscape Swiss cheese wedge pointing top-left.
        [
            [Some(tip), Some(hi), Some(hi), None],
            [Some(hi), Some(hole), Some(body), Some(rind)],
            [Some(body), Some(body_lo), Some(body_lo), Some(rind_dark)],
            [None, None, None, None],
        ]
    };

    // Click feedback: quick nibble notch at `(2, 0)` with a detached butter-
    // white crumb/sparkle popping off the upper-right at `(3, 0)`, keeping the
    // back rind solid.
    if view.click_pulse > 0.05 {
        let sparkle = if view.click_pulse > 0.45 {
            CHEESE_SPARKLE
        } else {
            CHEESE_TIP
        };
        grid[0][2] = None;
        grid[0][3] = Some(sparkle);
    }

    grid
}

/// Extract an RGB approximation from an existing cell colour so fading can
/// blend cleanly into whatever sits beneath the cursor.
#[inline]
fn color_to_rgb(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black | Color::Reset => (11, 15, 17),
        Color::Red => (128, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (128, 128, 0),
        Color::Blue => (0, 0, 128),
        Color::Magenta => (128, 0, 128),
        Color::Cyan => (0, 128, 128),
        Color::Gray => (192, 192, 192),
        Color::DarkGray => (96, 96, 96),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (0, 0, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(idx) => indexed_to_rgb(idx),
    }
}

#[inline]
fn indexed_to_rgb(idx: u8) -> Rgb {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if idx < 16 {
        match idx {
            0 => (0, 0, 0),
            1 => (128, 0, 0),
            2 => (0, 128, 0),
            3 => (128, 128, 0),
            4 => (0, 0, 128),
            5 => (128, 0, 128),
            6 => (0, 128, 128),
            7 => (192, 192, 192),
            8 => (128, 128, 128),
            9 => (255, 0, 0),
            10 => (0, 255, 0),
            11 => (255, 255, 0),
            12 => (0, 0, 255),
            13 => (255, 0, 255),
            14 => (0, 255, 255),
            _ => (255, 255, 255),
        }
    } else if idx < 232 {
        let i = (idx - 16) as usize;
        (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
    } else {
        let v = 8 + 10 * (idx - 232);
        (v, v, v)
    }
}

#[inline]
fn blend_alpha(src: Rgb, dst: Color, alpha: f32) -> Rgb {
    if alpha >= 0.99 {
        src
    } else {
        palette::mix(color_to_rgb(dst), src, alpha)
    }
}

/// Composite the cheese cursor onto `buf` inside `area`, using the resolved
/// terminal colour depth. Draws nothing when `view.alpha <= 0.02` or when the
/// area is empty. Clips cleanly at all four edges and never allocates.
pub fn draw(buf: &mut Buffer, area: Rect, view: CursorView) {
    draw_with_depth(buf, area, view, crate::term::color_depth());
}

/// Like [`draw`], with an explicit [`ColorDepth`] for deterministic testing.
pub fn draw_with_depth(buf: &mut Buffer, area: Rect, view: CursorView, depth: ColorDepth) {
    let alpha = view.alpha.clamp(0.0, 1.0);
    if alpha <= 0.02 || area.width == 0 || area.height == 0 {
        return;
    }

    let (origin_x, origin_y) = view.pos;
    let right = area.x.saturating_add(area.width);
    let bottom = area.y.saturating_add(area.height);
    let grid = build_sprite(view);

    for dy in 0..HEIGHT {
        let Some(cy) = origin_y.checked_add(dy) else {
            continue;
        };
        if cy < area.y || cy >= bottom {
            continue;
        }
        let top_row = (dy as usize) * 2;
        let bot_row = top_row + 1;

        for dx in 0..WIDTH {
            let Some(cx) = origin_x.checked_add(dx) else {
                continue;
            };
            if cx < area.x || cx >= right {
                continue;
            }
            let top = grid[top_row][dx as usize];
            let bot = grid[bot_row][dx as usize];
            if top.is_none() && bot.is_none() {
                continue;
            }

            let Some(cell) = buf.cell_mut((cx, cy)) else {
                continue;
            };

            // Respect REVERSED style on highlighted buttons before reading the
            // cell's existing top/bottom colours, then strip modifiers so the
            // cheese pixels never invert or pick up underlines.
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (raw_fg, raw_bg) = if reversed {
                (cell.bg, cell.fg)
            } else {
                (cell.fg, cell.bg)
            };
            let is_upper_half = cell.symbol() == "▀";
            let is_lower_half = cell.symbol() == "▄";
            let (old_top, old_bot) = if is_upper_half {
                (raw_fg, raw_bg)
            } else if is_lower_half {
                (raw_bg, raw_fg)
            } else {
                (raw_bg, raw_bg)
            };

            cell.modifier = Modifier::empty();

            match (top, bot) {
                (Some(t), Some(b)) => {
                    let tf = palette::to_term(blend_alpha(t, old_top, alpha), depth);
                    let bf = palette::to_term(blend_alpha(b, old_bot, alpha), depth);
                    cell.set_symbol("▀");
                    cell.set_fg(tf);
                    cell.set_bg(bf);
                }
                (Some(t), None) => {
                    let tf = palette::to_term(blend_alpha(t, old_top, alpha), depth);
                    cell.set_symbol("▀");
                    cell.set_fg(tf);
                    cell.set_bg(old_bot);
                }
                (None, Some(b)) => {
                    let bf = palette::to_term(blend_alpha(b, old_bot, alpha), depth);
                    if is_upper_half {
                        cell.set_symbol("▀");
                        cell.set_fg(old_top);
                        cell.set_bg(bf);
                    } else {
                        // Use lower half block `▄` over text/UI cells so a
                        // `Color::Reset` background stays `Color::Reset`.
                        cell.set_symbol("▄");
                        cell.set_fg(bf);
                        cell.set_bg(old_top);
                    }
                }
                (None, None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{sanitize_with, GlyphSet};

    fn view_at(x: u16, y: u16) -> CursorView {
        CursorView {
            pos: (x, y),
            hover: false,
            held: false,
            click_pulse: 0.0,
            bob_time: 0.0,
            alpha: 1.0,
            playing_steer: false,
        }
    }

    #[test]
    fn draws_at_reported_cell_with_top_left_hotspot() {
        let area = Rect::new(0, 0, 10, 6);
        let mut buf = Buffer::empty(area);
        draw_with_depth(&mut buf, area, view_at(3, 2), ColorDepth::Truecolor);

        let tip = &buf[(3, 2)];
        assert_eq!(tip.symbol(), "▀");
        assert_eq!(tip.fg, Color::Rgb(CHEESE_TIP.0, CHEESE_TIP.1, CHEESE_TIP.2));
        assert_eq!(tip.bg, Color::Rgb(CHEESE_HI.0, CHEESE_HI.1, CHEESE_HI.2));

        // Enclosed Swiss hole sits inside cell (4, 2) bottom pixel; sunlit ridge on top.
        let hole_cell = &buf[(4, 2)];
        assert_eq!(hole_cell.symbol(), "▀");
        assert_eq!(
            hole_cell.fg,
            Color::Rgb(CHEESE_HI.0, CHEESE_HI.1, CHEESE_HI.2)
        );
        assert_eq!(
            hole_cell.bg,
            Color::Rgb(CHEESE_HOLE.0, CHEESE_HOLE.1, CHEESE_HOLE.2)
        );

        // Shaded back rind corner anchors bottom-right of the wedge at (6, 3) top pixel.
        let rim_cell = &buf[(6, 3)];
        assert_eq!(rim_cell.symbol(), "▀");
        assert_eq!(
            rim_cell.fg,
            Color::Rgb(
                CHEESE_RIND_DARK.0,
                CHEESE_RIND_DARK.1,
                CHEESE_RIND_DARK.2
            )
        );

        // Cells outside the 4×2 footprint stay untouched.
        assert_eq!(buf[(2, 2)].symbol(), " ");
        assert_eq!(buf[(7, 2)].symbol(), " ");
        assert_eq!(buf[(3, 4)].symbol(), " ");
    }

    #[test]
    fn clips_cleanly_at_all_four_edges_and_tiny_terminals() {
        // 1×1 terminal: paints only the hot-spot tip cell, never panics.
        let one = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(one);
        draw_with_depth(&mut buf, one, view_at(0, 0), ColorDepth::Truecolor);
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(
            buf[(0, 0)].fg,
            Color::Rgb(CHEESE_TIP.0, CHEESE_TIP.1, CHEESE_TIP.2)
        );

        // 0×0 terminal: no-op.
        let zero = Rect::new(0, 0, 0, 0);
        let mut zbuf = Buffer::empty(zero);
        draw_with_depth(&mut zbuf, zero, view_at(0, 0), ColorDepth::Truecolor);

        // All four edges + corners + u16::MAX overflow guard on 12×6 buffer.
        let area = Rect::new(0, 0, 12, 6);
        for &(x, y) in &[
            (0, 0),
            (11, 0),
            (0, 5),
            (11, 5),
            (10, 5),
            (11, 4),
            (12, 6),
            (u16::MAX, u16::MAX),
        ] {
            let mut b = Buffer::empty(area);
            draw_with_depth(&mut b, area, view_at(x, y), ColorDepth::Truecolor);
            if x < area.width && y < area.height {
                assert_eq!(b[(x, y)].symbol(), "▀", "tip missing at ({x},{y})");
            }
        }
    }

    #[test]
    fn preserves_underlying_half_block_pixel_on_transparent_half() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        let floor_top = Color::Rgb(14, 22, 20);
        let floor_bot = Color::Rgb(40, 70, 64);
        // Cell (5, 1) is the upper-right of the 4×2 cursor at (2, 1):
        // top pixel is transparent (None), bottom pixel is CHEESE_RIND.
        buf[(5, 1)].set_symbol("▀");
        buf[(5, 1)].set_fg(floor_top);
        buf[(5, 1)].set_bg(floor_bot);

        draw_with_depth(&mut buf, area, view_at(2, 1), ColorDepth::Truecolor);

        assert_eq!(buf[(5, 1)].symbol(), "▀");
        assert_eq!(
            buf[(5, 1)].fg, floor_top,
            "transparent top half must preserve the playfield pixel beneath"
        );
        assert_eq!(
            buf[(5, 1)].bg,
            Color::Rgb(CHEESE_RIND.0, CHEESE_RIND.1, CHEESE_RIND.2)
        );
    }

    #[test]
    fn strips_reversed_modifier_on_highlighted_ui_cells() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        buf[(1, 1)].set_symbol("S");
        buf[(1, 1)].set_fg(Color::Rgb(11, 15, 17));
        buf[(1, 1)].set_bg(Color::Rgb(235, 232, 232));
        buf[(1, 1)].modifier = Modifier::REVERSED | Modifier::BOLD;

        draw_with_depth(&mut buf, area, view_at(1, 1), ColorDepth::Truecolor);
        assert_eq!(buf[(1, 1)].modifier, Modifier::empty());
        assert_eq!(buf[(1, 1)].symbol(), "▀");
    }

    #[test]
    fn degrades_to_yellow_in_256_and_16_colour_and_survives_safe_glyphs() {
        let area = Rect::new(0, 0, 8, 4);

        // 16-colour mode: tip and cut face read as LightYellow, enclosed hole
        // and back rind read as Yellow (ochre).
        let mut b16 = Buffer::empty(area);
        draw_with_depth(&mut b16, area, view_at(1, 1), ColorDepth::Ansi16);
        assert_eq!(b16[(1, 1)].fg, Color::LightYellow, "tip should be yellow");
        assert_eq!(b16[(1, 1)].bg, Color::LightYellow, "hi should be yellow");
        assert_eq!(b16[(2, 1)].bg, Color::Yellow, "enclosed hole should read as ochre");
        assert_eq!(b16[(3, 1)].bg, Color::LightYellow, "body should be yellow");
        assert_eq!(b16[(4, 1)].fg, Color::Yellow, "rind should be ochre yellow");
        assert_eq!(b16[(4, 2)].fg, Color::Yellow, "lower rind stays solid ochre");

        // Hover state in 16-colour mode must also remain yellow (never wash out to Gray/White).
        let mut h16 = Buffer::empty(area);
        let mut hv = view_at(1, 1);
        hv.hover = true;
        draw_with_depth(&mut h16, area, hv, ColorDepth::Ansi16);
        assert_eq!(h16[(1, 1)].fg, Color::LightYellow, "hover tip stays yellow");
        assert_eq!(h16[(2, 1)].fg, Color::LightYellow, "hover hi stays yellow");
        assert_eq!(h16[(2, 1)].bg, Color::Yellow, "hover hole stays ochre");
        // Hover cell (3, 1) has top nibbled (None) and bottom = body, so over a
        // blank cell it draws `▄` with fg = LightYellow.
        assert_eq!(h16[(3, 1)].symbol(), "▄");
        assert_eq!(h16[(3, 1)].fg, Color::LightYellow, "hover body stays yellow");

        // 256-colour mode: all painted colours are Indexed, never Rgb.
        let mut b256 = Buffer::empty(area);
        draw_with_depth(&mut b256, area, view_at(1, 1), ColorDepth::Ansi256);
        for y in 1..=2 {
            for x in 1..=4 {
                assert!(
                    matches!(b256[(x, y)].fg, Color::Indexed(_)),
                    "256-colour fg must be Indexed at ({x},{y})"
                );
            }
        }

        // Safe glyph set keeps `▀` and `▄` intact.
        let mut bsafe = Buffer::empty(area);
        draw_with_depth(&mut bsafe, area, view_at(1, 1), ColorDepth::Truecolor);
        let before: Vec<String> = (1..=2)
            .flat_map(|y| (1..=4).map(move |x| (x, y)))
            .map(|(x, y)| bsafe[(x, y)].symbol().to_string())
            .collect();
        sanitize_with(&mut bsafe, GlyphSet::Safe, ColorDepth::Truecolor);
        let after: Vec<String> = (1..=2)
            .flat_map(|y| (1..=4).map(move |x| (x, y)))
            .map(|(x, y)| bsafe[(x, y)].symbol().to_string())
            .collect();
        assert_eq!(before, after, "cursor glyphs must belong to the safe set");
    }

    #[test]
    fn state_appears_on_mouse_hides_on_keyboard_and_returns_instantly() {
        let mut st = CursorState::default();
        assert!(!st.is_visible(), "hidden before any mouse activity");

        // Mouse moves → immediately visible at full opacity.
        st.update(
            1.0 / 60.0,
            CursorInput {
                mouse_active: true,
                prefer_mouse: true,
                playing: true,
                ..Default::default()
            },
        );
        assert_eq!(st.alpha(), 1.0);
        assert!(st.is_visible());

        // Keyboard movement reclaims steering → stays briefly, fades, then hides.
        st.update(
            0.5,
            CursorInput {
                kb_pressed: true,
                kb_moving: true,
                playing: true,
                ..Default::default()
            },
        );
        assert!(st.is_visible(), "still visible during brief grace");
        st.update(
            0.35,
            CursorInput {
                kb_moving: true,
                playing: true,
                ..Default::default()
            },
        );
        assert!(
            st.alpha() > 0.0 && st.alpha() < 1.0,
            "should be mid-fade at 0.85s (got {})",
            st.alpha()
        );
        st.update(
            0.3,
            CursorInput {
                kb_moving: true,
                playing: true,
                ..Default::default()
            },
        );
        assert!(!st.is_visible(), "must hide after keyboard-only inactivity");
        assert_eq!(st.alpha(), 0.0);

        // Fresh mouse move → returns immediately at full opacity.
        st.update(
            1.0 / 60.0,
            CursorInput {
                mouse_active: true,
                prefer_mouse: true,
                playing: true,
                ..Default::default()
            },
        );
        assert_eq!(st.alpha(), 1.0);
        assert!(st.is_visible());
    }

    #[test]
    fn click_pulse_and_env_override_parsing() {
        let mut st = CursorState::default();
        st.update(
            0.016,
            CursorInput {
                mouse_active: true,
                mouse_clicked: true,
                prefer_mouse: true,
                playing: true,
                ..Default::default()
            },
        );
        assert_eq!(st.click_pulse(), 1.0);
        st.update(
            0.11,
            CursorInput {
                prefer_mouse: true,
                playing: true,
                ..Default::default()
            },
        );
        assert!(st.click_pulse() > 0.4 && st.click_pulse() < 0.6);
        st.update(
            0.2,
            CursorInput {
                prefer_mouse: true,
                playing: true,
                ..Default::default()
            },
        );
        assert_eq!(st.click_pulse(), 0.0);

        assert_eq!(parse_env_override("off"), Some(false));
        assert_eq!(parse_env_override("OFF"), Some(false));
        assert_eq!(parse_env_override("0"), Some(false));
        assert_eq!(parse_env_override("false"), Some(false));
        assert_eq!(parse_env_override("on"), Some(true));
        assert_eq!(parse_env_override("1"), Some(true));
        assert_eq!(parse_env_override(""), None);
        assert_eq!(parse_env_override("cheddar"), None);
    }
}
