//! Heads-up display: top status line + bottom HP / XP bars.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};
use ratatui::Frame;

use crate::game::world::World;
use crate::render::palette;

fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn draw_top(frame: &mut Frame, area: Rect, world: &World) {
    let accent = palette::to_color(world.level.palette.accent);
    let weapons: Vec<Span> = world
        .player
        .loadout
        .weapons
        .iter()
        .flat_map(|w| {
            vec![
                Span::styled(
                    format!(" {}", &w.kind.name()[..w.kind.name().len().min(4)]),
                    Style::default().fg(palette::to_color(w.kind.color())),
                ),
                Span::styled(
                    format!("{}", w.level),
                    Style::default().fg(Color::DarkGray),
                ),
            ]
        })
        .collect();

    let mut spans = vec![
        Span::styled("⏱ ", Style::default().fg(accent)),
        Span::styled(
            fmt_time(world.time_left()),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("Lv", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", world.player.level),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("☠ ", Style::default().fg(Color::Red)),
        Span::styled(format!("{}", world.kills), Style::default().fg(Color::White)),
        Span::raw("   "),
        Span::styled("Score ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", world.score),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  │"),
    ];
    spans.extend(weapons);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn draw_bottom(frame: &mut Frame, area: Rect, world: &World) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let hp = (world.player.hp.max(0.0) / world.player.max_hp()).clamp(0.0, 1.0);
    let hp_color = if hp > 0.5 {
        Color::Green
    } else if hp > 0.25 {
        Color::Yellow
    } else {
        Color::Red
    };
    let hp_gauge = Gauge::default()
        .ratio(hp as f64)
        .label(format!(
            "♥ {:.0}/{:.0}",
            world.player.hp.max(0.0),
            world.player.max_hp()
        ))
        .gauge_style(Style::default().fg(hp_color).bg(Color::Rgb(40, 20, 20)));
    frame.render_widget(hp_gauge, cols[0]);

    let xp = (world.player.xp as f32 / world.player.xp_to_next.max(1) as f32).clamp(0.0, 1.0);
    let dash = if world.player.dash_cd <= 0.0 {
        "DASH READY".to_string()
    } else {
        format!("dash {:.1}s", world.player.dash_cd)
    };
    let xp_gauge = Gauge::default()
        .ratio(xp as f64)
        .label(format!("XP   {}", dash))
        .gauge_style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::Rgb(20, 30, 45)),
        );
    frame.render_widget(xp_gauge, cols[1]);
}
