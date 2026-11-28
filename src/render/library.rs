//! **The Library screen** (`game::library`): the facility's specimen index.
//!
//! Four tabs across the top with their discovery counts, a scrolling list on
//! the left (group headers, `???` for anything not yet met, a `NEW` tag until
//! it has been read) and a detail card on the right: the entry's **real art**
//! drawn from the same functions the game draws it with, its name, its dossier
//! and the facts a player wants — read from the game's own tables, so a balance
//! pass moves the card.
//!
//! ## Art
//!
//! The card art is drawn into a side [`PixelBuffer`] (the same half-block grid
//! the playfield uses) and blitted with [`PixelBuffer::render_to`]:
//!
//! ## The animated plate
//!
//! A card with a runnable subject is drawn from the **real game**: the selected,
//! unlocked entry owns a [`scene::LibraryScene`] (a sandbox `World` running the
//! real schedule over a hand-built floor patch) and its art plate is that
//! world's own `draw` output. The scene is built only after the cursor has
//! rested on the card for [`scene::REST_SECS`], steps at a fixed dt from the
//! screen's clock, and is torn down when the selection, the tab or the screen
//! moves. `RATRUN_LIBRARY_STILL=1` freezes every card to the still pose below;
//! a locked (`???`) or half-catalogued card never builds one at all.
//!
//! The still pose — drawn for a card whose scene does not exist (locked, or
//! before the cursor has settled) and for every [`scene::Plan::Still`] entry —
//! is:
//!
//! | entry | drawn by |
//! |---|---|
//! | enemy | [`art_enemy::draw_enemy_styled`] at ~3× radius, posed through its walk/tell |
//! | miniboss | [`art_miniboss::draw`] + its telegraph |
//! | boss | the boss's own `draw` (and `_ground`) at a posed state |
//! | weapon / passive / passive icon | the authored [`art_icons`] sprite, scaled |
//! | pickup / coin / chest | [`art_floor`]'s own pickup art |
//! | trap / feature | [`art_traps`]' view structs (the same ones the generator fills) |
//! | hazard | [`art_floor::draw_hazards`] over a cache |
//! | prop | [`art_props::draw_prop`] |
//! | floor | [`art_floor::draw_floor`] over a cached miniature level |
//!
//! The miniature levels and the hazard caches are built **once** (lazily, on
//! the first frame that needs them) and then mutated in place, so a card costs
//! no allocation beyond the strings every menu already formats.
//!
//! ## Sizes
//!
//! Three layouts, matching the three sizes the app tests pin: at 57 × 20 the
//! list takes the whole panel and Enter opens the card over it (`Esc` returns
//! to the list), at 80 × 24 and 120 × 40 the list and the card sit side by
//! side.
//!
//! ## The card's height budget
//!
//! The dossier is the reason the card exists, so it is **measured first**: the
//! note is wrapped on words at the card's width and the art plate takes only
//! the rows that are left (never fewer than [`MIN_ART_H`]). A short note buys a
//! tall plate; a long one shrinks it. When even that is not enough the note is
//! cut on a whole sentence, else on a whole word followed by the safe `…`
//! glyph — never mid-word, never mid-clause.

use rand::SeedableRng;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::game::art_icons;
use crate::game::collision::SizeClass;
use crate::game::ecs::systems::boss::BossKind;
use crate::game::ecs::systems::miniboss::{self, Miniboss, MinibossState};
use crate::game::enemy::{Enemy, EnemyKind};
use crate::game::gem::{Gem, GemTier, PickupKind};
use crate::game::hazards;
use crate::game::level::{
    theme_props, Level, LevelObjects, Palette, Pit, PitKind, Prop, Rectf, Theme, TileRect,
    TrailerDeck,
};
use crate::game::library::{self, Fact, Key, Library, ObjectKind, Row, Tab};
use crate::game::weapon::WeaponKind;
use crate::math::Vec2;
use crate::render::framebuffer::PixelBuffer;
use crate::render::menu::{self, HitMap, Target};
use crate::render::palette::{mix, to_color, Rgb, COAT, EDGE, MUTED, PLATE, TRACK};

use crate::game::art_boss_broom::{self, BroomBossView};
use crate::game::art_boss_cat::{self, BossCatView};
use crate::game::art_boss_hawk::{self, HawkView};
use crate::game::art_boss_magpie::{self, MagpieView};
use crate::game::art_boss_mangle::{self, MangleView};
use crate::game::art_boss_mower::{self, MowerView};
use crate::game::art_boss_night_edition::{self, NightEditionView};
use crate::game::art_enemy;
use crate::game::art_floor;
use crate::game::art_miniboss;
use crate::game::art_props;
use crate::game::art_traps::{self, MousetrapView, RatHoleView, StickyTrapView};

#[allow(clippy::disallowed_methods)] // render-only: platform maths is fine
pub mod scene;

const VIAL: Color = Color::Rgb(196, 202, 214);
const ROW_SEL_BG: Color = Color::Rgb(26, 32, 34);

/// The most rows the dossier will take: past this the note is a page, not a
/// card, and the art has already given the plate every row it can.
const DOSSIER_MAX_LINES: usize = 9;
/// The art plate never shrinks below this many cells: a card with no specimen
/// is just a facts table.
const MIN_ART_H: u16 = 5;
/// The narrow (57 × 20) card's preferred art height.
const NARROW_ART_H: u16 = 6;
/// Rows the player's own record line claims at the foot of the card.
const STATS_H: u16 = 2;

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

fn put(f: &mut Frame, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 {
        return;
    }
    f.buffer_mut()
        .set_stringn(x, y, text, width as usize, style);
}

fn fill_bg(f: &mut Frame, rect: Rect, bg: Color) {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            f.buffer_mut()[(x, y)].set_bg(bg);
        }
    }
}

/// What the screen keeps between frames: tab/cursor/scroll, the art clock, and
/// the lazily built card scenes.
pub struct LibraryUi {
    pub tab: Tab,
    /// Selected entry **within its tab** (0-based, in row order).
    pub cursor: usize,
    /// Top visible display line (group headers count as lines).
    pub scroll: usize,
    /// Narrow terminals: the card replaces the list.
    pub card_open: bool,
    /// The still pose's own clock. It only advances while a card's scene runs
    /// (the scene draws its own picture), so a still plate never moves and the
    /// screen can go idle on it.
    pub anim: f32,
    /// `RATRUN_LIBRARY_STILL=1`: every card stays on its still pose.
    pub still: bool,
    /// The selected card's running scene, if any.
    scene: Option<scene::LibraryScene>,
    /// The key the cursor is resting on, and for how long.
    resting: Option<Key>,
    rest: f32,
    art: PixelBuffer,
    scenes: Scenes,
}

impl Default for LibraryUi {
    fn default() -> Self {
        LibraryUi {
            tab: Tab::Bestiary,
            cursor: 0,
            scroll: 0,
            card_open: false,
            anim: 0.0,
            still: scene::still_from_env(),
            scene: None,
            resting: None,
            rest: 0.0,
            art: PixelBuffer::new(1, 1),
            scenes: Scenes::default(),
        }
    }
}

impl LibraryUi {
    /// Switch tab: the cursor and scroll restart at the top of the new one.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.cursor = 0;
        self.scroll = 0;
        self.stop();
    }

    /// Tear the running scene down (the screen closed, the tab changed).
    pub fn stop(&mut self) {
        self.scene = None;
        self.resting = None;
        self.rest = 0.0;
        self.anim = 0.0;
    }

    /// Has the card on screen got a scene that has moved on since the last
    /// draw? The App treats this as an animation clock: it repaints at once, so
    /// an animating card holds the Library at 30 Hz.
    pub fn frame_due(&self) -> bool {
        self.scene.as_ref().is_some_and(|s| s.due())
    }

    /// The running scene's key, for tests and captures.
    pub fn scene_key(&self) -> Option<Key> {
        self.scene.as_ref().map(|s| s.key())
    }

    /// The running scene, for tests and captures.
    pub fn running_scene(&mut self) -> Option<&mut scene::LibraryScene> {
        self.scene.as_mut()
    }

    /// Drive the selected card's animation. `key` is the selection and `live`
    /// says whether its card is unlocked, catalogued and on screen; anything
    /// else stops the scene, and a card that stays selected long enough for
    /// [`scene::REST_SECS`] starts one.
    pub fn tick_card(&mut self, dt: f32, key: Option<Key>, live: bool) {
        let want = if self.still { None } else { key.filter(|_| live) };
        // A different card: the scene belongs to the old one.
        if self
            .scene
            .as_ref()
            .is_some_and(|running| Some(running.key()) != want)
        {
            self.scene = None;
            self.anim = 0.0;
        }
        if want != self.resting {
            self.resting = want;
            self.rest = 0.0;
        }
        let Some(key) = want else {
            return;
        };
        self.rest += dt.max(0.0);
        if self.scene.is_none() {
            if self.rest < scene::REST_SECS {
                return;
            }
            self.scene = scene::LibraryScene::build(key);
        }
        if let Some(running) = self.scene.as_mut() {
            running.advance(dt);
            self.anim += dt.max(0.0);
        }
    }

    /// Step the selected entry by `step`, clamped to the tab's entries.
    pub fn step_cursor(&mut self, step: isize, entries: usize) {
        if entries == 0 {
            return;
        }
        let next = (self.cursor as isize + step).clamp(0, entries as isize - 1);
        self.cursor = next as usize;
    }

    /// The key under the cursor in the current tab, if it has one.
    pub fn selected_key(&self, rows: &[Row]) -> Option<Key> {
        rows.iter()
            .filter(|r| r.tab == self.tab)
            .nth(self.cursor)
            .map(|r| r.key)
    }

    /// Select one entry **by key** and open its card. Returns `false` when the
    /// key is not in the current tab.
    ///
    /// Anything that names an entry from outside (a capture, a deep link, a
    /// test) must go through this: a row index is positional and moves whenever
    /// a group gains an entry — the `library-boss` capture's stale index landed
    /// on a named miniboss instead of the floor boss it meant.
    pub fn select_key(&mut self, rows: &[Row], key: Key) -> bool {
        let Some(index) = rows
            .iter()
            .filter(|r| r.tab == self.tab)
            .position(|r| r.key == key)
        else {
            return false;
        };
        self.cursor = index;
        self.scroll = 0;
        self.card_open = true;
        true
    }
}

/// The measurement the App needs after a frame: total display lines, how many
/// fit, and whether the narrow layout is in force.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawInfo {
    pub lines: usize,
    pub visible: usize,
    pub narrow: bool,
}

/// Lazily built card scenes (miniature levels and hazard caches).
#[derive(Default)]
struct Scenes {
    levels: [Option<Level>; Theme::Outfall as usize + 1],
    steam: Option<LevelObjects>,
    ink: Option<LevelObjects>,
}

/// Draw the whole screen. `rows` is the registry (`game::library::all_rows`).
pub fn draw(
    f: &mut Frame,
    area: Rect,
    rows: &[Row],
    lib: &Library,
    accent_rgb: Rgb,
    ui: &mut LibraryUi,
    hits: &mut HitMap,
) -> DrawInfo {
    let accent = to_color(accent_rgb);
    let w = area.width.saturating_sub(2).min(116);
    let h = area.height.saturating_sub(2).min(42);
    let panel = menu::centered_rect(w, h, area);
    let inner = menu::panel(f, panel, " ⌸ LIBRARY   /   SPECIMEN INDEX ", accent);
    let mut info = DrawInfo::default();
    if inner.width < 20 || inner.height < 8 {
        return info;
    }

    draw_tabs(f, inner, rows, lib, ui, accent, hits);

    // Rule under the tab strip.
    let rule = "─".repeat(usize::from(inner.width));
    f.render_widget(Line::styled(rule.as_str(), ink(TRACK)), band(inner, 1, 1));

    let body = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(4),
    );
    let narrow = inner.width < 70;
    info.narrow = narrow;

    let entries = entries_in(rows, ui.tab);
    if entries == 0 {
        return info;
    }
    ui.cursor = ui.cursor.min(entries - 1);
    let (lines, _selected_line) = measure(rows, ui.tab, ui.cursor);

    if narrow {
        if ui.card_open {
            draw_card(f, body, rows, lib, ui, accent, accent_rgb, true, hits);
        } else {
            info.visible = draw_list(f, body, rows, lib, ui, accent, &mut info, hits);
        }
    } else {
        let list_w = (inner.width * 2 / 5).clamp(26, 40);
        let list = Rect::new(body.x, body.y, list_w, body.height);
        let card = Rect::new(
            body.x + list_w + 1,
            body.y,
            body.width.saturating_sub(list_w + 1),
            body.height,
        );
        info.visible = draw_list(f, list, rows, lib, ui, accent, &mut info, hits);
        // Divider.
        for y in card.y..card.bottom() {
            f.buffer_mut()[(card.x - 1, y)]
                .set_char('│')
                .set_fg(TRACK)
                .set_bg(PLATE);
        }
        draw_card(f, card, rows, lib, ui, accent, accent_rgb, false, hits);
    }

    info.lines = lines;
    if info.visible > 0 {
        let max_scroll = lines.saturating_sub(info.visible);
        ui.scroll = ui.scroll.min(max_scroll);
    }

    // Footer: the legend and the way back.
    let rule_y = inner.y + inner.height - 2;
    f.render_widget(
        Line::styled("─".repeat(usize::from(inner.width)).as_str(), ink(TRACK)),
        band(inner, rule_y, 1),
    );
    let row = band(inner, inner.height - 1, 1);
    // Every key this list accepts is named here (and each of them works: the
    // screen's own keys are raw, not bindings — see the report on `Action`).
    let legend = if info.narrow {
        "↑↓ browse · Enter card · ←→/QE tab · Esc back"
    } else {
        "↑↓/WS/AD browse · ←→/Tab/QE tab · wheel scroll · Esc back"
    };
    put(f, row.x, row.y, row.width, legend, ink(MUTED));
    let back = if info.narrow && ui.card_open {
        "‹ list "
    } else {
        " back "
    };
    let bx = row.right().saturating_sub(back.chars().count() as u16 + 1);
    if bx > row.x + legend.chars().count() as u16 {
        put(
            f,
            bx,
            row.y,
            row.right() - bx,
            back,
            heavy(accent).bg(TRACK),
        );
        hits.add(
            Rect::new(bx, row.y, back.chars().count() as u16, 1),
            if info.narrow && ui.card_open {
                Target::LibraryCard
            } else {
                Target::Back
            },
        );
    }
    info
}

/// The tab strip with each tab's discovery count (`BESTIARY 9/22`).
fn draw_tabs(
    f: &mut Frame,
    inner: Rect,
    rows: &[Row],
    lib: &Library,
    ui: &mut LibraryUi,
    accent: Color,
    hits: &mut HitMap,
) {
    let mut x = inner.x;
    let row = inner.y;
    for tab in Tab::ALL {
        let (seen, total) = lib.counts(rows, tab);
        let selected = tab == ui.tab;
        let text = format!("{} {}/{}", tab.title(), seen, total);
        let label = if selected {
            format!("[{text}]")
        } else {
            format!(" {text} ")
        };
        let w = label.chars().count() as u16;
        if x + w > inner.right() {
            break;
        }
        let style = if selected {
            heavy(PLATE).bg(to_color(mix(theme_rgb(tab), (235, 216, 214), 0.16)))
        } else {
            ink(MUTED)
        };
        put(f, x, row, inner.right().saturating_sub(x), &label, style);
        hits.add(Rect::new(x, row, w, 1), Target::LibraryTab(tab.index()));
        x += w + 1;
    }
    let _ = accent;
}

/// A per-tab material tint for the selected bay.
fn theme_rgb(tab: Tab) -> Rgb {
    match tab {
        Tab::Bestiary => (150, 190, 176),
        Tab::Arsenal => (222, 178, 108),
        Tab::FieldGuide => (152, 176, 214),
        Tab::Floors => (186, 182, 168),
    }
}

fn entries_in(rows: &[Row], tab: Tab) -> usize {
    rows.iter().filter(|r| r.tab == tab).count()
}

/// Walk the tab's display lines (group headers first), calling `f(line,
/// entry_index, row)`. `entry_index` is `None` for a header line.
fn walk_lines(rows: &[Row], tab: Tab, mut f: impl FnMut(usize, Option<usize>, &Row)) {
    let mut line = 0usize;
    let mut group = "";
    for (entry, row) in rows.iter().filter(|r| r.tab == tab).enumerate() {
        if row.group != group {
            f(line, None, row);
            line += 1;
            group = row.group;
        }
        f(line, Some(entry), row);
        line += 1;
    }
}

/// Total display lines and the line the selected entry sits on.
fn measure(rows: &[Row], tab: Tab, cursor: usize) -> (usize, usize) {
    let mut total = 0;
    let mut selected = 0;
    walk_lines(rows, tab, |line, entry, _| {
        total = line + 1;
        if entry == Some(cursor) {
            selected = line;
        }
    });
    (total, selected)
}

/// The scrolling list. Returns the number of rows it could show.
#[allow(clippy::too_many_arguments)]
fn draw_list(
    f: &mut Frame,
    area: Rect,
    rows: &[Row],
    lib: &Library,
    ui: &mut LibraryUi,
    accent: Color,
    info: &mut DrawInfo,
    hits: &mut HitMap,
) -> usize {
    // Keep the cursor's line inside the window before drawing.
    let (lines, selected) = measure(rows, ui.tab, ui.cursor);
    let visible = area.height as usize;
    info.visible = visible;
    info.lines = lines;
    if selected < ui.scroll {
        ui.scroll = selected;
    } else if selected >= ui.scroll + visible {
        ui.scroll = selected + 1 - visible;
    }
    let scroll = ui.scroll;

    let mut drawn = 0usize;
    walk_lines(rows, ui.tab, |line, entry, row| {
        if line < scroll || line >= scroll + visible {
            return;
        }
        let y = area.y + (line - scroll) as u16;
        let r = Rect::new(area.x, y, area.width, 1);
        match entry {
            None => {
                // Group header: `── ORDINARY ─────`.
                let head = format!("── {} ", row.group);
                let pad = "─".repeat(usize::from(area.width).saturating_sub(head.chars().count()));
                put(f, r.x, y, r.width, &format!("{head}{pad}"), ink(EDGE));
            }
            Some(idx) => {
                drawn += 1;
                let selected = idx == ui.cursor;
                let seen = lib.is_seen(row.key);
                let full = catalogued(lib, row.key);
                if selected {
                    fill_bg(f, r, ROW_SEL_BG);
                }
                let marker = if selected { "▸ " } else { "  " };
                let name = if seen { row.name } else { "???" };
                let style = if !seen {
                    ink(MUTED)
                } else if full || matches!(row.key, Key::Enemy(_) | Key::Floor(_) | Key::Object(_))
                {
                    if selected {
                        heavy(accent)
                    } else {
                        ink(COAT)
                    }
                } else if selected {
                    heavy(accent)
                } else {
                    ink(VIAL)
                };
                let mut used = 0u16;
                put(f, r.x, y, r.width, marker, ink(MUTED));
                used += 2;
                let name_w = r
                    .width
                    .saturating_sub(used + if lib.is_new(row.key) { 5 } else { 0 });
                put(f, r.x + used, y, name_w, name, style);
                used += row.name.chars().count().min(name_w as usize) as u16;
                if lib.is_new(row.key) && seen {
                    let x = (r.x + used + 1).min(r.right().saturating_sub(4));
                    put(f, x, y, 4, "NEW", heavy(to_color((232, 198, 96))));
                }
                hits.add(r, Target::LibraryRow(idx));
            }
        }
    });
    drawn
}

/// True when the entry counts as fully catalogued (owned, met, seen). A weapon
/// or passive only *offered* on a card is a silhouette with a name.
///
/// This is also the animation gate: a plate that is silhouetted, or a `???`
/// card, never runs a scene (`App::tick_library_card`), so the art a player is
/// shown is never louder than the entry they have earned.
pub fn catalogued(lib: &Library, key: Key) -> bool {
    if !lib.is_seen(key) {
        return false;
    }
    match key {
        Key::Weapon(_) | Key::Passive(_) => lib.record(key).owned > 0,
        Key::Evolution(_) => lib.record(key).evolved > 0 || lib.record(key).owned > 0,
        _ => true,
    }
}

/// The real art for one entry, into `art`.
fn draw_art(art: &mut PixelBuffer, scenes: &mut Scenes, key: Key, accent: Rgb, t: f32) {
    let theme = library::theme_of(key);
    let centre = Vec2::new(art.w as f32 * 0.5, art.h as f32 * 0.52);
    // Every card starts on its own floor so the art has something to sit on.
    paint_floor(art, &Palette::for_theme(theme), t);
    let cam = Vec2::ZERO;
    match key {
        Key::Enemy(kind) => {
            let mut rng = rand::rngs::StdRng::seed_from_u64(0x1B);
            let mut e = Enemy::spawn(kind, centre, 1.0, &mut rng);
            e.radius = kind.radius() * 3.2;
            e.anim = t * 7.0;
            pose_enemy(&mut e, t);
            art_enemy::draw_enemy_styled(art, &e, cam, accent, false);
            art_enemy::draw_w3_tells(art, &e, cam, accent);
        }
        Key::Object(ObjectKind::Elite) => {
            let mut rng = rand::rngs::StdRng::seed_from_u64(0x1B);
            let mut e = Enemy::spawn(EnemyKind::Brute, centre, 1.0, &mut rng);
            e.radius = EnemyKind::Brute.radius() * 2.4;
            e.anim = t * 4.0;
            art_enemy::draw_enemy_styled(art, &e, cam, accent, true);
        }
        Key::Miniboss(kind) => {
            let mut rng = rand::rngs::StdRng::seed_from_u64(0x2B);
            let mut e = Enemy::spawn(EnemyKind::Brute, centre, 1.0, &mut rng);
            e.radius = kind.body_radius() * 3.0;
            e.named = true;
            e.anim = t * 4.0;
            let mut mb = Miniboss::new(kind, 0);
            mb.state = MinibossState::Tell;
            mb.timer = (t.rem_euclid(2.4) / 2.4) * miniboss::TELL_TIME;
            mb.origin = centre;
            mb.aim = Vec2::new(1.0, 0.0);
            mb.stamp_centers = [
                centre + Vec2::new(16.0, 0.0),
                centre + Vec2::new(32.0, 0.0),
                centre + Vec2::new(48.0, 0.0),
            ];
            art_miniboss::draw(art, &e, &mb, cam, accent);
            art_miniboss::draw_telegraph(art, &mb, cam, accent);
        }
        Key::Boss(kind) => draw_boss(art, kind, accent, t),
        Key::Weapon(kind) | Key::Evolution(kind) => {
            let icon = match key {
                Key::Evolution(_) => art_icons::evolution(kind).unwrap_or_else(|| art_icons::weapon(kind)),
                _ => art_icons::weapon(kind),
            };
            let gold = matches!(key, Key::Evolution(_));
            draw_icon(art, icon, gold, t);
        }
        Key::Passive(kind) => draw_icon(art, art_icons::passive(kind), false, t),
        Key::Splice(kind) => draw_icon(art, art_icons::splice(kind), false, t),
        Key::Pickup(PickupKind::Headlamp) => {
            crate::game::art_headlamp::draw_headlamp_icon(art, centre.x as i32 - 4, centre.y as i32 - 4);
        }
        Key::Pickup(kind) => {
            let s = (centre.x as i32, (centre.y + (t * 2.0).sin() * 2.0) as i32);
            art_floor::draw_pickup(art, kind, s, t);
        }
        Key::Object(kind) => draw_object_art(art, scenes, kind, centre, theme, accent, t),
        Key::Prop(kind) => {
            let (w, h) = kind.size();
            let prop = Prop {
                kind,
                rect: Rectf::new(centre.x - w * 0.5, centre.y - h * 0.5, w, h),
            };
            art_props::draw_prop_at(art, &prop, cam, &Palette::for_theme(theme), t);
        }
        Key::Floor(theme) => {
            let level = scene_level(scenes, theme);
            if theme == Theme::Outfall {
                draw_outfall_floor_plate(art, level, t);
                return;
            }
            art_floor::draw_floor(art, level, cam, t);
            for prop in &level.props {
                art_props::draw_prop_at(art, prop, cam, &level.palette, t);
            }
        }
    }
}

fn draw_outfall_floor_plate(art: &mut PixelBuffer, level: &Level, t: f32) {
    if art.w == 0 || art.h == 0 {
        return;
    }
    let mut bank = PixelBuffer::new(art.w * 2, art.h * 2);
    let cam = level.start + Vec2::new(0.0, 5.0) - Vec2::new(art.w as f32, art.h as f32);
    bank.clear(level.palette.bg);
    art_floor::draw_floor(&mut bank, level, cam, t);
    for patch in &level.seabed_patches {
        bank.set_pen(crate::game::art_seabed::seabed_pen(patch.kind));
        crate::game::art_seabed::draw_seabed_patch(&mut bank, patch, &level.tiles, cam);
    }
    bank.set_pen(0);
    art_floor::draw_outfall_hazards(&mut bank, level, cam, t);
    for prop in &level.props {
        art_props::draw_prop(&mut bank, prop, cam, &level.palette);
    }
    bank.resample_to(art);
}

/// The enemy card's loop: walk, then the kind's own tell.
fn pose_enemy(e: &mut Enemy, t: f32) {
    match e.kind {
        EnemyKind::Pillbug => {
            let cycle = t.rem_euclid(4.0);
            if cycle > 2.2 {
                e.w3.curl_t = cycle - 2.2;
                e.anim = 0.0;
            }
        }
        EnemyKind::BellFlea => {
            let cycle = t.rem_euclid(3.0);
            e.w3.home = e.pos;
            e.w3.hop_to = e.pos + Vec2::new(26.0, -4.0);
            e.w3.flea_phase = if cycle < 1.4 { 1 } else { 2 };
            e.w3.flea_t = 0.4;
        }
        EnemyKind::Turnkey => {
            let cycle = t.rem_euclid(3.0);
            e.w3.bar_pos = e.pos - Vec2::new(30.0, 0.0);
            e.w3.bar_dir = Vec2::new(0.0, 1.0);
            e.w3.bar_t = 1.6 - cycle * 0.5;
        }
        EnemyKind::Porter => {
            let cycle = t.rem_euclid(2.6);
            if cycle > 1.4 {
                e.w3.raise_t = cycle - 1.4;
            }
        }
        EnemyKind::Spitter => {
            // Low and falling: the muzzle lights before it fires.
            e.fire_cd = 1.4 - t.rem_euclid(1.4);
        }
        EnemyKind::Cat => {
            e.contact_cd = t.rem_euclid(1.2);
        }
        _ => {}
    }
}

/// The frame, in px at 1:1, a boss's card pose needs around its body point
/// for the whole animal to be in it (`(width, height)`). A plate is at most
/// 44 × 30 px and every boss sprite is 26–58 px long, so 1:1 is a crop.
fn boss_extent(kind: BossKind) -> (f32, f32) {
    match kind {
        BossKind::Cat => (66.0, 42.0),
        // The shoes stand `STANCE_HALF_WIDTH` either side of the body point
        // (each drawn 13 px in radius), and the broom head reaches
        // `SWEEP_OUTER_RADIUS` ahead. The sweep's far left and right (about
        // 36 px off-axis) are allowed to leave the plate: fitting them would
        // cost a 3x reduction and a 9 px pair of shoes.
        BossKind::Broom => (
            2.0 * (art_boss_broom::SWEEP_OUTER_RADIUS + 2.0),
            2.0 * (art_boss_broom::STANCE_HALF_WIDTH + 13.0 + 2.0),
        ),
        BossKind::Mower => (44.0, 42.0),
        // The machine is 26 px across, but its card pose is the intake: the
        // lane runs `NIP_OFFSET + INTAKE_LANE_LEN` (47.5 px) ahead of the body
        // point. Centred, that lane ran off the plate's right edge, so the
        // frame is the machine plus the whole lane and the body stands left
        // of centre (`boss_anchor_x`).
        BossKind::Mangle => (
            art_boss_mangle::BODY_RADIUS
                + art_boss_mangle::NIP_OFFSET
                + art_boss_mangle::INTAKE_LANE_LEN
                + 16.0,
            42.0,
        ),
        BossKind::NightEdition => (48.0, 44.0),
        BossKind::Magpie => (52.0, 42.0),
        // Scale the still plate with the shell and its long rear spike.
        BossKind::HorseshoeCrab => {
            let size = crate::game::art_boss_horseshoe_crab::BODY_SCALE;
            (82.0 * size, 46.0 * size)
        }
        BossKind::MegaRat => (68.0, 44.0),
        // Wings at full span over the hover's lift.
        BossKind::Hawk => (58.0, 60.0),
        // The open hood and the fangs ahead of it, whole. The slack body
        // (`rest_chain`) trails some 70 px astern and 60 px to either side,
        // far longer than any plate, so the card frames the head and lets
        // the first loops run off the edge.
        BossKind::Cobra => {
            use crate::game::art_boss_cobra as cobra;
            (
                2.0 * (cobra::HOOD_RADIUS + cobra::FANG_REACH + 8.0),
                2.0 * (cobra::HOOD_RADIUS + 8.0),
            )
        }
        BossKind::Lantern => (126.0, 64.0),
    }
}

/// Where the body point stands across the card, as a fraction of its width.
/// Every boss is centred but the Mangle, whose pose is one-sided: the machine
/// on the left, its intake lane reaching right.
fn boss_anchor_x(kind: BossKind) -> f32 {
    match kind {
        BossKind::Mangle => {
            let (need_w, _) = boss_extent(kind);
            (art_boss_mangle::BODY_RADIUS + 10.0) / need_w
        }
        BossKind::Lantern => 0.66,
        BossKind::HorseshoeCrab => 0.64,
        _ => 0.5,
    }
}

/// A posed boss (its tell or its signature), looping, fitted to the plate.
///
/// The art modules draw at 1:1 only (their geometry is the gameplay spec), so
/// a plate too small for the animal is drawn `k`× oversized into a scratch
/// buffer over the plate's own floor and box-filtered back down. The whole
/// pose — body, shadow, tell — shrinks together about the same body point.
fn draw_boss(art: &mut PixelBuffer, kind: BossKind, accent: Rgb, t: f32) {
    let (w, h) = (art.w, art.h);
    if w == 0 || h == 0 {
        return;
    }
    let (need_w, need_h) = boss_extent(kind);
    let k = ((need_w / w as f32).max(need_h / h as f32).ceil() as usize).clamp(1, 8);
    if k == 1 || art.pixels().len() < w * h {
        pose_boss(art, kind, accent, t);
        return;
    }
    let (bw, bh) = (w * k, h * k);
    let mut big = PixelBuffer::new(bw, bh);
    {
        let src = art.pixels();
        let dst = big.pixels_mut();
        for y in 0..bh {
            for x in 0..bw {
                dst[y * bw + x] = src[(y / k) * w + x / k];
            }
        }
    }
    pose_boss(&mut big, kind, accent, t);
    let n = (k * k) as u32;
    let src = big.pixels();
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for yy in 0..k {
                let row = (y * k + yy) * bw + x * k;
                for px in &src[row..row + k] {
                    r += px.0 as u32;
                    g += px.1 as u32;
                    b += px.2 as u32;
                }
            }
            art.plot(x as i32, y as i32, ((r / n) as u8, (g / n) as u8, (b / n) as u8));
        }
    }
}

/// Draw a boss's card pose at 1:1, body at the buffer's `(w/2, 0.56h)`.
fn pose_boss(art: &mut PixelBuffer, kind: BossKind, accent: Rgb, t: f32) {
    let c = Vec2::new(art.w as f32 * boss_anchor_x(kind), art.h as f32 * 0.56);
    let facing = Vec2::new(1.0, 0.0);
    let target = c + Vec2::new(28.0, 4.0);
    let cycle = t.rem_euclid(3.2) / 3.2;
    let anim = t * 1.7;
    match kind {
        BossKind::Cat => {
            let mut v = BossCatView::new(c, facing, target);
            v.state = crate::game::art_boss_cat::BossCatState::Crouch;
            v.progress = cycle;
            v.anim = anim;
            v.pounce_origin = c;
            art_boss_cat::draw_boss_cat_ground(art, &v, Vec2::ZERO, accent);
            art_boss_cat::draw_boss_cat(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Broom => {
            let mut v = BroomBossView {
                pos: c,
                facing,
                state: crate::game::art_boss_broom::BroomBossState::StompWalk { left_foot: true },
                progress: 0.62,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.8,
                target,
                enraged: false,
            };
            v.state = crate::game::art_boss_broom::BroomBossState::BroomSweep { clockwise: true };
            v.progress = cycle;
            art_boss_broom::draw_ground(art, &v, Vec2::ZERO, accent);
            art_boss_broom::draw(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Mower => {
            let mut v = MowerView::patrol(c, facing, anim);
            v.state = crate::game::art_boss_mower::MowerState::Scan;
            v.progress = cycle;
            v.target = target;
            art_boss_mower::draw_mower_ground(art, &v, Vec2::ZERO, accent);
            art_boss_mower::draw_mower(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Mangle => {
            let v = MangleView {
                pos: c,
                facing,
                state: crate::game::art_boss_mangle::MangleState::Intake { second: false },
                progress: cycle,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.7,
                target: c + Vec2::new(34.0, 0.0),
                phase: 1,
            };
            art_boss_mangle::draw_ground(art, &v, Vec2::ZERO, accent);
            art_boss_mangle::draw(art, &v, Vec2::ZERO, accent);
        }
        BossKind::NightEdition => {
            let v = NightEditionView {
                pos: c,
                facing,
                state: crate::game::art_boss_night_edition::NightEditionState::PageTell,
                progress: cycle,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.7,
                phase: 1,
                page: c + Vec2::new(20.0, 0.0),
                margin_side: 1.0,
                rule_from: c + Vec2::new(-30.0, 8.0),
                rule_to: c + Vec2::new(40.0, 8.0),
                bed: c,
            };
            art_boss_night_edition::draw_ground(art, &v, Vec2::ZERO, accent);
            art_boss_night_edition::draw(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Magpie => {
            let v = MagpieView {
                pos: c,
                facing,
                state: crate::game::art_boss_magpie::MagpieState::PeckWindup,
                progress: cycle,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.8,
                target: c + Vec2::new(24.0, 10.0),
                lane_start: c - Vec2::new(20.0, 0.0),
                sweep: 0.0,
                bank: 0.0,
                hunting: true,
            };
            art_boss_magpie::draw_magpie_ground(art, &v, Vec2::ZERO, accent);
            art_boss_magpie::draw_magpie(art, &v, Vec2::ZERO, accent);
        }
        BossKind::HorseshoeCrab => {
            use crate::game::art_boss_horseshoe_crab::{self as crab, CrabState, CrabView};
            // The tail cocked over one shoulder: the tell the fight turns on.
            let v = CrabView {
                pos: c,
                facing,
                state: CrabState::TelsonTell,
                progress: cycle,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.8,
                phase: 1,
                target,
                lane_start: c,
                sweep_side: 1.0,
            };
            crab::draw_ground(art, &v, Vec2::ZERO, accent);
            crab::draw(art, &v, Vec2::ZERO, accent);
        }
        BossKind::MegaRat => {
            use crate::game::art_boss_megarat::{self as megarat, MegaRatState, MegaRatView};
            let mut v = MegaRatView::new(c, facing, target);
            v.state = MegaRatState::GnawWindup;
            v.progress = cycle;
            v.anim = anim;
            megarat::draw_megarat_ground(art, &v, Vec2::ZERO, accent);
            megarat::draw_megarat(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Hawk => {
            // The hover: wings at full span a body-length over the planks,
            // downdraft rings under it. (A dive lifts the bird up to 58 px
            // above its ground point, clean off the top of any plate.)
            let mut v = HawkView::new(c, facing, target);
            v.state = crate::game::art_boss_hawk::HawkState::Hover;
            v.progress = cycle;
            v.anim = anim;
            v.hp_frac = 0.7;
            v.phase = 2;
            v.lane_start = c - Vec2::new(26.0, 6.0);
            v.decoy = target + Vec2::new(0.0, 20.0);
            art_boss_hawk::draw_hawk_ground(art, &v, Vec2::ZERO, accent);
            art_boss_hawk::draw_hawk(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Cobra => {
            use crate::game::art_boss_cobra::{self as cobra, CobraState, CobraView};
            // The hood open over a marked strike line: the tell the fight is
            // read by.
            let v = CobraView {
                pos: c,
                facing,
                state: CobraState::HoodTell,
                progress: cycle,
                anim,
                hit_flash: 0.0,
                hp_frac: 0.8,
                phase: 1,
                target,
                lane_start: target,
                gap_angle: 0.0,
                pools: [Vec2::ZERO; cobra::POOL_VIEW],
                pool_r: [0.0; cobra::POOL_VIEW],
                pool_n: 0,
                hunting: false,
                arena: crate::game::level::TileRect::new(0, 0, 20, 16),
            };
            cobra::draw_ground(art, &v, Vec2::ZERO, accent);
            cobra::draw(art, &v, Vec2::ZERO, accent);
        }
        BossKind::Lantern => {
            use crate::game::art_boss_lantern::{self as lantern, LanternBulbView, LanternState, LanternView};
            let view = LanternView {
                pos: c,
                facing,
                anim,
                progress: cycle,
                state: LanternState::Swim,
                hit_flash: 0.0,
                hp_frac: 1.0,
                phase: 1,
                attack_origin: c,
                attack_target: c,
                attack_facing: facing,
                attack_locked: false,
                attack: None,
                stamp: None,
                real_bulb: Some(LanternBulbView {
                    pos: c + Vec2::new(28.0, -7.0),
                    radius: 40.0,
                    gain: 1.0,
                    flicker: 0.0,
                    is_decoy: false,
                    planted: false,
                }),
                decoy: None,
            };
            lantern::draw_ground(art, &view, Vec2::ZERO, accent);
            lantern::draw(art, &view, Vec2::ZERO, accent);
        }
    }
}

/// Blit an authored icon sprite scaled up into the card (3×, centred), with the
/// chest's gilded frame for an evolution.
fn draw_icon(art: &mut PixelBuffer, icon: &art_icons::Icon, gold: bool, t: f32) {
    const SCALE: i32 = 3;
    let grid = art_icons::LARGE_PX_W as i32 * SCALE;
    let x0 = (art.w as i32 - grid) / 2;
    let y0 = (art.h as i32 - grid) / 2;
    if gold {
        for y in -2..grid + 2 {
            for x in -2..grid + 2 {
                let border = x < 0 || y < 0 || x >= grid || y >= grid;
                if border {
                    let g: Rgb = if (x + y) % 2 == 0 {
                        (255, 216, 62)
                    } else {
                        (214, 142, 20)
                    };
                    art.plot(x0 + x, y0 + y, g);
                }
            }
        }
    }
    for (y, row) in icon.large.iter().enumerate() {
        for (x, px) in row.iter().enumerate() {
            let Some(c) = px else {
                continue;
            };
            // A slow sheen so the still life has a pulse without faking a demo.
            let sheen = 0.92 + 0.08 * ((t * 2.0) + (x + y) as f32 * 0.4).sin();
            let c = mix(*c, (255, 255, 255), (1.0 - sheen).max(0.0));
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    art.plot(x0 + x as i32 * SCALE + dx, y0 + y as i32 * SCALE + dy, c);
                }
            }
        }
    }
    let _ = icon.accent;
}

/// The Field Guide's own objects: hazards over the live-cached scenes, the rest
/// through the art modules' view structs.
#[allow(clippy::too_many_arguments)]
fn draw_object_art(
    art: &mut PixelBuffer,
    scenes: &mut Scenes,
    kind: ObjectKind,
    centre: Vec2,
    theme: Theme,
    accent: Rgb,
    t: f32,
) {
    let pal = Palette::for_theme(theme);
    let cam = Vec2::ZERO;
    match kind {
        ObjectKind::StickyBoard => {
            let mut v = StickyTrapView::new(centre, Vec2::new(1.0, 0.0));
            if t.rem_euclid(3.0) > 1.8 {
                v.state = art_traps::StickyTrapState::Stuck;
                v.progress = 0.3;
                v.victim_pos = centre + Vec2::new(5.0, 3.0);
            }
            art_traps::draw_sticky_trap_ground(art, &v, cam, accent);
            art_traps::draw_sticky_trap(art, &v, cam, accent);
        }
        ObjectKind::Mousetrap => {
            let mut v = MousetrapView::new(centre, Vec2::new(1.0, 0.0));
            let cycle = t.rem_euclid(2.6);
            v.state = if cycle > 1.9 {
                art_traps::MousetrapState::Snapping
            } else {
                art_traps::MousetrapState::Armed
            };
            v.progress = ((cycle - 1.9) / 0.6).clamp(0.0, 1.0);
            art_traps::draw_mousetrap_ground(art, &v, cam, accent);
            art_traps::draw_mousetrap(art, &v, cam, accent);
        }
        ObjectKind::SteamGrate => {
            let objects = steam_scene(scenes, centre, t);
            art_floor::draw_hazards(art, Theme::Laundry, objects, cam, t);
        }
        ObjectKind::InkPad => {
            let objects = ink_scene(scenes, centre);
            art_floor::draw_hazards(art, Theme::Printworks, objects, cam, t);
        }
        ObjectKind::Pit | ObjectKind::SlidingCargo | ObjectKind::Slipstream => {
            let level = scene_level(scenes, Theme::Trailer);
            art_floor::draw_floor(art, level, cam, t);
        }
        ObjectKind::RatHole => {
            let v = RatHoleView::new(centre, Vec2::new(1.0, 0.0));
            art_traps::draw_rat_hole_ground(art, &v, cam, accent);
            art_traps::draw_rat_hole(art, &v, cam, accent);
        }
        ObjectKind::MazeCache => {
            let mut v = art_traps::MazeRewardView::new(centre, Vec2::new(0.0, 1.0));
            let cycle = t.rem_euclid(2.8);
            if cycle > 1.6 {
                v.state = art_traps::MazeRewardState::Opening;
                v.progress = (cycle - 1.6) / 0.9;
            }
            art_traps::draw_maze_reward_ground(art, &v, cam, accent);
            art_traps::draw_maze_reward(art, &v, cam, accent);
        }
        ObjectKind::OneWayDoor => {
            let mut v = art_traps::OneWayDoorView::new(centre, Vec2::new(1.0, 0.0));
            let cycle = t.rem_euclid(2.4);
            if cycle > 1.2 {
                v.state = art_traps::OneWayDoorState::Opening;
                v.progress = ((cycle - 1.2) / 0.6).clamp(0.0, 1.0);
            }
            art_traps::draw_one_way_door_ground(art, &v, cam, accent);
            art_traps::draw_one_way_door(art, &v, cam, accent);
        }
        ObjectKind::ShopHole => {
            let cell = (centre.x as i32 / 16, centre.y as i32 / 16);
            let hole = crate::game::level::ShopHole {
                index: 0,
                cell,
                pos: centre,
                facing: Vec2::new(1.0, 0.0),
                approach: crate::game::art_walls::Cardinal::East,
                mouth: (cell.0 + 1, cell.1),
                role: crate::game::level::ShopHoleRole::Fork,
            };
            crate::game::art_walls::draw_shop_hole(art, &hole, cam, t);
        }
        ObjectKind::Gold => {
            art_floor::draw_gold_coin(art, (centre.x as i32, centre.y as i32), 3, t, 0.5);
        }
        ObjectKind::Chest => {
            art_floor::draw_chest(art, (centre.x as i32, centre.y as i32), t);
        }
        ObjectKind::Elite => {
            let _ = pal;
        }
        ObjectKind::Gem => {
            let spacing = ((art.w as i32 - 9) / 4).clamp(1, 9);
            for (i, tier) in GemTier::ALL.into_iter().enumerate() {
                let pos = (centre.x as i32 + (i as i32 - 2) * spacing, centre.y as i32);
                let gem = Gem::new(centre, tier.min_value());
                crate::game::art_gems::draw_gem(art, pos, &gem, 0.0, i as u32);
            }
        }
        ObjectKind::CrabPot => {
            use crate::game::art_crabpot::{draw_crab_pot, draw_crab_pot_front};
            let mut view = crate::game::ecs::systems::traps::set_crab_pot(centre, Vec2::new(0.0, 1.0));
            view.anim = t;
            draw_crab_pot(art, &view, cam);
            draw_crab_pot_front(art, &view, cam);
        }
    }
}

/// The cached steam-grate scene (a single grate, mid-cycle).
fn steam_scene(scenes: &mut Scenes, centre: Vec2, t: f32) -> &LevelObjects {
    let objects = scenes.steam.get_or_insert_with(|| {
        let mut objects = LevelObjects::default();
        objects.steam_grates.push(hazards::SteamGrate::new(
            ((centre.x / 16.0) as i32, (centre.y / 16.0) as i32),
            0.0,
        ));
        objects
    });
    for grate in &mut objects.steam_grates {
        grate.seek(t.rem_euclid(hazards::STEAM_CYCLE_SECS));
    }
    objects
}

/// The cached ink-pad scene.
fn ink_scene(scenes: &mut Scenes, centre: Vec2) -> &LevelObjects {
    scenes.ink.get_or_insert_with(|| {
        let mut objects = LevelObjects::default();
        let cell = ((centre.x / 16.0) as i32, (centre.y / 16.0) as i32);
        objects.ink_pads.push(hazards::InkPad::new(
            TileRect::new(cell.0 - 1, cell.1 - 1, 3, 3),
            7,
        ));
        objects
    })
}

/// A cached miniature level for a theme: floor paint, a handful of props, the
/// theme's hazard, and (on the flatbed) a pit, a crate and a wind stretch.
fn scene_level(scenes: &mut Scenes, theme: Theme) -> &Level {
    scenes.levels[theme as usize].get_or_insert_with(|| build_scene(theme))
}

fn build_scene(theme: Theme) -> Level {
    if theme == Theme::Outfall {
        return scene::outfall_floor_patch();
    }
    let cols = 30;
    let rows = 20;
    let tiles = crate::game::collision::TileMap::new(cols, rows, theme, 0);
    let nav = crate::game::nav::NavGrid::from_tiles(&tiles, SizeClass::Large);
    let arena = Vec2::new(cols as f32 * 16.0, rows as f32 * 16.0);
    let mut level = Level {
        theme,
        seed: 0,
        arena,
        tiles,
        props: Vec::new(),
        palette: Palette::for_theme(theme),
        nav,
        rooms: Vec::new(),
        start: arena * 0.5,
        mazes: Vec::new(),
        objects: LevelObjects::default(),
        boss: crate::game::level::BossArena::default(),
        trailer: TrailerDeck::default(),
        puddles: Vec::new(),
        seabed_patches: Vec::new(),
        plants: Vec::new(),
        clothes_piles: Vec::new(),
    };
    // A few props on the floor, cycling the theme's own furniture.
    let kinds = theme_props(theme);
    for (i, kind) in kinds.iter().take(4).enumerate() {
        let (w, h) = kind.size();
        let x = 18.0 + (i as f32 % 2.0) * 90.0;
        let y = 26.0 + (i as f32 / 2.0) * 40.0;
        level.props.push(Prop {
            kind: *kind,
            rect: Rectf::new(x, y, w, h),
        });
    }
    match theme {
        Theme::Laundry => {
            level
                .objects
                .steam_grates
                .push(hazards::SteamGrate::new((9, 9), 4.6));
        }
        Theme::Printworks => {
            level
                .objects
                .ink_pads
                .push(hazards::InkPad::new(TileRect::new(8, 8, 3, 3), 11));
        }
        Theme::Trailer => {
            level.trailer = TrailerDeck {
                deck: TileRect::new(0, 0, cols, rows),
                tailgate: Vec2::new(24.0, arena.y * 0.5),
                pits: vec![Pit::new(TileRect::new(11, 8, 3, 2), PitKind::BrokenPlank)],
                wind: vec![TileRect::new(4, 0, 6, rows)],
                slides: Vec::new(),
            };
        }
        _ => {}
    }
    level
}

/// The card's own floor paint: the theme's two-tone ground, so the art has
/// something to cast a shadow on.
fn paint_floor(pb: &mut PixelBuffer, pal: &Palette, t: f32) {
    let base = mix(pal.bg, (0, 0, 0), 0.18);
    let alt = mix(pal.bg_alt, (0, 0, 0), 0.12);
    pb.clear(base);
    // 8 px checker offset every other row, very low contrast.
    let mut y = 0;
    while y < pb.h {
        let mut x = 0;
        while x < pb.w {
            if ((x / 8) + (y / 8)) % 2 == 0 {
                pb.rect_fill(x as i32, y as i32, 8, 8, alt);
            }
            x += 8;
        }
        y += 8;
    }
    // A faint seam, so a still card is not completely dead. It is fixed
    // because `anim` only advances while a scene runs (a moving still plate
    // would keep the screen from ever going idle).
    let seam = (t * 6.0) as i32 % (pb.w as i32).max(1);
    for yy in 0..pb.h as i32 {
        pb.plot(seam, yy, mix(alt, pal.accent, 0.22));
    }
}

/// The card: art, name, dossier, facts and the player's own record.
#[allow(clippy::too_many_arguments)]
fn draw_card(
    f: &mut Frame,
    area: Rect,
    rows: &[Row],
    lib: &Library,
    ui: &mut LibraryUi,
    accent: Color,
    accent_rgb: Rgb,
    narrow: bool,
    hits: &mut HitMap,
) -> Option<()> {
    if area.width < 12 || area.height < 6 {
        return None;
    }
    let row = entry_row(rows, ui.tab, ui.cursor)?;
    let gem_card = row.key == Key::Object(ObjectKind::Gem);
    let seen = lib.is_seen(row.key);
    let full = catalogued(lib, row.key);
    // Everything the text half will need, measured before the art takes a row.
    let facts = library::facts(row.key);
    let fact_rows = facts.rows().len() as u16;
    let stats = stats_line(lib, row.key);
    let flavour = if full {
        library::flavour_of(row.key)
    } else {
        None
    };
    let width = usize::from(area.width);
    let mut dossier = row.dossier;
    if !full {
        dossier = "The field note is withheld until the rat has carried this. ";
    }
    let mut dossier_lines = wrap_words(dossier, width);
    dossier_lines.truncate(DOSSIER_MAX_LINES);
    // flavour + note + (OFFERED and the `found:` hint) + rule + facts + record
    let mut want = u16::from(flavour.is_some()) + dossier_lines.len() as u16;
    want += if full { 0 } else { 2 };
    want += STATS_H;
    want = want.saturating_add(1 + fact_rows);
    let base_art: u16 = if narrow {
        NARROW_ART_H
    } else {
        (area.height / 2).clamp(9, 15)
    };
    // The plate keeps the top of the card and gives up only the rows the note
    // needs, so a long dossier is never cut for an oversized specimen.
    let art_h = area
        .height
        .saturating_sub(3 + want)
        .clamp(MIN_ART_H, base_art.max(MIN_ART_H));
    // Viewing the card is what clears the NEW mark (the App owns that).
    let mut x = area.x;
    let name = if seen { row.name } else { "???" };
    put(
        f,
        x,
        area.y,
        area.width,
        name,
        heavy(if seen { accent } else { MUTED }),
    );
    x += name.chars().count() as u16 + 1;
    let tag = if seen { lib_name(row.key) } else { row.group };
    let tag_w = tag.chars().count() as u16;
    if x + 1 + tag_w <= area.right() {
        let tx = area.right().saturating_sub(tag_w);
        put(f, tx, area.y, tag_w, tag, ink(MUTED));
    }

    // Art window.
    let art_w_cells = area
        .width
        .saturating_sub(2)
        .min(if narrow && !gem_card { 26 } else { 44 });
    let art_rect = Rect::new(area.x + 1, area.y + 2, art_w_cells, art_h);
    if art_rect.height >= 2 {
        ui.art
            .resize(art_rect.width as usize, art_rect.height as usize * 2);
        let anim = ui.anim;
        {
            let LibraryUi {
                art, scenes, scene, ..
            } = ui;
            // The card on screen and its scene: the real game. Anything else
            // (locked, half catalogued, still pose, the cursor still moving)
            // is the posed art.
            match scene.as_mut() {
                Some(running) if running.key() == row.key => running.draw(art),
                _ => draw_art(art, scenes, row.key, accent_rgb, anim),
            }
            if !seen || !full {
                silhouette(art);
            }
        }
        // A thin frame so the art reads as a plate, not a spill.
        for px in art_rect.x..art_rect.right() {
            f.buffer_mut()[(px, art_rect.y - 1)]
                .set_char('▁')
                .set_fg(TRACK)
                .set_bg(PLATE);
        }
        ui.art.render_to(art_rect, f.buffer_mut());
    }

    let mut y = art_rect.bottom() + 1;
    if y >= area.bottom() {
        hits.add(area, Target::LibraryRow(ui.cursor));
        return Some(());
    }

    if !seen {
        put(
            f,
            area.x,
            y,
            area.width,
            "SPECIMEN NOT YET CATALOGUED",
            heavy(MUTED),
        );
        y += 1;
        let hint = format!("found: {}", row.hint);
        let para = Paragraph::new(hint)
            .style(ink(VIAL))
            .wrap(Wrap { trim: true });
        let h = area.bottom().saturating_sub(y).min(3);
        if h > 0 {
            f.render_widget(para, Rect::new(area.x, y, area.width, h));
        }
        // A locked card is still clickable (it selects and clears nothing).
        hits.add(area, Target::LibraryRow(ui.cursor));
        return Some(());
    }

    // Flavour line (the game's own table, not retyped).
    if let Some(flavour) = flavour {
        let line = format!("\u{201c}{flavour}\u{201d}");
        put(
            f,
            area.x,
            y,
            area.width,
            &clip_line(&line, width),
            ink(VIAL).add_modifier(Modifier::ITALIC),
        );
        y += 1;
    }

    if !full {
        put(
            f,
            area.x,
            y,
            area.width,
            "OFFERED — NOT YET CARRIED",
            heavy(to_color((232, 198, 96))),
        );
        y += 1;
    }

    // Dossier (withheld until the entry is fully catalogued), wrapped on whole
    // words and cut on a sentence or an ellipsis rather than mid-clause.
    if y < area.bottom() {
        let tail = area.bottom() - y;
        let need = dossier_lines.len() as u16;
        // The record line keeps its place only where it does not cost a line
        // of the note: the note is what the card is for.
        let reserve = if tail >= need.saturating_add(1 + STATS_H) {
            1 + STATS_H
        } else {
            0
        };
        let room = usize::from(tail.saturating_sub(reserve));
        for line in fit_lines(&dossier_lines, room, width) {
            put(f, area.x, y, area.width, &line, ink(COAT));
            y += 1;
        }
    }
    if !full && y < area.bottom() {
        let hint = format!("found: {}", row.hint);
        put(f, area.x, y, area.width, &clip_line(&hint, width), ink(MUTED));
        y += 1;
    }

    // Short cards still fit the entire XP scale in two columns.
    if full && gem_card && area.bottom() - y <= fact_rows + STATS_H {
        let cols = area.width / 2;
        for pair in facts.rows().chunks(2) {
            if y >= area.bottom() {
                break;
            }
            for (i, fact) in pair.iter().enumerate() {
                let value = fact.value.strip_suffix(" XP").unwrap_or(&fact.value);
                let text = format!("{} {value}", fact.label);
                put(f, area.x + i as u16 * cols, y, cols, &text, ink(COAT));
            }
            y += 1;
        }
        hits.add(area, Target::LibraryRow(ui.cursor));
        return Some(());
    }

    // Facts: label, value, and a five-cell bar where a bar says more.
    if y < area.bottom().saturating_sub(STATS_H) {
        f.render_widget(
            Line::styled("─".repeat(usize::from(area.width)).as_str(), ink(TRACK)),
            band(area, y - area.y, 1),
        );
        y += 1;
    }
    for fact in facts.rows() {
        if y + STATS_H >= area.bottom() {
            break;
        }
        draw_fact(f, area, y, fact, accent);
        y += 1;
    }
    // The player's own record.
    if y < area.bottom() {
        let sy = area.bottom() - 1;
        put(f, area.x, sy, area.width, &clip_line(&stats, width), ink(MUTED));
    }
    hits.add(area, Target::LibraryRow(ui.cursor));
    Some(())
}

#[cfg(test)]
#[test]
fn gem_card_shows_every_tier_on_supported_terminal_sizes() {
    use ratatui::{backend::TestBackend, Terminal};

    let rows = library::all_rows();
    let key = Key::Object(ObjectKind::Gem);
    let mut lib = Library::default();
    lib.mark_seen(key);
    for (w, h) in [(57, 20), (80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut ui = LibraryUi::default();
        ui.set_tab(Tab::FieldGuide);
        assert!(ui.select_key(&rows, key));
        terminal
            .draw(|f| {
                let area = f.area();
                draw(f, area, &rows, &lib, (90, 220, 200), &mut ui, &mut HitMap::default());
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for (name, band) in [
            ("Shard", "pale green 1–4"),
            ("Chip", "cyan 5–24"),
            ("Gem", "violet 25–99"),
            ("Crystal", "amber 100–499"),
            ("Core", "crimson 500+"),
        ] {
            assert!(text.contains(name) && text.contains(band), "{w}x{h}: missing {name}");
        }
    }
}

/// Greedy word wrap at `width` columns. Words are never split, so a card can
/// not hyphenate a species name; a single word wider than the line is left
/// whole and clipped by the caller.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if width == 0 {
        return out;
    }
    let mut line = String::new();
    let mut line_w = 0usize;
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if line_w == 0 {
            line.push_str(word);
            line_w = w;
        } else if line_w + 1 + w <= width {
            line.push(' ');
            line.push_str(word);
            line_w += 1 + w;
        } else {
            out.push(std::mem::take(&mut line));
            line.push_str(word);
            line_w = w;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// True when the line ends a sentence (the card may cut there).
fn ends_sentence(line: &str) -> bool {
    matches!(line.trim_end().chars().last(), Some('.' | '!' | '?'))
}

/// Fit wrapped `lines` into `room` rows without ever cutting mid-word.
///
/// A whole sentence inside the window reads better than a trailing ellipsis, so
/// the last complete one is preferred (as long as it keeps at least half the
/// rows); otherwise the last row is trimmed back to a whole word and closed
/// with the safe `…` glyph.
fn fit_lines(lines: &[String], room: usize, width: usize) -> Vec<String> {
    if room == 0 {
        return Vec::new();
    }
    if lines.len() <= room {
        return lines.to_vec();
    }
    let mut kept: Vec<String> = lines[..room].to_vec();
    let half = room.div_ceil(2);
    if let Some(i) = kept
        .iter()
        .rposition(|line| ends_sentence(line))
        .filter(|&i| i + 1 >= half)
    {
        kept.truncate(i + 1);
        return kept;
    }
    if let Some(last) = kept.last_mut() {
        while last.chars().count() + 2 > width {
            match last.rfind(' ') {
                Some(i) if i > 0 => last.truncate(i),
                _ => {
                    *last = "\u{2026}".to_string();
                    break;
                }
            }
        }
        if !last.ends_with('\u{2026}') {
            last.push_str(" \u{2026}");
        }
    }
    kept
}

/// One line cut to `width`: unchanged when it fits, else whole words plus the
/// safe `…` glyph (never a sliced letter).
fn clip_line(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_string();
    }
    // Whole words first: the glyph and its space cost two cells of the line.
    let mut out = String::new();
    for word in text.split_whitespace() {
        let w = word.chars().count();
        let next = out.chars().count() + if out.is_empty() { w } else { w + 1 };
        if next + 2 > width {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        "\u{2026}".to_string()
    } else {
        out.push_str(" \u{2026}");
        out
    }
}

fn draw_fact(f: &mut Frame, area: Rect, y: u16, fact: &Fact, accent: Color) {
    const LABEL_W: u16 = 12;
    put(f, area.x, y, LABEL_W, fact.label, ink(MUTED));
    let mut vx = area.x + LABEL_W;
    let avail = area.right().saturating_sub(vx);
    if avail == 0 {
        return;
    }
    // Value first, then the bar in the space that is left.
    let bar_w: u16 = if fact.bar >= 0.0 { 6 } else { 0 };
    let value = &fact.value;
    let vw = (value.chars().count() as u16).min(avail.saturating_sub(bar_w));
    put(f, vx, y, vw, value, ink(COAT));
    vx += vw;
    if fact.bar >= 0.0 && avail > bar_w {
        let cells = (fact.bar * 5.0).round() as u16;
        let bar: String = (0..5).map(|i| if i < cells { '▮' } else { '▯' }).collect();
        put(f, vx + 1, y, 5, &bar, ink(accent));
    }
}

/// The player's lifetime record for one entry, phrased per category.
fn stats_line(lib: &Library, key: Key) -> String {
    let r = lib.record(key);
    if !lib.is_seen(key) {
        return "NOT YET LOGGED".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let picked = match key {
        Key::Pickup(_) => Some(("COLLECTED", r.picked)),
        Key::Weapon(_) | Key::Passive(_) => Some(("TAKEN", r.picked)),
        Key::Splice(_) => Some(("BOUGHT", r.owned)),
        _ => None,
    };
    if let Some((label, n)) = picked {
        if n > 0 {
            parts.push(format!("{label} {n}"));
        }
    }
    if r.seen > 0 {
        parts.push(format!("SEEN {}", r.seen));
    }
    if r.killed > 0 {
        parts.push(format!("KILLED {}", r.killed));
    }
    if r.killed_by > 0 {
        parts.push(format!("KILLED YOU {}", r.killed_by));
    }
    if r.owned > 0 && !matches!(key, Key::Splice(_)) {
        parts.push(format!("OWNED {}", r.owned));
    }
    if r.evolved > 0 {
        parts.push(format!("EVOLVED {}", r.evolved));
    }
    if parts.is_empty() {
        "NOT YET LOGGED".to_string()
    } else {
        format!("YOU · {}", parts.join(" · "))
    }
}

/// The row the cursor points at inside a tab.
fn entry_row(rows: &[Row], tab: Tab, cursor: usize) -> Option<Row> {
    rows.iter().filter(|r| r.tab == tab).nth(cursor).copied()
}

/// Short category tag printed top-right of the card.
fn lib_name(key: Key) -> &'static str {
    match key {
        Key::Enemy(_) => "ORDINARY",
        Key::Miniboss(_) => "MINIBOSS",
        Key::Boss(_) => "FLOOR BOSS",
        Key::Weapon(_) => "WEAPON",
        Key::Evolution(_) => "EVOLUTION",
        Key::Passive(_) => "PASSIVE",
        Key::Splice(_) => "SPLICE",
        Key::Pickup(_) => "POCKET FIND",
        Key::Prop(_) => "OBSTACLE",
        Key::Floor(_) => "FLOOR",
        Key::Object(_) => "FIELD GUIDE",
    }
}

/// Darken the art into a silhouette (every locked card).
fn silhouette(pb: &mut PixelBuffer) {
    for px in pb.pixels_mut() {
        let v = mix(*px, (34, 38, 42), 0.86);
        *px = v;
    }
}

/// A `PickupKind`'s colour (used by tests and the enemy card's tags).
pub fn pickup_tint(kind: PickupKind) -> Rgb {
    kind.color()
}

/// A weapon's colour, re-exported for the card's accents.
pub fn weapon_tint(kind: WeaponKind) -> Rgb {
    kind.color()
}

#[cfg(test)]
mod tests;

/// The boss plates: every boss fits its card whole (kept beside `draw_boss`).
#[cfg(test)]
mod boss_plate_tests {
    use super::*;

    const BG: Rgb = (10, 10, 10);

    fn plate(kind: BossKind, w: usize, h: usize, t: f32) -> PixelBuffer {
        let mut art = PixelBuffer::new(w, h);
        art.clear(BG);
        draw_boss(&mut art, kind, (90, 220, 200), t);
        art
    }

    /// No boss leaves its plate nearly empty (the Redtail's dive used to be
    /// drawn above the top row), at the wide and the narrow card sizes.
    #[test]
    fn every_boss_paints_its_plate() {
        for kind in BossKind::all() {
            for (w, h, min) in [(44usize, 30usize, 90usize), (26, 12, 20)] {
                for t in [0.0f32, 0.9, 2.1] {
                    let art = plate(kind, w, h, t);
                    let painted = art.pixels().iter().filter(|p| **p != BG).count();
                    assert!(
                        painted >= min,
                        "{kind:?} leaves a {w}x{h} plate nearly empty at t={t}: {painted} px"
                    );
                }
            }
        }
    }

    /// The Mangle's card pose is its intake: the machine and the whole lane
    /// ahead of it are on the plate. (Centred, the lane ran off the right
    /// edge.) Nothing touches the plate's first or last column at any point
    /// of the loop.
    #[test]
    fn the_mangle_and_its_intake_lane_fit_the_plate() {
        let (w, h) = (44usize, 30usize);
        for t in [0.0f32, 0.4, 0.9, 1.5, 2.1, 2.9] {
            let art = plate(BossKind::Mangle, w, h, t);
            let px = art.pixels();
            for y in 0..h * 2 {
                if y * w + w > px.len() {
                    break;
                }
                assert_eq!(px[y * w + w - 1], BG, "t={t}: row {y} runs off the right edge");
                // The machine's soft ground shadow may graze the left column;
                // nothing solid does.
                let left = px[y * w];
                let off = |a: u8, b: u8| a.abs_diff(b) > 10;
                assert!(
                    !(off(left.0, BG.0) || off(left.1, BG.1) || off(left.2, BG.2)),
                    "t={t}: row {y} runs off the left edge ({left:?})"
                );
            }
        }
        // The body stands left of centre, with room for itself.
        let (need_w, _) = boss_extent(BossKind::Mangle);
        let anchor = boss_anchor_x(BossKind::Mangle) * need_w;
        assert!(anchor >= crate::game::art_boss_mangle::BODY_RADIUS);
        assert!(
            anchor
                + crate::game::art_boss_mangle::NIP_OFFSET
                + crate::game::art_boss_mangle::INTAKE_LANE_LEN
                < need_w
        );
    }

    /// The reduction covers the animal: the extent table always yields a
    /// scratch frame at least as large as the boss needs, and degenerate
    /// plates do not panic.
    #[test]
    fn the_scratch_frame_holds_the_whole_boss() {
        for kind in BossKind::all() {
            let (need_w, need_h) = boss_extent(kind);
            for (w, h) in [(44usize, 30usize), (26, 12), (44, 18)] {
                let k = (need_w / w as f32).max(need_h / h as f32).ceil();
                assert!(w as f32 * k >= need_w && h as f32 * k >= need_h, "{kind:?}");
                assert!(k <= 8.0, "{kind:?} needs more than the 8x cap on {w}x{h}");
            }
            for (w, h) in [(1usize, 1usize), (2, 2), (200, 150)] {
                let _ = plate(kind, w, h, 1.0);
            }
        }
    }
}
