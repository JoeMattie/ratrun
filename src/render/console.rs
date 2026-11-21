//! The Quake-style drop-down console's drawing.
//!
//! A single [`draw`] entry point paints the panel over whatever the game just
//! rendered: a title bar, a scrollback window, an optional Tab-completion list
//! and the one-line prompt with a blinking cursor. It reads only the
//! [`Console`]'s presentation state, so it never touches the simulation.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::console::{Console, LineKind, PROMPT};
use crate::render::palette::{COAT, EDGE};

/// Panel background: the game's dark plate, a touch warmer than the title.
const CONSOLE_BG: Color = Color::Rgb(16, 19, 22);
/// The title bar's stripe.
const TITLE_BG: Color = Color::Rgb(34, 42, 48);
/// Command output.
const OUT_FG: Color = Color::Rgb(205, 214, 216);
/// A successful command's confirmation.
const OK_FG: Color = Color::Rgb(140, 230, 150);
/// An error.
const ERR_FG: Color = Color::Rgb(255, 120, 120);
/// The echoed command line.
const ECHO_FG: Color = Color::Rgb(255, 210, 90);
/// Dim informational text.
const INFO_FG: Color = Color::Rgb(150, 180, 200);
/// The prompt caret.
const CARET_FG: Color = Color::Rgb(255, 210, 90);

/// Draw the console over `area`. A no-op when the slide animation is closed.
pub fn draw(frame: &mut Frame, area: Rect, console: &Console) {
    let anim = console.anim.clamp(0.0, 1.0);
    if anim <= 0.001 || area.width < 16 || area.height < 5 {
        return;
    }

    // The panel slides down from the top: up to ~45% of the screen, growing
    // with the animation. One row of title, one row of prompt, the rest body.
    let max_h = (area.height as f32 * 0.45).max(4.0);
    let panel_h = ((max_h * anim).round() as u16).clamp(1, area.height);
    if panel_h < 2 {
        return;
    }
    let panel = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: panel_h,
    };

    let buf = frame.buffer_mut();
    // Fill the panel background so the scene behind does not bleed through.
    for y in panel.y..panel.bottom() {
        for x in panel.x..panel.right() {
            buf[(x, y)]
                .set_char(' ')
                .set_style(Style::default().bg(CONSOLE_BG).fg(COAT));
        }
    }

    // Title bar.
    let title = " CONSOLE — ` to close · help for commands · Esc closes ";
    buf.set_stringn(
        panel.x,
        panel.y,
        title,
        panel.width as usize,
        Style::default()
            .bg(TITLE_BG)
            .fg(COAT)
            .add_modifier(Modifier::BOLD),
    );
    // A rule under the title.
    if panel.height >= 3 {
        let rule_y = panel.y + 1;
        for x in panel.x..panel.right() {
            buf[(x, rule_y)]
                .set_char('─')
                .set_style(Style::default().bg(CONSOLE_BG).fg(EDGE));
        }
    }

    // The prompt is the last row; the body sits between the rule and the prompt.
    let prompt_y = panel.bottom().saturating_sub(1);
    let body_top = panel.y.saturating_add(2);
    let body_rows = prompt_y.saturating_sub(body_top) as usize;

    // The completion list, when open, takes the rows just above the prompt.
    let comp_shown = console.comp_idx.is_some() && !console.completions.is_empty();
    let comp_rows = if comp_shown {
        console.completions.len().min(body_rows.saturating_sub(1)).min(6)
    } else {
        0
    };
    let text_rows = body_rows.saturating_sub(comp_rows);

    // Scrollback window.
    let (_start, lines) = console.visible(text_rows);
    let mut y = body_top;
    for line in lines {
        if y >= prompt_y {
            break;
        }
        let style = match line.kind {
            LineKind::Output => Style::default().bg(CONSOLE_BG).fg(OUT_FG),
            LineKind::Success => Style::default().bg(CONSOLE_BG).fg(OK_FG),
            LineKind::Error => Style::default().bg(CONSOLE_BG).fg(ERR_FG),
            LineKind::Echo => Style::default()
                .bg(CONSOLE_BG)
                .fg(ECHO_FG)
                .add_modifier(Modifier::BOLD),
            LineKind::Info => Style::default().bg(CONSOLE_BG).fg(INFO_FG),
        };
        buf.set_stringn(panel.x, y, &line.text, panel.width as usize, style);
        y += 1;
    }

    // Completion list, right above the prompt.
    if comp_shown {
        let mut cy = prompt_y.saturating_sub(comp_rows as u16);
        for (i, cand) in console.completions.iter().take(comp_rows).enumerate() {
            if cy >= prompt_y {
                break;
            }
            let selected = console.comp_idx == Some(i);
            let style = if selected {
                Style::default()
                    .bg(Color::Rgb(255, 210, 90))
                    .fg(Color::Rgb(20, 22, 26))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(CONSOLE_BG).fg(INFO_FG)
            };
            buf.set_stringn(panel.x, cy, cand, panel.width as usize, style);
            cy += 1;
        }
    }

    // Prompt line: "> " + the edited text, with a blinking caret.
    let prompt_style = Style::default().bg(CONSOLE_BG).fg(COAT);
    buf.set_stringn(panel.x, prompt_y, PROMPT, panel.width as usize, prompt_style);
    let text_x = panel.x + PROMPT.len() as u16;
    let avail = panel.width.saturating_sub(PROMPT.len() as u16);
    buf.set_stringn(text_x, prompt_y, &console.input, avail as usize, prompt_style);

    // Caret: a solid block when the blink phase is "on". Place it at the
    // cursor's display column (byte index for ASCII, which commands are).
    let blink_on = console.blink < 0.53;
    if blink_on {
        let caret_col = text_x.saturating_add(console.cursor as u16);
        if caret_col < panel.right() {
            let ch = console
                .input
                .get(console.cursor..)
                .and_then(|s| s.chars().next())
                .unwrap_or(' ');
            buf[(caret_col, prompt_y)]
                .set_char(ch)
                .set_style(Style::default().bg(CARET_FG).fg(Color::Rgb(20, 22, 26)));
        }
    }
}

/// The panel's outer rect for a given area, matching [`draw`]'s geometry. Used
/// by tests to assert the layout at different screen sizes.
pub fn panel_rect(area: Rect, anim: f32) -> Rect {
    let anim = anim.clamp(0.0, 1.0);
    let max_h = (area.height as f32 * 0.45).max(4.0);
    let panel_h = ((max_h * anim).round() as u16).clamp(1, area.height);
    Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: panel_h,
    }
}

// ---------------------------------------------------------------------------
// Tests: the panel geometry and the three supported layouts.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Line;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// The three layouts the console must stay readable at (the game's own
    /// minimum, the classic 80x24, and the wide 120x40).
    const SIZES: [(u16, u16); 3] = [(57, 20), (80, 24), (120, 40)];

    fn open_console() -> Console {
        let mut c = Console::new();
        c.open_panel();
        c.anim = 1.0;
        c.push(Line::output("help — 51 commands"));
        c.push(Line::success("god on"));
        c.input = "give wea".into();
        c.cursor = c.input.len();
        c
    }

    fn buffer_text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn panel_geometry_fits_every_layout() {
        for (w, h) in SIZES {
            let area = Rect::new(0, 0, w, h);
            let p = panel_rect(area, 1.0);
            assert_eq!(p.x, 0, "{w}x{h}: panel should start at the left edge");
            assert_eq!(p.y, 0, "{w}x{h}: panel should start at the top edge");
            assert_eq!(p.width, w, "{w}x{h}: panel should span the width");
            assert!(p.height >= 5, "{w}x{h}: only {} rows", p.height);
            assert!(p.height <= h, "{w}x{h}: panel taller than the screen");
            assert!(
                p.height <= (h as f32 * 0.45).ceil() as u16,
                "{w}x{h}: panel takes more than 45% of the screen"
            );
        }
    }

    #[test]
    fn closed_panel_draws_nothing() {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut c = Console::new();
        term.draw(|f| draw(f, f.area(), &c)).unwrap();
        assert!(
            !buffer_text(&term).contains("CONSOLE"),
            "a closed console painted its panel"
        );
        c.open_panel();
        c.anim = 1.0;
        term.draw(|f| draw(f, f.area(), &c)).unwrap();
        assert!(
            buffer_text(&term).contains("CONSOLE"),
            "an open console did not paint its title"
        );
    }

    #[test]
    fn draw_works_at_every_layout() {
        for (w, h) in SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let c = open_console();
            term.draw(|f| draw(f, f.area(), &c)).unwrap();
            let text = buffer_text(&term);
            assert!(text.contains("CONSOLE"), "{w}x{h}: no title bar");
            assert!(text.contains("god on"), "{w}x{h}: scrollback missing");
            assert!(text.contains("give wea"), "{w}x{h}: prompt missing");
        }
    }
}
