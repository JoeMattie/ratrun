//! The Controls screen: scheme selector, a remappable action table and the
//! "reset to defaults" row.
//!
//! Layout is pure: [`layout`] is the single source of truth for where every
//! row sits, so the pointer rectangles the app registers can never disagree
//! with what the player sees. All state changes go through the app's
//! `update_controls` (this module only draws and reports what is under the
//! pointer).
//!
//! The screen is honest about the terminal: a note line under the table prints
//! why a key was refused ([`crate::controls::Unbindable`]) or which keys can
//! never work here (`Cmd`, most `Fn` combinations, `Ctrl+C`).

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::Frame;

use crate::controls::{Action, Controls, Scheme, ACTIONS, ACTION_COUNT};
use crate::render::menu::{centered_rect, heavy, ink, panel, HitMap, Target};
use crate::render::palette::{mix, to_color, COAT, EDGE, MUTED, PLATE, TRACK};

/// Rows in screen order: the scheme selector, every action, reset, back.
pub const SCHEME_ROW: usize = 0;
pub const RESET_ROW: usize = 1 + ACTION_COUNT;
pub const BACK_ROW: usize = 2 + ACTION_COUNT;
pub const ROW_COUNT: usize = 3 + ACTION_COUNT;

/// Row index of an action.
pub fn action_row(action: Action) -> usize {
    1 + action.index()
}

/// Preferred panel size; clamped to the terminal.
pub const PANEL_W: u16 = 66;
pub const PANEL_H: u16 = 28;
const LABEL_W: u16 = 13;
const KEY_W: u16 = 16;
const PAD_W: u16 = 7;

/// One frame of screen state, filled in by the app.
pub struct ControlsView<'a> {
    pub scheme: Scheme,
    pub controls: &'a Controls,
    /// Selected row (`0..ROW_COUNT`).
    pub row: usize,
    /// First visible row of the scrolling table.
    pub scroll: usize,
    /// The action being rebound, if a capture is armed.
    pub capturing: Option<Action>,
    /// A live note (a swap, a refused key, a terminal caveat).
    pub note: &'a str,
    pub accent: Color,
    /// Does this terminal report a lone modifier press?
    pub lone_ctrl: bool,
}

/// Rows and their rectangles for one terminal area.
#[derive(Clone, Copy)]
pub struct ControlsLayout {
    pub panel: Rect,
    pub inner: Rect,
    /// How many table rows fit.
    pub visible: usize,
    /// The first table row drawn.
    pub first: usize,
    pub header_rows: u16,
    pub legend_rows: u16,
}

/// Where the table sits inside `area` for `row`/`scroll`.
pub fn layout(area: Rect, scroll: usize) -> ControlsLayout {
    let panel = centered_rect(PANEL_W.min(area.width), PANEL_H.min(area.height), area);
    let inner = panel.inner(Margin::new(2, 0));
    // header (title + rule), legend (2 lines), note (1), brackets (0)
    let header_rows: u16 = 2;
    let legend_rows: u16 = 2;
    let visible = inner
        .height
        .saturating_sub(header_rows + legend_rows)
        .max(1) as usize;
    let first = scroll.min(ROW_COUNT.saturating_sub(visible));
    ControlsLayout {
        panel,
        inner,
        visible,
        first,
        header_rows,
        legend_rows,
    }
}

/// The text of one row's value column.
fn value_cells(view: &ControlsView, row: usize) -> (String, String, String) {
    if row == SCHEME_ROW {
        return (format!("‹ {} ›", view.scheme.name()), String::new(), String::new());
    }
    if row == RESET_ROW {
        return ("‹ RESET DEFAULTS ›".into(), String::new(), String::new());
    }
    if row == BACK_ROW {
        return ("‹ BACK ›".into(), String::new(), String::new());
    }
    let action = ACTIONS[row - 1];
    let (kbm, pad) = view.controls.row_labels(action);
    let label = if view.capturing == Some(action) {
        "PRESS…".to_string()
    } else if kbm.is_empty() {
        "—".to_string()
    } else {
        kbm
    };
    (label, pad, action.label().to_string())
}

/// Draw the Controls plate and register its mouse targets.
pub fn draw(f: &mut Frame, area: Rect, view: &ControlsView, hits: &mut HitMap) {
    hits.clear();
    if area.is_empty() {
        return;
    }
    let lay = layout(area, view.scroll);
    let tone = view.accent;
    let inner = panel(f, lay.panel, " R-47   /   CONTROLS ", tone);
    if inner.is_empty() {
        hits.add(lay.panel, Target::ControlsBack);
        return;
    }
    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    f.render_widget(
        Line::styled("BINDINGS   /   KEYBOARD · MOUSE · PAD", heavy(tone)).centered(),
        header,
    );
    f.render_widget(
        Line::styled("─".repeat(usize::from(inner.width)), ink(TRACK)),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );

    // The table.
    let table_y = inner.y + lay.header_rows;
    for vis in 0..lay.visible {
        let row = lay.first + vis;
        if row >= ROW_COUNT {
            break;
        }
        let y = table_y + vis as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let r = Rect::new(inner.x, y, inner.width, 1);
        let selected = row == view.row;
        let hovered = hits.hovered == Some(Target::ControlsRow(row));
        if selected {
            // Whole-row band, then the marker on top of it.
            f.buffer_mut().set_style(
                Rect::new(r.x, y, lay.panel.width.saturating_sub(4), 1),
                Style::default().bg(TRACK),
            );
            f.buffer_mut()[(r.x, y)].set_char('▸').set_fg(tone);
        }
        let label_style = if selected {
            Style::default().fg(COAT).add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default().fg(PLATE).bg(TRACK)
        } else {
            ink(COAT)
        };
        let (key, pad, action_label) = value_cells(view, row);
        let (label_text, value_text) = match row {
            SCHEME_ROW | RESET_ROW | BACK_ROW => (String::new(), key.clone()),
            _ => (action_label, key.clone()),
        };
        if !label_text.is_empty() {
            let lr = Rect::new(
                r.x + 1,
                y,
                LABEL_W.min(r.width.saturating_sub(1)),
                1,
            );
            f.render_widget(Line::styled(label_text, label_style), lr);
            let kr = Rect::new(
                (r.x + 1 + LABEL_W).min(r.right()),
                y,
                KEY_W.min(r.right().saturating_sub(r.x + 1 + LABEL_W)),
                1,
            );
            let kstyle = if view.capturing.map(|a| action_row(a) == row).unwrap_or(false) {
                Style::default().fg(PLATE).bg(tone).add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default().fg(tone).add_modifier(Modifier::BOLD)
            } else {
                ink(COAT)
            };
            f.render_widget(Line::styled(value_text, kstyle), kr);
            let pr = Rect::new(
                (r.x + 1 + LABEL_W + KEY_W).min(r.right()),
                y,
                PAD_W.min(r.right().saturating_sub(r.x + 1 + LABEL_W + KEY_W)),
                1,
            );
            if !pr.is_empty() {
                f.render_widget(Line::styled(pad, ink(MUTED)), pr);
            }
        } else {
            // A chrome row: its value is a centred button.
            let w = (value_text.chars().count() as u16 + 2).min(r.width);
            let bx = r.x + r.width.saturating_sub(w) / 2;
            let style = if selected {
                heavy(PLATE).bg(tone)
            } else if hovered {
                heavy(PLATE).bg(TRACK)
            } else {
                heavy(COAT).bg(TRACK)
            };
            f.render_widget(
                Line::styled(format!(" {value_text} "), style),
                Rect::new(bx, y, w, 1),
            );
        }
        let target = match row {
            BACK_ROW => Target::ControlsBack,
            RESET_ROW => Target::ControlsReset,
            _ => Target::ControlsRow(row),
        };
        hits.add(r, target);
    }

    // Legend, note and the terminal caveat.
    let legend_y = (inner.y + inner.height).saturating_sub(lay.legend_rows);
    if inner.height > lay.header_rows + 1 {
        let narrow = inner.width < 56;
        let lines: [&str; 2] = if view.capturing.is_some() {
            [
                "PRESS the new key, mouse or pad button · Esc cancels",
                "a conflict swaps the two bindings · Ctrl+C always quits",
            ]
        } else if narrow {
            [
                "↑↓ row · ←→ change · Enter rebind · R reset",
                "click a row to select · Esc back · wheel scrolls",
            ]
        } else {
            [
                "↑↓ row · ←→ scheme/value · Enter or A rebind · R reset · Esc back",
                "click a row to select and rebind · Cmd/Fn and Ctrl+C cannot bind",
            ]
        };
        for (i, line) in lines.iter().enumerate() {
            let y = legend_y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            f.render_widget(
                Line::styled(*line, ink(MUTED)).centered(),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
    }
    if !view.note.is_empty() && inner.height > lay.legend_rows {
        let y = inner.y + inner.height - 1;
        f.render_widget(
            Line::styled(view.note, heavy(EDGE)).centered(),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }

    // Focus edge, like the options plate.
    if lay.panel.width > 2 && lay.panel.height > 1 {
        f.buffer_mut().set_style(
            Rect::new(lay.panel.x, lay.panel.y, lay.panel.width, 1),
            ink(tone),
        );
    }
}

/// Accent colour for the panel edge: the same material tint the options plate
/// uses.
pub fn accent_from(rgb: crate::render::palette::Rgb) -> Color {
    to_color(mix(rgb, (235, 216, 214), 0.35))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn frame(w: u16, h: u16, row: usize, capturing: Option<Action>, scheme: Scheme) -> (String, HitMap) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = HitMap::default();
        let c = {
            let mut c = Controls::default();
            c.set_scheme(scheme);
            c
        };
        let view = ControlsView {
            scheme,
            controls: &c,
            row,
            scroll: 0,
            capturing,
            note: "a note",
            accent: Color::Rgb(144, 184, 173),
            lone_ctrl: true,
        };
        term.draw(|f| draw(f, f.area(), &view, &mut hits)).unwrap();
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

    #[test]
    fn rows_name_every_action_and_its_bindings() {
        let (text, hits) = frame(120, 40, SCHEME_ROW, None, Scheme::Classic);
        for needle in [
            "CLASSIC",
            "MOVE UP",
            "DASH",
            "FIRE ACTIVES",
            "RESET ZOOM",
            "BACK",
            "MMB",
        ] {
            assert!(text.contains(needle), "{needle:?} missing:\n{text}");
        }
        assert!(hits.rect_for(Target::ControlsBack).is_some());
    }

    #[test]
    fn a_capture_row_shows_press_and_the_capture_legend() {
        let (text, _) = frame(120, 40, action_row(Action::Dash), Some(Action::Dash), Scheme::Classic);
        assert!(text.contains("PRESS"), "{text}");
        assert!(text.contains("Esc cancels"), "{text}");
    }

    #[test]
    fn degenerate_terminals_do_not_panic() {
        for (w, h) in [(0u16, 0u16), (5, 3), (1, 1), (30, 12), (57, 20)] {
            let (text, _) = frame(w, h, 0, None, Scheme::Gamepad);
            assert_eq!(text.chars().filter(|c| *c == '\n').count(), h as usize);
        }
    }

    #[test]
    fn the_table_scrolls_to_the_selected_row() {
        let lay = layout(Rect::new(0, 0, 80, 24), 0);
        assert!(lay.visible >= 6, "a 24-row terminal fits a table");
        let far = layout(Rect::new(0, 0, 80, 24), ROW_COUNT);
        assert!(far.first > 0, "the table scrolls rather than clipping the tail");
    }
}
