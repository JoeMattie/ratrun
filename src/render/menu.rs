//! Title, level-up, pause, and end screens.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::game::level::Theme;
use crate::lore;
use crate::scores::ScoreTable;

pub fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

const LOGO: &[&str] = &[
    "  ▄▄▄   ▄▄▄  ▄▄▄▄▄    ▄▄▄  ▄▄  ▄▄ ▄▄▄  ▄▄ ",
    "  █  █ █▀▀█   █      █▀▀█ █  █ █ █  █  █ █ ",
    "  █▀▀▄ █▄▄█   █      █▄▄█ █  █ █  █ █  █ █ ",
    "  ▀  ▀ ▀  ▀   ▀      ▀  ▀  ▀▀  ▀  ▀ ▀▀▀  ▀ ",
];

pub fn draw_title(
    frame: &mut Frame,
    area: Rect,
    theme: Theme,
    menu_idx: usize,
    scores: &ScoreTable,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(90, 220, 200)))
        .style(Style::default().bg(Color::Rgb(8, 12, 14)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw(""));
    for l in LOGO {
        lines.push(Line::from(Span::styled(
            *l,
            Style::default()
                .fg(Color::Rgb(255, 220, 120))
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "a terminal bullet-hell horde survivor",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        lore::TITLE_BLURB,
        Style::default().fg(Color::Rgb(180, 160, 120)),
    )));
    lines.push(Line::raw(""));

    let start_style = sel_style(menu_idx == 0);
    let map_style = sel_style(menu_idx == 1);
    let quit_style = sel_style(menu_idx == 2);
    lines.push(Line::from(Span::styled("▶ START RUN", start_style)));
    lines.push(Line::from(vec![
        Span::styled("  Map: ", map_style),
        Span::styled(
            format!("◄ {} ►", theme.name()),
            Style::default()
                .fg(Color::Rgb(120, 220, 255))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled("  QUIT", quit_style)));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!("Best: {}", scores.best()),
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "WASD/Arrows move · weapons auto-fire · Space dash · M mute",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "↑/↓ select · ←/→ change map · Enter confirm · L story · Q quit",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        inner,
    );
}

fn sel_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(255, 220, 120))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

pub fn draw_levelup(
    frame: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    idx: usize,
) {
    let h = 4 + choices.len() as u16 * 4;
    let r = centered_rect(58, h.max(12), area);
    frame.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" LEVEL UP!  →  Lv {} ", level))
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Rgb(16, 16, 24)));
    let inner = block.inner(r);
    frame.render_widget(block, r);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            choices
                .iter()
                .map(|_| Constraint::Length(4))
                .collect::<Vec<_>>(),
        )
        .split(inner);

    for (i, (title, desc, flavor)) in choices.iter().enumerate() {
        let selected = i == idx;
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(255, 220, 120))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let marker = if selected { "▶ " } else { "  " };
        let card = Paragraph::new(vec![
            Line::from(Span::styled(format!("{}{}. {}", marker, i + 1, title), style)),
            Line::from(Span::styled(
                format!("    {}", desc),
                Style::default().fg(if selected { Color::Black } else { Color::Gray }),
            )),
            Line::from(Span::styled(
                format!("    “{}”", flavor),
                Style::default()
                    .fg(if selected {
                        Color::Rgb(60, 40, 0)
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(Modifier::ITALIC),
            )),
        ])
        .style(if selected {
            Style::default().bg(Color::Rgb(255, 220, 120))
        } else {
            Style::default()
        });
        frame.render_widget(card, rows[i]);
    }
}

pub fn draw_story(frame: &mut Frame, area: Rect) {
    let r = centered_rect(64, 22, area);
    frame.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" DOSSIER ")
        .border_style(Style::default().fg(Color::Rgb(90, 220, 200)))
        .style(Style::default().bg(Color::Rgb(8, 12, 14)));
    let inner = block.inner(r);
    frame.render_widget(block, r);

    let mut lines: Vec<Line> = Vec::new();
    for (i, l) in lore::INTRO.iter().enumerate() {
        let style = if i < 2 {
            Style::default()
                .fg(Color::Rgb(255, 120, 120))
                .add_modifier(Modifier::BOLD)
        } else if l.starts_with("EXTERMINATION") {
            Style::default()
                .fg(Color::Rgb(255, 200, 90))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(*l, style)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Enter / Esc — back",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        inner,
    );
}

pub fn draw_map_intro(frame: &mut Frame, area: Rect, theme: Theme) {
    let (heading, flavor) = lore::map_intro(theme);
    let r = centered_rect(56, 11, area);
    frame.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 220, 255)))
        .style(Style::default().bg(Color::Rgb(10, 12, 18)));
    let inner = block.inner(r);
    frame.render_widget(block, r);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(""),
            Line::from(Span::styled(
                heading,
                Style::default()
                    .fg(Color::Rgb(255, 220, 120))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled(flavor, Style::default().fg(Color::Gray))),
            Line::raw(""),
            Line::from(Span::styled(
                "Enter — begin",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true }),
        inner,
    );
}

pub fn draw_pause(frame: &mut Frame, area: Rect) {
    let r = centered_rect(36, 7, area);
    frame.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(10, 14, 20)));
    let inner = block.inner(r);
    frame.render_widget(block, r);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "PAUSED",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled("Esc/P resume · Q quit to menu", Style::default().fg(Color::Gray))),
        ])
        .alignment(Alignment::Center),
        inner,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_end(
    frame: &mut Frame,
    area: Rect,
    won: bool,
    score: u32,
    time: f32,
    level: u32,
    kills: u32,
    new_best: bool,
    lore_line: &str,
    scores: &ScoreTable,
) {
    let r = centered_rect(56, 21, area);
    frame.render_widget(Clear, r);
    let (title, color) = if won {
        ("YOU SURVIVED!", Color::Green)
    } else {
        ("YOU DIED", Color::Red)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(Color::Rgb(10, 10, 14)));
    let inner = block.inner(r);
    frame.render_widget(block, r);

    let mut lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            lore_line,
            Style::default()
                .fg(Color::Rgb(180, 170, 140))
                .add_modifier(Modifier::ITALIC),
        )),
        Line::raw(""),
        Line::from(format!("Score: {}", score)),
        Line::from(format!(
            "Time: {:02}:{:02}   Level: {}   Kills: {}",
            (time as u32) / 60,
            (time as u32) % 60,
            level,
            kills
        )),
    ];
    if new_best {
        lines.push(Line::from(Span::styled(
            "★ NEW HIGH SCORE ★",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "— High Scores —",
        Style::default().fg(Color::DarkGray),
    )));
    for (i, e) in scores.entries.iter().take(5).enumerate() {
        lines.push(Line::from(format!(
            "{}. {:>6}  {} ({:02}:{:02})",
            i + 1,
            e.score,
            e.map,
            (e.time as u32) / 60,
            (e.time as u32) % 60
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Enter restart · Q to menu",
        Style::default().fg(Color::Gray),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        inner,
    );
}
