//! Facility signs, specimen files and run records.
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::game::{
    art_icons,
    campaign,
    level::Theme,
    loadout::{Loadout, Upgrade},
    weapon::WeaponKind,
    world::World,
};
use crate::lore;
use crate::render::campaign::{
    campaign_pips_width, draw_campaign_pips, CampaignLevel, CampaignProgressView,
};
use crate::render::hud::{weapon_glyph, HEAL_MINT, TRAIL_GOLD, WARN_AMBER};
use crate::render::palette::{self, mix, to_color, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};
use crate::scores::ScoreTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    Quit,
    /// Previous / next *campaign sector* (the title's sector row and the
    /// level-select screen; the free map selector is gone).
    PreviousSector,
    NextSector,
    /// The title's sector row: opens the sector-select screen.
    Sectors,
    /// A sector card on the sector-select screen (`0..3`).
    SelectSector(usize),
    /// Start the sector selected on the sector-select screen.
    StartSector,
    /// Proceed past a non-final level-complete card.
    Continue,
    /// Keep-three manifest: toggle the card at this index.
    KeepItem(usize),
    KeepScrollUp,
    KeepScrollDown,
    KeepConfirm,
    Story,
    Back,
    Begin,
    Dash,
    Pause,
    Upgrade(usize),
    Resume,
    Menu,
    Restart,
    /// The compact minimap in the play field: a left press toggles the
    /// expanded map (the run keeps going either way).
    Minimap,
    /// Open the options screen (the title's OPTIONS row and the pause legend).
    Options,
    /// A row on the options screen (`0..options::ROW_COUNT`): selects it, and
    /// on the value rows also steps it forward.
    OptionsRow(usize),
    /// A volume slider track on the options screen (`0` music, `1` effects):
    /// clicking or dragging inside it sets the level from the pointer column.
    OptionsBar(usize),
    /// The options screen's Back row: returns to wherever the player came from.
    OptionsBack,
}

/// How many interactive rectangles one screen can register. The campaign
/// screens are the widest: the keep-three manifest registers one row per owned
/// upgrade (the art's own `HitList` holds 32) plus its confirm button.
const HIT_CAPACITY: usize = 40;

/// Only the currently displayed layer is interactive. More specific targets
/// registered last (the map's back arrow) take precedence over their parent.
pub struct HitMap {
    targets: [Option<(Rect, Target)>; HIT_CAPACITY],
    len: usize,
    pub hovered: Option<Target>,
}

impl Default for HitMap {
    fn default() -> Self {
        HitMap {
            targets: [None; HIT_CAPACITY],
            len: 0,
            hovered: None,
        }
    }
}

impl HitMap {
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn add(&mut self, rect: Rect, target: Target) {
        if !rect.is_empty() && self.len < self.targets.len() {
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

    /// The rectangle a target was registered with on the last frame.
    ///
    /// The options sliders need it to turn a pointer column into a level
    /// without duplicating the layout maths the renderer used.
    pub fn rect_for(&self, target: Target) -> Option<Rect> {
        self.targets[..self.len]
            .iter()
            .rev()
            .flatten()
            .find(|(_, t)| *t == target)
            .map(|(r, _)| *r)
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

pub(crate) fn ink(c: Color) -> Style {
    Style::default().fg(c)
}
pub(crate) fn heavy(c: Color) -> Style {
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
pub(crate) fn panel(f: &mut Frame, r: Rect, caption: &str, edge: Color) -> Rect {
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
/// The title's sector-row text: the campaign level number, its short name and
/// a `×` when the level has not been unlocked yet.
fn sector_name(sector: usize, unlocked: bool) -> String {
    let level = crate::render::campaign::CampaignLevel::from_index(sector);
    if unlocked {
        format!("{:02} {}", level.number(), level.short_name())
    } else {
        format!("{:02} {} ×", level.number(), level.short_name())
    }
}

pub(crate) fn material(theme: Theme) -> palette::Rgb {
    match theme {
        Theme::Sewer => (144, 184, 173),
        Theme::Kitchen => (213, 173, 122),
        Theme::Lab => (151, 175, 214),
    }
}

pub fn title_art_width(area: Rect) -> Option<u32> {
    let rows = area.height.saturating_sub(23);
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

/// The title screen: wordmark, specimen art, then `START RUN / SECTOR / QUIT`.
///
/// The old free map selector is gone; row 1 is the campaign's sector row. It
/// shows the selected sector and whether it is open, `←`/`→` move the
/// selection, and Enter opens the art's sector-select screen
/// ([`crate::render::campaign::draw_level_select`]).
#[allow(clippy::too_many_arguments)]
pub fn draw_title(
    f: &mut Frame,
    area: Rect,
    theme: Theme,
    choice: usize,
    sector: usize,
    sector_unlocked: bool,
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
    let body_h = rat_h + art_gap + logo_h + 7 + gap * 2;
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
    let menu = centered_rect(30, body.height, body);
    let sector_label = "Sector   ";
    let sector_text = sector_name(sector, sector_unlocked);
    let sector_row = format!("{sector_label}‹ {sector_text} ›");
    for (i, text) in ["START RUN", sector_row.as_str(), "OPTIONS", "QUIT"]
        .into_iter()
        .enumerate()
    {
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
        hits.add(
            rr,
            [Target::Start, Target::Sectors, Target::Options, Target::Quit][i],
        );
        let prefix = if selected { " ▸  " } else { "    " };
        if i == 1 {
            // The arrows carry their own targets, so the pointer can step the
            // sector without opening the select screen.
            let arrow_x =
                rr.x.saturating_add((prefix.chars().count() + sector_label.len()) as u16);
            if arrow_x < rr.right() {
                hits.add(Rect::new(arrow_x, rr.y, 1, rr.height), Target::PreviousSector);
            }
            let right_arrow = arrow_x
                .saturating_add(sector_text.chars().count() as u16)
                .saturating_add(3);
            if right_arrow < rr.right() {
                hits.add(Rect::new(right_arrow, rr.y, 1, rr.height), Target::NextSector);
            }
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(prefix), Span::raw(text)])).style(style),
            rr,
        );
    }
    y += 4 + gap;
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
            // Volume quick keys get their own line when there is room between
            // the rail and the control legend.
            if r.width >= 54 {
                center(
                    f,
                    r,
                    r.height - 3,
                    Line::from(vec![
                        label("VOLUME   ", accent),
                        Span::styled("[ ] music  ·  - = effects  ·  O options", ink(COAT)),
                    ]),
                );
            }
        }
        center(
            f,
            r,
            r.height - 2,
            label(
                if wide {
                    "WASD/arrows/mouse move · Space/click dash+active · right click pause · M mute"
                } else {
                    "WASD/arrows · Space dash+active · RMB pause · M mute"
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
    let n = choices.len().max(1) as u16;
    let large_cards = area.height >= n * 6 + 3 && area.width >= 64;
    let target_stride: u16 = if large_cards { 6 } else { 4 };
    let panel_w: u16 = if large_cards { 72 } else { 66 };
    let panel_h: u16 = if large_cards {
        n * 6 + if area.height >= n * 6 + 5 { 5 } else { 3 }
    } else {
        n * 4 + 5
    };
    let r = panel(
        f,
        centered_rect(panel_w, panel_h, area),
        &format!(" LEVEL UP   /   LV {level:02} "),
        TAIL,
    );
    let content = r.inner(Margin::new(1, 0));
    let stride = (content.height.saturating_sub(1) / n).min(target_stride);
    let top_pad = u16::from(content.height >= n * stride + 3);
    for (i, (title, desc, flavor)) in choices.iter().enumerate() {
        let selected = i == idx;
        let card = band(content, i as u16 * stride + top_pad, stride);
        hits.add(card, Target::Upgrade(i));
        let border = if selected { TAIL } else { EDGE };
        let (class, icon) = match upgrades.get(i) {
            Some(Upgrade::NewWeapon(k)) => ("NEW WEAPON", art_icons::weapon(*k)),
            Some(Upgrade::LevelWeapon(_)) => ("WEAPON", art_icons::placeholder()),
            Some(Upgrade::Passive(p)) => ("PASSIVE", art_icons::passive(*p)),
            None => ("", art_icons::placeholder()),
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
        let text_rect = draw_card_icon(f.buffer_mut(), inner, icon);
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(desc.as_str(), ink(COAT)),
                Line::styled(flavor.as_str(), ink(MUTED).add_modifier(Modifier::ITALIC)),
            ]),
            text_rect,
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

/// Blit a large (`8×4` cells) or small (`4×2` cells) icon on the left side of a
/// level-up card's inner rectangle and return the remaining rectangle for text.
fn draw_card_icon(buf: &mut ratatui::buffer::Buffer, inner: Rect, icon: &art_icons::Icon) -> Rect {
    if inner.height >= art_icons::LARGE_CELL_H && inner.width >= art_icons::LARGE_CELL_W + 14 {
        let icon_rect = Rect::new(
            inner.x,
            inner.y,
            art_icons::LARGE_CELL_W,
            art_icons::LARGE_CELL_H,
        );
        art_icons::draw(buf, icon_rect, icon);
        let pad = art_icons::LARGE_CELL_W + 2;
        let text_y = inner.y + inner.height.saturating_sub(2) / 2;
        Rect::new(
            inner.x + pad,
            text_y,
            inner.width.saturating_sub(pad),
            inner.bottom().saturating_sub(text_y),
        )
    } else if inner.height >= art_icons::SMALL_CELL_H && inner.width >= art_icons::SMALL_CELL_W + 10
    {
        let icon_rect = Rect::new(
            inner.x,
            inner.y,
            art_icons::SMALL_CELL_W,
            art_icons::SMALL_CELL_H,
        );
        art_icons::draw(buf, icon_rect, icon);
        let pad = art_icons::SMALL_CELL_W + if inner.width < 52 { 1 } else { 2 };
        Rect::new(
            inner.x + pad,
            inner.y,
            inner.width.saturating_sub(pad),
            inner.height,
        )
    } else {
        inner
    }
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
    let r = panel(f, centered_rect(46, 8, area), " RUN CONTROL ", MUTED);
    center(f, r, 1, Span::styled("━━━  ▌▌  ━━━", heavy(TAIL)));
    center(f, r, 2, Span::styled("PAUSED", heavy(COAT)));
    controls(
        f,
        band(r, 4, 1),
        hits,
        &[
            (" Esc/P resume ", Some(Target::Resume)),
            ("  ", None),
            (" O options ", Some(Target::Options)),
            ("  ", None),
            (" Q to menu ", Some(Target::Menu)),
        ],
    );
    if r.height >= 7 {
        center(f, r, 6, label("[ ] music · - = effects · O options", MUTED));
    }
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
    sector: u8,
    practice: bool,
) {
    let (status, accent) = if won && practice {
        ("SECTOR CLEARED", Color::Rgb(169, 210, 186))
    } else if won {
        ("YOU SURVIVED!", Color::Rgb(169, 210, 186))
    } else {
        ("YOU DIED", TAIL)
    };
    let r = panel(
        f,
        centered_rect(61, 21, area),
        &format!(
            " INCIDENT REPORT   /   R-47   /   {} ",
            sector_tag(sector, practice)
        ),
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
                "{:02}:{:02} total   ·   Lv {level}   ·   {kills} kills",
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

/// Snapshot of a completed run's metrics for the expanded end-of-run Incident Report.
#[derive(Clone, Debug)]
pub struct RunSummary {
    pub won: bool,
    pub theme: Theme,
    pub combat_score: u32,
    pub bonus_time: u32,
    pub bonus_level: u32,
    pub bonus_win: u32,
    pub final_score: u32,
    pub time: f32,
    pub level: u32,
    pub kills: u32,
    pub damage_taken: f32,
    pub dashes_used: u32,
    pub heals_picked: u32,
    pub boss_slain: bool,
    pub weapons: Vec<(WeaponKind, u8)>,
    pub damage_mult: f32,
    pub fire_rate_mult: f32,
    pub armor: f32,
    /// The campaign level the run reached (`1..=3`).
    pub sector: u8,
    /// A single-sector practice run (labelled, and never a campaign win).
    pub practice: bool,
}

/// `SECTOR 2/3`, plus a `PRACTICE` marker for single-sector runs.
fn sector_tag(sector: u8, practice: bool) -> String {
    let sector = sector.clamp(1, campaign::LEVEL_COUNT as u8);
    if practice {
        format!("SECTOR {sector}/{} PRACTICE", campaign::LEVEL_COUNT)
    } else {
        format!("SECTOR {sector}/{}", campaign::LEVEL_COUNT)
    }
}

fn weapon_pips(level: u8, max: u8) -> String {
    let mut s = String::with_capacity(max as usize * 3);
    for i in 1..=max {
        s.push(if i <= level { '●' } else { '○' });
    }
    s
}

/// Level-up overlay enriched with per-weapon colour badges, level pip
/// previews, and a compact current-loadout footer when vertical room allows.
/// Preserves every card geometry, title string, and mouse hit target of [`draw_levelup`].
#[allow(clippy::too_many_arguments)]
pub fn draw_levelup_with_loadout(
    f: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    upgrades: &[Upgrade],
    loadout: &Loadout,
    idx: usize,
    hits: &mut HitMap,
) {
    let n = choices.len().max(1) as u16;
    let large_cards = area.height >= n * 6 + 3 && area.width >= 64;
    let target_stride: u16 = if large_cards { 6 } else { 4 };
    let show_loadout = if large_cards {
        area.height >= n * 6 + 5 && area.width >= 60
    } else {
        area.height >= n * 4 + 9 && area.width >= 60
    };
    let airy = large_cards && area.height >= n * 6 + 7;
    let panel_w: u16 = if large_cards { 72 } else { 66 };
    let panel_h: u16 = if large_cards {
        n * 6 + if show_loadout { if airy { 7 } else { 5 } } else { 3 }
    } else {
        n * 4 + if show_loadout { 7 } else { 5 }
    };
    let r = panel(
        f,
        centered_rect(panel_w, panel_h, area),
        &format!(" LEVEL UP   /   LV {level:02} "),
        TAIL,
    );
    let content = r.inner(Margin::new(1, 0));
    let footer_rows: u16 = if show_loadout {
        if large_cards && !airy { 2 } else { 3 }
    } else {
        1
    };
    let stride = (content.height.saturating_sub(footer_rows) / n).min(target_stride);
    let top_pad = if large_cards {
        u16::from(airy && content.height > n * stride + footer_rows)
    } else {
        u16::from(content.height >= 15)
    };

    for (i, (title, desc, flavor)) in choices.iter().enumerate() {
        let selected = i == idx;
        let card = band(content, i as u16 * stride + top_pad, stride);
        hits.add(card, Target::Upgrade(i));

        let (class, badge_color, pips_preview, icon) = match upgrades.get(i) {
            Some(Upgrade::NewWeapon(k)) => (
                "NEW WEAPON",
                to_color(k.color()),
                Some(format!("{} {}", weapon_glyph(*k), weapon_pips(1, k.max_level()))),
                art_icons::weapon(*k),
            ),
            Some(Upgrade::LevelWeapon(slot)) => {
                if let Some(w) = loadout.weapons.get(*slot) {
                    let next = (w.level + 1).min(w.kind.max_level());
                    (
                        "WEAPON",
                        to_color(w.kind.color()),
                        Some(format!(
                            "{} {}",
                            weapon_glyph(w.kind),
                            weapon_pips(next, w.kind.max_level())
                        )),
                        art_icons::weapon(w.kind),
                    )
                } else {
                    ("WEAPON", TAIL, None, art_icons::placeholder())
                }
            }
            Some(Upgrade::Passive(p)) => ("PASSIVE", HEAL_MINT, None, art_icons::passive(*p)),
            None => ("", MUTED, None, art_icons::placeholder()),
        };

        let border = if selected { TAIL } else { EDGE };
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

        // Top-right weapon glyph + next-level pips inside the card border.
        if let Some(pips) = pips_preview {
            let pw = pips.chars().count() as u16 + 2;
            if card.width >= 48 && card.height >= 3 && card.right() > card.x + pw + 28 {
                f.buffer_mut().set_string(
                    card.right() - pw - 2,
                    card.y,
                    format!(" {pips} "),
                    ink(if selected { badge_color } else { MUTED }),
                );
            }
        }

        if card.width >= 49 && card.height >= 4 && !class.is_empty() {
            f.buffer_mut().set_string(
                card.right() - class.len() as u16 - 4,
                card.bottom() - 1,
                format!(" {class} "),
                ink(if selected { badge_color } else { MUTED }),
            );
        }

        let text_rect = draw_card_icon(f.buffer_mut(), inner, icon);
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(desc.as_str(), ink(COAT)),
                Line::styled(flavor.as_str(), ink(MUTED).add_modifier(Modifier::ITALIC)),
            ]),
            text_rect,
        );
    }

    // Current loadout strip so build decisions are informed right on the card screen.
    if show_loadout && r.height >= 4 {
        let strip_y = r.height.saturating_sub(if large_cards && !airy { 2 } else { 3 });
        let mut spans: Vec<Span<'_>> = Vec::with_capacity(loadout.weapons.len() * 3 + 4);
        spans.push(label("ARSENAL ", MUTED));
        for (i, w) in loadout.weapons.iter().enumerate() {
            if i > 0 {
                spans.push(label("  ", MUTED));
            }
            let short: String = w.kind.name().chars().take(4).collect();
            let lvl = if w.is_max() {
                "★".to_string()
            } else {
                w.level.to_string()
            };
            spans.push(Span::styled(
                format!("{}{short}", weapon_glyph(w.kind)),
                ink(to_color(w.kind.color())),
            ));
            spans.push(Span::styled(
                lvl,
                if w.is_max() {
                    heavy(TRAIL_GOLD)
                } else {
                    ink(MUTED)
                },
            ));
        }
        center(f, r, strip_y, Line::from(spans));
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

/// Pause overlay with live Run Dossier (arsenal + passives + biometrics) on
/// medium/large terminals, falling back cleanly to [`draw_pause`] on compact screens.
pub fn draw_pause_with_run(
    f: &mut Frame,
    area: Rect,
    world: &World,
    muted: bool,
    hits: &mut HitMap,
) {
    if area.width < 58 || area.height < 21 {
        draw_pause(f, area, hits);
        return;
    }

    let accent = palette::marking(world.level.palette.wall_edge, world.level.palette.accent);
    let icon_dossier = area.width >= 76 && area.height >= 24;
    let card = if icon_dossier {
        centered_rect(76, 22, area)
    } else {
        centered_rect(58, 18, area)
    };
    let r = panel(
        f,
        card,
        &format!(" RUN CONTROL   /   {} ", world.level.theme.name().to_uppercase()),
        accent,
    );
    let inner = r.inner(Margin::new(2, 0));

    // Campaign progress pips on the panel's top-right border (the art file's
    // "HUD / Pause" placement). Skipped when the caption would collide.
    let pips_w = campaign_pips_width(true);
    if card.width > pips_w + 34 {
        let pips = CampaignProgressView {
            current: CampaignLevel::from_index(world.level_index),
            cleared_count: world.level_index.min(campaign::LEVEL_COUNT) as u8,
            bg: None,
        };
        draw_campaign_pips(
            f,
            Rect::new(card.right().saturating_sub(pips_w + 1), card.y, pips_w, 1),
            &pips,
        );
    }

    let banner_row: u16 = if icon_dossier { 0 } else { 1 };
    let clock_row: u16 = banner_row + 1;
    let rule_row: u16 = if icon_dossier { 2 } else { 4 };
    let hdr_row: u16 = rule_row + 1;
    let body_row: u16 = hdr_row + 1;

    center(f, r, banner_row, Span::styled("━━━  ▌▌  PAUSED  ▌▌  ━━━", heavy(TAIL)));

    // The clock counts down to the boss and stops there; once the boss fight is
    // on, show how long it has been running instead of a dead 00:00.
    let clock_text = if world.boss_time_reached() {
        let fought = (world.elapsed - crate::game::director::BOSS_TIME).max(0.0) as u32;
        format!("BOSS FIGHT +{:02}:{:02}", fought / 60, fought % 60)
    } else {
        let rem = world.time_left().max(0.0) as u32;
        format!("{:02}:{:02} to boss", rem / 60, rem % 60)
    };
    center(
        f,
        r,
        clock_row,
        Line::from(vec![
            Span::styled(clock_text, heavy(COAT)),
            label("   ·   Lv ", MUTED),
            Span::styled(world.player.level.to_string(), heavy(COAT)),
            label("   ·   ", MUTED),
            Span::styled(format!("{} kills", world.kills), ink(COAT)),
            label("   ·   Score ", MUTED),
            Span::styled(world.score.to_string(), heavy(accent)),
        ]),
    );

    // Divider.
    let rule = "─".repeat(usize::from(inner.width));
    f.render_widget(Line::styled(rule.as_str(), ink(TRACK)), band(inner, rule_row, 1));

    // Left: Arsenal (weapons); Right: Biometrics + Passives.
    let left_w = inner.width / 2;
    let right_x = inner.x + left_w + 1;
    let right_w = inner.width.saturating_sub(left_w + 1);

    let buf = f.buffer_mut();
    buf.set_stringn(inner.x, inner.y + hdr_row, "ARSENAL", left_w as usize, heavy(accent));
    buf.set_stringn(right_x, inner.y + hdr_row, "BIOMETRICS", right_w as usize, heavy(accent));

    let body_bottom = r.bottom().saturating_sub(3);
    if icon_dossier {
        let rows_avail = body_bottom.saturating_sub(inner.y + body_row);
        for dy in 0..rows_avail {
            buf[(inner.x + left_w.saturating_sub(1), inner.y + body_row + dy)]
                .set_char('│')
                .set_fg(TRACK)
                .set_bg(PLATE);
        }

        // Left: up to 5 weapons with 4×4 half-block icons (2 rows each).
        if world.player.loadout.weapons.is_empty() {
            buf.set_stringn(inner.x, inner.y + body_row, "none", left_w as usize, ink(MUTED));
        } else {
            let max_slots = (rows_avail / 2) as usize;
            for (idx, w) in world.player.loadout.weapons.iter().take(max_slots).enumerate() {
                let y = inner.y + body_row + (idx as u16) * 2;
                let icon_rect = Rect::new(
                    inner.x,
                    y,
                    art_icons::SMALL_CELL_W,
                    art_icons::SMALL_CELL_H,
                );
                art_icons::draw(buf, icon_rect, art_icons::weapon(w.kind));
                let tx = inner.x + art_icons::SMALL_CELL_W + 2;
                let max_tw = (inner.x + left_w.saturating_sub(2)).saturating_sub(tx) as usize;
                let wcol = to_color(w.kind.color());
                buf.set_stringn(tx, y, w.kind.name(), max_tw, heavy(COAT));
                let pips = weapon_pips(w.level, w.kind.max_level());
                let pip_col = if w.is_max() { TRAIL_GOLD } else { wcol };
                let (px_end, _) = buf.set_stringn(tx, y + 1, &pips, max_tw, ink(pip_col));
                if w.is_max() {
                    buf.set_stringn(
                        px_end + 1,
                        y + 1,
                        "MAX",
                        max_tw.saturating_sub(6),
                        heavy(TRAIL_GOLD),
                    );
                } else if w.kind.is_active() {
                    let tag = if w.timer <= 0.0 { "READY" } else { "ACTIVE" };
                    buf.set_stringn(
                        px_end + 1,
                        y + 1,
                        tag,
                        max_tw.saturating_sub(6),
                        ink(if w.timer <= 0.0 { wcol } else { MUTED }),
                    );
                }
            }
        }
    } else {
        for row in 0..6u16 {
            let y = inner.y + body_row + row;
            if y >= body_bottom {
                break;
            }
            buf[(inner.x + left_w.saturating_sub(1), y)]
                .set_char('│')
                .set_fg(TRACK)
                .set_bg(PLATE);

            if let Some(w) = world.player.loadout.weapons.get(row as usize) {
                let mut gx = inner.x;
                let mut gbuf = [0u8; 4];
                let g = weapon_glyph(w.kind).encode_utf8(&mut gbuf);
                let wcol = to_color(w.kind.color());
                (gx, _) = buf.set_stringn(gx, y, g, 2, heavy(wcol));
                (gx, _) = buf.set_stringn(gx + 1, y, w.kind.name(), 13, ink(COAT));
                let pips = weapon_pips(w.level, w.kind.max_level());
                let pip_col = if w.is_max() { TRAIL_GOLD } else { wcol };
                let px = (inner.x + left_w).saturating_sub(7);
                if px > gx {
                    buf.set_stringn(px, y, &pips, 5, ink(pip_col));
                }
            } else if row == 0 {
                buf.set_stringn(inner.x, y, "none", left_w as usize, ink(MUTED));
            }
        }
    }

    let st = &world.player.loadout.stats;
    let stats_rows: [(&str, String); 6] = [
        ("HP", format!("{:.0}/{:.0}", world.player.hp.max(0.0), world.player.max_hp())),
        ("Damage", format!("{}%", (st.damage_mult * 100.0).round() as u32)),
        ("Fire Rate", format!("{}%", (st.fire_rate_mult * 100.0).round() as u32)),
        ("Speed", format!("{}%", (st.move_speed / 58.0 * 100.0).round() as u32)),
        ("Armor/Reg", format!("{:.0} / +{:.1}/s", st.armor, st.regen)),
        ("Magnet", format!("{:.0}px", st.magnet)),
    ];
    for (i, (k, v)) in stats_rows.iter().enumerate() {
        let y = inner.y + body_row + i as u16;
        if y >= body_bottom {
            break;
        }
        buf.set_stringn(right_x, y, k, 11, ink(MUTED));
        let vw = v.chars().count() as u16;
        let vx = (right_x + right_w).saturating_sub(vw);
        if vx > right_x + 9 {
            buf.set_stringn(vx, y, v, vw as usize, ink(COAT));
        }
    }

    // Passive icon tray beneath Biometrics when the icon dossier has vertical room.
    if icon_dossier {
        let pass_hdr_y = inner.y + body_row + 7;
        let pass_icon_y = pass_hdr_y + 1;
        if pass_icon_y + art_icons::SMALL_CELL_H <= body_bottom {
            buf.set_stringn(right_x, pass_hdr_y, "PASSIVES", right_w as usize, heavy(accent));
            if world.player.loadout.passives.is_empty() {
                buf.set_stringn(right_x, pass_icon_y, "none", right_w as usize, ink(MUTED));
            } else {
                let mut px = right_x;
                let slot_w = art_icons::SMALL_CELL_W + 3; // 4-col icon + ×N (2) + 1 gap
                for &(pk, stacks) in world.player.loadout.passives.iter().take(5) {
                    if px + art_icons::SMALL_CELL_W + 2 > right_x + right_w {
                        break;
                    }
                    let ic = art_icons::passive(pk);
                    art_icons::draw(
                        buf,
                        Rect::new(px, pass_icon_y, art_icons::SMALL_CELL_W, art_icons::SMALL_CELL_H),
                        ic,
                    );
                    let capped = stacks >= pk.max_stacks();
                    let stk = format!("×{stacks}");
                    buf.set_stringn(
                        px + art_icons::SMALL_CELL_W,
                        pass_icon_y,
                        &stk,
                        3,
                        if capped { heavy(TRAIL_GOLD) } else { ink(to_color(ic.accent)) },
                    );
                    px = px.saturating_add(slot_w);
                }
            }
        }
    }

    let mute_label = if muted {
        "M unmute  ·  audio muted"
    } else {
        "M mute  ·  WASD/mouse move  ·  Space/click dash"
    };
    center(
        f,
        r,
        r.height.saturating_sub(3),
        label(mute_label, if muted { WARN_AMBER } else { MUTED }),
    );
    // Volume quick keys, on their own line above the controls when the panel
    // is tall enough to carry it.
    if r.height >= 16 {
        center(
            f,
            r,
            r.height.saturating_sub(2),
            label("[ ] music  ·  - = effects  ·  O options", MUTED),
        );
    }
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (" Esc/P resume ", Some(Target::Resume)),
            ("   ", None),
            (" O options ", Some(Target::Options)),
            ("   ", None),
            (" Q to menu ", Some(Target::Menu)),
        ],
    );
}

/// Expanded Incident Report end-of-run summary with score breakdown, combat
/// telemetry, final arsenal, boss commendation, and ranked records.
/// Falls back cleanly to [`draw_end`] on compact terminals.
pub fn draw_end_summary(
    f: &mut Frame,
    area: Rect,
    summary: &RunSummary,
    new_best: bool,
    lore_line: &str,
    scores: &ScoreTable,
    hits: &mut HitMap,
) {
    if area.width < 68 || area.height < 23 {
        draw_end(
            f,
            area,
            summary.won,
            summary.final_score,
            summary.time,
            summary.level,
            summary.kills,
            new_best,
            lore_line,
            scores,
            hits,
            summary.sector,
            summary.practice,
        );
        return;
    }

    let compact_v = area.height < 27;
    let panel_h: u16 = if compact_v { 22 } else { 27 };
    let (status, accent) = if summary.won && summary.practice {
        ("SECTOR CLEARED", Color::Rgb(169, 210, 186))
    } else if summary.won {
        ("YOU SURVIVED!", Color::Rgb(169, 210, 186))
    } else {
        ("YOU DIED", TAIL)
    };

    let r = panel(
        f,
        centered_rect(68, panel_h, area),
        &format!(
            " INCIDENT REPORT   /   R-47   /   {}   /   {} ",
            summary.theme.name().to_uppercase(),
            sector_tag(summary.sector, summary.practice)
        ),
        accent,
    );
    let inner = r.inner(Margin::new(2, 0));

    let y_stamp: u16 = if compact_v { 0 } else { 1 };
    let y_lore: u16 = if compact_v { 2 } else { 3 };
    let y_rule: u16 = if compact_v { 4 } else { 6 };
    let y_cols: u16 = y_rule + 1;
    let y_best: u16 = if compact_v { y_cols + 6 } else { y_cols + 7 };
    let y_arsenal: u16 = y_best + 1;
    let y_table: u16 = y_arsenal + 2;

    // Primary status stamp (+ boss commendation if defeated).
    let mut header_spans = vec![Span::styled(
        format!(" ▰▰  {status}  ▰▰ "),
        heavy(PLATE).bg(accent),
    )];
    if summary.boss_slain {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            " ◆ APEX NEUTRALIZED ◆ ",
            heavy(PLATE).bg(TRAIL_GOLD),
        ));
    }
    center(f, r, y_stamp, Line::from(header_spans));

    // Lore line.
    f.render_widget(
        Paragraph::new(lore_line)
            .centered()
            .style(ink(MUTED))
            .wrap(Wrap { trim: true }),
        band(inner, y_lore, 2),
    );

    // Horizontal divider.
    let rule = "─".repeat(usize::from(inner.width));
    f.render_widget(
        Line::styled(rule.as_str(), ink(TRACK)),
        band(inner, y_rule, 1),
    );

    // Two-column breakdown (Left: Score Breakdown, Right: Combat Metrics).
    let half = inner.width / 2;
    let lx = inner.x;
    let lw = half.saturating_sub(2);
    let rx = inner.x + half + 1;
    let rw = inner.width.saturating_sub(half + 1);

    let buf = f.buffer_mut();
    buf.set_stringn(lx, inner.y + y_cols, "SCORE BREAKDOWN", lw as usize, heavy(accent));
    buf.set_stringn(rx, inner.y + y_cols, "COMBAT METRICS", rw as usize, heavy(accent));

    for row in 0..6u16 {
        buf[(inner.x + half.saturating_sub(1), inner.y + y_cols + row)]
            .set_char('│')
            .set_fg(TRACK)
            .set_bg(PLATE);
    }

    let score_rows: [(&str, String, Style); 5] = [
        // `combat_score` is the cumulative campaign score.
        ("Combat", format!("{}", summary.combat_score), ink(COAT)),
        ("Time bonus", format!("+{}", summary.bonus_time), ink(COAT)),
        ("Level bonus", format!("+{}", summary.bonus_level), ink(COAT)),
        (
            "Extraction",
            if summary.bonus_win > 0 {
                format!("+{}", summary.bonus_win)
            } else {
                "—".to_string()
            },
            if summary.bonus_win > 0 {
                heavy(HEAL_MINT)
            } else {
                ink(MUTED)
            },
        ),
        (
            "FINAL SCORE",
            format!("{}", summary.final_score),
            heavy(accent),
        ),
    ];
    for (i, (k, v, vstyle)) in score_rows.iter().enumerate() {
        let y = inner.y + y_cols + 1 + i as u16;
        let kstyle = if i == 4 { heavy(COAT) } else { ink(MUTED) };
        buf.set_stringn(lx, y, k, lw as usize, kstyle);
        let vw = v.chars().count() as u16;
        let vx = (lx + lw).saturating_sub(vw);
        if vx > lx + 8 {
            buf.set_stringn(vx, y, v, vw as usize, *vstyle);
        }
    }

    let secs = summary.time as u32;
    let kpm = if summary.time >= 1.0 {
        (summary.kills as f32 / (summary.time / 60.0)).round() as u32
    } else {
        0
    };
    let metric_rows: [(&str, String); 6] = [
        // Cumulative across the campaign: the totals the run banked, not just
        // this sector's clock.
        ("Total time", format!("{:02}:{:02}", secs / 60, secs % 60)),
        ("Total kills", format!("{}  ({}/m)", summary.kills, kpm)),
        (
            "Sector",
            format!(
                "{} / {}{}",
                summary.sector.clamp(1, campaign::LEVEL_COUNT as u8),
                campaign::LEVEL_COUNT,
                if summary.practice { " practice" } else { "" }
            ),
        ),
        ("Level", format!("Lv {}", summary.level)),
        ("Dmg Taken", format!("{:.0}", summary.damage_taken)),
        (
            "Dashes / Dmg",
            format!(
                "{}  ·  {}%",
                summary.dashes_used,
                (summary.damage_mult * 100.0).round() as u32
            ),
        ),
    ];
    for (i, (k, v)) in metric_rows.iter().enumerate() {
        let y = inner.y + y_cols + 1 + i as u16;
        buf.set_stringn(rx, y, k, rw as usize, ink(MUTED));
        let vw = v.chars().count() as u16;
        let vx = (rx + rw).saturating_sub(vw);
        if vx > rx + 8 {
            buf.set_stringn(vx, y, v, vw as usize, ink(COAT));
        }
    }

    // New High Score banner or divider.
    if new_best {
        center(
            f,
            r,
            y_best,
            Span::styled(" ◆  NEW HIGH SCORE  ◆ ", heavy(TAIL)),
        );
    } else {
        f.render_widget(
            Line::styled(rule.as_str(), ink(TRACK)),
            band(inner, y_best, 1),
        );
    }

    // Final Arsenal strip.
    if !summary.weapons.is_empty() {
        let mut wspans: Vec<Span<'_>> = Vec::with_capacity(summary.weapons.len() * 3 + 2);
        wspans.push(label("ARSENAL  ", accent));
        for (i, &(kind, lvl)) in summary.weapons.iter().enumerate() {
            if i > 0 {
                wspans.push(label("  ", MUTED));
            }
            let short: String = kind.name().chars().take(4).collect();
            let is_max = lvl >= kind.max_level();
            wspans.push(Span::styled(
                format!("{}{short}", weapon_glyph(kind)),
                ink(to_color(kind.color())),
            ));
            wspans.push(Span::styled(
                if is_max {
                    "★".to_string()
                } else {
                    lvl.to_string()
                },
                if is_max { heavy(TRAIL_GOLD) } else { ink(MUTED) },
            ));
        }
        center(f, r, y_arsenal, Line::from(wspans));
    }

    // Ranked Records leaderboard (highlight current run's entry).
    let table = centered_rect(41, inner.height, inner);
    f.render_widget(
        Line::styled("   RK    SCORE   SECTOR         TIME", ink(accent)),
        band(table, y_table, 1),
    );
    let max_rows = usize::from(r.height.saturating_sub(y_table + 3)).min(5);
    let mut highlighted = false;
    for (i, e) in scores.entries.iter().take(max_rows).enumerate() {
        let s = e.time as u32;
        let is_this_run = !highlighted
            && e.score == summary.final_score
            && (e.time - summary.time).abs() < 0.5
            && e.map == summary.theme.name();
        if is_this_run {
            highlighted = true;
        }
        let marker = if is_this_run { "▸" } else { " " };
        let row_style = if is_this_run {
            heavy(TAIL)
        } else if i == 0 {
            ink(COAT)
        } else {
            ink(MUTED)
        };
        f.render_widget(
            Line::styled(
                format!(
                    " {marker} {:02}  {:>7}   {:<13.13} {:02}:{:02}",
                    i + 1,
                    e.score,
                    e.map,
                    s / 60,
                    s % 60
                ),
                row_style,
            ),
            band(table, y_table + 1 + i as u16, 1),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::loadout::PassiveKind;
    use crate::game::weapon::Weapon;
    use crate::scores::ScoreEntry;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn screen_text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn levelup_with_loadout_shows_pips_and_arsenal() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        lo.weapons.push(Weapon::new(WeaponKind::SqueakNova));
        let upgrades = vec![
            Upgrade::LevelWeapon(0),
            Upgrade::NewWeapon(WeaponKind::AcidSpit),
            Upgrade::Passive(PassiveKind::Armor),
        ];
        let desc: Vec<_> = upgrades.iter().map(|u| lo.describe(u)).collect();
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 26)).unwrap();
        terminal
            .draw(|f| {
                draw_levelup_with_loadout(f, f.area(), 4, &desc, &upgrades, &lo, 0, &mut hits);
            })
            .unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("ARSENAL"), "arsenal footer missing in {text}");
        assert!(text.contains("●●○○○"), "weapon level pips missing in {text}");
        assert!(text.contains("NEW WEAPON"), "class badge missing in {text}");
    }

    #[test]
    fn pause_with_run_shows_dossier_and_handles_compact_sizes() {
        let mut w = World::new(Theme::Kitchen, 7);
        w.kills = 88;
        w.score = 1500;
        let mut hits = HitMap::default();

        let mut wide = Terminal::new(TestBackend::new(80, 24)).unwrap();
        wide.draw(|f| draw_pause_with_run(f, f.area(), &w, true, &mut hits))
            .unwrap();
        let wt = screen_text(&wide);
        assert!(wt.contains("ARSENAL"), "dossier arsenal missing");
        assert!(wt.contains("BIOMETRICS"), "dossier biometrics missing");
        assert!(wt.contains("audio muted"), "mute indicator missing");
        assert!(wt.contains("Esc/P resume"));

        // Compact fallback must not panic and keeps controls.
        hits.clear();
        let mut compact = Terminal::new(TestBackend::new(57, 20)).unwrap();
        compact
            .draw(|f| draw_pause_with_run(f, f.area(), &w, false, &mut hits))
            .unwrap();
        let ct = screen_text(&compact);
        assert!(ct.contains("PAUSED"));
        assert!(ct.contains("Esc/P resume"));
    }

    /// The pause rail counts down to the boss and, once the boss fight is on,
    /// counts the fight up instead of sitting at `00:00 left`.
    #[test]
    fn pause_shows_a_boss_fight_clock_instead_of_a_dead_countdown() {
        let mut w = World::new(Theme::Kitchen, 7);
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        w.elapsed = 65.0; // 255 s boss time → 03:10 to go
        terminal
            .draw(|f| draw_pause_with_run(f, f.area(), &w, true, &mut hits))
            .unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("03:10 to boss"), "no build-up clock: {text}");

        w.elapsed = crate::game::director::BOSS_TIME + 42.0;
        terminal
            .draw(|f| draw_pause_with_run(f, f.area(), &w, true, &mut hits))
            .unwrap();
        let text = screen_text(&terminal);
        assert!(
            text.contains("BOSS FIGHT +00:42"),
            "no boss-fight clock: {text}"
        );
        assert!(!text.contains("00:00 left"), "a dead clock is still shown");
    }

    #[test]
    fn end_summary_renders_breakdown_boss_badge_and_compact_fallback() {
        let summary = RunSummary {
            won: true,
            theme: Theme::Lab,
            combat_score: 2400,
            bonus_time: 1500,
            bonus_level: 900,
            bonus_win: 5000,
            final_score: 9800,
            time: 300.0,
            level: 9,
            kills: 312,
            damage_taken: 64.0,
            dashes_used: 21,
            heals_picked: 3,
            boss_slain: true,
            weapons: vec![(WeaponKind::Gnaw, 5), (WeaponKind::AcidSpit, 3)],
            damage_mult: 1.45,
            fire_rate_mult: 1.24,
            armor: 2.0,
            sector: 3,
            practice: false,
        };
        let mut scores = ScoreTable::default();
        scores.insert(ScoreEntry {
            score: 9800,
            time: 300.0,
            level: 9,
            map: "Lab".into(),
            won: true,
            campaign_level: 3,
            campaign_won: true,
        });

        let mut hits = HitMap::default();
        let mut big = Terminal::new(TestBackend::new(90, 32)).unwrap();
        big.draw(|f| {
            draw_end_summary(f, f.area(), &summary, true, "Escaped.", &scores, &mut hits)
        })
        .unwrap();
        let bt = screen_text(&big);
        assert!(bt.contains("YOU SURVIVED!"));
        assert!(bt.contains("SECTOR 3/3"), "the header names the sector reached");
        assert!(bt.contains("APEX NEUTRALIZED"));
        assert!(bt.contains("SCORE BREAKDOWN"));
        assert!(bt.contains("COMBAT METRICS"));
        assert!(bt.contains("NEW HIGH SCORE"));
        assert!(bt.contains("▸Gnaw★"));

        // A practice clear is labelled, and its status line says so.
        let mut practice = summary.clone();
        practice.practice = true;
        practice.sector = 2;
        hits.clear();
        big.draw(|f| {
            draw_end_summary(f, f.area(), &practice, false, "Cleared.", &scores, &mut hits)
        })
        .unwrap();
        let pt = screen_text(&big);
        assert!(pt.contains("SECTOR CLEARED"), "practice clear status missing");
        assert!(pt.contains("SECTOR 2/3 PRACTICE"), "practice label missing");

        for (tw, th) in [(0, 0), (1, 1), (57, 20), (80, 16)] {
            hits.clear();
            let mut small = Terminal::new(TestBackend::new(tw, th)).unwrap();
            small
                .draw(|f| {
                    draw_end_summary(f, f.area(), &summary, true, "Escaped.", &scores, &mut hits)
                })
                .unwrap();
        }
    }

    #[test]
    fn levelup_draws_icons_and_preserves_hit_rects_at_57x20_80x24_120x40() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        lo.weapons.push(Weapon::new(WeaponKind::CheeseSpray));
        let upgrades = vec![
            Upgrade::LevelWeapon(0),
            Upgrade::NewWeapon(WeaponKind::AcidSpit),
            Upgrade::Passive(PassiveKind::Magnet),
        ];
        let desc: Vec<_> = upgrades.iter().map(|u| lo.describe(u)).collect();

        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hits = HitMap::default();
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| {
                    draw_levelup_with_loadout(f, f.area(), 5, &desc, &upgrades, &lo, 1, &mut hits);
                })
                .unwrap();

            let buf = terminal.backend().buffer();
            let half_blocks = buf
                .content
                .iter()
                .filter(|c| matches!(c.symbol(), "▀" | "▄"))
                .count();
            // 3 cards with at least 4×2 icons (57×20) or 8×4 icons (80×24, 120×40).
            let min_cells = if tw >= 80 && th >= 24 { 45 } else { 15 };
            assert!(
                half_blocks >= min_cells,
                "{tw}x{th}: expected >= {min_cells} icon cells on level-up cards, got {half_blocks}"
            );

            // Every card keeps a non-empty clickable hit rectangle.
            for i in 0..3 {
                let rect = hits
                    .rect_for(Target::Upgrade(i))
                    .unwrap_or_else(|| panic!("{tw}x{th}: missing hit rect for card {i}"));
                assert!(rect.width >= 20 && rect.height >= 4, "{tw}x{th}: bad card {i} rect {rect:?}");
                assert_eq!(
                    hits.at(rect.x + rect.width / 2, rect.y + rect.height / 2),
                    Some(Target::Upgrade(i))
                );
            }

            let text = screen_text(&terminal);
            assert!(text.contains("Gnaw"), "{tw}x{th}: missing Gnaw title");
            assert!(text.contains("Acid Spit"), "{tw}x{th}: missing Acid Spit title");
            assert!(text.contains("Whiskers"), "{tw}x{th}: missing Whiskers title");
        }
    }

    #[test]
    fn pause_dossier_draws_weapon_and_passive_icons_at_80x24_and_120x40() {
        let mut w = World::new(Theme::Sewer, 7);
        w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::CheeseSpray));
        w.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::TailThrash));
        w.player.loadout.apply(&Upgrade::Passive(PassiveKind::MaxHp));
        w.player.loadout.apply(&Upgrade::Passive(PassiveKind::Armor));

        for (tw, th) in [(80u16, 24u16), (120, 40)] {
            let mut hits = HitMap::default();
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| draw_pause_with_run(f, f.area(), &w, false, &mut hits))
                .unwrap();
            let buf = terminal.backend().buffer();
            let half_blocks = buf
                .content
                .iter()
                .filter(|c| matches!(c.symbol(), "▀" | "▄"))
                .count();
            assert!(
                half_blocks >= 25,
                "{tw}x{th}: expected weapon & passive icons on pause dossier, got {half_blocks}"
            );
            let text = screen_text(&terminal);
            assert!(text.contains("ARSENAL"));
            assert!(text.contains("PASSIVES"));
            assert!(text.contains("BIOMETRICS"));
        }
    }
}
