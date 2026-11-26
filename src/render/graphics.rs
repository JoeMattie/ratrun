//! The **Graphics** page: the render-only picture settings that used to live on
//! console commands and environment variables alone.
//!
//! Open it with the Graphics action from play or pause, or from the Options
//! plate's `GRAPHICS` row. It draws over the frozen field or the title. Every
//! row cycles with Left/Right, Enter or a click. S cycles smoothing from any
//! row, and the Graphics action or Back returns to the screen that opened it.
//!
//! **Nothing here is simulated.** The lighting quality, the light effects, the
//! 1× edge smoothing, the parallax foreground, the round-glyph particles and the
//! camera's share of the hit shake are all read by the renderer only, so a
//! setting can never move a run, a scripted trace or the map pin. The values
//! beside the other preferences ([`crate::scores::GraphicsFile`]).
//!
//! [`layout`] is the single source of truth for where the rows sit, so the
//! pointer hit rectangles the app registers can never disagree with what the
//! player sees; all state changes go through the app's `adjust_graphics`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::Frame;

use crate::render::menu::{heavy, ink, panel, HitMap, Target};
use crate::render::palette::{COAT, MUTED, PLATE, TRACK};
use crate::render::subcell::Smooth;

/// Row indices in screen order.
pub const LIGHTING_ROW: usize = 0;
pub const FX_ROW: usize = 1;
pub const SMOOTH_ROW: usize = 2;
pub const FOREGROUND_ROW: usize = 3;
pub const GLYPHFX_ROW: usize = 4;
pub const SHAKE_ROW: usize = 5;
pub const BACK_ROW: usize = 6;
/// How many rows the page has.
pub const ROW_COUNT: usize = 7;

/// Preferred panel size; clamped to the terminal.
pub const PANEL_W: u16 = 58;
pub const PANEL_H: u16 = 14;
/// The label column, two cells wider than the longest label (`GLYPH PARTICLES`,
/// fifteen) so a value strip never butts against its label.
const LABEL_W: u16 = 17;

/// The shares of the simulation's shake the SCREEN SHAKE row cycles, ascending.
/// `0.65` is the value every effect had before the row existed, so a fresh
/// profile is unchanged.
pub const SHAKE_STEPS: [f32; 4] = [0.0, 0.5, crate::game::world::SHAKE_SCALE, 1.0];

/// What one row of the page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Lighting quality: the four live modes the console's `lights` cycles.
    Lighting,
    /// The lighting effects layer (`render::lighting::fx`, console `fx`).
    Fx,
    /// The 1× edge treatment (`render::subcell::Smooth`, console `smooth`).
    Smooth,
    /// The parallax overhead layer (console `foreground`).
    Foreground,
    /// The round-glyph particles (console `glyphfx`).
    GlyphFx,
    /// How much of a hit's shake reaches the camera.
    Shake,
    Back,
}

/// The page's rows in order; index with the constants above.
pub const ROWS: [Row; ROW_COUNT] = [
    Row::Lighting,
    Row::Fx,
    Row::Smooth,
    Row::Foreground,
    Row::GlyphFx,
    Row::Shake,
    Row::Back,
];

impl Row {
    /// Label drawn in the row's label column. Player-facing (`STYLE.md`): ALL
    /// CAPS, the concrete thing rather than the internal name of the switch.
    pub fn label(self) -> &'static str {
        match self {
            Row::Lighting => "LIGHTING",
            Row::Fx => "LIGHT EFFECTS",
            Row::Smooth => "SMOOTHING",
            Row::Foreground => "FOREGROUND",
            Row::GlyphFx => "GLYPH PARTICLES",
            Row::Shake => "SCREEN SHAKE",
            Row::Back => "BACK",
        }
    }

    /// The one-line note under the legend for this row (lower-case sentence
    /// fragment, like the Options plate's note).
    pub fn note(self) -> &'static str {
        match self {
            Row::Lighting => " lamp quality; a slow terminal wants coarse",
            Row::Fx => " material shine and overhead shadows",
            Row::Smooth => " edge treatment at 1x",
            Row::Foreground => " overhead pipes, lines and proofs",
            Row::GlyphFx => " motes, sparks and rising bubbles",
            Row::Shake => " how much a hit moves the camera",
            Row::Back => " right click to go back",
        }
    }
}

/// One frame of page state, filled in by the app.
#[derive(Clone, Copy, Debug)]
pub struct GraphicsView<'a> {
    pub lighting: crate::render::lighting::LightingMode,
    pub fx: bool,
    pub smooth: Smooth,
    pub foreground: bool,
    pub glyphfx: bool,
    /// The camera's share of the simulation's shake (`0.0..=1.0`).
    pub shake_scale: f32,
    /// Selected row (`0..ROW_COUNT`).
    pub row: usize,
    /// Accent for the panel edge and the selected row (theme or level material).
    pub accent: Color,
    pub graphics_key: &'a str,
    pub back_key: &'a str,
}

/// Rows for one terminal area.
#[derive(Clone, Copy, Debug)]
pub struct GraphicsLayout {
    pub panel: Rect,
    /// One band per row, `LIGHTING` first.
    pub rows: [Rect; ROW_COUNT],
}

fn band(r: Rect, row: u16, height: u16) -> Rect {
    let row = row.min(r.height);
    Rect::new(r.x, r.y + row, r.width, height.min(r.height - row))
}

/// A row's label/value strip, one cell in from the selection marker column.
///
/// `inner` is the plate's **content** rect, so row 0 sits two rows down: the
/// header owns row 0 and its divider row 1 (the same rule the Options plate
/// follows — a rule drawn through a label is the bug that put it there).
fn row_rect(inner: Rect, index: usize) -> Rect {
    band(inner, 2 + index as u16, 1)
}

/// Where everything sits inside `area`.
pub fn layout(area: Rect) -> GraphicsLayout {
    let card =
        crate::render::menu::centered_rect(PANEL_W.min(area.width), PANEL_H.min(area.height), area);
    // `menu::panel` returns the plate's content rect: the border takes the first
    // and last row, so the content starts at `card.y + 1`.
    let inner = Rect::new(
        card.x.saturating_add(2),
        card.y.saturating_add(1),
        card.width.saturating_sub(4),
        card.height.saturating_sub(2),
    );
    let rows = std::array::from_fn(|i| row_rect(inner, i));
    GraphicsLayout { panel: card, rows }
}

/// The word the page prints for a lighting quality. The four live modes are the
/// console's names in caps; `Off` exists only in tests.
pub fn lighting_label(mode: crate::render::lighting::LightingMode) -> &'static str {
    use crate::render::lighting::LightingMode;
    match mode {
        LightingMode::Full => "FULL",
        LightingMode::Hard => "HARD",
        LightingMode::Coarse => "COARSE",
        // `Compact` is the last live mode; the catch-all also covers the
        // test-only `Off`, which the page never shows.
        _ => "COMPACT",
    }
}

/// Step the lighting quality through
/// [`crate::render::lighting::LightingMode::LIVE`] (the same set the console's
/// `lights` cycles; a mode this build does not run live starts the walk at the
/// front).
pub fn step_lighting(
    cur: crate::render::lighting::LightingMode,
    dir: isize,
) -> crate::render::lighting::LightingMode {
    use crate::render::lighting::LightingMode;
    let live = LightingMode::LIVE;
    let i = live.iter().position(|m| *m == cur).unwrap_or(0);
    live[step_index(i, live.len(), dir)]
}

/// Step the 1× edge treatment through [`Smooth::ALL`], the one list a new level
/// has to be added to.
pub fn step_smooth(cur: Smooth, dir: isize) -> Smooth {
    let all = Smooth::ALL;
    let i = all.iter().position(|s| *s == cur).unwrap_or(0);
    all[step_index(i, all.len(), dir)]
}

/// Step the camera's shake share through [`SHAKE_STEPS`]. An off-ladder saved
/// value snaps to the nearest rung before it moves, so the row can never sit on
/// a value Left/Right cannot leave.
pub fn step_shake(cur: f32, dir: isize) -> f32 {
    let mut i = 0;
    let mut best = f32::INFINITY;
    for (n, step) in SHAKE_STEPS.iter().enumerate() {
        let d = (step - cur).abs();
        if d < best {
            best = d;
            i = n;
        }
    }
    SHAKE_STEPS[step_index(i, SHAKE_STEPS.len(), dir)]
}

/// `i` moved one step in `dir` (right/forward for `dir >= 0`), wrapping.
fn step_index(i: usize, len: usize, dir: isize) -> usize {
    if len == 0 {
        return 0;
    }
    if dir < 0 {
        (i + len - 1) % len
    } else {
        (i + 1) % len
    }
}

/// The shake share as the row prints it (`0%`, `50%`, `65%`, `100%`).
pub fn shake_label(scale: f32) -> String {
    format!("{}%", (scale.clamp(0.0, 1.0) * 100.0).round() as i32)
}

fn on_off(on: bool) -> String {
    if on { "ON".to_string() } else { "OFF".to_string() }
}

/// The value strip's text for `row` (the strip draws its own `‹ ›`).
pub fn value_text(view: &GraphicsView<'_>, row: Row) -> String {
    match row {
        Row::Lighting => lighting_label(view.lighting).to_string(),
        Row::Fx => on_off(view.fx),
        Row::Smooth => view.smooth.label().to_string(),
        Row::Foreground => on_off(view.foreground),
        Row::GlyphFx => on_off(view.glyphfx),
        Row::Shake => shake_label(view.shake_scale),
        Row::Back => String::new(),
    }
}

/// Draw the Graphics plate and register its mouse targets.
pub fn draw(f: &mut Frame, area: Rect, view: &GraphicsView<'_>, hits: &mut HitMap) {
    // Only this page's controls are interactive while it is open (it clears the
    // targets the plate behind it registered, exactly like the Options screen).
    hits.clear();
    if area.is_empty() {
        return;
    }
    let lay = layout(area);
    let tone = view.accent;
    let inner = panel(f, lay.panel, " R-47   /   GRAPHICS ", tone);
    if inner.is_empty() {
        // Degenerate terminal: the plate is all that fits.
        hits.add(lay.panel, Target::GraphicsBack);
        return;
    }
    // The rows come from the layout, so the header and its divider are placed
    // *from the first row up*: nothing here can land on a label.
    let header_y = lay.rows[0].y.saturating_sub(2);
    let rule_y = lay.rows[0].y.saturating_sub(1);
    if inner.width >= 20 && !lay.rows[0].is_empty() {
        f.render_widget(
            Line::styled("PICTURE", heavy(tone)).centered(),
            Rect::new(inner.x, header_y, inner.width, 1),
        );
        f.render_widget(
            Line::styled("─".repeat(usize::from(inner.width)), ink(TRACK)),
            Rect::new(inner.x, rule_y, inner.width, 1),
        );
    }

    for (i, row) in ROWS.iter().enumerate() {
        let r = lay.rows[i];
        if r.is_empty() {
            continue;
        }
        let selected = i == view.row;
        let hovered = hits.hovered == Some(Target::GraphicsRow(i));
        let value_col = if selected { tone } else { MUTED };
        let text_style = if selected {
            Style::default().fg(COAT).add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default().fg(PLATE).bg(TRACK)
        } else {
            ink(COAT)
        };

        // Selection marker column, with a soft band behind the whole row.
        if selected {
            f.buffer_mut().set_style(
                Rect::new(r.x, r.y, lay.panel.width.saturating_sub(4), 1),
                if hovered {
                    Style::default().bg(TRACK)
                } else {
                    Style::default().bg(PLATE)
                },
            );
            f.render_widget(
                Line::styled("▸", heavy(PLATE).bg(tone)),
                Rect::new(r.x, r.y, 1, r.height),
            );
        }
        // Label, one cell in from the marker column.
        let label_rect = Rect::new(
            r.x.saturating_add(1),
            r.y,
            LABEL_W.saturating_sub(1).min(r.width.saturating_sub(1)),
            r.height,
        );
        f.render_widget(
            Line::styled(row.label(), text_style.add_modifier(Modifier::BOLD)),
            label_rect,
        );

        if *row == Row::Back {
            // Back is the way out: show it as a button, not a value.
            let label = " ‹ BACK › ";
            let w = (label.chars().count() as u16).min(r.width);
            let bx = r.x + r.width.saturating_sub(w) / 2;
            let rect = Rect::new(bx, r.y, w, r.height);
            let style = if selected || hovered {
                heavy(PLATE).bg(tone)
            } else {
                heavy(COAT).bg(TRACK)
            };
            f.render_widget(Line::styled(label, style), rect);
            hits.add(r, Target::GraphicsRow(i));
            hits.add(rect, Target::GraphicsBack);
            continue;
        }

        let value = value_text(view, *row);
        let strip = format!(" ‹ {value} › ");
        let w = (strip.chars().count() as u16).min(r.width.saturating_sub(LABEL_W));
        let rect = Rect::new(
            r.x + LABEL_W,
            r.y,
            w.min(r.right().saturating_sub(r.x + LABEL_W)),
            r.height,
        );
        let style = if selected {
            heavy(PLATE).bg(value_col)
        } else if hovered {
            heavy(PLATE).bg(TRACK)
        } else {
            ink(COAT).bg(TRACK)
        };
        f.render_widget(Line::styled(strip, style), rect);
        hits.add(r, Target::GraphicsRow(i));
    }

    // Legend and a note on what the selected row actually changes.
    let legend_y = lay.rows[ROW_COUNT - 1].y.saturating_add(1);
    if inner.height > ROW_COUNT as u16 + 2 {
        let legend = if view.graphics_key == "—" || view.graphics_key == view.back_key {
            format!("S smoothing · {} back", view.back_key)
        } else {
            format!("S smoothing · {} or {} back", view.graphics_key, view.back_key)
        };
        f.render_widget(
            Line::styled(legend, ink(MUTED)).centered(),
            Rect::new(inner.x, legend_y, inner.width, 1),
        );
        // The note keeps a blank row under the legend when the plate has one to
        // spare, closes up when it does not, and never leaves the content rect
        // (on a short terminal there is no note at all rather than one printed
        // over the plate's bottom border).
        let last_row = inner.bottom().saturating_sub(1);
        let note_y = legend_y.saturating_add(2).min(last_row);
        if note_y > legend_y {
            let row = ROWS.get(view.row).copied().unwrap_or(Row::Lighting);
            f.render_widget(
                Line::styled(row.note(), ink(MUTED)).centered(),
                Rect::new(inner.x, note_y, inner.width, 1),
            );
        }
    }

    // Accent the panel's outer edge faintly so the plate reads as focused.
    if lay.panel.width > 2 && lay.panel.height > 1 {
        f.buffer_mut()
            .set_style(Rect::new(lay.panel.x, lay.panel.y, lay.panel.width, 1), ink(tone));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::lighting::LightingMode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn view(row: usize) -> GraphicsView<'static> {
        GraphicsView {
            lighting: LightingMode::Full,
            fx: true,
            smooth: Smooth::Strong,
            foreground: true,
            glyphfx: true,
            shake_scale: crate::game::world::SHAKE_SCALE,
            row,
            accent: Color::Rgb(144, 184, 173),
            graphics_key: "G",
            back_key: "Esc",
        }
    }

    fn frame(w: u16, h: u16, v: &GraphicsView<'_>) -> (String, HitMap) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = HitMap::default();
        term.draw(|f| draw(f, f.area(), v, &mut hits)).unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        (out, hits)
    }

    /// Every row of the page is drawn, clickable and legible at each reference
    /// size, and each one keeps a gap between its label and its value strip.
    #[test]
    fn every_row_is_drawn_and_clickable_at_the_reference_sizes() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitMap::default();
            let v = view(LIGHTING_ROW);
            term.draw(|f| draw(f, f.area(), &v, &mut hits)).unwrap();
            let buf = term.backend().buffer();
            assert!(hits.rect_for(Target::GraphicsBack).is_some(), "{w}x{h}: no Back");
            for (i, row) in ROWS.iter().enumerate().take(ROW_COUNT) {
                let rect = hits
                    .rect_for(Target::GraphicsRow(i))
                    .unwrap_or_else(|| panic!("{w}x{h}: row {i} is not clickable"));
                let drawn: String = (rect.x..rect.right())
                    .map(|x| buf[(x, rect.y)].symbol())
                    .collect();
                let drawn = drawn.trim_end();
                let label = row.label();
                assert!(drawn.contains(label), "{w}x{h}: row {i}: {drawn:?}");
                let after = drawn.split(label).nth(1).unwrap_or("");
                assert!(
                    after.is_empty() || after.starts_with(' '),
                    "{w}x{h}: {label:?} runs into its value: {drawn:?}"
                );
                if drawn.contains('›') {
                    assert_eq!(drawn.matches('‹').count(), 1, "{w}x{h}: {drawn:?}");
                    assert_eq!(drawn.matches('›').count(), 1, "{w}x{h}: {drawn:?}");
                }
            }
            let (text, _) = frame(w, h, &view(LIGHTING_ROW));
            assert!(text.contains("S smoothing · G or Esc back"), "{w}x{h}: no legend:\n{text}");
            assert!(text.contains("GRAPHICS"), "{w}x{h}: no plate title:\n{text}");
        }
    }

    /// The values the page prints, one per row: what the player reads is the
    /// same value the console prints for the same setting.
    #[test]
    fn every_row_prints_its_current_value() {
        let v = view(LIGHTING_ROW);
        assert_eq!(value_text(&v, Row::Lighting), "FULL");
        assert_eq!(value_text(&v, Row::Fx), "ON");
        assert_eq!(value_text(&v, Row::Smooth), "SHARP");
        assert_eq!(value_text(&v, Row::Foreground), "ON");
        assert_eq!(value_text(&v, Row::GlyphFx), "ON");
        assert_eq!(value_text(&v, Row::Shake), "65%");
        assert_eq!(value_text(&v, Row::Back), "");

        let off = GraphicsView {
            lighting: LightingMode::Compact,
            fx: false,
            smooth: Smooth::Off,
            foreground: false,
            glyphfx: false,
            shake_scale: 0.0,
            row: SHAKE_ROW,
            accent: Color::Rgb(0, 0, 0),
            graphics_key: "G",
            back_key: "Esc",
        };
        assert_eq!(value_text(&off, Row::Lighting), "COMPACT");
        assert_eq!(value_text(&off, Row::Fx), "OFF");
        assert_eq!(value_text(&off, Row::Smooth), "OFF");
        assert_eq!(value_text(&off, Row::Foreground), "OFF");
        assert_eq!(value_text(&off, Row::GlyphFx), "OFF");
        assert_eq!(value_text(&off, Row::Shake), "0%");
    }

    /// The rows the player reads carry no internal jargon (`STYLE.md`): an
    /// algorithm's name is not a description of the picture.
    #[test]
    fn the_smoothing_row_does_not_print_an_algorithm_name() {
        for level in Smooth::ALL {
            let label = level.label();
            assert!(!label.contains("EPX"), "{label}");
            assert!(label.chars().all(|c| c.is_ascii_uppercase() || c == ' '));
        }
        assert_eq!(Smooth::label(Smooth::Off), "OFF");
    }

    /// Left and right both reach every value and come back: the row can never
    /// get stuck on a setting, and a fresh profile's shake is the default.
    #[test]
    fn the_rows_cycle_through_every_value_and_wrap() {
        // Lighting: the four live modes, in LIVE order.
        let mut mode = LightingMode::LIVE[0];
        let mut seen = Vec::new();
        for _ in 0..LightingMode::LIVE.len() {
            seen.push(mode);
            mode = step_lighting(mode, 1);
        }
        assert_eq!(seen, LightingMode::LIVE.to_vec());
        assert_eq!(mode, LightingMode::LIVE[0], "right wraps round");
        assert_eq!(
            step_lighting(LightingMode::LIVE[0], -1),
            LightingMode::LIVE[LightingMode::LIVE.len() - 1],
            "left wraps the other way"
        );

        // Smoothing: everything in `Smooth::ALL`.
        let mut level = Smooth::ALL[0];
        let mut seen = Vec::new();
        for _ in 0..Smooth::ALL.len() {
            seen.push(level);
            level = step_smooth(level, 1);
        }
        assert_eq!(seen, Smooth::ALL);
        assert_eq!(step_smooth(Smooth::ALL[Smooth::ALL.len() - 1], 1), Smooth::ALL[0]);

        // Shake: the ladder, and the default sits on it.
        assert!(SHAKE_STEPS.contains(&crate::game::world::SHAKE_SCALE));
        assert_eq!(step_shake(0.65, 1), 1.0);
        assert_eq!(step_shake(1.0, 1), 0.0, "right wraps to the quietest");
        assert_eq!(step_shake(0.0, -1), 1.0, "left wraps to the loudest");
        // An off-ladder value snaps to the nearest rung, then steps.
        assert_eq!(step_shake(0.3, 1), 0.65);
        assert_eq!(shake_label(0.65), "65%");
        assert_eq!(shake_label(1.0), "100%");
        assert_eq!(shake_label(0.0), "0%");
    }

    /// The selected row's note explains the setting, and the note changes with
    /// the selection, so a player who does not know what "SOFT" means can find
    /// out without leaving the page.
    #[test]
    fn the_note_follows_the_selected_row() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let smooth_text = frame(w, h, &view(SMOOTH_ROW)).0;
            assert!(
                smooth_text.contains(Row::Smooth.note().trim()),
                "{w}x{h}: the smoothing note is missing:\n{smooth_text}"
            );
            let shake_text = frame(w, h, &view(SHAKE_ROW)).0;
            assert!(
                shake_text.contains(Row::Shake.note().trim()),
                "{w}x{h}: the shake note is missing:\n{shake_text}"
            );
        }
    }

    /// No plate rule is ever drawn through a row, and the selection marker sits
    /// on the selected row with the accent (the same review find the Options
    /// plate carries a test for).
    #[test]
    fn the_header_rule_sits_above_the_first_row() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitMap::default();
            let v = view(LIGHTING_ROW);
            term.draw(|f| draw(f, f.area(), &v, &mut hits)).unwrap();
            let buf = term.backend().buffer();
            let row = hits.rect_for(Target::GraphicsRow(LIGHTING_ROW)).unwrap();
            let text: String = (row.x..row.right())
                .map(|x| buf[(x, row.y)].symbol())
                .collect();
            assert!(text.contains("LIGHTING"), "{w}x{h}: {text:?}");
            assert!(!text.contains('─'), "{w}x{h}: a rule through the row: {text:?}");
            let marker = &buf[(row.x, row.y)];
            assert_eq!(marker.symbol(), "▸", "{w}x{h}");
            assert_eq!(marker.bg, v.accent, "{w}x{h}");
        }
    }

    #[test]
    fn degenerate_terminals_do_not_panic() {
        for (w, h) in [(0u16, 0u16), (5, 3), (1, 1), (3, 1), (1, 3), (2, 2), (57, 8)] {
            let (text, hits) = frame(w, h, &view(SHAKE_ROW));
            assert_eq!(text.chars().filter(|c| *c == '\n').count(), h as usize);
            let _ = hits.at(0, 0);
        }
    }
}
