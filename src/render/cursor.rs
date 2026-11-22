//! Cheese mouse cursor — a tiny half-block Swiss cheese wedge drawn on top of
//! every normal-game screen while the player is using the mouse.
//!
//! It is the **only** pointer, in every control scheme: in `Scheme::MouseAim`
//! the pointer is the aim point, so this wedge is the reticle too (there is no
//! crosshair). The hot spot is the tip, in the reported mouse cell, and that
//! cell is the cell whose centre `App::aim_point` hands to `TickInput.aim` —
//! so the shot goes where the tip points, and the aim maths never reads the
//! cursor at all.
//!
//! # Geometry & Hot Spot
//!
//! The cursor occupies a `6 × 2` terminal-cell bounding box (`6 × 4` square
//! half-block pixels using `▀` and `▄`) holding a cheese wedge tilted about
//! 25° clockwise, so it lies the way an arrow pointer does: the point at the
//! top-left, the body trailing down and to the right, the rind across the far
//! end. Its **hot spot** is the wedge's pointed tip, the top-left pixel of the
//! box, which sits in the upper half of the reported mouse cell `(col, row)`;
//! the rest of the wedge falls away to the right and into the row below.
//!
//! - On a 1-row button or menu label, the tip points at the target cell and
//!   only the thin end of the wedge covers text; nothing above the pointer or
//!   to its left is hidden.
//! - On terminals that do not implement OSC 22 (`Apple Terminal`, `WezTerm`),
//!   the wedge lies under the OS arrow pointer in the same orientation, so
//!   the two read as one pointer.
//! - On terminals that support OSC 22 (`Ghostty`, `Kitty`, `iTerm2`), the real
//!   pointer switches to a thin `crosshair` centered on the tip while the cheese
//!   is visible.
//! - A one-pixel dark keyline runs along the two exposed edges, so the wedge
//!   keeps its silhouette over a white explosion, a Squeak Nova ring or a
//!   Cheese Spray pellet of its own colour, and over the floor's darkest
//!   corners (in 16 colours it folds to black).
//!
//! # Where it is stamped
//!
//! [`draw`] writes ratatui cells, in **screen space**, from `App::render` after
//! the playfield has been blitted — after the half-block picture, after the
//! sub-cell encoder turned it into quadrant/sextant/octant glyphs, after the
//! lighting and the hostile layer, after the overlays, the HUD and the
//! minimap. Nothing runs after it but `term::sanitize` (whose safe set holds
//! `▀`/`▄`) and the presenter, so the _smoothing_ and the sub-cell encoder
//! cannot eat it: they only ever see the picture underneath. It is not part of
//! the cached world picture (`PictureKey`), so it never forces a redraw.
//!
//! # Zero-Allocation Contract
//!
//! [`draw`] and [`draw_with_depth`] operate purely on stack arrays and write at
//! most 12 cells directly into ratatui's [`Buffer`]. No heap allocation happens
//! per frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::render::palette::{self, Rgb, CHEESE_RGB};
use crate::term::ColorDepth;

/// Cursor bounding box in terminal cells (width × height).
/// The wedge's tip is the top-left pixel (`px = 0`, `py = 0`); the box hangs
/// from the mouse cell, which is its top-left cell.
pub const WIDTH: u16 = 6;
pub const HEIGHT: u16 = 2;

/// Half-block pixel height (`HEIGHT * 2`).
const PIXEL_W: usize = WIDTH as usize;
const PIXEL_H: usize = (HEIGHT as usize) * 2;

/// Hot spot as a cell offset inside the bounding box: the wedge tip is in the
/// box's top-left cell, which is drawn on the reported mouse cell.
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
/// One-pixel keyline along the wedge's two exposed edges (`Black` in
/// 16-colour). The cut faces are the brightest thing on the playfield, so over
/// a white explosion or a `CheeseSpray` pellet — which is exactly
/// `CHEESE_BODY` — the wedge would otherwise dissolve into its own colour.
const CHEESE_EDGE: Rgb = (26, 18, 10);
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
    /// True in `Scheme::MouseAim`, where the pointer is the **aim point** rather
    /// than a steering device: there the wedge is the reticle, the keyboard is
    /// the only way to move, and so moving must not retire the pointer (only
    /// the `ToggleCursor` action and the cursor option do).
    pub mouse_aims: bool,
    /// True on `Screen::Playing`.
    pub playing: bool,
}

impl CursorInput {
    /// Does this frame's keyboard activity retire the pointer?
    ///
    /// During play, Space (dash) / Tab (map) / M (mute) while still steering by
    /// mouse must not hide the cheese; only reclaiming steering via movement
    /// keys (`prefer_mouse == false`) hides it. Mouse aim is the exception:
    /// WASD is the only way to move there, the pointer is the aiming device and
    /// the wedge is its reticle, so walking while the pointer holds still must
    /// keep it on screen. Off the field, any keyboard activity does.
    fn keyboard_retires(&self) -> bool {
        if self.playing {
            !self.mouse_aims && (!self.prefer_mouse || self.kb_moving)
        } else {
            self.kb_pressed || self.kb_moving || !self.prefer_mouse
        }
    }
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

        if !inp.mouse_active && inp.keyboard_retires() {
            self.kb_hiding = true;
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

/// Build the `6 × 4` half-block pixel grid for the current cursor state.
///
/// Layout coordinates: `grid[py][px]` where `px ∈ 0..6` (columns) and
/// `py ∈ 0..4` (half-block rows; cell row 0 = `py 0,1`, cell row 1 = `py 2,3`).
/// Hot spot is the tip at `grid[0][0]`.
///
/// The wedge points up and to the left, like an arrow pointer (drawn by hand
/// for this size, not resampled):
///
/// ```text
///   T H H e e e      T tip (hot spot)   H lit upper edge
///   e H B B H R      B cut face         L lower, warmer face
///   . e B o B R      o Swiss hole       R rind   D shaded rind corner
///   . . e L L D      e dark keyline
/// ```
///
/// The upper edge drops one pixel on its way right and the lower edge falls
/// at 45°, so the body widens away from the tip to a three-pixel rind across
/// the far end (`x = 5`). The hole at `(3, 2)` is enclosed on all four sides
/// so it reads even in 16-colour mode. The keyline fills the six box pixels
/// along the two exposed edges — it never covers a cheese pixel, so the
/// silhouette and the hot-spot tip stay exactly where they were.
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
    // 1.3 Hz sunlit sheen across the upper edge & cut face, keeping the wedge
    // silhouette and hot-spot tip (0, 0) rock-steady.
    let bob_up = view.playing_steer && ((view.bob_time * 1.3).fract() < 0.45);

    let n = None;
    let mut grid: [[Option<Rgb>; PIXEL_W]; PIXEL_H] = if view.held {
        // Pressed: the wedge is pushed down onto a dark rim along its lower
        // edge. The tip, the upper edge and the rind do not move.
        [
            [Some(tip), Some(hi), Some(hi), n, n, n],
            [n, Some(rim), Some(body), Some(body), Some(hi), Some(rind)],
            [n, n, Some(rim), Some(hole), Some(body), Some(rind)],
            [n, n, n, Some(rim), Some(rim), Some(rind_dark)],
        ]
    } else if bob_up {
        // Gentle play-mode bob: the sunlit gleam glides down the upper edge.
        [
            [Some(tip), Some(tip), Some(hi), n, n, n],
            [n, Some(tip), Some(hi), Some(hi), Some(tip), Some(rind)],
            [n, n, Some(hi), Some(hole), Some(body), Some(rind)],
            [n, n, n, Some(body), Some(body_lo), Some(rind_dark)],
        ]
    } else {
        // Default wedge (hover only brightens it via `gleam`): the tip at the
        // hot spot, a lit upper edge, the cut face widening down and to the
        // right, and the rind standing across the far end.
        [
            [Some(tip), Some(hi), Some(hi), n, n, n],
            [n, Some(hi), Some(body), Some(body), Some(hi), Some(rind)],
            [n, n, Some(body), Some(hole), Some(body), Some(rind)],
            [n, n, n, Some(body_lo), Some(body_lo), Some(rind_dark)],
        ]
    };

    // One pixel of dark keyline along the two exposed edges: above the upper
    // edge's descent `(3..=5, 0)` and under the 45° lower edge
    // `(0, 1), (1, 2), (2, 3)`. Only still-empty pixels are filled, so the
    // pressed rim, the bob's sheen and the click crumb keep their own pixels
    // and no cheese pixel is darkened.
    for (px, py) in [(3, 0), (4, 0), (5, 0), (0, 1), (1, 2), (2, 3)] {
        if grid[py][px].is_none() {
            grid[py][px] = Some(CHEESE_EDGE);
        }
    }

    // Click feedback: a nibble out of the upper edge at `(2, 0)` and a
    // butter-white crumb popping off beyond it at `(4, 0)`.
    if view.click_pulse > 0.05 {
        let sparkle = if view.click_pulse > 0.45 {
            CHEESE_SPARKLE
        } else {
            CHEESE_TIP
        };
        grid[0][2] = None;
        grid[0][4] = Some(sparkle);
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
        // The box hangs from the mouse cell: its top row is the mouse row.
        let Some(cy) = origin_y
            .checked_add(dy)
            .and_then(|y| y.checked_sub(HOTSPOT.1))
        else {
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

    fn rgb(c: Rgb) -> Color {
        Color::Rgb(c.0, c.1, c.2)
    }

    /// The wedge has no crosshair form left: it is the pointer in every
    /// scheme, so `Scheme::MouseAim` draws exactly the cheese `Scheme::Classic`
    /// does (pinned end to end by `app::tests`). No reticle glyph is left on
    /// the hot spot or on its four arms.
    #[test]
    fn the_wedge_is_the_only_pointer() {
        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        draw_with_depth(&mut buf, area, view_at(4, 2), ColorDepth::Truecolor);
        for (x, y) in [(4u16, 2u16), (3, 2), (5, 2), (4, 1), (4, 3)] {
            assert!(
                !matches!(buf[(x, y)].symbol(), "•" | "─" | "│"),
                "reticle glyph left at ({x},{y})"
            );
        }
        assert_eq!(buf[(4, 2)].symbol(), "▀", "the tip cell is the wedge's top half");
        assert_eq!(buf[(4, 2)].fg, rgb(CHEESE_TIP));
    }

    #[test]
    fn draws_wedge_with_tip_on_the_reported_cell() {
        let area = Rect::new(0, 0, 10, 6);
        let mut buf = Buffer::empty(area);
        draw_with_depth(&mut buf, area, view_at(3, 2), ColorDepth::Truecolor);

        // The tip is the upper half of the mouse cell, at the box's top-left;
        // the keyline's shadow is under it, and nothing is drawn above it.
        let tip = &buf[(3, 2)];
        assert_eq!(tip.symbol(), "▀");
        assert_eq!(tip.fg, rgb(CHEESE_TIP));
        assert_eq!(tip.bg, rgb(CHEESE_EDGE), "the keyline sits under the hot spot");
        assert_eq!(buf[(3, 1)].symbol(), " ", "nothing above the tip");
        assert_eq!(buf[(3, 3)].symbol(), " ", "nothing under the tip: the body trails right");
        assert_eq!(
            buf[(4, 3)].fg,
            rgb(CHEESE_EDGE),
            "the keyline follows the lower edge down at 45 degrees"
        );

        // The body widens down and to the right: the upper edge is lit, the
        // cut face starts under it.
        assert_eq!(buf[(4, 2)].fg, rgb(CHEESE_HI));
        assert_eq!(buf[(4, 2)].bg, rgb(CHEESE_HI));
        assert_eq!(buf[(5, 2)].fg, rgb(CHEESE_HI));
        assert_eq!(buf[(5, 2)].bg, rgb(CHEESE_BODY));
        assert_eq!(
            buf[(6, 2)].fg,
            rgb(CHEESE_EDGE),
            "the keyline closes the open air above the falling upper edge"
        );
        assert_eq!(buf[(6, 2)].bg, rgb(CHEESE_BODY));
        assert_eq!(buf[(5, 3)].symbol(), "▀");
        assert_eq!(buf[(5, 3)].fg, rgb(CHEESE_BODY));
        assert_eq!(buf[(5, 3)].bg, rgb(CHEESE_EDGE), "the keyline under the cut face");

        // Enclosed Swiss hole: top pixel of cell (6, 3), the lower face beneath.
        let hole_cell = &buf[(6, 3)];
        assert_eq!(hole_cell.symbol(), "▀");
        assert_eq!(hole_cell.fg, rgb(CHEESE_HOLE));
        assert_eq!(hole_cell.bg, rgb(CHEESE_BODY_LO));
        assert_eq!(buf[(7, 3)].fg, rgb(CHEESE_BODY), "cheese right of the hole");

        // The rind stands across the far end, three pixels at x = 5; the
        // pixel above it is the keyline, which is the tilt of the upper edge.
        assert_eq!(buf[(7, 2)].symbol(), "▀");
        assert_eq!(buf[(7, 2)].fg, rgb(CHEESE_EDGE));
        assert_eq!(buf[(7, 2)].bg, rgb(CHEESE_HI));
        assert_eq!(buf[(8, 2)].symbol(), "▀", "the top-right pixel of the box is keylined");
        assert_eq!(buf[(8, 2)].fg, rgb(CHEESE_EDGE));
        assert_eq!(buf[(8, 2)].bg, rgb(CHEESE_RIND));
        assert_eq!(buf[(8, 3)].fg, rgb(CHEESE_RIND));
        assert_eq!(buf[(8, 3)].bg, rgb(CHEESE_RIND_DARK));

        // Cells outside the 6×2 footprint stay untouched.
        assert_eq!(buf[(2, 2)].symbol(), " ");
        assert_eq!(buf[(9, 3)].symbol(), " ");
        assert_eq!(buf[(5, 4)].symbol(), " ");
        assert_eq!(buf[(5, 1)].symbol(), " ");
    }

    #[test]
    fn clips_cleanly_at_all_four_edges_and_tiny_terminals() {
        // 1×1 terminal: paints only the hot-spot tip cell, never panics.
        let one = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(one);
        draw_with_depth(&mut buf, one, view_at(0, 0), ColorDepth::Truecolor);
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(buf[(0, 0)].fg, rgb(CHEESE_TIP));

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
        // A cell whose bottom half is transparent: (1, 1) of the box, under
        // the keyline that follows the lower edge.
        buf[(3, 2)].set_symbol("▀");
        buf[(3, 2)].set_fg(floor_top);
        buf[(3, 2)].set_bg(floor_bot);
        // A cell whose top half is transparent: the click nibble eats (2, 0)
        // of the box, above the cut face.
        buf[(4, 1)].set_symbol("▀");
        buf[(4, 1)].set_fg(floor_top);
        buf[(4, 1)].set_bg(floor_bot);

        let mut view = view_at(2, 1);
        view.click_pulse = 1.0;
        draw_with_depth(&mut buf, area, view, ColorDepth::Truecolor);

        assert_eq!(buf[(3, 2)].symbol(), "▀");
        assert_eq!(buf[(3, 2)].fg, rgb(CHEESE_EDGE));
        assert_eq!(
            buf[(3, 2)].bg, floor_bot,
            "transparent bottom half must preserve the playfield pixel beneath"
        );
        assert_eq!(buf[(4, 1)].symbol(), "▀");
        assert_eq!(
            buf[(4, 1)].fg, floor_top,
            "transparent top half must preserve the playfield pixel beneath"
        );
        assert_eq!(buf[(4, 1)].bg, rgb(CHEESE_BODY));
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

        // 16-colour mode: tip, upper edge and face read as LightYellow; the
        // enclosed hole and the rind read as Yellow (ochre).
        let mut b16 = Buffer::empty(area);
        draw_with_depth(&mut b16, area, view_at(1, 1), ColorDepth::Ansi16);
        assert_eq!(b16[(1, 1)].fg, Color::LightYellow, "tip should be yellow");
        assert_eq!(b16[(2, 1)].fg, Color::LightYellow, "upper edge should be yellow");
        assert_eq!(b16[(2, 1)].bg, Color::LightYellow, "face should be yellow");
        assert_eq!(b16[(4, 2)].fg, Color::Yellow, "enclosed hole should read as ochre");
        assert_eq!(b16[(4, 2)].bg, Color::LightYellow, "face under the hole is yellow");
        assert_eq!(b16[(3, 2)].fg, Color::LightYellow, "face left of the hole is yellow");
        assert_eq!(b16[(5, 2)].fg, Color::LightYellow, "face right of the hole is yellow");
        assert_eq!(b16[(6, 2)].fg, Color::Yellow, "rind should be ochre yellow");
        assert_eq!(b16[(4, 1)].fg, Color::Black, "the keyline reads black in 16 colours");
        assert_eq!(b16[(4, 1)].bg, Color::LightYellow, "the cut face under it stays yellow");
        assert_eq!(b16[(6, 1)].fg, Color::Black, "the keyline above the rind");
        assert_eq!(b16[(6, 1)].bg, Color::Yellow, "the upper rind stays solid ochre");
        assert_eq!(b16[(5, 1)].fg, Color::Black, "the keyline above the upper edge");
        assert_eq!(b16[(5, 1)].bg, Color::LightYellow, "the upper edge runs to the rind");

        // Hover state in 16-colour mode must also remain yellow (never wash out to Gray/White).
        let mut h16 = Buffer::empty(area);
        let mut hv = view_at(1, 1);
        hv.hover = true;
        draw_with_depth(&mut h16, area, hv, ColorDepth::Ansi16);
        assert_eq!(h16[(1, 1)].fg, Color::LightYellow, "hover tip stays yellow");
        assert_eq!(h16[(4, 2)].fg, Color::Yellow, "hover hole stays ochre");
        // Hover keeps the full silhouette: the face under (3, 0) of the box is
        // still there, only its keyline darkened.
        assert_eq!(h16[(4, 1)].symbol(), "▀");
        assert_eq!(h16[(4, 1)].fg, Color::Black, "the keyline stays dark on hover");
        assert_eq!(h16[(4, 1)].bg, Color::LightYellow, "hover face stays yellow");

        // 256-colour mode: all painted colours are Indexed, never Rgb.
        let mut b256 = Buffer::empty(area);
        draw_with_depth(&mut b256, area, view_at(1, 1), ColorDepth::Ansi256);
        for (x, y) in (1..=6).map(|x| (x, 1)).chain((3..=6).map(|x| (x, 2))) {
            assert!(
                matches!(b256[(x, y)].fg, Color::Indexed(_)),
                "256-colour fg must be Indexed at ({x},{y})"
            );
        }

        // Safe glyph set keeps `▀` and `▄` intact.
        let mut bsafe = Buffer::empty(area);
        draw_with_depth(&mut bsafe, area, view_at(1, 1), ColorDepth::Truecolor);
        let cells = |b: &Buffer| -> Vec<String> {
            (1..=2)
                .flat_map(|y| (1..=6).map(move |x| (x, y)))
                .map(|(x, y)| b[(x, y)].symbol().to_string())
                .collect()
        };
        let before = cells(&bsafe);
        sanitize_with(&mut bsafe, GlyphSet::Safe, ColorDepth::Truecolor);
        assert_eq!(before, cells(&bsafe), "cursor glyphs must belong to the safe set");
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

    /// In `Scheme::MouseAim` the pointer is the **aim point** and WASD is the
    /// only way to move, so walking with the pointer holding still must keep
    /// the wedge on screen (it is the reticle; retiring it would leave the
    /// player aiming blind). Classic and Gamepad keep the old rule, and so does
    /// every menu screen, whatever the scheme.
    #[test]
    fn mouse_aim_keeps_the_wedge_while_the_keyboard_moves() {
        let held = |mouse_aims: bool, playing: bool| {
            let mut st = CursorState::default();
            st.update(
                1.0 / 60.0,
                CursorInput {
                    mouse_active: true,
                    prefer_mouse: true,
                    playing,
                    mouse_aims,
                    ..Default::default()
                },
            );
            // Two seconds of holding a direction key without moving the mouse;
            // the keyboard also clears `prefer_mouse`.
            for _ in 0..120 {
                st.update(
                    1.0 / 60.0,
                    CursorInput {
                        kb_pressed: true,
                        kb_moving: true,
                        playing,
                        mouse_aims,
                        ..Default::default()
                    },
                );
            }
            st
        };

        let aiming = held(true, true);
        assert_eq!(aiming.alpha(), 1.0, "mouse aim: the pointer is the aim point");
        assert!(aiming.is_visible());

        let classic = held(false, true);
        assert_eq!(classic.alpha(), 0.0, "classic: the keyboard reclaims the pointer");
        assert!(!classic.is_visible());

        let menu = held(true, false);
        assert_eq!(menu.alpha(), 0.0, "a menu retires the wedge on a keypress");
        assert!(!menu.is_visible());
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
