//! Tactical minimap and full-level fog-of-war cartography.
//!
//! # Visual Technique: Half-Block Pixels (`▀`) + Selective Symbolic Overlays
//!
//! Rat Run's playfield renders at 1×2 square sub-pixels per terminal cell via
//! [`crate::render::framebuffer::PixelBuffer`], using the upper half-block `▀`
//! with independent 24-bit `fg` (top pixel) and `bg` (bottom pixel). The
//! minimap uses the same half-block substrate because:
//!
//! - **Square aspect ratio**: Ghostty / Menlo terminal cells are ~1:2, so each
//!   half-block pixel is 1:1 square — a `200 × 160` (5:4) level maps without
//!   horizontal or vertical stretching.
//! - **Independent 24-bit color per sub-pixel**: Fog void, explored frontier
//!   mist, memory-faded rooms, live-LOS rooms, architectural wall rims vs deep
//!   bedrock mass, thin maze partitions, doorways, rat-holes (near-only vs
//!   linked), one-way exits, boss-arena floor, viewport framing, and
//!   player/marker blips all need distinct hues inside a compact box.
//!   Quadrants (2×2) and sextants (2×3) allow only 2 colors per terminal cell
//!   (causing color bleed whenever fog, wall, floor, and a marker meet) and
//!   sextants lack uniform glyph coverage across terminal fonts; braille (2×4)
//!   is monochrome and visually sparse.
//! - **Two complementary scales**:
//!   - **Compact corner radar** renders a **1:1 cell-per-sub-pixel** window
//!     centered on the player (`22×16` cells on `80×24`, `32×24` on `120×40`,
//!     `42×32` on `200×60`, degrading to nothing below `72×18`). At 1:1 every
//!     1-cell maze aisle, rat-hole, doorway, and the live camera viewport
//!     brackets stay razor-sharp with zero downsampling aliasing.
//!   - **Expanded Tab map** fits the **entire `200 × 160` level** at exact 5:4
//!     aspect ratio (`45×36` px on `80×24`, `80×64` px on `120×40`, `100×80` px
//!     on `200×60`) using feature-preserving downsampling (openings, wall rims,
//!     and thin maze partitions never vanish), then stamps crisp single-cell
//!     glyphs (`◇`/`◆` cache status, `▸` seen breaches, 8-way `▲◥►◢▼◣◄◤`
//!     fog-edge boss bearing, and discovered arena names) on top, paired with a
//!     tactical legend sidebar.
//!
//! # Zero-Allocation Contract
//!
//! Retain one [`MinimapScratch`] on `App` (next to `HudState` and
//! `PixelBuffer`) and pass `&mut scratch` to [`draw`] / [`draw_into`] each
//! frame. [`MinimapScratch::default`] pre-allocates capacity for the largest
//! compact radar (`42 × 32` sub-pixels), so steady-state compact rendering
//! performs **zero heap allocations per frame** from frame 1 onward; expanded
//! mode resizes the same buffers only when the terminal dimensions grow.
//!
//! # Fog-of-War Guarantee
//!
//! The renderer **never** calls [`MinimapSource::cell_at`] for any cell where
//! [`MinimapSource::seen`] returns `false`. Unseen space is tinted purely from
//! screen/map coordinates (subtle 8×8 cartography grid + seen-adjacency
//! frontier mist), so undiscovered rooms, maze aisles, exit directions, and
//! prizes remain an inviting, spoiler-free blank.

#![allow(dead_code)]

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::render::palette::{self, Rgb, COAT, EDGE, MUTED, PLATE, TAIL, TRACK};

// ---- Semantic Accent Colors (matching `hud.rs` / `overlay.rs`) -------------

const BOSS_ROSE_RGB: Rgb = (236, 76, 128);
const BOSS_HOT_RGB: Rgb = (255, 118, 88);
const WARN_AMBER_RGB: Rgb = (248, 178, 86);
const CACHE_GOLD_RGB: Rgb = (252, 224, 128);
const CLAIMED_MINT_RGB: Rgb = (126, 224, 152);
const MAGNET_ICE_RGB: Rgb = (128, 206, 250);
const RAT_HOLE_LINKED_RGB: Rgb = (252, 148, 78);
const RAT_HOLE_NEAR_RGB: Rgb = (164, 100, 62);
const ONE_WAY_EXIT_RGB: Rgb = (96, 242, 182);
const PLATE_RGB: Rgb = (19, 24, 27);
const COAT_RGB: Rgb = (235, 232, 232);
const TAIL_RGB: Rgb = (239, 168, 185);

/// Maximum inner pixel capacity pre-allocated for compact mode (`42 × 32`).
const COMPACT_PREALLOC_PIXELS: usize = 44 * 34;

/// Pixel category tags stored in `MinimapScratch::kind` so post-passes
/// (wall-rim catchlights & fog-frontier mist) stay O(W×H) without re-querying
/// `MinimapSource::cell_at`.
const KIND_UNSEEN: u8 = 0;
const KIND_FLOOR: u8 = 1;
const KIND_WALL: u8 = 2;
const KIND_FEATURE: u8 = 3;

// ---- Public Interface Types (Simulation-Independent) -----------------------

/// Cardinal travel direction for one-way maze exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapDir {
    North,
    East,
    South,
    West,
}

/// Wall orientation axis for rat-holes (names the **wall's** axis, matching
/// `game::art_walls::WallAxis`).
///
/// - `Horizontal`: the wall runs east–west; the rat passes north–south.
/// - `Vertical`: the wall runs north–south; the rat passes east–west.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapAxis {
    Horizontal,
    Vertical,
}

/// Simplified tile classification consumed by the minimap renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapCell {
    /// Out of bounds or void.
    Unknown,
    /// Open walkable floor.
    Floor,
    /// Thick structural room/border wall or solid prop.
    Wall,
    /// Thin maze partition (`WallKind::Maze`).
    MazeWall,
    /// Room or corridor doorway.
    Doorway,
    /// One-way maze exit door; carries the allowed outward travel direction.
    OneWayDoor(MinimapDir),
    /// Rat-hole wall opening; carries the wall axis so the renderer checks
    /// perpendicular cells and never leaks an unseen far mouth.
    RatHole(MinimapAxis),
    /// Discovered boss arena floor (receives subtle set-piece chamber tint).
    BossFloor,
    /// Floor trap (sticky or snap) once seen.
    Trap,
}

/// Read-only world & fog-of-war data source implemented by the simulation.
///
/// The minimap renderer guarantees it will **never** query [`cell_at`](Self::cell_at)
/// for any coordinate where [`seen`](Self::seen) returns `false`.
pub trait MinimapSource {
    /// Map width in tile cells (200 for standard levels).
    fn cols(&self) -> i32;
    /// Map height in tile cells (160 for standard levels).
    fn rows(&self) -> i32;
    /// Cell classification at `(cx, cy)`. Only called when `seen(cx, cy)` is `true`.
    fn cell_at(&self, cx: i32, cy: i32) -> MinimapCell;
    /// Fog-of-war discovery flag: `true` if cell `(cx, cy)` has ever been seen.
    fn seen(&self, cx: i32, cy: i32) -> bool;

    /// Continuous visibility recency in `[0.0, 1.0]` for memory-fade shading:
    /// - `1.0` = currently inside live line of sight (full brightness);
    /// - `0.0` = seen previously, now faded into cartographic memory.
    ///
    /// Implementers with a recency buffer (like `World::explored_recency`) can
    /// return it directly for a smooth feather; implementers with only a binary
    /// `seen` bitset can leave the default (`1.0` for seen cells, `0.0` for unseen).
    fn recency(&self, cx: i32, cy: i32) -> f32 {
        if self.seen(cx, cy) { 1.0 } else { 0.0 }
    }

    /// Whether the far mouth of the rat-hole at `(cx, cy)` has been seen or
    /// crossed by the player.
    ///
    /// Default checks `seen` on both cells on the passage axis perpendicular to
    /// `axis`. Implementers tracking explicit crossings may override this.
    fn rat_hole_far_known(&self, cx: i32, cy: i32, axis: MinimapAxis) -> bool {
        let (a, b) = match axis {
            MinimapAxis::Horizontal => ((cx, cy - 1), (cx, cy + 1)),
            MinimapAxis::Vertical => ((cx - 1, cy), (cx + 1, cy)),
        };
        self.seen(a.0, a.1) && self.seen(b.0, b.1)
    }
}

/// Theme colors matching `game::level::Palette` without importing simulation types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinimapPalette {
    pub bg: Rgb,
    pub bg_alt: Rgb,
    pub wall: Rgb,
    pub wall_edge: Rgb,
    pub accent: Rgb,
    pub blood: Rgb,
}

impl MinimapPalette {
    pub const SEWER: Self = Self {
        bg: (14, 22, 20),
        bg_alt: (18, 28, 26),
        wall: (40, 70, 64),
        wall_edge: (70, 120, 110),
        accent: (90, 220, 200),
        blood: (120, 200, 90),
    };

    pub const KITCHEN: Self = Self {
        bg: (26, 20, 16),
        bg_alt: (34, 26, 20),
        wall: (110, 80, 50),
        wall_edge: (170, 130, 80),
        accent: (255, 180, 90),
        blood: (200, 60, 50),
    };

    pub const LAB: Self = Self {
        bg: (12, 14, 24),
        bg_alt: (16, 20, 34),
        wall: (40, 50, 90),
        wall_edge: (90, 120, 220),
        accent: (120, 220, 255),
        blood: (90, 200, 255),
    };
}

/// Display mode for the minimap widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MinimapMode {
    /// Local corner radar floating over the bottom-right of the playfield.
    #[default]
    Compact,
    /// Full-level Tab cartography modal with cache status, labels, and legend.
    Expanded,
}

/// Player-selectable size state for the compact corner minimap.
///
/// Cycled during play with `N` (`Normal -> Small -> Hidden -> Normal`) or
/// stepped with the mouse wheel over the compact radar. Persisted across runs
/// in the game's data store (`Normal` by default). On terminals too small for
/// `Normal`, [`effective_compact_size`] falls back automatically to `Small` or
/// `Hidden` without overwriting the saved preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MinimapSize {
    /// Full corner radar (`24×10` on `80×24`, `34×14` on `120×40`, `44×18` on `200×60`).
    #[default]
    Normal,
    /// Tight tactical corner radar (`14×7` on `57×20`/`80×24`, `18×9` on `120×40`, `24×11` on `200×60`):
    /// walls, the rat, boss/arena bearing, and seen maze entrances without viewport or pickup clutter.
    Small,
    /// Hidden: clears the playfield (leaving only a 1-cell corner hint, or the
    /// pre-warning arena bearing chevron when the HUD compass is not active).
    Hidden,
}

impl MinimapSize {
    /// Cycle through the three compact states (`Normal -> Small -> Hidden -> Normal`).
    #[inline]
    pub const fn cycle(self) -> Self {
        match self {
            Self::Normal => Self::Small,
            Self::Small => Self::Hidden,
            Self::Hidden => Self::Normal,
        }
    }

    /// Step the compact size from a scroll-wheel delta (`delta > 0` shrinks
    /// toward `Hidden`, `delta < 0` grows toward `Normal`), clamping at both ends.
    #[inline]
    pub const fn step(self, delta: i32) -> Self {
        if delta > 0 {
            match self {
                Self::Normal => Self::Small,
                Self::Small | Self::Hidden => Self::Hidden,
            }
        } else if delta < 0 {
            match self {
                Self::Hidden => Self::Small,
                Self::Small | Self::Normal => Self::Normal,
            }
        } else {
            self
        }
    }

    /// Canonical lowercase string persisted in the settings/score store.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Small => "small",
            Self::Hidden => "hidden",
        }
    }

    /// Parse a persisted setting string (case-insensitive); returns `None` for
    /// unrecognized values so callers fall back cleanly to [`Self::Normal`].
    pub fn parse_setting(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("normal") {
            Some(Self::Normal)
        } else if trimmed.eq_ignore_ascii_case("small") {
            Some(Self::Small)
        } else if trimmed.eq_ignore_ascii_case("hidden") {
            Some(Self::Hidden)
        } else {
            None
        }
    }

    /// Short uppercase status line for the expanded map's sidebar.
    #[inline]
    const fn sidebar_status(self) -> &'static str {
        match self {
            Self::Normal => "RADAR: NORMAL",
            Self::Small => "RADAR: SMALL",
            Self::Hidden => "RADAR: HIDDEN",
        }
    }
}

/// Axis-aligned rectangle in map cell coordinates (`world_px / 16.0`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CellRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Pickup category for active world pickup blips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickupTag {
    Heal,
    Magnet,
    Nuke,
    Chest,
}

/// Category of a minimap landmark or entity blip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    /// Active boss entity (pulsing rose blip; shown in compact & expanded).
    Boss,
    /// Discovered boss arena anchor (shown on expanded map once seen).
    BossArena,
    /// Bearing toward the undiscovered boss arena placed on the fog boundary
    /// (typically enabled from 180 s into the run). Never uncovers terrain.
    ArenaBearing,
    /// Seen maze entrance breach (only drawn if its cell is `seen`).
    /// Shown as a found fork in compact mode and a breach pip in expanded mode.
    MazeEntrance,
    /// Unclaimed maze cache status (`◇`), remembered once the maze entrance has
    /// been seen. Shown on the expanded Tab map.
    MazeCacheHollow,
    /// Claimed maze cache status (`◆` in mint). Shown on the expanded Tab map.
    MazeCacheClaimed,
    /// Visible maze reward pedestal inside a maze once its exact cell is seen.
    /// Strictly gated on `source.seen(cx, cy)`.
    MazeReward,
    /// Active pickup on the floor once its cell is seen.
    Pickup(PickupTag),
    /// Start room origin marker (expanded map).
    Start,
}

/// A point-of-interest blip in map cell coordinates.
#[derive(Clone, Copy, Debug)]
pub struct MapMarker {
    pub cx: f32,
    pub cy: f32,
    pub kind: MarkerKind,
}

/// Category of a discovered region label on the expanded map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Start,
    BossArena,
    Maze,
    Hall,
}

/// A room or arena name shown on the expanded Tab map once discovered.
#[derive(Clone, Copy, Debug)]
pub struct MapLabel<'a> {
    pub cx: f32,
    pub cy: f32,
    pub text: &'a str,
    pub kind: LabelKind,
    /// Must be `true` (and `(cx, cy)` must be `seen`) before the label draws.
    pub discovered: bool,
}

/// Per-frame view snapshot passed to [`draw`] / [`draw_into`].
pub struct MinimapView<'a> {
    /// Player position in map cell coordinates (`world_pos / 16.0`).
    pub player_cell: (f32, f32),
    /// Player facing vector `(dx, dy)` (unit length or zero).
    pub player_facing: (f32, f32),
    /// Visible camera viewport rectangle in map cell coordinates.
    pub viewport_cells: CellRect,
    /// Landmarks, cache statuses, seen entrances, boss, bearing, and pickups.
    pub markers: &'a [MapMarker],
    /// Discovered arena/room names (rendered only in [`MinimapMode::Expanded`]).
    pub labels: &'a [MapLabel<'a>],
    /// Optional bearing direction `(dx, dy)` from the player toward the
    /// undiscovered boss arena (set from `elapsed >= 180.0` until discovered).
    /// When `Some`, the renderer projects a ray through `source.seen` to place
    /// an [`MarkerKind::ArenaBearing`] marker on the outer fog boundary without
    /// uncovering the route. Callers may alternatively place an explicit
    /// `MarkerKind::ArenaBearing` entry in `markers`.
    pub boss_bearing_dir: Option<(f32, f32)>,
    /// Theme color palette.
    pub palette: MinimapPalette,
    /// Sector name displayed in the expanded map title bar (e.g. `"SEWER"`).
    pub sector_name: &'a str,
    /// Monotonic animation clock in seconds.
    pub clock: f32,
    /// Compact corner radar or expanded full-level map.
    pub mode: MinimapMode,
    /// Saved compact radar size preference (`Normal`, `Small`, or `Hidden`).
    /// Effective display size falls back automatically on smaller terminals
    /// via [`effective_compact_size`].
    pub compact_size: MinimapSize,
    /// True when the HUD edge compass (`overlay.rs`) is already pointing at
    /// the boss or pre-boss arena warning, so `Hidden` mode does not duplicate
    /// the bearing cue.
    pub hud_compass_active: bool,
    /// True when the mouse cursor hovers the compact minimap hit rect.
    pub hovered: bool,
    /// Optional precomputed exploration percentage (`0..=100`) for the expanded
    /// map readout; if `None`, computed on the fly in expanded mode only.
    pub explored_pct: Option<u8>,
    /// Whether the player has discovered a maze on this level.
    ///
    /// When `false` (on maze-less maps, or on maze-bearing maps before any maze
    /// entrance/reward is seen), maze-specific rows are omitted from the expanded
    /// map's legend and compact legend strip so rare mazes are neither dead
    /// clutter nor spoiled before discovery.
    pub maze_discovered: bool,
}

/// Reusable scratch buffers kept across frames by the caller (e.g. on `App`).
///
/// Pre-allocates enough capacity for the largest compact radar so compact mode
/// performs **zero heap allocations per frame** from the very first frame.
pub struct MinimapScratch {
    /// Half-block RGB pixel grid (`pw × ph`).
    pixels: Vec<Rgb>,
    /// Per-sub-pixel visibility & recency (`0` = unseen, `1..=255` = seen recency).
    vis: Vec<u8>,
    /// Per-sub-pixel semantic class (`KIND_*`) for edge-highlight passes.
    kind: Vec<u8>,
    /// Reusable text buffer for expanded map header / counters.
    text_buf: String,
    /// Reusable text buffer for secondary legend lines.
    sub_buf: String,
}

impl Default for MinimapScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl MinimapScratch {
    /// Create a scratch pad pre-sized for zero-allocation compact rendering.
    pub fn new() -> Self {
        Self {
            pixels: Vec::with_capacity(COMPACT_PREALLOC_PIXELS),
            vis: Vec::with_capacity(COMPACT_PREALLOC_PIXELS),
            kind: Vec::with_capacity(COMPACT_PREALLOC_PIXELS),
            text_buf: String::with_capacity(48),
            sub_buf: String::with_capacity(32),
        }
    }

    fn prepare(&mut self, len: usize) {
        self.pixels.clear();
        self.vis.clear();
        self.kind.clear();
        self.pixels.resize(len, PLATE_RGB);
        self.vis.resize(len, 0);
        self.kind.resize(len, KIND_UNSEEN);
    }
}

// ---- Layout & Hit-Testing --------------------------------------------------

/// Whether the playfield `area` has enough room for the requested compact size.
///
/// - `Normal` requires at least `72 × 18` playfield cells (e.g. `80 × 24` terminal).
/// - `Small` requires at least `48 × 14` playfield cells (includes `57 × 20` terminal).
/// - `Hidden` always fits.
#[inline]
pub fn compact_size_fits(area: Rect, size: MinimapSize) -> bool {
    match size {
        MinimapSize::Normal => area.width >= 72 && area.height >= 18,
        MinimapSize::Small => area.width >= 48 && area.height >= 14,
        MinimapSize::Hidden => true,
    }
}

/// Resolve the effective compact minimap size for `area` without modifying the
/// player's saved `preferred` setting.
///
/// Terminals too small for `Normal` (such as `57 × 20`) automatically step down
/// to `Small`, and tiny terminals (`< 48 × 14`) step down to `Hidden`.
#[inline]
pub fn effective_compact_size(area: Rect, preferred: MinimapSize) -> MinimapSize {
    match preferred {
        MinimapSize::Normal => {
            if compact_size_fits(area, MinimapSize::Normal) {
                MinimapSize::Normal
            } else if compact_size_fits(area, MinimapSize::Small) {
                MinimapSize::Small
            } else {
                MinimapSize::Hidden
            }
        }
        MinimapSize::Small => {
            if compact_size_fits(area, MinimapSize::Small) {
                MinimapSize::Small
            } else {
                MinimapSize::Hidden
            }
        }
        MinimapSize::Hidden => MinimapSize::Hidden,
    }
}

/// Returns the outer screen [`Rect`] occupied by the compact corner minimap
/// inside `area` for `preferred` (automatically clamped to
/// [`effective_compact_size`]).
///
/// Returns an empty `Rect::default()` when effective size is [`MinimapSize::Hidden`]
/// (explicitly hidden or terminal `< 48 × 14`), leaving no click blocker over
/// the playfield.
///
/// Anchored in the **bottom-right** corner inset by 1 column and 1 row so it
/// never collides with top-center boss bars, top-right callout pills, the
/// bottom-center critical-HP banner, or corner low-HP brackets.
pub fn compact_rect_for_size(area: Rect, preferred: MinimapSize) -> Rect {
    let (w, h) = match effective_compact_size(area, preferred) {
        MinimapSize::Hidden => return Rect::default(),
        MinimapSize::Normal => {
            if area.width >= 160 && area.height >= 44 {
                // Large (e.g. 200x60 -> game 200x58): inner 42x16 cells = 42x32 map cells.
                (44u16, 18u16)
            } else if area.width >= 106 && area.height >= 28 {
                // Medium (e.g. 120x40 -> game 120x38): inner 32x12 cells = 32x24 map cells.
                (34u16, 14u16)
            } else {
                // Standard (e.g. 80x24 -> game 80x22): inner 22x8 cells = 22x16 map cells.
                (24u16, 10u16)
            }
        }
        MinimapSize::Small => {
            if area.width >= 160 && area.height >= 44 {
                // Large small radar: inner 22x9 cells = 22x18 map cells (33% of normal area).
                (24u16, 11u16)
            } else if area.width >= 106 && area.height >= 28 {
                // Medium small radar (120x40): inner 16x7 cells = 16x14 map cells (34% of normal).
                (18u16, 9u16)
            } else {
                // Standard & narrow small radar (80x24 & 57x20): inner 12x5 cells = 12x10 map cells.
                (14u16, 7u16)
            }
        }
    };
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    if w < 14 || h < 7 {
        return Rect::default();
    }
    let x = area.right().saturating_sub(w + 1).max(area.x);
    let y = area.bottom().saturating_sub(h + 1).max(area.y);
    Rect::new(x, y, w, h)
}

/// Returns the outer screen [`Rect`] occupied by the default (`Normal`, falling
/// back to `Small` / `Hidden` on narrow terminals) compact corner minimap.
pub fn compact_rect(area: Rect) -> Rect {
    compact_rect_for_size(area, MinimapSize::Normal)
}

/// Returns the outer [`Rect`] occupied by the expanded Tab map modal inside `area`.
///
/// Returns an empty `Rect::default()` only on unusable terminals (`< 32 × 14`).
pub fn expanded_rect(area: Rect) -> Rect {
    if area.width < 32 || area.height < 14 {
        return Rect::default();
    }
    let (max_w, max_h) = if area.width >= 160 && area.height >= 46 {
        (136u16, 44u16)
    } else if area.width >= 108 && area.height >= 32 {
        (112u16, 34u16)
    } else if area.width >= 74 && area.height >= 20 {
        (76u16, 20u16)
    } else {
        (area.width.saturating_sub(2), area.height.saturating_sub(2))
    };
    let w = max_w.min(area.width.saturating_sub(2)).max(30).min(area.width);
    let h = max_h.min(area.height.saturating_sub(2)).max(12).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Cast a ray from `from` along direction `dir = (dx, dy)` through `source.seen`
/// and return the **outer fog boundary** cell — the furthest `seen` cell whose
/// immediate next step along the ray enters unseen fog.
///
/// Never calls [`MinimapSource::cell_at`], and returns `None` if `dir` is zero
/// or if no unseen fog exists along that ray.
pub fn fog_edge_cell(
    source: &(impl MinimapSource + ?Sized),
    from: (f32, f32),
    dir: (f32, f32),
) -> Option<(f32, f32)> {
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    if len < 1e-4 {
        return None;
    }
    let ux = dir.0 / len;
    let uy = dir.1 / len;
    let cols = source.cols();
    let rows = source.rows();
    if cols <= 0 || rows <= 0 {
        return None;
    }
    let max_dist = ((cols as f32).hypot(rows as f32)).min(400.0);
    let step = 0.75f32;
    let mut best: Option<(f32, f32)> = None;
    let mut prev_seen = false;
    let mut prev_pos = from;
    let mut t = 0.0f32;
    while t <= max_dist {
        let fx = from.0 + ux * t;
        let fy = from.1 + uy * t;
        let cx = fx.floor() as i32;
        let cy = fy.floor() as i32;
        if cx < 0 || cy < 0 || cx >= cols || cy >= rows {
            break;
        }
        let s = source.seen(cx, cy);
        if prev_seen && !s {
            best = Some(prev_pos);
        }
        prev_seen = s;
        if s {
            prev_pos = (fx, fy);
        }
        t += step;
    }
    best
}

// ---- Public Draw Entry Points ----------------------------------------------

/// Render the minimap (Compact or Expanded per `view.mode`) into `frame`.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    view: &MinimapView<'_>,
    source: &(impl MinimapSource + ?Sized),
    scratch: &mut MinimapScratch,
) {
    draw_into(frame.buffer_mut(), area, view, source, scratch);
}

/// Render the minimap directly into a `ratatui::buffer::Buffer`.
pub fn draw_into(
    buf: &mut Buffer,
    area: Rect,
    view: &MinimapView<'_>,
    source: &(impl MinimapSource + ?Sized),
    scratch: &mut MinimapScratch,
) {
    match view.mode {
        MinimapMode::Compact => draw_compact(buf, area, view, source, scratch),
        MinimapMode::Expanded => draw_expanded(buf, area, view, source, scratch),
    }
}

/// Render only the compact corner radar into `buf` (zero heap allocation after init).
///
/// Honors `view.compact_size` clamped via [`effective_compact_size`]:
/// - `Normal`: full local 1:1 radar with viewport brackets and all compact markers.
/// - `Small`: tighter corner radar keeping walls, the rat, boss/arena bearing,
///   and seen maze entrances without viewport brackets or pickup clutter.
/// - `Hidden`: clears the playfield except a 1-cell bottom-right corner hint
///   (or the pre-warning arena bearing chevron when the HUD compass is not active).
pub fn draw_compact(
    buf: &mut Buffer,
    area: Rect,
    view: &MinimapView<'_>,
    source: &(impl MinimapSource + ?Sized),
    scratch: &mut MinimapScratch,
) {
    let effective = effective_compact_size(area, view.compact_size);
    if effective == MinimapSize::Hidden {
        draw_hidden_hint(buf, area, view);
        return;
    }

    let rect = compact_rect_for_size(area, effective);
    if rect.is_empty() || source.cols() <= 0 || source.rows() <= 0 {
        return;
    }

    let inner = Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    );
    if inner.is_empty() {
        return;
    }

    let is_small = effective == MinimapSize::Small;
    let pw = inner.width as usize;
    let ph = (inner.height as usize) * 2;
    scratch.prepare(pw * ph);

    let cols = source.cols();
    let rows = source.rows();
    let theme = DerivedPalette::from_palette(view.palette);

    // Normal renders 1:1 cells-per-pixel; Small uses feature-preserving 2×2
    // downsampling so room walls, doorways, and nearby maze entrances remain
    // visible even inside a 14×7 corner box.
    let step: i32 = if is_small { 2 } else { 1 };
    let inv_step = 1.0 / step as f32;
    let span_w = (pw as i32 * step) as f32;
    let span_h = (ph as i32 * step) as f32;
    let max_ox = (cols as f32 - span_w).max(0.0);
    let max_oy = (rows as f32 - span_h).max(0.0);
    let origin_x = (view.player_cell.0 - span_w * 0.5)
        .clamp(0.0, max_ox)
        .floor() as i32;
    let origin_y = (view.player_cell.1 - span_h * 0.5)
        .clamp(0.0, max_oy)
        .floor() as i32;

    // 1. Sample cells into pixel + visibility + kind buffers (strictly skipping unseen cell_at!).
    if !is_small {
        for py in 0..ph {
            let cy = origin_y + py as i32;
            for px in 0..pw {
                let cx = origin_x + px as i32;
                let idx = py * pw + px;
                if cx < 0 || cy < 0 || cx >= cols || cy >= rows || !source.seen(cx, cy) {
                    scratch.vis[idx] = 0;
                    scratch.kind[idx] = KIND_UNSEEN;
                    scratch.pixels[idx] = fog_void_color(&theme, cx, cy);
                    continue;
                }
                let rec = source.recency(cx, cy).clamp(0.0, 1.0);
                scratch.vis[idx] = encode_vis(rec);
                let cell = source.cell_at(cx, cy);
                let (base_col, k) = classify_compact_cell(&theme, cell, cx, cy, source);
                scratch.kind[idx] = k;
                scratch.pixels[idx] = apply_recency_shading(&theme, base_col, rec);
            }
        }
    } else {
        for py in 0..ph {
            let cy0 = origin_y + (py as i32) * 2;
            for px in 0..pw {
                let cx0 = origin_x + (px as i32) * 2;
                let idx = py * pw + px;

                let mut seen_n = 0u32;
                let mut max_rec = 0.0f32;
                let mut floor_n = 0u32;
                let mut boss_floor_n = 0u32;
                let mut wall_n = 0u32;
                let mut maze_n = 0u32;
                let mut door_n = 0u32;
                let mut one_way_n = 0u32;
                let mut hole_linked_n = 0u32;
                let mut hole_near_n = 0u32;

                for dy in 0..2 {
                    let cy = cy0 + dy;
                    if cy < 0 || cy >= rows {
                        continue;
                    }
                    for dx in 0..2 {
                        let cx = cx0 + dx;
                        if cx < 0 || cx >= cols || !source.seen(cx, cy) {
                            continue;
                        }
                        seen_n += 1;
                        let r = source.recency(cx, cy).clamp(0.0, 1.0);
                        if r > max_rec {
                            max_rec = r;
                        }
                        match source.cell_at(cx, cy) {
                            MinimapCell::Unknown => {}
                            // Traps drop out in Small so amber stays exclusive to the bearing.
                            MinimapCell::Floor | MinimapCell::Trap => floor_n += 1,
                            MinimapCell::BossFloor => boss_floor_n += 1,
                            MinimapCell::Wall => wall_n += 1,
                            MinimapCell::MazeWall => maze_n += 1,
                            MinimapCell::Doorway => door_n += 1,
                            MinimapCell::OneWayDoor(_) => one_way_n += 1,
                            MinimapCell::RatHole(axis) => {
                                if source.rat_hole_far_known(cx, cy, axis) {
                                    hole_linked_n += 1;
                                } else {
                                    hole_near_n += 1;
                                }
                            }
                        }
                    }
                }

                if seen_n == 0 {
                    scratch.vis[idx] = 0;
                    scratch.kind[idx] = KIND_UNSEEN;
                    scratch.pixels[idx] = fog_void_color(&theme, px as i32, py as i32);
                    continue;
                }

                scratch.vis[idx] = encode_vis(max_rec);
                let floor_col = if boss_floor_n > 0 && boss_floor_n >= floor_n {
                    theme.boss_floor
                } else {
                    theme.floor_live
                };
                let (base_col, k) = if one_way_n > 0 {
                    (ONE_WAY_EXIT_RGB, KIND_FEATURE)
                } else if hole_linked_n > 0 {
                    (RAT_HOLE_LINKED_RGB, KIND_FEATURE)
                } else if hole_near_n > 0 {
                    (RAT_HOLE_NEAR_RGB, KIND_FEATURE)
                } else if door_n > 0 {
                    (theme.doorway, KIND_FLOOR)
                } else if maze_n > 0 && (floor_n + boss_floor_n) > 0 {
                    let t = (maze_n as f32 / (maze_n + floor_n + boss_floor_n) as f32)
                        .clamp(0.45, 0.85);
                    (palette::mix(floor_col, theme.maze_wall, t), KIND_FEATURE)
                } else if maze_n > 0 {
                    (theme.maze_wall, KIND_FEATURE)
                } else if wall_n > 0 && (floor_n + boss_floor_n) > 0 {
                    (theme.wall_rim, KIND_FEATURE)
                } else if wall_n > 0 {
                    (theme.wall_mass, KIND_WALL)
                } else {
                    (floor_col, KIND_FLOOR)
                };
                scratch.kind[idx] = k;
                scratch.pixels[idx] = apply_recency_shading(&theme, base_col, max_rec);
            }
        }
    }

    // 2. Architectural wall-rim elevation & fog-of-war frontier mist.
    apply_wall_rims_and_fog_frontier(scratch, pw, ph, &theme);

    // 3. Delicate corner-bracketed viewport frame (Normal only — in Small the
    //    tight window itself is the immediate area around the rat).
    if !is_small {
        draw_viewport_brackets(
            &mut scratch.pixels,
            pw,
            ph,
            view.viewport_cells,
            origin_x as f32,
            origin_y as f32,
            1.0,
            1.0,
            &theme,
            true,
        );
    }

    // 4. Compact markers:
    //    - Normal: found forks (seen maze entrances), live boss, seen rewards,
    //      seen pickups, and fog-edge arena bearing.
    //    - Small: only live boss, arena bearing, and seen maze entrances so
    //      every bright pixel reads immediately on the smaller canvas.
    let pulse = 0.5 + 0.5 * (view.clock * 6.0).sin();
    let fast_pulse = 0.5 + 0.5 * (view.clock * 9.5).sin();

    let mut edge_boss: Option<(f32, f32)> = None;
    let mut edge_bearing: Option<(f32, f32)> = None;

    for m in view.markers {
        match m.kind {
            MarkerKind::MazeCacheHollow
            | MarkerKind::MazeCacheClaimed
            | MarkerKind::BossArena
            | MarkerKind::Start => {
                // Designer rule (4): cache status & arena names belong on the expanded Tab map.
                continue;
            }
            MarkerKind::MazeEntrance => {
                let (mcx, mcy) = (m.cx.floor() as i32, m.cy.floor() as i32);
                if !in_bounds(source, mcx, mcy) || !source.seen(mcx, mcy) {
                    continue;
                }
                let col = palette::mix(theme.accent, COAT_RGB, 0.30 + 0.25 * pulse);
                plot_pixel_in_window(
                    &mut scratch.pixels,
                    pw,
                    ph,
                    (m.cx - origin_x as f32) * inv_step,
                    (m.cy - origin_y as f32) * inv_step,
                    col,
                );
            }
            MarkerKind::MazeReward => {
                if is_small {
                    continue;
                }
                let (mcx, mcy) = (m.cx.floor() as i32, m.cy.floor() as i32);
                if !in_bounds(source, mcx, mcy) || !source.seen(mcx, mcy) {
                    continue;
                }
                let col = palette::mix(CACHE_GOLD_RGB, COAT_RGB, 0.35 * pulse);
                plot_pixel_in_window(
                    &mut scratch.pixels,
                    pw,
                    ph,
                    (m.cx - origin_x as f32) * inv_step,
                    (m.cy - origin_y as f32) * inv_step,
                    col,
                );
            }
            MarkerKind::Pickup(tag) => {
                if is_small && tag != PickupTag::Chest {
                    continue;
                }
                let (mcx, mcy) = (m.cx.floor() as i32, m.cy.floor() as i32);
                if !in_bounds(source, mcx, mcy) || !source.seen(mcx, mcy) {
                    continue;
                }
                let base = match tag {
                    PickupTag::Heal => CLAIMED_MINT_RGB,
                    PickupTag::Magnet => MAGNET_ICE_RGB,
                    PickupTag::Nuke => BOSS_HOT_RGB,
                    PickupTag::Chest => (255, 204, 83),
                };
                plot_pixel_in_window(
                    &mut scratch.pixels,
                    pw,
                    ph,
                    (m.cx - origin_x as f32) * inv_step,
                    (m.cy - origin_y as f32) * inv_step,
                    base,
                );
            }
            MarkerKind::Boss => {
                let lx = (m.cx - origin_x as f32) * inv_step;
                let ly = (m.cy - origin_y as f32) * inv_step;
                let col = palette::mix(BOSS_ROSE_RGB, BOSS_HOT_RGB, fast_pulse);
                if !plot_pixel_in_window(&mut scratch.pixels, pw, ph, lx, ly, col) {
                    edge_boss = Some((m.cx - view.player_cell.0, m.cy - view.player_cell.1));
                }
            }
            MarkerKind::ArenaBearing => {
                let lx = (m.cx - origin_x as f32) * inv_step;
                let ly = (m.cy - origin_y as f32) * inv_step;
                let col = palette::mix(WARN_AMBER_RGB, CACHE_GOLD_RGB, 0.35 + 0.45 * pulse);
                if !plot_pixel_in_window(&mut scratch.pixels, pw, ph, lx, ly, col) {
                    edge_bearing = Some((m.cx - view.player_cell.0, m.cy - view.player_cell.1));
                }
            }
        }
    }

    // Automatic fog-edge boss bearing projection when `boss_bearing_dir` is set.
    if let Some(dir) = view.boss_bearing_dir {
        if let Some((bx, by)) = fog_edge_cell(source, view.player_cell, dir) {
            let lx = (bx - origin_x as f32) * inv_step;
            let ly = (by - origin_y as f32) * inv_step;
            let col = palette::mix(WARN_AMBER_RGB, CACHE_GOLD_RGB, 0.35 + 0.45 * pulse);
            if !plot_pixel_in_window(&mut scratch.pixels, pw, ph, lx, ly, col) {
                edge_bearing = Some(dir);
            }
        } else {
            edge_bearing = Some(dir);
        }
    }

    // 5. Player position + subtle directional facing pip (in Normal; Small uses
    //    a crisp single-pixel rat blip so nearby doorways/walls stay clear).
    let plx = (view.player_cell.0 - origin_x as f32) * inv_step;
    let ply = (view.player_cell.1 - origin_y as f32) * inv_step;
    if !is_small {
        let flen = (view.player_facing.0 * view.player_facing.0
            + view.player_facing.1 * view.player_facing.1)
            .sqrt();
        if flen > 0.1 {
            let fx = plx + (view.player_facing.0 / flen) * 1.15;
            let fy = ply + (view.player_facing.1 / flen) * 1.15;
            plot_pixel_in_window(&mut scratch.pixels, pw, ph, fx, fy, TAIL_RGB);
        }
    }
    let player_col = palette::mix(COAT_RGB, (255, 255, 255), 0.35 * pulse);
    plot_pixel_in_window(&mut scratch.pixels, pw, ph, plx, ply, player_col);

    // 6. Stamped-metal frame (matching `menu::panel` top-left catchlight).
    let accent_color = palette::marking(view.palette.wall_edge, view.palette.accent);
    let border_lit = if view.hovered { COAT } else { accent_color };
    draw_stamped_frame(buf, rect, border_lit, TRACK);

    let title_style = Style::default()
        .fg(if view.hovered { COAT } else { border_lit })
        .bg(PLATE)
        .add_modifier(Modifier::BOLD);
    buf.set_stringn(
        rect.x + 2,
        rect.y,
        " MAP ",
        rect.width.saturating_sub(4) as usize,
        title_style,
    );
    let hint_opt = if rect.width >= 18 {
        Some(if view.hovered { " ▸TAB " } else { " N·TAB " })
    } else if rect.width >= 14 {
        Some(if view.hovered { " ▸TAB " } else { " N " })
    } else {
        None
    };
    if let Some(hint) = hint_opt {
        let hw = hint.chars().count() as u16;
        let hx = rect.right().saturating_sub(hw + 1);
        if hx >= rect.x + 7 {
            let hint_style = if view.hovered {
                Style::default()
                    .fg(PLATE)
                    .bg(TAIL)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED).bg(PLATE)
            };
            buf.set_stringn(hx, rect.y, hint, hw as usize, hint_style);
        }
    }

    // 7. Blit half-block pixel buffer into `inner`.
    blit_half_blocks(buf, inner, &scratch.pixels, pw);

    // 8. Draw off-window 8-way directional bearing / boss chevron on the compact rim.
    if let Some((dx, dy)) = edge_bearing {
        draw_rim_chevron(
            buf,
            rect,
            dx,
            dy,
            palette::to_color(palette::mix(WARN_AMBER_RGB, CACHE_GOLD_RGB, 0.35 + 0.45 * pulse)),
        );
    }
    if let Some((dx, dy)) = edge_boss {
        draw_rim_chevron(
            buf,
            rect,
            dx,
            dy,
            palette::to_color(palette::mix(BOSS_ROSE_RGB, COAT_RGB, 0.4 * fast_pulse)),
        );
    }
}

/// Hidden compact state: leaves the playfield clear except a single bottom-right
/// corner cell (`N` key hint, or the 8-way amber arena bearing chevron during
/// the 180–230 s pre-warning window when the HUD compass is not yet active).
/// Draws nothing at all on unusable terminals where even the expanded map
/// cannot open (`< 32 × 14`). Zero allocations; never reads unseen cells.
fn draw_hidden_hint(buf: &mut Buffer, area: Rect, view: &MinimapView<'_>) {
    if expanded_rect(area).is_empty() || area.width < 2 || area.height < 2 {
        return;
    }
    let x = area.right() - 2;
    let y = area.bottom() - 2;

    if !view.hud_compass_active {
        let bearing = view.boss_bearing_dir.or_else(|| {
            view.markers
                .iter()
                .find(|m| m.kind == MarkerKind::ArenaBearing)
                .map(|m| (m.cx - view.player_cell.0, m.cy - view.player_cell.1))
        });
        if let Some((dx, dy)) = bearing {
            if dx.abs() >= 1e-4 || dy.abs() >= 1e-4 {
                let pulse = 0.5 + 0.5 * (view.clock * 6.0).sin();
                let fg = palette::to_color(palette::mix(
                    WARN_AMBER_RGB,
                    CACHE_GOLD_RGB,
                    0.35 + 0.45 * pulse,
                ));
                buf[(x, y)].set_char(dir_chevron_8(dx, dy)).set_style(
                    Style::default()
                        .fg(fg)
                        .bg(PLATE)
                        .add_modifier(Modifier::BOLD),
                );
                return;
            }
        }
    }

    // Quiet 1-cell hint that a map exists and `N` cycles it back.
    buf[(x, y)]
        .set_char('N')
        .set_style(Style::default().fg(EDGE).bg(PLATE));
}

/// Render the expanded full-level Tab cartography modal into `buf`.
pub fn draw_expanded(
    buf: &mut Buffer,
    area: Rect,
    view: &MinimapView<'_>,
    source: &(impl MinimapSource + ?Sized),
    scratch: &mut MinimapScratch,
) {
    let modal = expanded_rect(area);
    if modal.is_empty() || source.cols() <= 0 || source.rows() <= 0 {
        return;
    }

    let accent_color = palette::marking(view.palette.wall_edge, view.palette.accent);
    let theme = DerivedPalette::from_palette(view.palette);

    // Fill modal interior with dark metal `PLATE` and draw stamped border.
    for y in modal.y..modal.bottom() {
        for x in modal.x..modal.right() {
            buf[(x, y)].set_char(' ').set_fg(COAT).set_bg(PLATE);
        }
    }
    draw_stamped_frame(buf, modal, accent_color, TRACK);

    let inner = Rect::new(
        modal.x + 1,
        modal.y + 1,
        modal.width.saturating_sub(2),
        modal.height.saturating_sub(2),
    );
    if inner.width < 12 || inner.height < 6 {
        return;
    }

    let cols = source.cols();
    let rows = source.rows();

    let has_sidebar = inner.width >= 50 && inner.height >= 14;
    let sidebar_w: u16 = if !has_sidebar {
        0
    } else if inner.width >= 120 {
        32
    } else if inner.width >= 96 {
        28
    } else if inner.width >= 68 {
        25
    } else {
        17
    };

    let map_box = if has_sidebar {
        Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(sidebar_w + 1),
            inner.height,
        )
    } else {
        Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1))
    };

    let (map_rect, pw, ph) = fit_map_canvas(map_box, cols, rows);
    if pw == 0 || ph == 0 {
        return;
    }

    scratch.prepare(pw * ph);
    let sx = cols as f32 / pw as f32;
    let sy = rows as f32 / ph as f32;

    // 1. Feature-preserving downsample of the entire map (zero unseen cell_at calls!).
    let mut total_seen_blocks = 0usize;
    let mut total_blocks = 0usize;

    for py in 0..ph {
        let cy0 = ((py as f32) * sy).floor() as i32;
        let cy1 = (((py + 1) as f32) * sy).ceil() as i32;
        let cy0 = cy0.clamp(0, rows - 1);
        let cy1 = cy1.clamp(cy0 + 1, rows);
        for px in 0..pw {
            let cx0 = ((px as f32) * sx).floor() as i32;
            let cx1 = (((px + 1) as f32) * sx).ceil() as i32;
            let cx0 = cx0.clamp(0, cols - 1);
            let cx1 = cx1.clamp(cx0 + 1, cols);
            let idx = py * pw + px;
            total_blocks += 1;

            let mut seen_n = 0u32;
            let mut max_rec = 0.0f32;
            let mut floor_n = 0u32;
            let mut boss_floor_n = 0u32;
            let mut wall_n = 0u32;
            let mut maze_n = 0u32;
            let mut door_n = 0u32;
            let mut one_way_n = 0u32;
            let mut hole_linked_n = 0u32;
            let mut hole_near_n = 0u32;
            let mut trap_n = 0u32;

            for cy in cy0..cy1 {
                for cx in cx0..cx1 {
                    if !source.seen(cx, cy) {
                        continue;
                    }
                    seen_n += 1;
                    let r = source.recency(cx, cy).clamp(0.0, 1.0);
                    if r > max_rec {
                        max_rec = r;
                    }
                    match source.cell_at(cx, cy) {
                        MinimapCell::Unknown => {}
                        MinimapCell::Floor => floor_n += 1,
                        MinimapCell::BossFloor => boss_floor_n += 1,
                        MinimapCell::Wall => wall_n += 1,
                        MinimapCell::MazeWall => maze_n += 1,
                        MinimapCell::Doorway => door_n += 1,
                        MinimapCell::OneWayDoor(_) => one_way_n += 1,
                        MinimapCell::RatHole(axis) => {
                            if source.rat_hole_far_known(cx, cy, axis) {
                                hole_linked_n += 1;
                            } else {
                                hole_near_n += 1;
                            }
                        }
                        MinimapCell::Trap => trap_n += 1,
                    }
                }
            }

            if seen_n == 0 {
                scratch.vis[idx] = 0;
                scratch.kind[idx] = KIND_UNSEEN;
                scratch.pixels[idx] = fog_void_color(&theme, px as i32, py as i32);
                continue;
            }

            total_seen_blocks += 1;
            scratch.vis[idx] = encode_vis(max_rec);

            let floor_col = if boss_floor_n > 0 && boss_floor_n >= floor_n {
                theme.boss_floor
            } else {
                theme.floor_live
            };

            let (base_col, k) = if one_way_n > 0 {
                (ONE_WAY_EXIT_RGB, KIND_FEATURE)
            } else if hole_linked_n > 0 {
                (RAT_HOLE_LINKED_RGB, KIND_FEATURE)
            } else if hole_near_n > 0 {
                (RAT_HOLE_NEAR_RGB, KIND_FEATURE)
            } else if door_n > 0 {
                (theme.doorway, KIND_FLOOR)
            } else if maze_n > 0 && (floor_n + boss_floor_n) > 0 {
                let t = (maze_n as f32 / (maze_n + floor_n + boss_floor_n) as f32).clamp(0.38, 0.85);
                (palette::mix(floor_col, theme.maze_wall, t), KIND_FEATURE)
            } else if maze_n > 0 {
                (theme.maze_wall, KIND_FEATURE)
            } else if wall_n > 0 && (floor_n + boss_floor_n) > 0 {
                if wall_n * 2 >= seen_n {
                    (theme.wall_rim, KIND_FEATURE)
                } else {
                    (palette::mix(floor_col, theme.wall_rim, 0.34), KIND_FLOOR)
                }
            } else if wall_n > 0 {
                (theme.wall_mass, KIND_WALL)
            } else if trap_n > 0 {
                (palette::mix(floor_col, WARN_AMBER_RGB, 0.42), KIND_FLOOR)
            } else {
                (floor_col, KIND_FLOOR)
            };

            scratch.kind[idx] = k;
            scratch.pixels[idx] = apply_recency_shading(&theme, base_col, max_rec);
        }
    }

    // 2. Architectural wall-rim elevation & crisp fog-of-war frontier mist.
    apply_wall_rims_and_fog_frontier(scratch, pw, ph, &theme);

    // 3. Soft viewport brackets on the full map (skipped when collapsed to <=3 px).
    let pulse = 0.5 + 0.5 * (view.clock * 5.0).sin();
    draw_viewport_brackets(
        &mut scratch.pixels,
        pw,
        ph,
        view.viewport_cells,
        0.0,
        0.0,
        1.0 / sx,
        1.0 / sy,
        &theme,
        false,
    );

    // Cast a soft warm-amber glow into the *unseen fog* beside the bearing beacon
    // (never tinting explored walls/rooms behind it).
    let bearing_pos: Option<(f32, f32, f32, f32)> = view
        .markers
        .iter()
        .find(|m| m.kind == MarkerKind::ArenaBearing)
        .map(|m| (m.cx, m.cy, m.cx - view.player_cell.0, m.cy - view.player_cell.1))
        .or_else(|| {
            view.boss_bearing_dir.and_then(|dir| {
                fog_edge_cell(source, view.player_cell, dir)
                    .map(|(bx, by)| (bx, by, dir.0, dir.1))
            })
        });
    if let Some((bx, by, _, _)) = bearing_pos {
        let px = ((bx / sx).floor() as isize).clamp(0, pw as isize - 1) as usize;
        let py = ((by / sy).floor() as isize).clamp(0, ph as isize - 1) as usize;
        let glow = palette::mix(theme.fog_base, WARN_AMBER_RGB, 0.34 + 0.16 * pulse);
        for dy in -2isize..=2 {
            for dx in -2isize..=2 {
                let d2 = dx * dx + dy * dy;
                if d2 == 0 || d2 > 4 {
                    continue;
                }
                let nx = px as isize + dx;
                let ny = py as isize + dy;
                if nx >= 0 && ny >= 0 && nx < pw as isize && ny < ph as isize {
                    let ni = ny as usize * pw + nx as usize;
                    if scratch.vis[ni] == 0 {
                        let falloff = if d2 <= 1 { 0.65 } else { 0.30 };
                        scratch.pixels[ni] = palette::mix(scratch.pixels[ni], glow, falloff);
                    }
                }
            }
        }
    }

    // 4. Blit half-block terrain canvas into `map_rect`.
    blit_half_blocks(buf, map_rect, &scratch.pixels, pw);

    // 5. Stamp crisp single-cell symbolic markers & discovered arena names.
    let cell_to_term = |cx: f32, cy: f32| -> Option<(u16, u16)> {
        let px = ((cx / sx).floor() as isize).clamp(0, pw as isize - 1) as u16;
        let py = ((cy / sy).floor() as isize).clamp(0, ph as isize - 1) as u16;
        let tx = map_rect.x + px;
        let ty = map_rect.y + py / 2;
        if tx < map_rect.right() && ty < map_rect.bottom() {
            Some((tx, ty))
        } else {
            None
        }
    };

    let mut caches_unclaimed = 0u32;
    let mut caches_claimed = 0u32;
    let mut boss_arena_discovered = false;
    let mut boss_active = false;
    let mut bearing_active = false;

    let fast_pulse = 0.5 + 0.5 * (view.clock * 8.5).sin();

    // Pass A: secondary landmarks & cache status markers.
    for m in view.markers {
        let (mcx, mcy) = (m.cx.floor() as i32, m.cy.floor() as i32);
        match m.kind {
            MarkerKind::Start => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        stamp_glyph(buf, tx, ty, '◎', palette::to_color(theme.accent), false);
                    }
                }
            }
            MarkerKind::Pickup(tag) => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        let c = match tag {
                            PickupTag::Heal => palette::to_color(CLAIMED_MINT_RGB),
                            PickupTag::Magnet => palette::to_color(MAGNET_ICE_RGB),
                            PickupTag::Nuke => palette::to_color(BOSS_HOT_RGB),
                            PickupTag::Chest => palette::to_color((255, 204, 83)),
                        };
                        let glyph = if tag == PickupTag::Chest { '◆' } else { '•' };
                        stamp_glyph(buf, tx, ty, glyph, c, true);
                    }
                }
            }
            MarkerKind::MazeEntrance => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        stamp_glyph(
                            buf,
                            tx,
                            ty,
                            '▸',
                            palette::to_color(palette::mix(theme.accent, COAT_RGB, 0.3)),
                            true,
                        );
                    }
                }
            }
            MarkerKind::MazeReward => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        let c = palette::to_color(palette::mix(CACHE_GOLD_RGB, COAT_RGB, 0.35 * pulse));
                        stamp_glyph(buf, tx, ty, '✦', c, true);
                    }
                }
            }
            MarkerKind::MazeCacheHollow => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    caches_unclaimed += 1;
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        let c = palette::to_color(palette::mix(CACHE_GOLD_RGB, COAT_RGB, 0.25 * pulse));
                        stamp_glyph(buf, tx, ty, '◇', c, true);
                    }
                }
            }
            MarkerKind::MazeCacheClaimed => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    caches_claimed += 1;
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        stamp_glyph(buf, tx, ty, '◆', palette::to_color(CLAIMED_MINT_RGB), true);
                    }
                }
            }
            MarkerKind::BossArena => {
                if in_bounds(source, mcx, mcy) && source.seen(mcx, mcy) {
                    boss_arena_discovered = true;
                    if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                        stamp_glyph(buf, tx, ty, '◈', palette::to_color(BOSS_ROSE_RGB), true);
                    }
                }
            }
            MarkerKind::ArenaBearing => {
                bearing_active = true;
            }
            MarkerKind::Boss => {
                boss_active = true;
            }
        }
    }

    // Stamp the 8-way fog-edge arena bearing beacon.
    if let Some((bx, by, dx, dy)) = bearing_pos {
        bearing_active = true;
        if let Some((tx, ty)) = cell_to_term(bx, by) {
            let ch = dir_chevron_8(dx, dy);
            let c = palette::to_color(palette::mix(WARN_AMBER_RGB, CACHE_GOLD_RGB, 0.30 + 0.50 * pulse));
            stamp_glyph(buf, tx, ty, ch, c, true);
        }
    }

    // Pass B: discovered room/arena labels (expanded mode only).
    for lbl in view.labels {
        if !lbl.discovered || lbl.text.is_empty() {
            continue;
        }
        let (lcx, lcy) = (lbl.cx.floor() as i32, lbl.cy.floor() as i32);
        if !in_bounds(source, lcx, lcy) || !source.seen(lcx, lcy) {
            continue;
        }
        if let Some((tx, ty)) = cell_to_term(lbl.cx, lbl.cy) {
            let (fg, bg) = match lbl.kind {
                LabelKind::BossArena => (
                    palette::to_color(BOSS_ROSE_RGB),
                    palette::to_color(palette::mix(PLATE_RGB, (96, 24, 48), 0.35)),
                ),
                LabelKind::Start => (COAT, PLATE),
                LabelKind::Maze => (palette::to_color(CACHE_GOLD_RGB), PLATE),
                LabelKind::Hall => (MUTED, PLATE),
            };
            let lw = lbl.text.chars().count() as u16 + 2;
            let ly = if ty + 1 < map_rect.bottom() {
                ty + 1
            } else {
                ty.saturating_sub(1).max(map_rect.y)
            };
            let lx = tx
                .saturating_sub(lw / 2)
                .clamp(map_rect.x, map_rect.right().saturating_sub(lw).max(map_rect.x));
            let avail = map_rect.right().saturating_sub(lx) as usize;
            let style = Style::default()
                .fg(fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD);
            let (nx, _) = buf.set_stringn(lx, ly, " ", avail, style);
            let (nx, _) = buf.set_stringn(
                nx,
                ly,
                lbl.text,
                map_rect.right().saturating_sub(nx) as usize,
                style,
            );
            buf.set_stringn(
                nx,
                ly,
                " ",
                map_rect.right().saturating_sub(nx) as usize,
                style,
            );
        }
    }

    // Pass C: active boss and player position stay topmost.
    for m in view.markers {
        if m.kind == MarkerKind::Boss {
            if let Some((tx, ty)) = cell_to_term(m.cx, m.cy) {
                let c = palette::to_color(palette::mix(BOSS_ROSE_RGB, COAT_RGB, 0.4 * fast_pulse));
                stamp_glyph(buf, tx, ty, '◆', c, true);
            }
        }
    }
    if let Some((tx, ty)) = cell_to_term(view.player_cell.0, view.player_cell.1) {
        let pcol = palette::to_color(palette::mix(COAT_RGB, TAIL_RGB, 0.25 * pulse));
        stamp_glyph(buf, tx, ty, '●', pcol, true);
    }

    // 6. Exploration percentage & top frame header bar.
    let explored_pct = view.explored_pct.unwrap_or_else(|| {
        (total_seen_blocks * 100)
            .checked_div(total_blocks)
            .map_or(0, |p| p.min(100) as u8)
    });

    let sector = if view.sector_name.is_empty() {
        "UNKNOWN"
    } else {
        view.sector_name
    };
    scratch.text_buf.clear();
    if modal.width < 64 {
        let _ = write!(scratch.text_buf, " MAP · {} {}% ", sector, explored_pct);
    } else {
        let _ = write!(
            scratch.text_buf,
            " SECTOR MAP · {} · {}% ",
            sector, explored_pct
        );
    }
    let close_hint = if modal.width < 64 {
        " [TAB] "
    } else {
        " [TAB] CLOSE "
    };
    let cw = close_hint.chars().count() as u16;
    let hdr_max = modal.width.saturating_sub(cw + 4) as usize;
    buf.set_stringn(
        modal.x + 2,
        modal.y,
        &scratch.text_buf,
        hdr_max,
        Style::default()
            .fg(accent_color)
            .bg(PLATE)
            .add_modifier(Modifier::BOLD),
    );
    if modal.width > cw + 6 {
        buf.set_stringn(
            modal.right().saturating_sub(cw + 1),
            modal.y,
            close_hint,
            cw as usize,
            Style::default().fg(MUTED).bg(PLATE),
        );
    }

    // 7. Right tactical sidebar OR compact bottom legend strip.
    if has_sidebar {
        let div_x = inner.right().saturating_sub(sidebar_w + 1);
        for y in inner.y..inner.bottom() {
            buf[(div_x, y)].set_char('│').set_fg(EDGE).set_bg(PLATE);
        }
        let side = Rect::new(div_x + 1, inner.y, sidebar_w, inner.height);
        draw_expanded_sidebar(
            buf,
            side,
            &theme,
            accent_color,
            explored_pct,
            view.maze_discovered,
            caches_unclaimed,
            caches_claimed,
            boss_arena_discovered,
            boss_active,
            bearing_active,
            effective_compact_size(area, view.compact_size),
            scratch,
        );
    } else {
        let ly = inner.bottom().saturating_sub(1);
        let legend = if view.maze_discovered {
            " ●You ◇Cache ◆Done ▸Breach ◤Bear "
        } else {
            " ●You ▪Wall ▪Hole ◈Boss ◤Bear "
        };
        buf.set_stringn(
            inner.x,
            ly,
            legend,
            inner.width as usize,
            Style::default().fg(MUTED).bg(PLATE),
        );
    }
}

// ---- Expanded Sidebar Renderer ---------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_expanded_sidebar(
    buf: &mut Buffer,
    side: Rect,
    theme: &DerivedPalette,
    accent: Color,
    explored_pct: u8,
    maze_discovered: bool,
    caches_unclaimed: u32,
    caches_claimed: u32,
    boss_arena_discovered: bool,
    boss_active: bool,
    bearing_active: bool,
    compact_size: MinimapSize,
    scratch: &mut MinimapScratch,
) {
    if side.width < 12 || side.height < 10 {
        return;
    }
    let compact = side.width < 22;
    let x = side.x + 1;
    let max_w = side.width.saturating_sub(2) as usize;
    let mut y = side.y;
    let bottom = side.bottom();

    // Exploration gauge.
    buf.set_stringn(
        x,
        y,
        if compact { "CHARTED" } else { "SECTOR CHARTED" },
        max_w,
        Style::default().fg(MUTED).bg(PLATE),
    );
    y += 1;
    if y < bottom {
        let bar_w = side.width.saturating_sub(8).min(18) as usize;
        if bar_w >= 4 {
            let fill = ((usize::from(explored_pct) * bar_w) + 50) / 100;
            for i in 0..bar_w {
                let (ch, fg) = if i < fill {
                    ('█', accent)
                } else {
                    ('░', EDGE)
                };
                buf[(x + i as u16, y)].set_char(ch).set_fg(fg).set_bg(TRACK);
            }
            scratch.sub_buf.clear();
            let _ = write!(scratch.sub_buf, " {:>3}%", explored_pct);
            buf.set_stringn(
                x + bar_w as u16,
                y,
                &scratch.sub_buf,
                max_w.saturating_sub(bar_w),
                Style::default()
                    .fg(COAT)
                    .bg(PLATE)
                    .add_modifier(Modifier::BOLD),
            );
        }
        y += 2;
    }

    // Legend entries.
    if y < bottom {
        buf.set_stringn(
            x,
            y,
            "LEGEND",
            max_w,
            Style::default()
                .fg(accent)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }

    let entries: &[(&str, Color, &str, &str, bool)] = &[
        ("●", COAT, "You", "Player", false),
        ("▪", palette::to_color(theme.wall_rim), "Wall", "Wall Edge", false),
        ("▪", palette::to_color(theme.maze_wall), "Maze", "Maze Partition", true),
        ("▪", palette::to_color(RAT_HOLE_LINKED_RGB), "Hole", "Rat-Hole", false),
        ("▪", palette::to_color(ONE_WAY_EXIT_RGB), "Exit", "One-Way Exit", true),
        ("▸", palette::to_color(theme.accent), "Door", "Maze Breach", true),
        ("◇", palette::to_color(CACHE_GOLD_RGB), "Open", "Unclaimed Cache", true),
        ("◆", palette::to_color(CLAIMED_MINT_RGB), "Done", "Claimed Cache", true),
        ("◈", palette::to_color(BOSS_ROSE_RGB), "Boss", "Boss Arena", false),
        ("◤", palette::to_color(WARN_AMBER_RGB), "Bear", "Arena Bearing", false),
    ];

    for &(sym, col, short_lbl, long_lbl, maze_only) in entries {
        if maze_only && !maze_discovered {
            continue;
        }
        if y + 2 >= bottom {
            break;
        }
        buf.set_stringn(
            x,
            y,
            sym,
            2,
            Style::default()
                .fg(col)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        let label = if compact { short_lbl } else { long_lbl };
        buf.set_stringn(
            x + 2,
            y,
            label,
            max_w.saturating_sub(2),
            Style::default().fg(COAT).bg(PLATE),
        );
        y += 1;
    }

    // Status telemetry (caches & boss arena / bearing).
    if y + 2 < bottom {
        y += 1;
        let total_caches = caches_unclaimed + caches_claimed;
        if total_caches > 0 {
            scratch.sub_buf.clear();
            if compact {
                let _ = write!(scratch.sub_buf, "CACHE {}/{}", caches_claimed, total_caches);
            } else {
                let _ = write!(
                    scratch.sub_buf,
                    "CACHES: {} / {} CLAIMED",
                    caches_claimed, total_caches
                );
            }
            let ccol = if caches_unclaimed == 0 {
                palette::to_color(CLAIMED_MINT_RGB)
            } else {
                palette::to_color(CACHE_GOLD_RGB)
            };
            buf.set_stringn(
                x,
                y,
                &scratch.sub_buf,
                max_w,
                Style::default()
                    .fg(ccol)
                    .bg(PLATE)
                    .add_modifier(Modifier::BOLD),
            );
            y += 1;
        }
    }

    if y < bottom {
        let (status, scol) = if boss_active {
            (
                if compact { "APEX ACTIVE" } else { "◆ APEX ACTIVE" },
                palette::to_color(BOSS_ROSE_RGB),
            )
        } else if boss_arena_discovered {
            (
                if compact { "ARENA FOUND" } else { "◈ ARENA LOCATED" },
                palette::to_color(BOSS_ROSE_RGB),
            )
        } else if bearing_active {
            (
                if compact { "BEARING ON" } else { "◤ BEARING LOCKED" },
                palette::to_color(WARN_AMBER_RGB),
            )
        } else {
            (
                if compact { "UNEXPLORED" } else { "· FOG OF WAR" },
                MUTED,
            )
        };
        buf.set_stringn(
            x,
            y,
            status,
            max_w,
            Style::default()
                .fg(scol)
                .bg(PLATE)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }

    // Tall-terminal visibility key + controls footer (120×40 and 200×60).
    if !compact && y + 7 <= bottom {
        y += 1;
        for i in 0..max_w.min(24) {
            buf[(x + i as u16, y)].set_char('─').set_fg(EDGE).set_bg(PLATE);
        }
        y += 1;
        buf.set_stringn(
            x,
            y,
            "VISIBILITY",
            max_w,
            Style::default().fg(MUTED).bg(PLATE),
        );
        y += 1;
        let vis_rows: &[(&str, Color, &str)] = &[
            ("█", palette::to_color(theme.floor_live), "Live Vision"),
            (
                "█",
                palette::to_color(apply_recency_shading(theme, theme.floor_live, 0.0)),
                "Explored Memory",
            ),
            ("░", palette::to_color(theme.fog_frontier), "Uncharted Fog"),
        ];
        for &(swatch, col, lbl) in vis_rows {
            if y + 2 >= bottom {
                break;
            }
            buf.set_stringn(x, y, swatch, 2, Style::default().fg(col).bg(PLATE));
            buf.set_stringn(
                x + 2,
                y,
                lbl,
                max_w.saturating_sub(2),
                Style::default().fg(MUTED).bg(PLATE),
            );
            y += 1;
        }
    }

    // Pin radar size status + key-hint line at the bottom of tall sidebars.
    if !compact && bottom >= side.y + 22 {
        let ky = bottom - 1;
        if ky > y + 1 {
            buf.set_stringn(
                x,
                ky - 1,
                compact_size.sidebar_status(),
                max_w,
                Style::default().fg(MUTED).bg(PLATE),
            );
        }
        if ky > y {
            buf.set_stringn(
                x,
                ky,
                "[TAB] Close  [N] Size",
                max_w,
                Style::default().fg(EDGE).bg(PLATE),
            );
        }
    }
}

// ---- Color Palette & Fog-of-War Helpers ------------------------------------

struct DerivedPalette {
    fog_base: Rgb,
    fog_grid: Rgb,
    fog_frontier: Rgb,
    memory_sink: Rgb,
    floor_live: Rgb,
    boss_floor: Rgb,
    wall_mass: Rgb,
    wall_rim: Rgb,
    maze_wall: Rgb,
    doorway: Rgb,
    accent: Rgb,
}

impl DerivedPalette {
    fn from_palette(p: MinimapPalette) -> Self {
        let fog_base = palette::scale(palette::mix(PLATE_RGB, p.bg, 0.32), 0.52);
        let fog_grid = palette::add(fog_base, (5, 6, 7));
        let fog_frontier = palette::mix(fog_base, p.wall_edge, 0.24);
        let memory_sink = palette::mix(fog_base, PLATE_RGB, 0.52);

        // Figure-ground hierarchy:
        // 1. Deep bedrock wall mass sits dark (close to fog) so thick rock never
        //    masquerades as an open room.
        // 2. Walkable room floor is a clear, readable blueprint fill.
        // 3. Architectural wall rims (walls bordering seen floor) gleam crisply.
        // 4. Thin maze partitions carry a distinct luminous accent hue.
        let floor_live = palette::add(palette::mix(p.bg_alt, p.wall, 0.28), (11, 13, 13));
        let boss_floor = palette::mix(floor_live, (148, 44, 82), 0.42);
        let wall_mass = palette::mix(fog_base, p.wall, 0.26);
        let wall_rim = palette::mix(p.wall_edge, COAT_RGB, 0.30);
        let maze_wall = palette::mix(p.accent, COAT_RGB, 0.20);
        let doorway = palette::mix(floor_live, p.accent, 0.36);

        Self {
            fog_base,
            fog_grid,
            fog_frontier,
            memory_sink,
            floor_live,
            boss_floor,
            wall_mass,
            wall_rim,
            maze_wall,
            doorway,
            accent: p.accent,
        }
    }
}

#[inline]
fn encode_vis(recency: f32) -> u8 {
    (1.0 + recency.clamp(0.0, 1.0) * 254.0).round() as u8
}

#[inline]
fn decode_recency(v: u8) -> f32 {
    if v <= 1 {
        0.0
    } else {
        f32::from(v - 1) / 254.0
    }
}

/// Sparse 8×8 cartographic coordinate dots for unseen void.
/// Strictly coordinate-driven; never reads map geometry.
#[inline]
fn fog_void_color(theme: &DerivedPalette, x: i32, y: i32) -> Rgb {
    if x.rem_euclid(8) == 0 && y.rem_euclid(8) == 0 {
        theme.fog_grid
    } else {
        theme.fog_base
    }
}

/// Smoothly shade between deep explored memory (`recency == 0.0`) and crisp
/// live line-of-sight (`recency == 1.0`).
#[inline]
fn apply_recency_shading(theme: &DerivedPalette, live_col: Rgb, recency: f32) -> Rgb {
    let r = recency.clamp(0.0, 1.0);
    let fade = (1.0 - r) * 0.46;
    palette::mix(live_col, theme.memory_sink, fade)
}

#[inline]
fn classify_compact_cell(
    theme: &DerivedPalette,
    cell: MinimapCell,
    cx: i32,
    cy: i32,
    source: &(impl MinimapSource + ?Sized),
) -> (Rgb, u8) {
    match cell {
        MinimapCell::Unknown => (theme.fog_base, KIND_UNSEEN),
        MinimapCell::Floor => (theme.floor_live, KIND_FLOOR),
        MinimapCell::BossFloor => (theme.boss_floor, KIND_FLOOR),
        MinimapCell::Wall => (theme.wall_mass, KIND_WALL),
        MinimapCell::MazeWall => (theme.maze_wall, KIND_FEATURE),
        MinimapCell::Doorway => (theme.doorway, KIND_FLOOR),
        MinimapCell::OneWayDoor(_) => (ONE_WAY_EXIT_RGB, KIND_FEATURE),
        MinimapCell::RatHole(axis) => {
            if source.rat_hole_far_known(cx, cy, axis) {
                (RAT_HOLE_LINKED_RGB, KIND_FEATURE)
            } else {
                (RAT_HOLE_NEAR_RGB, KIND_FEATURE)
            }
        }
        MinimapCell::Trap => (palette::mix(theme.floor_live, WARN_AMBER_RGB, 0.42), KIND_FLOOR),
    }
}

/// Two-purpose O(W×H) post-pass using ONLY `scratch.vis` and `scratch.kind`:
/// 1. Elevates any seen `KIND_WALL` pixel adjacent to a seen `KIND_FLOOR`
///    pixel into a crisp `wall_rim` architectural outline (with subtle
///    top-left highlight matching Rat Run's 3/4 lighting).
/// 2. Adds an atmospheric frontier mist halo on unseen pixels touching seen
///    space, without ever reading unseen map cells.
fn apply_wall_rims_and_fog_frontier(
    scratch: &mut MinimapScratch,
    pw: usize,
    ph: usize,
    theme: &DerivedPalette,
) {
    for py in 0..ph {
        for px in 0..pw {
            let idx = py * pw + px;
            let v = scratch.vis[idx];
            if v == 0 {
                let touches_seen = (px > 0 && scratch.vis[idx - 1] > 0)
                    || (px + 1 < pw && scratch.vis[idx + 1] > 0)
                    || (py > 0 && scratch.vis[idx - pw] > 0)
                    || (py + 1 < ph && scratch.vis[idx + pw] > 0);
                if touches_seen {
                    scratch.pixels[idx] = theme.fog_frontier;
                }
                continue;
            }

            if scratch.kind[idx] == KIND_WALL {
                let floor_s = py + 1 < ph && scratch.kind[idx + pw] == KIND_FLOOR;
                let floor_e = px + 1 < pw && scratch.kind[idx + 1] == KIND_FLOOR;
                let floor_n = py > 0 && scratch.kind[idx - pw] == KIND_FLOOR;
                let floor_w = px > 0 && scratch.kind[idx - 1] == KIND_FLOOR;
                if floor_s || floor_e || floor_n || floor_w {
                    let rec = decode_recency(v);
                    let rim = if floor_s || floor_e {
                        palette::mix(theme.wall_rim, COAT_RGB, 0.14)
                    } else {
                        theme.wall_rim
                    };
                    scratch.pixels[idx] = apply_recency_shading(theme, rim, rec);
                }
            }

            let touches_unseen = (px > 0 && scratch.vis[idx - 1] == 0)
                || (px + 1 < pw && scratch.vis[idx + 1] == 0)
                || (py > 0 && scratch.vis[idx - pw] == 0)
                || (py + 1 < ph && scratch.vis[idx + pw] == 0);
            if touches_unseen && scratch.kind[idx] != KIND_FEATURE {
                scratch.pixels[idx] = palette::mix(scratch.pixels[idx], theme.fog_base, 0.22);
            }
        }
    }
}

// ---- Geometry & Raster Helpers ---------------------------------------------

#[inline]
fn in_bounds(source: &(impl MinimapSource + ?Sized), cx: i32, cy: i32) -> bool {
    cx >= 0 && cy >= 0 && cx < source.cols() && cy < source.rows()
}

/// Compute largest 1:1-aspect sub-pixel canvas inside `avail` for a `cols × rows` map.
fn fit_map_canvas(avail: Rect, cols: i32, rows: i32) -> (Rect, usize, usize) {
    if avail.width == 0 || avail.height == 0 || cols <= 0 || rows <= 0 {
        return (Rect::default(), 0, 0);
    }
    let max_pw = avail.width as f32;
    let max_ph = (avail.height as f32) * 2.0;
    let scale = (max_pw / cols as f32).min(max_ph / rows as f32);
    let pw = ((cols as f32 * scale).floor() as usize).clamp(1, avail.width as usize);
    let raw_ph = ((rows as f32 * scale).floor() as usize).clamp(2, avail.height as usize * 2);
    let ph = (raw_ph / 2).max(1) * 2;
    let tw = pw as u16;
    let th = (ph / 2) as u16;
    let ox = avail.x + (avail.width.saturating_sub(tw)) / 2;
    let oy = avail.y + (avail.height.saturating_sub(th)) / 2;
    (Rect::new(ox, oy, tw, th), pw, ph)
}

#[inline]
fn plot_pixel_in_window(
    pixels: &mut [Rgb],
    pw: usize,
    ph: usize,
    lx: f32,
    ly: f32,
    col: Rgb,
) -> bool {
    let px = lx.floor() as isize;
    let py = ly.floor() as isize;
    if px < 0 || py < 0 || px >= pw as isize || py >= ph as isize {
        return false;
    }
    pixels[py as usize * pw + px as usize] = col;
    true
}

/// Draw a subtle camera viewport rectangle: crisp corner ticks + delicate edge
/// hairlines so the player's screen footprint reads cleanly without obscuring
/// the room geometry or player marker inside it.
#[allow(clippy::too_many_arguments)]
fn draw_viewport_brackets(
    pixels: &mut [Rgb],
    pw: usize,
    ph: usize,
    vp: CellRect,
    origin_x: f32,
    origin_y: f32,
    scale_x: f32,
    scale_y: f32,
    theme: &DerivedPalette,
    compact_mode: bool,
) {
    if vp.w <= 0.0 || vp.h <= 0.0 {
        return;
    }
    let x0 = ((vp.x - origin_x) * scale_x).floor() as isize;
    let y0 = ((vp.y - origin_y) * scale_y).floor() as isize;
    let x1 = (((vp.x + vp.w - origin_x) * scale_x).ceil() as isize - 1).max(x0);
    let y1 = (((vp.y + vp.h - origin_y) * scale_y).ceil() as isize - 1).max(y0);
    let w = x1 - x0 + 1;
    let h = y1 - y0 + 1;

    if !compact_mode && (w < 4 || h < 3) {
        return;
    }

    let corner_col = palette::mix(COAT_RGB, theme.accent, 0.25);
    let edge_col = palette::mix(COAT_RGB, theme.accent, 0.45);
    let corner_alpha = if compact_mode { 0.56 } else { 0.50 };
    let edge_alpha = if compact_mode { 0.22 } else { 0.20 };

    let mut blend_at = |x: isize, y: isize, col: Rgb, a: f32| {
        if x >= 0 && y >= 0 && x < pw as isize && y < ph as isize {
            let idx = y as usize * pw + x as usize;
            pixels[idx] = palette::mix(pixels[idx], col, a);
        }
    };

    for x in x0..=x1 {
        let is_corner = x == x0 || x == x1 || (w >= 7 && (x == x0 + 1 || x == x1 - 1));
        let (col, a) = if is_corner {
            (corner_col, corner_alpha)
        } else {
            (edge_col, edge_alpha)
        };
        blend_at(x, y0, col, a);
        if y1 > y0 {
            blend_at(x, y1, col, a);
        }
    }
    for y in (y0 + 1)..y1 {
        let is_corner = h >= 6 && (y == y0 + 1 || y == y1 - 1);
        let (col, a) = if is_corner {
            (corner_col, corner_alpha)
        } else {
            (edge_col, edge_alpha)
        };
        blend_at(x0, y, col, a);
        if x1 > x0 {
            blend_at(x1, y, col, a);
        }
    }
}

/// Blit a `pw × (area.height * 2)` RGB pixel array into `buf` using `▀` half-blocks.
fn blit_half_blocks(buf: &mut Buffer, area: Rect, pixels: &[Rgb], pw: usize) {
    for row in 0..area.height {
        let top_y = (row as usize) * 2;
        let bot_y = top_y + 1;
        let ty = area.y + row;
        for col in 0..area.width {
            let px = col as usize;
            let top = pixels[top_y * pw + px];
            let bot = pixels[bot_y * pw + px];
            let cell = &mut buf[(area.x + col, ty)];
            if top == bot {
                cell.set_char(' ')
                    .set_fg(palette::to_color(top))
                    .set_bg(palette::to_color(bot));
            } else {
                cell.set_char('▀')
                    .set_fg(palette::to_color(top))
                    .set_bg(palette::to_color(bot));
            }
        }
    }
}

/// Stamp a single-cell symbolic glyph over an already-blitted half-block cell,
/// keeping its underlying background color so icons float seamlessly over terrain.
fn stamp_glyph(buf: &mut Buffer, x: u16, y: u16, ch: char, fg: Color, bold: bool) {
    let cell = &mut buf[(x, y)];
    let bg = cell.bg;
    let mut style = Style::default().fg(fg).bg(bg);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    cell.set_char(ch).set_style(style);
}

/// Stamped metal plate frame matching `render/menu.rs::panel`:
/// light catches the top and left edges (`lit`), shadow falls on bottom/right (`dim`).
fn draw_stamped_frame(buf: &mut Buffer, r: Rect, lit: Color, dim: Color) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;

    let lit_s = Style::default().fg(lit).bg(PLATE);
    let dim_s = Style::default().fg(dim).bg(PLATE);

    buf[(x0, y0)].set_char('┌').set_style(lit_s);
    buf[(x1, y0)].set_char('┐').set_style(lit_s);
    buf[(x0, y1)].set_char('└').set_style(lit_s);
    buf[(x1, y1)].set_char('┘').set_style(dim_s);

    for x in (x0 + 1)..x1 {
        buf[(x, y0)].set_char('─').set_style(lit_s);
        buf[(x, y1)].set_char('─').set_style(dim_s);
    }
    for y in (y0 + 1)..y1 {
        buf[(x0, y)].set_char('│').set_style(lit_s);
        buf[(x1, y)].set_char('│').set_style(dim_s);
    }
}

/// 8-way single-cell Geometric Shapes compass pointer (`▲◥►◢▼◣◄◤`).
#[inline]
fn dir_chevron_8(dx: f32, dy: f32) -> char {
    let angle = dy.atan2(dx);
    let sector = ((angle + std::f32::consts::PI) / std::f32::consts::FRAC_PI_4).round() as i32;
    match sector.rem_euclid(8) {
        0 => '◄', // West
        1 => '◤', // North-West
        2 => '▲', // North
        3 => '◥', // North-East
        4 => '►', // East
        5 => '◢', // South-East
        6 => '▼', // South
        _ => '◣', // South-West
    }
}

/// Place a crisp 8-way directional chevron on the compact minimap's outer rim
/// pointing toward an off-window boss or fog-edge arena bearing.
fn draw_rim_chevron(buf: &mut Buffer, rect: Rect, dx: f32, dy: f32, fg: Color) {
    if rect.width < 6 || rect.height < 5 || (dx.abs() < 1e-4 && dy.abs() < 1e-4) {
        return;
    }
    let ch = dir_chevron_8(dx, dy);
    let style = Style::default()
        .fg(fg)
        .bg(PLATE)
        .add_modifier(Modifier::BOLD);
    if dx.abs() >= dy.abs() {
        let frac = ((dy / dx.abs().max(1e-3)) * 0.5 + 0.5).clamp(0.15, 0.85);
        let y = rect.y + 1 + ((rect.height.saturating_sub(3)) as f32 * frac).round() as u16;
        let x = if dx >= 0.0 {
            rect.right() - 1
        } else {
            rect.x
        };
        buf[(x, y)].set_char(ch).set_style(style);
    } else {
        let frac = ((dx / dy.abs().max(1e-3)) * 0.5 + 0.5).clamp(0.20, 0.80);
        let x = rect.x + 1 + ((rect.width.saturating_sub(3)) as f32 * frac).round() as u16;
        let y = if dy >= 0.0 {
            rect.bottom() - 1
        } else {
            rect.y
        };
        let right_guard = if rect.width >= 18 { 8 } else { 4 };
        if y == rect.y && (x < rect.x + 7 || x >= rect.right().saturating_sub(right_guard)) {
            let left_bound = rect.x + 7;
            let right_bound = rect.right().saturating_sub(right_guard + 1).max(left_bound);
            let cx = left_bound + (right_bound - left_bound) / 2;
            buf[(cx, y)].set_char(ch).set_style(style);
        } else {
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}

// ---- Tests & Visual Dump Harness -------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as StdCell;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Synthetic 200 × 160 test map with rooms, halls, corridors, boss arena,
    /// three mazes (partially explored, claimed, and completely undiscovered),
    /// rat-holes (one linked, one near-only), real 2D raycasted LOS, and
    /// fog of war over most of the map.
    struct FakeMap {
        cols: i32,
        rows: i32,
        cells: Vec<MinimapCell>,
        seen: Vec<bool>,
        recency: Vec<f32>,
        unseen_reads: StdCell<usize>,
    }

    impl FakeMap {
        fn new(cols: i32, rows: i32) -> Self {
            let n = (cols * rows) as usize;
            Self {
                cols,
                rows,
                cells: vec![MinimapCell::Wall; n],
                seen: vec![false; n],
                recency: vec![0.0; n],
                unseen_reads: StdCell::new(0),
            }
        }

        fn idx(&self, cx: i32, cy: i32) -> usize {
            (cy * self.cols + cx) as usize
        }

        fn set(&mut self, cx: i32, cy: i32, cell: MinimapCell) {
            if cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows {
                let i = self.idx(cx, cy);
                self.cells[i] = cell;
            }
        }

        fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, cell: MinimapCell) {
            for cy in y..(y + h) {
                for cx in x..(x + w) {
                    self.set(cx, cy, cell);
                }
            }
        }

        fn is_opaque(&self, cx: i32, cy: i32) -> bool {
            if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
                return true;
            }
            matches!(
                self.cells[self.idx(cx, cy)],
                MinimapCell::Wall | MinimapCell::MazeWall | MinimapCell::RatHole(_)
            )
        }

        /// Realistic 2D line-of-sight reveal: casts rays in 360° from `(cx0, cy0)`,
        /// stopping right on the face of walls/maze partitions/rat-holes so walls
        /// cast genuine shadows and maze interiors stay dark behind partitions.
        fn reveal_los(&mut self, cx0: f32, cy0: f32, radius: f32, inner_r: f32, base_rec: f32) {
            let rays = 360;
            for r in 0..rays {
                let ang = (r as f32) * std::f32::consts::TAU / (rays as f32);
                let dx = ang.cos();
                let dy = ang.sin();
                let mut dist = 0.0f32;
                while dist <= radius {
                    let cx = (cx0 + dx * dist).floor() as i32;
                    let cy = (cy0 + dy * dist).floor() as i32;
                    if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
                        break;
                    }
                    let t = if dist <= inner_r {
                        1.0
                    } else {
                        (1.0 - (dist - inner_r) / (radius - inner_r).max(0.1)).clamp(0.0, 1.0)
                    };
                    let i = self.idx(cx, cy);
                    self.seen[i] = true;
                    self.recency[i] = self.recency[i].max(t * base_rec);
                    if self.is_opaque(cx, cy) {
                        break;
                    }
                    dist += 0.45;
                }
            }
        }

        fn reveal_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rec: f32) {
            for cy in y..(y + h) {
                for cx in x..(x + w) {
                    if cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows {
                        let i = self.idx(cx, cy);
                        self.seen[i] = true;
                        self.recency[i] = self.recency[i].max(rec);
                    }
                }
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn carve_maze(
            &mut self,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            entry: (i32, i32),
            exit: (i32, i32),
            exit_dir: MinimapDir,
        ) {
            self.fill_rect(x - 1, y - 1, w + 2, h + 2, MinimapCell::Floor);
            self.fill_rect(x, y, w, h, MinimapCell::MazeWall);
            for ay in (y + 1..y + h - 1).step_by(2) {
                for ax in (x + 1)..(x + w - 1) {
                    self.set(ax, ay, MinimapCell::Floor);
                }
                if ay + 2 < y + h - 1 {
                    let turn_x = if ((ay - y) / 2) % 2 == 0 {
                        x + w - 2
                    } else {
                        x + 1
                    };
                    self.set(turn_x, ay + 1, MinimapCell::Floor);
                    let mid_x = x + w / 2;
                    self.set(mid_x, ay + 1, MinimapCell::Floor);
                }
            }
            self.set(entry.0, entry.1, MinimapCell::Floor);
            self.set(entry.0, entry.1 + 1, MinimapCell::Floor);
            self.set(exit.0, exit.1, MinimapCell::OneWayDoor(exit_dir));
        }

        fn showcase(boss_discovered: bool) -> Self {
            let mut m = Self::new(200, 160);

            // 1. Start room (center 100, 80).
            m.fill_rect(88, 70, 24, 20, MinimapCell::Floor);
            m.fill_rect(93, 74, 2, 2, MinimapCell::Wall);
            m.fill_rect(105, 84, 2, 2, MinimapCell::Wall);

            // 2. East Hall (separated from Start by a 1-cell wall with doorway + linked rat-hole).
            m.fill_rect(113, 66, 26, 26, MinimapCell::Floor);
            m.set(112, 79, MinimapCell::Doorway);
            m.set(112, 80, MinimapCell::Doorway);
            m.set(112, 72, MinimapCell::RatHole(MinimapAxis::Vertical));

            // 3. North-East Vault (behind a rat-hole whose far mouth is UNSEEN!).
            m.fill_rect(140, 66, 20, 20, MinimapCell::Floor);
            m.set(139, 75, MinimapCell::RatHole(MinimapAxis::Vertical));

            // 4. Maze A (South-East of East Hall, breach seen, deeper warren still dark fog).
            m.carve_maze(144, 88, 22, 18, (144, 93), (165, 102), MinimapDir::East);
            m.fill_rect(139, 90, 5, 3, MinimapCell::Floor);
            m.set(139, 91, MinimapCell::Doorway);

            // 5. West Corridor & South-West Hall + Maze B (fully explored & claimed).
            m.fill_rect(54, 76, 34, 4, MinimapCell::Floor);
            m.set(87, 77, MinimapCell::Doorway);
            m.set(87, 78, MinimapCell::Doorway);
            m.fill_rect(42, 66, 16, 26, MinimapCell::Floor);
            m.carve_maze(34, 96, 22, 16, (44, 96), (55, 108), MinimapDir::East);
            m.fill_rect(43, 92, 4, 4, MinimapCell::Floor);
            m.set(44, 92, MinimapCell::Doorway);

            // 6. North Corridor toward Boss Arena (North-West, >= 400 cell chamber).
            m.fill_rect(48, 36, 4, 30, MinimapCell::Floor);
            m.set(49, 65, MinimapCell::Doorway);
            m.fill_rect(22, 16, 28, 22, MinimapCell::BossFloor);
            m.set(49, 38, MinimapCell::Doorway);
            m.set(50, 38, MinimapCell::Doorway);

            // 7. Maze C (Completely UNDISCOVERED in South fog!).
            m.carve_maze(96, 118, 24, 20, (96, 124), (119, 134), MinimapDir::East);
            m.fill_rect(96, 90, 4, 28, MinimapCell::Floor);
            m.set(97, 90, MinimapCell::Doorway);

            // Traps in explored east hall.
            m.set(122, 74, MinimapCell::Trap);
            m.set(128, 84, MinimapCell::Trap);

            // ---- Realistic Raycasted Fog-of-War Path ----
            // Player walked from Start -> West Hall -> Maze B -> North Corridor peek -> East Hall.
            for &(x, y, r, rec) in &[
                (100.0, 80.0, 18.0, 0.14),
                (76.0, 78.0, 14.0, 0.08),
                (52.0, 78.0, 16.0, 0.06),
                (50.0, 62.0, 12.0, 0.05),
                (50.0, 88.0, 12.0, 0.06),
                (118.0, 78.0, 16.0, 0.28),
            ] {
                m.reveal_los(x, y, r, r * 0.5, rec);
            }
            // Maze B fully walked and claimed in earlier exploration memory.
            m.reveal_rect(33, 95, 24, 18, 0.08);
            // Current player position (133.5, 86.5) casts crisp live LOS!
            m.reveal_los(133.5, 86.5, 16.0, 8.0, 1.0);
            // Player stepped up to Maze A's breach and peered one aisle in (rest stays dark!).
            m.reveal_los(143.5, 93.5, 7.5, 4.0, 0.94);
            m.reveal_los(145.5, 93.5, 3.2, 2.0, 0.90);

            if boss_discovered {
                for &(x, y, r, rec) in &[
                    (50.0, 48.0, 14.0, 0.12),
                    (36.0, 27.0, 22.0, 0.24),
                ] {
                    m.reveal_los(x, y, r, r * 0.6, rec);
                }
            }

            m
        }
    }

    impl MinimapSource for FakeMap {
        fn cols(&self) -> i32 {
            self.cols
        }
        fn rows(&self) -> i32 {
            self.rows
        }
        fn cell_at(&self, cx: i32, cy: i32) -> MinimapCell {
            if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
                return MinimapCell::Unknown;
            }
            let i = self.idx(cx, cy);
            if !self.seen[i] {
                self.unseen_reads.set(self.unseen_reads.get() + 1);
            }
            self.cells[i]
        }
        fn seen(&self, cx: i32, cy: i32) -> bool {
            if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
                return false;
            }
            self.seen[self.idx(cx, cy)]
        }
        fn recency(&self, cx: i32, cy: i32) -> f32 {
            if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
                return 0.0;
            }
            self.recency[self.idx(cx, cy)]
        }
    }

    fn sample_markers(boss_discovered: bool) -> Vec<MapMarker> {
        let mut markers = vec![
            MapMarker { cx: 100.0, cy: 80.0, kind: MarkerKind::Start },
            MapMarker { cx: 144.0, cy: 93.0, kind: MarkerKind::MazeEntrance },
            MapMarker { cx: 146.0, cy: 93.0, kind: MarkerKind::MazeCacheHollow },
            MapMarker { cx: 44.0, cy: 96.0, kind: MarkerKind::MazeEntrance },
            MapMarker { cx: 45.0, cy: 104.0, kind: MarkerKind::MazeCacheClaimed },
            // Unseen Maze C reward & entrance (must NEVER render!).
            MapMarker { cx: 96.0, cy: 124.0, kind: MarkerKind::MazeEntrance },
            MapMarker { cx: 112.0, cy: 132.0, kind: MarkerKind::MazeReward },
            MapMarker { cx: 108.0, cy: 128.0, kind: MarkerKind::MazeCacheHollow },
            // Active seen pickup in East Hall.
            MapMarker { cx: 125.0, cy: 82.0, kind: MarkerKind::Pickup(PickupTag::Heal) },
        ];
        if boss_discovered {
            markers.push(MapMarker { cx: 36.0, cy: 27.0, kind: MarkerKind::BossArena });
            markers.push(MapMarker { cx: 40.0, cy: 29.0, kind: MarkerKind::Boss });
        }
        markers
    }

    fn sample_labels(boss_discovered: bool) -> Vec<MapLabel<'static>> {
        vec![
            MapLabel { cx: 100.0, cy: 80.0, text: "START", kind: LabelKind::Start, discovered: true },
            MapLabel { cx: 36.0, cy: 27.0, text: "BOSS ARENA", kind: LabelKind::BossArena, discovered: boss_discovered },
            MapLabel { cx: 108.0, cy: 128.0, text: "SECRET MAZE", kind: LabelKind::Maze, discovered: false },
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn make_view<'a>(
        mode: MinimapMode,
        palette: MinimapPalette,
        sector: &'a str,
        markers: &'a [MapMarker],
        labels: &'a [MapLabel<'a>],
        boss_discovered: bool,
        term_w: u16,
        term_h: u16,
    ) -> MinimapView<'a> {
        let vp_w = (f32::from(term_w) / 16.0).max(4.0);
        let vp_h = (f32::from(term_h.saturating_sub(2)) * 2.0 / 16.0).max(2.5);
        MinimapView {
            player_cell: (133.5, 86.5),
            player_facing: (0.85, 0.52),
            viewport_cells: CellRect {
                x: 133.5 - vp_w * 0.5,
                y: 86.5 - vp_h * 0.5,
                w: vp_w,
                h: vp_h,
            },
            markers,
            labels,
            boss_bearing_dir: if boss_discovered {
                None
            } else {
                Some((-0.855, -0.518))
            },
            palette,
            sector_name: sector,
            clock: 185.4,
            mode,
            compact_size: MinimapSize::Normal,
            hud_compass_active: false,
            hovered: false,
            explored_pct: None,
            maze_discovered: true,
        }
    }

    #[test]
    fn renders_all_sizes_and_modes_without_panicking() {
        let map = FakeMap::showcase(false);
        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let mut scratch = MinimapScratch::new();

        for (w, h) in [
            (0, 0),
            (0, 5),
            (5, 0),
            (1, 1),
            (20, 10),
            (57, 20),
            (80, 24),
            (120, 40),
            (200, 60),
        ] {
            for mode in [MinimapMode::Compact, MinimapMode::Expanded] {
                for compact_size in [
                    MinimapSize::Normal,
                    MinimapSize::Small,
                    MinimapSize::Hidden,
                ] {
                    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let mut view = make_view(
                        mode,
                        MinimapPalette::SEWER,
                        "SEWER",
                        &markers,
                        &labels,
                        false,
                        w,
                        h,
                    );
                    view.compact_size = compact_size;
                    terminal
                        .draw(|f| draw(f, f.area(), &view, &map, &mut scratch))
                        .unwrap();
                }
            }
        }
        assert_eq!(
            map.unseen_reads.get(),
            0,
            "renderer must NEVER call cell_at on unseen cells"
        );
    }

    #[test]
    fn compact_mode_performs_zero_reallocation_across_all_sizes() {
        let map = FakeMap::showcase(false);
        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let mut scratch = MinimapScratch::new();
        let cap_pixels = scratch.pixels.capacity();
        let cap_vis = scratch.vis.capacity();
        let cap_kind = scratch.kind.capacity();
        let cap_text = scratch.text_buf.capacity();

        for (w, h) in [(57, 20), (80, 24), (120, 40), (200, 60)] {
            for compact_size in [
                MinimapSize::Normal,
                MinimapSize::Small,
                MinimapSize::Hidden,
            ] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut view = make_view(
                    MinimapMode::Compact,
                    MinimapPalette::SEWER,
                    "SEWER",
                    &markers,
                    &labels,
                    false,
                    w,
                    h,
                );
                view.compact_size = compact_size;
                for _ in 0..3 {
                    terminal
                        .draw(|f| draw(f, f.area(), &view, &map, &mut scratch))
                        .unwrap();
                }
            }
        }
        assert_eq!(scratch.pixels.capacity(), cap_pixels);
        assert_eq!(scratch.vis.capacity(), cap_vis);
        assert_eq!(scratch.kind.capacity(), cap_kind);
        assert_eq!(scratch.text_buf.capacity(), cap_text);
    }

    #[test]
    fn compact_degrades_to_empty_on_tiny_terminals() {
        // Tiny terminals below Small threshold (48x14) degrade to empty.
        assert!(compact_rect(Rect::new(0, 0, 47, 18)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 57, 13)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 20, 6)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 0, 0)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 0, 5)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 5, 0)).is_empty());
        assert!(compact_rect(Rect::new(0, 0, 1, 1)).is_empty());

        // Narrow terminals (e.g. 57x20 -> game 57x18) automatically step down
        // from Normal to Small without overwriting the saved preference.
        let game57 = Rect::new(0, 1, 57, 18);
        assert!(!compact_size_fits(game57, MinimapSize::Normal));
        assert!(compact_size_fits(game57, MinimapSize::Small));
        assert_eq!(
            effective_compact_size(game57, MinimapSize::Normal),
            MinimapSize::Small
        );
        let r57 = compact_rect_for_size(game57, MinimapSize::Normal);
        assert_eq!((r57.width, r57.height), (14, 7));
        assert!(compact_rect_for_size(game57, MinimapSize::Hidden).is_empty());

        // Standard (80x24 -> game 80x22): Normal 24x10, Small 14x7, Hidden empty.
        let game80 = Rect::new(0, 1, 80, 22);
        let r80 = compact_rect_for_size(game80, MinimapSize::Normal);
        let r80_sm = compact_rect_for_size(game80, MinimapSize::Small);
        assert_eq!((r80.width, r80.height), (24, 10));
        assert_eq!((r80_sm.width, r80_sm.height), (14, 7));
        assert!(r80_sm.area() < r80.area());
        assert!(compact_rect_for_size(game80, MinimapSize::Hidden).is_empty());

        // Medium (120x40 -> game 120x38): Normal 34x14, Small 18x9, Hidden empty.
        let game120 = Rect::new(0, 1, 120, 38);
        let r120 = compact_rect_for_size(game120, MinimapSize::Normal);
        let r120_sm = compact_rect_for_size(game120, MinimapSize::Small);
        assert_eq!((r120.width, r120.height), (34, 14));
        assert_eq!((r120_sm.width, r120_sm.height), (18, 9));
        assert!(r120_sm.area() < r120.area());
        assert!(compact_rect_for_size(game120, MinimapSize::Hidden).is_empty());

        // Large (200x60 -> game 200x58): Normal 44x18, Small 24x11, Hidden empty.
        let game200 = Rect::new(0, 1, 200, 58);
        let r200 = compact_rect_for_size(game200, MinimapSize::Normal);
        let r200_sm = compact_rect_for_size(game200, MinimapSize::Small);
        assert_eq!((r200.width, r200.height), (44, 18));
        assert_eq!((r200_sm.width, r200_sm.height), (24, 11));
        assert!(r200_sm.area() < r200.area());
        assert!(compact_rect_for_size(game200, MinimapSize::Hidden).is_empty());
    }

    #[test]
    fn minimap_size_cycle_step_and_setting_round_trip() {
        assert_eq!(MinimapSize::Normal.cycle(), MinimapSize::Small);
        assert_eq!(MinimapSize::Small.cycle(), MinimapSize::Hidden);
        assert_eq!(MinimapSize::Hidden.cycle(), MinimapSize::Normal);

        assert_eq!(MinimapSize::Normal.step(1), MinimapSize::Small);
        assert_eq!(MinimapSize::Small.step(1), MinimapSize::Hidden);
        assert_eq!(MinimapSize::Hidden.step(1), MinimapSize::Hidden);
        assert_eq!(MinimapSize::Hidden.step(-1), MinimapSize::Small);
        assert_eq!(MinimapSize::Small.step(-1), MinimapSize::Normal);
        assert_eq!(MinimapSize::Normal.step(-1), MinimapSize::Normal);

        for s in [MinimapSize::Normal, MinimapSize::Small, MinimapSize::Hidden] {
            assert_eq!(MinimapSize::parse_setting(s.as_str()), Some(s));
        }
        assert_eq!(MinimapSize::parse_setting("SMALL"), Some(MinimapSize::Small));
        assert_eq!(MinimapSize::parse_setting("bogus"), None);
        assert_eq!(MinimapSize::parse_setting(""), None);
    }

    #[test]
    fn hidden_mode_draws_only_one_cell_and_respects_hud_compass() {
        let map = FakeMap::showcase(false);
        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let mut scratch = MinimapScratch::new();

        // Pre-warning bearing active (hud_compass_active = false): single corner
        // cell shows the 8-way amber bearing chevron (`◤` for NW), nothing else.
        let mut view = make_view(
            MinimapMode::Compact,
            MinimapPalette::SEWER,
            "SEWER",
            &markers,
            &labels,
            false,
            80,
            24,
        );
        view.compact_size = MinimapSize::Hidden;
        view.hud_compass_active = false;
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| draw(f, f.area(), &view, &map, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let non_blank: Vec<(u16, u16, &str)> = (0..24u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .filter_map(|(x, y)| {
                let s = buf[(x, y)].symbol();
                (s != " ").then_some((x, y, s))
            })
            .collect();
        assert_eq!(non_blank, vec![(78, 22, "◤")]);

        // When the HUD warning compass is active, do NOT duplicate the bearing:
        // only the quiet 1-cell `N` hint appears.
        view.hud_compass_active = true;
        let mut term2 = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term2
            .draw(|f| draw(f, f.area(), &view, &map, &mut scratch))
            .unwrap();
        let buf2 = term2.backend().buffer();
        let non_blank2: Vec<(u16, u16, &str)> = (0..24u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .filter_map(|(x, y)| {
                let s = buf2[(x, y)].symbol();
                (s != " ").then_some((x, y, s))
            })
            .collect();
        assert_eq!(non_blank2, vec![(78, 22, "N")]);
    }

    #[test]
    fn unseen_cells_never_leak_wall_or_floor_colors() {
        let map_a = FakeMap::showcase(false);
        let mut map_b = FakeMap::showcase(false);
        for i in 0..map_b.cells.len() {
            if !map_b.seen[i] {
                map_b.cells[i] = match map_b.cells[i] {
                    MinimapCell::Wall => MinimapCell::Floor,
                    MinimapCell::Floor => MinimapCell::MazeWall,
                    MinimapCell::MazeWall => MinimapCell::OneWayDoor(MinimapDir::North),
                    _ => MinimapCell::Trap,
                };
            }
        }

        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let mut scratch = MinimapScratch::new();

        for (w, h) in [(80, 24), (120, 40)] {
            for mode in [MinimapMode::Compact, MinimapMode::Expanded] {
                for compact_size in [
                    MinimapSize::Normal,
                    MinimapSize::Small,
                    MinimapSize::Hidden,
                ] {
                    let mut view = make_view(
                        mode,
                        MinimapPalette::SEWER,
                        "SEWER",
                        &markers,
                        &labels,
                        false,
                        w,
                        h,
                    );
                    view.compact_size = compact_size;
                    let mut term_a = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let mut term_b = Terminal::new(TestBackend::new(w, h)).unwrap();
                    term_a
                        .draw(|f| draw(f, f.area(), &view, &map_a, &mut scratch))
                        .unwrap();
                    term_b
                        .draw(|f| draw(f, f.area(), &view, &map_b, &mut scratch))
                        .unwrap();
                    assert_eq!(
                        term_a.backend().buffer().content,
                        term_b.backend().buffer().content,
                        "fog of war leaked unseen terrain in {mode:?}/{compact_size:?} at {w}x{h}"
                    );
                }
            }
        }
        assert_eq!(map_a.unseen_reads.get(), 0);
        assert_eq!(map_b.unseen_reads.get(), 0);
    }

    #[test]
    fn unseen_markers_and_labels_are_never_drawn() {
        let map = FakeMap::showcase(false);
        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let mut scratch = MinimapScratch::new();
        let view = make_view(
            MinimapMode::Expanded,
            MinimapPalette::SEWER,
            "SEWER",
            &markers,
            &labels,
            false,
            120,
            40,
        );
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, f.area(), &view, &map, &mut scratch)).unwrap();
        let buf = term.backend().buffer();
        let full_text: String = buf.content.iter().map(|c| c.symbol()).collect();

        // Restrict map-canvas check to the left 82 columns of the 120-col screen
        // (strictly inside the map canvas, excluding the right legend sidebar).
        let mut canvas_text = String::new();
        for y in 4..36 {
            for x in 5..86 {
                canvas_text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(canvas_text.contains("START"), "discovered START label missing on map canvas");
        assert!(
            !full_text.contains("SECRET MAZE"),
            "undiscovered maze label leaked onto map"
        );
        assert!(
            !full_text.contains("BOSS ARENA"),
            "undiscovered boss arena label leaked onto map"
        );
        assert!(canvas_text.contains('◇'), "seen hollow cache symbol missing on map canvas");
        assert!(canvas_text.contains('◆'), "claimed cache symbol missing on map canvas");
        assert!(canvas_text.contains('◤'), "fog-edge northwest bearing missing on map canvas");
        assert!(
            full_text.contains("CACHES: 1 / 2 CLAIMED"),
            "sidebar should count both seen caches (1 claimed, 1 unclaimed)"
        );
        assert!(full_text.contains("BEARING"), "fog-edge boss bearing status missing");
    }

    #[test]
    fn rat_hole_far_mouth_stays_hidden_until_seen() {
        let mut map = FakeMap::showcase(false);
        assert!(map.seen(139, 75));
        assert!(!map.seen(140, 75));
        assert!(!map.rat_hole_far_known(139, 75, MinimapAxis::Vertical));
        assert!(map.rat_hole_far_known(112, 72, MinimapAxis::Vertical));

        map.reveal_rect(140, 74, 2, 3, 1.0);
        assert!(map.rat_hole_far_known(139, 75, MinimapAxis::Vertical));
    }

    #[test]
    fn fog_edge_cell_lands_on_explored_boundary_without_reading_unseen_terrain() {
        let map = FakeMap::showcase(false);
        let edge = fog_edge_cell(&map, (133.5, 86.5), (-0.855, -0.518));
        assert!(edge.is_some(), "expected a fog boundary along northwest bearing");
        let (ex, ey) = edge.unwrap();
        assert!(
            map.seen(ex.floor() as i32, ey.floor() as i32),
            "fog edge marker must sit on a seen cell"
        );
        assert_eq!(map.unseen_reads.get(), 0);
    }

    /// Collect each row of a `TestBackend` buffer as a `String`.
    fn buffer_rows(buf: &Buffer) -> Vec<String> {
        let area = buf.area;
        (area.y..area.bottom())
            .map(|y| {
                (area.x..area.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn expanded_legend_omits_maze_rows_until_maze_discovered() {
        let map = FakeMap::showcase(false);
        let markers = sample_markers(false);
        let labels = sample_labels(false);
        let non_maze_markers: Vec<MapMarker> = markers
            .iter()
            .copied()
            .filter(|m| {
                !matches!(
                    m.kind,
                    MarkerKind::MazeEntrance
                        | MarkerKind::MazeCacheHollow
                        | MarkerKind::MazeCacheClaimed
                        | MarkerKind::MazeReward
                )
            })
            .collect();
        let mut scratch = MinimapScratch::new();

        // 1. Sidebar layout (120×40): maze_discovered = false omits all 5 maze
        //    legend rows and closes the vertical gap (Boss Arena directly below Rat-Hole).
        let mut view_no_maze = make_view(
            MinimapMode::Expanded,
            MinimapPalette::SEWER,
            "SEWER",
            &non_maze_markers,
            &labels,
            false,
            120,
            40,
        );
        view_no_maze.maze_discovered = false;
        let mut term_no = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term_no
            .draw(|f| draw(f, f.area(), &view_no_maze, &map, &mut scratch))
            .unwrap();
        let rows_no = buffer_rows(term_no.backend().buffer());
        let text_no = rows_no.join("\n");
        for forbidden in [
            "Maze Partition",
            "One-Way Exit",
            "Maze Breach",
            "Unclaimed Cache",
            "Claimed Cache",
            "CACHES:",
        ] {
            assert!(
                !text_no.contains(forbidden),
                "maze legend row '{forbidden}' leaked when maze_discovered == false"
            );
        }
        let hole_row = rows_no
            .iter()
            .position(|r| r.contains("Rat-Hole"))
            .expect("Rat-Hole legend row missing");
        let boss_row = rows_no
            .iter()
            .position(|r| r.contains("Boss Arena"))
            .expect("Boss Arena legend row missing");
        assert_eq!(
            boss_row,
            hole_row + 1,
            "omitting maze rows must close the gap (Rat-Hole on {hole_row}, Boss Arena on {boss_row})"
        );

        // 2. Sidebar layout (120×40): maze_discovered = true renders all 5 maze rows.
        let mut view_maze = make_view(
            MinimapMode::Expanded,
            MinimapPalette::SEWER,
            "SEWER",
            &markers,
            &labels,
            false,
            120,
            40,
        );
        view_maze.maze_discovered = true;
        let mut term_yes = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term_yes
            .draw(|f| draw(f, f.area(), &view_maze, &map, &mut scratch))
            .unwrap();
        let text_yes = buffer_rows(term_yes.backend().buffer()).join("\n");
        for expected in [
            "Maze Partition",
            "One-Way Exit",
            "Maze Breach",
            "Unclaimed Cache",
            "Claimed Cache",
        ] {
            assert!(
                text_yes.contains(expected),
                "maze legend row '{expected}' missing when maze_discovered == true"
            );
        }

        // 3. Compact bottom-strip layout (48×16, where has_sidebar == false):
        //    no maze words when maze_discovered == false, maze key when true.
        let mut strip_no = make_view(
            MinimapMode::Expanded,
            MinimapPalette::SEWER,
            "SEWER",
            &non_maze_markers,
            &labels,
            false,
            48,
            16,
        );
        strip_no.maze_discovered = false;
        let mut term_strip_no = Terminal::new(TestBackend::new(48, 16)).unwrap();
        term_strip_no
            .draw(|f| draw(f, f.area(), &strip_no, &map, &mut scratch))
            .unwrap();
        let strip_text_no = buffer_rows(term_strip_no.backend().buffer()).join("\n");
        assert!(!strip_text_no.contains("Cache"));
        assert!(!strip_text_no.contains("Breach"));
        assert!(strip_text_no.contains("Hole"));
        assert!(strip_text_no.contains("Boss"));

        let mut strip_yes = make_view(
            MinimapMode::Expanded,
            MinimapPalette::SEWER,
            "SEWER",
            &markers,
            &labels,
            false,
            48,
            16,
        );
        strip_yes.maze_discovered = true;
        let mut term_strip_yes = Terminal::new(TestBackend::new(48, 16)).unwrap();
        term_strip_yes
            .draw(|f| draw(f, f.area(), &strip_yes, &map, &mut scratch))
            .unwrap();
        let strip_text_yes = buffer_rows(term_strip_yes.backend().buffer()).join("\n");
        assert!(strip_text_yes.contains("Cache"));
        assert!(strip_text_yes.contains("Breach"));
    }

    fn color_rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Reset | Color::Black => (14, 18, 20),
            _ => (200, 200, 200),
        }
    }

    fn paint_mock_game_backdrop(buf: &mut Buffer, area: Rect, pal: MinimapPalette, sector: &str) {
        if area.width < 10 || area.height < 6 {
            return;
        }
        let accent = palette::marking(pal.wall_edge, pal.accent);
        let top = Rect::new(area.x, area.y, area.width, 1);
        buf.set_style(top, Style::default().bg(PLATE));
        buf.set_stringn(
            area.x,
            area.y,
            " T ",
            3,
            Style::default().fg(PLATE).bg(accent).add_modifier(Modifier::BOLD),
        );
        let hdr = " 01:55  Lv7  KO 184  Score 2910  │ ▸Gnaw4 »Chee3 ✦Sque2";
        buf.set_stringn(
            area.x + 3,
            area.y,
            hdr,
            area.width.saturating_sub(14) as usize,
            Style::default().fg(COAT).bg(PLATE),
        );
        let badge = format!(" {} ", sector);
        let bw = badge.chars().count() as u16;
        if area.width > bw + 4 {
            buf.set_stringn(
                area.right() - bw,
                area.y,
                &badge,
                bw as usize,
                Style::default().fg(EDGE).bg(PLATE),
            );
        }

        let game = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
        for y in game.y..game.bottom() {
            for x in game.x..game.right() {
                let mortar = (x % 16 == 0) || (y % 4 == 0);
                let top_c = if mortar { pal.bg } else { pal.bg_alt };
                let bot_c = if (x % 16 == 0) || ((y * 2 + 1) % 8 == 0) {
                    pal.bg
                } else {
                    pal.bg_alt
                };
                buf[(x, y)]
                    .set_char('▀')
                    .set_fg(palette::to_color(top_c))
                    .set_bg(palette::to_color(bot_c));
            }
        }
        let cx = game.x + game.width / 2;
        let cy = game.y + game.height / 2;
        buf[(cx, cy)].set_char('▀').set_fg(COAT).set_bg(TAIL);

        let by = area.bottom() - 1;
        let bot = Rect::new(area.x, by, area.width, 1);
        buf.set_style(bot, Style::default().bg(PLATE));
        buf.set_stringn(
            area.x,
            by,
            " ♥ 84/100 ██████████░░  XP ██████░░░░  │ ▸ DASH READY ",
            area.width as usize,
            Style::default().fg(TAIL).bg(PLATE),
        );
    }

    #[test]
    fn dump_minimap_captures() {
        if std::env::var("RATRUN_DUMP").ok().as_deref() != Some("minimap") {
            return;
        }
        let dir = std::env::var("RATRUN_MINIMAP_DIR").unwrap_or_else(|_| {
            "/private/tmp/claude-501/-Users-joe-dev-ratrun/a0cccbef-9e45-4849-814a-e727185739a1/scratchpad/minimap/final".to_string()
        });
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let map_mid = FakeMap::showcase(false);
        let map_boss = FakeMap::showcase(true);
        let markers_mid = sample_markers(false);
        let markers_boss = sample_markers(true);
        let labels_mid = sample_labels(false);
        let labels_boss = sample_labels(true);
        let mut scratch = MinimapScratch::new();

        struct DumpCase {
            name: &'static str,
            w: u16,
            h: u16,
            mode: MinimapMode,
            pal: MinimapPalette,
            sector: &'static str,
            boss_disc: bool,
            hovered: bool,
            maze_discovered: bool,
        }

        let cases = [
            DumpCase { name: "57x20-compact-sewer", w: 57, h: 20, mode: MinimapMode::Compact, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "57x20-expanded-sewer", w: 57, h: 20, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "80x24-compact-sewer", w: 80, h: 24, mode: MinimapMode::Compact, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "80x24-compact-hovered", w: 80, h: 24, mode: MinimapMode::Compact, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: true, maze_discovered: true },
            DumpCase { name: "80x24-expanded-sewer", w: 80, h: 24, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "120x40-compact-sewer", w: 120, h: 40, mode: MinimapMode::Compact, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "120x40-expanded-sewer", w: 120, h: 40, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "120x40-expanded-no-maze-sewer", w: 120, h: 40, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: false },
            DumpCase { name: "120x40-expanded-boss-kitchen", w: 120, h: 40, mode: MinimapMode::Expanded, pal: MinimapPalette::KITCHEN, sector: "KITCHEN", boss_disc: true, hovered: false, maze_discovered: true },
            DumpCase { name: "120x40-compact-lab", w: 120, h: 40, mode: MinimapMode::Compact, pal: MinimapPalette::LAB, sector: "LAB", boss_disc: true, hovered: false, maze_discovered: true },
            DumpCase { name: "200x60-compact-sewer", w: 200, h: 60, mode: MinimapMode::Compact, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "200x60-expanded-lab", w: 200, h: 60, mode: MinimapMode::Expanded, pal: MinimapPalette::LAB, sector: "LAB", boss_disc: false, hovered: false, maze_discovered: true },
            DumpCase { name: "48x16-expanded-no-maze", w: 48, h: 16, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: false },
            DumpCase { name: "48x16-expanded-with-maze", w: 48, h: 16, mode: MinimapMode::Expanded, pal: MinimapPalette::SEWER, sector: "SEWER", boss_disc: false, hovered: false, maze_discovered: true },
        ];

        let markers_no_maze: Vec<MapMarker> = markers_mid
            .iter()
            .copied()
            .filter(|m| {
                !matches!(
                    m.kind,
                    MarkerKind::MazeEntrance
                        | MarkerKind::MazeCacheHollow
                        | MarkerKind::MazeCacheClaimed
                        | MarkerKind::MazeReward
                )
            })
            .collect();

        for c in &cases {
            let (name, w, h, mode, pal, sector, boss_disc, hovered, maze_disc) = (
                c.name,
                c.w,
                c.h,
                c.mode,
                c.pal,
                c.sector,
                c.boss_disc,
                c.hovered,
                c.maze_discovered,
            );
            let map = if boss_disc { &map_boss } else { &map_mid };
            let markers: &[MapMarker] = if !maze_disc {
                &markers_no_maze
            } else if boss_disc {
                &markers_boss
            } else {
                &markers_mid
            };
            let labels = if boss_disc { &labels_boss } else { &labels_mid };
            let mut view = make_view(mode, pal, sector, markers, labels, boss_disc, w, h);
            view.hovered = hovered;
            view.maze_discovered = maze_disc;

            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    let game = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(2));
                    paint_mock_game_backdrop(f.buffer_mut(), area, pal, sector);
                    draw(f, game, &view, map, &mut scratch);
                })
                .unwrap();

            let buf = terminal.backend().buffer();
            let mut ansi = String::with_capacity((w as usize) * (h as usize) * 28);
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
                }
                ansi.push_str("\x1b[0m\n");
            }
            std::fs::write(dir.join(format!("{name}.ansi")), ansi).unwrap();
        }
    }

    #[test]
    fn dump_minimap_size_captures() {
        if std::env::var("RATRUN_DUMP").ok().as_deref() != Some("minimap_sizes") {
            return;
        }
        let dir = std::env::var("RATRUN_MINIMAP_DIR").unwrap_or_else(|_| {
            "/private/tmp/claude-501/-Users-joe-dev-ratrun/a0cccbef-9e45-4849-814a-e727185739a1/scratchpad/minimap/sizes".to_string()
        });
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let map_mid = FakeMap::showcase(false);
        let markers_mid = sample_markers(false);
        let labels_mid = sample_labels(false);
        let mut scratch = MinimapScratch::new();

        struct SizeCase {
            name: &'static str,
            w: u16,
            h: u16,
            mode: MinimapMode,
            size: MinimapSize,
            hovered: bool,
            hud_compass: bool,
        }

        let cases = [
            // 80x24: Normal, Small, Small-hovered, Hidden (with pre-warning bearing), Hidden (quiet/HUD compass), Expanded
            SizeCase { name: "80x24-normal", w: 80, h: 24, mode: MinimapMode::Compact, size: MinimapSize::Normal, hovered: false, hud_compass: false },
            SizeCase { name: "80x24-small", w: 80, h: 24, mode: MinimapMode::Compact, size: MinimapSize::Small, hovered: false, hud_compass: false },
            SizeCase { name: "80x24-small-hovered", w: 80, h: 24, mode: MinimapMode::Compact, size: MinimapSize::Small, hovered: true, hud_compass: false },
            SizeCase { name: "80x24-hidden-bearing", w: 80, h: 24, mode: MinimapMode::Compact, size: MinimapSize::Hidden, hovered: false, hud_compass: false },
            SizeCase { name: "80x24-hidden-quiet", w: 80, h: 24, mode: MinimapMode::Compact, size: MinimapSize::Hidden, hovered: false, hud_compass: true },
            // 120x40: Normal, Small, Small-hovered, Hidden (with pre-warning bearing), Hidden (quiet/HUD compass), Expanded
            SizeCase { name: "120x40-normal", w: 120, h: 40, mode: MinimapMode::Compact, size: MinimapSize::Normal, hovered: false, hud_compass: false },
            SizeCase { name: "120x40-small", w: 120, h: 40, mode: MinimapMode::Compact, size: MinimapSize::Small, hovered: false, hud_compass: false },
            SizeCase { name: "120x40-small-hovered", w: 120, h: 40, mode: MinimapMode::Compact, size: MinimapSize::Small, hovered: true, hud_compass: false },
            SizeCase { name: "120x40-hidden-bearing", w: 120, h: 40, mode: MinimapMode::Compact, size: MinimapSize::Hidden, hovered: false, hud_compass: false },
            SizeCase { name: "120x40-hidden-quiet", w: 120, h: 40, mode: MinimapMode::Compact, size: MinimapSize::Hidden, hovered: false, hud_compass: true },
            SizeCase { name: "120x40-expanded-small-pref", w: 120, h: 40, mode: MinimapMode::Expanded, size: MinimapSize::Small, hovered: false, hud_compass: false },
            // 57x20: auto-Small fallback from Normal preference, plus Hidden
            SizeCase { name: "57x20-auto-small", w: 57, h: 20, mode: MinimapMode::Compact, size: MinimapSize::Normal, hovered: false, hud_compass: false },
            SizeCase { name: "57x20-hidden-bearing", w: 57, h: 20, mode: MinimapMode::Compact, size: MinimapSize::Hidden, hovered: false, hud_compass: false },
        ];

        for c in &cases {
            let mut view = make_view(
                c.mode,
                MinimapPalette::SEWER,
                "SEWER",
                &markers_mid,
                &labels_mid,
                false,
                c.w,
                c.h,
            );
            view.compact_size = c.size;
            view.hovered = c.hovered;
            view.hud_compass_active = c.hud_compass;

            let mut terminal = Terminal::new(TestBackend::new(c.w, c.h)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    let game = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(2));
                    paint_mock_game_backdrop(f.buffer_mut(), area, MinimapPalette::SEWER, "SEWER");
                    draw(f, game, &view, &map_mid, &mut scratch);
                })
                .unwrap();

            let buf = terminal.backend().buffer();
            let mut ansi = String::with_capacity((c.w as usize) * (c.h as usize) * 28);
            for y in 0..c.h {
                for x in 0..c.w {
                    let cell = &buf[(x, y)];
                    let (r, g, b) = color_rgb(cell.fg);
                    let (br, bg, bb) = color_rgb(cell.bg);
                    let _ = write!(
                        ansi,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        cell.symbol()
                    );
                }
                ansi.push_str("\x1b[0m\n");
            }
            std::fs::write(dir.join(format!("{}.ansi", c.name)), ansi).unwrap();
        }
    }
}
