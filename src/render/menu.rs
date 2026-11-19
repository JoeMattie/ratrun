//! Facility signs, specimen files and run records.
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::game::{level::Theme, loadout::Upgrade};
use crate::lore;
use crate::render::palette::{self, mix, to_color, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};
use crate::scores::ScoreTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    Quit,
    PreviousMap,
    NextMap,
    Story,
    Back,
    Begin,
    Dash,
    Pause,
    Upgrade(usize),
    Resume,
    Menu,
    Restart,
}

/// Only the currently displayed layer is interactive. More specific targets
/// registered last (the map's back arrow) take precedence over their parent.
#[derive(Default)]
pub struct HitMap {
    targets: [Option<(Rect, Target)>; 8],
    len: usize,
    pub hovered: Option<Target>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn add(&mut self, rect: Rect, target: Target) {
        if !rect.is_empty() {
            self.targets[self.len] = Some((rect, target));
            self.len += 1;
        }
    }
    pub fn at(&self, x: u16, y: u16) -> Option<Target> {
        self.targets[..self.len]
            .iter()
            .rev()
            .flatten()
            .find(|(r, _)| r.contains((x, y).into()))
            .map(|(_, t)| *t)
    }
}

pub fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn ink(c: Color) -> Style {
    Style::default().fg(c)
}
fn heavy(c: Color) -> Style {
    ink(c).add_modifier(Modifier::BOLD)
}
fn band(r: Rect, row: u16, height: u16) -> Rect {
    let row = row.min(r.height);
    Rect::new(r.x, r.y + row, r.width, height.min(r.height - row))
}
fn center<'a>(f: &mut Frame, r: Rect, row: u16, line: impl Into<Line<'a>>) {
    f.render_widget(line.into().centered(), band(r, row, 1));
}
fn label(s: &str, c: Color) -> Span<'_> {
    Span::styled(s, ink(c))
}

/// Lay out a centered strip from the actual label widths. The same clipped
/// rectangle paints each segment and receives its pointer events.
fn controls(f: &mut Frame, row: Rect, hits: &mut HitMap, parts: &[(&str, Option<Target>)]) {
    let width = parts.iter().map(|(s, _)| s.chars().count() as u16).sum();
    let strip = centered_rect(width, 1, row);
    let mut x = strip.x;
    for &(s, target) in parts {
        let w = (s.chars().count() as u16).min(strip.right() - x);
        let rect = Rect::new(x, strip.y, w, strip.height);
        let mut style = ink(MUTED);
        if let Some(t) = target {
            hits.add(rect, t);
            style = if hits.hovered == Some(t) {
                heavy(PLATE).bg(TAIL)
            } else {
                ink(COAT).bg(TRACK)
            };
        }
        f.render_widget(Line::styled(s, style), rect);
        x += w;
    }
}

/// A stamped metal plate with the light catching its upper and left edges.
fn panel(f: &mut Frame, r: Rect, caption: &str, edge: Color) -> Rect {
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_style(ink(TRACK))
        .style(ink(COAT).bg(PLATE));
    let inner = block.inner(r);
    f.render_widget(block, r);
    if !r.is_empty() {
        let buf = f.buffer_mut();
        buf.set_style(band(r, 0, 1), ink(edge));
        buf.set_style(Rect::new(r.x, r.y, 1, r.height), ink(edge));
        if r.width > 5 {
            buf.set_stringn(r.x + 2, r.y, caption, (r.width - 4) as usize, heavy(edge));
        }
    }
    inner
}

// Title screens have no World. These desaturated inks match the three facility materials.
fn material(theme: Theme) -> palette::Rgb {
    match theme {
        Theme::Sewer => (144, 184, 173),
        Theme::Kitchen => (213, 173, 122),
        Theme::Lab => (151, 175, 214),
    }
}

pub fn title_art_width(area: Rect) -> Option<u32> {
    let rows = area.height.saturating_sub(22);
    (rows >= 6 && area.width >= 54)
        .then(|| (u32::from(rows) * 4).min(60).min(u32::from(area.width) - 8))
}

// Six-row condensed capitals. Pairs of scanlines pack into one cell at either scale.
const LETTERS: [[u8; 6]; 6] = [
    [30, 27, 27, 30, 27, 25],
    [14, 27, 27, 31, 27, 27],
    [31, 31, 4, 4, 4, 4],
    [30, 27, 27, 30, 27, 25],
    [27, 27, 27, 27, 27, 14],
    [25, 29, 29, 23, 23, 19],
];
fn masthead(f: &mut Frame, r: Rect, large: bool, clock: usize) {
    let scale = if large { 2 } else { 1 };
    let w = 39 * scale;
    if r.width < w {
        center(f, r, 0, Span::styled("R A T   R U N", heavy(COAT)));
        return;
    }
    let x0 = r.x + (r.width - w) / 2;
    let sweep = (clock / 3) % 185;
    for y in 0..(3 * scale).min(r.height) {
        for (letter, bits) in LETTERS.iter().enumerate() {
            for col in 0..5 {
                let bit = 16 >> col;
                let top = bits[usize::from(y * 2 / scale)] & bit != 0;
                let bot = bits[usize::from((y * 2 + 1) / scale)] & bit != 0;
                let ch = match (top, bot) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    _ => continue,
                };
                let lean = (3 * scale - 1 - y) / 2;
                let x = lean + (letter as u16 * 6 + u16::from(letter >= 3) * 2 + col) * scale;
                for dx in 0..scale {
                    let light = 1.0 - ((x + dx) as usize).abs_diff(sweep).min(5) as f32 / 5.0;
                    let shade = mix(
                        (243, 237, 235),
                        (156, 143, 162),
                        f32::from(y) / f32::from(3 * scale),
                    );
                    f.buffer_mut()[(x0 + x + dx, r.y + y)]
                        .set_char(ch)
                        .set_fg(to_color(mix(shade, (255, 228, 232), light)));
                }
            }
        }
    }
}

pub fn draw_title(
    f: &mut Frame,
    area: Rect,
    theme: Theme,
    choice: usize,
    scores: &ScoreTable,
    rat: &[String],
    hits: &mut HitMap,
) {
    let tone = material(theme);
    let accent = to_color(tone);
    let r = panel(f, area, " R-47   /   CONTAINMENT ", accent);
    if area.width >= 53 && !area.is_empty() {
        f.buffer_mut().set_string(
            area.right() - 23,
            area.y,
            " BULLET-HELL SURVIVOR ",
            ink(MUTED),
        );
    }
    let clock = f.count();
    let large = r.width >= 78 && r.height >= 21;
    let logo_h = if large { 6 } else { 3 };
    let rat_h = (rat.len() as u16).min(r.height.saturating_sub(logo_h + 11));
    let gap = u16::from(r.height >= 17);
    let art_gap = u16::from(rat_h > 0);
    let body_h = rat_h + art_gap + logo_h + 6 + gap * 2;
    let body = band(r, r.height.saturating_sub(body_h + 3) / 2, body_h);
    if rat_h > 0 {
        let width = title_art_width(area).unwrap_or(0) as u16;
        let x = r.x + r.width.saturating_sub(width) / 2;
        // Registration marks frame the specimen without enclosing the title.
        let left = x.saturating_sub(3);
        let right = (x + width + 2).min(r.right() - 1);
        for (yy, a, b) in [(body.y, '┌', '┐'), (body.y + rat_h - 1, '└', '┘')] {
            f.buffer_mut()[(left, yy)].set_char(a).set_fg(EDGE);
            f.buffer_mut()[(right, yy)].set_char(b).set_fg(EDGE);
        }
        for (i, row) in rat.iter().take(rat_h as usize).enumerate() {
            let c = mix((248, 238, 239), (169, 163, 175), i as f32 / rat_h as f32);
            let yy = body.y + i as u16;
            f.buffer_mut()
                .set_stringn(x, yy, row, width as usize, ink(to_color(c)));
            // Tint the ear and curling tail of the embedded side profile.
            for (col, ch) in row.chars().take(width as usize).enumerate() {
                let u = col as f32 / f32::from(width);
                let v = i as f32 / f32::from(rat_h);
                if ch != ' '
                    && (((u - 0.745) / 0.059).powi(2) + ((v - 0.12) / 0.18).powi(2) < 1.0
                        || (v > 0.61 && u < 0.145)
                        || (v > 0.90 && u < 0.48))
                {
                    f.buffer_mut()[(x + col as u16, yy)].set_fg(TAIL);
                }
            }
        }
    }
    masthead(f, band(body, rat_h + art_gap, logo_h), large, clock);
    let mut y = rat_h + art_gap + logo_h;
    center(f, body, y, label(lore::TITLE_BLURB, MUTED));
    y += 1 + gap;
    let menu = centered_rect(26, body.height, body);
    let map_label = "Map     ";
    let map = format!("{map_label}‹ {} ›", theme.name());
    for (i, text) in ["START RUN", map.as_str(), "QUIT"].into_iter().enumerate() {
        let selected = i == choice;
        let style = if selected {
            heavy(PLATE).bg(to_color(mix(
                tone,
                (235, 216, 214),
                0.22 + 0.12 * (clock as f32 * 0.04).sin(),
            )))
        } else {
            ink(COAT)
        };
        let rr = band(menu, y + i as u16, 1);
        hits.add(rr, [Target::Start, Target::NextMap, Target::Quit][i]);
        let prefix = if selected { " ▸  " } else { "    " };
        if i == 1 {
            let arrow_x =
                rr.x.saturating_add((prefix.chars().count() + map_label.len()) as u16);
            if arrow_x < rr.right() {
                hits.add(Rect::new(arrow_x, rr.y, 1, rr.height), Target::PreviousMap);
            }
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(prefix), Span::raw(text)])).style(style),
            rr,
        );
    }
    y += 3 + gap;
    center(
        f,
        body,
        y,
        Line::from(vec![
            label("BEST   ", accent),
            Span::styled(scores.best().to_string(), heavy(COAT)),
        ]),
    );
    if r.height >= 12 {
        let wide = r.width >= 72;
        if r.height >= 16 {
            let rail = band(r, r.height - 4, 1);
            f.render_widget(
                Block::new()
                    .borders(ratatui::widgets::Borders::TOP)
                    .border_style(ink(TRACK)),
                rail,
            );
            if wide {
                f.buffer_mut()
                    .set_string(r.x + 2, rail.y, " CONTROLS ", ink(MUTED));
            }
        }
        center(
            f,
            r,
            r.height - 2,
            label(
                if wide {
                    "WASD/arrows/mouse move · Space/click dash · right click pause · M mute"
                } else {
                    "WASD/↑↓←→/mouse · Space/click dash · RMB pause · M mute"
                },
                MUTED,
            ),
        );
        controls(
            f,
            band(r, r.height - 1, 1),
            hits,
            &[
                (
                    if wide {
                        "↑↓ select · ←→/wheel map · Enter confirm or click · "
                    } else {
                        "↑↓ select · ←→ map · Enter/click · "
                    },
                    None,
                ),
                (" L story ", Some(Target::Story)),
                (" · Q quit", None),
            ],
        );
    }
}

pub fn draw_levelup(
    f: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    upgrades: &[Upgrade],
    idx: usize,
    hits: &mut HitMap,
) {
    let r = panel(
        f,
        centered_rect(66, choices.len() as u16 * 4 + 5, area),
        &format!(" LEVEL UP   /   LV {level:02} "),
        TAIL,
    );
    let content = r.inner(Margin::new(1, 0));
    let stride = (content.height.saturating_sub(1) / choices.len().max(1) as u16).min(4);
    for (i, (title, desc, flavor)) in choices.iter().enumerate() {
        let selected = i == idx;
        let card = band(
            content,
            i as u16 * stride + u16::from(content.height >= 15),
            stride,
        );
        hits.add(card, Target::Upgrade(i));
        let border = if selected { TAIL } else { EDGE };
        let class = match upgrades.get(i) {
            Some(Upgrade::NewWeapon(_)) => "NEW WEAPON",
            Some(Upgrade::LevelWeapon(_)) => "WEAPON",
            Some(Upgrade::Passive(_)) => "PASSIVE",
            None => "",
        };
        let name = format!(
            " {}  {:02}   {} ",
            if selected { "◆" } else { "◇" },
            i + 1,
            title
        );
        let block = Block::bordered()
            .border_style(ink(border))
            .title(Span::styled(
                name,
                heavy(if selected { TAIL } else { COAT }),
            ));
        let inner = block.inner(card).inner(Margin::new(1, 0));
        f.render_widget(block, card);
        if card.width >= 49 && card.height >= 4 && !class.is_empty() {
            f.buffer_mut().set_string(
                card.right() - class.len() as u16 - 4,
                card.bottom() - 1,
                format!(" {class} "),
                ink(if selected { TAIL } else { MUTED }),
            );
        }
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(desc.as_str(), ink(COAT)),
                Line::styled(flavor.as_str(), ink(MUTED).add_modifier(Modifier::ITALIC)),
            ]),
            inner,
        );
    }
    center(
        f,
        r,
        r.height.saturating_sub(1),
        label(
            "↑↓ select   ·   Enter/click equip   ·   1–3 quick pick",
            MUTED,
        ),
    );
}

pub fn draw_story(f: &mut Frame, area: Rect, time: f32, hits: &mut HitMap) {
    hits.clear();
    hits.add(area, Target::Back);
    // Lower the exposure of the title behind the open file.
    f.buffer_mut()
        .set_style(area, ink(Color::Rgb(40, 48, 50)).bg(Color::Rgb(11, 15, 17)));
    let r = panel(
        f,
        centered_rect(69, 22, area),
        " DOSSIER   /   CELL BLOCK C ",
        TAIL,
    );
    let text = r.inner(Margin::new(3, 0));
    let body = band(text, 3, text.height.saturating_sub(5));
    center(
        f,
        text,
        0,
        Span::styled("SUBJECT R-47     /     STATUS: ESCAPED", heavy(TAIL)),
    );
    // Start with the first paragraph in view. Feed the rest into the file
    // window one line at a time and hold the final page for reading.
    let width = usize::from(body.width.max(1));
    let mut lines = Vec::new();
    for &source in &lore::INTRO[3..] {
        let style = if source.starts_with("EXTERMINATION") {
            heavy(TAIL)
        } else {
            ink(COAT)
        };
        let mut rest = source;
        while rest.chars().count() > width {
            let edge = rest
                .char_indices()
                .nth(width)
                .map_or(rest.len(), |(n, _)| n);
            let cut = rest[..edge].rfind(' ').filter(|&n| n > 0).unwrap_or(edge);
            lines.push(Line::styled(&rest[..cut], style));
            rest = rest[cut..].trim_start();
        }
        lines.push(Line::styled(rest, style));
    }
    let n = (4 + (time / 1.1) as usize).min(lines.len());
    lines.truncate(n);
    let offset = n.saturating_sub(body.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), body);
    center(
        f,
        r,
        r.height.saturating_sub(1),
        Span::styled(
            " Enter / Esc / click   back ",
            if hits.hovered == Some(Target::Back) {
                heavy(PLATE).bg(TAIL)
            } else {
                ink(COAT).bg(TRACK)
            },
        ),
    );
}

pub fn draw_map_intro(f: &mut Frame, area: Rect, theme: Theme, accent: Color, hits: &mut HitMap) {
    let (heading, flavor) = lore::map_intro(theme);
    let card = centered_rect(60, 13, area);
    hits.add(card, Target::Begin);
    let r = panel(f, card, " ACCESS ROUTE ", accent);
    let emblem = match theme {
        Theme::Sewer => [
            "      ╭──┬──╮      ",
            "      ╰──┼──╯      ",
            "═════╡═══╧═══╞═════",
        ],
        Theme::Kitchen => [
            " ▄▄▄   ▄▄▄   ▄▄▄ ",
            " ▀█▀   ▀█▀   ▀█▀ ",
            "══╧═════╧═════╧══",
        ],
        Theme::Lab => [
            "  ╭──╮      ╭──╮  ",
            "──┤  ├──»───┤  ├──",
            "  ╰──╯      ╰──╯  ",
        ],
    };
    for (i, s) in emblem.iter().enumerate() {
        center(f, r, i as u16, label(s, accent));
    }
    center(f, r, 4, Span::styled(heading, heavy(COAT)));
    let text = r.inner(Margin::new(3, 0));
    f.render_widget(
        Paragraph::new(flavor)
            .centered()
            .wrap(Wrap { trim: true })
            .style(ink(MUTED)),
        band(text, 6, 3),
    );
    center(
        f,
        r,
        r.height.saturating_sub(1),
        Span::styled(
            " ▸  Enter / click   begin ",
            heavy(PLATE).bg(if hits.hovered == Some(Target::Begin) {
                TAIL
            } else {
                accent
            }),
        ),
    );
}

pub fn draw_pause(f: &mut Frame, area: Rect, hits: &mut HitMap) {
    let r = panel(f, centered_rect(39, 8, area), " RUN CONTROL ", MUTED);
    center(f, r, 1, Span::styled("━━━  ▌▌  ━━━", heavy(TAIL)));
    center(f, r, 2, Span::styled("PAUSED", heavy(COAT)));
    controls(
        f,
        band(r, 4, 1),
        hits,
        &[
            (" Esc/P resume ", Some(Target::Resume)),
            ("   ", None),
            (" Q to menu ", Some(Target::Menu)),
        ],
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_end(
    f: &mut Frame,
    area: Rect,
    won: bool,
    score: u32,
    time: f32,
    level: u32,
    kills: u32,
    new_best: bool,
    lore_line: &str,
    scores: &ScoreTable,
    hits: &mut HitMap,
) {
    let (status, accent) = if won {
        ("YOU SURVIVED!", Color::Rgb(169, 210, 186))
    } else {
        ("YOU DIED", TAIL)
    };
    let r = panel(
        f,
        centered_rect(61, 21, area),
        " INCIDENT REPORT   /   R-47 ",
        accent,
    );
    let inner = r.inner(Margin::new(2, 0));
    center(
        f,
        r,
        1,
        Span::styled(format!(" ▰▰  {status}  ▰▰ "), heavy(PLATE).bg(accent)),
    );
    f.render_widget(
        Paragraph::new(lore_line)
            .centered()
            .style(ink(MUTED))
            .wrap(Wrap { trim: true }),
        band(inner, 3, 3),
    );
    center(
        f,
        r,
        6,
        Line::from(vec![
            label("SCORE   ", accent),
            Span::styled(score.to_string(), heavy(COAT)),
        ]),
    );
    let seconds = time as u32;
    center(
        f,
        r,
        7,
        Line::styled(
            format!(
                "{:02}:{:02} elapsed   ·   Lv {level}   ·   {kills} kills",
                seconds / 60,
                seconds % 60
            ),
            ink(COAT),
        ),
    );
    if new_best {
        center(f, r, 8, Span::styled(" ◆  NEW HIGH SCORE  ◆ ", heavy(TAIL)));
    }
    let table = centered_rect(35, inner.height, inner);
    f.render_widget(
        Line::styled("RK    SCORE   SECTOR         TIME", ink(accent)),
        band(table, 10, 1),
    );
    for (i, e) in scores
        .entries
        .iter()
        .take(usize::from(r.height.saturating_sub(12)).min(5))
        .enumerate()
    {
        let s = e.time as u32;
        f.render_widget(
            Line::styled(
                format!(
                    "{:02}  {:>7}   {:<13.13} {:02}:{:02}",
                    i + 1,
                    e.score,
                    e.map,
                    s / 60,
                    s % 60
                ),
                ink(if i == 0 { COAT } else { MUTED }),
            ),
            band(table, 11 + i as u16, 1),
        );
    }
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (" Enter restart ", Some(Target::Restart)),
            ("   ", None),
            (" Q to menu ", Some(Target::Menu)),
        ],
    );
}
