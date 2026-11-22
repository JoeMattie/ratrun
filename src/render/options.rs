//! Options screen: the two independent volume sliders plus the preferences
//! that used to live on bare keys (minimap size, the cheese cursor, mute).
//!
//! The screen is drawn over whatever opened it — the title plate or the frozen
//! play field behind the pause overlay — in the same stamped-metal framing as
//! the rest of the menus ([`crate::render::menu::panel`]). It is pure layout:
//! [`layout`] is the single source of truth for where every row and slider
//! sits, so the pointer hit rectangles the app registers can never disagree
//! with what the player sees. All state changes go through [`Action`].

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::Frame;

use crate::audio::{self, VOLUME_STEP};
use crate::render::cursor::CursorSetting;
use crate::render::menu::{heavy, ink, panel, HitMap, Target};
use crate::render::minimap::MinimapSize;
use crate::render::palette::{mix, to_color, COAT, EDGE, MUTED, PLATE, TRACK};

/// Row indices in screen order.
pub const MUSIC_ROW: usize = 0;
pub const EFFECTS_ROW: usize = 1;
pub const MINIMAP_ROW: usize = 2;
pub const CURSOR_ROW: usize = 3;
pub const MUTE_ROW: usize = 4;
pub const ZOOM_ROW: usize = 5;
/// Auto-pick level-ups (persisted; the level-up screen's toggle writes it too).
pub const AUTO_PICK_ROW: usize = 6;
// Wave 3 controls.
pub const SCHEME_ROW: usize = 7;
pub const CONTROLS_ROW: usize = 8;
pub const DEADZONE_ROW: usize = 9;
pub const RUMBLE_ROW: usize = 10;
/// Wipe the nest (`crate::meta`): the bank and every permanent upgrade. It
/// takes two presses (the row reads `WIPE`, then `CONFIRM?`).
pub const RESET_ROW: usize = 11;
pub const BACK_ROW: usize = 12;
/// How many rows the screen has.
pub const ROW_COUNT: usize = 13;

/// Slider identity used by [`Target::OptionsBar`]: the value the track edits.
pub const MUSIC_BAR: usize = 0;
pub const EFFECTS_BAR: usize = 1;

/// Preferred panel size; clamped to the terminal.
pub const PANEL_W: u16 = 58;
pub const PANEL_H: u16 = 20;
/// Width of the label column inside a row ("MUSIC", "EFFECTS", …).
const LABEL_W: u16 = 10;
/// Keyboard hint under the rows.
const LEGEND: &str = "↑↓ row · ←→ adjust · wheel · Enter open · Esc back";

/// What one row of the screen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Music,
    Effects,
    Minimap,
    Cursor,
    Mute,
    Zoom,
    Back,
    /// Wave 3 controls, appended at the end of the enum.
    Scheme,
    Controls,
    Deadzone,
    Rumble,
    AutoPick,
    /// The nest's reset (appended last: the legacy rows keep their order).
    Reset,
}

/// The screen's rows in order; index with the constants above.
pub const ROWS: [Row; ROW_COUNT] = [
    Row::Music,
    Row::Effects,
    Row::Minimap,
    Row::Cursor,
    Row::Mute,
    Row::Zoom,
    Row::AutoPick,
    Row::Scheme,
    Row::Controls,
    Row::Deadzone,
    Row::Rumble,
    Row::Reset,
    Row::Back,
];

impl Row {
    /// Label drawn in the row's label column.
    pub fn label(self) -> &'static str {
        match self {
            Row::Music => "MUSIC",
            Row::Effects => "EFFECTS",
            Row::Minimap => "MINIMAP",
            Row::Cursor => "CURSOR",
            Row::Mute => "MUTE",
            Row::Zoom => "ZOOM",
            Row::Back => "BACK",
            Row::Scheme => "SCHEME",
            Row::Controls => "CONTROLS",
            Row::Deadzone => "DEADZONE",
            Row::Rumble => "RUMBLE",
            Row::AutoPick => "AUTO-PICK",
            Row::Reset => "NEST",
        }
    }

    /// The volume channel this row edits, if any.
    pub fn bar(self) -> Option<usize> {
        match self {
            Row::Music => Some(MUSIC_BAR),
            Row::Effects => Some(EFFECTS_BAR),
            _ => None,
        }
    }
}

/// One frame of screen state, filled in by the app.
#[derive(Clone, Copy, Debug)]
pub struct OptionsView {
    pub music: u8,
    pub effects: u8,
    pub minimap: MinimapSize,
    pub cursor: CursorSetting,
    pub muted: bool,
    /// View zoom: how much world the playfield shows (`1.0` = unzoomed, `2.0`
    /// = twice the world). Stepped along [`crate::render::zoom::STEPS`].
    pub zoom: f32,
    /// Selected row (`0..ROW_COUNT`).
    pub row: usize,
    /// The RESET row is armed: the next activation wipes the nest.
    pub reset_armed: bool,
    /// Accent for the panel edge and the sliders (theme or level material).
    pub accent: Color,
    /// Selected control scheme (wave 3).
    pub scheme: crate::controls::Scheme,
    /// Gamepad stick dead zone, 0.05..=0.90 (wave 3).
    pub deadzone: f32,
    /// Rumble on hits, off by default (wave 3; needs a force-feedback pad).
    pub rumble: bool,
    /// Auto-pick level-ups: the same preference the level-up screen's toggle
    /// writes. Chests are **not** auto-opened (see the row's note).
    pub auto_pick: bool,
}

/// Rows and slider tracks for one terminal area.
#[derive(Clone, Copy, Debug)]
pub struct OptionsLayout {
    pub panel: Rect,
    /// One band per row, `MUSIC` first.
    pub rows: [Rect; ROW_COUNT],
    /// Slider tracks: `[music, effects]`.
    pub bars: [Rect; 2],
}

fn band(r: Rect, row: u16, height: u16) -> Rect {
    let row = row.min(r.height);
    Rect::new(r.x, r.y + row, r.width, height.min(r.height - row))
}

/// A row's label/value strip, one cell in from the selection marker column.
///
/// `inner` is the plate's **content** rect (below its border), so row 0 sits two
/// rows down: the header owns row 0 and its divider row 1. Rows used to start at
/// the plate's first row, which put the header's `─` divider *on* the MUSIC row
/// at 57x20 — the label was painted over the rule and the rest of the rule ran
/// on beside it.
fn row_rect(inner: Rect, index: usize) -> Rect {
    band(inner, 2 + index as u16, 1)
}

/// Where everything sits inside `area`.
pub fn layout(area: Rect) -> OptionsLayout {
    let card = crate::render::menu::centered_rect(
        PANEL_W.min(area.width),
        PANEL_H.min(area.height),
        area,
    );
    // `menu::panel` returns the plate's content rect: the border takes the
    // first and last row, so the content starts at `card.y + 1`. The rows hang
    // off that rect (see `row_rect`), which is what keeps the header's divider
    // above them.
    let inner = Rect::new(
        card.x.saturating_add(2),
        card.y.saturating_add(1),
        card.width.saturating_sub(4),
        card.height.saturating_sub(2),
    );
    let rows = std::array::from_fn(|i| row_rect(inner, i));
    let bars = std::array::from_fn(|i| {
        let r = rows[i];
        // `[` + track + `]` + ` 100%`, right-aligned inside the row.
        let bar_w = r.width.saturating_sub(LABEL_W + 2 + 5).min(24);
        let x = r.x.saturating_add(LABEL_W);
        Rect::new(x, r.y, bar_w.min(r.right().saturating_sub(x)), 1)
    });
    OptionsLayout {
        panel: card,
        rows,
        bars,
    }
}

/// Step a volume level by `dir` slider steps, clamped to `0..=100`.
pub fn step_volume(current: u8, dir: isize) -> u8 {
    let steps = (100 / VOLUME_STEP) as isize;
    let cur = (audio::snap_percent(current) / VOLUME_STEP) as isize;
    let next = (cur + dir).clamp(0, steps);
    (next * VOLUME_STEP as isize) as u8
}

/// The level a click at `column` on a slider track means (a click on the left
/// edge is 0, the right edge is 100).
pub fn percent_at(track: Rect, column: u16) -> u8 {
    if track.width <= 1 {
        return if column < track.x { 0 } else { 100 };
    }
    let span = track.width.saturating_sub(1);
    let rel = column.saturating_sub(track.x).min(span);
    audio::snap_percent(((u32::from(rel) * 100) / u32::from(span)) as u8)
}

/// Step the minimap preference (wrapping through `Normal → Small → Hidden`,
/// so Left and Right both reach every size).
pub fn step_minimap(current: MinimapSize, dir: i32) -> MinimapSize {
    if dir > 0 {
        current.cycle()
    } else if dir < 0 {
        current.cycle().cycle()
    } else {
        current
    }
}

/// Step the cursor preference (`Left`/`Right` both flip it: it is binary).
pub fn step_cursor(current: CursorSetting, _dir: i32) -> CursorSetting {
    current.toggle()
}

fn value_text(view: &OptionsView, row: Row) -> String {
    match row {
        Row::Minimap => view.minimap.as_str().to_uppercase(),
        Row::Cursor => {
            if view.cursor.is_on() {
                "ON".to_string()
            } else {
                "OFF".to_string()
            }
        }
        Row::Mute => {
            if view.muted {
                "ON".to_string()
            } else {
                "OFF".to_string()
            }
        }
        Row::Zoom => {
            let mut s = String::with_capacity(8);
            crate::render::zoom::format_into(&mut s, view.zoom);
            s
        }
        Row::Reset => {
            if view.reset_armed {
                "CONFIRM?".to_string()
            } else {
                "WIPE".to_string()
            }
        }
        Row::Music => String::new(),
        Row::Effects => String::new(),
        Row::Back => String::new(),
        Row::Scheme => view.scheme.name().to_string(),
        Row::Controls => "OPEN ›".to_string(),
        Row::Deadzone => format!("{:.2}", view.deadzone),
        Row::AutoPick => {
            if view.auto_pick {
                "ON".to_string()
            } else {
                "OFF".to_string()
            }
        }
        Row::Rumble => {
            if view.rumble {
                "ON".to_string()
            } else {
                "OFF".to_string()
            }
        }
    }
}

/// Draw the options panel and register its mouse targets.
pub fn draw(f: &mut Frame, area: Rect, view: &OptionsView, hits: &mut HitMap) {
    // Only this screen's controls are interactive while it is open.
    hits.clear();
    if area.is_empty() {
        return;
    }
    let lay = layout(area);
    let tone = view.accent;
    let inner = panel(f, lay.panel, " R-47   /   OPTIONS ", tone);
    if inner.is_empty() {
        // Degenerate terminal: the plate is all that fits.
        hits.add(lay.panel, Target::OptionsBack);
        return;
    }
    // The rows come from the layout, so the header and its divider are placed
    // *from the first row up*. Nothing here can land on a label.
    let header_y = lay.rows[0].y.saturating_sub(2);
    let rule_y = lay.rows[0].y.saturating_sub(1);

    // Header: what this screen is, and which channel is where.
    let wide = inner.width >= 34;
    if wide && !lay.rows[0].is_empty() {
        let header = Rect::new(inner.x, header_y, inner.width, 1);
        f.render_widget(
            Line::styled(
                if inner.width >= 48 {
                    "AUDIO   /   PREFERENCES"
                } else {
                    "AUDIO"
                },
                heavy(tone),
            )
            .centered(),
            header,
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
        let hovered = hits.hovered == Some(Target::OptionsRow(i));
        let value_col = if selected { tone } else { MUTED };
        let text_style = if selected {
            Style::default().fg(COAT).add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default().fg(PLATE).bg(TRACK)
        } else {
            ink(COAT)
        };

        // Selection marker column.
        if selected {
            // A soft band behind the whole row, so the choice reads at a glance.
            f.buffer_mut().set_style(
                Rect::new(r.x, r.y, lay.panel.width.saturating_sub(4), 1),
                if hovered {
                    Style::default().bg(TRACK)
                } else {
                    Style::default().bg(PLATE)
                },
            );
            // Marker last, so the band does not paint over its accent.
            f.render_widget(
                Line::styled("▸", heavy(PLATE).bg(tone)),
                Rect::new(r.x, r.y, 1, r.height),
            );
        }
        // Label, one cell in from the selection marker column.
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

        match row.bar() {
            Some(bar_idx) => {
                let track = lay.bars[bar_idx];
                let level = if bar_idx == MUSIC_BAR {
                    view.music
                } else {
                    view.effects
                };
                let pct = format!("{:>3}%", level);
                if !track.is_empty() {
                    // Brackets around the track, then the percentage at the edge.
                    if track.x > r.x && track.x - 1 < r.right() {
                        f.buffer_mut()[(track.x - 1, r.y)]
                            .set_char('[')
                            .set_fg(value_col);
                    }
                    if track.right() < r.right() {
                        f.buffer_mut()[(track.right(), r.y)]
                            .set_char(']')
                            .set_fg(value_col);
                    }
                    let filled = (u32::from(level) * u32::from(track.width) + 50) / 100;
                    for dx in 0..track.width {
                        let cell = &mut f.buffer_mut()[(track.x + dx, r.y)];
                        if u32::from(dx) < filled {
                            cell.set_char('█')
                                .set_fg(if level == 0 { TRACK } else { value_col });
                        } else {
                            cell.set_char('░').set_fg(TRACK);
                        }
                    }
                    let px = r.right().saturating_sub(pct.chars().count() as u16);
                    if px >= r.x {
                        f.buffer_mut().set_stringn(
                            px,
                            r.y,
                            &pct,
                            r.right().saturating_sub(px) as usize,
                            if level == 0 {
                                ink(MUTED)
                            } else {
                                heavy(COAT)
                            },
                        );
                    }
                }
                // The whole row selects; the track itself also sets the level.
                hits.add(r, Target::OptionsRow(i));
                if !track.is_empty() {
                    hits.add(track, Target::OptionsBar(bar_idx));
                }
            }
            None => {
                let value = value_text(view, *row);
                if index_is_back(*row) {
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
                    hits.add(r, Target::OptionsRow(i));
                    hits.add(rect, Target::OptionsBack);
                } else {
                    let strip = format!(" ‹ {value} › ");
                    let w = (strip.chars().count() as u16).min(r.width.saturating_sub(LABEL_W));
                    let rect = Rect::new(
                        r.x + LABEL_W,
                        r.y,
                        w.min(r.right().saturating_sub(r.x + LABEL_W)),
                        r.height,
                    );
                    let style = if selected {
                        heavy(PLATE).bg(tone)
                    } else if hovered {
                        heavy(PLATE).bg(TRACK)
                    } else {
                        ink(COAT).bg(TRACK)
                    };
                    f.render_widget(Line::styled(strip, style), rect);
                    hits.add(r, Target::OptionsRow(i));
                }
            }
        }
    }

    // Legend + a note about what the sliders actually do.
    // One row under the last row (BACK), and the note two under it.
    let legend_y = lay.rows[ROW_COUNT - 1].y.saturating_add(1);
    let legend_row = Rect::new(inner.x, legend_y, inner.width, 1);
    if !legend_row.is_empty() && inner.height > ROW_COUNT as u16 + 3 {
        f.render_widget(
            Line::styled(
                if inner.width >= LEGEND.chars().count() as u16 {
                    LEGEND
                } else {
                    "↑↓ · ←→ · Esc back"
                },
                ink(MUTED),
            )
            .centered(),
            legend_row,
        );
        // The auto-pick row's note says what the setting does *not* do: a chest
        // is a reward moment and its evolution deserves its card.
        let note = if view.muted {
            " master mute on — levels are kept"
        } else if view.row == AUTO_PICK_ROW {
            " cards skipped; chests still open their card"
        } else {
            " effects preview while you adjust"
        };
        f.render_widget(
            Line::styled(note, ink(if view.muted { EDGE } else { MUTED })).centered(),
            Rect::new(inner.x, legend_y.saturating_add(2), inner.width, 1),
        );
    }

    // Accent the panel's outer edge faintly so the plate reads as focused.
    if lay.panel.width > 2 && lay.panel.height > 1 {
        f.buffer_mut()
            .set_style(Rect::new(lay.panel.x, lay.panel.y, lay.panel.width, 1), ink(tone));
    }
}

fn index_is_back(row: Row) -> bool {
    row == Row::Back
}

/// Accent colour for the panel edge: the same material tint the title uses.
pub fn accent_from(rgb: crate::render::palette::Rgb) -> Color {
    to_color(mix(rgb, (235, 216, 214), 0.35))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn view(row: usize) -> OptionsView {
        OptionsView {
            music: 60,
            effects: 85,
            minimap: MinimapSize::Small,
            cursor: CursorSetting::On,
            muted: false,
            zoom: 1.0,
            row,
            reset_armed: false,
            accent: Color::Rgb(144, 184, 173),
            scheme: crate::controls::Scheme::Classic,
            deadzone: 0.2,
            rumble: false,
            auto_pick: false,
        }
    }

    fn frame(w: u16, h: u16, v: &OptionsView) -> (String, HitMap) {
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

    #[test]
    fn rows_bars_and_legend_are_laid_out_at_the_reference_sizes() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let v = view(MUSIC_ROW);
            let (text, hits) = frame(w, h, &v);
            for label in [
                "MUSIC", "EFFECTS", "MINIMAP", "CURSOR", "MUTE", "ZOOM", "AUTO-PICK", "BACK",
            ] {
                assert!(text.contains(label), "{w}x{h} is missing {label}:\n{text}");
            }
            assert!(text.contains("60%"), "{w}x{h} missing the music read-out");
            assert!(text.contains("85%"), "{w}x{h} missing the effects read-out");
            assert!(text.contains("SMALL"), "minimap value missing at {w}x{h}");
            assert!(text.contains("Esc back"), "legend missing at {w}x{h}");
            assert!(text.contains('█'), "a filled slider segment at {w}x{h}");
            assert!(text.contains('░'), "an empty slider segment at {w}x{h}");
            // The slider tracks are registered for the pointer, and the music
            // one sits on the MUSIC row.
            let music_bar = hits.rect_for(Target::OptionsBar(MUSIC_BAR)).unwrap();
            let effects_bar = hits.rect_for(Target::OptionsBar(EFFECTS_BAR)).unwrap();
            assert_eq!(music_bar.y, hits.rect_for(Target::OptionsRow(MUSIC_ROW)).unwrap().y);
            assert_eq!(
                effects_bar.y,
                hits.rect_for(Target::OptionsRow(EFFECTS_ROW)).unwrap().y
            );
            assert!(music_bar.x > 0 && music_bar.width >= 2);
            // Back is a registered control on every size.
            assert!(hits.rect_for(Target::OptionsBack).is_some());
            // Selecting a different row moves the marker.
            let (text, _) = frame(w, h, &view(BACK_ROW));
            assert!(text.contains("▸"));
        }
    }

    /// No plate rule is ever drawn through a row: the header divider sits
    /// above the first row, so the selected MUSIC row (the 57x20 review case)
    /// shows its label and slider with no `─` in them.
    #[test]
    fn the_header_rule_is_never_drawn_through_the_music_row() {
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let v = view(MUSIC_ROW);
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitMap::default();
            term.draw(|f| draw(f, f.area(), &v, &mut hits)).unwrap();
            let buf = term.backend().buffer();
            let row = hits
                .rect_for(Target::OptionsRow(MUSIC_ROW))
                .unwrap_or_else(|| panic!("{w}x{h}: no MUSIC row"));
            let text: String = (row.x..row.right())
                .map(|x| buf[(x, row.y)].symbol())
                .collect();
            assert!(text.contains("MUSIC"), "{w}x{h}: {text:?}");
            assert!(
                !text.contains('─'),
                "{w}x{h}: a rule runs through the MUSIC row: {text:?}"
            );
            assert!(
                row.y > 0,
                "{w}x{h}: the MUSIC row has no room above it for the header"
            );
            let above: String = (row.x..row.right())
                .map(|x| buf[(x, row.y - 1)].symbol())
                .collect();
            assert!(
                above.contains('─'),
                "{w}x{h}: the header divider is not above the first row: {above:?}"
            );
        }
    }

    #[test]
    fn degenerate_terminals_do_not_panic() {
        for (w, h) in [(0u16, 0u16), (5, 3), (1, 1), (3, 1), (1, 3), (2, 2)] {
            let (text, hits) = frame(w, h, &view(0));
            assert_eq!(text.chars().filter(|c| *c == '\n').count(), h as usize);
            // A click anywhere is harmless.
            assert!(hits.at(0, 0).is_none() || hits.at(0, 0).is_some());
        }
    }

    #[test]
    fn volume_stepping_clamps_and_clicks_map_to_levels() {
        assert_eq!(step_volume(0, -1), 0);
        assert_eq!(step_volume(0, 1), 5);
        assert_eq!(step_volume(95, 1), 100);
        assert_eq!(step_volume(100, 1), 100);
        assert_eq!(step_volume(50, -3), 35);
        assert_eq!(step_volume(17, 0), 15);

        let track = Rect::new(10, 4, 21, 1);
        assert_eq!(percent_at(track, 0), 0, "left of the track is 0");
        assert_eq!(percent_at(track, 10), 0);
        assert_eq!(percent_at(track, 20), 50);
        assert_eq!(percent_at(track, 30), 100);
        assert_eq!(percent_at(track, 99), 100, "clamped past the right edge");
        assert_eq!(percent_at(Rect::new(5, 0, 1, 1), 9), 100);
    }

    #[test]
    fn value_rows_step_through_their_settings() {
        assert_eq!(step_minimap(MinimapSize::Normal, 1), MinimapSize::Small);
        assert_eq!(step_minimap(MinimapSize::Small, 1), MinimapSize::Hidden);
        assert_eq!(step_minimap(MinimapSize::Hidden, 1), MinimapSize::Normal);
        assert_eq!(step_minimap(MinimapSize::Normal, -1), MinimapSize::Hidden);
        assert_eq!(step_cursor(CursorSetting::On, 1), CursorSetting::Off);
        assert_eq!(step_cursor(CursorSetting::Off, -1), CursorSetting::On);
        for (i, row) in ROWS.iter().enumerate() {
            assert_eq!(
                row.bar(),
                [0, 1, 2, 3, 4, 5, 6].get(i).and_then(|_| row.bar())
            );
        }
        assert_eq!(Row::Music.bar(), Some(MUSIC_BAR));
        assert_eq!(Row::Effects.bar(), Some(EFFECTS_BAR));
        assert_eq!(Row::Mute.bar(), None);
    }

    #[test]
    fn selection_styles_the_chosen_row_with_its_accent() {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut hits = HitMap::default();
        let v = view(EFFECTS_ROW);
        term.draw(|f| draw(f, f.area(), &v, &mut hits)).unwrap();
        let buf = term.backend().buffer();
        let accent = v.accent;
        // The marker cell is painted with the accent background.
        let row = hits.rect_for(Target::OptionsRow(EFFECTS_ROW)).unwrap();
        let cell = &buf[(row.x, row.y)];
        assert_eq!(cell.symbol(), "▸");
        assert_eq!(cell.bg, accent);
    }
}
