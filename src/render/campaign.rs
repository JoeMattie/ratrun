//! Three-level campaign UI screens: Level Complete, Keep Three escape manifest,
//! Level 2 & 3 Intros, Campaign Victory, Title Level Select, and compact HUD
//! campaign progress pips.
//!
//! All draw entry points are pure functions of view structs defined in this
//! module (zero dependency on simulation types), perform zero heap allocation
//! per frame, use single-cell-width glyphs only (Ghostty-safe), and return
//! stack-allocated [`HitList`]s mapping clickable screen rectangles to actions.

use core::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::game::art_icons;
use crate::game::level::Theme;
use crate::render::palette::{mix, to_color, Rgb, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

// Semantic colors matching `src/render/hud.rs` and `src/render/menu.rs`.
const VICTORY_MINT: Color = Color::Rgb(169, 210, 186);
const HEAL_MINT: Color = Color::Rgb(126, 224, 152);
const TRAIL_GOLD: Color = Color::Rgb(248, 214, 152);
const WARN_AMBER: Color = Color::Rgb(242, 184, 102);
const DANGER: Color = Color::Rgb(248, 92, 102);
const PLATE_SUB: Color = Color::Rgb(24, 31, 35);
const PLATE_HILITE: Color = Color::Rgb(30, 40, 44);
const PLATE_PACKED: Color = Color::Rgb(22, 36, 33);
const PLATE_DIM: Color = Color::Rgb(15, 19, 22);

// ---------------------------------------------------------------------------
// Stack-allocated zero-heap-allocation formatting & hit-testing primitives
// ---------------------------------------------------------------------------

/// Fixed-capacity UTF-8 stack buffer for zero-allocation per-frame formatting.
struct StackBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackBuf<N> {
    #[inline]
    const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    #[inline]
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for StackBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            let mut tmp = [0u8; 4];
            let enc = ch.encode_utf8(&mut tmp).as_bytes();
            if self.len + enc.len() > N {
                break;
            }
            self.buf[self.len..self.len + enc.len()].copy_from_slice(enc);
            self.len += enc.len();
        }
        Ok(())
    }
}

/// Stack-allocated list of interactive rectangles returned by campaign screens.
///
/// Later entries take precedence on hit-testing (`at`), matching `menu::HitMap`.
#[derive(Clone, Copy, Debug)]
pub struct HitList<A: Copy, const N: usize> {
    entries: [Option<(Rect, A)>; N],
    len: usize,
}

impl<A: Copy, const N: usize> Default for HitList<A, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Copy, const N: usize> HitList<A, N> {
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            len: 0,
        }
    }

    pub fn push(&mut self, rect: Rect, action: A) {
        if rect.is_empty() || self.len >= N {
            return;
        }
        self.entries[self.len] = Some((rect, action));
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the topmost action covering cell `(x, y)`, if any.
    pub fn at(&self, x: u16, y: u16) -> Option<A> {
        self.entries[..self.len]
            .iter()
            .rev()
            .flatten()
            .find(|(r, _)| r.contains((x, y).into()))
            .map(|(_, a)| *a)
    }

    /// Iterate all registered `(Rect, Action)` pairs in registration order.
    pub fn iter(&self) -> impl Iterator<Item = (Rect, A)> + '_ {
        self.entries[..self.len].iter().flatten().copied()
    }

    /// Find the first registered rectangle matching `action`.
    pub fn find(&self, action: A) -> Option<Rect>
    where
        A: PartialEq,
    {
        self.iter().find(|(_, a)| *a == action).map(|(r, _)| r)
    }
}

// ---------------------------------------------------------------------------
// Public shared campaign view types (zero simulation dependencies)
// ---------------------------------------------------------------------------

/// The three sequential sectors of the Rat Run campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CampaignLevel {
    Sewer = 0,
    Kitchen = 1,
    Lab = 2,
    /// Stage two's side route (`WAVE3_DESIGN.md` §11): the laundry floor.
    Laundry = 3,
    /// Stage three's side route: the printworks.
    Printworks = 4,
}

impl CampaignLevel {
    /// The three campaign **stages**, in order: the original route. The side
    /// route floors are reachable through [`Self::from_theme`] but are not
    /// extra stages, so the sector-select screen and the progress pips stay
    /// three steps long.
    pub const fn all() -> [CampaignLevel; 3] {
        [
            CampaignLevel::Sewer,
            CampaignLevel::Kitchen,
            CampaignLevel::Lab,
        ]
    }

    /// The plate for a floor the player is on (including the side routes).
    pub const fn from_theme(theme: Theme) -> CampaignLevel {
        match theme {
            Theme::Sewer => CampaignLevel::Sewer,
            Theme::Kitchen => CampaignLevel::Kitchen,
            Theme::Lab => CampaignLevel::Lab,
            Theme::Laundry => CampaignLevel::Laundry,
            Theme::Printworks => CampaignLevel::Printworks,
            // The getaway ride has no campaign plate of its own: it is a
            // last-stage practice floor and borrows the Lab's (as its boss
            // bar does, see `boss::trailer_boss`).
            Theme::Trailer => CampaignLevel::Lab,
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Convert a 0-based campaign level index (`0 => Sewer`, `1 => Kitchen`, `2+ => Lab`).
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => CampaignLevel::Sewer,
            1 => CampaignLevel::Kitchen,
            2 => CampaignLevel::Lab,
            3 => CampaignLevel::Laundry,
            _ => CampaignLevel::Printworks,
        }
    }

    /// 1-based campaign level number (`1`, `2`, or `3`).
    pub const fn number(self) -> u8 {
        match self {
            CampaignLevel::Sewer => 1,
            CampaignLevel::Kitchen => 2,
            CampaignLevel::Lab => 3,
            CampaignLevel::Laundry => 2,
            CampaignLevel::Printworks => 3,
        }
    }

    pub const fn next(self) -> Option<CampaignLevel> {
        match self {
            CampaignLevel::Sewer => Some(CampaignLevel::Kitchen),
            CampaignLevel::Kitchen => Some(CampaignLevel::Lab),
            CampaignLevel::Lab => None,
            CampaignLevel::Laundry => Some(CampaignLevel::Lab),
            CampaignLevel::Printworks => Some(CampaignLevel::Printworks),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "THE SEWERS",
            CampaignLevel::Kitchen => "THE KITCHENS",
            CampaignLevel::Lab => "THE LAB",
            CampaignLevel::Laundry => "THE LAUNDRY",
            CampaignLevel::Printworks => "THE PRINTWORKS",
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "Sewer",
            CampaignLevel::Kitchen => "Kitchen",
            CampaignLevel::Lab => "Lab",
            CampaignLevel::Laundry => "Laundry",
            CampaignLevel::Printworks => "Printworks",
        }
    }

    pub const fn sector_code(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "SECTOR 01",
            CampaignLevel::Kitchen => "SECTOR 02",
            CampaignLevel::Lab => "SECTOR 03",
            CampaignLevel::Laundry => "SECTOR 02B",
            CampaignLevel::Printworks => "SECTOR 03B",
        }
    }

    pub const fn boss_name(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "APEX JAGUAR",
            CampaignLevel::Kitchen => "THE SWEEPER",
            CampaignLevel::Lab => "TURF-REAPER MK.IV",
            // The floors' own bosses (`WAVE3_DESIGN.md` §12). The Printworks
            // fights the Night Edition; the Laundry still borrows the broom
            // (`BossKind::for_theme`) until the Mangle lands, so its *fight* is
            // the placeholder and the plate is where the floor is going.
            CampaignLevel::Laundry => "THE MANGLE",
            CampaignLevel::Printworks => "THE NIGHT EDITION",
        }
    }

    pub const fn boss_verdict(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => "APEX JAGUAR  —  DOWN",
            CampaignLevel::Kitchen => "THE SWEEPER  —  ROUTED",
            CampaignLevel::Lab => "TURF-REAPER MK.IV  —  SCRAPPED",
            CampaignLevel::Laundry => "THE MANGLE  —  JAMMED",
            CampaignLevel::Printworks => "THE NIGHT EDITION  —  STOPPED",
        }
    }

    pub const fn boss_triumph_line(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => {
                "The big cat overshot its last pounce and skidded into the drain. Quiet returns to the pipes — climb toward the heat of the kitchens."
            }
            CampaignLevel::Kitchen => {
                "One ankle bite too many. The broom clatters across the linoleum and two huge slippers hop straight out the pantry door."
            }
            CampaignLevel::Lab => {
                "Belly-up on the tiles, rotary blades spinning down to silence. The emergency STOP flashes dark. The blast door ahead stands unlatched."
            }
            CampaignLevel::Laundry => {
                "For the first time in thirty years, the laundry stops. The drum ticks as it cools, and the service stair beyond it is unlatched."
            }
            CampaignLevel::Printworks => {
                "The last edition rolls off the belt, curls under the street door, and you are out with it."
            }
        }
    }

    pub const fn default_intro_flavor(self) -> &'static str {
        match self {
            CampaignLevel::Sewer => {
                "Cold pipes and colder things. The drainage threads under the whole facility. Follow the current up and out."
            }
            CampaignLevel::Kitchen => {
                "Grease, snap-traps, and the stink of bait. Everything here wants you fed — to something with a bigger mouth."
            }
            CampaignLevel::Lab => {
                "Where you were made. The lights hum a number that used to be your name. End it here. Walk out the front door."
            }
            CampaignLevel::Laundry => {
                "Indigo tile, steam between the machines, and a hundred doors that will not open. The whites come out pink down here."
            }
            CampaignLevel::Printworks => {
                "Ink in the seams of your paws and the presses still running overnight. Tomorrow's front page says you were caught."
            }
        }
    }

    /// Facility material tint matching `menu.rs::material`.
    pub const fn material_rgb(self) -> Rgb {
        match self {
            CampaignLevel::Sewer => (144, 184, 173),
            CampaignLevel::Kitchen => (213, 173, 122),
            CampaignLevel::Lab => (151, 175, 214),
            CampaignLevel::Laundry => (159, 200, 200),
            CampaignLevel::Printworks => (182, 180, 165),
        }
    }

    pub fn accent(self) -> Color {
        to_color(self.material_rgb())
    }
}

/// Whether an upgrade entry is a leveled weapon or a stacked passive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeCategory {
    Weapon { level: u8, max_level: u8 },
    Passive { stacks: u8 },
}

/// One owned upgrade presented on the Keep Three manifest or Level Intro pack strip.
#[derive(Clone, Copy, Debug)]
pub struct UpgradeEntry<'a> {
    /// Display name, e.g. `"Gnaw"`, `"Cheese Spray"`, `"Thick Hide"`.
    pub name: &'a str,
    /// Single-cell geometric glyph (e.g. `'▸'`, `'»'`, `'✦'`, `'◎'`, `'◈'`, `'♦'`, `'+'`).
    pub glyph: char,
    /// Accent RGB tint for the icon and level pips.
    pub color: Rgb,
    /// Weapon level/max or passive stack count.
    pub category: UpgradeCategory,
    /// Short mechanical summary shown on cards / detail strip.
    pub detail: &'a str,
}

// ---------------------------------------------------------------------------
// Screen 1: Level Complete
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelCompleteAction {
    /// Proceed to Keep Three (after Sewer/Kitchen) or Campaign Victory (after Lab).
    Continue,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelCompleteView<'a> {
    pub level: CampaignLevel,
    /// Time spent on this level in seconds.
    pub time_secs: f32,
    /// Kills scored on this level.
    pub kills: u32,
    /// Cumulative campaign score so far (including this level).
    pub score_so_far: u32,
    pub gold: u32,
    /// Score earned on this level alone.
    pub level_score: u32,
    /// Player level reached by the end of this stage.
    pub player_level: u32,
    /// Total upgrades currently owned (shown as a teaser for Keep Three).
    pub owned_upgrades: usize,
    /// Optional override for the per-boss triumph text.
    pub custom_triumph: Option<&'a str>,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelCompleteAction>,
}

// ---------------------------------------------------------------------------
// Screen 2: Keep Three (Escape Pack Manifest)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepThreeAction {
    /// Toggle whether `items[index]` is packed for the next level.
    ToggleItem(usize),
    /// Scroll the manifest viewport up.
    ScrollUp,
    /// Scroll the manifest viewport down.
    ScrollDown,
    /// Confirm the chosen pack and descend/ascend into the next level.
    /// Emitted in [`HitList`] only when [`keep_three_can_confirm`] is `true`.
    Confirm,
    /// Pick the route floor at this index (`0..2`) for the level being packed
    /// for (`WAVE3_DESIGN.md` §1: Kitchen or Laundry, then Lab or Printworks).
    Route(u8),
}

/// One route chip on the keep screen: the floor's name and whether it is the
/// chosen one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteChip<'a> {
    pub name: &'a str,
    pub chosen: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct KeepThreeView<'a> {
    /// Sector just cleared (unchosen upgrades stay behind here).
    pub from_level: CampaignLevel,
    /// Next sector the player is packing for.
    pub next_level: CampaignLevel,
    pub gold: u32,
    /// All upgrades owned at the end of `from_level` (up to ~13 entries or more).
    pub items: &'a [UpgradeEntry<'a>],
    /// Parallel selection flags (`selected.get(i).copied().unwrap_or(false)`).
    pub selected: &'a [bool],
    /// Target number of upgrades to carry (normally `3`).
    pub max_keep: usize,
    /// Currently focused keyboard cursor index in `items`.
    pub cursor: usize,
    /// Top visible item index (clamped by the renderer to valid scroll range).
    pub scroll: usize,
    /// Currently hovered mouse target.
    pub hovered: Option<KeepThreeAction>,
    /// The two floors the next level may be played on (original route first),
    /// or `[None, None]` when the next level has no choice (stage one, or the
    /// campaign is over).
    pub route: [Option<RouteChip<'a>>; 2],
}

// ---------------------------------------------------------------------------
// Screen 3: Level Intro (Levels 2 & 3; consistent with menu::draw_map_intro)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelIntroAction {
    Begin,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelIntroView<'a> {
    /// Level being entered (`Kitchen` or `Lab`; also handles `Sewer`).
    pub level: CampaignLevel,
    /// Optional flavor line override (`None` uses the built-in sector flavor).
    pub flavor: Option<&'a str>,
    /// Upgrades carried in from the previous level (0..=3).
    pub carried: &'a [UpgradeEntry<'a>],
    /// Cumulative campaign score entering this level.
    pub score_so_far: u32,
    pub gold: u32,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelIntroAction>,
}

// ---------------------------------------------------------------------------
// Screen 4: Campaign Victory
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampaignVictoryAction {
    PlayAgain,
    Menu,
}

/// Per-level metrics displayed in the Campaign Victory breakdown table.
#[derive(Clone, Copy, Debug)]
pub struct LevelBreakdown {
    pub level: CampaignLevel,
    pub time_secs: f32,
    pub kills: u32,
    pub score: u32,
    pub player_level: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct CampaignVictoryView<'a> {
    pub final_score: u32,
    pub total_time_secs: f32,
    pub total_kills: u32,
    pub gold: u32,
    pub new_best: bool,
    pub levels: [LevelBreakdown; 3],
    /// Final upgrades carried out the front door (rendered when space permits).
    pub final_loadout: &'a [UpgradeEntry<'a>],
    /// Optional custom finale copy.
    pub custom_blurb: Option<&'a str>,
    /// Currently keyboard-focused action button.
    pub focus: CampaignVictoryAction,
    /// Currently pointer-hovered action button.
    pub hovered: Option<CampaignVictoryAction>,
}

// ---------------------------------------------------------------------------
// Screen 5: Title Level Select
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelSelectAction {
    /// Select an unlocked level slot.
    SelectLevel(CampaignLevel),
    /// Begin run at the currently selected unlocked level.
    Start,
    /// Return to title menu.
    Back,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LevelBest {
    pub score: u32,
    pub best_time_secs: f32,
    pub cleared: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelSlotView {
    pub level: CampaignLevel,
    pub unlocked: bool,
    pub best: Option<LevelBest>,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelSelectView {
    pub slots: [LevelSlotView; 3],
    /// Currently selected slot index (`0..3`).
    pub selected: usize,
    /// Currently hovered mouse action.
    pub hovered: Option<LevelSelectAction>,
}

// ---------------------------------------------------------------------------
// Component 6: Compact Campaign Progress Pips (HUD / Pause)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct CampaignProgressView {
    /// Active campaign level (`1..=3`).
    pub current: CampaignLevel,
    /// Number of levels already cleared (`0..=3`).
    pub cleared_count: u8,
    /// Optional background fill color (defaults to `PLATE`).
    pub bg: Option<Color>,
}

// ---------------------------------------------------------------------------
// Pure helper functions (geometry & state logic shared with callers)
// ---------------------------------------------------------------------------

/// Required number of selections before Keep Three confirm becomes enabled.
pub fn keep_three_required(view: &KeepThreeView<'_>) -> usize {
    view.max_keep.min(view.items.len())
}

/// Count how many valid indices in `view.items` are currently marked selected.
pub fn keep_three_selected_count(view: &KeepThreeView<'_>) -> usize {
    view.items
        .iter()
        .enumerate()
        .filter(|(i, _)| view.selected.get(*i).copied().unwrap_or(false))
        .count()
}

/// Whether the Keep Three confirm button is enabled (`selected == min(max_keep, items.len())`).
pub fn keep_three_can_confirm(view: &KeepThreeView<'_>) -> bool {
    keep_three_selected_count(view) == keep_three_required(view)
}

#[derive(Clone, Copy)]
struct KeepThreeLayout {
    panel_rect: Rect,
    two_pane: bool,
    row_height: u16,
    visible_items: usize,
}

fn keep_three_layout(area: Rect) -> KeepThreeLayout {
    let two_pane = area.width >= 104 && area.height >= 26;
    let (pw, ph) = if two_pane {
        (area.width.min(108), area.height.min(36))
    } else if area.width >= 76 && area.height >= 23 {
        (area.width.min(78), area.height.min(30))
    } else {
        (area.width.min(74), area.height.min(24))
    };
    let panel_rect = centered_rect(pw, ph, area);
    let inner_h = panel_rect.height.saturating_sub(2);
    let row_height: u16 = if two_pane || (inner_h >= 21 && panel_rect.width >= 68) {
        2
    } else {
        1
    };
    let chrome_rows: u16 = if two_pane {
        // header (2) + rule (1) + footer hint (1)
        4
    } else if row_height == 2 {
        // top pack strip (3) + rule (1) + footer rule + confirm + hint (3)
        7
    } else {
        // compact: top pack counter (1) + inline slots (1) + rule (1) + footer rule + detail + confirm (3)
        6
    };
    let list_h = inner_h.saturating_sub(chrome_rows);
    let visible_items = usize::from(list_h / row_height.max(1)).max(1);
    KeepThreeLayout {
        panel_rect,
        two_pane,
        row_height,
        visible_items,
    }
}

/// Number of upgrade items simultaneously visible on Keep Three for `area`.
///
/// Callers can use this to keep `KeepThreeView::scroll` synchronized when
/// navigating with keyboard up/down arrows.
pub fn keep_three_visible_rows(area: Rect) -> usize {
    if area.width < 12 || area.height < 8 {
        return 1;
    }
    keep_three_layout(area).visible_items
}

/// Recommended column width for [`draw_campaign_pips`].
pub const fn campaign_pips_width(compact: bool) -> u16 {
    if compact {
        9
    } else {
        24
    }
}

// ---------------------------------------------------------------------------
// Zero-allocation buffer drawing primitives
// ---------------------------------------------------------------------------

#[inline]
fn ink(c: Color) -> Style {
    Style::default().fg(c)
}

#[inline]
fn heavy(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

#[inline]
fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

#[inline]
fn clamp_rect(r: Rect, bounds: Rect) -> Rect {
    r.intersection(bounds)
}

fn fill_rect(buf: &mut Buffer, rect: Rect, ch: char, style: Style) {
    let r = rect.intersection(buf.area);
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}

fn hline(buf: &mut Buffer, x: u16, y: u16, width: u16, ch: char, style: Style) {
    if y < buf.area.y || y >= buf.area.bottom() || width == 0 {
        return;
    }
    let start = x.max(buf.area.x);
    let end = x.saturating_add(width).min(buf.area.right());
    for cx in start..end {
        buf[(cx, y)].set_char(ch).set_style(style);
    }
}

fn vline(buf: &mut Buffer, x: u16, y: u16, height: u16, ch: char, style: Style) {
    if x < buf.area.x || x >= buf.area.right() || height == 0 {
        return;
    }
    let start = y.max(buf.area.y);
    let end = y.saturating_add(height).min(buf.area.bottom());
    for cy in start..end {
        buf[(x, cy)].set_char(ch).set_style(style);
    }
}

/// Write a single-line string clipped to `max_w` cells and `buf.area`.
/// Returns the next `x` column after the last written character.
fn put_str(buf: &mut Buffer, x: u16, y: u16, s: &str, max_w: u16, style: Style) -> u16 {
    if y < buf.area.y || y >= buf.area.bottom() || max_w == 0 || x >= buf.area.right() {
        return x;
    }
    let limit = x.saturating_add(max_w).min(buf.area.right());
    let mut cx = x;
    for ch in s.chars() {
        if cx >= limit {
            break;
        }
        if cx >= buf.area.x {
            buf[(cx, y)].set_char(ch).set_style(style);
        }
        cx = cx.saturating_add(1);
    }
    cx
}

/// Write a string truncated with `'…'` (`'.'`? no, single-cell `'.'` or `'·'` or `..` — wait,
/// let's use single-cell `'~'` or `".."`? Actually `'…'` U+2026 is East Asian Ambiguous width 1,
/// even safer: truncate cleanly or use `".."` when tight!).
fn put_str_ellipsis(buf: &mut Buffer, x: u16, y: u16, s: &str, max_w: u16, style: Style) -> u16 {
    if max_w == 0 {
        return x;
    }
    let count = s.chars().count();
    if count <= usize::from(max_w) {
        return put_str(buf, x, y, s, max_w, style);
    }
    if max_w <= 2 {
        return put_str(buf, x, y, s, max_w, style);
    }
    let keep = max_w - 2;
    let cx = put_str(buf, x, y, s, keep, style);
    put_str(buf, cx, y, "..", 2, style)
}

fn put_centered(buf: &mut Buffer, area: Rect, row: u16, s: &str, style: Style) -> Rect {
    if row >= area.height || area.width == 0 {
        return Rect::default();
    }
    let y = area.y + row;
    let len = (s.chars().count() as u16).min(area.width);
    let x = area.x + (area.width - len) / 2;
    put_str(buf, x, y, s, len, style);
    clamp_rect(Rect::new(x, y, len, 1), buf.area)
}

/// Zero-allocation word-wrapped paragraph renderer. Returns number of lines drawn.
fn put_wrapped(
    buf: &mut Buffer,
    area: Rect,
    text: &str,
    style: Style,
    centered: bool,
) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let max_w = usize::from(area.width);
    let mut rest = text.trim();
    let mut row: u16 = 0;

    while !rest.is_empty() && row < area.height {
        let total_chars = rest.chars().count();
        let (line, next) = if total_chars <= max_w {
            (rest, "")
        } else {
            let edge = rest
                .char_indices()
                .nth(max_w)
                .map_or(rest.len(), |(idx, _)| idx);
            let cut = rest[..edge]
                .rfind(' ')
                .filter(|&idx| idx > 0)
                .unwrap_or(edge);
            (&rest[..cut], rest[cut..].trim_start())
        };
        let line = line.trim_end();
        let lw = (line.chars().count() as u16).min(area.width);
        let x = if centered {
            area.x + (area.width - lw) / 2
        } else {
            area.x
        };
        put_str(buf, x, area.y + row, line, lw, style);
        rest = next;
        row += 1;
    }
    row
}

/// Stamped metal plate matching `menu::panel`: dark `PLATE` fill, `TRACK` right/bottom
/// edges, lit `edge` accent along top and left edges, bold stamped caption.
fn panel(
    buf: &mut Buffer,
    rect: Rect,
    caption: &str,
    right_caption: Option<&str>,
    edge: Color,
) -> Rect {
    let r = rect.intersection(buf.area);
    if r.is_empty() {
        return Rect::default();
    }
    fill_rect(buf, r, ' ', ink(COAT).bg(PLATE));
    if r.width < 2 || r.height < 2 {
        return r;
    }

    let lit = ink(edge).bg(PLATE);
    let dim = ink(TRACK).bg(PLATE);
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;

    buf[(x0, y0)].set_char('┌').set_style(lit);
    buf[(x1, y0)].set_char('┐').set_style(lit);
    buf[(x0, y1)].set_char('└').set_style(lit);
    buf[(x1, y1)].set_char('┘').set_style(dim);

    hline(buf, x0 + 1, y0, r.width - 2, '─', lit);
    hline(buf, x0 + 1, y1, r.width - 2, '─', dim);
    vline(buf, x0, y0 + 1, r.height - 2, '│', lit);
    vline(buf, x1, y0 + 1, r.height - 2, '│', dim);

    if r.width > 5 && !caption.is_empty() {
        put_str(buf, x0 + 2, y0, caption, r.width - 4, heavy(edge).bg(PLATE));
    }
    if let Some(rc) = right_caption {
        let rw = rc.chars().count() as u16;
        let cap_end = x0 + 2 + caption.chars().count() as u16 + 2;
        if rw > 0 && x1 > cap_end + rw + 1 {
            put_str(buf, x1 - rw - 1, y0, rc, rw, ink(MUTED).bg(PLATE));
        }
    }

    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

/// Bordered sub-card inside a panel.
fn sub_box(buf: &mut Buffer, rect: Rect, border_col: Color, bg: Color) -> Rect {
    let r = rect.intersection(buf.area);
    if r.is_empty() {
        return Rect::default();
    }
    fill_rect(buf, r, ' ', ink(COAT).bg(bg));
    if r.width < 2 || r.height < 2 {
        return r;
    }
    let bs = ink(border_col).bg(bg);
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;
    buf[(x0, y0)].set_char('┌').set_style(bs);
    buf[(x1, y0)].set_char('┐').set_style(bs);
    buf[(x0, y1)].set_char('└').set_style(bs);
    buf[(x1, y1)].set_char('┘').set_style(bs);
    hline(buf, x0 + 1, y0, r.width - 2, '─', bs);
    hline(buf, x0 + 1, y1, r.width - 2, '─', bs);
    vline(buf, x0, y0 + 1, r.height - 2, '│', bs);
    vline(buf, x1, y0 + 1, r.height - 2, '│', bs);
    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonState {
    Normal,
    Primary,
    Focused,
    Hovered,
    Disabled,
}

fn button_style(state: ButtonState, accent: Color) -> Style {
    match state {
        ButtonState::Normal => ink(COAT).bg(TRACK),
        ButtonState::Primary => heavy(PLATE).bg(accent),
        ButtonState::Focused => heavy(PLATE).bg(accent),
        ButtonState::Hovered => heavy(PLATE).bg(TAIL),
        ButtonState::Disabled => ink(MUTED).bg(PLATE_SUB),
    }
}

fn draw_button_at(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    label: &str,
    max_w: u16,
    state: ButtonState,
    accent: Color,
) -> Rect {
    if max_w == 0 || y < buf.area.y || y >= buf.area.bottom() || x >= buf.area.right() {
        return Rect::default();
    }
    let w = (label.chars().count() as u16).min(max_w);
    let style = button_style(state, accent);
    put_str(buf, x, y, label, w, style);
    clamp_rect(Rect::new(x, y, w, 1), buf.area)
}

fn fmt_time(secs: f32) -> StackBuf<12> {
    let mut b = StackBuf::<12>::new();
    let total = secs.max(0.0) as u32;
    let _ = write!(b, "{:02}:{:02}", total / 60, total % 60);
    b
}

fn fmt_pips(level: u8, max_level: u8) -> StackBuf<24> {
    let mut b = StackBuf::<24>::new();
    let max_c = max_level.clamp(1, 6);
    for i in 1..=max_c {
        let _ = b.write_char(if i <= level { '●' } else { '○' });
    }
    b
}

/// Draw inline pips into a panel's top-right border (only if it won't collide with `caption`).
fn stamp_header_pips(
    buf: &mut Buffer,
    panel_rect: Rect,
    caption: &str,
    current: CampaignLevel,
    cleared_count: u8,
) {
    if panel_rect.width < 46 || panel_rect.height < 2 {
        return;
    }
    let y = panel_rect.y;
    let pips_w: u16 = 14;
    let x = panel_rect.right().saturating_sub(pips_w + 1);
    let cap_end = panel_rect.x + 2 + caption.chars().count() as u16 + 2;
    if x <= cap_end {
        return;
    }
    let base = ink(MUTED).bg(PLATE);
    let mut cx = put_str(buf, x, y, " LV ", 4, base);
    let mut nb = StackBuf::<8>::new();
    let _ = write!(nb, "{}/3 ", current.number());
    cx = put_str(buf, cx, y, nb.as_str(), 4, heavy(COAT).bg(PLATE));
    for (idx, lvl) in CampaignLevel::all().into_iter().enumerate() {
        let (ch, col) = if (idx as u8) < cleared_count {
            ('●', HEAL_MINT)
        } else if lvl == current {
            ('◆', lvl.accent())
        } else {
            ('○', EDGE)
        };
        let mut g = [0u8; 4];
        cx = put_str(buf, cx, y, ch.encode_utf8(&mut g), 1, heavy(col).bg(PLATE));
        if idx < 2 {
            cx = put_str(buf, cx, y, " ", 1, base);
        }
    }
    let _ = put_str(buf, cx, y, " ", 1, base);
}

// ---------------------------------------------------------------------------
// 1. Level Complete Screen
// ---------------------------------------------------------------------------

fn draw_boss_triumph_emblem(buf: &mut Buffer, area: Rect, level: CampaignLevel, y: u16) {
    match level {
        CampaignLevel::Sewer => {
            // Apex Jaguar: pointed ears + extinguished × eyes + claw slash over drain pipe.
            let rows = [
                ("   ▄▀▄  · · ·  ▄▀▄   ", TRAIL_GOLD),
                ("  ▐█  × ──┬── ×  █▌  ", WARN_AMBER),
                ("══╧═══╲═══╧═══╱═══╧══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Kitchen => {
            // The Sweeper: snapped broom handle + splayed bristles + overturned slipper.
            let rows = [
                ("    ╲   ╭───╮   ▄▀▀▀▄   ", WARN_AMBER),
                ("     ╲  │▓▓▓│  ▐█████▌  ", TRAIL_GOLD),
                ("══════╲═╧═┴═╧═══▀▀▀▀▀═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Laundry => {
            // Placeholder: the Laundry's fight is the borrowed broom until the
            // Mangle lands, so its triumph plate is the broom's.
            let rows = [
                ("    ╲   ╭───╮   ▄▀▀▀▄   ", WARN_AMBER),
                ("     ╲  │▓▓▓│  ▐█████▌  ", TRAIL_GOLD),
                ("══════╲═╧═┴═╧═══▀▀▀▀▀═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Printworks => {
            // Placeholder: the Printworks' fight is the borrowed mower.
            let rows = [
                ("  ▄█▄  ▀▀▀▀▀▀▀▀▀▀  ▄█▄  ", WARN_AMBER),
                ("  ▐█▌  [  STOP  ]  ▐█▌  ", DANGER),
                ("═══╧═══╲════════╱═══╧═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
        CampaignLevel::Lab => {
            // TURF-REAPER MK.IV: flipped mower shell belly-up + dead STOP button + treads.
            let rows = [
                ("  ▄█▄  ▀▀▀▀▀▀▀▀▀▀  ▄█▄  ", WARN_AMBER),
                ("  ▐█▌  [  STOP  ]  ▐█▌  ", DANGER),
                ("═══╧═══╲════════╱═══╧═══", level.accent()),
            ];
            for (i, (line, col)) in rows.iter().enumerate() {
                if y + i as u16 >= area.height {
                    break;
                }
                put_centered(buf, area, y + i as u16, line, ink(*col).bg(PLATE));
            }
        }
    }
}

/// Draw the Level Complete triumph screen after a sector boss is defeated.
///
/// Returns a [`HitList`] containing [`LevelCompleteAction::Continue`] mapped to
/// the rendered primary continue button.
pub fn draw_level_complete(
    f: &mut Frame,
    area: Rect,
    view: &LevelCompleteView<'_>,
) -> HitList<LevelCompleteAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let accent = view.level.accent();
    let compact = area.width < 66 || area.height < 22;
    let pw = if compact {
        area.width.min(64)
    } else {
        area.width.min(72)
    };
    let ph = if area.height < 20 {
        area.height
    } else if area.height < 24 {
        area.height.min(20)
    } else {
        area.height.min(24)
    };
    let card = centered_rect(pw, ph, area);

    let mut cap = StackBuf::<48>::new();
    let _ = write!(
        cap,
        " SECTOR CLEARED   /   {} ",
        view.level.name()
    );
    let buf = f.buffer_mut();
    let inner = panel(buf, card, cap.as_str(), None, VICTORY_MINT);
    stamp_header_pips(buf, card, cap.as_str(), view.level, view.level.number());

    if inner.width < 10 || inner.height < 4 {
        return hits;
    }

    let mut y: u16 = if inner.height >= 19 { 1 } else { 0 };

    // Triumph status pill (boss-specific verdict).
    let mut badge = StackBuf::<64>::new();
    if inner.width >= 42 {
        let _ = write!(badge, " ▰▰  {}  ▰▰ ", view.level.boss_verdict());
    } else {
        let _ = write!(badge, " ▸ {} ", view.level.boss_verdict());
    }
    put_centered(
        buf,
        inner,
        y,
        badge.as_str(),
        heavy(PLATE).bg(VICTORY_MINT),
    );
    y += if inner.height >= 18 { 2 } else { 1 };

    // Boss-specific geometric triumph emblem (when vertical space allows).
    if inner.height >= 16 && inner.width >= 30 {
        draw_boss_triumph_emblem(buf, inner, view.level, y);
        y += 4;
    }

    // Narrative beat specific to the downed boss.
    let triumph = view
        .custom_triumph
        .unwrap_or_else(|| view.level.boss_triumph_line());
    let text_inset = if inner.width >= 40 { 3 } else { 1 };
    let text_rect = Rect::new(
        inner.x + text_inset,
        inner.y + y,
        inner.width.saturating_sub(text_inset * 2),
        inner.height.saturating_sub(y + 6).clamp(1, 3),
    );
    let drawn_lines = put_wrapped(buf, text_rect, triumph, ink(COAT).bg(PLATE), true);
    y += drawn_lines + u16::from(inner.height >= 16);

    // Horizontal divider.
    if y + 4 < inner.height {
        hline(
            buf,
            inner.x + 2,
            inner.y + y,
            inner.width.saturating_sub(4),
            '─',
            ink(TRACK).bg(PLATE),
        );
        y += 1;
    }

    // Level stats strip: Time, Kills, Score so far (plus Level Score / Rat Lv on wider layouts).
    let tbuf = fmt_time(view.time_secs);
    if inner.width >= 54 && y + 3 < inner.height {
        let cols = 4u16;
        let col_w = inner.width.saturating_sub(4) / cols;
        let x0 = inner.x + 2;

        let mut kills_b = StackBuf::<16>::new();
        let _ = write!(kills_b, "{}", view.kills);
        let mut score_b = StackBuf::<24>::new();
        let _ = write!(score_b, "{}", view.score_so_far);

        let mut gold_b = StackBuf::<16>::new();
        let _ = write!(gold_b, "{}", view.gold);

        let stat_items: [(&str, &str, Color); 4] = [
            ("TIME", tbuf.as_str(), COAT),
            ("KILLS", kills_b.as_str(), COAT),
            ("SCORE SO FAR", score_b.as_str(), TRAIL_GOLD),
            ("GOLD", gold_b.as_str(), TRAIL_GOLD),
        ];
        for (i, (lbl, val, vcol)) in stat_items.iter().enumerate() {
            let cx = x0 + i as u16 * col_w;
            let cell = Rect::new(cx, inner.y + y, col_w, 2);
            put_centered(buf, cell, 0, lbl, ink(MUTED).bg(PLATE));
            put_centered(buf, cell, 1, val, heavy(*vcol).bg(PLATE));
            if i + 1 < stat_items.len() {
                vline(buf, cx + col_w.saturating_sub(1), inner.y + y, 2, '│', ink(TRACK).bg(PLATE));
            }
        }
        y += 2;
        if y + 3 < inner.height {
            let mut sub = StackBuf::<64>::new();
            if inner.width >= 60 {
                let _ = write!(
                    sub,
                    "Stage score +{}   ·   Subject Lv {}   ·   {} upgrades owned",
                    view.level_score, view.player_level, view.owned_upgrades
                );
            } else {
                let _ = write!(
                    sub,
                    "Stage +{}  ·  Lv {}  ·  {} upgrades",
                    view.level_score, view.player_level, view.owned_upgrades
                );
            }
            put_centered(buf, inner, y, sub.as_str(), ink(MUTED).bg(PLATE));
            y += 1;
        }
    } else if y + 2 < inner.height {
        let mut line = StackBuf::<64>::new();
        let _ = write!(
            line,
            "{} · {} kills · Score {} · Gold {}",
            tbuf.as_str(),
            view.kills,
            view.score_so_far,
            view.gold
        );
        put_centered(buf, inner, y, line.as_str(), heavy(COAT).bg(PLATE));
        y += 1;
    }

    // Next-step teaser right above CTA.
    let btn_row = inner.height.saturating_sub(1);
    if y < btn_row.saturating_sub(1) {
        let mut teaser = StackBuf::<72>::new();
        match view.level.next() {
            Some(next) => {
                let keep_n = view.owned_upgrades.min(3);
                if inner.width >= 54 {
                    let _ = write!(
                        teaser,
                        "Next: LEVEL {} — {} · Pack {} of {} to carry",
                        next.number(),
                        next.name(),
                        keep_n,
                        view.owned_upgrades
                    );
                } else {
                    let _ = write!(
                        teaser,
                        "Next: {} · Pack {} of {}",
                        next.short_name(),
                        keep_n,
                        view.owned_upgrades
                    );
                }
            }
            None => {
                let _ = write!(teaser, "All three containment sectors breached");
            }
        }
        put_centered(
            buf,
            inner,
            btn_row.saturating_sub(2),
            teaser.as_str(),
            ink(accent).bg(PLATE),
        );
    }

    // Primary CTA button.
    let cta_label = match view.level.next() {
        Some(_) => {
            if inner.width >= 38 {
                " ▸  Enter / click   PACK 3 UPGRADES "
            } else {
                " ▸ Enter  PACK 3 "
            }
        }
        None => {
            if inner.width >= 38 {
                " ▸  Enter / click   ESCAPE FACILITY "
            } else {
                " ▸ Enter  ESCAPE "
            }
        }
    };
    let btn_state = if view.hovered == Some(LevelCompleteAction::Continue) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let bw = (cta_label.chars().count() as u16).min(inner.width);
    let bx = inner.x + (inner.width - bw) / 2;
    let btn_rect = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        cta_label,
        bw,
        btn_state,
        VICTORY_MINT,
    );
    hits.push(btn_rect, LevelCompleteAction::Continue);

    hits
}

// ---------------------------------------------------------------------------
// 2. Keep Three Screen (Escape Pack Manifest)
// ---------------------------------------------------------------------------

fn draw_pack_slot_pill(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_w: u16,
    slot_num: usize,
    item: Option<&UpgradeEntry<'_>>,
) -> u16 {
    if max_w < 6 {
        return x;
    }
    match item {
        Some(it) => {
            let mut sb = StackBuf::<24>::new();
            let _ = write!(sb, "[{} ", it.glyph);
            // Take first word or up to 6 chars cleanly (avoids awkward "Sharp T×3").
            let short = it.name.split(' ').next().unwrap_or(it.name);
            for ch in short.chars().take(6) {
                let _ = sb.write_char(ch);
            }
            match it.category {
                UpgradeCategory::Weapon { level, max_level } => {
                    if level >= max_level {
                        let _ = sb.write_char('★');
                    } else {
                        let _ = write!(sb, " {}", level);
                    }
                }
                UpgradeCategory::Passive { stacks } => {
                    let _ = write!(sb, "×{}", stacks);
                }
            }
            let _ = sb.write_char(']');
            put_str(
                buf,
                x,
                y,
                sb.as_str(),
                max_w,
                heavy(to_color(it.color)).bg(PLATE_PACKED),
            )
        }
        None => {
            let mut sb = StackBuf::<16>::new();
            let _ = write!(sb, "[{}: --- ]", slot_num + 1);
            put_str(buf, x, y, sb.as_str(), max_w, ink(EDGE).bg(PLATE_DIM))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_manifest_row(
    buf: &mut Buffer,
    row_rect: Rect,
    item: &UpgradeEntry<'_>,
    is_selected: bool,
    is_focused: bool,
    is_hovered: bool,
    pack_full: bool,
    two_line: bool,
) {
    if row_rect.is_empty() {
        return;
    }
    let bg = if is_hovered {
        PLATE_HILITE
    } else if is_selected {
        PLATE_PACKED
    } else if is_focused {
        PLATE_SUB
    } else if pack_full {
        PLATE_DIM
    } else {
        PLATE
    };
    fill_rect(buf, row_rect, ' ', ink(COAT).bg(bg));

    let item_col = to_color(item.color);
    let dim_unselected = pack_full && !is_selected && !is_focused && !is_hovered;

    // Left gutter indicator bar (cursor / selection state).
    let bar_ch = if is_focused || is_hovered {
        '▸'
    } else if is_selected {
        '▌'
    } else {
        ' '
    };
    let bar_col = if is_hovered {
        TAIL
    } else if is_selected {
        HEAL_MINT
    } else if is_focused {
        TRAIL_GOLD
    } else {
        TRACK
    };
    let mut cx = row_rect.x;
    let y0 = row_rect.y;
    let mut tmp = [0u8; 4];
    cx = put_str(buf, cx, y0, bar_ch.encode_utf8(&mut tmp), 1, heavy(bar_col).bg(bg));
    cx = put_str(buf, cx, y0, " ", 1, ink(COAT).bg(bg));

    // Check diamond.
    let chk = if is_selected { "◆ " } else { "◇ " };
    let chk_col = if is_selected {
        HEAL_MINT
    } else if is_focused || is_hovered {
        COAT
    } else {
        EDGE
    };
    cx = put_str(buf, cx, y0, chk, 2, heavy(chk_col).bg(bg));

    // Upgrade icon (4×4 pixel sprite on 2-line rows, single glyph on 1-line compact rows).
    let show_pixel_icon = two_line && row_rect.height >= 2 && row_rect.width >= 38;
    if show_pixel_icon {
        let icon_rect = Rect::new(cx, y0, art_icons::SMALL_CELL_W, art_icons::SMALL_CELL_H);
        art_icons::draw(buf, icon_rect, art_icons::by_name(item.name));
        cx = cx.saturating_add(art_icons::SMALL_CELL_W + 1);
    } else {
        let g_col = if dim_unselected {
            to_color(mix(item.color, (70, 78, 82), 0.6))
        } else {
            item_col
        };
        cx = put_str(buf, cx, y0, item.glyph.encode_utf8(&mut tmp), 1, heavy(g_col).bg(bg));
        cx = put_str(buf, cx, y0, " ", 1, ink(COAT).bg(bg));
    }

    // Right-side status badge (`[KEEP]` / `[LEAVE]`) and level pips/stacks.
    let tag = if is_selected {
        " KEEP "
    } else if pack_full {
        " LEAVE "
    } else {
        "       "
    };
    let tag_w = tag.chars().count() as u16;
    let tag_style = if is_selected {
        heavy(PLATE).bg(HEAL_MINT)
    } else if pack_full {
        ink(EDGE).bg(PLATE_DIM)
    } else {
        ink(EDGE).bg(bg)
    };

    let mut tier = StackBuf::<24>::new();
    let tier_col: Color = match item.category {
        UpgradeCategory::Weapon { level, max_level } => {
            if row_rect.width >= 44 {
                let pips = fmt_pips(level, max_level);
                let _ = write!(tier, "Lv{} {}", level, pips.as_str());
            } else {
                let _ = write!(tier, "Lv{}", level);
            }
            if dim_unselected {
                EDGE
            } else if level >= max_level {
                TRAIL_GOLD
            } else {
                item_col
            }
        }
        UpgradeCategory::Passive { stacks } => {
            let _ = write!(tier, "×{} PASSIVE", stacks);
            if dim_unselected { EDGE } else { HEAL_MINT }
        }
    };
    let tier_str = if row_rect.width < 40 {
        // Shorten passive label on narrow screens.
        if let UpgradeCategory::Passive { stacks } = item.category {
            tier = StackBuf::new();
            let _ = write!(tier, "×{}", stacks);
        }
        tier.as_str()
    } else {
        tier.as_str()
    };
    let tier_w = tier_str.chars().count() as u16;

    let right_reserved = tag_w + tier_w + 2;
    let name_w = row_rect.right().saturating_sub(cx).saturating_sub(right_reserved);
    let name_style = if is_selected || is_focused || is_hovered {
        heavy(COAT).bg(bg)
    } else if dim_unselected {
        ink(MUTED).bg(bg)
    } else {
        ink(COAT).bg(bg)
    };
    put_str_ellipsis(buf, cx, y0, item.name, name_w, name_style);

    // Draw tier right-aligned before badge.
    let tag_x = row_rect.right().saturating_sub(tag_w);
    let tier_x = tag_x.saturating_sub(tier_w + 1);
    if tier_x > cx + 2 {
        put_str(buf, tier_x, y0, tier_str, tier_w, ink(tier_col).bg(bg));
    }
    if tag_x > cx && (is_selected || pack_full) {
        put_str(buf, tag_x, y0, tag, tag_w, tag_style);
    }

    // Second line: mechanical detail + left accent continuation.
    if two_line && row_rect.height >= 2 {
        let y1 = y0 + 1;
        let cont_ch = if is_selected { "▌" } else { " " };
        put_str(buf, row_rect.x, y1, cont_ch, 1, ink(bar_col).bg(bg));
        let detail_x = if show_pixel_icon { cx } else { row_rect.x + 6 };
        let detail_w = row_rect.right().saturating_sub(detail_x + 2);
        let detail_style = if dim_unselected {
            ink(EDGE).bg(bg)
        } else {
            ink(MUTED).bg(bg)
        };
        put_str_ellipsis(buf, detail_x, y1, item.detail, detail_w, detail_style);
    }
}

/// Draw the Keep Three escape-packing screen.
///
/// Players choose `min(view.max_keep, view.items.len())` upgrades to carry into
/// `view.next_level`; everything else is left behind in `view.from_level`.
///
/// Returned [`HitList`] maps:
/// - each visible upgrade row → [`KeepThreeAction::ToggleItem(idx)`]
/// - scroll up/down arrows (when `items.len() > visible`) → `ScrollUp` / `ScrollDown`
/// - confirm button → [`KeepThreeAction::Confirm`] (only emitted when enabled!)
pub fn draw_keep_three(
    f: &mut Frame,
    area: Rect,
    view: &KeepThreeView<'_>,
) -> HitList<KeepThreeAction, 32> {
    let mut hits = HitList::new();
    if area.width < 16 || area.height < 8 {
        return hits;
    }

    let layout = keep_three_layout(area);
    let required = keep_three_required(view);
    let selected_count = keep_three_selected_count(view);
    let can_confirm = selected_count == required;
    let pack_full = required > 0 && selected_count >= required;
    let next_accent = view.next_level.accent();

    // Collect indices of currently selected items (up to 3 for slot previews).
    let mut packed_indices: [Option<usize>; 3] = [None; 3];
    let mut p_len = 0usize;
    for (i, _) in view.items.iter().enumerate() {
        if view.selected.get(i).copied().unwrap_or(false) && p_len < 3 {
            packed_indices[p_len] = Some(i);
            p_len += 1;
        }
    }

    let mut cap = StackBuf::<64>::new();
    if layout.panel_rect.width >= 64 {
        let _ = write!(
            cap,
            " PACK FOR ESCAPE   /   TO {} ",
            view.next_level.name()
        );
    } else if layout.panel_rect.width >= 50 {
        let _ = write!(
            cap,
            " PACK FOR ESCAPE / {} ",
            view.next_level.short_name().to_uppercase()
        );
    } else {
        let _ = write!(cap, " PACK {} OF {} ", required, view.items.len());
    }

    // The purse rides the caption when there is room for it *and* the route
    // chips that follow the caption on the same border; on a narrow panel
    // (57x20) the chips win the header and the purse moves to the bottom
    // border, so neither is ever clipped.
    let mut gold_text = StackBuf::<24>::new();
    let _ = write!(gold_text, " · GOLD {} ", view.gold);
    let chips_w: u16 = view
        .route
        .iter()
        .flatten()
        .map(|chip| chip.name.chars().count() as u16 + 3)
        .sum();
    let gold_w = gold_text.as_str().chars().count() as u16;
    let header_need = 2 + cap.as_str().chars().count() as u16 + gold_w + 3 + chips_w + 3;
    let gold_in_caption =
        chips_w == 0 || layout.panel_rect.width < 40 || header_need <= layout.panel_rect.width;
    if gold_in_caption {
        let _ = write!(cap, "{}", gold_text.as_str());
    }

    let buf = f.buffer_mut();
    let inner = panel(
        buf,
        layout.panel_rect,
        cap.as_str(),
        None,
        if can_confirm { HEAL_MINT } else { TAIL },
    );
    stamp_header_pips(
        buf,
        layout.panel_rect,
        cap.as_str(),
        view.next_level,
        view.from_level.number(),
    );

    if inner.width < 12 || inner.height < 6 {
        return hits;
    }

    // Clamp scroll window.
    let visible = layout.visible_items;
    let max_scroll = view.items.len().saturating_sub(visible);
    let scroll_top = view.scroll.min(max_scroll);
    let needs_scroll = view.items.len() > visible;

    if layout.two_pane {
        // ---- WIDE TWO-PANE LAYOUT (120x40, 200x60) ----
        let right_w: u16 = 38;
        let left_w = inner.width.saturating_sub(right_w + 3);
        let left_x = inner.x + 1;
        let div_x = left_x + left_w + 1;
        let right_x = div_x + 2;

        // Vertical divider between manifest and Escape Pack dossier.
        vline(
            buf,
            div_x,
            inner.y,
            inner.height.saturating_sub(1),
            '│',
            ink(TRACK).bg(PLATE),
        );

        // Left header.
        let mut lhdr = StackBuf::<48>::new();
        let _ = write!(
            lhdr,
            "UPGRADE MANIFEST ({} OWNED)",
            view.items.len()
        );
        put_str(buf, left_x, inner.y, lhdr.as_str(), left_w, heavy(COAT).bg(PLATE));
        put_str(
            buf,
            left_x,
            inner.y + 1,
            "Only what fits in your pack comes through the vent.",
            left_w,
            ink(MUTED).bg(PLATE),
        );
        hline(buf, left_x, inner.y + 2, left_w, '─', ink(TRACK).bg(PLATE));

        // Scrollbar reserve on right side of left pane when overflowing.
        let list_w = if needs_scroll {
            left_w.saturating_sub(2)
        } else {
            left_w
        };
        let list_y = inner.y + 3;
        let list_h = inner.height.saturating_sub(4);

        if view.items.is_empty() {
            put_centered(
                buf,
                Rect::new(left_x, list_y, list_w, list_h),
                1,
                "No upgrades to pack — claws and nerve only.",
                ink(MUTED).bg(PLATE),
            );
        } else {
            for slot in 0..visible {
                let idx = scroll_top + slot;
                if idx >= view.items.len() {
                    break;
                }
                let ry = list_y + slot as u16 * layout.row_height;
                if ry + layout.row_height > list_y + list_h {
                    break;
                }
                let rrect = Rect::new(left_x, ry, list_w, layout.row_height);
                let is_sel = view.selected.get(idx).copied().unwrap_or(false);
                let is_foc = idx == view.cursor;
                let is_hov = view.hovered == Some(KeepThreeAction::ToggleItem(idx));
                draw_manifest_row(
                    buf,
                    rrect,
                    &view.items[idx],
                    is_sel,
                    is_foc,
                    is_hov,
                    pack_full,
                    layout.row_height >= 2,
                );
                hits.push(clamp_rect(rrect, area), KeepThreeAction::ToggleItem(idx));
            }
        }

        // Scroll track + clickable up/down buttons.
        if needs_scroll && list_h >= 3 {
            let sx = left_x + list_w + 1;
            draw_scrollbar(
                buf,
                sx,
                list_y,
                list_h,
                scroll_top,
                max_scroll,
                view.hovered,
                &mut hits,
            );
        }

        // Right pane: ESCAPE PACK dossier.
        let rw = right_w.saturating_sub(1);
        let mut rhdr = StackBuf::<32>::new();
        let _ = write!(rhdr, "ESCAPE PACK   {}/{}", selected_count, required);
        put_str(
            buf,
            right_x,
            inner.y,
            rhdr.as_str(),
            rw,
            heavy(if can_confirm { HEAL_MINT } else { TAIL }).bg(PLATE),
        );

        // Progress pips right-aligned on right header row.
        let mut pips_x = right_x + rw.saturating_sub(7);
        for i in 0..required.min(3) {
            let filled = i < selected_count;
            let s = if filled { "▰ " } else { "▱ " };
            pips_x = put_str(
                buf,
                pips_x,
                inner.y,
                s,
                2,
                heavy(if filled { HEAL_MINT } else { EDGE }).bg(PLATE),
            );
        }

        let sub_status = if can_confirm {
            "Pack sealed. Ready to move."
        } else {
            "Choose upgrades to carry forward."
        };
        put_str(
            buf,
            right_x,
            inner.y + 1,
            sub_status,
            rw,
            ink(if can_confirm { VICTORY_MINT } else { MUTED }).bg(PLATE),
        );
        hline(buf, right_x, inner.y + 2, rw, '─', ink(TRACK).bg(PLATE));

        // 3 physical pack slot cards.
        let mut ry = inner.y + 3;
        let slots_to_draw = required.clamp(1, 3);
        for (s_idx, maybe_item) in packed_indices.iter().enumerate().take(slots_to_draw) {
            if ry + 4 > inner.bottom().saturating_sub(6) {
                break;
            }
            let sbox = Rect::new(right_x, ry, rw, 4);
            if let Some(item_idx) = *maybe_item {
                let it = &view.items[item_idx];
                let icol = to_color(it.color);
                let si = sub_box(buf, sbox, HEAL_MINT, PLATE_PACKED);
                let mut hdr = StackBuf::<24>::new();
                let _ = write!(hdr, " SLOT 0{} ", s_idx + 1);
                put_str(buf, sbox.x + 2, sbox.y, hdr.as_str(), 10, heavy(HEAL_MINT).bg(PLATE_PACKED));
                let text_x = if si.height >= art_icons::SMALL_CELL_H && si.width >= 16 {
                    let icon_rect = Rect::new(
                        si.x + 1,
                        si.y,
                        art_icons::SMALL_CELL_W,
                        art_icons::SMALL_CELL_H,
                    );
                    art_icons::draw(buf, icon_rect, art_icons::by_name(it.name));
                    si.x + 1 + art_icons::SMALL_CELL_W + 1
                } else {
                    let mut g = [0u8; 4];
                    let cx = put_str(buf, si.x + 1, si.y, it.glyph.encode_utf8(&mut g), 1, heavy(icol).bg(PLATE_PACKED));
                    put_str(buf, cx, si.y, " ", 1, ink(COAT).bg(PLATE_PACKED))
                };
                let mut tag = StackBuf::<16>::new();
                match it.category {
                    UpgradeCategory::Weapon { level, max_level } => {
                        let _ = write!(tag, "Lv{}/{}", level, max_level);
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(tag, "×{}", stacks);
                    }
                }
                let tw = tag.as_str().chars().count() as u16;
                let nw = si.right().saturating_sub(text_x + tw + 2);
                put_str_ellipsis(buf, text_x, si.y, it.name, nw, heavy(COAT).bg(PLATE_PACKED));
                put_str(buf, si.right().saturating_sub(tw + 1), si.y, tag.as_str(), tw, heavy(icol).bg(PLATE_PACKED));
                put_str_ellipsis(
                    buf,
                    text_x,
                    si.y + 1,
                    it.detail,
                    si.right().saturating_sub(text_x + 1),
                    ink(MUTED).bg(PLATE_PACKED),
                );
            } else {
                let si = sub_box(buf, sbox, TRACK, PLATE_DIM);
                let mut hdr = StackBuf::<24>::new();
                let _ = write!(hdr, " SLOT 0{} ", s_idx + 1);
                put_str(buf, sbox.x + 2, sbox.y, hdr.as_str(), 10, ink(MUTED).bg(PLATE_DIM));
                put_centered(buf, si, 0, "— empty slot —", ink(MUTED).bg(PLATE_DIM));
                put_centered(buf, si, 1, "Space or click an upgrade", ink(EDGE).bg(PLATE_DIM));
            }
            ry += 4;
        }

        // "LEFT BEHIND" section — reinforcing the emotional weight of sacrifice!
        let leaving = view.items.len().saturating_sub(selected_count);
        let confirm_y = inner.bottom().saturating_sub(3);
        let list_stop_y = confirm_y.saturating_sub(2);
        if ry + 3 <= list_stop_y {
            hline(buf, right_x, ry, rw, '─', ink(TRACK).bg(PLATE));
            ry += 1;
            let mut lb = StackBuf::<48>::new();
            let _ = write!(
                lb,
                "LEAVING IN {} ({})",
                view.from_level.name(),
                leaving
            );
            put_str_ellipsis(buf, right_x, ry, lb.as_str(), rw, heavy(MUTED).bg(PLATE));
            ry += 1;

            let mut shown = 0usize;
            for (i, it) in view.items.iter().enumerate() {
                if view.selected.get(i).copied().unwrap_or(false) {
                    continue;
                }
                if ry >= list_stop_y {
                    let rem = leaving.saturating_sub(shown);
                    if rem > 0 {
                        let mut more = StackBuf::<28>::new();
                        let _ = write!(more, "  + {} more left behind", rem);
                        put_str(buf, right_x, ry, more.as_str(), rw, ink(MUTED).bg(PLATE));
                    }
                    break;
                }
                let mut line = StackBuf::<40>::new();
                let _ = write!(line, "  × {} ", it.name);
                match it.category {
                    UpgradeCategory::Weapon { level, .. } => {
                        let _ = write!(line, "Lv{}", level);
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(line, "×{}", stacks);
                    }
                }
                put_str_ellipsis(buf, right_x, ry, line.as_str(), rw, ink(EDGE).bg(PLATE));
                ry += 1;
                shown += 1;
            }
        }
        hline(buf, right_x, confirm_y.saturating_sub(1), rw, '─', ink(TRACK).bg(PLATE));

        // Big Confirm button in right pane.
        let mut cbuf = StackBuf::<40>::new();
        if can_confirm {
            let _ = write!(cbuf, " ▸  ENTER {}  (Enter) ", view.next_level.name());
        } else {
            let rem = required.saturating_sub(selected_count);
            let _ = write!(cbuf, "  SELECT {} MORE TO CONFIRM  ", rem);
        }
        let cstate = if !can_confirm {
            ButtonState::Disabled
        } else if view.hovered == Some(KeepThreeAction::Confirm) {
            ButtonState::Hovered
        } else {
            ButtonState::Primary
        };
        let cw = (cbuf.as_str().chars().count() as u16).min(rw);
        let cx = right_x + (rw - cw) / 2;
        let crect = draw_button_at(buf, cx, confirm_y, cbuf.as_str(), cw, cstate, next_accent);
        if can_confirm {
            hits.push(crect, KeepThreeAction::Confirm);
        }

        // Bottom key hints across full panel.
        put_centered(
            buf,
            inner,
            inner.height.saturating_sub(1),
            "↑↓ move   ·   Space / click pack or unpack   ·   Enter confirm",
            ink(MUTED).bg(PLATE),
        );
    } else {
        // ---- MEDIUM (80x24) & COMPACT (57x20) SINGLE-COLUMN LAYOUT ----
        let compact = layout.row_height == 1;
        let mut y = inner.y;

        // Top Escape Pack counter & inline slot pills.
        let mut hdr = StackBuf::<32>::new();
        let _ = write!(hdr, "PACK {}/{} ", selected_count, required);
        let mut hx = put_str(
            buf,
            inner.x + 1,
            y,
            hdr.as_str(),
            12,
            heavy(if can_confirm { HEAL_MINT } else { TAIL }).bg(PLATE),
        );
        for i in 0..required.min(3) {
            let filled = i < selected_count;
            hx = put_str(
                buf,
                hx,
                y,
                if filled { "●" } else { "○" },
                1,
                heavy(if filled { HEAL_MINT } else { EDGE }).bg(PLATE),
            );
        }
        let leaving = view.items.len().saturating_sub(selected_count);
        let mut lmsg = StackBuf::<40>::new();
        if compact {
            let _ = write!(lmsg, "  ·  {} left behind", leaving);
        } else {
            let _ = write!(
                lmsg,
                "   ·   Leaving {} behind in {}",
                leaving,
                view.from_level.short_name()
            );
        }
        put_str_ellipsis(
            buf,
            hx,
            y,
            lmsg.as_str(),
            inner.right().saturating_sub(hx + 1),
            ink(MUTED).bg(PLATE),
        );
        y += 1;

        // Second header row: inline 3-slot pills (shown in both medium and compact modes).
        if inner.height >= 12 {
            let mut px = inner.x + 1;
            let slot_gap: u16 = if compact { 1 } else { 2 };
            let slot_w = (inner.width.saturating_sub(2 + slot_gap * 2) / 3).max(8);
            for (s_idx, maybe_idx) in packed_indices.iter().enumerate().take(required.clamp(1, 3))
            {
                let item = maybe_idx.and_then(|idx| view.items.get(idx));
                px = draw_pack_slot_pill(buf, px, y, slot_w, s_idx, item);
                if s_idx < 2 {
                    px = put_str(buf, px, y, "  ", slot_gap, ink(PLATE).bg(PLATE));
                }
            }
            y += 1;
        }

        if !compact {
            // Subtle narrative reminder.
            put_str_ellipsis(
                buf,
                inner.x + 1,
                y,
                "Choose what fits through the vent. Unselected gear stays behind.",
                inner.width.saturating_sub(2),
                ink(MUTED).bg(PLATE),
            );
            y += 1;
        }

        hline(buf, inner.x + 1, y, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
        y += 1;

        // Footer reservation (rule + detail/confirm + hints = 3 rows).
        let footer_rows: u16 = 3;
        let list_y = y;
        let list_bottom = inner.bottom().saturating_sub(footer_rows);
        let list_h = list_bottom.saturating_sub(list_y);
        let list_x = inner.x + 1;
        let list_w = if needs_scroll {
            inner.width.saturating_sub(4)
        } else {
            inner.width.saturating_sub(2)
        };

        if view.items.is_empty() {
            put_centered(
                buf,
                Rect::new(list_x, list_y, list_w, list_h.max(1)),
                0,
                "No upgrades owned — ready to proceed.",
                ink(MUTED).bg(PLATE),
            );
        } else {
            for slot in 0..visible {
                let idx = scroll_top + slot;
                if idx >= view.items.len() {
                    break;
                }
                let ry = list_y + slot as u16 * layout.row_height;
                if ry + layout.row_height > list_bottom {
                    break;
                }
                let rrect = Rect::new(list_x, ry, list_w, layout.row_height);
                let is_sel = view.selected.get(idx).copied().unwrap_or(false);
                let is_foc = idx == view.cursor;
                let is_hov = view.hovered == Some(KeepThreeAction::ToggleItem(idx));
                draw_manifest_row(
                    buf,
                    rrect,
                    &view.items[idx],
                    is_sel,
                    is_foc,
                    is_hov,
                    pack_full,
                    layout.row_height >= 2,
                );
                hits.push(clamp_rect(rrect, area), KeepThreeAction::ToggleItem(idx));
            }
        }

        if needs_scroll && list_h >= 3 {
            let sx = list_x + list_w + 1;
            draw_scrollbar(
                buf,
                sx,
                list_y,
                list_h,
                scroll_top,
                max_scroll,
                view.hovered,
                &mut hits,
            );
        }

        // Compact focused-item detail line (so 1-line mode still shows full description).
        if compact && list_bottom < inner.bottom() {
            hline(buf, inner.x + 1, list_bottom, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
            let detail_y = list_bottom + 1;
            if detail_y < inner.bottom() {
                if let Some(foc) = view.items.get(view.cursor) {
                    let mut db = StackBuf::<72>::new();
                    let _ = write!(db, "{} {}: {}", foc.glyph, foc.name, foc.detail);
                    put_str_ellipsis(
                        buf,
                        inner.x + 1,
                        detail_y,
                        db.as_str(),
                        inner.width.saturating_sub(2),
                        ink(MUTED).bg(PLATE),
                    );
                }
            }
        } else if list_bottom < inner.bottom() {
            hline(buf, inner.x + 1, list_bottom, inner.width.saturating_sub(2), '─', ink(TRACK).bg(PLATE));
        }

        // Bottom confirm button + hints.
        let confirm_row = if compact {
            inner.bottom().saturating_sub(1)
        } else {
            inner.bottom().saturating_sub(2)
        };
        let mut cbuf = StackBuf::<40>::new();
        if can_confirm {
            if inner.width >= 58 {
                let _ = write!(cbuf, " ▸  CONFIRM & ENTER {}  (Enter) ", view.next_level.short_name().to_uppercase());
            } else {
                let _ = write!(cbuf, " ▸ CONFIRM ({}/{}) ", selected_count, required);
            }
        } else {
            let rem = required.saturating_sub(selected_count);
            if inner.width >= 58 {
                let _ = write!(cbuf, "  SELECT {} MORE ({}/{})  ", rem, selected_count, required);
            } else {
                let _ = write!(cbuf, " PICK {} MORE ", rem);
            }
        }
        let cstate = if !can_confirm {
            ButtonState::Disabled
        } else if view.hovered == Some(KeepThreeAction::Confirm) {
            ButtonState::Hovered
        } else {
            ButtonState::Primary
        };

        if compact {
            // Left: hints, Right: confirm button.
            let cw = (cbuf.as_str().chars().count() as u16).min(inner.width.saturating_sub(2));
            let cx = inner.right().saturating_sub(cw + 1);
            let hint_w = cx.saturating_sub(inner.x + 2);
            if hint_w >= 10 {
                put_str_ellipsis(
                    buf,
                    inner.x + 1,
                    confirm_row,
                    "↑↓ nav · Space toggle · Enter",
                    hint_w,
                    ink(MUTED).bg(PLATE),
                );
            }
            let crect = draw_button_at(buf, cx, confirm_row, cbuf.as_str(), cw, cstate, next_accent);
            if can_confirm {
                hits.push(crect, KeepThreeAction::Confirm);
            }
        } else {
            let cw = (cbuf.as_str().chars().count() as u16).min(inner.width.saturating_sub(2));
            let cx = inner.x + (inner.width - cw) / 2;
            let crect = draw_button_at(buf, cx, confirm_row, cbuf.as_str(), cw, cstate, next_accent);
            if can_confirm {
                hits.push(crect, KeepThreeAction::Confirm);
            }
            put_centered(
                buf,
                inner,
                inner.height.saturating_sub(1),
                "↑↓ select   ·   Space/click pack   ·   Enter confirm",
                ink(MUTED).bg(PLATE),
            );
        }
    }

    // The next stage's route (`WAVE3_DESIGN.md` §1). The two chips live on the
    // panel header, after the caption, so every layout (120x40, 80x24, 57x20)
    // shows the choice without losing a manifest row. Original route first;
    // Tab toggles, and the chips are clickable.
    let has_route = view.route.iter().any(|chip| chip.is_some());
    if has_route && layout.panel_rect.width >= 40 {
        let y = layout.panel_rect.y;
        let mut x = layout.panel_rect.x + 2 + cap.as_str().chars().count() as u16 + 3;
        let room = layout.panel_rect.right().saturating_sub(3);
        for (i, chip) in view.route.iter().enumerate() {
            let Some(chip) = chip else { continue };
            let mut lbl = StackBuf::<16>::new();
            let _ = write!(lbl, "[{}]", chip.name);
            let w = lbl.as_str().chars().count() as u16;
            if x + w > room {
                break;
            }
            let style = if chip.chosen {
                heavy(HEAL_MINT).bg(PLATE)
            } else if view.hovered == Some(KeepThreeAction::Route(i as u8)) {
                ink(COAT).bg(PLATE_HILITE)
            } else {
                ink(MUTED).bg(PLATE)
            };
            put_str(buf, x, y, lbl.as_str(), w, style);
            hits.push(Rect::new(x, y, w, 1), KeepThreeAction::Route(i as u8));
            x += w + 1;
        }
        // The purse, when the chips took its place on the header.
        if !gold_in_caption {
            let mut purse = StackBuf::<24>::new();
            let _ = write!(purse, " GOLD {} ", view.gold);
            let pw = purse.as_str().chars().count() as u16;
            put_str(
                buf,
                layout.panel_rect.x + 2,
                layout.panel_rect.bottom().saturating_sub(1),
                purse.as_str(),
                pw,
                ink(TRAIL_GOLD).bg(PLATE),
            );
        }
        // The key hint rides the bottom border, where nothing else is drawn.
        let hint = "TAB route";
        let hw = hint.chars().count() as u16;
        let hx = layout.panel_rect.right().saturating_sub(hw + 2);
        put_str(
            buf,
            hx,
            layout.panel_rect.bottom().saturating_sub(1),
            hint,
            hw,
            ink(EDGE).bg(PLATE),
        );
    }

    hits
}

#[allow(clippy::too_many_arguments)]
fn draw_scrollbar(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    height: u16,
    scroll_top: usize,
    max_scroll: usize,
    hovered: Option<KeepThreeAction>,
    hits: &mut HitList<KeepThreeAction, 32>,
) {
    if height < 3 || x < buf.area.x || x >= buf.area.right() {
        return;
    }
    let can_up = scroll_top > 0;
    let can_down = scroll_top < max_scroll;

    let up_style = if !can_up {
        ink(TRACK).bg(PLATE)
    } else if hovered == Some(KeepThreeAction::ScrollUp) {
        heavy(PLATE).bg(TAIL)
    } else {
        heavy(COAT).bg(TRACK)
    };
    put_str(buf, x, y, "▴", 1, up_style);
    if can_up {
        hits.push(
            clamp_rect(Rect::new(x, y, 1, 1), buf.area),
            KeepThreeAction::ScrollUp,
        );
    }

    let track_h = height.saturating_sub(2);
    let thumb_rel = if max_scroll == 0 || track_h <= 1 {
        0
    } else {
        ((scroll_top * usize::from(track_h - 1)) + max_scroll / 2) / max_scroll
    } as u16;
    for dy in 0..track_h {
        let cy = y + 1 + dy;
        if dy == thumb_rel {
            put_str(buf, x, cy, "█", 1, ink(TAIL).bg(PLATE));
        } else {
            put_str(buf, x, cy, "│", 1, ink(TRACK).bg(PLATE));
        }
    }

    let bot_y = y + height - 1;
    let down_style = if !can_down {
        ink(TRACK).bg(PLATE)
    } else if hovered == Some(KeepThreeAction::ScrollDown) {
        heavy(PLATE).bg(TAIL)
    } else {
        heavy(COAT).bg(TRACK)
    };
    put_str(buf, x, bot_y, "▾", 1, down_style);
    if can_down {
        hits.push(
            clamp_rect(Rect::new(x, bot_y, 1, 1), buf.area),
            KeepThreeAction::ScrollDown,
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Level Intro Screen (Levels 2 & 3, consistent with menu::draw_map_intro)
// ---------------------------------------------------------------------------

fn level_emblem(level: CampaignLevel) -> [&'static str; 3] {
    match level {
        CampaignLevel::Sewer => [
            "      ╭──┬──╮      ",
            "      ╰──┼──╯      ",
            "═════╡═══╧═══╞═════",
        ],
        CampaignLevel::Kitchen => [
            " ▄▄▄   ▄▄▄   ▄▄▄ ",
            " ▀█▀   ▀█▀   ▀█▀ ",
            "══╧═════╧═════╧══",
        ],
        CampaignLevel::Lab => [
            "  ╭──╮      ╭──╮  ",
            "──┤  ├──»───┤  ├──",
            "  ╰──╯      ╰──╯  ",
        ],
        // The side route floors share the borrowed boss's bay until their own
        // art lands: the Laundry gets the broom's bay, the Printworks the
        // mower's.
        CampaignLevel::Laundry => [
            " ▄▄▄   ▄▄▄   ▄▄▄ ",
            " ▀█▀   ▀█▀   ▀█▀ ",
            "══╧═════╧═════╧══",
        ],
        CampaignLevel::Printworks => [
            "  ╭──╮      ╭──╮  ",
            "──┤  ├──»───┤  ├──",
            "  ╰──╯      ╰──╯  ",
        ],
    }
}

/// Draw the pre-level intro card for Campaign Levels 2 & 3 (and Level 1 if used).
///
/// Visual language mirrors `menu::draw_map_intro` (stamped `ACCESS ROUTE` plate,
/// 3-line architectural emblem, heading, flavor text, and Begin pill) extended
/// with a `CARRIED IN` section showing the upgrades packed from the prior sector.
pub fn draw_level_intro(
    f: &mut Frame,
    area: Rect,
    view: &LevelIntroView<'_>,
) -> HitList<LevelIntroAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let accent = view.level.accent();
    let carried_n = view.carried.len().min(3);
    let very_tall = area.width >= 74 && area.height >= 26;
    let spacious = area.width >= 66 && area.height >= 22;
    let pw = if very_tall {
        72
    } else if area.width >= 68 {
        66
    } else {
        area.width.min(60)
    };
    let ph = if very_tall {
        22
    } else if spacious {
        20
    } else if area.height >= 18 {
        18
    } else {
        area.height
    };
    let card = centered_rect(pw, ph, area);

    let buf = f.buffer_mut();
    let cap = " ACCESS ROUTE ";
    let inner = panel(buf, card, cap, None, accent);
    stamp_header_pips(
        buf,
        card,
        cap,
        view.level,
        view.level.number().saturating_sub(1),
    );

    if inner.width < 10 || inner.height < 4 {
        return hits;
    }

    let mut y: u16 = 0;

    // 3-line architectural emblem matching `menu::draw_map_intro`.
    if inner.height >= 14 {
        for line in level_emblem(view.level) {
            put_centered(buf, inner, y, line, ink(accent).bg(PLATE));
            y += 1;
        }
        y += 1;
    }

    // Heading: LEVEL N — SECTOR NAME.
    let mut heading = StackBuf::<40>::new();
    let _ = write!(heading, "LEVEL {} — {}", view.level.number(), view.level.name());
    put_centered(buf, inner, y, heading.as_str(), heavy(COAT).bg(PLATE));
    y += if inner.height >= 15 { 2 } else { 1 };

    // Flavor text.
    let flavor = view
        .flavor
        .unwrap_or_else(|| view.level.default_intro_flavor());
    let inset = if inner.width >= 40 { 3 } else { 1 };
    let flavor_h = if inner.height >= 15 { 3 } else { 2 };
    let flavor_rect = Rect::new(
        inner.x + inset,
        inner.y + y,
        inner.width.saturating_sub(inset * 2),
        flavor_h.min(inner.height.saturating_sub(y + 4).max(1)),
    );
    let lines_used = put_wrapped(buf, flavor_rect, flavor, ink(MUTED).bg(PLATE), true);
    y += lines_used + u16::from(inner.height >= 16);

    // CARRIED IN section.
    let btn_row = inner.height.saturating_sub(1);
    if y + 2 <= btn_row {
        hline(
            buf,
            inner.x + 2,
            inner.y + y,
            inner.width.saturating_sub(4),
            '─',
            ink(TRACK).bg(PLATE),
        );
        let lbl = " CARRIED IN ";
        let lx = inner.x + 4;
        if inner.width >= 24 {
            put_str(buf, lx, inner.y + y, lbl, 12, heavy(accent).bg(PLATE));
        }
        if inner.width >= 28 {
            let mut sb = StackBuf::<40>::new();
            if view.score_so_far > 0 && inner.width >= 52 {
                let _ = write!(sb, " SCORE {} ·", view.score_so_far);
            }
            let _ = write!(sb, " GOLD {} ", view.gold);
            let sw = sb.as_str().chars().count() as u16;
            put_str(
                buf,
                inner.right().saturating_sub(sw + 4),
                inner.y + y,
                sb.as_str(),
                sw,
                ink(TRAIL_GOLD).bg(PLATE),
            );
        }
        y += 1;

        if carried_n == 0 {
            put_centered(
                buf,
                inner,
                y,
                "Nothing carried — claws and nerve only.",
                ink(MUTED).bg(PLATE),
            );
        } else if spacious && y + 3 <= btn_row {
            // 3 side-by-side mini cards for carried upgrades!
            let card_h: u16 = if very_tall && y + 4 <= btn_row { 4 } else { 3 };
            let gap = 1u16;
            let total_w = inner.width.saturating_sub(4);
            let card_w = total_w.saturating_sub(gap * 2) / 3;
            let start_x = inner.x + 2 + (total_w - (card_w * 3 + gap * 2)) / 2;
            for (i, it) in view.carried.iter().take(3).enumerate() {
                let cx = start_x + i as u16 * (card_w + gap);
                let crect = Rect::new(cx, inner.y + y, card_w, card_h);
                let icol = to_color(it.color);
                let ci = sub_box(buf, crect, EDGE, PLATE_SUB);
                let show_ci_icon = card_h >= 4 && ci.height >= art_icons::SMALL_CELL_H && ci.width >= 15;
                let tx = if show_ci_icon {
                    let icon_rect = Rect::new(
                        ci.x,
                        ci.y,
                        art_icons::SMALL_CELL_W,
                        art_icons::SMALL_CELL_H,
                    );
                    art_icons::draw(buf, icon_rect, art_icons::by_name(it.name));
                    ci.x + art_icons::SMALL_CELL_W + 1
                } else {
                    let mut g = [0u8; 4];
                    let t0 = put_str(
                        buf,
                        ci.x,
                        ci.y,
                        it.glyph.encode_utf8(&mut g),
                        1,
                        heavy(icol).bg(PLATE_SUB),
                    );
                    put_str(buf, t0, ci.y, " ", 1, ink(COAT).bg(PLATE_SUB))
                };
                let mut badge = StackBuf::<28>::new();
                match it.category {
                    UpgradeCategory::Weapon { level, max_level } => {
                        if ci.width >= 18 && card_h == 3 {
                            let pips = fmt_pips(level, max_level);
                            let _ = write!(badge, "{}", pips.as_str());
                        } else if level >= max_level {
                            let _ = write!(badge, "Lv{}★", level);
                        } else {
                            let _ = write!(badge, "Lv{}", level);
                        }
                    }
                    UpgradeCategory::Passive { stacks } => {
                        let _ = write!(badge, "×{}", stacks);
                    }
                }
                let bw = badge.as_str().chars().count() as u16;
                let nw = ci.right().saturating_sub(tx + bw + 1);
                put_str_ellipsis(buf, tx, ci.y, it.name, nw, heavy(COAT).bg(PLATE_SUB));
                put_str(
                    buf,
                    ci.right().saturating_sub(bw),
                    ci.y,
                    badge.as_str(),
                    bw,
                    heavy(if matches!(it.category, UpgradeCategory::Weapon { level, max_level } if level >= max_level) {
                        TRAIL_GOLD
                    } else {
                        icol
                    })
                    .bg(PLATE_SUB),
                );
                if card_h >= 4 && ci.height >= 2 {
                    let dx = if show_ci_icon { tx } else { ci.x };
                    put_str_ellipsis(
                        buf,
                        dx,
                        ci.y + 1,
                        it.detail,
                        ci.right().saturating_sub(dx),
                        ink(MUTED).bg(PLATE_SUB),
                    );
                }
            }
        } else {
            // Compact stacked or inline rows for carried upgrades.
            let avail_rows = btn_row.saturating_sub(y);
            if avail_rows >= carried_n as u16 {
                for (i, it) in view.carried.iter().take(3).enumerate() {
                    let row_y = inner.y + y + i as u16;
                    let icol = to_color(it.color);
                    let mut g = [0u8; 4];
                    let mut cx = put_str(buf, inner.x + 3, row_y, "◆ ", 2, ink(HEAL_MINT).bg(PLATE));
                    cx = put_str(buf, cx, row_y, it.glyph.encode_utf8(&mut g), 1, heavy(icol).bg(PLATE));
                    cx = put_str(buf, cx, row_y, " ", 1, ink(COAT).bg(PLATE));
                    let mut tier = StackBuf::<32>::new();
                    match it.category {
                        UpgradeCategory::Weapon { level, max_level } => {
                            let pips = fmt_pips(level, max_level);
                            let _ = write!(tier, "Lv{} {}", level, pips.as_str());
                        }
                        UpgradeCategory::Passive { stacks } => {
                            let _ = write!(tier, "×{} passive", stacks);
                        }
                    }
                    let tw = tier.as_str().chars().count() as u16;
                    let nw = inner.right().saturating_sub(cx + tw + 4);
                    put_str_ellipsis(buf, cx, row_y, it.name, nw, heavy(COAT).bg(PLATE));
                    put_str(
                        buf,
                        inner.right().saturating_sub(tw + 3),
                        row_y,
                        tier.as_str(),
                        tw,
                        ink(icol).bg(PLATE),
                    );
                }
            } else {
                // Single-line inline pills when height is very tight.
                let mut px = inner.x + 2;
                let slot_w = (inner.width.saturating_sub(6) / carried_n.max(1) as u16).max(8);
                for (i, it) in view.carried.iter().take(3).enumerate() {
                    px = draw_pack_slot_pill(buf, px, inner.y + y, slot_w, i, Some(it));
                    px = put_str(buf, px, inner.y + y, " ", 1, ink(PLATE).bg(PLATE));
                }
            }
        }
    }

    // Begin CTA pill matching `menu::draw_map_intro`.
    let btn_label = if inner.width >= 30 {
        " ▸  Enter / click   begin "
    } else {
        " ▸ Enter  begin "
    };
    let bw = (btn_label.chars().count() as u16).min(inner.width);
    let bx = inner.x + (inner.width - bw) / 2;
    let bstate = if view.hovered == Some(LevelIntroAction::Begin) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let brect = draw_button_at(buf, bx, inner.y + btn_row, btn_label, bw, bstate, accent);
    hits.push(brect, LevelIntroAction::Begin);

    hits
}

// ---------------------------------------------------------------------------
// 4. Campaign Victory Screen
// ---------------------------------------------------------------------------

/// Draw the Campaign Victory screen ("the rat is out") with final score and
/// per-level breakdown across Sewer, Kitchen, and Lab.
pub fn draw_campaign_victory(
    f: &mut Frame,
    area: Rect,
    view: &CampaignVictoryView<'_>,
) -> HitList<CampaignVictoryAction, 4> {
    let mut hits = HitList::new();
    if area.width < 12 || area.height < 6 {
        return hits;
    }

    let wide = area.width >= 76;
    let tall = area.height >= 26;
    let pw = if wide {
        area.width.min(78)
    } else {
        area.width.min(64)
    };
    let ph = if tall {
        area.height.min(28)
    } else if area.height >= 22 {
        area.height.min(23)
    } else {
        area.height
    };
    let card = centered_rect(pw, ph, area);

    let buf = f.buffer_mut();
    let cap = if card.width >= 60 {
        " EXTRACTION COMPLETE   /   SUBJECT R-47 "
    } else {
        " EXTRACTION COMPLETE "
    };
    let inner = panel(buf, card, cap, None, VICTORY_MINT);
    stamp_header_pips(buf, card, cap, CampaignLevel::Lab, 3);

    if inner.width < 10 || inner.height < 5 {
        return hits;
    }

    let mut y: u16 = u16::from(inner.height >= 20);

    // Top status pill (+ optional NEW RECORD badge).
    let primary_pill = if inner.width >= 34 {
        " ▰▰  THE RAT IS OUT  ▰▰ "
    } else {
        " ▸ THE RAT IS OUT "
    };
    if view.new_best && inner.width >= 52 {
        let rec_pill = " ◆ NEW RECORD ◆ ";
        let total_w = primary_pill.chars().count() as u16 + 2 + rec_pill.chars().count() as u16;
        let sx = inner.x + (inner.width - total_w) / 2;
        let cx = put_str(buf, sx, inner.y + y, primary_pill, total_w, heavy(PLATE).bg(VICTORY_MINT));
        let cx = put_str(buf, cx, inner.y + y, "  ", 2, ink(PLATE).bg(PLATE));
        put_str(buf, cx, inner.y + y, rec_pill, total_w, heavy(PLATE).bg(TRAIL_GOLD));
    } else {
        put_centered(buf, inner, y, primary_pill, heavy(PLATE).bg(VICTORY_MINT));
    }
    y += if inner.height >= 22 { 2 } else { 1 };

    // Open blast door / night air emblem on tall screens (31 cols, mirror-symmetric).
    if inner.height >= 21 && inner.width >= 38 {
        let emblem = [
            ("   ▐█▌     ·   ✦   ·     ▐█▌   ", TRAIL_GOLD),
            ("═══▐█▌  ╱  OPEN  AIR  ╲  ▐█▌═══", VICTORY_MINT),
            ("   ▐█▌ ╱═══════════════╲ ▐█▌   ", HEAL_MINT),
        ];
        for (i, (line, col)) in emblem.iter().enumerate() {
            put_centered(buf, inner, y + i as u16, line, ink(*col).bg(PLATE));
        }
        y += 4;
    }

    // Finale narrative line.
    let default_blurb = if inner.width < 60 && inner.height < 20 {
        "Three sectors breached. Open air smells like rain and asphalt."
    } else {
        "Every alarm flatlines behind you. Three sectors, three apex threats — none of them held. Open air smells like rain and asphalt."
    };
    let blurb = view.custom_blurb.unwrap_or(default_blurb);
    let inset = if inner.width >= 42 { 3 } else { 1 };
    let blurb_rect = Rect::new(
        inner.x + inset,
        inner.y + y,
        inner.width.saturating_sub(inset * 2),
        if inner.height >= 17 { 2 } else { 1 },
    );
    let used = put_wrapped(buf, blurb_rect, blurb, ink(MUTED).bg(PLATE), true);
    y += used + u16::from(inner.height >= 20);

    // Final Totals highlight strip.
    if y + 3 < inner.height {
        hline(buf, inner.x + 2, inner.y + y, inner.width.saturating_sub(4), '─', ink(TRACK).bg(PLATE));
        y += 1;

        let ttime = fmt_time(view.total_time_secs);
        if inner.width >= 62 {
            let mut sb = StackBuf::<16>::new();
            let _ = write!(sb, "{}", view.final_score);
            let mut kb = StackBuf::<16>::new();
            let _ = write!(kb, "{}", view.total_kills);
            let mut gold_b = StackBuf::<16>::new();
            let _ = write!(gold_b, "{}", view.gold);
            let cols = 5u16;
            let cw = inner.width.saturating_sub(4) / cols;
            let x0 = inner.x + 2;
            let totals: [(&str, &str, Color); 5] = [
                ("FINAL SCORE", sb.as_str(), TRAIL_GOLD),
                ("TOTAL TIME", ttime.as_str(), COAT),
                ("TOTAL KILLS", kb.as_str(), COAT),
                ("SECTORS", "3 / 3", HEAL_MINT),
                ("GOLD", gold_b.as_str(), TRAIL_GOLD),
            ];
            for (i, (k, v, vc)) in totals.iter().enumerate() {
                let cx = x0 + i as u16 * cw;
                let cell = Rect::new(cx, inner.y + y, cw, 2);
                put_centered(buf, cell, 0, k, ink(MUTED).bg(PLATE));
                put_centered(buf, cell, 1, v, heavy(*vc).bg(PLATE));
                if i + 1 < totals.len() {
                    vline(buf, cx + cw.saturating_sub(1), inner.y + y, 2, '│', ink(TRACK).bg(PLATE));
                }
            }
            y += 2;
        } else {
            let mut line = StackBuf::<64>::new();
            let _ = write!(
                line,
                "FINAL {} · {} · {} kills · 3 / 3 · Gold {}",
                view.final_score,
                ttime.as_str(),
                view.total_kills,
                view.gold
            );
            put_centered(buf, inner, y, line.as_str(), heavy(TRAIL_GOLD).bg(PLATE));
            y += 1;
        }
    }

    // Per-Level Breakdown table (Sewer, Kitchen, Lab).
    let btn_row = inner.height.saturating_sub(1);
    if y + 5 <= btn_row {
        hline(buf, inner.x + 2, inner.y + y, inner.width.saturating_sub(4), '─', ink(TRACK).bg(PLATE));
        y += 1;

        let show_boss = inner.width >= 66;
        let table_w: u16 = if show_boss { 64 } else { inner.width.saturating_sub(4).min(50) };
        let tx = inner.x + (inner.width - table_w) / 2;

        if show_boss {
            put_str(
                buf,
                tx,
                inner.y + y,
                "SECTOR          BOSS SLAIN           TIME   KILLS     SCORE",
                table_w,
                heavy(VICTORY_MINT).bg(PLATE),
            );
        } else {
            put_str(
                buf,
                tx,
                inner.y + y,
                "SECTOR         TIME    KILLS     SCORE",
                table_w,
                heavy(VICTORY_MINT).bg(PLATE),
            );
        }
        y += 1;

        for row in &view.levels {
            if y >= btn_row {
                break;
            }
            let rtime = fmt_time(row.time_secs);
            let mut line = StackBuf::<72>::new();
            let sec_col = row.level.accent();
            let mut badge = StackBuf::<16>::new();
            let _ = write!(badge, "0{} {:<10}", row.level.number(), row.level.short_name());
            put_str(buf, tx, inner.y + y, badge.as_str(), 14, heavy(sec_col).bg(PLATE));

            if show_boss {
                let _ = write!(
                    line,
                    "  {:<18}  {}   {:>5}   {:>7}",
                    row.level.boss_name(),
                    rtime.as_str(),
                    row.kills,
                    row.score
                );
                put_str(buf, tx + 14, inner.y + y, line.as_str(), table_w.saturating_sub(14), ink(COAT).bg(PLATE));
            } else {
                let _ = write!(
                    line,
                    " {}   {:>5}   {:>7}",
                    rtime.as_str(),
                    row.kills,
                    row.score
                );
                put_str(buf, tx + 14, inner.y + y, line.as_str(), table_w.saturating_sub(14), ink(COAT).bg(PLATE));
            }
            y += 1;
        }
    }

    // Optional Final Loadout strip when vertical space remains.
    if !view.final_loadout.is_empty() && y + 2 <= btn_row && inner.width >= 48 {
        y += 1;
        let mut sx = inner.x + 3;
        sx = put_str(buf, sx, inner.y + y, "CARRIED OUT  ", 13, heavy(VICTORY_MINT).bg(PLATE));
        for (i, it) in view.final_loadout.iter().take(6).enumerate() {
            if sx + 8 >= inner.right() {
                break;
            }
            if i > 0 {
                sx = put_str(buf, sx, inner.y + y, "  ", 2, ink(PLATE).bg(PLATE));
            }
            let mut tok = StackBuf::<16>::new();
            let _ = tok.write_char(it.glyph);
            for ch in it.name.chars().take(5) {
                let _ = tok.write_char(ch);
            }
            match it.category {
                UpgradeCategory::Weapon { level, max_level } => {
                    if level >= max_level {
                        let _ = tok.write_char('★');
                    } else {
                        let _ = write!(tok, "{}", level);
                    }
                }
                UpgradeCategory::Passive { stacks } => {
                    let _ = write!(tok, "×{}", stacks);
                }
            }
            sx = put_str(
                buf,
                sx,
                inner.y + y,
                tok.as_str(),
                inner.right().saturating_sub(sx + 1),
                ink(to_color(it.color)).bg(PLATE),
            );
        }
    }

    // Bottom buttons: Play Again & Menu.
    let play_lbl = if inner.width >= 44 {
        " ▸ Enter  new campaign "
    } else {
        " Enter replay "
    };
    let menu_lbl = " Q  to menu ";
    let gap = 3u16;
    let pw_b = play_lbl.chars().count() as u16;
    let mw_b = menu_lbl.chars().count() as u16;
    let total = (pw_b + gap + mw_b).min(inner.width);
    let bx = inner.x + (inner.width - total) / 2;

    let play_state = if view.hovered == Some(CampaignVictoryAction::PlayAgain) {
        ButtonState::Hovered
    } else if view.focus == CampaignVictoryAction::PlayAgain {
        ButtonState::Focused
    } else {
        ButtonState::Normal
    };
    let menu_state = if view.hovered == Some(CampaignVictoryAction::Menu) {
        ButtonState::Hovered
    } else if view.focus == CampaignVictoryAction::Menu {
        ButtonState::Focused
    } else {
        ButtonState::Normal
    };

    let play_r = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        play_lbl,
        pw_b.min(inner.right().saturating_sub(bx)),
        play_state,
        VICTORY_MINT,
    );
    hits.push(play_r, CampaignVictoryAction::PlayAgain);

    let mx = bx + pw_b + gap;
    if mx < inner.right() {
        let menu_r = draw_button_at(
            buf,
            mx,
            inner.y + btn_row,
            menu_lbl,
            mw_b.min(inner.right().saturating_sub(mx)),
            menu_state,
            VICTORY_MINT,
        );
        hits.push(menu_r, CampaignVictoryAction::Menu);
    }

    hits
}

// ---------------------------------------------------------------------------
// 5. Title Level Select (Three Slots: Locked / Unlocked + Best Result)
// ---------------------------------------------------------------------------

/// Draw the three-sector Campaign Level Select screen for the title screen.
///
/// Renders 3 side-by-side sector bay cards on wide viewports (`width >= 76 && height >= 16`)
/// and 3 stacked horizontal rows on compact viewports (`57x20`).
///
/// Locked slots are rendered dimmed and do not emit click targets.
/// [`LevelSelectAction::Start`] is emitted only when the currently selected slot is unlocked.
pub fn draw_level_select(
    f: &mut Frame,
    area: Rect,
    view: &LevelSelectView,
) -> HitList<LevelSelectAction, 8> {
    let mut hits = HitList::new();
    if area.width < 16 || area.height < 8 {
        return hits;
    }

    let sel_idx = view.selected.min(2);
    let sel_slot = &view.slots[sel_idx];
    let sel_accent = if sel_slot.unlocked {
        sel_slot.level.accent()
    } else {
        MUTED
    };

    let side_by_side = area.width >= 76 && area.height >= 16;
    let pw = if side_by_side {
        area.width.min(92)
    } else {
        area.width.min(68)
    };
    let ph = if side_by_side {
        area.height.min(22)
    } else {
        area.height.min(20)
    };
    let card = centered_rect(pw, ph, area);

    let cleared_n = view
        .slots
        .iter()
        .filter(|s| s.best.is_some_and(|b| b.cleared))
        .count() as u8;

    let buf = f.buffer_mut();
    let cap = if card.width >= 42 {
        " CAMPAIGN   /   SELECT SECTOR "
    } else {
        " CAMPAIGN SECTORS "
    };
    let inner = panel(buf, card, cap, None, sel_accent);
    stamp_header_pips(buf, card, cap, sel_slot.level, cleared_n);

    if inner.width < 12 || inner.height < 6 {
        return hits;
    }

    let show_hints = side_by_side && inner.height >= 19;
    let btn_row = if show_hints {
        inner.height.saturating_sub(2)
    } else {
        inner.height.saturating_sub(1)
    };

    if side_by_side {
        // Subtitle strip.
        put_centered(
            buf,
            inner,
            0,
            "Three sectors to the surface. Clear a sector to unlock the next.",
            ink(MUTED).bg(PLATE),
        );

        // 3 horizontal bay cards.
        let top_y = inner.y + 2;
        let card_h = inner.height.saturating_sub(5).clamp(7, 14);
        let gap = 2u16;
        let avail_w = inner.width.saturating_sub(4);
        let slot_w = avail_w.saturating_sub(gap * 2) / 3;
        let start_x = inner.x + 2 + (avail_w - (slot_w * 3 + gap * 2)) / 2;

        for (i, slot) in view.slots.iter().enumerate() {
            let sx = start_x + i as u16 * (slot_w + gap);
            let srect = Rect::new(sx, top_y, slot_w, card_h);
            let is_sel = i == sel_idx;
            let is_hov = view.hovered == Some(LevelSelectAction::SelectLevel(slot.level));
            let accent = slot.level.accent();

            let (border_col, bg) = if !slot.unlocked {
                (TRACK, PLATE_DIM)
            } else if is_hov {
                (TAIL, PLATE_HILITE)
            } else if is_sel {
                (accent, PLATE_SUB)
            } else {
                (EDGE, PLATE)
            };

            let ci = sub_box(buf, srect, border_col, bg);

            // Top-left sector tag on box border.
            let mut tag = StackBuf::<16>::new();
            let _ = write!(
                tag,
                " {} 0{} ",
                if is_sel && slot.unlocked { "▸" } else { "·" },
                slot.level.number()
            );
            put_str(
                buf,
                srect.x + 2,
                srect.y,
                tag.as_str(),
                10,
                heavy(if slot.unlocked { accent } else { EDGE }).bg(bg),
            );

            // Status stamp (CLEARED / OPEN / LOCKED).
            let (status_txt, status_style) = if !slot.unlocked {
                ("× LOCKED", ink(MUTED).bg(bg))
            } else if slot.best.is_some_and(|b| b.cleared) {
                ("★ CLEARED", heavy(HEAL_MINT).bg(bg))
            } else {
                ("◆ OPEN", heavy(accent).bg(bg))
            };
            put_centered(buf, ci, 0, status_txt, status_style);

            // Sector name.
            put_centered(
                buf,
                ci,
                2,
                slot.level.name(),
                if slot.unlocked {
                    heavy(COAT).bg(bg)
                } else {
                    ink(MUTED).bg(bg)
                },
            );

            // Mini sector emblem when card is tall enough.
            let mut cy: u16 = 4;
            if ci.height >= 10 && ci.width >= 19 {
                for line in level_emblem(slot.level) {
                    put_centered(
                        buf,
                        ci,
                        cy,
                        line,
                        ink(if slot.unlocked { accent } else { EDGE }).bg(bg),
                    );
                    cy += 1;
                }
                cy += 1;
            }

            // Boss codename.
            if cy < ci.height.saturating_sub(2) {
                let boss_line = if slot.unlocked {
                    slot.level.boss_name()
                } else {
                    "BOSS CLASSIFIED"
                };
                put_centered(
                    buf,
                    ci,
                    cy,
                    boss_line,
                    ink(if slot.unlocked { MUTED } else { EDGE }).bg(bg),
                );
                cy += 1;
            }

            // Best result or unlock requirement.
            let best_y = ci.height.saturating_sub(2);
            if best_y >= cy {
                hline(buf, ci.x + 1, ci.y + best_y.saturating_sub(1), ci.width.saturating_sub(2), '─', ink(TRACK).bg(bg));
            }
            if !slot.unlocked {
                let req = match slot.level {
                    CampaignLevel::Sewer => "Locked",
                    CampaignLevel::Kitchen => "Clear Sewer first",
                    CampaignLevel::Lab => "Clear Kitchen first",
                    CampaignLevel::Laundry => "Clear Sewer first",
                    CampaignLevel::Printworks => "Clear Kitchen first",
                };
                put_centered(buf, ci, best_y, req, ink(MUTED).bg(bg));
            } else if let Some(best) = slot.best {
                let bt = fmt_time(best.best_time_secs);
                let mut r1 = StackBuf::<28>::new();
                let _ = write!(r1, "BEST {}", best.score);
                put_centered(buf, ci, best_y, r1.as_str(), heavy(TRAIL_GOLD).bg(bg));
                if best_y + 1 < ci.height {
                    let mut r2 = StackBuf::<24>::new();
                    let _ = write!(r2, "TIME {}", bt.as_str());
                    put_centered(buf, ci, best_y + 1, r2.as_str(), ink(MUTED).bg(bg));
                }
            } else {
                put_centered(buf, ci, best_y, "BEST  —", ink(MUTED).bg(bg));
                if best_y + 1 < ci.height {
                    put_centered(buf, ci, best_y + 1, "No record yet", ink(EDGE).bg(bg));
                }
            }

            if slot.unlocked {
                hits.push(clamp_rect(srect, area), LevelSelectAction::SelectLevel(slot.level));
            }
        }
    } else {
        // ---- COMPACT STACKED ROWS (57x20) ----
        let row_h: u16 = ((inner.height.saturating_sub(3)) / 3).clamp(2, 4);
        let start_y = inner.y + u16::from(inner.height >= 16);

        for (i, slot) in view.slots.iter().enumerate() {
            let ry = start_y + i as u16 * row_h;
            if ry + row_h > inner.y + btn_row {
                break;
            }
            let rrect = Rect::new(inner.x + 1, ry, inner.width.saturating_sub(2), row_h);
            let is_sel = i == sel_idx;
            let is_hov = view.hovered == Some(LevelSelectAction::SelectLevel(slot.level));
            let accent = slot.level.accent();

            let (border_col, bg) = if !slot.unlocked {
                (TRACK, PLATE_DIM)
            } else if is_hov {
                (TAIL, PLATE_HILITE)
            } else if is_sel {
                (accent, PLATE_SUB)
            } else {
                (EDGE, PLATE)
            };

            let ci = if row_h >= 3 {
                sub_box(buf, rrect, border_col, bg)
            } else {
                fill_rect(buf, rrect, ' ', ink(COAT).bg(bg));
                rrect
            };

            // Line 1: cursor + number + name + status.
            let mut cx = ci.x + 1;
            let cursor_s = if is_sel && slot.unlocked { "▸ " } else { "  " };
            cx = put_str(buf, cx, ci.y, cursor_s, 2, heavy(accent).bg(bg));
            let mut nb = StackBuf::<24>::new();
            let _ = write!(nb, "0{} {}", slot.level.number(), slot.level.name());
            cx = put_str(
                buf,
                cx,
                ci.y,
                nb.as_str(),
                18,
                if slot.unlocked { heavy(COAT).bg(bg) } else { ink(EDGE).bg(bg) },
            );

            let (st, st_style) = if !slot.unlocked {
                ("× LOCKED", ink(MUTED).bg(bg))
            } else if slot.best.is_some_and(|b| b.cleared) {
                ("★ CLEARED", heavy(HEAL_MINT).bg(bg))
            } else {
                ("◆ OPEN", heavy(accent).bg(bg))
            };
            let st_w = st.chars().count() as u16;
            if ci.right() > cx + st_w + 1 {
                put_str(buf, ci.right().saturating_sub(st_w + 1), ci.y, st, st_w, st_style);
            }

            // Line 2 (if height >= 2 inside box): best result or unlock hint.
            if ci.height >= 2 {
                let y2 = ci.y + 1;
                let mut info = StackBuf::<56>::new();
                if !slot.unlocked {
                    let req = match slot.level {
                        CampaignLevel::Sewer => "Locked",
                        CampaignLevel::Kitchen => "Clear THE SEWERS to unlock",
                        CampaignLevel::Lab => "Clear THE KITCHENS to unlock",
                        CampaignLevel::Laundry => "Clear THE SEWERS to unlock",
                        CampaignLevel::Printworks => "Clear THE KITCHENS to unlock",
                    };
                    let _ = write!(info, "  {}", req);
                    put_str_ellipsis(buf, ci.x + 1, y2, info.as_str(), ci.width.saturating_sub(2), ink(MUTED).bg(bg));
                } else if let Some(best) = slot.best {
                    let bt = fmt_time(best.best_time_secs);
                    let _ = write!(
                        info,
                        "  BEST {}  ·  {}  ·  {}",
                        best.score,
                        bt.as_str(),
                        slot.level.boss_name()
                    );
                    put_str_ellipsis(buf, ci.x + 1, y2, info.as_str(), ci.width.saturating_sub(2), ink(MUTED).bg(bg));
                } else {
                    let _ = write!(info, "  BEST —  ·  Boss: {}", slot.level.boss_name());
                    put_str_ellipsis(buf, ci.x + 1, y2, info.as_str(), ci.width.saturating_sub(2), ink(MUTED).bg(bg));
                }
            }

            if slot.unlocked {
                hits.push(clamp_rect(rrect, area), LevelSelectAction::SelectLevel(slot.level));
            }
        }
    }

    // Bottom action bar: Start + Back.
    let can_start = sel_slot.unlocked;
    let start_lbl = if !can_start {
        " SECTOR LOCKED "
    } else if inner.width >= 48 {
        " ▸  Enter  START SECTOR "
    } else {
        " ▸ Enter START "
    };
    let back_lbl = " Esc  back ";
    let gap = 3u16;
    let sw = start_lbl.chars().count() as u16;
    let bw = back_lbl.chars().count() as u16;
    let total = (sw + gap + bw).min(inner.width);
    let bx = inner.x + (inner.width - total) / 2;

    let start_state = if !can_start {
        ButtonState::Disabled
    } else if view.hovered == Some(LevelSelectAction::Start) {
        ButtonState::Hovered
    } else {
        ButtonState::Primary
    };
    let start_r = draw_button_at(
        buf,
        bx,
        inner.y + btn_row,
        start_lbl,
        sw.min(inner.right().saturating_sub(bx)),
        start_state,
        sel_accent,
    );
    if can_start {
        hits.push(start_r, LevelSelectAction::Start);
    }

    let back_x = bx + sw + gap;
    if back_x < inner.right() {
        let back_state = if view.hovered == Some(LevelSelectAction::Back) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        };
        let back_r = draw_button_at(
            buf,
            back_x,
            inner.y + btn_row,
            back_lbl,
            bw.min(inner.right().saturating_sub(back_x)),
            back_state,
            sel_accent,
        );
        hits.push(back_r, LevelSelectAction::Back);
    }

    if show_hints {
        put_centered(
            buf,
            inner,
            inner.height.saturating_sub(1),
            "←→ / click select sector   ·   Enter begin   ·   Esc back",
            ink(MUTED).bg(PLATE),
        );
    }

    hits
}

// ---------------------------------------------------------------------------
// 6. Compact Campaign Progress Indicator (HUD / Pause)
// ---------------------------------------------------------------------------

/// Draw a compact campaign progress indicator (`LV 1/3 ● ◆ ○ SEWER`) into `area`.
///
/// Adapts gracefully across widths:
/// - `>= 24`: `LV 2/3  ● ◆ ○  Kitchen`
/// - `>= 14`: `LV 2/3  ● ◆ ○`
/// - `>= 9`:  `2/3 ●◆○`
/// - `>= 5`:  `● ◆ ○`
/// - `>= 3`:  `●◆○`
///
/// Returns the exact bounding [`Rect`] drawn inside `area`.
pub fn draw_campaign_pips(
    f: &mut Frame,
    area: Rect,
    view: &CampaignProgressView,
) -> Rect {
    let r = area.intersection(f.buffer_mut().area);
    if r.is_empty() {
        return Rect::default();
    }

    let bg = view.bg.unwrap_or(PLATE);
    let buf = f.buffer_mut();
    let y = r.y;
    let mut cx = r.x;
    let accent = view.current.accent();

    if r.width >= 14 {
        cx = put_str(buf, cx, y, "LV ", 3, ink(MUTED).bg(bg));
        let mut nb = StackBuf::<8>::new();
        let _ = write!(nb, "{}/3  ", view.current.number());
        cx = put_str(buf, cx, y, nb.as_str(), 5, heavy(COAT).bg(bg));
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, true, bg);
        if r.width >= 24 {
            cx = put_str(buf, cx, y, "  ", 2, ink(MUTED).bg(bg));
            let rem = r.right().saturating_sub(cx);
            cx = put_str(buf, cx, y, view.current.short_name(), rem, heavy(accent).bg(bg));
        }
    } else if r.width >= 9 {
        let mut nb = StackBuf::<8>::new();
        let _ = write!(nb, "{}/3 ", view.current.number());
        cx = put_str(buf, cx, y, nb.as_str(), 4, heavy(COAT).bg(bg));
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, false, bg);
    } else if r.width >= 5 {
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, true, bg);
    } else {
        cx = draw_three_pips(buf, cx, y, r.right(), view.current, view.cleared_count, false, bg);
    }

    Rect::new(r.x, y, cx.saturating_sub(r.x), 1)
}

#[allow(clippy::too_many_arguments)]
fn draw_three_pips(
    buf: &mut Buffer,
    mut cx: u16,
    y: u16,
    max_x: u16,
    current: CampaignLevel,
    cleared_count: u8,
    spaced: bool,
    bg: Color,
) -> u16 {
    for (idx, lvl) in CampaignLevel::all().into_iter().enumerate() {
        if cx >= max_x {
            break;
        }
        let (ch, col) = if (idx as u8) < cleared_count {
            ('●', HEAL_MINT)
        } else if lvl == current {
            ('◆', lvl.accent())
        } else {
            ('○', EDGE)
        };
        let mut g = [0u8; 4];
        cx = put_str(buf, cx, y, ch.encode_utf8(&mut g), 1, heavy(col).bg(bg));
        if spaced && idx < 2 && cx < max_x {
            cx = put_str(buf, cx, y, " ", 1, ink(MUTED).bg(bg));
        }
    }
    cx
}

// ---------------------------------------------------------------------------
// Tests & visual capture dumper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const SIZES: [(u16, u16); 4] = [(57, 20), (80, 24), (120, 40), (200, 60)];
    const TINY_SIZES: [(u16, u16); 7] = [(0, 0), (1, 1), (4, 2), (8, 4), (15, 7), (30, 10), (45, 14)];

    fn sample_upgrades() -> [UpgradeEntry<'static>; 13] {
        [
            UpgradeEntry {
                name: "Gnaw",
                glyph: '▸',
                color: (255, 240, 180),
                category: UpgradeCategory::Weapon { level: 5, max_level: 5 },
                detail: "Homing dart at nearest foe · MAX damage & pierce",
            },
            UpgradeEntry {
                name: "Cheese Spray",
                glyph: '»',
                color: (255, 210, 90),
                category: UpgradeCategory::Weapon { level: 4, max_level: 5 },
                detail: "Forward fan of 5 dairy darts · tight spread",
            },
            UpgradeEntry {
                name: "Squeak Nova",
                glyph: '✦',
                color: (200, 160, 255),
                category: UpgradeCategory::Weapon { level: 3, max_level: 5 },
                detail: "12-shot sonic burst in every direction",
            },
            UpgradeEntry {
                name: "Spore Orbit",
                glyph: '◎',
                color: (140, 255, 180),
                category: UpgradeCategory::Weapon { level: 3, max_level: 5 },
                detail: "3 fungal blades circle and shred nearby foes",
            },
            UpgradeEntry {
                name: "Tail Whip",
                glyph: '◈',
                color: (255, 150, 200),
                category: UpgradeCategory::Weapon { level: 2, max_level: 5 },
                detail: "Melee shockwave pulse around the rat",
            },
            UpgradeEntry {
                name: "Acid Spit",
                glyph: '♦',
                color: (160, 255, 90),
                category: UpgradeCategory::Weapon { level: 2, max_level: 5 },
                detail: "Lobs corrosive bile that pools and burns",
            },
            UpgradeEntry {
                name: "Sharp Teeth",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 3 },
                detail: "+45% weapon damage across all attacks",
            },
            UpgradeEntry {
                name: "Quick Paws",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+24% move speed for kiting & escapes",
            },
            UpgradeEntry {
                name: "Frenzy",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+24% weapon fire rate",
            },
            UpgradeEntry {
                name: "Thick Hide",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+2 flat armor against every hit",
            },
            UpgradeEntry {
                name: "Whiskers",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 1 },
                detail: "+40% XP gem & loot pickup radius",
            },
            UpgradeEntry {
                name: "Fat Reserves",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 2 },
                detail: "+40 max HP cushion",
            },
            UpgradeEntry {
                name: "Resilience",
                glyph: '+',
                color: (126, 224, 152),
                category: UpgradeCategory::Passive { stacks: 1 },
                detail: "+0.5 HP/s steady regeneration",
            },
        ]
    }

    fn buffer_text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn assert_single_cell_glyphs(t: &Terminal<TestBackend>) {
        let buf = t.backend().buffer();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let sym = buf[(x, y)].symbol();
                assert_eq!(
                    sym.chars().count(),
                    1,
                    "multi-char cell at ({x},{y}): {sym:?}"
                );
                let ch = sym.chars().next().unwrap();
                assert!(
                    !('\u{1F000}'..='\u{1FFFF}').contains(&ch),
                    "emoji glyph forbidden at ({x},{y}): {ch:?}"
                );
            }
        }
    }

    fn assert_hits_inside_drawn_controls<A: Copy + core::fmt::Debug, const N: usize>(
        t: &Terminal<TestBackend>,
        hits: &HitList<A, N>,
    ) {
        let buf = t.backend().buffer();
        let area = buf.area;
        for (rect, action) in hits.iter() {
            assert!(!rect.is_empty(), "empty hit rect for {action:?}");
            assert!(
                rect.x >= area.x
                    && rect.y >= area.y
                    && rect.right() <= area.right()
                    && rect.bottom() <= area.bottom(),
                "hit rect {rect:?} for {action:?} outside area {area:?}"
            );
            // Verify at least one visible non-space character was painted inside the hit rect.
            let mut has_ink = false;
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    if buf[(x, y)].symbol() != " " {
                        has_ink = true;
                        break;
                    }
                }
            }
            assert!(
                has_ink,
                "hit rect {rect:?} for {action:?} covers only blank space"
            );
            // Center of hit rect must hit-test back to some action.
            let cx = rect.x + rect.width / 2;
            let cy = rect.y + rect.height / 2;
            assert!(
                hits.at(cx, cy).is_some(),
                "center ({cx},{cy}) of {rect:?} did not hit-test"
            );
        }
    }

    #[test]
    fn level_complete_all_bosses_and_sizes() {
        for lvl in CampaignLevel::all() {
            let view = LevelCompleteView {
                level: lvl,
                time_secs: 252.0,
                kills: 284,
                score_so_far: 6420,
                gold: 24,
                level_score: 3120,
                player_level: 9,
                owned_upgrades: 11,
                custom_triumph: None,
                hovered: None,
            };
            for &(w, h) in &SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| {
                    hits = draw_level_complete(f, f.area(), &view);
                })
                .unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);
                assert!(
                    hits.find(LevelCompleteAction::Continue).is_some(),
                    "Continue button missing at {w}x{h}"
                );
                let txt = buffer_text(&term);
                assert!(txt.contains(lvl.boss_name()), "boss name missing at {w}x{h}");
                assert!(txt.contains("6420"), "score missing at {w}x{h}");
            }
            for &(w, h) in &TINY_SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| {
                    let _ = draw_level_complete(f, f.area(), &view);
                })
                .unwrap();
            }
        }
    }

    #[test]
    fn keep_three_selection_states_and_scrolling() {
        let items = sample_upgrades();

        // 0 chosen, 2 chosen, 3 chosen across all 4 target sizes.
        let cases: [(&str, [bool; 13], bool); 3] = [
            ("0/3", [false; 13], false),
            (
                "2/3",
                [true, false, true, false, false, false, false, false, false, false, false, false, false],
                false,
            ),
            (
                "3/3",
                [true, false, true, false, false, false, true, false, false, false, false, false, false],
                true,
            ),
        ];

        for &(w, h) in &SIZES {
            for (tag, sel, expect_confirm) in &cases {
                let view = KeepThreeView {
                    from_level: CampaignLevel::Sewer,
                    next_level: CampaignLevel::Kitchen,
                    gold: 24,
                    items: &items,
                    selected: sel,
                    max_keep: 3,
                    cursor: 2,
                    scroll: 0,
                    hovered: if *expect_confirm {
                        Some(KeepThreeAction::Confirm)
                    } else {
                        Some(KeepThreeAction::ToggleItem(1))
                    },
                    route: [None, None],
                };
                assert_eq!(keep_three_can_confirm(&view), *expect_confirm);

                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| {
                    hits = draw_keep_three(f, f.area(), &view);
                })
                .unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);

                let has_confirm = hits.find(KeepThreeAction::Confirm).is_some();
                assert_eq!(
                    has_confirm, *expect_confirm,
                    "confirm hit mismatch for {tag} at {w}x{h}"
                );
                let txt = buffer_text(&term);
                assert!(txt.contains(tag), "count {tag} missing at {w}x{h}");
            }
        }

        // Scroll verification at 80x24 and 57x20 where 13 items overflow visible rows.
        for &(w, h) in &[(57u16, 20u16), (80, 24)] {
            let vis = keep_three_visible_rows(Rect::new(0, 0, w, h));
            assert!(vis < items.len(), "expected scroll needed at {w}x{h}, vis={vis}");

            // At top (scroll=0): ScrollDown present, ScrollUp absent.
            let sel = [false; 13];
            let top_view = KeepThreeView {
                from_level: CampaignLevel::Sewer,
                next_level: CampaignLevel::Kitchen,
                gold: 24,
                items: &items,
                selected: &sel,
                max_keep: 3,
                cursor: 0,
                scroll: 0,
                hovered: None,
                route: [None, None],
            };
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_keep_three(f, f.area(), &top_view)).unwrap();
            assert!(hits.find(KeepThreeAction::ScrollUp).is_none());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_some());
            assert!(hits.find(KeepThreeAction::ToggleItem(0)).is_some());
            assert!(hits.find(KeepThreeAction::ToggleItem(items.len() - 1)).is_none());

            // At bottom (scroll=99 clamped): ScrollUp present, ScrollDown absent, last item visible.
            let bot_view = KeepThreeView {
                scroll: 99,
                cursor: items.len() - 1,
                ..top_view
            };
            term.draw(|f| hits = draw_keep_three(f, f.area(), &bot_view)).unwrap();
            assert_hits_inside_drawn_controls(&term, &hits);
            assert!(hits.find(KeepThreeAction::ScrollUp).is_some());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_none());
            assert!(hits.find(KeepThreeAction::ToggleItem(items.len() - 1)).is_some());
        }

        // Fewer than 3 owned upgrades: confirm enabled when all owned are selected.
        let few = &items[..2];
        let sel_two = [true, true];
        let few_view = KeepThreeView {
            from_level: CampaignLevel::Sewer,
            next_level: CampaignLevel::Kitchen,
            gold: 24,
            items: few,
            selected: &sel_two,
            max_keep: 3,
            cursor: 0,
            scroll: 0,
            hovered: None,
            route: [None, None],
        };
        assert!(keep_three_can_confirm(&few_view));
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut hits = HitList::new();
        term.draw(|f| hits = draw_keep_three(f, f.area(), &few_view)).unwrap();
        assert!(hits.find(KeepThreeAction::Confirm).is_some());
        assert!(buffer_text(&term).contains("2/2"));

        // Zero owned upgrades: confirm immediately enabled without panic.
        let empty_view = KeepThreeView {
            items: &[],
            selected: &[],
            ..few_view
        };
        assert!(keep_three_can_confirm(&empty_view));
        term.draw(|f| hits = draw_keep_three(f, f.area(), &empty_view)).unwrap();
        assert!(hits.find(KeepThreeAction::Confirm).is_some());

        // Tiny sizes never panic.
        for &(w, h) in &TINY_SIZES {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small
                .draw(|f| {
                    let _ = draw_keep_three(f, f.area(), &few_view);
                })
                .unwrap();
        }

        // Large list (18 entries) forces scroll even in wide two-pane layouts (120x40, 200x60).
        let mut many = Vec::with_capacity(18);
        many.extend_from_slice(&items);
        many.extend_from_slice(&items[..5]);
        let many_sel = [false; 18];
        for &(w, h) in &[(120u16, 40u16), (200, 60)] {
            let vis = keep_three_visible_rows(Rect::new(0, 0, w, h));
            assert!(vis < many.len(), "expected wide scroll at {w}x{h}, vis={vis}");
            let mut wide_term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            let wide_view = KeepThreeView {
                from_level: CampaignLevel::Kitchen,
                next_level: CampaignLevel::Lab,
                gold: 24,
                items: &many,
                selected: &many_sel,
                max_keep: 3,
                cursor: 16,
                scroll: 2,
                hovered: Some(KeepThreeAction::ScrollUp),
                route: [None, None],
            };
            wide_term
                .draw(|f| hits = draw_keep_three(f, f.area(), &wide_view))
                .unwrap();
            assert_single_cell_glyphs(&wide_term);
            assert_hits_inside_drawn_controls(&wide_term, &hits);
            assert!(hits.find(KeepThreeAction::ScrollUp).is_some());
            assert!(hits.find(KeepThreeAction::ScrollDown).is_some());
        }
    }

    #[test]
    fn level_intro_levels_2_and_3_all_sizes() {
        let items = sample_upgrades();
        let carried = [items[0], items[2], items[6]];

        for lvl in [CampaignLevel::Kitchen, CampaignLevel::Lab] {
            let view = LevelIntroView {
                level: lvl,
                flavor: None,
                carried: &carried,
                score_so_far: 6420,
                gold: 24,
                hovered: None,
            };
            for &(w, h) in &SIZES {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = HitList::new();
                term.draw(|f| hits = draw_level_intro(f, f.area(), &view)).unwrap();
                assert_single_cell_glyphs(&term);
                assert_hits_inside_drawn_controls(&term, &hits);
                assert!(hits.find(LevelIntroAction::Begin).is_some());
                let txt = buffer_text(&term);
                assert!(txt.contains(lvl.name()), "level name missing at {w}x{h}");
                assert!(txt.contains("Gnaw"), "carried upgrade missing at {w}x{h}");
                if w == 57 && h == 20 {
                    assert!(
                        txt.contains("●●●●●"),
                        "max-level 5-pip string truncated at 57x20"
                    );
                }
            }
            for &(w, h) in &TINY_SIZES {
                let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
                small
                    .draw(|f| {
                        let _ = draw_level_intro(f, f.area(), &view);
                    })
                    .unwrap();
            }
        }
    }

    #[test]
    fn campaign_victory_all_sizes() {
        let items = sample_upgrades();
        let view = CampaignVictoryView {
            final_score: 24860,
            total_time_secs: 764.0,
            total_kills: 942,
            gold: 24,
            new_best: true,
            levels: [
                LevelBreakdown {
                    level: CampaignLevel::Sewer,
                    time_secs: 252.0,
                    kills: 284,
                    score: 6420,
                    player_level: 9,
                },
                LevelBreakdown {
                    level: CampaignLevel::Kitchen,
                    time_secs: 261.0,
                    kills: 318,
                    score: 8140,
                    player_level: 10,
                },
                LevelBreakdown {
                    level: CampaignLevel::Lab,
                    time_secs: 251.0,
                    kills: 340,
                    score: 10300,
                    player_level: 11,
                },
            ],
            final_loadout: &items[..4],
            custom_blurb: None,
            focus: CampaignVictoryAction::PlayAgain,
            hovered: None,
        };

        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_campaign_victory(f, f.area(), &view))
                .unwrap();
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            assert!(hits.find(CampaignVictoryAction::PlayAgain).is_some());
            assert!(hits.find(CampaignVictoryAction::Menu).is_some());
            let txt = buffer_text(&term);
            assert!(txt.contains("THE RAT IS OUT"));
            assert!(txt.contains("24860"));
            assert!(txt.contains("Sewer"));
            assert!(txt.contains("Kitchen"));
            assert!(txt.contains("Lab"));
        }

        for &(w, h) in &TINY_SIZES {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small
                .draw(|f| {
                    let _ = draw_campaign_victory(f, f.area(), &view);
                })
                .unwrap();
        }
    }

    #[test]
    fn level_select_locked_unlocked_and_all_sizes() {
        let view = LevelSelectView {
            slots: [
                LevelSlotView {
                    level: CampaignLevel::Sewer,
                    unlocked: true,
                    best: Some(LevelBest {
                        score: 6420,
                        best_time_secs: 252.0,
                        cleared: true,
                    }),
                },
                LevelSlotView {
                    level: CampaignLevel::Kitchen,
                    unlocked: true,
                    best: Some(LevelBest {
                        score: 4180,
                        best_time_secs: 198.0,
                        cleared: false,
                    }),
                },
                LevelSlotView {
                    level: CampaignLevel::Lab,
                    unlocked: false,
                    best: None,
                },
            ],
            selected: 1,
            hovered: Some(LevelSelectAction::SelectLevel(CampaignLevel::Sewer)),
        };

        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut hits = HitList::new();
            term.draw(|f| hits = draw_level_select(f, f.area(), &view))
                .unwrap();
            assert_single_cell_glyphs(&term);
            assert_hits_inside_drawn_controls(&term, &hits);
            assert!(hits.find(LevelSelectAction::SelectLevel(CampaignLevel::Sewer)).is_some());
            assert!(hits.find(LevelSelectAction::SelectLevel(CampaignLevel::Kitchen)).is_some());
            assert!(
                hits.find(LevelSelectAction::SelectLevel(CampaignLevel::Lab)).is_none(),
                "locked slot must not emit click target"
            );
            assert!(hits.find(LevelSelectAction::Start).is_some());
            assert!(hits.find(LevelSelectAction::Back).is_some());
            let txt = buffer_text(&term);
            assert!(txt.contains("CLEARED"));
            assert!(txt.contains("LOCKED"));
            assert!(txt.contains("6420"));
        }

        // If selected points to a locked slot, Start must not be emitted.
        let locked_sel = LevelSelectView { selected: 2, ..view };
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut hits = HitList::new();
        term.draw(|f| hits = draw_level_select(f, f.area(), &locked_sel))
            .unwrap();
        assert!(hits.find(LevelSelectAction::Start).is_none());

        for &(w, h) in &TINY_SIZES {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small
                .draw(|f| {
                    let _ = draw_level_select(f, f.area(), &view);
                })
                .unwrap();
        }
    }

    #[test]
    fn campaign_pips_all_widths_and_levels() {
        for lvl in CampaignLevel::all() {
            let view = CampaignProgressView {
                current: lvl,
                cleared_count: lvl.number().saturating_sub(1),
                bg: None,
            };
            for w in [0u16, 1, 3, 5, 9, 14, 24, 36] {
                let mut term = Terminal::new(TestBackend::new(w.max(1), 1)).unwrap();
                let mut drawn = Rect::default();
                term.draw(|f| {
                    drawn = draw_campaign_pips(f, Rect::new(0, 0, w, 1), &view);
                })
                .unwrap();
                assert!(drawn.width <= w);
                assert_single_cell_glyphs(&term);
                if w >= 9 {
                    let txt = buffer_text(&term);
                    let want = format!("{}/3", lvl.number());
                    assert!(txt.contains(&want), "missing {want} at width {w}: {txt}");
                }
            }
        }
    }

    fn color_rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (19, 24, 27),
        }
    }

    fn dump_terminal(dir: &std::path::Path, name: &str, t: &Terminal<TestBackend>) {
        let buf = t.backend().buffer();
        let w = buf.area.width;
        let h = buf.area.height;
        let mut ansi = String::with_capacity(usize::from(w) * usize::from(h) * 32);
        let mut plain = String::with_capacity(usize::from(w) * usize::from(h) + usize::from(h));
        for y in 0..h {
            for x in 0..w {
                let c = &buf[(x, y)];
                let (r, g, b) = color_rgb(c.fg);
                let (br, bg, bb) = color_rgb(c.bg);
                let _ = write!(
                    ansi,
                    "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                    c.symbol()
                );
                plain.push_str(c.symbol());
            }
            ansi.push_str("\x1b[0m\n");
            plain.push('\n');
        }
        let path = dir.join(format!("{w}x{h}-{name}"));
        std::fs::write(path.with_extension("ansi"), ansi).unwrap();
        std::fs::write(path.with_extension("txt"), plain).unwrap();
    }

    /// Capture `.ansi` and `.txt` frames when `RATRUN_CAMPAIGN_DUMP_DIR` is set.
    #[test]
    fn dump_campaign_screens() {
        let Some(dir) = std::env::var_os("RATRUN_CAMPAIGN_DUMP_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let items = sample_upgrades();
        let carried = [items[0], items[2], items[6]];
        let sel0 = [false; 13];
        let sel2 = [true, false, true, false, false, false, false, false, false, false, false, false, false];
        let sel3 = [true, false, true, false, false, false, true, false, false, false, false, false, false];

        let victory = CampaignVictoryView {
            final_score: 24860,
            total_time_secs: 764.0,
            total_kills: 942,
            gold: 24,
            new_best: true,
            levels: [
                LevelBreakdown {
                    level: CampaignLevel::Sewer,
                    time_secs: 252.0,
                    kills: 284,
                    score: 6420,
                    player_level: 9,
                },
                LevelBreakdown {
                    level: CampaignLevel::Kitchen,
                    time_secs: 261.0,
                    kills: 318,
                    score: 8140,
                    player_level: 10,
                },
                LevelBreakdown {
                    level: CampaignLevel::Lab,
                    time_secs: 251.0,
                    kills: 340,
                    score: 10300,
                    player_level: 11,
                },
            ],
            final_loadout: &items[..4],
            custom_blurb: None,
            focus: CampaignVictoryAction::PlayAgain,
            hovered: None,
        };

        let select = LevelSelectView {
            slots: [
                LevelSlotView {
                    level: CampaignLevel::Sewer,
                    unlocked: true,
                    best: Some(LevelBest {
                        score: 6420,
                        best_time_secs: 252.0,
                        cleared: true,
                    }),
                },
                LevelSlotView {
                    level: CampaignLevel::Kitchen,
                    unlocked: true,
                    best: Some(LevelBest {
                        score: 4180,
                        best_time_secs: 198.0,
                        cleared: false,
                    }),
                },
                LevelSlotView {
                    level: CampaignLevel::Lab,
                    unlocked: false,
                    best: None,
                },
            ],
            selected: 1,
            hovered: None,
        };

        for &(w, h) in &SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();

            // Level complete for each boss (Sewer at all sizes, Kitchen & Lab at 80x24 & 120x40).
            for lvl in CampaignLevel::all() {
                if lvl != CampaignLevel::Sewer && (w != 80 && w != 120) {
                    continue;
                }
                let lc = LevelCompleteView {
                    level: lvl,
                    time_secs: 252.0,
                    kills: 284,
                    score_so_far: 6420,
                    gold: 24,
                    level_score: 3120,
                    player_level: 9,
                    owned_upgrades: 13,
                    custom_triumph: None,
                    hovered: None,
                };
                term.draw(|f| {
                    let _ = draw_level_complete(f, f.area(), &lc);
                })
                .unwrap();
                dump_terminal(dir, &format!("complete-{}", lvl.short_name().to_lowercase()), &term);
            }

            // Keep three: 0/3, 2/3, 3/3, and scrolled.
            for (tag, sel, cursor, scroll, hov) in [
                ("keep-0of3", &sel0, 0, 0, None),
                ("keep-2of3", &sel2, 4, 0, Some(KeepThreeAction::ToggleItem(3))),
                ("keep-3of3", &sel3, 6, 0, Some(KeepThreeAction::Confirm)),
                ("keep-scroll", &sel2, 11, 6, None),
            ] {
                if (tag == "keep-0of3" || tag == "keep-scroll") && w == 200 {
                    continue;
                }
                let k3 = KeepThreeView {
                    from_level: CampaignLevel::Sewer,
                    next_level: CampaignLevel::Kitchen,
                    gold: 24,
                    items: &items,
                    selected: sel,
                    max_keep: 3,
                    cursor,
                    scroll,
                    hovered: hov,
                    route: [None, None],
                };
                term.draw(|f| {
                    let _ = draw_keep_three(f, f.area(), &k3);
                })
                .unwrap();
                dump_terminal(dir, tag, &term);
            }

            // Level intros (Kitchen & Lab).
            for lvl in [CampaignLevel::Kitchen, CampaignLevel::Lab] {
                let intro = LevelIntroView {
                    level: lvl,
                    flavor: None,
                    carried: &carried,
                    score_so_far: 6420,
                    gold: 24,
                    hovered: None,
                };
                term.draw(|f| {
                    let _ = draw_level_intro(f, f.area(), &intro);
                })
                .unwrap();
                dump_terminal(dir, &format!("intro-{}", lvl.short_name().to_lowercase()), &term);
            }

            // Campaign victory.
            term.draw(|f| {
                let _ = draw_campaign_victory(f, f.area(), &victory);
            })
            .unwrap();
            dump_terminal(dir, "victory", &term);

            // Level select.
            term.draw(|f| {
                let _ = draw_level_select(f, f.area(), &select);
            })
            .unwrap();
            dump_terminal(dir, "select", &term);
        }

        // Compact pips showcase strip.
        let mut pips_term = Terminal::new(TestBackend::new(60, 7)).unwrap();
        pips_term
            .draw(|f| {
                let area = f.area();
                fill_rect(f.buffer_mut(), area, ' ', ink(COAT).bg(PLATE));
                let rows = [
                    (CampaignLevel::Sewer, 0u8, 28u16),
                    (CampaignLevel::Kitchen, 1, 28),
                    (CampaignLevel::Lab, 2, 28),
                    (CampaignLevel::Kitchen, 1, 14),
                    (CampaignLevel::Kitchen, 1, 9),
                    (CampaignLevel::Lab, 3, 5),
                ];
                for (i, (lvl, clr, w)) in rows.into_iter().enumerate() {
                    let v = CampaignProgressView {
                        current: lvl,
                        cleared_count: clr,
                        bg: Some(PLATE_SUB),
                    };
                    let _ = draw_campaign_pips(f, Rect::new(2, i as u16, w, 1), &v);
                }
            })
            .unwrap();
        dump_terminal(dir, "pips", &pips_term);
    }
}
