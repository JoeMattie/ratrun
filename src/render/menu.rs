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
    loadout::{ChestReward, Loadout, Upgrade},
    weapon::WeaponKind,
    world::World,
};
use crate::controls::{Action, Controls};
use crate::lore;
use crate::render::campaign::{
    campaign_pips_width, draw_campaign_pips, CampaignLevel, CampaignProgressView,
};
use crate::render::hud::{weapon_glyph, weapon_tag, HEAL_MINT, TRAIL_GOLD, WARN_AMBER};
use crate::render::palette::{self, mix, to_color, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};
use crate::scores::ScoreTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    Quit,
    /// Previous / next *floor* (the title's sector row and the level-select
    /// screen; the free map selector is gone).
    PreviousSector,
    NextSector,
    /// The title's sector row: opens the sector-select screen.
    Sectors,
    /// A floor's card on the sector-select screen: an index into
    /// `Theme::playable()` (the same order the title row lists), never a
    /// campaign stage.
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
    /// The gene-splice shop: the shelf row at this index (`0..shop::ROW_COUNT`).
    ShopRow(usize),
    /// The shop's Molt two-step list row at this index.
    ShopMoltRow(usize),
    /// The shop's exit row (registered separately for a whole-row click).
    ShopLeave,
    /// The shop's free Molt cancellation.
    ShopCancel,
    /// Keep screen: pick route floor `i` (`0..2`) for the next campaign level.
    RouteOption(usize),
    /// The title's LIBRARY row (and the pause screen's library chip): opens the
    /// specimen index (`Screen::Library`).
    Library,
    /// A tab on the Library screen (`0..4`, the [`crate::game::library::Tab`]
    /// order).
    LibraryTab(usize),
    /// An entry row in the Library list: the index **within the current tab**
    /// (group headers are not targets).
    LibraryRow(usize),
    /// The narrow Library layout's "back to the list" chip (the card replaces
    /// the list below 70 columns).
    LibraryCard,
    /// Controls screen: a table row (scheme, an action, reset).
    ControlsRow(usize),
    /// Controls screen: the reset-to-defaults row.
    ControlsReset,
    /// Controls screen: the back row.
    ControlsBack,
    /// The level-up screen's AUTO-PICK pill: flips the preference (the AUTO-PICK action, `U` by default).
    AutoPickToggle,
    /// **The nest** (`crate::render::nest`): the permanent upgrade screen. The
    /// title's NEST row, the pause screen's read-only chip and the incident
    /// report's "spend your haul" all open it.
    Nest,
    /// A track row on the nest screen (`0..meta::TRACK_COUNT`): selects it, and
    /// on a click also buys it.
    NestRow(usize),
    /// The nest's free refund-all row.
    NestRefund,
}

/// How many interactive rectangles one screen can register. The campaign
/// screens are the widest: the keep-three manifest registers one row per owned
/// upgrade (the art's own `HitList` holds 32) plus its confirm button.
const HIT_CAPACITY: usize = 48;

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
/// The title's sector-row text for the floor at `floor` (an index into
/// [`Theme::playable`]): the campaign **stage** number, the floor's short name
/// and a `×` when its stage has not been unlocked yet.
///
/// The side-route floors carry their stage's number (`02 Laundry`,
/// `03 Printworks`). The row's last entry is the getaway ride, the campaign's
/// fourth level (`04 Flatbed`); as a practice floor it is always open
/// (`ScoreTable::floor_unlocked`), so it never shows the lock mark.
fn sector_name(floor: usize, unlocked: bool) -> String {
    let floors = Theme::playable();
    let theme = floors[floor.min(floors.len() - 1)];
    let level = crate::render::campaign::CampaignLevel::from_theme(theme);
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
        Theme::Laundry => (159, 200, 200),
        Theme::Printworks => (182, 180, 165),
        Theme::Trailer => (206, 178, 132),
    }
}

pub fn title_art_width(area: Rect) -> Option<u32> {
    let rows = area.height.saturating_sub(24);
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
    controls_src: &Controls,
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
    // Six menu rows (LIBRARY and THE NEST joined the list): the specimen art
    // gives up the row, never the menu or the legend.
    let rat_h = (rat.len() as u16).min(r.height.saturating_sub(logo_h + 12));
    // The 57x20 plate (18 inner rows) closes the two blank rows around the
    // menu so all six rows and the whole legend fit.
    let gap = u16::from(r.height >= 19);
    let art_gap = u16::from(rat_h > 0);
    let body_h = rat_h + art_gap + logo_h + 9 + gap * 2;
    let mut body_top = r.height.saturating_sub(body_h + 3) / 2;
    // The controls legend is a block of lines, most important first, laid
    // out from the bottom up: the plate takes as many as it has rows for and
    // never drops the first one. The lines carry the **current** bindings
    // (`controls::Controls::title_legend_lines`), so a remapped key shows here
    // at once.
    let legend = controls_src.title_legend_lines(r.width >= 96);
    // One row less than the old ladder: the title carries six menu rows now
    // (LIBRARY and THE NEST), so the legend keeps clear of the BEST line.
    let legend_room = if r.height >= 19 {
        4
    } else if r.height >= 18 {
        3
    } else if r.height >= 16 {
        2
    } else {
        1
    };
    let mut legend_n = legend.len().min(legend_room);
    if legend_n < legend.len() && r.height >= 16 {
        // One more line fits when the body's last painted row (BEST) still
        // clears the rail, if need be by lifting the body into the spare rows
        // above it. That is the 57x20 plate: all four lines, nothing dropped.
        let rail = r.height.saturating_sub(3 + legend_n as u16);
        let end = body_top + body_h - 1;
        if end <= rail {
            legend_n += 1;
        } else if end - rail <= body_top {
            body_top -= end - rail;
            legend_n += 1;
        }
    }
    while legend_n > 1 && r.height.saturating_sub(2 + legend_n as u16) < 8 {
        legend_n -= 1;
    }
    let body = band(r, body_top, body_h);
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
    // 32 columns: the sector row lists every playable floor now, and
    // `Sector ‹ 03 Printworks × ›` plus the ` ▸  ` pointer is 30 cells — it did
    // not fit the old 30-column band at 57x20.
    let menu = centered_rect(32, body.height, body);
    let sector_label = "Sector ";
    let sector_text = sector_name(sector, sector_unlocked);
    let sector_row = format!("{sector_label}‹ {sector_text} ›");
    // The nest row carries the bank, so the title always answers "what have I
    // got to spend" without opening the screen. Empty and unspent reads plain.
    let nest_row = if scores.meta.bank > 0 {
        format!("THE NEST  {}¢", scores.meta.bank)
    } else {
        "THE NEST".to_string()
    };
    let rows: [(Target, String); 6] = [
        (Target::Start, "START RUN".to_string()),
        (Target::Sectors, sector_row),
        (Target::Options, "OPTIONS".to_string()),
        (Target::Library, "LIBRARY".to_string()),
        (Target::Nest, nest_row),
        (Target::Quit, "QUIT".to_string()),
    ];
    for (i, (target, text)) in rows.iter().enumerate() {
        let text = text.as_str();
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
        hits.add(rr, *target);
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
    y += 6 + gap;
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
        let lines = legend;
        let n = legend_n;
        if r.height >= 16 {
            let rail_row = r.height.saturating_sub(2 + n as u16);
            f.render_widget(
                Block::new()
                    .borders(ratatui::widgets::Borders::TOP)
                    .border_style(ink(TRACK)),
                band(r, rail_row, 1),
            );
            if wide {
                f.buffer_mut().set_string(
                    r.x + 2,
                    r.y + rail_row,
                    " CONTROLS ",
                    ink(MUTED),
                );
            }
        }
        // The dash key is named from the live input mode: Ctrl where the
        // terminal reports a lone modifier press, Space where it cannot (see
        // `input::DashKey`).
        for (i, line) in lines.iter().take(n).enumerate() {
            center(f, r, r.height - 2 - i as u16, label(line, MUTED));
        }
        // The story key follows its binding (the STORY / LIBRARY action).
        // Each action is named **once**: the wide plate's own legend carries
        // `L story` and `Q quit` on its last line, so the rail below repeats a
        // chip only when that line is not on screen (the title printed `Q quit`
        // twice at 80 × 24 and 120 × 40).
        let named = |needle: &str| lines.iter().take(n).any(|line| line.contains(needle));
        let story_chip = format!(" {} story ", controls_src.action_label(Action::Library));
        let quit_key = controls_src.action_label(Action::Quit);
        let quit_chip = if wide {
            format!(" · {quit_key} quit")
        } else {
            format!("· {quit_key} quit")
        };
        // Confirm is an action like the rest (`Input::confirm_pressed`), so
        // the rail names its live key rather than a literal `Enter`.
        let confirm_key = controls_src.action_label(Action::Confirm);
        let lead = if wide {
            format!("↑↓ select · ←→/wheel sector · {confirm_key} confirm or click · ")
        } else {
            // With the default keys the narrow row is 54 cells: it has to
            // fit the 55-cell plate of a 57-column terminal with `Q quit`
            // intact.
            format!("↑↓ select · ←→ sector · {confirm_key}/click ·")
        };
        let mut parts: Vec<(&str, Option<Target>)> = vec![(lead.as_str(), None)];
        let story_needle = format!("{} story", controls_src.action_label(Action::Library));
        if !named(&story_needle) {
            parts.push((story_chip.as_str(), Some(Target::Story)));
        } else if let Some(i) = lines
            .iter()
            .take(n)
            .position(|line| line.contains(&story_needle))
        {
            // The legend line already names the story key, so the chip is not
            // repeated — but the **pointer** keeps the way in: that line is the
            // story target (it holds no other control).
            hits.add(
                band(r, r.height.saturating_sub(2 + i as u16), 1),
                Target::Story,
            );
        }
        if !named(&format!("{quit_key} quit")) {
            parts.push((quit_chip.as_str(), None));
        }
        controls(f, band(r, r.height - 1, 1), hits, &parts);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_levelup(
    f: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    upgrades: &[Upgrade],
    idx: usize,
    auto_pick: bool,
    auto_key: &str,
    hits: &mut HitMap,
) {
    draw_levelup_keyed(
        f, area, level, choices, upgrades, idx, auto_pick, auto_key, "Enter", hits,
    );
}

/// [`draw_levelup`] with the Confirm key named by the caller: the legend's
/// `Enter equip` is `Action::Confirm`, so the App should pass
/// `controls.action_label(Action::Confirm)` here (see the batch 6 report for
/// the call site). `draw_levelup` passes the default key's name.
#[allow(clippy::too_many_arguments)]
pub fn draw_levelup_keyed(
    f: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    upgrades: &[Upgrade],
    idx: usize,
    auto_pick: bool,
    auto_key: &str,
    confirm_key: &str,
    hits: &mut HitMap,
) {
    let n = choices.len().max(1) as u16;
    let large_cards = area.height >= n * 6 + 3 && area.width >= 64;
    let target_stride: u16 = if large_cards { 6 } else { 4 };
    let panel_w: u16 = if large_cards { 72 } else { 66 };
    // One extra row for the AUTO-PICK pill above the legend.
    let panel_h: u16 = if large_cards {
        n * 6 + (if area.height >= n * 6 + 5 { 5 } else { 3 }) + 1
    } else {
        n * 4 + 6
    };
    let r = panel(
        f,
        centered_rect(panel_w, panel_h, area),
        &levelup_header(level),
        TAIL,
    );
    let content = r.inner(Margin::new(1, 0));
    let stride = (content.height.saturating_sub(2) / n).min(target_stride);
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
            // Fallback consumables (`WAVE3_DESIGN.md` §2).
            Some(Upgrade::Bandage) => ("BANDAGE", art_icons::passive(
                crate::game::loadout::PassiveKind::Regen,
            )),
            Some(Upgrade::Purse) => ("PURSE", &art_icons::GOLD_COIN),
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
            Paragraph::new(levelup_card_lines(
                desc.as_str(),
                flavor.as_str(),
                text_rect.width,
                text_rect.height,
            )),
            text_rect,
        );
    }
    draw_auto_pick_pill(f, r, r.height.saturating_sub(2), auto_pick, hits);
    center(
        f,
        r,
        r.height.saturating_sub(1),
        Line::styled(
            format!(
                "↑↓ select · {confirm_key} equip · {} · {auto_key} auto-pick",
                pick_chip(choices.len())
            ),
            ink(MUTED),
        ),
    );
}

/// The number-key legend for a card row: the plate said `1–3 pick` even on a
/// screen holding one card, and only the cards that are on it answer.
fn pick_chip(cards: usize) -> String {
    match cards {
        0 | 1 => "1 pick".to_string(),
        n => format!("1–{n} pick"),
    }
}

/// Greedy word wrap to `width` cells. A word longer than a whole row is split
/// across rows, so nothing is ever lost off the right edge.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for word in text.split_whitespace() {
        let mut rest = word;
        loop {
            let len = rest.chars().count();
            let gap = usize::from(used > 0);
            if used + gap + len <= width {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(rest);
                used += gap + len;
                break;
            }
            if used > 0 {
                lines.push(std::mem::take(&mut line));
                used = 0;
                continue;
            }
            // A single word wider than the row.
            let cut = rest
                .char_indices()
                .nth(width)
                .map_or(rest.len(), |(i, _)| i);
            lines.push(rest[..cut].to_string());
            rest = &rest[cut..];
            if rest.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Keep at most `cap` of `lines`. When rows are dropped, the last kept row ends
/// in `…` on a word boundary, so the cut is marked and never a half word.
fn clip_lines(lines: &mut Vec<String>, cap: usize, width: usize) {
    if lines.len() <= cap {
        return;
    }
    lines.truncate(cap);
    let width = width.max(1);
    if let Some(last) = lines.last_mut() {
        while last.chars().count() + 2 > width {
            match last.rfind(' ') {
                Some(i) => last.truncate(i),
                None => {
                    last.clear();
                    break;
                }
            }
        }
        if !last.is_empty() {
            last.push(' ');
        }
        last.push('…');
    }
}

/// Format `(desc, flavor)` for a level-up card's `text_rect`: wrapped, never
/// clipped mid-word.
///
/// The stat line comes first and may take every row. An evolution partner note
/// (the part of `desc` after `" · "`) rides on the stat line where a two-row
/// card has the width for both, gets a row of its own on a taller card, and is
/// left out of a two-row card it does not fit. The flavor line takes whatever
/// rows remain, wrapped, and ends in `…` when it needs more than there are.
fn levelup_card_lines<'a>(
    desc: &'a str,
    flavor: &'a str,
    width: u16,
    height: u16,
) -> Vec<Line<'a>> {
    let w = usize::from(width).max(1);
    let rows = usize::from(height);
    let (main, partner) = match desc.split_once(" · ") {
        Some((main, partner)) => (main, Some(partner)),
        None => (desc, None),
    };
    let joined = rows < 3 && partner.is_some() && desc.chars().count() <= w;
    let top = if joined { desc } else { main };
    let partner = if joined || rows < 3 { None } else { partner };

    let mut top_lines = wrap_words(top, w);
    clip_lines(&mut top_lines, rows, w);
    let left = rows.saturating_sub(top_lines.len());
    // The partner note is build information, so it outranks the flavor line
    // for the last row; it is still printed under it.
    let partner = partner.filter(|_| left >= 1);
    let flavor_rows = left - usize::from(partner.is_some());
    let mut flavor_lines = if flavor_rows > 0 {
        wrap_words(flavor, w)
    } else {
        Vec::new()
    };
    clip_lines(&mut flavor_lines, flavor_rows, w);

    let mut out: Vec<Line<'a>> = Vec::with_capacity(rows);
    for line in top_lines {
        out.push(Line::styled(line, ink(COAT)));
    }
    for line in flavor_lines {
        out.push(Line::styled(line, ink(MUTED).add_modifier(Modifier::ITALIC)));
    }
    if let Some(partner) = partner {
        let p_col = if partner.contains("READY") {
            TRAIL_GOLD
        } else {
            MUTED
        };
        // One row: a note wider than the card is cut on a word.
        let mut note = wrap_words(partner, w);
        clip_lines(&mut note, 1, w);
        for line in note {
            out.push(Line::styled(line, ink(p_col)));
        }
    }
    out
}

/// The AUTO-PICK toggle on the level-up screen: one centred pill below the
/// cards. Clicking it flips the preference; the AUTO-PICK key (`U`) does the same.
/// Drawn as a real widget so the mouse hit target is the pill itself.
fn draw_auto_pick_pill(f: &mut Frame, r: Rect, row: u16, on: bool, hits: &mut HitMap) {
    let text = if on { " AUTO-PICK: ON " } else { " AUTO-PICK: OFF " };
    let w = text.chars().count() as u16;
    let band = band(r, row, 1);
    if band.is_empty() || w > band.width {
        return;
    }
    let rect = Rect::new(band.x + (band.width - w) / 2, band.y, w, 1);
    let style = if on {
        heavy(PLATE).bg(TAIL)
    } else {
        heavy(COAT).bg(TRACK)
    };
    f.render_widget(Line::styled(text, style), rect);
    hits.add(rect, Target::AutoPickToggle);
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
        // A washer drum and a press roller: the two side-route floors.
        Theme::Laundry => [
            "  ╭───────╮  ╭──╮ ",
            "──┤ ▓▓▓▓▓ ├──┤░░├─",
            "  ╰───────╯  ╰──╯ ",
        ],
        Theme::Printworks => [
            "  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ",
            "──█▓▓▓▓▓▓▓▓▓▓▓▓█──",
            "  ▀▀▀▀▀▀▀▀▀▀▀▀▀▀  ",
        ],
        Theme::Trailer => [
            " ▚▚▚▚▚▚▚▚▚▚▚▚▚▚▚▚ ",
            " ███▄▄███▄▄███▄▄██ ",
            "══╧══╧══╧══╧══╧══╧═",
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

/// The level-up plate's header: the level reached and the weapon damage that
/// level is worth (`player::level_damage_mult`).
pub fn levelup_header(level: u32) -> String {
    format!(
        " LEVEL UP   /   LV {level:02}   /   +{}% WEAPON DAMAGE ",
        crate::game::player::level_damage_percent(level)
    )
}

pub fn draw_pause(f: &mut Frame, area: Rect, controls_src: &Controls, hits: &mut HitMap) {
    // Nine rows: the border's two plus seven content rows, the last of which
    // (content row 6) is the legend. The plate used to be eight rows tall,
    // which left six content rows, so the `r.height >= 7` legend below could
    // never be drawn. A terminal shorter than nine rows still clamps the plate
    // and drops the legend, never the controls.
    let r = panel(f, centered_rect(46, 9, area), " RUN CONTROL ", MUTED);
    center(f, r, 1, Span::styled("━━━  ▌▌  ━━━", heavy(TAIL)));
    center(f, r, 2, Span::styled("PAUSED", heavy(COAT)));
    // The keys are read from the live bindings, the way the `L` library chip
    // is: `Esc` and `O` are `Action::Pause` / `Action::Options` and a rebind
    // must move the text with it. `Q` has no action (it is a raw key in the
    // App), so it stays literal — see the report on reserved keys.
    let pause_key = controls_src.action_label(Action::Pause);
    let opts_key = controls_src.action_label(Action::Options);
    let resume_chip = format!(" {pause_key} resume ");
    let opts_chip = format!(" {opts_key} options ");
    let menu_chip = format!(" {} to menu ", controls_src.action_label(Action::Quit));
    controls(
        f,
        band(r, 4, 1),
        hits,
        &[
            (resume_chip.as_str(), Some(Target::Resume)),
            ("  ", None),
            (opts_chip.as_str(), Some(Target::Options)),
            ("  ", None),
            (menu_chip.as_str(), Some(Target::Menu)),
        ],
    );
    if r.height >= 7 {
        // The narrow plate still names the dash key (the full plate has its own
        // legend line); `O options` is already on the controls row above.
        // The volume keys are actions too, so they follow their bindings.
        center(
            f,
            r,
            6,
            label(
                &format!(
                    "{} {} music · {} {} fx · {} dash · {} library",
                    controls_src.action_label(Action::MusicDown),
                    controls_src.action_label(Action::MusicUp),
                    controls_src.action_label(Action::EffectsDown),
                    controls_src.action_label(Action::EffectsUp),
                    controls_src.live_dash_label(),
                    controls_src.action_label(Action::Library)
                ),
                MUTED,
            ),
        );
    }
}

/// The short-terminal pause plate: banner, clock and the **full loadout** as
/// two half-block icon rows, then the controls legend.
///
/// Reached from [`draw_pause_with_run`] when the terminal is too small for the
/// 58×18 dossier (the review's 57×20 case). It carries the same information the
/// dossier does in the space available — weapon icons with their rank, passive
/// icons with their stack count, an evolved weapon marked in gold — so the
/// loadout never disappears just because the terminal is short.
pub fn draw_pause_compact(
    f: &mut Frame,
    area: Rect,
    world: &World,
    muted: bool,
    controls_src: &Controls,
    hits: &mut HitMap,
) {
    let w = area.width.saturating_sub(2).min(56);
    // Rows: banner, clock, rule, a weapon icon pair + its tag row, a passive
    // icon pair + its tag row, one spare row, two legend lines and the controls
    // row — 13 inner rows.
    //
    // The tag rows are what makes the short plate the reference for **names**: a
    // full 8 + 8 rack cannot carry a name beside every icon here, so each band
    // is followed by one compact line (`GN2 CSP1 ASP3 …` / `HP3 DMG2 …`) naming
    // every kind in the rack, rank and stack included.
    let h = area.height.saturating_sub(2).min(15);
    // Decide the fallback from the plate's content size (the border takes two
    // of each) *before* painting it: drawing this plate and then the bare one
    // over it left the first plate's edges showing around the second.
    if h.saturating_sub(2) < 9 || w.saturating_sub(2) < 30 {
        draw_pause(f, area, controls_src, hits);
        return;
    }
    let r = panel(f, centered_rect(w, h, area), " RUN CONTROL ", MUTED);
    center(f, r, 0, Span::styled("━━━  ▌▌  PAUSED  ━━━", heavy(TAIL)));
    let clock_text = if world.boss_time_reached() {
        let fought = (world.elapsed - crate::game::director::BOSS_TIME).max(0.0) as u32;
        format!("BOSS FIGHT +{:02}:{:02}", fought / 60, fought % 60)
    } else {
        let rem = world.time_left().max(0.0) as u32;
        format!(
            "{:02}:{:02} to boss  ·  Lv {}  ·  +{}% damage",
            rem / 60,
            rem % 60,
            world.player.level,
            crate::game::player::level_damage_percent(world.player.level)
        )
    };
    center(f, r, 1, label(&clock_text, COAT));
    let rule = "─".repeat(usize::from(r.width));
    f.render_widget(Line::styled(rule.as_str(), ink(TRACK)), band(r, 2, 1));

    // The two racks: an icon band plus its tag row. The icon is 4×2 cells, so a
    // slot is icon + one rank/stack cell; an empty bay up to the loadout's caps
    // keeps its footprint as a dim dot, and a `+N` marker stands for anything
    // the width cannot show (nothing is silently cut).
    let icon_w = crate::game::art_icons::SMALL_CELL_W;
    let icon_h = crate::game::art_icons::SMALL_CELL_H;
    let pitch = icon_w + 1;
    let loadout = &world.player.loadout;
    let buf = f.buffer_mut();
    let fits = usize::from(r.width / pitch).max(1);
    let slots_w = loadout.weapons.len().max(loadout.max_weapons());
    let slots_p = loadout.passives.len().max(loadout.max_passives());
    let marker_w: u16 = if slots_w + slots_p > fits { 2 } else { 0 };
    let room = usize::from(r.width.saturating_sub(marker_w)) / pitch as usize;
    let show_w = slots_w.min(room);
    let show_p = slots_p.min(room.saturating_sub(show_w));

    let weapon_y = r.y + 3;
    let weapon_tag_y = weapon_y + icon_h;
    let passive_y = weapon_tag_y + 1;
    let passive_tag_y = passive_y + icon_h;
    let x = r.x;
    if passive_tag_y < r.bottom().saturating_sub(2) {
        for i in 0..show_w {
            let icon_x = x + i as u16 * pitch;
            match loadout.weapons.get(i) {
                Some(w) => {
                    let icon = crate::game::art_icons::weapon(w.kind);
                    let rect = Rect::new(icon_x, weapon_y, icon_w, icon_h);
                    if w.evolved {
                        crate::game::art_icons::draw_evolved(buf, rect, icon);
                    } else {
                        crate::game::art_icons::draw(buf, rect, icon);
                    }
                    let (rank, style) = if w.evolved {
                        ('◆', heavy(TRAIL_GOLD))
                    } else if w.is_max() {
                        ('★', heavy(TRAIL_GOLD))
                    } else {
                        (char::from(b'0' + w.level.min(9)), ink(MUTED))
                    };
                    let mut tbuf = [0u8; 4];
                    buf.set_stringn(
                        icon_x + icon_w,
                        weapon_y + icon_h - 1,
                        rank.encode_utf8(&mut tbuf),
                        1,
                        style,
                    );
                    let tag = crate::render::hud::weapon_tag(w.kind, w.evolved);
                    let head: String = tag.chars().take(2).collect();
                    let line = format!(
                        "{}{}",
                        head,
                        if w.is_max() {
                            "*".to_string()
                        } else {
                            w.level.min(9).to_string()
                        }
                    );
                    buf.set_stringn(
                        icon_x,
                        weapon_tag_y,
                        &line,
                        pitch as usize,
                        if w.is_max() || w.evolved {
                            heavy(TRAIL_GOLD)
                        } else {
                            ink(to_color(w.kind.color()))
                        },
                    );
                }
                None => {
                    // An unlocked bay nobody has filled yet.
                    buf.set_stringn(icon_x + 1, weapon_y + icon_h - 1, "··", 2, ink(TRACK));
                    buf.set_stringn(icon_x, weapon_tag_y, "··", pitch as usize, ink(TRACK));
                }
            }
        }

        for i in 0..show_p {
            let icon_x = x + i as u16 * pitch;
            match loadout.passives.get(i) {
                Some(&(kind, stacks)) => {
                    let icon = crate::game::art_icons::passive(kind);
                    crate::game::art_icons::draw(
                        buf,
                        Rect::new(icon_x, passive_y, icon_w, icon_h),
                        icon,
                    );
                    let capped = stacks >= kind.max_stacks();
                    let digit = char::from(b'0' + (stacks.min(9) as u8));
                    let mut tbuf = [0u8; 4];
                    buf.set_stringn(
                        icon_x + icon_w,
                        passive_y + icon_h - 1,
                        digit.encode_utf8(&mut tbuf),
                        1,
                        if capped { heavy(TRAIL_GOLD) } else { ink(COAT) },
                    );
                    let tag = crate::render::hud::passive_short_tag(kind);
                    let line = format!("{}{}", tag, stacks.min(9));
                    buf.set_stringn(
                        icon_x,
                        passive_tag_y,
                        &line,
                        pitch as usize,
                        if capped {
                            heavy(TRAIL_GOLD)
                        } else {
                            ink(to_color(icon.accent))
                        },
                    );
                }
                None => {
                    buf.set_stringn(icon_x + 1, passive_y + icon_h - 1, "··", 2, ink(TRACK));
                    buf.set_stringn(icon_x, passive_tag_y, "··", pitch as usize, ink(TRACK));
                }
            }
        }

        // `+N` for the bays this plate could not show (a full 8 + 8 rack at a
        // narrow width).
        let hidden = slots_w + slots_p - show_w - show_p;
        if hidden > 0 && marker_w > 0 {
            let s = format!("+{hidden}");
            let mx = r.right().saturating_sub(marker_w);
            buf.set_stringn(mx, weapon_y, &s, marker_w as usize, heavy(WARN_AMBER));
            buf.set_stringn(mx, passive_y, &s, marker_w as usize, heavy(WARN_AMBER));
        }
    }

    // Legend + controls, bottom-up: both legend lines fit, then the controls
    // row with the mute chip (`M unmute` while muted), so the legend is never
    // replaced by the audio state on a short terminal either.
    let legend = controls_src.pause_legend_lines(false);
    for (i, line) in legend.iter().take(2).enumerate() {
        let row = r.height.saturating_sub(2 + i as u16);
        center(f, r, row, label(line, MUTED));
    }
    let mute_label = controls_src.action_label(Action::Mute);
    let mute_txt = if muted {
        format!(" {mute_label} unmute ")
    } else {
        format!(" {mute_label} mute ")
    };
    let lib_chip = format!(" {} lib ", controls_src.action_label(Action::Library));
    let pause_chip = format!(" {} resume ", controls_src.action_label(Action::Pause));
    let opts_chip = format!(" {} opts ", controls_src.action_label(Action::Options));
    let menu_chip = format!(" {} to menu ", controls_src.action_label(Action::Quit));
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (pause_chip.as_str(), Some(Target::Resume)),
            (" ", None),
            (opts_chip.as_str(), Some(Target::Options)),
            (" ", None),
            (lib_chip.as_str(), Some(Target::Library)),
            (" ", None),
            (mute_txt.as_str(), None),
            (" ", None),
            (menu_chip.as_str(), Some(Target::Menu)),
        ],
    );
}

/// Paint symmetric golden opening-flourish rays flanking a centered showcase box.
fn draw_chest_rays(f: &mut Frame, r: Rect, icon_x: u16, icon_w: u16, mid_y: u16) {
    let gold = Color::Rgb(255, 214, 78);
    let warm = Color::Rgb(218, 152, 46);
    let dim = Color::Rgb(138, 96, 36);
    let buf = f.buffer_mut();

    let left_avail = icon_x.saturating_sub(r.x + 1);
    let right_x = icon_x + icon_w + 1;
    let right_avail = r.right().saturating_sub(right_x + 1);

    let put = |buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, s: &str, c: Color| {
        if y >= r.y && y < r.bottom() && x >= r.x && x < r.right() {
            let max_w = usize::from(r.right().saturating_sub(x));
            buf.set_stringn(x, y, s, max_w, ink(c));
        }
    };

    if left_avail >= 11 && right_avail >= 11 {
        let lx = icon_x - 11;
        put(buf, lx + 2, mid_y.saturating_sub(1), "·  ✦  ───", warm);
        put(buf, lx, mid_y, "═══ ◈ ━━━", gold);
        put(buf, lx + 2, mid_y + 1, "·  ✦  ───", dim);

        put(buf, right_x, mid_y.saturating_sub(1), "───  ✦  ·", warm);
        put(buf, right_x, mid_y, "━━━ ◈ ═══", gold);
        put(buf, right_x, mid_y + 1, "───  ✦  ·", dim);
    } else if left_avail >= 6 && right_avail >= 6 {
        let lx = icon_x - 6;
        put(buf, lx, mid_y, "· ✦ ━━", gold);
        put(buf, right_x, mid_y, "━━ ✦ ·", gold);
    }
}

/// One line naming the whole build — `ARSENAL ▸GN2 ▸CSP3 │ HP3 DMG2 …` — for
/// the level-up cards' footer strip and the chest card.
///
/// The racks run to 8 + 8, so the line measures itself against `limit` cells:
/// as many chips as fit, then a `+N` marker for the rest. Nothing is silently
/// clipped, at any width. (Owned `String`s: this is a menu, not the HUD.)
fn loadout_strip(loadout: &Loadout, limit: usize) -> Vec<Span<'static>> {
    let mut chips: Vec<(String, Color, bool)> =
        Vec::with_capacity(loadout.weapons.len() + loadout.passives.len());
    for w in &loadout.weapons {
        let tag = weapon_tag(w.kind, w.evolved);
        let head: String = tag.chars().take(2).collect();
        let lvl = if w.is_max() || w.evolved {
            "★".to_string()
        } else {
            w.level.to_string()
        };
        chips.push((
            format!("{}{head}{lvl}", weapon_glyph(w.kind)),
            to_color(w.kind.color()),
            w.is_max() || w.evolved,
        ));
    }
    let weapons = chips.len();
    for &(pk, stacks) in &loadout.passives {
        let tag = crate::render::hud::passive_short_tag(pk);
        let head: String = tag.chars().take(2).collect();
        chips.push((format!("{head}{stacks}"), HEAL_MINT, stacks >= pk.max_stacks()));
    }
    // One space between chips, a `│` between the racks, two cells kept back for
    // the `+N` marker as soon as anything is dropped.
    let mut used = "ARSENAL ".len();
    let mut drawn = 0usize;
    for (i, (text, _, _)) in chips.iter().enumerate() {
        let gutter = if i == 0 {
            0
        } else if i == weapons {
            2
        } else {
            1
        };
        let more = if i + 1 < chips.len() { 2 } else { 0 };
        if used + text.chars().count() + gutter + more > limit {
            break;
        }
        used += text.chars().count() + gutter;
        drawn = i + 1;
    }
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(drawn * 2 + 3);
    spans.push(label("ARSENAL ", MUTED));
    for (i, (text, col, maxed)) in chips.into_iter().take(drawn).enumerate() {
        if i == weapons && i > 0 {
            spans.push(label(" │ ", MUTED));
        } else if i > 0 {
            spans.push(label(" ", MUTED));
        }
        spans.push(Span::styled(
            text,
            if maxed {
                heavy(TRAIL_GOLD)
            } else {
                ink(col)
            },
        ));
    }
    if drawn < weapons + loadout.passives.len() {
        let rest = weapons + loadout.passives.len() - drawn;
        spans.push(label(" ", MUTED));
        spans.push(Span::styled(format!("+{rest}"), heavy(WARN_AMBER)));
    }
    spans
}

/// The chest card. Opens with a gilded sunburst flourish framing the showcase
/// icon: evolutions receive the full royal plate (`10×5` gilded icon, chest
/// name, recipe, inscription and new attack); rank-ups and gold payouts center
/// the large `8×4` icon inside matching golden opening rays.
pub fn draw_chest(
    f: &mut Frame,
    area: Rect,
    reward: ChestReward,
    loadout: &Loadout,
    hits: &mut HitMap,
) {
    let tall = area.height >= 19 && area.width >= 62;
    let is_evolve = matches!(reward, ChestReward::Evolve { .. });
    // On the compact card the evolved attack wraps onto two rows when the
    // terminal has the height for it (the review's "compact chest text is a
    // single clipped attack line"); a really short terminal keeps the one-liner.
    let roomy = area.height >= 12;
    let (w, h) = if tall {
        if is_evolve { (64, 16) } else { (54, 13) }
    } else {
        (46, 10)
    };
    let r = panel(f, centered_rect(w, h, area), " CHEST OPENED ", TRAIL_GOLD);
    let last = r.height.saturating_sub(1);
    let gold_hi = Color::Rgb(255, 224, 96);
    let gold_dim = Color::Rgb(168, 122, 48);

    match reward {
        ChestReward::Evolve { kind, evo, partner_stacks, .. } => {
            let partner = format!(
                "{}  +  {} x{}",
                kind.name(),
                evo.passive.name(),
                partner_stacks.max(1)
            );
            let inscription = format!("\u{201c}{}\u{201d}", evo.flavor);
            if tall {
                center(
                    f,
                    r,
                    0,
                    Line::from(vec![
                        label("·  ✦  ━━━  ", gold_dim),
                        Span::styled("◈  EVOLUTION UNLOCKED  ◈", heavy(gold_hi)),
                        label("  ━━━  ✦  ·", gold_dim),
                    ]),
                );
                let icon_x = r.x + (r.width.saturating_sub(10)) / 2;
                let icon_area = Rect::new(icon_x, r.y + 1, 10, 5);
                draw_chest_rays(f, r, icon_x, 10, r.y + 3);
                art_icons::draw_evolved(f.buffer_mut(), icon_area, art_icons::weapon(kind));
                center(
                    f,
                    r,
                    6,
                    Line::from(vec![
                        label("◆  ", WARN_AMBER),
                        Span::styled(evo.name.to_uppercase(), heavy(TRAIL_GOLD)),
                        label("  ◆", WARN_AMBER),
                    ]),
                );
                center(f, r, 7, label(&partner, WARN_AMBER));
                center(
                    f,
                    r,
                    8,
                    Span::styled(inscription, ink(COAT).add_modifier(Modifier::ITALIC)),
                );
                let body = Rect::new(r.x + 3, r.y + 9, r.width.saturating_sub(6), 3);
                f.render_widget(
                    Paragraph::new(evo.attack)
                        .alignment(ratatui::layout::Alignment::Center)
                        .wrap(Wrap { trim: true })
                        .style(ink(COAT)),
                    body,
                );
                if last >= 13 {
                    center(f, r, 12, label("─────────── ✦ ───────────", gold_dim));
                }
            } else {
                center(f, r, 0, label("✦ ━━━  EVOLUTION  ━━━ ✦", gold_hi));
                center(f, r, 1, Span::styled(evo.name.to_uppercase(), heavy(TRAIL_GOLD)));
                center(f, r, 2, label(&partner, MUTED));
                center(f, r, 3, label(&inscription, COAT));
                if roomy {
                    // Two wrapped rows instead of one clipped line.
                    f.render_widget(
                        Paragraph::new(evo.attack)
                            .alignment(ratatui::layout::Alignment::Center)
                            .wrap(Wrap { trim: true })
                            .style(ink(MUTED)),
                        Rect::new(r.x + 1, r.y + 4, r.width.saturating_sub(2), 2),
                    );
                } else {
                    center(f, r, 4, label(evo.attack, MUTED));
                }
            }
        }
        ChestReward::WeaponLevel { kind, level } => {
            let title = format!("{} reached Lv {level}", kind.name());
            let note = "Filed under spare parts.";
            let icon = art_icons::weapon(kind);
            if tall {
                center(
                    f,
                    r,
                    0,
                    Line::from(vec![
                        label("·  ✦  ━━━  ", gold_dim),
                        Span::styled("◈  TREASURE CLAIMED  ◈", heavy(gold_hi)),
                        label("  ━━━  ✦  ·", gold_dim),
                    ]),
                );
                let icon_x = r.x + (r.width.saturating_sub(art_icons::LARGE_CELL_W)) / 2;
                let icon_area = Rect::new(
                    icon_x,
                    r.y + 2,
                    art_icons::LARGE_CELL_W,
                    art_icons::LARGE_CELL_H,
                );
                draw_chest_rays(f, r, icon_x, art_icons::LARGE_CELL_W, r.y + 3);
                art_icons::draw_large(f.buffer_mut(), icon_area, icon);
                center(f, r, 6, Span::styled(title, heavy(COAT)));
                center(
                    f,
                    r,
                    7,
                    Span::styled(note, ink(MUTED).add_modifier(Modifier::ITALIC)),
                );
                center(f, r, 8, label("──────── ✦ ────────", gold_dim));
            } else {
                center(f, r, 0, label("✦ ━━━  TREASURE  ━━━ ✦", gold_hi));
                art_icons::draw_small(f.buffer_mut(), Rect::new(r.x + 2, r.y + 2, 4, 2), icon);
                center(f, r, 2, Span::styled(title, heavy(COAT)));
                center(f, r, 3, label(note, MUTED));
            }
        }
        ChestReward::PassiveLevel { kind, stacks } => {
            let title = format!("{} x{stacks}", kind.name());
            let note = "Filed under spare parts.";
            let icon = art_icons::passive(kind);
            if tall {
                center(
                    f,
                    r,
                    0,
                    Line::from(vec![
                        label("·  ✦  ━━━  ", gold_dim),
                        Span::styled("◈  TREASURE CLAIMED  ◈", heavy(gold_hi)),
                        label("  ━━━  ✦  ·", gold_dim),
                    ]),
                );
                let icon_x = r.x + (r.width.saturating_sub(art_icons::LARGE_CELL_W)) / 2;
                let icon_area = Rect::new(
                    icon_x,
                    r.y + 2,
                    art_icons::LARGE_CELL_W,
                    art_icons::LARGE_CELL_H,
                );
                draw_chest_rays(f, r, icon_x, art_icons::LARGE_CELL_W, r.y + 3);
                art_icons::draw_large(f.buffer_mut(), icon_area, icon);
                center(f, r, 6, Span::styled(title, heavy(COAT)));
                center(
                    f,
                    r,
                    7,
                    Span::styled(note, ink(MUTED).add_modifier(Modifier::ITALIC)),
                );
                center(f, r, 8, label("──────── ✦ ────────", gold_dim));
            } else {
                center(f, r, 0, label("✦ ━━━  TREASURE  ━━━ ✦", gold_hi));
                art_icons::draw_small(f.buffer_mut(), Rect::new(r.x + 2, r.y + 2, 4, 2), icon);
                center(f, r, 2, Span::styled(title, heavy(COAT)));
                center(f, r, 3, label(note, MUTED));
            }
        }
        ChestReward::Gold(value) => {
            let title = format!("+{value} gold");
            let note = "Someone left lunch money.";
            let icon = &art_icons::GOLD_COIN;
            if tall {
                center(
                    f,
                    r,
                    0,
                    Line::from(vec![
                        label("·  ✦  ━━━  ", gold_dim),
                        Span::styled("◈  TREASURE CLAIMED  ◈", heavy(gold_hi)),
                        label("  ━━━  ✦  ·", gold_dim),
                    ]),
                );
                let icon_x = r.x + (r.width.saturating_sub(art_icons::LARGE_CELL_W)) / 2;
                let icon_area = Rect::new(
                    icon_x,
                    r.y + 2,
                    art_icons::LARGE_CELL_W,
                    art_icons::LARGE_CELL_H,
                );
                draw_chest_rays(f, r, icon_x, art_icons::LARGE_CELL_W, r.y + 3);
                art_icons::draw_large(f.buffer_mut(), icon_area, icon);
                center(f, r, 6, Span::styled(title, heavy(TRAIL_GOLD)));
                center(
                    f,
                    r,
                    7,
                    Span::styled(note, ink(MUTED).add_modifier(Modifier::ITALIC)),
                );
                center(f, r, 8, label("──────── ✦ ────────", gold_dim));
            } else {
                center(f, r, 0, label("✦ ━━━  TREASURE  ━━━ ✦", gold_hi));
                art_icons::draw_small(f.buffer_mut(), Rect::new(r.x + 2, r.y + 2, 4, 2), icon);
                center(f, r, 2, Span::styled(title, heavy(TRAIL_GOLD)));
                center(f, r, 3, label(note, MUTED));
            }
        }
    }
    // The build the reward lands in, above the controls: 8 + 8 racks measure
    // themselves into the card and a `+N` marker covers what does not fit.
    if last >= 4 {
        let spans = loadout_strip(loadout, r.width.saturating_sub(2) as usize);
        center(f, r, last.saturating_sub(2), Line::from(spans));
    }
    controls(f, band(r, last, 1), hits, &[(" Enter / Continue ", Some(Target::Continue))]);
}

#[allow(clippy::too_many_arguments)]
pub fn draw_end(
    f: &mut Frame,
    area: Rect,
    won: bool,
    score: u32,
    gold: u32,
    time: f32,
    level: u32,
    kills: u32,
    new_best: bool,
    lore_line: &str,
    scores: &ScoreTable,
    hits: &mut HitMap,
    sector: u8,
    // The floor the run was played on: the compact report names it, like the
    // expanded one does (a practice floor on a shared stage reads LAUNDRY, not
    // only its stage number).
    floor: Theme,
    practice: bool,
    // What the run banked into the nest (`crate::meta::BankTally`).
    nest: Option<&crate::meta::BankTally>,
    // The live bindings: the chips name the keys that actually work.
    controls_src: &Controls,
) {
    let (status, accent) = if won && practice {
        ("SECTOR CLEARED", Color::Rgb(169, 210, 186))
    } else if won {
        ("YOU SURVIVED", Color::Rgb(169, 210, 186))
    } else {
        ("YOU DIED", TAIL)
    };
    let r = panel(
        f,
        centered_rect(61, 21, area),
        &format!(
            " INCIDENT REPORT / {} / {} ",
            floor.name().to_uppercase(),
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
            Span::styled(format!("   ·   GOLD {gold}"), heavy(TRAIL_GOLD)),
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
    // The nest tally sits on row 9, under the numbers it came from. The plate
    // is at most 21 rows (19 of content), so the old `r.height >= 20` gate was
    // never true and the compact report never said what the run banked. A
    // plate with a spare row (the 57x20 one has exactly one) splits a tally too
    // long for one row into the breakdown and the sum, and the records table
    // moves down a row to make room.
    let mut table_shift: u16 = 0;
    if let (Some(nest), true) = (nest, r.height >= 11) {
        let style = if nest.voided {
            ink(WARN_AMBER)
        } else {
            heavy(TRAIL_GOLD)
        };
        let rows = if r.height >= 18 { 2 } else { 1 };
        let lines = nest_tally_lines(nest, r.width as usize, rows);
        for (i, line) in lines.iter().enumerate() {
            center(f, r, 9 + i as u16, Line::styled(line.as_str(), style));
        }
        table_shift = (lines.len() as u16).saturating_sub(1);
    }
    let table = centered_rect(35, inner.height, inner);
    // The header needs a row of its own above the chips.
    if r.height > 11 + table_shift {
        f.render_widget(
            Line::styled("RK    SCORE   SECTOR         TIME", ink(accent)),
            band(table, 10 + table_shift, 1),
        );
    }
    for (i, e) in scores
        .entries
        .iter()
        .take(usize::from(r.height.saturating_sub(12 + table_shift)).min(5))
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
            band(table, 11 + table_shift + i as u16, 1),
        );
    }
    let [restart_chip, nest_chip, menu_chip] = end_chips(controls_src);
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (restart_chip.as_str(), Some(Target::Restart)),
            ("   ", None),
            (nest_chip.as_str(), Some(Target::Nest)),
            ("   ", None),
            (menu_chip.as_str(), Some(Target::Menu)),
        ],
    );
}

/// The nest tally as the lines an end plate prints: `max` cells wide, on up to
/// `rows` rows.
///
/// The wording is [`crate::meta::BankTally::line`]'s own (nothing is retyped
/// here). The whole line is used where it fits. Where it does not and the plate
/// has two rows, it is split at the `=` into the breakdown (purse, floor, boss
/// and clear bonuses) and the sum with the new balance, so the report still
/// says where the gold came from. Otherwise it is the short gain-and-balance
/// form, which keeps the practice half-rate note whenever that fits: a halved
/// run must not look like a full-rate one. A voided (console) tally is one
/// line.
fn nest_tally_lines(nest: &crate::meta::BankTally, max: usize, rows: usize) -> Vec<String> {
    const HALF: &str = "(practice: half)";
    let fits = |s: &str| s.chars().count() <= max;
    let full = nest.line();
    if nest.voided || fits(&full) {
        return vec![full];
    }
    if rows >= 2 {
        if let Some((parts, sum)) = full.split_once(" = ") {
            let sum = format!("= {sum}");
            if fits(parts) && fits(&sum) {
                return vec![parts.to_string(), sum];
            }
            // The half-rate note rides on the shorter row.
            if let Some(bare) = parts.strip_suffix(HALF) {
                let bare = bare.trim_end();
                let noted = format!("{sum}  {HALF}");
                if fits(bare) && fits(&noted) {
                    return vec![bare.to_string(), noted];
                }
            }
        }
    }
    let short = nest.line_within(max);
    if nest.halved && !short.contains(HALF) {
        let noted = format!("{short}  {HALF}");
        if fits(&noted) {
            return vec![noted];
        }
    }
    vec![short]
}

/// The end screens' three chips, named from the live bindings so a rebind
/// moves the text with the key.
fn end_chips(controls_src: &Controls) -> [String; 3] {
    [
        format!(" {} restart ", controls_src.action_label(Action::Confirm)),
        format!(" {} the nest ", controls_src.action_label(Action::Nest)),
        format!(" {} to menu ", controls_src.action_label(Action::Quit)),
    ]
}

/// Snapshot of a completed run's metrics for the expanded end-of-run Incident Report.
#[derive(Clone, Debug)]
pub struct RunSummary {
    pub won: bool,
    pub theme: Theme,
    pub combat_score: u32,
    pub gold: u32,
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
    /// The nest's banking tally for this run (`crate::meta::bank`): what the
    /// purse, the floor/apex bonuses and the clear put in the bank, and the new
    /// total. `None` on a report drawn outside a finished run (the tests).
    pub nest: Option<crate::meta::BankTally>,
    /// The run used the admin console (no score or record was saved).
    pub console_used: bool,
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
    auto_pick: bool,
    auto_key: &str,
    hits: &mut HitMap,
) {
    draw_levelup_with_loadout_keyed(
        f, area, level, choices, upgrades, loadout, idx, auto_pick, auto_key, "Enter", hits,
    );
}

/// [`draw_levelup_with_loadout`] with the Confirm key named by the caller (the
/// legend's `Enter equip` is `Action::Confirm`). The unkeyed function passes
/// the default key's name.
#[allow(clippy::too_many_arguments)]
pub fn draw_levelup_with_loadout_keyed(
    f: &mut Frame,
    area: Rect,
    level: u32,
    choices: &[(String, String, String)],
    upgrades: &[Upgrade],
    loadout: &Loadout,
    idx: usize,
    auto_pick: bool,
    auto_key: &str,
    confirm_key: &str,
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
    // One extra row for the AUTO-PICK pill above the legend.
    let panel_h: u16 = if large_cards {
        n * 6 + (if show_loadout { if airy { 7 } else { 5 } } else { 3 }) + 1
    } else {
        n * 4 + (if show_loadout { 7 } else { 5 }) + 1
    };
    let r = panel(
        f,
        centered_rect(panel_w, panel_h, area),
        &levelup_header(level),
        TAIL,
    );
    let content = r.inner(Margin::new(1, 0));
    let footer_rows: u16 = if show_loadout {
        if large_cards && !airy { 3 } else { 4 }
    } else {
        2
    };
    let stride = (content.height.saturating_sub(footer_rows) / n).min(target_stride);
    let top_pad = if large_cards {
        u16::from(airy && content.height > n * stride + footer_rows)
    } else {
        // Centre the stack in the rows above the footer. Three cards on the
        // 57x20 plate have two spare rows (one above, one below, as before);
        // one or two cards used to sit against the top edge with the spare
        // rows all underneath.
        content.height.saturating_sub(footer_rows + n * stride) / 2
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
            // Fallback consumables (`WAVE3_DESIGN.md` §2): they read as
            // supplies, not gear, and neither touches a rack slot.
            Some(Upgrade::Bandage) => (
                "SUPPLY",
                HEAL_MINT,
                None,
                art_icons::passive(crate::game::loadout::PassiveKind::Regen),
            ),
            Some(Upgrade::Purse) => ("SUPPLY", TRAIL_GOLD, None, &art_icons::GOLD_COIN),
            None => ("", MUTED, None, art_icons::placeholder()),
        };

        let border = if selected { TAIL } else { EDGE };
        let name = format!(
            " {}  {:02}   {} ",
            if selected { "◆" } else { "◇" },
            i + 1,
            title
        );
        // The title sits on the top border from the card's second column.
        let name_w = name.chars().count() as u16;
        let block = Block::bordered()
            .border_style(ink(border))
            .title(Span::styled(
                name,
                heavy(if selected { TAIL } else { COAT }),
            ));
        let inner = block.inner(card).inner(Margin::new(1, 0));
        f.render_widget(block, card);

        // Top-right weapon glyph + next-level pips inside the card border.
        // Skipped when the title reaches them: a long evolved name used to be
        // overprinted by the pips on the same border row.
        if let Some(pips) = pips_preview {
            let pw = pips.chars().count() as u16 + 2;
            let clear_of_title = card.x + 1 + name_w < card.right().saturating_sub(pw + 2);
            if card.width >= 48
                && card.height >= 3
                && card.right() > card.x + pw + 28
                && clear_of_title
            {
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
            Paragraph::new(levelup_card_lines(
                desc.as_str(),
                flavor.as_str(),
                text_rect.width,
                text_rect.height,
            )),
            text_rect,
        );
    }

    // Current loadout strip so build decisions are informed right on the card
    // screen: every owned weapon (glyph + tag + rank) and passive (tag + stack),
    // as many as the panel can hold; the rack caps run to 8 + 8, so the strip
    // measures itself and ends in a `+N` marker rather than clipping a name.
    if show_loadout && r.height >= 4 {
        let strip_y = r.height.saturating_sub(if large_cards && !airy { 3 } else { 4 });
        let spans = loadout_strip(loadout, r.width.saturating_sub(4) as usize);
        center(f, r, strip_y, Line::from(spans));
    }

    draw_auto_pick_pill(f, r, r.height.saturating_sub(2), auto_pick, hits);
    center(
        f,
        r,
        r.height.saturating_sub(1),
        Line::styled(
            format!(
                "↑↓ select · {confirm_key} equip · {} · {auto_key} auto-pick",
                pick_chip(choices.len())
            ),
            ink(MUTED),
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
    controls_src: &Controls,
    hits: &mut HitMap,
) {
    if area.width < 58 || area.height < 21 {
        // A short terminal still gets the **loadout**: the review's compact
        // plate dropped the inventory entirely ("The short pause plate also
        // drops the inventory"), which left a 57×20 player with no way to see
        // what they were carrying. Below that even the compact plate needs a
        // bare legend.
        if area.width >= 40 && area.height >= 14 {
            draw_pause_compact(f, area, world, muted, controls_src, hits);
        } else {
            draw_pause(f, area, controls_src, hits);
        }
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
            // The plate follows the floor (a side route, or the getaway ride
            // borrowing the Lab's), not the stage's original theme.
            current: CampaignLevel::from_theme(world.level.theme),
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

        // Left: the weapon rack, icon + name + rank pips per entry (2 rows
        // each). One column holds `rows_avail / 2` slots; a full 8-slot rack on
        // a 22-row card needs two, so the columns split the rack and nothing is
        // dropped. A slot the build owns but the plate cannot show is counted
        // in a `+N` line.
        let loadout = &world.player.loadout;
        let slots_w = loadout.weapons.len().max(loadout.max_weapons());
        let max_slots = (rows_avail / 2) as usize;
        if loadout.weapons.is_empty() && slots_w == 0 {
            buf.set_stringn(inner.x, inner.y + body_row, "none", left_w as usize, ink(MUTED));
        } else {
            let per_col = max_slots.max(1);
            let cols = if slots_w <= per_col { 1 } else { 2 };
            let col_w = left_w / cols as u16;
            let shown = slots_w.min(per_col * cols);
            for idx in 0..shown {
                let col = (idx / per_col) as u16;
                let row = (idx % per_col) as u16;
                let ex = inner.x + col * col_w;
                let y = inner.y + body_row + row * 2;
                let Some(w) = loadout.weapons.get(idx) else {
                    // An unlocked bay with nothing in it yet.
                    buf.set_stringn(ex + 1, y + 1, "··", 2, ink(TRACK));
                    buf.set_stringn(ex + 4, y, "open slot", col_w.saturating_sub(5) as usize, ink(TRACK));
                    continue;
                };
                let icon_rect = Rect::new(ex, y, art_icons::SMALL_CELL_W, art_icons::SMALL_CELL_H);
                if w.evolved {
                    art_icons::draw_evolved(buf, icon_rect, art_icons::weapon(w.kind));
                } else {
                    art_icons::draw(buf, icon_rect, art_icons::weapon(w.kind));
                }
                let tx = ex + art_icons::SMALL_CELL_W + 2;
                let max_tw = (ex + col_w.saturating_sub(1)).saturating_sub(tx) as usize;
                let wcol = to_color(w.kind.color());
                // The chest name once evolved, so the pause screen lists
                // evolutions by the name the player earned.
                buf.set_stringn(tx, y, w.name(), max_tw, heavy(COAT));
                let pips = weapon_pips(w.level, w.kind.max_level());
                let pip_col = if w.is_max() { TRAIL_GOLD } else { wcol };
                let (px_end, _) = buf.set_stringn(tx, y + 1, &pips, max_tw, ink(pip_col));
                if w.evolved {
                    buf.set_stringn(
                        px_end + 1,
                        y + 1,
                        "EVO",
                        max_tw.saturating_sub(6),
                        heavy(TRAIL_GOLD),
                    );
                } else if w.is_max() {
                    buf.set_stringn(
                        px_end + 1,
                        y + 1,
                        "MAX",
                        max_tw.saturating_sub(6),
                        heavy(TRAIL_GOLD),
                    );
                } else if w.kind.is_active() {
                    // Name the key as well as the state: the review found the
                    // pause plate omitting the active-weapon trigger entirely.
                    let tag = if w.timer <= 0.0 { "SPACE READY" } else { "CHARGING" };
                    buf.set_stringn(
                        px_end + 1,
                        y + 1,
                        tag,
                        max_tw.saturating_sub(6),
                        ink(if w.timer <= 0.0 { wcol } else { MUTED }),
                    );
                }
            }
            let hidden = slots_w.saturating_sub(shown);
            if hidden > 0 {
                let y = inner.y + body_row + (shown as u16).min(per_col as u16 * 2);
                if y < body_bottom {
                    buf.set_stringn(
                        inner.x,
                        y,
                        format!("+{hidden} more"),
                        left_w as usize,
                        ink(WARN_AMBER),
                    );
                }
            }
        }

        // Right: the passive rack. Two entries per row when the plate is wide
        // enough for the names, icons and `×N` on one line; a full 8-passive
        // rack becomes 4 + 4 on two lines and no passive is dropped.
        let slots_p = loadout.passives.len().max(loadout.max_passives());
        let pass_hdr_y = inner.y + body_row + 7;
        let pass_icon_y = pass_hdr_y + 1;
        if pass_icon_y + art_icons::SMALL_CELL_H <= body_bottom + 2 {
            buf.set_stringn(right_x, pass_hdr_y, "PASSIVES", right_w as usize, heavy(accent));
            if loadout.passives.is_empty() && slots_p == 0 {
                buf.set_stringn(right_x, pass_icon_y, "none", right_w as usize, ink(MUTED));
            } else {
                let icon_w = art_icons::SMALL_CELL_W;
                let stack_w = 3;
                let per_row = (right_w / (icon_w + stack_w + 1)).max(1) as usize;
                let col_w = right_w / per_row as u16;
                for idx in 0..slots_p {
                    let row = (idx / per_row) as u16;
                    let col = (idx % per_row) as u16;
                    let y = pass_icon_y + row * art_icons::SMALL_CELL_H;
                    if y + art_icons::SMALL_CELL_H > body_bottom + 2 {
                        // No room for a further icon row: count the rest.
                        let rest = slots_p - idx;
                        if rest > 0 {
                            buf.set_stringn(
                                right_x,
                                pass_icon_y + art_icons::SMALL_CELL_H,
                                format!("+{rest}"),
                                right_w as usize,
                                ink(WARN_AMBER),
                            );
                        }
                        break;
                    }
                    let px = right_x + col * col_w;
                    match loadout.passives.get(idx) {
                        Some(&(pk, stacks)) => {
                            let ic = art_icons::passive(pk);
                            art_icons::draw(
                                buf,
                                Rect::new(px, y, icon_w, art_icons::SMALL_CELL_H),
                                ic,
                            );
                            let capped = stacks >= pk.max_stacks();
                            let stk = format!("×{stacks}");
                            buf.set_stringn(
                                px + icon_w,
                                y,
                                &stk,
                                stack_w as usize,
                                if capped { heavy(TRAIL_GOLD) } else { ink(to_color(ic.accent)) },
                            );
                            let tag = crate::render::hud::passive_short_tag(pk);
                            let tx = px + icon_w + stack_w;
                            buf.set_stringn(
                                tx,
                                y,
                                tag,
                                (col_w.saturating_sub(icon_w + stack_w)).min(3) as usize,
                                ink(MUTED),
                            );
                        }
                        None => {
                            buf.set_stringn(px + 1, y + 1, "··", 2, ink(TRACK));
                        }
                    }
                }
            }
        }
    } else {
        // The narrow dossier (58×18): a text list, no icons. Every weapon and
        // passive is still named; a rack longer than the card is counted in a
        // `+N more` line instead of being cut.
        let rows = usize::from(body_bottom.saturating_sub(inner.y + body_row));
        let n_w = world.player.loadout.weapons.len();
        let n_p = world.player.loadout.passives.len();
        let pass_line = usize::from(rows > 1 && n_p > 0);
        let list_rows = rows.saturating_sub(pass_line);
        let shown = n_w.min(if n_w > list_rows { list_rows.saturating_sub(1) } else { list_rows });
        for row in 0..shown as u16 {
            let y = inner.y + body_row + row;
            buf[(inner.x + left_w.saturating_sub(1), y)]
                .set_char('│')
                .set_fg(TRACK)
                .set_bg(PLATE);
            if row == 0 {
                buf.set_stringn(right_x, y, "BIOMETRICS", right_w as usize, heavy(accent));
            }
            let w = &world.player.loadout.weapons[row as usize];
            let mut gbuf = [0u8; 4];
            let g = weapon_glyph(w.kind).encode_utf8(&mut gbuf);
            let wcol = to_color(w.kind.color());
            let (gx, _) = buf.set_stringn(inner.x, y, g, 2, heavy(wcol));
            let (gx, _) = buf.set_stringn(gx + 1, y, w.name(), 13, ink(COAT));
            let pips = weapon_pips(w.level, w.kind.max_level());
            let pip_col = if w.is_max() { TRAIL_GOLD } else { wcol };
            let px = (inner.x + left_w).saturating_sub(7);
            if px > gx {
                buf.set_stringn(px, y, &pips, 5, ink(pip_col));
            }
        }
        if n_w == 0 {
            buf.set_stringn(inner.x, inner.y + body_row, "none", left_w as usize, ink(MUTED));
            buf.set_stringn(right_x, inner.y + body_row, "BIOMETRICS", right_w as usize, heavy(accent));
        } else if n_w > shown {
            let y = inner.y + body_row + shown as u16;
            buf.set_stringn(
                inner.x,
                y,
                format!("+{} more", n_w - shown),
                left_w as usize,
                ink(WARN_AMBER),
            );
        }
        if pass_line > 0 {
            let y = inner.y + body_row + (rows as u16 - 1);
            if y < body_bottom {
                buf.set_stringn(inner.x, y, "PASSIVES", left_w as usize, heavy(accent));
                let mut px = inner.x + 9;
                let mut counted = 0usize;
                for &(pk, stacks) in &world.player.loadout.passives {
                    let chip = format!("{}×{stacks} ", crate::render::hud::passive_short_tag(pk));
                    if px + chip.chars().count() as u16 + 3 > inner.x + inner.width {
                        break;
                    }
                    let capped = stacks >= pk.max_stacks();
                    buf.set_stringn(
                        px,
                        y,
                        &chip,
                        chip.chars().count(),
                        if capped { heavy(TRAIL_GOLD) } else { ink(COAT) },
                    );
                    px += chip.chars().count() as u16;
                    counted += 1;
                }
                if counted < n_p {
                    buf.set_stringn(px, y, format!("+{}", n_p - counted), left_w as usize, ink(WARN_AMBER));
                }
            }
        }
    }

    let st = &world.player.loadout.stats;
    let stats_rows: [(&str, String); 6] = [
        ("HP", format!("{:.0}/{:.0}", world.player.hp.max(0.0), world.player.max_hp())),
        // The build's multiplier times the character level's, then the
        // level's own share so the player can see what a level is worth.
        ("Damage", {
            let total = (world.player.damage_mult() * 100.0).round() as u32;
            let level = crate::game::player::level_damage_percent(world.player.level);
            let long = format!("{total}% · Lv +{level}%");
            if long.chars().count() as u16 + 10 < right_w {
                long
            } else {
                format!("{total}%")
            }
        }),
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

    // The nest: the permanent bonuses this run is playing with, kept apart from
    // the in-run passives the tray above shows (`the nest bought this` vs `this
    // floor gave me that`). A fresh profile prints nothing at all.
    let nest_line = world.player.loadout.meta.summary();
    if !nest_line.is_empty() {
        let y = inner.y + body_row + 6;
        if y < body_bottom {
            let (lx, _) = buf.set_stringn(right_x, y, "NEST ", 5, heavy(TRAIL_GOLD));
            buf.set_stringn(
                lx,
                y,
                &nest_line,
                (right_x + right_w).saturating_sub(lx) as usize,
                ink(TRAIL_GOLD),
            );
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

    // The controls legend stays on screen **while muted**: a muted player is
    // exactly the one who needs to find `M`, the volume keys and the active
    // trigger again. The audio state is a chip on the controls row instead of a
    // replacement for the whole legend (the review's "pause omits the active
    // trigger … mute replaces its controls legend with audio status").
    let legend = controls_src.pause_legend_lines(icon_dossier);
    let rows = if r.height >= 16 { 2 } else { 1 };
    for (i, line) in legend.iter().take(rows).enumerate() {
        let row = r.height.saturating_sub(2 + i as u16);
        center(f, r, row, label(line, MUTED));
    }
    let mute_label = controls_src.action_label(Action::Mute);
    let mute_chip = if muted {
        format!(" {mute_label} unmute ")
    } else {
        format!(" {mute_label} mute ")
    };
    // The clickable chip says just `Resume`: the legend above already names
    // `Esc/P`, and a duplicated label makes the chip's hit target ambiguous.
    // Options names its own live key (the same `Action::Options` the app opens
    // the screen with), so a rebind moves the text; so do the Nest and to-menu chips.
    let opts_key = controls_src.action_label(Action::Options);
    let opts = if icon_dossier {
        format!(" {opts_key} options ")
    } else {
        format!(" {opts_key} opts ")
    };
    let quit_key = controls_src.action_label(Action::Quit);
    let quit = if icon_dossier {
        format!(" {quit_key} to menu ")
    } else {
        format!(" {quit_key} menu ")
    };
    let nest_chip = format!(" {} nest ", controls_src.action_label(Action::Nest));
    let library_chip = format!(" {} library ", controls_src.action_label(Action::Library));
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (" Resume ", Some(Target::Resume)),
            (" ", None),
            (opts.as_str(), Some(Target::Options)),
            (" ", None),
            (library_chip.as_str(), Some(Target::Library)),
            (" ", None),
            (nest_chip.as_str(), Some(Target::Nest)),
            (" ", None),
            (mute_chip.as_str(), None),
            (" ", None),
            (quit.as_str(), Some(Target::Menu)),
        ],
    );
}

/// Expanded Incident Report end-of-run summary with score breakdown, combat
/// telemetry, final arsenal, boss commendation, and ranked records.
/// Falls back cleanly to [`draw_end`] on compact terminals.
#[allow(clippy::too_many_arguments)]
pub fn draw_end_summary(
    f: &mut Frame,
    area: Rect,
    summary: &RunSummary,
    new_best: bool,
    lore_line: &str,
    scores: &ScoreTable,
    controls_src: &Controls,
    hits: &mut HitMap,
) {
    if area.width < 68 || area.height < 23 {
        draw_end(
            f,
            area,
            summary.won,
            summary.final_score,
            summary.gold,
            summary.time,
            summary.level,
            summary.kills,
            new_best,
            lore_line,
            scores,
            hits,
            summary.sector,
            summary.theme,
            summary.practice,
            summary.nest.as_ref(),
            controls_src,
        );
        return;
    }

    let compact_v = area.height < 27;
    let panel_h: u16 = if compact_v { 22 } else { 27 };
    let (status, accent) = if summary.won && summary.practice {
        ("SECTOR CLEARED", Color::Rgb(169, 210, 186))
    } else if summary.won {
        ("YOU SURVIVED", Color::Rgb(169, 210, 186))
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
    if summary.console_used {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            " CONSOLE USED ",
            heavy(PLATE).bg(Color::Rgb(255, 120, 120)),
        ));
    }
    header_spans.push(Span::styled(format!("   GOLD {}", summary.gold), heavy(TRAIL_GOLD)));
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

    // The short plate has five rows under the column heads before the divider
    // (`y_best`); the tall one has six. The sixth metric used to be drawn on
    // the divider's row and then ruled over.
    let metric_n: usize = if compact_v { 5 } else { 6 };
    for row in 0..=metric_n as u16 {
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
    for (i, (k, v)) in metric_rows.iter().take(metric_n).enumerate() {
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

    // The nest: what this run banked. One line above the records table, so the
    // tally sits under the numbers it came from.
    // The tall plate has a spare row above the chips, so a tally too long for
    // one row is split into the breakdown and the sum there, and the records
    // table moves down a row. The short plate keeps one row.
    let mut y_table = y_table;
    if let Some(nest) = summary.nest.as_ref() {
        let style = if nest.voided {
            ink(WARN_AMBER)
        } else {
            heavy(TRAIL_GOLD)
        };
        let rows = if compact_v { 1 } else { 2 };
        let lines = nest_tally_lines(nest, inner.width as usize, rows);
        let first = y_table.saturating_sub(1);
        for (i, line) in lines.iter().enumerate() {
            f.render_widget(
                Line::styled(line.as_str(), style).centered(),
                band(inner, first + i as u16, 1),
            );
        }
        y_table += (lines.len() as u16).saturating_sub(1);
    }

    // Final Arsenal strip.
    if !summary.weapons.is_empty() {
        let mut wspans: Vec<Span<'_>> = Vec::with_capacity(summary.weapons.len() * 3 + 2);
        wspans.push(label("ARSENAL  ", accent));
        for (i, &(kind, lvl)) in summary.weapons.iter().enumerate() {
            if i > 0 {
                wspans.push(label("  ", MUTED));
            }
            let short = crate::render::hud::weapon_tag(kind, false);
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
    // Rows between the table head and the chips, less the spacer the unsplit
    // layout keeps above the chips (a split tally spends that spacer).
    let spacer = if y_table == y_arsenal + 2 { 3 } else { 2 };
    let max_rows = usize::from(r.height.saturating_sub(y_table + spacer)).min(5);
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

    let [restart_chip, nest_chip, menu_chip] = end_chips(controls_src);
    controls(
        f,
        band(r, r.height.saturating_sub(1), 1),
        hits,
        &[
            (restart_chip.as_str(), Some(Target::Restart)),
            ("   ", None),
            (nest_chip.as_str(), Some(Target::Nest)),
            ("   ", None),
            (menu_chip.as_str(), Some(Target::Menu)),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::{Binding, KeyBind};
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

    /// The title plate's whole frame as text plus its hit map, at a pinned size.
    fn title_frame(w: u16, h: u16, controls: &Controls) -> (String, HitMap) {
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let scores = crate::scores::ScoreTable::default();
        let rat: Vec<String> = Vec::new();
        terminal
            .draw(|f| {
                draw_title(
                    f,
                    f.area(),
                    Theme::Sewer,
                    0,
                    0,
                    true,
                    &scores,
                    &rat,
                    controls,
                    &mut hits,
                );
            })
            .unwrap();
        (screen_text(&terminal), hits)
    }

    /// The title legend printed `Q quit` twice (`… Q quit` on the plate's own
    /// legend line and again on the rail under it). Each action is named once.
    #[test]
    fn the_title_legend_names_each_action_once() {
        let controls = Controls::default();
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let (text, hits) = title_frame(w, h, &controls);
            assert!(
                hits.rect_for(Target::Story).is_some(),
                "{w}x{h}: the story lost its click target"
            );
            for needle in ["Q quit", "L story"] {
                let n = text.matches(needle).count();
                assert_eq!(n, 1, "{w}x{h}: {needle:?} printed {n} times:\n{text}");
            }
        }
    }

    #[test]
    fn the_title_rail_names_the_live_confirm_key() {
        let mut c = Controls::default();
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let (text, _) = title_frame(w, h, &c);
            let key = c.action_label(Action::Confirm).to_string();
            assert!(
                text.contains(&format!("{key} confirm")) || text.contains(&format!("{key}/click")),
                "{w}x{h}: the rail does not name {key:?}:\n{text}"
            );
        }
        let _ = c.bind_active_kbm(Action::Confirm, Binding::Key(KeyBind::Char('f')));
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let (text, _) = title_frame(w, h, &c);
            assert!(
                text.contains("F confirm") || text.contains("F/click"),
                "{w}x{h}: the rebound confirm key is not named:\n{text}"
            );
            assert!(
                !text.contains("Enter confirm") && !text.contains("Enter/click"),
                "{w}x{h}: a stale confirm key:\n{text}"
            );
        }
    }

    /// A rebind moves the title's chips with it, and the rail still names the
    /// action exactly once.
    #[test]
    fn the_title_quit_chip_follows_the_binding_and_stays_single() {
        let mut controls = Controls::default();
        let _ = controls.bind_active_kbm(Action::Quit, Binding::Key(KeyBind::Char('k')));
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let (text, _) = title_frame(w, h, &controls);
            assert_eq!(
                text.matches("K quit").count(),
                1,
                "{w}x{h}: the live quit key is not named once:\n{text}"
            );
            assert!(!text.contains("Q quit"), "{w}x{h}: a stale quit key:\n{text}");
        }
    }

    /// The level-up legend names only the number keys the plate answers: `1–3`
    /// on a full three-card screen, `1 pick` on a one-card one.
    #[test]
    fn the_level_up_legend_promises_only_the_cards_that_answer() {
        assert_eq!(pick_chip(3), "1–3 pick");
        assert_eq!(pick_chip(2), "1–2 pick");
        assert_eq!(pick_chip(1), "1 pick");
        assert_eq!(pick_chip(0), "1 pick");
        let lo = Loadout::new(WeaponKind::Gnaw);
        let upgrades = vec![Upgrade::Bandage];
        let desc: Vec<_> = upgrades.iter().map(|u| lo.describe(u)).collect();
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                draw_levelup(f, f.area(), 4, &desc, &upgrades, 0, false, "U", &mut hits);
            })
            .unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("1 pick"), "one card is one key:\n{text}");
        assert!(!text.contains("1–3"));
    }

    /// The four banking outcomes, built by `meta::bank` itself so the plates are
    /// checked against the real rule and never against retyped numbers.
    fn bank_cases() -> Vec<(&'static str, bool, bool, crate::meta::BankTally)> {
        use crate::meta::{bank, MetaProfile, RunEnd};
        let run = |floors, apex, won, practice, console_used| {
            let mut profile = MetaProfile {
                bank: 300,
                ..MetaProfile::default()
            };
            bank(
                &mut profile,
                &RunEnd {
                    purse: 41,
                    spent: 12,
                    floors,
                    apex,
                    won,
                    practice,
                    console_used,
                },
            )
        };
        vec![
            ("death", false, false, run(1, 2, false, false, false)),
            ("clear", true, false, run(3, 6, true, false, false)),
            ("practice", true, true, run(1, 1, true, true, false)),
            ("console", false, false, run(2, 3, false, false, true)),
        ]
    }

    fn assert_tally_is_true(text: &str, tally: &crate::meta::BankTally, at: &str) {
        if tally.voided {
            assert!(text.contains("nothing banked"), "{at}: no console line:\n{text}");
            assert!(!text.contains("bank now"), "{at}: a voided run shows a gain:\n{text}");
            return;
        }
        let gain = format!("+{}¢", tally.banked);
        let balance = format!("bank now {}¢", tally.total);
        assert!(text.contains(&gain), "{at}: no {gain:?}:\n{text}");
        assert!(text.contains(&balance), "{at}: no {balance:?}:\n{text}");
        if tally.halved {
            assert!(text.contains("practice: half"), "{at}: the half rate is not named:\n{text}");
        }
    }

    /// The compact incident report never printed the nest tally: it was gated
    /// on a content height the 21-row plate cannot have. It prints at every
    /// size now, for each banking outcome, with the chips still registered.
    #[test]
    fn the_compact_incident_report_prints_the_bank_tally() {
        let scores = ScoreTable::default();
        for (name, won, practice, tally) in bank_cases() {
            for (w, h) in [(57u16, 20u16), (57, 16), (66, 22), (67, 40)] {
                let mut hits = HitMap::default();
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                t.draw(|f| {
                    draw_end(
                        f,
                        f.area(),
                        won,
                        4200,
                        41,
                        312.0,
                        9,
                        180,
                        false,
                        "The floor keeps what it takes.",
                        &scores,
                        &mut hits,
                        1,
                        Theme::Sewer,
                        practice,
                        Some(&tally),
                        &Controls::default(),
                    );
                })
                .unwrap();
                let text = screen_text(&t);
                let at = format!("{name} at {w}x{h}");
                assert_tally_is_true(&text, &tally, &at);
                assert!(text.contains("INCIDENT REPORT"), "{at}: no caption");
                for target in [Target::Restart, Target::Nest, Target::Menu] {
                    let rect = hits
                        .rect_for(target)
                        .unwrap_or_else(|| panic!("{at}: {target:?} is not registered"));
                    assert!(rect.right() <= w && rect.bottom() <= h, "{at}: {target:?} off screen");
                }
            }
        }
        // The 57x20 plate has the row to say where a death's gold came from.
        let (_, won, practice, tally) = bank_cases().remove(0);
        let mut hits = HitMap::default();
        let mut t = Terminal::new(TestBackend::new(57, 20)).unwrap();
        t.draw(|f| {
            draw_end(
                f, f.area(), won, 4200, 41, 312.0, 9, 180, false, "Lore.", &scores, &mut hits, 1,
                Theme::Sewer, practice, Some(&tally), &Controls::default(),
            );
        })
        .unwrap();
        let text = screen_text(&t);
        let floors = format!("floors {}¢", tally.floor_bonus);
        assert!(text.contains(&floors), "57x20: no breakdown ({floors:?}):\n{text}");
        assert!(text.contains("RK    SCORE"), "57x20: the records head was pushed out:\n{text}");
    }

    fn summary_for(won: bool, practice: bool, tally: crate::meta::BankTally) -> RunSummary {
        RunSummary {
            won,
            theme: Theme::Sewer,
            combat_score: 3000,
            gold: 41,
            bonus_time: 600,
            bonus_level: 600,
            bonus_win: 0,
            final_score: 4200,
            time: 312.0,
            level: 9,
            kills: 180,
            damage_taken: 64.0,
            dashes_used: 17,
            heals_picked: 2,
            boss_slain: false,
            weapons: vec![(WeaponKind::Gnaw, 3)],
            damage_mult: 1.2,
            fire_rate_mult: 1.0,
            armor: 0.0,
            sector: 1,
            practice,
            console_used: tally.voided,
            nest: Some(tally),
        }
    }

    /// The expanded report prints a true tally for each outcome on both plates,
    /// and the tall plate (which has the row) keeps the breakdown.
    #[test]
    fn the_expanded_incident_report_prints_the_bank_tally() {
        let scores = ScoreTable::default();
        for (name, won, practice, tally) in bank_cases() {
            for (w, h) in [(68u16, 23u16), (80, 24), (90, 32), (120, 40)] {
                let summary = summary_for(won, practice, tally);
                let mut hits = HitMap::default();
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                t.draw(|f| {
                    draw_end_summary(
                        f, f.area(), &summary, false, "Lore.", &scores, &Controls::default(), &mut hits,
                    );
                })
                .unwrap();
                let text = screen_text(&t);
                let at = format!("{name} at {w}x{h}");
                assert_tally_is_true(&text, &tally, &at);
                if h >= 27 && !tally.voided {
                    let floors = format!("floors {}¢", tally.floor_bonus);
                    assert!(text.contains(&floors), "{at}: no breakdown ({floors:?}):\n{text}");
                }
                assert!(text.contains("RK    SCORE"), "{at}: no records head:\n{text}");
                for target in [Target::Restart, Target::Nest, Target::Menu] {
                    assert!(hits.rect_for(target).is_some(), "{at}: {target:?} lost");
                }
            }
        }
    }

    /// The short expanded plate has five metric rows above its divider. The
    /// sixth used to be drawn on the divider's row and ruled over; it is left
    /// to the tall plate now, and the five that fit are whole.
    #[test]
    fn the_short_report_drops_the_sixth_metric_instead_of_ruling_over_it() {
        let scores = ScoreTable::default();
        let (_, won, practice, tally) = bank_cases().remove(0);
        let summary = summary_for(won, practice, tally);
        for (w, h, sixth) in [(80u16, 24u16, false), (90, 32, true)] {
            let mut hits = HitMap::default();
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                draw_end_summary(
                    f, f.area(), &summary, false, "Lore.", &scores, &Controls::default(), &mut hits,
                );
            })
            .unwrap();
            let text = screen_text(&t);
            assert!(text.contains("Dmg Taken"), "{w}x{h}: the fifth metric is missing:\n{text}");
            assert_eq!(text.contains("Dashes / Dmg"), sixth, "{w}x{h}:\n{text}");
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| &*s.content).collect()
    }

    #[test]
    fn card_text_wraps_on_words_and_marks_a_cut() {
        assert_eq!(wrap_words("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap_words("abcdefghij kl", 4), vec!["abcd", "efgh", "ij", "kl"]);
        assert!(wrap_words("", 10).is_empty());
        let mut lines = wrap_words("alpha beta gamma delta", 10);
        clip_lines(&mut lines, 1, 10);
        assert_eq!(lines, vec!["alpha …"]);
        let mut whole = wrap_words("alpha beta", 10);
        clip_lines(&mut whole, 1, 10);
        assert_eq!(whole, vec!["alpha beta"], "a line that fits is not marked");

        let desc = "Bites the nearest thing for 12 damage every 0.8 s and chains twice";
        let flavor = "Teeth first, questions never, and certainly no apologies afterwards.";
        for (w, h) in [(20u16, 2u16), (30, 2), (44, 2), (44, 3), (58, 3), (58, 4)] {
            let lines = levelup_card_lines(desc, flavor, w, h);
            assert!(lines.len() <= usize::from(h), "{w}x{h}: {} rows", lines.len());
            let source: Vec<&str> = desc.split_whitespace().chain(flavor.split_whitespace()).collect();
            for line in &lines {
                let text = line_text(line);
                assert!(text.chars().count() <= usize::from(w), "{w}x{h}: {text:?} overflows");
                for word in text.split_whitespace() {
                    assert!(
                        word == "…" || source.contains(&word),
                        "{w}x{h}: {word:?} is a clipped word in {text:?}"
                    );
                }
            }
            // The stat line always starts the card.
            assert!(line_text(&lines[0]).starts_with("Bites"), "{w}x{h}");
        }
        // Short text keeps the old shape: stat line, then flavor.
        let lines = levelup_card_lines("+10% damage", "Sharper.", 44, 2);
        assert_eq!(lines.iter().map(line_text).collect::<Vec<_>>(), vec!["+10% damage", "Sharper."]);
        // The partner note: its own row on a tall card, joined on a short one
        // that has the width, dropped from a short one that has not.
        let noted = "+10% damage · Milk Teeth READY";
        let tall = levelup_card_lines(noted, "Sharper.", 44, 3);
        assert_eq!(
            tall.iter().map(line_text).collect::<Vec<_>>(),
            vec!["+10% damage", "Sharper.", "Milk Teeth READY"]
        );
        let short = levelup_card_lines(noted, "Sharper.", 44, 2);
        assert_eq!(line_text(&short[0]), noted);
        let narrow = levelup_card_lines(noted, "Sharper.", 20, 2);
        assert_eq!(line_text(&narrow[0]), "+10% damage");
        assert!(!narrow.iter().any(|l| line_text(l).contains("Milk")));
    }

    /// One, two and three cards: the AUTO-PICK chip is on screen, is its own
    /// click target clear of every card, and the legend names the Confirm key it
    /// is given. On the compact plate a short stack sits centred above the chip.
    #[test]
    fn levelup_lays_out_one_two_and_three_cards_with_the_auto_pick_chip() {
        let mut lo = Loadout::new(WeaponKind::Gnaw);
        lo.weapons.push(Weapon::new(WeaponKind::CheeseSpray));
        let all = [
            Upgrade::LevelWeapon(0),
            Upgrade::NewWeapon(WeaponKind::AcidSpit),
            Upgrade::Passive(PassiveKind::Magnet),
        ];
        for n in 1..=3usize {
            let upgrades = all[..n].to_vec();
            let desc: Vec<_> = upgrades.iter().map(|u| lo.describe(u)).collect();
            for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
                let at = format!("{n} cards at {w}x{h}");
                let mut hits = HitMap::default();
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                t.draw(|f| {
                    draw_levelup_with_loadout_keyed(
                        f, f.area(), 5, &desc, &upgrades, &lo, 0, false, "U", "F", &mut hits,
                    );
                })
                .unwrap();
                let buf = t.backend().buffer();
                let row_text = |y: u16| -> String { (0..w).map(|x| buf[(x, y)].symbol()).collect() };
                let chip = hits
                    .rect_for(Target::AutoPickToggle)
                    .unwrap_or_else(|| panic!("{at}: no auto-pick chip"));
                assert!(row_text(chip.y).contains("AUTO-PICK: OFF"), "{at}: {:?}", row_text(chip.y));
                assert_eq!(
                    hits.at(chip.x + chip.width / 2, chip.y),
                    Some(Target::AutoPickToggle),
                    "{at}: the chip is not what a click on it hits"
                );
                let cards: Vec<Rect> = (0..n)
                    .map(|i| {
                        hits.rect_for(Target::Upgrade(i))
                            .unwrap_or_else(|| panic!("{at}: card {i} has no rect"))
                    })
                    .collect();
                for (i, card) in cards.iter().enumerate() {
                    assert!(card.height >= 4, "{at}: card {i} is {card:?}");
                    assert!(card.bottom() <= chip.y, "{at}: card {i} covers the chip row");
                    assert_eq!(
                        hits.at(card.x + card.width / 2, card.y + card.height / 2),
                        Some(Target::Upgrade(i)),
                        "{at}"
                    );
                    if i > 0 {
                        assert!(cards[i - 1].bottom() <= card.y, "{at}: cards {i} overlap");
                    }
                }
                assert!(hits.rect_for(Target::Upgrade(n)).is_none(), "{at}: a phantom card");
                let text = screen_text(&t);
                assert!(text.contains("F equip"), "{at}: the Confirm key is not named");
                assert!(!text.contains("Enter equip"), "{at}: a stale Confirm key");
                assert!(text.contains(&pick_chip(n)), "{at}: wrong pick legend");
                if (w, h) == (57, 20) {
                    let caption = (0..h)
                        .find(|&y| row_text(y).contains("LEVEL UP"))
                        .expect("the plate's caption row");
                    let above = cards[0].y - (caption + 1);
                    let below = chip.y - cards[n - 1].bottom();
                    assert!(
                        above.abs_diff(below) <= 1,
                        "{at}: the stack is not centred ({above} rows above, {below} below)"
                    );
                }
            }
        }
        // The unkeyed entry point keeps the default key's name.
        let upgrades = all.to_vec();
        let desc: Vec<_> = upgrades.iter().map(|u| lo.describe(u)).collect();
        let mut hits = HitMap::default();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| {
            draw_levelup_with_loadout(f, f.area(), 5, &desc, &upgrades, &lo, 0, false, "U", &mut hits);
        })
        .unwrap();
        assert!(screen_text(&t).contains("Enter equip"));
    }

    /// The bare pause plate was eight rows tall, so its content was six rows
    /// and the legend on content row 6 was never drawn. It is nine now, and the
    /// volume keys on it are the live bindings, not a literal `[ ]` / `- =`.
    #[test]
    fn the_bare_pause_plate_prints_its_legend_from_the_live_bindings() {
        let mut c = Controls::default();
        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hits = HitMap::default();
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| draw_pause(f, f.area(), &c, &mut hits)).unwrap();
            let text = screen_text(&t);
            let music = format!(
                "{} {} music",
                c.action_label(Action::MusicDown),
                c.action_label(Action::MusicUp)
            );
            assert!(text.contains(&music), "{w}x{h}: no {music:?}:\n{text}");
            assert!(text.contains(" fx · "), "{w}x{h}: no effects keys:\n{text}");
            assert!(text.contains("dash · "), "{w}x{h}: no dash key:\n{text}");
            assert!(text.contains("library"), "{w}x{h}: no library key:\n{text}");
            for target in [Target::Resume, Target::Options, Target::Menu] {
                assert!(hits.rect_for(target).is_some(), "{w}x{h}: {target:?} lost");
            }
        }
        // A rebind moves the legend with it.
        let _ = c.bind_active_kbm(Action::MusicDown, Binding::Key(KeyBind::Char('j')));
        let mut hits = HitMap::default();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw_pause(f, f.area(), &c, &mut hits)).unwrap();
        let text = screen_text(&t);
        let music = format!(
            "{} {} music",
            c.action_label(Action::MusicDown),
            c.action_label(Action::MusicUp)
        );
        assert!(text.contains(&music), "the rebound key is not named:\n{text}");
        assert!(!text.contains("[ ] music"), "a stale volume key:\n{text}");

        // Too short for the legend: the controls strip is what survives.
        let mut hits = HitMap::default();
        let mut t = Terminal::new(TestBackend::new(50, 8)).unwrap();
        t.draw(|f| draw_pause(f, f.area(), &c, &mut hits)).unwrap();
        assert!(screen_text(&t).contains("resume"));
        assert!(hits.rect_for(Target::Resume).is_some());
    }

    /// A terminal too narrow for the compact plate falls back to the bare one
    /// *before* anything is painted: the fallback used to be drawn over the
    /// compact plate's frame, which showed as a second `RUN CONTROL` caption.
    #[test]
    fn the_compact_pause_fallback_paints_one_plate() {
        let w = World::new(Theme::Kitchen, 7);
        for (tw, th) in [(30u16, 20u16), (31, 24), (57, 10)] {
            let mut hits = HitMap::default();
            let mut t = Terminal::new(TestBackend::new(tw, th)).unwrap();
            t.draw(|f| {
                draw_pause_compact(f, f.area(), &w, false, &Controls::default(), &mut hits)
            })
            .unwrap();
            let text = screen_text(&t);
            assert_eq!(
                text.matches("RUN CONTROL").count(),
                1,
                "{tw}x{th}: two plates:\n{text}"
            );
            assert!(text.contains("PAUSED"), "{tw}x{th}:\n{text}");
        }
    }

    /// The next-rank pips share the card's top border with its title. A long
    /// name keeps every letter; the pips give way.
    #[test]
    fn levelup_pips_never_overprint_a_long_card_title() {
        let lo = Loadout::new(WeaponKind::Gnaw);
        let upgrades = vec![Upgrade::NewWeapon(WeaponKind::AcidSpit)];
        let long = "Longname ".repeat(6).trim_end().to_string();
        let desc = vec![(long.clone(), "Does a thing.".to_string(), "Flavor.".to_string())];
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let mut hits = HitMap::default();
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                draw_levelup_with_loadout(f, f.area(), 4, &desc, &upgrades, &lo, 0, false, "U", &mut hits);
            })
            .unwrap();
            let text = screen_text(&t);
            assert!(text.contains(&long), "{w}x{h}: the title was overprinted:\n{text}");
        }
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
                draw_levelup_with_loadout(f, f.area(), 4, &desc, &upgrades, &lo, 0, false, "U", &mut hits);
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
        wide.draw(|f| draw_pause_with_run(f, f.area(), &w, true, &Controls::default(), &mut hits))
            .unwrap();
        let wt = screen_text(&wide);
        assert!(wt.contains("ARSENAL"), "dossier arsenal missing");
        assert!(wt.contains("BIOMETRICS"), "dossier biometrics missing");
        // The mute state is a chip on the controls row, not a replacement for
        // the legend.
        assert!(wt.contains("M unmute"), "the way back out of mute is missing");
        assert!(
            wt.contains("WASD/mouse move"),
            "the controls legend vanished while muted"
        );
        assert!(wt.contains("[ ] music"), "the volume keys vanished while muted");
        assert!(wt.contains("Esc/P unpause"));

        // Compact fallback must not panic and keeps controls.
        hits.clear();
        let mut compact = Terminal::new(TestBackend::new(57, 20)).unwrap();
        compact
            .draw(|f| draw_pause_with_run(f, f.area(), &w, false, &Controls::default(), &mut hits))
            .unwrap();
        let ct = screen_text(&compact);
        assert!(ct.contains("PAUSED"));
        // The short plate names the same controls in its own wording.
        assert!(ct.contains("Esc/P unpause"));
        assert!(ct.contains("M mute"));
    }

    /// The pause rail counts down to the boss and, once the boss fight is on,
    /// counts the fight up instead of sitting at `00:00 left`.
    #[test]
    fn pause_shows_a_boss_fight_clock_instead_of_a_dead_countdown() {
        let mut w = World::new(Theme::Kitchen, 7);
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        w.elapsed = 65.0; // 900 s boss time → 13:55 to go
        terminal
            .draw(|f| draw_pause_with_run(f, f.area(), &w, true, &Controls::default(), &mut hits))
            .unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("13:55 to boss"), "no build-up clock: {text}");

        w.elapsed = crate::game::director::BOSS_TIME + 42.0;
        terminal
            .draw(|f| draw_pause_with_run(f, f.area(), &w, true, &Controls::default(), &mut hits))
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
            gold: 24,
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
            nest: Some(crate::meta::BankTally {
                raised: 240,
                spent: 30,
                purse: 210,
                floor_bonus: 75,
                apex_bonus: 18,
                win_bonus: 75,
                halved: false,
                voided: false,
                banked: 378,
                total: 900,
            }),
            console_used: false,
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
            draw_end_summary(f, f.area(), &summary, true, "Escaped.", &scores, &Controls::default(), &mut hits)
        })
        .unwrap();
        let bt = screen_text(&big);
        assert!(bt.contains("YOU SURVIVED"));
        assert!(bt.contains("SECTOR 3/4"), "the header names the sector reached");
        assert!(bt.contains("APEX NEUTRALIZED"));
        assert!(bt.contains("SCORE BREAKDOWN"));
        assert!(bt.contains("COMBAT METRICS"));
        assert!(bt.contains("NEW HIGH SCORE"));
        assert!(bt.contains("▸GN★"));

        // A practice clear is labelled, and its status line says so.
        let mut practice = summary.clone();
        practice.practice = true;
        practice.sector = 2;
        hits.clear();
        big.draw(|f| {
            draw_end_summary(f, f.area(), &practice, false, "Cleared.", &scores, &Controls::default(), &mut hits)
        })
        .unwrap();
        let pt = screen_text(&big);
        assert!(pt.contains("SECTOR CLEARED"), "practice clear status missing");
        assert!(pt.contains("SECTOR 2/4 PRACTICE"), "practice label missing");

        for (tw, th) in [(0, 0), (1, 1), (57, 20), (80, 16)] {
            hits.clear();
            let mut small = Terminal::new(TestBackend::new(tw, th)).unwrap();
            small
                .draw(|f| {
                    draw_end_summary(f, f.area(), &summary, true, "Escaped.", &scores, &Controls::default(), &mut hits)
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
                    draw_levelup_with_loadout(f, f.area(), 5, &desc, &upgrades, &lo, 1, false, "U", &mut hits);
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
                .draw(|f| draw_pause_with_run(f, f.area(), &w, false, &Controls::default(), &mut hits))
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

    /// The review's short-pause complaint: "the short pause plate also drops the
    /// inventory". The compact plate now carries the full loadout as icon rows,
    /// and the controls legend survives every size — muted or not.
    #[test]
    fn the_short_pause_plate_keeps_the_loadout_and_the_legend() {
        let mut w = World::new(Theme::Sewer, 7);
        // Five weapons: one more than a fresh profile's rack holds.
        w.player.loadout.set_caps(crate::game::loadout::SlotCaps::new(5, 5));
        for kind in [
            WeaponKind::CheeseSpray,
            WeaponKind::TailThrash,
            WeaponKind::AcidRain,
            WeaponKind::LaserEyes,
        ] {
            w.player.loadout.apply(&Upgrade::NewWeapon(kind));
        }
        for pk in [
            PassiveKind::MaxHp,
            PassiveKind::Armor,
            PassiveKind::SpringHeel,
        ] {
            w.player.loadout.apply(&Upgrade::Passive(pk));
        }
        for (tw, th) in [(57u16, 20u16), (50, 16)] {
            for muted in [false, true] {
                let mut hits = HitMap::default();
                let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
                terminal
                    .draw(|f| {
                        draw_pause_with_run(
                            f,
                            f.area(),
                            &w,
                            muted,
                            &{
                                let mut c = Controls::default();
                                c.set_lone_ctrl_hint(true);
                                c
                            },
                            &mut hits,
                        )
                    })
                    .unwrap();
                let text = screen_text(&terminal);
                let half_blocks = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .filter(|c| matches!(c.symbol(), "▀" | "▄"))
                    .count();
                assert!(
                    half_blocks >= 56,
                    "{tw}x{th} muted={muted}: the compact plate lost the loadout \
                     ({half_blocks} icon cells): {text}"
                );
                assert!(text.contains("PAUSED"), "{tw}x{th}: no banner");
                assert!(
                    text.contains("WASD") && text.contains("dash") && text.contains("actives"),
                    "{tw}x{th} muted={muted}: the controls legend is gone: {text}"
                );
                assert!(
                    text.contains("LMB dash") && text.contains("RMB actives"),
                    "{tw}x{th} muted={muted}: the pointer's two buttons are not named: {text}"
                );
                if muted {
                    assert!(
                        text.contains("M unmute"),
                        "{tw}x{th}: the mute chip does not offer the way back: {text}"
                    );
                } else {
                    assert!(text.contains("M mute"), "{tw}x{th}: no mute key: {text}");
                }
            }
        }
    }

    /// A world carrying a **full 8 + 8 rack** in the state the review will
    /// photograph: two charged active weapons, three evolved, every passive at
    /// a stack, gold in the purse, running pocket finds and the dash on
    /// cooldown.
    fn full_rack_world() -> World {
        use crate::game::loadout::{SlotCaps, Upgrade};
        let mut w = World::new(Theme::Sewer, 7);
        w.player.loadout.set_caps(SlotCaps::FULL);
        w.player.loadout.weapons.clear();
        w.player.loadout.passives.clear();
        let kinds = [
            WeaponKind::TailWhip,
            WeaponKind::TailThrash,
            WeaponKind::AcidRain,
            WeaponKind::Gnaw,
            WeaponKind::LaserEyes,
            WeaponKind::CheeseSpray,
            WeaponKind::Hantavirus,
            WeaponKind::SonicSqueak,
        ];
        for (i, kind) in kinds.iter().enumerate() {
            let mut weapon = Weapon::new(*kind);
            weapon.level = match i {
                0 => kind.max_level(),
                3 => kind.max_level(),
                5 => kind.max_level(),
                1 => 4,
                _ => 2,
            };
            // Three evolved, two charged actives.
            weapon.evolved = i == 0 || i == 3 || i == 5;
            if kind.is_active() && i % 2 == 1 {
                weapon.timer = 0.0;
            }
            w.player.loadout.weapons.push(weapon);
        }
        for kind in PassiveKind::all().into_iter().take(8) {
            w.player.loadout.apply(&Upgrade::Passive(kind));
            w.player.loadout.apply(&Upgrade::Passive(kind));
        }
        w.player.buffs.shield = 12.0;
        w.player.buffs.socks = 6.0;
        w.player.buffs.decoy = 4.0;
        w.player.buffs.slow_bullets = 3.0;
        w.gold = 47;
        w.score = 3420;
        w.kills = 248;
        w.player.level = 9;
        w.elapsed = 65.0;
        w.player.hp = 84.0;
        w.player.dash_cd = 0.9;
        w
    }

    /// The **pause** screens with a full 8 + 8 rack: the icon dossier on the
    /// two sizes that get it, and the short plate at 57×20, where each band is
    /// followed by its tag row so every kind is still named. Nothing clips and
    /// nothing panics.
    #[test]
    fn the_pause_screens_lay_out_a_full_rack_at_57_80_and_120() {
        let w = full_rack_world();
        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hits = HitMap::default();
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| {
                    draw_pause_with_run(f, f.area(), &w, false, &crate::controls::Controls::default(), &mut hits)
                })
                .unwrap();
            let text = screen_text(&terminal);
            assert!(text.contains("PAUSED"), "{tw}x{th}: no banner: {text}");
            if tw >= 80 {
                // The dossier: both rack headers, the biometrics and the
                // evolution marks of the three chest weapons.
                assert!(text.contains("ARSENAL"), "{tw}x{th}: no arsenal: {text}");
                assert!(text.contains("PASSIVES"), "{tw}x{th}: no passive tray: {text}");
                assert!(text.contains("BIOMETRICS"), "{tw}x{th}: no biometrics: {text}");
                assert!(text.contains("EVO"), "{tw}x{th}: evolved weapons unmarked: {text}");
                // Weapon names, not only icons: the two tails must both read.
                assert!(text.contains("Tail"), "{tw}x{th}: no weapon names: {text}");
            } else {
                // The short plate carries both racks as icon bands plus their
                // tag rows: the icons must actually be there.
                let icons = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .filter(|c| matches!(c.symbol(), "▀" | "▄"))
                    .count();
                assert!(
                    icons >= 32,
                    "{tw}x{th}: the short plate lost the racks ({icons} icon cells): {text}"
                );
                assert!(text.contains("Lv 9"), "{tw}x{th}: no clock line: {text}");
            }
        }
    }

    /// The level-up cards' ARSENAL strip with a full 8 + 8 rack: it measures
    /// itself and ends in a `+N` marker rather than clipping a name, and a
    /// small rack never grows one.
    #[test]
    fn the_arsenal_strip_measures_a_full_rack_and_marks_the_rest() {
        use crate::game::loadout::Upgrade;
        let w = full_rack_world();
        let choices: Vec<(String, String, String)> = vec![
            ("Card".to_string(), "desc".to_string(), "flavor".to_string()),
            ("Card".to_string(), "desc".to_string(), "flavor".to_string()),
            ("Card".to_string(), "desc".to_string(), "flavor".to_string()),
        ];
        let upgrades = vec![
            Upgrade::LevelWeapon(0),
            Upgrade::Passive(PassiveKind::Armor),
            Upgrade::Purse,
        ];
        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hits = HitMap::default();
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| {
                    draw_levelup_with_loadout(
                        f,
                        f.area(),
                        5,
                        &choices,
                        &upgrades,
                        &w.player.loadout,
                        1,
                        false,
                        "U",
                        &mut hits,
                    )
                })
                .unwrap();
            let text = screen_text(&terminal);
            assert!(text.contains("LEVEL UP"), "{tw}x{th}: no level-up card");
            if tw >= 60 {
                assert!(text.contains("ARSENAL"), "{tw}x{th}: strip missing: {text}");
                // 8 + 8 chips are more than one strip can carry: the remainder
                // is counted.
                assert!(
                    text.contains('+'),
                    "{tw}x{th}: a full rack was clipped without a marker: {text}"
                );
            }
        }
        // A small build fits with no marker at all.
        let mut small = World::new(Theme::Sewer, 7);
        small.player.loadout.apply(&Upgrade::NewWeapon(WeaponKind::AcidSpit));
        small.player.loadout.apply(&Upgrade::Passive(PassiveKind::Armor));
        let mut hits = HitMap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                draw_levelup_with_loadout(
                    f,
                    f.area(),
                    2,
                    &choices,
                    &upgrades,
                    &small.player.loadout,
                    0,
                    false,
                    "U",
                    &mut hits,
                )
            })
            .unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("ARSENAL"), "no strip: {text}");
        assert!(!text.contains("+2"), "a two-item rack needs no marker: {text}");
    }

    /// The chest card with a full 8 + 8 rack: the reward keeps the card and the
    /// loadout strip under it measures itself (a `+N` marker past the edge).
    #[test]
    fn the_chest_card_lays_out_a_full_rack_at_57_80_and_120() {
        use crate::game::loadout::ChestReward;
        use crate::game::weapon::WeaponKind;
        let w = full_rack_world();
        let reward = ChestReward::WeaponLevel {
            kind: WeaponKind::Gnaw,
            level: 4,
        };
        for (tw, th) in [(57u16, 20u16), (80, 24), (120, 40)] {
            let mut hits = HitMap::default();
            let mut terminal = Terminal::new(TestBackend::new(tw, th)).unwrap();
            terminal
                .draw(|f| draw_chest(f, f.area(), reward, &w.player.loadout, &mut hits))
                .unwrap();
            let text = screen_text(&terminal);
            assert!(text.contains("CHEST OPENED"), "{tw}x{th}: no chest card: {text}");
            assert!(text.contains("ARSENAL"), "{tw}x{th}: no loadout strip: {text}");
            assert!(
                hits.rect_for(Target::Continue).is_some(),
                "{tw}x{th}: no continue target"
            );
        }
    }
    #[test]
    fn the_levelup_header_states_the_levels_damage() {
        // Four percent a level since batch twelve (the level pace quartered):
        // level 12 is worth +44 %, level 14 +52 %.
        assert_eq!(
            levelup_header(12),
            " LEVEL UP   /   LV 12   /   +44% WEAPON DAMAGE "
        );
        assert!(levelup_header(2).contains("+4% WEAPON DAMAGE"));
        assert!(levelup_header(1).contains("+0% WEAPON DAMAGE"));
        // The header reads the live constant, never `level - 1`.
        assert!(levelup_header(14).contains("+52% WEAPON DAMAGE"));
        // The widest header still fits the narrowest plate (57 columns less
        // the frame).
        assert!(levelup_header(199).chars().count() <= 55);
        assert!(levelup_header(1000).chars().count() <= 55);
    }

}
